# Golden worksheet: statement of changes in members' equity (two members, with rounding and a close)

Golden: `crates/ledger-core/tests/golden/members_equity_two_member.json`
Test: `crates/ledger-core/tests/statements.rs` fn `golden_members_equity_two_member`
Fixture: `partnership(close_january)` in the same file (existing; not changed).

Every number below was computed by hand from the entries before the golden
was written or the report was printed.

## Rules relied on (docs/spec/rules.md)

- **B-13** (26 U.S.C. §704(a); Treas. Reg. §1.704-1(b)(2)(iv)(b); Form 1065
  Schedule M-2): a partner's capital account is one running balance
  = contributions + allocations of income − distributions. AUTHORITY.
  - Share of income: by the agreement (here 1/2 each).
  - Rounding a share to the cent: **no authority**; CHOICE (owner, 2026-10-13):
    largest remainder, a tied odd cent to the member listed first.
  - The statement splits *undistributed earnings cumulatively* (CHOICE, D-38):
    a member's earnings share as of a date d is allocate(C + U(d)) − allocate(C),
    with C = 0 here (no members close ever happens: the fixture's close goes to
    Retained Earnings, which "allocates nothing to the members"), U(d) = the
    partnership's undistributed earnings to date.
  - Statement INVARIANT 3a: a member's ending equals their beginning on the
    next statement, exactly.
- **B-11**: revenue and expenses close into P&L, then capital; the close is not
  income or spending (so it is not a contribution).
- **B-7 / B-6**: accounting equation / trial balance: used to tie the Total
  column to balance-sheet equity.

## Fixture entries (all USD, shares 1/2 and 1/2, Alice listed first)

Accounts: 1000 Cash (asset), 3000 Capital — Alice, 3001 Capital — Bob
(equity), 4000 Revenue, 6000 Expense.

| # | date   | Dr                  | Cr                   | amount |
|---|--------|---------------------|----------------------|--------|
| 1 | Jan 2  | 1000 Cash           | 3000 Alice capital   | 600.00 |
| 2 | Jan 2  | 1000 Cash           | 3001 Bob capital     | 400.00 |
| 3 | Jan 10 | 1000 Cash           | 4000 Revenue         | 1000.00 |
| 4 | Jan 20 | 6000 Expense        | 1000 Cash            | 333.33 |
| – | Jan 31 | (variant only) close of January to Retained Earnings 3999: Dr 4000 1000.00, Cr 6000 333.33, Cr 3999 666.67 |
| 5 | Feb 5  | 3001 Bob capital    | 1000 Cash            | 100.00 |
| 6 | Feb 9  | 1000 Cash           | 4000 Revenue         | 100.01 |

The statement must be the same whether or not January is closed (INVARIANT
3c, B-13: "a member's capital on a date never moves when a period ending on
or before it is later closed"), so the golden is one set of numbers asserted
against both variants.

## January 1 – January 31

Beginning: nothing posted before Jan 1: capital 0 and undistributed earnings 0
for both. Beginning = 0.00 / 0.00.

Contributions (credits to the member's own capital account in the span, no
close, no reversal): Alice 600.00 (entry 1), Bob 400.00 (entry 2).
Distributions: none.

Net income for the period = revenue − expense = 1000.00 − 333.33 = **666.67**.

Share, cumulative to the cent. Work in cents: 66667. Exact half = 33333.5.
Floor each: 33333 + 33333 = 66666; 1 cent left over. Both have the same
remainder (0.5): a tie, so the odd cent goes to the member listed first,
Alice. Alice 33334 cents = **333.34**, Bob 33333 cents = **333.33**.
(333.34 + 333.33 = 666.67 ✓.)

Because there are no earlier statements, each member's income for the period
is their cumulative share minus their beginning share (0).

| column        | Alice  | Bob    | Total   |
|---------------|--------|--------|---------|
| beginning     | 0.00   | 0.00   | 0.00    |
| contributions | 600.00 | 400.00 | 1000.00 |
| net income    | 333.34 | 333.33 | 666.67  |
| distributions | 0.00   | 0.00   | 0.00    |
| other         | 0.00   | 0.00   | 0.00    |
| ending        | 933.34 | 733.33 | 1666.67 |

Roll-forward per row: Alice 0 + 600 + 333.34 − 0 = 933.34 ✓;
Bob 0 + 400 + 333.33 − 0 = 733.33 ✓.

"Other" is the part of the change in equity not explained by contributions,
distributions or income. Change in equity Jan 1→Jan 31 = 1666.67 − 0;
explained = contributions 1000 − distributions 0 + NI 666.67 = 1666.67.
Unexplained 0.00 ✓.

Tie to the balance sheet (B-7): total equity at Jan 31 = capital 600 + 400
+ earnings 666.67 (current earnings in the unclosed variant, retained earnings
3999 in the closed one) = **1666.67** = Total ending ✓. (Assets: cash
600 + 400 + 1000 − 333.33 = 1666.67, no liabilities: the equation holds.)

## February 1 – February 28

Beginning = each member's ending on Jan 31 (INVARIANT 3a):
Alice 933.34, Bob 733.33; total 1666.67.

Contributions: none (entry 5 debits Bob's capital; entry 6 is revenue).
Distributions: Bob 100.00 (entry 5 debits his capital account; the Jan 31
close, in the closed variant, is not a distribution: B-11). Alice 0.
Net income for February = 100.01 (revenue, no expense).

Cumulative undistributed earnings to Feb 28: 666.67 + 100.01 = 766.68
= 76668 cents. Half = 38334 cents exactly, no remainder, no tie. So each
member's cumulative share is **383.34**.

Month income per member = cumulative share to Feb 28 − cumulative share to
Jan 31:
- Alice: 383.34 − 333.34 = **50.00**.
- Bob:   383.34 − 333.33 = **50.01**.
  Sum 100.01 = February net income ✓ (INVARIANT 3b).

| column        | Alice  | Bob    | Total   |
|---------------|--------|--------|---------|
| beginning     | 933.34 | 733.33 | 1666.67 |
| contributions | 0.00   | 0.00   | 0.00    |
| net income    | 50.00  | 50.01  | 100.01  |
| distributions | 0.00   | 100.00 | 100.00  |
| other         | 0.00   | 0.00   | 0.00    |
| ending        | 983.34 | 683.34 | 1666.68 |

Roll-forward: Alice 933.34 + 0 + 50.00 − 0 = 983.34 ✓;
Bob 733.33 + 0 + 50.01 − 100.00 = 683.34 ✓.
Cross-check each ending from first principles, from inception:
Alice 600 contributed + 383.34 earnings − 0 = 983.34 ✓;
Bob 400 + 383.34 − 100 = 683.34 ✓.

"Other": change in equity = 1666.68 − 1666.67 = 0.01; explained = 0 − 100.00 +
100.01 = 0.01. Unexplained 0.00 ✓.

Tie to balance sheet: equity at Feb 28 = 1000 contributed + 766.68 earnings
− 100 distributed = **1666.68** = Total ending ✓. (Cash: 1666.67 − 100 + 100.01
= 1666.68 ✓.)

## Golden file contents

Per statement: `start`, `end`, one column per member in the order given,
`total`, and `balance_sheet_equity` (hand-computed above, compared with the
engine's balance sheet total_equity). Amounts are decimal strings,
normalized (no trailing zeros), as the existing `snapshot` golden does: the
test normalizes both sides so a harmless scale difference ("0" vs "0.00")
is not a failure but every value must still agree exactly.
