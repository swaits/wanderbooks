//! The books open while something holds them (plan 3.9, D-21, D-23): the
//! host, its leases, the helper, the agents' bridge, and `serve`, `close`
//! and `books`. Oracles: the INVARIANTS of ledger-host's host.rs,
//! opener.rs, start.rs, bridge.rs and lease.rs, and the P2 tests the plan
//! names, each covered here under a name of its own (books_close_with_last_holder, openers_share_one_server,
//! bridge_opens_closed_books, bridge_survives_server_restart,
//! close_now_ends_every_lease, host_starts_clean, host_respawns,
//! respawn_backs_off, lease_socket_accepts_only_owner_intents,
//! restart_is_not_close, children_match_host_version,
//! bridge_initializes_during_cold_start, no_local_agent_tcp).
#![cfg(target_os = "linux")]

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Short timings for tests (the defaults are the plan's).
const KNOBS: &str =
    "close_after=2500,backoff=50,exe_poll=100,init_wait=5000,queue_wait=30000,backup_quiet=300";

/// A home with a data root, and the program the tests run (possibly a
/// copy, for restarts).
struct Home {
    dir: tempfile::TempDir,
    exe: PathBuf,
}

/// Wait until a program just copied can be run. A child another test
/// thread forked while the copy was being written holds the copy open for
/// writing until it starts its own program, and running the copy before
/// then fails with "text file busy".
fn runnable(program: &Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match Command::new(program)
            .arg("--help")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
        {
            Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy => {
                assert!(Instant::now() < deadline, "the copy stayed busy");
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => panic!("the copy can't be run: {e}"),
            Ok(_) => return,
        }
    }
}

impl Home {
    fn new(tag: &str) -> Self {
        let dir = tempfile::Builder::new()
            .prefix(&format!("wb-held-{tag}-"))
            .tempdir()
            .unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_wanderbooks"))
            .args(["setup", "--home"])
            .arg(dir.path())
            .env_remove("XDG_DATA_HOME")
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        Self {
            dir,
            exe: PathBuf::from(env!("CARGO_BIN_EXE_wanderbooks")),
        }
    }

    /// Run the program from a copy in the home, so a test can replace it.
    fn with_copy(mut self) -> Self {
        let bin = self.dir.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        let copy = bin.join("wanderbooks");
        std::fs::copy(env!("CARGO_BIN_EXE_wanderbooks"), &copy).unwrap();
        runnable(&copy);
        self.exe = copy;
        self
    }

    /// Replace the program with a new file (as a package upgrade does).
    fn replace_program(&self) {
        let next = self.exe.with_extension("new");
        std::fs::copy(env!("CARGO_BIN_EXE_wanderbooks"), &next).unwrap();
        runnable(&next);
        std::fs::rename(&next, &self.exe).unwrap();
    }

    fn root(&self) -> PathBuf {
        self.dir.path().join(".local/share/wanderbooks")
    }

    fn run_dir(&self) -> PathBuf {
        self.root().join("run")
    }

    /// The program, with the root, the test knobs, and the test home's
    /// environment alone.
    fn cmd(&self, mode: &str) -> Command {
        let mut c = Command::new(&self.exe);
        c.arg(mode)
            .arg("--root")
            .arg(self.root())
            .args(["--test-knobs", KNOBS])
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", self.dir.path());
        c
    }

    fn books(&self, args: &[&str]) -> std::process::Output {
        let mut c = self.cmd("books");
        c.args(args).stdin(Stdio::null());
        c.output().unwrap()
    }

    /// New books, and the agents' token the bridges use for them.
    fn create(&self, id: &str) {
        let out = self.books(&["create", id]);
        assert!(out.status.success(), "{out:?}");
        let out = self
            .cmd("token")
            .args(["create", "--book", id])
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
    }

    /// The pids running `mode` for this root.
    fn pids(&self, mode: &str) -> Vec<u32> {
        let root = self.root().display().to_string();
        let mut out = Vec::new();
        for entry in std::fs::read_dir("/proc").unwrap().flatten() {
            let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
                continue;
            };
            let Ok(raw) = std::fs::read(entry.path().join("cmdline")) else {
                continue;
            };
            let args: Vec<String> = raw
                .split(|b| *b == 0)
                .map(|a| String::from_utf8_lossy(a).into_owned())
                .collect();
            if args.get(1).map(String::as_str) == Some(mode) && args.contains(&root) {
                // A zombie has an empty cmdline; this one is running.
                out.push(pid);
            }
        }
        out
    }

    /// The host. Across a restart the old one hands over to the new for a
    /// moment; any longer, two hosts is a failure.
    fn host(&self) -> Option<u32> {
        wait_for(5, || self.pids("host").len() <= 1);
        let pids = self.pids("host");
        assert!(pids.len() <= 1, "more than one host: {pids:?}");
        pids.first().copied()
    }

    /// Wait until there is no host (the books closed), at most `secs`.
    fn wait_closed(&self, secs: u64) -> bool {
        wait_for(secs, || {
            self.host().is_none() && self.pids("server").is_empty()
        })
    }

    /// One line to the lease socket, the answer.
    fn lease_line(&self, line: &str) -> String {
        use std::io::Read;
        let short = ledger_store::root::SocketPath::new(&self.run_dir(), "lease.sock").unwrap();
        let mut s = std::os::unix::net::UnixStream::connect(short.path()).unwrap();
        s.write_all(line.as_bytes()).unwrap();
        s.shutdown(std::net::Shutdown::Write).unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        out
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        // Leave nothing running: close the books, then make sure.
        let _ = self
            .cmd("close")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        // Holders first (one dying could start a host), then the rest,
        // again until nothing of this root is left.
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let mut found = false;
            for mode in ["serve", "mcp", "host", "server"] {
                for pid in self.pids(mode) {
                    found = true;
                    // SAFETY: a process of this test's own root.
                    unsafe { libc::kill(pid as i32, libc::SIGKILL) };
                }
            }
            if !found || Instant::now() > deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

fn wait_for(secs: u64, mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    done()
}

/// A child whose stdout lines arrive on a channel.
struct Running {
    child: Child,
    lines: mpsc::Receiver<String>,
}

impl Running {
    fn start(mut c: Command) -> Self {
        let mut child = c
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let (tx, lines) = mpsc::channel();
        let out = child.stdout.take().unwrap();
        std::thread::spawn(move || {
            for line in BufReader::new(out).lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        Self { child, lines }
    }

    /// Wait for a line containing `text`.
    fn expect(&self, text: &str, secs: u64) -> String {
        let deadline = Instant::now() + Duration::from_secs(secs);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.lines.recv_timeout(left) {
                Ok(line) if line.contains(text) => return line,
                Ok(_) => {}
                Err(_) => panic!("no line with {text:?} in {secs} s"),
            }
        }
    }

    fn wait(&mut self, secs: u64) -> Option<std::process::ExitStatus> {
        let deadline = Instant::now() + Duration::from_secs(secs);
        while Instant::now() < deadline {
            if let Some(s) = self.child.try_wait().unwrap() {
                return Some(s);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        None
    }

    fn signal(&self, sig: i32) {
        // SAFETY: our own child.
        unsafe { libc::kill(self.child.id() as i32, sig) };
    }

    fn stderr(&mut self) -> String {
        use std::io::Read;
        let mut s = String::new();
        if let Some(mut e) = self.child.stderr.take() {
            let _ = e.read_to_string(&mut s);
        }
        s
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// An agent: the bridge over stdio, spoken to as an MCP client.
struct Agent {
    run: Running,
    next: u64,
    /// Notifications received.
    notes: Vec<Value>,
    /// How it runs the commands it is given (`wanderbooks upload`), as
    /// [`Home::cmd`] runs them, and the folder its files are in: the
    /// program, the data root, the home.
    commands: Option<(PathBuf, PathBuf, PathBuf)>,
}

impl Agent {
    fn start(home: &Home, book: &str) -> Self {
        let mut c = home.cmd("mcp");
        c.args(["--book", book]);
        let mut a = Self::from(c);
        a.commands = Some((home.exe.clone(), home.root(), home.dir.path().to_path_buf()));
        a
    }

    fn from(c: Command) -> Self {
        Self {
            run: Running::start(c),
            next: 1,
            notes: Vec::new(),
            commands: None,
        }
    }

    /// Send `bytes` as `name` as an agent does (D-46): books.begin_upload,
    /// the file written in the home, and the command it returns run on
    /// it. The upload_id.
    fn upload(&mut self, bytes: &[u8], name: &str) -> Result<String, String> {
        let (exe, root, home) = self.commands.clone().expect("an agent Agent::start made");
        let begun = self.call(
            "books.begin_upload",
            json!({"sha256": ledger_store::journal::sha256_hex(bytes), "size": bytes.len(),
                   "mime_type": "text/plain", "file_name": name, "purpose": "receipt"}),
        )?;
        let command = begun["command"]
            .as_str()
            .ok_or_else(|| format!("no upload begun: {begun}"))?;
        let words: Vec<&str> = command
            .strip_suffix(" <the file's path>")
            .ok_or_else(|| command.to_string())?
            .split(' ')
            .collect();
        let path = home.join(format!("upload-{}", begun["upload_id"].as_str().unwrap()));
        std::fs::write(&path, bytes).unwrap();
        let out = Command::new(exe)
            .arg(words[1])
            .arg("--root")
            .arg(root)
            .args(["--test-knobs", KNOBS])
            .args(&words[2..])
            .arg(&path)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &home)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        let _ = std::fs::remove_file(&path);
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).to_string());
        }
        Ok(begun["upload_id"].as_str().unwrap().to_string())
    }

    fn send(&mut self, message: Value) {
        let stdin = self.run.child.stdin.as_mut().unwrap();
        writeln!(stdin, "{message}").unwrap();
        stdin.flush().unwrap();
    }

    /// Send a request and wait for its answer.
    fn request(&mut self, method: &str, params: Value, secs: u64) -> Value {
        let id = self.next;
        self.next += 1;
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        let deadline = Instant::now() + Duration::from_secs(secs);
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let line = self
                .run
                .lines
                .recv_timeout(left)
                .unwrap_or_else(|_| panic!("no answer to {method} in {secs} s"));
            let v: Value = serde_json::from_str(&line).unwrap();
            if v.get("id") == Some(&json!(id)) {
                return v;
            }
            self.notes.push(v);
        }
    }

    fn initialize(&mut self) -> Value {
        let r = self.request(
            "initialize",
            json!({"protocolVersion": "2025-06-18", "capabilities": {},
                   "clientInfo": {"name": "test", "version": "0"}}),
            20,
        );
        self.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        r
    }

    /// Call a tool: Ok(its structured result) or Err(the error's text).
    fn call(&mut self, tool: &str, args: Value) -> Result<Value, String> {
        let r = self.request("tools/call", json!({"name": tool, "arguments": args}), 60);
        if let Some(e) = r.get("error") {
            return Err(e["message"].as_str().unwrap_or_default().to_string());
        }
        let result = &r["result"];
        if result["isError"] == true {
            return Err(result["content"][0]["text"]
                .as_str()
                .unwrap_or_default()
                .to_string());
        }
        Ok(result["structuredContent"].clone())
    }

    /// End the session as an agent does: close stdin.
    fn end(mut self) -> Option<std::process::ExitStatus> {
        drop(self.run.child.stdin.take());
        self.run.wait(30)
    }
}

fn chart(a: &mut Agent) {
    for (code, name, ty) in [
        ("1000", "Cash", "asset"),
        ("3000", "Capital", "equity"),
        ("6000", "Supplies", "expense"),
    ] {
        a.call(
            "books.add_account",
            json!({"code": code, "name": name, "account_type": ty}),
        )
        .unwrap();
    }
}

fn post_one(a: &mut Agent, n: u64) -> Result<u64, String> {
    let e = a.call(
        "books.draft_entry",
        json!({"date": "2026-03-01", "description": format!("entry {n}")}),
    )?;
    let id = e["entry_id"].as_u64().unwrap();
    for (account, side) in [("6000", "debit"), ("1000", "credit")] {
        a.call(
            "books.add_line",
            json!({"entry_id": id, "account": account, "side": side, "amount": "1.00"}),
        )?;
    }
    // Bytes of each entry's own: the same bytes stored already would be
    // named for linking, not uploaded again.
    let upload = a.upload(format!("receipt {n}").as_bytes(), &format!("r{n}.txt"))?;
    a.call(
        "books.attach_receipt",
        json!({"entry_id": id, "upload_id": upload}),
    )?;
    a.call(
        "books.post_entry",
        json!({"entry_id": id, "confirmed": true, "no_party_reason": "test"}),
    )?;
    Ok(id)
}

fn serve(home: &Home) -> Running {
    let r = Running::start(home.cmd("serve"));
    // The first time, it says how to let go.
    r.expect("The books are open. Press Ctrl-C", 30);
    r
}

#[test]
fn a_command_opens_the_books_and_they_close_after_the_last_holder() {
    let home = Home::new("last");
    assert!(home.host().is_none());
    home.create("acme");
    // The command let go; the books stay open for the close delay, then
    // close.
    assert!(home.host().is_some(), "closed at once");
    assert!(home.wait_closed(10), "never closed");
    for sock in ["lease.sock", "mcp.sock", "admin.sock"] {
        assert!(!home.run_dir().join(sock).exists(), "{sock} left behind");
    }
    // A lease arriving within the delay cancels the close.
    home.create("beta");
    std::thread::sleep(Duration::from_millis(200));
    let host = home.host().unwrap();
    home.create("gamma");
    std::thread::sleep(Duration::from_millis(1000));
    assert_eq!(
        home.host(),
        Some(host),
        "closed though a lease came in time"
    );
    assert!(home.wait_closed(10));
    // An agent session killed outright: closed after the delay, too.
    let mut a = Agent::start(&home, "acme");
    a.initialize();
    a.call("books.start", json!({})).unwrap();
    a.run.child.kill().unwrap();
    a.run.child.wait().unwrap();
    std::thread::sleep(Duration::from_millis(300));
    assert!(home.host().is_some(), "closed before the delay");
    assert!(home.wait_closed(10), "never closed after the bridge died");
    // A holder that is killed: closed after the delay, not before.
    let mut held = serve(&home);
    let leases = home.lease_line("{\"proto\":1,\"leases\":null}\n");
    assert!(leases.contains("\"serve\""), "{leases}");
    assert!(leases.contains(&held.child.id().to_string()), "{leases}");
    held.child.kill().unwrap();
    held.child.wait().unwrap();
    std::thread::sleep(Duration::from_millis(300));
    assert!(home.host().is_some(), "closed before the delay");
    assert!(home.wait_closed(10), "never closed after the holder died");
    // Ctrl-C at the terminal: serve lets go, and the books close.
    let mut held = serve(&home);
    held.signal(libc::SIGINT);
    assert!(held.wait(10).is_some_and(|s| s.success()));
    assert!(home.wait_closed(10));
    // The books verify after every close.
    let mut store = ledger_store::Store::open(&home.root().join("books/acme")).unwrap();
    store.verify_and_heal().unwrap();
}

#[test]
fn openers_share_one_host_and_one_server() {
    let home = Home::new("share");
    home.create("acme");
    assert!(home.wait_closed(10));
    // All at once, from closed.
    let mut holders = vec![
        Running::start(home.cmd("serve")),
        Running::start({
            let mut c = home.cmd("mcp");
            c.args(["--book", "acme"]);
            c
        }),
        Running::start({
            let mut c = home.cmd("mcp");
            c.args(["--book", "acme"]);
            c
        }),
    ];
    let list = std::thread::spawn({
        let mut c = home.cmd("books");
        c.arg("list").stdin(Stdio::null());
        move || c.output().unwrap()
    });
    holders[0].expect("The books are open", 30);
    let out = list.join().unwrap();
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("acme  open"),
        "{out:?}"
    );
    assert!(wait_for(10, || home.pids("host").len() == 1));
    assert_eq!(home.pids("host").len(), 1, "one host");
    assert_eq!(home.pids("server").len(), 1, "one server");
    // Both bridges hold (each takes its lease as it starts).
    assert!(
        wait_for(30, || home
            .lease_line("{\"proto\":1,\"leases\":null}\n")
            .matches("\"bridge\"")
            .count()
            == 2),
        "the bridges' leases"
    );
    holders.clear();
    assert!(home.wait_closed(15));
}

#[test]
fn the_bridge_opens_closed_books_and_holds_them_for_the_session() {
    let home = Home::new("bridge");
    home.create("acme");
    assert!(home.wait_closed(10));
    let mut a = Agent::start(&home, "acme");
    let init = a.initialize();
    // Ready in time: the server's own answer, its connect text.
    let result = &init["result"];
    assert_eq!(result["serverInfo"]["name"], "wanderbooks");
    assert_eq!(result["capabilities"]["tools"]["listChanged"], true);
    let text = result["instructions"].as_str().unwrap();
    assert!(
        !text.contains("opening now"),
        "the bridge's stand-in: {text}"
    );
    assert!(!text.contains(&home.root().display().to_string()));
    chart(&mut a);
    post_one(&mut a, 1).unwrap();
    let tools = a.request("tools/list", json!({}), 20);
    assert!(tools["result"]["tools"].as_array().unwrap().len() > 10);
    // A tool error and a protocol error come through as they are.
    assert!(a.call("books.get_entry", json!({"entry_id": 999})).is_err());
    let unknown = a.request("no/such/method", json!({}), 20);
    assert!(unknown.get("error").is_some(), "{unknown}");
    // Held while the session lasts, even past the close delay.
    std::thread::sleep(Duration::from_millis(1500));
    assert!(home.host().is_some());
    // The bridge never uses TCP (no_local_agent_tcp), and TCP is off.
    let bridge = a.run.child.id();
    let server = home.pids("server")[0];
    for pid in [bridge, server] {
        assert!(tcp_sockets(pid).is_empty(), "pid {pid} has TCP sockets");
    }
    assert!(a.end().is_some_and(|s| s.success()));
    assert!(home.wait_closed(10));
    // Unknown books: a plain error, no folder named.
    let mut a = Agent::start(&home, "nobody");
    a.initialize();
    let err = a.call("books.start", json!({})).unwrap_err();
    assert!(err.contains("no books called"), "{err}");
    assert!(!err.contains(&home.root().display().to_string()));
}

/// The TCP sockets a process holds (by inode, against /proc/net/tcp*).
fn tcp_sockets(pid: u32) -> Vec<String> {
    let mut tcp = String::new();
    for f in ["/proc/net/tcp", "/proc/net/tcp6"] {
        tcp.push_str(&std::fs::read_to_string(f).unwrap_or_default());
    }
    let inodes: Vec<&str> = tcp
        .lines()
        .skip(1)
        .filter_map(|l| l.split_whitespace().nth(9))
        .collect();
    let mut found = Vec::new();
    if let Ok(fds) = std::fs::read_dir(format!("/proc/{pid}/fd")) {
        for fd in fds.flatten() {
            if let Ok(target) = std::fs::read_link(fd.path()) {
                let t = target.to_string_lossy().into_owned();
                if let Some(inode) = t.strip_prefix("socket:[").and_then(|r| r.strip_suffix(']'))
                    && inodes.contains(&inode)
                {
                    found.push(t);
                }
            }
        }
    }
    found
}

#[test]
fn the_bridge_answers_initialize_itself_during_a_slow_start() {
    let home = Home::new("cold");
    home.create("acme");
    assert!(home.wait_closed(10));
    let mut c = home.cmd("mcp");
    c.args([
        "--book",
        "acme",
        "--test-knobs",
        "close_after=1500,backoff=50,server_delay=3000,init_wait=300,queue_wait=30000",
    ]);
    let mut a = Agent::from(c);
    let started = Instant::now();
    let init = a.initialize();
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
    let text = init["result"]["instructions"].as_str().unwrap();
    assert!(text.contains("books.start"), "{text}");
    assert!(text.contains("opening now"), "{text}");
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
    // The first call waits for the books, then runs.
    // A ping is answered at once, not after the books open.
    let asked = Instant::now();
    let pong = a.request("ping", json!({}), 10);
    assert!(pong.get("result").is_some(), "{pong}");
    assert!(
        asked.elapsed() < Duration::from_secs(1),
        "{:?}",
        asked.elapsed()
    );
    // A call cancelled while it waits for the books is never made.
    let id = a.next;
    a.next += 1;
    a.send(
        json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {
        "name": "books.add_account",
        "arguments": {"code": "1000", "name": "Cash", "account_type": "asset"}}}),
    );
    std::thread::sleep(Duration::from_millis(200));
    a.send(
        json!({"jsonrpc": "2.0", "method": "notifications/cancelled",
        "params": {"requestId": id, "reason": "changed my mind"}}),
    );
    // A batch is refused, each request in it answered.
    a.send(json!([
        {"jsonrpc": "2.0", "id": "b1", "method": "ping"},
        {"jsonrpc": "2.0", "method": "notifications/progress", "params": {}},
    ]));
    let line = a.run.lines.recv_timeout(Duration::from_secs(10)).unwrap();
    let batch: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(batch[0]["id"], "b1", "{batch}");
    assert_eq!(batch[0]["error"]["code"], -32600);
    // The first call waits for the books, then runs.
    let start = a.call("books.start", json!({})).unwrap();
    assert!(start.is_object());
    assert!(started.elapsed() >= Duration::from_secs(3));
    let chart = a.call("books.chart", json!({})).unwrap();
    assert_eq!(chart["accounts"], json!([]), "the cancelled call was made");
    assert!(
        !a.notes.iter().any(|n| n.get("id") == Some(&json!(id))),
        "the cancelled call was answered: {:?}",
        a.notes
    );
}

