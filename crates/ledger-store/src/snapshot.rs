//! [`Snapshot`]: a read-only view of the books, for viewers.
//!
//! A snapshot never writes to the data directory. It reads the journal,
//! verifies the hash chain, replays it into its own [`Ledger`], and projects
//! it into a private in-memory SQLite database with exactly the code the
//! writer uses, so attachments and events read the same as through
//! [`crate::Store`]. Blobs are read (and verified) straight from the blob
//! store.
//!
//! [`Snapshot::refresh`] follows a live journal: it reads only the bytes
//! appended since the last read, verifies they chain onto what it has, and
//! applies them. A line the writer is still in the middle of appending
//! (no newline yet) is left for the next refresh, never treated as
//! corruption. If the file shrank or no longer chains, the snapshot reloads
//! from scratch.

use crate::blob::BlobStore;
use crate::error::StoreError;
use crate::journal::{GENESIS_PREV, JournalLine, sha256_hex};
use crate::store::{
    Attachment, BLOBS_DIR, DB_FILE, JOURNAL_FILE, Store, apply_line_to_sqlite, compare_projections,
    projection_rows, query_attachments, read_attachment_bytes, replay_line_into_ledger,
};
use ledger_core::{Currency, EntryId, Ledger};
use rusqlite::Connection;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// How the writer's on-disk SQLite database compares with the journal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiskCheck {
    /// Every table matches a fresh projection of the journal.
    Matches,
    /// The database hasn't applied the newest journal lines yet (normal
    /// for a moment after a write; the writer applies right after
    /// journaling, and heals on its next start).
    Behind {
        /// Journal lines the database has applied.
        applied: u64,
        /// Journal lines there are.
        journal: u64,
    },
    /// The database has applied journal lines this snapshot hasn't read
    /// yet (normal while the writer is busy; the next refresh catches up).
    Ahead {
        /// Journal lines the database has applied.
        applied: u64,
        /// Journal lines this snapshot has read.
        journal: u64,
    },
    /// No database file (the writer rebuilds it on start).
    Missing,
    /// The database differs from the journal: what differs.
    Differs(String),
    /// The database couldn't be read.
    Unreadable(String),
}

/// A read-only, live-refreshable view of a data directory.
pub struct Snapshot {
    data_dir: PathBuf,
    conn: Connection,
    blobs: BlobStore,
    ledger: Ledger,
    base_currency: Currency,
    lines: Vec<JournalLine>,
    /// Bytes of the journal consumed (complete lines only).
    offset: u64,
    /// Hash of the last line read: what the next line must chain onto.
    head: String,
}

impl Snapshot {
    /// Load the books in `data_dir`, read-only.
    pub fn load(data_dir: &Path) -> Result<Self, StoreError> {
        let journal_path = data_dir.join(JOURNAL_FILE);
        if !journal_path.exists() {
            return Err(StoreError::NotFound(data_dir.display().to_string()));
        }
        let conn = Connection::open_in_memory()?;
        Store::migrate(&conn)?;
        let mut snap = Self {
            data_dir: data_dir.to_path_buf(),
            conn,
            blobs: BlobStore::open_read_only(&data_dir.join(BLOBS_DIR)),
            ledger: Ledger::new(Currency::USD),
            base_currency: Currency::USD,
            lines: Vec::new(),
            offset: 0,
            head: GENESIS_PREV.to_string(),
        };
        snap.read_new()?;
        Ok(snap)
    }

