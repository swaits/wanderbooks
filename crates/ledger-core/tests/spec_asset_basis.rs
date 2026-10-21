//! Spec-derived tests for an asset's basis facts and tax figures
//! (rules.md B-25), its two dates and its two accumulated figures
//! (rules.md B-15, B-24): `BasisRule`, `Sec704cMethod`, `AssetBasis`
//! (`check`, `check_journaled`, `check_legacy_journaled`),
//! `Ledger::set_asset_basis` / `replay_set_asset_basis` /
//! `replay_set_asset_tax_basis`, `Asset::basis_figures`,
//! `Ledger::asset_acquired_date`, the acquisition link's dates,
//! `Ledger::accumulated_posted` and the runs, and the tax fields of
//! `reports::fixed_asset_register`.
//!
//! Every expected value comes from the written specification: the doc
//! comments and INVARIANTS/SCENARIOS blocks on those items, and the
//! vendored sources (`docs/spec/sources/irs-partnership-basis.md`): cost
//! basis (IRC §1012(a)), the §723 basis (the contributor's adjusted basis),
//! the lesser of fair market value and adjusted basis on a conversion
//! from personal use (Treas. Reg. §1.168(i)-4(b)(1); D-41), and the
//! §704(c) built-in gain or loss as fair market value less the §723 basis
//! at contribution (Treas. Reg. §1.704-3(a)(3)(ii)). Amounts are worked by
//! hand. Every name and figure is invented.

use std::str::FromStr;

use chrono::NaiveDate;
use ledger_core::{
    AccountId, AccountType, AssetBasis, AssetCorrection, AssetId, BasisRule, Currency, Ledger,
    LedgerError, Money, NewAsset, Sec704cMethod, Side, fixed_asset_register,
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
    cash: AccountId,
    equipment: AccountId,
    accum: AccountId,
    capital: AccountId,
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
    let capital = acct("3000", "Member Capital", AccountType::Equity);
    let depr_expense = acct("6200", "Depreciation Expense", AccountType::Expense);
    let e = l.try_create_entry(date(2026, 1, 1), "funding").unwrap();
    l.add_line(e, cash, Side::Debit, usd("50000.00")).unwrap();
    l.add_line(e, capital, Side::Credit, usd("50000.00"))
        .unwrap();
    l.post_entry(e).unwrap();
    (
        l,
        Accts {
            cash,
            equipment,
            accum,
            capital,
            depr_expense,
        },
    )
}

fn new_asset(a: &Accts, description: &str, cost: &str, life: u32, on: NaiveDate) -> NewAsset {
    NewAsset {
        description: description.to_string(),
        in_service_date: on,
        cost: usd(cost),
        salvage_value: usd("0.00"),
        useful_life_months: life,
        cost_account: a.equipment,
        accum_depr_account: a.accum,
        expense_account: a.depr_expense,
    }
}

fn register(l: &mut Ledger, a: &Accts, description: &str, cost: &str) -> AssetId {
    l.register_asset(new_asset(a, description, cost, 36, date(2026, 1, 1)))
        .unwrap()
}

/// A ledger with one asset of book cost (fair market value) `cost`.
fn one(cost: &str) -> (Ledger, AssetId) {
    let (mut l, a) = fixture();
    let id = register(&mut l, &a, "Laptop", cost);
    (l, id)
}

fn contribution(rule: BasisRule, adjusted: &str, fmv: &str) -> AssetBasis {
    AssetBasis::contribution(rule, d(adjusted), d(fmv))
}

/// A pre-P3 record: rule and contributor basis, no value at contribution.
fn legacy(rule: BasisRule, basis: Option<&str>) -> AssetBasis {
    AssetBasis {
        rule,
        contributor_adjusted_basis: basis.map(d),
        fmv_at_contribution: None,
        sec704c_method: None,
        source: None,
    }
}

/// Post an entry of `lines` on `on`; returns it.
fn post(l: &mut Ledger, on: NaiveDate, lines: &[(AccountId, Side, &str)]) -> ledger_core::EntryId {
    let e = l.try_create_entry(on, "entry").unwrap();
    for (acct, side, amt) in lines {
        l.add_line(e, *acct, *side, usd(amt)).unwrap();
    }
    l.post_entry(e).unwrap();
    e
}

/// One run for `asset`'s month of `on`, as a store posts and records it.
fn run(
    l: &mut Ledger,
    a: &Accts,
    asset: AssetId,
    on: NaiveDate,
    amt: &str,
) -> ledger_core::EntryId {
    let e = post(
        l,
        on,
        &[
            (a.depr_expense, Side::Debit, amt),
            (a.accum, Side::Credit, amt),
        ],
    );
    let period = on.format("%Y-%m").to_string();
    l.check_depreciation_run(asset, &period, e, d(amt)).unwrap();
    l.record_depreciation_run(asset, &period, e, d(amt))
        .unwrap();
    e
}

const RULES: [BasisRule; 3] = [
    BasisRule::Purchase,
    BasisRule::Carryover723,
    BasisRule::PersonalUseConversion,
];

// ------------------------------------------------------------ BasisRule

/// BasisRule::key: the wire names `purchase`, `carryover_723`,
/// `personal_use_conversion`; parse round-trips with key.
#[test]
fn rule_keys_are_the_documented_wire_names() {
    assert_eq!(BasisRule::Purchase.key(), "purchase");
    assert_eq!(BasisRule::Carryover723.key(), "carryover_723");
    assert_eq!(
        BasisRule::PersonalUseConversion.key(),
        "personal_use_conversion"
    );
    for r in RULES {
        assert_eq!(BasisRule::parse(r.key()), Some(r));
    }
}

/// parse takes the wire names only; parse_legacy the pre-P3 names (for
/// replay, the projection, and the tools' aliases).
#[test]
fn legacy_names_parse_only_as_legacy() {
    assert_eq!(BasisRule::parse("cost"), None);
    assert_eq!(BasisRule::parse("carryover"), None);
    assert_eq!(BasisRule::parse_legacy("cost"), Some(BasisRule::Purchase));
    assert_eq!(
        BasisRule::parse_legacy("carryover"),
        Some(BasisRule::Carryover723)
    );
    assert_eq!(
        BasisRule::parse_legacy("personal_use_conversion"),
        Some(BasisRule::PersonalUseConversion)
    );
    assert_eq!(BasisRule::parse_legacy("purchase"), None);
    assert_eq!(BasisRule::parse_legacy("carryover_723"), None);
    for bad in [
        "",
        "Purchase",
        " purchase",
        "carryover723",
        "personal-use-conversion",
        "fmv",
    ] {
        assert_eq!(BasisRule::parse(bad), None, "{bad:?}");
        assert_eq!(BasisRule::parse_legacy(bad), None, "{bad:?}");
    }
}

