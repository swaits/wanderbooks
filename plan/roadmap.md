# Roadmap

## Phase 1 — Semantics core (done)

The engine: money, accounts, journal, ledger commands, trial balance, P&L,
balance sheet, golden files, property tests, and the docs an agent needs to
work here (AGENTS.md, requirements, architecture, oracle, parity, glossary).
Parity floor: 6 done.

## Phase 2 — AI-first interface (now)

The primary interface is MCP, not a cloned GUI (the owner's call, 2026-10-10).

- `ledger-store`: SQLite persistence, append-only triggers, amounts as TEXT,
  event log, migrations, rebuild-from-log verify. Spec: `docs/receipt-store.md`.
- Blob store: content-addressed receipts and documents, attachment metadata,
  Drive backfill script.
- `ledger-mcp`: stdio transport first, Streamable HTTP later; narrow tools,
  preview-before-post, digest, read-only resources, the agent operating
  contract in the tool descriptions. Spec: `docs/mcp-interface.md`.
- Capability parity vs the GSheet+GDrive system: `docs/capability-parity.md`.
- Parallel run with the sheet; cutover per the cutover rule.
- Done when: every specced capability-parity row is covered, a month of
  parallel run diffs clean, the owner approves the cutover.

## Phase 3 — Oracle harness

- `ledger-import`: GnuCash XML read-only import
- `ledger-oracle`: fixture books through our engine and GnuCash headless,
  automatic `snapshot()` diffing via `cargo xtask oracle`
- Parity catalog becomes machine-readable; CI enforces the non-regression
  floor
- Done when: a fixture book produces byte-identical snapshots from both
  engines with zero manual steps

## Phase 4 — Read-only GUI

- Consumes the `ledger-store` event log: live updates via tail/SSE — the
  answer to "watch its files to update live."
- egui or web; a dumb client holding no accounting logic.
- This is the trust layer for humans (the owner, the other member, the CPA), not the
  primary interface. Read-only at first; editing stays with the agent.

## Phase 5 — Workflow parity

- A/R and A/P workflows, bank reconciliation, budgets, scheduled transactions
- Multi-currency with FX translation
- Period close and cash flow (engine R-FUTURE items) land as their phases
  demand, not before
- The goal: a self-hosted bookkeeping system with a tested claim to
  semantic parity with GnuCash and capability parity with the old
  GSheet+GDrive workflow — no SaaS subscription required

## Explicitly not on the roadmap

Bank feeds, payment processing, payroll, tax filing. Those are integrations
and regulated domains, not cloning targets. The engine gets clean seams for
them; someone else's API provides them.
