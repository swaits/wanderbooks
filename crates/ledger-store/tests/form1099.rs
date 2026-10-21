//! 1099 decisions and payment methods in the journal (plan P3, defects
//! 12, 18 and 23; D-40): written only in books that declare their
//! features (this version's from creation, an older version's after the
//! format upgrade), replayed and projected exactly, refused without a
//! trace, and books written before them read as they always meant. Every
//! name is fictional.
//!
//! ```text
//! GOAL: the decision ("no 1099" included) and how each payment was made
//!       are kept as records, and no version ever misreads them.
//! SOURCE: rules.md B-19, B-27; ledger_store::form1099 INVARIANTS 1-5.
//! ```

use chrono::NaiveDate;
use ledger_core::form1099::Applied;
use ledger_core::settings::Source;
use ledger_core::{
    AccountType, Currency, Dated1099, Decision1099, ExemptReason, Form1099, InstrumentKind, Money,
    Side,
};
use ledger_store::format::{FORMAT_SET, PAYMENT_INSTRUMENTS, VENDOR_1099_DECISIONS, current};
use ledger_store::journal::read_and_verify_journal;
use ledger_store::{
    JOURNAL_FILE, JournalFormat, JournalWriter, Snapshot, Store, StoreError, db_file,
};
use rust_decimal::Decimal;
use serde_json::json;
use std::path::{Path, PathBuf};

/// A test's dir, removed when the test ends, pass or panic.
struct TestDir(PathBuf);

