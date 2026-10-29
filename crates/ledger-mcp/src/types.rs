//! Tool parameter and output types. Amounts cross MCP as decimal strings,
//! never JSON floats.

use ledger_core::calendar::{AS_OF_DOC, END_DOC, START_DOC, YEAR_DOC};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Chart and accounts
// ---------------------------------------------------------------------------

/// Parameters for `books.add_account`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct AddAccountParams {
    /// Account code, e.g. "1000". Must be unique.
    pub code: String,
    /// Human name, e.g. "Business Checking": one line of at most 200
    /// characters, kept trimmed; no other account under the same parent
    /// has it (ignoring capitals), and it holds no run of nine digits
    /// (write an account number's last four).
    pub name: String,
    /// One of: asset, liability, equity, revenue, expense.
    pub account_type: String,
    /// Parent account id for hierarchy. Omit for top-level.
    pub parent: Option<u64>,
    /// True for header accounts that never take postings.
    pub placeholder: Option<bool>,
    /// The tax category of a revenue or expense account (a key from
    /// `books.tax_categories`): the line of the return it maps to.
    #[serde(default)]
    pub tax_category: Option<String>,
    /// Percent deductible, overriding the category's default (0 to 100),
    /// as a decimal string. Needs tax_category.
    #[serde(default)]
    pub deductible_percent: Option<String>,
}

/// Result of `books.add_account`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct AddAccountOut {
    /// The new account's id.
    pub account_id: u64,
    /// What to tell the human: a revenue or expense account created
    /// without a tax category (the tax summary lists it as uncategorized
    /// until it has one).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// Parameters for `books.rename_account`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RenameAccountParams {
    /// The account's code (or id).
    pub account: String,
    /// Its new name, kept trimmed: one line of at most 200 characters,
    /// not the one it has, not one another account under the same parent
    /// has (ignoring capitals), and no run of nine digits.
    pub name: String,
    /// Why, in a line or two, for the journal (the human's words, or what
    /// they asked). Optional.
    #[serde(default)]
    pub reason: Option<String>,
}

/// Result of `books.rename_account`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct RenameAccountOut {
    /// The account's id (unchanged).
    pub id: u64,
    /// Its code (unchanged).
    pub code: String,
    /// The name it had: the journal keeps it.
    pub previous_name: String,
    /// The name it has now.
    pub name: String,
    /// What else the new name changes: a set of accounts picked by name
    /// it joins or leaves (cash and card accounts, when the settings name
    /// none), conventions whose text names the old name, and the
    /// QuickBooks import (IIF matches accounts by name).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

/// One account in the chart.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ChartAccount {
    /// Account id.
    pub id: u64,
    /// Account code.
    pub code: String,
    /// Human name.
    pub name: String,
    /// Account type.
    pub account_type: String,
    /// Parent account id, if any.
    pub parent: Option<u64>,
    /// True for header accounts.
    pub placeholder: bool,
    /// Current balance in the account's normal side, as a decimal string.
    /// Positive means a normal balance.
    pub balance: String,
    /// Tax category key, if assigned.
    pub tax_category: Option<String>,
    /// Deductible percent override, if any.
    pub deductible_percent: Option<String>,
}

/// Parameters for `books.create_schedule`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateScheduleParams {
    /// "prepaid_expense" (asset expensed monthly) or "deferred_revenue"
    /// (liability earned monthly).
    pub kind: String,
    /// What it is, e.g. "2027 liability insurance".
    pub description: String,
    /// Amount to spread, decimal string.
    pub total: String,
    /// The first day covered, YYYY-MM-DD: any day for daily; the 1st of a
    /// month for monthly_even.
    pub start: String,
    /// Months covered.
    pub months: u32,
    /// Prepaid (asset) or deferred revenue (liability) account code.
    pub balance_account: String,
    /// Expense or revenue account code.
    pub pl_account: String,
    /// "daily" (by the day from start, like depreciation: a mid-month
    /// start posts a small first and last month, one more month end than
    /// its months) or "monthly_even" (equal months, starting on the 1st).
    /// Default: the books' `schedule_convention` setting, else
    /// daily.
    #[serde(default)]
    pub convention: Option<String>,
}

/// One month of a schedule.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ScheduleMonthOut {
    /// Month-end date the posting lands on.
    pub date: String,
    /// Amount, decimal string.
    pub amount: String,
    /// True once posted.
    pub posted: bool,
}

/// An amortization schedule.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ScheduleOut {
    /// Schedule id.
    pub schedule_id: u64,
    /// Description.
    pub description: String,
    /// "prepaid_expense" or "deferred_revenue".
    pub kind: String,
    /// Balance-sheet account code.
    pub balance_account: String,
    /// P&L account code.
    pub pl_account: String,
    /// Total, decimal string.
    pub total: String,
    /// "daily" or "monthly_even".
    pub convention: String,
    /// The first day covered, YYYY-MM-DD (for a monthly_even schedule an
    /// earlier version took mid-month, the 1st of its month).
    pub start: String,
    /// The last day covered, YYYY-MM-DD; null past the last date the books
    /// can compute.
    pub end: Option<String>,
    /// The next month end to post, YYYY-MM-DD; null once every month has
    /// posted (or when the plan can't be computed: see `plan_error`).
    pub next: Option<String>,
    /// Not yet recognized, decimal string. Null when the plan can't be
    /// computed (see `plan_error`).
    pub remaining: Option<String>,
    /// Every month (none when the plan can't be computed).
    pub months: Vec<ScheduleMonthOut>,
    /// Why the plan can't be computed, when it can't: a schedule an
    /// earlier version accepted whose months run past the last date the
    /// books can compute. It opens and lists, and never posts.
    pub plan_error: Option<String>,
}

/// Result of `books.list_schedules`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ListSchedulesOut {
    /// Schedules.
    pub schedules: Vec<ScheduleOut>,
}

/// Parameters for `books.run_schedules`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RunSchedulesParams {
    /// Post months ending on or before this date (default today).
    pub through: Option<String>,
    /// Without true, a preview: nothing posts.
    #[serde(default)]
    pub confirmed: bool,
}

/// One posting from a schedule run.
#[derive(Debug, Serialize, JsonSchema)]
pub struct SchedulePostingOut {
    /// Schedule id.
    pub schedule_id: u64,
    /// Month-end date.
    pub date: String,
    /// Entry description.
    pub description: String,
    /// Amount, decimal string.
    pub amount: String,
    /// The posted entry (absent in a preview).
    pub entry_id: Option<u64>,
}

/// A schedule month a run leaves unposted.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ScheduleSkippedOut {
    /// Schedule id.
    pub schedule_id: u64,
    /// Month-end date.
    pub date: String,
    /// Why it can't post.
    pub reason: String,
}

/// Result of `books.run_schedules`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct RunSchedulesOut {
    /// "preview" or "posted".
    pub status: String,
    /// Postings due (preview) or made.
    pub postings: Vec<SchedulePostingOut>,
    /// Months due that a run leaves for a human: dated in a closed period,
    /// or posted and then reversed after the month ended (posting it again
    /// would recognize it twice); a human books each in an open period by
    /// hand, or not at all.
    pub skipped: Vec<ScheduleSkippedOut>,
}

/// One line of a recurring entry.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RecurringLineParam {
    /// Account code.
    pub account: String,
    /// "debit" or "credit".
    pub side: String,
    /// Decimal string.
    pub amount: String,
}

/// Parameters for `books.create_recurring`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateRecurringParams {
    /// Description for each generated draft.
    pub description: String,
    /// "weekly", "monthly", "quarterly", or "yearly".
    pub frequency: String,
    /// First occurrence, YYYY-MM-DD (monthly and longer repeat on this day,
    /// clamped to month end).
    pub start: String,
    /// Last date an occurrence may fall on, YYYY-MM-DD; omit for none.
    pub end: Option<String>,
    /// Balanced lines.
    pub lines: Vec<RecurringLineParam>,
}

/// A recurring entry.
#[derive(Debug, Serialize, JsonSchema)]
pub struct RecurringOut {
    /// Recurring entry id.
    pub recurring_id: u64,
    /// Description.
    pub description: String,
    /// Frequency.
    pub frequency: String,
    /// First occurrence.
    pub start: String,
    /// Last date, if it ends.
    pub end: Option<String>,
    /// Occurrences generated so far.
    pub generated: Vec<String>,
}

/// Result of `books.list_recurring`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ListRecurringOut {
    /// Recurring entries.
    pub recurring: Vec<RecurringOut>,
}

/// Parameters for `books.run_recurring`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RunRecurringParams {
    /// Generate occurrences on or before this date (default today).
    pub through: Option<String>,
}

/// One draft from a recurring run.
#[derive(Debug, Serialize, JsonSchema)]
pub struct RecurringDraftOut {
    /// Recurring entry id.
    pub recurring_id: u64,
    /// Occurrence date.
    pub date: String,
    /// The draft entry to review, attach a receipt to, and post.
    pub entry_id: u64,
}

/// A recurring occurrence left ungenerated (its date is in a closed period).
#[derive(Debug, Serialize, JsonSchema)]
pub struct RecurringSkippedOut {
    /// Recurring entry id.
    pub recurring_id: u64,
    /// Occurrence date.
    pub date: String,
}

/// Result of `books.run_recurring`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct RunRecurringOut {
    /// Drafts created.
    pub drafts: Vec<RecurringDraftOut>,
    /// Occurrences not generated because their date falls in a closed
    /// period. Ask the human: book them by hand in an open period, or
    /// leave them out.
    pub skipped_closed_period: Vec<RecurringSkippedOut>,
}

/// A revenue or fee line of a payout.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct PayoutLineParam {
    /// Account code or id.
    pub account: String,
    /// Positive decimal string.
    pub amount: String,
    /// Optional class (a project: name or id; see books.projects).
    pub class: Option<String>,
}

/// An adjustment on a payout statement.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct PayoutAdjustmentParam {
    /// Account code or id.
    pub account: String,
    /// "debit" reduces the deposit (refund, chargeback, reserve held, tax
    /// withheld); "credit" increases it (reserve released, adjustment in
    /// your favor).
    pub side: String,
    /// Positive decimal string.
    pub amount: String,
    /// Optional class (a project: name or id; see books.projects).
    pub class: Option<String>,
}

/// Parameters for `books.record_payout`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RecordPayoutParams {
    /// Deposit date, YYYY-MM-DD.
    pub date: String,
    /// E.g. "App Store payout, October 2026".
    pub description: String,
    /// Account the money landed in.
    pub deposit_account: String,
    /// Net deposit, decimal string; negative when the platform debited you.
    pub deposit: String,
    /// Gross revenue lines (credited).
    pub revenue: Vec<PayoutLineParam>,
    /// Fees withheld (debited).
    #[serde(default)]
    pub fees: Vec<PayoutLineParam>,
    /// Everything else on the statement.
    #[serde(default)]
    pub adjustments: Vec<PayoutAdjustmentParam>,
}

/// Result of `books.record_payout`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct RecordPayoutOut {
    /// The draft entry.
    pub entry_id: u64,
    /// "draft".
    pub status: String,
    /// Gross revenue.
    pub gross: String,
    /// Fees withheld.
    pub fees: String,
    /// Net effect of adjustments on the deposit.
    pub adjustments: String,
    /// The deposit.
    pub deposit: String,
    /// The entry's lines, e.g. "Debit 1000 1265.01".
    pub lines: Vec<String>,
    /// A note for each finished or abandoned project a line was tagged
    /// to, as books.add_line gives; left out when none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    /// What to do next.
    pub next: String,
}

/// A vendor.
#[derive(Debug, Serialize, JsonSchema)]
pub struct VendorOut {
    /// Vendor id.
    pub id: u64,
    /// Name.
    pub name: String,
    /// "1099-NEC" or "1099-MISC" when its 1099 decision in force today
    /// files one; absent otherwise.
    pub form_1099: Option<String>,
    /// Its 1099 decision in force today. value "undetermined" with no
    /// decided_by: nobody has decided.
    pub decision_1099: Decision1099Out,
    /// Every 1099 decision recorded, in date order (the one from the
    /// beginning first): more than one when its status changed on a date.
    pub decisions_1099: Vec<Decision1099Out>,
    /// W-9 on file.
    pub tin_on_file: bool,
    /// The party the vendor record belongs to.
    pub party_id: Option<u64>,
}

/// A vendor's 1099 decision.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Decision1099Out {
    /// "nec", "misc", "not_reportable" or "undetermined".
    pub value: String,
    /// With not_reportable: the kind of payee (corporation, tax_exempt,
    /// united_states, state, foreign_government, foreign_person,
    /// merchandise, employee, other).
    pub reason_kind: Option<String>,
    /// With not_reportable: the facts, in words.
    pub reason: Option<String>,
    /// With not_reportable: the rule that exempts that kind of payee.
    pub authority: Option<String>,
    /// The day it takes effect, YYYY-MM-DD; absent: from the beginning.
    pub effective: Option<String>,
    /// Who decided; absent for a vendor nobody decided, or a decision
    /// recorded before decisions were kept.
    pub decided_by: Option<String>,
    /// When they decided, YYYY-MM-DD.
    pub decided_on: Option<String>,
    /// What it rests on, if anything.
    pub citation: Option<String>,
    /// Anything kept with it.
    pub note: Option<String>,
}

/// Result of `books.list_vendors`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ListVendorsOut {
    /// Vendors, by id.
    pub vendors: Vec<VendorOut>,
}

/// Parameters for `books.set_vendor_1099`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetVendor1099Params {
    /// Vendor id or name.
    pub vendor: String,
    /// The decision: "nec" (1099-NEC: contractors, freelancers,
    /// attorneys), "misc" (1099-MISC: rent, royalties, prizes),
    /// "not_reportable" (with reason_kind and reason), or "undetermined".
    /// Omit to change only tin_on_file.
    pub form_1099: Option<String>,
    /// With not_reportable: the kind of payee no 1099 is required for:
    /// corporation, tax_exempt, united_states, state, foreign_government,
    /// foreign_person, merchandise (sells only merchandise), employee, or
    /// other (give a citation). Never corporation for a payee paid for
    /// legal services or for medical and health care (a law firm or a
    /// medical practice, often a "P.C."): those payments stay reportable
    /// even to a corporation (Treas. Reg. §1.6041-3(p)(1)).
    pub reason_kind: Option<String>,
    /// With not_reportable: the facts, in words ("a corporation by name",
    /// "Delaware public benefit corporation, from the public record").
    pub reason: Option<String>,
    /// The day the decision takes effect, YYYY-MM-DD: only when the
    /// payee's status changed (an LLC's corporate election takes effect
    /// on its Form 8832 date; a new W-9 showing a new classification, on
    /// the form's date). Omit for its status from the beginning.
    pub effective: Option<String>,
    /// A W-9 is on file (the TIN itself is never stored). Omit to keep it
    /// as it is.
    pub tin_on_file: Option<bool>,
    /// Who decided (a person's name): required with form_1099.
    pub decided_by: Option<String>,
    /// When they decided, YYYY-MM-DD. Default: today.
    pub decided_on: Option<String>,
    /// The rule or document it rests on ("Treas. Reg. §1.6041-3(p)(1)", a
    /// W-9 dated 2026-03-02).
    pub citation: Option<String>,
    /// Anything else worth keeping with it.
    pub note: Option<String>,
}

/// Result of `books.set_vendor_1099`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct SetVendor1099Out {
    /// The vendor as it stands now.
    #[serde(flatten)]
    pub vendor: VendorOut,
    /// What the decision did: "set", "removed" (a dated decision taken
    /// out: the one before it applies again), or "unchanged" (already so;
    /// nothing journaled but a changed W-9 flag).
    pub changed: String,
    /// Payments the change reaches in a closed period, or in a year whose
    /// 1099s were due: tell the human (a filed 1099 may need correcting).
    pub warnings: Vec<String>,
}

/// Parameters for `books.set_entry_vendor`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetEntryVendorParams {
    /// Entry id (draft or posted).
    pub entry_id: u64,
    /// Vendor id or name; omit to clear the tag.
    pub vendor: Option<String>,
}

/// Parameters for `books.vendor_1099`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct Vendor1099Params {
    /// Calendar year of payment.
    pub year: i32,
    /// Override the reporting threshold, decimal string (default: $600
    /// through 2025, $2,000 from 2026).
    pub threshold: Option<String>,
    /// True: each line carries its payee's full TIN, read from the TIN
    /// vault (or a note saying why not). Default false (last four digits
    /// only).
    #[serde(default)]
    pub include_tins: bool,
    /// True: also hand the worksheet over as a CSV file (TINs included
    /// with include_tins), to save with its download command.
    #[serde(default)]
    pub csv: bool,
    /// The cash account codes payments are counted out of, e.g.
    /// `["1000"]`. Defaults to `books.cash_accounts`, as the CPA
    /// package's do: one or the other is required.
    #[serde(default)]
    pub cash_accounts: Vec<String>,
    /// The financing liability codes (what the members advanced, loans)
    /// payments are counted out of. Defaults to
    /// `books.financing_accounts`, as the CPA package's do.
    #[serde(default)]
    pub financing_accounts: Vec<String>,
    /// Count only the payments dated on or before this day of `year`
    /// (YYYY-MM-DD): the worksheet as of a day, the one a CPA package as
    /// of that day carries. Default: the whole year.
    pub through: Option<String>,
}

/// One vendor's year.
#[derive(Debug, Serialize, JsonSchema)]
pub struct Vendor1099LineOut {
    /// Vendor id.
    pub vendor_id: u64,
    /// Name.
    pub name: String,
    /// "1099-NEC" or "1099-MISC" when its decision at the year's end files
    /// one; absent otherwise.
    pub form_1099: Option<String>,
    /// Its 1099 decision at the year's end.
    pub decision_1099: Decision1099Out,
    /// Every decision in force during the year, in date order (more than
    /// one when its status changed in the year; none when never decided).
    pub decisions_in_year: Vec<Decision1099Out>,
    /// W-9 on file.
    pub tin_on_file: bool,
    /// What counts toward its 1099, decimal string: paid by check, ACH,
    /// transfer or cash (net of refunds and reversals) while it wasn't
    /// decided not reportable.
    pub paid: String,
    /// Paid by card or payment network: reported by the processor on Form
    /// 1099-K, never on the company's 1099, decimal string.
    pub excluded_card: String,
    /// Paid through a financing account (or a member's or a payment
    /// platform's) with no payment method recorded: counts or not once it
    /// is (books.set_entry_instrument), decimal string.
    pub undetermined: String,
    /// Paid while it was decided not reportable, decimal string.
    pub not_reportable: String,
    /// "reportable" (file a 1099), "under_threshold", "not_reportable"
    /// (its decision says why), or "undetermined" (a decision or a payment
    /// method is needed first).
    pub status: String,
    /// Some of its payments that could count were made while its decision
    /// was undetermined: ask the human and record one.
    pub decision_needed: bool,
    /// File a 1099: status reportable.
    pub reportable: bool,
    /// The vendor's party.
    pub party_id: Option<u64>,
    /// The TIN's last four digits: from the party's TIN facts
    /// (books.set_tin), else its W-9 facts.
    pub tin_last4: Option<String>,
    /// The full TIN, formatted (123-45-6789, 12-3456789): only with
    /// include_tins, and only when the vault holds it.
    pub tin: Option<String>,
    /// The TIN's type (ssn, ein, itin): only with include_tins.
    pub tin_type: Option<String>,
    /// With include_tins, why `tin` is missing (not in the vault, the vault
    /// locked, a record that doesn't match the books).
    pub tin_note: Option<String>,
}

/// One payment on the 1099 worksheet's lists.
#[derive(Debug, Serialize, JsonSchema)]
pub struct Payment1099Out {
    /// The entry.
    pub entry_id: u64,
    /// Its date, YYYY-MM-DD.
    pub date: String,
    /// The vendor it paid.
    pub vendor_id: u64,
    /// The vendor's name.
    pub vendor: String,
    /// The vendor's part the list is about, decimal string.
    pub amount: String,
    /// The payment method it is tagged with (label), if any.
    pub instrument: Option<String>,
}

/// Result of `books.vendor_1099`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct Vendor1099Out {
    /// Calendar year.
    pub year: i32,
    /// Threshold applied, decimal string.
    pub threshold: String,
    /// Vendors decided for a 1099 during the year, or with any payment in
    /// it.
    pub vendors: Vec<Vendor1099LineOut>,
    /// Vendors a 1099 is or may be due for without a W-9 on file
    /// ("(may be reportable)" after the undetermined): request one.
    pub missing_w9: Vec<String>,
    /// Payments by card or payment network, excluded (Form 1099-K's).
    pub excluded_card_payments: Vec<Payment1099Out>,
    /// Payments with no payment method recorded where it decides whether
    /// they count: record each with books.set_entry_instrument.
    pub undetermined_payments: Vec<Payment1099Out>,
    /// Money received from a vendor for something else (a sale, its
    /// payment of an invoice, a disposal, a loan), not netted against its
    /// payments, with the reversals of each.
    pub other_receipts: Vec<Payment1099Out>,
    /// Posted entries in the year that paid money out with no vendor tag.
    pub untagged_entries: Vec<u64>,
    /// Entries naming several parties whose shares don't resolve (each
    /// counted in full toward its vendor).
    pub unresolved: Vec<u64>,
    /// Caveats for the human and CPA.
    pub notes: Vec<String>,
    /// With include_tins: this output holds full TINs, for the CPA.
    pub warning: Option<String>,
    /// With csv: the worksheet as a CSV file, handed over.
    pub csv: Option<HandedFile>,
}

