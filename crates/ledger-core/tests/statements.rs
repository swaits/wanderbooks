//! Period-scoped financial statements: profit and loss for a date range and
//! the balance sheet as of a date, line per account.
//!
//! Every expected number below is computed by hand from the entries, not
//! read off the engine. The fixture is two months of a small company:
//!
//! | date   | entry                         | Dr            | Cr            |
//! |--------|-------------------------------|---------------|---------------|
//! | Jan 2  | owner contributes 10,000      | 1000 Cash     | 3000 Capital  |
//! | Jan 15 | client pays 2,500             | 1000 Cash     | 4000 Revenue  |
//! | Jan 20 | rent 800                      | 6100 Rent     | 1000 Cash     |
//! | Feb 3  | software 120 on the card      | 6000 Software | 2000 Card     |
//! | Feb 10 | client pays 1,000             | 1000 Cash     | 4000 Revenue  |
//! | Feb 25 | pay the card 120              | 2000 Card     | 1000 Cash     |

use chrono::NaiveDate;
use ledger_core::reports::{
    balance_sheet, balance_sheet_as_of, profit_and_loss, profit_and_loss_for_period,
};
use ledger_core::{
    AccountId, AccountType, CashFlowParams, ClassId, Currency, Ledger, Money, Side, StatementLine,
    cash_flow,
};
use rust_decimal::Decimal;

fn d(m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, m, day).unwrap()
}

fn usd(n: i64) -> Money {
    Money::new(Decimal::from(n), Currency::USD)
}

fn dec(n: i64) -> Decimal {
    Decimal::from(n)
}

struct Books {
    ledger: Ledger,
    cash: AccountId,
    software_class: ClassId,
}

fn post(ledger: &mut Ledger, date: NaiveDate, desc: &str, dr: AccountId, cr: AccountId, n: i64) {
    let e = ledger.create_entry(date, desc);
    ledger.add_line(e, dr, Side::Debit, usd(n)).unwrap();
    ledger.add_line(e, cr, Side::Credit, usd(n)).unwrap();
    ledger.post_entry(e).unwrap();
}

/// The fixture. With `close_january`, January is closed before any February
/// entry exists (the close sweeps every revenue and expense balance).
fn books(close_january: bool) -> Books {
    let mut l = Ledger::new(Currency::USD);
    let cash = l
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    let card = l
        .add_account("2000", "Card", AccountType::Liability, None, false)
        .unwrap();
    let capital = l
        .add_account("3000", "Capital", AccountType::Equity, None, false)
        .unwrap();
    let revenue = l
        .add_account("4000", "Revenue", AccountType::Revenue, None, false)
        .unwrap();
    let software = l
        .add_account("6000", "Software", AccountType::Expense, None, false)
        .unwrap();
    let rent = l
        .add_account("6100", "Rent", AccountType::Expense, None, false)
        .unwrap();
    let software_class = l.add_class("Apps").unwrap();

    post(&mut l, d(1, 2), "Owner contribution", cash, capital, 10_000);
    post(&mut l, d(1, 15), "Client payment", cash, revenue, 2_500);
    post(&mut l, d(1, 20), "Rent", rent, cash, 800);
    if close_january {
        let jan = l.create_period("2026-01", d(1, 1), d(1, 31)).unwrap();
        l.close_period(jan).unwrap();
    }
    // Software on the card, tagged to the Apps class.
    let e = l.create_entry(d(2, 3), "Software");
    l.add_line(e, software, Side::Debit, usd(120)).unwrap();
    l.add_line(e, card, Side::Credit, usd(120)).unwrap();
    l.set_line_class(e, 0, Some(software_class)).unwrap();
    l.post_entry(e).unwrap();
    post(&mut l, d(2, 10), "Client payment", cash, revenue, 1_000);
    post(&mut l, d(2, 25), "Pay card", card, cash, 120);
    Books {
        ledger: l,
        cash,
        software_class,
    }
}

fn line(code: &str, name: &str, n: i64) -> StatementLine {
    StatementLine {
        code: code.to_string(),
        name: name.to_string(),
        amount: dec(n),
    }
}

