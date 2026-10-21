//! Store tests: the durability story, proven.
//!
//! Round-trip persistence, tamper detection, rebuild-from-journal healing,
//! blob deduplication and hash verification.

use chrono::{Datelike, NaiveDate};
use ledger_core::{AccountType, Currency, Money, Side};
use ledger_store::{JournalWriter, Store, StoreError};
use rust_decimal::Decimal;
use std::path::PathBuf;

fn usd(amount: u64) -> Money {
    Money::new(Decimal::from(amount), Currency::USD)
}

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
    let dir = std::env::temp_dir().join(format!("wanderbooks-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    TestDir(dir)
}

fn build_books(dir: &std::path::Path) -> Store {
    let mut store = Store::create(dir, Currency::USD).unwrap();
    let cash = store
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    let equity = store
        .add_account("3000", "Owner's Equity", AccountType::Equity, None, false)
        .unwrap();
    let revenue = store
        .add_account("4000", "Service Revenue", AccountType::Revenue, None, false)
        .unwrap();
    let rent = store
        .add_account("6000", "Rent Expense", AccountType::Expense, None, false)
        .unwrap();

    let e1 = store
        .create_entry(
            NaiveDate::from_ymd_opt(2026, 1, 2).unwrap(),
            "Owner investment",
        )
        .unwrap();
    store.add_line(e1, cash, Side::Debit, usd(10_000)).unwrap();
    store
        .add_line(e1, equity, Side::Credit, usd(10_000))
        .unwrap();
    store.post_entry(e1).unwrap();

    let e2 = store
        .create_entry(
            NaiveDate::from_ymd_opt(2026, 1, 10).unwrap(),
            "Invoice client",
        )
        .unwrap();
    store.add_line(e2, cash, Side::Debit, usd(4_000)).unwrap();
    store
        .add_line(e2, revenue, Side::Credit, usd(4_000))
        .unwrap();
    store.post_entry(e2).unwrap();

    let e3 = store
        .create_entry(
            NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
            "January rent",
        )
        .unwrap();
    store.add_line(e3, rent, Side::Debit, usd(800)).unwrap();
    store.add_line(e3, cash, Side::Credit, usd(800)).unwrap();
    store.post_entry(e3).unwrap();

    // A receipt attached to the rent entry.
    let (_rid, sha, dedup) = store
        .attach_receipt(
            Some(e3),
            "rent-receipt.png",
            "image/png",
            b"fake-png-bytes",
            Some("January rent receipt"),
        )
        .unwrap();
    assert!(!dedup);
    assert_eq!(sha.len(), 64);

    // A reversal, to exercise the reversal path through the journal.
    store
        .reverse_entry(
            e3,
            NaiveDate::from_ymd_opt(2026, 1, 16).unwrap(),
            "Reverse rent",
        )
        .unwrap();

    store
}

#[test]
fn round_trip_survives_reopen() {
    let dir = test_dir("roundtrip");
    let store = build_books(&dir);
    // Live ledger: rent was posted then reversed → net zero movement on rent.
    assert_eq!(
        store
            .ledger()
            .account_movement(store.ledger().chart().find_by_code("6000").unwrap().id),
        Decimal::from(0)
    );
    drop(store);

    let store = Store::open(&dir).unwrap();
    // Same facts after reopen: cash = 10000 + 4000 = 14000.
    let cash_id = store.ledger().chart().find_by_code("1000").unwrap().id;
    assert_eq!(
        store.ledger().account_movement(cash_id),
        Decimal::from(14_000)
    );
    // Trial balance still balances.
    let tb = ledger_core::reports::trial_balance(store.ledger());
    assert!(tb.balanced());
    assert_eq!(tb.total_debits, Decimal::from(14_000));
    // Receipt survived with its bytes.
    let receipts = store.list_attachments(None).unwrap();
    assert_eq!(receipts.len(), 1);
    assert_eq!(receipts[0].file_name, "rent-receipt.png");
    assert_eq!(
        store.attachment_bytes(receipts[0].id).unwrap(),
        b"fake-png-bytes"
    );
    // Full verification passes.
    store.verify().unwrap();
    assert_eq!(store.verify_blobs().unwrap(), 1);
}

#[test]
fn journal_tamper_is_detected() {
    let dir = test_dir("tamper");
    let store = build_books(&dir);
    drop(store);

    // Flip a byte in the middle of the journal.
    let journal_path = dir.join(ledger_store::JOURNAL_FILE);
    let mut bytes = std::fs::read(&journal_path).unwrap();
    let mid = bytes.len() / 2;
    bytes[mid] = if bytes[mid] == b'X' { b'Y' } else { b'X' };
    std::fs::write(&journal_path, &bytes).unwrap();

    let err = match Store::open(&dir) {
        Ok(_) => panic!("expected the tampered journal to be rejected"),
        Err(e) => e,
    };
    assert!(
        format!("{err:?}").contains("JournalCorrupt"),
        "expected JournalCorrupt, got {err:?}"
    );
}

/// Oracle (rank 1, journal::repair_torn_tail's spec): an append cut off
/// mid-line was never acknowledged, so opening sets it aside, keeps every
/// acknowledged line, puts the repair on the record, and reports it.
#[test]
fn a_torn_tail_is_set_aside_on_open() {
    let dir = test_dir("torntail");
    let store = build_books(&dir);
    let journal = dir.join(ledger_store::JOURNAL_FILE);
    let before = ledger_core::reports::snapshot(store.ledger()).unwrap();
    let seq_before = JournalWriter::open(&journal).unwrap().next_seq() - 1;
    drop(store);

    use std::io::Write;
    let torn = b"{\"seq\": 999, \"trunca";
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(&journal)
        .unwrap();
    f.write_all(torn).unwrap();
    f.sync_all().unwrap();
    drop(f);

    let store = Store::open(&dir).unwrap();
    store.verify().unwrap();
    assert_eq!(
        ledger_core::reports::snapshot(store.ledger()).unwrap(),
        before
    );
    let repairs = store.tail_repairs().unwrap();
    assert_eq!(repairs.len(), 1);
    assert_eq!(repairs[0].1, torn.len() as u64);
    // The cut-off bytes are kept, exactly, beside the journal.
    assert_eq!(std::fs::read(dir.join(&repairs[0].2)).unwrap(), torn);
    // The repair is the one new line, right after the last good one.
    let lines = ledger_store::journal::read_and_verify_journal(&journal).unwrap();
    assert_eq!(lines.len() as u64, seq_before + 1);
    assert_eq!(lines.last().unwrap().line_type, "journal.tail_repaired");
    // Opening again repairs nothing more.
    drop(store);
    let store = Store::open(&dir).unwrap();
    assert_eq!(store.tail_repairs().unwrap().len(), 1);
    store.verify().unwrap();
}

/// A damaged *complete* line is corruption, not a torn tail: opening still
/// refuses it, and the journal is left as it was.
#[test]
fn damage_before_the_last_newline_still_refuses_to_open() {
    let dir = test_dir("tornmid");
    drop(build_books(&dir));
    let journal = dir.join(ledger_store::JOURNAL_FILE);
    let mut raw = std::fs::read(&journal).unwrap();
    let cut = raw.len() - 2; // inside the last complete line
    raw.remove(cut - 5);
    std::fs::write(&journal, &raw).unwrap();
    assert!(Store::open(&dir).is_err());
    assert_eq!(std::fs::read(&journal).unwrap(), raw);
}

/// Power loss at any byte of the last append: whatever prefix reached the
/// disk, the books open with exactly what was acknowledged before it.
#[test]
fn power_loss_at_any_byte_of_the_last_append_is_recovered() {
    let dir = test_dir("tornevery");
    let store = build_books(&dir);
    let journal = dir.join(ledger_store::JOURNAL_FILE);
    let before_bytes = std::fs::read(&journal).unwrap();
    let before = ledger_core::reports::snapshot(store.ledger()).unwrap();
    drop(store);
    // The books as they were on disk before the last append (the database
    // is never ahead of the journal: the line is written first).
    let pre = test_dir("tornevery-pre");
    for e in std::fs::read_dir(&dir).unwrap() {
        let e = e.unwrap();
        if e.file_type().unwrap().is_file() {
            std::fs::copy(e.path(), pre.join(e.file_name())).unwrap();
        }
    }
    let mut store = Store::open(&dir).unwrap();
    store.add_customer("Acme").unwrap();
    drop(store);
    let full = std::fs::read(&journal).unwrap();
    let last = &full[before_bytes.len()..];
    assert_eq!(last.last(), Some(&b'\n'));
    for cut in 1..last.len() {
        let case = test_dir(&format!("tornevery-{cut}"));
        for e in std::fs::read_dir(&pre).unwrap() {
            let e = e.unwrap();
            if e.file_type().unwrap().is_file() {
                std::fs::copy(e.path(), case.join(e.file_name())).unwrap();
            }
        }
        let mut torn = before_bytes.clone();
        torn.extend_from_slice(&last[..cut]);
        std::fs::write(case.join(ledger_store::JOURNAL_FILE), &torn).unwrap();
        let store = Store::open(&case).unwrap_or_else(|e| panic!("cut at {cut}: {e}"));
        store.verify().unwrap();
        assert_eq!(
            ledger_core::reports::snapshot(store.ledger()).unwrap(),
            before,
            "cut at {cut}"
        );
        assert!(store.ledger().list_customers().is_empty(), "cut at {cut}");
        drop(store);
        let _ = std::fs::remove_dir_all(&case);
    }
}

#[test]
fn rebuild_heals_a_lost_database() {
    let dir = test_dir("rebuild");
    let mut store = build_books(&dir);
    let tb_before = ledger_core::reports::trial_balance(store.ledger());
    // Simulate total SQLite loss.
    store.rebuild().unwrap();
    let tb_after = ledger_core::reports::trial_balance(store.ledger());
    assert_eq!(tb_before.total_debits, tb_after.total_debits);
    assert_eq!(tb_before.total_credits, tb_after.total_credits);
    store.verify().unwrap();
    // Attachments survived (metadata re-applied from the journal).
    assert_eq!(store.list_attachments(None).unwrap().len(), 1);
}

#[test]
fn missing_database_heals_on_open() {
    let dir = test_dir("healopen");
    let store = build_books(&dir);
    drop(store);
    std::fs::remove_file(dir.join(ledger_store::db_file())).unwrap();
    // No database at all: open rebuilds the projection from the journal.
    let store = Store::open(&dir).unwrap();
    let tb = ledger_core::reports::trial_balance(store.ledger());
    assert!(tb.balanced());
    store.verify().unwrap();
}

#[test]
fn blob_dedup_and_hash_verification() {
    let dir = test_dir("blobs");
    let mut store = Store::create(&dir, Currency::USD).unwrap();
    let bytes = b"same bytes twice";
    let (_, sha1, dedup1) = store
        .file_document("tax", "w2.pdf", "application/pdf", bytes, None)
        .unwrap();
    let (_, sha2, dedup2) = store
        .file_document("tax", "w2-copy.pdf", "application/pdf", bytes, None)
        .unwrap();
    assert!(!dedup1);
    assert!(dedup2);
    assert_eq!(sha1, sha2);
    // Only one blob row; two attachment rows.
    assert_eq!(store.verify_blobs().unwrap(), 1);
    assert_eq!(store.list_attachments(None).unwrap().len(), 2);

    // Corrupt the blob file on disk → the next read detects it.
    let shard = dir
        .join("blobs")
        .join(&sha1[0..2])
        .join(&sha1[2..4])
        .join(&sha1);
    let mut raw = std::fs::read(&shard).unwrap();
    raw[0] ^= 0xff;
    std::fs::write(&shard, &raw).unwrap();
    let err = match store.verify_blobs() {
        Ok(_) => panic!("expected the corrupted blob to be detected"),
        Err(e) => e,
    };
    assert!(format!("{err:?}").contains("CorruptBlob"));
}

#[test]
fn unbalanced_entry_never_reaches_the_journal() {
    let dir = test_dir("unbalanced");
    let mut store = Store::create(&dir, Currency::USD).unwrap();
    let cash = store
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    let equity = store
        .add_account("3000", "Owner's Equity", AccountType::Equity, None, false)
        .unwrap();
    let seq_before = JournalWriter::open(&dir.join(ledger_store::JOURNAL_FILE))
        .unwrap()
        .next_seq();

    let e = store
        .create_entry(NaiveDate::from_ymd_opt(2026, 1, 2).unwrap(), "Bad")
        .unwrap();
    store.add_line(e, cash, Side::Debit, usd(100)).unwrap();
    store.add_line(e, equity, Side::Credit, usd(99)).unwrap();
    assert!(store.post_entry(e).is_err());

    // The failed post journaled nothing new: the draft creation and its
    // two lines were already journaled (durable drafts); entry.posted was not.
    let seq_after = JournalWriter::open(&dir.join(ledger_store::JOURNAL_FILE))
        .unwrap()
        .next_seq();
    assert_eq!(seq_after, seq_before + 3);
    // And a fresh open sees a clean, balanced book.
    drop(store);
    let store = Store::open(&dir).unwrap();
    store.verify().unwrap();
}

#[test]
fn draft_lines_survive_reopen() {
    // Drafts are durable: build a draft, close the store, reopen, and the
    // draft comes back with its lines intact.
    let dir = test_dir("draft-durable");
    let (cash, revenue);
    let draft_id;
    {
        let mut store = build_books(&dir);
        cash = store.ledger().chart().find_by_code("1000").unwrap().id;
        revenue = store.ledger().chart().find_by_code("4000").unwrap().id;
        draft_id = store
            .create_entry(
                NaiveDate::from_ymd_opt(2026, 2, 1).unwrap(),
                "unposted sale",
            )
            .unwrap();
        store
            .add_line(draft_id, cash, Side::Debit, usd(250))
            .unwrap();
        store
            .add_line(draft_id, revenue, Side::Credit, usd(250))
            .unwrap();
        // Draft is not posted: trial balance only sees the posted entry.
        let tb = ledger_core::reports::trial_balance(store.ledger());
        assert_eq!(tb.total_debits, Decimal::from(14000));
    }
    // Reopen: the draft and its lines are back.
    let mut store = Store::open(&dir).unwrap();
    let draft = store.ledger().get_entry(draft_id).unwrap();
    assert_eq!(draft.lines().len(), 2);
    assert_eq!(draft.status(), ledger_core::EntryStatus::Draft);
    // Post it after reopen: works, and the books balance.
    store.post_entry(draft_id).unwrap();
    let tb = ledger_core::reports::trial_balance(store.ledger());
    assert_eq!(tb.total_debits, Decimal::from(14250));
    assert_eq!(tb.total_credits, Decimal::from(14250));
    store.verify().unwrap();
}

#[test]
fn corrupt_projection_is_quarantined_and_rebuilt() {
    let dir = test_dir("corrupt-projection");
    {
        let _store = build_books(&dir);
    }
    // Corrupt the SQLite file with garbage.
    let db_path = dir.join(ledger_store::db_file());
    std::fs::write(&db_path, b"this is not a sqlite database at all").unwrap();
    // Reopen heals from the journal instead of failing.
    let store = Store::open(&dir).unwrap();
    // The damaged file was quarantined for forensics.
    let quarantined: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            // Kept beside it under its own name: wanderbooks.v{N}.corrupt.*
            let stem = ledger_store::db_file().trim_end_matches(".db").to_string();
            e.file_name()
                .to_string_lossy()
                .starts_with(&format!("{stem}.corrupt."))
        })
        .collect();
    assert_eq!(quarantined.len(), 1);
    // The books are intact and verify clean.
    let tb = ledger_core::reports::trial_balance(store.ledger());
    assert_eq!(tb.total_debits, Decimal::from(14000));
    store.verify().unwrap();
}

#[test]
fn receipt_corrections_are_journaled_and_survive_rebuild() {
    let dir = test_dir("receipt-corrections");
    let mut store = Store::create(&dir, Currency::USD).unwrap();
    let cash = store
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    let rent = store
        .add_account("6000", "Rent Expense", AccountType::Expense, None, false)
        .unwrap();
    let e = store
        .create_entry(
            NaiveDate::from_ymd_opt(2026, 2, 1).unwrap(),
            "February rent",
        )
        .unwrap();
    store.add_line(e, rent, Side::Debit, usd(800)).unwrap();
    store.add_line(e, cash, Side::Credit, usd(800)).unwrap();
    store.post_entry(e).unwrap();
    let (eml, _, _) = store
        .attach_receipt(Some(e), "rent.eml", "message/rfc822", b"eml-bytes", None)
        .unwrap();
    let (pdf, _, _) = store
        .attach_receipt(Some(e), "rent.pdf", "application/pdf", b"pdf-bytes", None)
        .unwrap();

    // Every correction needs a reason.
    assert!(matches!(
        store.unlink_receipt(eml, "  "),
        Err(StoreError::ReasonRequired)
    ));
    // A linked receipt can't be withdrawn or moved: unlink it first.
    assert!(matches!(
        store.withdraw_attachment(eml, "dup"),
        Err(StoreError::AttachmentLinked(a, b)) if a == eml && b == e
    ));
    assert!(matches!(
        store.link_receipt(pdf, e),
        Err(StoreError::AttachmentLinked(_, _))
    ));

    // Swap the .eml for the PDF: unlink, then withdraw.
    assert_eq!(
        store
            .unlink_receipt(eml, "replaced by the PDF render")
            .unwrap(),
        e
    );
    assert!(matches!(
        store.unlink_receipt(eml, "again"),
        Err(StoreError::AttachmentNotLinked(_))
    ));
    store
        .withdraw_attachment(eml, "file PDFs, not .eml")
        .unwrap();
    assert!(matches!(
        store.link_receipt(eml, e),
        Err(StoreError::AttachmentWithdrawn(_))
    ));
    assert!(matches!(
        store.withdraw_attachment(eml, "again"),
        Err(StoreError::AttachmentWithdrawn(_))
    ));
    assert!(matches!(
        store.unlink_receipt(999, "x"),
        Err(StoreError::UnknownAttachment(999))
    ));

    let check = |store: &Store| {
        let linked: Vec<u64> = store
            .list_attachments(Some(e))
            .unwrap()
            .iter()
            .map(|a| a.id)
            .collect();
        assert_eq!(linked, vec![pdf]);
        let all = store.list_attachments(None).unwrap();
        let gone = all.iter().find(|a| a.id == eml).unwrap();
        assert_eq!(gone.entry_id, None);
        assert!(gone.withdrawn_at.is_some());
        assert_eq!(
            gone.withdrawn_reason.as_deref(),
            Some("file PDFs, not .eml")
        );
        // Withdrawn is not deleted: the bytes are still there, hash-verified.
        assert_eq!(store.attachment_bytes(eml).unwrap(), b"eml-bytes");
    };
    check(&store);

    // The journal is the history: the attach, the unlink, and the withdrawal.
    let journal = std::fs::read_to_string(dir.join("journal.jsonl")).unwrap();
    for t in [
        "receipt.attached",
        "receipt.unlinked",
        "attachment.withdrawn",
    ] {
        assert!(
            journal.contains(&format!("\"type\":\"{t}\"")),
            "missing {t}"
        );
    }

    drop(store);
    let mut store = Store::open(&dir).unwrap();
    check(&store);
    store.rebuild().unwrap();
    check(&store);
    store.verify().unwrap();
}

