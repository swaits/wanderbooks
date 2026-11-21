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
//!      (bridge.rs INVARIANT 12). The marker is never removed: a root the
//!      app has served is the app's for good.
//!   8. A server the window started is either managed by the window or
//!      stopped, never neither ([`AppHost`]): one that is alive and not
//!      yet ready is waited for, not given up on; a start that fails, or
//!      a window closed before the start has stored the server, closes
//!      that server's input and waits for it, bounded.
//!   9. The window's close closes each open company first, by an admin
//!      request, so a closing backup that was not whole is returned in
//!      words, and a server that exits badly is returned in words too
//!      ([`AppHost::stop`]): a close is never silent about either. What
//!      the server does for the headless path is not changed.
//!  10. The format upgrade of a company, under the app, goes to the
//!      server as an admin request (`UpgradeFormat`), the way the host
//!      hands it on; there is no host and no lease socket to ask.
//!  11. After the server is ready, the companies the window wanted open
//!      are checked against the server's list: one that is not open is
//!      returned in words ([`AppHost::not_open`]).
//!  12. Who is connected, and the tokens there are, are read with two
//!      quick admin requests (`Sessions`, `TokenList`) that wait for
//!      neither the server's start nor an answer for long
//!      ([`AppHost::roster`]): a server that is starting, gone or slow
//!      is an `Err` in words, never a wait; a token is revoked by the
//!      request `wanderbooks token revoke` makes, and its copy in the
//!      token file goes with it ([`AppHost::revoke_token`]). Neither ever
//!      holds a token's text.
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
    /// Gone: how (in words), and whether it was a clean exit.
    Exited(String, bool),
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
            (Phase::Exited(..), _) | (Phase::Ready, Phase::Starting) => {}
            _ => *phase = to,
        }
        self.changed.notify_all();
    }
}

/// What [`AppServer::stop`] came to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stopped {
    /// The server has exited cleanly: every company is closed and let go.
    Closed,
    /// The server has exited, and not cleanly (a nonzero status, or a
    /// signal): how, in words. Its books may not have been closed whole.
    Failed(String),
    /// The server was told to close and has not exited within the wait;
    /// it goes on closing by itself (INVARIANT 4).
    StillClosing,
}

