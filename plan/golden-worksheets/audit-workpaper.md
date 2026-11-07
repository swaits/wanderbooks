# Golden worksheet — the audit workpaper's findings

Programme: `ledger_core::audit::statements` (rules.md B-45), the statements
half of `books.audit`; the store's integrity checks are `ledger_store::audit`
and are not in this golden (see "Scope of this golden"). B-45 (invariant 1)
asks only that the findings be a deterministic list in a fixed order and
that a check which cannot be made be a finding; it neither lists the checks
nor states severities. The checks, their order and their severities are the
code's: `audit::statements` and the `CODES` table in
`crates/ledger-core/src/audit.rs` (see "Where the severities come from").
Golden: `crates/ledger-core/tests/golden/audit_workpaper.json`.
Test: `golden_audit_workpaper` in `crates/ledger-core/tests/golden_audit.rs`.

## Why a new fixture

`audit_detects_each_seeded_defect` (`crates/ledger-core/tests/audit.rs`) seeds
each code in a *separate*, tiny book, and the store's
`four_defects_give_exactly_four_findings` needs raw journal surgery to file a
duplicate blob. Neither is one small book a person can audit whole. This
fixture is new and deliberately small: five defects in five different check
areas, with every other check passing. Every command is listed below. The
1099 finding is derived from a payment posted in the books, through the same
derivation `books.audit` uses, not handed in.

## Fixture

Currency USD, `Ledger::new(Currency::USD)`, default `Settings` for the audit
(unset means: receipts required strictly above **25.00**, no required or
exempt accounts, a missing tax category is a warning). The books name the
bank, **1000**, as their one cash account (`books.cash_accounts`; no
financing or investing accounts), which the 1099 report and the cash-flow
statement need. `through = 2026-03-31`.

### Accounts, in creation order

`Ledger::add_account` numbers accounts from 1; every account takes the next
number.

| Id | Code | Name | Type | Tax category |
|---|---|---|---|---|
| 1 | 1000 | Bank | Asset | — |
| 2 | 1300 | Prepaid Insurance | Asset | — |
| 3 | 3000 | Capital | Equity | — |
| 4 | 4000 | Sales | Revenue | `gross_receipts` |
| 5 | 4100 | Other income | Revenue | **(none)** |
| 6 | 6000 | Supplies | Expense | `supplies` |
| 7 | 6100 | Insurance | Expense | `insurance` |

Only 4100 is a Revenue or Expense account with no category, so the
tax-category check has exactly one thing to say.

### Entries, in posting order

`Ledger::create_entry` numbers entries from 1. `post` = create, add the
lines, `post_entry`. A 3,000.00 opening keeps the bank positive after the
prepaid payment and the vendor's payment (2,000.00 would leave it at
−140.00).

| Id | Date | Description | Lines | Receipt filed | Status |
|---|---|---|---|---|---|
| 1 | 2026-01-01 | Opening | Dr 1000 Bank 3,000.00; Cr 3000 Capital 3,000.00 | yes | posted |
| 2 | 2026-01-05 | Printer paper | Dr 6000 Supplies 40.00; Cr 1000 Bank 40.00 | **no** | posted |
| 3 | 2026-01-01 | Prepaid insurance | Dr 1300 Prepaid Insurance 1,200.00; Cr 1000 Bank 1,200.00 | yes | posted |
| 4 | 2026-02-10 | Stamps | Dr 6000 Supplies 5.00; Cr 1000 Bank 5.00 | — | **draft** |
| 5 | 2026-03-10 | Supplies from Tobin Harlow | Dr 6000 Supplies 900.00; Cr 1000 Bank 900.00; vendor **Tobin Harlow Supply** | yes | posted |

The vendor is added with `Ledger::add_vendor("Tobin Harlow Supply")`, which
also makes it a party of its own (`party_of_vendor` is `Some`), and entry 5
is tagged with it (`set_entry_vendor`). Its 1099 decision is never set, so it
reads *undetermined*. The payment is created after the draft so the draft
stays entry 4. It debits Supplies (6000), which already has a tax category,
so no other finding changes.

The receipts handed to the audit (`AuditInput::receipts`) are entries
**{1, 3, 5}**. Entry 2 is the evidence defect. The draft is never posted,
so the receipt policy does not reach it. Entry 5 debits 900.00, over the
25.00 threshold, so it needs its receipt, and has one.

### Schedule

Registered (not posted):

