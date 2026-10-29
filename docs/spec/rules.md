# Rule catalog

Every rule the engine enforces, the authority for it, and what checks it.
Status is **AUTHORITY** (a source states the rule), **CONVENTION** (common
practice no authority states; the best reference is given), or
**CHOICE** (a human chose, and when; bookspec's CHOICE: the specification
names the options, an engine declares its pick). Vendored sources are in
`sources/`; FASB and OpenStax are quoted briefly under their copyright and
license (see `README.md`). Quotes from FASB's Codification were read in
Deloitte's DART roadmaps, which reproduce it ("FASB Accounting Standards
Codification: Copyright © 2005 - 2026 by Financial Accounting
Foundation").

Spec blocks in the code cite these as `SOURCE: rules.md B-n`. Every rule
also has a bookspec identifier, `**bookspec:** BS-<area>-<n>`: the table
from B-1 to B-45 is `bookspec/spec/rules-index.md` (P13, D-34).

---

## B-1 Every entry is a double entry: debits equal credits

**bookspec:** `BS-entry-1`

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
take postings when the line is added, not only at post: CHOICE
(the owner, 2026-10-13), `Ledger::add_line`
(`a_line_on_an_unknown_or_placeholder_account_is_refused_when_added`). **Checked:** Kani
`check_lines_is_exact`, `accepted_entry_nets_to_zero`; Lean
`Accepted.net_zero`; property `prop_posted_entries_balance`.

## B-2 A line's amount is positive; its side gives the direction

**bookspec:** `BS-entry-2`

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

**bookspec:** `BS-ledger-1`

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
match a bank statement: CHOICE (the owner, 2026-10-13), after the GnuCash
cross-check found wanderbooks kept $108.34567 where GnuCash booked
$108.35. Lines booked before then replay as booked.

**Enforced:** `Ledger::add_line` (base currency only, no fraction of the
minor unit), `Ledger::add_line_fx` (base amount = foreign amount × rate,
rounded to the minor unit; original kept), `Ledger::post_entry` (every
line, whatever made it: payroll, closes, disposals; `FinerThanMinorUnit`,
since 2026-10-13 after a payroll run posted $65.27794). Journals from
before replay as booked (`replay_post_entry`). A new asset's cost and
salvage are in the minor unit too (P3: every depreciation and disposal
line derives from them; `validate_new_asset`), and depreciation and daily
schedules round to it (B-15, B-16); assets an earlier version took finer
replay as journaled. **Checked:**
`an_asset_cost_finer_than_a_cent_is_refused`,
`a_foreign_line_is_booked_in_whole_base_units`,
`no_entry_posts_a_fraction_of_the_minor_unit`,
`foreign_currency_line_is_rejected`, `fx_overflow_is_an_error`; the
GnuCash oracle.

## B-4 Five elements, closed list

**bookspec:** `BS-ledger-2`

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
parent's type: CHOICE (the owner, 2026-10-13), `Ledger::add_account`
(`ParentTypeMismatch`). **Checked:** `a_child_account_has_its_parents_type`.

## B-5 Normal balances

**bookspec:** `BS-ledger-3`

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

**bookspec:** `BS-statement-1`

**Status:** AUTHORITY

Pacioli ch. 36: "the total of the credits — I do not say creditors —
should be equal to the total of the debits — I do not say debtors. If they
were not equal there would be a mistake in the Ledger." IRS Pub. 583 (B-1).

**Enforced:** follows from B-1. **Checked:** Kani `trial_balance_columns`;
Lean `trial_balance_iff`, `Reachable.trial_balance`,
`Reachable.trial_balance_through`; properties.

## B-7 The accounting equation

**bookspec:** `BS-statement-2`

**Status:** AUTHORITY

CON 8 Ch. 4 ¶E62 (cited): equity "is the difference between the entity's
assets and its liabilities." On a live book, income not yet closed is part
of equity (B-11).

**Checked:** Kani `accounting_equation`; Lean `accounting_equation`,
`Reachable.accounting_equation[_through]`; property
`prop_balance_sheet_balances`.

## B-8 Posted is forever: the book is written only forward

**bookspec:** `BS-ledger-4`

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

**bookspec:** `BS-ledger-5`

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

**bookspec:** `BS-ledger-6`

**Status:** CHOICE (the owner, 2026-10-13)

Follows from B-9 (a correction corrects once) and B-11 (a closed period
stays closed). **Enforced:** `Ledger::check_reversal`. **Checked:**
`an_entry_is_reversed_at_most_once`, `a_closing_entry_is_never_reversed`.

## B-11 Revenue and expenses close into profit and loss, and that into capital

**bookspec:** `BS-period-1`

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

**bookspec:** `BS-period-2`

**Status:** CONVENTION

Periods themselves: IRS Pub. 538 (`sources/irs-periods-and-methods.md`)
defines the tax year, calendar and fiscal. Locking a closed period and
closing in order are practice (and follow from B-8: a closed period's
book is not rewritten). Periods nest (a month in a quarter in a year)
and never partially overlap: CHOICE (the owner, 2026-10-13),
`Ledger::create_period` (`PeriodsOverlap`;
`periods_nest_but_never_partially_overlap`). **Enforced:** `closed_period_containing`,
`CloseOutOfOrder`, `Store::rehearse`. **Checked:** Lean `Steps.frozen`;
store property `refused_commands_never_reach_the_journal` (no draft in a
closed period).

## B-13 Partners share income by the partnership agreement; capital accounts carry it

**bookspec:** `BS-partner-1`

**Status:** AUTHORITY (shares; capital accounts; a count ends where the
members or shares change: §706(d)(1), Treas. Reg. §1.706-4(a)); CHOICE
(cent rounding, the owner 2026-10-13); CHOICE (counted from the company's
first close, D-10, 2026-10-16); CHOICE (the statement splits unclosed
earnings the same running way, D-38, 2026-10-20)

- 26 U.S.C. §704(a) (`sources/irs-partnerships.md`): "A partner's
  distributive share of income, gain, loss, deduction, or credit shall,
  except as otherwise provided in this chapter, be determined by the
  partnership agreement."
- Treas. Reg. §1.704-1(b)(2)(iv)(b) (`sources/irs-partnerships.md`): each
  partner's capital account "is increased by … (3) allocations to him of
  partnership income and gain" and "is decreased by … (7) allocations of
  partnership loss and deduction", beside the money and property
  contributed and distributed. A capital account is one running balance of
  contributions, allocations and distributions: the closes are the
  allocations.
- Form 1065 Schedule M-2: "Show what caused changes during the tax year in
  the partners' tax-basis capital accounts." Schedule K-1 Item L reports
  each partner's capital account "using the tax-basis method"; that
  analysis "is based on the partnership's books and records" (the Partner's
  Instructions' Basis Limitations section, `sources/irs-partnerships.md`;
  not Item L, which an earlier version of this rule spliced in). The K-1
  worksheet carries the book capital analysis, and beside it the tax-basis
  capital as far as the books know it (B-36).
- **No authority** says how to round a share to the cent. Largest
  remainder, ties to the member listed first: CHOICE (the owner,
  2026-10-13). Closes made before then replay with the last member
  absorbing it.
- **Members closes split cumulatively, counted from the company's first
  close** (P3 defect 17; D-10). A *count* is the members closes with one
  set of (capital account, share) pairs, from the first such close since
  the last members close with another set: closes to retained earnings or
  to one account are skipped (they allocate nothing to the members), and
  neither the members' order nor their distribution accounts matter. Each
  close gives member *i* allocate(C + net)*ᵢ* − given*ᵢ*: their split of
  everything the count has closed (C before it, plus its own net), to the
  base currency's minor unit (cents, whole yen, thousandths of a dinar;
  B-3; for a code ISO 4217 gives none, such as gold, the finest a line
  carries, `kernel::MAX_LINE_SCALE` places, so every amount is still whole
  units), less what the count's earlier closes gave them under whatever
  rule they were made. A tied odd unit goes to the member the count's
  first close listed first, whatever order later closes list the members
  in, so a reorder of the members moves no cent; the statement below
  breaks ties the same way, on a day before a count's first close in the
  order of the count the next members close begins. Therefore each close hands out
  exactly its net; after every close that posts, each member's total from
  the count is one split of the count's total, under one minor unit from
  their exact share; and rounding by closes made under earlier rules is
  trued up by the first close that posts (D-10: from inception; listed in
  the close's `members`). The true-up is under a unit per member for each
  break in an earlier run and each per-close close; a last-absorbs close
  rounded each member but the last to within half a unit, and the last
  absorbed up to half a unit for each of the others (a unit with three
  members, more with four or more).
  - A change of members or shares starts a new count (26 U.S.C.
    §706(d)(1): a year's distributive shares are determined by a method
    "which takes into account the varying interests of the partners in the
    partnership during such taxable year"; Treas. Reg. §1.706-4(a)(3)(vii):
    the partnership "shall treat each segment as though the segment were a
    separate distributive share period"). Income closed under
    the old shares is never split again under the new ones. So the bound
    holds per count: across k changes a member's total can be up to one
    unit per count from the sum of their exact shares (the D-10 lifetime
    bound holds while members and shares are unchanged).
  - A true-up rides only on a close that posts an entry (income to close,
    or a distribution to sweep): a close that posts nothing credits no one,
    and the true-up waits for the next close that posts (close INVARIANT
    5: never post a closing entry when there is nothing to close).
  - The bound's precondition: every amount is a whole number of minor
    units (B-3: every posted line is), and every product |total| × share
    is exact. A product Decimal would round (a share of many decimal places
    times a large total) is refused, never split
    (`periods::allocate_cumulative` INVARIANT 4). Replaying a journal from
    before B-3, a sub-unit fraction can make the bound two units.
  - A year's income per member is the difference of two running totals,
    each under a unit from exact, so under two units from share × the
    year's income; the return is in whole dollars ("Rounding on the
    return", `sources/irs-partnerships.md`).
  - A period closes at the shares in force at the close: a share change
    inside a period isn't split at its date (§706(d) and §1.706-4 would);
    close the period at the change.
  - Journaled `"rounding": "cumulative_since"` with `"since_seq": N`, the
    close the count starts at, under the journal feature
    `members_cumulative` (B-27); replay derives N again and refuses a close
    journaled with another. Closes journaled `"cumulative"` (the run rule of
    2026-10-13 to P3: a back-to-back run with the same members in the same
    order, restarted by any other close, so a tied cent could go to the
    same member at every restart), `"largest_remainder"` (each close on its
    own) and none (the last member absorbs) replay under their own rules,
    never changed (B-8).
- A capital account is one running balance (§1.704-1(b)(2)(iv)(b)). So the
  statement of members' equity, the K-1 worksheet and the viewer split
  undistributed earnings U as of each date the way the count's next close
  would split them: allocate(C(d) + U(d)) − allocate(C(d)), C(d) being what
  the count's closes dated on or before d closed (D-38; P3 review R-1). A
  member's capital on a date never moves when a period ending on or before
  it is later closed; a true-up of rounding by closes made under an earlier
  rule shows only once a close posts it, dated at that close. A
  statement's beginning is as of the day before its period, its ending as
  of its last day: a member's ending on one statement is their beginning on
  the next, to the unit, and a member's period income is the roll-forward's
  remainder (their share within a rounding unit at each end).
- A correction undoes what it corrects (B-9): an entry and its reversal
  both in a statement's span are neither contributions nor
  distributions (`reports::cancelled_in_span`; the K-1 worksheet reads
  the same). The K-1 worksheet's contributed property (Item L) is what
  the member owned and handed over, with its own basis (§723): a prepaid
  service contract credited straight to capital is property (a contract
  handed over), while a prepaid service they paid a vendor for on the
  company's behalf, booked through Due to Shareholders (Dr prepaid / Cr
  2100, then Dr 2100 / Cr capital), is cash (CHOICE, the owner,
  2026-10-15); and it is shared among
  members credited in one entry in proportion to their credits
  (`k1` INVARIANTS 3, 4, 6).

**Enforced:** `CloseTo::Members`, `CloseTo::MembersCumulative`,
`Ledger::members_regime`, `Regime::allocate`,
`periods::allocate_cumulative`, `kernel::apportion_cumulative`,
`periods::allocate`, `kernel::apportion`, `members_equity`,
`Store::close_period_to` (the feature it needs, B-27), the store's
`close_to_from_journal`. **Checked:** Kani `apportion_is_exact`,
`apportion_is_fair`, `apportion_cumulative_is_exact`,
`cumulative_close_never_drifts_a_unit_at_halves`, `_at_tenths`,
`_at_near_thirds`,
`cumulative_closing_entry_balances`; Lean `runPart_telescopes`,
`closePart_total`, `trueUp_total`, `trueUp_running`,
`closePart_eq_trueUp`, `cumulative_drift`, `statement_after_close`,
`statement_unmoved_by_close`, `Apportion.rem_pos_of_rank_lt`,
`apportion_drift`, `allocW_sum`, `allocW_drift`,
`allocW_cumulative_drift`, `allocW_cumulative_total`; properties
`prop_split_*`, `each_members_capital_chains_across_statements`,
`each_member_stays_within_a_cent_of_their_share`,
`members_capital_never_drifts_a_cent_from_their_share`,
`each_members_capital_chains_and_holds_through_closes`,
`period_ends_rise_with_close_order`, `apportion_cumulative_agrees`,
`allocate_cumulative_agrees_with_the_integer_model`;
`the_audit_odd_cent_never_piles_up_across_closes` and the rest of
`ledger-core` `tests/odd_cent.rs`; `a_run_of_closes_never_piles_up_rounding`,
`a_different_close_ends_a_legacy_run`,
`a_different_close_no_longer_restarts_the_count`,
`members_closes_replay_under_the_rule_they_were_journaled_with`,
`ledger-store` `tests/member_closes.rs` and the old-books fixture
`x_member_closes`; `closes_journaled_under_the_old_rule_replay_with_it`;
`ledger-mcp` `close_period_shows_each_members_running_split`,
`old_books_name_each_closes_rule_and_true_up_at_the_next_close`;
`ledger-gui` `a_members_page_shows_the_same_capital_before_and_after_a_close`,
`a_yen_members_page_splits_in_whole_yen`; the bookspec examples
`members-close.*`.

## B-14 Statements: P&L for a period; cash flows by the direct method

**bookspec:** `BS-statement-3`

**Status:** AUTHORITY, except where marked CONVENTION or CHOICE

The P&L covers a period; a closing entry is not income or spending
(B-11), so period statements exclude it.

The statement of cash flows (P3 defect 1, D-35, D-42; the paragraphs were read
in public sources that reproduce the Codification, `sources/fasb-asc.md`):

1. **Cash receipts and payments.** The statement reports "the cash
   effects" of operations, investing and financing (ASC 230-10-45-1; its
   objective, 10-1: "cash receipts and cash payments"). An entry
   contributes to the body only its net movement of the cash accounts,
   and what a member or a lender paid or received for it (rule 11, which
   nets to zero within the entry); an entry that moves neither
   contributes nothing; a transfer between two cash accounts is no flow. The period is [start, end], inclusive. The
   cash accounts are every account holding cash, cash equivalents or
   restricted cash: the statement explains the change in their total
   (45-4, as ASU 2016-18 amended it), and transfers among them "are not
   reported as cash flow activities" (45-5). Which accounts those are is
   the books' `books.cash_accounts`.
2. **Partition.** Every receipt and payment is investing, financing or
   operating (45-10), so every non-cash account is in exactly one section:
   investing (the cost and accumulated-depreciation accounts of every
   registered fixed asset, and `books.investing_accounts`); financing
   (equity, `books.financing_accounts`, and a liability that holds
   distributions declared, below); operating (every other balance-sheet
   account, and revenue and expense, but see rule 4). A liability left
   operating that, in the posted entries dated through the period's end
   (closes and reversals aside), only entries debiting nothing but equity
   credit, at least once (Dr Distributions / Cr Distributions Payable; a
   corporation's Dr Retained Earnings / Cr Dividends Payable), holds
   distributions declared and unpaid: paying it is a distribution
   (45-15(a): "payments of dividends or other distributions to owners"),
   listed as "Distributions to members — <the payable>" (the statement
   reads no parties, so not by member), and the declaration an exchange
   inside financing, disclosed (rule 7; 50-3). A liability credited by any
   other entry as well (what the members advanced, an accrued expense,
   money borrowed) stays where it is (P3 seam review, 2026-10-21: before,
   a distribution declared then paid read as "Cash paid to suppliers and
   employees", the declaration as "Other noncash items" beside an increase
   in the payable; `a_distribution_declared_then_paid_is_a_distribution_paid`,
   `a_distribution_declared_then_paid_is_financing_when_paid`). The
   roles must agree with the chart and each other, or the statement is
   refused with the setting to change: cash and investing accounts are
   assets; financing accounts are liabilities (equity listed is accepted:
   it is financing anyway); no code twice in a list or in two lists; no
   registered asset's account as cash; at least one cash account. One
   check (`cashflow::role_problems`) is behind both the refusal and
   `books.settings`' problems.
3. **Only the cash portion** of a part-cash transaction is reported
   (50-5). An entry's cash `c` is applied to the sections moving against
   it, in the order operating, investing, financing, each up to its own
   movement; what is left of a section's movement is the entry's noncash
   part, disclosed (rule 7). CONVENTION: the order (the ASC is silent on
   allocating within one entry); it matters only for an entry with a
   noncash source and two or more sections paid by cash, and is disclosed.
   Within a section the same order splits the cash among its kinds (rule
   5).
4. **Gains and losses on investing assets are investing**: receipts from
   sales of PP&E are investing (45-12(c)), so the whole proceeds are, and
   the gain or loss is taken out of net income (45-28(b)). In an entry that
   credits a registered cost account or an investing account, the revenue
   and expense lines are investing, except interest, received or paid (tax
   categories `interest_income` and `interest_expense`): interest is
   operating in any entry (45-16(b), 45-17(d)). The trigger is broad (a
   return, a reclass or an impairment credits the cost account too), so the
   reconciling line reads "(Gain) loss and other noncash changes in
   investing assets".
5. **Gross lines by kind** (45-7; investing and financing inflows and
   outflows "reported separately", 45-26). One line per entry, section and
   kind, with the entry, date, label and the kind's accounts. Operating
   lines carry the class 45-25 requires "at a minimum" (a)-(g), kind by
   kind within the entry, by the direction of the cash: the cash an
   entry's `interest_income` lines take is (b) interest received, and its
   `interest_expense` lines' (e) interest paid, whatever else the entry
   settles (a loan payment on an operating payable is (d) for the
   principal and (e) for the interest, never all (e)); the rest of its
   operating cash, by its other operating lines: a receipt is (a) if a
   line is revenue or the receivables control, or the entry's party is a
   customer, else (c); a payment is (g) if every such line is on an
   account whose tax category is `taxes_licenses`, else (d) if a line is
   an expense, a liability or a non-receivables asset, else (g); then,
   of a payment that is (d), what went to a government is (g): the
   entry names a party with the `government_agency` or `tax_agency`
   role (its chain root's parties, for a reversal), on any account. ASC
   230-10-45-17 lists "Cash payments to governments for taxes, duties,
   fines, and other fees or penalties" (c) apart from "Cash payments to
   other suppliers and employees for other goods or services" (b) (EY FRD
   p. 43, which reproduces the paragraph), and 45-25 names only income
   taxes, (f), among them, so the rest are other operating cash payments.
   The payee decides because the account need not: a state's filing fee
   for forming the company is an organization cost (tax category
   `organization_startup`), still paid to a government. Every party a
   government: the whole payment is (g). A government and others named
   together: the payment is split by their resolved shares (B-35), the
   government's part (g), the rest (d), because 45-22 has "each
   separately identifiable source or each separately identifiable use"
   classed "on the basis of the nature of the underlying cash flows" (EY
   p. 44; it speaks of the three activities, read here for the classes
   too, CONVENTION), and the shares identify each part; the government's
   part is the payment × its share ÷ the whole, rounded half to even to
   the base currency's places, the rest exact, so the two always sum to
   the payment and a reversal mirrors them (CONVENTION). Shares that
   don't resolve leave the whole payment (d). Not a government's: a
   government selling goods or services (postage, a park's entry fee) is
   45-17(b)'s supplier "for other goods or services", so such a payee
   is added as a vendor (the `vendor` role, books.add_vendor), never
   given `government_agency` (the books can't tell what a payment
   bought; the role says it). Payroll keeps its
   structure (CONVENTION): an entry with a line on the salary expense or
   a payroll payable control account (B-23's roles: `salary_expense`,
   `federal_withholding`, `state_withholding`, `social_security_payable`,
   `medicare_payable`, `other_deductions_payable`), or on an account of
   a pay tax category (`salaries_wages`, `officer_compensation`,
   `employee_benefits`, `pension_profit_sharing`), stays (d) whoever it
   names, because taxes withheld and remitted are the employees' pay
   (45-17(b), "to ... employees") and the payables mix them with the
   employer's own taxes, which the remittance can't tell apart; the
   employer's payroll tax paid on its own expense account is a tax, (g).
   As before this change, a remittance from the payables is (d); a
   payroll liability kept at a code of the chart's own needs its role
   (books.assign_role) to be read so. P3 cutover review, open point 2,
   2026-10-22: before, only the tax category counted, so a filing fee on
   an organization-costs account read as cash paid to suppliers
   (`a_payment_to_a_government_is_other_operating`,
   `a_government_payee_moves_a_fee_on_any_account`,
   `a_shared_government_payment_splits_to_the_cent`,
   `a_payroll_remittance_to_a_tax_agency_stays_pay`,
   `a_government_selling_services_is_a_supplier`,
   `a_reversal_takes_its_originals_payee`,
   `a_shared_government_payment_a_member_paid_splits_both_ways`, all in
   ledger-core `tests/spec_cash_flow.rs`;
   `a_government_part_rounds_half_to_even` and
   `a_government_part_sums_and_mirrors` in ledger-core
   `cashflow::tests`; bookspec `cash-flow.government-fees`). A
   government's payment a member paid on the company's behalf (rule 11)
   is classed and split the same way, its `on_behalf` part with it. (Interest paid back out
   is (g); interest refunded to the company, (c).) The operating cash is
   split among these kinds as a section's among its kinds (rule 3), in
   the order: the rest, interest received, interest paid (CONVENTION).
   Class (f) stays empty until a tax category marks an entity-level income
   tax (a
   partnership pays none "as such", IRC §701, but can under §6225 or a
   state pass-through election). Investing and financing kinds (D-35):
   purchases and sales of property and equipment, payments for and
   collections on each investing account, contributions from and
   distributions to each member (by name; equity not a member's by
   account), proceeds of and repayments of each financing liability. An
   original's direction is its nature (45-15(a): cash out of equity is a
   distribution). The printed statement and the viewer show operating one
   row per class, and investing and financing one row per kind with its
   total, each transaction listed under it (D-35); `books.cash_flow` and
   the exports carry every line. Known limits: an entry's other operating
   lines take one class, so its mixed receipts and payments other than
   interest net within it; class (b) relies on portfolio interest being
   categorized `interest_income` (interest receivable collected is (c)).
6. **Reversals.** A reversal takes its chain root's sections, class and
   kind, with its own sign (`Ledger::reversal_chain`). Pairing walks each
   chain from its root: an entry and its reversal both in the period drop
   out of every line and disclosure, when their flows are exact negatives
   (proved, and checked at runtime), so a reversal of a reversal whose
   partner is paired is shown. A reversal shown alone is labelled
   "Reversal of #N — <N's description>", a reinstatement "Reinstatement
   of #A (reverses #B)", listed under the original's kind: never a label
   from a reversal's sign (D1-c).
7. **Noncash investing and financing activities are disclosed** (50-3,
   50-4; placement, 50-6). Disclosed: every entry not netted whose noncash
   part moves financing, or moves investing through an account that is
   neither accumulated depreciation nor a rule-4 P&L line, or that
   exchanges inside investing and financing without cash (50-4's
   "converting debt to equity", a note for equipment, a trade-in): `x =
   max(0, min(D - out, C - in))` over the net debits `D` and credits `C`
   of its investing and financing balance-sheet accounts, `in` and `out`
   the cash those two sections took in and paid out, what a member or
   lender handled for the entry included (rule 11: equipment a bank paid
   for is 1,000 in and 1,000 out, nothing exchanged). Amount: `max(Σ
   max(r_k, 0), x)`. Depreciation is a
   reconciling item, not an activity. Limit: a trade-in within one cost
   account nets away.
8. **Net income reconciled to operating cash**, a separate schedule with
   the direct method (45-30; major classes of reconciling items, 45-29;
   what is removed, 45-28): operating = NI + G + X + W, over the period's
   non-closing entries. NI, net income; G, the rule-4 P&L lines; X, each
   entry's operating noncash part, as "Depreciation" (its investing and
   financing balance-sheet lines all accumulated depreciation), "Noncash:
   through <A>" (those lines, accumulated depreciation aside, all on A),
   or "Other noncash items"; W, minus each
   operating balance-sheet account's movement ("Increase in" / "Decrease
   in"). An identity over balanced entries (Lean
   `operating_reconciliation_ties`), checked at runtime. G's lines on a
   registered asset's depreciation expense account (a disposal's
   depreciation still due through its day, or what the month's run posted
   past it, credited back: B-15 rule 5) are shown in the "Depreciation"
   line, not the gain or loss, so that line is the P&L's depreciation
   expense for the period (45-28(a)'s depreciation is the expense
   recognized; the sum NI + G + X + W is unchanged). P3 seam review,
   2026-10-21: before, a disposal that credited back 25.00 of a month's run
   showed depreciation 25.00 above the P&L's and the loss 25.00 below the
   entry's (`a_disposals_depreciation_credit_back_is_depreciation`,
   `depreciation_is_one_figure_everywhere`).