/// The serde names are the pre-P3 ones, frozen: the books an earlier
/// version wrote are compared with the asset's serde form.
#[test]
fn rule_serde_names_are_frozen() {
    for (r, name) in [
        (BasisRule::Purchase, "cost"),
        (BasisRule::Carryover723, "carryover"),
        (BasisRule::PersonalUseConversion, "personal_use_conversion"),
    ] {
        let j = serde_json::to_value(r).unwrap();
        assert_eq!(j, serde_json::Value::String(name.into()));
        assert_eq!(serde_json::from_value::<BasisRule>(j).unwrap(), r);
    }
    // A pre-P3 record serializes exactly as it did.
    let v = serde_json::to_value(legacy(BasisRule::Carryover723, Some("900.00"))).unwrap();
    assert_eq!(
        v,
        serde_json::json!({"rule": "carryover", "contributor_basis": "900.00"})
    );
}

/// Sec704cMethod: the three methods of Treas. Reg. §1.704-3(b)-(d).
#[test]
fn methods_are_the_three_of_1_704_3() {
    let keys: Vec<&str> = Sec704cMethod::ALL.iter().map(|m| m.key()).collect();
    assert_eq!(
        keys,
        ["traditional", "traditional_with_curative", "remedial"]
    );
    for m in Sec704cMethod::ALL {
        assert_eq!(Sec704cMethod::parse(m.key()), Some(m));
        let j = serde_json::to_value(m).unwrap();
        assert_eq!(j, serde_json::Value::String(m.key().into()));
    }
    assert_eq!(
        Sec704cMethod::Traditional.citation(),
        "Treas. Reg. §1.704-3(b)"
    );
    assert_eq!(Sec704cMethod::parse("ceiling"), None);
}

// --------------------------------------------------------- check refusals

/// INVARIANT 1 (check): a purchase takes no contributor basis, value or
/// method.
#[test]
fn a_purchase_with_contribution_facts_is_refused() {
    let (mut l, id) = one("1000.00");
    let before = l.clone();
    let mut with_basis = AssetBasis::purchase();
    with_basis.contributor_adjusted_basis = Some(d("500.00"));
    let mut with_fmv = AssetBasis::purchase();
    with_fmv.fmv_at_contribution = Some(d("1000.00"));
    let mut with_method = AssetBasis::purchase();
    with_method.sec704c_method = Some(Sec704cMethod::Traditional);
    for b in [with_basis, with_fmv, with_method] {
        assert!(matches!(
            l.check_asset_basis(id, &b),
            Err(LedgerError::InvalidTaxBasis(_))
        ));
        assert!(matches!(
            l.set_asset_basis(id, b),
            Err(LedgerError::InvalidTaxBasis(_))
        ));
    }
    assert_eq!(l, before, "a refusal changes nothing");
}

/// A contribution needs both the member's adjusted basis and the value at
/// contribution (a fact the human gives, never assumed from cost).
#[test]
fn a_contribution_missing_a_fact_is_refused() {
    let (mut l, id) = one("1000.00");
    let before = l.clone();
    for r in [BasisRule::Carryover723, BasisRule::PersonalUseConversion] {
        let mut no_basis = contribution(r, "500.00", "1000.00");
        no_basis.contributor_adjusted_basis = None;
        let mut no_fmv = contribution(r, "500.00", "1000.00");
        no_fmv.fmv_at_contribution = None;
        for b in [no_basis, no_fmv] {
            assert!(matches!(
                l.set_asset_basis(id, b),
                Err(LedgerError::InvalidTaxBasis(_))
            ));
        }
    }
    assert_eq!(l, before);
}

/// Each fact is non-negative, in cents, at most one quadrillion.
#[test]
fn a_negative_sub_cent_or_huge_fact_is_refused() {
    let (mut l, id) = one("1000.00");
    let before = l.clone();
    let huge = (ledger_core::kernel::MAX_LINE_AMOUNT + Decimal::ONE).to_string();
    for r in [BasisRule::Carryover723, BasisRule::PersonalUseConversion] {
        for (adj, fmv) in [
            ("-0.01", "100.00"),
            ("100.00", "-0.01"),
            ("100.001", "100.00"),
            ("100.00", "0.005"),
            (huge.as_str(), "100.00"),
            ("100.00", huge.as_str()),
        ] {
            assert!(
                matches!(
                    l.set_asset_basis(id, contribution(r, adj, fmv)),
                    Err(LedgerError::InvalidTaxBasis(_))
                ),
                "{r:?} {adj} {fmv}"
            );
        }
    }
    assert_eq!(l, before);
}

/// A source must name who gave the facts (Source::check).
#[test]
fn a_source_without_a_name_is_refused() {
    let (mut l, id) = one("1000.00");
    let mut b = contribution(BasisRule::Carryover723, "500.00", "1000.00");
    b.source = Some(ledger_core::settings::Source {
        decided_by: "   ".into(),
        decided_on: date(2026, 10, 20),
        citation: None,
        note: None,
    });
    assert!(matches!(
        l.set_asset_basis(id, b),
        Err(LedgerError::InvalidTaxBasis(_))
    ));
}

/// A zero adjusted basis (fully depreciated) and a zero value (worthless
/// at contribution) are facts like any other (R21).
#[test]
fn zero_basis_and_zero_value_are_accepted() {
    for (adj, fmv) in [("0.00", "300.00"), ("300.00", "0"), ("0", "0.00")] {
        let (mut l, id) = one("1000.00");
        l.set_asset_basis(id, contribution(BasisRule::Carryover723, adj, fmv))
            .unwrap();
        let f = l.get_asset(id).unwrap().basis_figures().unwrap().unwrap();
        assert_eq!(f.tax_basis_723, Some(d(adj)));
        assert_eq!(f.built_in_gain_704c, Some(d(fmv) - d(adj)));
    }
}

