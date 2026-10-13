//! Error type for the ledger engine.

use rust_decimal::Decimal;
use thiserror::Error;

/// Every way a ledger command can fail.
///
/// Variants are part of the engine's contract: agents adding features add
/// variants here rather than inventing ad-hoc error reporting.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum LedgerError {
    /// A journal entry must have at least two lines (one debit, one credit).
    #[error("journal entry has fewer than two lines")]
    TooFewLines,

    /// Debits and credits must be exactly equal. No epsilon, ever.
    #[error("journal entry does not balance: debits {debits} != credits {credits}")]
    UnbalancedEntry {
        /// Sum of debit lines.
        debits: Decimal,
        /// Sum of credit lines.
        credits: Decimal,
    },

    /// Zero-amount lines are noise; they are rejected, not ignored.
    #[error("journal line has a zero amount")]
    ZeroAmountLine,

    /// Line amounts are positive; the side says which way the money moves.
    #[error("journal line has a negative amount; use the other side instead")]
    NegativeAmountLine,

    /// A line larger than one quadrillion, or with more than ten decimal
    /// places, is a mistake, and one that could overflow exact arithmetic.
    #[error("journal line amount is out of range (at most 1000000000000000, ten decimal places)")]
    AmountOutOfRange,

    /// The referenced account does not exist in the chart of accounts.
    #[error("unknown account id {0}")]
    UnknownAccount(u64),

    /// Placeholder (header) accounts organize the chart; they take no postings.
    #[error("account '{0}' is a placeholder and cannot take postings")]
    PlaceholderAccount(String),

    /// All lines of an entry (and their accounts) must share one currency.
    #[error("currency mismatch: expected {expected}, found {found}")]
    CurrencyMismatch {
        /// The currency that was expected.
        expected: String,
        /// The currency that was found.
        found: String,
    },

    /// Currency codes are three ASCII letters (ISO 4217 alpha-3).
    #[error("invalid currency code '{0}': must be three ASCII letters")]
    InvalidCurrencyCode(String),

    /// Posted entries are immutable. This is load-bearing: it is what makes
    /// the ledger an audit trail instead of a spreadsheet.
    #[error("entry {0} is already posted and immutable")]
    EntryAlreadyPosted(u64),

    /// Only posted entries can be reversed.
    #[error("entry {0} is not posted yet")]
    EntryNotPosted(u64),

    /// The referenced journal entry does not exist.
    #[error("entry {0} does not exist")]
    UnknownEntry(u64),

    /// Account codes must be unique within a chart of accounts.
    #[error("account code '{0}' is already in use")]
    DuplicateAccountCode(String),

    /// Account codes must not be empty.
    #[error("account code must not be empty")]
    EmptyAccountCode,

    /// The referenced parent account does not exist.
    #[error("unknown parent account id {0}")]
    UnknownParentAccount(u64),

    /// Any attempt to mutate a posted entry: add lines, delete it, edit it.
    #[error("cannot modify a posted entry; post a reversing entry instead")]
    PostedEntryImmutable,

    /// The referenced fixed asset does not exist.
    #[error("unknown asset id {0}")]
    UnknownAsset(u64),

    /// Asset descriptions must not be empty.
    #[error("asset description must not be empty")]
    EmptyAssetDescription,

    /// Asset cost must be positive.
    #[error("asset cost must be positive, got {0}")]
    InvalidAssetCost(Decimal),

    /// Salvage value must be non-negative and less than cost.
    #[error("salvage value must be between zero and cost, got {0}")]
    InvalidSalvageValue(Decimal),

    /// Useful life must be at least one month.
    #[error("useful life must be at least one month, got {0}")]
    InvalidUsefulLife(u32),

    /// The asset was already disposed.
    #[error("asset {0} is already disposed")]
    AssetAlreadyDisposed(u64),

    /// Asset account links must have the right types: cost and accumulated
    /// depreciation are asset accounts, the depreciation expense account is
    /// an expense account.
    #[error("account '{code}' has type {found}, but the asset {role} account must be {expected}")]
    AssetAccountWrongType {
        /// Account code.
        code: String,
        /// Role the account was given: "cost", "accumulated depreciation", or
        /// "depreciation expense".
        role: &'static str,
        /// The type it must be.
        expected: &'static str,
        /// The type it has.
        found: &'static str,
    },

    /// Disposal proceeds must not be negative.
    #[error("disposal proceeds must not be negative, got {0}")]
    NegativeProceeds(Decimal),

    /// Disposal date must not precede the purchase date.
    #[error("disposal date {0} is before the purchase date")]
    DisposalBeforePurchase(String),

    /// The three asset account links must be three distinct accounts.
    #[error("asset account links must be distinct; account id {0} is linked twice")]
    DuplicateAssetAccount(u64),

    /// purchase_date + useful life overflows the date range.
    #[error("purchase date plus useful life overflows the calendar")]
    AssetDateOverflow,

    /// An amortization schedule or recurring entry is invalid, unknown, or
    /// already posted for that date.
    #[error("invalid schedule: {0}")]
    InvalidSchedule(String),

    /// A payout doesn't reconcile or uses the wrong accounts.
    #[error("invalid payout: {0}")]
    InvalidPayout(String),

    /// A tax category doesn't exist, doesn't fit the account, or has a bad
    /// deductible percent.
    #[error("invalid tax category: {0}")]
    InvalidTaxCategory(String),

    /// Partnership members are misconfigured (shares, capital accounts).
    #[error("invalid members: {0}")]
    InvalidMembers(String),

    /// Cash-flow period end is before period start.
    #[error("invalid date range: end {end} is before start {start}")]
    InvalidDateRange {
        /// Period start, YYYY-MM-DD.
        start: String,
        /// Period end, YYYY-MM-DD.
        end: String,
    },

    /// The referenced accounting period does not exist.
    #[error("unknown period id {0}")]
    UnknownPeriod(u64),

    /// The accounting period is already closed.
    #[error("period {0} is already closed")]
    PeriodAlreadyClosed(u64),

    /// Period end is before period start.
    #[error("invalid period: end {end} is before start {start}")]
    InvalidPeriodRange {
        /// Period start, YYYY-MM-DD.
        start: String,
        /// Period end, YYYY-MM-DD.
        end: String,
    },

    /// A draft entry exists with a date inside the period being closed.
    #[error("cannot close period {0}: draft entries exist with dates in the period")]
    DraftsInPeriod(u64),

    /// A period can't close while a later period is already closed.
    #[error("period {0} can't close: later period {1} is already closed (close in order)")]
    CloseOutOfOrder(u64, u64),

    /// Entries cannot be created or posted in a closed period.
    #[error("period {0} is closed; entries dated {1} are rejected")]
    ClosedPeriod(u64, String),

    /// The referenced customer does not exist.
    #[error("unknown customer id {0}")]
    UnknownCustomer(u64),

    /// The referenced vendor does not exist.
    #[error("unknown vendor id {0}")]
    UnknownVendor(u64),

    /// The referenced invoice does not exist.
    #[error("unknown invoice id {0}")]
    UnknownInvoice(u64),

    /// The referenced bill does not exist.
    #[error("unknown bill id {0}")]
    UnknownBill(u64),

    /// Invoice/bill must have at least one line.
    #[error("invoice/bill must have at least one line")]
    EmptyDocumentLines,

    /// Invoice/bill line amounts must be positive.
    #[error("document line amount must be positive, got {0}")]
    InvalidDocumentLineAmount(Decimal),

    /// Payment amount must be positive.
    #[error("payment amount must be positive, got {0}")]
    InvalidPaymentAmount(Decimal),

    /// Payment exceeds the remaining balance.
    #[error("payment of {0} exceeds remaining balance of {1}")]
    Overpayment(Decimal, Decimal),

    /// Invoice/bill due date is before its date.
    #[error("due date {0} is before document date")]
    DueBeforeDate(String),

    /// Line index out of range for cleared-flag update.
    #[error("line index {0} out of range for entry {1}")]
    LineIndexOutOfRange(usize, u64),

    /// Cannot set a cleared flag on a draft entry's line.
    #[error("cannot mark cleared on draft entry {0}; only posted lines can clear")]
    ClearedOnDraft(u64),

    /// FX rate must be positive.
    #[error("fx rate must be positive, got {0}")]
    InvalidFxRate(Decimal),

    /// FX line requires both foreign currency and foreign amount.
    #[error("fx line requires foreign currency, foreign amount, and fx rate")]
    IncompleteFxLine,

    /// Replay recorded an id the allocator was not expecting: the journal
    /// and the engine disagree about event ordering.
    #[error("replay id mismatch: journal says {0}, allocator expects {1}")]
    ReplayIdMismatch(u64, u64),

    /// A payroll run must have at least one employee line.
    #[error("payroll run must have at least one line")]
    EmptyPayroll,

    /// Payroll amounts must not be negative.
    #[error("payroll amount must not be negative, got {0}")]
    NegativePayrollAmount(Decimal),

    /// Net pay went negative: withholdings exceed gross for this employee.
    #[error("payroll net pay for '{0}' is negative")]
    NegativeNetPay(String),

    /// No payroll tax payables are outstanding: nothing to remit.
    #[error("no payroll tax payables to remit")]
    NothingToRemit,

    /// The referenced payroll run does not exist.
    #[error("unknown payroll run id {0}")]
    UnknownPayrollRun(u64),

    /// Class names must not be empty.
    #[error("class name must not be empty")]
    EmptyClassName,

    /// The referenced class does not exist.
    #[error("unknown class id {0}")]
    UnknownClass(u32),
}
