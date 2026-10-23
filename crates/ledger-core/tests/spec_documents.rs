//! Documents read their figures from the ledger lines tagged with them
//! (rules.md B-38; plan P4b S1, D-45): one test per scenario of the
//! B-38 spec block, then the properties.
//!
//! ```text
//! GOAL: invoices and bills are the subledgers of the receivables and
//!       payables control accounts; every figure about a document is read
//!       from the ledger lines tagged with it.
//! SOURCE: docs/spec/rules.md B-8, B-9, B-10, B-18, B-23, B-28, B-38;
//!         ASC 310-10, ASC 405-20-40-1, ASC 606-10-45-4.
//! ```

use chrono::NaiveDate;
use ledger_core::arap::DocumentLine;
use ledger_core::{
    AccountId, AccountType, Currency, DocRef, DocStatus, Ledger, LedgerError, Money, Movement,
    PaymentDetails, Side,
};
use proptest::prelude::*;
use rust_decimal::Decimal;

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn cents(c: i64) -> Decimal {
    Decimal::new(c, 2)
}

fn usd(c: i64) -> Money {
    Money::new(cents(c), Currency::USD)
}

fn line(what: &str, c: i64) -> Vec<DocumentLine> {
    vec![DocumentLine {
        description: what.into(),
        amount: cents(c),
        class_id: None,
    }]
}

/// Bank 1000, a card 2050, capital 3000, revenue 4000 (made by the first
/// invoice), and a customer and a vendor.
struct Books {
    l: Ledger,
    bank: AccountId,
    card: AccountId,
    capital: AccountId,
    customer: u64,
    vendor: u64,
}

fn books() -> Books {
    let mut l = Ledger::new(Currency::USD);
    let bank = l
        .add_account("1000", "Bank", AccountType::Asset, None, false)
        .unwrap();
    let card = l
        .add_account("2050", "Card", AccountType::Liability, None, false)
        .unwrap();
    let capital = l
        .add_account("3000", "Capital", AccountType::Equity, None, false)
        .unwrap();
    let customer = l.add_customer("Fernhill Labs").unwrap();
    let vendor = l.add_vendor("Tobin Harlow Supply").unwrap();
    Books {
        l,
        bank,
        card,
        capital,
        customer,
        vendor,
    }
}

impl Books {
    fn invoice(&mut self, on: NaiveDate, c: i64) -> u64 {
        self.l
            .create_invoice(
                self.customer,
                on,
                on + chrono::Days::new(30),
                line("Work", c),
            )
            .unwrap()
    }

    fn bill(&mut self, on: NaiveDate, c: i64) -> u64 {
        self.l
            .create_bill(
                self.vendor,
                on,
                on + chrono::Days::new(30),
                line("Paper", c),
            )
            .unwrap()
    }

    fn state(&self, doc: DocRef, on: NaiveDate) -> ledger_core::DocState {
        self.l.doc_state(doc, on).unwrap().expect("issued by then")
    }

    fn account(&self, code: &str) -> AccountId {
        self.l.chart().find_by_code(code).unwrap().id
    }

    /// Every control account's tie-out holds: no difference (B-38
    /// INVARIANT 1).
    fn ties(&self, on: NaiveDate) {
        for c in self.l.control_check(on).unwrap() {
            assert_eq!(c.difference, Decimal::ZERO, "{c:?}");
        }
    }
}

/// Scenario 1: invoice 100, payment 100, payment reversed: Open, owing
/// 100, A/R 100.
#[test]
fn reversing_a_payment_reopens_the_invoice() {
    let mut b = books();
    let inv = b.invoice(d(2026, 1, 10), 10_000);
    let pay =
        b.l.receive_payment(inv, d(2026, 1, 20), cents(10_000), b.bank)
            .unwrap();
    let doc = DocRef::Invoice(inv);
    assert_eq!(b.state(doc, d(2026, 1, 21)).status, DocStatus::Paid);
    b.l.reverse_entry(pay, d(2026, 1, 25), "Bounced").unwrap();
    let st = b.state(doc, d(2026, 1, 31));
    assert_eq!(st.status, DocStatus::Open);
    assert_eq!((st.paid, st.outstanding), (Decimal::ZERO, cents(10_000)));
    let ar = b.account("1200");
    assert_eq!(b.l.account_balance(ar).unwrap(), cents(10_000));
    // The record's figures follow: what is owed at the end of the books.
    assert_eq!(b.l.get_invoice(inv).unwrap().balance(), Ok(cents(10_000)));
    // The mirror carries the document: the payment and its reversal.
    let lines = b.l.doc_lines(doc);
    assert_eq!(lines.len(), 3);
    assert!(
        lines
            .iter()
            .filter(|l| l.reversal)
            .all(|l| l.movement == Movement::Payment)
    );
    b.ties(d(2026, 1, 31));
}

/// Scenario 2: an invoice reversed with nothing paid is Void, not in the
/// aging, its id (later its number) kept.
#[test]
fn a_voided_invoice_never_ages() {
    let mut b = books();
    let inv = b.invoice(d(2026, 2, 1), 20_000);
    let r =
        b.l.void_document(DocRef::Invoice(inv), d(2026, 2, 5), "wrong customer")
            .unwrap();
    assert_eq!(
        b.l.get_entry(r).unwrap().description(),
        format!("Void invoice {inv}: wrong customer")
    );
    let st = b.state(DocRef::Invoice(inv), d(2026, 3, 31));
    assert_eq!(
        (st.status, st.outstanding),
        (DocStatus::Void, Decimal::ZERO)
    );
    assert!(b.l.ar_aging(d(2026, 3, 31)).unwrap().is_empty());
    assert!(b.l.get_invoice(inv).is_some(), "kept");
    // Before the void it was open, and aged.
    assert_eq!(
        b.state(DocRef::Invoice(inv), d(2026, 2, 4)).status,
        DocStatus::Open
    );
    assert_eq!(
        b.l.ar_aging(d(2026, 2, 4)).unwrap()[0].current,
        cents(20_000)
    );
    // Void already: refused, naming the reversal.
    match b
        .l
        .void_document(DocRef::Invoice(inv), d(2026, 2, 6), "again")
    {
        Err(LedgerError::InvalidVoid(why)) => assert!(why.contains(&r.to_string()), "{why}"),
        other => panic!("expected InvalidVoid, got {other:?}"),
    }
    // Reversing the void re-instates it (B-10); voiding again reverses
    // that reversal, the end of the chain.
    let back = b.l.reverse_entry(r, d(2026, 2, 7), "Re-instate").unwrap();
    assert_eq!(
        b.state(DocRef::Invoice(inv), d(2026, 2, 8)).status,
        DocStatus::Open
    );
    let again =
        b.l.void_document(DocRef::Invoice(inv), d(2026, 2, 9), "duplicate")
            .unwrap();
    assert_eq!(b.l.get_entry(again).unwrap().reversal_of(), Some(back));
    assert_eq!(
        b.state(DocRef::Invoice(inv), d(2026, 2, 9)).status,
        DocStatus::Void
    );
    b.ties(d(2026, 2, 28));
}