`NewSchedule { description: "Prepaid insurance", kind: PrepaidExpense,
total: 1,200.00, start: 2026-01-01, months: 12, balance_account: 2 (1300),
pl_account: 7 (6100), convention: MonthlyEven }`.

MonthlyEven: 1,200.00 / 12 = 100.00 each, dated at each month's end, the
last month absorbing rounding (`amortization_plan`). Nothing is posted, so
every month due through 2026-03-31 is unposted: 2026-01-31, 2026-02-28,
2026-03-31 — **3 months, 300.00 in all**.

### Facts the caller hands in

Two of the inputs are derived from the ledger exactly as
`ledger-mcp/src/tools/audit.rs` (`audit_findings`) derives them, with
`ledger_package` (a dev-dependency of the core), and nothing in them is
typed in:

- `cash_flow` — because the books name cash accounts, the server tries the
  statement of cash flows from the first entry (2026-01-01) through the
  date and hands in its footing: `ledger_package::cash_flow_for(...)`
  `.map(|s| s.reconciles)`. By hand, see "The vendor's 1099 and the cash
  flow, by hand".
- `undecided_1099` — `ledger_package::vendor_1099_report(ledger, settings,
  2026, None, Some(through))`, keeping each vendor line with
  `decision_needed` as `(name, paid)`. By hand, see below.
- `settings_problems: &[]` — the settings name an account (1000) that the
  chart has, so there is no problem to report; the test hands in none (the
  real caller computes `settings_problems(ledger, &cfg)`, which needs
  `ledger-mcp`'s config, and this golden leaves that check silent).

## The books through 2026-03-31, by hand

Trial balance (posted entries only; the draft is excluded):

| Code | Debits | Credits |
|---|---|---|
| 1000 Bank | 3,000.00 − 40.00 − 1,200.00 − 900.00 = **860.00** | |
| 1300 Prepaid Insurance | **1,200.00** | |
| 3000 Capital | | **3,000.00** |
| 6000 Supplies | 40.00 + 900.00 = **940.00** | |

Debits 860 + 1,200 + 940 = **3,000.00**; credits **3,000.00**. Balanced.

Balance sheet: assets 860 + 1,200 = **2,060.00**; liabilities 0; equity =
capital 3,000.00 + current earnings (−940.00) = **2,060.00**. Balanced.

P&L 2026-01-01 to 2026-03-31: revenue 0.00, expenses 940.00 (supplies 40.00
and 900.00; the prepaid months were never posted), net income **−940.00**.
Current earnings on the balance sheet are −940.00 and no close has run, so
closed net is 0.00: −940.00 = −940.00 + 0.00. Articulated.

What the earlier version of this fixture (2,000.00 opening, no payment to
a vendor) had: bank 760.00, equity 1,960.00, expenses 40.00. The only
changes are the opening (+1,000.00 of capital and bank), the vendor's
payment (−900.00 bank, +900.00 supplies), and the figures that follow from
them; no finding's text or subject changes.

## The vendor's 1099 and the cash flow, by hand

**The 1099 report** (`vendor_1099_report`, rules.md B-19), year 2026,
as of 2026-03-31, cash account 1000:

- Threshold: B-19 gives $2,000 for payments made after Dec. 31, 2025 (the
  default for 2026), so **T = 2,000.00**.
- Entry 5 is dated 2026-03-10, in 2026 and on or before 2026-03-31, posted
  and tagged with the vendor, so it counts toward it. Entries 2 and 3 also
  pay money out of the bank but carry no vendor tag: they are not the
  vendor's (they only show in the report's untagged note).