/// Valid records: check agrees with set, and a valid set changes nothing
/// but the asset's basis record (book figures don't change).
#[test]
fn valid_records_are_accepted_and_book_figures_stay() {
    let mut src = contribution(BasisRule::PersonalUseConversion, "750.50", "700.00");
    src.sec704c_method = Some(Sec704cMethod::Traditional);
    src.source = Some(ledger_core::settings::Source {
        decided_by: "Morgan Ellis".into(),
        decided_on: date(2026, 10, 20),
        citation: Some("entry 2".into()),
        note: None,
    });
    for b in [
        AssetBasis::purchase(),
        contribution(BasisRule::Carryover723, "750", "1000.00"),
        contribution(BasisRule::PersonalUseConversion, "750.5", "1000"),
        src,
    ] {
        let (mut l, id) = one("1000.00");
        l.check_asset_basis(id, &b).unwrap();
        let before = l.get_asset(id).unwrap().clone();
        l.set_asset_basis(id, b.clone()).unwrap();
        let after = l.get_asset(id).unwrap();
        assert_eq!(after.basis, Some(b.clone()));
        let mut same = after.clone();
        same.basis = before.basis.clone();
        assert_eq!(same, before, "book figures don't change: {b:?}");
    }
}

/// Set again, the new record replaces the old; a refusal keeps the old.
#[test]
fn setting_again_replaces_and_a_refusal_keeps() {
    let (mut l, id) = one("1000.00");
    l.set_asset_basis(id, AssetBasis::purchase()).unwrap();
    assert_eq!(
        l.get_asset(id).unwrap().depreciable_basis(),
        Some(d("1000.00"))
    );
    l.set_asset_basis(
        id,
        contribution(BasisRule::Carryover723, "400.00", "1000.00"),
    )
    .unwrap();
    assert_eq!(
        l.get_asset(id).unwrap().depreciable_basis(),
        Some(d("400.00"))
    );
    let before = l.clone();
    let mut bad = AssetBasis::purchase();
    bad.contributor_adjusted_basis = Some(d("1.00"));
    assert!(l.set_asset_basis(id, bad).is_err());
    assert_eq!(l, before);
}

// ------------------------------------------------------------ the figures

/// No record: no figures.
#[test]
fn no_record_means_no_figures() {
    let (l, id) = one("1000.00");
    let a = l.get_asset(id).unwrap();
    assert_eq!(a.basis_figures().unwrap(), None);
    assert_eq!(a.tax_basis_723(), None);
    assert_eq!(a.depreciable_basis(), None);
    assert_eq!(a.built_in_gain_704c(), None);
    assert!(!a.sec704c_1c());
    assert_eq!(a.fmv_at_contribution(), None);
}

/// A purchase: depreciable = cost (IRC §1012(a)); no §723 basis, no
/// built-in gain (not property contributed).
#[test]
fn a_purchase_depreciates_from_cost() {
    let (mut l, id) = one("1234.56");
    l.set_asset_basis(id, AssetBasis::purchase()).unwrap();
    let a = l.get_asset(id).unwrap();
    assert_eq!(a.depreciable_basis(), Some(d("1234.56")));
    assert_eq!(a.tax_basis_723(), None);
    assert_eq!(a.built_in_gain_704c(), None);
    assert_eq!(a.fmv_at_contribution(), None);
}

/// Named regression 4 (the brief's reproduction 5, anonymized): a laptop
/// worth 1,440.00 that the member held at 1,688.40 from personal use:
/// §723 basis 1,688.40, depreciable basis 1,440.00 (the lesser, D-41),
/// built-in loss 248.40, which counts only for the contributor
/// (§704(c)(1)(C)). Before P3 its "tax basis" was the 1,440.00.
#[test]
fn personal_use_contribution_keeps_its_723_basis_apart_from_depreciable_basis() {
    let (mut l, id) = one("1440.00");
    l.set_asset_basis(
        id,
        contribution(BasisRule::PersonalUseConversion, "1688.40", "1440.00"),
    )
    .unwrap();
    let a = l.get_asset(id).unwrap();
    assert_eq!(a.tax_basis_723(), Some(d("1688.40")));
    assert_eq!(a.depreciable_basis(), Some(d("1440.00")));
    assert_eq!(a.built_in_gain_704c(), Some(d("-248.40")));
    assert!(a.sec704c_1c());
    assert!(a.fmv_is_recorded());
}

/// Named regression 5 (the brief's reproduction 4): value 1,000.00 and
/// adjusted basis 400.00 give a built-in gain of 600.00, fixed at
/// contribution (Treas. Reg. §1.704-3(a)(3)(ii)): correcting the book cost
/// to 1,100.00 leaves it 600.00. A record from before P3 (no value
/// recorded, read as the book cost) moves with the cost, as it did then.
#[test]
fn built_in_gain_is_fixed_at_contribution_not_book_cost() {
    let (mut l, id) = one("1000.00");
    l.set_asset_basis(
        id,
        contribution(BasisRule::Carryover723, "400.00", "1000.00"),
    )
    .unwrap();
    assert_eq!(
        l.get_asset(id).unwrap().built_in_gain_704c(),
        Some(d("600.00"))
    );
    let fix = AssetCorrection {
        cost: usd("1100.00"),
        salvage_value: usd("0.00"),
        useful_life_months: 36,
        in_service_date: date(2026, 1, 1),
    };
    l.correct_asset(id, fix).unwrap();
    let a = l.get_asset(id).unwrap();
    assert_eq!(a.cost.amount, d("1100.00"));
    assert_eq!(a.built_in_gain_704c(), Some(d("600.00")));
    assert_eq!(a.fmv_at_contribution(), Some(d("1000.00")));
    // The pre-P3 record: value read as the book cost.
    let (mut l, id) = one("1000.00");
    l.replay_set_asset_tax_basis(id, BasisRule::Carryover723, Some(d("400.00")))
        .unwrap();
    assert_eq!(
        l.get_asset(id).unwrap().built_in_gain_704c(),
        Some(d("600.00"))
    );
    l.correct_asset(id, fix).unwrap();
    let a = l.get_asset(id).unwrap();
    assert_eq!(a.built_in_gain_704c(), Some(d("700.00")));
    assert!(!a.fmv_is_recorded());
}