/// Scenario 3: a void with a payment standing SHOULD refuse, naming the
/// payment; after the payment's reversal it is taken.
#[test]
fn void_refused_while_paid() {
    let mut b = books();
    let inv = b.invoice(d(2026, 3, 1), 10_000);
    let pay =
        b.l.receive_payment(inv, d(2026, 3, 5), cents(4_000), b.bank)
            .unwrap();
    let before = b.l.clone();
    let origin = b.l.get_invoice(inv).unwrap().entry_id;
    for refused in [
        b.l.void_document(DocRef::Invoice(inv), d(2026, 3, 10), "duplicate"),
        b.l.reverse_entry(origin, d(2026, 3, 10), "Void"),
    ] {
        match refused {
            Err(LedgerError::DocumentBounds(why)) => {
                assert!(why.contains(&format!("entry {pay}")), "{why}")
            }
            other => panic!("expected DocumentBounds, got {other:?}"),
        }
        assert!(
            b.l.differs_from(&before).is_empty(),
            "a refusal changes nothing"
        );
    }
    b.l.reverse_entry(pay, d(2026, 3, 9), "Refund").unwrap();
    b.l.void_document(DocRef::Invoice(inv), d(2026, 3, 10), "duplicate")
        .unwrap();
    assert_eq!(
        b.state(DocRef::Invoice(inv), d(2026, 3, 31)).status,
        DocStatus::Void
    );
    b.ties(d(2026, 3, 31));
}

/// Scenario 4: as of a date between a payment and its reversal, it counts
/// as paid; aging as of then ignores the later reversal.
#[test]
fn aging_as_of_ignores_a_later_reversal() {
    let mut b = books();
    let inv = b.invoice(d(2026, 1, 10), 10_000);
    let bill = b.bill(d(2026, 1, 15), 8_000);
    let p =
        b.l.receive_payment(inv, d(2026, 1, 20), cents(10_000), b.bank)
            .unwrap();
    let q =
        b.l.pay_bill(bill, d(2026, 1, 20), cents(8_000), b.bank)
            .unwrap();
    b.l.reverse_entry(p, d(2026, 1, 25), "Bounced").unwrap();
    b.l.reverse_entry(q, d(2026, 1, 28), "Paid in error")
        .unwrap();
    assert!(b.l.ar_aging(d(2026, 1, 22)).unwrap().is_empty());
    assert!(b.l.ap_aging(d(2026, 1, 22)).unwrap().is_empty());
    assert!(b.l.stands_on(p, d(2026, 1, 22)));
    assert!(!b.l.stands_on(p, d(2026, 1, 25)));
    assert!(!b.l.stands_on(p, d(2026, 1, 19)), "not before its date");
    assert_eq!(
        b.l.ar_aging(d(2026, 1, 31)).unwrap()[0].current,
        cents(10_000)
    );
    assert_eq!(
        b.l.ap_aging(d(2026, 1, 31)).unwrap()[0].current,
        cents(8_000)
    );
    assert_eq!(
        b.state(DocRef::Bill(bill), d(2026, 1, 27)).status,
        DocStatus::Paid
    );
    assert_eq!(
        b.state(DocRef::Bill(bill), d(2026, 1, 28)).status,
        DocStatus::Open
    );
    // An overdue flag, not a status.
    let st = b.state(DocRef::Invoice(inv), d(2026, 3, 1));
    assert_eq!((st.status, st.overdue_days), (DocStatus::Open, 20));
}

/// Scenario 5: the wrong payment reversed, the right one is accepted;
/// a payment above what is owed on any later day is still refused.
#[test]
fn a_corrected_payment_after_reversing_the_wrong_one() {
    let mut b = books();
    let inv = b.invoice(d(2026, 1, 10), 10_000);
    let wrong =
        b.l.receive_payment(inv, d(2026, 1, 20), cents(10_000), b.bank)
            .unwrap();
    b.l.reverse_entry(wrong, d(2026, 1, 21), "Wrong invoice")
        .unwrap();
    assert_eq!(
        b.l.payments_recorded(DocRef::Invoice(inv)),
        Ok(cents(10_000))
    );
    b.l.receive_payment(inv, d(2026, 1, 22), cents(10_000), b.bank)
        .unwrap();
    let st = b.state(DocRef::Invoice(inv), d(2026, 1, 31));
    assert_eq!((st.status, st.paid), (DocStatus::Paid, cents(10_000)));
    assert_eq!(
        b.l.payments_recorded(DocRef::Invoice(inv)),
        Ok(cents(20_000))
    );
    assert!(matches!(
        b.l.receive_payment(inv, d(2026, 1, 23), cents(1), b.bank),
        Err(LedgerError::Overpayment(..))
    ));
    // A payment dated before a later one is bounded by every day after
    // it: dated on the 21st, only nothing more could be paid.
    let mut c = books();
    let inv = c.invoice(d(2026, 1, 10), 10_000);
    c.l.receive_payment(inv, d(2026, 1, 25), cents(6_000), c.bank)
        .unwrap();
    assert_eq!(
        c.l.receive_payment(inv, d(2026, 1, 21), cents(5_000), c.bank),
        Err(LedgerError::Overpayment(cents(5_000), cents(4_000)))
    );
    c.l.receive_payment(inv, d(2026, 1, 21), cents(4_000), c.bank)
        .unwrap();
}

/// Re-instating a payment (reversing its reversal) beside the payment that
/// replaced it would overpay: refused. Reversing a payment is always
/// taken.
#[test]
fn reinstating_a_payment_beside_its_replacement_is_refused() {
    let mut b = books();
    let inv = b.invoice(d(2026, 1, 10), 10_000);
    let first =
        b.l.receive_payment(inv, d(2026, 1, 20), cents(10_000), b.bank)
            .unwrap();
    let undo = b.l.reverse_entry(first, d(2026, 1, 21), "Bounced").unwrap();
    b.l.receive_payment(inv, d(2026, 1, 22), cents(10_000), b.bank)
        .unwrap();
    let before = b.l.clone();
    assert!(matches!(
        b.l.reverse_entry(undo, d(2026, 1, 23), "It cleared after all"),
        Err(LedgerError::DocumentBounds(_))
    ));
    assert!(b.l.differs_from(&before).is_empty());
}

/// Scenario 6a: a hand line on receivables or payables SHOULD refuse with
/// the document tools named, when added and (a draft drawn up before the
/// role moved there) when posted; a recurring template naming one too.
#[test]
fn hand_line_on_control_refused() {
    let mut b = books();
    b.invoice(d(2026, 1, 10), 10_000);
    b.bill(d(2026, 1, 10), 1_000);
    let (ar, ap, revenue) = (b.account("1200"), b.account("2000"), b.account("4000"));
    for (account, side) in [(ar, Side::Debit), (ap, Side::Credit)] {
        let e = b.l.try_create_entry(d(2026, 1, 15), "by hand").unwrap();
        match b.l.add_line(e, account, side, usd(2_500)) {
            Err(LedgerError::UntaggedControlLine { account, at }) => {
                assert!(account.starts_with("1200") || account.starts_with("2000"));
                assert_eq!(at, format!("line 0 of entry {e}"));
            }
            other => panic!("expected UntaggedControlLine, got {other:?}"),
        }
        let err =
            b.l.add_line_fx(
                e,
                account,
                side,
                Currency::new("EUR").unwrap(),
                cents(100),
                Decimal::ONE,
            )
            .unwrap_err();
        assert!(matches!(err, LedgerError::UntaggedControlLine { .. }));
        let text = LedgerError::UntaggedControlLine {
            account: "1200".into(),
            at: "x".into(),
        }
        .to_string();
        for tool in [
            "books.create_invoice",
            "books.receive_payment",
            "books.reverse_entry",
        ] {
            assert!(text.contains(tool), "{text}");
        }
    }
    // A draft drawn up while 1150 held no role, posted after receivables
    // moved there (its balance zero): refused at the post.
    let mut c = books();
    let other =
        c.l.add_account("1150", "Trade Receivables", AccountType::Asset, None, false)
            .unwrap();
    let e =
        c.l.try_create_entry(d(2026, 1, 15), "drafted early")
            .unwrap();
    c.l.add_line(e, other, Side::Debit, usd(100)).unwrap();
    c.l.add_line(e, c.capital, Side::Credit, usd(100)).unwrap();
    c.l.assign_role("receivables", Some(other)).unwrap();
    assert!(matches!(
        c.l.post_entry(e),
        Err(LedgerError::UntaggedControlLine { .. })
    ));
    // A recurring template naming a control account.
    let err =
        b.l.register_recurring(ledger_core::NewRecurring {
            description: "Retainer".into(),
            lines: vec![
                (ar, Side::Debit, usd(1_000)),
                (revenue, Side::Credit, usd(1_000)),
            ],
            frequency: ledger_core::Frequency::Monthly,
            start: d(2026, 2, 1),
            end: None,
        })
        .unwrap_err();
    assert!(
        matches!(err, LedgerError::UntaggedControlLine { .. }),
        "{err:?}"
    );
    // An account at the default code but of another type fills no role:
    // lines on it are anyone's.
    let mut e2 = Ledger::new(Currency::USD);
    let savings = e2
        .add_account("2000", "Savings", AccountType::Asset, None, false)
        .unwrap();
    let cap = e2
        .add_account("3000", "Capital", AccountType::Equity, None, false)
        .unwrap();
    let e = e2.try_create_entry(d(2026, 1, 1), "save").unwrap();
    e2.add_line(e, savings, Side::Debit, usd(5)).unwrap();
    e2.add_line(e, cap, Side::Credit, usd(5)).unwrap();
    e2.post_entry(e).unwrap();
}

