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
