# Rule catalog

Every rule the engine enforces, the authority for it, and what checks it.
Status is **AUTHORITY** (a source states the rule), **CONVENTION** (common
practice no authority states; the best reference is given), or
**DECISION** (a human chose, and when). Vendored sources are in
`sources/`; FASB and OpenStax are quoted briefly under their copyright and
license (see `README.md`). Quotes from FASB's Codification were read in
Deloitte's DART roadmaps, which reproduce it ("FASB Accounting Standards
Codification: Copyright © 2005 - 2026 by Financial Accounting
Foundation").

Spec blocks in the code cite these as `SOURCE: rules.md B-n`.

---

## B-1 Every entry is a double entry: debits equal credits

**Status:** AUTHORITY

- Pacioli ch. 36 (`sources/pacioli-1494.md`): "All entries made in the
  ledger have to be double entries — that is, if you make one creditor,
  you must make some one debtor."
- IRS Pub. 583 (`sources/irs-records.md`): "It is self-balancing because you
  record every transaction as a debit entry in one account and as a credit
  entry in another. Under this system, the total debits must equal the
  total credits after you post the journal entries to the ledger accounts."

**Enforced:** `kernel::check_lines` via `JournalEntry::validate` (two or
more lines, debits = credits exactly). A line's account must exist and
take postings when the line is added, not only at post: DECISION
(Stephen, 2026-10-13), `Ledger::add_line`
(`a_line_on_an_unknown_or_placeholder_account_is_refused_when_added`). **Checked:** Kani
`check_lines_is_exact`, `accepted_entry_nets_to_zero`; Lean
`Accepted.net_zero`; property `prop_posted_entries_balance`.

## B-2 A line's amount is positive; its side gives the direction

**Status:** CONVENTION

Pacioli keeps amounts in a debit column and a credit column (ch. 36: "All
the creditors must appear in the Ledger at the right hand side, and all the
debtors at the left"); the side, not a sign, says which way money moves. No
authority forbids a negative amount outright; wanderbooks refuses them so
that every line reads one way (2026-10-13, found by the formal pass).

**Enforced:** `kernel::check_lines` (`NegativeAmountLine`, `ZeroAmountLine`).
**Checked:** Kani `check_lines_is_exact`; test
`negative_amount_line_is_rejected`.

## B-3 One money through the ledger; foreign amounts at the transaction-date rate

**Status:** AUTHORITY

- Pacioli ch. 36: "the money that you reckon by at the beginning should be
  the same all through the Ledger."
- ASC 830-20-30-1 (cited): "each asset, liability, revenue, expense, gain,
  or loss arising from the transaction shall be measured initially in the
  functional currency of the recording entity by use of the exchange rate
  in effect at that date."

A foreign line is booked in the base currency's smallest unit (ISO 4217
minor units, `sources/iso-4217.md`), half away from zero; a ledger in one
money holds no fractions of its smallest unit, and its cash can then
match a bank statement: DECISION (Stephen, 2026-10-13), after the GnuCash
cross-check found wanderbooks kept $108.34567 where GnuCash booked
$108.35. Lines booked before then replay as booked.

**Enforced:** `Ledger::add_line` (base currency only, no fraction of the
minor unit), `Ledger::add_line_fx` (base amount = foreign amount × rate,
rounded to the minor unit; original kept), `Ledger::post_entry` (every
line, whatever made it: payroll, closes, disposals; `FinerThanMinorUnit`,
since 2026-10-13 after a payroll run posted $65.27794). Journals from
before replay as booked (`replay_post_entry`). **Checked:**
`a_foreign_line_is_booked_in_whole_base_units`,
`no_entry_posts_a_fraction_of_the_minor_unit`; the GnuCash oracle.
**Checked:** `foreign_currency_line_is_rejected`, `fx_overflow_is_an_error`.

## B-4 Five elements, closed list

**Status:** AUTHORITY (definitions); the five-type chart is CONVENTION

FASB Concepts Statement No. 8, Ch. 4 (2021), which replaced Concepts
Statement 6 (cited): E16 "An asset is a present right of an entity to an
economic benefit." E37 "A liability is a present obligation of an entity to
transfer an economic benefit." E61 equity is "the residual interest in the
assets of an entity that remains after deducting its liabilities." E80
revenues "are inflows or other enhancements of assets … or settlements of
its liabilities … from delivering or producing goods, rendering services,
or carrying out other activities." E81 expenses "are outflows or other
using up of assets … or incurrences of its liabilities …".

