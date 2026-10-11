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

/// Result of `books.trial_balance`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct TrialBalanceOut {
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
    pub amount: String,
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

/// Parameters for `books.list_receipts`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListReceiptsParams {
    /// Filter to one entry. Omit for all.
    pub entry_id: Option<u64>,
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
    /// "csv" or "json".
    pub format: String,
    /// "journal", "trial_balance", "profit_and_loss", or "balance_sheet".
    pub report: String,
    /// Period start for profit_and_loss, YYYY-MM-DD.
    pub start: Option<String>,
    /// Period end for profit_and_loss, YYYY-MM-DD.
    pub end: Option<String>,
    /// As-of date for balance_sheet, YYYY-MM-DD.
    pub as_of: Option<String>,
}

/// Result of `books.export`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ExportOut {
    /// The requested format.
    pub format: String,
    /// The requested report.
    pub report: String,
    /// The exported content.
    pub content: String,
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
