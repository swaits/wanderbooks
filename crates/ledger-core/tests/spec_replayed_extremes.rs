//! Figures derived from what replay accepts: computed exactly where exact
//! arithmetic and chrono's calendar reach, refused (TooLargeToCompute,
//! DateOutOfRange) where they don't, never a panic (rules.md B-28; P0
//! review round 4, 2026-10-17).
//!
//! ```text
//! GOAL: books an earlier version wrote (a draft line of Decimal's
//!       largest, shares and bases past the line limit, a document's lines
//!       or a payroll run as journaled, dates at chrono's last day) can be
//!       read every way the engine reads books, and the server stays up.
//! SOURCE: DECISION (P0 review round 4: posted is forever; derived
//!         computations refuse rather than panic).
//! ```

use chrono::{Datelike, NaiveDate};
use ledger_core::{
    AccountType, Currency, DocumentLine, Ledger, LedgerError, Money, NewAsset, PartyShare,
    PayrollLine, Side,
};
use rust_decimal::Decimal;

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn usd(a: Decimal) -> Money {
    Money::new(a, Currency::USD)
}

fn too_large(r: Result<impl std::fmt::Debug, LedgerError>) -> bool {
    matches!(r, Err(LedgerError::TooLargeToCompute(_)))
}

/// An asset whose life ends in chrono's last month (an earlier version
/// accepted any life whose end chrono holds): its schedule walks into
/// December 262142, prorates that month by its 31 days, and sums to its
/// cost exactly.
#[test]
fn depreciation_reaches_chronos_last_month() {
    let mut l = Ledger::new(Currency::USD);
    let mut add = |code: &str, ty| l.add_account(code, code, ty, None, false).unwrap();
    let cost_account = add("1500", AccountType::Asset);
    let accum = add("1510", AccountType::Asset);
    let expense = add("6100", AccountType::Expense);
    let y = NaiveDate::MAX.year();
    let id = l
        .replay_register_asset(NewAsset {
            description: "edge".into(),
            purchase_date: d(y, 1, 15),
            cost: usd(Decimal::new(120000, 2)),
            salvage_value: usd(Decimal::ZERO),
            useful_life_months: 11,
            cost_account,
            accum_depr_account: accum,
            expense_account: expense,
        })
        .unwrap();
    let a = l.get_asset(id).unwrap();
    let full = ledger_core::depreciation_schedule(a, NaiveDate::MAX);
    assert_eq!(full.len(), 12);
    assert_eq!(full[0].period, format!("{y}-01"));
    assert_eq!(full[11].period, format!("{y}-12"));
    assert_eq!(full[11].accumulated, Decimal::new(120000, 2));
    assert_eq!(
        full.iter().map(|l| l.amount).sum::<Decimal>(),
        Decimal::new(120000, 2)
    );
    // Through November: eleven months, December not yet.
    let nov = ledger_core::depreciation_schedule(a, d(y, 11, 30));
    assert_eq!(nov.len(), 11);
    assert!(nov.last().unwrap().accumulated < Decimal::new(120000, 2));
    // December is walked without stepping past chrono's last day.
    let dec = ledger_core::depreciation_schedule(a, d(y, 12, 10));
    assert_eq!(dec.last().unwrap().period, format!("{y}-12"));
    assert_eq!(
        ledger_core::accumulated_depreciation(a, NaiveDate::MAX),
        Decimal::new(120000, 2)
    );
    assert_eq!(ledger_core::book_value(a, NaiveDate::MAX), Decimal::ZERO);
}

/// An asset whose life runs past 9999 (or starts before year 1) trues up
/// in its final month like any other: its lifetime is exactly its cost.
#[test]
fn a_life_past_9999_trues_up_to_the_cent() {
    let mut l = Ledger::new(Currency::USD);
    let mut add = |code: &str, ty| l.add_account(code, code, ty, None, false).unwrap();
    let cost_account = add("1500", AccountType::Asset);
    let accum = add("1510", AccountType::Asset);
    let expense = add("6100", AccountType::Expense);
    // March 17th to April 17th: a 31-day first month and a 30-day last, so
    // the partial months drift and the true-up is needed.
    for (start, months) in [
        (d(9999, 3, 17), 61),
        (d(-10, 3, 17), 61),
        (d(9998, 3, 17), 25),
    ] {
        let id = l
            .replay_register_asset(NewAsset {
                description: "far".into(),
                purchase_date: start,
                cost: usd(Decimal::new(100_000, 2)),
                salvage_value: usd(Decimal::new(1, 2)),
                useful_life_months: months,
                cost_account,
                accum_depr_account: accum,
                expense_account: expense,
            })
            .unwrap();
        let a = l.get_asset(id).unwrap();
        let s = ledger_core::depreciation_schedule(a, NaiveDate::MAX);
        assert_eq!(
            s.last().unwrap().accumulated,
            Decimal::new(99_999, 2),
            "{start} + {months}"
        );
        assert_eq!(
            s.iter().map(|l| l.amount).sum::<Decimal>(),
            Decimal::new(99_999, 2),
            "{start} + {months}"
        );
    }
}

