//! [`Store`]: the write path, the SQLite projection, and maintenance.
//!
//! Write-path order for every mutation is **validate → journal → mutate →
//! apply**. Journaling before mutating means a failed write leaves nothing
//! behind: the journal is the source of truth, the in-memory [`Ledger`] is
//! the validation authority, and SQLite is the queryable projection that
//! the next open heals via `last_applied_seq`.

use crate::blob::BlobStore;
use crate::error::StoreError;
use crate::journal::{JournalLine, JournalWriter, read_and_verify_journal};
use crate::schema;
use ledger_core::{
    AccountId, AccountType, Currency, EntryId, EntryStatus, Ledger, LedgerError, Money, Side,
    reports,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// File names inside the data directory.
pub const JOURNAL_FILE: &str = "journal.jsonl";
/// File names inside the data directory.
pub const DB_FILE: &str = "wanderbooks.db";
const BLOBS_DIR: &str = "blobs";

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
}

/// The store: one writer, one data directory.
///
/// `Store` is `Send` but not `Sync` (it owns a `rusqlite::Connection`).
/// There is exactly one writer per data directory.
pub struct Store {
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
    pub fn create(data_dir: &Path, base_currency: Currency) -> Result<Self, StoreError> {
        let journal_path = data_dir.join(JOURNAL_FILE);
        if journal_path.exists() {
            return Err(StoreError::AlreadyExists(data_dir.display().to_string()));
        }
        std::fs::create_dir_all(data_dir)?;
        let db_path = data_dir.join(DB_FILE);
        let conn = Self::open_conn(&db_path)?;
        Self::migrate(&conn)?;
        Self::write_meta(&conn, "base_currency", base_currency.code())?;
        Self::write_meta(&conn, "last_applied_seq", "0")?;
        let mut store = Self {
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
    pub fn open(data_dir: &Path) -> Result<Self, StoreError> {
        let journal_path = data_dir.join(JOURNAL_FILE);
        if !journal_path.exists() {
            return Err(StoreError::NotFound(data_dir.display().to_string()));
        }
        // Single verified scan; the writer reuses it.
        let lines = read_and_verify_journal(&journal_path)?;
        let journal = JournalWriter::from_lines(&journal_path, &lines)?;
        let db_path = data_dir.join(DB_FILE);
        let fresh_db = !db_path.exists();
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
            data_dir: data_dir.to_path_buf(),
            db_path,
            journal,
            conn,
            blobs: BlobStore::open(&data_dir.join(BLOBS_DIR))?,
            ledger: Ledger::new(base_currency),
            base_currency,
        };
        store.catch_up(&lines)?;
        Ok(store)
    }

    fn open_conn(db_path: &Path) -> Result<Connection, StoreError> {
        let conn = Connection::open(db_path)?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;",
        )?;
        let check: String = conn.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
        if check != "ok" {
            return Err(StoreError::JournalCorrupt(format!(
                "sqlite integrity_check failed: {check}"
            )));
        }
        Ok(conn)
    }

