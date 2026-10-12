//! Time series. Numbers computed by hand.
//!
//! Cash 1000 and card 2100. Jan 5: owner puts in 5,000. Jan 20: 300 of
//! software on the card. Feb 3: 1,200 of sales into cash. Feb 10: pay the
//! card 300. Mar 15: 800 rent from cash.

use chrono::NaiveDate;
use ledger_core::analysis::{
    Interval, balance_history, balance_series, flows_by_interval, pnl_by_interval,
};
use ledger_core::{AccountType, Currency, Ledger, Money, Side};
use rust_decimal::Decimal;

fn d(m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, m, day).unwrap()
}

fn dec(n: i64) -> Decimal {
    Decimal::from(n)
}

#[test]
fn balances_pnl_and_flows_over_time() {
    let mut l = Ledger::new(Currency::USD);
    let mut a = |c: &str, t| l.add_account(c, c, t, None, false).unwrap();
    let cash = a("1000", AccountType::Asset);
    let card = a("2100", AccountType::Liability);
    let equity = a("3000", AccountType::Equity);
    let sales = a("4000", AccountType::Revenue);
    let software = a("6300", AccountType::Expense);
    let rent = a("6600", AccountType::Expense);
    for (date, dr, cr, amt) in [
        (d(1, 5), cash, equity, 5000),
        (d(1, 20), software, card, 300),
        (d(2, 3), cash, sales, 1200),
        (d(2, 10), card, cash, 300),
        (d(3, 15), rent, cash, 800),
    ] {
        let e = l.create_entry(date, "x");
        let m = Money::new(dec(amt), Currency::USD);
        l.add_line(e, dr, Side::Debit, m).unwrap();
        l.add_line(e, cr, Side::Credit, m).unwrap();
        l.post_entry(e).unwrap();
    }
    assert_eq!(
        balance_series(&l, &[cash], &[d(1, 1), d(1, 31), d(2, 28), d(3, 31)]),
        [dec(0), dec(5000), dec(5900), dec(5100)]
    );
    // The card is a liability: owed 300, then paid off.
    assert_eq!(
        balance_series(&l, &[card], &[d(1, 31), d(2, 28)]),
        [dec(300), dec(0)]
    );
    assert_eq!(
        balance_history(&l, &[cash]),
        [
            (d(1, 5), dec(5000)),
            (d(2, 3), dec(6200)),
            (d(2, 10), dec(5900)),
            (d(3, 15), dec(5100))
        ]
    );
    let pnl = pnl_by_interval(&l, d(1, 1), d(3, 31), Interval::Month).unwrap();
    let nets: Vec<Decimal> = pnl.iter().map(|p| p.net()).collect();
    assert_eq!(nets, [dec(-300), dec(1200), dec(-800)]);
    let flows = flows_by_interval(&l, &[cash], d(1, 1), d(3, 31), Interval::Month);
    let io: Vec<(Decimal, Decimal)> = flows.iter().map(|f| (f.2, f.3)).collect();
    assert_eq!(
        io,
        [
            (dec(5000), dec(0)),
            (dec(1200), dec(300)),
            (dec(0), dec(800))
        ]
    );
    // Cash and card together: a card charge is money out when it's made,
    // and paying the card moves nothing out of the pair.
    let both = flows_by_interval(&l, &[cash, card], d(1, 1), d(2, 28), Interval::Month);
    let io: Vec<(Decimal, Decimal)> = both.iter().map(|f| (f.2, f.3)).collect();
    assert_eq!(io, [(dec(5000), dec(300)), (dec(1200), dec(0))]);
}