9. **The sections foot** (45-24: the statement reconciles beginning and
   ending cash). `reconciles {sections_sum, net_change, difference}`; any
   difference, a section unequal to its lines, or a reconciliation unequal
   to operating is `CashFlowDoesNotFoot`, an engine error: nothing is
   produced. It can't happen on posted entries (Lean
   `cashflow_sections_sum_to_cash_change`, `Reachable.cashflow_foots`).
10. **Closing entries are never in the statement** (B-11). A close moves
    only revenue, expense and equity, and cash accounts are assets, so a
    close has no cash line.
11. **What a member or a lender pays, or receives, for the company is
    cash the company borrowed (or a member contributed) and spent**
    (CHOICE, D-42, the owner, 2026-10-20, revising D-35's first point;
    bookspec's CHOICE: the standard leaves it open).
    *Source.* The Codification does not address a payment a third party
    makes on the entity's behalf (PwC Viewpoint FSP 6.9.3: "ASC 230 does
    not address these situations"; BDO Blueprint 3.4: "While ASC 230 is
    silent"; EY FRD 4.6.2: "judgment will be needed"; KPMG Handbook
    Q8.3.10), so 105-10-05-2 sends the question to nonauthoritative
    guidance. There the firms present it by substance, as constructive
    receipt and disbursement: PwC 6.9.3.1, "a reporting entity should
    include cash flows received or paid by a third party on behalf of the
    reporting entity" (a bank paying a vendor: "a financing inflow ... and
    an operating outflow"); DART Roadmap 7.2 (third-party financing,
    Example 7-2: a lender paying an insurance premium, an operating outflow
    and a financing inflow; the contrast, a supplier's own finance arm, is
    AICPA TQA 1300.16 and the SEC staff's 2005 view: noncash); BDO 3.4
    (Example 3-5: a bank paying for equipment, an investing outflow and a
    financing inflow; it names "a parent might pay on behalf of a
    subsidiary expenses"); EY 5.2.4 and 5.3.1 (third-party financing,
    unlike the seller's, is a financing inflow and an operating or
    investing outflow). KPMG Q8.3.10 also accepts a noncash presentation
    for equipment a lender pays for. A CPA would present it as the firms
    do; the change in cash is the same either way. The rule, entry by
    entry, after its own cash (rule 3; `kernel::constructive`):
    - **Paid for the company.** A financing liability
      (`books.financing_accounts`: the members' financing account such as
      Due to Members, a loan, a card listed as financing) credited against
      operating or investing lines debited paid for them: a financing
      inflow under the liability's kind ("Proceeds from borrowing — 2100
      Due to Members", 45-14(b)) and the operating outflow (its class by
      rule 5) or the investing outflow (45-13(c)), both on the entry's
      date. A member's capital credited against revenue or expense debited
      paid for them likewise, a contribution ("Contributions from members
      — <member>", 45-14(a)): the expense-to-capital case below.
    - **Received for the company.** A financing liability or a member's
      capital debited against revenue, expense or the receivables control
      credited (a customer paying a member, a vendor refunding to a
      member's card): an operating inflow, and a repayment or a
      distribution (PwC 6.9.3.1: cash "received or paid").
    - Each line's `on_behalf` is the part of its cash so handled. The parts
      net to zero within the entry, so the sections still sum to the
      change in cash (rule 9; Kani `constructive_nets_to_zero`). Kinds take
      it in order, operating before investing and, within operating,
      balance-sheet lines before revenue and expense (CONVENTION, as rule
      3's: it matters only for an entry whose sources cover part of its
      uses, and then leaves capital the revenue and expense it can pay
      for). A reversal mirrors its root (rule 6; Kani
      `constructive_mirrors`), so a payment and its reversal net out, and
      a reimbursement is the repayment it is (45-15(b)).
    - **Stays noncash**, disclosed (rule 7): property a member contributes
      that it already owned (capital credited against an asset, a prepaid
      included: no cash moved, to anyone, 50-4); property handed over in
      kind (an asset credited against a financing liability or capital
      debited); a debt exchanged for another or for equity (50-4's
      "converting debt to equity"); interest and guaranteed payments
      charged into what the company owes the lender or member (tax
      categories `interest_expense`, `interest_income`,
      `guaranteed_payments`: their own charges, accrued, paid to no one;
      "interest paid", 45-25(e), is cash paid); and a purchase on the
      seller's own credit, an operating payable (50-4, "acquiring assets by
      assuming directly related liabilities"; EY 5.3.1), flagged
      `asset_on_operating_payable` (rule 12).
    - **An expense a member pays into capital** (Dr expense / Cr the
      member's capital, no cash) is a constructive contribution and an
      operating outflow. The member paid cash to a vendor for the company;
      whether the company owes it back (Due to Members) or not (capital)
      is only the form of the financing, not the substance of the payment,
      which the guides follow ("the same economic result", DART 7.2). BDO
      3.4 lists a parent paying a subsidiary's expenses among the cases
      for constructive cash flows without asking how the subsidiary
      records what it owes back (a parent's payment is often booked as a
      contribution), and none of the guides read presents an expense
      someone paid on the entity's behalf as noncash. The books' own K-1
      worksheet already counts it as a cash contribution (`k1.rs`
      INVARIANT 3, CHOICE 2026-10-15), so the two agree.
    - **Card purchases** follow the card's classification (D-35,
      unchanged): on a card listed as financing, the issuer is a lender
      that paid the merchant, so the purchase is borrowed and spent and the
      card payment a repayment; on a card that is an operating payable, the
      purchase is an accrual and its payment operating (d).
    - *Limits* (the books can't tell who was paid). A lender's or member's
      own charge credited to what the company owes them, other than
      interest and guaranteed payments (a card's annual fee, rent a member
      charges), reads as paid for the company: its operating outflow shows
      when charged, its repayment as financing (each section's total over
      the charge and its payment is right; the timing is the charge's).
      Post what the company owes a member for their own goods or services
      to a payable, not to the members' financing account. A note to the
      seller listed in `books.financing_accounts` reads as a lender's loan
      (borrowed and invested, where 50-4 makes it noncash): keep a seller's
      note out of it (its principal payments then show as operating, where
      45-13(c) makes them financing; flagged). An expense a member
      contributes in kind (an item already owned, expensed at once) reads
      as paid: post it to an asset first. A debt a member or a new lender
      pays off directly is a noncash exchange (disclosed), where PwC
      6.9.3.1 and BDO Example 3-6 would gross it up. Debt issue costs, a
      financing outflow (45-15(f)), posted to an expense account read as
      operating: post them to a contra-liability listed as financing, so a
      fee the lender withholds is a noncash exchange inside financing and
      the loan reads as its net proceeds. An entry that moves revenue or
      expense straight to capital by hand, as a close does, reads as paid
      or received for the company: close periods with `books.close_period`,
      whose entries are never in the statement (rule 10).
12. **Equipment paid for, or sold, later** (D-35; 45-12(c), 45-13(c)). A
    bill payment carries its bill's split, equipment part first: the
    payables line's equipment part is investing, so its payment is an
    investing outflow, the reconciliation's payables change leaves it out,
    and the purchase is disclosed only for what is unpaid at the period's
    end (a payment reversed by then is unpaid again; EY FRD 5.3.3; DART
    6.1.3; bills post to the general-expenses
    control today, so this waits for bills with account lines). A card
    purchase follows the card's classification: on an operating payable it
    is disclosed and flagged (`asset_on_operating_payable`), on a financing
    one it is borrowed and invested (rule 11) and its payment a repayment. A sale for an amount owed on an
    operating receivable is disclosed and flagged with the fix
    (`asset_sold_on_operating_receivable`: post it to an investing
    account). The flags are the statement's `warnings`, and items in the
    viewer's Needs attention. Limits: payments more than "soon after" purchase are
    financing by 45-13(c) when the seller financed it, which the books
    can't tell from a bill; unregistered equipment is operating (the asset
    register's `unregistered_cost` flags it).

Scope: single-currency books (R-5). A foreign-currency cash account's
exchange effect would be a separate reconciling line (830-230-45-1); a
hand-posted revaluation of a cash account shows as an operating receipt.

**Enforced:** `profit_and_loss_for_period`; `cashflow::cash_flow` (with
`kernel::entry_flows`, `apply_cash`, `constructive`, `exchanged`),
`role_problems`, `foot`; `ledger_package::cash_flow_for` (the server, the
package and the viewer). **Checked:** Lean `statements_add_up`,
`cashflow_sections_sum_to_cash_change`, `entry_flows_sum`,
`flows_reversal`, `constructive_nets`, `constructive_neg`,
`constructive_sections_net`, `stmt_flows_sum`, `stmt_flows_reversal`,
`operating_reconciliation_ties`, `cashflow_reversal_nets`,
`Reachable.cashflow_foots`, `Reachable.cashflow_is_change_in_cash`; Kani
`entry_flows_*`, `no_cash_no_flow`, `take_is_odd_and_bounded`,
`apply_cash_mirrors`, `apply_cash_spends_exactly`,
`exchanged_is_bounded`, `constructive_nets_to_zero`,
`constructive_mirrors`; `prop_period_pnl_partitions`,
`prop_cash_flow_laws` (5,000 cases, reversals of reversals, rule 11's
shapes), `prop_pure_entries_sections_are_movements`; regressions
`defect1_reversed_contributions_foot_regression`,
`defect1_two_member_llc_first_year_regression`,
`defect1_two_member_llc_as_of_march_18_regression` (and bookspec
`cash-flow.*`, `cash-flow.paid-on-the-companys-behalf` for rule 11,
`cash-flow.government-fees` for rule 5's governments);
`disposal_proceeds_all_investing`,
`an_expense_a_member_paid_is_borrowed_and_spent`,
`reimbursing_a_member_is_a_repayment_of_borrowing`,
`equipment_a_lender_paid_for_is_borrowed_and_invested`,
`an_expense_a_member_paid_into_capital_is_contributed_and_spent`,
`property_a_member_contributes_stays_noncash`,
`equipment_on_the_sellers_credit_stays_noncash_and_flagged`,
`a_payment_made_for_the_company_and_its_reversal_net`,
`part_paid_by_a_member_and_a_refund_a_member_received`,
`a_lender_pays_balance_sheet_lines_first_and_a_customer_pays_a_member`,
`interest_and_guaranteed_payments_charged_to_a_creditor_stay_noncash`,
`a_fee_withheld_from_a_loan_follows_its_account`,
`odd_shapes_foot_and_net`, `operating_classes_follow_asc_230_10_45_25`,
`a_payment_to_a_government_is_other_operating`,
`a_government_payee_moves_a_fee_on_any_account`,
`a_shared_government_payment_splits_to_the_cent`,
`a_payroll_remittance_to_a_tax_agency_stays_pay`,
`a_government_selling_services_is_a_supplier`,
`a_reversal_takes_its_originals_payee`,
`a_shared_government_payment_a_member_paid_splits_both_ways`,
`a_reversal_of_a_reversal_is_shown_and_foots`,
`a_reversal_three_levels_down_is_labelled_a_reversal`,
`operating_cash_is_classed_kind_by_kind`,
`two_kinds_of_one_class_are_one_line`,
`interest_paid_in_a_disposal_stays_operating`,
`a_loss_on_disposal_on_a_revenue_or_expense_account_is_investing`,
`an_investments_gain_is_investing`, `cash_goes_to_the_sections_in_order`,
`cash_goes_to_a_sections_kinds_in_order`,
`converting_member_debt_to_capital_is_disclosed`,
`a_bill_payment_carries_its_bills_split`,
`a_reversed_bill_payment_leaves_the_purchase_unpaid`, the role refusals,
`foot_refuses_any_difference`,
`a_distribution_declared_then_paid_is_a_distribution_paid`,
`a_disposals_depreciation_credit_back_is_depreciation`
(`crates/ledger-core/tests/spec_cash_flow.rs`); the seam review's year
of books (`ledger-package` `tests/p3_seams.rs`, its books
`tests/seams/books.rs`; the viewer's `ledger-gui` `tests/p3_seams.rs`;
the tools' `ledger-cli` `tests/p3_seams.rs`).

## B-15 Depreciation: straight line, salvage never depreciated, partial months by days

**bookspec:** `BS-asset-1`

**Status:** CONVENTION (book method); AUTHORITY for tax conventions, which
wanderbooks does not apply

OpenStax §11.3 (cited): "Straight-line depreciation is a method of
depreciation that evenly splits the depreciable amount across the useful
life of the asset," with "(Cost – salvage value)/Useful life"; partial
periods: "A common method is to allocate depreciation expense based on the
number of months the asset is owned in a year." wanderbooks prorates
partial months by days, including a schedule through a mid-month date
(CHOICE, the owner 2026-10-13). ASC 360-10-35-4 (read 2026-10-14,
`sources/fasb-asc.md`) prescribes no method: cost less salvage is spread
over the useful life "in a systematic and rational manner," as equitably
as possible to the periods served; straight line by days is one such
allocation.
Tax conventions (half-year, mid-month, mid-quarter): IRS Pub. 946
(`sources/irs-depreciation.md`), for the CPA.

The rule, exactly (P3, defect 22; the engineer settled it from ASC
360-10-35-4, 2026-10-20):

1. The life is `[in-service date, in-service date + life months)` (chrono
   clamps a day past a month's end: Jan 31 plus one month is Feb 28), cut
   into the calendar months it touches.
2. Through day `k` of a month but the life's last, the depreciation is
   (cost − salvage) / life × (the months before, each as days held over
   days in the month, + `k` / days in this month): the monthly rate by
   days.
3. The life's last month takes what the months before it leave, evenly
   over its days: through its last day the depreciation is exactly cost
   − salvage, and through an earlier day of it, prorated by days, never
   plugged to the whole. (Without clamping its daily rate is the first
   month's: the remainder is that month's missing days.) Before P3 any
   schedule through a day of the final month, and every disposed asset's,
   plugged to the whole: a disposal in the final month posted too much
   depreciation and too large a gain.
4. Each cumulative figure is rounded half away from zero to the
   currency's minor unit (cents; whole yen; a thousandth of a dinar;
   ISO 4217, `sources/iso-4217.md`), in exact integer arithmetic
   (`kernel::prorated_through`); a month's amount is the difference of
   consecutive ones. Through every month end this is the per-month
   running total books posted before P3, except where the exact figure is
   a half-cent tie, which the old 28-digit Decimal quotients could round
   either way; P3 rounds it up, exactly, moving that cent to the month
   before.
5. A disposed asset depreciates through its disposal day, held
   (inclusive): its schedule, accumulated depreciation and book value
   stop there, and equal what the disposal took. The disposal's book
   value is cost less the schedule through that day whatever the runs
   posted: depreciation still due posts as the final depreciation;
   depreciation the disposal month's run (dated its end) posted past the
   day is credited back to depreciation expense in the disposal entry
   (`excess_depreciation`); and the disposal clears what the runs carry,
   so the asset leaves the accumulated depreciation account at zero. A
   disposal is refused unless every run carries at the end of the
   disposal's month what it carries now (the disposal clears what they
   carry now): while a run of a later month stands (reverse it first:
   credited back at the disposal day, it would leave the disposal month's
   accumulated depreciation a debit and its expense short), or while a
   reversal (or re-instatement) of an earlier run dated after the month
   isn't undone (the disposal would charge that run again, and the
   reversal take it away only later; undone on any open day, so a closed
   month never blocks a disposal for ever). A change a run carries only
   at later month ends (a later month's run reversed after its month) is
   that run's error and its correction, dated when made, and the disposal
   leaves it as it is.
6. A month's run posts what the schedule says is due through the
   month's end, less what the books carry then, but never so much that
   the books would carry more than the schedule at a later month end
   (`kernel::catch_up`). What the books carry is read entry by entry, each
   in its own month (`assets::depreciation_effects`): a run adds its
   amount in its month, a reversal of it takes it away in the reversal's
   month. So a month skipped, or a run reversed, is caught up in the month
   the run is made (plan/served.md P3, settled in design, 2026-10-20: an
   immaterial miss; a material one is restated under ASC 250, a
   correction for a human and the CPA), and a month whose run was
   reversed after it ended is never run twice (it carried the month at
   its end; the run of the reversal's month catches it up). A month a
   later run caught up, or that rounds to nothing, posts nothing (an entry
   carries no zero line); a run after the life posts what is still due.
   A reversed run is un-posted (its month due again, when reversed within
   it); re-instating one (reversing its reversal) is refused when it would
   carry the books past the schedule (a later run caught its month up:
   `assets::catch_up_room`); a disposal, or a run its disposal took as
   final, can't be reversed; and a run, or a reversal of one, can't be
   reversed on a day before its own (a correction comes after what it
   corrects, B-9: a run of March reversed on Jan 31 would leave January
   and February with negative depreciation; commands only, journals
   replay as written). The viewer and the store read every run the books
   recorded (`Ledger::depreciation_runs`: each `asset.depreciation_posted`,
   as replay records it), not a month's latest (a run re-instated after its
   month was run again and that run reversed is the month's run).
7. Two dates (P3, defect 13). Depreciation starts on the day the asset is
   placed in service, never the day it was bought: ASC 360-10-35-4
   spreads cost over the periods the asset serves, and an asset serves no
   period before it is in service. (For tax, Pub. 946's "ready and
   available for a specific use", `sources/irs-partnership-basis.md`, is
   the CPA's.) The register holds an asset from the day the company
   acquired it: the date of the entry it is linked to (its purchase or
   contribution), never later than the in-service date, or the in-service
   date when no entry is linked (settled in design, 2026-10-20: "an
   asset's acquired date comes from the entry it is linked to"; one
   source of truth, so the register and the general ledger can't
   disagree, B-24). A new link to an entry dated after the in-service date
   or the disposal is refused, as is a new in-service date before the
   linked entry's date; a disposal can't come before the acquired date (a
   command's bound; replay accepts what earlier versions did, B-28), and
   one before the in-service date takes no depreciation.
8. Posted and scheduled. What the schedule says is due through a date
   (`assets::accumulated_scheduled_through`, the tools'
   `accumulated_scheduled_through`) and what the books carry
   (`Ledger::accumulated_posted`: the runs read entry by entry as rule 6
   reads them, each on its own day: a run adds its amount, a reversal of
   it takes it away, a reversal of that reversal adds it back, B-8, B-9;
   zero once the asset is disposed, its disposal entry having cleared it)
   are two figures, never one: book value is cost less the posted figure
   (`assets::scheduled_book_value` is the schedule's own, for the
   schedule only). An asset's depreciation expense over a span is its
   runs' (`Ledger::depreciation_posted_between`) plus its disposal entry's
   lines on its expense account (rule 5): the P&L's, the cash flow
   reconciliation's "Depreciation" (B-14 rule 8), the tax-basis K-1's
   open item (B-36) and the IIF's all show that one figure
   (`depreciation_is_one_figure_everywhere`). `books.list_assets` as of a
   day says "disposed" only for a disposal dated by then, as `held` and
   the figures do (B-33; before the seam review a later disposal showed
   on an earlier day).

**Enforced:** `assets::depreciation_schedule`, `assets::run_amount`,
`assets::catch_up_room`, `assets::depreciation_effects`,
`kernel::prorated_through`, `kernel::catch_up`, `kernel::true_up`,
`calendar::month_parts`, `Store::run_depreciation`,
`Store::dispose_preview` (`check_nothing_after_disposal`),
`Store::check_reverse_entry`, `Ledger::asset_acquired_date`,
`set_asset_acquisition` (`replay_set_asset_acquisition` for replay),
`check_correct_asset`, `dispose_asset` (strict: the acquired date),
`record_depreciation_run` / `check_depreciation_run`,
`accumulated_posted`, `live_depreciation_run`. **Checked:**
`schedule_sums_exactly_to_depreciable_amount`,
`salvage_value_is_never_depreciated`, `prop_depreciation_schedule_exact`,
`defect22_depreciation_through_mid_final_month_is_prorated_not_plugged`,
`defect22_final_month_drift_is_spread_by_days`,
`defect22_a_disposed_assets_schedule_stops_at_its_disposal_day`,
`defect22_a_disposal_in_the_final_month_posts_depreciation_by_days`,
`defect22_a_disposal_inside_a_posted_month_takes_back_the_extra_days`,
`a_run_reinstated_after_its_rerun_was_reversed_still_counts`,
`the_viewer_counts_a_run_reinstated_after_its_rerun_was_reversed`,
`a_reversal_dated_before_the_entry_it_reverses_is_refused`,
`a_disposal_is_taken_once_a_late_change_is_undone_after_its_month_closed`,
`a_disposal_whose_month_has_run_charges_what_is_still_due_in_its_entry`,
`every_order_of_runs_reversals_disposals_and_closes_keeps_the_books_right`,
`prop_any_sequence_keeps_the_books_right`,
`prop_the_viewer_reads_an_asset_as_the_store_does`,
`prop_run_amounts_are_each_months_run_amount`,
`jpy_depreciation_posts_whole_yen`, `a_skipped_month_is_caught_up_in_the_month_run`,
`a_reversed_depreciation_month_can_be_run_again`,
`a_run_reversed_after_its_month_is_caught_up_in_the_reversals_month`,
`reinstating_a_run_a_later_run_caught_up_is_refused`,
`a_disposal_before_a_later_months_run_is_refused_until_it_is_reversed`,
`a_disposal_before_a_later_reversal_of_an_earlier_run_is_refused`,
`a_disposal_entry_cannot_be_reversed`,
`prop_the_final_month_takes_the_remainder_by_its_days`,
`depreciation_effects_read_each_entry_in_its_month`,
`prop_depreciation_is_monotone_and_bounded`,
`prop_disposed_schedule_equals_what_disposal_computed`,
`prop_month_end_schedules_match_the_legacy_arithmetic_except_half_cent_ties`;
`the_charge_never_falls_and_is_continuous_over_real_calendars`; Kani
`prorate_parts_sum_to_total`, `prorate_last_part_absorbs_the_remainder`,
`a_months_charge_is_within_half_a_cent`,
`a_months_charge_in_tenths_is_within_half_a_cent`,
`the_last_months_charge_is_within_half_a_cent`,
`the_last_months_charge_in_tenths_is_within_half_a_cent`,
`prorated_through_refuses_malformed_input`,
`catch_up_never_overtakes_the_schedule`,
`catch_ups_is_every_suffixs_catch_up`, `true_up_reaches_the_target`;
Lean `Prorate.kernel_parts_total`, `Prorate.kernel_parts_nonneg`,
`Prorate.part_near`, `Prorate.cum_near`, `Prorate.chargeMid_eq_cum`,
`Prorate.chargeLast_eq_cum`, `Prorate.chargeLast_start`,
`Prorate.chargeLast_mono`, `Prorate.cum_mono` (`docs/verification.md`);
`ledger-core` `tests/spec_asset_basis.rs` (the in-service and acquired
dates, `accumulated_posted` and its edges,
`a_reinstated_run_counts_and_is_live`); `ledger-store`
`tests/asset_tags.rs` (`runs_replay_reload_and_verify_catches_a_tampered_run`,
`the_viewer_and_writer_agree_on_a_bad_depreciation_posted_payload`);
`ledger-cli` `tests/spec_assets.rs` (`a_reversed_run_is_not_posted`,
`list_assets_book_value_is_what_the_books_carry`).

## B-16 Prepaid expenses and deferred revenue are recognized over the period benefited or earned

**bookspec:** `BS-prepaid-1`

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

Conventions (P3, defect 9): `daily` or `monthly_even`. A new schedule
spreads by the day from its start (the first day covered), as
depreciation does (B-15 rules 1-4: full months equal, the first and last
partial months by their days, the last taking what the others leave,
rounded to the minor unit): a 12-month policy from March 15 posts
thirteen month ends, a small March and a small final March (CHOICE,
the owner, D-11 confirmed in D-37, 2026-10-20). Schedules registered before
P3 keep equal months, the last absorbing rounding, forever: a posted
month's replay re-derives its plan. `books.schedule_convention` sets the
convention new schedules take (unset: daily). An equal-months schedule
starts on the 1st of a month, for prepaid expenses and deferred revenue
alike (CHOICE, the owner, D-37, 2026-10-20): revenue is never recognized
before it is earned (ASC 606-10-25-23), and one rule for both is simpler
and never early; schedules an earlier version took mid-month replay as
they were. A reversed month is un-posted, so it is due again; a month in
a closed period, or one whose posting was reversed after the month ended
(posting it again on its own date would recognize it twice until the
reversal; a schedule posts its plan's months, so it can't catch one up
later), is left by a run for a human, listed with why, never a failed run.
A month's posting can't be reversed on a day before its own (a
correction comes after what it corrects, B-9; commands only). Not yet
vendored: ASC 606-10-25-31 (progress toward satisfying an obligation over
time), 606-10-25-32 (one method, applied consistently), 606-10-55-20 (time
elapsed as an input method): to read from the owner's ASC login before
B-16 cites them (P3 depreciation review R7).

**Enforced:** `schedules` (`daily_plan`, `amortization_plan`,
`validate_schedule`), `kernel::prorate_by_days`, `Ledger::reverse_entry`,
`Ledger::schedule_month_hold`.
**Checked:** `a_prepaid_expense_is_expensed_month_by_month`,
`deferred_revenue_is_earned_to_the_cent`,
`defect9_a_new_schedule_spreads_by_the_day`,
`defect9_deferred_revenue_daily_is_earned_to_the_cent`,
`defect9_legacy_schedules_stay_monthly_even`,
`an_equal_months_schedule_starts_on_the_first`,
`a_jpy_daily_schedule_posts_whole_yen`, `a_kwd_daily_schedule_rounds_to_the_fils`,
`a_reversed_schedule_month_is_due_again`,
`a_reversal_dated_before_the_entry_it_reverses_is_refused`,
`a_schedule_month_reversed_after_it_ended_is_left_for_a_human`,
`a_month_that_rounds_to_nothing_is_never_a_posting`,
`a_schedule_month_posted_again_waits_for_month_reposting_and_replays`,
`daily_plans_pin_month_end_clamping`,
`prop_daily_plan_sums_exactly_and_lands_on_month_ends`,
`prop_monthly_even_plan_is_unchanged`; `ledger-cli`
`mcp_daily_schedule`.

## B-17 Platform payouts are recorded gross when the business is the principal

**bookspec:** `BS-revenue-1`

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

**bookspec:** `BS-arap-1`

**Status:** CONVENTION (bucket edges); CHOICE (due today is Current)

ASC 326-20-55-37/38 (cited) illustrates "one way" to age trade
receivables with buckets "current … 1–30 days past due … 31–60 … 61–90 …
more than 90 days past due"; it is illustrative, not a rule. The FFIEC
Call Report instructions (bank regulators, not binding here): "The past
due status … should be determined in accordance with its contractual
repayment terms." A document due today is not yet past due: CHOICE
(the owner, 2026-10-13). Aging as of a date shows only documents issued by
then, less only payments made by then; and a payment never predates its
invoice or bill (a real prepayment is a deposit): CHOICES (the owner,
2026-10-13). What a document owes as of a date is read from its ledger
lines that stand then (B-38): a void invoice is in no bucket, a payment
reversed by then no longer counts, one reversed later still does; the
buckets partition what the documents owe, exactly.

**Enforced:** `arap::aging_bucket`, `Ledger::ar_aging`/`ap_aging`
(through `Ledger::doc_state`), `receive_payment`/`pay_bill`
(`PaymentBeforeDocument`). **Checked:**
`aging_boundaries`, `aging_is_exclusive_and_only_ages`,
`aging_as_of_a_date_sees_only_what_existed_then`,
`a_payment_never_predates_its_document`, `aging_as_of_ignores_a_later_reversal`,
`a_voided_invoice_never_ages`, `status_is_a_function_of_journal_and_date`.

## B-19 1099 reporting thresholds, payment methods, and decisions

**bookspec:** `BS-1099-1`

**Status:** AUTHORITY (`sources/irs-1099-thresholds.md`,
`sources/irs-1099-payment-method.md`), with the CHOICEs named below.

$600 through 2025; $2,000 for payments made after Dec. 31, 2025 (26 U.S.C.
§6041(a), §6041A(a)(2), as amended by Pub. L. 119-21 §70433; §70433(f):
"shall apply with respect to payments made after December 31, 2025"); for
calendar years after 2026, indexed to inflation and rounded to the nearest
$100 (§6041(h)). §70433 sets the one amount for Form 1099-NEC
(§6041A(a)(2), §70433(c)) and the §6041(a) payments on 1099-MISC alike.
Gross proceeds paid to an attorney (a payment in connection with legal
services that §6041 doesn't cover, a settlement paid to a claimant's
attorney, say: attorneys' fees for the company's own legal work are
§6041's, on Form 1099-NEC) stay at $600 (Treas. Reg. §1.6045-5(a)(1),
(c)(4)), and royalties report at $10 (§6050N(a)); neither is modeled, and
the worksheet says so.

**Who pays.** The payor is the person engaged in a trade or business who
makes the payment in its course (Treas. Reg. §1.6041-1(a)(1)(i)). A person
paying on behalf of another is the payor only if it performs management
or oversight functions in connection with the payment or has a
significant economic interest in it; "mere administrative or ministerial
functions such as writing checks at another's direction" are neither
(§1.6041-1(e)(1)). A member who pays a company bill from their own funds
at the company's direction, the company bearing the cost through Due to
Members, does only that: the company stays the payor, so a member's own
cash, check or transfer through a financing account counts.

**What is paid by card or network.** A transaction reportable under both
§6041 and §6050W "[is] reported under section 6050W and not section
6041", and for that test the third-party-network de minimis is
disregarded (§1.6041-1(a)(1)(iv); the 1099-MISC/NEC instructions, "Form
1099-K"). A payment card is "any card, including any stored-value card"
(§1.6050W-1(b)(3)); the IRS names credit, debit and stored-value cards
("Understanding your Form 1099-K"). The exclusion follows the instrument,
whoever's card it was: a member's personal card is excluded too. A payment
card transaction is one in which the card "is accepted as payment"
(§1.6050W-1(b)(1)) by the payee: a card that only funded a transfer the
payee didn't take as a card payment is not one (a member's
friends-and-family transfer is their cash). Automated clearing houses
"do not qualify as TPSOs" (Instructions for Form 1099-K): ACH, bill-pay
and Zelle are bank payments, not network payments.

**The rule (`form1099::vendor_1099_report`, `kernel::payment_bucket`).**
Payment methods are recorded once (`instrument.added`: bank,
business_card, member_card, member_cash_or_check, platform, each with the
account it pays through) and entries tagged with the one they were paid
with (`entry.instrument_set`, the reversal chain together). Each line of
a vendor-tagged posted entry has a role from its account: cash
(`books.cash_accounts`), financing (`books.financing_accounts`, and the
account of a member_card, member_cash_or_check or platform method: only
the entry's method says how a payment through it was made), a company
card's account (a business_card method's), or other. First match wins:

| The line | Bucket |
|---|---|
| the entry's method pays through this line's account, card or network | card (excluded) |
| the entry's method pays through this line's account, bank or a member's own cash or check | counts |
| a cash account (no method, or one naming another account) | counts: presumed check, ACH or transfer (CHOICE D-40, owner, 2026-10-20); tag a debit-card payment with the card |
| a financing account, or a member's or platform method's account (no method, or one naming another account) | undetermined until the method is recorded |
| a company card's account (a business_card method's) | card (excluded) |
| anything else (an expense, payables, prepaid) | no payment line |

Each entry's buckets are netted (`kernel::net_buckets`, P3 review R1):
money moved between the company's own sources pays no one, so the card
bill (Dr card / Cr bank), a member reimbursed (Dr Due to Members / Cr
bank) and a loan's principal net to zero; when the signs differ, the net
goes to the one bucket with its sign, else to undetermined. A bill is not
paid until it is; a prepayment counts when made (payments made in the
calendar year, §6041(a)); a worksheet as of a day (the CPA package's,
B-33) counts the year's payments from January 1 through that day; a
refund and a reversal net. Money received from a vendor for something else (the entry nets money
in and credits, beside its payment lines, anything a refund of what was
paid doesn't: revenue, receivables, a registered asset's cost account,
equity, or a liability other than payables; a sale, a customer's payment,
a disposal, a loan) is left out and listed, with its reversals (P3 review
R6); a refund credits only an expense, payables, or an asset no money is
paid out of and no one owes (prepaid, a deposit).
Books with no cash accounts named are refused (never "every asset", P3
review R5), as are books in another currency without a threshold given.
With cash accounts named and no methods or financing accounts, what counts
is exactly the old figure (credits minus debits to cash), but for money
received for something else.

**The decision.** Each vendor's 1099 decision is nec, misc,
not_reportable with a reason kind and the facts in words, or undetermined;
"not reportable" is never "undetermined" (plan defects 18, 23). No return
is required for payments of bills for merchandise, telephone, freight,
storage and similar (§1.6041-3(c)), or to a corporation (§1.6041-3(p)(1),
except attorneys' fees and medical and health care; a payor may treat a
payee as a corporation by its name "Incorporated, Inc., Corporation,
Corp., P.C., (but not Company or Co.)", a W-9 or a corporate resolution:
§1.6049-4(c)(1)(ii)(A)), a tax-exempt organization ((p)(2)), the United
States ((p)(3)), a State or its political subdivision ((p)(4)), a foreign
government ((p)(5)) or a documented foreign payee (§1.6041-4(a)(1)); an
employee's wages go on Form W-2 (§1.6041-2(a)(1)). Paid by card is never a
vendor's reason: it is a property of each payment. Every new decision
names who made it and when (`settings::Source`).

**Dated decisions** (CHOICE D-40, owner, 2026-10-20: "legally this has
to be 2 dated statuses"). Each decision takes effect on a date (a W-9's,
or a tax election's: Form 8832's effective date, §301.7701-3(c)(1)(iii)),
or from the beginning; each payment takes the decision in force on its
reversal chain's first entry's date. An LLC electing corporate taxation
keeps its EIN and its vendor record and changes on the election's date; a
payee that incorporates is a new legal person, a new vendor with its own
W-9. A decision written before decisions were kept (no date, no source)
is in force from the beginning, and an old null reads undetermined (plan
5.5). A dated decision with nothing decided before it is refused (it would
leave every earlier payment undetermined); one the same as the decision
before its date takes out any decision recorded for that date. A change
reaching payments in a closed period, or in a year whose 1099s were due
(January 31 of the next year), warns in `books.set_vendor_1099` and
`books.set_entry_instrument`.

**Status** (per vendor and year; T the threshold, F the payments made
while decided for a 1099, U those while undetermined, lo/hi each with
every reversal chain's unknown settled low or high, P3 review R3):
nothing in F or U is not_reportable when something was paid while not
reportable or the year-end decision is, else under_threshold; anything in
U is undetermined when hi(F) + max(hi(U), 0) ≥ T (a decision is needed),
else under_threshold; otherwise reportable when lo(F) ≥ T, under_threshold
when hi(F) < T, else undetermined. `reportable` is `status = reportable`.
A W-9 is on file by the vendor's flag or its party's W-9 facts.

An entry naming several parties (a payment shared between a contractor
and a member; `ledger_core::shares`) counts only the vendor's share
toward its 1099: what was paid to the payee (§6041(a)), not what the
entry moved for others. The share is resolved (`shares::resolve`)
against the entry's target (B-35: its money, or, when it moved none, its
parties' own-account movement; a reversal's is its original's negated),
the same target the MCP server checks shares against, then scaled to the
1099 payment (a ratio, so B-35's signs change no 1099 figure: a reversal
negates both the payment and the target): the share unchanged when the
payment equals the target (the usual case, an expense paid from cash or
by a member), else payment × share ÷ target, rounded to the nearest cent,
half away from zero (so a part on a card no method names scales out). The
vendor's part is split across the entry's netted buckets in proportion,
each rounded down to the cent and the cents short handed out by largest
remainder (`kernel::apportion`, P3 review R12), so they sum to it
exactly. A percent split's leftover cent goes to the party named first
(shares INVARIANT 8). A reversal carries the shares and subtracts the
same part, and re-apportioning either one re-apportions the other
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
unresolved`; a `books.vendor_1099` note). CHOICE (the owner,
2026-10-15: shares).

**The quieter note.** Untagged posted entries are named only when they
paid money out (netted counts plus undetermined above zero) for something
(a debit to an expense, to payables, or to an asset no money is paid out
of and no one owes), leaving out payroll runs, entries marked as having
no counterparty, entries naming only parties with no 1099 role, and
entries reversed in the year (P3 review R7): one count, the entries when
ten or fewer, and the tool to tag them.

**Cards and party totals.** The worksheet's card accounts are the
business-card methods' accounts only. Party money's cards are the same
once one is recorded (B-35 item 5: `parties::card_accounts`), and, while
none is, the liabilities named "card"; a charge to such a card leaves no
cash or financing account, so the worksheet counts it for no one either
way, and lists it as a card's only once the card is recorded. The
worksheet is never a party's totals: B-35 item 9 says how the two relate
(a member paying a payee through the members' financing is paid to the
payee in party totals, and the company's payment here), and they never
contradict.

**Enforced:** `form1099::default_threshold`, a dated rule
(`ledger_core::rules::THRESHOLD_1099`, known through 2026, review from
2026-11-01). For a later year the engine refuses with `RuleNotKnown`
rather than guess; the MCP digest and the viewer's "Needs attention" list
flag the rule once it is due (CHOICE, the owner, 2026-10-13). The
bucket table is `kernel::payment_bucket` (Kani `payment_bucket_is_the_table`,
`payment_split_is_exact`, `net_buckets_keeps_the_total`). **Checked:**
`an_unknown_year_threshold_is_never_guessed`,
`dated_rules_go_stale_on_their_review_date`,
`the_report_needs_the_cash_accounts_and_dollars`,
`payments_come_from_cash_and_financing_only`,
`undecided_is_not_not_reportable`,
`a_members_personal_card_payment_is_not_a_1099_payment`,
`card_account_payments_are_listed_not_dropped`,
`a_debit_card_payment_from_the_bank_is_excluded_when_tagged`,
`a_tag_naming_another_account_is_undetermined`,
`paying_down_a_financing_liability_pays_no_vendor`,
`paying_the_card_bill_pays_no_vendor`,
`a_negative_unknown_keeps_a_reportable_vendor_undetermined`,
`a_sale_to_a_contractor_does_not_lower_its_1099`,
`money_from_a_vendor_that_is_no_refund_does_not_lower_its_1099`,
`a_status_change_mid_year_applies_by_payment_date`,
`two_status_changes_in_a_year_and_a_reversal_across_them`,
`an_incorporation_on_december_30_is_a_new_vendor`,
`an_llc_electing_corporate_taxation_keeps_its_vendor`,
`a_dated_decision_records_a_change`,
`a_decision_change_names_the_payments_it_reaches`,
`an_undated_old_decision_is_in_force_from_the_beginning`,
`payment_bucket_is_the_table_in_all_16_cases`,
`a_shared_payment_splits_across_buckets_to_the_cent`, the properties
`no_instruments_no_financing_is_the_old_report`,
`buckets_partition_each_payment`, `a_reversal_nets_every_bucket_to_zero`,
`the_worksheet_ties_to_the_ledger`, `party_totals_are_the_1099s_buckets`
(ledger-core `tests/form1099.rs`);
`the_untagged_note_counts_only_money_out`,
`the_other_receipts_note_says_what_each_entry_did` (ledger-package); the
MCP tests through a real books server (ledger-cli); bookspec
`bookspec/examples/1099-*.toml`.

## B-20 Every transaction has supporting documents; records are kept

**bookspec:** `BS-ledger-7`

**Status:** AUTHORITY (`sources/irs-records.md`)

IRS Pub. 583: "In addition, you must keep supporting documents"; they
"support the entries in your books and on your tax return"; keep records
"until the period of limitations for that return runs out" (3 years
generally; 7 for bad debts).

**Enforced:** the MCP receipt policy (not the engine; R-13).

## B-21 Exact amounts in the books; rounding only on returns

**bookspec:** `BS-entry-3`

**Status:** AUTHORITY (for where rounding happens)

Form 1065 instructions (`sources/irs-partnerships.md`): "If two or more
amounts are added to figure the amount to enter on a line, include cents
when adding the amounts and round off only the total." The books keep
cents (and FX precision) exactly (R-2).

**Enforced:** `Decimal` money, `Amount` checked arithmetic, line range.
**Checked:** Kani `sums_are_exact`; bridge tests.

## B-22 Expense or capitalize (book policy, mirrors tax)

**bookspec:** `BS-asset-2`

**Status:** AUTHORITY (`sources/irs-capitalization.md`)

Treas. Reg. §1.263(a)-1(f)(1)(ii)(D) sets $500 per invoice or item for a
taxpayer without an applicable financial statement "or other amount as
identified in published guidance"; Notice 2015-82 raised it to $2,500 for
taxable years beginning on or after January 1, 2016. The 12-month rule:
§1.263(a)-4(f)(1). The election is made yearly on the return.

**Enforced:** bookkeeper policy (books' AGENTS.md), not the engine.

## B-23 Control roles: one account each, of the right kind, moved only when empty

**bookspec:** `BS-ledger-8`

**Status:** CONVENTION (a control account agrees with the records it
controls); CHOICES (the owner, 2026-10-13 and 2026-10-15)

Invoices, bills, payroll, and closes post to accounts the engine picks by
role: receivables, revenue, payables, general expense, retained earnings,
salary and payroll tax expense, and five payroll withholdings. A role uses
its default code (1200, 4000, 2000, 6999, 3999, 6100, 6110, 2110–2150)
until the human assigns it one of the chart's own accounts; the assignment
is journaled, so replay never depends on a setting outside the journal
(CHOICE, 2026-10-15). A role's account is postable and of the role's
type (B-4), and the engine refuses one that isn't rather than renumber
the chart (CHOICE, 2026-10-13). No account fills two roles. A
receivables, payables, withholding, or retained earnings account must keep
agreeing with what it controls (open invoices, open bills, taxes withheld,
closed earnings): such a role moves only when its account holds no
balance. For a withholding or retained earnings role the bookkeeper
first moves the balance with an entry, which the journal shows. Revenue
and expense roles move at any time; earlier postings stay where they
are. Receivables and payables are different (B-38, 2026-10-23): their
documents read their figures from their lines, so their control accounts
take no line by hand and equal their documents by construction. Their
balance moves with its documents only (plan P4b S7); until then a
receivables or payables role whose control account holds a balance stays
where it is, and the refusal says so (it names no hand entry, which the
control guard would refuse). Such a role also never moves onto an
account holding a line that is no document's, and the account it leaves
stays guarded while it holds its documents' lines. The account at a
role's default code is the role's control account only once a document
posts there (or the role is assigned to it): before that, a chart's own
1200 Equipment or a card at 2000 is the chart's.

**Enforced:** `Ledger::check_controls`, `Ledger::assign_role` /
`check_assign_role`, `Ledger::check_renumber_account` (an assigned role
frees its code); the control guard and `Ledger::control_check` (B-38).
**Checked:** `ledger-core` `tests/spec_roles.rs`, `tests/spec_documents.rs`;
`ledger-store` replay of `role.assigned`.

## B-24 Asset tags: one label per asset, never reused

**bookspec:** `BS-asset-3`

**Status:** CHOICE (the owner, 2026-10-15)

Each fixed asset may carry a tag, the label stuck on the thing itself
(generated `PREFIX-0001` or the human's own: 1–32 ASCII letters, digits,
`- _ . /`), and the manufacturer's serial number. Tags are unique ignoring
case and are never reused by another asset, even after a retag or a
disposal, so an old label can never point at the wrong thing. Both are
journaled. The fixed asset register lists every asset acquired by a date
(B-15) and reconciles to the general ledger's cost and accumulated
depreciation accounts: cost per register and per books
(`unregistered_cost`), accumulated depreciation by schedule and per
books (`unposted_depreciation`: run depreciation for it), and from the
assets' runs and per books (`unattributed_accumulated`: an entry made by
hand). Book value on the register is cost less the depreciation posted,
the books' figure, never the schedule's (P3, defect 13).

**Enforced:** `Ledger::set_asset_tag` / `check_asset_tag_for`,
`assets::check_asset_tag`, `assets::check_serial`. **Checked:**
`ledger-core` `tests/spec_asset_tags.rs`; `ledger-store`
`tests/asset_tags.rs`; `ledger-cli` `tests/spec_assets.rs`.

## B-25 Tax figures beside book value: the §723 basis, the depreciable basis, the §704(c) built-in gain

**bookspec:** `BS-asset-4`

**Status:** AUTHORITY (`sources/irs-partnership-basis.md`); CHOICE
(the owner, 2026-10-15: the register carries tax basis); CHOICE D-41
(the owner, 2026-10-20: the depreciable basis of property a member
contributed from personal use is the lesser of its fair market value and
the member's adjusted basis, the CPA to confirm at year end; the
§704(c) method is traditional, recorded as a choice the CPA confirms;
the contributed property's values on the contribution date are the
books' recorded amounts)

Book value and tax figures are different numbers for the same asset. The
book value of a member's contribution is its fair market value (Treas.
Reg. §1.704-1(b)(2)(iv)(d)(1), cited; Treas. Reg. §1.704-3(a)(3)(i):
"book value is equal to fair market value at the time of contribution").
Three tax figures, each named for what it is, follow from the rule and
the facts recorded (`asset.basis_set`: the rule, the contributing
member's adjusted basis, the fair market value at contribution, the
§704(c) method if chosen, and who gave them, B-29):

- **Purchase** (`purchase`): "The basis of property shall be the cost of
  such property" (IRC §1012(a)); depreciable basis is the cost. Not
  §704(c) property: no §723 basis and no built-in gain.
- **§723 basis** of contributed property: "the adjusted basis of such
  property to the contributing partner at the time of the contribution"
  (IRC §723; Pub. 541: "the partnership's basis for determining
  depreciation, depletion, gain, or loss"). Adjusted basis is cost plus
  improvements, less depreciation allowed but not less than allowable
  (IRC §1016(a)(1)-(2)). wanderbooks does not model §721(b) gain (an
  investment-company partnership only), so the §723 basis is the
  member's adjusted basis.
- **Depreciable basis:** for business property contributed
  (`carryover_723`), the §723 basis; for property a member contributed
  from personal use (`personal_use_conversion`), "the lesser of its fair
  market value or its adjusted depreciable basis" (Treas. Reg.
  §1.168(i)-4(b)(1); Pub. 946, "Property changed from personal use";
  D-41).
- **§704(c) built-in gain or loss at contribution:** fair market value
  less the §723 basis ("the excess of the property's book value over the
  contributing partner's adjusted tax basis upon contribution", Treas.
  Reg. §1.704-3(a)(3)(ii)); fixed at contribution, so correcting the book
  cost later never moves it. A built-in loss counts only for the
  contributing member (IRC §704(c)(1)(C)). The §704(c) method
  (traditional, traditional with curative allocations, remedial: Treas.
  Reg. §1.704-3(b)-(d)) is chosen property by property (§1.704-3(a)(2)).

The figures are at contribution (or purchase): tax depreciation reduces
the basis afterwards (§1016(a)(2)) and the built-in gain or loss "is
thereafter reduced" (§1.704-3(a)(3)(ii)); both are the CPA's (MACRS,
conventions, elections: `sources/irs-depreciation.md`). The facts are
kept, never the figures: the figures are derived when read, under the
rule in force, so a later change to a rule changes the reports without
refusing a line an earlier version wrote (B-28). A record made before
P3 (`asset.tax_basis_set`, with the old names `cost` and `carryover`)
has no fair market value recorded: it reads as the book cost, which
reproduces that version's figures exactly, and Needs attention asks for
the value. wanderbooks' book depreciation is GAAP straight line, not
§704(b) book depreciation, which must keep the tax depreciation's ratio
(Treas. Reg. §1.704-1(b)(2)(iv)(g)(3), cited): neither is the other.

**Enforced:** `kernel::basis_figures` (Kani `basis_figures_laws`),
`AssetBasis::check` (a purchase takes no contribution facts; a
contribution needs the adjusted basis and the fair market value, each
non-negative, in cents, at most one quadrillion) and its replay pairs
`check_journaled` and `check_legacy_journaled`, `Ledger::set_asset_basis`
/ `check_asset_basis` / `replay_set_asset_basis` /
`replay_set_asset_tax_basis`, `Asset::basis_figures`,
`reports::fixed_asset_register` (per-row tax fields,
`total_tax_basis_723`, `total_depreciable_basis`, `missing_basis`);
`ledger-store` `Store::set_asset_basis` (the journal feature
`asset_basis`); `ledger-mcp` `books.set_asset_tax_basis` and
`books.register_asset` (a contribution's fair market value is required,
never assumed). **Checked:** `ledger-core` `tests/spec_asset_basis.rs`,
`tests/kernel_bridge.rs` (`basis_figures_agree`), `tests/properties.rs`
(`prop_basis_figures_laws`); `ledger-store` `tests/asset_tags.rs`;
`ledger-cli` `tests/spec_assets.rs`.

## B-26 Parties: one directory, no full TIN in the books

**bookspec:** `BS-partner-2`

**Status:** AUTHORITY (truncated TINs: `sources/irs-truncated-tin.md`);
CHOICES (the owner, 2026-10-15)

Everyone the company deals with is a party, with roles (vendor,
contractor, customer, employee, member, payout platform, tax agency,
financial institution, landlord). Vendor and customer records link to
their party; entries are tagged with theirs. Names are unique among parties
not archived, ignoring case and surrounding space. A W-9 is recorded as its
facts and the TIN's last four digits (the part a truncated TIN shows,
Treas. Reg. §301.6109-4(a)); no party field may hold a nine-digit number,
and the full TIN lives only in the encrypted vault, outside the journal
(CHOICE). W-9 facts need a 1099 role (contractor, landlord); W-4 facts
need the employee role; payroll lines that name their employee name a
party with that role. A project's text (its name, description, links and
notes: B-37) holds no full TIN either (`settings::line_text`,
`long_text`). A link is a web address, so its rule reads it as one
(`settings::holds_printed_tin_in_link`, CHOICE of this rule, so store
listings can be kept): the printed SSN (3-2-4) and EIN (2-7) forms and a
bare nine digits standing alone are refused, but nine digits joined to a
letter or to `/ = ? & #` are an id in a path or query (an app store
listing's `.../id284882215`) and are kept. A project's name keeps the
plain rule: a nine-digit job or PO number is written by its last four,
as everywhere else in the books.

The full TIN, when the books need it for the 1099 worksheet, lives in the
TIN vault (`vault/` in the book's own folder in the data root, encrypted
under the book's own key, `keys/legacy.key` beside it): `books.set_w9`
`tin` writes it there and
journals only the last four and the vault record's hash; the signed W-9
goes there too, never to the attachment store. Any party may have its
TIN in the vault, whatever its roles: every member's (for their K-1) and
every 1099 payee's belongs there (CHOICE, the owner, 2026-10-15: "In the
vault, that's the WHOLE reason we built it. And it needs to be in the
damn CPA package."). `books.set_tin` stores one for any party; the books
journal its TIN facts (type, last four, the record's hash:
`party.tin_set`), which never disagree with its W-9 facts' last four. No
tool output, error, or log holds a full TIN except `books.vendor_1099`
with `include_tins` (CHOICE, the owner, 2026-10-15: for the CPA, when
asked): a refusal that quotes its input masks any TIN in it
(`parties::redact_full_tins`, checked by `refusals_never_echo_a_tin`),
and a party text field refuses nine digits however they are separated
(`has_full_tin`: up to three spaces, hyphens, dots, slashes or
underscores between neighbors, in any script's digits, a run of spaces
counting once however long it is; and a comma or a parenthesis between
a TIN's 3-2-4 or 2-7 groups (CHOICE, P7, 2026-10-26), so
`123,45,6789` and `(123) 45-6789` read as a TIN while an amount's
comma-grouped thousands do not, `1,234,567.89` being 1-3-3-2; past ASCII
every Unicode space and dash and the fullwidth and small forms count, and
invisible characters such as a zero-width space or a right-to-left mark
don't count toward the three: `sources/unicode-tin-text.md`, Unicode
17.0). Files holding one (`books.get_w9`, `books.vendor_1099`'s
`csv` with `include_tins`, and the CPA package, whose K-1 and 1099
worksheets carry every member's and payee's TIN by default) are never
written to a path: no tool takes one (D-29, P2 slice 6). Each is handed
over as an export the asking token fetches with `wanderbooks download`
(kept 15 minutes), never inline (D-44; since D-46 no tool takes or
gives any file's bytes inline: files come in only by upload and go out
only by download). (Until slice 6 they were
written only outside the books dir: CHOICE, P0 review, 2026-10-16, so
none could be filed back in as a receipt and read out again.) No file
holding a full TIN is filed into the books
(CHOICE, P0 review round 2, 2026-10-16): the server keeps, beside the
vault, the SHA-256 of every file it read or wrote with full TINs (the W-9
`books.set_w9` reads, recorded once the call has passed every check, just
before the vault keeps it, so a refused call marks no file; each
`books.get_w9` copy;
a 1099 worksheet with TINs; a CPA package with TINs and each file in it
that holds one), shared by every session and every later server for the
books, and every way a file enters the books (`books.attach_receipt`,
`books.file_document`, by upload, the only way since D-46) refuses those
exact files,
and refuses any text file (by its type or name, or because it decodes as
UTF-8 or UTF-16, with a byte-order mark or, NUL-interleaved, without one)
holding (a) a TIN the vault holds for a party of the
books, anywhere in the text, as nine digits in a row, as 3-2-4 digits
joined by hyphens, spaces, dots or commas, or as 2-7 digits joined by a
hyphen, a space or a comma (compared on the server against the decrypted
vault, never returned or logged; a vault that can't be opened leaves (b)
alone), or
(b) a number printed as an SSN or EIN is, standing alone: 3-2-4 digits
joined by hyphens or commas (123-45-6789, 123,45,6789) or 2-7 joined by a
hyphen or a comma (12-3456789, 12,3456789), with no digit, and no hyphen
or dot followed by a digit, on either side,
except the company's own EIN as its settings hold it, printed 2-7: it is
the company's, no person's, the settings already hold it in full (B-29),
and every company document goes into the books, its formation papers and
plans printing it (CHOICE, the owner, 2026-10-17); (a) still refuses it
should the vault hold that number for a party. Full-width digits and
Unicode hyphens and spaces count as their ASCII kin (`tin_files.rs`). The text check is narrow on purpose (CHOICE,
the owner, 2026-10-16): bank exports are filed for reconciliation, and
their routing numbers, long references and dates next to amounts are
not TINs; the broad `has_full_tin` reading stays on what the server says
(refusals and results). Because (a) compares against the vault, its
answer would tell an agent whether a text it chose holds a vault TIN;
the tools show each TIN's last four, so an agent filing texts full of
nine-digit numbers ending in those four could find the TIN in about 17
filings. So (a) answers for at most two candidates a filing (CHOICE,
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
party's vault records and journals that it did. Wherever the books show a
party's last four (the directory, `books.party`, the 1099 and K-1
worksheets, the purge preview) it is the TIN facts', else the W-9
facts' (`Party::tin_last4`; the two never disagree; exactly four digits
or nothing, however the facts were built), kept after a purge
(P3 defect 11: the directory read W-9 facts only, so a member's read as
missing).

**Enforced:** `crate::parties` (has_full_tin, check_text, W9Facts::check,
W4Facts::check), `Ledger::add_party`, `update_party`, `set_party_role`,
`set_party_w9`, `set_party_w4`, `set_party_tin` (TinFacts::check; TIN and
W-9 facts agree), `set_entry_party`, `validate_payroll`; `ledger-core`
`k1` (the K-1 worksheet); `ledger-vault` (Tin::new, the encrypted
records, records_held, purge_records); `ledger-mcp` `books.set_w9`,
`books.set_tin`, `books.get_w9`, `books.purge_tin`, `books.vendor_1099`,
`books.export_cpa_package` (open_vault opens the book's own key;
vault_tin checks the vault against the books); `ledger-mcp` `tin_files`
(the registry, the text
scan and its probe bound, on every filing tool; `known_tin_last4` gives
the books' last fours).
**Checked:** `ledger-core` `tests/spec_parties.rs`; `ledger-store`
`tests/parties.rs`; `ledger-vault` `tests/vault.rs`; `ledger-cli`
`tests/spec_parties.rs`, `tests/spec_vault.rs` (every response, every
file handed over, the server's log, and every file in the books' folder
outside `vault/` and the files kept for their download scanned for the
TINs in any formatting), `tests/tin_files.rs`
(`a_tin_bearing_file_is_refused_from_every_session_and_after_a_restart`,
`text_with_a_vault_tin_or_a_printed_one_is_refused_and_a_bank_export_is_not`,
`text_probing_for_a_tin_is_refused_alike_by_every_filing_tool`), and the
unit tests in `tin_files.rs`.

## B-27 The journal format gate: newer is not corrupt

**bookspec:** `BS-format-1`

**Status:** CHOICE (the owner, 2026-10-16; `plan/served.md` 5.1 and P0);
CHOICE (P0 review, 2026-10-16: a line's own hash is checked before the
line is called newer; the envelope and its hash are frozen across
formats); CHOICE (the owner, 2026-10-20, D-39, and `plan/served.md`
3.9: older books move to a newer format only with the owner's go-ahead,
asked at every open until given)

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

A line an earlier version can't read is announced by a journal feature
(`ledger_store::format::KNOWN_FEATURES`): a new event type, or a field or
value on an event type an earlier version knows but would misread. The
books declare it in a `journal.format_set` before the first line that
needs it, so an earlier reader stops at the declaration; this version
declares every feature it knows at once (below), never one at a time on
the way. P3's: `members_cumulative` (B-13: a members close counted from
its count's first close, `"rounding": "cumulative_since"`),
`account_renamed` (an `account.renamed`, B-31), `require_tax_category` (a
`settings.set` naming `policy.require_tax_category`, B-32),
`tax_categories_gain_exempt` (an `account.tax_set` naming
`form_4797_gain_loss` or `tax_exempt_income`, categories an earlier version
refuses), `disposal_before_service` (an `asset.disposed` dated before the
asset's in-service date, on or after the day it was acquired: an earlier
version refuses a disposal before its purchase date), `depreciation_catch_up`
(an `asset.depreciation_posted` whose amount isn't its month's own: months
skipped or reversed caught up in it, B-15; an earlier version reads it as the
month's own, shows the months it caught up as not yet run, and running them
there posts them twice),
`party_creditors` (`party.creditor_set`: defect 24, B-35),
`payment_instruments` (`instrument.added`, `entry.instrument_set`:
defect 12, B-19), `vendor_1099_decisions` (`vendor.1099_set` with who
decided, a `not_reportable` decision, its reason or the date it takes
effect, and `vendor.w9_flag_set`: defects 18 and 23, D-40, B-19),
`schedule_convention` (a daily schedule's `schedule.registered` carries
`convention: daily`; an older version would read it as equal months,
B-16; and a `settings.set` naming `books.schedule_convention`, a key an
earlier version refuses), `month_reposting` (a schedule month or a depreciation period
posted again after a reversal un-posted it; an older version would refuse
the second `schedule.posted`, and its projection ignore the second
`asset.depreciation_posted`, B-15), `asset_basis` (an
`asset.basis_set`, a new event type: an asset's basis facts with the
value at contribution and the §704(c) method, B-25) and `projects` (a
`project.set`, a new event type: a class's project facts; an
`attachment.linked` to the subject kind `project`, which an older version
would call corrupt; a `decision.recorded` with `projects`, which an older
version would drop silently: B-37). After P3: `document_line_projects`
(an `invoice.created` or `bill.created` line carrying `class_id`, its
project, B-37 item 8; an older version would restore the document without
it, silently). It is a feature of its own, not part of `projects`: books
that declared `projects` before it existed wait for their owner's next
upgrade before a document's line names a project. `documents_v2` (plan
P4b S4, one feature for S4 to S7: D-52) is the next: a `settings.set` of
the `invoicing.*`, `bills.*`, `company.email` or `company.website` keys,
which an earlier version refuses (S4.2); a `party.updated` with an email,
a default terms fact (S4.2), or a contact-block fact — a phone, a contact
person, a billing or shipping address, a billing email (D-55); an
`invoice.created` or `bill.created`
carrying a number, terms, a bill-to or seller snapshot, a customer
reference or memo, a vendor reference, a receipt date or a duplicate's
reason, or a line naming its account, quantity or unit price (S4.3);
S5's `invoice.sent` (who sent an invoice, how, to whom and when: a record
that moves no figure); S6's payments with `line`, `discount`,
`discount_line`, `discount_reason`, `fee` or `fee_line`; and S7's
`invoice.written_off`. An older version would
restore the document without them silently: every invoice line would post
to the revenue role and every bill line to general expenses, and every new
event type would be refused. A command asks for
the feature it needs (`Store::ensure_feature`) before its first journaled
line, and only when the line it is about to write uses it, so a write
refused for waiting on the upgrade journals nothing, never an entry
without the line that says what it is
(`a_write_waiting_for_the_upgrade_journals_nothing`), and a write that
needs nothing new is never refused. A tool journaling
several commands as one asks before the first of them, never between
them (a refusal there would leave the tool's work half done):
`books.set_w9` asks for `vendor_1099_decisions` before the W-9 when the
vendor's W-9 flag follows it. A value in a line of a known type that only
a newer version writes (a 1099 decision or reason kind, a payment
method's kind) is newer, never corrupt.

**The other way round: the format upgrade.** This version's format is
the newest it reads with every feature it knows (`format::current`). Books
it creates declare it in their second line, so they wait for nothing.
Books an older version wrote declare less (format 1, written before the
gate, declares nothing): they *wait for a format upgrade*, which is a fact
of their journal alone (`JournalFormat::upgrade`). They open and replay as
they are, byte for byte; everything that needs nothing new is written as
ever; a write that would need the newer format or a feature they haven't
declared is refused before anything is journaled
(`StoreError::FormatUpgradePending`, guard `format_upgrade_pending`), never
declared on the way. Every setting key and tax category an earlier version
didn't read needs a feature (`format::setting_feature`,
`format::tax_category_feature`: the P3 data-safety review, 2026-10-21,
found `books.schedule_convention`, both new tax categories, a disposal
before the in-service date and a depreciation catch-up journaled without
one, each leaving books P2 refused or misread before consent;
`tests/p2_compat.rs`). Only the owner's go-ahead moves them, with the owner
passphrase (the server checks it, Argon2id): in the app, which asks at
every open until it is given, or with `wanderbooks books upgrade ID` at a
terminal, whose commands that open the books also ask; never an agent, and
no tool. Before the upgrade the server commits the books to their own
backup repository and proves the copy holds them exactly as they are
(journal, stored files, vault and keys, byte for byte), all under the
books' lock; a backup that can't be made or proven upgrades nothing. The
upgrade is then one `journal.format_set` declaring the whole format, so a
book's format moves exactly once per version; books already in it are left
alone. An adoption's copy is new books of this version and moves to its
format as it is made (the original keeps its own). The upgrade never
weakens the gate: books declaring a feature this version doesn't know are
newer, as above.

**Each projection schema in a file of its own** (the lead's decision,
2026-10-21, for 3.9's "before consent, the way back is always there"). The
books' SQLite projection is derived from the journal, and its schema moves
with the version (`schema::SCHEMA_VERSION`), but an older version refuses a
database of a newer schema (`SchemaTooNew`). So each schema keeps its own
file, `wanderbooks.v{N}.db` (`store::db_file_for`): a version opens or
creates only its own; its own missing, it is built from the journal; the
legacy `wanderbooks.db` (every version through P2, schema 12) is used only
while it already holds this version's schema, so books needing no
migration aren't rebuilt for nothing; a file of any other schema, older or
newer, is never migrated, written, refused or deleted, and stays byte for
byte for the version that wrote it (going back finds it as it left it, or
builds its own). A file named for this version's schema that holds another
is damaged, kept aside and rebuilt, as a corrupt one is. `books.verify`
(`database`, `other_databases`) and `wanderbooks doctor` say which file is
in use and which others are there; none is deleted for being stale.

**Enforced:** `ledger_store::format` (`JournalFormat::admit`, `declared`,
`gate`, `settle`, `declaration`, `current`, `JournalFormat::upgrade`,
`require_format`, `require_feature`), `journal::ENVELOPE_KEYS`,
`JournalLine::parse`, `journal::read_journal_and_format`,
`Snapshot::read_new`, `Store::open`, `Store::create`, `store::db_file_for`,
`store::database_in_use` (and `Snapshot::check_disk`, which checks the file
it names), `Store::database_file`, `Store::other_databases`,
`Store::declare_journal_format`, `Store::upgrade_format`,
`Store::ensure_feature`, `Root::adopt`, `StoreError::NewerJournal` (its
`verified` flag picks the message), `StoreError::FormatUpgradePending`,
`ledger_store::backup::prove_archive`, ledger-mcp `Backups::upgrade_format`
and `local.rs` (the passphrase), ledger-host `host.rs` (the lease intent,
handed to the server) and `commands.rs` (`books upgrade`, asking at open),
ledger-gui `owner.rs` (the notice and the dialog).
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
`ledger-gui` `tests/newer_journal.rs`; `ledger-cli` `tests/startup.rs`;
the `fuzz_store_replay` target (the writer and the viewer agree on newer
versus bad). The upgrade: `ledger-store` `tests/format_upgrade.rs` (new
books in this version's format, a change waiting for the upgrade, every
old-books fixture upgrading to its figures, a newer feature still
refused), the unit tests in `format.rs` and `backup.rs`
(`the_archive_proves_to_hold_the_book_as_it_is_keys_and_all`),
`tests/settings.rs` (`a_change_needs_journal_format_2`), `tests/root.rs`
(adoption), `tests/schema_files.rs` (a P2-shaped books dir, its
`wanderbooks.db` used at its schema and left byte for byte past it; an
older schema's file left byte for byte and whole while this version's is
built and kept up; a newer schema's file left alone, not refused; a damaged
file of this version's schema rebuilt; new books' file named for their
schema); `ledger-mcp` `local.rs` (`format_upgrade_waits_for_consent`,
`no_upgrade_without_a_backup`,
`no_upgrade_unless_the_backup_just_made_is_proven`); `ledger-host`
`lease.rs` (`a_consent_never_shows_its_passphrase`); `ledger-cli`
`tests/format_upgrade.rs`
(from end to end); `ledger-gui` `tests/format_upgrade.rs`.

## B-28 What new commands take: four-digit years, amounts a line can carry

**bookspec:** `BS-entry-4`

**Status:** CONVENTION (ISO 8601's basic calendar dates have four-digit
years, as do the spreadsheet and PDF formats the books export to);
CHOICE (P0 review round 3, 2026-10-16, finding S1: the server stops
for every session when a request panics, so no input may reach one);
CHOICE (P0 review round 4, 2026-10-17: posted is forever, so the
bounds are the commands', never replay's)

**New commands.** Every date a command takes or derives is in
0001-01-01 through 9999-12-31 (`ledger_core::calendar`): a date given
to a tool is exactly YYYY-MM-DD, a real day in that range (no sign, no
space, no fifth year digit); a period is a month of it; a new schedule's
last month (a daily one's last day), a new asset's last day of
depreciation (so a life ending on 9999-12-31 is taken, and one a day
later refused: P3 settled the plan's defect 22 question this way, lives
past 9999 refused at registration, the bound the life's last day held;
ones an earlier version took open, and their months after 9999-12 show
as scheduled and never post), a new recurring
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
command's (`Ledger::add_line`, `register_asset`, `AssetBasis::check`,
`shares::check_shares`, ...) and replay's (`replay_add_line`,
`replay_register_asset`, `AssetBasis::check_journaled`, `AssetBasis::check_legacy_journaled`,
`shares::check_shares_journaled`, ...), and every bound has its pair of
tests: the command refuses past it; replay of an old line past it is
accepted. The full-TIN rule (B-26) is one of these bounds: P3's detector
finds a TIN across every Unicode space and dash and invisible characters,
P2's only across ASCII separators, the no-break space and a few dashes, so
replay finds one as P2 did (`ledger_core::parties::replaying`, around every
replayed line), and a name, reason or fact P2 took opens
(`a_name_p2_took_replays_though_the_command_now_refuses_it`; the P3
data-safety review, 2026-10-21, found P3 refusing to open such books). What an earlier version left beside its journal is kept for it
the same way: its projection database, in its own schema's file, is never
migrated or written by a later version (B-27, "each projection schema in a
file of its own"), so the earlier version, gone back to, opens its books
as it left them.

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
`shares::check_shares`, `AssetBasis::check`, `reconciliation_report`; for
replay, `replay_add_line`, `replay_add_line_fx`,
`replay_register_asset`, `replay_correct_asset`,
`replay_dispose_asset`, `replay_set_asset_tax_basis`, `replay_set_asset_basis`,
`replay_set_entry_parties`, `replay_register_recurring`,
`replay_register_schedule` (`validate_schedule_shape`),
`replay_create_period` (`validate_period_order`), `replay_invoice`,
`replay_bill`, `replay_invoice_payment`, `replay_bill_payment`,
`replay_payroll_run`, `validate_new_asset_journaled`,
`validate_recurring_journaled`, `check_shares_journaled`,
`AssetBasis::check_journaled`, `AssetBasis::check_legacy_journaled`; derived: `Invoice::total`/`balance`,
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
`ledger-cli` `tests/hostile_args.rs` (every tool with generated hostile
arguments and every resource with generated URIs: the server never
stops, and the books reopen after; the inputs that once panicked,
pinned; every tool over every old book) and `server.rs`
`periods_are_a_year_and_a_month`, `dates_are_four_digit_years_and_real_days`;
`ledger-gui` `tests/old_books.rs`.

## B-29 The books keep how they are kept

**bookspec:** `BS-ledger-9`

**SOURCE:** CHOICE (plan/served.md I-3, owner decisions of 2026-10-16:
"the journal holds all books state"; light governance: decisions are
recorded with who made them and when, not as paperwork); IRS Pub. 538 and
the rules of B-1 to B-28 for what each setting means.

A company's settings, its conventions (how it posts each kind of
transaction, its policies and close steps), the decisions its owners made,
what is open, and approvals of draft entries are facts about the books,
kept in the journal like every other fact, each with who decided it and
when (journal format 2: `settings.set`, `convention.set`,
`convention.retired`, `decision.recorded`, `open_item.raised`,
`open_item.resolved`, `entry.approved`). A settings file is only text to
import from or export to; no server reads one (D-44).

- A setting is one of the known keys, checked by its key and kept in one
  spelling; a change is all or nothing, and the settings after it hold
  together. Replay takes any value of the right shape (B-28).
- No setting, convention id, convention, decision, open item or approval
  holds a number that reads as a full TIN (nine digits standing alone,
  bare or printed 3-2-4 or 2-7 with a dash or a space), except the
  company's own EIN, shown masked everywhere but reports, packages and the
  viewer, and a D-U-N-S number in its own key.
- A record of how the books are kept changes no entry, draft, period or
  account (proved for the model: `Step.record_changes_no_figure`; checked
  on the engine after every record in `settings_replay_is_pure`). Settings
  that say how the same entries are presented or later allocated (the
  fiscal year, the cash flow roles, the members) present them by the
  settings in force; the fiscal year can't move once a period is closed
  under it (a changed tax year needs the IRS's consent, Form 1128).
- Books keep their own settings, and only `books.set_setting` changes
  them; old books bring their settings file in once, whole, as they are
  adopted (`wanderbooks books adopt`), and nothing reads one after.
- A setting key or convention kind this version doesn't know is a newer
  version's: the books are "newer", never "damaged" (B-27).
- A posting template's lines split the amount exactly on each side, odd
  cents by largest remainder (B-13's rule); a slot (`{expense}`) takes the
  account each draft names.
- Approval rules block anything only when the company enforces them
  (`policy.approval_rules = enforce`; off by default). Then a draft a rule
  covers posts only once its approver approved it as it stands (its
  fingerprint: date, description, every line and class), and an operation
  that would post such an entry at once (an invoice, a bill, a payment,
  payroll, a remittance, a disposal) is refused: it is drafted instead.
  Reversals, closes, depreciation and amortization follow from entries
  already posted and are exempt. A rule names accounts that take postings,
  and an account a rule names can't be renumbered until the rule is
  revised.

**Enforced:** `ledger_core::settings` (`check`, `Settings::check_changes`,
`check_whole`, `shape`, `apply`), `ledger_core::conventions` (`check_new`,
`check_decision`, `check_open_item`, `split`), `Ledger::check_settings`,
`check_convention`, `approvals_missing`, `template_lines`; `ledger-store`
`Store::change_settings`, `adopt_settings`, `set_convention`,
`draft_from_template` (rehearsed whole), `record_decision`; the format gate
(format 2 declared before the first such line); `ledger_config::codec`
(`import`, `export`); `Store::post_entry` and `approval_gate`
(`ApprovalRequired`, `ApprovalNeedsDraft`), `check_renumber_account`;
`ledger-mcp` adoption (`local.rs` `settings_to_move`: a settings file,
whole, through `codec::import`), `books.post_entry`
(`approval_required`).
**Checked:** `ledger-core` settings and conventions unit tests;
`ledger-store` `tests/settings.rs` (`settings_replay_is_pure`, 256 cases
per check, 2,000 by hand, with posts between records and every record
checked to change no figure or entry; `approval_rules_off_by_default`,
`approvals_cover_every_post`, `the_fiscal_year_stays_under_closed_periods`,
`a_newer_key_or_kind_is_newer`;
`books_written_under_p1_open_to_their_record`, fixture
`old-books/p1_record`); `ledger-config` `tests/codec_props.rs`
(`exports_round_trip`) and codec unit tests; `ledger-cli`
`tests/spec_kept.rs`; Lean `Step.record_changes_no_figure`,
`Reachable.records`; fuzz `fuzz_settings_replay`.

## B-30 A report asked for without dates covers this fiscal year so far

**bookspec:** `BS-period-3`

**Status:** AUTHORITY for the period (IRS Pub. 538,
`sources/irs-periods-and-methods.md`: "A tax year is an annual accounting
period for keeping records and reporting income and expenses"; 26 U.S.C.
§6041(a), "in any calendar year", and §6041A(a)(2), "during such calendar
year", `sources/irs-1099-thresholds.md`, for the calendar-year counts);
CHOICE for using it as the default (no authority says what a tool's
omitted dates mean; the owner, 2026-10-20, `plan/served.md` P3 "Settled in
design": "a report asked for without dates covers this fiscal year so
far, everywhere, and an account's register opens on it with 'Show all
dates' one click away").

The books' annual period is the fiscal year (`company.fiscal_year_start`,
B-29). Every place a date may be omitted resolves it by one rule
(`ledger_core::calendar::PeriodDefaults`), so the agent's report tools,
`books.show`, the viewer and the tool descriptions can't say different
things:

| Kind | Used for | Both dates omitted | Only `end` given | Only `start` given |
|---|---|---|---|---|
| A period | profit and loss (and by class), cash flow, tax summary, members' equity, general ledger, general journal (csv, json, xlsx), an account's register, `books.show` of an account or a report | the first day of the fiscal year that holds today, through today | the first day of the fiscal year that holds `end`, through `end` | `start` through today |
| An instant | balance sheet, trial balance, A/R and A/P aging, fixed asset register, the IIF's `as_of` (every posted entry from the first through it, B-34), the CPA package's `as_of` | today | | |
| A calendar year | party totals (`books.parties`, `books.party`, the viewer's party pages), the viewer's 1099 worksheet | today's calendar year | | |

- A resolved start after the resolved end is refused, whether the caller
  gave both dates (`InvalidDateRange`) or only a start after today, the
  end an omitted end means (`StartAfterToday`, whose message says the end
  was defaulted: "pass end"). It is never swapped or clamped to fit.
- A derived start before 0001-01-01 is the calendar's first day (only an
  end early in year 1 under a non-January fiscal year, B-28).
- The first year: when the company was formed after its fiscal year's
  first day, the default start comes before formation. The amounts are
  the same, and the start is not moved to `company.formed`: start-up and
  organizational costs (IRC §195, §709) can be dated before formation.
- "Today" is the company's (`company.timezone`, B-29).
- The CPA package's period and the viewer's "Year to date" preset are
  the fiscal year that holds their day (`calendar::fiscal_year_start_on`),
  the fact itself, never "what an omitted date means": they don't follow
  this default if it ever changes.
- Not covered (no default period): search filters, the activity digest
  (its `since` is required; the `books://digest` resource reads this
  month so far), the `through` of `books.run_schedules` and
  `books.run_recurring` (today: an operation, not a period), and the
  viewer's drill-downs from a balance, which run from the books' first
  entry because a balance is cumulative. The IIF is never a period (B-34):
  it refuses `start` and `end`, so the general journal's default period
  never shortens it, nor anything in the CPA package, which gives its
  statements and ledgers its own fiscal year to date.

**Enforced:** `ledger_core::calendar` (`period_defaults`,
`PeriodDefaults::statement`, `statement_range`, `as_of`,
`calendar_year`, `fiscal_year_start_on`, `PeriodKind::describe`,
`START_DOC`, `END_DOC`, `AS_OF_DOC`, `YEAR_DOC`); `ledger_package`
`statement_range`, `as_of_date`, `pnl_statement`, `bs_statement`,
`cash_flow_statement`, `build_package`; `ledger-mcp` `BooksServer::periods`,
`statement_range`, `as_of`, every report tool, `books.show`, the
generated descriptions (`described`) and parameter docs; `ledger-gui`
`Books::periods`, `default_period`, `fiscal_ytd`, `App::show`,
`RegisterRange`. **Checked:** `ledger-core` `calendar.rs` unit tests and
`tests/properties.rs` `prop_period_defaults_resolve`; `ledger-mcp`
`server.rs` `period_tools_describe_the_one_default`; `ledger-cli`
`tests/mcp_journeys.rs` `one_default_period_everywhere_defect_16` and
`the_default_period_never_shortens_the_iif_or_the_package`,
`tests/oracle_mcp.rs`
`show_sends_the_dates_it_names_defect_16`,
`show_refuses_start_after_end_defect_15`,
`show_refuses_a_derived_inverted_range_defect_15`; `ledger-gui`
`tests/show.rs`; bookspec `examples/default-period-fiscal-year-to-date.toml`.

## B-31 Renaming an account changes its label, nothing else

**bookspec:** `BS-ledger-10`

**Status:** CHOICE (the owner, `plan/served.md` 4.9 and P3 "Settled in
design", 2026-10-20: "a new account named like a sibling (ignoring
capitals), or holding a nine-digit number, is refused"); B-8 (posted is
forever); AUTHORITY for keeping the old name (Rev. Proc. 98-25 §5.01,
`sources/irs-records.md`: "the relationship (i.e., audit trail): (a)
between the total of the amounts in the taxpayer's machine-sensible
records by account and the account totals in the taxpayer's books");
Intuit IIF (`sources/intuit-iif.md`: "Accounts are identified by NAME")
for refusing a sibling's name.

Entries name accounts by id, so a rename changes no entry, line, hash,
balance or statement amount: every report, past periods included, prints
the new name. The journal keeps the old one (`account.renamed {account_id,
code, name, previous, reason}`), so a report printed under the old name
can be tied to the new. A rename is refused when the account is unknown,
the name is blank, the name (trimmed) is the one it has, another account
under the same parent (top-level accounts are siblings too) has it
ignoring capitals and surrounding space (QuickBooks would merge the two),
it holds a run of nine digits (a full SSN, EIN or account number, B-26:
every report prints a name), or it is not one line of at most 200
characters (`account::MAX_ACCOUNT_NAME`, the bound of the company's own
name: no line break, tab or other control character, which no report, CSV
or IIF line can hold). The name is kept trimmed. The same name rules hold
for a new account (`books.add_account`, `books.import_chart_csv`), which
is journaled with its name trimmed too; accounts the engine
creates for a control role keep their fixed names and are never refused
for one. Approval rules, roles, settings and closed periods name accounts
by code or id, and a name is no posting, so none of them blocks a rename.
Existing charts are untouched: the rules are the commands', never replay's
(B-28).

A rename can still change what the books pick by name (no amount): with
`books.cash_accounts` unset, the cash accounts are the asset accounts
whose names say they hold cash, and, while the books record no
business-card payment method, the card accounts are the liabilities
named "card" but not "gift" (`ledger_core::parties::card_accounts`, B-35
item 5); the result says
when the new name moves the account into or out of either. It also lists
conventions whose text names the old name. QuickBooks matches accounts
by name, so a CPA who imported an earlier IIF is told.

**Enforced:** `Ledger::rename_account` / `check_rename_account`,
`check_account_name` (also in `add_account`), `account::MAX_ACCOUNT_NAME`,
`check_rename_reason`,
`replay_rename_account` (only that the account exists),
`ChartOfAccounts::set_name`, `Ledger::add_control_account`;
`Store::rename_account` (journal feature `account_renamed`, which the
books declare before their first rename, B-27); `books.rename_account` (scope
`books:post`, like renumbering). **Checked:** `ledger-core`
`tests/spec_ledger.rs` (`rename_account_*`), `tests/model.rs`
(`Cmd::Rename`); `ledger-store` `tests/store.rs`
(`rename_account_changes_only_the_name_defect_15`,
`prop_rename_changes_no_figure`, `a_journaled_rename_replays_past_the_command_rules`),
`tests/format.rs`; `ledger-cli` `tests/mcp_journeys.rs`
(`rename_account_end_to_end_defect_15`), `tests/hostile_args.rs`;
`ledger-core` `tests/spec_ledger.rs`
(`a_new_name_is_one_trimmed_line_of_bounded_length`), `ledger-store`
`tests/store.rs` (`a_new_accounts_name_is_journaled_trimmed`); Lean
`Step.record` (a name is outside the model: `docs/verification.md` law 15);
bookspec `examples/rename-changes-no-amount.toml`.

## B-32 Every revenue and expense account maps to a line of the return

**bookspec:** `BS-ledger-11`

**Status:** AUTHORITY for why it matters (IRS Pub. 583,
`sources/irs-records.md`: "Your books must show your gross income, as
well as your deductions and credits"; IRS Pub. 538: "you must maintain
records that will enable you to file a correct return"; Rev. Proc. 98-25
§5.01: the relationship "between the total of the amounts in the
taxpayer's machine-sensible records by account and the taxpayer's
return"); CHOICE for warn versus refuse (Pub. 583: "the law does not
require any specific kind of records"; the owner, 2026-10-20: warn by
default, header accounts exempt, the first company set to refuse at the
P3 cutover).

A tax category is the account's link to the return (`ledger_core::tax`,
`sources/irs-tax-lines.md`). An account needs one when it is a revenue or
expense account and not a header (a header takes no postings, so nothing
of its own reaches the return: no entry posts to one, a replayed one
included, and an account's header flag never changes).

- Needs attention and the digest (`uncategorized_accounts`) list every
  account that needs a category and has none, with activity or not.
- `policy.require_tax_category`: `warn` (unset) or `refuse`. Under `warn`,
  `books.add_account` and `books.import_chart_csv` create the account and
  their results say so. Under `refuse`, they refuse it (the file whole),
  naming the account and where the categories are listed; both take the
  category at creation, so the refusal is always met in one call; and
  `books.set_tax_category` won't clear a category an account needs.
- The accounts invoices, bills and payroll create for a control role
  (Revenue 4000, General Expenses 6999, Salary & Wage Expense 6100,
  Payroll Tax Expense 6110) are never refused, so an invoice is never
  refused over a chart label: their results warn, and Needs attention
  lists them. The policy is "no uncategorized account made through the
  chart tools", not "never any".
- Turning the policy to `refuse` changes no existing account: it applies
  to new commands (B-28). Needs attention keeps listing the gaps.
- Every account a tool can require has a category that fits it on every
  return: the gain or loss account of a disposal (`form_4797_gain_loss`,
  Form 4797), tax-exempt income (`tax_exempt_income`, Schedule K), and
  each control role's. A gain or loss on business property is figured on
  Form 4797 from the tax basis, and its split (ordinary gain, net section
  1231 gain) decides the line, never Schedule C: the tax summary lists it
  apart (`TaxSummary::form_4797`), at its book amount, with no tax amount
  and in no total, on every return.

**Enforced:** `tax::needs_category`, `account_needs_category`,
`uncategorized_accounts`, `CATEGORIES`; `Ledger::check_tax_category_required`,
`check_tax_category_kept` (commands only, never `set_account_tax`, which
replay calls); `settings::TaxCategoryPolicy`, `Settings::require_tax_category`;
`Store::add_account_with_tax`, `add_account`, `set_account_tax`
(journal feature `require_tax_category`, which the books declare before
their first `settings.set` naming the key, B-27); `ledger-mcp` `books.add_account`,
`books.import_chart_csv`, `books.digest`; `ledger-gui` Needs attention
(Info under warn, Todo under refuse). **Checked:** `ledger-core`
`tests/tax.rs`; `ledger-store` `tests/store.rs`
(`prop_tax_policy_refuses_exactly`, `a_journaled_clear_replays_under_refuse`);
`ledger-cli` `tests/spec_tax_policy.rs`; `ledger-gui` `tests/view_model.rs`
(`needs_attention_flags_every_uncategorized_pl_account_defect_8`);
bookspec `examples/tax-category-required.toml`.

## B-33 A package as of a day shows the books through that day

**bookspec:** `BS-statement-4`

**Status:** CONVENTION (a report as of a day reports the books on that
day: balances through it, a period's activity from the period's start
through it; nearest authorities the accounting period, IRS Pub. 538,
`sources/irs-periods-and-methods.md`, and the information returns'
calendar year, `sources/irs-1099-thresholds.md`); CHOICE (owner,
2026-10-20, plan D-36: the 1099 worksheet stops at as_of too)

Nothing in the CPA package made as of a day (`books.export_cpa_package`,
the viewer's Export for CPA) reflects anything dated after it: no figure,
entry, receipt or worksheet row. The statements and the ledgers cover
the fiscal year containing as_of, through it; the balance sheet, the
trial balance and the fixed asset register are as of it (the register
only when an asset was bought by then); receipts/ holds the evidence of
the year's entries through it; the members' equity statement and the
K-1 worksheet, the year to date; general-journal.iif, every posted entry
from the first through it (B-34). The 1099 worksheet counts the payments
of as_of's calendar year from January 1 through as_of: information
returns add up what is paid "in any calendar year" (26 U.S.C. §6041(a);
"during such calendar year", §6041A(a)(2)), so a worksheet as of June 30
is the year to date, and a vendor not yet at the threshold is not yet
reportable; a December 31 package counts the whole year, as before. The
chart of accounts and documents/ are as they stand when the package is
made. One list of cash accounts and one of financing accounts serve the
whole package (the request's, else the settings':
`ledger_package::Settings::with_accounts`): the cash flow, the K-1
worksheet, the IIF's bank accounts and the 1099 payments read the same.
So the package's 1099 worksheet is `books.vendor_1099`'s CSV for the
same `cash_accounts`, `financing_accounts` and `through` = as_of, byte
for byte; the tool, given no accounts, uses the settings', as a package
given none does.

**Enforced:** `ledger_package::build_package` (INVARIANTS 1, 6, 7),
`ledger_package::Settings::with_accounts`, `ledger_package::export_iif`,
`ledger_package::vendor_1099_worksheet` and `ledger_core::vendor_1099_report`
(`through`), `books.vendor_1099` (`cash_accounts`, `financing_accounts`,
`through`).
**Checked:** `ledger-package` `tests/package.rs`
(`every_file_in_an_as_of_package_stops_at_as_of`: books the same through
as_of, with and without activity after it (a 1099 payment with its
receipt, an asset bought, a reversal of an entry dated as_of, the
members' contributions, an earlier asset sold), make the same package
byte for byte, every file and which files there are, with and without a
fixed asset register, mid-year and at year end; `as_of_package_iif_stops_at_as_of`,
`the_iif_through_as_of_at_its_edges`,
`the_1099_worksheet_stops_at_as_of`,
`one_list_of_cash_accounts_per_package`,
`payments_to_a_vendor_come_from_the_packages_accounts`; the property
`iif_ties_to_the_trial_balance_as_of`, 2,000 cases); `ledger-cli`
`oracle_mcp.rs` (`cpa_package_holds_the_year_and_its_evidence`: the
package's CSV is the tool's for the same accounts and `through`); the
bookspec examples `bookspec/examples/as-of-package-stops-at-as-of.toml`
and `bookspec/examples/1099-worksheet-through-as-of.toml` (`ledger-core`
`tests/bookspec.rs`).

## B-34 The QuickBooks Desktop file (IIF): plain ASCII, every entry through as_of, closes left out, classes carried

**bookspec:** `BS-format-2`

**Status:** AUTHORITY (Intuit, `sources/intuit-iif.md`: IIF files are
ASCII text, tab-separated, with no escape; an account is known by its
NAME, a subaccount's NAME its path joined by colons; lists come before
what uses them; DOCNUM is "the transaction's number"; the kit's 2007+
example file ends every line CR LF; QuickBooks Desktop holds an account
name of 31 characters, an account number of 7 and a memo of 4,095; "The
parent and subaccount must be the same account type"; a journal entry
to accounts receivable or payable needs a customer or vendor, and takes
one such account a transaction; a class list's keyword is CLASS, "Only
the NAME column is required", a subclass's NAME its parents' first,
joined by colons, and the kit declares its classes before the
transactions that name them, line by line in CLASS; Intuit's qbXML
Onscreen Reference, `ClassAdd`: a class's Name at most 31 characters in
QuickBooks Desktop, as an account's); CONVENTION (a
QuickBooks integrator and an IIF importer, same file: a longer name is
cut on a chart import but not in a journal's lines; a double quote is a
text delimiter; Conductor's QuickBooks Desktop API: a class's name
case-insensitive, so no two alike ignoring case); CHOICE (owner, 2026-10-20, plan D-36: plain ASCII, what isn't
ASCII transliterated and listed; CR LF; DOCNUM the entry's id; closing
entries left out; every entry since the company began until the CPA
says otherwise); Unicode Standard Annex #14 (the mandatory line breaks)

1. **One transaction per posted entry dated on or before as_of**, from
   the first, in the order posted, **except period-closing entries**:
   QuickBooks keeps no closing entries (it works out retained earnings
   itself; its "closing date" only locks a period), so an imported close
   would zero its period's profit. The file's balances are the trial
   balance as of as_of without the closes, and its income and expenses
   each period's profit and loss; the members' shares are in the
   package's members' equity statement and K-1 worksheet.
   `books.export` iif takes as_of (default today), never a range.
   Whether the file should hold only the fiscal year instead goes to the
   CPA before the 2027 package (D-36); for the first year the two are
   the same.
2. **DOCNUM is the entry's id** in the books, on the TRNS and every SPL,
   so the CPA can trace a QuickBooks transaction to its entry and its
   receipts.
3. **The file is ASCII and every line ends CR LF.** In a field (a NAME,
   an ACCNUM, a MEMO) a tab or a mandatory line break (LF, VT, FF, CR,
   NEL, U+2028, U+2029) is a space; any other control character is '?';
   printable ASCII is itself, but a double quote, which is a single one
   (QuickBooks reads a double quote as a text delimiter); any other
   character is its transliteration by deunicode 1.6.0, pinned exactly
   ("René" "Rene", "北京" "Bei Jing", "—" "--", "“" "'", "😀" "grinning";
   a transliterated word's trailing space dropped at the end or before a
   space), or '?' when it has none. Every other file in the package keeps
   the books' text (the PDFs carry Inter and Noto fallbacks for Chinese,
   Japanese, Korean, Arabic, Hebrew, Thai and Devanagari, Arabic and
   Hebrew set right to left; no emoji font, so an emoji prints as an
   empty box).
4. **An account's own name**, so written, has each ':' replaced by '-'
   (a fullwidth colon too), is trimmed ('?' if nothing is left) and cut
   to 31 characters (QuickBooks Desktop's limit; cut by QuickBooks, the
   ACCNT line and the transactions' would no longer match); its NAME is
   its parents' NAMEs and its own, joined by ':'. A subaccount whose
   QuickBooks type differs from its parent's (a bank account under a
   header that is no bank, an asset's cost under a "Fixed assets" header)
   is written at the top level, its own subaccounts with it: QuickBooks
   gives a subaccount its parent's type, and the type decides where its
   balance shows. Each parent is declared before its subaccounts, the
   accounts otherwise in the order of their codes. Accounts whose NAMEs
   would match ignoring case, which QuickBooks would merge, get
   " (code)" after their own names (the name cut to make room, the code
   written like a name, "no number" for a blank one), level by level from
   the top and again until no two match; a match left after that (two
   codes written alike, ignoring case, or alike in their first 30
   characters) is refused.
5. **ACCNUM is the account's code**, written as a field and trimmed, or
   left blank (ACCNUM is optional; the NAME identifies the account) when
   it is longer than 7 characters (QuickBooks Desktop's limit) or another
   account's would be written the same, ignoring case. **A MEMO** is cut
   to 4,095 characters (QuickBooks Desktop's limit).
6. **Nothing changes silently.** Each account name, number or place and
   each memo the file writes differently from the books (`Iif::changes`)
   is said, both texts quoted and escaped, one line each, in the
   `warnings` of `books.export` and `books.export_cpa_package` and in the
   package README (the viewer's note counts them): each account's name,
   number and place a line; the memos grouped by what is written
   differently, each run of changed characters once with every entry it
   is in (`"—" is written "--" in the memos of entries 3, 5, 9`), then
   the memos cut. No line holds a full TIN (B-26): a run of nine digits,
   in any script's digits and separators, is masked, and so is every
   numeral of a text that holds one once written as a field (digits a
   tab apart, letters deunicode writes as digits); a memo line whose
   digits are masked says so.
7. **No customer or vendor NAME.** Intuit's IIF help makes NAME
   optional and states no need of it in a journal; QuickBooks Desktop
   asks for a customer or vendor on a journal entry to accounts
   receivable or payable, and takes one A/R or A/P account a transaction
   (Intuit staff and help, `sources/intuit-iif.md`). Writing names would
   need the customer and vendor lists declared as well, with names unique
   across them. So the file names none, and the package README names the
   entries with a line on an A/R or A/P account (`Iif::receivable_payable`),
   so the CPA can add the name if QuickBooks asks.
8. **Each line's class (project, B-37) is its CLASS**, and every class in
   the books is declared in a `!CLASS` list (NAME) after the accounts and
   before the transactions, as the kit declares its lists; books with no
   class write no list. A class's NAME is written as an account's own
   name is (rules 3 and 4: plain ASCII, a ':' a '-' since a colon starts
   a subclass, trimmed, '?' if nothing is left, cut to 31 characters);
   classes whose NAMEs would match ignoring case get " (id)" after their
   names (the name cut to make room), round after round until none does,
   as accounts get " (code)", except that among names that match, those
   already written differently from the books (made safe, or tagged)
   yield first, another " (id)" each time need be (CONVENTION: the CPA sees
   the books' own names wherever they can stand): a class's name
   written exactly as the books have it is kept unless another class's
   own name is the same ignoring case (classes 1 "Café", 2 "Cafe" and 3
   "Cafe (1)": "Cafe (1) (1)", the other two kept; review of the P4 follow-ups, finding 4,
   2026-10-22: before, all three were renamed; and a fourth "Cafe (1) (1)"
   makes class 1 "Cafe (1) (1) (1)", where a cap of two tags once renamed
   class 4 instead). A tag that would change nothing (a name cut to 31
   characters, ending in its own " (id)") isn't made: the name it meets
   is tagged instead. Each class name written differently is
   listed with rule 6's (`class 2's name "Café rebrand" is written "Cafe
   rebrand"`), and the package README's "written differently" paragraph
   says class names are held to 31 characters, no two alike. A line
   whose class isn't declared is refused, never written classless. Before
   2026-10-22 the file left CLASS empty on every line and the README
   didn't say so (the P3 cutover review's open point 1).

**Enforced:** `ledger_import::iif` (`field`, `iif_accounts`,
`iif_classes`, `journal_to_iif`, `Iif::warnings`), `ledger_package::export_iif` and
`build_package`, `ledger-mcp` `books.export` and
`books.export_cpa_package`.
**Checked:** `ledger-import` `iif.rs` unit tests (every one of the
1,112,064 Unicode scalar values written as printable ASCII, alone and
beside a letter, and kept by a second pass; the transliteration table
pinned); `tests/spec_import.rs` (`iif_docnum_is_the_entry_id`,
`iif_writes_plain_ascii_and_lists_every_change`,
`iif_every_line_ends_cr_lf`, `iif_distinct_accounts_never_share_a_name`,
`iif_a_subaccount_of_another_type_stands_at_the_top_level`,
`iif_declares_each_parent_before_its_subaccounts`,
`iif_fits_quickbooks_desktop_limits`,
`iif_warnings_group_the_memos_by_what_is_written_differently`,
`iif_warnings_never_hold_a_tin`,
`iif_change_warnings_are_one_line_each`,
`iif_keeps_a_tin_readable_to_the_package_scan`,
`iif_writes_each_lines_class_and_declares_the_classes_first`,
`iif_class_names_fit_quickbooks_and_every_change_is_listed`,
`iif_class_names_written_as_the_books_have_them_are_kept`,
`iif_refuses_an_undeclared_or_unfit_class`, the properties
`iif_round_trips_random_books`,
`iif_names_never_clash_and_every_change_is_listed` and
`iif_class_names_never_clash`); `ledger-package`
`tests/package.rs` (`iif_leaves_out_period_closes`,
`the_readme_lists_what_the_iif_writes_differently`,
`the_readme_names_the_entries_on_receivables_and_payables`,
`the_package_iif_carries_each_lines_project`,
`the_bookspec_iif_example_matches_the_engine`, and B-33's); the bookspec
example `bookspec/examples/iif-quickbooks-journal-file.toml` (formats
appendix: the file line by line and its changes, derived by hand);
`ledger-cli` `oracle_mcp.rs`
(`iif_declares_accounts_by_role_and_balances_every_transaction`,
`iif_without_as_of_is_as_of_today_and_ties_to_the_trial_balance`),
`mcp_journeys.rs` (`mcp_import_export`,
`the_default_period_never_shortens_the_iif_or_the_package`).

## B-35 A party's part of an entry, and a party's totals

**bookspec:** `BS-partner-3`

**Status:** CONVENTION (the sign: B-2's, a line on the party's own
account); CHOICE (the owner, 2026-10-15: `entry_flow`, money paid down
counted once; the owner, 2026-10-20, P3 design settled with each review's
recommendation: a member's entry counts what moved their capital, money
is cash, cards, the members' financing and loans, and a liability credit
never counts as paying that liability's own creditor); AUTHORITY for what
a member's capital counts (Treas. Reg. §1.704-1(b)(2)(iv)(b), (c);
Form 1065 Schedule K-1 instructions, Item L:
`sources/irs-capital-accounts.md`) and for a promise not being payment
(Helvering v. Price, 309 U.S. 409 (1940):
`sources/helvering-v-price-1940.md`). Not ASC 230's cash (see item 8) and
not the books' accounting method (`books.accounting_basis`).

1. **Sign.** A party's part of an entry is signed as a line on the
   party's own account would be (debit-positive, B-2): positive, the
   company paid it or gave it value; negative, the company received from
   it. A bill is negative (owed to the vendor), an invoice positive.
2. **Direction, cash, in kind.** Every part carries its direction (paid,
   received, neither) and what of it moved as money (cash) and as property
   (in kind); what is neither is a promise still to be settled (a bill, an
   invoice, a fee on the party's own card, a distribution still owed) or
   moves nothing with anyone (a close), and counts in no total. A part's
   cash and in kind have its sign and together never exceed it; what of a
   paid part is neither (a distribution partly still owed) is a promise,
   counted when it is paid.
3. **What the shares split** (`shares::target`), in order:
   1. A period's closing entry: its parties' own-account movement (the
      members' equity, when only members are named), else its total
      debits; neither (B-11, B-13: a close allocates income, it moves
      nothing with anyone).
   2. A reversal: its original's target, negated, paid and received
      swapped, from the root original down (B-9: "a mistake is corrected
      by an opposite entry"); never re-read from its own lines, because
      `entry_flow` isn't odd when one liability pays down another (a
      member paying the card from their own funds, reversed, read as a
      second payment: P3 design §1.4).
   3. Every party named has the member role in the directory and the
      entry moves equity other than retained earnings (E): E, a
      contribution received (E < 0) or a distribution paid (E > 0).
      - Of a contribution, in kind is the property in it, at most |E|:
        what asset accounts that are neither cash nor receivables took in
        (the K-1 worksheet's test, k1 INVARIANT 3), so property
        contributed subject to a loan is the member's net property
        (§1.704-1(b)(2)(iv)(b)(2)). The rest is cash, as the K-1 counts
        it: money, or what the member paid for the company (an expense
        booked straight to capital, a debt to them converted,
        §1.704-1(b)(2)(iv)(c)(1)).
      - Of a distribution, cash is the money it paid (case 4's net, at
        most E: §1.704-1(b)(2)(iv)(b)(4)) and in kind the property it
        gave up, at most what is left, as the books carry it (the
        statement's figure; (b)(5) counts its fair market value, which
        stays with the CPA). The rest, credited to a liability that
        isn't money for the entry (distributions payable, or what the
        members advanced once they are named its creditors), is still
        owed: it counts as nothing until it is paid (Helvering v. Price;
        before the P3 review a distribution declared to distributions
        payable counted when declared and again when paid, and a
        distribution of money with depreciated property read the money
        as property).
      Read from journaled facts only (roles, the chart, creditors), so a
      member leaving `[[company.members]]` never changes a posted entry;
      the settings' members matter only to a derived split among several,
      which the server journals as amounts when the entry posts.
   4. Money moved (`parties::entry_flow` net, item 5's accounts less the
      liabilities owed to a party named): the net, paid (+) or received
      (−), all of it cash.
   5. Otherwise its parties' own-account movement (payables, receivables,
      a liability owed to a party named), else the total debits; neither.
   The parts split the target as `shares` INVARIANTS 7-12 say: a
   reversal's parts are exactly its original's negated, and a member's in
   kind is shared as the K-1 worksheet shares property (in proportion,
   leftover cents by largest remainder, ties to the member listed first).
4. **Totals: `basis` cash (the default) or all, always labelled.** Cash
   counts each part's cash; all adds what moved in kind (Item L counts
   money and property alike). Each part counts on its root original's
   side, so a reversal nets what it reverses, and reversing the reversal
   brings it back. A bill or invoice counts on neither basis (a promise is
   not payment: Helvering v. Price); `billed`, `invoiced`, `owed` and
   `receivable` report those. An entry that counts nothing is never
   unresolved (its target is found before its split). For a member, over
   the entries that name only members and move their capital, plus the
   payments of distributions declared to them: basis all's received and
   paid are the statement of members' equity's contributions and
   distributions for the same dates, and basis cash's received is the
   K-1 worksheet's cash contributions, when
   - each such entry names exactly the members whose capital it moves
     (derived from their capital lines, or amounts equal to them) and
     moves every one of them the same way (capital moved from one member
     to another moves nothing with the company: neither, though the
     statement shows a distribution and a contribution);
   - no distribution declared in the span is still unpaid at its end (the
     statement counts it when declared, party totals when paid); and
   - no reversal in the span undoes an entry outside it (the statement
     then counts the reversal by its side; party totals net it on the
     original's side).

   Over any span at all, even one a reversal straddles, received less
   paid still agrees with contributions less distributions, given the
   first two conditions (the property
   `member_totals_are_the_members_equity_statement` checks every span).
5. **Money** (P3 defect 24): the cash accounts (`[books] cash_accounts`;
   every asset account but receivables when none are set), the cards
   (`parties::card_accounts`, the one rule the reconciliation reminders
   and the viewer read too: the liability accounts of the business-card
   payment methods the books record, B-19; while they record none, the
   liability accounts whose names say "card" but not "gift card"; never
   the account of a member's card, a member's cash or check, or a
   payment platform, which pay through the members' financing account or
   their own balance), and `[books] financing_accounts` (what the members
   advanced, loans). Every other liability (payables, the payroll withholdings, an
   accrued expense, deferred revenue, gift cards the company sold) is a
   promise: counted when it is settled, never also when it is booked
   (before P3 an accrual through any liability but payables and payroll
   counted twice). So, before any business card is recorded, a card
   account whose name doesn't say "card" is not money until it is listed
   in `financing_accounts` or a business-card method names it, and with
   `financing_accounts` unset what the members
   advanced is not money either: a purchase through either then counts
   for no one until it is paid off (the handbook tells the agent to list
   them).
6. **A party's own liabilities.** A party may be named the creditor of
   the liabilities the company owes it itself (`creditor_of`: a card's
   issuer, a loan's lender, each member for what the members advanced;
   `party.creditor_set`, a journal feature). For an entry naming it, a
   credit to one never counts as paying it (a fee on its card, interest on
   its loan, equipment on the seller's note: neither, until paid; a
   distribution credited to the members' financing: still owed), and a
   debit to one is what the money paid for (a card payment, a
   reimbursement: paid, as before).
7. **Known limits**, stated in the tools' text: an entry that both repays
   a member and distributes to them counts the distribution only; money a
   member lends under their own name through the members' financing and
   later converts to capital counts both times (received as a loan, then
   as a contribution), as does a distribution declared through it while
   the members aren't named its creditors (paid when declared, and again
   when paid); an entry naming a payee together with the creditor of the
   liability that paid it (a vendor and the member who paid it through
   the members' financing) reads neither for both; an expense a member
   paid that is booked straight to capital and tagged with the vendor
   reads neither (the vendor isn't counted as paid; tagged with the
   member, it is the member's cash contribution). Record them apart, and
   a member's payment for the company through the members' financing,
   tagged with the payee: the house conventions already do.
8. **How party totals relate to the cash flow statement** (B-14, D-42,
   P3 defect 1). The statement reports the company's cash, ASC 230's: a
   member paying a vendor for the company is cash borrowed from the member
   and spent, on that date (a financing inflow and an operating or
   investing outflow); paying the member back is a financing outflow;
   property a member contributes, and a debt to them converted to
   capital, are noncash (disclosed); a distribution declared to a
   distributions payable is a distribution on the statement when it is
   paid (B-14 rule 2), listed under the payable ("Distributions to
   members — 2200 Distributions Payable"), not by member: the statement
   reads accounts, not parties, so the member a payment from the payable
   went to is party totals' (the payment's own parties) and the
   declaration's (the statement of members' equity, at its date). So over
   a span, the statement's distributions (by member and under the payable)
   are the statement of members' equity's less what is declared and still
   unpaid at the span's end, and party totals' paid to the members on the
   entries that move their capital or pay a declared distribution, exactly
   (`the_members_figures_agree_on_every_date`). Party totals report what moved with
   each party: the vendor is paid when the member pays it (the same date
   and amount as the statement's outflow); the member is paid back when
   the statement shows the financing outflow; and a member's capital
   counts when it moves, a conversion as the cash contribution the K-1
   calls it (the money they paid for the company), property in kind; a
   distribution counts when it is paid, as on the statement. Where both
   show a payment they agree on its amount and date. The statement shows
   more than party totals do in two places: the financing inflow from a
   member who pays a vendor is in no party's totals (the entry names the
   vendor; the member's totals show only the repayment), and an expense
   booked straight to capital and tagged with the vendor isn't counted
   as paid to the vendor (item 7). "Cash" in party totals is money
   through the company's cash accounts, cards and financing (and, on a
   member's contribution, the K-1's cash), not the statement's ASC 230
   cash.
9. **How party totals relate to the 1099 worksheet** (B-19; P3 defects
   12, 18, 23; D-40). They answer different questions, and neither is
   read for the other (`books.party` gives both, labelled). Party totals
   report what moved with each party, whoever handled the money; the
   1099 worksheet reports what the company, as the payor, paid a payee
   toward its information return. A member paying a contractor for the
   company through the members' financing account, the entry tagged
   with the contractor: in party totals it is paid to the contractor on
   that date, in money (the members' financing is money, item 5), and
   nothing moves with the member until the company pays them back, which
   is paid to the member. On the 1099 worksheet it is the company's
   payment to the contractor (Treas. Reg. §1.6041-1(e)(1): the member only
   writes the check at the company's direction): it counts toward the
   contractor's 1099 when it was the member's own cash, check or
   transfer, is excluded when it was a card, the member's own included
   (§1.6041-1(a)(1)(iv): Form 1099-K), and is undetermined until its
   payment method is recorded; the reimbursement pays no payee (money
   between the company's own sources). A company card's charge is paid
   in party totals and excluded on the 1099 alike. So the figures never
   contradict: over a calendar year, for a payee every entry of which
   names it alone, in books whose cash accounts are named, whose company
   cards are recorded as business-card payment methods, and that pay only
   out of cash accounts, cards and financing accounts, its party totals'
   paid less received (basis cash) is exactly its 1099 worksheet's
   `paid` + `excluded_card` + `undetermined` + `not_reportable`, less what
   the worksheet lists as money received from it for something else
   (a refund is netted on the worksheet and received in party totals).
   The bookspec example `1099-and-party-totals-for-a-members-payment`
   checks a year of it, and the property
   `party_totals_are_the_1099s_buckets` checks it over random books. A
   bill counts in neither until it is paid, and a payee's dated decision
   changes only the 1099 worksheet.

**Enforced:** `kernel::party_target`, `kernel::reverse_target`,
`shares::{target, split, resolve}`, `parties::{flow, money_accounts,
card_accounts, MoneyAccounts::for_parties, activity_with, Activity::totals,
Basis}`, `Ledger::set_party_creditor`, `Store::set_party_creditor` (the
`party_creditors` journal feature, asked for before its first line);
`ledger-mcp` `books.get_entry` (`resolved`, `direction`, `cash`,
`in_kind`), `books.party` and `books.parties` (`basis`), `books.update_party`
(`creditor_of`); the viewer's party pages and entry page.
**Checked:** `ledger-core` `tests/spec_party_totals.rs` (the audit's
table, the members' equity and K-1 identities over every span, the
reversed card payment, accruals, a gift card, a party's own liabilities,
distributions declared or of depreciated property, the properties), Kani
`party_target_direction_is_its_sign`, `party_target_is_odd`,
`reverse_target_is_an_involution`, bridge `party_target_agrees`; Lean
`reverseTarget_involutive`, `capitalTarget_odd`, `partyTarget_odd`,
`sumOver_movement_reversal`; `ledger-store` `tests/party_creditors.rs`;
`ledger-cli` `tests/spec_party_totals.rs`; `ledger-gui` `tests/parties.rs`.

## B-36 The K-1 worksheet reports tax-basis capital beside book capital

**bookspec:** `BS-partner-4`

**Status:** AUTHORITY (`sources/irs-partnerships.md`: Instructions for
Form 1065 (2025), Item L, "Tax-basis method"; required from tax year
2020: Instructions for Form 1065 (2020), Item L, "you must use the tax
basis method this year"); CHOICE D-41 (the owner, 2026-10-20: "the
K-1 worksheet reports tax-basis capital ... the contributed property at
its §723 carryover basis, beside the book figures")

Schedule K-1 Item L reports each member's capital account on the tax
basis, by the transactional approach: beginning capital (last year's
ending), "the amount of cash plus the adjusted tax basis of all property
contributed", the member's share of income "as figured for tax
purposes", other increases and decreases "for tax purposes", and "the
amount of cash plus the adjusted tax basis of all property distributed".
The K-1 worksheet shows, per member, each Item L line on the book basis
(the members' equity roll-forward, B-13) beside the tax basis, and how
book capital reconciles to tax capital:

- Property contributed enters at its §723 basis (B-25): the member's
  share of the property's book value less their share of its built-in
  gain or loss at contribution, shared within the entry as its book value
  is (B-13). To the cent, these shares (and those of property without a
  basis, of a prepaid at book, and of property distributed) run
  cumulatively, D-38's rule (the owner, 2026-10-22): each member's
  running share is the largest-remainder rounding of their exact running
  share (ties to the member listed first), less what earlier entries
  gave them, so it is never a cent from exact, and tied odd cents
  alternate rather than all going to one member (three equal odd-cent
  properties at 50/50 leave the members a cent apart, never three). How
  the count runs differs from the closes': a close splits by the
  members' shares among all the members, so its count starts afresh when
  members or shares change; here each entry splits by its own amounts
  (each member's property in it, or what each took) among the members
  with a part in it. So there is one count for each set of members with
  a part in an entry, and it never restarts: shares don't enter it, and
  a member outside the set is never moved by it (Item L's capital
  contributed is the basis of the property that member contributed, so
  a member who contributed nothing in an entry takes none of its
  cents). The count takes every posted entry from the books' first in
  date order (ties in posting order), whatever span the worksheet is
  for, and an entry's reversal takes the original's split back out of
  the count, each member exactly their part: an entry's cents are the
  same in every span it is in, and next year's beginning is this year's
  ending to the cent. The book property each member contributed in an
  entry (B-13's share of the entry's property among the members'
  credits) stays split entry by entry, ties to the member listed first:
  each member's property there must stay within their own credit in the
  entry, which a running true-up could break; its odd cents arise only
  in an entry part cash and part property whose credits don't divide
  the property evenly.
  Property contributed with no basis in the books (no
  registered asset linked to the entry holds a contribution basis for it)
  is open. Property contributed to a prepaid account (the balance
  account of a prepaid-expense schedule: a service contract the member
  handed over, B-13) with no fixed asset for it enters at its book
  amount, with a note: a contract's carryover basis is what the books
  recorded for it (CHOICE, P3 review 2026-10-21; the CPA to confirm).
- What the books can't know is never guessed. The share of net income
  "as figured for tax purposes" is the return's, always: the books are
  kept on their own basis, and every book-tax difference (Schedule M-1)
  is the CPA's; so is the ending, and the beginning of any year after
  the first (Item L: "the partner's ending capital account as
  determined for last year"). The line names the differences the books
  do see: the tax depreciation of every fixed asset in service (MACRS,
  and for contributed property the §704(c) method's allocation: the
  CPA's, B-15), and the tax gain on a disposal, with a contributed
  property's built-in gain or loss at contribution, which that gain must
  take account of (IRC §704(c)(1)(A)). Also open: the adjusted tax basis
  of property distributed, and the tax treatment of a change in book
  capital the books can't explain. Each such line is left blank and
  marked open, with what the books know so far and the book amount the
  CPA's figure replaces.
  The book depreciation the tax figure replaces is the period's
  depreciation expense, the P&L's: each asset's runs and what its
  disposal entry posts to its expense account (B-15 rule 8).
- The reconciliation: book ending less tax ending (as far as the books
  know it) = beginning (book less tax) + the built-in gain or loss share
  + property without a basis (at book) − property distributed (at book),
  exactly.

**Enforced:** `ledger_core::tax_capital::tax_basis_capital`;
`ledger-package` `k1_tax` (`k1-tax-basis-capital.csv` in the CPA
package); the odd cents, `ledger_core::tax_capital::RunningSplit` on
`kernel::apportion_cumulative`. **Checked:** `ledger-core`
`tests/tax_capital.rs` (`tied_odd_cents_of_property_basis_run_cumulatively`,
`a_member_outside_an_entry_never_takes_its_cents`,
`an_entrys_odd_cent_is_the_same_in_every_span`,
`the_split_runs_in_date_order`, the law
`a_running_split_never_drifts_a_cent`), the unit tests in
`src/tax_capital.rs`, `bookspec/examples/partnership-k1-tax-basis-odd-cents.toml`;
`ledger-package` `tests/package.rs`
(`the_k1_worksheet_reports_tax_basis_capital`), `tests/p3_seams.rs`
(`depreciation_is_one_figure_everywhere`,
`the_cpa_package_as_of_each_date_is_the_tools_figures`).

## B-37 A project is a class with its facts; its figures are its tagged lines

**bookspec:** `BS-statement-5`

**Status:** CHOICE (the owner, 2026-10-21: "a Projects page that treats
a class as a project ... Projects are entities too and can have name,
description, whatever other metadata you think they should have"; the
convention `link-to-projects` in the books: "whenever applicable, we link
things to [projects]"); CHOICE (the owner, 2026-10-22, plan D-43: an invoice's or bill's
lines take a project, each line one or none, item 8); CONVENTION (job
costing, items 3 and 8; the snapshot, item 7: a presentation choice from
management accounting, not GAAP); AUTHORITY (on
spec, item 4: ASC 450-30-25-1; ASC 340-40-25-5 with B-22; directly
related costs, item 8: ASC 340-40-25-7;
`sources/fasb-asc.md`, both quoted through Deloitte's DART roadmaps and
not yet re-read in the ASC basic view: an open item)

1. **A project is a class.** Same id and name; every class is one (a
   class an earlier version wrote has the default facts: active, kind not
   recorded). Its facts, the few a job cost sheet carries: a
   description (its scope of work), a status (`active`, `paused`,
   `finished`, `abandoned`), a kind (`internal`: the company's own
   product; `client`: work for a customer; `on_spec`: work done hoping to
   be paid; `other`; or not recorded), a party (who it is for or with),
   the dates it started and ended (an end only when finished or
   abandoned, never before the start; informational: no figure filters by
   them), up to eight links (web addresses, never opened) and notes. Names
   are one line, unique ignoring capitals (in any script) and surrounding
   space, and never digits only (a project is named by id or name, so such
   a name would read as an id); no field holds a full TIN (B-26). A
   change that keeps a project's own name never checks the name again, so
   old books (replay never checks names) holding a duplicate can still
   change either project, and a fact-only change never renames. Journaled as `class.added`, then the whole
   record as `project.set` when there is more than a name (journal feature
   `projects`, B-27). Replay checks only what it can't do without (the
   class and the party exist), never the text rules, so a rule tightened
   later never breaks books already kept. No budget, estimate, contract
   value or percent complete: each is a document linked to the project, a
   revenue-recognition question the books don't face, or the CPA's.
2. **Links.** A line is tagged with a project (its class); a receipt or
   document is linked to a project (attachment subject `project`); a
   decision names the projects it is about. Whether to link is the
   company's convention; how is this rule.
3. **Figures (job costing).** For a period: the revenue and expense lines
   tagged to it, dated in the period, closing entries left out (B-11),
   each on its normal side; net is revenue less expenses. To date: every
   such line through the day, from the first day the books keep, whatever
   the project's dates. A tagged balance-sheet line counts nothing (an
   asset bought for it counts as it is depreciated or expensed, tagged
   then). The books allocate no cost: a shared cost counts toward
   projects only as allocation entries of their own, made by the
   company's allocation convention (item 8). The projects with the
   untagged lines
   partition the P&L. By month, the period's last 240 months at most.
4. **On spec.** Work done hoping to be paid: its costs are expenses when
   incurred (B-22; ASC 340-40-25-5 makes a cost of an anticipated
   contract an asset only if, among its criteria, it is expected to be
   recovered, which on spec it isn't). The hoped-for payment is a gain
   contingency: no receivable and no income until received (ASC
   450-30-25-1). A repayment is revenue tagged to the project (a
   reimbursement credited to the expense it repays, tagged, comes to the
   same). Its unrecovered cost, "if repaid", is expenses less revenue to
   date, never below zero: a figure, never an asset.
5. **Status.** Finished and abandoned projects still take lines (a late
   cost, a final invoice); `books.add_line`, `books.record_payout`,
   `books.create_invoice` and `books.create_bill` say so when one is
   tagged.
6. Omitted dates mean what B-30 says (`books.project`'s period); to date
   is always stated as such, beside it.
7. **Snapshot (a presentation choice, not GAAP).** A project's net is
   shown as its *contribution before overhead*: its revenue less its
   direct costs (every expense line tagged to it), with no company
   overhead allocated, since item 3 allocates none. The owner's label is
   "contribution"; what it means is revenue less the costs traceable to
   the project, fixed and variable alike (the books don't split costs by
   behavior), before the common costs no project causes. That is
   Garrison, Noreen and Brewer's *segment margin* (*Managerial
   Accounting*, segment reporting: a segment's sales less its variable
   expenses and its traceable fixed expenses, the defining source here),
   who warn against allocating common costs to judge a segment. It is
   not Horngren's *contribution margin* (revenue less variable costs
   only; Horngren, Datar and Rajan, *Cost Accounting: A Managerial
   Emphasis*), which the books can't compute and the viewer doesn't
   claim; "contribution" here is only the everyday sense, what the
   project contributes toward overhead. So a project in the black can
   still leave the company in the red. Its margin ("contribution
   margin" on the page, `margin_pct` in the tools) is that contribution
   as a percent of revenue, one place, half to even, and none without
   revenue (zero or less). "In the black" is a contribution above zero,
   "in the red" below, "break-even" lines tagged that net to zero (a
   cost and its reversal). The running net by month is the net to date
   through each month's end; its last is the net to date. No GAAP
   standard defines or requires these figures (ASC 280's segment
   reporting is for public entities' operating segments, not projects);
   they come from item 3's, one function the tools and the viewer share.
8. **Invoices and bills (the owner's decision, 2026-10-22, D-43).** Each
   line of an invoice or bill names one project or none. A document that
   spans N projects is N lines, each with its own amount; there is no
   percentage split, so every figure is exact, as entered, with nothing
   to round. A cost truly shared among projects is not split on the
   document: its line names none, and it reaches the projects, if at
   all, only as allocation entries of their own, on the stated basis of
   the company's monthly allocation convention when `books.conventions`
   has one; without one the line stays untagged and the agent asks the
   human. The reasoning: job costing (item 3) charges a job the costs
   traceable to it, its direct costs, whole, and reaches a shared
   (indirect) cost only by an allocation on a stated basis, never by
   guessing at the document. ASC 340-40-25-7 counts as relating
   directly to a contract (25-5(a)) both the costs traceable to it
   (direct labor and materials, costs explicitly chargeable to the
   customer, costs incurred only because of the contract, such as
   subcontractors: 25-7(a), (b), (d), (e)) and allocations of costs that
   relate directly to the contract or its activities (25-7(c)); 25-8
   sends general and administrative costs to expense unless the contract
   makes them chargeable to the customer (`sources/fasb-asc.md`). The
   books carry the first kind whole on the document's line and the
   second as entries under the allocation convention on its basis;
   neither is a percentage on the document, and 25-7(c) is what the
   allocation convention models. An invoice line naming an on-spec
   project (item 4) is taken, and the tools ask whether there is now an
   agreement: on spec, a hoped-for payment is no receivable until it is
   received. Its revenue (an
   invoice's) or cost (a bill's) then counts toward the project exactly
   as a tagged line of a hand-built entry does: the document's entry
   posts one revenue (expense) line per project its lines name, in the
   order each is first named, each the sum of that project's lines, and
   the untagged lines' sum as one line (`arap::document_parts`), so a
   document naming none posts exactly the one line every document posted
   before. The receivable (payable) line is never tagged: it is a
   balance-sheet line (item 3 counts it nothing), and so are the
   payments (`books.receive_payment`, `books.pay_bill`: cash against the
   receivable or payable), which take no project. Nothing else moves: the
   P&L, the 1099 worksheet (B-19: payments made) and a party's totals
   (B-35: what moved the money) read the same with or without the
   projects; only the projects' figures and the P&L by class differ. A
   line naming a finished or abandoned project is taken, and the tools
   say so (item 5); one naming a project the books don't have is refused,
   the document whole. Journaled as the line's `class_id` in
   `invoice.created` or `bill.created` and an `entry.line_class_set` right
   after its revenue or expense line's `entry.line_added`, behind the
   journal feature `document_line_projects` (B-27).

**Enforced:** `ledger_core::classes::{ProjectFacts, ProjectStatus,
ProjectKind}`, `Ledger::{check_set_project, set_project,
replay_set_project, projects_of_party}`, `reports::{project_figures,
project_to_date, project_entries, contribution_margin_pct}`,
`ProjectToDate::{net, margin_pct, any_tagged}`, `ProjectFigures::{months_net_to_date,
period_margin_pct, period_lines}`, `kernel::unrecovered_cost`;
`arap::document_parts`, `Ledger::{create_invoice, create_bill}` (item
8); `Store::{add_project, set_project, subject_feature}` (feature
`projects`), `Store::{create_invoice, create_bill}` (feature
`document_line_projects`); `books.add_project`, `books.update_project`,
`books.projects`, `books.project`, `books.create_invoice` and
`books.create_bill` (`class` on a line); the viewer's Projects pages and
an invoice's or bill's page (each line's project).
**Checked:** `ledger-core` `tests/spec_projects.rs` and
`tests/spec_document_projects.rs`, `ledger-store` `tests/projects.rs` and
`tests/document_projects.rs`, `ledger-cli` `tests/spec_projects.rs`,
`ledger-gui` `tests/projects.rs`, `ledger-cli` `tests/projects_parity.rs`
and `tests/format_upgrade.rs` (`books_p2_served_cut_over_whole`); bookspec
`projects-net-is-its-tagged-lines`,
`projects-on-spec-costs-are-expenses` and
`projects-invoice-and-bill-lines-count-by-project`; proptest
`prop_document_parts_partition_the_total`; Kani `unrecovered_cost_laws`;
proptests `prop_project_nets_partition_the_pnl`,
`prop_project_set_round_trips`, `prop_project_contribution_is_the_net_to_date`
(core) and `prop_snapshot_contribution_is_the_net_to_date` (viewer).

## B-38 A document's figures are its ledger lines that stand; a control account is its documents

**bookspec:** `BS-arap-2`

**Status:** CHOICE (the owner, 2026-10-23, plan D-45: "we kinda need
stronger invoice support. Like make it a first class citizen"; hand
entries to receivables and payables refused, the owner's answer 5);
CONVENTION (a control account agrees with its subledger, as B-23; the
subledger tie-out is the first thing a CPA checks); sources cited and not
yet vendored, so this rule keeps the CONVENTION label (the Open list in
`README.md`): ASC 310-10 (receivables), ASC 405-20-40-1 (a liability is
removed only when paid or released), ASC 606-10-45-4 (a receivable is a
right to consideration that is unconditional).

1. **Tags.** Every ledger line on the receivables or payables control
   account carries the document it belongs to (an invoice or a bill) and
   what it does to it: its origin, a payment (later: a discount, a
   write-off, an adoption, a role move). The tag lives beside the line,
   like its cleared flag; the line and its hash never change (B-8). The
   document commands write it in the step that posts the line; replay
   derives it from the document events already journaled (an invoice's
   or bill's own entry: its one line on the role's account as the role
   stood at that point of the journal; a payment: its first untagged
   control line for the payment's amount, so several payment events
   naming one entry tag its lines in order), each on the side its
   movement posts (an origin on the owed side, a payment on the other, so
   an older payment deposited into receivables itself pays with its
   credit); a reversal's mirror line carries its original's.
2. **Figures.** What a document owes as of a day is the signed sum of
   the lines tagged with it dated on or before that day (a debit to
   receivables counts plus for an invoice, a credit to payables for a
   bill); what it was paid is minus the same sum over its payment lines.
   Its status: Void when its own entry doesn't stand on that day (B-9;
   an entry stands while an even number of the reversals after it in its
   chain are dated by then, counted in chain order: a reversal counts
   only once what it reverses does), else Unattached when its own entry's
   control line carries no tag (an older entry replay couldn't match),
   else Overpaid when it owes less than nothing (a credit balance, owed
   back: never a receivable or payable, ASC 310-10), else Written off,
   Paid, Partly paid or Open by what it owes. Overdue is a flag (owed past
   the due date), not a status. No figure is stored: a reversal, a void
   or a bounced payment is right by construction, and the aging as of a
   day (B-18) shows what was owed then; it leaves out a document owing
   less than nothing, which the control check lists with the unattached
   ones for attention.
3. **The control guard.** A control account is one the receivables or
   payables role is assigned to, or one holding a line of one of the
   role's documents (the role's default code once a document posted
   there; an account the role moved off). A line on one that carries no
   document is refused, when added and when posted, whatever writes it
   (a hand entry, a template, a recurring entry); a recurring template
   naming one is refused when registered. The one exception is the
   mirror of an untagged line in a reversal that takes the line off the
   books (the hand line itself, or a re-instatement of it, reversed):
   reversing an old hand line is how it is cleaned up, and reversing that
   cleanup, which would put it back, is refused. The role never moves
   onto an account holding an untagged line (B-23). Replay never refuses
   (B-28). A chart's own account at a default code (1200 Equipment, a
   card at 2000) takes hand lines until a document posts there; lines it
   held before are listed as unattached from then on.
4. **Bounds.** At every day, 0 ≤ owed ≤ total, for every document
   written under these rules. A payment can't exceed what is owed on any
   day from its date on; a reversal that would break the bound where it
   held (a void under a payment that stands; re-instating a payment
   beside its replacement) is refused, naming what to reverse first;
   reversing a payment is always taken. Books an earlier version left out
   of bounds take any step that makes no day worse, so the remedy is
   accepted (a step may carry a day across the range, overpaid to
   over-owed, so long as it lands nearer the range). A reversal of a
   document's entry is never dated before that entry.
5. **Payments.** The deposit or payer account is a postable asset or
   liability that holds no role: a bank, a card, a member's payable. A
   control account (item 3) holds its role; another role is held by its
   assigned account, or by the account at its code of its type.
   The payment method and the processor's reference go on the payment's
   entry in the same step (B-19, D-40).
6. **Voids.** Voiding a document reverses its own entry, described
   "Void invoice 7: reason"; it keeps its id (and number), and reversing
   the void re-instates it (B-10).
7. **Journal.** A payment accepted only because an earlier payment on
   its document was reversed needs the journal feature
   `document_standing` (B-27): an older version, which adds every
   payment event to the document's paid amount (its line tagged or not),
   would show it paid above its total. Everything else is journaled as before, and a void is the
   `entry.reversed` it is.

```text
GOAL: invoices and bills are the subledgers of the receivables and
      payables control accounts; every figure about a document is read
      from the ledger lines tagged with it.
INVARIANTS:
  1. For every control account A and day D:
     GL(A, D) = Σ_docs outstanding(d, D) + unattached(A, D).
     No command adds an untagged control line; unattached changes only by
     reversing (or, later, adopting) a legacy line.
  2. 0 ≤ outstanding(d, D) ≤ total(d) at every D for every document
     written under these rules; replay accepts what was written.
  3. Reversing a tagged entry changes outstanding by exactly the opposite
     of its tagged lines, from the reversal's day; reversing that
     reversal restores it.
  4. A document's state is a function of (the journal, the day).
  5. Aging as of D partitions Σ outstanding(·, D) over the documents
     owing something; a credit balance (only in books an earlier version
     left out of bounds) is left out and listed by the control check.
  6. Every refusal leaves the books and the journal unchanged; NEVER panics.
SCENARIOS:
  1. Invoice 100, payment 100, payment reversed: Open, owing 100, A/R 100.
  2. Invoice reversed with nothing paid: Void, not in aging, id kept.
  3. Void with a payment standing: SHOULD refuse, naming the payment.
  4. As of a day between a payment and its reversal: counted as paid.
  5. Wrong payment reversed, right one recorded: accepted (feature
     document_standing declared first).
  6. Hand Dr 1200 / Cr 4000: SHOULD refuse with the document tools
     named; reversing a legacy hand line on 1200: accepted, unattached
     shrinks.
  7. (S7) Half paid, rest written off: Written off, 50 paid / 50 written off.
  8. receive_payment into 1200 or 4000: SHOULD refuse.
```

**Enforced:** `ledger_core::arap::{DocRef, DocTag, Movement, DocState,
DocStatus, doc_status, PaymentDetails, ControlCheck}`,
`kernel::{running_bounds, bounds_kept}`, `Ledger::{line_doc, doc_lines,
doc_state, invoice_state, bill_state, stands_on, in_effect,
control_role_of, control_check, payments_recorded, standing_payments,
receive_payment_with, pay_bill_with, void_document, check_void_document,
check_reversal, replay_reverse_entry, ar_aging, ap_aging}` (the guard in
`add_line`, `add_line_fx`, `post_entry` and `register_recurring`);
`Store::{receive_payment_with, pay_bill_with, void_document}` (feature
`document_standing`); `Ledger::check_assign_role` (B-23); the projection
(schema 15) keeps no document figure; the viewer reads what a document
owes and whether it is late from `doc_state` (as of today, or of its own
date when it is dated ahead).
**Checked:** `ledger-core` `tests/spec_documents.rs` (a test per
scenario and finding of its reviews; proptests
`subledger_equals_control`, `aging_partitions_what_documents_owe`,
`reversal_twice_restores_state`); `ledger-store`
`tests/arap_standing.rs` (the fixture an earlier version wrote, its
golden figures adjudicated; the feature gate; replay equals live),
`tests/arap_props.rs` (proptest `status_is_a_function_of_journal_and_date`:
live, replayed and snapshot books agree at every date, closed months
included), the store fuzz test's voids and payments with details;
Kani `bounds_check_matches_definition`, `bounds_kept_is_the_definition_on_books_in_bounds`;
Lean `Documents.control_agrees`, `Documents.Step.document_unattached`,
`Documents.Step.reverse_moves`, `Documents.Reachable.unattached_zero`
(through role assignments and releases),
`Documents.Reachable.control_is_documents`. The model's limits: it
starts from empty books (legacy lines are covered by the identity
`control_agrees` alone), it names documents by one id space (an injective
coding of `DocRef`), and an account becoming a control account by its
first document, holding lines from before, is outside it (those lines are
listed as unattached).

## B-41 A debt that will not be collected is written off, and a recovery is income

**bookspec:** `BS-arap-5`

**Status:** CHOICE (the owner, 2026-10-23, plan 8.3.1 A-5: bad debts by
the **direct write-off** method, and 8.3.2: for a small company's
receivables a CPA accepts the direct method and notes it, where GAAP
prefers an allowance under ASC 326-20). AUTHORITY for what the rule
rests on: IRC §166(a) (a debt that becomes worthless is deductible in
that year), Treas. Reg. §1.166-1(e) (a debt never counted as income has
no basis to deduct), §1.166-2(a) (worthlessness on all the pertinent
evidence), §1.166-3(a) (a partial write-off, to the extent charged off),
and IRC §111 (a recovery of an amount deducted is income in the year
received). ASC 405-20-40-1 for a bill's release.

1. **The write-off.** Writing off an invoice posts one balanced entry:
   Dr the bad-debt expense role (`bad_debts`, 6990) and Cr receivables
   for the amount, the receivable line carrying the invoice with the
   movement `WriteOff`, so what the invoice owes falls by the amount from
   the write-off's day and what was written off rises by it. The amount
   is above zero and at most what the invoice owes, and at most the least
   it owes on any day from the write-off's on (so no later day, a payment
   after it included, takes the document below nothing: B-38 INVARIANT 2).
   A partial write-off is allowed (§1.166-3(a)); the rest stays owed and
   aged.
2. **The day.** The write-off's day is in the calendar, in an open period
   (B-12), and not before the invoice's. A document's entry is reversed,
   never edited (B-8, B-9): a write-off is not a void and a void is not a
   write-off.
3. **Cash basis refused.** Books on the cash basis never counted the
   invoice as income, so the debt has no basis to deduct (§1.166-1(e)):
   the command refuses, changing nothing.
4. **Bills are never written off** (ASC 405-20-40-1): a liability leaves
   the books only when paid or when the creditor releases it, which is a
   vendor credit (not scheduled). The command takes an invoice.
5. **A recovery is not a new movement** (plan D-49). In the same tax year,
   reverse the write-off: the invoice reopens and owes what the write-off
   took off, and the payment is recorded as any payment. In a later year,
   record a plain entry Dr the deposit account / Cr a "Bad debt
   recoveries" revenue account (tax category other income), its memo
   naming the invoice: the recovery is income (§111) and the invoice stays
   written off. There is no `Movement::Recovery` and no bad-debt-recoveries
   role.
6. **Evidence.** The write-off's evidence of worthlessness — collection
   correspondence, a bankruptcy or liquidation notice (§1.166-2(a)) — is
   filed in the same call: a `receipt.attached` on the write-off's entry,
   linked to the invoice. A write-off with no evidence is refused. The tax
   judgments (whether the debt is worthless, and in which year) stay with
   the CPA: the books flag each write-off made this year for the CPA's
   note.
7. **Journal.** `invoice.written_off {invoice_id, entry_id, date, amount,
   reason}` is a new event type and needs journal feature
   `documents_v2` (B-27, D-52): an older version refuses it rather than
   misreading a document as still owed. Replay tags the receivable line
   `WriteOff`; the projection keeps no document figure (B-38).

```text
GOAL: write off what will not be collected, as the tax rules require.
SOURCE: IRC §166(a), §111; Treas. Reg. §1.166-1(e), (f), -2(a), -3(a);
        ASC 326-20 and ASC 405-20-40-1 noted; CHOICE: the direct
        write-off method (plan 8.3.1 A-5).
INVARIANTS:
  1. amount <= outstanding on the write-off's day and on every day from
     it; Dr bad_debts / Cr receivables [d, WriteOff]; the evidence of
     worthlessness filed in the same call.
  2. Its reversal re-instates the debt; bills are never written off.
  3. Refused on cash-basis books.
  4. A refusal leaves the books and the journal unchanged; NEVER panics.
SCENARIOS:
  1. Half paid, the rest written off: Written off, with 50 paid / 50
     written off.
  2. A write-off of 0, a negative amount, or more than is owed: SHOULD
     refuse, naming what is owed.
  3. Cash-basis books: SHOULD refuse (no basis to deduct).
  4. The write-off reversed in the same year and the payment recorded:
     Paid; receivables back to zero.
  5. A later year's recovery: a plain entry to a bad-debt-recoveries
     revenue account; the invoice stays written off.
```

**Enforced:** `ledger_core::Ledger::write_off` (and its replay
`replay_invoice_write_off`), `LedgerError::{WriteOffOnCashBasis,
InvalidWriteOff}`, the `bad_debts` control role (`BAD_DEBTS`, 6990);
`ledger_store::Store::write_off_invoice` (feature `documents_v2`; the
evidence filed and linked in the same call), `WriteOffEvidence`,
`StoreError::WriteOffNeedsEvidence`; `books.write_off_invoice`
(`crates/ledger-mcp/src/tools/write_offs.rs`); the viewer's timeline
(`Movement::WriteOff`); Needs attention's write-off item.
**Checked:** `ledger-core` `tests/spec_documents.rs`
(`half_paid_rest_written_off_is_written_off`,
`write_off_refused_on_cash_basis`, `write_off_above_outstanding_refused`,
`write_off_then_recovery_same_year`); `ledger-store`
`tests/arap_standing.rs` (`write_off_waits_for_documents_v2`,
`write_off_replays_and_projects`, `write_off_without_evidence_is_refused`),
`tests/arap_props.rs` (`status_is_a_function_of_journal_and_date` with
write-offs); over the wire `crates/ledger-cli/tests/documents.rs`
(`write_off_then_recovery`, `write_off_needs_evidence`); bookspec
`bad-debt-direct-write-off`; Lean `Documents.Step.writeOff_unattached`,
`Documents.writeOff_within_bounds`; the viewer
`a_write_off_shows_on_the_timeline`; the item
`a_write_off_is_flagged_for_the_cpa`.

## B-42 A settlement is exact and names its lines

**bookspec:** `BS-arap-6`

**Status:** CONVENTION (a payment that settles several documents at once is
still one balanced entry, and each control line carries its document; the
subledger tie-out is B-38's); CHOICE (the gross method for an
early-payment discount, A-8; the processor's fee posted gross, A-4; D-49's
refusal to apportion a discount across a bill's asset lines). ASC
606-10-32-5 to 32-9 are the strict treatment of an expected discount;
this company's size makes the gross method immaterial (A-8). Sources are
cited and not yet vendored, so this rule keeps the CONVENTION label (the
Open list in `README.md`).

1. **One party, one kind.** A settlement applies to one or more invoices
   of one customer, or one or more bills of one vendor, never a mix, and
   never the same document twice. Deposits: Σ cash + Σ discounts is what
   it applies to the documents; Σ cash − fee is what reaches the deposit
   account (`kernel::settle`). The arithmetic is exact: no epsilon, no
   rounding, and a fee above the cash is refused.
2. **Bounds.** Each application's cash + discount is at most what its
   document owes on every day from the settlement's date on
   (`payment_room`), so no day is overpaid. A payment is never dated
   before its document, and never in a closed period.
3. **Discounts.** A discount is at most `kernel::discount_max(total,
   percent_bp)` for the document's terms and is dated on or before
   `terms.discount_by(date)`; outside that it needs a reason, journaled
   with the payment (a vendor honouring a late discount). Terms that
   state no discount are the same: any discount needs a reason. A discount
   on a bill whose lines include an asset or prepaid account is refused
   with D-49's remedy: record the bill net of the discount, or reclassify
   Dr 6995 Purchase Discounts / Cr the asset.
4. **The fee.** A processor's fee is a customer payment's only. It posts
   gross: Dr the processing-fee expense, Dr the deposit account the net,
   Cr accounts receivable the gross. Only cash movements count toward a
   1099, so the fee is not a payment to the processor.
5. **Lines and journal.** The entry's rows are the payment lines in
   application order, then the discount lines, then the fee line, then
   the cash line; a new payment event names the line it tags (`line`) and,
   when it took one, its discount and discount line (`discount`,
   `discount_line`, `discount_reason`), and the fee's on the first
   application's event (`fee`, `fee_line`). Replay tags exactly those
   lines; a legacy event with no `line` takes the first untagged control
   line for its amount (B-38 item 1). A line replay cannot match stays
   untagged and the control check lists it: replay never refuses (B-28).
6. **Exports.** An aging as of a day lists one row per open document with
   its number, party, dates, terms, total, paid, discounted, outstanding,
   bucket and days overdue, then a tie-out line against its control
   account (B-23, B-38 INVARIANT 1). The CPA package and `books.export`
   write the same CSV.

```text
GOAL: a settlement is exact and names its lines.
SOURCE: CONVENTION; ASC 606-10-32-5 to 32-9 noted; CHOICE: the gross
        method (A-8), the fee's gross presentation (A-4).
INVARIANTS:
  1. Σ cash + Σ discounts = Σ applications; deposit = Σ cash − fee
     (kernel::settle); one party per settlement; one application per
     document.
  2. Each application's cash + discount ≤ what the document owes on every
     day from the payment's date on.
  3. Each new payment event names the line it tags; replay tags that line;
     a legacy event without one takes the first untagged control line for
     its amount.
  4. A discount on a bill with an asset or prepaid line is refused.
SCENARIOS:
  1. Two invoices of one customer, 300 and 80, paid 380 by one deposit:
     two payment lines, each tagged, one cash line.
  2. "2/10 net 30" bill 100, paid 98 within ten days: Dr A/P 98 (payment)
     and 2 (discount), Cr purchase discounts 2, Cr bank 98.
  3. The same discount after the tenth day without a reason: refused,
     naming discount_reason; with one: taken and journaled.
  4. An invoice 100 paid through a processor keeping 3: Dr bank 97, Dr
     processing fees 3, Cr A/R 100.
  5. A discount on a bill whose line is equipment: refused with the
     record-net or reclassify remedy (D-49).
```

**Enforced:** `kernel::settle`, `Ledger::{settle, settlement_lines,
check_discount}`, `arap::{Application, Fee, SettleParams, SettlementRow}`,
`Ledger::{replay_invoice_payment_at, replay_bill_payment_at}`,
`Store::settle`, the aging builders (`Ledger::aging_documents`,
`ledger_package::{aging_csv, aging_statement}`). **Checked:** `ledger-core` `tests/spec_documents.rs`
(`bill_on_terms_with_discount`, `late_discount_needs_a_reason`,
`discount_on_an_asset_bill_is_refused_with_the_remedy`,
`a_fee_posts_gross_to_receivables_and_net_to_the_bank`,
`a_settlement_names_its_lines_and_refuses_a_document_twice`),
`tests/kernel_bridge.rs::settle_agrees`; Kani `settle_is_exact`;
`ledger-store` `tests/arap_standing.rs`
(`a_settlement_names_its_lines_and_replays`, `settlement_waits_for_documents_v2`,
`legacy_payment_events_keep_the_positional_rule`), `tests/arap_props.rs`;
`ledger-cli` `tests/documents.rs`; `ledger-package`'s aging tie-out test.
The Lean step is `Documents.Step.document` with every control line tagged
(`Documents.settle_unattached`); no new `Step` constructor.
