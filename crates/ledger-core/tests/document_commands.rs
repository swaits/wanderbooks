//! The document commands refuse what their rules refuse, and a refusal
//! changes nothing (plan P14 unit V-D2): `mark_sent`, `settle`,
//! `write_off` and the aging list. Coverage found `mark_sent_steps` wholly
//! unreached from the core's own suite and `settle`'s and `write_off`'s
//! refusal arms thin (the store's replay tests are their other callers).
//!
//! ```text
//! GOAL: an invoice's send, a settlement and a write-off are recorded only
//!       when the books can stand them; each refusal says why and leaves the
//!       books as they were; each success moves exactly the figures its
//!       rule says.
//! SOURCE: BS-arap-2 (a document keeps no figure of its own); BS-arap-5
//!         (the direct write-off); BS-arap-6 (settlement); BS-ledger-5,
//!         BS-ledger-6 (reversal and void); BS-period-2 (a closed period
//!         takes no postings); BS-format-1 (the invoice.sent line).
//! ```
//!
//! Every test compares the whole ledger to a clone taken before a refused
//! command (`Ledger: PartialEq`) and shows the numbers a successful one
//! moves.

use chrono::NaiveDate;
use ledger_core::arap::{Application, Fee, InvoiceFields, SendKind, SentVia, SettleParams};
use ledger_core::settings::{Change, Source};
use ledger_core::{
    AccountId, AccountType, Currency, DocRef, DocStatus, DocumentLine, Ledger, LedgerError, Sent,
};
use rust_decimal::Decimal;
use serde_json::json;
use std::str::FromStr;

fn d(m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, m, day).unwrap()
}

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

fn lines(amount: &str) -> Vec<DocumentLine> {
    vec![DocumentLine {
        description: "Work".into(),
        amount: dec(amount),
        class_id: None,
        account: None,
        quantity: None,
        unit_price: None,
    }]
}

struct Books {
    l: Ledger,
    bank: AccountId,
    revenue: AccountId,
    fees: AccountId,
    customer: u64,
    other_customer: u64,
    vendor: u64,
}

fn books() -> Books {
    let mut l = Ledger::new(Currency::USD);
    let mut a = |code: &str, name: &str, t| l.add_account(code, name, t, None, false).unwrap();
    let bank = a("1000", "Bank", AccountType::Asset);
    let revenue = a("4010", "Consulting", AccountType::Revenue);
    let fees = a("6950", "Card fees", AccountType::Expense);
    let customer = l.add_customer("Fernhill Labs").unwrap();
    let other_customer = l.add_customer("Birchwood").unwrap();
    let vendor = l.add_vendor("Tobin Harlow Supply").unwrap();
    Books {
        l,
        bank,
        revenue,
        fees,
        customer,
        other_customer,
        vendor,
    }
}

impl Books {
    fn invoice(&mut self, on: NaiveDate, amount: &str) -> u64 {
        self.l
            .create_invoice(self.customer, on, on + chrono::Days::new(30), lines(amount))
            .unwrap()
    }
    fn bill(&mut self, on: NaiveDate, amount: &str) -> u64 {
        self.l
            .create_bill(self.vendor, on, on + chrono::Days::new(30), lines(amount))
            .unwrap()
    }
    fn balance(&self, code: &str) -> Decimal {
        match self.l.chart().find_by_code(code) {
            Some(a) => self.l.account_balance(a.id).unwrap(),
            None => Decimal::ZERO,
        }
    }
}

/// Run `f`, which must be refused, and the ledger must be as it was.
fn refused<T: std::fmt::Debug>(
    l: &mut Ledger,
    f: impl FnOnce(&mut Ledger) -> Result<T, LedgerError>,
) -> LedgerError {
    let before = l.clone();
    let e = f(l).expect_err("the command should be refused");
    assert_eq!(*l, before, "a refusal changes nothing");
    e
}

fn sent(on: NaiveDate, to: &[&str]) -> Sent {
    Sent {
        on,
        by: "Steve".into(),
        via: SentVia::Email,
        to: to.iter().map(|s| s.to_string()).collect(),
        kind: SendKind::Original,
        reference: None,
        note: None,
    }
}

// ---------------------------------------------------------------- mark_sent

