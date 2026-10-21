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
    // A year of insurance from March 1, paid that day: an asset, not an
    // expense yet. Equal months, so it starts on the 1st (D-37).
    post(&mut b.l, d(3, 1), b.prepaid, b.cash, "1200");
    let id =
        b.l.register_schedule(NewSchedule {
            description: "Liability insurance".into(),
            kind: ScheduleKind::PrepaidExpense,
            total: usd("1200"),
            start: d(3, 1),
            months: 12,
            balance_account: b.prepaid,
            pl_account: b.insurance,
            convention: ledger_core::ScheduleConvention::MonthlyEven,
        })
        .unwrap();
    // Through May: March, April, May, at month end, 100 each.
    let due: Vec<_> = b.l.get_schedule(id).unwrap().due(d(5, 31)).unwrap();
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
        b.l.get_schedule(id)
            .unwrap()
            .due(d(5, 31))
            .unwrap()
            .is_empty(),
        "idempotent"
    );
    assert!(matches!(
        b.l.post_schedule_month(id, d(4, 30)),
        Err(LedgerError::InvalidSchedule(_))
    ));
    assert_eq!(
        b.l.get_schedule(id).unwrap().remaining().unwrap(),
        dec("900")
    );
    let bs = balance_sheet_as_of(&b.l, d(5, 31)).unwrap();
    let prepaid = bs.assets.iter().find(|a| a.code == "1300").unwrap();
    assert_eq!(prepaid.amount, dec("900"));
    let pl = profit_and_loss_for_period(&b.l, d(3, 1), d(5, 31), None).unwrap();
    assert_eq!(pl.total_expenses, dec("300"));
}

#[test]
fn deferred_revenue_is_earned_to_the_cent() {
    let mut b = books();
    // Three months of a subscription from January 1, collected up front.
    // Equal months, so it starts on the 1st (D-37).
    post(&mut b.l, d(1, 1), b.cash, b.deferred, "1000");
    let id =
        b.l.register_schedule(NewSchedule {
            description: "Quarterly plan, Acme".into(),
            kind: ScheduleKind::DeferredRevenue,
            total: usd("1000"),
            start: d(1, 1),
            months: 3,
            balance_account: b.deferred,
            pl_account: b.revenue,
            convention: ledger_core::ScheduleConvention::MonthlyEven,
        })
        .unwrap();
    let plan: Vec<Decimal> =
        b.l.get_schedule(id)
            .unwrap()
            .due(d(12, 31))
            .unwrap()
            .into_iter()
            .map(|(_, a)| a)
            .collect();
    assert_eq!(plan, [dec("333.33"), dec("333.33"), dec("333.34")]);
    for (m, _) in b.l.get_schedule(id).unwrap().due(d(12, 31)).unwrap() {
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
            convention: ledger_core::ScheduleConvention::MonthlyEven,
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
        convention: ledger_core::ScheduleConvention::MonthlyEven,
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

/// Regression (B-12, found by the store's refusal property on
/// 2026-10-13): a recurring occurrence dated in a closed period is
/// refused, leaves nothing behind, and is reported as needing a human.
#[test]
fn a_recurring_occurrence_in_a_closed_period_is_refused() {
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
            end: None,
        })
        .unwrap();
    let jan = b.l.create_period("2026-01", d(1, 1), d(1, 31)).unwrap();
    b.l.close_period(jan).unwrap();
    let next = b.l.next_entry_id();
    assert!(matches!(
        b.l.generate_recurring(id, d(1, 31)),
        Err(LedgerError::ClosedPeriod(p, _)) if p == jan
    ));
    assert_eq!(b.l.next_entry_id(), next, "no id spent");
    assert!(b.l.get_entry(next).is_none(), "no draft left behind");
    assert_eq!(b.l.recurring_in_closed_periods(d(2, 28)), [(id, d(1, 31))]);
    // Later occurrences still generate.
    assert_eq!(b.l.generate_recurring(id, d(2, 28)).unwrap(), next);
}

#[test]
fn schedule_length_is_one_to_six_hundred_months() {
    let b = books();
    let sched = |months| NewSchedule {
        description: "Insurance".into(),
        kind: ScheduleKind::PrepaidExpense,
        total: usd("600"),
        start: d(1, 1),
        months,
        balance_account: b.prepaid,
        pl_account: b.insurance,
        convention: ledger_core::ScheduleConvention::MonthlyEven,
    };
    let chart = b.l.chart();
    for ok in [1, 600] {
        assert!(
            ledger_core::schedules::validate_schedule(&sched(ok), chart).is_ok(),
            "{ok} months"
        );
    }
    for bad in [0, 601] {
        assert!(
            matches!(
                ledger_core::schedules::validate_schedule(&sched(bad), chart),
                Err(LedgerError::InvalidSchedule(_))
            ),
            "{bad} months"
        );
    }
}

