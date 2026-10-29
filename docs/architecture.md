# Architecture

## The one big idea: engine-first, everything is a command

`ledger-core` owns all accounting knowledge. Every state change is a method
on `Ledger`:

- `add_account` — extend the chart of accounts
- `create_entry` / `add_line` — build a draft journal entry
- `post_entry` — validate and apply (one-way)
- `reverse_entry` — correct via reversal (one-way)

Each command validates *before* it applies, and each failure is a typed
`LedgerError` variant. There is no other mutation path: struct fields are
private, and constructors are `pub(crate)` where it matters. This is what
makes the ledger auditable and what will make the future oracle harness
trustworthy — the harness only needs to drive the command surface.

Reports (`trial_balance`, `balance_sheet`, `unclosed_earnings`, `snapshot`)
are free functions: pure, total, and deterministic. Same ledger in, same
report out, every time, on every platform. That determinism is the entire
basis of `docs/oracle.md`.

## The crates (2026-10-28)

Two Cargo workspaces: the engine and its application in `crates/`, and
the specification in `bookspec/` (which never depends on the engine).

| Crate | Role | Depends on (ours) |
|---|---|---|
| `ledger-core` | the semantics engine: money, chart, journal, `Ledger` commands, reports, periods, assets, schedules, parties, A/R and A/P, payroll, projects, settings and conventions, the kernel and its Kani harnesses | nothing |
| `ledger-vault` | the TIN vault: XChaCha20-Poly1305 records under the book's key | nothing |
| `ledger-config` | the `wanderbooks.toml` codec (adoption in, `books.export_settings` out) and the receipt policy until P4 moves it into the core | `ledger-core` |
| `ledger-store` | the source of truth: the hash-chained JSONL journal, its format gate, the SQLite projection (schema 18; a file per schema), the blob store, backups, the access log, `Snapshot` for readers | `ledger-core`, `ledger-reader` (P6, D-61) |
| `ledger-reader` | the one reader: a PDF's text and pages, an XLSX's cells as exact decimal strings, plain text (P6; a plain library, in-process, D-61); `ledger-import` re-exports it as `extract` | none (`hayro`, `zip`, `quick-xml`) |
| `ledger-import` | formats: QuickBooks CSV in; IIF, XLSX, Typst PDFs (statements, the invoice, labels) out | `ledger-core`, `ledger-reader` |
| `ledger-package` | the shared read-only view over `Store` or `Snapshot` (`PackageBooks`): the CPA package, the 1099 worksheet, the invoice document, Needs attention | `ledger-core`, `ledger-store`, `ledger-config`, `ledger-import`, `ledger-vault` |
| `ledger-mcp` | the books server: the MCP tools (122), resources, the handbook, scopes, the door and the access log, backups' scheduling | all of the above |
| `ledger-host` | the host and its leases, the agent bridge (`wanderbooks mcp`), tokens in 0600 files under the data root (D-63), the owner's commands, uploads and downloads | `ledger-store`, `ledger-mcp`, `ledger-config` |
| `ledger-gui` | the viewer: read-only, live, egui; the Books menu hands work to the server | `ledger-core`, `ledger-store`, `ledger-config`, `ledger-import`, `ledger-package`, `ledger-vault`, `ledger-host` |
| `ledger-cli` | `wanderbooks`, the one executable: every mode (`serve`, `server`, `host`, `mcp`, `gui`, `setup`, `books`, `token`, `connect`, `doctor`, …) and the over-the-wire test suites | everything |
| `bookspec/checker`, `bookspec/runner` | the example schema and parser, the rule catalogue, the `Engine` trait; the per-rule runner | nothing of ours (the adapter in `ledger-core`'s tests depends on them) |

Dependency direction is always inward toward `ledger-core`; nothing in
the core depends on another crate of ours. UI concerns (formatting,
locale, widgets) never leak into the core; the core speaks `Decimal` and
ISO codes. A crate that was planned and never created is not missing:
`ledger-export` became part of `ledger-import`; `ledger-oracle` is the
test `crates/ledger-core/tests/oracle_gnucash.rs` with `oracle/gnucash/`
(`just oracle`); `ledger-view` and `ledger-extract` were closed by D-53
(`PackageBooks` is the shared view; extraction is in-process in
`ledger-import`, P6).

Decided and done (Q-9 → D-61, unit U0d): P6 had made `ledger-store` depend
on `ledger-import` so the writer can extract a file's text at attach time,
which pulled Typst, the fonts and hayro into the store's dependency
closure. The reader now lives in the small library crate `ledger-reader`;
`ledger-store` and `ledger-import` both depend on it (`ledger-import`
re-exports it as `extract`), the store no longer builds the typesetter,
and `ledger-import` still never depends on the store.

`ledger-core` itself (`crates/ledger-core/src/`): `money`, `account`,
`journal`, `ledger` (the command surface and replay), `kernel` (every
money arithmetic, generic over `Amount`) and `proofs` (its Kani
harnesses), `reports`, `cashflow`, `analysis`, `periods`, `assets`,
`schedules`, `recurring`, `payouts`, `payroll`, `form1099`, `tax`,
`classes`, `parties`, `arap`, `settings`, `conventions`, `calendar`,
`shares`, `tax_capital`, `rules`, `error`; its tests in `tests/`
(`invariants.rs`, `properties.rs`, `spec_*.rs`, `kernel_bridge.rs`,
`model.rs`, `bookspec.rs`, `golden/`).

## Dependency rules

`ledger-core` may depend only on:

- `rust_decimal` — exact decimal arithmetic
- `thiserror` — the error type
- `chrono` — dates (`NaiveDate`; no wall-clock reads in the core)
- `serde` / `serde_json` — the oracle interchange format
- `jiff` — the bundled time-zone database behind `company.timezone`
  ("today" is computed by the caller and passed in; the core reads no
  clock)

New dependencies need justification in the commit message. In particular:
no I/O, no networking, no async runtime, no logging framework in the core.
The core is a pure domain library and stays that way.

## Why Rust

Exact decimal semantics, exhaustive pattern matching on the five account
types, and a type system that makes invalid states (mutable posted entries,
float money) unrepresentable. The storytold pipeline this project is modeled
on is Rust + egui for the same reasons; when the UI crate arrives it will be
one codebase compiling to native and WASM.

## The GUI (`ledger-gui`)

Read-only by construction: it opens a `ledger_store::Snapshot` (journal
verified and replayed into its own `Ledger`, projected into a private
in-memory SQLite, blobs read through the verifying store) and never writes
to the data directory. A loader thread watches the directory and applies
appended journal lines; the UI renders an immutable view model rebuilt per
change. Statements and series come from `ledger-core` (`reports`,
`analysis`); the GUI only arranges them. See `plan/gui.md`.

## Shared read-only views (`ledger-package`)

`ledger-package` is the shared read-only view: packages, the invoice PDF,
and Needs attention, over `Store` or `Snapshot`. Its `PackageBooks` trait
is what those builders read of the books (ledger, settings, journal,
attachments and their links and bytes, statements, reconciliations), so the
server and the viewer make the same package, the same invoice, and the same
attention list from one piece of code; nothing in it writes. The viewer's
`attention` module only maps each item's `Target` to its own route, and
`books.start` turns the items into its next steps, so the human's list and
the agent's cannot drift apart. The crate depends inward (`ledger-core`,
`ledger-store`, `ledger-config`, `ledger-import`, `ledger-vault`), never on
`ledger-mcp` or `ledger-gui`.

