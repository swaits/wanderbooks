//! Fixed-asset tests: the depreciation math must be exact to the penny.

use chrono::NaiveDate;
use ledger_core::{
    AccountType, AssetStatus, Currency, DepreciationMethod, Ledger, Money, NewAsset,
    accumulated_scheduled_through, depreciation_schedule, disposal_amounts, scheduled_book_value,
};
use rust_decimal::Decimal;

fn usd(amount: &str) -> Money {
    Money::new(amount.parse().unwrap(), Currency::USD)
}

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

fn test_ledger() -> (Ledger, u64, u64, u64) {
    let mut ledger = Ledger::new(Currency::USD);
    let cost = ledger
        .add_account("1500", "Equipment", AccountType::Asset, None, false)
        .unwrap();
    let accum = ledger
        .add_account(
            "1501",
            "Accumulated Depreciation",
            AccountType::Asset,
            None,
            false,
        )
        .unwrap();
    let expense = ledger
        .add_account(
            "6100",
            "Depreciation Expense",
            AccountType::Expense,
            None,
            false,
        )
        .unwrap();
    (ledger, cost, accum, expense)
}

/// Register the standard test asset: bought 2026-01-15, 60-month life,
/// zero salvage, straight-line.
fn van_asset(
    ledger: &mut Ledger,
    dollars: u64,
    cost_acct: u64,
    accum_acct: u64,
    expense_acct: u64,
) -> u64 {
    ledger
        .register_asset(NewAsset {
            description: "Sprinter van".to_string(),
            in_service_date: date(2026, 1, 15),
            cost: usd(&dollars.to_string()),
            salvage_value: usd("0"),
            useful_life_months: 60,
            cost_account: cost_acct,
            accum_depr_account: accum_acct,
            expense_account: expense_acct,
        })
        .unwrap()
}

#[test]
fn schedule_sums_exactly_to_depreciable_amount() {
    let (mut ledger, cost, accum, expense) = test_ledger();
    // Odd numbers chosen to not divide evenly: 100000 / 60.
    let id = van_asset(&mut ledger, 100_000, cost, accum, expense);
    let asset = ledger.get_asset(id).unwrap();
    let sched = depreciation_schedule(asset, date(2031, 12, 31)).unwrap();
    // 61 calendar months touched (partial first and last), 60 months' worth.
    assert_eq!(sched.len(), 61);
    let total: Decimal = sched.iter().map(|l| l.amount).sum();
    assert_eq!(
        total,
        Decimal::from(100_000),
        "lifetime total must be exact"
    );
    assert_eq!(sched.last().unwrap().accumulated, Decimal::from(100_000));
    assert_eq!(sched.last().unwrap().book_value, Decimal::ZERO);
    // First month prorated: 17 of 31 days → 100000/60 * 17/31 = 913.98.
    let first = &sched[0];
    assert_eq!(first.period, "2026-01");
    assert_eq!(first.amount, Decimal::new(91398, 2));
    // Last month is the complement: 14 of 31 days.
    assert_eq!(sched[60].period, "2031-01");
}

#[test]
fn partial_months_complement_to_whole() {
    let (mut ledger, cost, accum, expense) = test_ledger();
    let id = van_asset(&mut ledger, 100_000, cost, accum, expense);
    let asset = ledger.get_asset(id).unwrap();
    let sched = depreciation_schedule(asset, date(2031, 12, 31)).unwrap();
    // Jan 2026 (17/31) + Jan 2031 (14/31) = exactly one month's charge.
    let monthly = Decimal::from(100_000) / Decimal::from(60);
    let monthly_rounded =
        monthly.round_dp_with_strategy(2, rust_decimal::RoundingStrategy::MidpointAwayFromZero);
    let pair = sched[0].amount + sched[60].amount;
    assert!(
        (pair - monthly_rounded).abs() <= Decimal::new(1, 2),
        "partial months should complement: {pair} vs {monthly_rounded}"
    );
}

