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
    l.add_account("3000", "Capital — Tobin", AccountType::Equity, None, false)
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

// ------------------------------------------------ P4b S4.2: the new kinds

/// `Kind::Count`: a positive whole number, in one canonical JSON number
/// (a settings file writes it as digits; the journal keeps the number).
#[test]
fn counts_are_positive_whole_numbers() {
    let days = |v: Value| checked("invoicing.unsent_days", v);
    assert_eq!(days(json!(2)), Ok(json!(2)));
    assert_eq!(days(json!("14")), Ok(json!(14)));
    assert_eq!(
        checked("invoicing.first_number", json!(1000)),
        Ok(json!(1000))
    );
    for bad in [
        json!(0),
        json!(-1),
        json!("0"),
        json!("-2"),
        json!(1.5),
        json!("2.0"),
        json!("x"),
        json!(true),
        json!(null),
    ] {
        assert!(days(bad.clone()).is_err(), "{bad}");
    }
    // Every day-count key takes the same kind.
    for k in [
        "invoicing.first_number",
        "invoicing.unsent_days",
        "invoicing.reminder_days",
        "bills.remind_days",
        "bills.discount_warn_days",
    ] {
        assert_eq!(checked(k, json!(7)), Ok(json!(7)), "{k}");
        assert!(checked(k, json!(0)).is_err(), "{k}");
    }
}

/// `Kind::Terms`: canonicalized through [`ledger_core::Terms`], so two
/// spellings of one agreement are one value (rules.md B-39).
#[test]
fn terms_keep_one_canonical_spelling() {
    let terms = |s: &str| checked("invoicing.default_terms", json!(s));
    assert_eq!(terms("net 30"), Ok(json!("Net 30")));
    assert_eq!(terms(" NET 15 EOM "), Ok(json!("Net 15 EOM")));
    assert_eq!(terms("2/10 net 30"), Ok(json!("2/10 net 30")));
    assert_eq!(terms("Due On Receipt"), Ok(json!("Due on receipt")));
    assert_eq!(terms("due 2026-11-15"), Ok(json!("Due 2026-11-15")));
    assert_eq!(terms("2.5/10 net 30"), Ok(json!("2.5/10 net 30")));
    for bad in [
        "net",
        "Net 4000",
        "Net -5",
        "2/40 net 30",
        "0/10 net 30",
        "2/10 net 15 eom",
        "2/10 net 30 extra",
        "Due 9999-13-01",
        "Due 2026-1-5",
        "",
    ] {
        assert!(terms(bad).is_err(), "{bad:?}");
    }
}

/// `Kind::PaymentMethods` (A-3): a routing number is nine digits, so an
/// `ach` or `wire` method's bank-detail lines are the one text exempt from
/// the full-TIN check; every other text in the list is scanned. At most
/// eight methods, each line at most 120 characters.
#[test]
fn payment_methods_scan_every_text_but_ach_and_wire_lines() {
    let methods = |v: Value| checked("invoicing.payment_methods", v);
    for kind in ["ach", "wire"] {
        let got = methods(json!([{
            "kind": kind,
            "label": "Bank transfer",
            "lines": ["Routing 123456789", "Account 000123456789"],
        }]))
        .unwrap();
        assert_eq!(got[0]["lines"][0], json!("Routing 123456789"), "{kind}");
        assert_eq!(got[0]["lines"][1], json!("Account 000123456789"), "{kind}");
    }
    for bad in [
        json!([{"kind": "link", "label": "123456789", "lines": []}]),
        json!([{"kind": "link", "label": "Pay here", "lines": ["123456789"]}]),
        json!([{"kind": "check", "label": "Check", "lines": ["pay to 123456789"]}]),
        json!([{"kind": "other", "label": "Cash", "lines": [], "link": "123456789"}]),
    ] {
        assert!(methods(bad.clone()).is_err(), "{bad}");
    }
    // Canonical: the kind's spelling and the texts are trimmed.
    assert_eq!(
        methods(json!([{"kind": "ACH", "label": " Bank ", "lines": [" 123456789 "], "qr": true}]))
            .unwrap(),
        json!([{"kind": "ach", "label": "Bank", "lines": ["123456789"], "qr": true}])
    );
    // Unknown kind and unknown field are refused.
    assert!(methods(json!([{"kind": "barter", "label": "X", "lines": []}])).is_err());
    assert!(
        methods(json!([{"kind": "other", "label": "X", "lines": [], "note": "y"}])).is_err()
    );
    // At most eight methods.
    let pool = |n: usize| {
        Value::Array(
            (0..n)
                .map(|i| json!({"kind": "other", "label": format!("Method {i}"), "lines": []}))
                .collect(),
        )
    };
    assert!(methods(pool(8)).is_ok());
    assert!(methods(pool(9)).is_err());
    // Each line at most 120 characters.
    let long = "x".repeat(121);
    assert!(
        methods(json!([{"kind": "other", "label": "M", "lines": [long]}])).is_err(),
        "121 characters"
    );
}

/// `invoicing.number_prefix` is 1 to 12 visible ASCII characters: a number
/// with a space or a control character in it reads wrong, and an empty
/// prefix is no prefix.
#[test]
fn number_prefix_is_short_visible_ascii() {
    let prefix = |s: &str| checked("invoicing.number_prefix", json!(s));
    for ok in ["INV-", "A", "2026/", "ABCDEFGHIJKL"] {
        assert_eq!(prefix(ok), Ok(json!(ok)), "{ok:?}");
    }
    assert_eq!(prefix("  INV-  "), Ok(json!("INV-")));
    for bad in ["", "   ", "ABCDEFGHIJKLM", "INV 1", "INV\t1", "Ünï"] {
        assert!(prefix(bad).is_err(), "{bad:?}");
    }
}

/// The company's contact details are simple company keys, so the seller
/// snapshot reads them through `Company::profile`.
#[test]
fn contact_details_are_in_the_company_profile() {
    let s = settings(&[
        ("company.name", json!("Acme LLC")),
        ("company.email", json!("billing@acme.example")),
        ("company.website", json!("https://acme.example")),
    ]);
    let profile: Vec<(String, String)> = s.company("USD").profile;
    assert!(
        profile.contains(&("email".into(), "billing@acme.example".into())),
        "{profile:?}"
    );
    assert!(
        profile.contains(&("website".into(), "https://acme.example".into())),
        "{profile:?}"
    );
}

/// Replay checks the new kinds by shape, never by a later version's
/// bounds: a count is a positive whole number, terms any string, and a
/// method list of objects with their fields (rules.md B-28).
#[test]
fn replay_shapes_the_new_kinds() {
    let count = key("invoicing.unsent_days").unwrap();
    assert!(shape(count, &json!(2)).is_ok());
    assert!(shape(count, &json!("2")).is_ok());
    assert!(shape(count, &json!(0)).is_err());
    let terms = key("invoicing.default_terms").unwrap();
    assert!(shape(terms, &json!("Net 30")).is_ok());
    assert!(shape(terms, &json!("a spelling this version doesn't know")).is_ok());
    assert!(shape(terms, &json!(30)).is_err());
    let methods = key("invoicing.payment_methods").unwrap();
    assert!(shape(methods, &json!([])).is_ok());
    assert!(
        shape(
            methods,
            &json!([{"kind": "ach", "label": "Bank", "lines": ["123456789"]}])
        )
        .is_ok()
    );
    assert!(shape(methods, &json!([{"kind": "ach", "label": "Bank"}])).is_err());
}