impl std::ops::Deref for TestDir {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn test_dir(name: &str) -> TestDir {
    let dir = std::env::temp_dir().join(format!("wanderbooks-1099-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    TestDir(dir)
}

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn src() -> Source {
    Source {
        decided_by: "Riley Chen".into(),
        decided_on: d(2026, 10, 16),
        citation: Some("Treas. Reg. §1.6041-3(p)(1)".into()),
        note: None,
    }
}

fn from_beginning(decision: Decision1099) -> Dated1099 {
    Dated1099::from_beginning(decision, Some(src()))
}

/// Books: bank 1000, Due to Members 2100, contract labor 6000; Harbor
/// Design LLC paid 2,500.00 by a member (Dr 6000 / Cr 2100) and reversed,
/// then paid again.
struct Books {
    store: Store,
    due: u64,
    harbor: u64,
    paid: u64,
    reversal: u64,
}

fn books(dir: &Path) -> Books {
    books_in(dir, &current())
}

/// [`books`] in the journal format `format`: format 1
/// ([`JournalFormat::default`]) is books an older version wrote.
fn books_in(dir: &Path, format: &JournalFormat) -> Books {
    let mut store = Store::create_in_format(dir, Currency::USD, format).unwrap();
    store
        .add_account("1000", "Bank", AccountType::Asset, None, false)
        .unwrap();
    let due = store
        .add_account(
            "2100",
            "Due to Members",
            AccountType::Liability,
            None,
            false,
        )
        .unwrap();
    let labor = store
        .add_account("6000", "Contract Labor", AccountType::Expense, None, false)
        .unwrap();
    let harbor = store.add_vendor("Harbor Design LLC").unwrap();
    let pay = |store: &mut Store, day| {
        let e = store.create_entry(d(2026, 5, day), "Design work").unwrap();
        let m = Money::new(Decimal::new(250_000, 2), Currency::USD);
        store.add_line(e, labor, Side::Debit, m).unwrap();
        store.add_line(e, due, Side::Credit, m).unwrap();
        store.post_entry(e).unwrap();
        store.set_entry_vendor(e, Some(harbor)).unwrap();
        e
    };
    let paid = pay(&mut store, 1);
    let reversal = store.reverse_entry(paid, d(2026, 5, 2), "twice").unwrap();
    pay(&mut store, 3);
    Books {
        store,
        due,
        harbor,
        paid,
        reversal,
    }
}

/// A worksheet line: name, paid, undetermined, excluded card, status,
/// decision needed.
type Row1099 = (String, Decimal, Decimal, Decimal, &'static str, bool);

/// A row of the projection's vendor_1099_decisions: effective, form,
/// reason kind, source.
type DecisionRow = (Option<String>, Option<String>, Option<String>, String);

fn types(dir: &Path) -> Vec<(String, serde_json::Value)> {
    read_and_verify_journal(&dir.join(JOURNAL_FILE))
        .unwrap()
        .into_iter()
        .map(|l| (l.line_type, l.payload))
        .collect()
}

/// B-27: a line that needs a feature is written only in books that
/// declare it, and a command never declares one on the way. Books an
/// older version wrote (format 1) refuse a decision and a payment method
/// while they wait for their format upgrade, journaling nothing; the
/// upgrade is one declaration of every feature; after it, as in books this
/// version made (which declared them when created), each command journals
/// its own lines and nothing else. A version that doesn't know a feature
/// stops at the declaration (`tests/format.rs`
/// old_reader_gets_newer_journal, for any feature it doesn't know).
#[test]
fn each_feature_waits_for_the_upgrade_and_is_never_declared_on_the_way() {
    let dir = test_dir("declare");
    let mut b = books_in(&dir, &JournalFormat::default());
    let was = std::fs::read(dir.join(JOURNAL_FILE)).unwrap();
    assert!(
        !types(&dir).iter().any(|(t, _)| t == FORMAT_SET),
        "books an older version wrote declare nothing"
    );
    let nec = from_beginning(Decision1099::Files(Form1099::Nec));
    fn waits<T: std::fmt::Debug>(r: Result<T, StoreError>, feature: &str) {
        assert!(
            matches!(&r, Err(StoreError::FormatUpgradePending { need }) if need.contains(feature)),
            "{feature}: {r:?}"
        );
    }
    waits(
        b.store.set_vendor_1099(b.harbor, &nec, None),
        VENDOR_1099_DECISIONS,
    );
    waits(
        b.store.set_vendor_w9_flag(b.harbor, true),
        VENDOR_1099_DECISIONS,
    );
    waits(
        b.store.add_instrument(
            InstrumentKind::MemberCard,
            "Riley's Visa",
            Some("4242"),
            None,
            b.due,
        ),
        PAYMENT_INSTRUMENTS,
    );
    assert_eq!(
        std::fs::read(dir.join(JOURNAL_FILE)).unwrap(),
        was,
        "nothing journaled"
    );
    let before = types(&dir).len();
    assert!(b.store.upgrade_format().unwrap().is_some());
    b.store.set_vendor_1099(b.harbor, &nec, None).unwrap();
    b.store.set_vendor_w9_flag(b.harbor, true).unwrap();
    let card = b
        .store
        .add_instrument(
            InstrumentKind::MemberCard,
            "Riley's Visa",
            Some("4242"),
            None,
            b.due,
        )
        .unwrap();
    b.store.set_entry_instrument(b.paid, Some(card)).unwrap();
    b.store
        .add_instrument(
            InstrumentKind::MemberCashOrCheck,
            "Riley's checks",
            None,
            None,
            b.due,
        )
        .unwrap();
    let j = types(&dir);
    let added: Vec<&str> = j[before..].iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(
        added,
        [
            FORMAT_SET,
            "vendor.1099_set",
            "vendor.w9_flag_set",
            "instrument.added",
            "entry.instrument_set",
            "instrument.added",
        ]
    );
    let declared = &j[before].1["features"];
    for feature in [VENDOR_1099_DECISIONS, PAYMENT_INSTRUMENTS] {
        assert!(
            declared.as_array().unwrap().contains(&json!(feature)),
            "the upgrade declares {feature}"
        );
    }
    assert_eq!(b.store.journal_format(), &current());
    b.store.verify().unwrap();

    // Books this version made: declared at creation, so the same commands
    // journal only their own lines.
    let dir = test_dir("declare-new");
    let mut b = books(&dir);
    let before = types(&dir).len();
    b.store.set_vendor_1099(b.harbor, &nec, None).unwrap();
    b.store
        .add_instrument(
            InstrumentKind::MemberCashOrCheck,
            "Riley's checks",
            None,
            None,
            b.due,
        )
        .unwrap();
    let j = types(&dir);
    let added: Vec<&str> = j[before..].iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(added, ["vendor.1099_set", "instrument.added"]);
    assert_eq!(j.iter().filter(|(t, _)| t == FORMAT_SET).count(), 1);
}

/// Every record replays and projects exactly: reopening gives back the
/// same ledger, part for part; a projection rebuilt from the journal
/// equals the one the writer kept, table by table (verify); the viewer's
/// snapshot reads the same; and the projection holds each decision with
/// who made it, each method, and the tag on the whole reversal chain.
#[test]
fn decisions_and_methods_replay_and_project() {
    let dir = test_dir("replay");
    let mut b = books(&dir);
    let corp = Decision1099::not_reportable(
        ExemptReason::Corporation,
        "Delaware public benefit corporation, from the public record",
    )
    .unwrap();
    b.store
        .set_vendor_1099(
            b.harbor,
            &from_beginning(Decision1099::Files(Form1099::Nec)),
            Some(true),
        )
        .unwrap();
    b.store
        .set_vendor_1099(
            b.harbor,
            &Dated1099 {
                effective: Some(d(2026, 12, 30)),
                decision: corp.clone(),
                source: Some(src()),
            },
            None,
        )
        .unwrap();
    let card = b
        .store
        .add_instrument(
            InstrumentKind::MemberCard,
            "Riley's Visa",
            Some("4242"),
            None,
            b.due,
        )
        .unwrap();
    assert!(
        b.store
            .set_entry_instrument(b.reversal, Some(card))
            .unwrap()
    );
    let chain = [b.paid as i64, b.reversal as i64];
    let live = b.store.ledger().clone();
    let v = live.get_vendor(b.harbor).unwrap();
    assert!(v.tin_on_file);
    assert_eq!(v.decisions_1099.len(), 2);
    assert_eq!(v.decision_on(d(2026, 12, 31)), Some(&corp));
    assert_eq!(live.entry_instrument(b.paid), Some(card));
    drop(b);
    let store = Store::open(&dir).unwrap();
    assert!(live.differs_from(store.ledger()).is_empty());
    store.verify().unwrap();
    drop(store);
    std::fs::remove_file(dir.join(db_file())).unwrap();
    let store = Store::open(&dir).unwrap();
    store.verify().unwrap();
    assert!(live.differs_from(store.ledger()).is_empty());
    drop(store);
    let snap = Snapshot::load(&dir).unwrap();
    assert!(live.differs_from(snap.ledger()).is_empty());
    let conn = rusqlite::Connection::open(dir.join(db_file())).unwrap();
    let decisions: Vec<DecisionRow> = conn
        .prepare("SELECT effective, form_1099, reason_kind, source_json FROM vendor_1099_decisions ORDER BY seq")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(decisions.len(), 2);
    assert_eq!(
        (decisions[0].0.as_deref(), decisions[0].1.as_deref()),
        (None, Some("nec"))
    );
    assert_eq!(
        (
            decisions[1].0.as_deref(),
            decisions[1].1.as_deref(),
            decisions[1].2.as_deref()
        ),
        (
            Some("2026-12-30"),
            Some("not_reportable"),
            Some("corporation")
        )
    );
    assert!(decisions[1].3.contains("Riley Chen"));
    let (form, on_file): (String, bool) = conn
        .query_row("SELECT form_1099, tin_on_file FROM vendors", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!(
        (form.as_str(), on_file),
        ("nec", true),
        "the decision from the beginning"
    );
    let tagged: Vec<i64> = conn
        .prepare("SELECT entry_id FROM entry_instruments ORDER BY entry_id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(tagged, chain);
}

/// The engine-side half of verification item 13a (P3 review R14): a 1099
/// decision, a W-9 flag, a payment method and a payment-method tag change
/// no entry, no balance and no period.
#[test]
fn decision_and_method_records_change_no_entry() {
    let dir = test_dir("no-entry");
    let mut b = books(&dir);
    let figures = |s: &Store| {
        let l = s.ledger();
        (
            ledger_core::reports::snapshot(l).unwrap(),
            l.posted_entries().cloned().collect::<Vec<_>>(),
            l.list_periods().into_iter().cloned().collect::<Vec<_>>(),
        )
    };
    let start = figures(&b.store);
    b.store
        .set_vendor_1099(
            b.harbor,
            &from_beginning(Decision1099::Files(Form1099::Misc)),
            Some(false),
        )
        .unwrap();
    assert_eq!(figures(&b.store), start);
    b.store.set_vendor_w9_flag(b.harbor, true).unwrap();
    assert_eq!(figures(&b.store), start);
    let i = b
        .store
        .add_instrument(InstrumentKind::Platform, "Harbor Pay", None, None, b.due)
        .unwrap();
    assert_eq!(figures(&b.store), start);
    b.store.set_entry_instrument(b.paid, Some(i)).unwrap();
    assert_eq!(figures(&b.store), start);
    b.store.set_entry_instrument(b.paid, None).unwrap();
    assert_eq!(figures(&b.store), start);
}

/// A refused command journals nothing, not even a feature declaration;
/// one that changes nothing journals nothing either (P3 review R8).
#[test]
fn refusals_and_no_ops_journal_nothing() {
    let dir = test_dir("refused");
    let mut b = books(&dir);
    let journal = || std::fs::read(dir.join(JOURNAL_FILE)).unwrap();
    let at_start = journal();
    let nec = Decision1099::Files(Form1099::Nec);
    // No source.
    let err = b
        .store
        .set_vendor_1099(
            b.harbor,
            &Dated1099::from_beginning(nec.clone(), None),
            None,
        )
        .unwrap_err();
    assert!(err.to_string().contains("decided_by"), "{err}");
    // Dated with nothing decided before it.
    let dated = Dated1099 {
        effective: Some(d(2026, 7, 1)),
        decision: nec.clone(),
        source: Some(src()),
    };
    assert!(b.store.set_vendor_1099(b.harbor, &dated, None).is_err());
    // A blank reason; an unknown vendor.
    let blank = Dated1099::from_beginning(
        Decision1099::NotReportable {
            kind: ExemptReason::Other,
            reason: " ".into(),
        },
        Some(src()),
    );
    assert!(b.store.set_vendor_1099(b.harbor, &blank, None).is_err());
    assert!(
        b.store
            .set_vendor_1099(99, &from_beginning(nec.clone()), None)
            .is_err()
    );
    assert!(b.store.set_vendor_w9_flag(99, true).is_err());
    // A bad method, an unknown entry or method.
    assert!(
        b.store
            .add_instrument(InstrumentKind::Bank, "Bank", Some("12345"), None, b.due)
            .is_err()
    );
    assert!(b.store.set_entry_instrument(999, None).is_err());
    assert!(b.store.set_entry_instrument(b.paid, Some(7)).is_err());
    assert_eq!(journal(), at_start, "refusals left a trace");
    // A flag already so, a tag already so: nothing.
    assert!(!b.store.set_vendor_w9_flag(b.harbor, false).unwrap());
    assert!(!b.store.set_entry_instrument(b.paid, None).unwrap());
    assert_eq!(journal(), at_start);
    // Recorded once; the same again: unchanged, nothing more.
    assert_eq!(
        b.store
            .set_vendor_1099(b.harbor, &from_beginning(nec.clone()), None)
            .unwrap(),
        Applied::Set
    );
    let once = journal();
    assert_eq!(
        b.store
            .set_vendor_1099(b.harbor, &from_beginning(nec.clone()), None)
            .unwrap(),
        Applied::Unchanged
    );
    assert_eq!(journal(), once);
    // The same decision with a W-9 flag: only the flag is journaled.
    assert_eq!(
        b.store
            .set_vendor_1099(b.harbor, &from_beginning(nec), Some(true))
            .unwrap(),
        Applied::Unchanged
    );
    let last = types(&dir).pop().unwrap();
    assert_eq!(last.0, "vendor.w9_flag_set");
}

/// P3 review R8: marking a W-9 on file keeps the 1099 decision and who
/// made it, verbatim.
#[test]
fn marking_a_w9_keeps_who_decided_the_1099() {
    let dir = test_dir("w9");
    let mut b = books(&dir);
    let decision = from_beginning(Decision1099::Files(Form1099::Nec));
    b.store
        .set_vendor_1099(b.harbor, &decision, Some(false))
        .unwrap();
    assert!(b.store.set_vendor_w9_flag(b.harbor, true).unwrap());
    let v = b.store.ledger().get_vendor(b.harbor).unwrap();
    assert!(v.tin_on_file);
    assert_eq!(v.decisions_1099, [decision]);
}

/// B-27: a value only a newer version writes, in a line of a type this
/// version knows (a 1099 decision, a reason kind, a payment method's
/// kind), is newer, never corrupt; a malformed one is corrupt.
#[test]
fn an_unknown_1099_value_is_newer_not_corrupt() {
    let source = json!({"decided_by": "Riley Chen", "decided_on": "2026-10-16"});
    let cases = [
        (
            "vendor.1099_set",
            json!({"vendor_id": 1, "form_1099": "1099-k", "tin_on_file": false, "source": source}),
            Some("the 1099 decision \"1099-k\""),
        ),
        (
            "vendor.1099_set",
            json!({"vendor_id": 1, "form_1099": "not_reportable", "reason_kind": "charity_lottery", "reason": "x", "tin_on_file": false, "source": source}),
            Some("the 1099 reason kind \"charity_lottery\""),
        ),
        (
            "instrument.added",
            json!({"instrument_id": 1, "kind": "crypto_wallet", "label": "x", "account_id": 1}),
            Some("the payment method kind \"crypto_wallet\""),
        ),
        (
            "vendor.1099_set",
            json!({"vendor_id": 1, "form_1099": "not_reportable", "reason_kind": "corporation", "tin_on_file": false, "source": source}),
            None,
        ),
    ];
    for (i, (t, payload, need)) in cases.into_iter().enumerate() {
        let dir = test_dir(&format!("newer-{i}"));
        let b = books(&dir);
        drop(b);
        let mut w = JournalWriter::open(&dir.join(JOURNAL_FILE)).unwrap();
        w.append(
            FORMAT_SET,
            json!({"format": 2, "features": [PAYMENT_INSTRUMENTS, VENDOR_1099_DECISIONS]}),
        )
        .unwrap();
        let seq = w.append(t, payload).unwrap().seq;
        drop(w);
        std::fs::remove_file(dir.join(db_file())).unwrap();
        match (Store::open(&dir), need) {
            (
                Err(StoreError::NewerJournal {
                    seq: s, need: n, ..
                }),
                Some(want),
            ) => {
                assert_eq!((s, n.as_str()), (seq, want));
            }
            (Err(StoreError::JournalCorrupt(_)), None) => {}
            (got, want) => panic!("case {i}: {:?} for {want:?}", got.err()),
        }
    }
}

/// Plan 5.5, D-40: books written before decisions were kept (lines with a
/// form or null and a W-9 flag, no source, no date) open unchanged, byte
/// for byte, with no feature declared: each old line reads as a decision
/// from the beginning (a null as undetermined, never "not reportable"),
/// and the projection is the one those books always had.
#[test]
fn books_written_before_decisions_read_as_they_always_meant() {
    let dir = test_dir("legacy");
    let b = books_in(&dir, &JournalFormat::default());
    let (harbor, paid) = (b.harbor, b.paid);
    drop(b);
    let mut w = JournalWriter::open(&dir.join(JOURNAL_FILE)).unwrap();
    for (vendor, name) in [(2u64, "Pinecrest Supply"), (3, "Ledgerline Bookkeeping")] {
        w.append("vendor.added", json!({"id": vendor, "name": name}))
            .unwrap();
    }
    for p in [
        json!({"vendor_id": harbor, "form_1099": "nec", "tin_on_file": false}),
        json!({"vendor_id": 2, "form_1099": null, "tin_on_file": false}),
        // The old W-9 sync on an undecided vendor.
        json!({"vendor_id": 3, "form_1099": null, "tin_on_file": true}),
    ] {
        w.append("vendor.1099_set", p).unwrap();
    }
    drop(w);
    std::fs::remove_file(dir.join(db_file())).unwrap();
    let journal = std::fs::read(dir.join(JOURNAL_FILE)).unwrap();
    let store = Store::open(&dir).unwrap();
    store.verify().unwrap();
    let l = store.ledger();
    let h = l.get_vendor(harbor).unwrap();
    assert_eq!(
        h.decisions_1099,
        [Dated1099::from_beginning(
            Decision1099::Files(Form1099::Nec),
            None
        )]
    );
    for v in [2, 3] {
        assert_eq!(
            l.get_vendor(v).unwrap().decisions_1099,
            [Dated1099::from_beginning(Decision1099::Undetermined, None)]
        );
    }
    assert!(l.get_vendor(3).unwrap().tin_on_file);
    assert_eq!(l.entry_instrument(paid), None);
    assert_eq!(store.journal_format().features, Vec::<String>::new());
    drop(store);
    assert_eq!(std::fs::read(dir.join(JOURNAL_FILE)).unwrap(), journal);
    let conn = rusqlite::Connection::open(dir.join(db_file())).unwrap();
    let forms: Vec<(i64, Option<String>, bool)> = conn
        .prepare("SELECT id, form_1099, tin_on_file FROM vendors ORDER BY id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        forms,
        [
            (1, Some("nec".to_string()), false),
            (2, None, false),
            (3, None, true)
        ]
    );
    let n: i64 = conn
        .query_row("SELECT COUNT(*) FROM vendor_1099_decisions", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(n, 0, "nothing an old line didn't say");
}

/// P3 brief 8.2, compatibility: tests/fixtures/old-books/p2_vendor_1099,
/// written through the Store API of the version before P3 (a vendor
/// marked 1099-NEC, two marked "none" and "", an undecided one whose W-9
/// sync wrote a null; payments out of the bank 1000, the members' 2100 and
/// a card 2050), opens to its record: its journal byte for byte, no
/// feature declared, every null undetermined and "nec" a 1099-NEC from
/// the beginning that nobody is named for; and the 1099 figures, by hand,
/// for cash 1000 and financing 2100:
/// - 2026, threshold 2,000.00: Harbor Design LLC paid 2,500.00 from the
///   bank (counts), 700.00 by a member (undetermined) and 400.00 on the
///   card (no payment method names 2050: no payment line): reportable,
///   2,500.00 counting whatever the 700.00 turns out. Bigco Supply,
///   undecided, 900.00 from the bank: under the threshold, a decision
///   needed. Cedar Studio and Dana Studio: nothing in 2026, not listed.
/// - 2025, threshold 600.00: Dana Studio, undecided, 650.00 by a member
///   with no method: undetermined (it could reach 600.00), W-9 on file;
///   Harbor Design LLC listed (decided for a 1099), nothing paid.
#[test]
fn books_written_under_p2_open_to_their_record() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/old-books/p2_vendor_1099/journal.jsonl");
    let dir = test_dir("p2-fixture");
    std::fs::copy(&fixture, dir.join(JOURNAL_FILE)).unwrap();
    let journal = std::fs::read(dir.join(JOURNAL_FILE)).unwrap();
    let store = Store::open(&dir).unwrap();
    store.verify().unwrap();
    assert!(store.journal_format().features.is_empty());
    let l = store.ledger();
    let by_name = |n: &str| l.list_vendors().into_iter().find(|v| v.name == n).unwrap();
    assert_eq!(
        by_name("Harbor Design LLC").decisions_1099,
        [Dated1099::from_beginning(
            Decision1099::Files(Form1099::Nec),
            None
        )]
    );
    for n in ["Bigco Supply", "Cedar Studio", "Dana Studio"] {
        assert_eq!(
            by_name(n).decisions_1099,
            [Dated1099::from_beginning(Decision1099::Undetermined, None)],
            "{n}"
        );
    }
    assert!(by_name("Dana Studio").tin_on_file);
    let code = |c: &str| l.chart().find_by_code(c).unwrap().id;
    let report = |year| {
        ledger_core::vendor_1099_report(
            l,
            year,
            None,
            &ledger_core::PaymentAccounts {
                cash: vec![code("1000")],
                financing: vec![code("2100")],
            },
            &[],
            None,
        )
        .unwrap()
    };
    let rows = |r: &ledger_core::Vendor1099Report| -> Vec<Row1099> {
        r.vendors
            .iter()
            .map(|v| {
                (
                    v.name.clone(),
                    v.paid,
                    v.undetermined,
                    v.excluded_card,
                    v.status.key(),
                    v.decision_needed,
                )
            })
            .collect()
    };
    let c = |n: i64| Decimal::new(n, 2);
    assert_eq!(
        rows(&report(2026)),
        [
            (
                "Bigco Supply".into(),
                c(90_000),
                c(0),
                c(0),
                "under_threshold",
                true
            ),
            (
                "Harbor Design LLC".into(),
                c(250_000),
                c(70_000),
                c(0),
                "reportable",
                false
            ),
        ]
    );
    assert_eq!(
        rows(&report(2025)),
        [
            (
                "Dana Studio".into(),
                c(0),
                c(65_000),
                c(0),
                "undetermined",
                true
            ),
            // Decided for a 1099 from the beginning: listed, nothing paid.
            (
                "Harbor Design LLC".into(),
                c(0),
                c(0),
                c(0),
                "under_threshold",
                false
            ),
        ]
    );
    drop(store);
    assert_eq!(std::fs::read(dir.join(JOURNAL_FILE)).unwrap(), journal);
}