/// BS-arap-2 / mark_sent INVARIANT 4: a send moves no money. The invoice's
/// state is the same on every date before and after, the trial balance is
/// the same, and the send is recorded in order.
#[test]
fn a_send_is_a_fact_of_the_invoices_life_and_moves_no_figure() {
    let mut b = books();
    let inv = b.invoice(d(3, 1), "500.00");
    let doc = DocRef::Invoice(inv);
    let dates = [d(3, 1), d(3, 15), d(4, 15), d(12, 31)];
    let states = |l: &Ledger| -> Vec<_> {
        dates
            .iter()
            .map(|&x| l.doc_state(doc, x).unwrap())
            .collect()
    };
    let (before_states, before_ar, before_bank) =
        (states(&b.l), b.balance("1200"), b.balance("1000"));
    assert_eq!(before_ar, dec("500.00"));

    let mut first = sent(d(3, 2), &["ap@fernhill.example"]);
    first.reference = Some("<msg-1@mail>".into());
    b.l.mark_sent(inv, first.clone()).unwrap();
    let mut reminder = sent(d(4, 5), &["ap@fernhill.example", "ceo@fernhill.example"]);
    reminder.kind = SendKind::Reminder;
    reminder.note = Some("Second notice".into());
    b.l.mark_sent(inv, reminder.clone()).unwrap();

    assert_eq!(b.l.get_invoice(inv).unwrap().sends, vec![first, reminder]);
    assert_eq!(states(&b.l), before_states, "no figure moved on any date");
    // Receivables still owe the invoice, and the bank is where it was.
    assert_eq!(b.balance("1200"), dec("500.00"));
    assert_eq!(b.balance("1200"), before_ar);
    assert_eq!(b.balance("1000"), before_bank);
}

/// mark_sent INVARIANT 1 and SCENARIO 1: the invoice must exist and the
/// send must not be dated before it; each refusal changes nothing.
#[test]
fn a_send_needs_an_invoice_and_a_date_no_earlier_than_it() {
    let mut b = books();
    let inv = b.invoice(d(3, 10), "500.00");
    let e = refused(&mut b.l, |l| {
        l.mark_sent(99, sent(d(3, 11), &["a@b.example"]))
    });
    assert_eq!(e, LedgerError::UnknownInvoice(99));
    let e = refused(&mut b.l, |l| {
        l.mark_sent(inv, sent(d(3, 9), &["a@b.example"]))
    });
    assert!(
        matches!(&e, LedgerError::InvalidSend(m)
        if m.contains("is dated 2026-03-10") && m.contains("on 2026-03-09")),
        "{e:?}"
    );
    // The same day is fine.
    b.l.mark_sent(inv, sent(d(3, 10), &["a@b.example"]))
        .unwrap();
}

/// mark_sent INVARIANT 2: a void invoice is never sent (DocumentVoid) but
/// is judged on the day of the send: a void LATER than the send does not
/// un-send it, a void BEFORE it does. Open, partial, paid and written-off
/// invoices all may be sent (a receipt copy, a reminder).
#[test]
fn a_void_invoice_is_not_sent_but_the_day_of_the_send_decides() {
    let mut b = books();
    let voided = b.invoice(d(3, 1), "100.00");
    b.l.void_document(DocRef::Invoice(voided), d(3, 20), "duplicate")
        .unwrap();
    // Sent before it was voided: allowed. After: refused.
    b.l.mark_sent(voided, sent(d(3, 10), &["a@b.example"]))
        .unwrap();
    let e = refused(&mut b.l, |l| {
        l.mark_sent(voided, sent(d(3, 25), &["a@b.example"]))
    });
    assert_eq!(e, LedgerError::DocumentVoid(voided));

    // Paid, partly paid and written off are all sendable.
    let paid = b.invoice(d(4, 1), "100.00");
    b.l.receive_payment(paid, d(4, 5), dec("100.00"), b.bank)
        .unwrap();
    let partial = b.invoice(d(4, 1), "100.00");
    b.l.receive_payment(partial, d(4, 5), dec("40.00"), b.bank)
        .unwrap();
    let written_off = b.invoice(d(4, 1), "100.00");
    b.l.write_off(written_off, d(4, 6), dec("100.00"), "uncollectible")
        .unwrap();
    for (inv, status) in [
        (paid, DocStatus::Paid),
        (partial, DocStatus::Partial),
        (written_off, DocStatus::WrittenOff),
    ] {
        assert_eq!(
            b.l.doc_state(DocRef::Invoice(inv), d(4, 10))
                .unwrap()
                .unwrap()
                .status,
            status
        );
        b.l.mark_sent(inv, sent(d(4, 10), &["a@b.example"]))
            .unwrap();
    }
}

