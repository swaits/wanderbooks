# Golden worksheet — the audit workpaper's findings

Programme: `ledger_core::audit::statements` (rules.md B-45), the statements
half of `books.audit`; the store's integrity checks are `ledger_store::audit`
and are not in this golden (see "Scope of this golden").
Golden: `crates/ledger-core/tests/golden/audit_workpaper.json`.
Test: `golden_audit_workpaper` in `crates/ledger-core/tests/golden_audit.rs`.

## Why a new fixture

`audit_detects_each_seeded_defect` (`crates/ledger-core/tests/audit.rs`) seeds
each code in a *separate*, tiny book, and the store's
`four_defects_give_exactly_four_findings` needs raw journal surgery to file a
duplicate blob. Neither is one small book a person can audit whole. This
fixture is new and deliberately small: five defects in five different check
areas, with every other check passing. Every command is listed below.

## Fixture

Currency USD, `Ledger::new(Currency::USD)`, default `Settings` (unset means:
receipts required strictly above **25.00**, no required or exempt accounts,
a missing tax category is a warning). `through = 2026-03-31`.

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
lines, `post_entry`. A 2,000.00 opening keeps the bank positive after the
prepaid payment.

| Id | Date | Description | Lines | Receipt filed | Status |
|---|---|---|---|---|---|
| 1 | 2026-01-01 | Opening | Dr 1000 Bank 2,000.00; Cr 3000 Capital 2,000.00 | yes | posted |
| 2 | 2026-01-05 | Printer paper | Dr 6000 Supplies 40.00; Cr 1000 Bank 40.00 | **no** | posted |
| 3 | 2026-01-01 | Prepaid insurance | Dr 1300 Prepaid Insurance 1,200.00; Cr 1000 Bank 1,200.00 | yes | posted |
| 4 | 2026-02-10 | Stamps | Dr 6000 Supplies 5.00; Cr 1000 Bank 5.00 | — | **draft** |

The receipts handed to the audit (`AuditInput::receipts`) are entries
**{1, 3}** only. Entry 2 is the evidence defect. The draft is never posted,
so the receipt policy does not reach it.

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

- `cash_flow: None` — the default settings name no cash accounts, so the
  server computes no statement and the check has nothing to state (its
  `None` arm is a silent pass by design; an audit does say when a statement
  *cannot* be made, but no statement was asked for).
- `settings_problems: &[]` — default settings name no account or role.
- `undecided_1099: &[("Tobin Harlow Supply", 900.00)]` — one vendor paid
  900.00 in a way that could need a 1099, with no decision. (This is the
  package layer's reading; the core takes it as input, B-45's programme
  includes the 1099 decisions.)

## The books through 2026-03-31, by hand

Trial balance (posted entries only; the draft is excluded):

| Code | Debits | Credits |
|---|---|---|
| 1000 Bank | 2,000.00 − 40.00 − 1,200.00 = **760.00** | |
| 1300 Prepaid Insurance | **1,200.00** | |
| 3000 Capital | | **2,000.00** |
| 6000 Supplies | **40.00** | |

Debits 760 + 1,200 + 40 = **2,000.00**; credits **2,000.00**. Balanced.

Balance sheet: assets 760 + 1,200 = **1,960.00**; liabilities 0; equity =
capital 2,000.00 + current earnings (−40.00) = **1,960.00**. Balanced.

P&L 2026-01-01 to 2026-03-31: revenue 0.00, expenses 40.00 (supplies; the
prepaid months were never posted), net income **−40.00**. Current earnings
on the balance sheet are −40.00 and no close has run, so closed net is 0.00:
−40.00 = −40.00 + 0.00. Articulated.

## Every check, from B-45's programme

`audit::statements` runs them in this order; `sort` then puts failures
first, and within a severity by (code, subject, message). There is **one
failure** (`evidence_missing`; a required receipt that is not there is the
books being wrong, not unfinished work). Within the warnings the order is
by code.

| Code | Severity | Verdict | Why |
|---|---|---|---|
| `trial_balance_unbalanced` | failure | pass | debits 2,000.00 = credits 2,000.00 exactly. |
| `equation_broken` | failure | pass | assets 1,960.00 = liabilities 0.00 + equity 1,960.00. |
| `pnl_not_articulated` | failure | pass | −40.00 = −40.00 + 0.00 (see above). |
| `cash_flow_unfooted` | failure | pass | no statement asked for (`None`). |
| `control_difference` | failure | pass | no role account is assigned and no document posts a line, so the control check has no rows. |
| `unattached_control_line` | warning | pass | same: no control account, so no unattached line. |
| `document_overpaid` | warning | pass | no invoice or bill exists. |
| `close_split_drifts` | failure | pass | no period is closed. |
| `schedule_behind` | warning | **FAIL** | the 12-month plan has 3 months due and unposted, 300.00. |
| `depreciation_behind` | warning | pass | no asset is registered, so nothing is scheduled. |
| `register_differs` | failure | pass | the register holds no cost and no cost account has a balance: 0.00 = 0.00. |
| `evidence_missing` | failure | **FAIL** | entry 2 posted 40.00 of debits, strictly over 25.00, with no receipt. Entries 1 and 3 have receipts; the draft is not posted, so it is not checked here. |
| `legacy_reversal_exempt` | info | pass | no reversal exists. |
| `draft_open` | warning | **FAIL** | entry 4 is a draft dated 2026-02-10, on or before 2026-03-31. |
| `tax_category_missing` | warning | **FAIL** | account 4100 (Revenue) has no category; 4000, 6000, 6100 do. |
| `vendor_1099_undecided` | warning | **FAIL** | Tobin Harlow Supply, 900.00, no decision (handed in). |
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

### Correction made while checking

The first pass of this worksheet's prose said "no failures, five warnings"
even though its own table above marked `evidence_missing` a failure. The
programme returned 1 failure / 4 warnings, matching the table and
`CODES`; the prose and the first draft of the golden were the error, and
were corrected (a required receipt that is absent is B-45's "books are
wrong", and `CODES` is where the programme keeps that severity).

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
