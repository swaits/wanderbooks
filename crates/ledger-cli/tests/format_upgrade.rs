//! The format upgrade from end to end (plan/served.md 3.9, D-39), through
//! the one executable: books an older wanderbooks wrote, in a data root,
//! opened by the books server of this version. Oracle: the plan's
//! `format_upgrade_waits_for_consent` (they open on the old format until
//! the owner consents; a backup with the keys is made first; no MCP tool
//! can consent; declining leaves them openable) and D-39 (asked at every
//! open until given; books.start puts it first).
//!
//! ```text
//! INVARIANTS:
//!   1. Waiting books open and are served; books.start says so first, and
//!      a change that needs the new format is refused, journaling nothing;
//!      a change that needs nothing new is made.
//!   2. No tool consents: none is offered, and none moves the format.
//!   3. The owner's commands ask at every open, at a terminal; elsewhere
//!      they say so. Declining, or a wrong passphrase, changes nothing.
//!   4. With the passphrase: a backup of the books as they were, keys in
//!      it, then the whole format in one line; the agent's session goes on
//!      and the change is made; the books verify, and reopen upgraded.
//! ```
#![cfg(target_os = "linux")]

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Short timings for tests, and agents' tokens kept in files in the test
/// root, not this computer's keyring.
const KNOBS: &str = "close_after=2500,backoff=50,exe_poll=100,init_wait=5000,queue_wait=30000,file_keyring=1,backup_quiet=300";

/// The owner passphrase the tests set.
const PASS: &str = "correct horse battery";

/// A home with a data root (`setup`) and its owner passphrase.
struct Home {
    dir: tempfile::TempDir,
}

impl Home {
    fn new(tag: &str) -> Self {
        let dir = tempfile::Builder::new()
            .prefix(&format!("wb-upgrade-{tag}-"))
            .tempdir()
            .unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_wanderbooks"))
            .args(["setup", "--home"])
            .arg(dir.path())
            .env_remove("XDG_DATA_HOME")
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        let home = Self { dir };
        ledger_store::passphrase::set(&home.root().join("auth"), PASS, None).unwrap();
        home
    }

    fn root(&self) -> PathBuf {
        self.dir.path().join(".local/share/wanderbooks")
    }

    fn book_dir(&self, id: &str) -> PathBuf {
        self.root().join("books").join(id)
    }

    fn run_dir(&self) -> PathBuf {
        self.root().join("run")
    }

    /// The words of `mode` with the root and the test knobs.
    fn words(&self, mode: &str) -> Vec<String> {
        vec![
            mode.to_string(),
            "--root".into(),
            self.root().display().to_string(),
            "--test-knobs".into(),
            KNOBS.into(),
        ]
    }

