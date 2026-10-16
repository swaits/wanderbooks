//! Spec-derived tests for fixed assets (depreciation, disposal) and payroll.
//!
//! Every expected value here is a literal worked by hand from the written
//! specification (the interface doc comments, `docs/spec/rules.md` B-15 and
//! B-22, and IRS Pub. 15 in `docs/spec/sources/irs-payroll.md`), or a law the
//! specification states. Nothing is read back from the code under test and
//! then asserted.

use std::str::FromStr;

use chrono::{Datelike, Months, NaiveDate};
use ledger_core::payroll::{
    PayrollAccounts, payroll_entry_plan, payroll_remit_plan, validate_payroll_lines,
};
use ledger_core::{
    AccountId, AccountType, Asset, AssetId, AssetStatus, Currency, DepreciationMethod, EntryId,
    Ledger, LedgerError, Money, NewAsset, PayrollLine, Side, accumulated_depreciation, book_value,
    depreciation_schedule, disposal_amounts,
};
use proptest::prelude::*;
use rust_decimal::Decimal;

// ---------------------------------------------------------------- helpers

fn d(s: &str) -> Decimal {
    Decimal::from_str(s).expect("decimal literal")
}

fn usd(s: &str) -> Money {
    Money::new(d(s), Currency::USD)
}

fn date(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).expect("valid date")
}

fn month_end(day: NaiveDate) -> NaiveDate {
    let first = NaiveDate::from_ymd_opt(day.year(), day.month(), 1).expect("first of month");
    first
        .checked_add_months(Months::new(1))
        .expect("next month")
        .pred_opt()
        .expect("previous day")
}

/// A free-standing asset for the pure functions (account ids are opaque
/// there; nothing looks them up).
fn asset(cost: &str, salvage: &str, life: u32, purchase: NaiveDate) -> Asset {
    Asset {
        id: 1,
        description: "Test asset".to_string(),
        purchase_date: purchase,
        cost: usd(cost),
        salvage_value: usd(salvage),
        useful_life_months: life,
        method: DepreciationMethod::StraightLine,
        cost_account: 10,
        accum_depr_account: 11,
        expense_account: 12,
        status: AssetStatus::Active,
        tag: None,
        serial: None,
        acquired_by: None,
        tax: None,
    }
}

/// The accounts a fixture ledger holds.
struct Accts {
    cash: AccountId,
    capital: AccountId,
    equipment: AccountId,
    accum: AccountId,
    depr_expense: AccountId,
    liability: AccountId,
}

/// A USD ledger with a chart for assets and payroll, funded with $10,000
/// cash on 2026-01-01 (Dr 1000 / Cr 3000).
fn fixture() -> (Ledger, Accts) {
    let mut l = Ledger::new(Currency::USD);
    let cash = l
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    let equipment = l
        .add_account("1500", "Equipment", AccountType::Asset, None, false)
        .unwrap();
    let accum = l
        .add_account(
            "1510",
            "Accumulated Depreciation",
            AccountType::Asset,
            None,
            false,
        )
        .unwrap();
    let liability = l
        .add_account("2500", "Loan", AccountType::Liability, None, false)
        .unwrap();
    let capital = l
        .add_account("3000", "Owner Capital", AccountType::Equity, None, false)
        .unwrap();
    let depr_expense = l
        .add_account(
            "6200",
            "Depreciation Expense",
            AccountType::Expense,
            None,
            false,
        )
        .unwrap();
    let e = l.try_create_entry(date(2026, 1, 1), "Opening").unwrap();
    l.add_line(e, cash, Side::Debit, usd("10000.00")).unwrap();
    l.add_line(e, capital, Side::Credit, usd("10000.00"))
        .unwrap();
    l.post_entry(e).unwrap();
    (
        l,
        Accts {
            cash,
            capital,
            equipment,
            accum,
            depr_expense,
            liability,
        },
    )
}

fn new_asset(a: &Accts) -> NewAsset {
    NewAsset {
        description: "Laptop".to_string(),
        purchase_date: date(2026, 1, 1),
        cost: usd("12000.00"),
        salvage_value: usd("0.00"),
        useful_life_months: 12,
        cost_account: a.equipment,
        accum_depr_account: a.accum,
        expense_account: a.depr_expense,
    }
}

/// Everything observable about the books, for "a refusal leaves the books
/// unchanged".
#[derive(Debug, PartialEq)]
struct Snapshot {
    accounts: Vec<(String, AccountType, Decimal)>,
    posted: Vec<EntryId>,
    next_entry: EntryId,
    next_asset: AssetId,
    next_run: u64,
    assets: Vec<Asset>,
    runs: usize,
}

fn snapshot(l: &Ledger) -> Snapshot {
    let mut accounts: Vec<(String, AccountType, Decimal)> = l
        .chart()
        .all()
        .map(|a| {
            (
                a.code.clone(),
                a.account_type,
                l.account_balance(a.id).unwrap(),
            )
        })
        .collect();
    accounts.sort_by(|x, y| x.0.cmp(&y.0));
    Snapshot {
        accounts,
        posted: l.posted_entries().map(|e| e.id()).collect(),
        next_entry: l.next_entry_id(),
        next_asset: l.next_asset_id(),
        next_run: l.next_payroll_run_id(),
        assets: l.registered_assets().cloned().collect(),
        runs: l.list_payroll_runs().len(),
    }
}

fn balance_of(l: &Ledger, code: &str) -> Decimal {
    let acct = l.chart().find_by_code(code).expect("account exists");
    l.account_balance(acct.id).unwrap()
}

#[allow(clippy::too_many_arguments)]
fn pline(
    name: &str,
    gross: &str,
    fed: &str,
    state: &str,
    ss_e: &str,
    med_e: &str,
    ss_r: &str,
    med_r: &str,
    other: &str,
) -> PayrollLine {
    PayrollLine {
        employee_name: name.to_string(),
        employee: None,
        gross_pay: d(gross),
        federal_withholding: d(fed),
        state_withholding: d(state),
        ss_employee: d(ss_e),
        medicare_employee: d(med_e),
        ss_employer: d(ss_r),
        medicare_employer: d(med_r),
        other_deductions: d(other),
    }
}

/// $5,000 gross; FICA at the Pub. 15 2026 rates: SS 6.2% = 310.00 each
/// share, Medicare 1.45% = 72.50 each share.
fn alice() -> PayrollLine {
    pline(
        "Alice", "5000.00", "600.00", "250.00", "310.00", "72.50", "310.00", "72.50", "200.00",
    )
}

/// $2,000 gross, no state withholding, no other deductions.
fn bob() -> PayrollLine {
    pline(
        "Bob", "2000.00", "150.00", "0", "124.00", "29.00", "124.00", "29.00", "0",
    )
}

