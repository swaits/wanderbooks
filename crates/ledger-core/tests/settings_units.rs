//! The settings registry's small parts, value by value: each accessor, each
//! check boundary, each tax classification, each masked or joined display.
//! `settings.rs`'s own tests cover the invariants; these pin the exact
//! outputs the server and viewer rely on (P1 mutation testing,
//! 2026-10-17).

use chrono::{Datelike, NaiveDate};
use ledger_core::settings::{
    Change, Company, Member, PartyPolicy, Settings, Source, check, display,
    form_for_classification, holds_printed_tin, key, line_text, long_text, parse_month_day,
    parse_share, shape, today_in,
};
use ledger_core::{AccountType, Currency, Ledger, PartyField, Role, TaxForm};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use std::str::FromStr;

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

fn src() -> Source {
    Source {
        decided_by: "Owner".into(),
        decided_on: date(2026, 10, 17),
        citation: None,
        note: None,
    }
}

/// Settings holding `pairs`, checked then applied as one change.
fn settings(pairs: &[(&str, Value)]) -> Settings {
    let changes: Vec<Change> = pairs
        .iter()
        .map(|(k, v)| Change {
            key: (*k).into(),
            value: Some(v.clone()),
        })
        .collect();
    let mut s = Settings::default();
    let checked = s.check_changes(&changes, &src()).unwrap();
    s.apply(&checked, &src(), 1).unwrap();
    s
}

fn checked(name: &str, v: Value) -> Result<Value, String> {
    check(key(name).unwrap(), &v)
}

#[test]
fn accessors_return_what_was_set() {
    let s = settings(&[
        ("policy.evidence.threshold", json!("0.00")),
        ("policy.evidence.required_accounts", json!(["1000", "2050"])),
        ("policy.evidence.exempt_accounts", json!(["6900"])),
        ("books.cash_accounts", json!(["1000", "1010"])),
        ("books.financing_accounts", json!(["2500"])),
        ("books.investing_accounts", json!(["1500", "1510"])),
        ("books.asset_tag_prefix", json!("FL")),
        ("policy.parties", json!("off")),
        ("books.close_sweep", json!(["3100"])),
    ]);
    assert_eq!(s.receipt_threshold(), Some("0.00"));
    assert_eq!(s.receipt_required_accounts(), ["1000", "2050"]);
    assert_eq!(s.receipt_exempt_accounts(), ["6900"]);
    assert_eq!(s.cash_accounts(), ["1000", "1010"]);
    assert_eq!(s.financing_accounts(), ["2500"]);
    assert_eq!(s.investing_accounts(), ["1500", "1510"]);
    assert_eq!(s.asset_tag_prefix(), Some("FL"));
    assert_eq!(s.parties(), PartyPolicy::Off);
    assert_eq!(s.company("USD").close_sweep, ["3100"]);
    let s = settings(&[("policy.parties", json!("require"))]);
    assert_eq!(s.parties(), PartyPolicy::Require);
    // Unset: nothing, and the default policy.
    let s = Settings::default();
    assert_eq!(s.receipt_threshold(), None);
    assert!(s.cash_accounts().is_empty());
    assert_eq!(s.asset_tag_prefix(), None);
    assert_eq!(s.parties(), PartyPolicy::Suggest);
}

#[test]
fn the_company_profile_is_its_company_keys_name_first() {
    let s = settings(&[
        ("company.ein", json!("12-3456789")),
        ("company.name", json!("Acme LLC")),
        ("company.duns", json!("149976972")),
        (
            "company.members",
            json!([
                {"name": "Mo", "share": "50%", "capital_account": "3001"},
                {"name": "Tobin", "share": "50%", "capital_account": "3000"},
            ]),
        ),
        ("books.accounting_basis", json!("cash")),
        ("books.asset_tag_prefix", json!("FL")),
    ]);
    let c = s.company("USD");
    let profile: Vec<(&str, &str)> = c
        .profile
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    assert_eq!(
        profile,
        [
            ("name", "Acme LLC"),
            ("base_currency", "USD"),
            ("ein", "12-3456789"),
            ("duns", "149976972"),
        ]
    );
    assert_eq!(c.name, "Acme LLC");
    assert_eq!(c.basis, "Cash");
    assert_eq!(c.members.len(), 2);
}