    /// The program, with the test home's environment alone.
    fn cmd(&self, mode: &str) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_wanderbooks"));
        c.args(self.words(mode))
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", self.dir.path());
        c
    }

    /// `books ARGS`, not at a terminal.
    fn books(&self, args: &[&str]) -> std::process::Output {
        let mut c = self.cmd("books");
        c.args(args).stdin(Stdio::null());
        c.output().unwrap()
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
                out.push(pid);
            }
        }
        out
    }

    /// `words` run at a terminal (a pseudo-terminal, through util-linux
    /// `script`), typing `lines` one at a time, each once it has been
    /// asked for (a prompt ending ": " or "] "): its exit code and what
    /// the terminal showed.
    fn at_terminal(&self, words: &[String], lines: &[&str]) -> (i32, String) {
        let command = std::iter::once(env!("CARGO_BIN_EXE_wanderbooks").to_string())
            .chain(words.iter().cloned())
            .collect::<Vec<_>>()
            .join(" ");
        let mut child = Command::new("script")
            .args(["-qefc", &command, "/dev/null"])
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", self.dir.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("util-linux script");
        let mut stdin = child.stdin.take().unwrap();
        let shown = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
        let mut stdout = child.stdout.take().unwrap();
        let reader = {
            let shown = shown.clone();
            std::thread::spawn(move || {
                use std::io::Read;
                let mut buf = [0u8; 4096];
                while let Ok(n) = stdout.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    shown.lock().unwrap().extend_from_slice(&buf[..n]);
                }
            })
        };
        let prompts = |shown: &[u8]| {
            String::from_utf8_lossy(shown)
                .split(['\r', '\n'])
                .filter(|l| l.ends_with(": ") || l.ends_with("] "))
                .count()
        };
        for (i, line) in lines.iter().enumerate() {
            let deadline = Instant::now() + Duration::from_secs(120);
            while prompts(&shown.lock().unwrap()) <= i {
                if child.try_wait().unwrap().is_some() || Instant::now() > deadline {
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            // It turns its echo off just after asking for a passphrase.
            std::thread::sleep(Duration::from_millis(150));
            let _ = stdin.write_all(format!("{line}\n").as_bytes());
        }
        let deadline = Instant::now() + Duration::from_secs(300);
        while child.try_wait().unwrap().is_none() {
            assert!(Instant::now() < deadline, "it never ended");
            std::thread::sleep(Duration::from_millis(50));
        }
        drop(stdin);
        let out = child.wait_with_output().unwrap();
        let _ = reader.join();
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&shown.lock().unwrap()),
            String::from_utf8_lossy(&out.stderr)
        );
        (out.status.code().unwrap_or(-1), text)
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = self
            .cmd("close")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
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
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// `serve`, holding the books open until dropped.
fn serve(home: &Home) -> Running {
    let held = Running::start(home.cmd("serve"));
    held.expect("The books are open", 60);
    held
}

/// An agent: the bridge over stdio, spoken to as an MCP client.
struct Agent {
    run: Running,
    next: u64,
}

impl Agent {
    fn start(home: &Home, book: &str) -> Self {
        let mut c = home.cmd("mcp");
        c.args(["--book", book]);
        let mut a = Self {
            run: Running::start(c),
            next: 1,
        };
        a.request(
            "initialize",
            json!({"protocolVersion": "2025-06-18", "capabilities": {},
                   "clientInfo": {"name": "test", "version": "0"}}),
            30,
        );
        a.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        a
    }

    fn send(&mut self, message: Value) {
        let stdin = self.run.child.stdin.as_mut().unwrap();
        writeln!(stdin, "{message}").unwrap();
        stdin.flush().unwrap();
    }

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
        }
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
}