#[test]
fn recurring_templates_take_any_number_of_lines_and_may_end_on_their_start() {
    let mut b = books();
    // Three balanced lines are a valid template.
    b.l.register_recurring(NewRecurring {
        description: "Rent and insurance".into(),
        lines: vec![
            (b.rent, Side::Debit, usd("2000")),
            (b.insurance, Side::Debit, usd("100")),
            (b.cash, Side::Credit, usd("2100")),
        ],
        frequency: Frequency::Monthly,
        start: d(1, 1),
        end: None,
    })
    .unwrap();
    // An end on the start date is one occurrence, not an error.
    let once =
        b.l.register_recurring(NewRecurring {
            description: "One-off".into(),
            lines: vec![
                (b.rent, Side::Debit, usd("2000")),
                (b.cash, Side::Credit, usd("2000")),
            ],
            frequency: Frequency::Monthly,
            start: d(5, 1),
            end: Some(d(5, 1)),
        })
        .unwrap();
    let r = b.l.recurring_entries().find(|r| r.id == once).unwrap();
    assert_eq!(r.due(d(12, 31)), [d(5, 1)]);
    // An end before the start is.
    let bad = b.l.register_recurring(NewRecurring {
        description: "Backwards".into(),
        lines: vec![
            (b.rent, Side::Debit, usd("2000")),
            (b.cash, Side::Credit, usd("2000")),
        ],
        frequency: Frequency::Monthly,
        start: d(5, 2),
        end: Some(d(5, 1)),
    });
    assert!(matches!(bad, Err(LedgerError::InvalidSchedule(_))));
}

#[test]
fn frequencies_parse_and_step_from_the_start() {
    assert_eq!(Frequency::parse("weekly"), Some(Frequency::Weekly));
    assert_eq!(Frequency::parse(" Monthly "), Some(Frequency::Monthly));
    assert_eq!(Frequency::parse("QUARTERLY"), Some(Frequency::Quarterly));
    assert_eq!(Frequency::parse("annually"), Some(Frequency::Yearly));
    assert_eq!(Frequency::parse("fortnightly"), None);
    // Weekly: the kth occurrence is 7k days on.
    let start = d(1, 1);
    for (k, want) in [(0, d(1, 1)), (1, d(1, 8)), (2, d(1, 15)), (3, d(1, 22))] {
        assert_eq!(Frequency::Weekly.nth(start, k), Some(want), "week {k}");
    }
    assert_eq!(Frequency::Quarterly.nth(start, 2), Some(d(7, 1)));
    for (k, year) in [(1, 2027), (2, 2028), (3, 2029)] {
        assert_eq!(
            Frequency::Yearly.nth(start, k),
            NaiveDate::from_ymd_opt(year, 1, 1),
            "year {k}"
        );
    }
}

/// Regression (found by the store's refusal property, 2026-10-13): a
/// schedule whose total no line could carry, or finer than a cent, is
/// refused when registered, not at a month end.
#[test]
fn a_schedule_total_no_month_could_post_is_refused() {
    let mut b = books();
    let new = |total: &str| NewSchedule {
        description: "Insurance".into(),
        kind: ScheduleKind::PrepaidExpense,
        total: usd(total),
        start: d(1, 1),
        months: 6,
        balance_account: b.prepaid,
        pl_account: b.insurance,
        convention: ledger_core::ScheduleConvention::MonthlyEven,
    };
    let huge = new("46116860184273879.03");
    let sub_cent = new("120.005");
    assert!(matches!(
        b.l.register_schedule(huge),
        Err(LedgerError::AmountOutOfRange)
    ));
    assert!(matches!(
        b.l.register_schedule(sub_cent),
        Err(LedgerError::FinerThanMinorUnit { .. })
    ));
}