#[test]
fn salvage_value_is_never_depreciated() {
    let (mut ledger, cost, accum, expense) = test_ledger();
    let id = ledger
        .register_asset(NewAsset {
            description: "Machine".to_string(),
            in_service_date: date(2026, 3, 1),
            cost: usd("50000"),
            salvage_value: usd("5000"),
            useful_life_months: 36,
            cost_account: cost,
            accum_depr_account: accum,
            expense_account: expense,
        })
        .unwrap();
    let asset = ledger.get_asset(id).unwrap();
    let sched = depreciation_schedule(asset, date(2030, 12, 31)).unwrap();
    let total: Decimal = sched.iter().map(|l| l.amount).sum();
    assert_eq!(total, Decimal::from(45_000));
    assert_eq!(sched.last().unwrap().book_value, Decimal::from(5_000));
}

#[test]
fn book_value_tracks_accumulated() {
    let (mut ledger, cost, accum, expense) = test_ledger();
    let id = van_asset(&mut ledger, 60_000, cost, accum, expense);
    let asset = ledger.get_asset(id).unwrap();
    let acc = accumulated_scheduled_through(asset, date(2026, 12, 31)).unwrap();
    // Jan 2026 prorated 17/31 of 1000 = 548.39, then 11 full months of 1000.
    assert_eq!(acc, Decimal::new(54839, 2) + Decimal::from(11_000));
    assert_eq!(
        scheduled_book_value(asset, date(2026, 12, 31)).unwrap(),
        Decimal::from(60_000) - acc
    );
}

#[test]
fn disposal_math() {
    let (mut ledger, cost, accum, expense) = test_ledger();
    let id = van_asset(&mut ledger, 60_000, cost, accum, expense);
    let asset = ledger.get_asset(id).unwrap();
    // Sell after one year for 55,000. Book value is 60000 − 11548.39 =
    // 48451.61, so this is a gain of 6548.39.
    let d = disposal_amounts(asset, date(2026, 12, 31), usd("55000")).unwrap();
    assert_eq!(d.book_value, Decimal::new(4845161, 2));
    assert_eq!(d.accumulated, Decimal::new(1154839, 2));
    assert_eq!(d.gain_loss, Decimal::new(654839, 2));
    // Disposing twice fails.
    ledger
        .dispose_asset(id, date(2026, 12, 31), usd("55000"))
        .unwrap();
    assert!(
        ledger
            .dispose_asset(id, date(2027, 1, 1), usd("1"))
            .is_err()
    );
    let asset = ledger.get_asset(id).unwrap();
    assert!(matches!(asset.status, AssetStatus::Disposed { .. }));
}

#[test]
fn registration_validation() {
    let (mut ledger, cost, accum, expense) = test_ledger();
    let liability = ledger
        .add_account("2000", "Payable", AccountType::Liability, None, false)
        .unwrap();
    let base = NewAsset {
        description: "X".to_string(),
        in_service_date: date(2026, 1, 1),
        cost: usd("1000"),
        salvage_value: usd("0"),
        useful_life_months: 12,
        cost_account: cost,
        accum_depr_account: accum,
        expense_account: expense,
    };
    // Wrong account type for the expense link.
    let mut bad = base.clone();
    bad.expense_account = liability;
    assert!(ledger.register_asset(bad).is_err());
    // Salvage >= cost.
    let mut bad = base.clone();
    bad.salvage_value = usd("1000");
    assert!(ledger.register_asset(bad).is_err());
    // Zero life.
    let mut bad = base.clone();
    bad.useful_life_months = 0;
    assert!(ledger.register_asset(bad).is_err());
    // Duplicate account links.
    let mut bad = base.clone();
    bad.accum_depr_account = cost;
    assert!(ledger.register_asset(bad).is_err());
    // Unknown account.
    let mut bad = base.clone();
    bad.cost_account = 999;
    assert!(ledger.register_asset(bad).is_err());
    // The good one works.
    assert!(ledger.register_asset(base).is_ok());
}

#[test]
fn method_is_straight_line() {
    let (mut ledger, cost, accum, expense) = test_ledger();
    let id = van_asset(&mut ledger, 12_000, cost, accum, expense);
    let asset = ledger.get_asset(id).unwrap();
    assert_eq!(asset.method, DepreciationMethod::StraightLine);
}