/// Who is connected and which tokens are live, as the server says
/// (INVARIANT 12).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Roster {
    /// For each open company, its live sessions.
    pub sessions: Vec<admin::BookSessions>,
    /// Every live token.
    pub tokens: Vec<admin::TokenInfo>,
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
                    let (said, clean) = match child.wait() {
                        Ok(status) => (
                            format!("the books server stopped ({status})"),
                            status.success(),
                        ),
                        Err(e) => (format!("the books server stopped ({})", e.kind()), false),
                    };
                    shared.set(Phase::Exited(said, clean));
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
            Phase::Exited(why, _) => Some(why.clone()),
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
                Phase::Exited(why, _) => {
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

    /// Whether the server has said it is ready (and is still there).
    pub fn is_ready(&self) -> bool {
        matches!(&*self.shared.phase(), Phase::Ready)
    }

    /// Wait for the server to be ready for as long as it lives: a server
    /// still opening companies, or waiting for another server to finish
    /// (local.rs INVARIANT 1), is waited for and not given up on; `Err`
    /// only when it has exited (INVARIANT 8). `slice` is how long each
    /// wait runs before the server is looked at again.
    pub fn wait_started(&self, slice: Duration) -> Result<(), String> {
        loop {
            match self.wait_ready(slice) {
                Ok(()) => return Ok(()),
                Err(_) if self.exited().is_none() => {}
                Err(why) => return Err(why),
            }
        }
    }

    /// Open the company `id` in the server, so assistants can reach it
    /// (INVARIANT 6): `Ok(None)` when it is open now, `Err` with why not.
    /// It waits for the server to be ready, and for the open.
    pub fn open_book(&self, id: &str) -> Result<Option<String>, String> {
        self.wait_ready(READY_WAIT)?;
        match self.ask(Request::OpenBook { id: id.into() }, None)? {
            Response::OpenBook { outcome, .. } => open_words(id, outcome),
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
        self.close_book_within(id, None)
    }

    /// [`AppServer::close_book`], waiting at most `wait` for the answer.
    pub fn close_book_within(
        &self,
        id: &str,
        wait: Option<Duration>,
    ) -> Result<Option<String>, String> {
        self.wait_ready(READY_WAIT)?;
        match self.ask(Request::CloseBook { id: id.into() }, wait)? {
            Response::CloseBook { outcome, .. } => Ok(close_words(id, outcome)),
            Response::Refused(why) => Err(why),
            other => Err(format!("the books server answered oddly: {other:?}")),
        }
    }

    /// The books the server holds, and whether each is open (INVARIANT 9).
    pub fn books(&self, wait: Option<Duration>) -> Result<Vec<admin::BookStatus>, String> {
        self.wait_ready(READY_WAIT)?;
        match self.ask(Request::List, wait)? {
            Response::Books(books) => Ok(books),
            Response::Refused(why) => Err(why),
            other => Err(format!("the books server answered oddly: {other:?}")),
        }
    }

    /// Who is connected to each open company, and every live token
    /// (INVARIANT 12). It does not wait for the server to be ready: a
    /// server still starting is an `Err`, and so is one that does not
    /// answer within `wait`.
    pub fn roster(&self, wait: Duration) -> Result<Roster, String> {
        if !self.is_ready() {
            return Err(match self.exited() {
                Some(why) => why,
                None => "the books server is still starting".into(),
            });
        }
        let sessions = match self.ask(Request::Sessions, Some(wait))? {
            Response::Sessions(open) => open,
            Response::Refused(why) => return Err(why),
            other => return Err(format!("the books server answered oddly: {other:?}")),
        };
        let tokens = match self.ask(Request::TokenList, Some(wait))? {
            Response::Tokens(tokens) => tokens,
            Response::Refused(why) => return Err(why),
            other => return Err(format!("the books server answered oddly: {other:?}")),
        };
        Ok(Roster { sessions, tokens })
    }

    /// End the key `id`, named `name`, of the company `book`, as
    /// `wanderbooks token revoke` does (INVARIANT 12): the server ends that
    /// key and the sessions it opened, and the token file's copy is
    /// removed. It goes by the key's id, never its name alone: a row that
    /// shows a key since replaced must not end the new one. `Ok` says what
    /// was done, in words. When the key is ended but the token file's copy
    /// cannot be deleted, that is an `Err` that says so (the key is dead; a
    /// file holding its text remains at that path), never its text.
    pub fn revoke_token(&self, book: &str, name: &str, id: &str) -> Result<String, String> {
        self.wait_ready(READY_WAIT)?;
        let request = Request::TokenRevoke {
            book: book.into(),
            name: name.into(),
            id: Some(id.into()),
        };
        match self.ask_raw(request, Some(Duration::from_secs(30))) {
            Ok(Response::TokenRevoked(t)) => {
                let said = format!("Ended {:?}'s access to {}.", t.name, t.book);
                match crate::commands::forget_token_file(&self.run, book, name) {
                    Ok(()) => Ok(said),
                    Err(why) => {
                        let at = crate::commands::token_file_path(&self.run, book, name);
                        Err(format!(
                            "{said} The key is dead and opens nothing now. But the copy of its text kept on this computer could not be deleted ({why}); a file with its text remains at {}. Delete that file yourself.",
                            at.display()
                        ))
                    }
                }
            }
            Ok(Response::Refused(why)) => {
                // Refused because that key is gone (replaced or revoked
                // since the row was read), or for another reason.
                let gone = matches!(
                    self.ask_raw(Request::TokenList, Some(Duration::from_secs(10))),
                    Ok(Response::Tokens(list)) if !list.iter().any(|t| t.id == id)
                );
                Err(if gone {
                    format!(
                        "This key was already replaced or revoked, so nothing was ended. The list now shows the keys of {book} as they stand."
                    )
                } else {
                    why
                })
            }
            Ok(other) => Err(format!("the books server answered oddly: {other:?}")),
            Err(e) => Err(e.to_string()),
        }
    }

    /// The owner's go-ahead for the format upgrade of the company `id`,
    /// with the owner passphrase they typed, sent to the server as the
    /// host sends it (INVARIANT 10): what came of it, a line each; or why
    /// not, which says whether it may have been done all the same.
    pub fn upgrade_format(&self, id: &str, passphrase: &str) -> Result<Vec<String>, String> {
        self.wait_ready(READY_WAIT)
            .map_err(|why| format!("not upgraded: {why}"))?;
        let request = Request::UpgradeFormat {
            id: id.into(),
            passphrase: admin::Passphrase(passphrase.into()),
        };
        match self.ask_raw(request, None) {
            Ok(Response::FormatUpgraded(u)) => Ok(crate::commands::upgraded_lines(&u)),
            Ok(Response::Refused(why)) => Err(format!("not upgraded: {why}")),
            Ok(_) => Err(
                "the books server answered oddly, so it may or may not have been done: `wanderbooks books list` says whether the books still wait for it"
                    .into(),
            ),
            Err(admin::CallError::NotSent(why)) => Err(format!("not upgraded: {why}")),
            Err(admin::CallError::Unknown(why)) => Err(why),
        }
    }

    /// One admin request, answered or why not in words.
    fn ask(&self, request: Request, wait: Option<Duration>) -> Result<Response, String> {
        self.ask_raw(request, wait).map_err(|e| e.to_string())
    }

    /// One admin request, as the admin socket answers it.
    fn ask_raw(
        &self,
        request: Request,
        wait: Option<Duration>,
    ) -> Result<Response, admin::CallError> {
        if let Some(why) = self.exited() {
            return Err(admin::CallError::NotSent(why));
        }
        admin::call_within(
            &self.run.join(ledger_mcp::local::ADMIN_SOCKET),
            request,
            wait,
        )
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
            if let Phase::Exited(how, clean) = &*phase {
                return if *clean {
                    Stopped::Closed
                } else {
                    Stopped::Failed(how.clone())
                };
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

/// How long a server whose start has failed is waited for to close
/// (INVARIANT 8): it exits as soon as its input ends if it was waiting
/// or opening, and makes a last backup of what it had open.
pub const FAILED_START_STOP_WAIT: Duration = Duration::from_secs(30);

/// What it says when the server is still closing after a stop ran out of
/// time (INVARIANT 4).
pub const STILL_CLOSING: &str = "The books are still closing in the background (most likely a last backup): Wanderbooks finishes it on its own. Start it again once it has.";

type Starter = Box<dyn Fn(&[String]) -> Result<AppServer, String> + Send + Sync>;

/// What the window does with its books server, whole: start it, open and
/// close its companies, ask which are open, hand it the format upgrade,
/// stop it. The program's owner (`ViewerOwner` in ledger-cli) only
/// forwards to this, so what the tests drive is what the window runs.
pub struct AppHost {
    starter: Starter,
    slice: Duration,
    server: Mutex<Option<Arc<AppServer>>>,
    /// The window has asked to close (INVARIANT 8): no server is started
    /// after it, and one stored just before it is stopped.
    stopped: std::sync::atomic::AtomicBool,
}

impl AppHost {
    /// A host that starts the data root's server (`root`, else this
    /// account's) from this program's own executable.
    pub fn new(root: Option<PathBuf>) -> Self {
        Self::with_starter(READY_WAIT, move |open| {
            AppServer::start(root.as_deref(), open)
        })
    }

    /// A host that starts its server with `starter` (tests give it
    /// another program and an environment of its own); `slice` is how
    /// long a wait for the ready line runs before the server is looked
    /// at again.
    pub fn with_starter(
        slice: Duration,
        starter: impl Fn(&[String]) -> Result<AppServer, String> + Send + Sync + 'static,
    ) -> Self {
        Self {
            starter: Box::new(starter),
            slice,
            server: Mutex::new(None),
            stopped: std::sync::atomic::AtomicBool::new(false),
        }
    }

    fn slot(&self) -> std::sync::MutexGuard<'_, Option<Arc<AppServer>>> {
        self.server.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn closing(&self) -> bool {
        self.stopped.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// The running server, or why there is none.
    pub fn running(&self) -> Result<Arc<AppServer>, String> {
        self.slot()
            .clone()
            .ok_or_else(|| "the books server isn't running".to_string())
    }

    /// Start the server with the companies `open` open and return when it
    /// is ready, or why it will not be (INVARIANT 8). Blocks. Called again
    /// after a start that failed, it starts a new server; called while one
    /// runs it waits for that one.
    pub fn start(&self, open: &[String]) -> Result<(), String> {
        if self.closing() {
            return Err("Wanderbooks is closing".into());
        }
        let alive = self.slot().clone().filter(|s| s.exited().is_none());
        let server = match alive {
            Some(server) => server,
            None => {
                let server = Arc::new((self.starter)(open)?);
                let mut slot = self.slot();
                // The window may have asked to close since the check
                // above: it looks at the slot after setting the flag, so
                // one of the two sees the other.
                if self.closing() {
                    drop(slot);
                    server.stop(FAILED_START_STOP_WAIT);
                    return Err("Wanderbooks is closing".into());
                }
                *slot = Some(server.clone());
                server
            }
        };
        match server.wait_started(self.slice) {
            Ok(()) => Ok(()),
            Err(why) => {
                // Managed or stopped, never neither: this server is let
                // go of, and its input closed.
                {
                    let mut slot = self.slot();
                    if slot.as_ref().is_some_and(|s| Arc::ptr_eq(s, &server)) {
                        *slot = None;
                    }
                }
                Err(match server.stop(FAILED_START_STOP_WAIT) {
                    Stopped::StillClosing => {
                        format!("{why}; it is still closing in the background")
                    }
                    _ => why,
                })
            }
        }
    }

    /// Open the company `id` in the server (a tab opened).
    pub fn open_book(&self, id: &str) -> Result<Option<String>, String> {
        self.running()?.open_book(id)
    }

    /// Close the company `id` in the server (a tab closed).
    pub fn close_book(&self, id: &str) -> Result<Option<String>, String> {
        self.running()?.close_book(id)
    }

    /// Why the server is gone, if it exited on its own.
    pub fn exited(&self) -> Option<String> {
        self.slot().as_ref()?.exited()
    }

    /// For each of the companies `wanted` that the server does not have
    /// open (INVARIANT 11): its id, and why in words. Nothing is said for
    /// a server that isn't running.
    pub fn not_open(&self, wanted: &[String]) -> Vec<(String, String)> {
        let Ok(server) = self.running() else {
            return Vec::new();
        };
        let books = match server.books(Some(Duration::from_secs(30))) {
            Ok(books) => books,
            Err(why) => {
                return wanted
                    .iter()
                    .map(|id| {
                        (
                            id.clone(),
                            format!("can't tell whether {id} is open for your assistants: {why}"),
                        )
                    })
                    .collect();
            }
        };
        wanted
            .iter()
            .filter_map(|id| {
                let held = books.iter().find(|b| &b.id == id);
                let not_open = match held {
                    Some(b) if b.open => return None,
                    Some(b) => b
                        .problem
                        .clone()
                        .unwrap_or_else(|| "it isn't open in the books server".into()),
                    None => "the books server doesn't have it".into(),
                };
                let words = open_words(id, OpenOutcome::CouldNotOpen(not_open))
                    .err()
                    .unwrap_or_default();
                Some((id.clone(), words))
            })
            .collect()
    }

    /// Who is connected, and which tokens are live (INVARIANT 12): quick,
    /// for a server that is not running or not ready an `Err`.
    pub fn roster(&self, wait: Duration) -> Result<Roster, String> {
        self.running()?.roster(wait)
    }

    /// End the key `id` (INVARIANT 12).
    pub fn revoke_token(&self, book: &str, name: &str, id: &str) -> Result<String, String> {
        self.running()?.revoke_token(book, name, id)
    }

    /// The format upgrade of the company `id` (INVARIANT 10).
    pub fn upgrade_format(&self, id: &str, passphrase: &str) -> Result<Vec<String>, String> {
        match self.running() {
            Ok(server) => server.upgrade_format(id, passphrase),
            Err(_) => Err(
                "not upgraded: the books server isn't running, so nothing was done. Assistants can't reach your companies either: try again from the notice at the top of the window, or restart Wanderbooks"
                    .into(),
            ),
        }
    }

    /// Close the window's books (INVARIANT 9): each open company is closed
    /// first, so its closing backup is reported if it was not whole; then
    /// the server's input is closed and its exit waited for, the whole
    /// taking about `within`. `Ok(None)` when all went whole; else the
    /// words, a line each (the companies whose backup was incomplete, a
    /// server that exited badly or is still closing). Never kills.
    pub fn stop(&self, within: Duration) -> Result<Option<String>, String> {
        self.stopped
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let Some(server) = self.slot().take() else {
            return Ok(None);
        };
        let begun = Instant::now();
        let mut said: Vec<String> = Vec::new();
        if server.is_ready() && server.exited().is_none() {
            match server.books(Some(within)) {
                Ok(books) => {
                    let open: Vec<String> =
                        books.into_iter().filter(|b| b.open).map(|b| b.id).collect();
                    // Each company's last backup runs on its own: none
                    // waits for another's.
                    let notices: Vec<Option<String>> = std::thread::scope(|scope| {
                        let handles: Vec<_> = open
                            .iter()
                            .map(|id| {
                                let server = &server;
                                scope.spawn(move || {
                                    match server.close_book_within(id, Some(within)) {
                                        Ok(notice) => notice,
                                        Err(why) => {
                                            Some(format!("{id} could not be closed cleanly: {why}"))
                                        }
                                    }
                                })
                            })
                            .collect();
                        handles
                            .into_iter()
                            .map(|h| h.join().unwrap_or(None))
                            .collect()
                    });
                    said.extend(notices.into_iter().flatten());
                }
                Err(why) => said.push(format!(
                    "Couldn't check that each company closed whole: {why}"
                )),
            }
        }
        let left = within
            .saturating_sub(begun.elapsed())
            .max(Duration::from_secs(5));
        match server.stop(left) {
            Stopped::Closed => {}
            Stopped::Failed(how) => said.push(format!(
                "The books server ended badly ({how}). Its log is {SERVER_LOG} in the books' run folder; `wanderbooks doctor` says whether the books are whole."
            )),
            Stopped::StillClosing => said.push(STILL_CLOSING.into()),
        }
        Ok(if said.is_empty() {
            None
        } else {
            Some(said.join("\n"))
        })
    }
}

/// What opening the company `id` came to, for the person: `Ok(None)` when
/// it is open, else why not (INVARIANT 6).
fn open_words(id: &str, outcome: OpenOutcome) -> Result<Option<String>, String> {
    match outcome {
        OpenOutcome::Opened | OpenOutcome::AlreadyOpen => Ok(None),
        OpenOutcome::NoSuchBook => Err(format!(
            "there are no books called {id:?}, so assistants can't reach them"
        )),
        OpenOutcome::CouldNotOpen(why) => Err(format!(
            "{id} couldn't be opened for your assistants: {why}. You can still read it in this window."
        )),
    }
}

/// What closing the company `id` came to: a notice when its last backup
/// wasn't whole, never a plain success then (INVARIANT 6).
fn close_words(id: &str, outcome: CloseOutcome) -> Option<String> {
    match outcome {
        CloseOutcome::Closed | CloseOutcome::WasNotOpen | CloseOutcome::NoSuchBook => None,
        CloseOutcome::ClosedBackupIncomplete(why) => Some(format!(
            "{id} is closed to your assistants, but its last backup wasn't whole: {why}. What it holds since the last good backup isn't backed up."
        )),
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
    fn a_close_with_an_incomplete_backup_is_a_notice_never_a_plain_success() {
        assert_eq!(close_words("acme", CloseOutcome::Closed), None);
        assert_eq!(close_words("acme", CloseOutcome::WasNotOpen), None);
        let said = close_words(
            "acme",
            CloseOutcome::ClosedBackupIncomplete("the backup folder is gone".into()),
        )
        .expect("a notice");
        assert!(
            said.contains("acme")
                && said.contains("last backup wasn't whole")
                && said.contains("the backup folder is gone"),
            "{said}"
        );
    }

    #[test]
    fn an_open_that_did_not_happen_says_why() {
        assert_eq!(open_words("acme", OpenOutcome::Opened), Ok(None));
        assert_eq!(open_words("acme", OpenOutcome::AlreadyOpen), Ok(None));
        let refused = open_words("acme", OpenOutcome::CouldNotOpen("a damaged line".into()))
            .expect_err("not opened");
        assert!(
            refused.contains("acme") && refused.contains("a damaged line"),
            "{refused}"
        );
        assert!(open_words("acme", OpenOutcome::NoSuchBook).is_err());
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
        shared.set(Phase::Exited("gone".into(), true));
        shared.set(Phase::Ready);
        assert_eq!(*shared.phase(), Phase::Exited("gone".into(), true));
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
