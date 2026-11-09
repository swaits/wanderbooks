# Claims: statement

Wanderbooks' claims for bookspec area **statement**: the engine's pick for each rule, where it enforces it, and which tests check it. The accounting text lives in bookspec's records (`rules/statement/`).

Rules: B-6, B-7, B-14, B-33, B-37.

---

## B-6 The trial balance balances

**bookspec:** `BS-statement-1`

**Claims:** yes

**Enforced:** follows from B-1. **Checked:** Kani `trial_balance_columns`;
Lean `trial_balance_iff`, `Reachable.trial_balance`,
`Reachable.trial_balance_through`; properties.

## B-7 The accounting equation

**bookspec:** `BS-statement-2`

**Claims:** yes

**Enforced:** every posted entry obeys B-1, so `kernel::check_lines`
refuses an unbalanced one; `Ledger::post_entry` applies it to every
posting, and `kernel::normal_balance` reads each account by its type, so
`reports::balance_sheet` and `reports::trial_balance` present the same
assets and liabilities-plus-equity at every date.
**Checked:** Kani `accounting_equation`; Lean `accounting_equation`,
`Reachable.accounting_equation[_through]`; property
`prop_balance_sheet_balances`.

## B-14 Statements: P&L for a period; cash flows by the direct method

**bookspec:** `BS-statement-3`

**Pick (BS-statement-6):** `direct`, the owner, 2026-10-20

**Pick (BS-statement-7):** `constructive`, the owner, 2026-10-20, D-42