#[test]
fn registration_rejects_date_overflow() {
    let (mut ledger, cost, accum, expense) = test_ledger();
    // A purchase date near the end of the calendar with a long life would
    // overflow the schedule's month arithmetic; registration must refuse.
    let err = ledger
        .register_asset(NewAsset {
            description: "Time machine".to_string(),
            in_service_date: NaiveDate::MAX,
            cost: usd("1000"),
            salvage_value: usd("0"),
            useful_life_months: 12,
            cost_account: cost,
            accum_depr_account: accum,
            expense_account: expense,
        })
        .unwrap_err();
    assert!(matches!(
        err,
        ledger_core::LedgerError::DateOutOfRange(d) if d == NaiveDate::MAX
    ));
    // Inside the calendar the books keep (ledger_core::calendar), but a
    // life that runs past its last day: refused too, whatever chrono
    // could still reach (P0 review round 3, S1).
    let new = |date: NaiveDate, months: u32| NewAsset {
        description: "Late machine".to_string(),
        in_service_date: date,
        cost: usd("1000"),
        salvage_value: usd("0"),
        useful_life_months: months,
        cost_account: cost,
        accum_depr_account: accum,
        expense_account: expense,
    };
    let d = |y, m, day| NaiveDate::from_ymd_opt(y, m, day).unwrap();
    for (date, months) in [
        (d(9999, 6, 1), 12),
        (d(9999, 12, 31), 1),
        (d(2026, 1, 1), u32::MAX),
    ] {
        assert!(matches!(
            ledger.register_asset(new(date, months)).unwrap_err(),
            ledger_core::LedgerError::AssetDateOverflow
        ));
    }
    // A life that ends on the calendar's last day is kept.
    ledger.register_asset(new(d(9999, 11, 30), 1)).unwrap();
    ledger.register_asset(new(d(1, 1, 1), 12)).unwrap();
}

#[test]
fn schedule_through_purchase_date_covers_one_day() {
    let (mut ledger, cost, accum, expense) = test_ledger();
    let id = ledger
        .register_asset(NewAsset {
            description: "X".to_string(),
            in_service_date: date(2026, 1, 15),
            cost: usd("62000"),
            salvage_value: usd("0"),
            useful_life_months: 62,
            cost_account: cost,
            accum_depr_account: accum,
            expense_account: expense,
        })
        .unwrap();
    let asset = ledger.get_asset(id).unwrap();
    // Through the day before purchase: nothing.
    assert!(
        depreciation_schedule(asset, date(2026, 1, 14))
            .unwrap()
            .is_empty()
    );
    // Through the purchase date itself: one day of the first month.
    let one_day = depreciation_schedule(asset, date(2026, 1, 15)).unwrap();
    assert_eq!(one_day.len(), 1);
    assert_eq!(one_day[0].period, "2026-01");
    // 62000/62 = 1000/month; 1/31 of that = 32.26.
    assert_eq!(one_day[0].amount, Decimal::new(3226, 2));
}

#[test]
fn van_schedule_is_the_hand_computed_one() {
    // Rank 1, computed by hand (rules.md B-15), replacing an unreviewed
    // golden file (2026-10-13; every row of the old file was checked
    // against this computation first, and matched).
    // $60,000 over 60 months is $1,000 a month. Bought 2026-01-15: January
    // holds 17 of 31 days = 548.39; February 2026 through December 2030 are
    // 59 full months; life ends 2031-01-15, so January 2031 holds 14 of 31
    // days = 451.61. 548.39 + 59,000 + 451.61 = 60,000.00 exactly.
    let (mut ledger, cost, accum, expense) = test_ledger();
    let id = van_asset(&mut ledger, 60_000, cost, accum, expense);
    let asset = ledger.get_asset(id).unwrap();
    let sched = depreciation_schedule(asset, date(2032, 1, 1)).unwrap();
    assert_eq!(sched.len(), 61);
    let mut accumulated = Decimal::ZERO;
    for (k, line) in sched.iter().enumerate() {
        let (year, month) = (2026 + (k as i32) / 12, (k as u32) % 12 + 1);
        let amount = match k {
            0 => Decimal::new(54839, 2),
            60 => Decimal::new(45161, 2),
            _ => Decimal::new(100000, 2),
        };
        accumulated += amount;
        assert_eq!(line.period, format!("{year}-{month:02}"));
        assert_eq!(line.amount, amount, "{}", line.period);
        assert_eq!(line.accumulated, accumulated, "{}", line.period);
        assert_eq!(
            line.book_value,
            Decimal::from(60_000) - accumulated,
            "{}",
            line.period
        );
    }
    assert_eq!(accumulated, Decimal::from(60_000));
}