#[test]
fn the_bridge_gives_up_on_books_that_never_open() {
    let home = Home::new("never");
    home.create("acme");
    assert!(home.wait_closed(10));
    let mut c = home.cmd("mcp");
    c.args([
        "--book",
        "acme",
        "--test-knobs",
        "close_after=800,backoff=50,server_delay=20000,init_wait=200,queue_wait=1000",
    ]);
    let mut a = Agent::from(c);
    a.initialize();
    let err = a.call("books.start", json!({})).unwrap_err();
    assert!(err.contains("wanderbooks doctor"), "{err}");
    // A ping is answered even so.
    let pong = a.request("ping", json!({}), 10);
    assert!(pong.get("result").is_some(), "{pong}");
}

#[test]
fn closing_the_books_ends_every_lease_and_the_bridge_stays_closed() {
    let home = Home::new("closenow");
    home.create("acme");
    let mut held = serve(&home);
    let mut a = Agent::start(&home, "acme");
    a.initialize();
    a.call("books.start", json!({})).unwrap();
    let out = home.cmd("close").output().unwrap();
    assert!(out.status.success(), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("The books are closed."),
        "{out:?}"
    );
    held.expect("The books were closed", 10);
    assert!(held.wait(10).is_some_and(|s| s.success()));
    assert!(home.wait_closed(10));
    // The agent's session carries on, told the owner closed them; it
    // never opens them again.
    let err = a.call("books.start", json!({})).unwrap_err();
    assert!(err.contains("owner closed the books"), "{err}");
    std::thread::sleep(Duration::from_millis(500));
    assert!(home.host().is_none(), "the bridge reopened the books");
    // Closing closed books says so.
    let out = home.cmd("close").output().unwrap();
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("already closed"),
        "{out:?}"
    );
}

/// The host half of books_close_cleanly: the owner closes the books, or
/// the host is signalled, during a burst of posts through a bridge.
#[test]
fn closing_during_a_burst_of_posts_through_the_bridge_loses_nothing() {
    for how in ["close", "term", "hup"] {
        let home = Home::new(&format!("burst-{how}"));
        home.create("acme");
        let mut a = Agent::start(&home, "acme");
        a.initialize();
        chart(&mut a);
        let poster = std::thread::spawn(move || {
            let mut posted = Vec::new();
            let mut why = String::new();
            for k in 0.. {
                match post_one(&mut a, k) {
                    Ok(id) => posted.push(id),
                    Err(e) => {
                        why = e;
                        break;
                    }
                }
            }
            (posted, why)
        });
        std::thread::sleep(Duration::from_millis(1500));
        match how {
            "close" => assert!(home.cmd("close").output().unwrap().status.success()),
            _ => {
                let host = home.host().unwrap() as i32;
                let sig = if how == "term" {
                    libc::SIGTERM
                } else {
                    libc::SIGHUP
                };
                // SAFETY: this test's own host.
                unsafe { libc::kill(host, sig) };
            }
        }
        let (mut posted, why) = poster.join().unwrap();
        assert!(!posted.is_empty(), "{how}: nothing posted");
        // Told the owner closed them: never "closing, not run", which the
        // bridge sends again, and never "may have run".
        assert!(why.contains("closed the books"), "{how}: stopped by {why}");
        assert!(
            !why.contains("may or may not"),
            "{how}: a call was cut off: {why}"
        );
        assert!(home.wait_closed(15), "{how}");
        let mut store = ledger_store::Store::open(&home.root().join("books/acme")).unwrap();
        store.verify_and_heal().unwrap();
        let mut in_books: Vec<u64> = store.ledger().posted_entries().map(|e| e.id()).collect();
        in_books.sort_unstable();
        posted.sort_unstable();
        assert_eq!(
            in_books, posted,
            "{how}: a post lost, or made without its answer"
        );
    }
}

