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