fn entry_lines_by_code(l: &Ledger, e: EntryId) -> Vec<(String, Side, Decimal)> {
    l.get_entry(e)
        .expect("entry exists")
        .lines()
        .iter()
        .map(|ln| {
            (
                l.chart().get(ln.account).unwrap().code.clone(),
                ln.side,
                ln.amount.amount,
            )
        })
        .collect()
}

fn ent(code: &str, side: Side, amt: &str) -> (String, Side, Decimal) {
    (code.to_string(), side, d(amt))
}

fn plan_totals(plan: &[(AccountId, Side, Money)]) -> (Decimal, Decimal) {
    let mut dr = Decimal::ZERO;
    let mut cr = Decimal::ZERO;
    for (_, side, m) in plan {
        match side {
            Side::Debit => dr += m.amount,
            Side::Credit => cr += m.amount,
        }
    }
    (dr, cr)
}

fn plan_sum(plan: &[(AccountId, Side, Money)], acct: AccountId, side: Side) -> Decimal {
    plan.iter()
        .filter(|(a, s, _)| *a == acct && *s == side)
        .map(|(_, _, m)| m.amount)
        .sum()
}

const PA: PayrollAccounts = PayrollAccounts {
    salary_expense: 1,
    payroll_tax_expense: 2,
    federal_payable: 3,
    state_payable: 4,
    ss_payable: 5,
    medicare_payable: 6,
    other_payable: 7,
};
const PLAN_CASH: AccountId = 100;

// ------------------------------------------------- depreciation schedule

/// Oracle: B-15 straight line, (cost − salvage) / life = 12000 / 12 = 1000.00
/// per full month; a purchase on the 1st makes every month full.
#[test]
fn full_months_each_take_one_twelfth_b15() {
    let a = asset("12000.00", "0.00", 12, date(2026, 1, 1));
    let s = depreciation_schedule(&a, date(2026, 12, 31));
    assert_eq!(s.len(), 12);
    for (i, line) in s.iter().enumerate() {
        let n = i as i64 + 1;
        assert_eq!(line.period, format!("2026-{:02}", n));
        assert_eq!(line.amount, d("1000.00"));
        assert_eq!(line.accumulated, Decimal::from(1000 * n));
        assert_eq!(line.book_value, Decimal::from(12000 - 1000 * n));
    }
}

/// Oracle: depreciation_schedule SCENARIO 1 (DECISION, Stephen 2026-10-13,
/// resolving this test's original disagreement in favor of the code): a
/// schedule through 2026-06-15 prorates June by days: 15/30 of 1,000.
#[test]
fn schedule_through_a_mid_month_date_prorates_that_month_by_days() {
    let a = asset("12000.00", "0.00", 12, date(2026, 1, 1));
    let s = depreciation_schedule(&a, date(2026, 6, 15));
    assert_eq!(s.len(), 6);
    assert_eq!(s.last().unwrap().period, "2026-06");
    assert_eq!(s.last().unwrap().amount, d("500.00"));
    assert_eq!(s.last().unwrap().accumulated, d("5500.00"));
}

/// Oracle: interface doc "clamped to the depreciation end": a schedule asked
/// through 2030 still totals cost − salvage = 12000.00, all in 12 non-zero
/// months.
#[test]
fn schedule_clamps_to_end_of_life() {
    let a = asset("12000.00", "0.00", 12, date(2026, 1, 1));
    let s = depreciation_schedule(&a, date(2030, 12, 31));
    let total: Decimal = s.iter().map(|l| l.amount).sum();
    assert_eq!(total, d("12000.00"));
    assert_eq!(s.iter().filter(|l| !l.amount.is_zero()).count(), 12);
    assert_eq!(s.last().unwrap().book_value, d("0.00"));
}

/// Oracle: B-15 partial months prorated by days; monthly 620/2 = 310.00.
/// Bought 2026-01-15: Jan holds 15..31 = 17/31 → 170.00; Feb full 310.00;
/// life ends 2026-03-15 so Mar holds 1..14 = 14/31 → 140.00. 17/31 + 14/31
/// = one full month, and the total is exactly 620.00.
///
/// CONVENTION (docs/testing.md #9): life ends on purchase_date + useful_life_months and the end day
/// itself is not held (matches the doc's "17/31 + 14/30" pairing for a
/// purchase on the 15th).
#[test]
fn partial_first_and_last_months_make_one_full_month_b15() {
    let a = asset("620.00", "0.00", 2, date(2026, 1, 15));
    let s = depreciation_schedule(&a, date(2027, 12, 31));
    let got: Vec<(&str, Decimal, Decimal, Decimal)> = s
        .iter()
        .map(|l| (l.period.as_str(), l.amount, l.accumulated, l.book_value))
        .collect();
    assert_eq!(
        got,
        vec![
            ("2026-01", d("170.00"), d("170.00"), d("450.00")),
            ("2026-02", d("310.00"), d("480.00"), d("140.00")),
            ("2026-03", d("140.00"), d("620.00"), d("0.00")),
        ]
    );
}

/// Oracle: interface doc "17/31 + 14/30 ≠ 1, so the last month absorbs the
/// drift and the lifetime total is exactly (cost − salvage)". Depreciable
/// 1030 − 100 = 930.00 over 3 months = 310.00/month. Bought 2026-01-15:
/// Jan 17/31 → 170.00, Feb 310.00, Mar 310.00 (acc 790.00), Apr (30 days)
/// trues up to 930 − 790 = 140.00, not 14/30 × 310 = 144.67.
///
/// CONVENTION (#9): same end-of-life convention as the test above.
#[test]
fn last_month_trues_up_unequal_month_lengths_b15() {
    let a = asset("1030.00", "100.00", 3, date(2026, 1, 15));
    let s = depreciation_schedule(&a, date(2027, 12, 31));
    let got: Vec<(&str, Decimal, Decimal, Decimal)> = s
        .iter()
        .map(|l| (l.period.as_str(), l.amount, l.accumulated, l.book_value))
        .collect();
    assert_eq!(
        got,
        vec![
            ("2026-01", d("170.00"), d("170.00"), d("860.00")),
            ("2026-02", d("310.00"), d("480.00"), d("550.00")),
            ("2026-03", d("310.00"), d("790.00"), d("240.00")),
            ("2026-04", d("140.00"), d("930.00"), d("100.00")),
        ]
    );
}

/// Oracle: B-15 "salvage never depreciated": cost 5000, salvage 1000, life 4
/// → 1000.00/month, book value stops at 1000.00 forever.
#[test]
fn salvage_value_is_never_depreciated_b15() {
    let a = asset("5000.00", "1000.00", 4, date(2026, 3, 1));
    let s = depreciation_schedule(&a, date(2026, 6, 30));
    let amounts: Vec<Decimal> = s.iter().map(|l| l.amount).collect();
    assert_eq!(amounts, vec![d("1000.00"); 4]);
    assert_eq!(s.last().unwrap().book_value, d("1000.00"));
    assert_eq!(book_value(&a, date(2040, 1, 31)), d("1000.00"));
    assert_eq!(
        accumulated_depreciation(&a, date(2040, 1, 31)),
        d("4000.00")
    );
}

