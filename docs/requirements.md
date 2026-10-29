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
  not converted, unless the line records its foreign currency, amount, and
  rate (R-FUTURE-1, done: see below).
- **R-6 — Line hygiene.** Zero-amount lines are rejected, not ignored.
  Unknown accounts are rejected. Single-line entries are rejected.

## Reports

- **R-7 — Trial balance.** Every non-zero, non-placeholder account appears
  with its debit or credit balance, sorted by code. Total debits must equal
  total credits (the `balanced()` check).
- **R-8 — Balance sheet.** Assets, liabilities, equity grouped by account
  type. The accounting equation holds exactly: assets = liabilities + equity.
  Current earnings (revenue minus expenses not yet closed to retained
  earnings) are folded into equity so the equation holds on a live book.
  `balance_sheet_as_of` gives the statement as of any date, a line per
  account, counting every entry dated on or before it (closing entries
  included: after a close the earnings sit in retained earnings).
- **R-9 — Profit and loss.** Revenue and expenses with net income.
  `profit_and_loss_for_period` scopes it to a date range (optionally one
  class), a line per account, and **excludes closing entries**: a close moves
  earnings into retained earnings and must not make a closed period read as
  break-even. The cash flow statement excludes closing entries for the same
  reason. Period P&Ls partition: split a range anywhere and the halves sum
  to the whole, and a range's net income equals the change in current
  earnings across it (property-tested).
- **R-9a — General ledger.** `general_ledger(start, end)`: by account in
  code order, each with its beginning balance, every posting in the period
  (date, entry, description, debit, credit, running balance on the account's
  normal side), and its ending balance, which ties to the trial balance as
  of `end`. Each posting names its *split*: the one account on the entry's
  opposite side, or "-Split-" when that side has several (the QuickBooks
  convention). The entry-ordered listing is the *general journal*, a
  different report.
- **R-9b — Trial balance as of a date.** `trial_balance_as_of(as_of)`.
- **R-9c — Statement of changes in members' equity.** For a multi-member LLC
  (partnership): each member's capital roll-forward — beginning,
  contributions (credits to their capital account), allocated net income
  (loss), distributions (debits), ending — plus a total that equals
  balance-sheet equity. Earnings not yet distributed (retained earnings and
  unclosed income) and the period's income are allocated by ownership share
  to the currency's minor unit by largest remainder (the odd cent goes to
  the largest remainder, ties to the earlier member; the owner, 2026-10-13),
  earnings not yet closed the way their close will split them (D-38,
  rules.md B-13), so every row's total is exact and a member's capital on a
  day never changes when its month is closed. Any equity change the roll-forward doesn't explain is shown as
  "other", never hidden.
- **R-9d — Tax categories.** Revenue and expense accounts map to stable
  tax categories (`ledger_core::tax::CATEGORIES`: key, label, income /
  cost of goods / deduction, the returns it appears on — Schedule C, 1065,
  1120-S, 1120 — default deductible percent, guidance), with an optional
  per-account deductible percent. `tax_summary(start, end, form)` groups
  the P&L by category with book and deductible amounts and lists
  uncategorized accounts. Book basis only: no tax depreciation, basis, or
  filing; not tax advice.
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

- **R-13a — Receipt corrections.** Posted entries are immutable (R-3); their
  evidence is not frozen but is append-only. A receipt is replaced by
  attaching the new one and unlinking the old with a reason; unwanted
  receipts and documents are withdrawn with a reason. Unlinking may never
  leave an entry the policy (R-13) requires a receipt for with none. Every
  step is a journal event; no blob is deleted.
- **R-13 — Receipt enforcement.** The server (not the engine — the engine
  stays policy-free) enforces the books' evidence policy: postings at or
  above a threshold (the journaled setting `policy.evidence.threshold`,
  with required and exempt accounts; P1, rules.md B-29) must have at least
  one attached receipt. Evaluated by `ledger-config`'s `ReceiptPolicy`
  today; P4 moves the evaluation into `ledger_core::evidence` so the
  store, the server and the viewer call one function (plan I-5, D-47:
  reversals, closes, schedule and depreciation runs are exempt by origin). `post_entry` with `confirmed: true` refuses with a typed
  `receipt_required` error otherwise. The draft preview always includes the
  receipt checklist (`attached: [...]`, `required_but_missing: bool`), so the
  human sees it before confirming. Full spec: `docs/mcp-interface.md`.

## Explicitly future work

- **R-FUTURE-6 — Budgets.** Explicitly declined as bloat. (Scheduled
  transactions, the other half of R-FUTURE-6, are done: see below.)

## Promoted from future (done)

- **R-? — Cash flow statement** (direct method, with the reconciliation
  of net income to operating cash and the noncash investing and financing
  activities; rules.md B-14, P3 defect 1). Was R-FUTURE-3.
  `ledger-core::reports::cash_flow` + `books.cash_flow`.
