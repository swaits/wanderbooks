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

    /// The receipt policy covers the entry and it has no receipt (plan P4,
    /// I-5; rules.md B-43): refused before anything is journaled.
    #[error(
        "entry {} needs a receipt before it posts: {} ({}); attach one with books.attach_receipt",
        .0.entry_id, .0.reason, .0.policy
    )]
    ReceiptRequired(Box<ReceiptRequiredInfo>),

    /// Filesystem error.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// SQLite error.
    #[error("database error: {0}")]
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

    /// A write that needs a journal format or feature these books haven't
    /// declared: they were written by an older version, and wait for the
    /// owner's go-ahead to move to this version's format (the format
    /// upgrade: plan/served.md 3.9, D-39). Nothing was written; everything
    /// that needs nothing new still works.
    #[error("{}", format_upgrade_text(need))]
    FormatUpgradePending {
        /// What the write needs: "journal format 2", "the journal
        /// feature x".
        need: String,
    },

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

    /// A write-off with no evidence (rules.md B-41 INVARIANT 1): a bad
    /// debt is deducted on evidence that it is worthless (Treas. Reg.
    /// §1.166-2), so the books take the write-off only with its file.
    #[error(
        "a write-off needs its evidence: file the collection correspondence or the bankruptcy \
         notice with upload_id, or name an attachment already filed with existing_attachment_id \
         (Treas. Reg. §1.166-2: worthlessness on all the pertinent evidence)"
    )]
    WriteOffNeedsEvidence,

    /// The requested base currency differs from the store's.
    #[error("base currency mismatch: store is {0}, requested {1}")]
    CurrencyMismatch(String, String),
}

/// The guard code a refusal for books waiting for their format upgrade
/// carries (plan/served.md 3.9: `format_upgrade_pending`).
pub const FORMAT_UPGRADE_PENDING: &str = "format_upgrade_pending";

/// What [`StoreError::FormatUpgradePending`] says: what the write needs,
/// that nothing was changed, that only the owner can give the go-ahead
/// (and where), and that the rest still works.
fn format_upgrade_text(need: &str) -> String {
    format!(
        "{FORMAT_UPGRADE_PENDING}: this needs {need}, and these books are still in the journal \
         format an older version of wanderbooks wrote: they are waiting for the owner's \
         go-ahead to upgrade to this version's format. Nothing was changed. Only the owner can \
         give it, with the owner passphrase: in the app (it asks when it opens the books) or \
         with `wanderbooks books upgrade ID` at a terminal; no tool can. Until then everything \
         that needs nothing new works as before"
    )
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

/// What a refused post tells (boxed in [`StoreError::ReceiptRequired`]).
#[derive(Debug)]
pub struct ReceiptRequiredInfo {
    /// The entry that needs the receipt.
    pub entry_id: u64,
    /// Its total debits, `None` when they pass exact arithmetic.
    pub debits: Option<rust_decimal::Decimal>,
    /// The policy that triggered, in words.
    pub policy: String,
    /// Why this entry is covered.
    pub reason: String,
    /// The posted entries it appears to split (anti-splitting).
    pub split_group: Vec<u64>,
}
