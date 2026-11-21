//! The app as the host (ledger-host app_server.rs and bridge.rs INVARIANT
//! 12; plan/p15-app-and-relay.md 3.1; WB-app-1, WB-app-3): the books
//! server is a child process of the window, and the bridge talks to the
//! running app and says so in words when it cannot.
//!
//! ```text
//! GOAL: the child starts, answers, opens and closes one company at a
//!       time on request, closes cleanly when its input does (the
//!       window's close, or the window's death), is never killed, and a
//!       start is refused while another program holds the books; and an
//!       agent's bridge is told, in words, that the app isn't running or
//!       that the company isn't open, then goes through once it is.
//! SOURCE: plan/p15-app-and-relay.md 3.1 (the app starts the server as a
//!         child and closes it by closing its input; the bridge answers
//!         "Wanderbooks isn't running" or "Company X isn't open"), D-44,
//!         D-77, D-84, WB-app-1, WB-app-3.
//! INVARIANTS:
//!   1. A server started by `AppServer` is `wanderbooks server` of this
//!      program on the test's own root alone, says ready, and holds
//!      run/server.lock until it exits.
//!   2. Closing its input (`stop`) closes the books as the host does:
//!      exit 0, the lock let go, the socket removed.
//!   3. A window that is killed outright leaves no server: its input
//!      closes with it and the books close cleanly.
//!   4. A start is refused, in words, while a server or a host holds the
//!      books, and touches nothing.
//!   5. A stop that runs out of time says the server is still closing and
//!      leaves it alone: nothing is signalled.
//!   6. A tab's company is opened and closed in the server by request; a
//!      company closed is not reachable, one opened is.
//!   7. The bridge, with no host (and no mark of any kind on the root):
//!      "Wanderbooks isn't running" with the app not running, "Company X
//!      isn't open" with it running and the company not, and neither once
//!      both are so, in a session that was already running; it starts no
//!      host.
//!   8. What the window runs with its server is `AppHost` (the program's
//!      `ViewerOwner` only forwards to it): a server that is alive and
//!      late is waited for and stays managed; one that never became ready
//!      is stopped when the window stops, and a window stopped before the
//!      child was stored stops it; the window's stop says, in words, each
//!      company whose closing backup was incomplete and a server that
//!      exited badly; the format upgrade reaches the server; a company
//!      that is not open is told.
//!   9. The roster, from the real server, lists a key and the bridge
//!      session opened with it; ending the key takes both off the roster,
//!      refuses the session's next call and deletes the token file's copy.
//! ```

mod common;

use common::{BOOK, Home, Server, clean_env};
use ledger_host::app_server::{ANOTHER_HAS_THEM, AppHost, AppServer, RETIRED_MARKER, Stopped};
use ledger_host::opener::{host_lock_held, server_lock_held};
use ledger_mcp::admin::{Request, Response};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

fn exe() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_wanderbooks"))
}

/// The server the app starts, on `home`'s root alone, with `open` open.
fn start(home: &Home, open: &[String]) -> AppServer {
    let dir = home.dir.path().to_path_buf();
    AppServer::start_in(exe(), Some(&home.root()), open, move |c: &mut Command| {
        clean_env(c, &dir);
        c.env("XDG_CONFIG_HOME", dir.join(".config"));
    })
    .unwrap_or_else(|why| panic!("{why}"))
}

