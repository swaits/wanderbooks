//! Starting the server: arguments, where the books and settings are found,
//! and what happens when another writer has the books.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_ledger-mcp");

/// A test's dir, removed when the test ends, pass or panic.
struct TestDir(std::path::PathBuf);

impl std::ops::Deref for TestDir {
    type Target = std::path::Path;
    fn deref(&self) -> &std::path::Path {
        &self.0
    }
}

impl AsRef<std::path::Path> for TestDir {
    fn as_ref(&self) -> &std::path::Path {
        &self.0
    }
}

impl AsRef<std::ffi::OsStr> for TestDir {
    fn as_ref(&self) -> &std::ffi::OsStr {
        self.0.as_os_str()
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn temp(tag: &str) -> TestDir {
    let d = std::env::temp_dir().join(format!("ledger-mcp-start-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    TestDir(d)
}

/// Start the server, initialize, call one tool, and return its text
/// result (or the error text), then end the session.
fn one_call(cmd: &mut Command, tool: &str, args: serde_json::Value) -> String {
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap());
    let mut send = |v: serde_json::Value| writeln!(stdin, "{v}").unwrap();
    send(
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
        "protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}),
    );
    let mut line = String::new();
    out.read_line(&mut line).unwrap();
    send(serde_json::json!({"jsonrpc":"2.0","method":"notifications/initialized","params":{}}));
    send(
        serde_json::json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":tool,"arguments":args}}),
    );
    line.clear();
    out.read_line(&mut line).unwrap();
    drop(stdin);
    let _ = child.wait();
    line
}

#[test]
fn help_names_the_options() {
    let out = Command::new(BIN).arg("--help").output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout) + String::from_utf8_lossy(&out.stderr);
    for opt in ["--data-dir", "--books-toml", "--config", "--http"] {
        assert!(text.contains(opt), "--help lacks {opt}: {text}");
    }
}

/// With no --data-dir the books live in ~/.local/share/wanderbooks.
#[test]
fn the_default_data_dir_is_under_home() {
    let home = temp("home");
    one_call(
        Command::new(BIN).env("HOME", &home),
        "books.chart",
        serde_json::json!({}),
    );
    assert!(
        home.join(".local/share/wanderbooks/journal.jsonl").exists(),
        "no books under {}",
        home.display()
    );
}

/// wanderbooks.toml beside the data folder is read without --books-toml:
/// its receipt threshold (0.00) refuses a 1.00 entry with no receipt,
/// which the default ($25.00) would post.
#[test]
fn the_settings_beside_the_books_are_read() {
    let dir = temp("beside");
    std::fs::write(
        dir.join("wanderbooks.toml"),
        "[books]\nreceipt_threshold = \"0.00\"\n",
    )
    .unwrap();
    let data = dir.join("books-data");
    let mut store = ledger_store::Store::create(&data, ledger_core::Currency::USD).unwrap();
    let cash = store
        .add_account("1000", "Cash", ledger_core::AccountType::Asset, None, false)
        .unwrap();
    let rent = store
        .add_account(
            "6000",
            "Rent",
            ledger_core::AccountType::Expense,
            None,
            false,
        )
        .unwrap();
    let e = store
        .create_entry(chrono::NaiveDate::from_ymd_opt(2026, 3, 1).unwrap(), "x")
        .unwrap();
    let m = ledger_core::Money::new(rust_decimal::Decimal::ONE, ledger_core::Currency::USD);
    store
        .add_line(e, rent, ledger_core::Side::Debit, m)
        .unwrap();
    store
        .add_line(e, cash, ledger_core::Side::Credit, m)
        .unwrap();
    drop(store);
    let reply = one_call(
        Command::new(BIN).arg("--data-dir").arg(&data),
        "books.post_entry",
        serde_json::json!({"entry_id": e, "confirmed": true}),
    );
    assert!(
        reply.contains("receipt_required"),
        "posted without a receipt: {reply}"
    );
}

/// The cash flow roles in wanderbooks.toml beside the data folder are
/// read without --books-toml: its cash account lets cash_flow run with no
/// cash_accounts of its own.
#[test]
fn the_cash_roles_beside_the_books_are_read() {
    let dir = temp("roles");
    std::fs::write(
        dir.join("wanderbooks.toml"),
        "[books]\ncash_accounts = [\"1000\"]\n",
    )
    .unwrap();
    let data = dir.join("books-data");
    let mut store = ledger_store::Store::create(&data, ledger_core::Currency::USD).unwrap();
    store
        .add_account("1000", "Cash", ledger_core::AccountType::Asset, None, false)
        .unwrap();
    drop(store);
    let reply = one_call(
        Command::new(BIN).arg("--data-dir").arg(&data),
        "books.cash_flow",
        serde_json::json!({"start": "2026-01-01", "end": "2026-03-31"}),
    );
    assert!(reply.contains("\"isError\":false"), "{reply}");
}

/// A writer that keeps the books is a conflict: the server gives up with
/// an error (after its ten-second wait), never hangs.
#[test]
fn a_held_lock_is_refused_not_waited_on_forever() {
    let dir = temp("locked");
    let _holder = ledger_store::Store::create(&dir, ledger_core::Currency::USD).unwrap();
    let start = Instant::now();
    let mut child = Command::new(BIN)
        .arg("--data-dir")
        .arg(&dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let status = loop {
        if let Some(s) = child.try_wait().unwrap() {
            break s;
        }
        if start.elapsed() > Duration::from_secs(60) {
            let _ = child.kill();
            panic!("the server waited on a held lock for a minute");
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    assert!(!status.success());
    assert!(
        start.elapsed() >= Duration::from_secs(9),
        "gave up before waiting"
    );
}

/// Books written by a newer wanderbooks: the server won't start on them,
/// and says to update it, not that the books are corrupt (rules.md B-27).
#[test]
fn newer_books_are_refused_as_newer_not_corrupt() {
    let dir = temp("newer");
    drop(ledger_store::Store::create(&dir, ledger_core::Currency::USD).unwrap());
    let mut w = ledger_store::JournalWriter::open(&dir.join(ledger_store::JOURNAL_FILE)).unwrap();
    w.append(
        "journal.format_set",
        serde_json::json!({"format": 3, "features": []}),
    )
    .unwrap();
    // The next line chains onto it, so its bytes are checked (B-27).
    w.append(
        "fx.rate_set",
        serde_json::json!({"currency": "EUR", "date": "2026-10-01", "rate": "1.10"}),
    )
    .unwrap();
    let journal = std::fs::read(dir.join(ledger_store::JOURNAL_FILE)).unwrap();
    let out = Command::new(BIN)
        .arg("--data-dir")
        .arg(&dir)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("written by a newer version of wanderbooks"),
        "{err}"
    );
    assert!(err.contains("Update wanderbooks"), "{err}");
    assert!(!err.contains("corrupt"), "{err}");
    assert_eq!(
        std::fs::read(dir.join(ledger_store::JOURNAL_FILE)).unwrap(),
        journal
    );
}

/// `--import-root` is gone (files come in as tool input, P2 slice 5): a
/// server started with it stops at once, as for any unknown argument,
/// and never serves as if it had read from that folder.
#[test]
fn the_import_root_flag_is_refused_as_unknown() {
    let dir = temp("import-root");
    let out = Command::new(BIN)
        .arg("--data-dir")
        .arg(dir.join("books-data"))
        .arg("--import-root")
        .arg(dir.join("Downloads"))
        .env("XDG_CONFIG_HOME", dir.join("config"))
        .env("XDG_RUNTIME_DIR", dir.join("config"))
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("unknown argument: --import-root"), "{err}");
    assert!(!err.contains("serving"), "{err}");
    assert!(!dir.join("books-data").exists(), "opened the books");
}