#[test]
fn the_host_starts_clean_from_a_hostile_bridge() {
    let home = Home::new("clean");
    home.create("acme");
    assert!(home.wait_closed(10));
    // A pipe the bridge inherits; the host must not.
    let (read_end, write_end) = std::io::pipe().unwrap();
    use std::os::fd::AsRawFd;
    let leaked = write_end.as_raw_fd();
    let mut c = home.cmd("mcp");
    c.args(["--book", "acme"])
        .env("XDG_DATA_HOME", "/tmp/wb-evil-data")
        .env("LD_PRELOAD", "/nonexistent/wb-evil.so")
        .env("WB_EVIL", "1")
        .env("XDG_RUNTIME_DIR", "/tmp")
        .env("DBUS_SESSION_BUS_ADDRESS", "unix:path=/tmp/evil-bus");
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: dup2 into the child before exec, keeping the pipe open
        // across it (no close-on-exec) as descriptor 9.
        unsafe {
            c.pre_exec(move || {
                if libc::dup2(leaked, 9) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    let mut a = Agent::from(c);
    a.initialize();
    a.call("books.start", json!({})).unwrap();
    let host = home.host().expect("a host");
    let environ = std::fs::read(format!("/proc/{host}/environ")).unwrap();
    let vars: Vec<String> = environ
        .split(|b| *b == 0)
        .filter(|v| !v.is_empty())
        .map(|v| String::from_utf8_lossy(v).into_owned())
        .collect();
    for v in &vars {
        let name = v.split('=').next().unwrap();
        assert!(
            [
                "PATH",
                "HOME",
                "XDG_RUNTIME_DIR",
                "DBUS_SESSION_BUS_ADDRESS",
                // Set by systemd itself for the host's scope, not passed on.
                "INVOCATION_ID",
                "SYSTEMD_EXEC_PID",
            ]
            .contains(&name),
            "the host inherited {v}"
        );
    }
    assert!(!vars.iter().any(|v| v.contains("evil")), "{vars:?}");
    let home_var = vars.iter().find(|v| v.starts_with("HOME=")).unwrap();
    assert_eq!(
        home_var,
        &format!(
            "HOME={}",
            ledger_store::root::account_home().unwrap().display()
        ),
        "HOME from the account's record"
    );
    // No inherited descriptor: nothing points at the pipe.
    let pipe = std::fs::read_link(format!("/proc/self/fd/{}", read_end.as_raw_fd())).unwrap();
    for fd in std::fs::read_dir(format!("/proc/{host}/fd"))
        .unwrap()
        .flatten()
    {
        let target = std::fs::read_link(fd.path()).unwrap_or_default();
        assert_ne!(target, pipe, "the host holds the inherited pipe");
    }
    // Its own session, its own umask, and not confined.
    let status = std::fs::read_to_string(format!("/proc/{host}/status")).unwrap();
    assert!(status.contains("Umask:\t0077"), "{status}");
    assert!(status.contains("Seccomp:\t0"));
    let session = |pid: u32| {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
        let after = stat.rsplit_once(") ").unwrap().1.to_string();
        after.split_whitespace().nth(3).unwrap().to_string()
    };
    assert_ne!(session(host), session(a.run.child.id()));
    assert_eq!(
        session(host),
        host.to_string(),
        "the host leads its session"
    );
    // The server is started from the host's own program, by /proc/self/exe.
    let server = home.pids("server")[0];
    assert_eq!(
        std::fs::metadata(format!("/proc/{server}/exe"))
            .unwrap()
            .len(),
        std::fs::metadata(format!("/proc/{host}/exe"))
            .unwrap()
            .len()
    );
    drop(write_end);
}

#[test]
fn a_killed_host_is_replaced_by_its_holders() {
    let home = Home::new("respawn");
    home.create("acme");
    let mut held = serve(&home);
    let first = home.host().unwrap();
    // SAFETY: this test's own host.
    unsafe { libc::kill(first as i32, libc::SIGKILL) };
    held.expect("opening the books again", 10);
    held.expect("The books are open again", 30);
    let second = home.host().unwrap();
    assert_ne!(first, second);
    let out = home.books(&["list"]);
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("acme  open"),
        "{out:?}"
    );
    held.signal(libc::SIGTERM);
    assert!(held.wait(10).is_some_and(|s| s.success()));
    assert!(home.wait_closed(10));
}

#[test]
fn holders_back_off_and_give_up_on_a_host_that_keeps_failing() {
    let home = Home::new("backoff");
    // The server can't bind its socket: every start fails.
    std::fs::create_dir(home.run_dir().join("mcp.sock")).unwrap();
    let started = Instant::now();
    let mut held = Running::start(home.cmd("serve"));
    let status = held.wait(60).expect("never gave up");
    assert_eq!(status.code(), Some(1));
    let err = held.stderr();
    assert!(
        err.contains("keeps failing") && err.contains("wanderbooks doctor"),
        "{err}"
    );
    // 50 + 100 + 200 + 400 + 800 ms of waiting at least.
    assert!(
        started.elapsed() >= Duration::from_millis(1550),
        "{:?}",
        started.elapsed()
    );
    let log = std::fs::read_to_string(home.run_dir().join("host.log")).unwrap();
    // One start and five tries; a busy machine may add a start (a host
    // slow to take its lock counts as one that died), never fewer.
    let starts = log.matches("the books server stopped").count();
    assert!(starts >= 6, "one start and five tries: {log}");
    std::fs::remove_dir(home.run_dir().join("mcp.sock")).unwrap();
}

#[test]
fn the_lease_socket_takes_only_the_owners_intents() {
    let home = Home::new("intents");
    home.create("acme");
    let _held = serve(&home);
    for (line, want) in [
        ("hello\n", "not understood"),
        ("{\"proto\":1,\"intent\":\"delete\"}\n", "not understood"),
        (
            "{\"proto\":1,\"intent\":{\"restore\":{\"path\":\"/etc\"}}}\n",
            "not understood",
        ),
        ("{\"proto\":1,\"exec\":\"sh\"}\n", "not understood"),
        ("{\"proto\":9,\"leases\":null}\n", "another version"),
        (
            "{\"proto\":1,\"intent\":{\"consent_format_upgrade\":{\"book\":\"acme\"}}}\n",
            "passphrase",
        ),
        (
            "{\"proto\":1,\"intent\":{\"back_up_now\":{\"book\":\"acme\"}}}\n",
            "not understood",
        ),
        (
            "{\"proto\":1,\"intent\":{\"restore\":{\"source\":\"/x\",\"id\":\"acme\"}}}\n",
            "not understood",
        ),
        (
            "{\"proto\":1,\"intent\":{\"new_books\":{\"id\":\"beta\",\"currency\":\"USD\"}}}\n",
            "not understood",
        ),
    ] {
        let answer = home.lease_line(line);
        assert!(answer.contains(want), "{line:?}: {answer}");
        assert!(
            !answer.contains(&home.root().display().to_string()),
            "{answer}"
        );
    }
    let out = home.books(&["list"]);
    assert!(
        !String::from_utf8_lossy(&out.stdout).contains("beta"),
        "{out:?}"
    );
}

#[test]
fn a_restart_is_not_a_close() {
    let home = Home::new("restart").with_copy();
    home.create("acme");
    let held = serve(&home);
    let other = serve(&home);
    let mut a = Agent::start(&home, "acme");
    a.initialize();
    chart(&mut a);
    let tools_before = a.request("tools/list", json!({}), 20);
    let host = home.host().unwrap();
    let server = home.pids("server")[0];
    // The server runs the host's program: the same file.
    let exe = |pid: u32| std::fs::read_link(format!("/proc/{pid}/exe")).unwrap();
    assert_eq!(exe(server), exe(host));
    home.replace_program();
    // Writes made all through the restart are each answered, and each
    // answered one is in the books, once: those that meet the restart
    // wait for the new server.
    let deadline = Instant::now() + Duration::from_secs(40);
    let (mut restarting, mut reopened, mut during) = (false, false, 0);
    let mut written = Vec::new();
    let mut k = 0u32;
    while !reopened {
        assert!(Instant::now() < deadline, "the restart never finished");
        // Sent while the restart is known to be under way, or (a restart
        // quicker than the lines that tell of it) sent before the books
        // reopened and answered only after waiting for the new server.
        let mid_restart = restarting && !reopened;
        let code = format!("{}", 5000 + k);
        k += 1;
        let sent = Instant::now();
        let r = a.call(
            "books.add_account",
            json!({"code": code, "name": format!("R{code}"), "account_type": "expense"}),
        );
        assert!(r.is_ok(), "a write across the restart: {r:?}");
        written.push(code);
        if mid_restart || sent.elapsed() > Duration::from_millis(300) {
            during += 1;
        }
        while let Ok(line) = held.lines.try_recv() {
            restarting |= line.contains("restarting");
            reopened |= line.contains("open again");
        }
    }
    assert!(
        restarting && during > 0,
        "no write was sent during the restart"
    );
    let chart = a.call("books.chart", json!({})).unwrap();
    let mut in_books: Vec<String> = chart["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|acc| acc["code"].as_str().map(str::to_string))
        .filter(|c| c.starts_with('5'))
        .collect();
    in_books.sort();
    written.sort();
    assert_eq!(
        in_books, written,
        "a write lost, or made twice, across the restart"
    );
    other.expect("The books are open again", 30);
    // The holders waited for the new host: none started one of its own.
    let log = std::fs::read_to_string(home.run_dir().join("host.log")).unwrap();
    assert!(!log.contains("another host has these books open"), "{log}");
    let new_host = home.host().unwrap();
    assert_ne!(new_host, host);
    let new_server = home.pids("server")[0];
    assert_eq!(
        exe(new_server),
        exe(new_host),
        "children match the host's version"
    );
    assert!(!exe(new_host).to_string_lossy().contains("(deleted)"));
    // The agent carries on with no reconnect, and its tools didn't
    // change, so it is told nothing.
    post_one(&mut a, 7).unwrap();
    let tools_after = a.request("tools/list", json!({}), 20);
    assert_eq!(tools_before["result"], tools_after["result"]);
    assert!(
        !a.notes
            .iter()
            .any(|n| n["method"] == "notifications/tools/list_changed"),
        "{:?}",
        a.notes
    );
    assert!(a.call("books.get_entry", json!({"entry_id": 1})).is_ok());
}

#[test]
fn books_commands_hold_the_books_while_they_run() {
    let home = Home::new("cmds");
    let out = home.books(&["list"]);
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("No books yet"),
        "{out:?}"
    );
    home.create("acme");
    let out = home.books(&["create", "acme"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("already books called"),
        "{out:?}"
    );
    let out = home.books(&["create", "euro", "--currency", "EUR"]);
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("(EUR)"),
        "{out:?}"
    );
    // Adopting: previewed, and not done without a yes.
    let old = tempfile::Builder::new()
        .prefix("wb-held-old-")
        .tempdir()
        .unwrap();
    let data = old.path().join("books-data");
    drop(ledger_store::Store::create(&data, ledger_core::Currency::USD).unwrap());
    std::fs::write(
        old.path().join("wanderbooks.toml"),
        "[company]\nname = \"Old Co\"\n",
    )
    .unwrap();
    let toml = old.path().join("wanderbooks.toml");
    let args = |extra: &[&str]| {
        let mut v = vec![
            "adopt",
            data.to_str().unwrap(),
            "--id",
            "old",
            "--toml",
            toml.to_str().unwrap(),
        ];
        v.extend_from_slice(extra);
        v.into_iter().map(String::from).collect::<Vec<_>>()
    };
    let run = |a: Vec<String>| {
        let a: Vec<&str> = a.iter().map(String::as_str).collect();
        home.books(&a)
    };
    let out = run(args(&[]));
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("company.name = \"Old Co\""), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("confirm at a terminal"),
        "{out:?}"
    );
    assert_eq!(out.status.code(), Some(1));
    // A vault beside the books must be named.
    std::fs::create_dir(old.path().join("vault")).unwrap();
    let out = run(args(&["--yes"]));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("--no-vault"),
        "{out:?}"
    );
    let out = run(args(&["--yes", "--no-vault"]));
    assert!(out.status.success(), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("Adopted as \"old\""),
        "{out:?}"
    );
    let out = home.books(&["list"]);
    let text = String::from_utf8_lossy(&out.stdout);
    for id in ["acme", "euro", "old"] {
        assert!(text.contains(&format!("{id}  open")), "{text}");
    }
    // Nothing printed names the books' private folder.
    let root = home.root().display().to_string();
    for o in [
        &out,
        &run(args(&["--yes", "--no-vault"])),
        &home.books(&["create", "acme"]),
    ] {
        for text in [&o.stdout, &o.stderr] {
            assert!(!String::from_utf8_lossy(text).contains(&root), "{o:?}");
        }
    }
}

