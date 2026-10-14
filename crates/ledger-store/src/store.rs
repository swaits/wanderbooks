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
use crate::journal::{
    GENESIS_PREV, JournalLine, JournalWriter, read_and_verify_journal, sha256_hex,
};
use crate::schema;
use chrono::{Months, NaiveDate};
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
/// File names inside the data directory.
pub const DB_FILE: &str = "wanderbooks.db";

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

/// The outcome of posting one month of depreciation for one asset.
#[derive(Clone, Debug)]
pub struct DepreciationRun {
    /// Asset id.
    pub asset_id: u64,
    /// Calendar month, "YYYY-MM".
    pub period: String,
    /// Amount posted, exact cents.
    pub amount: Decimal,
    /// The posted entry's id.
    pub entry_id: u64,
    /// True when this period was already posted (idempotent no-op).
    pub already_posted: bool,
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
    /// Accumulated depreciation that will be cleared: already posted plus
    /// the final partial period.
    pub accumulated: Decimal,
    /// Depreciation auto-posted for the final partial period. Zero when the
    /// disposal month was already posted.
    pub final_depreciation: Decimal,
    /// Book value at disposal.
    pub book_value: Decimal,
    /// proceeds − book_value. Positive is a gain.
    pub gain_loss: Decimal,
    /// The disposal month, "YYYY-MM".
    pub period: String,
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
    /// GOAL: start empty books in a data directory.
    /// SOURCE: docs/receipt-store.md "Line format" (genesis prev); R-11.
    /// INVARIANTS:
    ///   1. The journal's first line MUST be `store.created` with the base
    ///      currency, chained from the genesis hash (64 zeros).
    ///   2. The writer lock MUST be held from creation on.
    /// SCENARIOS:
    ///   1. IF a journal already exists there: SHOULD refuse
    ///      - NEVER overwrite or append to existing books.
    /// ```
    pub fn create(data_dir: &Path, base_currency: Currency) -> Result<Self, StoreError> {
        let journal_path = data_dir.join(JOURNAL_FILE);
        if journal_path.exists() {
            return Err(StoreError::AlreadyExists(data_dir.display().to_string()));
        }
        std::fs::create_dir_all(data_dir)?;
        let lock = lock_books(data_dir)?;
        let db_path = data_dir.join(DB_FILE);
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
        };
        let line = store.journal.append(
            "store.created",
            json!({ "base_currency": base_currency.code() }),
        )?;
        apply_line_to_sqlite(&store.conn, &line)?;
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
    ///   3. IF the last line is cut off (no newline): SHOULD set it aside,
    ///      truncate, and journal `journal.tail_repaired`.
    ///   4. IF a complete line fails its hash, parse, or `seq`: SHOULD refuse
    ///      - NEVER repair or skip it (that is corruption, for a human).
    ///   5. IF the database is missing: SHOULD rebuild it from the journal.
    ///   6. IF the database fails SQLite's integrity check: SHOULD keep it
    ///      aside for forensics and rebuild from the journal.
    ///   7. IF the database is behind the journal: SHOULD apply the rest.
    /// ```
    pub fn open(data_dir: &Path) -> Result<Self, StoreError> {
        let journal_path = data_dir.join(JOURNAL_FILE);
        if !journal_path.exists() {
            return Err(StoreError::NotFound(data_dir.display().to_string()));
        }
        // The writer lock first: nothing is read until no one else writes.
        let lock = lock_books(data_dir)?;
        // An append cut off mid-line was never acknowledged: set it aside.
        let torn = crate::journal::repair_torn_tail(&journal_path)?;
        // Single verified scan; the writer reuses it.
        let lines = read_and_verify_journal(&journal_path)?;
        let journal = JournalWriter::from_lines(&journal_path, &lines)?;
        let db_path = data_dir.join(DB_FILE);
        let mut fresh_db = !db_path.exists();
        if !fresh_db && !Self::db_file_healthy(&db_path) {
            // The projection is corrupt. The journal is the source of truth,
            // so quarantine the damaged file (for forensics) and rebuild.
            let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
            let quarantine = db_path.with_extension(format!("corrupt.{stamp}"));
            std::fs::rename(&db_path, &quarantine).map_err(StoreError::Io)?;
            // WAL/SHM are derived state; drop them so they can't contaminate
            // the rebuild.
            let _ = std::fs::remove_file(db_path.with_extension("db-wal"));
            let _ = std::fs::remove_file(db_path.with_extension("db-shm"));
            eprintln!(
                "wanderbooks: sqlite projection failed integrity_check; quarantined to {} and rebuilding from journal",
                quarantine.display()
            );
            fresh_db = true;
        }
        let conn = Self::open_conn(&db_path)?;
        Self::migrate(&conn)?;
        let base_currency = if fresh_db {
            // Heal from the journal: currency comes from store.created.
            let first = lines.first().ok_or_else(|| {
                StoreError::JournalCorrupt("journal has no store.created line".into())
            })?;
            if first.line_type != "store.created" {
                return Err(StoreError::JournalCorrupt(
                    "journal's first line is not store.created".into(),
                ));
            }
            let code = first.payload["base_currency"].as_str().ok_or_else(|| {
                StoreError::JournalCorrupt("store.created without base_currency".into())
            })?;
            let cur = Currency::new(code)
                .map_err(|_| StoreError::JournalCorrupt(format!("bad currency {code}")))?;
            Self::write_meta(&conn, "base_currency", cur.code())?;
            Self::write_meta(&conn, "last_applied_seq", "0")?;
            cur
        } else {
            let cur = Self::read_currency(&conn)?;
            // Cross-check the projection against the journal.
            if let Some(first) = lines.first()
                && first.line_type == "store.created"
            {
                let jc = first.payload["base_currency"].as_str().unwrap_or("");
                if jc != cur.code() {
                    return Err(StoreError::JournalCorrupt(format!(
                        "base currency mismatch: journal says {jc}, sqlite says {}",
                        cur.code()
                    )));
                }
            }
            cur
        };
        let mut store = Self {
            _lock: lock,
            data_dir: data_dir.to_path_buf(),
            db_path,
            journal,
            conn,
            blobs: BlobStore::open(&data_dir.join(BLOBS_DIR))?,
            ledger: Ledger::new(base_currency),
            base_currency,
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
                "wanderbooks: the journal's last line was cut off (an append that never finished); set aside {} bytes in {}",
                t.bytes,
                t.saved_as.display()
            );
        }
        Ok(store)
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

