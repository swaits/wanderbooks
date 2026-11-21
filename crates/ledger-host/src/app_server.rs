//! The app's books server: a child process of the window (plan P15 3.1).
//!
//! ```text
//! GOAL: the desktop app is the host. It starts the books server from its
//!       own executable when the window starts and closes it by closing
//!       the server's input when the window closes; a window that dies
//!       any way at all leaves no server behind and no book half-closed;
//!       and no window ever waits forever on a server that will not exit.
//! SOURCE: plan/p15-app-and-relay.md 3.1 (the app starts the server as a
//!         child, from its own executable, and closes it by closing its
//!         input, exactly as the host does), D-77 (the app is the host),
//!         D-84 (a child and not a thread: a crash in the window or its
//!         graphics driver closes the server's input, and the books
//!         close cleanly), WB-app-1, WB-app-2, WB-app-4; host.rs
//!         INVARIANTS 3 and 10 (the host's own way: close the input,
//!         never kill).
//! INVARIANTS:
//!   1. A start is refused, in words, when another program has these
//!      books open: a host holds `run/host.lock`, or a server holds
//!      `run/server.lock` (one still finishing after its window went
//!      included). The locks are the existing ones; nothing is added to
//!      them. Nothing is touched by a refused start.
//!   2. The server is this program's own executable, started as
//!      `wanderbooks server --open IDS` (the companies the window
//!      restored as tabs; `none` when there are none), its input a pipe
//!      this struct holds, its output read for the ready line, its error
//!      output the root's `run/server.log`. On Windows it has no console
//!      window of its own.
//!   3. The server's input is closed when [`AppServer::stop`] is called,
//!      when the value is dropped, and when this process ends for any
//!      reason (the system closes the pipe): the server then closes as
//!      local.rs INVARIANT 6 says. Nothing here sends it a signal or
//!      kills it, ever.
//!   4. [`AppServer::stop`] waits for the exit at most as long as it is
//!      told and then says the server is still closing: the window can
//!      end while the server finishes its backup alone, and the next
//!      start is refused, in words (1), until it has.
//!   5. A server that exits on its own is known as soon as it has
//!      (`exited`), and every call that needs it says so, in words.
//!   6. A book is opened or closed in the server only by an admin
//!      request, waiting for the server to be ready first; a close that
//!      says its last backup was not whole is returned as a notice, never
//!      as a plain success.
//!   7. Starting writes `run/app-hosted`, a file with nothing in it: the
//!      root is this app's from then on, and the agents' bridge says
//!      "Wanderbooks isn't running" for it, instead of starting a host
//!      (bridge.rs INVARIANT 12).
//! ```

use crate::host::{SERVER_LOG, children_executable, open_log, private_options};
use crate::opener::{host_lock_held, server_lock_held};
use ledger_mcp::admin::{self, CloseOutcome, OpenOutcome, Request, Response};
use ledger_store::root::Root;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// The file in `run/` that says the root is the desktop app's
/// (INVARIANT 7).
pub const APP_MARKER: &str = "app-hosted";

/// How long a server may take to say it is ready: it opens and verifies
/// every company it was told to, and a large one rebuilding a drifted
/// database takes a while.
pub const READY_WAIT: Duration = Duration::from_secs(300);

/// How long [`AppServer::stop`] callers wait for a close, by default: the
/// last backup of a company is allowed 60 s (local.rs `BACKUP_AT_CLOSE`),
/// and running calls finish first.
pub const STOP_WAIT: Duration = Duration::from_secs(90);

/// What a refused start says (INVARIANT 1).
pub const ANOTHER_HAS_THEM: &str = "Another program has these books open: another Wanderbooks window, `wanderbooks serve`, or a books server that is still finishing its last backup. Close it (`wanderbooks close` closes the books) or wait a moment, then start Wanderbooks again.";

/// Where the server is in its life.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Phase {
    /// Started, not yet ready.
    Starting,
    /// Ready: its sockets are bound and the companies tried.
    Ready,
    /// Gone, and how (in words).
    Exited(String),
}

struct Shared {
    phase: Mutex<Phase>,
    changed: Condvar,
}

impl Shared {
    fn phase(&self) -> std::sync::MutexGuard<'_, Phase> {
        self.phase.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn set(&self, to: Phase) {
        let mut phase = self.phase();
        // Exited is final; Ready only follows Starting.
        match (&*phase, &to) {
            (Phase::Exited(_), _) | (Phase::Ready, Phase::Starting) => {}
            _ => *phase = to,
        }
        self.changed.notify_all();
    }
}

/// What [`AppServer::stop`] came to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stopped {
    /// The server has exited: every company is closed and let go.
    Closed,
    /// The server was told to close and has not exited within the wait;
    /// it goes on closing by itself (INVARIANT 4).
    StillClosing,
}