/// Scenario 6b: reversing a legacy hand line on receivables is accepted,
/// and what is unattached shrinks to nothing.
#[test]
fn reversing_a_legacy_hand_line_is_allowed() {
    let mut b = books();
    b.invoice(d(2026, 1, 10), 10_000);
    let (ar, revenue) = (b.account("1200"), b.account("4000"));
    // A hand line as an earlier version wrote it: replayed.
    let hand = ledger_core::parties::replaying(|| {
        let e = b.l.create_entry(d(2026, 1, 15), "Hand-booked sale");
        b.l.add_line(e, ar, Side::Debit, usd(2_500)).unwrap();
        b.l.add_line(e, revenue, Side::Credit, usd(2_500)).unwrap();
        b.l.post_entry(e).unwrap();
        e
    });
    let check = |l: &Ledger, on| {
        l.control_check(on)
            .unwrap()
            .into_iter()
            .find(|c| c.role == "receivables")
            .unwrap()
    };
    let c = check(&b.l, d(2026, 1, 31));
    assert_eq!(c.balance, cents(12_500));
    assert_eq!(c.documents_total, cents(10_000));
    assert_eq!(c.unattached.len(), 1);
    assert_eq!(
        (c.unattached[0].entry, c.unattached[0].amount),
        (hand, cents(2_500))
    );
    assert_eq!(c.difference, Decimal::ZERO);
    b.l.reverse_entry(hand, d(2026, 1, 20), "Re-enter as an invoice")
        .unwrap();
    let c = check(&b.l, d(2026, 1, 31));
    assert!(c.unattached.is_empty(), "{c:?}");
    assert_eq!(c.balance, cents(10_000));
    // As of a day before the reversal, still there.
    assert_eq!(check(&b.l, d(2026, 1, 16)).unattached.len(), 1);
}

/// Scenario 8: receive_payment's deposit account (pay_bill's payer) must
/// be a bank, a card or a payable that holds no role.
#[test]
fn deposit_account_must_be_cash_like() {
    let mut b = books();
    let inv = b.invoice(d(2026, 1, 10), 10_000);
    let bill = b.bill(d(2026, 1, 10), 1_000);
    let (ar, ap, revenue) = (b.account("1200"), b.account("2000"), b.account("4000"));
    let before = b.l.clone();
    for account in [ar, ap, revenue, b.capital] {
        assert!(matches!(
            b.l.receive_payment(inv, d(2026, 1, 20), cents(100), account),
            Err(LedgerError::PaymentAccount { .. })
        ));
        assert!(matches!(
            b.l.pay_bill(bill, d(2026, 1, 20), cents(100), account),
            Err(LedgerError::PaymentAccount { .. })
        ));
    }
    assert!(b.l.differs_from(&before).is_empty());
    // A card is fine for a bill.
    b.l.pay_bill(bill, d(2026, 1, 20), cents(1_000), b.card)
        .unwrap();
}

/// A reversal of an entry a document owns is never dated before it
/// (B-9; P4b.3, "Dating rules").
#[test]
fn reversal_before_original_refused_for_documents() {
    let mut b = books();
    let inv = b.invoice(d(2026, 1, 10), 10_000);
    let pay =
        b.l.receive_payment(inv, d(2026, 1, 20), cents(100), b.bank)
            .unwrap();
    assert!(matches!(
        b.l.reverse_entry(pay, d(2026, 1, 19), "early"),
        Err(LedgerError::DocumentReversalBeforeOriginal { .. })
    ));
    assert!(matches!(
        b.l.void_document(DocRef::Invoice(inv), d(2026, 1, 9), "early"),
        Err(LedgerError::DocumentReversalBeforeOriginal { .. })
    ));
    // The same day is fine.
    b.l.reverse_entry(pay, d(2026, 1, 20), "same day").unwrap();
}

/// A payment's method tags its entry in the same step (D-40); its
/// reference is in the description; a bad one is refused.
#[test]
fn a_payment_carries_its_method_and_reference() {
    let mut b = books();
    let bill = b.bill(d(2026, 1, 10), 1_000);
    let card =
        b.l.add_instrument(
            ledger_core::InstrumentKind::BusinessCard,
            "Visa",
            Some("4242"),
            None,
            b.card,
        )
        .unwrap();
    let details = PaymentDetails {
        instrument: Some(card),
        reference: Some("ch_3Nx".into()),
    };
    let e =
        b.l.pay_bill_with(bill, d(2026, 1, 12), cents(1_000), b.card, &details)
            .unwrap();
    assert_eq!(b.l.entry_instrument(e), Some(card));
    assert_eq!(
        b.l.get_entry(e).unwrap().description(),
        format!("Payment on bill {bill} (ref ch_3Nx)")
    );
    let inv = b.invoice(d(2026, 1, 10), 1_000);
    let before = b.l.clone();
    for bad in [
        PaymentDetails {
            instrument: Some(99),
            reference: None,
        },
        PaymentDetails {
            instrument: None,
            reference: Some("two\nlines".into()),
        },
        PaymentDetails {
            instrument: None,
            reference: Some("x".repeat(201)),
        },
    ] {
        assert!(
            b.l.receive_payment_with(inv, d(2026, 1, 12), cents(100), b.bank, &bad)
                .is_err()
        );
    }
    assert!(b.l.differs_from(&before).is_empty());
}

