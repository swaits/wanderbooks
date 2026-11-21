//! Backups (plan 3.7, D-30, D-31): the book as it is, in git. Each book
//! keeps a bare repository, `archive.git`, whose commits are its journal,
//! its stored files, its vault and keys, and its access log as they were;
//! it is pushed to the owner's private repository and to a repository in
//! the owner's folder. Not encrypted (D-31): the destinations keep it
//! private.
//!
//! ```text
//! GOAL: a book's every write survives the loss of this computer, and a
//!       backup is proven whole before it is trusted or restored.
//! SOURCE: plan 3.7 ("Each run: verifies the chain; commits the new
//!         journal lines and blobs to archive.git; pushes to the
//!         targets"; "Proven before use"; drills), D-30 (both targets),
//!         D-31 (no encryption). The chain isn't verified at each run: the
//!         server verifies it when it opens the books, every write after
//!         goes through the store, and drills prove what reached each
//!         target.
//! INVARIANTS:
//!   1. A commit holds exactly the files under the backed-up paths
//!      (BACKED_UP) of the book's folder as they are at that moment: every
//!      one, by name (checked against the folder after staging; no ignore
//!      rule applies), and byte for byte (no attributes rule can reach it:
//!      none of the owner's or the system's is read, the archive's own is
//!      taken out, and a `.gitattributes` in the book's folder refuses the
//!      backup). The caller holds the store's lock, so the journal is whole
//!      lines and the vault matches it. A commit with nothing changed is
//!      not made.
//!   2. A push only moves a target forward (fast-forward): one that has
//!      diverged is reported, never forced.
//!   3. A backup is trusted only after it is cloned, its tree found to be
//!      plain files under BACKED_UP only (no link, no submodule, nothing
//!      else) before any of it is written out, its journal's chain
//!      replayed, every stored file re-hashed, every TIN record the books
//!      name present with the vault's key file, and its head found
//!      equal to the one its commit records (`verify`); a drill also
//!      checks its journal is a byte prefix of the live one.
//!   4. git runs with its configuration pinned where it matters: no system
//!      config or attributes; no hooks, here or in a repository on this
//!      computer it pushes to; no prompts, signing, ignore or attributes
//!      files, file watchers or command-running transports; our own
//!      identity, the files ref format. A repository on this computer is
//!      pushed to or read only if plain: a bare repository at that very
//!      path, nothing in it sending git elsewhere (a `.git`, `commondir`,
//!      `gitdir`, objects borrowed from elsewhere, a link where it or its
//!      own parts are, something beside it git would take instead), its configuration only
//!      what `git init --bare` writes and where a copy of one came from;
//!      and it is read as a far one is. A git
//!      that says nothing for GIT_TIMEOUT is stopped with all it started.
//!      What git said of itself is kept in whole lines; callers hide the
//!      paths in it before an agent sees it (ledger-mcp backup.rs,
//!      root.rs). None of ours starts another git to go on after it ends
//!      (`gc.auto=0`, `maintenance.auto=false`): when a git of ours has
//!      ended, nothing it began is still working.
//!   5. A git that is stopped (cancelled, or silent for GIT_TIMEOUT) is
//!      stopped gently: SIGTERM to its whole process group first (git
//!      removes the lock files it holds when it is sent one, and a push's
//!      receiving end the objects it had not yet kept), up to TERM_GRACE
//!      (5 s: git's handler takes milliseconds, so the grace is only for a
//!      machine too loaded to schedule it) for the group to end, and only
//!      then SIGKILL. So a lock left behind is the rare case, and there is
//!      nothing to clean up in the usual one. (Windows has no signal to
//!      send a group: the nearest thing the code has, `taskkill /T /F`,
//!      ends the tree at once, and the rules below cover what is left.)
//!      Once the stopped command is reaped AND every process it started is
//!      gone (a killed process can still take a lock in its last
//!      instructions, so the process group is waited for, GROUP_GONE_WAIT,
//!      and nothing is looked at as left over while it isn't), the locks in
//!      the repositories that command could write are dealt with, each
//!      noted ([`LockNote`], for the caller's record). Each repository is
//!      looked at as the command begins (its folder opened once, the locks
//!      already there listed); "ours" is a lock that wasn't there then.
//!        (A stopped `init` of the archive, or of a folder target's new
//!        repository, is such a command too: its HEAD lock is dealt with
//!        the same way.)
//!        a. The book's own archive (`archive.git` in the book's folder,
//!           private to this program): the locks of ours are removed once
//!           the group is gone, each noted. Nothing else is removed from
//!           it in a stop. At open (`clear_archive_locks`), and when a
//!           backup begins, only a lock older than ARCHIVE_STALE (10
//!           minutes) is removed, each removal noted; a younger one is
//!           left (the next backup tries again, and removes it once it is
//!           old). Running git by hand inside the archive is unsupported:
//!           only the store lock keeps another wanderbooks process out, and
//!           what protects the archive from one that doesn't is that this
//!           code never removes a lock younger than ARCHIVE_STALE except
//!           one that its own stopped git made (a lock a person's git
//!           holds that long is the person's to deal with).
//!        b. A target on this computer (a folder, or a repository named by
//!           a path): its git is our `receive-pack`, a child of the one
//!           stopped. This program NEVER writes to, or deletes from, a
//!           folder that is the owner's (an external disk, a synced folder
//!           such as Syncthing's, shared with other computers and other
//!           gits): only git itself does. A lock git left there when it
//!           had to be killed, that wasn't there when the push began, is
//!           left in place and reported with its whole path: a note
//!           (`lock_left`), and [`BackupError::LockLeft`], which marks the
//!           target's backup failed and names the files and what the owner
//!           may do (remove them, only when nothing else is using the
//!           backup folder). A lock that was there when the push began is
//!           another's: untouched, and not named. A later backup that finds
//!           the lock gone goes on normally. A repository far away (ssh,
//!           https) is never looked at: its receiving end cleans up when
//!           the connection ends.
//!        c. No removal follows a path. A removal goes through the folder
//!           handle opened and verified once, when the command began (it is
//!           the archive in the book's folder, not a link; git_locks), never
//!           follows a link, and removes only a regular file named as git
//!           names its locks, in the places git puts them.
//!   6. A step that has to end when the books are let go (the caller's
//!      cancel flag, [`Git::with_cancel`]) ends promptly, wherever it is,
//!      and leaves nothing half-made: it looks for the flag between every
//!      git command and phase of a verification (`clone`, `tree`,
//!      `checkout`, `replay`, `verify`, `blobs`, `vault`, `prefix`), once
//!      per file listed or hashed, once per chunk of the journal compared
//!      and once per journal line replayed (cancel.rs); a git command
//!      running is killed. A stopped drill or proof removes its scratch
//!      clone (the one removal that is not itself stopped: it is as long
//!      as the clone is big). The longest between two looks is one git
//!      command's stop (SIGTERM and up to TERM_GRACE, then a kill, GROUP_GONE_WAIT,
//!      and up to 10 s for its pipes), one journal line, one file, or one 64 KiB chunk.
//! ```

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// A book's backup repository, in its own folder.
pub const ARCHIVE_DIR: &str = "archive.git";
/// What a backup holds, of a book's folder (INVARIANT 1).
pub const BACKED_UP: [&str; 5] = [
    crate::JOURNAL_FILE,
    "blobs",
    crate::root::VAULT_DIR,
    crate::root::KEYS_DIR,
    crate::access::ACCESS_FILE,
];
/// The longest a git command may say nothing before it is stopped.
pub const GIT_TIMEOUT: Duration = Duration::from_secs(300);
/// How long a git that is to be stopped is given, after SIGTERM, to end
/// (and so remove its own lock files) before its whole process group is
/// killed (INVARIANT 5). Git's handler takes milliseconds; the grace is for
/// a machine so loaded that the handler is slow to be scheduled. It is
/// generous on purpose: a lock left in the owner's folder costs the owner a
/// step by hand, while a longer grace costs only a git that ignores SIGTERM.
pub const TERM_GRACE: Duration = Duration::from_secs(5);
/// How old a lock in the book's own archive must be, at open and when a
/// backup begins, before it is taken for a dead git's (INVARIANT 5a): a
/// younger one is left, and the next backup tries again.
pub const ARCHIVE_STALE: Duration = Duration::from_secs(600);

/// Why a backup couldn't be made, pushed or trusted. Never names a path of
/// the data root (INVARIANT 4).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BackupError {
    /// git isn't installed.
    #[error("git isn't installed: backups need it (install git, then try again)")]
    NoGit,
    /// A git command failed; what it said, in whole lines (paths in it are
    /// the caller's to hide, INVARIANT 4).
    #[error("git {0}: {1}")]
    Git(String, String),
    /// A git command ran past its time limit.
    #[error("git {0} took too long and was stopped")]
    TimedOut(String),
    /// A git command stopped because the books it served were let go
    /// ([`Git::with_cancel`]).
    #[error("git {0} was stopped: the books are closing")]
    Cancelled(String),
    /// A git of ours was stopped while it was writing a backup folder (or a
    /// repository on this computer), and left lock files there (every one
    /// named, whole paths, separated by `, `), which this program never
    /// removes from a place that is the owner's (INVARIANT 5b).
    #[error(
        "git was stopped while it was writing the backup, and left lock files in your backup folder: {0}. This program doesn't remove files from your backup folder. Remove those files (only those), and only when nothing else is using the backup folder (another computer, a sync program, git run by hand); the next backup then goes on"
    )]
    LockLeft(String),
    /// A folder target that isn't there (an external disk not mounted).
    #[error("the backup folder isn't there (is its disk connected?)")]
    NoFolder,
    /// A folder target that took these books' backups but holds none now
    /// (a disk not mounted on its mount point, or a new disk).
    #[error(
        "the backup folder holds no backup of these books: connect the disk that does; if it is a new disk, back up now saying so (new_disk)"
    )]
    NoBackupThere,
    /// The target moved on without us: never forced (INVARIANT 2).
    #[error("the backup at {0} has changes this computer doesn't: it was not overwritten")]
    Diverged(String),
    /// A backup that isn't whole.
    #[error("that backup can't be trusted: {0}")]
    Damaged(String),
    /// The disk refused.
    #[error("the backup couldn't be made: {0}")]
    Io(std::io::ErrorKind),
}

impl From<std::io::Error> for BackupError {
    fn from(e: std::io::Error) -> Self {
        BackupError::Io(e.kind())
    }
}

/// A lock file a git left, as it was dealt with (INVARIANT 5): for the
/// caller's own record of the backup (never names a path of the data root:
/// the repository is `archive`, or the target's kind, and the lock is
/// named from the repository's own folder).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockNote {
    /// `archive`, `folder` or `repository`.
    pub repo: &'static str,
    /// The lock, as a path in the repository (`index.lock`,
    /// `refs/heads/main.lock`).
    pub lock: String,
    /// Removed (the archive's, made by the git just stopped), or left.
    pub removed: bool,
    /// Why: what ended the git that made it, or why it was left.
    pub why: &'static str,
    /// For a lock left in a folder that is the owner's: its whole path, the
    /// file the owner is told to remove (INVARIANT 5b).
    pub path: Option<String>,
}

/// A watcher of a verification's phases ([`Git::with_phase_watch`]).
pub type PhaseWatch = std::sync::Arc<dyn Fn(&str) + Send + Sync>;

/// What a caller may watch of a git's work, and look at after it (tests,
/// and the server's record of a backup).
#[derive(Clone, Default)]
struct Watch {
    /// Told the name of each phase of a verification as it begins.
    phase: Option<PhaseWatch>,
    /// The lock files dealt with since the caller last took them.
    notes: std::sync::Arc<std::sync::Mutex<Vec<LockNote>>>,
}

impl std::fmt::Debug for Watch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Watch")
    }
}

/// The git program, found once.
#[derive(Debug, Clone)]
pub struct Git {
    exe: PathBuf,
    /// How long it may say nothing before it is stopped (GIT_TIMEOUT).
    quiet: Duration,
    /// Set by whoever serves the books when they are let go: a git
    /// command running is stopped, with what it started, and none starts.
    cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    /// How long a git that is to be stopped has to end after SIGTERM
    /// before it is killed (TERM_GRACE).
    term_grace: Duration,
    watch: Watch,
}

impl Git {
    /// git, from the usual places (never the working directory).
    pub fn find() -> Result<Git, BackupError> {
        let dirs = std::env::var_os("PATH")
            .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
            .unwrap_or_default();
        dirs.iter()
            .chain(
                [
                    "/usr/bin",
                    "/bin",
                    "/usr/local/bin",
                    "/opt/homebrew/bin",
                    // Git for Windows' own folders (a host started clean
                    // has a short PATH).
                    r"C:\Program Files\Git\cmd",
                    r"C:\Program Files\Git\bin",
                ]
                .iter()
                .map(PathBuf::from)
                .collect::<Vec<_>>()
                .iter(),
            )
            .filter(|d| d.is_absolute())
            .map(|d| d.join(if cfg!(windows) { "git.exe" } else { "git" }))
            .find(|p| p.is_file())
            .map(|exe| Git {
                exe,
                quiet: GIT_TIMEOUT,
                cancel: None,
                term_grace: TERM_GRACE,
                watch: Watch::default(),
            })
            .ok_or(BackupError::NoGit)
    }

    /// This git, given `grace` (not TERM_GRACE) to end after SIGTERM before
    /// it is killed (a seam for tests).
    #[must_use]
    pub fn with_term_grace(mut self, grace: Duration) -> Self {
        self.term_grace = grace;
        self
    }

    /// This git, stopped (every command running, and any that would
    /// start) once `cancel` is set: how the server's close ends a backup
    /// that is still going, instead of leaving it to run against books it
    /// reported closed.
    #[must_use]
    pub fn with_cancel(mut self, cancel: std::sync::Arc<std::sync::atomic::AtomicBool>) -> Self {
        self.cancel = Some(cancel);
        self
    }

    fn cancelled(&self) -> bool {
        self.cancel
            .as_ref()
            .is_some_and(|c| c.load(std::sync::atomic::Ordering::SeqCst))
    }

    /// Ok, or `Cancelled(what)` once this git was cancelled: looked at
    /// between a step's phases and inside its loops over files, so a step
    /// that is not running a git command ends as promptly as one that is
    /// (INVARIANT 6).
    fn check(&self, what: &str) -> Result<(), BackupError> {
        if self.cancelled() {
            Err(BackupError::Cancelled(what.to_string()))
        } else {
            Ok(())
        }
    }

    /// This git, telling `watch` the name of each phase of a verification
    /// as it begins (a seam for tests and for the server's own test knob
    /// that makes a drill slow: `watch` may wait, but must give up when
    /// the cancel flag is set).
    #[must_use]
    pub fn with_phase_watch(mut self, watch: PhaseWatch) -> Self {
        self.watch.phase = Some(watch);
        self
    }

    /// This git, sharing `sink` for the lock files it deals with
    /// (INVARIANT 5), which the caller takes with [`Self::take_lock_notes`]:
    /// every git made from the same sink reports into it.
    #[must_use]
    pub fn with_lock_notes(mut self, sink: LockNotes) -> Self {
        self.watch.notes = sink.0;
        self
    }

    /// The lock files dealt with since this was last called.
    pub fn take_lock_notes(&self) -> Vec<LockNote> {
        std::mem::take(&mut *self.watch.notes.lock().unwrap_or_else(|e| e.into_inner()))
    }

    fn note(&self, note: LockNote) {
        self.watch
            .notes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(note);
    }

    fn phase(&self, name: &str) {
        if let Some(watch) = &self.watch.phase {
            watch(name);
        }
    }

