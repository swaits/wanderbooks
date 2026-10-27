//! [`Store`]: the write path, the SQLite projection, and maintenance.
//!
//! Write-path order for every mutation is **validate → journal → mutate →
//! apply**. Journaling before mutating means a failed write leaves nothing
//! behind: the journal is the source of truth, the in-memory [`Ledger`] is
//! the validation authority, and SQLite is the queryable projection that
//! the next open heals via `last_applied_seq`.
//!
//! ```text
//! GOAL: every change to the books is one durable, ordered, verifiable fact,
//!       or nothing at all.
//! SOURCE: R-11 (append-only log: "The log plus the chart fully determine
//!         every report"); R-12 (one command surface); docs/receipt-store.md
//!         "Write path" ("append to the journal, fsync, then apply to
//!         SQLite"); IRS Pub. 583 ("Your books must show your gross income,
//!         as well as your deductions and credits"; keep "supporting
//!         documents"); Rev. Proc. 98-25 §5.01 (machine-sensible records
//!         with an audit trail, "capable of being processed").
//! INVARIANTS (every command method of Store):
//!   1. A refused command MUST leave the journal byte-identical, and the
//!      ledger and projection unchanged (rehearsed on a clone first).
//!   2. An accepted command MUST be journaled (appended and synced) before
//!      the in-memory ledger changes, and applied to SQLite after.
//!   3. The journal MUST hold only whole lines, gapless `seq`, each `prev`
//!      the SHA-256 of the line before (genesis: 64 zeros).
//!   4. Replaying the journal MUST rebuild exactly the live ledger, every
//!      part of it, not only its balances (Store::verify).
//!   5. A command made of several journaled steps (a disposal, a recurring
//!      run) MUST be rehearsed whole, so a refusal anywhere writes nothing.
//!   6. NEVER delete or rewrite a journal line or a stored file.
//! ```

use crate::blob::BlobStore;
use crate::error::StoreError;
use crate::format::JournalFormat;
use crate::journal::{
    GENESIS_PREV, JournalLine, JournalWriter, read_and_verify_journal, sha256_hex,
};
use crate::schema;
use chrono::{Datelike, Months, NaiveDate};
use ledger_core::{
    AccountId, AccountType, AssetId, AssetStatus, BillId, Class, ClassId, CloseTo, Currency,
    CustomerId, DocumentLine, EntryId, EntryStatus, InvoiceId, Ledger, LedgerError, Money,
    NewAsset, PayrollLine, PayrollRun, PayrollRunId, PeriodId, Side, VendorId, arap,
    payroll::{PayrollAccounts, payroll_entry_plan, payroll_remit_plan},
    reports,
};
use rusqlite::{Connection, OptionalExtension, params};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// File names inside the data directory.
pub const JOURNAL_FILE: &str = "journal.jsonl";

/// The projection database's one file before each schema had a file of
/// its own (every version through P2, whose schema was 12). This version
/// uses it only when it already holds this version's schema, so books that
/// need no migration aren't rebuilt for nothing; otherwise it is left as
/// it is, byte for byte, for the version that wrote it ([`db_file_for`]).
pub const LEGACY_DB_FILE: &str = "wanderbooks.db";

/// The projection database's file for schema `schema`:
/// `wanderbooks.v{schema}.db`.
///
/// ```text
/// GOAL: going back to an older version always finds the books it can
///       open (plan/served.md 3.9: before consent, the way back is always
///       there), however far this version's projection has moved.
/// SOURCE: plan/served.md 3.9, the lead's decision (2026-10-21: each
///         schema in a file of its own); rules.md B-27.
/// INVARIANTS:
///   1. A version opens or creates only its own schema's file (or the
///      legacy file when that already holds its schema); a missing one is
///      built from the journal.
///   2. It never migrates, writes or deletes a file of another schema,
///      older or newer: each is left byte for byte for its own version.
///   3. The name depends only on the schema number, so versions agree on
///      it whatever number each was given.
/// ```
pub fn db_file_for(schema: u32) -> String {
    format!("wanderbooks.v{schema}.db")
}

/// This version's own projection file: [`db_file_for`] its schema
/// ([`crate::schema::SCHEMA_VERSION`]).
pub fn db_file() -> String {
    db_file_for(crate::schema::SCHEMA_VERSION)
}

/// Whether `name` is a projection database's file, of any schema (the
/// legacy one included): what [`Store::other_databases`] lists.
pub fn is_database_file(name: &str) -> bool {
    name == LEGACY_DB_FILE
        || name
            .strip_prefix("wanderbooks.v")
            .and_then(|rest| rest.strip_suffix(".db"))
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// The projection database this version uses for the books in `data_dir`
/// ([`db_file_for`] INVARIANT 1): its own schema's file if there is one;
/// else the legacy file if that already holds this version's schema; else
/// its own schema's file, to be built from the journal. Read only: a file
/// of another schema is looked at, never opened for writing.
pub fn database_in_use(data_dir: &Path) -> PathBuf {
    let own = data_dir.join(db_file());
    if std::fs::symlink_metadata(&own).is_ok() {
        return own;
    }
    let legacy = data_dir.join(LEGACY_DB_FILE);
    if schema_of(&legacy) == Some(crate::schema::SCHEMA_VERSION) {
        legacy
    } else {
        own
    }
}

/// The schema a projection file says it holds, read without writing a
/// byte of it (an immutable read when no writer has it open; None when it
/// isn't there, isn't a database, or says none).
pub(crate) fn schema_of(path: &Path) -> Option<u32> {
    use rusqlite::OpenFlags;
    if !path.is_file() {
        return None;
    }
    let conn = if path.with_extension("db-wal").exists() {
        Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
    } else {
        Connection::open_with_flags(
            crate::snapshot::immutable_uri(path),
            OpenFlags::SQLITE_OPEN_READ_ONLY
                | OpenFlags::SQLITE_OPEN_NO_MUTEX
                | OpenFlags::SQLITE_OPEN_URI,
        )
    }
    .ok()?;
    conn.query_row(
        "SELECT value FROM meta WHERE key='schema_version'",
        [],
        |r| r.get::<_, String>(0),
    )
    .ok()?
    .parse()
    .ok()
}

/// The lock file a writer holds while the books are open. Readers (the
/// viewer's [`crate::Snapshot`]) never take it.
pub const LOCK_FILE: &str = "wanderbooks.lock";

/// Take the books' writer lock, or say who has it.
///
/// ```text
/// GOAL: one writer at a time.
/// SOURCE: R-11 (one append-only log); docs/receipt-store.md (single write
///         path).
/// SCENARIOS:
///   1. IF another process (or another Store in this one) has the books
///      open for writing:
///      - SHOULD refuse with Locked, saying so
///      - NEVER let two writers extend the journal
///      BECAUSE each would chain from the same last line and break it.
/// ```
fn lock_books(data_dir: &Path) -> Result<std::fs::File, StoreError> {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(data_dir.join(LOCK_FILE))?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(std::fs::TryLockError::WouldBlock) => Err(StoreError::Locked(format!(
            "another wanderbooks process has the books in {} open for writing (an agent session, \
             or its MCP server); close it first. The viewer is fine: it only reads",
            data_dir.display()
        ))),
        Err(std::fs::TryLockError::Error(e)) => Err(StoreError::Io(e)),
    }
}
/// Take a data dir's writer lock to copy it (adoption, `root.rs`): held,
/// no other writer can append while the copy is made.
pub(crate) fn lock_books_for_adoption(data_dir: &Path) -> Result<std::fs::File, StoreError> {
    lock_books(data_dir)
}

pub(crate) const BLOBS_DIR: &str = "blobs";

/// One row of an entry listing.
#[derive(Clone, Debug)]
pub struct EntryRow {
    /// Entry id.
    pub id: u64,
    /// ISO date.
    pub date: String,
    /// Description.
    pub description: String,
    /// Id of the reversed entry, if this is a reversal.
    pub reversal_of: Option<u64>,
}

/// One row of the event log.
#[derive(Clone, Debug)]
pub struct EventRow {
    /// Event seq (1-based, per projection).
    pub seq: u64,
    /// Journal line type.
    pub event_type: String,
    /// Event payload.
    pub payload: Value,
    /// UTC timestamp.
    pub created_at: String,
}

/// The outcome of one month's depreciation run for one asset.
#[derive(Clone, Debug)]
pub struct DepreciationRun {
    /// Asset id.
    pub asset_id: u64,
    /// Calendar month, "YYYY-MM".
    pub period: String,
    /// Amount posted, exact to the minor unit: the month's own
    /// depreciation and any month before it caught up
    /// (ledger_core::run_amount). Zero when nothing was due.
    pub amount: Decimal,
    /// The month's own scheduled depreciation (zero after the life).
    pub scheduled: Decimal,
    /// The posted entry's id; `None` when nothing was due, so nothing
    /// posted.
    pub entry_id: Option<u64>,
    /// True when this period was already posted (idempotent no-op).
    pub already_posted: bool,
    /// Why nothing posted, when nothing did and the period wasn't already
    /// posted: the month rounds to nothing, a later run caught it up, its
    /// run was reversed after the month ended (the reversal's month catches
    /// it up), the life is over and fully depreciated, or the books carry
    /// more than the schedule (a correction for a human).
    pub nothing_due: Option<String>,
}

/// The outcome of disposing an asset, computed without posting anything.
/// The MCP layer previews this; [`Store::dispose_asset`] posts it.
#[derive(Clone, Debug)]
pub struct DisposalPreview {
    /// Asset id.
    pub asset_id: u64,
    /// Disposal date.
    pub date: chrono::NaiveDate,
    /// Proceeds.
    pub proceeds: Money,
    /// Accumulated depreciation the disposal entry clears: every run the
    /// books carry for the asset, plus the final depreciation.
    pub accumulated: Decimal,
    /// Depreciation auto-posted (its own entry) for the months the runs
    /// haven't reached, through the disposal day. Zero when the runs
    /// already reach it.
    pub final_depreciation: Decimal,
    /// Depreciation the runs posted beyond the disposal day (the disposal
    /// month's run, dated its end; a run of a later month is refused, to be
    /// reversed first), taken back by a credit to depreciation expense in
    /// the disposal entry.
    pub excess_depreciation: Decimal,
    /// Depreciation still due through the disposal day when the disposal
    /// month already has a run (an earlier month's run reversed and not
    /// posted again), charged by a debit to depreciation expense in the
    /// disposal entry: the month's run can't be posted twice.
    pub catch_up_depreciation: Decimal,
    /// Book value at disposal: cost less the schedule through the disposal
    /// day, whatever the runs posted.
    pub book_value: Decimal,
    /// proceeds − book_value. Positive is a gain.
    pub gain_loss: Decimal,
    /// The disposal month, "YYYY-MM".
    pub period: String,
}

/// A depreciation run the books carry for an asset: one the ledger
/// recorded (`Ledger::depreciation_runs`), its entry in effect (not
/// reversed).
#[derive(Clone, Debug, PartialEq, Eq)]
struct LiveRun {
    /// Its month, "YYYY-MM" as journaled.
    period: String,
    /// The entry that posted it.
    entry_id: EntryId,
    /// What it posted.
    amount: Decimal,
}

/// Attachment metadata (bytes live in the blob store).
#[derive(Clone, Debug)]
pub struct Attachment {
    /// Attachment id.
    pub id: u64,
    /// SHA-256 of the bytes.
    pub blob_sha: String,
    /// Linked entry, if any.
    pub entry_id: Option<u64>,
    /// `receipts`, `formation`, `legal`, `tax`, `accounting`, `other`.
    pub category: String,
    /// Original file name.
    pub file_name: String,
    /// MIME type.
    pub mime_type: String,
    /// Size in bytes.
    pub size_bytes: u64,
    /// Optional description.
    pub description: Option<String>,
    /// UTC timestamp when filed.
    pub created_at: String,
    /// UTC timestamp when withdrawn, if it was.
    pub withdrawn_at: Option<String>,
    /// Why it was withdrawn, if it was.
    pub withdrawn_reason: Option<String>,
}

/// Input to [`Store::run_payroll`]: one employee's gross pay and every
/// withholding and deduction. The engine's [`PayrollLine`], aliased so the
/// write-path signature reads as input rather than stored state.
pub type PayrollLineInput = PayrollLine;

/// One line of a previewed posting, the way the page shows it: the
/// account's code and name (resolved in the probe, so an account the
/// preview would create is named even though the live books don't have it
/// yet), its side, amount and project (plan P4b S4.4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreviewPosting {
    /// The account's code ("1200").
    pub account: String,
    /// Its name ("Accounts Receivable").
    pub name: String,
    /// Debit or credit.
    pub side: Side,
    /// The amount.
    pub amount: Decimal,
    /// The project it is tagged with, if any.
    pub class_id: Option<ClassId>,
}

/// What issuing an invoice would post and take, journaling nothing
/// ([`Store::preview_invoice`]; plan P4b S4.3 item 12). The posting is the
/// entry's lines in order: account, side, amount, project.
#[derive(Clone, Debug, PartialEq)]
pub struct InvoicePreview {
    /// The entry it would post, line by line.
    pub posting: Vec<(AccountId, Side, Decimal, Option<ClassId>)>,
    /// The same lines with their accounts' codes and names (plan P4b
    /// S4.4: the preview page shows them, and an account the issue would
    /// create has no id the live books know yet).
    pub lines: Vec<PreviewPosting>,
    /// The document's total.
    pub total: Decimal,
    /// The number it would take (A-2), when the books number their
    /// invoices.
    pub number: Option<String>,
    /// Its due date.
    pub due_date: NaiveDate,
    /// The invoice the probe wrote: its lines, terms, snapshots and
    /// dates, exactly as the issue would store them. What the DRAFT copy
    /// prints (plan P4b S4.4).
    pub invoice: arap::Invoice,
}

/// What recording a bill would post, journaling nothing
/// ([`Store::preview_bill`]; plan P4b S4.3 item 12).
#[derive(Clone, Debug, PartialEq)]
pub struct BillPreview {
    /// The entry it would post, line by line.
    pub posting: Vec<(AccountId, Side, Decimal, Option<ClassId>)>,
    /// The same lines with their accounts' codes and names
    /// ([`InvoicePreview::lines`]).
    pub lines: Vec<PreviewPosting>,
    /// The document's total.
    pub total: Decimal,
    /// Its due date.
    pub due_date: NaiveDate,
}

/// The most recurring drafts one [`Store::run_recurring`] creates: more is
/// refused, so a far-off date can't make one request write without end.
pub const MAX_RECURRING_RUN: usize = 1000;

/// A probe entry's lines as a preview shows them: each account's code and
/// name, read from the probe (plan P4b S4.4). The probe always holds the
/// accounts its posting names, so a missing one is a bug, reported as an
/// error rather than a panic.
fn preview_lines(
    probe: &Ledger,
    lines: &[ledger_core::journal::JournalLine],
) -> Result<Vec<PreviewPosting>, StoreError> {
    lines
        .iter()
        .map(|l| {
            let account = probe
                .chart()
                .get(l.account)
                .ok_or(LedgerError::UnknownAccount(l.account))?;
            Ok(PreviewPosting {
                account: account.code.clone(),
                name: account.name.clone(),
                side: l.side,
                amount: l.amount.amount,
                class_id: l.class_id,
            })
        })
        .collect()
}

/// The store: one writer, one data directory.
///
/// `Store` is `Send` but not `Sync` (it owns a `rusqlite::Connection`).
/// There is exactly one writer per data directory.
pub struct Store {
    /// The exclusive lock on [`LOCK_FILE`], held while the store is open:
    /// one writer at a time. Released when dropped, or when the process
    /// exits, even after a crash.
    _lock: std::fs::File,
    data_dir: PathBuf,
    db_path: PathBuf,
    journal: JournalWriter,
    conn: Connection,
    blobs: BlobStore,
    ledger: Ledger,
    base_currency: Currency,
    /// The journal format in force (the format gate's record).
    format: JournalFormat,
}

/// One filing of bytes into the blob store and journal.
struct AttachRequest<'a> {
    line_type: &'a str,
    category: &'a str,
    entry_id: Option<EntryId>,
    file_name: &'a str,
    mime_type: &'a str,
    bytes: &'a [u8],
    description: Option<&'a str>,
}

impl Store {
    // ------------------------------------------------------------------
    // Open / create
    // ------------------------------------------------------------------

    /// Create a new store. Fails if one already exists.
    ///
    /// ```text
    /// GOAL: start empty books in a data directory, in this version's
    ///       journal format.
    /// SOURCE: docs/receipt-store.md "Line format" (genesis prev); R-11;
    ///         plan/served.md 3.9 and D-39 (books this version makes wait
    ///         for no format upgrade).
    /// INVARIANTS:
    ///   1. The journal's first line MUST be `store.created` with the base
    ///      currency, chained from the genesis hash (64 zeros).
    ///   2. The writer lock MUST be held from creation on.
    ///   3. The second line declares this version's journal format, every
    ///      feature it knows (format::current()): the new books wait for
    ///      no format upgrade.
    /// SCENARIOS:
    ///   1. IF a journal already exists there: SHOULD refuse
    ///      - NEVER overwrite or append to existing books.
    /// ```
    pub fn create(data_dir: &Path, base_currency: Currency) -> Result<Self, StoreError> {
        Self::create_in_format(data_dir, base_currency, &crate::format::current())
    }

    /// [`Self::create`], declaring the journal format `format` (format 1,
    /// [`JournalFormat::default`], declares nothing): books as an older
    /// version made them, for tests of their format upgrade. Everything
    /// else creates books with [`Self::create`].
    #[doc(hidden)]
    pub fn create_in_format(
        data_dir: &Path,
        base_currency: Currency,
        format: &JournalFormat,
    ) -> Result<Self, StoreError> {
        let journal_path = data_dir.join(JOURNAL_FILE);
        if journal_path.exists() {
            return Err(StoreError::AlreadyExists(data_dir.display().to_string()));
        }
        std::fs::create_dir_all(data_dir)?;
        let lock = lock_books(data_dir)?;
        // New books: this version's own schema's file (db_file_for).
        let db_path = data_dir.join(db_file());
        let conn = Self::open_conn(&db_path)?;
        Self::migrate(&conn)?;
        Self::write_meta(&conn, "base_currency", base_currency.code())?;
        Self::write_meta(&conn, "last_applied_seq", "0")?;
        let mut store = Self {
            _lock: lock,
            data_dir: data_dir.to_path_buf(),
            db_path,
            journal: JournalWriter::open(&journal_path)?,
            conn,
            blobs: BlobStore::open(&data_dir.join(BLOBS_DIR))?,
            ledger: Ledger::new(base_currency),
            base_currency,
            format: JournalFormat::default(),
        };
        let line = store.journal.append(
            "store.created",
            json!({ "base_currency": base_currency.code() }),
        )?;
        apply_line_to_sqlite(&store.conn, &line)?;
        if *format != JournalFormat::default() {
            let features: Vec<&str> = format.features.iter().map(String::as_str).collect();
            store.declare_journal_format(format.format, &features)?;
        }
        Ok(store)
    }

    /// Open an existing store, verifying the journal chain and catching the
    /// SQLite projection up to the journal head. A missing database file is
    /// healed by full replay: the journal is the source of truth.
    ///
    /// ```text
    /// GOAL: the books as journaled, with a projection that matches them.
    /// SOURCE: docs/receipt-store.md "Write path" (a crash between append
    ///         and apply is healed from `last_applied_seq`), "Verification";
    ///         journal::repair_torn_tail (an unacknowledged cut-off line).
    /// INVARIANTS:
    ///   1. The writer lock MUST be taken before anything is read.
    ///   2. The ledger MUST be the replay of every journal line, in order.
    ///   3. The projection MUST end caught up to the journal head.
    /// SCENARIOS:
    ///   1. IF there is no journal: SHOULD refuse (NotFound).
    ///   2. IF another writer has the books: SHOULD refuse (Locked).
    ///   3. IF the last line is cut off (no newline) and every complete
    ///      line passes (its chain, the format gate, the first line's
    ///      store.created, and its replay into the ledger, all in memory):
    ///      SHOULD set it aside, truncate, and journal
    ///      `journal.tail_repaired`; IF a complete line fails: SHOULD
    ///      refuse with the journal byte for byte as it was.
    ///   4. IF a complete line fails its hash, parse, or `seq`: SHOULD refuse
    ///      - NEVER repair or skip it (that is corruption, for a human).
    ///   4a. IF a whole, chained line is one this version can't read (an
    ///      unknown type, a newer journal format, a key beside the
    ///      envelope's): SHOULD refuse with NewerJournal before touching
    ///      the database or the journal, a cut-off last line included
    ///      BECAUSE a newer wanderbooks wrote it, and may be appending.
    ///   5. IF the database is missing: SHOULD rebuild it from the journal.
    ///   6. IF the database fails SQLite's integrity check, or records a
    ///      base currency other than the journal's: SHOULD keep it aside
    ///      for forensics and rebuild from the journal
    ///      BECAUSE the database is derived; the journal is the record.
    ///   7. IF the database is behind the journal: SHOULD apply the rest.
    ///   8. The database is this version's own schema's file, or the
    ///      legacy one when that already holds this version's schema
    ///      ([`database_in_use`]); a file of any other schema, older or
    ///      newer, is left byte for byte, never migrated, refused or
    ///      deleted; this version's own missing: SHOULD build it from the
    ///      journal (SCENARIO 5). One named as this version's that holds
    ///      another schema is damaged (SCENARIO 6).
    /// ```
    pub fn open(data_dir: &Path) -> Result<Self, StoreError> {
        let journal_path = data_dir.join(JOURNAL_FILE);
        if !journal_path.exists() {
            return Err(StoreError::NotFound(data_dir.display().to_string()));
        }
        // The writer lock first: nothing is read until no one else writes.
        let lock = lock_books(data_dir)?;
        // Single verified scan of the complete lines, through the format
        // gate, before anything is changed: books from a newer wanderbooks
        // are refused whole, never read in part, and never altered (a
        // newer version may be mid-append). The writer reuses the scan.
        let raw = std::fs::read(&journal_path)?;
        let complete = raw.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
        let (lines, format) = crate::journal::verify_journal_and_format(&raw[..complete])?;
        // The base currency comes from the journal, the source of truth; a
        // journal without a sound store.created line is corrupt.
        let base_currency = journal_currency(&lines)?;
        // Every complete line replayed, in memory, before anything on disk
        // changes (SCENARIO 3): a line the books can't apply refuses the
        // open with the journal and the database as they were.
        let mut ledger = Ledger::new(base_currency);
        for line in &lines {
            replay_line_into_ledger(&mut ledger, line, base_currency)?;
        }
        // Only then is an append cut off mid-line (never acknowledged) set
        // aside, and journaled below.
        let torn = if complete < raw.len() {
            crate::journal::repair_torn_tail(&journal_path)?
        } else {
            None
        };
        let journal = JournalWriter::from_lines(&journal_path, &lines)?;
        // This version's own schema's file (SCENARIO 8): another schema's
        // is never opened for writing.
        let db_path = database_in_use(data_dir);
        let mut fresh_db = !db_path.exists();
        let schema = crate::schema::SCHEMA_VERSION;
        let damage = if fresh_db {
            None
        } else if !Self::db_file_healthy(&db_path) {
            Some("failed integrity_check".to_string())
        } else if let Some(other) = schema_of(&db_path).filter(|v| *v != schema) {
            Some(format!("holds schema {other}, not this version's {schema}"))
        } else {
            match Self::db_currency(&db_path) {
                Some(c) if c == base_currency => None,
                other => Some(format!(
                    "says base currency {}, the journal {}",
                    other.map_or("(none)".to_string(), |c| c.code().to_string()),
                    base_currency.code()
                )),
            }
        };
        if let Some(why) = damage {
            // The projection is wrong. The journal is the source of truth,
            // so quarantine the database (for forensics) and rebuild.
            let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
            let quarantine = db_path.with_extension(format!("corrupt.{stamp}"));
            std::fs::rename(&db_path, &quarantine).map_err(StoreError::Io)?;
            // WAL/SHM are derived state; drop them so they can't contaminate
            // the rebuild.
            let _ = std::fs::remove_file(db_path.with_extension("db-wal"));
            let _ = std::fs::remove_file(db_path.with_extension("db-shm"));
            eprintln!(
                "wanderbooks: sqlite projection {why}; quarantined to {} beside it and rebuilding from journal",
                quarantine.file_name().unwrap_or_default().to_string_lossy()
            );
            fresh_db = true;
        }
        let conn = Self::open_conn(&db_path)?;
        Self::migrate(&conn)?;
        if fresh_db {
            Self::write_meta(&conn, "base_currency", base_currency.code())?;
            Self::write_meta(&conn, "last_applied_seq", "0")?;
        }
        let mut store = Self {
            _lock: lock,
            data_dir: data_dir.to_path_buf(),
            db_path,
            journal,
            conn,
            blobs: BlobStore::open(&data_dir.join(BLOBS_DIR))?,
            ledger,
            base_currency,
            format,
        };
        store.catch_up(&lines)?;
        if let Some(t) = torn {
            // On the record, so the repair is part of the audit trail.
            let line = store.append_journal(
                "journal.tail_repaired",
                json!({
                    "bytes": t.bytes,
                    "sha256": t.sha256,
                    "saved_as": t.saved_as.file_name().map(|n| n.to_string_lossy().into_owned()),
                }),
            )?;
            apply_line_to_sqlite(&store.conn, &line)?;
            eprintln!(
                "wanderbooks: the journal's last line was cut off (an append that never finished); set aside {} bytes in {} beside it",
                t.bytes,
                t.saved_as.file_name().unwrap_or_default().to_string_lossy()
            );
        }
        Ok(store)
    }

    /// The base currency an existing database records, if it can say.
    fn db_currency(db_path: &Path) -> Option<Currency> {
        let conn = Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .ok()?;
        let code: String = conn
            .query_row(
                "SELECT value FROM meta WHERE key='base_currency'",
                [],
                |r| r.get(0),
            )
            .ok()?;
        Currency::new(&code).ok()
    }