    /// Apply whatever the writer appended since the last read. Returns the
    /// number of new journal lines (0 when nothing changed).
    ///
    /// ```text
    /// GOAL: a viewer that follows the live books without ever writing.
    /// SOURCE: docs/receipt-store.md (the journal is the source of truth;
    ///         lines are appended whole and chained); R-11.
    /// INVARIANTS:
    ///   1. NEVER write to the data directory.
    ///   2. MUST apply only complete lines (ending in a newline) that chain
    ///      onto the last one read; a line still being appended waits for
    ///      the next refresh.
    ///   3. After a refresh, the ledger MUST equal a replay of the lines
    ///      read so far.
    /// SCENARIOS:
    ///   1. IF nothing was appended: SHOULD return 0 and change nothing.
    ///   2. IF the file shrank or no longer chains (a restore, a repaired
    ///      tail): SHOULD reload from scratch.
    ///   3. IF a complete new line fails its hash or parse: SHOULD refuse.
    /// ```
    pub fn refresh(&mut self) -> Result<usize, StoreError> {
        let len = std::fs::metadata(self.data_dir.join(JOURNAL_FILE))?.len();
        if len < self.offset {
            // Rewritten underneath us (e.g. restored from a backup).
            *self = Self::load(&self.data_dir)?;
            return Ok(self.lines.len());
        }
        match self.read_new() {
            Ok(n) => Ok(n),
            Err(StoreError::JournalCorrupt(_)) => {
                // No longer chains onto what we have: start over, and let a
                // real corruption surface from the full load.
                *self = Self::load(&self.data_dir)?;
                Ok(self.lines.len())
            }
            Err(e) => Err(e),
        }
    }

