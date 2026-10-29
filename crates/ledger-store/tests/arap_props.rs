//! Documents through the store, at random (rules.md B-38 INVARIANTS 1, 2,
//! 4 and 6; plan P4b.15): histories of invoices, bills, payments (with a
//! payment method and a reference, or none), voids, reversals of any
//! entry, closed months and refused hand lines, run through `Store`, then
//! replayed (the books reopened) and read by a `Snapshot`.
//!
//! ```text
//! GOAL: a document's state is a function of the journal and the day:
//!       the live books, their replay and a viewer's snapshot agree on
//!       every figure, at every date, and the control accounts equal
//!       their documents.
//! SOURCE: docs/spec/rules.md B-28 (replay), B-38 INVARIANTS 1, 2, 4, 6.
//! ```

use chrono::NaiveDate;
use ledger_core::arap::DocumentLine;
use ledger_core::{
    AccountType, Currency, DocRef, Ledger, LedgerError, Money, PaymentDetails, Side,
};
use ledger_store::{Store, StoreError, snapshot::Snapshot};
use proptest::prelude::*;
use rust_decimal::Decimal;
use std::path::{Path, PathBuf};

fn day(n: u8) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 1, 1).unwrap() + chrono::Days::new(u64::from(n))
}

fn cents(c: i64) -> Decimal {
    Decimal::new(c, 2)
}

/// A test's dir, removed when the test ends, pass or panic.
struct TestDir(PathBuf);

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn test_dir() -> TestDir {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "wanderbooks-arap-props-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    TestDir(dir)
}

/// One step of a random history.
#[derive(Clone, Debug)]
enum Op {
    /// An invoice (true) or a bill, on day `n`, for `c` cents.
    Create(bool, u8, i64),
    /// A payment on document `k` (mod the documents) on day `n`, for `c`
    /// cents, by card or not, with a reference or not.
    Pay(usize, u8, i64, bool, bool),
    /// Reverse entry `k` (mod the posted entries) on day `n`.
    Reverse(usize, u8),
    /// Void document `k` on day `n`.
    Void(usize, u8),
    /// Record a send of document `k` (an invoice; a bill is skipped) on
    /// day `n`, a reminder after its first send. A send moves no figure.
    MarkSent(usize, u8),
    /// Close month `m` (January to March), in order.
    Close(u8),
    /// A hand line on receivables (true) or payables: always refused.
    Hand(bool, i64),
}

fn arb_op() -> impl Strategy<Value = Op> {
    prop_oneof![
        3 => (any::<bool>(), 0u8..80, 1i64..50_000).prop_map(|(i, n, c)| Op::Create(i, n, c)),
        4 => (0usize..8, 0u8..90, 1i64..60_000, any::<bool>(), any::<bool>())
            .prop_map(|(k, n, c, card, r)| Op::Pay(k, n, c, card, r)),
        3 => (0usize..40, 0u8..90).prop_map(|(k, n)| Op::Reverse(k, n)),
        1 => (0usize..8, 0u8..90).prop_map(|(k, n)| Op::Void(k, n)),
        2 => (0usize..8, 0u8..90).prop_map(|(k, n)| Op::MarkSent(k, n)),
        1 => (1u8..4).prop_map(Op::Close),
        1 => (any::<bool>(), 1i64..10_000).prop_map(|(r, c)| Op::Hand(r, c)),
    ]
}

/// The books' fixed accounts: bank, card (and its payment method).
struct Setup {
    bank: u64,
    card: u64,
    visa: u64,
    customer: u64,
    vendor: u64,
}

fn setup(dir: &Path) -> (Store, Setup) {
    let mut s = Store::create(dir, Currency::USD).unwrap();
    let bank = s
        .add_account("1000", "Bank", AccountType::Asset, None, false)
        .unwrap();
    let card = s
        .add_account("2050", "Card", AccountType::Liability, None, false)
        .unwrap();
    s.add_account("3000", "Capital", AccountType::Equity, None, false)
        .unwrap();
    let visa = s
        .add_instrument(
            ledger_core::InstrumentKind::BusinessCard,
            "Fernhill Visa",
            Some("4242"),
            None,
            card,
        )
        .unwrap();
    let customer = s.add_customer("Fernhill Labs").unwrap();
    let vendor = s.add_vendor("Tobin Harlow Supply").unwrap();
    (
        s,
        Setup {
            bank,
            card,
            visa,
            customer,
            vendor,
        },
    )
}

