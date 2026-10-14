//! Concurrency (rank 2): the viewer reads while the agent writes.
//!
//! The viewer ([`Snapshot`]) follows the live journal of a [`Store`] that
//! another thread (in production, another process) is writing as fast as
//! it can. Its own spec (snapshot.rs): a line still being appended is left
//! for the next refresh, never treated as corruption; it never writes.
//!
//! Oracle, at every refresh: the snapshot's books are whole (the trial
//! balance balances, every posted entry is valid), they never go backward,
//! and the writer's database is never seen to differ from the journal
//! (behind is fine: the writer applies right after journaling). At the end
//! the snapshot equals the writer, every part of the ledger, attachments
//! included.

use chrono::NaiveDate;
use ledger_core::{AccountType, Currency, Money, Side};
use ledger_store::{DiskCheck, Snapshot, Store};
use rust_decimal::Decimal;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier};

fn test_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("wanderbooks-conc-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

const ENTRIES: u32 = 400;

#[test]
fn a_viewer_reading_while_the_books_are_written_sees_only_whole_books() {
    let dir = test_dir("viewer");
    let mut store = Store::create(&dir, Currency::USD).unwrap();
    let cash = store
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    let sales = store
        .add_account("4000", "Sales", AccountType::Revenue, None, false)
        .unwrap();
    let mut snapshot = Snapshot::load(&dir).unwrap();

    let done = Arc::new(AtomicBool::new(false));
    let start = Arc::new(Barrier::new(2));
    let writer = {
        let (done, start) = (done.clone(), start.clone());
        std::thread::spawn(move || {
            start.wait();
            for n in 0..ENTRIES {
                let day = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()
                    + chrono::Days::new(u64::from(n % 365));
                let e = store.create_entry(day, &format!("sale {n}")).unwrap();
                let m = Money::new(Decimal::new(i64::from(n) * 100 + 1, 2), Currency::USD);
                store.add_line(e, cash, Side::Debit, m).unwrap();
                store.add_line(e, sales, Side::Credit, m).unwrap();
                if n % 3 == 0 {
                    // Leave every third a draft for a while: drafts are
                    // journaled too, and must read back whole.
                    continue;
                }
                if n % 10 == 1 {
                    store
                        .attach_receipt(
                            Some(e),
                            &format!("r{n}.pdf"),
                            "application/pdf",
                            format!("receipt {n}").as_bytes(),
                            None,
                        )
                        .unwrap();
                }
                store.post_entry(e).unwrap();
            }
            done.store(true, Ordering::SeqCst);
            store
        })
    };

    start.wait();
    let (mut last_seq, mut last_posted, mut refreshes) = (0u64, 0usize, 0u64);
    loop {
        let finished = done.load(Ordering::SeqCst);
        snapshot.refresh().unwrap();
        refreshes += 1;
        let l = snapshot.ledger();
        let tb = ledger_core::reports::trial_balance(l);
        assert_eq!(tb.total_debits, tb.total_credits, "refresh {refreshes}");
        for e in l.posted_entries() {
            assert!(
                e.validate(l.chart()).is_ok(),
                "entry {} at refresh {refreshes}",
                e.id()
            );
        }
        let posted = l.posted_entries().count();
        assert!(
            snapshot.journal_seq() >= last_seq,
            "the journal went backward"
        );
        assert!(posted >= last_posted, "posted entries went backward");
        (last_seq, last_posted) = (snapshot.journal_seq(), posted);
        match snapshot.check_disk() {
            DiskCheck::Matches | DiskCheck::Behind { .. } | DiskCheck::Ahead { .. } => {}
            other => panic!("refresh {refreshes}: the database {other:?}"),
        }
        if finished {
            break;
        }
    }
    let store = writer.join().unwrap();
    snapshot.refresh().unwrap();
    let parts = store.ledger().differs_from(snapshot.ledger());
    assert!(
        parts.is_empty(),
        "the viewer differs from the writer in {parts:?}"
    );
    assert_eq!(
        snapshot.attachments(None).unwrap().len(),
        store.list_attachments(None).unwrap().len()
    );
    assert_eq!(snapshot.check_disk(), DiskCheck::Matches);
    assert!(refreshes > 1, "the reader never overlapped the writer");
    if std::env::var("CONC_STATS").is_ok() {
        eprintln!("REFRESHES {refreshes}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
