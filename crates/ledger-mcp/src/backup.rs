//! The server's backups of a book (plan 3.7, D-30, D-31): made a while
//! after the books are written (when the writing stops), when they open,
//! before they close, and when asked; each pushed to the targets the
//! book's settings name; each recorded in the book's `ops/backup.jsonl`;
//! each target's backup drilled weekly.
//!
//! ```text
//! GOAL: every write reaches the owner's backups soon after it is made,
//!       and the owner (and agent) can see that it did, or why not.
//! SOURCE: plan 3.7 (triggers, "ops/backup.jsonl", books.backup, Needs
//!         attention for a target more than 24 h behind, drills), D-30 (a
//!         git repository and a folder), D-31 (plain git).
//! INVARIANTS:
//!   1. A backup is committed under the store's lock (ledger_store::backup
//!      INVARIANT 1; its stored files staged before, outside it) and
//!      pushed outside it: the books are never held while the network is
//!      slow. One run or drill at a time.
//!   2. Every commit, push and drill is a record of `ops/backup.jsonl` (a
//!      chain log, not the journal: a backup never moves the books' head),
//!      each target's outcome a record of its own, keyed by the target
//!      itself (a new folder or address starts with no standing).
//!   3. A target is stale when it lacks the books' head and the first try
//!      to back up what it lacks (the first commit or failure since its
//!      last good push, or since its first try if it never had one, or
//!      since the failed commits that kept it from any) is more than
//!      STALE_AFTER old; its last error is its last push's, or a
//!      later failed commit's.
//!   4. Nothing here names a path of the data root to an agent.
//!   5. Every target that holds a backup is drilled (ledger_store::backup::
//!      drill: cloned, proven whole, its journal the live one's beginning)
//!      once DRILL_EVERY has passed since its last good drill, or
//!      RETRY_AFTER since a failed one (at the server's next look for
//!      drills); a failed drill is said in the standing until one passes.
//!   6. A target whose push failed lately (RETRY_AFTER) is left out of a
//!      backup after a write, and only that one; a folder target's
//!      repository is made only while the target never took a backup of
//!      these books (one gone since is a disk not connected), restored
//!      books counting the backups of the books they were.
//!   7. Books restored beside the books they were (under another id) never
//!      push to the repository those books back up to (it would stop
//!      theirs): it is paused for them, and said, until another is named.
//!   8. A damaged backup record never keeps the books shut: it is set
//!      aside and a new one begun.
//! ```

use crate::server::SharedStore;
use ledger_store::backup::{self, BackupError, Git, Target};
use ledger_store::chain::ChainLog;
use serde::Serialize;
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// The book's backup record, in its `ops/`.
pub const LOG_FILE: &str = "backup.jsonl";
/// The record's genesis text.
const GENESIS: &str = "wanderbooks-backup-v1";
/// How long a target may lack the books' head before it is stale.
pub const STALE_AFTER: chrono::Duration = chrono::Duration::hours(24);
/// How often each target's backup is drilled (plan 3.7: weekly).
pub const DRILL_EVERY: chrono::Duration = chrono::Duration::days(7);
/// How long after a failed run (or drill) the next is tried.
pub const RETRY_AFTER: std::time::Duration = std::time::Duration::from_secs(15 * 60);
/// The most of an error kept.
const ERROR_MAX: usize = 500;

/// A book's backups.
pub struct Backups {
    book: String,
    dir: PathBuf,
    store: Arc<SharedStore>,
    log: Mutex<ChainLog>,
    /// One run at a time.
    running: tokio::sync::Mutex<()>,
}

/// One target's standing (INVARIANT 3).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TargetStatus {
    /// "repository" or "folder".
    pub target: String,
    /// Where (the repository's address, or "the backup folder").
    pub at: String,
    /// When it last took a push, if ever (UTC).
    pub last_ok_at: Option<String>,
    /// The journal seq it holds, if any.
    pub through_seq: Option<u64>,
    /// Whether it is stale (INVARIANT 3).
    pub stale: bool,
    /// Why its last backup failed, if it did.
    pub last_error: Option<String>,
    /// When its backup was last drilled (UTC), if ever.
    pub last_drill_at: Option<String>,
    /// Why its last drill failed, if it did.
    pub drill_error: Option<String>,
    /// Paused for books restored as a copy of others (INVARIANT 7).
    pub paused: bool,
}

