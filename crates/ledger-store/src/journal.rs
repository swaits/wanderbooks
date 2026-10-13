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
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
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
    /// Event type: `account.added`, `entry.created`, `entry.line_added`,
    /// `entry.posted`, `entry.reversed`, `entry.line_class_set`,
    /// `receipt.attached`, `receipt.linked`, `receipt.unlinked`,
    /// `document.filed`, `attachment.withdrawn`,
    /// `asset.registered`, `asset.depreciation_posted`, `asset.disposed`,
    /// `period.created`, `period.closed`, `line.cleared`, `customer.added`,
    /// `vendor.added`, `invoice.created`, `bill.created`,
    /// `invoice.payment_received`, `bill.paid`, `fx.rate_set`,
    /// `payroll.run`, `class.added`, `store.created`.
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

impl JournalLine {
    /// Parse one journal line, refusing what the journal never holds.
    ///
    /// ```text
    /// GOAL: read a journal line so that its canonical form (which the
    ///       hash chain covers) is a fixed point.
    /// SOURCE: R-2 (no floats in money paths); found by fuzz_journal_line
    ///         (2026-10-13): a float payload re-canonicalized differently.
    /// INVARIANTS:
    ///   1. NEVER accepts a floating-point number anywhere in the line:
    ///      amounts are decimal strings and ids are integers.
    ///   2. For an accepted line, parse(canonical) re-canonicalizes to the
    ///      same bytes.
    /// ```
    pub fn parse(raw: &[u8]) -> Result<JournalLine, String> {
        let value: Value = serde_json::from_slice(raw).map_err(|e| e.to_string())?;
        if has_float(&value) {
            return Err("a number with a fraction or exponent; the journal holds none".into());
        }
        serde_json::from_value(value).map_err(|e| e.to_string())
    }
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
/// every append. The sequence only advances after a successful fsync, so a
/// torn tail can never exist in a file written by this writer.
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
            let _ = self.file.set_len(before);
            let _ = self.file.sync_all();
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

    /// Path of the journal file.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Read and fully verify a journal: every line parses, seqs are gapless from
/// 1, and the hash chain is intact. Any failure names the offending seq.
pub fn read_and_verify_journal(path: &Path) -> Result<Vec<JournalLine>, StoreError> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut lines = Vec::new();
    let mut expected_prev = GENESIS_PREV.to_string();
    for (i, raw) in reader.lines().enumerate() {
        let raw = raw?;
        let line: JournalLine = JournalLine::parse(raw.as_bytes()).map_err(|e| {
            StoreError::JournalCorrupt(format!("seq {}: unparseable line: {e}", i + 1))
        })?;
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
        expected_prev = sha256_hex(line.to_canonical().as_bytes());
        lines.push(line);
    }
    Ok(lines)
}

/// Truncate a journal file to its last good line. For recovering *copies*
/// with a torn tail (e.g. a backup taken mid-write): the projection never
/// saw the torn line, so dropping it is safe. Returns the last good seq.
/// The live journal written by [`JournalWriter`] cannot have a torn tail.
pub fn truncate_to_last_good_seq(path: &Path) -> Result<u64, StoreError> {
    let raw = std::fs::read(path)?;
    let mut good_len = 0usize;
    let mut last_good_seq = 0u64;
    let mut expected_prev = GENESIS_PREV.to_string();
    // Walk raw bytes: a line is bytes up to and including b'\n'. Trailing
    // bytes without a newline are a torn tail and are dropped.
    let mut start = 0usize;
    for (i, &b) in raw.iter().enumerate() {
        if b != b'\n' {
            continue;
        }
        let line_bytes = &raw[start..i];
        start = i + 1;
        let line_str = std::str::from_utf8(line_bytes).map_err(|_| {
            StoreError::JournalCorrupt(format!("seq {}: invalid utf-8", last_good_seq + 1))
        })?;
        if line_str.trim().is_empty() {
            break; // blank line: our writer never emits these
        }
        match JournalLine::parse(line_str.as_bytes()) {
            Ok(line) if line.seq == last_good_seq + 1 && line.prev == expected_prev => {
                expected_prev = sha256_hex(line.to_canonical().as_bytes());
                last_good_seq = line.seq;
                good_len = i + 1; // include the newline
            }
            _ => break,
        }
    }
    let file = OpenOptions::new().write(true).open(path)?;
    file.set_len(good_len as u64)?;
    file.sync_all()?;
    Ok(last_good_seq)
}