/// Under a systemd app scope (as uwsm sets up), closing the app that
/// started the host leaves the host for the other holders. Run only where
/// a systemd user manager is there to make scopes.
#[test]
fn the_host_outlives_the_app_scope_that_started_it() {
    let uid = unsafe { libc::geteuid() };
    let cgroup = std::fs::read_to_string("/proc/self/cgroup").unwrap_or_default();
    if !cgroup.contains(&format!("/user@{uid}.service/"))
        || !Path::new("/usr/bin/systemd-run").exists()
    {
        eprintln!("skipped: no systemd user manager here");
        return;
    }
    let home = Home::new("scope");
    home.create("acme");
    assert!(home.wait_closed(10));
    let unit = format!("wb-test-{}", std::process::id());
    // `serve` in its own app scope, as an app launched from a desktop.
    let mut c = Command::new("/usr/bin/systemd-run");
    c.args([
        "--user",
        "--scope",
        "--quiet",
        "--collect",
        &format!("--unit={unit}.scope"),
    ])
    .arg(&home.exe)
    .arg("serve")
    .arg("--root")
    .arg(home.root())
    .args(["--test-knobs", KNOBS])
    .env("HOME", home.dir.path());
    let held = Running::start(c);
    held.expect("The books are open", 30);
    let host = home.host().unwrap();
    // The app's scope stops: everything in it is killed.
    let out = Command::new("systemctl")
        .args(["--user", "stop", &format!("{unit}.scope")])
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    std::thread::sleep(Duration::from_millis(300));
    // The host wasn't in it: it lives on, then closes as nothing holds it.
    let alive = std::fs::metadata(format!("/proc/{host}")).is_ok();
    assert!(alive, "the host died with the app's scope");
    assert!(home.wait_closed(10));
}

/// Plan 3.2: both protocol eras are served. A 2026-07-28 client has no
/// initialize; each request carries its version and client in `_meta`.
#[test]
fn the_bridge_serves_a_client_without_initialize() {
    let home = Home::new("stateless");
    home.create("acme");
    let mut a = Agent::start(&home, "acme");
    let meta = json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientCapabilities": {},
        "io.modelcontextprotocol/clientInfo": {"name": "test", "version": "0"},
    });
    let found = a.request("server/discover", json!({"_meta": meta}), 30);
    assert_eq!(
        found["result"]["capabilities"]["tools"],
        json!({}),
        "{found}"
    );
    let chart = a.request(
        "tools/call",
        json!({"name": "books.chart", "arguments": {}, "_meta": meta}),
        30,
    );
    assert_eq!(
        chart["result"]["structuredContent"]["base_currency"], "USD",
        "{chart}"
    );
}

/// Review round 3, finding 1: the server dying under a bridge (with a call
/// going out) doesn't wedge it: it holds again, a new host starts, and
/// the agent's next calls are answered.
#[test]
fn the_bridge_recovers_when_the_server_dies_under_it() {
    let home = Home::new("dies");
    home.create("acme");
    let mut a = Agent::start(&home, "acme");
    a.initialize();
    a.call("books.start", json!({})).unwrap();
    let server = home.pids("server")[0];
    // SAFETY: this test's own server.
    unsafe { libc::kill(server as i32, libc::SIGKILL) };
    // A call at once: refused unrun (or answered "may have run"), then
    // the next ones work again on a new host and server.
    let _ = a.call("books.start", json!({}));
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let r = a.call("books.chart", json!({}));
        if r.is_ok() {
            break;
        }
        if Instant::now() > deadline {
            let log = std::fs::read_to_string(home.run_dir().join("host.log")).unwrap_or_default();
            let _ = a.run.child.kill();
            let err = a.run.stderr();
            panic!("the bridge never recovered: {r:?}\nhost.log:\n{log}\nbridge:\n{err}");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(home.pids("server").iter().all(|p| *p != server));
}

/// Review round 3, finding 2: a cancel that follows its call at once is
/// still in time while the call waits for the books.
#[test]
fn a_call_cancelled_at_once_is_never_made() {
    let home = Home::new("cancel");
    home.create("acme");
    assert!(home.wait_closed(10));
    let mut c = home.cmd("mcp");
    c.args([
        "--book",
        "acme",
        "--test-knobs",
        "close_after=1500,backoff=50,server_delay=1500,init_wait=200,queue_wait=30000",
    ]);
    let mut a = Agent::from(c);
    a.initialize();
    let mut ids = Vec::new();
    for k in 0..20u64 {
        let id = a.next;
        a.next += 1;
        ids.push(id);
        // The call and its cancel in one write.
        let stdin = a.run.child.stdin.as_mut().unwrap();
        writeln!(
            stdin,
            "{}\n{}",
            json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {
                "name": "books.add_account",
                "arguments": {"code": format!("{}", 1000 + k), "name": format!("A{k}"),
                              "account_type": "asset"}}}),
            json!({"jsonrpc": "2.0", "method": "notifications/cancelled",
                   "params": {"requestId": id}})
        )
        .unwrap();
        stdin.flush().unwrap();
    }
    let chart = a.call("books.chart", json!({})).unwrap();
    assert_eq!(chart["accounts"], json!([]), "a cancelled call was made");
    assert!(
        !a.notes.iter().any(|n| n
            .get("id")
            .is_some_and(|i| ids.contains(&i.as_u64().unwrap_or(0)))),
        "a cancelled call was answered: {:?}",
        a.notes
    );
}

#[test]
fn doctor_says_whether_the_books_are_open_and_what_holds_them() {
    let home = Home::new("doctor");
    home.create("acme");
    assert!(home.wait_closed(10));
    let text = |o: std::process::Output| String::from_utf8_lossy(&o.stdout).into_owned();
    let closed = text(home.cmd("doctor").output().unwrap());
    assert!(closed.contains("The books are closed."), "{closed}");
    assert!(closed.contains("same_user (advisory)"), "{closed}");
    let _held = serve(&home);
    let open = text(home.cmd("doctor").output().unwrap());
    assert!(open.contains("The books are open."), "{open}");
    assert!(open.contains("acme  open"), "{open}");
    assert!(
        open.contains("Held by:") && open.contains("wanderbooks serve"),
        "{open}"
    );
    assert!(open.contains("The host, last said:"), "{open}");
    for t in [&closed, &open] {
        assert!(!t.contains(&home.root().display().to_string()), "{t}");
    }
}

/// Review round 4, finding 2: a bridge stopped by a signal goes at once,
/// even with a call waiting for the books (never sent).
#[test]
fn a_signalled_bridge_goes_at_once() {
    let home = Home::new("sigbridge");
    home.create("acme");
    assert!(home.wait_closed(10));
    let mut c = home.cmd("mcp");
    c.args([
        "--book",
        "acme",
        "--test-knobs",
        "close_after=1500,backoff=50,server_delay=20000,init_wait=200,queue_wait=30000",
    ]);
    let mut a = Agent::from(c);
    a.initialize();
    a.send(json!({"jsonrpc": "2.0", "id": 99, "method": "tools/call",
        "params": {"name": "books.start", "arguments": {}}}));
    std::thread::sleep(Duration::from_millis(300));
    let stopped = Instant::now();
    a.run.signal(libc::SIGTERM);
    assert!(a.run.wait(10).is_some(), "the bridge lingered");
    assert!(
        stopped.elapsed() < Duration::from_secs(5),
        "{:?}",
        stopped.elapsed()
    );
}

/// Review round 4, finding 10: a message too large is refused and the
/// session goes on.
#[test]
fn a_message_too_large_is_refused_and_the_session_goes_on() {
    let home = Home::new("toolarge");
    home.create("acme");
    let mut a = Agent::start(&home, "acme");
    a.initialize();
    {
        let stdin = a.run.child.stdin.as_mut().unwrap();
        // One byte over the limit (1 MiB), its newline aside.
        let max = ledger_mcp::local::MAX_BODY;
        stdin.write_all(b"{\"x\":\"").unwrap();
        stdin.write_all(&vec![b'a'; max + 1 - 8]).unwrap();
        stdin.write_all(b"\"}\n").unwrap();
        stdin.flush().unwrap();
    }
    let line = a.run.lines.recv_timeout(Duration::from_secs(30)).unwrap();
    assert!(line.contains("too large"), "{line}");
    let pong = a.request("ping", json!({}), 20);
    assert!(pong.get("result").is_some(), "{pong}");
    assert!(a.call("books.start", json!({})).is_ok());
}

/// Review round 4, finding 1: where systemd-run can't make its scope (no
/// user bus reachable, as in a container), the host starts directly.
#[test]
fn the_host_starts_without_a_scope_when_systemd_run_fails() {
    // SAFETY: geteuid has no preconditions.
    let uid = unsafe { libc::geteuid() };
    let cgroup = std::fs::read_to_string("/proc/self/cgroup").unwrap_or_default();
    let in_scope = cgroup.lines().any(|l| {
        l.starts_with("0::")
            && l.contains(&format!("/user@{uid}.service/"))
            && l.ends_with(".scope")
    });
    if !in_scope || !Path::new("/usr/bin/systemd-run").exists() {
        eprintln!("skipped: not in a systemd app scope");
        return;
    }
    let home = Home::new("noscope");
    // A private runtime folder with no bus in it.
    let runtime = home.dir.path().join("rt");
    std::fs::create_dir(&runtime).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o700)).unwrap();
    let started = Instant::now();
    let out = home
        .cmd("books")
        .args(["create", "acme"])
        .env("XDG_RUNTIME_DIR", &runtime)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );
}

/// Review round 5, findings 2 and 3: a malformed byte is answered and the
/// session goes on; a message exactly at the limit is taken, and the next
/// one is not lost.
#[test]
fn malformed_and_boundary_input_never_ends_the_session() {
    let home = Home::new("bytes");
    home.create("acme");
    let mut a = Agent::start(&home, "acme");
    a.initialize();
    {
        let stdin = a.run.child.stdin.as_mut().unwrap();
        stdin
            .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":\"\xff\",\"method\":\"ping\"}\n")
            .unwrap();
        stdin.flush().unwrap();
    }
    let line = a.run.lines.recv_timeout(Duration::from_secs(20)).unwrap();
    assert!(line.contains("-32700"), "{line}");
    let pong = a.request("ping", json!({}), 20);
    assert!(pong.get("result").is_some(), "{pong}");
    // Exactly the limit, 1 MiB of message (a JSON string padded with
    // spaces), then a ping: the big one is the server's to refuse, the
    // ping answered.
    {
        let max = 1024 * 1024;
        let head = b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/x\",\"params\":{\"pad\":\"";
        let tail = b"\"}}";
        let stdin = a.run.child.stdin.as_mut().unwrap();
        stdin.write_all(head).unwrap();
        let mut left = max - head.len() - tail.len();
        let chunk = vec![b' '; 1024 * 1024];
        while left > 0 {
            let n = left.min(chunk.len());
            stdin.write_all(&chunk[..n]).unwrap();
            left -= n;
        }
        stdin.write_all(tail).unwrap();
        stdin.write_all(b"\n").unwrap();
        stdin.flush().unwrap();
    }
    let pong = a.request("ping", json!({}), 60);
    assert!(pong.get("result").is_some(), "{pong}");
    assert!(
        !a.notes.iter().any(|n| n.to_string().contains("too large")),
        "a message at the limit was refused: {:?}",
        a.notes
    );
}

