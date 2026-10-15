//! Spec-derived tests for asset tags and serial numbers (rules.md B-24) and
//! the fixed asset register (`reports::fixed_asset_register`).
//!
//! Every expected value comes from the written specification: the doc
//! comments and GOAL/SOURCE/INVARIANTS/SCENARIOS blocks on
//! `assets::check_asset_tag`, `assets::check_serial`, `assets::tag_key`,
//! `Ledger::set_asset_tag`, `check_asset_tag_for`, `set_asset_serial`,
//! `asset_by_tag`, `next_asset_tag`, `fixed_asset_register` and
//! `AssetRegister`, and `docs/spec/rules.md` B-15 and B-24. Depreciation
//! amounts are worked by hand (straight line, whole months) or, for
//! mid-month dates, taken from the already-tested building block
//! `accumulated_depreciation`. Nothing is read back from the code under
//! test and then asserted.

use std::collections::BTreeMap;
use std::str::FromStr;

use chrono::NaiveDate;
use ledger_core::assets::{
    MAX_ASSET_TAG_LEN, MAX_SERIAL_LEN, check_asset_tag, check_serial, tag_key,
};
use ledger_core::{
    AccountId, AccountType, AssetId, Currency, Ledger, LedgerError, Money, NewAsset, Side,
    accumulated_depreciation, fixed_asset_register,
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
    vehicles: AccountId,
    vehicles_accum: AccountId,
    depr_expense: AccountId,
    gain: AccountId,
}

/// A USD ledger with a fixed-asset chart, funded with $50,000 cash on
/// 2026-01-01 (Dr 1000 / Cr 3000).
fn fixture() -> (Ledger, Accts) {
    let mut l = Ledger::new(Currency::USD);
    let mut acct = |code: &str, name: &str, ty: AccountType| {
        l.add_account(code, name, ty, None, false).unwrap()
    };
    let cash = acct("1000", "Cash", AccountType::Asset);
    let equipment = acct("1500", "Equipment", AccountType::Asset);
    let accum = acct("1510", "Accumulated Depreciation", AccountType::Asset);
    let vehicles = acct("1520", "Vehicles", AccountType::Asset);
    let vehicles_accum = acct(
        "1530",
        "Accumulated Depreciation — Vehicles",
        AccountType::Asset,
    );
    let capital = acct("3000", "Owner Capital", AccountType::Equity);
    let gain = acct("4900", "Gain on Disposal", AccountType::Revenue);
    let depr_expense = acct("6200", "Depreciation Expense", AccountType::Expense);
    post(
        &mut l,
        date(2026, 1, 1),
        &[
            (cash, Side::Debit, "50000.00"),
            (capital, Side::Credit, "50000.00"),
        ],
    );
    (
        l,
        Accts {
            cash,
            equipment,
            accum,
            vehicles,
            vehicles_accum,
            depr_expense,
            gain,
        },
    )
}

fn post(l: &mut Ledger, on: NaiveDate, lines: &[(AccountId, Side, &str)]) {
    let e = l.try_create_entry(on, "test entry").unwrap();
    for (a, side, amt) in lines {
        l.add_line(e, *a, *side, usd(amt)).unwrap();
    }
    l.post_entry(e).unwrap();
}

fn equipment(a: &Accts, description: &str, cost: &str, life: u32, on: NaiveDate) -> NewAsset {
    NewAsset {
        description: description.to_string(),
        purchase_date: on,
        cost: usd(cost),
        salvage_value: usd("0.00"),
        useful_life_months: life,
        cost_account: a.equipment,
        accum_depr_account: a.accum,
        expense_account: a.depr_expense,
    }
}

/// A ledger with `n` registered laptops (purchased 2026-01-01).
fn with_assets(n: usize) -> (Ledger, Accts, Vec<AssetId>) {
    let (mut l, a) = fixture();
    let ids = (0..n)
        .map(|i| {
            l.register_asset(equipment(
                &a,
                &format!("Laptop {i}"),
                "1200.00",
                12,
                date(2026, 1, 1),
            ))
            .unwrap()
        })
        .collect();
    (l, a, ids)
}

fn tag_of(l: &Ledger, id: AssetId) -> Option<String> {
    l.get_asset(id).unwrap().tag.clone()
}

fn serial_of(l: &Ledger, id: AssetId) -> Option<String> {
    l.get_asset(id).unwrap().serial.clone()
}

// ------------------------------------------------- check_asset_tag (form)

/// check_asset_tag: 1 to MAX_ASSET_TAG_LEN characters of ASCII letters,
/// digits, `- _ . /`, at least one letter or digit.
#[test]
fn check_asset_tag_accepts_the_documented_forms() {
    for ok in [
        "FL-0001",
        "a",
        "7",
        "A_B.C/D-9",
        "fl-0001",
        "-a-",
        "./x",
        "LAPTOP",
    ] {
        assert!(check_asset_tag(ok).is_ok(), "{ok:?} should be accepted");
    }
    let longest = "A".repeat(MAX_ASSET_TAG_LEN);
    assert!(check_asset_tag(&longest).is_ok());
}

/// B-24 says 1–32 characters: the constant agrees with the rule.
#[test]
fn max_asset_tag_len_is_32_per_b24() {
    assert_eq!(MAX_ASSET_TAG_LEN, 32);
}

