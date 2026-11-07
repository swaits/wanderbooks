# Golden worksheet — trial balance as of a date

Report: `trial_balance_as_of(&ledger, 2026-01-31)`.
Golden: `crates/ledger-core/tests/golden/trial_balance_as_of.json`.
Test: `golden_trial_balance_as_of` in
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
not. A trial balance shows each account's signed movement: debit positive,
credit negative, then in one column on the side it falls (never negative).

Per account over entries 1–7:

- **1000 Cash:** debits 10,000 (e1) + 1,500 (e5) = 11,500; credits 2,500
  (e2) + 800 (e4) + 1,000 (e6) = 4,300; net debit 11,500 − 4,300 =
  **debit 7,200.00**.
- **1200 Accounts Receivable:** debits 4,000 (e3); credits 1,500 (e5); net
  **debit 2,500.00**.
- **1500 Equipment:** debits 2,500 (e2) + 1,500 (e6) = 4,000; credits 0;
  **debit 4,000.00**.
- **2000 Accounts Payable:** credits 500 (e6); debits 0; **credit 500.00**.
- **3000 Owner's Equity:** credits 10,000 (e1); debits 0; **credit
  10,000.00**.
- **3999 Retained Earnings:** credits 3,200 (e7); debits 0; **credit
  3,200.00**.
- **4000 Service Revenue:** credits 4,000 (e3), debits 4,000 (close e7);
  net 0 — the invariant says a zero balance is not listed.
- **6000 Rent Expense:** debits 800 (e4), credits 800 (close e7); net 0 —
  not listed.

Lines are in code order: 1000, 1200, 1500, 2000, 3000, 3999.

Totals:

- `total_debits` = 7,200 + 2,500 + 4,000 = **13,700.00**.
- `total_credits` = 500 + 10,000 + 3,200 = **13,700.00**.
- Balanced exactly.

**Entry 8 is left out.** If included it would add 600 to AR (3,100) and
600 to revenue (credit 600), shifting both columns by 600; the golden pins
the post-closing January trial balance after the close, with the February
entry absent.
