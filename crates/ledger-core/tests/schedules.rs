//! Amortization schedules and recurring entries. Numbers computed by hand.

use chrono::NaiveDate;
use ledger_core::reports::{balance_sheet_as_of, profit_and_loss_for_period};
use ledger_core::{
    AccountType, Currency, EntryStatus, Frequency, Ledger, LedgerError, Money, NewRecurring,
    NewSchedule, ScheduleKind, Side,
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
    cash: u64,
    prepaid: u64,
    deferred: u64,
    insurance: u64,
    revenue: u64,
    rent: u64,
}

fn books() -> Books {
    let mut l = Ledger::new(Currency::USD);
    let mut a = |code: &str, name: &str, t| l.add_account(code, name, t, None, false).unwrap();
    let cash = a("1000", "Cash", AccountType::Asset);
    let prepaid = a("1300", "Prepaid Expenses", AccountType::Asset);
    let deferred = a("2400", "Deferred Revenue", AccountType::Liability);
    let insurance = a("6300", "Insurance", AccountType::Expense);
    let revenue = a("4000", "Subscription Revenue", AccountType::Revenue);
    let rent = a("6600", "Rent", AccountType::Expense);
    Books {
        l,
        cash,
        prepaid,
        deferred,
        insurance,
        revenue,
        rent,
    }
}

fn post(l: &mut Ledger, date: NaiveDate, dr: u64, cr: u64, amt: &str) {
    let e = l.create_entry(date, "x");
    l.add_line(e, dr, Side::Debit, usd(amt)).unwrap();
    l.add_line(e, cr, Side::Credit, usd(amt)).unwrap();
    l.post_entry(e).unwrap();
}

#[test]
fn a_prepaid_expense_is_expensed_month_by_month() {
    let mut b = books();
    // A year of insurance, paid March 15: an asset, not an expense yet.
    post(&mut b.l, d(3, 15), b.prepaid, b.cash, "1200");
    let id =
        b.l.register_schedule(NewSchedule {
            description: "Liability insurance".into(),
            kind: ScheduleKind::PrepaidExpense,
            total: usd("1200"),
            start: d(3, 15),
            months: 12,
            balance_account: b.prepaid,
            pl_account: b.insurance,
        })
        .unwrap();
    // Through May: March, April, May, at month end, 100 each.
    let due: Vec<_> = b.l.get_schedule(id).unwrap().due(d(5, 31));
    assert_eq!(
        due,
        [
            (d(3, 31), dec("100")),
            (d(4, 30), dec("100")),
            (d(5, 31), dec("100"))
        ]
    );
    for (month, _) in due {
        b.l.post_schedule_month(id, month).unwrap();
    }
    assert!(
        b.l.get_schedule(id).unwrap().due(d(5, 31)).is_empty(),
        "idempotent"
    );
    assert!(matches!(
        b.l.post_schedule_month(id, d(4, 30)),
        Err(LedgerError::InvalidSchedule(_))
    ));
    assert_eq!(b.l.get_schedule(id).unwrap().remaining(), dec("900"));
    let bs = balance_sheet_as_of(&b.l, d(5, 31)).unwrap();
    let prepaid = bs.assets.iter().find(|a| a.code == "1300").unwrap();
    assert_eq!(prepaid.amount, dec("900"));
    let pl = profit_and_loss_for_period(&b.l, d(3, 1), d(5, 31), None).unwrap();
    assert_eq!(pl.total_expenses, dec("300"));
}

