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

Reports (`trial_balance`, `balance_sheet`, `profit_and_loss`, `snapshot`)
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
    reports.rs    # trial_balance, profit_and_loss, balance_sheet, snapshot
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
| `ledger-oracle` | Diff harness: our engine vs GnuCash on fixture books | `ledger-core`, `ledger-import` |
| `books-app` | egui UI (native + WASM), command-driven | `ledger-core`, `ledger-store` |

Dependency direction is always inward toward `ledger-core`. Nothing in the
core may depend on a future crate. UI concerns (formatting, locale, widgets)
never leak into the core; the core speaks `Decimal` and ISO codes.

## Why Rust

Exact decimal semantics, exhaustive pattern matching on the five account
types, and a type system that makes invalid states (mutable posted entries,
float money) unrepresentable. The storytold pipeline this project is modeled
on is Rust + egui for the same reasons; when the UI crate arrives it will be
one codebase compiling to native and WASM.