/// check_asset_tag INVARIANT 1: no spaces, no control or non-ASCII
/// characters, at least one letter or digit; and 1..=MAX length.
#[test]
fn check_asset_tag_refuses_wrong_forms() {
    let too_long = "A".repeat(MAX_ASSET_TAG_LEN + 1);
    for bad in [
        "",
        "FL 0001",
        " FL",
        "FL\t1",
        "FL\n1",
        "FL\u{7}1",
        "ÄB-1",
        "FL–0001", // en dash, not ASCII
        "FL#1",
        "FL,1",
        "---",
        "./",
        "_",
        too_long.as_str(),
    ] {
        let r = check_asset_tag(bad);
        assert!(r.is_err(), "{bad:?} should be refused");
        assert!(!r.unwrap_err().is_empty(), "refusal says why");
    }
}

// ------------------------------------------------- check_serial (form)

/// check_serial: 1 to MAX_SERIAL_LEN characters, no control characters,
/// no leading or trailing space. Spaces inside and non-ASCII are allowed
/// (only control characters and edge spaces are excluded).
#[test]
fn check_serial_accepts_the_documented_forms() {
    for ok in ["C02XG0FDJGH5", "SN 123 456", "x", "Ünïcødé-ß", "a/b#c"] {
        assert!(check_serial(ok).is_ok(), "{ok:?} should be accepted");
    }
    assert!(check_serial(&"S".repeat(MAX_SERIAL_LEN)).is_ok());
    // The limit is in characters: 64 two-byte characters are 64 characters.
    assert!(check_serial(&"é".repeat(MAX_SERIAL_LEN)).is_ok());
}

#[test]
fn max_serial_len_is_64() {
    assert_eq!(MAX_SERIAL_LEN, 64);
}

#[test]
fn check_serial_refuses_wrong_forms() {
    let too_long = "S".repeat(MAX_SERIAL_LEN + 1);
    for bad in [
        "",
        " ",
        "   ",
        " SN1",
        "SN1 ",
        "SN\n1",
        "SN\u{0}1",
        "SN\u{7f}1",
        too_long.as_str(),
    ] {
        let r = check_serial(bad);
        assert!(r.is_err(), "{bad:?} should be refused");
        assert!(!r.unwrap_err().is_empty(), "refusal says why");
    }
}

// ------------------------------------------------- tag_key

/// tag_key: "fl-0001" is "FL-0001".
#[test]
fn tag_key_ignores_case() {
    assert_eq!(tag_key("fl-0001"), tag_key("FL-0001"));
    assert_eq!(tag_key("Fl-0001"), tag_key("fL-0001"));
    assert_ne!(tag_key("FL-0001"), tag_key("FL-0002"));
}

// ------------------------------------------------- set_asset_tag

/// Asset.tag doc: "None until one is set"; same for serial.
#[test]
fn a_new_asset_has_no_tag_or_serial() {
    let (l, _, ids) = with_assets(1);
    assert_eq!(tag_of(&l, ids[0]), None);
    assert_eq!(serial_of(&l, ids[0]), None);
}

#[test]
fn set_asset_tag_records_the_tag() {
    let (mut l, _, ids) = with_assets(1);
    l.set_asset_tag(ids[0], "FL-0001").unwrap();
    assert_eq!(tag_of(&l, ids[0]).as_deref(), Some("FL-0001"));
    assert_eq!(l.asset_by_tag("FL-0001").map(|a| a.id), Some(ids[0]));
}

/// SCENARIO 1: unknown asset: refuse; INVARIANT 3: nothing changes.
#[test]
fn set_asset_tag_refuses_an_unknown_asset_unchanged() {
    let (mut l, _, ids) = with_assets(1);
    let before = l.clone();
    assert!(l.set_asset_tag(ids[0] + 100, "FL-0001").is_err());
    assert_eq!(l, before);
    // ...and the tag is still free for a real asset.
    l.set_asset_tag(ids[0], "FL-0001").unwrap();
}

/// SCENARIO 2: wrong form: InvalidAssetTag, nothing changes.
#[test]
fn set_asset_tag_refuses_a_bad_form_unchanged() {
    let (mut l, _, ids) = with_assets(1);
    l.set_asset_tag(ids[0], "FL-0001").unwrap();
    let before = l.clone();
    for bad in ["", "FL 2", "É-1", "--"] {
        let err = l.set_asset_tag(ids[0], bad).unwrap_err();
        assert!(
            matches!(&err, LedgerError::InvalidAssetTag { tag, .. } if tag == bad),
            "{bad:?}: {err:?}"
        );
        assert_eq!(l, before);
    }
}

/// SCENARIO 3 and INVARIANT 1: another asset holds the tag (ignoring case):
/// AssetTagTaken naming the holder, nothing changes.
#[test]
fn set_asset_tag_refuses_a_tag_another_asset_holds_ignoring_case() {
    let (mut l, _, ids) = with_assets(2);
    l.set_asset_tag(ids[0], "FL-0001").unwrap();
    let before = l.clone();
    for t in ["FL-0001", "fl-0001", "Fl-0001"] {
        let err = l.set_asset_tag(ids[1], t).unwrap_err();
        assert!(
            matches!(&err, LedgerError::AssetTagTaken { asset, .. } if *asset == ids[0]),
            "{t:?}: {err:?}"
        );
        assert_eq!(l, before);
    }
}