/// Parameters for `books.verify`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct VerifyParams {
    /// Also re-hash every stored receipt and document (default true).
    #[serde(default = "default_true")]
    pub blobs: bool,
}

fn default_true() -> bool {
    true
}

/// Result of `books.verify`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct VerifyOut {
    /// "ok", or "healed" when the database was rebuilt from the journal.
    pub status: String,
    /// Journal events whose hash chain verified.
    pub events: u64,
    /// SHA-256 of the last journal line: the verified head.
    pub head: String,
    /// Why the database was rebuilt, if it was.
    pub healed: Option<String>,
    /// Stored files re-hashed, when checked.
    pub blobs_verified: Option<u64>,
    /// The books' database file verified (a file for each schema, named for
    /// it: "wanderbooks.v13.db", or the older "wanderbooks.db" when it holds
    /// this version's schema).
    pub database: String,
    /// Other schemas' database files beside it: left as they are for the
    /// versions that use them, never deleted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub other_databases: Vec<String>,
    /// What was checked, in words.
    pub checked: Vec<String>,
}

/// Parameters for `books.show`. Give exactly one target.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ShowParams {
    /// An entry to open (lines, receipt, history).
    pub entry_id: Option<u64>,
    /// An account code to open its register.
    pub account: Option<String>,
    /// A report: profit_and_loss, balance_sheet, cash_flow,
    /// members_equity, trial_balance, general_ledger, tax_summary,
    /// vendor_1099, ar_aging, ap_aging, fixed_asset_register.
    pub report: Option<String>,
    /// A stored receipt or document (attachment id).
    pub attachment_id: Option<u64>,
    /// A fixed asset, by tag or id.
    pub asset: Option<String>,
    /// A party, by id or name (as `books.party`).
    pub party: Option<String>,
    /// A project, by id or name (as `books.project`).
    pub project: Option<String>,
    /// A reconciliation (id from `books.reconciliations`): its account's
    /// page, with the reconciliation and its statement.
    pub reconciliation: Option<u64>,
    /// Search the transactions list (description, account, vendor, amount).
    pub search: Option<String>,
    /// A page: overview, cash, profit, transactions, reports, accounts,
    /// assets, parties, projects, documents, company, journal.
    pub page: Option<String>,
    /// For an account or a report over a period: its first day (see
    /// [`show_start_doc`]).
    #[schemars(description = show_start_doc())]
    pub start: Option<String>,
    /// For an account or a report: its last day, or the day a report as
    /// of a day is as of (see [`show_end_doc`]).
    #[schemars(description = show_end_doc())]
    pub end: Option<String>,
    /// For a report: split into columns, "total" (default), "month",
    /// "quarter", or "year" (P&L, balance sheet, cash flow).
    pub columns: Option<String>,
    /// For a report: compare with "prior_period" or "prior_year" (P&L,
    /// balance sheet, cash flow, tax summary; not with a split).
    pub compare: Option<String>,
}

/// `books.show`'s `start`, in the one default's words
/// ([`ledger_core::calendar::START_DOC`]).
pub fn show_start_doc() -> String {
    format!(
        "For an account, or a report over a period (a balance sheet only when split or compared with the prior period). {START_DOC}"
    )
}

/// `books.show`'s `end`, in the one default's words
/// ([`ledger_core::calendar::END_DOC`]).
pub fn show_end_doc() -> String {
    format!(
        "For an account or a report: the last day, or the day a report as of a day is as of (vendor_1099: a day in its year). {END_DOC}"
    )
}

/// Result of `books.show`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ShowOut {
    /// What the viewer was asked to show.
    pub shown: String,
    /// Whether a viewer has these books open now.
    pub viewer_open: bool,
    /// Where this goes and what to tell the human.
    pub note: String,
}

/// Parameters for `books.chart_template`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ChartTemplateParams {
    /// "sole_proprietorship" (incl. single-member LLC), "partnership"
    /// (incl. multi-member LLC), "s_corporation", or "c_corporation".
    pub entity_type: String,
    /// Owners' names: one capital and distributions account each for a
    /// partnership (at least two). Ignored otherwise.
    #[serde(default)]
    pub owners: Vec<String>,
}

/// Result of `books.chart_template`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ChartTemplateOut {
    /// Chart CSV (code,name,type,parent_code,tax_category) for
    /// `books.import_chart_csv`, after review.
    pub csv: String,
    /// The settings that go with it, as changes for `books.set_setting`
    /// (after review, with who decided): tax form, owners, where period
    /// closes go.
    pub settings: Vec<TemplateSettingOut>,
    /// What to decide or know.
    pub notes: Vec<String>,
}

/// One setting a chart template goes with.
#[derive(Debug, Serialize, JsonSchema)]
pub struct TemplateSettingOut {
    /// The key.
    pub key: String,
    /// Its value, as `books.set_setting` takes it.
    pub value: serde_json::Value,
}

/// Parameters for `books.renumber_account`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RenumberAccountParams {
    /// The account's current code (or id).
    pub account: String,
    /// Its new code: non-empty and not in use.
    pub code: String,
}

/// Result of `books.renumber_account`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct RenumberAccountOut {
    /// The account's id (unchanged).
    pub id: u64,
    /// The code it had.
    pub previous_code: String,
    /// The code it has now.
    pub code: String,
    /// Settings that still name the old code (as books.set_setting keys),
    /// and conventions whose template or approval rule names it: change
    /// each (books.set_setting, books.set_convention).
    pub config_mentions: Vec<String>,
}

/// One member, as `books.settings` reports it.
#[derive(Debug, Serialize, JsonSchema)]
pub struct MemberSettingOut {
    /// Name.
    pub name: String,
    /// Share of income, as a decimal string ("0.5").
    pub share: String,
    /// Capital account code.
    pub capital_account: String,
    /// Distributions account code, if any.
    pub distribution_account: Option<String>,
    /// The member they file a joint individual return with, if any.
    pub files_jointly_with: Option<String>,
}

/// Result of `books.settings`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct SettingsOut {
    /// The company's time zone (IANA), or null: the computer's.
    pub timezone: Option<String>,
    /// Today, in that time zone.
    pub today: String,
    /// Whether approval rules block posting: "off" or "enforce".
    pub approval_rules: String,
    /// Every setting the books know: what it takes, its value (null:
    /// unset, its default applies), and who set it.
    pub keys: Vec<crate::server::SettingKeyOut>,
    /// `company.name`.
    pub company: String,
    /// `company.fiscal_year_start`, "MM-DD".
    pub fiscal_year_start: String,
    /// The return filed (`company.tax_form`, or from
    /// `company.tax_classification`), or null.
    pub tax_form: Option<String>,
    /// `books.accounting_basis`.
    pub basis: String,
    /// The receipt rule in force (it shows a few account codes of each
    /// kind, shortened; the two lists below hold them all).
    pub receipt_rule: String,
    /// Every account code whose entries always need a receipt, whatever
    /// the amount: the receipt policy's whole list, as written.
    pub receipt_required_accounts: Vec<String>,
    /// Every account code exempt from the receipt rule (an entry whose
    /// every line is in one needs none): the policy's whole list, as
    /// written.
    pub receipt_exempt_accounts: Vec<String>,
    /// `policy.parties`: "off", "suggest", or "require".
    pub parties: String,
    /// `company.members`.
    pub members: Vec<MemberSettingOut>,
    /// `books.cash_accounts`.
    pub cash_accounts: Vec<String>,
    /// `books.financing_accounts`.
    pub financing_accounts: Vec<String>,
    /// `books.investing_accounts`.
    pub investing_accounts: Vec<String>,
    /// `books.close_into`.
    pub close_into: Option<String>,
    /// `books.close_sweep`.
    pub close_sweep: Vec<String>,
    /// Settings that disagree with the books, each saying what to change
    /// (with books.set_setting, or in the chart). Empty: nothing to fix.
    pub problems: Vec<String>,
}

/// Parameters for `books.assign_role`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct AssignRoleParams {
    /// The role, from books.roles: receivables, revenue, payables,
    /// general_expense, retained_earnings, salary_expense,
    /// payroll_tax_expense, federal_withholding, state_withholding,
    /// social_security_payable, medicare_payable, other_deductions_payable.
    pub role: String,
    /// The account to fill it (code or id); null returns the role to its
    /// default code.
    pub account: Option<String>,
}

/// One control role and the account filling it.
#[derive(Debug, Serialize, JsonSchema)]
pub struct RoleOut {
    /// The role's key, for books.assign_role.
    pub role: String,
    /// What it is for, e.g. "accounts receivable".
    pub purpose: String,
    /// The account type it needs.
    pub account_type: String,
    /// The code it uses when no account is assigned.
    pub default_code: String,
    /// Whether an account is assigned (false: the default code is used).
    pub assigned: bool,
    /// The account filling it now: its code, or null when none exists yet
    /// (the first command that needs it creates it at the default code).
    pub account_code: Option<String>,
    /// That account's name.
    pub account_name: Option<String>,
}

/// Result of `books.roles`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct RolesOut {
    /// Every control role, in the engine's order.
    pub roles: Vec<RoleOut>,
}

/// Parameters for `books.set_tax_category`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetTaxCategoryParams {
    /// Account code (or id).
    pub account: String,
    /// Tax category key from `books.tax_categories`; null clears it.
    pub category: Option<String>,
    /// Percent deductible, overriding the category default (decimal
    /// string, 0 to 100), e.g. "100" for a fully deductible company party.
    pub deductible_percent: Option<String>,
}

/// Result of `books.set_tax_category`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct SetTaxCategoryOut {
    /// True when set.
    pub ok: bool,
}

/// One tax category.
#[derive(Debug, Serialize, JsonSchema)]
pub struct TaxCategoryOut {
    /// Stable key.
    pub key: String,
    /// Label.
    pub label: String,
    /// "income", "cost_of_goods", or "deduction".
    pub kind: String,
    /// Percent deductible by default.
    pub default_deductible_percent: u32,
    /// Guidance.
    pub note: String,
}

/// Result of `books.tax_categories`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct TaxCategoriesOut {
    /// The return the categories are filtered to, if configured.
    pub form: Option<String>,
    /// Categories.
    pub categories: Vec<TaxCategoryOut>,
}

/// Result of `books.chart`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ChartOut {
    /// Accounts in code order.
    pub accounts: Vec<ChartAccount>,
    /// Base currency code.
    pub base_currency: String,
}

// ---------------------------------------------------------------------------
// Reports
// ---------------------------------------------------------------------------

/// One trial-balance line.
#[derive(Debug, Serialize, JsonSchema)]
pub struct TrialBalanceLineOut {
    /// Account code.
    pub code: String,
    /// Account name.
    pub name: String,
    /// Debit balance, decimal string ("0.00" when none).
    pub debit: String,
    /// Credit balance, decimal string ("0.00" when none).
    pub credit: String,
}

/// Parameters for `books.trial_balance`.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct TrialBalanceParams {
    /// As of this day ([`AS_OF_DOC`]).
    #[serde(default)]
    #[schemars(description = AS_OF_DOC)]
    pub as_of: Option<String>,
}

/// Result of `books.trial_balance`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct TrialBalanceOut {
    /// The as-of date, YYYY-MM-DD.
    pub as_of: String,
    /// Lines in code order.
    pub lines: Vec<TrialBalanceLineOut>,
    /// Sum of the debit column, decimal string.
    pub total_debits: String,
    /// Sum of the credit column, decimal string.
    pub total_credits: String,
    /// True when debits equal credits exactly.
    pub balanced: bool,
}

/// Parameters for `books.profit_and_loss`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ProfitAndLossParams {
    /// Period start ([`START_DOC`]).
    #[schemars(description = START_DOC)]
    pub start: Option<String>,
    /// Period end ([`END_DOC`]).
    #[schemars(description = END_DOC)]
    pub end: Option<String>,
    /// Optional class (a project: name or id; see books.projects) to
    /// filter to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<String>,
}

/// Result of `books.profit_and_loss`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ProfitAndLossOut {
    /// Period start, YYYY-MM-DD.
    pub start: String,
    /// Period end, YYYY-MM-DD.
    pub end: String,
    /// Total revenue, decimal string.
    pub revenue: String,
    /// Total expenses, decimal string.
    pub expenses: String,
    /// Revenue minus expenses, decimal string. Positive is profit.
    pub net_income: String,
    /// Revenue accounts with movement in the period.
    pub revenue_accounts: Vec<StatementLineOut>,
    /// Expense accounts with movement in the period.
    pub expense_accounts: Vec<StatementLineOut>,
}

/// One account's line on a statement.
#[derive(Debug, Serialize, JsonSchema)]
pub struct StatementLineOut {
    /// Account code.
    pub code: String,
    /// Account name.
    pub name: String,
    /// Amount on the account's normal side, decimal string.
    pub amount: String,
}

/// Parameters for `books.cash_flow`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CashFlowParams {
    /// Period start ([`START_DOC`]).
    #[schemars(description = START_DOC)]
    pub start: Option<String>,
    /// Period end ([`END_DOC`]).
    #[schemars(description = END_DOC)]
    pub end: Option<String>,
    /// Asset account codes treated as cash (e.g. `["1000", "1010"]`): the
    /// statement explains the change in these accounts. Defaults to the
    /// books' cash accounts setting (books.settings shows it); one or the
    /// other is required.
    #[serde(default)]
    pub cash_accounts: Vec<String>,
    /// Liability account codes treated as financing (loans, and what
    /// members advanced: ASC 230-10-45-14(b)); equity is always financing.
    /// Defaults to the books' financing accounts setting, else empty
    /// (other liabilities are operating).
    #[serde(default)]
    pub financing_accounts: Vec<String>,
    /// Asset account codes treated as investing: loans the company makes,
    /// investments it buys (ASC 230-10-20). Registered fixed assets are
    /// investing unlisted. Defaults to the books' investing accounts
    /// setting.
    #[serde(default)]
    pub investing_accounts: Vec<String>,
}

/// One entry's cash in one kind of one section of the cash flow
/// statement.
#[derive(Debug, Serialize, JsonSchema)]
pub struct CashFlowLineOut {
    /// The entry that moved the cash.
    pub entry: u64,
    /// Its date, YYYY-MM-DD.
    pub date: String,
    /// "<description> (#N)", "Reversal of #N — <N's description>", or
    /// "Reinstatement of #A (reverses #B)".
    pub label: String,
    /// The kind it is listed under: an operating class ("Cash paid to
    /// suppliers and employees") or an investing or financing kind
    /// ("Contributions from members — <member>").
    pub kind: String,
    /// The cash, decimal string; positive is cash in.
    pub amount: String,
    /// Of `amount`, what a member or a lender paid or received for the
    /// company with no cash account moving (a constructive receipt or
    /// payment, D-42: an expense a member paid personally is borrowed from
    /// the member and spent), decimal string; absent when none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_behalf: Option<String>,
    /// Codes of this kind's accounts the entry moved.
    pub accounts: Vec<String>,
    /// The entry this one reverses, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reverses: Option<u64>,
    /// Operating lines: the ASC 230-10-45-25 class key
    /// (`from_customers`, `interest_and_dividends_received`,
    /// `other_receipts`, `to_suppliers_and_employees`, `interest_paid`,
    /// `income_taxes_paid`, `other_payments`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class: Option<String>,
}

/// A kind's total: how the printed statement and the viewer show a
/// section.
#[derive(Debug, Serialize, JsonSchema)]
pub struct CashFlowKindOut {
    /// The caption.
    pub kind: String,
    /// The operating class key, for an operating kind.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class: Option<String>,
    /// The kind's cash, decimal string.
    pub amount: String,
    /// Its entries.
    pub entries: Vec<u64>,
}

/// One reconciling item between net income and operating cash.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ReconcilingLineOut {
    /// `depreciation`, `investing_gain_loss`, `noncash_through`,
    /// `other_noncash`, or `operating_balance_change`.
    pub kind: String,
    /// "Depreciation", "Noncash: through 2500 Bank loan", "Increase
    /// in 1300 Prepaid Expenses", ...
    pub label: String,
    /// Added to net income, decimal string.
    pub amount: String,
    /// The account it is about, if one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    /// The entries behind it (an entry and its reversal in the period
    /// left out).
    pub entries: Vec<u64>,
}

/// Net income reconciled to net cash from operating activities (ASC
/// 230-10-45-30: a separate schedule with the direct method).
#[derive(Debug, Serialize, JsonSchema)]
pub struct OperatingReconciliationOut {
    /// The period's net income, decimal string.
    pub net_income: String,
    /// The reconciling items.
    pub lines: Vec<ReconcilingLineOut>,
    /// Net income plus the items: equal to cash_from_operations.
    pub net_cash_from_operating: String,
}

/// A noncash investing or financing activity (ASC 230-10-50-3).
#[derive(Debug, Serialize, JsonSchema)]
pub struct NoncashDisclosureOut {
    /// The entry.
    pub entry: u64,
    /// Its date, YYYY-MM-DD.
    pub date: String,
    /// Its label.
    pub label: String,
    /// The amount disclosed, decimal string.
    pub amount: String,
    /// The operating part no cash settled, debit-positive.
    pub operating: String,
    /// The investing part no cash settled, debit-positive.
    pub investing: String,
    /// The financing part no cash settled, debit-positive.
    pub financing: String,
    /// What was exchanged inside investing and financing (debt to equity,
    /// one asset for another).
    pub exchanged: String,
    /// The entry it reverses, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reverses: Option<u64>,
}

/// The footing check: always a zero difference (a nonzero one is an
/// engine error, never output).
#[derive(Debug, Serialize, JsonSchema)]
pub struct CashFlowFootingOut {
    /// Operating + investing + financing.
    pub sections_sum: String,
    /// The cash accounts' ending balance less their beginning balance.
    pub net_change: String,
    /// sections_sum − net_change: "0.00".
    pub difference: String,
}

/// An entry whose later settlement the books can't classify by
/// themselves, with what to change.
#[derive(Debug, Serialize, JsonSchema)]
pub struct CashFlowWarningOut {
    /// The entry.
    pub entry: u64,
    /// `asset_on_operating_payable` or `asset_sold_on_operating_receivable`.
    pub kind: String,
    /// What happened and the fix.
    pub message: String,
}

/// Result of `books.cash_flow`; `books.export` writes the same JSON.
#[derive(Debug, Serialize, JsonSchema)]
pub struct CashFlowOut {
    /// "direct" (ASC 230-10-45-25).
    pub method: String,
    /// Period start, YYYY-MM-DD.
    pub start: String,
    /// Period end, YYYY-MM-DD.
    pub end: String,
    /// Operating cash by ASC 230-10-45-25 class, (a) to (g).
    pub operating_by_class: Vec<CashFlowKindOut>,
    /// Every entry's operating cash, by class, then date.
    pub operating: Vec<CashFlowLineOut>,
    /// Net cash from operating activities, decimal string.
    pub cash_from_operations: String,
    /// Investing cash by kind (purchases and sales of property and
    /// equipment, loans made and collected).
    pub investing_by_kind: Vec<CashFlowKindOut>,
    /// Every entry's investing cash, by kind, then date.
    pub investing: Vec<CashFlowLineOut>,
    /// Net cash from investing activities, decimal string.
    pub cash_from_investing: String,
    /// Financing cash by kind (contributions and distributions per member,
    /// borrowing and repayments per lender).
    pub financing_by_kind: Vec<CashFlowKindOut>,
    /// Every entry's financing cash, by kind, then date.
    pub financing: Vec<CashFlowLineOut>,
    /// Net cash from financing activities, decimal string.
    pub cash_from_financing: String,
    /// Net change in cash, decimal string.
    pub net_change_in_cash: String,
    /// Cash at the start, decimal string.
    pub cash_beginning: String,
    /// Cash at the end, decimal string.
    pub cash_ending: String,
    /// Noncash investing and financing activities.
    pub noncash_disclosures: Vec<NoncashDisclosureOut>,
    /// Net income reconciled to operating cash.
    pub operating_reconciliation: OperatingReconciliationOut,
    /// The footing check.
    pub reconciles: CashFlowFootingOut,
    /// Entries whose later settlement the books can't classify.
    pub warnings: Vec<CashFlowWarningOut>,
}

/// Parameters for `books.import_bank_csv`. No other argument is taken: a
/// file's text never comes as one (D-46).
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImportBankCsvParams {
    /// The CSV (QBO bank upload layout: Date,Description,Amount, or
    /// Date,Description, money out, money in): its upload, made with
    /// books.begin_upload (purpose import) and `wanderbooks upload`. A
    /// file never passes through the agent (D-46).
    pub upload_id: String,
    /// Account code for the bank account (the offset side of each entry).
    pub bank_account_code: String,
    /// Account code for the default offset (e.g. an expense account).
    /// Used when no rule matches.
    pub default_offset_code: String,
    /// Optional categorization rules: { "pattern": "account_code" }. A
    /// pattern matches a description containing it (ignoring case); the
    /// longest matching pattern wins.
    #[serde(default)]
    pub rules: std::collections::HashMap<String, String>,
    /// How to read slash dates the file doesn't settle itself:
    /// "month_first" (mm/dd/yyyy, US banks) or "day_first" (dd/mm/yyyy).
    #[serde(default)]
    pub date_order: Option<String>,
    /// The CSV as filed (`books.file_document` category import_source, or
    /// any stored attachment): linked to the bank account imported into,
    /// so the import traces to its source. Never linked to the drafts: a
    /// bank export is not a receipt.
    pub source_attachment_id: Option<u64>,
}