/// The status rules (P4b.3, "Status as of D") on their own.
#[test]
fn status_follows_the_figures() {
    use ledger_core::arap::doc_status;
    let (t, z) = (cents(10_000), Decimal::ZERO);
    assert_eq!(doc_status(false, true, t, t, z), DocStatus::Void);
    assert_eq!(doc_status(false, true, t, z, z), DocStatus::Void);
    assert_eq!(doc_status(false, false, t, -t, z), DocStatus::Void);
    assert_eq!(doc_status(true, false, t, z, z), DocStatus::Unattached);
    assert_eq!(doc_status(true, true, t, -cents(1), z), DocStatus::Overpaid);
    assert_eq!(
        doc_status(true, true, t, z, cents(1)),
        DocStatus::WrittenOff
    );
    assert_eq!(doc_status(true, true, t, z, z), DocStatus::Paid);
    assert_eq!(doc_status(true, true, t, cents(1), z), DocStatus::Partial);
    assert_eq!(doc_status(true, true, t, t, z), DocStatus::Open);
    assert_eq!(doc_status(true, true, t, t + t, z), DocStatus::Open);
    let mut keys = Vec::new();
    for s in [
        DocStatus::Open,
        DocStatus::Partial,
        DocStatus::Paid,
        DocStatus::WrittenOff,
        DocStatus::Void,
        DocStatus::Overpaid,
        DocStatus::Unattached,
    ] {
        assert!(!s.key().is_empty() && !s.label().is_empty());
        keys.push(s.key());
    }
    keys.sort_unstable();
    keys.dedup();
    assert_eq!(keys.len(), 7, "each status its own key");
}

/// A document dated after the day asked about isn't issued yet.
#[test]
fn a_document_after_the_day_has_no_state() {
    let mut b = books();
    let inv = b.invoice(d(2026, 5, 1), 100);
    assert_eq!(b.l.invoice_state(inv, d(2026, 4, 30)), Ok(None));
    assert!(b.l.invoice_state(inv, d(2026, 5, 1)).unwrap().is_some());
    assert_eq!(
        b.l.invoice_state(99, d(2026, 5, 1)),
        Err(LedgerError::UnknownInvoice(99))
    );
    assert_eq!(
        b.l.bill_state(99, d(2026, 5, 1)),
        Err(LedgerError::UnknownBill(99))
    );
}

/// Replay tags a document's control line on the role's account as it
/// stood then: an invoice journaled before receivables moved to 1150
/// keeps its 1200 line; a payment after, its 1150 line.
#[test]
fn role_reassigned_mid_stream_tags_the_role_then() {
    let mut l = Ledger::new(Currency::USD);
    let bank = l
        .add_account("1000", "Bank", AccountType::Asset, None, false)
        .unwrap();
    let old = l
        .add_account(
            "1200",
            "Accounts Receivable",
            AccountType::Asset,
            None,
            false,
        )
        .unwrap();
    let new = l
        .add_account("1150", "Trade Receivables", AccountType::Asset, None, false)
        .unwrap();
    let revenue = l
        .add_account("4000", "Revenue", AccountType::Revenue, None, false)
        .unwrap();
    let c = l.add_customer("Fernhill Labs").unwrap();
    let entry = |l: &mut Ledger, dr, cr, n| {
        let e = l.create_entry(d(2026, 1, 5), "replayed");
        l.replay_add_line(e, dr, Side::Debit, usd(n)).unwrap();
        l.replay_add_line(e, cr, Side::Credit, usd(n)).unwrap();
        l.replay_post_entry(e).unwrap();
        e
    };
    let e1 = entry(&mut l, old, revenue, 10_000);
    l.replay_invoice(1, c, d(2026, 1, 5), d(2026, 2, 4), line("Work", 10_000), e1)
        .unwrap();
    // The role moves (as an earlier version could, with a hand entry
    // moving the balance), then the payment.
    let moved = entry(&mut l, new, old, 10_000);
    ledger_core::parties::replaying(|| l.assign_role("receivables", Some(new))).unwrap();
    let p = entry(&mut l, bank, new, 10_000);
    l.replay_invoice_payment(1, cents(10_000), p).unwrap();
    assert_eq!(l.line_doc(e1, 0).unwrap().doc, DocRef::Invoice(1));
    assert_eq!(l.line_doc(p, 1).unwrap().movement, Movement::Payment);
    assert_eq!(l.line_doc(moved, 0), None, "a hand line is no document's");
    let st = l.invoice_state(1, d(2026, 1, 31)).unwrap().unwrap();
    assert_eq!(
        (st.status, st.outstanding),
        (DocStatus::Paid, Decimal::ZERO)
    );
    for c in l.control_check(d(2026, 1, 31)).unwrap() {
        assert_eq!(c.difference, Decimal::ZERO, "{c:?}");
    }
}

/// Several payment events naming one entry tag its control lines in line
/// order (the ordinal rule, P4b.3), each at its own amount.
#[test]
fn several_payment_events_on_one_entry_tag_its_lines_in_order() {
    let mut l = Ledger::new(Currency::USD);
    let bank = l
        .add_account("1000", "Bank", AccountType::Asset, None, false)
        .unwrap();
    let ar = l
        .add_account(
            "1200",
            "Accounts Receivable",
            AccountType::Asset,
            None,
            false,
        )
        .unwrap();
    let revenue = l
        .add_account("4000", "Revenue", AccountType::Revenue, None, false)
        .unwrap();
    let c = l.add_customer("Fernhill Labs").unwrap();
    for id in [1, 2] {
        let e = l.create_entry(d(2026, 1, 5), "replayed");
        l.replay_add_line(e, ar, Side::Debit, usd(1_000 * id))
            .unwrap();
        l.replay_add_line(e, revenue, Side::Credit, usd(1_000 * id))
            .unwrap();
        l.replay_post_entry(e).unwrap();
        l.replay_invoice(
            id as u64,
            c,
            d(2026, 1, 5),
            d(2026, 2, 4),
            line("W", 1_000 * id),
            e,
        )
        .unwrap();
    }
    let p = l.create_entry(d(2026, 1, 9), "one deposit, two invoices");
    l.replay_add_line(p, bank, Side::Debit, usd(3_000)).unwrap();
    l.replay_add_line(p, ar, Side::Credit, usd(1_000)).unwrap();
    l.replay_add_line(p, ar, Side::Credit, usd(2_000)).unwrap();
    l.replay_post_entry(p).unwrap();
    l.replay_invoice_payment(1, cents(1_000), p).unwrap();
    l.replay_invoice_payment(2, cents(2_000), p).unwrap();
    assert_eq!(l.line_doc(p, 1).unwrap().doc, DocRef::Invoice(1));
    assert_eq!(l.line_doc(p, 2).unwrap().doc, DocRef::Invoice(2));
    for id in [1, 2] {
        let st = l.invoice_state(id, d(2026, 1, 31)).unwrap().unwrap();
        assert_eq!(st.status, DocStatus::Paid, "invoice {id}");
    }
    // Counted once each by the payments recorded.
    assert_eq!(l.payments_recorded(DocRef::Invoice(2)), Ok(cents(2_000)));
}

/// A void waits on the payments that stand, and names exactly those: a
/// bounced payment's reversal stands too, but re-instating the bounced
/// payment is never the remedy (review of S1, finding 1).
#[test]
fn standing_payments_never_names_a_reversal() {
    let mut b = books();
    let inv = b.invoice(d(2026, 1, 10), 10_000);
    let doc = DocRef::Invoice(inv);
    let p =
        b.l.receive_payment(inv, d(2026, 1, 12), cents(4_000), b.bank)
            .unwrap();
    let r = b.l.reverse_entry(p, d(2026, 1, 14), "Bounced").unwrap();
    let q =
        b.l.receive_payment(inv, d(2026, 1, 16), cents(4_000), b.bank)
            .unwrap();
    assert_eq!(b.l.standing_payments(doc), vec![q]);
    match b.l.void_document(doc, d(2026, 1, 20), "duplicate") {
        Err(LedgerError::DocumentBounds(why)) => {
            assert!(why.contains(&format!("entry {q}")), "{why}");
            assert!(!why.contains(&format!("entry {r}")), "{why}");
            assert!(!why.contains(&format!("entry {p}")), "{why}");
        }
        other => panic!("expected DocumentBounds, got {other:?}"),
    }
    // Two payments in one day, and each named once.
    let q2 =
        b.l.receive_payment(inv, d(2026, 1, 16), cents(1_000), b.bank)
            .unwrap();
    assert_eq!(b.l.standing_payments(doc), vec![q, q2]);
}

