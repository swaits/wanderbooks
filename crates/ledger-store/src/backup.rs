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
//!      root.rs).
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
/// The longest a git command may take before it is stopped.
pub const GIT_TIMEOUT: Duration = Duration::from_secs(300);

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

/// The git program, found once.
#[derive(Debug, Clone)]
pub struct Git {
    exe: PathBuf,
    /// How long it may say nothing before it is stopped (GIT_TIMEOUT).
    quiet: Duration,
}

impl Git {
    /// git, from the usual places (never the working directory).
    pub fn find() -> Result<Git, BackupError> {
        let dirs = std::env::var_os("PATH")
            .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
            .unwrap_or_default();
        dirs.iter()
            .chain(
                ["/usr/bin", "/bin", "/usr/local/bin", "/opt/homebrew/bin"]
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
            })
            .ok_or(BackupError::NoGit)
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
        use std::sync::atomic::{AtomicU64, Ordering};
        let mut c = self.command();
        c.args(args);
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            c.process_group(0);
        }
        let mut child = c.spawn().map_err(|e| match e.kind() {
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
        // Git and every process it started (its own group, above).
        #[cfg(unix)]
        // SAFETY: kill has no preconditions; the group is git's own
        // (process_group(0) above), so this stops it and what it started.
        let kill_group = |pid: u32| unsafe {
            libc::kill(-(pid as libc::pid_t), libc::SIGKILL);
        };
        #[cfg(not(unix))]
        let kill_group = |_: u32| {};
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            let quiet_for = started
                .elapsed()
                .saturating_sub(Duration::from_millis(heard.load(Ordering::Relaxed)));
            if quiet_for > self.quiet {
                kill_group(child.id());
                let _ = child.kill();
                let _ = child.wait();
                return Err(BackupError::TimedOut(what.to_string()));
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        // What it started may still hold its pipes: not waited for long.
        kill_group(child.id());
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

/// The most kept of what a git says of itself.
const OUTPUT_MAX: usize = 1 << 20;

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
        git.run(
            "init",
            &[os("init"), os("--bare"), os("-q"), dir.as_os_str()],
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
    clear_old_locks(&archive);
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
        git.run("add", &add)?;
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
        git.run("rm", &rm)?;
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
        files_under(book_dir, Path::new(p), &mut on_disk)?;
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
    git.run(
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
    clear_old_locks(&archive);
    let git_dir = format!("--git-dir={}", archive.display());
    let work_tree = format!("--work-tree={}", book_dir.display());
    git.run(
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
    )?;
    Ok(())
}

/// The locks a git left behind (stopped, the server killed) would refuse
/// every later backup: cleared once older than any git may run quiet,
/// so never one a git still at work holds.
fn clear_old_locks(archive: &Path) {
    for lock in ["index.lock", "HEAD.lock", "refs/heads/main.lock"] {
        let path = archive.join(lock);
        let old = std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .is_ok_and(|t| t.elapsed().is_ok_and(|e| e > GIT_TIMEOUT));
        if old {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Every file under `rel` in `base` (or `rel` itself), as paths relative to
/// `base` with `/` between their parts.
fn files_under(base: &Path, rel: &Path, out: &mut Vec<String>) -> Result<(), BackupError> {
    let full = base.join(rel);
    let meta = std::fs::symlink_metadata(&full)?;
    if meta.is_dir() {
        for entry in std::fs::read_dir(&full)? {
            files_under(base, &rel.join(entry?.file_name()), out)?;
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
                git.run(
                    "init",
                    &[os("init"), os("--bare"), os("-q"), repo.as_os_str()],
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
    match git.run("push", &args) {
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
        let (meta, path) = entry.split_once('\t').unwrap_or((entry, ""));
        let mode = meta.split(' ').next().unwrap_or("");
        let top = path.split('/').next().unwrap_or("");
        if !["100644", "100755"].contains(&mode) || !BACKED_UP.contains(&top) {
            return Err(BackupError::Damaged(format!(
                "it holds {path:?}, which no book keeps"
            )));
        }
    }
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
    let mut store = crate::Store::open(scratch).map_err(|e| BackupError::Damaged(e.to_string()))?;
    store
        .verify_and_heal()
        .map_err(|e| BackupError::Damaged(e.to_string()))?;
    let blobs = store
        .verify_blobs()
        .map_err(|e| BackupError::Damaged(e.to_string()))?;
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
    if !named.is_empty() {
        use sha2::{Digest, Sha256};
        let vault = scratch.join(crate::root::VAULT_DIR);
        let mut held = std::collections::BTreeSet::new();
        if vault.is_dir() {
            for f in std::fs::read_dir(&vault)? {
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
        match is_prefix(live, &v.tree.join(crate::JOURNAL_FILE)) {
            Ok(true) => Ok(v.head),
            Ok(false) => Err(BackupError::Damaged(
                "its journal isn't this book's history".into(),
            )),
            Err(e) => Err(e.into()),
        }
    });
    let _ = std::fs::remove_dir_all(scratch);
    checked
}

/// Whether the journal at `backup` is a byte prefix of the one at `live`
/// (a drill: the backup holds the book's history, nothing else).
pub fn is_prefix(live: &Path, backup: &Path) -> std::io::Result<bool> {
    use std::io::Read;
    let mut a = std::fs::File::open(live)?;
    let mut b = std::fs::File::open(backup)?;
    let (mut x, mut y) = (vec![0u8; 64 * 1024], vec![0u8; 64 * 1024]);
    loop {
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
        let then = std::time::SystemTime::now() - GIT_TIMEOUT - Duration::from_secs(1);
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

    /// A stand-in for git: a script.
    fn fake_git(dir: &Path, script: &str, quiet: Duration) -> Git {
        use std::os::unix::fs::PermissionsExt;
        let exe = dir.join("fake-git");
        std::fs::write(&exe, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        Git { exe, quiet }
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
