//! Equal-months schedules round to the currency's minor unit (K-U5, D-72,
//! BS-prepaid-4), and a schedule written before that keeps the cent rule.
//!
//! ```text
//! GOAL: a new monthly-even schedule's months are whole units of its
//!       currency, while every schedule written under the cent rule
//!       replays to the same figures, printed the same way.
//! SOURCE: BS-prepaid-1, BS-prepaid-4, BS-entry-8; plan/served.md D-72.
//! INVARIANTS:
//!   1. The cent rule (`amortization_plan`) is exactly what every earlier
//!      version computed: the same amounts at the same printed scale,
//!      checked against a verbatim copy of the function as it was.
//!   2. A currency with two places gets the same plan under both rules.
//!   3. Under the minor-unit rule every month is a whole multiple of the
//!      currency's unit and the months sum to the total.
//! SCENARIOS:
//!   1. IF 100,000 yen over 3 months, minor unit: SHOULD be 33,333,
//!      33,333, 33,334 (the cent rule: 33,333.33, 33,333.33, 33,333.34).
//!   2. IF 1.000 dinar over 3 months, minor unit: SHOULD be 0.333, 0.333,
//!      0.334 (the cent rule: 0.33, 0.33, 0.340).
//! ```

use chrono::{Datelike, Months, NaiveDate};
use ledger_core::schedules::{
    ScheduleRounding, amortization_plan, amortization_plan_under, month_end,
};
use ledger_core::{
    AccountType, Currency, Ledger, LedgerError, Money, NewSchedule, ScheduleConvention,
    ScheduleKind, kernel,
};
use proptest::prelude::*;
use rust_decimal::Decimal;

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn dec(s: &str) -> Decimal {
    s.parse().unwrap()
}

fn kwd() -> Currency {
    Currency::new("KWD").unwrap()
}

fn amounts(plan: &[(NaiveDate, Decimal)]) -> Vec<String> {
    plan.iter().map(|(_, a)| a.to_string()).collect()
}

/// `amortization_plan` verbatim as every version before K-U5 had it
/// (`round_places` at two places, the last month the total less
/// `each x (months - 1)`): the oracle for "the cent rule never moves".
fn legacy_plan(
    total: Decimal,
    start: NaiveDate,
    months: u32,
) -> Result<Vec<(NaiveDate, Decimal)>, LedgerError> {
    let too_large = || LedgerError::TooLargeToCompute(format!("a {months}-month plan of {total}"));
    let each = kernel::round_places(
        total
            .checked_div(Decimal::from(months))
            .ok_or_else(too_large)?,
        2,
        kernel::Rounding::HalfToEven,
    )
    .ok_or_else(too_large)?;
    let first = start
        .with_day(1)
        .ok_or(LedgerError::DateOutOfRange(start))?;
    (0..months)
        .map(|i| {
            let month = first
                .checked_add_months(Months::new(i))
                .ok_or(LedgerError::DateOutOfRange(start))?;
            let amount = if i + 1 == months {
                each.checked_mul(Decimal::from(months - 1))
                    .and_then(|before| total.checked_sub(before))
                    .ok_or_else(too_large)?
            } else {
                each
            };
            Ok((month_end(month), amount))
        })
        .collect()
}

/// Same dates, same values, and the same text for every amount: a posted
/// month's replay compares its journaled lines as text.
fn assert_same_plan(
    got: &[(NaiveDate, Decimal)],
    want: &[(NaiveDate, Decimal)],
    what: impl std::fmt::Display,
) {
    assert_eq!(got.len(), want.len(), "{what}: months");
    for (g, w) in got.iter().zip(want) {
        assert_eq!(g.0, w.0, "{what}: date");
        assert_eq!(g.1.to_string(), w.1.to_string(), "{what}: amount text");
    }
}

