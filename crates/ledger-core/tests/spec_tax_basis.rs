//! Spec-derived tests for an asset's federal tax basis (rules.md B-25):
//! `TaxBasisRule`, `Ledger::set_asset_tax_basis` / `check_asset_tax_basis`,
//! `Asset::tax_basis`, `Asset::built_in_gain`, and the tax fields of
//! `reports::fixed_asset_register`.
//!
//! Every expected value comes from the written specification: the doc
//! comments and INVARIANTS/SCENARIOS blocks on those items, and the
//! vendored sources (`docs/spec/sources/irs-partnership-basis.md`): cost
//! basis (IRC §1012(a)), carryover of the contributor's adjusted basis
//! (IRC §723), the lesser of fair market value and adjusted basis on a
//! conversion from personal use (Treas. Reg. §1.168(i)-4(b)(1)), and
//! built-in gain or loss as fair market value less the contributor's basis
//! (IRC §704(c)). Amounts are worked by hand.

use std::str::FromStr;

use chrono::NaiveDate;
use ledger_core::{
    AccountId, AccountType, AssetId, Currency, Ledger, LedgerError, Money, NewAsset, Side,
    TaxBasis, TaxBasisRule, fixed_asset_register,
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

struct Accts {
    equipment: AccountId,
    accum: AccountId,
    depr_expense: AccountId,
}

/// A USD ledger with a fixed-asset chart, funded with $50,000 cash.
fn fixture() -> (Ledger, Accts) {
    let mut l = Ledger::new(Currency::USD);
    let mut acct = |code: &str, name: &str, ty: AccountType| {
        l.add_account(code, name, ty, None, false).unwrap()
    };
    let cash = acct("1000", "Cash", AccountType::Asset);
    let equipment = acct("1500", "Equipment", AccountType::Asset);
    let accum = acct("1510", "Accumulated Depreciation", AccountType::Asset);
    let capital = acct("3000", "Owner Capital", AccountType::Equity);
    let depr_expense = acct("6200", "Depreciation Expense", AccountType::Expense);
    let e = l.try_create_entry(date(2026, 1, 1), "funding").unwrap();
    l.add_line(e, cash, Side::Debit, usd("50000.00")).unwrap();
    l.add_line(e, capital, Side::Credit, usd("50000.00"))
        .unwrap();
    l.post_entry(e).unwrap();
    (
        l,
        Accts {
            equipment,
            accum,
            depr_expense,
        },
    )
}

fn register(l: &mut Ledger, a: &Accts, description: &str, cost: &str) -> AssetId {
    l.register_asset(NewAsset {
        description: description.to_string(),
        purchase_date: date(2026, 1, 1),
        cost: usd(cost),
        salvage_value: usd("0.00"),
        useful_life_months: 36,
        cost_account: a.equipment,
        accum_depr_account: a.accum,
        expense_account: a.depr_expense,
    })
    .unwrap()
}

fn tb(rule: TaxBasisRule, basis: Option<&str>) -> TaxBasis {
    TaxBasis {
        rule,
        contributor_basis: basis.map(d),
    }
}

/// A ledger with one asset of book cost (fair market value) `cost`.
fn one(cost: &str) -> (Ledger, AssetId) {
    let (mut l, a) = fixture();
    let id = register(&mut l, &a, "Laptop", cost);
    (l, id)
}

fn set(l: &mut Ledger, id: AssetId, rule: TaxBasisRule, basis: Option<&str>) {
    l.set_asset_tax_basis(id, tb(rule, basis)).unwrap();
}

const RULES: [TaxBasisRule; 3] = [
    TaxBasisRule::Cost,
    TaxBasisRule::Carryover,
    TaxBasisRule::PersonalUseConversion,
];

// ---------------------------------------------------------------- TaxBasisRule

/// TaxBasisRule::key: the wire names `cost`, `carryover`,
/// `personal_use_conversion`.
#[test]
fn rule_keys_are_the_documented_wire_names() {
    assert_eq!(TaxBasisRule::Cost.key(), "cost");
    assert_eq!(TaxBasisRule::Carryover.key(), "carryover");
    assert_eq!(
        TaxBasisRule::PersonalUseConversion.key(),
        "personal_use_conversion"
    );
}

/// TaxBasisRule::parse: the rule named `key`; round-trips with key().
#[test]
fn rule_parse_round_trips_with_key() {
    for r in RULES {
        assert_eq!(TaxBasisRule::parse(r.key()), Some(r));
    }
}

/// Anything other than a documented wire name is no rule.
#[test]
fn rule_parse_refuses_other_names() {
    for bad in [
        "",
        "Cost",
        "COST",
        " cost",
        "cost ",
        "carry_over",
        "personal-use-conversion",
        "PersonalUseConversion",
        "fmv",
    ] {
        assert_eq!(TaxBasisRule::parse(bad), None, "{bad:?}");
    }
}

/// The serde names are snake_case, the same as the wire names.
#[test]
fn rule_serde_names_are_the_wire_names() {
    for r in RULES {
        let j = serde_json::to_value(r).unwrap();
        assert_eq!(j, serde_json::Value::String(r.key().to_string()));
        let back: TaxBasisRule = serde_json::from_value(j).unwrap();
        assert_eq!(back, r);
    }
}

// ---------------------------------------------------------------- set / check

/// SCENARIO 1: an unknown asset is refused with UnknownAsset.
#[test]
fn an_unknown_asset_is_refused() {
    let (mut l, id) = one("1000.00");
    let before = l.clone();
    let unknown = id + 99;
    for t in [
        tb(TaxBasisRule::Cost, None),
        tb(TaxBasisRule::Carryover, Some("500.00")),
    ] {
        assert!(matches!(
            l.check_asset_tax_basis(unknown, t),
            Err(LedgerError::UnknownAsset(x)) if x == unknown
        ));
        assert!(matches!(
            l.set_asset_tax_basis(unknown, t),
            Err(LedgerError::UnknownAsset(x)) if x == unknown
        ));
    }
    assert_eq!(l, before, "INVARIANT 3");
}

/// INVARIANT 1: Cost takes no contributor basis.
#[test]
fn cost_with_a_contributor_basis_is_refused() {
    let (mut l, id) = one("1000.00");
    let before = l.clone();
    for b in ["0.00", "500.00", "1000.00"] {
        let t = tb(TaxBasisRule::Cost, Some(b));
        assert!(matches!(
            l.check_asset_tax_basis(id, t),
            Err(LedgerError::InvalidTaxBasis(_))
        ));
        assert!(matches!(
            l.set_asset_tax_basis(id, t),
            Err(LedgerError::InvalidTaxBasis(_))
        ));
    }
    assert_eq!(l, before, "INVARIANT 3");
    assert_eq!(l.get_asset(id).unwrap().tax, None);
}

/// INVARIANT 1: Carryover and PersonalUseConversion require one.
#[test]
fn a_contribution_without_a_contributor_basis_is_refused() {
    let (mut l, id) = one("1000.00");
    let before = l.clone();
    for r in [TaxBasisRule::Carryover, TaxBasisRule::PersonalUseConversion] {
        let t = tb(r, None);
        assert!(matches!(
            l.check_asset_tax_basis(id, t),
            Err(LedgerError::InvalidTaxBasis(_))
        ));
        assert!(matches!(
            l.set_asset_tax_basis(id, t),
            Err(LedgerError::InvalidTaxBasis(_))
        ));
    }
    assert_eq!(l, before, "INVARIANT 3");
}

/// INVARIANT 2: a contributor basis is non-negative.
#[test]
fn a_negative_contributor_basis_is_refused() {
    let (mut l, id) = one("1000.00");
    let before = l.clone();
    for r in [TaxBasisRule::Carryover, TaxBasisRule::PersonalUseConversion] {
        for b in ["-0.01", "-1", "-500.00"] {
            let t = tb(r, Some(b));
            assert!(
                matches!(
                    l.set_asset_tax_basis(id, t),
                    Err(LedgerError::InvalidTaxBasis(_))
                ),
                "{r:?} {b}"
            );
            assert!(l.check_asset_tax_basis(id, t).is_err());
        }
    }
    assert_eq!(l, before, "INVARIANT 3");
}

/// INVARIANT 2: in cents at most.
#[test]
fn more_than_two_decimal_places_is_refused() {
    let (mut l, id) = one("1000.00");
    let before = l.clone();
    for r in [TaxBasisRule::Carryover, TaxBasisRule::PersonalUseConversion] {
        for b in ["500.001", "0.005", "1.2345"] {
            let t = tb(r, Some(b));
            assert!(
                matches!(
                    l.set_asset_tax_basis(id, t),
                    Err(LedgerError::InvalidTaxBasis(_))
                ),
                "{r:?} {b}"
            );
            assert!(l.check_asset_tax_basis(id, t).is_err());
        }
    }
    assert_eq!(l, before, "INVARIANT 3");
}

/// Valid records: zero is non-negative; whole dollars and one decimal
/// place are "in cents at most". check agrees with set, and a valid set
/// changes nothing but the asset's tax record (book figures don't change).
#[test]
fn valid_records_are_accepted_and_book_figures_stay() {
    for (r, b) in [
        (TaxBasisRule::Cost, None),
        (TaxBasisRule::Carryover, Some("0")),
        (TaxBasisRule::Carryover, Some("0.00")),
        (TaxBasisRule::Carryover, Some("750")),
        (TaxBasisRule::PersonalUseConversion, Some("750.5")),
        (TaxBasisRule::PersonalUseConversion, Some("750.50")),
        (TaxBasisRule::PersonalUseConversion, Some("2500.00")),
    ] {
        let (mut l, id) = one("1000.00");
        let t = tb(r, b);
        l.check_asset_tax_basis(id, t).unwrap();
        let before = l.get_asset(id).unwrap().clone();
        l.set_asset_tax_basis(id, t).unwrap();
        let after = l.get_asset(id).unwrap();
        assert_eq!(after.tax, Some(t), "{r:?} {b:?}");
        // Book figures don't change.
        let mut same = after.clone();
        same.tax = before.tax;
        assert_eq!(same, before, "{r:?} {b:?}");
    }
}

/// SCENARIO 2: set again, the new record replaces the old.
#[test]
fn setting_again_replaces() {
    let (mut l, id) = one("1000.00");
    set(&mut l, id, TaxBasisRule::Cost, None);
    assert_eq!(l.get_asset(id).unwrap().tax_basis(), Some(d("1000.00")));
    set(&mut l, id, TaxBasisRule::Carryover, Some("400.00"));
    let a = l.get_asset(id).unwrap();
    assert_eq!(a.tax, Some(tb(TaxBasisRule::Carryover, Some("400.00"))));
    assert_eq!(a.tax_basis(), Some(d("400.00")));
    set(
        &mut l,
        id,
        TaxBasisRule::PersonalUseConversion,
        Some("1200.00"),
    );
    let a = l.get_asset(id).unwrap();
    assert_eq!(
        a.tax,
        Some(tb(TaxBasisRule::PersonalUseConversion, Some("1200.00")))
    );
    assert_eq!(a.tax_basis(), Some(d("1000.00")));
    set(&mut l, id, TaxBasisRule::Cost, None);
    let a = l.get_asset(id).unwrap();
    assert_eq!(a.tax, Some(tb(TaxBasisRule::Cost, None)));
    assert_eq!(a.built_in_gain(), None);
}

/// INVARIANT 3: a refusal after a valid record leaves that record.
#[test]
fn a_refusal_keeps_the_earlier_record() {
    let (mut l, id) = one("1000.00");
    set(&mut l, id, TaxBasisRule::Carryover, Some("300.00"));
    let before = l.clone();
    assert!(
        l.set_asset_tax_basis(id, tb(TaxBasisRule::Cost, Some("1.00")))
            .is_err()
    );
    assert!(
        l.set_asset_tax_basis(id, tb(TaxBasisRule::Carryover, None))
            .is_err()
    );
    assert!(
        l.set_asset_tax_basis(id, tb(TaxBasisRule::Carryover, Some("-1.00")))
            .is_err()
    );
    assert!(
        l.set_asset_tax_basis(id, tb(TaxBasisRule::Carryover, Some("1.001")))
            .is_err()
    );
    assert_eq!(l, before);
    assert_eq!(l.get_asset(id).unwrap().tax_basis(), Some(d("300.00")));
}

/// Setting one asset's record leaves the others alone.
#[test]
fn a_record_belongs_to_one_asset() {
    let (mut l, a) = fixture();
    let x = register(&mut l, &a, "Laptop", "1000.00");
    let y = register(&mut l, &a, "Desk", "400.00");
    set(&mut l, x, TaxBasisRule::Carryover, Some("800.00"));
    assert_eq!(l.get_asset(y).unwrap().tax, None);
    assert_eq!(l.get_asset(y).unwrap().tax_basis(), None);
}

// ---------------------------------------------------------------- tax_basis

/// Not recorded: None (and no built-in gain).
#[test]
fn no_record_means_no_tax_basis() {
    let (l, id) = one("1000.00");
    let a = l.get_asset(id).unwrap();
    assert_eq!(a.tax, None);
    assert_eq!(a.tax_basis(), None);
    assert_eq!(a.built_in_gain(), None);
}

/// INVARIANT 1: Cost: the book cost (IRC §1012(a)).
#[test]
fn cost_basis_is_the_book_cost() {
    let (mut l, id) = one("1234.56");
    set(&mut l, id, TaxBasisRule::Cost, None);
    let a = l.get_asset(id).unwrap();
    assert_eq!(a.tax_basis(), Some(d("1234.56")));
    // A purchase has no built-in gain.
    assert_eq!(a.built_in_gain(), None);
}

/// INVARIANT 2: Carryover: the contributor's adjusted basis (IRC §723),
/// whether above or below fair market value.
#[test]
fn carryover_basis_is_the_contributors_basis() {
    for (fmv, basis, gain) in [
        ("1000.00", "600.00", "400.00"),   // built-in gain
        ("1000.00", "1500.00", "-500.00"), // built-in loss
        ("1000.00", "1000.00", "0.00"),    // boundary
        ("1000.00", "0.00", "1000.00"),    // fully depreciated
    ] {
        let (mut l, id) = one(fmv);
        set(&mut l, id, TaxBasisRule::Carryover, Some(basis));
        let a = l.get_asset(id).unwrap();
        assert_eq!(a.tax_basis(), Some(d(basis)), "{fmv} {basis}");
        assert_eq!(a.built_in_gain(), Some(d(gain)), "{fmv} {basis}");
    }
}

/// INVARIANT 3: PersonalUseConversion: min(book cost, contributor's basis)
/// (Treas. Reg. §1.168(i)-4(b)(1): "the lesser of its fair market value or
/// its adjusted depreciable basis").
#[test]
fn personal_use_conversion_is_the_lesser_of_value_and_basis() {
    for (fmv, basis, tax, gain) in [
        // Paid 2,000, worth 1,200 now: basis 1,200; built-in loss −800.
        ("1200.00", "2000.00", "1200.00", "-800.00"),
        // Paid 300, worth 450: basis 300; built-in gain 150.
        ("450.00", "300.00", "300.00", "150.00"),
        // Boundary: value equals basis.
        ("999.99", "999.99", "999.99", "0.00"),
        // A cent either side of the boundary.
        ("1000.00", "1000.01", "1000.00", "-0.01"),
        ("1000.00", "999.99", "999.99", "0.01"),
    ] {
        let (mut l, id) = one(fmv);
        set(&mut l, id, TaxBasisRule::PersonalUseConversion, Some(basis));
        let a = l.get_asset(id).unwrap();
        assert_eq!(a.tax_basis(), Some(d(tax)), "{fmv} {basis}");
        assert_eq!(a.built_in_gain(), Some(d(gain)), "{fmv} {basis}");
    }
}

/// built_in_gain: FMV (book cost) less the contributor's basis; positive
/// for a gain, negative for a loss, zero at FMV == basis, regardless of
/// which contribution rule (the definition does not depend on the rule).
#[test]
fn built_in_gain_sign_and_boundary() {
    for r in [TaxBasisRule::Carryover, TaxBasisRule::PersonalUseConversion] {
        let (mut l, id) = one("800.00");
        set(&mut l, id, r, Some("500.00"));
        let g = l.get_asset(id).unwrap().built_in_gain().unwrap();
        assert_eq!(g, d("300.00"));
        assert!(g > Decimal::ZERO, "{r:?}: FMV > basis is a gain");

        set(&mut l, id, r, Some("1100.00"));
        let g = l.get_asset(id).unwrap().built_in_gain().unwrap();
        assert_eq!(g, d("-300.00"));
        assert!(g < Decimal::ZERO, "{r:?}: basis > FMV is a loss");

        set(&mut l, id, r, Some("800.00"));
        let g = l.get_asset(id).unwrap().built_in_gain().unwrap();
        assert!(g.is_zero(), "{r:?}: FMV == basis is neither");
    }
}

// ---------------------------------------------------------------- register

struct RegisterBooks {
    l: Ledger,
    bought: AssetId,
    carried: AssetId,
    converted: AssetId,
    unrecorded: AssetId,
    sold: AssetId,
}

/// Five assets: a purchase at cost, a carryover contribution, a
/// personal-use conversion, one with no tax basis, and one disposed (with
/// a tax basis, which the totals must leave out).
fn register_books() -> RegisterBooks {
    let (mut l, a) = fixture();
    let bought = register(&mut l, &a, "Server", "3000.00");
    let carried = register(&mut l, &a, "Printer", "500.00");
    let converted = register(&mut l, &a, "Laptop", "1200.00");
    let unrecorded = register(&mut l, &a, "Desk", "700.00");
    let sold = register(&mut l, &a, "Old monitor", "360.00");
    set(&mut l, bought, TaxBasisRule::Cost, None);
    set(&mut l, carried, TaxBasisRule::Carryover, Some("650.25"));
    set(
        &mut l,
        converted,
        TaxBasisRule::PersonalUseConversion,
        Some("2000.00"),
    );
    set(&mut l, sold, TaxBasisRule::Cost, None);
    l.dispose_asset(sold, date(2026, 3, 31), usd("100.00"))
        .unwrap();
    RegisterBooks {
        l,
        bought,
        carried,
        converted,
        unrecorded,
        sold,
    }
}

/// INVARIANT 5: each row's tax fields are the asset's.
#[test]
fn register_rows_carry_the_assets_tax_fields() {
    let b = register_books();
    let r = fixed_asset_register(&b.l, date(2026, 6, 30)).unwrap();
    assert_eq!(r.rows.len(), 5);
    for row in &r.rows {
        let a = b.l.get_asset(row.asset_id).unwrap();
        assert_eq!(row.tax_rule, a.tax.map(|t| t.rule));
        assert_eq!(
            row.contributor_basis,
            a.tax.and_then(|t| t.contributor_basis)
        );
        assert_eq!(row.tax_basis, a.tax_basis());
        assert_eq!(row.built_in_gain, a.built_in_gain());
    }
    let row = |id| r.rows.iter().find(|x| x.asset_id == id).unwrap();
    // By hand.
    let x = row(b.bought);
    assert_eq!(x.tax_rule, Some(TaxBasisRule::Cost));
    assert_eq!(x.contributor_basis, None);
    assert_eq!(x.tax_basis, Some(d("3000.00")));
    assert_eq!(x.built_in_gain, None);
    let x = row(b.carried);
    assert_eq!(x.tax_rule, Some(TaxBasisRule::Carryover));
    assert_eq!(x.contributor_basis, Some(d("650.25")));
    assert_eq!(x.tax_basis, Some(d("650.25")));
    assert_eq!(x.built_in_gain, Some(d("-150.25")));
    let x = row(b.converted);
    assert_eq!(x.tax_rule, Some(TaxBasisRule::PersonalUseConversion));
    assert_eq!(x.contributor_basis, Some(d("2000.00")));
    assert_eq!(x.tax_basis, Some(d("1200.00")));
    assert_eq!(x.built_in_gain, Some(d("-800.00")));
    let x = row(b.unrecorded);
    assert_eq!(x.tax_rule, None);
    assert_eq!(x.contributor_basis, None);
    assert_eq!(x.tax_basis, None);
    assert_eq!(x.built_in_gain, None);
    // A disposed asset's row still shows its record.
    let x = row(b.sold);
    assert!(x.disposed.is_some());
    assert_eq!(x.tax_rule, Some(TaxBasisRule::Cost));
    assert_eq!(x.tax_basis, Some(d("360.00")));
}

/// INVARIANT 3: total_tax_basis sums held assets with a basis, exactly;
/// missing_tax_basis counts held ones without; disposed excluded.
#[test]
fn register_tax_totals_cover_held_assets_only() {
    let b = register_books();
    let r = fixed_asset_register(&b.l, date(2026, 6, 30)).unwrap();
    // 3000.00 + 650.25 + 1200.00 (the disposed 360.00 left out).
    assert_eq!(r.total_tax_basis, d("4850.25"));
    assert_eq!(r.missing_tax_basis, 1);
    // Before the disposal date the old monitor is still held.
    let r = fixed_asset_register(&b.l, date(2026, 2, 28)).unwrap();
    assert_eq!(r.total_tax_basis, d("5210.25"));
    assert_eq!(r.missing_tax_basis, 1);
}

/// A disposed asset without a tax basis isn't counted as missing.
#[test]
fn a_disposed_asset_without_a_basis_is_not_missing() {
    let (mut l, a) = fixture();
    let x = register(&mut l, &a, "Laptop", "1000.00");
    let y = register(&mut l, &a, "Desk", "400.00");
    l.dispose_asset(y, date(2026, 3, 31), usd("0.00")).unwrap();
    let r = fixed_asset_register(&l, date(2026, 6, 30)).unwrap();
    assert_eq!(r.missing_tax_basis, 1);
    assert_eq!(r.total_tax_basis, Decimal::ZERO);
    set(&mut l, x, TaxBasisRule::Carryover, Some("900.00"));
    let r = fixed_asset_register(&l, date(2026, 6, 30)).unwrap();
    assert_eq!(r.missing_tax_basis, 0);
    assert_eq!(r.total_tax_basis, d("900.00"));
}

/// No assets: zero total, none missing.
#[test]
fn an_empty_register_has_no_tax_basis() {
    let (l, _) = fixture();
    let r = fixed_asset_register(&l, date(2026, 6, 30)).unwrap();
    assert!(r.rows.is_empty());
    assert_eq!(r.total_tax_basis, Decimal::ZERO);
    assert_eq!(r.missing_tax_basis, 0);
}

/// Recording tax basis leaves the book totals alone.
#[test]
fn tax_basis_does_not_move_book_totals() {
    let (mut l, a) = fixture();
    let x = register(&mut l, &a, "Laptop", "1200.00");
    let before = fixed_asset_register(&l, date(2026, 6, 30)).unwrap();
    set(
        &mut l,
        x,
        TaxBasisRule::PersonalUseConversion,
        Some("2000.00"),
    );
    let after = fixed_asset_register(&l, date(2026, 6, 30)).unwrap();
    assert_eq!(after.total_cost, before.total_cost);
    assert_eq!(after.total_accumulated, before.total_accumulated);
    assert_eq!(after.total_book_value, before.total_book_value);
    assert_eq!(after.ledger_cost, before.ledger_cost);
    assert_eq!(after.ledger_accumulated, before.ledger_accumulated);
}

// ---------------------------------------------------------------- proptest

fn cents() -> impl Strategy<Value = Decimal> {
    (0i64..=100_000_000).prop_map(|c| Decimal::new(c, 2))
}

fn positive_cents() -> impl Strategy<Value = Decimal> {
    (1i64..=100_000_000).prop_map(|c| Decimal::new(c, 2))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Asset::tax_basis INVARIANTS 1–3 and built_in_gain over random
    /// costs, bases, and rules; register totals agree.
    #[test]
    fn prop_tax_basis_follows_its_rule(
        assets in proptest::collection::vec((positive_cents(), cents(), 0usize..4), 1..8)
    ) {
        let (mut l, a) = fixture();
        let mut expected_total = Decimal::ZERO;
        let mut expected_missing = 0usize;
        for (i, (cost, basis, rule)) in assets.iter().enumerate() {
            let id = register(&mut l, &a, &format!("Asset {i}"), &cost.to_string());
            let want = match rule {
                0 => {
                    set(&mut l, id, TaxBasisRule::Cost, None);
                    Some((*cost, None))
                }
                1 => {
                    l.set_asset_tax_basis(id, TaxBasis {
                        rule: TaxBasisRule::Carryover,
                        contributor_basis: Some(*basis),
                    }).unwrap();
                    Some((*basis, Some(*cost - *basis)))
                }
                2 => {
                    l.set_asset_tax_basis(id, TaxBasis {
                        rule: TaxBasisRule::PersonalUseConversion,
                        contributor_basis: Some(*basis),
                    }).unwrap();
                    Some((if cost < basis { *cost } else { *basis }, Some(*cost - *basis)))
                }
                _ => None,
            };
            let asset = l.get_asset(id).unwrap();
            match want {
                Some((tax, gain)) => {
                    prop_assert_eq!(asset.tax_basis(), Some(tax));
                    prop_assert_eq!(asset.built_in_gain(), gain);
                    expected_total += tax;
                }
                None => {
                    prop_assert_eq!(asset.tax_basis(), None);
                    expected_missing += 1;
                }
            }
            if let Some(t) = asset.tax_basis() {
                // Personal-use conversion never exceeds fair market value.
                if *rule == 2 {
                    prop_assert!(t <= *cost);
                    prop_assert!(t <= *basis);
                }
            }
        }
        let r = fixed_asset_register(&l, date(2026, 6, 30)).unwrap();
        prop_assert_eq!(r.total_tax_basis, expected_total);
        prop_assert_eq!(r.missing_tax_basis, expected_missing);
    }

    /// check agrees with set, and a refused record changes nothing.
    #[test]
    fn prop_check_agrees_with_set(
        cents in -1_000_000i64..1_000_000,
        scale in 0u32..5,
        rule in 0usize..3,
        some in any::<bool>(),
    ) {
        let (mut l, id) = one("1000.00");
        let basis = some.then(|| Decimal::new(cents, scale));
        let t = TaxBasis { rule: RULES[rule], contributor_basis: basis };
        let before = l.clone();
        let checked = l.check_asset_tax_basis(id, t);
        let done = l.set_asset_tax_basis(id, t);
        prop_assert_eq!(checked.is_ok(), done.is_ok());
        // The spec, independently: INVARIANTS 1 and 2.
        let valid = match (RULES[rule], basis) {
            (TaxBasisRule::Cost, None) => true,
            (TaxBasisRule::Cost, Some(_)) => false,
            (_, None) => false,
            (_, Some(b)) => b >= Decimal::ZERO && b.normalize().scale() <= 2,
        };
        prop_assert_eq!(done.is_ok(), valid, "{:?} {:?}", RULES[rule], basis);
        if done.is_err() {
            prop_assert_eq!(&l, &before);
        } else {
            prop_assert_eq!(l.get_asset(id).unwrap().tax, Some(t));
        }
    }
}
