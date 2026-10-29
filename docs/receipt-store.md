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
  is for a human, never repaired automatically. The server does better
  (plan 3.7): it commits the journal, the blobs, the vault, its key and
  the access log to each book's own git repository (the blobs staged
  ahead, the commit under the store's lock) and pushes it to the
  owner's private repository and backup folder, versioned and
  deduplicated, with full history (docs/mcp-interface.md, "Tools —
  backups").
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
- `type`: `account.added`, `entry.posted`, `entry.reversed`, `invoice.created`,
  `bill.created`, `invoice.sent` (a recorded send: a document's record,
  never a ledger line or a figure; P4b S5), `invoice.written_off` (P4b S7),
  `receipt.attached`, `receipt.linked`, `receipt.unlinked`,
  `document.filed`, `attachment.withdrawn`, `attachment.text_extracted`,
  `attachment.annotated`, `entry.settles` (P6), `entry.draft_state_set`
  (a draft building or awaiting its evidence: a note on the draft, no
  figure) and `entry.draft_discarded` (a draft thrown away, with why; the
  draft's earlier lines stay in the journal) (P4, feature `drafts_v2`),
  and `convention.source_revised {id, source, reason}`,
  `decision.source_revised {id, source: {decided_by: [..], decided_on,
  citation?}, reason}` and `open_item.source_revised {id, source, reason}`
  (P5, feature `source_revisions`: who decided a convention, a decision or
  an open item, when and on what, corrected without touching its words;
  replay sets the source and nothing else, and the projection updates the
  row's existing source columns, so no schema changed)
  (plus the A/R, A/P, asset,
  period, payroll, and class events listed on `JournalLine`).
- `payload`: the full domain facts (account fields, entry with lines, blob
  sha, ...). Everything the applier needs; nothing it must look up elsewhere.
  An `invoice.created` or `bill.created` written after the `documents_v2`
  upgrade (plan P4b S4.3) carries only the new fields it names, so a
  document naming none is byte for byte the payload every earlier version
  wrote: the invoice's `number`, `terms`, `bill_to` and `seller` snapshots
  (as they were at issue), `customer_ref` and `memo`; the bill's
  `vendor_ref`, `received_on`, `terms`, `memo` and the `duplicate_reason`
  that allowed a duplicate of a standing vendor invoice; and each line's
  `account`, `quantity` and `unit_price`. The issued copy of an invoice is
  **not** a field on the event (its attachment id is allocated after the
  entry exists): it is a `receipt.attached` on the invoice's own entry
  (`INV-0007.pdf`, `application/pdf`, "Issued invoice"), its SHA-256 the
  `blob_sha`, linked to the invoice with `attachment.linked` and found
  through that link (plan 8.3.11). An `invoice.written_off` (plan 8.3.6,
  P4b S7) carries `{invoice_id, entry_id, date, amount, reason}`; replay
  tags the entry's receivables line `WriteOff`, and the projection keeps
  no written-off figure either: it is the tagged lines' sum (rules.md
  B-38, B-41). Its evidence of worthlessness rides beside it, as a
  `receipt.attached` on the write-off's entry and an `attachment.linked`
  to the invoice, filed in the same call.
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
Any mismatch is a loud failure. Run it on startup (cheap at this scale);
a backup is then proven where it lands, by a drill (docs/mcp-interface.md,
"Tools — backups").

## Blob store

- **Content-addressed.** The key is SHA-256 of the exact bytes. Path:
  `$DATA/blobs/<first2>/<next2>/<full-hex>`. Same bytes, same path, forever.
- **Immutable.** Blobs are never modified or deleted by the application. A
  retention/deletion policy is an explicit future human decision, not an
  accident of implementation.
- **Deduplicated.** The same bytes are one blob, and since P6 one live
  attachment record: filing them again returns that record (with
  `deduplicated: true`) and links it to the new subject instead of adding
  a second row. Records an older version filed twice stay as written until
  `books.consolidate_duplicates` links the oldest to every subject the
  others had and withdraws the rest (`duplicate_of`).
- **Limits.** 25 MiB per file, as an upload (`books.begin_upload`): the
  only way a file comes in (D-46: nothing is given inline). The name and MIME type are the client's, checked
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
    plus `receipts`, and every other company record (D-20): `statement`,
    `contract`, `photo`, `warranty`, `import_source`, `notices`,
    `correspondence`, `plans`, `history`, `other`.
  - `duplicate_of` names the attachment a withdrawn duplicate stands for
    (`attachment.withdrawn`'s optional `duplicate_of`, schema v16).
- Documents filed via `books.file_document` and receipts via
  `books.attach_receipt` share the blob store; they differ only in metadata.
- **Receipts change by events, never by edits.** `receipt.unlinked`
  (`attachment_id`, `entry_id`, `reason`) detaches a receipt from its entry;
  `attachment.withdrawn` (`attachment_id`, `reason`, `duplicate_of?`)
  retires an unlinked receipt or a document (`withdrawn_at`,
  `withdrawn_reason`, `duplicate_of` columns, schema v5 and v16). Linking requires an unlinked, non-withdrawn attachment, so evidence
  only leaves an entry through a recorded unlink. Blobs are untouched: a
  withdrawn file is still hash-verified and readable, and the journal shows
  when it was filed, linked, unlinked, and withdrawn, and why.
- **Extraction is derived, and journaled** (P6, rules.md B-44):
  `attachment.text_extracted {attachment_id, text_sha256, extractor,
  version, pages, status}` records what reading a file found (`ok`,
  `none`, `failed`); the text itself is a content-addressed blob like any
  other, and `books.verify` checks its presence and hash, never re-derives
  it. One record per attachment, extractor and version: an extractor
  upgrade adds a record, never replaces one.
- **Annotations and settlements are records, not edits** (P6):
  `attachment.annotated {attachment_id, fields, note?}` is what a human
  read off a file (the closed field list; a later annotation replaces the
  earlier record), and `entry.settles {entry_id, settles[]}` says which
  entries a payment pays back (the only source of "reimbursed").
- The store is policy-free: it records attachments faithfully. Whether a
  posting *requires* a receipt is decided by the MCP layer's receipt policy
  (`docs/mcp-interface.md`), which reads the `attachments` table.

## `ledger-store`: the SQLite projection

SQLite is the read-optimized projection of the journal. Queries, reports,
the MCP server's reads, and the future GUI all hit SQLite — and SQLite is
**disposable by design**: `rebuild` drops it and replays the journal.
Suspected corruption is fixed by rebuild, never by forensics.

- **Location.** `$WANDERBOOKS_DATA/wanderbooks.v{N}.db`, falling back to
  `~/.local/share/wanderbooks/wanderbooks.v{N}.db`, N the schema: each
  schema keeps a file of its own, and a version never migrates, writes or
  deletes another schema's (`ledger_store::db_file_for`; the legacy
  `wanderbooks.db` is used only while it holds this version's schema;
  rules.md B-27). The journal lives next to it
  at `$WANDERBOOKS_DATA/journal.jsonl`; blobs under
  `$WANDERBOOKS_DATA/blobs/`. WAL mode on, `synchronous=FULL`,
  `integrity_check` on startup.
- **Schema.** `meta(key, value)` — `schema_version`, `base_currency`,
  `last_applied_seq`; `accounts`, `entries`, `lines`, `blobs`, `attachments`
  as above; `extractions(attachment_id, extractor, version, text_sha256,
  pages, status, seq)` and `annotations(attachment_id, receipt_number,
  payee, amount, date, instrument, category, source, reimbursable_to,
  note, seq)` (P6; amounts TEXT, never REAL); `events(seq, type, payload,
  created_at)` — the applier's output,
  a queryable mirror of the journal stream and the feed the future GUI
  tails. (Poll `WHERE seq > ?`; later SSE over the HTTP transport. This is
  the correct answer to "watch its files to update live" — watch the event
  stream, not the files.)
- **Document fields.** Since schema v16 (plan P4b S4.3) the documents'
  new fields are columns: `invoices(number, terms, bill_to, seller,
  customer_ref, memo)` and `bills(vendor_ref, received_on, terms, memo,
  duplicate_reason)`, with terms and snapshots as canonical JSON TEXT and a
  document written before them NULL there. A line's `account`, `quantity`
  and `unit_price` ride its document's `lines_json`, as its project does.
  The party's email and default terms are journaled and replayed into the
  ledger; the projection keeps no party table to hold them.
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

## Migration from Google Drive (done)

The company's records were filed into the books at the P1 cutover
(2026-10-17, D-20: formation papers, notices, plans and history, through
`books.file_document` with their categories) and the books cut over on
2026-10-20. The Drive manifest and comparison tools once planned for P6
were struck (`plan/served.md` 8.4): the Drive folder is frozen history.
P6 (on its branch) adds the categories `notices`, `correspondence`,
`plans`, `history` and text extraction for every filed document.