/// SCENARIOS 1 and 2, recomputed by hand.
///
/// 100,000 yen over 3: 100,000 / 3 = 33,333.33..., to a whole yen 33,333;
/// the last month takes 100,000 - 2 x 33,333 = 33,334. 1.000 dinar over 3:
/// 0.3333... to a thousandth 0.333; the last 1.000 - 0.666 = 0.334. A tie
/// goes to the even multiple: 5 yen over 2 is 2.5, to 2 (the last takes 3);
/// 7 over 2 is 3.5, to 4 (the last takes 3, less than the others).
#[test]
fn a_jpy_and_a_kwd_plan_are_whole_units() {
    let start = d(2027, 1, 1);
    let yen = |a: &str| Money::new(dec(a), Currency::JPY);
    let dinar = |a: &str| Money::new(dec(a), kwd());
    let minor = ScheduleRounding::MinorUnit;
    let plan = amortization_plan_under(minor, yen("100000"), start, 3).unwrap();
    assert_eq!(amounts(&plan), ["33333", "33333", "33334"]);
    assert_eq!(
        plan.iter().map(|(m, _)| *m).collect::<Vec<_>>(),
        [d(2027, 1, 31), d(2027, 2, 28), d(2027, 3, 31)]
    );
    let plan = amortization_plan_under(minor, dinar("1.000"), start, 3).unwrap();
    assert_eq!(amounts(&plan), ["0.333", "0.333", "0.334"]);
    let plan = amortization_plan_under(minor, yen("5"), start, 2).unwrap();
    assert_eq!(amounts(&plan), ["2", "3"]);
    let plan = amortization_plan_under(minor, yen("7"), start, 2).unwrap();
    assert_eq!(amounts(&plan), ["4", "3"]);
    // The cent rule, which every schedule before D-72 replays under, is
    // unchanged: ¥33,333.33 months no yen account can post.
    let cent = amortization_plan(dec("100000"), start, 3).unwrap();
    assert_eq!(amounts(&cent), ["33333.33", "33333.33", "33333.34"]);
    let cent = amortization_plan(dec("1.000"), start, 3).unwrap();
    assert_eq!(amounts(&cent), ["0.33", "0.33", "0.340"]);
}

/// Prints the numbers the change moves (read with --nocapture): the
/// verification protocol's "show the figures".
#[test]
fn print_the_plans_before_and_after() {
    let start = d(2027, 1, 1);
    for (label, money) in [
        ("JPY 100000", Money::new(dec("100000"), Currency::JPY)),
        ("KWD 1.000", Money::new(dec("1.000"), kwd())),
        ("USD 1000.00", Money::new(dec("1000.00"), Currency::USD)),
    ] {
        let before = amortization_plan(money.amount, start, 3).unwrap();
        let after = amortization_plan_under(ScheduleRounding::MinorUnit, money, start, 3).unwrap();
        println!("{label} over 3 months");
        println!("  cent rule (before): {:?}", amounts(&before));
        println!("  minor unit (after): {:?}", amounts(&after));
    }
}

/// INVARIANT 1 and 2 over a large range: totals of every scale 0 to 5,
/// months 1 to 40. The cent rule equals the legacy copy value for value
/// and text for text, and a two-place currency gets the same plan under
/// the minor-unit rule (the same call, so this is a check of the claim,
/// not its proof).
#[test]
fn the_cent_rule_is_unchanged_and_usd_is_the_same_under_both_rules() {
    let start = d(2027, 3, 1);
    let mut checked = 0u64;
    for scale in 0..=5u32 {
        for mantissa in (1..=1_200i64).chain([9_999, 10_000, 99_999, 123_456_789, 4_000_000_007]) {
            let total = Decimal::new(mantissa, scale);
            for months in (1..=40u32).chain([360, 600]) {
                let want = legacy_plan(total, start, months).unwrap();
                let got = amortization_plan(total, start, months).unwrap();
                assert_same_plan(&got, &want, format!("{total} over {months}"));
                if scale <= 2 {
                    let usd = Money::new(total, Currency::USD);
                    let under =
                        amortization_plan_under(ScheduleRounding::MinorUnit, usd, start, months)
                            .unwrap();
                    assert_same_plan(&under, &want, format!("USD {total} over {months}"));
                }
                checked += 1;
            }
        }
    }
    assert!(checked > 300_000, "{checked}");
}

/// Under the minor-unit rule, for every 0-, 2-, 3- and 4-place currency:
/// the months sum to the total, all but the last are equal, and each is a
/// whole multiple of the unit.
#[test]
fn the_minor_unit_rule_gives_whole_units_that_sum_to_the_total() {
    let start = d(2027, 1, 1);
    for (currency, places) in [
        (Currency::JPY, 0u32),
        (Currency::USD, 2),
        (kwd(), 3),
        (Currency::new("CLF").unwrap(), 4),
    ] {
        for units in (1..=2_000i64).chain([99_991, 1_000_003]) {
            let total = Decimal::new(units, places);
            for months in 1..=24u32 {
                let plan = amortization_plan_under(
                    ScheduleRounding::MinorUnit,
                    Money::new(total, currency),
                    start,
                    months,
                )
                .unwrap();
                let sum: Decimal = plan.iter().map(|(_, a)| *a).sum();
                assert_eq!(sum, total, "{currency} {total} over {months}");
                let each = plan[0].1;
                for (_, a) in &plan[..plan.len() - 1] {
                    assert_eq!(*a, each);
                }
                for (_, a) in &plan {
                    assert!(a.normalize().scale() <= places, "{currency} {a}");
                    assert!(a.scale() <= places, "{currency} {a} prints past its unit");
                }
            }
        }
    }
}

