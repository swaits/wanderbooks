//! Documents read their figures from the ledger lines tagged with them
//! (plan P4b S1, D-45; rules.md B-38): the books an earlier version wrote,
//! replayed, and the golden figures that version showed for them.
//!
//! The fixture `tests/fixtures/p4-arap-before` is a books dir written by
//! the version before S1 (main at 0df7f52a), and `figures.json` beside it
//! is what that version reported for it. This version opens the same
//! journal; every figure that moves is adjudicated here, cent by cent
//! (P4b.14 item 2): only documents whose entry or payment was reversed
//! change, and the general ledger doesn't move at all.

use chrono::NaiveDate;
use ledger_core::LedgerError;
use ledger_core::arap;
use ledger_core::arap::{DocumentLine, SendKind, Sent, SentVia};
use ledger_core::{AccountType, Currency, DocRef, Money, Side};
use ledger_store::{JOURNAL_FILE, Store, StoreError, Subject, WriteOffEvidence};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn cents(c: i64) -> Decimal {
    Decimal::new(c, 2)
}

fn line(what: &str, c: i64) -> DocumentLine {
    DocumentLine {
        description: what.into(),
        amount: cents(c),
        class_id: None,
        account: None,
        quantity: None,
        unit_price: None,
    }
}

/// The dates the golden figures are taken at: between a payment and its
/// reversal, a month end, and the end of the books.
const GOLDEN_DATES: [(i32, u32, u32); 3] = [(2026, 1, 22), (2026, 1, 31), (2026, 3, 31)];

/// Write the fixture's books into `dir`, through the Store's commands as
/// the version before S1 had them: two customers' invoices (one paid then
/// the payment reversed, one reversed outright, one part paid), a vendor's
/// bills (one paid then the payment reversed), and a hand entry on
/// receivables. Only a version before S1 can (S1 refuses the hand entry).
fn write_books(dir: &Path) {
    let mut s = Store::create(dir, Currency::USD).unwrap();
    let bank = s
        .add_account("1000", "Bank", AccountType::Asset, None, false)
        .unwrap();
    let capital = s
        .add_account("3000", "Capital", AccountType::Equity, None, false)
        .unwrap();
    let seed = s.create_entry(d(2026, 1, 1), "Opening capital").unwrap();
    let m = Money::new(Decimal::from(1000), Currency::USD);
    s.add_line(seed, bank, Side::Debit, m).unwrap();
    s.add_line(seed, capital, Side::Credit, m).unwrap();
    s.post_entry(seed).unwrap();
    let fernhill = s.add_customer("Fernhill Labs").unwrap();
    let morgan = s.add_customer("Morgan Ellis Studio").unwrap();
    let vendor = s.add_vendor("Tobin Harlow Supply").unwrap();
    // Invoice 1: paid in full, then the payment reversed (a bounced check).
    let i1 = s
        .create_invoice(
            fernhill,
            d(2026, 1, 10),
            d(2026, 2, 9),
            vec![line("Design", 10_000)],
        )
        .unwrap();
    let p1 = s
        .receive_payment(i1, d(2026, 1, 20), cents(10_000), bank)
        .unwrap();
    s.reverse_entry(p1, d(2026, 1, 25), "Check bounced")
        .unwrap();
    // Invoice 2: issued to the wrong customer and reversed (a void).
    let i2 = s
        .create_invoice(
            morgan,
            d(2026, 2, 1),
            d(2026, 3, 3),
            vec![line("Wrong customer", 20_000)],
        )
        .unwrap();
    let e2 = s.ledger().get_invoice(i2).unwrap().entry_id;
    s.reverse_entry(e2, d(2026, 2, 5), "Void: wrong customer")
        .unwrap();
    // Invoice 3: part paid, nothing reversed.
    let i3 = s
        .create_invoice(
            morgan,
            d(2026, 3, 1),
            d(2026, 3, 31),
            vec![line("Workshop", 30_000)],
        )
        .unwrap();
    s.receive_payment(i3, d(2026, 3, 10), cents(12_000), bank)
        .unwrap();
    // Bill 1: paid, then the payment reversed (sent to the wrong vendor).
    let b1 = s
        .create_bill(
            vendor,
            d(2026, 1, 15),
            d(2026, 2, 14),
            vec![line("Paper", 8_000)],
        )
        .unwrap();
    let q1 = s.pay_bill(b1, d(2026, 1, 20), cents(8_000), bank).unwrap();
    s.reverse_entry(q1, d(2026, 1, 28), "Paid in error")
        .unwrap();
    // Bill 2: open.
    s.create_bill(
        vendor,
        d(2026, 2, 10),
        d(2026, 3, 12),
        vec![line("Ink", 4_000)],
    )
    .unwrap();
    // A hand entry on receivables, no document behind it.
    let ar = s.ledger().chart().find_by_code("1200").unwrap().id;
    let revenue = s.ledger().chart().find_by_code("4000").unwrap().id;
    let hand = s.create_entry(d(2026, 2, 15), "Hand-booked sale").unwrap();
    let m = Money::new(cents(2_500), Currency::USD);
    s.add_line(hand, ar, Side::Debit, m).unwrap();
    s.add_line(hand, revenue, Side::Credit, m).unwrap();
    s.post_entry(hand).unwrap();
}

/// An aging report as JSON: each row's name and buckets, as strings.
fn aging_json(rows: &[ledger_core::AgingRow]) -> Value {
    Value::Array(
        rows.iter()
            .map(|r| {
                json!({
                    "name": r.name,
                    "current": r.current.normalize().to_string(),
                    "days_30": r.days_30.normalize().to_string(),
                    "days_60": r.days_60.normalize().to_string(),
                    "days_90": r.days_90.normalize().to_string(),
                    "days_90_plus": r.days_90_plus.normalize().to_string(),
                })
            })
            .collect(),
    )
}

/// The trial balance as JSON: each account's code and signed balance.
fn trial_balance_json(s: &Store) -> Value {
    let tb = ledger_core::reports::trial_balance(s.ledger());
    Value::Array(
        tb.lines
            .iter()
            .map(|l| {
                json!({
                    "code": l.code,
                    "debit": l.debit.normalize().to_string(),
                    "credit": l.credit.normalize().to_string(),
                })
            })
            .collect(),
    )
}

/// How tests/fixtures/p4-arap-before was written, and its golden figures
/// taken: by the version before S1. Run with WB_WRITE_P4_ARAP_FIXTURE=DIR
/// on that version (S1 refuses the fixture's hand entry on receivables).
#[test]
fn write_the_p4_arap_fixture() {
    let Ok(out) = std::env::var("WB_WRITE_P4_ARAP_FIXTURE") else {
        return;
    };
    assert!(
        !ledger_store::format::KNOWN_FEATURES.contains(&"document_standing"),
        "only a version before S1 writes it"
    );
    let out = PathBuf::from(out);
    write_books(&out);
    let s = Store::open(&out).unwrap();
    let mut at = serde_json::Map::new();
    for (y, m, day) in GOLDEN_DATES {
        let on = d(y, m, day);
        at.insert(
            on.to_string(),
            json!({
                "ar_aging": aging_json(&s.ledger().ar_aging(on).unwrap()),
                "ap_aging": aging_json(&s.ledger().ap_aging(on).unwrap()),
            }),
        );
    }
    let invoices: Vec<Value> = s
        .ledger()
        .invoices()
        .iter()
        .map(|i| {
            json!({"id": i.id, "balance": i.balance().unwrap().normalize().to_string(),
                   "status": format!("{:?}", i.status)})
        })
        .collect();
    let bills: Vec<Value> = s
        .ledger()
        .bills()
        .iter()
        .map(|b| {
            json!({"id": b.id, "balance": b.balance().unwrap().normalize().to_string(),
                   "status": format!("{:?}", b.status)})
        })
        .collect();
    let figures = json!({
        "written_by": "wanderbooks main 0df7f52a, before P4b S1",
        "at": at,
        "invoices": invoices,
        "bills": bills,
        "trial_balance": trial_balance_json(&s),
    });
    drop(s);
    std::fs::write(
        out.join("figures.json"),
        serde_json::to_string_pretty(&figures).unwrap() + "\n",
    )
    .unwrap();
    for f in [ledger_store::LOCK_FILE, &ledger_store::db_file()] {
        let _ = std::fs::remove_file(out.join(f));
    }
    let _ = std::fs::remove_dir(out.join("blobs"));
    assert!(out.join(JOURNAL_FILE).exists());
}