#[test]
fn party_policy_spellings() {
    assert_eq!(PartyPolicy::Off.key(), "off");
    assert_eq!(PartyPolicy::Suggest.key(), "suggest");
    assert_eq!(PartyPolicy::Require.key(), "require");
    assert_eq!(PartyPolicy::parse("off"), Some(PartyPolicy::Off));
    assert_eq!(PartyPolicy::parse("suggest"), Some(PartyPolicy::Suggest));
    assert_eq!(PartyPolicy::parse("require"), Some(PartyPolicy::Require));
    assert_eq!(PartyPolicy::parse("required"), Some(PartyPolicy::Require));
    assert_eq!(PartyPolicy::parse("Require"), None);
    assert_eq!(PartyPolicy::parse(""), None);
}

#[test]
fn fiscal_year_bounds() {
    let c = Company {
        fiscal_year_start: (7, 1),
        ..Company::default()
    };
    assert_eq!(
        c.fiscal_year_containing(date(2026, 3, 15)),
        (date(2025, 7, 1), date(2026, 6, 30))
    );
    assert_eq!(
        c.fiscal_year_containing(date(2026, 6, 30)),
        (date(2025, 7, 1), date(2026, 6, 30))
    );
    assert_eq!(
        c.fiscal_year_containing(date(2026, 7, 1)),
        (date(2026, 7, 1), date(2027, 6, 30))
    );
    let calendar = Company::default();
    assert_eq!(
        calendar.fiscal_year_containing(date(2024, 2, 29)),
        (date(2024, 1, 1), date(2024, 12, 31))
    );
}

/// A member's party is the live one of their name, else the most recent
/// archived one of their name: never another archived member's.
#[test]
fn party_members_find_each_members_own_party() {
    let mut l = Ledger::new(Currency::USD);
    let capital = l
        .add_account("3001", "Capital — Mo", AccountType::Equity, None, false)
        .unwrap();
    l.add_account(
        "3000",
        "Capital — Tobin",
        AccountType::Equity,
        None,
        false,
    )
    .unwrap();
    let mo = l.add_party("Mo", &[Role::Member]).unwrap();
    let other = l.add_party("Pat", &[Role::Member]).unwrap();
    l.update_party(mo, PartyField::Archived(true)).unwrap();
    l.update_party(other, PartyField::Archived(true)).unwrap();
    let c = Company {
        members: vec![Member {
            name: "mo ".into(),
            share: Decimal::ONE,
            capital_account: "3001".into(),
            distribution_account: None,
            files_jointly_with: None,
        }],
        ..Company::default()
    };
    let got = c.party_members(&l);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].party, mo);
    assert_eq!(got[0].capital, capital);
    assert_eq!(got[0].share, Decimal::ONE);
    assert_eq!(got[0].distributions, None);
}

#[test]
fn printed_tin_joiners() {
    // Three spaces is not a joiner; a 3-6 split is no TIN's shape; two
    // dashes is not one joiner.
    assert!(!holds_printed_tin("123   45 6789"));
    assert!(!holds_printed_tin("123 456789"));
    assert!(!holds_printed_tin("123--45--6789"));
    // Two spaces, and a dash with a space each side, are.
    assert!(holds_printed_tin("123  45  6789"));
    assert!(holds_printed_tin("123 - 45 - 6789"));
    assert!(holds_printed_tin("12 3456789"));
}

#[test]
fn text_length_boundaries() {
    assert_eq!(line_text("x", "aaaaa", 5).unwrap(), "aaaaa");
    assert_eq!(
        line_text("x", "aaaaaa", 5).unwrap_err(),
        "x is longer than 5 characters"
    );
    assert_eq!(long_text("x", "aaaaa", 5).unwrap(), "aaaaa");
    assert_eq!(
        long_text("x", "aaaaaa", 5).unwrap_err(),
        "x is longer than 5 characters"
    );
    assert_eq!(long_text("x", "a\nb\tc", 10).unwrap(), "a\nb\tc");
    assert_eq!(
        long_text("x", "a\u{7}b", 10).unwrap_err(),
        "x holds a control character"
    );
}

