//! Tool parameter and output types. Amounts cross MCP as decimal strings,
//! never JSON floats.

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
    /// Human name, e.g. "Business Checking".
    pub name: String,
    /// One of: asset, liability, equity, revenue, expense.
    pub account_type: String,
    /// Parent account id for hierarchy. Omit for top-level.
    pub parent: Option<u64>,
    /// True for header accounts that never take postings.
    pub placeholder: Option<bool>,
}

/// Result of `books.add_account`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct AddAccountOut {
    /// The new account's id.
    pub account_id: u64,
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
    /// Any date in the first month covered, YYYY-MM-DD.
    pub start: String,
    /// Months covered.
    pub months: u32,
    /// Prepaid (asset) or deferred revenue (liability) account code.
    pub balance_account: String,
    /// Expense or revenue account code.
    pub pl_account: String,
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

/// Result of `books.run_schedules`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct RunSchedulesOut {
    /// "preview" or "posted".
    pub status: String,
    /// Postings due (preview) or made.
    pub postings: Vec<SchedulePostingOut>,
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
    /// Optional class (name or id), e.g. the app or product line.
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
    /// Optional class (name or id).
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
    /// "1099-NEC", "1099-MISC", or absent.
    pub form_1099: Option<String>,
    /// W-9 on file.
    pub tin_on_file: bool,
    /// The party the vendor record belongs to.
    pub party_id: Option<u64>,
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
    /// "nec" (contractors, attorneys), "misc" (rent, royalties, other), or
    /// "none" (corporations, most merchants).
    pub form_1099: String,
    /// A W-9 is on file (the TIN itself is never stored).
    #[serde(default)]
    pub tin_on_file: bool,
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
}