#[test]
fn profit_and_loss_is_scoped_to_the_period() {
    let b = books(false);
    let jan = profit_and_loss_for_period(&b.ledger, d(1, 1), d(1, 31), None).unwrap();
    assert_eq!(jan.revenue, vec![line("4000", "Revenue", 2_500)]);
    assert_eq!(jan.expenses, vec![line("6100", "Rent", 800)]);
    assert_eq!(jan.net_income(), dec(1_700));

    let feb = profit_and_loss_for_period(&b.ledger, d(2, 1), d(2, 28), None).unwrap();
    assert_eq!(feb.revenue, vec![line("4000", "Revenue", 1_000)]);
    assert_eq!(feb.expenses, vec![line("6000", "Software", 120)]);
    assert_eq!(feb.net_income(), dec(880));

    // Both months: 3,500 revenue, 920 expenses, lines sorted by code.
    let both = profit_and_loss_for_period(&b.ledger, d(1, 1), d(2, 28), None).unwrap();
    assert_eq!(
        both.expenses,
        vec![line("6000", "Software", 120), line("6100", "Rent", 800)]
    );
    assert_eq!(
        (both.total_revenue, both.total_expenses),
        (dec(3_500), dec(920))
    );
    // ... which is the life-of-ledger P&L, because nothing is outside it.
    let all = profit_and_loss(&b.ledger).unwrap();
    assert_eq!((all.revenue, all.expenses), (dec(3_500), dec(920)));

    // One class: only the tagged software line.
    let apps =
        profit_and_loss_for_period(&b.ledger, d(1, 1), d(12, 31), Some(b.software_class)).unwrap();
    assert!(apps.revenue.is_empty());
    assert_eq!(apps.expenses, vec![line("6000", "Software", 120)]);

    // A backwards range is rejected, not silently empty.
    assert!(profit_and_loss_for_period(&b.ledger, d(2, 1), d(1, 1), None).is_err());
}

#[test]
fn balance_sheet_is_as_of_the_date() {
    let b = books(false);
    // Before anything happened: empty and balanced.
    let start = balance_sheet_as_of(&b.ledger, d(1, 1)).unwrap();
    assert!(start.assets.is_empty() && start.equity.is_empty());
    assert!(start.balanced());

    // Jan 31: cash 10,000 + 2,500 − 800 = 11,700; equity 10,000 + 1,700.
    let jan = balance_sheet_as_of(&b.ledger, d(1, 31)).unwrap();
    assert_eq!(jan.assets, vec![line("1000", "Cash", 11_700)]);
    assert!(jan.liabilities.is_empty());
    assert_eq!(jan.equity, vec![line("3000", "Capital", 10_000)]);
    assert_eq!(jan.current_earnings, dec(1_700));
    assert_eq!(jan.total_equity, dec(11_700));
    assert!(jan.balanced());

    // Feb 15: cash 12,700; the card owes 120; earnings 1,700 + 880.
    let mid = balance_sheet_as_of(&b.ledger, d(2, 15)).unwrap();
    assert_eq!(mid.total_assets, dec(12_700));
    assert_eq!(mid.liabilities, vec![line("2000", "Card", 120)]);
    assert_eq!(mid.current_earnings, dec(2_580));
    assert_eq!(mid.total_liabilities + mid.total_equity, dec(12_700));

    // Feb 28: card paid off, so it drops off the statement.
    let end = balance_sheet_as_of(&b.ledger, d(2, 28)).unwrap();
    assert_eq!(end.total_assets, dec(12_580));
    assert!(end.liabilities.is_empty());
    assert!(end.balanced());
    // ... and agrees with the life-of-ledger balance sheet.
    let all = balance_sheet(&b.ledger).unwrap();
    assert_eq!(
        (all.assets, all.liabilities, all.equity),
        (end.total_assets, end.total_liabilities, end.total_equity)
    );
}

#[test]
fn a_closed_period_still_reports_its_earnings() {
    let b = books(true);
    // The close posted Dr 4000 2,500 / Cr 6100 800 / Cr 3999 1,700 on Jan
    // 31. The January P&L must not count it: January earned 1,700.
    let jan = profit_and_loss_for_period(&b.ledger, d(1, 1), d(1, 31), None).unwrap();
    assert_eq!(jan.net_income(), dec(1_700));
    assert_eq!(jan.revenue, vec![line("4000", "Revenue", 2_500)]);
    let feb = profit_and_loss_for_period(&b.ledger, d(2, 1), d(2, 28), None).unwrap();
    assert_eq!(feb.net_income(), dec(880));

    // After the close, January's earnings sit in retained earnings, not in
    // current earnings; February's are current.
    let jan_bs = balance_sheet_as_of(&b.ledger, d(1, 31)).unwrap();
    assert_eq!(
        jan_bs.equity,
        vec![
            line("3000", "Capital", 10_000),
            line("3999", "Retained Earnings", 1_700)
        ]
    );
    assert_eq!(jan_bs.current_earnings, Decimal::ZERO);
    assert!(jan_bs.balanced());
    let feb_bs = balance_sheet_as_of(&b.ledger, d(2, 28)).unwrap();
    assert_eq!(feb_bs.current_earnings, dec(880));
    assert_eq!(feb_bs.total_equity, dec(12_580));
    assert!(feb_bs.balanced());

    // The cash flow for January: net income 1,700, and the only financing
    // flow is the owner's 10,000. The close is not an owner contribution.
    let cf = cash_flow(
        &b.ledger,
        &CashFlowParams {
            start: d(1, 1),
            end: d(1, 31),
            cash_accounts: vec![b.cash],
            financing_accounts: vec![],
        },
    )
    .unwrap();
    assert_eq!(cf.net_income, dec(1_700));
    assert_eq!(cf.financing.len(), 1);
    assert_eq!(cf.cash_from_financing, dec(10_000));
    assert_eq!(cf.net_change_in_cash, dec(11_700));
}
