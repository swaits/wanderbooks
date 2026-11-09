//! Replaying document events the way a journaling store does (plan P14
//! unit V-D2). The store rebuilds a payment's entry from its own entry
//! events and then tells the ledger which of its lines is the payment, the
//! discount, the write-off; the ledger tags the line when it can and
//! leaves it untagged, never refusing, when it cannot. The core's own suite
//! reached none of this (only the store's replay tests did).
//!
//! ```text
//! GOAL: books written under earlier rules, or by a newer version's events,
//!       open and tie out: a document's payment is read from the line an
//!       event names, the control check lists what it could not match.
//! SOURCE: BS-entry-4 (replay takes what was journaled); BS-arap-6
//!         INVARIANT 3 (the posting rows' line indices are stable);
//!         BS-arap-2 (a document's figures are read from its tagged lines,
//!         and the control account ties to them plus the untagged lines).
//! ```

use chrono::NaiveDate;
use ledger_core::{
    AccountId, AccountType, Currency, DocRef, DocStatus, DocumentLine, EntryId, Ledger,
    LedgerError, Money, Side,
};
use rust_decimal::Decimal;
use std::str::FromStr;

fn d(m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, m, day).unwrap()
}

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

fn usd(s: &str) -> Money {
    Money::new(dec(s), Currency::USD)
}

struct Books {
    l: Ledger,
    bank: AccountId,
    allowance: AccountId,
    invoice: u64,
    bill: u64,
}

/// A 500.00 invoice and a 300.00 bill, dated March 1.
fn books() -> Books {
    let mut l = Ledger::new(Currency::USD);
    let bank = l
        .add_account("1000", "Bank", AccountType::Asset, None, false)
        .unwrap();
    let allowance = l
        .add_account(
            "4900",
            "Sales allowances",
            AccountType::Revenue,
            None,
            false,
        )
        .unwrap();
    let customer = l.add_customer("Fernhill Labs").unwrap();
    let vendor = l.add_vendor("Tobin Harlow Supply").unwrap();
    let line = |amount: &str| {
        vec![DocumentLine {
            description: "Work".into(),
            amount: dec(amount),
            class_id: None,
            account: None,
            quantity: None,
            unit_price: None,
        }]
    };
    let invoice = l
        .create_invoice(customer, d(3, 1), d(3, 31), line("500.00"))
        .unwrap();
    let bill = l
        .create_bill(vendor, d(3, 1), d(3, 31), line("300.00"))
        .unwrap();
    Books {
        l,
        bank,
        allowance,
        invoice,
        bill,
    }
}

impl Books {
    fn code(&self, code: &str) -> AccountId {
        self.l.chart().find_by_code(code).unwrap().id
    }

    /// An entry as a journal would rebuild it: lines pushed as recorded,
    /// then posted by replay (no control guard, no bounds).
    fn rebuilt(&mut self, on: NaiveDate, lines: &[(AccountId, Side, &str)]) -> EntryId {
        let e = self.l.create_entry(on, "replayed");
        for &(account, side, amount) in lines {
            self.l
                .replay_add_line(e, account, side, usd(amount))
                .unwrap();
        }
        self.l.replay_post_entry(e).unwrap();
        e
    }

    fn unattached(&self, on: NaiveDate) -> Vec<(EntryId, usize)> {
        self.l
            .control_check(on)
            .unwrap()
            .iter()
            .flat_map(|c| c.unattached.iter().map(|u| (u.entry, u.line)))
            .collect()
    }

    /// Every control account (receivables and payables) ties out.
    fn ties(&self, on: NaiveDate) -> bool {
        let checks = self.l.control_check(on).unwrap();
        !checks.is_empty() && checks.iter().all(|c| c.difference.is_zero())
    }
}

/// BS-arap-6 INVARIANT 3: a payment event that names the line (index 1, the
/// receivables credit) tags exactly that line. The invoice then reads 40.00
/// paid and 460.00 owed, and the control account ties out with nothing
/// untagged.
#[test]
fn a_payment_event_tags_the_line_it_names() {
    let mut b = books();
    let ar = b.code("1200");
    let e = b.rebuilt(
        d(3, 10),
        &[(b.bank, Side::Debit, "40.00"), (ar, Side::Credit, "40.00")],
    );
    b.l.replay_invoice_payment_at(b.invoice, dec("40.00"), e, Some(1), Decimal::ZERO, None)
        .unwrap();
    let st =
        b.l.doc_state(DocRef::Invoice(b.invoice), d(3, 31))
            .unwrap()
            .unwrap();
    assert_eq!(
        (st.paid, st.outstanding, st.status),
        (dec("40.00"), dec("460.00"), DocStatus::Partial)
    );
    assert!(b.unattached(d(3, 31)).is_empty());
    assert!(b.ties(d(3, 31)));
    assert_eq!(b.l.account_balance(ar).unwrap(), dec("460.00"));
    // The payment's entry is with the invoice's customer, like the invoice's own.
    let own = b.l.get_invoice(b.invoice).unwrap().entry_id;
    assert!(b.l.entry_party(e).is_some());
    assert_eq!(b.l.entry_party(e), b.l.entry_party(own));
}