    fn read_new(&mut self) -> Result<usize, StoreError> {
        let mut file = std::fs::File::open(self.data_dir.join(JOURNAL_FILE))?;
        file.seek(SeekFrom::Start(self.offset))?;
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)?;
        // Complete lines only: a torn tail is a write still in progress.
        let Some(last_newline) = buf.iter().rposition(|&b| b == b'\n') else {
            return Ok(0);
        };
        let complete = &buf[..=last_newline];
        let mut added = 0;
        for raw in complete.split(|&b| b == b'\n').filter(|l| !l.is_empty()) {
            let line: JournalLine = JournalLine::parse(raw).map_err(|e| {
                StoreError::JournalCorrupt(format!(
                    "seq {}: unparseable line: {e}",
                    self.lines.len() + 1
                ))
            })?;
            let want_seq = self.lines.len() as u64 + 1;
            if line.seq != want_seq {
                return Err(StoreError::JournalCorrupt(format!(
                    "seq gap: expected {want_seq}, found {}",
                    line.seq
                )));
            }
            if line.prev != self.head {
                return Err(StoreError::JournalCorrupt(format!(
                    "hash chain broken at seq {}",
                    line.seq
                )));
            }
            if line.seq == 1 {
                let code = line.payload["base_currency"].as_str().unwrap_or("USD");
                self.base_currency = Currency::new(code)
                    .map_err(|_| StoreError::JournalCorrupt(format!("bad base_currency {code}")))?;
                self.ledger = Ledger::new(self.base_currency);
            }
            replay_line_into_ledger(&mut self.ledger, &line, self.base_currency)?;
            apply_line_to_sqlite(&self.conn, &line)?;
            self.head = sha256_hex(line.to_canonical().as_bytes());
            self.lines.push(line);
            added += 1;
        }
        self.offset += complete.len() as u64;
        Ok(added)
    }

    /// The replayed ledger: the source for every report.
    pub fn ledger(&self) -> &Ledger {
        &self.ledger
    }

    /// The books' base currency.
    pub fn base_currency(&self) -> Currency {
        self.base_currency
    }

    /// The data directory.
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// Every journal line read so far, in order: the complete history,
    /// with timestamps and hashes.
    pub fn journal(&self) -> &[JournalLine] {
        &self.lines
    }

    /// Sequence number of the last journal line read.
    pub fn journal_seq(&self) -> u64 {
        self.lines.len() as u64
    }

    /// Hash of the last journal line: the head of the verified chain.
    pub fn head_hash(&self) -> &str {
        &self.head
    }

    /// Attachments (receipts and documents, withdrawn ones included),
    /// optionally only one entry's.
    pub fn attachments(&self, entry_id: Option<EntryId>) -> Result<Vec<Attachment>, StoreError> {
        query_attachments(&self.conn, entry_id)
    }

    /// Compare the writer's SQLite database (opened read-only) with this
    /// snapshot's own projection of the journal, every table, row for row.
    ///
    /// The applied position and the tables are read in one read
    /// transaction, so a write landing between the two reads can't make
    /// them disagree (the writer is often busy while a viewer checks: found
    /// by the concurrency test, 2026-10-13, as a false "differs").
    pub fn check_disk(&self) -> DiskCheck {
        use rusqlite::OpenFlags;
        let path = self.data_dir.join(DB_FILE);
        if !path.exists() {
            return DiskCheck::Missing;
        }
        // A WAL reader needs the -wal and -shm files, and creates them if
        // they're missing: a write into the books, and a failure when the
        // books are read-only to this user. With no -wal file no writer
        // has the database open (SQLite deletes it when the last one
        // closes), so it can't change under us: read it as an immutable
        // file, creating nothing (found by the viewer's read-only test,
        // 2026-10-14).
        let live = path.with_extension("db-wal").exists();
        let opened = if live {
            Connection::open_with_flags(
                &path,
                OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )
        } else {
            Connection::open_with_flags(
                immutable_uri(&path),
                OpenFlags::SQLITE_OPEN_READ_ONLY
                    | OpenFlags::SQLITE_OPEN_NO_MUTEX
                    | OpenFlags::SQLITE_OPEN_URI,
            )
        };
        let mut conn = match opened {
            Ok(c) => c,
            Err(e) => return DiskCheck::Unreadable(e.to_string()),
        };
        // One consistent view of the database for every read below.
        let conn = match conn.transaction() {
            Ok(t) => t,
            Err(e) => return DiskCheck::Unreadable(e.to_string()),
        };
        let applied: u64 = match conn.query_row(
            "SELECT value FROM meta WHERE key='last_applied_seq'",
            [],
            |r| r.get::<_, String>(0),
        ) {
            Ok(v) => v.parse().unwrap_or(0),
            Err(e) => return DiskCheck::Unreadable(e.to_string()),
        };
        let journal = self.journal_seq();
        if applied < journal {
            return DiskCheck::Behind { applied, journal };
        }
        if applied > journal {
            return DiskCheck::Ahead { applied, journal };
        }
        let (disk, ours) = match (projection_rows(&conn), projection_rows(&self.conn)) {
            (Ok(d), Ok(o)) => (d, o),
            (Err(e), _) | (_, Err(e)) => return DiskCheck::Unreadable(e.to_string()),
        };
        match compare_projections(&disk, &ours) {
            Ok(()) => DiskCheck::Matches,
            Err(StoreError::VerifyMismatch(why)) => DiskCheck::Differs(why),
            Err(e) => DiskCheck::Unreadable(e.to_string()),
        }
    }

    /// Re-hash every stored file. Returns how many verified, and each
    /// failure as (attachment id, why).
    pub fn verify_blobs(&self) -> (u64, Vec<(u64, String)>) {
        let mut ok = 0;
        let mut bad = Vec::new();
        for a in self.attachments(None).unwrap_or_default() {
            match self.blobs.get(&a.blob_sha) {
                Ok(_) => ok += 1,
                Err(e) => bad.push((a.id, e.to_string())),
            }
        }
        (ok, bad)
    }

    /// An attachment's bytes, verified against its hash.
    pub fn attachment_bytes(&self, attachment_id: u64) -> Result<Vec<u8>, StoreError> {
        read_attachment_bytes(&self.conn, &self.blobs, attachment_id)
    }
}

/// `file:` URI for `path`, read-only and immutable (sqlite.org/uri.html):
/// every byte outside the unreserved set is percent-encoded.
fn immutable_uri(path: &Path) -> String {
    let mut out = String::from("file:");
    for b in path.to_string_lossy().bytes() {
        if b.is_ascii_alphanumeric() || b"/-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out.push_str("?mode=ro&immutable=1");
    out
}