/// Review round 5, finding 4: `doctor` doesn't hang on a server that
/// doesn't answer, and still shows the logs.
#[test]
fn doctor_doesnt_hang_on_a_frozen_server() {
    let home = Home::new("frozen");
    home.create("acme");
    let _held = serve(&home);
    let server = home.pids("server")[0] as i32;
    // SAFETY: this test's own server.
    unsafe { libc::kill(server, libc::SIGSTOP) };
    let started = Instant::now();
    let out = home.cmd("doctor").output().unwrap();
    // SAFETY: as above.
    unsafe { libc::kill(server, libc::SIGCONT) };
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        started.elapsed() < Duration::from_secs(15),
        "{:?}",
        started.elapsed()
    );
    assert!(text.contains("doesn't answer"), "{text}");
    assert!(text.contains("The host, last said:"), "{text}");
}

/// Review round 5, finding 1: an app started with a scrubbed environment
/// (no XDG_RUNTIME_DIR, as Codex starts its tools) doesn't take the host
/// down with it when it stops.
#[test]
fn a_scrubbed_app_scope_doesnt_take_the_host_with_it() {
    // SAFETY: geteuid has no preconditions.
    let uid = unsafe { libc::geteuid() };
    let cgroup = std::fs::read_to_string("/proc/self/cgroup").unwrap_or_default();
    if !cgroup.contains(&format!("/user@{uid}.service/"))
        || !Path::new("/usr/bin/systemd-run").exists()
    {
        eprintln!("skipped: no systemd user manager here");
        return;
    }
    let home = Home::new("scrubbed");
    home.create("acme");
    assert!(home.wait_closed(10));
    let unit = format!("wb-scrub-{}", std::process::id());
    let mut c = Command::new("/usr/bin/systemd-run");
    c.args([
        "--user",
        "--scope",
        "--quiet",
        "--collect",
        &format!("--unit={unit}.scope"),
    ])
    .arg("/usr/bin/env")
    .arg("-i")
    .arg("PATH=/usr/bin:/bin")
    .arg(format!("HOME={}", home.dir.path().display()))
    .arg(&home.exe)
    .arg("serve")
    .arg("--root")
    .arg(home.root())
    .args(["--test-knobs", KNOBS]);
    let held = Running::start(c);
    held.expect("The books are open", 30);
    let host = home.host().unwrap();
    let host_cgroup = std::fs::read_to_string(format!("/proc/{host}/cgroup")).unwrap();
    assert!(
        !host_cgroup.contains(&unit),
        "the host is in the app's scope: {host_cgroup}"
    );
    let out = Command::new("systemctl")
        .args(["--user", "stop", &format!("{unit}.scope")])
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        std::fs::metadata(format!("/proc/{host}")).is_ok(),
        "the host died with the app"
    );
    assert!(home.wait_closed(10));
}

/// Review round 6, finding 1: a command ends at its deadline even while a
/// restart holds the update lock.
#[test]
fn a_command_ends_at_its_deadline_during_a_restart() {
    let home = Home::new("deadline");
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(home.run_dir().join("update.lock"))
        .unwrap();
    lock.lock().unwrap();
    let started = Instant::now();
    let out = home
        .cmd("books")
        .args(["list", "--test-knobs", "open_wait=1000,close_after=2500"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("didn't open in time"),
        "{out:?}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(6),
        "{:?}",
        started.elapsed()
    );
    drop(lock);
}

/// Review round 6, finding 2: `doctor` doesn't wait long on a frozen host.
#[test]
fn doctor_doesnt_hang_on_a_frozen_host() {
    let home = Home::new("frozenhost");
    home.create("acme");
    let _held = serve(&home);
    let host = home.host().unwrap() as i32;
    // SAFETY: this test's own host.
    unsafe { libc::kill(host, libc::SIGSTOP) };
    let started = Instant::now();
    let out = home.cmd("doctor").output().unwrap();
    // SAFETY: as above.
    unsafe { libc::kill(host, libc::SIGCONT) };
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        started.elapsed() < Duration::from_secs(15),
        "{:?}",
        started.elapsed()
    );
    assert!(text.contains("isn't answering"), "{text}");
}

/// Review round 6, finding 3: Ctrl-C reaches `serve` while it waits to
/// try again.
#[test]
fn serve_hears_ctrl_c_while_it_waits_to_try_again() {
    let home = Home::new("ctrlc");
    std::fs::create_dir(home.run_dir().join("mcp.sock")).unwrap();
    let mut held = Running::start({
        let mut c = home.cmd("serve");
        c.args(["--test-knobs", "close_after=2500,backoff=20000"]);
        c
    });
    // The first start fails at once; serve then waits 20 s to try again.
    held.expect("Opening the books", 30);
    assert!(wait_for(10, || std::fs::read_to_string(
        home.run_dir().join("host.log")
    )
    .is_ok_and(|l| l.contains("the books server stopped"))));
    std::thread::sleep(Duration::from_millis(500));
    let asked = Instant::now();
    held.signal(libc::SIGINT);
    let status = held.wait(10).expect("serve didn't hear Ctrl-C");
    assert!(status.success(), "{status:?}");
    assert!(
        asked.elapsed() < Duration::from_secs(5),
        "{:?}",
        asked.elapsed()
    );
    std::fs::remove_dir(home.run_dir().join("mcp.sock")).unwrap();
}