/// A test's dir, removed when the test ends, pass or panic.
struct TestDir(PathBuf);

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn test_dir(name: &str) -> TestDir {
    let dir = std::env::temp_dir().join(format!(
        "wanderbooks-arap-standing-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    TestDir(dir)
}

/// A copy of the fixture's journal (its projection is built on open).
fn fixture(name: &str) -> TestDir {
    let dir = test_dir(name);
    let from = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/p4-arap-before");
    std::fs::copy(from.join(JOURNAL_FILE), dir.0.join(JOURNAL_FILE)).unwrap();
    dir
}

fn golden() -> Value {
    serde_json::from_str(include_str!("fixtures/p4-arap-before/figures.json")).unwrap()
}

fn journal_len(dir: &Path) -> usize {
    std::fs::read_to_string(dir.join(JOURNAL_FILE))
        .unwrap()
        .lines()
        .count()
}

/// The lines journaled from `from` on, as (type, payload).
fn since(dir: &Path, from: usize) -> Vec<(String, Value)> {
    std::fs::read_to_string(dir.join(JOURNAL_FILE))
        .unwrap()
        .lines()
        .skip(from)
        .map(|l| {
            let v: Value = serde_json::from_str(l).unwrap();
            (
                v["type"].as_str().unwrap().to_string(),
                v["payload"].clone(),
            )
        })
        .collect()
}

/// The fixture an earlier version wrote opens, verifies, and shows each
/// figure that version got wrong, right (P4b.14 item 2, adjudicated cent
/// by cent against figures.json):
///
/// - The general ledger is the golden's, line for line: the GL already
///   held every reversal.
/// - Jan 22, between the payments and their reversals: as the golden
///   (both documents paid then).
/// - Jan 31: the golden showed nothing owed; Fernhill's invoice 1 owes
///   100.00 again (its payment reversed Jan 25) and Tobin's bill 1 80.00
///   (its payment reversed Jan 28), both current.
/// - Mar 31: the golden aged Morgan's voided invoice 2 (200.00, 1-30)
///   and left Fernhill out; now Morgan owes only invoice 3's 180.00
///   (current), Fernhill 100.00 at 50 days past due (31-60), and Tobin
///   80.00 at 45 days (31-60) beside bill 2's 40.00 (1-30).
/// - Receivables 305.00 = the invoices' 280.00 + the hand line's 25.00,
///   unattached.
#[test]
fn legacy_reversal_now_counts() {
    let dir = fixture("golden");
    let s = Store::open(&dir.0).unwrap();
    s.verify().unwrap();
    let g = golden();
    assert_eq!(
        trial_balance_json(&s),
        g["trial_balance"],
        "the GL doesn't move"
    );
    let on = |y, m, day| d(y, m, day).to_string();
    let at = |date: String| g["at"][&date].clone();
    let now = |date: NaiveDate| {
        json!({
            "ar_aging": aging_json(&s.ledger().ar_aging(date).unwrap()),
            "ap_aging": aging_json(&s.ledger().ap_aging(date).unwrap()),
        })
    };
    assert_eq!(now(d(2026, 1, 22)), at(on(2026, 1, 22)), "unchanged");
    let row = |name: &str, current: &str, d30: &str, d60: &str| {
        json!({"name": name, "current": current, "days_30": d30, "days_60": d60,
               "days_90": "0", "days_90_plus": "0"})
    };
    assert_eq!(at(on(2026, 1, 31)), json!({"ar_aging": [], "ap_aging": []}));
    assert_eq!(
        now(d(2026, 1, 31)),
        json!({
            "ar_aging": [row("Fernhill Labs", "100", "0", "0")],
            "ap_aging": [row("Tobin Harlow Supply", "80", "0", "0")],
        })
    );
    assert_eq!(
        at(on(2026, 3, 31))["ar_aging"],
        json!([row("Morgan Ellis Studio", "180", "200", "0")])
    );
    assert_eq!(
        now(d(2026, 3, 31)),
        json!({
            "ar_aging": [
                row("Fernhill Labs", "0", "0", "100"),
                row("Morgan Ellis Studio", "180", "0", "0"),
            ],
            "ap_aging": [row("Tobin Harlow Supply", "0", "40", "80")],
        })
    );
    let state = |doc| s.ledger().doc_state(doc, d(2026, 12, 31)).unwrap().unwrap();
    use ledger_core::{DocRef, DocStatus};
    for (doc, status, owed) in [
        (DocRef::Invoice(1), DocStatus::Open, 10_000),
        (DocRef::Invoice(2), DocStatus::Void, 0),
        (DocRef::Invoice(3), DocStatus::Partial, 18_000),
        (DocRef::Bill(1), DocStatus::Open, 8_000),
        (DocRef::Bill(2), DocStatus::Open, 4_000),
    ] {
        let st = state(doc);
        assert_eq!((st.status, st.outstanding), (status, cents(owed)), "{doc}");
    }
    let checks = s.ledger().control_check(d(2026, 12, 31)).unwrap();
    let ar = checks.iter().find(|c| c.role == "receivables").unwrap();
    assert_eq!(
        (ar.balance, ar.documents_total, ar.difference),
        (cents(30_500), cents(28_000), Decimal::ZERO)
    );
    assert_eq!(ar.unattached.len(), 1);
    assert_eq!(ar.unattached[0].amount, cents(2_500));
    let ap = checks.iter().find(|c| c.role == "payables").unwrap();
    assert_eq!(
        (ap.balance, ap.documents_total, ap.unattached.len()),
        (cents(12_000), cents(12_000), 0)
    );
    // Opening wrote nothing.
    drop(s);
    let again = Store::open(&dir.0).unwrap();
    assert_eq!(trial_balance_json(&again), g["trial_balance"]);
}

/// B-38 scenario 5 and item 7 in books an earlier version keeps: the
/// right payment after the wrong one was reversed waits for the journal
/// feature document_standing (an older version would count both),
/// refused and journaling nothing until the owner's upgrade; a payment
/// any version reads alike is taken at once; after the upgrade the
/// re-payment is taken, and the books replay to what was live.
#[test]
fn repaying_after_a_reversal_waits_for_document_standing() {
    use ledger_store::StoreError;
    let dir = fixture("standing");
    let mut s = Store::open(&dir.0).unwrap();
    assert!(
        !s.journal_format()
            .has_feature(ledger_store::format::DOCUMENT_STANDING)
    );
    let bank = s.ledger().chart().find_by_code("1000").unwrap().id;
    let head = s.head();
    match s.receive_payment(1, d(2026, 2, 1), cents(10_000), bank) {
        Err(StoreError::FormatUpgradePending { need }) => {
            assert_eq!(need, "the journal feature document_standing")
        }
        other => panic!("expected FormatUpgradePending, got {other:?}"),
    }
    assert!(matches!(
        s.pay_bill(1, d(2026, 2, 1), cents(8_000), bank),
        Err(StoreError::FormatUpgradePending { .. })
    ));
    assert_eq!(s.head(), head, "nothing journaled");
    // Invoice 3: nothing reversed on it, so any version reads it alike.
    s.receive_payment(3, d(2026, 3, 20), cents(18_000), bank)
        .unwrap();
    assert!(s.upgrade_format().unwrap().is_some());
    s.receive_payment(1, d(2026, 2, 1), cents(10_000), bank)
        .unwrap();
    s.pay_bill(1, d(2026, 2, 1), cents(8_000), bank).unwrap();
    s.verify().unwrap();
    let live = s.ledger().clone();
    drop(s);
    let reopened = Store::open(&dir.0).unwrap();
    assert!(
        live.differs_from(reopened.ledger()).is_empty(),
        "{:?}",
        live.differs_from(reopened.ledger())
    );
    let st = reopened
        .ledger()
        .invoice_state(1, d(2026, 12, 31))
        .unwrap()
        .unwrap();
    assert_eq!(st.status, ledger_core::DocStatus::Paid);
}

/// The control guard through the store, and a void under a payment:
/// refused, journaling nothing (B-38 scenarios 3 and 6).
#[test]
fn the_store_refuses_hand_lines_and_a_void_under_a_payment() {
    let dir = fixture("guard");
    let mut s = Store::open(&dir.0).unwrap();
    let ar = s.ledger().chart().find_by_code("1200").unwrap().id;
    let e = s.create_entry(d(2026, 3, 1), "by hand").unwrap();
    let head = s.head();
    let m = Money::new(cents(100), Currency::USD);
    assert!(matches!(
        s.add_line(e, ar, Side::Debit, m),
        Err(ledger_store::StoreError::Ledger(
            ledger_core::LedgerError::UntaggedControlLine { .. }
        ))
    ));
    assert!(
        s.void_document(ledger_core::DocRef::Invoice(3), d(2026, 3, 31), "dup")
            .is_err()
    );
    let inv3 = s.ledger().get_invoice(3).unwrap().entry_id;
    assert!(s.reverse_entry(inv3, d(2026, 3, 31), "Void").is_err());
    assert_eq!(s.head(), head, "nothing journaled");
    // The remedy: reverse the payment, then the void is taken.
    let pay = s
        .ledger()
        .standing_payments(ledger_core::DocRef::Invoice(3));
    assert_eq!(pay.len(), 1);
    s.reverse_entry(pay[0], d(2026, 3, 30), "Refunded").unwrap();
    s.void_document(ledger_core::DocRef::Invoice(3), d(2026, 3, 31), "duplicate")
        .unwrap();
    s.verify().unwrap();
}

/// A void writes no event of its own: it is the entry.reversed of the
/// document's entry, described "Void invoice N: reason" (P4b.7), and
/// replays to the same books.
#[test]
fn a_void_is_journaled_as_its_reversal() {
    let dir = test_dir("void");
    let mut s = Store::create(&dir.0, Currency::USD).unwrap();
    let c = s.add_customer("Fernhill Labs").unwrap();
    let inv = s
        .create_invoice(c, d(2026, 4, 1), d(2026, 5, 1), vec![line("Design", 5_000)])
        .unwrap();
    let origin = s.ledger().get_invoice(inv).unwrap().entry_id;
    let from = journal_len(&dir.0);
    let r = s
        .void_document(
            ledger_core::DocRef::Invoice(inv),
            d(2026, 4, 2),
            "duplicate",
        )
        .unwrap();
    let got = since(&dir.0, from);
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0].0, "entry.reversed");
    assert_eq!(got[0].1["id"], json!(r));
    assert_eq!(got[0].1["reverses"], json!(origin));
    assert_eq!(
        got[0].1["description"],
        json!(format!("Void invoice {inv}: duplicate"))
    );
    let live = s.ledger().clone();
    drop(s);
    let s = Store::open(&dir.0).unwrap();
    assert!(live.differs_from(s.ledger()).is_empty());
    let st = s
        .ledger()
        .invoice_state(inv, d(2026, 4, 30))
        .unwrap()
        .unwrap();
    assert_eq!(st.status, ledger_core::DocStatus::Void);
    s.verify().unwrap();
}

