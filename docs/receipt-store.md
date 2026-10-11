# Receipt & document store + persistence spec

Two coupled pieces: where the bytes live (content-addressed blob store) and
where the facts live (SQLite via `ledger-store`). Both are immutable-first,
matching the ledger's philosophy: the books never forget, and neither does
the filing cabinet.

## Blob store

- **Content-addressed.** The key is SHA-256 of the exact bytes. Path:
  `$DATA/blobs/<first2>/<next2>/<full-hex>`. Same bytes, same path, forever.
- **Immutable.** Blobs are never modified or deleted by the application. A
  retention/deletion policy is an explicit future human decision, not an
  accident of implementation.
- **Deduplicated.** Attaching the same receipt twice stores one blob and two
  attachment rows. `attach_receipt` / `file_document` return
  `deduplicated: true` when the blob already existed.
- **Limits.** 25 MB per file by default, configurable. MIME type is accepted
  from the client and stored; server-side sniffing is future work.
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
- The store is policy-free: it records attachments faithfully. Whether a
  posting *requires* a receipt is decided by the MCP layer's receipt policy
  (`docs/mcp-interface.md`), which reads the `attachments` table.

## `ledger-store` (SQLite)

- **Location.** `$WANDERBOOKS_DATA/wanderbooks.db`, falling back to
  `~/.local/share/wanderbooks/wanderbooks.db`. Blobs live next to it under
  `$WANDERBOOKS_DATA/blobs/`. WAL mode on.
- **Schema.**
  - `meta(key TEXT PK, value TEXT)` — `schema_version`, `base_currency`.
  - `accounts(id INTEGER PK, code TEXT UNIQUE, name TEXT, type TEXT,
    parent INTEGER NULL, placeholder INTEGER, currency TEXT)`.
  - `entries(id INTEGER PK, date TEXT, description TEXT, status TEXT,
    reversal_of INTEGER NULL)`.
  - `lines(id INTEGER PK, entry_id INTEGER, account_id INTEGER, side TEXT,
    amount TEXT, currency TEXT)`. **Amounts are TEXT (decimal strings).
    Never REAL.** This is load-bearing: a REAL column would silently
    reintroduce binary floats into the money path.
  - `blobs`, `attachments` as above.
  - `events(seq INTEGER PK AUTOINCREMENT, type TEXT, payload TEXT,
    created_at TEXT)` — appended on every mutation: account added, entry
    posted, entry reversed, receipt attached, document filed. Payload is JSON.
- **Append-only enforcement.** Application code never UPDATEs or DELETEs
  `entries`/`lines`/`blobs`. Belt and suspenders: SQLite triggers abort any
  such statement:
  ```sql
  CREATE TRIGGER no_update_entries BEFORE UPDATE ON entries BEGIN
    SELECT RAISE(ABORT, 'entries are immutable; post a reversing entry');
  END;
  ```
  Same shape for `lines` (UPDATE and DELETE) and `blobs`.
- **The event log is the live-update mechanism.** The future read-only GUI
  does not poll the database and does not watch the filesystem: it tails
  `events` (poll `WHERE seq > ?`, later SSE over the HTTP transport). This
  is the correct answer to "watch its files to update live" — watch the
  event stream, not the files.
- **Migrations.** Numbered SQL files, applied in order, bumping
  `meta.schema_version`. The store opens read-write only after migrations
  are current; a newer schema version than the binary understands is a
  hard error, not a guess.

## Rebuildability

The database is a cache, not the source of truth. The source of truth is:
the append-only entry log + the blob store. `ledger-store` must be able to
rebuild every report from those two. (A `verify` subcommand that replays the
log and compares against live reports is a good acceptance test for the
crate.)

## Migration from Google Drive

Backfill walks the Drive `Wanderland Labs LLC` folders and files each
document via the equivalent of `books.file_document` with its category.
The Drive folders stay exactly as they are until the capability-parity
cutover rule (see `docs/capability-parity.md`) is satisfied. Migration is an
implementation task, not part of this spec — but the spec is written so the
migration is a straightforward script, not a redesign.