/// Regression: a schedule month refused after its entry was begun leaves
/// the ledger exactly as it was: no half-built draft, no id spent
/// (Ledger::atomically). The doomed schedule can only come from an older
/// journal now, so it is registered through the replay path.
#[test]
fn a_refused_schedule_month_leaves_nothing_behind() {
    let mut b = books();
    let id =
        b.l.replay_register_schedule(NewSchedule {
            description: "Insurance".into(),
            kind: ScheduleKind::PrepaidExpense,
            total: usd("46116860184273879.03"),
            start: d(1, 1),
            months: 6,
            balance_account: b.prepaid,
            pl_account: b.insurance,
            convention: ledger_core::ScheduleConvention::MonthlyEven,
        })
        .unwrap();
    let before = b.l.clone();
    assert!(b.l.post_schedule_month(id, d(1, 31)).is_err());
    assert!(
        b.l.differs_from(&before).is_empty(),
        "{:?}",
        b.l.differs_from(&before)
    );
}

// ------------------------------------------- defect 9: schedules by the day

use ledger_core::ScheduleConvention;

fn new(
    b: &Books,
    total: Money,
    start: NaiveDate,
    months: u32,
    convention: ScheduleConvention,
) -> NewSchedule {
    NewSchedule {
        description: "Liability insurance".into(),
        kind: ScheduleKind::PrepaidExpense,
        total,
        start,
        months,
        balance_account: b.prepaid,
        pl_account: b.insurance,
        convention,
    }
}