/// A payment with its method and reference: the reference in its
/// entry's description, the method as entry.instrument_set right after
/// the payment event (feature payment_instruments); a payment with
/// neither is journaled exactly as before, field for field.
#[test]
fn a_payment_journals_its_method_and_reference() {
    let dir = test_dir("method");
    let mut s = Store::create(&dir.0, Currency::USD).unwrap();
    let card = s
        .add_account("2050", "Card", AccountType::Liability, None, false)
        .unwrap();
    let bank = s
        .add_account("1000", "Bank", AccountType::Asset, None, false)
        .unwrap();
    let v = s.add_vendor("Tobin Harlow Supply").unwrap();
    let c = s.add_customer("Fernhill Labs").unwrap();
    let visa = s
        .add_instrument(
            ledger_core::InstrumentKind::BusinessCard,
            "Visa",
            Some("4242"),
            None,
            card,
        )
        .unwrap();
    let bill = s
        .create_bill(v, d(2026, 4, 1), d(2026, 5, 1), vec![line("Paper", 2_000)])
        .unwrap();
    let inv = s
        .create_invoice(c, d(2026, 4, 1), d(2026, 5, 1), vec![line("Design", 3_000)])
        .unwrap();
    let from = journal_len(&dir.0);
    let details = ledger_core::PaymentDetails {
        instrument: Some(visa),
        reference: Some("ch_3Nx".into()),
    };
    let e = s
        .pay_bill_with(bill, d(2026, 4, 3), cents(2_000), card, &details)
        .unwrap();
    let got = since(&dir.0, from);
    let types: Vec<&str> = got.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(
        types,
        [
            "entry.created",
            "entry.line_added",
            "entry.line_added",
            "entry.posted",
            "bill.paid",
            "entry.instrument_set"
        ]
    );
    assert_eq!(
        got[0].1["description"],
        json!(format!("Payment on bill {bill} (ref ch_3Nx)"))
    );
    assert_eq!(s.ledger().entry_instrument(e), Some(visa));
    let from = journal_len(&dir.0);
    s.receive_payment(inv, d(2026, 4, 4), cents(3_000), bank)
        .unwrap();
    let got = since(&dir.0, from);
    assert_eq!(got.len(), 5, "{got:?}");
    let keys: Vec<&String> = got[4].1.as_object().unwrap().keys().collect();
    assert_eq!(
        keys,
        ["amount", "cash_account", "date", "entry_id", "invoice_id"]
    );
    let live = s.ledger().clone();
    drop(s);
    let s = Store::open(&dir.0).unwrap();
    assert!(live.differs_from(s.ledger()).is_empty());
    s.verify().unwrap();
}

/// Tags are derived on replay, nothing journaled for them: the fixture's
/// invoice 1 carries its receivable line (Origin) and its payment's, the
/// payment's reversal its mirror; the hand line none.
#[test]
fn legacy_documents_replay_with_tags() {
    use ledger_core::{DocRef, Movement};
    let dir = fixture("tags");
    let s = Store::open(&dir.0).unwrap();
    let l = s.ledger();
    let e = l.get_invoice(1).unwrap().entry_id;
    let tag = l.line_doc(e, 0).unwrap();
    assert_eq!(
        (tag.doc, tag.movement),
        (DocRef::Invoice(1), Movement::Origin)
    );
    let lines = l.doc_lines(DocRef::Invoice(1));
    assert_eq!(
        lines
            .iter()
            .map(|x| (x.movement, x.reversal))
            .collect::<Vec<_>>(),
        [
            (Movement::Origin, false),
            (Movement::Payment, false),
            (Movement::Payment, true)
        ]
    );
    let bill = l.get_bill(1).unwrap().entry_id;
    let last = l.get_entry(bill).unwrap().lines().len() - 1;
    assert_eq!(l.line_doc(bill, last).unwrap().doc, DocRef::Bill(1));
    let hand = l
        .posted_entries()
        .find(|e| e.description() == "Hand-booked sale")
        .unwrap()
        .id();
    assert!((0..2).all(|i| l.line_doc(hand, i).is_none()));
}

