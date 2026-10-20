//! `wanderbooks host`: holds the books open while any lease lasts, and
//! closes them when none does (plan 3.9).
//!
//! ```text
//! GOAL: the books are open exactly while something holds them; closing
//!       is always clean; one host and one server per root, even in a
//!       race; nothing of the host outlives the last holder by more than
//!       the close delay.
//! SOURCE: plan/served.md 3.9 ("Who holds them", "Starting the host
//!         cleanly", "One host, even in a race", "Closing", "Restarting",
//!         "One version at a time"), D-21, D-23.
//! INVARIANTS:
//!   1. One host per root: it holds run/host.lock for its whole life; a
//!      host that can't take it within HOST_LOCK_WAIT exits 3, having
//!      touched nothing of the host that has it (it may have made run/,
//!      and wrote and removed its own probe file).
//!   2. It refuses to run confined (seccomp, or a root it can't write),
//!      and closes every descriptor above 2 it was handed.
//!   3. It reads no books: it starts the server from its own executable
//!      (/proc/self/exe on Linux, so a replaced file can't mix versions)
//!      and closes the books by closing the server's standard input.
//!   4. The books close `close_after` after the last lease ends (or after
//!      the start, if none comes); a lease arriving in that time cancels
//!      the close.
//!   5. The host never ends a lease without first telling it why.
//!   6. Close (the owner's intent, SIGTERM, SIGINT or SIGHUP): every lease
//!      is told closed_by_owner and ended, new ones are turned away
//!      (closing); once the holders have let go (they send nothing more),
//!      or LEASE_DRAIN has passed, the server is closed: it finishes what
//!      is in progress and exits; the socket is removed, and the host
//!      exits 0.
//!   7. Restart (its executable's file replaced): run/update.lock is
//!      taken first, every lease told restarting, the server closed as
//!      in 6, a new host started from the file now on disk; host.lock is
//!      let go, and update.lock only once the new host answers (or
//!      HOST_LOCK_WAIT has passed), so waiting openers find it.
//!   8. The server stopping on its own: every lease is told server_failed,
//!      and the host exits 1.
//!   9. Only this account's connections, and only lease.rs's messages.
//!  10. A closing server is never killed: it finishes what it is doing,
//!      however long that takes.
//!  11. The owner's close while restarting is refused (the restart is
//!      already under way; close again once it is done).
//! ```

use crate::knobs::{FLAG, Knobs};
use crate::lease::{self, Ask, HostMsg, Intent, LeaseInfo, Reason};
use ledger_store::root::Root;
use std::collections::BTreeMap;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot};
use tokio::time::Instant;
use tokio_util::task::TaskTracker;

/// How long a new host waits for another to let go of the lock (one
/// closing, or a probe) before giving up (INVARIANT 1).
const HOST_LOCK_WAIT: Duration = Duration::from_secs(30);
/// How long a probe of another host waits for its answer.
const PROBE_WAIT: Duration = Duration::from_secs(2);
/// How long a connection has to say what it wants.
const HELLO_WAIT: Duration = Duration::from_secs(10);
/// How long holders have to let go of their ended leases (having heard
/// why, so they send no more) before the server is closed anyway.
const LEASE_DRAIN: Duration = Duration::from_secs(2);
/// The line a host writes to its log first.
pub const STARTED: &str = "wanderbooks host: starting";
/// The server's log, in the root's `run/`.
pub const SERVER_LOG: &str = "server.log";
/// A log larger than this is moved aside (to `.1`) when the host starts.
const LOG_LIMIT: u64 = 8 * 1024 * 1024;
/// The exit code when another host holds the root.
pub const EXIT_ANOTHER_HOST: i32 = 3;

/// Close every descriptor above 2 (INVARIANT 2).
fn close_stray_descriptors() {
    #[cfg(target_os = "linux")]
    {
        // SAFETY: close_range only closes descriptors; nothing is open
        // yet that this program needs.
        if unsafe { libc::syscall(libc::SYS_close_range, 3u32, u32::MAX, 0u32) } == 0 {
            return;
        }
    }
    // Elsewhere, or an old kernel: one by one.
    // SAFETY: getdtablesize has no preconditions.
    let top = unsafe { libc::getdtablesize() }.clamp(3, 65536);
    for fd in 3..top {
        // SAFETY: closing a descriptor that may not be open is harmless.
        unsafe { libc::close(fd) };
    }
}

/// Whether this process is confined: seccomp in force, or a root it
/// can't write (Landlock) (INVARIANT 2).
fn confined(run: &Path) -> Option<&'static str> {
    if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
        let seccomp = status
            .lines()
            .find_map(|l| l.strip_prefix("Seccomp:"))
            .map(str::trim);
        if seccomp.is_some_and(|mode| mode != "0") {
            return Some("a seccomp filter");
        }
    }
    let probe = run.join(format!(".host-probe-{}", std::process::id()));
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&probe)
    {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            None
        }
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            Some("a sandbox that won't let it write the books' folder")
        }
        Err(_) => None,
    }
}