**Enforced:** `AccountType` (five variants). A child account has its
parent's type: DECISION (Stephen, 2026-10-13), `Ledger::add_account`
(`ParentTypeMismatch`). **Checked:** `a_child_account_has_its_parents_type`.

## B-5 Normal balances

**Status:** CONVENTION

No standard defines normal balances. OpenStax, *Principles of Accounting,
Vol. 1* §3.1 (cited; "Access for free at openstax.org"): "As assets and
expenses increase on the debit side, their normal balance is a debit…
Since liabilities, equity (such as common stock), and revenues increase
with a credit, their 'normal' balance is a credit." Pacioli ch. 12: capital
"must always be placed as creditor … and the cash always debtor."

**Enforced:** `AccountType::normal_side`, `kernel::normal_balance`.
**Checked:** Kani `normal_balance_is_signed`, `normal_side_increases`; Lean
`balance_after_line`.

## B-6 The trial balance balances

**Status:** AUTHORITY

Pacioli ch. 36: "the total of the credits — I do not say creditors —
should be equal to the total of the debits — I do not say debtors. If they
were not equal there would be a mistake in the Ledger." IRS Pub. 583 (B-1).

**Enforced:** follows from B-1. **Checked:** Kani `trial_balance_columns`;
Lean `trial_balance_iff`, `Reachable.trial_balance`,
`Reachable.trial_balance_through`; properties.

## B-7 The accounting equation

**Status:** AUTHORITY

CON 8 Ch. 4 ¶E62 (cited): equity "is the difference between the entity's
assets and its liabilities." On a live book, income not yet closed is part
of equity (B-11).

**Checked:** Kani `accounting_equation`; Lean `accounting_equation`,
`Reachable.accounting_equation[_through]`; property
`prop_balance_sheet_balances`.

## B-8 Posted is forever: the book is written only forward

**Status:** AUTHORITY

- Pacioli ch. 36: "You should not write backward, but always forward —
  that is, go forward as the days go, which never come back. If you do
  otherwise, the book would be untrue."
- German Commercial Code (HGB) §239(3) (cited; official English
  translation): "An entry or a documented record may not be modified such
  that it is no longer possible to establish its original content."
- Rev. Proc. 98-25 (`sources/irs-records.md`): machine-sensible records
  must keep the audit trail.

**Enforced:** posted entries immutable; the journal is append-only and
hash-chained. **Checked:** Lean `Step.extends`, `Steps.extends`; property
`prop_posted_entries_immutable`; store `journal_tamper_is_detected`.

## B-9 A mistake is corrected by an opposite entry, never erased

**Status:** AUTHORITY

Pacioli ch. 31 (the *storno*): "you shall make another entry opposite this
one in the credit for the same amount … After you have made this
correction it is just as if you had written nothing in the debit column."
Ch. 36: "Mark with a cross or with an "H" that special entry, and then make
an entry on the opposite side under the same account."

**Enforced:** `Ledger::reverse_entry` (same lines, sides flipped, linked by
`reversal_of`). **Checked:** Kani `reversal_mirrors`, `reversal_cancels`;
Lean `accepted_reversal_iff`, `movement_with_reversal`.

## B-10 An entry is reversed at most once; a closing entry never

**Status:** DECISION (Stephen, 2026-10-13)

Follows from B-9 (a correction corrects once) and B-11 (a closed period
stays closed). **Enforced:** `Ledger::check_reversal`. **Checked:**
`an_entry_is_reversed_at_most_once`, `a_closing_entry_is_never_reversed`.

## B-11 Revenue and expenses close into profit and loss, and that into capital

**Status:** AUTHORITY (mechanics); the account the net lands in depends on
the entity

- Pacioli ch. 34: "you shall close this account into the capital account";
  ch. 27: profit and loss "must then be transferred for its closing (saldo)
  into the capital account."
- OpenStax §5.1 (cited): "Temporary (nominal) accounts are accounts that
  are closed at the end of each accounting period." Corporations close into
  retained earnings; partnerships into each partner's capital (B-13).

**Enforced:** `Ledger::close_period_to`. **Checked:** Kani
`closing_line_zeroes`, `closing_entry_balances`; Lean
`closingEntry_zeroes`, `closingEntry_accepted`, `close_zeroes`,
`close_credits_net_income`; `a_close_sweeps_only_through_the_period_end`.

## B-12 A closed period takes no new postings; periods close in order

**Status:** CONVENTION