#[test]
fn registration_rejects_wrong_typed_accounts() {
    let (mut ledger, cost, accum, expense) = test_ledger();
    let base = || NewAsset {
        description: "X".to_string(),
        in_service_date: date(2026, 1, 15),
        cost: usd("1000"),
        salvage_value: usd("0"),
        useful_life_months: 12,
        cost_account: cost,
        accum_depr_account: accum,
        expense_account: expense,
    };
    // An expense account cannot hold the cost.
    let err = ledger
        .register_asset(NewAsset {
            cost_account: expense,
            ..base()
        })
        .unwrap_err();
    assert!(matches!(
        err,
        ledger_core::LedgerError::AssetAccountWrongType { .. }
    ));
    // An asset account cannot be the depreciation expense.
    let err = ledger
        .register_asset(NewAsset {
            expense_account: cost,
            ..base()
        })
        .unwrap_err();
    assert!(matches!(
        err,
        ledger_core::LedgerError::AssetAccountWrongType { .. }
    ));
    // The three links must be distinct accounts.
    let err = ledger
        .register_asset(NewAsset {
            accum_depr_account: cost,
            ..base()
        })
        .unwrap_err();
    assert!(matches!(
        err,
        ledger_core::LedgerError::DuplicateAssetAccount(_)
    ));
    // Unknown accounts are rejected.
    let err = ledger
        .register_asset(NewAsset {
            cost_account: 999,
            ..base()
        })
        .unwrap_err();
    assert!(matches!(err, ledger_core::LedgerError::UnknownAccount(999)));
}

#[test]
fn disposal_accepts_zero_proceeds() {
    // Scrapping an asset for nothing is legitimate: full loss.
    let (mut ledger, cost, accum, expense) = test_ledger();
    let id = van_asset(&mut ledger, 60_000, cost, accum, expense);
    let asset = ledger.get_asset(id).unwrap().clone();
    let d = disposal_amounts(&asset, date(2026, 6, 30), usd("0")).unwrap();
    assert!(d.gain_loss < Decimal::ZERO);
    assert_eq!(d.book_value + d.gain_loss, Decimal::ZERO);
}

#[test]
fn two_assets_get_distinct_ids_and_list() {
    let (mut ledger, cost, accum, expense) = test_ledger();
    let id1 = van_asset(&mut ledger, 60_000, cost, accum, expense);
    let id2 = van_asset(&mut ledger, 30_000, cost, accum, expense);
    assert_ne!(id1, id2);
    let listed: Vec<_> = ledger.registered_assets().collect();
    assert_eq!(listed.len(), 2);
    assert!(listed.iter().any(|a| a.id == id1));
    assert!(listed.iter().any(|a| a.id == id2));
}

#[test]
fn one_month_useful_life_is_valid() {
    let (mut ledger, cost, accum, expense) = test_ledger();
    let id = ledger
        .register_asset(NewAsset {
            description: "Cheap tool".to_string(),
            in_service_date: date(2026, 3, 10),
            cost: usd("300"),
            salvage_value: usd("0"),
            useful_life_months: 1,
            cost_account: cost,
            accum_depr_account: accum,
            expense_account: expense,
        })
        .unwrap();
    let asset = ledger.get_asset(id).unwrap();
    let sched = depreciation_schedule(asset, date(2026, 12, 31)).unwrap();
    // A one-month life starting Mar 10 spans two calendar months.
    assert_eq!(sched.len(), 2);
    // March: 22/31 of 300 = 212.90.
    assert_eq!(sched[0].amount, Decimal::new(21290, 2));
    // Lifetime total is exactly 300.00.
    let total: Decimal = sched.iter().map(|l| l.amount).sum();
    assert_eq!(total, Decimal::new(30000, 2));
}

