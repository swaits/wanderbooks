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

/// Start the server and report whether it refused to start (it never got
/// as far as serving), with its standard error.
fn refuses_to_start(cmd: &mut Command) -> Option<String> {
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // A server that starts waits on stdin; closing it ends the server.
    drop(child.stdin.take());
    let out = child.wait_with_output().unwrap();
    let err = String::from_utf8_lossy(&out.stderr).into_owned();
    (!err.contains("serving over stdio")).then_some(err)
}

/// local_files.rs INVARIANT 0: a named import root that is `/`, the home
/// folder, a folder holding it, a folder holding the books, the data dir
/// (a symlinked --data-dir placed where it really is), the vault or a key
/// file, or a folder inside the books, the data dir or the vault, stops
/// the server at startup, saying why; a folder holding none of them is
/// used.
#[test]
fn an_import_root_holding_the_books_or_the_home_folder_is_refused() {
    let tmp = temp("bad-roots");
    let dir = tmp.canonicalize().unwrap();
    let home = dir.join("home");
    let books = home.join("work/books");
    let data = books.join("books-data");
    let keys = dir.join("keys");
    for d in [&home.join("Downloads"), &books, &keys, &dir.join("config")] {
        std::fs::create_dir_all(d).unwrap();
    }
    std::fs::write(
        books.join("wanderbooks.toml"),
        format!(
            "[vault]\nkey_file = \"{}\"\n",
            keys.join("vault.key").display()
        ),
    )
    .unwrap();
    let link = dir.join("data-link");
    std::os::unix::fs::symlink(&data, &link).unwrap();
    let start = |data: &std::path::Path, root: &std::path::Path| {
        let mut cmd = Command::new(BIN);
        cmd.arg("--data-dir")
            .arg(data)
            .arg("--import-root")
            .arg(root)
            .env("HOME", &home)
            .env("XDG_CONFIG_HOME", dir.join("config"))
            .env("XDG_RUNTIME_DIR", dir.join("config"));
        refuses_to_start(&mut cmd)
    };
    for (root, why) in [
        (std::path::PathBuf::from("/"), "whole filesystem"),
        (home.clone(), "home folder"),
        (dir.clone(), "home folder"),
        (home.join("work"), "holds the books"),
        (books.clone(), "holds the books"),
        (data.clone(), "holds the books"),
        (
            keys.clone(),
            "holds the books, the TIN vault, or a vault key",
        ),
        (
            dir.join("config"),
            "holds the books, the TIN vault, or a vault key",
        ),
    ] {
        let err = start(&data, &root).unwrap_or_else(|| panic!("{} was used", root.display()));
        assert!(
            err.contains("can't be used") && err.contains(why),
            "{}: {err}",
            root.display()
        );
    }
    // A folder inside the books, the data dir or the vault: every file in
    // it would be refused, so the root is.
    for root in [
        books.join("company"),
        data.join("blobs"),
        books.join("vault/inner"),
    ] {
        std::fs::create_dir_all(&root).unwrap();
        let err = start(&data, &root).unwrap_or_else(|| panic!("{} was used", root.display()));
        assert!(
            err.contains("can't be used") && err.contains("inside the books"),
            "{}: {err}",
            root.display()
        );
    }
    // Through a symlinked data dir, the books are where they really are.
    let err = start(&link, &home.join("work")).expect("a root holding the real books");
    assert!(err.contains("holds the books"), "{err}");
    // A folder holding none of them is used.
    assert_eq!(start(&data, &home.join("Downloads")), None);
}

/// local_files.rs INVARIANT 0, the default: an XDG_DOWNLOAD_DIR of
/// "$HOME/" (or "/") never makes the home folder an import root. With no
/// real ~/Downloads there is no root, and an import says so; with one, it
/// is the root, and the rest of the home folder stays out of reach.
#[test]
fn a_download_dir_of_home_never_opens_the_home_folder() {
    let tmp = temp("xdg-home");
    let dir = tmp.canonicalize().unwrap();
    let home = dir.join("home");
    std::fs::create_dir_all(home.join(".ssh")).unwrap();
    std::fs::write(home.join(".ssh/id_ed25519"), "PRIVATE KEY").unwrap();
    let key = home.join(".ssh/id_ed25519");
    let import = |xdg: &str, path: &std::path::Path| {
        one_call(
            Command::new(BIN)
                .arg("--data-dir")
                .arg(dir.join("books/books-data"))
                .env("HOME", &home)
                .env("XDG_DOWNLOAD_DIR", xdg)
                .env("XDG_CONFIG_HOME", dir.join("config"))
                .env("XDG_RUNTIME_DIR", dir.join("config")),
            "books.import_local_file",
            serde_json::json!({"path": path.to_str().unwrap(), "category": "document"}),
        )
    };
    for xdg in ["$HOME/", "$HOME", "/"] {
        let reply = import(xdg, &key);
        assert!(reply.contains("has no import folder"), "{xdg}: {reply}");
    }
    std::fs::create_dir_all(home.join("Downloads")).unwrap();
    std::fs::write(home.join("Downloads/receipt.pdf"), "%PDF a receipt").unwrap();
    for xdg in ["$HOME/", "/"] {
        let reply = import(xdg, &key);
        assert!(
            reply.contains("no such file in the server's import folders"),
            "{xdg}: {reply}"
        );
        assert!(!reply.contains("PRIVATE"), "{reply}");
    }
    let reply = import("$HOME/", &home.join("Downloads/receipt.pdf"));
    assert!(reply.contains("attachment_id"), "{reply}");
}

/// local_files.rs INVARIANT 0, a refused default: a downloads folder
/// that holds the books, or one inside them, is dropped, never fatal
/// (only a named root stops the server): it serves, with no import
/// folder, and an import says so.
#[test]
fn a_default_download_dir_holding_the_books_is_dropped_not_fatal() {
    let tmp = temp("xdg-books");
    let dir = tmp.canonicalize().unwrap();
    let home = dir.join("home");
    let downloads = dir.join("downloads");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(downloads.join("books")).unwrap();
    std::fs::write(downloads.join("receipt.pdf"), "%PDF a receipt").unwrap();
    let reply = one_call(
        Command::new(BIN)
            .arg("--data-dir")
            .arg(downloads.join("books/books-data"))
            .env("HOME", &home)
            .env("XDG_DOWNLOAD_DIR", &downloads)
            .env("XDG_CONFIG_HOME", dir.join("config"))
            .env("XDG_RUNTIME_DIR", dir.join("config")),
        "books.import_local_file",
        serde_json::json!({"path": downloads.join("receipt.pdf").to_str().unwrap(), "category": "document"}),
    );
    assert!(reply.contains("has no import folder"), "{reply}");
    // A downloads folder inside the books dir is dropped the same way.
    let books = dir.join("books");
    std::fs::create_dir_all(books.join("Downloads")).unwrap();
    std::fs::write(books.join("Downloads/receipt.pdf"), "%PDF a receipt").unwrap();
    let reply = one_call(
        Command::new(BIN)
            .arg("--data-dir")
            .arg(books.join("books-data"))
            .env("HOME", &home)
            .env("XDG_DOWNLOAD_DIR", books.join("Downloads"))
            .env("XDG_CONFIG_HOME", dir.join("config"))
            .env("XDG_RUNTIME_DIR", dir.join("config")),
        "books.import_local_file",
        serde_json::json!({"path": books.join("Downloads/receipt.pdf").to_str().unwrap(), "category": "document"}),
    );
    assert!(reply.contains("has no import folder"), "{reply}");
}
