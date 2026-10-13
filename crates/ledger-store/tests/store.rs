//! Store tests: the durability story, proven.
//!
//! Round-trip persistence, tamper detection, rebuild-from-journal healing,
//! blob deduplication and hash verification.

use chrono::NaiveDate;
use ledger_core::{AccountType, Currency, Money, Side};
use ledger_store::{JournalWriter, Store, StoreError};
use rust_decimal::Decimal;
use std::path::PathBuf;

fn usd(amount: u64) -> Money {
    Money::new(Decimal::from(amount), Currency::USD)
}

fn test_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("wanderbooks-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
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

#[test]
fn torn_tail_is_truncatable() {
    let dir = test_dir("torntail");
    let store = build_books(&dir);
    let seq_before = {
        let w = JournalWriter::open(&dir.join(ledger_store::JOURNAL_FILE)).unwrap();
        w.next_seq() - 1
    };
    drop(store);

    // Simulate a torn backup: append half a line.
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(dir.join(ledger_store::JOURNAL_FILE))
        .unwrap();
    f.write_all(b"{\"seq\": 999, \"trunca").unwrap();
    f.sync_all().unwrap();
    drop(f);

    // Strict open now fails...
    assert!(Store::open(&dir).is_err());
    // ...but truncation recovers to the last good seq.
    let last_good =
        ledger_store::journal::truncate_to_last_good_seq(&dir.join(ledger_store::JOURNAL_FILE))
            .unwrap();
    assert_eq!(last_good, seq_before);
    let store = Store::open(&dir).unwrap();
    store.verify().unwrap();
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
    std::fs::remove_file(dir.join(ledger_store::DB_FILE)).unwrap();
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
    let db_path = dir.join(ledger_store::DB_FILE);
    std::fs::write(&db_path, b"this is not a sqlite database at all").unwrap();
    // Reopen heals from the journal instead of failing.
    let store = Store::open(&dir).unwrap();
    // The damaged file was quarantined for forensics.
    let quarantined: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("wanderbooks.corrupt.")
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

#[test]
fn vendor_1099_tags_replay_and_project() {
    use ledger_core::arap::DocumentLine;
    use ledger_core::{Form1099, vendor_1099_report};
    let dir = test_dir("form1099");
    let mut store = Store::create(&dir, Currency::USD).unwrap();
    let cash = store
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    let labor = store
        .add_account("6000", "Contract Labor", AccountType::Expense, None, false)
        .unwrap();
    let dana = store.add_vendor("Dana").unwrap();
    store
        .set_vendor_1099(dana, Some(Form1099::Nec), true)
        .unwrap();
    let d = |m, day| NaiveDate::from_ymd_opt(2026, m, day).unwrap();
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
    let before = vendor_1099_report(store.ledger(), 2026, None).unwrap();
    assert_eq!(before.vendors[0].paid, Decimal::from(2200));
    assert!(before.vendors[0].reportable);
    drop(store);

    // Reopen (replay), then rebuild the projection from scratch.
    for rebuild in [false, true] {
        if rebuild {
            std::fs::remove_file(dir.join(ledger_store::DB_FILE)).unwrap();
        }
        let store = Store::open(&dir).unwrap();
        assert_eq!(
            vendor_1099_report(store.ledger(), 2026, None).unwrap(),
            before
        );
        let conn = rusqlite::Connection::open(dir.join(ledger_store::DB_FILE)).unwrap();
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
    let db = dir.join(ledger_store::DB_FILE);
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
        SetVendor1099(u64, bool),
        SetAccountTax(u64, u8),
    }

    fn day(n: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 1, 1).unwrap() + chrono::Days::new(u64::from(n % 120))
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
            (0u64..4, any::<bool>()).prop_map(|(v, t)| Op::SetVendor1099(v, t)),
            (0u64..10, 0u8..4).prop_map(|(a, k)| Op::SetAccountTax(a, k)),
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

    fn apply(store: &mut Store, op: &Op) -> Result<(), StoreError> {
        let usd = |c: i64| Money::new(Decimal::new(c, 2), Currency::USD);
        let line = |c: i64| {
            vec![DocumentLine {
                description: "work".into(),
                amount: Decimal::new(c, 2),
            }]
        };
        match *op {
            Op::AddAccount(c, t, p, h) => store
                .add_account(CODES[c as usize], "acct", TYPES[t as usize], p, h)
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
            Op::SetVendor1099(v, t) => store.set_vendor_1099(v, Some(Form1099::Nec), t),
            Op::SetAccountTax(a, k) => store.set_account_tax(a, TAX[k as usize], None),
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
        #![proptest_config(ProptestConfig {
            cases: 1000,
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
            let dir = test_dir(&format!("refusals-{seed}"));
            let mut store = Store::create(&dir, Currency::USD).unwrap();
            let journal = dir.join(ledger_store::JOURNAL_FILE);
            for op in &ops {
                check_invariants(&store)?;
                let before = std::fs::read_to_string(&journal).unwrap();
                if let Err(e) = apply(&mut store, op) {
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
            drop(store);
            let reopened = Store::open(&dir);
            prop_assert!(reopened.is_ok(), "books failed to reopen: {:?}", reopened.err());
            reopened.unwrap().verify().unwrap();
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}

/// Contract (2026-10-13): rules added to the engine never lock anyone out
/// of books written before them. A journal holding what the new rules
/// refuse (a child account of another type, partially overlapping periods,
/// a draft line on a missing account) must still open and verify; replay
/// goes through Ledger::replay_*.
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
    let _ = std::fs::remove_file(dir.join(ledger_store::DB_FILE));
    let store = Store::open(&dir).expect("older books still open");
    store.verify().unwrap();
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