/// February has 29 days exactly in the years chrono calls leap, from its
/// first year to its last (year 0 and the negative years included), so a
/// schedule in any of them prorates February rightly.
#[test]
fn february_is_prorated_by_chronos_calendar_in_every_year() {
    let mut l = Ledger::new(Currency::USD);
    let mut add = |code: &str, ty| l.add_account(code, code, ty, None, false).unwrap();
    let cost_account = add("1500", AccountType::Asset);
    let accum = add("1510", AccountType::Asset);
    let expense = add("6100", AccountType::Expense);
    let min = NaiveDate::MIN.year();
    let max = NaiveDate::MAX.year();
    for year in [
        min,
        min + 1,
        -400,
        -100,
        -4,
        -1,
        0,
        1,
        4,
        100,
        400,
        1900,
        2000,
        2024,
        2026,
        max - 1,
        max,
    ] {
        let leap = NaiveDate::from_ymd_opt(year, 2, 29).is_some();
        // Bought February 1st, two months of life, so February is a whole
        // month of it.
        let id = l
            .replay_register_asset(NewAsset {
                description: format!("{year}"),
                purchase_date: d(year, 2, 1),
                cost: usd(Decimal::from(2900)),
                salvage_value: usd(Decimal::ZERO),
                useful_life_months: 2,
                cost_account,
                accum_depr_account: accum,
                expense_account: expense,
            })
            .unwrap();
        let a = l.get_asset(id).unwrap();
        // Through the 14th: 14 of February's days.
        let half = ledger_core::depreciation_schedule(a, d(year, 2, 14));
        let days = if leap { 29 } else { 28 };
        let want = (Decimal::from(1450) * Decimal::from(14) / Decimal::from(days))
            .round_dp_with_strategy(2, rust_decimal::RoundingStrategy::MidpointAwayFromZero);
        assert_eq!(half.last().unwrap().amount, want, "{year}");
    }
}