## The evidence pipeline (`ledger-reader`, `ledger-core`, `ledger-store`, P6)

One extractor lives in `ledger-reader` (`src/lib.rs`, D-53, D-61: in-process,
a plain library): a PDF's text and pages, an XLSX's cells as exact decimal
strings, and plain text; the caps are 25 MiB in, 30 pages and 2 MiB of
text, and a reader panic is caught there. `ledger-store` depends on
`ledger-reader` so the writer can read a file as it is filed and journal
`attachment.text_extracted` (content-addressed text blob, extractor,
version, pages, `ok`/`none`/`failed`), one record per attachment, extractor
and version; `Store::verify` checks the text blob's hash and never
re-derives it. The server's filing scan (`tin_files`) and the viewer's PDF
rasterizer read through the same module, so a PDF's text is read one way.
Annotations and settlements are ledger state (`ledger_core::evidence`,
`Ledger::differs_from`), projected into schema 18's `annotations` table;
extractions have a table of their own. The viewer's Documents page groups
by category and filters by subject; Needs attention reads both through
`PackageBooks`.

**The evidence gate (P4, I-5).** `ledger_core::evidence::requirement`
(with `Policy`, the settings' `policy.evidence.*` read in the core) is the
one decision whether an entry needs a receipt; `Store::post_entry_checked`,
the server's previews and digest, the viewer and Needs attention all call
it, and `ReceiptPolicy` in `ledger-config` keeps only the words. A draft
has a state (`DraftState`, `entry.draft_state_set`) and can be discarded
(`entry.draft_discarded`); `Ledger::posted_seq` is the journal line that
posted each entry. Every MCP tool with `confirmed` takes the digest
`Preview` (`tools/helpers.rs`) its preview returned.

## The journal format gate (`ledger-store`, P0)

Every journal line passes a gate after its parse, seq and chain checks
(`ledger_store::format`): a type this version knows, or a
`journal.format_set` this version reads. A line it can't read stops the
read whole (the writer never opens, the viewer applies none of the new
lines): never skipped, never half-read. A line with a key beside the
envelope's (`seq`, `ts`, `type`, `payload`, `prev`, frozen across formats)
is refused the same way, never read with the key dropped. The refusal is
`NewerJournal`, not `JournalCorrupt`: verified ("line N is intact; this
version can't check the lines after it") when the next line's `prev`,
read leniently whatever format frames that line, matches the refused
line's hash (the SHA-256 of its canonical JSON, every key included);
corruption when a `prev` it can read doesn't match; "newer or damaged"
when there is no next line or no `prev` it can read. The writer
(`Store::open`) and the viewer (`Snapshot`) read the journal the same way
(bytes split on newlines) and share the same gate and settling
(`format::gate`, `format::settle`), so they always give the same answer
for the same journal (fuzzed by `fuzz_store_replay`). The writer checks
every complete line before it changes anything: a torn tail is set aside
only when the gate passes, so an older version never alters newer books.
Rule: `docs/spec/rules.md` B-27.

## How the books are kept, in the books (P1)

Settings, conventions, decisions, open items and approvals are journaled
(journal format 2, declared with `journal.format_set` before the first
such line, so a P0 reader stops there with "update wanderbooks"). The
engine keeps them beside the ledger (`ledger_core::settings::Settings`,
`ledger_core::conventions::Conventions`, fields of `Ledger`), so the store,
its replay and the viewer's `Snapshot` share one reading; `Ledger::
differs_from` and `verify` compare them, and the projection has a table
for each (schema 18 on 2026-10-29; a file per schema). `ledger-config` is the codec between them and a
`wanderbooks.toml` (`codec::import`, `codec::export`): adoption (`wanderbooks
books adopt`) moves an older books dir's file in through it, once, and
`books.export_settings` writes the books' settings out as one. The books
are their settings' only source (D-44): the server (`settings::Live`)
takes them whenever their last `settings.set` changes (on every lock of
the store), the viewer reads them from the journal it shows, and neither
reads a file; "today" is the company's time zone's, set per request. Rule: `docs/spec/rules.md` B-29.

## The MCP server's layout (`ledger-mcp`, P0; U0a)

`crates/ledger-mcp/src/server.rs` is the server itself: `BooksServer`, the
shared store and its lock, `call_tool` and `get_info` (the `ServerHandler`),
`tool_router`, the request machinery and the helpers that cross groups.
Tool handlers live one file per group in `crates/ledger-mcp/src/tools/`,
each declared with `#[path]`, opening with `use super::*`, and holding a
`#[tool_router(router = <group>_router, vis = "pub(super)")] impl
BooksServer`; `server.rs`'s `tool_router` adds every group's router. The
groups are:

- `documents` — invoices, bills, voids, `books.documents`,
  `books.document` and the control check;
- `sent` — `books.mark_sent`, the record that an invoice went out (P4b S5);
- `settlements` — payments that settle one or several documents
  (`books.receive_payment`, `books.pay_bill`), `books.bills_due`, and the
  agings (`books.ar_aging`, `books.ap_aging`) with their tie-out (P4b S6);
- `entries` — `books.draft_entry`, `books.add_line`, `books.post_entry`,
  `books.reverse_entry`, `books.get_entry`, `books.search_entries`, the
  bank CSV import;
- `reports` — the statements, `books.show`, `books.verify`,
  `books.digest`, `books.export`, the CPA package, the P&L by class, and
  the export builders;
- `assets`, `schedules` (schedules and recurring), `periods`,
  `reconciliation`, `payroll`, `payouts`, `tax`;
- `parties` — parties, vendors, customers, roles, W-9s, the TIN vault,
  1099s, entry parties;
- `attachments` — receipts, filed documents, and their links;
- `write_offs` — `books.write_off_invoice`, the bad debt (P4b S7; rules.md
  B-41), kept apart from `documents` so S4.4's document tools and this one
  land in different files;
- `evidence` — finding a file by its hash, number or text, annotating it,
  recording what a payment settles, and consolidating duplicates (P6);
- `settings` — the handbook, `books.settings` and the recorded FX rates
  (the module is `settings_tools`, since `crate::settings` is the live
  settings);
- `chart` — the chart of accounts, the template and CSV import, renaming
  and renumbering, and the control roles;

plus the groups split earlier: `kept` (`books.start`, settings changes,
conventions, decisions, open items, posting templates, approvals),
`projects`, `files` (uploads), `instruments`, `access` and `backup`. A
helper only one group uses lives in that group's file; the ones several
groups share live in `tools/helpers.rs` and `server.rs` re-exports them,
so no group file reaches into another group's. The tests that span the
whole tool surface sit beside `server.rs` in `server_tests.rs`; a group's
own tests live in its group file. The split changed no tool's name,
description, annotations or behaviour.

## The MCP server's store lock (`ledger-mcp`, P0)

One `Store` is shared by every session (`SharedStore`: a
`tokio::sync::Mutex` and a `stopped` flag). Tool bodies run on tokio's
blocking threads (`run_blocking`), never on the async workers, and take the
lock with `blocking_lock`; the guard is `!Send`, so no tool can hold it
across an `.await`. The store journals a command before it changes memory,
so a panic between the two would leave memory behind the journal; and a
tokio mutex never poisons. So the server fails closed: a panic in a
request (its task, or while it holds the guard) sets `stopped`, and every
later request, on every session, is refused until a restart, which
re-verifies the journal.

## bookspec (`bookspec/`, P13)

bookspec is the formal accounting specification (D-34): every rule stated
normatively, sourced, with a symbolic form and concrete examples, and a
conformance harness any engine can be run against. It lives in its own
Cargo workspace inside this repository until the harness is stable enough
to move to its own repository (P13 step 6, D-34, Z-17):

- `bookspec/checker` is engine-independent: the example schema and its
  parser (`Example::from_toml`, every structure `deny_unknown_fields`, the
  cross-field shape checked by `Example::validate`), the rule catalogue
  (`rules()`, `RuleId` `BS-<area>-<n>`, the refusal-to-rule table) and the
  `Engine` trait (`load`, `apply`, `query`, `claims`, and `change` for a
  `[[changes]]` entry, which posts no entry and so is neither a setup nor
  an event).
- `bookspec/runner` drives any `Engine` through the examples and reports
  per rule: pass, fail or not claimed. It compares the engine's answers
  with the examples' exact expected values; it never asks the engine
  whether its own answer is right.
- `bookspec/spec/rules-index.md` maps B-1 to B-45 to the bookspec ids;
  `docs/spec/rules.md` carries each stated rule's id under its heading.
- `crates/ledger-core/tests/bookspec.rs` is the wanderbooks adapter: it
  implements `Engine` over `Store` (a dev-dependency only, so no engine
  crate depends on the specification) and remains the loader that checks
  every figure. `ledger-package` answers the formats appendix's IIF
  queries through the same adapter, as the package test does.

Neither bookspec crate depends on `ledger-core`; the dependency points
inward from the adapter. `just check` runs the bookspec workspace's
format, clippy and tests beside the wanderbooks workspace's, so an example
cannot rot (P13 step 1).

## Self-host: the same server, another account (P9a, D-51)

The books can live on a machine the owner controls (plan 3.8). It is the
same `ledger-mcp` server, the same store and the same tools; only the
process boundary changes. `wanderbooks serve` runs as the dedicated
account `wanderbooks` under `deploy/wanderbooks-server.service`, over a
data root only that account can read (`/srv/wanderbooks`, 0700). Agents
have no account on that machine, so the isolation is enforced by the
operating system rather than advisory: the server is told
`--isolation enforced` (the flag, never a guess) and `books.start`
reports `service_user (enforced)` (`server::Isolation`).

The door is the same HTTP service on a second listener:
`--http 127.0.0.1:7411` binds the MCP routes beside the Unix sockets
(3.9), and the address is loopback only (any other is refused before
anything binds: the door is plaintext with a static bearer, so the TLS
proxy or tunnel belongs in front of loopback); Caddy, Tailscale Funnel or
cloudflared puts TLS and the public name in front of it. The bearer token guards every call exactly as on the
socket; the tunnel only forwards. The socket's peer-credential check
(`OwnerOnly`) is for local callers and does not apply to the loopback
door, which is why the bearer and the loopback-only bind matter there.
The flag travels `serve` → `ledger-host`'s host → `server`
(`Opener::starting_with`, `start::start_host`, `host::Args`), so a host
already running with other flags keeps them; on a self-host machine
`serve` is the unit's only holder.

The unit sets `IPAddressDeny=any` and `IPAddressAllow=localhost`: the
server reaches nothing out. A book whose backup target is a remote git
repository therefore needs that host allowed in a drop-in (D-31 backups
still run on the server). The recipe, the head check for the move and the
two-account CI job are in `deploy/README.md`, `deploy/wanderbooks-server.service`
and `.github/workflows/two-user.yml`; the checks are
`crates/ledger-cli/tests/two_user.rs`.