/// A document's new fields (its number, terms, snapshots, references, a
/// line's account, quantity or price) wait for the journal feature
/// `documents_v2` (plan P4b S4.3; rules.md B-27): refused,
/// FormatUpgradePending, journaling nothing, in books that haven't declared
/// it; after the owner's upgrade the same writes are taken, and the books
/// replay to what was live.
#[test]
fn documents_v2_waits_for_the_upgrade() {
    let dir = fixture("documents-v2");
    let mut s = Store::open(&dir.0).unwrap();
    assert!(
        !s.journal_format()
            .has_feature(ledger_store::format::DOCUMENTS_V2)
    );
    // A revenue account for a line to name, created before the head we
    // compare (it needs nothing new).
    let revenue = s
        .add_account("4100", "Consulting", AccountType::Revenue, None, false)
        .unwrap();
    let head = s.head();
    let terms = ledger_core::Terms::new(ledger_core::TermsKind::Net(30), None).unwrap();
    let invoice_fields = arap::InvoiceFields {
        number: Some("INV-0001".into()),
        terms: Some(terms.clone()),
        ..arap::InvoiceFields::default()
    };
    let bill_fields = arap::BillFields {
        vendor_ref: Some("TOBIN-77".into()),
        received_on: Some(d(2026, 3, 2)),
        ..arap::BillFields::default()
    };
    let with_account = DocumentLine {
        description: "Consulting".into(),
        amount: cents(3_000),
        class_id: None,
        account: Some(revenue),
        quantity: None,
        unit_price: None,
    };
    for refused in [
        s.create_invoice_with(
            1,
            d(2026, 3, 1),
            d(2026, 3, 31),
            vec![line("Work", 3_000)],
            invoice_fields.clone(),
        ),
        s.create_bill_with(
            1,
            d(2026, 3, 1),
            d(2026, 3, 31),
            vec![line("Paper", 2_000)],
            bill_fields.clone(),
        ),
        s.create_invoice_with(
            1,
            d(2026, 3, 1),
            d(2026, 3, 31),
            vec![with_account.clone()],
            arap::InvoiceFields::default(),
        ),
    ] {
        match refused {
            Err(StoreError::FormatUpgradePending { need }) => {
                assert_eq!(need, "the journal feature documents_v2")
            }
            other => panic!("expected FormatUpgradePending, got {other:?}"),
        }
    }
    assert_eq!(s.head(), head, "nothing journaled");

    // Filing the issued copy needs nothing new: an old-shape invoice keeps
    // its "Invoice {id}" (no number) and the copy is filed and linked.
    let unnumbered = s
        .create_invoice_issued(
            1,
            d(2026, 3, 2),
            d(2026, 3, 31),
            vec![line("Old shape", 1_000)],
            arap::InvoiceFields::default(),
            b"%PDF-1.4 old\n%%EOF\n".to_vec(),
        )
        .unwrap();
    assert_eq!(s.ledger().get_invoice(unnumbered.0).unwrap().number, None);
    assert_eq!(
        s.attachments_for(Subject::Invoice(unnumbered.0)).unwrap()[0]
            .file_name
            .as_str(),
        format!("Invoice {}.pdf", unnumbered.0)
    );

    assert!(s.upgrade_format().unwrap().is_some());
    let inv = s
        .create_invoice_with(
            1,
            d(2026, 3, 1),
            d(2026, 3, 31),
            vec![line("Work", 3_000)],
            invoice_fields,
        )
        .unwrap();
    let bill = s
        .create_bill_with(
            1,
            d(2026, 3, 1),
            d(2026, 3, 31),
            vec![line("Paper", 2_000)],
            bill_fields,
        )
        .unwrap();
    let lined = s
        .create_invoice_with(
            1,
            d(2026, 3, 1),
            d(2026, 3, 31),
            vec![with_account],
            arap::InvoiceFields::default(),
        )
        .unwrap();
    let live = s.ledger().clone();
    // The fields are journaled, not just held live.
    let replay = live
        .invoices()
        .into_iter()
        .find(|i| i.id == inv)
        .expect("invoice 4");
    assert_eq!(replay.number.as_deref(), Some("INV-0001"));
    assert_eq!(
        replay.terms.as_ref().map(|t| t.describe()),
        Some("Net 30".into())
    );
    assert_eq!(replay.bill_to, None);
    let lined_lines = live.get_invoice(lined).unwrap().lines.clone();
    assert_eq!(lined_lines[0].account, Some(revenue));
    let replayed_bill = live.bills().into_iter().find(|b| b.id == bill).unwrap();
    assert_eq!(replayed_bill.vendor_ref.as_deref(), Some("TOBIN-77"));
    assert_eq!(replayed_bill.received_on, Some(d(2026, 3, 2)));
    drop(s);
    let s = Store::open(&dir.0).unwrap();
    assert!(
        live.differs_from(s.ledger()).is_empty(),
        "{:?}",
        live.differs_from(s.ledger())
    );
    s.verify().unwrap();
}

/// A document naming none of the new fields is journaled byte for byte as
/// every version before `documents_v2` wrote it (plan P4b S4.3; the
/// `document_line_projects` pattern): the same five events, the same
/// payload keys, no key for a field that is None.
#[test]
fn no_new_fields_journals_byte_for_byte() {
    let dir = test_dir("old-shape-documents");
    let mut s = Store::create(&dir.0, Currency::USD).unwrap();
    let c = s.add_customer("Fernhill Labs").unwrap();
    let v = s.add_vendor("Tobin Harlow Supply").unwrap();
    // A first invoice creates the control accounts, so the assertions
    // below see only the document's own events.
    s.create_invoice(c, d(2026, 3, 1), d(2026, 4, 1), vec![line("Seed", 1_000)])
        .unwrap();
    s.create_bill(v, d(2026, 3, 1), d(2026, 4, 1), vec![line("Seed", 1_000)])
        .unwrap();

    let from = journal_len(&dir.0);
    let inv = s
        .create_invoice(c, d(2026, 4, 1), d(2026, 5, 1), vec![line("Design", 5_000)])
        .unwrap();
    let got = since(&dir.0, from);
    assert_eq!(got.len(), 5, "{got:?}");
    assert_eq!(got[4].0, "invoice.created");
    let entry = s.ledger().get_invoice(inv).unwrap().entry_id;
    assert_eq!(
        got[4].1,
        json!({
            "id": inv,
            "customer_id": c,
            "date": "2026-04-01",
            "due_date": "2026-05-01",
            "lines": [{"description": "Design", "amount": "50.00"}],
            "entry_id": entry,
        }),
        "the invoice event is the shape every earlier version wrote"
    );

    let from = journal_len(&dir.0);
    let bill = s
        .create_bill(v, d(2026, 4, 1), d(2026, 5, 1), vec![line("Paper", 2_000)])
        .unwrap();
    let got = since(&dir.0, from);
    assert_eq!(got.len(), 5, "{got:?}");
    assert_eq!(got[4].0, "bill.created");
    let entry = s.ledger().get_bill(bill).unwrap().entry_id;
    assert_eq!(
        got[4].1,
        json!({
            "id": bill,
            "vendor_id": v,
            "date": "2026-04-01",
            "due_date": "2026-05-01",
            "lines": [{"description": "Paper", "amount": "20.00"}],
            "entry_id": entry,
        }),
        "the bill event is the shape every earlier version wrote"
    );
    let live = s.ledger().clone();
    drop(s);
    let s = Store::open(&dir.0).unwrap();
    assert!(live.differs_from(s.ledger()).is_empty());
    assert_eq!(s.ledger().get_invoice(inv).unwrap().number, None);
    s.verify().unwrap();
}