/// INVARIANT 2: a tag released by a retag is never reused by another asset.
#[test]
fn a_retagged_assets_old_tag_is_never_reused() {
    let (mut l, _, ids) = with_assets(2);
    l.set_asset_tag(ids[0], "FL-0001").unwrap();
    l.set_asset_tag(ids[0], "FL-0100").unwrap();
    assert_eq!(tag_of(&l, ids[0]).as_deref(), Some("FL-0100"));
    let before = l.clone();
    let err = l.set_asset_tag(ids[1], "fl-0001").unwrap_err();
    assert!(
        matches!(err, LedgerError::AssetTagTaken { asset, .. } if asset == ids[0]),
        "{err:?}"
    );
    assert_eq!(l, before);
}

/// INVARIANT 2: a disposed asset's tag is never reused either.
#[test]
fn a_disposed_assets_tag_is_never_reused() {
    let (mut l, _, ids) = with_assets(2);
    l.set_asset_tag(ids[0], "FL-0001").unwrap();
    l.dispose_asset(ids[0], date(2026, 6, 30), usd("100.00"))
        .unwrap();
    let before = l.clone();
    let err = l.set_asset_tag(ids[1], "FL-0001").unwrap_err();
    assert!(
        matches!(err, LedgerError::AssetTagTaken { asset, .. } if asset == ids[0]),
        "{err:?}"
    );
    assert_eq!(l, before);
}

/// INVARIANT 2: "The same asset may take back a tag it held."
#[test]
fn an_asset_may_take_back_a_tag_it_held() {
    let (mut l, _, ids) = with_assets(1);
    l.set_asset_tag(ids[0], "FL-0001").unwrap();
    l.set_asset_tag(ids[0], "FL-0002").unwrap();
    l.set_asset_tag(ids[0], "FL-0001").unwrap();
    assert_eq!(tag_of(&l, ids[0]).as_deref(), Some("FL-0001"));
    assert_eq!(l.asset_by_tag("FL-0001").map(|a| a.id), Some(ids[0]));
    assert!(l.asset_by_tag("FL-0002").is_none());
}

/// SCENARIO 4: the asset already has that tag: accept, no change.
#[test]
fn setting_the_same_tag_again_changes_nothing() {
    let (mut l, _, ids) = with_assets(1);
    l.set_asset_tag(ids[0], "FL-0001").unwrap();
    let before = l.clone();
    l.set_asset_tag(ids[0], "FL-0001").unwrap();
    assert_eq!(l, before);
}

/// Unique ignoring case is about *other* assets: an asset's own tag in
/// another case is its own (INVARIANT 1 is about two assets).
#[test]
fn an_asset_may_take_its_own_tag_in_another_case() {
    let (mut l, _, ids) = with_assets(1);
    l.set_asset_tag(ids[0], "FL-0001").unwrap();
    l.set_asset_tag(ids[0], "fl-0001").unwrap();
    assert_eq!(l.asset_by_tag("FL-0001").map(|a| a.id), Some(ids[0]));
}

// ------------------------------------------------- check_asset_tag_for

/// check_asset_tag_for mirrors set_asset_tag's refusals and changes nothing.
#[test]
fn check_asset_tag_for_mirrors_set_asset_tag() {
    let (mut l, _, ids) = with_assets(2);
    l.set_asset_tag(ids[0], "FL-0001").unwrap();
    l.set_asset_tag(ids[0], "FL-0002").unwrap();
    let before = l.clone();
    // An existing asset.
    assert!(l.check_asset_tag_for(Some(ids[1]), "FL-0003").is_ok());
    assert!(matches!(
        l.check_asset_tag_for(Some(ids[1]), "fl-0001"),
        Err(LedgerError::AssetTagTaken { asset, .. }) if asset == ids[0]
    ));
    assert!(matches!(
        l.check_asset_tag_for(Some(ids[1]), "bad tag"),
        Err(LedgerError::InvalidAssetTag { .. })
    ));
    assert!(l.check_asset_tag_for(Some(ids[0]), "FL-0001").is_ok());
    assert!(l.check_asset_tag_for(Some(ids[1] + 50), "FL-0009").is_err());
    // An asset not yet registered: any tag ever held is taken.
    assert!(l.check_asset_tag_for(None, "FL-0003").is_ok());
    assert!(matches!(
        l.check_asset_tag_for(None, "FL-0002"),
        Err(LedgerError::AssetTagTaken { asset, .. }) if asset == ids[0]
    ));
    assert!(matches!(
        l.check_asset_tag_for(None, "FL-0001"),
        Err(LedgerError::AssetTagTaken { asset, .. }) if asset == ids[0]
    ));
    assert!(matches!(
        l.check_asset_tag_for(None, ""),
        Err(LedgerError::InvalidAssetTag { .. })
    ));
    assert_eq!(l, before, "checking changes nothing");
}

// ------------------------------------------------- asset_by_tag

/// asset_by_tag: the asset whose *current* tag is `tag`, ignoring case.
#[test]
fn asset_by_tag_finds_current_tags_ignoring_case() {
    let (mut l, _, ids) = with_assets(2);
    l.set_asset_tag(ids[0], "FL-0001").unwrap();
    l.set_asset_tag(ids[1], "Desk/7").unwrap();
    assert_eq!(l.asset_by_tag("fl-0001").map(|a| a.id), Some(ids[0]));
    assert_eq!(l.asset_by_tag("DESK/7").map(|a| a.id), Some(ids[1]));
    assert!(l.asset_by_tag("FL-0002").is_none());
    assert!(l.asset_by_tag("").is_none());
    l.set_asset_tag(ids[0], "FL-0005").unwrap();
    assert!(
        l.asset_by_tag("FL-0001").is_none(),
        "an old tag is no longer current"
    );
    assert_eq!(l.asset_by_tag("FL-0005").map(|a| a.id), Some(ids[0]));
}

