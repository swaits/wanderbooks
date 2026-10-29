//! Hash-chained logs beside the journal: a book's access log
//! (`books/<id>/access.jsonl`, plan I-9) and the root's principals
//! (`auth/principals.jsonl`, plan 3.3, 3.4).
//!
//! ```text
//! GOAL: a record, once acknowledged, is on disk and chained, and a later
//!       change to it, or to any record before it, is found when the log
//!       is read. (Whole records cut off the end, or the file deleted,
//!       leave a shorter chain that still verifies: only a head kept
//!       elsewhere finds that, plan I-10.)
//! SOURCE: plan 3.4 ("hash-chained access log, own genesis"), I-9
//!         ("logging fails closed"), I-10.
//! INVARIANTS:
//!   1. A log is lines of canonical JSON (journal::canonical_json), each
//!      an object whose `seq` is 1, 2, 3 … with no gap, whose `prev` is
//!      the SHA-256 of the line before (for the first, of the log's own
//!      genesis text, so one log never passes for another), whose `ts` is
//!      a string, and whose `hash` is the SHA-256 of the record without
//!      it: the last record, which no later `prev` covers, is checked too
//!      (against damage; a forger who rewrites a record's hash as well is
//!      found only by a head kept elsewhere, plan I-10).
//!   2. append returns Ok only once the whole line is written and
//!      synced. One that fails leaves the file cut back to where it was,
//!      and the log takes no more appends: a log whose state is in doubt
//!      is reopened (and so re-read) before it records anything else.
//!   3. read refuses a line that breaks 1, or isn't in canonical form
//!      byte for byte, naming its seq.
//!   4. Only an unfinished last line (no newline: an append cut short by
//!      a crash, never acknowledged) is set aside: open cuts it away,
//!      read reports it.
//! ```

use crate::journal::{canonical_json, sha256_hex};
use serde_json::{Map, Value};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Where a chain ends: its last record's seq (0 for an empty log) and the
/// SHA-256 the next record's `prev` must be.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Head {
    /// The last record's seq.
    pub seq: u64,
    /// The last record's hash (the genesis hash for an empty log).
    pub hash: String,
}

/// Why a log can't be used. Never names a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChainError {
    /// A record breaks the chain (INVARIANT 3).
    Damaged {
        /// The line, counting from 1.
        line: u64,
        /// What is wrong with it.
        why: String,
    },
    /// The disk refused.
    Io(std::io::ErrorKind),
    /// An earlier append failed: the log must be reopened (INVARIANT 2).
    Broken,
}

impl std::fmt::Display for ChainError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChainError::Damaged { line, why } => write!(f, "record {line} is damaged: {why}"),
            ChainError::Io(kind) => write!(f, "the disk refused: {kind}"),
            ChainError::Broken => f.write_str("an earlier record couldn't be written"),
        }
    }
}

impl std::error::Error for ChainError {}

impl From<std::io::Error> for ChainError {
    fn from(e: std::io::Error) -> Self {
        ChainError::Io(e.kind())
    }
}

/// The `prev` of a log's first record: the SHA-256 of its genesis text.
pub fn genesis(text: &str) -> String {
    sha256_hex(text.as_bytes())
}

/// A log read and checked.
#[derive(Debug, Clone)]
pub struct Read {
    /// Every record, in order, `seq`, `ts`, `prev` and `hash` included.
    pub records: Vec<Map<String, Value>>,
    /// Where the chain ends.
    pub head: Head,
    /// Whether an unfinished last line follows them (INVARIANT 4).
    pub torn: bool,
    /// How many bytes the whole records take: where a torn line begins.
    pub good_len: u64,
}

