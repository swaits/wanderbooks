//! `ledger-store`: durable persistence for the books.
//!
//! Three pieces, one job — keep the books safe:
//!
//! * [`journal`] — the source of truth. An append-only, hash-chained JSONL
//!   journal. Every state change is an immutable fact, fsync'd on append.
//! * [`blob`] — the bytes. Content-addressed (SHA-256), immutable,
//!   deduplicated receipt and document storage.
//! * [`Store`] — the SQLite projection plus the write path. SQLite is the
//!   query engine and is disposable by design: [`Store::rebuild`] drops it
//!   and replays the journal. [`Store::verify`] replays the journal into a
//!   fresh engine and diffs its reports against both the live ledger and
//!   the projection.
//!
//! The write path is singular: **validate → journal → mutate → apply**.
//! Journal first means a failed write leaves nothing behind; a failed
//! SQLite apply leaves the journal ahead, and the next open heals it via
//! `last_applied_seq`. There is exactly one writer: `Store` is `!Sync` by
//! construction (it owns a `rusqlite::Connection`).

pub mod access;
pub mod auth;
pub mod backup;
pub mod blob;
pub mod chain;
pub mod error;
pub mod exports;
pub mod form1099;
pub mod format;
pub mod journal;
pub mod meta;
pub mod ops;
pub mod passphrase;
pub mod reconcilable;
#[cfg(test)]
mod refuse;
pub mod root;
pub mod saving;
pub mod schema;
pub mod snapshot;
pub mod store;
pub mod uploads;

pub use blob::BlobStore;
pub use error::StoreError;
pub use format::{JournalFormat, SUPPORTED_JOURNAL_FORMAT};
pub use journal::{GENESIS_PREV, JournalLine, JournalWriter};
pub use snapshot::{DiskCheck, Snapshot};
pub use store::{
    Attachment, DB_FILE, DepreciationRun, DisposalPreview, EntryRow, EventRow, JOURNAL_FILE,
    LOCK_FILE, MAX_RECURRING_RUN, PayrollLineInput, Reconciliation, Statement, Store, Subject,
    VerifyReport,
};