/// The books server, a child process of this one.
pub struct AppServer {
    run: PathBuf,
    /// The server's input: dropping it closes the books (INVARIANT 3).
    stdin: Mutex<Option<std::process::ChildStdin>>,
    shared: Arc<Shared>,
    pid: u32,
}

/// Whether the root whose `run/` folder this is belongs to the desktop
/// app (INVARIANT 7).
pub fn app_hosted(run: &Path) -> bool {
    run.join(APP_MARKER).is_file()
}

/// Whether a books server runs for the app now: the root is the app's,
/// no host holds it, and a server does.
pub fn the_apps_server_runs(run: &Path) -> bool {
    app_hosted(run) && !host_lock_held(run) && server_lock_held(run)
}

impl AppServer {
    /// Start the server of the data root (`root`, else this account's),
    /// from this program's own executable, with the companies `open` open
    /// (INVARIANTS 1, 2). It returns at once; [`AppServer::wait_ready`]
    /// waits for the ready line.
    pub fn start(root: Option<&Path>, open: &[String]) -> Result<Self, String> {
        Self::start_in(&children_executable(), root, open, |_| {})
    }

    /// [`AppServer::start`] from the program `exe`, `configure` having the
    /// last word on its command (tests give it an environment of its own).
    pub fn start_in(
        exe: &Path,
        root: Option<&Path>,
        open: &[String],
        configure: impl FnOnce(&mut Command),
    ) -> Result<Self, String> {
        let found = Root::find(root).map_err(|e| e.to_string())?;
        let run = found.run_dir().map_err(|e| e.to_string())?;
        if host_lock_held(&run) || server_lock_held(&run) {
            return Err(ANOTHER_HAS_THEM.into());
        }
        private_options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(run.join(APP_MARKER))
            .map_err(|e| format!("can't mark these books as the app's: {}", e.kind()))?;
        let log = open_log(&run.join(SERVER_LOG))
            .map_err(|e| format!("can't open the server's log: {}", e.kind()))?;
        let mut command = Command::new(exe);
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.arg0("wanderbooks");
        }
        command.arg("server");
        if let Some(root) = root {
            command.arg("--root").arg(root);
        }
        command.arg("--open").arg(open_argument(open));
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // CREATE_NO_WINDOW: a console of its own that nobody sees, so
            // the window's own Ctrl+C never reaches it (it closes by its
            // input alone).
            command.creation_flags(0x0800_0000);
        }
        configure(&mut command);
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(log)
            .spawn()
            .map_err(|e| format!("can't start the books server: {e}"))?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let pid = child.id();
        let shared = Arc::new(Shared {
            phase: Mutex::new(Phase::Starting),
            changed: Condvar::new(),
        });
        if let Some(stdout) = stdout {
            let shared = shared.clone();
            std::thread::Builder::new()
                .name("app-server-ready".into())
                .spawn(move || {
                    // Read to the end, whatever it says: a pipe nobody
                    // reads could stop the server one day.
                    for line in std::io::BufReader::new(stdout).lines() {
                        match line {
                            Ok(line) if line == ledger_mcp::local::READY => {
                                shared.set(Phase::Ready);
                            }
                            Ok(_) => {}
                            Err(_) => break,
                        }
                    }
                })
                .map_err(|e| format!("can't watch the books server: {e}"))?;
        }
        {
            let shared = shared.clone();
            std::thread::Builder::new()
                .name("app-server-exit".into())
                .spawn(move || {
                    let said = match child.wait() {
                        Ok(status) => format!("the books server stopped ({status})"),
                        Err(e) => format!("the books server stopped ({})", e.kind()),
                    };
                    shared.set(Phase::Exited(said));
                })
                .map_err(|e| format!("can't watch the books server: {e}"))?;
        }
        Ok(Self {
            run,
            stdin: Mutex::new(stdin),
            shared,
            pid,
        })
    }

    /// The server's process id (tests; never signalled by this program).
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// The root's `run/` folder.
    pub fn run_dir(&self) -> &Path {
        &self.run
    }

    /// Why the server is gone, if it is (INVARIANT 5).
    pub fn exited(&self) -> Option<String> {
        match &*self.shared.phase() {
            Phase::Exited(why) => Some(why.clone()),
            _ => None,
        }
    }

    /// Wait at most `within` for the server to be ready: Ok when it is,
    /// else why not, in words (it exited, or is still opening).
    pub fn wait_ready(&self, within: Duration) -> Result<(), String> {
        let deadline = Instant::now() + within;
        let mut phase = self.shared.phase();
        loop {
            match &*phase {
                Phase::Ready => return Ok(()),
                Phase::Exited(why) => {
                    return Err(format!(
                        "{why}: its log is {} in the books' run folder",
                        SERVER_LOG
                    ));
                }
                Phase::Starting => {}
            }
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                return Err(format!(
                    "the books server is still opening the companies after {} s",
                    within.as_secs()
                ));
            };
            phase = self
                .shared
                .changed
                .wait_timeout(phase, left)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }

    /// Open the company `id` in the server, so assistants can reach it
    /// (INVARIANT 6): `Ok(None)` when it is open now, `Err` with why not.
    /// It waits for the server to be ready, and for the open.
    pub fn open_book(&self, id: &str) -> Result<Option<String>, String> {
        self.wait_ready(READY_WAIT)?;
        match self.ask(Request::OpenBook { id: id.into() })? {
            Response::OpenBook { outcome, .. } => match outcome {
                OpenOutcome::Opened | OpenOutcome::AlreadyOpen => Ok(None),
                OpenOutcome::NoSuchBook => Err(format!(
                    "there are no books called {id:?}, so assistants can't reach them"
                )),
                OpenOutcome::CouldNotOpen(why) => Err(format!(
                    "{id} couldn't be opened for your assistants: {why}. You can still read it in this window."
                )),
            },
            Response::Refused(why) => Err(why),
            other => Err(format!("the books server answered oddly: {other:?}")),
        }
    }

    /// Close the company `id` in the server (INVARIANT 6): running calls
    /// on it finish, its assistants' sessions end, its last backup is made
    /// and it is let go. `Ok(None)` when that is all so; `Ok(Some(words))`
    /// when it is closed but its last backup wasn't whole; `Err` when the
    /// request itself failed (the company may still be open).
    pub fn close_book(&self, id: &str) -> Result<Option<String>, String> {
        self.wait_ready(READY_WAIT)?;
        match self.ask(Request::CloseBook { id: id.into() })? {
            Response::CloseBook { outcome, .. } => match outcome {
                CloseOutcome::Closed | CloseOutcome::WasNotOpen | CloseOutcome::NoSuchBook => {
                    Ok(None)
                }
                CloseOutcome::ClosedBackupIncomplete(why) => Ok(Some(format!(
                    "{id} is closed to your assistants, but its last backup wasn't whole: {why}. What it holds since the last good backup isn't backed up."
                ))),
            },
            Response::Refused(why) => Err(why),
            other => Err(format!("the books server answered oddly: {other:?}")),
        }
    }

    /// One admin request, answered or why not in words.
    fn ask(&self, request: Request) -> Result<Response, String> {
        if let Some(why) = self.exited() {
            return Err(why);
        }
        admin::call(&self.run.join(ledger_mcp::local::ADMIN_SOCKET), request)
            .map_err(|e| e.to_string())
    }

    /// Close the server's input: it begins to close the books and exits
    /// when it is done (INVARIANT 3). Does not wait.
    pub fn close_input(&self) {
        *self.stdin.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// Close the server's input and wait at most `within` for it to exit
    /// (INVARIANTS 3, 4). Never kills it.
    pub fn stop(&self, within: Duration) -> Stopped {
        self.close_input();
        let deadline = Instant::now() + within;
        let mut phase = self.shared.phase();
        loop {
            if matches!(&*phase, Phase::Exited(_)) {
                return Stopped::Closed;
            }
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                return Stopped::StillClosing;
            };
            phase = self
                .shared
                .changed
                .wait_timeout(phase, left)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }
}