/// One vendor's year.
#[derive(Debug, Serialize, JsonSchema)]
pub struct Vendor1099LineOut {
    /// Vendor id.
    pub vendor_id: u64,
    /// Name.
    pub name: String,
    /// "1099-NEC", "1099-MISC", or absent.
    pub form_1099: Option<String>,
    /// W-9 on file.
    pub tin_on_file: bool,
    /// Cash paid in the year, decimal string.
    pub paid: String,
    /// Marked for a 1099 and paid at least the threshold.
    pub reportable: bool,
    /// The vendor's party.
    pub party_id: Option<u64>,
    /// The TIN's last four digits, from the party's W-9 facts.
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

/// Result of `books.vendor_1099`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct Vendor1099Out {
    /// Calendar year.
    pub year: i32,
    /// Threshold applied, decimal string.
    pub threshold: String,
    /// Vendors marked for a 1099 or paid in the year.
    pub vendors: Vec<Vendor1099LineOut>,
    /// Reportable vendors without a W-9 on file: request one.
    pub missing_w9: Vec<String>,
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
    /// A reconciliation (id from `books.reconciliations`): its account's
    /// page, with the reconciliation and its statement.
    pub reconciliation: Option<u64>,
    /// Search the transactions list (description, account, vendor, amount).
    pub search: Option<String>,
    /// A page: overview, cash, profit, transactions, reports, accounts,
    /// assets, parties, documents, company, journal.
    pub page: Option<String>,
    /// For account and report: first date, YYYY-MM-DD.
    pub start: Option<String>,
    /// For account and report: last date, YYYY-MM-DD.
    pub end: Option<String>,
    /// For a report: split into columns, "total" (default), "month",
    /// "quarter", or "year" (P&L, balance sheet, cash flow).
    pub columns: Option<String>,
    /// For a report: compare with "prior_period" or "prior_year".
    pub compare: Option<String>,
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
    /// The `wanderbooks.toml` settings that go with it: tax form, owners,
    /// where period closes go.
    pub settings_toml: String,
    /// What to decide or know.
    pub notes: Vec<String>,
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
    /// Settings in wanderbooks.toml that still name the old code: change
    /// each to the new one (the server reloads the file).
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
    /// Where the settings come from: "books" (the books keep them; change
    /// them with books.set_setting) or "file" (a settings file these books
    /// still read: move it in with books.import_settings).
    pub source: String,
    /// The company's time zone (IANA), or null: the computer's.
    pub timezone: Option<String>,
    /// Today, in that time zone.
    pub today: String,
    /// Whether approval rules block posting: "off" or "enforce".
    pub approval_rules: String,
    /// Every setting the books know: what it takes, its value (null:
    /// unset, its default applies), and who set it.
    pub keys: Vec<crate::server::SettingKeyOut>,
    /// The wanderbooks.toml the server reads (null: the books keep their
    /// own settings, or settings built in).
    pub books_toml: Option<String>,
    /// Whether that file exists (absent: the defaults apply).
    pub books_toml_exists: bool,
    /// Whether the server reads a receipt policy from a configuration
    /// file of its own.
    pub config_json_present: bool,
    /// Why the last edit failed to load, or null. While set, the settings
    /// below are the last ones that loaded: fix the file.
    pub reload_error: Option<String>,
    /// How many times the settings have loaded (the first included); it
    /// goes up each time an edit loads.
    pub loads: u64,
    /// `[company] name`.
    pub company: String,
    /// `[company] fiscal_year_start`, "MM-DD".
    pub fiscal_year_start: String,
    /// The return filed (`tax_form`, or from `tax_classification`), or null.
    pub tax_form: Option<String>,
    /// `[books] accounting_basis`.
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
    /// `[books] parties`: "off", "suggest", or "require".
    pub parties: String,
    /// `[[company.members]]`.
    pub members: Vec<MemberSettingOut>,
    /// `[books] cash_accounts`.
    pub cash_accounts: Vec<String>,
    /// `[books] financing_accounts`.
    pub financing_accounts: Vec<String>,
    /// `[books] investing_accounts`.
    pub investing_accounts: Vec<String>,
    /// `[books] close_into`.
    pub close_into: Option<String>,
    /// `[books] close_sweep`.
    pub close_sweep: Vec<String>,
    /// Settings that disagree with the books, each saying what to change in
    /// wanderbooks.toml (or the chart). Empty: nothing to fix.
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
    /// As of this date, YYYY-MM-DD. Defaults to today.
    #[serde(default)]
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
    /// Period start, YYYY-MM-DD. Defaults to the beginning of time.
    pub start: Option<String>,
    /// Period end, YYYY-MM-DD. Defaults to today.
    pub end: Option<String>,
    /// Optional class name or id to filter to one division.
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
    /// Period start, YYYY-MM-DD. Defaults to the beginning of time.
    pub start: Option<String>,
    /// Period end, YYYY-MM-DD. Defaults to today.
    pub end: Option<String>,
    /// Account codes treated as cash (e.g. `["1000", "1010"]`): the
    /// statement reconciles to the net change in these accounts. Defaults
    /// to `[books] cash_accounts`; one or the other is required.
    #[serde(default)]
    pub cash_accounts: Vec<String>,
    /// Account codes for liabilities treated as financing (loans, and what
    /// owners advanced: ASC 230-10-45-14(b)). Defaults to `[books]
    /// financing_accounts`, else empty (liabilities are working capital).
    #[serde(default)]
    pub financing_accounts: Vec<String>,
    /// Asset account codes treated as investing for cash_flow: loans the
    /// company makes, investments it buys (ASC 230-10-20). Defaults to
    /// `[books] investing_accounts`.
    #[serde(default)]
    pub investing_accounts: Vec<String>,
}

/// One cash flow line.
#[derive(Debug, Serialize, JsonSchema)]
pub struct CashFlowLineOut {
    /// Human label.
    pub label: String,
    /// Signed amount, decimal string. Positive = inflow.
    pub amount: String,
}

/// Result of `books.cash_flow`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct CashFlowOut {
    /// Period start, YYYY-MM-DD.
    pub start: String,
    /// Period end, YYYY-MM-DD.
    pub end: String,
    /// Period net income, decimal string.
    pub net_income: String,
    /// Non-cash depreciation added back, decimal string.
    pub depreciation: String,
    /// Working-capital adjustments.
    pub working_capital: Vec<CashFlowLineOut>,
    /// Cash from operations, decimal string.
    pub cash_from_operations: String,
    /// Investing flows (purchases, disposal proceeds).
    pub investing: Vec<CashFlowLineOut>,
    /// Cash from investing, decimal string.
    pub cash_from_investing: String,
    /// Financing flows (equity, designated debt).
    pub financing: Vec<CashFlowLineOut>,
    /// Cash from financing, decimal string.
    pub cash_from_financing: String,
    /// Net change in cash, decimal string.
    pub net_change_in_cash: String,
    /// Cash balance at period start, decimal string.
    pub cash_beginning: String,
    /// Cash balance at period end, decimal string.
    pub cash_ending: String,
}