/// Replay tags a payment's line on the side a payment posts: an older
/// payment deposited into receivables itself (Dr 1200 / Cr 1200, plan
/// P4b.2 gap 5) pays its invoice with the credit, and the debit is no
/// document's (review of S1, finding 2).
#[test]
fn a_payment_into_the_control_account_tags_its_credit() {
    let mut l = Ledger::new(Currency::USD);
    let ar = l
        .add_account(
            "1200",
            "Accounts Receivable",
            AccountType::Asset,
            None,
            false,
        )
        .unwrap();
    let revenue = l
        .add_account("4000", "Revenue", AccountType::Revenue, None, false)
        .unwrap();
    let c = l.add_customer("Fernhill Labs").unwrap();
    let e = l.create_entry(d(2026, 1, 5), "Invoice 1");
    l.replay_add_line(e, ar, Side::Debit, usd(10_000)).unwrap();
    l.replay_add_line(e, revenue, Side::Credit, usd(10_000))
        .unwrap();
    l.replay_post_entry(e).unwrap();
    l.replay_invoice(1, c, d(2026, 1, 5), d(2026, 2, 4), line("W", 10_000), e)
        .unwrap();
    let p = l.create_entry(d(2026, 1, 9), "Payment on invoice 1");
    l.replay_add_line(p, ar, Side::Debit, usd(3_000)).unwrap();
    l.replay_add_line(p, ar, Side::Credit, usd(3_000)).unwrap();
    l.replay_post_entry(p).unwrap();
    l.replay_invoice_payment(1, cents(3_000), p).unwrap();
    assert_eq!(l.line_doc(p, 0), None, "the debit is no document's");
    assert_eq!(l.line_doc(p, 1).unwrap().movement, Movement::Payment);
    let st = l.invoice_state(1, d(2026, 1, 31)).unwrap().unwrap();
    assert_eq!(
        (st.paid, st.outstanding, st.status),
        (cents(3_000), cents(7_000), DocStatus::Partial)
    );
    let c = &l.control_check(d(2026, 1, 31)).unwrap()[0];
    assert_eq!(
        (c.balance, c.documents_total, c.difference),
        (cents(10_000), cents(7_000), Decimal::ZERO)
    );
    assert_eq!(c.unattached.len(), 1);
    assert_eq!(c.unattached[0].amount, cents(3_000));
}

/// The record's cached figures (what the MCP reads until it reads
/// doc_state) never call a void paid: it settled nothing and owes
/// nothing (review of S1, finding 3).
#[test]
fn a_void_never_reads_as_paid_in_the_record() {
    let mut b = books();
    let inv = b.invoice(d(2026, 1, 10), 10_000);
    b.l.void_document(DocRef::Invoice(inv), d(2026, 1, 11), "duplicate")
        .unwrap();
    let r = b.l.get_invoice(inv).unwrap();
    assert_eq!(r.amount_paid, Decimal::ZERO);
    assert_eq!(r.balance(), Ok(Decimal::ZERO));
    assert_eq!(r.status, ledger_core::DocumentStatus::Open);
    assert_eq!(
        b.state(DocRef::Invoice(inv), d(2026, 1, 31)).status,
        DocStatus::Void
    );
    // Paid in part, the rest owed: the record says so.
    let bill = b.bill(d(2026, 1, 10), 1_000);
    b.l.pay_bill(bill, d(2026, 1, 12), cents(400), b.bank)
        .unwrap();
    let r = b.l.get_bill(bill).unwrap();
    assert_eq!(
        (r.amount_paid, r.balance(), r.status),
        (
            cents(400),
            Ok(cents(600)),
            ledger_core::DocumentStatus::Partial
        )
    );
}

/// Receivables and payables move only to an account whose lines are all
/// documents', and never while their account holds a balance; the
/// refusal names no hand entry, which the guard refuses (review of S1:
/// first review's finding 4, second's finding 2). An account the role
/// moved off stays guarded while it holds documents' lines.
#[test]
fn moving_a_document_role_keeps_the_guard() {
    let mut b = books();
    let inv = b.invoice(d(2026, 1, 10), 10_000);
    let (ar, revenue) = (b.account("1200"), b.account("4000"));
    let fresh =
        b.l.add_account("1150", "Trade Receivables", AccountType::Asset, None, false)
            .unwrap();
    let err = b.l.assign_role("receivables", Some(fresh)).unwrap_err();
    assert!(
        matches!(err, LedgerError::DocumentRoleHoldsBalance { .. }),
        "{err:?}"
    );
    let text = err.to_string();
    assert!(!text.contains("journal entry"), "{text}");
    assert!(text.contains("hand entry"), "{text}");
    // A hand line on the account it would move to.
    let used =
        b.l.add_account("1160", "Old Receivables", AccountType::Asset, None, false)
            .unwrap();
    let e = b.l.try_create_entry(d(2026, 1, 3), "by hand").unwrap();
    b.l.add_line(e, used, Side::Debit, usd(500)).unwrap();
    b.l.add_line(e, revenue, Side::Credit, usd(500)).unwrap();
    b.l.post_entry(e).unwrap();
    b.l.receive_payment(inv, d(2026, 1, 12), cents(10_000), b.bank)
        .unwrap();
    match b.l.assign_role("receivables", Some(used)) {
        Err(LedgerError::RoleAccountHasHandLines { entry, code, .. }) => {
            assert_eq!((entry, code.as_str()), (e, "1160"));
        }
        other => panic!("expected RoleAccountHasHandLines, got {other:?}"),
    }
    // Replay takes what an earlier version journaled.
    let mut replayed = b.l.clone();
    ledger_core::parties::replaying(|| replayed.assign_role("receivables", Some(used))).unwrap();
    // Nothing owed now: the role moves to the fresh account, and 1200,
    // holding the invoice's lines, stays guarded.
    b.l.assign_role("receivables", Some(fresh)).unwrap();
    assert!(b.l.control_role_of(ar).is_some());
    let e = b.l.try_create_entry(d(2026, 1, 20), "by hand").unwrap();
    assert!(matches!(
        b.l.add_line(e, ar, Side::Debit, usd(100)),
        Err(LedgerError::UntaggedControlLine { .. })
    ));
    // A later reversal of the old payment lands on 1200, tagged, and
    // both accounts tie.
    let pay = b.l.standing_payments(DocRef::Invoice(inv))[0];
    b.l.reverse_entry(pay, d(2026, 1, 21), "Bounced").unwrap();
    b.ties(d(2026, 1, 31));
}