/// The figure table (kernel::basis_figures INVARIANTS 2-4) on the ledger:
/// value equal to basis (no built-in gain: not §704(c) property); value
/// above basis for personal use (depreciable = basis); carryover above
/// and below.
#[test]
fn the_figures_follow_the_rule() {
    for (rule, fmv, adj, dep, gain) in [
        (
            BasisRule::Carryover723,
            "1000.00",
            "600.00",
            "600.00",
            "400.00",
        ),
        (
            BasisRule::Carryover723,
            "1000.00",
            "1500.00",
            "1500.00",
            "-500.00",
        ),
        (
            BasisRule::Carryover723,
            "1000.00",
            "1000.00",
            "1000.00",
            "0.00",
        ),
        (
            BasisRule::PersonalUseConversion,
            "1200.00",
            "2000.00",
            "1200.00",
            "-800.00",
        ),
        (
            BasisRule::PersonalUseConversion,
            "450.00",
            "300.00",
            "300.00",
            "150.00",
        ),
        (
            BasisRule::PersonalUseConversion,
            "999.99",
            "999.99",
            "999.99",
            "0.00",
        ),
        (
            BasisRule::PersonalUseConversion,
            "1000.00",
            "1000.01",
            "1000.00",
            "-0.01",
        ),
        (
            BasisRule::PersonalUseConversion,
            "1000.00",
            "999.99",
            "999.99",
            "0.01",
        ),
    ] {
        let (mut l, id) = one(fmv);
        l.set_asset_basis(id, contribution(rule, adj, fmv)).unwrap();
        let a = l.get_asset(id).unwrap();
        assert_eq!(a.tax_basis_723(), Some(d(adj)), "{rule:?} {fmv} {adj}");
        assert_eq!(a.depreciable_basis(), Some(d(dep)), "{rule:?} {fmv} {adj}");
        assert_eq!(
            a.built_in_gain_704c(),
            Some(d(gain)),
            "{rule:?} {fmv} {adj}"
        );
        assert_eq!(a.sec704c_1c(), d(gain) < Decimal::ZERO);
    }
}

/// The four-contributed-devices shape: two members contribute four devices
/// from personal use, each at the lower of its value and its basis: a
/// laptop and a phone worth less than their basis (built-in losses,
/// §704(c)(1)(C)), a camera worth more (a built-in gain: depreciable is its
/// basis), a monitor at its basis (neither). Register totals by hand:
/// book cost 1,400.00 + 900.00 + 1,300.00 + 500.00 = 4,100.00; §723 basis
/// 1,750.40 + 1,129.99 + 1,080.00 + 500.00 = 4,460.39; depreciable basis
/// 1,400.00 + 900.00 + 1,080.00 + 500.00 = 3,880.00; built-in gain
/// −350.40 − 229.99 + 220.00 + 0.00 = −360.39 = 4,100.00 − 4,460.39.
#[test]
fn four_contributed_devices_each_take_the_lower_of_value_and_basis() {
    let (mut l, a) = fixture();
    let devices = [
        ("Laptop", "1400.00", "1750.40", "1400.00", "-350.40", true),
        ("Phone", "900.00", "1129.99", "900.00", "-229.99", true),
        ("Camera", "1300.00", "1080.00", "1080.00", "220.00", false),
        ("Monitor", "500.00", "500.00", "500.00", "0.00", false),
    ];
    let mut ids = Vec::new();
    for (name, fmv, adj, _, _, _) in devices {
        let id = register(&mut l, &a, name, fmv);
        let mut b = contribution(BasisRule::PersonalUseConversion, adj, fmv);
        b.sec704c_method = Some(Sec704cMethod::Traditional);
        l.set_asset_basis(id, b).unwrap();
        ids.push(id);
    }
    for (id, (name, _, adj, dep, gain, loss)) in ids.iter().zip(devices) {
        let x = l.get_asset(*id).unwrap();
        assert_eq!(x.tax_basis_723(), Some(d(adj)), "{name}");
        assert_eq!(x.depreciable_basis(), Some(d(dep)), "{name}");
        assert_eq!(x.built_in_gain_704c(), Some(d(gain)), "{name}");
        assert_eq!(x.sec704c_1c(), loss, "{name}");
    }
    let r = fixed_asset_register(&l, date(2026, 6, 30)).unwrap();
    assert_eq!(r.total_cost, d("4100.00"));
    assert_eq!(r.total_tax_basis_723, d("4460.39"));
    assert_eq!(r.total_depreciable_basis, d("3880.00"));
    assert_eq!(r.missing_basis, 0);
    let gain: Decimal = r.rows.iter().filter_map(|x| x.built_in_gain_704c).sum();
    assert_eq!(gain, d("-360.39"));
    assert_eq!(gain, r.total_cost - r.total_tax_basis_723);
    assert_eq!(r.rows.iter().filter(|x| x.sec704c_1c).count(), 2);
    assert!(
        r.rows
            .iter()
            .all(|x| x.sec704c_method == Some(Sec704cMethod::Traditional))
    );
}

// -------------------------------------------------- legacy (pre-P3) lines

/// A pre-P3 `asset.tax_basis_set` replays to the figures that version
/// gave: cost → the book cost; carryover → the contributor's basis;
/// personal-use conversion → min(book cost, basis); built-in gain = book
/// cost − basis; the value read as the book cost, not recorded.
#[test]
fn legacy_records_give_the_figures_they_gave() {
    for (rule, basis, cost, dep, gain) in [
        (BasisRule::Purchase, None, "1234.56", "1234.56", None),
        (
            BasisRule::Carryover723,
            Some("650.25"),
            "500.00",
            "650.25",
            Some("-150.25"),
        ),
        (
            BasisRule::PersonalUseConversion,
            Some("2000.00"),
            "1200.00",
            "1200.00",
            Some("-800.00"),
        ),
        (
            BasisRule::PersonalUseConversion,
            Some("300.00"),
            "450.00",
            "300.00",
            Some("150.00"),
        ),
    ] {
        let (mut l, id) = one(cost);
        l.replay_set_asset_tax_basis(id, rule, basis.map(d))
            .unwrap();
        let a = l.get_asset(id).unwrap();
        assert_eq!(a.depreciable_basis(), Some(d(dep)), "{rule:?}");
        assert_eq!(a.built_in_gain_704c(), gain.map(d), "{rule:?}");
        assert_eq!(
            a.tax_basis_723(),
            basis.filter(|_| rule.is_contribution()).map(d)
        );
        assert!(!a.fmv_is_recorded());
        assert_eq!(a.basis, Some(legacy(rule, basis)));
    }
}

/// The legacy checks (as written through 2026-10-16) at replay: a purchase
/// takes no basis, a contribution needs one, non-negative, in cents; no
/// upper bound.
#[test]
fn legacy_replay_checks_what_it_checked() {
    let (mut l, id) = one("1000.00");
    for (rule, basis) in [
        (BasisRule::Purchase, Some("1.00")),
        (BasisRule::Carryover723, None),
        (BasisRule::PersonalUseConversion, Some("-1.00")),
        (BasisRule::Carryover723, Some("1.001")),
    ] {
        assert!(matches!(
            l.replay_set_asset_tax_basis(id, rule, basis.map(d)),
            Err(LedgerError::InvalidTaxBasis(_))
        ));
    }
    l.replay_set_asset_tax_basis(id, BasisRule::Carryover723, Some(Decimal::MAX))
        .unwrap();
}

