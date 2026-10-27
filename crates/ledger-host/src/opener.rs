//! Holding the books: how every holder takes its lease (plan 3.9).
//!
//! ```text
//! GOAL: a holder gets its lease on the one host, starting one when none
//!       answers, even when several holders start at once; across a
//!       restart it simply holds again; a host that keeps failing is
//!       given up on, plainly, instead of retried forever.
//! SOURCE: plan/served.md 3.9 ("One host, even in a race", "Closing":
//!         backoff 1, 2, 4 … 60 s, at most five tries; "Restarting":
//!         openers wait on update.lock); tests host_respawns,
//!         respawn_backs_off.
//! INVARIANTS:
//!   1. Before each try the holder waits while a restart holds
//!      run/update.lock.
//!   2. No host answering: it starts one (cleanly, start.rs) and keeps
//!      trying;
//!      a host that is closing or restarting turns it away and it tries
//!      again until a host takes the lease.
//!   3. A start that brings no lease within START_WAIT (or whose host
//!      died before taking its lock), a host that doesn't answer, and a
//!      lease that ends with server_failed or with no reason at all, are
//!      failures: the holder tries again after knobs.backoff, doubling
//!      each time (at most 60 s), MAX_TRIES times; the failure after that
//!      gives up, naming `wanderbooks doctor`. Books that stayed open for
//!      STABLE clear the count; a server that fails soon after opening
//!      doesn't.
//!   4. The host's end of the socket must be this account.
//!   5. A holder that has held before, turned away because the owner
//!      closed the books, stays closed (HoldError::ClosedByOwner); a new
//!      holder waits for a new host.
//! ```

use crate::knobs::Knobs;
use crate::lease::{self, Ask, Hello, HostMsg, Kind, Reason};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
use tokio::time::Instant;

/// How long a started host has to take a lease (longer than a new host
/// waits for an old one's lock).
const START_WAIT: Duration = Duration::from_secs(45);
/// How long a started host has to take its lock: after that, no host
/// holding it means the one started has died.
const LOCK_WAIT: Duration = Duration::from_secs(2);
/// How long the host has to answer a hello.
const ANSWER_WAIT: Duration = Duration::from_secs(10);
/// Tries after a failure before giving up (INVARIANT 3).
pub const MAX_TRIES: u32 = 5;
/// How long the books must stay open for earlier failures to be
/// forgotten (INVARIANT 3).
const STABLE: Duration = Duration::from_secs(60);
/// The longest wait between tries.
const MAX_BACKOFF: Duration = Duration::from_secs(60);

/// What a holder says when the host keeps failing.
pub const KEEPS_FAILING: &str =
    "the books server keeps failing to start: run `wanderbooks doctor` to see why";

/// Why no lease was taken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HoldError {
    /// The owner closed the books, and this holder had held them
    /// (INVARIANT 5).
    ClosedByOwner,
    /// Anything else, in plain words.
    Failed(String),
}

impl std::fmt::Display for HoldError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HoldError::ClosedByOwner => write!(f, "the owner closed the books"),
            HoldError::Failed(why) => f.write_str(why),
        }
    }
}

impl From<String> for HoldError {
    fn from(why: String) -> Self {
        HoldError::Failed(why)
    }
}

impl From<&str> for HoldError {
    fn from(why: &str) -> Self {
        HoldError::Failed(why.to_string())
    }
}

/// Takes leases for one holder.
pub struct Opener {
    root: Option<PathBuf>,
    run: PathBuf,
    knobs: Knobs,
    kind: Kind,
    book: Option<String>,
    failures: u32,
    /// When the books last opened on a lease.
    opened_at: Option<std::time::Instant>,
    /// Whether this holder has held a lease (INVARIANT 5).
    has_held: bool,
    /// Flags the server this opener starts is started with (P9a):
    /// `--isolation` and `--http`. Empty for every ordinary holder.
    server_flags: Vec<String>,
}

/// A held lease: the books stay open while it lives.
pub struct Lease {
    /// Its number.
    pub id: u64,
    lines: tokio::io::Lines<tokio::io::BufReader<tokio::io::Take<tokio::net::unix::OwnedReadHalf>>>,
    _write: tokio::net::unix::OwnedWriteHalf,
}

