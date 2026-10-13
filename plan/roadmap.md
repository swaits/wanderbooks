# Roadmap

## Phase 1 — Semantics core (done)

The engine: money, accounts, journal, ledger commands, trial balance, P&L,
balance sheet, cash flow, fixed assets with depreciation, golden files,
property tests, mutation testing, fuzzing, and the docs an agent needs to
work here. Parity floor: 12 done.

## Phase 2 — AI-first interface (done)

The primary interface is MCP, not a cloned GUI (Stephen's call, 2026-10-10).

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

- `ledger-oracle`: fixture books through our engine and other
  implementations (GnuCash headless first), automatic `snapshot()`
  diffing. Each diff is adjudicated from principles; no other program is
  the authority.
- Parity catalog becomes machine-readable; CI enforces the non-regression
  floor.
- Done when: a fixture book produces byte-identical snapshots from both
  engines with zero manual steps.

## Phase 4 — Read-only GUI (first version done, 2026-10-12)

`ledger-gui`: see `plan/gui.md` for the design and what's built.

- Consumes the `ledger-store` event log: live updates — the answer to
  "watch its files to update live."
- egui or web; a dumb client holding no accounting logic.
- This is the trust layer for humans (Stephen, Kat, the CPA), not the
  primary interface. Read-only at first; editing stays with the agent.
- Design references (Stephen, 2026-10-11; screenshots shared in session,
  described here):
  - **Reports with a date range and a column interval** (QuickBooks
    Desktop's P&L: From/To plus Columns = Year/Quarter/Month/...): any
    statement split into one column per interval plus TOTAL, to see the
    books over time. The multi-column statement layout in
    `ledger-import::report` already renders this shape; the GUI (and the
    exports) need a per-interval builder over the period-scoped core
    statements.
  - **Financial dashboard** (Coupler.io's QuickBooks dashboard): P&L,
    balance sheet, and cash flow overview tables with Last month / Month to
    date / Year to date columns, each beside charts — income vs. COGS +
    expenses vs. earnings bars; stacked assets vs. liabilities + equity;
    cash at end of period; net cash change.
  - **Business overview** (QuickBooks Online home): cards for cash balance
    with a trend, expenses by category (donut), profit and loss (income
    vs. expenses), invoices (unpaid, overdue, paid), sales trend, and bank
    account balances.
  - **Revenue dashboard** (Coupler.io): KPI tiles (paid, overdue, open
    invoices this month), an invoice table (number, customer, dates,
    currency, home and foreign amounts, balance, totals), expenses by
    category by month, invoices by status per month.
  - **QuickBooks Online dashboard**: invoices (unpaid in the last year,
    overdue vs. not yet due; paid in the last 30 days, deposited vs. not),
    expenses by category (donut, last month), bank account balances (book
    vs. bank, items to review), profit and loss (net income, income vs.
    expenses bars), sales trend (line, by quarter).
  - **KPI tiles with comparison** (QuickBooks accounting dashboard): income,
    cost of goods sold, expenses, net profit, each with % change vs. the
    previous 30 days; an auto date-range picker; cash balance over time
    with a trend line; revenue by month, stacked.
  - **Cash-flow dashboard**: bank balance, debtors, burn rate, cash runway
    (days) tiles; income vs. expenses for the last six months; a net cash
    flow forecast with a confidence band; A/P and A/R aging by days past
    due (the engine already computes both agings); total burn over time.

## Phase 5 — Workflow parity

- A/R and A/P workflows, bank reconciliation, budgets, scheduled transactions.
- Multi-currency with FX translation.
- Period close.
- The goal: a self-hosted bookkeeping system with a tested claim to
  proved accounting semantics (cross-checked against GnuCash and others)
  and capability parity with the old
  GSheet+GDrive workflow — no SaaS subscription required.

## Explicitly not on the roadmap

Bank feeds, payment processing, payroll, tax filing. Those are integrations
and regulated domains, not cloning targets. The engine gets clean seams for
them; someone else's API provides them.
