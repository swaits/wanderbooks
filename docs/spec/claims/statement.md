# Claims: statement

Wanderbooks' claims for bookspec area **statement**: what each rule requires, where the engine enforces it, and which tests check it. Being reduced to the engine's pick and its evidence as P14 unit S-D lands; the accounting text moves to bookspec's `spec/statement.md`.

Rules: B-6, B-7, B-14, B-33, B-37.

---

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
`tests/bookspec/`).

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

