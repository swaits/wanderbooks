# Golden worksheet — balance sheet as of a date

Report: `balance_sheet_as_of(&ledger, 2026-01-31)`.
Golden: `crates/ledger-core/tests/golden/balance_sheet_as_of.json`.
Test: `golden_balance_sheet_as_of` in
`crates/ledger-core/tests/golden_reports.rs`.

## Fixture

The shared extended copy of `fixture_company()`; see
`profit-and-loss-period.md` for the full table and why it was extended.
Entries 1–5 are the original fixture, 6 is a multi-leg purchase, 7 is the
January close, 8 is a February entry after the as-of date:

| # | Date | Description | Lines |
|---|------|-------------|-------|
| 1 | 2026-01-02 | Owner investment | Dr 1000 Cash 10,000.00; Cr 3000 Owner's Equity 10,000.00 |
| 2 | 2026-01-05 | Buy equipment | Dr 1500 Equipment 2,500.00; Cr 1000 Cash 2,500.00 |
| 3 | 2026-01-10 | Invoice client | Dr 1200 Accounts Receivable 4,000.00; Cr 4000 Service Revenue 4,000.00 |
| 4 | 2026-01-15 | January rent | Dr 6000 Rent Expense 800.00; Cr 1000 Cash 800.00 |
| 5 | 2026-01-20 | Client payment | Dr 1000 Cash 1,500.00; Cr 1200 Accounts Receivable 1,500.00 |
| 6 | 2026-01-25 | Buy more equipment | Dr 1500 Equipment 1,500.00; Cr 1000 Cash 1,000.00; Cr 2000 Accounts Payable 500.00 |
| 7 | 2026-01-31 | Close period 2026-01 (generated) | Dr 4000 Service Revenue 4,000.00; Cr 6000 Rent Expense 800.00; Cr 3999 Retained Earnings 3,200.00 |
| 8 | 2026-02-05 | February service | Dr 1200 Accounts Receivable 600.00; Cr 4000 Service Revenue 600.00 |

Accounts: 1000 Cash (Asset), 1200 Accounts Receivable (Asset), 1500
Equipment (Asset), 2000 Accounts Payable (Liability), 3000 Owner's Equity
(Equity), 4000 Service Revenue (Revenue), 6000 Rent Expense (Expense), 3999
Retained Earnings (Equity, created by the close).

## Hand computation

As of 2026-01-31, inclusive: entries 1–7 count, entry 8 (2026-02-05) does
not. Closing entries count on a balance sheet: after a close the earnings
sit in retained earnings.

Asset accounts, normal side debit, amount = debits − credits:

- 1000 Cash: debits 10,000 + 1,500 = 11,500; credits 2,500 + 800 + 1,000 =
  4,300; 11,500 − 4,300 = **7,200.00**.
- 1200 Accounts Receivable: debits 4,000; credits 1,500; 4,000 − 1,500 =
  **2,500.00**.
- 1500 Equipment: debits 2,500 + 1,500 = 4,000; credits 0; **4,000.00**.
- `total_assets` = 7,200 + 2,500 + 4,000 = **13,700.00**.

Liability accounts, normal side credit, amount = credits − debits:

- 2000 Accounts Payable: credits 500; debits 0; **500.00**.
- `total_liabilities` = **500.00**.

Equity accounts, normal side credit, amount = credits − debits:

- 3000 Owner's Equity: credits 10,000; debits 0; **10,000.00**.
- 3999 Retained Earnings: credits 3,200; debits 0; **3,200.00**.

Current earnings (revenue minus expenses through the as-of date, not yet
closed):

- 4000 Service Revenue: credit 4,000 (entry 3) and the close's debit 4,000
  (entry 7) → movement 0, no line.
- 6000 Rent Expense: debit 800 (entry 4) and the close's credit 800 (entry
  7) → movement 0, no line.
- `current_earnings` = **0.00**.

Totals:

- `total_equity` = 10,000 + 3,200 + 0 = **13,200.00**.
- Accounting equation: assets 13,700 = liabilities 500 + equity 13,200.
  Balanced exactly.

**Entry 8 is left out.** If it were included (wrong for this as-of date),
AR would be 3,100, revenue/current earnings 600, total assets 14,300 and
total equity 13,800 — balanced but dated 2026-02-05. The golden pins the
January-only figures.