/// Wait until `ok`, at most a minute.
fn until(what: &str, mut ok: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(60);
    while !ok() {
        assert!(Instant::now() < end, "never: {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Whether the books `id` are open, by the server's own list.
fn open_in_server(home: &Home, id: &str) -> bool {
    match home.admin(Request::List) {
        Response::Books(books) => books.iter().any(|b| b.id == id && b.open),
        other => panic!("{other:?}"),
    }
}

#[test]
fn the_server_is_a_child_that_closes_when_its_input_does() {
    let home = Home::new("app-child");
    let run = home.run();
    let server = start(&home, &[]);
    server.wait_ready(Duration::from_secs(60)).unwrap();
    assert!(
        !run.join(RETIRED_MARKER).exists(),
        "nothing marks the root as the app's"
    );
    assert!(server_lock_held(&run), "it holds the server lock");
    assert!(!host_lock_held(&run), "no host was started beside it");
    assert!(
        matches!(home.admin(Request::List), Response::Books(_)),
        "it answers on its admin socket"
    );
    assert_eq!(server.exited(), None);
    // Closed by closing its input, as the host does.
    assert_eq!(server.stop(Duration::from_secs(60)), Stopped::Closed);
    assert!(
        server
            .exited()
            .is_some_and(|how| how.contains("exit status: 0") || how.contains("exit code: 0")),
        "it exited cleanly: {:?}",
        server.exited()
    );
    until("the lock is let go", || !server_lock_held(&run));
    #[cfg(unix)]
    assert!(
        !run.join("admin.sock").exists() && !run.join("mcp.sock").exists(),
        "its sockets are removed"
    );
}

/// A mark an earlier build left (`run/app-hosted`) means nothing now: the
/// root works as one without it, and the next start of the app's server
/// removes it (app_server.rs INVARIANT 7).
#[test]
fn a_mark_an_earlier_build_left_is_ignored_and_removed() {
    let home = Home::new("app-old-mark");
    let run = home.run();
    std::fs::write(run.join(RETIRED_MARKER), b"").unwrap();
    // Nothing runs: the bridge says so in words, mark or none.
    let mut bridge = Bridge::start(&home);
    assert!(error_of(&bridge.initialize()).starts_with("Wanderbooks isn't running"));
    let server = start(&home, &[]);
    server.wait_ready(Duration::from_secs(60)).unwrap();
    assert!(
        !run.join(RETIRED_MARKER).exists(),
        "the old mark is removed"
    );
    assert!(matches!(home.admin(Request::List), Response::Books(_)));
    assert_eq!(server.stop(Duration::from_secs(60)), Stopped::Closed);
}

/// The server is in a process group of its own, so a Ctrl+C at the
/// terminal the window was started from (sent to the terminal's
/// foreground group, the window's) never reaches it: it closes by its
/// input alone (app_server.rs INVARIANT 3). The window here is a process
/// of its own in a group of its own, that ignores the signal as a window
/// that handles Ctrl+C does; the group is sent SIGINT, as a terminal does.
#[cfg(unix)]
#[test]
fn a_terminal_ctrl_c_reaches_the_window_and_never_the_server() {
    let home = Home::new("app-ctrl-c");
    let run = home.run();
    let mut window = {
        use std::os::unix::process::CommandExt;
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "a_window_that_is_about_to_die", "--nocapture"])
            .env("WB_APP_DYING_HOME", home.dir.path())
            .env("WB_APP_IGNORE_SIGINT", "1")
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap()
    };
    struct Reap(Child);
    impl Drop for Reap {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut lines = BufReader::new(window.stdout.take().unwrap()).lines();
    let mut server_pid = None;
    loop {
        let line = lines.next().expect("the window said ready").unwrap();
        if let Some(pid) = line.strip_prefix(CHILD_PID) {
            server_pid = pid.trim().parse::<i32>().ok();
        }
        if line == READY {
            break;
        }
    }
    let mut window = Reap(window);
    let server_pid = server_pid.expect("the window gave the server's pid");
    let window_group = window.0.id() as i32;
    // SAFETY: getpgid only reads a process-group id.
    let (its, ours) = unsafe { (libc::getpgid(server_pid), libc::getpgrp()) };
    assert_ne!(its, window_group, "the server is in the window's group");
    assert_ne!(its, ours);
    // The terminal's Ctrl+C: SIGINT to the window's whole group.
    // SAFETY: the group of a process this test started and leads.
    assert_eq!(unsafe { libc::killpg(window_group, libc::SIGINT) }, 0);
    // The window is still there, and so is the server, with its books
    // open and answering: nothing closed them.
    std::thread::sleep(Duration::from_millis(1500));
    assert!(window.0.try_wait().unwrap().is_none(), "the window died");
    assert!(server_lock_held(&run), "the server closed on Ctrl+C");
    assert!(matches!(home.admin(Request::List), Response::Books(_)));
    // The window going is what closes it.
    window.0.kill().unwrap();
    window.0.wait().unwrap();
    until("the server let go of the books", || !server_lock_held(&run));
}

/// A command run (as a subprocess, as a person's terminal does) while the
/// app's server runs goes to it: no host, no second server, and what it
/// says is the app's (WB-app-5). It never closes the app's books.
#[test]
fn a_command_with_the_app_running_goes_to_its_server_and_leaves_it_running() {
    let home = Home::new("app-command");
    let run = home.run();
    let server = start(&home, &[]);
    server.wait_ready(Duration::from_secs(60)).unwrap();
    home.create(BOOK, "USD");
    let command = |args: &[&str]| {
        let mut c = Command::new(exe());
        c.args(args)
            .arg("--root")
            .arg(home.root())
            .stdin(Stdio::null());
        clean_env(&mut c, home.dir.path());
        c.output().unwrap()
    };
    let said = |o: &std::process::Output| String::from_utf8_lossy(&o.stdout).into_owned();
    let listed = command(&["books", "list"]);
    assert!(listed.status.success(), "{listed:?}");
    assert!(
        said(&listed).contains(&format!("{BOOK}  open")),
        "{listed:?}"
    );
    // A company is opened and closed in the app's server by the command.
    let closed = command(&["books", "close", BOOK]);
    assert!(closed.status.success(), "{closed:?}");
    assert!(!open_in_server(&home, BOOK));
    let opened = command(&["books", "open", BOOK]);
    assert!(opened.status.success(), "{opened:?}");
    assert!(open_in_server(&home, BOOK));
    // `close` and `doctor` say whose the books are, in words.
    let said_close = command(&["close"]);
    assert!(
        said(&said_close).contains("Wanderbooks (the app) has these books open"),
        "{said_close:?}"
    );
    let doctor = command(&["doctor"]);
    assert!(
        said(&doctor).contains("Wanderbooks (the app) has them")
            && said(&doctor).contains(&format!("{BOOK}  open")),
        "{doctor:?}"
    );
    // Through all of it: no host, one server, still the app's.
    assert!(!host_lock_held(&run));
    assert!(!run.join("lease.sock").exists());
    assert_eq!(server.exited(), None);
    assert!(server_lock_held(&run));
    assert_eq!(server.stop(Duration::from_secs(60)), Stopped::Closed);
}

/// With nothing running, a command that manages the books' files runs a
/// server of its own and leaves nothing behind; one that needs an open
/// company says to start Wanderbooks (WB-app-5).
#[test]
fn with_nothing_running_a_command_runs_its_own_server_or_says_to_start_wanderbooks() {
    let home = Home::new("app-own");
    let run = home.run();
    let command = |args: &[&str]| {
        let mut c = Command::new(exe());
        c.args(args)
            .arg("--root")
            .arg(home.root())
            .stdin(Stdio::null());
        clean_env(&mut c, home.dir.path());
        c.output().unwrap()
    };
    let created = command(&["books", "create", BOOK]);
    assert!(created.status.success(), "{created:?}");
    assert!(!server_lock_held(&run), "the command's server was left");
    assert!(!host_lock_held(&run));
    // The folder is read for a list: nothing is open.
    let listed = command(&["books", "list"]);
    assert!(
        String::from_utf8_lossy(&listed.stdout)
            .contains(&format!("{BOOK}  not open: Wanderbooks isn't running")),
        "{listed:?}"
    );
    // Work on a company opens it in the command's own server, then closes.
    let token = command(&["token", "create", "--book", BOOK]);
    assert!(token.status.success(), "{token:?}");
    let backed = command(&["books", "back-up", BOOK]);
    assert!(
        backed.status.success() || String::from_utf8_lossy(&backed.stderr).contains("backup"),
        "{backed:?}"
    );
    assert!(!server_lock_held(&run), "the command's server was left");
    // Opening a company means nothing with no server: said in words.
    let open = command(&["books", "open", BOOK]);
    assert_eq!(open.status.code(), Some(1), "{open:?}");
    let err = String::from_utf8_lossy(&open.stderr);
    assert!(
        err.contains("Start Wanderbooks first") && err.contains("wanderbooks serve"),
        "{err}"
    );
    assert!(!server_lock_held(&run));
}

/// A command arriving while a server holds the books and has no admin
/// socket yet (the app opening large companies, or finishing a backup)
/// waits, says so in words, and then says plainly that it is still
/// opening: it neither fails with a connection error nor starts a second
/// server beside it (WB-app-5). Here the server is stood in for by its
/// lock.
#[test]
fn a_command_waits_in_words_for_a_server_that_is_still_opening() {
    let home = Home::new("app-opening");
    let run = home.run();
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(run.join("server.lock"))
        .unwrap();
    lock.lock().unwrap();
    let mut c = Command::new(exe());
    c.args(["token", "list", "--root"])
        .arg(home.root())
        .args(["--test-knobs", "open_wait=1500"])
        .stdin(Stdio::null());
    clean_env(&mut c, home.dir.path());
    let started = Instant::now();
    let out = c.output().unwrap();
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let (said, err) = (
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(
        err.contains("Wanderbooks is still opening (or closing) the books: waiting for it"),
        "{said} {err}"
    );
    assert!(
        err.contains("didn't open in time") && err.contains("try again in a moment"),
        "{err}"
    );
    assert!(!err.contains("can't reach"), "a connection error: {err}");
    assert!(started.elapsed() < Duration::from_secs(10));
    // No server of its own was started beside the one holding the lock.
    assert!(!host_lock_held(&run));
    drop(lock);
}

#[test]
fn opening_and_closing_a_company_drives_the_servers_book() {
    let home = Home::new("app-tabs");
    let server = start(&home, &[]);
    server.wait_ready(Duration::from_secs(60)).unwrap();
    home.create(BOOK, "USD");
    assert!(open_in_server(&home, BOOK));
    // A tab closed: the book is closed in the server.
    assert_eq!(server.close_book(BOOK), Ok(None));
    assert!(!open_in_server(&home, BOOK));
    // Closed again, or one that isn't there: nothing to say.
    assert_eq!(server.close_book(BOOK), Ok(None));
    // A tab opened: the book is open again.
    assert_eq!(server.open_book(BOOK), Ok(None));
    assert!(open_in_server(&home, BOOK));
    assert_eq!(server.open_book(BOOK), Ok(None), "already open: fine");
    // A company that isn't in the root is said in words.
    let refused = server.open_book("nope").unwrap_err();
    assert!(refused.contains("no books called"), "{refused}");
    assert_eq!(server.stop(Duration::from_secs(60)), Stopped::Closed);
    // After it, a request says so rather than hanging.
    assert!(server.open_book(BOOK).is_err());
}

#[test]
fn a_start_is_refused_when_another_program_has_the_books() {
    let home = Home::new("app-refused");
    let run = home.run();
    let dir = home.dir.path().to_path_buf();
    let try_start = || {
        let dir = dir.clone();
        AppServer::start_in(exe(), Some(&home.root()), &[], move |c: &mut Command| {
            clean_env(c, &dir);
        })
    };
    // A books server holds them (another window, or one still finishing).
    let other = Server::start_with(&home, &["--open", "none"]);
    let refused = try_start().err().expect("refused");
    assert_eq!(refused, ANOTHER_HAS_THEM);
    assert!(
        !run.join(RETIRED_MARKER).exists(),
        "a refusal touches nothing"
    );
    other.close();
    until("the other let go", || !server_lock_held(&run));
    // A host holds them.
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(run.join("host.lock"))
        .unwrap();
    lock.lock().unwrap();
    assert_eq!(try_start().err().expect("refused"), ANOTHER_HAS_THEM);
    assert!(!run.join(RETIRED_MARKER).exists());
    drop(lock);
    // Held by nobody: it starts.
    let server = try_start().expect("started");
    server.wait_ready(Duration::from_secs(60)).unwrap();
    assert_eq!(server.stop(Duration::from_secs(60)), Stopped::Closed);
}

/// What the helper process below says, one line each.
const CHILD_PID: &str = "wb-app-child-pid ";
const READY: &str = "wb-app-ready";

/// Not a test of its own: the "window" the next test kills. It starts the
/// books server as the app does and then waits to be killed; run by any
/// other means it does nothing.
#[test]
fn a_window_that_is_about_to_die() {
    let Some(home) = std::env::var_os("WB_APP_DYING_HOME") else {
        return;
    };
    let home = std::path::PathBuf::from(home);
    #[cfg(unix)]
    if std::env::var_os("WB_APP_IGNORE_SIGINT").is_some() {
        // A window that survives its terminal's Ctrl+C (it handles it).
        // SAFETY: setting a signal's disposition to ignore.
        unsafe { libc::signal(libc::SIGINT, libc::SIG_IGN) };
    }
    let root = ledger_store::root::default_root(&home);
    let dir = home.clone();
    let server = AppServer::start_in(exe(), Some(&root), &[], move |c: &mut Command| {
        clean_env(c, &dir);
        c.env("XDG_CONFIG_HOME", dir.join(".config"));
    })
    .expect("started");
    server.wait_ready(Duration::from_secs(60)).unwrap();
    println!("{CHILD_PID}{}", server.pid());
    println!("{READY}");
    let _ = std::io::stdout().flush();
    std::thread::sleep(Duration::from_secs(600));
}

#[test]
fn a_window_that_dies_leaves_no_server_and_no_half_closed_book() {
    let home = Home::new("app-dies");
    let run = home.run();
    let mut window = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "a_window_that_is_about_to_die", "--nocapture"])
        .env("WB_APP_DYING_HOME", home.dir.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    // Whatever happens, the stand-in window does not outlive this test.
    struct Reap(Child);
    impl Drop for Reap {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut lines = BufReader::new(window.stdout.take().unwrap()).lines();
    let mut server_pid = None;
    loop {
        let line = lines.next().expect("the window said ready").unwrap();
        if let Some(pid) = line.strip_prefix(CHILD_PID) {
            server_pid = pid.trim().parse::<u32>().ok();
        }
        if line == READY {
            break;
        }
    }
    let mut window = Reap(window);
    assert!(server_lock_held(&run), "the server is running");
    home.create(BOOK, "USD");
    // The window is killed outright: no chance to close anything.
    window.0.kill().unwrap();
    window.0.wait().unwrap();
    until("the server let go of the books", || !server_lock_held(&run));
    #[cfg(unix)]
    {
        let pid = server_pid.expect("the window gave the server's pid") as i32;
        until("the server's process is gone", || {
            // SAFETY: signal 0 only asks whether the process exists; the
            // pid is the server this test's own helper started.
            unsafe { libc::kill(pid, 0) != 0 }
        });
        assert!(
            !run.join("admin.sock").exists(),
            "the books closed as at any close: the socket is removed"
        );
    }
    let _ = server_pid;
    // The books are whole: a new server opens them without a word.
    let again = start(&home, &[BOOK.to_string()]);
    again.wait_ready(Duration::from_secs(60)).unwrap();
    assert!(open_in_server(&home, BOOK));
    assert_eq!(again.stop(Duration::from_secs(60)), Stopped::Closed);
}

/// A "server" that takes five seconds to close once its input ends.
#[cfg(unix)]
fn slow_closer(dir: &Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join("slow-server");
    std::fs::write(
        &path,
        "#!/bin/sh\necho ready\ncat >/dev/null\nsleep 5\nexit 0\n",
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

#[cfg(unix)]
#[test]
fn a_stop_that_runs_out_of_time_leaves_the_server_closing_and_says_so() {
    let home = Home::new("app-slow");
    let slow = slow_closer(home.dir.path());
    let server = AppServer::start_in(&slow, Some(&home.root()), &[], |_| {}).unwrap();
    server.wait_ready(Duration::from_secs(60)).unwrap();
    let pid = server.pid() as i32;
    let started = Instant::now();
    assert_eq!(
        server.stop(Duration::from_millis(300)),
        Stopped::StillClosing
    );
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "the window is not held for the server's whole close"
    );
    // Left alone: still running, nothing signalled.
    assert_eq!(server.exited(), None);
    // SAFETY: signal 0 only asks whether the process exists; it is the
    // child this test started.
    assert_eq!(unsafe { libc::kill(pid, 0) }, 0, "it was not killed");
    // It finishes by itself.
    assert_eq!(server.stop(Duration::from_secs(30)), Stopped::Closed);
}

/// An agent's bridge, spoken to as a client over stdio.
struct Bridge {
    child: Child,
    stdin: std::process::ChildStdin,
    lines: mpsc::Receiver<Value>,
    next: u64,
}

impl Bridge {
    fn start(home: &Home) -> Self {
        let mut c = Command::new(exe());
        c.args(["mcp", "--book", BOOK, "--root"]).arg(home.root());
        clean_env(&mut c, home.dir.path());
        let mut child = c
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Ok(v) = serde_json::from_str::<Value>(&line) {
                    let _ = tx.send(v);
                }
            }
        });
        Self {
            child,
            stdin,
            lines,
            next: 1,
        }
    }

    /// Send a request and return the answer to it.
    fn ask(&mut self, method: &str, params: Value) -> Value {
        let id = self.next;
        self.next += 1;
        let line = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        writeln!(self.stdin, "{line}").unwrap();
        self.stdin.flush().unwrap();
        let end = Instant::now() + Duration::from_secs(60);
        loop {
            let left = end.saturating_duration_since(Instant::now());
            let said = self
                .lines
                .recv_timeout(left)
                .unwrap_or_else(|_| panic!("no answer to {method}"));
            if said["id"] == json!(id) {
                return said;
            }
        }
    }

    fn initialize(&mut self) -> Value {
        self.ask(
            "initialize",
            json!({"protocolVersion": common::PROTOCOL, "capabilities": {},
                   "clientInfo": {"name": "test", "version": "0"}}),
        )
    }

    fn start_call(&mut self) -> Value {
        self.ask(
            "tools/call",
            json!({"name": "books.start", "arguments": {}}),
        )
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn error_of(answer: &Value) -> String {
    answer["error"]["message"]
        .as_str()
        .unwrap_or_else(|| panic!("an error was expected: {answer}"))
        .to_string()
}

#[test]
fn the_bridge_says_the_app_isnt_running_then_works_once_it_is() {
    let home = Home::new("app-bridge");
    let run = home.run();
    // The app isn't running, and no host is: a root that was never the
    // app's, so nothing marks it.
    let mut bridge = Bridge::start(&home);
    let not_running = format!(
        "Wanderbooks isn't running: start Wanderbooks and open {BOOK} (on a computer with no screen, run `wanderbooks serve`), then try again."
    );
    assert_eq!(error_of(&bridge.initialize()), not_running);
    assert_eq!(error_of(&bridge.start_call()), not_running);
    // No host was started for it.
    assert!(!host_lock_held(&run));
    assert!(!run.join("lease.sock").exists());
    // The app starts, with the company closed.
    let server = start(&home, &[]);
    server.wait_ready(Duration::from_secs(60)).unwrap();
    home.create(BOOK, "USD");
    let token = home.token_with(BOOK, "agent", &["read", "write", "post", "admin"]);
    ledger_host::tokens::keeper(&run)
        .set(
            &ledger_host::tokens::Account::new(&home.root(), BOOK, "agent"),
            &token,
        )
        .unwrap();
    assert_eq!(server.close_book(BOOK), Ok(None));
    let not_open = format!("Company {BOOK} isn't open: open it in Wanderbooks.");
    assert_eq!(error_of(&bridge.start_call()), not_open);
    // Its tab opens: the same session goes through.
    assert_eq!(server.open_book(BOOK), Ok(None));
    let answered = bridge.start_call();
    assert!(
        answered.get("error").is_none() && answered.get("result").is_some(),
        "{answered}"
    );
    assert!(!host_lock_held(&run), "still no host");
    // The company's tab closes under the running session: said again.
    assert_eq!(server.close_book(BOOK), Ok(None));
    assert_eq!(error_of(&bridge.start_call()), not_open);
    // The window closes: said again, in the same session.
    assert_eq!(server.stop(Duration::from_secs(60)), Stopped::Closed);
    assert_eq!(error_of(&bridge.start_call()), not_running);
}

/// The window's host, starting the real server of this program on
/// `home`'s root alone, looking at it every `slice` while it starts.
fn host_of(home: &Home, slice: Duration) -> AppHost {
    let dir = home.dir.path().to_path_buf();
    let root = home.root();
    AppHost::with_starter(slice, move |open| {
        let dir = dir.clone();
        AppServer::start_in(exe(), Some(&root), open, move |c: &mut Command| {
            clean_env(c, &dir);
            c.env("XDG_CONFIG_HOME", dir.join(".config"));
        })
    })
}

/// An executable shell script `body` in `dir`.
#[cfg(unix)]
fn script(dir: &Path, name: &str, body: &str) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join(name);
    std::fs::write(&path, body).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

/// A host whose "server" is `program`, which is not a books server.
#[cfg(unix)]
fn host_of_program(
    program: std::path::PathBuf,
    home: &Home,
    started: std::sync::Arc<std::sync::atomic::AtomicUsize>,
) -> AppHost {
    let root = home.root();
    AppHost::with_starter(Duration::from_millis(100), move |open| {
        started.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        AppServer::start_in(&program, Some(&root), open, |_| {})
    })
}

#[cfg(unix)]
fn alive(pid: u32) -> bool {
    // SAFETY: signal 0 only asks whether the process exists; the pid is a
    // child this test started.
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

/// A server that is alive but never ready, and a window that stops: no
/// server is left running that nothing manages (finding: a failed or
/// late start).
#[cfg(unix)]
#[test]
fn a_server_that_never_gets_ready_is_stopped_with_the_window_and_not_left_behind() {
    let home = Home::new("host-never-ready");
    let never = script(
        home.dir.path(),
        "never-ready",
        "#!/bin/sh\ncat >/dev/null\nexit 0\n",
    );
    let started = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let host = std::sync::Arc::new(host_of_program(never, &home, started));
    let starting = std::thread::spawn({
        let host = host.clone();
        move || host.start(&[])
    });
    until("the server was started", || host.running().is_ok());
    let pid = host.running().unwrap().pid();
    // Waited for, not given up on: well past several looks at it.
    std::thread::sleep(Duration::from_millis(500));
    assert!(!starting.is_finished(), "a live server is waited for");
    assert!(alive(pid));
    // The window closes: the server's input closes, it exits, the start
    // ends, and nothing runs.
    assert_eq!(host.stop(Duration::from_secs(30)), Ok(None));
    let came = starting.join().unwrap();
    assert!(came.is_err(), "{came:?}");
    until("the server's process is gone", || !alive(pid));
}

/// The window closes after the child was spawned and before the start
/// stored it (finding: the window closed before the start thread stored
/// the child): the start stops it itself.
#[cfg(unix)]
#[test]
fn a_window_closed_before_the_child_was_stored_stops_the_child() {
    let home = Home::new("host-closed-early");
    let never = script(
        home.dir.path(),
        "never-ready",
        "#!/bin/sh\ncat >/dev/null\nexit 0\n",
    );
    let pids = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let me: std::sync::Arc<std::sync::Mutex<Option<std::sync::Arc<AppHost>>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));
    let root = home.root();
    let host = std::sync::Arc::new(AppHost::with_starter(Duration::from_millis(100), {
        let (pids, me) = (pids.clone(), me.clone());
        move |open| {
            let server = AppServer::start_in(&never, Some(&root), open, |_| {})?;
            pids.lock().unwrap().push(server.pid());
            // The window is closed right here: after the spawn, before
            // the host has the child.
            let host = me.lock().unwrap().clone().unwrap();
            assert_eq!(host.stop(Duration::from_secs(5)), Ok(None));
            Ok(server)
        }
    }));
    *me.lock().unwrap() = Some(host.clone());
    let came = host.start(&[]);
    assert!(came.is_err(), "{came:?}");
    let pid = pids.lock().unwrap()[0];
    until("the child is gone", || !alive(pid));
    // And no start after the window has closed spawns anything.
    let before = pids.lock().unwrap().len();
    assert!(host.start(&[]).is_err());
    assert_eq!(pids.lock().unwrap().len(), before);
}

/// A server that is alive but late (still waiting on another's lock, or
/// opening a big company) is waited for, and then managed like any.
#[cfg(unix)]
#[test]
fn a_late_server_is_waited_for_and_stays_the_windows_to_manage() {
    let home = Home::new("host-late");
    let late = script(
        home.dir.path(),
        "late",
        &format!("#!/bin/sh\nsleep 1\nexec \"{}\" \"$@\"\n", exe().display()),
    );
    let dir = home.dir.path().to_path_buf();
    let root = home.root();
    let host = AppHost::with_starter(Duration::from_millis(150), move |open| {
        let dir = dir.clone();
        AppServer::start_in(&late, Some(&root), open, move |c: &mut Command| {
            clean_env(c, &dir);
            c.env("XDG_CONFIG_HOME", dir.join(".config"));
        })
    });
    let begun = Instant::now();
    host.start(&[]).expect("started, late");
    assert!(
        begun.elapsed() >= Duration::from_millis(900),
        "{:?}",
        begun.elapsed()
    );
    // Managed: a tab's company opens and closes in it.
    home.create(BOOK, "USD");
    assert_eq!(host.close_book(BOOK), Ok(None));
    assert_eq!(host.open_book(BOOK), Ok(None));
    assert_eq!(host.stop(Duration::from_secs(60)), Ok(None));
}

#[test]
fn a_whole_close_says_nothing_and_a_second_stop_is_quiet() {
    let home = Home::new("host-whole");
    let host = host_of(&home, Duration::from_secs(60));
    host.start(&[]).unwrap();
    home.create(BOOK, "USD");
    assert_eq!(host.stop(Duration::from_secs(60)), Ok(None));
    assert_eq!(host.stop(Duration::from_secs(60)), Ok(None));
    assert!(!server_lock_held(&home.run()));
}

/// A server that exits badly is said, in words (finding: the window's
/// close hides a failed exit).
#[cfg(unix)]
#[test]
fn a_server_that_exits_badly_is_said_when_the_window_closes() {
    let home = Home::new("host-bad-exit");
    let bad = script(
        home.dir.path(),
        "bad-exit",
        &format!(
            "#!/bin/sh\necho {}\ncat >/dev/null\nexit 3\n",
            ledger_mcp::local::READY
        ),
    );
    let started = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let host = host_of_program(bad, &home, started);
    host.start(&[]).unwrap();
    let said = host.stop(Duration::from_secs(30)).unwrap().expect("a word");
    assert!(
        said.contains("ended badly") && said.contains("exit status: 3"),
        "{said}"
    );
}

/// A company whose last backup is not whole at the window's close is
/// named, in words, with the reason (finding: the window's close hides an
/// incomplete last backup).
#[test]
fn a_closing_backup_that_was_not_whole_is_said_when_the_window_closes() {
    let home = Home::new("host-backup");
    let host = host_of(&home, Duration::from_secs(60));
    host.start(&[]).unwrap();
    home.create(BOOK, "USD");
    // A backup folder that becomes a file: the backup cannot be made.
    let disk = home.dir.path().join("disk");
    std::fs::create_dir_all(&disk).unwrap();
    let mut w = common::Wire::new(&home, BOOK);
    w.initialize("owner");
    let set = w.call(
        "books.set_setting",
        json!({"changes": [{"key": "backup.folder", "value": disk.display().to_string()}],
               "decided_by": "Test Owner", "note": "Backups."}),
    );
    assert_eq!(set["result"]["isError"], json!(false), "{set}");
    let drafted = w.call(
        "books.draft_entry",
        json!({"date": "2026-03-01", "description": "moves the head"}),
    );
    assert_eq!(drafted["result"]["isError"], json!(false), "{drafted}");
    w.end();
    std::fs::remove_dir_all(&disk).unwrap();
    std::fs::write(&disk, b"not a folder").unwrap();
    let said = host
        .stop(Duration::from_secs(60))
        .unwrap()
        .expect("the incomplete backup is said");
    assert!(
        said.contains(BOOK) && said.contains("last backup wasn't whole"),
        "{said}"
    );
    assert!(
        !said.contains(&home.dir.path().display().to_string()),
        "names the home: {said}"
    );
}

#[test]
fn a_company_the_server_does_not_have_open_is_told() {
    let home = Home::new("host-not-open");
    let host = host_of(&home, Duration::from_secs(60));
    host.start(&[]).unwrap();
    home.create(BOOK, "USD");
    assert_eq!(host.not_open(&[BOOK.to_string()]), vec![]);
    // Closed in the server: it is not open for assistants.
    assert_eq!(host.close_book(BOOK), Ok(None));
    let told = host.not_open(&[BOOK.to_string(), "nope".to_string()]);
    assert_eq!(told.len(), 2, "{told:?}");
    assert_eq!(told[0].0, BOOK);
    assert!(
        told[0].1.contains("couldn't be opened for your assistants"),
        "{told:?}"
    );
    assert_eq!(told[1].0, "nope");
    assert!(told[1].1.contains("doesn't have it"), "{told:?}");
    // Nothing is said about a server that isn't there.
    assert_eq!(host.stop(Duration::from_secs(60)), Ok(None));
    assert_eq!(host.not_open(&[BOOK.to_string()]), vec![]);
}

/// The format upgrade, asked under the app: it goes to the window's own
/// server and is answered by it, not by a host lease that does not exist
/// (finding: "Upgrade format" is broken under the app).
#[test]
fn the_format_upgrade_goes_to_the_windows_own_server() {
    let home = Home::new("host-upgrade");
    let host = host_of(&home, Duration::from_secs(60));
    // No server yet: said before anything is done, in words.
    let early = host.upgrade_format(BOOK, "a passphrase").unwrap_err();
    assert!(
        early.contains("isn't running") && early.contains("nothing was done"),
        "{early}"
    );
    host.start(&[]).unwrap();
    home.create(BOOK, "USD");
    // The server answers (a refusal of this passphrase, or books already
    // in this format): never the lease-socket's "host gave no answer".
    match host.upgrade_format(BOOK, "not the owner passphrase") {
        Ok(lines) => assert!(!lines.is_empty()),
        Err(why) => {
            assert!(why.starts_with("not upgraded: "), "{why}");
            assert!(!why.contains("host gave no answer"), "{why}");
        }
    }
    // A company that isn't there is refused by the server, in its words.
    let refused = host.upgrade_format("nope", "x").unwrap_err();
    assert!(refused.starts_with("not upgraded: "), "{refused}");
    assert_eq!(host.stop(Duration::from_secs(60)), Ok(None));
}

/// What the window's Connect page reads and does, through the real server
/// (not a fake): a bridge session opened with a key is on the roster, and
/// revoking the key takes it off, refuses its next call and forgets its
/// token file (plan/p15-app-and-relay.md 3.3; app_server.rs INVARIANT 12).
#[test]
fn the_roster_lists_a_connected_key_and_revoking_it_ends_its_use() {
    let home = Home::new("app-roster");
    let run = home.run();
    let server = start(&home, &[]);
    server.wait_ready(Duration::from_secs(60)).unwrap();
    home.create(BOOK, "USD");
    assert_eq!(server.open_book(BOOK), Ok(None));
    let token = home.token_with(BOOK, "agent", &["read", "write", "post", "admin"]);
    let account = ledger_host::tokens::Account::new(&home.root(), BOOK, "agent");
    let keeper = ledger_host::tokens::keeper(&run);
    keeper.set(&account, &token).unwrap();

    // A bridge session opens with the key and makes a call.
    let mut bridge = Bridge::start(&home);
    let init = bridge.initialize();
    assert!(init.get("error").is_none(), "{init}");
    let answered = bridge.start_call();
    assert!(
        answered.get("error").is_none() && answered.get("result").is_some(),
        "{answered}"
    );

    // The roster, as the window asks for it, lists the key and its session.
    let roster = server.roster(Duration::from_secs(30)).unwrap();
    assert!(
        roster
            .tokens
            .iter()
            .any(|t| t.book == BOOK && t.name == "agent"),
        "{roster:?}"
    );
    let sessions_on_key = |r: &ledger_host::app_server::Roster| {
        r.sessions
            .iter()
            .filter(|b| b.book == BOOK)
            .flat_map(|b| &b.sessions)
            .filter(|s| s.token == "agent")
            .count()
    };
    assert_eq!(sessions_on_key(&roster), 1, "{roster:?}");
    assert_eq!(keeper.get(&account).unwrap().as_deref(), Some(&*token));

    // Revoked: said in words, and no longer usable.
    let id = roster
        .tokens
        .iter()
        .find(|t| t.book == BOOK && t.name == "agent")
        .map(|t| t.id.clone())
        .unwrap();
    let said = server.revoke_token(BOOK, "agent", &id).unwrap();
    assert!(said.contains("access"), "{said}");
    // (a) the roster no longer lists the key, nor a session on it,
    let after = server.roster(Duration::from_secs(30)).unwrap();
    assert!(
        !after
            .tokens
            .iter()
            .any(|t| t.book == BOOK && t.name == "agent"),
        "{after:?}"
    );
    assert_eq!(sessions_on_key(&after), 0, "{after:?}");
    // (b) the next call with that key is refused,
    let refused = bridge.start_call();
    assert!(
        refused.get("error").is_some() || refused["result"]["isError"] == json!(true),
        "{refused}"
    );
    // (c) and the token file's copy is gone.
    assert_eq!(keeper.get(&account).unwrap(), None);
    // Asking again for a key that is gone says so, and breaks nothing.
    assert!(server.revoke_token(BOOK, "agent", &id).is_err());
    assert_eq!(server.stop(Duration::from_secs(60)), Stopped::Closed);
}

/// The host's own `Common` on `home`'s root: what the window's owner holds.
fn common_on(home: &Home) -> ledger_host::commands::Common {
    ledger_host::commands::Common {
        root: Some(home.root()),
        ..Default::default()
    }
}

/// The id of the live key `agent` of `BOOK`, by the roster.
fn key_id(server: &AppServer) -> Option<String> {
    server
        .roster(Duration::from_secs(30))
        .unwrap()
        .tokens
        .iter()
        .find(|t| t.book == BOOK && t.name == "agent")
        .map(|t| t.id.clone())
}

/// The scopes of the live key `agent` of `BOOK`, by the roster.
fn key_scopes(server: &AppServer) -> Vec<String> {
    server
        .roster(Duration::from_secs(30))
        .unwrap()
        .tokens
        .iter()
        .find(|t| t.book == BOOK && t.name == "agent")
        .map(|t| t.scopes.clone())
        .unwrap()
}

/// "Make the key" (the Connect page) makes the key once, with read, write
/// and post and no admin; a second click, or a click with the copy here
/// gone, replaces nothing and says why in words; a revoke of a row whose
/// key was since replaced ends nothing.
#[test]
fn make_the_key_never_replaces_a_key_and_a_stale_revoke_ends_nothing() {
    use ledger_store::auth::Scope;
    let scopes = [Scope::Read, Scope::Write, Scope::Post];
    let home = Home::new("app-make-key");
    let run = home.run();
    let server = start(&home, &[]);
    server.wait_ready(Duration::from_secs(60)).unwrap();
    home.create(BOOK, "USD");
    assert_eq!(server.open_book(BOOK), Ok(None));
    let common = common_on(&home);
    let account = ledger_host::tokens::Account::new(&home.root(), BOOK, "agent");
    let keeper = ledger_host::tokens::keeper(&run);

    // Made once, and kept; read, write and post, not admin.
    assert_eq!(
        ledger_host::commands::ensure_key(&common, BOOK, &scopes),
        Ok(true)
    );
    let first = key_id(&server).expect("the key was made");
    let text = keeper.get(&account).unwrap().expect("its copy is kept");
    assert_eq!(
        key_scopes(&server),
        ["books:read", "books:write", "books:post"]
    );

    // A second click: nothing is made, nothing replaced.
    assert_eq!(
        ledger_host::commands::ensure_key(&common, BOOK, &scopes),
        Ok(false)
    );
    assert_eq!(key_id(&server).as_deref(), Some(first.as_str()));
    assert_eq!(keeper.get(&account).unwrap().as_deref(), Some(&*text));

    // The key works for ordinary bookkeeping and is refused an admin call.
    let mut bridge = Bridge::start(&home);
    assert!(bridge.initialize().get("error").is_none());
    let read = bridge.start_call();
    assert!(
        read.get("error").is_none() && read["result"]["isError"] != json!(true),
        "{read}"
    );
    let admin = bridge.ask(
        "tools/call",
        json!({"name": "books.access_log", "arguments": {}}),
    );
    let refused = admin.get("error").is_some() || admin["result"]["isError"] == json!(true);
    assert!(refused, "an admin call must be refused: {admin}");
    assert!(
        admin.to_string().contains("books:admin"),
        "it is refused for want of admin: {admin}"
    );

    // The copy here goes missing while the key lives (an assistant may be
    // using it): said in words, and nothing is replaced.
    keeper.delete(&account).unwrap();
    let said = ledger_host::commands::ensure_key(&common, BOOK, &scopes).unwrap_err();
    assert!(
        said.contains("already exists") && said.contains("revoke it"),
        "{said}"
    );
    assert!(!said.contains(&*text), "never the key's text");
    assert_eq!(key_id(&server).as_deref(), Some(first.as_str()));
    assert_eq!(keeper.get(&account).unwrap(), None, "no copy was made");
    // The assistant already using the key is not cut off.
    let still = bridge.start_call();
    assert!(still.get("error").is_none(), "{still}");

    // The same when the copy here is not the live key's text.
    keeper.set(&account, "wb_not_the_key").unwrap();
    let said = ledger_host::commands::ensure_key(&common, BOOK, &scopes).unwrap_err();
    assert!(said.contains("not its text any more"), "{said}");
    assert_eq!(key_id(&server).as_deref(), Some(first.as_str()));

    // The key is made again elsewhere (a row of the old one is on screen):
    // revoking the old row's id ends nothing, and says why.
    let newer_text = home.token_with(BOOK, "agent", &["read", "write", "post"]);
    keeper.set(&account, &newer_text).unwrap();
    let newer = key_id(&server).unwrap();
    assert_ne!(newer, first);
    let said = server.revoke_token(BOOK, "agent", &first).unwrap_err();
    assert!(said.contains("already replaced or revoked"), "{said}");
    assert_eq!(key_id(&server).as_deref(), Some(newer.as_str()));
    assert_eq!(
        keeper.get(&account).unwrap().as_deref(),
        Some(newer_text.as_str()),
        "the newer key's copy stays"
    );
    // Its own id ends it.
    assert!(server.revoke_token(BOOK, "agent", &newer).is_ok());
    assert_eq!(key_id(&server), None);
    assert_eq!(keeper.get(&account).unwrap(), None);
    drop(bridge);
    assert_eq!(server.stop(Duration::from_secs(60)), Stopped::Closed);
}

/// A key ended whose token file copy cannot be deleted is not reported as
/// done: the words say the key is dead and where the file is, and never
/// hold the file's text.
#[cfg(unix)]
#[test]
fn a_revoke_that_leaves_the_token_file_says_so() {
    use std::os::unix::fs::PermissionsExt;
    // SAFETY: geteuid has no preconditions.
    if unsafe { libc::geteuid() } == 0 {
        return; // root deletes in a read-only folder: nothing to show
    }
    let home = Home::new("app-revoke-left");
    let run = home.run();
    let server = start(&home, &[]);
    server.wait_ready(Duration::from_secs(60)).unwrap();
    home.create(BOOK, "USD");
    assert_eq!(server.open_book(BOOK), Ok(None));
    let token = home.token_with(BOOK, "agent", &["read"]);
    let account = ledger_host::tokens::Account::new(&home.root(), BOOK, "agent");
    let keeper = ledger_host::tokens::keeper(&run);
    keeper.set(&account, &token).unwrap();
    let id = key_id(&server).unwrap();
    let folder = run.join("tokens");
    std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o500)).unwrap();
    let result = server.revoke_token(BOOK, "agent", &id);
    std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o700)).unwrap();
    let said = result.unwrap_err();
    assert!(said.contains("key is dead"), "{said}");
    assert!(said.contains("a file with its text remains at"), "{said}");
    assert!(
        said.contains(
            &ledger_host::commands::token_file_path(&run, BOOK, "agent")
                .display()
                .to_string()
        ),
        "{said}"
    );
    assert!(!said.contains(&*token), "never the file's text");
    // The key is dead all the same.
    assert_eq!(key_id(&server), None);
    assert_eq!(keeper.get(&account).unwrap().as_deref(), Some(&*token));
    assert_eq!(server.stop(Duration::from_secs(60)), Stopped::Closed);
}
