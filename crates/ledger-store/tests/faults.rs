//! Fault injection (rank 1): the store under failing I/O.
//!
//! A full disk is simulated without any seam in the code: the test re-runs
//! this binary as a child process under a shell that caps the file size
//! (`ulimit -f`) and ignores SIGXFSZ, so writes past the cap fail with
//! EFBIG exactly as a full disk fails a write. The child appends until a
//! write fails; the parent then checks the books.
//!
//! Its own file: the child mode needs a test binary of its own to re-run.
//!
//! Unix only: the cap is `ulimit -f` under a Unix shell and the signal is
//! SIGXFSZ; Windows has neither (its `bash` is WSL's, when one exists).
//! The books' own full-disk behavior is the same code on every OS.
#![cfg(unix)]

use chrono::NaiveDate;
use ledger_core::{AccountType, Currency, Money, Side};
use ledger_store::Store;
use rust_decimal::Decimal;
use std::path::{Path, PathBuf};
use std::process::Command;

/// A test's dir, removed when the test ends, pass or panic.
struct TestDir(PathBuf);

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

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn test_dir(name: &str) -> TestDir {
    let dir = std::env::temp_dir().join(format!("wanderbooks-fault-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    TestDir(dir)
}

fn post_one(store: &mut Store, n: u32) -> Result<(), ledger_store::StoreError> {
    let cash = store.ledger().chart().find_by_code("1000").unwrap().id;
    let sales = store.ledger().chart().find_by_code("4000").unwrap().id;
    let day = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap() + chrono::Days::new(u64::from(n % 300));
    let e = store.create_entry(day, &format!("sale {n}"))?;
    let m = Money::new(Decimal::new(i64::from(n) * 100 + 1, 2), Currency::USD);
    store.add_line(e, cash, Side::Debit, m)?;
    store.add_line(e, sales, Side::Credit, m)?;
    store.post_entry(e)
}

/// The child: append until a write fails, then report how far it got.
#[test]
fn child_append_until_the_disk_is_full() {
    let Ok(dir) = std::env::var("WB_FAULT_DIR") else {
        return; // Only runs as the child.
    };
    let mut store = Store::open(Path::new(&dir)).expect("child opens the books");
    for n in 0..100_000 {
        if let Err(e) = post_one(&mut store, n) {
            println!("CHILD write failed after {n} entries: {e}");
            // Keep going a little: later commands must fail cleanly too,
            // and must not write after a partial line.
            for k in 0..5 {
                let _ = post_one(&mut store, n + 1 + k);
            }
            return;
        }
    }
    println!("CHILD never failed");
}

/// Oracle (rank 1, docs/receipt-store.md: a failed write leaves nothing
/// behind; R-11): when the disk fills mid-command, the command fails, the
/// journal holds only whole lines, and the books open and verify with
/// everything that was acknowledged. SQLite's write-ahead log is the
/// first file to fill here; the journal-side case is tested on its own
/// below.
fn full_disk_case(name: &str, drafts: u32) -> String {
    let dir = test_dir(name);
    {
        let mut store = Store::create(&dir, Currency::USD).unwrap();
        store
            .add_account("1000", "Cash", AccountType::Asset, None, false)
            .unwrap();
        store
            .add_account("4000", "Sales", AccountType::Revenue, None, false)
            .unwrap();
        for n in 0..5 {
            post_one(&mut store, n).unwrap();
        }
        let cash = store.ledger().chart().find_by_code("1000").unwrap().id;
        for n in 0..drafts {
            let e = store
                .create_entry(
                    NaiveDate::from_ymd_opt(2026, 6, 1).unwrap(),
                    &format!("draft {n} {}", "x".repeat(200)),
                )
                .unwrap();
            let m = Money::new(Decimal::new(100, 2), Currency::USD);
            store.add_line(e, cash, Side::Debit, m).unwrap();
        }
    }
    let journal = dir.join(ledger_store::JOURNAL_FILE);
    // The cap applies to every file the child writes, so it must clear
    // the biggest one with a little room: whichever grows past it first
    // fails, as on a full disk. bash counts ulimit -f in 1024-byte blocks.
    let biggest = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok()?.metadata().ok())
        .filter(|m| m.is_file())
        .map(|m| m.len())
        .max()
        .unwrap();
    let blocks = biggest / 1024 + 3;
    let exe = std::env::current_exe().unwrap();
    let out = Command::new("bash")
        .arg("-c")
        .arg(format!(
            "trap '' XFSZ; ulimit -f {blocks}; exec \"$0\" --exact child_append_until_the_disk_is_full --nocapture --test-threads=1"
        ))
        .arg(&exe)
        .env("WB_FAULT_DIR", dir.as_os_str())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let child = stdout
        .lines()
        .find(|l| l.contains("CHILD"))
        .unwrap_or("no child line")
        .to_string();
    assert!(
        child.contains("CHILD write failed"),
        "the cap never bit: {child}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let bytes = std::fs::read(&journal).unwrap();
    assert_eq!(
        bytes.last(),
        Some(&b'\n'),
        "the journal ends mid-line ({child})"
    );
    let store = Store::open(&dir).unwrap_or_else(|e| panic!("the books don't open ({child}): {e}"));
    store.verify().unwrap();
    assert!(store.ledger().posted_entries().count() >= 5);
    child
}

#[test]
fn a_full_disk_mid_command_leaves_the_books_whole() {
    let db_first = full_disk_case("full-disk-db", 0);
    assert!(
        db_first.contains("database error"),
        "expected the database to fill first: {db_first}"
    );
}

/// The child for the journal-side case: the journal writer alone (no
/// SQLite, whose write-ahead log otherwise fills first), appending until a
/// write fails, then a few more appends.
#[test]
fn child_journal_appends_until_the_disk_is_full() {
    let Ok(path) = std::env::var("WB_FAULT_JOURNAL") else {
        return;
    };
    let mut w =
        ledger_store::JournalWriter::open(Path::new(&path)).expect("child opens the journal");
    for n in 0..1_000_000u64 {
        if let Err(e) = w.append(
            "test.line",
            serde_json::json!({ "n": n, "pad": "x".repeat(300) }),
        ) {
            println!("CHILD write failed after {n} lines: {e}");
            for k in 0..5 {
                let r = w.append("test.line", serde_json::json!({ "n": n + 1 + k }));
                println!(
                    "CHILD retry {k}: {}",
                    if r.is_ok() { "ok" } else { "failed" }
                );
            }
            return;
        }
    }
}

/// Oracle (rank 1): a journal append that fails partway (a full disk)
/// leaves no partial line: the journal holds only whole lines and its
/// hash chain verifies.
#[test]
fn a_full_disk_mid_append_leaves_only_whole_journal_lines() {
    let dir = test_dir("journal-full");
    let path = dir.join(ledger_store::JOURNAL_FILE);
    {
        let mut w = ledger_store::JournalWriter::open(&path).unwrap();
        for n in 0..20u64 {
            w.append("test.line", serde_json::json!({ "n": n }))
                .unwrap();
        }
    }
    let blocks = std::fs::metadata(&path).unwrap().len() / 1024 + 8;
    let exe = std::env::current_exe().unwrap();
    let out = Command::new("bash")
        .arg("-c")
        .arg(format!(
            "trap '' XFSZ; ulimit -f {blocks}; exec \"$0\" --exact child_journal_appends_until_the_disk_is_full --nocapture --test-threads=1"
        ))
        .arg(&exe)
        .env("WB_FAULT_JOURNAL", &path)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("CHILD write failed"),
        "the cap never bit:\n{stdout}"
    );
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(bytes.last(), Some(&b'\n'), "a partial line was left behind");
    let lines = ledger_store::journal::read_and_verify_journal(&path)
        .unwrap_or_else(|e| panic!("the journal doesn't verify: {e}"));
    assert!(lines.len() >= 20);
}