/// The issued copy is filed with the invoice or nothing is (plan P4b S4.3
/// item 11, A-7; rules.md B-40 item 3): a refusal leaves the journal, the
/// ledger and the allocators exactly as they were; an issue journals the
/// entry, `invoice.created`, `receipt.attached` on that entry
/// (`INV-0007.pdf`, `application/pdf`, "Issued invoice") and
/// `attachment.linked` to the invoice, and the copy found through the link
/// is the bytes given, by their SHA-256.
#[test]
fn issued_pdf_is_filed_with_the_invoice_or_nothing_is() {
    let dir = test_dir("issued-pdf");
    let mut s = Store::create(&dir.0, Currency::USD).unwrap();
    let c = s.add_customer("Fernhill Labs").unwrap();
    // The invoice's control accounts exist already, so the issue journals
    // only its own lines.
    s.add_account(
        "1200",
        "Accounts Receivable",
        AccountType::Asset,
        None,
        false,
    )
    .unwrap();
    s.add_account("4000", "Revenue", AccountType::Revenue, None, false)
        .unwrap();
    let head = s.head();
    let next_invoice = s.ledger().next_invoice_id();
    let pdf = b"%PDF-1.4\n1 0 obj\n<<>>\nendobj\ntrailer\n<<>>\n%%EOF\n".to_vec();
    let fields = |number: &str| arap::InvoiceFields {
        number: Some(number.into()),
        ..arap::InvoiceFields::default()
    };
    // An unknown customer: refused before anything is journaled.
    assert!(matches!(
        s.create_invoice_issued(
            999,
            d(2026, 4, 1),
            d(2026, 5, 1),
            vec![line("Design", 5_000)],
            fields("INV-0007"),
            pdf.clone(),
        ),
        Err(StoreError::Ledger(LedgerError::UnknownCustomer(999)))
    ));
    // A line that is not a document line: refused too.
    assert!(
        s.create_invoice_issued(
            c,
            d(2026, 4, 1),
            d(2026, 5, 1),
            vec![line("Design", -1)],
            fields("INV-0007"),
            pdf.clone(),
        )
        .is_err()
    );
    assert_eq!(s.head(), head, "nothing journaled");
    assert_eq!(s.ledger().next_invoice_id(), next_invoice);
    assert!(s.list_attachments(None).unwrap().is_empty());

    // The issue itself, in one call.
    let from = journal_len(&dir.0);
    let (inv, entry, attachment) = s
        .create_invoice_issued(
            c,
            d(2026, 4, 1),
            d(2026, 5, 1),
            vec![line("Design", 5_000)],
            fields("INV-0007"),
            pdf.clone(),
        )
        .unwrap();
    let got = since(&dir.0, from);
    let types: Vec<&str> = got.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(
        types,
        [
            "entry.created",
            "entry.line_added",
            "entry.line_added",
            "entry.posted",
            "invoice.created",
            "receipt.attached",
            "attachment.linked",
        ]
    );
    let sha = ledger_store::journal::sha256_hex(&pdf);
    assert_eq!(got[5].1["attachment_id"], json!(attachment));
    assert_eq!(got[5].1["entry_id"], json!(entry));
    assert_eq!(got[5].1["blob_sha"], json!(sha));
    assert_eq!(got[5].1["category"], json!("receipts"));
    assert_eq!(got[5].1["file_name"], json!("INV-0007.pdf"));
    assert_eq!(got[5].1["mime_type"], json!("application/pdf"));
    assert_eq!(got[5].1["description"], json!("Issued invoice"));
    assert_eq!(got[5].1["size_bytes"], json!(pdf.len() as u64));
    assert_eq!(
        got[6].1["subject"],
        json!({"kind": "invoice", "id": inv}),
        "the copy is linked to the invoice"
    );
    // Found through the link, never by guessing; and it is the entry's
    // evidence too.
    let linked = s.attachments_for(Subject::Invoice(inv)).unwrap();
    assert_eq!(linked.len(), 1);
    assert_eq!(linked[0].id, attachment);
    assert_eq!(linked[0].blob_sha, sha);
    assert_eq!(s.attachment_bytes(attachment).unwrap(), pdf);
    assert!(
        s.list_attachments(Some(entry))
            .unwrap()
            .iter()
            .any(|a| a.id == attachment)
    );
    let live = s.ledger().clone();
    drop(s);
    let s = Store::open(&dir.0).unwrap();
    assert!(live.differs_from(s.ledger()).is_empty());
    assert_eq!(s.attachments_for(Subject::Invoice(inv)).unwrap().len(), 1);
    s.verify().unwrap();
}

/// Previews journal nothing (plan P4b S4.3 item 12): the invoice preview
/// is the posting, total, number and due date the issue would take (the
/// account each line posts to and the number from the books' settings),
/// and a refusal in it is the issue's own.
#[test]
fn preview_invoice_and_bill_journal_nothing() {
    let dir = test_dir("preview");
    let mut s = Store::create(&dir.0, Currency::USD).unwrap();
    let c = s.add_customer("Fernhill Labs").unwrap();
    let v = s.add_vendor("Tobin Harlow Supply").unwrap();
    let revenue = s
        .add_account("4100", "Consulting", AccountType::Revenue, None, false)
        .unwrap();
    let expenses = s
        .add_account("6500", "Supplies", AccountType::Expense, None, false)
        .unwrap();
    // The preview's posting names the control accounts the issue would
    // use; they exist in the books because a document made them before.
    s.add_account(
        "1200",
        "Accounts Receivable",
        AccountType::Asset,
        None,
        false,
    )
    .unwrap();
    s.add_account(
        "2000",
        "Accounts Payable",
        AccountType::Liability,
        None,
        false,
    )
    .unwrap();
    let head = s.head();
    let next_invoice = s.ledger().next_invoice_id();
    let preview = s
        .preview_invoice(
            c,
            d(2026, 4, 1),
            d(2026, 5, 1),
            vec![DocumentLine {
                description: "Consulting".into(),
                amount: cents(6_000),
                class_id: None,
                account: Some(revenue),
                quantity: None,
                unit_price: None,
            }],
            arap::InvoiceFields::default(),
        )
        .unwrap();
    assert_eq!(preview.number.as_deref(), Some("INV-0001"));
    assert_eq!(preview.due_date, d(2026, 5, 1));
    assert_eq!(preview.total, cents(6_000));
    let ar = s.ledger().chart().find_by_code("1200").unwrap().id;
    assert_eq!(
        preview.posting,
        vec![
            (ar, Side::Debit, cents(6_000), None),
            (revenue, Side::Credit, cents(6_000), None),
        ]
    );
    let bill = s
        .preview_bill(
            v,
            d(2026, 4, 1),
            d(2026, 5, 1),
            vec![DocumentLine {
                description: "Supplies".into(),
                amount: cents(2_000),
                class_id: None,
                account: Some(expenses),
                quantity: None,
                unit_price: None,
            }],
            arap::BillFields::default(),
        )
        .unwrap();
    let ap = s.ledger().chart().find_by_code("2000").unwrap().id;
    assert_eq!(
        bill.posting,
        vec![
            (expenses, Side::Debit, cents(2_000), None),
            (ap, Side::Credit, cents(2_000), None),
        ]
    );
    assert_eq!(bill.total, cents(2_000));
    assert_eq!(s.head(), head, "a preview journals nothing");
    assert_eq!(s.ledger().next_invoice_id(), next_invoice);
    assert!(s.ledger().invoices().is_empty());
    // What a preview refuses, the issue refuses too.
    assert!(
        s.preview_invoice(
            999,
            d(2026, 4, 1),
            d(2026, 5, 1),
            vec![line("Design", 5_000)],
            arap::InvoiceFields::default(),
        )
        .is_err()
    );
    s.verify().unwrap();
}

/// A send, as these tests record one (plan P4b S5, A-4).
fn sent(on: NaiveDate, kind: SendKind, reference: Option<&str>) -> Sent {
    Sent {
        on,
        by: "agent".into(),
        via: SentVia::Email,
        to: vec!["billing@fernhill.example".into()],
        kind,
        reference: reference.map(str::to_string),
        note: None,
    }
}

