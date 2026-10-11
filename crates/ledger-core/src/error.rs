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
}
