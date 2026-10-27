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
use ledger_core::arap::DocumentLine;
use ledger_core::{AccountType, Currency, Money, Side};
use ledger_store::{JOURNAL_FILE, Store, StoreError};
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

/// S4.1's line fields (the account, the quantity and unit price) are not
/// journaled yet: S4.3's `documents_v2` writes them. The store refuses
/// such a line before it rehearses or journals anything, so replay can
/// never lose one; an ordinary line still posts exactly as before, with
/// no new key in its journal payload.
#[test]
fn document_line_fields_wait_for_documents_v2() {
    let dir = test_dir("unjournaled-fields");
    let mut s = Store::create(&dir.0, Currency::USD).unwrap();
    let bank = s
        .add_account("1000", "Bank", AccountType::Asset, None, false)
        .unwrap();
    let c = s.add_customer("Fernhill Labs").unwrap();
    let v = s.add_vendor("Tobin Harlow Supply").unwrap();
    // A first invoice creates the control accounts, so the assertions
    // below see only the document's own events.
    s.create_invoice(c, d(2026, 3, 1), d(2026, 4, 1), vec![line("Seed", 1_000)])
        .unwrap();
    let from = journal_len(&dir.0);
    let next_invoice = s.ledger().next_invoice_id();
    let next_bill = s.ledger().next_bill_id();

    let with_account = DocumentLine {
        description: "Design".into(),
        amount: cents(5_000),
        class_id: None,
        account: Some(bank),
        quantity: None,
        unit_price: None,
    };
    for refused in [
        s.create_invoice(c, d(2026, 4, 1), d(2026, 5, 1), vec![with_account.clone()]),
        s.create_bill(v, d(2026, 4, 1), d(2026, 5, 1), vec![with_account]),
    ] {
        assert!(
            matches!(
                refused,
                Err(StoreError::DocumentFieldsAwaitUpgrade {
                    field: "an account"
                })
            ),
            "{refused:?}"
        );
    }

    let with_price = DocumentLine {
        description: "Hours".into(),
        amount: cents(9_999),
        class_id: None,
        account: None,
        quantity: Some(Decimal::from(3)),
        unit_price: Some(cents(3_333)),
    };
    assert!(matches!(
        s.create_invoice(c, d(2026, 4, 1), d(2026, 5, 1), vec![with_price.clone()]),
        Err(StoreError::DocumentFieldsAwaitUpgrade {
            field: "a quantity and unit price"
        })
    ));
    assert!(matches!(
        s.create_bill(v, d(2026, 4, 1), d(2026, 5, 1), vec![with_price]),
        Err(StoreError::DocumentFieldsAwaitUpgrade {
            field: "a quantity and unit price"
        })
    ));

    // Every refusal left the journal and the allocators exactly as they
    // were.
    assert_eq!(journal_len(&dir.0), from);
    assert_eq!(s.ledger().next_invoice_id(), next_invoice);
    assert_eq!(s.ledger().next_bill_id(), next_bill);
    s.verify().unwrap();

    // An ordinary line posts exactly as before: the same five events, and
    // the line carries no key the older journal did not have.
    let inv = s
        .create_invoice(c, d(2026, 4, 1), d(2026, 5, 1), vec![line("Design", 5_000)])
        .unwrap();
    let got = since(&dir.0, from);
    assert_eq!(got.len(), 5, "{got:?}");
    assert_eq!(got[4].0, "invoice.created");
    let keys: Vec<&String> = got[4].1["lines"][0].as_object().unwrap().keys().collect();
    assert_eq!(keys, ["amount", "description"]);
    let live = s.ledger().clone();
    drop(s);
    let s = Store::open(&dir.0).unwrap();
    assert!(live.differs_from(s.ledger()).is_empty());
    assert_eq!(
        s.ledger().get_invoice(inv).unwrap().total(),
        Ok(cents(5_000))
    );
    s.verify().unwrap();
}
