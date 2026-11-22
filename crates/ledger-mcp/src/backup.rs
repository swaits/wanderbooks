//! The server's backups of a book (plan 3.7, D-30, D-91): made a while
//! after the books are written (when the writing stops), when they open,
//! before they close, and when asked; written to the backup folder the
//! book's settings name; each recorded in the book's `ops/backup.jsonl`;
//! the folder's backup proven weekly.
//!
//! ```text
//! GOAL: every write reaches the owner's backup soon after it is made, and
//!       the owner (and agent) can see that it did, or why not.
//! SOURCE: plan 3.7 (triggers, "ops/backup.jsonl", books.backup, Needs
//!         attention for a backup more than 24 h behind, drills), D-91
//!         (a plain folder, no git), D-92 (a passphrase), 3.9 and D-39 (a
//!         proven local backup with the keys before a format upgrade),
//!         WB-backup-1.
//! INVARIANTS:
//!   1. A backup is taken under the store's lock as a cut of the books
//!      (ledger_store::backup::Cut: the journal's length and head, the
//!      vault and keys) and written outside it: the books are never held
//!      while a disk or a network share is slow. One run or drill at a
//!      time.
//!   2. Every cut, write and drill is a record of `ops/backup.jsonl` (a
//!      chain log, not the journal: a backup never moves the books' head),
//!      the destination's outcome a record of its own, keyed by the
//!      destination itself (its folder, and the passphrase's fingerprint: a
//!      new folder, or a changed passphrase, starts with no standing).
//!   3. A backup is stale when it lacks the books' head and the first try
//!      to back up what it lacks (the first cut or failure since its last
//!      good write, or since its first try if it never had one, or since the
//!      failed cuts that kept it from any) is more than STALE_AFTER old; its
//!      last error is its last write's, or a later failed cut's.
//!   4. Nothing here names a path of the data root to an agent.
//!   5. A backup that holds a backup is drilled (ledger_store::backup::
//!      drill: its books built and proven whole, its journal the live one's
//!      beginning) once DRILL_EVERY has passed since its last good drill, or
//!      RETRY_AFTER since a failed one (at the server's next look for
//!      drills); a failed drill is said in the standing until one passes.
//!   6. A destination whose write failed lately (RETRY_AFTER) is left out of
//!      a backup after a write; a destination's backup is begun only while
//!      it never took one of these books (one gone since is a disk not
//!      connected), restored books counting the backups of the books they
//!      were.
//!   7. A damaged backup record never keeps the books shut: it is set
//!      aside and a new one begun.
//!   8. A format upgrade (plan 3.9, D-39) is made only after the books are
//!      written to their local backup (`local-backup/` in the book's folder,
//!      the same form) and that backup proven to hold them exactly as they
//!      are (journal, vault and keys; every stored file re-hashed), all
//!      under the store's lock, so nothing is written in between; a backup
//!      that can't be made or proven upgrades nothing. Both are records.
//!   9. These backups can be shut (`shut`), once, for good: the file copy in
//!      progress stops at its next piece and none starts; the call waits for
//!      every step that was under way to END (each step's count rides in its
//!      own blocking task, so it is not lost when the future that began it
//!      is dropped, as a close's 60 s bound drops one), and for the run or
//!      drill holding the books' turn; then the backup record is closed
//!      under its own lock. After `shut` returns, this holder (a timer's, a
//!      request's, any clone from before) runs nothing, and writes nothing,
//!      whatever the books are doing.
//!  10. A close is not held by a step it doesn't wait for. When a close
//!      begins (`hurry`, before its last backup), these stop at their next
//!      look: a drill, a format upgrade that has not yet reached its final
//!      look, and a backup a call or a timer asked for (`run_with` for any
//!      `why` but `close`), looking between pieces (ledger_store::backups
//!      INVARIANT 8) and between destinations. A drill's scratch folder is
//!      removed and nothing is said of its destination (it is due again
//!      when the books next open). A run stopped this way records
//!      `stopped` (what it was doing: `cut` or `write`) and no result of its
//!      own for the step it didn't finish: what it had written stands, and
//!      the destination proves whole at its previous manifest (the one in
//!      place before the run); its caller is told it was stopped because
//!      the books are closing (tools/backup.rs, which marks the call's
//!      access record `stopped_by_close`). An upgrade is reported not done,
//!      the books unchanged, if the close's stop reaches it before its final
//!      look, which is made once everything it needs is proven and just
//!      before the `journal.format_set` line is appended: a stop is
//!      honoured up to that look. One that arrives between it and the append
//!      does not undo the upgrade. The last backup of the close, and the
//!      stop `shut` makes of what is left of it, are not hurried: only
//!      `shut` stops them. The close's whole bound is the sum written in the
//!      books server's INVARIANT 10 (local.rs).
//!  11. The passphrase's key is read from the book's `local/` folder at each
//!      run: with one there, every write is sealed; one that is there and
//!      can't be read stops the write (never a plain one). The key never
//!      reaches a record, an error or a status.
//!  12. Old forms are left alone and said once: a private repository named
//!      by the books' old setting is no longer used, and the old backup
//!      history in the book's folder is neither read, written nor removed;
//!      the status says so until a backup has been made in the new form.
//! ```

use crate::server::SharedStore;
use ledger_store::backup::{self as backup, BackupError, BackupKey, Ctx, Cut, Destination};
use ledger_store::chain::ChainLog;
use serde::Serialize;
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// What a call is told when these backups were shut (INVARIANT 9).
pub const SHUT: &str = "the books were closed: nothing was backed up";

/// Blocking steps of the backups under way (INVARIANT 9).
#[derive(Default)]
struct Busy {
    steps: AtomicUsize,
    ended: tokio::sync::Notify,
}

/// One step under way, counted until it is dropped: it rides in the
/// step's own blocking task, so it ends with the work, not with whatever
/// was waiting for it.
struct Step(Arc<Busy>);

impl Drop for Step {
    fn drop(&mut self) {
        self.0.steps.fetch_sub(1, Ordering::SeqCst);
        self.0.ended.notify_waiters();
    }
}

/// The test knob's file in a book's `ops/`: while it is there, a backup
/// waits at each phase (the `--slow-steps` knob).
pub const SLOW_BACKUP: &str = "slow-backup";
/// The file beside it that gets a line each time a backup reaches a phase
/// and waits.
pub const SLOW_BACKUP_REACHED: &str = "slow-backup-reached";