**Claims:** partly (bookspec's record states rules 1-12; the picks and the product surface are below)

**Pick:** the direct method; an entry's cash allocated to its sections in the order operating, investing, financing (CONVENTION, the ASC silent, disclosed); within a section, the cash split among its kinds in the order the rest of the section's kinds, interest received, interest paid (CONVENTION, as the engine splits it); what a member or a lender pays or receives for the company presented as constructive cash, borrowed (or contributed) and spent (the owner, D-42, 2026-10-20, revising D-35's first point); a payment shared between a government and others split by their resolved shares, the government's part rounded half to even to the base currency's places and the rest exact (CONVENTION); payroll entries keep their (d) class whoever they name (CONVENTION).

**Enforced:** `profit_and_loss_for_period`; `cashflow::cash_flow` (with
`kernel::entry_flows`, `apply_cash`, `constructive`, `exchanged`),
`role_problems`, `foot`; `Ledger::reversal_chain` (rule 6's chains);
`ledger_package::cash_flow_for` (the server, the
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

### Product rules

**WB-statement-1** Which accounts are cash, investing or financing in
wanderbooks is book configuration: the `books.cash_accounts`,
`books.financing_accounts` and `books.investing_accounts` settings (the
portable form is bookspec's examples' `setup.cash_accounts` etc.). One
check (`cashflow::role_problems`) is behind both the statement's refusal
and `books.settings`' problems, and the refusal names the setting to
change.

**Enforced:** `cashflow::role_problems`, `books.cash_accounts`,
`books.financing_accounts`, `books.investing_accounts`. **Checked:** the
role refusals in `crates/ledger-core/tests/spec_cash_flow.rs`.

**WB-statement-2** The flags `asset_on_operating_payable` and
`asset_sold_on_operating_receivable` are the statement's `warnings` and
items in the viewer's Needs attention, each with its fix (post the asset
to an investing account). The printed statement and the viewer show
operating one row per class, and investing and financing one row per
kind with its total, each transaction listed under it (D-35);
`books.cash_flow` and the exports carry every line. A distribution
payable's line reads "Distributions to members — <the payable>" (the
statement reads no parties, so not by member). A government selling
goods or services is added as a vendor (`books.add_vendor`, the `vendor`
role), never given `government_agency`; a payroll liability kept at a
code of the chart's own needs its role (`books.assign_role`) to be read
as payroll.

**Enforced:** `cashflow::cash_flow` (`warnings`),
`ledger_package::cash_flow_for`, `books.cash_flow`. **Checked:**
`equipment_on_the_sellers_credit_stays_noncash_and_flagged`,
`a_bill_payment_carries_its_bills_split`,
`a_government_selling_services_is_a_supplier`.

**WB-statement-3** Rule 11's wanderbooks limits, from what the books
cannot tell: a lender's or member's own charge credited to what the
company owes them, other than interest and guaranteed payments (a card's
annual fee, rent a member charges), reads as paid for the company, so
post what the company owes a member for their own goods or services to a
payable, not to the members' financing account. A note to the seller
listed in `books.financing_accounts` reads as a lender's loan: keep a
seller's note out of it. An expense a member contributes in kind reads
as paid: post it to an asset first. A debt a member or a new lender pays
off directly is a noncash exchange (disclosed). Debt issue costs posted
to an expense account read as operating: post them to a contra-liability
listed as financing. An entry that moves revenue or expense straight to
capital by hand reads as paid or received for the company: close periods
with `books.close_period`, whose entries are never in the statement.

**Enforced:** `cashflow::cash_flow`, `kernel::constructive`. **Checked:**
`a_fee_withheld_from_a_loan_follows_its_account`,
`interest_and_guaranteed_payments_charged_to_a_creditor_stay_noncash`,
`property_a_member_contributes_stays_noncash`.

## B-33 A package as of a day shows the books through that day

**bookspec:** `BS-statement-4`

**Pick (BS-statement-8):** `year-through-as-of`, the owner, 2026-10-20, D-36

**Claims:** partly (bookspec's record states the as-of principle and the calendar-year counting; the CPA package itself is wanderbooks product, below)

**Pick:** the 1099 worksheet stops at as_of too (the owner, 2026-10-20, plan D-36).

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

### Product rules

**WB-statement-4** The CPA package (`books.export_cpa_package`, the
viewer's Export for CPA) made as of a day holds: the statements and the
ledgers covering the fiscal year containing as_of, through it; the
balance sheet, the trial balance and the fixed asset register as of it
(the register only when an asset was bought by then); receipts/ holding
the evidence of the year's entries through it; the members' equity
statement and the K-1 worksheet, the year to date; general-journal.iif
with every posted entry from the first through it (B-34). The chart of
accounts and documents/ are as they stand when the package is made. One
list of cash accounts and one of financing accounts serve the whole
package (the request's, else the settings':
`ledger_package::Settings::with_accounts`), so the package's 1099
worksheet is `books.vendor_1099`'s CSV for the same `cash_accounts`,
`financing_accounts` and `through` = as_of, byte for byte; the tool,
given no accounts, uses the settings', as a package given none does.

**Enforced:** `ledger_package::build_package`,
`ledger_package::Settings::with_accounts`. **Checked:**
`every_file_in_an_as_of_package_stops_at_as_of`,
`one_list_of_cash_accounts_per_package`,
`cpa_package_holds_the_year_and_its_evidence`.

## B-37 A project is a class with its facts; its figures are its tagged lines

**bookspec:** `BS-statement-5`

**Pick (BS-statement-9):** `one-project-per-line`, the owner, 2026-10-22, D-43

**Claims:** partly (bookspec's record states the class, the figures, on-spec and document-line rules; the project facts and the snapshot presentation are wanderbooks product, below)

**Pick (BS-statement-5):** `project-is-a-class`, the owner, 2026-10-21.

**Pick:** a project is a class (the owner, 2026-10-21: "a Projects page that treats a class as a project ... Projects are entities too and can have name, description, whatever other metadata you think they should have"; the convention `link-to-projects` in the books: "whenever applicable, we link things to [projects]"); an invoice's or bill's lines take a project, each line one or none (the owner, 2026-10-22, plan D-43).

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

### Product rules

**WB-statement-5** A project's facts (bookspec keeps only that a project
is a class with its figures): a description (its scope of work), a
status (`active`, `paused`, `finished`, `abandoned`), a kind (`internal`:
the company's own product; `client`: work for a customer; `on_spec`:
work done hoping to be paid; `other`; or not recorded), a party (who it
is for or with), the dates it started and ended (an end only when
finished or abandoned, never before the start; informational: no figure
filters by them), up to eight links (web addresses, never opened) and
notes. A class an earlier version wrote has the default facts: active,
kind not recorded. Names are one line, unique ignoring capitals (in any
script) and surrounding space, and never digits only (a project is named
by id or name, so such a name would read as an id); no field holds a
full TIN (B-26). A change that keeps a project's own name never checks
the name again, so old books holding a duplicate can still change either
project, and a fact-only change never renames. Journaled as
`class.added`, then the whole record as `project.set` when there is more
than a name (journal feature `projects`, B-27). Replay checks only what
it cannot do without (the class and the party exist), never the text
rules, so a rule tightened later never breaks books already kept. No
budget, estimate, contract value or percent complete: each is a document
linked to the project, a revenue-recognition question the books do not
face, or the CPA's.

**Enforced:** `ledger_core::classes::{ProjectFacts, ProjectStatus,
ProjectKind}`, `Ledger::{check_set_project, set_project,
replay_set_project}`, `Store::{add_project, set_project}`.
**Checked:** `tests/spec_projects.rs`, `tests/projects.rs`,
`prop_project_set_round_trips`.

**WB-statement-6** Links and notices: a line is tagged with a project
(its class); a receipt or document is linked to a project (attachment
subject `project`); a decision names the projects it is about. Whether
to link is the company's convention
(`link-to-projects`). Finished and abandoned projects still take lines,
and `books.add_line`, `books.record_payout`, `books.create_invoice` and
`books.create_bill` say so when one is tagged. An invoice line naming an
on-spec project is taken, and the tools ask whether there is now an
agreement. A line naming a project the books do not have is refused, the
document whole. Omitted dates in `books.project` mean what B-30 says; to
date is always stated as such, beside it.

**Enforced:** `Store::subject_feature`, `books.create_invoice`,
`books.create_bill`, `books.project`. **Checked:**
`tests/spec_document_projects.rs`, `tests/document_projects.rs`.

**WB-statement-7** The snapshot (a presentation choice, not GAAP): a
project's net is shown as its contribution before overhead, its revenue
less its direct costs (every expense line tagged to it), with no company
overhead allocated. This is Garrison, Noreen and Brewer's segment margin
(a segment's sales less its variable expenses and its traceable fixed
expenses), not Horngren's contribution margin (revenue less variable
costs only), which the books cannot compute and the viewer does not
claim; "contribution" is only the everyday sense, what the project
contributes toward overhead, so a project in the black can still leave
the company in the red. Its margin ("contribution margin" on the page,
`margin_pct` in the tools) is that contribution as a percent of revenue,
one place, half to even, and none without revenue (zero or less). "In
the black" is a contribution above zero, "in the red" below,
"break-even" lines tagged that net to zero (a cost and its reversal).
The running net by month is the net to date through each month's end;
its last is the net to date. By month, the period's last 240 months at
most. No GAAP standard defines or requires these figures (ASC 280's
segment reporting is for public entities' operating segments, not
projects); they come from one function the tools and the viewer share.

**Enforced:** `reports::{project_figures, project_to_date,
contribution_margin_pct}`, `ProjectToDate::margin_pct`. **Checked:**
`prop_project_contribution_is_the_net_to_date`,
`prop_snapshot_contribution_is_the_net_to_date`, Kani
`unrecovered_cost_laws`.

**WB-statement-8** The statement's words and groupings are
wanderbooks' product strings, not accounting (the accounting is
BS-statement-3's statements 5, 11 and 13): a reversal shown alone reads
"Reversal of #N — <N's description>" and a reinstatement "Reinstatement
of #A (reverses #B)", both listed under the original's kind; a
financing inflow under a borrowing reads "Proceeds from borrowing —
<account>" and a contribution "Contributions from members — <member>";
the rule-4 gain or loss line reads "(Gain) loss and other noncash changes
in investing assets"; and the reconciliation's noncash items read
"Depreciation" (the entry's investing and financing balance-sheet lines
all accumulated depreciation), "Noncash: through <account>" (all on that
account, accumulated depreciation aside) or "Other noncash items", with
each operating balance-sheet account's line reading "Increase in
<account>" or "Decrease in <account>". The engine builds these strings in
one place so every surface agrees.

**Enforced:** `cashflow::cash_flow` (`label`, `reconcile`). **Checked:**
`a_reversal_of_a_reversal_is_shown_and_foots`,
`a_reversal_three_levels_down_is_labelled_a_reversal`,
`a_payment_made_for_the_company_and_its_reversal_net`,
`an_expense_a_member_paid_is_borrowed_and_spent`,
`defect1_two_member_llc_first_year_regression`,
`a_disposals_depreciation_credit_back_is_depreciation`.

**WB-statement-9** A document's entry posts one revenue or expense line
per project its lines name, in the order each is first named, each the
sum of that project's lines, and the untagged lines' sum as one line
(`arap::document_parts`); a document naming none posts exactly the one
line every document posted before. The receivable or payable line and
the payments against it take no project. Journaled as the line's
`class_id` in `invoice.created` or `bill.created` and an
`entry.line_class_set` right after its revenue or expense line's
`entry.line_added`, behind the journal feature `document_line_projects`
(B-27).

**Enforced:** `arap::document_parts`, `Ledger::{create_invoice,
create_bill}`, `Store::{create_invoice, create_bill}` (feature
`document_line_projects`). **Checked:** `tests/spec_document_projects.rs`,
`tests/document_projects.rs`, proptest
`prop_document_parts_partition_the_total`.