/// The books' backup standing.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Status {
    /// The books' head now.
    pub head_seq: u64,
    /// Each target the settings name.
    pub targets: Vec<TargetStatus>,
    /// None set up: what to do.
    pub note: Option<String>,
}

/// A target the settings name.
#[derive(Debug, Clone)]
struct Named {
    target: Target,
    /// Where, as an agent is told it.
    at: String,
    /// The target itself, as its records name it (INVARIANT 2): a digest
    /// of where it is (never a path an agent is shown).
    key: String,
}

/// The targets the book's settings name (D-30).
fn targets(store: &ledger_store::Store, book: &str) -> Vec<Named> {
    use sha2::{Digest, Sha256};
    let s = store.ledger().settings();
    let mut out = Vec::new();
    if let Some(url) = s.backup_git_remote() {
        out.push((Target::Remote(url.to_string()), url.to_string()));
    }
    if let Some(folder) = s.backup_folder() {
        out.push((
            Target::Folder(PathBuf::from(folder)),
            "the backup folder".to_string(),
        ));
    }
    out.into_iter()
        .map(|(target, at)| Named {
            key: Sha256::digest(target.source(book).as_bytes())[..8]
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect(),
            target,
            at,
        })
        .collect()
}

impl Backups {
    /// The backups of the book `book`, kept in `dir`, served from `store`.
    pub fn open(book: &str, dir: &Path, store: Arc<SharedStore>) -> Result<Self, String> {
        ledger_store::ops::prepare(dir)
            .map_err(|e| format!("its backup record can't be kept: {e}"))?;
        let path = ledger_store::ops::ops_dir(dir).join(LOG_FILE);
        let (log, carried) = match ChainLog::open(&path, GENESIS) {
            Ok((log, _)) => (log, Vec::new()),
            // INVARIANT 8: set aside, begun again, what it knew of whose
            // backups the targets hold carried into the new one.
            Err(ledger_store::chain::ChainError::Damaged { .. }) => {
                let known = known_before_damage(&path);
                let aside = path.with_extension(format!(
                    "jsonl.damaged-{}",
                    chrono::Utc::now().format("%Y%m%d-%H%M%S")
                ));
                eprintln!(
                    "wanderbooks server: a backup record was damaged: set aside, begun again"
                );
                std::fs::rename(&path, &aside)
                    .map_err(|e| format!("its backup record can't be set aside: {}", e.kind()))?;
                let log = ChainLog::open(&path, GENESIS)
                    .map_err(|e| format!("its backup record can't be used: {e}"))?
                    .0;
                (log, known)
            }
            Err(e) => return Err(format!("its backup record can't be used: {e}")),
        };
        let me = Self {
            book: book.to_string(),
            dir: dir.to_path_buf(),
            store,
            log: Mutex::new(log),
            running: tokio::sync::Mutex::new(()),
        };
        for fields in carried {
            me.record(fields);
        }
        // Books just restored: whose they were (INVARIANTS 6, 7).
        let marker = ledger_store::ops::ops_dir(dir).join(ledger_store::root::RESTORED_FROM);
        if let Ok(origin) = std::fs::read_to_string(&marker) {
            let origin = Some(origin.trim()).filter(|o| !o.is_empty());
            let named = me
                .store
                .try_with_store(|s| targets(s, book))
                .ok_or("its backups can't be read yet: the books are busy")?;
            me.restored(origin, &named);
            std::fs::remove_file(&marker)
                .map_err(|e| format!("its backup record can't be kept: {}", e.kind()))?;
        }
        Ok(me)
    }

    fn record(&self, fields: Value) {
        let Value::Object(mut map) = fields else {
            return;
        };
        // INVARIANT 4: a git or store message may quote a path in the root.
        if let Some(Value::String(e)) = map.get_mut("error") {
            *e = hide(e, &self.dir);
        }
        let mut log = self.log.lock().unwrap_or_else(|e| e.into_inner());
        if let Err(e) = log.append(map) {
            eprintln!("wanderbooks server: the books' backup record couldn't be written: {e}");
        }
    }

    fn records(&self) -> Vec<Map<String, Value>> {
        ledger_store::chain::read(
            &ledger_store::ops::ops_dir(&self.dir).join(LOG_FILE),
            GENESIS,
        )
        .map(|r| r.records)
        .unwrap_or_default()
    }