/// One imported bank transaction (as a draft entry).
#[derive(Debug, Serialize, JsonSchema)]
pub struct ImportedTxnOut {
    /// Draft entry id.
    pub entry_id: u64,
    /// Transaction date, YYYY-MM-DD.
    pub date: String,
    /// Bank description.
    pub description: String,
    /// Signed amount, decimal string.
    pub amount: String,
    /// Account code used for the offset.
    pub offset_account: String,
}

/// Result of `books.import_bank_csv`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ImportBankCsvOut {
    /// Drafts created (one per transaction).
    pub imported: Vec<ImportedTxnOut>,
    /// Number of transactions imported.
    pub count: usize,
    /// The source attachment, if one was given.
    pub source_attachment_id: Option<u64>,
    /// True when this call linked it to the account (false when it already
    /// was, or none was given).
    pub source_linked: bool,
}

/// Parameters for `books.import_chart_csv`. No other argument is taken: a
/// file's text never comes as one (D-46).
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImportChartCsvParams {
    /// The CSV (code,name,type,parent_code,tax_category with a header
    /// row): its upload, made with books.begin_upload (purpose import) and
    /// `wanderbooks upload`. A file never passes through the agent (D-46).
    pub upload_id: String,
}

/// Result of `books.import_chart_csv`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ImportChartCsvOut {
    /// Account codes created, in order.
    pub created: Vec<String>,
    /// Number of accounts created.
    pub count: usize,
    /// What to tell the human: each revenue or expense account created
    /// without a tax category.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// Parameters for `books.balance_sheet`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct BalanceSheetParams {
    /// As of this day ([`AS_OF_DOC`]).
    #[schemars(description = AS_OF_DOC)]
    pub as_of: Option<String>,
}

/// Result of `books.balance_sheet`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct BalanceSheetOut {
    /// As of this date, YYYY-MM-DD.
    pub as_of: String,
    /// Total assets, decimal string.
    pub assets: String,
    /// Total liabilities, decimal string.
    pub liabilities: String,
    /// Total equity including current earnings, decimal string.
    pub equity: String,
    /// Net income to date, folded into equity, decimal string.
    pub current_earnings: String,
    /// True when assets equal liabilities plus equity exactly.
    pub balanced: bool,
    /// Asset accounts with a balance.
    pub asset_accounts: Vec<StatementLineOut>,
    /// Liability accounts with a balance.
    pub liability_accounts: Vec<StatementLineOut>,
    /// Equity accounts with a balance (current earnings is separate).
    pub equity_accounts: Vec<StatementLineOut>,
}

// ---------------------------------------------------------------------------
// Entries
// ---------------------------------------------------------------------------

/// Parameters for `books.draft_entry`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DraftEntryParams {
    /// Entry date, YYYY-MM-DD.
    pub date: String,
    /// Description of what happened.
    pub description: String,
    /// The vendor this entry pays (id or name), for 1099 tracking.
    pub vendor: Option<String>,
    /// How it was paid: a payment method's id or label (books.instruments),
    /// for 1099 tracking.
    pub instrument: Option<String>,
    /// The party this entry is with (id or name; see `books.parties`). A
    /// party with a vendor record also tags the entry's vendor; given with
    /// `vendor`, both must name the same vendor.
    pub party: Option<String>,
    /// Why the entry has no counterparty (a transfer between the company's
    /// own accounts, an accrual), when `policy.parties = "require"` and
    /// none applies. Not with `party`, `parties`, or `vendor`.
    pub no_party_reason: Option<String>,
    /// Several parties, each with their share (as `books.set_entry_party`
    /// `parties`), in place of `party`. Not with `party`, `vendor`, or
    /// `no_party_reason`. Whether the shares add up is checked once the
    /// entry has balanced lines (`books.post_entry` refuses until they do).
    pub parties: Option<Vec<PartyShareParam>>,
}

/// One party named on an entry, with their share. Give `amount` or
/// `percent`, or neither (derived): a sole party gets all of it; members
/// named together get what their capital (or distributions) lines on the
/// entry say, else their `[[company.members]]` share. Every party on an
/// entry gives the same kind of share.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct PartyShareParam {
    /// The party: id or name.
    pub party: String,
    /// Exactly this much of what the entry moved, a positive decimal
    /// string in cents ("30.00"). The amounts must sum to what the entry
    /// moved (its counterparty money, or its total when it moves none).
    pub amount: Option<String>,
    /// This percent of what the entry moved, a decimal string ("60",
    /// "33.333"); the percents must sum to exactly 100. Each part is
    /// rounded down to the cent and the leftover cents go to the first
    /// party listed.
    pub percent: Option<String>,
}

/// One party on an entry, with its share and what it resolves to.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct EntryPartyOut {
    /// Party id.
    pub party_id: u64,
    /// Its label (display name, else name).
    pub party: String,
    /// "amount", "percent", or "derived".
    pub share: String,
    /// The amount given, for an amount share, decimal string.
    pub amount: Option<String>,
    /// The percent given, for a percent share, decimal string.
    pub percent: Option<String>,
    /// Its exact part of what the entry moved, signed as a line on the
    /// party's own account would be (debit-positive): positive, the
    /// company paid it or gave it value; negative, received from it. With
    /// direction "neither", its part of the entry's movement of its own
    /// accounts (a bill is negative, owed to it; an invoice positive), else
    /// of the entry's total. A reversal's is exactly its original's
    /// negated. A member's entry is what moved their capital (a property
    /// contribution is negative, received, in kind). Decimal string. Null
    /// when the shares don't resolve (see `parties_error`).
    pub resolved: Option<String>,
    /// "paid", "received" or "neither": neither is a promise not yet
    /// settled (a bill, an invoice, a fee charged on its own card, a
    /// distribution declared and not yet paid) or an entry that moved
    /// nothing with anyone (a close), counted in no party total. Null
    /// exactly when `resolved` is.
    pub direction: Option<String>,
    /// What of `resolved` moved as money (cash, a card, the members'
    /// financing, a loan; on a member's contribution, what the K-1
    /// worksheet counts as cash; of a distribution, the money it paid):
    /// what the party's totals count on basis "cash". "0.00" for direction
    /// neither. Decimal string, null when `resolved` is.
    pub cash: Option<String>,
    /// What of `resolved` moved as property (contributed or distributed in
    /// kind): basis "all" adds it. resolved − cash − in_kind is a promise
    /// not yet paid, counted on neither basis. Decimal string, null when
    /// `resolved` is.
    pub in_kind: Option<String>,
}

/// Result of `books.draft_entry`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct DraftEntryOut {
    /// The draft's entry id. Use it with `books.add_line`.
    pub entry_id: u64,
    /// Always "draft" — drafts are not journaled until posted.
    pub status: String,
}

/// Parameters for `books.add_line`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct AddLineParams {
    /// Draft entry id from `books.draft_entry`.
    pub entry_id: u64,
    /// Account id, or account code (e.g. "1000").
    pub account: String,
    /// "debit" or "credit".
    pub side: String,
    /// Exact decimal amount as a string, e.g. "123.45". Never a float.
    /// For foreign-currency lines, this is ignored; use foreign_amount + fx_rate.
    pub amount: String,
    /// Optional foreign currency code (e.g. "EUR") for a foreign-currency line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub foreign_currency: Option<String>,
    /// Optional foreign amount, decimal string. Requires foreign_currency and fx_rate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub foreign_amount: Option<String>,
    /// Optional FX rate (foreign -> base), decimal string. Requires foreign_currency
    /// and foreign_amount. The base amount is foreign_amount * fx_rate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fx_rate: Option<String>,
    /// Optional class (a project: name or id, see books.projects) to tag
    /// this line with. Applied after the line is added; only valid on
    /// draft entries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<String>,
}

/// Result of `books.add_line`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct AddLineOut {
    /// True when the line was added.
    pub ok: bool,
    /// Something to know about the line: it is tagged to a finished or
    /// abandoned project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// One line of a posting preview.
#[derive(Debug, Serialize, JsonSchema)]
pub struct PreviewLine {
    /// Account id.
    pub account_id: u64,
    /// Account code.
    pub code: String,
    /// Account name.
    pub name: String,
    /// "debit" or "credit".
    pub side: String,
    /// Line amount, decimal string.
    pub amount: String,
    /// Account balance before posting, decimal string.
    pub balance_before: String,
    /// Account balance after posting, decimal string.
    pub balance_after: String,
}

/// One attached receipt in a preview.
#[derive(Debug, Serialize, JsonSchema)]
pub struct PreviewReceipt {
    /// Attachment id.
    pub attachment_id: u64,
    /// Original file name.
    pub file_name: String,
    /// SHA-256 of the bytes.
    pub sha256: String,
}

/// Parameters for `books.post_entry`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct PostEntryParams {
    /// Draft entry id.
    pub entry_id: u64,
    /// Pass true to post after reviewing the preview. Default false: returns
    /// the preview without posting.
    pub confirmed: Option<bool>,
    /// Optional safety rail: abort if the entry's total debits differ from
    /// this decimal string.
    pub expected_total: Option<String>,
}

/// Result of `books.post_entry`: either a preview or a posted confirmation.
#[derive(Debug, Serialize, JsonSchema)]
pub struct PostEntryOut {
    /// "preview" or "posted".
    pub status: String,
    /// The entry id.
    pub entry_id: u64,
    /// Entry date, YYYY-MM-DD.
    pub date: String,
    /// Entry description.
    pub description: String,
    /// Total debits, decimal string.
    pub total: String,
    /// Full posting lines with resulting balances.
    pub lines: Vec<PreviewLine>,
    /// Receipts currently attached.
    pub receipts: Vec<PreviewReceipt>,
    /// True when the receipt policy requires a receipt for this entry.
    pub receipt_required: bool,
    /// True when a receipt is required but none is attached. Posting with
    /// confirmed=true fails in this state.
    pub receipt_missing: bool,
    /// Posted entry ids that look like they split this entry to dodge the
    /// receipt threshold (same date, description, and debit accounts).
    /// Empty when no split is detected.
    pub split_detected: Vec<u64>,
    /// `policy.parties`: "off", "suggest", or "require".
    pub party_policy: String,
    /// The entry's counterparty, when it names exactly one.
    pub party_id: Option<u64>,
    /// Its name.
    pub party: Option<String>,
    /// Every party the entry names, each with its share resolved to the
    /// cent against the entry's lines (they sum to what it moved).
    pub parties: Vec<EntryPartyOut>,
    /// Why the parties' shares don't resolve against the lines (amounts
    /// that don't sum, a derived share for a non-member), when they don't.
    /// Posting with confirmed=true fails (party_shares) in this state.
    pub parties_error: Option<String>,
    /// Why it has none, when marked so.
    pub no_party_reason: Option<String>,
    /// True when the entry names no party and isn't marked as having none
    /// (always false when the policy is "off"). Under "require", posting
    /// with confirmed=true fails in this state.
    pub party_missing: bool,
    /// Likely counterparties, best first, when `party_missing`: tag one
    /// with `books.set_entry_party`, or add the party with
    /// `books.add_party` first.
    pub party_suggestions: Vec<PartySuggestionOut>,
    /// Approvals the entry still needs before it posts, when the books
    /// enforce approval rules (`policy.approval_rules = enforce`): each
    /// rule covering it and its approver. Empty otherwise. Record each
    /// with `books.approve_entry` once the approver says yes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub approvals_missing: Vec<ApprovalNeededOut>,
}

/// An approval an entry needs.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ApprovalNeededOut {
    /// The approval rule (a convention id).
    pub rule: String,
    /// Who approves.
    pub approver: String,
}

/// A likely counterparty for an entry.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PartySuggestionOut {
    /// The party's id.
    pub party_id: u64,
    /// Its name.
    pub name: String,
    /// Why: "named in the description", "named in a receipt", or "tagged
    /// on entry N with the same description".
    pub why: String,
}

/// Parameters for `books.reverse_entry`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReverseEntryParams {
    /// Id of the posted entry to reverse.
    pub entry_id: u64,
    /// Date of the reversing entry, YYYY-MM-DD.
    pub date: String,
    /// Description of the reversal.
    pub description: String,
    /// Pass true to post after reviewing the preview. Default false.
    pub confirmed: Option<bool>,
}

/// Result of `books.reverse_entry`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ReverseEntryOut {
    /// "preview" or "posted".
    pub status: String,
    /// The new reversing entry's id.
    pub reversal_id: u64,
    /// Id of the entry being reversed.
    pub reverses: u64,
    /// Reversed lines with resulting balances.
    pub lines: Vec<PreviewLine>,
}

// ---------------------------------------------------------------------------
// Receipts and documents
// ---------------------------------------------------------------------------

/// Parameters for `books.attach_receipt`. No other argument is taken: a
/// file's bytes never come as one (D-46).
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AttachReceiptParams {
    /// Entry id to attach to. Omit to file unlinked, then link later with
    /// `books.link_receipt`.
    pub entry_id: Option<u64>,
    /// Original file name, e.g. "receipt.png" (an upload's own, if not
    /// given).
    #[serde(default)]
    pub file_name: Option<String>,
    /// MIME type, e.g. "image/png" (an upload's own, if not given).
    #[serde(default)]
    pub mime_type: Option<String>,
    /// The file: its upload, made with books.begin_upload and `wanderbooks
    /// upload` (its id). A file never passes through the agent (D-46).
    pub upload_id: String,
    /// Optional description.
    pub description: Option<String>,
    /// More subjects to link it to in the same call (each journaled as its
    /// own `attachment.linked`). Every subject is checked before anything
    /// is filed: one bad subject files nothing.
    #[serde(default)]
    pub link_to: Vec<SubjectParam>,
}

/// Result of `books.attach_receipt`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct AttachReceiptOut {
    /// Attachment id.
    pub attachment_id: u64,
    /// SHA-256 of the stored bytes.
    pub sha256: String,
    /// True when identical bytes were already stored (deduplicated).
    pub deduplicated: bool,
    /// Everything it is linked to now (the entry it is filed on first).
    pub links: Vec<SubjectOut>,
}

/// Parameters for `books.file_document`. No other argument is taken: a
/// file's bytes never come as one (D-46).
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FileDocumentParams {
    /// One of: formation, legal, tax, accounting, statement, contract,
    /// photo, warranty, import_source, other.
    pub category: String,
    /// Original file name (an upload's own, if not given).
    #[serde(default)]
    pub file_name: Option<String>,
    /// MIME type (an upload's own, if not given).
    #[serde(default)]
    pub mime_type: Option<String>,
    /// The file: its upload, made with books.begin_upload and `wanderbooks
    /// upload` (its id). A file never passes through the agent (D-46).
    pub upload_id: String,
    /// Optional description.
    pub description: Option<String>,
    /// Subjects to link it to in the same call (each journaled as its own
    /// `attachment.linked`). Every subject is checked before anything is
    /// filed: one bad subject files nothing.
    #[serde(default)]
    pub link_to: Vec<SubjectParam>,
}

/// Parameters for `books.link_receipt`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct LinkReceiptParams {
    /// Attachment id from `books.attach_receipt`.
    pub attachment_id: u64,
    /// Entry id to link it to.
    pub entry_id: u64,
}

/// Result of `books.link_receipt`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct LinkReceiptOut {
    /// True when linked.
    pub ok: bool,
}

/// Parameters for `books.unlink_receipt`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct UnlinkReceiptParams {
    /// Attachment id of the linked receipt.
    pub attachment_id: u64,
    /// Why it is being unlinked, recorded in the journal (e.g. "replaced by
    /// the PDF render, attachment 9").
    pub reason: String,
}

/// Result of `books.unlink_receipt`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct UnlinkReceiptOut {
    /// True when unlinked.
    pub ok: bool,
    /// The entry it was unlinked from.
    pub entry_id: u64,
    /// Receipts still linked to that entry.
    pub remaining_receipts: u64,
}

/// Parameters for `books.withdraw_attachment`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct WithdrawAttachmentParams {
    /// Attachment id of an unlinked receipt or a filed document.
    pub attachment_id: u64,
    /// Why it is being withdrawn, recorded in the journal.
    pub reason: String,
}

/// Result of `books.withdraw_attachment`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct WithdrawAttachmentOut {
    /// True when withdrawn.
    pub ok: bool,
}

/// Parameters for `books.list_receipts`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListReceiptsParams {
    /// Filter to one entry. Omit for all.
    pub entry_id: Option<u64>,
    /// Include withdrawn attachments. Default false.
    #[serde(default)]
    pub include_withdrawn: Option<bool>,
}

/// One receipt or document.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ReceiptOut {
    /// Attachment id.
    pub attachment_id: u64,
    /// SHA-256 of the bytes.
    pub sha256: String,
    /// Linked entry id, if any.
    pub entry_id: Option<u64>,
    /// Category: receipts (books.attach_receipt), or a document's:
    /// formation, legal, tax, accounting, statement, contract, photo,
    /// warranty, import_source, other.
    pub category: String,
    /// Original file name.
    pub file_name: String,
    /// MIME type.
    pub mime_type: String,
    /// Size in bytes.
    pub size_bytes: u64,
    /// Description, if any.
    pub description: Option<String>,
    /// True when withdrawn (only listed with include_withdrawn).
    pub withdrawn: bool,
    /// Why it was withdrawn, if it was.
    pub withdrawn_reason: Option<String>,
    /// Everything it is linked to (the entry it is filed on first).
    pub links: Vec<SubjectOut>,
}

/// Result of `books.list_receipts`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ListReceiptsOut {
    /// Attachments.
    pub receipts: Vec<ReceiptOut>,
}

/// Parameters for `books.get_receipt`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetReceiptParams {
    /// Attachment id.
    pub attachment_id: u64,
    /// What to return: "meta" (default: what it is and what it's linked
    /// to), or "bytes" (also the file, handed over: the command that
    /// saves a copy, never its bytes; D-46).
    #[serde(default)]
    pub view: Option<String>,
}

/// Result of `books.get_receipt`: what it is and what it's linked to, and
/// with view `bytes`, the file handed over (read hash-verified).
#[derive(Debug, Serialize, JsonSchema)]
pub struct GetReceiptOut {
    /// Attachment id.
    pub attachment_id: u64,
    /// SHA-256 of the bytes.
    pub sha256: String,
    /// Linked entry id, if any.
    pub entry_id: Option<u64>,
    /// Category.
    pub category: String,
    /// Original file name.
    pub file_name: String,
    /// MIME type.
    pub mime_type: String,
    /// Size in bytes.
    pub size_bytes: u64,
    /// Description, if any.
    pub description: Option<String>,
    /// Everything it is linked to (the entry it is filed on first).
    pub links: Vec<SubjectOut>,
    /// With view `bytes`: the file, handed over: run its `download` to
    /// save a copy in a folder you name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<HandedFile>,
}

// ---------------------------------------------------------------------------
// Search, digest, export
// ---------------------------------------------------------------------------

/// Parameters for `books.get_entry`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetEntryParams {
    /// Entry id.
    pub entry_id: u64,
}

/// One line of a full entry.
#[derive(Debug, Serialize, JsonSchema)]
pub struct EntryLineOut {
    /// Account id.
    pub account_id: u64,
    /// Account code.
    pub code: String,
    /// Account name.
    pub name: String,
    /// "debit" or "credit".
    pub side: String,
    /// Amount, decimal string.
    pub amount: String,
    /// The project (class) id the line is tagged with, if any (books.add_line
    /// class, an invoice's or bill's line class).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class_id: Option<u32>,
    /// That project's name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
}

/// Result of `books.get_entry`: the full entry with lines and receipts.
#[derive(Debug, Serialize, JsonSchema)]
pub struct GetEntryOut {
    /// Entry id.
    pub entry_id: u64,
    /// Entry date, YYYY-MM-DD.
    pub date: String,
    /// Description.
    pub description: String,
    /// "draft" or "posted".
    pub status: String,
    /// Id of the reversed entry, for reversals.
    pub reversal_of: Option<u64>,
    /// Full lines.
    pub lines: Vec<EntryLineOut>,
    /// Total debits, decimal string. Null when they are past exact
    /// arithmetic (a draft an earlier version journaled; see
    /// `total_error`): such a draft can't be posted.
    pub total: Option<String>,
    /// Why the total can't be computed, when it can't.
    pub total_error: Option<String>,
    /// Attached receipts.
    pub receipts: Vec<PreviewReceipt>,
    /// The vendor the entry is tagged with, for 1099 tracking.
    pub vendor: Option<String>,
    /// The payment method it is tagged with (label), for 1099 tracking.
    pub instrument: Option<String>,
    /// That payment method's id.
    pub instrument_id: Option<u64>,
    /// The id of the party the entry is tagged with (its counterparty),
    /// when it names exactly one.
    pub party_id: Option<u64>,
    /// That party's label (display name, else name).
    pub party: Option<String>,
    /// Every party the entry names, with their shares resolved (as in the
    /// `books.post_entry` preview).
    pub parties: Vec<EntryPartyOut>,
    /// Why the shares don't resolve, when they don't.
    pub parties_error: Option<String>,
    /// Why it has no counterparty, when it is marked so (a reversal
    /// carries its original's mark).
    pub no_party_reason: Option<String>,
}