#[test]
fn deferred_revenue_is_earned_to_the_cent() {
    let mut b = books();
    // Three months of a subscription collected up front.
    post(&mut b.l, d(1, 5), b.cash, b.deferred, "1000");
    let id =
        b.l.register_schedule(NewSchedule {
            description: "Quarterly plan, Acme".into(),
            kind: ScheduleKind::DeferredRevenue,
            total: usd("1000"),
            start: d(1, 5),
            months: 3,
            balance_account: b.deferred,
            pl_account: b.revenue,
        })
        .unwrap();
    let plan: Vec<Decimal> =
        b.l.get_schedule(id)
            .unwrap()
            .due(d(12, 31))
            .into_iter()
            .map(|(_, a)| a)
            .collect();
    assert_eq!(plan, [dec("333.33"), dec("333.33"), dec("333.34")]);
    for (m, _) in b.l.get_schedule(id).unwrap().due(d(12, 31)) {
        b.l.post_schedule_month(id, m).unwrap();
    }
    let pl = profit_and_loss_for_period(&b.l, d(1, 1), d(3, 31), None).unwrap();
    assert_eq!(pl.total_revenue, dec("1000"));
    let bs = balance_sheet_as_of(&b.l, d(3, 31)).unwrap();
    assert!(bs.liabilities.is_empty(), "fully earned");
}

#[test]
fn schedules_respect_closed_periods_and_account_types() {
    let mut b = books();
    let jan = b.l.create_period("2026-01", d(1, 1), d(1, 31)).unwrap();
    b.l.close_period(jan).unwrap();
    let id =
        b.l.register_schedule(NewSchedule {
            description: "Insurance".into(),
            kind: ScheduleKind::PrepaidExpense,
            total: usd("120"),
            start: d(1, 1),
            months: 12,
            balance_account: b.prepaid,
            pl_account: b.insurance,
        })
        .unwrap();
    assert!(matches!(
        b.l.post_schedule_month(id, d(1, 31)),
        Err(LedgerError::ClosedPeriod(..))
    ));
    b.l.post_schedule_month(id, d(2, 28)).unwrap();
    // A prepaid schedule needs an asset and an expense, not a liability.
    let bad = b.l.register_schedule(NewSchedule {
        description: "x".into(),
        kind: ScheduleKind::PrepaidExpense,
        total: usd("120"),
        start: d(1, 1),
        months: 12,
        balance_account: b.deferred,
        pl_account: b.insurance,
    });
    assert!(matches!(bad, Err(LedgerError::InvalidSchedule(_))));
}

#[test]
fn recurring_entries_generate_drafts_never_posts() {
    let mut b = books();
    let id =
        b.l.register_recurring(NewRecurring {
            description: "Office rent".into(),
            lines: vec![
                (b.rent, Side::Debit, usd("2000")),
                (b.cash, Side::Credit, usd("2000")),
            ],
            frequency: Frequency::Monthly,
            start: d(1, 31),
            end: Some(d(4, 30)),
        })
        .unwrap();
    // Through March: Jan 31, Feb 28, Mar 31 (month ends, no drift).
    let due = b.l.recurring_entries().next().unwrap().due(d(3, 31));
    assert_eq!(due, [d(1, 31), d(2, 28), d(3, 31)]);
    let drafts: Vec<_> = due
        .iter()
        .map(|date| b.l.generate_recurring(id, *date).unwrap())
        .collect();
    for e in &drafts {
        assert_eq!(b.l.get_entry(*e).unwrap().status(), EntryStatus::Draft);
    }
    // Drafts don't touch the books until posted.
    let q1 = profit_and_loss_for_period(&b.l, d(1, 1), d(3, 31), None).unwrap();
    assert_eq!(q1.total_expenses, Decimal::ZERO);
    b.l.post_entry(drafts[1]).unwrap();
    let feb = profit_and_loss_for_period(&b.l, d(2, 1), d(2, 28), None).unwrap();
    assert_eq!(feb.total_expenses, dec("2000"));
    // Never twice; nothing after the end date.
    assert!(matches!(
        b.l.generate_recurring(id, d(2, 28)),
        Err(LedgerError::InvalidSchedule(_))
    ));
    assert_eq!(
        b.l.recurring_entries().next().unwrap().due(d(12, 31)),
        [d(4, 30)]
    );
    // Templates must balance.
    let bad = b.l.register_recurring(NewRecurring {
        description: "x".into(),
        lines: vec![
            (b.rent, Side::Debit, usd("2000")),
            (b.cash, Side::Credit, usd("1999")),
        ],
        frequency: Frequency::Monthly,
        start: d(1, 1),
        end: None,
    });
    assert!(matches!(bad, Err(LedgerError::InvalidSchedule(_))));
    let _ = b.revenue;
}