/// Take run/host.lock, waiting a while for a host that is closing
/// (INVARIANT 1).
fn take_host_lock(run: &Path) -> Result<Option<std::fs::File>, String> {
    take_host_lock_within(run, HOST_LOCK_WAIT)
}

/// [`take_host_lock`], waiting at most `wait` (tests shorten it).
fn take_host_lock_within(run: &Path, wait: Duration) -> Result<Option<std::fs::File>, String> {
    let path = run.join(lease::HOST_LOCK);
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .open(&path)
        .map_err(|e| format!("can't open the host lock: {}", e.kind()))?;
    let deadline = std::time::Instant::now() + wait;
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(Some(file)),
            // A host that is open and serving: this one isn't needed.
            Err(std::fs::TryLockError::WouldBlock)
                if matches!(
                    crate::opener::ask_once_within(run, Ask::Leases, PROBE_WAIT),
                    Ok(Some(HostMsg::Leases { .. }))
                ) =>
            {
                return Ok(None);
            }
            // One starting or closing: wait for it.
            Err(std::fs::TryLockError::WouldBlock) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(std::fs::TryLockError::WouldBlock) => return Ok(None),
            Err(std::fs::TryLockError::Error(e)) => {
                return Err(format!("can't take the host lock: {}", e.kind()));
            }
        }
    }
}

/// Move a log larger than LOG_LIMIT aside, then open it for appending.
fn open_log(path: &Path) -> std::io::Result<std::fs::File> {
    open_log_within(path, LOG_LIMIT)
}

/// [`open_log`], moving aside a log larger than `limit` bytes.
fn open_log_within(path: &Path, limit: u64) -> std::io::Result<std::fs::File> {
    if std::fs::metadata(path).is_ok_and(|m| m.len() > limit) {
        let _ = std::fs::rename(path, path.with_extension("log.1"));
    }
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)
}

/// The executable the server is started from (INVARIANT 3).
fn children_executable() -> PathBuf {
    if cfg!(target_os = "linux") {
        PathBuf::from("/proc/self/exe")
    } else {
        std::env::current_exe().unwrap_or_else(|_| PathBuf::from("wanderbooks"))
    }
}

/// The running executable's file identity, and its file's now: they
/// differ once the file was replaced (INVARIANT 7).
fn running_identity() -> Option<(u64, u64)> {
    let running = if cfg!(target_os = "linux") {
        std::fs::metadata("/proc/self/exe").ok()?
    } else {
        std::fs::metadata(std::env::current_exe().ok()?).ok()?
    };
    Some((running.dev(), running.ino()))
}

/// The identity of the file at this executable's path, if it is a
/// program (a file half-written or gone doesn't count yet).
fn file_identity() -> Option<(u64, u64)> {
    identity_of(&crate::start::this_executable().ok()?)
}

/// The identity (device, inode) of the program at `path`: a file someone
/// may run; else None.
fn identity_of(path: &Path) -> Option<(u64, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    (meta.is_file() && meta.permissions().mode() & 0o111 != 0).then(|| (meta.dev(), meta.ino()))
}