fn ymd(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

/// Defect 9 (plan/served.md D-11, D-37): a new schedule spreads by the
/// day. Hand check: 1,200.00 of insurance from 2026-03-15 for 12 months
/// covers 2026-03-15 through 2027-03-14. March holds 17 of its 31 days:
/// 100.00 × 17/31 = 54.838 → 54.84. April through February are whole
/// months, 100.00 each (eleven). The last March holds 14 days and takes
/// what the others leave: 1,200.00 − 1,154.84 = 45.16 (= 100.00 × 14/31,
/// to the cent). Thirteen month ends.
#[test]
fn defect9_a_new_schedule_spreads_by_the_day() {
    let mut b = books();
    post(&mut b.l, d(3, 15), b.prepaid, b.cash, "1200");
    let id =
        b.l.register_schedule(new(
            &b,
            usd("1200.00"),
            d(3, 15),
            12,
            ScheduleConvention::Daily,
        ))
        .unwrap();
    let s = b.l.get_schedule(id).unwrap();
    assert_eq!(s.convention, ScheduleConvention::Daily);
    let plan = s.plan().unwrap();
    let mut want = vec![(d(3, 31), dec("54.84"))];
    for m in 4..=12 {
        want.push((ledger_core::schedules::month_end(d(m, 1)), dec("100.00")));
    }
    want.push((ymd(2027, 1, 31), dec("100.00")));
    want.push((ymd(2027, 2, 28), dec("100.00")));
    want.push((ymd(2027, 3, 31), dec("45.16")));
    assert_eq!(plan, want);
    assert_eq!(s.first_day(), d(3, 15));
    assert_eq!(s.last_day(), Some(ymd(2027, 3, 14)));
    assert_eq!(s.next().unwrap(), Some(d(3, 31)));
    // Posted through June: the books carry the days used.
    for (m, _) in s.due(d(6, 30)).unwrap() {
        b.l.post_schedule_month(id, m).unwrap();
    }
    let pl = profit_and_loss_for_period(&b.l, d(3, 1), d(6, 30), None).unwrap();
    assert_eq!(pl.total_expenses, dec("354.84"));
    let s = b.l.get_schedule(id).unwrap();
    assert_eq!(s.remaining().unwrap(), dec("845.16"));
    assert_eq!(s.next().unwrap(), Some(d(7, 31)));
}

/// Defect 9 for deferred revenue: earned as the days pass, to the cent.
/// Hand check: 1,000.00 from 2026-01-05 for 3 months covers Jan 5 through
/// Apr 4. Weights: January 27/31, February and March whole, April the
/// rest. Cumulative at each month end: 1,000 × (27/31)/3 = 290.3226 →
/// 290.32; × (27/31 + 1)/3 = 623.6559 → 623.66; × (27/31 + 2)/3 = 956.9892
/// → 956.99; then 1,000.00. Months: 290.32, 333.34, 333.33, 43.01.
#[test]
fn defect9_deferred_revenue_daily_is_earned_to_the_cent() {
    let mut b = books();
    post(&mut b.l, d(1, 5), b.cash, b.deferred, "1000");
    let id =
        b.l.register_schedule(NewSchedule {
            description: "Quarterly plan, a customer".into(),
            kind: ScheduleKind::DeferredRevenue,
            total: usd("1000"),
            start: d(1, 5),
            months: 3,
            balance_account: b.deferred,
            pl_account: b.revenue,
            convention: ScheduleConvention::Daily,
        })
        .unwrap();
    let plan: Vec<Decimal> =
        b.l.get_schedule(id)
            .unwrap()
            .due(d(12, 31))
            .unwrap()
            .into_iter()
            .map(|(_, a)| a)
            .collect();
    assert_eq!(
        plan,
        [dec("290.32"), dec("333.34"), dec("333.33"), dec("43.01")]
    );
    for (m, _) in b.l.get_schedule(id).unwrap().due(d(12, 31)).unwrap() {
        b.l.post_schedule_month(id, m).unwrap();
    }
    // Earned through March: 956.99; still owed for April 1-4: 43.01.
    let pl = profit_and_loss_for_period(&b.l, d(1, 1), d(3, 31), None).unwrap();
    assert_eq!(pl.total_revenue, dec("956.99"));
    let bs = balance_sheet_as_of(&b.l, d(3, 31)).unwrap();
    assert_eq!(
        bs.liabilities
            .iter()
            .find(|a| a.code == "2400")
            .unwrap()
            .amount,
        dec("43.01")
    );
    let bs = balance_sheet_as_of(&b.l, d(4, 30)).unwrap();
    assert!(bs.liabilities.is_empty(), "fully earned");
    assert_eq!(b.l.get_schedule(id).unwrap().next().unwrap(), None);
}

/// Defect 9: schedules an earlier version registered (no convention in
/// the journal) stay equal months, mid-month start and all, exactly as
/// amortization_plan has always made them: replay re-derives every posted
/// month from it.
#[test]
fn defect9_legacy_schedules_stay_monthly_even() {
    let mut b = books();
    let id =
        b.l.replay_register_schedule(new(
            &b,
            usd("1000"),
            d(1, 5),
            3,
            ScheduleConvention::MonthlyEven,
        ))
        .unwrap();
    let s = b.l.get_schedule(id).unwrap();
    assert_eq!(
        s.plan().unwrap(),
        ledger_core::schedules::amortization_plan(dec("1000"), d(1, 5), 3).unwrap()
    );
    assert_eq!(
        s.plan().unwrap(),
        [
            (d(1, 31), dec("333.33")),
            (d(2, 28), dec("333.33")),
            (d(3, 31), dec("333.34"))
        ]
    );
    // It covers whole months: from the 1st of its first.
    assert_eq!(s.first_day(), d(1, 1));
    assert_eq!(s.last_day(), Some(d(3, 31)));
    // The serialized ledger says nothing of a convention it never had.
    let v = serde_json::to_value(s).unwrap();
    assert!(v.get("convention").is_none());
    assert!(v.get("postings").is_none());
}

/// D-37 (owner, 2026-10-20): a new equal-months schedule starts on the
/// 1st of a month, for prepaid expenses and deferred revenue alike, so it
/// never recognizes days it doesn't cover (revenue is never earned early,
/// ASC 606-10-25-23). Refused otherwise, naming the 1st and the daily
/// convention; replay keeps the ones an earlier version took.
#[test]
fn an_equal_months_schedule_starts_on_the_first() {
    let mut b = books();
    for kind in [ScheduleKind::PrepaidExpense, ScheduleKind::DeferredRevenue] {
        let (balance, pl) = match kind {
            ScheduleKind::PrepaidExpense => (b.prepaid, b.insurance),
            ScheduleKind::DeferredRevenue => (b.deferred, b.revenue),
        };
        let mid = NewSchedule {
            description: "x".into(),
            kind,
            total: usd("1200"),
            start: d(3, 15),
            months: 12,
            balance_account: balance,
            pl_account: pl,
            convention: ScheduleConvention::MonthlyEven,
        };
        let before = b.l.clone();
        match b.l.register_schedule(mid.clone()) {
            Err(LedgerError::InvalidSchedule(m)) => {
                assert!(m.contains("2026-03-01"), "{m}");
                assert!(m.contains("daily"), "{m}");
                assert!(m.contains("ASC 606-10-25-23"), "{m}");
            }
            other => panic!("{kind:?}: {other:?}"),
        }
        assert!(b.l.differs_from(&before).is_empty());
        b.l.register_schedule(NewSchedule {
            start: d(3, 1),
            ..mid.clone()
        })
        .unwrap();
        b.l.replay_register_schedule(mid).unwrap();
    }
}

/// R1 (P3 depreciation review): a daily schedule rounds to the currency's
/// minor unit, so in yen every month is whole yen. Hand check: ¥1,000 from
/// 2026-01-05 over 3 months: cumulative 290.32 → 290, 623.66 → 624, 956.99
/// → 957, then 1,000: ¥290, ¥334, ¥333, ¥43. An equal-months yen schedule
/// whose months aren't whole yen (¥333.33) is refused: it could never post.
#[test]
fn a_jpy_daily_schedule_posts_whole_yen() {
    let mut l = Ledger::new(Currency::JPY);
    let prepaid = l
        .add_account("1300", "Prepaid", AccountType::Asset, None, false)
        .unwrap();
    let insurance = l
        .add_account("6300", "Insurance", AccountType::Expense, None, false)
        .unwrap();
    let yen = |a: &str| Money::new(dec(a), Currency::JPY);
    let schedule = |start, convention| NewSchedule {
        description: "Insurance".into(),
        kind: ScheduleKind::PrepaidExpense,
        total: yen("1000"),
        start,
        months: 3,
        balance_account: prepaid,
        pl_account: insurance,
        convention,
    };
    let id = l
        .register_schedule(schedule(d(1, 5), ScheduleConvention::Daily))
        .unwrap();
    let plan: Vec<Decimal> = l
        .get_schedule(id)
        .unwrap()
        .plan()
        .unwrap()
        .into_iter()
        .map(|(_, a)| a)
        .collect();
    assert_eq!(plan, [dec("290"), dec("334"), dec("333"), dec("43")]);
    for (m, _) in l.get_schedule(id).unwrap().due(d(12, 31)).unwrap() {
        l.post_schedule_month(id, m).unwrap();
    }
    match l.register_schedule(schedule(d(1, 1), ScheduleConvention::MonthlyEven)) {
        Err(LedgerError::InvalidSchedule(m)) => assert!(m.contains("333.33"), "{m}"),
        other => panic!("{other:?}"),
    }
}

/// R1 for a three-place currency: a daily schedule of 1,000.000 KWD from
/// 2026-01-05 over 3 months, in thousandths. Hand check: cumulative
/// 290.32258 → 290.323, 623.65591 → 623.656, 956.98925 → 956.989, then
/// 1,000.000: 290.323, 333.333, 333.333, 43.011.
#[test]
fn a_kwd_daily_schedule_rounds_to_the_fils() {
    let kwd = |a: &str| Money::new(dec(a), Currency::new("KWD").unwrap());
    let plan: Vec<Decimal> = ledger_core::schedules::daily_plan(kwd("1000.000"), d(1, 5), 3)
        .unwrap()
        .into_iter()
        .map(|(_, a)| a)
        .collect();
    assert_eq!(
        plan,
        [
            dec("290.323"),
            dec("333.333"),
            dec("333.333"),
            dec("43.011")
        ]
    );
    assert!(plan.iter().all(|a| a.scale() == 3));
}

/// R3 (P3 depreciation review): a reversed schedule month is un-posted, so
/// it is due again; reversing the reversal posts it back; and once it has
/// posted again, re-instating the first posting is refused (one month is
/// never recognized twice).
#[test]
fn a_reversed_schedule_month_is_due_again() {
    let mut b = books();
    let id =
        b.l.register_schedule(new(&b, usd("1200"), d(1, 1), 12, ScheduleConvention::Daily))
            .unwrap();
    let jan = b.l.post_schedule_month(id, d(1, 31)).unwrap();
    let rev = b.l.reverse_entry(jan, d(1, 31), "wrong month").unwrap();
    let s = b.l.get_schedule(id).unwrap();
    assert!(!s.posted.contains(&d(1, 31)));
    assert_eq!(s.next().unwrap(), Some(d(1, 31)));
    assert_eq!(s.remaining().unwrap(), dec("1200"));
    assert!(b.l.schedule_month_posted_before(id, d(1, 31)));
    // Reversing the reversal re-instates January.
    let back = b.l.reverse_entry(rev, d(1, 31), "it was right").unwrap();
    assert!(b.l.get_schedule(id).unwrap().posted.contains(&d(1, 31)));
    assert!(b.l.post_schedule_month(id, d(1, 31)).is_err());
    // Reverse it again, post January again: then re-instating the first
    // posting would recognize January twice.
    b.l.reverse_entry(back, d(1, 31), "no, wrong").unwrap();
    let again = b.l.post_schedule_month(id, d(1, 31)).unwrap();
    assert!(b.l.in_effect(again));
    let before = b.l.clone();
    let rev_of_back =
        b.l.posted_entries()
            .find(|e| e.reversal_of() == Some(back))
            .unwrap()
            .id();
    match b.l.reverse_entry(rev_of_back, d(1, 31), "re-instate") {
        Err(LedgerError::InvalidSchedule(m)) => {
            assert!(m.contains(&format!("entry {again}")), "{m}")
        }
        other => panic!("{other:?}"),
    }
    assert!(b.l.differs_from(&before).is_empty());
    let pl = profit_and_loss_for_period(&b.l, d(1, 1), d(12, 31), None).unwrap();
    assert_eq!(pl.total_expenses, dec("100"));
}

/// P3 review: a schedule month whose posting was reversed after the month
/// ended carried the month at its end; posting it again on its own date
/// would recognize it twice there (until the reversal), so it is left for
/// a human, said, never posted twice. Hand check: 1,200.00 daily from Jan
/// 1 over 12 months, January 100.00 (Jan 31), reversed Feb 3: posting
/// January again is refused, the hold names Feb 3; January's expense on
/// Jan 31 stays 100.00, and February posts as ever.
#[test]
fn a_schedule_month_reversed_after_it_ended_is_left_for_a_human() {
    let mut b = books();
    let id =
        b.l.register_schedule(new(&b, usd("1200"), d(1, 1), 12, ScheduleConvention::Daily))
            .unwrap();
    let jan = b.l.post_schedule_month(id, d(1, 31)).unwrap();
    b.l.reverse_entry(jan, d(2, 3), "wrong policy").unwrap();
    let s = b.l.get_schedule(id).unwrap();
    assert!(!s.posted.contains(&d(1, 31)));
    assert_eq!(
        b.l.schedule_month_reversed_after(id, d(1, 31)),
        Some(d(2, 3))
    );
    let why = b.l.schedule_month_hold(id, d(1, 31)).unwrap();
    assert!(why.contains("2026-02-03"), "{why}");
    let before = b.l.clone();
    match b.l.post_schedule_month(id, d(1, 31)) {
        Err(LedgerError::InvalidSchedule(m)) => assert!(m.contains("twice"), "{m}"),
        other => panic!("{other:?}"),
    }
    assert!(b.l.differs_from(&before).is_empty());
    assert_eq!(b.l.schedule_month_hold(id, d(2, 28)), None);
    b.l.post_schedule_month(id, d(2, 28)).unwrap();
    let jan_pl = profit_and_loss_for_period(&b.l, d(1, 1), d(1, 31), None).unwrap();
    assert_eq!(jan_pl.total_expenses, dec("100"));
}

/// P3 review: a month that rounds to nothing is never a posting (no entry
/// carries a zero line, P3 depreciation review R10). Hand check, daily:
/// 0.02 from Jan 1 over 3 months: the cumulatives are 0.0066 → 0.01,
/// 0.0133 → 0.01, then 0.02, so January 0.01, February nothing (left out
/// of the plan), March 0.01. Equal months: 0.01 over 3 months is 0.00,
/// 0.00 and 0.01; only March is ever due, and it is the next.
#[test]
fn a_month_that_rounds_to_nothing_is_never_a_posting() {
    let mut b = books();
    let id =
        b.l.register_schedule(new(&b, usd("0.02"), d(1, 1), 3, ScheduleConvention::Daily))
            .unwrap();
    let s = b.l.get_schedule(id).unwrap();
    assert_eq!(
        s.plan().unwrap(),
        [(d(1, 31), dec("0.01")), (d(3, 31), dec("0.01"))]
    );
    let id =
        b.l.register_schedule(new(
            &b,
            usd("0.01"),
            d(1, 1),
            3,
            ScheduleConvention::MonthlyEven,
        ))
        .unwrap();
    let s = b.l.get_schedule(id).unwrap();
    assert_eq!(s.plan().unwrap().len(), 3);
    assert_eq!(s.due(d(3, 31)).unwrap(), [(d(3, 31), dec("0.01"))]);
    assert_eq!(s.next().unwrap(), Some(d(3, 31)));
    assert_eq!(s.remaining().unwrap(), dec("0.01"));
}