/// An `asset.basis_set` at replay: its shape only (the facts fit the rule,
/// none negative), no bound on size or places (a later version may allow
/// more).
#[test]
fn basis_set_replay_checks_its_shape_only() {
    let (mut l, id) = one("1000.00");
    let mut purchase_with_fmv = AssetBasis::purchase();
    purchase_with_fmv.fmv_at_contribution = Some(d("1.00"));
    let mut no_fmv = contribution(BasisRule::Carryover723, "1.00", "1.00");
    no_fmv.fmv_at_contribution = None;
    for b in [
        purchase_with_fmv,
        no_fmv,
        contribution(BasisRule::Carryover723, "-1.00", "1.00"),
        contribution(BasisRule::Carryover723, "1.00", "-1.00"),
    ] {
        assert!(matches!(
            l.replay_set_asset_basis(id, b),
            Err(LedgerError::InvalidTaxBasis(_))
        ));
    }
    let wide = AssetBasis::contribution(BasisRule::Carryover723, d("1.0001"), Decimal::MAX);
    l.replay_set_asset_basis(id, wide.clone()).unwrap();
    assert_eq!(l.get_asset(id).unwrap().basis, Some(wide));
}

// ------------------------------------------------------------- the dates

/// Named regression 3 (the brief's reproduction 3): equipment bought
/// 2026-03-10 for 3,000.00 and placed in service 2026-04-01, 36 months.
/// Linked to its purchase, it is on the register from 2026-03-10: as of
/// 2026-03-31 its row is there with nothing accrued and the books' cost is
/// all registered; its schedule starts in April (a March charge would be
/// 3,000/36 × 22/31 = 59.14: none).
#[test]
fn an_asset_bought_before_it_is_in_service_is_on_the_register_from_acquisition() {
    let (mut l, a) = fixture();
    let bought = post(
        &mut l,
        date(2026, 3, 10),
        &[
            (a.equipment, Side::Debit, "3000.00"),
            (a.cash, Side::Credit, "3000.00"),
        ],
    );
    let id = l
        .register_asset(new_asset(&a, "Kiln", "3000.00", 36, date(2026, 4, 1)))
        .unwrap();
    // Unlinked, it would join the register only on 2026-04-01, while the
    // books carry its cost from 2026-03-10.
    let r = fixed_asset_register(&l, date(2026, 3, 31)).unwrap();
    assert!(r.rows.is_empty());
    l.set_asset_acquisition(id, bought).unwrap();
    assert_eq!(l.asset_acquired_date(id), Some(date(2026, 3, 10)));
    let r = fixed_asset_register(&l, date(2026, 3, 31)).unwrap();
    assert_eq!(r.rows.len(), 1);
    let x = &r.rows[0];
    assert_eq!(x.acquired_date, date(2026, 3, 10));
    assert_eq!(x.in_service_date, date(2026, 4, 1));
    assert_eq!(x.accumulated_scheduled, Decimal::ZERO);
    assert_eq!(x.accumulated_posted, Decimal::ZERO);
    assert_eq!(x.book_value, d("3000.00"));
    assert_eq!(r.unregistered_cost(), Decimal::ZERO);
    let sched =
        ledger_core::depreciation_schedule(l.get_asset(id).unwrap(), date(2026, 4, 30)).unwrap();
    assert_eq!(sched.len(), 1);
    assert_eq!(sched[0].period, "2026-04");
    assert_eq!(sched[0].amount, d("83.33"));
    // Before acquisition: not on the register.
    assert!(
        fixed_asset_register(&l, date(2026, 3, 9))
            .unwrap()
            .rows
            .is_empty()
    );
}

/// With no entry linked the acquired date is the in-service date; a link
/// gives the entry's, never later than the in-service date.
#[test]
fn the_acquired_date_comes_from_the_linked_entry() {
    let (mut l, a) = fixture();
    let id = l
        .register_asset(new_asset(&a, "Lathe", "900.00", 12, date(2026, 2, 1)))
        .unwrap();
    assert_eq!(l.asset_acquired_date(id), Some(date(2026, 2, 1)));
    let e = post(
        &mut l,
        date(2026, 1, 20),
        &[
            (a.equipment, Side::Debit, "900.00"),
            (a.capital, Side::Credit, "900.00"),
        ],
    );
    l.set_asset_acquisition(id, e).unwrap();
    assert_eq!(l.asset_acquired_date(id), Some(date(2026, 1, 20)));
    assert_eq!(l.asset_acquired_date(id + 9), None);
}

/// set_asset_acquisition INVARIANT 2: a new link to an entry dated after
/// the in-service date, or after the disposal, is refused; replay
/// accepts what earlier versions did, and the acquired date is never
/// later than the in-service date.
#[test]
fn a_link_after_service_or_disposal_is_refused_and_replays() {
    let (mut l, a) = fixture();
    let id = l
        .register_asset(new_asset(&a, "Lathe", "900.00", 12, date(2026, 2, 1)))
        .unwrap();
    let late = post(
        &mut l,
        date(2026, 2, 2),
        &[
            (a.equipment, Side::Debit, "900.00"),
            (a.cash, Side::Credit, "900.00"),
        ],
    );
    let before = l.clone();
    assert!(matches!(
        l.set_asset_acquisition(id, late),
        Err(LedgerError::AcquisitionEntryMismatch { .. })
    ));
    assert_eq!(l, before);
    l.replay_set_asset_acquisition(id, late).unwrap();
    assert_eq!(l.get_asset(id).unwrap().acquired_by, Some(late));
    assert_eq!(l.asset_acquired_date(id), Some(date(2026, 2, 1)));
    // After the disposal: bought 05-01, scrapped 05-15 before its
    // in-service date 06-01; a relink to an entry of 05-20 is refused.
    let (mut l, a) = fixture();
    let id = l
        .register_asset(new_asset(&a, "Drill", "300.00", 12, date(2026, 6, 1)))
        .unwrap();
    let buy = |l: &mut Ledger, on| {
        post(
            l,
            on,
            &[
                (a.equipment, Side::Debit, "300.00"),
                (a.cash, Side::Credit, "300.00"),
            ],
        )
    };
    let first = buy(&mut l, date(2026, 5, 1));
    l.set_asset_acquisition(id, first).unwrap();
    l.dispose_asset(id, date(2026, 5, 15), usd("0.00")).unwrap();
    let later = buy(&mut l, date(2026, 5, 20));
    assert!(matches!(
        l.set_asset_acquisition(id, later),
        Err(LedgerError::AcquisitionEntryMismatch { .. })
    ));
    let on_the_day = buy(&mut l, date(2026, 5, 15));
    l.set_asset_acquisition(id, on_the_day).unwrap();
}