/// Oracle: B-21/interface "exact cents … never a penny off": 1000.00 over 3
/// months cannot split evenly; months are 333.33 or 333.34 and sum to
/// exactly 1000.00.
#[test]
fn uneven_split_rounds_to_cents_and_sums_exactly() {
    let a = asset("1000.00", "0.00", 3, date(2026, 1, 1));
    let s = depreciation_schedule(&a, date(2026, 3, 31));
    assert_eq!(s.len(), 3);
    for l in &s {
        assert!(
            l.amount == d("333.33") || l.amount == d("333.34"),
            "month {} = {}",
            l.period,
            l.amount
        );
    }
    let total: Decimal = s.iter().map(|l| l.amount).sum();
    assert_eq!(total, d("1000.00"));
}

/// Oracle: interface doc "from the purchase month": nothing is depreciated
/// before purchase.
///
/// ASSUMED: a `through` date before the purchase month yields an empty
/// schedule, and accumulated depreciation before purchase is 0.
#[test]
fn nothing_depreciates_before_purchase() {
    let a = asset("12000.00", "0.00", 12, date(2026, 1, 1));
    assert!(depreciation_schedule(&a, date(2025, 12, 31)).is_empty());
    assert_eq!(accumulated_depreciation(&a, date(2025, 12, 31)), d("0"));
    assert_eq!(book_value(&a, date(2025, 12, 31)), d("12000.00"));
}

/// Oracle: interface docs "accumulated through as_of" and "book value =
/// cost − accumulated": mid-life at 2026-06-30 = 6 × 1000.00.
#[test]
fn accumulated_and_book_value_mid_life() {
    let a = asset("12000.00", "0.00", 12, date(2026, 1, 1));
    assert_eq!(
        accumulated_depreciation(&a, date(2026, 6, 30)),
        d("6000.00")
    );
    assert_eq!(book_value(&a, date(2026, 6, 30)), d("6000.00"));
}

// ------------------------------------------------------------- disposal

/// Oracle: dispose_asset INVARIANT 1, gain = proceeds − book value:
/// 7000 − (12000 − 6000) = +1000.00.
#[test]
fn disposal_gain_is_proceeds_minus_book_value() {
    let a = asset("12000.00", "0.00", 12, date(2026, 1, 1));
    let got = disposal_amounts(&a, date(2026, 6, 30), usd("7000.00")).unwrap();
    assert_eq!(got.accumulated, d("6000.00"));
    assert_eq!(got.book_value, d("6000.00"));
    assert_eq!(got.gain_loss, d("1000.00"));
}

/// Oracle: DisposalAmounts doc "loss is negative": 5000 − 6000 = −1000.00;
/// scrapping for 0 loses the whole book value, −6000.00.
#[test]
fn disposal_loss_is_negative() {
    let a = asset("12000.00", "0.00", 12, date(2026, 1, 1));
    let got = disposal_amounts(&a, date(2026, 6, 30), usd("5000.00")).unwrap();
    assert_eq!(got.gain_loss, d("-1000.00"));
    let scrap = disposal_amounts(&a, date(2026, 6, 30), usd("0.00")).unwrap();
    assert_eq!(scrap.gain_loss, d("-6000.00"));
}

/// Oracle: B-15 + gain law: after full life, book value = salvage 1000.00,
/// selling for 1000.00 is neither gain nor loss.
#[test]
fn disposal_after_full_life_at_salvage_is_break_even() {
    let a = asset("5000.00", "1000.00", 4, date(2026, 3, 1));
    let got = disposal_amounts(&a, date(2027, 12, 31), usd("1000.00")).unwrap();
    assert_eq!(got.accumulated, d("4000.00"));
    assert_eq!(got.book_value, d("1000.00"));
    assert_eq!(got.gain_loss, d("0"));
}

/// Oracle: LedgerError::NegativeProceeds doc "disposal proceeds must not be
/// negative".
#[test]
fn disposal_refuses_negative_proceeds() {
    let a = asset("12000.00", "0.00", 12, date(2026, 1, 1));
    assert_eq!(
        disposal_amounts(&a, date(2026, 6, 30), usd("-1.00")),
        Err(LedgerError::NegativeProceeds(d("-1.00")))
    );
}

/// Oracle: LedgerError::DisposalBeforePurchase doc "disposal date must not
/// precede the purchase date".
#[test]
fn disposal_refuses_date_before_purchase() {
    let a = asset("12000.00", "0.00", 12, date(2026, 1, 1));
    assert!(matches!(
        disposal_amounts(&a, date(2025, 12, 31), usd("100.00")),
        Err(LedgerError::DisposalBeforePurchase(_))
    ));
}

// ------------------------------------------------- register / dispose

/// Oracle: register_asset GOAL + next_asset_id doc: a valid asset is
/// registered under the announced id, Active, straight-line, with its inputs.
#[test]
fn register_asset_records_inputs_under_next_id() {
    let (mut l, a) = fixture();
    let new = new_asset(&a);
    assert_eq!(l.validate_new_asset(&new), Ok(()));
    let expected_id = l.next_asset_id();
    let id = l.register_asset(new).unwrap();
    assert_eq!(id, expected_id);
    let got = l.get_asset(id).unwrap();
    assert_eq!(got.description, "Laptop");
    assert_eq!(got.purchase_date, date(2026, 1, 1));
    assert_eq!(got.cost, usd("12000.00"));
    assert_eq!(got.salvage_value, usd("0.00"));
    assert_eq!(got.useful_life_months, 12);
    assert_eq!(got.method, DepreciationMethod::StraightLine);
    assert_eq!(got.cost_account, a.equipment);
    assert_eq!(got.accum_depr_account, a.accum);
    assert_eq!(got.expense_account, a.depr_expense);
    assert_eq!(got.status, AssetStatus::Active);
    assert_eq!(l.registered_assets().count(), 1);
}

/// Checks a refusal from both validate_new_asset and register_asset, and that
/// neither changed the books.
fn assert_asset_refused(new: NewAsset, l: &mut Ledger, check: impl Fn(&LedgerError) -> bool) {
    let before = snapshot(l);
    let v = l.validate_new_asset(&new);
    assert!(
        matches!(&v, Err(e) if check(e)),
        "validate_new_asset: {v:?}"
    );
    let r = l.register_asset(new);
    assert!(matches!(&r, Err(e) if check(e)), "register_asset: {r:?}");
    assert_eq!(snapshot(l), before, "refusal changed the books");
}

