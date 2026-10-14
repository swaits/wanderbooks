//! Starting the server: arguments, where the books and settings are found,
//! and what happens when another writer has the books.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_ledger-mcp");

fn temp(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("ledger-mcp-start-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
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