impl Lease {
    /// The host's next word: `Ready`, or `Ending` with why; `None` when
    /// the host is gone without a word.
    pub async fn next(&mut self) -> Option<HostMsg> {
        loop {
            match self.lines.next_line().await {
                Ok(Some(line)) => match serde_json::from_str::<HostMsg>(&line) {
                    Ok(msg) => return Some(msg),
                    Err(_) => continue,
                },
                _ => return None,
            }
        }
    }
}

impl Opener {
    /// An opener for the root (`root`, else the one this account's host
    /// finds), holding as `kind` for `book`.
    pub fn new(
        root: Option<&Path>,
        knobs: Knobs,
        kind: Kind,
        book: Option<String>,
    ) -> Result<Self, String> {
        let found = ledger_store::root::Root::find(root).map_err(|e| e.to_string())?;
        let run = found.run_dir().map_err(|e| e.to_string())?;
        Ok(Self {
            root: root.map(|_| found.path().to_path_buf()),
            run,
            knobs,
            kind,
            book,
            failures: 0,
            opened_at: None,
            has_held: false,
            server_flags: Vec::new(),
        })
    }

    /// Start the server this opener brings up with these flags (P9a:
    /// `--isolation`, `--http`). Only `serve` uses it: on the owner's own
    /// machine it is the one holder, so the host it starts is the server's
    /// only one. A host already running was started with other flags, and
    /// this one has no way to change it.
    pub fn starting_with(mut self, flags: Vec<String>) -> Self {
        self.server_flags = flags;
        self
    }

    /// The timings in force.
    pub fn knobs(&self) -> Knobs {
        self.knobs
    }

    /// The root's `run/` folder.
    pub fn run_dir(&self) -> &Path {
        &self.run
    }

    /// The books opened on a lease (INVARIANT 3).
    pub fn opened(&mut self) {
        self.opened_at = Some(std::time::Instant::now());
    }

    /// Count a failure and wait before the next try, or give up
    /// (INVARIANT 3).
    pub async fn failed(&mut self) -> Result<(), String> {
        if self
            .opened_at
            .take()
            .is_some_and(|at| at.elapsed() >= STABLE)
        {
            self.failures = 0;
        }
        self.failures += 1;
        if self.failures > MAX_TRIES {
            return Err(KEEPS_FAILING.into());
        }
        tokio::time::sleep(backoff(self.knobs.backoff, self.failures)).await;
        Ok(())
    }

    /// Start counting failures over (a holder that waited a while after
    /// giving up tries again); what it has held is kept (INVARIANT 5).
    pub fn forget_failures(&mut self) {
        self.failures = 0;
    }