/// The guard locks only accounts whose balance is documents: a chart's
/// own 1200 Equipment, or a card at 2000, is anyone's while no document
/// posts there and the role is assigned elsewhere (second review of S1,
/// finding 1).
#[test]
fn a_chart_whose_1200_is_not_receivables() {
    let mut l = Ledger::new(Currency::USD);
    let equipment = l
        .add_account("1200", "Equipment", AccountType::Asset, None, false)
        .unwrap();
    let card = l
        .add_account("2000", "Visa", AccountType::Liability, None, false)
        .unwrap();
    let payables = l
        .add_account(
            "2010",
            "Accounts Payable",
            AccountType::Liability,
            None,
            false,
        )
        .unwrap();
    l.add_account("3000", "Capital", AccountType::Equity, None, false)
        .unwrap();
    // Buying equipment by hand, on the card.
    let e = l.try_create_entry(d(2026, 1, 3), "Laptop").unwrap();
    l.add_line(e, equipment, Side::Debit, usd(150_000)).unwrap();
    l.add_line(e, card, Side::Credit, usd(150_000)).unwrap();
    l.post_entry(e).unwrap();
    assert!(l.control_role_of(equipment).is_none());
    assert!(l.control_role_of(card).is_none());
    assert!(l.control_check(d(2026, 1, 31)).unwrap().is_empty());
    // Bills post to the assigned payables; the card at 2000 pays one.
    l.assign_role("payables", Some(payables)).unwrap();
    let v = l.add_vendor("Tobin Harlow Supply").unwrap();
    let bill = l
        .create_bill(v, d(2026, 1, 5), d(2026, 2, 4), line("Paper", 2_000))
        .unwrap();
    // Payables' own account may not pay it.
    assert!(matches!(
        l.pay_bill(bill, d(2026, 1, 6), cents(1), payables),
        Err(LedgerError::PaymentAccount { .. })
    ));
    l.pay_bill(bill, d(2026, 1, 6), cents(2_000), card).unwrap();
    assert_eq!(l.control_role_of(payables).map(|c| c.key), Some("payables"));
    let checks = l.control_check(d(2026, 1, 31)).unwrap();
    assert_eq!(checks.len(), 1);
    assert_eq!(checks[0].account, payables);
    assert_eq!(checks[0].difference, Decimal::ZERO);
}

/// Books an earlier version left out of bounds: a credit balance is
/// Overpaid (or Void, for a void under a payment that stands), never
/// aged as a negative receivable, and listed by the control check; a
/// document whose own line couldn't be tagged is Unattached, never Paid
/// (first review of S1, finding 6; second review's finding 9).
#[test]
fn out_of_bounds_documents_are_listed_not_aged() {
    let mut l = Ledger::new(Currency::USD);
    let bank = l
        .add_account("1000", "Bank", AccountType::Asset, None, false)
        .unwrap();
    let ar = l
        .add_account(
            "1200",
            "Accounts Receivable",
            AccountType::Asset,
            None,
            false,
        )
        .unwrap();
    let revenue = l
        .add_account("4000", "Revenue", AccountType::Revenue, None, false)
        .unwrap();
    let c = l.add_customer("Fernhill Labs").unwrap();
    let post = |l: &mut Ledger, on, lines: &[(AccountId, Side, i64)]| {
        let e = l.create_entry(on, "replayed");
        for &(a, side, n) in lines {
            l.replay_add_line(e, a, side, usd(n)).unwrap();
        }
        l.replay_post_entry(e).unwrap();
        e
    };
    ledger_core::parties::replaying(|| {
        // Invoice 1, paid twice over by two payment events.
        let e = post(
            &mut l,
            d(2026, 1, 5),
            &[(ar, Side::Debit, 10_000), (revenue, Side::Credit, 10_000)],
        );
        l.replay_invoice(1, c, d(2026, 1, 5), d(2026, 2, 4), line("W", 10_000), e)
            .unwrap();
        for _ in 0..2 {
            let p = post(
                &mut l,
                d(2026, 1, 9),
                &[(bank, Side::Debit, 6_000), (ar, Side::Credit, 6_000)],
            );
            l.replay_invoice_payment(1, cents(6_000), p).unwrap();
        }
        // Invoice 2, paid, then voided under the payment.
        let e2 = post(
            &mut l,
            d(2026, 1, 5),
            &[(ar, Side::Debit, 5_000), (revenue, Side::Credit, 5_000)],
        );
        l.replay_invoice(2, c, d(2026, 1, 5), d(2026, 2, 4), line("W", 5_000), e2)
            .unwrap();
        let p = post(
            &mut l,
            d(2026, 1, 9),
            &[(bank, Side::Debit, 5_000), (ar, Side::Credit, 5_000)],
        );
        l.replay_invoice_payment(2, cents(5_000), p).unwrap();
        l.replay_reverse_entry(e2, d(2026, 1, 10), "Void").unwrap();
        // Invoice 3, whose entry posted to revenue only (no line to tag).
        let e3 = post(
            &mut l,
            d(2026, 1, 5),
            &[(bank, Side::Debit, 700), (revenue, Side::Credit, 700)],
        );
        l.replay_invoice(3, c, d(2026, 1, 5), d(2026, 2, 4), line("W", 700), e3)
            .unwrap();
        // Invoice 4, owing 30.00, to show aging still counts the owed.
        let e4 = post(
            &mut l,
            d(2026, 1, 5),
            &[(ar, Side::Debit, 3_000), (revenue, Side::Credit, 3_000)],
        );
        l.replay_invoice(4, c, d(2026, 1, 5), d(2026, 2, 4), line("W", 3_000), e4)
            .unwrap();
    });
    let on = d(2026, 1, 31);
    let st = |id| l.invoice_state(id, on).unwrap().unwrap();
    assert_eq!(
        (st(1).status, st(1).outstanding),
        (DocStatus::Overpaid, -cents(2_000))
    );
    assert!(st(1).out_of_bounds());
    assert_eq!(
        (st(2).status, st(2).outstanding),
        (DocStatus::Void, -cents(5_000))
    );
    assert_eq!(st(3).status, DocStatus::Unattached);
    assert!(!st(4).out_of_bounds());
    let aged: Vec<_> = l.ar_aging(on).unwrap();
    assert_eq!(aged.len(), 1);
    assert_eq!(aged[0].total(), Ok(cents(3_000)), "{aged:?}");
    let check = &l.control_check(on).unwrap()[0];
    assert_eq!(check.difference, Decimal::ZERO);
    let listed: Vec<_> = check
        .attention
        .iter()
        .map(|a| (a.doc, a.status, a.outstanding))
        .collect();
    assert_eq!(
        listed,
        [
            (DocRef::Invoice(1), DocStatus::Overpaid, -cents(2_000)),
            (DocRef::Invoice(2), DocStatus::Void, -cents(5_000)),
            (DocRef::Invoice(3), DocStatus::Unattached, Decimal::ZERO),
        ]
    );
}

/// Reversing a legacy hand line takes it off; reversing that reversal
/// would put it back, and is refused: no command adds an untagged control
/// line (B-38 INVARIANT 1 and item 3; first review of S1).
#[test]
fn putting_back_a_removed_hand_line_is_refused() {
    let mut b = books();
    b.invoice(d(2026, 1, 10), 10_000);
    let (ar, revenue) = (b.account("1200"), b.account("4000"));
    let hand = ledger_core::parties::replaying(|| {
        let e = b.l.create_entry(d(2026, 1, 15), "Hand-booked sale");
        b.l.add_line(e, ar, Side::Debit, usd(2_500)).unwrap();
        b.l.add_line(e, revenue, Side::Credit, usd(2_500)).unwrap();
        b.l.post_entry(e).unwrap();
        e
    });
    let off = b.l.reverse_entry(hand, d(2026, 1, 20), "Re-enter").unwrap();
    let before = b.l.clone();
    assert!(matches!(
        b.l.reverse_entry(off, d(2026, 1, 21), "Put it back"),
        Err(LedgerError::UntaggedControlLine { .. })
    ));
    assert!(b.l.differs_from(&before).is_empty());
    // An earlier version's re-instatement replays, and a later reversal
    // of it takes the line off again.
    let back =
        b.l.replay_reverse_entry(off, d(2026, 1, 21), "Put back")
            .unwrap();
    b.l.reverse_entry(back, d(2026, 1, 22), "Off again")
        .unwrap();
    let c = &b.l.control_check(d(2026, 1, 31)).unwrap()[0];
    assert!(c.unattached.is_empty(), "{c:?}");
}