/// Parameters for `books.import_bank_csv`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ImportBankCsvParams {
    /// The CSV text (QBO bank upload layout: Date,Description,Amount, or
    /// Date,Description, money out, money in)…
    #[serde(default)]
    pub csv_text: Option<String>,
    /// …or an upload of it made with books.begin_upload (purpose import).
    #[serde(default)]
    pub upload_id: Option<String>,
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

/// Parameters for `books.import_chart_csv`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ImportChartCsvParams {
    /// The CSV text: code,name,type,parent_code with a header row…
    #[serde(default)]
    pub csv_text: Option<String>,
    /// …or an upload of it made with books.begin_upload (purpose import).
    #[serde(default)]
    pub upload_id: Option<String>,
}

/// Result of `books.import_chart_csv`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ImportChartCsvOut {
    /// Account codes created, in order.
    pub created: Vec<String>,
    /// Number of accounts created.
    pub count: usize,
}

/// Parameters for `books.balance_sheet`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct BalanceSheetParams {
    /// As of this date, YYYY-MM-DD. Defaults to today.
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
    /// The party this entry is with (id or name; see `books.parties`). A
    /// party with a vendor record also tags the entry's vendor; given with
    /// `vendor`, both must name the same vendor.
    pub party: Option<String>,
    /// Why the entry has no counterparty (a transfer between the company's
    /// own accounts, an accrual), when `[books] parties = "require"` and
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
    /// Its exact part of what the entry moved (positive: paid to it;
    /// negative: received from it; for an entry moving no money, its part
    /// of the total), decimal string. Null when the shares don't resolve
    /// (see `parties_error`).
    pub resolved: Option<String>,
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
    /// Optional class name or id to tag this line for divisional tracking.
    /// Applied after the line is added; only valid on draft entries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<String>,
}

/// Result of `books.add_line`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct AddLineOut {
    /// True when the line was added.
    pub ok: bool,
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
    /// `[books] parties`: "off", "suggest", or "require".
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

/// Parameters for `books.attach_receipt`.
#[derive(Debug, Deserialize, JsonSchema)]
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
    /// The file's bytes: an upload made with books.begin_upload (its id)…
    #[serde(default)]
    pub upload_id: Option<String>,
    /// …or, for a file of at most 256 KiB, its bytes base64-encoded, with
    /// sha256.
    #[serde(default)]
    pub content_base64: Option<String>,
    /// The SHA-256 of the bytes in content_base64, hex.
    #[serde(default)]
    pub sha256: Option<String>,
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

/// Parameters for `books.file_document`.
#[derive(Debug, Deserialize, JsonSchema)]
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
    /// The file's bytes: an upload made with books.begin_upload (its id)…
    #[serde(default)]
    pub upload_id: Option<String>,
    /// …or, for a file of at most 256 KiB, its bytes base64-encoded, with
    /// sha256.
    #[serde(default)]
    pub content_base64: Option<String>,
    /// The SHA-256 of the bytes in content_base64, hex.
    #[serde(default)]
    pub sha256: Option<String>,
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
    /// Category: receipts, formation, legal, tax, accounting, other.
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
    /// to), or "bytes" (also its bytes, base64, for a file of at most
    /// 256 KiB; a larger one comes with `wanderbooks download`).
    #[serde(default)]
    pub view: Option<String>,
}