    fn open_conn(db_path: &Path) -> Result<Connection, StoreError> {
        let conn = Connection::open(db_path)?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;",
        )?;
        Ok(conn)
    }

    /// True when the SQLite file at `db_path` passes `integrity_check`.
    /// Any failure (missing file, not a database, corruption) counts as
    /// unhealthy. Opens a throwaway connection without touching pragmas.
    fn db_file_healthy(db_path: &Path) -> bool {
        let conn = match Connection::open(db_path) {
            Ok(c) => c,
            Err(_) => return false,
        };
        let check: Result<String, _> = conn.query_row("PRAGMA integrity_check", [], |r| r.get(0));
        matches!(check, Ok(s) if s == "ok")
    }

    pub(crate) fn migrate(conn: &Connection) -> Result<(), StoreError> {
        let mut v = Self::current_version(conn)?;
        if v > schema::SCHEMA_VERSION {
            return Err(StoreError::SchemaTooNew(v, schema::SCHEMA_VERSION));
        }
        while v < schema::SCHEMA_VERSION {
            conn.execute_batch(schema::MIGRATIONS[v as usize])?;
            v += 1;
            Self::write_meta(conn, "schema_version", &v.to_string())?;
        }
        Ok(())
    }

    fn current_version(conn: &Connection) -> Result<u32, StoreError> {
        let has: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='meta'",
            [],
            |r| r.get(0),
        )?;
        if has == 0 {
            return Ok(0);
        }
        let v: Option<String> = conn
            .query_row(
                "SELECT value FROM meta WHERE key='schema_version'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        Ok(v.and_then(|s| s.parse().ok()).unwrap_or(0))
    }

    fn write_meta(conn: &Connection, key: &str, value: &str) -> Result<(), StoreError> {
        conn.execute(
            "INSERT INTO meta(key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    /// Catch the projection up: apply to SQLite every line past
    /// `last_applied_seq` (the ledger is already the replay of every line,
    /// done in memory by [`Store::open`] before anything changed).
    fn catch_up(&mut self, lines: &[JournalLine]) -> Result<(), StoreError> {
        let last_applied: u64 = self
            .conn
            .query_row(
                "SELECT value FROM meta WHERE key='last_applied_seq'",
                [],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        // The ledger is already their replay (Store::open): only the
        // projection is caught up here.
        for line in lines.iter().filter(|l| l.seq > last_applied) {
            apply_line_to_sqlite(&self.conn, line)?;
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // Accessors
    // ------------------------------------------------------------------

    /// The live ledger: validation authority and report source.
    pub fn ledger(&self) -> &Ledger {
        &self.ledger
    }

    /// Base currency of this store.
    pub fn base_currency(&self) -> Currency {
        self.base_currency
    }

    /// The journal's head: the last line's seq and its hash (what
    /// `verify` reports as the head once it has checked the chain).
    pub fn head(&self) -> (u64, String) {
        let (seq, hash) = self.journal.head();
        (seq, hash.to_string())
    }

    /// The journal format in force: format 1 unless the books declared
    /// another with `journal.format_set`.
    pub fn journal_format(&self) -> &JournalFormat {
        &self.format
    }

    /// The projection database's file this version uses, by name
    /// ([`database_in_use`]): what `books.verify` and `wanderbooks doctor`
    /// say.
    pub fn database_file(&self) -> String {
        file_name_of(&self.db_path)
    }

    /// The other projection files beside it (other schemas', the legacy
    /// one if unused), by name, sorted: listed, never deleted
    /// ([`db_file_for`] INVARIANT 2).
    pub fn other_databases(&self) -> Vec<String> {
        let in_use = self.database_file();
        let mut out: Vec<String> = std::fs::read_dir(&self.data_dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| is_database_file(n) && *n != in_use)
            .collect();
        out.sort();
        out
    }

    /// Declare the journal format: journal a `journal.format_set` line.
    /// A writer declares the format before the first event that needs it,
    /// so an older reader stops at the declaration and says it is too old:
    /// new books as they are created ([`Self::create`]), older books at
    /// their format upgrade ([`Self::upgrade_format`]), never a tool.
    ///
    /// ```text
    /// GOAL: books that tell every reader which format they are in.
    /// SOURCE: rules.md B-27; plan/served.md 5.1 (format gate).
    /// SCENARIOS:
    ///   1. IF the format is newer than SUPPORTED_JOURNAL_FORMAT, or 0:
    ///      SHOULD refuse (FormatRefused), journaling nothing
    ///      BECAUSE this version couldn't open the books it left.
    ///   2. IF the format is lower than the one in force: SHOULD refuse
    ///      BECAUSE events written under it may need the higher one.
    ///   3. IF a feature isn't in KNOWN_FEATURES: SHOULD refuse.
    ///   4. Otherwise: SHOULD journal it (features sorted, once each) and
    ///      make it the format in force.
    /// ```
    pub fn declare_journal_format(
        &mut self,
        format: u32,
        features: &[&str],
    ) -> Result<(), StoreError> {
        let payload = self.format.declaration(format, features)?;
        let line = self.append_journal(crate::format::FORMAT_SET, payload)?;
        self.format = crate::format::declared(&line)?;
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // How the books are kept: settings, conventions, decisions, open
    // items, approvals (journal format 2, crate::meta)
    // ------------------------------------------------------------------

    /// Refuse an event that needs journal format 2 in books that haven't
    /// declared it: books an older version wrote, waiting for their format
    /// upgrade (the owner's go-ahead, [`Self::upgrade_format`]). Books this
    /// version made declared it when they were created.
    fn ensure_format(&self) -> Result<(), StoreError> {
        self.format.require_format(crate::meta::FORMAT)
    }

    /// Refuse a write that needs the journal feature `feature`
    /// (crate::format::KNOWN_FEATURES: a field or value on an event type an
    /// older version knows, which that version would misread silently)
    /// unless the books declared it. Books this version made declared it
    /// when they were created; older books declare it only at their format
    /// upgrade, with every other feature this version knows, never one at
    /// a time and never without the owner's go-ahead
    /// ([`Self::upgrade_format`]). Ask it only for a write that uses the
    /// feature (the line about to be journaled holds the new field or
    /// value): a write that doesn't is never refused, since declining the
    /// upgrade keeps everything else working (D-39).
    ///
    /// Each command here that writes a feature's line asks for it first. A
    /// caller that journals several commands as one (a tool drafting an
    /// entry and tagging it) calls this before the first of them, once
    /// every check has passed: a refusal never lands after another line of
    /// the same command (which would leave the command half done), and a
    /// refused command leaves none.
    ///
    /// ```text
    /// GOAL: an older wanderbooks stops at the declaration (NewerJournal)
    ///       instead of misreading a line it thinks it knows, and books
    ///       move past what their owner's older version reads only when
    ///       the owner says so.
    /// SOURCE: rules.md B-27; plan/served.md 5.1 (format gate), 3.9 and
    ///         D-39 (the format upgrade).
    /// SCENARIOS:
    ///   1. IF the books declared it: SHOULD pass.
    ///   2. IF not: SHOULD refuse (FormatUpgradePending), journaling
    ///      nothing.
    /// ```
    pub fn ensure_feature(&self, feature: &str) -> Result<(), StoreError> {
        debug_assert!(
            crate::format::KNOWN_FEATURES.contains(&feature),
            "{feature} isn't a feature this version knows"
        );
        self.format.require_feature(feature)
    }

    /// The format upgrade (plan/served.md 3.9, D-39): declare this
    /// version's journal format, with every feature it knows, in one
    /// `journal.format_set`, for books an older version wrote. What was
    /// upgraded; None (journaling nothing) when the books already are in
    /// this version's format. The books' figures don't change: only what
    /// may be written from here on.
    ///
    /// Only the owner's go-ahead comes here: the books server, after the
    /// owner passphrase and a verified backup with the keys (ledger-mcp
    /// local.rs and backup.rs); and an adoption, whose copy is new books
    /// of this version while the original keeps its own format (root.rs).
    /// No tool does.
    ///
    /// ```text
    /// GOAL: older books move to this version's format at once, on the
    ///       record, exactly once.
    /// SOURCE: plan/served.md 3.9 ("Format changes are the server's
    ///         gate"); D-39; rules.md B-27.
    /// INVARIANTS:
    ///   1. One journal line declares the whole format: the format moves
    ///      exactly once, never feature by feature.
    ///   2. Books already in this version's format: nothing is journaled.
    ///   3. The ledger is unchanged: every report reads as before.
    /// ```
    pub fn upgrade_format(&mut self) -> Result<Option<crate::format::FormatUpgrade>, StoreError> {
        let Some(upgrade) = self.format.upgrade() else {
            return Ok(None);
        };
        let features: Vec<&str> = upgrade.to.features.iter().map(String::as_str).collect();
        self.declare_journal_format(upgrade.to.format, &features)?;
        debug_assert_eq!(self.format, upgrade.to);
        Ok(Some(upgrade))
    }

    /// [`Self::ensure_feature`] for each feature a write needs: refused
    /// (FormatUpgradePending, journaling nothing) unless the books declared
    /// every one.
    fn ensure_features(&self, needed: &[&str]) -> Result<(), StoreError> {
        needed.iter().try_for_each(|f| self.ensure_feature(f))
    }

    /// Change settings (`settings.set`): all or nothing, each value
    /// canonical, the result whole (ledger_core::settings). Returns the
    /// changes made; none (journaling nothing) when every change is the
    /// value already set.
    pub fn change_settings(
        &mut self,
        changes: &[ledger_core::settings::Change],
        source: &ledger_core::settings::Source,
    ) -> Result<Vec<ledger_core::settings::Change>, StoreError> {
        self.journal_settings(changes, source, false)
    }

    /// [`Self::change_settings`] for books moving their settings in from a
    /// settings file: the first `settings.set` is journaled even when it
    /// changes nothing (a file that sets only defaults), so from then on
    /// the books keep their own settings and no file is read.
    pub fn adopt_settings(
        &mut self,
        changes: &[ledger_core::settings::Change],
        source: &ledger_core::settings::Source,
    ) -> Result<Vec<ledger_core::settings::Change>, StoreError> {
        self.journal_settings(changes, source, true)
    }

    fn journal_settings(
        &mut self,
        changes: &[ledger_core::settings::Change],
        source: &ledger_core::settings::Source,
        adopt: bool,
    ) -> Result<Vec<ledger_core::settings::Change>, StoreError> {
        let changes = self
            .ledger
            .check_settings_adopting(changes, source, adopt)?;
        if changes.is_empty() && !(adopt && !self.ledger.settings().is_journaled()) {
            return Ok(changes);
        }
        // A key an older version doesn't know needs its feature
        // (crate::format::setting_feature), and every settings.set format
        // 2: refused, journaling nothing, in books that haven't declared
        // them (their format upgrade does).
        let needed: Vec<&str> = changes
            .iter()
            .filter_map(|c| crate::format::setting_feature(&c.key))
            .collect();
        self.ensure_features(&needed)?;
        self.ensure_format()?;
        // invoicing.first_number is settable only while no numbered invoice
        // exists (A-2, rules.md B-40): the series has begun, and a number is
        // never reused or renumbered. Changing or clearing it is refused
        // before anything is journaled.
        if changes.iter().any(|c| c.key == "invoicing.first_number")
            && self.ledger.invoices().iter().any(|i| i.number.is_some())
        {
            return Err(LedgerError::FirstNumberLocked.into());
        }
        let line =
            self.append_journal("settings.set", crate::meta::settings_set(&changes, source))?;
        self.ledger.apply_settings(&changes, source, line.seq)?;
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(changes)
    }

    /// Set or revise a convention (`convention.set`). Returns it as
    /// journaled.
    pub fn set_convention(
        &mut self,
        c: &ledger_core::conventions::NewConvention,
    ) -> Result<ledger_core::conventions::NewConvention, StoreError> {
        let c = self.ledger.check_convention(c)?;
        self.ensure_format()?;
        let line = self.append_journal("convention.set", crate::meta::convention_set(&c))?;
        self.ledger.apply_convention(c.clone(), line.seq)?;
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(c)
    }

    /// Retire a convention (`convention.retired`).
    pub fn retire_convention(
        &mut self,
        id: &str,
        reason: &str,
        source: &ledger_core::settings::Source,
    ) -> Result<(), StoreError> {
        let reason = self.ledger.check_retire_convention(id, reason, source)?;
        self.ensure_format()?;
        let line = self.append_journal(
            "convention.retired",
            crate::meta::convention_retired(id, &reason, source),
        )?;
        self.ledger
            .apply_retire_convention(id, reason, source.clone(), line.seq)?;
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(())
    }

    /// Record a decision (`decision.recorded`); its evidence, if any, is
    /// a filed attachment not withdrawn. Returns its id.
    pub fn record_decision(
        &mut self,
        d: &ledger_core::conventions::NewDecision,
    ) -> Result<u64, StoreError> {
        let (id, d) = self.ledger.check_decision(d)?;
        if let Some(a) = d.evidence_attachment
            && self.attachment_state(a)?.1
        {
            return Err(StoreError::AttachmentWithdrawn(a));
        }
        self.ensure_format()?;
        if !d.projects.is_empty() {
            self.ensure_feature(crate::format::PROJECTS)?;
        }
        let line =
            self.append_journal("decision.recorded", crate::meta::decision_recorded(id, &d))?;
        self.ledger.apply_decision(id, d, line.seq)?;
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(id)
    }

    /// Raise an open item (`open_item.raised`). Returns its id.
    pub fn raise_open_item(
        &mut self,
        i: &ledger_core::conventions::NewOpenItem,
    ) -> Result<u64, StoreError> {
        let (id, i) = self.ledger.check_open_item(i)?;
        self.ensure_format()?;
        let line =
            self.append_journal("open_item.raised", crate::meta::open_item_raised(id, &i))?;
        self.ledger.apply_open_item(id, i, line.seq)?;
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(id)
    }

    /// Resolve an open item (`open_item.resolved`).
    pub fn resolve_open_item(
        &mut self,
        id: u64,
        resolution: &str,
        source: &ledger_core::settings::Source,
    ) -> Result<(), StoreError> {
        let text = self.ledger.check_resolve_item(id, resolution, source)?;
        self.ensure_format()?;
        let line = self.append_journal(
            "open_item.resolved",
            crate::meta::open_item_resolved(id, &text, source),
        )?;
        self.ledger
            .apply_resolve_item(id, text, source.clone(), line.seq)?;
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(())
    }

    /// Record an approval of a draft entry (`entry.approved`), as the
    /// draft stands now.
    pub fn approve_entry(
        &mut self,
        entry: EntryId,
        approver: &str,
        note: Option<&str>,
        source: &ledger_core::settings::Source,
    ) -> Result<ledger_core::conventions::Approval, StoreError> {
        let a = self.ledger.check_approval(entry, approver, note, source)?;
        self.ensure_format()?;
        let line = self.append_journal("entry.approved", crate::meta::entry_approved(entry, &a))?;
        let a = ledger_core::conventions::Approval { seq: line.seq, ..a };
        self.ledger.apply_approval(entry, a.clone())?;
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(a)
    }

    /// Draft an entry from a posting template: `amount` split by the
    /// template's shares (ledger_core::conventions INVARIANT 2), its slots
    /// filled from `slots` (slot name → account code), dated
    /// `date`, described as given or as the template says. All or
    /// nothing: no half-built draft is ever journaled. Returns the draft.
    pub fn draft_from_template(
        &mut self,
        template_id: &str,
        date: chrono::NaiveDate,
        amount: Decimal,
        description: Option<&str>,
        slots: &std::collections::BTreeMap<String, String>,
    ) -> Result<EntryId, StoreError> {
        let (default_description, lines) =
            self.ledger.template_lines(template_id, amount, slots)?;
        let description = description
            .map(str::to_string)
            .or(default_description)
            .unwrap_or_else(|| {
                self.ledger
                    .conventions()
                    .convention(template_id)
                    .map(|c| c.title.clone())
                    .unwrap_or_default()
            });
        self.rehearse(|l| {
            let e = l.try_create_entry(date, &description)?;
            for (account, side, amount) in &lines {
                l.add_line(e, *account, *side, *amount)?;
            }
            Ok(e)
        })?;
        let e = self.create_entry(date, &description)?;
        for (account, side, amount) in lines {
            self.add_line(e, account, side, amount)?;
        }
        Ok(e)
    }

    /// Every torn journal tail set aside on open, oldest first, as
    /// (when, bytes, file it was saved to).
    pub fn tail_repairs(&self) -> Result<Vec<(String, u64, String)>, StoreError> {
        let lines = crate::journal::read_and_verify_journal(&self.data_dir.join(JOURNAL_FILE))?;
        Ok(lines
            .iter()
            .filter(|l| l.line_type == "journal.tail_repaired")
            .map(|l| {
                (
                    l.ts.clone(),
                    l.payload["bytes"].as_u64().unwrap_or(0),
                    l.payload["saved_as"].as_str().unwrap_or("").to_string(),
                )
            })
            .collect())
    }

    /// Data directory.
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    // ------------------------------------------------------------------
    // Mutations: validate → journal → mutate → apply
    // ------------------------------------------------------------------

    fn append_journal(
        &mut self,
        line_type: &str,
        payload: Value,
    ) -> Result<JournalLine, StoreError> {
        self.journal.append(line_type, payload)
    }

    /// Add an account to the chart, with no tax category (see
    /// [`Self::add_account_with_tax`]).
    pub fn add_account(
        &mut self,
        code: &str,
        name: &str,
        account_type: AccountType,
        parent: Option<AccountId>,
        placeholder: bool,
    ) -> Result<AccountId, StoreError> {
        self.add_account_with_tax(code, name, account_type, parent, placeholder, None)
    }

    /// Add an account to the chart (`Ledger::add_account`) with, when
    /// `tax` is given, its tax category and deductible percent:
    /// `account.added`, then `account.tax_set`, the two events
    /// `books.import_chart_csv` has always written.
    ///
    /// ```text
    /// GOAL: an account created with the line of the return it maps to.
    /// SOURCE: rules.md B-32 (policy.require_tax_category).
    /// INVARIANTS:
    ///   1. Rehearsed whole first: a refusal (of the account, or of its
    ///      category) journals nothing.
    ///   2. Under policy.require_tax_category = refuse, a revenue or
    ///      expense account that isn't a header and has no category is
    ///      refused (TaxCategoryRequired), journaling nothing.
    ///   3. No new event and no new field: a power cut between the two
    ///      appends leaves an account without a category, the warn
    ///      state, which Needs attention lists; never a damaged journal.
    ///   4. The name is journaled trimmed, as `Ledger::add_account` keeps
    ///      it and a rename journals one, so live equals replay.
    /// ```
    pub fn add_account_with_tax(
        &mut self,
        code: &str,
        name: &str,
        account_type: AccountType,
        parent: Option<AccountId>,
        placeholder: bool,
        tax: Option<(&str, Option<Decimal>)>,
    ) -> Result<AccountId, StoreError> {
        let name = name.trim();
        let (category, pct) = match tax {
            Some((c, p)) => (Some(c), p),
            None => (None, None),
        };
        self.rehearse(|l| {
            let id = l.add_account(code, name, account_type, parent, placeholder)?;
            if category.is_some() {
                l.set_account_tax(id, category, pct)?;
            }
            Ok(id)
        })?;
        self.ledger
            .check_tax_category_required(code, name, account_type, placeholder, category)?;
        // A category an earlier version doesn't know needs its feature:
        // asked before the account's first line, never between the two.
        if let Some(f) = category.and_then(crate::format::tax_category_feature) {
            self.ensure_feature(f)?;
        }
        let id =
            self.journal_account_added(code, name, account_type, parent, placeholder, |l| {
                l.add_account(code, name, account_type, parent, placeholder)
            })?;
        if category.is_some() {
            self.journal_account_tax(id, category, pct)?;
        }
        Ok(id)
    }

    /// Journal `account.added` and apply it with `add` (the command the
    /// caller rehearsed).
    fn journal_account_added(
        &mut self,
        code: &str,
        name: &str,
        account_type: AccountType,
        parent: Option<AccountId>,
        placeholder: bool,
        add: impl FnOnce(&mut Ledger) -> Result<AccountId, LedgerError>,
    ) -> Result<AccountId, StoreError> {
        // Pure pre-validation replicating ChartOfAccounts::add, so the
        // journal write below cannot be followed by a failed mutation.
        let chart = self.ledger.chart();
        if code.is_empty() {
            return Err(LedgerError::EmptyAccountCode.into());
        }
        if chart.find_by_code(code).is_some() {
            return Err(LedgerError::DuplicateAccountCode(code.to_string()).into());
        }
        if let Some(p) = parent
            && chart.get(p).is_none()
        {
            return Err(LedgerError::UnknownParentAccount(p).into());
        }
        let id = self.ledger.next_account_id();
        let line = self.append_journal(
            "account.added",
            json!({
                "id": id,
                "code": code,
                "name": name,
                "account_type": serde_json::to_value(account_type)?,
                "parent": parent,
                "placeholder": placeholder,
                "currency": self.base_currency.code(),
            }),
        )?;
        let live = add(&mut self.ledger)?;
        debug_assert_eq!(live, id, "id allocator diverged from journal");
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(live)
    }

    /// Start a draft entry. Drafts are journaled as created (for id
    /// determinism) but never touch the projection until posted.
    pub fn create_entry(
        &mut self,
        date: chrono::NaiveDate,
        description: &str,
    ) -> Result<EntryId, StoreError> {
        // The checked constructor: no draft is ever dated in a closed
        // period (the Lean invariant `Inv.draftsOpen`).
        self.rehearse(|l| l.try_create_entry(date, description))?;
        let id = self.ledger.next_entry_id();
        let line = self.append_journal(
            "entry.created",
            json!({ "id": id, "date": date.to_string(), "description": description }),
        )?;
        let live = self.ledger.create_entry(date, description);
        debug_assert_eq!(live, id, "id allocator diverged from journal");
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(live)
    }

    /// Add a line to a draft entry. Drafts are durable: the line is
    /// journaled as `entry.line_added`, so drafts (and their receipts)
    /// survive restarts. Drafts never touch the SQLite projection until
    /// posted.
    pub fn add_line(
        &mut self,
        entry: EntryId,
        account: AccountId,
        side: Side,
        amount: Money,
    ) -> Result<(), StoreError> {
        let mut next = self.ledger.clone();
        next.add_line(entry, account, side, amount)?;
        let line = self.append_journal(
            "entry.line_added",
            json!({
                "id": entry,
                "account": account,
                "side": match side {
                    Side::Debit => "debit",
                    Side::Credit => "credit",
                },
                "amount": amount.amount.to_string(),
                "currency": amount.currency.code(),
            }),
        )?;
        self.ledger = next;
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(())
    }

    /// Validate, journal, and post a draft entry. An entry an enforced
    /// approval rule covers posts only once its approvers approved it as
    /// it stands (ledger_core::conventions INVARIANT 3).
    pub fn post_entry(&mut self, entry: EntryId) -> Result<(), StoreError> {
        let missing = self.ledger.approvals_missing(entry)?;
        if !missing.is_empty() {
            return Err(LedgerError::ApprovalRequired {
                entry,
                missing: missing
                    .iter()
                    .map(|(rule, approver)| format!("{rule} (approver: {approver})"))
                    .collect::<Vec<_>>()
                    .join(", "),
            }
            .into());
        }
        self.post_entry_unchecked(entry)
    }

    /// Approval rules (ledger_core::conventions INVARIANT 3): refuse an
    /// operation that would post at once an entry an enforced rule covers
    /// (such an entry has no draft to approve: it must be drafted and
    /// approved instead). `last_only`: only the operation's last posted
    /// entry is its own (a disposal's; the depreciation before it follows
    /// from the books and is exempt). Changes nothing.
    fn approval_gate<T>(
        &self,
        op: impl FnOnce(&mut Ledger) -> Result<T, LedgerError>,
        last_only: bool,
    ) -> Result<(), StoreError> {
        if self.ledger.settings().approval_rules() == ledger_core::settings::ApprovalMode::Off
            || self.ledger.conventions().approval_rules().next().is_none()
        {
            return Ok(());
        }
        let mut probe = self.ledger.clone();
        let first = probe.next_entry_id();
        op(&mut probe)?;
        let posted: Vec<EntryId> = (first..probe.next_entry_id())
            .filter(|id| {
                probe
                    .get_entry(*id)
                    .is_some_and(|e| e.status() == EntryStatus::Posted)
            })
            .collect();
        let own = if last_only {
            posted.last().map(std::slice::from_ref).unwrap_or(&[])
        } else {
            &posted[..]
        };
        for &id in own {
            let missing = probe.approvals_missing(id)?;
            if !missing.is_empty() {
                return Err(LedgerError::ApprovalNeedsDraft(
                    missing
                        .iter()
                        .map(|(rule, approver)| format!("{rule} (approver: {approver})"))
                        .collect::<Vec<_>>()
                        .join(", "),
                )
                .into());
            }
        }
        Ok(())
    }

    /// [`Self::post_entry`] for entries the store makes and posts itself
    /// (depreciation, a disposal, whose own gate ran first).
    fn post_entry_unchecked(&mut self, entry: EntryId) -> Result<(), StoreError> {
        self.rehearse(|l| l.post_entry(entry))?;
        let snap = self
            .ledger
            .get_entry(entry)
            .ok_or(LedgerError::UnknownEntry(entry))?
            .clone();
        if snap.status() == EntryStatus::Posted {
            return Err(LedgerError::EntryAlreadyPosted(entry).into());
        }
        // Pure validation before anything is journaled or mutated.
        snap.validate(self.ledger.chart())?;
        let line = self.append_journal(
            "entry.posted",
            json!({
                "id": entry,
                "date": snap.date().to_string(),
                "description": snap.description(),
                "reversal_of": snap.reversal_of(),
                "lines": snap.lines().iter().map(journal_line_json).collect::<Vec<_>>(),
            }),
        )?;
        self.ledger.post_entry(entry)?;
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(())
    }

    /// Post a reversing entry for a posted entry. A reversed depreciation
    /// run or schedule month is un-posted (due again; P3 depreciation review
    /// R3); see [`Self::check_reverse_entry`] for what is refused.
    pub fn reverse_entry(
        &mut self,
        entry: EntryId,
        date: chrono::NaiveDate,
        description: &str,
    ) -> Result<EntryId, StoreError> {
        let orig = self
            .ledger
            .get_entry(entry)
            .ok_or(LedgerError::UnknownEntry(entry))?
            .clone();
        // Every refusal before the journal: a journaled reversal the engine
        // then refuses could never be replayed, and the books would not open.
        self.check_reverse_entry(entry, date)?;
        let new_id = self.ledger.next_entry_id();
        let flipped: Vec<Value> = orig.lines().iter().map(flip_line_json).collect();
        // Reversals journal a single entry.reversed line (no separate
        // entry.created): replay calls Ledger::reverse_entry, which allocates
        // exactly once, so the id stays deterministic.
        let reversed = self.append_journal(
            "entry.reversed",
            json!({
                "id": new_id,
                "reverses": entry,
                "date": date.to_string(),
                "description": description,
                "lines": flipped,
            }),
        )?;
        let live = self.ledger.reverse_entry(entry, date, description)?;
        debug_assert_eq!(live, new_id, "id allocator diverged from journal");
        apply_line_to_sqlite(&self.conn, &reversed)?;
        Ok(live)
    }

    /// Every refusal of [`Self::reverse_entry`]: the ledger's
    /// ([`Ledger::check_reversal`]) and those only the store can make,
    /// since only it knows which entries a fixed asset's disposal and
    /// depreciation runs are.
    ///
    /// ```text
    /// GOAL: a reversal never leaves a fixed asset's figures and its books
    ///       apart.
    /// SOURCE: docs/spec/rules.md B-9, B-15; P3 depreciation review R3.
    /// SCENARIOS (the entry, or the original its reversal chain starts
    /// from, since reversing a reversal re-instates that original):
    ///   1. IF it is an asset's disposal entry:
    ///      - SHOULD refuse (AssetEntryIrreversible)
    ///      BECAUSE the register would still say disposed: un-disposing is
    ///      not a feature (correct with an entry by hand).
    ///   2. IF it is a depreciation run of a disposed asset (the disposal's
    ///      final depreciation included):
    ///      - SHOULD refuse
    ///      BECAUSE the disposal cleared what the runs posted, and took them
    ///      as final; reversing one would leave the asset's accumulated
    ///      depreciation behind it.
    ///   3. IF reversing re-instates a run whose period has been run again:
    ///      - SHOULD refuse, naming the run that posts it now
    ///      - NEVER carry one month's depreciation twice.
    ///   4. IF reversing re-instates a run that would carry the books past
    ///      the schedule at a month end from the reversal's month on (a
    ///      later run caught the month up: ledger_core::catch_up_room):
    ///      - SHOULD refuse, naming the room left
    ///      - NEVER let accumulated depreciation pass the schedule, or cost
    ///        less salvage.
    ///   5. IF it is a depreciation run or a schedule month's posting, or a
    ///      reversal in the chain of one, and `date` is before the entry's
    ///      own date:
    ///      - SHOULD refuse
    ///      - NEVER carry a correction at month ends the entry it corrects
    ///        never reached (a run of March reversed on Jan 31 would leave
    ///        January and February with negative depreciation, and a
    ///        disposal in February clearing what they don't carry)
    ///      BECAUSE a correction comes after what it corrects (B-9, Pacioli
    ///      ch. 31: "after you have made this correction"). Commands only:
    ///      a journal an earlier version wrote replays as it was (B-28).
    ///   6. Otherwise: a run of an active asset is un-posted (its month is
    ///      due again, caught up by the next run).
    /// ```
    pub fn check_reverse_entry(
        &self,
        entry: EntryId,
        date: chrono::NaiveDate,
    ) -> Result<(), StoreError> {
        self.ledger.check_reversal(entry, date)?;
        let top = self
            .ledger
            .reversal_chain(entry)
            .first()
            .copied()
            .unwrap_or(entry);
        let refuse = |why: String| Err(LedgerError::AssetEntryIrreversible(entry, why).into());
        // SCENARIO 5: the date of the entry reversed (it ends its chain).
        let dated = self
            .ledger
            .get_entry(entry)
            .map(ledger_core::JournalEntry::date);
        let before_it = dated.filter(|&d| date < d);
        if let (Some(on), Some((sid, month))) = (before_it, self.ledger.schedule_posting(top)) {
            return Err(LedgerError::InvalidSchedule(format!(
                "entry {entry} (schedule {sid}'s {month}) is dated {on}: a reversal of it can't be dated before it, on {date}"
            ))
            .into());
        }
        let payloads = |t: &str| -> Result<Vec<Value>, StoreError> {
            let mut stmt = self
                .conn
                .prepare("SELECT payload FROM events WHERE type = ? ORDER BY seq")?;
            let rows = stmt.query_map(params![t], |r| r.get::<_, String>(0))?;
            rows.map(|r| Ok(serde_json::from_str(&r?)?)).collect()
        };
        let what = if top == entry {
            "it is"
        } else {
            "reversing it re-instates"
        };
        for p in payloads("asset.disposed")? {
            if p.get("entry_id").and_then(Value::as_u64) == Some(top) {
                let asset = p.get("asset_id").and_then(Value::as_u64).unwrap_or(0);
                return refuse(format!(
                    "{what} the disposal of asset {asset}, and an asset can't be un-disposed (correct it with an entry by hand)"
                ));
            }
        }
        for p in payloads("asset.depreciation_posted")? {
            if p.get("entry_id").and_then(Value::as_u64) != Some(top) {
                continue;
            }
            let asset_id = get_u64(&p, "asset_id")?;
            let period = get_str(&p, "period")?;
            if let Some(on) = before_it {
                return refuse(format!(
                    "it is dated {on}, and a reversal of asset {asset_id}'s depreciation for {period} can't be dated before it, on {date}"
                ));
            }
            if self
                .ledger
                .get_asset(asset_id)
                .is_some_and(|a| a.status != AssetStatus::Active)
            {
                return refuse(format!(
                    "{what} depreciation of asset {asset_id}, which is disposed: its disposal cleared what the runs posted (correct it with an entry by hand)"
                ));
            }
            if self.ledger.in_effect(top) {
                continue;
            }
            // Reversing `entry` re-instates the run, adding it back in the
            // reversal's month.
            if let Some((now, _)) = self.depreciation_posted_entry(asset_id, period)
                && now != top
            {
                return refuse(format!(
                    "it would post asset {asset_id}'s depreciation for {period} a second time: entry {now} posts it now (reverse that one instead)"
                ));
            }
            let amount: Decimal = get_str(&p, "amount")?
                .parse()
                .map_err(|_| StoreError::JournalCorrupt("bad amount".into()))?;
            if let Some(asset) = self.ledger.get_asset(asset_id) {
                let room = ledger_core::catch_up_room(
                    asset,
                    date.year(),
                    date.month(),
                    &self.depreciation_effects(asset_id),
                )?
                .amount;
                if amount > room {
                    return refuse(format!(
                        "re-instating asset {asset_id}'s depreciation for {period} ({amount}) on {date} would carry more than its schedule: the books have room for {room} from {} on (a later run caught the month up; run a month again instead, which posts what is due)",
                        date.format("%Y-%m")
                    ));
                }
            }
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // Fixed assets
    // ------------------------------------------------------------------

    /// Register a fixed asset. Journaled as `asset.registered`.
    pub fn register_asset(&mut self, new: NewAsset) -> Result<AssetId, StoreError> {
        self.rehearse(|l| l.register_asset(new.clone()))?;
        // Pure pre-validation so the journal write cannot be followed by a
        // failed mutation.
        self.ledger.validate_new_asset(&new)?;
        let id = self.ledger.next_asset_id();
        let line = self.append_journal(
            "asset.registered",
            json!({
                "id": id,
                "description": new.description,
                // The in-service date, as every asset.registered names it.
                "purchase_date": new.in_service_date.to_string(),
                "cost": { "amount": new.cost.amount.to_string(), "currency": new.cost.currency.code() },
                "salvage_value": { "amount": new.salvage_value.amount.to_string(), "currency": new.salvage_value.currency.code() },
                "useful_life_months": new.useful_life_months,
                "method": "straight_line",
                "cost_account": new.cost_account,
                "accum_depr_account": new.accum_depr_account,
                "expense_account": new.expense_account,
            }),
        )?;
        let live = self.ledger.register_asset(new)?;
        debug_assert_eq!(live, id, "id allocator diverged from journal");
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(live)
    }

    /// Give an asset a tag (see `Ledger::set_asset_tag`). Journaled as
    /// `asset.tagged`, with the tag it had.
    pub fn set_asset_tag(&mut self, id: AssetId, tag: &str) -> Result<(), StoreError> {
        self.ledger.check_asset_tag_for(Some(id), tag)?;
        let previous = self.ledger.get_asset(id).and_then(|a| a.tag.clone());
        let line = self.append_journal(
            "asset.tagged",
            json!({"asset_id": id, "tag": tag, "previous_tag": previous}),
        )?;
        self.ledger.set_asset_tag(id, tag)?;
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Parties (ledger_core::parties)
    // ------------------------------------------------------------------

    /// Add a party. Journaled as `party.added`.
    pub fn add_party(
        &mut self,
        name: &str,
        roles: &[ledger_core::Role],
    ) -> Result<ledger_core::PartyId, StoreError> {
        self.ledger.check_add_party(name, roles)?;
        let id = self.ledger.next_party_id();
        let line = self.append_journal(
            "party.added",
            json!({"id": id, "name": name.trim(), "roles": roles.iter().map(|r| r.key()).collect::<Vec<_>>()}),
        )?;
        let got = self.ledger.add_party(name, roles)?;
        debug_assert_eq!(got, id, "party id allocator diverged from journal");
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(got)
    }

    /// Change one of a party's facts. Journaled as `party.updated`.
    ///
    /// An email or a default terms fact needs the `documents_v2` feature
    /// (plan P4b S4.2): books that haven't declared it refuse the write
    /// with the upgrade remedy, journaling nothing.
    pub fn update_party(
        &mut self,
        id: ledger_core::PartyId,
        field: ledger_core::PartyField,
    ) -> Result<(), StoreError> {
        self.ledger.check_update_party(id, &field)?;
        if field.needs_documents_v2() {
            self.ensure_feature(crate::format::DOCUMENTS_V2)?;
        }
        let line = self.append_journal("party.updated", json!({"id": id, "change": field}))?;
        self.ledger.update_party(id, field)?;
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(())
    }

    /// Give or take a party role. Journaled as `party.role_set`.
    pub fn set_party_role(
        &mut self,
        id: ledger_core::PartyId,
        role: ledger_core::Role,
        on: bool,
    ) -> Result<(), StoreError> {
        self.ledger.check_set_party_role(id, role, on)?;
        let line = self.append_journal(
            "party.role_set",
            json!({"id": id, "role": role.key(), "on": on}),
        )?;
        self.ledger.set_party_role(id, role, on)?;
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(())
    }

    /// Name a party the creditor of a liability account the company owes
    /// it itself on, or stop (`on` false): ledger_core's
    /// `Ledger::set_party_creditor`. Journaled as `party.creditor_set
    /// {id, account_id, on}`, which needs the journal feature
    /// [`crate::format::PARTY_CREDITORS`], so an older wanderbooks stops at
    /// the declaration and says it is too old instead of misreading the
    /// books. Books that haven't declared it (an older version's, until
    /// their format upgrade) refuse it, FormatUpgradePending, journaling
    /// nothing.
    pub fn set_party_creditor(
        &mut self,
        id: ledger_core::PartyId,
        account: AccountId,
        on: bool,
    ) -> Result<(), StoreError> {
        self.ledger.check_set_party_creditor(id, account, on)?;
        self.ensure_feature(crate::format::PARTY_CREDITORS)?;
        let line = self.append_journal(
            "party.creditor_set",
            json!({"id": id, "account_id": account, "on": on}),
        )?;
        self.ledger.set_party_creditor(id, account, on)?;
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(())
    }

    /// Record or clear a party's W-9 facts. Journaled as `party.w9_set`.
    pub fn set_party_w9(
        &mut self,
        id: ledger_core::PartyId,
        facts: Option<ledger_core::W9Facts>,
    ) -> Result<(), StoreError> {
        self.ledger.check_set_party_w9(id, facts.as_ref())?;
        let line = self.append_journal("party.w9_set", json!({"id": id, "facts": facts}))?;
        self.ledger.set_party_w9(id, facts)?;
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(())
    }

    /// Record or clear a party's taxpayer-ID facts (the TIN's type, last
    /// four, and vault record; never the number). Journaled as
    /// `party.tin_set`.
    pub fn set_party_tin(
        &mut self,
        id: ledger_core::PartyId,
        facts: Option<ledger_core::TinFacts>,
    ) -> Result<(), StoreError> {
        self.ledger.check_set_party_tin(id, facts.as_ref())?;
        let line = self.append_journal("party.tin_set", json!({"id": id, "facts": facts}))?;
        self.ledger.set_party_tin(id, facts)?;
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(())
    }

    /// Record or clear a party's W-4 facts. Journaled as `party.w4_set`.
    pub fn set_party_w4(
        &mut self,
        id: ledger_core::PartyId,
        facts: Option<ledger_core::W4Facts>,
    ) -> Result<(), StoreError> {
        self.ledger.check_set_party_w4(id, facts.as_ref())?;
        let line = self.append_journal("party.w4_set", json!({"id": id, "facts": facts}))?;
        self.ledger.set_party_w4(id, facts)?;
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(())
    }

    /// Tag an entry with its party, or clear the tag. Journaled as
    /// `entry.party_set`.
    pub fn set_entry_party(
        &mut self,
        entry: EntryId,
        party: Option<ledger_core::PartyId>,
    ) -> Result<(), StoreError> {
        let mut rehearsal = self.ledger.clone();
        rehearsal.set_entry_party(entry, party)?;
        let line = self.append_journal(
            "entry.party_set",
            json!({"entry_id": entry, "party_id": party}),
        )?;
        self.ledger.set_entry_party(entry, party)?;
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(())
    }

    /// Name the parties an entry was with, each with their share, or clear
    /// them (an empty list): `Ledger::set_entry_parties`. Checked before
    /// journaling, so a refusal journals nothing. Journaled as
    /// `entry.parties_set {entry_id, parties: [{party_id, share}]}`, a share
    /// being `{"amount": "30.00"}`, `{"percent": "60"}`, or `"derived"`.
    /// Whether the shares resolve against the entry's lines depends on the
    /// settings: callers check `ledger_core::shares::resolve` first.
    pub fn set_entry_parties(
        &mut self,
        entry: EntryId,
        parties: &[ledger_core::PartyShare],
    ) -> Result<(), StoreError> {
        self.ledger.check_set_entry_parties(entry, parties)?;
        let line = self.append_journal(
            "entry.parties_set",
            json!({"entry_id": entry, "parties": parties}),
        )?;
        self.ledger.set_entry_parties(entry, parties)?;
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(())
    }

    /// Mark an entry as having no counterparty, with why
    /// (`Ledger::set_entry_no_party`). Journaled as `entry.no_party
    /// {entry_id, reason}`.
    pub fn set_entry_no_party(&mut self, entry: EntryId, reason: &str) -> Result<(), StoreError> {
        self.ledger.check_set_entry_no_party(entry, reason)?;
        let line = self.append_journal(
            "entry.no_party",
            json!({"entry_id": entry, "reason": reason.trim()}),
        )?;
        self.ledger.set_entry_no_party(entry, reason)?;
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(())
    }

    /// Correct a registered asset's cost, salvage, life, or in-service date
    /// (see `Ledger::correct_asset`), with the reason. Refused while the
    /// books carry any depreciation for it (a run posted and not reversed:
    /// `Ledger::accumulated_posted`). Journaled as `asset.corrected`, with
    /// the figures it had.
    pub fn correct_asset(
        &mut self,
        id: AssetId,
        fix: ledger_core::AssetCorrection,
        reason: &str,
    ) -> Result<(), StoreError> {
        if reason.trim().is_empty() {
            return Err(StoreError::ReasonRequired);
        }
        self.ledger.check_correct_asset(id, &fix)?;
        if !self
            .ledger
            .accumulated_posted(id, chrono::NaiveDate::MAX)?
            .is_zero()
        {
            return Err(StoreError::AssetDepreciationPosted(id));
        }
        let a = self.ledger.get_asset(id).expect("checked").clone();
        let money =
            |m: &Money| json!({"amount": m.amount.to_string(), "currency": m.currency.code()});
        let line = self.append_journal(
            "asset.corrected",
            json!({
                "asset_id": id,
                "cost": money(&fix.cost),
                "salvage_value": money(&fix.salvage_value),
                "useful_life_months": fix.useful_life_months,
                // The in-service date, as every asset.corrected names it.
                "purchase_date": fix.in_service_date.to_string(),
                "reason": reason,
                "previous": {
                    "cost": money(&a.cost),
                    "salvage_value": money(&a.salvage_value),
                    "useful_life_months": a.useful_life_months,
                    "purchase_date": a.in_service_date.to_string(),
                },
            }),
        )?;
        self.ledger.correct_asset(id, fix)?;
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(())
    }

    /// Record how the company got an asset and the facts its tax figures
    /// follow from (see `Ledger::set_asset_basis`). Journaled as
    /// `asset.basis_set {asset_id, rule, contributor_adjusted_basis,
    /// fmv_at_contribution, sec704c_method, source, tax_basis_723,
    /// depreciable_basis, built_in_gain_704c}`: the facts, and for the
    /// line's readers the figures as derived when written (null for a
    /// purchase's depreciable basis: the cost, whatever it becomes).
    /// Replay reads the facts and never recomputes the figures (rules.md
    /// B-28). It needs the journal feature `asset_basis`: refused,
    /// journaling nothing, in books that haven't declared it (books an
    /// older version wrote, until their format upgrade: B-27).
    pub fn set_asset_basis(
        &mut self,
        id: AssetId,
        basis: ledger_core::AssetBasis,
    ) -> Result<(), StoreError> {
        self.ledger.check_asset_basis(id, &basis)?;
        let mut asset = self.ledger.get_asset(id).expect("checked").clone();
        asset.basis = Some(basis.clone());
        let figures = asset.basis_figures()?.expect("a checked basis has figures");
        let dec = |d: Option<Decimal>| d.map(|d| d.to_string());
        self.ensure_feature(crate::format::ASSET_BASIS)?;
        let line = self.append_journal(
            "asset.basis_set",
            json!({
                "asset_id": id,
                "rule": basis.rule.key(),
                "contributor_adjusted_basis": dec(basis.contributor_adjusted_basis),
                "fmv_at_contribution": dec(basis.fmv_at_contribution),
                "sec704c_method": basis.sec704c_method.map(|m| m.key()),
                "source": basis.source,
                "tax_basis_723": dec(figures.tax_basis_723),
                "depreciable_basis": dec(basis
                    .rule
                    .is_contribution()
                    .then_some(figures.depreciable_basis)),
                "built_in_gain_704c": dec(figures.built_in_gain_704c),
            }),
        )?;
        self.ledger.set_asset_basis(id, basis)?;
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(())
    }

    /// Change an asset's description (see `Ledger::set_asset_description`).
    /// Journaled as `asset.described`, with the description it had.
    pub fn set_asset_description(
        &mut self,
        id: AssetId,
        description: &str,
    ) -> Result<(), StoreError> {
        self.ledger.check_asset_description(id, description)?;
        let previous = self
            .ledger
            .get_asset(id)
            .map(|a| a.description.clone())
            .unwrap_or_default();
        let line = self.append_journal(
            "asset.described",
            json!({"asset_id": id, "description": description, "previous_description": previous}),
        )?;
        self.ledger.set_asset_description(id, description)?;
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(())
    }

    /// Link an asset to the entry that put it on the books (see
    /// `Ledger::set_asset_acquisition`). Journaled as
    /// `asset.acquisition_linked`.
    pub fn set_asset_acquisition(&mut self, id: AssetId, entry: EntryId) -> Result<(), StoreError> {
        self.ledger.check_asset_acquisition(id, entry)?;
        let line = self.append_journal(
            "asset.acquisition_linked",
            json!({"asset_id": id, "entry_id": entry}),
        )?;
        self.ledger.set_asset_acquisition(id, entry)?;
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(())
    }

    /// Record or clear an asset's serial number (see
    /// `Ledger::set_asset_serial`). Journaled as `asset.serial_set`.
    pub fn set_asset_serial(
        &mut self,
        id: AssetId,
        serial: Option<&str>,
    ) -> Result<(), StoreError> {
        self.ledger.check_asset_serial(id, serial)?;
        let line = self.append_journal(
            "asset.serial_set",
            json!({"asset_id": id, "serial": serial}),
        )?;
        self.ledger.set_asset_serial(id, serial)?;
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(())
    }

    /// The depreciation runs the books carry for an asset: every run
    /// recorded for it (`asset.depreciation_posted`, its disposal's final
    /// depreciation included: [`Ledger::depreciation_runs`]) whose entry is
    /// in effect, in journal order. A reversed run is left out, so its
    /// month is due again (P3 depreciation review R3); a reversed reversal
    /// counts again, even after its month was run again and that run
    /// reversed (P3 review round 2).
    fn live_runs(&self, asset_id: AssetId) -> Vec<LiveRun> {
        self.ledger
            .depreciation_runs(asset_id)
            .iter()
            .filter(|r| self.ledger.in_effect(r.entry))
            .map(|r| LiveRun {
                period: r.period.clone(),
                entry_id: r.entry,
                amount: r.amount,
            })
            .collect()
    }

    /// The run the books carry for this asset and period, if any: (entry,
    /// amount) ([`Ledger::live_depreciation_run`]).
    fn depreciation_posted_entry(&self, asset_id: AssetId, period: &str) -> Option<(u64, Decimal)> {
        self.ledger
            .live_depreciation_run(asset_id, period)
            .map(|r| (r.entry, r.amount))
    }

    /// Whether this asset's period was ever run, reversed or not: running
    /// it again writes a second `asset.depreciation_posted` for it, which an
    /// older version would ignore (feature `month_reposting`).
    fn depreciation_period_was_run(&self, asset_id: AssetId, period: &str) -> bool {
        self.ledger
            .depreciation_runs(asset_id)
            .iter()
            .any(|r| r.period == period)
    }

    /// What the books carry for an asset's depreciation, entry by entry,
    /// each in its own month: every run and every reversal of one
    /// (ledger_core::depreciation_effects), as the catch-up rule reads it.
    fn depreciation_effects(&self, asset_id: AssetId) -> Vec<ledger_core::PostedRun> {
        self.ledger.depreciation_effects_of(asset_id)
    }

    /// Post one month's depreciation for one asset: what is due through the
    /// month's end (ledger_core::run_amount), dated then, Dr depreciation
    /// expense / Cr accumulated depreciation.
    ///
    /// Depreciation is an allocation, not money movement, so it is exempt
    /// from the receipt policy by construction: the registered asset and its
    /// schedule are the supporting documentation.
    ///
    /// ```text
    /// GOAL: the books carry the schedule through the month's end.
    /// SOURCE: docs/spec/rules.md B-15; plan/served.md P3 "Settled in
    ///         design" (2026-10-20): a month skipped is caught up in the
    ///         month the run is made (an immaterial miss; a material one is
    ///         restated, ASC 250, by a human and the CPA).
    /// INVARIANTS:
    ///   1. Idempotent: a period the books carry a run for returns that run
    ///      (already_posted), journaling nothing.
    ///   2. What posts is ledger_core::run_amount over what the books carry,
    ///      every run and reversal in its own month (Self::depreciation_effects):
    ///      earlier months skipped or reversed are caught up in this one,
    ///      and the books never carry more than the schedule at a month end
    ///      a run reached; a month whose run was reversed after it ended
    ///      isn't run twice (the reversal's month catches it up).
    ///   3. Nothing due (the month rounds to nothing, a later run caught it
    ///      up, the life is over and posted) journals nothing: an entry
    ///      can't carry a zero line (P3 depreciation review R10).
    ///   4. A period run before and reversed is run again only in books
    ///      that declare journal feature month_reposting (books an older
    ///      version wrote: after their format upgrade); refused before
    ///      anything is journaled otherwise.
    /// SCENARIOS:
    ///   1. IF the asset is unknown or disposed, the period malformed, or
    ///      it ends before the asset was placed in service: SHOULD refuse.
    ///   2. IF the period is after the life and something is still due (its
    ///      final month skipped, or in a closed period): SHOULD post it.
    /// ```
    pub fn run_depreciation(
        &mut self,
        asset_id: AssetId,
        period: &str,
    ) -> Result<DepreciationRun, StoreError> {
        let (year, month) = parse_period(period)?;
        let asset = self
            .ledger
            .get_asset(asset_id)
            .ok_or(LedgerError::UnknownAsset(asset_id))?
            .clone();
        let period_end = last_day_of_month(year, month)?;
        let own = |a: &ledger_core::Asset| -> Result<Decimal, StoreError> {
            Ok(ledger_core::depreciation_schedule(a, period_end)?
                .into_iter()
                .find(|l| l.period == period)
                .map_or(Decimal::ZERO, |l| l.amount))
        };
        if let Some((entry_id, amount)) = self.depreciation_posted_entry(asset_id, period) {
            return Ok(DepreciationRun {
                asset_id,
                period: period.to_string(),
                amount,
                scheduled: own(&asset)?,
                entry_id: Some(entry_id),
                already_posted: true,
                nothing_due: None,
            });
        }
        if asset.status != AssetStatus::Active {
            return Err(LedgerError::AssetAlreadyDisposed(asset_id).into());
        }
        let due =
            ledger_core::run_amount(&asset, year, month, &self.depreciation_effects(asset_id))?;
        if due.amount.is_zero() {
            let last = ledger_core::life_end(&asset)?
                .pred_opt()
                .map(|d| (d.year(), d.month()));
            let why = if !due.over.is_zero() {
                format!(
                    "the books carry {} more than the schedule through {period}, which no run takes back (a correction for a human)",
                    due.over
                )
            } else if last.is_some_and(|l| (year, month) > l) {
                "the asset's life is over, and the books carry all of its depreciation".to_string()
            } else if due.scheduled.is_zero() {
                format!("{period}'s depreciation rounds to nothing")
            } else {
                format!(
                    "the books already carry the schedule through {period} (a later run caught it up, or its run was reversed after the month ended: the run of the reversal's month catches that up)"
                )
            };
            return Ok(DepreciationRun {
                asset_id,
                period: period.to_string(),
                amount: Decimal::ZERO,
                scheduled: due.scheduled,
                entry_id: None,
                already_posted: false,
                nothing_due: Some(why),
            });
        }
        let amount = due.amount;
        let description = format!("Depreciation — {} — {period}", asset.description);
        let money = Money::new(amount, self.base_currency);
        // Every refusal before the first journaled line: the entry on a
        // copy (and that it is the period's run, no more and no less:
        // Ledger::check_depreciation_run), then the journal feature a
        // period run again needs. A refusal after the entry posted would
        // leave it without the line that makes it the period's run.
        self.rehearse(|l| {
            let e = l.try_create_entry(period_end, &description)?;
            l.add_line(e, asset.expense_account, Side::Debit, money)?;
            l.add_line(e, asset.accum_depr_account, Side::Credit, money)?;
            l.post_entry(e)?;
            l.check_depreciation_run(asset_id, period, e, amount)
        })?;
        if self.depreciation_period_was_run(asset_id, period) {
            self.ensure_feature(crate::format::MONTH_REPOSTING)?;
        }
        // More or less than the month's own (another month caught up in
        // it): a run an earlier version would read as the month's own.
        if amount != due.scheduled {
            self.ensure_feature(crate::format::DEPRECIATION_CATCH_UP)?;
        }
        let entry_id = self.create_entry(period_end, &description)?;
        self.add_line(entry_id, asset.expense_account, Side::Debit, money)?;
        self.add_line(entry_id, asset.accum_depr_account, Side::Credit, money)?;
        self.post_entry_unchecked(entry_id)?;
        let dline = self.append_journal(
            "asset.depreciation_posted",
            json!({
                "asset_id": asset_id,
                "period": period,
                "entry_id": entry_id,
                "amount": amount.to_string(),
            }),
        )?;
        self.ledger
            .record_depreciation_run(asset_id, period, entry_id, amount)?;
        apply_line_to_sqlite(&self.conn, &dline)?;
        Ok(DepreciationRun {
            asset_id,
            period: period.to_string(),
            amount,
            scheduled: due.scheduled,
            entry_id: Some(entry_id),
            already_posted: false,
            nothing_due: None,
        })
    }

    /// The disposal entry's lines for an asset, its preview, and the
    /// accounts chosen: the one place the shape of a disposal is decided,
    /// for both the preview and the post. Zero lines are left out (an
    /// engine entry has none): scrapping brings no proceeds, and an asset
    /// disposed before its first depreciation has none accumulated.
    ///
    /// ```text
    /// INVARIANTS:
    ///   1. Dr accumulated depreciation by everything the books carry for
    ///      the asset (preview.accumulated), Dr proceeds, Cr cost; depreciation
    ///      the runs posted past the disposal day credited back to the
    ///      asset's depreciation expense (excess), or depreciation still due
    ///      debited to it (catch_up); the gain or loss the difference.
    ///   2. The lines balance, and the book value they take is cost less
    ///      the schedule through the disposal day (Ledger::dispose_asset
    ///      INVARIANT 1).
    /// ```
    pub fn disposal_lines(
        asset: &ledger_core::Asset,
        preview: &DisposalPreview,
        proceeds: Decimal,
        proceeds_account: AccountId,
        gain_loss_account: AccountId,
    ) -> Vec<(AccountId, Side, Decimal)> {
        [
            (asset.accum_depr_account, Side::Debit, preview.accumulated),
            (proceeds_account, Side::Debit, proceeds),
            (
                asset.expense_account,
                Side::Debit,
                preview.catch_up_depreciation,
            ),
            (asset.cost_account, Side::Credit, asset.cost.amount),
            (
                asset.expense_account,
                Side::Credit,
                preview.excess_depreciation,
            ),
            (gain_loss_account, Side::Credit, preview.gain_loss),
            (gain_loss_account, Side::Debit, -preview.gain_loss),
        ]
        .into_iter()
        .filter(|(_, _, a)| *a > Decimal::ZERO)
        .collect()
    }

    /// Refuse a disposal unless every depreciation run of the asset carries,
    /// at the end of the disposal's month, what it carries in the end (in
    /// effect, or reversed): the disposal entry clears what the runs carry
    /// in effect, so it must be what they carry at that month end.
    ///
    /// ```text
    /// GOAL: a disposal never leaves the books wrong at the end of its
    ///       month, nor in the end: the expense is the schedule through the
    ///       disposal day, and accumulated depreciation 0.00. It neither
    ///       makes nor hides an error at a later month end: what a run
    ///       carries there and not in the end (a later month's run reversed
    ///       after its month, a run reversed and re-instated after the
    ///       disposal's month) is that run's own error and correction, dated
    ///       when they were made, the same with or without the disposal.
    /// SOURCE: docs/spec/rules.md B-15 (a disposed asset depreciates through
    ///         its disposal day); B-9 (a mistake is corrected by an opposite
    ///         entry); plan/served.md P3, settled in design (a miss is
    ///         corrected in the period the correction is made, a material
    ///         one restated); Ledger::dispose_asset INVARIANT 1. P3 review
    ///         round 2: round 1 checked only the runs of the disposal's month
    ///         and before at later month ends, which refused a disposal for
    ///         ever once the month that could undo a change was closed, and
    ///         let a later month's run reversed on a day in or before the
    ///         disposal's month through (its reversal charged the disposal
    ///         month, the disposal charged the month again).
    /// SCENARIOS:
    ///   1. IF July ran and the asset is disposed of on Jun 15:
    ///      - SHOULD refuse, naming July's run: reverse it first (dated
    ///        when the correction is made; a reversal dated after July
    ///        leaves July's books as they were, the correction a later
    ///        month's)
    ///      - NEVER credit July back at Jun 15 (June's expense 1,000.00
    ///        short, its accumulated depreciation a 1,000.00 debit)
    ///   2. IF the disposal month's own run (dated its end) stands: SHOULD
    ///      allow it (no month end falls between; the disposal entry credits
    ///      the days after the disposal back).
    ///   3. IF May's run was reversed on Aug 1 and the asset is disposed of
    ///      on Jun 15: SHOULD refuse (the disposal would charge May again
    ///      at Jun 15, and the reversal take it away only in August), until
    ///      that reversal is itself reversed (May stands again), on any day.
    ///   4. IF March's run was reversed on Jan 31 (a reversal dated before
    ///      its run) and the asset is disposed of on Feb 15: SHOULD refuse
    ///      (February's end would carry the reversal and the disposal's
    ///      whole charge through Feb 15).
    /// ```
    fn check_nothing_after_disposal(
        &self,
        asset_id: AssetId,
        date: NaiveDate,
    ) -> Result<(), StoreError> {
        let month = (date.year(), date.month());
        let month_end = ledger_core::schedules::month_end(date);
        let (mut later, mut late) = (Vec::new(), Vec::new());
        for r in self.ledger.depreciation_runs(asset_id) {
            let (run, amount, period) = (r.entry, r.amount, &r.period);
            if period_month(period) > month && self.ledger.in_effect(run) {
                later.push(format!("{period} (entry {run})"));
                continue;
            }
            // The run's chain, each entry dated: what it carries at the
            // disposal month's end must be what it carries in the end.
            let dated: Vec<(EntryId, NaiveDate, Decimal)> = self
                .ledger
                .reversal_chain(run)
                .into_iter()
                .skip_while(|&e| e != run)
                .enumerate()
                .filter_map(|(i, e)| {
                    let d = self.ledger.get_entry(e)?.date();
                    Some((e, d, if i % 2 == 0 { amount } else { -amount }))
                })
                .collect();
            let at_disposal: Decimal = dated
                .iter()
                .filter(|(_, d, _)| *d <= month_end)
                .map(|(_, _, a)| *a)
                .sum();
            let in_the_end: Decimal = dated.iter().map(|(_, _, a)| *a).sum();
            if at_disposal != in_the_end {
                // The entries the difference is in: those dated after the
                // month (or the run itself, dated after a reversal of it).
                let after: Vec<String> = dated
                    .iter()
                    .filter(|(_, d, _)| *d > month_end)
                    .map(|(e, d, _)| format!("entry {e} on {d}"))
                    .collect();
                late.push(format!(
                    "{period}'s run (entry {run}), by {}",
                    after.join(", ")
                ));
            }
        }
        if !later.is_empty() {
            return Err(LedgerError::DepreciationPastDisposal(format!(
                "asset {asset_id} has depreciation runs for months after {}, the disposal's month: {}; reverse them first (books.reverse_entry), then dispose of it",
                date.format("%Y-%m"),
                later.join(", ")
            ))
            .into());
        }
        if !late.is_empty() {
            return Err(LedgerError::DepreciationPastDisposal(format!(
                "what asset {asset_id}'s runs carry at {month_end}, the end of the disposal's month, isn't what they carry now: {}; the disposal clears what they carry now, so make the two agree first (reverse the latest of those entries, on any open day)",
                late.join("; ")
            ))
            .into());
        }
        Ok(())
    }

    /// Compute a disposal without posting: the schedule through the
    /// disposal day against what the books carry ([`Store::dispose_asset`]
    /// posts it).
    ///
    /// ```text
    /// GOAL: a disposal takes exactly the depreciation of the days held,
    ///       whatever the runs posted.
    /// SOURCE: docs/spec/rules.md B-15 (the disposal day is held);
    ///         Ledger::dispose_asset INVARIANTS 1, 2; P3 depreciation
    ///         review R2.
    /// INVARIANTS:
    ///   1. target = the schedule through the disposal day; posted = the
    ///      runs the books carry (reversed ones left out); (up, down) =
    ///      kernel::true_up(target, posted).
    ///   2. book_value = cost − target; gain_loss = proceeds − book_value.
    ///   3. up is posted as the final depreciation (its own entry), or, when
    ///      the disposal month already has a run (which can't be posted
    ///      twice), as catch_up_depreciation in the disposal entry; down is
    ///      excess_depreciation, credited back in the disposal entry.
    ///   4. accumulated (cleared) = posted + final_depreciation: the asset
    ///      leaves the accumulated depreciation account at zero.
    /// SCENARIOS:
    ///   1. IF June ran (1,000.00 dated Jun 30) and the asset is disposed
    ///      of on Jun 15 for 7,000.00 (cost 12,000.00, 1,000.00 a month):
    ///      - SHOULD clear 6,000.00, credit back 500.00, book value
    ///        6,500.00, gain 500.00
    ///      - NEVER keep June's last 15 days of depreciation (a book value
    ///        of 6,000.00, a gain of 1,000.00)
    ///      BECAUSE the asset was held through Jun 15, not Jun 30.
    ///   2. IF a run carries at the end of the disposal's month other than
    ///      what it carries now (a run of a later month stands; a reversal
    ///      or re-instatement dated after the month isn't undone): SHOULD
    ///      refuse (DepreciationPastDisposal,
    ///      Self::check_nothing_after_disposal).
    /// ```
    pub fn dispose_preview(
        &self,
        asset_id: AssetId,
        date: chrono::NaiveDate,
        proceeds: Money,
    ) -> Result<DisposalPreview, StoreError> {
        let asset = self
            .ledger
            .get_asset(asset_id)
            .ok_or(LedgerError::UnknownAsset(asset_id))?;
        if asset.status != AssetStatus::Active {
            return Err(LedgerError::AssetAlreadyDisposed(asset_id).into());
        }
        if proceeds.amount < Decimal::ZERO {
            return Err(LedgerError::NegativeProceeds(proceeds.amount).into());
        }
        // Proceeds are a line of the disposal entry (P0 review round 3, S1).
        if !ledger_core::kernel::Amount::in_line_range(proceeds.amount) {
            return Err(LedgerError::AmountOutOfRange.into());
        }
        ledger_core::calendar::check(date)?;
        if proceeds.currency != self.base_currency {
            return Err(LedgerError::CurrencyMismatch {
                expected: self.base_currency.code().to_string(),
                found: proceeds.currency.code().to_string(),
            }
            .into());
        }
        let acquired = self
            .ledger
            .asset_acquired_date(asset_id)
            .expect("known asset");
        if date < acquired {
            return Err(LedgerError::DisposalBeforeAcquisition {
                date: date.to_string(),
                acquired: acquired.to_string(),
            }
            .into());
        }
        let period = date.format("%Y-%m").to_string();
        self.check_nothing_after_disposal(asset_id, date)?;
        let runs = self.live_runs(asset_id);
        let too_large =
            || StoreError::JournalCorrupt(format!("asset {asset_id}'s depreciation is too large"));
        let posted = runs
            .iter()
            .try_fold(Decimal::ZERO, |t, r| t.checked_add(r.amount))
            .ok_or_else(too_large)?;
        let target = ledger_core::accumulated_scheduled_through(asset, date)?;
        let (up, down) = ledger_core::kernel::true_up(target, posted).ok_or_else(too_large)?;
        let in_month = runs.iter().any(|r| r.period == period);
        let (final_depreciation, catch_up_depreciation) = if in_month {
            (Decimal::ZERO, up)
        } else {
            (up, Decimal::ZERO)
        };
        let accumulated = posted
            .checked_add(final_depreciation)
            .ok_or_else(too_large)?;
        let book_value = asset
            .cost
            .amount
            .checked_sub(target)
            .ok_or_else(too_large)?;
        Ok(DisposalPreview {
            asset_id,
            date,
            proceeds,
            accumulated,
            final_depreciation,
            excess_depreciation: down,
            catch_up_depreciation,
            book_value,
            gain_loss: proceeds
                .amount
                .checked_sub(book_value)
                .ok_or_else(too_large)?,
            period,
        })
    }

    /// Dispose of an asset: auto-post any unposted depreciation through the
    /// disposal date, then post the disposal entry (clear cost and
    /// accumulated depreciation, record proceeds and gain/loss) and mark
    /// the asset disposed. Returns the disposal entry's id.
    ///
    /// The disposal entry clears exactly the accumulated depreciation the
    /// books carry for the asset, and takes the book value the schedule
    /// gives through the disposal day ([`Store::dispose_preview`]): a run
    /// that reached past the disposal day (the disposal month's, dated its
    /// end) is taken back by a credit to depreciation expense, so the
    /// depreciation and the gain or loss are both of the days held.
    pub fn dispose_asset(
        &mut self,
        asset_id: AssetId,
        date: chrono::NaiveDate,
        proceeds: Money,
        proceeds_account: AccountId,
        gain_loss_account: AccountId,
    ) -> Result<EntryId, StoreError> {
        self.rehearse(|l| l.dispose_asset(asset_id, date, proceeds))?;
        let asset = self
            .ledger
            .get_asset(asset_id)
            .ok_or(LedgerError::UnknownAsset(asset_id))?
            .clone();
        // The receiving account holds cash (or a receivable): an asset.
        // The gain/loss account is a revenue or expense account.
        for (id, role) in [
            (proceeds_account, "proceeds"),
            (gain_loss_account, "gain/loss"),
        ] {
            let a = self
                .ledger
                .chart()
                .get(id)
                .ok_or(LedgerError::UnknownAccount(id))?;
            if a.placeholder {
                return Err(LedgerError::PlaceholderAccount(a.code.clone()).into());
            }
            let ok = match role {
                "proceeds" => a.account_type == AccountType::Asset,
                _ => {
                    a.account_type == AccountType::Revenue || a.account_type == AccountType::Expense
                }
            };
            if !ok {
                return Err(LedgerError::AssetAccountWrongType {
                    code: a.code.clone(),
                    role: "disposal",
                    expected: if role == "proceeds" {
                        "asset"
                    } else {
                        "revenue or expense"
                    },
                    found: match a.account_type {
                        AccountType::Asset => "asset",
                        AccountType::Liability => "liability",
                        AccountType::Equity => "equity",
                        AccountType::Revenue => "revenue",
                        AccountType::Expense => "expense",
                    },
                }
                .into());
            }
        }
        if proceeds.amount < Decimal::ZERO {
            return Err(LedgerError::NegativeProceeds(proceeds.amount).into());
        }
        // Proceeds are a line of the disposal entry (P0 review round 3, S1).
        if !ledger_core::kernel::Amount::in_line_range(proceeds.amount) {
            return Err(LedgerError::AmountOutOfRange.into());
        }
        ledger_core::calendar::check(date)?;
        if proceeds.currency != self.base_currency {
            return Err(LedgerError::CurrencyMismatch {
                expected: self.base_currency.code().to_string(),
                found: proceeds.currency.code().to_string(),
            }
            .into());
        }
        if asset.status != AssetStatus::Active {
            return Err(LedgerError::AssetAlreadyDisposed(asset_id).into());
        }
        let acquired = self
            .ledger
            .asset_acquired_date(asset_id)
            .expect("known asset");
        if date < acquired {
            return Err(LedgerError::DisposalBeforeAcquisition {
                date: date.to_string(),
                acquired: acquired.to_string(),
            }
            .into());
        }
        let base = self.base_currency;

        // One source of truth for the math: the preview computes the
        // posted-vs-scheduled reconciliation; posting follows it.
        let preview = self.dispose_preview(asset_id, date, proceeds)?;
        let depreciation = (preview.final_depreciation > Decimal::ZERO).then(|| {
            (
                format!(
                    "Depreciation — {} — {} (final)",
                    asset.description, preview.period
                ),
                vec![
                    (
                        asset.expense_account,
                        Side::Debit,
                        preview.final_depreciation,
                    ),
                    (
                        asset.accum_depr_account,
                        Side::Credit,
                        preview.final_depreciation,
                    ),
                ],
            )
        });
        let disposal = (
            format!("Disposal — {} — {}", asset.description, date),
            Self::disposal_lines(
                &asset,
                &preview,
                proceeds.amount,
                proceeds_account,
                gain_loss_account,
            ),
        );
        // Several commands make one disposal: rehearse them all together,
        // so a refusal anywhere leaves nothing journaled (the final
        // depreciation's entry checked as the period's run, no more and no
        // less: Ledger::check_depreciation_run).
        self.rehearse(|l| {
            for (i, (description, lines)) in depreciation.iter().chain([&disposal]).enumerate() {
                let e = l.try_create_entry(date, description)?;
                for (account, side, amount) in lines {
                    l.add_line(e, *account, *side, Money::new(*amount, base))?;
                }
                l.post_entry(e)?;
                if i == 0 && depreciation.is_some() {
                    l.check_depreciation_run(
                        asset_id,
                        &preview.period,
                        e,
                        preview.final_depreciation,
                    )?;
                }
            }
            l.dispose_asset(asset_id, date, proceeds)
        })?;
        // An enforced approval rule covering the disposal entry (the last
        // posted) refuses it before anything is journaled.
        self.approval_gate(
            |l| {
                for (description, lines) in depreciation.iter().chain([&disposal]) {
                    let e = l.try_create_entry(date, description)?;
                    for (account, side, amount) in lines {
                        l.add_line(e, *account, *side, Money::new(*amount, base))?;
                    }
                    l.post_entry(e)?;
                }
                l.dispose_asset(asset_id, date, proceeds)
            },
            true,
        )?;
        // The disposal month's run was reversed (else the final
        // depreciation would be in the disposal entry): its line again,
        // which needs a journal feature, asked for (and refused in books
        // that haven't declared it) before the first line is journaled.
        if depreciation.is_some() && self.depreciation_period_was_run(asset_id, &preview.period) {
            self.ensure_feature(crate::format::MONTH_REPOSTING)?;
        }
        // Disposed before it was placed in service (acquired, not yet in
        // use): an earlier version refuses that line (its floor was the
        // in-service date), so it needs its feature, asked here, before
        // the first line.
        if date < asset.in_service_date {
            self.ensure_feature(crate::format::DISPOSAL_BEFORE_SERVICE)?;
        }
        if let Some((description, lines)) = &depreciation {
            let eid = self.create_entry(date, description)?;
            for (account, side, amount) in lines {
                self.add_line(eid, *account, *side, Money::new(*amount, base))?;
            }
            self.post_entry_unchecked(eid)?;
            let dline = self.append_journal(
                "asset.depreciation_posted",
                json!({
                    "asset_id": asset_id,
                    "period": preview.period,
                    "entry_id": eid,
                    "amount": preview.final_depreciation.to_string(),
                }),
            )?;
            self.ledger.record_depreciation_run(
                asset_id,
                &preview.period,
                eid,
                preview.final_depreciation,
            )?;
            apply_line_to_sqlite(&self.conn, &dline)?;
        }
        let entry_id = self.create_entry(date, &disposal.0)?;
        for (account, side, amount) in &disposal.1 {
            self.add_line(entry_id, *account, *side, Money::new(*amount, base))?;
        }
        self.post_entry_unchecked(entry_id)?;
        let line = self.append_journal(
            "asset.disposed",
            json!({
                "asset_id": asset_id,
                "date": date.to_string(),
                "proceeds": { "amount": proceeds.amount.to_string(), "currency": proceeds.currency.code() },
                "proceeds_account": proceeds_account,
                "gain_loss_account": gain_loss_account,
                "entry_id": entry_id,
            }),
        )?;
        self.ledger.dispose_asset(asset_id, date, proceeds)?;
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(entry_id)
    }

    // ------------------------------------------------------------------
    // Accounting periods
    // ------------------------------------------------------------------

    /// Create an accounting period. Journaled as `period.created`.
    pub fn create_period(
        &mut self,
        name: &str,
        start: NaiveDate,
        end: NaiveDate,
    ) -> Result<PeriodId, StoreError> {
        self.rehearse(|l| l.create_period(name, start, end))?;
        // Pure pre-validation so the journal write cannot be followed by a
        // failed mutation.
        ledger_core::periods::validate_period_range(start, end)?;
        let id = self.ledger.next_period_id();
        let line = self.append_journal(
            "period.created",
            json!({
                "id": id,
                "name": name,
                "start": start.to_string(),
                "end": end.to_string(),
            }),
        )?;
        let live = self.ledger.create_period(name, start, end)?;
        debug_assert_eq!(live, id, "id allocator diverged from journal");
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(live)
    }

    /// Close an accounting period, auto-posting the closing entry (revenue
    /// and expense accounts zeroed into Retained Earnings). Journaled as
    /// `period.closed` with the closing entry's id and its full detail
    /// (date, description, lines).
    ///
    /// The closing entry is part of the close: it is NOT separately
    /// journaled as entry.created/entry.posted. Its detail rides in the
    /// period.closed payload so the SQLite projection can record it; the
    /// detail only exists after the close, so unlike most mutations this
    /// journals after mutating (the pre-close validation makes a failed
    /// close impossible, so the journal cannot be left behind). Replay
    /// re-runs the deterministic close logic and checks the closing entry
    /// id against the journal.
    pub fn close_period(&mut self, id: PeriodId) -> Result<Option<EntryId>, StoreError> {
        self.close_period_to(id, &CloseTo::RetainedEarnings { sweep: vec![] })
    }

    /// Close a period with its net income going to `to`: Retained Earnings,
    /// or members' capital accounts by share (see
    /// [`ledger_core::Ledger::close_period_to`]). The target rides in the
    /// journal so replay re-derives the identical closing entry.
    pub fn close_period_to(
        &mut self,
        id: PeriodId,
        to: &CloseTo,
    ) -> Result<Option<EntryId>, StoreError> {
        // Rehearsed whole: a doomed close journals nothing, not even the
        // Retained Earnings account creation below.
        self.rehearse(|l| l.close_period_to(id, to))?;
        if matches!(to, CloseTo::Members(_) | CloseTo::MembersCumulative { .. }) {
            // A close counted from its regime's start: an older version
            // would replay it under the run rule (format gate, B-27).
            // Refused before anything is journaled until the books
            // declare it.
            self.ensure_feature(crate::format::MEMBERS_CUMULATIVE)?;
        }
        if matches!(to, CloseTo::RetainedEarnings { .. }) {
            // Retained Earnings must exist before the close so its creation
            // (if needed) is journaled as account.added; the close's own
            // ensure then no-ops and replay stays deterministic.
            self.ensure_control_account(&ledger_core::RETAINED_EARNINGS)?;
        }
        let mut next = self.ledger.clone();
        let closing_entry = next.close_period_to(id, to)?;
        let entry_detail = match closing_entry {
            Some(eid) => {
                let e = next
                    .get_entry(eid)
                    .expect("close just posted the closing entry");
                Some(json!({
                    "id": eid,
                    "date": e.date().to_string(),
                    "description": e.description(),
                    "lines": e.lines().iter().map(journal_line_json).collect::<Vec<_>>(),
                }))
            }
            None => None,
        };
        let close_to = match to {
            CloseTo::RetainedEarnings { sweep } if sweep.is_empty() => json!("retained_earnings"),
            CloseTo::RetainedEarnings { sweep } => json!({"retained_earnings": {"sweep": sweep}}),
            CloseTo::Account { into, sweep } => json!({"account": {"into": into, "sweep": sweep}}),
            _ => {
                let members = to.members().expect("every other target is members");
                let mut v = json!({
                    "members": members
                        .iter()
                        .map(|m| json!({
                            "account": m.capital,
                            "share": m.share.to_string(),
                            "distributions": m.distributions,
                        }))
                        .collect::<Vec<_>>(),
                });
                // The rounding rule, so replay closes the same way. Closes
                // journaled before 2026-10-13 carry none (the last member
                // absorbed it).
                match to {
                    CloseTo::Members(_) | CloseTo::MembersCumulative { .. } => {
                        let rule = next
                            .get_period(id)
                            .and_then(|p| p.members_closed.as_ref())
                            .map(|m| m.rule);
                        let Some(ledger_core::MemberRule::Cumulative { since_seq }) = rule else {
                            unreachable!("a members close records its cumulative rule")
                        };
                        v["rounding"] = json!("cumulative_since");
                        v["since_seq"] = json!(since_seq);
                    }
                    CloseTo::MembersRun(_) => v["rounding"] = json!("cumulative"),
                    CloseTo::MembersPerClose(_) => v["rounding"] = json!("largest_remainder"),
                    _ => {}
                }
                v
            }
        };
        let line = self.append_journal(
            "period.closed",
            json!({
                "id": id,
                "closing_entry": closing_entry,
                "close_to": close_to,
                "entry": entry_detail,
            }),
        )?;
        self.ledger = next;
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(closing_entry)
    }

    // ------------------------------------------------------------------
    // Bank reconciliation
    // ------------------------------------------------------------------

    /// Mark a posted entry's line cleared (or uncleared) for bank
    /// reconciliation. Journaled as `line.cleared`.
    ///
    /// ```text
    /// GOAL: a recorded reconciliation stays proof: the lines it cleared
    ///       stay cleared while it stands.
    /// SOURCE: docs/parties-and-attachments.md "Reconciliations" (never
    ///         edited; void to redo).
    /// SCENARIOS:
    ///   1. IF the line is one a reconciliation not voided recorded as
    ///      cleared and `cleared` is false: SHOULD refuse
    ///      (ReconciliationRefused, naming it: void it first), journaling
    ///      nothing.
    ///   2. Clearing a line dated on or before a reconciliation (an
    ///      outstanding check that clears next month) is allowed: the
    ///      later statement shows it.
    ///   3. Otherwise as Ledger::mark_cleared.
    /// ```
    pub fn mark_cleared(
        &mut self,
        entry_id: EntryId,
        line_index: usize,
        cleared: bool,
    ) -> Result<(), StoreError> {
        if !cleared
            && let Some(r) = self
                .reconciliations(None)?
                .into_iter()
                .find(|r| r.voided.is_none() && r.cleared.contains(&(entry_id, line_index)))
        {
            return Err(StoreError::ReconciliationRefused(format!(
                "entry {entry_id} line {line_index} is cleared by reconciliation {} (account {} through {}): void that reconciliation (with a reason) to un-clear it",
                r.id, r.account_id, r.statement_date
            )));
        }
        let mut next = self.ledger.clone();
        next.mark_cleared(entry_id, line_index, cleared)?;
        let line = self.append_journal(
            "line.cleared",
            json!({
                "entry_id": entry_id,
                "line_index": line_index as u64,
                "cleared": cleared,
            }),
        )?;
        self.ledger = next;
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Accounts receivable / payable
    // ------------------------------------------------------------------

    /// Rehearse a command on a scratch copy of the ledger before anything
    /// is journaled, so every refusal the engine would make surfaces first.
    /// A journaled command the engine then refused could never be replayed,
    /// and the books would not reopen (found by the refusal property in
    /// `tests/store.rs`, 2026-10-13). Hand-written pre-checks drift from
    /// the engine; the engine itself cannot.
    fn rehearse<T>(
        &self,
        op: impl FnOnce(&mut Ledger) -> Result<T, LedgerError>,
    ) -> Result<(), StoreError> {
        op(&mut self.ledger.clone())?;
        Ok(())
    }

    /// Fetch a control role's account (the one assigned to it, else by
    /// code), creating it at its code through the journaled
    /// account path when missing. The ledger's internal ensure_account
    /// then no-ops, which keeps replay deterministic: every account
    /// creation is an `account.added` event. The engine's account
    /// (`Ledger::add_control_account`): never refused for its name or a
    /// missing tax category (rules.md B-32), so an invoice, a bill or a
    /// payroll run is never refused over a chart label.
    fn ensure_control_account(
        &mut self,
        c: &ledger_core::ControlAccount,
    ) -> Result<AccountId, StoreError> {
        if let Some(id) = self.ledger.role_account(c) {
            return Ok(id);
        }
        self.rehearse(|l| l.add_control_account(c))?;
        self.journal_account_added(c.code, c.name, c.account_type, None, false, |l| {
            l.add_control_account(c)
        })
    }

    /// Journal an auto-posted two-line entry (debit one account, credit
    /// another for the same amount): [`Self::journal_auto_lines`].
    fn journal_auto_entry(
        &mut self,
        date: NaiveDate,
        description: &str,
        debit_account: AccountId,
        credit_account: AccountId,
        amount: Decimal,
    ) -> Result<(EntryId, Vec<JournalLine>), StoreError> {
        self.journal_auto_lines(
            date,
            description,
            &[
                (debit_account, Side::Debit, amount, None),
                (credit_account, Side::Credit, amount, None),
            ],
        )
    }

    /// Journal an auto-posted entry of `lines` (account, side, amount,
    /// project) through the standard entry.created / entry.line_added /
    /// entry.posted flow, a tagged line's `entry.line_class_set` right
    /// after its `entry.line_added` (as `books.add_line` journals one).
    /// Returns the predicted entry id and the journaled lines; the caller
    /// mutates the ledger afterwards and applies the lines to SQLite.
    fn journal_auto_lines(
        &mut self,
        date: NaiveDate,
        description: &str,
        lines: &[(AccountId, Side, Decimal, Option<ClassId>)],
    ) -> Result<(EntryId, Vec<JournalLine>), StoreError> {
        let entry_id = self.ledger.next_entry_id();
        let base = self.base_currency;
        let mut journaled = Vec::with_capacity(lines.len() * 2 + 2);
        journaled.push(self.append_journal(
            "entry.created",
            json!({
                "id": entry_id,
                "date": date.to_string(),
                "description": description,
            }),
        )?);
        for (index, &(account, side, amount, class)) in lines.iter().enumerate() {
            journaled.push(self.append_journal(
                "entry.line_added",
                json!({
                    "id": entry_id,
                    "account": account,
                    "side": match side {
                        Side::Debit => "debit",
                        Side::Credit => "credit",
                    },
                    "amount": amount.to_string(),
                    "currency": base.code(),
                }),
            )?);
            if let Some(class) = class {
                journaled.push(self.append_journal(
                    "entry.line_class_set",
                    json!({
                        "entry_id": entry_id,
                        "line_index": index as u64,
                        "class_id": class,
                    }),
                )?);
            }
        }
        let posted: Vec<Value> = lines
            .iter()
            .map(|&(account, side, amount, class_id)| {
                journal_line_json(&ledger_core::JournalLine {
                    account,
                    side,
                    amount: Money::new(amount, base),
                    foreign_currency: None,
                    foreign_amount: None,
                    class_id,
                })
            })
            .collect();
        journaled.push(self.append_journal(
            "entry.posted",
            json!({
                "id": entry_id,
                "date": date.to_string(),
                "description": description,
                "reversal_of": Option::<u64>::None,
                "lines": posted,
            }),
        )?);
        Ok((entry_id, journaled))
    }

    /// The feature a document's lines need (rules.md B-27): a line naming
    /// a project is `document_line_projects`'s; a document naming none is
    /// written as every version before it wrote one.
    fn ensure_document_lines(&self, lines: &[DocumentLine]) -> Result<(), StoreError> {
        if lines.iter().any(|l| l.class_id.is_some()) {
            self.ensure_feature(crate::format::DOCUMENT_LINE_PROJECTS)?;
        }
        Ok(())
    }

    /// The feature a document's new fields need (rules.md B-27): an
    /// invoice or bill naming a number, terms, a snapshot, a reference, a
    /// memo or a duplicate's reason, or a line naming its account, quantity
    /// or unit price, needs `documents_v2` (plan P4b S4.3; D-52). A
    /// document naming none of them is written byte for byte as before and
    /// is never refused.
    fn ensure_document_fields(&self, fields: &DocumentFields<'_>) -> Result<(), StoreError> {
        if fields.any_new() {
            self.ensure_feature(crate::format::DOCUMENTS_V2)?;
        }
        Ok(())
    }

    /// Add a customer. Journaled as `customer.added`.
    pub fn add_customer(&mut self, name: &str) -> Result<CustomerId, StoreError> {
        self.rehearse(|l| l.add_customer(name))?;
        // The engine's only failure mode is an empty name; replicate it so
        // the journal write cannot be followed by a failed mutation.
        if name.trim().is_empty() {
            return Err(LedgerError::EmptyAssetDescription.into());
        }
        let id = self.ledger.next_customer_id();
        let line = self.append_journal("customer.added", json!({ "id": id, "name": name }))?;
        let live = self.ledger.add_customer(name)?;
        debug_assert_eq!(live, id, "id allocator diverged from journal");
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(live)
    }

    /// Add a vendor. Journaled as `vendor.added`.
    pub fn add_vendor(&mut self, name: &str) -> Result<VendorId, StoreError> {
        self.rehearse(|l| l.add_vendor(name))?;
        if name.trim().is_empty() {
            return Err(LedgerError::EmptyAssetDescription.into());
        }
        let id = self.ledger.next_vendor_id();
        let line = self.append_journal("vendor.added", json!({ "id": id, "name": name }))?;
        let live = self.ledger.add_vendor(name)?;
        debug_assert_eq!(live, id, "id allocator diverged from journal");
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(live)
    }

    /// Create a sales invoice. The auto-posted entry (Dr A/R, Cr Revenue)
    /// is journaled first through the standard entry flow, then the
    /// invoice itself as `invoice.created`.
    ///
    /// [`Self::create_invoice_with`] with no new fields: the invoice
    /// every version before `documents_v2` wrote.
    pub fn create_invoice(
        &mut self,
        customer_id: CustomerId,
        date: NaiveDate,
        due_date: NaiveDate,
        lines: Vec<DocumentLine>,
    ) -> Result<InvoiceId, StoreError> {
        self.create_invoice_with(
            customer_id,
            date,
            due_date,
            lines,
            arap::InvoiceFields::default(),
        )
    }

    /// Create a sales invoice with its new fields (number, terms, snapshots,
    /// references: plan P4b S4.3). The fields are journaled on
    /// `invoice.created` and restored by replay; a document whose fields are
    /// all `None` is journaled exactly as before
    /// ([`Self::create_invoice`]).
    ///
    /// ```text
    /// GOAL: a document's new fields are journaled with it, so replay can
    ///       never lose them, and a document naming none is written byte
    ///       for byte as an earlier version wrote it.
    /// SOURCE: docs/spec/rules.md B-27, B-40 (plan P4b S4.3); A-2, A-13.
    /// INVARIANTS:
    ///   1. A write naming a number, terms, a snapshot, a reference, a memo,
    ///      a line account, quantity or unit price MUST need journal feature
    ///      documents_v2: refused (FormatUpgradePending), journaling
    ///      nothing, in books that haven't declared it.
    ///   2. A refusal MUST leave the journal, the ledger and the projection
    ///      unchanged.
    ///   3. The event's fields MUST be exactly the ones given: no key is
    ///      written for a field that is None.
    /// ```
    pub fn create_invoice_with(
        &mut self,
        customer_id: CustomerId,
        date: NaiveDate,
        due_date: NaiveDate,
        lines: Vec<DocumentLine>,
        fields: arap::InvoiceFields,
    ) -> Result<InvoiceId, StoreError> {
        self.check_invoice_create(customer_id, date, due_date, &lines, &fields)?;
        let (invoice_id, entry_id, journaled) =
            self.journal_invoice(customer_id, date, due_date, &lines, &fields)?;
        let live = self
            .ledger
            .create_invoice_with(customer_id, date, due_date, lines, fields)?;
        debug_assert_eq!(live, invoice_id, "id allocator diverged from journal");
        debug_assert_eq!(
            self.ledger
                .get_invoice(live)
                .expect("invoice exists")
                .entry_id,
            entry_id,
            "invoice entry diverged from journal"
        );
        for line in &journaled {
            apply_line_to_sqlite(&self.conn, line)?;
        }
        Ok(live)
    }

    /// Everything [`Self::create_invoice_with`] refuses, before anything is
    /// journaled: the engine's whole check on a clone, its approval rules,
    /// and the journal format the fields need.
    fn check_invoice_create(
        &self,
        customer_id: CustomerId,
        date: NaiveDate,
        due_date: NaiveDate,
        lines: &[DocumentLine],
        fields: &arap::InvoiceFields,
    ) -> Result<(), StoreError> {
        self.rehearse(|l| {
            l.create_invoice_with(customer_id, date, due_date, lines.to_vec(), fields.clone())
        })?;
        self.approval_gate(
            |l| {
                l.create_invoice_with(customer_id, date, due_date, lines.to_vec(), fields.clone())?;
                Ok(())
            },
            false,
        )?;
        // Pure pre-validation mirroring create_invoice_steps, so the
        // journal writes below cannot be followed by a failed mutation.
        // (The auto-posted entry balances by construction and its accounts
        // are ensured below.)
        if self.ledger.get_customer(customer_id).is_none() {
            return Err(LedgerError::UnknownCustomer(customer_id).into());
        }
        arap::validate_lines(lines)?;
        arap::validate_due_date(date, due_date)?;
        if let Some(pid) = self.ledger.closed_period_containing(date) {
            return Err(LedgerError::ClosedPeriod(pid, date.to_string()).into());
        }
        if let Some(number) = &fields.number
            && self
                .ledger
                .invoices()
                .iter()
                .any(|i| i.number.as_deref() == Some(number.as_str()))
        {
            return Err(LedgerError::DuplicateInvoiceNumber(number.clone()).into());
        }
        self.ensure_document_lines(lines)?;
        self.ensure_document_fields(&DocumentFields {
            invoice: Some(fields),
            bill: None,
            lines,
        })?;
        Ok(())
    }

    /// Journal an invoice's auto-posted entry and its `invoice.created`
    /// event, without touching the live ledger. [`Self::check_invoice_create`]
    /// has run.
    fn journal_invoice(
        &mut self,
        customer_id: CustomerId,
        date: NaiveDate,
        due_date: NaiveDate,
        lines: &[DocumentLine],
        fields: &arap::InvoiceFields,
    ) -> Result<(InvoiceId, EntryId, Vec<JournalLine>), StoreError> {
        let ar = self.ensure_control_account(&ledger_core::RECEIVABLES)?;
        let revenue = self.ensure_control_account(&ledger_core::REVENUE)?;
        let invoice_id = self.ledger.next_invoice_id();
        let total: Decimal = lines.iter().map(|l| l.amount).sum();
        let description = format!("Invoice {invoice_id}");
        let mut posting = vec![(ar, Side::Debit, total, None)];
        posting.extend(arap::document_parts(lines, "the invoice")?.into_iter().map(
            |((account, class), amount)| (account.unwrap_or(revenue), Side::Credit, amount, class),
        ));
        let (entry_id, mut journaled) = self.journal_auto_lines(date, &description, &posting)?;
        let doc_lines: Vec<Value> = lines.iter().map(document_line_json).collect();
        let mut payload = json!({
            "id": invoice_id,
            "customer_id": customer_id,
            "date": date.to_string(),
            "due_date": due_date.to_string(),
            "lines": doc_lines,
            "entry_id": entry_id,
        });
        invoice_fields_json(&mut payload, fields);
        journaled.push(self.append_journal("invoice.created", payload)?);
        Ok((invoice_id, entry_id, journaled))
    }

    /// The issued copy of an invoice, filed in the call that issues it
    /// (plan P4b S4.3 item 11, A-7; rules.md B-40 item 3): the entry, then
    /// `invoice.created`, then `receipt.attached` on the entry
    /// (`INV-0007.pdf`, `application/pdf`, "Issued invoice"), then
    /// `attachment.linked` to the invoice. The server renders the PDF
    /// first, so a rendering failure never reaches the store.
    ///
    /// The copy is found through the link, never by guessing
    /// ([`Self::attachments_for`] with [`Subject::Invoice`]); its SHA-256
    /// is the `blob_sha` in `receipt.attached`. There is no `issued_pdf`
    /// field on the event: the attachment id is allocated after the entry
    /// exists (CD-A6).
    ///
    /// ```text
    /// GOAL: the copy sent is kept with the invoice, or nothing is.
    /// SOURCE: IRS Pub. 583 (keep invoices); Rev. Proc. 98-25 (the audit
    ///         trail keeps the record as issued); plan P4b S4.3, 8.3.11;
    ///         rules.md B-40 item 3.
    /// INVARIANTS:
    ///   1. Every refusal (an unknown customer, an invalid line or due
    ///      date, a duplicate number, a closed period, a feature the books
    ///      haven't declared) MUST happen before the first journal line.
    ///   2. The issued copy MUST be a `receipt.attached` on the invoice's
    ///      own entry, linked to the invoice, with the PDF's SHA-256.
    ///   3. The invoice is numbered: the number given, or the next from
    ///      invoicing.number_prefix and invoicing.first_number (A-2).
    /// ```
    pub fn create_invoice_issued(
        &mut self,
        customer_id: CustomerId,
        date: NaiveDate,
        due_date: NaiveDate,
        lines: Vec<DocumentLine>,
        fields: arap::InvoiceFields,
        pdf_bytes: Vec<u8>,
    ) -> Result<(InvoiceId, EntryId, u64), StoreError> {
        let fields = self.numbered(fields);
        self.check_invoice_create(customer_id, date, due_date, &lines, &fields)?;
        // The bytes hit the content-addressed store before anything is
        // journaled: an orphan blob from a later refusal is harmless
        // (unreferenced, deduplicated), a journal line naming a blob that
        // failed to store would not be.
        self.blobs.put(&pdf_bytes)?;
        let (invoice_id, entry_id, journaled) =
            self.journal_invoice(customer_id, date, due_date, &lines, &fields)?;
        let live =
            self.ledger
                .create_invoice_with(customer_id, date, due_date, lines, fields.clone())?;
        debug_assert_eq!(live, invoice_id, "id allocator diverged from journal");
        debug_assert_eq!(
            self.ledger
                .get_invoice(live)
                .expect("invoice exists")
                .entry_id,
            entry_id,
            "invoice entry diverged from journal"
        );
        let file_name = match &fields.number {
            Some(n) => format!("{n}.pdf"),
            None => format!("Invoice {live}.pdf"),
        };
        // The entry's and the invoice's lines apply first: they were
        // journaled before the attachment's, and the projection's
        // `last_applied_seq` follows the last line applied.
        for line in &journaled {
            apply_line_to_sqlite(&self.conn, line)?;
        }
        let (attachment_id, _, _) = self.attach_receipt(
            Some(entry_id),
            &file_name,
            "application/pdf",
            &pdf_bytes,
            Some("Issued invoice"),
        )?;
        self.link_attachment(attachment_id, Subject::Invoice(live))?;
        Ok((live, entry_id, attachment_id))
    }

    /// `fields` with a number when the books number their invoices: the one
    /// given, or the next from the books' invoicing settings (A-2,
    /// [`Ledger::next_invoice_number`]). Books that haven't declared
    /// `documents_v2` keep the legacy shape (migration item 5: "Invoice
    /// {id}", uncounted), so their issue files the copy and takes no number.
    fn numbered(&self, fields: arap::InvoiceFields) -> arap::InvoiceFields {
        if fields.number.is_some()
            || !self
                .journal_format()
                .has_feature(crate::format::DOCUMENTS_V2)
        {
            return fields;
        }
        let setting = |key: &str| self.ledger.settings().get(key).map(|r| r.value.clone());
        let prefix = setting("invoicing.number_prefix")
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_else(|| "INV-".to_string());
        let first = setting("invoicing.first_number")
            .and_then(|v| v.as_u64())
            .unwrap_or(1);
        arap::InvoiceFields {
            number: Some(self.ledger.next_invoice_number(&prefix, first)),
            ..fields
        }
    }

    /// What issuing an invoice would post and take, journaling nothing
    /// (plan P4b S4.3 item 12): the engine runs on a clone (the
    /// rehearsal pattern: the engine runs on a clone), so every refusal is
    /// the issue's own.
    ///
    /// ```text
    /// GOAL: the confirm is never a surprise: the posting, the total, the
    ///       number it would take and the due date are shown first.
    /// SOURCE: plan P4b S4.3 item 12; A-1 (an abandoned preview uses up no
    ///         number).
    /// INVARIANTS:
    ///   1. MUST journal nothing and change nothing, allocators included.
    ///   2. The posting MUST be the entry [`Self::create_invoice_issued`]
    ///      would post, and the number the one it would take.
    /// ```
    pub fn preview_invoice(
        &self,
        customer_id: CustomerId,
        date: NaiveDate,
        due_date: NaiveDate,
        lines: Vec<DocumentLine>,
        fields: arap::InvoiceFields,
    ) -> Result<InvoicePreview, StoreError> {
        let fields = self.numbered(fields);
        // The same checks the issue makes, so a preview refuses exactly what
        // the confirm will.
        self.ensure_document_lines(&lines)?;
        self.ensure_document_fields(&DocumentFields {
            invoice: Some(&fields),
            bill: None,
            lines: &lines,
        })?;
        self.approval_gate(
            |l| {
                l.create_invoice_with(customer_id, date, due_date, lines.clone(), fields.clone())?;
                Ok(())
            },
            false,
        )?;
        let mut probe = self.ledger.clone();
        let id = probe.create_invoice_with(customer_id, date, due_date, lines, fields.clone())?;
        let inv = probe.get_invoice(id).expect("the probe made the invoice");
        let entry = probe.get_entry(inv.entry_id).expect("its entry");
        Ok(InvoicePreview {
            posting: entry
                .lines()
                .iter()
                .map(|l| (l.account, l.side, l.amount.amount, l.class_id))
                .collect(),
            lines: preview_lines(&probe, entry.lines())?,
            total: inv.total()?,
            number: fields.number,
            due_date: inv.due_date,
            invoice: inv.clone(),
        })
    }

    /// Create a vendor bill. The auto-posted entry (Dr General Expenses, Cr
    /// A/P) is journaled first through the standard entry flow, then the
    /// bill itself as `bill.created`.
    ///
    /// [`Self::create_bill_with`] with no new fields: the bill every
    /// version before `documents_v2` wrote.
    pub fn create_bill(
        &mut self,
        vendor_id: VendorId,
        date: NaiveDate,
        due_date: NaiveDate,
        lines: Vec<DocumentLine>,
    ) -> Result<BillId, StoreError> {
        self.create_bill_with(
            vendor_id,
            date,
            due_date,
            lines,
            arap::BillFields::default(),
        )
    }

    /// Create a vendor bill with its new fields (the vendor's invoice
    /// number, receipt date, terms, memo, a duplicate's reason: plan P4b
    /// S4.3). [`Self::create_invoice_with`]'s INVARIANTS hold here too; a
    /// bill whose fields are all `None` is journaled exactly as before.
    pub fn create_bill_with(
        &mut self,
        vendor_id: VendorId,
        date: NaiveDate,
        due_date: NaiveDate,
        lines: Vec<DocumentLine>,
        fields: arap::BillFields,
    ) -> Result<BillId, StoreError> {
        self.check_bill_create(vendor_id, date, due_date, &lines, &fields)?;
        let (bill_id, entry_id, journaled) =
            self.journal_bill(vendor_id, date, due_date, &lines, &fields)?;
        let live = self
            .ledger
            .create_bill_with(vendor_id, date, due_date, lines, fields)?;
        debug_assert_eq!(live, bill_id, "id allocator diverged from journal");
        debug_assert_eq!(
            self.ledger.get_bill(live).expect("bill exists").entry_id,
            entry_id,
            "bill entry diverged from journal"
        );
        for line in &journaled {
            apply_line_to_sqlite(&self.conn, line)?;
        }
        Ok(live)
    }

    /// Everything [`Self::create_bill_with`] refuses, before anything is
    /// journaled; [`Self::check_invoice_create`]'s comments apply.
    fn check_bill_create(
        &self,
        vendor_id: VendorId,
        date: NaiveDate,
        due_date: NaiveDate,
        lines: &[DocumentLine],
        fields: &arap::BillFields,
    ) -> Result<(), StoreError> {
        self.rehearse(|l| {
            l.create_bill_with(vendor_id, date, due_date, lines.to_vec(), fields.clone())
        })?;
        self.approval_gate(
            |l| {
                l.create_bill_with(vendor_id, date, due_date, lines.to_vec(), fields.clone())?;
                Ok(())
            },
            false,
        )?;
        if self.ledger.get_vendor(vendor_id).is_none() {
            return Err(LedgerError::UnknownVendor(vendor_id).into());
        }
        arap::validate_lines(lines)?;
        arap::validate_due_date(date, due_date)?;
        if let Some(received_on) = fields.received_on {
            ledger_core::calendar::check(received_on)?;
        }
        if let Some(vendor_ref) = fields.vendor_ref.as_deref() {
            self.ledger.check_vendor_ref(
                vendor_id,
                vendor_ref,
                fields.duplicate_reason.as_deref(),
            )?;
        }
        if let Some(pid) = self.ledger.closed_period_containing(date) {
            return Err(LedgerError::ClosedPeriod(pid, date.to_string()).into());
        }
        self.ensure_document_lines(lines)?;
        self.ensure_document_fields(&DocumentFields {
            invoice: None,
            bill: Some(fields),
            lines,
        })?;
        Ok(())
    }

    /// Journal a bill's auto-posted entry and its `bill.created` event,
    /// without touching the live ledger; [`Self::check_bill_create`] has
    /// run.
    fn journal_bill(
        &mut self,
        vendor_id: VendorId,
        date: NaiveDate,
        due_date: NaiveDate,
        lines: &[DocumentLine],
        fields: &arap::BillFields,
    ) -> Result<(BillId, EntryId, Vec<JournalLine>), StoreError> {
        let expense = self.ensure_control_account(&ledger_core::GENERAL_EXPENSES)?;
        let ap = self.ensure_control_account(&ledger_core::PAYABLES)?;
        let bill_id = self.ledger.next_bill_id();
        let total: Decimal = lines.iter().map(|l| l.amount).sum();
        let description = format!("Bill {bill_id}");
        let mut posting: Vec<_> = arap::document_parts(lines, "the bill")?
            .into_iter()
            .map(|((account, class), amount)| {
                (account.unwrap_or(expense), Side::Debit, amount, class)
            })
            .collect();
        posting.push((ap, Side::Credit, total, None));
        let (entry_id, mut journaled) = self.journal_auto_lines(date, &description, &posting)?;
        let doc_lines: Vec<Value> = lines.iter().map(document_line_json).collect();
        let mut payload = json!({
            "id": bill_id,
            "vendor_id": vendor_id,
            "date": date.to_string(),
            "due_date": due_date.to_string(),
            "lines": doc_lines,
            "entry_id": entry_id,
        });
        bill_fields_json(&mut payload, fields);
        journaled.push(self.append_journal("bill.created", payload)?);
        Ok((bill_id, entry_id, journaled))
    }

    /// What recording a bill would post, journaling nothing (plan P4b S4.3
    /// item 12); [`Self::preview_invoice`]'s INVARIANTS hold here too.
    pub fn preview_bill(
        &self,
        vendor_id: VendorId,
        date: NaiveDate,
        due_date: NaiveDate,
        lines: Vec<DocumentLine>,
        fields: arap::BillFields,
    ) -> Result<BillPreview, StoreError> {
        self.ensure_document_lines(&lines)?;
        self.ensure_document_fields(&DocumentFields {
            invoice: None,
            bill: Some(&fields),
            lines: &lines,
        })?;
        self.approval_gate(
            |l| {
                l.create_bill_with(vendor_id, date, due_date, lines.clone(), fields.clone())?;
                Ok(())
            },
            false,
        )?;
        let mut probe = self.ledger.clone();
        let id = probe.create_bill_with(vendor_id, date, due_date, lines, fields)?;
        let bill = probe.get_bill(id).expect("the probe made the bill");
        let entry = probe.get_entry(bill.entry_id).expect("its entry");
        Ok(BillPreview {
            posting: entry
                .lines()
                .iter()
                .map(|l| (l.account, l.side, l.amount.amount, l.class_id))
                .collect(),
            lines: preview_lines(&probe, entry.lines())?,
            total: bill.total()?,
            due_date: bill.due_date,
        })
    }

    /// Record a customer payment against an invoice
    /// ([`Self::receive_payment_with`], with no payment method or
    /// reference).
    pub fn receive_payment(
        &mut self,
        invoice_id: InvoiceId,
        date: NaiveDate,
        amount: Decimal,
        cash_account: AccountId,
    ) -> Result<EntryId, StoreError> {
        self.receive_payment_with(
            invoice_id,
            date,
            amount,
            cash_account,
            &ledger_core::PaymentDetails::default(),
        )
    }

    /// Record a customer payment against an invoice, with its payment
    /// method and reference ([`Ledger::receive_payment_with`]). The
    /// payment entry (Dr the deposit account, Cr A/R) is journaled through
    /// the standard entry flow first (the reference in its description),
    /// then `invoice.payment_received`, then the payment method as
    /// `entry.instrument_set` (feature `payment_instruments`).
    ///
    /// ```text
    /// GOAL: a payment is journaled once and replays to the same figures,
    ///       in any version that can open the books.
    /// SOURCE: docs/spec/rules.md B-27, B-38; plan/served.md P4b.7
    ///         (document_standing).
    /// SCENARIOS:
    ///   1. IF the engine refuses it (overpaid, before the invoice, a
    ///      deposit account that isn't cash): SHOULD refuse, journaling
    ///      nothing.
    ///   2. IF it is accepted only because a payment on the invoice was
    ///      reversed (an older version, which counts every payment event,
    ///      would see it paid above its total): SHOULD need the journal
    ///      feature document_standing, refused (FormatUpgradePending) in
    ///      books that haven't declared it, journaling nothing.
    ///   3. IF it names a payment method: SHOULD need payment_instruments.
    /// ```
    pub fn receive_payment_with(
        &mut self,
        invoice_id: InvoiceId,
        date: NaiveDate,
        amount: Decimal,
        cash_account: AccountId,
        details: &ledger_core::PaymentDetails,
    ) -> Result<EntryId, StoreError> {
        self.rehearse(|l| l.receive_payment_with(invoice_id, date, amount, cash_account, details))?;
        self.approval_gate(
            |l| l.receive_payment_with(invoice_id, date, amount, cash_account, details),
            false,
        )?;
        // The rehearsal above is the whole check: hand-written mirrors of
        // the engine's refusals drift from it (and were never reached).
        self.ensure_payment_features(ledger_core::DocRef::Invoice(invoice_id), amount, details)?;
        let ar = self.ensure_control_account(&ledger_core::RECEIVABLES)?;
        let description = details.description(&format!("Payment on invoice {invoice_id}"));
        let (entry_id, mut journaled) =
            self.journal_auto_entry(date, &description, cash_account, ar, amount)?;
        journaled.push(self.append_journal(
            "invoice.payment_received",
            json!({
                "invoice_id": invoice_id,
                "date": date.to_string(),
                "amount": amount.to_string(),
                "cash_account": cash_account,
                "entry_id": entry_id,
            }),
        )?);
        if details.instrument.is_some() {
            journaled.push(self.append_journal(
                "entry.instrument_set",
                crate::form1099::entry_instrument_set(entry_id, details.instrument),
            )?);
        }
        let live =
            self.ledger
                .receive_payment_with(invoice_id, date, amount, cash_account, details)?;
        debug_assert_eq!(live, entry_id, "id allocator diverged from journal");
        for line in &journaled {
            apply_line_to_sqlite(&self.conn, line)?;
        }
        Ok(live)
    }

    /// Pay a vendor bill ([`Self::pay_bill_with`], with no payment method
    /// or reference).
    pub fn pay_bill(
        &mut self,
        bill_id: BillId,
        date: NaiveDate,
        amount: Decimal,
        cash_account: AccountId,
    ) -> Result<EntryId, StoreError> {
        self.pay_bill_with(
            bill_id,
            date,
            amount,
            cash_account,
            &ledger_core::PaymentDetails::default(),
        )
    }

    /// Pay a vendor bill, with its payment method and reference
    /// ([`Ledger::pay_bill_with`]). The payment entry (Dr A/P, Cr the
    /// payer account) is journaled through the standard entry flow first,
    /// then `bill.paid`, then the payment method as `entry.instrument_set`;
    /// as [`Self::receive_payment_with`] (its SCENARIOS too).
    pub fn pay_bill_with(
        &mut self,
        bill_id: BillId,
        date: NaiveDate,
        amount: Decimal,
        cash_account: AccountId,
        details: &ledger_core::PaymentDetails,
    ) -> Result<EntryId, StoreError> {
        self.rehearse(|l| l.pay_bill_with(bill_id, date, amount, cash_account, details))?;
        self.approval_gate(
            |l| l.pay_bill_with(bill_id, date, amount, cash_account, details),
            false,
        )?;
        self.ensure_payment_features(ledger_core::DocRef::Bill(bill_id), amount, details)?;
        let ap = self.ensure_control_account(&ledger_core::PAYABLES)?;
        let description = details.description(&format!("Payment on bill {bill_id}"));
        let (entry_id, mut journaled) =
            self.journal_auto_entry(date, &description, ap, cash_account, amount)?;
        journaled.push(self.append_journal(
            "bill.paid",
            json!({
                "bill_id": bill_id,
                "date": date.to_string(),
                "amount": amount.to_string(),
                "cash_account": cash_account,
                "entry_id": entry_id,
            }),
        )?);
        if details.instrument.is_some() {
            journaled.push(self.append_journal(
                "entry.instrument_set",
                crate::form1099::entry_instrument_set(entry_id, details.instrument),
            )?);
        }
        let live = self
            .ledger
            .pay_bill_with(bill_id, date, amount, cash_account, details)?;
        debug_assert_eq!(live, entry_id, "id allocator diverged from journal");
        for line in &journaled {
            apply_line_to_sqlite(&self.conn, line)?;
        }
        Ok(live)
    }

    /// The journal features a payment of `amount` on `doc` needs
    /// ([`Self::receive_payment_with`] SCENARIOS 2 and 3): document_standing
    /// when more than an older version thinks is still owed (the
    /// document's total less every payment event it recorded, reversed
    /// or not, its line tagged or not: `Ledger::payments_recorded`),
    /// payment_instruments when it names a payment method.
    fn ensure_payment_features(
        &self,
        doc: ledger_core::DocRef,
        amount: Decimal,
        details: &ledger_core::PaymentDetails,
    ) -> Result<(), StoreError> {
        let total = match doc {
            ledger_core::DocRef::Invoice(id) => self.ledger.get_invoice(id).map(|i| i.total()),
            ledger_core::DocRef::Bill(id) => self.ledger.get_bill(id).map(|b| b.total()),
        };
        if let Some(total) = total {
            let older_owed = total?.checked_sub(self.ledger.payments_recorded(doc)?);
            if older_owed.is_none_or(|owed| amount > owed) {
                self.ensure_feature(crate::format::DOCUMENT_STANDING)?;
            }
        }
        if details.instrument.is_some() {
            self.ensure_feature(crate::format::PAYMENT_INSTRUMENTS)?;
        }
        Ok(())
    }

    /// Void an invoice or bill: reverse its own entry, described "Void
    /// invoice 7: reason" ([`Ledger::void_document`]), journaled as the
    /// reversal it is (`entry.reversed`: a void writes no event of its
    /// own, plan P4b.7). Refused, journaling nothing, as
    /// [`Ledger::check_void_document`] and [`Self::check_reverse_entry`]
    /// refuse it (a payment standing on it, void already, a date before
    /// it). Returns the reversal's entry.
    pub fn void_document(
        &mut self,
        doc: ledger_core::DocRef,
        date: NaiveDate,
        reason: &str,
    ) -> Result<EntryId, StoreError> {
        let (target, description) = self.ledger.check_void_document(doc, date, reason)?;
        self.reverse_entry(target, date, &description)
    }

    // ------------------------------------------------------------------
    // Foreign exchange
    // ------------------------------------------------------------------

    /// Record a historical FX rate (foreign -> base) for reference.
    /// Journaled as `fx.rate_set`.
    pub fn set_fx_rate(
        &mut self,
        currency: Currency,
        date: NaiveDate,
        rate: Decimal,
    ) -> Result<(), StoreError> {
        let mut next = self.ledger.clone();
        next.set_fx_rate(currency, date, rate)?;
        let line = self.append_journal(
            "fx.rate_set",
            json!({
                "currency": currency.code(),
                "date": date.to_string(),
                "rate": rate.to_string(),
            }),
        )?;
        self.ledger = next;
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(())
    }

    /// Add a foreign-currency line to a draft entry. The base-currency
    /// amount is stored alongside the foreign original and the rate used.
    /// Journaled as `entry.line_added` with the fx fields
    /// (`foreign_currency`, `foreign_amount`, `fx_rate`).
    pub fn add_line_fx(
        &mut self,
        entry_id: EntryId,
        account: AccountId,
        side: Side,
        foreign_currency: Currency,
        foreign_amount: Decimal,
        fx_rate: Decimal,
    ) -> Result<(), StoreError> {
        let mut next = self.ledger.clone();
        next.add_line_fx(
            entry_id,
            account,
            side,
            foreign_currency,
            foreign_amount,
            fx_rate,
        )?;
        // Read back the stored base amount rather than recomputing it: the
        // journal must carry exactly what the ledger holds.
        let stored = next
            .get_entry(entry_id)
            .expect("entry exists after add_line_fx")
            .lines()
            .last()
            .expect("line was just added")
            .amount;
        let line = self.append_journal(
            "entry.line_added",
            json!({
                "id": entry_id,
                "account": account,
                "side": match side {
                    Side::Debit => "debit",
                    Side::Credit => "credit",
                },
                "amount": stored.amount.to_string(),
                "currency": stored.currency.code(),
                "foreign_currency": foreign_currency.code(),
                "foreign_amount": foreign_amount.to_string(),
                "fx_rate": fx_rate.to_string(),
            }),
        )?;
        self.ledger = next;
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Payroll
    // ------------------------------------------------------------------

    /// Ensure the seven payroll chart accounts (`6100`/`6110`/`2110`–`2150`)
    /// through the journaled account path. The ledger's internal ensure
    /// then no-ops, which keeps replay deterministic: every account
    /// creation is an `account.added` event.
    fn ensure_payroll_accounts(&mut self) -> Result<PayrollAccounts, StoreError> {
        let [salary, tax, federal, state, ss, medicare, other] = ledger_core::PAYROLL;
        Ok(PayrollAccounts {
            salary_expense: self.ensure_control_account(&salary)?,
            payroll_tax_expense: self.ensure_control_account(&tax)?,
            federal_payable: self.ensure_control_account(&federal)?,
            state_payable: self.ensure_control_account(&state)?,
            ss_payable: self.ensure_control_account(&ss)?,
            medicare_payable: self.ensure_control_account(&medicare)?,
            other_payable: self.ensure_control_account(&other)?,
        })
    }

    /// Run payroll: validate, journal the auto-posted entry (Dr Salary &
    /// Wage Expense for gross, Dr Payroll Tax Expense for the employer FICA
    /// share, Cr the `2110`–`2150` payables, Cr cash for net pay) through
    /// the standard entry flow, then journal `payroll.run` with the run's
    /// detail. Accounting only — no tax tables; every withholding number
    /// comes from the caller.
    pub fn run_payroll(
        &mut self,
        date: NaiveDate,
        lines: Vec<PayrollLineInput>,
        cash_account: AccountId,
    ) -> Result<PayrollRunId, StoreError> {
        self.rehearse(|l| l.run_payroll(date, lines.clone(), cash_account))?;
        self.approval_gate(|l| l.run_payroll(date, lines.clone(), cash_account), false)?;
        // Pure pre-validation mirroring Ledger::run_payroll, so the journal
        // writes below cannot be followed by a failed mutation.
        self.ledger.validate_payroll(date, &lines, cash_account)?;
        let accounts = self.ensure_payroll_accounts()?;
        let run_id = self.ledger.next_payroll_run_id();
        let entry_id = self.ledger.next_entry_id();
        let base = self.base_currency;
        let plan = payroll_entry_plan(&lines, accounts, cash_account, base)?;
        let description = format!("Payroll {date}");
        let mut journaled = Vec::with_capacity(plan.len() + 3);
        journaled.push(self.append_journal(
            "entry.created",
            json!({
                "id": entry_id,
                "date": date.to_string(),
                "description": description,
            }),
        )?);
        for (account, side, amount) in &plan {
            journaled.push(self.append_journal(
                "entry.line_added",
                json!({
                    "id": entry_id,
                    "account": account,
                    "side": match side {
                        Side::Debit => "debit",
                        Side::Credit => "credit",
                    },
                    "amount": amount.amount.to_string(),
                    "currency": amount.currency.code(),
                }),
            )?);
        }
        journaled.push(self.append_journal(
            "entry.posted",
            json!({
                "id": entry_id,
                "date": date.to_string(),
                "description": description,
                "reversal_of": Option::<u64>::None,
                "lines": plan.iter().map(|(a, s, m)| plain_line_json(*a, *s, *m)).collect::<Vec<_>>(),
            }),
        )?);
        journaled.push(self.append_journal(
            "payroll.run",
            json!({
                "id": run_id,
                "date": date.to_string(),
                "lines": lines.iter().map(payroll_line_json).collect::<Vec<_>>(),
                "entry_id": entry_id,
            }),
        )?);
        let live = self.ledger.run_payroll(date, lines, cash_account)?;
        debug_assert_eq!(live, run_id, "id allocator diverged from journal");
        debug_assert_eq!(
            self.ledger
                .get_payroll_run(live)
                .expect("payroll run exists")
                .entry_id,
            entry_id,
            "payroll entry diverged from journal"
        );
        for line in &journaled {
            apply_line_to_sqlite(&self.conn, line)?;
        }
        Ok(live)
    }

    /// Remit payroll taxes: journal the payoff entry (Dr each payable with
    /// a non-zero balance, Cr cash for the total) through the standard
    /// entry flow, then post it through the ledger. Fails before journaling
    /// anything when nothing is owed.
    pub fn remit_payroll_taxes(
        &mut self,
        date: NaiveDate,
        cash_account: AccountId,
    ) -> Result<EntryId, StoreError> {
        self.rehearse(|l| l.remit_payroll_taxes(date, cash_account))?;
        self.approval_gate(|l| l.remit_payroll_taxes(date, cash_account), false)?;
        // Pure pre-validation mirroring Ledger::remit_payroll_taxes.
        if self.ledger.chart().get(cash_account).is_none() {
            return Err(LedgerError::UnknownAccount(cash_account).into());
        }
        let payables = self.ledger.payroll_tax_payables();
        let total: Decimal = payables.iter().map(|(_, b)| *b).sum();
        if total.is_zero() {
            return Err(LedgerError::NothingToRemit.into());
        }
        if let Some(pid) = self.ledger.closed_period_containing(date) {
            return Err(LedgerError::ClosedPeriod(pid, date.to_string()).into());
        }
        let entry_id = self.ledger.next_entry_id();
        let base = self.base_currency;
        let plan = payroll_remit_plan(&payables, cash_account, base);
        let description = format!("Payroll tax remittance {date}");
        let mut journaled = Vec::with_capacity(plan.len() + 2);
        journaled.push(self.append_journal(
            "entry.created",
            json!({
                "id": entry_id,
                "date": date.to_string(),
                "description": description,
            }),
        )?);
        for (account, side, amount) in &plan {
            journaled.push(self.append_journal(
                "entry.line_added",
                json!({
                    "id": entry_id,
                    "account": account,
                    "side": match side {
                        Side::Debit => "debit",
                        Side::Credit => "credit",
                    },
                    "amount": amount.amount.to_string(),
                    "currency": amount.currency.code(),
                }),
            )?);
        }
        journaled.push(self.append_journal(
            "entry.posted",
            json!({
                "id": entry_id,
                "date": date.to_string(),
                "description": description,
                "reversal_of": Option::<u64>::None,
                "lines": plan.iter().map(|(a, s, m)| plain_line_json(*a, *s, *m)).collect::<Vec<_>>(),
            }),
        )?);
        // Name the entry as a remittance, as payroll.run names a run's
        // (readers like the viewer tell it from a hand-made entry by this).
        let total: Decimal = plan
            .iter()
            .filter(|(_, s, _)| *s == Side::Debit)
            .map(|(_, _, m)| m.amount)
            .sum();
        journaled.push(self.append_journal(
            "payroll.taxes_remitted",
            json!({"entry_id": entry_id, "date": date.to_string(), "total": total.to_string()}),
        )?);
        let live = self.ledger.remit_payroll_taxes(date, cash_account)?;
        debug_assert_eq!(live, entry_id, "id allocator diverged from journal");
        for line in &journaled {
            apply_line_to_sqlite(&self.conn, line)?;
        }
        Ok(live)
    }

    /// All payroll runs, in id order.
    pub fn list_payroll_runs(&self) -> Vec<PayrollRun> {
        self.ledger
            .list_payroll_runs()
            .into_iter()
            .cloned()
            .collect()
    }

    // ------------------------------------------------------------------
    // Classes
    // ------------------------------------------------------------------

    /// Add a class (division) for line-level P&L tracking: a project with
    /// only its name. Journaled as `class.added`.
    pub fn add_class(&mut self, name: &str) -> Result<ClassId, StoreError> {
        // Rehearsed on a copy: the name rules (blank, length, a name
        // another class has) refuse here, so the journal write can't be
        // followed by a failed mutation.
        self.rehearse(|l| l.add_class(name))?;
        self.append_class(name)
    }

    /// `class.added` for a name already rehearsed.
    fn append_class(&mut self, name: &str) -> Result<ClassId, StoreError> {
        let name = name.trim();
        let id = self.ledger.next_class_id();
        let line = self.append_journal("class.added", json!({ "id": id, "name": name }))?;
        let live = self.ledger.add_class(name)?;
        debug_assert_eq!(live, id, "id allocator diverged from journal");
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(live)
    }

    /// Add a project (rules.md B-37): `class.added`, then `project.set`
    /// when `facts` say more than the defaults. Returns its id.
    ///
    /// ```text
    /// GOAL: a project in one command, journaled as two events only when
    ///       it has facts, so a name-only project is a plain class.
    /// SOURCE: docs/spec/rules.md B-37 item 1; B-27 (journal feature
    ///         `projects`).
    /// INVARIANTS:
    ///   1. A refusal journals nothing: the whole record is rehearsed, and
    ///      the feature asked for, before the first line.
    ///   2. Facts equal to the defaults journal `class.added` alone (books
    ///      waiting for their format upgrade can still add one).
    /// ```
    pub fn add_project(
        &mut self,
        name: &str,
        facts: &ledger_core::ProjectFacts,
    ) -> Result<ClassId, StoreError> {
        self.rehearse(|l| {
            let id = l.add_class(name)?;
            if !facts.is_default() {
                l.set_project(id, name, facts.clone())?;
            }
            Ok(())
        })?;
        let with_facts = !facts.is_default();
        if with_facts {
            self.ensure_feature(crate::format::PROJECTS)?;
        }
        let id = self.append_class(name)?;
        if with_facts {
            let (name, facts) = self.ledger.check_set_project(id, name, facts)?;
            self.append_project(id, name, facts)?;
        }
        Ok(id)
    }

    /// Set a project's whole record (`project.set`): its name and facts.
    /// Unchanged (the same name and facts once checked): journals nothing
    /// and returns false.
    pub fn set_project(
        &mut self,
        id: ClassId,
        name: &str,
        facts: &ledger_core::ProjectFacts,
    ) -> Result<bool, StoreError> {
        let (name, facts) = self.ledger.check_set_project(id, name, facts)?;
        let same_name = self.ledger.get_class(id).is_some_and(|c| c.name == name);
        if same_name && self.ledger.project_facts(id).as_ref() == Some(&facts) {
            return Ok(false);
        }
        self.ensure_feature(crate::format::PROJECTS)?;
        self.append_project(id, name, facts)?;
        Ok(true)
    }

    /// `project.set` for a record already checked.
    fn append_project(
        &mut self,
        id: ClassId,
        name: String,
        facts: ledger_core::ProjectFacts,
    ) -> Result<(), StoreError> {
        let line =
            self.append_journal("project.set", crate::meta::project_set(id, &name, &facts))?;
        self.ledger.set_project(id, &name, facts)?;
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(())
    }

    /// A class's project facts (the defaults for a class with none);
    /// `None` for no such class.
    pub fn project_facts(&self, id: ClassId) -> Option<ledger_core::ProjectFacts> {
        self.ledger.project_facts(id)
    }

    /// All classes, in id order.
    pub fn list_classes(&self) -> Vec<Class> {
        self.ledger.list_classes().into_iter().cloned().collect()
    }

    /// Tag a draft entry's line with a class, or clear it with `None`.
    /// Journaled as `entry.line_class_set`. Draft-only: the ledger rejects
    /// posted entries, and this checks before journaling.
    pub fn set_line_class(
        &mut self,
        entry_id: EntryId,
        line_index: usize,
        class_id: Option<ClassId>,
    ) -> Result<(), StoreError> {
        self.rehearse(|l| l.set_line_class(entry_id, line_index, class_id))?;
        let line = self.append_journal(
            "entry.line_class_set",
            json!({
                "entry_id": entry_id,
                "line_index": line_index as u64,
                "class_id": class_id,
            }),
        )?;
        self.ledger.set_line_class(entry_id, line_index, class_id)?;
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Blobs and attachments
    // ------------------------------------------------------------------

    /// Attach a receipt to an entry (or file it unlinked with `entry_id=None`
    /// and link later). Returns `(attachment_id, sha256, deduplicated)`.
    ///
    /// ```text
    /// GOAL: keep a supporting document, by its exact bytes, with the entry
    ///       it supports.
    /// SOURCE: IRS Pub. 583 ("you must keep supporting documents ... they
    ///         support the entries in your books"); docs/receipt-store.md
    ///         "Blob store" (content-addressed, immutable, deduplicated).
    /// INVARIANTS:
    ///   1. The bytes MUST be stored under their SHA-256 and read back
    ///      identical, verified against it.
    ///   2. The same bytes filed twice MUST share one stored file, and the
    ///      second filing MUST say deduplicated.
    ///   3. Each filing MUST be its own attachment, journaled
    ///      (`receipt.attached`), with its name, type, and description.
    /// SCENARIOS:
    ///   1. IF `entry_id` names no entry: SHOULD refuse
    ///      - NEVER store an attachment pointing at nothing.
    ///   2. IF `entry_id` is None: SHOULD file it unlinked, to link later.
    /// ```
    pub fn attach_receipt(
        &mut self,
        entry_id: Option<EntryId>,
        file_name: &str,
        mime_type: &str,
        bytes: &[u8],
        description: Option<&str>,
    ) -> Result<(u64, String, bool), StoreError> {
        self.attach(AttachRequest {
            line_type: "receipt.attached",
            category: "receipts",
            entry_id,
            file_name,
            mime_type,
            bytes,
            description,
        })
    }

    /// File a document under a category (`formation`, `legal`, `tax`,
    /// `accounting`, `other`).
    pub fn file_document(
        &mut self,
        category: &str,
        file_name: &str,
        mime_type: &str,
        bytes: &[u8],
        description: Option<&str>,
    ) -> Result<(u64, String, bool), StoreError> {
        self.attach(AttachRequest {
            line_type: "document.filed",
            category,
            entry_id: None,
            file_name,
            mime_type,
            bytes,
            description,
        })
    }

    /// One filing of bytes into the blob store and journal.
    fn attach(&mut self, req: AttachRequest<'_>) -> Result<(u64, String, bool), StoreError> {
        if let Some(eid) = req.entry_id {
            self.ledger
                .get_entry(eid)
                .ok_or(LedgerError::UnknownEntry(eid))?;
        }
        // Bytes hit the content-addressed store first; an orphan blob from a
        // failed journal append is harmless (unreferenced, deduplicated).
        let (sha, dedup) = self.blobs.put(req.bytes)?;
        let id: u64 = self.conn.query_row(
            "SELECT COALESCE(MAX(id), 0) + 1 FROM attachments",
            [],
            |r| r.get(0),
        )?;
        let line = self.append_journal(
            req.line_type,
            json!({
                "attachment_id": id,
                "blob_sha": sha,
                "entry_id": req.entry_id,
                "category": req.category,
                "file_name": req.file_name,
                "mime_type": req.mime_type,
                "size_bytes": req.bytes.len() as u64,
                "description": req.description,
            }),
        )?;
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok((id, sha, dedup))
    }

    /// Register an amortization schedule. Journaled as `schedule.registered`,
    /// with its `convention` (`daily` or `monthly_even`). A daily schedule
    /// needs journal feature `schedule_convention` (refused, journaling
    /// nothing, in books that haven't declared it): an older version reads
    /// every schedule as equal months, and would refuse a daily one's
    /// postings as wrong (rather than as newer).
    pub fn register_schedule(
        &mut self,
        new: ledger_core::NewSchedule,
    ) -> Result<ledger_core::ScheduleId, StoreError> {
        self.rehearse(|l| l.register_schedule(new.clone()))?;
        ledger_core::schedules::validate_schedule(&new, self.ledger.chart())?;
        let id = self.ledger.next_schedule_id();
        if new.convention == ledger_core::ScheduleConvention::Daily {
            self.ensure_feature(crate::format::SCHEDULE_CONVENTION)?;
        }
        let line = self.append_journal(
            "schedule.registered",
            json!({
                "id": id,
                "description": new.description,
                "kind": serde_json::to_value(new.kind)?,
                "total": {"amount": new.total.amount.to_string(), "currency": new.total.currency.code()},
                "start": new.start.to_string(),
                "months": new.months,
                "balance_account": new.balance_account,
                "pl_account": new.pl_account,
                "convention": new.convention.key(),
            }),
        )?;
        let got = self.ledger.register_schedule(new)?;
        debug_assert_eq!(got, id);
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(id)
    }

    /// Post one month of a schedule. Journaled as `schedule.posted` with the
    /// whole entry, so a month is never half-posted or posted twice; replay
    /// re-derives the entry and checks it line by line. A month a reversal
    /// un-posted posts again only in books that declare journal feature
    /// `month_reposting` (refused, journaling nothing, otherwise): an older
    /// version would refuse it as posted twice.
    pub fn post_schedule_month(
        &mut self,
        id: ledger_core::ScheduleId,
        month_end: NaiveDate,
    ) -> Result<EntryId, StoreError> {
        // On a copy first: the posting can still be refused after the
        // preview (an amount out of range, found by the refusal property,
        // 2026-10-13), and a refusal must leave the live books untouched.
        let (date, _, _) = self.ledger.preview_schedule_month(id, month_end)?;
        let mut next = self.ledger.clone();
        let entry = next.post_schedule_month(id, month_end)?;
        if self.ledger.schedule_month_posted_before(id, date) {
            self.ensure_feature(crate::format::MONTH_REPOSTING)?;
        }
        let line = self.append_journal(
            "schedule.posted",
            json!({
                "schedule_id": id,
                "month": date.to_string(),
                "entry": entry_detail_json(&next, entry),
            }),
        )?;
        self.ledger = next;
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(entry)
    }

    /// Post every schedule's due months through `through`, oldest first.
    /// Returns (schedule, month, entry) for each. A month a run must leave
    /// for a human (dated in a closed period, which takes no new entries;
    /// or its posting reversed after the month ended, so posting it again
    /// would recognize it twice: Ledger::schedule_month_hold) is left
    /// unposted, as run_recurring leaves its occurrences, and
    /// [`Self::schedule_months_held`] lists it.
    pub fn run_schedules(
        &mut self,
        through: NaiveDate,
    ) -> Result<Vec<(ledger_core::ScheduleId, NaiveDate, EntryId)>, StoreError> {
        // A schedule whose plan can't be computed (one an earlier version
        // journaled, past chrono's last day) refuses the run, before any
        // month posts.
        let mut due: Vec<(NaiveDate, ledger_core::ScheduleId)> = Vec::new();
        for s in self.ledger.schedules() {
            due.extend(s.due(through)?.into_iter().map(|(d, _)| (d, s.id)));
        }
        due.sort();
        let mut out = Vec::with_capacity(due.len());
        for (month, id) in due {
            if self.ledger.schedule_month_hold(id, month).is_some() {
                continue;
            }
            out.push((id, month, self.post_schedule_month(id, month)?));
        }
        Ok(out)
    }

    /// Schedule months due through `through` that a run leaves for a human,
    /// as (schedule, month, why): dated in a closed period, or reversed
    /// after the month ended (Ledger::schedule_month_hold). A human books
    /// each in an open period by hand, or not at all.
    pub fn schedule_months_held(
        &self,
        through: NaiveDate,
    ) -> Result<Vec<(ledger_core::ScheduleId, NaiveDate, String)>, StoreError> {
        let mut out = Vec::new();
        for s in self.ledger.schedules() {
            for (d, _) in s.due(through)? {
                if let Some(why) = self.ledger.schedule_month_hold(s.id, d) {
                    out.push((s.id, d, why));
                }
            }
        }
        Ok(out)
    }

    /// Register a recurring entry. Journaled as `recurring.registered`.
    pub fn register_recurring(
        &mut self,
        new: ledger_core::NewRecurring,
    ) -> Result<ledger_core::RecurringId, StoreError> {
        self.rehearse(|l| l.register_recurring(new.clone()))?;
        ledger_core::schedules::validate_recurring(&new, self.ledger.chart())?;
        let id = self.ledger.next_recurring_id();
        let lines: Vec<Value> = new
            .lines
            .iter()
            .map(|(account, side, money)| {
                json!({
                    "account": account,
                    "side": if *side == Side::Debit { "debit" } else { "credit" },
                    "amount": money.amount.to_string(),
                    "currency": money.currency.code(),
                })
            })
            .collect();
        let line = self.append_journal(
            "recurring.registered",
            json!({
                "id": id,
                "description": new.description,
                "frequency": serde_json::to_value(new.frequency)?,
                "start": new.start.to_string(),
                "end": new.end.map(|e| e.to_string()),
                "lines": lines,
            }),
        )?;
        let got = self.ledger.register_recurring(new)?;
        debug_assert_eq!(got, id);
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(id)
    }

    /// Generate every recurring entry's due occurrences through `through`
    /// as drafts. Each journaled as `recurring.generated`. Returns
    /// (recurring entry, date, draft) for each. Occurrences dated in a
    /// closed period are left ungenerated (a closed period takes no new
    /// entries); [`Ledger::recurring_in_closed_periods`] lists them. At
    /// most [`MAX_RECURRING_RUN`] drafts in one call; more is refused whole.
    pub fn run_recurring(
        &mut self,
        through: NaiveDate,
    ) -> Result<Vec<(ledger_core::RecurringId, NaiveDate, EntryId)>, StoreError> {
        let mut due: Vec<(NaiveDate, ledger_core::RecurringId)> = self
            .ledger
            .recurring_entries()
            .flat_map(|r| r.due(through).into_iter().map(move |d| (d, r.id)))
            .collect();
        // One call journals at most MAX_RECURRING_RUN drafts: a far-off
        // `through` (a weekly entry through 9999) would otherwise write
        // hundreds of thousands of lines in one request (P0 review round
        // 3, S1).
        if due.len() > MAX_RECURRING_RUN {
            return Err(LedgerError::InvalidSchedule(format!(
                "running recurring entries through {through} would create {} drafts, more than {MAX_RECURRING_RUN} at once: run them through an earlier date first",
                due.len()
            ))
            .into());
        }
        due.sort();
        let mut out = Vec::with_capacity(due.len());
        for (date, id) in due {
            if self.ledger.closed_period_containing(date).is_some() {
                continue;
            }
            self.rehearse(|l| l.generate_recurring(id, date))?;
            let entry = self.ledger.next_entry_id();
            let line = self.append_journal(
                "recurring.generated",
                json!({"recurring_id": id, "date": date.to_string(), "entry_id": entry}),
            )?;
            let got = self.ledger.generate_recurring(id, date)?;
            debug_assert_eq!(got, entry);
            apply_line_to_sqlite(&self.conn, &line)?;
            out.push((id, date, entry));
        }
        Ok(out)
    }

    /// Give an account a new code (see `Ledger::renumber_account`).
    /// Journaled as `account.renumbered`, with the code it had.
    pub fn renumber_account(&mut self, account: AccountId, code: &str) -> Result<(), StoreError> {
        self.rehearse(|l| l.renumber_account(account, code))?;
        self.ledger.check_renumber_account(account, code)?;
        let previous = self
            .ledger
            .chart()
            .get(account)
            .map(|a| a.code.clone())
            .unwrap_or_default();
        let line = self.append_journal(
            "account.renumbered",
            json!({"account_id": account, "code": code, "previous_code": previous}),
        )?;
        self.ledger.renumber_account(account, code)?;
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(())
    }

    /// Give an account a new name (see `Ledger::rename_account`), with an
    /// optional reason. Journaled as `account.renamed`, keyed by the
    /// account's id (a code can change; an id can't), with its code, the
    /// new name, the name it had and the reason. It needs the journal
    /// feature `account_renamed` ([`crate::format::FEATURE_ACCOUNT_RENAMED`]),
    /// so an earlier version stops at the declaration.
    ///
    /// ```text
    /// GOAL: rename an account, keeping the name it had in the journal.
    /// SOURCE: rules.md B-31; Rev. Proc. 98-25 (docs/spec/sources/
    ///         irs-records.md): a report printed under the old name can
    ///         be tied to the new one.
    /// INVARIANTS:
    ///   1. Only the account's name changes (Ledger::rename_account
    ///      INVARIANT 1): every entry, line, hash and earlier journal line
    ///      stays as it was.
    ///   2. Rehearsed and checked first: a refusal journals nothing (and
    ///      books that haven't declared `account_renamed` refuse it,
    ///      FormatUpgradePending, until their format upgrade).
    /// ```
    pub fn rename_account(
        &mut self,
        account: AccountId,
        name: &str,
        reason: Option<&str>,
    ) -> Result<(), StoreError> {
        self.rehearse(|l| l.rename_account(account, name))?;
        self.ledger.check_rename_account(account, name)?;
        let reason = reason.map(Ledger::check_rename_reason).transpose()?;
        let a = self
            .ledger
            .chart()
            .get(account)
            .ok_or(LedgerError::UnknownAccount(account))?;
        let (code, previous) = (a.code.clone(), a.name.clone());
        let name = name.trim();
        self.ensure_feature(crate::format::FEATURE_ACCOUNT_RENAMED)?;
        let line = self.append_journal(
            "account.renamed",
            json!({
                "account_id": account,
                "code": code,
                "name": name,
                "previous": previous,
                "reason": reason,
            }),
        )?;
        self.ledger.rename_account(account, name)?;
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(())
    }

    /// Assign a control role an account, or (`None`) return it to its code
    /// (see `Ledger::assign_role`). Journaled as `role.assigned`, with the
    /// account that filled the role before.
    pub fn assign_role(
        &mut self,
        role: &str,
        account: Option<AccountId>,
    ) -> Result<(), StoreError> {
        let c = self.ledger.check_assign_role(role, account)?;
        let previous = self.ledger.role_account(c);
        let line = self.append_journal(
            "role.assigned",
            json!({"role": c.key, "account_id": account, "previous_account_id": previous}),
        )?;
        self.ledger.assign_role(role, account)?;
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(())
    }

    /// Set (or clear) an account's tax category and deductible percent.
    /// Journaled as `account.tax_set`. Under
    /// `policy.require_tax_category = refuse`, an account that needs a
    /// category is never left without one (rules.md B-32;
    /// `Ledger::check_tax_category_kept`).
    pub fn set_account_tax(
        &mut self,
        account: AccountId,
        category: Option<&str>,
        deductible_percent: Option<Decimal>,
    ) -> Result<(), StoreError> {
        self.rehearse(|l| l.set_account_tax(account, category, deductible_percent))?;
        self.ledger.check_tax_category_kept(account, category)?;
        self.journal_account_tax(account, category, deductible_percent)
    }

    /// Journal `account.tax_set` and apply it (the policy is the caller's).
    fn journal_account_tax(
        &mut self,
        account: AccountId,
        category: Option<&str>,
        deductible_percent: Option<Decimal>,
    ) -> Result<(), StoreError> {
        self.ledger
            .check_account_tax(account, category, deductible_percent)?;
        if let Some(f) = category.and_then(crate::format::tax_category_feature) {
            self.ensure_feature(f)?;
        }
        let line = self.append_journal(
            "account.tax_set",
            json!({
                "account_id": account,
                "tax_category": category,
                "deductible_percent": deductible_percent.map(|p| p.to_string()),
            }),
        )?;
        self.ledger
            .set_account_tax(account, category, deductible_percent)?;
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(())
    }

    /// Record a vendor's 1099 decision, from the date it takes effect (or
    /// from the beginning), with who decided; and its W-9 flag when given
    /// (`None` keeps it). Journaled as `vendor.1099_set` under the
    /// `vendor_1099_decisions` feature. Returns what it did to the
    /// vendor's decisions.
    ///
    /// ```text
    /// GOAL: the decision ("no 1099" included) is kept as a decision, with
    ///       who made it and when (plan defects 18, 23; D-40).
    /// SOURCE: ledger_core::form1099 (plan_decision, check_new_decision);
    ///         P3 review R8.
    /// SCENARIOS:
    ///   1. IF the decision has no source: SHOULD refuse
    ///      BECAUSE every new decision names who made it.
    ///   2. IF the core refuses it: SHOULD refuse, journaling nothing.
    ///   3. IF it changes no decision: SHOULD journal nothing but a
    ///      changed W-9 flag (Store::set_vendor_w9_flag).
    ///   4. Otherwise: SHOULD journal it, in books that declare the feature
    ///      (refused, FormatUpgradePending, journaling nothing, in books
    ///      that don't).
    /// ```
    pub fn set_vendor_1099(
        &mut self,
        vendor: VendorId,
        decision: &ledger_core::Dated1099,
        tin_on_file: Option<bool>,
    ) -> Result<ledger_core::form1099::Applied, StoreError> {
        if decision.source.is_none() {
            return Err(LedgerError::Invalid1099(
                "a 1099 decision names who made it (decided_by)".into(),
            )
            .into());
        }
        let applied = self.ledger.check_vendor_1099(vendor, decision)?;
        let current = self
            .ledger
            .get_vendor(vendor)
            .ok_or(LedgerError::UnknownVendor(vendor))?
            .tin_on_file;
        let flag = tin_on_file.unwrap_or(current);
        if applied == ledger_core::form1099::Applied::Unchanged {
            self.set_vendor_w9_flag(vendor, flag)?;
            return Ok(applied);
        }
        self.ensure_feature(crate::format::VENDOR_1099_DECISIONS)?;
        let line = self.append_journal(
            "vendor.1099_set",
            crate::form1099::decision_set(vendor, decision, flag),
        )?;
        let got = self
            .ledger
            .set_vendor_1099(vendor, decision.clone(), flag)?;
        debug_assert_eq!(got, applied, "check and record diverged");
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(applied)
    }

    /// Mark whether a vendor's W-9 is on file, keeping its 1099 decisions
    /// and who made them (P3 review R8). Journaled as
    /// `vendor.w9_flag_set` under the `vendor_1099_decisions` feature;
    /// nothing when the flag is already so. Returns whether it changed.
    pub fn set_vendor_w9_flag(
        &mut self,
        vendor: VendorId,
        on_file: bool,
    ) -> Result<bool, StoreError> {
        let v = self
            .ledger
            .get_vendor(vendor)
            .ok_or(LedgerError::UnknownVendor(vendor))?;
        if v.tin_on_file == on_file {
            return Ok(false);
        }
        self.ensure_feature(crate::format::VENDOR_1099_DECISIONS)?;
        let line = self.append_journal(
            "vendor.w9_flag_set",
            crate::form1099::w9_flag_set(vendor, on_file),
        )?;
        self.ledger.set_vendor_w9_flag(vendor, on_file)?;
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(true)
    }

    /// Add a payment method (ledger_core::instruments), journaled as
    /// `instrument.added` under the `payment_instruments` feature. Returns
    /// its id.
    pub fn add_instrument(
        &mut self,
        kind: ledger_core::InstrumentKind,
        label: &str,
        last4: Option<&str>,
        owner_party: Option<ledger_core::PartyId>,
        account: AccountId,
    ) -> Result<ledger_core::InstrumentId, StoreError> {
        let label = self
            .ledger
            .check_add_instrument(label, last4, owner_party, account)?;
        let id = self.ledger.next_instrument_id();
        let instrument = ledger_core::Instrument {
            id,
            kind,
            label,
            last4: last4.map(str::to_string),
            owner_party,
            account,
        };
        self.ensure_feature(crate::format::PAYMENT_INSTRUMENTS)?;
        let line = self.append_journal(
            "instrument.added",
            crate::form1099::instrument_added(&instrument),
        )?;
        let live =
            self.ledger
                .add_instrument(kind, &instrument.label, last4, owner_party, account)?;
        debug_assert_eq!(live, id, "id allocator diverged from journal");
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(live)
    }

    /// Tag an entry (and its reversal chain) with the payment method it
    /// was paid with, or clear it. Journaled as `entry.instrument_set`
    /// under the `payment_instruments` feature; nothing when every entry
    /// of the chain already says so. Returns whether it changed.
    pub fn set_entry_instrument(
        &mut self,
        entry: EntryId,
        instrument: Option<ledger_core::InstrumentId>,
    ) -> Result<bool, StoreError> {
        self.rehearse(|l| l.set_entry_instrument(entry, instrument))?;
        if self
            .ledger
            .reversal_chain(entry)
            .into_iter()
            .all(|e| self.ledger.entry_instrument(e) == instrument)
        {
            return Ok(false);
        }
        self.ensure_feature(crate::format::PAYMENT_INSTRUMENTS)?;
        let line = self.append_journal(
            "entry.instrument_set",
            crate::form1099::entry_instrument_set(entry, instrument),
        )?;
        self.ledger.set_entry_instrument(entry, instrument)?;
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(true)
    }

    /// Tag an entry with the vendor it paid, or clear the tag. Journaled as
    /// `entry.vendor_set`.
    pub fn set_entry_vendor(
        &mut self,
        entry: EntryId,
        vendor: Option<VendorId>,
    ) -> Result<(), StoreError> {
        self.rehearse(|l| l.set_entry_vendor(entry, vendor))?;
        // Validate first, so nothing invalid is journaled.
        self.ledger
            .get_entry(entry)
            .ok_or(LedgerError::UnknownEntry(entry))?;
        if let Some(v) = vendor {
            self.ledger
                .get_vendor(v)
                .ok_or(LedgerError::UnknownVendor(v))?;
        }
        let line = self.append_journal(
            "entry.vendor_set",
            json!({ "entry_id": entry, "vendor_id": vendor }),
        )?;
        self.ledger.set_entry_vendor(entry, vendor)?;
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(())
    }

    /// Link a previously filed (unlinked) receipt to an entry. A receipt
    /// already linked elsewhere must be unlinked first, so moving evidence
    /// off an entry always goes through [`Store::unlink_receipt`] and its
    /// recorded reason.
    ///
    /// ```text
    /// GOAL: attach filed evidence to the entry it supports.
    /// SOURCE: R-13a (receipt corrections are append-only events); IRS Pub.
    ///         583 (supporting documents "support the entries in your
    ///         books").
    /// SCENARIOS:
    ///   1. IF the attachment is unknown: SHOULD refuse.
    ///   2. IF it is withdrawn: SHOULD refuse (AttachmentWithdrawn).
    ///   3. IF it is linked (to any entry): SHOULD refuse (AttachmentLinked).
    ///   4. IF the entry is unknown: SHOULD refuse.
    ///   5. IF it is already linked to that entry (link_attachment): SHOULD
    ///      refuse (AlreadyLinked) BECAUSE filed on and linked to the same
    ///      entry is one receipt counted once, which neither unlink_receipt
    ///      nor unlink_attachment could then take off (link_attachment
    ///      SCENARIO 2, the other way round).
    ///   6. Otherwise: SHOULD journal `receipt.linked`.
    /// ```
    pub fn link_receipt(
        &mut self,
        attachment_id: u64,
        entry_id: EntryId,
    ) -> Result<(), StoreError> {
        self.ledger
            .get_entry(entry_id)
            .ok_or(LedgerError::UnknownEntry(entry_id))?;
        self.check_linkable(attachment_id)?;
        let subject = Subject::Entry(entry_id);
        if self.attachment_links(attachment_id)?.contains(&subject) {
            return Err(StoreError::AlreadyLinked(attachment_id, subject.describe()));
        }
        let line = self.append_journal(
            "receipt.linked",
            json!({ "attachment_id": attachment_id, "entry_id": entry_id }),
        )?;
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(())
    }

    /// Fail unless the attachment exists, is unlinked, and is not
    /// withdrawn: the precondition of [`Store::link_receipt`], checkable
    /// before posting anything that will link it.
    pub fn check_linkable(&self, attachment_id: u64) -> Result<(), StoreError> {
        let (entry_id, withdrawn) = self.attachment_state(attachment_id)?;
        if withdrawn {
            return Err(StoreError::AttachmentWithdrawn(attachment_id));
        }
        if let Some(eid) = entry_id {
            return Err(StoreError::AttachmentLinked(attachment_id, eid));
        }
        Ok(())
    }

    /// Detach a receipt from its entry, recording why. The attachment stays
    /// filed (unlinked) and can be linked again or withdrawn. The journal
    /// keeps both the link and the unlink: the evidence history is
    /// append-only even when the current evidence set changes. Returns the
    /// entry it was linked to.
    ///
    /// ```text
    /// GOAL: take evidence off an entry, on the record.
    /// SOURCE: R-13a.
    /// SCENARIOS:
    ///   1. IF the attachment is unknown, withdrawn, or not linked: SHOULD
    ///      refuse.
    ///   2. IF the reason is blank: SHOULD refuse
    ///      BECAUSE a correction without a why is not an audit trail.
    ///   3. Otherwise: SHOULD journal `receipt.unlinked` with the entry and
    ///      reason; the file stays stored and filed.
    /// ```
    ///
    /// Policy-free like the rest of the store: whether an entry may be left
    /// without a receipt is the MCP layer's call.
    pub fn unlink_receipt(
        &mut self,
        attachment_id: u64,
        reason: &str,
    ) -> Result<EntryId, StoreError> {
        let reason = reason.trim();
        if reason.is_empty() {
            return Err(StoreError::ReasonRequired);
        }
        let (entry_id, withdrawn) = self.attachment_state(attachment_id)?;
        if withdrawn {
            return Err(StoreError::AttachmentWithdrawn(attachment_id));
        }
        let entry_id = entry_id.ok_or(StoreError::AttachmentNotLinked(attachment_id))?;
        let line = self.append_journal(
            "receipt.unlinked",
            json!({ "attachment_id": attachment_id, "entry_id": entry_id, "reason": reason }),
        )?;
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(entry_id)
    }

    /// Withdraw a receipt or document, recording why: it is marked withdrawn
    /// (active listings omit it) and can never be linked again. Nothing is deleted: the bytes
    /// stay in the blob store and the journal keeps the filing and the
    /// withdrawal. A linked receipt must be unlinked first.
    ///
    /// ```text
    /// GOAL: retire a filed file, keeping it and the record of why.
    /// SOURCE: R-13a ("no blob is deleted"); IRS Pub. 583 (keep records
    ///         while they may be needed).
    /// SCENARIOS:
    ///   1. IF the attachment is unknown or already withdrawn: SHOULD refuse.
    ///   2. IF it is filed on an entry: SHOULD refuse (AttachmentLinked,
    ///      undone with unlink_receipt). IF it is linked to any subject
    ///      (link_attachment): SHOULD refuse (AlreadyLinked, naming the
    ///      first subject; undone with unlink_attachment).
    ///   3. IF the reason is blank: SHOULD refuse.
    ///   4. Otherwise: SHOULD journal `attachment.withdrawn`; the store's
    ///      listing marks it withdrawn with the reason (the MCP listing
    ///      hides it unless asked); its bytes still verify and read.
    /// ```
    pub fn withdraw_attachment(
        &mut self,
        attachment_id: u64,
        reason: &str,
    ) -> Result<(), StoreError> {
        let reason = reason.trim();
        if reason.is_empty() {
            return Err(StoreError::ReasonRequired);
        }
        self.check_linkable(attachment_id)?;
        if let Some(sub) = self.attachment_links(attachment_id)?.first() {
            return Err(StoreError::AlreadyLinked(attachment_id, sub.describe()));
        }
        let line = self.append_journal(
            "attachment.withdrawn",
            json!({ "attachment_id": attachment_id, "reason": reason }),
        )?;
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Attachment links, statements, reconciliations
    // ------------------------------------------------------------------

    /// The journal feature a link to `subject` needs, if any: a subject
    /// kind an older version doesn't know (a project, rules.md B-37).
    /// Callers filing a file with links ask it before filing, so a refusal
    /// leaves nothing half-filed.
    pub fn subject_feature(subject: Subject) -> Option<&'static str> {
        match subject {
            Subject::Project(_) => Some(crate::format::PROJECTS),
            _ => None,
        }
    }

    /// Whether `subject` exists in the books (what [`Store::link_attachment`]
    /// checks, so callers can validate several subjects before linking any).
    pub fn subject_exists(&self, subject: Subject) -> Result<bool, StoreError> {
        let l = &self.ledger;
        Ok(match subject {
            Subject::Entry(i) => l.get_entry(i).is_some(),
            Subject::Account(i) => l.chart().get(i).is_some(),
            Subject::Asset(i) => l.get_asset(i).is_some(),
            Subject::Party(i) => l.get_party(i).is_some(),
            Subject::Project(i) => u32::try_from(i).ok().and_then(|c| l.get_class(c)).is_some(),
            Subject::Invoice(i) => l.get_invoice(i).is_some(),
            Subject::Bill(i) => l.get_bill(i).is_some(),
            Subject::Period(i) => l.list_periods().iter().any(|p| p.id == i),
            Subject::PayrollRun(i) => l.get_payroll_run(i).is_some(),
            Subject::Schedule(i) => l.get_schedule(i).is_some(),
            Subject::Reconciliation(i) => self
                .conn
                .query_row(
                    "SELECT 1 FROM reconciliations WHERE id = ?",
                    params![i as i64],
                    |_| Ok(()),
                )
                .optional()?
                .is_some(),
            Subject::Company => true,
        })
    }

    /// The subjects an attachment is linked to (its filing entry, if any,
    /// first).
    pub fn attachment_links(&self, attachment_id: u64) -> Result<Vec<Subject>, StoreError> {
        let (entry, _) = self.attachment_state(attachment_id)?;
        let mut out: Vec<Subject> = entry.map(Subject::Entry).into_iter().collect();
        let mut stmt = self.conn.prepare(
            "SELECT subject_kind, subject_id FROM attachment_links WHERE attachment_id = ? ORDER BY subject_kind, subject_id",
        )?;
        let rows = stmt.query_map(params![attachment_id as i64], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })?;
        for row in rows {
            let (kind, id) = row?;
            if let Some(sub) = Subject::from_parts(&kind, id as u64) {
                out.push(sub);
            }
        }
        Ok(out)
    }

    /// The attachments linked to a subject (for an entry: filed on it or
    /// linked to it), not withdrawn.
    pub fn attachments_for(&self, subject: Subject) -> Result<Vec<Attachment>, StoreError> {
        if let Subject::Entry(e) = subject {
            return Ok(query_attachments(&self.conn, Some(e))?
                .into_iter()
                .filter(|a| a.withdrawn_at.is_none())
                .collect());
        }
        let ids: Vec<i64> = self
            .conn
            .prepare(
                "SELECT attachment_id FROM attachment_links WHERE subject_kind = ? AND subject_id = ? ORDER BY attachment_id",
            )?
            .query_map(params![subject.kind(), subject.id() as i64], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        Ok(query_attachments(&self.conn, None)?
            .into_iter()
            .filter(|a| ids.contains(&(a.id as i64)) && a.withdrawn_at.is_none())
            .collect())
    }

    /// Link a stored attachment to a subject: another entry, an account, an
    /// asset, a party, a project, an invoice, a bill, a period, a payroll
    /// run, a schedule, a reconciliation, or the company.
    ///
    /// ```text
    /// GOAL: one file, linked wherever it's evidence, without refiling.
    /// SOURCE: docs/parties-and-attachments.md "Attachments"; DECISION
    ///         (owner, 2026-10-15).
    /// INVARIANTS:
    ///   1. Journaled as `attachment.linked {attachment_id, subject}`.
    ///   2. An entry's receipts are the files filed on it and the files
    ///      linked to it; the receipt policy counts both.
    ///   3. A refusal journals nothing.
    /// SCENARIOS:
    ///   1. IF the attachment is unknown or withdrawn, or the subject
    ///      doesn't exist: SHOULD refuse.
    ///   2. IF it is already linked to that subject (or filed on that
    ///      entry): SHOULD refuse.
    /// ```
    pub fn link_attachment(
        &mut self,
        attachment_id: u64,
        subject: Subject,
    ) -> Result<(), StoreError> {
        let (entry, withdrawn) = self.attachment_state(attachment_id)?;
        if withdrawn {
            return Err(StoreError::AttachmentWithdrawn(attachment_id));
        }
        if !self.subject_exists(subject)? {
            return Err(StoreError::UnknownSubject(subject.describe()));
        }
        if entry.map(Subject::Entry) == Some(subject)
            || self.attachment_links(attachment_id)?.contains(&subject)
        {
            return Err(StoreError::AlreadyLinked(attachment_id, subject.describe()));
        }
        if let Some(f) = Self::subject_feature(subject) {
            self.ensure_feature(f)?;
        }
        let line = self.append_journal(
            "attachment.linked",
            json!({"attachment_id": attachment_id, "subject": subject}),
        )?;
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(())
    }

    /// Take a link off, with the reason. The file stays filed; a receipt
    /// filed on an entry comes off with `unlink_receipt` instead.
    ///
    /// ```text
    /// SCENARIOS:
    ///   1. IF the reason is blank, or the attachment isn't linked to the
    ///      subject (through this table): SHOULD refuse.
    ///   2. Otherwise: journaled as `attachment.unlinked {attachment_id,
    ///      subject, reason}`.
    /// ```
    pub fn unlink_attachment(
        &mut self,
        attachment_id: u64,
        subject: Subject,
        reason: &str,
    ) -> Result<(), StoreError> {
        let reason = reason.trim();
        if reason.is_empty() {
            return Err(StoreError::ReasonRequired);
        }
        self.attachment_state(attachment_id)?;
        let linked = self
            .conn
            .query_row(
                "SELECT 1 FROM attachment_links WHERE attachment_id = ? AND subject_kind = ? AND subject_id = ?",
                params![attachment_id as i64, subject.kind(), subject.id() as i64],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !linked {
            return Err(StoreError::NotLinkedTo(attachment_id, subject.describe()));
        }
        let line = self.append_journal(
            "attachment.unlinked",
            json!({"attachment_id": attachment_id, "subject": subject, "reason": reason}),
        )?;
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(())
    }

    /// Record what a filed statement says: its account, period, and
    /// balances. Links it to the account.
    ///
    /// ```text
    /// GOAL: the bank and card statements are in the books, so a
    ///       reconciliation names the one it agrees with.
    /// INVARIANTS:
    ///   1. Journaled as `statement.recorded {attachment_id, account_id,
    ///      period_start, period_end, opening_balance, closing_balance}`,
    ///      then `attachment.linked` to the account.
    /// SCENARIOS:
    ///   1. IF the attachment is unknown or withdrawn, already records a
    ///      statement (not voided), the account is unknown or not an asset
    ///      or liability (a bank, cash, or card account), the period runs
    ///      backwards, or a balance has more than two decimal places:
    ///      SHOULD refuse, journaling nothing.
    ///   2. IF its statement was voided (void_statement): records it
    ///      afresh, the correction of a mis-recorded statement.
    /// ```
    pub fn record_statement(&mut self, st: Statement) -> Result<(), StoreError> {
        let (_, withdrawn) = self.attachment_state(st.attachment_id)?;
        if withdrawn {
            return Err(StoreError::AttachmentWithdrawn(st.attachment_id));
        }
        let bad = |why: &str| Err(StoreError::InvalidStatement(why.to_string()));
        let acct = self
            .ledger
            .chart()
            .get(st.account_id)
            .ok_or(LedgerError::UnknownAccount(st.account_id))?;
        if !matches!(
            acct.account_type,
            AccountType::Asset | AccountType::Liability
        ) {
            return bad("a statement is for a bank, cash, or card account (an asset or liability)");
        }
        if st.period_end < st.period_start {
            return bad("the period ends before it starts");
        }
        if [st.opening_balance, st.closing_balance]
            .iter()
            .any(|b| b.normalize().scale() > 2)
        {
            return bad("balances are in cents at most");
        }
        // No larger than a line may move: a reconciliation subtracts them
        // (P0 review round 3, S1).
        if [st.opening_balance, st.closing_balance]
            .iter()
            .any(|b| b.abs() > ledger_core::kernel::MAX_LINE_AMOUNT)
        {
            return bad("balances are at most one quadrillion");
        }
        if !ledger_core::calendar::in_calendar(st.period_start)
            || !ledger_core::calendar::in_calendar(st.period_end)
        {
            return bad("the period is outside 0001-01-01 through 9999-12-31");
        }
        if self.statement(st.attachment_id)?.is_some() {
            return bad("that attachment already records a statement");
        }
        let line = self.append_journal(
            "statement.recorded",
            json!({
                "attachment_id": st.attachment_id,
                "account_id": st.account_id,
                "period_start": st.period_start.to_string(),
                "period_end": st.period_end.to_string(),
                "opening_balance": st.opening_balance.to_string(),
                "closing_balance": st.closing_balance.to_string(),
            }),
        )?;
        apply_line_to_sqlite(&self.conn, &line)?;
        if !self
            .attachment_links(st.attachment_id)?
            .contains(&Subject::Account(st.account_id))
        {
            self.link_attachment(st.attachment_id, Subject::Account(st.account_id))?;
        }
        Ok(())
    }

    /// Void a recorded statement, with the reason: what it says was
    /// recorded wrong (a mistyped balance or period), or it shouldn't have
    /// been recorded. Journaled as `statement.voided {attachment_id,
    /// reason}`.
    ///
    /// ```text
    /// GOAL: a mis-recorded statement can be corrected on the record,
    ///       nothing erased (the journal keeps what it said and why it was
    ///       voided).
    /// SOURCE: docs/parties-and-attachments.md "Reconciliations"; R-13a.
    /// INVARIANTS:
    ///   1. After it, the statement is no longer listed (statements,
    ///      statement), backs no reconciliation, and the same attachment
    ///      may record a statement again (record_statement).
    /// SCENARIOS:
    ///   1. IF the reason is blank: SHOULD refuse (ReasonRequired).
    ///   2. IF the attachment records no statement: SHOULD refuse.
    ///   3. IF a reconciliation not voided names it: SHOULD refuse
    ///      (ReconciliationRefused: void that reconciliation first).
    ///   4. Every refusal journals nothing.
    /// ```
    pub fn void_statement(&mut self, attachment_id: u64, reason: &str) -> Result<(), StoreError> {
        let reason = reason.trim();
        if reason.is_empty() {
            return Err(StoreError::ReasonRequired);
        }
        if self.statement(attachment_id)?.is_none() {
            return Err(StoreError::InvalidStatement(format!(
                "attachment {attachment_id} records no statement"
            )));
        }
        if let Some(r) = self
            .reconciliations(None)?
            .into_iter()
            .find(|r| r.voided.is_none() && r.attachment_id == attachment_id)
        {
            return Err(StoreError::ReconciliationRefused(format!(
                "statement {attachment_id} backs reconciliation {}: void that reconciliation first",
                r.id
            )));
        }
        let line = self.append_journal(
            "statement.voided",
            json!({"attachment_id": attachment_id, "reason": reason}),
        )?;
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(())
    }

    /// The statement an attachment records, if any (not voided, its file
    /// not withdrawn).
    pub fn statement(&self, attachment_id: u64) -> Result<Option<Statement>, StoreError> {
        Ok(self
            .statements(None)?
            .into_iter()
            .find(|s| s.attachment_id == attachment_id))
    }

    /// Recorded statements, optionally one account's, by period end: those
    /// not voided whose file isn't withdrawn (a withdrawn file retires its
    /// statement).
    pub fn statements(&self, account: Option<AccountId>) -> Result<Vec<Statement>, StoreError> {
        query_statements(&self.conn, account)
    }

    /// Reconcile an account to a recorded statement: the books' cleared
    /// balance through the statement date must equal its closing balance.
    ///
    /// ```text
    /// GOAL: a monthly close that leaves proof: which statement, which
    ///       balance, which lines it cleared.
    /// SOURCE: CONVENTION (bank reconciliation); docs/parties-and-
    ///         attachments.md "Reconciliations"; DECISION (owner,
    ///         2026-10-15).
    /// INVARIANTS:
    ///   1. Recorded only when the difference is exactly zero
    ///      (Ledger::reconciliation_report against the statement's date and
    ///      closing balance).
    ///   2. It records the cleared lines on the account dated on or before
    ///      the statement date, and links the statement to it.
    ///   3. Reconciliations of an account go forward: a new one's date is
    ///      after every one not voided.
    ///   4. Journaled as `reconciliation.completed`; never edited (void and
    ///      redo).
    /// SCENARIOS:
    ///   1. IF the attachment is withdrawn: SHOULD refuse
    ///      (AttachmentWithdrawn). IF it records no statement (or only a
    ///      voided one): SHOULD refuse.
    ///   2. IF the books differ from the statement: SHOULD refuse with
    ///      NotReconciled naming the difference.
    ///   3. IF a reconciliation not voided is dated on or after it: SHOULD
    ///      refuse.
    ///   4. Every refusal comes before anything is journaled.
    /// ```
    pub fn complete_reconciliation(
        &mut self,
        statement_attachment: u64,
    ) -> Result<Reconciliation, StoreError> {
        let (_, withdrawn) = self.attachment_state(statement_attachment)?;
        if withdrawn {
            return Err(StoreError::AttachmentWithdrawn(statement_attachment));
        }
        let st = self.statement(statement_attachment)?.ok_or_else(|| {
            StoreError::InvalidStatement(format!(
                "attachment {statement_attachment} records no statement (record_statement first)"
            ))
        })?;
        if let Some(last) = self
            .reconciliations(Some(st.account_id))?
            .into_iter()
            .filter(|r| r.voided.is_none())
            .map(|r| r.statement_date)
            .max()
            && last >= st.period_end
        {
            return Err(StoreError::ReconciliationRefused(format!(
                "account {} is already reconciled through {last}; void that reconciliation to redo it",
                st.account_id
            )));
        }
        let report =
            self.ledger
                .reconciliation_report(st.account_id, st.period_end, st.closing_balance)?;
        if !report.difference.is_zero() {
            return Err(StoreError::NotReconciled {
                difference: report.difference,
            });
        }
        let mut cleared = Vec::new();
        for e in self.ledger.posted_entries() {
            if e.date() > st.period_end {
                continue;
            }
            for (i, l) in e.lines().iter().enumerate() {
                if l.account == st.account_id && self.ledger.is_cleared(e.id(), i) {
                    cleared.push((e.id(), i));
                }
            }
        }
        cleared.sort_unstable();
        let id: u64 = self.conn.query_row(
            "SELECT COALESCE(MAX(id), 0) + 1 FROM reconciliations",
            [],
            |r| r.get::<_, i64>(0),
        )? as u64;
        let line = self.append_journal(
            "reconciliation.completed",
            json!({
                "id": id,
                "account_id": st.account_id,
                "statement_date": st.period_end.to_string(),
                "statement_balance": st.closing_balance.to_string(),
                "attachment_id": statement_attachment,
                "cleared": cleared,
            }),
        )?;
        apply_line_to_sqlite(&self.conn, &line)?;
        self.link_attachment(statement_attachment, Subject::Reconciliation(id))?;
        Ok(Reconciliation {
            id,
            account_id: st.account_id,
            statement_date: st.period_end,
            statement_balance: st.closing_balance,
            attachment_id: statement_attachment,
            cleared,
            voided: None,
        })
    }

    /// Void a reconciliation, with the reason (to redo it). Journaled as
    /// `reconciliation.voided`.
    pub fn void_reconciliation(&mut self, id: u64, reason: &str) -> Result<(), StoreError> {
        let reason = reason.trim();
        if reason.is_empty() {
            return Err(StoreError::ReasonRequired);
        }
        let r = self
            .reconciliations(None)?
            .into_iter()
            .find(|r| r.id == id)
            .ok_or_else(|| StoreError::UnknownSubject(format!("reconciliation {id}")))?;
        if r.voided.is_some() {
            return Err(StoreError::ReconciliationRefused(format!(
                "reconciliation {id} is already voided"
            )));
        }
        let line =
            self.append_journal("reconciliation.voided", json!({"id": id, "reason": reason}))?;
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(())
    }

    /// Reconciliations, optionally one account's, by date.
    pub fn reconciliations(
        &self,
        account: Option<AccountId>,
    ) -> Result<Vec<Reconciliation>, StoreError> {
        query_reconciliations(&self.conn, account)
    }

    /// `(linked entry, withdrawn?)` for an attachment.
    fn attachment_state(&self, attachment_id: u64) -> Result<(Option<EntryId>, bool), StoreError> {
        self.conn
            .query_row(
                "SELECT entry_id, withdrawn_at IS NOT NULL FROM attachments WHERE id = ?",
                params![attachment_id as i64],
                |r| {
                    Ok((
                        r.get::<_, Option<i64>>(0)?.map(|v| v as EntryId),
                        r.get::<_, bool>(1)?,
                    ))
                },
            )
            .optional()?
            .ok_or(StoreError::UnknownAttachment(attachment_id))
    }

    /// List attachments, optionally filtered to one entry.
    pub fn list_attachments(
        &self,
        entry_id: Option<EntryId>,
    ) -> Result<Vec<Attachment>, StoreError> {
        query_attachments(&self.conn, entry_id)
    }

    /// Read attachment bytes, verifying the SHA-256.
    pub fn attachment_bytes(&self, attachment_id: u64) -> Result<Vec<u8>, StoreError> {
        read_attachment_bytes(&self.conn, &self.blobs, attachment_id)
    }

    // ------------------------------------------------------------------
    // Queries
    // ------------------------------------------------------------------

    /// Search posted entries with optional filters. Newest first.
    pub fn search_entries(
        &self,
        account: Option<AccountId>,
        date_from: Option<&str>,
        date_to: Option<&str>,
        text: Option<&str>,
        limit: u64,
        offset: u64,
    ) -> Result<Vec<EntryRow>, StoreError> {
        let mut sql = String::from(
            "SELECT DISTINCT e.id, e.date, e.description, e.reversal_of FROM entries e",
        );
        if account.is_some() {
            sql.push_str(" JOIN lines l ON l.entry_id = e.id");
        }
        let mut conds: Vec<String> = Vec::new();
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
        if let Some(a) = account {
            conds.push("l.account_id = ?".to_string());
            args.push(Box::new(a as i64));
        }
        if let Some(d) = date_from {
            conds.push("e.date >= ?".to_string());
            args.push(Box::new(d.to_string()));
        }
        if let Some(d) = date_to {
            conds.push("e.date <= ?".to_string());
            args.push(Box::new(d.to_string()));
        }
        if let Some(t) = text {
            conds.push("e.description LIKE ?".to_string());
            args.push(Box::new(format!("%{t}%")));
        }
        if !conds.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&conds.join(" AND "));
        }
        sql.push_str(" ORDER BY e.date DESC, e.id DESC LIMIT ? OFFSET ?");
        args.push(Box::new(limit as i64));
        args.push(Box::new(offset as i64));
        let refs: Vec<&dyn rusqlite::ToSql> = args.iter().map(|b| b.as_ref()).collect();
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(refs.as_slice(), |r| {
            Ok(EntryRow {
                id: r.get::<_, i64>(0)? as u64,
                date: r.get(1)?,
                description: r.get(2)?,
                reversal_of: r.get::<_, Option<i64>>(3)?.map(|v| v as u64),
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Tail the event log from `seq` (exclusive).
    pub fn events_since(&self, seq: u64, limit: u64) -> Result<Vec<EventRow>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT seq, type, payload, created_at FROM events WHERE seq > ? ORDER BY seq LIMIT ?",
        )?;
        let rows = stmt.query_map(params![seq as i64, limit as i64], |r| {
            let payload_str: String = r.get(2)?;
            Ok(EventRow {
                seq: r.get::<_, i64>(0)? as u64,
                event_type: r.get(1)?,
                payload: serde_json::from_str(&payload_str).unwrap_or(Value::Null),
                created_at: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    // ------------------------------------------------------------------
    // Maintenance: verify and rebuild
    // ------------------------------------------------------------------

    /// Sequence number of the last journal line: the exact journal state a
    /// report reflects. Zero for an empty journal.
    pub fn journal_seq(&self) -> u64 {
        self.journal.next_seq() - 1
    }

    /// Verify everything: the journal hash chain, a fresh replay's reports
    /// against the live ledger, and the SQLite projection against the replay.
    /// Loud on any divergence.
    ///
    /// ```text
    /// GOAL: prove the books are what the journal says.
    /// SOURCE: docs/receipt-store.md "Verification"; Rev. Proc. 98-25 §5.01
    ///         (the relationship between the records and the books must be
    ///         demonstrable: an audit trail).
    /// INVARIANTS:
    ///   1. MUST change nothing.
    ///   2. MUST fail (VerifyMismatch, naming what differs) unless: the
    ///      chain verifies; a fresh replay equals the live ledger in every
    ///      part; the SQLite read-back's reports, asset register, classes
    ///      and payroll runs equal the replay's; and every projection table
    ///      equals a fresh projection of the journal, row for row.
    /// ```
    pub fn verify(&self) -> Result<(), StoreError> {
        let lines = read_and_verify_journal(self.journal.path())?;
        let mut replayed = Ledger::new(self.base_currency);
        for line in &lines {
            replay_line_into_ledger(&mut replayed, line, self.base_currency)?;
        }
        let a = reports::snapshot(&replayed).map_err(StoreError::Ledger)?;
        let b = reports::snapshot(&self.ledger).map_err(StoreError::Ledger)?;
        if a != b {
            return Err(StoreError::VerifyMismatch(
                "live ledger diverged from journal replay".into(),
            ));
        }
        // A projection that can't even be read back has drifted too.
        let c = self.load_ledger_from_sqlite().map_err(|e| {
            StoreError::VerifyMismatch(format!("sqlite projection can't be read back: {e}"))
        })?;
        let c_snap = reports::snapshot(&c).map_err(StoreError::Ledger)?;
        if a != c_snap {
            return Err(StoreError::VerifyMismatch(
                "sqlite projection diverged from journal".into(),
            ));
        }
        // The whole ledger, not only its balances: drafts, documents,
        // schedules, periods, settings. A replay bug in any of them shows
        // here even when every balance agrees. (The SQLite read-back below
        // is partial by design; the projection is checked table by table
        // at the end.)
        let parts = replayed.differs_from(&self.ledger);
        if !parts.is_empty() {
            return Err(StoreError::VerifyMismatch(format!(
                "live ledger diverged from journal replay in: {}",
                parts.join(", ")
            )));
        }
        // The live ledger equals the replay (checked above, in full); the
        // SQLite read-back's asset register, classes, and payroll runs
        // must equal it too (the accounting snapshot ignores them).
        let assets_of = |l: &Ledger| {
            l.registered_assets()
                .map(|a| (a.clone(), l.depreciation_runs(a.id).to_vec()))
                .collect::<Vec<_>>()
        };
        if assets_of(&replayed) != assets_of(&c) {
            return Err(StoreError::VerifyMismatch(
                "asset register diverged between journal, live ledger, and sqlite projection"
                    .into(),
            ));
        }
        let classes_of = |l: &Ledger| l.list_classes().into_iter().cloned().collect::<Vec<_>>();
        if classes_of(&replayed) != classes_of(&c) {
            return Err(StoreError::VerifyMismatch(
                "class list diverged between journal, live ledger, and sqlite projection".into(),
            ));
        }
        let runs_of = |l: &Ledger| {
            l.list_payroll_runs()
                .into_iter()
                .cloned()
                .collect::<Vec<_>>()
        };
        if runs_of(&replayed) != runs_of(&c) {
            return Err(StoreError::VerifyMismatch(
                "payroll runs diverged between journal, live ledger, and sqlite projection".into(),
            ));
        }
        // Finally, the whole projection, not just what reports read: every
        // table, row for row, against a fresh projection of the journal.
        compare_projections(
            &projection_rows(&self.conn)?,
            &projection_rows(&fresh_projection(&lines)?)?,
        )
    }

    /// [`Self::verify`], and if the SQLite projection is what diverged,
    /// heal it: keep the divergent file for forensics, rebuild from the
    /// journal, and verify again. A journal that fails its own chain is
    /// never "healed": that's an error for a human.
    ///
    /// ```text
    /// GOAL: books that verify, healing only what is derived.
    /// SOURCE: docs/receipt-store.md ("the SQLite database is disposable";
    ///         "don't investigate — rebuild").
    /// SCENARIOS:
    ///   1. IF verify passes: SHOULD report the event count and head hash,
    ///      healed: none.
    ///   2. IF the projection or ledger diverged (VerifyMismatch): SHOULD copy
    ///      the database aside, rebuild from the journal, verify again, and
    ///      report why it healed.
    ///   3. IF the journal itself fails: SHOULD refuse
    ///      - NEVER touch the journal.
    /// ```
    pub fn verify_and_heal(&mut self) -> Result<VerifyReport, StoreError> {
        let healed = match self.verify() {
            Ok(()) => None,
            Err(StoreError::VerifyMismatch(why)) => {
                let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
                let keep = self.db_path.with_extension(format!("mismatch.{stamp}"));
                std::fs::copy(&self.db_path, &keep)?;
                eprintln!(
                    "wanderbooks: {why}; kept the old database as {} beside it and rebuilt from the journal",
                    keep.file_name().unwrap_or_default().to_string_lossy()
                );
                self.rebuild()?;
                self.verify()?;
                Some(why)
            }
            Err(e) => return Err(e),
        };
        let lines = read_and_verify_journal(self.journal.path())?;
        Ok(VerifyReport {
            events: lines.len() as u64,
            head: lines
                .last()
                .map(|l| sha256_hex(l.to_canonical().as_bytes()))
                .unwrap_or_else(|| GENESIS_PREV.to_string()),
            healed,
            database: self.database_file(),
            other_databases: self.other_databases(),
        })
    }

    /// The SHA-256 of every stored file the books name (receipts and
    /// documents, withdrawn ones included).
    pub fn stored_blob_shas(&self) -> Result<Vec<String>, StoreError> {
        let mut stmt = self.conn.prepare("SELECT sha256 FROM blobs")?;
        let shas = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(shas.into_iter().map(|s| s.to_lowercase()).collect())
    }

    /// Verify every blob's bytes against its SHA-256. Returns the count
    /// checked. Reads all bytes; run on a schedule, not per request.
    pub fn verify_blobs(&self) -> Result<u64, StoreError> {
        let mut stmt = self.conn.prepare("SELECT sha256 FROM blobs")?;
        let shas: Vec<String> = stmt
            .query_map([], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        let mut n = 0u64;
        for sha in shas {
            self.blobs.get(&sha)?;
            n += 1;
        }
        Ok(n)
    }

    /// Drop the SQLite projection and rebuild it from the journal, then
    /// re-hydrate the live ledger. The answer to "what if SQLite corrupts":
    /// don't investigate — rebuild.
    pub fn rebuild(&mut self) -> Result<(), StoreError> {
        let old = std::mem::replace(&mut self.conn, Connection::open_in_memory()?);
        old.close().map_err(|(_, e)| StoreError::Rusqlite(e))?;
        std::fs::remove_file(&self.db_path)?;
        // WAL and SHM are derived state too; a stale WAL must not leak into
        // the rebuilt database.
        let _ = std::fs::remove_file(self.db_path.with_extension("db-wal"));
        let _ = std::fs::remove_file(self.db_path.with_extension("db-shm"));
        self.conn = Self::open_conn(&self.db_path)?;
        Self::migrate(&self.conn)?;
        Self::write_meta(&self.conn, "base_currency", self.base_currency.code())?;
        Self::write_meta(&self.conn, "last_applied_seq", "0")?;
        let lines = read_and_verify_journal(self.journal.path())?;
        for line in &lines {
            apply_line_to_sqlite(&self.conn, line)?;
        }
        self.ledger = Ledger::new(self.base_currency);
        for line in &lines {
            replay_line_into_ledger(&mut self.ledger, line, self.base_currency)?;
        }
        Ok(())
    }

    /// Rebuild a ledger purely from the SQLite projection (used by verify).
    /// Rebuild the ledger from the SQLite projection, for the cross-check.
    /// Like journal replay, it reads history, so it uses the engine's
    /// `replay_*` commands: rules added later never refuse books written
    /// before them.
    fn load_ledger_from_sqlite(&self) -> Result<Ledger, StoreError> {
        let mut ledger = Ledger::new(self.base_currency);
        let mut stmt = self.conn.prepare(
            "SELECT id, code, name, type, parent, placeholder, currency FROM accounts ORDER BY id",
        )?;
        let accounts = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<i64>>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, String>(6)?,
            ))
        })?;
        for a in accounts {
            let (id, code, name, typ, parent, placeholder, currency) = a?;
            let account_type: AccountType = serde_json::from_value(Value::String(typ))?;
            let cur = Currency::new(&currency)
                .map_err(|_| StoreError::JournalCorrupt(format!("bad currency {currency}")))?;
            if cur != self.base_currency {
                return Err(StoreError::JournalCorrupt(format!(
                    "account {code} currency {currency} != base"
                )));
            }
            let got = ledger.replay_add_account(
                &code,
                &name,
                account_type,
                parent.map(|p| p as u64),
                placeholder != 0,
            )?;
            if got != id as u64 {
                return Err(StoreError::ReplayMismatch(format!(
                    "account id: sqlite says {id}, replay got {got}"
                )));
            }
        }
        // Classes: master data in id order, before entries (lines may
        // reference them).
        let mut class_stmt = self
            .conn
            .prepare("SELECT id, name FROM classes ORDER BY id")?;
        let class_rows =
            class_stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
        for c in class_rows {
            let (id, name) = c?;
            ledger.replay_class(id as u32, name)?;
        }
        let mut estmt = self
            .conn
            .prepare("SELECT id, date, description, reversal_of FROM entries ORDER BY id")?;
        let entries = estmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<i64>>(3)?,
            ))
        })?;
        for e in entries {
            let (id, date, description, reversal_of) = e?;
            let date = chrono::NaiveDate::parse_from_str(&date, "%Y-%m-%d")
                .map_err(|_| StoreError::JournalCorrupt(format!("bad date {date}")))?;
            // Drafts aren't projected until posted, so ids can skip: a draft
            // #8 before a posted #9. Hold their ids with empty drafts, which
            // no report counts.
            while ledger.next_entry_id() < id as u64 {
                ledger.create_entry(date, "(draft, not projected)");
            }
            match reversal_of {
                Some(orig) => {
                    // Reversals allocate the next id; replayed in id order the
                    // allocator is in sync, so the id must match the row.
                    let got = ledger.replay_reverse_entry(orig as u64, date, &description)?;
                    if got != id as u64 {
                        return Err(StoreError::ReplayMismatch(format!(
                            "reversal id: sqlite says {id}, replay got {got}"
                        )));
                    }
                }
                None => {
                    let got = ledger.create_entry(date, &description);
                    if got != id as u64 {
                        return Err(StoreError::ReplayMismatch(format!(
                            "entry id: sqlite says {id}, replay got {got}"
                        )));
                    }
                    let mut lstmt = self.conn.prepare(
                        "SELECT account_id, side, amount, currency, class_id FROM lines WHERE entry_id = ? ORDER BY id",
                    )?;
                    let lrows = lstmt.query_map(params![id], |r| {
                        Ok((
                            r.get::<_, i64>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, String>(2)?,
                            r.get::<_, String>(3)?,
                            r.get::<_, Option<i64>>(4)?,
                        ))
                    })?;
                    for (idx, lr) in lrows.enumerate() {
                        let (account, side, amount, currency, class_id) = lr?;
                        ledger.replay_add_line(
                            got,
                            account as u64,
                            parse_side(&side)?,
                            Money::new(
                                amount.parse().map_err(|_| {
                                    StoreError::JournalCorrupt(format!("bad amount {amount}"))
                                })?,
                                Currency::new(&currency).map_err(|_| {
                                    StoreError::JournalCorrupt(format!("bad currency {currency}"))
                                })?,
                            ),
                        )?;
                        // Draft-only tagging, before the post below.
                        if let Some(cid) = class_id {
                            ledger.set_line_class(got, idx, Some(cid as u32))?;
                        }
                    }
                    ledger.replay_post_entry(got)?;
                }
            }
        }
        // Assets: registration order is id order; disposal is a per-row flag.
        let mut astmt = self.conn.prepare(
            "SELECT id, description, purchase_date, cost_amount, cost_currency, salvage_amount, salvage_currency, useful_life_months, method, cost_account, accum_depr_account, expense_account, status, disposed_date, proceeds_amount FROM assets ORDER BY id",
        )?;
        let assets = astmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, i64>(7)?,
                r.get::<_, String>(8)?,
                r.get::<_, i64>(9)?,
                r.get::<_, i64>(10)?,
                r.get::<_, i64>(11)?,
                r.get::<_, String>(12)?,
                r.get::<_, Option<String>>(13)?,
                r.get::<_, Option<String>>(14)?,
            ))
        })?;
        for a in assets {
            let (
                id,
                description,
                purchase_date,
                cost_amount,
                cost_currency,
                salvage_amount,
                salvage_currency,
                life_months,
                method,
                cost_account,
                accum_depr_account,
                expense_account,
                status,
                disposed_date,
                proceeds_amount,
            ) = a?;
            if method != "straight_line" {
                return Err(StoreError::JournalCorrupt(format!(
                    "unknown method {method}"
                )));
            }
            let parse_amt = |s: &str, c: &str| -> Result<Money, StoreError> {
                Ok(Money::new(
                    s.parse()
                        .map_err(|_| StoreError::JournalCorrupt(format!("bad amount {s}")))?,
                    Currency::new(c)
                        .map_err(|_| StoreError::JournalCorrupt(format!("bad currency {c}")))?,
                ))
            };
            let got = ledger.replay_register_asset(NewAsset {
                description,
                in_service_date: parse_date(&purchase_date)?,
                cost: parse_amt(&cost_amount, &cost_currency)?,
                salvage_value: parse_amt(&salvage_amount, &salvage_currency)?,
                useful_life_months: life_months as u32,
                cost_account: cost_account as u64,
                accum_depr_account: accum_depr_account as u64,
                expense_account: expense_account as u64,
            })?;
            if got != id as u64 {
                return Err(StoreError::ReplayMismatch(format!(
                    "asset id: sqlite says {id}, replay got {got}"
                )));
            }
            if status == "disposed" {
                let date = disposed_date
                    .as_deref()
                    .ok_or_else(|| StoreError::JournalCorrupt("disposed without date".into()))?;
                let proceeds = proceeds_amount.as_deref().ok_or_else(|| {
                    StoreError::JournalCorrupt("disposed without proceeds".into())
                })?;
                ledger.replay_dispose_asset(
                    got,
                    parse_date(date)?,
                    parse_amt(proceeds, &cost_currency)?,
                )?;
            }
        }
        // Tags in the order they were given (a retag keeps the old tag
        // reserved), then serial numbers.
        let mut tstmt = self
            .conn
            .prepare("SELECT asset_id, tag FROM asset_tags_log ORDER BY seq")?;
        let tags = tstmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
        for t in tags {
            let (id, tag) = t?;
            ledger.set_asset_tag(id as u64, &tag)?;
        }
        let mut sstmt = self
            .conn
            .prepare("SELECT id, serial FROM assets WHERE serial IS NOT NULL ORDER BY id")?;
        let serials = sstmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
        for row in serials {
            let (id, serial) = row?;
            ledger.set_asset_serial(id as u64, Some(&serial))?;
        }
        // Basis records: a pre-P3 one (no value at contribution) as its
        // event replays, a newer one with its facts.
        let mut xstmt = self.conn.prepare(
            "SELECT id, tax_rule, contributor_basis, fmv_at_contribution, sec704c_method, basis_source_json FROM assets WHERE tax_rule IS NOT NULL ORDER BY id",
        )?;
        let taxes = xstmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, Option<String>>(5)?,
            ))
        })?;
        for row in taxes {
            let (id, rule, basis, fmv, method, source) = row?;
            let corrupt = |what: &str| StoreError::JournalCorrupt(format!("assets.{what}"));
            let dec = |v: Option<String>, what: &str| {
                v.map(|s| s.parse::<Decimal>().map_err(|_| corrupt(what)))
                    .transpose()
            };
            let fmv = dec(fmv, "fmv_at_contribution")?;
            if fmv.is_none()
                && method.is_none()
                && source.is_none()
                && let Some(legacy) = ledger_core::BasisRule::parse_legacy(&rule)
            {
                ledger.replay_set_asset_tax_basis(
                    id as u64,
                    legacy,
                    dec(basis, "contributor_basis")?,
                )?;
                continue;
            }
            let basis = ledger_core::AssetBasis {
                rule: ledger_core::BasisRule::parse(&rule).ok_or_else(|| corrupt("tax_rule"))?,
                contributor_adjusted_basis: dec(basis, "contributor_basis")?,
                fmv_at_contribution: fmv,
                sec704c_method: method
                    .map(|m| {
                        ledger_core::Sec704cMethod::parse(&m)
                            .ok_or_else(|| corrupt("sec704c_method"))
                    })
                    .transpose()?,
                source: source
                    .map(|j| serde_json::from_str(&j).map_err(|_| corrupt("basis_source_json")))
                    .transpose()?,
            };
            ledger.replay_set_asset_basis(id as u64, basis)?;
        }
        let mut qstmt = self.conn.prepare(
            "SELECT id, acquisition_entry FROM assets WHERE acquisition_entry IS NOT NULL ORDER BY id",
        )?;
        let links = qstmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?;
        for row in links {
            let (id, entry) = row?;
            ledger.restore_asset_acquisition(id as u64, entry as u64)?;
        }
        // Depreciation runs, each asset's in the order they were posted
        // (entry order, as replay records them).
        let mut dstmt = self.conn.prepare(
            "SELECT asset_id, period, entry_id, amount FROM asset_depreciation ORDER BY asset_id, entry_id",
        )?;
        let runs = dstmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?;
        for row in runs {
            let (asset, period, entry, amount) = row?;
            let amount = amount.parse::<Decimal>().map_err(|_| {
                StoreError::JournalCorrupt(format!("asset_depreciation amount {amount}"))
            })?;
            ledger.record_depreciation_run(asset as u64, &period, entry as u64, amount)?;
        }
        // Payroll runs: id order; lines in insertion order. Amounts are
        // TEXT in SQLite, so they parse here with exact Decimal math.
        let parse_payroll_amount = |s: String| {
            s.parse()
                .map_err(|_| StoreError::JournalCorrupt(format!("bad payroll amount {s}")))
        };
        let mut run_stmt = self
            .conn
            .prepare("SELECT id, date, entry_id FROM payroll_runs ORDER BY id")?;
        let run_rows = run_stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?;
        for r in run_rows {
            let (id, date, entry_id) = r?;
            let mut pl_stmt = self.conn.prepare(
                "SELECT employee_name, gross_pay, federal_withholding, state_withholding, ss_employee, medicare_employee, ss_employer, medicare_employer, other_deductions, employee_id FROM payroll_lines WHERE run_id = ? ORDER BY id",
            )?;
            let pl_rows = pl_stmt.query_map(params![id], |lr| {
                Ok((
                    lr.get::<_, String>(0)?,
                    lr.get::<_, String>(1)?,
                    lr.get::<_, String>(2)?,
                    lr.get::<_, String>(3)?,
                    lr.get::<_, String>(4)?,
                    lr.get::<_, String>(5)?,
                    lr.get::<_, String>(6)?,
                    lr.get::<_, String>(7)?,
                    lr.get::<_, String>(8)?,
                    lr.get::<_, Option<i64>>(9)?,
                ))
            })?;
            let mut lines = Vec::new();
            for lr in pl_rows {
                let (name, gross, fed, state, ss_e, med_e, ss_r, med_r, other, emp) = lr?;
                lines.push(PayrollLine {
                    employee_name: name,
                    employee: emp.map(|e| e as u64),
                    gross_pay: parse_payroll_amount(gross)?,
                    federal_withholding: parse_payroll_amount(fed)?,
                    state_withholding: parse_payroll_amount(state)?,
                    ss_employee: parse_payroll_amount(ss_e)?,
                    medicare_employee: parse_payroll_amount(med_e)?,
                    ss_employer: parse_payroll_amount(ss_r)?,
                    medicare_employer: parse_payroll_amount(med_r)?,
                    other_deductions: parse_payroll_amount(other)?,
                });
            }
            ledger.replay_payroll_run(id as u64, parse_date(&date)?, lines, entry_id as u64)?;
        }
        Ok(ledger)
    }
}

// ---------------------------------------------------------------------------
// Replay and apply helpers
// ---------------------------------------------------------------------------

fn journal_line_json(l: &ledger_core::JournalLine) -> Value {
    let side = match l.side {
        Side::Debit => "debit",
        Side::Credit => "credit",
    };
    let mut v = json!({
        "account": l.account,
        "side": side,
        "amount": l.amount.amount.to_string(),
        "currency": l.amount.currency.code(),
    });
    // Foreign-currency provenance, when the line carries it. Journals
    // written before foreign-currency lines existed have no such lines;
    // readers treat missing fields as None.
    let obj = v
        .as_object_mut()
        .expect("journal_line_json builds an object");
    if let Some(fc) = l.foreign_currency {
        obj.insert(
            "foreign_currency".to_string(),
            Value::String(fc.code().to_string()),
        );
    }
    if let Some(fa) = l.foreign_amount {
        obj.insert("foreign_amount".to_string(), Value::String(fa.to_string()));
    }
    // Class tags ride the line JSON so the projection learns them when the
    // entry posts (drafts never touch the projection). Journals written
    // before classes existed carry no such lines; readers treat missing as
    // unclassified.
    if let Some(c) = l.class_id {
        obj.insert(
            "class_id".to_string(),
            Value::Number(serde_json::Number::from(c)),
        );
    }
    v
}

/// A plain base-currency line, serialized through the one line-JSON path.
fn plain_line_json(account: AccountId, side: Side, amount: Money) -> Value {
    journal_line_json(&ledger_core::JournalLine {
        account,
        side,
        amount,
        foreign_currency: None,
        foreign_amount: None,
        class_id: None,
    })
}

/// One invoice/bill document line as journal JSON. Amounts are decimal
/// strings, never floats; a new field is written only when set, so a line
/// naming none is byte for byte the line every earlier version wrote.
fn document_line_json(l: &DocumentLine) -> Value {
    let mut v = json!({
        "description": l.description,
        "amount": l.amount.to_string(),
    });
    // Its project, behind the feature document_line_projects (rules.md
    // B-27, B-37 item 8); a line naming none is written as before it.
    if let Some(c) = l.class_id {
        v["class_id"] = json!(c);
    }
    // Its account, quantity and unit price, behind documents_v2 (plan P4b
    // S4.3); a line naming none is written as before it.
    if let Some(a) = l.account {
        v["account"] = json!(a);
    }
    if let Some(q) = l.quantity {
        v["quantity"] = json!(q.to_string());
    }
    if let Some(u) = l.unit_price {
        v["unit_price"] = json!(u.to_string());
    }
    v
}

/// The new fields a document write names, for
/// [`Store::ensure_document_fields`]: one of the two record shapes, and the
/// lines (whose account, quantity and unit price also need `documents_v2`).
struct DocumentFields<'a> {
    invoice: Option<&'a arap::InvoiceFields>,
    bill: Option<&'a arap::BillFields>,
    lines: &'a [DocumentLine],
}

impl DocumentFields<'_> {
    /// Whether any field the journal couldn't write before `documents_v2`
    /// is some.
    fn any_new(&self) -> bool {
        let invoice = self.invoice.is_some_and(|f| {
            f.number.is_some()
                || f.terms.is_some()
                || f.bill_to.is_some()
                || f.seller.is_some()
                || f.customer_ref.is_some()
                || f.memo.is_some()
        });
        let bill = self.bill.is_some_and(|f| {
            f.vendor_ref.is_some()
                || f.received_on.is_some()
                || f.terms.is_some()
                || f.memo.is_some()
                || f.duplicate_reason.is_some()
        });
        invoice
            || bill
            || self
                .lines
                .iter()
                .any(|l| l.account.is_some() || l.quantity.is_some() || l.unit_price.is_some())
    }
}

/// Add an invoice's new fields to its `invoice.created` payload, each key
/// only when set (plan P4b S4.3, feature `documents_v2`).
fn invoice_fields_json(payload: &mut Value, fields: &arap::InvoiceFields) {
    if let Some(n) = &fields.number {
        payload["number"] = json!(n);
    }
    if let Some(t) = &fields.terms {
        payload["terms"] = json!(t);
    }
    if let Some(s) = &fields.bill_to {
        payload["bill_to"] = json!(s);
    }
    if let Some(s) = &fields.seller {
        payload["seller"] = json!(s);
    }
    if let Some(r) = &fields.customer_ref {
        payload["customer_ref"] = json!(r);
    }
    if let Some(m) = &fields.memo {
        payload["memo"] = json!(m);
    }
}

/// Add a bill's new fields to its `bill.created` payload, each key only when
/// set. A duplicate's reason is journaled for the record (rules.md B-40 item
/// 4); the bill itself keeps none.
fn bill_fields_json(payload: &mut Value, fields: &arap::BillFields) {
    if let Some(r) = &fields.vendor_ref {
        payload["vendor_ref"] = json!(r);
    }
    if let Some(on) = fields.received_on {
        payload["received_on"] = json!(on.to_string());
    }
    if let Some(t) = &fields.terms {
        payload["terms"] = json!(t);
    }
    if let Some(m) = &fields.memo {
        payload["memo"] = json!(m);
    }
    if let Some(r) = &fields.duplicate_reason {
        payload["duplicate_reason"] = json!(r);
    }
}

/// The invoice's new fields from an `invoice.created` payload; absent keys
/// are the invoice every earlier version wrote.
fn parse_invoice_fields(p: &Value) -> Result<arap::InvoiceFields, StoreError> {
    Ok(arap::InvoiceFields {
        number: opt_str(p, "number")?.map(str::to_string),
        terms: parse_terms(p.get("terms"))?,
        bill_to: parse_snapshot(p.get("bill_to"))?,
        seller: parse_snapshot(p.get("seller"))?,
        customer_ref: opt_str(p, "customer_ref")?.map(str::to_string),
        memo: opt_str(p, "memo")?.map(str::to_string),
    })
}

/// The bill's new fields from a `bill.created` payload; absent keys are the
/// bill every earlier version wrote.
fn parse_bill_fields(p: &Value) -> Result<arap::BillFields, StoreError> {
    Ok(arap::BillFields {
        vendor_ref: opt_str(p, "vendor_ref")?.map(str::to_string),
        received_on: match opt_str(p, "received_on")? {
            Some(d) => Some(parse_date(d)?),
            None => None,
        },
        terms: parse_terms(p.get("terms"))?,
        memo: opt_str(p, "memo")?.map(str::to_string),
        duplicate_reason: opt_str(p, "duplicate_reason")?.map(str::to_string),
    })
}

/// A payload's optional value kept as its canonical JSON text (terms, a
/// party snapshot): absent or null is SQL NULL.
fn json_column(p: &Value, key: &str) -> Option<String> {
    p.get(key).filter(|v| !v.is_null()).map(|v| v.to_string())
}

/// A payload's optional string: absent or null is none, anything but a
/// string is damage.
fn opt_str<'a>(p: &'a Value, key: &str) -> Result<Option<&'a str>, StoreError> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(_) => Err(StoreError::JournalCorrupt(format!("bad {key}"))),
    }
}