/// R-A step 3 (plan P4b S5): `invoice.sent` is `documents_v2`'s, so books
/// that haven't declared it refuse a send, journaling nothing; after the
/// owner's go-ahead it journals, and the reopened books hold it.
#[test]
fn mark_sent_waits_for_documents_v2() {
    let dir = test_dir("sent-waits");
    let mut s = Store::create_in_format(
        &dir.0,
        Currency::USD,
        &ledger_store::JournalFormat::default(),
    )
    .unwrap();
    let c = s.add_customer("Fernhill Labs").unwrap();
    let inv = s
        .create_invoice(
            c,
            d(2026, 3, 1),
            d(2026, 3, 31),
            vec![line("Design", 10_000)],
        )
        .unwrap();
    let head = s.head();
    let from = journal_len(&dir.0);
    let one = sent(d(2026, 3, 2), SendKind::Original, None);
    match s.mark_sent(inv, one.clone()) {
        Err(StoreError::FormatUpgradePending { need }) => {
            assert_eq!(need, "the journal feature documents_v2")
        }
        other => panic!("expected FormatUpgradePending, got {other:?}"),
    }
    assert_eq!(s.head(), head, "a refusal journals nothing");
    assert_eq!(journal_len(&dir.0), from);
    assert!(s.upgrade_format().unwrap().is_some(), "the go-ahead");
    s.mark_sent(inv, one).unwrap();
    let live = s.ledger().clone();
    drop(s);
    let reopened = Store::open(&dir.0).unwrap();
    reopened.verify().unwrap();
    assert!(
        live.differs_from(reopened.ledger()).is_empty(),
        "{:?}",
        live.differs_from(reopened.ledger())
    );
    assert_eq!(reopened.ledger().get_invoice(inv).unwrap().sends.len(), 1);
}

/// R-A and R-B (plan P4b S5): a send journals one `invoice.sent` line with
/// the whole record, replays into the invoice, and projects into
/// `invoice_sends`; two sends keep their order.
#[test]
fn sends_replay_and_project() {
    let dir = test_dir("sent-replay");
    let mut s = Store::create(&dir.0, Currency::USD).unwrap();
    let c = s.add_customer("Fernhill Labs").unwrap();
    let inv = s
        .create_invoice(
            c,
            d(2026, 3, 1),
            d(2026, 3, 31),
            vec![line("Design", 10_000)],
        )
        .unwrap();
    let from = journal_len(&dir.0);
    let original = sent(d(2026, 3, 2), SendKind::Original, None);
    let reminder = sent(d(2026, 3, 20), SendKind::Reminder, Some("msg-2"));
    s.mark_sent(inv, original.clone()).unwrap();
    s.mark_sent(inv, reminder.clone()).unwrap();
    let got = since(&dir.0, from);
    assert_eq!(got.len(), 2, "{got:?}");
    assert_eq!(got[0].0, "invoice.sent");
    assert_eq!(got[0].1["invoice_id"], json!(inv));
    assert_eq!(got[0].1["on"], json!("2026-03-02"));
    assert_eq!(got[0].1["by"], json!("agent"));
    assert_eq!(got[0].1["via"], json!("email"));
    assert_eq!(got[0].1["to"], json!(["billing@fernhill.example"]));
    assert_eq!(got[0].1["kind"], json!("original"));
    assert_eq!(got[0].1.get("reference"), None, "absent, not null");
    assert_eq!(got[1].1["kind"], json!("reminder"));
    assert_eq!(got[1].1["reference"], json!("msg-2"));
    s.verify().unwrap();
    let live = s.ledger().clone();
    drop(s);
    let reopened = Store::open(&dir.0).unwrap();
    reopened.verify().unwrap();
    assert!(
        live.differs_from(reopened.ledger()).is_empty(),
        "{:?}",
        live.differs_from(reopened.ledger())
    );
    assert_eq!(
        reopened.ledger().get_invoice(inv).unwrap().sends,
        vec![original, reminder]
    );
}

/// A send changes no figure (plan P4b S5, B-38): on the fixture book an
/// earlier version wrote, every report, every derived document figure and
/// the invoice's whole DocState are the same before and after it.
#[test]
fn a_send_changes_no_figure() {
    let dir = fixture("sent-figure");
    let mut s = Store::open(&dir.0).unwrap();
    s.upgrade_format().unwrap();
    let before = ledger_core::reports::snapshot(s.ledger()).unwrap();
    let state_before = s.ledger().invoice_state(1, d(9999, 12, 31)).unwrap();
    let cached_before: Vec<_> = s
        .ledger()
        .invoices()
        .into_iter()
        .map(|i| (i.id, i.amount_paid, i.owed, i.status))
        .collect();
    s.mark_sent(1, sent(d(2026, 4, 1), SendKind::Original, None))
        .unwrap();
    assert_eq!(
        ledger_core::reports::snapshot(s.ledger()).unwrap(),
        before,
        "a send moves no report"
    );
    assert_eq!(
        s.ledger().invoice_state(1, d(9999, 12, 31)).unwrap(),
        state_before,
        "a send moves no DocState"
    );
    assert_eq!(
        s.ledger()
            .invoices()
            .into_iter()
            .map(|i| (i.id, i.amount_paid, i.owed, i.status))
            .collect::<Vec<_>>(),
        cached_before,
        "and no derived cache"
    );
    assert_eq!(
        trial_balance_json(&s),
        golden()["trial_balance"],
        "the golden's general ledger, a send included"
    );
    assert_eq!(s.ledger().get_invoice(1).unwrap().sends.len(), 1);
    s.verify().unwrap();
}

/// A void invoice is refused a send (plan P4b S5, `DocumentVoid`),
/// journaling nothing; an invoice open on the send's date but voided after
/// it keeps the send that happened first.
#[test]
fn mark_sent_on_void_refused() {
    let dir = test_dir("sent-void");
    let mut s = Store::create(&dir.0, Currency::USD).unwrap();
    let c = s.add_customer("Fernhill Labs").unwrap();
    let inv = s
        .create_invoice(
            c,
            d(2026, 3, 1),
            d(2026, 3, 31),
            vec![line("Design", 10_000)],
        )
        .unwrap();
    s.void_document(DocRef::Invoice(inv), d(2026, 3, 10), "duplicate")
        .unwrap();
    let head = s.head();
    let from = journal_len(&dir.0);
    match s.mark_sent(inv, sent(d(2026, 3, 11), SendKind::Original, None)) {
        Err(StoreError::Ledger(LedgerError::DocumentVoid(id))) => assert_eq!(id, inv),
        other => panic!("expected DocumentVoid, got {other:?}"),
    }
    assert_eq!(s.head(), head, "a refusal journals nothing");
    assert_eq!(journal_len(&dir.0), from);
    // Dated before the void, the send was a real one: it replays.
    s.mark_sent(inv, sent(d(2026, 3, 5), SendKind::Original, None))
        .unwrap();
    assert_eq!(s.ledger().get_invoice(inv).unwrap().sends.len(), 1);
    s.verify().unwrap();
}