/// Result of `books.get_receipt`: what it is and what it's linked to, and
/// with view `bytes`, its bytes (hash-verified on read) or the command that
/// saves a copy.
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
    /// Its bytes, base64-encoded: with view `bytes`, for a file of at most
    /// 256 KiB.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_base64: Option<String>,
    /// With view `bytes`, for a file too large to come back here: the
    /// command that saves a copy in a folder you name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download: Option<String>,
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
    /// Settings in wanderbooks.toml that need changing: an edit that
    /// didn't load, or a setting that disagrees with the chart. Fix each
    /// with the human, then check books.settings.
    pub settings_problems: Vec<String>,
    /// Cash and card accounts (`[books] cash_accounts`, and liabilities
    /// with a recorded statement) with activity not reconciled through the
    /// end of last month: record each statement and reconcile it.
    pub unreconciled_accounts: Vec<UnreconciledAccountOut>,
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
    /// "csv", "json", "xlsx", "pdf", or "iif". XLSX and PDF are
    /// base64-encoded; PDF covers profit_and_loss, balance_sheet, and
    /// cash_flow; IIF is only valid with report "journal".
    pub format: String,
    /// "journal", "trial_balance", "profit_and_loss", "balance_sheet", or
    /// "cash_flow".
    pub report: String,
    /// Period start for profit_and_loss and cash_flow, YYYY-MM-DD.
    pub start: Option<String>,
    /// Period end for profit_and_loss and cash_flow, YYYY-MM-DD.
    pub end: Option<String>,
    /// As-of date for balance_sheet, YYYY-MM-DD.
    pub as_of: Option<String>,
    /// Cash account codes for cash_flow. Defaults to `[books]
    /// cash_accounts`; one or the other is required for that report.
    #[serde(default)]
    pub cash_accounts: Vec<String>,
    /// Liability account codes treated as financing for cash_flow.
    /// Defaults to `[books] financing_accounts`, else empty.
    #[serde(default)]
    pub financing_accounts: Vec<String>,
    /// Asset account codes treated as investing for cash_flow: loans the
    /// company makes, investments it buys (ASC 230-10-20). Defaults to
    /// `[books] investing_accounts`.
    #[serde(default)]
    pub investing_accounts: Vec<String>,
}

/// Result of `books.export`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ExportOut {
    /// The requested format.
    pub format: String,
    /// The requested report.
    pub report: String,
    /// The exported content. Base64-encoded when `content_encoding` is
    /// "base64" (XLSX, PDF); otherwise raw text.
    pub content: String,
    /// "base64" for XLSX and PDF, otherwise absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_encoding: Option<String>,
}

// ---------------------------------------------------------------------------
// Fixed assets
// ---------------------------------------------------------------------------

/// Parameters for `books.register_asset`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct RegisterAssetParams {
    /// What it is, e.g. "2023 Sprinter van".
    pub description: String,
    /// Purchase date, YYYY-MM-DD. Depreciation starts here.
    pub purchase_date: String,
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
    /// Omit to generate the next one ("FA-0001", from `[books]
    /// asset_tag_prefix` or the company's initials).
    pub tag: Option<String>,
    /// The manufacturer's serial number, if known.
    pub serial: Option<String>,
    /// The posted entry that put it on the books (its purchase, or a
    /// member's contribution of it); it must debit the cost account.
    pub acquisition_entry: Option<u64>,
    /// The federal tax basis rule (see books.set_asset_tax_basis): cost,
    /// carryover, or personal_use_conversion. Omit to record it later.
    pub tax_basis_rule: Option<String>,
    /// The contributing member's adjusted basis, for carryover and
    /// personal_use_conversion.
    pub contributor_basis: Option<String>,
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
    /// Corrected in-service date, YYYY-MM-DD. Omit to keep it.
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
    pub in_service: String,
    /// Its federal tax basis (follows the corrected cost under
    /// personal_use_conversion and cost).
    pub tax: Option<TaxBasisOut>,
}

/// Parameters for `books.set_asset_tax_basis`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetAssetTaxBasisParams {
    /// The asset: its id or tag.
    pub asset: String,
    /// "cost" (the company bought it: basis is its cost), "carryover" (a
    /// member contributed business property: the member's adjusted basis,
    /// IRC §723), or "personal_use_conversion" (a member contributed
    /// personal-use property: the lesser of fair market value and the
    /// member's adjusted basis, Treas. Reg. §1.168(i)-4(b)).
    pub rule: String,
    /// The contributing member's adjusted basis at contribution, decimal
    /// string (what they paid, less any depreciation they took). Required
    /// for carryover and personal_use_conversion; omit for cost.
    pub contributor_basis: Option<String>,
}

/// An asset's federal tax basis, as tools report it.
#[derive(Debug, Serialize, JsonSchema)]
pub struct TaxBasisOut {
    /// The rule: cost, carryover, or personal_use_conversion.
    pub rule: String,
    /// The contributing member's adjusted basis, decimal string, if a
    /// contribution.
    pub contributor_basis: Option<String>,
    /// The depreciable federal tax basis, decimal string.
    pub tax_basis: Option<String>,
    /// Built-in gain (+) or loss (−) at contribution (fair market value less
    /// the contributor's basis), IRC §704(c), decimal string.
    pub built_in_gain: Option<String>,
}