/// A disposed asset keeps its tag (nothing in the spec takes it away).
#[test]
fn asset_by_tag_still_finds_a_disposed_asset() {
    let (mut l, _, ids) = with_assets(1);
    l.set_asset_tag(ids[0], "FL-0001").unwrap();
    l.dispose_asset(ids[0], date(2026, 6, 30), usd("100.00"))
        .unwrap();
    assert_eq!(l.asset_by_tag("FL-0001").map(|a| a.id), Some(ids[0]));
}

// ------------------------------------------------- next_asset_tag

#[test]
fn next_asset_tag_starts_at_0001() {
    let (l, _, _) = with_assets(0);
    assert_eq!(l.next_asset_tag("FL"), "FL-0001");
    assert_eq!(l.next_asset_tag("FA"), "FA-0001");
}

/// The lowest number no asset holds or held under that prefix.
#[test]
fn next_asset_tag_is_the_lowest_never_held_number() {
    let (mut l, _, ids) = with_assets(3);
    l.set_asset_tag(ids[0], "FL-0001").unwrap();
    l.set_asset_tag(ids[1], "FL-0003").unwrap();
    assert_eq!(l.next_asset_tag("FL"), "FL-0002");
    l.set_asset_tag(ids[2], "FL-0002").unwrap();
    assert_eq!(l.next_asset_tag("FL"), "FL-0004");
    // Retagging FL-0001 away doesn't free it.
    l.set_asset_tag(ids[0], "Laptop-A").unwrap();
    assert_eq!(l.next_asset_tag("FL"), "FL-0004");
    // Other prefixes are independent.
    assert_eq!(l.next_asset_tag("FA"), "FA-0001");
}

/// Tags are unique ignoring case, so a lower-case held tag is held.
#[test]
fn next_asset_tag_skips_numbers_held_in_another_case() {
    let (mut l, _, ids) = with_assets(1);
    l.set_asset_tag(ids[0], "fl-0001").unwrap();
    assert_eq!(l.next_asset_tag("FL"), "FL-0002");
}

/// A disposed asset's tag stays held.
#[test]
fn next_asset_tag_skips_a_disposed_assets_tag() {
    let (mut l, _, ids) = with_assets(1);
    l.set_asset_tag(ids[0], "FL-0001").unwrap();
    l.dispose_asset(ids[0], date(2026, 3, 31), usd("0.00"))
        .unwrap();
    assert_eq!(l.next_asset_tag("FL"), "FL-0002");
}

/// "Numbers past 9999 widen."
#[test]
fn next_asset_tag_widens_past_9999() {
    let (mut l, _, ids) = with_assets(1);
    for n in 1..=9999 {
        l.set_asset_tag(ids[0], &format!("FL-{n:04}")).unwrap();
    }
    assert_eq!(l.next_asset_tag("FL"), "FL-10000");
}

/// A generated tag is always one the ledger would accept for a new asset.
#[test]
fn next_asset_tag_is_accepted() {
    let (mut l, _, ids) = with_assets(2);
    l.set_asset_tag(ids[0], "FL-0001").unwrap();
    let t = l.next_asset_tag("FL");
    assert!(l.check_asset_tag_for(None, &t).is_ok());
    l.set_asset_tag(ids[1], &t).unwrap();
}

// ------------------------------------------------- set_asset_serial

#[test]
fn set_asset_serial_records_and_clears() {
    let (mut l, _, ids) = with_assets(1);
    l.set_asset_serial(ids[0], Some("C02XG0FDJGH5")).unwrap();
    assert_eq!(serial_of(&l, ids[0]).as_deref(), Some("C02XG0FDJGH5"));
    l.set_asset_serial(ids[0], Some("SN 2")).unwrap();
    assert_eq!(serial_of(&l, ids[0]).as_deref(), Some("SN 2"));
    l.set_asset_serial(ids[0], None).unwrap();
    assert_eq!(serial_of(&l, ids[0]), None);
}

/// SCENARIO 1: unknown asset or bad form: refuse, changing nothing.
#[test]
fn set_asset_serial_refusals_change_nothing() {
    let (mut l, _, ids) = with_assets(1);
    l.set_asset_serial(ids[0], Some("SN-1")).unwrap();
    let before = l.clone();
    assert!(l.set_asset_serial(ids[0] + 9, Some("SN-2")).is_err());
    assert!(l.set_asset_serial(ids[0] + 9, None).is_err());
    assert_eq!(l, before);
    for bad in ["", " SN", "SN\n"] {
        assert!(
            matches!(
                l.set_asset_serial(ids[0], Some(bad)),
                Err(LedgerError::InvalidSerial(_))
            ),
            "{bad:?}"
        );
        assert_eq!(l, before);
    }
    // check_asset_serial agrees and changes nothing.
    assert!(l.check_asset_serial(ids[0], Some("ok")).is_ok());
    assert!(l.check_asset_serial(ids[0], None).is_ok());
    assert!(matches!(
        l.check_asset_serial(ids[0], Some(" x")),
        Err(LedgerError::InvalidSerial(_))
    ));
    assert!(l.check_asset_serial(ids[0] + 9, Some("ok")).is_err());
    assert_eq!(l, before);
}

/// Serials don't touch tags.
#[test]
fn a_serial_is_independent_of_the_tag() {
    let (mut l, _, ids) = with_assets(1);
    l.set_asset_tag(ids[0], "FL-0001").unwrap();
    l.set_asset_serial(ids[0], Some("FL-0002")).unwrap();
    assert_eq!(tag_of(&l, ids[0]).as_deref(), Some("FL-0001"));
    assert_eq!(l.next_asset_tag("FL"), "FL-0002");
}

