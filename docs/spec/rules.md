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
(the owner, 2026-10-13), `Ledger::add_line`
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
match a bank statement: DECISION (the owner, 2026-10-13), after the GnuCash
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
parent's type: DECISION (the owner, 2026-10-13), `Ledger::add_account`
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

**Status:** DECISION (the owner, 2026-10-13)

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
and never partially overlap: DECISION (the owner, 2026-10-13),
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
  remainder, ties to the earlier member: DECISION (the owner, 2026-10-13).
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
- A correction undoes what it corrects (B-9): an entry and its reversal
  both in a statement's span are neither contributions nor
  distributions (`reports::cancelled_in_span`; the K-1 worksheet reads
  the same). The K-1 worksheet's contributed property (Item L) is what
  the member owned and handed over, with its own basis (§723): a prepaid
  service contract credited straight to capital is property (a contract
  handed over), while a prepaid service they paid a vendor for on the
  company's behalf, booked through Due to Shareholders (Dr prepaid / Cr
  2100, then Dr 2100 / Cr capital), is cash (DECISION, the owner,
  2026-10-15); and it is shared among
  members credited in one entry in proportion to their credits
  (`k1` INVARIANTS 3, 4, 6).

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
(DECISION, the owner 2026-10-13). ASC 360-10-35-4 (read 2026-10-14,
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
(the owner, 2026-10-13). Aging as of a date shows only documents issued by
then, less only payments made by then; and a payment never predates its
invoice or bill (a real prepayment is a deposit): DECISIONS (the owner,
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

An entry naming several parties (a payment shared between a contractor
and a member; `ledger_core::shares`) counts only the vendor's share
toward its 1099: what was paid to the payee (§6041(a)), not what the
entry moved for others. The share is resolved (`shares::resolve`)
against the entry's flow, the same target the MCP server checks shares
against, then scaled to the 1099 payment: the share unchanged when the
payment equals the target (the usual case, an expense paid from cash or
by a member), else payment × share ÷ target, rounded to the nearest cent,
half away from zero (so a part charged to a card scales out, 1099-K). A
percent split's leftover cent goes to the party named first (shares
INVARIANT 8). A reversal carries the shares and subtracts the same part,
and re-apportioning either one re-apportions the other
(`Ledger::set_entry_parties` INVARIANT 5), so the pair nets to zero. The
vendor tag follows the parties: on an entry naming several,
`set_entry_vendor` never replaces their shares (refused unless the
vendor's party is named), and a vendor record never links to a party
named beside another vendor (`add_vendor`), checked by
`tagging_the_vendor_keeps_a_shared_entrys_parties`,
`clearing_the_vendor_on_a_shared_entry_is_refused`,
`re_apportioning_a_reversed_entry_re_apportions_its_reversal`,
`a_named_party_gaining_a_vendor_record_keeps_one_vendor_per_entry`.
Shares that don't resolve count the whole payment toward the vendor (the
conservative answer) and the entry is listed (`Vendor1099Report::
unresolved`; a `books.vendor_1099` note). An entry naming one party
counts as before (`form1099::vendor_1099_report_with_members`, checked
by `a_shared_payment_counts_the_vendors_share`,
`a_percent_split_gives_the_leftover_cent_to_the_first`,
`a_reversed_shared_payment_nets_to_zero`,
`a_shared_payment_partly_on_a_card_scales_to_the_payment`,
`unresolved_shares_count_in_full_and_are_listed`). DECISION (the owner,
2026-10-15: shares).

**Enforced:** `form1099::default_threshold`, a dated rule
(`ledger_core::rules::THRESHOLD_1099`, known through 2026, review from
2026-11-01). For a later year the engine refuses with `RuleNotKnown`
rather than guess; the MCP digest and the viewer's "Needs attention" list
flag the rule once it is due (DECISION, the owner, 2026-10-13). **Checked:**
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
controls); DECISIONS (the owner, 2026-10-13 and 2026-10-15)

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

**Status:** DECISION (the owner, 2026-10-15)

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
(the owner, 2026-10-15: the register carries tax basis)

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
DECISIONS (the owner, 2026-10-15)

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