/// A settlement of two invoices, a discount and a processor's fee: one
/// entry whose events name the lines they tag, every figure derived, and
/// the books replaying to the same lines (plan P4b S6; rules.md B-42
/// INVARIANTS 1 and 3).
#[test]
fn a_settlement_names_its_lines_and_replays() {
    use ledger_core::{Application, DocRef, Fee, SettleParams};
    let dir = test_dir("settlement-lines");
    let mut s = Store::create(&dir.0, Currency::USD).unwrap();
    let bank = s
        .add_account("1000", "Bank", AccountType::Asset, None, false)
        .unwrap();
    let c = s.add_customer("Fernhill Labs").unwrap();
    let first = s
        .create_invoice(c, d(2026, 4, 1), d(2026, 5, 1), vec![line("Design", 7_000)])
        .unwrap();
    let second = s
        .create_invoice(
            c,
            d(2026, 4, 2),
            d(2026, 5, 2),
            vec![line("Support", 2_000)],
        )
        .unwrap();
    let from = journal_len(&dir.0);
    let entry = s
        .settle(SettleParams {
            date: d(2026, 4, 10),
            description: "One deposit, two invoices".into(),
            cash_account: bank,
            party: c,
            applications: vec![
                Application {
                    doc: DocRef::Invoice(first),
                    cash: cents(6_000),
                    discount: cents(1_000),
                    discount_reason: Some("vendor honoured the early discount late".into()),
                },
                Application {
                    doc: DocRef::Invoice(second),
                    cash: cents(2_000),
                    discount: Decimal::ZERO,
                    discount_reason: None,
                },
            ],
            fee: Some(Fee {
                amount: cents(500),
                account: None,
                vendor: None,
            }),
            instrument: None,
            reference: Some("ch_42".into()),
        })
        .unwrap();
    let got = since(&dir.0, from);
    // The two role accounts created on demand, then the entry, then one
    // event per application.
    assert_eq!(
        got.iter().filter(|(t, _)| t == "account.added").count(),
        2,
        "{got:?}"
    );
    assert_eq!(
        got.iter()
            .filter(|(t, _)| t == "invoice.payment_received")
            .count(),
        2
    );
    assert_eq!(
        got.iter().filter(|(t, _)| t == "entry.line_added").count(),
        6
    );
    // One credit per invoice's cash, one for the discount, the fee, the
    // discount expense and the net deposit: the rows the event lines name.
    let events: Vec<&Value> = got
        .iter()
        .filter(|(t, _)| t == "invoice.payment_received")
        .map(|(_, p)| p)
        .collect();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["line"], json!(0));
    assert_eq!(events[0]["discount"], json!("10.00"));
    assert_eq!(events[0]["discount_line"], json!(2));
    assert_eq!(events[0]["fee"], json!("5.00"));
    assert_eq!(events[0]["fee_line"], json!(3));
    assert_eq!(events[1]["line"], json!(1));
    assert!(events[1].get("discount").is_none());
    assert!(events[1].get("fee").is_none());
    let live = s.ledger().clone();
    let ar = live.control_check(d(2026, 4, 10)).unwrap();
    let receivables = ar.iter().find(|c| c.role == "receivables").unwrap();
    assert_eq!(receivables.balance, cents(0));
    assert_eq!(receivables.documents_total, cents(0));
    assert_eq!(receivables.difference, Decimal::ZERO);
    for (id, paid, discounted) in [(first, 6_000, 1_000), (second, 2_000, 0)] {
        let st = live.invoice_state(id, d(2026, 4, 10)).unwrap().unwrap();
        assert_eq!(
            (st.paid, st.discounted, st.outstanding),
            (cents(paid), cents(discounted), Decimal::ZERO)
        );
    }
    assert_eq!(s.ledger().get_entry(entry).unwrap().lines().len(), 6);
    drop(s);
    let s = Store::open(&dir.0).unwrap();
    assert!(
        live.differs_from(s.ledger()).is_empty(),
        "{:?}",
        live.differs_from(s.ledger())
    );
    s.verify().unwrap();
}

/// A settlement of more than one document, a discount or a fee needs
/// `documents_v2`: refused, journaling nothing, in books without it, and
/// accepted after the owner's upgrade, replaying to the same books (plan
/// P4b S6; rules.md B-27, B-42).
#[test]
fn settlement_waits_for_documents_v2() {
    use ledger_core::{Application, DocRef, SettleParams};
    let dir = fixture("settlement-v2");
    let mut s = Store::open(&dir.0).unwrap();
    assert!(
        !s.journal_format()
            .has_feature(ledger_store::format::DOCUMENTS_V2)
    );
    let bank = s.ledger().chart().find_by_code("1000").unwrap().id;
    let head = s.head();
    let params = |discount: Decimal| SettleParams {
        date: d(2026, 2, 15),
        description: "Two bills, one payment".into(),
        cash_account: bank,
        party: 1,
        applications: vec![
            Application {
                doc: DocRef::Bill(1),
                cash: cents(7_000),
                discount,
                discount_reason: (!discount.is_zero()).then_some("late discount".into()),
            },
            Application {
                doc: DocRef::Bill(2),
                cash: cents(4_000),
                discount: Decimal::ZERO,
                discount_reason: None,
            },
        ],
        fee: None,
        instrument: None,
        reference: None,
    };
    match s.settle(params(Decimal::ZERO)) {
        Err(StoreError::FormatUpgradePending { need }) => {
            assert_eq!(need, "the journal feature documents_v2")
        }
        other => panic!("expected FormatUpgradePending, got {other:?}"),
    }
    assert_eq!(s.head(), head, "nothing journaled");
    s.upgrade_format().unwrap();
    // The discount is outside the invoice's terms (it has none), so it
    // needs its reason; the payment is also accepted only because the
    // earlier one was reversed, which needs document_standing too.
    s.settle(params(cents(1_000))).unwrap();
    s.verify().unwrap();
    let live = s.ledger().clone();
    drop(s);
    let s = Store::open(&dir.0).unwrap();
    assert!(
        live.differs_from(s.ledger()).is_empty(),
        "{:?}",
        live.differs_from(s.ledger())
    );
    let st = s.ledger().bill_state(1, d(2026, 12, 31)).unwrap().unwrap();
    assert_eq!(st.status, ledger_core::DocStatus::Paid);
}

/// Legacy payment events carry no `line`, so replay keeps the positional
/// rule: the first untagged control line for the payment's amount, in the
/// order the events come (plan P4b S6; rules.md B-42 INVARIANT 3).
#[test]
fn legacy_payment_events_keep_the_positional_rule() {
    use ledger_core::{DocRef, Movement};
    let dir = fixture("legacy-positional");
    let s = Store::open(&dir.0).unwrap();
    let payments: Vec<Value> = std::fs::read_to_string(dir.0.join(JOURNAL_FILE))
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .filter(|v| v["type"] == json!("invoice.payment_received"))
        .map(|v| v["payload"].clone())
        .collect();
    assert!(!payments.is_empty());
    for p in &payments {
        assert!(p.get("line").is_none(), "{p}");
    }
    let lines = s.ledger().doc_lines(DocRef::Invoice(3));
    assert_eq!(
        lines
            .iter()
            .map(|l| (l.movement, l.amount))
            .collect::<Vec<_>>(),
        [
            (Movement::Origin, cents(30_000)),
            (Movement::Payment, cents(-12_000))
        ]
    );
    assert_eq!(
        s.ledger()
            .line_doc(lines[1].entry, lines[1].line)
            .unwrap()
            .doc,
        DocRef::Invoice(3)
    );
}

// ---------------------------------------------------------------------------
// Bad-debt write-offs (plan P4b S7; rules.md B-41)
// ---------------------------------------------------------------------------

/// The evidence a write-off test files: a collection letter.
fn collection_letter() -> &'static [u8] {
    b"%PDF-1.4\nFinal demand, unpaid\n%%EOF\n"
}