    /// A git command with a clean configuration (INVARIANT 4).
    fn command(&self) -> Command {
        let mut c = Command::new(&self.exe);
        c.env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_ATTR_NOSYSTEM", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_ASKPASS", "true")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .args([
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "commit.gpgsign=false",
                "-c",
                "user.name=wanderbooks",
                "-c",
                "user.email=backup@wanderbooks.invalid",
                "-c",
                "init.defaultBranch=main",
                "-c",
                "core.autocrlf=false",
                // The owner's own ignore and attributes files never leave a
                // file out of a backup or change its bytes; nothing runs
                // to watch the files.
                "-c",
                "core.excludesFile=/dev/null",
                "-c",
                "core.attributesFile=/dev/null",
                "-c",
                "core.fsmonitor=false",
                "-c",
                "init.templateDir=",
                "-c",
                "init.defaultRefFormat=files",
                "-c",
                "init.defaultObjectFormat=sha1",
                "-c",
                "log.showSignature=false",
                "-c",
                "push.gpgSign=false",
                // No git of ours starts another to go on after it ends
                // (a detached `gc` would outlive a stopped backup and hold
                // locks: INVARIANT 5).
                "-c",
                "gc.auto=0",
                "-c",
                "maintenance.auto=false",
                // A link is never made from a backup's tree (verify
                // refuses one before checkout, too).
                "-c",
                "core.symlinks=false",
                // Only these ways of reaching a repository: never one that
                // runs a command (ext::, fd::), whatever the owner's own
                // configuration allows.
                "-c",
                "protocol.allow=never",
                "-c",
                "protocol.ext.allow=never",
                "-c",
                "protocol.fd.allow=never",
                "-c",
                "protocol.file.allow=always",
                "-c",
                "protocol.ssh.allow=always",
                "-c",
                "protocol.https.allow=always",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        c
    }

    /// Run `args` (named `what` in errors): its output. Stopped, with every
    /// process it started, once it has said nothing for GIT_TIMEOUT
    /// (a push or a clone reports its progress as it goes).
    fn run(&self, what: &str, args: &[&std::ffi::OsStr]) -> Result<String, BackupError> {
        self.run_killed(what, args).0
    }

    /// [`Self::run`], and whether the command had to be killed (cancelled
    /// or gone quiet) and what became of everything it started.
    fn run_killed(
        &self,
        what: &str,
        args: &[&std::ffi::OsStr],
    ) -> (Result<String, BackupError>, Killed) {
        let killed = std::cell::Cell::new(Killed::No);
        let result = self.run_with(what, args, &killed);
        (result, killed.get())
    }

    /// [`Self::run`] where every repository the command may write (its
    /// `guards`) is looked at as the command begins, and has the locks it
    /// left dealt with if the command was stopped, once all it started is
    /// gone (INVARIANT 5). A stop that left a lock in the owner's folder is
    /// said so: [`BackupError::LockLeft`].
    fn run_in(
        &self,
        what: &str,
        args: &[&std::ffi::OsStr],
        guards: &[&Watched],
    ) -> Result<String, BackupError> {
        let snaps: Vec<Snapshot> = guards.iter().map(|g| g.snapshot()).collect();
        let (result, killed) = self.run_killed(what, args);
        if killed != Killed::No {
            let mut left = Vec::new();
            for (guard, snap) in guards.iter().zip(&snaps) {
                left.extend(guard.after_kill(self, snap, killed == Killed::Gone));
            }
            if matches!(
                result,
                Err(BackupError::Cancelled(_) | BackupError::TimedOut(_))
            ) && !left.is_empty()
            {
                return Err(BackupError::LockLeft(left.join(", ")));
            }
        }
        result
    }

    fn run_with(
        &self,
        what: &str,
        args: &[&std::ffi::OsStr],
        killed: &std::cell::Cell<Killed>,
    ) -> Result<String, BackupError> {
        use std::sync::atomic::{AtomicU64, Ordering};
        if self.cancelled() {
            return Err(BackupError::Cancelled(what.to_string()));
        }
        let mut c = self.command();
        // A path made canonical on Windows carries `\\?\`, which git refuses.
        c.args(args.iter().map(|a| match a.to_str() {
            Some(text) if text.contains(r"\\?\") => {
                std::ffi::OsString::from(crate::root::without_extended_prefix(text))
            }
            _ => (*a).to_os_string(),
        }));
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            c.process_group(0);
        }
        let mut child = crate::lock::spawn(&mut c).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => BackupError::NoGit,
            kind => BackupError::Io(kind),
        })?;
        let started = Instant::now();
        // Milliseconds since `started` when git last said anything.
        let heard = std::sync::Arc::new(AtomicU64::new(0));
        let drain = |pipe: Option<Box<dyn std::io::Read + Send>>, cap: usize| {
            let (tx, rx) = std::sync::mpsc::channel();
            let heard = heard.clone();
            std::thread::spawn(move || {
                let mut bytes = Vec::new();
                if let Some(mut p) = pipe {
                    let mut buf = [0u8; 8192];
                    while let Ok(n) = p.read(&mut buf) {
                        if n == 0 {
                            break;
                        }
                        heard.store(started.elapsed().as_millis() as u64, Ordering::Relaxed);
                        // Bounded: a git that says this much is kept only
                        // in part, its last part (where its errors are).
                        bytes.extend_from_slice(&buf[..n]);
                        if bytes.len() > cap.saturating_mul(2) {
                            bytes.drain(..bytes.len() - cap);
                        }
                    }
                }
                if bytes.len() > cap {
                    bytes.drain(..bytes.len() - cap);
                }
                let _ = tx.send(bytes);
            });
            rx
        };
        use std::io::Read;
        // What it answers is kept whole (a listing of every file); what it
        // says of itself, in part.
        let stdout = drain(child.stdout.take().map(|p| Box::new(p) as _), usize::MAX);
        let stderr = drain(child.stderr.take().map(|p| Box::new(p) as _), OUTPUT_MAX);
        // Once git has ended normally, what it started is stopped (Unix:
        // its group; Windows has no group to signal, and nothing is looked
        // for: a process spawned per git call would cost more than the
        // leftovers it could find).
        #[cfg(unix)]
        let kill_leftovers = |pid: u32| {
            // SAFETY: kill has no preconditions; the group is git's own
            // (process_group(0) above).
            unsafe {
                libc::kill(-(pid as libc::pid_t), libc::SIGKILL);
            }
        };
        #[cfg(windows)]
        let kill_leftovers = |_: u32| {};
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            let quiet_for = started
                .elapsed()
                .saturating_sub(Duration::from_millis(heard.load(Ordering::Relaxed)));
            if self.cancelled() {
                killed.set(self.stop(&mut child));
                return Err(BackupError::Cancelled(what.to_string()));
            }
            if quiet_for > self.quiet {
                killed.set(self.stop(&mut child));
                return Err(BackupError::TimedOut(what.to_string()));
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        // What it started may still hold its pipes: not waited for long.
        kill_leftovers(child.id());
        let wait = Duration::from_secs(10);
        let stdout = stdout.recv_timeout(wait).unwrap_or_default();
        let stderr = stderr.recv_timeout(wait).unwrap_or_default();
        if status.success() {
            Ok(String::from_utf8_lossy(&stdout).trim().to_string())
        } else {
            let cut = stderr.len() >= OUTPUT_MAX;
            Err(BackupError::Git(what.to_string(), said(&stderr, cut)))
        }
    }
}

