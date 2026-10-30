//! The journal: the source of truth.
//!
//! An append-only file of JSON objects, one per line (JSONL), hash-chained:
//! each line carries the SHA-256 of the previous line's canonical bytes.
//! Bit rot, torn writes, and casual editing are detected on read — never
//! silently absorbed.
//!
//! Canonical form is keys-sorted, whitespace-free JSON. The writer always
//! emits canonical form, so parse → re-serialize round-trips byte-identically
//! and the chain verifies.

use crate::error::StoreError;
use crate::format::JournalFormat;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

/// `prev` value of the first journal line: 64 zeros.
pub const GENESIS_PREV: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// One journal line: a sequenced, timestamped, hash-chained domain event.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JournalLine {
    /// 1-based, gapless sequence number.
    pub seq: u64,
    /// UTC timestamp, RFC 3339. Recorded, never trusted for ordering.
    pub ts: String,
    /// Event type, e.g. `account.added`, `entry.posted`, `asset.tagged`:
    /// every type the store writes is in [`crate::format::EVENT_TYPES`],
    /// has a replay arm in `replay_line_into_ledger`, and has journal text
    /// in the viewer (`ledger-store` and `ledger-gui` tests check). A type
    /// this version doesn't know is refused as `NewerJournal`, not as
    /// corrupt: a newer wanderbooks wrote it.
    #[serde(rename = "type")]
    pub line_type: String,
    /// The full domain facts. Everything a replay needs.
    pub payload: Value,
    /// Hex SHA-256 of the previous line's canonical bytes.
    pub prev: String,
}

impl JournalLine {
    /// Canonical bytes of this line: keys sorted recursively, no whitespace.
    pub fn to_canonical(&self) -> String {
        canonical_json(&serde_json::to_value(self).expect("JournalLine serializes"))
    }
}

/// Recursively key-sorted, whitespace-free JSON.
pub fn canonical_json(value: &Value) -> String {
    serde_json::to_string(&sort_value(value)).expect("JSON serialization cannot fail")
}

fn sort_value(v: &Value) -> Value {
    match v {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let mut sorted = serde_json::Map::with_capacity(map.len());
            for k in keys {
                sorted.insert(k.clone(), sort_value(&map[k]));
            }
            Value::Object(sorted)
        }
        Value::Array(arr) => Value::Array(arr.iter().map(sort_value).collect()),
        _ => v.clone(),
    }
}

/// The keys of a journal line, the same in every journal format
/// (rules.md B-27): a newer format adds its fields inside `payload`, never
/// beside it, and a line's hash is always the SHA-256 of its canonical
/// JSON ([`canonical_json`]), whatever the format.
pub const ENVELOPE_KEYS: [&str; 5] = ["payload", "prev", "seq", "ts", "type"];

impl JournalLine {
    /// Parse one journal line, refusing what the journal never holds.
    ///
    /// ```text
    /// GOAL: read a journal line so that its canonical form (which the
    ///       hash chain covers) is a fixed point.
    /// SOURCE: R-2 (no floats in money paths); found by fuzz_journal_line
    ///         (2026-10-13): a float payload re-canonicalized differently;
    ///         rules.md B-27 (the envelope is frozen).
    /// INVARIANTS:
    ///   1. NEVER accepts a floating-point number anywhere in the line:
    ///      amounts are decimal strings and ids are integers.
    ///   2. NEVER drops a key: a line with a key beside the five of
    ///      ENVELOPE_KEYS is refused (the store's reader calls it newer).
    ///   3. For an accepted line, parse(canonical) re-canonicalizes to the
    ///      same bytes.
    /// ```
    pub fn parse(raw: &[u8]) -> Result<JournalLine, String> {
        let read = read_line(raw)?;
        if read.unknown_keys.is_empty() {
            Ok(read.line)
        } else {
            Err(format!(
                "line fields this version doesn't know: {}",
                read.unknown_keys.join(", ")
            ))
        }
    }
}

/// One journal line as the store's readers take it in.
pub(crate) struct ReadLine {
    /// The line, without any key this version doesn't know.
    pub line: JournalLine,
    /// SHA-256 of the line's canonical JSON, every key included: what the
    /// next line's `prev` must be.
    pub hash: String,
    /// Keys beside [`ENVELOPE_KEYS`], sorted: none in any line this
    /// version writes.
    pub unknown_keys: Vec<String>,
}

