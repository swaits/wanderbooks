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
    AccountId, AccountType, CashFlowParams, ClassId, Currency, Ledger, LedgerError, MemberCapital,
    Money, Side, StatementLine, cash_flow, general_ledger, members_equity, trial_balance_as_of,
};
use rust_decimal::Decimal;
use std::str::FromStr;

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

/// Cash flow lines come out in account-code order, every time. (The chart
/// iterates in hash order; the statement must not.)
#[test]
fn cash_flow_lines_are_in_code_order() {
    for _ in 0..20 {
        let mut l = Ledger::new(Currency::USD);
        let cash = l
            .add_account("1000", "Cash", AccountType::Asset, None, false)
            .unwrap();
        let mut owners = Vec::new();
        for code in ["3005", "3001", "3009", "3000", "3003"] {
            owners.push(
                l.add_account(code, code, AccountType::Equity, None, false)
                    .unwrap(),
            );
        }
        for o in &owners {
            post(&mut l, d(3, 1), "Contribution", cash, *o, 100);
        }
        let cf = cash_flow(
            &l,
            &CashFlowParams {
                start: d(1, 1),
                end: d(12, 31),
                cash_accounts: vec![cash],
                financing_accounts: vec![],
            },
        )
        .unwrap();
        // Labels read "Owner contributions — 3000 3000" (code, then name).
        let codes: Vec<&str> = cf
            .financing
            .iter()
            .map(|f| f.label.rsplit(' ').next().unwrap())
            .collect();
        assert_eq!(
            codes,
            ["3000", "3001", "3003", "3005", "3009"],
            "{:?}",
            cf.financing
        );
    }
}