impl Git {
    /// Stop a git that is running, and everything it started (INVARIANT 5):
    /// SIGTERM to its process group first (git removes the lock files it
    /// holds when it is sent one, and a push's receiving end the objects it
    /// had not yet kept), then up to `term_grace` for the group to end, and
    /// only then SIGKILL. The leader is reaped as the group is waited for
    /// (a zombie is still in its group). What became of the group.
    ///
    /// Windows has no signal to send a group: the nearest thing the code has
    /// is `taskkill /T /F`, which ends the tree at once, so there is no
    /// grace there, and no reliance on git's own clean-up: the lock rules
    /// (INVARIANT 5) cover what is left.
    fn stop(&self, child: &mut std::process::Child) -> Killed {
        #[cfg(unix)]
        {
            let pid = child.id();
            // SAFETY: kill has no preconditions; the group is git's own
            // (process_group(0) when it was started).
            unsafe {
                libc::kill(-(pid as libc::pid_t), libc::SIGTERM);
            }
            let by = Instant::now() + self.term_grace;
            loop {
                let _ = child.try_wait();
                if group_empty(pid) {
                    return Killed::Gone;
                }
                if Instant::now() >= by {
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            // SAFETY: as above.
            unsafe {
                libc::kill(-(pid as libc::pid_t), libc::SIGKILL);
            }
            let _ = child.kill();
            let _ = child.wait();
            group_gone(pid)
        }
        #[cfg(windows)]
        {
            let _ = std::process::Command::new("taskkill")
                .args(["/T", "/F", "/PID", &child.id().to_string()])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            let _ = child.kill();
            let _ = child.wait();
            Killed::Gone
        }
    }
}

/// The most kept of what a git says of itself.
const OUTPUT_MAX: usize = 1 << 20;

/// What became of a git command that was killed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Killed {
    /// It ended by itself.
    No,
    /// Killed, and it and everything it started is gone.
    Gone,
    /// Killed, but something it started was still there when we stopped
    /// looking (GROUP_GONE_WAIT).
    Lingering,
}

/// How long, after a git was killed, to wait for everything it started to
/// be gone.
pub const GROUP_GONE_WAIT: Duration = Duration::from_secs(2);

/// Whether the process group `pid` led has no process left.
#[cfg(unix)]
fn group_empty(pid: u32) -> bool {
    // SAFETY: signal 0 only asks whether a process is in the group.
    let found = unsafe { libc::kill(-(pid as libc::pid_t), 0) };
    found != 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
}

/// Wait until the process group `pid` led has no process left, for
/// GROUP_GONE_WAIT at most: a killed process can run a few more
/// instructions before the kernel takes it, and could take a lock in them.
fn group_gone(pid: u32) -> Killed {
    #[cfg(unix)]
    {
        let by = Instant::now() + GROUP_GONE_WAIT;
        loop {
            if group_empty(pid) {
                return Killed::Gone;
            }
            if Instant::now() >= by {
                return Killed::Lingering;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    #[cfg(windows)]
    {
        // taskkill /T /F ended the tree before it returned.
        let _ = pid;
        Killed::Gone
    }
}

/// A shared place for the lock files dealt with (see
/// [`Git::with_lock_notes`]).
#[derive(Clone, Default)]
pub struct LockNotes(std::sync::Arc<std::sync::Mutex<Vec<LockNote>>>);

impl LockNotes {
    /// What was dealt with since this was last called.
    pub fn take(&self) -> Vec<LockNote> {
        std::mem::take(&mut *self.0.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

/// A repository a git command may leave locks in when it is stopped.
pub struct Watched {
    dir: PathBuf,
    /// `archive`, `folder` or `repository`.
    kind: &'static str,
}

/// A repository as it was when a command began: its folder opened once
/// (never again by its path: INVARIANT 5), and the locks already in it.
struct Snapshot {
    handle: Option<crate::git_locks::RepoDir>,
    before: std::collections::BTreeSet<String>,
}

impl Watched {
    /// The book's own archive, `dir` (`<book>/archive.git`): private to
    /// this program, one git of ours at a time.
    fn archive(dir: &Path) -> Self {
        Self {
            dir: dir.to_path_buf(),
            kind: "archive",
        }
    }

    /// A target repository on this computer, the owner's.
    fn target(dir: &Path, kind: &'static str) -> Self {
        Self {
            dir: dir.to_path_buf(),
            kind,
        }
    }

    fn is_archive(&self) -> bool {
        self.kind == "archive"
    }

    fn open(&self) -> std::io::Result<crate::git_locks::RepoDir> {
        if self.is_archive() {
            match (
                self.dir.parent(),
                self.dir.file_name().and_then(|n| n.to_str()),
            ) {
                (Some(book), Some(name)) => crate::git_locks::RepoDir::open_in(book, name),
                _ => Err(std::io::ErrorKind::InvalidInput.into()),
            }
        } else {
            crate::git_locks::RepoDir::open(&self.dir)
        }
    }

    /// Looked at as a command begins.
    fn snapshot(&self) -> Snapshot {
        let handle = self.open().ok();
        let before = handle
            .as_ref()
            .map(|h| h.locks().into_iter().map(|f| f.lock).collect())
            .unwrap_or_default();
        Snapshot { handle, before }
    }

    /// The command was stopped. Once all it started is `gone`, deal with
    /// the locks it left (INVARIANT 5): in the archive, remove those that
    /// were not there when the command began; in the owner's folder remove
    /// nothing, and say so. The whole paths of the locks left there.
    fn after_kill(&self, git: &Git, snap: &Snapshot, gone: bool) -> Vec<String> {
        // A repository that didn't exist when the command began (a new
        // target's, made by an `init` that was stopped) is looked at, never
        // changed. The archive is only ever touched through the handle
        // opened before the command.
        let fresh;
        let handle = match (&snap.handle, self.is_archive()) {
            (Some(h), _) => h,
            (None, true) => return Vec::new(),
            (None, false) => match self.open() {
                Ok(h) => {
                    fresh = h;
                    &fresh
                }
                Err(_) => return Vec::new(),
            },
        };
        let mut left = Vec::new();
        for found in handle.locks() {
            if snap.before.contains(&found.lock) {
                continue;
            }
            let lock = found.lock;
            if self.is_archive() {
                if !gone {
                    git.note(LockNote {
                        repo: self.kind,
                        lock,
                        removed: false,
                        why: "what git started was still running",
                        path: None,
                    });
                } else if handle.remove(&lock).unwrap_or(false) {
                    git.note(LockNote {
                        repo: self.kind,
                        lock,
                        removed: true,
                        why: "git was stopped",
                        path: None,
                    });
                }
            } else {
                let path = handle.path().join(&lock).display().to_string();
                git.note(LockNote {
                    repo: self.kind,
                    lock,
                    removed: false,
                    why: if gone {
                        "git was stopped while it held it: the owner's folder is never changed by this program"
                    } else {
                        "what git started was still running"
                    },
                    path: Some(path.clone()),
                });
                left.push(path);
            }
        }
        left
    }
}

/// The locks of the book's archive in `book_dir` that are older than
/// ARCHIVE_STALE, removed, each noted in `notes`; and, when `say_young`,
/// each younger one noted as left. The archive is opened once and only
/// regular files named as git names its locks are touched (INVARIANT 5a).
fn clear_stale_archive_locks(
    book_dir: &Path,
    removed_why: &'static str,
    young: Option<&'static str>,
    notes: &mut Vec<LockNote>,
) {
    let Ok(handle) = crate::git_locks::RepoDir::open_in(book_dir, ARCHIVE_DIR) else {
        return;
    };
    let now = std::time::SystemTime::now();
    for found in handle.locks() {
        let old = found
            .modified
            .and_then(|t| now.duration_since(t).ok())
            .is_some_and(|age| age >= ARCHIVE_STALE);
        if old {
            if handle.remove(&found.lock).unwrap_or(false) {
                notes.push(LockNote {
                    repo: "archive",
                    lock: found.lock,
                    removed: true,
                    why: removed_why,
                    path: None,
                });
            }
        } else if let Some(why) = young {
            notes.push(LockNote {
                repo: "archive",
                lock: found.lock,
                removed: false,
                why,
                path: None,
            });
        }
    }
}

/// The locks a git of ours left in the book's archive at an earlier time
/// (killed with the server, a power cut), for the caller to say it removed
/// them: those older than ARCHIVE_STALE are removed, younger ones are left
/// and noted as left (the next backup tries again, and removes them once
/// they are old). Call it while this process holds the book's store lock:
/// no other wanderbooks process runs git on the archive (INVARIANT 5a).
pub fn clear_archive_locks(book_dir: &Path) -> Vec<LockNote> {
    let mut notes = Vec::new();
    clear_stale_archive_locks(
        book_dir,
        "found when the books opened, older than ten minutes",
        Some("younger than ten minutes when the books opened: left, the next backup tries again"),
        &mut notes,
    );
    notes
}

/// What git said when it failed: its messages without its progress, the
/// last whole lines of it (never a line cut part way, which could hide a
/// path from those who hide paths, INVARIANT 4); `cut` when only the last
/// of what it said was kept (its first line may be part of one).
fn said(stderr: &[u8], cut: bool) -> String {
    let text = String::from_utf8_lossy(stderr);
    let text = match (cut, text.find(['\r', '\n'])) {
        (true, Some(at)) => &text[at + 1..],
        (true, None) => "",
        (false, _) => &text[..],
    };
    let lines: Vec<&str> = text
        .split(['\r', '\n'])
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.contains("% (") && !l.ends_with(", done."))
        .collect();
    let mut kept: Vec<&str> = Vec::new();
    let mut size = 0;
    for line in lines.iter().rev() {
        if line.len() > 1000 || size + line.len() > 1000 {
            break;
        }
        size += line.len() + 1;
        kept.push(line);
    }
    kept.reverse();
    kept.join("\n")
}

/// One backup made: its commit and the journal head it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Made {
    /// The commit's id.
    pub commit: String,
    /// The journal's head it holds: seq and hash.
    pub head: (u64, String),
}

fn os(s: &str) -> &std::ffi::OsStr {
    std::ffi::OsStr::new(s)
}

/// The archive of the book in `book_dir`, made (bare, private) if missing.
/// No attributes rule of its own (which could change the bytes it keeps)
/// is ever left in it.
fn archive(git: &Git, book_dir: &Path) -> Result<PathBuf, BackupError> {
    let dir = book_dir.join(ARCHIVE_DIR);
    if !dir.join("HEAD").is_file() {
        crate::root::private_book_folder(book_dir, ARCHIVE_DIR)
            .map_err(|e| BackupError::Damaged(e.to_string()))?;
        // A git killed in the middle of making it leaves HEAD's lock, which
        // would stop every later `init` (INVARIANT 5).
        git.run_in(
            "init",
            &[os("init"), os("--bare"), os("-q"), dir.as_os_str()],
            &[&Watched::archive(&dir)],
        )?;
    }
    let _ = std::fs::remove_file(dir.join("info").join("attributes"));
    // Nor one in the book's folder: refused, never followed.
    if book_dir.join(".gitattributes").exists() {
        return Err(BackupError::Damaged(
            "the book's folder holds a file (.gitattributes) no book keeps".into(),
        ));
    }
    Ok(dir)
}

/// Commit the book in `book_dir`, whose journal's head is `head`, to its
/// archive (INVARIANT 1): the commit, or None when nothing changed since
/// the last. The caller holds the store's lock while this runs.
pub fn commit(
    git: &Git,
    book_dir: &Path,
    book: &str,
    head: (u64, String),
) -> Result<Option<Made>, BackupError> {
    let archive = archive(git, book_dir)?;
    clear_old_locks(git, book_dir);
    let guard = Watched::archive(&archive);
    let git_dir = format!("--git-dir={}", archive.display());
    let work_tree = format!("--work-tree={}", book_dir.display());
    let paths: Vec<&str> = BACKED_UP
        .iter()
        .copied()
        .filter(|p| book_dir.join(p).exists())
        .collect();
    if !paths.is_empty() {
        let mut add = vec![
            os(&git_dir),
            os(&work_tree),
            os("add"),
            os("-A"),
            os("-f"),
            os("--"),
        ];
        add.extend(paths.iter().map(|p| os(p)));
        git.run_in("add", &add, &[&guard])?;
    }
    // What was backed up and is now gone altogether (a vault purged to
    // nothing) goes from the next commit too.
    let gone: Vec<&str> = BACKED_UP
        .iter()
        .copied()
        .filter(|p| !book_dir.join(p).exists())
        .collect();
    if !gone.is_empty() {
        let mut rm = vec![
            os(&git_dir),
            os(&work_tree),
            os("rm"),
            os("-r"),
            os("-q"),
            os("--cached"),
            os("--ignore-unmatch"),
            os("--"),
        ];
        rm.extend(gone.iter().map(|p| os(p)));
        git.run_in("rm", &rm, &[&guard])?;
    }
    // Every file there is in it, nothing else (INVARIANT 1).
    let listed = git.run(
        "ls-files",
        &[os(&git_dir), os(&work_tree), os("ls-files"), os("-z")],
    )?;
    let mut indexed: Vec<String> = listed
        .split('\0')
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect();
    indexed.sort();
    let mut on_disk = Vec::new();
    for p in &paths {
        files_under(git, book_dir, Path::new(p), &mut on_disk)?;
    }
    on_disk.sort();
    if on_disk
        .iter()
        .any(|p| p.rsplit('/').next() == Some(".gitattributes"))
    {
        return Err(BackupError::Damaged(
            "the book's folder holds a file (.gitattributes) no book keeps".into(),
        ));
    }
    if indexed != on_disk {
        return Err(BackupError::Damaged(
            "the backup wouldn't hold the books exactly as they are".into(),
        ));
    }
    let unchanged = git
        .run(
            "diff",
            &[
                os(&git_dir),
                os(&work_tree),
                os("diff"),
                os("--cached"),
                os("--quiet"),
            ],
        )
        .is_ok();
    let has_commits = git
        .run(
            "rev-parse",
            &[
                os(&git_dir),
                os("rev-parse"),
                os("-q"),
                os("--verify"),
                os("HEAD"),
            ],
        )
        .is_ok();
    if unchanged && has_commits {
        return Ok(None);
    }
    // The books it holds by name too (a restore tells their own from
    // others').
    let message = format!("seq {} head {}\n\nbook {book}", head.0, head.1);
    git.run_in(
        "commit",
        &[
            os(&git_dir),
            os(&work_tree),
            os("commit"),
            os("-q"),
            os("--no-verify"),
            os("--allow-empty"),
            os("-m"),
            os(&message),
        ],
        &[&guard],
    )?;
    let commit = git.run("rev-parse", &[os(&git_dir), os("rev-parse"), os("HEAD")])?;
    Ok(Some(Made { commit, head }))
}

/// Stage the book's stored files in its archive ahead of a commit, outside
/// the store's lock: they never change once written (named by their
/// hash), so the commit under the lock has only the new ones to read.
pub fn stage_blobs(git: &Git, book_dir: &Path) -> Result<(), BackupError> {
    let blobs = crate::store::BLOBS_DIR;
    if !book_dir.join(blobs).is_dir() {
        return Ok(());
    }
    let archive = archive(git, book_dir)?;
    clear_old_locks(git, book_dir);
    let guard = Watched::archive(&archive);
    let git_dir = format!("--git-dir={}", archive.display());
    let work_tree = format!("--work-tree={}", book_dir.display());
    git.run_in(
        "add",
        &[
            os(&git_dir),
            os(&work_tree),
            os("add"),
            os("-A"),
            os("-f"),
            os("--"),
            os(blobs),
        ],
        &[&guard],
    )?;
    Ok(())
}

/// The locks a git left behind (the server killed, a power cut) would
/// refuse every later backup: those older than ARCHIVE_STALE are removed,
/// each noted, so none a git still at work holds is touched (INVARIANT 5a).
fn clear_old_locks(git: &Git, book_dir: &Path) {
    let mut notes = Vec::new();
    clear_stale_archive_locks(
        book_dir,
        "older than ten minutes when a backup began",
        None,
        &mut notes,
    );
    for note in notes {
        git.note(note);
    }
}

/// Every file under `rel` in `base` (or `rel` itself), as paths relative to
/// `base` with `/` between their parts.
fn files_under(
    git: &Git,
    base: &Path,
    rel: &Path,
    out: &mut Vec<String>,
) -> Result<(), BackupError> {
    // Looked at once per entry: a folder of any size is left promptly
    // (INVARIANT 6).
    git.check("listing the files")?;
    let full = base.join(rel);
    let meta = std::fs::symlink_metadata(&full)?;
    if meta.is_dir() {
        for entry in std::fs::read_dir(&full)? {
            files_under(git, base, &rel.join(entry?.file_name()), out)?;
        }
    } else {
        let parts: Vec<String> = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        out.push(parts.join("/"));
    }
    Ok(())
}

/// What `git init --bare` writes in a repository's configuration, and
/// nothing else a backup's repository may hold: a setting of its own (a
/// command to run, objects borrowed from elsewhere, a work tree to write)
/// could make a push or a clone do more than move the books.
const PLAIN_SETTINGS: [&str; 9] = [
    "core.repositoryformatversion",
    "core.filemode",
    "core.bare",
    "core.logallrefupdates",
    "core.ignorecase",
    "core.precomposeunicode",
    "core.symlinks",
    "extensions.objectformat",
    "extensions.refstorage",
];

/// Refuse the repository on this computer at `address` unless it is plain:
/// its configuration only what `git init --bare` writes, no objects of
/// another's.
fn plain_repository(git: &Git, address: &str) -> Result<(), BackupError> {
    let dir = PathBuf::from(address.strip_prefix("file://").unwrap_or(address));
    let not_plain = || {
        BackupError::Damaged(
            "its repository holds settings of its own: back up to one made only for these books"
                .into(),
        )
    };
    // A bare repository right there, as git will find it: nothing that
    // sends git somewhere else first (a .git in it, another repository's
    // files named in it), and not a name git would look beside.
    // git takes it as a repository (and so looks no further, beside it)
    // only when it holds objects, refs and a HEAD naming one.
    let head = std::fs::read_to_string(dir.join("HEAD")).unwrap_or_default();
    let head = head.trim();
    let names_one = head.strip_prefix("ref: refs/").is_some()
        || (head.len() >= 40 && head.bytes().all(|b| b.is_ascii_hexdigit()));
    if !names_one
        || !dir.join("config").is_file()
        || !dir.join("objects").is_dir()
        || !dir.join("refs").is_dir()
    {
        return Err(BackupError::Damaged(
            "that isn't a backup's repository: name the repository itself".into(),
        ));
    }
    for elsewhere in [".git", "commondir", "gitdir"] {
        if std::fs::symlink_metadata(dir.join(elsewhere)).is_ok() {
            return Err(not_plain());
        }
    }
    // Nor a link where its own parts are (which would send what is
    // written there elsewhere); the folder holding it may be one.
    let trimmed = PathBuf::from(
        address
            .strip_prefix("file://")
            .unwrap_or(address)
            .trim_end_matches('/'),
    );
    for part in [
        trimmed.clone(),
        dir.join("HEAD"),
        dir.join("config"),
        dir.join("objects"),
        dir.join("refs"),
    ] {
        if std::fs::symlink_metadata(&part).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(not_plain());
        }
    }
    // Nor anything git would take in its place, beside it.
    let bare = address
        .strip_prefix("file://")
        .unwrap_or(address)
        .trim_end_matches('/');
    for beside in [".git", ".bundle"] {
        if std::fs::symlink_metadata(format!("{bare}{beside}")).is_ok() {
            return Err(not_plain());
        }
    }
    if dir.join("objects").join("info").join("alternates").exists() {
        return Err(not_plain());
    }
    let config = dir.join("config");
    let file = format!("--file={}", config.display());
    let names = git.run(
        "config",
        &[os("config"), os(&file), os("--list"), os("--name-only")],
    )?;
    // A copy made with `git clone --bare` also names where it came from:
    // only that.
    let plain = |name: &str| {
        let name = name.trim().to_ascii_lowercase();
        let tail = |section: &str, keys: &[&str]| {
            name.strip_prefix(section)
                .and_then(|rest| rest.rsplit_once('.'))
                .is_some_and(|(_, key)| keys.contains(&key))
        };
        PLAIN_SETTINGS.contains(&name.as_str())
            || tail("remote.", &["url", "fetch", "mirror"])
            || tail("branch.", &["remote", "merge"])
            // Made with `git init --bare --shared`: harmless.
            || name == "core.sharedrepository"
            || name == "receive.denynonfastforwards"
    };
    if names.lines().any(|n| !plain(n)) {
        return Err(not_plain());
    }
    Ok(())
}

/// Whether git reads `address` as a repository on this computer (a path,
/// or `file://`), not one it reaches over the network (`ssh://`,
/// `https://`, or `host:path`).
pub fn is_local(address: &str) -> bool {
    if let Some((scheme, _)) = address.split_once("://") {
        return scheme == "file";
    }
    match address.split_once(':') {
        // host:path, where the host part holds no slash.
        Some((host, _)) => host.contains('/'),
        None => true,
    }
}

/// Where a backup goes (D-30).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// The owner's private repository, as git names it.
    Remote(String),
    /// The owner's folder: a repository in it named `<book>.git`.
    Folder(PathBuf),
}

impl Target {
    /// Where the backup of the book `book` is, as git reads it.
    pub fn source(&self, book: &str) -> String {
        match self {
            Target::Remote(url) => url.clone(),
            Target::Folder(folder) => folder.join(format!("{book}.git")).display().to_string(),
        }
    }

    /// What it is called in records: "repository" or "folder".
    pub fn kind(&self) -> &'static str {
        match self {
            Target::Remote(_) => "repository",
            Target::Folder(_) => "folder",
        }
    }
}

/// Push the archive of the book `book` in `book_dir` to `target`
/// (INVARIANT 2): the commit it now holds.
///
/// A folder target's repository is made only when `may_create` (it never
/// took a backup): one that took backups and is gone is a disk not
/// connected (a mount point with nothing mounted on it), never made again
/// where the disk should be.
pub fn push(
    git: &Git,
    book_dir: &Path,
    book: &str,
    target: &Target,
    may_create: bool,
) -> Result<String, BackupError> {
    let archive = archive(git, book_dir)?;
    let git_dir = format!("--git-dir={}", archive.display());
    let to = match target {
        // Never read as an option, never a command (protocols, above).
        Target::Remote(url) if url.starts_with('-') || url.trim().is_empty() => {
            return Err(BackupError::Damaged(
                "that isn't a repository address git takes".into(),
            ));
        }
        Target::Remote(url) => url.clone(),
        Target::Folder(folder) => {
            if !folder.is_dir() {
                return Err(BackupError::NoFolder);
            }
            let repo = folder.join(format!("{book}.git"));
            if !repo.join("HEAD").is_file() {
                if !may_create {
                    return Err(BackupError::NoBackupThere);
                }
                // The repository is new: any lock in it is ours, by the
                // same rules as a push's (INVARIANT 5b).
                git.run_in(
                    "init",
                    &[os("init"), os("--bare"), os("-q"), repo.as_os_str()],
                    &[&Watched::target(&repo, target.kind())],
                )?;
            }
            repo.display().to_string()
        }
    };
    if matches!(target, Target::Folder(_)) || is_local(&to) {
        plain_repository(git, &to)?;
    }
    // A repository on this computer is updated by a git we start, which
    // doesn't get our configuration: told, in its own words, to run no
    // hooks (a folder others can write could hold some).
    let exe = git.exe.display().to_string();
    let receive = format!("--receive-pack='{exe}' -c core.hooksPath=/dev/null receive-pack");
    // Its progress keeps it from counting as stalled (Git::run).
    let mut args = vec![
        os(&git_dir),
        os("push"),
        os("--progress"),
        os("--no-verify"),
    ];
    if matches!(target, Target::Folder(_)) || is_local(&to) {
        if exe.contains('\'') {
            return Err(BackupError::Damaged(
                "git is installed where its hooks can't be turned off (a quote in its path)".into(),
            ));
        }
        args.push(os(&receive));
    }
    args.extend([os(&to), os("HEAD:refs/heads/main")]);
    // The repositories this push may leave a lock in if it is killed: the
    // archive, and the target if it is on this computer (one far away
    // cleans up after its own connection's end, and isn't ours to touch).
    let mine = Watched::archive(&archive);
    let theirs = (matches!(target, Target::Folder(_)) || is_local(&to)).then(|| {
        Watched::target(
            Path::new(to.strip_prefix("file://").unwrap_or(&to)),
            target.kind(),
        )
    });
    let mut guards = vec![&mine];
    guards.extend(theirs.as_ref());
    match git.run_in("push", &args, &guards) {
        Ok(_) => {}
        Err(BackupError::Git(_, said))
            if said.contains("non-fast-forward")
                || said.contains("[rejected]")
                || said.contains("fetch first") =>
        {
            return Err(BackupError::Diverged(match target {
                Target::Remote(url) => url.clone(),
                Target::Folder(_) => "the backup folder".to_string(),
            }));
        }
        Err(e) => return Err(e),
    }
    git.run("rev-parse", &[os(&git_dir), os("rev-parse"), os("HEAD")])
}

/// A backup proven whole (INVARIANT 3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verified {
    /// Its journal's head: seq and hash.
    pub head: (u64, String),
    /// Stored files re-hashed.
    pub blobs: u64,
    /// Where it was cloned (the caller's scratch folder), its working tree
    /// the book as it was.
    pub tree: PathBuf,
    /// The id of the books it holds, as their last backup named them.
    pub book: Option<String>,
}

/// Clone the backup at `source` (a repository's address, or its folder:
/// `<book>.git` in a backup folder)
/// into `scratch` (a new folder) and prove it whole (INVARIANT 3).
pub fn verify(git: &Git, source: &str, scratch: &Path) -> Result<Verified, BackupError> {
    if scratch.exists() {
        return Err(BackupError::Damaged("its scratch folder is in use".into()));
    }
    // One on this computer is read as one far away would be (so it reports
    // its progress), once it is known to run nothing of its own.
    if is_local(source) {
        plain_repository(git, source)?;
    }
    // Stops at its next step once the books are let go (cancel.rs); what
    // it had cloned is the caller's scratch folder, removed by the caller.
    let _stoppable = git.cancel.clone().map(crate::cancel::scope);
    let store_err = |e: crate::StoreError| match e {
        crate::StoreError::Cancelled => BackupError::Cancelled("verifying".into()),
        e => BackupError::Damaged(e.to_string()),
    };
    git.phase("clone");
    git.check("clone")?;
    git.run(
        "clone",
        &[
            os("clone"),
            os("--progress"),
            os("--no-local"),
            os("--no-checkout"),
            os("--branch"),
            os("main"),
            os("--"),
            os(source),
            scratch.as_os_str(),
        ],
    )?;
    git.phase("tree");
    let git_dir = format!("--git-dir={}", scratch.join(".git").display());
    // Its tree before any of it is written out: plain files, only where a
    // book keeps what is backed up. A link or a submodule (which could
    // point the checkout, or the store opened on it, anywhere) or anything
    // else is refused.
    let listed = git.run(
        "ls-tree",
        &[
            os(&git_dir),
            os("ls-tree"),
            os("-r"),
            os("-z"),
            os("--full-tree"),
            os("HEAD"),
        ],
    )?;
    for entry in listed.split('\0').filter(|e| !e.is_empty()) {
        git.check("reading its tree")?;
        let (meta, path) = entry.split_once('\t').unwrap_or((entry, ""));
        let mode = meta.split(' ').next().unwrap_or("");
        let top = path.split('/').next().unwrap_or("");
        if !["100644", "100755"].contains(&mode) || !BACKED_UP.contains(&top) {
            return Err(BackupError::Damaged(format!(
                "it holds {path:?}, which no book keeps"
            )));
        }
    }
    git.phase("checkout");
    let work_tree = format!("--work-tree={}", scratch.display());
    git.run(
        "checkout",
        &[
            os(&git_dir),
            os(&work_tree),
            os("checkout"),
            os("-q"),
            os("-f"),
            os("main"),
        ],
    )?;
    git.phase("replay");
    let mut store = crate::Store::open(scratch).map_err(store_err)?;
    git.phase("verify");
    store.verify_and_heal().map_err(store_err)?;
    git.phase("blobs");
    let blobs = store.verify_blobs().map_err(store_err)?;
    let head = store.head();
    // Every TIN record the books name is there, and the key that opens
    // them.
    let named: Vec<String> = store
        .ledger()
        .parties()
        .flat_map(|p| {
            [
                p.tin.as_ref().and_then(|t| t.vault_record.clone()),
                p.w9.as_ref().and_then(|w| w.vault_record.clone()),
            ]
        })
        .flatten()
        .collect();
    drop(store);
    git.phase("vault");
    if !named.is_empty() {
        use sha2::{Digest, Sha256};
        let vault = scratch.join(crate::root::VAULT_DIR);
        let mut held = std::collections::BTreeSet::new();
        if vault.is_dir() {
            for f in std::fs::read_dir(&vault)? {
                git.check("reading its vault")?;
                held.insert(hex::encode(Sha256::digest(std::fs::read(f?.path())?)));
            }
        }
        if let Some(missing) = named.iter().find(|h| !held.contains(*h)) {
            return Err(BackupError::Damaged(format!(
                "a TIN record the books name isn't in it ({})",
                &missing[..12.min(missing.len())]
            )));
        }
        let key = scratch
            .join(crate::root::KEYS_DIR)
            .join(crate::root::LEGACY_KEY_FILE);
        if !key.is_file() {
            return Err(BackupError::Damaged(
                "its TIN records are in it, but not the key that opens them".into(),
            ));
        }
    }
    // The head the backup records: its commit says "seq N head H".
    let recorded = git.run(
        "log",
        &[os(&git_dir), os("log"), os("-1"), os("--format=%s")],
    )?;
    if recorded.trim() != format!("seq {} head {}", head.0, head.1) {
        return Err(BackupError::Damaged(format!(
            "its journal ends at event {}, but the backup records another head",
            head.0
        )));
    }
    let body = git.run(
        "log",
        &[os(&git_dir), os("log"), os("-1"), os("--format=%b")],
    )?;
    let book = body
        .lines()
        .find_map(|l| l.trim().strip_prefix("book "))
        .map(str::to_string);
    Ok(Verified {
        head,
        blobs,
        tree: scratch.to_path_buf(),
        book,
    })
}

/// A restore drill (INVARIANT 3): the backup of the book `book` at
/// `target` cloned into `scratch` (a new folder, removed after), proven
/// whole, and its journal a byte prefix of `live` (the book's journal
/// now). Its head.
pub fn drill(
    git: &Git,
    target: &Target,
    book: &str,
    live: &Path,
    scratch: &Path,
) -> Result<(u64, String), BackupError> {
    if let Target::Folder(folder) = target
        && !folder.is_dir()
    {
        return Err(BackupError::NoFolder);
    }
    let checked = verify(git, &target.source(book), scratch).and_then(|v| {
        git.phase("prefix");
        match is_prefix_with(live, &v.tree.join(crate::JOURNAL_FILE), || {
            git.check("comparing journals")
        }) {
            Ok(true) => Ok(v.head),
            Ok(false) => Err(BackupError::Damaged(
                "its journal isn't this book's history".into(),
            )),
            Err(e) => Err(e),
        }
    });
    let _ = std::fs::remove_dir_all(scratch);
    checked
}

/// Prove the book's own archive holds the book in `book_dir` exactly as it
/// is now, its journal's head `head` (INVARIANT 3, and more): the archive
/// cloned into `scratch` (a new folder, removed after) and proven whole as
/// a restore proves a backup, its head `head`, its journal the book's byte
/// for byte, and its TIN vault and keys the book's, file for file and byte
/// for byte. What the format upgrade asks of the backup it makes first
/// (plan/served.md 3.9: "a verified local backup that includes the
/// keys"). The caller holds the store's lock, so the book can't move
/// while it is compared.
pub fn prove_archive(
    git: &Git,
    book_dir: &Path,
    head: &(u64, String),
    scratch: &Path,
) -> Result<Proven, BackupError> {
    let archive = book_dir.join(ARCHIVE_DIR);
    let source = archive.display().to_string();
    let proven = verify(git, &source, scratch).and_then(|v| {
        if v.head != *head {
            return Err(BackupError::Damaged(format!(
                "it holds the books through event {}, and they are at event {}",
                v.head.0, head.0
            )));
        }
        if std::fs::read(book_dir.join(crate::JOURNAL_FILE))?
            != std::fs::read(v.tree.join(crate::JOURNAL_FILE))?
        {
            return Err(BackupError::Damaged(
                "its journal isn't the books' own, byte for byte".into(),
            ));
        }
        git.phase("compare");
        git.check("comparing")?;
        for kept in [crate::root::VAULT_DIR, crate::root::KEYS_DIR] {
            if held_files(git, book_dir, kept)? != held_files(git, &v.tree, kept)? {
                return Err(BackupError::Damaged(format!(
                    "its {} isn't the books' own, file for file",
                    if kept == crate::root::KEYS_DIR {
                        "keys folder"
                    } else {
                        "TIN vault"
                    }
                )));
            }
        }
        let git_dir = format!("--git-dir={}", v.tree.join(".git").display());
        let commit = git.run("rev-parse", &[os(&git_dir), os("rev-parse"), os("HEAD")])?;
        Ok(Proven {
            head: v.head,
            blobs: v.blobs,
            commit,
        })
    });
    let _ = std::fs::remove_dir_all(scratch);
    proven
}

/// The book's archive proven to hold the book as it is ([`prove_archive`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proven {
    /// The journal's head it holds: the book's.
    pub head: (u64, String),
    /// Stored files re-hashed.
    pub blobs: u64,
    /// The commit proven: the archive's latest.
    pub commit: String,
}