/// A draft an earlier version journaled with lines of Decimal's largest:
/// its money flow and debits are refused, its shares don't resolve, its
/// post refuses it.
#[test]
fn a_draft_of_decimals_largest_is_refused_where_summed() {
    let mut l = Ledger::new(Currency::USD);
    let cash = l
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    let card = l
        .add_account("2050", "Card", AccountType::Liability, None, false)
        .unwrap();
    let rent = l
        .add_account("6000", "Rent", AccountType::Expense, None, false)
        .unwrap();
    let p = l
        .add_party("Acme", &[ledger_core::Role::Contractor])
        .unwrap();
    let q = l
        .add_party("Beta", &[ledger_core::Role::Contractor])
        .unwrap();
    let capital = l
        .add_account("3000", "Capital", AccountType::Equity, None, false)
        .unwrap();
    let member = l
        .add_party("Tobin Harlow", &[ledger_core::Role::Member])
        .unwrap();
    let money = ledger_core::parties::money_accounts(&l, &[cash], &[]);
    let resolve = |l: &Ledger, e, shares: &[PartyShare]| {
        ledger_core::shares::resolve(l, e, shares, &money, &[])
    };
    for (dr, cr) in [(rent, cash), (rent, card), (card, cash), (cash, card)] {
        let e = l.create_entry(d(2026, 3, 1), "huge");
        for side in [Side::Debit, Side::Debit] {
            l.replay_add_line(e, dr, side, usd(Decimal::MAX)).unwrap();
        }
        l.replay_add_line(e, cr, Side::Credit, usd(Decimal::MAX))
            .unwrap();
        l.replay_add_line(e, cr, Side::Credit, usd(Decimal::MAX))
            .unwrap();
        let entry = l.get_entry(e).unwrap();
        assert!(too_large(ledger_core::parties::entry_flow(entry, &money)));
        assert!(too_large(ledger_core::shares::target(
            &l,
            e,
            &[PartyShare::whole(p)],
            &money
        )));
        let why = resolve(&l, e, &[PartyShare::whole(p)]).unwrap_err();
        assert!(why.contains("too large to compute"), "{why}");
        assert_eq!(l.post_entry(e).unwrap_err(), LedgerError::AmountOutOfRange);
    }
    // A member's capital moved past exact arithmetic (rules.md B-35, case
    // 3, B-28): the target is refused, never a panic.
    let e = l.create_entry(d(2026, 3, 1), "huge capital");
    for _ in 0..2 {
        l.replay_add_line(e, cash, Side::Debit, usd(Decimal::MAX))
            .unwrap();
        l.replay_add_line(e, capital, Side::Credit, usd(Decimal::MAX))
            .unwrap();
    }
    assert!(too_large(ledger_core::shares::target(
        &l,
        e,
        &[PartyShare::whole(member)],
        &money
    )));
    // One line of Decimal's largest flows, and its share resolves whole.
    let e = l.create_entry(d(2026, 3, 1), "one huge");
    l.replay_add_line(e, rent, Side::Debit, usd(Decimal::MAX))
        .unwrap();
    l.replay_add_line(e, cash, Side::Credit, usd(Decimal::MAX))
        .unwrap();
    let entry = l.get_entry(e).unwrap();
    assert_eq!(
        ledger_core::parties::entry_flow(entry, &money).unwrap().net,
        Decimal::MAX
    );
    let r = resolve(&l, e, &[PartyShare::whole(p)]).unwrap();
    assert_eq!(
        (r.parts[0].party, r.parts[0].amount, r.parts[0].cash),
        (p, Decimal::MAX, Decimal::MAX)
    );
    // A percent of it: the product is past exact arithmetic.
    let halves = [p, q].map(|party| PartyShare {
        party,
        share: ledger_core::shares::Share::Percent(Decimal::from(50)),
    });
    let why = resolve(&l, e, &halves).unwrap_err();
    assert!(why.contains("too large to compute"), "{why}");
}