/// Oracle: register_asset SCENARIO 1 "description is blank: SHOULD refuse";
/// refusal leaves the books unchanged (B-8 / post_entry INVARIANT 3 spirit).
#[test]
fn register_asset_refuses_empty_description() {
    let (mut l, a) = fixture();
    let mut n = new_asset(&a);
    n.description = String::new();
    assert_asset_refused(n, &mut l, |e| *e == LedgerError::EmptyAssetDescription);
}

/// Oracle: register_asset SCENARIO 1 says "blank", not just empty.
///
/// ASSUMED: whitespace-only counts as blank.
#[test]
fn register_asset_refuses_whitespace_description() {
    let (mut l, a) = fixture();
    let mut n = new_asset(&a);
    n.description = "   ".to_string();
    assert_asset_refused(n, &mut l, |e| *e == LedgerError::EmptyAssetDescription);
}

/// Oracle: register_asset SCENARIO 1 "cost isn't positive" → InvalidAssetCost.
#[test]
fn register_asset_refuses_non_positive_cost() {
    for cost in ["0.00", "-5.00"] {
        let (mut l, a) = fixture();
        let mut n = new_asset(&a);
        n.cost = usd(cost);
        n.salvage_value = usd("0.00");
        let want = d(cost);
        assert_asset_refused(
            n,
            &mut l,
            |e| matches!(e, LedgerError::InvalidAssetCost(c) if *c == want),
        );
    }
}

/// Oracle: register_asset SCENARIO 1 "salvage is negative or not below cost".
#[test]
fn register_asset_refuses_bad_salvage() {
    for salvage in ["-0.01", "12000.00", "12000.01"] {
        let (mut l, a) = fixture();
        let mut n = new_asset(&a);
        n.salvage_value = usd(salvage);
        assert_asset_refused(n, &mut l, |e| {
            matches!(e, LedgerError::InvalidSalvageValue(_))
        });
    }
}

/// Oracle: register_asset SCENARIO 1 "useful life is zero months".
#[test]
fn register_asset_refuses_zero_life() {
    let (mut l, a) = fixture();
    let mut n = new_asset(&a);
    n.useful_life_months = 0;
    assert_asset_refused(n, &mut l, |e| *e == LedgerError::InvalidUsefulLife(0));
}

/// Oracle: register_asset doc "asset/asset/expense types" and
/// AssetAccountWrongType's role names.
#[test]
fn register_asset_refuses_wrong_account_types() {
    // cost account is equity
    let (mut l, a) = fixture();
    let mut n = new_asset(&a);
    n.cost_account = a.capital;
    assert_asset_refused(
        n,
        &mut l,
        |e| matches!(e, LedgerError::AssetAccountWrongType { role, .. } if *role == "cost"),
    );

    // accumulated depreciation is a liability
    let (mut l, a) = fixture();
    let mut n = new_asset(&a);
    n.accum_depr_account = a.liability;
    assert_asset_refused(
        n,
        &mut l,
        |e| matches!(e, LedgerError::AssetAccountWrongType { role, .. } if *role == "accumulated depreciation"),
    );

    // depreciation expense is equity
    let (mut l, a) = fixture();
    let mut n = new_asset(&a);
    n.expense_account = a.capital;
    assert_asset_refused(
        n,
        &mut l,
        |e| matches!(e, LedgerError::AssetAccountWrongType { role, .. } if *role == "depreciation expense"),
    );
}

/// Oracle: register_asset doc "three distinct accounts" → DuplicateAssetAccount.
#[test]
fn register_asset_refuses_duplicate_account_links() {
    let (mut l, a) = fixture();
    let mut n = new_asset(&a);
    n.accum_depr_account = a.equipment;
    let dup = a.equipment;
    assert_asset_refused(n, &mut l, |e| *e == LedgerError::DuplicateAssetAccount(dup));
}

/// Oracle: NewAsset doc "cost … in the ledger's base currency".
///
/// ASSUMED: the error variant is unspecified; any refusal passes.
#[test]
fn register_asset_refuses_foreign_currency() {
    let (mut l, a) = fixture();
    let mut n = new_asset(&a);
    n.cost = Money::new(d("12000.00"), Currency::EUR);
    n.salvage_value = Money::new(d("0.00"), Currency::EUR);
    assert_asset_refused(n, &mut l, |_| true);
}

/// Oracle: dispose_asset doc "records the lifecycle change" → status
/// Disposed { date, proceeds }.
#[test]
fn dispose_asset_marks_disposed() {
    let (mut l, a) = fixture();
    let id = l.register_asset(new_asset(&a)).unwrap();
    l.dispose_asset(id, date(2026, 6, 30), usd("7000.00"))
        .unwrap();
    assert_eq!(
        l.get_asset(id).unwrap().status,
        AssetStatus::Disposed {
            date: date(2026, 6, 30),
            proceeds: usd("7000.00"),
        }
    );
}

/// Oracle: dispose_asset SCENARIO 1 "unknown or already disposed: SHOULD
/// refuse", leaving the books unchanged.
#[test]
fn dispose_asset_refuses_unknown_and_twice() {
    let (mut l, a) = fixture();
    let id = l.register_asset(new_asset(&a)).unwrap();
    let before = snapshot(&l);
    assert_eq!(
        l.dispose_asset(id + 100, date(2026, 6, 30), usd("1.00")),
        Err(LedgerError::UnknownAsset(id + 100))
    );
    assert_eq!(snapshot(&l), before);

    l.dispose_asset(id, date(2026, 6, 30), usd("7000.00"))
        .unwrap();
    let before = snapshot(&l);
    assert_eq!(
        l.dispose_asset(id, date(2026, 7, 31), usd("1.00")),
        Err(LedgerError::AssetAlreadyDisposed(id))
    );
    assert_eq!(snapshot(&l), before);
}

/// Oracle: NegativeProceeds / DisposalBeforePurchase error docs.
///
/// ASSUMED: dispose_asset applies the same disposal checks as
/// disposal_amounts (it records the lifecycle change of a valid disposal).
#[test]
fn dispose_asset_refuses_bad_disposals_unchanged() {
    let (mut l, a) = fixture();
    let id = l.register_asset(new_asset(&a)).unwrap();
    let before = snapshot(&l);
    assert_eq!(
        l.dispose_asset(id, date(2026, 6, 30), usd("-1.00")),
        Err(LedgerError::NegativeProceeds(d("-1.00")))
    );
    assert!(matches!(
        l.dispose_asset(id, date(2025, 12, 1), usd("1.00")),
        Err(LedgerError::DisposalBeforePurchase(_))
    ));
    assert_eq!(snapshot(&l), before);
}

// ---------------------------------------------------------------- payroll

/// Oracle: PayrollLine::net_pay doc "gross minus all employee withholdings
/// and deductions": 5000 − 600 − 250 − 310 − 72.50 − 200 = 3567.50 (employer
/// shares are not withheld from the employee).
#[test]
fn net_pay_is_gross_minus_employee_withholdings() {
    assert_eq!(alice().net_pay().unwrap(), d("3567.50"));
    assert_eq!(bob().net_pay().unwrap(), d("1697.00"));
}