/// Books `id` in the root as an older version wrote them (journal format
/// 1), with a chart, an entry and their vault key, before any server ran.
fn old_books(home: &Home, id: &str) -> PathBuf {
    use ledger_core::{AccountType, Currency, Money, Side};
    use std::os::unix::fs::PermissionsExt;
    let dir = home.book_dir(id);
    std::fs::create_dir(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut s = ledger_store::Store::create_in_format(
        &dir,
        Currency::USD,
        &ledger_store::JournalFormat::default(),
    )
    .unwrap();
    let cash = s
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    let cap = s
        .add_account("3000", "Capital", AccountType::Equity, None, false)
        .unwrap();
    let e = s
        .create_entry(chrono_date(2026, 3, 1), "Capital in")
        .unwrap();
    let m = Money::new(500.into(), Currency::USD);
    s.add_line(e, cash, Side::Debit, m).unwrap();
    s.add_line(e, cap, Side::Credit, m).unwrap();
    s.post_entry(e).unwrap();
    drop(s);
    let keys = dir.join("keys");
    std::fs::create_dir(&keys).unwrap();
    std::fs::set_permissions(&keys, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::write(keys.join("legacy.key"), "ab".repeat(32)).unwrap();
    std::fs::set_permissions(
        keys.join("legacy.key"),
        std::fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    dir
}

fn chrono_date(y: i32, m: u32, d: u32) -> chrono::NaiveDate {
    chrono::NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

fn journal(dir: &Path) -> Vec<u8> {
    std::fs::read(dir.join("journal.jsonl")).unwrap()
}

fn format_lines(dir: &Path) -> usize {
    String::from_utf8(journal(dir))
        .unwrap()
        .lines()
        .filter(|l| l.contains(r#""type":"journal.format_set""#))
        .count()
}

/// git, on the books' own backup repository: what it said.
fn archive_git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg(format!("--git-dir={}", dir.join("archive.git").display()))
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    String::from_utf8(out.stdout).unwrap()
}

fn a_setting() -> Value {
    json!({"changes": [{"key": "company.name", "value": "Fernhill Labs LLC"}],
           "decided_by": "Avery Quill"})
}

/// plan/served.md 3.9's `format_upgrade_waits_for_consent`, and D-39.
#[test]
fn format_upgrade_waits_for_consent() {
    let home = Home::new("consent");
    let dir = old_books(&home, "acme");
    let before = journal(&dir);
    // INVARIANT 3 (not at a terminal): a command that opens the books
    // says they wait, and how to agree; it asks nothing.
    let out = home
        .cmd("token")
        .args(["create", "--book", "acme"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("wait for a format upgrade") && err.contains("wanderbooks books upgrade acme"),
        "{err}"
    );
    let _held = serve(&home);
    let out = home.books(&["list"]);
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("acme  open, waiting for a format upgrade"),
        "{out:?}"
    );
    let doctor = home.cmd("doctor").stdin(Stdio::null()).output().unwrap();
    assert!(
        String::from_utf8_lossy(&doctor.stdout).contains("waiting for a format upgrade"),
        "{doctor:?}"
    );
    // And which database file the books use (one for each schema).
    assert!(
        String::from_utf8_lossy(&doctor.stdout)
            .contains(&format!("database: {}", ledger_store::db_file())),
        "{doctor:?}"
    );

    // INVARIANTS 1 and 2: the agent's view.
    let mut a = Agent::start(&home, "acme");
    let start = a.call("books.start", json!({})).unwrap();
    assert_eq!(start["format_upgrade"]["guard"], "format_upgrade_pending");
    assert_eq!(start["format_upgrade"]["format"], 1);
    assert_eq!(start["next_steps"][0]["tool"], "books.handbook");
    assert_eq!(start["next_steps"][0]["args"]["section"], "format-upgrade");
    let why = start["next_steps"][0]["why"].as_str().unwrap();
    assert!(why.starts_with("Tell the human first"), "{why}");
    let section = a
        .call("books.handbook", json!({"section": "format-upgrade"}))
        .unwrap();
    assert!(
        section["section"]["text"]
            .as_str()
            .unwrap()
            .contains("Only the owner can agree"),
        "{section}"
    );
    let refused = a.call("books.set_setting", a_setting()).unwrap_err();
    assert!(refused.contains("format_upgrade_pending"), "{refused}");
    assert!(refused.contains("no tool can"), "{refused}");
    assert_eq!(journal(&dir), before, "a refusal journals nothing");
    // What needs nothing new is made.
    a.call(
        "books.add_account",
        json!({"code": "6000", "name": "Supplies", "account_type": "expense"}),
    )
    .unwrap();
    let tools = a.request("tools/list", json!({}), 30);
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert!(names.len() > 50, "{names:?}");
    for name in &names {
        assert!(
            !name.contains("upgrade") && !name.contains("consent"),
            "a tool that could consent: {name}"
        );
    }
    assert_eq!(format_lines(&dir), 0, "no tool moved the format");

    // INVARIANT 3: the go-ahead. Only at a terminal; a wrong passphrase,
    // or none, changes nothing.
    let out = home.books(&["upgrade", "acme"]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("typed at a terminal"),
        "{out:?}"
    );
    let answer = home.lease_line(&format!(
        "{}\n",
        json!({"proto": 1, "intent": {"consent_format_upgrade":
            {"book": "acme", "passphrase": "not the passphrase at all"}}})
    ));
    assert!(answer.contains("isn't the owner passphrase"), "{answer}");
    let mut words = home.words("books");
    words.extend(["upgrade".into(), "acme".into()]);
    let (code, shown) = home.at_terminal(&words, &[""]);
    assert_eq!(code, 0, "{shown}");
    assert!(shown.contains("Not upgraded"), "{shown}");
    let wrong = "not the passphrase at all";
    let (code, shown) = home.at_terminal(&words, &[wrong]);
    assert_eq!(code, 1, "{shown}");
    assert!(
        shown.contains("error: not upgraded: that isn't the owner passphrase"),
        "{shown}"
    );
    assert!(!shown.contains(wrong), "the passphrase was shown: {shown}");
    assert!(
        !dir.join("archive.git").exists(),
        "no backup before consent"
    );
    assert_eq!(format_lines(&dir), 0);
    // Asked at every open until given (D-39): declined twice, asked twice.
    let mut list = home.words("books");
    list.push("list".into());
    for _ in 0..2 {
        let (code, shown) = home.at_terminal(&list, &["n"]);
        assert_eq!(code, 0, "{shown}");
        assert!(shown.contains("Upgrade them now?"), "{shown}");
        assert!(shown.contains("Not now"), "{shown}");
        assert!(shown.contains("acme  open, waiting"), "{shown}");
    }
    // Never asked where the question can't be seen: at a terminal, but
    // its output going to a file (or a pipe), the command only notes it on
    // the terminal, asks nothing, and its output is its own.
    let listed = home.dir.path().join("listed.txt");
    let mut to_file = list.clone();
    to_file.extend([">".to_string(), listed.display().to_string()]);
    let (code, shown) = home.at_terminal(&to_file, &["n"]);
    assert_eq!(code, 0, "{shown}");
    assert!(shown.contains("note: the books \"acme\" wait"), "{shown}");
    assert!(!shown.contains("Upgrade them now?"), "{shown}");
    assert_eq!(
        std::fs::read_to_string(&listed).unwrap(),
        "acme  open, waiting for a format upgrade (`wanderbooks books upgrade acme`)\n"
    );
    assert_eq!(format_lines(&dir), 0);
    let before = journal(&dir);

    // INVARIANT 4: given, with the passphrase.
    let (code, shown) = home.at_terminal(&words, &[PASS]);
    assert_eq!(code, 0, "{shown}");
    assert!(!shown.contains(PASS), "the passphrase was shown: {shown}");
    assert!(shown.contains("Backed up first"), "{shown}");
    assert!(
        shown.contains("Upgraded the books \"acme\" to this version's journal format"),
        "{shown}"
    );
    let after = journal(&dir);
    assert!(after.starts_with(&before));
    assert_eq!(format_lines(&dir), 1, "the format moved once");
    let backup = archive_git(&dir, &["rev-parse", "HEAD"]);
    assert_eq!(
        archive_git(&dir, &["show", &format!("{}:journal.jsonl", backup.trim())]).as_bytes(),
        before.as_slice(),
        "the backup holds the books as they were"
    );
    assert_eq!(
        archive_git(
            &dir,
            &["show", &format!("{}:keys/legacy.key", backup.trim())]
        ),
        "ab".repeat(32),
        "and their key"
    );
    // The agent's session goes on: nothing waits, and the change is made.
    let start = a.call("books.start", json!({})).unwrap();
    assert!(start.get("format_upgrade").is_none(), "{start}");
    assert_ne!(start["next_steps"][0]["args"]["section"], "format-upgrade");
    a.call("books.set_setting", a_setting()).unwrap();
    let verified = a.call("books.verify", json!({})).unwrap();
    assert_eq!(verified["database"], ledger_store::db_file(), "{verified}");
    assert!(verified.get("other_databases").is_none(), "{verified}");
    let out = home.books(&["list"]);
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("acme  open\n"),
        "{out:?}"
    );
    assert!(!String::from_utf8_lossy(&out.stderr).contains("format upgrade"));
    // Asked again: nothing to do.
    let (code, shown) = home.at_terminal(&words, &[]);
    assert_eq!(code, 0, "{shown}");
    assert!(
        shown.contains("already in this version's journal format"),
        "{shown}"
    );
    assert_eq!(format_lines(&dir), 1);
}

/// Copy the tree `from` into `to`, private as the server keeps a book
/// (folders 0700, files 0600).
fn copy_private(from: &Path, to: &Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(to).unwrap();
    std::fs::set_permissions(to, std::fs::Permissions::from_mode(0o700)).unwrap();
    for entry in std::fs::read_dir(from).unwrap().flatten() {
        let (src, dst) = (entry.path(), to.join(entry.file_name()));
        if entry.file_type().unwrap().is_dir() {
            copy_private(&src, &dst);
        } else {
            std::fs::copy(&src, &dst).unwrap();
            std::fs::set_permissions(&dst, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
    }
}

/// The cutover of books P2 served, from end to end (the P3 data-safety
/// review, 2026-10-21). `tests/fixtures/p2-served` is a book the P2
/// executable itself wrote (9c99a0cf, the revision before P3; fictional
/// throughout): 470 journal lines through its MCP tools (settings, which
/// declared format 2 with no features, a chart, members, vendors, two TINs
/// in its vault with its key, 64 entries with receipts, an asset and ten
/// months of its depreciation, a schedule, a closed quarter), and P2's
/// projection `wanderbooks.db` at schema 12 as P2 left it.
///
/// ```text
/// INVARIANTS:
///   1. Before consent: the books open and are served, waiting; P2's
///      database is never written; what P2 can read is written, and every
///      write P2 would refuse or misread is refused, journaling nothing
///      (a setting key, a tax category, a depreciation catch-up, a members
///      close counted from the count's start, an invoice's or bill's line
///      naming its project).
///   2. The consent backup holds the books exactly as they were, keys and
///      vault too; the upgrade is one line after P2's, byte for byte.
///   3. After it: the same figures, and every refused write is made; the
///      books verify; P2's database is still byte for byte.
/// ```
#[test]
fn books_p2_served_cut_over_whole() {
    let home = Home::new("p2-served");
    let dir = home.book_dir("acme");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/p2-served");
    copy_private(&fixture, &dir);
    let p2_db = std::fs::read(dir.join(ledger_store::LEGACY_DB_FILE)).unwrap();
    let p2_journal = journal(&dir);
    let _held = serve(&home);
    let out = home
        .cmd("token")
        .args(["create", "--book", "acme"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let mut a = Agent::start(&home, "acme");
    let start = a.call("books.start", json!({})).unwrap();
    assert_eq!(start["format_upgrade"]["format"], 2, "{start}");
    let verified = a.call("books.verify", json!({})).unwrap();
    assert_eq!(verified["status"], "ok", "{verified}");
    assert_eq!(verified["events"], 470);
    assert_eq!(verified["database"], ledger_store::db_file());
    assert_eq!(verified["other_databases"], json!(["wanderbooks.db"]));
    let figures = |a: &mut Agent| {
        [
            a.call("books.trial_balance", json!({"as_of": "2025-12-31"})),
            a.call(
                "books.profit_and_loss",
                json!({"start": "2025-01-01", "end": "2025-12-31"}),
            ),
            a.call("books.balance_sheet", json!({"as_of": "2025-12-31"})),
        ]
        .map(Result::unwrap)
    };
    let before = figures(&mut a);

    // INVARIANT 1: what P2 reads is written; the rest waits.
    a.call(
        "books.add_account",
        json!({"code": "6600", "name": "Travel", "account_type": "expense"}),
    )
    .unwrap();
    let q2 = a
        .call(
            "books.create_period",
            json!({"name": "2025-Q2", "start": "2025-04-01", "end": "2025-06-30"}),
        )
        .unwrap()["period_id"]
        .clone();
    // A project by name alone is a plain class, which P2 reads.
    a.call("books.add_project", json!({"name": "Lighthouse"}))
        .unwrap();
    // An invoice for the send below: a document P2 reads (no new fields),
    // so it is written before the go-ahead; its send waits for it.
    let invoice = a
        .call(
            "books.create_invoice",
            json!({"party_id": 1, "date": "2026-01-05", "due_date": "2026-02-04",
                   "lines": [{"description": "Design", "amount": "400.00"}]}),
        )
        .unwrap()["id"]
        .clone();
    let waiting = journal(&dir);
    let refused: Vec<(&str, Value)> = vec![
        // An invoice's or bill's line naming its project
        // (document_line_projects, rules.md B-37 item 8).
        (
            "books.create_invoice",
            json!({"party_id": 1, "date": "2026-02-10", "due_date": "2026-03-12",
                   "lines": [{"description": "Design", "amount": "500.00", "class": "Lighthouse"}]}),
        ),
        (
            "books.create_bill",
            json!({"party_id": 1, "date": "2026-02-11", "due_date": "2026-03-13",
                   "lines": [{"description": "Drafting", "amount": "120.00", "class": "Lighthouse"}]}),
        ),
        (
            "books.set_setting",
            json!({"decided_by": "Avery Quill",
                   "changes": [{"key": "books.schedule_convention", "value": "monthly_even"}]}),
        ),
        // An invoicing setting (documents_v2, P4b S4.2): an older version
        // refuses the key.
        (
            "books.set_setting",
            json!({"decided_by": "Avery Quill",
                   "changes": [{"key": "invoicing.default_terms", "value": "Net 30"}]}),
        ),
        // A recorded send (invoice.sent, documents_v2, P4b S5): an older
        // version refuses the type.
        (
            "books.mark_sent",
            json!({"invoice": invoice.to_string(), "on": "2026-01-06",
                   "by": "Avery Quill", "recipients": ["billing@fernhill.example"]}),
        ),
        (
            "books.add_account",
            json!({"code": "4900", "name": "Gain on sale", "account_type": "revenue",
                   "tax_category": "form_4797_gain_loss"}),
        ),
        // January and February 2026 not run: March's run would carry them.
        (
            "books.run_depreciation",
            json!({"period": "2026-03", "asset_id": 1}),
        ),
        (
            "books.close_period",
            json!({"period_id": q2, "confirmed": true}),
        ),
    ];
    for (tool, args) in &refused {
        let e = a.call(tool, args.clone()).unwrap_err();
        assert!(e.starts_with("format_upgrade_pending"), "{tool}: {e}");
    }
    assert_eq!(journal(&dir), waiting, "a refusal journals nothing");
    assert!(journal(&dir).starts_with(&p2_journal));
    assert_eq!(
        std::fs::read(dir.join(ledger_store::LEGACY_DB_FILE)).unwrap(),
        p2_db,
        "P2's database is never written"
    );

    // INVARIANT 2: the go-ahead.
    let mut words = home.words("books");
    words.extend(["upgrade".into(), "acme".into()]);
    let (code, shown) = home.at_terminal(&words, &[PASS]);
    assert_eq!(code, 0, "{shown}");
    let after = journal(&dir);
    assert!(after.starts_with(&waiting));
    assert_eq!(
        format_lines(&dir),
        2,
        "P2's declaration, then the upgrade's"
    );
    let backup = archive_git(&dir, &["rev-parse", "HEAD"]);
    assert_eq!(
        archive_git(&dir, &["show", &format!("{}:journal.jsonl", backup.trim())]).as_bytes(),
        waiting.as_slice(),
        "the backup holds the books as they were"
    );
    for kept in ["keys/legacy.key", "vault/tin-1.bin", "vault/tin-3.bin"] {
        let out = Command::new("git")
            .arg(format!("--git-dir={}", dir.join("archive.git").display()))
            .args(["show", &format!("{}:{kept}", backup.trim())])
            .output()
            .unwrap();
        assert_eq!(out.stdout, std::fs::read(dir.join(kept)).unwrap(), "{kept}");
    }

    // INVARIANT 3.
    assert_eq!(figures(&mut a), before, "the upgrade moves no figure");
    for (tool, args) in refused {
        a.call(tool, args.clone())
            .unwrap_or_else(|e| panic!("{tool} after the upgrade: {e}"));
    }
    let verified = a.call("books.verify", json!({})).unwrap();
    assert_eq!(verified["status"], "ok", "{verified}");
    assert_eq!(
        std::fs::read(dir.join(ledger_store::LEGACY_DB_FILE)).unwrap(),
        p2_db,
        "and never after"
    );
}