/// BS-entry-4: an event that names a line the entry cannot answer to (the
/// wrong account, the wrong amount, no such line) is not refused: the line
/// stays untagged and the control check lists it, so the human sees it and
/// the control account still ties (balance = documents + untagged).
#[test]
fn a_payment_event_that_names_a_line_that_does_not_match_leaves_it_untagged() {
    let mut b = books();
    let ar = b.code("1200");
    for (named, amount) in [(Some(0), "40.00"), (Some(1), "41.00"), (Some(9), "40.00")] {
        let e = b.rebuilt(
            d(3, 10),
            &[(b.bank, Side::Debit, "40.00"), (ar, Side::Credit, "40.00")],
        );
        b.l.replay_invoice_payment_at(b.invoice, dec(amount), e, named, Decimal::ZERO, None)
            .unwrap();
        assert!(
            b.unattached(d(3, 31)).contains(&(e, 1)),
            "{named:?} {amount}: the receivables line is listed, not tagged"
        );
        assert!(b.ties(d(3, 31)));
    }
    // A named line already tagged is not tagged twice.
    let e = b.rebuilt(
        d(3, 12),
        &[(b.bank, Side::Debit, "10.00"), (ar, Side::Credit, "10.00")],
    );
    b.l.replay_invoice_payment_at(b.invoice, dec("10.00"), e, Some(1), Decimal::ZERO, None)
        .unwrap();
    b.l.replay_invoice_payment_at(b.invoice, dec("10.00"), e, Some(1), Decimal::ZERO, None)
        .unwrap();
    assert!(!b.unattached(d(3, 31)).contains(&(e, 1)));
}

/// BS-arap-6 INVARIANT 3: a payment with an early-payment discount posts a
/// payment line and a discount line against the same control account; the
/// event names both. A 500.00 invoice settled with 490.00 in cash and
/// 10.00 of discount reads paid 490.00, discounted 10.00, nothing owed.
#[test]
fn a_payment_event_names_its_payment_line_and_its_discount_line() {
    let mut b = books();
    let ar = b.code("1200");
    let e = b.rebuilt(
        d(3, 10),
        &[
            (b.bank, Side::Debit, "490.00"),
            (b.allowance, Side::Debit, "10.00"),
            (ar, Side::Credit, "490.00"),
            (ar, Side::Credit, "10.00"),
        ],
    );
    b.l.replay_invoice_payment_at(b.invoice, dec("490.00"), e, Some(2), dec("10.00"), Some(3))
        .unwrap();
    let st =
        b.l.doc_state(DocRef::Invoice(b.invoice), d(3, 31))
            .unwrap()
            .unwrap();
    assert_eq!(
        (st.paid, st.discounted, st.outstanding, st.status),
        (dec("490.00"), dec("10.00"), Decimal::ZERO, DocStatus::Paid)
    );
    assert!(b.unattached(d(3, 31)).is_empty());
    assert_eq!(b.l.account_balance(ar).unwrap(), Decimal::ZERO);
    // A discount named with no line to tag is just not tagged.
    let e2 = b.rebuilt(
        d(3, 11),
        &[(b.bank, Side::Debit, "5.00"), (ar, Side::Credit, "5.00")],
    );
    b.l.replay_invoice_payment_at(b.invoice, dec("5.00"), e2, Some(1), dec("2.00"), None)
        .unwrap();
}

/// BS-arap-6: a bill's payment is the mirror (a debit to payables), and the
/// event that names the wrong side leaves the line untagged.
#[test]
fn a_bill_payment_event_is_the_mirror() {
    let mut b = books();
    let ap = b.code("2000");
    let e = b.rebuilt(
        d(3, 10),
        &[
            (ap, Side::Debit, "100.00"),
            (b.bank, Side::Credit, "100.00"),
        ],
    );
    b.l.replay_bill_payment_at(b.bill, dec("100.00"), e, Some(0), Decimal::ZERO, None)
        .unwrap();
    let st =
        b.l.doc_state(DocRef::Bill(b.bill), d(3, 31))
            .unwrap()
            .unwrap();
    assert_eq!((st.paid, st.outstanding), (dec("100.00"), dec("200.00")));
    assert!(b.unattached(d(3, 31)).is_empty());

    // Payables credited instead (the side a bill's own line takes): not a payment line.
    let wrong = b.rebuilt(
        d(3, 11),
        &[(b.bank, Side::Debit, "50.00"), (ap, Side::Credit, "50.00")],
    );
    b.l.replay_bill_payment_at(b.bill, dec("50.00"), wrong, Some(1), Decimal::ZERO, None)
        .unwrap();
    assert!(b.unattached(d(3, 31)).contains(&(wrong, 1)));

    // The old form (no line named) tags the first untagged line of that amount.
    let old = b.rebuilt(
        d(3, 12),
        &[(ap, Side::Debit, "25.00"), (b.bank, Side::Credit, "25.00")],
    );
    b.l.replay_bill_payment(b.bill, dec("25.00"), old).unwrap();
    assert!(!b.unattached(d(3, 31)).contains(&(old, 0)));

    // A bill's discount line, named, is tagged as a discount.
    let ap2 = b.rebuilt(
        d(3, 13),
        &[
            (ap, Side::Debit, "20.00"),
            (ap, Side::Debit, "5.00"),
            (b.bank, Side::Credit, "20.00"),
            (b.allowance, Side::Credit, "5.00"),
        ],
    );
    b.l.replay_bill_payment_at(b.bill, dec("20.00"), ap2, Some(0), dec("5.00"), Some(1))
        .unwrap();
    let st =
        b.l.doc_state(DocRef::Bill(b.bill), d(3, 31))
            .unwrap()
            .unwrap();
    assert_eq!(st.discounted, dec("5.00"));
}