proptest! {
    /// The cent rule is the legacy rule for any total below the exact
    /// limit, at any scale up to 8, and any number of months: values and
    /// printed text.
    #[test]
    fn prop_the_cent_rule_is_the_legacy_rule(
        mantissa in 1i128..=99_999_999_999_999_999_999_999_999i128,
        scale in 0u32..=8,
        months in 1u32..=600,
    ) {
        let total = Decimal::try_from_i128_with_scale(mantissa, scale).unwrap();
        prop_assume!(total < Decimal::from_i128_with_scale(10i128.pow(25), 0));
        let start = d(2027, 1, 1);
        let want = legacy_plan(total, start, months).unwrap();
        let got = amortization_plan(total, start, months).unwrap();
        assert_same_plan(&got, &want, format!("{total} over {months}"));
    }
}

fn ledger_in(currency: Currency) -> (Ledger, u64, u64) {
    let mut l = Ledger::new(currency);
    let prepaid = l
        .add_account("1300", "Prepaid", AccountType::Asset, None, false)
        .unwrap();
    let insurance = l
        .add_account("6300", "Insurance", AccountType::Expense, None, false)
        .unwrap();
    (l, prepaid, insurance)
}

fn monthly_even(total: Money, prepaid: u64, expense: u64) -> NewSchedule {
    NewSchedule {
        description: "Insurance".into(),
        kind: ScheduleKind::PrepaidExpense,
        total,
        start: d(2027, 1, 1),
        months: 3,
        balance_account: prepaid,
        pl_account: expense,
        convention: ScheduleConvention::MonthlyEven,
    }
}

/// The two registrations side by side: the plain one (the cent rule)
/// still refuses ¥1,000 over 3 months, a schedule registered under the
/// minor-unit rule takes it, plans ¥333, ¥333, ¥334 and carries the rule.
#[test]
fn a_schedule_carries_the_rule_it_was_written_under() {
    let yen = |a: &str| Money::new(dec(a), Currency::JPY);
    let (mut l, prepaid, expense) = ledger_in(Currency::JPY);
    match l.register_schedule(monthly_even(yen("1000"), prepaid, expense)) {
        Err(LedgerError::InvalidSchedule(m)) => assert!(m.contains("333.33"), "{m}"),
        other => panic!("{other:?}"),
    }
    let id = l
        .register_schedule_under(
            monthly_even(yen("1000"), prepaid, expense),
            ScheduleRounding::MinorUnit,
        )
        .unwrap();
    let s = l.get_schedule(id).unwrap();
    assert_eq!(s.rounding, ScheduleRounding::MinorUnit);
    assert_eq!(amounts(&s.plan().unwrap()), ["333", "333", "334"]);
    for (m, _) in s.due(d(2027, 12, 31)).unwrap() {
        l.post_schedule_month(id, m).unwrap();
    }
    assert_eq!(l.get_schedule(id).unwrap().remaining().unwrap(), dec("0"));
    // A schedule written the old way, in the same books, still plans in
    // cents: a dollar-sized yen total that divides evenly.
    let old = l
        .register_schedule(monthly_even(yen("900"), prepaid, expense))
        .unwrap();
    let s = l.get_schedule(old).unwrap();
    assert_eq!(s.rounding, ScheduleRounding::Cent);
    assert_eq!(amounts(&s.plan().unwrap()), ["300", "300", "300"]);
}

/// The rule is journaled only when it isn't the cent rule: a serialized
/// ledger from before D-72 reads back as the cent rule, and a cent
/// schedule serializes exactly as it did (no `rounding` key).
#[test]
fn the_cent_rule_is_left_unsaid_in_a_serialized_ledger() {
    let (mut l, prepaid, expense) = ledger_in(Currency::USD);
    let usd = |a: &str| Money::new(dec(a), Currency::USD);
    let old = l
        .register_schedule(monthly_even(usd("1000.00"), prepaid, expense))
        .unwrap();
    let new = l
        .register_schedule_under(
            monthly_even(usd("1000.00"), prepaid, expense),
            ScheduleRounding::MinorUnit,
        )
        .unwrap();
    let old_json = serde_json::to_value(l.get_schedule(old).unwrap()).unwrap();
    let new_json = serde_json::to_value(l.get_schedule(new).unwrap()).unwrap();
    assert!(old_json.get("rounding").is_none(), "{old_json}");
    assert_eq!(new_json["rounding"], "minor_unit");
    let back: ledger_core::AmortizationSchedule = serde_json::from_value(old_json).unwrap();
    assert_eq!(back.rounding, ScheduleRounding::Cent);
}