/// mark_sent INVARIANT 3: at least one recipient, and every recipient, the
/// reference and the note pass the party text rule (no full TIN, no
/// control character, at most 500 characters).
#[test]
fn a_send_needs_a_recipient_and_clean_text() {
    let mut b = books();
    let inv = b.invoice(d(3, 1), "500.00");
    let e = refused(&mut b.l, |l| l.mark_sent(inv, sent(d(3, 2), &[])));
    assert!(
        matches!(&e, LedgerError::InvalidSend(m) if m.contains("needs at least one recipient")),
        "{e:?}"
    );

    let e = refused(&mut b.l, |l| {
        l.mark_sent(inv, sent(d(3, 2), &["ok@x.example", "SSN 123-45-6789"]))
    });
    assert!(
        matches!(&e, LedgerError::InvalidSend(m) if m.contains("recipient") && m.contains("taxpayer identification")),
        "{e:?}"
    );
    let e = refused(&mut b.l, |l| {
        l.mark_sent(inv, sent(d(3, 2), &["a\u{7}b@x.example"]))
    });
    assert!(
        matches!(&e, LedgerError::InvalidSend(m) if m.contains("control characters")),
        "{e:?}"
    );

    let mut with_ref = sent(d(3, 2), &["a@b.example"]);
    with_ref.reference = Some("x".repeat(501));
    let e = refused(&mut b.l, |l| l.mark_sent(inv, with_ref));
    assert!(
        matches!(&e, LedgerError::InvalidSend(m) if m.contains("the reference is at most 500")),
        "{e:?}"
    );

    let mut with_note = sent(d(3, 2), &["a@b.example"]);
    with_note.note = Some("EIN 12-3456789".into());
    let e = refused(&mut b.l, |l| l.mark_sent(inv, with_note));
    assert!(
        matches!(&e, LedgerError::InvalidSend(m) if m.contains("the note")),
        "{e:?}"
    );
    assert!(b.l.get_invoice(inv).unwrap().sends.is_empty());
}

// ---------------------------------------------------------------- settle

fn settle_one(b: &Books, inv: u64, cash: &str) -> SettleParams {
    SettleParams {
        date: d(3, 20),
        description: String::new(),
        cash_account: b.bank,
        party: b.customer,
        applications: vec![Application {
            doc: DocRef::Invoice(inv),
            cash: dec(cash),
            discount: Decimal::ZERO,
            discount_reason: None,
        }],
        fee: None,
        instrument: None,
        reference: None,
    }
}

/// BS-arap-6 statements 1, 4, 5 and 6 (invariants 4 and 5): each thing a
/// settlement can be refused for,
/// refused with its own error, the books untouched. The invoice is for
/// 500.00 dated March 1; the payment is dated March 20.
#[test]
fn a_settlement_is_refused_for_what_it_cannot_make_whole() {
    let mut b = books();
    let inv = b.invoice(d(3, 1), "500.00");
    let other =
        b.l.create_invoice(b.other_customer, d(3, 1), d(3, 31), lines("80.00"))
            .unwrap();
    let bill = b.bill(d(3, 1), "300.00");
    let go = |b: &mut Books, f: &dyn Fn(&mut SettleParams)| {
        let mut p = settle_one(b, inv, "100.00");
        f(&mut p);
        refused(&mut b.l, |l| l.settle(p))
    };

    assert_eq!(
        go(&mut b, &|p| p.applications.clear()),
        LedgerError::SettlementEmpty
    );
    assert!(matches!(
        go(&mut b, &|p| p.applications.push(Application {
            doc: DocRef::Bill(bill), cash: dec("1.00"), discount: Decimal::ZERO, discount_reason: None
        })),
        LedgerError::SettlementMixedKinds(m) if m.contains(&bill.to_string())
    ));
    assert!(matches!(
        go(&mut b, &|p| p.applications.push(Application {
            doc: DocRef::Invoice(other), cash: dec("1.00"), discount: Decimal::ZERO, discount_reason: None
        })),
        LedgerError::SettlementPartiesDiffer(_, party) if party == b.customer
    ));
    assert_eq!(
        go(&mut b, &|p| p.applications[0].doc = DocRef::Invoice(77)),
        LedgerError::UnknownInvoice(77)
    );
    assert!(matches!(
        go(&mut b, &|p| p.date = NaiveDate::MAX),
        LedgerError::DateOutOfRange(_)
    ));
    assert_eq!(
        go(&mut b, &|p| p.instrument = Some(9)),
        LedgerError::UnknownInstrument(9)
    );
    let revenue = b.revenue;
    assert!(matches!(
        go(&mut b, &|p| p.cash_account = revenue),
        LedgerError::PaymentAccount { why, .. } if why.contains("revenue")
    ));
    assert!(matches!(
        go(&mut b, &|p| p.applications.push(p.applications[0].clone())),
        LedgerError::SettlementDuplicateDocument(_)
    ));
    assert_eq!(
        go(&mut b, &|p| p.applications[0].cash = Decimal::ZERO),
        LedgerError::InvalidPaymentAmount(Decimal::ZERO)
    );
    assert!(matches!(
        go(&mut b, &|p| p.applications[0].discount = dec("-1.00")),
        LedgerError::InvalidSettlement(m) if m.contains("a discount cannot be negative")
    ));
    assert!(matches!(
        go(&mut b, &|p| p.date = d(2, 28)),
        LedgerError::PaymentBeforeDocument { .. }
    ));
    assert_eq!(
        go(&mut b, &|p| p.applications[0].cash = dec("500.01")),
        LedgerError::Overpayment(dec("500.01"), dec("500.00"))
    );
    for bad in ["two\nlines".to_string(), "x".repeat(300)] {
        assert!(matches!(
            go(&mut b, &|p| {
                p.applications[0].discount = dec("1.00");
                p.applications[0].discount_reason = Some(bad.clone());
            }),
            LedgerError::InvalidDiscountReason(_)
        ));
    }
    // A discount the terms don't state and no reason is refused, saying why.
    assert!(matches!(
        go(&mut b, &|p| p.applications[0].discount = dec("1.00")),
        LedgerError::DiscountNeedsReason { because, .. }
            if because.contains("no early-payment discount")
    ));
}