/// Plan P4b.3 ("Dating rules"): an invoice from a closed period is voided
/// in an open one. As of the closed period's end it is still Open; the
/// reversal's revenue lands in the open period, the closed one's figures
/// untouched (ASC 250: an immaterial error corrected when found).
#[test]
fn a_void_of_a_closed_periods_invoice_lands_in_the_open_period() {
    use ledger_core::reports::profit_and_loss_for_period;
    let mut b = books();
    let inv = b.invoice(d(2026, 1, 10), 10_000);
    let jan =
        b.l.create_period("2026-01", d(2026, 1, 1), d(2026, 1, 31))
            .unwrap();
    b.l.close_period(jan).unwrap();
    let doc = DocRef::Invoice(inv);
    assert!(matches!(
        b.l.void_document(doc, d(2026, 1, 20), "duplicate"),
        Err(LedgerError::ClosedPeriod(..))
    ));
    b.l.void_document(doc, d(2026, 2, 3), "duplicate").unwrap();
    assert_eq!(b.state(doc, d(2026, 1, 31)).status, DocStatus::Open);
    assert_eq!(b.state(doc, d(2026, 2, 3)).status, DocStatus::Void);
    let revenue = |from, to| {
        profit_and_loss_for_period(&b.l, from, to, None)
            .unwrap()
            .total_revenue
    };
    assert_eq!(revenue(d(2026, 1, 1), d(2026, 1, 31)), cents(10_000));
    assert_eq!(revenue(d(2026, 2, 1), d(2026, 2, 28)), -cents(10_000));
    let aged = b.l.ar_aging(d(2026, 1, 31)).unwrap();
    assert_eq!(aged[0].total(), Ok(cents(10_000)));
    assert!(b.l.ar_aging(d(2026, 2, 28)).unwrap().is_empty());
    b.ties(d(2026, 1, 31));
    b.ties(d(2026, 2, 28));
}

/// The payments an older version counts are every payment event, its
/// line tagged or not, so the document_standing gate counts what that
/// version would (second review of S1, finding 7); past exact arithmetic
/// they are refused, never a panic (finding 4).
#[test]
fn payments_recorded_counts_every_event() {
    let mut l = Ledger::new(Currency::USD);
    let bank = l
        .add_account("1000", "Bank", AccountType::Asset, None, false)
        .unwrap();
    let ar = l
        .add_account(
            "1200",
            "Accounts Receivable",
            AccountType::Asset,
            None,
            false,
        )
        .unwrap();
    let revenue = l
        .add_account("4000", "Revenue", AccountType::Revenue, None, false)
        .unwrap();
    let c = l.add_customer("Fernhill Labs").unwrap();
    let e = l.create_entry(d(2026, 1, 5), "Invoice 1");
    l.replay_add_line(e, ar, Side::Debit, usd(10_000)).unwrap();
    l.replay_add_line(e, revenue, Side::Credit, usd(10_000))
        .unwrap();
    l.replay_post_entry(e).unwrap();
    l.replay_invoice(1, c, d(2026, 1, 5), d(2026, 2, 4), line("W", 10_000), e)
        .unwrap();
    // A payment event whose amount matches no line: untagged, yet an
    // older version adds it.
    let p = l.create_entry(d(2026, 1, 9), "Payment");
    l.replay_add_line(p, bank, Side::Debit, usd(4_000)).unwrap();
    l.replay_add_line(p, ar, Side::Credit, usd(4_000)).unwrap();
    l.replay_post_entry(p).unwrap();
    l.replay_invoice_payment(1, cents(3_000), p).unwrap();
    assert_eq!(l.line_doc(p, 1), None);
    assert_eq!(l.payments_recorded(DocRef::Invoice(1)), Ok(cents(3_000)));
    let huge = Decimal::MAX / Decimal::TWO + Decimal::ONE;
    l.replay_invoice_payment(1, huge, p).unwrap();
    l.replay_invoice_payment(1, huge, p).unwrap();
    assert!(matches!(
        l.payments_recorded(DocRef::Invoice(1)),
        Err(LedgerError::TooLargeToCompute(_))
    ));
}

/// stands_on counts a reversal chain in order: an older journal's
/// re-reversal dated before the reversal it undoes counts only from that
/// reversal's date (second review of S1, finding 9).
#[test]
fn a_chain_counts_in_order() {
    let mut b = books();
    let e = b.l.try_create_entry(d(2026, 1, 1), "Capital in").unwrap();
    b.l.add_line(e, b.bank, Side::Debit, usd(100)).unwrap();
    b.l.add_line(e, b.capital, Side::Credit, usd(100)).unwrap();
    b.l.post_entry(e).unwrap();
    let r = b.l.reverse_entry(e, d(2026, 1, 20), "undo").unwrap();
    let rr = b.l.replay_reverse_entry(r, d(2026, 1, 10), "redo").unwrap();
    assert!(b.l.stands_on(e, d(2026, 1, 15)), "nothing reversed it yet");
    assert!(!b.l.stands_on(r, d(2026, 1, 15)));
    assert!(b.l.stands_on(e, d(2026, 1, 25)));
    assert!(!b.l.stands_on(r, d(2026, 1, 25)));
    assert!(b.l.stands_on(rr, d(2026, 1, 25)));
}

// ---------------------------------------------------------------------------
// Properties
// ---------------------------------------------------------------------------

/// One step of a random history.
#[derive(Clone, Debug)]
enum Op {
    /// An invoice (true) or a bill, on day `n`, for `c` cents.
    Create(bool, u8, i64),
    /// A payment on document `k` (mod the documents) on day `n`.
    Pay(usize, u8, i64),
    /// Reverse entry `k` (mod the posted entries) on day `n`.
    Reverse(usize, u8),
    /// Void document `k` on day `n`.
    Void(usize, u8),
    /// A hand line on receivables (true) or payables: always refused.
    Hand(bool, i64),
}

fn arb_op() -> impl Strategy<Value = Op> {
    prop_oneof![
        3 => (any::<bool>(), 0u8..60, 1i64..50_000).prop_map(|(i, n, c)| Op::Create(i, n, c)),
        4 => (0usize..8, 0u8..90, 1i64..60_000).prop_map(|(k, n, c)| Op::Pay(k, n, c)),
        3 => (0usize..40, 0u8..90).prop_map(|(k, n)| Op::Reverse(k, n)),
        1 => (0usize..8, 0u8..90).prop_map(|(k, n)| Op::Void(k, n)),
        1 => (any::<bool>(), 1i64..10_000).prop_map(|(r, c)| Op::Hand(r, c)),
    ]
}