/// The book's backup record, in its `ops/`.
pub const LOG_FILE: &str = "backup.jsonl";
/// The record's genesis text.
const GENESIS: &str = "wanderbooks-backup-v1";
/// How long a destination may lack the books' head before it is stale.
pub const STALE_AFTER: chrono::Duration = chrono::Duration::hours(24);
/// How often each destination's backup is drilled (plan 3.7: weekly).
pub const DRILL_EVERY: chrono::Duration = chrono::Duration::days(7);
/// How long after a failed run (or drill) the next is tried.
pub const RETRY_AFTER: std::time::Duration = std::time::Duration::from_secs(15 * 60);
/// The most of an error kept.
const ERROR_MAX: usize = 500;
/// The most damaged files a record names.
const REPAIRED_MAX: usize = 20;

/// The test knob that holds a stoppable call as it begins ([`Backups::set_hold`]).
type CallHold = Arc<dyn Fn(&str) + Send + Sync>;

/// A book's backups.
pub struct Backups {
    book: String,
    dir: PathBuf,
    store: Arc<SharedStore>,
    log: Mutex<ChainLog>,
    /// One run at a time.
    running: tokio::sync::Mutex<()>,
    /// Set by `shut` (INVARIANT 9): copying stops, nothing starts.
    shut: Arc<AtomicBool>,
    /// The blocking steps under way.
    busy: Arc<Busy>,
    /// Set when a close begins, before its last backup (INVARIANT 10): the
    /// steps a close doesn't wait for (a drill, a format upgrade not yet
    /// journaled, a backup a call or a timer asked for) stop, and `shut`
    /// sets it too. It is the book's store's flag (`SharedStore::hurry`), so
    /// the book's calls that can run long see it too. The close's own last
    /// backup is stopped by `shut` alone, so it can run.
    hurry: Arc<AtomicBool>,
    /// A test knob (`--slow-steps`): how long a drill or a format upgrade
    /// waits, in milliseconds, at each phase, giving up when hurried.
    slow_ms: AtomicU64,
    /// Phases that began their wait (the test knob), for a test to see
    /// that a step is inside one.
    paused: Arc<AtomicUsize>,
    /// A test knob (`--timer-hold`): told the name of a call that can be
    /// stopped by a close as it begins (`books.verify`, ...); it may wait,
    /// and gives up when the close begins (`hurry`). Nothing without it.
    hold: Mutex<Option<CallHold>>,
}

/// What a format upgrade did ([`Backups::upgrade_format`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upgraded {
    /// The upgrade; None when the books were already in this version's
    /// format (nothing was done).
    pub upgrade: Option<ledger_store::FormatUpgrade>,
    /// The verified backup made first (None when nothing was upgraded).
    pub backup: Option<UpgradeBackup>,
    /// The books' head now.
    pub head: (u64, String),
}

/// The backup a format upgrade made first: the books as they were.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpgradeBackup {
    /// Whether it was written now (false: the local backup already held
    /// the books as they were).
    pub made: bool,
    /// Its number in the local backup.
    pub commit: String,
    /// The head it holds: the books before the upgrade.
    pub head: (u64, String),
}

/// Why a format upgrade wasn't made: nothing was upgraded.
struct UpgradeFailed {
    /// The backup written on the way, if one was.
    made: Option<backup::Written>,
    /// Why, in words.
    why: String,
}

impl UpgradeFailed {
    /// The backup couldn't be made.
    fn backup(e: BackupError) -> Self {
        Self {
            made: None,
            why: format!(
                "the backup the upgrade makes first couldn't be made ({e}): nothing was upgraded"
            ),
        }
    }

    /// The backup `made` (now, or earlier) didn't prove whole.
    fn proof(made: Option<backup::Written>, e: BackupError) -> Self {
        Self {
            made,
            why: format!(
                "the backup the upgrade makes first didn't prove whole ({e}): nothing was upgraded"
            ),
        }
    }

    /// Proven, and the books refused the upgrade.
    fn store(made: Option<backup::Written>, e: ledger_store::StoreError) -> Self {
        Self {
            made,
            why: format!("{e}: nothing was upgraded"),
        }
    }

    /// The books server stopped, or the books are closing.
    fn stopped(why: String) -> Self {
        Self {
            made: None,
            why: format!("{why}: nothing was upgraded"),
        }
    }
}

/// One destination's standing (INVARIANT 3).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TargetStatus {
    /// "folder".
    pub target: String,
    /// Where (a name, never a path of the data root).
    pub at: String,
    /// When it last took a backup (UTC), if ever.
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
    /// Protected by a passphrase.
    pub sealed: bool,
    /// Its disk isn't connected (its last try found no folder).
    pub disconnected: bool,
    /// What kind of trouble its last backup or drill had (`ahead`,
    /// `elsewhere`, `key_missing`, `not_downloaded`, `newest_damaged`,
    /// `no_folder`, `other`), for the window to say what to do.
    pub error_kind: Option<String>,
    /// Files in the backup folder that were found damaged and written again
    /// (each is left where it is: nothing in it is deleted).
    pub damaged_files: Vec<String>,
}

/// The books' backup standing.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Status {
    /// The books' head now.
    pub head_seq: u64,
    /// Each destination the settings name.
    pub targets: Vec<TargetStatus>,
    /// None set up, or an old form left alone: what to know and do.
    pub note: Option<String>,
}

/// A destination the settings name.
#[derive(Debug, Clone)]
struct Named {
    /// Where, and sealed or not; Err when its key can't be read, or the
    /// backups are sealed and this computer has lost the key (INVARIANT 11).
    dest: Result<Destination, BackupError>,
    /// Where, as an agent is told it.
    at: String,
    /// The destination itself, as its records name it (INVARIANT 2): a
    /// digest of where it is and which passphrase (never a path an agent
    /// is shown).
    key: String,
    sealed: bool,
}

/// What a person is told when no backup folder is set.
pub const NONE_SET: &str = "no backup is set up: choose a backup folder (a folder on another disk, or one that Dropbox, iCloud, OneDrive or Google Drive keeps in sync) with books.set_setting backup.folder, or in the window";

/// What books that still name a private repository are told.
pub const OLD_REMOTE: &str = "These books' settings still name a private repository for backups. That kind of backup is no longer made and the repository is left alone: choose a backup folder instead.";

/// What books that still carry an old backup history are told.
pub const OLD_ARCHIVE: &str = "An older backup folder from an earlier version is still in the books' folder. It is no longer written or read, and nothing was deleted: you may delete it yourself once the new backup is whole.";