/// BS-arap-6, the fee: it comes out of a customer payment gross, goes to an
/// expense account, and cannot be the whole payment; the cash and
/// discounts must cover the applications exactly. A closed period takes no
/// settlement.
#[test]
fn a_fee_is_an_expense_taken_from_the_payment_and_a_closed_period_refuses() {
    let mut b = books();
    let inv = b.invoice(d(3, 1), "500.00");
    let fee = |amount: &str, account: Option<AccountId>| {
        Some(Fee {
            amount: dec(amount),
            account,
            vendor: None,
        })
    };
    // The fee is the whole payment: nothing would be deposited.
    let mut p = settle_one(&b, inv, "100.00");
    p.fee = fee("100.00", Some(b.fees));
    assert!(matches!(
        refused(&mut b.l, |l| l.settle(p)),
        LedgerError::InvalidSettlement(m) if m.contains("deposit would be nothing")
    ));
    // The fee is above the cash.
    let mut p = settle_one(&b, inv, "100.00");
    p.fee = fee("100.01", Some(b.fees));
    assert_eq!(
        refused(&mut b.l, |l| l.settle(p)),
        LedgerError::FeeAboveCash {
            fee: dec("100.01"),
            cash: dec("100.00")
        }
    );
    // The fee's account must be an expense that takes postings.
    let mut p = settle_one(&b, inv, "100.00");
    p.fee = fee("3.20", Some(b.revenue));
    assert!(matches!(
        refused(&mut b.l, |l| l.settle(p)),
        LedgerError::InvalidFee(m) if m.contains("is not an expense account")
    ));

    // A real one: 100.00 paid, 3.20 withheld: the receivable falls by the
    // gross 100.00, the fee is 3.20 of expense, the bank gets 96.80.
    let mut p = settle_one(&b, inv, "100.00");
    p.fee = fee("3.20", Some(b.fees));
    p.description = "Stripe payout".into();
    let entry = b.l.settle(p).unwrap();
    assert_eq!(b.l.get_entry(entry).unwrap().description(), "Stripe payout");
    assert_eq!(b.balance("1000"), dec("96.80"));
    assert_eq!(b.balance("6950"), dec("3.20"));
    assert_eq!(b.balance("1200"), dec("400.00"));
    let st =
        b.l.doc_state(DocRef::Invoice(inv), d(3, 31))
            .unwrap()
            .unwrap();
    assert_eq!(
        (st.paid, st.outstanding, st.status),
        (dec("100.00"), dec("400.00"), DocStatus::Partial)
    );

    // A closed period takes no settlement.
    let q = b.l.create_period("March", d(3, 1), d(3, 31)).unwrap();
    b.l.close_period(q).unwrap();
    let p = settle_one(&b, inv, "100.00");
    assert!(
        matches!(refused(&mut b.l, |l| l.settle(p)), LedgerError::ClosedPeriod(id, _) if id == q)
    );
}

