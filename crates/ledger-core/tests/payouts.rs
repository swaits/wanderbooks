//! Payout gross-up. Numbers computed by hand.
//!
//! An app-store month: App A sold 1,000.00, App B 500.00; the store kept
//! 15% (225.00) and clawed back a 9.99 refund. Deposit: 1,500.00 − 225.00
//! − 9.99 = 1,265.01.

use chrono::NaiveDate;
use ledger_core::reports::profit_and_loss_for_period;
use ledger_core::{
    AccountType, Currency, Ledger, LedgerError, Money, Payout, PayoutAdjustment, PayoutLine, Side,
    payout_plan,
};
use rust_decimal::Decimal;
use std::str::FromStr;

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

fn d(day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 11, day).unwrap()
}

struct Books {
    l: Ledger,
    bank: u64,
    sales: u64,
    fees: u64,
    refunds: u64,
}

fn books() -> Books {
    let mut l = Ledger::new(Currency::USD);
    let mut a = |code: &str, name: &str, t| l.add_account(code, name, t, None, false).unwrap();
    let bank = a("1000", "Bank", AccountType::Asset);
    let sales = a("4000", "App Sales", AccountType::Revenue);
    let fees = a("6000", "Platform Fees", AccountType::Expense);
    let refunds = a("4100", "Refunds", AccountType::Expense);
    Books {
        l,
        bank,
        sales,
        fees,
        refunds,
    }
}

fn line(account: u64, amount: &str) -> PayoutLine {
    PayoutLine {
        account,
        amount: dec(amount),
        class: None,
    }
}

fn month(b: &Books, deposit: &str) -> Payout {
    Payout {
        deposit_account: b.bank,
        deposit: dec(deposit),
        revenue: vec![line(b.sales, "1000.00"), line(b.sales, "500.00")],
        fees: vec![line(b.fees, "225.00")],
        adjustments: vec![PayoutAdjustment {
            account: b.refunds,
            side: Side::Debit,
            amount: dec("9.99"),
            class: None,
        }],
    }
}

#[test]
fn a_payout_is_grossed_up_to_revenue_and_fees() {
    let mut b = books();
    let (plan, totals) = payout_plan(&month(&b, "1265.01"), b.l.chart()).unwrap();
    assert_eq!(totals.gross, dec("1500.00"));
    assert_eq!(totals.fees, dec("225.00"));
    assert_eq!(totals.adjustments, dec("-9.99"));
    assert_eq!(
        plan.iter()
            .map(|(a, s, m, _)| (*a, *s, *m))
            .collect::<Vec<_>>(),
        [
            (b.bank, Side::Debit, dec("1265.01")),
            (b.fees, Side::Debit, dec("225.00")),
            (b.refunds, Side::Debit, dec("9.99")),
            (b.sales, Side::Credit, dec("1000.00")),
            (b.sales, Side::Credit, dec("500.00")),
        ]
    );
    let e = b.l.create_entry(d(30), "App store payout");
    for (a, s, m, _) in plan {
        b.l.add_line(e, a, s, Money::new(m, Currency::USD)).unwrap();
    }
    b.l.post_entry(e).unwrap();
    let pl = profit_and_loss_for_period(&b.l, d(1), d(30), None).unwrap();
    assert_eq!(pl.total_revenue, dec("1500.00"));
    assert_eq!(pl.total_expenses, dec("234.99"));
}

#[test]
fn a_payout_must_reconcile_to_the_cent() {
    let b = books();
    let err = payout_plan(&month(&b, "1265.00"), b.l.chart()).unwrap_err();
    let LedgerError::InvalidPayout(msg) = err else {
        panic!("{err:?}")
    };
    assert!(msg.contains("= 1265.01"), "{msg}");
    assert!(msg.contains("off by -0.01"), "{msg}");
}

#[test]
fn a_negative_payout_credits_the_bank() {
    let b = books();
    // 10.00 of sales, 3.00 of fees, a 50.00 refund: the store debits 43.00.
    let p = Payout {
        deposit_account: b.bank,
        deposit: dec("-43.00"),
        revenue: vec![line(b.sales, "10.00")],
        fees: vec![line(b.fees, "3.00")],
        adjustments: vec![PayoutAdjustment {
            account: b.refunds,
            side: Side::Debit,
            amount: dec("50.00"),
            class: None,
        }],
    };
    let (plan, _) = payout_plan(&p, b.l.chart()).unwrap();
    assert_eq!(plan[0], (b.bank, Side::Credit, dec("43.00"), None));
}

#[test]
fn accounts_must_fit_their_roles() {
    let b = books();
    let invalid = |p: Payout| {
        matches!(
            payout_plan(&p, b.l.chart()),
            Err(LedgerError::InvalidPayout(_))
        )
    };
    let mut p = month(&b, "1265.01");
    p.fees[0].account = b.sales;
    assert!(invalid(p), "fees must be expenses");
    let mut p = month(&b, "1265.01");
    p.revenue[0].account = b.fees;
    assert!(invalid(p), "revenue must be revenue or a liability");
    let mut p = month(&b, "1265.01");
    p.deposit_account = b.sales;
    assert!(invalid(p), "deposits land in an asset");
    let mut p = month(&b, "1265.01");
    p.fees[0].amount = dec("-225.00");
    assert!(invalid(p), "amounts are positive");
    let mut p = month(&b, "1265.01");
    p.revenue.clear();
    assert!(invalid(p), "needs revenue");
}

#[test]
fn a_zero_payout_has_no_bank_line() {
    let b = books();
    // 10.00 of sales, 3.00 of fees, a 7.00 refund: nothing deposited.
    let p = Payout {
        deposit_account: b.bank,
        deposit: dec("0.00"),
        revenue: vec![line(b.sales, "10.00")],
        fees: vec![line(b.fees, "3.00")],
        adjustments: vec![PayoutAdjustment {
            account: b.refunds,
            side: Side::Debit,
            amount: dec("7.00"),
            class: None,
        }],
    };
    let (plan, totals) = payout_plan(&p, b.l.chart()).unwrap();
    assert_eq!(totals.deposit, Decimal::ZERO);
    assert_eq!(
        plan.iter()
            .map(|(a, s, m, _)| (*a, *s, *m))
            .collect::<Vec<_>>(),
        [
            (b.fees, Side::Debit, dec("3.00")),
            (b.refunds, Side::Debit, dec("7.00")),
            (b.sales, Side::Credit, dec("10.00")),
        ]
    );
}