/// What the record says of whether the backups are sealed: the newest
/// record that says (a set-up that chose a passphrase or took it off, a
/// push that was made sealed or plain), with the key's fingerprint, which
/// opens nothing.
fn sealed_before(records: &[Map<String, Value>]) -> Option<(bool, Option<String>)> {
    let says = |r: &Map<String, Value>| -> Option<bool> {
        if is(r, "setup") {
            match r.get("passphrase").and_then(Value::as_str) {
                Some("set") => Some(true),
                Some("removed") => Some(false),
                _ => None,
            }
        } else if is(r, "push") {
            r.get("sealed").and_then(Value::as_bool)
        } else {
            None
        }
    };
    let (at, sealed) = records
        .iter()
        .enumerate()
        .rev()
        .find_map(|(i, r)| says(r).map(|s| (i, s)))?;
    // The fingerprint of the key the backups were sealed with: the newest
    // record that was made with one (a push that couldn't be made says
    // none).
    let fp = sealed
        .then(|| {
            records[..=at]
                .iter()
                .rev()
                .find_map(|r| says(r).filter(|s| *s).and_then(|_| text(r, "fp")))
        })
        .flatten();
    Some((sealed, fp))
}

/// The records of a book's backup log, if it can be read.
fn read_records(dir: &Path) -> Vec<Map<String, Value>> {
    ledger_store::chain::read(&ledger_store::ops::ops_dir(dir).join(LOG_FILE), GENESIS)
        .map(|r| r.records)
        .unwrap_or_default()
}

/// The destination the book's settings name (D-91).
fn targets(store: &ledger_store::Store, book: &str, dir: &Path) -> Vec<Named> {
    use sha2::{Digest, Sha256};
    let Some(folder) = store.ledger().settings().backup_folder() else {
        return Vec::new();
    };
    let digest = |fp: &str| -> String {
        Sha256::digest(format!("{folder}\0{book}\0{fp}").as_bytes())[..8]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    };
    let at = "the backup folder".to_string();
    let named = |dest: Result<Destination, BackupError>, sealed: bool, fp: &str| Named {
        dest,
        at: at.clone(),
        key: digest(fp),
        sealed,
    };
    let dest = |key: Option<BackupKey>| Destination {
        folder: PathBuf::from(folder),
        key,
    };
    let one = match BackupKey::load(dir) {
        Ok(Some(k)) => {
            let fp = k.fingerprint();
            named(Ok(dest(Some(k))), true, &fp)
        }
        Err(e) => named(Err(e), true, "unreadable"),
        // No key file. Plain, unless the backups were sealed and the file is
        // what was lost (INVARIANT 11): the record says what was chosen
        // last, and where it has nothing, the folder itself does.
        Ok(None) => match sealed_before(&read_records(dir)) {
            Some((true, fp)) => named(
                Err(BackupError::KeyMissing),
                true,
                fp.as_deref().unwrap_or(""),
            ),
            Some((false, _)) => named(Ok(dest(None)), false, ""),
            None if ledger_store::backup::holds_only_sealed(Path::new(folder), book) => {
                named(Err(BackupError::KeyMissing), true, "")
            }
            None => named(Ok(dest(None)), false, ""),
        },
    };
    vec![one]
}

/// What an error of a write is, for the record: tells a disk not connected.
fn error_kind(e: &BackupError) -> &'static str {
    match e {
        BackupError::NoFolder => "no_folder",
        BackupError::NoBackupThere => "no_backup_there",
        BackupError::Elsewhere => "elsewhere",
        BackupError::Ahead(..) => "ahead",
        BackupError::KeyMissing | BackupError::NeedsPassphrase => "key_missing",
        BackupError::NotDownloaded => "not_downloaded",
        BackupError::NewestDamaged { .. } => "newest_damaged",
        _ => "other",
    }
}

