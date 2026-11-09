//! A new monthly-even schedule rounds to its currency's minor unit; one
//! written before the journal feature `schedule_minor_unit` keeps the
//! cent rule on replay (K-U5, BS-prepaid-4, D-72).
//!
//! ```text
//! GOAL: a yen schedule's months are whole yen, and what an earlier
//!       version journaled never moves.
//! SOURCE: BS-prepaid-4, BS-format-1, BS-entry-8; plan/served.md D-72
//!         (owner, 2026-11-08).
//! INVARIANTS:
//!   1. In books that declared the feature, a monthly_even schedule's
//!      line carries `"rounding": "minor_unit"` and its months are whole
//!      units of its currency.
//!   2. In books that haven't, nothing new is written: the line has no
//!      `rounding`, the plan is the cent rule's, and a month finer than
//!      the currency's unit is refused as before.
//!   3. A schedule's rule is its own: books upgraded after a cent-rule
//!      schedule was written replay it, and post its months, under the
//!      cent rule.
//!   4. A reader that doesn't know the feature stops at its declaration
//!      as newer books (format.rs's schedule_minor_unit_is_its_own_feature).
//! ```

use chrono::NaiveDate;
use ledger_core::{
    AccountType, Currency, Money, NewSchedule, ScheduleConvention, ScheduleKind, ScheduleRounding,
};
use ledger_store::format::{SCHEDULE_MINOR_UNIT, current};
use ledger_store::{JOURNAL_FILE, Store, StoreError};
use rust_decimal::Decimal;
use serde_json::Value;
use std::path::Path;

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn kwd() -> Currency {
    Currency::new("KWD").unwrap()
}

fn amount(s: &str, c: Currency) -> Money {
    Money::new(s.parse::<Decimal>().unwrap(), c)
}

fn plan_text(store: &Store, id: u64) -> Vec<String> {
    store
        .ledger()
        .get_schedule(id)
        .unwrap()
        .plan()
        .unwrap()
        .iter()
        .map(|(_, a)| a.to_string())
        .collect()
}

/// The journal's `schedule.registered` payloads, in order.
fn registered(dir: &Path) -> Vec<Value> {
    std::fs::read_to_string(dir.join(JOURNAL_FILE))
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .filter(|l| l["type"] == "schedule.registered")
        .map(|l| l["payload"].clone())
        .collect()
}

fn books(dir: &Path, currency: Currency, with_feature: bool) -> (Store, u64, u64) {
    let mut format = current();
    if !with_feature {
        format.features.retain(|f| f != SCHEDULE_MINOR_UNIT);
    }
    let mut s = Store::create_in_format(dir, currency, &format).unwrap();
    let prepaid = s
        .add_account("1300", "Prepaid", AccountType::Asset, None, false)
        .unwrap();
    let expense = s
        .add_account("6300", "Insurance", AccountType::Expense, None, false)
        .unwrap();
    (s, prepaid, expense)
}

fn schedule(total: Money, prepaid: u64, expense: u64) -> NewSchedule {
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

/// INVARIANT 1, by hand: 100,000 yen over 3 is 33,333.33..., to the yen
/// 33,333, the last month 100,000 - 66,666 = 33,334; 1.000 dinar is 0.333,
/// 0.333, 0.334. The line carries the rule, every month posts whole units,
/// and the books replay on reopening (the replay compares each posted
/// line as text).
#[test]
fn new_books_round_a_yen_and_a_dinar_schedule_to_the_unit() {
    for (currency, total, want) in [
        (Currency::JPY, "100000", ["33333", "33333", "33334"]),
        (kwd(), "1.000", ["0.333", "0.333", "0.334"]),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let (mut s, prepaid, expense) = books(dir.path(), currency, true);
        let id = s
            .register_schedule(schedule(amount(total, currency), prepaid, expense))
            .unwrap();
        assert_eq!(plan_text(&s, id), want);
        assert_eq!(
            s.ledger().get_schedule(id).unwrap().rounding,
            ScheduleRounding::MinorUnit
        );
        let line = &registered(dir.path())[0];
        assert_eq!(line["rounding"], "minor_unit");
        assert_eq!(line["convention"], "monthly_even");
        assert_eq!(s.run_schedules(d(2027, 12, 31)).unwrap().len(), 3);
        s.verify().unwrap();
        let ledger_before = s.ledger().clone();
        drop(s);
        let reopened = Store::open(dir.path()).unwrap();
        assert_eq!(reopened.ledger(), &ledger_before);
        assert_eq!(plan_text(&reopened, id), want);
        reopened.verify().unwrap();
    }
}

/// A daily schedule is written as it always was: no `rounding`.
#[test]
fn a_daily_schedule_carries_no_rounding() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, prepaid, expense) = books(dir.path(), Currency::JPY, true);
    s.register_schedule(NewSchedule {
        convention: ScheduleConvention::Daily,
        ..schedule(amount("100000", Currency::JPY), prepaid, expense)
    })
    .unwrap();
    assert!(registered(dir.path())[0].get("rounding").is_none());
}