/// Oracle: validate_payroll_lines doc "non-empty".
#[test]
fn validate_lines_refuses_empty() {
    assert_eq!(validate_payroll_lines(&[]), Err(LedgerError::EmptyPayroll));
}

/// Oracle: validate_payroll_lines doc "no negative amounts", every field.
#[test]
fn validate_lines_refuses_each_negative_field() {
    for field in 0..8 {
        let mut ln = alice();
        let neg = d("-1.00");
        match field {
            0 => ln.gross_pay = neg,
            1 => ln.federal_withholding = neg,
            2 => ln.state_withholding = neg,
            3 => ln.ss_employee = neg,
            4 => ln.medicare_employee = neg,
            5 => ln.ss_employer = neg,
            6 => ln.medicare_employer = neg,
            _ => ln.other_deductions = neg,
        }
        assert_eq!(
            validate_payroll_lines(&[ln]),
            Err(LedgerError::NegativePayrollAmount(neg)),
            "field {field}"
        );
    }
}

/// Oracle: validate_payroll_lines doc "no negative net pay": withholdings of
/// 5000.01 against 5000.00 gross.
#[test]
fn validate_lines_refuses_negative_net_pay() {
    let mut ln = alice();
    ln.other_deductions = d("3767.51"); // 600+250+310+72.50+3767.51 = 5000.01
    assert_eq!(
        validate_payroll_lines(&[ln]),
        Err(LedgerError::NegativeNetPay("Alice".to_string()))
    );
}

/// Oracle: net_pay doc "Must be >= 0": exactly zero net pay is allowed; the
/// employer shares may exceed what's left (they're not withheld).
#[test]
fn validate_lines_accepts_zero_net_pay() {
    let mut ln = alice();
    ln.other_deductions = d("3767.50");
    assert_eq!(ln.net_pay().unwrap(), d("0"));
    assert_eq!(validate_payroll_lines(&[ln]), Ok(()));
}

/// Oracle: payroll_entry_plan doc + run_payroll INVARIANT 1, worked by hand:
/// Dr 6100 5000.00, Dr 6110 382.50 (310 + 72.50), Cr 2110 600, Cr 2120 250,
/// Cr 2130 620 (both SS shares), Cr 2140 145 (both Medicare), Cr 2150 200,
/// Cr cash 3567.50. Debits = credits = 5382.50.
#[test]
fn entry_plan_matches_hand_worked_entry() {
    let plan = payroll_entry_plan(&[alice()], PA, PLAN_CASH, Currency::USD).unwrap();
    let got: Vec<(AccountId, Side, Money)> = plan;
    let want = [
        (1, Side::Debit, usd("5000.00")),
        (2, Side::Debit, usd("382.50")),
        (3, Side::Credit, usd("600.00")),
        (4, Side::Credit, usd("250.00")),
        (5, Side::Credit, usd("620.00")),
        (6, Side::Credit, usd("145.00")),
        (7, Side::Credit, usd("200.00")),
        (PLAN_CASH, Side::Credit, usd("3567.50")),
    ];
    assert_eq!(got.len(), want.len(), "{got:?}");
    for (g, w) in got.iter().zip(want.iter()) {
        assert_eq!(
            (g.0, g.1, g.2.amount, g.2.currency),
            (w.0, w.1, w.2.amount, w.2.currency)
        );
    }
}

/// Oracle: payroll_entry_plan doc "Zero totals are skipped": Bob has no state
/// withholding and no other deductions, so no 2120 or 2150 line.
#[test]
fn entry_plan_skips_zero_totals() {
    let plan = payroll_entry_plan(&[bob()], PA, PLAN_CASH, Currency::USD).unwrap();
    assert!(plan.iter().all(|(_, _, m)| !m.amount.is_zero()));
    assert!(
        plan.iter()
            .all(|(a, _, _)| *a != PA.state_payable && *a != PA.other_payable)
    );
    assert_eq!(plan_totals(&plan), (d("2153.00"), d("2153.00")));
}

/// Oracle: payroll_remit_plan doc "Dr each payable with a non-zero balance,
/// Cr cash for the total": 600 + 145 = 745.00; the zero is skipped.
#[test]
fn remit_plan_hand_worked() {
    let plan = payroll_remit_plan(
        &[(3, d("600.00")), (4, d("0")), (6, d("145.00"))],
        PLAN_CASH,
        Currency::USD,
    );
    let got: Vec<(AccountId, Side, Decimal)> =
        plan.iter().map(|(a, s, m)| (*a, *s, m.amount)).collect();
    assert_eq!(
        got,
        vec![
            (3, Side::Debit, d("600.00")),
            (6, Side::Debit, d("145.00")),
            (PLAN_CASH, Side::Credit, d("745.00")),
        ]
    );
}

/// Oracle: run_payroll INVARIANTS 1–2 and doc (auto-create 6100/6110/
/// 2110–2150), hand-worked Alice entry; balances after: cash 10000 − 3567.50
/// = 6432.50; payables as in the plan.
#[test]
fn run_payroll_posts_hand_worked_entry() {
    let (mut l, a) = fixture();
    assert_eq!(
        l.validate_payroll(date(2026, 2, 27), &[alice()], a.cash),
        Ok(())
    );
    let next = l.next_payroll_run_id();
    let id = l
        .run_payroll(date(2026, 2, 27), vec![alice()], a.cash)
        .unwrap();
    assert_eq!(id, next);
    let run = l.get_payroll_run(id).unwrap().clone();
    assert_eq!(run.date, date(2026, 2, 27));
    assert_eq!(run.lines, vec![alice()]);
    let e = l.get_entry(run.entry_id).unwrap();
    assert_eq!(e.status(), ledger_core::EntryStatus::Posted);
    assert_eq!(e.date(), date(2026, 2, 27));
    assert_eq!(
        entry_lines_by_code(&l, run.entry_id),
        vec![
            ent("6100", Side::Debit, "5000.00"),
            ent("6110", Side::Debit, "382.50"),
            ent("2110", Side::Credit, "600.00"),
            ent("2120", Side::Credit, "250.00"),
            ent("2130", Side::Credit, "620.00"),
            ent("2140", Side::Credit, "145.00"),
            ent("2150", Side::Credit, "200.00"),
            ent("1000", Side::Credit, "3567.50"),
        ]
    );
    assert_eq!(balance_of(&l, "1000"), d("6432.50"));
    assert_eq!(balance_of(&l, "6100"), d("5000.00"));
    assert_eq!(balance_of(&l, "6110"), d("382.50"));
    assert_eq!(balance_of(&l, "2130"), d("620.00"));
}