/// BS-entry-4: replay of a payment or a write-off for a document the books
/// do not have is the one thing it refuses, naming it; the ledger is
/// unchanged. The write-off tags the receivables credit of the amount.
#[test]
fn replaying_for_an_unknown_document_is_the_one_refusal_and_a_write_off_tags_its_line() {
    let mut b = books();
    let ar = b.code("1200");
    let e = b.rebuilt(
        d(3, 10),
        &[(b.bank, Side::Debit, "5.00"), (ar, Side::Credit, "5.00")],
    );
    let before = b.l.clone();
    assert_eq!(
        b.l.replay_invoice_payment_at(77, dec("5.00"), e, Some(1), Decimal::ZERO, None)
            .unwrap_err(),
        LedgerError::UnknownInvoice(77)
    );
    assert_eq!(
        b.l.replay_bill_payment_at(88, dec("5.00"), e, Some(0), Decimal::ZERO, None)
            .unwrap_err(),
        LedgerError::UnknownBill(88)
    );
    assert_eq!(
        b.l.replay_invoice_write_off(77, dec("5.00"), e)
            .unwrap_err(),
        LedgerError::UnknownInvoice(77)
    );
    assert_eq!(b.l, before);

    let bad_debt =
        b.l.add_account("6990", "Bad debt", AccountType::Expense, None, false)
            .unwrap();
    let w = b.rebuilt(
        d(3, 20),
        &[
            (bad_debt, Side::Debit, "60.00"),
            (ar, Side::Credit, "60.00"),
        ],
    );
    b.l.replay_invoice_write_off(b.invoice, dec("60.00"), w)
        .unwrap();
    let st =
        b.l.doc_state(DocRef::Invoice(b.invoice), d(3, 31))
            .unwrap()
            .unwrap();
    assert_eq!(
        (st.written_off, st.outstanding),
        (dec("60.00"), dec("440.00"))
    );
    assert!(!b.unattached(d(3, 31)).contains(&(w, 1)));
}

/// BS-entry-4: a draft's state is restored on a draft that exists; one
/// naming an entry since posted is refused like the command; a foreign
/// line as journaled is restored in the base amount booked, and a foreign
/// line in the wrong base currency is refused.
#[test]
fn a_draft_state_and_a_foreign_line_replay_as_journaled() {
    let mut b = books();
    let e = b.l.create_entry(d(3, 5), "Draft");
    b.l.replay_set_draft_state(
        e,
        ledger_core::DraftState::AwaitingEvidence {
            note: "Figma invoice due".into(),
        },
    )
    .unwrap();
    assert!(matches!(
        b.l.get_entry(e).unwrap().draft_state(),
        ledger_core::DraftState::AwaitingEvidence { note } if note == "Figma invoice due"
    ));

    b.l.replay_add_line_fx(
        e,
        b.bank,
        Side::Debit,
        usd("108.35"),
        Currency::EUR,
        dec("100.00"),
    )
    .unwrap();
    let line = &b.l.get_entry(e).unwrap().lines()[0];
    assert_eq!(
        (line.amount.amount, line.foreign_amount),
        (dec("108.35"), Some(dec("100.00")))
    );
    let eur = Money::new(dec("1.00"), Currency::EUR);
    assert!(matches!(
        b.l.replay_add_line_fx(e, b.bank, Side::Debit, eur, Currency::EUR, dec("1.00")),
        Err(LedgerError::CurrencyMismatch { .. })
    ));

    // Posted: the command's refusal stands for replay.
    let posted = b.rebuilt(
        d(3, 6),
        &[
            (b.bank, Side::Debit, "1.00"),
            (b.allowance, Side::Credit, "1.00"),
        ],
    );
    assert!(
        b.l.replay_set_draft_state(posted, ledger_core::DraftState::Building)
            .is_err()
    );
}