/// A payload's optional payment terms, as journaled.
fn parse_terms(v: Option<&Value>) -> Result<Option<arap::Terms>, StoreError> {
    match v {
        None | Some(Value::Null) => Ok(None),
        Some(v) => serde_json::from_value(v.clone())
            .map(Some)
            .map_err(|_| StoreError::JournalCorrupt("bad terms".into())),
    }
}

/// A payload's optional party snapshot, as journaled.
fn parse_snapshot(v: Option<&Value>) -> Result<Option<arap::PartySnapshot>, StoreError> {
    match v {
        None | Some(Value::Null) => Ok(None),
        Some(v) => serde_json::from_value(v.clone())
            .map(Some)
            .map_err(|_| StoreError::JournalCorrupt("bad party snapshot".into())),
    }
}

/// Parse the `lines` array of an invoice/bill journal payload back into
/// document lines.
fn parse_document_lines(p: &Value) -> Result<Vec<DocumentLine>, StoreError> {
    let arr = p
        .get("lines")
        .and_then(|v| v.as_array())
        .ok_or_else(|| StoreError::JournalCorrupt("missing 'lines'".into()))?;
    arr.iter()
        .map(|l| {
            Ok(DocumentLine {
                description: get_str(l, "description")?.to_string(),
                amount: get_str(l, "amount")?
                    .parse()
                    .map_err(|_| StoreError::JournalCorrupt("bad line amount".into()))?,
                // Absent (every document before document_line_projects)
                // or null: no project.
                class_id: match l.get("class_id") {
                    None | Some(Value::Null) => None,
                    Some(c) => Some(
                        c.as_u64()
                            .and_then(|c| ClassId::try_from(c).ok())
                            .ok_or_else(|| {
                                StoreError::JournalCorrupt("bad document line class_id".into())
                            })?,
                    ),
                },
                // Absent (every document before documents_v2) or null:
                // the line names none, exactly the line it was written as.
                account: match l.get("account") {
                    None | Some(Value::Null) => None,
                    Some(a) => Some(a.as_u64().ok_or_else(|| {
                        StoreError::JournalCorrupt("bad document line account".into())
                    })?),
                },
                quantity: opt_decimal(l, "quantity")?,
                unit_price: opt_decimal(l, "unit_price")?,
            })
        })
        .collect()
}