/// A document is restored as journaled: its total and balance are exact,
/// or refused past exact arithmetic; aging sums by party, checked.
#[test]
fn documents_and_aging_are_exact_or_refused() {
    let mut l = Ledger::new(Currency::USD);
    let customer = l.add_customer("Client").unwrap();
    let line = |a: Decimal| DocumentLine {
        description: "x".into(),
        amount: a,
    };
    let fine = vec![
        line(Decimal::new(1, 11)),
        line("9.99999999999".parse().unwrap()),
    ];
    l.replay_invoice(1, customer, d(2026, 3, 1), d(2026, 3, 31), fine, 1)
        .unwrap();
    let inv = l.get_invoice(1).unwrap();
    assert_eq!(inv.total(), Ok(Decimal::TEN));
    assert_eq!(inv.balance(), Ok(Decimal::TEN));
    let rows = l.ar_aging(d(2026, 6, 1)).unwrap();
    // 62 days past due on June 1st: 61-90.
    assert_eq!(rows[0].days_90, Decimal::TEN);
    assert_eq!(rows[0].total(), Ok(Decimal::TEN));
    // Two documents of half Decimal's largest each: each is exact, the
    // party's aging bucket is not.
    let half = Decimal::MAX / Decimal::TWO;
    for id in [2, 3] {
        l.replay_invoice(
            id,
            customer,
            d(2026, 3, 1),
            d(2026, 3, 31),
            vec![line(half)],
            id,
        )
        .unwrap();
        assert_eq!(l.get_invoice(id).unwrap().total(), Ok(half));
    }
    assert!(too_large(l.ar_aging(d(2026, 6, 1))));
    // Paid more than its total, as journaled: a negative balance, exact.
    l.replay_invoice_payment(1, Decimal::from(11), 9).unwrap();
    assert_eq!(l.get_invoice(1).unwrap().balance(), Ok(-Decimal::ONE));
    // A total of lines that overflow, and its balance.
    l.replay_invoice(
        4,
        customer,
        d(2026, 3, 1),
        d(2026, 3, 31),
        vec![line(Decimal::MAX), line(Decimal::MAX)],
        4,
    )
    .unwrap();
    assert!(too_large(l.get_invoice(4).unwrap().total()));
    assert!(too_large(l.get_invoice(4).unwrap().balance()));
    // A balance past exact arithmetic: a total of Decimal's largest paid
    // a negative amount.
    l.replay_invoice(
        5,
        customer,
        d(2026, 3, 1),
        d(2026, 3, 31),
        vec![line(Decimal::MAX)],
        5,
    )
    .unwrap();
    l.replay_invoice_payment(5, -Decimal::ONE, 10).unwrap();
    assert!(too_large(l.get_invoice(5).unwrap().balance()));
    // Bills alike.
    let vendor = l.add_vendor("Supplier").unwrap();
    l.replay_bill(
        1,
        vendor,
        d(2026, 3, 1),
        d(2026, 3, 31),
        vec![line(Decimal::MAX), line(Decimal::ONE)],
        6,
    )
    .unwrap();
    assert!(too_large(l.get_bill(1).unwrap().total()));
    assert!(too_large(l.ap_aging(d(2026, 6, 1))));
    let row = ledger_core::AgingRow {
        name: "x".into(),
        current: Decimal::MAX,
        days_30: Decimal::ONE,
        days_60: Decimal::ZERO,
        days_90: Decimal::ZERO,
        days_90_plus: Decimal::ZERO,
    };
    assert!(too_large(row.total()));
    for (i, one) in [
        (0, Decimal::ONE),
        (1, Decimal::TWO),
        (2, Decimal::TEN),
        (3, Decimal::ONE_HUNDRED),
        (4, Decimal::ONE_THOUSAND),
    ] {
        let mut r = ledger_core::AgingRow {
            current: Decimal::ZERO,
            ..row.clone()
        };
        r.days_30 = Decimal::ZERO;
        match i {
            0 => r.current = one,
            1 => r.days_30 = one,
            2 => r.days_60 = one,
            3 => r.days_90 = one,
            _ => r.days_90_plus = one,
        }
        assert_eq!(r.total(), Ok(one), "bucket {i}");
    }
}

/// A payroll run restored as journaled: net pay is exact, or refused past
/// exact arithmetic; a run's plan refuses totals it can't hold.
#[test]
fn payroll_figures_are_exact_or_refused() {
    let line = |gross: Decimal, fed: Decimal| PayrollLine {
        employee_name: "Dana".into(),
        employee: None,
        gross_pay: gross,
        federal_withholding: fed,
        state_withholding: Decimal::new(1, 2),
        ss_employee: Decimal::new(2, 2),
        medicare_employee: Decimal::new(3, 2),
        ss_employer: Decimal::ZERO,
        medicare_employer: Decimal::ZERO,
        other_deductions: Decimal::new(4, 2),
    };
    assert_eq!(
        line(Decimal::from(100), Decimal::new(1, 12)).net_pay(),
        Ok("99.899999999999".parse().unwrap())
    );
    assert!(too_large(line(-Decimal::MAX, Decimal::ONE).net_pay()));
    let accounts = ledger_core::payroll::PayrollAccounts {
        salary_expense: 1,
        payroll_tax_expense: 2,
        federal_payable: 3,
        state_payable: 4,
        ss_payable: 5,
        medicare_payable: 6,
        other_payable: 7,
    };
    let big = line(Decimal::MAX, Decimal::ZERO);
    assert!(too_large(ledger_core::payroll::payroll_entry_plan(
        &[big.clone(), big],
        accounts,
        8,
        Currency::USD
    )));
    let plan = ledger_core::payroll::payroll_entry_plan(
        &[line(Decimal::from(100), Decimal::ONE)],
        accounts,
        8,
        Currency::USD,
    )
    .unwrap();
    let debits: Decimal = plan
        .iter()
        .filter(|(_, s, _)| *s == Side::Debit)
        .map(|(_, _, m)| m.amount)
        .sum();
    let credits: Decimal = plan
        .iter()
        .filter(|(_, s, _)| *s == Side::Credit)
        .map(|(_, _, m)| m.amount)
        .sum();
    assert_eq!(debits, Decimal::from(100));
    assert_eq!(credits, Decimal::from(100));
}