/// correct_asset INVARIANT 3: a new in-service date before the linked
/// entry's date is refused.
#[test]
fn an_in_service_date_before_the_acquisition_is_refused() {
    let (mut l, a) = fixture();
    let id = l
        .register_asset(new_asset(&a, "Lathe", "900.00", 12, date(2026, 2, 1)))
        .unwrap();
    let e = post(
        &mut l,
        date(2026, 1, 20),
        &[
            (a.equipment, Side::Debit, "900.00"),
            (a.cash, Side::Credit, "900.00"),
        ],
    );
    l.set_asset_acquisition(id, e).unwrap();
    let fix = |on| AssetCorrection {
        cost: usd("900.00"),
        salvage_value: usd("0.00"),
        useful_life_months: 12,
        in_service_date: on,
    };
    let before = l.clone();
    assert!(matches!(
        l.correct_asset(id, fix(date(2026, 1, 19))),
        Err(LedgerError::InServiceBeforeAcquisition { .. })
    ));
    assert_eq!(l, before);
    l.correct_asset(id, fix(date(2026, 1, 20))).unwrap();
    assert_eq!(l.get_asset(id).unwrap().in_service_date, date(2026, 1, 20));
}

/// The disposal floor is the acquired date: a disposal between acquisition
/// and service has nothing accrued; one before acquisition is refused.
#[test]
fn a_disposal_between_acquisition_and_service_is_allowed() {
    let (mut l, a) = fixture();
    let id = l
        .register_asset(new_asset(&a, "Kiln", "3000.00", 36, date(2026, 4, 1)))
        .unwrap();
    let e = post(
        &mut l,
        date(2026, 3, 10),
        &[
            (a.equipment, Side::Debit, "3000.00"),
            (a.cash, Side::Credit, "3000.00"),
        ],
    );
    l.set_asset_acquisition(id, e).unwrap();
    assert!(matches!(
        l.dispose_asset(id, date(2026, 3, 9), usd("0.00")),
        Err(LedgerError::DisposalBeforeAcquisition { .. })
    ));
    let amounts =
        ledger_core::disposal_amounts(l.get_asset(id).unwrap(), date(2026, 3, 20), usd("2500.00"))
            .unwrap();
    assert_eq!(amounts.accumulated, Decimal::ZERO);
    assert_eq!(amounts.gain_loss, d("-500.00"));
    l.dispose_asset(id, date(2026, 3, 20), usd("2500.00"))
        .unwrap();
}

// ------------------------------------------------- posted and scheduled

/// Books with one asset (1,200.00 over 12 months from 2026-01-01: 100.00 a
/// month) and January's run.
fn one_run() -> (Ledger, Accts, AssetId, ledger_core::EntryId) {
    let (mut l, a) = fixture();
    let id = l
        .register_asset(new_asset(&a, "Printer", "1200.00", 12, date(2026, 1, 1)))
        .unwrap();
    let e = run(&mut l, &a, id, date(2026, 1, 31), "100.00");
    (l, a, id, e)
}

/// accumulated_posted INVARIANT 2: a run counts from its entry's date; a
/// reversal takes it back from the reversal's date (so a reversal dated
/// after as_of still leaves it counted).
#[test]
fn posted_follows_the_entries_dates() {
    let (mut l, _, id, e) = one_run();
    assert_eq!(l.accumulated_posted(id, date(2026, 1, 30)).unwrap(), d("0"));
    assert_eq!(
        l.accumulated_posted(id, date(2026, 1, 31)).unwrap(),
        d("100.00")
    );
    l.reverse_entry(e, date(2026, 3, 1), "undo").unwrap();
    assert_eq!(
        l.accumulated_posted(id, date(2026, 2, 28)).unwrap(),
        d("100.00")
    );
    assert_eq!(l.accumulated_posted(id, date(2026, 3, 1)).unwrap(), d("0"));
    assert_eq!(l.live_depreciation_run(id, "2026-01"), None);
}

/// A reversal of the reversal reinstates the run (B-9), and the run counts
/// again: the month is not due twice (review R2).
#[test]
fn a_reinstated_run_counts_and_is_live() {
    let (mut l, _, id, e) = one_run();
    let undo = l.reverse_entry(e, date(2026, 2, 1), "undo").unwrap();
    let redo = l.reverse_entry(undo, date(2026, 2, 2), "redo").unwrap();
    assert_eq!(l.accumulated_posted(id, date(2026, 2, 1)).unwrap(), d("0"));
    assert_eq!(
        l.accumulated_posted(id, date(2026, 2, 2)).unwrap(),
        d("100.00")
    );
    let live = l.live_depreciation_run(id, "2026-01").unwrap();
    assert_eq!(live.entry, e);
    let _ = redo;
}

/// A reversal dated before its run: between the two dates the books carry
/// −100.00, as the trial balance does (review R2).
#[test]
fn a_reversal_dated_before_its_run_matches_the_trial_balance() {
    let (mut l, a, id, e) = one_run();
    l.reverse_entry(e, date(2026, 1, 15), "early").unwrap();
    let on = date(2026, 1, 20);
    assert_eq!(l.accumulated_posted(id, on).unwrap(), d("-100.00"));
    let tb = ledger_core::trial_balance_as_of(&l, on);
    let accum = l.chart().get(a.accum).unwrap().code.clone();
    let line = tb.lines.iter().find(|x| x.code == accum).unwrap();
    assert_eq!(line.debit - line.credit, d("100.00"));
    assert_eq!(l.accumulated_posted(id, date(2026, 1, 31)).unwrap(), d("0"));
}

/// accumulated_posted INVARIANT 1: zero once disposed on or before as_of;
/// counted before.
#[test]
fn posted_is_zero_once_disposed() {
    let (mut l, _, id, _) = one_run();
    l.dispose_asset(id, date(2026, 2, 15), usd("900.00"))
        .unwrap();
    assert_eq!(
        l.accumulated_posted(id, date(2026, 2, 14)).unwrap(),
        d("100.00")
    );
    assert_eq!(l.accumulated_posted(id, date(2026, 2, 15)).unwrap(), d("0"));
    // Its runs stay: depreciation posted in January is still January's.
    assert_eq!(
        l.depreciation_posted_between(id, date(2026, 1, 1), date(2026, 12, 31))
            .unwrap(),
        d("100.00")
    );
}