/// Read one journal line (JournalLine::parse INVARIANT 1), keeping the
/// keys this version doesn't know out of the line but in its hash, so a
/// reader can still check the chain through it.
pub(crate) fn read_line(raw: &[u8]) -> Result<ReadLine, String> {
    let mut value: Value = serde_json::from_slice(raw).map_err(|e| e.to_string())?;
    if has_float(&value) {
        return Err("a number with a fraction or exponent; the journal holds none".into());
    }
    let hash = sha256_hex(canonical_json(&value).as_bytes());
    let mut unknown_keys = Vec::new();
    if let Value::Object(map) = &mut value {
        unknown_keys = map
            .keys()
            .filter(|k| !ENVELOPE_KEYS.contains(&k.as_str()))
            .cloned()
            .collect();
        unknown_keys.sort();
        for k in &unknown_keys {
            map.remove(k);
        }
    }
    let line = serde_json::from_value(value).map_err(|e| e.to_string())?;
    Ok(ReadLine {
        line,
        hash,
        unknown_keys,
    })
}

fn has_float(v: &Value) -> bool {
    match v {
        Value::Number(n) => n.is_f64(),
        Value::Array(a) => a.iter().any(has_float),
        Value::Object(m) => m.values().any(has_float),
        _ => false,
    }
}

/// Hex SHA-256 of bytes.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

/// Append-only writer. Assigns `seq`, stamps `ts`, chains `prev`, fsyncs
/// every append. The sequence only advances after a successful fsync, and a
/// failed write is truncated away, so an append is acknowledged only once
/// its whole line is on disk. A crash or power loss mid-append can still
/// leave a partial last line; it was never acknowledged, and
/// [`repair_torn_tail`] sets it aside when the books are next opened.
pub struct JournalWriter {
    path: PathBuf,
    file: File,
    next_seq: u64,
    prev_hash: String,
}

impl JournalWriter {
    /// Open an existing journal (verifying its chain) or create a new one.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let lines = if path.exists() {
            read_and_verify_journal(path)?
        } else {
            Vec::new()
        };
        Self::from_lines(path, &lines)
    }

    /// Build a writer from already-verified lines (avoids re-scanning).
    pub(crate) fn from_lines(path: &Path, lines: &[JournalLine]) -> Result<Self, StoreError> {
        let (next_seq, prev_hash) = match lines.last() {
            Some(last) => (last.seq + 1, sha256_hex(last.to_canonical().as_bytes())),
            None => (1, GENESIS_PREV.to_string()),
        };
        if !path.exists()
            && let Some(parent) = path.parent()
        {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(Self {
            path: path.to_path_buf(),
            file,
            next_seq,
            prev_hash,
        })
    }

    /// Append one event. Returns the journaled line.
    pub fn append(&mut self, line_type: &str, payload: Value) -> Result<JournalLine, StoreError> {
        let line = JournalLine {
            seq: self.next_seq,
            ts: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
            line_type: line_type.to_string(),
            payload,
            prev: self.prev_hash.clone(),
        };
        let bytes = line.to_canonical();
        // A write that fails partway (a full disk) must leave nothing
        // behind: cut the file back to where it was. Otherwise a partial
        // line stays, and every later line lands after it, mid-journal
        // (found by the full-disk fault test, 2026-10-13).
        let before = self.file.metadata()?.len();
        let written = self
            .file
            .write_all(bytes.as_bytes())
            .and_then(|()| self.file.write_all(b"\n"))
            .and_then(|()| self.file.sync_all());
        if let Err(e) = written {
            let _ = crate::chain::truncate(&self.file, &self.path, before);
            return Err(e.into());
        }
        self.prev_hash = sha256_hex(bytes.as_bytes());
        self.next_seq += 1;
        Ok(line)
    }

    /// The seq the next append will use.
    pub fn next_seq(&self) -> u64 {
        self.next_seq
    }

    /// The head of the chain: the last line's seq (0 for an empty
    /// journal) and its hash (the genesis value for an empty one).
    pub fn head(&self) -> (u64, &str) {
        (self.next_seq - 1, &self.prev_hash)
    }

    /// Path of the journal file.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Read and fully verify a journal: every line parses, seqs are gapless from
/// 1, and the hash chain is intact. Any failure names the offending seq.
/// This is the journal's own integrity, whatever its lines' types; opening
/// the books also passes every line through the format gate
/// ([`read_journal_and_format`]).
pub fn read_and_verify_journal(path: &Path) -> Result<Vec<JournalLine>, StoreError> {
    verify_lines(&std::fs::read(path)?, |_| Ok(()))
}

/// [`read_and_verify_journal`] with the format gate, also returning the
/// journal format in force.
///
/// ```text
/// GOAL: books read whole and verified, or refused with the reason.
/// SOURCE: docs/receipt-store.md "Line format"; rules.md B-27.
/// INVARIANTS:
///   1. Line by line, in order: parse, seq, chain, then the format gate.
///   2. Damage before the first line this version can't read is
///      JournalCorrupt, that line included: its seq and prev are checked
///      before the gate, and its own hash by the next line's prev
///      (format::settle), so a flipped bit in its type is corrupt, not
///      newer.
///   3. At the first whole, chained line this version can't read (an
///      unknown type, a newer journal.format_set, or a key beside the
///      envelope's): SHOULD stop with NewerJournal (verified when the next
///      line's prev matches it; unverified, saying "newer or damaged",
///      when there is no next line or it has no prev this version can
///      read)
///      BECAUSE a newer format may frame later lines in ways this version
///      can't check; judging them is the newer version's job.
///   4. The journal is read as bytes, split on newlines, as the viewer
///      reads it (crate::Snapshot): a line that isn't UTF-8 JSON is
///      JournalCorrupt, never an I/O error.
/// ```
pub fn read_journal_and_format(
    path: &Path,
) -> Result<(Vec<JournalLine>, JournalFormat), StoreError> {
    verify_journal_and_format(&std::fs::read(path)?)
}

/// [`read_journal_and_format`] over a journal's bytes (`raw`).
pub(crate) fn verify_journal_and_format(
    raw: &[u8],
) -> Result<(Vec<JournalLine>, JournalFormat), StoreError> {
    let mut format = JournalFormat::default();
    let lines = verify_lines(raw, |line| format.admit(line))?;
    Ok((lines, format))
}

/// Check each line of `raw` (bytes, split on newlines; a final newline
/// ends the last line): its parse, seq and chain, then `admit` on it, in
/// order; the first failure stops the read.
fn verify_lines(
    raw: &[u8],
    mut admit: impl FnMut(&JournalLine) -> Result<(), StoreError>,
) -> Result<Vec<JournalLine>, StoreError> {
    let mut lines = Vec::new();
    if raw.is_empty() {
        return Ok(lines);
    }
    let body = raw.strip_suffix(b"\n").unwrap_or(raw);
    let mut expected_prev = GENESIS_PREV.to_string();
    let mut raws = body.split(|&b| b == b'\n').enumerate().peekable();
    while let Some((i, raw)) = raws.next() {
        let read = read_line(raw).map_err(|e| {
            StoreError::JournalCorrupt(format!("seq {}: unparseable line: {e}", i + 1))
        })?;
        let line = &read.line;
        let want_seq = i as u64 + 1;
        if line.seq != want_seq {
            return Err(StoreError::JournalCorrupt(format!(
                "seq gap: expected {want_seq}, found {}",
                line.seq
            )));
        }
        if line.prev != expected_prev {
            return Err(StoreError::JournalCorrupt(format!(
                "hash chain broken at seq {}",
                line.seq
            )));
        }
        if let Err(refused) = crate::format::gate(&read, &mut admit) {
            // A line's own hash is checked only by the next line's prev:
            // read it before saying whether the books are newer or damaged.
            let next = raws.peek().map(|(_, next)| *next);
            return Err(crate::format::settle(refused, &read.hash, next));
        }
        expected_prev = read.hash;
        lines.push(read.line);
    }
    Ok(lines)
}

/// A torn final journal line, set aside when the books were opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TornTail {
    /// How many bytes followed the last complete line.
    pub bytes: u64,
    /// SHA-256 of those bytes.
    pub sha256: String,
    /// Where they were saved, beside the journal.
    pub saved_as: PathBuf,
}

