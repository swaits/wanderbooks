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
