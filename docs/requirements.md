# Requirements — accounting semantics (Phase 1)

Numbered requirements for the engine. `R-N` is current scope; `R-FUTURE-N`
is explicitly out of scope for now but designed for. Agents: if code and a
requirement conflict, the requirement wins.

## Core double-entry

- **R-1 — Balanced entries.** Every journal entry has at least two lines and
  total debits exactly equal total credits (`Decimal` equality, no epsilon).
  Unbalanced entries are rejected at post time with the offending totals.
- **R-2 — Exact arithmetic.** All money is `Decimal`. No binary floats in any
  money path. No silent rounding: amounts are exact end to end. (Presentation
  rounding, when the UI arrives, is half-up to the currency's minor unit and
  is a display concern only.)
- **R-3 — Immutability.** Posted entries cannot be edited or deleted. The only
  correction mechanism is a reversing entry (same lines, sides flipped)
  linked via `reversal_of`. Draft → Posted is one-way.
- **R-4 — Chart of accounts.** Five closed types (Asset, Liability, Equity,
  Revenue, Expense) with fixed normal sides (debit-normal: Asset, Expense;
  credit-normal: Liability, Equity, Revenue). Codes are unique and non-empty.
  Accounts may have parents (hierarchy). Placeholder (header) accounts
  organize the chart and reject postings.
- **R-5 — Single currency per ledger (day one).** Every account and every
  journal line uses the ledger's base currency. Anything else is rejected,
  not converted. Multi-currency with translation is R-FUTURE-1.
- **R-6 — Line hygiene.** Zero-amount lines are rejected, not ignored.
  Unknown accounts are rejected. Single-line entries are rejected.

## Reports

- **R-7 — Trial balance.** Every non-zero, non-placeholder account appears
  with its debit or credit balance, sorted by code. Total debits must equal
  total credits (the `balanced()` check).
- **R-8 — Balance sheet.** Assets, liabilities, equity grouped by account
  type. The accounting equation holds exactly: assets = liabilities + equity.
  Because period close (R-FUTURE-2) doesn't exist yet, current earnings (net
  income to date) are folded into equity so the equation holds on a live book.
- **R-9 — Profit and loss.** Revenue and expenses for the life of the ledger,
  with net income. Period filtering arrives with R-FUTURE-2.
- **R-10 — Canonical snapshots.** `reports::snapshot()` renders the three
  core reports as deterministic JSON (decimal strings, sorted lines, fixed
  keys). Two engines fed the same books must produce identical output. This
  is the oracle interchange format.

## Auditability

- **R-11 — Append-only log.** Posted entries are kept in posting order. The
  log plus the chart fully determine every report.
- **R-12 — Command surface.** All mutations go through `Ledger` methods.
  There is no second way to change the books.

## Receipt policy (MCP phase)

- **R-13 — Receipt enforcement.** The MCP layer (not the engine — the engine
  stays policy-free) enforces a configurable receipt policy: postings at or
  above a threshold (`require_receipts_above`, default `"25.00"` in base
  currency) must have at least one attached receipt, with per-account
  overrides. `post_entry` with `confirmed: true` refuses with a typed
  `receipt_required` error otherwise. The draft preview always includes the
  receipt checklist (`attached: [...]`, `required_but_missing: bool`), so the
  human sees it before confirming. Full spec: `docs/mcp-interface.md`.

## Explicitly future work

- **R-FUTURE-1 — Multi-currency.** Per-account currencies, entry-level FX
  rates, translation gains/losses, reporting in a presentation currency.
- **R-FUTURE-2 — Periods and close.** Fiscal years/periods, closing entries
  that move P&L balances to retained earnings, locked periods.
- **R-FUTURE-5 — A/R and A/P workflows** (invoices, bills, payments, aging).
- **R-FUTURE-6 — Budgets and scheduled transactions.**

## Promoted from future (done)

- **R-? — Cash flow statement** (indirect method). Was R-FUTURE-3.
  `ledger-core::reports::cash_flow` + `books.cash_flow`.
- **R-? — QuickBooks-compatible import/export.** Was R-FUTURE-4.
  `ledger-import` crate: CSV bank/chart import, IIF and XLSX export.

## Non-goals (all phases of the engine)

The engine never does I/O, networking, wall-clock time sourcing, or UI. It is
a pure domain library: data in, validated books out.