    /// The targets the settings name now.
    async fn named(&self) -> Vec<Named> {
        let (store, book) = (self.store.clone(), self.book.clone());
        tokio::task::spawn_blocking(move || store.with_store(|s| targets(s, &book)).ok())
            .await
            .ok()
            .flatten()
            .unwrap_or_default()
    }

    /// Note that these books were just restored from a backup of the books
    /// `origin` (INVARIANTS 6, 7), `named` the targets they name: the same
    /// books (their targets took their backups), or others' (the
    /// repository those back up to is paused for these).
    fn restored(&self, origin: Option<&str>, named: &[Named]) {
        let same = origin == Some(self.book.as_str());
        let keys = |remote: bool| -> Vec<String> {
            named
                .iter()
                .filter(|n| !remote || matches!(n.target, Target::Remote(_)))
                .map(|n| n.key.clone())
                .collect()
        };
        if same {
            self.record(json!({"kind": "restored", "origin": origin, "held": keys(false)}));
        } else {
            self.record(json!({"kind": "restored", "origin": origin, "paused": keys(true)}));
        }
    }

    /// Wait for any run or drill to end, and start none while the guard
    /// lives: books being replaced by a restore push nothing more.
    pub async fn pause(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.running.lock().await
    }

    /// The books' head now.
    pub async fn head(&self) -> Option<(u64, String)> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || store.with_store(|s| s.head()).ok())
            .await
            .ok()
            .flatten()
    }

    /// Back the books up now (INVARIANTS 1, 2): `why` is recorded (a
    /// write, open, close, asked). The status after.
    pub async fn run(&self, why: &str) -> Result<Status, String> {
        self.run_with(why, false).await
    }

    /// [`Self::run`]; with `new_disk`, a folder target that took these
    /// books' backups and holds none now is begun again there (the owner
    /// says it is a new disk).
    pub async fn run_with(&self, why: &str, new_disk: bool) -> Result<Status, String> {
        let _one = self.running.lock().await;
        if self.named().await.is_empty() {
            return Ok(self.status().await);
        }
        let (store, dir, book) = (self.store.clone(), self.dir.clone(), self.book.clone());
        let made = tokio::task::spawn_blocking(move || {
            let git = Git::find()?;
            // The stored files first, outside the lock: they never change.
            // Only a head start (a file being written can trip it): the
            // commit stages them all again.
            let _ = backup::stage_blobs(&git, &dir);
            // The commit, under it.
            store
                .with_store(|s| -> Result<_, BackupError> {
                    let made = backup::commit(&git, &dir, &book, s.head())?;
                    Ok((made, targets(s, &book), s.head()))
                })
                .map_err(BackupError::Damaged)
                .and_then(|r| r)
        })
        .await
        .unwrap_or(Err(BackupError::Damaged("the backup stopped".into())));
        let (made, named, head) = match made {
            Ok(m) => m,
            Err(e) => {
                let e = hide(&e.to_string(), &self.dir);
                self.record(json!({"kind": "commit", "why": why, "ok": false, "error": e}));
                return Err(e);
            }
        };
        if let Some(m) = &made {
            self.record(
                json!({"kind": "commit", "why": why, "ok": true, "commit": m.commit,
                "head": {"seq": m.head.0, "hash": m.head.1}}),
            );
        }
        // Each target, outside the lock; after a write, one that failed
        // lately waits out RETRY_AFTER (the others don't wait for it).
        let records = self.records();
        let now = chrono::Utc::now();
        for n in named {
            if paused(&n.key, &records) || (why == "write" && failed_lately(&n.key, &records, now))
            {
                continue;
            }
            // Its repository is made only where it never took a backup.
            let may_create = new_disk || !held_before(&n.key, &records);
            let (dir, book, t) = (self.dir.clone(), self.book.clone(), n.target.clone());
            let pushed = tokio::task::spawn_blocking(move || {
                let git = Git::find()?;
                backup::push(&git, &dir, &book, &t, may_create)
            })
            .await
            .unwrap_or(Err(BackupError::Damaged("the push stopped".into())));
            match pushed {
                Ok(commit) => self.record(
                    json!({"kind": "push", "why": why, "target": n.target.kind(),
                    "at": n.at, "key": n.key, "ok": true, "commit": commit,
                    "head": {"seq": head.0, "hash": head.1}}),
                ),
                Err(e) => self.record(
                    json!({"kind": "push", "why": why, "target": n.target.kind(),
                    "at": n.at, "key": n.key, "ok": false, "error": e.to_string()}),
                ),
            }
        }
        Ok(self.status().await)
    }

    /// Whether a drill is due (INVARIANT 5).
    pub async fn drill_due(&self) -> bool {
        let records = self.records();
        let now = chrono::Utc::now();
        self.named()
            .await
            .iter()
            .any(|n| drill_due(&n.key, &records, now))
    }

    /// Drill each target whose drill is due (INVARIANT 5), each recorded.
    pub async fn drill(&self) {
        let _one = self.running.lock().await;
        let records = self.records();
        let now = chrono::Utc::now();
        for n in self.named().await {
            if paused(&n.key, &records) || !drill_due(&n.key, &records, now) {
                continue;
            }
            let (dir, book, t) = (self.dir.clone(), self.book.clone(), n.target.clone());
            let drilled = tokio::task::spawn_blocking(move || {
                let scratch = ledger_store::ops::ops_dir(&dir).join(format!("drill-{}", t.kind()));
                let _ = std::fs::remove_dir_all(&scratch);
                let git = Git::find()?;
                backup::drill(
                    &git,
                    &t,
                    &book,
                    &dir.join(ledger_store::JOURNAL_FILE),
                    &scratch,
                )
            })
            .await
            .unwrap_or(Err(BackupError::Damaged("the drill stopped".into())));
            match drilled {
                Ok(head) => {
                    self.record(json!({"kind": "drill", "target": n.target.kind(),
                    "at": n.at, "key": n.key, "ok": true, "head": {"seq": head.0, "hash": head.1}}))
                }
                Err(e) => self.record(json!({"kind": "drill", "target": n.target.kind(),
                    "at": n.at, "key": n.key, "ok": false, "error": e.to_string()})),
            }
        }
    }

    /// The books' backup standing (INVARIANT 3).
    pub async fn status(&self) -> Status {
        let store = self.store.clone();
        let (book, dir) = (self.book.clone(), self.dir.clone());
        tokio::task::spawn_blocking(move || store.with_store(|s| standing(s, &book, &dir)).ok())
            .await
            .ok()
            .flatten()
            .unwrap_or_else(|| standing_of(&[], &[], 0, chrono::Utc::now()))
    }

    /// The standing, read from `store` (held by the caller: books.start).
    pub fn standing(&self, store: &ledger_store::Store) -> Status {
        standing(store, &self.book, &self.dir)
    }
}