// ---------------------------------------------------------------------------
// Search, digest, export
// ---------------------------------------------------------------------------

/// Parameters for `books.search_entries`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchEntriesParams {
    /// Filter to an account id.
    pub account_id: Option<u64>,
    /// Filter to entries on or after this date, YYYY-MM-DD.
    pub date_from: Option<String>,
    /// Filter to entries on or before this date, YYYY-MM-DD.
    pub date_to: Option<String>,
    /// Free-text match on the description.
    pub text: Option<String>,
    /// Max rows. Default 50.
    pub limit: Option<u64>,
    /// Rows to skip. Default 0.
    pub offset: Option<u64>,
}

/// One entry in search results.
#[derive(Debug, Serialize, JsonSchema)]
pub struct EntrySummary {
    /// Entry id.
    pub entry_id: u64,
    /// Entry date, YYYY-MM-DD.
    pub date: String,
    /// Description.
    pub description: String,
    /// Total debits, decimal string.
    pub total: String,
    /// Id of the reversed entry, for reversals.
    pub reversal_of: Option<u64>,
    /// Number of receipts attached.
    pub receipt_count: u64,
}

/// Result of `books.search_entries`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct SearchEntriesOut {
    /// Matching entries, newest first.
    pub entries: Vec<EntrySummary>,
}

/// Parameters for `books.digest`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DigestParams {
    /// Include entries posted on or after this date, YYYY-MM-DD.
    pub since: String,
}

/// One entry in the digest.
#[derive(Debug, Serialize, JsonSchema)]
pub struct DigestEntry {
    /// Entry id.
    pub entry_id: u64,
    /// Entry date, YYYY-MM-DD.
    pub date: String,
    /// Description.
    pub description: String,
    /// Total debits, decimal string.
    pub total: String,
    /// Id of the reversed entry, for reversals.
    pub reversal_of: Option<u64>,
    /// Number of receipts attached.
    pub receipt_count: u64,
    /// True when the receipt policy required a receipt.
    pub receipt_required: bool,
    /// True when required but missing (should not happen for posted
    /// entries; the server rejects such posts).
    pub receipt_missing: bool,
}

/// Net movement of one account over the digest period.
#[derive(Debug, Serialize, JsonSchema)]
pub struct DigestTotal {
    /// Account code.
    pub code: String,
    /// Account name.
    pub name: String,
    /// Net movement in the account's normal side, decimal string.
    pub movement: String,
}

/// Result of `books.digest`: what changed, for human review.
#[derive(Debug, Serialize, JsonSchema)]
pub struct DigestOut {
    /// The since date, YYYY-MM-DD.
    pub since: String,
    /// Entries posted in the period, newest first.
    pub entries: Vec<DigestEntry>,
    /// Net movement per account over the period.
    pub totals_by_account: Vec<DigestTotal>,
    /// Rules set by law (like the 1099 threshold) that wanderbooks may no
    /// longer know as of today. For each: research the current value from
    /// the official source or ask the human (owner, CPA), then tell the
    /// human, who can report it upstream so the rule is updated for
    /// everyone.
    pub stale_rules: Vec<StaleRule>,
    /// Appends cut off mid-line (a crash or power loss) that were set aside
    /// when the books were opened, since `since`. Nothing acknowledged was
    /// lost; tell the human, and if they had asked for a change moments before,
    /// check it is there and redo it if not.
    pub journal_repairs: Vec<JournalRepair>,
    /// Settings that need changing: a setting that disagrees with the
    /// chart. Fix each with the human (books.set_setting), then check
    /// books.settings.
    pub settings_problems: Vec<String>,
    /// Cash and card accounts (`books.cash_accounts`, and liabilities
    /// with a recorded statement) with activity not reconciled through the
    /// end of last month: record each statement and reconcile it.
    pub unreconciled_accounts: Vec<UnreconciledAccountOut>,
    /// Revenue and expense accounts (not headers) with no tax category,
    /// active or not, "CODE Name": the list the viewer's Needs attention
    /// shows. Propose a category for each from books.tax_categories and
    /// set it with books.set_tax_category once the human agrees.
    pub uncategorized_accounts: Vec<String>,
    /// Receivables and payables lines that are no document's, and
    /// documents needing attention (overpaid, not in the ledger), as of
    /// today (books.control_check): look at each with the human.
    pub control_problems: Vec<String>,
}

/// An account due for reconciliation (`books.digest`).
#[derive(Debug, Serialize, JsonSchema)]
pub struct UnreconciledAccountOut {
    /// Account code.
    pub code: String,
    /// Account name.
    pub name: String,
    /// The last statement date it is reconciled through, if any.
    pub reconciled_through: Option<String>,
    /// The date it should be reconciled through: the end of last month.
    pub due_through: String,
    /// The calls that complete it.
    pub next_step: String,
}

/// A torn journal tail set aside on open.
#[derive(Debug, Serialize, JsonSchema)]
pub struct JournalRepair {
    /// When it was set aside (UTC timestamp).
    pub at: String,
    /// How many bytes were cut off.
    pub bytes: u64,
    /// When the bytes were set aside, as the stamp they were saved under
    /// (a human can find them by it).
    pub set_aside: String,
}

/// A dated rule due for review (see `ledger_core::rules`).
#[derive(Debug, Serialize, JsonSchema)]
pub struct StaleRule {
    /// Plain name, e.g. "1099 reporting threshold".
    pub name: String,
    /// The last year wanderbooks knows it for.
    pub known_through: i32,
    /// The official source of its next value: the statute or
    /// publication, with its address.
    pub source: String,
    /// Why it's due, and where the next value comes from.
    pub why: String,
}

/// Parameters for `books.export`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ExportParams {
    /// "csv", "json", "xlsx", "pdf", or "iif": the file is handed over,
    /// never its bytes (D-46). PDF covers every report but general_ledger and
    /// general_journal; IIF (plain ASCII text) is only valid with report
    /// "general_journal" (alias "journal").
    pub format: String,
    /// "general_journal" (alias "journal"), "general_ledger",
    /// "trial_balance", "profit_and_loss", "balance_sheet", "cash_flow",
    /// "members_equity", "tax_summary", or "fixed_asset_register".
    pub report: String,
    /// Period start for the period reports (see [`export_start_doc`]).
    #[schemars(description = export_start_doc())]
    pub start: Option<String>,
    /// Period end for the period reports (see [`export_end_doc`]).
    #[schemars(description = export_end_doc())]
    pub end: Option<String>,
    /// As-of day for the reports as of a day (see [`export_as_of_doc`]).
    #[schemars(description = export_as_of_doc())]
    pub as_of: Option<String>,
    /// Cash account codes for cash_flow, and for iif its BANK accounts.
    /// Defaults to `books.cash_accounts`; one or the other is required
    /// for cash_flow.
    #[serde(default)]
    pub cash_accounts: Vec<String>,
    /// Liability account codes treated as financing for cash_flow.
    /// Defaults to `books.financing_accounts`, else empty.
    #[serde(default)]
    pub financing_accounts: Vec<String>,
    /// Asset account codes treated as investing for cash_flow: loans the
    /// company makes, investments it buys (ASC 230-10-20). Defaults to
    /// `books.investing_accounts`.
    #[serde(default)]
    pub investing_accounts: Vec<String>,
}

/// `books.export`'s `start`, in the one default's words
/// ([`ledger_core::calendar::START_DOC`]).
pub fn export_start_doc() -> String {
    format!(
        "For the period reports: profit_and_loss, cash_flow, members_equity, tax_summary, general_ledger, general_journal (csv, json, xlsx; iif takes as_of instead). {START_DOC}"
    )
}

/// `books.export`'s `end`, in the one default's words
/// ([`ledger_core::calendar::END_DOC`]).
pub fn export_end_doc() -> String {
    format!("For the period reports, as for start. {END_DOC}")
}

/// `books.export`'s `as_of`, in the one default's words
/// ([`ledger_core::calendar::AS_OF_DOC`]).
pub fn export_as_of_doc() -> String {
    format!(
        "For balance_sheet, trial_balance, fixed_asset_register and iif (every posted entry dated on or before it). {AS_OF_DOC}"
    )
}

/// Result of `books.export`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ExportOut {
    /// The requested format.
    pub format: String,
    /// The requested report.
    pub report: String,
    /// The exported file, handed over: run its `download` to save it in a
    /// folder you name. Never its bytes (D-46).
    pub file: HandedFile,
    /// IIF: what the file writes differently from the books to fit
    /// QuickBooks Desktop (plain ASCII; account names at most 31
    /// characters a level, numbers 7, memos 4,095; no two names or numbers
    /// alike; a subaccount of another type than its parent's at the top
    /// level): each account's name, number and place a line, and each thing written
    /// differently in the memos a line naming the entries it is in
    /// (`"—" is written "--" in the memos of entries 3, 5, 9`). No full
    /// TIN. Tell the human. Absent when there are none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

// ---------------------------------------------------------------------------
// Fixed assets
// ---------------------------------------------------------------------------

/// Parameters for `books.register_asset`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RegisterAssetParams {
    /// What it is, e.g. "2023 Sprinter van".
    pub description: String,
    /// The day it was placed in service (ready and available for its use),
    /// YYYY-MM-DD: depreciation starts here. Required (or its alias
    /// purchase_date).
    #[serde(default)]
    pub in_service_date: Option<String>,
    /// Deprecated alias of in_service_date (it always meant the in-service
    /// date), accepted for one release. Giving both with different dates is
    /// refused.
    #[serde(default)]
    pub purchase_date: Option<String>,
    /// Capitalized cost, decimal string, e.g. "60000.00".
    pub cost: String,
    /// Expected residual value, decimal string. Defaults to "0".
    pub salvage_value: Option<String>,
    /// Useful life in months, at least 1.
    pub useful_life_months: u32,
    /// Cost account (asset), id or code.
    pub cost_account: String,
    /// Accumulated depreciation account (asset, contra), id or code.
    pub accum_depr_account: String,
    /// Depreciation expense account, id or code.
    pub expense_account: String,
    /// The asset tag for its label (letters, digits, - _ . /; at most 32).
    /// Omit to generate the next one ("FA-0001", from
    /// `books.asset_tag_prefix` or the company's initials).
    pub tag: Option<String>,
    /// The manufacturer's serial number, if known.
    pub serial: Option<String>,
    /// The posted entry that put it on the books (its purchase, or a
    /// member's contribution of it): it must debit the cost account and be
    /// dated on or before the in-service date. Its date is the asset's
    /// acquired date, which puts it on the register.
    pub acquisition_entry: Option<u64>,
    /// How the company got it (see books.set_asset_tax_basis): purchase,
    /// carryover_723, or personal_use_conversion. Omit to record it later.
    #[serde(default)]
    pub basis_rule: Option<String>,
    /// Deprecated alias of basis_rule, with the old names (cost,
    /// carryover, personal_use_conversion), for one release.
    #[serde(default)]
    pub tax_basis_rule: Option<String>,
    /// The contributing member's adjusted basis at contribution, for
    /// carryover_723 and personal_use_conversion.
    #[serde(default)]
    pub contributor_adjusted_basis: Option<String>,
    /// Deprecated alias of contributor_adjusted_basis, for one release.
    #[serde(default)]
    pub contributor_basis: Option<String>,
    /// Fair market value at contribution, for carryover_723 and
    /// personal_use_conversion (required: ask the human).
    #[serde(default)]
    pub fmv_at_contribution: Option<String>,
    /// The §704(c) method chosen for a contribution: traditional,
    /// traditional_with_curative, or remedial.
    #[serde(default)]
    pub sec704c_method: Option<String>,
    /// Who gave the basis facts, when, and on what (e.g. the entry that
    /// records the value).
    #[serde(default)]
    pub basis_source: Option<BasisSourceIn>,
}

/// Who gave an asset's basis facts, and when: the shape every tool that
/// records a decision takes (books.set_setting's source).
#[derive(Debug, Deserialize, JsonSchema)]
pub struct BasisSourceIn {
    /// Who gave them: a person's name, as the human says.
    pub decided_by: String,
    /// When, YYYY-MM-DD. Default: today.
    #[serde(default)]
    pub decided_on: Option<String>,
    /// The document or entry they rest on, if any ("entry 14").
    #[serde(default)]
    pub citation: Option<String>,
    /// Anything else worth keeping with them.
    #[serde(default)]
    pub note: Option<String>,
}

/// Parameters for `books.correct_asset`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CorrectAssetParams {
    /// The asset: its id or tag.
    pub asset: String,
    /// Corrected cost, decimal string. Omit to keep it.
    pub cost: Option<String>,
    /// Corrected salvage value, decimal string. Omit to keep it.
    pub salvage_value: Option<String>,
    /// Corrected useful life in months. Omit to keep it.
    pub useful_life_months: Option<u32>,
    /// Corrected in-service date, YYYY-MM-DD (never before its acquisition
    /// entry's date). Omit to keep it.
    #[serde(default)]
    pub in_service_date: Option<String>,
    /// Deprecated alias of in_service_date, for one release.
    #[serde(default)]
    pub in_service: Option<String>,
    /// Why (required): what was wrong.
    pub reason: String,
}

/// Result of `books.correct_asset`: the asset's figures now.
#[derive(Debug, Serialize, JsonSchema)]
pub struct AssetSummaryOne {
    /// Asset id.
    pub asset_id: u64,
    /// Its tag.
    pub tag: Option<String>,
    /// Cost, decimal string.
    pub cost: String,
    /// Salvage value, decimal string.
    pub salvage_value: String,
    /// Useful life in months.
    pub useful_life_months: u32,
    /// In-service date, YYYY-MM-DD.
    pub in_service_date: String,
    /// Acquired date (its acquisition entry's), YYYY-MM-DD.
    pub acquired_date: String,
    /// Its basis facts and tax figures (a purchase's depreciable basis
    /// follows the corrected cost; a contribution's figures stay as
    /// contributed).
    pub basis: Option<AssetBasisOut>,
    /// Anything the call did beyond the correction (an alias used).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

/// Parameters for `books.set_asset_tax_basis`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetAssetTaxBasisParams {
    /// The asset: its id or tag.
    pub asset: String,
    /// How the company got it: "purchase" (bought: basis is cost, IRC
    /// §1012), "carryover_723" (a member contributed business property:
    /// the member's adjusted basis carries over, IRC §723), or
    /// "personal_use_conversion" (a member contributed personal-use
    /// property: §723 basis as for carryover, depreciable basis the lesser
    /// of fair market value and that basis, Treas. Reg. §1.168(i)-4(b)).
    /// The old names cost and carryover are aliases for one release.
    pub rule: String,
    /// The contributing member's adjusted basis at contribution, decimal
    /// string: cost plus improvements, less depreciation allowed or
    /// allowable (IRC §1016(a)). Required for a contribution; omit for a
    /// purchase.
    #[serde(default)]
    pub contributor_adjusted_basis: Option<String>,
    /// Deprecated alias of contributor_adjusted_basis, for one release.
    #[serde(default)]
    pub contributor_basis: Option<String>,
    /// Fair market value at contribution, decimal string. Required for a
    /// contribution: ask the human (the books' amount in the contribution
    /// entry, when that is what it was worth).
    #[serde(default)]
    pub fmv_at_contribution: Option<String>,
    /// The §704(c) method chosen for a contribution: traditional,
    /// traditional_with_curative, or remedial (Treas. Reg. §1.704-3(b)-(d)).
    #[serde(default)]
    pub sec704c_method: Option<String>,
    /// Who gave the facts, when, and on what (e.g. the contribution
    /// entry).
    #[serde(default)]
    pub basis_source: Option<BasisSourceIn>,
}

/// Where each basis figure's rule comes from.
#[derive(Debug, Serialize, JsonSchema)]
pub struct BasisSourcesOut {
    /// The §723 basis's: "IRC §723" (contributions).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tax_basis_723: Option<String>,
    /// The depreciable basis's: "IRC §1012" (purchase), "IRC §723"
    /// (carryover), or "Treas. Reg. §1.168(i)-4(b)" (personal use).
    pub depreciable_basis: String,
    /// The built-in gain's: "Treas. Reg. §1.704-3(a)(3)(ii)"
    /// (contributions).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub built_in_gain_704c: Option<String>,
}

/// Who gave an asset's basis facts, as tools show it.
#[derive(Debug, Serialize, JsonSchema)]
pub struct BasisSourceOut {
    /// Who gave them.
    pub decided_by: String,
    /// When, YYYY-MM-DD.
    pub decided_on: String,
    /// What they rest on, if anything.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub citation: Option<String>,
    /// Anything kept with them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// An asset's basis facts and tax figures, as tools report them. The
/// figures are at contribution (or purchase): the CPA adjusts them for tax
/// depreciation and, for the §704(c) gain or loss, as it is reduced after.
#[derive(Debug, Serialize, JsonSchema)]
pub struct AssetBasisOut {
    /// How the company got it: purchase, carryover_723, or
    /// personal_use_conversion.
    pub rule: String,
    /// The contributing member's adjusted basis at contribution, decimal
    /// string (a contribution).
    pub contributor_adjusted_basis: Option<String>,
    /// Fair market value at contribution, decimal string (a contribution):
    /// recorded, or the book cost for a record made before the value was
    /// a fact (fmv_recorded false).
    pub fmv_at_contribution: Option<String>,
    /// Whether fmv_at_contribution was recorded as a fact.
    pub fmv_recorded: bool,
    /// The company's basis in contributed property at contribution (IRC
    /// §723: the member's adjusted basis), decimal string.
    pub tax_basis_723: Option<String>,
    /// The basis tax depreciation starts from, decimal string.
    pub depreciable_basis: String,
    /// Fair market value less the §723 basis at contribution, IRC §704(c)
    /// (positive a built-in gain, negative a built-in loss), decimal string.
    /// What remains later is the CPA's (Treas. Reg. §1.704-3(a)(3)(ii)).
    pub built_in_gain_704c: Option<String>,
    /// For a built-in loss: it counts only for the contributing member(s);
    /// for the others the basis is treated as the value (IRC
    /// §704(c)(1)(C)).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sec704c_1c_note: Option<String>,
    /// The §704(c) method chosen: traditional, traditional_with_curative,
    /// or remedial; null until chosen.
    pub sec704c_method: Option<String>,
    /// Who gave the facts, when, and on what.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<BasisSourceOut>,
    /// The authority for each figure.
    pub sources: BasisSourcesOut,
}

/// Result of `books.set_asset_tax_basis`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct SetAssetTaxBasisOut {
    /// The asset.
    pub asset_id: u64,
    /// Its basis facts and figures now.
    pub basis: AssetBasisOut,
    /// Anything to know (an alias used).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

/// Result of `books.register_asset`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct RegisterAssetOut {
    /// The new asset's id.
    pub asset_id: u64,
    /// Its tag (given or generated).
    pub tag: String,
    /// In-service date, YYYY-MM-DD.
    pub in_service_date: String,
    /// Acquired date, YYYY-MM-DD: the acquisition entry's, or the
    /// in-service date with none linked.
    pub acquired_date: String,
    /// Its basis facts and figures, if given.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub basis: Option<AssetBasisOut>,
    /// Anything to know (an alias used).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

/// Parameters for `books.label_asset`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct LabelAssetParams {
    /// The asset: its id or its current tag.
    pub asset: String,
    /// A new tag (never another asset's, now or before). Omit to keep it.
    pub tag: Option<String>,
    /// The manufacturer's serial number. Omit to keep it.
    pub serial: Option<String>,
    /// True to clear the serial number.
    pub clear_serial: Option<bool>,
    /// A new description (what it is, as the register and label print it).
    /// Omit to keep it.
    pub description: Option<String>,
    /// Link the posted entry that put it on the books (purchase or
    /// contribution; it must debit the asset's cost account and not be
    /// reversed). Use after correcting that entry, to link the replacement.
    pub acquisition_entry: Option<u64>,
}

/// Result of `books.label_asset`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct LabelAssetOut {
    /// Asset id.
    pub asset_id: u64,
    /// Its tag now.
    pub tag: Option<String>,
    /// Its serial number now.
    pub serial: Option<String>,
    /// The entry that acquired it, if linked.
    pub acquisition_entry: Option<u64>,
    /// Its description now.
    pub description: String,
}

/// Parameters for `books.asset_labels`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct AssetLabelsParams {
    /// The assets to print (ids or tags), in order. Omit for every active
    /// asset with a tag.
    pub assets: Option<Vec<String>>,
    /// Label positions to skip on the first sheet, to reuse a partly used
    /// one (0 to 29). Default 0.
    pub skip: Option<u32>,
}

/// Result of `books.asset_labels`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct AssetLabelsOut {
    /// The label sheet, a PDF, handed over: run its `download` to save it.
    /// Avery 5160/8160 (US letter, 30 labels of 1" x 2-5/8"); print at
    /// 100% scale, not "fit to page".
    pub file: HandedFile,
    /// The tags printed, in order.
    pub tags: Vec<String>,
}

/// One month of a depreciation schedule.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ScheduleLineOut {
    /// Calendar month, "YYYY-MM".
    pub period: String,
    /// Depreciation for the month, decimal string.
    pub amount: String,
    /// Accumulated depreciation through this month, decimal string.
    pub accumulated: String,
    /// Cost less the schedule's accumulated through this month, decimal
    /// string: the schedule's figure; what the books carry is
    /// books.list_assets' book_value.
    pub book_value: String,
}