/// `--open`'s value for these companies.
fn open_argument(open: &[String]) -> String {
    if open.is_empty() {
        "none".into()
    } else {
        open.join(",")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_company_is_none_and_the_rest_are_a_list() {
        assert_eq!(open_argument(&[]), "none");
        assert_eq!(open_argument(&["acme".into()]), "acme");
        assert_eq!(open_argument(&["acme".into(), "beta".into()]), "acme,beta");
    }

    #[test]
    fn a_server_that_has_exited_stays_exited() {
        let shared = Shared {
            phase: Mutex::new(Phase::Starting),
            changed: Condvar::new(),
        };
        shared.set(Phase::Ready);
        assert_eq!(*shared.phase(), Phase::Ready);
        shared.set(Phase::Starting);
        assert_eq!(*shared.phase(), Phase::Ready, "ready doesn't go back");
        shared.set(Phase::Exited("gone".into()));
        shared.set(Phase::Ready);
        assert_eq!(*shared.phase(), Phase::Exited("gone".into()));
    }

    #[test]
    fn a_root_is_the_apps_only_once_marked() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!app_hosted(dir.path()));
        std::fs::write(dir.path().join(APP_MARKER), b"").unwrap();
        assert!(app_hosted(dir.path()));
        // No server runs there: not the app's server.
        assert!(!the_apps_server_runs(dir.path()));
    }
}