/// Every file under `rel` in `base`, by its path there, with its bytes
/// (none when `rel` isn't there).
fn held_files(git: &Git, base: &Path, rel: &str) -> Result<Vec<(String, Vec<u8>)>, BackupError> {
    if std::fs::symlink_metadata(base.join(rel)).is_err() {
        return Ok(Vec::new());
    }
    let mut names = Vec::new();
    files_under(git, base, Path::new(rel), &mut names)?;
    names.sort();
    names
        .into_iter()
        .map(|n| {
            git.check("reading the files")?;
            let bytes = std::fs::read(base.join(&n))?;
            Ok((n, bytes))
        })
        .collect()
}

/// Whether the journal at `backup` is a byte prefix of the one at `live`
/// (a drill: the backup holds the book's history, nothing else).
pub fn is_prefix(live: &Path, backup: &Path) -> std::io::Result<bool> {
    is_prefix_with(live, backup, || Ok(())).map_err(|e| match e {
        BackupError::Io(kind) => std::io::Error::from(kind),
        other => std::io::Error::other(other.to_string()),
    })
}

/// [`is_prefix`], asking `each` before every chunk read (a step that has
/// to stop looks for its stop there, INVARIANT 6).
fn is_prefix_with(
    live: &Path,
    backup: &Path,
    mut each: impl FnMut() -> Result<(), BackupError>,
) -> Result<bool, BackupError> {
    use std::io::Read;
    let mut a = std::fs::File::open(live)?;
    let mut b = std::fs::File::open(backup)?;
    let (mut x, mut y) = (vec![0u8; 64 * 1024], vec![0u8; 64 * 1024]);
    loop {
        each()?;
        let n = b.read(&mut y)?;
        if n == 0 {
            return Ok(true);
        }
        let mut got = 0;
        while got < n {
            let m = a.read(&mut x[got..n])?;
            if m == 0 {
                return Ok(false);
            }
            got += m;
        }
        if x[..n] != y[..n] {
            return Ok(false);
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::Store;
    use ledger_core::{AccountType, Currency, Money, Side};

    fn usd(s: &str) -> Money {
        Money::new(s.parse().unwrap(), Currency::USD)
    }

    /// A book with a few entries and a stored file.
    fn book(dir: &Path) -> Store {
        let mut s = Store::create(dir, Currency::USD).unwrap();
        let cash = s
            .add_account("1000", "Cash", AccountType::Asset, None, false)
            .unwrap();
        let cap = s
            .add_account("3000", "Capital", AccountType::Equity, None, false)
            .unwrap();
        let e = s
            .create_entry(
                chrono::NaiveDate::from_ymd_opt(2026, 3, 1).unwrap(),
                "Start",
            )
            .unwrap();
        s.add_line(e, cash, Side::Debit, usd("100.00")).unwrap();
        s.add_line(e, cap, Side::Credit, usd("100.00")).unwrap();
        s.post_entry(e).unwrap();
        s.attach_receipt(Some(e), "r.pdf", "application/pdf", b"%PDF-1.4 r", None)
            .unwrap();
        s
    }

    #[test]
    fn a_backup_is_the_book_and_proves_whole() {
        let git = Git::find().expect("git is installed");
        let dir = tempfile::tempdir().unwrap();
        let b = dir.path().join("book");
        let mut s = book(&b);
        let made = commit(&git, &b, "acme", s.head())
            .unwrap()
            .expect("a first backup");
        assert_eq!(made.head, s.head());
        // Nothing new: no commit.
        assert_eq!(commit(&git, &b, "acme", s.head()).unwrap(), None);
        // To a folder, then proven from it.
        let folder = dir.path().join("disk");
        std::fs::create_dir(&folder).unwrap();
        let target = Target::Folder(folder.clone());
        assert_eq!(push(&git, &b, "acme", &target, true).unwrap(), made.commit);
        let v = verify(
            &git,
            folder.join("acme.git").to_str().unwrap(),
            &dir.path().join("v1"),
        )
        .unwrap();
        assert_eq!(v.head, s.head());
        assert_eq!(v.blobs, 1);
        assert!(
            is_prefix(
                &b.join(crate::JOURNAL_FILE),
                &v.tree.join(crate::JOURNAL_FILE)
            )
            .unwrap()
        );
        // More writes: a new commit, pushed forward.
        s.add_account("6000", "Supplies", AccountType::Expense, None, false)
            .unwrap();
        let next = commit(&git, &b, "acme", s.head()).unwrap().unwrap();
        assert_ne!(next.commit, made.commit);
        push(&git, &b, "acme", &target, true).unwrap();
        let v2 = verify(
            &git,
            folder.join("acme.git").to_str().unwrap(),
            &dir.path().join("v2"),
        )
        .unwrap();
        assert_eq!(v2.head, s.head());
        // The older backup is a prefix of the live journal; the live one is
        // not a prefix of the older.
        assert!(
            is_prefix(
                &b.join(crate::JOURNAL_FILE),
                &v.tree.join(crate::JOURNAL_FILE)
            )
            .unwrap()
        );
        assert!(
            !is_prefix(
                &v.tree.join(crate::JOURNAL_FILE),
                &b.join(crate::JOURNAL_FILE)
            )
            .unwrap()
        );
        // The database, its lock and the side channels aren't backed up.
        let listed = git
            .run(
                "ls-tree",
                &[
                    os(&format!("--git-dir={}", b.join(ARCHIVE_DIR).display())),
                    os("ls-tree"),
                    os("--name-only"),
                    os("HEAD"),
                ],
            )
            .unwrap();
        let mut names: Vec<&str> = listed.lines().collect();
        names.sort();
        assert_eq!(names, ["blobs", crate::JOURNAL_FILE]);
    }

    #[test]
    fn a_folder_targets_hooks_never_run() {
        use std::os::unix::fs::PermissionsExt;
        let git = Git::find().expect("git is installed");
        let dir = tempfile::tempdir().unwrap();
        let b = dir.path().join("book");
        let s = book(&b);
        commit(&git, &b, "acme", s.head()).unwrap();
        let folder = dir.path().join("disk");
        let repo = folder.join("acme.git");
        std::fs::create_dir(&folder).unwrap();
        git.run(
            "init",
            &[os("init"), os("--bare"), os("-q"), repo.as_os_str()],
        )
        .unwrap();
        let ran = dir.path().join("ran");
        std::fs::create_dir_all(repo.join("hooks")).unwrap();
        for hook in ["pre-receive", "update", "post-receive", "post-update"] {
            let h = repo.join("hooks").join(hook);
            std::fs::write(
                &h,
                format!("#!/bin/sh\necho {hook} >> '{}'\n", ran.display()),
            )
            .unwrap();
            std::fs::set_permissions(&h, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        // As a folder, and named as a repository by its path.
        push(&git, &b, "acme", &Target::Folder(folder.clone()), true).unwrap();
        push(
            &git,
            &b,
            "acme",
            &Target::Remote(repo.display().to_string()),
            true,
        )
        .unwrap();
        assert!(
            !ran.exists(),
            "a hook ran: {:?}",
            std::fs::read_to_string(&ran)
        );
        let v = verify(&git, repo.to_str().unwrap(), &dir.path().join("v")).unwrap();
        assert_eq!(v.head, s.head());
    }

    #[test]
    fn a_target_that_moved_on_is_never_overwritten() {
        let git = Git::find().expect("git is installed");
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("disk");
        std::fs::create_dir(&folder).unwrap();
        let target = Target::Folder(folder.clone());
        // Two books of the same id push to one folder: the second has its
        // own history, which isn't the first's.
        let one = dir.path().join("one");
        let s = book(&one);
        commit(&git, &one, "acme", s.head()).unwrap();
        push(&git, &one, "acme", &target, true).unwrap();
        let two = dir.path().join("two");
        let mut s2 = book(&two);
        s2.add_account("6000", "Supplies", AccountType::Expense, None, false)
            .unwrap();
        commit(&git, &two, "acme", s2.head()).unwrap();
        let e = push(&git, &two, "acme", &target, true).unwrap_err();
        assert!(matches!(e, BackupError::Diverged(_)), "{e}");
        // The folder still holds the first's.
        let v = verify(
            &git,
            folder.join("acme.git").to_str().unwrap(),
            &dir.path().join("v"),
        )
        .unwrap();
        assert_eq!(v.head, s.head());
    }

    #[test]
    fn a_missing_folder_or_a_strange_address_is_said_plainly() {
        let git = Git::find().expect("git is installed");
        let dir = tempfile::tempdir().unwrap();
        let b = dir.path().join("book");
        let s = book(&b);
        commit(&git, &b, "acme", s.head()).unwrap();
        let e = push(
            &git,
            &b,
            "acme",
            &Target::Folder(dir.path().join("unplugged")),
            true,
        )
        .unwrap_err();
        assert_eq!(e, BackupError::NoFolder);
        // Never read as an option, never empty: refused before git runs.
        for url in ["--upload-pack=touch /x", "-x", "", "  "] {
            let e = push(&git, &b, "acme", &Target::Remote(url.into()), true).unwrap_err();
            assert!(matches!(e, BackupError::Damaged(_)), "{url}: {e}");
        }
        // Never a command.
        assert!(
            push(
                &git,
                &b,
                "acme",
                &Target::Remote("ext::sh -c 'touch x'".into()),
                true
            )
            .is_err()
        );
        assert!(!Path::new("x").exists());
        // A source that isn't a backup can't be trusted.
        let e = verify(
            &git,
            dir.path().join("nothing").to_str().unwrap(),
            &dir.path().join("v"),
        )
        .unwrap_err();
        assert!(e.to_string().contains("name the repository itself"), "{e}");
        let e = verify(
            &git,
            "ssh://nowhere.invalid/books.git",
            &dir.path().join("v"),
        )
        .unwrap_err();
        assert!(matches!(e, BackupError::Git(..)), "{e}");
        let text = e.to_string();
        assert!(!text.contains(&b.display().to_string()), "{text}");
    }

    #[test]
    fn no_ignore_rule_leaves_a_file_out() {
        let git = Git::find().expect("git is installed");
        let dir = tempfile::tempdir().unwrap();
        let b = dir.path().join("book");
        let s = book(&b);
        for (d, f) in [
            (crate::root::VAULT_DIR, "tin-1.bin"),
            (crate::root::KEYS_DIR, "legacy.key"),
        ] {
            std::fs::create_dir(b.join(d)).unwrap();
            std::fs::write(b.join(d).join(f), b"x").unwrap();
        }
        commit(&git, &b, "acme", s.head()).unwrap();
        // Ignore rules where git looks for them, as a template or a hand
        // might leave them.
        std::fs::create_dir_all(b.join(ARCHIVE_DIR).join("info")).unwrap();
        std::fs::write(
            b.join(ARCHIVE_DIR).join("info").join("exclude"),
            "*.key\n*.bin\nblobs/\n",
        )
        .unwrap();
        std::fs::write(b.join(crate::root::VAULT_DIR).join("tin-2.bin"), b"y").unwrap();
        commit(&git, &b, "acme", s.head())
            .unwrap()
            .expect("the new record is a change");
        let listed = git
            .run(
                "ls-tree",
                &[
                    os(&format!("--git-dir={}", b.join(ARCHIVE_DIR).display())),
                    os("ls-tree"),
                    os("-r"),
                    os("--name-only"),
                    os("HEAD"),
                ],
            )
            .unwrap();
        for want in [
            "keys/legacy.key",
            "vault/tin-1.bin",
            "vault/tin-2.bin",
            crate::JOURNAL_FILE,
        ] {
            assert!(listed.lines().any(|l| l == want), "{want}: {listed}");
        }
        assert!(listed.lines().any(|l| l.starts_with("blobs/")), "{listed}");
    }

    #[test]
    fn what_git_said_keeps_whole_lines_and_drops_its_progress() {
        let text = b"Counting objects: 100% (3/3), done.\rWriting objects:  50% (1/2)\r\nfatal: '/root/books/acme' is not a repository\n";
        assert_eq!(
            said(text, false),
            "fatal: '/root/books/acme' is not a repository"
        );
        // A line too long to keep whole is left out, never cut.
        let long = format!("fatal: {}\nerror: short\n", "x".repeat(2000));
        assert_eq!(said(long.as_bytes(), false), "error: short");
        // A long line last: nothing before it is reached past it.
        let last = format!("error: first\nfatal: {}\n", "x".repeat(2000));
        assert_eq!(said(last.as_bytes(), false), "");
        assert_eq!(
            said(
                b"remote: Enumerating objects: 5, done.\nfatal: gone\n",
                false
            ),
            "fatal: gone"
        );
    }

    #[test]
    fn a_purged_vault_leaves_the_next_backup() {
        let git = Git::find().expect("git is installed");
        let dir = tempfile::tempdir().unwrap();
        let b = dir.path().join("book");
        let s = book(&b);
        std::fs::create_dir(b.join(crate::root::VAULT_DIR)).unwrap();
        std::fs::write(b.join(crate::root::VAULT_DIR).join("tin-1.bin"), b"sealed").unwrap();
        commit(&git, &b, "acme", s.head()).unwrap().unwrap();
        std::fs::remove_dir_all(b.join(crate::root::VAULT_DIR)).unwrap();
        commit(&git, &b, "acme", s.head())
            .unwrap()
            .expect("the purge is a change");
        let folder = dir.path().join("disk");
        std::fs::create_dir(&folder).unwrap();
        push(&git, &b, "acme", &Target::Folder(folder.clone()), true).unwrap();
        let v = verify(
            &git,
            folder.join("acme.git").to_str().unwrap(),
            &dir.path().join("v"),
        )
        .unwrap();
        assert!(!v.tree.join(crate::root::VAULT_DIR).exists());
    }

    #[test]
    fn a_lock_left_behind_never_stops_the_next_backup() {
        let git = Git::find().expect("git is installed");
        let dir = tempfile::tempdir().unwrap();
        let b = dir.path().join("book");
        let mut s = book(&b);
        commit(&git, &b, "acme", s.head()).unwrap().unwrap();
        // A git stopped part way leaves its locks: while one might still be
        // at work they stay (the backup waits), once older they go.
        let locks = ["index.lock", "HEAD.lock", "refs/heads/main.lock"];
        for lock in locks {
            std::fs::write(b.join(ARCHIVE_DIR).join(lock), b"").unwrap();
        }
        s.add_account("6000", "Supplies", AccountType::Expense, None, false)
            .unwrap();
        assert!(commit(&git, &b, "acme", s.head()).is_err());
        let then = std::time::SystemTime::now() - ARCHIVE_STALE - Duration::from_secs(1);
        for lock in locks {
            std::fs::File::options()
                .write(true)
                .open(b.join(ARCHIVE_DIR).join(lock))
                .unwrap()
                .set_modified(then)
                .unwrap();
        }
        let made = commit(&git, &b, "acme", s.head())
            .unwrap()
            .expect("backed up");
        assert_eq!(made.head, s.head());
        for lock in locks {
            assert!(!b.join(ARCHIVE_DIR).join(lock).exists(), "{lock}");
        }
    }

    #[test]
    fn a_backup_with_a_changed_stored_file_or_a_busy_scratch_is_refused() {
        let git = Git::find().expect("git is installed");
        let dir = tempfile::tempdir().unwrap();
        let b = dir.path().join("book");
        let s = book(&b);
        commit(&git, &b, "acme", s.head()).unwrap();
        let folder = dir.path().join("disk");
        std::fs::create_dir(&folder).unwrap();
        push(&git, &b, "acme", &Target::Folder(folder.clone()), true).unwrap();
        let source = folder.join("acme.git").display().to_string();
        // A scratch folder in use is never written into.
        let busy = dir.path().join("busy");
        std::fs::create_dir(&busy).unwrap();
        let e = verify(&git, &source, &busy).unwrap_err();
        assert!(matches!(e, BackupError::Damaged(_)), "{e}");
        assert_eq!(std::fs::read_dir(&busy).unwrap().count(), 0);
        // One stored file's bytes changed, its name kept.
        let work = dir.path().join("work");
        git.run(
            "clone",
            &[os("clone"), os("-q"), os(&source), work.as_os_str()],
        )
        .unwrap();
        let mut blob = None;
        for f in walk(&work.join("blobs")) {
            blob = Some(f);
        }
        let blob = blob.expect("a stored file");
        std::fs::write(&blob, b"not the receipt").unwrap();
        let w = format!("--git-dir={}", work.join(".git").display());
        let t = format!("--work-tree={}", work.display());
        git.run(
            "commit",
            &[
                os(&w),
                os(&t),
                os("commit"),
                os("-qam"),
                os(&format!("seq {} head {}", s.head().0, s.head().1)),
            ],
        )
        .unwrap();
        let bad = dir.path().join("bad.git");
        git.run(
            "clone",
            &[
                os("clone"),
                os("-q"),
                os("--bare"),
                work.as_os_str(),
                bad.as_os_str(),
            ],
        )
        .unwrap();
        let e = verify(&git, bad.to_str().unwrap(), &dir.path().join("v")).unwrap_err();
        assert!(matches!(e, BackupError::Damaged(_)), "{e}");
    }

    #[test]
    fn a_drill_proves_this_books_history_only() {
        let git = Git::find().expect("git is installed");
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("disk");
        std::fs::create_dir(&folder).unwrap();
        let target = Target::Folder(folder.clone());
        let one = dir.path().join("one");
        let s = book(&one);
        commit(&git, &one, "acme", s.head()).unwrap();
        push(&git, &one, "acme", &target, true).unwrap();
        let journal = one.join(crate::JOURNAL_FILE);
        let scratch = dir.path().join("drill");
        assert_eq!(
            drill(&git, &target, "acme", &journal, &scratch).unwrap(),
            s.head()
        );
        assert!(!scratch.exists(), "the scratch folder is removed");
        // Another book's backup, whole in itself, isn't this book's.
        // (Another currency: its journal differs from its first line.)
        let two = dir.path().join("two");
        let mut s2 = Store::create(&two, Currency::new("EUR").unwrap()).unwrap();
        s2.add_account("6000", "Supplies", AccountType::Expense, None, false)
            .unwrap();
        let e = drill(
            &git,
            &target,
            "acme",
            &two.join(crate::JOURNAL_FILE),
            &scratch,
        )
        .unwrap_err();
        assert!(e.to_string().contains("isn't this book's history"), "{e}");
        assert!(!scratch.exists());
        // A folder that isn't there.
        let gone = Target::Folder(dir.path().join("unplugged"));
        assert_eq!(
            drill(&git, &gone, "acme", &journal, &scratch).unwrap_err(),
            BackupError::NoFolder
        );
        // Where each target's backup is.
        assert_eq!(
            target.source("acme"),
            folder.join("acme.git").display().to_string()
        );
        assert_eq!(
            Target::Remote("ssh://h/b".into()).source("acme"),
            "ssh://h/b"
        );
    }

    /// The backup a format upgrade makes first (plan 3.9): the archive
    /// proves to hold the book exactly as it is, its keys and vault file
    /// for file; a key, a vault record or the journal that differs from
    /// the archive's latest, or a head it doesn't hold, is refused; and
    /// the scratch folder goes either way.
    #[test]
    fn the_archive_proves_to_hold_the_book_as_it_is_keys_and_all() {
        let git = Git::find().expect("git is installed");
        let dir = tempfile::tempdir().unwrap();
        let b = dir.path().join("book");
        let mut s = book(&b);
        let keys = b.join(crate::root::KEYS_DIR);
        std::fs::create_dir(&keys).unwrap();
        std::fs::write(keys.join(crate::root::LEGACY_KEY_FILE), b"ab".repeat(32)).unwrap();
        let vault = b.join(crate::root::VAULT_DIR);
        std::fs::create_dir(&vault).unwrap();
        std::fs::write(vault.join("tin-1.bin"), b"sealed").unwrap();
        let made = commit(&git, &b, "acme", s.head()).unwrap().unwrap();
        let scratch = dir.path().join("check");
        let proven = prove_archive(&git, &b, &s.head(), &scratch).unwrap();
        assert_eq!(proven.head, s.head());
        assert_eq!(proven.commit, made.commit);
        assert_eq!(proven.blobs, 1);
        assert!(!scratch.exists(), "the scratch folder is removed");
        let refused = |b: &Path, head: &(u64, String)| {
            let e = prove_archive(&git, b, head, &scratch).unwrap_err();
            assert!(!scratch.exists());
            e.to_string()
        };
        // A key that isn't the one backed up.
        std::fs::write(keys.join(crate::root::LEGACY_KEY_FILE), b"cd".repeat(32)).unwrap();
        assert!(refused(&b, &s.head()).contains("keys folder"));
        std::fs::write(keys.join(crate::root::LEGACY_KEY_FILE), b"ab".repeat(32)).unwrap();
        // A key the backup lacks.
        std::fs::write(keys.join("other.key"), b"x").unwrap();
        assert!(refused(&b, &s.head()).contains("keys folder"));
        std::fs::remove_file(keys.join("other.key")).unwrap();
        // A vault record that isn't the one backed up.
        std::fs::write(vault.join("tin-1.bin"), b"sealed again").unwrap();
        assert!(refused(&b, &s.head()).contains("TIN vault"));
        std::fs::write(vault.join("tin-1.bin"), b"sealed").unwrap();
        assert!(prove_archive(&git, &b, &s.head(), &scratch).is_ok());
        // The books moved on since the backup.
        s.add_account("6000", "Fees", AccountType::Expense, None, false)
            .unwrap();
        let e = refused(&b, &s.head());
        assert!(e.contains("they are at event"), "{e}");
        // The head it holds, but the journal on disk changed under it.
        let old_head = made.head.clone();
        let e = refused(&b, &old_head);
        assert!(e.contains("byte for byte"), "{e}");
    }

    #[test]
    fn a_backup_holding_a_link_or_a_stranger_is_refused_before_checkout() {
        let git = Git::find().expect("git is installed");
        let dir = tempfile::tempdir().unwrap();
        let b = dir.path().join("book");
        let s = book(&b);
        commit(&git, &b, "acme", s.head()).unwrap();
        let folder = dir.path().join("disk");
        std::fs::create_dir(&folder).unwrap();
        push(&git, &b, "acme", &Target::Folder(folder.clone()), true).unwrap();
        let subject = format!("seq {} head {}", s.head().0, s.head().1);
        let planted = dir.path().join("planted.db");
        type Spoil = fn(&Path, &Path);
        let spoils: [(&str, Spoil); 4] = [
            ("db-link", |w, planted| {
                std::os::unix::fs::symlink(planted, w.join("wanderbooks.db")).unwrap();
            }),
            ("vault-link", |w, _| {
                std::os::unix::fs::symlink("/etc", w.join(crate::root::VAULT_DIR)).unwrap();
            }),
            ("journal-link", |w, _| {
                let j = w.join(crate::JOURNAL_FILE);
                let real = w.join("elsewhere.jsonl");
                std::fs::rename(&j, &real).unwrap();
                std::os::unix::fs::symlink(&real, &j).unwrap();
            }),
            ("stranger", |w, _| {
                std::fs::write(w.join("README"), b"hello").unwrap();
            }),
        ];
        for (tag, spoil) in spoils {
            let work = dir.path().join(format!("work-{tag}"));
            let source = folder.join("acme.git");
            let out = Command::new("git")
                .args(["clone", "-q"])
                .arg(&source)
                .arg(&work)
                .output()
                .unwrap();
            assert!(out.status.success(), "{out:?}");
            spoil(&work, &planted);
            let out = Command::new("git")
                .arg("-C")
                .arg(&work)
                .args(["-c", "user.name=t", "-c", "user.email=t@t.invalid"])
                .args(["add", "-A", "."])
                .output()
                .unwrap();
            assert!(out.status.success(), "{out:?}");
            let out = Command::new("git")
                .arg("-C")
                .arg(&work)
                .args([
                    "-c",
                    "user.name=t",
                    "-c",
                    "user.email=t@t.invalid",
                    "-c",
                    "commit.gpgsign=false",
                ])
                .args(["commit", "-qm", &subject])
                .output()
                .unwrap();
            assert!(out.status.success(), "{tag}: {out:?}");
            let bad = dir.path().join(format!("{tag}.git"));
            let out = Command::new("git")
                .args(["clone", "-q", "--bare"])
                .arg(&work)
                .arg(&bad)
                .output()
                .unwrap();
            assert!(out.status.success(), "{out:?}");
            let scratch = dir.path().join(format!("v-{tag}"));
            let e = verify(&git, bad.to_str().unwrap(), &scratch).unwrap_err();
            assert!(matches!(e, BackupError::Damaged(_)), "{tag}: {e}");
            assert!(!planted.exists(), "{tag}: wrote outside its folder");
            assert!(
                !scratch.join(crate::JOURNAL_FILE).exists(),
                "{tag}: checked out"
            );
        }
    }

    #[test]
    fn local_addresses_are_told_from_networked_ones() {
        for (address, local) in [
            ("/srv/books.git", true),
            ("file:///srv/books.git", true),
            ("acme.git", true),
            ("backups/acme.git", true),
            ("./x:y", true),
            ("ssh://host/books.git", false),
            ("https://host/owner/books", false),
            ("git@host:owner/books.git", false),
            ("host:books.git", false),
        ] {
            assert_eq!(is_local(address), local, "{address}");
        }
    }

    #[test]
    fn a_file_git_wont_take_fails_the_backup() {
        let git = Git::find().expect("git is installed");
        let dir = tempfile::tempdir().unwrap();
        let b = dir.path().join("book");
        let s = book(&b);
        std::fs::create_dir(b.join(crate::root::VAULT_DIR)).unwrap();
        // git leaves out a file named .git without a word.
        std::fs::write(b.join(crate::root::VAULT_DIR).join(".git"), b"x").unwrap();
        let e = commit(&git, &b, "acme", s.head()).unwrap_err();
        assert!(matches!(e, BackupError::Damaged(_)), "{e}");
    }

    #[test]
    fn no_attributes_rule_changes_a_byte() {
        let git = Git::find().expect("git is installed");
        let dir = tempfile::tempdir().unwrap();
        let b = dir.path().join("book");
        let s = book(&b);
        std::fs::create_dir(b.join(crate::root::VAULT_DIR)).unwrap();
        let bytes = b"$Id: kept$\r\nline two\r\n";
        std::fs::write(b.join(crate::root::VAULT_DIR).join("tin-1.bin"), bytes).unwrap();
        commit(&git, &b, "acme", s.head()).unwrap();
        // The archive's own rules are taken out before each backup.
        let info = b.join(ARCHIVE_DIR).join("info");
        std::fs::create_dir_all(&info).unwrap();
        std::fs::write(info.join("attributes"), "* ident text eol=lf\n").unwrap();
        std::fs::write(b.join(crate::root::VAULT_DIR).join("tin-2.bin"), bytes).unwrap();
        commit(&git, &b, "acme", s.head()).unwrap().unwrap();
        assert!(!info.join("attributes").exists());
        let gd = format!("--git-dir={}", b.join(ARCHIVE_DIR).display());
        for f in ["vault/tin-1.bin", "vault/tin-2.bin"] {
            let out = Command::new(&git.exe)
                .args([&gd, "cat-file", "-p", &format!("HEAD:{f}")])
                .output()
                .unwrap();
            assert_eq!(out.stdout, bytes, "{f}");
        }
        // A rules file in the book's folder, or in what is backed up:
        // refused.
        for at in [
            b.join(".gitattributes"),
            b.join(crate::root::VAULT_DIR).join(".gitattributes"),
        ] {
            std::fs::write(&at, "* ident\n").unwrap();
            let e = commit(&git, &b, "acme", s.head()).unwrap_err();
            assert!(e.to_string().contains(".gitattributes"), "{e}");
            std::fs::remove_file(&at).unwrap();
        }
    }

    #[test]
    fn the_archives_own_hooks_never_run() {
        use std::os::unix::fs::PermissionsExt;
        let git = Git::find().expect("git is installed");
        let dir = tempfile::tempdir().unwrap();
        let b = dir.path().join("book");
        let mut s = book(&b);
        commit(&git, &b, "acme", s.head()).unwrap();
        let ran = dir.path().join("ran");
        let hooks = b.join(ARCHIVE_DIR).join("hooks");
        std::fs::create_dir_all(&hooks).unwrap();
        for hook in [
            "pre-commit",
            "post-commit",
            "reference-transaction",
            "pre-push",
            "post-index-change",
        ] {
            let h = hooks.join(hook);
            std::fs::write(
                &h,
                format!("#!/bin/sh\necho {hook} >> '{}'\n", ran.display()),
            )
            .unwrap();
            std::fs::set_permissions(&h, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        s.add_account("6000", "Supplies", AccountType::Expense, None, false)
            .unwrap();
        commit(&git, &b, "acme", s.head()).unwrap().unwrap();
        let folder = dir.path().join("disk");
        std::fs::create_dir(&folder).unwrap();
        push(&git, &b, "acme", &Target::Folder(folder), true).unwrap();
        assert!(
            !ran.exists(),
            "a hook ran: {:?}",
            std::fs::read_to_string(&ran)
        );
    }

    /// A git that runs the real one with the owner's own configuration set
    /// to everything that could spoil a backup.
    fn hostile_git(dir: &Path, real: &Git) -> Git {
        use std::os::unix::fs::PermissionsExt;
        let home = dir.join("hostile-home");
        let hooks = home.join("hooks");
        std::fs::create_dir_all(&hooks).unwrap();
        let marker = dir.join("marker");
        for hook in [
            "post-commit",
            "pre-push",
            "post-receive",
            "reference-transaction",
        ] {
            let h = hooks.join(hook);
            std::fs::write(
                &h,
                format!("#!/bin/sh\necho {hook} >> '{}'\n", marker.display()),
            )
            .unwrap();
            std::fs::set_permissions(&h, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        std::fs::write(home.join("ignore"), "*.bin\n*.key\nblobs/\n").unwrap();
        std::fs::write(home.join("attributes"), "* text=auto eol=lf ident\n").unwrap();
        std::fs::write(
            home.join(".gitconfig"),
            format!(
                "[core]\n\tautocrlf = true\n\texcludesFile = {h}/ignore\n\tattributesFile = {h}/attributes\n\thooksPath = {h}/hooks\n\tfsmonitor = touch '{m}.fsmonitor'\n\tsymlinks = true\n[commit]\n\tgpgsign = true\n[push]\n\tgpgSign = true\n[gpg]\n\tprogram = false\n[init]\n\tdefaultRefFormat = reftable\n\ttemplateDir = {h}/template\n[protocol \"ext\"]\n\tallow = always\n[log]\n\tshowSignature = true\n",
                h = home.display(),
                m = marker.display()
            ),
        )
        .unwrap();
        let wrapper = dir.join("git");
        std::fs::write(
            &wrapper,
            format!(
                "#!/bin/sh\nexport HOME='{}'\nexport XDG_CONFIG_HOME='{}'\nexec '{}' \"$@\"\n",
                home.display(),
                home.display(),
                real.exe.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
        Git {
            exe: wrapper,
            quiet: GIT_TIMEOUT,
            cancel: None,
            term_grace: TERM_GRACE,
            watch: Watch::default(),
        }
    }

    #[test]
    fn the_owners_git_configuration_never_spoils_a_backup() {
        let real = Git::find().expect("git is installed");
        let dir = tempfile::tempdir().unwrap();
        let git = hostile_git(dir.path(), &real);
        let b = dir.path().join("book");
        let s = book(&b);
        std::fs::create_dir(b.join(crate::root::VAULT_DIR)).unwrap();
        let crlf = b"$Id: x$\r\nline\r\n";
        std::fs::write(b.join(crate::root::VAULT_DIR).join("tin-1.bin"), crlf).unwrap();
        std::fs::create_dir(b.join(crate::root::KEYS_DIR)).unwrap();
        std::fs::write(b.join(crate::root::KEYS_DIR).join("legacy.key"), b"k").unwrap();
        commit(&git, &b, "acme", s.head()).unwrap().unwrap();
        let folder = dir.path().join("disk");
        std::fs::create_dir(&folder).unwrap();
        push(&git, &b, "acme", &Target::Folder(folder.clone()), true).unwrap();
        let v = verify(
            &git,
            folder.join("acme.git").to_str().unwrap(),
            &dir.path().join("v"),
        )
        .unwrap();
        assert_eq!(v.head, s.head());
        assert_eq!(std::fs::read(v.tree.join("vault/tin-1.bin")).unwrap(), crlf);
        assert!(v.tree.join("keys/legacy.key").is_file());
        assert!(!dir.path().join("marker").exists(), "a hook ran");
        assert!(
            !dir.path().join("marker.fsmonitor").exists(),
            "a watcher ran"
        );
        assert!(
            push(
                &git,
                &b,
                "acme",
                &Target::Remote("ext::sh -c 'touch x'".into()),
                true
            )
            .is_err()
        );
    }

    /// A git that is the real one with the hooks folder of its own: each
    /// `core.hooksPath=/dev/null` it is given (in its own arguments, and in
    /// the `--receive-pack` command a push to this computer runs) names
    /// `hooks` instead, where a `reference-transaction` hook waits, while
    /// `hooks/slow` exists, at the point where git holds its ref locks (and
    /// a commit its index lock): how a real git is made slow enough to be
    /// killed inside the work. `hooks/hooked` has a line per hook that waited.
    fn slow_git(dir: &Path, real: &Git) -> (Git, PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let hooks = dir.join("slow-hooks");
        std::fs::create_dir_all(&hooks).unwrap();
        let hook = hooks.join("reference-transaction");
        std::fs::write(
            &hook,
            "#!/bin/sh\ncat >/dev/null\n[ \"$1\" = prepared ] || exit 0\nd=\"$(dirname \"$0\")\"\nif [ -e \"$d/slow\" ]; then\n  echo $$ >> \"$d/hooked\"\n  while [ -e \"$d/slow\" ]; do sleep 0.05; done\nfi\nexit 0\n",
        )
        .unwrap();
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        // The receiving end's `pre-receive`: waits while `slow-pack` exists,
        // after the pack has been received into the quarantine.
        let pre = hooks.join("pre-receive");
        std::fs::write(
            &pre,
            "#!/bin/sh\ncat >/dev/null\nd=\"$(dirname \"$0\")\"\nif [ -e \"$d/slow-pack\" ]; then\n  echo $$ >> \"$d/hooked-pack\"\n  while [ -e \"$d/slow-pack\" ]; do sleep 0.05; done\nfi\nexit 0\n",
        )
        .unwrap();
        std::fs::set_permissions(&pre, std::fs::Permissions::from_mode(0o755)).unwrap();
        let wrapper = dir.join("slow-git");
        std::fs::write(
            &wrapper,
            format!(
                "#!/bin/sh\nn=$#\nwhile [ $n -gt 0 ]; do\n  a=\"$1\"; shift\n  case \"$a\" in\n    *core.hooksPath=/dev/null*) a=$(printf '%s' \"$a\" | sed 's|core.hooksPath=/dev/null|core.hooksPath={}|g') ;;\n  esac\n  set -- \"$@\" \"$a\"\n  n=$((n-1))\ndone\nexec '{}' \"$@\"\n",
                hooks.display(),
                real.exe.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
        (
            Git {
                exe: wrapper,
                quiet: GIT_TIMEOUT,
                cancel: None,
                term_grace: TERM_GRACE,
                watch: Watch::default(),
            },
            hooks,
        )
    }

    /// Wait until a hook of `slow_git` is waiting.
    fn wait_hooked(hooks: &Path) {
        let by = Instant::now() + Duration::from_secs(30);
        while !hooks.join("hooked").exists() {
            assert!(Instant::now() < by, "git never reached its ref update");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn fsck(git: &Git, repo: &Path) {
        git.run(
            "fsck",
            &[
                os(&format!("--git-dir={}", repo.display())),
                os("fsck"),
                os("--strict"),
            ],
        )
        .unwrap_or_else(|e| panic!("the repository isn't whole: {e}"));
    }

    /// The locks in `repo`, as paths in it.
    fn locks_in(repo: &Path) -> Vec<String> {
        crate::git_locks::RepoDir::open(repo)
            .unwrap()
            .locks()
            .into_iter()
            .map(|f| f.lock)
            .collect()
    }

    /// Hold every process of the group `pid` leads still (SIGSTOP): a
    /// SIGTERM sent to it then stays pending, git can't run its handler,
    /// and only the kill after the grace ends it. This is how a git that
    /// can't end on SIGTERM, with the locks it holds, is made on purpose.
    fn freeze_group_of(pid: u32) {
        let out = std::process::Command::new("ps")
            .args(["-o", "pgid=", "-p", &pid.to_string()])
            .output()
            .unwrap();
        let group: i32 = String::from_utf8_lossy(&out.stdout)
            .trim()
            .parse()
            .expect("ps names the group");
        // SAFETY: stopping the group this test's own git started.
        assert_eq!(unsafe { libc::kill(-group, libc::SIGSTOP) }, 0);
    }

    /// The pid of the first hook of `slow_git` that waited, in `file`.
    fn hooked_pid(hooks: &Path, file: &str) -> u32 {
        std::fs::read_to_string(hooks.join(file))
            .unwrap()
            .lines()
            .next()
            .unwrap()
            .trim()
            .parse()
            .unwrap()
    }

    /// INVARIANT 5, the archive, told to stop: a real git in the middle of a
    /// commit, holding the ref locks, ends on SIGTERM and removes them
    /// itself. Nothing is left, and nothing was cleaned up here (no note):
    /// the grace is long, so a git that needed the kill would show.
    #[test]
    fn a_git_stopped_by_sigterm_mid_commit_leaves_no_lock_and_needs_no_cleanup() {
        let real = Git::find().expect("git is installed");
        let dir = tempfile::tempdir().unwrap();
        let (slow, hooks) = slow_git(dir.path(), &real);
        let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let cancelled = slow
            .clone()
            .with_cancel(flag.clone())
            .with_term_grace(Duration::from_secs(120));
        let b = dir.path().join("book");
        let mut s = book(&b);
        commit(&slow, &b, "acme", s.head()).unwrap().unwrap();
        s.add_account("6000", "Supplies", AccountType::Expense, None, false)
            .unwrap();
        let archive = b.join(ARCHIVE_DIR);
        std::fs::write(hooks.join("slow"), b"").unwrap();
        let running = {
            let (git, b, head) = (cancelled.clone(), b.clone(), s.head());
            std::thread::spawn(move || commit(&git, &b, "acme", head))
        };
        wait_hooked(&hooks);
        // Inside the commit's ref update it holds the ref locks.
        let held = locks_in(&archive);
        assert!(held.contains(&"HEAD.lock".to_string()), "{held:?}");
        assert!(
            held.contains(&"refs/heads/main.lock".to_string()),
            "{held:?}"
        );
        flag.store(true, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(
            running.join().unwrap().unwrap_err(),
            BackupError::Cancelled("commit".into())
        );
        assert_eq!(locks_in(&archive), Vec::<String>::new());
        assert_eq!(
            cancelled.take_lock_notes(),
            Vec::new(),
            "git removed its own locks: there was nothing for this program to clean up"
        );
        // The next backup, with no one to remove anything by hand.
        std::fs::remove_file(hooks.join("slow")).unwrap();
        let made = commit(&slow, &b, "acme", s.head())
            .unwrap()
            .expect("the next backup is made");
        assert_eq!(made.head, s.head());
        fsck(&real, &archive);
    }

    /// INVARIANT 5, the archive, a git that can't end on SIGTERM: killed
    /// only after the grace; once its group is gone the locks it left, none
    /// of which was there when the command began, are removed through the
    /// archive's own handle, each noted; the next backup needs no hand.
    #[test]
    fn a_git_that_cannot_end_on_sigterm_is_killed_after_the_grace_and_its_archive_locks_go() {
        let real = Git::find().expect("git is installed");
        let dir = tempfile::tempdir().unwrap();
        let (slow, hooks) = slow_git(dir.path(), &real);
        let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let grace = Duration::from_millis(400);
        let cancelled = slow
            .clone()
            .with_cancel(flag.clone())
            .with_term_grace(grace);
        let b = dir.path().join("book");
        let mut s = book(&b);
        commit(&slow, &b, "acme", s.head()).unwrap().unwrap();
        s.add_account("6000", "Supplies", AccountType::Expense, None, false)
            .unwrap();
        let archive = b.join(ARCHIVE_DIR);
        std::fs::write(hooks.join("slow"), b"").unwrap();
        let running = {
            let (git, b, head) = (cancelled.clone(), b.clone(), s.head());
            std::thread::spawn(move || commit(&git, &b, "acme", head))
        };
        wait_hooked(&hooks);
        let held = locks_in(&archive);
        assert!(!held.is_empty(), "git held no lock inside its ref update");
        freeze_group_of(hooked_pid(&hooks, "hooked"));
        let stopped = Instant::now();
        flag.store(true, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(
            running.join().unwrap().unwrap_err(),
            BackupError::Cancelled("commit".into())
        );
        assert!(
            stopped.elapsed() >= grace,
            "killed before the grace had run out"
        );
        assert_eq!(locks_in(&archive), Vec::<String>::new());
        let notes = cancelled.take_lock_notes();
        for lock in &held {
            assert!(
                notes.iter().any(|n| n.repo == "archive"
                    && &n.lock == lock
                    && n.removed
                    && n.why == "git was stopped"
                    && n.path.is_none()),
                "{lock} wasn't noted: {notes:?}"
            );
        }
        std::fs::remove_file(hooks.join("slow")).unwrap();
        let made = commit(&slow, &b, "acme", s.head())
            .unwrap()
            .expect("the next backup is made");
        assert_eq!(made.head, s.head());
        fsck(&real, &archive);
    }

    /// INVARIANT 5, the index lock: a real git stopped inside `add` (here
    /// hashing a very large, empty, stored file) holds the index lock, and
    /// removes it itself on SIGTERM; the next backup needs no hand.
    #[test]
    fn a_git_stopped_mid_add_leaves_no_index_lock() {
        let real = Git::find().expect("git is installed");
        let dir = tempfile::tempdir().unwrap();
        let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let git = real
            .clone()
            .with_cancel(flag.clone())
            .with_term_grace(Duration::from_secs(120));
        let b = dir.path().join("book");
        let s = book(&b);
        commit(&real, &b, "acme", s.head()).unwrap().unwrap();
        let big = b.join("blobs").join("big-and-empty");
        std::fs::File::create(&big)
            .unwrap()
            .set_len(64 << 30)
            .unwrap();
        let archive = b.join(ARCHIVE_DIR);
        let running = {
            let (git, b) = (git.clone(), b.clone());
            std::thread::spawn(move || stage_blobs(&git, &b))
        };
        // The lock is taken as `add` begins and held until it ends, which
        // for a file this size is minutes: seeing it is being inside.
        let by = Instant::now() + Duration::from_secs(60);
        while !archive.join("index.lock").exists() {
            assert!(Instant::now() < by, "git never took the index lock");
            std::thread::sleep(Duration::from_millis(5));
        }
        flag.store(true, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(
            running.join().unwrap().unwrap_err(),
            BackupError::Cancelled("add".into())
        );
        assert_eq!(locks_in(&archive), Vec::<String>::new());
        assert_eq!(git.take_lock_notes(), Vec::new());
        std::fs::remove_file(&big).unwrap();
        stage_blobs(&real, &b).unwrap();
        fsck(&real, &archive);
    }

    /// A book backed up once to a folder target, then changed and
    /// committed again, ready for a push: (git, hooks, book folder, target
    /// repository, the target, the store).
    fn ready_to_push(
        dir: &Path,
        real: &Git,
    ) -> (Git, PathBuf, PathBuf, PathBuf, Target, crate::Store) {
        let (slow, hooks) = slow_git(dir, real);
        let b = dir.join("book");
        let mut s = book(&b);
        commit(&slow, &b, "acme", s.head()).unwrap().unwrap();
        let folder = dir.join("disk");
        std::fs::create_dir(&folder).unwrap();
        let target = Target::Folder(folder.clone());
        push(&slow, &b, "acme", &target, true).unwrap();
        s.add_account("6000", "Supplies", AccountType::Expense, None, false)
            .unwrap();
        commit(&slow, &b, "acme", s.head()).unwrap().unwrap();
        (slow, hooks, b, folder.join("acme.git"), target, s)
    }

    fn main_of(real: &Git, repo: &Path) -> String {
        real.run(
            "rev-parse",
            &[
                os(&format!("--git-dir={}", repo.display())),
                os("rev-parse"),
                os("refs/heads/main"),
            ],
        )
        .unwrap()
    }

    /// INVARIANT 5b, told to stop: a real git in the middle of a push (its
    /// receive-pack holding the target's ref lock) ends on SIGTERM and its
    /// receiving end removes the lock itself. Nothing was removed here, a
    /// lock that was there before the push is untouched, the next push
    /// succeeds, and both repositories pass git's check.
    #[test]
    fn a_push_stopped_by_sigterm_leaves_no_lock_in_the_target_and_removes_nothing_there() {
        let real = Git::find().expect("git is installed");
        let dir = tempfile::tempdir().unwrap();
        let (slow, hooks, b, repo, target, _s) = ready_to_push(dir.path(), &real);
        // Another writer's lock, held before our push begins.
        std::fs::write(repo.join("refs/heads/another.lock"), b"").unwrap();
        let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let cancelled = slow
            .clone()
            .with_cancel(flag.clone())
            .with_term_grace(Duration::from_secs(120));
        std::fs::write(hooks.join("slow"), b"").unwrap();
        let running = {
            let (git, b, target) = (cancelled.clone(), b.clone(), target.clone());
            std::thread::spawn(move || push(&git, &b, "acme", &target, true))
        };
        wait_hooked(&hooks);
        assert!(repo.join("refs/heads/main.lock").exists());
        flag.store(true, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(
            running.join().unwrap().unwrap_err(),
            BackupError::Cancelled("push".into())
        );
        assert!(!repo.join("refs/heads/main.lock").exists());
        assert!(
            repo.join("refs/heads/another.lock").exists(),
            "a lock that was there before the push was removed"
        );
        assert_eq!(cancelled.take_lock_notes(), Vec::new());
        // The next push, nobody's hand in it.
        std::fs::remove_file(hooks.join("slow")).unwrap();
        std::fs::remove_file(hooks.join("hooked")).unwrap();
        let commit_id = push(&slow, &b, "acme", &target, true).unwrap();
        assert_eq!(main_of(&real, &repo), commit_id);
        fsck(&real, &repo);
        fsck(&real, &b.join(ARCHIVE_DIR));
    }

    /// INVARIANT 5b, a git that can't end on SIGTERM: killed after the
    /// grace, it leaves the target's ref lock, and nothing in the owner's
    /// folder is deleted by us, whatever the lock's age. The push is
    /// reported as failed, naming the file and what the owner may do. A
    /// lock that was there before the push began is not named, and is
    /// untouched. Once the owner (here, the test) removes the file the next
    /// push succeeds, and both repositories pass git's check.
    #[test]
    fn a_lock_left_in_a_folder_target_by_a_kill_is_never_removed_but_reported() {
        let real = Git::find().expect("git is installed");
        let dir = tempfile::tempdir().unwrap();
        let (slow, hooks, b, repo, target, _s) = ready_to_push(dir.path(), &real);
        std::fs::write(repo.join("refs/heads/another.lock"), b"").unwrap();
        let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let grace = Duration::from_millis(400);
        let cancelled = slow
            .clone()
            .with_cancel(flag.clone())
            .with_term_grace(grace);
        std::fs::write(hooks.join("slow"), b"").unwrap();
        let running = {
            let (git, b, target) = (cancelled.clone(), b.clone(), target.clone());
            std::thread::spawn(move || push(&git, &b, "acme", &target, true))
        };
        wait_hooked(&hooks);
        let lock = repo.join("refs/heads/main.lock");
        assert!(lock.exists());
        freeze_group_of(hooked_pid(&hooks, "hooked"));
        let stopped = Instant::now();
        flag.store(true, std::sync::atomic::Ordering::SeqCst);
        let said = running.join().unwrap().unwrap_err();
        assert!(stopped.elapsed() >= grace);
        let BackupError::LockLeft(named) = &said else {
            panic!("{said:?}");
        };
        assert!(
            named.split(", ").any(|p| p == lock.display().to_string()),
            "{said}"
        );
        // Every lock it left is named, by its whole path, and each is there.
        for path in named.split(", ") {
            assert!(Path::new(path).exists(), "{path}");
        }
        let words = said.to_string();
        assert!(
            words.contains("nothing else is using the backup folder"),
            "{words}"
        );
        assert!(!words.contains("another.lock"), "{words}");
        // Left where it was, and the one before our push untouched.
        assert!(lock.exists(), "a file in the owner's folder was deleted");
        assert!(repo.join("refs/heads/another.lock").exists());
        let notes = cancelled.take_lock_notes();
        assert!(
            notes.iter().all(|n| n.repo == "folder"
                && !n.removed
                && n.path.as_deref() == Some(repo.join(&n.lock).display().to_string().as_str())),
            "{notes:?}"
        );
        assert!(
            notes.iter().any(|n| n.lock == "refs/heads/main.lock"),
            "{notes:?}"
        );
        assert!(
            !notes.iter().any(|n| n.lock.contains("another")),
            "{notes:?}"
        );
        // Still there, a later push fails by git's own words, and the file
        // stays.
        std::fs::remove_file(hooks.join("slow")).unwrap();
        assert!(push(&slow, &b, "acme", &target, true).is_err());
        assert!(lock.exists(), "a later push removed it");
        // The owner removes it; the next backup goes on.
        std::fs::remove_file(&lock).unwrap();
        let commit_id = push(&slow, &b, "acme", &target, true).unwrap();
        assert_eq!(main_of(&real, &repo), commit_id);
        fsck(&real, &repo);
        fsck(&real, &b.join(ARCHIVE_DIR));
    }

    /// A book whose next push carries a large object (about 48 MiB that
    /// doesn't compress), with a `pre-receive` hook on the receiving side
    /// that waits while `slow-pack` exists: the pack has been received into
    /// the target's quarantine by then, and the push is held inside the
    /// transfer's receiving end.
    fn large_push_ready(dir: &Path, real: &Git) -> (Git, PathBuf, PathBuf, PathBuf, Target) {
        let (slow, hooks, b, repo, target, _s) = ready_to_push(dir, real);
        let mut bytes = vec![0u8; 48 << 20];
        let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
        for chunk in bytes.chunks_mut(8) {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            chunk.copy_from_slice(&x.to_le_bytes()[..chunk.len()]);
        }
        std::fs::write(b.join("blobs").join("large-and-random"), &bytes).unwrap();
        commit(&slow, &b, "acme", (1, "x".into()))
            .map(|_| ())
            .unwrap();
        (slow, hooks, b, repo, target)
    }

    /// INVARIANT 5, 6: a push stopped while its pack is being received (a
    /// large object, the receiving end held up by a hook) leaves both
    /// repositories whole, however it was ended: git's own check is run on
    /// both at once, with nothing cleaned up between.
    #[test]
    fn a_push_stopped_during_the_pack_transfer_leaves_both_repositories_whole() {
        let real = Git::find().expect("git is installed");
        for (name, frozen) in [("by SIGTERM", false), ("by the kill after the grace", true)] {
            let dir = tempfile::tempdir().unwrap();
            let (slow, hooks, b, repo, target) = large_push_ready(dir.path(), &real);
            let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let cancelled = slow
                .clone()
                .with_cancel(flag.clone())
                .with_term_grace(Duration::from_millis(if frozen { 400 } else { 120_000 }));
            std::fs::write(hooks.join("slow-pack"), b"").unwrap();
            let running = {
                let (git, b, target) = (cancelled.clone(), b.clone(), target.clone());
                std::thread::spawn(move || push(&git, &b, "acme", &target, true))
            };
            let by = Instant::now() + Duration::from_secs(300);
            while !hooks.join("hooked-pack").exists() {
                assert!(
                    Instant::now() < by,
                    "{name}: the push never reached the receiving end's hook"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            // The pack is in the target's quarantine: a stop is in the
            // transfer's receiving end.
            let incoming = std::fs::read_dir(repo.join("objects"))
                .unwrap()
                .flatten()
                .filter(|e| e.file_name().to_string_lossy().contains("incoming-"))
                .count();
            assert!(incoming > 0, "{name}: nothing was being received");
            if frozen {
                freeze_group_of(hooked_pid(&hooks, "hooked-pack"));
            }
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
            let said = running.join().unwrap().unwrap_err();
            assert!(
                matches!(said, BackupError::Cancelled(_) | BackupError::LockLeft(_)),
                "{name}: {said:?}"
            );
            // At once: nothing cleaned up, nothing waited for.
            fsck(&real, &repo);
            fsck(&real, &b.join(ARCHIVE_DIR));
            // And the target still takes the next push, after the owner's
            // own removal of a lock a kill left (none, when SIGTERM ended
            // git).
            std::fs::remove_file(hooks.join("slow-pack")).unwrap();
            std::fs::remove_file(hooks.join("hooked-pack")).unwrap();
            if let BackupError::LockLeft(path) = said {
                assert!(frozen, "{name}: SIGTERM left a lock");
                std::fs::remove_file(path).unwrap();
            }
            let commit_id = push(&slow, &b, "acme", &target, true).unwrap();
            assert_eq!(main_of(&real, &repo), commit_id, "{name}");
            fsck(&real, &repo);
        }
    }

    /// INVARIANT 5a: at open, a lock in the archive is removed only if it
    /// is older than ARCHIVE_STALE, each removal noted; a younger one is
    /// left (and said so), nothing that isn't a lock's name is touched, and
    /// the next backup removes a lock once it has grown old.
    #[test]
    fn the_archives_age_rule_at_open() {
        let git = Git::find().expect("git is installed");
        let dir = tempfile::tempdir().unwrap();
        let b = dir.path().join("book");
        let mut s = book(&b);
        commit(&git, &b, "acme", s.head()).unwrap().unwrap();
        let archive = b.join(ARCHIVE_DIR);
        let old_enough = std::time::SystemTime::now() - ARCHIVE_STALE - Duration::from_secs(5);
        let age = |path: &Path, to: std::time::SystemTime| {
            std::fs::File::options()
                .write(true)
                .open(path)
                .unwrap()
                .set_modified(to)
                .unwrap();
        };
        let old = ["index.lock", "refs/heads/main.lock"];
        let young = ["HEAD.lock", "packed-refs.lock"];
        for lock in old.iter().chain(&young) {
            std::fs::write(archive.join(lock), b"").unwrap();
        }
        for lock in old {
            age(&archive.join(lock), old_enough);
        }
        // Not locks, however old: a file of git's own, a folder named like
        // one, and a link.
        std::fs::write(archive.join("description.lock-not"), b"").unwrap();
        age(&archive.join("description.lock-not"), old_enough);
        std::fs::create_dir(archive.join("shallow.lock")).unwrap();
        let notes = clear_archive_locks(&b);
        let removed: Vec<&str> = notes
            .iter()
            .filter(|n| n.removed)
            .map(|n| n.lock.as_str())
            .collect();
        let mut removed_sorted = removed.clone();
        removed_sorted.sort();
        assert_eq!(removed_sorted, ["index.lock", "refs/heads/main.lock"]);
        let mut left: Vec<&str> = notes
            .iter()
            .filter(|n| !n.removed)
            .map(|n| n.lock.as_str())
            .collect();
        left.sort();
        assert_eq!(left, ["HEAD.lock", "packed-refs.lock"]);
        assert!(notes.iter().all(|n| n.repo == "archive"));
        assert_eq!(locks_in(&archive), ["HEAD.lock", "packed-refs.lock"]);
        assert!(archive.join("description.lock-not").exists());
        assert!(archive.join("shallow.lock").is_dir());
        // The next backup meets a young lock: it fails by git's words, and
        // the lock stays. Once old it is removed, noted, and the backup goes.
        s.add_account("6000", "Supplies", AccountType::Expense, None, false)
            .unwrap();
        assert!(commit(&git, &b, "acme", s.head()).is_err());
        assert!(archive.join("HEAD.lock").exists());
        for lock in young {
            age(&archive.join(lock), old_enough);
        }
        let git = git.with_lock_notes(LockNotes::default());
        let made = commit(&git, &b, "acme", s.head())
            .unwrap()
            .expect("backed up");
        assert_eq!(made.head, s.head());
        let notes = git.take_lock_notes();
        let mut removed: Vec<&str> = notes.iter().map(|n| n.lock.as_str()).collect();
        removed.sort();
        assert_eq!(removed, ["HEAD.lock", "packed-refs.lock"]);
        assert!(notes.iter().all(|n| n.removed && n.repo == "archive"));
        assert_eq!(locks_in(&archive), Vec::<String>::new());
    }

    /// INVARIANT 5a and git_locks INVARIANT 1: what is removed after a stop
    /// is removed through the archive's folder as it was opened when the
    /// command began. A `refs` folder swapped for a link, or the archive's
    /// own path swapped for one, in the meantime, touches nothing outside.
    #[test]
    fn removal_after_a_stop_goes_through_the_handle_opened_before_the_command() {
        let git = Git::find().expect("git is installed");
        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(outside.join("refs/heads")).unwrap();
        for lock in ["index.lock", "HEAD.lock", "refs/heads/main.lock"] {
            std::fs::write(outside.join(lock), b"theirs").unwrap();
        }
        let untouched = |what: &str| {
            for lock in ["index.lock", "HEAD.lock", "refs/heads/main.lock"] {
                assert_eq!(
                    std::fs::read(outside.join(lock)).unwrap(),
                    b"theirs",
                    "{what}: {lock} outside was touched"
                );
            }
        };
        // 1. `refs` swapped for a link.
        let b = dir.path().join("book");
        let s = book(&b);
        commit(&git, &b, "acme", s.head()).unwrap().unwrap();
        let archive = b.join(ARCHIVE_DIR);
        let watched = Watched::archive(&archive);
        let snap = watched.snapshot();
        std::fs::write(archive.join("index.lock"), b"ours").unwrap();
        std::fs::write(archive.join("refs/heads/main.lock"), b"ours").unwrap();
        std::fs::rename(archive.join("refs"), dir.path().join("refs-moved")).unwrap();
        std::os::unix::fs::symlink(outside.join("refs"), archive.join("refs")).unwrap();
        let left = watched.after_kill(&git, &snap, true);
        assert!(left.is_empty());
        assert!(!archive.join("index.lock").exists(), "its own lock stays");
        assert!(
            dir.path().join("refs-moved/heads/main.lock").exists(),
            "the folder moved aside was reached through the link"
        );
        untouched("refs swapped");
        let notes = git.take_lock_notes();
        assert_eq!(
            notes.iter().map(|n| n.lock.as_str()).collect::<Vec<_>>(),
            ["index.lock"]
        );
        // 2. The archive's own path swapped for a link.
        std::fs::remove_file(archive.join("refs")).unwrap();
        std::fs::rename(dir.path().join("refs-moved"), archive.join("refs")).unwrap();
        std::fs::remove_file(archive.join("refs/heads/main.lock")).unwrap();
        let snap = watched.snapshot();
        std::fs::write(archive.join("index.lock"), b"ours").unwrap();
        let moved = b.join("archive-moved.git");
        std::fs::rename(&archive, &moved).unwrap();
        std::os::unix::fs::symlink(&outside, &archive).unwrap();
        let left = watched.after_kill(&git, &snap, true);
        assert!(left.is_empty());
        assert!(!moved.join("index.lock").exists());
        untouched("archive path swapped");
        // 3. A command that began when the archive path was already a link
        // has no handle, and nothing is removed anywhere.
        std::fs::write(outside.join("new.lock"), b"theirs").unwrap();
        let snap = watched.snapshot();
        assert!(snap.handle.is_none());
        std::fs::write(outside.join("index.lock"), b"theirs").unwrap();
        assert!(watched.after_kill(&git, &snap, true).is_empty());
        untouched("path a link from the start");
    }

    /// INVARIANT 6: a drill stops inside whichever phase it is told to stop
    /// in, never going on to the next, and leaves nothing (its scratch
    /// folder is removed). The stop is set as the phase begins; each case
    /// fails when the stop is no longer looked at inside that phase.
    #[test]
    fn a_drill_stops_inside_the_phase_it_is_told_to_stop_in() {
        let real = Git::find().expect("git is installed");
        let dir = tempfile::tempdir().unwrap();
        let b = dir.path().join("book");
        let s = book(&b);
        commit(&real, &b, "acme", s.head()).unwrap().unwrap();
        let folder = dir.path().join("disk");
        std::fs::create_dir(&folder).unwrap();
        let target = Target::Folder(folder);
        push(&real, &b, "acme", &target, true).unwrap();
        let order = [
            "clone", "tree", "checkout", "replay", "verify", "blobs", "vault", "prefix",
        ];
        for stop_in in ["clone", "replay", "verify", "blobs", "prefix"] {
            let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
            let git = real.clone().with_cancel(flag.clone()).with_phase_watch({
                let (flag, seen) = (flag.clone(), seen.clone());
                std::sync::Arc::new(move |phase: &str| {
                    seen.lock().unwrap().push(phase.to_string());
                    if phase == stop_in {
                        flag.store(true, std::sync::atomic::Ordering::SeqCst);
                    }
                })
            });
            let scratch = dir.path().join(format!("scratch-{stop_in}"));
            let drilled = drill(
                &git,
                &target,
                "acme",
                &b.join(crate::JOURNAL_FILE),
                &scratch,
            );
            assert!(
                matches!(drilled, Err(BackupError::Cancelled(_))),
                "{stop_in}: {drilled:?}"
            );
            let seen = seen.lock().unwrap().clone();
            let last = order
                .iter()
                .position(|p| *p == seen.last().unwrap())
                .unwrap();
            let at = order.iter().position(|p| *p == stop_in).unwrap();
            assert_eq!(last, at, "{stop_in}: went on to {seen:?}");
            assert!(!scratch.exists(), "{stop_in}: its scratch folder was left");
        }
        // Not stopped, it goes through every phase.
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let git = real.clone().with_phase_watch({
            let seen = seen.clone();
            std::sync::Arc::new(move |phase: &str| seen.lock().unwrap().push(phase.to_string()))
        });
        drill(
            &git,
            &target,
            "acme",
            &b.join(crate::JOURNAL_FILE),
            &dir.path().join("whole"),
        )
        .unwrap();
        assert_eq!(*seen.lock().unwrap(), order.map(String::from));
    }

    /// A stand-in for git: a script.
    fn fake_git(dir: &Path, script: &str, quiet: Duration) -> Git {
        use std::os::unix::fs::PermissionsExt;
        let exe = dir.join("fake-git");
        std::fs::write(&exe, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        Git {
            exe,
            quiet,
            cancel: None,
            term_grace: TERM_GRACE,
            watch: Watch::default(),
        }
    }

    /// Cancelled, a git that is running is stopped at once, with all it
    /// started, and one that would start doesn't (the server's close ends
    /// a backup instead of leaving it to run against closed books).
    #[test]
    fn a_cancelled_git_stops_what_runs_and_starts_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let pid = dir.path().join("pid");
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let slow = fake_git(
            dir.path(),
            &format!("sleep 30 & echo $! > '{}'; sleep 30", pid.display()),
            Duration::from_secs(60),
        )
        .with_cancel(cancel.clone());
        let running = {
            let slow = slow.clone();
            std::thread::spawn(move || slow.run("slow", &[]))
        };
        // It is running (it wrote its child's pid) before it is cancelled.
        let by = Instant::now() + Duration::from_secs(10);
        while !pid.exists() {
            assert!(Instant::now() < by, "the fake git never started");
            std::thread::sleep(Duration::from_millis(10));
        }
        let cancelled_at = Instant::now();
        cancel.store(true, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(
            running.join().unwrap().unwrap_err(),
            BackupError::Cancelled("slow".into())
        );
        assert!(cancelled_at.elapsed() < Duration::from_secs(5));
        let child: i32 = std::fs::read_to_string(&pid)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        // `run` returned: the group was waited for, so what it started is
        // gone now, not at some later moment.
        // SAFETY: signal 0 only asks whether the process is there.
        assert_ne!(unsafe { libc::kill(child, 0) }, 0, "what it started ran on");
        // And none starts now.
        std::fs::remove_file(&pid).unwrap();
        assert_eq!(
            slow.run("again", &[]).unwrap_err(),
            BackupError::Cancelled("again".into())
        );
        assert!(!pid.exists(), "a cancelled git started anyway");
    }

    /// INVARIANT 5: a git is told to end (SIGTERM, to its whole group)
    /// before it is killed, and killed only once the grace has run out: one
    /// that ignores the signal (a trap, as a hook may set) is waited for the
    /// whole grace, then killed with all it started.
    #[test]
    fn a_git_that_ignores_sigterm_is_killed_after_the_grace_with_all_it_started() {
        let dir = tempfile::tempdir().unwrap();
        let (started, termed) = (dir.path().join("started"), dir.path().join("termed"));
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let grace = Duration::from_millis(500);
        let stubborn = fake_git(
            dir.path(),
            &format!(
                "trap 'echo term >> \"{}\"' TERM\necho $$ > '{}'\nwhile :; do sleep 0.05; done",
                termed.display(),
                started.display()
            ),
            Duration::from_secs(60),
        )
        .with_cancel(cancel.clone())
        .with_term_grace(grace);
        let running = {
            let stubborn = stubborn.clone();
            std::thread::spawn(move || stubborn.run("stubborn", &[]))
        };
        let by = Instant::now() + Duration::from_secs(30);
        while !started.exists() {
            assert!(Instant::now() < by, "the fake git never started");
            std::thread::sleep(Duration::from_millis(10));
        }
        let stopped = Instant::now();
        cancel.store(true, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(
            running.join().unwrap().unwrap_err(),
            BackupError::Cancelled("stubborn".into())
        );
        assert!(
            stopped.elapsed() >= grace,
            "killed before the grace had run out"
        );
        assert!(termed.exists(), "it was never sent SIGTERM first");
        let shell: i32 = std::fs::read_to_string(&started)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        // SAFETY: signal 0 only asks whether the process is there.
        assert_ne!(unsafe { libc::kill(shell, 0) }, 0, "it ran on");
    }

    #[test]
    fn a_git_is_stopped_only_once_it_falls_quiet_and_with_all_it_started() {
        let dir = tempfile::tempdir().unwrap();
        let limit = Duration::from_millis(500);
        // Talking all along, for longer than the limit: let run.
        let talking = fake_git(
            dir.path(),
            "for i in 1 2 3 4 5 6 7; do echo $i >&2; sleep 0.2; done; echo done",
            limit,
        );
        assert_eq!(talking.run("talk", &[]).unwrap(), "done");
        // Quiet: stopped, and what it started too.
        let pid = dir.path().join("pid");
        let quiet = fake_git(
            dir.path(),
            &format!("sleep 30 & echo $! > '{}'; sleep 30", pid.display()),
            Duration::from_millis(300),
        );
        let started = Instant::now();
        assert_eq!(
            quiet.run("wait", &[]).unwrap_err(),
            BackupError::TimedOut("wait".into())
        );
        assert!(started.elapsed() < Duration::from_secs(10));
        let child: i32 = std::fs::read_to_string(&pid)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        std::thread::sleep(Duration::from_millis(200));
        // SAFETY: signal 0 only asks whether the process is there.
        assert_ne!(
            unsafe { libc::kill(child, 0) },
            0,
            "what it started still runs"
        );
        // Done, leaving something holding its output: not waited for.
        let lingering = fake_git(dir.path(), "(sleep 30 &) ; echo ok", GIT_TIMEOUT);
        let started = Instant::now();
        assert_eq!(lingering.run("linger", &[]).unwrap(), "ok");
        assert!(started.elapsed() < Duration::from_secs(5));
        // Failing: what it said, in whole lines.
        let failing = fake_git(dir.path(), "echo 'fatal: no' >&2; exit 1", GIT_TIMEOUT);
        assert_eq!(
            failing.run("fail", &[]).unwrap_err(),
            BackupError::Git("fail".into(), "fatal: no".into())
        );
        // Said at length, then failing: the end of it is kept (where the
        // error is), from a whole line on.
        let chatty = fake_git(
            dir.path(),
            "i=0; while [ $i -lt 30000 ]; do echo \"remote: line $i of a long story that goes on\" >&2; i=$((i+1)); done; echo 'fatal: the end' >&2; exit 1",
            GIT_TIMEOUT,
        );
        match chatty.run("chat", &[]).unwrap_err() {
            BackupError::Git(_, said) => {
                assert!(said.ends_with("fatal: the end"), "{said}");
                assert!(
                    said.lines()
                        .all(|l| l.starts_with("remote: line ") || l == "fatal: the end"),
                    "{said}"
                );
            }
            other => panic!("{other:?}"),
        }
        // Where its path holds a quote, a push to this computer's own
        // repository is refused (its hooks couldn't be turned off).
        let quoted = dir.path().join("it's");
        std::fs::create_dir(&quoted).unwrap();
        let real = Git::find().expect("git is installed");
        std::os::unix::fs::symlink(&real.exe, quoted.join("git")).unwrap();
        let odd = Git {
            exe: quoted.join("git"),
            quiet: GIT_TIMEOUT,
            cancel: None,
            term_grace: TERM_GRACE,
            watch: Watch::default(),
        };
        let b = dir.path().join("book");
        let s = book(&b);
        commit(&real, &b, "acme", s.head()).unwrap();
        let disk = dir.path().join("disk");
        std::fs::create_dir(&disk).unwrap();
        let e = push(&odd, &b, "acme", &Target::Folder(disk), true).unwrap_err();
        assert!(e.to_string().contains("quote"), "{e}");
        // A long answer is kept whole (only what it says of itself is cut).
        let long = fake_git(
            dir.path(),
            "i=0; while [ $i -lt 40000 ]; do echo blobs/ab/cd/0123456789012345678901234567890123456789; i=$((i+1)); done",
            GIT_TIMEOUT,
        );
        assert_eq!(long.run("list", &[]).unwrap().lines().count(), 40000);
    }

    #[test]
    fn a_repository_with_settings_of_its_own_is_never_pushed_to_or_read() {
        let git = Git::find().expect("git is installed");
        let dir = tempfile::tempdir().unwrap();
        let b = dir.path().join("book");
        let s = book(&b);
        commit(&git, &b, "acme", s.head()).unwrap();
        let folder = dir.path().join("disk");
        std::fs::create_dir(&folder).unwrap();
        let target = Target::Folder(folder.clone());
        push(&git, &b, "acme", &target, true).unwrap();
        let repo = folder.join("acme.git");
        let marker = dir.path().join("ran");
        let config = std::fs::read_to_string(repo.join("config")).unwrap();
        // A command to run on a push; then objects borrowed from elsewhere.
        std::fs::write(
            repo.join("config"),
            format!(
                "{config}[core]\n\talternateRefsCommand = touch '{}'\n",
                marker.display()
            ),
        )
        .unwrap();
        for e in [
            push(&git, &b, "acme", &target, true).unwrap_err(),
            push(
                &git,
                &b,
                "acme",
                &Target::Remote(repo.display().to_string()),
                true,
            )
            .unwrap_err(),
            verify(&git, repo.to_str().unwrap(), &dir.path().join("v1")).unwrap_err(),
        ] {
            assert!(e.to_string().contains("settings of its own"), "{e}");
        }
        // Where a copy came from is no setting of its own; a command to
        // run on its way in is.
        std::fs::write(
            repo.join("config"),
            format!("{config}[remote \"origin\"]\n\turl = /x\n\tfetch = +refs/heads/*:refs/remotes/origin/*\n[branch \"main\"]\n\tremote = origin\n\tmerge = refs/heads/main\n"),
        )
        .unwrap();
        push(&git, &b, "acme", &target, true).unwrap();
        std::fs::write(
            repo.join("config"),
            format!(
                "{config}[remote \"origin\"]\n\tuploadpack = touch '{}'\n",
                marker.display()
            ),
        )
        .unwrap();
        assert!(push(&git, &b, "acme", &target, true).is_err());
        std::fs::write(repo.join("config"), &config).unwrap();
        std::fs::write(repo.join("objects/info/alternates"), "/elsewhere/objects\n").unwrap();
        let e = verify(&git, repo.to_str().unwrap(), &dir.path().join("v2")).unwrap_err();
        assert!(e.to_string().contains("settings of its own"), "{e}");
        assert!(!marker.exists());
        // Plain again: fine.
        std::fs::remove_file(repo.join("objects/info/alternates")).unwrap();
        push(&git, &b, "acme", &target, true).unwrap();
        // Something in it that sends git to another repository first: a
        // .git in it, or another's files named in it.
        let other = dir.path().join("other.git");
        git.run(
            "init",
            &[os("init"), os("--bare"), os("-q"), other.as_os_str()],
        )
        .unwrap();
        for (name, text) in [
            (".git", format!("gitdir: {}\n", other.display())),
            ("commondir", format!("{}\n", other.display())),
            ("gitdir", format!("{}\n", other.display())),
        ] {
            std::fs::write(repo.join(name), text).unwrap();
            let e = push(&git, &b, "acme", &target, true).unwrap_err();
            assert!(e.to_string().contains("settings of its own"), "{name}: {e}");
            let e = verify(
                &git,
                repo.to_str().unwrap(),
                &dir.path().join(format!("v-{name}")),
            )
            .unwrap_err();
            assert!(e.to_string().contains("settings of its own"), "{name}: {e}");
            std::fs::remove_file(repo.join(name)).unwrap();
        }
        // Something beside it git would take in its place: a repository
        // named the same with .git after it, or a bundle.
        let decoy = folder.join("acme.git.git");
        git.run(
            "init",
            &[os("init"), os("--bare"), os("-q"), decoy.as_os_str()],
        )
        .unwrap();
        let e = push(&git, &b, "acme", &target, true).unwrap_err();
        assert!(e.to_string().contains("settings of its own"), "{e}");
        std::fs::remove_dir_all(&decoy).unwrap();
        std::fs::write(folder.join("acme.git.bundle"), b"x").unwrap();
        let e = verify(&git, repo.to_str().unwrap(), &dir.path().join("v-bundle")).unwrap_err();
        assert!(e.to_string().contains("settings of its own"), "{e}");
        std::fs::remove_file(folder.join("acme.git.bundle")).unwrap();
        // A HEAD and a config but no objects or refs: not a repository git
        // stops at (it would look beside it).
        let hollow = dir.path().join("hollow.git");
        std::fs::create_dir(&hollow).unwrap();
        std::fs::write(hollow.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(hollow.join("config"), "[core]\n\tbare = true\n").unwrap();
        let e = push(
            &git,
            &b,
            "acme",
            &Target::Remote(hollow.display().to_string()),
            true,
        )
        .unwrap_err();
        assert!(e.to_string().contains("name the repository itself"), "{e}");
        // Its parts, or itself, a link elsewhere: refused (what is written
        // there would land elsewhere).
        let elsewhere = dir.path().join("elsewhere-objects");
        std::fs::rename(repo.join("objects"), &elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, repo.join("objects")).unwrap();
        let e = push(&git, &b, "acme", &target, true).unwrap_err();
        assert!(e.to_string().contains("settings of its own"), "{e}");
        std::fs::remove_file(repo.join("objects")).unwrap();
        std::fs::rename(&elsewhere, repo.join("objects")).unwrap();
        let moved = dir.path().join("moved.git");
        std::fs::rename(&repo, &moved).unwrap();
        std::os::unix::fs::symlink(&moved, &repo).unwrap();
        let e = push(&git, &b, "acme", &target, true).unwrap_err();
        assert!(e.to_string().contains("settings of its own"), "{e}");
        std::fs::remove_file(&repo).unwrap();
        std::fs::rename(&moved, &repo).unwrap();
        push(&git, &b, "acme", &target, true).unwrap();
        // The folder holding it a link (to a disk's mount, say): fine.
        let linked = dir.path().join("linked-disk");
        std::os::unix::fs::symlink(&folder, &linked).unwrap();
        push(&git, &b, "acme", &Target::Folder(linked), true).unwrap();
        // Shared, a copy's origin, a newer ref store: all plain.
        let shared = dir.path().join("shared.git");
        git.run(
            "init",
            &[
                os("init"),
                os("--bare"),
                os("-q"),
                os("--shared"),
                shared.as_os_str(),
            ],
        )
        .unwrap();
        let plain = std::fs::read_to_string(shared.join("config")).unwrap();
        std::fs::write(
            shared.join("config"),
            format!(
                "{plain}[remote \"origin\"]\n\tmirror = true\n[extensions]\n\trefStorage = files\n"
            ),
        )
        .unwrap();
        plain_repository(&git, shared.to_str().unwrap()).unwrap();
        // Not the repository itself (git would look beside it, or in it).
        for address in [
            folder.join("acme"),
            folder.clone(),
            dir.path().join("nothing-here"),
        ] {
            let e = push(
                &git,
                &b,
                "acme",
                &Target::Remote(address.display().to_string()),
                true,
            )
            .unwrap_err();
            assert!(
                e.to_string().contains("name the repository itself"),
                "{address:?}: {e}"
            );
        }
        assert!(!marker.exists());
    }

    #[test]
    fn a_backup_folder_gone_from_a_connected_disk_is_never_made_again() {
        let git = Git::find().expect("git is installed");
        let dir = tempfile::tempdir().unwrap();
        let b = dir.path().join("book");
        let s = book(&b);
        commit(&git, &b, "acme", s.head()).unwrap();
        // A mount point with nothing mounted: there, but not the backup.
        let mount = dir.path().join("mnt");
        std::fs::create_dir(&mount).unwrap();
        let target = Target::Folder(mount.clone());
        assert_eq!(
            push(&git, &b, "acme", &target, false).unwrap_err(),
            BackupError::NoBackupThere
        );
        // A folder that isn't there at all: not connected.
        let gone = Target::Folder(dir.path().join("gone"));
        assert_eq!(
            push(&git, &b, "acme", &gone, false).unwrap_err(),
            BackupError::NoFolder
        );
        assert!(!mount.join("acme.git").exists());
        // Never backed up there yet: made.
        push(&git, &b, "acme", &target, true).unwrap();
        assert!(mount.join("acme.git").join("HEAD").is_file());
    }

    #[test]
    fn what_git_said_cut_short_ends_at_its_last_whole_line() {
        assert_eq!(
            said(b"ot/books/acme'\nfatal: one\nfatal: two", true),
            "fatal: one\nfatal: two"
        );
        assert_eq!(said(b"ot/books/acme' is not", true), "");
        assert_eq!(
            said(b"fatal: one\nfatal: two", false),
            "fatal: one\nfatal: two"
        );
    }

    /// Every file under `dir`.
    fn walk(dir: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                out.extend(walk(&p));
            } else {
                out.push(p);
            }
        }
        out
    }
}