impl Backups {
    /// The backups of the book `book`, kept in `dir`, served from `store`.
    pub fn open(book: &str, dir: &Path, store: Arc<SharedStore>) -> Result<Self, String> {
        ledger_store::ops::prepare(dir)
            .map_err(|e| format!("its backup record can't be kept: {e}"))?;
        let path = ledger_store::ops::ops_dir(dir).join(LOG_FILE);
        let (log, carried) = match ChainLog::open(&path, GENESIS) {
            Ok((log, _)) => (log, Vec::new()),
            // INVARIANT 7: set aside, begun again, what it knew of whose
            // backups the destinations hold carried into the new one.
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
        let hurry = store.hurry_flag();
        let me = Self {
            book: book.to_string(),
            dir: dir.to_path_buf(),
            store,
            log: Mutex::new(log),
            running: tokio::sync::Mutex::new(()),
            shut: Arc::default(),
            busy: Arc::default(),
            hurry,
            slow_ms: AtomicU64::new(0),
            paused: Arc::default(),
            hold: Mutex::new(None),
        };
        for fields in carried {
            me.record(fields);
        }
        // An old backup history in the book's folder is left alone, and
        // noted once (INVARIANT 12).
        if dir.join(OLD_ARCHIVE_DIR).exists() && !me.records().iter().any(|r| is(r, "old_archive"))
        {
            me.record(json!({"kind": "old_archive"}));
        }
        // Books just restored: whose they were (INVARIANT 6).
        let marker = ledger_store::ops::ops_dir(dir).join(ledger_store::root::RESTORED_FROM);
        if let Ok(origin) = std::fs::read_to_string(&marker) {
            let origin = Some(origin.trim()).filter(|o| !o.is_empty());
            let named = me
                .store
                .try_with_store(|s| targets(s, book, dir))
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
        // INVARIANT 4: a store message may quote a path in the root.
        if let Some(Value::String(e)) = map.get_mut("error") {
            *e = hide(e, &self.dir);
        }
        let mut log = self.log.lock().unwrap_or_else(|e| e.into_inner());
        match log.append(map) {
            Ok(_) | Err(ledger_store::chain::ChainError::Closed) => {}
            Err(e) => {
                eprintln!("wanderbooks server: the books' backup record couldn't be written: {e}");
            }
        }
    }

    /// Whether these backups were shut (INVARIANT 9).
    fn is_shut(&self) -> bool {
        self.shut.load(Ordering::SeqCst)
    }

    /// A step begins: counted before the shut flag is looked at, and
    /// `shut` sets the flag before it looks at the count, so a step either
    /// sees them shut or is waited for, never neither (INVARIANT 9).
    fn begin(&self) -> Result<Step, String> {
        self.busy.steps.fetch_add(1, Ordering::SeqCst);
        let step = Step(self.busy.clone());
        if self.is_shut() {
            return Err(SHUT.to_string());
        }
        Ok(step)
    }

    /// The stop for the close's own last backup (INVARIANT 9), or, for a
    /// backup a call or a timer asked for, the one a close begins
    /// (INVARIANT 10), so it can't hold the close.
    fn ctx_for(&self, why: &str) -> Ctx {
        let flag = if why == "close" {
            self.shut.clone()
        } else {
            self.hurry.clone()
        };
        let ctx = Ctx::new().with_stop(flag.clone());
        // The test knob: a backup waits at each phase while
        // `ops/slow-backup` is there (and `--slow-steps` was given), until
        // the stop it runs under is asked for, as a slow disk would hold it.
        if self.slow_ms.load(Ordering::SeqCst) == 0 {
            return ctx;
        }
        let marker = ledger_store::ops::ops_dir(&self.dir).join(SLOW_BACKUP);
        let paused = self.paused.clone();
        ctx.with_phase_watch(Arc::new(move |_phase: &str| {
            if !marker.exists() {
                return;
            }
            paused.fetch_add(1, Ordering::SeqCst);
            // Said to the test: a backup has reached a phase and waits.
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(marker.with_file_name(SLOW_BACKUP_REACHED))
            {
                use std::io::Write;
                let _ = f.write_all(b"x\n");
            }
            while marker.exists() && !flag.load(Ordering::SeqCst) {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }))
    }

    /// The stop for a step a close hurries (a drill, a format upgrade), with
    /// the test knob's wait at each phase.
    fn hurried_ctx(&self) -> Ctx {
        let ctx = Ctx::new().with_stop(self.hurry.clone());
        let ms = self.slow_ms.load(Ordering::SeqCst);
        if ms == 0 {
            return ctx;
        }
        let (hurry, paused) = (self.hurry.clone(), self.paused.clone());
        ctx.with_phase_watch(Arc::new(move |phase: &str| {
            // From the second phase on (the first is the fetch, before
            // there is anything of the step's own to see).
            if phase == "fetch" {
                return;
            }
            paused.fetch_add(1, Ordering::SeqCst);
            let by = std::time::Instant::now() + std::time::Duration::from_millis(ms);
            while std::time::Instant::now() < by && !hurry.load(Ordering::SeqCst) {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }))
    }

    /// Make each phase of a drill or format upgrade wait `by` (a test
    /// knob: a book large enough for these to take long).
    pub(crate) fn slow_steps(&self, by: std::time::Duration) {
        self.slow_ms.store(by.as_millis() as u64, Ordering::SeqCst);
    }

    /// Install the test knob that holds a stoppable call as it begins.
    pub(crate) fn set_hold(&self, hold: CallHold) {
        *self.hold.lock().unwrap_or_else(|e| e.into_inner()) = Some(hold);
    }

    /// A call that a close can stop has begun (the test knob's seam).
    pub(crate) fn hold(&self, call: &str) {
        let hold = self.hold.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(hold) = hold {
            hold(call);
        }
    }

    /// How many waits of the test knob have begun.
    #[cfg(test)]
    pub(crate) fn pauses_begun(&self) -> usize {
        self.paused.load(Ordering::SeqCst)
    }

    /// A close begins (INVARIANT 10): the steps it doesn't wait for stop at
    /// their next look. The close's last backup, and what `shut` stops, are
    /// not affected until `shut`.
    pub fn hurry(&self) {
        self.hurry.store(true, Ordering::SeqCst);
    }

    fn hurried(&self) -> bool {
        self.hurry.load(Ordering::SeqCst)
    }

    /// Shut these backups for good (INVARIANT 9): copying stopped, every
    /// step ended, the record closed. Call it after the last backup the
    /// books will have, and before the books' store is let go.
    pub async fn shut(&self) {
        self.shut.store(true, Ordering::SeqCst);
        self.hurry.store(true, Ordering::SeqCst);
        // The run or drill holding the books' turn ends (its copying was
        // just stopped), and none begins after.
        let _turn = self.running.lock().await;
        loop {
            let ended = self.busy.ended.notified();
            if self.busy.steps.load(Ordering::SeqCst) == 0 {
                break;
            }
            ended.await;
        }
        self.log.lock().unwrap_or_else(|e| e.into_inner()).close();
    }

    fn records(&self) -> Vec<Map<String, Value>> {
        ledger_store::chain::read(
            &ledger_store::ops::ops_dir(&self.dir).join(LOG_FILE),
            GENESIS,
        )
        .map(|r| r.records)
        .unwrap_or_default()
    }

    /// The destination the settings name now.
    async fn named(&self) -> Vec<Named> {
        let (store, book, dir) = (self.store.clone(), self.book.clone(), self.dir.clone());
        tokio::task::spawn_blocking(move || store.with_store(|s| targets(s, &book, &dir)).ok())
            .await
            .ok()
            .flatten()
            .unwrap_or_default()
    }

    /// Note that these books were just restored from a backup of the books
    /// `origin` (INVARIANT 6), `named` the destinations they name: the same
    /// books (their destination took their backups), or others'.
    fn restored(&self, origin: Option<&str>, named: &[Named]) {
        let same = origin == Some(self.book.as_str());
        let keys: Vec<String> = named.iter().map(|n| n.key.clone()).collect();
        if same {
            self.record(json!({"kind": "restored", "origin": origin, "held": keys}));
        } else {
            self.record(json!({"kind": "restored", "origin": origin}));
        }
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

    /// [`Self::run`]; with `new_disk`, a destination that took these
    /// books' backups and holds none now is begun again there (the owner
    /// says it is a new disk).
    pub async fn run_with(&self, why: &str, new_disk: bool) -> Result<Status, String> {
        self.run_with_set(why, new_disk, false).await
    }

    /// [`Self::run_with`]; with `new_set`, a destination that holds a
    /// longer history of these books than they have now (as after a restore
    /// from an older backup) gets a new backup set beside the old one,
    /// which is never touched: only at the person's explicit request.
    pub async fn run_with_set(
        &self,
        why: &str,
        new_disk: bool,
        new_set: bool,
    ) -> Result<Status, String> {
        let _one = self.running.lock().await;
        // Books let go of (closed, or replaced by a restore) write nothing
        // more, and leave no record of a run that never began.
        if self.is_shut() {
            return Err(SHUT.to_string());
        }
        if self.store.is_stopped() {
            return Err(crate::server::STOPPED.to_string());
        }
        // A run a call or a timer asked for is stopped by a close
        // (INVARIANT 10); the close's own last backup is not.
        let stoppable = why != "close";
        if stoppable && self.hurried() {
            return Err(self.stopped_words());
        }
        if self.named().await.is_empty() {
            return Ok(self.status().await);
        }
        let step = self.begin()?;
        let (store, dir, book) = (self.store.clone(), self.dir.clone(), self.book.clone());
        let cut = tokio::task::spawn_blocking(move || {
            let _step = step;
            // The cut, under the lock.
            store
                .with_store(|s| -> Result<_, BackupError> {
                    Ok((Cut::take(&book, &dir, s.head())?, targets(s, &book, &dir)))
                })
                .map_err(BackupError::Damaged)
                .and_then(|r| r)
        })
        .await
        .unwrap_or(Err(BackupError::Damaged("the backup stopped".into())));
        let (cut, named) = match cut {
            Ok(m) => m,
            Err(e) => {
                let e = hide(&e.to_string(), &self.dir);
                self.record(json!({"kind": "commit", "why": why, "ok": false, "error": e}));
                return Err(e);
            }
        };
        let head = cut.head().clone();
        // The destination, outside the lock; after a write, one that
        // failed lately waits out RETRY_AFTER.
        let records = self.records();
        let now = chrono::Utc::now();
        for n in named {
            if why == "write" && failed_lately(&n.key, &records, now) {
                continue;
            }
            if stoppable && self.hurried() {
                self.record(json!({"kind": "stopped", "why": why, "during": "write"}));
                return Err(self.stopped_words());
            }
            let dest = match &n.dest {
                Ok(d) => d.clone(),
                Err(e) => {
                    self.record(
                        json!({"kind": "push", "why": why, "target": "folder", "at": n.at,
                        "key": n.key, "ok": false, "error": e.to_string(),
                        "error_kind": error_kind(e), "sealed": n.sealed}),
                    );
                    continue;
                }
            };
            let fp = dest.key.as_ref().map(BackupKey::fingerprint);
            // Its folder is begun only where it never took a backup.
            let may_create = new_disk || !held_before(&n.key, &records);
            let step = self.begin()?;
            let ctx = self.ctx_for(why);
            let cut = cut.clone();
            let written = tokio::task::spawn_blocking(move || {
                let _step = step;
                backup::write_destination_with(
                    &dest,
                    &cut,
                    backup::Options {
                        may_create,
                        new_set,
                    },
                    &ctx,
                )
            })
            .await
            .unwrap_or(Err(BackupError::Damaged("the backup stopped".into())));
            match written {
                Ok(w) => self.record(
                    json!({"kind": "push", "why": why, "target": "folder", "at": n.at,
                    "key": n.key, "ok": true, "number": w.number, "new": w.new,
                    "sealed": n.sealed, "fp": fp, "files_proven": w.files_proven,
                    "repaired": w.repaired.iter().take(REPAIRED_MAX).collect::<Vec<_>>(),
                    "head": {"seq": head.0, "hash": head.1}}),
                ),
                // Stopped by a close: not a failure of the destination, and
                // not a success: said once, of the run (INVARIANT 10).
                Err(BackupError::Cancelled(_)) if stoppable => {
                    self.record(json!({"kind": "stopped", "why": why, "during": "write",
                        "key": n.key}));
                    return Err(self.stopped_words());
                }
                Err(e) => self.record(
                    json!({"kind": "push", "why": why, "target": "folder", "at": n.at,
                    "key": n.key, "ok": false, "error": e.to_string(),
                    "error_kind": error_kind(&e), "sealed": n.sealed, "fp": fp}),
                ),
            }
        }
        Ok(self.status().await)
    }

    /// What a run a close stopped answers: that it was stopped because the
    /// books are closing (the call's access record says so).
    fn stopped_words(&self) -> String {
        crate::server::STOPPED_BY_CLOSE_WORDS.to_string()
    }

    /// The format upgrade, after a verified local backup with the keys
    /// (plan/served.md 3.9, D-39; INVARIANT 8). The caller has the owner's
    /// go-ahead (the passphrase, checked by the server).
    ///
    /// Under the store's lock, so nothing is written between the backup
    /// and the upgrade: the books written to their local backup (whatever
    /// destination the settings name, or none), that backup proven to hold
    /// the books exactly as they are, journal, stored files, vault and keys
    /// ([`backup::prove_local`]); only then this version's journal format
    /// declared (ledger_store `Store::upgrade_format`). Books already in it:
    /// nothing is done. Both the backup and the upgrade are records of
    /// `ops/backup.jsonl`; the destination takes them at the next backup,
    /// the books having moved.
    pub async fn upgrade_format(&self) -> Result<Upgraded, String> {
        let _one = self.running.lock().await;
        // Books let go of, between the request's start and now, upgrade
        // nothing and leave no record (a failure written through this
        // handle would land in the book's record after it was opened
        // again): INVARIANT 9.
        let step = self.begin()?;
        let (store, dir, book) = (self.store.clone(), self.dir.clone(), self.book.clone());
        let ctx = self.hurried_ctx();
        let cancel = self.hurry.clone();
        let done = tokio::task::spawn_blocking(move || {
            let _step = step;
            // Books that wait for nothing are left as they are: not even
            // their local backup is made (asked again under the lock below;
            // only an upgrade moves the format, one at a time).
            match store.with_store(|s| (s.journal_format().upgrade().is_none(), s.head())) {
                Ok((true, head)) => {
                    return Ok(Upgraded {
                        upgrade: None,
                        backup: None,
                        head,
                    });
                }
                Ok((false, _)) => {}
                Err(why) => return Err(UpgradeFailed::stopped(why)),
            }
            store
                .with_store_mut(|s| -> Result<Upgraded, UpgradeFailed> {
                    let head = s.head();
                    let Some(waiting) = s.journal_format().upgrade() else {
                        return Ok(Upgraded {
                            upgrade: None,
                            backup: None,
                            head,
                        });
                    };
                    let cut =
                        Cut::take(&book, &dir, head.clone()).map_err(UpgradeFailed::backup)?;
                    let made =
                        backup::write_local(&dir, &cut, &ctx).map_err(UpgradeFailed::backup)?;
                    let scratch = ledger_store::ops::ops_dir(&dir).join("upgrade-check");
                    let _ = std::fs::remove_dir_all(&scratch);
                    backup::prove_local(&dir, &book, &head, &scratch, &ctx)
                        .map_err(|e| UpgradeFailed::proof(Some(made.clone()), e))?;
                    // The last look for a close (INVARIANT 10): once the
                    // `journal.format_set` line is appended the upgrade
                    // finishes, which is quick; before it, it is stopped,
                    // the books unchanged.
                    if cancel.load(Ordering::SeqCst) {
                        return Err(UpgradeFailed {
                            made: Some(made.clone()),
                            why: "the books are closing: nothing was upgraded".into(),
                        });
                    }
                    let upgrade = match s.upgrade_format() {
                        Ok(upgrade) => upgrade,
                        // Journaled, and what follows the line failed (its
                        // database row, a full disk): the books are in this
                        // version's format all the same, and are never told
                        // otherwise (the database catches up as the books
                        // next open).
                        Err(e) if s.journal_format().upgrade().is_none() => {
                            eprintln!(
                                "wanderbooks server: the format upgrade was journaled, then: {}",
                                hide(&e.to_string(), &dir)
                            );
                            Some(waiting)
                        }
                        Err(e) => return Err(UpgradeFailed::store(Some(made.clone()), e)),
                    };
                    Ok(Upgraded {
                        upgrade,
                        backup: Some(UpgradeBackup {
                            made: made.new,
                            commit: format!("local backup {}", made.number),
                            head,
                        }),
                        head: s.head(),
                    })
                })
                .map_err(UpgradeFailed::stopped)
                .and_then(|r| r)
        })
        .await
        .unwrap_or_else(|_| Err(UpgradeFailed::stopped("the upgrade stopped".into())));
        match done {
            Ok(u) => {
                if let (Some(b), Some(up)) = (&u.backup, &u.upgrade) {
                    if b.made {
                        self.record(
                            json!({"kind": "commit", "why": "format_upgrade", "ok": true,
                            "commit": b.commit, "head": {"seq": b.head.0, "hash": b.head.1}}),
                        );
                    }
                    self.record(json!({"kind": "format_upgrade", "ok": true,
                        "backup": {"commit": b.commit, "head": {"seq": b.head.0, "hash": b.head.1}},
                        "from": {"format": up.from.format, "features": up.from.features},
                        "to": {"format": up.to.format, "features": up.to.features},
                        "head": {"seq": u.head.0, "hash": u.head.1}}));
                }
                Ok(u)
            }
            Err(failed) => {
                if let Some(m) = &failed.made {
                    self.record(
                        json!({"kind": "commit", "why": "format_upgrade", "ok": true,
                        "commit": format!("local backup {}", m.number),
                        "head": {"seq": m.head.0, "hash": m.head.1}}),
                    );
                }
                let why = hide(&failed.why, &self.dir);
                self.record(json!({"kind": "format_upgrade", "ok": false, "error": why}));
                Err(why)
            }
        }
    }

    /// Whether a drill is due (INVARIANT 5).
    pub async fn drill_due(&self) -> bool {
        // Through the store first: books let go name no destination, and
        // their record isn't read for a timer.
        let named = self.named().await;
        if named.is_empty() || self.is_shut() {
            return false;
        }
        let records = self.records();
        let now = chrono::Utc::now();
        named.iter().any(|n| drill_due(&n.key, &records, now))
    }

    /// Drill each destination whose drill is due (INVARIANT 5), each
    /// recorded.
    pub async fn drill(&self) {
        let _one = self.running.lock().await;
        if self.is_shut() || self.store.is_stopped() {
            return;
        }
        let records = self.records();
        let now = chrono::Utc::now();
        for n in self.named().await {
            if !drill_due(&n.key, &records, now) {
                continue;
            }
            if self.hurried() {
                return;
            }
            let Ok(dest) = n.dest.clone() else {
                continue;
            };
            let Ok(step) = self.begin() else {
                return;
            };
            let ctx = self.hurried_ctx();
            let (dir, book) = (self.dir.clone(), self.book.clone());
            // The newest backup must reach what was last recorded good.
            let expect = last_good_head(&n.key, &records);
            let drilled = tokio::task::spawn_blocking(move || {
                let _step = step;
                let scratch = ledger_store::ops::ops_dir(&dir).join("drill-folder");
                let _ = std::fs::remove_dir_all(&scratch);
                backup::drill(
                    &dest,
                    &book,
                    &dir.join(ledger_store::JOURNAL_FILE),
                    &scratch,
                    expect.as_ref(),
                    &ctx,
                )
            })
            .await
            .unwrap_or(Err(BackupError::Damaged("the drill stopped".into())));
            match drilled {
                // Stopped by a close (INVARIANT 10): neither a pass nor a
                // failure, so nothing is said of the destination; the drill
                // is due again when the books next open.
                Err(BackupError::Cancelled(_)) => {}
                Ok(head) => {
                    self.record(json!({"kind": "drill", "target": "folder",
                    "at": n.at, "key": n.key, "ok": true, "head": {"seq": head.0, "hash": head.1}}))
                }
                Err(e) => self.record(json!({"kind": "drill", "target": "folder",
                    "at": n.at, "key": n.key, "ok": false, "error": e.to_string(),
                    "error_kind": error_kind(&e)})),
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
            .unwrap_or_else(|| standing_of(&[], &[], 0, chrono::Utc::now(), None))
    }

    /// The standing, read from `store` (held by the caller: books.start).
    pub fn standing(&self, store: &ledger_store::Store) -> Status {
        standing(store, &self.book, &self.dir)
    }
}

/// What the window shows of a book's backups: the status, the folder the
/// person chose (their own path, never one of the data root's), and whether
/// the backups are protected by a passphrase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Standing {
    /// How the folder stands.
    pub status: Status,
    /// The backup folder, if one is chosen.
    pub folder: Option<String>,
    /// Protected by a passphrase now.
    pub sealed: bool,
    /// What a person is told of the folder when a sync service keeps it
    /// (or may): None when no folder is chosen.
    pub sync: Option<String>,
}

impl Backups {
    /// What the window shows (read under the store's lock).
    pub async fn window_standing(&self) -> Standing {
        let store = self.store.clone();
        let (book, dir) = (self.book.clone(), self.dir.clone());
        let found = tokio::task::spawn_blocking(move || {
            store
                .with_store(|s| {
                    let folder = s.ledger().settings().backup_folder().map(str::to_string);
                    Standing {
                        status: standing(s, &book, &dir),
                        sync: folder
                            .as_deref()
                            .map(|f| sync_words(sync_service(Path::new(f)))),
                        folder,
                        sealed: matches!(BackupKey::load(&dir), Ok(Some(_))),
                    }
                })
                .ok()
        })
        .await
        .ok()
        .flatten();
        found.unwrap_or_else(|| Standing {
            status: standing_of(&[], &[], 0, chrono::Utc::now(), None),
            folder: None,
            sealed: false,
            sync: None,
        })
    }

    /// Set up the backups (D-91, D-92), as the owner does it in the window:
    /// the folder (`folder`, journaled as the books' `backup.folder` setting,
    /// the owner the one who decided), and the passphrase (`passphrase`
    /// makes a new key, kept on this computer only: backups from the next on
    /// are sealed, in a set of their own beside any earlier one, which is
    /// untouched; `remove_passphrase` forgets the key: the next backup is
    /// plain). What was done, in words.
    pub async fn set_up(
        &self,
        folder: Option<String>,
        passphrase: Option<zeroize::Zeroizing<String>>,
        remove_passphrase: bool,
    ) -> Result<Vec<String>, String> {
        let _one = self.running.lock().await;
        if self.is_shut() {
            return Err(SHUT.to_string());
        }
        let step = self.begin()?;
        let (store, dir) = (self.store.clone(), self.dir.clone());
        let done = tokio::task::spawn_blocking(move || -> Result<SetUpDone, String> {
            let _step = step;
            let mut lines = Vec::new();
            if let Some(folder) = folder.filter(|f| !f.trim().is_empty()) {
                let path = PathBuf::from(folder.trim());
                if !path.is_absolute() {
                    return Err("the backup folder is named by its whole path".into());
                }
                if !path.is_dir() {
                    return Err("that folder isn't there: choose a folder that exists".into());
                }
                if ledger_store::root::inside_a_root(&path) {
                    return Err(format!(
                        "that folder is inside {}: choose a folder somewhere else, on another disk if you can",
                        ledger_store::root::THE_ROOT
                    ));
                }
                let change = ledger_core::settings::Change {
                    key: "backup.folder".into(),
                    value: Some(Value::String(path.display().to_string())),
                };
                let source = ledger_core::settings::Source {
                    decided_by: "the owner, in the window".into(),
                    decided_on: chrono::Utc::now().date_naive(),
                    citation: None,
                    note: Some("The backup folder.".into()),
                };
                let made = store
                    .with_store_mut(|s| s.change_settings(&[change], &source))
                    .map_err(|e| e.to_string())?
                    .map_err(|e| e.to_string())?;
                lines.push(if made.is_empty() {
                    "The backup folder is the one chosen already.".to_string()
                } else {
                    "The backup folder is chosen: the books back themselves up there a little after each change, when they open and before they close.".to_string()
                });
                lines.push(sync_words(sync_service(&path)));
            }
            if remove_passphrase && passphrase.is_some() {
                return Err("choose a passphrase or remove it, not both".into());
            }
            let mut chose: Option<(&str, Option<String>)> = None;
            if let Some(p) = passphrase {
                let key = BackupKey::generate(&p).map_err(|e| e.to_string())?;
                key.store(&dir).map_err(|e| hide(&e.to_string(), &dir))?;
                chose = Some(("set", Some(key.fingerprint())));
                lines.push("Backups are now protected by the passphrase: from the next backup on, every file in the backup folder is sealed. Backups made before stay as they were, unprotected, and are never deleted: delete their folder yourself if you want them gone. A forgotten passphrase cannot be recovered by anyone.".to_string());
            } else if remove_passphrase {
                BackupKey::forget(&dir).map_err(|e| hide(&e.to_string(), &dir))?;
                chose = Some(("removed", None));
                lines.push("The passphrase is taken off: from the next backup on, files are written without protection. Backups sealed before stay sealed with the passphrase they were made with.".to_string());
            }
            if lines.is_empty() {
                lines.push("Nothing was changed.".to_string());
            }
            Ok((lines, chose.map(|(c, fp)| (c.to_string(), fp))))
        })
        .await
        .unwrap_or_else(|_| Err("the set-up stopped".into()));
        match done {
            Ok((lines, chose)) => {
                // What was chosen about the passphrase is kept in the
                // record, so that a key file lost later is told from a
                // passphrase taken off (INVARIANT 11).
                let mut rec = json!({"kind": "setup", "lines": lines.len()});
                if let Some((choice, fp)) = chose {
                    rec["passphrase"] = json!(choice);
                    rec["fp"] = json!(fp);
                }
                self.record(rec);
                Ok(lines)
            }
            Err(e) => Err(e),
        }
    }

    /// Save a backup as one file at `path` ("Save a backup..."): the same
    /// contents as the backup folder's, sealed when the backups are.
    pub async fn save_file(&self, path: &str) -> Result<Vec<String>, String> {
        let _one = self.running.lock().await;
        if self.is_shut() {
            return Err(SHUT.to_string());
        }
        if self.hurried() {
            return Err(self.stopped_words());
        }
        let mut file = PathBuf::from(path.trim());
        if !file.is_absolute() {
            return Err("the file is named by its whole path".into());
        }
        if file.extension().is_none_or(|e| e != backup::FILE_EXTENSION) {
            let mut name = file.file_name().unwrap_or_default().to_os_string();
            name.push(format!(".{}", backup::FILE_EXTENSION));
            file.set_file_name(name);
        }
        if file.parent().is_none_or(|p| !p.is_dir()) {
            return Err("the folder to save it in isn't there".into());
        }
        if ledger_store::root::inside_a_root(&file) {
            return Err(format!(
                "that is inside {}: save it somewhere else",
                ledger_store::root::THE_ROOT
            ));
        }
        let step = self.begin()?;
        let (store, dir, book) = (self.store.clone(), self.dir.clone(), self.book.clone());
        let ctx = self.ctx_for("asked");
        let sealed_to = file.clone();
        let saved = tokio::task::spawn_blocking(move || {
            let _step = step;
            let key = BackupKey::load(&dir).map_err(|e| e.to_string())?;
            let cut = store
                .with_store(|s| Cut::take(&book, &dir, s.head()))?
                .map_err(|e| e.to_string())?;
            backup::save_file(&sealed_to, &cut, key.as_ref(), &ctx)
                .map(|w| (w, key.is_some()))
                .map_err(|e| e.to_string())
        })
        .await
        .unwrap_or_else(|_| Err("the backup stopped".into()));
        match saved {
            Ok((w, sealed)) => {
                self.record(json!({"kind": "saved", "ok": true, "sealed": sealed,
                    "head": {"seq": w.head.0, "hash": w.head.1}}));
                Ok(vec![format!(
                    "Saved a backup of the books through event {} as {}{}.",
                    w.head.0,
                    file.display(),
                    if sealed {
                        ", protected by the passphrase"
                    } else {
                        ""
                    }
                )])
            }
            Err(why) => {
                let why = hide(&why, &self.dir);
                self.record(json!({"kind": "saved", "ok": false, "error": why}));
                Err(why)
            }
        }
    }
}

/// What a person is told about a backup folder a sync service keeps (set
/// up, and in the window), `service` being its name when the path tells.
pub fn sync_words(service: Option<&str>) -> String {
    let whose = match service {
        Some(name) => format!("This folder is kept in sync by {name}."),
        None => "If this folder is kept in sync by a service (Dropbox, iCloud Drive, OneDrive, Google Drive or another):".to_string(),
    };
    format!(
        "{whose} Use it from one computer only: two computers backing up into one synced folder can overwrite each other's work, and a second computer needs a folder of its own. Keep it available offline (not online only), so every file is on this computer when a backup is made or checked: a file that is online only reads as \"not downloaded yet\", and the backup waits."
    )
}

/// The sync service whose folder `path` is inside, by the names of the
/// folders in it: reliable for the folders these services make for
/// themselves on each system (`Dropbox`, `OneDrive`, `iCloud Drive` and
/// `Library/Mobile Documents/com~apple~CloudDocs`, `Google Drive` and
/// `My Drive`, `Nextcloud`, `pCloud Drive`, `Box`), and silent about a
/// service's folder that was renamed or put somewhere else (then the
/// set-up says it in general words instead).
pub fn sync_service(path: &Path) -> Option<&'static str> {
    path.components().find_map(|c| {
        let name = c.as_os_str().to_str()?.to_lowercase();
        let has = |p: &str| {
            name == p || name.starts_with(&format!("{p} ")) || name.starts_with(&format!("{p}-"))
        };
        if has("dropbox") {
            Some("Dropbox")
        } else if has("onedrive") {
            Some("OneDrive")
        } else if matches!(
            name.as_str(),
            "icloud drive" | "iclouddrive" | "com~apple~clouddocs" | "mobile documents"
        ) {
            Some("iCloud Drive")
        } else if matches!(name.as_str(), "google drive" | "googledrive" | "my drive")
            || name.starts_with("googledrive-")
        {
            Some("Google Drive")
        } else if has("nextcloud") || has("owncloud") {
            Some("Nextcloud")
        } else if has("pcloud") || name == "pcloud drive" {
            Some("pCloud")
        } else if name == "box" || name == "box sync" {
            Some("Box")
        } else {
            None
        }
    })
}

/// What a set-up did, in words, and the choice it made about the
/// passphrase ("set" with the key's fingerprint, or "removed").
type SetUpDone = (Vec<String>, Option<(String, Option<String>)>);

/// The old backup history's folder in a book's folder: left alone.
const OLD_ARCHIVE_DIR: &str = "archive.git";

fn standing(store: &ledger_store::Store, book: &str, dir: &Path) -> Status {
    let records =
        ledger_store::chain::read(&ledger_store::ops::ops_dir(dir).join(LOG_FILE), GENESIS)
            .map(|r| r.records)
            .unwrap_or_default();
    let old_remote = store.ledger().settings().backup_git_remote().is_some();
    standing_of(
        &targets(store, book, dir),
        &records,
        store.head().0,
        chrono::Utc::now(),
        Some(old_remote),
    )
}

fn standing_of(
    named: &[Named],
    records: &[Map<String, Value>],
    head_seq: u64,
    now: chrono::DateTime<chrono::Utc>,
    old_remote: Option<bool>,
) -> Status {
    let targets: Vec<TargetStatus> = named
        .iter()
        .map(|n| status_of("folder", &n.at, &n.key, n.sealed, records, head_seq, now))
        .collect();
    // The old forms, said until a backup has been made in the new one
    // (INVARIANT 12).
    let mut notes: Vec<&str> = Vec::new();
    if named.is_empty() {
        notes.push(NONE_SET);
    }
    if old_remote == Some(true) {
        notes.push(OLD_REMOTE);
    }
    let new_form_made = records.iter().any(|r| is(r, "push") && ok(r));
    let old_archive_at = records.iter().rposition(|r| is(r, "old_archive"));
    if old_archive_at.is_some_and(|i| !records[i..].iter().any(|r| is(r, "push") && ok(r)))
        && !new_form_made
    {
        notes.push(OLD_ARCHIVE);
    }
    Status {
        head_seq,
        note: (!notes.is_empty()).then(|| notes.join(" ")),
        targets,
    }
}

impl Status {
    /// Whether the human should hear of it: none set up, an old form to
    /// act on, or one stale, failing, or failing its drill.
    pub fn needs_attention(&self) -> bool {
        self.note.is_some()
            || self.targets.iter().any(|t| {
                t.stale
                    || t.last_error.is_some()
                    || t.drill_error.is_some()
                    || !t.damaged_files.is_empty()
            })
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

/// The head of the target `key`'s last backup recorded good.
fn last_good_head(key: &str, records: &[Map<String, Value>]) -> Option<(u64, String)> {
    let r = records
        .iter()
        .rfind(|r| is(r, "push") && of(r, key) && ok(r))?;
    let head = r.get("head")?;
    Some((
        head.get("seq")?.as_u64()?,
        head.get("hash")?.as_str()?.to_string(),
    ))
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
    sealed: bool,
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
    let disconnected = records
        .iter()
        .rfind(|r| is(r, "push") && of(r, key))
        .is_some_and(|r| {
            !ok(r) && r.get("error_kind").and_then(Value::as_str) == Some("no_folder")
        });
    let error_kind = {
        let push = records.iter().rfind(|r| is(r, "push") && of(r, key));
        let from_push = push.filter(|r| !ok(r)).and_then(|r| text(r, "error_kind"));
        let from_drill = drill.filter(|r| !ok(r)).and_then(|r| text(r, "error_kind"));
        from_push.or(from_drill)
    };
    let mut damaged_files: Vec<String> = Vec::new();
    for r in records
        .iter()
        .filter(|r| is(r, "push") && of(r, key) && ok(r))
    {
        for name in r
            .get("repaired")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            if !damaged_files.iter().any(|d| d == name) && damaged_files.len() < REPAIRED_MAX {
                damaged_files.push(name.to_string());
            }
        }
    }
    TargetStatus {
        target: kind.to_string(),
        at: at.to_string(),
        last_ok_at: last_ok.and_then(|i| text(&records[i], "ts")),
        through_seq,
        stale: behind && old && head_seq > 0,
        last_error,
        last_drill_at: drill.and_then(|r| text(r, "ts")),
        drill_error: drill.filter(|r| !ok(r)).and_then(|r| text(r, "error")),
        sealed,
        disconnected,
        error_kind,
        damaged_files,
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
                "cannot write '/home/u/.local/share/wanderbooks/books/acme/journal.jsonl' and /home/u/.local/share/wanderbooks/run",
                dir
            ),
            format!(
                "cannot write 'the book's folder/journal.jsonl' and {}/run",
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
            status_of("folder", "f", "k", false, records, head, now)
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
        // The books unreadable (say) from the first run on: never a write of its own.
        let failing = [
            rec(now, "commit", "", false, 0, day + hour),
            rec(now, "commit", "", false, 0, hour),
        ];
        let t = status_of("folder", "f", "k", false, &failing, 3, now);
        assert_eq!(t.last_error.as_deref(), Some("commit failed"));
        assert!(t.stale, "{t:?}");
        // Failing an hour only: said, not yet stale.
        let t = status_of("folder", "f", "k", false, &failing[1..], 3, now);
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
            status_of("folder", "f", "k", false, &mended, 3, now).last_error,
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
        // Others' (restored under another id): no backup of theirs is held.
        let beside = [restored("origin", &["other"])];
        assert!(!held_before("r", &beside));
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
        let t = status_of("folder", "f", "k", false, &records, 3, now);
        assert!(!t.stale, "{t:?}");
        assert_eq!(t.last_error.as_deref(), Some("push failed"));
        // Holding the head: never stale; one behind since long: stale.
        let held = [
            rec(now, "push", "k", true, 3, day * 3),
            rec(now, "push", "k", false, 0, day * 2),
        ];
        assert!(!status_of("folder", "f", "k", false, &held, 3, now).stale);
        assert!(status_of("folder", "f", "k", false, &held, 4, now).stale);
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
            false,
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
            false,
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
            sealed: false,
            disconnected: false,
            error_kind: None,
            damaged_files: Vec::new(),
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