- **R-? — QuickBooks-compatible import/export.** Was R-FUTURE-4.
  `ledger-import` crate: CSV bank/chart import, IIF and XLSX export.
- **R-? — Multi-currency (foreign-currency lines).** Was R-FUTURE-1.
  `JournalLine` carries optional foreign currency/amount; `amount` is the
  base-currency equivalent. `Ledger::add_line_fx`, `Ledger::set_fx_rate`.
  Reports work in base currency. (Full per-account currencies and translation
  remain future.)
- **R-? — Periods and close.** Was R-FUTURE-2.
  `ledger-core::periods`: `create_period`, `close_period_to(id, to)`; closed
  periods reject new entries. The closing entry, dated at period end,
  zeroes each revenue and expense account's balance **as of the period
  end** (later-dated entries belong to later periods) and puts the net in
  `to`: `CloseTo::RetainedEarnings` (`3999`, the corporate convention;
  `close_period` is shorthand) or `CloseTo::Members` (each member's capital
  account by share, to the currency's minor unit, odd units by largest
  remainder, cumulatively from the first members close with these members
  and shares, so no member is ever a unit from their exact share: the
  partnership convention, rules.md B-13; closes journaled under earlier
  rules replay with the rule they were made under: `CloseTo::MembersRun`,
  the run rule of 2026-10-13 to P3, `CloseTo::MembersPerClose`, and
  before 2026-10-13 the last member absorbing rounding), or `CloseTo::Account` (any equity account, e.g.
  a sole proprietor's capital). Every target can sweep owner-pay equity
  accounts — draws, distributions, dividends — into it: a member's
  distributions account into their own capital. The members' equity
  statement counts a member's distributions account as part of their
  capital. Periods close in order (`CloseOutOfOrder`). The
  journal records the target, and replay re-derives the closing entry and
  checks every line against the journal.
- **R-? — Prepaid expenses, deferred revenue, and recurring entries.** Was
  part of R-FUTURE-6 ("scheduled transactions"). Amortization schedules
  post monthly, equal to the cent, idempotently, honoring closed periods;
  the journal records each posting with its full entry and replay checks
  it. Recurring entries generate drafts only, never posts, so the receipt
  policy always applies.
- **R-? — Payout gross-up.** Platform deposits are recorded gross:
  revenue, fees, and adjustments, reconciled to the deposit to the cent,
  as a draft that carries the payout statement as its receipt.
- **R-? — 1099 tracking.** Vendors carry the information return they get
  and whether a W-9 is on file (never the TIN). Entries carry the vendor
  they paid; bills, bill payments, and reversals carry it automatically.
  The year's worksheet counts cash paid (asset-account credits minus
  debits), so card payments are excluded, against a year-dependent,
  overridable threshold. Schema v7.
- **R-? — A/R and A/P workflows.** Was R-FUTURE-5.
  `ledger-core::arap`: customers, vendors, invoices, bills, payments,
  AR/AP aging. MCP tools for the full workflow. Since P4b S1 (D-45,
  rules.md B-38): a document's figures are the signed sum of its tagged
  ledger lines dated on or before the day asked about, so reversals,
  voids and write-offs are right by construction; a hand line on a
  control account is refused; `books.control_check` ties the subledger to
  the control account. S4 to S7 add terms, numbers, line accounts, the
  issued PDF, sends, settlements with discounts and the processor's fee,
  and bad debts, behind the `documents_v2` journal feature.
- **R-? — Bank reconciliation.** Cleared flags on posted lines,
  `reconciliation_report` with uncleared-line detail.
- **R-? — Payroll (accounting only).** `ledger-core::payroll`:
  `run_payroll` posts one balanced entry (Dr 6100 gross, Dr 6110 employer
  FICA, Cr 2110–2150 payables, Cr cash net); `remit_payroll_taxes` clears
  the payables. No tax tables — the caller supplies every number.
- **R-? — Tracking classes (divisional P&L).** `ledger-core::classes`:
  `JournalLine.class_id`, `add_class`, `set_line_class` (drafts only),
  `profit_and_loss_filtered`, `profit_and_loss_by_class`. Shared costs are
  split explicitly across lines at entry time.
- **R-? — CPA handoff package.** `books.export_cpa_package` (and the
  viewer's Export menu, D-29): a zip, built by `ledger-package` for both,
  as of a day (B-33), with the trial balance and general ledger as XLSX,
  the statements as Typst PDFs, the journal as IIF (B-34), every receipt
  behind the period's entries with an index, the company's documents, the
  1099 worksheet, the members' equity and K-1 worksheets (book and tax
  basis, B-36), and a README; handed over as a download (D-46).

## Non-goals (all phases of the engine)

The engine never does I/O, networking, wall-clock time sourcing, or UI. It is
a pure domain library: data in, validated books out.