/// Wait at most `wait` for a host to answer on the lease socket: whether
/// one did.
fn wait_for_a_host(run: &Path, wait: Duration) -> bool {
    let deadline = std::time::Instant::now() + wait;
    loop {
        if matches!(
            crate::opener::ask_once_within(run, Ask::Leases, PROBE_WAIT),
            Ok(Some(HostMsg::Leases { .. }))
        ) {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The host's command line.
struct Args {
    /// `--root`, if given (tests): passed on to the server; otherwise the
    /// server finds the root itself, and no command line names it.
    root: Option<PathBuf>,
    knobs: Knobs,
}

fn parse_args(prog: &str, args: impl IntoIterator<Item = String>) -> Result<Option<Args>, String> {
    let mut root = None;
    let mut knobs = Knobs::default();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--root" => root = Some(PathBuf::from(args.next().ok_or("--root needs a folder")?)),
            FLAG => knobs = Knobs::parse(&args.next().ok_or("--test-knobs needs a value")?)?,
            "--help" | "-h" => {
                eprintln!("usage: {prog} [--root DIR]");
                eprintln!("  Holds the books open while anything holds a lease on it; started by");
                eprintln!("  whatever opens the books first, not by hand.");
                return Ok(None);
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(Some(Args { root, knobs }))
}

/// `wanderbooks host`.
pub fn run(
    prog: &str,
    args: impl IntoIterator<Item = String>,
) -> Result<(), Box<dyn std::error::Error>> {
    close_stray_descriptors();
    // The first line of its log: a starter knows from it that the host
    // itself ran (start.rs INVARIANT 3).
    eprintln!("{STARTED} (pid {})", std::process::id());
    let args = match parse_args(prog, args) {
        Ok(Some(args)) => args,
        Ok(None) => return Ok(()),
        Err(why) => {
            eprintln!("{why}");
            eprintln!("usage: {prog} [--root DIR]");
            return Err(Box::new(crate::Exit(2)));
        }
    };
    let root = Root::find(args.root.as_deref()).map_err(|e| e.to_string())?;
    let run = root.run_dir().map_err(|e| e.to_string())?;
    if let Some(what) = confined(&run) {
        eprintln!("wanderbooks host: refusing to run under {what}");
        return Err(Box::new(crate::Exit(1)));
    }
    let Some(lock) = take_host_lock(&run)? else {
        eprintln!("wanderbooks host: another host has these books open");
        return Err(Box::new(crate::Exit(EXIT_ANOTHER_HOST)));
    };
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let outcome = rt.block_on(Host::new(root, run.clone(), args).serve());
    rt.shutdown_timeout(Duration::from_secs(1));
    let _ = std::fs::remove_file(run.join(lease::LEASE_SOCKET));
    match outcome {
        Ok(Ending::Closed) => {
            drop(lock);
            Ok(())
        }
        Ok(Ending::Restart(update)) => {
            // INVARIANT 7: the new host, then its lock; openers wait on
            // the update lock until the new host answers.
            let started = restart();
            drop(lock);
            if started.is_ok() {
                wait_for_a_host(&run, HOST_LOCK_WAIT);
            }
            drop(update);
            started.map_err(Into::into)
        }
        Ok(Ending::ServerFailed) => {
            drop(lock);
            Err(Box::new(crate::Exit(1)))
        }
        Err(e) => Err(e.into()),
    }
}

/// Start the next host from the file now on disk, with this one's
/// arguments and (already clean) environment (INVARIANT 7).
fn restart() -> Result<(), String> {
    use std::os::unix::process::CommandExt;
    let exe = crate::start::this_executable()?;
    let mut c = std::process::Command::new(exe);
    c.args(std::env::args_os().skip(1))
        .current_dir("/")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null());
    // SAFETY: setsid is async-signal-safe and touches only the child.
    unsafe {
        c.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    c.spawn()
        .map(drop)
        .map_err(|e| format!("can't start the new host: {e}"))
}

/// How the host's work ended.
enum Ending {
    /// Closed (INVARIANTS 4, 6).
    Closed,
    /// Restarting, holding run/update.lock (INVARIANT 7).
    Restart(std::fs::File),
    /// The server stopped on its own (INVARIANT 8).
    ServerFailed,
}

/// What the connections and watchers tell the host.
enum Event {
    /// A connection asks to hold the books.
    Hold {
        info: LeaseInfo,
        to_holder: mpsc::UnboundedSender<HostMsg>,
        answer: oneshot::Sender<Option<u64>>,
    },
    /// A lease's connection ended.
    Released(u64),
    /// The leases held now.
    List(oneshot::Sender<HostMsg>),
    /// An owner intent (passphrase already checked).
    Intent(Intent, oneshot::Sender<HostMsg>),
    /// The server said it is ready.
    ServerReady,
    /// The server exited.
    ServerExited(std::process::ExitStatus),
}

/// A held lease.
struct Held {
    info: LeaseInfo,
    to_holder: mpsc::UnboundedSender<HostMsg>,
}

/// What the host is doing.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Serving (the server ready or starting).
    Open,
    /// Closing for this reason; new leases are turned away.
    Closing(Reason),
}

struct Host {
    root: Root,
    run: PathBuf,
    knobs: Knobs,
    leases: BTreeMap<u64, Held>,
    next_lease: u64,
    ready: bool,
    phase: Phase,
    /// The server's standard input: dropping it closes the books.
    server_stdin: Option<tokio::process::ChildStdin>,
    server_pid: Option<u32>,
    update_lock: Option<std::fs::File>,
    /// Leases ended by the host whose holders haven't let go yet.
    draining: std::collections::BTreeSet<u64>,
    /// When the server is closed even if some haven't.
    drain_until: Option<Instant>,
    /// Whether the root was given on the command line (tests).
    explicit_root: bool,
}

impl Host {
    fn new(root: Root, run: PathBuf, args: Args) -> Self {
        Self {
            root,
            run,
            knobs: args.knobs,
            leases: BTreeMap::new(),
            next_lease: 1,
            ready: false,
            phase: Phase::Open,
            server_stdin: None,
            server_pid: None,
            update_lock: None,
            draining: std::collections::BTreeSet::new(),
            drain_until: None,
            explicit_root: args.root.is_some(),
        }
    }

    /// Bind the lease socket, start the server, and serve leases until
    /// the books close.
    async fn serve(mut self) -> Result<Ending, String> {
        let listener = bind(&self.run)?;
        let (events, mut inbox) = mpsc::unbounded_channel::<Event>();
        let connections = TaskTracker::new();
        let mut signals = Vec::new();
        for kind in [
            tokio::signal::unix::SignalKind::terminate(),
            tokio::signal::unix::SignalKind::interrupt(),
            tokio::signal::unix::SignalKind::hangup(),
        ] {
            signals.push(
                tokio::signal::unix::signal(kind)
                    .map_err(|e| format!("can't watch for signals: {e}"))?,
            );
        }
        let (stop_tx, mut stop) = mpsc::unbounded_channel::<()>();
        for mut s in signals {
            let stop_tx = stop_tx.clone();
            tokio::spawn(async move {
                while s.recv().await.is_some() {
                    let _ = stop_tx.send(());
                }
            });
        }
        let started_as = running_identity();
        let mut exe_poll = tokio::time::interval(self.knobs.exe_poll);
        exe_poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut idle_at = Some(Instant::now() + self.knobs.close_after);
        let server_start = tokio::time::sleep(self.knobs.server_delay);
        tokio::pin!(server_start);
        let mut server_started = false;
        loop {
            let idle = async {
                match idle_at {
                    Some(at) => tokio::time::sleep_until(at).await,
                    None => std::future::pending().await,
                }
            };
            let drained = async {
                match self.drain_until {
                    Some(at) => tokio::time::sleep_until(at).await,
                    None => std::future::pending().await,
                }
            };
            tokio::select! {
                () = &mut server_start, if !server_started => {
                    server_started = true;
                    if matches!(self.phase, Phase::Open)
                        && let Err(why) = self.start_server(events.clone())
                    {
                        // INVARIANT 5: the holders are told why.
                        eprintln!("wanderbooks host: {why}");
                        self.end_leases(Reason::ServerFailed, Some(why));
                        self.phase = Phase::Closing(Reason::ServerFailed);
                        return self.finish(connections, Ending::ServerFailed).await;
                    }
                }
                accepted = listener.accept() => match accepted {
                    Ok((stream, _)) if ledger_mcp::local::peer_is_us(&stream) => {
                        // The holder's process, as the kernel knows it.
                        let pid = ledger_mcp::local::peer_pid(&stream);
                        connections.spawn(connection(stream, pid, events.clone()));
                    }
                    Ok(_) => {}
                    // Out of descriptors and the like: wait, don't spin.
                    Err(_) => tokio::time::sleep(Duration::from_millis(500)).await,
                },
                Some(event) = inbox.recv() => {
                    match event {
                        Event::Hold { info, to_holder, answer } => {
                            if let Phase::Closing(reason) = self.phase {
                                let _ = to_holder.send(HostMsg::Ending {
                                    // Said as it is: a holder that held before
                                    // stays closed after the owner's close.
                                    reason,
                                    detail: None,
                                });
                                let _ = answer.send(None);
                                continue;
                            }
                            let id = self.next_lease;
                            self.next_lease += 1;
                            let _ = to_holder.send(HostMsg::Held { lease: id });
                            if self.ready {
                                let _ = to_holder.send(HostMsg::Ready);
                            }
                            eprintln!("wanderbooks host: lease {id} held by {} (pid {})", info.kind.label(), info.pid);
                            self.leases.insert(id, Held { info: LeaseInfo { id, ..info }, to_holder });
                            idle_at = None;
                            let _ = answer.send(Some(id));
                        }
                        Event::Released(id) => {
                            if self.leases.remove(&id).is_some() {
                                eprintln!("wanderbooks host: lease {id} ended");
                            }
                            if self.draining.remove(&id) && self.draining.is_empty() {
                                self.close_server();
                            }
                            if self.leases.is_empty() && matches!(self.phase, Phase::Open) {
                                idle_at = Some(Instant::now() + self.knobs.close_after);
                            }
                        }
                        Event::List(answer) => {
                            // Closing, there are none to list, and a new
                            // host asking must wait for this one.
                            let _ = answer.send(match self.phase {
                                Phase::Open => HostMsg::Leases {
                                    ready: self.ready,
                                    leases: self.leases.values().map(|h| h.info.clone()).collect(),
                                },
                                Phase::Closing(reason) => HostMsg::Ending { reason, detail: None },
                            });
                        }
                        Event::Intent(Intent::NewBooks { id, currency }, answer)
                            if self.ready && self.phase == Phase::Open =>
                        {
                            // The server does it; the loop goes on meanwhile.
                            let socket = self.run.join(ledger_mcp::local::ADMIN_SOCKET);
                            tokio::spawn(async move {
                                let msg = new_books(socket, id, currency).await;
                                let _ = answer.send(msg);
                            });
                        }
                        Event::Intent(Intent::Close, answer) if self.phase != Phase::Open => {
                            // INVARIANT 11; closing anyway: already so.
                            let _ = answer.send(match self.phase {
                                Phase::Closing(Reason::Restarting) => HostMsg::Refused(
                                    "the books are restarting (a new version): close them again in a moment".into(),
                                ),
                                _ => HostMsg::Done {
                                    detail: Some("the books are already closing".into()),
                                },
                            });
                        }
                        Event::Intent(intent, answer) => {
                            let (msg, closes) = self.intent(intent);
                            let _ = answer.send(msg);
                            if let Some(reason) = closes {
                                self.begin_close(reason);
                                idle_at = None;
                                if self.server_pid.is_none() {
                                    return self.finish(connections, Ending::Closed).await;
                                }
                            }
                        }
                        Event::ServerReady => {
                            self.ready = true;
                            eprintln!("wanderbooks host: the books are open");
                            for held in self.leases.values() {
                                let _ = held.to_holder.send(HostMsg::Ready);
                            }
                        }
                        Event::ServerExited(status) => {
                            self.server_pid = None;
                            self.server_stdin = None;
                            match self.phase {
                                Phase::Closing(Reason::Restarting) => {
                                    let lock = self.update_lock.take().expect("taken before restarting");
                                    return self.finish(connections, Ending::Restart(lock)).await;
                                }
                                Phase::Closing(_) => {
                                    return self.finish(connections, Ending::Closed).await;
                                }
                                Phase::Open => {
                                    let detail = format!("the books server stopped ({status})");
                                    eprintln!("wanderbooks host: {detail}");
                                    self.end_leases(Reason::ServerFailed, Some(detail));
                                    self.phase = Phase::Closing(Reason::ServerFailed);
                                    return self.finish(connections, Ending::ServerFailed).await;
                                }
                            }
                        }
                    }
                }
                () = idle, if matches!(self.phase, Phase::Open) => {
                    eprintln!("wanderbooks host: nothing holds the books: closing them");
                    self.begin_close(Reason::Closing);
                    idle_at = None;
                    if self.server_pid.is_none() {
                        return self.finish(connections, Ending::Closed).await;
                    }
                }
                Some(()) = stop.recv() => {
                    if matches!(self.phase, Phase::Open) {
                        eprintln!("wanderbooks host: told to stop: closing the books");
                        self.begin_close(Reason::ClosedByOwner);
                        idle_at = None;
                        if self.server_pid.is_none() {
                            return self.finish(connections, Ending::Closed).await;
                        }
                    }
                }
                _ = exe_poll.tick(), if matches!(self.phase, Phase::Open) => {
                    if started_as.is_some() && file_identity().is_some_and(|now| Some(now) != started_as) {
                        eprintln!("wanderbooks host: a new version is installed: restarting");
                        // Taken only if free now: the loop never waits on it
                        // (a restart still handing over holds it).
                        let lock = take_update_lock(&self.run);
                        let lock = match lock {
                            Ok(lock) => lock,
                            Err(why) => {
                                // Not restarting, then: the books stay open
                                // on this version, and it tries again.
                                eprintln!("wanderbooks host: can't restart yet: {why}");
                                continue;
                            }
                        };
                        self.update_lock = Some(lock);
                        self.begin_close(Reason::Restarting);
                        idle_at = None;
                        if self.server_pid.is_none() {
                            let lock = self.update_lock.take().expect("just taken");
                            return self.finish(connections, Ending::Restart(lock)).await;
                        }
                    }
                }
                () = drained => self.close_server(),
            }
        }
    }

    /// Start the server, from this host's own executable, watching for
    /// its ready line and its exit (INVARIANT 3).
    fn start_server(&mut self, events: mpsc::UnboundedSender<Event>) -> Result<(), String> {
        let log = open_log(&self.run.join(SERVER_LOG))
            .map_err(|e| format!("can't open the server's log: {}", e.kind()))?;
        let mut command = tokio::process::Command::new(children_executable());
        command.arg0("wanderbooks").arg("server");
        if self.explicit_root {
            command.arg("--root").arg(self.root.path());
        }
        let d = Knobs::default();
        for (flag, now, default) in [
            ("--backup-quiet", self.knobs.backup_quiet, d.backup_quiet),
            ("--drill-first", self.knobs.drill_first, d.drill_first),
        ] {
            if now != default {
                command.arg(flag).arg(now.as_millis().to_string());
            }
        }
        let mut child = command
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(log)
            .spawn()
            .map_err(|e| format!("can't start the books server: {e}"))?;
        self.server_stdin = child.stdin.take();
        self.server_pid = child.id();
        let stdout = child.stdout.take().expect("piped");
        let ready = events.clone();
        tokio::spawn(async move {
            let mut lines = tokio::io::BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if line == ledger_mcp::local::READY {
                    let _ = ready.send(Event::ServerReady);
                }
            }
        });
        tokio::spawn(async move {
            if let Ok(status) = child.wait().await {
                let _ = events.send(Event::ServerExited(status));
            }
        });
        Ok(())
    }

    /// An owner intent: the answer, and whether it closes the books.
    fn intent(&mut self, intent: Intent) -> (HostMsg, Option<Reason>) {
        match intent {
            Intent::Close => (
                HostMsg::Done {
                    detail: Some("the books are closing".into()),
                },
                Some(Reason::ClosedByOwner),
            ),
            Intent::NewBooks { .. } if self.phase == Phase::Open => (
                HostMsg::Refused("the books are still opening: try again in a moment".into()),
                None,
            ),
            Intent::NewBooks { .. } => (
                HostMsg::Refused("the books are closing: nothing was done".into()),
                None,
            ),
            Intent::BackUpNow { .. } => (
                HostMsg::Refused(
                    "the books back themselves up; to back up now, ask your agent (books.back_up_now)"
                        .into(),
                ),
                None,
            ),
            Intent::Restore { .. } => (
                HostMsg::Refused(
                    "restore from a terminal for now: `wanderbooks books restore BACKUP`".into(),
                ),
                None,
            ),
            Intent::ConsentFormatUpgrade { .. } => (
                HostMsg::Refused("no books are waiting for a format upgrade".into()),
                None,
            ),
        }
    }

    /// Tell every lease why it ends, and end it (INVARIANT 5).
    fn end_leases(&mut self, reason: Reason, detail: Option<String>) {
        for (id, held) in std::mem::take(&mut self.leases) {
            self.draining.insert(id);
            let _ = held.to_holder.send(HostMsg::Ending {
                reason,
                detail: detail.clone(),
            });
        }
    }

    /// Start closing: leases told and ended, new ones turned away, the
    /// server's input closed (INVARIANTS 6, 7).
    fn begin_close(&mut self, reason: Reason) {
        self.phase = Phase::Closing(reason);
        self.end_leases(reason, None);
        if self.draining.is_empty() {
            self.close_server();
        } else {
            self.drain_until = Some(Instant::now() + LEASE_DRAIN);
        }
    }

    /// Close the server's input: it closes the books (INVARIANT 3).
    fn close_server(&mut self) {
        self.server_stdin = None;
        self.drain_until = None;
        self.draining.clear();
    }

    /// The last of closing: the connections' last words sent.
    async fn finish(self, connections: TaskTracker, ending: Ending) -> Result<Ending, String> {
        drop(self);
        connections.close();
        let _ = tokio::time::timeout(Duration::from_secs(2), connections.wait()).await;
        Ok(ending)
    }
}

/// "New books…": the server makes them, through its admin socket.
async fn new_books(socket: PathBuf, id: String, currency: String) -> HostMsg {
    use ledger_mcp::admin::{Request, Response};
    let answer = tokio::task::spawn_blocking(move || {
        ledger_mcp::admin::call(&socket, Request::Create { id, currency })
    })
    .await;
    match answer {
        Ok(Ok(Response::Created { id, problem: None })) => HostMsg::Done {
            detail: Some(format!("new books {id} made")),
        },
        Ok(Ok(Response::Created {
            id,
            problem: Some(why),
        })) => HostMsg::Refused(format!("new books {id} were made but didn't open: {why}")),
        Ok(Ok(Response::Refused(why))) => HostMsg::Refused(why),
        Ok(Ok(_)) => HostMsg::Refused("the books server answered oddly".into()),
        Ok(Err(ledger_mcp::admin::CallError::NotSent(why))) => HostMsg::Refused(why),
        // Sent, and no answer: never reported as not done.
        Ok(Err(ledger_mcp::admin::CallError::Unknown(why))) => HostMsg::Uncertain(why),
        Err(_) => HostMsg::Refused("the host hit an internal error".into()),
    }
}

/// Take run/update.lock (exclusive), if it is free now.
fn take_update_lock(run: &Path) -> Result<std::fs::File, String> {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .open(run.join(lease::UPDATE_LOCK))
        .map_err(|e| format!("can't open the update lock: {}", e.kind()))?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(std::fs::TryLockError::WouldBlock) => {
            Err("an earlier restart is still handing over".into())
        }
        Err(std::fs::TryLockError::Error(e)) => {
            Err(format!("can't take the update lock: {}", e.kind()))
        }
    }
}

/// Bind run/lease.sock, replacing a stale one (only the host holding the
/// lock gets here).
fn bind(run: &Path) -> Result<tokio::net::UnixListener, String> {
    let path = run.join(lease::LEASE_SOCKET);
    match std::fs::remove_file(&path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("can't replace the lease socket: {}", e.kind())),
    }
    let short = ledger_store::root::SocketPath::new(run, lease::LEASE_SOCKET)
        .map_err(|e| format!("can't make the lease socket: {e}"))?;
    let listener = tokio::net::UnixListener::bind(short.path())
        .map_err(|e| format!("can't make the lease socket: {}", e.kind()))?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| format!("can't make the lease socket private: {}", e.kind()))?;
    Ok(listener)
}

