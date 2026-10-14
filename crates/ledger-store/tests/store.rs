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
        RegisterSchedule(bool, i64, u32, u32, u64, u64),
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
        GoodSchedule(bool, i64, u32),
        GoodInvoice(u32, i64),
        GoodBill(u32, i64),
        GoodClose(u32),
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
                0u64..10
            )
                .prop_map(|(k, t, st, m, b, p)| Op::RegisterSchedule(k, t, st, m, b, p)),
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
            (any::<bool>(), 1i64..1_000_000, 1u32..24)
                .prop_map(|(k, t, m)| Op::GoodSchedule(k, t, m)),
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
                    purchase_date: day(0),
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
            Op::RegisterSchedule(k, t, st, m, b, pl) => store
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
                    purchase_date: day(0),
                    cost: usd(c),
                    salvage_value: usd(0),
                    useful_life_months: m,
                    cost_account: 2,
                    accum_depr_account: 3,
                    expense_account: 9,
                })
                .map(drop),
            Op::GoodSchedule(k, t, m) => store
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
            // The converse law: an accepted command is on the record. Only
            // a batch run may find nothing to do.
            if result.is_ok() && !matches!(op, Op::RunDepreciation(..) | Op::RunRecurring(..)) {
                let after = std::fs::read_to_string(&journal).unwrap();
                prop_assert!(
                    after.len() > before.len(),
                    "accepted {:?} but journaled nothing",
                    op
                );
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
            purchase_date: day(1, 1),
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
/// and replays under it: "cumulative" for new closes, "largest_remainder"
/// for the per-close rule of earlier on 2026-10-13. Three one-cent months,
/// 50/50: cumulative ends A 0.02 / B 0.01, per close A 0.03 / B 0.00.
#[test]
fn members_closes_replay_under_the_rule_they_were_journaled_with() {
    for (cumulative, rule, a, b) in [
        (true, "\"rounding\":\"cumulative\"", "0.02", "0.01"),
        (false, "\"rounding\":\"largest_remainder\"", "0.03", "0.00"),
    ] {
        let dir = test_dir(&format!("close-rule-{cumulative}"));
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
        let members: Vec<ledger_core::MemberClose> = caps
            .iter()
            .map(|&capital| ledger_core::MemberClose {
                capital,
                share: Decimal::new(5, 1),
                distributions: None,
            })
            .collect();
        let to = if cumulative {
            ledger_core::CloseTo::Members(members)
        } else {
            ledger_core::CloseTo::MembersPerClose(members)
        };
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
            store.close_period_to(p, &to).unwrap();
        }
        let journal = std::fs::read_to_string(dir.join(ledger_store::JOURNAL_FILE)).unwrap();
        assert_eq!(journal.matches(rule).count(), 3, "{rule}");
        drop(store);
        let store = Store::open(&dir).unwrap();
        store.verify().unwrap();
        let bal = |id| store.ledger().account_balance(id).unwrap();
        assert_eq!(bal(caps[0]), a.parse::<Decimal>().unwrap(), "{rule}");
        assert_eq!(bal(caps[1]), b.parse::<Decimal>().unwrap(), "{rule}");
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
            purchase_date: day(1, 1),
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
    let before_write = std::fs::read(dir.join(ledger_store::DB_FILE)).unwrap();
    store.add_vendor("Supplier").unwrap();
    drop(store);
    std::fs::write(dir.join(ledger_store::DB_FILE), &before_write).unwrap();
    let _ = std::fs::remove_file(dir.join("wanderbooks.db-wal"));
    let _ = std::fs::remove_file(dir.join("wanderbooks.db-shm"));
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
    let conn = rusqlite::Connection::open(dir.join(ledger_store::DB_FILE)).unwrap();
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
    let conn = rusqlite::Connection::open(dir.join(ledger_store::DB_FILE)).unwrap();
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
        let conn = rusqlite::Connection::open(dir.join(ledger_store::DB_FILE)).unwrap();
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
    let _ = std::fs::remove_file(dir.join(ledger_store::DB_FILE));
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
    let conn = rusqlite::Connection::open(dir.join(ledger_store::DB_FILE)).unwrap();
    conn.execute("DROP TABLE events", []).unwrap();
    drop(conn);
    let store = Store::open(&dir);
    let msg = match store {
        Ok(s) => s.verify().unwrap_err().to_string(),
        Err(e) => e.to_string(),
    };
    assert!(msg.contains("events"), "{msg}");
}