/// Read a log and check its chain (INVARIANTS 1, 3, 4). A log that
/// doesn't exist is empty.
pub fn read(path: &Path, genesis_text: &str) -> Result<Read, ChainError> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(e.into()),
    };
    let mut records = Vec::new();
    let mut prev = genesis(genesis_text);
    let mut at = 0usize;
    let mut torn = false;
    while at < bytes.len() {
        let Some(end) = bytes[at..].iter().position(|&b| b == b'\n') else {
            torn = true;
            break;
        };
        let line = &bytes[at..at + end];
        let n = records.len() as u64 + 1;
        let damaged = |why: &str| ChainError::Damaged {
            line: n,
            why: why.to_string(),
        };
        let value: Value =
            serde_json::from_slice(line).map_err(|_| damaged("it isn't a JSON record"))?;
        let canonical = canonical_json(&value);
        if canonical.as_bytes() != line {
            return Err(damaged("it was changed after it was written"));
        }
        let Value::Object(map) = value else {
            return Err(damaged("it isn't a JSON record"));
        };
        if map.get("seq").and_then(Value::as_u64) != Some(n) {
            return Err(damaged("its number is out of order"));
        }
        if map.get("prev").and_then(Value::as_str) != Some(prev.as_str()) {
            return Err(damaged("it doesn't follow the record before it"));
        }
        if !map.get("ts").is_some_and(Value::is_string) {
            return Err(damaged("it has no time"));
        }
        let mut body = map.clone();
        let own = body.remove("hash");
        if own.as_ref().and_then(Value::as_str)
            != Some(sha256_hex(canonical_json(&Value::Object(body)).as_bytes()).as_str())
        {
            return Err(damaged("it was changed after it was written"));
        }
        prev = sha256_hex(canonical.as_bytes());
        records.push(map);
        at += end + 1;
    }
    let head = Head {
        seq: records.len() as u64,
        hash: prev,
    };
    Ok(Read {
        records,
        head,
        torn,
        good_len: at as u64,
    })
}

/// A log open for appending.
pub struct ChainLog {
    path: PathBuf,
    file: File,
    head: Head,
    broken: bool,
}

impl std::fmt::Debug for ChainLog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChainLog")
            .field("head", &self.head)
            .field("broken", &self.broken)
            .finish_non_exhaustive()
    }
}

