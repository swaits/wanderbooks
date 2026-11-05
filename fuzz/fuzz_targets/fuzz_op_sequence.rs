// Fuzz target: arbitrary command sequences against a Ledger, including
// balanced pairs, foreign-currency lines, periods, and closes, with
// amounts built from raw decimal bits (huge, tiny, odd scales). Refusals
// are fine; panics, hangs, and broken books are not. After any sequence:
// the trial balance balances, every posted entry validates, no draft sits
// in a closed period (the Lean invariant Inv), and the control accounts
// equal their documents (control_check's difference is zero, B-38).
//
// The documents (invoices, bills), their settlements and payments, write-offs
// and voids, and the draft-state ops (P12) run on the Ledger the store wraps:
// the store adds the journal, the engine is the state machine fuzzed here.
#![no_main]

use arbitrary::Arbitrary;
use chrono::NaiveDate;
use ledger_core::arap::{DocRef, DocumentLine};
use ledger_core::{
    AccountType, Application, Currency, DraftState, EntryStatus, Ledger, Money, PaymentDetails,
    SettleParams, Side,
};
use libfuzzer_sys::fuzz_target;
use rust_decimal::Decimal;

#[derive(Arbitrary, Debug, Clone, Copy)]
enum Op {
    AddAccount {
        code: u8,
        typ: u8,
        placeholder: bool,
    },
    CreateEntry {
        day: u8,
    },
    AddLine {
        entry: u8,
        account: u8,
        debit: bool,
        amount: Amount,
    },
    Pair {
        entry: u8,
        debit: u8,
        credit: u8,
        amount: Amount,
    },
    AddLineFx {
        entry: u8,
        account: u8,
        debit: bool,
        foreign: Amount,
        rate: Amount,
    },
    PostEntry {
        entry: u8,
    },
    ReverseEntry {
        entry: u8,
        day: u8,
    },
    CreatePeriod {
        start: u8,
        len: u8,
    },
    ClosePeriod {
        period: u8,
    },
    CreateInvoice {
        day: u8,
        terms: u8,
        amount: Amount,
    },
    CreateBill {
        day: u8,
        terms: u8,
        amount: Amount,
    },
    /// A payment on a document (mod the documents) from the bank or a
    /// fuzzed account, with a reference or not.
    Pay {
        doc: u8,
        day: u8,
        amount: Amount,
        from: u8,
        reference: bool,
    },
    /// One payment settling two documents, the first with a discount.
    Settle {
        a: u8,
        b: u8,
        day: u8,
        cash: Amount,
        discount: Amount,
    },
    WriteOff {
        doc: u8,
        day: u8,
        amount: Amount,
    },
    VoidDocument {
        doc: u8,
        day: u8,
    },
    SetDraftState {
        entry: u8,
        awaiting: bool,
        note: u8,
    },
    DiscardDraft {
        entry: u8,
    },
}

/// A decimal from raw parts: any mantissa up to 96 bits, any scale 0-28.
#[derive(Arbitrary, Debug, Clone, Copy)]
struct Amount {
    lo: u32,
    mid: u32,
    hi: u32,
    negative: bool,
    scale: u8,
    small: bool,
}

impl Amount {
    fn get(self) -> Decimal {
        if self.small {
            // Mostly ordinary money, so entries balance and post.
            return Decimal::new(i64::from(self.lo % 1_000_000), 2);
        }
        Decimal::from_parts(
            self.lo,
            self.mid,
            self.hi,
            self.negative,
            u32::from(self.scale % 29),
        )
    }
}

fn account_type(t: u8) -> AccountType {
    match t % 5 {
        0 => AccountType::Asset,
        1 => AccountType::Liability,
        2 => AccountType::Equity,
        3 => AccountType::Revenue,
        _ => AccountType::Expense,
    }
}

fn day(d: u8) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 1, 1).unwrap() + chrono::Days::new(u64::from(d % 200))
}

