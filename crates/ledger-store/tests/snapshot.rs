//! Snapshot: a read-only view that follows a live writer.

use chrono::NaiveDate;
use ledger_core::reports::snapshot;
use ledger_core::{AccountType, Currency, Money, Side};
use ledger_store::{JOURNAL_FILE, Snapshot, Store};
use rust_decimal::Decimal;
use std::io::Write;
use std::path::PathBuf;

fn test_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("wanderbooks-snap-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn usd(n: i64) -> Money {
    Money::new(Decimal::from(n), Currency::USD)
}

fn post(store: &mut Store, day: u32, dr: u64, cr: u64, amount: i64) -> u64 {
    let d = NaiveDate::from_ymd_opt(2026, 10, day).unwrap();
    let e = store.create_entry(d, "x").unwrap();
    store.add_line(e, dr, Side::Debit, usd(amount)).unwrap();
    store.add_line(e, cr, Side::Credit, usd(amount)).unwrap();
    store.post_entry(e).unwrap();
    e
}

#[test]
fn a_snapshot_follows_the_writer_and_never_writes() {
    let dir = test_dir("follow");
    let mut store = Store::create(&dir, Currency::USD).unwrap();
    let cash = store
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    let equity = store
        .add_account("3000", "Equity", AccountType::Equity, None, false)
        .unwrap();
    let e = post(&mut store, 1, cash, equity, 1000);
    store
        .attach_receipt(Some(e), "deposit.png", "image/png", b"png-bytes", None)
        .unwrap();

    let listing = |dir: &std::path::Path| {
        let mut v: Vec<_> = walk(dir);
        v.sort();
        v
    };
    let before = listing(&dir);
    let mut snap = Snapshot::load(&dir).unwrap();
    assert_eq!(listing(&dir), before, "loading writes nothing");
    assert_eq!(snap.journal_seq(), store.journal_seq());
    assert_eq!(
        snapshot(snap.ledger()).unwrap(),
        snapshot(store.ledger()).unwrap()
    );
    let atts = snap.attachments(Some(e)).unwrap();
    assert_eq!(atts.len(), 1);
    assert_eq!(snap.attachment_bytes(atts[0].id).unwrap(), b"png-bytes");

    // The writer keeps going; refresh picks up exactly the new lines.
    assert_eq!(snap.refresh().unwrap(), 0);
    let seq = store.journal_seq();
    post(&mut store, 2, cash, equity, 250);
    let added = snap.refresh().unwrap();
    assert_eq!(added as u64, store.journal_seq() - seq);
    assert_eq!(
        snapshot(snap.ledger()).unwrap(),
        snapshot(store.ledger()).unwrap()
    );
}

#[test]
fn a_half_written_line_waits_for_the_next_refresh() {
    let dir = test_dir("torn");
    let mut store = Store::create(&dir, Currency::USD).unwrap();
    store
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    drop(store);
    let mut snap = Snapshot::load(&dir).unwrap();
    let seq = snap.journal_seq();
    // Simulate the writer mid-append: half a line, no newline yet.
    let journal = dir.join(JOURNAL_FILE);
    let full = std::fs::read(&journal).unwrap();
    let mut store = Store::open(&dir).unwrap();
    store
        .add_account("3000", "Equity", AccountType::Equity, None, false)
        .unwrap();
    drop(store);
    let grown = std::fs::read(&journal).unwrap();
    let new_line = &grown[full.len()..];
    std::fs::write(&journal, &full).unwrap();
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(&journal)
        .unwrap();
    f.write_all(&new_line[..new_line.len() / 2]).unwrap();
    f.flush().unwrap();
    assert_eq!(snap.refresh().unwrap(), 0, "torn tail is not corruption");
    f.write_all(&new_line[new_line.len() / 2..]).unwrap();
    f.flush().unwrap();
    assert_eq!(snap.refresh().unwrap(), 1);
    assert_eq!(snap.journal_seq(), seq + 1);
    assert!(snap.ledger().chart().find_by_code("3000").is_some());
}

#[test]
fn a_rewritten_journal_reloads() {
    let dir = test_dir("rewrite");
    let mut store = Store::create(&dir, Currency::USD).unwrap();
    let cash = store
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    let equity = store
        .add_account("3000", "Equity", AccountType::Equity, None, false)
        .unwrap();
    let journal = dir.join(JOURNAL_FILE);
    let early = std::fs::read(&journal).unwrap();
    post(&mut store, 1, cash, equity, 10);
    drop(store);
    let mut snap = Snapshot::load(&dir).unwrap();
    let later = snap.journal_seq();
    // Restored from an earlier backup: shorter file.
    std::fs::write(&journal, &early).unwrap();
    snap.refresh().unwrap();
    assert!(snap.journal_seq() < later);
    assert!(snap.ledger().posted_entries().next().is_none());
}

fn walk(dir: &std::path::Path) -> Vec<(String, u64)> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).unwrap() {
        let e = e.unwrap();
        let m = e.metadata().unwrap();
        if m.is_dir() {
            out.extend(walk(&e.path()));
        } else {
            out.push((e.path().display().to_string(), m.len()));
        }
    }
    out
}

#[test]
fn a_snapshot_checks_the_writers_database_and_files() {
    use ledger_store::DiskCheck;
    let dir = test_dir("disk");
    let mut store = Store::create(&dir, Currency::USD).unwrap();
    let cash = store
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    let equity = store
        .add_account("3000", "Equity", AccountType::Equity, None, false)
        .unwrap();
    let e = post(&mut store, 1, cash, equity, 100);
    store
        .attach_receipt(Some(e), "r.png", "image/png", b"receipt-bytes", None)
        .unwrap();
    drop(store);
    let snap = Snapshot::load(&dir).unwrap();
    assert_eq!(snap.check_disk(), DiskCheck::Matches);
    assert_eq!(snap.verify_blobs(), (1, vec![]));
    // Someone edits the database behind the journal's back.
    let conn = rusqlite::Connection::open(dir.join(ledger_store::DB_FILE)).unwrap();
    conn.execute(
        "UPDATE accounts SET name = 'Petty cash' WHERE code = '1000'",
        [],
    )
    .unwrap();
    drop(conn);
    match snap.check_disk() {
        DiskCheck::Differs(why) => assert!(why.contains("accounts"), "{why}"),
        other => panic!("drift not seen: {other:?}"),
    }
    // A damaged file is caught too.
    let blob = walk(&dir.join("blobs"))
        .into_iter()
        .map(|(p, _)| p)
        .find(|p| !p.ends_with(".tmp"))
        .unwrap();
    std::fs::write(&blob, b"tampered").unwrap();
    let (ok, bad) = snap.verify_blobs();
    assert_eq!((ok, bad.len()), (0, 1));
}