    fn read_currency(conn: &Connection) -> Result<Currency, StoreError> {
        let code: String = conn.query_row(
            "SELECT value FROM meta WHERE key='base_currency'",
            [],
            |r| r.get(0),
        )?;
        Currency::new(&code)
            .map_err(|_| StoreError::JournalCorrupt(format!("bad base_currency in meta: {code}")))
    }

    /// Replay the journal: hydrate the in-memory ledger fully, and apply to
    /// SQLite every line past `last_applied_seq`.
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
        for line in lines {
            replay_line_into_ledger(&mut self.ledger, line, self.base_currency)?;
            if line.seq > last_applied {
                apply_line_to_sqlite(&self.conn, line)?;
            }
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

    /// Add an account to the chart.
    pub fn add_account(
        &mut self,
        code: &str,
        name: &str,
        account_type: AccountType,
        parent: Option<AccountId>,
        placeholder: bool,
    ) -> Result<AccountId, StoreError> {
        self.rehearse(|l| l.add_account(code, name, account_type, parent, placeholder))?;
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
        let live = self
            .ledger
            .add_account(code, name, account_type, parent, placeholder)?;
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

    /// Validate, journal, and post a draft entry.
    pub fn post_entry(&mut self, entry: EntryId) -> Result<(), StoreError> {
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

    /// Post a reversing entry for a posted entry.
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
        self.ledger.check_reversal(entry, date)?;
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
                "purchase_date": new.purchase_date.to_string(),
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

    /// Was depreciation already posted for this asset and period?
    fn depreciation_posted_entry(
        &self,
        asset_id: AssetId,
        period: &str,
    ) -> Result<Option<(u64, Decimal)>, StoreError> {
        let row: Option<(i64, String)> = self
            .conn
            .query_row(
                "SELECT entry_id, amount FROM asset_depreciation WHERE asset_id = ? AND period = ?",
                params![asset_id as i64, period],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        row.map(|(eid, amount)| {
            let amount = amount
                .parse()
                .map_err(|_| StoreError::JournalCorrupt(format!("bad amount {amount}")))?;
            Ok((eid as u64, amount))
        })
        .transpose()
    }

    /// Sum of depreciation amounts posted for an asset. Summed in Rust with
    /// exact Decimal math: amounts are TEXT in SQLite, so SQLite must not
    /// sum them (REAL would lose pennies).
    fn asset_depreciation_total(&self, asset_id: AssetId) -> Result<Decimal, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT amount FROM asset_depreciation WHERE asset_id = ?")?;
        let amounts = stmt.query_map(params![asset_id as i64], |r| r.get::<_, String>(0))?;
        let mut total = Decimal::ZERO;
        for a in amounts {
            let s: String = a?;
            total += s
                .parse::<Decimal>()
                .map_err(|_| StoreError::JournalCorrupt(format!("bad amount {s}")))?;
        }
        Ok(total)
    }

    /// Post one month of depreciation for one asset. Idempotent: running it
    /// twice for the same period returns the original entry.
    ///
    /// Depreciation is an allocation, not money movement, so it is exempt
    /// from the receipt policy by construction: the registered asset and its
    /// schedule are the supporting documentation.
    pub fn run_depreciation(
        &mut self,
        asset_id: AssetId,
        period: &str,
    ) -> Result<DepreciationRun, StoreError> {
        if let Some((entry_id, amount)) = self.depreciation_posted_entry(asset_id, period)? {
            return Ok(DepreciationRun {
                asset_id,
                period: period.to_string(),
                amount,
                entry_id,
                already_posted: true,
            });
        }
        let asset = self
            .ledger
            .get_asset(asset_id)
            .ok_or(LedgerError::UnknownAsset(asset_id))?
            .clone();
        if asset.status != AssetStatus::Active {
            return Err(LedgerError::AssetAlreadyDisposed(asset_id).into());
        }
        let (year, month) = parse_period(period)?;
        let period_end = last_day_of_month(year, month);
        let sched = ledger_core::depreciation_schedule(&asset, period_end);
        let line = sched.iter().find(|l| l.period == period).ok_or_else(|| {
            StoreError::JournalCorrupt(format!(
                "period {period} is outside asset {asset_id}'s depreciable life"
            ))
        })?;
        let amount = line.amount;
        debug_assert!(amount > Decimal::ZERO);

        let entry_id = self.create_entry(
            period_end,
            &format!("Depreciation — {} — {period}", asset.description),
        )?;
        self.add_line(
            entry_id,
            asset.expense_account,
            Side::Debit,
            Money::new(amount, self.base_currency),
        )?;
        self.add_line(
            entry_id,
            asset.accum_depr_account,
            Side::Credit,
            Money::new(amount, self.base_currency),
        )?;
        self.post_entry(entry_id)?;
        let dline = self.append_journal(
            "asset.depreciation_posted",
            json!({
                "asset_id": asset_id,
                "period": period,
                "entry_id": entry_id,
                "amount": amount.to_string(),
            }),
        )?;
        apply_line_to_sqlite(&self.conn, &dline)?;
        Ok(DepreciationRun {
            asset_id,
            period: period.to_string(),
            amount,
            entry_id,
            already_posted: false,
        })
    }

    /// The disposal entry's lines for an asset, its preview, and the
    /// accounts chosen: the one place the shape of a disposal is decided,
    /// for both the preview and the post. Zero lines are left out (an
    /// engine entry has none): scrapping brings no proceeds, and an asset
    /// disposed before its first depreciation has none accumulated.
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
            (asset.cost_account, Side::Credit, asset.cost.amount),
            (gain_loss_account, Side::Credit, preview.gain_loss),
            (gain_loss_account, Side::Debit, -preview.gain_loss),
        ]
        .into_iter()
        .filter(|(_, _, a)| *a > Decimal::ZERO)
        .collect()
    }

    /// Compute a disposal without posting. Pure reads plus the posted-vs-
    /// scheduled reconciliation that [`Store::dispose_asset`] performs.
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
        if proceeds.currency != self.base_currency {
            return Err(LedgerError::CurrencyMismatch {
                expected: self.base_currency.code().to_string(),
                found: proceeds.currency.code().to_string(),
            }
            .into());
        }
        if date < asset.purchase_date {
            return Err(LedgerError::DisposalBeforePurchase(date.to_string()).into());
        }
        let period = date.format("%Y-%m").to_string();
        let posted = self.asset_depreciation_total(asset_id)?;
        let target = ledger_core::accumulated_depreciation(asset, date);
        let already = self.depreciation_posted_entry(asset_id, &period)?.is_some();
        let final_depreciation = if already {
            Decimal::ZERO
        } else {
            (target - posted).max(Decimal::ZERO)
        };
        let accumulated = posted + final_depreciation;
        let book_value = asset.cost.amount - accumulated;
        Ok(DisposalPreview {
            asset_id,
            date,
            proceeds,
            accumulated,
            final_depreciation,
            book_value,
            gain_loss: proceeds.amount - book_value,
            period,
        })
    }

