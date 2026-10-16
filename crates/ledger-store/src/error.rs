//! Error type for the store.

use thiserror::Error;

/// Every way a store operation can fail.
#[derive(Debug, Error)]
pub enum StoreError {
    /// An attachment link names a subject that doesn't exist.
    #[error("no {0} to link to")]
    UnknownSubject(String),
    /// The attachment is already linked to that subject.
    #[error("attachment {0} is already linked to {1}")]
    AlreadyLinked(u64, String),
    /// The attachment isn't linked to that subject.
    #[error("attachment {0} isn't linked to {1}")]
    NotLinkedTo(u64, String),
    /// A statement record that doesn't fit its account or itself.
    #[error("bad statement: {0}")]
    InvalidStatement(String),
    /// The books don't agree with the statement yet.
    #[error(
        "not reconciled: the cleared balance differs from the statement by {difference}; \
         clear the lines the statement shows (or find what's missing) first"
    )]
    NotReconciled {
        /// Cleared balance less the statement balance.
        difference: rust_decimal::Decimal,
    },
    /// A reconciliation command that breaks the sequence.
    #[error("{0}")]
    ReconciliationRefused(String),

    /// An asset can't be corrected once depreciation for it is posted: the
    /// posted entries would disagree with the corrected schedule.
    #[error("asset {0} already has depreciation posted; correct it with entries instead")]
    AssetDepreciationPosted(u64),

    /// Filesystem error.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// SQLite error.
    #[error("sqlite error: {0}")]
    Rusqlite(#[from] rusqlite::Error),

    /// JSON error.
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    /// The engine rejected the operation. The journal is untouched.
    #[error("ledger error: {0}")]
    Ledger(#[from] ledger_core::LedgerError),

    /// The journal failed integrity verification.
    #[error("journal corrupt: {0}")]
    JournalCorrupt(String),

    /// A whole, correctly chained journal line this version can't read:
    /// a newer wanderbooks wrote the books. When the line after it chains
    /// onto it (`verified`), its own bytes are checked: it is intact,
    /// though the lines after it are the newer version's to check. When
    /// nothing checks it (it is the last line, or the next has no `prev`
    /// this version can read), it is either newer or damaged, and the
    /// message says so (rules.md B-27).
    #[error("{}", newer_journal_text(*seq, event_type, need, *verified))]
    NewerJournal {
        /// The first line this version can't read.
        seq: u64,
        /// Its type.
        event_type: String,
        /// What reading it needs: a journal format, features, or an event
        /// type this version doesn't know.
        need: String,
        /// Whether the line's own hash was checked (the next line's `prev`
        /// matches it). False when nothing checks it: a damaged line would
        /// read the same.
        verified: bool,
    },

    /// A journal format this version can't declare: newer than it reads,
    /// lower than the books already use, or with a feature it doesn't know.
    #[error("{0}")]
    FormatRefused(String),

    /// Replay produced different ids than the journal recorded.
    #[error("replay mismatch: {0}")]
    ReplayMismatch(String),

    /// [`crate::Store::verify`] found divergence.
    #[error("verify failed: {0}")]
    VerifyMismatch(String),

    /// A blob's bytes did not match its SHA-256.
    #[error("blob corrupt: hash mismatch for {0}")]
    CorruptBlob(String),

    /// Referenced attachment does not exist.
    #[error("unknown attachment {0}")]
    UnknownAttachment(u64),

    /// The attachment is not linked to any entry.
    #[error("attachment {0} is not linked to an entry")]
    AttachmentNotLinked(u64),

    /// The attachment is linked to an entry; unlink it first.
    #[error("attachment {0} is linked to entry {1}; unlink it first with books.unlink_receipt")]
    AttachmentLinked(u64, u64),

    /// The attachment was withdrawn and can no longer be linked or changed.
    #[error("attachment {0} is withdrawn")]
    AttachmentWithdrawn(u64),

    /// A correction to the evidence trail needs a reason.
    #[error("a non-empty reason is required")]
    ReasonRequired,

    /// The store's schema is newer than this binary understands.
    #[error("schema version {0} newer than supported {1}")]
    SchemaTooNew(u32, u32),

    /// `create` called where a store already exists.
    #[error("store already exists at {0}")]
    AlreadyExists(String),

    /// Another process has these books open for writing. One writer at a
    /// time: two would each extend the hash chain from the same place and
    /// break it (found by the max-testing pass, 2026-10-13).
    #[error("{0}")]
    Locked(String),

    /// `open` called where no store exists.
    #[error("store not found at {0}")]
    NotFound(String),

    /// The requested base currency differs from the store's.
    #[error("base currency mismatch: store is {0}, requested {1}")]
    CurrencyMismatch(String, String),
}

/// What [`StoreError::NewerJournal`] says. Only a line whose own hash was
/// checked lets it say that line is intact; it never vouches for the
/// lines after it, which this version can't read.
fn newer_journal_text(seq: u64, event_type: &str, need: &str, verified: bool) -> String {
    let format = crate::format::SUPPORTED_JOURNAL_FORMAT;
    if verified {
        format!(
            "these books were written by a newer version of wanderbooks: journal line {seq} \
             ({event_type}) needs {need}, and this version reads journal format {format}. \
             Update wanderbooks to open them. Line {seq} is intact (the line after it checks \
             it); this version can't check the lines after it"
        )
    } else {
        format!(
            "journal line {seq} ({event_type}) needs {need}, and this version reads journal \
             format {format}. Nothing this version can read checks that line (it is the last \
             line, or the line after it isn't one this version reads): either a newer version \
             of wanderbooks wrote it, or the line is damaged. Update wanderbooks, then run \
             verify to tell which"
        )
    }
}
