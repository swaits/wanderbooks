# Receipt & document store + persistence spec

Three pieces: the **journal** (the source of truth), the **blob store**
(the bytes), and **SQLite** (the queryable projection). All immutable-first,
matching the ledger's philosophy: the books never forget, and neither does
the filing cabinet.

## The journal: the source of truth

SQLite is the query engine, not the source of truth. The source of truth is
an **append-only journal of domain events**: one JSON object per line
(JSONL), hash-chained. This is the deliberate answer to "insanely durable,
easily backupable, journable":

- **Durable.** Every state change is an immutable fact appended to
  `journal.jsonl`, fsync'd per append. Each line carries the SHA-256 of the
  previous line's canonical bytes (`prev`), forming a tamper-evident chain:
  bit rot, torn writes, and casual editing are detected on read/verify, not
  discovered at tax time. Honest scope: this detects corruption and casual
  tampering. It does not stop a determined attacker with full write access
  from rewriting the chain — nothing local does.
- **Backupable.** The backup is `cp -r $WANDERBOOKS_DATA /backup`: the
  journal file plus the blob directory. Plain files, no WAL-file timing
  windows, no special backup API. A copy taken mid-write, like a crash or
  power loss mid-append, can end in a cut-off line (bytes after the last
  newline). It was never acknowledged, so opening the books saves those
  bytes to `journal.jsonl.torn.<time>`, truncates to the last newline, and
  journals a `journal.tail_repaired` line; `books.digest` reports it.
  Damage before the last newline is corruption and refuses to open: that
  is for a human, never repaired automatically. Better: the journal is
  git-friendly, so an optional `wanderbooks backup` that commits it to a
  private git repo gives versioned, distributed, deduplicated backup with
  full history.
- **Journable.** It literally is a journal: `{seq, ts, type, payload, prev}`.
  Greppable, diffable, replayable. The audit trail isn't a feature bolted
  on — it's the storage format.

### Write path

There is exactly one write path: **append to the journal, fsync, then apply
to SQLite.** No dual writes, no background applier thread (the volume
doesn't justify it): the MCP server appends, fsyncs, applies the new lines
to the projection synchronously, and returns. A crash between append and
apply is safe — on startup the applier replays every journal line with
`seq > last_applied_seq`.

### Line format

```json
{"seq": 42, "ts": "2026-10-10T18:20:00Z", "type": "entry.posted",
 "payload": { "...": "..." }, "prev": "abc123..."}
```

- `seq`: 1-based, gapless, assigned by the writer.
- `ts`: UTC, RFC 3339. Recorded, never trusted for ordering (`seq` orders).
- `type`: `account.added`, `entry.posted`, `entry.reversed`,
  `receipt.attached`, `receipt.linked`, `receipt.unlinked`,
  `document.filed`, `attachment.withdrawn` (plus the A/R, A/P, asset,
  period, payroll, and class events listed on `JournalLine`).
- `payload`: the full domain facts (account fields, entry with lines, blob
  sha, ...). Everything the applier needs; nothing it must look up elsewhere.
- `prev`: hex SHA-256 of the previous line's canonical JSON (keys sorted, no
  whitespace). The genesis line uses `"prev": "000...0"` (64 zeros).

### Drafts are journaled (durable)

Draft entries are durable: `entry.created` and `entry.line_added` journal
lines record the draft as the agent builds it, so drafts (and receipts
attached to them) survive server restarts. Drafts never touch the SQLite
projection until posted — reports only see posted entries. A draft that
never posts leaves its `created`/`line_added` lines in the journal, which
is correct: the journal records what happened, including abandoned work.

### Verification

`wanderbooks verify` (a `ledger-store` API, later a CLI subcommand):
re-read the journal, recompute the hash chain, replay every line into a fresh
in-memory `Ledger`, and compare its reports against the SQLite projection's.
Any mismatch is a loud failure. Run it on startup (cheap at this scale) and
before backups.

## Blob store

- **Content-addressed.** The key is SHA-256 of the exact bytes. Path:
  `$DATA/blobs/<first2>/<next2>/<full-hex>`. Same bytes, same path, forever.
- **Immutable.** Blobs are never modified or deleted by the application. A
  retention/deletion policy is an explicit future human decision, not an
  accident of implementation.
- **Deduplicated.** Attaching the same receipt twice stores one blob and two
  attachment rows. `attach_receipt` / `file_document` return
  `deduplicated: true` when the blob already existed.
