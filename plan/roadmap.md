# Roadmap

## Phase 1 — Semantics core (done)

The engine: money, accounts, journal, ledger commands, trial balance, P&L,
balance sheet, cash flow, fixed assets with depreciation, golden files,
property tests, mutation testing, fuzzing, and the docs an agent needs to
work here. Parity floor: 12 done.

## Phase 2 — AI-first interface (done)

The primary interface is MCP, not a cloned GUI (the owner's call, 2026-10-10).

- `ledger-store`: append-only hash-chained JSONL journal (source of truth),
  SQLite as disposable projection with auto-rebuild, content-addressed
  blob store, durable drafts.
- `ledger-mcp`: 27 tools over stdio and Streamable HTTP, preview-before-post,
  server-side receipt policy (>$25) with anti-splitting, asset management,
  digest for human review, read-only resources.
- `ledger-import`: QuickBooks CSV import (bank, chart), IIF and XLSX export.
- Capability parity 12 of 12 vs the GSheet+GDrive system.
- Parallel run with the sheet; cutover per the cutover rule
  (`docs/parallel-run.md`).

## Phase 3 — Oracle harness (next)

- `ledger-oracle`: fixture books through our engine and GnuCash headless,
  automatic `snapshot()` diffing.
- Parity catalog becomes machine-readable; CI enforces the non-regression
  floor.
- Done when: a fixture book produces byte-identical snapshots from both
  engines with zero manual steps.

## Phase 4 — Read-only GUI

- Consumes the `ledger-store` event log: live updates — the answer to
  "watch its files to update live."
- egui or web; a dumb client holding no accounting logic.
- This is the trust layer for humans (the owner, the other member, the CPA), not the
  primary interface. Read-only at first; editing stays with the agent.

## Phase 5 — Workflow parity

- A/R and A/P workflows, bank reconciliation, budgets, scheduled transactions.
- Multi-currency with FX translation.
- Period close.
- The goal: a self-hosted bookkeeping system with a tested claim to
  semantic parity with GnuCash and capability parity with the old
  GSheet+GDrive workflow — no SaaS subscription required.

## Explicitly not on the roadmap

Bank feeds, payment processing, payroll, tax filing. Those are integrations
and regulated domains, not cloning targets. The engine gets clean seams for
them; someone else's API provides them.
