# Roadmap

## Phase 1 — Semantics core (now)

The engine: money, accounts, journal, ledger commands, trial balance, P&L,
balance sheet, golden files, property tests, and the docs an agent needs to
work here (AGENTS.md, requirements, architecture, oracle, parity, glossary).
Done when the parity floor is 6+ and every `done` item is tested +
golden-covered.

## Phase 2 — Close the accounting loop

- Period close (fiscal periods, closing entries, locked periods)
- Cash flow statement, indirect method
- `proptest` with shrinking; properties for close and cash flow
- Parity floor moves; goldens grow

## Phase 3 — Oracle harness

- `ledger-import`: GnuCash XML read-only import
- `ledger-oracle`: fixture books through our engine and GnuCash headless,
  automatic `snapshot()` diffing via `cargo xtask oracle`
- Parity catalog becomes machine-readable; CI enforces the non-regression
  floor
- Done when: a fixture book produces byte-identical snapshots from both
  engines with zero manual steps

## Phase 4 — App shell (egui)

- `books-app`: one Rust codebase, native desktop + WASM (the storytold
  stack: egui/eframe, no Electron, no React)
- Command-driven UI: every UI action goes through the `Ledger` command
  surface; the UI holds no accounting logic
- storytold's screenshot-in-the-loop returns: agents verify UI changes
  visually, against reference screenshots of GnuCash/QuickBooks kept
  maintainer-local (never committed)
- Workflow parity catalog starts (QuickBooks/Xero reference): invoices,
  bills, bank rec, chart management

## Phase 5 — Workflow parity

- A/R and A/P workflows, bank reconciliation, budgets, scheduled transactions
- Multi-currency with FX translation
- The goal: a self-hosted bookkeeping app with a tested claim to
  semantic parity with GnuCash and workflow parity with QuickBooks —
  no SaaS subscription required

## Explicitly not on the roadmap

Bank feeds, payment processing, payroll, tax filing. Those are integrations
and regulated domains, not cloning targets. The engine gets clean seams for
them; someone else's API provides them.