// ------------------------------------------------- proptest: tag commands

const POOL: &[&str] = &[
    "T-0001", "t-0001", "T-0002", "T-0003", "t-0003", "X/1", "x/1", "", "a b", "--", "Ü-1",
];
const SERIALS: &[&str] = &["SN-1", "sn 2", "", " lead", "trail ", "ctl\u{1}"];

#[derive(Clone, Debug)]
enum Cmd {
    /// Tag asset (index; 3 is unknown) with POOL[i].
    Tag(usize, usize),
    /// Serial (asset index; 3 is unknown) Some(SERIALS[i]) or None.
    Serial(usize, Option<usize>),
    /// Dispose asset (index; 3 is unknown).
    Dispose(usize),
}

fn arb_cmd() -> impl Strategy<Value = Cmd> {
    prop_oneof![
        4 => (0usize..4, 0..POOL.len()).prop_map(|(a, t)| Cmd::Tag(a, t)),
        2 => (0usize..4, proptest::option::of(0..SERIALS.len()))
            .prop_map(|(a, s)| Cmd::Serial(a, s)),
        1 => (0usize..4).prop_map(Cmd::Dispose),
    ]
}

/// The model, from the spec alone.
#[derive(Default)]
struct Model {
    /// tag key → asset that holds or held it.
    ever: BTreeMap<String, usize>,
    current: [Option<String>; 3],
    serial: [Option<String>; 3],
    disposed: [bool; 3],
}

fn form_ok(t: &str) -> bool {
    (1..=32).contains(&t.len())
        && t.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'))
        && t.chars().any(|c| c.is_ascii_alphanumeric())
}

fn serial_ok(s: &str) -> bool {
    let n = s.chars().count();
    (1..=64).contains(&n)
        && !s.starts_with(' ')
        && !s.ends_with(' ')
        && !s.chars().any(char::is_control)
        && !s.chars().all(|c| c == ' ')
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// B-24 / set_asset_tag INVARIANTS 1-3 and set_asset_serial SCENARIO 1
    /// over random command sequences, against a model built from the spec.
    #[test]
    fn prop_tag_commands_keep_the_invariants(cmds in proptest::collection::vec(arb_cmd(), 1..40)) {
        let (mut l, _, ids) = with_assets(3);
        let unknown = ids[2] + 1000;
        let id_of = |i: usize| if i < 3 { ids[i] } else { unknown };
        let mut m = Model::default();
        for cmd in cmds {
            let before = l.clone();
            match cmd {
                Cmd::Tag(ai, ti) => {
                    let t = POOL[ti];
                    let r = l.set_asset_tag(id_of(ai), t);
                    if ai == 3 {
                        prop_assert!(r.is_err(), "unknown asset accepted");
                        prop_assert!(l == before);
                    } else if !form_ok(t) {
                        prop_assert!(
                            matches!(r, Err(LedgerError::InvalidAssetTag { .. })),
                            "bad form {:?}: {:?}", t, r
                        );
                        prop_assert!(l == before);
                    } else if let Some(&holder) = m.ever.get(&t.to_ascii_uppercase())
                        && holder != ai
                    {
                        prop_assert!(
                            matches!(r, Err(LedgerError::AssetTagTaken { asset, .. }) if asset == ids[holder]),
                            "taken {:?}: {:?}", t, r
                        );
                        prop_assert!(l == before);
                    } else {
                        prop_assert!(r.is_ok(), "{:?} on {}: {:?}", t, ai, r);
                        m.ever.insert(t.to_ascii_uppercase(), ai);
                        m.current[ai] = Some(t.to_string());
                    }
                }
                Cmd::Serial(ai, si) => {
                    let s = si.map(|i| SERIALS[i]);
                    let r = l.set_asset_serial(id_of(ai), s);
                    if ai == 3 {
                        prop_assert!(r.is_err());
                        prop_assert!(l == before);
                    } else if s.is_some_and(|s| !serial_ok(s)) {
                        prop_assert!(matches!(r, Err(LedgerError::InvalidSerial(_))), "{:?}", r);
                        prop_assert!(l == before);
                    } else {
                        prop_assert!(r.is_ok(), "{:?}", r);
                        m.serial[ai] = s.map(str::to_string);
                    }
                }
                Cmd::Dispose(ai) => {
                    let r = l.dispose_asset(id_of(ai), date(2026, 6, 30), usd("100.00"));
                    if ai == 3 || m.disposed[ai] {
                        prop_assert!(r.is_err());
                        prop_assert!(l == before);
                    } else {
                        prop_assert!(r.is_ok(), "{:?}", r);
                        m.disposed[ai] = true;
                    }
                }
            }
            // State agrees with the model.
            for (i, &id) in ids.iter().enumerate() {
                prop_assert_eq!(tag_of(&l, id), m.current[i].clone());
                prop_assert_eq!(serial_of(&l, id), m.serial[i].clone());
            }
            // INVARIANT 1: current tags unique ignoring case.
            let keys: Vec<String> = (0..3)
                .filter_map(|i| tag_of(&l, ids[i]).map(|t| tag_key(&t)))
                .collect();
            let mut dedup = keys.clone();
            dedup.sort();
            dedup.dedup();
            prop_assert_eq!(keys.len(), dedup.len());
            // asset_by_tag: current tag, ignoring case.
            for t in POOL {
                let want = (0..3).find(|&i| {
                    m.current[i].as_deref().map(|c| c.to_ascii_uppercase())
                        == Some(t.to_ascii_uppercase())
                });
                prop_assert_eq!(l.asset_by_tag(t).map(|a| a.id), want.map(|i| ids[i]));
            }
            // next_asset_tag: lowest T-NNNN never held.
            let want_next = (1..)
                .map(|n| format!("T-{n:04}"))
                .find(|t| !m.ever.contains_key(t))
                .unwrap();
            prop_assert_eq!(l.next_asset_tag("T"), want_next);
        }
    }
}