#[test]
fn disposal_on_the_purchase_date_is_allowed() {
    let (mut ledger, cost, accum, expense) = test_ledger();
    let id = van_asset(&mut ledger, 60_000, cost, accum, expense);
    let asset = ledger.get_asset(id).unwrap().clone();
    // Held one day: 1,000/month × 1/31 = 32.26 of depreciation.
    let d = disposal_amounts(&asset, date(2026, 1, 15), usd("60000")).unwrap();
    assert_eq!(d.accumulated, Decimal::new(3226, 2));
    assert_eq!(d.book_value, Decimal::new(5_996_774, 2));
    assert_eq!(d.gain_loss, Decimal::new(3226, 2));
    // The day before it is in service nothing has accrued (whether the
    // company held it by then is its acquisition entry's: the ledger
    // checks that floor, Ledger::dispose_asset).
    let d = disposal_amounts(&asset, date(2026, 1, 14), usd("60000")).unwrap();
    assert_eq!(d.accumulated, Decimal::ZERO);
    assert_eq!(d.book_value, asset.cost.amount);
}

/// An asset links to the posted entry that acquired it, if that entry
/// debits its cost account and isn't a reversal or reversed; a correction
/// (reverse, repost) links the replacement. Refusals change nothing.
#[test]
fn acquisition_links_only_to_a_live_entry_debiting_the_cost_account() {
    use ledger_core::{LedgerError, Side};
    let (mut ledger, cost, accum, expense) = test_ledger();
    let capital = ledger
        .add_account("3000", "Capital", AccountType::Equity, None, false)
        .unwrap();
    let id = van_asset(&mut ledger, 1480, cost, accum, expense);
    let post = |l: &mut Ledger, dr: u64, cr: u64| {
        let e = l.create_entry(date(2026, 1, 15), "contribution");
        l.add_line(e, dr, Side::Debit, usd("1480")).unwrap();
        l.add_line(e, cr, Side::Credit, usd("1480")).unwrap();
        l.post_entry(e).unwrap();
        e
    };
    let original = post(&mut ledger, cost, capital);
    let wrong = post(&mut ledger, expense, capital);
    let draft = ledger.create_entry(date(2026, 1, 15), "draft");
    let refused = |l: &mut Ledger, entry: u64| {
        let before = l.clone();
        let err = l.set_asset_acquisition(id, entry).unwrap_err();
        assert_eq!(*l, before);
        err
    };
    assert!(matches!(
        refused(&mut ledger, wrong),
        LedgerError::AcquisitionEntryMismatch { .. }
    ));
    assert!(matches!(
        refused(&mut ledger, draft),
        LedgerError::AcquisitionEntryMismatch { .. }
    ));
    assert!(matches!(
        refused(&mut ledger, 999),
        LedgerError::UnknownEntry(999)
    ));
    assert!(matches!(
        ledger.set_asset_acquisition(99, original),
        Err(LedgerError::UnknownAsset(99))
    ));
    ledger.set_asset_acquisition(id, original).unwrap();
    assert_eq!(ledger.get_asset(id).unwrap().acquired_by, Some(original));

    // Corrected: reversed, reposted. Neither the original nor its
    // reversal can be the link; the replacement can.
    let reversal = ledger
        .reverse_entry(original, date(2026, 1, 16), "joint, not one member's")
        .unwrap();
    let replacement = post(&mut ledger, cost, capital);
    assert!(matches!(
        refused(&mut ledger, original),
        LedgerError::AcquisitionEntryMismatch { .. }
    ));
    assert!(matches!(
        refused(&mut ledger, reversal),
        LedgerError::AcquisitionEntryMismatch { .. }
    ));
    ledger.set_asset_acquisition(id, replacement).unwrap();
    assert_eq!(ledger.get_asset(id).unwrap().acquired_by, Some(replacement));
}
