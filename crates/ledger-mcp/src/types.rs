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
    /// Not yet recognized, decimal string.
    pub remaining: String,
    /// Every month.
    pub months: Vec<ScheduleMonthOut>,
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

/// Result of `books.run_recurring`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct RunRecurringOut {
    /// Drafts created.
    pub drafts: Vec<RecurringDraftOut>,
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
    /// vendor_1099, ar_aging, ap_aging.
    pub report: Option<String>,
    /// A stored receipt or document (attachment id).
    pub attachment_id: Option<u64>,
    /// Search the transactions list (description, account, vendor, amount).
    pub search: Option<String>,
    /// A page: overview, cash, profit, transactions, reports, accounts,
    /// documents, company, journal.
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
    /// Debit balance as a decimal string, or "0".
    pub debit: String,
    /// Credit balance as a decimal string, or "0".
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
    /// Account codes treated as cash (e.g. `["1000", "1010"]`). Required:
    /// the statement reconciles to the net change in these accounts.
    pub cash_accounts: Vec<String>,
    /// Account codes for liabilities treated as financing (debt).
    /// Defaults to empty: liabilities are operating working capital.
    #[serde(default)]
    pub financing_accounts: Vec<String>,
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
    /// The CSV text (QBO bank upload layout: Date,Description,Amount or
    /// Date,Description,Expense,Income).
    pub csv_text: String,
    /// Account code for the bank account (the offset side of each entry).
    pub bank_account_code: String,
    /// Account code for the default offset (e.g. an expense account).
    /// Used when no rule matches.
    pub default_offset_code: String,
    /// Optional categorization rules: { "pattern": "account_code" }.
    /// First case-insensitive substring match on the description wins.
    #[serde(default)]
    pub rules: std::collections::HashMap<String, String>,
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
}

/// Parameters for `books.import_chart_csv`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ImportChartCsvParams {
    /// The CSV text: code,name,type,parent_code with a header row.
    pub csv_text: String,
}

/// Result of `books.import_chart_csv`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ImportChartCsvOut {
    /// Account codes created, in order.
    pub created: Vec<String>,
    /// Number of accounts created.
    pub count: usize,
}

/// Parameters for `books.import_local_file`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ImportLocalFileParams {
    /// Absolute path to the file on the server's filesystem.
    pub path: String,
    /// "receipt" or "document".
    pub category: String,
    /// Entry id to attach to (receipts only). Omit to file unlinked.
    #[serde(default)]
    pub entry_id: Option<u64>,
    /// Document category for documents: formation, legal, tax, accounting, other.
    #[serde(default)]
    pub document_category: Option<String>,
    /// Override the file name (defaults to the path's file name).
    #[serde(default)]
    pub file_name: Option<String>,
    /// Optional description.
    #[serde(default)]
    pub description: Option<String>,
}

/// Result of `books.import_local_file`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ImportLocalFileOut {
    /// Attachment id.
    pub attachment_id: u64,
    /// SHA-256 hex of the content.
    pub sha256: String,
    /// True if the content was already stored (deduplicated).
    pub deduplicated: bool,
    /// File name used.
    pub file_name: String,
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
    /// Original file name, e.g. "receipt.png".
    pub file_name: String,
    /// MIME type, e.g. "image/png".
    pub mime_type: String,
    /// File bytes, base64-encoded. Max 25 MB.
    pub content_base64: String,
    /// Optional description.
    pub description: Option<String>,
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
}

/// Parameters for `books.file_document`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct FileDocumentParams {
    /// One of: formation, legal, tax, accounting, other.
    pub category: String,
    /// Original file name.
    pub file_name: String,
    /// MIME type.
    pub mime_type: String,
    /// File bytes, base64-encoded. Max 25 MB.
    pub content_base64: String,
    /// Optional description.
    pub description: Option<String>,
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
}

/// Result of `books.get_receipt`: metadata plus base64 bytes. The bytes are
/// hash-verified on read.
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
    /// File bytes, base64-encoded.
    pub content_base64: String,
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
    /// Total debits, decimal string.
    pub total: String,
    /// Attached receipts.
    pub receipts: Vec<PreviewReceipt>,
    /// The vendor the entry is tagged with, for 1099 tracking.
    pub vendor: Option<String>,
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
    /// Cash account codes for cash_flow (required for that report).
    #[serde(default)]
    pub cash_accounts: Vec<String>,
    /// Liability account codes treated as financing (debt) for cash_flow.
    /// Defaults to empty: liabilities are operating working capital.
    #[serde(default)]
    pub financing_accounts: Vec<String>,
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
}

/// Result of `books.register_asset`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct RegisterAssetOut {
    /// The new asset's id.
    pub asset_id: u64,
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
    /// Asset id.
    pub asset_id: u64,
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
    /// Asset id.
    pub asset_id: u64,
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
    /// ("0" when the period was already posted).
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

/// Parameters for `books.reconcile`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReconcileParams {
    /// Account id or code (e.g. "1000") to reconcile.
    pub account: String,
    /// Statement date, YYYY-MM-DD.
    pub statement_date: String,
    /// Statement balance, decimal string.
    pub statement_balance: String,
}

/// One uncleared line in the reconciliation report.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UnclearedLineOut {
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
    /// Account code.
    pub account_code: String,
    /// Book balance (all posted lines), decimal string.
    pub book_balance: String,
    /// Cleared balance (cleared lines on/before statement date), decimal string.
    pub cleared_balance: String,
    /// Statement balance, decimal string.
    pub statement_balance: String,
    /// cleared_balance - statement_balance. Zero means reconciled.
    pub difference: String,
    /// Uncleared lines on/before the statement date.
    pub uncleared_lines: Vec<UnclearedLineOut>,
}

// ---------------------------------------------------------------------------
// A/R and A/P
// ---------------------------------------------------------------------------

/// Parameters for `books.add_customer` / `books.add_vendor`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AddPartyParams {
    /// Customer or vendor name.
    pub name: String,
}

/// Output of `books.add_customer` / `books.add_vendor`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AddPartyOut {
    /// The new party's id.
    pub id: u64,
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
    /// Employee name.
    pub employee_name: String,
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
    /// Cash account codes the cash flow statement reconciles to (required),
    /// e.g. `["1000"]`.
    #[serde(default)]
    pub cash_accounts: Vec<String>,
    /// Liability account codes treated as financing (debt). Defaults to
    /// empty: liabilities are operating working capital.
    #[serde(default)]
    pub financing_accounts: Vec<String>,
    /// Include every receipt for the year's entries and the company's
    /// documents (default true).
    #[serde(default = "default_true")]
    pub include_receipts: bool,
}

/// Output of `books.export_cpa_package`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExportCpaPackageOut {
    /// File name, e.g. "cpa-package-2026-10-11.zip".
    pub file_name: String,
    /// The zip, base64-encoded.
    pub content_base64: String,
}