#[test]
fn a_close_into_members_capital_replays_exactly() {
    let dir = test_dir("members-close");
    let mut store = Store::create(&dir, Currency::USD).unwrap();
    let cash = store
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    let alice = store
        .add_account("3000", "Capital — Alice", AccountType::Equity, None, false)
        .unwrap();
    let bob = store
        .add_account("3001", "Capital — Bob", AccountType::Equity, None, false)
        .unwrap();
    let revenue = store
        .add_account("4000", "Revenue", AccountType::Revenue, None, false)
        .unwrap();
    let jan15 = NaiveDate::from_ymd_opt(2026, 1, 15).unwrap();
    let e = store.create_entry(jan15, "Sale").unwrap();
    store
        .add_line(
            e,
            cash,
            Side::Debit,
            Money::new(Decimal::new(10001, 2), Currency::USD),
        )
        .unwrap();
    store
        .add_line(
            e,
            revenue,
            Side::Credit,
            Money::new(Decimal::new(10001, 2), Currency::USD),
        )
        .unwrap();
    store.post_entry(e).unwrap();
    let jan = store
        .create_period(
            "2026-01",
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 1, 31).unwrap(),
        )
        .unwrap();
    let half = Decimal::new(5, 1);
    store
        .close_period_to(
            jan,
            &ledger_core::CloseTo::Members(vec![
                ledger_core::MemberClose {
                    capital: alice,
                    share: half,
                    distributions: None,
                },
                ledger_core::MemberClose {
                    capital: bob,
                    share: half,
                    distributions: None,
                },
            ]),
        )
        .unwrap();
    // 100.01 splits 50.01 / 50.00 into the capital accounts.
    let balances = |s: &Store| {
        (
            s.ledger().account_balance(alice).unwrap(),
            s.ledger().account_balance(bob).unwrap(),
        )
    };
    let want = (Decimal::new(5001, 2), Decimal::new(5000, 2));
    assert_eq!(balances(&store), want);
    let journal = std::fs::read_to_string(dir.join("journal.jsonl")).unwrap();
    assert!(journal.contains("\"close_to\":{\"members\""));

    drop(store);
    let mut store = Store::open(&dir).unwrap();
    assert_eq!(balances(&store), want);
    store.rebuild().unwrap();
    assert_eq!(balances(&store), want);
    store.verify().unwrap();
}

#[test]
fn schedules_and_recurring_entries_replay_exactly() {
    let dir = test_dir("schedules");
    let mut store = Store::create(&dir, Currency::USD).unwrap();
    let cash = store
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    let prepaid = store
        .add_account("1300", "Prepaid", AccountType::Asset, None, false)
        .unwrap();
    let ins = store
        .add_account("6300", "Insurance", AccountType::Expense, None, false)
        .unwrap();
    let rent = store
        .add_account("6600", "Rent", AccountType::Expense, None, false)
        .unwrap();
    let date = |m, d| NaiveDate::from_ymd_opt(2026, m, d).unwrap();
    let usd_s = |s: &str| Money::new(s.parse().unwrap(), Currency::USD);
    store
        .register_schedule(ledger_core::NewSchedule {
            description: "Insurance".into(),
            kind: ledger_core::ScheduleKind::PrepaidExpense,
            total: usd_s("1000"),
            start: date(1, 1),
            months: 3,
            balance_account: prepaid,
            pl_account: ins,
            convention: ledger_core::ScheduleConvention::MonthlyEven,
        })
        .unwrap();
    store
        .register_recurring(ledger_core::NewRecurring {
            description: "Rent".into(),
            lines: vec![
                (rent, Side::Debit, usd_s("500")),
                (cash, Side::Credit, usd_s("500")),
            ],
            frequency: ledger_core::Frequency::Monthly,
            start: date(1, 1),
            end: None,
        })
        .unwrap();
    // 333.33, 333.33 through February; rent drafts for Jan 1 and Feb 1.
    let posted = store.run_schedules(date(2, 28)).unwrap();
    assert_eq!(posted.len(), 2);
    let drafts = store.run_recurring(date(2, 28)).unwrap();
    assert_eq!(
        drafts.iter().map(|d| d.1).collect::<Vec<_>>(),
        [date(1, 1), date(2, 1)]
    );
    store.post_entry(drafts[0].2).unwrap();
    let state = |s: &Store| {
        (
            s.ledger().account_balance(ins).unwrap(),
            s.ledger().account_balance(rent).unwrap(),
            s.ledger().get_entry(drafts[1].2).unwrap().status(),
        )
    };
    let want = (
        Decimal::new(66666, 2),
        Decimal::new(500, 0),
        ledger_core::EntryStatus::Draft,
    );
    assert_eq!(state(&store), want);

    drop(store);
    let mut store = Store::open(&dir).unwrap();
    assert_eq!(state(&store), want);
    store.rebuild().unwrap();
    assert_eq!(state(&store), want);
    store.verify().unwrap();
    // Idempotent after replay: nothing new is due through February.
    assert!(store.run_schedules(date(2, 28)).unwrap().is_empty());
    assert!(store.run_recurring(date(2, 28)).unwrap().is_empty());
}

/// Regression (B-12, found by the refusal property on 2026-10-13):
/// `run_recurring` leaves occurrences in a closed period ungenerated
/// instead of creating drafts there, and still generates the rest.
#[test]
fn run_recurring_skips_occurrences_in_a_closed_period() {
    let dir = test_dir("recurring-closed");
    let date = |m, d| NaiveDate::from_ymd_opt(2026, m, d).unwrap();
    let usd_s = |s: &str| Money::new(s.parse().unwrap(), Currency::USD);
    let mut store = Store::create(&dir, Currency::USD).unwrap();
    let cash = store
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    let rent = store
        .add_account("6000", "Rent", AccountType::Expense, None, false)
        .unwrap();
    store
        .register_recurring(ledger_core::NewRecurring {
            description: "Rent".into(),
            lines: vec![
                (rent, Side::Debit, usd_s("500")),
                (cash, Side::Credit, usd_s("500")),
            ],
            frequency: ledger_core::Frequency::Monthly,
            start: date(1, 1),
            end: None,
        })
        .unwrap();
    let jan = store
        .create_period("2026-01", date(1, 1), date(1, 31))
        .unwrap();
    store.close_period(jan).unwrap();
    let drafts = store.run_recurring(date(2, 28)).unwrap();
    assert_eq!(drafts.iter().map(|d| d.1).collect::<Vec<_>>(), [date(2, 1)]);
    assert_eq!(
        store.ledger().recurring_in_closed_periods(date(2, 28)),
        [(1, date(1, 1))]
    );
    drop(store);
    let store = Store::open(&dir).unwrap();
    store.verify().unwrap();
    assert_eq!(
        store
            .ledger()
            .recurring_in_closed_periods(date(2, 28))
            .len(),
        1
    );
}

#[test]
fn vendor_1099_tags_replay_and_project() {
    use ledger_core::arap::DocumentLine;
    use ledger_core::{Dated1099, Decision1099, Form1099, PaymentAccounts, vendor_1099_report};
    let dir = test_dir("form1099");
    let mut store = Store::create(&dir, Currency::USD).unwrap();
    let cash = store
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    let labor = store
        .add_account("6000", "Contract Labor", AccountType::Expense, None, false)
        .unwrap();
    let dana = store.add_vendor("Dana").unwrap();
    let d = |m, day| NaiveDate::from_ymd_opt(2026, m, day).unwrap();
    store
        .set_vendor_1099(
            dana,
            &Dated1099::from_beginning(
                Decision1099::Files(Form1099::Nec),
                Some(ledger_core::settings::Source {
                    decided_by: "Riley Chen".into(),
                    decided_on: d(1, 5),
                    citation: None,
                    note: None,
                }),
            ),
            Some(true),
        )
        .unwrap();
    let report = |l: &ledger_core::Ledger| {
        vendor_1099_report(
            l,
            2026,
            None,
            &PaymentAccounts {
                cash: vec![cash],
                financing: vec![],
            },
            &[],
            None,
        )
        .unwrap()
    };
    let bill = store
        .create_bill(
            dana,
            d(3, 1),
            d(3, 31),
            vec![DocumentLine {
                description: "Design".into(),
                amount: Decimal::from(1500),
            }],
        )
        .unwrap();
    store
        .pay_bill(bill, d(3, 20), Decimal::from(1500), cash)
        .unwrap();
    let e = store.create_entry(d(6, 1), "Dana, June").unwrap();
    store.add_line(e, labor, Side::Debit, usd(700)).unwrap();
    store.add_line(e, cash, Side::Credit, usd(700)).unwrap();
    store.post_entry(e).unwrap();
    store.set_entry_vendor(e, Some(dana)).unwrap();
    let dup = store.create_entry(d(6, 2), "Dana, duplicate").unwrap();
    store.add_line(dup, labor, Side::Debit, usd(700)).unwrap();
    store.add_line(dup, cash, Side::Credit, usd(700)).unwrap();
    store.post_entry(dup).unwrap();
    store.set_entry_vendor(dup, Some(dana)).unwrap();
    store.reverse_entry(dup, d(6, 3), "duplicate").unwrap();
    let before = report(store.ledger());
    assert_eq!(before.vendors[0].paid, Decimal::from(2200));
    assert!(before.vendors[0].reportable);
    drop(store);

    // Reopen (replay), then rebuild the projection from scratch.
    for rebuild in [false, true] {
        if rebuild {
            std::fs::remove_file(dir.join(ledger_store::db_file())).unwrap();
        }
        let store = Store::open(&dir).unwrap();
        assert_eq!(report(store.ledger()), before);
        let conn = rusqlite::Connection::open(dir.join(ledger_store::db_file())).unwrap();
        let tagged: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM entry_vendors WHERE vendor_id = ?",
                [dana as i64],
                |r| r.get(0),
            )
            .unwrap();
        // Bill, its payment, June, the duplicate, and its reversal.
        assert_eq!(tagged, 5);
        let form: String = conn
            .query_row("SELECT form_1099 FROM vendors", [], |r| r.get(0))
            .unwrap();
        assert_eq!(form, "nec");
        // The decision, with who made it, line by line.
        let (decided, by): (String, String) = conn
            .query_row(
                "SELECT form_1099, source_json FROM vendor_1099_decisions",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(decided, "nec");
        assert!(by.contains("Riley Chen"), "{by}");
    }
}

#[test]
fn the_whole_projection_is_checked_and_healed() {
    let dir = test_dir("heal");
    let mut store = build_books(&dir);
    let e = store
        .ledger()
        .posted_entries()
        .next()
        .map(|e| e.id())
        .unwrap();
    store
        .attach_receipt(Some(e), "r.png", "image/png", b"receipt", None)
        .unwrap();
    store.verify().unwrap();
    drop(store);
    // Drift no accounting report would notice: a receipt quietly unlinked.
    let db = dir.join(ledger_store::db_file());
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute("UPDATE attachments SET entry_id = NULL", [])
        .unwrap();
    drop(conn);
    let mut store = Store::open(&dir).unwrap();
    match store.verify() {
        Err(StoreError::VerifyMismatch(why)) => assert!(why.contains("attachments"), "{why}"),
        other => panic!("drift not caught: {other:?}"),
    }
    let report = store.verify_and_heal().unwrap();
    assert!(report.healed.is_some(), "healed");
    assert!(report.events > 0);
    store.verify().unwrap();
    assert_eq!(
        store.list_attachments(Some(e)).unwrap().len(),
        1,
        "link restored"
    );
    let kept = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|d| d.ok())
        .any(|d| d.file_name().to_string_lossy().contains(".mismatch."));
    assert!(kept, "the divergent database is kept for forensics");
    // A healthy store reports nothing to heal.
    assert_eq!(store.verify_and_heal().unwrap().healed, None);
}

#[test]
fn a_draft_before_a_posted_entry_verifies_and_reopens() {
    // The real case: a reimbursement drafted (#8, waiting on the bank),
    // then a domain posted (#9). Drafts aren't in the projection, so its
    // entry ids skip.
    let dir = test_dir("draft-gap");
    let mut store = build_books(&dir);
    let cash = store.ledger().chart().find_by_code("1000").unwrap().id;
    let equity = store.ledger().chart().find_by_code("3000").unwrap().id;
    let d = NaiveDate::from_ymd_opt(2026, 10, 12).unwrap();
    let draft = store.create_entry(d, "waiting on the bank").unwrap();
    store.add_line(draft, cash, Side::Debit, usd(5)).unwrap();
    store.add_line(draft, equity, Side::Credit, usd(5)).unwrap();
    let posted = store.create_entry(d, "posted after the draft").unwrap();
    store.add_line(posted, cash, Side::Debit, usd(7)).unwrap();
    store
        .add_line(posted, equity, Side::Credit, usd(7))
        .unwrap();
    store.post_entry(posted).unwrap();
    store.verify().unwrap();
    drop(store);
    let mut store = Store::open(&dir).unwrap();
    assert_eq!(store.verify_and_heal().unwrap().healed, None);
}

/// Regression (found by the max-testing pass, 2026-10-13): reversing into a
/// closed period journaled the reversal *before* the engine refused it, so
/// the journal held an event replay could never apply and the books would
/// not reopen. Oracle (rank 1, spec R-3 + closed periods): a refused
/// command leaves no journal line, and the books always reopen.
#[test]
fn reversal_into_a_closed_period_is_refused_before_journaling() {
    let dir = test_dir("reverse-closed");
    let mut store = build_books(&dir);
    let jan = store
        .create_period(
            "2026-01",
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 1, 31).unwrap(),
        )
        .unwrap();
    store.close_period(jan).unwrap();
    let posted = store.ledger().posted_entries().next().unwrap().id();
    let journal = std::fs::read_to_string(dir.join("journal.jsonl")).unwrap();
    let err = store.reverse_entry(
        posted,
        NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
        "into a closed month",
    );
    assert!(err.is_err());
    assert_eq!(
        std::fs::read_to_string(dir.join("journal.jsonl")).unwrap(),
        journal,
        "a refused reversal must not reach the journal"
    );
    drop(store);
    let reopened = Store::open(&dir).expect("the books still open");
    reopened.verify().unwrap();
}

// ---------------------------------------------------------------------------
// Refusals never reach the journal (rank 2: a law over every command
// sequence). Found by the max-testing pass, 2026-10-13: a reversal into a
// closed period was journaled before the engine refused it, and the books
// would not reopen.
// ---------------------------------------------------------------------------

mod refusals {
    use super::*;
    use ledger_core::{DocumentLine, Form1099};
    use proptest::prelude::*;

