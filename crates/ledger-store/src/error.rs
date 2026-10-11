//! Error type for the store.

use thiserror::Error;

/// Every way a store operation can fail.
#[derive(Debug, Error)]
pub enum StoreError {
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

    /// The store's schema is newer than this binary understands.
    #[error("schema version {0} newer than supported {1}")]
    SchemaTooNew(u32, u32),

    /// `create` called where a store already exists.
    #[error("store already exists at {0}")]
    AlreadyExists(String),

    /// `open` called where no store exists.
    #[error("store not found at {0}")]
    NotFound(String),

    /// The requested base currency differs from the store's.
    #[error("base currency mismatch: store is {0}, requested {1}")]
    CurrencyMismatch(String, String),
}