#[test]
fn share_boundaries() {
    assert_eq!(parse_share("K", "1").unwrap(), Decimal::ONE);
    assert_eq!(parse_share("K", "100%").unwrap(), Decimal::ONE);
    assert_eq!(parse_share("K", "0.5").unwrap(), dec("0.5"));
    assert_eq!(parse_share("K", "50%").unwrap(), dec("0.5"));
    assert_eq!(parse_share("K", "0.0001").unwrap(), dec("0.0001"));
    for bad in ["0", "0%", "-0.5", "1.0001", "101%", "2"] {
        assert_eq!(
            parse_share("K", bad).unwrap_err(),
            format!("member K: share {bad:?} must be like \"50%\" or \"0.5\""),
        );
    }
}

#[test]
fn month_day_digits_only() {
    assert_eq!(parse_month_day("07-01").unwrap(), (7, 1));
    for bad in ["+1-01", "01-+1", "-01", "01-"] {
        assert_eq!(
            parse_month_day(bad).unwrap_err(),
            format!("fiscal_year_start {bad:?}: expected \"MM-DD\", e.g. \"01-01\""),
        );
    }
}

#[test]
fn every_classification_names_its_return() {
    use TaxForm::*;
    for (c, want) in [
        ("partnership", Some(Form1065)),
        ("General Partnership", Some(Form1065)),
        ("S corporation", Some(Form1120S)),
        ("S-corp", Some(Form1120S)),
        ("s corp", Some(Form1120S)),
        ("scorp", Some(Form1120S)),
        ("C corporation", Some(Form1120)),
        ("ccorp", Some(Form1120)),
        ("corporation", Some(Form1120)),
        ("corp", Some(Form1120)),
        ("association", Some(Form1120)),
        ("disregarded entity", Some(ScheduleC)),
        ("sole proprietor", Some(ScheduleC)),
        ("single-member LLC", Some(ScheduleC)),
        ("member single", None),
        ("corporation or S corporation", None),
        ("partnership or disregarded", None),
        ("LLC", None),
        ("", None),
    ] {
        assert_eq!(form_for_classification(c), want, "{c:?}");
    }
}

#[test]
fn codes_and_choices_keep_their_spelling() {
    assert_eq!(
        checked("books.close_into", json!(" 3000 ")),
        Ok(json!("3000"))
    );
    assert_eq!(
        checked("books.cash_accounts", json!([" 1000", "1010 "])),
        Ok(json!(["1000", "1010"]))
    );
    for (k, v) in [
        ("books.accounting_basis", "accrual"),
        ("books.accounting_basis", "cash"),
        ("policy.parties", "off"),
        ("policy.parties", "suggest"),
        ("policy.parties", "require"),
        ("policy.approval_rules", "off"),
        ("policy.approval_rules", "enforce"),
    ] {
        assert_eq!(checked(k, json!(v)), Ok(json!(v)), "{k} {v}");
    }
}

#[test]
fn amount_bound_is_inclusive() {
    let max = ledger_core::kernel::MAX_LINE_AMOUNT.to_string();
    assert_eq!(
        checked("policy.evidence.threshold", json!(max)),
        Ok(json!(max))
    );
    let over = format!("{max}.01");
    assert_eq!(
        checked("policy.evidence.threshold", json!(over)),
        Err(format!(
            "policy.evidence.threshold {over:?} is out of range"
        ))
    );
}

#[test]
fn ein_shapes() {
    let ein = |s: &str| checked("company.ein", json!(s));
    assert_eq!(ein("12-3456789"), Ok(json!("12-3456789")));
    assert_eq!(ein("123456789"), Ok(json!("12-3456789")));
    let refused = Err("company.ein: expected an EIN, NN-NNNNNNN".to_string());
    for bad in [
        "12345678",
        "12-34-567",
        "12345678x",
        "123-456789",
        "1-23456789",
    ] {
        assert_eq!(ein(bad), refused, "{bad:?}");
    }
}