/// Oracle: payroll_entry_plan doc (one line per account total) with two
/// employees: gross 7000.00, employer FICA 382.50 + 153.00 = 535.50, fed
/// 750.00, state 250.00, SS 620 + 248 = 868.00, Medicare 145 + 58 = 203.00,
/// other 200.00, net 3567.50 + 1697.00 = 5264.50.
///
/// CONFIRMED (docs/testing.md #10): lines are aggregated per account ("zero totals are skipped"
/// implies totals), not one line per employee.
#[test]
fn run_payroll_aggregates_employees() {
    let (mut l, a) = fixture();
    let id = l
        .run_payroll(date(2026, 2, 27), vec![alice(), bob()], a.cash)
        .unwrap();
    let entry = l.get_payroll_run(id).unwrap().entry_id;
    assert_eq!(
        entry_lines_by_code(&l, entry),
        vec![
            ent("6100", Side::Debit, "7000.00"),
            ent("6110", Side::Debit, "535.50"),
            ent("2110", Side::Credit, "750.00"),
            ent("2120", Side::Credit, "250.00"),
            ent("2130", Side::Credit, "868.00"),
            ent("2140", Side::Credit, "203.00"),
            ent("2150", Side::Credit, "200.00"),
            ent("1000", Side::Credit, "5264.50"),
        ]
    );
}

/// Oracle: payroll_tax_payables doc (code order, normal-side, zero/missing
/// skipped) and remit_payroll_taxes INVARIANT 1 "MUST zero every payroll
/// payable against cash": remit 600 + 250 + 620 + 145 + 200 = 1815.00, cash
/// 6432.50 → 4617.50; a second remit refuses with NothingToRemit, unchanged.
#[test]
fn remit_zeroes_every_payable() {
    let (mut l, a) = fixture();
    assert!(l.payroll_tax_payables().is_empty());
    l.run_payroll(date(2026, 2, 27), vec![alice()], a.cash)
        .unwrap();
    let codes: Vec<(String, Decimal)> = l
        .payroll_tax_payables()
        .into_iter()
        .map(|(id, bal)| (l.chart().get(id).unwrap().code.clone(), bal))
        .collect();
    assert_eq!(
        codes,
        vec![
            ("2110".to_string(), d("600.00")),
            ("2120".to_string(), d("250.00")),
            ("2130".to_string(), d("620.00")),
            ("2140".to_string(), d("145.00")),
            ("2150".to_string(), d("200.00")),
        ]
    );
    let e = l.remit_payroll_taxes(date(2026, 3, 15), a.cash).unwrap();
    assert_eq!(
        entry_lines_by_code(&l, e),
        vec![
            ent("2110", Side::Debit, "600.00"),
            ent("2120", Side::Debit, "250.00"),
            ent("2130", Side::Debit, "620.00"),
            ent("2140", Side::Debit, "145.00"),
            ent("2150", Side::Debit, "200.00"),
            ent("1000", Side::Credit, "1815.00"),
        ]
    );
    for code in ["2110", "2120", "2130", "2140", "2150"] {
        assert_eq!(balance_of(&l, code), d("0"), "{code}");
    }
    assert_eq!(balance_of(&l, "1000"), d("4617.50"));
    assert!(l.payroll_tax_payables().is_empty());

    let before = snapshot(&l);
    assert_eq!(
        l.remit_payroll_taxes(date(2026, 3, 16), a.cash),
        Err(LedgerError::NothingToRemit)
    );
    assert_eq!(snapshot(&l), before);
}

/// Oracle: remit_payroll_taxes SCENARIO 1 on fresh books: nothing owed.
#[test]
fn remit_with_nothing_owed_refuses() {
    let (mut l, a) = fixture();
    let before = snapshot(&l);
    assert_eq!(
        l.remit_payroll_taxes(date(2026, 3, 15), a.cash),
        Err(LedgerError::NothingToRemit)
    );
    assert_eq!(snapshot(&l), before);
}

fn assert_payroll_refused(
    l: &mut Ledger,
    when: NaiveDate,
    lines: Vec<PayrollLine>,
    cash: AccountId,
    check: impl Fn(&LedgerError) -> bool,
) {
    let before = snapshot(l);
    let v = l.validate_payroll(when, &lines, cash);
    assert!(matches!(&v, Err(e) if check(e)), "validate_payroll: {v:?}");
    let r = l.run_payroll(when, lines, cash);
    assert!(matches!(&r, Err(e) if check(e)), "run_payroll: {r:?}");
    assert_eq!(snapshot(l), before, "refusal changed the books");
}

/// Oracle: run_payroll SCENARIO 1 (negative amount, withholdings exceed
/// gross) and validate_payroll_lines (empty); a refusal leaves the books —
/// including the chart, which must not gain payroll accounts — unchanged.
#[test]
fn run_payroll_refusals_leave_books_unchanged() {
    let (mut l, a) = fixture();
    assert_payroll_refused(&mut l, date(2026, 2, 27), vec![], a.cash, |e| {
        *e == LedgerError::EmptyPayroll
    });
    let mut neg = alice();
    neg.state_withholding = d("-0.01");
    assert_payroll_refused(&mut l, date(2026, 2, 27), vec![bob(), neg], a.cash, |e| {
        *e == LedgerError::NegativePayrollAmount(d("-0.01"))
    });
    let mut over = bob();
    over.federal_withholding = d("1847.01"); // 1847.01+124+29 = 2000.01
    assert_payroll_refused(
        &mut l,
        date(2026, 2, 27),
        vec![alice(), over],
        a.cash,
        |e| *e == LedgerError::NegativeNetPay("Bob".to_string()),
    );
}

/// Oracle: run_payroll SCENARIO 1 "gross is zero: SHOULD refuse".
///
/// ASSUMED: a run whose only line is all zeros is "gross is zero"; the error
/// variant is unspecified.
#[test]
fn run_payroll_refuses_zero_gross() {
    let (mut l, a) = fixture();
    let zero = pline("Zed", "0", "0", "0", "0", "0", "0", "0", "0");
    assert_payroll_refused(&mut l, date(2026, 2, 27), vec![zero], a.cash, |_| true);
}

/// Oracle: validate_payroll/run_payroll take a cash account; an unknown one
/// cannot be credited (B-1 / R-1 "no unknown accounts").
///
/// ASSUMED: the refusal comes before posting; variant unspecified.
#[test]
fn run_payroll_refuses_unknown_cash_account() {
    let (mut l, _) = fixture();
    assert_payroll_refused(&mut l, date(2026, 2, 27), vec![alice()], 9_999, |_| true);
}