/// Set aside a torn final line: bytes after the journal's last newline.
///
/// ```text
/// GOAL: open books whose last append was cut off (power loss, crash).
/// SOURCE: docs/receipt-store.md (a command is acknowledged only after its
///   whole line, newline included, is synced); the same rule write-ahead
///   logs use for a torn tail record (SQLite, PostgreSQL: recovery stops
///   at the first incomplete record).
/// SCENARIOS:
///   1. IF the journal ends with a newline: SHOULD change nothing.
///   2. IF bytes follow the last newline: SHOULD save them to a file
///      beside the journal, truncate the journal to the last newline, and
///      report them
///      BECAUSE [`JournalWriter::append`] syncs the newline before it
///      returns, so a line without one was never acknowledged: no one was
///      told it happened, and dropping it loses nothing.
///   3. NEVER touch a complete line: damage before the last newline is
///      corruption, and opening refuses it.
/// ```
pub fn repair_torn_tail(path: &Path) -> Result<Option<TornTail>, StoreError> {
    let raw = std::fs::read(path)?;
    let keep = raw.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
    if keep == raw.len() {
        return Ok(None);
    }
    let tail = &raw[keep..];
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%S%.fZ").to_string();
    let mut saved_as = path.as_os_str().to_owned();
    saved_as.push(format!(".torn.{stamp}"));
    let saved_as = PathBuf::from(saved_as);
    {
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&saved_as)?;
        f.write_all(tail)?;
        f.sync_all()?;
    }
    if let Some(dir) = path.parent()
        && let Ok(d) = File::open(dir)
    {
        let _ = d.sync_all();
    }
    let file = OpenOptions::new().write(true).open(path)?;
    file.set_len(keep as u64)?;
    file.sync_all()?;
    Ok(Some(TornTail {
        bytes: tail.len() as u64,
        sha256: sha256_hex(tail),
        saved_as,
    }))
}