/// BS-arap-6 and the settlement's own description: with no description it
/// says how many documents; a payment on a bill is the mirror (Dr payables,
/// Cr cash) and a fee on a bill is not a thing.
#[test]
fn a_bill_settlement_mirrors_an_invoices_and_describes_itself() {
    let mut b = books();
    let bill = b.bill(d(3, 1), "300.00");
    let p = SettleParams {
        date: d(3, 20),
        description: "   ".into(),
        cash_account: b.bank,
        party: b.vendor,
        applications: vec![Application {
            doc: DocRef::Bill(bill),
            cash: dec("300.00"),
            discount: Decimal::ZERO,
            discount_reason: None,
        }],
        fee: None,
        instrument: None,
        reference: None,
    };
    let entry = b.l.settle(p).unwrap();
    assert_eq!(
        b.l.get_entry(entry).unwrap().description(),
        "Settlement of 1 document"
    );
    assert_eq!(b.balance("2000"), Decimal::ZERO, "the payable is settled");
    assert_eq!(b.balance("1000"), dec("-300.00"));
    let st =
        b.l.doc_state(DocRef::Bill(bill), d(3, 31))
            .unwrap()
            .unwrap();
    assert_eq!(st.status, DocStatus::Paid);
}

// ---------------------------------------------------------------- write_off

/// BS-arap-5 statements 2, 4 and 6 (invariants 2, 3 and 5; the one-line
/// reason is the engine's own guard, beside statement 9's evidence of the
/// debt): a write-off is refused on the cash basis,
/// without a one-line reason, for an amount that is not above zero, off the
/// calendar, before the invoice, in a closed period, or more than the
/// invoice owes on every day from then on; the refusals change nothing.
#[test]
fn a_write_off_is_refused_for_each_rule_it_breaks() {
    let mut b = books();
    let inv = b.invoice(d(3, 1), "100.00");
    b.l.receive_payment(inv, d(4, 10), dec("30.00"), b.bank)
        .unwrap();
    let off = |l: &mut Ledger, date, amount: &str, reason: &str| {
        refused(l, |l| l.write_off(inv, date, dec(amount), reason))
    };
    let msg = |e: LedgerError| match e {
        LedgerError::InvalidWriteOff(m) => m,
        other => panic!("{other:?}"),
    };

    assert!(msg(off(&mut b.l, d(4, 1), "10.00", "  ")).contains("needs its reason"));
    assert!(msg(off(&mut b.l, d(4, 1), "10.00", "a\nb")).contains("needs its reason"));
    assert!(msg(off(&mut b.l, d(4, 1), "10.00", &"x".repeat(300))).contains("needs its reason"));
    assert!(msg(off(&mut b.l, d(4, 1), "0", "gone")).contains("must be above zero"));
    assert!(msg(off(&mut b.l, d(4, 1), "-5", "gone")).contains("must be above zero"));
    let e = refused(&mut b.l, |l| {
        l.write_off(inv, NaiveDate::MAX, dec("5"), "gone")
    });
    assert!(matches!(e, LedgerError::DateOutOfRange(_)));
    assert!(msg(off(&mut b.l, d(2, 20), "10.00", "gone")).contains("can't be before invoice"));
    let e = refused(&mut b.l, |l| l.write_off(77, d(4, 1), dec("5"), "gone"));
    assert_eq!(e, LedgerError::UnknownInvoice(77));
    // On April 1 the invoice owes 100.00, but the 30.00 payment on April 10
    // means no day may owe less than nothing: only 70.00 can go.
    let m = msg(off(&mut b.l, d(4, 1), "80.00", "gone"));
    assert!(
        m.contains("more than invoice") && m.contains("100.00") && m.contains("70.00"),
        "{m}"
    );
    // After the payment the room is what is owed; more than that is refused
    // without the "once payments after it are counted" clause.
    let m = msg(off(&mut b.l, d(4, 15), "70.01", "gone"));
    assert!(
        m.contains("owes on 2026-04-15: 70.00") && !m.contains("once the payments"),
        "{m}"
    );

    // A closed period.
    let q = b.l.create_period("March", d(3, 1), d(3, 31)).unwrap();
    b.l.close_period(q).unwrap();
    let e = refused(&mut b.l, |l| l.write_off(inv, d(3, 20), dec("5"), "gone"));
    assert!(matches!(e, LedgerError::ClosedPeriod(id, _) if id == q));

    // Written off at the room that is left: bad debt 70.00, receivable 0.
    let entry =
        b.l.write_off(inv, d(4, 15), dec("70.00"), "  uncollectible ")
            .unwrap();
    assert_eq!(
        b.l.get_entry(entry).unwrap().description(),
        "Write off invoice 1: uncollectible"
    );
    assert_eq!(b.balance("6990"), dec("70.00"));
    assert_eq!(b.balance("1200"), Decimal::ZERO);
}