/// A payload's optional exact amount, journaled as a decimal string.
fn opt_decimal(p: &Value, key: &str) -> Result<Option<Decimal>, StoreError> {
    match opt_str(p, key)? {
        None => Ok(None),
        Some(s) => s
            .parse()
            .map(Some)
            .map_err(|_| StoreError::JournalCorrupt(format!("bad {key}"))),
    }
}

fn flip_line_json(l: &ledger_core::JournalLine) -> Value {
    let side = match l.side {
        Side::Debit => "credit",
        Side::Credit => "debit",
    };
    let mut v = json!({
        "account": l.account,
        "side": side,
        "amount": l.amount.amount.to_string(),
        "currency": l.amount.currency.code(),
    });
    // Reversals offset the original line for line: carry the class so
    // divisional P&L nets correctly (mirrors journal_line_json).
    if let Some(c) = l.class_id
        && let Some(obj) = v.as_object_mut()
    {
        obj.insert(
            "class_id".to_string(),
            Value::Number(serde_json::Number::from(c)),
        );
    }
    v
}

/// One payroll line as journal JSON. Amounts are decimal strings, never
/// floats.
fn payroll_line_json(l: &PayrollLine) -> Value {
    json!({
        "employee_name": l.employee_name,
        "employee": l.employee,
        "gross_pay": l.gross_pay.to_string(),
        "federal_withholding": l.federal_withholding.to_string(),
        "state_withholding": l.state_withholding.to_string(),
        "ss_employee": l.ss_employee.to_string(),
        "medicare_employee": l.medicare_employee.to_string(),
        "ss_employer": l.ss_employer.to_string(),
        "medicare_employer": l.medicare_employer.to_string(),
        "other_deductions": l.other_deductions.to_string(),
    })
}

