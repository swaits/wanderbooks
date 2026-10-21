# Glossary

For agents (and humans) new to accounting. The engine's naming follows these
definitions exactly.

- **Debit / Credit** — The two sides of every journal line. Not "good" and
  "bad": debit = left, credit = right. What they *do* depends on the account
  type (see normal balance).
- **Normal balance** — The side that increases an account. Assets and expenses
  are debit-normal (a debit increases them); liabilities, equity, and revenue
  are credit-normal. `AccountType::normal_side()`.
- **Chart of accounts** — The registry of all accounts, organized by code
  (e.g. `1000` Cash, `6000` Rent Expense). Five types, no more.
- **Journal entry** — A set of balanced lines with a date and description.
  The atomic unit: it posts whole or not at all.
- **Ledger** — The books: the chart plus the append-only log of posted
  entries. (In this codebase `Ledger` is also the command surface.)
- **Posting** — Validating a draft entry and appending it to the log.
  One-way.
- **Trial balance** — A listing of every account's debit or credit balance.
  Total debits must equal total credits; if they don't, the books are broken.
- **Balance sheet** — Assets = Liabilities + Equity, at a point in time.
- **Profit & loss (income statement)** — Revenue − Expenses = Net income,
  over a period.
- **Reversing entry** — An entry that exactly undoes a posted entry (same
  lines, sides flipped), linked to the original. The correction mechanism.
- **Placeholder account** — A header in the chart (e.g. "Current Assets")
  that organizes other accounts but takes no postings itself.
- **Closing the books** — Period-end ritual: move revenue/expense balances
  into retained earnings so the new period starts clean. (Future: R-FUTURE-2.)
- **Current earnings** — Net income to date on an unclosed book. We fold it
  into equity so the accounting equation holds before close.
- **Placed in service (in-service date)** — The day an asset is ready and
  available for its use. Depreciation starts here (`Asset::in_service_date`;
  journaled under its old name `purchase_date`).
- **Acquired date** — The day the company got the asset: the date of the
  entry that put it on the books (its purchase or a member's contribution),
  never later than the in-service date. The register holds the asset from
  here (`Ledger::asset_acquired_date`).
- **Accumulated depreciation, posted and scheduled** — What the books carry
  for an asset (its depreciation runs, less any reversed:
  `Ledger::accumulated_posted`), and what its schedule says is due by a date
  (`accumulated_depreciation`, `accumulated_scheduled_through` in the
  tools). Book value is cost less the posted figure.
- **§723 basis** — The company's tax basis in property a member contributed:
  the member's adjusted basis at contribution (IRC §723).
- **Adjusted basis** — Cost plus improvements, less depreciation allowed or
  allowable (IRC §1016(a)).
- **Depreciable basis** — What tax depreciation starts from: the cost for
  something bought; the §723 basis for business property contributed; for
  property contributed from personal use, the lesser of its fair market
  value and the member's adjusted basis (Treas. Reg. §1.168(i)-4(b)).
- **Built-in gain or loss (§704(c))** — Property's fair market value less
  its §723 basis when contributed (Treas. Reg. §1.704-3(a)(3)(ii)). A
  built-in loss counts only for the member who contributed it (IRC
  §704(c)(1)(C)).
- **§704(c) method** — How the tax items of contributed property are shared
  among the members so the built-in gain or loss stays with its
  contributor: traditional, traditional with curative allocations, or
  remedial (Treas. Reg. §1.704-3(b)-(d)), chosen property by property.
- **Tax-basis capital** — A member's capital account as Schedule K-1 Item L
  reports it: cash plus the adjusted tax basis of property contributed,
  plus their share of income as figured for tax, less cash and the adjusted
  tax basis of property distributed (Form 1065 instructions, Item L).
- **Project** — A class with its facts (rules.md B-37): what it is, where it
  stands (active, paused, finished, abandoned), what kind of work it is
  (internal product, client work, on spec, other), who it is for, its
  dates and links. Its figures are the revenue and expense lines tagged to
  it (job costing).
- **On spec** — Work done hoping to be paid. Its costs are expenses as they
  are spent; the hoped-for payment is a gain contingency, recorded neither
  as a receivable nor as income until it arrives (ASC 450-30-25-1).
- **Unrecovered cost** — An on-spec project's expenses less its revenue to
  date, never below zero: what it would take to be repaid in full. A
  figure, never an asset.