/// Run `ops` on the store; a refusal must journal nothing.
fn run(s: &mut Store, k: &Setup, ops: &[Op], docs: &mut Vec<DocRef>) -> Result<(), TestCaseError> {
    for op in ops {
        let head = s.head();
        let line = |c: i64| {
            vec![DocumentLine {
                description: "work".into(),
                amount: cents(c),
                class_id: None,
                account: None,
                quantity: None,
                unit_price: None,
            }]
        };
        let result: Result<(), StoreError> = match *op {
            Op::Create(true, n, c) => s
                .create_invoice(k.customer, day(n), day(n) + chrono::Days::new(30), line(c))
                .map(|i| docs.push(DocRef::Invoice(i))),
            Op::Create(false, n, c) => s
                .create_bill(k.vendor, day(n), day(n) + chrono::Days::new(30), line(c))
                .map(|i| docs.push(DocRef::Bill(i))),
            Op::Pay(..) if docs.is_empty() => Ok(()),
            Op::Pay(i, n, c, by_card, reference) => {
                let details = PaymentDetails {
                    instrument: by_card.then_some(k.visa),
                    reference: reference.then(|| format!("trace {i}{n}")),
                };
                match docs[i % docs.len()] {
                    DocRef::Invoice(id) => s
                        .receive_payment_with(id, day(n), cents(c), k.bank, &details)
                        .map(drop),
                    DocRef::Bill(id) => {
                        let from = if by_card { k.card } else { k.bank };
                        s.pay_bill_with(id, day(n), cents(c), from, &details)
                            .map(drop)
                    }
                }
            }
            Op::Reverse(i, n) => {
                let posted: Vec<u64> = s.ledger().posted_entries().map(|e| e.id()).collect();
                match posted.get(i % posted.len().max(1)) {
                    Some(&e) => s.reverse_entry(e, day(n), "undo").map(drop),
                    None => Ok(()),
                }
            }
            Op::Void(..) if docs.is_empty() => Ok(()),
            Op::Void(i, n) => s
                .void_document(docs[i % docs.len()], day(n), "duplicate")
                .map(drop),
            Op::MarkSent(..) if docs.is_empty() => Ok(()),
            Op::MarkSent(i, n) => match docs[i % docs.len()] {
                // Bills get no sends: we receive them (plan P4b S5).
                DocRef::Bill(_) => Ok(()),
                DocRef::Invoice(id) => {
                    let kind = if s
                        .ledger()
                        .get_invoice(id)
                        .is_some_and(|i| !i.sends.is_empty())
                    {
                        ledger_core::arap::SendKind::Reminder
                    } else {
                        ledger_core::arap::SendKind::Original
                    };
                    s.mark_sent(
                        id,
                        ledger_core::arap::Sent {
                            on: day(n),
                            by: "agent".into(),
                            via: ledger_core::arap::SentVia::Email,
                            to: vec!["billing@fernhill.example".into()],
                            kind,
                            reference: None,
                            note: None,
                        },
                    )
                    .map(drop)
                }
            },
            Op::Close(m) => {
                let start = NaiveDate::from_ymd_opt(2026, u32::from(m), 1).unwrap();
                let end = start + chrono::Months::new(1) - chrono::Days::new(1);
                s.create_period(&format!("2026-{m:02}"), start, end)
                    .and_then(|p| {
                        let mid = s.head();
                        s.close_period(p).map(drop).inspect_err(|_| {
                            // The period itself was journaled: only the
                            // close must leave nothing behind.
                            assert_eq!(s.head(), mid);
                        })
                    })
                    .map(drop)
            }
            Op::Hand(receivables, c) => {
                let code = if receivables { "1200" } else { "2000" };
                match s.ledger().chart().find_by_code(code).map(|a| a.id) {
                    Some(a) if s.ledger().control_role_of(a).is_some() => {
                        // After every month the history can close.
                        let on = NaiveDate::from_ymd_opt(2026, 12, 1).unwrap();
                        let e = s.create_entry(on, "hand").unwrap();
                        let mid = s.head();
                        let m = Money::new(cents(c), Currency::USD);
                        let r = s.add_line(e, a, Side::Debit, m);
                        prop_assert!(
                            matches!(
                                r,
                                Err(StoreError::Ledger(LedgerError::UntaggedControlLine { .. }))
                            ),
                            "{r:?}"
                        );
                        prop_assert_eq!(s.head(), mid);
                        Ok(())
                    }
                    _ => Ok(()),
                }
            }
        };
        if result.is_err() && !matches!(op, Op::Close(_)) {
            prop_assert_eq!(s.head(), head, "{:?} refused, yet journaled", op);
        }
    }
    Ok(())
}