/// Run `ops`, checking after every step that a refusal changed nothing;
/// returns the books and the documents made.
fn run(ops: &[Op]) -> (Books, Vec<DocRef>) {
    let mut b = books();
    let mut docs: Vec<DocRef> = Vec::new();
    let day = |n: u8| d(2026, 1, 1) + chrono::Days::new(u64::from(n));
    for op in ops {
        let before = b.l.clone();
        let result: Result<(), LedgerError> = match *op {
            Op::Create(true, n, c) => {
                b.l.create_invoice(b.customer, day(n), day(n), line("w", c))
                    .map(|i| docs.push(DocRef::Invoice(i)))
            }
            Op::Create(false, n, c) => {
                b.l.create_bill(b.vendor, day(n), day(n), line("w", c))
                    .map(|i| docs.push(DocRef::Bill(i)))
            }
            Op::Pay(_, _, _) if docs.is_empty() => Ok(()),
            Op::Pay(k, n, c) => match docs[k % docs.len()] {
                DocRef::Invoice(i) => b.l.receive_payment(i, day(n), cents(c), b.bank).map(|_| ()),
                DocRef::Bill(i) => b.l.pay_bill(i, day(n), cents(c), b.bank).map(|_| ()),
            },
            Op::Reverse(k, n) => {
                let posted: Vec<u64> = b.l.posted_entries().map(|e| e.id()).collect();
                match posted.get(k % posted.len().max(1)) {
                    Some(&e) => b.l.reverse_entry(e, day(n), "undo").map(|_| ()),
                    None => Ok(()),
                }
            }
            Op::Void(_, _) if docs.is_empty() => Ok(()),
            Op::Void(k, n) => {
                b.l.void_document(docs[k % docs.len()], day(n), "void")
                    .map(|_| ())
            }
            Op::Hand(receivables, c) => {
                let code = if receivables { "1200" } else { "2000" };
                match b.l.chart().find_by_code(code).map(|a| a.id) {
                    Some(a) => {
                        let e = b.l.try_create_entry(day(0), "hand").unwrap();
                        let r = b.l.add_line(e, a, Side::Debit, usd(c));
                        assert!(
                            matches!(r, Err(LedgerError::UntaggedControlLine { .. })),
                            "{r:?}"
                        );
                        b.l = before.clone();
                        Err(LedgerError::EmptyDocumentLines)
                    }
                    None => Ok(()),
                }
            }
        };
        if result.is_err() {
            assert!(
                b.l.differs_from(&before).is_empty(),
                "{op:?} refused, yet changed the books"
            );
        }
    }
    (b, docs)
}

/// Every day a figure could change on, and the end.
fn days(b: &Books) -> Vec<NaiveDate> {
    let mut v: Vec<NaiveDate> = b.l.posted_entries().map(|e| e.date()).collect();
    v.push(d(2026, 12, 31));
    v.sort();
    v.dedup();
    v
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// B-38 INVARIANTS 1, 2 and 6 over random histories of creates,
    /// payments, voids, reversals of any entry and refused hand lines: at
    /// every date the control accounts equal their documents (no
    /// difference, nothing unattached: no command adds a hand line), each
    /// document owes between nothing and its total, its own status agrees
    /// with its figures, and every refusal left the books unchanged.
    #[test]
    fn subledger_equals_control(ops in prop::collection::vec(arb_op(), 0..40)) {
        let (b, docs) = run(&ops);
        for on in days(&b) {
            let checks = b.l.control_check(on).unwrap();
            for c in &checks {
                prop_assert_eq!(c.difference, Decimal::ZERO);
                prop_assert!(c.unattached.is_empty(), "{:?}", c);
            }
            for doc in &docs {
                let Some(st) = b.l.doc_state(*doc, on).unwrap() else { continue };
                prop_assert!(st.outstanding >= Decimal::ZERO && st.outstanding <= st.total,
                    "{doc} on {on}: {st:?}");
                prop_assert_eq!(st.outstanding, st.total * Decimal::from(
                    u8::from(st.status != DocStatus::Void)) - st.paid);
            }
            // The control accounts' balances are the documents' sums.
            let owed = |inv: bool| -> Decimal {
                docs.iter()
                    .filter(|d| matches!(d, DocRef::Invoice(_)) == inv)
                    .filter_map(|d| b.l.doc_state(*d, on).unwrap())
                    .map(|s| s.outstanding)
                    .sum()
            };
            for c in &checks {
                prop_assert_eq!(c.balance, owed(c.role == "receivables"));
            }
        }
    }

    /// B-38 INVARIANT 5: the aging as of a day partitions what the
    /// documents owe then (in books these commands write, none owes less
    /// than nothing). INVARIANT 4 (Store, replay and Snapshot agree) is
    /// ledger-store's `status_is_a_function_of_journal_and_date`.
    #[test]
    fn aging_partitions_what_documents_owe(ops in prop::collection::vec(arb_op(), 0..30)) {
        let (a, docs) = run(&ops);
        for on in days(&a) {
            let (mut ar, mut ap) = (Decimal::ZERO, Decimal::ZERO);
            for doc in &docs {
                let Some(st) = a.l.doc_state(*doc, on).unwrap() else { continue };
                match doc {
                    DocRef::Invoice(_) => ar += st.outstanding,
                    DocRef::Bill(_) => ap += st.outstanding,
                }
            }
            let aged = |rows: Vec<ledger_core::AgingRow>| -> Decimal {
                rows.iter().map(|r| r.total().unwrap()).sum()
            };
            prop_assert_eq!(aged(a.l.ar_aging(on).unwrap()), ar);
            prop_assert_eq!(aged(a.l.ap_aging(on).unwrap()), ap);
        }
    }

    /// B-38 INVARIANT 3: reversing a tagged entry moves its documents by
    /// exactly the opposite of the entry's own tagged lines from the
    /// reversal's day (read from the entry reversed, not from the mirror,
    /// so a mirror that lost its tag is caught); reversing that reversal
    /// restores them. Reversing a payment that stands is always accepted
    /// (P4b.3, Bounds), so the property is never vacuous on payments.
    #[test]
    fn reversal_twice_restores_state(ops in prop::collection::vec(arb_op(), 1..30), k in 0usize..40) {
        let (mut b, docs) = run(&ops);
        let posted: Vec<u64> = b.l.posted_entries().map(|e| e.id()).collect();
        prop_assume!(!posted.is_empty());
        let e = posted[k % posted.len()];
        let (earlier, later) = (d(2026, 10, 31), d(2026, 11, 1));
        let state = |b: &Books, on| -> Vec<_> {
            docs.iter().map(|doc| b.l.doc_state(*doc, on).unwrap()).collect()
        };
        let (before_earlier, before) = (state(&b, earlier), state(&b, later));
        let original: Vec<Decimal> = docs
            .iter()
            .map(|doc| b.l.doc_lines(*doc).iter().filter(|l| l.entry == e).map(|l| l.amount).sum())
            .collect();
        let unreversed = b.l.reversal_chain(e).last() == Some(&e);
        let a_payment = b.l.get_entry(e).is_some_and(|x| x.reversal_of().is_none())
            && docs.iter().any(|doc| b.l.doc_lines(*doc).iter()
                .any(|l| l.entry == e && l.movement == Movement::Payment));
        let reversed = b.l.reverse_entry(e, later, "undo");
        if a_payment && unreversed {
            prop_assert!(reversed.is_ok(), "reversing payment {e}: {reversed:?}");
        }
        let Ok(r) = reversed else { return Ok(()) };
        let moved = state(&b, later);
        // Nothing moves before the reversal's day.
        prop_assert_eq!(state(&b, earlier), before_earlier);
        for i in 0..docs.len() {
            let (Some(x), Some(y)) = (before[i], moved[i]) else { continue };
            prop_assert_eq!(y.outstanding - x.outstanding, -original[i]);
        }
        if b.l.reverse_entry(r, d(2026, 11, 2), "redo").is_ok() {
            for (i, doc) in docs.iter().enumerate() {
                let now = b.l.doc_state(*doc, d(2026, 11, 2)).unwrap();
                prop_assert_eq!(now.map(|s| (s.outstanding, s.status)), before[i].map(|s| (s.outstanding, s.status)));
            }
        }
    }
}