/// Oracle: ControlAccount doc "a command refuses one of the wrong type
/// (ControlAccountMismatch)": 6100 already exists as an Asset.
#[test]
fn run_payroll_refuses_wrong_type_control_account() {
    let (mut l, a) = fixture();
    l.add_account("6100", "Not wages", AccountType::Asset, None, false)
        .unwrap();
    assert_payroll_refused(
        &mut l,
        date(2026, 2, 27),
        vec![alice()],
        a.cash,
        |e| matches!(e, LedgerError::ControlAccountMismatch { code, .. } if code == "6100"),
    );
}

/// Oracle: B-12 "a closed period takes no new postings"; remit doc "rejects
/// dates in closed periods".
#[test]
fn payroll_and_remit_refuse_closed_period() {
    let (mut l, a) = fixture();
    let p = l
        .create_period("2026-01", date(2026, 1, 1), date(2026, 1, 31))
        .unwrap();
    l.close_period(p).unwrap();
    assert_payroll_refused(&mut l, date(2026, 1, 20), vec![alice()], a.cash, |e| {
        matches!(e, LedgerError::ClosedPeriod(..))
    });
    l.run_payroll(date(2026, 2, 27), vec![alice()], a.cash)
        .unwrap();
    let before = snapshot(&l);
    assert!(matches!(
        l.remit_payroll_taxes(date(2026, 1, 20), a.cash),
        Err(LedgerError::ClosedPeriod(..))
    ));
    assert_eq!(snapshot(&l), before);
}

// ------------------------------------------------------------- properties

fn arb_purchase() -> impl Strategy<Value = NaiveDate> {
    (2020i32..=2030, 1u32..=12, 1u32..=31).prop_map(|(y, m, day)| {
        let first = NaiveDate::from_ymd_opt(y, m, 1).unwrap();
        let last = month_end(first).day();
        NaiveDate::from_ymd_opt(y, m, day.min(last)).unwrap()
    })
}

fn arb_asset() -> impl Strategy<Value = Asset> {
    (1i64..=10_000_000, 1u32..=120, arb_purchase())
        .prop_flat_map(|(cost, life, p)| (Just(cost), 0..cost, Just(life), Just(p)))
        .prop_map(|(cost, salvage, life, p)| Asset {
            cost: Money::new(Decimal::new(cost, 2), Currency::USD),
            salvage_value: Money::new(Decimal::new(salvage, 2), Currency::USD),
            ..asset("1.00", "0.00", life, p)
        })
}

fn next_period(p: &str) -> String {
    let y: i32 = p[0..4].parse().unwrap();
    let m: u32 = p[5..7].parse().unwrap();
    if m == 12 {
        format!("{:04}-01", y + 1)
    } else {
        format!("{:04}-{:02}", y, m + 1)
    }
}

fn period_end(p: &str) -> NaiveDate {
    let y: i32 = p[0..4].parse().unwrap();
    let m: u32 = p[5..7].parse().unwrap();
    month_end(NaiveDate::from_ymd_opt(y, m, 1).unwrap())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Oracle: B-15 laws — a full schedule sums to exactly cost − salvage;
    /// amounts are non-negative whole cents; accumulated is the running sum
    /// and non-decreasing; book value = cost − accumulated, never rises, never
    /// below salvage, ends at salvage; months are consecutive from the
    /// purchase month.
    ///
    /// ASSUMED: a full schedule has `life` or `life + 1` months (life + 1
    /// when the first month is partial).
    #[test]
    fn prop_full_schedule_laws(a in arb_asset()) {
        let cost = a.cost.amount;
        let salvage = a.salvage_value.amount;
        let s = depreciation_schedule(&a, date(2050, 12, 31));
        prop_assert!(!s.is_empty());
        let total: Decimal = s.iter().map(|l| l.amount).sum();
        prop_assert_eq!(total, cost - salvage);
        let life = a.useful_life_months as usize;
        prop_assert!(s.len() == life || s.len() == life + 1, "len {} life {}", s.len(), life);
        prop_assert_eq!(&s[0].period, &format!("{:04}-{:02}", a.purchase_date.year(), a.purchase_date.month()));
        let mut acc = Decimal::ZERO;
        let mut prev_bv = cost;
        for (i, l) in s.iter().enumerate() {
            if i > 0 {
                prop_assert_eq!(&l.period, &next_period(&s[i - 1].period));
            }
            prop_assert!(l.amount >= Decimal::ZERO);
            prop_assert_eq!(l.amount, l.amount.round_dp(2));
            acc += l.amount;
            prop_assert_eq!(l.accumulated, acc);
            prop_assert_eq!(l.book_value, cost - l.accumulated);
            prop_assert!(l.book_value <= prev_bv);
            prop_assert!(l.book_value >= salvage);
            prev_bv = l.book_value;
        }
        prop_assert_eq!(s.last().unwrap().book_value, salvage);
    }

    /// Oracle: accumulated_depreciation / book_value docs agree with the
    /// schedule at each month end, a shorter schedule is a prefix of the full
    /// one, and nothing is depreciated before purchase.
    #[test]
    fn prop_point_queries_agree_with_schedule(a in arb_asset()) {
        let full = depreciation_schedule(&a, date(2050, 12, 31));
        for (i, l) in full.iter().enumerate() {
            let end = period_end(&l.period);
            prop_assert_eq!(accumulated_depreciation(&a, end), l.accumulated);
            prop_assert_eq!(book_value(&a, end), l.book_value);
            let prefix = depreciation_schedule(&a, end);
            prop_assert_eq!(&prefix[..], &full[..=i]);
        }
        let before = a.purchase_date.pred_opt().unwrap();
        prop_assert_eq!(accumulated_depreciation(&a, before), Decimal::ZERO);
        prop_assert_eq!(book_value(&a, before), a.cost.amount);
    }

    /// Oracle: B-15 partial months by days + interface "the last month absorbs
    /// the drift". When (cost − salvage) / life is a whole number of cents M
    /// and the purchase is after the 1st, the middle months are each exactly
    /// M and the partial first and last months together are exactly M.
    #[test]
    fn prop_partial_first_and_last_sum_to_one_month(
        m in 1i64..=100_000,
        life in 2u32..=60,
        salvage in 0i64..=100_000,
        p in arb_purchase().prop_filter("after the 1st", |p| p.day() > 1),
    ) {
        let monthly = Decimal::new(m, 2);
        let mut a = asset("1.00", "0.00", life, p);
        a.salvage_value = Money::new(Decimal::new(salvage, 2), Currency::USD);
        a.cost = Money::new(Decimal::new(salvage + m * i64::from(life), 2), Currency::USD);
        let s = depreciation_schedule(&a, date(2050, 12, 31));
        prop_assert_eq!(s.len(), life as usize + 1);
        for l in &s[1..s.len() - 1] {
            prop_assert_eq!(l.amount, monthly);
        }
        prop_assert_eq!(s[0].amount + s[s.len() - 1].amount, monthly);
    }

    /// Oracle: dispose_asset INVARIANT 1 / DisposalAmounts docs: gain_loss =
    /// proceeds − book_value, book_value = cost − accumulated, both matching
    /// the point queries for the disposal date.
    #[test]
    fn prop_disposal_amounts_laws(
        a in arb_asset(),
        offset in 0u32..=200,
        proceeds in 0i64..=20_000_000,
    ) {
        let when = month_end(a.purchase_date.checked_add_months(Months::new(offset)).unwrap());
        let pr = Decimal::new(proceeds, 2);
        let got = disposal_amounts(&a, when, Money::new(pr, Currency::USD)).unwrap();
        prop_assert_eq!(got.accumulated, accumulated_depreciation(&a, when));
        prop_assert_eq!(got.book_value, a.cost.amount - got.accumulated);
        prop_assert_eq!(got.book_value, book_value(&a, when));
        prop_assert_eq!(got.gain_loss, pr - got.book_value);
        prop_assert!(got.book_value >= a.salvage_value.amount);
    }
}