// ================================================= fixed asset register

/// Depreciation for asset ledger postings: Dr 6200 / Cr accumulated.
fn depreciate(l: &mut Ledger, a: &Accts, accum: AccountId, on: NaiveDate, amt: &str) {
    post(
        l,
        on,
        &[
            (a.depr_expense, Side::Debit, amt),
            (accum, Side::Credit, amt),
        ],
    );
}

/// Buy equipment for cash: Dr cost account / Cr 1000.
fn buy(l: &mut Ledger, a: &Accts, account: AccountId, on: NaiveDate, amt: &str) {
    post(
        l,
        on,
        &[(account, Side::Debit, amt), (a.cash, Side::Credit, amt)],
    );
}

fn month_end(m: u32) -> NaiveDate {
    let next = if m == 12 {
        date(2027, 1, 1)
    } else {
        date(2026, m + 1, 1)
    };
    next.pred_opt().unwrap()
}

/// Laptop: $12,000, no salvage, 12 months from 2026-01-01 → $1,000/month.
/// Server: $2,400, no salvage, 24 months from 2026-01-01 → $100/month.
/// Both in 1500/1510, bought for cash, depreciation posted Jan–Mar.
fn register_fixture() -> (Ledger, Accts, AssetId, AssetId) {
    let (mut l, a) = fixture();
    buy(&mut l, &a, a.equipment, date(2026, 1, 1), "12000.00");
    let laptop = l
        .register_asset(equipment(&a, "Laptop", "12000.00", 12, date(2026, 1, 1)))
        .unwrap();
    buy(&mut l, &a, a.equipment, date(2026, 1, 1), "2400.00");
    let server = l
        .register_asset(equipment(&a, "Server", "2400.00", 24, date(2026, 1, 1)))
        .unwrap();
    for m in 1..=3 {
        depreciate(&mut l, &a, a.accum, month_end(m), "1100.00");
    }
    (l, a, laptop, server)
}

/// INVARIANT 3: totals are the sums over held (not disposed) rows.
fn assert_totals_are_sums(r: &ledger_core::AssetRegister) {
    let held = r.rows.iter().filter(|x| x.disposed.is_none());
    let (mut c, mut acc, mut bv) = (Decimal::ZERO, Decimal::ZERO, Decimal::ZERO);
    for x in held {
        c += x.cost;
        acc += x.accumulated;
        bv += x.book_value;
    }
    assert_eq!(r.total_cost, c);
    assert_eq!(r.total_accumulated, acc);
    assert_eq!(r.total_book_value, bv);
    for x in &r.rows {
        // INVARIANT 2 (held) / INVARIANT 1 (disposed).
        if x.disposed.is_some() {
            assert_eq!(x.accumulated, Decimal::ZERO);
            assert_eq!(x.book_value, Decimal::ZERO);
        } else {
            assert_eq!(x.book_value, x.cost - x.accumulated);
        }
    }
}

#[test]
fn register_agrees_with_the_books_when_depreciation_is_posted() {
    let (mut l, _, laptop, server) = register_fixture();
    l.set_asset_tag(laptop, "FL-0001").unwrap();
    l.set_asset_serial(laptop, Some("C02XG0FDJGH5")).unwrap();
    let r = fixed_asset_register(&l, date(2026, 3, 31));
    assert_eq!(r.as_of, date(2026, 3, 31));
    assert_eq!(r.rows.len(), 2);
    // INVARIANT 1: registration order.
    assert_eq!(r.rows[0].asset_id, laptop);
    assert_eq!(r.rows[1].asset_id, server);
    let x = &r.rows[0];
    assert_eq!(x.tag.as_deref(), Some("FL-0001"));
    assert_eq!(x.serial.as_deref(), Some("C02XG0FDJGH5"));
    assert_eq!(x.description, "Laptop");
    assert_eq!(x.in_service, date(2026, 1, 1));
    assert_eq!(x.cost, d("12000.00"));
    assert_eq!(x.salvage, d("0.00"));
    assert_eq!(x.useful_life_months, 12);
    assert_eq!(x.accumulated, d("3000.00"));
    assert_eq!(x.book_value, d("9000.00"));
    assert_eq!(x.disposed, None);
    let y = &r.rows[1];
    assert_eq!(y.tag, None);
    assert_eq!(y.serial, None);
    assert_eq!(y.accumulated, d("300.00"));
    assert_eq!(y.book_value, d("2100.00"));
    assert_eq!(r.total_cost, d("14400.00"));
    assert_eq!(r.total_accumulated, d("3300.00"));
    assert_eq!(r.total_book_value, d("11100.00"));
    // INVARIANT 4: the GL, each shared account counted once.
    assert_eq!(r.ledger_cost, d("14400.00"));
    assert_eq!(r.ledger_accumulated, d("3300.00"));
    assert_eq!(r.unposted_depreciation(), Decimal::ZERO);
    assert_eq!(r.unregistered_cost(), Decimal::ZERO);
    assert_totals_are_sums(&r);
}