#[test]
fn name_control_and_tag_prefix_bounds() {
    let nc = |s: &str| checked("company.irs_name_control", json!(s));
    for ok in ["WAND", "A&B-", "A&", "A-B", "w"] {
        assert_eq!(nc(ok), Ok(json!(ok.to_uppercase())), "{ok:?}");
    }
    assert_eq!(
        nc("WANDE"),
        Err("company.irs_name_control \"WANDE\": one to four letters, digits, & or -".into())
    );
    assert!(nc("W.A").is_err());
    let tag = |s: &str| checked("books.asset_tag_prefix", json!(s));
    assert_eq!(tag("FL"), Ok(json!("FL")));
    assert_eq!(tag("ABCDEFGH"), Ok(json!("ABCDEFGH")));
    assert_eq!(
        tag("ABCDEFGHI"),
        Err("books.asset_tag_prefix \"ABCDEFGHI\": 1 to 8 letters or digits".into())
    );
    assert!(tag("").is_err());
    assert!(tag("W-L").is_err());
}

/// Replay checks an amount and a fiscal year start by their shape.
#[test]
fn replay_shapes_amounts_and_month_starts() {
    let threshold = key("policy.evidence.threshold").unwrap();
    assert!(shape(threshold, &json!("25.00")).is_ok());
    assert_eq!(
        shape(threshold, &json!("abc")),
        Err("policy.evidence.threshold \"abc\" is not a decimal".into())
    );
    let fy = key("company.fiscal_year_start").unwrap();
    assert!(shape(fy, &json!("07-15")).is_ok());
    for bad in ["13-01", "02-30", "07"] {
        assert_eq!(
            shape(fy, &json!(bad)),
            Err(format!("fiscal_year_start {bad:?}: expected \"MM-DD\"")),
        );
    }
}

#[test]
fn today_is_the_companys_today() {
    let utc = chrono::Utc::now().date_naive();
    let near = |d: NaiveDate| (d - utc).num_days().abs() <= 1;
    let s = settings(&[("company.timezone", json!("America/Denver"))]);
    assert!(near(s.today()), "{}", s.today());
    assert!(near(today_in(None)));
    // UTC+14 and UTC-11 are 25 hours apart: never the same day.
    let east = today_in(Some("Pacific/Kiritimati"));
    let west = today_in(Some("Pacific/Pago_Pago"));
    assert!(near(east) && near(west));
    assert!(east > west, "{east} {west}");
    assert!(east.year() >= 2026);
}

#[test]
fn display_joins_members_and_lists() {
    let members = key("company.members").unwrap();
    let v = json!([
        {"name": "Mo", "share": "0.5", "capital_account": "3001"},
        {"name": "Tobin", "share": "0.5", "capital_account": "3000"},
    ]);
    assert_eq!(display(members, &v, false), "Mo 0.5, Tobin 0.5");
    let cash = key("books.cash_accounts").unwrap();
    assert_eq!(display(cash, &json!(["1000", "1010"]), false), "1000, 1010");
    let ein = key("company.ein").unwrap();
    assert_eq!(display(ein, &json!("12-3456789"), false), "**-***6789");
    assert_eq!(display(ein, &json!("12-3456789"), true), "12-3456789");
}

/// A date is taken only as written YYYY-MM-DD, one spelling per day.
#[test]
fn dates_take_one_spelling() {
    let formed = |s: &str| checked("company.formed", json!(s));
    assert_eq!(formed("2026-10-04"), Ok(json!("2026-10-04")));
    assert_eq!(formed(" 2026-10-04 "), Ok(json!("2026-10-04")));
    for bad in ["2026-1-4", "2026-10-4", "+2026-10-04"] {
        assert_eq!(
            formed(bad),
            Err(format!(
                "company.formed {bad:?}: expected a date, YYYY-MM-DD"
            )),
        );
    }
}
