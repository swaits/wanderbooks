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

    /// Cash-flow period end is before period start.
    #[error("invalid date range: end {end} is before start {start}")]
    InvalidDateRange {
        /// Period start, YYYY-MM-DD.
        start: String,
        /// Period end, YYYY-MM-DD.
        end: String,
    },
}