/// unposted_depreciation: due by schedule but not in the books.
#[test]
fn register_shows_unposted_depreciation() {
    let (l, _, _, _) = register_fixture();
    let r = fixed_asset_register(&l, date(2026, 4, 30));
    assert_eq!(r.total_accumulated, d("4400.00"));
    assert_eq!(r.ledger_accumulated, d("3300.00"));
    assert_eq!(r.unposted_depreciation(), d("1100.00"));
    assert_eq!(r.unregistered_cost(), Decimal::ZERO);
    assert_totals_are_sums(&r);
}

/// Negative unposted_depreciation when the books carry more.
#[test]
fn register_shows_overposted_depreciation_as_negative() {
    let (mut l, a, _, _) = register_fixture();
    depreciate(&mut l, &a, a.accum, date(2026, 3, 31), "50.00");
    let r = fixed_asset_register(&l, date(2026, 3, 31));
    assert_eq!(r.ledger_accumulated, d("3350.00"));
    assert_eq!(r.unposted_depreciation(), d("-50.00"));
}

/// The GL balances are on as_of: later postings don't count.
#[test]
fn register_ledger_balances_are_as_of_the_date() {
    let (l, _, _, _) = register_fixture();
    let r = fixed_asset_register(&l, date(2026, 2, 28));
    assert_eq!(r.total_accumulated, d("2200.00"));
    assert_eq!(r.ledger_accumulated, d("2200.00"));
    assert_eq!(r.ledger_cost, d("14400.00"));
    assert_eq!(r.unposted_depreciation(), Decimal::ZERO);
}

/// INVARIANT 2 mid-month: accumulated is accumulated_depreciation through
/// as_of.
#[test]
fn register_accumulated_mid_month_follows_the_schedule() {
    let (l, _, laptop, server) = register_fixture();
    let on = date(2026, 5, 17);
    let r = fixed_asset_register(&l, on);
    let la = accumulated_depreciation(l.get_asset(laptop).unwrap(), on);
    let sa = accumulated_depreciation(l.get_asset(server).unwrap(), on);
    assert_eq!(r.rows[0].accumulated, la);
    assert_eq!(r.rows[1].accumulated, sa);
    assert_eq!(r.rows[0].book_value, d("12000.00") - la);
    assert_eq!(r.total_accumulated, la + sa);
    assert_totals_are_sums(&r);
}

/// unregistered_cost: equipment in the books that no asset accounts for.
#[test]
fn register_shows_unregistered_cost() {
    let (mut l, a, _, _) = register_fixture();
    buy(&mut l, &a, a.equipment, date(2026, 2, 10), "500.00");
    let r = fixed_asset_register(&l, date(2026, 3, 31));
    assert_eq!(r.ledger_cost, d("14900.00"));
    assert_eq!(r.total_cost, d("14400.00"));
    assert_eq!(r.unregistered_cost(), d("500.00"));
    // Registered cost not in the books goes negative.
    let (mut l2, a2) = fixture();
    l2.register_asset(equipment(&a2, "Unpaid", "800.00", 8, date(2026, 1, 1)))
        .unwrap();
    let r2 = fixed_asset_register(&l2, date(2026, 1, 31));
    assert_eq!(r2.ledger_cost, Decimal::ZERO);
    assert_eq!(r2.unregistered_cost(), d("-800.00"));
    assert_eq!(r2.unposted_depreciation(), d("100.00"));
}

/// INVARIANT 1: only assets registered (placed in service) by as_of.
#[test]
fn register_leaves_out_assets_not_yet_in_service() {
    let (mut l, a, laptop, server) = register_fixture();
    buy(&mut l, &a, a.equipment, date(2026, 6, 1), "600.00");
    let chair = l
        .register_asset(equipment(&a, "Chair", "600.00", 6, date(2026, 6, 1)))
        .unwrap();
    let r = fixed_asset_register(&l, date(2026, 5, 31));
    let ids: Vec<AssetId> = r.rows.iter().map(|x| x.asset_id).collect();
    assert_eq!(ids, vec![laptop, server]);
    assert_eq!(r.ledger_cost, d("14400.00"));
    let r = fixed_asset_register(&l, date(2026, 6, 1));
    let ids: Vec<AssetId> = r.rows.iter().map(|x| x.asset_id).collect();
    assert_eq!(ids, vec![laptop, server, chair]);
    assert_eq!(
        r.rows[2].accumulated,
        accumulated_depreciation(l.get_asset(chair).unwrap(), date(2026, 6, 1))
    );
    // Before anything: empty and zero.
    let r = fixed_asset_register(&l, date(2025, 12, 31));
    assert!(r.rows.is_empty());
    assert_eq!(r.total_cost, Decimal::ZERO);
    assert_eq!(r.total_accumulated, Decimal::ZERO);
    assert_eq!(r.total_book_value, Decimal::ZERO);
}

/// Disposal: Server sold 2026-06-30 for $2,000 after six months ($600)
/// depreciation: book value 1,800, gain 200. Entry: Dr 1000 2,000,
/// Dr 1510 600, Cr 1500 2,400, Cr 4900 200.
fn dispose_server(l: &mut Ledger, a: &Accts, server: AssetId) {
    for m in 4..=6 {
        depreciate(l, a, a.accum, month_end(m), "1100.00");
    }
    l.dispose_asset(server, date(2026, 6, 30), usd("2000.00"))
        .unwrap();
    post(
        l,
        date(2026, 6, 30),
        &[
            (a.cash, Side::Debit, "2000.00"),
            (a.accum, Side::Debit, "600.00"),
            (a.equipment, Side::Credit, "2400.00"),
            (a.gain, Side::Credit, "200.00"),
        ],
    );
}