/// BS-arap-5 statement 6 (invariant 5): on cash-basis books nothing is receivable to
/// write off, so the command is refused whatever else is true.
#[test]
fn a_write_off_is_refused_on_the_cash_basis() {
    let mut b = books();
    let inv = b.invoice(d(3, 1), "100.00");
    let src = Source {
        decided_by: "Owner".into(),
        decided_on: d(11, 9),
        citation: None,
        note: None,
    };
    let changes = [Change {
        key: "books.accounting_basis".into(),
        value: Some(json!("cash")),
    }];
    let made = b.l.check_settings(&changes, &src).unwrap();
    b.l.apply_settings(&made, &src, 1).unwrap();
    let e = refused(&mut b.l, |l| {
        l.write_off(inv, d(4, 1), dec("10.00"), "gone")
    });
    assert_eq!(e, LedgerError::WriteOffOnCashBasis);
}

// ---------------------------------------------------------------- aging

/// BS-arap-2: the aging list is the documents that owe something on the
/// day, oldest due first, each in the bucket its due date puts it in, and
/// invoices and bills are listed apart. A paid, void or not yet issued
/// document is not listed.
#[test]
fn aging_lists_what_is_owed_oldest_due_first() {
    let mut b = books();
    let late = b.invoice(d(1, 5), "100.00"); // due Feb 4
    let newer = b.invoice(d(3, 1), "200.00"); // due Mar 31
    let paid = b.invoice(d(1, 6), "50.00");
    b.l.receive_payment(paid, d(1, 10), dec("50.00"), b.bank)
        .unwrap();
    let voided = b.invoice(d(1, 7), "60.00");
    b.l.void_document(DocRef::Invoice(voided), d(1, 8), "dup")
        .unwrap();
    let future = b.invoice(d(6, 1), "70.00");
    let bill = b.bill(d(2, 1), "300.00");
    b.l.pay_bill(bill, d(2, 5), dec("100.00"), b.bank).unwrap();

    let aging = b.l.aging_documents(d(4, 15), true).unwrap();
    let ids: Vec<DocRef> = aging.iter().map(|a| a.doc).collect();
    assert_eq!(ids, vec![DocRef::Invoice(late), DocRef::Invoice(newer)]);
    assert!(!ids.contains(&DocRef::Invoice(future)));
    assert_eq!(aging[0].party, "Fernhill Labs");
    assert_eq!(aging[0].outstanding, dec("100.00"));
    assert_eq!(aging[0].due_date, d(2, 4));
    assert!(aging[0].days_overdue > aging[1].days_overdue);
    assert_eq!(aging[1].bucket, "days_30");

    let bills = b.l.aging_documents(d(4, 15), false).unwrap();
    assert_eq!(bills.len(), 1);
    assert_eq!(bills[0].doc, DocRef::Bill(bill));
    assert_eq!(
        (bills[0].total, bills[0].paid, bills[0].outstanding),
        (dec("300.00"), dec("100.00"), dec("200.00"))
    );
    assert_eq!(bills[0].party, "Tobin Harlow Supply");

    // With a terms string the list shows it.
    let fields = InvoiceFields {
        terms: Some(ledger_core::arap::Terms::parse("2/10 net 30").unwrap()),
        ..InvoiceFields::default()
    };
    let termed =
        b.l.create_invoice_with(b.customer, d(4, 1), d(5, 1), lines("10.00"), fields)
            .unwrap();
    let aging = b.l.aging_documents(d(4, 15), true).unwrap();
    let t = aging
        .iter()
        .find(|a| a.doc == DocRef::Invoice(termed))
        .unwrap();
    assert_eq!(t.terms.as_deref(), Some("2/10 net 30"));
}