    /// One store command, with arguments drawn to hit both the happy path
    /// and every refusal: small id spaces so unknown, duplicate, posted,
    /// closed, and already-reversed cases come up constantly.
    #[derive(Clone, Debug)]
    enum Op {
        AddAccount(u8, u8, Option<u64>, bool),
        CreateEntry(u32),
        AddLine(u64, u64, bool, i64),
        Post(u64),
        Reverse(u64, u32),
        CreatePeriod(u32, u32),
        Close(u64),
        AddCustomer(bool),
        AddVendor(bool),
        Invoice(u64, u32, u32, i64),
        Bill(u64, u32, u32, i64),
        ReceivePayment(u64, u32, i64, u64),
        PayBill(u64, u32, i64, u64),
        AddClass(u8),
        SetLineClass(u64, usize, Option<u32>),
        SetEntryVendor(u64, Option<u64>),
        /// A 1099 decision (0 undetermined, 1 nec, 2 a corporation, 3 a
        /// blank reason, 4 no source), dated or not, with a W-9 flag or
        /// none.
        SetVendor1099(u64, u8, Option<u32>, Option<bool>),
        SetW9Flag(u64, bool),
        /// A payment method: kind, label, account.
        AddInstrument(u8, u8, u64),
        SetEntryInstrument(u64, Option<u64>),
        SetAccountTax(u64, u8),
        Renumber(u64, u8),
        // P3 (defects 8, 15): renames, accounts added with a category,
        // and the tax category policy turned on and off.
        Rename(u64, u8),
        AddAccountTax(u8, u8, u8, bool),
        SetTaxPolicy(bool),
        // The rest of the command surface (2026-10-13).
        AttachReceipt(Option<u64>, u8),
        FileDocument(u8),
        LinkReceipt(u64, u64),
        UnlinkReceipt(u64),
        Withdraw(u64),
        SetFxRate(u32, i64),
        AddLineFx(u64, u64, bool, i64, i64),
        RegisterAsset(u64, u64, u64, i64, u32),
        RunDepreciation(u64, u32),
        DisposeAsset(u64, u32, i64, u64, u64),
        RunPayroll(u32, i64, i64, u64),
        Remit(u32, u64),
        RegisterSchedule(bool, i64, u32, u32, u64, u64, bool),
        PostScheduleMonth(u64, u32),
        RegisterRecurring(u64, u64, i64, u32),
        RunRecurring(u32),
        MarkCleared(u64, usize, bool),
        CloseMembers(u64, u64, u64),
        /// A balanced pair of lines on a draft, so entries post.
        Pair(u64, u64, u64, i64),
        // Compound commands with valid arguments, so the success paths
        // that need a chain of earlier steps get exercised too.
        Book(u32, u64, u64, i64),
        GoodAsset(i64, u32),
        GoodSchedule(bool, i64, u32, bool),
        GoodInvoice(u32, i64),
        GoodBill(u32, i64),
        GoodClose(u32),
    }

    fn day(n: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 1, 1).unwrap() + chrono::Days::new(u64::from(n % 120))
    }

    /// A schedule's convention: by the day, or equal months (refused
    /// unless it starts on the 1st, D-37).
    fn convention(daily: bool) -> ledger_core::ScheduleConvention {
        if daily {
            ledger_core::ScheduleConvention::Daily
        } else {
            ledger_core::ScheduleConvention::MonthlyEven
        }
    }

    /// Cents, biased toward the edges: zero, negative, tiny, huge.
    fn arb_cents() -> impl Strategy<Value = i64> {
        prop_oneof![
            4 => 1i64..200_000,
            1 => Just(0i64),
            1 => -50_000i64..0,
            1 => Just(i64::MAX / 2),
        ]
    }

    fn arb_op() -> impl Strategy<Value = Op> {
        let id = 0u64..12;
        prop_oneof![
            (0u8..8, 0u8..5, prop::option::of(0u64..10), any::<bool>())
                .prop_map(|(c, t, p, h)| Op::AddAccount(c, t, p, h)),
            (0u32..120).prop_map(Op::CreateEntry),
            (id.clone(), 0u64..10, any::<bool>(), arb_cents())
                .prop_map(|(e, a, d, c)| Op::AddLine(e, a, d, c)),
            id.clone().prop_map(Op::Post),
            (id.clone(), 0u32..120).prop_map(|(e, d)| Op::Reverse(e, d)),
            (0u32..120, 0u32..120).prop_map(|(a, b)| Op::CreatePeriod(a, b)),
            (0u64..5).prop_map(Op::Close),
            any::<bool>().prop_map(Op::AddCustomer),
            any::<bool>().prop_map(Op::AddVendor),
            (0u64..4, 0u32..120, 0u32..120, arb_cents())
                .prop_map(|(c, d, due, a)| Op::Invoice(c, d, due, a)),
            (0u64..4, 0u32..120, 0u32..120, arb_cents())
                .prop_map(|(v, d, due, a)| Op::Bill(v, d, due, a)),
            (0u64..4, 0u32..120, arb_cents(), 0u64..10)
                .prop_map(|(i, d, a, c)| Op::ReceivePayment(i, d, a, c)),
            (0u64..4, 0u32..120, arb_cents(), 0u64..10)
                .prop_map(|(b, d, a, c)| Op::PayBill(b, d, a, c)),
            (0u8..3).prop_map(Op::AddClass),
            (id.clone(), 0usize..4, prop::option::of(0u32..4))
                .prop_map(|(e, i, c)| Op::SetLineClass(e, i, c)),
            (id.clone(), prop::option::of(0u64..4)).prop_map(|(e, v)| Op::SetEntryVendor(e, v)),
            (
                0u64..4,
                0u8..5,
                prop::option::of(0u32..120),
                prop::option::of(any::<bool>())
            )
                .prop_map(|(v, k, d, t)| Op::SetVendor1099(v, k, d, t)),
            (0u64..4, any::<bool>()).prop_map(|(v, t)| Op::SetW9Flag(v, t)),
            (0u8..6, 0u8..3, 0u64..12).prop_map(|(k, l, a)| Op::AddInstrument(k, l, a)),
            (id.clone(), prop::option::of(0u64..4)).prop_map(|(e, i)| Op::SetEntryInstrument(e, i)),
            (0u64..10, 0u8..4).prop_map(|(a, k)| Op::SetAccountTax(a, k)),
            (0u64..10, 0u8..9).prop_map(|(a, c)| Op::Renumber(a, c)),
            (0u64..12, 0u8..8).prop_map(|(a, n)| Op::Rename(a, n)),
            (0u8..8, 0u8..5, 0u8..4, any::<bool>())
                .prop_map(|(c, t, k, h)| Op::AddAccountTax(c, t, k, h)),
            any::<bool>().prop_map(Op::SetTaxPolicy),
            (prop::option::of(id.clone()), 0u8..4).prop_map(|(e, b)| Op::AttachReceipt(e, b)),
            (0u8..4).prop_map(Op::FileDocument),
            (0u64..6, id.clone()).prop_map(|(a, e)| Op::LinkReceipt(a, e)),
            (0u64..6).prop_map(Op::UnlinkReceipt),
            (0u64..6).prop_map(Op::Withdraw),
            (0u32..120, -2i64..30_000).prop_map(|(d, r)| Op::SetFxRate(d, r)),
            (
                id.clone(),
                0u64..10,
                any::<bool>(),
                arb_cents(),
                1i64..30_000
            )
                .prop_map(|(e, a, s, f, r)| Op::AddLineFx(e, a, s, f, r)),
            (0u64..10, 0u64..10, 0u64..10, arb_cents(), 0u32..40)
                .prop_map(|(c, d, x, cost, m)| Op::RegisterAsset(c, d, x, cost, m)),
            (0u64..4, 0u32..12).prop_map(|(a, m)| Op::RunDepreciation(a, m)),
            (0u64..4, 0u32..120, arb_cents(), 0u64..10, 0u64..10)
                .prop_map(|(a, d, p, c, g)| Op::DisposeAsset(a, d, p, c, g)),
            (0u32..120, arb_cents(), arb_cents(), 0u64..10)
                .prop_map(|(d, g, w, c)| Op::RunPayroll(d, g, w, c)),
            (0u32..120, 0u64..10).prop_map(|(d, c)| Op::Remit(d, c)),
            (
                any::<bool>(),
                arb_cents(),
                0u32..120,
                0u32..30,
                0u64..10,
                0u64..10,
                any::<bool>()
            )
                .prop_map(|(k, t, st, m, b, p, daily)| Op::RegisterSchedule(
                    k, t, st, m, b, p, daily
                )),
            (0u64..4, 0u32..12).prop_map(|(s, m)| Op::PostScheduleMonth(s, m)),
            (0u64..10, 0u64..10, arb_cents(), 0u32..120)
                .prop_map(|(a, b, c, d)| Op::RegisterRecurring(a, b, c, d)),
            (0u32..180).prop_map(Op::RunRecurring),
            (id.clone(), 0usize..4, any::<bool>()).prop_map(|(e, i, c)| Op::MarkCleared(e, i, c)),
            (0u64..5, 0u64..10, 0u64..10).prop_map(|(p, a, b)| Op::CloseMembers(p, a, b)),
            (id.clone(), 1u64..10, 1u64..10, 1i64..500_000)
                .prop_map(|(e, a, b, c)| Op::Pair(e, a, b, c)),
            (id.clone(), 1u64..10, 1u64..10, 1i64..500_000)
                .prop_map(|(e, a, b, c)| Op::Pair(e, a, b, c)),
            (id.clone(), 1u64..10, 1u64..10, 1i64..500_000)
                .prop_map(|(e, a, b, c)| Op::Pair(e, a, b, c)),
            id.clone().prop_map(Op::Post),
            id.clone().prop_map(Op::Post),
            (0u32..120, 1u64..10, 1u64..10, 1i64..500_000)
                .prop_map(|(d, a, b, c)| Op::Book(d, a, b, c)),
            (0u32..120, 1u64..10, 1u64..10, 1i64..500_000)
                .prop_map(|(d, a, b, c)| Op::Book(d, a, b, c)),
            (0u32..120, 1u64..10, 1u64..10, 1i64..500_000)
                .prop_map(|(d, a, b, c)| Op::Book(d, a, b, c)),
            (1i64..5_000_000, 1u32..60).prop_map(|(c, m)| Op::GoodAsset(c, m)),
            (any::<bool>(), 1i64..1_000_000, 1u32..24, any::<bool>())
                .prop_map(|(k, t, m, daily)| Op::GoodSchedule(k, t, m, daily)),
            (0u32..120, 1i64..1_000_000).prop_map(|(d, c)| Op::GoodInvoice(d, c)),
            (0u32..120, 1i64..1_000_000).prop_map(|(d, c)| Op::GoodBill(d, c)),
            (1u32..5).prop_map(Op::GoodClose),
        ]
    }

    const CODES: [&str; 8] = [
        "1000", "1200", "2000", "3000", "3999", "4000", "6000", "6100",
    ];
    const TYPES: [AccountType; 5] = [
        AccountType::Asset,
        AccountType::Liability,
        AccountType::Equity,
        AccountType::Revenue,
        AccountType::Expense,
    ];
    const TAX: [Option<&str>; 4] = [
        None,
        Some("advertising"),
        Some("no-such-category"),
        Some(""),
    ];

    /// Runs one op. A compound op is several commands; `mark` is called
    /// after each step that succeeded, so the refusal law is checked
    /// against the one command that was refused.
    fn apply(store: &mut Store, op: &Op, mark: &mut dyn FnMut()) -> Result<(), StoreError> {
        let usd = |c: i64| Money::new(Decimal::new(c, 2), Currency::USD);
        let line = |c: i64| {
            vec![DocumentLine {
                description: "work".into(),
                amount: Decimal::new(c, 2),
            }]
        };
        match *op {
            // Named for the code: a sibling's name is refused (B-31), and
            // a code is the first refusal.
            Op::AddAccount(c, t, p, h) => store
                .add_account(
                    CODES[c as usize],
                    &format!("Account {}", CODES[c as usize]),
                    TYPES[t as usize],
                    p,
                    h,
                )
                .map(drop),
            // A setup account's own name, another's (in other capitals),
            // a blank, nine digits, and fresh names.
            Op::Rename(a, n) => store.rename_account(
                a,
                [
                    "Sales",
                    "sales",
                    "CASH",
                    "",
                    "Card 123456789",
                    "Renamed",
                    "Office",
                    "Rent ",
                ][n as usize],
                (n % 2 == 0).then_some("owner asked"),
            ),
            Op::AddAccountTax(c, t, k, h) => store
                .add_account_with_tax(
                    CODES[c as usize],
                    &format!("Taxed {}", CODES[c as usize]),
                    TYPES[t as usize],
                    None,
                    h,
                    TAX[k as usize].map(|cat| (cat, None)),
                )
                .map(drop),
            Op::SetTaxPolicy(refuse) => store
                .change_settings(
                    &[ledger_core::settings::Change {
                        key: "policy.require_tax_category".into(),
                        value: Some(serde_json::json!(if refuse { "refuse" } else { "warn" })),
                    }],
                    &ledger_core::settings::Source {
                        decided_by: "Owner".into(),
                        decided_on: day(0),
                        citation: None,
                        note: None,
                    },
                )
                .map(drop),
            Op::CreateEntry(d) => store.create_entry(day(d), "e").map(drop),
            Op::AddLine(e, a, d, c) => {
                let side = if d { Side::Debit } else { Side::Credit };
                store.add_line(e, a, side, usd(c))
            }
            Op::Post(e) => store.post_entry(e),
            Op::Reverse(e, d) => store.reverse_entry(e, day(d), "undo").map(drop),
            Op::CreatePeriod(a, b) => store.create_period("p", day(a), day(b)).map(drop),
            Op::Close(p) => store.close_period(p).map(drop),
            Op::AddCustomer(blank) => store
                .add_customer(if blank { " " } else { "Acme" })
                .map(drop),
            Op::AddVendor(blank) => store.add_vendor(if blank { "" } else { "Vend" }).map(drop),
            Op::Invoice(c, d, due, a) => {
                store.create_invoice(c, day(d), day(due), line(a)).map(drop)
            }
            Op::Bill(v, d, due, a) => store.create_bill(v, day(d), day(due), line(a)).map(drop),
            Op::ReceivePayment(i, d, a, c) => store
                .receive_payment(i, day(d), Decimal::new(a, 2), c)
                .map(drop),
            Op::PayBill(b, d, a, c) => store.pay_bill(b, day(d), Decimal::new(a, 2), c).map(drop),
            Op::AddClass(n) => store.add_class(["A", "B", ""][n as usize]).map(drop),
            Op::SetLineClass(e, i, c) => store.set_line_class(e, i, c),
            Op::SetEntryVendor(e, v) => store.set_entry_vendor(e, v),
            Op::SetVendor1099(v, k, d, t) => {
                let decision = match k {
                    0 => ledger_core::Decision1099::Undetermined,
                    1 => ledger_core::Decision1099::Files(Form1099::Nec),
                    2 => ledger_core::Decision1099::not_reportable(
                        ledger_core::ExemptReason::Corporation,
                        "a corporation by name",
                    )
                    .unwrap(),
                    _ => ledger_core::Decision1099::NotReportable {
                        kind: ledger_core::ExemptReason::Other,
                        reason: " ".into(),
                    },
                };
                let source = (k != 4).then(|| ledger_core::settings::Source {
                    decided_by: "Riley Chen".into(),
                    decided_on: day(0),
                    citation: None,
                    note: None,
                });
                let dated = ledger_core::Dated1099 {
                    effective: d.map(day),
                    decision,
                    source,
                };
                store.set_vendor_1099(v, &dated, t).map(drop)
            }
            Op::SetW9Flag(v, t) => store.set_vendor_w9_flag(v, t).map(drop),
            Op::AddInstrument(k, l, a) => store
                .add_instrument(
                    [
                        ledger_core::InstrumentKind::Bank,
                        ledger_core::InstrumentKind::BusinessCard,
                        ledger_core::InstrumentKind::MemberCard,
                        ledger_core::InstrumentKind::MemberCashOrCheck,
                        ledger_core::InstrumentKind::Platform,
                        ledger_core::InstrumentKind::Bank,
                    ][k as usize],
                    ["Harbor Bank", "Riley's Visa", ""][l as usize],
                    (k == 5).then_some("12345"),
                    None,
                    a,
                )
                .map(drop),
            Op::SetEntryInstrument(e, i) => store.set_entry_instrument(e, i).map(drop),
            Op::SetAccountTax(a, k) => store.set_account_tax(a, TAX[k as usize], None),
            // The chart's codes, control codes (payroll's 6100 and 2110,
            // revenue, receivables), a blank, and fresh ones.
            Op::Renumber(a, c) => store.renumber_account(
                a,
                [
                    "1000", "6100", "2110", "4000", "1200", "", "7001", "7002", "3999",
                ][c as usize],
            ),
            Op::AttachReceipt(e, b) => store
                .attach_receipt(e, "r.pdf", "application/pdf", &[b; 3], Some("receipt"))
                .map(drop),
            Op::FileDocument(b) => store
                .file_document("legal", "d.pdf", "application/pdf", &[b, 9], None)
                .map(drop),
            Op::LinkReceipt(a, e) => store.link_receipt(a, e),
            Op::UnlinkReceipt(a) => store.unlink_receipt(a, "wrong receipt").map(drop),
            Op::Withdraw(a) => store.withdraw_attachment(a, "duplicate"),
            Op::SetFxRate(d, r) => store.set_fx_rate(Currency::EUR, day(d), Decimal::new(r, 4)),
            Op::AddLineFx(e, a, d, f, r) => {
                let side = if d { Side::Debit } else { Side::Credit };
                store.add_line_fx(
                    e,
                    a,
                    side,
                    Currency::EUR,
                    Decimal::new(f, 2),
                    Decimal::new(r, 4),
                )
            }
            Op::RegisterAsset(c, d, x, cost, m) => store
                .register_asset(ledger_core::NewAsset {
                    description: "van".into(),
                    in_service_date: day(0),
                    cost: usd(cost),
                    salvage_value: usd(0),
                    useful_life_months: m,
                    cost_account: c,
                    accum_depr_account: d,
                    expense_account: x,
                })
                .map(drop),
            Op::RunDepreciation(a, m) => store
                .run_depreciation(a, &format!("2026-{:02}", m + 1))
                .map(drop),
            Op::DisposeAsset(a, d, p, c, g) => {
                store.dispose_asset(a, day(d), usd(p), c, g).map(drop)
            }
            Op::RunPayroll(d, g, w, c) => store
                .run_payroll(
                    day(d),
                    vec![ledger_core::PayrollLine {
                        employee_name: "Pat".into(),
                        employee: None,
                        gross_pay: Decimal::new(g, 2),
                        federal_withholding: Decimal::new(w, 2),
                        state_withholding: Decimal::ZERO,
                        ss_employee: Decimal::ZERO,
                        medicare_employee: Decimal::ZERO,
                        // 6.2%, rounded to the cent as a caller must.
                        ss_employer: (Decimal::new(g, 2) * Decimal::new(62, 3)).round_dp(2),
                        medicare_employer: Decimal::ZERO,
                        other_deductions: Decimal::ZERO,
                    }],
                    c,
                )
                .map(drop),
            Op::Remit(d, c) => store.remit_payroll_taxes(day(d), c).map(drop),
            Op::RegisterSchedule(k, t, st, m, b, pl, daily) => store
                .register_schedule(ledger_core::NewSchedule {
                    description: "sched".into(),
                    kind: if k {
                        ledger_core::ScheduleKind::PrepaidExpense
                    } else {
                        ledger_core::ScheduleKind::DeferredRevenue
                    },
                    total: usd(t),
                    start: day(st),
                    months: m,
                    balance_account: b,
                    pl_account: pl,
                    convention: convention(daily),
                })
                .map(drop),
            Op::PostScheduleMonth(id, m) => {
                let first = NaiveDate::from_ymd_opt(2026, m + 1, 1).unwrap();
                let end = first + chrono::Months::new(1) - chrono::Days::new(1);
                store.post_schedule_month(id, end).map(drop)
            }
            Op::RegisterRecurring(a, b, c, d) => store
                .register_recurring(ledger_core::NewRecurring {
                    description: "rent".into(),
                    lines: vec![(a, Side::Debit, usd(c)), (b, Side::Credit, usd(c))],
                    frequency: ledger_core::Frequency::Monthly,
                    start: day(d),
                    end: None,
                })
                .map(drop),
            Op::RunRecurring(d) => store
                .run_recurring(
                    NaiveDate::from_ymd_opt(2026, 1, 1).unwrap() + chrono::Days::new(u64::from(d)),
                )
                .map(drop),
            Op::MarkCleared(e, i, c) => store.mark_cleared(e, i, c),
            Op::Book(d, a, b, c) => {
                let e = store.create_entry(day(d), "book")?;
                mark();
                store.add_line(e, a, Side::Debit, usd(c))?;
                mark();
                store.add_line(e, b, Side::Credit, usd(c))?;
                mark();
                store.post_entry(e)
            }
            Op::GoodAsset(c, m) => store
                .register_asset(ledger_core::NewAsset {
                    description: "machine".into(),
                    in_service_date: day(0),
                    cost: usd(c),
                    salvage_value: usd(0),
                    useful_life_months: m,
                    cost_account: 2,
                    accum_depr_account: 3,
                    expense_account: 9,
                })
                .map(drop),
            Op::GoodSchedule(k, t, m, daily) => store
                .register_schedule(ledger_core::NewSchedule {
                    description: "insurance".into(),
                    kind: if k {
                        ledger_core::ScheduleKind::PrepaidExpense
                    } else {
                        ledger_core::ScheduleKind::DeferredRevenue
                    },
                    total: usd(t),
                    start: day(0),
                    months: m,
                    balance_account: if k { 1 } else { 6 },
                    pl_account: if k { 8 } else { 7 },
                    convention: convention(daily),
                })
                .map(drop),
            Op::GoodInvoice(d, c) => {
                let who = store.add_customer("Client")?;
                mark();
                store
                    .create_invoice(who, day(d), day(d) + chrono::Days::new(30), line(c))
                    .map(drop)
            }
            Op::GoodBill(d, c) => {
                let who = store.add_vendor("Supplier")?;
                mark();
                store
                    .create_bill(who, day(d), day(d) + chrono::Days::new(30), line(c))
                    .map(drop)
            }
            Op::GoodClose(m) => {
                let start = NaiveDate::from_ymd_opt(2026, m, 1).unwrap();
                let end = NaiveDate::from_ymd_opt(2026, m + 1, 1)
                    .unwrap()
                    .pred_opt()
                    .unwrap();
                let p = store.create_period(&format!("2026-{m:02}"), start, end)?;
                mark();
                store
                    .close_period_to(
                        p,
                        &ledger_core::CloseTo::Members(vec![
                            ledger_core::MemberClose {
                                capital: 4,
                                share: Decimal::new(5, 1),
                                distributions: None,
                            },
                            ledger_core::MemberClose {
                                capital: 5,
                                share: Decimal::new(5, 1),
                                distributions: None,
                            },
                        ]),
                    )
                    .map(drop)
            }
            Op::Pair(e, a, b, c) => {
                store.add_line(e, a, Side::Debit, usd(c))?;
                mark();
                store.add_line(e, b, Side::Credit, usd(c))
            }
            Op::CloseMembers(p, a, b) => store
                .close_period_to(
                    p,
                    &ledger_core::CloseTo::Members(vec![
                        ledger_core::MemberClose {
                            capital: a,
                            share: Decimal::new(5, 1),
                            distributions: None,
                        },
                        ledger_core::MemberClose {
                            capital: b,
                            share: Decimal::new(5, 1),
                            distributions: None,
                        },
                    ]),
                )
                .map(drop),
        }
    }

    /// The Lean model's `Inv` (verify/lean/Wanderbooks/Ledger.lean), checked
    /// on the real store: every posted entry is acceptable, the books net
    /// to zero, and no draft is dated in a closed period.
    fn check_invariants(store: &Store) -> Result<(), TestCaseError> {
        let l = store.ledger();
        for e in l.posted_entries() {
            prop_assert!(
                e.validate(l.chart()).is_ok(),
                "posted entry {} invalid",
                e.id()
            );
        }
        let tb = ledger_core::reports::trial_balance(l);
        prop_assert_eq!(tb.total_debits, tb.total_credits);
        prop_assert_eq!(store.list_classes().len(), l.list_classes().len());
        prop_assert_eq!(store.list_payroll_runs().len(), l.list_payroll_runs().len());
        for id in 1..l.next_entry_id() {
            if let Some(e) = l.get_entry(id)
                && e.status() == ledger_core::EntryStatus::Draft
            {
                prop_assert!(
                    l.closed_period_containing(e.date()).is_none(),
                    "draft {} is dated {} inside a closed period",
                    id,
                    e.date()
                );
            }
        }
        Ok(())
    }

    proptest! {
        // 256 cases on every check; `STORE_CASES=5000` for a deeper run.
        #![proptest_config(ProptestConfig {
            cases: std::env::var("STORE_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(256),
            failure_persistence: None,
            ..ProptestConfig::default()
        })]

        /// A refused command leaves the journal byte-identical, and after
        /// any sequence the books reopen and verify.
        #[test]
        fn refused_commands_never_reach_the_journal(
            ops in prop::collection::vec(arb_op(), 1..40),
            seed in any::<u64>(),
        ) {
            run_ops(&ops, seed)?;
        }
    }

    /// Run one sequence of ops against fresh books and check every law.
    fn run_ops(ops: &[Op], seed: u64) -> Result<(), TestCaseError> {
        let dir = test_dir(&format!("refusals-{seed}"));
        let mut store = Store::create(&dir, Currency::USD).unwrap();
        // Start from realistic books, so most commands act on real
        // things and refusals come from bad arguments (the first
        // version started empty and almost nothing ever posted).
        // Ids 1..=9: cash, equipment, accumulated depreciation, two
        // members' capital, a liability, sales, rent, depreciation.
        for (code, name, t) in [
            ("1000", "Cash", AccountType::Asset),
            ("1500", "Equipment", AccountType::Asset),
            ("1510", "Accumulated depreciation", AccountType::Asset),
            ("3000", "Capital A", AccountType::Equity),
            ("3001", "Capital B", AccountType::Equity),
            ("2500", "Loan", AccountType::Liability),
            ("4100", "Sales", AccountType::Revenue),
            ("6000", "Rent", AccountType::Expense),
            ("6200", "Depreciation", AccountType::Expense),
        ] {
            store.add_account(code, name, t, None, false).unwrap();
        }
        for (n, (dr, cr, cents)) in [(1u64, 4u64, 1_000_000i64), (8, 1, 120_000), (1, 7, 250_000)]
            .into_iter()
            .enumerate()
        {
            let e = store.create_entry(day(n as u32 * 10), "setup").unwrap();
            let m = Money::new(Decimal::new(cents, 2), Currency::USD);
            store.add_line(e, dr, Side::Debit, m).unwrap();
            store.add_line(e, cr, Side::Credit, m).unwrap();
            store.post_entry(e).unwrap();
        }
        let journal = dir.join(ledger_store::JOURNAL_FILE);
        for op in ops {
            check_invariants(&store)?;
            let ledger_before = store.ledger().clone();
            let mut before = std::fs::read_to_string(&journal).unwrap();
            let result = apply(&mut store, op, &mut || {
                before = std::fs::read_to_string(&journal).unwrap();
            });
            if std::env::var("STORE_STATS").is_ok() {
                let kind = format!("{op:?}");
                let kind = kind.split('(').next().unwrap_or("");
                match &result {
                    Ok(()) => eprintln!("OK {kind}"),
                    Err(e) => eprintln!("REFUSED {kind} {e}"),
                }
            }
            // The converse law: an accepted command is on the record, or
            // changed nothing at all (a batch run with nothing due; a
            // setting changed to the value it has; a 1099 decision, W-9
            // flag or payment method already so).
            if result.is_ok() {
                let after = std::fs::read_to_string(&journal).unwrap();
                if after.len() == before.len() {
                    let parts = ledger_before.differs_from(store.ledger());
                    prop_assert!(
                        parts.is_empty(),
                        "accepted {:?}, journaled nothing, but changed {:?}",
                        op,
                        parts
                    );
                    prop_assert!(
                        matches!(
                            op,
                            Op::RunDepreciation(..)
                                | Op::RunRecurring(..)
                                | Op::SetTaxPolicy(..)
                                | Op::SetVendor1099(..)
                                | Op::SetW9Flag(..)
                                | Op::SetEntryInstrument(..)
                        ),
                        "accepted {:?} but journaled nothing",
                        op
                    );
                }
            }
            if let Err(e) = result {
                let after = std::fs::read_to_string(&journal).unwrap();
                prop_assert!(
                    after == before,
                    "refused {:?} ({}) but the journal grew by:\n{}",
                    op,
                    e,
                    after.strip_prefix(&before).unwrap_or(&after)
                );
            }
        }
        check_invariants(&store)?;
        // Reopening gives back exactly these books (rank 3: the live
        // ledger against journal replay).
        // The ledger the commands built against its own journal's
        // replay, every part of it (rank 3).
        let verified = store.verify();
        prop_assert!(verified.is_ok(), "live vs. replay: {:?}", verified.err());
        let before = ledger_core::reports::snapshot(store.ledger()).unwrap();
        let live = store.ledger().clone();
        let attachments = store.list_attachments(None).unwrap().len();
        drop(store);
        let reopened = Store::open(&dir);
        prop_assert!(
            reopened.is_ok(),
            "books failed to reopen: {:?}",
            reopened.err()
        );
        let reopened = reopened.unwrap();
        reopened.verify().unwrap();
        prop_assert_eq!(
            ledger_core::reports::snapshot(reopened.ledger()).unwrap(),
            before
        );
        let parts = live.differs_from(reopened.ledger());
        prop_assert!(parts.is_empty(), "reopened books differ in: {:?}", parts);
        prop_assert_eq!(reopened.list_attachments(None).unwrap().len(), attachments);
        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    /// Regression (CI, 2026-10-13): a payroll run with unrounded employer
    /// tax posted $65.27794, and sums of it differed in scale between the
    /// journal replay and the SQLite read-back. Posting now refuses
    /// fractions of a cent (B-3; `no_entry_posts_a_fraction_of_the_minor_unit`);
    /// this replays the sequence with the tax rounded, as callers must.
    #[test]
    fn regression_payroll_with_a_fraction_of_a_cent() {
        run_ops(
            &[
                Op::Book(0, 1, 1, 1),
                Op::Book(0, 1, 1, 1),
                Op::CreateEntry(0),
                Op::RunPayroll(0, 105_287, 1, 1),
                Op::GoodClose(2),
                Op::Pair(6, 8, 1, 1),
                Op::Post(6),
            ],
            0,
        )
        .unwrap();
    }
}