fn standing(store: &ledger_store::Store, book: &str, dir: &Path) -> Status {
    let records =
        ledger_store::chain::read(&ledger_store::ops::ops_dir(dir).join(LOG_FILE), GENESIS)
            .map(|r| r.records)
            .unwrap_or_default();
    standing_of(
        &targets(store, book),
        &records,
        store.head().0,
        chrono::Utc::now(),
    )
}

fn standing_of(
    named: &[Named],
    records: &[Map<String, Value>],
    head_seq: u64,
    now: chrono::DateTime<chrono::Utc>,
) -> Status {
    Status {
        head_seq,
        note: named.is_empty().then(|| {
            "no backups are set up: set backup.git_remote (a private repository) and backup.folder (a folder on another disk) with books.set_setting".to_string()
        }),
        targets: named
            .iter()
            .map(|n| status_of(n.target.kind(), &n.at, &n.key, records, head_seq, now))
            .collect(),
    }
}

impl Status {
    /// Whether the human should hear of it: none set up, or one stale,
    /// failing, or failing its drill.
    pub fn needs_attention(&self) -> bool {
        self.note.is_some()
            || self
                .targets
                .iter()
                .any(|t| t.stale || t.last_error.is_some() || t.drill_error.is_some())
    }
}

/// `text` with the book's folder `dir`, and the data root above it, named
/// in place of their paths (INVARIANT 4); at most ERROR_MAX characters.
fn hide(text: &str, dir: &Path) -> String {
    let mut text = text.to_string();
    let real = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    for d in [dir, real.as_path()] {
        text = text.replace(&d.display().to_string(), "the book's folder");
        if let Some(root) = d.parent().and_then(Path::parent) {
            text = text.replace(&root.display().to_string(), ledger_store::root::THE_ROOT);
        }
    }
    text.chars().take(ERROR_MAX).collect()
}

fn is(r: &Map<String, Value>, kind: &str) -> bool {
    r.get("kind").and_then(Value::as_str) == Some(kind)
}

fn ok(r: &Map<String, Value>) -> bool {
    r.get("ok") == Some(&Value::Bool(true))
}