- **Limits.** 25 MiB per file as an upload (`books.begin_upload`), 256
  KiB given inline. The name and MIME type are the client's, checked
  when the file is filed: a name of 1 to 255 bytes, no `/` or `\`, not
  `.` or `..`, no control or invisible characters; a `type/subtype` of
  visible ASCII, at most 127 bytes (`docs/mcp-interface.md`, "Tools —
  receipts & documents"). The type is stored as given; server-side
  sniffing is future work (only a W-9 is judged by its first bytes).
- **Metadata** lives in SQLite (`blobs` table), not in the filesystem:
  `sha256 PK, size_bytes, mime_type, original_name, created_at`.

## Attachments & documents

- `attachments`: `id PK, blob_sha FK, entry_id FK NULL, category,
  description, created_at`.
  - `entry_id` is nullable: a receipt can be filed before its journal entry
    exists, then linked with `books.link_receipt`.
  - `category` mirrors today's Drive layout: `formation`, `legal`, `tax`,
    `accounting` (today's 01 Formation / 02 Legal / 03 Tax / 04 Accounting),
    plus `receipts`.
- Documents filed via `books.file_document` and receipts via
  `books.attach_receipt` share the blob store; they differ only in metadata.
- **Receipts change by events, never by edits.** `receipt.unlinked`
  (`attachment_id`, `entry_id`, `reason`) detaches a receipt from its entry;
  `attachment.withdrawn` (`attachment_id`, `reason`) retires an unlinked
  receipt or a document (`withdrawn_at`, `withdrawn_reason` columns, schema
  v5). Linking requires an unlinked, non-withdrawn attachment, so evidence
  only leaves an entry through a recorded unlink. Blobs are untouched: a
  withdrawn file is still hash-verified and readable, and the journal shows
  when it was filed, linked, unlinked, and withdrawn, and why.
- The store is policy-free: it records attachments faithfully. Whether a
  posting *requires* a receipt is decided by the MCP layer's receipt policy
  (`docs/mcp-interface.md`), which reads the `attachments` table.

## `ledger-store`: the SQLite projection

SQLite is the read-optimized projection of the journal. Queries, reports,
the MCP server's reads, and the future GUI all hit SQLite — and SQLite is
**disposable by design**: `rebuild` drops it and replays the journal.
Suspected corruption is fixed by rebuild, never by forensics.

- **Location.** `$WANDERBOOKS_DATA/wanderbooks.db`, falling back to
  `~/.local/share/wanderbooks/wanderbooks.db`. The journal lives next to it
  at `$WANDERBOOKS_DATA/journal.jsonl`; blobs under
  `$WANDERBOOKS_DATA/blobs/`. WAL mode on, `synchronous=FULL`,
  `integrity_check` on startup.
- **Schema.** `meta(key, value)` — `schema_version`, `base_currency`,
  `last_applied_seq`; `accounts`, `entries`, `lines`, `blobs`, `attachments`
  as above; `events(seq, type, payload, created_at)` — the applier's output,
  a queryable mirror of the journal stream and the feed the future GUI
  tails. (Poll `WHERE seq > ?`; later SSE over the HTTP transport. This is
  the correct answer to "watch its files to update live" — watch the event
  stream, not the files.)
- **Amounts as TEXT, never REAL.** Unchanged and still load-bearing: a REAL
  column would silently reintroduce binary floats into the money path.
- **Append-only, three layers.** (1) The engine's API never mutates posted
  state. (2) The journal is append-only by file discipline plus the hash
  chain. (3) The applier only ever INSERTs into the projection. The SQLite
  triggers from the earlier draft are deliberately dropped: they protected
  the projection, which is disposable — the journal is the trust anchor, and
  piling mechanisms on the disposable layer adds complexity without adding
  safety. Simplicity is a durability feature too.
- **Migrations.** Numbered SQL files, applied in order, bumping
  `meta.schema_version`. Every migration is tested by rebuilding a
  projection from the journal and comparing reports before/after. A newer
  schema version than the binary understands is a hard error, not a guess.

## Migration from Google Drive

Backfill walks the Drive `the first company` folders and files each
document via the equivalent of `books.file_document` with its category.
The Drive folders stay exactly as they are until the capability-parity
cutover rule (see `docs/capability-parity.md`) is satisfied. Migration is an
implementation task, not part of this spec — but the spec is written so the
migration is a straightforward script, not a redesign.