/// Parse the `lines` array of a `payroll.run` journal payload back into
/// payroll lines.
fn parse_payroll_lines(p: &Value) -> Result<Vec<PayrollLine>, StoreError> {
    let arr = p
        .get("lines")
        .and_then(|v| v.as_array())
        .ok_or_else(|| StoreError::JournalCorrupt("missing 'lines'".into()))?;
    arr.iter()
        .map(|l| {
            let amount = |key: &str| {
                get_str(l, key)?
                    .parse()
                    .map_err(|_| StoreError::JournalCorrupt(format!("bad payroll amount '{key}'")))
            };
            Ok(PayrollLine {
                employee_name: get_str(l, "employee_name")?.to_string(),
                employee: match l.get("employee") {
                    None | Some(Value::Null) => None,
                    Some(v) => Some(v.as_u64().ok_or_else(|| {
                        StoreError::JournalCorrupt("payroll line employee".into())
                    })?),
                },
                gross_pay: amount("gross_pay")?,
                federal_withholding: amount("federal_withholding")?,
                state_withholding: amount("state_withholding")?,
                ss_employee: amount("ss_employee")?,
                medicare_employee: amount("medicare_employee")?,
                ss_employer: amount("ss_employer")?,
                medicare_employer: amount("medicare_employer")?,
                other_deductions: amount("other_deductions")?,
            })
        })
        .collect()
}