    /// Dispose of an asset: auto-post any unposted depreciation through the
    /// disposal date, then post the disposal entry (clear cost and
    /// accumulated depreciation, record proceeds and gain/loss) and mark
    /// the asset disposed. Returns the disposal entry's id.
    ///
    /// The auto-post matters: the disposal entry must clear exactly the
    /// accumulated depreciation the books actually carry, not the schedule's
    /// theoretical number. If the agent already ran depreciation for the
    /// disposal month, that posted amount is the books' truth.
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
        if date < asset.purchase_date {
            return Err(LedgerError::DisposalBeforePurchase(date.to_string()).into());
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
        // so a refusal anywhere leaves nothing journaled.
        self.rehearse(|l| {
            for (description, lines) in depreciation.iter().chain([&disposal]) {
                let e = l.try_create_entry(date, description)?;
                for (account, side, amount) in lines {
                    l.add_line(e, *account, *side, Money::new(*amount, base))?;
                }
                l.post_entry(e)?;
            }
            l.dispose_asset(asset_id, date, proceeds)
        })?;
        if let Some((description, lines)) = &depreciation {
            let eid = self.create_entry(date, description)?;
            for (account, side, amount) in lines {
                self.add_line(eid, *account, *side, Money::new(*amount, base))?;
            }
            self.post_entry(eid)?;
            let dline = self.append_journal(
                "asset.depreciation_posted",
                json!({
                    "asset_id": asset_id,
                    "period": preview.period,
                    "entry_id": eid,
                    "amount": preview.final_depreciation.to_string(),
                }),
            )?;
            apply_line_to_sqlite(&self.conn, &dline)?;
        }
        let entry_id = self.create_entry(date, &disposal.0)?;
        for (account, side, amount) in &disposal.1 {
            self.add_line(entry_id, *account, *side, Money::new(*amount, base))?;
        }
        self.post_entry(entry_id)?;
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
            CloseTo::Members(members)
            | CloseTo::MembersPerClose(members)
            | CloseTo::MembersLastAbsorbs(members) => {
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
                    CloseTo::Members(_) => v["rounding"] = json!("cumulative"),
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
    pub fn mark_cleared(
        &mut self,
        entry_id: EntryId,
        line_index: usize,
        cleared: bool,
    ) -> Result<(), StoreError> {
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

    /// Fetch a control account by code, creating it through the journaled
    /// account path when missing. The ledger's internal ensure_account
    /// then no-ops, which keeps replay deterministic: every account
    /// creation is an `account.added` event.
    fn ensure_control_account(
        &mut self,
        c: &ledger_core::ControlAccount,
    ) -> Result<AccountId, StoreError> {
        if let Some(a) = self.ledger.chart().find_by_code(c.code) {
            return Ok(a.id);
        }
        self.add_account(c.code, c.name, c.account_type, None, false)
    }

    /// Journal an auto-posted two-line entry (debit one account, credit
    /// another for the same amount) through the standard entry.created /
    /// entry.line_added / entry.posted flow. Returns the predicted entry id
    /// and the journaled lines; the caller mutates the ledger afterwards
    /// and applies the lines to SQLite.
    fn journal_auto_entry(
        &mut self,
        date: NaiveDate,
        description: &str,
        debit_account: AccountId,
        credit_account: AccountId,
        amount: Decimal,
    ) -> Result<(EntryId, Vec<JournalLine>), StoreError> {
        let entry_id = self.ledger.next_entry_id();
        let base = self.base_currency;
        let mut journaled = Vec::with_capacity(4);
        journaled.push(self.append_journal(
            "entry.created",
            json!({
                "id": entry_id,
                "date": date.to_string(),
                "description": description,
            }),
        )?);
        for (account, side) in [(debit_account, Side::Debit), (credit_account, Side::Credit)] {
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
        }
        journaled.push(self.append_journal(
            "entry.posted",
            json!({
                "id": entry_id,
                "date": date.to_string(),
                "description": description,
                "reversal_of": Option::<u64>::None,
                "lines": [
                    plain_line_json(debit_account, Side::Debit, Money::new(amount, base)),
                    plain_line_json(credit_account, Side::Credit, Money::new(amount, base)),
                ],
            }),
        )?);
        Ok((entry_id, journaled))
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
    pub fn create_invoice(
        &mut self,
        customer_id: CustomerId,
        date: NaiveDate,
        due_date: NaiveDate,
        lines: Vec<DocumentLine>,
    ) -> Result<InvoiceId, StoreError> {
        self.rehearse(|l| {
            ensure_on(l, &ledger_core::RECEIVABLES)?;
            ensure_on(l, &ledger_core::REVENUE)?;
            l.create_invoice(customer_id, date, due_date, lines.clone())
        })?;
        // Pure pre-validation mirroring Ledger::create_invoice, so the
        // journal writes below cannot be followed by a failed mutation.
        // (The auto-posted entry balances by construction and its accounts
        // are ensured below.)
        if self.ledger.get_customer(customer_id).is_none() {
            return Err(LedgerError::UnknownCustomer(customer_id).into());
        }
        arap::validate_lines(&lines)?;
        arap::validate_due_date(date, due_date)?;
        if let Some(pid) = self.ledger.closed_period_containing(date) {
            return Err(LedgerError::ClosedPeriod(pid, date.to_string()).into());
        }
        let ar = self.ensure_control_account(&ledger_core::RECEIVABLES)?;
        let revenue = self.ensure_control_account(&ledger_core::REVENUE)?;
        let invoice_id = self.ledger.next_invoice_id();
        let total: Decimal = lines.iter().map(|l| l.amount).sum();
        let description = format!("Invoice {invoice_id}");
        let (entry_id, mut journaled) =
            self.journal_auto_entry(date, &description, ar, revenue, total)?;
        let doc_lines: Vec<Value> = lines.iter().map(document_line_json).collect();
        journaled.push(self.append_journal(
            "invoice.created",
            json!({
                "id": invoice_id,
                "customer_id": customer_id,
                "date": date.to_string(),
                "due_date": due_date.to_string(),
                "lines": doc_lines,
                "entry_id": entry_id,
            }),
        )?);
        let live = self
            .ledger
            .create_invoice(customer_id, date, due_date, lines)?;
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

    /// Create a vendor bill. The auto-posted entry (Dr General Expenses, Cr
    /// A/P) is journaled first through the standard entry flow, then the
    /// bill itself as `bill.created`.
    pub fn create_bill(
        &mut self,
        vendor_id: VendorId,
        date: NaiveDate,
        due_date: NaiveDate,
        lines: Vec<DocumentLine>,
    ) -> Result<BillId, StoreError> {
        self.rehearse(|l| {
            ensure_on(l, &ledger_core::GENERAL_EXPENSES)?;
            ensure_on(l, &ledger_core::PAYABLES)?;
            l.create_bill(vendor_id, date, due_date, lines.clone())
        })?;
        if self.ledger.get_vendor(vendor_id).is_none() {
            return Err(LedgerError::UnknownVendor(vendor_id).into());
        }
        arap::validate_lines(&lines)?;
        arap::validate_due_date(date, due_date)?;
        if let Some(pid) = self.ledger.closed_period_containing(date) {
            return Err(LedgerError::ClosedPeriod(pid, date.to_string()).into());
        }
        let expense = self.ensure_control_account(&ledger_core::GENERAL_EXPENSES)?;
        let ap = self.ensure_control_account(&ledger_core::PAYABLES)?;
        let bill_id = self.ledger.next_bill_id();
        let total: Decimal = lines.iter().map(|l| l.amount).sum();
        let description = format!("Bill {bill_id}");
        let (entry_id, mut journaled) =
            self.journal_auto_entry(date, &description, expense, ap, total)?;
        let doc_lines: Vec<Value> = lines.iter().map(document_line_json).collect();
        journaled.push(self.append_journal(
            "bill.created",
            json!({
                "id": bill_id,
                "vendor_id": vendor_id,
                "date": date.to_string(),
                "due_date": due_date.to_string(),
                "lines": doc_lines,
                "entry_id": entry_id,
            }),
        )?);
        let live = self.ledger.create_bill(vendor_id, date, due_date, lines)?;
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

    /// Record a customer payment against an invoice. The payment entry (Dr
    /// cash, Cr A/R) is journaled through the standard entry flow first,
    /// then `invoice.payment_received`.
    pub fn receive_payment(
        &mut self,
        invoice_id: InvoiceId,
        date: NaiveDate,
        amount: Decimal,
        cash_account: AccountId,
    ) -> Result<EntryId, StoreError> {
        self.rehearse(|l| {
            ensure_on(l, &ledger_core::RECEIVABLES)?;
            l.receive_payment(invoice_id, date, amount, cash_account)
        })?;
        // Pure pre-validation mirroring Ledger::receive_payment, plus the
        // account checks post_entry would perform: a failed post after the
        // journal writes would leave the journal ahead of the ledger.
        if amount <= Decimal::ZERO {
            return Err(LedgerError::InvalidPaymentAmount(amount).into());
        }
        let balance = self
            .ledger
            .get_invoice(invoice_id)
            .ok_or(LedgerError::UnknownInvoice(invoice_id))?
            .balance();
        if amount > balance {
            return Err(LedgerError::Overpayment(amount, balance).into());
        }
        if let Some(pid) = self.ledger.closed_period_containing(date) {
            return Err(LedgerError::ClosedPeriod(pid, date.to_string()).into());
        }
        let cash = self
            .ledger
            .chart()
            .get(cash_account)
            .ok_or(LedgerError::UnknownAccount(cash_account))?;
        if cash.placeholder {
            return Err(LedgerError::PlaceholderAccount(cash.code.clone()).into());
        }
        let ar = self.ensure_control_account(&ledger_core::RECEIVABLES)?;
        let description = format!("Payment on invoice {invoice_id}");
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
        let live = self
            .ledger
            .receive_payment(invoice_id, date, amount, cash_account)?;
        debug_assert_eq!(live, entry_id, "id allocator diverged from journal");
        for line in &journaled {
            apply_line_to_sqlite(&self.conn, line)?;
        }
        Ok(live)
    }

    /// Pay a vendor bill. The payment entry (Dr A/P, Cr cash) is journaled
    /// through the standard entry flow first, then `bill.paid`.
    pub fn pay_bill(
        &mut self,
        bill_id: BillId,
        date: NaiveDate,
        amount: Decimal,
        cash_account: AccountId,
    ) -> Result<EntryId, StoreError> {
        self.rehearse(|l| {
            ensure_on(l, &ledger_core::PAYABLES)?;
            l.pay_bill(bill_id, date, amount, cash_account)
        })?;
        if amount <= Decimal::ZERO {
            return Err(LedgerError::InvalidPaymentAmount(amount).into());
        }
        let balance = self
            .ledger
            .get_bill(bill_id)
            .ok_or(LedgerError::UnknownBill(bill_id))?
            .balance();
        if amount > balance {
            return Err(LedgerError::Overpayment(amount, balance).into());
        }
        if let Some(pid) = self.ledger.closed_period_containing(date) {
            return Err(LedgerError::ClosedPeriod(pid, date.to_string()).into());
        }
        let cash = self
            .ledger
            .chart()
            .get(cash_account)
            .ok_or(LedgerError::UnknownAccount(cash_account))?;
        if cash.placeholder {
            return Err(LedgerError::PlaceholderAccount(cash.code.clone()).into());
        }
        let ap = self.ensure_control_account(&ledger_core::PAYABLES)?;
        let description = format!("Payment on bill {bill_id}");
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
        let live = self.ledger.pay_bill(bill_id, date, amount, cash_account)?;
        debug_assert_eq!(live, entry_id, "id allocator diverged from journal");
        for line in &journaled {
            apply_line_to_sqlite(&self.conn, line)?;
        }
        Ok(live)
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
        self.rehearse(|l| {
            for c in &ledger_core::PAYROLL {
                ensure_on(l, c)?;
            }
            l.run_payroll(date, lines.clone(), cash_account)
        })?;
        // Pure pre-validation mirroring Ledger::run_payroll, so the journal
        // writes below cannot be followed by a failed mutation.
        self.ledger.validate_payroll(date, &lines, cash_account)?;
        let accounts = self.ensure_payroll_accounts()?;
        let run_id = self.ledger.next_payroll_run_id();
        let entry_id = self.ledger.next_entry_id();
        let base = self.base_currency;
        let plan = payroll_entry_plan(&lines, accounts, cash_account, base);
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

    /// Add a class (division) for line-level P&L tracking. Journaled as
    /// `class.added`.
    pub fn add_class(&mut self, name: &str) -> Result<ClassId, StoreError> {
        self.rehearse(|l| l.add_class(name))?;
        // The engine's only failure mode is an empty name; replicate it so
        // the journal write cannot be followed by a failed mutation.
        if name.trim().is_empty() {
            return Err(LedgerError::EmptyClassName.into());
        }
        let id = self.ledger.next_class_id();
        let line = self.append_journal("class.added", json!({ "id": id, "name": name }))?;
        let live = self.ledger.add_class(name)?;
        debug_assert_eq!(live, id, "id allocator diverged from journal");
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(live)
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
        // Draft-only check via the ledger, before anything is journaled.
        let entry = self
            .ledger
            .get_entry(entry_id)
            .ok_or(LedgerError::UnknownEntry(entry_id))?;
        if entry.status() != EntryStatus::Draft {
            return Err(LedgerError::PostedEntryImmutable.into());
        }
        if let Some(cid) = class_id
            && self.ledger.get_class(cid).is_none()
        {
            return Err(LedgerError::UnknownClass(cid).into());
        }
        if line_index >= entry.lines().len() {
            return Err(LedgerError::LineIndexOutOfRange(line_index, entry_id).into());
        }
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

    /// Register an amortization schedule. Journaled as `schedule.registered`.
    pub fn register_schedule(
        &mut self,
        new: ledger_core::NewSchedule,
    ) -> Result<ledger_core::ScheduleId, StoreError> {
        self.rehearse(|l| l.register_schedule(new.clone()))?;
        ledger_core::schedules::validate_schedule(&new, self.ledger.chart())?;
        let id = self.ledger.next_schedule_id();
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
            }),
        )?;
        let got = self.ledger.register_schedule(new)?;
        debug_assert_eq!(got, id);
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(id)
    }

    /// Post one month of a schedule. Journaled as `schedule.posted` with the
    /// whole entry, so a month is never half-posted or posted twice; replay
    /// re-derives the entry and checks it line by line.
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
    /// Returns (schedule, month, entry) for each.
    pub fn run_schedules(
        &mut self,
        through: NaiveDate,
    ) -> Result<Vec<(ledger_core::ScheduleId, NaiveDate, EntryId)>, StoreError> {
        let mut due: Vec<(NaiveDate, ledger_core::ScheduleId)> = self
            .ledger
            .schedules()
            .flat_map(|s| s.due(through).into_iter().map(move |(d, _)| (d, s.id)))
            .collect();
        due.sort();
        let mut out = Vec::with_capacity(due.len());
        for (month, id) in due {
            out.push((id, month, self.post_schedule_month(id, month)?));
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
    /// entries); [`Ledger::recurring_in_closed_periods`] lists them.
    pub fn run_recurring(
        &mut self,
        through: NaiveDate,
    ) -> Result<Vec<(ledger_core::RecurringId, NaiveDate, EntryId)>, StoreError> {
        let mut due: Vec<(NaiveDate, ledger_core::RecurringId)> = self
            .ledger
            .recurring_entries()
            .flat_map(|r| r.due(through).into_iter().map(move |d| (d, r.id)))
            .collect();
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

    /// Set (or clear) an account's tax category and deductible percent.
    /// Journaled as `account.tax_set`.
    pub fn set_account_tax(
        &mut self,
        account: AccountId,
        category: Option<&str>,
        deductible_percent: Option<Decimal>,
    ) -> Result<(), StoreError> {
        self.rehearse(|l| l.set_account_tax(account, category, deductible_percent))?;
        self.ledger
            .check_account_tax(account, category, deductible_percent)?;
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

    /// Set which 1099 a vendor gets (or none) and whether its W-9 is on
    /// file. Journaled as `vendor.1099_set`.
    pub fn set_vendor_1099(
        &mut self,
        vendor: VendorId,
        form: Option<ledger_core::Form1099>,
        tin_on_file: bool,
    ) -> Result<(), StoreError> {
        self.rehearse(|l| l.set_vendor_1099(vendor, form, tin_on_file))?;
        self.ledger
            .get_vendor(vendor)
            .ok_or(LedgerError::UnknownVendor(vendor))?;
        let line = self.append_journal(
            "vendor.1099_set",
            json!({ "vendor_id": vendor, "form_1099": form, "tin_on_file": tin_on_file }),
        )?;
        self.ledger.set_vendor_1099(vendor, form, tin_on_file)?;
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(())
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
    ///   5. Otherwise: SHOULD journal `receipt.linked`.
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
    ///   2. IF it is linked to an entry: SHOULD refuse (AttachmentLinked).
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
        let line = self.append_journal(
            "attachment.withdrawn",
            json!({ "attachment_id": attachment_id, "reason": reason }),
        )?;
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(())
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
        // The accounting snapshot ignores the asset register, so compare it
        // explicitly: journal replay, live ledger, and projection must agree.
        let assets_of = |l: &Ledger| l.registered_assets().cloned().collect::<Vec<_>>();
        let (ra, rb, rc) = (assets_of(&replayed), assets_of(&self.ledger), assets_of(&c));
        if ra != rb || ra != rc {
            return Err(StoreError::VerifyMismatch(
                "asset register diverged between journal, live ledger, and sqlite projection"
                    .into(),
            ));
        }
        // Classes and payroll runs are master data the snapshot also
        // ignores; compare them explicitly the same way.
        let classes_of = |l: &Ledger| l.list_classes().into_iter().cloned().collect::<Vec<_>>();
        let (ca, cb, cc) = (
            classes_of(&replayed),
            classes_of(&self.ledger),
            classes_of(&c),
        );
        if ca != cb || ca != cc {
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
        let (pa, pb, pc) = (runs_of(&replayed), runs_of(&self.ledger), runs_of(&c));
        if pa != pb || pa != pc {
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
                    "wanderbooks: {why}; kept the old database as {} and rebuilt from the journal",
                    keep.display()
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
        })
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
                    let got = ledger.reverse_entry(orig as u64, date, &description)?;
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
            let got = ledger.register_asset(NewAsset {
                description,
                purchase_date: parse_date(&purchase_date)?,
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
                ledger.dispose_asset(
                    got,
                    parse_date(date)?,
                    parse_amt(proceeds, &cost_currency)?,
                )?;
            }
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
                "SELECT employee_name, gross_pay, federal_withholding, state_withholding, ss_employee, medicare_employee, ss_employer, medicare_employer, other_deductions FROM payroll_lines WHERE run_id = ? ORDER BY id",
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
                ))
            })?;
            let mut lines = Vec::new();
            for lr in pl_rows {
                let (name, gross, fed, state, ss_e, med_e, ss_r, med_r, other) = lr?;
                lines.push(PayrollLine {
                    employee_name: name,
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
/// strings, never floats.
fn document_line_json(l: &DocumentLine) -> Value {
    json!({
        "description": l.description,
        "amount": l.amount.to_string(),
    })
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
            })
        })
        .collect()
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
    if parts.next().is_some() || year < 1 || !(1..=12).contains(&month) {
        return Err(err());
    }
    Ok((year, month))
}

/// Last calendar day of a month.
fn last_day_of_month(year: i32, month: u32) -> NaiveDate {
    let first = NaiveDate::from_ymd_opt(year, month, 1).expect("valid period");
    first
        .checked_add_months(Months::new(1))
        .expect("month arithmetic")
        .pred_opt()
        .expect("month arithmetic")
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
/// [`Store::ensure_control_account`] on a bare ledger, for rehearsals.
fn ensure_on(l: &mut Ledger, c: &ledger_core::ControlAccount) -> Result<AccountId, LedgerError> {
    match l.chart().find_by_code(c.code) {
        Some(a) => Ok(a.id),
        None => l.add_account(c.code, c.name, c.account_type, None, false),
    }
}

pub(crate) fn replay_line_into_ledger(
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
            let got = ledger.reverse_entry(
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
            let got = ledger.replay_register_schedule(ledger_core::NewSchedule {
                description: get_str(p, "description")?.to_string(),
                kind: serde_json::from_value(p["kind"].clone())?,
                total: parse_money(p, "total")?,
                start: parse_date(get_str(p, "start")?)?,
                months: get_u64(p, "months")? as u32,
                balance_account: get_u64(p, "balance_account")?,
                pl_account: get_u64(p, "pl_account")?,
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
            let got = ledger.register_recurring(ledger_core::NewRecurring {
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
            let form = match p.get("form_1099").and_then(|v| v.as_str()) {
                Some(s) => Some(ledger_core::Form1099::parse(s).ok_or_else(|| {
                    StoreError::JournalCorrupt("vendor.1099_set form_1099".into())
                })?),
                None => None,
            };
            ledger.set_vendor_1099(
                get_u64(p, "vendor_id")?,
                form,
                p.get("tin_on_file")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
            )?;
        }
        "entry.vendor_set" => {
            ledger.set_entry_vendor(
                get_u64(p, "entry_id")?,
                p.get("vendor_id").and_then(|v| v.as_u64()),
            )?;
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
        "receipt.attached"
        | "receipt.linked"
        | "receipt.unlinked"
        | "document.filed"
        | "attachment.withdrawn" => {
            // Not accounting state; the projection records them.
        }
        "asset.registered" => {
            let want = get_u64(p, "id")?;
            let got = ledger.register_asset(NewAsset {
                description: get_str(p, "description")?.to_string(),
                purchase_date: parse_date(get_str(p, "purchase_date")?)?,
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
        "asset.depreciation_posted" => {
            // The entry itself was journaled as entry.created/entry.posted;
            // this line is idempotency bookkeeping for the projection.
        }
        "asset.disposed" => {
            let id = get_u64(p, "asset_id")?;
            ledger.dispose_asset(
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
            let to = close_to_from_journal(p.get("close_to"))?;
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
            ledger.replay_invoice(
                get_u64(p, "id")?,
                get_u64(p, "customer_id")?,
                parse_date(get_str(p, "date")?)?,
                parse_date(get_str(p, "due_date")?)?,
                parse_document_lines(p)?,
                get_u64(p, "entry_id")?,
            )?;
        }
        "bill.created" => {
            ledger.replay_bill(
                get_u64(p, "id")?,
                get_u64(p, "vendor_id")?,
                parse_date(get_str(p, "date")?)?,
                parse_date(get_str(p, "due_date")?)?,
                parse_document_lines(p)?,
                get_u64(p, "entry_id")?,
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
        "journal.tail_repaired" => {}
        other => {
            return Err(StoreError::JournalCorrupt(format!(
                "unknown line type: {other}"
            )));
        }
    }
    Ok(())
}

/// Apply a payment to an invoice or bill row in the SQLite projection:
/// bump `amount_paid` and re-derive the status. Amounts are TEXT in
/// SQLite, so the math happens here in Rust with exact Decimals.
/// `table` is `invoices` or `bills`; an internal constant, never user input.
fn apply_document_payment(
    conn: &Connection,
    table: &str,
    id: u64,
    amount: Decimal,
) -> Result<(), StoreError> {
    let (paid_str, lines_json): (String, String) = conn.query_row(
        &format!("SELECT amount_paid, lines_json FROM {table} WHERE id = ?"),
        params![id as i64],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let paid: Decimal = paid_str
        .parse()
        .map_err(|_| StoreError::JournalCorrupt(format!("bad amount_paid {paid_str}")))?;
    let new_paid = paid + amount;
    let lines: Vec<Value> = serde_json::from_str(&lines_json)
        .map_err(|_| StoreError::JournalCorrupt("bad lines_json".into()))?;
    let mut total = Decimal::ZERO;
    for l in &lines {
        total += get_str(l, "amount")?
            .parse::<Decimal>()
            .map_err(|_| StoreError::JournalCorrupt("bad line amount".into()))?;
    }
    let status = if new_paid.is_zero() {
        "open"
    } else if new_paid >= total {
        "paid"
    } else {
        "partial"
    };
    conn.execute(
        &format!("UPDATE {table} SET amount_paid = ?, status = ? WHERE id = ?"),
        params![new_paid.to_string(), status, id as i64],
    )?;
    Ok(())
}

/// Apply one journal line to the SQLite projection: INSERTs plus the events
/// mirror, then advance `last_applied_seq`. The applier is a pure function
/// of the journal line: INSERTs for new facts, targeted UPDATEs/DELETEs
/// for lifecycle changes (receipt linking, disposal, period close,
/// cleared flags, document payments, fx rate updates).
/// The close target recorded in a `period.closed` line. Absent or
/// `"retained_earnings"`: Retained Earnings with no sweeps (every journal
/// written before targets existed).
fn close_to_from_journal(v: Option<&Value>) -> Result<CloseTo, StoreError> {
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
    // per close, then "cumulative" over a run of closes.
    match v.get("rounding").and_then(|r| r.as_str()) {
        None => Ok(CloseTo::MembersLastAbsorbs(members)),
        Some("largest_remainder") => Ok(CloseTo::MembersPerClose(members)),
        Some("cumulative") => Ok(CloseTo::Members(members)),
        Some(_) => Err(corrupt("unknown rounding")),
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
        "store.created" | "journal.tail_repaired" => {}
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
            conn.execute(
                "UPDATE vendors SET form_1099 = ?, tin_on_file = ? WHERE id = ?",
                params![
                    p.get("form_1099").and_then(|v| v.as_str()),
                    p.get("tin_on_file")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false),
                    get_u64(p, "vendor_id")? as i64,
                ],
            )?;
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
                "INSERT INTO invoices(id, customer_id, date, due_date, lines_json, amount_paid, status, entry_id) VALUES (?,?,?,?,?,?,?,?)",
                params![
                    get_u64(p, "id")? as i64,
                    get_u64(p, "customer_id")? as i64,
                    get_str(p, "date")?,
                    get_str(p, "due_date")?,
                    lines.to_string(),
                    "0",
                    "open",
                    get_u64(p, "entry_id")? as i64,
                ],
            )?;
        }
        "bill.created" => {
            let lines = p
                .get("lines")
                .ok_or_else(|| StoreError::JournalCorrupt("bill.created without lines".into()))?;
            conn.execute(
                "INSERT INTO bills(id, vendor_id, date, due_date, lines_json, amount_paid, status, entry_id) VALUES (?,?,?,?,?,?,?,?)",
                params![
                    get_u64(p, "id")? as i64,
                    get_u64(p, "vendor_id")? as i64,
                    get_str(p, "date")?,
                    get_str(p, "due_date")?,
                    lines.to_string(),
                    "0",
                    "open",
                    get_u64(p, "entry_id")? as i64,
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
        "invoice.payment_received" => {
            let amount: Decimal = get_str(p, "amount")?
                .parse()
                .map_err(|_| StoreError::JournalCorrupt("bad amount".into()))?;
            apply_document_payment(conn, "invoices", get_u64(p, "invoice_id")?, amount)?;
        }
        "bill.paid" => {
            let amount: Decimal = get_str(p, "amount")?
                .parse()
                .map_err(|_| StoreError::JournalCorrupt("bad amount".into()))?;
            apply_document_payment(conn, "bills", get_u64(p, "bill_id")?, amount)?;
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
                    "INSERT INTO payroll_lines(run_id, employee_name, gross_pay, federal_withholding, state_withholding, ss_employee, medicare_employee, ss_employer, medicare_employer, other_deductions) VALUES (?,?,?,?,?,?,?,?,?,?)",
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
        other => {
            return Err(StoreError::JournalCorrupt(format!(
                "unknown line type: {other}"
            )));
        }
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

/// What [`Store::verify_and_heal`] found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifyReport {
    /// Journal events verified.
    pub events: u64,
    /// Hash of the last journal line: the verified head of the chain.
    pub head: String,
    /// Why the projection was rebuilt, if it was.
    pub healed: Option<String>,
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
        sql.push_str(" WHERE a.entry_id = ?");
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