/// Result of `books.register_asset`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct RegisterAssetOut {
    /// The new asset's id.
    pub asset_id: u64,
    /// Its tag (given or generated).
    pub tag: String,
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
    /// A file name for the sheet.
    pub file_name: String,
    /// The label sheet as PDF, base64. Avery 5160/8160 (US letter, 30
    /// labels of 1" x 2-5/8"); print at 100% scale, not "fit to page".
    pub content_base64: String,
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
    /// Book value after this month, decimal string.
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

/// One asset in `books.list_assets`.
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
    /// Its federal tax basis, if recorded.
    pub tax: Option<TaxBasisOut>,
    /// Asset description.
    pub description: String,
    /// Purchase date, YYYY-MM-DD.
    pub purchase_date: String,
    /// Cost, decimal string.
    pub cost: String,
    /// Useful life in months.
    pub useful_life_months: u32,
    /// "active" or "disposed".
    pub status: String,
    /// Accumulated depreciation through today, decimal string.
    pub accumulated: String,
    /// Book value today, decimal string.
    pub book_value: String,
}

/// Result of `books.list_assets`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ListAssetsOut {
    /// All registered assets.
    pub assets: Vec<AssetSummary>,
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
    /// Amount posted, decimal string.
    pub amount: String,
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
    /// Assets skipped by a run-all because the period is outside their
    /// depreciable life.
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
    /// `[books] parties = "require"` a sale with proceeds needs one; a
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
    /// Depreciation auto-posted for the final partial period, decimal string
    /// ("0.00" when the period was already posted).
    pub final_depreciation: String,
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
}

/// Parameters for `books.create_invoice` / `books.create_bill`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateDocumentParams {
    /// Customer id (invoice) or vendor id (bill).
    pub party_id: u64,
    /// Document date, YYYY-MM-DD.
    pub date: String,
    /// Due date, YYYY-MM-DD.
    pub due_date: String,
    /// Line items.
    pub lines: Vec<DocumentLineParam>,
}

/// Output of `books.create_invoice` / `books.create_bill`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreateDocumentOut {
    /// The new document's id.
    pub id: u64,
    /// Total amount, decimal string.
    pub total: String,
    /// The auto-posted journal entry's id.
    pub entry_id: u64,
}

/// Parameters for `books.receive_payment` / `books.pay_bill`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PaymentParams {
    /// Invoice id (receive_payment) or bill id (pay_bill).
    pub document_id: u64,
    /// Payment date, YYYY-MM-DD.
    pub date: String,
    /// Payment amount, decimal string.
    pub amount: String,
    /// Cash account id or code.
    pub cash_account: String,
}

/// Output of `books.receive_payment` / `books.pay_bill`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PaymentOut {
    /// The payment journal entry's id.
    pub entry_id: u64,
    /// Remaining balance, decimal string.
    pub remaining: String,
    /// "open", "partial", or "paid".
    pub status: String,
}

/// Parameters for `books.ar_aging` / `books.ap_aging`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AgingParams {
    /// As-of date, YYYY-MM-DD.
    pub as_of: String,
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

/// Output of `books.ar_aging` / `books.ap_aging`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AgingOut {
    /// Rows, one per party with an outstanding balance.
    pub rows: Vec<AgingRowOut>,
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
}

/// Parameters for `books.remit_payroll_taxes`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RemitPayrollTaxesParams {
    /// Remittance date, YYYY-MM-DD.
    pub date: String,
    /// Cash account id or code the remittance comes from.
    pub cash_account: String,
    /// The tax agency paid (party id or name, e.g. the IRS), named on the
    /// remittance entry. Required under `[books] parties = "require"`.
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
    /// As-of date, YYYY-MM-DD. Defaults to today.
    pub as_of: Option<String>,
    /// Cash account codes the cash flow statement reconciles to, e.g.
    /// `["1000"]`. Defaults to `[books] cash_accounts`; one or the other
    /// is required.
    #[serde(default)]
    pub cash_accounts: Vec<String>,
    /// Liability account codes treated as financing. Defaults to `[books]
    /// financing_accounts`, else empty.
    #[serde(default)]
    pub financing_accounts: Vec<String>,
    /// Asset account codes treated as investing for cash_flow: loans the
    /// company makes, investments it buys (ASC 230-10-20). Defaults to
    /// `[books] investing_accounts`.
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
/// name, display_name, website, address, notes, archived.
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
    /// True: no longer dealt with (its history stays; it no longer
    /// resolves by name, and its name is free for another party). False:
    /// dealt with again (refused while another party has its name).
    pub archived: Option<bool>,
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
/// itself. Either `clear: true` alone, or every required fact.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
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
    /// books.begin_upload. Stored only in the vault (it shows the full
    /// TIN), never with the receipts and documents.
    #[serde(default)]
    pub w9_upload_id: Option<String>,
    /// …or, for one of at most 256 KiB, its bytes base64-encoded, with
    /// w9_sha256.
    #[serde(default)]
    pub w9_content_base64: Option<String>,
    /// The SHA-256 of the bytes in w9_content_base64, hex.
    #[serde(default)]
    pub w9_sha256: Option<String>,
    /// The W-9 file's name (default: the upload's, else w9.pdf, w9.png
    /// or w9.jpg by its type).
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
    /// The command that saves it in a folder you name (run it; books
    /// served by `wanderbooks mcp`). Kept 15 minutes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download: Option<String>,
    /// When it lapses (UTC), with `download`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    /// Over stdio: its bytes, base64.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_base64: Option<String>,
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
    /// Calendar year for `paid` and `received` (default: this year, so
    /// year to date).
    pub year: Option<i32>,
}