fn parse_side(s: &str) -> Result<Side, StoreError> {
    match s {
        "debit" => Ok(Side::Debit),
        "credit" => Ok(Side::Credit),
        _ => Err(StoreError::JournalCorrupt(format!("bad side {s}"))),
    }
}

/// A journaled date: any date chrono reads as `%Y-%m-%d`, as every
/// earlier version accepted ("+10000-01-01" and "-0001-01-01" included).
/// The calendar the books keep (ledger_core::calendar) bounds new
/// commands, never replay: a date an earlier version journaled always
/// replays (rules.md B-28).
fn parse_date(s: &str) -> Result<chrono::NaiveDate, StoreError> {
    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map_err(|_| StoreError::JournalCorrupt(format!("bad date {s}")))
}

/// Parse a nested {"amount","currency"} money object.
fn parse_money(p: &Value, key: &str) -> Result<Money, StoreError> {
    let o = p
        .get(key)
        .ok_or_else(|| StoreError::JournalCorrupt(format!("missing '{key}'")))?;
    Ok(Money::new(
        get_str(o, "amount")?
            .parse()
            .map_err(|_| StoreError::JournalCorrupt("bad amount".into()))?,
        Currency::new(get_str(o, "currency")?)
            .map_err(|_| StoreError::JournalCorrupt("bad currency".into()))?,
    ))
}

/// Parse a calendar-month period "YYYY-MM".
fn parse_period(period: &str) -> Result<(i32, u32), StoreError> {
    let err = || StoreError::JournalCorrupt(format!("bad period '{period}': want YYYY-MM"));
    let mut parts = period.split('-');
    let year: i32 = parts.next().ok_or_else(err)?.parse().map_err(|_| err())?;
    let month: u32 = parts.next().ok_or_else(err)?.parse().map_err(|_| err())?;
    // A month of the calendar the books keep (ledger_core::calendar).
    if parts.next().is_some() || !(1..=9999).contains(&year) || !(1..=12).contains(&month) {
        return Err(err());
    }
    Ok((year, month))
}

/// A run's period as (year, month), read leniently, since an earlier
/// version posted "10000-01"; a period no month parses as counts before
/// every month.
fn period_month(period: &str) -> (i32, u32) {
    period
        .split_once('-')
        .and_then(|(y, m)| Some((y.parse::<i32>().ok()?, m.parse::<u32>().ok()?)))
        .filter(|(_, m)| (1..=12).contains(m))
        .unwrap_or((i32::MIN, 1))
}

/// Last calendar day of a month; refused, never a panic, for a month
/// chrono can't reach.
fn last_day_of_month(year: i32, month: u32) -> Result<NaiveDate, StoreError> {
    NaiveDate::from_ymd_opt(year, month, 1)
        .and_then(|first| first.checked_add_months(Months::new(1)))
        .and_then(|next| next.pred_opt())
        .ok_or_else(|| StoreError::JournalCorrupt(format!("no month {year:04}-{month:02}")))
}

fn get_u64(p: &Value, key: &str) -> Result<u64, StoreError> {
    p.get(key)
        .and_then(|v| v.as_u64())
        .ok_or_else(|| StoreError::JournalCorrupt(format!("missing/invalid '{key}'")))
}

fn get_str<'a>(p: &'a Value, key: &str) -> Result<&'a str, StoreError> {
    p.get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| StoreError::JournalCorrupt(format!("missing/invalid '{key}'")))
}

/// Replay one journal line into a ledger, asserting recorded ids. Used by
/// hydration, `verify`, and `rebuild`: the journal is the script, the engine
/// is the actor.
///
/// A full TIN is found as the version before P3 found one
/// (`ledger_core::parties::replaying`): a name, reason or fact an earlier
/// version took replays, however the commands' detector has widened since
/// (rules.md B-28).
pub(crate) fn replay_line_into_ledger(
    ledger: &mut Ledger,
    line: &JournalLine,
    base_currency: Currency,
) -> Result<(), StoreError> {
    ledger_core::parties::replaying(|| replay_line(ledger, line, base_currency))
}