/// Contract (2026-10-13): rules added to the engine never lock anyone out
/// of books written before them. A journal holding what the new rules
/// refuse (a child account of another type, partially overlapping periods,
/// a draft line on a missing account) must still open and verify; replay
/// goes through Ledger::replay_*. Each phase adds what its books hold (P0,
/// 2026-10-16: the journal format declared, `journal.format_set` format 1).
#[test]
fn books_written_under_older_rules_still_open() {
    use serde_json::json;
    let dir = test_dir("older-rules");
    let mut store = Store::create(&dir, Currency::USD).unwrap();
    let cash = store
        .add_account("1000", "Cash", AccountType::Asset, None, true)
        .unwrap();
    drop(store);
    // What an older wanderbooks could have written.
    let mut w = JournalWriter::open(&dir.join(ledger_store::JOURNAL_FILE)).unwrap();
    w.append("journal.format_set", json!({"format": 1, "features": []}))
        .unwrap();
    w.append(
        "account.added",
        json!({"id": 2, "code": "1010", "name": "Rent", "account_type": "Expense",
               "parent": cash, "placeholder": false, "currency": "USD"}),
    )
    .unwrap();
    w.append(
        "period.created",
        json!({"id": 1, "name": "2026-01", "start": "2026-01-01", "end": "2026-01-31"}),
    )
    .unwrap();
    w.append(
        "period.created",
        json!({"id": 2, "name": "straddle", "start": "2026-01-15", "end": "2026-02-15"}),
    )
    .unwrap();
    w.append(
        "entry.created",
        json!({"id": 1, "date": "2026-03-01", "description": "draft"}),
    )
    .unwrap();
    w.append(
        "entry.line_added",
        json!({"id": 1, "account": 999, "side": "debit", "amount": "10.00", "currency": "USD"}),
    )
    .unwrap();
    // A foreign line booked before rounding to the cent (2026-10-13).
    w.append(
        "entry.line_added",
        json!({"id": 1, "account": cash, "side": "credit", "amount": "108.34567", "currency": "USD",
               "foreign_currency": "EUR", "foreign_amount": "100.00", "fx_rate": "1.0834567"}),
    )
    .unwrap();
    drop(w);
    // Opening rebuilds from the journal; it must accept all of it.
    let _ = std::fs::remove_file(dir.join(ledger_store::db_file()));
    let store = Store::open(&dir).expect("older books still open");
    store.verify().unwrap();
    assert_eq!(store.journal_format().format, 1);
    assert_eq!(store.ledger().chart().len(), 2);
    assert_eq!(store.ledger().list_periods().len(), 2);
    let lines = store.ledger().get_entry(1).unwrap().lines();
    assert_eq!(lines.len(), 2);
    // Replayed exactly as booked, not re-rounded.
    assert_eq!(
        lines[1].amount.amount,
        "108.34567".parse::<Decimal>().unwrap()
    );
}

/// The books an earlier wanderbooks wrote, one directory each under
/// tests/fixtures/old-books: its journal (and blobs) exactly as it wrote
/// them, and expected.json, what that version computed from them. Each was
/// written through main 188c90ea's Store API (or, for x_older_rules, its
/// JournalWriter, as above), and expected.json by main's own code: verify,
/// the canonical report snapshot, and every record as replayed. They hold
/// what P0 bounds for new commands only (rules.md B-28): dates before 0001
/// and after 9999 (to chrono's first and last days), draft, recurring and
/// share amounts past the line limit (to Decimal's largest), a contributor
/// basis past it, sub-cent document and payroll lines, an asset life of
/// 10,000 years, schedules and assets running into chrono's last month,
/// depreciation posted for 10000-01, one recurring run of 1,044 drafts.
/// tests/fixtures/old-books-writer is the harness that wrote them; each
/// later phase adds the books its version writes.
fn old_books() -> Vec<PathBuf> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/old-books");
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    // The P0 review's thirteen and its control, and the edges added with
    // them (2026-10-17).
    assert!(dirs.len() >= 29, "{} old books", dirs.len());
    dirs
}

/// A fixture's books (journal and blobs, never expected.json) in a fresh
/// directory of its own for `test`: opening writes a database and a lock
/// beside them.
fn old_books_copy(fixture: &std::path::Path, test: &str) -> TestDir {
    fn copy(from: &std::path::Path, to: &std::path::Path) {
        std::fs::create_dir_all(to).unwrap();
        for e in std::fs::read_dir(from).unwrap() {
            let e = e.unwrap();
            if e.file_name() == "expected.json" {
                continue;
            }
            if e.file_type().unwrap().is_dir() {
                copy(&e.path(), &to.join(e.file_name()));
            } else {
                std::fs::copy(e.path(), to.join(e.file_name())).unwrap();
            }
        }
    }
    let name = fixture.file_name().unwrap().to_string_lossy();
    let dir = test_dir(&format!("old-books-{test}-{name}"));
    copy(fixture, &dir);
    dir
}

/// What expected.json records of a ledger (beside verify): the same JSON
/// main's harness wrote, field for field.
fn old_books_figures(l: &ledger_core::Ledger) -> serde_json::Value {
    let entries: Vec<_> = (1..l.next_entry_id())
        .filter_map(|id| l.get_entry(id))
        .collect();
    let parties: Vec<_> = (1..l.next_entry_id())
        .map(|id| (id, l.entry_parties(id)))
        .filter(|(_, p)| !p.is_empty())
        .collect();
    serde_json::json!({
        "snapshot": ledger_core::reports::snapshot(l).map_err(|e| e.to_string()),
        "entries": entries,
        "assets": l.registered_assets().collect::<Vec<_>>(),
        "periods": l.list_periods(),
        "schedules": l.schedules().collect::<Vec<_>>(),
        "recurring": l.recurring_entries().collect::<Vec<_>>(),
        "invoices": l.invoices(),
        "bills": l.bills(),
        "payroll_runs": l.list_payroll_runs(),
        "entry_parties": parties,
    })
}