    fn migrate(conn: &Connection) -> Result<(), StoreError> {
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

    /// Add a line to a draft entry. Draft state is ephemeral and is not
    /// journaled; only the posted entry hits the journal.
    pub fn add_line(
        &mut self,
        entry: EntryId,
        account: AccountId,
        side: Side,
        amount: Money,
    ) -> Result<(), StoreError> {
        self.ledger.add_line(entry, account, side, amount)?;
        Ok(())
    }

    /// Validate, journal, and post a draft entry.
    pub fn post_entry(&mut self, entry: EntryId) -> Result<(), StoreError> {
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
        if orig.status() != EntryStatus::Posted {
            return Err(LedgerError::EntryNotPosted(entry).into());
        }
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
    // Blobs and attachments
    // ------------------------------------------------------------------

    /// Attach a receipt to an entry (or file it unlinked with `entry_id=None`
    /// and link later). Returns `(attachment_id, sha256, deduplicated)`.
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

    /// Link a previously filed receipt to an entry.
    pub fn link_receipt(
        &mut self,
        attachment_id: u64,
        entry_id: EntryId,
    ) -> Result<(), StoreError> {
        self.ledger
            .get_entry(entry_id)
            .ok_or(LedgerError::UnknownEntry(entry_id))?;
        let exists: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM attachments WHERE id = ?",
            params![attachment_id as i64],
            |r| r.get(0),
        )?;
        if exists == 0 {
            return Err(StoreError::UnknownAttachment(attachment_id));
        }
        let line = self.append_journal(
            "receipt.linked",
            json!({ "attachment_id": attachment_id, "entry_id": entry_id }),
        )?;
        apply_line_to_sqlite(&self.conn, &line)?;
        Ok(())
    }

    /// List attachments, optionally filtered to one entry.
    pub fn list_attachments(
        &self,
        entry_id: Option<EntryId>,
    ) -> Result<Vec<Attachment>, StoreError> {
        let mut sql = String::from(
            "SELECT a.id, a.blob_sha, a.entry_id, a.category, b.original_name, b.mime_type, \
             b.size_bytes, a.description, a.created_at FROM attachments a \
             JOIN blobs b ON b.sha256 = a.blob_sha",
        );
        if entry_id.is_some() {
            sql.push_str(" WHERE a.entry_id = ?");
        }
        sql.push_str(" ORDER BY a.id");
        let mut stmt = self.conn.prepare(&sql)?;
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

    /// Read attachment bytes, verifying the SHA-256.
    pub fn attachment_bytes(&self, attachment_id: u64) -> Result<Vec<u8>, StoreError> {
        let sha: String = self.conn.query_row(
            "SELECT blob_sha FROM attachments WHERE id = ?",
            params![attachment_id as i64],
            |r| r.get(0),
        )?;
        self.blobs.get(&sha)
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

    /// Verify everything: the journal hash chain, a fresh replay's reports
    /// against the live ledger, and the SQLite projection against the replay.
    /// Loud on any divergence.
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
        let c = self.load_ledger_from_sqlite()?;
        let c_snap = reports::snapshot(&c).map_err(StoreError::Ledger)?;
        if a != c_snap {
            return Err(StoreError::VerifyMismatch(
                "sqlite projection diverged from journal".into(),
            ));
        }
        Ok(())
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
            let got = ledger.add_account(
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
                        "SELECT account_id, side, amount, currency FROM lines WHERE entry_id = ? ORDER BY id",
                    )?;
                    let lrows = lstmt.query_map(params![id], |r| {
                        Ok((
                            r.get::<_, i64>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, String>(2)?,
                            r.get::<_, String>(3)?,
                        ))
                    })?;
                    for lr in lrows {
                        let (account, side, amount, currency) = lr?;
                        ledger.add_line(
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
                    }
                    ledger.post_entry(got)?;
                }
            }
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
    json!({
        "account": l.account,
        "side": side,
        "amount": l.amount.amount.to_string(),
        "currency": l.amount.currency.code(),
    })
}

fn flip_line_json(l: &ledger_core::JournalLine) -> Value {
    let side = match l.side {
        Side::Debit => "credit",
        Side::Credit => "debit",
    };
    json!({
        "account": l.account,
        "side": side,
        "amount": l.amount.amount.to_string(),
        "currency": l.amount.currency.code(),
    })
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
fn replay_line_into_ledger(
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
            let got = ledger.add_account(
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
        "entry.posted" => {
            let id = get_u64(p, "id")?;
            // The draft was created by the entry.created line; add lines, post.
            for l in p
                .get("lines")
                .and_then(|v| v.as_array())
                .ok_or_else(|| StoreError::JournalCorrupt("entry.posted without lines".into()))?
            {
                ledger.add_line(
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
            ledger.post_entry(id)?;
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
        "receipt.attached" | "receipt.linked" | "document.filed" => {
            // Not accounting state; the projection records them.
        }
        other => {
            return Err(StoreError::JournalCorrupt(format!(
                "unknown line type: {other}"
            )));
        }
    }
    Ok(())
}

/// Apply one journal line to the SQLite projection: INSERTs plus the events
/// mirror, then advance `last_applied_seq`. The applier only ever INSERTs
/// (and one UPDATE for receipt linking).
fn apply_line_to_sqlite(conn: &Connection, line: &JournalLine) -> Result<(), StoreError> {
    let p = &line.payload;
    match line.line_type.as_str() {
        "store.created" => {}
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
                    "INSERT INTO lines(entry_id, account_id, side, amount, currency) VALUES (?,?,?,?,?)",
                    params![
                        get_u64(p, "id")? as i64,
                        get_u64(l, "account")? as i64,
                        get_str(l, "side")?,
                        get_str(l, "amount")?,
                        get_str(l, "currency")?,
                    ],
                )?;
            }
        }
        "entry.reversed" => {
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
                    "INSERT INTO lines(entry_id, account_id, side, amount, currency) VALUES (?,?,?,?,?)",
                    params![
                        get_u64(p, "id")? as i64,
                        get_u64(l, "account")? as i64,
                        get_str(l, "side")?,
                        get_str(l, "amount")?,
                        get_str(l, "currency")?,
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
                "INSERT INTO attachments(id, blob_sha, entry_id, category, description, created_at) VALUES (?,?,?,?,?,?)",
                params![
                    get_u64(p, "attachment_id")? as i64,
                    get_str(p, "blob_sha")?,
                    p.get("entry_id").and_then(|v| v.as_u64()).map(|v| v as i64),
                    get_str(p, "category")?,
                    p.get("description").and_then(|v| v.as_str()),
                    line.ts,
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
    })
}