/// Review round 6, finding 4: every malformed message is answered.
#[test]
fn every_malformed_message_is_answered() {
    let home = Home::new("malformed");
    home.create("acme");
    let mut a = Agent::start(&home, "acme");
    a.initialize();
    for (line, id) in [
        (r#"{"jsonrpc":"2.0","id":5,"method":7}"#, json!(5)),
        ("42", Value::Null),
        (r#"{"jsonrpc":"2.0","id":6}"#, json!(6)),
        (
            r#"{"jsonrpc":"2.0","id":{"a":1},"method":"ping"}"#,
            Value::Null,
        ),
        ("[]", Value::Null),
    ] {
        let stdin = a.run.child.stdin.as_mut().unwrap();
        writeln!(stdin, "{line}").unwrap();
        stdin.flush().unwrap();
        let answer = a.run.lines.recv_timeout(Duration::from_secs(20)).unwrap();
        let v: Value = serde_json::from_str(&answer).unwrap();
        assert_eq!(v["error"]["code"], -32600, "{line}: {answer}");
        assert_eq!(v["id"], id, "{line}: {answer}");
    }
    // And the session goes on.
    assert!(a.call("books.start", json!({})).is_ok());
}

/// Review round 7, finding 1: a call to books that didn't open is
/// refused with why (it never ran), even for a client that skips
/// initialize; and a duplicate in-flight id is refused.
#[test]
fn books_that_didnt_open_refuse_plainly() {
    let home = Home::new("damaged");
    home.create("acme");
    assert!(home.wait_closed(10));
    // A damaged last line: the books won't open.
    use std::io::Write as _;
    let mut j = std::fs::OpenOptions::new()
        .append(true)
        .open(home.root().join("books/acme/journal.jsonl"))
        .unwrap();
    writeln!(j, "{{\"not\": \"a journal line\"}}").unwrap();
    drop(j);
    let mut a = Agent::start(&home, "acme");
    let meta = json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientCapabilities": {},
        "io.modelcontextprotocol/clientInfo": {"name": "test", "version": "0"},
    });
    let r = a.request(
        "tools/call",
        json!({"name": "books.chart", "arguments": {}, "_meta": meta}),
        60,
    );
    let why = r["error"]["message"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(why.contains("didn't open"), "{r}");
    assert!(!why.contains("may or may not"), "{r}");
    assert!(!why.contains(&home.root().display().to_string()), "{r}");
}

/// Review round 7, findings 2 and 3: `close` and `books list` don't hang
/// on a frozen host or server.
#[test]
fn close_and_list_dont_hang_on_frozen_processes() {
    let home = Home::new("frozen2");
    home.create("acme");
    let _held = serve(&home);
    let server = home.pids("server")[0] as i32;
    let host = home.host().unwrap() as i32;
    // SAFETY: this test's own server and host.
    unsafe { libc::kill(server, libc::SIGSTOP) };
    let started = Instant::now();
    let out = home.books(&["list"]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(
        started.elapsed() < Duration::from_secs(45),
        "{:?}",
        started.elapsed()
    );
    // SAFETY: as above.
    unsafe {
        libc::kill(server, libc::SIGCONT);
        libc::kill(host, libc::SIGSTOP);
    }
    let started = Instant::now();
    let out = home.cmd("close").output().unwrap();
    // SAFETY: as above.
    unsafe { libc::kill(host, libc::SIGCONT) };
    assert!(
        started.elapsed() < Duration::from_secs(15),
        "{:?}",
        started.elapsed()
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("isn't answering"),
        "{out:?}"
    );
}

/// Review round 8, findings 1, 2 and 7: books made after the agent's
/// session began are reached on the first call after; an initialize the
/// server won't take fails plainly, not as books that never open; and a
/// request id still in flight is refused.
#[test]
fn books_made_mid_session_and_bad_requests() {
    let home = Home::new("midsession");
    home.create("acme");
    let mut a = Agent::start(&home, "beta");
    a.initialize();
    let err = a.call("books.start", json!({})).unwrap_err();
    assert!(err.contains("no books called"), "{err}");
    home.create("beta");
    assert!(
        a.call("books.start", json!({})).is_ok(),
        "the first call after books create"
    );
    // An initialize without clientInfo: the server's own refusal, fast.
    let mut b = Agent::start(&home, "acme");
    let started = Instant::now();
    let _ = b.request(
        "initialize",
        json!({"protocolVersion": "2025-06-18", "capabilities": {}}),
        20,
    );
    b.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
    let err = b.call("books.start", json!({})).unwrap_err();
    assert!(!err.contains("didn't open within"), "{err}");
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "{:?}",
        started.elapsed()
    );
    // Two requests with one id while the books open: the second refused.
    let home2 = Home::new("dupid");
    home2.create("acme");
    assert!(home2.wait_closed(10));
    let mut c = home2.cmd("mcp");
    c.args([
        "--book",
        "acme",
        "--test-knobs",
        "close_after=2500,backoff=50,server_delay=2000,init_wait=200,queue_wait=30000",
    ]);
    let mut d = Agent::from(c);
    d.initialize();
    let call = json!({"jsonrpc": "2.0", "id": "same", "method": "tools/call",
        "params": {"name": "books.start", "arguments": {}}});
    d.send(call.clone());
    d.send(call);
    let first = d.run.lines.recv_timeout(Duration::from_secs(20)).unwrap();
    let v: Value = serde_json::from_str(&first).unwrap();
    assert_eq!(v["id"], "same", "{first}");
    assert_eq!(v["error"]["code"], -32600, "{first}");
    let second = d.run.lines.recv_timeout(Duration::from_secs(30)).unwrap();
    let v: Value = serde_json::from_str(&second).unwrap();
    assert!(
        v.get("result").is_some(),
        "the first one is still answered: {second}"
    );
}

/// `wanderbooks books`: its flags taken for what they are, its usage on
/// request, and adoption's choices honoured (slice 3 mutation testing).
#[test]
fn the_books_command_takes_its_flags_and_asks_before_adopting() {
    let home = Home::new("flags");
    home.create("acme");
    let out = home.books(&["--help"]);
    assert_eq!(out.status.code(), Some(0));
    let usage = String::from_utf8_lossy(&out.stderr);
    assert!(usage.contains("adopt DATA-DIR --id ID"), "{usage}");
    let out = home.books(&["-h"]);
    assert_eq!(out.status.code(), Some(0));
    let out = home.books(&["list", "--bogus"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("unknown flag: --bogus"));
    // Old books to adopt: one setting the books don't know.
    let old = tempfile::Builder::new()
        .prefix("wb-held-flags-")
        .tempdir()
        .unwrap();
    let data = old.path().join("books-data");
    drop(ledger_store::Store::create(&data, ledger_core::Currency::USD).unwrap());
    let toml = old.path().join("wanderbooks.toml");
    std::fs::write(&toml, "[company]\nname = \"Old Co\"\nmystery = \"what\"\n").unwrap();
    let json = old.path().join("config.json");
    std::fs::write(&json, "{}").unwrap();
    let vault = old.path().join("elsewhere");
    std::fs::create_dir(&vault).unwrap();
    let key = old.path().join("vault.key");
    std::fs::write(&key, "not used").unwrap();
    let adopt = |extra: &[&str]| {
        let mut a = vec![
            "adopt",
            data.to_str().unwrap(),
            "--id",
            "old",
            "--toml",
            toml.to_str().unwrap(),
        ];
        a.extend_from_slice(extra);
        home.books(&a)
    };
    let err = |o: &std::process::Output| String::from_utf8_lossy(&o.stderr).to_string();
    // A vault and its key come together, and never with --no-vault.
    let v = vault.to_str().unwrap();
    let k = key.to_str().unwrap();
    for (extra, want) in [
        (vec!["--vault", v], "give the vault's key too"),
        (vec!["--vault-key", k], "give the vault's folder too"),
        (vec!["--no-vault", "--vault", v], "can't both be given"),
        (vec!["--no-vault", "--vault-key", k], "can't both be given"),
    ] {
        let out = adopt(&extra);
        assert_eq!(out.status.code(), Some(1), "{extra:?}: {out:?}");
        assert!(err(&out).contains(want), "{extra:?}: {}", err(&out));
    }
    // An entry the books don't know: refused unless left behind.
    let out = adopt(&[
        "--yes",
        "--no-vault",
        "--config-json",
        json.to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(err(&out).contains("--skip-unknown"), "{}", err(&out));
    // At a terminal: asked, and only a yes adopts.
    let at_terminal = |answer: &str| {
        use std::io::Write;
        let mut args: Vec<String> = vec![
            home.exe.display().to_string(),
            "books".into(),
            "--root".into(),
            home.root().display().to_string(),
            "--test-knobs".into(),
            KNOBS.into(),
        ];
        for a in [
            "adopt",
            data.to_str().unwrap(),
            "--id",
            "old",
            "--toml",
            toml.to_str().unwrap(),
            "--no-vault",
            "--skip-unknown",
            "--config-json",
            json.to_str().unwrap(),
        ] {
            args.push(a.to_string());
        }
        let mut child = Command::new("script")
            .args(["-qefc", &args.join(" "), "/dev/null"])
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", home.dir.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut stdin = child.stdin.take().unwrap();
        std::thread::sleep(Duration::from_millis(1500));
        let _ = writeln!(stdin, "{answer}");
        let deadline = Instant::now() + Duration::from_secs(60);
        while child.try_wait().unwrap().is_none() {
            assert!(Instant::now() < deadline, "the adoption never ended");
            std::thread::sleep(Duration::from_millis(50));
        }
        drop(stdin);
        let out = child.wait_with_output().unwrap();
        String::from_utf8_lossy(&out.stdout).to_string()
    };
    let shown = at_terminal("n");
    assert!(shown.contains("Adopt them? [y/N]"), "{shown}");
    assert!(shown.contains("Not adopted."), "{shown}");
    let listed = String::from_utf8_lossy(&home.books(&["list"]).stdout).to_string();
    assert!(!listed.contains("old"), "{listed}");
    let shown = at_terminal("y");
    assert!(shown.contains("Adopted as \"old\""), "{shown}");
    assert!(shown.contains("1 settings moved in."), "{shown}");
    let head = shown
        .lines()
        .find(|l| l.starts_with("Head: event"))
        .unwrap_or_else(|| panic!("{shown}"));
    assert!(
        head.trim_end().ends_with(").") && head.contains(" ("),
        "{head}"
    );
    let hash = head
        .split('(')
        .nth(1)
        .unwrap()
        .trim_end_matches([')', '.', '\r']);
    assert_eq!(hash.len(), 12, "{head}");
    assert!(hash.chars().all(|c| c.is_ascii_hexdigit()), "{head}");
}

#[test]
fn adopted_books_old_copies_are_put_aside_from_the_command_line() {
    let home = Home::new("aside");
    let old = tempfile::Builder::new()
        .prefix("wb-held-aside-")
        .tempdir()
        .unwrap();
    let data = old.path().join("books-data");
    drop(ledger_store::Store::create(&data, ledger_core::Currency::USD).unwrap());
    let toml = old.path().join("wanderbooks.toml");
    std::fs::write(&toml, "[company]\nname = \"Old Co\"\n").unwrap();
    let (d, t) = (data.to_str().unwrap(), toml.to_str().unwrap());
    let out = home.books(&[
        "adopt",
        d,
        "--id",
        "old",
        "--toml",
        t,
        "--no-vault",
        "--yes",
    ]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let err = |o: &std::process::Output| String::from_utf8_lossy(&o.stderr).to_string();
    let said = |o: &std::process::Output| String::from_utf8_lossy(&o.stdout).to_string();
    // Not at a terminal and no --yes: what would go aside, and nothing moved.
    let out = home.books(&["put-aside", "old", d, t]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(err(&out).contains("not put aside"), "{}", err(&out));
    assert!(
        said(&out).contains("These go aside, each whole"),
        "{}",
        said(&out)
    );
    assert!(said(&out).contains("byte for byte"), "{}", said(&out));
    assert!(said(&out).contains("Nothing is deleted"), "{}", said(&out));
    assert!(data.is_dir() && toml.is_file());
    // A vault comes with its key; books of no such name hold nothing.
    let out = home.books(&["put-aside", "old", d, "--vault", t, "--yes"]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(err(&out).contains("together"), "{}", err(&out));
    let out = home.books(&["put-aside", "nobody", d, "--yes"]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(data.is_dir());
    // With --yes: moved, whole, into the legacy folder; the books go on.
    let out = home.books(&["put-aside", "old", d, t, "--yes"]);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(said(&out).contains("Put aside: 2 paths"), "{}", said(&out));
    assert!(!data.exists() && !toml.exists());
    let legacy: Vec<_> = std::fs::read_dir(home.root().join("legacy"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(legacy.len(), 1, "{legacy:?}");
    assert!(legacy[0].join("1/books-data/journal.jsonl").is_file());
    assert!(legacy[0].join("put-aside.txt").is_file());
    assert_eq!(
        std::fs::read_to_string(legacy[0].join("2/wanderbooks.toml")).unwrap(),
        "[company]\nname = \"Old Co\"\n"
    );
    let listed = said(&home.books(&["list"]));
    assert!(listed.contains("old"), "{listed}");
}

/// A program that starts the bridge with its stderr closed (some launchers
/// do) still gets a working bridge, its descriptor 2 on /dev/null: Rust's
/// runtime opens it before main, so no socket the bridge opens can take
/// its number and be sent the error messages.
#[test]
fn a_bridge_started_without_stderr_keeps_its_descriptors_apart() {
    use std::os::unix::process::CommandExt;
    let home = Home::new("nostderr");
    home.create("acme");
    let mut c = home.cmd("mcp");
    c.args(["--book", "acme"]);
    // SAFETY: close is async-signal-safe.
    unsafe {
        c.pre_exec(|| {
            libc::close(2);
            Ok(())
        });
    }
    let mut a = Agent::from(c);
    a.initialize();
    assert!(a.call("books.start", json!({})).is_ok());
    let pid = a.run.child.id();
    let fd2 = std::fs::read_link(format!("/proc/{pid}/fd/2")).unwrap();
    assert_eq!(fd2, Path::new("/dev/null"));
    assert!(a.end().is_some_and(|s| s.success()));
}

/// P2 slice 4: `wanderbooks token` keeps the token where the bridge finds
/// it (commands.rs INVARIANT 4, bridge.rs INVARIANT 11): made, used,
/// limited by its scopes, listed without its text, revoked at once.
#[test]
fn the_token_commands_and_the_bridge() {
    let home = Home::new("tokencmd");
    let out = home.books(&["create", "acme"]);
    assert!(out.status.success(), "{out:?}");
    let token = |args: &[&str]| {
        let out = home
            .cmd("token")
            .args(args)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).to_string(),
            String::from_utf8_lossy(&out.stderr).to_string(),
        )
    };
    // A session begun before any token: told how to make one.
    let mut a = Agent::start(&home, "acme");
    a.initialize();
    let err = a.call("books.start", json!({})).unwrap_err();
    assert!(err.contains("wanderbooks token create"), "{err}");
    // A read-only token, made: the same session uses it from now on.
    let (code, made, _) = token(&["create", "--book", "acme", "--scopes", "read"]);
    assert_eq!(code, Some(0), "{made}");
    assert!(
        made.contains("kept in a token file in the data root"),
        "{made}"
    );
    assert!(
        made.contains("books:read") && !made.contains("wb_"),
        "{made}"
    );
    assert!(!made.contains(&home.root().display().to_string()));
    assert!(a.call("books.start", json!({})).is_ok());
    let err = a
        .call(
            "books.add_account",
            json!({"code": "1000", "name": "Cash", "account_type": "asset"}),
        )
        .unwrap_err();
    assert!(err.contains("needs books:write"), "{err}");
    // A name taken; replaced on request, and the running session takes
    // the new token at its next call (the old one refused at the door, the
    // token file read again: review of slice 4).
    let (code, _, err) = token(&["create", "--book", "acme"]);
    assert_eq!(code, Some(1));
    assert!(err.contains("already have a token"), "{err}");
    let (code, _, _) = token(&["create", "--book", "acme", "--replace"]);
    assert_eq!(code, Some(0));
    a.call(
        "books.add_account",
        json!({"code": "1000", "name": "Cash", "account_type": "asset"}),
    )
    .unwrap();
    assert!(a.end().is_some());
    let mut b = Agent::start(&home, "acme");
    b.initialize();
    for (code, name, ty) in [
        ("3000", "Capital", "equity"),
        ("6000", "Supplies", "expense"),
    ] {
        b.call(
            "books.add_account",
            json!({"code": code, "name": name, "account_type": ty}),
        )
        .unwrap();
    }
    // Listed without its text; printed only by `print`.
    let (_, listed, _) = token(&["list"]);
    assert!(
        listed.contains("acme  agent  books:read,books:write,books:post,books:admin"),
        "{listed}"
    );
    assert!(!listed.contains("wb_"));
    let (code, printed, _) = token(&["print", "--book", "acme"]);
    assert_eq!(code, Some(0));
    assert!(
        printed.trim().starts_with("wb_") && printed.trim().len() == 67,
        "{printed}"
    );
    // Revoked: the next call is refused, and the token file forgets it.
    let (code, said, _) = token(&["revoke", "--book", "acme"]);
    assert_eq!(code, Some(0), "{said}");
    assert!(b.call("books.chart", json!({})).is_err());
    let (code, _, err) = token(&["print", "--book", "acme"]);
    assert_eq!(code, Some(1));
    assert!(err.contains("keeps no token"), "{err}");
    let (_, listed, _) = token(&["list"]);
    assert!(listed.contains("No tokens yet"), "{listed}");
    // Asked of books there aren't, or with a scope there isn't.
    let (code, _, err) = token(&["create", "--book", "nobody"]);
    assert_eq!(code, Some(1));
    assert!(err.contains("no books called"), "{err}");
    let (code, _, err) = token(&["create", "--book", "acme", "--scopes", "read,everything"]);
    assert_eq!(code, Some(2));
    assert!(err.contains("isn't a scope"), "{err}");
    assert!(b.end().is_some());
}

/// Run `command` (as books.begin_upload or books.get_receipt gave it,
/// with its placeholder cut off) with `more` words after it: its output.
fn run_given(
    home: &Home,
    command: &str,
    placeholder: &str,
    more: &[&std::ffi::OsStr],
) -> std::process::Output {
    let words: Vec<&str> = command
        .strip_suffix(placeholder)
        .unwrap_or_else(|| panic!("{command}"))
        .split(' ')
        .collect();
    assert_eq!(words[0], "wanderbooks", "{command}");
    home.cmd(words[1])
        .args(&words[2..])
        .args(more)
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

#[test]
fn an_agent_sends_and_saves_files_with_the_commands_it_is_given() {
    let home = Home::new("files");
    home.create("acme");
    let mut a = Agent::start(&home, "acme");
    a.initialize();
    let folder = home.dir.path().join("Downloads");
    std::fs::create_dir(&folder).unwrap();
    let mut bytes = b"%PDF-1.4 a big receipt ".to_vec();
    bytes.resize(400 * 1024, b'x');
    let file = folder.join("big receipt.pdf");
    std::fs::write(&file, &bytes).unwrap();
    let begin = |a: &mut Agent| {
        a.call(
            "books.begin_upload",
            json!({"sha256": ledger_store::journal::sha256_hex(&bytes), "size": bytes.len(),
                   "mime_type": "application/pdf", "file_name": "big receipt.pdf",
                   "purpose": "receipt"}),
        )
        .unwrap()
    };
    let begun = begin(&mut a);
    let upload = begun["upload_id"].as_str().unwrap().to_string();
    let command = begun["command"].as_str().unwrap().to_string();
    let quiet = |out: &std::process::Output| {
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(!text.contains(&home.root().display().to_string()), "{text}");
        text
    };
    // Another file than the one declared: refused, nothing kept.
    let other = folder.join("other.pdf");
    std::fs::write(&other, b"%PDF-1.4 not it").unwrap();
    let out = run_given(&home, &command, " <the file's path>", &[other.as_os_str()]);
    assert!(!out.status.success());
    assert!(quiet(&out).contains("not the"), "{}", quiet(&out));
    // A file of exactly the largest size isn't refused for its size.
    let edge = folder.join("edge.pdf");
    std::fs::File::create(&edge)
        .unwrap()
        .set_len(25 * 1024 * 1024)
        .unwrap();
    let out = run_given(&home, &command, " <the file's path>", &[edge.as_os_str()]);
    assert!(!out.status.success());
    assert!(
        quiet(&out).contains("more than the") && !quiet(&out).contains("at most"),
        "{}",
        quiet(&out)
    );
    std::fs::remove_file(&edge).unwrap();
    // A file larger than any upload is refused before it is read.
    let huge = folder.join("huge.pdf");
    std::fs::File::create(&huge)
        .unwrap()
        .set_len(25 * 1024 * 1024 + 1)
        .unwrap();
    let out = run_given(&home, &command, " <the file's path>", &[huge.as_os_str()]);
    assert!(!out.status.success());
    assert!(quiet(&out).contains("at most"), "{}", quiet(&out));
    // With no token kept for the name given: what the human does.
    let out = home
        .cmd("upload")
        .args(["--book", "acme", "--token", "nobody", &upload])
        .arg(&file)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(
        quiet(&out).contains("wanderbooks token create"),
        "{}",
        quiet(&out)
    );
    // A path that looks like a flag goes after the command's `--`.
    let dashed = folder.join("-r.pdf");
    std::fs::copy(&file, &dashed).unwrap();
    let out = home
        .cmd("upload")
        .args(["--book", "acme", "not-an-id", "--"])
        .arg(&dashed)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    assert!(
        quiet(&out).contains("isn't an upload's id"),
        "{}",
        quiet(&out)
    );
    // The file itself goes (by its copy whose name starts with `-`).
    let out = run_given(&home, &command, " <the file's path>", &[dashed.as_os_str()]);
    assert!(out.status.success(), "{}", quiet(&out));
    assert!(quiet(&out).contains(&format!("Sent {} bytes", bytes.len())));
    let filed = a
        .call(
            "books.file_document",
            json!({"category": "other", "upload_id": upload}),
        )
        .unwrap();
    let att = filed["attachment_id"].as_u64().unwrap();
    // Handed over (D-46): the command saves a copy, beside any file of
    // its name, never over it.
    let back = a
        .call(
            "books.get_receipt",
            json!({"attachment_id": att, "view": "bytes"}),
        )
        .unwrap();
    let download = back["file"]["download"].as_str().unwrap().to_string();
    for n in 1..=2 {
        let out = run_given(
            &home,
            &download,
            " --to <a folder>",
            &["--to".as_ref(), folder.as_os_str()],
        );
        assert!(out.status.success(), "{}", quiet(&out));
        let saved = folder.join(format!("big receipt ({n}).pdf"));
        assert_eq!(std::fs::read(&saved).unwrap(), bytes);
        assert!(quiet(&out).contains(&saved.display().to_string()));
    }
    assert_eq!(std::fs::read(&file).unwrap(), bytes, "never overwritten");
    let out = run_given(
        &home,
        &download,
        " --to <a folder>",
        &["--to".as_ref(), file.as_os_str()],
    );
    assert!(!out.status.success());
    assert!(quiet(&out).contains("isn't a folder"));
    let out = home
        .cmd("download")
        .args(["--book", "acme", "999", "--to"])
        .arg(&folder)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!out.status.success());
    quiet(&out);
    assert_eq!(
        std::fs::read_dir(&folder).unwrap().count(),
        6,
        "nothing else saved"
    );
    a.end();
}

#[test]
fn the_commands_given_run_as_the_agents_own_token() {
    let home = Home::new("files-token");
    home.create("acme");
    let out = home
        .cmd("token")
        .args(["create", "--book", "acme", "--name", "bookkeeper"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let mut c = home.cmd("mcp");
    c.args(["--book", "acme", "--token", "bookkeeper"]);
    let mut a = Agent::from(c);
    a.initialize();
    let folder = home.dir.path().join("Downloads");
    std::fs::create_dir(&folder).unwrap();
    let mut bytes = b"%PDF-1.4 kept by the bookkeeper ".to_vec();
    bytes.resize(300 * 1024, b'y');
    let file = folder.join("r.pdf");
    std::fs::write(&file, &bytes).unwrap();
    let begun = a
        .call(
            "books.begin_upload",
            json!({"sha256": ledger_store::journal::sha256_hex(&bytes), "size": bytes.len(),
                   "mime_type": "application/pdf", "file_name": "r.pdf", "purpose": "receipt"}),
        )
        .unwrap();
    let command = begun["command"].as_str().unwrap().to_string();
    assert!(command.contains(" --token bookkeeper "), "{command}");
    let out = run_given(&home, &command, " <the file's path>", &[file.as_os_str()]);
    assert!(out.status.success(), "{out:?}");
    let filed = a
        .call(
            "books.file_document",
            json!({"category": "other", "upload_id": begun["upload_id"]}),
        )
        .unwrap();
    let back = a
        .call(
            "books.get_receipt",
            json!({"attachment_id": filed["attachment_id"], "view": "bytes"}),
        )
        .unwrap();
    let download = back["file"]["download"].as_str().unwrap().to_string();
    assert!(download.contains(" --token bookkeeper "), "{download}");
    let out = run_given(
        &home,
        &download,
        " --to <a folder>",
        &["--to".as_ref(), folder.as_os_str()],
    );
    assert!(out.status.success(), "{out:?}");
    assert_eq!(std::fs::read(folder.join("r (1).pdf")).unwrap(), bytes);
    // A file the books make (an export) is saved the same way. The 1099
    // worksheet counts payments out of the books' cash accounts.
    let _ = a.call(
        "books.add_account",
        json!({"code": "1000", "name": "Cash", "account_type": "asset"}),
    );
    a.call(
        "books.set_setting",
        json!({"changes": [{"key": "books.cash_accounts", "value": ["1000"]}], "decided_by": "Test Owner"}),
    )
    .unwrap();
    let w = a
        .call("books.vendor_1099", json!({"year": 2026, "csv": true}))
        .unwrap();
    let out = run_given(
        &home,
        w["csv"]["download"].as_str().unwrap(),
        " --to <a folder>",
        &["--to".as_ref(), folder.as_os_str()],
    );
    assert!(out.status.success(), "{out:?}");
    let csv = std::fs::read(folder.join("1099-worksheet-2026.csv")).unwrap();
    assert_eq!(
        w["csv"]["sha256"],
        json!(ledger_store::journal::sha256_hex(&csv))
    );
    assert!(csv.starts_with(b"vendor_id,name,"));
    // A file whose name isn't text goes, and comes back into a folder
    // whose name isn't text.
    use std::os::unix::ffi::OsStrExt;
    let odd = folder.join(std::ffi::OsStr::from_bytes(b"r\xff.pdf"));
    let mut other = bytes.clone();
    other[40] = b'z';
    std::fs::write(&odd, &other).unwrap();
    let begun = a
        .call(
            "books.begin_upload",
            json!({"sha256": ledger_store::journal::sha256_hex(&other), "size": other.len(),
                   "mime_type": "application/pdf", "file_name": "r2.pdf", "purpose": "receipt"}),
        )
        .unwrap();
    let out = run_given(
        &home,
        begun["command"].as_str().unwrap(),
        " <the file's path>",
        &[odd.as_os_str()],
    );
    assert!(out.status.success(), "{out:?}");
    let filed = a
        .call(
            "books.file_document",
            json!({"category": "other", "upload_id": begun["upload_id"]}),
        )
        .unwrap();
    let odd_folder = folder.join(std::ffi::OsStr::from_bytes(b"out\xfe"));
    std::fs::create_dir(&odd_folder).unwrap();
    let out = home
        .cmd("download")
        .args(["--book", "acme", "--token", "bookkeeper"])
        .arg(filed["attachment_id"].to_string())
        .arg("--to")
        .arg(&odd_folder)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    assert_eq!(std::fs::read(odd_folder.join("r2.pdf")).unwrap(), other);
    // Anywhere else, a word that isn't text is refused plainly.
    let out = home
        .cmd("books")
        .arg(std::ffi::OsStr::from_bytes(b"\xff"))
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stderr).contains("isn't UTF-8"));
    a.end();
}

#[test]
fn a_backup_is_restored_from_the_command_line() {
    let home = Home::new("restore");
    home.create("acme");
    let disk = home.dir.path().join("disk");
    std::fs::create_dir(&disk).unwrap();
    let mut a = Agent::start(&home, "acme");
    a.initialize();
    chart(&mut a);
    post_one(&mut a, 1).unwrap();
    a.call(
        "books.set_setting",
        json!({"changes": [{"key": "backup.folder", "value": disk.display().to_string()}],
               "decided_by": "Test Owner"}),
    )
    .unwrap();
    // Backed up by the server alone, soon after (the host passed on its
    // quiet).
    let mut s = Value::Null;
    assert!(
        wait_for(30, || {
            s = a.call("books.backup", json!({})).unwrap();
            s["targets"][0]["through_seq"] == s["head_seq"]
        }),
        "never backed up: {s}"
    );
    assert_eq!(s["targets"][0]["last_error"], Value::Null, "{s}");
    let head = s["head_seq"].as_u64().unwrap();
    let backup = disk.join("acme.git");
    let backup = backup.to_str().unwrap();
    // Previewed, and not done without a yes.
    let out = home.books(&["restore", backup, "--id", "copy"]);
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(said.contains(&format!("through event {head}")), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("confirm at a terminal"),
        "{out:?}"
    );
    assert_eq!(out.status.code(), Some(1));
    // Under its own name, over books that exist: refused.
    let out = home.books(&["restore", backup, "--yes"]);
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("already books called"),
        "{out:?}"
    );
    // A repository named otherwise: restored as the books it holds, by
    // default (here: over those, refused), said.
    let renamed = home.dir.path().join("disk").join("fernhill-backup.git");
    let out = Command::new("git")
        .args(["clone", "-q", "--bare"])
        .arg(backup)
        .arg(&renamed)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let out = home.books(&["restore", renamed.to_str().unwrap(), "--yes"]);
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("already books called \"acme\""),
        "{out:?}"
    );
    let out = home.books(&["restore", renamed.to_str().unwrap(), "--id", "other"]);
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(
        said.contains("It is a backup of the books \"acme\""),
        "{out:?}"
    );
    assert!(said.contains("--id acme"), "{out:?}");
    // A repository whose name is no id: still restored as its books.
    let odd = home.dir.path().join("disk").join("Fernhill_Backup.git");
    std::fs::rename(&renamed, &odd).unwrap();
    let out = home.books(&["restore", odd.to_str().unwrap(), "--yes"]);
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("already books called \"acme\""),
        "{out:?}"
    );
    std::fs::rename(&odd, &renamed).unwrap();
    // Beside them.
    let out = home.books(&["restore", backup, "--id", "copy", "--yes"]);
    assert!(out.status.success(), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("Restored as \"copy\""),
        "{out:?}"
    );
    // Replacing them, the old kept: from the repository named otherwise,
    // given no id (the books it holds are replaced).
    post_one(&mut a, 2).unwrap();
    let out = home.books(&["restore", renamed.to_str().unwrap(), "--replace", "--yes"]);
    assert!(out.status.success(), "{out:?}");
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(
        said.contains("kept in the legacy folder as acme-"),
        "{out:?}"
    );
    // The head it restored (post 2 may have been backed up by then).
    let restored: u64 = said
        .split("Restored as \"acme\": through event ")
        .nth(1)
        .and_then(|rest| rest.split(' ').next())
        .and_then(|n| n.parse().ok())
        .expect("the restored head is said");
    assert!(restored >= head, "{said}");
    let head = restored;
    assert!(!said.contains(&home.root().display().to_string()), "{said}");
    let out = home.books(&["list"]);
    let listed = String::from_utf8_lossy(&out.stdout);
    assert!(
        listed.contains("acme  open") && listed.contains("copy  open"),
        "{out:?}"
    );
    // The session of before never reaches the books replaced: refused, or
    // (its bridge opening a new session) the restored books.
    if let Ok(start) = a.call("books.start", json!({})) {
        assert_eq!(start["head"]["seq"].as_u64(), Some(head), "{start}");
    }
    a.end();
    // A new session sees the restored books, at the backup's head.
    let mut fresh = Agent::start(&home, "acme");
    fresh.initialize();
    let start = fresh.call("books.start", json!({})).unwrap();
    assert_eq!(start["head"]["seq"].as_u64(), Some(head), "{start}");
    fresh.end();
}

/// A stand-in for an agent's own command line (`claude`, `codex`): it
/// notes each call's arguments, a line each.
fn fake_agent(home: &Home, name: &str) -> (PathBuf, PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let bin = home.dir.path().join("agent-bin");
    std::fs::create_dir_all(&bin).unwrap();
    let log = home.dir.path().join(format!("{name}.log"));
    let script = bin.join(name);
    std::fs::write(
        &script,
        format!("#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\n", log.display()),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    (bin, log)
}

#[test]
fn an_agent_is_connected_and_reaches_the_books_by_what_it_was_given() {
    let home = Home::new("connect");
    let out = home.books(&["create", "beta"]);
    assert!(out.status.success(), "{out:?}");
    let (bin, log) = fake_agent(&home, "claude");
    let connect = |agent: &str, path: &Path| {
        home.cmd("connect")
            .args([agent, "--book", "beta"])
            .env("PATH", format!("{}:/usr/bin:/bin", path.display()))
            .stdin(Stdio::null())
            .output()
            .unwrap()
    };
    let out = connect("claude", &bin);
    assert!(out.status.success(), "{out:?}");
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(
        said.contains("Made the agent token of the books beta,"),
        "{said}"
    );
    assert!(
        said.contains("Claude Code reaches the books beta"),
        "{said}"
    );
    let calls = std::fs::read_to_string(&log).unwrap();
    let lines: Vec<&str> = calls.lines().collect();
    assert_eq!(lines[0], "mcp remove --scope user wanderbooks-beta");
    let add = lines[1];
    assert!(
        add.starts_with("mcp add --scope user wanderbooks-beta -- "),
        "{add}"
    );
    assert!(add.contains(" mcp --book beta --root "), "{add}");
    assert!(!said.contains("wb_"), "a token's text was shown");
    // The command it was given reaches the books (with the token kept).
    let given: Vec<&str> = add.split(" -- ").nth(1).unwrap().split(' ').collect();
    let mut c = Command::new(given[0]);
    c.args(&given[1..])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", home.dir.path());
    let mut agent = Agent::from(c);
    agent.initialize();
    let start = agent.call("books.start", json!({})).unwrap();
    assert!(start["head"]["seq"].as_u64().is_some(), "{start}");
    agent.end();
    // Again: the token kept is used, not made anew.
    let out = connect("claude", &bin);
    assert!(out.status.success(), "{out:?}");
    assert!(
        !String::from_utf8_lossy(&out.stdout).contains("Made the agent token"),
        "{out:?}"
    );
    // Codex: its own list, no scope.
    let (bin, log) = fake_agent(&home, "codex");
    let out = connect("codex", &bin);
    assert!(out.status.success(), "{out:?}");
    let calls = std::fs::read_to_string(&log).unwrap();
    assert!(
        calls
            .lines()
            .nth(1)
            .unwrap()
            .starts_with("mcp add wanderbooks-beta -- "),
        "{calls}"
    );
    // An agent that isn't installed: said. (Never where a real one could
    // be found and run: skipped on a machine with one installed system-wide.)
    if ["/usr/local/bin/claude", "/opt/homebrew/bin/claude"]
        .iter()
        .any(|p| Path::new(p).exists())
    {
        return;
    }
    let empty = home.dir.path().join("empty-bin");
    std::fs::create_dir_all(&empty).unwrap();
    let out = home
        .cmd("connect")
        .args(["claude", "--book", "beta"])
        .env("PATH", empty.display().to_string())
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("isn't installed"),
        "{out:?}"
    );
}

#[test]
fn books_are_backed_up_now_from_the_command_line() {
    let home = Home::new("back-up");
    home.create("acme");
    // None set up: said, and failing.
    let out = home.books(&["back-up", "acme"]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("No backups are set up"),
        "{out:?}"
    );
    let disk = home.dir.path().join("disk");
    std::fs::create_dir(&disk).unwrap();
    let mut a = Agent::start(&home, "acme");
    a.initialize();
    a.call(
        "books.set_setting",
        json!({"changes": [{"key": "backup.folder", "value": disk.display().to_string()}],
               "decided_by": "Test Owner"}),
    )
    .unwrap();
    let out = home.books(&["back-up", "acme"]);
    assert!(out.status.success(), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("Backed up to the backup folder"),
        "{out:?}"
    );
    assert!(disk.join("acme.git").join("HEAD").is_file());
    // A folder gone since: said, and --new-disk begins it again.
    std::fs::rename(disk.join("acme.git"), home.dir.path().join("away.git")).unwrap();
    let out = home.books(&["back-up", "acme"]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("holds no backup"),
        "{out:?}"
    );
    let out = home.books(&["back-up", "acme", "--new-disk"]);
    assert!(out.status.success(), "{out:?}");
    assert!(disk.join("acme.git").join("HEAD").is_file());
    a.end();
}

/// The viewer's owner (ledger-cli's `ViewerOwner`, over these ledger-host
/// functions): its hold is a lease of kind app; new books are made by the
/// server; "Close the books" ends every lease, the viewer's hold with it.
/// (A `serve` holds the books first: run in this test's process, a hold
/// that had to start the host would start this test's own program.)
#[test]
fn the_viewers_hold_new_books_and_close_the_books() {
    use ledger_host::commands::{Common, close_books, hold_for_viewer, new_books};
    let home = Home::new("viewer-owner");
    home.create("acme");
    let serving = serve(&home);
    let common = Common {
        root: Some(home.root()),
        knobs: ledger_host::knobs::Knobs::parse(KNOBS).unwrap(),
        ..Default::default()
    };
    let hold = hold_for_viewer(&common, Some("acme".into()));
    assert!(
        wait_for(20, || home
            .lease_line("{\"proto\":1,\"leases\":null}\n")
            .contains("\"app\"")),
        "the viewer's lease never showed"
    );
    assert_eq!(
        new_books(&common, "beta", "USD").unwrap(),
        "Made new books \"beta\" (USD)."
    );
    assert!(
        new_books(&common, "beta", "USD")
            .unwrap_err()
            .contains("already books called")
    );
    let out = home.books(&["list"]);
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("beta  open"),
        "{out:?}"
    );
    let said = close_books(&common).unwrap();
    assert!(said.starts_with("The books are closing"), "{said}");
    // The viewer's hold lets go, for good (closed by the owner).
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(hold.join().is_ok());
    });
    assert_eq!(rx.recv_timeout(Duration::from_secs(30)), Ok(true));
    serving.expect("closed", 30);
    assert!(home.wait_closed(30));
    // Closed: said so.
    assert_eq!(close_books(&common).unwrap(), "The books are closed.");
}

#[test]
fn connect_makes_anew_a_token_whose_file_is_gone_and_says_when_an_agent_refuses() {
    use std::os::unix::fs::PermissionsExt;
    let home = Home::new("connect-lost");
    home.create("acme");
    // The server keeps the books' agent token; its token file is gone.
    std::fs::remove_dir_all(home.run_dir().join("tokens")).unwrap();
    let (bin, _log) = fake_agent(&home, "claude");
    let connect = |path: &Path| {
        home.cmd("connect")
            .args(["claude", "--book", "acme"])
            .env("PATH", format!("{}:/usr/bin:/bin", path.display()))
            .stdin(Stdio::null())
            .output()
            .unwrap()
    };
    let out = connect(&bin);
    assert!(out.status.success(), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("Made the agent token of the books acme,"),
        "{out:?}"
    );
    // The agent's own command refuses: said, with what it said.
    let refusing = home.dir.path().join("refusing-bin");
    std::fs::create_dir_all(&refusing).unwrap();
    let script = refusing.join("claude");
    std::fs::write(
        &script,
        "#!/bin/sh\n[ \"$2\" = add ] && { echo 'no room at the inn' >&2; exit 1; }\nexit 0\n",
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let out = connect(&refusing);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("Claude Code didn't add the books: no room at the inn"),
        "{err}"
    );
}

/// tokens.rs INVARIANT 4, commands.rs INVARIANT 4, bridge.rs INVARIANT
/// 11: `token check` says whether the token file's token is the books'
/// live one, never its text; an unreadable token file is said, by the
/// check and by a bridge, never "no token"; and a token the file can't
/// keep is revoked again.
#[test]
fn token_check_says_where_the_token_file_stands_and_an_unreadable_one_is_said() {
    let home = Home::new("tokencheck");
    let out = home.books(&["create", "acme"]);
    assert!(out.status.success(), "{out:?}");
    let token = |args: &[&str]| {
        let out = home
            .cmd("token")
            .args(args)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        let said = (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).to_string(),
            String::from_utf8_lossy(&out.stderr).to_string(),
        );
        assert!(
            !said.1.contains("wb_") && !said.2.contains("wb_"),
            "{said:?}"
        );
        said
    };
    let check = || token(&["check", "--book", "acme"]);
    let (code, said, _) = check();
    assert_eq!(code, Some(1));
    assert!(said.contains("There is no token \"agent\""), "{said}");
    let (code, _, err) = token(&["create", "--book", "acme"]);
    assert_eq!(code, Some(0), "{err}");
    let (code, said, _) = check();
    assert_eq!(code, Some(0), "{said}");
    assert!(said.contains("is the live one"), "{said}");
    // The one file kept, mode 0600.
    let folder = home.run_dir().join("tokens");
    let item = std::fs::read_dir(&folder)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect::<Vec<_>>();
    assert_eq!(item.len(), 1);
    let item = item[0].clone();
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&item).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    let mut a = Agent::start(&home, "acme");
    a.initialize();
    a.call("books.start", json!({})).unwrap();
    assert!(a.end().is_some());
    // The file holding a token replaced since: not the live one.
    let old = std::fs::read_to_string(&item).unwrap();
    let (code, _, _) = token(&["create", "--book", "acme", "--replace"]);
    assert_eq!(code, Some(0));
    std::fs::write(&item, &old).unwrap();
    let (code, said, _) = check();
    assert_eq!(code, Some(1));
    assert!(
        said.contains("isn't the live one") && said.contains("--replace"),
        "{said}"
    );
    // A token file that can't be read (a folder in its place): a bridge
    // with no token says so, never "make a token"; and a token made then
    // is revoked again.
    std::fs::remove_file(&item).unwrap();
    std::fs::create_dir(&item).unwrap();
    let mut b = Agent::start(&home, "acme");
    b.initialize();
    let err = b.call("books.start", json!({})).unwrap_err();
    assert!(
        err.contains("token file in the data root couldn't be read")
            && err.contains("token itself is fine"),
        "{err}"
    );
    assert!(!err.contains("token create"), "{err}");
    assert!(b.end().is_some());
    let (code, _, err) = check();
    assert_eq!(code, Some(1));
    assert!(err.contains("token file couldn't give"), "{err}");
    let (code, _, err) = token(&["create", "--book", "acme", "--replace"]);
    assert_eq!(code, Some(1));
    assert!(
        err.contains("couldn't keep it") && err.contains("revoked again"),
        "{err}"
    );
    std::fs::remove_dir(&item).unwrap();
    let (_, listed, _) = token(&["list"]);
    assert!(listed.contains("No tokens yet"), "{listed}");
}