The full TIN, when the books need it for the 1099 worksheet, lives in the
TIN vault (`vault/` in the books dir, git-ignored; encrypted under a key
file outside the books dir): `books.set_w9` `tin` writes it there and
journals only the last four and the vault record's hash; the signed W-9
goes there too, never to the attachment store. Any party may have its
TIN in the vault, whatever its roles: every member's (for their K-1) and
every 1099 payee's belongs there (DECISION, the owner, 2026-10-15: "In the
vault, that's the WHOLE reason we built it. And it needs to be in the
damn CPA package."). `books.set_tin` stores one for any party; the books
journal its TIN facts (type, last four, the record's hash:
`party.tin_set`), which never disagree with its W-9 facts' last four. No
tool output, error, or log holds a full TIN except `books.vendor_1099`
with `include_tins` (DECISION, the owner, 2026-10-15: for the CPA, when
asked): a refusal that quotes its input masks any TIN in it
(`parties::redact_full_tins`, checked by `refusals_never_echo_a_tin`),
and a party text field refuses nine digits however they are separated
(space, hyphen, dot, slash, underscore; `has_full_tin`). Files holding one (`books.get_w9`, `save_csv_to`, and the CPA
package, whose K-1 and 1099 worksheets carry every member's and payee's
TIN by default) are written only outside the books dir and outside the
server's import folders (DECISION, P0 review, 2026-10-16: so none can be
filed back in as a receipt and read out again), and the package comes
back only as its path. No file holding a full TIN is filed into the books
(DECISION, P0 review round 2, 2026-10-16): the server keeps, beside the
vault, the SHA-256 of every file it read or wrote with full TINs (the W-9
`books.set_w9` reads, recorded once the call has passed every check, just
before the vault keeps it, so a refused call marks no file; each
`books.get_w9` copy;
a 1099 worksheet with TINs; a CPA package with TINs and each file in it
that holds one), shared by every session and every later server for the
books, and every way a file enters the books (`books.import_local_file`,
`books.attach_receipt`, `books.file_document`) refuses those exact files,
and refuses any text file (by its type or name, or because it decodes as
UTF-8 or UTF-16, with a byte-order mark or, NUL-interleaved, without one)
holding (a) a TIN the vault holds for a party of the
books, anywhere in the text, as nine digits in a row, as 3-2-4 digits
joined by hyphens, spaces or dots, or as 2-7 digits joined by a hyphen
or a space (compared on the server against the decrypted vault, never
returned or logged; a vault that can't be opened leaves (b) alone), or
(b) a number printed as an SSN or EIN is, standing alone: 3-2-4 digits
joined by hyphens (123-45-6789) or 2-7 joined by a hyphen (12-3456789),
with no digit, and no hyphen or dot followed by a digit, on either side.
Full-width digits and Unicode hyphens and spaces count as their ASCII
kin (`tin_files.rs`). The text check is narrow on purpose (DECISION,
the owner, 2026-10-16): bank exports are filed for reconciliation, and
their routing numbers, long references and dates next to amounts are
not TINs; the broad `has_full_tin` reading stays on what the server says
(refusals and results). Because (a) compares against the vault, its
answer would tell an agent whether a text it chose holds a vault TIN;
the tools show each TIN's last four, so an agent filing texts full of
nine-digit numbers ending in those four could find the TIN in about 17
filings. So (a) answers for at most two candidates a filing (DECISION,
P0 review, 2026-10-16): before the vault is opened, a text holding more
than two distinct TIN-shaped numbers (in any rendering (a) reads; one
number written several ways counts once) that end in the last four of a
TIN the books know (each party's TIN facts and W-9 facts, read from the
books; once the vault is opened, its own records' last fours too) is
refused with words of its own ("more than two numbers shaped like a
TIN"), the same byte for byte whether or not one of them is a vault TIN
and whether or not the vault is open; only a text with no more than two
is compared against the vault. With the last four known, a TIN is one of
100,000 numbers, so finding one this way takes on the order of 50,000
filings, each one refused and seen. What this can't catch: a changed
copy of a binary file (a W-9 PDF re-saved or re-scanned), a TIN in an
image, or a TIN the vault doesn't hold written other than (b);
`plan/served.md` P2 and P7 take the path tools and full TINs out of the
agent's reach. A known limit, open until then: (a) still answers for two
candidates a filing, so an agent that makes those 50,000 refused filings
can find a TIN; P2's access log records every call, refusals included,
and P7's human-only TIN channel takes full TINs, and with them this
check, off the agent's path. A bank export holding three numbers that
happen to end in a known last four is refused the same way (filed as a
PDF instead).
A locked vault or a missing TIN never fails
the package: it says which TINs are missing. `books.purge_tin` deletes a
party's vault records and journals that it did.

**Enforced:** `crate::parties` (has_full_tin, check_text, W9Facts::check,
W4Facts::check), `Ledger::add_party`, `update_party`, `set_party_role`,
`set_party_w9`, `set_party_w4`, `set_party_tin` (TinFacts::check; TIN and
W-9 facts agree), `set_entry_party`, `validate_payroll`; `ledger-core`
`k1` (the K-1 worksheet); `ledger-vault` (Tin::new, the encrypted
records, records_held, purge_records); `ledger-mcp` `books.set_w9`,
`books.set_tin`, `books.get_w9`, `books.purge_tin`, `books.vendor_1099`,
`books.export_cpa_package` (open_vault refuses a key file in the books
dir; outside_books for files with a TIN; vault_tin checks the vault
against the books); `ledger-mcp` `tin_files` (the registry, the text
scan and its probe bound, on every filing tool; `known_tin_last4` gives
the books' last fours).
**Checked:** `ledger-core` `tests/spec_parties.rs`; `ledger-store`
`tests/parties.rs`; `ledger-vault` `tests/vault.rs`; `ledger-mcp`
`tests/spec_parties.rs`, `tests/spec_vault.rs` (every response, the log,
and every file in the books dir outside `vault/` scanned for the TINs in
any formatting), `tests/tin_files.rs`
(`a_tin_bearing_file_is_refused_from_every_session_and_after_a_restart`,
`text_with_a_vault_tin_or_a_printed_one_is_refused_and_a_bank_export_is_not`,
`text_probing_for_a_tin_is_refused_alike_by_every_filing_tool`), and the
unit tests in `tin_files.rs`.

## B-27 The journal format gate: newer is not corrupt

**Status:** DECISION (the owner, 2026-10-16; `plan/served.md` 5.1 and P0);
DECISION (P0 review, 2026-10-16: a line's own hash is checked before the
line is called newer; the envelope and its hash are frozen across
formats)

Every journal line has a type, and a version of wanderbooks reads a fixed
set of them (`ledger_store::format::EVENT_TYPES`). A newer version may
write a type an older one doesn't know, or declare a newer journal format
with a `journal.format_set {format, features}` line before the first event
that needs it. An older reader stops at the first line it can't read and
says so: it never skips a line (the books it showed would differ from the
record without saying so), and it never calls newer books corrupt.

What every journal format shares is frozen, so that an older reader can
always follow the chain one line past the first line it can't read. A
line is the envelope `{seq, ts, type, payload, prev}` and nothing beside
it; its hash (what the next line's `prev` holds) is the SHA-256 of its
canonical JSON: keys sorted at every depth, no whitespace. A newer format
puts its new fields inside `payload`, never in the envelope, and never
changes how a line is hashed. A line with a key beside the envelope's is
refused as newer, never read with the key dropped, and its hash covers
the key, so the next line still checks it.

Damage stays corruption. A line's seq and `prev` are checked before the
gate, and its own hash is checked by the next line's `prev`; so when the
gate refuses a line, the reader reads the next one first, taking only its
`prev` (any JSON object with a string `prev`: it may use floats, a string
seq or keys of a format this version doesn't read). If that `prev` doesn't
match, the refused line was altered after it was written (a flipped bit in
its type reads like an unknown type): corrupt. If it matches, the refused
line is intact; the reader says so, and that it can't check the lines
after it. If there is no next line, or it has no `prev` this version can
read, nothing checks the refused line: the reader says it needs a newer
wanderbooks or the line is damaged, and to run verify after updating, and
never that it is intact. A newer format number is newer whatever the rest
of its declaration looks like (a newer format may shape its features
differently); a malformed declaration of a format this version reads is
corrupt. A blank line, or one that isn't UTF-8 JSON, is damage, to the
writer and the viewer alike: both read the journal as bytes split on
newlines. A writer never declares a format it couldn't read back, or
lowers the format in force. Opening the books checks every complete line
before it changes anything: a cut-off last line is set aside only when the
lines before it pass, so an older version never alters newer books.

**Enforced:** `ledger_store::format` (`JournalFormat::admit`, `declared`,
`gate`, `settle`, `declaration`), `journal::ENVELOPE_KEYS`,
`JournalLine::parse`, `journal::read_journal_and_format`,
`Snapshot::read_new`, `Store::open`, `Store::declare_journal_format`,
`StoreError::NewerJournal` (its `verified` flag picks the message).
**Checked:** `ledger-store` `tests/format.rs` (`old_reader_gets_newer_journal`,
`a_damaged_line_that_looks_newer_is_corrupt_when_the_next_line_checks_it`,
`an_unreadable_last_line_is_reported_as_newer_or_damaged`,
`a_newer_line_the_next_line_chains_onto_is_newer_and_intact`,
`a_newer_format_frames_its_lines_its_own_way`,
`an_unknown_envelope_key_is_newer_and_checked_through`,
`a_refused_line_followed_by_a_garbled_line_is_unverified`,
`a_line_that_is_not_utf8_is_corrupt_for_the_viewer_and_the_writer`,
`an_old_reader_never_repairs_newer_books`,
`a_blank_line_is_corrupt_for_the_viewer_and_the_writer`,
`damaged_journals_are_still_corrupt`) and the unit tests in `format.rs`;
`ledger-gui` `tests/newer_journal.rs`; `ledger-mcp` `tests/startup.rs`;
the `fuzz_store_replay` target (the writer and the viewer agree on newer
versus bad).

## B-28 What new commands take: four-digit years, amounts a line can carry

**Status:** CONVENTION (ISO 8601's basic calendar dates have four-digit
years, as do the spreadsheet and PDF formats the books export to);
DECISION (P0 review round 3, 2026-10-16, finding S1: the server stops
for every session when a request panics, so no input may reach one);
DECISION (P0 review round 4, 2026-10-17: posted is forever, so the
bounds are the commands', never replay's)

**New commands.** Every date a command takes or derives is in
0001-01-01 through 9999-12-31 (`ledger_core::calendar`): a date given
to a tool is exactly YYYY-MM-DD, a real day in that range (no sign, no
space, no fifth year digit); a period is a month of it; a new schedule's
last month, a new asset's last day of depreciation, a new recurring
entry's start and end, a new period, a new invoice's or bill's date and
due date, a disposal's date stay in it. Every amount a command takes,
not only a posted line's, is at most a journal line's
(1,000,000,000,000,000, `kernel::MAX_LINE_AMOUNT`): a draft line (by
size; its precision is checked when it posts), a document line, a
payout's lines and deposit, every payroll figure, a statement's
balances, a party's share, a contributor's basis, disposal proceeds, a
recurring template's lines. One run of the recurring entries creates at
most 1,000 drafts (`MAX_RECURRING_RUN`); a later date is run in steps.

**Replay accepts every line an earlier version accepted.** The bounds
above came after books were written, and posted is forever (B-8): a
journal an earlier wanderbooks wrote opens, in the writer and the
viewer alike, and replays to the same figures, forever. Replay applies
each check exactly as the version before the bound did: a journaled date
is any date chrono reads as `%Y-%m-%d` (chrono's own range, about
262,000 years either way, written "+10000-01-01" past 9999); a draft
line, a recurring template, a party's share, a contributor's basis, a
document line, a payment, a payroll run and disposal proceeds carry no
upper bound beyond what their own rules always said (positive, cents,
balanced, and so on); a schedule's months are 1 to 600 and an asset's
life ends on a date chrono holds. The checks split in two: the
command's (`Ledger::add_line`, `register_asset`, `TaxBasis::check`,
`shares::check_shares`, ...) and replay's (`replay_add_line`,
`replay_register_asset`, `TaxBasis::check_journaled`,
`shares::check_shares_journaled`, ...), and every bound has its pair of
tests: the command refuses past it; replay of an old line past it is
accepted.

**Derived figures refuse, never panic.** Because replay accepts what
the commands now refuse, every figure the books derive is computed with
checked arithmetic: a sum or product exact arithmetic can't hold is
refused with "too large to compute exactly" (`TooLargeToCompute`), and
a date past chrono's last day with `DateOutOfRange`, at the point of
computation (a draft's debits, a document's total and balance, aging,
net pay, a payroll plan, an entry's money flow and its shares'
resolution, the fixed asset register, an amortization plan), never
wrapped, rounded early or panicked on, and the server stays up. Date
walks reach chrono's last day without stepping past it (an asset's
depreciation in December 262142, an interval ending at
`NaiveDate::MAX`). A replayed line whose own figures overflow (a
recurring template whose debits Decimal can't sum: no version could
write it, and the last one panicked replaying it) is refused as the
books open, never a panic.

**Enforced:** `ledger_core::calendar`; `Ledger::try_create_entry`,
`add_line`, `add_line_fx`, `validate_new_asset`, `dispose_asset`,
`schedules::validate_schedule`, `validate_recurring`,
`periods::validate_period_range`, `arap::validate_lines`,
`validate_due_date`, `payroll::check_payroll_ranges`, `payout_plan`,
`shares::check_shares`, `TaxBasis::check`, `reconciliation_report`; for
replay, `replay_add_line`, `replay_add_line_fx`,
`replay_register_asset`, `replay_correct_asset`,
`replay_dispose_asset`, `replay_set_asset_tax_basis`,
`replay_set_entry_parties`, `replay_register_recurring`,
`replay_register_schedule` (`validate_schedule_shape`),
`replay_create_period` (`validate_period_order`), `replay_invoice`,
`replay_bill`, `replay_invoice_payment`, `replay_bill_payment`,
`replay_payroll_run`, `validate_new_asset_journaled`,
`validate_recurring_journaled`, `check_shares_journaled`,
`TaxBasis::check_journaled`; derived: `Invoice::total`/`balance`,
`Bill::total`/`balance`, `AgingRow::total`, `ar_aging`, `ap_aging`,
`PayrollLine::net_pay`, `payroll_entry_plan`, `parties::entry_flow`,
`shares::share_target`, `shares::resolve`, `fixed_asset_register`,
`schedules::amortization_plan`, `month_end`, `depreciation_schedule`,
`analysis::intervals`, `form1099::vendor_part`, `k1::share_property`,
`ReceiptPolicy::entry_debits`; `ledger-store` `parse_date` (replay:
chrono's range), `parse_period`, `last_day_of_month`,
`record_statement`, `dispose_preview`, `run_recurring`; `ledger-mcp`
`parse_date`, `parse_period_mcp`, `last_day_of_month`.
**Checked:** `ledger-core` `tests/spec_bounds.rs` (each bound and its
replay pair), `calendar.rs`, the unit tests of `amortization_plan`,
`intervals`, `vendor_part` and `share_property`, `assets.rs`
`registration_rejects_date_overflow`, `invariants.rs`
`out_of_range_amount_is_rejected`; `ledger-store` `tests/store.rs`
`books_an_earlier_version_wrote_open_to_its_figures` (every book in
`tests/fixtures/old-books`, written by main 188c90ea, opens in the writer
and the viewer and replays to exactly the figures main computed) and
`every_figure_over_old_books_is_computed_or_refused`, `tests/bounds.rs`;
`ledger-mcp` `tests/hostile_args.rs` (every tool with generated hostile
arguments and every resource with generated URIs: the server never
stops, and the books reopen after; the inputs that once panicked,
pinned; every tool over every old book) and `server.rs`
`periods_are_a_year_and_a_month`, `dates_are_four_digit_years_and_real_days`;
`ledger-gui` `tests/old_books.rs`.