/// [`replay_line_into_ledger`]'s work.
fn replay_line(
    ledger: &mut Ledger,
    line: &JournalLine,
    base_currency: Currency,
) -> Result<(), StoreError> {
    let p = &line.payload;
    match line.line_type.as_str() {
        "store.created" => {
            let code = get_str(p, "base_currency")?;
            if Currency::new(code)
                .map_err(|_| StoreError::JournalCorrupt(format!("bad currency {code}")))?
                != base_currency
            {
                return Err(StoreError::JournalCorrupt(
                    "base currency changed mid-journal".into(),
                ));
            }
        }
        "account.added" => {
            let want = get_u64(p, "id")?;
            let account_type: AccountType = serde_json::from_value(p["account_type"].clone())?;
            let parent = p.get("parent").and_then(|v| v.as_u64());
            let got = ledger.replay_add_account(
                get_str(p, "code")?,
                get_str(p, "name")?,
                account_type,
                parent,
                p.get("placeholder")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
            )?;
            if got != want {
                return Err(StoreError::ReplayMismatch(format!(
                    "account id: journal says {want}, replay got {got}"
                )));
            }
        }
        "entry.created" => {
            let want = get_u64(p, "id")?;
            let got =
                ledger.create_entry(parse_date(get_str(p, "date")?)?, get_str(p, "description")?);
            if got != want {
                return Err(StoreError::ReplayMismatch(format!(
                    "entry id: journal says {want}, replay got {got}"
                )));
            }
        }
        "entry.line_added" => {
            let id = get_u64(p, "id")?;
            let account = get_u64(p, "account")?;
            let side = parse_side(get_str(p, "side")?)?;
            let amount: Decimal = get_str(p, "amount")?
                .parse()
                .map_err(|_| StoreError::JournalCorrupt("bad amount".into()))?;
            let currency = Currency::new(get_str(p, "currency")?)
                .map_err(|_| StoreError::JournalCorrupt("bad currency".into()))?;
            // The fx fields are optional: journals written before
            // foreign-currency lines existed carry none, and replay as
            // plain base-currency lines.
            match p.get("foreign_currency").and_then(|v| v.as_str()) {
                Some(fc) => {
                    let foreign_amount: Decimal = get_str(p, "foreign_amount")?
                        .parse()
                        .map_err(|_| StoreError::JournalCorrupt("bad foreign amount".into()))?;
                    // The journaled base amount is the record: lines booked
                    // before 2026-10-13 were not rounded to the cent, and
                    // replay keeps them exactly as booked.
                    ledger.replay_add_line_fx(
                        id,
                        account,
                        side,
                        Money::new(amount, currency),
                        Currency::new(fc).map_err(|_| {
                            StoreError::JournalCorrupt("bad foreign currency".into())
                        })?,
                        foreign_amount,
                    )?;
                }
                None => {
                    ledger.replay_add_line(id, account, side, Money::new(amount, currency))?;
                }
            }
        }
        "entry.posted" => {
            let id = get_u64(p, "id")?;
            // New journals build the draft via entry.line_added events; old
            // journals (pre-durable-drafts) only have the lines in the
            // posted payload. If the draft already has lines, just post.
            let has_lines = ledger
                .get_entry(id)
                .map(|e| !e.lines().is_empty())
                .unwrap_or(false);
            if !has_lines {
                for l in p.get("lines").and_then(|v| v.as_array()).ok_or_else(|| {
                    StoreError::JournalCorrupt("entry.posted without lines".into())
                })? {
                    ledger.replay_add_line(
                        id,
                        get_u64(l, "account")?,
                        parse_side(get_str(l, "side")?)?,
                        Money::new(
                            get_str(l, "amount")?
                                .parse()
                                .map_err(|_| StoreError::JournalCorrupt("bad amount".into()))?,
                            Currency::new(get_str(l, "currency")?)?,
                        ),
                    )?;
                }
            }
            ledger.replay_post_entry(id)?;
        }
        "entry.reversed" => {
            let want = get_u64(p, "id")?;
            let orig = get_u64(p, "reverses")?;
            // As journaled: a reversal later rules would refuse (a void
            // under a payment that stands) replays (rules.md B-28).
            let got = ledger.replay_reverse_entry(
                orig,
                parse_date(get_str(p, "date")?)?,
                get_str(p, "description")?,
            )?;
            if got != want {
                return Err(StoreError::ReplayMismatch(format!(
                    "reversal id: journal says {want}, replay got {got}"
                )));
            }
        }
        "schedule.registered" => {
            let want = get_u64(p, "id")?;
            // No convention (every schedule before P3): equal months, as
            // they were posted. A convention this version doesn't know is
            // newer, never corrupt (rules.md B-27).
            let convention = match p.get("convention") {
                None | Some(Value::Null) => ledger_core::ScheduleConvention::MonthlyEven,
                Some(Value::String(c)) => {
                    ledger_core::ScheduleConvention::parse(c).ok_or_else(|| {
                        crate::format::newer_content(line, format!("the schedule convention {c}"))
                    })?
                }
                Some(_) => {
                    return Err(StoreError::JournalCorrupt(
                        "schedule.registered convention".into(),
                    ));
                }
            };
            let got = ledger.replay_register_schedule(ledger_core::NewSchedule {
                description: get_str(p, "description")?.to_string(),
                kind: serde_json::from_value(p["kind"].clone())?,
                total: parse_money(p, "total")?,
                start: parse_date(get_str(p, "start")?)?,
                months: get_u64(p, "months")? as u32,
                balance_account: get_u64(p, "balance_account")?,
                pl_account: get_u64(p, "pl_account")?,
                convention,
            })?;
            if got != want {
                return Err(StoreError::ReplayMismatch(format!(
                    "schedule id: journal says {want}, replay got {got}"
                )));
            }
        }
        "schedule.posted" => {
            let got = ledger.post_schedule_month(
                get_u64(p, "schedule_id")?,
                parse_date(get_str(p, "month")?)?,
            )?;
            check_replayed_entry(ledger, got, &p["entry"])?;
        }
        "recurring.registered" => {
            let want = get_u64(p, "id")?;
            let lines = p["lines"]
                .as_array()
                .ok_or_else(|| StoreError::JournalCorrupt("recurring lines".into()))?
                .iter()
                .map(|l| {
                    Ok((
                        get_u64(l, "account")?,
                        parse_side(get_str(l, "side")?)?,
                        Money::new(
                            get_str(l, "amount")?.parse().map_err(|_| {
                                StoreError::JournalCorrupt("recurring amount".into())
                            })?,
                            Currency::new(get_str(l, "currency")?).map_err(|_| {
                                StoreError::JournalCorrupt("recurring currency".into())
                            })?,
                        ),
                    ))
                })
                .collect::<Result<Vec<_>, StoreError>>()?;
            let got = ledger.replay_register_recurring(ledger_core::NewRecurring {
                description: get_str(p, "description")?.to_string(),
                lines,
                frequency: serde_json::from_value(p["frequency"].clone())?,
                start: parse_date(get_str(p, "start")?)?,
                end: match p.get("end").and_then(|v| v.as_str()) {
                    Some(e) => Some(parse_date(e)?),
                    None => None,
                },
            })?;
            if got != want {
                return Err(StoreError::ReplayMismatch(format!(
                    "recurring id: journal says {want}, replay got {got}"
                )));
            }
        }
        "recurring.generated" => {
            let want = get_u64(p, "entry_id")?;
            let got = ledger.replay_generate_recurring(
                get_u64(p, "recurring_id")?,
                parse_date(get_str(p, "date")?)?,
            )?;
            if got != want {
                return Err(StoreError::ReplayMismatch(format!(
                    "recurring draft: journal says {want}, replay got {got}"
                )));
            }
        }
        "vendor.1099_set" => {
            let (vendor, decision, tin_on_file) = crate::form1099::decision_of(line)?;
            ledger.replay_vendor_1099(vendor, decision, tin_on_file)?;
        }
        "vendor.w9_flag_set" => {
            let (vendor, on_file) = crate::form1099::w9_flag_of(p)?;
            ledger.set_vendor_w9_flag(vendor, on_file)?;
        }
        "instrument.added" => {
            let i = crate::form1099::instrument_of(line)?;
            let got = ledger.add_instrument(
                i.kind,
                &i.label,
                i.last4.as_deref(),
                i.owner_party,
                i.account,
            )?;
            if got != i.id {
                return Err(StoreError::ReplayMismatch(format!(
                    "payment method id: journal says {}, replay got {got}",
                    i.id
                )));
            }
        }
        "entry.instrument_set" => {
            let (entry, instrument) = crate::form1099::entry_instrument_of(p)?;
            ledger.set_entry_instrument(entry, instrument)?;
        }
        "entry.vendor_set" => {
            ledger.set_entry_vendor(
                get_u64(p, "entry_id")?,
                p.get("vendor_id").and_then(|v| v.as_u64()),
            )?;
        }
        "account.renumbered" => {
            ledger.renumber_account(get_u64(p, "account_id")?, get_str(p, "code")?)?;
        }
        "account.renamed" => {
            // Only that the account exists: the name rules are the
            // command's (rules.md B-28). `code`, `previous` and `reason`
            // are the record, not state.
            ledger.replay_rename_account(get_u64(p, "account_id")?, get_str(p, "name")?)?;
        }
        "role.assigned" => {
            let account = match p.get("account_id") {
                None | Some(Value::Null) => None,
                Some(v) => Some(v.as_u64().ok_or_else(|| {
                    StoreError::JournalCorrupt("role.assigned account_id".into())
                })?),
            };
            ledger.assign_role(get_str(p, "role")?, account)?;
        }
        "account.tax_set" => {
            let pct = match p.get("deductible_percent").and_then(|v| v.as_str()) {
                Some(s) => Some(s.parse().map_err(|_| {
                    StoreError::JournalCorrupt("account.tax_set deductible_percent".into())
                })?),
                None => None,
            };
            ledger.set_account_tax(
                get_u64(p, "account_id")?,
                p.get("tax_category").and_then(|v| v.as_str()),
                pct,
            )?;
        }
        "attachment.linked"
        | "attachment.unlinked"
        | "statement.recorded"
        | "statement.voided"
        | "reconciliation.completed"
        | "reconciliation.voided" => {
            // Evidence, not accounting state: the projection records them.
        }
        "receipt.attached"
        | "receipt.linked"
        | "receipt.unlinked"
        | "document.filed"
        | "attachment.withdrawn" => {
            // Not accounting state; the projection records them.
        }
        "payroll.taxes_remitted" => {
            // Names the remittance entry (rebuilt by the entry events just
            // before it) for readers; no ledger state of its own.
        }
        "asset.registered" => {
            let want = get_u64(p, "id")?;
            let got = ledger.replay_register_asset(NewAsset {
                description: get_str(p, "description")?.to_string(),
                // `purchase_date` has always meant the in-service date.
                in_service_date: parse_date(get_str(p, "purchase_date")?)?,
                cost: parse_money(p, "cost")?,
                salvage_value: parse_money(p, "salvage_value")?,
                useful_life_months: get_u64(p, "useful_life_months")? as u32,
                cost_account: get_u64(p, "cost_account")?,
                accum_depr_account: get_u64(p, "accum_depr_account")?,
                expense_account: get_u64(p, "expense_account")?,
            })?;
            if got != want {
                return Err(StoreError::ReplayMismatch(format!(
                    "asset id: journal says {want}, replay got {got}"
                )));
            }
        }
        "asset.tagged" => {
            ledger.set_asset_tag(get_u64(p, "asset_id")?, get_str(p, "tag")?)?;
        }
        "party.added" => {
            let want = get_u64(p, "id")?;
            let roles = p
                .get("roles")
                .and_then(|v| v.as_array())
                .ok_or_else(|| StoreError::JournalCorrupt("party.added roles".into()))?
                .iter()
                .map(|r| {
                    r.as_str()
                        .and_then(ledger_core::Role::parse)
                        .ok_or_else(|| StoreError::JournalCorrupt("party.added role".into()))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let got = ledger.add_party(get_str(p, "name")?, &roles)?;
            if got != want {
                return Err(StoreError::ReplayMismatch(format!(
                    "party id: journal says {want}, replay got {got}"
                )));
            }
        }
        "party.updated" => {
            // A field this version doesn't know: a newer one wrote it
            // (rules.md B-27). A known field with a bad value is damage.
            if let Some(field) = p
                .get("change")
                .and_then(|c| c.get("field"))
                .and_then(Value::as_str)
                && !ledger_core::PartyField::known_field(field)
            {
                return Err(crate::format::newer_content(
                    line,
                    format!("the party field {field}"),
                ));
            }
            let field: ledger_core::PartyField = serde_json::from_value(p["change"].clone())
                .map_err(|_| StoreError::JournalCorrupt("party.updated change".into()))?;
            ledger.update_party(get_u64(p, "id")?, field)?;
        }
        "party.role_set" => {
            let role = ledger_core::Role::parse(get_str(p, "role")?)
                .ok_or_else(|| StoreError::JournalCorrupt("party.role_set role".into()))?;
            let on = p
                .get("on")
                .and_then(|v| v.as_bool())
                .ok_or_else(|| StoreError::JournalCorrupt("party.role_set on".into()))?;
            ledger.set_party_role(get_u64(p, "id")?, role, on)?;
        }
        "party.creditor_set" => {
            let on = p
                .get("on")
                .and_then(|v| v.as_bool())
                .ok_or_else(|| StoreError::JournalCorrupt("party.creditor_set on".into()))?;
            ledger.set_party_creditor(get_u64(p, "id")?, get_u64(p, "account_id")?, on)?;
        }
        "party.w9_set" => {
            let facts: Option<ledger_core::W9Facts> = serde_json::from_value(p["facts"].clone())
                .map_err(|_| StoreError::JournalCorrupt("party.w9_set facts".into()))?;
            ledger.set_party_w9(get_u64(p, "id")?, facts)?;
        }
        "party.w4_set" => {
            let facts: Option<ledger_core::W4Facts> = serde_json::from_value(p["facts"].clone())
                .map_err(|_| StoreError::JournalCorrupt("party.w4_set facts".into()))?;
            ledger.set_party_w4(get_u64(p, "id")?, facts)?;
        }
        "party.tin_set" => {
            let facts: Option<ledger_core::TinFacts> =
                serde_json::from_value(p["facts"].clone())
                    .map_err(|_| StoreError::JournalCorrupt("party.tin_set facts".into()))?;
            ledger.set_party_tin(get_u64(p, "id")?, facts)?;
        }
        "entry.party_set" => {
            let party = match p.get("party_id") {
                None | Some(Value::Null) => None,
                Some(v) => Some(v.as_u64().ok_or_else(|| {
                    StoreError::JournalCorrupt("entry.party_set party_id".into())
                })?),
            };
            ledger.set_entry_party(get_u64(p, "entry_id")?, party)?;
        }
        "entry.parties_set" => {
            let parties: Vec<ledger_core::PartyShare> =
                serde_json::from_value(p["parties"].clone())
                    .map_err(|_| StoreError::JournalCorrupt("entry.parties_set parties".into()))?;
            ledger.replay_set_entry_parties(get_u64(p, "entry_id")?, &parties)?;
        }
        "entry.no_party" => {
            ledger.set_entry_no_party(get_u64(p, "entry_id")?, get_str(p, "reason")?)?;
        }
        "asset.corrected" => {
            ledger.replay_correct_asset(
                get_u64(p, "asset_id")?,
                ledger_core::AssetCorrection {
                    cost: parse_money(p, "cost")?,
                    salvage_value: parse_money(p, "salvage_value")?,
                    useful_life_months: get_u64(p, "useful_life_months")? as u32,
                    in_service_date: parse_date(get_str(p, "purchase_date")?)?,
                },
            )?;
        }
        "asset.tax_basis_set" => {
            let (rule, contributor_basis) = tax_basis_from_journal(p)?;
            ledger.replay_set_asset_tax_basis(get_u64(p, "asset_id")?, rule, contributor_basis)?;
        }
        "asset.basis_set" => {
            let basis = asset_basis_from_journal(line, p)?;
            ledger.replay_set_asset_basis(get_u64(p, "asset_id")?, basis)?;
        }
        "asset.described" => {
            ledger.set_asset_description(get_u64(p, "asset_id")?, get_str(p, "description")?)?;
        }
        "asset.acquisition_linked" => {
            ledger
                .replay_set_asset_acquisition(get_u64(p, "asset_id")?, get_u64(p, "entry_id")?)?;
        }
        "asset.serial_set" => {
            let serial =
                match p.get("serial") {
                    None | Some(Value::Null) => None,
                    Some(v) => Some(v.as_str().ok_or_else(|| {
                        StoreError::JournalCorrupt("asset.serial_set serial".into())
                    })?),
                };
            ledger.set_asset_serial(get_u64(p, "asset_id")?, serial)?;
        }
        "asset.depreciation_posted" => {
            // The entry itself was journaled as entry.created/entry.posted;
            // this line says it was the asset's run for the month, which is
            // what the books carry for the asset (Ledger::accumulated_posted).
            // Replay checks only that it is well formed and names an asset
            // and entry that exist (rules.md B-28).
            let amount = get_str(p, "amount")?.parse::<Decimal>().map_err(|_| {
                StoreError::JournalCorrupt("asset.depreciation_posted amount".into())
            })?;
            ledger.record_depreciation_run(
                get_u64(p, "asset_id")?,
                get_str(p, "period")?,
                get_u64(p, "entry_id")?,
                amount,
            )?;
        }
        "asset.disposed" => {
            let id = get_u64(p, "asset_id")?;
            ledger.replay_dispose_asset(
                id,
                parse_date(get_str(p, "date")?)?,
                parse_money(p, "proceeds")?,
            )?;
        }
        "period.created" => {
            let want = get_u64(p, "id")?;
            let got = ledger.replay_create_period(
                get_str(p, "name")?,
                parse_date(get_str(p, "start")?)?,
                parse_date(get_str(p, "end")?)?,
            )?;
            if got != want {
                return Err(StoreError::ReplayMismatch(format!(
                    "period id: journal says {want}, replay got {got}"
                )));
            }
        }
        "period.closed" => {
            let id = get_u64(p, "id")?;
            let want = p.get("closing_entry").and_then(|v| v.as_u64());
            // Deterministic: the close re-derives the closing entry from
            // the replayed state and the journaled target (absent: Retained
            // Earnings), so it must match the journal's id and every line.
            let to = close_to_from_journal(line, p.get("close_to"))?;
            let got = ledger.close_period_to(id, &to)?;
            if got != want {
                return Err(StoreError::ReplayMismatch(format!(
                    "closing entry: journal says {want:?}, replay got {got:?}"
                )));
            }
            if let Some(eid) = got {
                let replayed: Vec<Value> = ledger
                    .get_entry(eid)
                    .expect("close posted it")
                    .lines()
                    .iter()
                    .map(journal_line_json)
                    .collect();
                if p["entry"]["lines"].as_array() != Some(&replayed) {
                    return Err(StoreError::ReplayMismatch(format!(
                        "closing entry {eid}: replayed lines differ from the journal"
                    )));
                }
            }
        }
        "line.cleared" => {
            ledger.mark_cleared(
                get_u64(p, "entry_id")?,
                get_u64(p, "line_index")? as usize,
                p.get("cleared").and_then(|v| v.as_bool()).unwrap_or(false),
            )?;
        }
        "customer.added" => {
            let want = get_u64(p, "id")?;
            let got = ledger.add_customer(get_str(p, "name")?)?;
            if got != want {
                return Err(StoreError::ReplayMismatch(format!(
                    "customer id: journal says {want}, replay got {got}"
                )));
            }
        }
        "vendor.added" => {
            let want = get_u64(p, "id")?;
            let got = ledger.add_vendor(get_str(p, "name")?)?;
            if got != want {
                return Err(StoreError::ReplayMismatch(format!(
                    "vendor id: journal says {want}, replay got {got}"
                )));
            }
        }
        "invoice.created" => {
            // The entry was already rebuilt by the standard entry handlers;
            // this only restores the invoice record, without re-posting.
            ledger.replay_invoice_with(
                get_u64(p, "id")?,
                get_u64(p, "customer_id")?,
                parse_date(get_str(p, "date")?)?,
                parse_date(get_str(p, "due_date")?)?,
                parse_document_lines(p)?,
                get_u64(p, "entry_id")?,
                parse_invoice_fields(p)?,
            )?;
        }
        "bill.created" => {
            ledger.replay_bill_with(
                get_u64(p, "id")?,
                get_u64(p, "vendor_id")?,
                parse_date(get_str(p, "date")?)?,
                parse_date(get_str(p, "due_date")?)?,
                parse_document_lines(p)?,
                get_u64(p, "entry_id")?,
                parse_bill_fields(p)?,
            )?;
        }
        "invoice.payment_received" => {
            // The payment entry was already rebuilt; this only moves the
            // invoice's paid amount and status.
            ledger.replay_invoice_payment(
                get_u64(p, "invoice_id")?,
                get_str(p, "amount")?
                    .parse()
                    .map_err(|_| StoreError::JournalCorrupt("bad amount".into()))?,
                get_u64(p, "entry_id")?,
            )?;
        }
        "bill.paid" => {
            ledger.replay_bill_payment(
                get_u64(p, "bill_id")?,
                get_str(p, "amount")?
                    .parse()
                    .map_err(|_| StoreError::JournalCorrupt("bad amount".into()))?,
                get_u64(p, "entry_id")?,
            )?;
        }
        "fx.rate_set" => {
            ledger.set_fx_rate(
                Currency::new(get_str(p, "currency")?)
                    .map_err(|_| StoreError::JournalCorrupt("bad currency".into()))?,
                parse_date(get_str(p, "date")?)?,
                get_str(p, "rate")?
                    .parse()
                    .map_err(|_| StoreError::JournalCorrupt("bad rate".into()))?,
            )?;
        }
        "payroll.run" => {
            // The entry was already rebuilt by the standard entry handlers;
            // this only restores the run record, without re-posting.
            ledger.replay_payroll_run(
                get_u64(p, "id")?,
                parse_date(get_str(p, "date")?)?,
                parse_payroll_lines(p)?,
                get_u64(p, "entry_id")?,
            )?;
        }
        "class.added" => {
            ledger.replay_class(get_u64(p, "id")? as u32, get_str(p, "name")?.to_string())?;
        }
        "project.set" => {
            let (id, name, facts) = crate::meta::project_set_of(line)?;
            ledger.replay_set_project(id, name, facts)?;
        }
        "entry.line_class_set" => {
            // Journaled while the entry was a draft, so at replay time the
            // draft exists and takes the class. Missing/null class_id
            // clears the tag.
            ledger.set_line_class(
                get_u64(p, "entry_id")?,
                get_u64(p, "line_index")? as usize,
                p.get("class_id").and_then(|v| v.as_u64()).map(|v| v as u32),
            )?;
        }
        "settings.set" => {
            let (changes, source) = crate::meta::settings_set_of(p)?;
            // A key this version doesn't know: a newer one wrote it.
            if let Some(c) = changes
                .iter()
                .find(|c| ledger_core::settings::key(&c.key).is_none())
            {
                return Err(crate::format::newer_content(
                    line,
                    format!("the setting {}", c.key),
                ));
            }
            ledger.apply_settings(&changes, &source, line.seq)?;
        }
        "convention.set" => {
            // A kind this version doesn't know: a newer one wrote it.
            if let Some(kind) = p.get("kind").and_then(Value::as_str)
                && ledger_core::conventions::ConventionKind::parse(kind).is_none()
            {
                return Err(crate::format::newer_content(
                    line,
                    format!("the convention kind {kind}"),
                ));
            }
            let c = crate::meta::convention_set_of(p)?;
            // A template or rule whose structure this version can't read:
            // a newer one wrote it (never skipped: a rule must not fail
            // open).
            let probe = ledger_core::conventions::Convention {
                id: c.id.clone(),
                kind: c.kind,
                title: String::new(),
                body_md: String::new(),
                structured: c.structured.clone(),
                source: c.source.clone(),
                seq: line.seq,
                revision: 1,
                retired: None,
            };
            let unreadable = match c.kind {
                ledger_core::conventions::ConventionKind::PostingTemplate => {
                    probe.template().is_none()
                }
                ledger_core::conventions::ConventionKind::ApprovalRule => {
                    probe.approval_rule().is_none()
                }
                _ => false,
            };
            if unreadable {
                return Err(crate::format::newer_content(
                    line,
                    format!("the structure of convention {}", c.id),
                ));
            }
            ledger.apply_convention(c, line.seq)?;
        }
        "convention.retired" => {
            let (id, reason, source) = crate::meta::convention_retired_of(p)?;
            ledger.apply_retire_convention(&id, reason, source, line.seq)?;
        }
        "decision.recorded" => {
            let (id, d) = crate::meta::decision_recorded_of(p)?;
            ledger.apply_decision(id, d, line.seq)?;
        }
        "open_item.raised" => {
            let (id, i) = crate::meta::open_item_raised_of(p)?;
            ledger.apply_open_item(id, i, line.seq)?;
        }
        "open_item.resolved" => {
            let (id, resolution, source) = crate::meta::open_item_resolved_of(p)?;
            ledger.apply_resolve_item(id, resolution, source, line.seq)?;
        }
        "entry.approved" => {
            let (entry, approval) = crate::meta::entry_approved_of(p, line.seq)?;
            ledger.apply_approval(entry, approval)?;
        }
        "journal.tail_repaired" => {}
        "journal.format_set" => {
            // No ledger state; a format this version can't read is
            // refused here too, for callers that skip the gate.
            crate::format::declared(line)?;
        }
        // A whole, chained line of a type this version doesn't know: a
        // newer wanderbooks wrote it (the format gate).
        _ => return Err(crate::format::unknown_type(line)),
    }
    Ok(())
}

/// Apply one journal line to the SQLite projection: INSERTs plus the events
/// mirror, then advance `last_applied_seq`. The applier is a pure function
/// of the journal line: INSERTs for new facts, targeted UPDATEs/DELETEs
/// for lifecycle changes (receipt linking, disposal, period close,
/// cleared flags, fx rate updates).
/// The close target recorded in a `period.closed` line. Absent or
/// `"retained_earnings"`: Retained Earnings with no sweeps (every journal
/// written before targets existed).
/// An `asset.tax_basis_set` payload's rule and contributor basis, strictly:
/// an unknown rule or a basis that isn't a decimal string is corrupt.
/// A pre-P3 `asset.tax_basis_set`: the rule by its pre-P3 name (`cost`,
/// `carryover`, `personal_use_conversion`) and the contributor basis.
fn tax_basis_from_journal(
    p: &Value,
) -> Result<(ledger_core::BasisRule, Option<Decimal>), StoreError> {
    let corrupt = |what: &str| StoreError::JournalCorrupt(format!("asset.tax_basis_set {what}"));
    let rule =
        ledger_core::BasisRule::parse_legacy(get_str(p, "rule")?).ok_or_else(|| corrupt("rule"))?;
    Ok((
        rule,
        optional_decimal(p, "contributor_basis").map_err(|_| corrupt("contributor_basis"))?,
    ))
}

/// An optional decimal string: absent or null is None; anything else must
/// parse.
fn optional_decimal(p: &Value, key: &str) -> Result<Option<Decimal>, ()> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_str()
            .and_then(|s| s.parse::<Decimal>().ok())
            .map(Some)
            .ok_or(()),
    }
}

/// An `asset.basis_set`'s facts. A rule or §704(c) method this version
/// doesn't know was written by a newer one: NewerJournal, never corrupt
/// (rules.md B-27). The figures the line carries are type-checked and
/// otherwise left to its readers: replay never recomputes them (B-28).
fn asset_basis_from_journal(
    line: &JournalLine,
    p: &Value,
) -> Result<ledger_core::AssetBasis, StoreError> {
    let corrupt = |what: &str| StoreError::JournalCorrupt(format!("asset.basis_set {what}"));
    let key = get_str(p, "rule")?;
    let rule = ledger_core::BasisRule::parse(key)
        .ok_or_else(|| crate::format::newer_content(line, format!("the basis rule {key:?}")))?;
    let sec704c_method = match p.get("sec704c_method") {
        None | Some(Value::Null) => None,
        Some(v) => {
            let key = v.as_str().ok_or_else(|| corrupt("sec704c_method"))?;
            Some(ledger_core::Sec704cMethod::parse(key).ok_or_else(|| {
                crate::format::newer_content(line, format!("the §704(c) method {key:?}"))
            })?)
        }
    };
    let source = match p.get("source") {
        None | Some(Value::Null) => None,
        Some(v) => Some(serde_json::from_value(v.clone()).map_err(|_| corrupt("source"))?),
    };
    for figure in ["tax_basis_723", "depreciable_basis", "built_in_gain_704c"] {
        optional_decimal(p, figure).map_err(|_| corrupt(figure))?;
    }
    Ok(ledger_core::AssetBasis {
        rule,
        contributor_adjusted_basis: optional_decimal(p, "contributor_adjusted_basis")
            .map_err(|_| corrupt("contributor_adjusted_basis"))?,
        fmv_at_contribution: optional_decimal(p, "fmv_at_contribution")
            .map_err(|_| corrupt("fmv_at_contribution"))?,
        sec704c_method,
        source,
    })
}

/// A journaled close's target. A rounding rule only a newer version
/// writes (an unknown name, or a value that isn't a name) is newer, never
/// corrupt (rules.md B-27).
fn close_to_from_journal(line: &JournalLine, v: Option<&Value>) -> Result<CloseTo, StoreError> {
    let corrupt = |what: &str| StoreError::JournalCorrupt(format!("close_to: {what}"));
    let ids = |v: &Value| -> Result<Vec<AccountId>, StoreError> {
        v.as_array()
            .ok_or_else(|| corrupt("sweep"))?
            .iter()
            .map(|x| x.as_u64().ok_or_else(|| corrupt("sweep id")))
            .collect()
    };
    let Some(v) = v.filter(|v| !v.is_null() && v.as_str() != Some("retained_earnings")) else {
        return Ok(CloseTo::RetainedEarnings { sweep: vec![] });
    };
    if let Some(re) = v.get("retained_earnings") {
        return Ok(CloseTo::RetainedEarnings {
            sweep: ids(&re["sweep"])?,
        });
    }
    if let Some(a) = v.get("account") {
        return Ok(CloseTo::Account {
            into: get_u64(a, "into")?,
            sweep: ids(&a["sweep"])?,
        });
    }
    let members = v
        .get("members")
        .and_then(|m| m.as_array())
        .ok_or_else(|| corrupt("unknown target"))?;
    let members = members
        .iter()
        .map(|m| {
            Ok(ledger_core::MemberClose {
                capital: get_u64(m, "account")?,
                share: get_str(m, "share")?
                    .parse()
                    .map_err(|_| corrupt("member share"))?,
                distributions: m.get("distributions").and_then(|d| d.as_u64()),
            })
        })
        .collect::<Result<_, StoreError>>()?;
    // The rounding rule the close was made under: absent before
    // 2026-10-13, when the last member absorbed it; "largest_remainder"
    // per close, then "cumulative" over a run of closes, then (P3)
    // "cumulative_since" the close numbered since_seq.
    let rounding = match v.get("rounding") {
        None | Some(Value::Null) => None,
        Some(Value::String(r)) => Some(r.as_str()),
        Some(other) => {
            return Err(crate::format::newer_content(
                line,
                format!("a members close's rounding {other}"),
            ));
        }
    };
    let since_seq = v.get("since_seq");
    let older = |to: CloseTo| match since_seq {
        // Only "cumulative_since" counts from a close.
        Some(_) => Err(corrupt("since_seq beside another rounding")),
        None => Ok(to),
    };
    match rounding {
        None => older(CloseTo::MembersLastAbsorbs(members)),
        Some("largest_remainder") => older(CloseTo::MembersPerClose(members)),
        Some("cumulative") => older(CloseTo::MembersRun(members)),
        Some("cumulative_since") => Ok(CloseTo::MembersCumulative {
            members,
            since_seq: since_seq
                .and_then(Value::as_u64)
                .filter(|&s| s >= 1)
                .ok_or_else(|| corrupt("since_seq"))?,
        }),
        Some(other) => Err(crate::format::newer_content(
            line,
            format!("a members close's rounding {other:?}"),
        )),
    }
}

/// Record a posted entry that rides inside another event's payload (a
/// closing entry, a schedule posting) in the projection, like any posted
/// entry.
fn project_posted_entry(conn: &Connection, e: &Value) -> Result<(), StoreError> {
    let eid = get_u64(e, "id")? as i64;
    conn.execute(
        "INSERT INTO entries(id, date, description, status, reversal_of) VALUES (?,?,?,'posted',NULL)",
        params![eid, get_str(e, "date")?, get_str(e, "description")?],
    )?;
    for l in e
        .get("lines")
        .and_then(|v| v.as_array())
        .ok_or_else(|| StoreError::JournalCorrupt("posted entry without lines".into()))?
    {
        conn.execute(
            "INSERT INTO lines(entry_id, account_id, side, amount, currency, foreign_currency, foreign_amount, class_id) VALUES (?,?,?,?,?,?,?,?)",
            params![
                eid,
                get_u64(l, "account")? as i64,
                get_str(l, "side")?,
                get_str(l, "amount")?,
                get_str(l, "currency")?,
                l.get("foreign_currency").and_then(|v| v.as_str()),
                l.get("foreign_amount").and_then(|v| v.as_str()),
                l.get("class_id").and_then(|v| v.as_u64()).map(|v| v as i64),
            ],
        )?;
    }
    Ok(())
}

/// The full detail of a posted entry, for events that carry one.
fn entry_detail_json(ledger: &Ledger, id: EntryId) -> Value {
    let e = ledger.get_entry(id).expect("entry just posted");
    json!({
        "id": id,
        "date": e.date().to_string(),
        "description": e.description(),
        "lines": e.lines().iter().map(journal_line_json).collect::<Vec<_>>(),
    })
}

/// On replay, an entry re-derived from master data must match the journal
/// line for line.
fn check_replayed_entry(
    ledger: &Ledger,
    got: EntryId,
    journaled: &Value,
) -> Result<(), StoreError> {
    let want = get_u64(journaled, "id")?;
    if got != want {
        return Err(StoreError::ReplayMismatch(format!(
            "entry id: journal says {want}, replay got {got}"
        )));
    }
    let lines: Vec<Value> = ledger
        .get_entry(got)
        .expect("replay posted it")
        .lines()
        .iter()
        .map(journal_line_json)
        .collect();
    if journaled["lines"].as_array() != Some(&lines) {
        return Err(StoreError::ReplayMismatch(format!(
            "entry {got}: replayed lines differ from the journal"
        )));
    }
    Ok(())
}

pub(crate) fn apply_line_to_sqlite(
    conn: &Connection,
    line: &JournalLine,
) -> Result<(), StoreError> {
    // All or nothing, per line: the rows, the events mirror, and
    // last_applied_seq commit together. Without this a write that failed
    // partway (a full disk) committed the rows but not last_applied_seq,
    // and the next open's catch-up re-applied the line into a UNIQUE
    // violation: the books would not open (found by the full-disk fault
    // test, 2026-10-13). A savepoint, so it nests inside a caller's
    // transaction.
    conn.execute_batch("SAVEPOINT apply_line")?;
    match apply_line_rows(conn, line) {
        Ok(()) => {
            conn.execute_batch("RELEASE apply_line")?;
            Ok(())
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK TO apply_line; RELEASE apply_line");
            Err(e)
        }
    }
}