/// Every day a figure could change on, and the end.
fn days(l: &Ledger) -> Vec<NaiveDate> {
    let mut v: Vec<NaiveDate> = l.posted_entries().map(|e| e.date()).collect();
    v.push(NaiveDate::from_ymd_opt(2026, 12, 31).unwrap());
    v.sort();
    v.dedup();
    v
}

/// What the books say about their documents on day `on`: each state, the
/// control check and both agings, as one comparable value.
fn figures(l: &Ledger, docs: &[DocRef], on: NaiveDate) -> serde_json::Value {
    let states: Vec<_> = docs.iter().map(|d| l.doc_state(*d, on).unwrap()).collect();
    serde_json::json!({
        "states": states,
        "control": l.control_check(on).unwrap(),
        "ar": l.ar_aging(on).unwrap(),
        "ap": l.ap_aging(on).unwrap(),
    })
}

/// The live books, their replay and a snapshot agree; the control
/// accounts equal their documents, each within bounds, at every date.
fn agree(live: &Ledger, dir: &Path, docs: &[DocRef]) -> Result<(), TestCaseError> {
    let replayed = Store::open(dir).unwrap();
    let snap = Snapshot::load(dir).unwrap();
    prop_assert!(
        live.differs_from(replayed.ledger()).is_empty(),
        "replay differs: {:?}",
        live.differs_from(replayed.ledger())
    );
    prop_assert!(
        live.differs_from(snap.ledger()).is_empty(),
        "snapshot differs: {:?}",
        live.differs_from(snap.ledger())
    );
    for on in days(live) {
        let f = figures(live, docs, on);
        prop_assert_eq!(
            &f,
            &figures(replayed.ledger(), docs, on),
            "replay on {}",
            on
        );
        prop_assert_eq!(&f, &figures(snap.ledger(), docs, on), "snapshot on {}", on);
        for c in live.control_check(on).unwrap() {
            prop_assert_eq!(c.difference, Decimal::ZERO);
            prop_assert!(c.unattached.is_empty() && c.attention.is_empty(), "{:?}", c);
        }
        for d in docs {
            if let Some(st) = live.doc_state(*d, on).unwrap() {
                prop_assert!(!st.out_of_bounds(), "{d} on {on}: {st:?}");
            }
        }
    }
    replayed.verify().unwrap();
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    /// B-38 INVARIANT 4 (and 1, 2, 6 through the store): a document's
    /// state is a function of the journal and the day. Half the history,
    /// then the books reopened and compared; the rest on the reopened
    /// books, compared again.
    #[test]
    fn status_is_a_function_of_journal_and_date(
        first in prop::collection::vec(arb_op(), 0..18),
        then in prop::collection::vec(arb_op(), 0..18),
    ) {
        let dir = test_dir();
        let (mut s, k) = setup(&dir.0);
        let mut docs = Vec::new();
        run(&mut s, &k, &first, &mut docs)?;
        let live = s.ledger().clone();
        drop(s);
        agree(&live, &dir.0, &docs)?;
        let mut s = Store::open(&dir.0).unwrap();
        run(&mut s, &k, &then, &mut docs)?;
        let live = s.ledger().clone();
        drop(s);
        agree(&live, &dir.0, &docs)?;
    }
}