- Bucket (B-19's table): its credit line is the cash account 1000 with no
  payment method recorded, so it is a cash account line: **counts**,
  presumed check, ACH or transfer (CHOICE D-40). The debit to Supplies is
  an expense, so "no payment line". Counts = 900.00; card = 0; unknown = 0.
- The vendor's decision is the default, *undetermined*, so the 900.00 goes
  to U (payments made while undetermined), none to F (decided for a
  1099) and none to *not reportable*.
- Status (B-19): anything in U is *undetermined* when hi(F) + max(hi(U), 0)
  ≥ T, else *under threshold*. Here 0 + 900.00 = 900.00 < 2,000.00, so the
  status is **under_threshold**.
- `paid` = F + U = **900.00**.

**Is it flagged?** The audit takes each vendor line with `decision_needed`.
`decision_needed` is `has_undecided && hi(U) > 0` (`form1099.rs`): any
undecided payment above zero, **not** compared with the threshold. Here the
vendor has undecided payments and hi(U) = 900.00 > 0, so `decision_needed`
is true and the vendor is handed to the audit as `("Tobin Harlow Supply",
900.00)`, one `vendor_1099_undecided` finding. So the finding is raised by
the code's rule although the status is under_threshold. B-19's status
sentence puts "(a decision is needed)" on the at-or-over-threshold case and
does not define the `decision_needed` flag separately, so this golden pins
the **code's** rule here, and that is a convention of the caller, not a
statement of B-19. (An earlier worry that the audit's message, "could need
a 1099", over-reports a vendor under the threshold is a question for the
rule, not for this golden.)

**The cash flow** (rules.md B-14, direct method; accounts 1000 is the only
cash account, none financing or investing), 2026-01-01 to 2026-03-31:

| Entry | Cash moved | Section | Why |
|---|---|---|---|
| 1 Opening | +3,000.00 | financing | capital contributed by the owner (equity) |
| 2 Printer paper | −40.00 | operating | paid to suppliers |
| 3 Prepaid insurance | −1,200.00 | operating | paid to suppliers (a prepaid asset is operating when no investing account lists it) |
| 5 Tobin Harlow | −900.00 | operating | paid to suppliers |

Operating **−2,140.00**, investing 0.00, financing **+3,000.00**; the
sections sum to **+860.00**. Cash at the start 0.00, at the end 860.00 (the
bank's balance above), so the change is **+860.00**: the sections foot
(difference 0.00, B-14 rule 9) and `cash_flow_unfooted` stays silent. The
draft moved no cash. The test asserts these hand values on the statement
and the footing before it asserts the findings.

## Every check, in the order `audit::statements` runs them

`audit::statements` runs them in this order; `sort` then puts failures
first, and within a severity by (code, subject, message). The Severity
column is the `CODES` table's (see "Where the severities come from"). There
is **one failure** (`evidence_missing`: `CODES` files a required receipt
that is not there as a failure). Within the warnings the order is by code.

| Code | Severity | Verdict | Why |
|---|---|---|---|
| `trial_balance_unbalanced` | failure | pass | debits 3,000.00 = credits 3,000.00 exactly. |
| `equation_broken` | failure | pass | assets 2,060.00 = liabilities 0.00 + equity 2,060.00. |
| `pnl_not_articulated` | failure | pass | −940.00 = −940.00 + 0.00 (see above). |
| `cash_flow_unfooted` | failure | pass | the statement can be made and its sections sum to +860.00 = the cash change (see above). |
| `control_difference` | failure | pass | no role account is assigned and no document posts a line, so the control check has no rows. |
| `unattached_control_line` | warning | pass | same: no control account, so no unattached line. |
| `document_overpaid` | warning | pass | no invoice or bill exists. |
| `close_split_drifts` | failure | pass | no period is closed. |
| `schedule_behind` | warning | **FAIL** | the 12-month plan has 3 months due and unposted, 300.00. |
| `depreciation_behind` | warning | pass | no asset is registered, so nothing is scheduled. |
| `register_differs` | failure | pass | the register holds no cost and no cost account has a balance: 0.00 = 0.00. |
| `evidence_missing` | failure | **FAIL** | entry 2 posted 40.00 of debits, strictly over 25.00, with no receipt. Entries 1, 3 and 5 have receipts; the draft is not posted, so it is not checked here. |
| `legacy_reversal_exempt` | info | pass | no reversal exists. |
| `draft_open` | warning | **FAIL** | entry 4 is a draft dated 2026-02-10, on or before 2026-03-31. |
| `tax_category_missing` | warning | **FAIL** | account 4100 (Revenue) has no category; 4000, 6000, 6100 do. |
| `vendor_1099_undecided` | warning | **FAIL** | Tobin Harlow Supply: 900.00 paid by entry 5, undetermined, so `decision_needed` (see above). |
| `settings_problem` | warning | pass | no settings problems handed in. |

Failures 1, warnings 4, notes 0 → summary
`"1 failure(s), 4 warning(s), 0 note(s)"`.

## The five findings, in the golden's order

The failure first, then the four warnings by code (draft_ &lt; schedule_ &lt;
tax_ &lt; vendor_).

1. `evidence_missing`, **failure**, subject `{"kind":"entry","id":2}`:
   `entry 2 (2026-01-05) has no receipt: its debits 40.00 are strictly greater than the 25.00 threshold`
2. `draft_open`, warning, subject `{"kind":"entry","id":4}`:
   `draft 4 (2026-02-10) "Stamps" is not posted`
3. `schedule_behind`, warning, subject `{"kind":"schedule","id":1}` (the first
   registered schedule):
   `"Prepaid insurance" has 3 month(s) due and not posted, from 2026-01-31, 300.00 in all`
4. `tax_category_missing`, warning, subject `{"kind":"account","id":5}`:
   `account 4100 Other income has no tax category`
5. `vendor_1099_undecided`, warning, subject `{"kind":"vendor","id":"Tobin Harlow Supply"}`:
   `Tobin Harlow Supply was paid 900.00 in a way that could need a 1099 and has no decision`

### Where the severities come from

B-45 states no severities (see the top of this worksheet). Each code's
severity is the `CODES` table's in `crates/ledger-core/src/audit.rs`, and
the meaning is the `Severity` doc comment there: a **failure** is "the books are wrong or cannot be shown
right: an audit with one is not clean"; a **warning** is "unfinished work or
a policy not met"; an **info** is "listed so the workpaper is whole; not a
defect". That table is the code's own **CONVENTION**, not an authority
(nothing in the IRS or FASB sources assigns a severity); this golden pins
it, so a change of a code's severity fails the golden and is a decision to
make on purpose.

### Correction made while checking

The first pass of this worksheet's prose said "no failures, five warnings"
even though its own table above marked `evidence_missing` a failure. The
programme returned 1 failure / 4 warnings, matching the table and `CODES`;
the prose and the first draft of the golden were the error, and were
corrected. (The correction at the time said the severity came from B-45;
it does not: it is `CODES`'s, as above.)

### Checks this golden leaves silent by construction

Twelve of the seventeen codes are silent here. Eight are silent because the
fixture has nothing for them to check, and only a book built for it makes
them speak:

- the control accounts: `control_difference`, `unattached_control_line`,
  `document_overpaid` (no role account, no invoice or bill);
- closes: `close_split_drifts` (no period is closed);
- assets: `depreciation_behind`, `register_differs` (no asset is
  registered);
- reversals: `legacy_reversal_exempt` (no reversal exists);
- the settings: `settings_problem` (the test hands in no problem).

The other four, `trial_balance_unbalanced`, `equation_broken`,
`pnl_not_articulated` and `cash_flow_unfooted`, are silent because the books
are right (shown by hand above); a posted ledger cannot be made wrong
through the engine's commands, so the golden can only show them passing.

`audit_detects_each_seeded_defect` (`crates/ledger-core/tests/audit.rs`)
seeds every row of `CODES` separately, a book or a figure of its own for
each (the four statement checks by handing the check function a wrong
figure), and checks exactly that finding comes out. So each code that is
silent here is pinned to fire elsewhere.

## Scope of this golden: what is in it, and what is left out

The workpaper `books.audit` files (`ledger-mcp/src/tools/audit.rs`) is

```
company, title, scope, through, through_seq, program_version,
tb_hash, findings_sha256, summary, findings, checks
```

This golden holds the part the core programme states and a person can check
by hand:

```
scope, through, summary, findings
```

Left out, and why:

- `company` — the settings' company name (`"Unnamed company"` by default),
  pure but not a finding, and it would pin the settings' default;
- `through_seq` (the figure head) and `tb_hash`/`findings_sha256` —
  deterministic, but they are journal seqs and SHA-256 digests, not figures
  anyone can work out from B-45; a golden that pins a hash cannot be
  hand-verified, so it does not belong here. The head is already covered by
  the store's `a_run_is_idempotent_on_the_figure_head`;
- `program_version` — `concat!("audit-", CARGO_PKG_VERSION, "-1")` changes
  with every release, so it is a property of the build, not of the books;
- `checks` — the programme's static catalogue (`CODES`: code, severity,
  predicate); it has no books in it and `audit_detects_each_seeded_defect`
  already pins that every row has a seed. The worksheet's table above
  records the catalogue's verdict on these books instead.
- the store's integrity findings (`ledger_store::audit`) — a golden of those
  needs a store directory, JSON journal and blobs to be hand-checked for
  content that is by construction "nothing was tampered with"; the store's
  `integrity_detects_each_seeded_defect` and
  `books_the_version_before_p8_wrote_wait_for_the_upgrade` already pin the
  clean case. The statements half is where the accounting figures are.