#[test]
fn register_shows_a_disposed_asset_outside_the_totals() {
    let (mut l, a, laptop, server) = register_fixture();
    l.set_asset_tag(server, "FL-0002").unwrap();
    dispose_server(&mut l, &a, server);
    let r = fixed_asset_register(&l, date(2026, 6, 30));
    assert_eq!(r.rows.len(), 2, "a disposed asset still has a row");
    let s = r.rows.iter().find(|x| x.asset_id == server).unwrap();
    assert_eq!(s.disposed, Some((date(2026, 6, 30), d("2000.00"))));
    assert_eq!(s.accumulated, Decimal::ZERO);
    assert_eq!(s.book_value, Decimal::ZERO);
    assert_eq!(s.tag.as_deref(), Some("FL-0002"));
    assert_eq!(s.cost, d("2400.00"));
    let lp = r.rows.iter().find(|x| x.asset_id == laptop).unwrap();
    assert_eq!(lp.accumulated, d("6000.00"));
    assert_eq!(r.total_cost, d("12000.00"));
    assert_eq!(r.total_accumulated, d("6000.00"));
    assert_eq!(r.total_book_value, d("6000.00"));
    assert_eq!(r.ledger_cost, d("12000.00"));
    assert_eq!(r.ledger_accumulated, d("6000.00"));
    assert_eq!(r.unposted_depreciation(), Decimal::ZERO);
    assert_eq!(r.unregistered_cost(), Decimal::ZERO);
    assert_totals_are_sums(&r);
}

/// Before its disposal date, a later-disposed asset is still held.
#[test]
fn register_before_the_disposal_date_shows_the_asset_held() {
    let (mut l, a, _, server) = register_fixture();
    dispose_server(&mut l, &a, server);
    let r = fixed_asset_register(&l, date(2026, 5, 31));
    let s = r.rows.iter().find(|x| x.asset_id == server).unwrap();
    assert_eq!(s.disposed, None);
    assert_eq!(s.accumulated, d("500.00"));
    assert_eq!(s.book_value, d("1900.00"));
    assert_eq!(r.total_cost, d("14400.00"));
    assert_eq!(r.total_accumulated, d("5500.00"));
    assert_eq!(r.ledger_cost, d("14400.00"));
    assert_eq!(r.ledger_accumulated, d("5500.00"));
    assert_eq!(r.unposted_depreciation(), Decimal::ZERO);
    assert_totals_are_sums(&r);
}

/// INVARIANT 4 with separate account pairs: each pair's GL balance counts.
#[test]
fn register_sums_the_gl_over_distinct_asset_accounts() {
    let (mut l, a, _, _) = register_fixture();
    buy(&mut l, &a, a.vehicles, date(2026, 1, 1), "36000.00");
    l.register_asset(NewAsset {
        description: "Van".to_string(),
        purchase_date: date(2026, 1, 1),
        cost: usd("36000.00"),
        salvage_value: usd("6000.00"),
        useful_life_months: 60,
        cost_account: a.vehicles,
        accum_depr_account: a.vehicles_accum,
        expense_account: a.depr_expense,
    })
    .unwrap();
    // (36,000 − 6,000) / 60 = 500/month; Jan–Mar posted.
    for m in 1..=3 {
        depreciate(&mut l, &a, a.vehicles_accum, month_end(m), "500.00");
    }
    let r = fixed_asset_register(&l, date(2026, 3, 31));
    assert_eq!(r.rows[2].salvage, d("6000.00"));
    assert_eq!(r.rows[2].accumulated, d("1500.00"));
    assert_eq!(r.rows[2].book_value, d("34500.00"));
    assert_eq!(r.total_cost, d("50400.00"));
    assert_eq!(r.total_accumulated, d("4800.00"));
    assert_eq!(r.total_book_value, d("45600.00"));
    assert_eq!(r.ledger_cost, d("50400.00"));
    assert_eq!(r.ledger_accumulated, d("4800.00"));
    assert_eq!(r.unposted_depreciation(), Decimal::ZERO);
    assert_eq!(r.unregistered_cost(), Decimal::ZERO);
    assert_totals_are_sums(&r);
}

/// Accounts no registered asset uses don't count in the GL figures.
#[test]
fn register_ignores_accounts_no_asset_uses() {
    let (mut l, a, _, _) = register_fixture();
    // Vehicles bought but never registered under 1520: not one of the
    // registered assets' accounts.
    buy(&mut l, &a, a.vehicles, date(2026, 1, 1), "9000.00");
    let r = fixed_asset_register(&l, date(2026, 3, 31));
    assert_eq!(r.ledger_cost, d("14400.00"));
    assert_eq!(r.unregistered_cost(), Decimal::ZERO);
}

/// An empty ledger has an empty register that agrees with the books.
#[test]
fn register_of_no_assets_is_empty() {
    let (l, _) = fixture();
    let r = fixed_asset_register(&l, date(2026, 12, 31));
    assert!(r.rows.is_empty());
    assert_eq!(r.ledger_cost, Decimal::ZERO);
    assert_eq!(r.ledger_accumulated, Decimal::ZERO);
    assert_eq!(r.unposted_depreciation(), Decimal::ZERO);
    assert_eq!(r.unregistered_cost(), Decimal::ZERO);
}
