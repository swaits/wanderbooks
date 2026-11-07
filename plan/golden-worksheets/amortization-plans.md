# Golden worksheet: prepaid expense and deferred revenue amortization

Golden: `crates/ledger-core/tests/golden/amortization_plans.json`
Test: `golden_amortization_plans` in `crates/ledger-core/tests/schedules.rs`
Fixture: the existing `books()` fixture and the two hand-verified setups in
`a_prepaid_expense_is_expensed_month_by_month` and
`deferred_revenue_is_earned_to_the_cent` (same accounts, same totals, same
conventions).
Rules: `docs/spec/rules.md` B-16 (prepaids recognized over the period
benefited, deferred revenue as it is earned; equal-months schedules, the
last month absorbing rounding), with B-15's rounding discipline, and D-37
(monthly_even schedules start on the 1st).

The schedules in this fixture ask explicitly for `monthly_even` (the
convention of every schedule registered before P3), so the plan is
`amortization_plan`: each period is `total ÷ months` rounded to the cent,
and the last period is `total − each × (months − 1)`, so the plan sums to
the total exactly.

## Prepaid expense: a year of insurance from 2026-03-01

Entry: 2026-03-01, Dr 1300 Prepaid Expenses 1,200 / Cr 1000 Cash 1,200.

Plan, 12 equal months (monthly_even; the 1st is the start):

- Each of the first 11 months: 1,200 ÷ 12 = **100.00**.
- Last month: 1,200 − 100.00 × 11 = 1,200 − 1,100 = **100.00** (no
  remainder here; the division is exact).

| # | Period | Amount | Cumulative | Remaining |
|---|---|---|---|---|
| 1 | 2026-03-31 | 100 | 100 | 1,100 |
| 2 | 2026-04-30 | 100 | 200 | 1,000 |
| 3 | 2026-05-31 | 100 | 300 | 900 |
| 4 | 2026-06-30 | 100 | 400 | 800 |
| 5 | 2026-07-31 | 100 | 500 | 700 |
| 6 | 2026-08-31 | 100 | 600 | 600 |
| 7 | 2026-09-30 | 100 | 700 | 500 |
| 8 | 2026-10-31 | 100 | 800 | 400 |
| 9 | 2026-11-30 | 100 | 900 | 300 |
| 10 | 2026-12-31 | 100 | 1,000 | 200 |
| 11 | 2027-01-31 | 100 | 1,100 | 100 |
| 12 | 2027-02-28 | 100 | 1,200 | 0 |

The fixture posts the first three months (March, April, May). Remaining =
1,200 − 300 = **900**. `posted` is those three month ends.

Postings: each month Dr 6300 Insurance / Cr 1300 Prepaid Expenses, 100.

Profit and loss, 2026-03-01 through 2026-05-31: insurance expense
100 + 100 + 100 = **300**; revenue 0.

Balance sheet at 2026-05-31:
- Cash: 0 − 1,200 = **−1,200** (the fixture pays the premium from cash and
  has no other entries).
- Prepaid Expenses: 1,200 − 300 = **900**.
- Total assets: −1,200 + 900 = **−300**.
- Liabilities: **0**.
- Current earnings: revenue 0 − expense 300 = **−300**; equity **−300**.
- −300 = 0 + (−300): balanced.

## Deferred revenue: three months from 2026-01-01

Entry: 2026-01-01, Dr 1000 Cash 1,000 / Cr 2400 Deferred Revenue 1,000.

Plan, 3 equal months:

- Each of the first 2 months: 1,000 ÷ 3 = 333.333… → **333.33**.
- Last month: 1,000 − 333.33 × 2 = 1,000 − 666.66 = **333.34** (the
  rounding remainder, 0.01, is absorbed by the last period).
- Sum: 333.33 + 333.33 + 333.34 = **1,000.00** exactly.

| # | Period | Amount | Cumulative | Remaining |
|---|---|---|---|---|
| 1 | 2026-01-31 | 333.33 | 333.33 | 666.67 |
| 2 | 2026-02-28 | 333.33 | 666.66 | 333.34 |
| 3 | 2026-03-31 | 333.34 | 1,000.00 | 0 |

All three months are posted, so remaining = **0**. Postings: each month
Dr 2400 Deferred Revenue / Cr 4000 Subscription Revenue.

Profit and loss, 2026-01-01 through 2026-03-31: revenue 333.33 + 333.33 +
333.34 = **1,000.00**; expenses 0.

Balance sheet at 2026-03-31:
- Cash **1,000**.
- Liabilities: the deferred revenue is fully earned → **0**.
- Current earnings **1,000.00**; equity **1,000.00**.
- 1,000 = 0 + 1,000.00: balanced.

## Notes on the serialized figures

- The decimal strings keep the arithmetic scale: the prepaid plan divides
  exactly, so its amounts print as `"100"`, `"300"`, `"900"`, `"1200"`;
  the deferred plan carries cents, so its amounts print as `"333.33"`,
  `"333.34"`, `"1000.00"`, and its remaining as `"0"`.
- `total` is the schedule's `Money.amount`; `currency` is the code (USD),
  not `Money`'s three-byte serde form, so the golden stays readable.
- `posted` is the schedule's posted month ends; `plan` is every period,
  posted or not, in order.