/// Parameters for `books.asset_schedule`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct AssetScheduleParams {
    /// Asset id (or pass `tag`).
    pub asset_id: Option<u64>,
    /// The asset's tag, instead of its id.
    pub tag: Option<String>,
    /// Run the schedule through this date, YYYY-MM-DD. Defaults to the end
    /// of the asset's depreciable life.
    pub through: Option<String>,
}

/// Result of `books.asset_schedule`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct AssetScheduleOut {
    /// Asset id.
    pub asset_id: u64,
    /// Asset description.
    pub description: String,
    /// Cost, decimal string.
    pub cost: String,
    /// Salvage value, decimal string.
    pub salvage_value: String,
    /// Useful life in months.
    pub useful_life_months: u32,
    /// "active" or "disposed".
    pub status: String,
    /// Month-by-month schedule.
    pub schedule: Vec<ScheduleLineOut>,
}

/// One asset in `books.list_assets`, as of the date asked.
#[derive(Debug, Serialize, JsonSchema)]
pub struct AssetSummary {
    /// Asset id.
    pub asset_id: u64,
    /// Its tag, if it has one.
    pub tag: Option<String>,
    /// Its serial number, if recorded.
    pub serial: Option<String>,
    /// The entry that acquired it, if linked.
    pub acquisition_entry: Option<u64>,
    /// Its basis facts and tax figures, if recorded.
    pub basis: Option<AssetBasisOut>,
    /// Asset description.
    pub description: String,
    /// In-service date, YYYY-MM-DD: depreciation starts here.
    pub in_service_date: String,
    /// Acquired date, YYYY-MM-DD: on the books from here.
    pub acquired_date: String,
    /// Cost, decimal string.
    pub cost: String,
    /// Useful life in months.
    pub useful_life_months: u32,
    /// "active" or "disposed", on as_of: "disposed" once a disposal dated
    /// on or before it is recorded (a later one doesn't show).
    pub status: String,
    /// Whether the books hold it on as_of: acquired by then and not yet
    /// disposed.
    pub held: bool,
    /// Accumulated depreciation the books carry on as_of (runs posted, less
    /// reversals), decimal string; "0.00" once disposed; null before it
    /// was acquired.
    pub accumulated_posted: Option<String>,
    /// Accumulated depreciation the schedule says is due through as_of,
    /// decimal string; "0.00" once disposed; null before it was acquired.
    pub accumulated_scheduled_through: Option<String>,
    /// Scheduled less posted: run books.run_depreciation for it. Decimal
    /// string; null before it was acquired.
    pub unposted_depreciation: Option<String>,
    /// What the books carry: cost less accumulated_posted ("0.00" once
    /// disposed), decimal string; null before it was acquired.
    pub book_value: Option<String>,
    /// Why the figures are null for an asset acquired by as_of: one an
    /// earlier version took whose schedule the books can't compute. It
    /// lists, and the rest of the register with it.
    pub schedule_error: Option<String>,
}

/// Result of `books.list_assets`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ListAssetsOut {
    /// The date the figures are as of, YYYY-MM-DD.
    pub as_of: String,
    /// All registered assets.
    pub assets: Vec<AssetSummary>,
}

/// Parameters for `books.list_assets`.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct ListAssetsParams {
    /// The date the figures are as of, YYYY-MM-DD. Default: today (the
    /// company's time zone).
    #[serde(default)]
    pub as_of: Option<String>,
}

/// Parameters for `books.run_depreciation`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RunDepreciationParams {
    /// Calendar month, "YYYY-MM".
    pub period: String,
    /// Asset id. Omit to run depreciation for every active asset.
    pub asset_id: Option<u64>,
}

/// One asset's depreciation run.
#[derive(Debug, Serialize, JsonSchema)]
pub struct DepreciationRunOut {
    /// Asset id.
    pub asset_id: u64,
    /// Asset description.
    pub description: String,
    /// Calendar month posted.
    pub period: String,
    /// Amount posted, decimal string: the month's own depreciation plus
    /// any earlier month caught up.
    pub amount: String,
    /// What the run caught up from earlier months (skipped, or reversed),
    /// decimal string: the amount less the month's own depreciation, when
    /// more ("0.00" otherwise).
    pub caught_up: String,
    /// The posted entry's id.
    pub entry_id: u64,
    /// True when this period was already posted (idempotent no-op).
    pub already_posted: bool,
}

/// Result of `books.run_depreciation`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct RunDepreciationOut {
    /// One result per asset run.
    pub runs: Vec<DepreciationRunOut>,
    /// Assets with nothing posted: a run-all's assets not yet in service,
    /// and any asset with nothing due (the month rounds to nothing, a later
    /// run caught it up or its run was reversed after it ended, its life is
    /// over and fully posted, or the books carry more than its schedule),
    /// each with why.
    pub skipped: Vec<SkippedDepreciation>,
}

/// An asset skipped by a run-all depreciation.
#[derive(Debug, Serialize, JsonSchema)]
pub struct SkippedDepreciation {
    /// Asset id.
    pub asset_id: u64,
    /// Asset description.
    pub description: String,
    /// Why it was skipped.
    pub reason: String,
}

/// Parameters for `books.dispose_asset`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DisposeAssetParams {
    /// Asset id (or pass `tag`).
    pub asset_id: Option<u64>,
    /// The asset's tag, instead of its id.
    pub tag: Option<String>,
    /// Disposal date, YYYY-MM-DD.
    pub date: String,
    /// Cash (or receivable) received, decimal string.
    pub proceeds: String,
    /// Account receiving the proceeds (asset, e.g. bank), id or code.
    pub proceeds_account: String,
    /// Gain/loss account (revenue or expense), id or code.
    pub gain_loss_account: String,
    /// Attachment ids (filed via books.file_document, e.g. the bill of
    /// sale) to link to the disposal entry on posting. Required when the
    /// receipt policy applies.
    pub receipt_attachment_ids: Option<Vec<u64>>,
    /// The buyer (party id or name), named on the disposal entry. Under
    /// `policy.parties = "require"` a sale with proceeds needs one; a
    /// retirement with no proceeds and no buyer is marked as having no
    /// counterparty.
    #[serde(default)]
    pub buyer: Option<String>,
    /// Confirm the disposal. Without confirmed=true returns a preview and
    /// posts nothing.
    pub confirmed: Option<bool>,
}

/// Result of `books.dispose_asset`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct DisposeAssetOut {
    /// "preview" or "posted".
    pub status: String,
    /// Asset id.
    pub asset_id: u64,
    /// The disposal entry's id (0 in preview).
    pub entry_id: u64,
    /// Accumulated depreciation being cleared, decimal string.
    pub accumulated: String,
    /// Book value at disposal, decimal string.
    pub book_value: String,
    /// Proceeds, decimal string.
    pub proceeds: String,
    /// Gain (positive) or loss (negative), decimal string.
    pub gain_loss: String,
    /// Depreciation auto-posted (its own entry) through the disposal day,
    /// decimal string ("0.00" when the runs already reach it).
    pub final_depreciation: String,
    /// Depreciation the runs posted past the disposal day (the disposal
    /// month's run is dated its end), taken back by a credit to
    /// depreciation expense in the disposal entry, decimal string.
    pub excess_depreciation: String,
    /// Depreciation still due through the disposal day when the disposal
    /// month already has a run (an earlier run reversed and not run
    /// again), debited to depreciation expense in the disposal entry,
    /// decimal string.
    pub catch_up_depreciation: String,
    /// The disposal entry's lines with resulting balances.
    pub lines: Vec<PreviewLine>,
    /// True when posting requires a receipt (bill of sale) first.
    pub receipt_required: bool,
}

/// Structured machine-readable tool error for receipt policy rejections.
///
/// Tool errors surface as strings in MCP; this serializes to JSON so
/// clients can match on `code` instead of parsing prose. The human-readable
/// `message` is still included for logs and chat.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReceiptRequiredError {
    /// Stable machine-readable code: always `"receipt_required"`.
    pub code: String,
    /// Human-readable explanation with the remedy.
    pub message: String,
    /// The entry (or asset) that needs the receipt. Set for entries.
    pub entry_id: Option<u64>,
    /// The asset being disposed. Set for disposals.
    pub asset_id: Option<u64>,
    /// Total debits moved, decimal string.
    pub debits: String,
    /// The policy that triggered, e.g. "strictly greater than $25.00".
    pub policy: String,
    /// Tool to call to satisfy the requirement.
    pub remedy_tool: String,
    /// Concrete next step for the agent.
    pub remedy_hint: String,
    /// When the requirement was triggered by anti-splitting detection: the
    /// ids of the posted entries this entry appears to split.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub split_group: Vec<u64>,
}

impl ReceiptRequiredError {
    /// Serialize to the string form returned as the MCP tool error.
    pub fn to_tool_error(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| self.message.clone())
    }
}

// ---------------------------------------------------------------------------
// Periods
// ---------------------------------------------------------------------------

/// Parameters for `books.create_period`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreatePeriodParams {
    /// Human name, e.g. "2026-10" or "Q4 2026".
    pub name: String,
    /// First date in the period, YYYY-MM-DD (inclusive).
    pub start: String,
    /// Last date in the period, YYYY-MM-DD (inclusive).
    pub end: String,
}

/// Output of `books.create_period`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreatePeriodOut {
    /// The new period's id.
    pub period_id: u64,
}

/// Parameters for `books.close_period`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ClosePeriodParams {
    /// The period to close.
    pub period_id: u64,
    /// Without confirmed=true, returns a preview (closing lines) and closes
    /// nothing. With confirmed=true, posts the closing entry and closes.
    #[serde(default)]
    pub confirmed: bool,
}

/// Output of `books.close_period`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ClosePeriodOut {
    /// "preview" or "closed".
    pub status: String,
    /// The period id.
    pub period_id: u64,
    /// The closing entry's id (if any was needed).
    pub closing_entry: Option<u64>,
    /// Human-readable summary of the closing lines.
    pub lines: Vec<String>,
    /// For a close to the members: how it splits among them (the same in
    /// the preview and once confirmed).
    #[serde(default)]
    pub members: Option<MembersCloseOut>,
}

/// How a close splits the net among the members: cumulatively, each
/// member's share of everything closed to these members and shares since
/// `since_close`, to the minor unit, less what earlier closes gave them.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MembersCloseOut {
    /// The close the count starts at: the first members close with these
    /// members and shares (1 is the books' first close of all).
    pub since_close: u64,
    /// Each member, in the settings' order.
    pub members: Vec<MemberCloseOut>,
}

/// One member's part of a close (`books.close_period`).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MemberCloseOut {
    /// The member's name, as in the settings.
    pub name: String,
    /// Their capital account's code.
    pub capital_account: String,
    /// Their share, as a decimal fraction ("0.50").
    pub share: String,
    /// What this close credits their capital (negative: debits), a true-up
    /// of earlier rounding included. It can run against the period's
    /// result by a cent.
    pub this_close: String,
    /// What the count's closes have credited them in all, this one
    /// included.
    pub running: String,
    /// Their exact share of everything the count has closed (share ×
    /// total, unrounded): `running` is within one minor unit of it once a
    /// close of the count has posted under the cumulative rule (before
    /// that, a close that posted nothing still carries the rounding of
    /// closes made under an earlier rule).
    pub exact: String,
}

/// One period in `books.list_periods`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PeriodOut {
    /// Period id.
    pub id: u64,
    /// Human name.
    pub name: String,
    /// Start date, YYYY-MM-DD.
    pub start: String,
    /// End date, YYYY-MM-DD.
    pub end: String,
    /// "open" or "closed".
    pub status: String,
    /// Closing entry id, if closed with one.
    pub closing_entry: Option<u64>,
    /// For a period closed to the members, the rounding rule it was closed
    /// under: "members_cumulative_since:N" (counted from close N, the rule
    /// since P3), "members_run" (2026-10-13 to P3), "members_per_close"
    /// (2026-10-13) or "members_last_absorbs" (before 2026-10-13). Absent
    /// (null) otherwise.
    #[serde(default)]
    pub close_rule: Option<String>,
}

/// Output of `books.list_periods`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListPeriodsOut {
    /// All periods, in creation order.
    pub periods: Vec<PeriodOut>,
}

// ---------------------------------------------------------------------------
// Bank reconciliation
// ---------------------------------------------------------------------------

/// Parameters for `books.mark_cleared`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MarkClearedParams {
    /// Entry id containing the line.
    pub entry_id: u64,
    /// Zero-based index of the line within the entry.
    pub line_index: usize,
    /// True to mark cleared, false to unmark.
    pub cleared: bool,
}

/// Output of `books.mark_cleared`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MarkClearedOut {
    /// Always true on success.
    pub ok: bool,
}

/// Parameters for `books.reconcile`. Either a recorded statement
/// (`statement_attachment`), or `account`, `statement_date`, and
/// `statement_balance` for a check against any balance.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReconcileParams {
    /// Account id or code (e.g. "1000") to reconcile. With
    /// statement_attachment, optional (it must be the statement's).
    pub account: Option<String>,
    /// Statement date, YYYY-MM-DD. With statement_attachment, optional
    /// (it must be the statement's period end).
    pub statement_date: Option<String>,
    /// Statement balance, decimal string. With statement_attachment,
    /// optional (it must be the statement's closing balance).
    pub statement_balance: Option<String>,
    /// The attachment holding the statement, recorded with
    /// `books.record_statement`: reconcile to what it says.
    pub statement_attachment: Option<u64>,
    /// True: record the reconciliation (needs statement_attachment and a
    /// difference of exactly zero). Without it, a preview that records
    /// nothing.
    pub confirmed: Option<bool>,
}

/// One line in the reconciliation report.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReconcileLineOut {
    /// Entry id.
    pub entry_id: u64,
    /// Line index within the entry.
    pub line_index: usize,
    /// Entry date, YYYY-MM-DD.
    pub date: String,
    /// Entry description.
    pub description: String,
    /// "debit" or "credit".
    pub side: String,
    /// Amount, decimal string.
    pub amount: String,
}

/// Output of `books.reconcile`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReconcileOut {
    /// "preview" (nothing recorded) or "reconciled" (recorded).
    pub status: String,
    /// Account code.
    pub account_code: String,
    /// The statement date reconciled through, YYYY-MM-DD.
    pub statement_date: String,
    /// The statement's attachment, when reconciling to a recorded one.
    pub statement_attachment: Option<u64>,
    /// Book balance (all posted lines), decimal string.
    pub book_balance: String,
    /// Cleared balance (cleared lines on/before statement date), decimal string.
    pub cleared_balance: String,
    /// Statement balance, decimal string.
    pub statement_balance: String,
    /// cleared_balance - statement_balance. Zero means reconciled.
    pub difference: String,
    /// Uncleared lines on/before the statement date.
    pub uncleared_lines: Vec<ReconcileLineOut>,
    /// Cleared lines on/before the statement date: what a confirmed
    /// reconciliation records.
    pub cleared_lines: Vec<ReconcileLineOut>,
    /// Why confirmed=true would be refused now (empty: it would record).
    pub problems: Vec<String>,
    /// The recorded reconciliation (status "reconciled" only).
    pub reconciliation: Option<ReconciliationOut>,
}

// ---------------------------------------------------------------------------
// A/R and A/P
// ---------------------------------------------------------------------------

/// Parameters for `books.add_customer` / `books.add_vendor`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AddRecordParams {
    /// Customer or vendor name. The record links to the party of that name
    /// (ignoring case and surrounding space, not archived) that has no such
    /// record yet, else to a new party.
    pub name: String,
}

/// Output of `books.add_customer` / `books.add_vendor`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AddRecordOut {
    /// The new customer's or vendor's id (for `books.create_invoice`,
    /// `books.create_bill`, `books.set_vendor_1099`).
    pub id: u64,
    /// The party the record belongs to (`books.party`).
    pub party_id: u64,
}

/// One line on an invoice or bill.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DocumentLineParam {
    /// Description of the goods/services.
    pub description: String,
    /// Line amount, decimal string.
    pub amount: String,
    /// The one project (name or id, as books.projects lists them) the
    /// line's revenue (an invoice's) or cost (a bill's) belongs to, or
    /// none. Several projects are several lines, each with its own amount,
    /// never a percentage; a shared cost names none (the allocation
    /// convention's own entries reach the projects). The posted
    /// entry's revenue or expense line for it carries the tag; the
    /// receivable or payable line never does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<String>,
    /// The account the line's revenue, expense or asset goes to (a code or
    /// an id): an invoice line a revenue account or a liability holding no
    /// role (deferred revenue), a bill line an expense account or an asset
    /// that is not cash and holds no role. Omitted: the kind's own role
    /// account (revenue for an invoice, general expenses for a bill). A
    /// bill's line must name one in books that have declared the journal
    /// feature documents_v2 (A-10).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    /// How many units the line bills, with unit_price, when the line is
    /// priced rather than summed: quantity × unit_price must equal amount
    /// exactly. Both or neither.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quantity: Option<String>,
    /// The price of one unit, with quantity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit_price: Option<String>,
}

/// A document's party as the page prints it: the customer's or vendor's
/// name, address and email at issue (A-13), and its contact block (D-55).
/// Every part omitted is the party record's.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct BillToParam {
    /// Its name at issue.
    pub name: Option<String>,
    /// Its address at issue (the general one), one line per line.
    pub address: Option<String>,
    /// Its email at issue (the general one).
    pub email: Option<String>,
    /// Its telephone number at issue.
    pub phone: Option<String>,
    /// The person there to write to at issue.
    pub contact: Option<String>,
    /// Where its bills go: the bill-to address when set, else `address`.
    pub billing_address: Option<String>,
    /// Where goods go: the page prints a Ship-to line for it.
    pub shipping_address: Option<String>,
    /// Where its bills are emailed: the bill-to email when set, else
    /// `email`.
    pub billing_email: Option<String>,
}

/// Parameters for `books.create_invoice`: it previews without
/// `confirmed`, and posts with it (A-1).
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct CreateInvoiceParams {
    /// The customer: a customer id, its party's id, or its name. A name
    /// that two customers answer to is refused.
    pub customer: Option<String>,
    /// The old spelling of `customer` (accepted until P5): a customer id.
    #[serde(default)]
    pub party_id: Option<u64>,
    /// The invoice date, YYYY-MM-DD.
    pub date: String,
    /// Its payment terms ("Net 30", "Net 15 EOM", "2/10 net 30", "Due on
    /// receipt", "Due 2026-11-15"). The due date is derived from them.
    pub terms: Option<String>,
    /// Its due date, YYYY-MM-DD: used as given, and it overrides `terms`.
    pub due_date: Option<String>,
    /// Line items.
    pub lines: Vec<DocumentLineParam>,
    /// The customer's own reference (their purchase order).
    pub customer_ref: Option<String>,
    /// A memo printed on the invoice.
    pub memo: Option<String>,
    /// The customer as the page prints it (omitted: the party's name,
    /// address and email at issue).
    pub bill_to: Option<BillToParam>,
    /// True: post it. Absent or false: preview it and post nothing.
    pub confirmed: Option<bool>,
}

/// Parameters for `books.create_bill`: it previews without `confirmed`,
/// and posts with it.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct CreateBillParams {
    /// The vendor: a vendor id, its party's id, or its name. A name that
    /// two vendors answer to is refused.
    pub vendor: Option<String>,
    /// The old spelling of `vendor` (accepted until P5): a vendor id.
    #[serde(default)]
    pub party_id: Option<u64>,
    /// The bill's date, YYYY-MM-DD.
    pub date: String,
    /// The day the bill was received, when it differs from its date.
    pub received_on: Option<String>,
    /// The vendor's own invoice number: required in books that have
    /// declared documents_v2 (a second bill with the same vendor and
    /// number is refused unless duplicate_reason says why).
    pub vendor_ref: Option<String>,
    /// Its payment terms; the due date is derived from them.
    pub terms: Option<String>,
    /// Its due date, YYYY-MM-DD: used as given, and it overrides `terms`.
    pub due_date: Option<String>,
    /// Line items.
    pub lines: Vec<DocumentLineParam>,
    /// A memo about the bill.
    pub memo: Option<String>,
    /// The vendor's invoice itself: an upload made with books.begin_upload
    /// and `wanderbooks upload` (D-46). Required in books that have
    /// declared documents_v2: it is filed on the bill's entry as its
    /// evidence.
    pub upload_id: Option<String>,
    /// The vendor's invoice, stored already (books.attachments): filed
    /// like an upload.
    pub existing_attachment_id: Option<u64>,
    /// Why a vendor invoice number that already names a standing bill is
    /// entered again anyway.
    pub duplicate_reason: Option<String>,
    /// True: post it. Absent or false: preview it and post nothing.
    pub confirmed: Option<bool>,
}

/// One invoice or bill a payment applies to (plan P4b S6; B-42).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PaymentApplicationParam {
    /// The invoice (books.receive_payment) or bill (books.pay_bill), by id.
    #[serde(default)]
    pub invoice: Option<u64>,
    /// The bill (books.pay_bill), by id.
    #[serde(default)]
    pub bill: Option<u64>,
    /// The cash paid against it, decimal string, positive.
    pub amount: String,
    /// An early-payment discount taken off it, decimal string.
    #[serde(default)]
    pub discount: Option<String>,
    /// Why a discount outside the document's terms was taken (a late
    /// discount the vendor honours): one line, recorded with the payment.
    #[serde(default)]
    pub discount_reason: Option<String>,
}

