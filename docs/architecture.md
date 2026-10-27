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

## Current crate layout

```
crates/ledger-core/
  src/
    lib.rs        # crate docs, re-exports, design rules
    money.rs      # Currency + Money (Decimal, checked arithmetic)
    account.rs    # AccountType, Side, Account, ChartOfAccounts
    journal.rs    # JournalLine, JournalEntry, validate()
    ledger.rs     # Ledger: the command surface, append-only log
    reports.rs    # trial_balance, unclosed_earnings, balance_sheet, snapshot
  tests/
    invariants.rs # accounting-law tests + randomized property test
    golden/       # hand-verified expected report snapshots
```

## Dependency rules

`ledger-core` may depend only on:

- `rust_decimal` — exact decimal arithmetic
- `thiserror` — the error type
- `chrono` — dates (`NaiveDate`; no wall-clock reads in the core)
- `serde` / `serde_json` — the oracle interchange format

New dependencies need justification in the commit message. In particular:
no I/O, no networking, no async runtime, no logging framework in the core.
The core is a pure domain library and stays that way.

## Future crates (do not create until the roadmap says so)

| Crate | Purpose | Depends on |
|---|---|---|
| `ledger-store` | Journal (hash-chained JSONL) + SQLite projection, blob store | `ledger-core` |
| `ledger-import` | QuickBooks CSV read-only import (bank transactions, chart of accounts) | `ledger-core` |
| `ledger-export` | Format renderers: CSV, XLSX, IIF journal entries (QuickBooks Desktop) | `ledger-core` |
| `ledger-oracle` | Cross-check harness: our engine vs other implementations (GnuCash first) on fixture books; diffs are adjudicated, not obeyed | `ledger-core`, `ledger-import` |

Dependency direction is always inward toward `ledger-core`. Nothing in the
core may depend on a future crate. UI concerns (formatting, locale, widgets)
never leak into the core; the core speaks `Decimal` and ISO codes.

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
for each (schema v12). `ledger-config` is the codec between them and a
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

- `documents` — invoices, bills, payments, voids, `books.documents`,
  `books.document`, the control check and the agings;
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