/// The register's book value is cost less posted, the scheduled figure
/// beside it, and a hand entry in the account is unattributed.
#[test]
fn the_registers_book_value_is_what_the_books_carry() {
    let (mut l, a, id, _) = one_run();
    let r = fixed_asset_register(&l, date(2026, 3, 31)).unwrap();
    let x = &r.rows[0];
    assert_eq!(x.accumulated_scheduled, d("300.00"));
    assert_eq!(x.accumulated_posted, d("100.00"));
    assert_eq!(x.book_value, d("1100.00"));
    assert_eq!(r.total_book_value, d("1100.00"));
    assert_eq!(r.unattributed_accumulated(), Decimal::ZERO);
    post(
        &mut l,
        date(2026, 2, 28),
        &[
            (a.depr_expense, Side::Debit, "100.00"),
            (a.accum, Side::Credit, "100.00"),
        ],
    );
    let r = fixed_asset_register(&l, date(2026, 3, 31)).unwrap();
    assert_eq!(r.rows[0].book_value, d("1100.00"));
    assert_eq!(r.unattributed_accumulated(), d("100.00"));
    assert_eq!(r.unposted_depreciation(), d("100.00"));
    let _ = id;
}

/// A second line for the same run (asset, period and entry: a journal
/// edited by hand) is the run already recorded, as the projection keys it:
/// its entry is carried once (P3 review, 2026-10-21: replay counted it
/// twice, 200.00, and a reload once).
#[test]
fn a_duplicated_run_line_is_carried_once() {
    let (mut l, _, id, e) = one_run();
    l.record_depreciation_run(id, "2026-01", e, d("100.00"))
        .unwrap();
    assert_eq!(l.depreciation_runs(id).len(), 1);
    assert_eq!(
        l.accumulated_posted(id, date(2026, 1, 31)).unwrap(),
        d("100.00")
    );
}

/// check_depreciation_run: a month, a positive amount in cents, a posted
/// entry crediting the asset's accumulated depreciation by exactly that;
/// record_depreciation_run (replay) needs only the asset and entry.
#[test]
fn a_run_must_be_its_entry() {
    let (mut l, a, id, e) = one_run();
    for (period, amount, why) in [
        ("2026-1", "100.00", "period"),
        ("10000-01", "100.00", "period"),
        ("2026-13", "100.00", "period"),
        ("2026-01", "0", "amount"),
        ("2026-01", "100.001", "amount"),
        ("2026-01", "99.99", "credit"),
    ] {
        assert!(
            matches!(
                l.check_depreciation_run(id, period, e, d(amount)),
                Err(LedgerError::InvalidDepreciationRun { .. })
            ),
            "{why}"
        );
    }
    assert!(matches!(
        l.check_depreciation_run(id + 9, "2026-01", e, d("100.00")),
        Err(LedgerError::UnknownAsset(_))
    ));
    let draft = l.try_create_entry(date(2026, 2, 28), "draft").unwrap();
    l.add_line(draft, a.depr_expense, Side::Debit, usd("100.00"))
        .unwrap();
    l.add_line(draft, a.accum, Side::Credit, usd("100.00"))
        .unwrap();
    assert!(
        l.check_depreciation_run(id, "2026-02", draft, d("100.00"))
            .is_err()
    );
    // Replay: any journaled period and amount, the asset and entry known.
    l.record_depreciation_run(id, "10000-01", e, d("0.001"))
        .unwrap();
    assert!(matches!(
        l.record_depreciation_run(id, "2026-01", e + 99, d("1")),
        Err(LedgerError::UnknownEntry(_))
    ));
    assert!(matches!(
        l.record_depreciation_run(id + 9, "2026-01", e, d("1")),
        Err(LedgerError::UnknownAsset(_))
    ));
}

// ---------------------------------------------------------- the register

/// Register rows carry each asset's facts and figures (INVARIANT 5) and
/// the totals cover held assets only (INVARIANT 3): a purchase, a
/// carryover, a personal-use conversion, one with no basis, one disposed.
/// By hand: depreciable 3,000.00 + 650.25 + 1,200.00 = 4,850.25; §723
/// 650.25 + 2,000.00 = 2,650.25; the disposed one left out.
#[test]
fn register_rows_carry_the_assets_figures_and_totals_the_held() {
    let (mut l, a) = fixture();
    let bought = register(&mut l, &a, "Server", "3000.00");
    let carried = register(&mut l, &a, "Printer", "500.00");
    let converted = register(&mut l, &a, "Laptop", "1200.00");
    let unrecorded = register(&mut l, &a, "Desk", "700.00");
    let sold = register(&mut l, &a, "Old monitor", "360.00");
    l.set_asset_basis(bought, AssetBasis::purchase()).unwrap();
    l.set_asset_basis(
        carried,
        contribution(BasisRule::Carryover723, "650.25", "500.00"),
    )
    .unwrap();
    l.set_asset_basis(
        converted,
        contribution(BasisRule::PersonalUseConversion, "2000.00", "1200.00"),
    )
    .unwrap();
    l.set_asset_basis(sold, AssetBasis::purchase()).unwrap();
    l.dispose_asset(sold, date(2026, 3, 31), usd("100.00"))
        .unwrap();
    let r = fixed_asset_register(&l, date(2026, 6, 30)).unwrap();
    for row in &r.rows {
        let x = l.get_asset(row.asset_id).unwrap();
        let f = x.basis_figures().unwrap();
        assert_eq!(row.basis_rule, x.basis.as_ref().map(|b| b.rule));
        assert_eq!(row.tax_basis_723, f.and_then(|f| f.tax_basis_723));
        assert_eq!(row.depreciable_basis, f.map(|f| f.depreciable_basis));
        assert_eq!(row.built_in_gain_704c, f.and_then(|f| f.built_in_gain_704c));
        assert_eq!(row.fmv_at_contribution, x.fmv_at_contribution());
    }
    let row = |id| r.rows.iter().find(|x| x.asset_id == id).unwrap();
    assert_eq!(row(carried).built_in_gain_704c, Some(d("-150.25")));
    assert!(row(carried).sec704c_1c);
    assert_eq!(row(converted).depreciable_basis, Some(d("1200.00")));
    assert_eq!(row(converted).tax_basis_723, Some(d("2000.00")));
    assert_eq!(row(unrecorded).depreciable_basis, None);
    assert!(row(sold).disposed.is_some());
    assert_eq!(r.total_depreciable_basis, d("4850.25"));
    assert_eq!(r.total_tax_basis_723, d("2650.25"));
    assert_eq!(r.missing_basis, 1);
    // Before the disposal the monitor is held.
    let r = fixed_asset_register(&l, date(2026, 2, 28)).unwrap();
    assert_eq!(r.total_depreciable_basis, d("5210.25"));
}