/// A processor's fee withheld from a customer payment (plan P4b S6; A-4).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FeeParam {
    /// The fee, decimal string, positive.
    pub amount: String,
    /// The expense account it goes to (id or code); omitted: the
    /// payment-processing-fees role's account.
    #[serde(default)]
    pub account: Option<String>,
    /// The processor, when the books know it as a vendor (id or name).
    #[serde(default)]
    pub vendor: Option<String>,
}

/// Parameters for `books.receive_payment` / `books.pay_bill`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PaymentParams {
    /// Invoice id (receive_payment) or bill id (pay_bill); the older name
    /// of `invoice` / `bill`.
    #[serde(default)]
    pub document_id: Option<u64>,
    /// The invoice, by id (books.receive_payment).
    #[serde(default)]
    pub invoice: Option<u64>,
    /// The bill, by id (books.pay_bill).
    #[serde(default)]
    pub bill: Option<u64>,
    /// Payment date, YYYY-MM-DD.
    pub date: String,
    /// Payment amount, decimal string; with `invoice` or `bill` only. The
    /// applications' amounts are the cash; a discount is beside it.
    #[serde(default)]
    pub amount: Option<String>,
    /// Cash account id or code; the older name of `deposit_to` /
    /// `paid_from`.
    #[serde(default)]
    pub cash_account: Option<String>,
    /// The account the cash reaches (receive_payment): a bank or card that
    /// holds no role.
    #[serde(default)]
    pub deposit_to: Option<String>,
    /// The account the cash leaves (pay_bill): a bank, card or payable
    /// that holds no role.
    #[serde(default)]
    pub paid_from: Option<String>,
    /// How the money moved: a payment method's id or label
    /// (`books.instruments`), tagged on the payment's entry in the same
    /// call (D-40: a card payment is left out of 1099-NEC).
    #[serde(default)]
    pub instrument: Option<String>,
    /// The processor's or bank's own reference for this payment (a charge
    /// id, a trace or check number): one line, at most 200 characters,
    /// carried in the entry's description (D-45).
    #[serde(default)]
    pub reference: Option<String>,
    /// Several invoices (receive_payment) or bills (pay_bill) settled by
    /// one payment, one party.
    #[serde(default)]
    pub applications: Vec<PaymentApplicationParam>,
    /// The processor's fee withheld from a customer payment (never on a
    /// bill): receivables are credited the gross, the fee goes to expense,
    /// the deposit account gets the net.
    #[serde(default)]
    pub fee: Option<FeeParam>,
    /// A check or note marked "paid in full" for a disputed amount: adds a
    /// warning under UCC §3-311 and records nothing else.
    #[serde(default)]
    pub paid_in_full_marking: bool,
}

/// One document a payment settled, after it.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PaidDocumentOut {
    /// "invoice:7" or "bill:12".
    pub r#ref: String,
    /// Its number, when it has one ("INV-0007").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number: Option<String>,
    /// What it still owes after the payment, decimal string.
    pub outstanding: String,
    /// Its status after it: open, partial, paid, written_off, void,
    /// overpaid, or unattached.
    pub status: String,
}

/// Output of `books.receive_payment` / `books.pay_bill`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PaymentOut {
    /// The payment journal entry's id.
    pub entry_id: u64,
    /// Every document it settled, in application order.
    pub documents: Vec<PaidDocumentOut>,
    /// What reached (receive_payment) or left (pay_bill) the account: the
    /// cash less a processor's fee, decimal string.
    pub deposited: String,
    /// What the single document still owes (the older shape; present when
    /// one document was settled).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remaining: Option<String>,
    /// The single document's status (the older shape).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// What to tell the human (a "paid in full" marking's UCC §3-311
    /// warning).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// Parameters for `books.documents`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DocumentsParams {
    /// "invoices" or "bills".
    pub kind: String,
    /// Which: open (default: anything still owed, partly paid and
    /// overdue included), overdue, paid, void, written_off, overpaid, or
    /// all.
    #[serde(default)]
    pub status: Option<String>,
    /// Only this customer's or vendor's (party or customer/vendor name, or
    /// id).
    #[serde(default)]
    pub party: Option<String>,
    /// Only those due on or before this day, YYYY-MM-DD.
    #[serde(default)]
    pub due_by: Option<String>,
    /// As of this day ([`AS_OF_DOC`]).
    #[serde(default)]
    #[schemars(description = AS_OF_DOC)]
    pub as_of: Option<String>,
}

/// One document in a list.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DocumentRowOut {
    /// "invoice:7" or "bill:12": what books.document takes.
    pub r#ref: String,
    /// The customer or vendor.
    pub party: String,
    /// Its date.
    pub date: String,
    /// When payment is due.
    pub due_date: String,
    /// The sum of its lines.
    pub total: String,
    /// What it still owes as of the day.
    pub outstanding: String,
    /// open, partial, paid, written_off, void, overpaid, or unattached.
    pub status: String,
    /// Days past due while something is owed; 0 otherwise.
    pub overdue_days: i64,
    /// The day it last went out, when a send is recorded (P4b S5);
    /// absent for a document never sent and for every bill.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_sent: Option<String>,
}

/// Output of `books.documents`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DocumentsOut {
    /// The day.
    pub as_of: String,
    /// "invoices" or "bills".
    pub kind: String,
    /// Due date first, then number.
    pub documents: Vec<DocumentRowOut>,
    /// What the listed documents still owe, together.
    pub outstanding_total: String,
}

/// Parameters for `books.document`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DocumentParams {
    /// "invoice:7" or "bill:12".
    pub r#ref: String,
    /// As of this day ([`AS_OF_DOC`]).
    #[serde(default)]
    #[schemars(description = AS_OF_DOC)]
    pub as_of: Option<String>,
    /// For an invoice: also hand over its PDF (as of the day, re-created
    /// from the books) to download.
    #[serde(default)]
    pub pdf: bool,
}

/// One line of a document.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DocumentLineOut {
    /// What it is for.
    pub description: String,
    /// Its amount.
    pub amount: String,
    /// The project it belongs to, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
}

/// One event in a document's life: a ledger line tagged with it.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DocumentEventOut {
    /// The entry's date.
    pub date: String,
    /// The entry.
    pub entry_id: u64,
    /// What it is: origin (the document itself), payment, discount,
    /// write_off, adopted, or role_move.
    pub kind: String,
    /// What it did to what is owed (plus raises it).
    pub amount: String,
    /// The entry reverses another (this line undoes an earlier one).
    pub reversal: bool,
    /// Whether its entry still stands as of the day: a payment that was
    /// reversed doesn't; its reversal does, and the two net to nothing.
    pub stands: bool,
}

/// Output of `books.document`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DocumentOut {
    /// "invoice:7" or "bill:12".
    pub r#ref: String,
    /// The customer or vendor.
    pub party: String,
    /// Its date.
    pub date: String,
    /// When payment is due.
    pub due_date: String,
    /// The entry that recorded it.
    pub entry_id: u64,
    /// Its lines.
    pub lines: Vec<DocumentLineOut>,
    /// The sum of its lines.
    pub total: String,
    /// Cash paid against it that stands.
    pub paid: String,
    /// Discounts taken that stand.
    pub discounted: String,
    /// Amounts written off that stand.
    pub written_off: String,
    /// What it still owes as of the day.
    pub outstanding: String,
    /// Its status as of the day.
    pub status: String,
    /// Days past due while something is owed; 0 otherwise.
    pub overdue_days: i64,
    /// Every ledger line tagged with it, in posting order.
    pub timeline: Vec<DocumentEventOut>,
    /// Every recorded send, in order (P4b S5): who, how, to whom, when,
    /// original or reminder, and the channel's own reference. Empty for a
    /// bill and for an invoice never sent.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sends: Vec<SentOut>,
    /// The invoice's PDF, handed over (with `pdf`): run its download.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pdf: Option<HandedFile>,
}

/// Parameters for `books.void_document`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VoidDocumentParams {
    /// "invoice" or "bill".
    pub kind: String,
    /// The invoice's or bill's id.
    pub id: u64,
    /// The void's date, YYYY-MM-DD: on or after the document's, in an
    /// open period.
    pub date: String,
    /// Why it should never have existed (a duplicate, the wrong
    /// customer): one line, at most 200 characters.
    pub reason: String,
}

/// Output of `books.void_document`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct VoidDocumentOut {
    /// The reversal entry that voids it.
    pub entry_id: u64,
    /// The document's status now ("void").
    pub status: String,
}

/// One line on a control account that is no document's.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UnattachedLineOut {
    /// The entry.
    pub entry_id: u64,
    /// The line's index in it.
    pub line: usize,
    /// The entry's date.
    pub date: String,
    /// Its amount as the control account counts it (plus raises it).
    pub amount: String,
}

/// A document the control check lists for attention.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DocAttentionOut {
    /// "invoice" or "bill".
    pub kind: String,
    /// Its id.
    pub id: u64,
    /// Its status as of the day (overpaid, unattached, ...).
    pub status: String,
    /// Its total.
    pub total: String,
    /// What it owes as of the day.
    pub outstanding: String,
}

/// One control account's tie-out.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ControlCheckRowOut {
    /// "receivables" or "payables".
    pub role: String,
    /// The account's code.
    pub code: String,
    /// The account's name.
    pub name: String,
    /// Its general-ledger balance on its normal side.
    pub balance: String,
    /// What its documents still owe on it.
    pub documents_total: String,
    /// Its lines that are no document's (hand or older lines).
    pub unattached: Vec<UnattachedLineOut>,
    /// balance - documents_total - the unattached lines: always "0.00".
    pub difference: String,
    /// The role's documents needing attention.
    pub attention: Vec<DocAttentionOut>,
}

/// Output of `books.control_check`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ControlCheckOut {
    /// The day.
    pub as_of: String,
    /// True when every control account equals its documents (no
    /// unattached line, nothing needing attention, no difference).
    pub agrees: bool,
    /// One row per control account.
    pub accounts: Vec<ControlCheckRowOut>,
}

/// Parameters for `books.ar_aging` / `books.ap_aging`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AgingParams {
    /// As of this day ([`AS_OF_DOC`]).
    #[serde(default)]
    #[schemars(description = AS_OF_DOC)]
    pub as_of: Option<String>,
    /// Also one row per document, with its number, dates, terms and
    /// bucket.
    #[serde(default)]
    pub detail: bool,
}

/// One row of an aging report.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AgingRowOut {
    /// Customer or vendor name.
    pub name: String,
    /// Not yet due, decimal string.
    pub current: String,
    /// 1-30 days past due, decimal string.
    pub days_30: String,
    /// 31-60 days past due, decimal string.
    pub days_60: String,
    /// 61-90 days past due, decimal string.
    pub days_90: String,
    /// Over 90 days past due, decimal string.
    pub days_90_plus: String,
    /// Total outstanding, decimal string.
    pub total: String,
}

/// One document in an aging's detail.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AgingDocumentOut {
    /// "invoice:7" or "bill:12".
    pub r#ref: String,
    /// Its number, when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number: Option<String>,
    /// The customer or vendor.
    pub party: String,
    /// Its own date.
    pub date: String,
    /// When payment is due.
    pub due_date: String,
    /// Its terms, as they print ("2/10 net 30"), when it has them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terms: Option<String>,
    /// The sum of its lines.
    pub total: String,
    /// Cash paid that stands.
    pub paid: String,
    /// Discounts taken that stand.
    pub discounted: String,
    /// What it still owes as of the day.
    pub outstanding: String,
    /// current, days_30, days_60, days_90 or days_90_plus.
    pub bucket: String,
    /// Days past its due date while something is owed; 0 otherwise.
    pub days_overdue: i64,
}

/// The aging's account against its documents (the control check, B-23,
/// B-38 INVARIANT 1).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AgingControlOut {
    /// "1200 Accounts Receivable" or "2000 Accounts Payable".
    pub account: String,
    /// Its ledger balance as of the day.
    pub balance: String,
    /// What its documents owe on it, together.
    pub documents_total: String,
    /// The lines on it that are no document's.
    pub unattached: Vec<UnattachedLineOut>,
    /// balance − documents_total − unattached: always 0.00.
    pub difference: String,
}

/// Output of `books.ar_aging` / `books.ap_aging`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AgingOut {
    /// The day it is as of, YYYY-MM-DD.
    pub as_of: String,
    /// Rows, one per party with an outstanding balance.
    pub rows: Vec<AgingRowOut>,
    /// One row per open document, when `detail` was asked for.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub documents: Vec<AgingDocumentOut>,
    /// The account's tie-out.
    pub control: AgingControlOut,
}

/// Parameters for `books.bills_due`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BillsDueParams {
    /// Show bills due on or before this day, YYYY-MM-DD (omitted: today).
    #[serde(default)]
    pub through: Option<String>,
    /// As of this day for what each bill owes ([`AS_OF_DOC`]).
    #[serde(default)]
    #[schemars(description = AS_OF_DOC)]
    pub as_of: Option<String>,
}

/// One bill to pay, with its discount window when its terms give one.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DueBillOut {
    /// "bill:12".
    pub r#ref: String,
    /// The vendor.
    pub vendor: String,
    /// The vendor's own invoice number, when recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vendor_ref: Option<String>,
    /// When payment is due.
    pub due_date: String,
    /// What it still owes as of the day.
    pub outstanding: String,
    /// The last day its early-payment discount is open, when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discount_by: Option<String>,
    /// The most its terms allow off, decimal string.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_discount: Option<String>,
    /// What to pay to take the discount: the outstanding less the
    /// discount, decimal string.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pay_by_discount_date_amount: Option<String>,
}

/// One date's running total of what falls due through it.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DueDateTotalOut {
    /// The due date.
    pub date: String,
    /// What falls due on or before it, together.
    pub amount: String,
}

/// Output of `books.bills_due`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BillsDueOut {
    /// The day it is as of.
    pub as_of: String,
    /// The last day a bill is included for.
    pub through: String,
    /// Bills due by `through`, due date first.
    pub bills: Vec<DueBillOut>,
    /// The running total, one row per due date.
    pub by_date: Vec<DueDateTotalOut>,
}

// ---------------------------------------------------------------------------
// Foreign exchange
// ---------------------------------------------------------------------------

/// Parameters for `books.set_fx_rate`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SetFxRateParams {
    /// Foreign currency code, e.g. "EUR".
    pub currency: String,
    /// Rate date, YYYY-MM-DD.
    pub date: String,
    /// Rate: how much base currency one unit of foreign buys, decimal string.
    pub rate: String,
}

/// Output of `books.set_fx_rate`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SetFxRateOut {
    /// Always true on success.
    pub ok: bool,
}

// ---------------------------------------------------------------------------
// Payroll (accounting only — no tax tables; the caller supplies all numbers)
// ---------------------------------------------------------------------------

/// One employee's payroll numbers. All amounts are decimal strings.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PayrollLineIn {
    /// The employee's party (id or name), which must have the employee
    /// role. New runs should name it; without it the line carries only
    /// `employee_name`.
    pub employee: Option<String>,
    /// Employee name for the run record. Defaults to the employee party's
    /// name; required without `employee`.
    pub employee_name: Option<String>,
    /// Gross pay, decimal string.
    pub gross_pay: String,
    /// Federal withholding, decimal string.
    pub federal_withholding: String,
    /// State withholding, decimal string.
    pub state_withholding: String,
    /// Employee Social Security, decimal string.
    pub ss_employee: String,
    /// Employee Medicare, decimal string.
    pub medicare_employee: String,
    /// Employer Social Security, decimal string.
    pub ss_employer: String,
    /// Employer Medicare, decimal string.
    pub medicare_employer: String,
    /// Other deductions (401k, insurance, etc.), decimal string.
    pub other_deductions: String,
}

/// Parameters for `books.run_payroll`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RunPayrollParams {
    /// Pay date, YYYY-MM-DD.
    pub date: String,
    /// Cash account id or code the net pay comes from.
    pub cash_account: String,
    /// One entry per employee.
    pub lines: Vec<PayrollLineIn>,
}

/// Output of `books.run_payroll`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RunPayrollOut {
    /// Payroll run id.
    pub run_id: u64,
    /// The posted journal entry id.
    pub entry_id: u64,
    /// Total gross pay, decimal string.
    pub total_gross: String,
    /// Total net pay, decimal string.
    pub total_net: String,
    /// What to tell the human: an expense account this created (Salary &
    /// Wage Expense 6100, Payroll Tax Expense 6110) has no tax category
    /// yet.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// Parameters for `books.remit_payroll_taxes`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RemitPayrollTaxesParams {
    /// Remittance date, YYYY-MM-DD.
    pub date: String,
    /// Cash account id or code the remittance comes from.
    pub cash_account: String,
    /// The tax agency paid (party id or name, e.g. the IRS), named on the
    /// remittance entry. Required under `policy.parties = "require"`.
    #[serde(default)]
    pub agency: Option<String>,
}

/// Output of `books.remit_payroll_taxes`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RemitPayrollTaxesOut {
    /// The posted journal entry id.
    pub entry_id: u64,
    /// Total remitted, decimal string.
    pub total: String,
}

/// One payroll run for `books.list_payroll_runs`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PayrollRunOut {
    /// Run id.
    pub id: u64,
    /// Pay date, YYYY-MM-DD.
    pub date: String,
    /// Number of employees.
    pub employee_count: usize,
    /// Total gross, decimal string.
    pub total_gross: String,
    /// The posted journal entry id.
    pub entry_id: u64,
}

/// Output of `books.list_payroll_runs`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListPayrollRunsOut {
    /// Runs in id order.
    pub runs: Vec<PayrollRunOut>,
}

// ---------------------------------------------------------------------------
// Classes (divisional tracking)
// ---------------------------------------------------------------------------

/// Parameters for `books.add_class`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AddClassParams {
    /// Class name, e.g. "Rondo".
    pub name: String,
}

/// Output of `books.add_class`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AddClassOut {
    /// Class id.
    pub id: u32,
}

/// One class for `books.list_classes`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ClassOut {
    /// Class id.
    pub id: u32,
    /// Class name.
    pub name: String,
}

/// Output of `books.list_classes`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListClassesOut {
    /// Classes in id order.
    pub classes: Vec<ClassOut>,
}

// ---------------------------------------------------------------------------
// Projects (a class with its facts, rules.md B-37)
// ---------------------------------------------------------------------------
//
// A `project` reference is a class's id or its name (ignoring capitals),
// as `books.add_line`'s class takes it.

/// Parameters for `books.add_project`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct AddProjectParams {
    /// Its name: one line, unique ignoring capitals, never a full TIN.
    pub name: String,
    /// What the project is (its scope of work).
    #[serde(default)]
    pub description: Option<String>,
    /// active (default), paused, finished, or abandoned.
    #[serde(default)]
    pub status: Option<String>,
    /// internal (the company's own product), client (work for a
    /// customer), on_spec (work done hoping to be paid), or other.
    #[serde(default)]
    pub kind: Option<String>,
    /// Who it is for or with: a party's id or name.
    #[serde(default)]
    pub party: Option<String>,
    /// When it started, YYYY-MM-DD.
    #[serde(default)]
    pub started_on: Option<String>,
    /// When it ended, YYYY-MM-DD (finished or abandoned only).
    #[serde(default)]
    pub ended_on: Option<String>,
    /// Web addresses (its site, store listing, repository): at most 8.
    #[serde(default)]
    pub links: Vec<String>,
    /// Anything else.
    #[serde(default)]
    pub notes: Option<String>,
}

/// Parameters for `books.update_project`. Give at least one field to
/// change; "" clears description, kind, party, started_on, ended_on or
/// notes.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct UpdateProjectParams {
    /// The project: id or name.
    pub project: String,
    /// A new name (not blank, not another project's).
    #[serde(default)]
    pub name: Option<String>,
    /// What the project is; "" clears it.
    #[serde(default)]
    pub description: Option<String>,
    /// active, paused, finished, or abandoned.
    #[serde(default)]
    pub status: Option<String>,
    /// internal, client, on_spec, or other; "" clears it (not recorded).
    #[serde(default)]
    pub kind: Option<String>,
    /// Who it is for or with (party id or name); "" clears it.
    #[serde(default)]
    pub party: Option<String>,
    /// When it started, YYYY-MM-DD; "" clears it.
    #[serde(default)]
    pub started_on: Option<String>,
    /// When it ended, YYYY-MM-DD (finished or abandoned only); "" clears it.
    #[serde(default)]
    pub ended_on: Option<String>,
    /// Replaces its links; [] clears them.
    #[serde(default)]
    pub links: Option<Vec<String>>,
    /// Notes; "" clears them.
    #[serde(default)]
    pub notes: Option<String>,
}

/// A party named by a project.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PartyRefOut {
    /// Party id.
    pub party_id: u64,
    /// Its name as shown.
    pub label: String,
}

/// A project: a class with its facts.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ProjectOut {
    /// Its id (the class id: books.add_line's class takes it).
    pub id: u32,
    /// Its name.
    pub name: String,
    /// active, paused, finished, or abandoned.
    pub status: String,
    /// internal, client, on_spec, or other; null: not recorded.
    pub kind: Option<String>,
    /// What it is.
    pub description: Option<String>,
    /// Who it is for or with.
    pub party: Option<PartyRefOut>,
    /// When it started, YYYY-MM-DD.
    pub started_on: Option<String>,
    /// When it ended, YYYY-MM-DD.
    pub ended_on: Option<String>,
    /// Its web addresses (never opened by the books).
    pub links: Vec<String>,
    /// Notes.
    pub notes: Option<String>,
}