/// The statements behind [`apply_line_to_sqlite`], run inside its
/// savepoint.
fn apply_line_rows(conn: &Connection, line: &JournalLine) -> Result<(), StoreError> {
    let p = &line.payload;
    match line.line_type.as_str() {
        "store.created" | "journal.tail_repaired" | "journal.format_set" => {}
        "account.added" => {
            conn.execute(
                "INSERT INTO accounts(id, code, name, type, parent, placeholder, currency) VALUES (?,?,?,?,?,?,?)",
                params![
                    get_u64(p, "id")? as i64,
                    get_str(p, "code")?,
                    get_str(p, "name")?,
                    p["account_type"].as_str().unwrap_or(""),
                    p.get("parent").and_then(|v| v.as_u64()).map(|v| v as i64),
                    p.get("placeholder").and_then(|v| v.as_bool()).unwrap_or(false) as i64,
                    get_str(p, "currency")?,
                ],
            )?;
        }
        "entry.created" => {}
        // Draft lines are durable in the journal but never touch the
        // projection until posted.
        "entry.line_added" => {}
        "entry.posted" => {
            conn.execute(
                "INSERT INTO entries(id, date, description, status, reversal_of) VALUES (?,?,?,'posted',?)",
                params![
                    get_u64(p, "id")? as i64,
                    get_str(p, "date")?,
                    get_str(p, "description")?,
                    p.get("reversal_of").and_then(|v| v.as_u64()).map(|v| v as i64),
                ],
            )?;
            for l in p
                .get("lines")
                .and_then(|v| v.as_array())
                .ok_or_else(|| StoreError::JournalCorrupt("entry.posted without lines".into()))?
            {
                conn.execute(
                    "INSERT INTO lines(entry_id, account_id, side, amount, currency, foreign_currency, foreign_amount, class_id) VALUES (?,?,?,?,?,?,?,?)",
                    params![
                        get_u64(p, "id")? as i64,
                        get_u64(l, "account")? as i64,
                        get_str(l, "side")?,
                        get_str(l, "amount")?,
                        get_str(l, "currency")?,
                        l.get("foreign_currency").and_then(|v| v.as_str()),
                        l.get("foreign_amount").and_then(|v| v.as_str()),
                        l.get("class_id").and_then(|v| v.as_u64()).map(|v| v as i64),
                    ],
                )?;
            }
        }
        "entry.reversed" => {
            // A reversed payment un-pays the vendor: carry the tag.
            conn.execute(
                "INSERT OR REPLACE INTO entry_vendors(entry_id, vendor_id) \
                 SELECT ?, vendor_id FROM entry_vendors WHERE entry_id = ?",
                params![get_u64(p, "id")? as i64, get_u64(p, "reverses")? as i64],
            )?;
            // And the way it was paid.
            conn.execute(
                "INSERT OR REPLACE INTO entry_instruments(entry_id, instrument_id) \
                 SELECT ?, instrument_id FROM entry_instruments WHERE entry_id = ?",
                params![get_u64(p, "id")? as i64, get_u64(p, "reverses")? as i64],
            )?;
            conn.execute(
                "INSERT INTO entries(id, date, description, status, reversal_of) VALUES (?,?,?,'posted',?)",
                params![
                    get_u64(p, "id")? as i64,
                    get_str(p, "date")?,
                    get_str(p, "description")?,
                    get_u64(p, "reverses")? as i64,
                ],
            )?;
            for l in p
                .get("lines")
                .and_then(|v| v.as_array())
                .ok_or_else(|| StoreError::JournalCorrupt("entry.reversed without lines".into()))?
            {
                conn.execute(
                    "INSERT INTO lines(entry_id, account_id, side, amount, currency, foreign_currency, foreign_amount, class_id) VALUES (?,?,?,?,?,?,?,?)",
                    params![
                        get_u64(p, "id")? as i64,
                        get_u64(l, "account")? as i64,
                        get_str(l, "side")?,
                        get_str(l, "amount")?,
                        get_str(l, "currency")?,
                        l.get("foreign_currency").and_then(|v| v.as_str()),
                        l.get("foreign_amount").and_then(|v| v.as_str()),
                        l.get("class_id").and_then(|v| v.as_u64()).map(|v| v as i64),
                    ],
                )?;
            }
        }
        "payroll.taxes_remitted" => {
            // The entry was projected by its own events; this only names it.
        }
        "attachment.linked" | "attachment.unlinked" => {
            let sub = &p["subject"];
            let kind = get_str(sub, "kind")?;
            let id = sub.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
            if Subject::from_parts(kind, id).is_none() {
                return Err(StoreError::JournalCorrupt(format!(
                    "attachment subject {kind}"
                )));
            }
            let att = get_u64(p, "attachment_id")? as i64;
            if line.line_type == "attachment.linked" {
                conn.execute(
                    "INSERT INTO attachment_links(attachment_id, subject_kind, subject_id) VALUES (?,?,?)",
                    params![att, kind, id as i64],
                )?;
            } else {
                conn.execute(
                    "DELETE FROM attachment_links WHERE attachment_id = ? AND subject_kind = ? AND subject_id = ?",
                    params![att, kind, id as i64],
                )?;
            }
        }
        "statement.recorded" => {
            conn.execute(
                "INSERT INTO statements(attachment_id, account_id, period_start, period_end, opening_balance, closing_balance) VALUES (?,?,?,?,?,?)",
                params![
                    get_u64(p, "attachment_id")? as i64,
                    get_u64(p, "account_id")? as i64,
                    get_str(p, "period_start")?,
                    get_str(p, "period_end")?,
                    get_str(p, "opening_balance")?,
                    get_str(p, "closing_balance")?,
                ],
            )?;
        }
        "statement.voided" => {
            // The journal keeps what it said; the projection lists only
            // statements that stand.
            conn.execute(
                "DELETE FROM statements WHERE attachment_id = ?",
                params![get_u64(p, "attachment_id")? as i64],
            )?;
        }
        "reconciliation.completed" => {
            let id = get_u64(p, "id")? as i64;
            conn.execute(
                "INSERT INTO reconciliations(id, account_id, statement_date, statement_balance, attachment_id) VALUES (?,?,?,?,?)",
                params![
                    id,
                    get_u64(p, "account_id")? as i64,
                    get_str(p, "statement_date")?,
                    get_str(p, "statement_balance")?,
                    get_u64(p, "attachment_id")? as i64,
                ],
            )?;
            let cleared = p
                .get("cleared")
                .and_then(|v| v.as_array())
                .ok_or_else(|| StoreError::JournalCorrupt("reconciliation cleared".into()))?;
            for pair in cleared {
                let e = pair.get(0).and_then(|v| v.as_u64());
                let l = pair.get(1).and_then(|v| v.as_u64());
                let (Some(e), Some(l)) = (e, l) else {
                    return Err(StoreError::JournalCorrupt(
                        "reconciliation cleared line".into(),
                    ));
                };
                conn.execute(
                    "INSERT INTO reconciliation_lines(reconciliation_id, entry_id, line_index) VALUES (?,?,?)",
                    params![id, e as i64, l as i64],
                )?;
            }
        }
        "reconciliation.voided" => {
            conn.execute(
                "UPDATE reconciliations SET voided_reason = ? WHERE id = ?",
                params![get_str(p, "reason")?, get_u64(p, "id")? as i64],
            )?;
        }
        "role.assigned" | "party.added" | "party.updated" | "party.role_set" | "party.w9_set"
        | "party.w4_set" | "party.tin_set" | "party.creditor_set" | "entry.party_set"
        | "entry.parties_set" | "entry.no_party" => {
            // Control roles and the party directory live in the ledger
            // (rebuilt by replay); the events mirror below keeps the
            // history.
        }
        "asset.tagged" => {
            let (id, tag) = (get_u64(p, "asset_id")? as i64, get_str(p, "tag")?);
            conn.execute("UPDATE assets SET tag = ? WHERE id = ?", params![tag, id])?;
            conn.execute(
                "INSERT INTO asset_tags_log(asset_id, tag) VALUES (?, ?)",
                params![id, tag],
            )?;
        }
        "asset.corrected" => {
            let amt = |k: &str| -> Result<String, StoreError> {
                Ok(get_str(&p[k], "amount")?.to_string())
            };
            conn.execute(
                "UPDATE assets SET cost_amount = ?, salvage_amount = ?, useful_life_months = ?, purchase_date = ? WHERE id = ?",
                params![
                    amt("cost")?,
                    amt("salvage_value")?,
                    get_u64(p, "useful_life_months")? as i64,
                    get_str(p, "purchase_date")?,
                    get_u64(p, "asset_id")? as i64
                ],
            )?;
        }
        "asset.tax_basis_set" => {
            // A pre-P3 record: its rule by its pre-P3 name, no value at
            // contribution, method or source.
            conn.execute(
                "UPDATE assets SET tax_rule = ?, contributor_basis = ?, fmv_at_contribution = NULL, sec704c_method = NULL, basis_source_json = NULL WHERE id = ?",
                params![
                    get_str(p, "rule")?,
                    p.get("contributor_basis").and_then(|v| v.as_str()),
                    get_u64(p, "asset_id")? as i64
                ],
            )?;
        }
        "asset.basis_set" => {
            // The facts, the rule by its wire name (tax_rule holds either
            // set: the names differ but for personal_use_conversion, which
            // means the same in both).
            let source = match p.get("source") {
                None | Some(Value::Null) => None,
                Some(v) => Some(v.to_string()),
            };
            conn.execute(
                "UPDATE assets SET tax_rule = ?, contributor_basis = ?, fmv_at_contribution = ?, sec704c_method = ?, basis_source_json = ? WHERE id = ?",
                params![
                    get_str(p, "rule")?,
                    p.get("contributor_adjusted_basis").and_then(|v| v.as_str()),
                    p.get("fmv_at_contribution").and_then(|v| v.as_str()),
                    p.get("sec704c_method").and_then(|v| v.as_str()),
                    source,
                    get_u64(p, "asset_id")? as i64
                ],
            )?;
        }
        "asset.described" => {
            conn.execute(
                "UPDATE assets SET description = ? WHERE id = ?",
                params![get_str(p, "description")?, get_u64(p, "asset_id")? as i64],
            )?;
        }
        "asset.acquisition_linked" => {
            conn.execute(
                "UPDATE assets SET acquisition_entry = ? WHERE id = ?",
                params![
                    get_u64(p, "entry_id")? as i64,
                    get_u64(p, "asset_id")? as i64
                ],
            )?;
        }
        "asset.serial_set" => {
            conn.execute(
                "UPDATE assets SET serial = ? WHERE id = ?",
                params![
                    p.get("serial").and_then(|v| v.as_str()),
                    get_u64(p, "asset_id")? as i64
                ],
            )?;
        }
        "receipt.attached" | "document.filed" => {
            conn.execute(
                "INSERT OR IGNORE INTO blobs(sha256, size_bytes, mime_type, original_name, created_at) VALUES (?,?,?,?,?)",
                params![
                    get_str(p, "blob_sha")?,
                    get_u64(p, "size_bytes")? as i64,
                    get_str(p, "mime_type")?,
                    get_str(p, "file_name")?,
                    line.ts,
                ],
            )?;
            conn.execute(
                "INSERT INTO attachments(id, blob_sha, entry_id, category, description, created_at, file_name, mime_type) VALUES (?,?,?,?,?,?,?,?)",
                params![
                    get_u64(p, "attachment_id")? as i64,
                    get_str(p, "blob_sha")?,
                    p.get("entry_id").and_then(|v| v.as_u64()).map(|v| v as i64),
                    get_str(p, "category")?,
                    p.get("description").and_then(|v| v.as_str()),
                    line.ts,
                    get_str(p, "file_name")?,
                    get_str(p, "mime_type")?,
                ],
            )?;
        }
        "receipt.linked" => {
            conn.execute(
                "UPDATE attachments SET entry_id = ? WHERE id = ?",
                params![
                    get_u64(p, "entry_id")? as i64,
                    get_u64(p, "attachment_id")? as i64,
                ],
            )?;
        }
        "vendor.1099_set" => {
            let (vendor, d, tin_on_file) = crate::form1099::decision_of(line)?;
            let key = match &d.decision {
                ledger_core::Decision1099::Undetermined => None,
                other => Some(other.key()),
            };
            // A decision from the beginning is the vendor's form, as every
            // line written before decisions were dated was.
            if d.effective.is_none() {
                conn.execute(
                    "UPDATE vendors SET form_1099 = ? WHERE id = ?",
                    params![key, vendor as i64],
                )?;
            }
            conn.execute(
                "UPDATE vendors SET tin_on_file = ? WHERE id = ?",
                params![tin_on_file, vendor as i64],
            )?;
            if let Some(s) = &d.source {
                let (kind, reason) = match &d.decision {
                    ledger_core::Decision1099::NotReportable { kind, reason } => {
                        (Some(kind.key()), Some(reason.as_str()))
                    }
                    _ => (None, None),
                };
                conn.execute(
                    "INSERT INTO vendor_1099_decisions(seq, vendor_id, effective, form_1099, reason_kind, reason, source_json) VALUES (?,?,?,?,?,?,?)",
                    params![
                        line.seq as i64,
                        vendor as i64,
                        d.effective.map(|e| e.to_string()),
                        key,
                        kind,
                        reason,
                        crate::meta::source_json(s).to_string(),
                    ],
                )?;
            }
        }
        "vendor.w9_flag_set" => {
            let (vendor, on_file) = crate::form1099::w9_flag_of(p)?;
            conn.execute(
                "UPDATE vendors SET tin_on_file = ? WHERE id = ?",
                params![on_file, vendor as i64],
            )?;
        }
        "instrument.added" => {
            let i = crate::form1099::instrument_of(line)?;
            conn.execute(
                "INSERT INTO instruments(id, kind, label, last4, owner_party, account_id) VALUES (?,?,?,?,?,?)",
                params![
                    i.id as i64,
                    i.kind.key(),
                    i.label,
                    i.last4,
                    i.owner_party.map(|p| p as i64),
                    i.account as i64,
                ],
            )?;
        }
        "entry.instrument_set" => {
            let (entry, instrument) = crate::form1099::entry_instrument_of(p)?;
            // The whole reversal chain, as the ledger tags it.
            for e in sqlite_reversal_chain(conn, entry as i64)? {
                match instrument {
                    Some(i) => conn.execute(
                        "INSERT OR REPLACE INTO entry_instruments(entry_id, instrument_id) VALUES (?, ?)",
                        params![e, i as i64],
                    )?,
                    None => conn.execute(
                        "DELETE FROM entry_instruments WHERE entry_id = ?",
                        params![e],
                    )?,
                };
            }
        }
        "entry.vendor_set" => match p.get("vendor_id").and_then(|v| v.as_u64()) {
            Some(v) => {
                conn.execute(
                    "INSERT OR REPLACE INTO entry_vendors(entry_id, vendor_id) VALUES (?, ?)",
                    params![get_u64(p, "entry_id")? as i64, v as i64],
                )?;
            }
            None => {
                conn.execute(
                    "DELETE FROM entry_vendors WHERE entry_id = ?",
                    params![get_u64(p, "entry_id")? as i64],
                )?;
            }
        },
        "account.renumbered" => {
            conn.execute(
                "UPDATE accounts SET code = ? WHERE id = ?",
                params![get_str(p, "code")?, get_u64(p, "account_id")? as i64],
            )?;
        }
        "account.renamed" => {
            conn.execute(
                "UPDATE accounts SET name = ? WHERE id = ?",
                params![get_str(p, "name")?, get_u64(p, "account_id")? as i64],
            )?;
        }
        "account.tax_set" => {
            conn.execute(
                "UPDATE accounts SET tax_category = ?, deductible_percent = ? WHERE id = ?",
                params![
                    p.get("tax_category").and_then(|v| v.as_str()),
                    p.get("deductible_percent").and_then(|v| v.as_str()),
                    get_u64(p, "account_id")? as i64,
                ],
            )?;
        }
        "schedule.registered" | "recurring.registered" | "recurring.generated" => {
            // Master data and drafts: the ledger holds them; no report rows.
        }
        "schedule.posted" => project_posted_entry(conn, &p["entry"])?,
        "receipt.unlinked" => {
            conn.execute(
                "UPDATE attachments SET entry_id = NULL WHERE id = ?",
                params![get_u64(p, "attachment_id")? as i64],
            )?;
        }
        "attachment.withdrawn" => {
            conn.execute(
                "UPDATE attachments SET withdrawn_at = ?, withdrawn_reason = ? WHERE id = ?",
                params![
                    line.ts,
                    get_str(p, "reason")?,
                    get_u64(p, "attachment_id")? as i64,
                ],
            )?;
        }
        "asset.registered" => {
            let cost = &p["cost"];
            let salvage = &p["salvage_value"];
            conn.execute(
                "INSERT INTO assets(id, description, purchase_date, cost_amount, cost_currency, salvage_amount, salvage_currency, useful_life_months, method, cost_account, accum_depr_account, expense_account, status) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?)",
                params![
                    get_u64(p, "id")? as i64,
                    get_str(p, "description")?,
                    get_str(p, "purchase_date")?,
                    get_str(cost, "amount")?,
                    get_str(cost, "currency")?,
                    get_str(salvage, "amount")?,
                    get_str(salvage, "currency")?,
                    get_u64(p, "useful_life_months")? as i64,
                    get_str(p, "method")?,
                    get_u64(p, "cost_account")? as i64,
                    get_u64(p, "accum_depr_account")? as i64,
                    get_u64(p, "expense_account")? as i64,
                    "active",
                ],
            )?;
        }
        "asset.depreciation_posted" => {
            // Every run, a period run again after its run was reversed
            // (feature month_reposting) beside the first: keyed by its
            // entry too (schema v13), as the ledger records them and the
            // projection's read-back restores them.
            conn.execute(
                "INSERT OR IGNORE INTO asset_depreciation(asset_id, period, entry_id, amount) VALUES (?,?,?,?)",
                params![
                    get_u64(p, "asset_id")? as i64,
                    get_str(p, "period")?,
                    get_u64(p, "entry_id")? as i64,
                    get_str(p, "amount")?,
                ],
            )?;
        }
        "asset.disposed" => {
            let proceeds = &p["proceeds"];
            conn.execute(
                "UPDATE assets SET status = 'disposed', disposed_date = ?, proceeds_amount = ? WHERE id = ?",
                params![
                    get_str(p, "date")?,
                    get_str(proceeds, "amount")?,
                    get_u64(p, "asset_id")? as i64,
                ],
            )?;
        }
        "period.created" => {
            conn.execute(
                "INSERT INTO periods(id, name, start, end, status, closing_entry) VALUES (?,?,?,?,?,?)",
                params![
                    get_u64(p, "id")? as i64,
                    get_str(p, "name")?,
                    get_str(p, "start")?,
                    get_str(p, "end")?,
                    "open",
                    Option::<i64>::None,
                ],
            )?;
        }
        "period.closed" => {
            // The closing entry rides in the payload (it is not separately
            // journaled as entry.created/entry.posted): record it like any
            // posted entry so the projection stays complete, then mark the
            // period row closed.
            if !p.get("entry").map(|v| v.is_null()).unwrap_or(true) {
                project_posted_entry(conn, &p["entry"])?;
            }
            conn.execute(
                "UPDATE periods SET status = 'closed', closing_entry = ? WHERE id = ?",
                params![
                    p.get("closing_entry")
                        .and_then(|v| v.as_u64())
                        .map(|v| v as i64),
                    get_u64(p, "id")? as i64,
                ],
            )?;
        }
        "line.cleared" => {
            let entry_id = get_u64(p, "entry_id")? as i64;
            let line_index = get_u64(p, "line_index")? as i64;
            if p.get("cleared").and_then(|v| v.as_bool()).unwrap_or(false) {
                conn.execute(
                    "INSERT OR IGNORE INTO cleared_lines(entry_id, line_index) VALUES (?, ?)",
                    params![entry_id, line_index],
                )?;
            } else {
                conn.execute(
                    "DELETE FROM cleared_lines WHERE entry_id = ? AND line_index = ?",
                    params![entry_id, line_index],
                )?;
            }
        }
        "customer.added" => {
            conn.execute(
                "INSERT INTO customers(id, name) VALUES (?, ?)",
                params![get_u64(p, "id")? as i64, get_str(p, "name")?],
            )?;
        }
        "vendor.added" => {
            conn.execute(
                "INSERT INTO vendors(id, name) VALUES (?, ?)",
                params![get_u64(p, "id")? as i64, get_str(p, "name")?],
            )?;
        }
        "invoice.created" => {
            let lines = p.get("lines").ok_or_else(|| {
                StoreError::JournalCorrupt("invoice.created without lines".into())
            })?;
            conn.execute(
                "INSERT INTO invoices(id, customer_id, date, due_date, lines_json, entry_id, number, terms, bill_to, seller, customer_ref, memo) VALUES (?,?,?,?,?,?,?,?,?,?,?,?)",
                params![
                    get_u64(p, "id")? as i64,
                    get_u64(p, "customer_id")? as i64,
                    get_str(p, "date")?,
                    get_str(p, "due_date")?,
                    lines.to_string(),
                    get_u64(p, "entry_id")? as i64,
                    opt_str(p, "number")?,
                    json_column(p, "terms"),
                    json_column(p, "bill_to"),
                    json_column(p, "seller"),
                    opt_str(p, "customer_ref")?,
                    opt_str(p, "memo")?,
                ],
            )?;
        }
        "bill.created" => {
            let lines = p
                .get("lines")
                .ok_or_else(|| StoreError::JournalCorrupt("bill.created without lines".into()))?;
            conn.execute(
                "INSERT INTO bills(id, vendor_id, date, due_date, lines_json, entry_id, vendor_ref, received_on, terms, memo, duplicate_reason) VALUES (?,?,?,?,?,?,?,?,?,?,?)",
                params![
                    get_u64(p, "id")? as i64,
                    get_u64(p, "vendor_id")? as i64,
                    get_str(p, "date")?,
                    get_str(p, "due_date")?,
                    lines.to_string(),
                    get_u64(p, "entry_id")? as i64,
                    opt_str(p, "vendor_ref")?,
                    opt_str(p, "received_on")?,
                    json_column(p, "terms"),
                    opt_str(p, "memo")?,
                    opt_str(p, "duplicate_reason")?,
                ],
            )?;
            conn.execute(
                "INSERT OR REPLACE INTO entry_vendors(entry_id, vendor_id) VALUES (?, ?)",
                params![
                    get_u64(p, "entry_id")? as i64,
                    get_u64(p, "vendor_id")? as i64
                ],
            )?;
        }
        // A payment moves no document row: what a document is paid is read
        // from the ledger lines tagged with it (schema v15, rules.md B-38).
        "invoice.payment_received" => {}
        "bill.paid" => {
            conn.execute(
                "INSERT OR REPLACE INTO entry_vendors(entry_id, vendor_id) \
                 SELECT ?, vendor_id FROM bills WHERE id = ?",
                params![
                    get_u64(p, "entry_id")? as i64,
                    get_u64(p, "bill_id")? as i64
                ],
            )?;
        }
        "fx.rate_set" => {
            conn.execute(
                "INSERT INTO fx_rates(currency, date, rate) VALUES (?,?,?) \
                 ON CONFLICT(currency, date) DO UPDATE SET rate=excluded.rate",
                params![
                    get_str(p, "currency")?,
                    get_str(p, "date")?,
                    get_str(p, "rate")?,
                ],
            )?;
        }
        "payroll.run" => {
            // The entry itself arrived through the standard entry events
            // journaled just before this line; this records the run.
            let run_id = get_u64(p, "id")? as i64;
            conn.execute(
                "INSERT INTO payroll_runs(id, date, entry_id) VALUES (?,?,?)",
                params![run_id, get_str(p, "date")?, get_u64(p, "entry_id")? as i64,],
            )?;
            for l in parse_payroll_lines(p)? {
                conn.execute(
                    "INSERT INTO payroll_lines(run_id, employee_name, gross_pay, federal_withholding, state_withholding, ss_employee, medicare_employee, ss_employer, medicare_employer, other_deductions, employee_id) VALUES (?,?,?,?,?,?,?,?,?,?,?)",
                    params![
                        run_id,
                        l.employee_name,
                        l.gross_pay.to_string(),
                        l.federal_withholding.to_string(),
                        l.state_withholding.to_string(),
                        l.ss_employee.to_string(),
                        l.medicare_employee.to_string(),
                        l.ss_employer.to_string(),
                        l.medicare_employer.to_string(),
                        l.other_deductions.to_string(),
                        l.employee.map(|e| e as i64),
                    ],
                )?;
            }
        }
        "class.added" => {
            conn.execute(
                "INSERT INTO classes(id, name) VALUES (?,?)",
                params![get_u64(p, "id")? as i64, get_str(p, "name")?],
            )?;
        }
        "project.set" => {
            // Facts live in the ledger (as parties'); the projection keeps
            // the name, so verify's class check stays exact after a rename.
            conn.execute(
                "UPDATE classes SET name = ? WHERE id = ?",
                params![get_str(p, "name")?, get_u64(p, "id")? as i64],
            )?;
        }
        "entry.line_class_set" => {
            // Journaled while the entry is a draft, so the projection
            // usually has no lines row yet (drafts never touch the
            // projection): the UPDATE is a no-op then, and the class lands
            // through the entry.posted payload's per-line class_id. When a
            // row does exist, the UPDATE applies directly. Rows are
            // inserted in line order, so OFFSET maps line_index to the row.
            let class_id: Option<i64> =
                p.get("class_id").and_then(|v| v.as_u64()).map(|v| v as i64);
            conn.execute(
                "UPDATE lines SET class_id = ? WHERE id = (SELECT id FROM lines WHERE entry_id = ? ORDER BY id LIMIT 1 OFFSET ?)",
                params![
                    class_id,
                    get_u64(p, "entry_id")? as i64,
                    get_u64(p, "line_index")? as i64,
                ],
            )?;
        }
        "settings.set" => {
            let (changes, source) = crate::meta::settings_set_of(p)?;
            let source = crate::meta::source_json(&source).to_string();
            for c in &changes {
                match &c.value {
                    Some(v) => conn.execute(
                        "INSERT INTO settings(key, value_json, seq, source_json) VALUES (?,?,?,?) ON CONFLICT(key) DO UPDATE SET value_json=excluded.value_json, seq=excluded.seq, source_json=excluded.source_json",
                        params![c.key, v.to_string(), line.seq as i64, source],
                    )?,
                    None => conn.execute("DELETE FROM settings WHERE key = ?", params![c.key])?,
                };
            }
        }
        "convention.set" => {
            let c = crate::meta::convention_set_of(p)?;
            conn.execute(
                "INSERT INTO conventions(id, kind, title, body_md, structured_json, source_json, seq, revision) VALUES (?,?,?,?,?,?,?,1) ON CONFLICT(id) DO UPDATE SET kind=excluded.kind, title=excluded.title, body_md=excluded.body_md, structured_json=excluded.structured_json, source_json=excluded.source_json, seq=excluded.seq, revision=conventions.revision + 1",
                params![
                    c.id,
                    c.kind.key(),
                    c.title,
                    c.body_md,
                    c.structured.as_ref().map(Value::to_string),
                    crate::meta::source_json(&c.source).to_string(),
                    line.seq as i64,
                ],
            )?;
        }
        "convention.retired" => {
            let (id, reason, source) = crate::meta::convention_retired_of(p)?;
            conn.execute(
                "UPDATE conventions SET retired_reason = ?, retired_source_json = ?, retired_seq = ? WHERE id = ?",
                params![
                    reason,
                    crate::meta::source_json(&source).to_string(),
                    line.seq as i64,
                    id
                ],
            )?;
        }
        "decision.recorded" => {
            let (id, d) = crate::meta::decision_recorded_of(p)?;
            conn.execute(
                "INSERT INTO decisions(id, subject, decision_md, decided_by_json, decided_on, evidence_attachment, supersedes, citation, seq) VALUES (?,?,?,?,?,?,?,?,?)",
                params![
                    id as i64,
                    d.subject,
                    d.decision_md,
                    serde_json::to_string(&d.decided_by)?,
                    d.decided_on.to_string(),
                    d.evidence_attachment.map(|a| a as i64),
                    d.supersedes.map(|a| a as i64),
                    d.citation,
                    line.seq as i64,
                ],
            )?;
            if let Some(old) = d.supersedes {
                conn.execute(
                    "UPDATE decisions SET superseded_by = ? WHERE id = ?",
                    params![id as i64, old as i64],
                )?;
            }
        }
        "open_item.raised" => {
            let (id, i) = crate::meta::open_item_raised_of(p)?;
            conn.execute(
                "INSERT INTO open_items(id, title, detail_md, due, source_json, seq) VALUES (?,?,?,?,?,?)",
                params![
                    id as i64,
                    i.title,
                    i.detail_md,
                    i.due.map(|d| d.to_string()),
                    crate::meta::source_json(&i.source).to_string(),
                    line.seq as i64,
                ],
            )?;
        }
        "open_item.resolved" => {
            let (id, resolution, source) = crate::meta::open_item_resolved_of(p)?;
            conn.execute(
                "UPDATE open_items SET resolution = ?, resolved_source_json = ?, resolved_seq = ? WHERE id = ?",
                params![
                    resolution,
                    crate::meta::source_json(&source).to_string(),
                    line.seq as i64,
                    id as i64
                ],
            )?;
        }
        "entry.approved" => {
            let (entry, a) = crate::meta::entry_approved_of(p, line.seq)?;
            conn.execute(
                "INSERT INTO entry_approvals(seq, entry_id, approver, fingerprint, note, source_json) VALUES (?,?,?,?,?,?)",
                params![
                    line.seq as i64,
                    entry as i64,
                    a.approver,
                    a.fingerprint,
                    a.note,
                    crate::meta::source_json(&a.source).to_string(),
                ],
            )?;
        }
        _ => return Err(crate::format::unknown_type(line)),
    }
    conn.execute(
        "INSERT INTO events(type, payload, created_at) VALUES (?,?,?)",
        params![line.line_type, p.to_string(), line.ts],
    )?;
    conn.execute(
        "INSERT INTO meta(key, value) VALUES ('last_applied_seq', ?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![line.seq.to_string()],
    )?;
    Ok(())
}

/// An entry's reversal chain in the projection, as
/// `Ledger::reversal_chain` walks it: up to the original, then down
/// through each posted reversal. A draft (not projected) is its own chain.
fn sqlite_reversal_chain(conn: &Connection, entry: i64) -> Result<Vec<i64>, StoreError> {
    let up = |e: i64| -> Result<Option<i64>, StoreError> {
        Ok(conn
            .query_row(
                "SELECT reversal_of FROM entries WHERE id = ?",
                params![e],
                |r| r.get::<_, Option<i64>>(0),
            )
            .optional()?
            .flatten())
    };
    let mut top = entry;
    while let Some(o) = up(top)? {
        if o == entry {
            break;
        }
        top = o;
    }
    let mut chain = vec![top];
    let mut cur = top;
    while let Some(r) = conn
        .query_row(
            "SELECT id FROM entries WHERE reversal_of = ? ORDER BY id LIMIT 1",
            params![cur],
            |r| r.get::<_, i64>(0),
        )
        .optional()?
    {
        if chain.contains(&r) {
            break;
        }
        chain.push(r);
        cur = r;
    }
    Ok(chain)
}

/// What [`Store::verify_and_heal`] found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifyReport {
    /// Journal events verified.
    pub events: u64,
    /// Hash of the last journal line: the verified head of the chain.
    pub head: String,
    /// Why the projection was rebuilt, if it was.
    pub healed: Option<String>,
    /// The projection database's file verified: this version's
    /// ([`Store::database_file`]).
    pub database: String,
    /// Other schemas' projection files beside it, left as they are
    /// ([`Store::other_databases`]).
    pub other_databases: Vec<String>,
}

/// A path's last part, as text.
fn file_name_of(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// A projection of `lines` into a private in-memory database, built with
/// exactly the code the writer uses.
pub(crate) fn fresh_projection(lines: &[JournalLine]) -> Result<Connection, StoreError> {
    let conn = Connection::open_in_memory()?;
    Store::migrate(&conn)?;
    for line in lines {
        apply_line_to_sqlite(&conn, line)?;
    }
    Ok(conn)
}

/// Every row of every projected table, as canonical strings, sorted: two
/// projections hold the same data exactly when these are equal. The `meta`
/// table (bookkeeping about the projection itself) is left out.
pub(crate) fn projection_rows(
    conn: &Connection,
) -> Result<std::collections::BTreeMap<String, Vec<String>>, StoreError> {
    use rusqlite::types::ValueRef;
    let tables: Vec<String> = conn
        .prepare(
            "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' AND name <> 'meta' ORDER BY name",
        )?
        .query_map([], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    let mut out = std::collections::BTreeMap::new();
    for t in tables {
        let mut stmt = conn.prepare(&format!("SELECT * FROM \"{t}\""))?;
        let n = stmt.column_count();
        let mut rows: Vec<String> = stmt
            .query_map([], |r| {
                let mut s = String::new();
                for i in 0..n {
                    let v = match r.get_ref(i)? {
                        ValueRef::Null => "∅".to_string(),
                        ValueRef::Integer(x) => format!("i{x}"),
                        ValueRef::Real(x) => format!("r{x}"),
                        ValueRef::Text(b) => format!("t{}", String::from_utf8_lossy(b)),
                        ValueRef::Blob(b) => format!("b{}", sha256_hex(b)),
                    };
                    s.push_str(&v);
                    s.push('\u{1f}');
                }
                Ok(s)
            })?
            .collect::<Result<_, _>>()?;
        rows.sort();
        out.insert(t, rows);
    }
    Ok(out)
}

/// Compare two projections; name the first table that differs.
pub(crate) fn compare_projections(
    have: &std::collections::BTreeMap<String, Vec<String>>,
    want: &std::collections::BTreeMap<String, Vec<String>>,
) -> Result<(), StoreError> {
    for (table, rows) in want {
        match have.get(table) {
            None => {
                return Err(StoreError::VerifyMismatch(format!(
                    "sqlite projection is missing table {table}"
                )));
            }
            Some(h) if h != rows => {
                let missing = rows.iter().filter(|r| !h.contains(r)).count();
                let extra = h.iter().filter(|r| !rows.contains(r)).count();
                return Err(StoreError::VerifyMismatch(format!(
                    "sqlite projection differs from the journal in table {table}: {missing} rows missing, {extra} unexpected"
                )));
            }
            _ => {}
        }
    }
    Ok(())
}

/// What an attachment can be linked to (docs/parties-and-attachments.md).
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum Subject {
    /// An entry (beyond the one a receipt is filed on).
    Entry(u64),
    /// An account (e.g. its statements, the agreement that opened it).
    Account(u64),
    /// A fixed asset (photos, warranty, manuals).
    Asset(u64),
    /// A party (contracts, correspondence).
    Party(u64),
    /// An invoice.
    Invoice(u64),
    /// A bill.
    Bill(u64),
    /// An accounting period (its close package).
    Period(u64),
    /// A payroll run.
    PayrollRun(u64),
    /// An amortization schedule (the prepaid contract).
    Schedule(u64),
    /// A reconciliation (its statement).
    Reconciliation(u64),
    /// A project (its contract, spec, pitch, correspondence; rules.md
    /// B-37). Journal feature `projects`.
    Project(u64),
    /// The company itself (formation documents).
    Company,
}

impl Subject {
    /// Its kind's wire name.
    pub fn kind(self) -> &'static str {
        match self {
            Subject::Entry(_) => "entry",
            Subject::Account(_) => "account",
            Subject::Asset(_) => "asset",
            Subject::Party(_) => "party",
            Subject::Invoice(_) => "invoice",
            Subject::Bill(_) => "bill",
            Subject::Period(_) => "period",
            Subject::PayrollRun(_) => "payroll_run",
            Subject::Schedule(_) => "schedule",
            Subject::Reconciliation(_) => "reconciliation",
            Subject::Project(_) => "project",
            Subject::Company => "company",
        }
    }

    /// Its id (0 for the company).
    pub fn id(self) -> u64 {
        match self {
            Subject::Entry(i)
            | Subject::Account(i)
            | Subject::Asset(i)
            | Subject::Party(i)
            | Subject::Invoice(i)
            | Subject::Bill(i)
            | Subject::Period(i)
            | Subject::PayrollRun(i)
            | Subject::Schedule(i)
            | Subject::Reconciliation(i)
            | Subject::Project(i) => i,
            Subject::Company => 0,
        }
    }

    /// The subject of `kind` and `id`.
    pub fn from_parts(kind: &str, id: u64) -> Option<Subject> {
        Some(match kind {
            "entry" => Subject::Entry(id),
            "account" => Subject::Account(id),
            "asset" => Subject::Asset(id),
            "party" => Subject::Party(id),
            "invoice" => Subject::Invoice(id),
            "bill" => Subject::Bill(id),
            "period" => Subject::Period(id),
            "payroll_run" => Subject::PayrollRun(id),
            "schedule" => Subject::Schedule(id),
            "reconciliation" => Subject::Reconciliation(id),
            "project" => Subject::Project(id),
            "company" => Subject::Company,
            _ => return None,
        })
    }

    /// Human words, e.g. "account 1", for messages.
    pub fn describe(self) -> String {
        match self {
            Subject::Company => "the company".into(),
            s => format!("{} {}", s.kind().replace('_', " "), s.id()),
        }
    }
}

/// A bank or card statement: the attachment that holds it, and what it
/// says.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Statement {
    /// The attachment holding the statement.
    pub attachment_id: u64,
    /// The account it is for.
    pub account_id: AccountId,
    /// First day it covers.
    pub period_start: NaiveDate,
    /// Last day it covers (its statement date).
    pub period_end: NaiveDate,
    /// Its opening balance, in the account's normal direction.
    pub opening_balance: Decimal,
    /// Its closing balance, in the account's normal direction.
    pub closing_balance: Decimal,
}

/// A completed reconciliation.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Reconciliation {
    /// Its id.
    pub id: u64,
    /// The account reconciled.
    pub account_id: AccountId,
    /// The statement date it reconciles through.
    pub statement_date: NaiveDate,
    /// The statement's closing balance.
    pub statement_balance: Decimal,
    /// The statement's attachment.
    pub attachment_id: u64,
    /// The lines it found cleared, (entry, line index).
    pub cleared: Vec<(EntryId, usize)>,
    /// Why it was voided, if it was.
    pub voided: Option<String>,
}

/// Recorded statements in the projection, optionally one account's, by
/// period end (shared by [`Store::statements`] and the read-only
/// snapshot): those not voided, leaving out any whose attachment is
/// withdrawn.
pub(crate) fn query_statements(
    conn: &Connection,
    account: Option<AccountId>,
) -> Result<Vec<Statement>, StoreError> {
    let mut stmt = conn.prepare(
            "SELECT attachment_id, account_id, period_start, period_end, opening_balance, closing_balance FROM statements WHERE attachment_id NOT IN (SELECT id FROM attachments WHERE withdrawn_at IS NOT NULL) ORDER BY period_end, attachment_id",
        )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, String>(5)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (att, acct, start, end, open, close) = row?;
        if account.is_some_and(|a| a != acct as u64) {
            continue;
        }
        let dec = |t: &str| {
            t.parse::<Decimal>()
                .map_err(|_| StoreError::JournalCorrupt(format!("statement amount {t}")))
        };
        out.push(Statement {
            attachment_id: att as u64,
            account_id: acct as u64,
            period_start: parse_date(&start)?,
            period_end: parse_date(&end)?,
            opening_balance: dec(&open)?,
            closing_balance: dec(&close)?,
        });
    }
    Ok(out)
}

/// Reconciliations in the projection, optionally one account's, by date
/// (shared by [`Store::reconciliations`] and the read-only snapshot).
pub(crate) fn query_reconciliations(
    conn: &Connection,
    account: Option<AccountId>,
) -> Result<Vec<Reconciliation>, StoreError> {
    let mut stmt = conn.prepare(
            "SELECT id, account_id, statement_date, statement_balance, attachment_id, voided_reason FROM reconciliations ORDER BY statement_date, id",
        )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, i64>(4)?,
            r.get::<_, Option<String>>(5)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (id, acct, date, bal, att, voided) = row?;
        if account.is_some_and(|a| a != acct as u64) {
            continue;
        }
        let cleared: Vec<(EntryId, usize)> = conn
                .prepare(
                    "SELECT entry_id, line_index FROM reconciliation_lines WHERE reconciliation_id = ? ORDER BY entry_id, line_index",
                )?
                .query_map(params![id], |r| {
                    Ok((r.get::<_, i64>(0)? as u64, r.get::<_, i64>(1)? as usize))
                })?
                .collect::<Result<_, _>>()?;
        out.push(Reconciliation {
            id: id as u64,
            account_id: acct as u64,
            statement_date: parse_date(&date)?,
            statement_balance: bal
                .parse()
                .map_err(|_| StoreError::JournalCorrupt(format!("reconciliation balance {bal}")))?,
            attachment_id: att as u64,
            cleared,
            voided,
        });
    }
    Ok(out)
}

/// Every row of `attachment_links`: (attachment, subject), by attachment
/// then subject. A receipt's filing entry isn't a row here (it is the
/// attachment's `entry_id`).
pub(crate) fn query_attachment_links(conn: &Connection) -> Result<Vec<(u64, Subject)>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT attachment_id, subject_kind, subject_id FROM attachment_links ORDER BY attachment_id, subject_kind, subject_id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (att, kind, id) = row?;
        if let Some(sub) = Subject::from_parts(&kind, id as u64) {
            out.push((att as u64, sub));
        }
    }
    Ok(out)
}

/// Attachments in the projection, optionally only one entry's, by id.
pub(crate) fn query_attachments(
    conn: &Connection,
    entry_id: Option<EntryId>,
) -> Result<Vec<Attachment>, StoreError> {
    let mut sql = String::from(
        "SELECT a.id, a.blob_sha, a.entry_id, a.category, \
         COALESCE(a.file_name, b.original_name), COALESCE(a.mime_type, b.mime_type), \
         b.size_bytes, a.description, a.created_at, a.withdrawn_at, a.withdrawn_reason \
         FROM attachments a \
         JOIN blobs b ON b.sha256 = a.blob_sha",
    );
    if entry_id.is_some() {
        // Filed on the entry, or linked to it.
        sql.push_str(
            " WHERE a.entry_id = ?1 OR a.id IN \
             (SELECT attachment_id FROM attachment_links WHERE subject_kind = 'entry' AND subject_id = ?1)",
        );
    }
    sql.push_str(" ORDER BY a.id");
    let mut stmt = conn.prepare(&sql)?;
    let rows: Vec<Attachment> = match entry_id {
        Some(eid) => stmt
            .query_map(params![eid as i64], row_to_attachment)?
            .collect::<Result<_, _>>()?,
        None => stmt
            .query_map([], row_to_attachment)?
            .collect::<Result<_, _>>()?,
    };
    Ok(rows)
}

/// An attachment's bytes, verified against its SHA-256.
pub(crate) fn read_attachment_bytes(
    conn: &Connection,
    blobs: &BlobStore,
    attachment_id: u64,
) -> Result<Vec<u8>, StoreError> {
    let sha: String = conn.query_row(
        "SELECT blob_sha FROM attachments WHERE id = ?",
        params![attachment_id as i64],
        |r| r.get(0),
    )?;
    blobs.get(&sha)
}

fn row_to_attachment(r: &rusqlite::Row) -> rusqlite::Result<Attachment> {
    Ok(Attachment {
        id: r.get::<_, i64>(0)? as u64,
        blob_sha: r.get(1)?,
        entry_id: r.get::<_, Option<i64>>(2)?.map(|v| v as u64),
        category: r.get(3)?,
        file_name: r.get(4)?,
        mime_type: r.get(5)?,
        size_bytes: r.get::<_, i64>(6)? as u64,
        description: r.get(7)?,
        created_at: r.get(8)?,
        withdrawn_at: r.get(9)?,
        withdrawn_reason: r.get(10)?,
    })
}

/// The base currency of a journal: its first line's, which must be a
/// sound `store.created` ([`created_currency`]).
pub(crate) fn journal_currency(lines: &[JournalLine]) -> Result<Currency, StoreError> {
    let first = lines
        .first()
        .ok_or_else(|| StoreError::JournalCorrupt("journal has no store.created line".into()))?;
    created_currency(first)
}

/// The base currency the journal's first line sets: it must be a
/// `store.created` line naming a currency, or the journal is corrupt. The
/// one rule for the writer ([`Store::open`]) and the viewer
/// ([`crate::Snapshot`]), so the two never read different books.
pub(crate) fn created_currency(first: &JournalLine) -> Result<Currency, StoreError> {
    if first.line_type != "store.created" {
        return Err(StoreError::JournalCorrupt(
            "journal's first line is not store.created".into(),
        ));
    }
    let code = first.payload["base_currency"]
        .as_str()
        .ok_or_else(|| StoreError::JournalCorrupt("store.created without base_currency".into()))?;
    Currency::new(code).map_err(|_| StoreError::JournalCorrupt(format!("bad currency {code}")))
}