fn of(r: &Map<String, Value>, key: &str) -> bool {
    r.get("key").and_then(Value::as_str) == Some(key)
}

fn ts(r: &Map<String, Value>) -> Option<chrono::DateTime<chrono::Utc>> {
    r.get("ts")
        .and_then(Value::as_str)
        .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
        .map(|t| t.with_timezone(&chrono::Utc))
}

fn text(r: &Map<String, Value>, field: &str) -> Option<String> {
    r.get(field).and_then(Value::as_str).map(str::to_string)
}

/// What a damaged record knew, up to its first line that can't be read at
/// all (a line read but out of the chain still counts: what it adds only
/// holds more back), of whose backups the
/// targets hold: its restored notes, and each target that took a backup
/// (as held).
fn known_before_damage(path: &Path) -> Vec<Value> {
    let text = std::fs::read(path).unwrap_or_default();
    let mut held: Vec<String> = Vec::new();
    let mut out = Vec::new();
    for line in text.split(|b| *b == b'\n') {
        let Ok(Value::Object(r)) = serde_json::from_slice::<Value>(line) else {
            break;
        };
        if is(&r, "push") && ok(&r) {
            if let Some(k) = text_field(&r, "key")
                && !held.contains(&k)
            {
                held.push(k);
            }
        } else if is(&r, "restored") {
            let mut r = r.clone();
            for k in ["seq", "ts", "prev", "hash"] {
                r.remove(k);
            }
            out.push(Value::Object(r));
        }
    }
    if !held.is_empty() {
        out.push(json!({"kind": "restored", "carried": true, "held": held}));
    }
    out
}

fn text_field(r: &Map<String, Value>, field: &str) -> Option<String> {
    r.get(field).and_then(Value::as_str).map(str::to_string)
}

/// Whether the target `key` is paused for books restored beside others
/// (INVARIANT 7).
fn paused(key: &str, records: &[Map<String, Value>]) -> bool {
    records.iter().any(|r| {
        is(r, "restored")
            && r.get("paused")
                .and_then(Value::as_array)
                .is_some_and(|keys| keys.iter().any(|k| k.as_str() == Some(key)))
    })
}

/// Whether the target `key` took a backup of these books (INVARIANT 6).
fn held_before(key: &str, records: &[Map<String, Value>]) -> bool {
    records.iter().any(|r| {
        (is(r, "push") && of(r, key) && ok(r))
            || (is(r, "restored")
                && r.get("held")
                    .and_then(Value::as_array)
                    .is_some_and(|keys| keys.iter().any(|k| k.as_str() == Some(key))))
    })
}

/// What books restored beside others are told of the repository paused
/// for them.
pub const PAUSED: &str = "not backed up here: these books were restored as a copy of the books this repository keeps, and backing up here would stop theirs; name a repository of their own (backup.git_remote)";

/// Whether the target `key`'s last push failed less than RETRY_AFTER ago.
fn failed_lately(
    key: &str,
    records: &[Map<String, Value>],
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    records
        .iter()
        .rfind(|r| is(r, "push") && of(r, key))
        .filter(|r| !ok(r))
        .and_then(ts)
        .is_some_and(|at| {
            now.signed_duration_since(at)
                < chrono::Duration::from_std(RETRY_AFTER).unwrap_or(chrono::Duration::zero())
        })
}

/// Whether the target `key`'s drill is due (INVARIANT 5).
fn drill_due(
    key: &str,
    records: &[Map<String, Value>],
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    if !records.iter().any(|r| is(r, "push") && of(r, key) && ok(r)) {
        return false;
    }
    match records.iter().rfind(|r| is(r, "drill") && of(r, key)) {
        None => true,
        Some(d) => ts(d).is_none_or(|at| {
            let wait = if ok(d) {
                DRILL_EVERY
            } else {
                chrono::Duration::from_std(RETRY_AFTER).unwrap_or(DRILL_EVERY)
            };
            now.signed_duration_since(at) >= wait
        }),
    }
}