fn arb_line() -> impl Strategy<Value = PayrollLine> {
    (1i64..=1_000_000)
        .prop_flat_map(|gross| {
            let cap = gross / 5;
            (
                Just(gross),
                prop::collection::vec((0i64..=cap, any::<bool>()), 5),
                prop::collection::vec((0i64..=gross, any::<bool>()), 2),
            )
        })
        .prop_map(|(gross, w, e)| {
            let z = |(v, keep): (i64, bool)| Decimal::new(if keep { v } else { 0 }, 2);
            PayrollLine {
                employee_name: "E".to_string(),
                employee: None,
                gross_pay: Decimal::new(gross, 2),
                federal_withholding: z(w[0]),
                state_withholding: z(w[1]),
                ss_employee: z(w[2]),
                medicare_employee: z(w[3]),
                other_deductions: z(w[4]),
                ss_employer: z(e[0]),
                medicare_employer: z(e[1]),
            }
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Oracle: payroll_entry_plan doc — always balances, no zero lines, base
    /// currency, Dr 6100 = Σ gross, Dr 6110 = Σ employer FICA, each payable =
    /// its withholdings (both FICA shares), Cr cash = Σ net pay, and net pay
    /// = gross − employee withholdings.
    #[test]
    fn prop_entry_plan_laws(lines in prop::collection::vec(arb_line(), 1..5)) {
        prop_assert_eq!(validate_payroll_lines(&lines), Ok(()));
        let plan = payroll_entry_plan(&lines, PA, PLAN_CASH, Currency::USD).unwrap();
        let (dr, cr) = plan_totals(&plan);
        prop_assert_eq!(dr, cr);
        prop_assert!(plan.iter().all(|(_, _, m)| m.amount > Decimal::ZERO && m.currency == Currency::USD));
        let sum = |f: fn(&PayrollLine) -> Decimal| -> Decimal { lines.iter().map(f).sum() };
        for l in &lines {
            prop_assert_eq!(
                l.net_pay().unwrap(),
                l.gross_pay - l.federal_withholding - l.state_withholding
                    - l.ss_employee - l.medicare_employee - l.other_deductions
            );
        }
        prop_assert_eq!(plan_sum(&plan, PA.salary_expense, Side::Debit), sum(|l| l.gross_pay));
        prop_assert_eq!(plan_sum(&plan, PA.payroll_tax_expense, Side::Debit), sum(|l| l.ss_employer + l.medicare_employer));
        prop_assert_eq!(plan_sum(&plan, PA.federal_payable, Side::Credit), sum(|l| l.federal_withholding));
        prop_assert_eq!(plan_sum(&plan, PA.state_payable, Side::Credit), sum(|l| l.state_withholding));
        prop_assert_eq!(plan_sum(&plan, PA.ss_payable, Side::Credit), sum(|l| l.ss_employee + l.ss_employer));
        prop_assert_eq!(plan_sum(&plan, PA.medicare_payable, Side::Credit), sum(|l| l.medicare_employee + l.medicare_employer));
        prop_assert_eq!(plan_sum(&plan, PA.other_payable, Side::Credit), sum(|l| l.other_deductions));
        prop_assert_eq!(plan_sum(&plan, PLAN_CASH, Side::Credit), sum(|l| l.net_pay().unwrap()));
    }

    /// Oracle: payroll_remit_plan doc "balances by construction": Σ Dr = Cr
    /// cash, one debit per non-zero payable, no zero lines.
    #[test]
    fn prop_remit_plan_balances(bals in prop::collection::vec(0i64..=1_000_000, 1..6)) {
        let payables: Vec<(AccountId, Decimal)> = bals
            .iter()
            .enumerate()
            .map(|(i, c)| (i as AccountId + 1, Decimal::new(*c, 2)))
            .collect();
        let plan = payroll_remit_plan(&payables, PLAN_CASH, Currency::USD);
        let total: Decimal = payables.iter().map(|(_, b)| *b).sum();
        let (dr, cr) = plan_totals(&plan);
        prop_assert_eq!(dr, cr);
        prop_assert_eq!(dr, total);
        prop_assert!(plan.iter().all(|(_, _, m)| !m.amount.is_zero()));
        for (acct, bal) in &payables {
            prop_assert_eq!(plan_sum(&plan, *acct, Side::Debit), *bal);
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// Oracle: run_payroll INVARIANT 1 + remit INVARIANT 1: after a run and a
    /// remittance, every 2110–2150 payable is zero and cash has fallen by
    /// exactly gross + employer FICA (= 6100 + 6110), the run's whole cost.
    #[test]
    fn prop_run_then_remit_zeroes_payables(lines in prop::collection::vec(arb_line(), 1..4)) {
        let (mut l, a) = fixture();
        prop_assert_eq!(l.validate_payroll(date(2026, 2, 27), &lines, a.cash), Ok(()));
        l.run_payroll(date(2026, 2, 27), lines.clone(), a.cash).unwrap();
        let cost: Decimal = lines.iter().map(|x| x.gross_pay + x.ss_employer + x.medicare_employer).sum();
        let withheld: Decimal = lines
            .iter()
            .map(|x| x.gross_pay - x.net_pay().unwrap() + x.ss_employer + x.medicare_employer)
            .sum();
        let owed: Decimal = l.payroll_tax_payables().iter().map(|(_, b)| *b).sum();
        prop_assert_eq!(owed, withheld);
        if !withheld.is_zero() {
            l.remit_payroll_taxes(date(2026, 3, 15), a.cash).unwrap();
        }
        prop_assert!(l.payroll_tax_payables().is_empty());
        for code in ["2110", "2120", "2130", "2140", "2150"] {
            if let Some(acct) = l.chart().find_by_code(code) {
                prop_assert_eq!(l.account_balance(acct.id).unwrap(), Decimal::ZERO);
            }
        }
        prop_assert_eq!(balance_of(&l, "1000"), d("10000.00") - cost);
    }
}