/// One connection: its first line, then a lease or one answer
/// (lease.rs INVARIANT 1).
async fn connection(
    stream: tokio::net::UnixStream,
    peer_pid: Option<u32>,
    events: mpsc::UnboundedSender<Event>,
) {
    let (read, mut write) = stream.into_split();
    let mut reader = tokio::io::BufReader::new(read.take(lease::MAX_LINE as u64 + 1));
    let mut first = String::new();
    let answer = |msg: HostMsg| lease::line(&msg);
    let hello = match tokio::time::timeout(HELLO_WAIT, reader.read_line(&mut first)).await {
        Ok(Ok(_)) if first.ends_with('\n') => {
            lease::parse_hello(&first).map_err(|e| format!("not understood: {e}"))
        }
        Ok(Ok(_)) => Err("the request is too long, or cut off".into()),
        Ok(Err(e)) => Err(format!("couldn't be read: {}", e.kind())),
        Err(_) => Err("nothing arrived in time".into()),
    };
    let hello = match hello {
        Ok(h) if lease::accepts(h.proto) => h,
        Ok(_) => {
            let _ = write
                .write_all(
                    answer(HostMsg::Refused(
                        "this program is another version of wanderbooks than the books' host: \
                         restart it (for an agent, start a new session)"
                            .into(),
                    ))
                    .as_bytes(),
                )
                .await;
            return;
        }
        Err(why) => {
            let _ = write
                .write_all(answer(HostMsg::Refused(why)).as_bytes())
                .await;
            return;
        }
    };
    match hello.ask {
        Ask::Hold {
            book: Some(book), ..
        } if ledger_store::root::BookId::parse(&book).is_err() => {
            let _ = write
                .write_all(answer(HostMsg::Refused("that isn't a books' id".into())).as_bytes())
                .await;
        }
        Ask::Hold { kind, pid, book } => {
            let (to_holder, mut from_host) = mpsc::unbounded_channel();
            let (tx, rx) = oneshot::channel();
            let info = LeaseInfo {
                id: 0,
                kind,
                pid: peer_pid.unwrap_or(pid),
                book,
            };
            if events
                .send(Event::Hold {
                    info,
                    to_holder,
                    answer: tx,
                })
                .is_err()
            {
                return;
            }
            let id = rx.await.ok().flatten();
            let mut sink = [0u8; 256];
            // Past the hello, what a holder sends is only read to see it
            // go: no limit on it (lease.rs INVARIANT 6 is for the hello).
            let mut inner = reader.into_inner().into_inner();
            let mut ended = false;
            loop {
                tokio::select! {
                    msg = from_host.recv() => match msg {
                        Some(msg) => {
                            let last = matches!(msg, HostMsg::Ending { .. });
                            if write.write_all(answer(msg).as_bytes()).await.is_err() {
                                break;
                            }
                            if last {
                                ended = true;
                                break;
                            }
                        }
                        None => break,
                    },
                    read = inner.read(&mut sink), if id.is_some() => match read {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {}
                    },
                }
            }
            let _ = write.shutdown().await;
            // Told why it ends: the lease lasts until the holder lets go
            // (it has heard, and sends no more), at most LEASE_DRAIN
            // (INVARIANT 6).
            if ended && id.is_some() {
                let _ = tokio::time::timeout(LEASE_DRAIN, async {
                    while let Ok(n) = inner.read(&mut sink).await {
                        if n == 0 {
                            break;
                        }
                    }
                })
                .await;
            }
            if let Some(id) = id {
                let _ = events.send(Event::Released(id));
            }
        }
        Ask::Leases => {
            let (tx, rx) = oneshot::channel();
            if events.send(Event::List(tx)).is_ok()
                && let Ok(msg) = rx.await
            {
                let _ = write.write_all(answer(msg).as_bytes()).await;
            }
        }
        Ask::Intent(intent) => {
            let msg = match intent.check_passphrase() {
                Err(why) => HostMsg::Refused(why),
                Ok(()) => {
                    let (tx, rx) = oneshot::channel();
                    if events.send(Event::Intent(intent, tx)).is_err() {
                        return;
                    }
                    match rx.await {
                        Ok(msg) => msg,
                        Err(_) => return,
                    }
                }
            };
            let _ = write.write_all(answer(msg).as_bytes()).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_logs_limit_is_eight_mebibytes() {
        assert_eq!(LOG_LIMIT, 8_388_608);
    }

    #[test]
    fn a_log_over_its_limit_is_moved_aside_and_one_at_it_kept() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("server.log");
        std::fs::write(&log, "12345").unwrap();
        drop(open_log_within(&log, 5).unwrap());
        assert_eq!(
            std::fs::read_to_string(&log).unwrap(),
            "12345",
            "at the limit: kept"
        );
        assert!(!dir.path().join("server.log.1").exists());
        std::fs::write(&log, "123456").unwrap();
        drop(open_log_within(&log, 5).unwrap());
        assert_eq!(
            std::fs::read_to_string(dir.path().join("server.log.1")).unwrap(),
            "123456"
        );
        assert_eq!(std::fs::read_to_string(&log).unwrap(), "", "a new log");
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&log).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn a_folder_the_host_can_write_isnt_a_sandbox() {
        let seccomp = std::fs::read_to_string("/proc/self/status")
            .unwrap_or_default()
            .lines()
            .find_map(|l| l.strip_prefix("Seccomp:").map(|m| m.trim() != "0"))
            .unwrap_or(false);
        let dir = tempfile::tempdir().unwrap();
        if seccomp {
            // Tests run under a seccomp filter here (some CI containers):
            // the host would say so, whatever the folder.
            assert_eq!(confined(dir.path()), Some("a seccomp filter"));
            return;
        }
        assert_eq!(confined(dir.path()), None);
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            0,
            "its probe went"
        );
        // A folder it may not write: a sandbox (unless this is root, who
        // may write anywhere).
        use std::os::unix::fs::PermissionsExt;
        let locked = dir.path().join("locked");
        std::fs::create_dir(&locked).unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o500)).unwrap();
        // SAFETY: geteuid has no preconditions.
        if unsafe { libc::geteuid() } != 0 {
            assert!(confined(&locked).is_some());
        }
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).unwrap();
        // A folder that isn't there: no sign of a sandbox.
        assert_eq!(confined(&dir.path().join("gone")), None);
    }

    #[test]
    fn only_a_program_file_has_an_identity() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("wanderbooks");
        std::fs::write(&file, "x").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(identity_of(&file), None, "not one anyone may run");
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o700)).unwrap();
        let first = identity_of(&file).expect("a program");
        // Replaced by another file (as an update does): another identity.
        let next = dir.path().join("next");
        std::fs::write(&next, "y").unwrap();
        std::fs::set_permissions(&next, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::rename(&next, &file).unwrap();
        let second = identity_of(&file).unwrap();
        assert_ne!(first, second);
        assert_eq!(identity_of(dir.path()), None, "a folder isn't one");
        assert_eq!(identity_of(&dir.path().join("gone")), None);
    }

    #[test]
    fn a_restart_waits_a_while_for_the_new_host() {
        let dir = tempfile::tempdir().unwrap();
        // Nobody answering: waited for, the whole while, then not found.
        let started = std::time::Instant::now();
        assert!(!wait_for_a_host(dir.path(), Duration::from_millis(400)));
        assert!(started.elapsed() >= Duration::from_millis(400));
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn the_host_lock_is_waited_for_then_taken_or_given_up() {
        let dir = tempfile::tempdir().unwrap();
        let run = dir.path();
        // Free: taken at once.
        let first = take_host_lock_within(run, Duration::from_millis(100))
            .unwrap()
            .expect("a free lock is taken");
        // Held, and nobody answering on the lease socket: waited for until
        // the time is up, then not taken.
        let started = std::time::Instant::now();
        assert!(
            take_host_lock_within(run, Duration::from_millis(400))
                .unwrap()
                .is_none()
        );
        assert!(started.elapsed() >= Duration::from_millis(400), "it waited");
        assert!(started.elapsed() < Duration::from_secs(5), "and gave up");
        // Let go while another waits: that one takes it.
        let releaser = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            drop(first);
        });
        let second = take_host_lock_within(run, Duration::from_secs(5)).unwrap();
        assert!(second.is_some(), "taken once let go");
        releaser.join().unwrap();
    }
}