impl ChainLog {
    /// Open a log (made, private, if it doesn't exist), checking it whole
    /// and cutting away an unfinished last line (INVARIANT 4). Returns its
    /// records too.
    pub fn open(
        path: &Path,
        genesis_text: &str,
    ) -> Result<(Self, Vec<Map<String, Value>>), ChainError> {
        let read = read(path, genesis_text)?;
        #[cfg(unix)]
        let new = !path.exists();
        let mut options = OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(path)?;
        // One made by an older version, or loosened by hand: private again.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if file.metadata()?.permissions().mode() & 0o077 != 0 {
                file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
            }
        }
        #[cfg(windows)]
        if !crate::winsec::is_private(path).unwrap_or(false) {
            crate::winsec::set_owner_only(path)?;
        }
        if read.torn {
            file.set_len(read.good_len)?;
            file.sync_all()?;
        }
        // A log just made is in its folder for good (its entry synced).
        #[cfg(unix)]
        if new && let Some(dir) = path.parent() {
            File::open(dir)?.sync_all()?;
        }
        Ok((
            Self {
                path: path.to_path_buf(),
                file,
                head: read.head,
                broken: false,
            },
            read.records,
        ))
    }

    /// Where the chain ends now.
    pub fn head(&self) -> &Head {
        &self.head
    }

    /// The log's file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Append one record: `fields` and the chain's `seq`, `ts`, `prev`
    /// and `hash` (which `fields` must not hold). Returns the record as
    /// written and the new head (INVARIANT 2).
    pub fn append(&mut self, fields: Map<String, Value>) -> Result<(Value, Head), ChainError> {
        if self.broken {
            return Err(ChainError::Broken);
        }
        debug_assert!(
            !["seq", "ts", "prev", "hash"]
                .iter()
                .any(|k| fields.contains_key(*k)),
            "the chain sets seq, ts, prev and hash"
        );
        let mut record = fields;
        let seq = self.head.seq + 1;
        record.insert("seq".into(), Value::from(seq));
        record.insert(
            "ts".into(),
            Value::from(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)),
        );
        record.insert("prev".into(), Value::from(self.head.hash.clone()));
        let own = sha256_hex(canonical_json(&Value::Object(record.clone())).as_bytes());
        record.insert("hash".into(), Value::from(own));
        let record = Value::Object(record);
        let mut line = canonical_json(&record);
        let hash = sha256_hex(line.as_bytes());
        line.push('\n');
        let before = match self.file.metadata() {
            Ok(m) => m.len(),
            Err(e) => {
                self.broken = true;
                return Err(e.into());
            }
        };
        let written = self
            .file
            .write_all(line.as_bytes())
            .and_then(|()| self.file.sync_data());
        if let Err(e) = written {
            self.broken = true;
            let _ = self.file.set_len(before);
            let _ = self.file.sync_all();
            return Err(e.into());
        }
        self.head = Head { seq, hash };
        Ok((record, self.head.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const G: &str = "wanderbooks-test-v1";

    fn fields(n: u64) -> Map<String, Value> {
        let mut m = Map::new();
        m.insert("n".into(), Value::from(n));
        m.insert("word".into(), Value::from("café ✓"));
        m
    }

    #[test]
    fn appended_records_read_back_chained() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log.jsonl");
        let (mut log, records) = ChainLog::open(&path, G).unwrap();
        assert!(records.is_empty());
        assert_eq!(log.head().seq, 0);
        assert_eq!(log.head().hash, genesis(G));
        let mut heads = Vec::new();
        for n in 1..=3 {
            let (record, head) = log.append(fields(n)).unwrap();
            assert_eq!(record["seq"], n);
            assert_eq!(head.seq, n);
            heads.push(head);
        }
        let back = read(&path, G).unwrap();
        assert_eq!(back.records.len(), 3);
        assert_eq!(&back.head, heads.last().unwrap());
        assert!(!back.torn);
        assert_eq!(back.records[1]["prev"], heads[0].hash.as_str());
        assert_eq!(back.records[2]["word"], "café ✓");
        // Reopened, it carries on from the same head.
        drop(log);
        let (mut log, records) = ChainLog::open(&path, G).unwrap();
        assert_eq!(records.len(), 3);
        let (_, head) = log.append(fields(4)).unwrap();
        assert_eq!(head.seq, 4);
        assert_eq!(read(&path, G).unwrap().head, head);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
            // Loosened by hand: private again when next opened.
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
            drop(log);
            drop(ChainLog::open(&path, G).unwrap());
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn another_logs_genesis_doesnt_pass() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log.jsonl");
        let (mut log, _) = ChainLog::open(&path, G).unwrap();
        log.append(fields(1)).unwrap();
        let e = read(&path, "wanderbooks-other-v1").unwrap_err();
        assert!(matches!(e, ChainError::Damaged { line: 1, .. }), "{e}");
    }

    fn rewrite(path: &Path, f: impl FnOnce(&mut Vec<String>)) {
        let text = std::fs::read_to_string(path).unwrap();
        let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
        f(&mut lines);
        let mut out = lines.join("\n");
        out.push('\n');
        std::fs::write(path, out).unwrap();
    }

    fn three(dir: &Path) -> PathBuf {
        let path = dir.join("log.jsonl");
        let (mut log, _) = ChainLog::open(&path, G).unwrap();
        for n in 1..=3 {
            log.append(fields(n)).unwrap();
        }
        path
    }

    fn damaged_at(path: &Path) -> u64 {
        match read(path, G).unwrap_err() {
            ChainError::Damaged { line, .. } => line,
            e => panic!("{e}"),
        }
    }

    #[test]
    fn every_kind_of_change_is_found_at_its_line() {
        let dir = tempfile::tempdir().unwrap();
        // A field changed: the record no longer matches its own hash.
        let path = three(dir.path());
        rewrite(&path, |l| l[1] = l[1].replace("\"n\":2", "\"n\":9"));
        assert_eq!(damaged_at(&path), 2);
        // The last one too, which no later prev covers.
        let path = three(&dir.path().join("a").tap_mkdir());
        rewrite(&path, |l| l[2] = l[2].replace("\"n\":3", "\"n\":8"));
        assert_eq!(damaged_at(&path), 3);
        // A record and its own hash rewritten together: the next line's
        // prev catches it.
        let path = three(&dir.path().join("h").tap_mkdir());
        rewrite(&path, |l| {
            let mut v: Map<String, Value> = serde_json::from_str(&l[1]).unwrap();
            v.insert("n".into(), Value::from(9));
            v.remove("hash");
            let own = sha256_hex(canonical_json(&Value::Object(v.clone())).as_bytes());
            v.insert("hash".into(), Value::from(own));
            l[1] = canonical_json(&Value::Object(v));
        });
        assert_eq!(damaged_at(&path), 3);
        // A record dropped.
        let path = three(&dir.path().join("b").tap_mkdir());
        rewrite(&path, |l| {
            l.remove(1);
        });
        assert_eq!(damaged_at(&path), 2);
        // Records swapped.
        let path = three(&dir.path().join("c").tap_mkdir());
        rewrite(&path, |l| l.swap(0, 1));
        assert_eq!(damaged_at(&path), 1);
        // Not canonical (a space added).
        let path = three(&dir.path().join("d").tap_mkdir());
        rewrite(&path, |l| l[2] = l[2].replacen(':', ": ", 1));
        assert_eq!(damaged_at(&path), 3);
        // Not JSON.
        let path = three(&dir.path().join("e").tap_mkdir());
        rewrite(&path, |l| l[0] = "nonsense".into());
        assert_eq!(damaged_at(&path), 1);
        // Not an object.
        let path = three(&dir.path().join("f").tap_mkdir());
        rewrite(&path, |l| l[0] = "[1]".into());
        assert_eq!(damaged_at(&path), 1);
        // A record without its time, or without its own hash.
        for (sub, key) in [("g", "ts"), ("i", "hash")] {
            let path = three(&dir.path().join(sub).tap_mkdir());
            rewrite(&path, |l| {
                let mut v: Map<String, Value> = serde_json::from_str(&l[2]).unwrap();
                v.remove(key);
                l[2] = canonical_json(&Value::Object(v));
            });
            assert_eq!(damaged_at(&path), 3, "a record without its {key}");
        }
    }

    trait TapMkdir {
        fn tap_mkdir(self) -> Self;
    }
    impl TapMkdir for PathBuf {
        fn tap_mkdir(self) -> Self {
            std::fs::create_dir_all(&self).unwrap();
            self
        }
    }

    #[test]
    fn a_log_the_disk_wont_read_is_refused_not_taken_for_empty() {
        // A folder where the log should be: reading it fails, and an empty
        // log is never made of that.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log.jsonl");
        std::fs::create_dir(&path).unwrap();
        assert!(matches!(read(&path, G), Err(ChainError::Io(_))));
        assert!(matches!(ChainLog::open(&path, G), Err(ChainError::Io(_))));
    }

    #[test]
    fn a_log_shows_its_head_but_never_its_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = three(dir.path());
        let (log, _) = ChainLog::open(&path, G).unwrap();
        let shown = format!("{log:?}");
        assert!(shown.starts_with("ChainLog"), "{shown}");
        assert!(shown.contains("seq: 3"), "{shown}");
        assert!(shown.contains(&log.head().hash), "{shown}");
        assert!(shown.contains("broken: false"), "{shown}");
        assert!(
            !shown.contains(&dir.path().display().to_string()),
            "{shown}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn only_a_new_log_syncs_its_folder_and_only_others_bits_are_taken_away() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = three(dir.path());
        // The owner's own bits are the owner's: a log 0700 stays 0700.
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        drop(ChainLog::open(&path, G).unwrap());
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
        // A folder that can be entered but not listed: a log already in it
        // opens (its folder's entry for it was synced when it was made);
        // a new one can't be made there, as its entry can't be synced.
        let shut = dir.path().join("shut");
        std::fs::create_dir(&shut).unwrap();
        let (mut log, _) = ChainLog::open(&shut.join("old.jsonl"), G).unwrap();
        log.append(fields(1)).unwrap();
        drop(log);
        std::fs::set_permissions(&shut, std::fs::Permissions::from_mode(0o300)).unwrap();
        if File::open(&shut).is_err() {
            let (_, records) = ChainLog::open(&shut.join("old.jsonl"), G).unwrap();
            assert_eq!(records.len(), 1);
            assert_eq!(
                ChainLog::open(&shut.join("new.jsonl"), G).unwrap_err(),
                ChainError::Io(std::io::ErrorKind::PermissionDenied)
            );
        }
        std::fs::set_permissions(&shut, std::fs::Permissions::from_mode(0o700)).unwrap();
    }

    #[test]
    fn an_unfinished_last_line_is_cut_away_on_open() {
        let dir = tempfile::tempdir().unwrap();
        let path = three(dir.path());
        let whole = std::fs::read(&path).unwrap();
        let mut torn = whole.clone();
        torn.extend_from_slice(br#"{"n":4,"pr"#);
        std::fs::write(&path, &torn).unwrap();
        let r = read(&path, G).unwrap();
        assert!(r.torn);
        assert_eq!(r.records.len(), 3);
        assert_eq!(r.good_len, whole.len() as u64);
        let (mut log, records) = ChainLog::open(&path, G).unwrap();
        assert_eq!(records.len(), 3);
        assert_eq!(std::fs::read(&path).unwrap(), whole);
        let (_, head) = log.append(fields(4)).unwrap();
        assert_eq!(head.seq, 4);
        assert!(!read(&path, G).unwrap().torn);
    }

    #[test]
    fn a_damaged_log_isnt_opened() {
        let dir = tempfile::tempdir().unwrap();
        let path = three(dir.path());
        rewrite(&path, |l| l[0] = l[0].replace("\"n\":1", "\"n\":7"));
        assert!(matches!(
            ChainLog::open(&path, G).unwrap_err(),
            ChainError::Damaged { line: 1, .. }
        ));
    }

    #[test]
    fn errors_name_no_path() {
        for e in [
            ChainError::Damaged {
                line: 3,
                why: "x".into(),
            },
            ChainError::Io(std::io::ErrorKind::PermissionDenied),
            ChainError::Broken,
        ] {
            assert!(!e.to_string().contains('/'), "{e}");
        }
        // Each says what went wrong, and where in the log.
        assert_eq!(
            ChainError::Damaged {
                line: 3,
                why: "its number is out of order".into(),
            }
            .to_string(),
            "record 3 is damaged: its number is out of order"
        );
        assert_eq!(
            ChainError::Io(std::io::ErrorKind::PermissionDenied).to_string(),
            "the disk refused: permission denied"
        );
        assert_eq!(
            ChainError::Broken.to_string(),
            "an earlier record couldn't be written"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_failed_append_leaves_the_log_whole_and_takes_no_more() {
        // /dev/full refuses every write: an append there fails, and the
        // log won't take another until reopened.
        let file = OpenOptions::new().append(true).open("/dev/full").unwrap();
        let mut log = ChainLog {
            path: PathBuf::from("/dev/full"),
            file,
            head: Head {
                seq: 0,
                hash: genesis(G),
            },
            broken: false,
        };
        assert!(matches!(log.append(fields(1)), Err(ChainError::Io(_))));
        assert_eq!(log.head().seq, 0, "the head didn't move");
        assert_eq!(log.append(fields(1)).unwrap_err(), ChainError::Broken);
    }
}