fuzz_target!(|ops: Vec<Op>| {
    let mut l = Ledger::new(Currency::USD);
    let mut entries: Vec<u64> = Vec::new();
    let mut docs: Vec<DocRef> = Vec::new();
    // The books' fixed parts: the control accounts documents post to, a bank,
    // a customer and a vendor.
    let _ = (
        l.add_control_account(&ledger_core::RECEIVABLES),
        l.add_control_account(&ledger_core::PAYABLES),
        l.add_control_account(&ledger_core::BAD_DEBTS),
    );
    let bank = l
        .add_account("1900", "Bank", AccountType::Asset, None, false)
        .ok();
    let customer = l.add_customer("Fernhill Labs").ok();
    let vendor = l.add_vendor("Tobin Harlow Supply").ok();
    let doc_line = |a: Amount| {
        vec![DocumentLine {
            description: "work".into(),
            amount: a.get(),
            class_id: None,
            account: None,
            quantity: None,
            unit_price: None,
        }]
    };
    let pick = |v: &[u64], i: u8| v.get(i as usize % v.len().max(1)).copied();
    for op in ops.into_iter().take(200) {
        let accounts: Vec<u64> = l.chart().all().map(|a| a.id).collect();
        match op {
            Op::AddAccount {
                code,
                typ,
                placeholder,
            } => {
                let _ = l.add_account(
                    &format!("{}", 1000 + u32::from(code % 40)),
                    "f",
                    account_type(typ),
                    None,
                    placeholder,
                );
            }
            Op::CreateEntry { day: d } => {
                if let Ok(id) = l.try_create_entry(day(d), "f") {
                    entries.push(id);
                }
            }
            Op::AddLine {
                entry,
                account,
                debit,
                amount,
            } => {
                if let (Some(e), Some(a)) = (pick(&entries, entry), pick(&accounts, account)) {
                    let side = if debit { Side::Debit } else { Side::Credit };
                    let _ = l.add_line(e, a, side, Money::new(amount.get(), Currency::USD));
                }
            }
            Op::Pair {
                entry,
                debit,
                credit,
                amount,
            } => {
                if let (Some(e), Some(a), Some(b)) = (
                    pick(&entries, entry),
                    pick(&accounts, debit),
                    pick(&accounts, credit),
                ) {
                    let m = Money::new(amount.get(), Currency::USD);
                    let _ = l.add_line(e, a, Side::Debit, m);
                    let _ = l.add_line(e, b, Side::Credit, m);
                }
            }
            Op::AddLineFx {
                entry,
                account,
                debit,
                foreign,
                rate,
            } => {
                if let (Some(e), Some(a)) = (pick(&entries, entry), pick(&accounts, account)) {
                    let side = if debit { Side::Debit } else { Side::Credit };
                    let _ = l.add_line_fx(e, a, side, Currency::EUR, foreign.get(), rate.get());
                }
            }
            Op::PostEntry { entry } => {
                if let Some(e) = pick(&entries, entry) {
                    let _ = l.post_entry(e);
                }
            }
            Op::ReverseEntry { entry, day: d } => {
                if let Some(e) = pick(&entries, entry)
                    && let Ok(r) = l.reverse_entry(e, day(d), "f")
                {
                    entries.push(r);
                }
            }
            Op::CreatePeriod { start, len } => {
                let _ = l.create_period("p", day(start), day(start.saturating_add(len % 60)));
            }
            Op::CreateInvoice {
                day: d,
                terms,
                amount,
            } => {
                if let Some(c) = customer
                    && let Ok(id) = l.create_invoice_with(
                        c,
                        day(d),
                        day(d) + chrono::Days::new(u64::from(terms)),
                        doc_line(amount),
                        Default::default(),
                    )
                {
                    docs.push(DocRef::Invoice(id));
                }
            }
            Op::CreateBill {
                day: d,
                terms,
                amount,
            } => {
                if let Some(v) = vendor
                    && let Ok(id) = l.create_bill_with(
                        v,
                        day(d),
                        day(d) + chrono::Days::new(u64::from(terms)),
                        doc_line(amount),
                        Default::default(),
                    )
                {
                    docs.push(DocRef::Bill(id));
                }
            }
            Op::Pay {
                doc,
                day: d,
                amount,
                from,
                reference,
            } => {
                if !docs.is_empty() {
                    let cash = if from % 4 == 0 {
                        pick(&accounts, from)
                    } else {
                        bank
                    };
                    let details = PaymentDetails {
                        instrument: None,
                        reference: reference.then(|| format!("trace {doc}{d}")),
                    };
                    if let Some(cash) = cash {
                        let _ = match docs[doc as usize % docs.len()] {
                            DocRef::Invoice(id) => {
                                l.receive_payment_with(id, day(d), amount.get(), cash, &details)
                            }
                            DocRef::Bill(id) => {
                                l.pay_bill_with(id, day(d), amount.get(), cash, &details)
                            }
                        };
                    }
                }
            }
            Op::Settle {
                a,
                b,
                day: d,
                cash,
                discount,
            } => {
                if let (false, Some(cash_account)) = (docs.is_empty(), bank) {
                    let first = docs[a as usize % docs.len()];
                    let second = docs[b as usize % docs.len()];
                    let party = match first {
                        DocRef::Invoice(id) => l.get_invoice(id).map(|i| i.customer_id),
                        DocRef::Bill(id) => l.get_bill(id).map(|v| v.vendor_id),
                    };
                    if let Some(party) = party {
                        let _ = l.settle(SettleParams {
                            date: day(d),
                            description: "two documents, one payment".into(),
                            cash_account,
                            party,
                            applications: vec![
                                Application {
                                    doc: first,
                                    cash: cash.get(),
                                    discount: discount.get(),
                                    discount_reason: Some("fuzz".into()),
                                },
                                Application {
                                    doc: second,
                                    cash: cash.get(),
                                    discount: Decimal::ZERO,
                                    discount_reason: None,
                                },
                            ],
                            fee: None,
                            instrument: None,
                            reference: None,
                        });
                    }
                }
            }
            Op::WriteOff {
                doc,
                day: d,
                amount,
            } => {
                if !docs.is_empty()
                    && let DocRef::Invoice(id) = docs[doc as usize % docs.len()]
                {
                    let _ = l.write_off(id, day(d), amount.get(), "uncollectible");
                }
            }
            Op::VoidDocument { doc, day: d } => {
                if !docs.is_empty() {
                    let _ = l.void_document(docs[doc as usize % docs.len()], day(d), "duplicate");
                }
            }
            Op::SetDraftState {
                entry,
                awaiting,
                note,
            } => {
                if let Some(e) = pick(&entries, entry) {
                    let state = if awaiting {
                        DraftState::AwaitingEvidence {
                            note: format!("receipt {note}"),
                        }
                    } else {
                        DraftState::Building
                    };
                    let _ = l.set_draft_state(e, state);
                }
            }
            Op::DiscardDraft { entry } => {
                if let Some(e) = pick(&entries, entry) {
                    let _ = l.discard_draft(e, "fuzz");
                }
            }
            Op::ClosePeriod { period } => {
                let ids: Vec<u64> = l.list_periods().iter().map(|p| p.id).collect();
                if let Some(p) = pick(&ids, period) {
                    let _ = l.close_period(p);
                }
            }
        }
    }
    let tb = ledger_core::reports::trial_balance(&l);
    assert_eq!(tb.total_debits, tb.total_credits, "trial balance broken");
    for e in l.posted_entries() {
        assert!(
            e.validate(l.chart()).is_ok(),
            "posted entry {} invalid",
            e.id()
        );
    }
    // The control accounts equal their documents, at any day (B-38): too
    // large to compute is a refusal, never a difference.
    for d in [0u8, 60, 199] {
        if let Ok(checks) = l.control_check(day(d)) {
            for c in checks {
                assert_eq!(
                    c.difference,
                    Decimal::ZERO,
                    "{} differs from its documents",
                    c.role
                );
            }
        }
    }
    for id in 1..l.next_entry_id() {
        if let Some(e) = l.get_entry(id)
            && e.status() == EntryStatus::Draft
        {
            assert!(
                l.closed_period_containing(e.date()).is_none(),
                "draft in a closed period"
            );
        }
    }
});