/// A write-off needs journal feature documents_v2 (its event type is new;
/// an older version refuses the unknown type): refused, journaling
/// nothing, until the owner's upgrade; then it is taken and replays to the
/// same books (R-A, B-27).
#[test]
fn write_off_waits_for_documents_v2() {
    let dir = fixture("write-off-v2");
    let mut s = Store::open(&dir.0).unwrap();
    assert!(
        !s.journal_format()
            .has_feature(ledger_store::format::DOCUMENTS_V2)
    );
    let head = s.head();
    let len = journal_len(&dir.0);
    let evidence = || {
        Some(WriteOffEvidence::File {
            file_name: "collection-letter.pdf",
            mime_type: "application/pdf",
            bytes: collection_letter(),
            description: Some("Collection attempts"),
        })
    };
    match s.write_off_invoice(1, d(2026, 2, 1), cents(10_000), "uncollectible", evidence()) {
        Err(StoreError::FormatUpgradePending { need }) => {
            assert_eq!(need, "the journal feature documents_v2")
        }
        other => panic!("expected FormatUpgradePending, got {other:?}"),
    }
    assert_eq!(s.head(), head, "nothing journaled");
    assert_eq!(journal_len(&dir.0), len);
    assert!(s.upgrade_format().unwrap().is_some());
    let (entry, attachment) = s
        .write_off_invoice(1, d(2026, 2, 1), cents(10_000), "uncollectible", evidence())
        .unwrap();
    s.verify().unwrap();
    let live = s.ledger().clone();
    let st = live.invoice_state(1, d(2026, 12, 31)).unwrap().unwrap();
    assert_eq!(st.status, ledger_core::DocStatus::WrittenOff);
    assert_eq!(st.written_off, cents(10_000));
    // The evidence is on the write-off's entry and on its invoice.
    let on_entry = s.attachments_for(Subject::Entry(entry)).unwrap();
    assert_eq!(on_entry.len(), 1);
    assert_eq!(on_entry[0].id, attachment);
    let on_invoice = s.attachments_for(Subject::Invoice(1)).unwrap();
    assert!(on_invoice.iter().any(|a| a.id == attachment));
    drop(s);
    let s = Store::open(&dir.0).unwrap();
    assert!(
        live.differs_from(s.ledger()).is_empty(),
        "{:?}",
        live.differs_from(s.ledger())
    );
    s.verify().unwrap();
}

/// A write-off is journaled as its entry and one `invoice.written_off`,
/// and replay derives the same figures and the projection agrees: the
/// invoice reads Written off, the bad-debt expense is on 6990, and the
/// control account still equals its documents (B-41, B-38 INVARIANT 1).
#[test]
fn write_off_replays_and_projects() {
    let dir = test_dir("write-off-replay");
    let mut s = Store::create(&dir.0, Currency::USD).unwrap();
    let bank = s
        .add_account("1000", "Bank", AccountType::Asset, None, false)
        .unwrap();
    let c = s.add_customer("Fernhill Labs").unwrap();
    let inv = s
        .create_invoice(
            c,
            d(2026, 4, 1),
            d(2026, 5, 1),
            vec![line("Design", 10_000)],
        )
        .unwrap();
    // Half paid, the rest written off: the invoice reads Written off with
    // 50 paid and 50 written off (B-38 SCENARIO 7).
    s.receive_payment(inv, d(2026, 4, 20), cents(5_000), bank)
        .unwrap();
    let from = journal_len(&dir.0);
    let (entry, attachment) = s
        .write_off_invoice(
            inv,
            d(2026, 4, 30),
            cents(5_000),
            "collection exhausted",
            Some(WriteOffEvidence::File {
                file_name: "collection-letter.pdf",
                mime_type: "application/pdf",
                bytes: collection_letter(),
                description: Some("Collection attempts"),
            }),
        )
        .unwrap();
    // The event: one invoice.written_off with the five facts, right after
    // the entry's own events.
    let got = since(&dir.0, from);
    let written: Vec<&(String, Value)> = got
        .iter()
        .filter(|(t, _)| t == "invoice.written_off")
        .collect();
    assert_eq!(written.len(), 1, "{got:?}");
    let (_, p) = written[0];
    assert_eq!(p["invoice_id"], json!(inv));
    assert_eq!(p["entry_id"], json!(entry));
    assert_eq!(p["date"], json!("2026-04-30"));
    assert_eq!(p["amount"], json!("50.00"));
    assert_eq!(p["reason"], json!("collection exhausted"));
    // The evidence: a receipt.attached on the entry, then attachment.linked
    // to the invoice.
    assert!(got.iter().any(|(t, v)| t == "receipt.attached"
        && v["attachment_id"] == json!(attachment)
        && v["entry_id"] == json!(entry)));
    assert!(
        got.iter()
            .any(|(t, v)| t == "attachment.linked" && v["attachment_id"] == json!(attachment))
    );
    // The figures, live.
    let st = s
        .ledger()
        .invoice_state(inv, d(2026, 5, 1))
        .unwrap()
        .unwrap();
    assert_eq!(st.status, ledger_core::DocStatus::WrittenOff);
    assert_eq!(
        (st.paid, st.written_off, st.outstanding),
        (cents(5_000), cents(5_000), Decimal::ZERO)
    );
    let bad = s.ledger().chart().find_by_code("6990").unwrap().id;
    assert_eq!(s.ledger().account_balance(bad).unwrap(), cents(5_000));
    let ar = s.ledger().chart().find_by_code("1200").unwrap().id;
    assert_eq!(s.ledger().account_balance(ar).unwrap(), Decimal::ZERO);
    assert!(
        s.ledger()
            .control_check(d(2026, 5, 1))
            .unwrap()
            .iter()
            .all(|c| c.difference.is_zero() && c.attention.is_empty())
    );
    // `verify` compares the projection with the journal (the write-off
    // moves no document row: the figures are the tagged lines').
    s.verify().unwrap();
    let live = s.ledger().clone();
    drop(s);
    let s = Store::open(&dir.0).unwrap();
    assert!(
        live.differs_from(s.ledger()).is_empty(),
        "{:?}",
        live.differs_from(s.ledger())
    );
    let replayed = s
        .ledger()
        .invoice_state(inv, d(2026, 5, 1))
        .unwrap()
        .unwrap();
    assert_eq!(replayed, st);
    s.verify().unwrap();
}

/// No evidence, no write-off (B-41 INVARIANT 1; Treas. Reg.
/// §1.166-2(a)): refused, journaling nothing; an attachment already filed
/// is linked instead, to the entry and the invoice.
#[test]
fn write_off_without_evidence_is_refused() {
    let dir = test_dir("write-off-evidence");
    let mut s = Store::create(&dir.0, Currency::USD).unwrap();
    let c = s.add_customer("Fernhill Labs").unwrap();
    let inv = s
        .create_invoice(
            c,
            d(2026, 4, 1),
            d(2026, 5, 1),
            vec![line("Design", 10_000)],
        )
        .unwrap();
    let head = s.head();
    let len = journal_len(&dir.0);
    assert!(matches!(
        s.write_off_invoice(inv, d(2026, 4, 30), cents(10_000), "uncollectible", None),
        Err(StoreError::WriteOffNeedsEvidence)
    ));
    assert_eq!(s.head(), head, "nothing journaled");
    assert_eq!(journal_len(&dir.0), len);
    // An attachment that isn't in the books refuses it too, before
    // anything is written.
    assert!(matches!(
        s.write_off_invoice(
            inv,
            d(2026, 4, 30),
            cents(10_000),
            "uncollectible",
            Some(WriteOffEvidence::Existing(999)),
        ),
        Err(StoreError::UnknownAttachment(999))
    ));
    assert_eq!(s.head(), head, "nothing journaled");
    // One already filed (unlinked) is linked to the write-off's entry and
    // its invoice.
    let (attachment, _, _) = s
        .file_document(
            "other",
            "collection-letter.pdf",
            "application/pdf",
            collection_letter(),
            Some("Collection attempts"),
        )
        .unwrap();
    let (entry, evidence) = s
        .write_off_invoice(
            inv,
            d(2026, 4, 30),
            cents(10_000),
            "uncollectible",
            Some(WriteOffEvidence::Existing(attachment)),
        )
        .unwrap();
    assert_eq!(evidence, attachment);
    assert!(
        s.attachments_for(Subject::Entry(entry))
            .unwrap()
            .iter()
            .any(|a| a.id == attachment)
    );
    assert!(
        s.attachments_for(Subject::Invoice(inv))
            .unwrap()
            .iter()
            .any(|a| a.id == attachment)
    );
    s.verify().unwrap();
}