Periods themselves: IRS Pub. 538 (`sources/irs-periods-and-methods.md`)
defines the tax year, calendar and fiscal. Locking a closed period and
closing in order are practice (and follow from B-8: a closed period's
book is not rewritten). Periods nest (a month in a quarter in a year)
and never partially overlap: DECISION (Stephen, 2026-10-13),
`Ledger::create_period` (`PeriodsOverlap`;
`periods_nest_but_never_partially_overlap`). **Enforced:** `closed_period_containing`,
`CloseOutOfOrder`, `Store::rehearse`. **Checked:** Lean `Steps.frozen`;
store property `refused_commands_never_reach_the_journal` (no draft in a
closed period).

## B-13 Partners share income by the partnership agreement; capital accounts carry it

**Status:** AUTHORITY (shares); DECISION (cent rounding)

- 26 U.S.C. §704(a) (`sources/irs-partnerships.md`): "A partner's
  distributive share of income, gain, loss, deduction, or credit shall,
  except as otherwise provided in this chapter, be determined by the
  partnership agreement."
- Form 1065 Schedule M-2: "Show what caused changes during the tax year in
  the partners' tax-basis capital accounts."
- **No authority** says how to round a share to the cent. Largest
  remainder, ties to the earlier member: DECISION (Stephen, 2026-10-13).
  Closes made before then replay with the last member absorbing it.
- Rounding never piles up: a run of members closes (same members and
  shares, back to back) splits cumulatively, each close giving a member
  their allocation of the run's total so far less their allocation before
  it, so each member's total from the run is within a cent of their exact
  share (follows from the running capital account below; found
  2026-10-13: closing each month on its own gave every tied cent to the
  earlier member). Journaled as `"rounding": "cumulative"`; closes
  journaled `"largest_remainder"` replay one at a time.
- A capital account is one running balance: Form 1065 Schedule K-1
  instructions, Item L: "The partnership must report your beginning
  capital account and ending capital account for the year", "based on the
  partnership's books and records." So a statement of members' equity
  splits undistributed earnings cumulatively, as of each date (beginning
  as of the day before the period, ending as of its last day), never one
  period at a time: a member's ending on one statement is their beginning
  on the next, to the cent, and a member's period income is the
  roll-forward's remainder (their share within a rounding cent).

**Enforced:** `CloseTo::Members`, `periods::allocate`,
`kernel::apportion`, `members_equity`. **Checked:** Kani
`apportion_is_exact`, `apportion_is_fair`; Lean `runPart_telescopes`,
`closePart_total`; properties `prop_split_*`,
`each_members_capital_chains_across_statements`,
`each_member_stays_within_a_cent_of_their_share`;
`a_run_of_closes_never_piles_up_rounding`,
`members_closes_replay_under_the_rule_they_were_journaled_with`;
`closes_journaled_under_the_old_rule_replay_with_it`.

## B-14 Statements: P&L for a period; cash flow by the indirect method

**Status:** AUTHORITY

ASC 230-10-45-28 (cited): an entity not using the direct method "shall
determine and report the same amount for net cash flow from operating
activities indirectly by adjusting net income … (the indirect or
reconciliation method)." ASC 230-10-45-16: "All of the following are cash
inflows from operating activities: Cash receipts from sales of goods or
services…". A closing entry is not income or spending (B-11), so period
statements exclude it.