/// A project's figures to date: every posted revenue and expense line
/// tagged to it through `through`, closing entries left out, whatever its
/// dates.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ToDateOut {
    /// The last day counted, YYYY-MM-DD.
    pub through: String,
    /// Tagged revenue, decimal string.
    pub revenue: String,
    /// Tagged expenses, decimal string.
    pub expenses: String,
    /// Revenue less expenses, decimal string: its contribution before
    /// overhead (untagged company costs aren't counted).
    pub net: String,
    /// Net as a percent of revenue, one place ("64.0"); null without
    /// revenue (zero or less). Net is revenue less every cost traced to
    /// the project, fixed and variable alike (a segment margin), not a
    /// variable-cost contribution margin.
    pub margin_pct: Option<String>,
    /// On spec only: expenses less revenue, never below zero (what it
    /// would take to be repaid in full; a figure, never a receivable).
    /// Null for any other kind.
    pub unrecovered_cost: Option<String>,
    /// `books.project` only, always there (`[]` when nothing tagged nets
    /// to anything): revenue accounts, then expense accounts (its direct
    /// costs by category), by code, to date. Absent from `books.projects`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by_account: Option<Vec<ProjectAccountOut>>,
}

/// Parameters for `books.projects`.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct ProjectsParams {
    /// Only projects with this status: active, paused, finished, abandoned.
    #[serde(default)]
    pub status: Option<String>,
}

/// One project in `books.projects`.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ProjectRowOut {
    /// Its facts.
    #[serde(flatten)]
    pub project: ProjectOut,
    /// Its figures to date.
    pub to_date: ToDateOut,
}

/// Result of `books.projects`.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ProjectsOut {
    /// The day the figures run through (today in the company's time zone).
    pub through: String,
    /// The projects, by name (ignoring capitals).
    pub projects: Vec<ProjectRowOut>,
}

/// Parameters for `books.project`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ProjectParams {
    /// The project: id or name.
    pub project: String,
    /// Period start ([`START_DOC`]).
    #[schemars(description = START_DOC)]
    pub start: Option<String>,
    /// Period end ([`END_DOC`]).
    #[schemars(description = END_DOC)]
    pub end: Option<String>,
}

/// One account's tagged amount in a project's period.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ProjectAccountOut {
    /// Account code.
    pub code: String,
    /// Account name.
    pub name: String,
    /// "revenue" or "expense".
    #[serde(rename = "type")]
    pub account_type: String,
    /// On the account's normal side, decimal string.
    pub amount: String,
}

/// One month of a project's period.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ProjectMonthOut {
    /// YYYY-MM.
    pub month: String,
    /// Its first day in the period, YYYY-MM-DD.
    pub start: String,
    /// Its last day in the period, YYYY-MM-DD.
    pub end: String,
    /// Tagged revenue, decimal string.
    pub revenue: String,
    /// Tagged expenses, decimal string.
    pub expenses: String,
    /// Revenue less expenses, decimal string.
    pub net: String,
    /// Net to date through its end (every tagged line from the first day
    /// the books keep): the running net toward or away from break-even.
    pub net_to_date: String,
}

/// A project's figures for the period asked.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ProjectPeriodOut {
    /// First day, YYYY-MM-DD.
    pub start: String,
    /// Last day, YYYY-MM-DD.
    pub end: String,
    /// Tagged revenue, decimal string.
    pub revenue: String,
    /// Tagged expenses, decimal string.
    pub expenses: String,
    /// Revenue less expenses, decimal string: its contribution before
    /// overhead.
    pub net: String,
    /// Net as a percent of revenue, one place; null without revenue
    /// (as to date's: every traced cost, not only variable ones).
    pub margin_pct: Option<String>,
    /// Revenue accounts, then expense accounts, by code.
    pub by_account: Vec<ProjectAccountOut>,
    /// By calendar month (the period's last 240 months at most).
    pub by_month: Vec<ProjectMonthOut>,
}

/// An entry with a line tagged to a project.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ProjectEntryOut {
    /// Entry id.
    pub entry_id: u64,
    /// Entry date, YYYY-MM-DD.
    pub date: String,
    /// Description.
    pub description: String,
    /// "draft" or "posted".
    pub status: String,
    /// Its tagged revenue less its tagged expenses, decimal string (a
    /// tagged balance-sheet line counts zero).
    pub project_amount: String,
    /// The entry's total debits, decimal string.
    pub total: Option<String>,
    /// Id of the reversed entry, for reversals.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reversal_of: Option<u64>,
    /// Its receipts (attachment ids: filed on it or linked to it).
    pub receipts: Vec<u64>,
}

/// A project named by a decision, a party or a list.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ProjectRefOut {
    /// Project (class) id.
    pub id: u32,
    /// Its name.
    pub name: String,
}

/// One class's P&L for `books.profit_and_loss_by_class`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ClassPnlOut {
    /// Class name, or null for unclassified lines.
    pub class: Option<String>,
    /// Revenue, decimal string.
    pub revenue: String,
    /// Expenses, decimal string.
    pub expenses: String,
    /// Net income, decimal string.
    pub net_income: String,
}

/// Output of `books.profit_and_loss_by_class`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ProfitAndLossByClassOut {
    /// Period start, YYYY-MM-DD.
    pub start: String,
    /// Period end, YYYY-MM-DD.
    pub end: String,
    /// One row per class (plus unclassified).
    pub classes: Vec<ClassPnlOut>,
}

// ---------------------------------------------------------------------------
// CPA handoff package
// ---------------------------------------------------------------------------

/// Parameters for `books.export_cpa_package`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExportCpaPackageParams {
    /// As of this day ([`AS_OF_DOC`]); the period reports cover the
    /// fiscal year that holds it, through it.
    #[schemars(description = AS_OF_DOC)]
    pub as_of: Option<String>,
    /// Cash account codes the cash flow statement reconciles to, e.g.
    /// `["1000"]`. Defaults to `books.cash_accounts`; one or the other
    /// is required.
    #[serde(default)]
    pub cash_accounts: Vec<String>,
    /// Liability account codes treated as financing. Defaults to
    /// `books.financing_accounts`, else empty.
    #[serde(default)]
    pub financing_accounts: Vec<String>,
    /// Asset account codes treated as investing for cash_flow: loans the
    /// company makes, investments it buys (ASC 230-10-20). Defaults to
    /// `books.investing_accounts`.
    #[serde(default)]
    pub investing_accounts: Vec<String>,
    /// Include every receipt for the year's entries (every current file
    /// filed on or linked to each posted entry, one copy per entry, each
    /// with a row in receipts/index.csv) and the company's documents (the
    /// files behind no entry) (default true).
    #[serde(default = "default_true")]
    pub include_receipts: bool,
    /// Carry every member's and 1099 payee's full TIN from the TIN vault
    /// in the K-1 and 1099 worksheets (default true: the CPA needs them).
    /// False: last fours only.
    #[serde(default = "default_true")]
    pub include_tins: bool,
}

/// Output of `books.export_cpa_package`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExportCpaPackageOut {
    /// The zip: run its `download` to save it.
    pub file: HandedFile,
    /// Whether the worksheets carry full TINs from the vault.
    pub tins_included: bool,
    /// With TINs: each member or 1099 payee whose TIN the package lacks,
    /// and why (not in the vault, the vault locked, ...). Collect them
    /// with books.set_tin or books.set_w9.
    #[serde(default)]
    pub tin_gaps: Vec<String>,
    /// What general-journal.iif writes differently from the books, as
    /// `books.export` iif's `warnings` says it (the memos grouped); the
    /// README lists them too. Absent when there are none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    /// What to tell the human.
    pub note: String,
}

// ---------------------------------------------------------------------------
// Parties (the directory of everyone the company deals with)
// ---------------------------------------------------------------------------
//
// A `party` reference is a party's id or its name: the name ignoring case
// and surrounding space, among parties not archived (an archived party goes
// by id). A number that is one party's id and another party's name is
// refused, never guessed.

/// Parameters for `books.add_party`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct AddPartyParams {
    /// Legal or trade name, as on its documents. Unique (ignoring case and
    /// surrounding space) among parties not archived; never a full TIN.
    pub name: String,
    /// Roles: contractor, employee, member, payout_platform, tax_agency,
    /// government_agency, financial_institution, landlord. Not vendor or customer: those come
    /// with their records (`books.add_vendor`, `books.add_customer`, which
    /// link to the party of the same name). May be empty.
    #[serde(default)]
    pub roles: Vec<String>,
}

/// Parameters for `books.update_party`. Give at least one field to change.
/// Every field given is checked before any is written: a bad one changes
/// nothing. Each is journaled as its own `party.updated`, in this order:
/// name, display_name, website, address, notes, archived; then one
/// `party.creditor_set` per `creditor_of` account added or dropped.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct UpdatePartyParams {
    /// The party: id or name.
    pub party: String,
    /// A new legal or trade name (not blank, not another party's).
    pub name: Option<String>,
    /// A shorter name to show; "" clears it.
    pub display_name: Option<String>,
    /// Website; "" clears it.
    pub website: Option<String>,
    /// Business address; "" clears it.
    pub address: Option<String>,
    /// Notes; "" clears them.
    pub notes: Option<String>,
    /// Email address, where the company writes to them (A-13): copied onto
    /// an invoice's bill-to or a bill's vendor at issue. "" clears it.
    pub email: Option<String>,
    /// The payment terms this party grants the company as a customer
    /// ("Net 30", "2/10 net 30"): an invoice's due date falls back to them
    /// when neither it nor the call gives terms. "" clears them.
    pub default_terms_as_customer: Option<String>,
    /// The payment terms this party, as a vendor, gives the company: a
    /// bill's due date falls back to them. "" clears them.
    pub default_terms_as_vendor: Option<String>,
    /// Telephone number (D-55). "" clears it.
    pub phone: Option<String>,
    /// The named person there to write to (D-55). "" clears it.
    pub contact: Option<String>,
    /// Where its bills go (D-55): an invoice's bill-to address when set,
    /// else `address`. "" clears it.
    pub billing_address: Option<String>,
    /// Where goods go (D-55). "" clears it.
    pub shipping_address: Option<String>,
    /// Where its bills are emailed (D-55): an invoice's bill-to email when
    /// set, else `email`. "" clears it.
    pub billing_email: Option<String>,
    /// True: no longer dealt with (its history stays; it no longer
    /// resolves by name, and its name is free for another party). False:
    /// dealt with again (refused while another party has its name).
    pub archived: Option<bool>,
    /// The liability accounts (codes or ids) the company owes this party
    /// itself on: the card it issued, the loan it made, what the members
    /// advanced (name each member). Replaces its list; [] clears it. A
    /// credit to one never counts as paying the party (a fee on its card,
    /// interest on its loan, a distribution still owed: a promise, counted
    /// when paid). Liabilities only, not placeholders, payables or payroll
    /// withholdings.
    pub creditor_of: Option<Vec<String>>,
}

/// Parameters for `books.set_party_roles`. Give at least one role to add
/// or remove.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SetPartyRolesParams {
    /// The party: id or name.
    pub party: String,
    /// Roles to give it (see `books.add_party`; not vendor or customer).
    #[serde(default)]
    pub add: Vec<String>,
    /// Roles to take away. Refused while facts need the role: clear the
    /// W-9 (`books.set_w9` clear) before removing its last 1099 role
    /// (contractor, landlord), and the W-4 before removing employee.
    #[serde(default)]
    pub remove: Vec<String>,
}

/// Parameters for `books.set_w9`: what a W-9 said, never the number
/// itself. Either `clear: true` alone, or every required fact. No other
/// argument is taken: the form's bytes never come as one (D-46).
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetW9Params {
    /// The party: id or name. Needs a 1099 role (contractor or landlord).
    pub party: String,
    /// When the W-9 was received, YYYY-MM-DD. Required unless clearing.
    pub received_on: Option<String>,
    /// Line 3 federal tax classification, e.g. "individual/sole
    /// proprietor", "C corporation", "LLC (partnership)". Required unless
    /// clearing.
    pub entity_type: Option<String>,
    /// Line 4 exempt payee code, if any.
    pub exempt_payee_code: Option<String>,
    /// Line 4 FATCA exemption code, if any.
    pub fatca_code: Option<String>,
    /// The TIN's last four digits: exactly four digits. A full TIN (nine
    /// digits, however written) is refused here and in every party field.
    /// Required unless clearing.
    pub tin_last4: Option<String>,
    /// Where the form itself is kept, e.g. "CPA portal" or "filed in the
    /// books, document 12". Required unless clearing.
    pub kept_at: Option<String>,
    /// True: clear the W-9 facts (give no other fact).
    #[serde(default)]
    pub clear: bool,
    /// The full TIN (nine digits, hyphens allowed), stored only in the
    /// encrypted TIN vault, never in the books; the books keep its last
    /// four. Needs tin_type. Never echoed back.
    pub tin: Option<SecretTin>,
    /// The TIN's type: ssn, ein, or itin. Required with tin.
    pub tin_type: Option<String>,
    /// The signed W-9 itself (a PDF, PNG, or JPEG): an upload made with
    /// books.begin_upload purpose w9 and `wanderbooks upload` (its id; a
    /// file never passes through the agent, D-46). Stored only in the
    /// vault (it shows the full TIN), never with the receipts and
    /// documents.
    #[serde(default)]
    pub w9_upload_id: Option<String>,
    /// The W-9 file's name (default: the upload's).
    #[serde(default)]
    pub w9_file_name: Option<String>,
}

/// A full TIN as a tool takes it: text (or a JSON number) whose value is
/// never printed: its Debug, and any error deserializing it, say nothing
/// of the digits.
#[derive(Clone, JsonSchema)]
#[schemars(transparent)]
pub struct SecretTin(pub String);

impl std::fmt::Debug for SecretTin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretTin(..)")
    }
}

impl<'de> Deserialize<'de> for SecretTin {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl serde::de::Visitor<'_> for V {
            type Value = SecretTin;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a TIN: nine digits as a string")
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<SecretTin, E> {
                Ok(SecretTin(v.to_string()))
            }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<SecretTin, E> {
                Ok(SecretTin(v.to_string()))
            }
            fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<SecretTin, E> {
                Err(E::custom("tin must be nine digits"))
            }
            fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<SecretTin, E> {
                Err(E::custom("tin must be nine digits"))
            }
        }
        d.deserialize_any(V)
    }
}

/// Parameters for `books.set_tin`: a party's full taxpayer
/// identification number, for the TIN vault only.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SetTinParams {
    /// The party: id or name. Any role (a member, a contractor, ...).
    pub party: String,
    /// The full TIN (nine digits, hyphens allowed), stored only in the
    /// encrypted TIN vault; the books keep its type and last four. Never
    /// echoed back.
    pub tin: SecretTin,
    /// The TIN's type: ssn, ein, or itin.
    pub tin_type: String,
}

/// A party's taxpayer-ID facts: never more of the number than its last
/// four.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct TinOut {
    /// ssn, ein, or itin.
    pub tin_type: String,
    /// The last four digits.
    pub last4: String,
    /// True while the TIN vault holds the full number (by the books'
    /// record of it).
    pub in_vault: bool,
    /// When and why the vault's copy was purged, if it was.
    pub purged: Option<String>,
}

/// Parameters for `books.get_w9`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct GetW9Params {
    /// The party: id or name.
    pub party: String,
}

/// A file the books made for an agent, handed over (tools/files.rs
/// INVARIANT 6; D-29).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct HandedFile {
    /// Its name, e.g. "cpa-package-2026-10-11.zip".
    pub file_name: String,
    /// Its MIME type.
    pub mime_type: String,
    /// Its size in bytes.
    pub size_bytes: u64,
    /// Its SHA-256, hex.
    pub sha256: String,
    /// The command that saves it in a folder you name (run it). Kept 15
    /// minutes.
    pub download: String,
    /// When it lapses (UTC).
    pub expires_at: String,
}

/// Result of `books.get_w9`: the W-9 file, handed over.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct GetW9Out {
    /// The party.
    pub party_id: u64,
    /// The file: run its `download` to save it.
    pub file: HandedFile,
}

/// Parameters for `books.purge_tin`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct PurgeTinParams {
    /// The party: id or name.
    pub party: String,
    /// Why (retention period over, the payee asked, a wrong TIN):
    /// journaled with the W-9 facts.
    pub reason: String,
    /// True: delete the vault records and journal it. Without it, a
    /// preview that changes nothing.
    pub confirmed: Option<bool>,
}

/// Result of `books.purge_tin`.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PurgeTinOut {
    /// "preview" or "purged".
    pub status: String,
    /// The party.
    pub party_id: u64,
    /// Its name.
    pub name: String,
    /// Whether the vault holds (preview) or held (purged) its TIN.
    pub tin_in_vault: bool,
    /// Whether the vault holds (held) its W-9 file.
    pub w9_file_in_vault: bool,
    /// The TIN's last four digits, which the books keep.
    pub tin_last4: Option<String>,
    /// True when the purge is journaled: `party.tin_set` with the vault
    /// record cleared and `purged` saying when and why, and `party.w9_set`
    /// likewise (kept_at) when the party has W-9 facts; false when the
    /// books have no facts to update.
    pub journaled: bool,
    /// What happens (preview) or happened.
    pub note: String,
}

/// Parameters for `books.set_w4`: what a W-4 said, never the SSN. Either
/// `clear: true` alone, or every fact.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SetW4Params {
    /// The party: id or name. Needs the employee role.
    pub party: String,
    /// When the W-4 was received, YYYY-MM-DD. Required unless clearing.
    pub received_on: Option<String>,
    /// The work state: two capital letters, e.g. "UT". Required unless
    /// clearing.
    pub state: Option<String>,
    /// Where the form itself is kept. Required unless clearing.
    pub kept_at: Option<String>,
    /// True: clear the W-4 facts (give no other fact).
    #[serde(default)]
    pub clear: bool,
}

/// Parameters for `books.parties`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ListPartiesParams {
    /// Only parties with this role (any role, vendor and customer
    /// included).
    pub role: Option<String>,
    /// Include archived parties (default false).
    #[serde(default)]
    pub include_archived: bool,
    /// Calendar year for `paid` and `received` ([`YEAR_DOC`]: this year,
    /// so year to date).
    #[schemars(description = YEAR_DOC)]
    pub year: Option<i32>,
    /// What `paid` and `received` count: "cash" (the default: money) or
    /// "all" (money and property contributed or distributed in kind). Not
    /// the books' accounting method. See `books.party` totals.
    pub basis: Option<String>,
}

/// Parameters for `books.party`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct PartyParams {
    /// The party: id or name.
    pub party: String,
    /// Calendar year for the totals ([`YEAR_DOC`]: this year, so year to
    /// date).
    #[schemars(description = YEAR_DOC)]
    pub year: Option<i32>,
    /// What `paid` and `received` count: "cash" (the default: money) or
    /// "all" (money and property contributed or distributed in kind). Not
    /// the books' accounting method. See the totals' schema.
    pub basis: Option<String>,
}

/// Parameters for `books.set_entry_party`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SetEntryPartyParams {
    /// Entry id (draft or posted).
    pub entry_id: u64,
    /// The party (id or name); null or omitted clears the tag. A party with
    /// a vendor record also tags the entry's vendor (so the payment counts
    /// in `books.vendor_1099`); one without, or clearing, clears the vendor
    /// tag too.
    pub party: Option<String>,
    /// Several parties, each with their share, in place of `party`
    /// (journaled as `entry.parties_set`; an empty list clears). Name both
    /// members on a joint member transaction and let the shares derive;
    /// give amounts (or percents) when a split is uneven or a party isn't a
    /// member. Refused: a party named twice, an unknown party, mixed kinds
    /// of share, percents not summing to 100, two parties with vendor
    /// records (a 1099 counts a payment toward one vendor; the one vendor
    /// party's record is tagged), and, once the entry has balanced lines,
    /// shares that don't add up to what it moved.
    pub parties: Option<Vec<PartyShareParam>>,
    /// With `party` null: mark the entry as having no counterparty, with
    /// why (journaled as `entry.no_party`). Satisfies `policy.parties =
    /// "require"`.
    pub no_party_reason: Option<String>,
}

/// A party's W-9 facts.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct W9Out {
    /// When it was received, YYYY-MM-DD.
    pub received_on: String,
    /// Line 3 federal tax classification.
    pub entity_type: String,
    /// Line 4 exempt payee code, if any.
    pub exempt_payee_code: Option<String>,
    /// Line 4 FATCA exemption code, if any.
    pub fatca_code: Option<String>,
    /// The TIN's last four digits.
    pub tin_last4: String,
    /// Where the form is kept.
    pub kept_at: String,
    /// True when the books record a TIN vault record for it (the full TIN
    /// is in the vault, never here).
    pub in_vault: bool,
}

/// A party's W-4 facts.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct W4Out {
    /// When it was received, YYYY-MM-DD.
    pub received_on: String,
    /// The work state.
    pub state: String,
    /// Where the form is kept.
    pub kept_at: String,
}