/// Contract (posted is forever, rules.md B-28; P0 review round 4,
/// 2026-10-17): every book an earlier version wrote opens, in the writer
/// (Store::open, its database rebuilt from the journal) and the viewer
/// (Snapshot::load) alike, verifies as it did, and replays to exactly the
/// figures that version computed: the trial balance, P&L and balance sheet,
/// and every entry, asset, period, schedule, recurring entry, document,
/// payroll run and share, to the last decimal place as journaled.
#[test]
fn books_an_earlier_version_wrote_open_to_its_figures() {
    for fixture in old_books() {
        let name = fixture.file_name().unwrap().to_string_lossy().to_string();
        let expected: serde_json::Value =
            serde_json::from_slice(&std::fs::read(fixture.join("expected.json")).unwrap()).unwrap();
        let dir = old_books_copy(&fixture, "figures");
        let store = Store::open(&dir).unwrap_or_else(|e| panic!("{name}: the writer refused: {e}"));
        let verify = store.verify().map(|_| ()).map_err(|e| e.to_string());
        assert_eq!(
            serde_json::json!(verify),
            expected["verify"],
            "{name}: verify"
        );
        let mut figures = old_books_figures(store.ledger());
        figures["verify"] = expected["verify"].clone();
        assert_eq!(figures, expected, "{name}: the writer's figures");
        drop(store);
        // And again from the database it built (no rebuild this time).
        let store = Store::open(&dir).unwrap_or_else(|e| panic!("{name}: reopen: {e}"));
        let mut again = old_books_figures(store.ledger());
        again["verify"] = expected["verify"].clone();
        assert_eq!(again, expected, "{name}: reopened");
        drop(store);
        let snap = ledger_store::Snapshot::load(&dir)
            .unwrap_or_else(|e| panic!("{name}: the viewer refused: {e}"));
        let mut viewed = old_books_figures(snap.ledger());
        viewed["verify"] = expected["verify"].clone();
        assert_eq!(viewed, expected, "{name}: the viewer's figures");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Every figure the engine derives from old books (B-28: computed, or
/// refused with TooLargeToCompute or DateOutOfRange, never a panic): every
/// report over the books' whole span and the calendar's, every asset's
/// schedule and register row, every amortization plan, every recurring
/// entry's due dates, aging, 1099 and K-1 worksheets, shares and money
/// flows of every entry, documents, payroll, period closes, and
/// reconciliations. Returns how many were refused, so a run that refuses
/// everything can't pass unnoticed.
fn every_figure_of(l: &ledger_core::Ledger) -> (usize, usize) {
    use ledger_core::analysis::{self, Interval};
    use ledger_core::reports;
    let (mut ok, mut refused) = (0usize, 0usize);
    let mut tally = |r: Result<(), ledger_core::LedgerError>| match r {
        Ok(()) => ok += 1,
        Err(
            ledger_core::LedgerError::TooLargeToCompute(_)
            | ledger_core::LedgerError::DateOutOfRange(_),
        ) => refused += 1,
        // Any other refusal is a reason the books give (an unknown
        // account, a period already closed): answered, not a panic.
        Err(_) => refused += 1,
    };
    let dates: Vec<NaiveDate> = (1..l.next_entry_id())
        .filter_map(|id| l.get_entry(id))
        .map(|e| e.date())
        .chain(l.registered_assets().map(|a| a.in_service_date))
        .chain(l.list_periods().iter().flat_map(|p| [p.start, p.end]))
        .chain(l.schedules().map(|s| s.start))
        .collect();
    let first = dates.iter().min().copied().unwrap_or(NaiveDate::MIN);
    let last = dates.iter().max().copied().unwrap_or(NaiveDate::MAX);
    let spans = [
        (first, last),
        (
            NaiveDate::from_ymd_opt(1, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(9999, 12, 31).unwrap(),
        ),
        (NaiveDate::MIN, NaiveDate::MAX),
    ];
    let accounts: Vec<_> = l.chart().all().map(|a| a.id).collect();
    let cash: Vec<_> = l
        .chart()
        .all()
        .filter(|a| a.code == "1000")
        .map(|a| a.id)
        .collect();
    let members: Vec<reports::MemberCapital> = l
        .chart()
        .all()
        .filter(|a| a.account_type == AccountType::Equity)
        .map(|a| reports::MemberCapital {
            name: a.name.clone(),
            capital_account: a.id,
            distribution_account: None,
            share: Decimal::ONE,
        })
        .collect();
    tally(reports::snapshot(l).map(|_| ()));
    let _ = reports::trial_balance(l);
    tally(reports::balance_sheet(l).map(|_| ()));
    tally(reports::unclosed_earnings(l).map(|_| ()));
    tally(reports::profit_and_loss_by_class(l).map(|_| ()));
    for (start, end) in spans {
        for d in [start, end] {
            let _ = reports::trial_balance_as_of(l, d);
            tally(reports::balance_sheet_as_of(l, d).map(|_| ()));
            tally(l.ar_aging(d).map(|_| ()));
            tally(l.ap_aging(d).map(|_| ()));
            tally(ledger_core::fixed_asset_register(l, d).map(|_| ()));
            for r in l.recurring_entries() {
                // Weekly from year 1 through chrono's last day is
                // 13.7 million dates: through the books' own span only.
                if d <= last {
                    let _ = r.due(d);
                }
            }
            for s in l.schedules() {
                tally(s.due(d).map(|_| ()));
            }
            for a in l.registered_assets() {
                tally(ledger_core::depreciation_schedule(a, d).map(|_| ()));
                tally(ledger_core::scheduled_book_value(a, d).map(|_| ()));
                tally(
                    ledger_core::disposal_amounts(
                        a,
                        d,
                        Money::new(Decimal::ZERO, l.base_currency()),
                    )
                    .map(|_| ()),
                );
            }
        }
        tally(reports::profit_and_loss_for_period(l, start, end, None).map(|_| ()));
        tally(reports::general_ledger(l, start, end).map(|_| ()));
        tally(reports::members_equity(l, start, end, &members).map(|_| ()));
        // The statement of cash flows foots on every book, or is refused
        // for a reason: never an engine error (rules.md B-14 rule 9).
        let cf = reports::cash_flow(
            l,
            &reports::CashFlowParams {
                start,
                end,
                cash_accounts: cash.clone(),
                financing_accounts: vec![],
                investing_accounts: vec![],
                members: members.clone(),
            },
        );
        assert!(
            !matches!(
                cf,
                Err(ledger_core::LedgerError::CashFlowDoesNotFoot { .. })
            ),
            "{cf:?}"
        );
        tally(cf.map(|_| ()));
        tally(ledger_core::k1::k1_worksheet(l, start, end, &members, &cash).map(|_| ()));
        tally(ledger_core::tax_summary(l, start, end, None).map(|_| ()));
        let _ = reports::cancelled_in_span(l, start, end);
        // Interval series over a span of 524,000 years are not what anyone
        // asks: the books' own span, and the calendar's in years.
        if (end - start).num_days() < 400 * 366 {
            for interval in [Interval::Month, Interval::Quarter, Interval::Year] {
                let pieces = analysis::intervals(start, end, interval);
                let ends: Vec<NaiveDate> = pieces.iter().map(|(_, e)| *e).collect();
                let _ = analysis::balance_series(l, &accounts, &ends);
                tally(analysis::pnl_by_interval(l, start, end, interval).map(|_| ()));
                let _ = analysis::flows_by_interval(l, &accounts, start, end, interval);
            }
        } else {
            let _ = analysis::intervals(start, end, Interval::Year).len();
        }
    }
    let _ = analysis::balance_history(l, &accounts);
    for year in [first.year(), last.year(), 2026] {
        let sources = ledger_core::form1099::PaymentAccounts {
            cash: cash.clone(),
            financing: vec![],
        };
        tally(
            ledger_core::form1099::vendor_1099_report(l, year, None, &sources, &[], None)
                .map(|_| ()),
        );
    }
    let money = ledger_core::parties::money_accounts(l, &cash, &[]);
    let party_members: Vec<ledger_core::PartyMember> = l
        .parties()
        .map(|p| ledger_core::PartyMember {
            party: p.id,
            share: Decimal::new(5, 1),
            capital: members.first().map_or(0, |m| m.capital_account),
            distributions: None,
        })
        .collect();
    for id in 1..l.next_entry_id() {
        let Some(e) = l.get_entry(id) else { continue };
        tally(ledger_core::parties::entry_flow(e, &money).map(|_| ()));
        tally(ledger_core::shares::target(l, id, l.entry_parties(id), &money).map(|_| ()));
        tally(ledger_core::parties::flow(l, id, l.entry_parties(id), &money).map(|_| ()));
        let _ = ledger_core::shares::resolve(l, id, l.entry_parties(id), &money, &party_members);
        let _ = e.validate(l.chart());
    }
    for p in l.parties() {
        for (start, end) in spans {
            let _ =
                ledger_core::parties::activity_with(l, p.id, (start, end), &money, &party_members);
        }
    }
    for inv in l.invoices() {
        tally(inv.total().map(|_| ()));
        tally(inv.balance().map(|_| ()));
    }
    for bill in l.bills() {
        tally(bill.total().map(|_| ()));
        tally(bill.balance().map(|_| ()));
    }
    for run in l.list_payroll_runs() {
        for line in &run.lines {
            tally(line.net_pay().map(|_| ()));
        }
    }
    for s in l.schedules() {
        tally(s.plan().map(|_| ()));
        tally(s.remaining().map(|_| ()));
    }
    for p in l.list_periods() {
        tally(l.preview_close_period(p.id).map(|_| ()));
    }
    for &account in &cash {
        for d in [first, last] {
            for balance in [Decimal::ZERO, Decimal::MAX, Decimal::MIN] {
                tally(l.reconciliation_report(account, d, balance).map(|_| ()));
            }
        }
    }
    (ok, refused)
}

/// B-28 over every old book: each figure is computed or refused, never a
/// panic, in the writer's ledger and the viewer's.
#[test]
fn every_figure_over_old_books_is_computed_or_refused() {
    let (mut ok, mut refused) = (0, 0);
    for fixture in old_books() {
        let dir = old_books_copy(&fixture, "derived");
        let store = Store::open(&dir).unwrap();
        let (o, r) = every_figure_of(store.ledger());
        drop(store);
        let snap = ledger_store::Snapshot::load(&dir).unwrap();
        assert_eq!(
            every_figure_of(snap.ledger()),
            (o, r),
            "{}",
            fixture.display()
        );
        ok += o;
        refused += r;
        let _ = std::fs::remove_dir_all(&dir);
    }
    // Most figures compute; the edges refuse.
    assert!(ok > refused, "{ok} computed, {refused} refused");
    assert!(refused > 0, "none refused: the edges weren't reached");
}

/// Regression (fuzz_journal_line, 2026-10-13, crash-c9fa20f3): a journal
/// line holding a number too large for an integer parsed as a float, and
/// its canonical form (which the hash chain covers) re-canonicalized
/// differently. The journal never holds floats (R-2), so the reader now
/// refuses them; float-free lines are a fixed point.
#[test]
fn a_journal_line_with_a_float_is_refused() {
    use ledger_store::JournalLine;
    let crash = br#"{"payload":4444454444444444404444,"prev":"x","seq":8,"ts":";u","type":""}"#;
    assert!(JournalLine::parse(crash).is_err());
    assert!(
        JournalLine::parse(br#"{"payload":{"amount":1.5},"prev":"x","seq":1,"ts":"t","type":"a"}"#)
            .is_err()
    );
    // Anywhere in the line: inside an array too (an entry's lines).
    assert!(
        JournalLine::parse(
            br#"{"payload":{"lines":[{"amount":"1.00"},{"amount":2.5}]},"prev":"x","seq":1,"ts":"t","type":"a"}"#
        )
        .is_err()
    );
    let ok = br#"{"payload":{"amount":"1.50","id":7},"prev":"x","seq":1,"ts":"t","type":"a"}"#;
    let line = JournalLine::parse(ok).unwrap();
    let canon = line.to_canonical();
    assert_eq!(
        JournalLine::parse(canon.as_bytes()).unwrap().to_canonical(),
        canon
    );
}

/// Regression (max-testing pass on the store, 2026-10-13): nothing stopped
/// two processes (say, two agent sessions, each with its own MCP server)
/// from writing the same books. One write from each broke the hash chain
/// ("seq gap") and the books would not open. Oracle (rank 1, R-11: one
/// append-only log, one writer): a second writer is refused while the
/// first is open, and allowed once it closes; the books always reopen.
#[test]
fn a_second_writer_is_refused_while_the_first_has_the_books() {
    let dir = test_dir("two-writers");
    drop(Store::create(&dir, Currency::USD).unwrap());
    let mut first = Store::open(&dir).unwrap();
    match Store::open(&dir) {
        Err(StoreError::Locked(why)) => assert!(why.contains("another"), "{why}"),
        Err(e) => panic!("expected Locked, got {e}"),
        Ok(_) => panic!("a second writer was allowed"),
    }
    first
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    drop(first);
    // Released on close: the next writer gets in, and the books are whole.
    let reopened = Store::open(&dir).unwrap();
    reopened.verify().unwrap();
    assert_eq!(reopened.ledger().chart().len(), 1);
}

/// Security (rank 2; max-testing pass on the store, 2026-10-13): a blob id
/// comes from the journal, which is untrusted input on disk. Whatever the
/// id, the blob store never panics and never touches a path outside its
/// own directory: only a 64-hex-digit id names a blob.
#[test]
fn a_blob_id_never_reaches_outside_the_blob_store() {
    let dir = test_dir("blob-ids");
    let blobs = ledger_store::BlobStore::open(&dir.join("blobs")).unwrap();
    let outside = dir.join("secret.txt");
    std::fs::write(&outside, b"not a blob").unwrap();
    for id in [
        "",
        "a",
        "ab",
        "abc",
        "é",
        "日本語",
        "../secret.txt",
        "../../secret.txt",
        "ab/../../secret.txt",
        "/etc/passwd",
        "zz",
        &"g".repeat(64),
        &"a".repeat(63),
        &"a".repeat(65),
    ] {
        assert!(blobs.get(id).is_err(), "{id:?} was served");
        assert!(!blobs.exists(id), "{id:?} probed a path");
    }
    let (sha, _) = blobs.put(b"receipt").unwrap();
    assert_eq!(blobs.get(&sha).unwrap(), b"receipt");
    assert_eq!(blobs.get(&sha.to_uppercase()).unwrap(), b"receipt");
}

/// Regression (found by the refusal property on 2026-10-13): scrapping an
/// asset (no proceeds) is a valid disposal. It posts without a zero
/// proceeds line, books the whole book value as a loss, and survives
/// reopen. Before the fix it journaled a catch-up depreciation entry and a
/// half-built disposal draft, then refused.
#[test]
fn scrapping_an_asset_for_nothing_disposes_it() {
    let dir = test_dir("scrap");
    let mut store = Store::create(&dir, Currency::USD).unwrap();
    let mut add = |code: &str, t| store_add(&mut store, code, t);
    let cash = add("1000", AccountType::Asset);
    let equipment = add("1500", AccountType::Asset);
    let accum = add("1510", AccountType::Asset);
    let loss = add("6900", AccountType::Expense);
    let depr = add("6200", AccountType::Expense);
    let day = |m, d| NaiveDate::from_ymd_opt(2026, m, d).unwrap();
    let buy = store.create_entry(day(1, 1), "laptop").unwrap();
    store
        .add_line(buy, equipment, Side::Debit, usd(1200))
        .unwrap();
    store.add_line(buy, cash, Side::Credit, usd(1200)).unwrap();
    store.post_entry(buy).unwrap();
    let asset = store
        .register_asset(ledger_core::NewAsset {
            description: "laptop".into(),
            in_service_date: day(1, 1),
            cost: usd(1200),
            salvage_value: usd(0),
            useful_life_months: 12,
            cost_account: equipment,
            accum_depr_account: accum,
            expense_account: depr,
        })
        .unwrap();
    let entry = store
        .dispose_asset(asset, day(3, 31), usd(0), cash, loss)
        .unwrap();
    let lines = store.ledger().get_entry(entry).unwrap().lines().to_vec();
    assert!(lines.iter().all(|l| !l.amount.amount.is_zero()));
    assert!(lines.iter().all(|l| l.account != cash), "no proceeds line");
    // January through March depreciated (300), so a 900 loss on 1,200.
    let balance = |s: &Store, a| s.ledger().account_balance(a).unwrap();
    assert_eq!(balance(&store, loss), Decimal::from(900));
    assert_eq!(balance(&store, equipment), Decimal::ZERO);
    assert_eq!(balance(&store, accum), Decimal::ZERO);
    drop(store);
    let store = Store::open(&dir).unwrap();
    store.verify().unwrap();
    assert_eq!(balance(&store, loss), Decimal::from(900));
}

fn store_add(store: &mut Store, code: &str, t: AccountType) -> ledger_core::AccountId {
    store.add_account(code, code, t, None, false).unwrap()
}

/// Contract (rules.md B-13): each members close journals its rounding rule
/// and replays under it: "cumulative_since" with the close its count starts
/// at for new closes (P3), "cumulative" for the run rule of 2026-10-13 to
/// P3, "largest_remainder" for the per-close rule of earlier on 2026-10-13.
/// Three one-cent months, 50/50: both cumulative rules end A 0.02 / B
/// 0.01, per close A 0.03 / B 0.00. With February closed to retained
/// earnings instead, the two cumulative rules differ: cumulative_since
/// skips it (the running 0.02 splits A 0.01 / B 0.01), the run rule
/// restarts at March (A gets both cents: A 0.02 / B 0.00).
#[test]
fn members_closes_replay_under_the_rule_they_were_journaled_with() {
    use ledger_core::{CloseTo, MemberClose};
    type Rule = fn(Vec<MemberClose>) -> CloseTo;
    let since: Rule = CloseTo::Members;
    let run: Rule = CloseTo::MembersRun;
    let per: Rule = CloseTo::MembersPerClose;
    for (name, to, re_february, rule, a, b) in [
        (
            "since",
            since,
            false,
            "\"rounding\":\"cumulative_since\"",
            "0.02",
            "0.01",
        ),
        (
            "run",
            run,
            false,
            "\"rounding\":\"cumulative\"",
            "0.02",
            "0.01",
        ),
        (
            "per",
            per,
            false,
            "\"rounding\":\"largest_remainder\"",
            "0.03",
            "0.00",
        ),
        (
            "since-re",
            since,
            true,
            "\"rounding\":\"cumulative_since\"",
            "0.01",
            "0.01",
        ),
        (
            "run-re",
            run,
            true,
            "\"rounding\":\"cumulative\"",
            "0.02",
            "0.00",
        ),
    ] {
        let dir = test_dir(&format!("close-rule-{name}"));
        let mut store = Store::create(&dir, Currency::USD).unwrap();
        let cash = store
            .add_account("1000", "Cash", AccountType::Asset, None, false)
            .unwrap();
        let caps = [
            store
                .add_account("3000", "A", AccountType::Equity, None, false)
                .unwrap(),
            store
                .add_account("3001", "B", AccountType::Equity, None, false)
                .unwrap(),
        ];
        let rev = store
            .add_account("4000", "Sales", AccountType::Revenue, None, false)
            .unwrap();
        let members: Vec<MemberClose> = caps
            .iter()
            .map(|&capital| MemberClose {
                capital,
                share: Decimal::new(5, 1),
                distributions: None,
            })
            .collect();
        let cent = Money::new(Decimal::new(1, 2), Currency::USD);
        for m in 1..=3u32 {
            let day = |d| NaiveDate::from_ymd_opt(2026, m, d).unwrap();
            let e = store.create_entry(day(10), "a cent").unwrap();
            store.add_line(e, cash, Side::Debit, cent).unwrap();
            store.add_line(e, rev, Side::Credit, cent).unwrap();
            store.post_entry(e).unwrap();
            let end = NaiveDate::from_ymd_opt(2026, m + 1, 1)
                .unwrap()
                .pred_opt()
                .unwrap();
            let p = store
                .create_period(&format!("2026-{m:02}"), day(1), end)
                .unwrap();
            if re_february && m == 2 {
                store.close_period(p).unwrap();
            } else {
                store.close_period_to(p, &to(members.clone())).unwrap();
            }
        }
        let journal = std::fs::read_to_string(dir.join(ledger_store::JOURNAL_FILE)).unwrap();
        let closes = if re_february { 2 } else { 3 };
        assert_eq!(journal.matches(rule).count(), closes, "{name}");
        if rule.contains("cumulative_since") {
            // Every count starts at the first close: February's close to
            // retained earnings is skipped, not a restart.
            assert_eq!(journal.matches("\"since_seq\":1").count(), closes, "{name}");
        }
        drop(store);
        let store = Store::open(&dir).unwrap();
        store.verify().unwrap();
        let bal = |id| store.ledger().account_balance(id).unwrap();
        assert_eq!(bal(caps[0]), a.parse::<Decimal>().unwrap(), "{name}");
        assert_eq!(bal(caps[1]), b.parse::<Decimal>().unwrap(), "{name}");
    }
}

/// The viewer's journal accessors: the lines it has read, in order, and
/// the hash the next line must chain onto (the SHA-256 of the last line's
/// canonical form; docs/receipt-store.md "Line format").
#[test]
fn the_viewer_exposes_the_lines_it_read_and_their_head() {
    let dir = test_dir("viewer-head");
    let store = build_books(&dir);
    drop(store);
    let snap = ledger_store::Snapshot::load(&dir).unwrap();
    let lines =
        ledger_store::journal::read_and_verify_journal(&dir.join(ledger_store::JOURNAL_FILE))
            .unwrap();
    assert_eq!(snap.journal().len(), lines.len());
    assert_eq!(snap.journal_seq(), lines.last().unwrap().seq);
    assert_eq!(
        snap.head_hash(),
        ledger_store::journal::sha256_hex(lines.last().unwrap().to_canonical().as_bytes())
    );
}

/// The scrap test's books: a 1,200 laptop, 12-month life, bought Jan 1,
/// paid in cash. Returns (store, asset, cash, gain/loss account).
fn laptop(dir: &std::path::Path) -> (Store, u64, u64, u64) {
    let mut store = Store::create(dir, Currency::USD).unwrap();
    let mut add = |code: &str, t| store_add(&mut store, code, t);
    let cash = add("1000", AccountType::Asset);
    let equipment = add("1500", AccountType::Asset);
    let accum = add("1510", AccountType::Asset);
    let gain = add("4900", AccountType::Revenue);
    let depr = add("6200", AccountType::Expense);
    let day = |m, d| NaiveDate::from_ymd_opt(2026, m, d).unwrap();
    let buy = store.create_entry(day(1, 1), "laptop").unwrap();
    store
        .add_line(buy, equipment, Side::Debit, usd(1200))
        .unwrap();
    store.add_line(buy, cash, Side::Credit, usd(1200)).unwrap();
    store.post_entry(buy).unwrap();
    let asset = store
        .register_asset(ledger_core::NewAsset {
            description: "laptop".into(),
            in_service_date: day(1, 1),
            cost: usd(1200),
            salvage_value: usd(0),
            useful_life_months: 12,
            cost_account: equipment,
            accum_depr_account: accum,
            expense_account: depr,
        })
        .unwrap();
    (store, asset, cash, gain)
}

/// Hand-computed: January's 100 posted, sold Mar 31 for 1,000. Through
/// Mar 31 the schedule says 300, so the final catch-up is 300 - 100 = 200,
/// accumulated 300, book value 900, a gain of 100.
#[test]
fn a_disposal_catches_up_exactly_what_was_not_yet_posted() {
    let dir = test_dir("dispose-catch-up");
    let (mut store, asset, cash, gain) = laptop(&dir);
    store.run_depreciation(asset, "2026-01").unwrap();
    let day = NaiveDate::from_ymd_opt(2026, 3, 31).unwrap();
    let p = store.dispose_preview(asset, day, usd(1000)).unwrap();
    assert_eq!(
        (
            p.final_depreciation,
            p.accumulated,
            p.book_value,
            p.gain_loss
        ),
        (
            Decimal::from(200),
            Decimal::from(300),
            Decimal::from(900),
            Decimal::from(100)
        )
    );
    store
        .dispose_asset(asset, day, usd(1000), cash, gain)
        .unwrap();
    assert_eq!(
        store.ledger().account_balance(gain).unwrap(),
        Decimal::from(100)
    );
    store.verify().unwrap();
}

/// A disposal may be dated the day the asset was bought, never before it.
#[test]
fn a_disposal_may_fall_on_the_purchase_date_but_not_before() {
    let dir = test_dir("dispose-dates");
    let (mut store, asset, cash, gain) = laptop(&dir);
    let jan1 = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let before = jan1.pred_opt().unwrap();
    assert!(store.dispose_preview(asset, before, usd(1200)).is_err());
    assert!(
        store
            .dispose_asset(asset, before, usd(1200), cash, gain)
            .is_err()
    );
    let p = store.dispose_preview(asset, jan1, usd(1200)).unwrap();
    assert_eq!(p.book_value, Decimal::from(1200) - p.accumulated);
    store
        .dispose_asset(asset, jan1, usd(1200), cash, gain)
        .unwrap();
    // Disposed: a second disposal is refused.
    assert!(store.dispose_preview(asset, jan1, usd(1)).is_err());
    store.verify().unwrap();
}

/// A close to retained earnings that sweeps a draws account journals the
/// sweep, so replay zeroes the draws again (the bare "retained_earnings"
/// form is only for a close with nothing to sweep).
#[test]
fn a_retained_earnings_close_with_a_sweep_replays_it() {
    let dir = test_dir("re-sweep");
    let mut store = Store::create(&dir, Currency::USD).unwrap();
    let mut add = |code: &str, t| store_add(&mut store, code, t);
    let cash = add("1000", AccountType::Asset);
    let draws = add("3100", AccountType::Equity);
    let sales = add("4000", AccountType::Revenue);
    let day = |d| NaiveDate::from_ymd_opt(2026, 1, d).unwrap();
    for (dr, cr, n) in [(cash, sales, 500u64), (draws, cash, 200)] {
        let e = store.create_entry(day(10), "x").unwrap();
        store.add_line(e, dr, Side::Debit, usd(n)).unwrap();
        store.add_line(e, cr, Side::Credit, usd(n)).unwrap();
        store.post_entry(e).unwrap();
    }
    let p = store.create_period("2026-01", day(1), day(31)).unwrap();
    store
        .close_period_to(
            p,
            &ledger_core::CloseTo::RetainedEarnings { sweep: vec![draws] },
        )
        .unwrap();
    let re = store.ledger().chart().find_by_code("3999").unwrap().id;
    let state = |s: &Store| {
        (
            s.ledger().account_balance(draws).unwrap(),
            s.ledger().account_balance(re).unwrap(),
        )
    };
    // Net income 500 in, draws 200 swept out: retained earnings 300.
    assert_eq!(state(&store), (Decimal::ZERO, Decimal::from(300)));
    drop(store);
    let store = Store::open(&dir).unwrap();
    store.verify().unwrap();
    assert_eq!(state(&store), (Decimal::ZERO, Decimal::from(300)));
}

/// The viewer tells a database that hasn't applied the newest lines
/// (Behind) from one that has applied lines the viewer hasn't read yet
/// (Ahead), and neither is a false "differs".
#[test]
fn the_viewer_tells_behind_from_ahead() {
    use ledger_store::{DiskCheck, Snapshot};
    let dir = test_dir("behind-ahead");
    let store = build_books(&dir);
    drop(store);
    // Ahead: the viewer loads, then the writer appends and applies.
    let snap = Snapshot::load(&dir).unwrap();
    let mut store = Store::open(&dir).unwrap();
    store.add_customer("Acme").unwrap();
    assert!(
        matches!(snap.check_disk(), DiskCheck::Ahead { .. }),
        "{:?}",
        snap.check_disk()
    );
    // Behind: a database restored from before that write.
    drop(store);
    let mut store = Store::open(&dir).unwrap();
    let before_write = std::fs::read(dir.join(ledger_store::db_file())).unwrap();
    store.add_vendor("Supplier").unwrap();
    drop(store);
    std::fs::write(dir.join(ledger_store::db_file()), &before_write).unwrap();
    let _ = std::fs::remove_file(dir.join(format!("{}-wal", ledger_store::db_file())));
    let _ = std::fs::remove_file(dir.join(format!("{}-shm", ledger_store::db_file())));
    let snap = Snapshot::load(&dir).unwrap();
    match snap.check_disk() {
        DiskCheck::Behind { applied, journal } => assert!(applied < journal),
        other => panic!("expected Behind, got {other:?}"),
    }
}

/// The table check says how many rows are missing and how many are
/// unexpected, so a person can tell a lost row from a stray one.
#[test]
fn the_table_check_counts_missing_and_unexpected_rows() {
    let dir = test_dir("table-counts");
    let store = build_books(&dir);
    drop(store);
    let conn = rusqlite::Connection::open(dir.join(ledger_store::db_file())).unwrap();
    // The events table mirrors the journal and isn't read back into the
    // ledger, so only the table check sees this.
    conn.execute(
        "DELETE FROM events WHERE seq = (SELECT MAX(seq) FROM events)",
        [],
    )
    .unwrap();
    drop(conn);
    let store = Store::open(&dir).unwrap();
    let err = store.verify().unwrap_err().to_string();
    assert!(
        err.contains("table events: 1 rows missing, 0 unexpected"),
        "{err}"
    );
}

/// A journal writer for a path whose directory doesn't exist yet creates
/// it; the blob store reports a stored file as existing, and an unknown
/// one as not.
#[test]
fn a_new_journal_makes_its_directory_and_blobs_exist_once_stored() {
    let dir = test_dir("new-dirs");
    let path = dir
        .join("nested")
        .join("deeper")
        .join(ledger_store::JOURNAL_FILE);
    let mut w = JournalWriter::open(&path).unwrap();
    w.append("test.line", serde_json::json!({"n": 1})).unwrap();
    assert!(path.exists());
    let blobs = ledger_store::BlobStore::open(&dir.join("blobs")).unwrap();
    let (sha, _) = blobs.put(b"receipt").unwrap();
    assert!(blobs.exists(&sha));
    assert!(!blobs.exists(&"0".repeat(64)));
}

/// Store::open SCENARIO 6: a database that records a base currency other
/// than the journal's is the projection's fault, not the journal's: it is
/// kept aside and rebuilt (before 2026-10-13 the books refused to open,
/// blaming the journal).
#[test]
fn a_database_with_the_wrong_currency_is_rebuilt() {
    let dir = test_dir("db-currency");
    let store = build_books(&dir);
    let before = ledger_core::reports::snapshot(store.ledger()).unwrap();
    drop(store);
    let conn = rusqlite::Connection::open(dir.join(ledger_store::db_file())).unwrap();
    conn.execute(
        "UPDATE meta SET value = 'EUR' WHERE key = 'base_currency'",
        [],
    )
    .unwrap();
    drop(conn);
    let store = Store::open(&dir).unwrap();
    assert_eq!(store.base_currency(), Currency::USD);
    assert_eq!(
        ledger_core::reports::snapshot(store.ledger()).unwrap(),
        before
    );
    store.verify().unwrap();
    let kept = std::fs::read_dir(&dir)
        .unwrap()
        .filter(|e| {
            e.as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains(".corrupt.")
        })
        .count();
    assert_eq!(kept, 1, "the wrong database is kept for forensics");
}

/// A disposal's proceeds go to an asset account and its gain or loss to a
/// revenue or expense account; anything else is refused before anything
/// is journaled.
#[test]
fn a_disposal_refuses_accounts_of_the_wrong_type() {
    let dir = test_dir("dispose-types");
    let (mut store, asset, cash, gain) = laptop(&dir);
    let jan31 = NaiveDate::from_ymd_opt(2026, 1, 31).unwrap();
    let journal = dir.join(ledger_store::JOURNAL_FILE);
    let before = std::fs::read(&journal).unwrap();
    // Proceeds into a revenue account, gain/loss into an asset account.
    assert!(
        store
            .dispose_asset(asset, jan31, usd(900), gain, gain)
            .is_err()
    );
    assert!(
        store
            .dispose_asset(asset, jan31, usd(900), cash, cash)
            .is_err()
    );
    assert_eq!(std::fs::read(&journal).unwrap(), before);
    store
        .dispose_asset(asset, jan31, usd(900), cash, gain)
        .unwrap();
}

/// A depreciation period is "YYYY-MM": month 1 to 12, year 1 or later.
#[test]
fn depreciation_periods_must_be_real_months() {
    let dir = test_dir("periods");
    let (mut store, asset, _, _) = laptop(&dir);
    for bad in ["2026-13", "2026-00", "0-05", "2026-01-01", "2026", "x-05"] {
        let err = store.run_depreciation(asset, bad).unwrap_err().to_string();
        assert!(err.contains("bad period"), "{bad}: {err}");
    }
    // Year 1 is a real year (whatever else the asset makes of it).
    if let Err(e) = store.run_depreciation(asset, "0001-01") {
        assert!(!e.to_string().contains("bad period"), "{e}");
    }
    store.run_depreciation(asset, "2026-02").unwrap();
}

/// Searching posted entries: by account, date range, and text, newest
/// first, with a limit and offset; drafts never show.
#[test]
fn searching_entries_filters_and_pages() {
    let dir = test_dir("search");
    let mut store = Store::create(&dir, Currency::USD).unwrap();
    let mut add = |code: &str, t| store_add(&mut store, code, t);
    let cash = add("1000", AccountType::Asset);
    let sales = add("4000", AccountType::Revenue);
    let rent = add("6000", AccountType::Expense);
    let day = |m, d| NaiveDate::from_ymd_opt(2026, m, d).unwrap();
    let mut post = |date, desc: &str, dr, cr| {
        let e = store.create_entry(date, desc).unwrap();
        store.add_line(e, dr, Side::Debit, usd(10)).unwrap();
        store.add_line(e, cr, Side::Credit, usd(10)).unwrap();
        store.post_entry(e).unwrap();
        e
    };
    let a = post(day(1, 5), "Sale to Acme", cash, sales);
    let b = post(day(2, 5), "Rent February", rent, cash);
    let c = post(day(3, 5), "Sale to Beta", cash, sales);
    store.create_entry(day(3, 6), "Sale draft").unwrap();
    let ids =
        |rows: Vec<ledger_store::store::EntryRow>| rows.iter().map(|r| r.id).collect::<Vec<_>>();
    assert_eq!(
        ids(store.search_entries(None, None, None, None, 10, 0).unwrap()),
        [c, b, a]
    );
    assert_eq!(
        ids(store
            .search_entries(Some(rent), None, None, None, 10, 0)
            .unwrap()),
        [b]
    );
    assert_eq!(
        ids(store
            .search_entries(None, Some("2026-02-01"), Some("2026-03-01"), None, 10, 0)
            .unwrap()),
        [b]
    );
    assert_eq!(
        ids(store
            .search_entries(None, None, None, Some("Sale"), 10, 0)
            .unwrap()),
        [c, a]
    );
    assert_eq!(
        ids(store.search_entries(None, None, None, None, 1, 1).unwrap()),
        [b]
    );
}

/// The database's invoice row follows its payments: amount paid and
/// open / partial / paid, matching the ledger.
#[test]
fn the_database_tracks_what_an_invoice_has_been_paid() {
    use ledger_core::arap::DocumentLine;
    let dir = test_dir("invoice-paid");
    let mut store = Store::create(&dir, Currency::USD).unwrap();
    let cash = store_add(&mut store, "1000", AccountType::Asset);
    let who = store.add_customer("Acme").unwrap();
    let day = |d| NaiveDate::from_ymd_opt(2026, 1, d).unwrap();
    let inv = store
        .create_invoice(
            who,
            day(1),
            day(31),
            vec![DocumentLine {
                description: "work".into(),
                amount: Decimal::from(100),
            }],
        )
        .unwrap();
    let row = |dir: &std::path::Path| -> (String, String) {
        let conn = rusqlite::Connection::open(dir.join(ledger_store::db_file())).unwrap();
        conn.query_row(
            "SELECT amount_paid, status FROM invoices WHERE id = ?",
            [inv as i64],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap()
    };
    assert_eq!(row(&dir).1, "open");
    store
        .receive_payment(inv, day(10), Decimal::from(40), cash)
        .unwrap();
    assert_eq!(row(&dir), ("40".into(), "partial".into()));
    store
        .receive_payment(inv, day(20), Decimal::from(60), cash)
        .unwrap();
    assert_eq!(row(&dir), ("100".into(), "paid".into()));
    store.verify().unwrap();
}

/// Rewrite a journal through a change to its lines, re-chaining it, so
/// the result passes the hash chain and reaches replay.
fn rechain(dir: &std::path::Path, change: impl Fn(&mut Vec<ledger_store::JournalLine>)) {
    let path = dir.join(ledger_store::JOURNAL_FILE);
    let mut lines = ledger_store::journal::read_and_verify_journal(&path).unwrap();
    change(&mut lines);
    std::fs::remove_file(&path).unwrap();
    let _ = std::fs::remove_file(dir.join(ledger_store::db_file()));
    let mut w = JournalWriter::open(&path).unwrap();
    for l in lines {
        w.append(&l.line_type, l.payload).unwrap();
    }
}

/// Replay re-derives a closing entry and refuses a journal whose recorded
/// entry differs from it (a hand edit that re-chained the file), and a
/// database missing a table fails the table check.
#[test]
fn replay_refuses_a_journaled_entry_it_does_not_rederive() {
    let dir = test_dir("rederive");
    let mut store = Store::create(&dir, Currency::USD).unwrap();
    let cash = store_add(&mut store, "1000", AccountType::Asset);
    let sales = store_add(&mut store, "4000", AccountType::Revenue);
    let day = |d| NaiveDate::from_ymd_opt(2026, 1, d).unwrap();
    let e = store.create_entry(day(10), "sale").unwrap();
    store.add_line(e, cash, Side::Debit, usd(500)).unwrap();
    store.add_line(e, sales, Side::Credit, usd(500)).unwrap();
    store.post_entry(e).unwrap();
    let p = store.create_period("2026-01", day(1), day(31)).unwrap();
    store.close_period(p).unwrap();
    drop(store);
    // An untouched re-chain opens fine (the helper itself is sound).
    rechain(&dir, |_| {});
    drop(Store::open(&dir).unwrap());
    // The recorded closing entry says 400 where replay computes 500.
    rechain(&dir, |lines| {
        let close = lines
            .iter_mut()
            .find(|l| l.line_type == "period.closed")
            .unwrap();
        for l in close.payload["entry"]["lines"].as_array_mut().unwrap() {
            l["amount"] = serde_json::json!("400.00");
        }
    });
    let err = Store::open(&dir).err().expect("refused").to_string();
    assert!(err.contains("differ"), "{err}");

    // A projection missing a table.
    let dir = test_dir("missing-table");
    drop(build_books(&dir));
    let conn = rusqlite::Connection::open(dir.join(ledger_store::db_file())).unwrap();
    conn.execute("DROP TABLE events", []).unwrap();
    drop(conn);
    let store = Store::open(&dir);
    let msg = match store {
        Ok(s) => s.verify().unwrap_err().to_string(),
        Err(e) => e.to_string(),
    };
    assert!(msg.contains("events"), "{msg}");
}

/// Replay re-derives a schedule's monthly posting and refuses a journal
/// whose recorded entry differs from it.
#[test]
fn replay_refuses_a_schedule_posting_it_does_not_rederive() {
    let dir = test_dir("rederive-schedule");
    let mut store = Store::create(&dir, Currency::USD).unwrap();
    let prepaid = store_add(&mut store, "1300", AccountType::Asset);
    let insurance = store_add(&mut store, "6300", AccountType::Expense);
    let day = |m, d| NaiveDate::from_ymd_opt(2026, m, d).unwrap();
    let id = store
        .register_schedule(ledger_core::NewSchedule {
            description: "Insurance".into(),
            kind: ledger_core::ScheduleKind::PrepaidExpense,
            total: usd(1200),
            start: day(1, 1),
            months: 12,
            balance_account: prepaid,
            pl_account: insurance,
            convention: ledger_core::ScheduleConvention::MonthlyEven,
        })
        .unwrap();
    store.post_schedule_month(id, day(1, 31)).unwrap();
    drop(store);
    rechain(&dir, |_| {});
    drop(Store::open(&dir).unwrap());
    rechain(&dir, |lines| {
        let posted = lines
            .iter_mut()
            .find(|l| l.line_type == "schedule.posted")
            .unwrap();
        for l in posted.payload["entry"]["lines"].as_array_mut().unwrap() {
            l["amount"] = serde_json::json!("99.00");
        }
    });
    let err = Store::open(&dir).err().expect("refused").to_string();
    assert!(err.contains("differ"), "{err}");
}

/// A disposal refused for an account's type says which type it needed.
#[test]
fn a_disposal_account_refusal_names_the_type_needed() {
    let dir = test_dir("dispose-type-message");
    let (mut store, asset, cash, gain) = laptop(&dir);
    let jan31 = NaiveDate::from_ymd_opt(2026, 1, 31).unwrap();
    let proceeds = store
        .dispose_asset(asset, jan31, usd(900), gain, gain)
        .unwrap_err()
        .to_string();
    let gain_loss = store
        .dispose_asset(asset, jan31, usd(900), cash, cash)
        .unwrap_err()
        .to_string();
    assert!(proceeds.contains("asset"), "{proceeds}");
    assert!(gain_loss.contains("revenue or expense"), "{gain_loss}");
}

/// A database whose pages are damaged though its settings still read
/// fails SQLite's integrity check, and is kept aside and rebuilt.
#[test]
fn a_database_with_damaged_pages_is_rebuilt() {
    let dir = test_dir("damaged-pages");
    let mut store = build_books(&dir);
    for n in 0..200u64 {
        store
            .add_customer(&format!("customer {n} {}", "x".repeat(100)))
            .unwrap();
    }
    let before = ledger_core::reports::snapshot(store.ledger()).unwrap();
    drop(store);
    let db = dir.join(ledger_store::db_file());
    let mut bytes = std::fs::read(&db).unwrap();
    // Scramble only the last pages (the customers just added); the header,
    // schema, and settings at the front still read.
    let tail = bytes.len() - bytes.len() / 4;
    for b in bytes.iter_mut().skip(tail).step_by(7) {
        *b ^= 0x5a;
    }
    std::fs::write(&db, &bytes).unwrap();
    let store = Store::open(&dir).unwrap();
    store.verify().unwrap();
    assert_eq!(
        ledger_core::reports::snapshot(store.ledger()).unwrap(),
        before
    );
    let kept = std::fs::read_dir(&dir)
        .unwrap()
        .filter(|e| {
            e.as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains(".corrupt.")
        })
        .count();
    assert_eq!(kept, 1);
}

// ---------------------------------------------------------------------------
// P3: renaming an account, and the tax category policy (defects 15 and 8;
// rules.md B-31, B-32).
// ---------------------------------------------------------------------------

/// Every row of every projection table but the journal's own (`events`,
/// `meta`), as text, with `accounts.name` blanked: what a rename may
/// change is that one column.
fn projection_but_names(dir: &std::path::Path) -> Vec<(String, Vec<String>)> {
    let conn = rusqlite::Connection::open(dir.join(ledger_store::db_file())).unwrap();
    let mut tables: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    // The journal's own rows (every line, and its head) grow with every
    // command: what a rename adds there is checked from the journal.
    tables.retain(|t| !t.starts_with("sqlite_") && t != "events" && t != "meta");
    tables
        .into_iter()
        .map(|t| {
            let mut stmt = conn.prepare(&format!("SELECT * FROM \"{t}\"")).unwrap();
            let names: Vec<String> = stmt.column_names().iter().map(|c| c.to_string()).collect();
            let mut rows: Vec<String> = stmt
                .query_map([], |r| {
                    Ok((0..names.len())
                        .map(|i| {
                            if t == "accounts" && names[i] == "name" {
                                "-".to_string()
                            } else {
                                format!("{:?}", r.get_ref(i).unwrap())
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("|"))
                })
                .unwrap()
                .map(Result::unwrap)
                .collect();
            rows.sort();
            (t, rows)
        })
        .collect()
}

/// Every figure of the books with account names left out: the trial
/// balance, the P&L and balance sheet (whole and for January), and every
/// entry's lines.
fn figures_but_names(l: &ledger_core::Ledger) -> serde_json::Value {
    use ledger_core::reports;
    let jan = |d: u32| NaiveDate::from_ymd_opt(2026, 1, d).unwrap();
    let lines = |v: &[ledger_core::StatementLine]| {
        v.iter()
            .map(|x| (x.code.clone(), x.amount.normalize().to_string()))
            .collect::<Vec<_>>()
    };
    let tb = reports::trial_balance(l);
    let pl = reports::profit_and_loss_for_period(l, jan(1), jan(31), None).unwrap();
    let bs = reports::balance_sheet_as_of(l, jan(31)).unwrap();
    serde_json::json!({
        "tb": tb.lines.iter().map(|x| (x.code.clone(), x.debit.normalize().to_string(), x.credit.normalize().to_string())).collect::<Vec<_>>(),
        "tb_totals": (tb.total_debits.normalize().to_string(), tb.total_credits.normalize().to_string()),
        "pl": (lines(&pl.revenue), lines(&pl.expenses), pl.net_income().normalize().to_string()),
        "bs": (lines(&bs.assets), lines(&bs.liabilities), lines(&bs.equity), bs.total_assets.normalize().to_string()),
        "entries": (1..l.next_entry_id()).filter_map(|id| l.get_entry(id)).collect::<Vec<_>>(),
    })
}

/// rename_account (repro 1.2; rules.md B-31): a rename changes the
/// account's name and nothing else: no figure, entry or line, no earlier
/// journal line, no projection row but `accounts.name`; the journal gains
/// one `account.renamed` line (keyed by id, with the old name and the
/// reason) after the feature's declaration; a refused rename (unknown,
/// blank, unchanged, a sibling's name, nine digits, a bad reason) appends
/// nothing; the name survives a reopen, a rebuild and the viewer's read.
#[test]
fn rename_account_changes_only_the_name_defect_15() {
    let dir = test_dir("rename-only-name");
    let mut store = build_books(&dir);
    let journal = dir.join(ledger_store::JOURNAL_FILE);
    let text = std::fs::read_to_string(&journal).unwrap();
    let figures = figures_but_names(store.ledger());
    let rows = projection_but_names(&dir);
    let rent = store.ledger().chart().find_by_code("6000").unwrap().id;
    for (id, name, reason) in [
        (999, "Office Rent", None),
        (rent, "", None),
        (rent, "  Rent Expense ", None),
        (rent, "cash", None),
        (rent, "Lease 123456789", None),
        (rent, "Office Rent", Some("")),
        (rent, "Office Rent", Some("lease no. 123-45-6789")),
    ] {
        assert!(
            store.rename_account(id, name, reason).is_err(),
            "{name:?} {reason:?}"
        );
        assert_eq!(std::fs::read_to_string(&journal).unwrap(), text, "{name:?}");
        assert_eq!(projection_but_names(&dir), rows, "{name:?}");
    }
    let (seq, _) = store.head();
    store
        .rename_account(rent, " Office Rent ", Some("owner asked 2026-10-21"))
        .unwrap();
    let after = std::fs::read_to_string(&journal).unwrap();
    // Every earlier line kept, byte for byte; one line appended, the
    // rename (its feature, account_renamed, the books declared as they
    // were created, with the rest of this version's format).
    assert!(after.starts_with(&text));
    assert!(
        store
            .journal_format()
            .has_feature(ledger_store::format::FEATURE_ACCOUNT_RENAMED)
    );
    let added: Vec<serde_json::Value> = after[text.len()..]
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(added.len(), 1);
    assert_eq!(added[0]["type"], "account.renamed");
    assert_eq!(
        added[0]["payload"],
        serde_json::json!({
            "account_id": rent, "code": "6000", "name": "Office Rent",
            "previous": "Rent Expense", "reason": "owner asked 2026-10-21"
        })
    );
    assert_eq!(store.head().0, seq + 1);
    assert_eq!(
        store.ledger().chart().get(rent).unwrap().name,
        "Office Rent"
    );
    assert_eq!(figures_but_names(store.ledger()), figures);
    assert_eq!(projection_but_names(&dir), rows);
    // The one column a rename changes, read from the projection itself
    // (projection_but_names blanks it).
    let db_name = |dir: &std::path::Path| -> String {
        rusqlite::Connection::open(dir.join(ledger_store::db_file()))
            .unwrap()
            .query_row(
                "SELECT name FROM accounts WHERE id = ?",
                [rent as i64],
                |r| r.get(0),
            )
            .unwrap()
    };
    assert_eq!(db_name(&dir), "Office Rent");
    store.verify().unwrap();
    // A second rename declares nothing more.
    store.rename_account(rent, "Rent", None).unwrap();
    let again = std::fs::read_to_string(&journal).unwrap();
    let last: serde_json::Value = serde_json::from_str(again.lines().last().unwrap()).unwrap();
    assert_eq!(again.lines().count(), after.lines().count() + 1);
    assert_eq!(last["payload"]["previous"], "Office Rent");
    assert_eq!(last["payload"]["reason"], serde_json::Value::Null);
    drop(store);
    let reopened = Store::open(&dir).unwrap();
    reopened.verify().unwrap();
    assert_eq!(reopened.ledger().chart().get(rent).unwrap().name, "Rent");
    assert_eq!(figures_but_names(reopened.ledger()), figures);
    drop(reopened);
    let _ = std::fs::remove_file(dir.join(ledger_store::db_file()));
    let rebuilt = Store::open(&dir).unwrap();
    assert_eq!(rebuilt.ledger().chart().get(rent).unwrap().name, "Rent");
    assert_eq!(projection_but_names(&dir), rows);
    assert_eq!(db_name(&dir), "Rent");
    drop(rebuilt);
    let snap = ledger_store::Snapshot::load(&dir).unwrap();
    assert_eq!(snap.ledger().chart().get(rent).unwrap().name, "Rent");
}

/// rules.md B-28's pair for renames (review R5): the command refuses a
/// blank name, a sibling's or nine digits; replay takes a journaled line
/// holding any of them (a later version may tighten the rules, never lock
/// out books it wrote), and refuses only an account that doesn't exist.
#[test]
fn a_journaled_rename_replays_past_the_command_rules() {
    use serde_json::json;
    for (name, opens) in [
        ("", true),
        ("Cash", true),
        ("Lease 123456789", true),
        ("Rent Expense", true),
    ] {
        let dir = test_dir("rename-replay");
        let store = build_books(&dir);
        let rent = store.ledger().chart().find_by_code("6000").unwrap().id;
        drop(store);
        let mut w = JournalWriter::open(&dir.join(ledger_store::JOURNAL_FILE)).unwrap();
        w.append(
            "journal.format_set",
            json!({"format": 2, "features": ["account_renamed"]}),
        )
        .unwrap();
        w.append(
            "account.renamed",
            json!({"account_id": rent, "code": "6000", "name": name, "previous": "Rent Expense", "reason": null}),
        )
        .unwrap();
        drop(w);
        let _ = std::fs::remove_file(dir.join(ledger_store::db_file()));
        let store = Store::open(&dir);
        assert_eq!(store.is_ok(), opens, "{name:?}");
        let store = store.unwrap();
        store.verify().unwrap();
        assert_eq!(store.ledger().chart().get(rent).unwrap().name, name);
    }
    // An account that doesn't exist: the line can't be replayed.
    let dir = test_dir("rename-replay-unknown");
    drop(build_books(&dir));
    let mut w = JournalWriter::open(&dir.join(ledger_store::JOURNAL_FILE)).unwrap();
    w.append(
        "journal.format_set",
        json!({"format": 2, "features": ["account_renamed"]}),
    )
    .unwrap();
    w.append(
        "account.renamed",
        json!({"account_id": 99, "code": "9", "name": "x", "previous": "y", "reason": null}),
    )
    .unwrap();
    drop(w);
    let _ = std::fs::remove_file(dir.join(ledger_store::db_file()));
    assert!(Store::open(&dir).is_err());
}

fn tax_policy(store: &mut Store, value: Option<&str>) {
    store
        .change_settings(
            &[ledger_core::settings::Change {
                key: "policy.require_tax_category".into(),
                value: value.map(|v| serde_json::json!(v)),
            }],
            &ledger_core::settings::Source {
                decided_by: "Owner".into(),
                decided_on: NaiveDate::from_ymd_opt(2026, 10, 20).unwrap(),
                citation: None,
                note: None,
            },
        )
        .unwrap();
}

/// add_account_with_tax and the policy (rules.md B-32): under warn an
/// account is created without a category; under refuse a revenue or
/// expense account that isn't a header is refused without one, journaling
/// nothing, and created with one as account.added then account.tax_set;
/// a needed category can't be cleared; the accounts the engine creates
/// for a control role are never refused. Setting the key needs the feature
/// require_tax_category, which new books declared as they were created:
/// the setting journals only itself.
#[test]
fn the_tax_category_policy_refuses_only_what_it_says() {
    let dir = test_dir("tax-policy");
    let mut store = build_books(&dir);
    let journal = dir.join(ledger_store::JOURNAL_FILE);
    store
        .add_account("6150", "Software", AccountType::Expense, None, false)
        .unwrap();
    let before = std::fs::read_to_string(&journal).unwrap();
    tax_policy(&mut store, Some("refuse"));
    let declared = std::fs::read_to_string(&journal).unwrap();
    let new: Vec<serde_json::Value> = declared[before.len()..]
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(new.len(), 1);
    assert_eq!(new[0]["type"], "settings.set");
    for (code, t, header, tax, refused) in [
        ("6160", AccountType::Expense, false, None, true),
        ("4100", AccountType::Revenue, false, None, true),
        ("6170", AccountType::Expense, true, None, false),
        ("1100", AccountType::Asset, false, None, false),
        ("2100", AccountType::Liability, false, None, false),
        ("3100", AccountType::Equity, false, None, false),
        (
            "6180",
            AccountType::Expense,
            false,
            Some(("bogus", None)),
            true,
        ),
        (
            "4200",
            AccountType::Revenue,
            false,
            Some(("supplies", None)),
            true,
        ),
    ] {
        let text = std::fs::read_to_string(&journal).unwrap();
        let got = store.add_account_with_tax(code, &format!("Acct {code}"), t, None, header, tax);
        assert_eq!(got.is_err(), refused, "{code} {got:?}");
        if refused {
            assert_eq!(std::fs::read_to_string(&journal).unwrap(), text, "{code}");
            assert!(store.ledger().chart().find_by_code(code).is_none());
        }
    }
    let err = store
        .add_account("6160", "Hosting", AccountType::Expense, None, false)
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("policy.require_tax_category is refuse"),
        "{err}"
    );
    assert!(err.contains("6160 Hosting"), "{err}");
    // With a category: two lines, account.added then account.tax_set.
    let text = std::fs::read_to_string(&journal).unwrap();
    let id = store
        .add_account_with_tax(
            "6160",
            "Hosting",
            AccountType::Expense,
            None,
            false,
            Some(("other_deductions", Some(Decimal::from(80)))),
        )
        .unwrap();
    let added: Vec<serde_json::Value> = std::fs::read_to_string(&journal).unwrap()[text.len()..]
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(
        added
            .iter()
            .map(|l| l["type"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["account.added", "account.tax_set"]
    );
    let a = store.ledger().chart().get(id).unwrap();
    assert_eq!(a.tax_category.as_deref(), Some("other_deductions"));
    assert_eq!(a.deductible_percent, Some(Decimal::from(80)));
    // A needed category is never cleared; a header's or an asset's is no
    // concern; an existing uncategorized account is left as it was.
    let text = std::fs::read_to_string(&journal).unwrap();
    assert!(store.set_account_tax(id, None, None).is_err());
    let software = store.ledger().chart().find_by_code("6150").unwrap().id;
    assert!(store.set_account_tax(software, None, None).is_err());
    assert_eq!(std::fs::read_to_string(&journal).unwrap(), text);
    assert!(
        store
            .ledger()
            .chart()
            .get(software)
            .unwrap()
            .tax_category
            .is_none()
    );
    store
        .set_account_tax(software, Some("other_deductions"), None)
        .unwrap();
    // The engine's own accounts: a bill makes 6999 General Expenses with
    // no category, under refuse too.
    let v = store.add_vendor("Coastline Hosting").unwrap();
    store
        .create_bill(
            v,
            NaiveDate::from_ymd_opt(2026, 2, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 3, 1).unwrap(),
            vec![ledger_core::DocumentLine {
                description: "hosting".into(),
                amount: Decimal::new(4_200, 2),
            }],
        )
        .unwrap();
    let general = store.ledger().chart().find_by_code("6999").unwrap();
    assert!(general.tax_category.is_none());
    // Back to warn: no second declaration, and the account is created.
    let text = std::fs::read_to_string(&journal).unwrap();
    tax_policy(&mut store, Some("warn"));
    let lines: Vec<serde_json::Value> = std::fs::read_to_string(&journal).unwrap()[text.len()..]
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["type"], "settings.set");
    store
        .add_account("6190", "Domains", AccountType::Expense, None, false)
        .unwrap();
    store.verify().unwrap();
    drop(store);
    Store::open(&dir).unwrap().verify().unwrap();
}

/// rules.md B-28's pair for the policy (review R5): the policy is the
/// commands', never replay's. A journal that sets refuse and then clears
/// a category, or adds an uncategorized expense account, replays.
#[test]
fn a_journaled_clear_replays_under_refuse() {
    use serde_json::json;
    let dir = test_dir("tax-policy-replay");
    let mut store = build_books(&dir);
    let rent = store.ledger().chart().find_by_code("6000").unwrap().id;
    store
        .set_account_tax(rent, Some("rent_property"), None)
        .unwrap();
    tax_policy(&mut store, Some("refuse"));
    let next = store.ledger().next_account_id();
    drop(store);
    let mut w = JournalWriter::open(&dir.join(ledger_store::JOURNAL_FILE)).unwrap();
    w.append(
        "account.tax_set",
        json!({"account_id": rent, "tax_category": null, "deductible_percent": null}),
    )
    .unwrap();
    w.append(
        "account.added",
        json!({"id": next, "code": "6150", "name": "Software", "account_type": "Expense",
               "parent": null, "placeholder": false, "currency": "USD"}),
    )
    .unwrap();
    drop(w);
    let _ = std::fs::remove_file(dir.join(ledger_store::db_file()));
    let store = Store::open(&dir).unwrap();
    store.verify().unwrap();
    assert!(
        store
            .ledger()
            .chart()
            .get(rent)
            .unwrap()
            .tax_category
            .is_none()
    );
    assert!(store.ledger().chart().find_by_code("6150").is_some());
    assert_eq!(
        store.ledger().settings().require_tax_category(),
        ledger_core::settings::TaxCategoryPolicy::Refuse
    );
}

mod p3_properties {
    use super::*;
    use proptest::prelude::*;

    const NAMES: [&str; 8] = [
        "Office",
        "office",
        "Cash",
        "",
        "Lease 123456789",
        "Rent Expense",
        "Studio",
        "Tools",
    ];

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: std::env::var("STORE_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(64),
            failure_persistence: None,
            ..ProptestConfig::default()
        })]

        /// rules.md B-31 (the engine-side counterpart to the Lean note,
        /// docs/verification.md law 15): on books with postings, any sequence
        /// of renames changes no figure (the ledger's statements, without
        /// names), no entry and no earlier journal line; live equals
        /// replay; the projection equals a fresh one. The sets picked by
        /// name (cash, card accounts) are presentation, outside this law.
        #[test]
        fn prop_rename_changes_no_figure(
            renames in prop::collection::vec((1u64..6, 0usize..NAMES.len()), 1..12),
            seed in any::<u64>(),
        ) {
            let dir = test_dir(&format!("prop-rename-{seed}"));
            let mut store = build_books(&dir);
            let journal = dir.join(ledger_store::JOURNAL_FILE);
            let figures = figures_but_names(store.ledger());
            let rows = projection_but_names(&dir);
            let mut text = std::fs::read_to_string(&journal).unwrap();
            for (account, n) in renames {
                let ok = store.rename_account(account, NAMES[n], None).is_ok();
                let now = std::fs::read_to_string(&journal).unwrap();
                prop_assert!(now.starts_with(&text));
                prop_assert_eq!(ok, now.len() > text.len());
                text = now;
                prop_assert_eq!(&figures_but_names(store.ledger()), &figures);
                prop_assert_eq!(&projection_but_names(&dir), &rows);
            }
            prop_assert!(store.verify().is_ok());
            let live = store.ledger().clone();
            drop(store);
            let reopened = Store::open(&dir).unwrap();
            prop_assert!(live.differs_from(reopened.ledger()).is_empty());
        }

        /// rules.md B-32: add_account_with_tax is refused exactly when the
        /// policy is refuse, the account needs a category (revenue or
        /// expense, not a header) and none is given, or when the category
        /// doesn't fit; a refusal journals nothing.
        #[test]
        fn prop_tax_policy_refuses_exactly(
            refuse in any::<bool>(),
            t in 0usize..5,
            header in any::<bool>(),
            cat in 0usize..4,
            seed in any::<u64>(),
        ) {
            let types = [
                AccountType::Asset,
                AccountType::Liability,
                AccountType::Equity,
                AccountType::Revenue,
                AccountType::Expense,
            ];
            let cats = [None, Some("other_income"), Some("supplies"), Some("bogus")];
            let dir = test_dir(&format!("prop-tax-{seed}"));
            let mut store = build_books(&dir);
            tax_policy(&mut store, Some(if refuse { "refuse" } else { "warn" }));
            let journal = dir.join(ledger_store::JOURNAL_FILE);
            let text = std::fs::read_to_string(&journal).unwrap();
            let t = types[t];
            let needs = matches!(t, AccountType::Revenue | AccountType::Expense) && !header;
            let fits = match cats[cat] {
                None => true,
                Some("other_income") => matches!(t, AccountType::Revenue | AccountType::Expense),
                Some("supplies") => t == AccountType::Expense,
                _ => false,
            };
            let want_refused = !fits || (refuse && needs && cats[cat].is_none());
            let got = store.add_account_with_tax(
                "7000", "Prop", t, None, header, cats[cat].map(|c| (c, None)),
            );
            prop_assert_eq!(got.is_err(), want_refused, "{:?}", got);
            let after = std::fs::read_to_string(&journal).unwrap();
            if want_refused {
                prop_assert_eq!(after, text);
            } else {
                let lines = after[text.len()..].lines().count();
                prop_assert_eq!(lines, if cats[cat].is_some() { 2 } else { 1 });
            }
        }
    }
}

/// Each account as the books hold it, by id: what chart.json in the
/// p3_rename fixture records (expected.json's figures carry no names).
fn chart_of(l: &ledger_core::Ledger) -> serde_json::Value {
    let mut accounts: Vec<_> = l.chart().all().collect();
    accounts.sort_by_key(|a| a.id);
    serde_json::json!(accounts)
}

fn p3_fixture() -> PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/old-books/p3_rename")
}

/// How the p3_rename fixture was written (2026-10-20, P3): through the
/// Store API of the version that introduced `account.renamed` and
/// `policy.require_tax_category`, with their journal features, a header
/// and its children, refused commands between them. Run once with
/// `--ignored`; the journal it wrote is frozen in the fixture, and later
/// versions must open it to the same figures
/// (books_an_earlier_version_wrote_open_to_its_figures) and names
/// (books_written_under_p3_keep_their_names).
#[test]
#[ignore]
fn write_p3_rename_fixture() {
    let dir = test_dir("p3-fixture");
    let mut s = Store::create(&dir, Currency::USD).unwrap();
    let day = |m: u32, d: u32| NaiveDate::from_ymd_opt(2026, m, d).unwrap();
    let bank = s
        .add_account("1000", "Bank", AccountType::Asset, None, false)
        .unwrap();
    let capital = s
        .add_account("3000", "Capital", AccountType::Equity, None, false)
        .unwrap();
    let ops = s
        .add_account("6000", "Operating", AccountType::Expense, None, true)
        .unwrap();
    let software = s
        .add_account("6150", "Software", AccountType::Expense, Some(ops), false)
        .unwrap();
    s.add_account("6160", "Hosting", AccountType::Expense, Some(ops), false)
        .unwrap();
    s.add_account_with_tax(
        "4000",
        "Sales",
        AccountType::Revenue,
        None,
        false,
        Some(("gross_receipts", None)),
    )
    .unwrap();
    tax_policy(&mut s, Some("refuse"));
    assert!(
        s.add_account("6170", "Domains", AccountType::Expense, Some(ops), false)
            .is_err()
    );
    s.add_account_with_tax(
        "6170",
        "Domains",
        AccountType::Expense,
        Some(ops),
        false,
        Some(("other_deductions", None)),
    )
    .unwrap();
    for (m, d, dr, cr, cents) in [
        (1u32, 5u32, bank, capital, 500_000i64),
        (2, 3, software, bank, 4_999),
        (3, 9, software, bank, 4_999),
    ] {
        let e = s.create_entry(day(m, d), "fixture").unwrap();
        let money = Money::new(Decimal::new(cents, 2), Currency::USD);
        s.add_line(e, dr, Side::Debit, money).unwrap();
        s.add_line(e, cr, Side::Credit, money).unwrap();
        s.post_entry(e).unwrap();
    }
    assert!(s.rename_account(software, "hosting", None).is_err());
    s.rename_account(
        software,
        "Software subscriptions",
        Some("owner asked 2026-10-20"),
    )
    .unwrap();
    s.rename_account(ops, "Operating expenses", None).unwrap();
    assert!(s.set_account_tax(software, None, None).is_err());
    s.set_account_tax(software, Some("other_deductions"), None)
        .unwrap();
    s.rename_account(software, "Software", Some("back to the short name"))
        .unwrap();
    tax_policy(&mut s, None);
    s.verify().unwrap();
    let out = p3_fixture();
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    std::fs::copy(
        dir.join(ledger_store::JOURNAL_FILE),
        out.join(ledger_store::JOURNAL_FILE),
    )
    .unwrap();
    let mut figures = old_books_figures(s.ledger());
    figures["verify"] = serde_json::json!({"Ok": null});
    std::fs::write(
        out.join("expected.json"),
        serde_json::to_string_pretty(&figures).unwrap() + "\n",
    )
    .unwrap();
    std::fs::write(
        out.join("chart.json"),
        serde_json::to_string_pretty(&chart_of(s.ledger())).unwrap() + "\n",
    )
    .unwrap();
}

/// Books written under P3 (fixture p3_rename): every account's name,
/// code, parent and tax category, as journaled through renames and the
/// tax category policy, read back the same by the writer, a rebuild and
/// the viewer; and the features it declared are the ones this version
/// knows.
#[test]
fn books_written_under_p3_keep_their_names() {
    let fixture = p3_fixture();
    let want: serde_json::Value =
        serde_json::from_slice(&std::fs::read(fixture.join("chart.json")).unwrap()).unwrap();
    let dir = old_books_copy(&fixture, "p3-names");
    let store = Store::open(&dir).unwrap();
    store.verify().unwrap();
    assert_eq!(chart_of(store.ledger()), want);
    assert_eq!(
        store.journal_format().features,
        ["account_renamed", "require_tax_category"]
    );
    let names: Vec<&str> = want
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "Bank",
            "Capital",
            "Operating expenses",
            "Software",
            "Hosting",
            "Sales",
            "Domains"
        ]
    );
    drop(store);
    let _ = std::fs::remove_file(dir.join(ledger_store::db_file()));
    let rebuilt = Store::open(&dir).unwrap();
    assert_eq!(chart_of(rebuilt.ledger()), want);
    drop(rebuilt);
    let snap = ledger_store::Snapshot::load(&dir).unwrap();
    assert_eq!(chart_of(snap.ledger()), want);
}

/// rules.md B-31: a new account's name is journaled trimmed, as a rename
/// journals one, so the live books, a reopen and the projection all read
/// the same name; the command's text rules refuse a name a report can't
/// print on one line, journaling nothing.
#[test]
fn a_new_accounts_name_is_journaled_trimmed() {
    let dir = test_dir("add-trimmed");
    let mut store = build_books(&dir);
    let journal = dir.join(ledger_store::JOURNAL_FILE);
    let text = std::fs::read_to_string(&journal).unwrap();
    for name in ["Two\nlines", &"N".repeat(201)] {
        assert!(
            store
                .add_account("6150", name, AccountType::Expense, None, false)
                .is_err(),
            "{name:?}"
        );
        assert_eq!(std::fs::read_to_string(&journal).unwrap(), text);
    }
    let id = store
        .add_account("6150", "  Hosting\t", AccountType::Expense, None, false)
        .unwrap();
    let after = std::fs::read_to_string(&journal).unwrap();
    let line: serde_json::Value = serde_json::from_str(after.lines().last().unwrap()).unwrap();
    assert_eq!(line["type"], "account.added");
    assert_eq!(line["payload"]["name"], "Hosting");
    assert_eq!(store.ledger().chart().get(id).unwrap().name, "Hosting");
    let live = store.ledger().clone();
    drop(store);
    let _ = std::fs::remove_file(dir.join(ledger_store::db_file()));
    let reopened = Store::open(&dir).unwrap();
    assert!(live.differs_from(reopened.ledger()).is_empty());
    let conn = rusqlite::Connection::open(dir.join(ledger_store::db_file())).unwrap();
    let name: String = conn
        .query_row("SELECT name FROM accounts WHERE id = ?", [id as i64], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(name, "Hosting");
}
