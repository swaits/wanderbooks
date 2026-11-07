# Golden worksheet: tax line mapping + tax summary (Form 1065 first quarter)

Golden: `crates/ledger-core/tests/golden/tax_summary_form_1065.json`
Test: `crates/ledger-core/tests/tax.rs` fn `golden_tax_summary_form_1065`
Fixture: `books()` in the same file (existing; not changed). Summary asked
for 2026-01-01 .. 2026-03-31 as Form 1065.

Every number below was computed by hand from the entries before the golden
was written or the report was printed.

## Rules relied on

- **B-32** (docs/spec/rules.md; IRS Pub. 583, Pub. 538, Rev. Proc. 98-25):
  every revenue and expense account maps to a line of the return; an account
  with none is listed, not silently dropped.
- **B-11 / B-14**: the summary covers a period, built from the P&L (closing
  entries excluded). Book amounts are the accounts' normal-side balances
  (revenue credits, expense debits).
- Category-to-line mapping: `docs/spec/sources/irs-tax-lines.md`
  (derived from the Schedule C / Form 1065 / 1120-S / 1120 instructions;
  vendored). Relevant rows:
  - Form 1065 line 1a "Gross receipts or sales", line 1b "Returns and
    allowances": returns reduce gross receipts (income).
  - Form 1065 line 10 "Guaranteed payments to partners": a deduction on
    Form 1065 only.
  - `commissions_fees`, `other_deductions`: deductions (Form 1065 line 21).
  - `meals`: Schedule C line 24b instructions: "Your deductible business meal
    expenses are a percentage of your actual business meal expenses ... In most
    cases, the percentage is 50%." AUTHORITY for 50%.
  - `nondeductible`: Schedule C Part V: "you can't deduct fines or penalties
    paid to a government for violating any law." AUTHORITY for 0% (no line).
- **Cent rounding of a deductible amount** was NOT stated by any authority in
  rules.md when this was written (since 2026-11-07, D-67's CHOICE in B-32: once per category) (see "Observations" below). Applied here: the cent, half away from
  zero (the engine's documented rule, `tax_summary` doc; B-3's half-away-
  from-zero for currency). Only the meals line needs it.
- Display order of categories is the catalog order in `tax::CATEGORIES`
  (a presentation convention, not an accounting number): income
  gross_receipts, returns_allowances; deductions commissions_fees,
  guaranteed_payments, meals, other_deductions, nondeductible.

## Fixture entries (all USD, all dated Feb 1; each is Dr X / Cr Cash or the reverse)

| #  | entry                              | Dr             | Cr          | amount    |
|----|------------------------------------|----------------|-------------|-----------|
| 1  | sales                              | 1000 Cash      | 4000 Sales  | 10,000.00 |
| 2  | refunds                            | 4100 Refunds   | 1000 Cash   | 250.00    |
| 3  | platform fees                      | 6000 Fees      | 1000 Cash   | 1,500.00  |
| 4  | meals                              | 6100 Meals     | 1000 Cash   | 333.33    |
| 5  | penalties                          | 6200 Penalties | 1000 Cash   | 40.00     |
| 6  | software                           | 6300 Software  | 1000 Cash   | 600.00    |
| 7  | partner pay                        | 6400 Partner pay | 1000 Cash | 2,000.00  |
| 8  | misc (no tax category)             | 6500 Misc      | 1000 Cash   | 75.00     |

Chart tags: 4000 Sales (Revenue) gross_receipts; 4100 Refunds (**Expense**
type) returns_allowances; 6000 commissions_fees; 6100 meals (catalog default
50%); 6200 nondeductible (default 0%); 6300 other_deductions; 6400
guaranteed_payments; 6500 none. No account sets its own deductible percent
override, so each uses its category's default.

## Hand computation

Book amount is the account's normal-side balance for the period. One account
per category here, so category = account.

### Income

- gross_receipts (4000 Sales): revenue credit 10,000.00. Book 10,000.00.
  Percent 100 → tax 10,000.00.
- returns_allowances (4100 Refunds): the account is an *expense* account
  (debit 250.00) in an *income* category, so it counts with its sign flipped: a
  contra to income reduces gross receipts (Form 1065 line 1b is subtracted
  from 1a). Book −250.00. 100% → tax −250.00.
- Income total: book 10,000 − 250 = 9,750.00; tax 9,750.00.

### Cost of goods sold: none. Separately stated (Schedule K): none. Form 4797: none.

### Deductions

| category            | account            | book    | percent | tax (exact)   | tax (cent)  |
|---------------------|--------------------|---------|---------|---------------|-------------|
| commissions_fees    | 6000 Platform fees | 1500.00 | 100     | 1500.00       | 1500.00     |
| guaranteed_payments | 6400 Partner pay   | 2000.00 | 100     | 2000.00       | 2000.00     |
| meals               | 6100 Meals         | 333.33  | 50      | 333.33×0.5 = 166.665 | **166.67** (half away from zero) |
| other_deductions    | 6300 Software      | 600.00  | 100     | 600.00        | 600.00      |
| nondeductible       | 6200 Penalties     | 40.00   | 0       | 0.00          | 0.00        |

Deductions total: book 1500 + 2000 + 333.33 + 600 + 40 = **4,473.33**;
tax 1500 + 2000 + 166.67 + 600 + 0 = **4,266.67**.

### Uncategorized

6500 Misc (Expense), book 75.00: no category, so it is listed (B-32) and is in
no total (the nets are "of the categorized accounts").

### Nets

- Book net = income book − deductions book = 9,750.00 − 4,473.33 = **5,276.67**.
- Tax net = 9,750.00 − 4,266.67 = **5,483.33**.
- Book-to-tax difference = 5,483.33 − 5,276.67 = 206.66
  = the disallowed half of meals (333.33 − 166.67 = 166.66)
  + penalties (40.00) = 206.66 ✓ (the Schedule M-1 items).
- Cross-check with the whole P&L: all accounts' net = 9,750 − 4,473.33 − 75
  = 5,201.67 (the categorized net 5,276.67 less the uncategorized 75.00 ✓).

## Golden file contents

The serialized `TaxSummary` (start, end, form, income, cost_of_goods,
deductions, separately_stated, form_4797, uncategorized) plus two keys the
test adds from the hand numbers above: `book_net` and `tax_net` (methods on
the report, not serialized fields). `accounts` lists "CODE Name" of the
accounts that roll into a category; `label` is the catalog's label (the
return's own line name). Decimals compare by value (normalized, as the
existing `snapshot` golden writes them).

## Observations (not disagreements; nothing in the golden depends on them)

- No rule in `docs/spec/rules.md` states how a deductible fraction rounds to
  the cent (only the code's doc comment does). B-21 says "round off only the
  total" on the return. With one account per category the two approaches agree
  (166.665 → 166.67 either way); with several accounts in one category the
  engine rounds each account and sums, which can differ from rounding the
  category's sum by a cent per account. Worth a CONVENTION entry in rules.md.
- Resolved 2026-11-07: the owner chose rounding once per category (D-67,
  a CHOICE line in rules.md B-32); the engine now does so. With one account
  per category this golden is unchanged.