/// INVARIANT 2: books that haven't declared the feature (waiting for the
/// owner's upgrade) keep writing and planning as before: a yen schedule
/// whose months aren't whole yen is refused, a schedule that divides
/// evenly is taken without a `rounding`, and nothing journals the feature
/// uninvited.
#[test]
fn books_without_the_feature_keep_the_cent_rule_and_write_nothing_new() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, prepaid, expense) = books(dir.path(), Currency::JPY, false);
    let head = s.head();
    match s.register_schedule(schedule(amount("1000", Currency::JPY), prepaid, expense)) {
        Err(StoreError::Ledger(e)) => assert!(e.to_string().contains("333.33"), "{e}"),
        other => panic!("expected a refusal, got {other:?}"),
    }
    assert_eq!(s.head(), head, "a refusal journals nothing");
    let id = s
        .register_schedule(schedule(amount("900", Currency::JPY), prepaid, expense))
        .unwrap();
    assert_eq!(plan_text(&s, id), ["300", "300", "300"]);
    assert_eq!(
        s.ledger().get_schedule(id).unwrap().rounding,
        ScheduleRounding::Cent
    );
    assert!(registered(dir.path())[0].get("rounding").is_none());
}

/// INVARIANT 3 and the replay test: a dinar schedule written before the
/// feature plans 0.33, 0.33, 0.340 (the cent rule; each a valid dinar
/// amount) and posts its first month at that figure. After the owner's
/// upgrade, the same books replay to the same plan and the same posted
/// line; the old schedule still posts its other months at the cent rule's
/// figures, while a schedule written after the upgrade gets 0.333, 0.333,
/// 0.334.
#[test]
fn a_schedule_written_before_the_feature_replays_to_the_same_plan_after_it() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, prepaid, expense) = books(dir.path(), kwd(), false);
    let old = s
        .register_schedule(schedule(amount("1.000", kwd()), prepaid, expense))
        .unwrap();
    let before = plan_text(&s, old);
    assert_eq!(before, ["0.33", "0.33", "0.340"]);
    s.post_schedule_month(old, d(2027, 1, 31)).unwrap();
    let first_month = s
        .ledger()
        .posted_entries()
        .last()
        .unwrap()
        .lines()
        .iter()
        .map(|l| l.amount.amount.to_string())
        .collect::<Vec<_>>();
    assert_eq!(first_month, ["0.33", "0.33"]);

    assert!(s.upgrade_format().unwrap().is_some());
    assert!(s.journal_format().has_feature(SCHEDULE_MINOR_UNIT));
    assert_eq!(plan_text(&s, old), before, "the upgrade moves no plan");
    drop(s);

    // Replay from the journal, as a fresh process does.
    let mut s = Store::open(dir.path()).unwrap();
    s.verify().unwrap();
    let schedule_old = s.ledger().get_schedule(old).unwrap();
    assert_eq!(schedule_old.rounding, ScheduleRounding::Cent);
    assert_eq!(plan_text(&s, old), before, "replay keeps the cent rule");
    let replayed = s
        .ledger()
        .posted_entries()
        .last()
        .unwrap()
        .lines()
        .iter()
        .map(|l| l.amount.amount.to_string())
        .collect::<Vec<_>>();
    assert_eq!(replayed, first_month, "the posted line is as it was");

    // The rest of the old schedule posts at the cent rule's figures, and a
    // new one, in the same books, at the dinar's.
    s.post_schedule_month(old, d(2027, 2, 28)).unwrap();
    s.post_schedule_month(old, d(2027, 3, 31)).unwrap();
    let new = s
        .register_schedule(schedule(amount("1.000", kwd()), prepaid, expense))
        .unwrap();
    assert_eq!(plan_text(&s, new), ["0.333", "0.333", "0.334"]);
    assert_eq!(
        s.ledger().get_schedule(new).unwrap().rounding,
        ScheduleRounding::MinorUnit
    );
    let lines = registered(dir.path());
    assert!(lines[0].get("rounding").is_none());
    assert_eq!(lines[1]["rounding"], "minor_unit");
    s.verify().unwrap();
    drop(s);
    let again = Store::open(dir.path()).unwrap();
    assert_eq!(plan_text(&again, old), before);
    assert_eq!(plan_text(&again, new), ["0.333", "0.333", "0.334"]);
    again.verify().unwrap();
}

/// A US-dollar schedule is the same under both rules, in every book:
/// written before or after the feature, it plans the same figures.
#[test]
fn a_dollar_schedule_plans_the_same_in_old_and_new_books() {
    let plan_in = |with_feature: bool| {
        let dir = tempfile::tempdir().unwrap();
        let (mut s, prepaid, expense) = books(dir.path(), Currency::USD, with_feature);
        let id = s
            .register_schedule(schedule(amount("1000.00", Currency::USD), prepaid, expense))
            .unwrap();
        plan_text(&s, id)
    };
    assert_eq!(plan_in(false), ["333.33", "333.33", "333.34"]);
    assert_eq!(plan_in(true), plan_in(false));
}

/// A `schedule.registered` naming a rounding this version doesn't know is
/// newer books, never corrupt (BS-format-1): the line is whole and
/// chained, and this reader says it is old.
#[test]
fn an_unknown_rounding_is_newer_not_corrupt() {
    let dir = tempfile::tempdir().unwrap();
    let (s, prepaid, expense) = books(dir.path(), Currency::USD, true);
    drop(s);
    {
        let mut w = ledger_store::JournalWriter::open(&dir.path().join(JOURNAL_FILE)).unwrap();
        w.append(
            "schedule.registered",
            serde_json::json!({
                "id": 1, "description": "x", "kind": "prepaid_expense",
                "total": {"amount": "1000.00", "currency": "USD"},
                "start": "2027-01-01", "months": 3,
                "balance_account": prepaid, "pl_account": expense,
                "convention": "monthly_even", "rounding": "to_the_nickel"
            }),
        )
        .unwrap();
    }
    match Store::open(dir.path()) {
        Err(StoreError::NewerJournal { need, .. }) => {
            assert!(need.contains("to_the_nickel"), "{need}")
        }
        other => panic!("expected NewerJournal, got {:?}", other.map(|_| ())),
    }
}