/// One party: its facts, as every party tool that changes one returns it.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PartyOut {
    /// Party id.
    pub party_id: u64,
    /// Legal or trade name.
    pub name: String,
    /// The name to show: the display name, else the name.
    pub label: String,
    /// Display name, if set.
    pub display_name: Option<String>,
    /// Roles, in display order: the ones set by hand, plus vendor and
    /// customer when it has those records.
    pub roles: Vec<String>,
    /// Website.
    pub website: Option<String>,
    /// Business address.
    pub address: Option<String>,
    /// Notes.
    pub notes: Option<String>,
    /// Email address (A-13): the general one, where the company writes to
    /// it.
    pub email: Option<String>,
    /// Telephone number (D-55).
    pub phone: Option<String>,
    /// The named person there to write to (D-55).
    pub contact: Option<String>,
    /// Where its bills go (D-55): an invoice's bill-to address when set,
    /// else `address`.
    pub billing_address: Option<String>,
    /// Where goods go (D-55).
    pub shipping_address: Option<String>,
    /// Where its bills are emailed (D-55): the bill-to email when set,
    /// else `email`.
    pub billing_email: Option<String>,
    /// The terms it grants the company as a customer ("Net 30"), when set.
    pub default_terms_as_customer: Option<String>,
    /// The terms it gives the company as a vendor, when set.
    pub default_terms_as_vendor: Option<String>,
    /// No longer dealt with.
    pub archived: bool,
    /// Its vendor record's id, if it bills the company.
    pub vendor_id: Option<u64>,
    /// Its customer record's id, if the company invoices it.
    pub customer_id: Option<u64>,
    /// W-9 facts, if on file.
    pub w9: Option<W9Out>,
    /// W-4 facts, if on file.
    pub w4: Option<W4Out>,
    /// Taxpayer-ID facts (any role), if on file: type and last four.
    pub tin: Option<TinOut>,
    /// The liability accounts the company owes this party itself on (its
    /// card, its loan, what the members advanced), by code: a credit to
    /// one never counts as paying it.
    pub creditor_of: Vec<String>,
}

/// One party in `books.parties`.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PartySummaryOut {
    /// Party id.
    pub party_id: u64,
    /// Legal or trade name.
    pub name: String,
    /// The name to show.
    pub label: String,
    /// Roles, vendor and customer included.
    pub roles: Vec<String>,
    /// Its vendor record's id, if any.
    pub vendor_id: Option<u64>,
    /// Its customer record's id, if any.
    pub customer_id: Option<u64>,
    /// W-9 facts are on file.
    pub w9_on_file: bool,
    /// The TIN's last four digits when the books know it: from its TIN
    /// facts (`books.set_tin`; a member's SSN for the K-1) or its W-9
    /// facts, which never disagree. Null when neither is on file. Whether
    /// the vault still holds the full number: `books.party` `tin.in_vault`.
    pub tin_last4: Option<String>,
    /// W-4 facts are on file.
    pub w4_on_file: bool,
    /// No longer dealt with.
    pub archived: bool,
    /// Paid to the party in the year on `basis`, decimal string (as
    /// `books.party` totals).
    pub paid: String,
    /// Received from the party in the year on `basis`, decimal string (as
    /// `books.party` totals).
    pub received: String,
}

/// Result of `books.parties`.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ListPartiesOut {
    /// The year `paid` and `received` cover.
    pub year: i32,
    /// What `paid` and `received` count: "cash" or "all".
    pub basis: String,
    /// Parties, by id.
    pub parties: Vec<PartySummaryOut>,
}

/// A party's year, from the journal. "The party's entries" are the
/// entries tagged with the party (`books.set_entry_party`, `books.draft_entry`
/// party), the entries tagged with its vendor record (bills and their
/// payments, `books.set_entry_vendor`), its customer record's invoices and
/// the payments received on them, and reversals of any of these. Only
/// posted entries dated in the calendar year count.
///
/// `paid` and `received` add up each entry's part for the party, as
/// `books.get_entry` shows it (`resolved`, `direction`, `cash`,
/// `in_kind`): on `basis` "cash" (the default) what moved as money, on
/// "all" that plus property contributed or distributed in kind. A
/// reversal counts on its original's side and nets it to zero. (Not the
/// books' accounting method.)
///
/// Money is what left or reached the `books.cash_accounts` (every asset
/// account but receivables when none are set), the cards (the
/// business-card payment methods' liabilities, `books.instruments`; while
/// none is recorded, liabilities whose names say "card", not "gift
/// card"), and the `books.financing_accounts` (what the members
/// advanced, loans): a card charge or a member paying for the company is
/// paid to the payee then; cash that pays one of these down is paid to
/// its creditor (a reimbursement to the members, a card payment to the
/// issuer); cash it raises is received from the lender. Any other
/// liability is a promise (a bill, an accrual, deferred revenue): counted
/// when it is settled, never also when it is booked. A credit to what the
/// company owes a party itself (`books.party` `creditor_of`: a fee on its
/// own card, interest on its own loan, a distribution still owed to a
/// member) never counts as paying it.
///
/// An entry naming only members that moves equity counts what moved their
/// capital: a contribution received, the property in it in kind (asset
/// accounts that are neither cash nor receivables) and the rest cash, as
/// on the K-1 worksheet; a distribution paid, the money it paid as cash
/// and the property it gave in kind, the rest (declared to distributions
/// payable, or credited to what the members advanced once they are named
/// its creditors) still owed, counted when it is paid. So for a member,
/// basis "all" over those entries and those payments is the statement of
/// members' equity's contributions and distributions, and basis "cash"
/// its cash contributions as the K-1 worksheet has them, but for a
/// distribution declared and not yet paid, which is in the statement only. Known limits (record them apart): an entry
/// that both repays a member and distributes to them counts the
/// distribution only; a member's money that goes through the members'
/// financing account both ways under their own name (lent, then converted
/// to capital) counts both times, as does a distribution declared through
/// it before the members are named its creditors; an expense a member paid
/// booked straight to capital and tagged with the vendor isn't paid to the
/// vendor; capital moved between members counts for neither.
///
/// An entry naming several parties counts each one's share (its amount,
/// its percent, or its derived share: see `books.set_entry_party`
/// `parties`); an entry whose shares don't resolve counts for none of
/// them. A transfer between the company's own cash accounts nets to zero.
/// (`ledger_core::parties::activity_with`, with the `[[company.members]]`,
/// computes it; the viewer shows the same numbers.)
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PartyTotalsOut {
    /// What `paid` and `received` count: "cash" or "all".
    pub basis: String,
    /// Paid to the party on `basis`, decimal string.
    pub paid: String,
    /// Received from the party on `basis`, decimal string.
    pub received: String,
    /// Its customer record's invoices dated in the year, total, decimal
    /// string.
    pub invoiced: String,
    /// Its vendor record's bills dated in the year, total, decimal string.
    pub billed: String,
    /// Still owed to it now: its open bills' balances (every year),
    /// decimal string.
    pub owed: String,
    /// Still owed by it now: its open invoices' balances (every year),
    /// decimal string.
    pub receivable: String,
    /// For a party with a vendor record: what `books.vendor_1099` counts
    /// toward that vendor's 1099 in the year (out of the cash and
    /// financing accounts; card and payment-network payments excluded,
    /// payments with no method recorded on a financing account left
    /// undetermined), decimal string. Absent otherwise, or when the
    /// worksheet can't be made (no cash accounts named).
    pub amount_1099: Option<String>,
    /// The information return its vendor record's decision in force today
    /// files ("1099-NEC", "1099-MISC"), if any.
    pub form_1099: Option<String>,
    /// Its vendor record's 1099 decision in force today ("nec", "misc",
    /// "not_reportable", "undetermined"). Absent without a vendor record.
    pub decision_1099: Option<String>,
    /// Gross pay on payroll lines naming it, decimal string.
    pub gross_pay: String,
    /// Net pay on payroll lines naming it, decimal string.
    pub net_pay: String,
}

/// One of a party's entries.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PartyEntryOut {
    /// Entry id.
    pub entry_id: u64,
    /// Entry date, YYYY-MM-DD.
    pub date: String,
    /// Description.
    pub description: String,
    /// "draft" or "posted".
    pub status: String,
    /// Total debits, decimal string. Null for a draft whose debits are
    /// past exact arithmetic (one an earlier version journaled).
    pub total: Option<String>,
    /// Id of the reversed entry, for reversals.
    pub reversal_of: Option<u64>,
}

/// One of a party's invoices or bills.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PartyDocumentOut {
    /// Invoice or bill id.
    pub id: u64,
    /// Document date, YYYY-MM-DD.
    pub date: String,
    /// Due date, YYYY-MM-DD.
    pub due_date: String,
    /// Total, decimal string.
    pub total: String,
    /// Paid so far, decimal string.
    pub paid: String,
    /// Still owed, decimal string.
    pub balance: String,
    /// "open", "partial", or "paid".
    pub status: String,
    /// The posted entry that raised it.
    pub entry_id: u64,
}

/// A payroll run line naming the party.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PartyPayrollOut {
    /// Payroll run id.
    pub run_id: u64,
    /// Pay date, YYYY-MM-DD.
    pub date: String,
    /// The run's posted entry.
    pub entry_id: u64,
    /// Gross pay, decimal string.
    pub gross_pay: String,
    /// Net pay, decimal string.
    pub net_pay: String,
}

/// Result of `books.party`.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PartyDetailOut {
    /// Its facts and roles.
    pub party: PartyOut,
    /// The calendar year the totals cover.
    pub year: i32,
    /// The year's totals.
    pub totals: PartyTotalsOut,
    /// Its entries (every year, drafts included), newest first.
    pub entries: Vec<PartyEntryOut>,
    /// Its customer record's invoices, by id.
    pub invoices: Vec<PartyDocumentOut>,
    /// Its vendor record's bills, by id.
    pub bills: Vec<PartyDocumentOut>,
    /// Payroll lines naming it, by run.
    pub payroll: Vec<PartyPayrollOut>,
    /// The projects for or with it, by name.
    pub projects: Vec<ProjectRefOut>,
}

// ---------------------------------------------------------------------------
// Attachment links, statements, reconciliations
// ---------------------------------------------------------------------------

/// A reference: a number or text (a code, tag, or name).
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum RefId {
    /// A number (an id; for an account, its code or id).
    Number(u64),
    /// Text: an account code, an asset tag, a party name, or an id.
    Text(String),
}

/// What an attachment is linked to.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SubjectParam {
    /// entry, account, asset, party, project, invoice, bill, period,
    /// payroll_run, schedule, reconciliation, or company.
    pub kind: String,
    /// Which one: an entry, invoice, bill, period, payroll run, schedule,
    /// or reconciliation id; an account's code (or id); an asset's tag (or
    /// id); a party's or a project's id or name. Omitted for the company.
    pub id: Option<RefId>,
}

/// A subject in a tool's output.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SubjectOut {
    /// Its kind (as in SubjectParam).
    pub kind: String,
    /// Its id (absent for the company). For an account, the account id;
    /// `label` gives the code.
    pub id: Option<u64>,
    /// Human words, e.g. "account 1000 Bank", "entry #5 January rent".
    pub label: String,
}

/// Parameters for `books.link_attachment`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct LinkAttachmentParams {
    /// A stored attachment (receipt or document), not withdrawn.
    pub attachment_id: u64,
    /// What to link it to.
    pub subject: SubjectParam,
}

/// Parameters for `books.unlink_attachment`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct UnlinkAttachmentParams {
    /// The attachment.
    pub attachment_id: u64,
    /// The subject to take it off.
    pub subject: SubjectParam,
    /// Why, recorded in the journal.
    pub reason: String,
}

/// Result of `books.link_attachment` / `books.unlink_attachment`.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct AttachmentLinksOut {
    /// The attachment.
    pub attachment_id: u64,
    /// Everything it is linked to now (the entry it is filed on first).
    pub links: Vec<SubjectOut>,
}

/// Parameters for `books.attachments`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct AttachmentsParams {
    /// The subject whose files to list.
    pub subject: SubjectParam,
}

/// Result of `books.attachments`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct AttachmentsOut {
    /// The subject.
    pub subject: SubjectOut,
    /// Its files (for an entry: filed on it or linked to it), not
    /// withdrawn, by id, each with all its links.
    pub attachments: Vec<ReceiptOut>,
}

/// Parameters for `books.record_statement`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct RecordStatementParams {
    /// The filed statement (`books.file_document` category statement).
    pub attachment_id: u64,
    /// The bank, cash, or card account it is for: code or id.
    pub account: String,
    /// First day it covers, YYYY-MM-DD.
    pub period_start: String,
    /// Last day it covers (the statement date), YYYY-MM-DD.
    pub period_end: String,
    /// Opening balance as printed, in the account's normal direction
    /// (what the bank holds for a bank account; what is owed for a card),
    /// decimal string, cents at most.
    pub opening_balance: String,
    /// Closing balance as printed, same direction, decimal string.
    pub closing_balance: String,
}

/// A recorded statement.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct StatementOut {
    /// The attachment holding it.
    pub attachment_id: u64,
    /// Its file name.
    pub file_name: String,
    /// Account code.
    pub account_code: String,
    /// Account name.
    pub account_name: String,
    /// First day, YYYY-MM-DD.
    pub period_start: String,
    /// Last day (statement date), YYYY-MM-DD.
    pub period_end: String,
    /// Opening balance, decimal string.
    pub opening_balance: String,
    /// Closing balance, decimal string.
    pub closing_balance: String,
    /// The reconciliation (not voided) made against it, if any.
    pub reconciliation_id: Option<u64>,
}

/// Parameters for `books.statements` and `books.reconciliations`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct AccountFilterParams {
    /// Only this account's (code or id). Omit for every account.
    pub account: Option<String>,
}

/// Result of `books.statements`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct StatementsOut {
    /// Recorded statements, by statement date.
    pub statements: Vec<StatementOut>,
}

/// A cleared line a reconciliation recorded.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ClearedRefOut {
    /// Entry id.
    pub entry_id: u64,
    /// Line index within the entry.
    pub line_index: usize,
}

/// A recorded reconciliation.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReconciliationOut {
    /// Its id.
    pub reconciliation_id: u64,
    /// Account code.
    pub account_code: String,
    /// Account name.
    pub account_name: String,
    /// The statement date it reconciles through, YYYY-MM-DD.
    pub statement_date: String,
    /// The statement's closing balance, decimal string.
    pub statement_balance: String,
    /// The statement's attachment.
    pub statement_attachment: u64,
    /// The lines it found cleared.
    pub cleared: Vec<ClearedRefOut>,
    /// True when voided.
    pub voided: bool,
    /// Why it was voided, if it was.
    pub voided_reason: Option<String>,
}

/// Parameters for `books.void_statement`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct VoidStatementParams {
    /// The attachment whose recorded statement to void.
    pub attachment_id: u64,
    /// Why (what was recorded wrong), recorded in the journal.
    pub reason: String,
}

/// Parameters for `books.void_reconciliation`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct VoidReconciliationParams {
    /// The reconciliation's id.
    pub reconciliation_id: u64,
    /// Why, recorded in the journal.
    pub reason: String,
}

/// Result of `books.reconciliations`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ReconciliationsOut {
    /// Reconciliations (voided ones included, marked), by date.
    pub reconciliations: Vec<ReconciliationOut>,
}

/// Parameters for `books.handbook`.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct HandbookParams {
    /// The section to read: contract, entries, drafts-and-reversals, evidence-and-receipts, attachments, files-in-out, reconciliation, fixed-assets, prepaid-and-recurring, revenue-and-payouts, projects, parties-and-tins, 1099, cpa-handoff, settings-and-conventions, viewer, backups, self-host, format-upgrade, rules-that-change, control-roles, tracing-figures, invoices-and-bills, or "all" for the whole handbook in order. Omit for the index.
    #[serde(default)]
    pub section: Option<String>,
}

/// One handbook section in the index.
#[derive(Debug, Serialize, JsonSchema)]
pub struct HandbookSectionOut {
    /// Section id, for `section` and `books://handbook/{id}`.
    pub id: String,
    /// Section title.
    pub title: String,
    /// The section's resource URI.
    pub uri: String,
    /// Length of the section's text, in characters.
    pub chars: usize,
}

/// One handbook section's text.
#[derive(Debug, Serialize, JsonSchema)]
pub struct HandbookTextOut {
    /// Section id.
    pub id: String,
    /// Section title.
    pub title: String,
    /// The section, as Markdown.
    pub text: String,
}

/// Result of `books.handbook`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct HandbookOut {
    /// Where to start reading.
    pub read_first: String,
    /// Every section, in reading order.
    pub sections: Vec<HandbookSectionOut>,
    /// The section asked for (null: none was).
    pub section: Option<HandbookTextOut>,
}

// ---------------------------------------------------------------------------
// P4b S4.4: invoices and bills preview, then confirm
// ---------------------------------------------------------------------------

/// One line of a document's posting, as a preview shows it: the account's
/// code and name, its side, the amount, and the project it is tagged with.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct DocumentPostingOut {
    /// The account's code ("1200").
    pub account: String,
    /// Its name ("Accounts Receivable").
    pub name: String,
    /// "debit" or "credit".
    pub side: String,
    /// The amount, decimal string.
    pub amount: String,
    /// The project the line is tagged with, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<String>,
}

/// An invoice's PDF: the DRAFT copy a preview hands over, or the issued
/// copy a confirm files, named by its attachment (A-7).
#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum DocumentPdfOut {
    /// The DRAFT copy, handed over: run its `download` to save it (D-46).
    Draft(HandedFile),
    /// The issued copy, filed with the invoice in the call that issued it.
    Issued(IssuedFileOut),
}

/// A file filed with a document: its attachment and SHA-256.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct IssuedFileOut {
    /// The attachment (books.get_receipt).
    pub attachment_id: u64,
    /// The file's SHA-256, hex: the same one books.get_receipt reports.
    pub sha256: String,
}

/// Result of `books.create_invoice`: a preview (nothing posted) or the
/// posted invoice.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct InvoiceOut {
    /// True: a preview, and nothing was posted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<bool>,
    /// The invoice's id (confirm).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<u64>,
    /// Its number (confirm: the one issued; preview: the one it would
    /// take). Absent in books that don't number their invoices yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number: Option<String>,
    /// The total, decimal string.
    pub total: String,
    /// The due date, YYYY-MM-DD.
    pub due_date: String,
    /// The terms' text ("Net 30"), when it has them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terms: Option<String>,
    /// The auto-posted entry's id (confirm).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_id: Option<u64>,
    /// The posting, line by line (preview).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lines: Vec<DocumentPostingOut>,
    /// The PDF: DRAFT and handed over on a preview, the issued copy filed
    /// with the invoice on a confirm.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pdf: Option<DocumentPdfOut>,
    /// A `mailto:` link for a human to send it from their own mail
    /// program (the books never send, A-4). The file is attached by hand.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mailto: Option<String>,
    /// What to tell the human: a revenue account this created (4000) has
    /// no tax category yet.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    /// One per finished or abandoned project a line names, and one per
    /// on-spec project a line names (is there now an agreement?).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

/// Result of `books.create_bill`: a preview (nothing posted) or the
/// posted bill.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct BillOut {
    /// True: a preview, and nothing was posted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<bool>,
    /// The bill's id (confirm).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<u64>,
    /// The total, decimal string.
    pub total: String,
    /// The due date, YYYY-MM-DD.
    pub due_date: String,
    /// The terms' text ("2/10 net 30"), when it has them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terms: Option<String>,
    /// The auto-posted entry's id (confirm).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_id: Option<u64>,
    /// The posting, line by line (preview).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lines: Vec<DocumentPostingOut>,
    /// The last day its early-payment discount is open, when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discount_by: Option<String>,
    /// The largest discount its terms allow, decimal string (the caller
    /// gives the exact cents when it takes one).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_discount: Option<String>,
    /// The vendor's invoice, filed with the bill (confirm).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<IssuedFileOut>,
    /// What to tell the human: an expense account this created (6999) has
    /// no tax category yet.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    /// One per finished or abandoned project a line names.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

// P4b S5: sent tracking.

/// Parameters for `books.mark_sent`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MarkSentParams {
    /// The invoice: `invoice:7` (as books.documents names it) or its id,
    /// `7`.
    pub invoice: String,
    /// The day it went out, YYYY-MM-DD (omitted: today in the company's
    /// time zone).
    #[serde(default)]
    pub on: Option<String>,
    /// Who sent it: the owner, or a named delegate such as an agent
    /// (omitted: this session's token name).
    #[serde(default)]
    pub by: Option<String>,
    /// The channel: email (the default), mail, portal, hand, or another
    /// channel's own name.
    #[serde(default)]
    pub via: Option<String>,
    /// The recipients (omitted: the invoice's bill-to email, else the
    /// customer's email; refused when neither is set). The journal keeps
    /// them as the send's `to`.
    #[serde(default)]
    pub recipients: Option<Vec<String>>,
    /// original (the default for a first send) or reminder (the default
    /// after one).
    #[serde(default)]
    pub kind: Option<String>,
    /// The channel's own id (a message id, a tracking number).
    #[serde(default)]
    pub reference: Option<String>,
    /// A note about the send.
    #[serde(default)]
    pub note: Option<String>,
}

/// One recorded send of an invoice (P4b S5).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SentOut {
    /// The day it went out.
    pub on: String,
    /// Who sent it.
    pub by: String,
    /// How: email, mail, portal, hand, or another channel's name.
    pub via: String,
    /// The recipients.
    pub to: Vec<String>,
    /// original or reminder.
    pub kind: String,
    /// The channel's own id, when given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    /// A note about the send, when given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Output of `books.mark_sent`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MarkSentOut {
    /// The invoice, as `invoice:7`.
    pub invoice: String,
    /// Every send on it, in order, the newest last.
    pub sends: Vec<SentOut>,
}