/// Recording a basis leaves the book totals alone.
#[test]
fn a_basis_does_not_move_book_totals() {
    let (mut l, a) = fixture();
    let x = register(&mut l, &a, "Laptop", "1200.00");
    let before = fixed_asset_register(&l, date(2026, 6, 30)).unwrap();
    l.set_asset_basis(
        x,
        contribution(BasisRule::PersonalUseConversion, "2000.00", "1200.00"),
    )
    .unwrap();
    let after = fixed_asset_register(&l, date(2026, 6, 30)).unwrap();
    assert_eq!(after.total_cost, before.total_cost);
    assert_eq!(
        after.total_accumulated_scheduled,
        before.total_accumulated_scheduled
    );
    assert_eq!(
        after.total_accumulated_posted,
        before.total_accumulated_posted
    );
    assert_eq!(after.total_book_value, before.total_book_value);
    assert_eq!(after.ledger_cost, before.ledger_cost);
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

    /// The figures follow the rule over random costs, values and bases,
    /// and the register's totals agree (prop_basis_figures_laws on the
    /// ledger).
    #[test]
    fn prop_figures_follow_their_rule(
        assets in proptest::collection::vec((positive_cents(), cents(), cents(), 0usize..4), 1..8)
    ) {
        let (mut l, a) = fixture();
        let (mut dep_total, mut t723_total, mut missing) = (Decimal::ZERO, Decimal::ZERO, 0usize);
        for (i, (cost, adj, fmv, rule)) in assets.iter().enumerate() {
            let id = register(&mut l, &a, &format!("Asset {i}"), &cost.to_string());
            match rule {
                0 => {
                    l.set_asset_basis(id, AssetBasis::purchase()).unwrap();
                    let x = l.get_asset(id).unwrap();
                    prop_assert_eq!(x.depreciable_basis(), Some(*cost));
                    prop_assert_eq!(x.built_in_gain_704c(), None);
                    dep_total += *cost;
                }
                1 | 2 => {
                    let r = RULES[*rule];
                    l.set_asset_basis(id, AssetBasis::contribution(r, *adj, *fmv)).unwrap();
                    let x = l.get_asset(id).unwrap();
                    prop_assert_eq!(x.tax_basis_723(), Some(*adj));
                    prop_assert_eq!(x.built_in_gain_704c(), Some(*fmv - *adj));
                    prop_assert_eq!(x.sec704c_1c(), adj > fmv);
                    let dep = x.depreciable_basis().unwrap();
                    if r == BasisRule::PersonalUseConversion {
                        prop_assert!(dep <= *adj && dep <= *fmv);
                        prop_assert!(dep == *adj || dep == *fmv);
                    } else {
                        prop_assert_eq!(dep, *adj);
                    }
                    dep_total += dep;
                    t723_total += *adj;
                }
                _ => missing += 1,
            }
        }
        let r = fixed_asset_register(&l, date(2026, 6, 30)).unwrap();
        prop_assert_eq!(r.total_depreciable_basis, dep_total);
        prop_assert_eq!(r.total_tax_basis_723, t723_total);
        prop_assert_eq!(r.missing_basis, missing);
    }

    /// check agrees with set, and a refused record changes nothing; the
    /// spec, independently: a purchase takes no facts, a contribution both,
    /// each non-negative and in cents.
    #[test]
    fn prop_check_agrees_with_set(
        adj in proptest::option::of((-1_000_000i64..1_000_000, 0u32..5)),
        fmv in proptest::option::of((-1_000_000i64..1_000_000, 0u32..5)),
        rule in 0usize..3,
    ) {
        let (mut l, id) = one("1000.00");
        let b = AssetBasis {
            rule: RULES[rule],
            contributor_adjusted_basis: adj.map(|(c, s)| Decimal::new(c, s)),
            fmv_at_contribution: fmv.map(|(c, s)| Decimal::new(c, s)),
            sec704c_method: None,
            source: None,
        };
        let before = l.clone();
        let checked = l.check_asset_basis(id, &b);
        let done = l.set_asset_basis(id, b.clone());
        prop_assert_eq!(checked.is_ok(), done.is_ok());
        let fine = |v: Option<Decimal>| v.is_some_and(|v| v >= Decimal::ZERO && v.normalize().scale() <= 2);
        let valid = match RULES[rule] {
            BasisRule::Purchase => adj.is_none() && fmv.is_none(),
            _ => fine(b.contributor_adjusted_basis) && fine(b.fmv_at_contribution),
        };
        prop_assert_eq!(done.is_ok(), valid, "{:?}", b);
        if done.is_err() {
            prop_assert_eq!(&l, &before);
        } else {
            prop_assert_eq!(l.get_asset(id).unwrap().basis.clone(), Some(b));
        }
    }
}

/// Recording a basis or a run changes no entry and no figure of
/// the trial balance (review R16: these commands are records about the
/// books, as Lean's `Step.record` is; the purity law of
/// `settings_replay_is_pure`).
#[test]
fn basis_links_and_runs_change_no_entry_or_balance() {
    let (mut l, a, id, e) = one_run();
    let tb = |l: &Ledger| {
        serde_json::to_value(ledger_core::trial_balance_as_of(l, date(2026, 12, 31))).unwrap()
    };
    let entries =
        |l: &Ledger| -> Vec<ledger_core::JournalEntry> { l.posted_entries().cloned().collect() };
    let (before_tb, before_entries) = (tb(&l), entries(&l));
    l.set_asset_basis(
        id,
        contribution(BasisRule::Carryover723, "1000.00", "1200.00"),
    )
    .unwrap();
    l.replay_set_asset_tax_basis(id, BasisRule::PersonalUseConversion, Some(d("1500.00")))
        .unwrap();
    l.record_depreciation_run(id, "2026-01", e, d("100.00"))
        .unwrap();
    let _ = a;
    assert_eq!(tb(&l), before_tb);
    assert_eq!(entries(&l), before_entries);
}