/// Parameters for `books.party`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct PartyParams {
    /// The party: id or name.
    pub party: String,
    /// Calendar year for the totals (default: this year, so year to date).
    pub year: Option<i32>,
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
    /// why (journaled as `entry.no_party`). Satisfies `[books] parties =
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
    /// The W-9's TIN last four digits, when on file.
    pub tin_last4: Option<String>,
    /// W-4 facts are on file.
    pub w4_on_file: bool,
    /// No longer dealt with.
    pub archived: bool,
    /// Paid to the party in the year, decimal string (as `books.party`
    /// totals).
    pub paid: String,
    /// Received from the party in the year, decimal string (as
    /// `books.party` totals).
    pub received: String,
}

/// Result of `books.parties`.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ListPartiesOut {
    /// The year `paid` and `received` cover.
    pub year: i32,
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
/// Each entry's money is `ledger_core::parties::entry_flow`: what left or
/// reached the `[books] cash_accounts` (every asset account but
/// receivables when none are set), plus what the company paid through a
/// liability (a card, a member paying for it, a loan; not payables or
/// payroll withholdings), with cash that pays a liability down counting
/// once, as paid to that liability's creditor (a reimbursement to the
/// members, a card payment to the issuer), a liability paid down by
/// another counting the same way (a member paying the card from their own
/// funds, a loan paying it off), and cash raised by a liability counting
/// as received from the lender. Positive is paid to the party, negative
/// received from it; a reversal counts against its root original's side
/// (reversing the reversal counts on that side again). A transfer between the company's own cash accounts nets to zero.
/// A bill is not paid until its payment is.
/// An entry naming several parties counts each one's share of that money
/// (its amount, its percent, or its derived share: see
/// `books.set_entry_party` `parties`); an entry whose shares don't resolve
/// counts for none of them.
/// (`ledger_core::parties::activity_with`, with the `[[company.members]]`,
/// computes it; the viewer shows the same numbers.)
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PartyTotalsOut {
    /// Paid to the party, decimal string.
    pub paid: String,
    /// Received from the party, decimal string.
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
    /// For a party with a vendor record: the cash `books.vendor_1099`
    /// counts as paid to that vendor in the year (asset accounts only, so
    /// card payments drop out), decimal string. Absent otherwise.
    pub amount_1099: Option<String>,
    /// The information return its vendor record gets ("1099-NEC",
    /// "1099-MISC"), if any.
    pub form_1099: Option<String>,
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
    /// entry, account, asset, party, invoice, bill, period, payroll_run,
    /// schedule, reconciliation, or company.
    pub kind: String,
    /// Which one: an entry, invoice, bill, period, payroll run, schedule,
    /// or reconciliation id; an account's code (or id); an asset's tag (or
    /// id); a party's id or name. Omitted for the company.
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
    /// The section to read: contract, entries, drafts-and-reversals, evidence-and-receipts, attachments, files-in-out, reconciliation, fixed-assets, prepaid-and-recurring, revenue-and-payouts, parties-and-tins, 1099, cpa-handoff, settings-and-conventions, viewer, backups, rules-that-change, control-roles, tracing-figures, or "all" for the whole handbook in order. Omit for the index.
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