/// A target's standing from the backup record (INVARIANT 3).
fn status_of(
    kind: &str,
    at: &str,
    key: &str,
    records: &[Map<String, Value>],
    head_seq: u64,
    now: chrono::DateTime<chrono::Utc>,
) -> TargetStatus {
    let last_ok = records
        .iter()
        .rposition(|r| is(r, "push") && of(r, key) && ok(r));
    let through_seq = last_ok
        .and_then(|i| records[i].get("head"))
        .and_then(|h| h.get("seq"))
        .and_then(Value::as_u64);
    // Since its last good push (or, never having had one, since its first
    // try; or, never tried, the failed commits that kept it from one):
    // this target's tries, and the commits.
    let from = match last_ok {
        Some(i) => i + 1,
        None => records.iter().position(|r| of(r, key)).unwrap_or_else(|| {
            records
                .iter()
                .rposition(|r| !(is(r, "commit") && !ok(r)))
                .map_or(0, |i| i + 1)
        }),
    };
    let since: Vec<&Map<String, Value>> = records[from..]
        .iter()
        .filter(|r| is(r, "commit") || (is(r, "push") && of(r, key)))
        .collect();
    let behind = through_seq.is_none_or(|s| s < head_seq);
    let old = since
        .first()
        .and_then(|r| ts(r))
        .is_some_and(|t| now.signed_duration_since(t) > STALE_AFTER);
    let last_error = since
        .iter()
        .rev()
        .find(|r| !ok(r) || is(r, "push"))
        .filter(|r| !ok(r))
        .and_then(|r| text(r, "error"));
    let drill = records.iter().rfind(|r| is(r, "drill") && of(r, key));
    let paused = paused(key, records);
    TargetStatus {
        target: kind.to_string(),
        at: at.to_string(),
        last_ok_at: last_ok.and_then(|i| text(&records[i], "ts")),
        through_seq,
        stale: behind && old && head_seq > 0 && !paused,
        last_error: if paused {
            Some(PAUSED.to_string())
        } else {
            last_error
        },
        last_drill_at: drill.and_then(|r| text(r, "ts")),
        drill_error: drill.filter(|r| !ok(r)).and_then(|r| text(r, "error")),
        paused,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_error_names_the_root() {
        let dir = Path::new("/home/u/.local/share/wanderbooks/books/acme");
        assert_eq!(
            hide(
                "fatal: '/home/u/.local/share/wanderbooks/books/acme/archive.git' and /home/u/.local/share/wanderbooks/run",
                dir
            ),
            format!(
                "fatal: 'the book's folder/archive.git' and {}/run",
                ledger_store::root::THE_ROOT
            )
        );
        assert_eq!(hide(&"x".repeat(2000), dir).len(), ERROR_MAX);
    }

    /// A record of `kind` for target `key`, `ago` before `now`.
    fn rec(
        now: chrono::DateTime<chrono::Utc>,
        kind: &str,
        key: &str,
        ok: bool,
        seq: u64,
        ago: chrono::Duration,
    ) -> Map<String, Value> {
        let mut m = Map::new();
        m.insert("kind".into(), json!(kind));
        if kind != "commit" {
            m.insert("key".into(), json!(key));
        }
        m.insert("ok".into(), json!(ok));
        m.insert("ts".into(), json!((now - ago).to_rfc3339()));
        if ok {
            m.insert("head".into(), json!({"seq": seq, "hash": "h"}));
        } else {
            m.insert("error".into(), json!(format!("{kind} failed")));
        }
        m
    }

    #[test]
    fn a_target_is_stale_only_when_behind_since_a_day() {
        let now = chrono::Utc::now();
        let r = |kind, ok, seq, ago| rec(now, kind, "k", ok, seq, ago);
        let st = |records: &[Map<String, Value>], head| {
            status_of("folder", "f", "k", records, head, now)
        };
        let day = chrono::Duration::hours(24);
        let hour = chrono::Duration::hours(1);
        // Nothing tried yet: not stale.
        assert!(!st(&[], 0).stale);
        assert!(!st(&[], 4).stale);
        // Never pushed, the first try a day ago: stale; an hour ago: not.
        assert!(
            st(
                &[
                    r("commit", true, 3, day + hour),
                    r("push", false, 0, day + hour)
                ],
                3
            )
            .stale
        );
        assert!(!st(&[r("commit", true, 3, hour), r("push", false, 0, hour)], 3).stale);
        // Pushed long ago, and quiet since: a new write isn't stale at once.
        let quiet = [r("commit", true, 3, day * 3), r("push", true, 3, day * 3)];
        assert!(!st(&quiet, 4).stale);
        // ... nor while it holds the head.
        assert!(!st(&quiet, 3).stale);
        // Behind since a commit a day old, its pushes failing: stale.
        let failing = [
            r("push", true, 3, day * 3),
            r("commit", true, 4, day + hour),
            r("push", false, 0, day),
            r("push", false, 0, hour),
        ];
        let t = st(&failing, 4);
        assert!(t.stale);
        assert_eq!(
            (t.through_seq, t.last_error.as_deref()),
            (Some(3), Some("push failed"))
        );
        // A good push ends the error.
        let mended = [
            &failing[..],
            &[r("push", true, 4, chrono::Duration::zero())],
        ]
        .concat();
        let t = st(&mended, 4);
        assert_eq!(
            (t.stale, t.last_error, t.through_seq),
            (false, None, Some(4))
        );
        // A failed commit is every target's error, until a good push.
        let t = st(&[r("push", true, 3, day), r("commit", false, 0, hour)], 4);
        assert_eq!(t.last_error.as_deref(), Some("commit failed"));
        // A good commit after a failed push doesn't hide the failure.
        let t = st(&[r("push", false, 0, hour), r("commit", true, 4, hour)], 4);
        assert_eq!(t.last_error.as_deref(), Some("push failed"));
        // Another target's records are not its: a new folder starts clean.
        let other = [
            rec(now, "push", "other", true, 3, hour),
            rec(now, "push", "other", false, 0, hour),
        ];
        let t = st(&other, 3);
        assert_eq!(
            (t.through_seq, t.last_error, t.last_ok_at),
            (None, None, None)
        );
    }

    #[test]
    fn a_target_failing_lately_is_left_out_of_a_write_alone() {
        let now = chrono::Utc::now();
        let retry = chrono::Duration::from_std(RETRY_AFTER).unwrap();
        let second = chrono::Duration::seconds(1);
        let failed = |ago| rec(now, "push", "k", false, 0, ago);
        assert!(failed_lately("k", &[failed(retry - second)], now));
        assert!(!failed_lately("k", &[failed(retry)], now));
        assert!(!failed_lately(
            "k",
            &[failed(second), rec(now, "push", "k", true, 3, second)],
            now
        ));
        assert!(!failed_lately("other", &[failed(second)], now));
        assert!(!failed_lately("k", &[], now));
    }

    #[test]
    fn commits_failing_from_the_start_are_every_targets_trouble() {
        let now = chrono::Utc::now();
        let day = chrono::Duration::hours(24);
        let hour = chrono::Duration::hours(1);
        // git missing (say) from the first run on: never a push of its own.
        let failing = [
            rec(now, "commit", "", false, 0, day + hour),
            rec(now, "commit", "", false, 0, hour),
        ];
        let t = status_of("folder", "f", "k", &failing, 3, now);
        assert_eq!(t.last_error.as_deref(), Some("commit failed"));
        assert!(t.stale, "{t:?}");
        // Failing an hour only: said, not yet stale.
        let t = status_of("folder", "f", "k", &failing[1..], 3, now);
        assert_eq!(
            (t.last_error.as_deref(), t.stale),
            (Some("commit failed"), false)
        );
        // A commit that worked since (its push still to come): no error.
        let mended = [
            &failing[..],
            &[rec(now, "commit", "", true, 3, chrono::Duration::zero())],
        ]
        .concat();
        assert_eq!(
            status_of("folder", "f", "k", &mended, 3, now).last_error,
            None
        );
    }

    #[test]
    fn restored_books_know_which_backups_were_theirs() {
        let now = chrono::Utc::now();
        let restored = |field: &str, keys: &[&str]| {
            let mut m = Map::new();
            m.insert("kind".into(), json!("restored"));
            m.insert(field.into(), json!(keys));
            m.insert("ts".into(), json!(now.to_rfc3339()));
            m
        };
        // The same books: their targets took their backups.
        let same = [restored("held", &["k"])];
        assert!(held_before("k", &same));
        assert!(!held_before("new", &same));
        assert!(!paused("k", &same));
        // Others' (restored beside them): their repository is paused, said.
        let beside = [restored("paused", &["r"])];
        assert!(paused("r", &beside));
        assert!(!paused("k", &beside));
        assert!(!held_before("r", &beside));
        let t = status_of("repository", "x", "r", &beside, 9, now);
        assert_eq!(t.last_error.as_deref(), Some(PAUSED));
        assert!(!t.stale && t.paused);
        // A push of its own counts as held, too.
        assert!(held_before(
            "k",
            &[rec(now, "push", "k", true, 3, chrono::Duration::zero())]
        ));
        assert!(!held_before(
            "k",
            &[rec(now, "push", "k", false, 0, chrono::Duration::zero())]
        ));
    }

    #[test]
    fn a_target_named_later_starts_its_own_clock() {
        let now = chrono::Utc::now();
        let day = chrono::Duration::hours(24);
        let hour = chrono::Duration::hours(1);
        // Commits for a week, to another target; then this one's first try.
        let records = [
            rec(now, "commit", "", true, 2, day * 7),
            rec(now, "push", "old", true, 2, day * 7),
            rec(now, "commit", "", true, 3, day * 2),
            rec(now, "push", "k", false, 0, hour),
        ];
        let t = status_of("folder", "f", "k", &records, 3, now);
        assert!(!t.stale, "{t:?}");
        assert_eq!(t.last_error.as_deref(), Some("push failed"));
        // Holding the head: never stale; one behind since long: stale.
        let held = [
            rec(now, "push", "k", true, 3, day * 3),
            rec(now, "push", "k", false, 0, day * 2),
        ];
        assert!(!status_of("folder", "f", "k", &held, 3, now).stale);
        assert!(status_of("folder", "f", "k", &held, 4, now).stale);
    }

    #[test]
    fn drills_are_due_weekly_after_a_pass_and_soon_after_a_failure() {
        let now = chrono::Utc::now();
        let r = |kind, ok, ago| rec(now, kind, "k", ok, 3, ago);
        let hour = chrono::Duration::hours(1);
        let week = DRILL_EVERY;
        let retry = chrono::Duration::from_std(RETRY_AFTER).unwrap();
        // Nothing backed up there: nothing to drill.
        assert!(!drill_due("k", &[], now));
        assert!(!drill_due("k", &[r("push", false, hour)], now));
        // Backed up, never drilled: due.
        let pushed = r("push", true, hour);
        assert!(drill_due("k", std::slice::from_ref(&pushed), now));
        // Passed: due a week after.
        assert!(!drill_due(
            "k",
            &[pushed.clone(), r("drill", true, week - hour)],
            now
        ));
        assert!(drill_due(
            "k",
            &[pushed.clone(), r("drill", true, week)],
            now
        ));
        // Failed: tried again RETRY_AFTER after.
        let t = drill_due(
            "k",
            &[
                pushed.clone(),
                r("drill", false, retry - chrono::Duration::seconds(1)),
            ],
            now,
        );
        assert!(!t);
        assert!(drill_due(
            "k",
            &[pushed.clone(), r("drill", false, retry)],
            now
        ));
        // Another target's drill isn't its, nor its push.
        assert!(drill_due(
            "k",
            &[pushed.clone(), rec(now, "drill", "o", true, 3, hour)],
            now
        ));
        assert!(!drill_due(
            "k",
            &[rec(now, "push", "o", true, 3, hour)],
            now
        ));
        // A failed drill is said until one passes.
        let t = status_of(
            "folder",
            "f",
            "k",
            &[pushed.clone(), r("drill", false, hour)],
            3,
            now,
        );
        assert_eq!(t.drill_error.as_deref(), Some("drill failed"));
        assert!(t.last_drill_at.is_some());
        let t = status_of(
            "folder",
            "f",
            "k",
            &[pushed, r("drill", false, hour), r("drill", true, hour)],
            3,
            now,
        );
        assert_eq!(t.drill_error, None);
    }

    #[test]
    fn each_kind_of_trouble_needs_attention_alone() {
        let clean = TargetStatus {
            target: "folder".into(),
            at: "f".into(),
            last_ok_at: Some("t".into()),
            through_seq: Some(3),
            stale: false,
            last_error: None,
            last_drill_at: None,
            drill_error: None,
            paused: false,
        };
        let status = |t: TargetStatus| Status {
            head_seq: 3,
            targets: vec![t],
            note: None,
        };
        assert!(!status(clean.clone()).needs_attention());
        assert!(
            status(TargetStatus {
                stale: true,
                ..clean.clone()
            })
            .needs_attention()
        );
        assert!(
            status(TargetStatus {
                last_error: Some("e".into()),
                ..clean.clone()
            })
            .needs_attention()
        );
        assert!(
            status(TargetStatus {
                drill_error: Some("e".into()),
                ..clean.clone()
            })
            .needs_attention()
        );
        let none = Status {
            head_seq: 3,
            targets: vec![],
            note: Some("n".into()),
        };
        assert!(none.needs_attention());
    }
}