    /// Take a lease, starting a host if none answers (INVARIANTS 1-4).
    pub async fn hold(&mut self) -> Result<Lease, HoldError> {
        let mut started: Option<Instant> = None;
        loop {
            wait_for_restart(&self.run).await;
            let short = ledger_store::root::SocketPath::new(&self.run, lease::LEASE_SOCKET)
                .map_err(|e| format!("can't reach the books' host: {e}"))?;
            match tokio::net::UnixStream::connect(short.path()).await {
                Ok(stream) => {
                    if !ledger_mcp::local::peer_is_us(&stream) {
                        return Err(
                            "the books' host socket belongs to another account: not used".into(),
                        );
                    }
                    match self.ask(stream).await? {
                        Asked::Held(lease) => {
                            self.has_held = true;
                            return Ok(lease);
                        }
                        Asked::OwnerClosed if self.has_held => {
                            return Err(HoldError::ClosedByOwner);
                        }
                        Asked::OwnerClosed => {
                            tokio::time::sleep(Duration::from_millis(100)).await;
                        }
                        Asked::Again => tokio::time::sleep(Duration::from_millis(100)).await,
                        Asked::Failed => {
                            started = None;
                            self.failed().await?;
                        }
                    }
                }
                // No host answering, or one going away as we came.
                Err(e) if no_host(&e) => {
                    // A host holding the lock is starting or closing: wait
                    // for it rather than start another.
                    let held = host_lock_held(&self.run);
                    match started {
                        // A start that fails (a spawn error) is a
                        // failure like any other (INVARIANT 3).
                        None if !held => match self.start().await {
                            Ok(()) => started = Some(Instant::now()),
                            Err(_) => self.failed().await?,
                        },
                        None => {}
                        // The one started died (no host holds the lock),
                        // or never answered.
                        Some(at)
                            if (!held && at.elapsed() > LOCK_WAIT) || at.elapsed() > START_WAIT =>
                        {
                            started = None;
                            self.failed().await?;
                        }
                        Some(_) => {}
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                Err(e) => {
                    return Err(format!("can't reach the books' host: {}", e.kind()).into());
                }
            }
        }
    }

    /// Start a host, cleanly (start.rs).
    async fn start(&self) -> Result<(), String> {
        let root = self.root.clone();
        let knobs = self.knobs;
        let flags = self.server_flags.clone();
        tokio::task::spawn_blocking(move || {
            crate::start::start_host(root.as_deref(), &knobs, &flags)
        })
        .await
        .map_err(|e| e.to_string())?
    }

    /// Say hello on a connection and read the host's first word.
    async fn ask(&self, stream: tokio::net::UnixStream) -> Result<Asked, String> {
        let (read, mut write) = stream.into_split();
        let hello = Hello {
            proto: lease::PROTO,
            ask: Ask::Hold {
                kind: self.kind,
                pid: std::process::id(),
                book: self.book.clone(),
            },
        };
        if write
            .write_all(lease::line(&hello).as_bytes())
            .await
            .is_err()
        {
            return Ok(Asked::Again);
        }
        let mut lines = tokio::io::BufReader::new(read.take(lease::MAX_LINE as u64 * 4)).lines();
        let first = match tokio::time::timeout(ANSWER_WAIT, lines.next_line()).await {
            Ok(Ok(Some(line))) => serde_json::from_str::<HostMsg>(&line)
                .map_err(|e| format!("the books' host answered oddly: {e}"))?,
            // Gone mid-hello (closing, or a crash): try again.
            Ok(_) => return Ok(Asked::Again),
            // Not answering: a failure, tried again after a wait.
            Err(_) => return Ok(Asked::Failed),
        };
        Ok(match first {
            HostMsg::Held { lease } => Asked::Held(Lease {
                id: lease,
                lines,
                _write: write,
            }),
            HostMsg::Ending {
                reason: Reason::ServerFailed,
                ..
            } => Asked::Failed,
            HostMsg::Ending {
                reason: Reason::ClosedByOwner,
                ..
            } => Asked::OwnerClosed,
            HostMsg::Ending { .. } => Asked::Again,
            HostMsg::Refused(why) => return Err(why),
            other => return Err(format!("the books' host answered oddly: {other:?}")),
        })
    }
}

/// What a hello came to.
enum Asked {
    Held(Lease),
    Again,
    Failed,
    /// Turned away: the owner is closing the books (INVARIANT 5).
    OwnerClosed,
}

/// The wait before try `n` (1-based): `base`, doubling, at most 60 s.
pub fn backoff(base: Duration, n: u32) -> Duration {
    let factor = 1u32 << n.saturating_sub(1).min(16);
    base.saturating_mul(factor).min(MAX_BACKOFF)
}

/// Whether a connect error means no host is answering (none there, or
/// one closing as we came), rather than something wrong.
fn no_host(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::NotFound
            | std::io::ErrorKind::ConnectionRefused
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::BrokenPipe
    )
}

/// Whether a books server holds run/server.lock now (one finishing after
/// its host is gone included).
pub fn server_lock_held(run: &Path) -> bool {
    match std::fs::File::open(run.join(ledger_mcp::local::SERVER_LOCK)) {
        Ok(file) => matches!(
            file.try_lock_shared(),
            Err(std::fs::TryLockError::WouldBlock)
        ),
        Err(_) => false,
    }
}

/// Whether a host holds run/host.lock now (starting, serving or closing).
pub fn host_lock_held(run: &Path) -> bool {
    match std::fs::File::open(run.join(lease::HOST_LOCK)) {
        Ok(file) => matches!(
            file.try_lock_shared(),
            Err(std::fs::TryLockError::WouldBlock)
        ),
        Err(_) => false,
    }
}

/// Wait while a restart holds run/update.lock (INVARIANT 1).
async fn wait_for_restart(run: &Path) {
    // Looked at, not waited on in a thread: a holder that gives up while
    // a restart runs is never stuck behind it.
    let path = run.join(lease::UPDATE_LOCK);
    loop {
        // No lock file: no restart has ever happened, so none is on.
        let restarting = std::fs::File::open(&path)
            .is_ok_and(|f| matches!(f.try_lock_shared(), Err(std::fs::TryLockError::WouldBlock)));
        if !restarting {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Ask the host one thing on its own connection (leases, an intent):
/// `Ok(None)` when no host is running.
pub fn ask_once(run: &Path, ask: Ask) -> Result<Option<HostMsg>, String> {
    ask_once_within(run, ask, Duration::from_secs(120))
}

/// [`ask_once`], waiting at most `wait` for the answer (a probe).
pub fn ask_once_within(run: &Path, ask: Ask, wait: Duration) -> Result<Option<HostMsg>, String> {
    use std::io::{BufRead, Write};
    let short = ledger_store::root::SocketPath::new(run, lease::LEASE_SOCKET)
        .map_err(|e| format!("can't reach the books' host: {e}"))?;
    let stream = match std::os::unix::net::UnixStream::connect(short.path()) {
        Ok(s) => s,
        Err(e) if no_host(&e) => return Ok(None),
        Err(e) => return Err(format!("can't reach the books' host: {}", e.kind())),
    };
    if !ledger_mcp::local::peer_is_us(&stream) {
        return Err("the books' host socket belongs to another account: not used".into());
    }
    stream
        .set_read_timeout(Some(wait))
        .map_err(|e| e.to_string())?;
    let not_answering = |e: std::io::Error| match e.kind() {
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut => {
            "the books' host isn't answering".to_string()
        }
        kind => format!("the books' host didn't answer: {kind}"),
    };
    (&stream)
        .write_all(
            lease::line(&Hello {
                proto: lease::PROTO,
                ask,
            })
            .as_bytes(),
        )
        .map_err(|e| format!("can't reach the books' host: {}", e.kind()))?;
    let mut answer = String::new();
    std::io::BufReader::new(std::io::Read::take(&stream, lease::MAX_LINE as u64 * 4))
        .read_line(&mut answer)
        .map_err(not_answering)?;
    if answer.is_empty() {
        return Ok(None);
    }
    serde_json::from_str(answer.trim_end())
        .map(Some)
        .map_err(|e| format!("the books' host answered oddly: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wait_doubles_from_the_base_up_to_a_minute() {
        let s = Duration::from_secs;
        let waits: Vec<Duration> = (1..=7).map(|n| backoff(s(1), n)).collect();
        assert_eq!(waits, [s(1), s(2), s(4), s(8), s(16), s(32), s(60)]);
        assert_eq!(backoff(s(1), 0), s(1));
        assert_eq!(backoff(s(1), 400), s(60));
        assert_eq!(
            backoff(Duration::from_millis(10), 3),
            Duration::from_millis(40)
        );
    }

    #[tokio::test]
    async fn after_five_failures_in_a_row_it_gives_up() {
        let dir = tempfile::tempdir().unwrap();
        let mut o = Opener {
            root: None,
            run: dir.path().to_path_buf(),
            knobs: Knobs {
                backoff: Duration::from_millis(1),
                ..Knobs::default()
            },
            kind: Kind::Cli,
            book: None,
            failures: 0,
            opened_at: None,
            has_held: false,
            server_flags: Vec::new(),
        };
        for _ in 0..MAX_TRIES {
            o.failed().await.unwrap();
        }
        assert_eq!(o.failed().await.unwrap_err(), KEEPS_FAILING);
        // Opening briefly doesn't clear the count; staying open does.
        o.opened();
        assert!(o.failed().await.is_err(), "a brief open cleared the count");
        o.opened_at = Some(std::time::Instant::now() - STABLE);
        assert!(o.failed().await.is_ok(), "staying open clears the count");
        assert_eq!(o.failures, 1);
    }

    #[test]
    fn a_host_going_away_is_no_host() {
        use std::io::{Error, ErrorKind};
        for kind in [
            ErrorKind::NotFound,
            ErrorKind::ConnectionRefused,
            ErrorKind::ConnectionReset,
            ErrorKind::ConnectionAborted,
            ErrorKind::BrokenPipe,
        ] {
            assert!(no_host(&Error::from(kind)), "{kind:?}");
        }
        for kind in [ErrorKind::PermissionDenied, ErrorKind::InvalidInput] {
            assert!(!no_host(&Error::from(kind)), "{kind:?}");
        }
    }

    #[test]
    fn no_host_is_none() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(ask_once(dir.path(), Ask::Leases).unwrap(), None);
    }
}