fn cents(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

#[test]
fn trial_balance_is_as_of_the_date() {
    let b = books(false);
    // Jan 31: Dr cash 11,700 and rent 800; Cr capital 10,000 and revenue 2,500.
    let tb = trial_balance_as_of(&b.ledger, d(1, 31));
    let rows: Vec<(&str, Decimal, Decimal)> = tb
        .lines
        .iter()
        .map(|l| (l.code.as_str(), l.debit, l.credit))
        .collect();
    assert_eq!(
        rows,
        [
            ("1000", dec(11_700), dec(0)),
            ("3000", dec(0), dec(10_000)),
            ("4000", dec(0), dec(2_500)),
            ("6100", dec(800), dec(0)),
        ]
    );
    assert_eq!(
        (tb.total_debits, tb.total_credits),
        (dec(12_500), dec(12_500))
    );
}

#[test]
fn general_ledger_runs_balances_by_account() {
    let b = books(false);
    let gl = general_ledger(&b.ledger, d(2, 1), d(2, 28)).unwrap();
    let codes: Vec<&str> = gl.accounts.iter().map(|a| a.code.as_str()).collect();
    // Every account with a beginning balance or February activity, by code.
    assert_eq!(codes, ["1000", "2000", "3000", "4000", "6000", "6100"]);

    let cash = &gl.accounts[0];
    assert_eq!(cash.beginning_balance, dec(11_700));
    let p: Vec<(Decimal, Decimal, &str, Decimal)> = cash
        .postings
        .iter()
        .map(|p| (p.debit, p.credit, p.split.as_str(), p.balance))
        .collect();
    assert_eq!(
        p,
        [
            (dec(1_000), dec(0), "4000 Revenue", dec(12_700)),
            (dec(0), dec(120), "2000 Card", dec(12_580)),
        ]
    );
    assert_eq!(cash.ending_balance, dec(12_580));

    // The card: borrowed 120, paid 120, back to zero (credit-normal).
    let card = &gl.accounts[1];
    assert_eq!(card.beginning_balance, dec(0));
    assert_eq!(card.postings[0].balance, dec(120));
    assert_eq!(card.ending_balance, dec(0));
    // Capital: no February activity, still listed with its balance.
    assert!(gl.accounts[2].postings.is_empty());
    assert_eq!(gl.accounts[2].ending_balance, dec(10_000));

    // Every ending balance ties to the trial balance as of the end.
    let tb = trial_balance_as_of(&b.ledger, d(2, 28));
    for a in &gl.accounts {
        let tb_line = tb.lines.iter().find(|l| l.code == a.code);
        let tb_amount = tb_line.map_or(Decimal::ZERO, |l| l.debit - l.credit);
        let normal = match a.account_type {
            AccountType::Asset | AccountType::Expense => a.ending_balance,
            _ => -a.ending_balance,
        };
        assert_eq!(tb_amount, normal, "account {}", a.code);
    }
}

#[test]
fn general_ledger_marks_compound_entries_as_split() {
    let mut l = Ledger::new(Currency::USD);
    let cash = l
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    let card = l
        .add_account("2000", "Card", AccountType::Liability, None, false)
        .unwrap();
    let soft = l
        .add_account("6000", "Software", AccountType::Expense, None, false)
        .unwrap();
    // Dr software 100 / Cr cash 60 / Cr card 40: one debit, split credits.
    let e = l.create_entry(d(3, 1), "Annual license, paid two ways");
    l.add_line(e, soft, Side::Debit, usd(100)).unwrap();
    l.add_line(e, cash, Side::Credit, usd(60)).unwrap();
    l.add_line(e, card, Side::Credit, usd(40)).unwrap();
    l.post_entry(e).unwrap();
    let gl = general_ledger(&l, d(1, 1), d(12, 31)).unwrap();
    let split_of = |code: &str| {
        let a = gl.accounts.iter().find(|a| a.code == code).unwrap();
        (a.postings[0].split.clone(), a.postings[0].entry_id)
    };
    // The debit side has two other accounts; each credit has one.
    assert_eq!(split_of("6000"), ("-Split-".to_string(), e));
    assert_eq!(split_of("1000"), ("6000 Software".to_string(), e));
    assert_eq!(split_of("2000"), ("6000 Software".to_string(), e));
}

/// Two members, 50/50: Alice and Bob. January: Alice puts in 600, Bob 400;
/// revenue 1,000; expense 333.33, so net income 666.67, which doesn't split
/// evenly. February: Bob takes a 100 distribution; revenue 100.01.
fn partnership(close_january: bool) -> (Ledger, Vec<MemberCapital>) {
    let mut l = Ledger::new(Currency::USD);
    let cash = l
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    let alice = l
        .add_account("3000", "Capital — Alice", AccountType::Equity, None, false)
        .unwrap();
    let bob = l
        .add_account("3001", "Capital — Bob", AccountType::Equity, None, false)
        .unwrap();
    let rev = l
        .add_account("4000", "Revenue", AccountType::Revenue, None, false)
        .unwrap();
    let exp = l
        .add_account("6000", "Expense", AccountType::Expense, None, false)
        .unwrap();
    let money = |s: &str| Money::new(cents(s), Currency::USD);
    let post = |l: &mut Ledger, date, dr, cr, amt: &str| {
        let e = l.create_entry(date, "x");
        l.add_line(e, dr, Side::Debit, money(amt)).unwrap();
        l.add_line(e, cr, Side::Credit, money(amt)).unwrap();
        l.post_entry(e).unwrap();
    };
    post(&mut l, d(1, 2), cash, alice, "600");
    post(&mut l, d(1, 2), cash, bob, "400");
    post(&mut l, d(1, 10), cash, rev, "1000");
    post(&mut l, d(1, 20), exp, cash, "333.33");
    if close_january {
        let jan = l.create_period("2026-01", d(1, 1), d(1, 31)).unwrap();
        l.close_period(jan).unwrap();
    }
    post(&mut l, d(2, 5), bob, cash, "100");
    post(&mut l, d(2, 9), cash, rev, "100.01");
    let half = cents("0.5");
    let members = vec![
        MemberCapital {
            name: "Alice".into(),
            capital_account: alice,
            distribution_account: None,
            share: half,
        },
        MemberCapital {
            name: "Bob".into(),
            capital_account: bob,
            distribution_account: None,
            share: half,
        },
    ];
    (l, members)
}

#[test]
fn members_equity_rolls_capital_forward_and_ties_to_the_balance_sheet() {
    for close in [false, true] {
        let (l, members) = partnership(close);
        // January: 666.67 splits 333.34 / 333.33 (half a cent rounds up for
        // Alice; Bob gets the remainder so the total is exact).
        let jan = members_equity(&l, d(1, 1), d(1, 31), &members).unwrap();
        let row = |c: &ledger_core::MemberEquityColumn| {
            (
                c.beginning,
                c.contributions,
                c.net_income,
                c.distributions,
                c.ending,
            )
        };
        assert_eq!(
            row(&jan.members[0]),
            (
                cents("0"),
                cents("600"),
                cents("333.34"),
                cents("0"),
                cents("933.34")
            )
        );
        assert_eq!(
            row(&jan.members[1]),
            (
                cents("0"),
                cents("400"),
                cents("333.33"),
                cents("0"),
                cents("733.33")
            )
        );
        assert_eq!(jan.total.ending, cents("1666.67"));
        assert_eq!(
            jan.total.ending,
            balance_sheet_as_of(&l, d(1, 31)).unwrap().total_equity
        );

        // February begins where January ended, whether or not January was
        // closed to retained earnings. 100.01 splits 50.01 / 50.00.
        let feb = members_equity(&l, d(2, 1), d(2, 28), &members).unwrap();
        assert_eq!(
            row(&feb.members[0]),
            (
                cents("933.34"),
                cents("0"),
                cents("50.01"),
                cents("0"),
                cents("983.35")
            )
        );
        assert_eq!(
            row(&feb.members[1]),
            (
                cents("733.33"),
                cents("0"),
                cents("50.00"),
                cents("100"),
                cents("683.33")
            )
        );
        assert_eq!(feb.total.ending, cents("1666.68"));
        assert_eq!(
            feb.total.ending,
            balance_sheet_as_of(&l, d(2, 28)).unwrap().total_equity
        );
        assert!(
            feb.members.iter().all(|m| m.other.is_zero()),
            "close={close}"
        );
    }
}

#[test]
fn members_must_be_configured_correctly() {
    let (l, mut members) = partnership(false);
    members[1].share = cents("0.4");
    assert!(matches!(
        members_equity(&l, d(1, 1), d(1, 31), &members),
        Err(LedgerError::InvalidMembers(_))
    ));
    members[1].share = cents("0.5");
    members[1].capital_account = members[0].capital_account;
    assert!(matches!(
        members_equity(&l, d(1, 1), d(1, 31), &members),
        Err(LedgerError::InvalidMembers(_))
    ));
}

#[test]
fn cash_flow_lines_name_their_account() {
    let (l, _) = partnership(false);
    let cf = cash_flow(
        &l,
        &CashFlowParams {
            start: d(1, 1),
            end: d(12, 31),
            cash_accounts: vec![1],
            financing_accounts: vec![],
        },
    )
    .unwrap();
    let accounts: Vec<Option<&str>> = cf.financing.iter().map(|f| f.account.as_deref()).collect();
    assert_eq!(accounts, [Some("3000"), Some("3001")]);
}

#[test]
fn a_close_sweeps_only_through_the_period_end() {
    // February is already posted when January closes: the close must take
    // January's 2,500 revenue and 800 rent (net 1,700) and leave
    // February's 1,000 revenue and 120 software open.
    let mut b = books(false);
    let jan = b
        .ledger
        .create_period("2026-01", d(1, 1), d(1, 31))
        .unwrap();
    let closing = b.ledger.close_period(jan).unwrap().unwrap();
    let lines: Vec<(String, Side, Decimal)> = b
        .ledger
        .get_entry(closing)
        .unwrap()
        .lines()
        .iter()
        .map(|l| {
            (
                b.ledger.chart().get(l.account).unwrap().code.clone(),
                l.side,
                l.amount.amount,
            )
        })
        .collect();
    assert_eq!(
        lines,
        [
            ("4000".to_string(), Side::Debit, dec(2_500)),
            ("6100".to_string(), Side::Credit, dec(800)),
            ("3999".to_string(), Side::Credit, dec(1_700)),
        ]
    );
    let feb = profit_and_loss_for_period(&b.ledger, d(2, 1), d(2, 28), None).unwrap();
    assert_eq!(feb.net_income(), dec(880));
    let bs = balance_sheet_as_of(&b.ledger, d(2, 28)).unwrap();
    assert_eq!(bs.current_earnings, dec(880));
    assert_eq!(
        bs.equity.last().unwrap(),
        &line("3999", "Retained Earnings", 1_700)
    );
    assert!(bs.balanced());
}

#[test]
fn periods_close_in_order() {
    let mut b = books(false);
    let jan = b
        .ledger
        .create_period("2026-01", d(1, 1), d(1, 31))
        .unwrap();
    let feb = b
        .ledger
        .create_period("2026-02", d(2, 1), d(2, 28))
        .unwrap();
    b.ledger.close_period(feb).unwrap();
    // February swept everything through Feb 28, January included.
    assert!(matches!(
        b.ledger.close_period(jan),
        Err(LedgerError::CloseOutOfOrder(j, f)) if j == jan && f == feb
    ));
}

#[test]
fn a_partnership_closes_into_members_capital() {
    let (mut l, members) = partnership(false);
    let to = ledger_core::CloseTo::Members(
        members
            .iter()
            .map(|m| ledger_core::MemberClose {
                capital: m.capital_account,
                share: m.share,
                distributions: None,
            })
            .collect(),
    );
    let jan = l.create_period("2026-01", d(1, 1), d(1, 31)).unwrap();
    l.close_period_to(jan, &to).unwrap();
    // January earned 666.67: Alice's capital gets 333.34, Bob's 333.33,
    // and there is no Retained Earnings account at all.
    assert!(l.chart().find_by_code("3999").is_none());
    let bs = balance_sheet_as_of(&l, d(1, 31)).unwrap();
    let capital: Vec<Decimal> = bs.equity.iter().map(|e| e.amount).collect();
    assert_eq!(capital, [cents("933.34"), cents("733.33")]);
    assert_eq!(bs.current_earnings, Decimal::ZERO);

    // The statement still reads: contributions are contributions (the
    // close is income, not a contribution), and February starts where
    // January ended.
    let row = |c: &ledger_core::MemberEquityColumn| {
        (
            c.beginning,
            c.contributions,
            c.net_income,
            c.distributions,
            c.ending,
        )
    };
    let jan_me = members_equity(&l, d(1, 1), d(1, 31), &members).unwrap();
    assert_eq!(
        row(&jan_me.members[0]),
        (
            cents("0"),
            cents("600"),
            cents("333.34"),
            cents("0"),
            cents("933.34")
        )
    );
    assert_eq!(
        row(&jan_me.members[1]),
        (
            cents("0"),
            cents("400"),
            cents("333.33"),
            cents("0"),
            cents("733.33")
        )
    );
    let feb_me = members_equity(&l, d(2, 1), d(2, 28), &members).unwrap();
    assert_eq!(feb_me.members[0].beginning, cents("933.34"));
    assert_eq!(feb_me.members[1].ending, cents("683.33"));
    assert_eq!(
        feb_me.total.ending,
        balance_sheet_as_of(&l, d(2, 28)).unwrap().total_equity
    );

    // Shares must still add up.
    let feb = l.create_period("2026-02", d(2, 1), d(2, 28)).unwrap();
    let bad = ledger_core::CloseTo::Members(vec![ledger_core::MemberClose {
        capital: members[0].capital_account,
        share: cents("0.5"),
        distributions: None,
    }]);
    assert!(matches!(
        l.close_period_to(feb, &bad),
        Err(LedgerError::InvalidMembers(_))
    ));
}

fn entry_lines(l: &Ledger, id: ledger_core::EntryId) -> Vec<(String, Side, Decimal)> {
    l.get_entry(id)
        .unwrap()
        .lines()
        .iter()
        .map(|x| {
            (
                l.chart().get(x.account).unwrap().code.clone(),
                x.side,
                x.amount.amount,
            )
        })
        .collect()
}

#[test]
fn a_sole_proprietor_closes_net_income_and_draws_into_capital() {
    let mut l = Ledger::new(Currency::USD);
    let cash = l
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    let capital = l
        .add_account("3000", "Owner's Capital", AccountType::Equity, None, false)
        .unwrap();
    let draws = l
        .add_account("3100", "Owner's Draws", AccountType::Equity, None, false)
        .unwrap();
    let sales = l
        .add_account("4000", "Sales", AccountType::Revenue, None, false)
        .unwrap();
    let rent = l
        .add_account("6000", "Rent", AccountType::Expense, None, false)
        .unwrap();
    post(&mut l, d(1, 2), "Owner invests", cash, capital, 5_000);
    post(&mut l, d(1, 10), "Sales", cash, sales, 3_000);
    post(&mut l, d(1, 15), "Rent", rent, cash, 1_000);
    post(&mut l, d(1, 20), "Owner draw", draws, cash, 500);
    let jan = l.create_period("2026-01", d(1, 1), d(1, 31)).unwrap();
    let to = ledger_core::CloseTo::Account {
        into: capital,
        sweep: vec![draws],
    };
    let closing = l.close_period_to(jan, &to).unwrap().unwrap();
    // Net 2,000 to capital; the 500 of draws zeroed into capital.
    assert_eq!(
        entry_lines(&l, closing),
        [
            ("4000".to_string(), Side::Debit, dec(3_000)),
            ("6000".to_string(), Side::Credit, dec(1_000)),
            ("3000".to_string(), Side::Credit, dec(2_000)),
            ("3100".to_string(), Side::Credit, dec(500)),
            ("3000".to_string(), Side::Debit, dec(500)),
        ]
    );
    let bs = balance_sheet_as_of(&l, d(1, 31)).unwrap();
    // 5,000 + 2,000 − 500 = 6,500, which is exactly the cash.
    assert_eq!(bs.equity, vec![line("3000", "Owner's Capital", 6_500)]);
    assert_eq!(bs.total_assets, dec(6_500));
    assert_eq!(bs.current_earnings, Decimal::ZERO);
}

#[test]
fn a_corporation_closes_dividends_into_retained_earnings() {
    let mut l = Ledger::new(Currency::USD);
    let cash = l
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    let stock = l
        .add_account("3000", "Common Stock", AccountType::Equity, None, false)
        .unwrap();
    let dividends = l
        .add_account("3200", "Dividends Paid", AccountType::Equity, None, false)
        .unwrap();
    let sales = l
        .add_account("4000", "Sales", AccountType::Revenue, None, false)
        .unwrap();
    post(&mut l, d(1, 2), "Stock issued", cash, stock, 1_000);
    post(&mut l, d(1, 10), "Sales", cash, sales, 900);
    post(&mut l, d(1, 25), "Dividend", dividends, cash, 300);
    let jan = l.create_period("2026-01", d(1, 1), d(1, 31)).unwrap();
    let to = ledger_core::CloseTo::RetainedEarnings {
        sweep: vec![dividends],
    };
    l.close_period_to(jan, &to).unwrap();
    let bs = balance_sheet_as_of(&l, d(1, 31)).unwrap();
    // Retained earnings: 900 earned − 300 paid out.
    assert_eq!(
        bs.equity,
        vec![
            line("3000", "Common Stock", 1_000),
            line("3999", "Retained Earnings", 600)
        ]
    );
    assert!(bs.balanced());
}

#[test]
fn partner_distribution_accounts_roll_into_capital() {
    let mut l = Ledger::new(Currency::USD);
    let cash = l
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    let cap_a = l
        .add_account("3000", "Capital — Alice", AccountType::Equity, None, false)
        .unwrap();
    let cap_b = l
        .add_account("3001", "Capital — Bob", AccountType::Equity, None, false)
        .unwrap();
    let dist_a = l
        .add_account(
            "3100",
            "Distributions — Alice",
            AccountType::Equity,
            None,
            false,
        )
        .unwrap();
    let dist_b = l
        .add_account(
            "3101",
            "Distributions — Bob",
            AccountType::Equity,
            None,
            false,
        )
        .unwrap();
    let sales = l
        .add_account("4000", "Sales", AccountType::Revenue, None, false)
        .unwrap();
    post(&mut l, d(1, 2), "Alice invests", cash, cap_a, 100);
    post(&mut l, d(1, 2), "Bob invests", cash, cap_b, 100);
    post(&mut l, d(1, 10), "Sales", cash, sales, 50);
    post(&mut l, d(1, 20), "Bob's distribution", dist_b, cash, 20);
    let half = cents("0.5");
    let members = vec![
        MemberCapital {
            name: "Alice".into(),
            capital_account: cap_a,
            distribution_account: Some(dist_a),
            share: half,
        },
        MemberCapital {
            name: "Bob".into(),
            capital_account: cap_b,
            distribution_account: Some(dist_b),
            share: half,
        },
    ];
    // January: Bob's distribution comes from his distributions account.
    let jan_me = members_equity(&l, d(1, 1), d(1, 31), &members).unwrap();
    let row = |c: &ledger_core::MemberEquityColumn| {
        (c.contributions, c.net_income, c.distributions, c.ending)
    };
    assert_eq!(
        row(&jan_me.members[0]),
        (dec(100), dec(25), dec(0), dec(125))
    );
    assert_eq!(
        row(&jan_me.members[1]),
        (dec(100), dec(25), dec(20), dec(105))
    );
    assert_eq!(
        jan_me.total.ending,
        balance_sheet_as_of(&l, d(1, 31)).unwrap().total_equity
    );

    // The close sweeps Bob's distributions into his capital.
    let jan = l.create_period("2026-01", d(1, 1), d(1, 31)).unwrap();
    let to = ledger_core::CloseTo::Members(
        members
            .iter()
            .map(|m| ledger_core::MemberClose {
                capital: m.capital_account,
                share: m.share,
                distributions: m.distribution_account,
            })
            .collect(),
    );
    l.close_period_to(jan, &to).unwrap();
    let bs = balance_sheet_as_of(&l, d(1, 31)).unwrap();
    assert_eq!(
        bs.equity,
        vec![
            line("3000", "Capital — Alice", 125),
            line("3001", "Capital — Bob", 105)
        ]
    );
    // February starts there.
    let feb_me = members_equity(&l, d(2, 1), d(2, 28), &members).unwrap();
    assert_eq!(
        (feb_me.members[0].beginning, feb_me.members[1].beginning),
        (dec(125), dec(105))
    );

    // A sweep account must be equity.
    let mut l2 = Ledger::new(Currency::USD);
    let c = l2
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    let e = l2
        .add_account("3000", "Capital", AccountType::Equity, None, false)
        .unwrap();
    let p = l2.create_period("2026-01", d(1, 1), d(1, 31)).unwrap();
    let bad = ledger_core::CloseTo::Account {
        into: e,
        sweep: vec![c],
    };
    assert!(matches!(
        l2.close_period_to(p, &bad),
        Err(LedgerError::InvalidMembers(_))
    ));
}

// ---------------------------------------------------------------------------
// Splitting odd cents among members: largest remainder (Stephen,
// 2026-10-13). Spec: periods::allocate.
// ---------------------------------------------------------------------------

fn split(amount: &str, shares: &[&str]) -> Vec<Decimal> {
    let shares: Vec<Decimal> = shares.iter().map(|s| cents(s)).collect();
    ledger_core::periods::allocate(cents(amount), &shares)
}

#[test]
fn an_odd_cent_goes_to_the_largest_remainder_not_the_last_member() {
    // Three equal partners: the cent goes to the first (ties to the
    // earlier), where the old rule always handed it to the last.
    assert_eq!(
        split("100.00", &["0.3333333333333333333333333333"; 3]),
        [cents("33.34"), cents("33.33"), cents("33.33")]
    );
    // Unequal shares: 6.666... has the larger remainder.
    assert_eq!(
        split(
            "10.00",
            &[
                "0.6666666666666666666666666667",
                "0.3333333333333333333333333333"
            ]
        ),
        [cents("6.67"), cents("3.33")]
    );
    // Equal remainders (0.035 each): ties go to the earlier member.
    assert_eq!(
        split("0.07", &["0.5", "0.5"]),
        [cents("0.04"), cents("0.03")]
    );
    // 0.015 and 0.035: half a cent each, a tie again.
    assert_eq!(
        split("0.05", &["0.3", "0.7"]),
        [cents("0.02"), cents("0.03")]
    );
    // 0.003 and 0.007: the later member's remainder is larger, so the
    // cent is theirs.
    assert_eq!(
        split("0.01", &["0.3", "0.7"]),
        [cents("0.00"), cents("0.01")]
    );
    // One cent, two ways: someone gets it, nobody gets a fraction.
    assert_eq!(
        split("0.01", &["0.5", "0.5"]),
        [cents("0.01"), cents("0.00")]
    );
}

#[test]
fn a_loss_splits_as_the_negation_of_the_same_gain() {
    assert_eq!(
        split("-100.00", &["0.3333333333333333333333333333"; 3]),
        [cents("-33.34"), cents("-33.33"), cents("-33.33")]
    );
}

#[test]
fn closes_journaled_under_the_old_rule_replay_with_it() {
    // Contract: a close made before 2026-10-13 (last member absorbs) must
    // replay to the same lines forever.
    let shares = [cents("0.3333333333333333333333333333"); 3];
    assert_eq!(
        ledger_core::periods::allocate_last_absorbs(cents("100.00"), &shares),
        [cents("33.33"), cents("33.33"), cents("33.34")]
    );
}

// ---------------------------------------------------------------------------
// Boundaries, signs, and labels found untested by mutation testing
// (cargo-mutants on src/reports.rs). Expected values are hand-computed.
// ---------------------------------------------------------------------------

#[test]
fn the_balance_checks_catch_an_imbalance() {
    let b = books(false);
    let mut tb = trial_balance_as_of(&b.ledger, d(2, 28));
    assert!(tb.balanced());
    tb.total_credits += cents("0.01");
    assert!(!tb.balanced(), "a one-cent difference is out of balance");

    let mut bs = balance_sheet(&b.ledger).unwrap();
    assert!(bs.balanced());
    bs.liabilities += cents("0.01");
    assert!(!bs.balanced());

    let mut bss = balance_sheet_as_of(&b.ledger, d(2, 28)).unwrap();
    assert!(bss.balanced());
    bss.total_assets -= cents("0.01");
    assert!(!bss.balanced());
}

#[test]
fn a_one_day_general_ledger_is_valid_and_a_backwards_one_is_not() {
    let b = books(false);
    // Jan 15 only: cash begins at the Jan 2 contribution, then the 2,500
    // client payment lands.
    let gl = general_ledger(&b.ledger, d(1, 15), d(1, 15)).unwrap();
    let cash = gl.accounts.iter().find(|a| a.code == "1000").unwrap();
    assert_eq!(cash.beginning_balance, dec(10_000));
    assert_eq!(cash.postings.len(), 1);
    assert_eq!(cash.ending_balance, dec(12_500));
    assert!(matches!(
        general_ledger(&b.ledger, d(1, 16), d(1, 15)),
        Err(LedgerError::InvalidDateRange { .. })
    ));
}

fn cf_params(start: NaiveDate, end: NaiveDate, cash: AccountId) -> CashFlowParams {
    CashFlowParams {
        start,
        end,
        cash_accounts: vec![cash],
        financing_accounts: vec![],
    }
}

fn reconciles(cf: &ledger_core::CashFlowStatement) {
    // B-14 / cash_flow INVARIANT 1: the three sections sum to the change in
    // cash, exactly.
    assert_eq!(
        cf.cash_from_operations + cf.cash_from_investing + cf.cash_from_financing,
        cf.net_change_in_cash
    );
    assert_eq!(cf.cash_ending - cf.cash_beginning, cf.net_change_in_cash);
}

#[test]
fn cash_flow_starts_counting_on_the_start_date() {
    let b = books(false);
    // A one-day period is a valid range. The Jan 15 payment is in it, not
    // in the beginning balance.
    let cf = cash_flow(&b.ledger, &cf_params(d(1, 15), d(1, 15), b.cash)).unwrap();
    assert_eq!(cf.cash_beginning, dec(10_000));
    assert_eq!(cf.cash_ending, dec(12_500));
    assert_eq!(cf.net_income, dec(2_500));
    reconciles(&cf);
    // Backwards is refused.
    assert!(matches!(
        cash_flow(&b.ledger, &cf_params(d(1, 16), d(1, 15), b.cash)),
        Err(LedgerError::InvalidDateRange { .. })
    ));
}

#[test]
fn cash_flow_labels_say_which_way_each_balance_moved() {
    let mut l = Ledger::new(Currency::USD);
    let cash = l
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    let ar = l
        .add_account("1100", "Receivable", AccountType::Asset, None, false)
        .unwrap();
    let ap = l
        .add_account("2000", "Payable", AccountType::Liability, None, false)
        .unwrap();
    let loan = l
        .add_account("2500", "Loan", AccountType::Liability, None, false)
        .unwrap();
    let capital = l
        .add_account("3000", "Capital", AccountType::Equity, None, false)
        .unwrap();
    let rev = l
        .add_account("4000", "Revenue", AccountType::Revenue, None, false)
        .unwrap();
    let exp = l
        .add_account("6000", "Expense", AccountType::Expense, None, false)
        .unwrap();
    // January: every account starts February with a balance.
    post(&mut l, d(1, 2), "Contribution", cash, capital, 10_000);
    post(&mut l, d(1, 3), "Invoice", ar, rev, 1_000);
    post(&mut l, d(1, 4), "Bill", exp, ap, 500);
    post(&mut l, d(1, 5), "Borrow", cash, loan, 2_000);
    // February: every balance goes up.
    post(&mut l, d(2, 3), "Invoice", ar, rev, 100);
    post(&mut l, d(2, 4), "Bill", exp, ap, 50);
    post(&mut l, d(2, 5), "Borrow", cash, loan, 300);
    post(&mut l, d(2, 6), "Contribution", cash, capital, 400);
    // March: every balance comes back down.
    post(&mut l, d(3, 3), "Collect", cash, ar, 100);
    post(&mut l, d(3, 4), "Pay bill", ap, cash, 50);
    post(&mut l, d(3, 5), "Repay", loan, cash, 300);
    post(&mut l, d(3, 6), "Distribution", capital, cash, 400);

    let lines = |v: &[ledger_core::CashFlowLine]| -> Vec<(String, Decimal)> {
        v.iter().map(|l| (l.label.clone(), l.amount)).collect()
    };
    let s = |x: &str| x.to_string();
    let params = |start, end| CashFlowParams {
        start,
        end,
        cash_accounts: vec![cash],
        financing_accounts: vec![loan],
    };

    // February: net income 100 - 50 = 50; receivable up 100 (cash out),
    // payable up 50 (cash in): operations 0. Borrowed 300, contributed 400.
    let feb = cash_flow(&l, &params(d(2, 1), d(2, 28))).unwrap();
    assert_eq!(feb.net_income, dec(50));
    assert_eq!(
        lines(&feb.working_capital),
        [
            (s("Increase in 1100 Receivable"), dec(-100)),
            (s("Increase in 2000 Payable"), dec(50)),
        ]
    );
    assert_eq!(
        lines(&feb.financing),
        [
            (s("Proceeds from 2500 Loan"), dec(300)),
            (s("Owner contributions — 3000 Capital"), dec(400)),
        ]
    );
    assert_eq!(feb.cash_from_operations, dec(0));
    assert_eq!(feb.net_change_in_cash, dec(700));
    reconciles(&feb);

    // March: the mirror image. Collected 100, paid 50, repaid 300,
    // distributed 400.
    let mar = cash_flow(&l, &params(d(3, 1), d(3, 31))).unwrap();
    assert_eq!(mar.net_income, dec(0));
    assert_eq!(
        lines(&mar.working_capital),
        [
            (s("Decrease in 1100 Receivable"), dec(100)),
            (s("Decrease in 2000 Payable"), dec(-50)),
        ]
    );
    assert_eq!(
        lines(&mar.financing),
        [
            (s("Repayment of 2500 Loan"), dec(-300)),
            (s("Owner distributions — 3000 Capital"), dec(-400)),
        ]
    );
    assert_eq!(mar.net_change_in_cash, dec(-650));
    reconciles(&mar);
}

#[test]
fn a_loss_on_disposal_moves_out_of_operating_into_investing() {
    let mut l = Ledger::new(Currency::USD);
    let cash = l
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    let equipment = l
        .add_account("1500", "Equipment", AccountType::Asset, None, false)
        .unwrap();
    let accum = l
        .add_account("1501", "Accum Depr", AccountType::Asset, None, false)
        .unwrap();
    let capital = l
        .add_account("3000", "Capital", AccountType::Equity, None, false)
        .unwrap();
    let depr = l
        .add_account("6500", "Depreciation", AccountType::Expense, None, false)
        .unwrap();
    let loss = l
        .add_account(
            "6900",
            "Loss on Disposal",
            AccountType::Expense,
            None,
            false,
        )
        .unwrap();
    l.register_asset(ledger_core::NewAsset {
        description: "Laptop".to_string(),
        purchase_date: d(1, 10),
        cost: usd(1_000),
        salvage_value: usd(0),
        useful_life_months: 10,
        cost_account: equipment,
        accum_depr_account: accum,
        expense_account: depr,
    })
    .unwrap();
    post(&mut l, d(1, 5), "Contribution", cash, capital, 5_000);
    post(&mut l, d(1, 10), "Buy laptop", equipment, cash, 1_000);
    post(&mut l, d(1, 31), "Depreciation", depr, accum, 100);
    // Sold for 600 at a book value of 900: a 300 loss.
    let e = l.create_entry(d(2, 15), "Sell laptop");
    l.add_line(e, cash, Side::Debit, usd(600)).unwrap();
    l.add_line(e, accum, Side::Debit, usd(100)).unwrap();
    l.add_line(e, loss, Side::Debit, usd(300)).unwrap();
    l.add_line(e, equipment, Side::Credit, usd(1_000)).unwrap();
    l.post_entry(e).unwrap();

    let lines = |v: &[ledger_core::CashFlowLine]| -> Vec<(String, Decimal)> {
        v.iter().map(|l| (l.label.clone(), l.amount)).collect()
    };
    // February: net income -300; the loss is added back (positive, it used
    // no cash), so operations is 0; the 600 proceeds are investing.
    let feb = cash_flow(&l, &cf_params(d(2, 1), d(2, 28), cash)).unwrap();
    assert_eq!(feb.net_income, dec(-300));
    assert_eq!(
        lines(&feb.working_capital),
        [(
            "Less: gain on sale of fixed assets (in investing)".to_string(),
            dec(300)
        )]
    );
    assert_eq!(feb.cash_from_operations, dec(0));
    assert_eq!(
        lines(&feb.investing),
        [("Sale of fixed assets — 1000 Cash".to_string(), dec(600))]
    );
    assert_eq!(feb.net_change_in_cash, dec(600));
    reconciles(&feb);
}

#[test]
fn a_contribution_on_the_start_date_is_a_contribution_not_beginning_capital() {
    let (l, members) = partnership(false);
    // The partnership fixture's contributions are dated Jan 2. Starting the
    // statement that day must give the same rows as all of January.
    let me = members_equity(&l, d(1, 2), d(1, 31), &members).unwrap();
    let row = |c: &ledger_core::MemberEquityColumn| (c.beginning, c.contributions, c.ending);
    assert_eq!(
        row(&me.members[0]),
        (cents("0"), cents("600"), cents("933.34"))
    );
    assert_eq!(
        row(&me.members[1]),
        (cents("0"), cents("400"), cents("733.33"))
    );
}

#[test]
fn distribution_accounts_reduce_capital_and_returns_reduce_distributions() {
    let mut l = Ledger::new(Currency::USD);
    let cash = l
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    let cap_a = l
        .add_account("3000", "Capital — Alice", AccountType::Equity, None, false)
        .unwrap();
    let cap_b = l
        .add_account("3001", "Capital — Bob", AccountType::Equity, None, false)
        .unwrap();
    let dist_a = l
        .add_account("3100", "Dist — Alice", AccountType::Equity, None, false)
        .unwrap();
    let dist_b = l
        .add_account("3101", "Dist — Bob", AccountType::Equity, None, false)
        .unwrap();
    post(&mut l, d(1, 2), "Alice invests", cash, cap_a, 100);
    post(&mut l, d(1, 2), "Bob invests", cash, cap_b, 100);
    post(&mut l, d(1, 20), "Bob's distribution", dist_b, cash, 20);
    // February: Bob takes 30 and hands 10 back: 20 distributed, net.
    post(&mut l, d(2, 5), "Bob's distribution", dist_b, cash, 30);
    post(&mut l, d(2, 6), "Bob returns some", cash, dist_b, 10);
    let half = cents("0.5");
    let members = vec![
        MemberCapital {
            name: "Alice".into(),
            capital_account: cap_a,
            distribution_account: Some(dist_a),
            share: half,
        },
        MemberCapital {
            name: "Bob".into(),
            capital_account: cap_b,
            distribution_account: Some(dist_b),
            share: half,
        },
    ];
    // No close: Bob's January distribution is still in his distributions
    // account, and it comes off his capital, not the partnership's.
    let feb = members_equity(&l, d(2, 1), d(2, 28), &members).unwrap();
    let row = |c: &ledger_core::MemberEquityColumn| {
        (c.beginning, c.contributions, c.distributions, c.ending)
    };
    assert_eq!(row(&feb.members[0]), (dec(100), dec(0), dec(0), dec(100)));
    assert_eq!(row(&feb.members[1]), (dec(80), dec(0), dec(20), dec(60)));
    assert_eq!(
        feb.total.ending,
        balance_sheet_as_of(&l, d(2, 28)).unwrap().total_equity
    );
    assert!(feb.members.iter().all(|m| m.other.is_zero()));
}