Investing and financing are gross (ASC 230-10-45-7, read directly
2026-10-14, `sources/fasb-asc.md`:
"information about the gross amounts of cash receipts and cash payments
during a period is more relevant than information about the net
amounts"; 45-8 allows netting only where "the turnover is quick, the
amounts are large, and the maturities are short"). Financing inflows
include "Proceeds from issuing equity instruments" and "other short- or
long-term borrowing" (45-14(a), (b)); outflows include "distributions to
owners" and "Repayments of amounts borrowed" (45-15(a), (b)). Every cash
flow is investing, financing, or operating (45-10). Investing
"include[s] making and collecting loans and acquiring and disposing of
debt or equity instruments and property, plant, and equipment" (ASC
230-10-20). Which accounts hold loans made, investments, and debt is a
fact of each company's chart, so it is a setting: `[books]
financing_accounts` and `investing_accounts` in wanderbooks.toml (with
`cash_accounts`), read by the MCP tools and the viewer alike.

Money an owner advances, by paying a company expense personally (a
"Due to owners" liability), is borrowing: PwC Viewpoint, Financial
statement presentation 6.9.3.1 (constructive receipts and
disbursements) presents cash a third party pays on the entity's behalf
"as though the transaction took place through the bank accounts of the
reporting entity": a financing inflow and an operating outflow. Listing
that liability in `financing_accounts` gives exactly that; repaying the
owner is then a repayment of amounts borrowed. "Judgment is required"
(PwC), so it is the company's setting, not a fixed rule.

**Enforced:** `profit_and_loss_for_period`, `cash_flow`. **Checked:** Lean
`statements_add_up`; `prop_period_pnl_partitions`;
`cash_flow_reconciles_to_change_in_cash`;
`financing_is_gross_borrowing_apart_from_repayment`,
`a_loan_the_entity_makes_is_investing_gross`,
`an_expense_the_owner_paid_is_borrowing_then_operating`.

## B-15 Depreciation: straight line, salvage never depreciated, partial months by days

**Status:** CONVENTION (book method); AUTHORITY for tax conventions, which
wanderbooks does not apply

OpenStax §11.3 (cited): "Straight-line depreciation is a method of
depreciation that evenly splits the depreciable amount across the useful
life of the asset," with "(Cost – salvage value)/Useful life"; partial
periods: "A common method is to allocate depreciation expense based on the
number of months the asset is owned in a year." wanderbooks prorates
partial months by days, including a schedule through a mid-month date
(DECISION, Stephen 2026-10-13). ASC 360-10-35-4 (read 2026-10-14,
`sources/fasb-asc.md`) prescribes no method: cost less salvage is spread
over the useful life "in a systematic and rational manner," as equitably
as possible to the periods served; straight line by days is one such
allocation.
Tax conventions (half-year, mid-month, mid-quarter): IRS Pub. 946
(`sources/irs-depreciation.md`), for the CPA.

**Enforced:** `assets::depreciation_schedule`. **Checked:**
`schedule_sums_exactly_to_depreciable_amount`,
`salvage_value_is_never_depreciated`, `prop_depreciation_schedule_exact`.

## B-16 Prepaid expenses and deferred revenue are recognized over the period benefited or earned

**Status:** AUTHORITY (revenue); CONVENTION (prepaids)

ASC 606-10-45-2 (cited): payment before transfer → "the entity shall
present the contract as a contract liability"; 606-10-25-23: "An entity
shall recognize revenue when (or as) the entity satisfies a performance
obligation by transferring a promised good or service." Prepaids: OpenStax
§4.2 (cited); ASC 340-10-05-4 and 210-10-45-1(g), -2 (read 2026-10-14,
`sources/fasb-asc.md`): a prepaid is an asset paid in advance of its use,
current within the operating cycle; the ASC prescribes no expensing
method, so monthly expensing stays a convention. Tax: the 12-month rule, Treas. Reg. §1.263(a)-4(f)
(`sources/irs-capitalization.md`).

**Enforced:** `schedules`. **Checked:** `a_prepaid_expense_is_expensed_month_by_month`,
`deferred_revenue_is_earned_to_the_cent`.

## B-17 Platform payouts are recorded gross when the business is the principal

**Status:** AUTHORITY

ASC 606-10-55-37 (cited): "An entity is a principal if it controls the
specified good or service before that good or service is transferred to a
customer." 606-10-55-38: an agent "recognizes revenue in the amount of any
fee or commission to which it expects to be entitled." 606-10-55-37B
(read 2026-10-14, `sources/fasb-asc.md`): a principal recognizes revenue
"in the gross amount of consideration." Whether an app
developer is principal for app-store sales is a judgment under 55-39; the
CPA confirms.

**Enforced:** `payouts::payout_plan`. **Checked:**
`a_payout_is_grossed_up_to_revenue_and_fees`.

## B-18 Aging buckets: Current, 1–30, 31–60, 61–90, 90+ days past due

**Status:** CONVENTION (bucket edges); DECISION (due today is Current)

ASC 326-20-55-37/38 (cited) illustrates "one way" to age trade
receivables with buckets "current … 1–30 days past due … 31–60 … 61–90 …
more than 90 days past due"; it is illustrative, not a rule. The FFIEC
Call Report instructions (bank regulators, not binding here): "The past
due status … should be determined in accordance with its contractual
repayment terms." A document due today is not yet past due: DECISION
(Stephen, 2026-10-13). Aging as of a date shows only documents issued by
then, less only payments made by then; and a payment never predates its
invoice or bill (a real prepayment is a deposit): DECISIONS (Stephen,
2026-10-13).

**Enforced:** `arap::aging_bucket`, `Ledger::ar_aging`/`ap_aging`,
`receive_payment`/`pay_bill` (`PaymentBeforeDocument`). **Checked:**
`aging_boundaries`, `aging_is_exclusive_and_only_ages`,
`aging_as_of_a_date_sees_only_what_existed_then`,
`a_payment_never_predates_its_document`.

## B-19 1099 reporting thresholds

**Status:** AUTHORITY (`sources/irs-1099-thresholds.md`)

$600 through 2025; $2,000 for payments made after Dec. 31, 2025 (26 U.S.C.
§6041(a), §6041A(a)(2), as amended by Pub. L. 119-21 §70433); for calendar
years after 2026, indexed to inflation and rounded to the nearest $100
(§6041(h)). Card payments are reported on Form 1099-K, not 1099-NEC/MISC.
Attorneys' gross proceeds stay at $600 (not modeled).

What counts as paid: payments made in the calendar year (§6041(a)), so a
prepayment counts when made. The engine counts credits minus debits to the
payment sources on a vendor's tagged entries: `[books] cash_accounts` and
`financing_accounts` (a member or lender paying on the company's behalf;
the company is still the payer). A card charge leaves neither (1099-K); a
bill is not paid until it is (`form1099::vendor_1099_report_with`,
checked by `payments_come_from_cash_and_financing_only`). A W-9 is on
file by the vendor's flag or its party's W-9 facts.

**Enforced:** `form1099::default_threshold`, a dated rule
(`ledger_core::rules::THRESHOLD_1099`, known through 2026, review from
2026-11-01). For a later year the engine refuses with `RuleNotKnown`
rather than guess; the MCP digest and the viewer's "Needs attention" list
flag the rule once it is due (DECISION, Stephen, 2026-10-13). **Checked:**
`an_unknown_year_threshold_is_never_guessed`,
`dated_rules_go_stale_on_their_review_date`, MCP stdio test.

## B-20 Every transaction has supporting documents; records are kept

**Status:** AUTHORITY (`sources/irs-records.md`)

IRS Pub. 583: "In addition, you must keep supporting documents"; they
"support the entries in your books and on your tax return"; keep records
"until the period of limitations for that return runs out" (3 years
generally; 7 for bad debts).

**Enforced:** the MCP receipt policy (not the engine; R-13).

## B-21 Exact amounts in the books; rounding only on returns

**Status:** AUTHORITY (for where rounding happens)

Form 1065 instructions (`sources/irs-partnerships.md`): "If two or more
amounts are added to figure the amount to enter on a line, include cents
when adding the amounts and round off only the total." The books keep
cents (and FX precision) exactly (R-2).

**Enforced:** `Decimal` money, `Amount` checked arithmetic, line range.
**Checked:** Kani `sums_are_exact`; bridge tests.

## B-22 Expense or capitalize (book policy, mirrors tax)

**Status:** AUTHORITY (`sources/irs-capitalization.md`)

Treas. Reg. §1.263(a)-1(f)(1)(ii)(D) sets $500 per invoice or item for a
taxpayer without an applicable financial statement "or other amount as
identified in published guidance"; Notice 2015-82 raised it to $2,500 for
taxable years beginning on or after January 1, 2016. The 12-month rule:
§1.263(a)-4(f)(1). The election is made yearly on the return.

**Enforced:** bookkeeper policy (books' AGENTS.md), not the engine.

## B-23 Control roles: one account each, of the right kind, moved only when empty

**Status:** CONVENTION (a control account agrees with the records it
controls); DECISIONS (Stephen, 2026-10-13 and 2026-10-15)

Invoices, bills, payroll, and closes post to accounts the engine picks by
role: receivables, revenue, payables, general expense, retained earnings,
salary and payroll tax expense, and five payroll withholdings. A role uses
its default code (1200, 4000, 2000, 6999, 3999, 6100, 6110, 2110–2150)
until the human assigns it one of the chart's own accounts; the assignment
is journaled, so replay never depends on a setting outside the journal
(DECISION, 2026-10-15). A role's account is postable and of the role's
type (B-4), and the engine refuses one that isn't rather than renumber
the chart (DECISION, 2026-10-13). No account fills two roles. A
receivables, payables, withholding, or retained earnings account must keep
agreeing with what it controls (open invoices, open bills, taxes withheld,
closed earnings): such a role moves only when its account holds no
balance, so the bookkeeper first moves the balance with an entry, which
the journal shows. Revenue and expense roles move at any time; earlier
postings stay where they are.

**Enforced:** `Ledger::check_controls`, `Ledger::assign_role` /
`check_assign_role`, `Ledger::check_renumber_account` (an assigned role
frees its code). **Checked:** `ledger-core` `tests/spec_roles.rs`;
`ledger-store` replay of `role.assigned`.

## B-24 Asset tags: one label per asset, never reused

**Status:** DECISION (Stephen, 2026-10-15)

Each fixed asset may carry a tag, the label stuck on the thing itself
(generated `PREFIX-0001` or the human's own: 1–32 ASCII letters, digits,
`- _ . /`), and the manufacturer's serial number. Tags are unique ignoring
case and are never reused by another asset, even after a retag or a
disposal, so an old label can never point at the wrong thing. Both are
journaled. The fixed asset register lists every asset as of a date and
reconciles to the general ledger's cost and accumulated depreciation
accounts.

**Enforced:** `Ledger::set_asset_tag` / `check_asset_tag_for`,
`assets::check_asset_tag`, `assets::check_serial`. **Checked:**
`ledger-core` `tests/spec_asset_tags.rs`; `ledger-store`
`tests/asset_tags.rs`; `ledger-mcp` `tests/spec_assets.rs`.

## B-25 Tax basis beside book value

**Status:** AUTHORITY (`sources/irs-partnership-basis.md`); DECISION
(Stephen, 2026-10-15: the register carries tax basis)

Book value and tax basis are different numbers for the same asset. The
book value of a member's contribution is its fair market value (Treas.
Reg. §1.704-1(b)(2)(iv)(d)(1), cited). Its federal tax basis follows the
rule that applies: property the company buys has a cost basis ("The basis
of property shall be the cost of such property", IRC §1012(a)); property a
member contributes keeps "the adjusted basis of such property to the
contributing partner at the time of the contribution" (IRC §723,
carryover); property a member converts from personal use depreciates from
"the lesser of its fair market value or its adjusted depreciable basis"
(Treas. Reg. §1.168(i)-4(b)(1)). The difference between fair market value
and the contributor's adjusted basis is the built-in gain (positive) or
loss (negative) that IRC §704(c) makes the partnership account for; a
built-in loss counts only for the contributing member (§704(c)(1)(C)).
wanderbooks records the rule, the contributor's adjusted basis, the
resulting tax basis, and the built-in gain or loss; tax depreciation
(MACRS, conventions, elections) stays with the CPA (`sources/irs-depreciation.md`).

**Enforced:** `Ledger::set_asset_tax_basis` / `check_asset_tax_basis`
(cost takes no contributor basis, a contribution requires one,
non-negative, in cents), `Asset::tax_basis`, `Asset::built_in_gain`,
`reports::fixed_asset_register` (per-row tax fields, `total_tax_basis`,
`missing_tax_basis`). **Checked:** `ledger-core` `tests/spec_tax_basis.rs`;
`ledger-store` `tests/asset_tags.rs` (`asset.tax_basis_set` journaling,
replay, v9 migration, corrupt payloads); `ledger-mcp` `tests/spec_assets.rs`.

## B-26 Parties: one directory, no full TIN in the books

**Status:** AUTHORITY (truncated TINs: `sources/irs-truncated-tin.md`);
DECISIONS (Stephen, 2026-10-15)

Everyone the company deals with is a party, with roles (vendor,
contractor, customer, employee, member, payout platform, tax agency,
financial institution, landlord). Vendor and customer records link to
their party; entries are tagged with theirs. Names are unique among parties
not archived, ignoring case and surrounding space. A W-9 is recorded as its
facts and the TIN's last four digits (the part a truncated TIN shows,
Treas. Reg. §301.6109-4(a)); no party field may hold a nine-digit number,
and the full TIN lives only in the encrypted vault, outside the journal
(DECISION). W-9 facts need a 1099 role (contractor, landlord); W-4 facts
need the employee role; payroll lines that name their employee name a
party with that role.

**Enforced:** `crate::parties` (has_full_tin, check_text, W9Facts::check,
W4Facts::check), `Ledger::add_party`, `update_party`, `set_party_role`,
`set_party_w9`, `set_party_w4`, `set_entry_party`, `validate_payroll`.
**Checked:** `ledger-core` `tests/spec_parties.rs`; `ledger-store`
`tests/parties.rs`; `ledger-mcp` `tests/spec_parties.rs`.
