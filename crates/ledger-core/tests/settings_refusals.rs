//! The settings registry refuses what its kinds refuse, and replay takes
//! what it was given (plan P14 unit V-D2; coverage found the validators of
//! `settings.rs` among its uncovered lines: members, payment methods, the
//! attestation bar, terms, the changes of a whole command).
//!
//! ```text
//! GOAL: a setting is checked by its key's kind and kept in one spelling
//!       before it is journaled; a change is all or nothing; old books
//!       whose values a later version would refuse still open.
//! SOURCE: WB-ledger-5 (settings are kept in the journal); WB-ledger-6 (no
//!         full TIN in a setting); WB-ledger-8 (a key this version
//!         doesn't know is a newer version's); BS-entry-4 (replay takes
//!         any value of the right shape); settings.rs INVARIANTS 1 to 8.
//! ```
//!
//! Each test asks the public check (`settings::check`, `Settings::
//! check_changes`, `Ledger::check_settings`) and, for a refused command,
//! compares the settings to what they were: a refusal changes nothing.

use chrono::NaiveDate;
use ledger_core::settings::{
    ApprovalMode, Change, Company, Member, PartyPolicy, Settings, Source, TaxCategoryPolicy, check,
    display, key, shape, today_in,
};
use ledger_core::{AccountType, Currency, Ledger, PartyField, Role, ScheduleConvention};
use rust_decimal::Decimal;
use serde_json::{Value, json};

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn src() -> Source {
    Source {
        decided_by: "Owner".into(),
        decided_on: d(2026, 11, 9),
        citation: None,
        note: None,
    }
}

/// A folder the host calls rooted. `backup.folder` is a folder on the
/// server's own computer, so it is checked by the host's path rules: on
/// Windows `/mnt/backup/books` names no disk and is refused (the
/// portable-windows gate, 2026-11-09), so the test names one the host
/// takes.
const BACKUP_FOLDER: &str = if cfg!(windows) {
    r"D:\backup\books"
} else {
    "/mnt/backup/books"
};

fn change(k: &str, v: Value) -> Change {
    Change {
        key: k.into(),
        value: Some(v),
    }
}

fn checked(name: &str, v: Value) -> Result<Value, String> {
    check(key(name).unwrap_or_else(|| panic!("no key {name}")), &v)
}

fn refused(name: &str, v: Value) -> String {
    checked(name, v.clone()).expect_err(&format!("{name} = {v} should be refused"))
}

/// Settings holding `pairs`, checked then applied as one change.
fn settings(pairs: &[(&str, Value)]) -> Settings {
    let changes: Vec<Change> = pairs.iter().map(|(k, v)| change(k, v.clone())).collect();
    let mut s = Settings::default();
    let made = s.check_changes(&changes, &src()).unwrap();
    s.apply(&made, &src(), 1).unwrap();
    s
}

fn member(name: &str, share: &str, capital: &str) -> Value {
    json!({"name": name, "share": share, "capital_account": capital})
}

// ---------------------------------------------------------------- members

/// INVARIANT 1, `company.members`: each member has a name, a share and a
/// capital account; the shares sum to exactly 100%; nobody is listed twice
/// or shares a capital account; a distributions account is not another
/// member's capital account; a joint filer names a member who names them
/// back. Every refusal names the member.
#[test]
fn members_are_refused_unless_they_hold_together() {
    let members = |v: Vec<Value>| refused("company.members", Value::Array(v));
    let ok = [member("Mo", "50%", "3001"), member("Tobin", "0.5", "3002")];

    // Shape.
    assert!(
        members(vec![json!("Mo")]).contains("member 1: expected {name, share, capital_account}")
    );
    assert!(
        members(vec![
            json!({"name": "Mo", "share": "1", "capital_account": "3001", "age": 3})
        ])
        .contains("member 1: unknown field \"age\"")
    );
    assert!(
        members(vec![
            json!({"name": 7, "share": "1", "capital_account": "3001"})
        ])
        .contains("member 1 name: expected a string")
    );
    assert!(
        members(vec![
            json!({"name": "  ", "share": "1", "capital_account": "3001"})
        ])
        .contains("member 1: name is blank")
    );
    assert!(
        members(vec![json!({"name": "Mo", "capital_account": "3001"})])
            .contains("member Mo: share is required")
    );
    assert!(
        members(vec![json!({"name": "Mo", "share": "1"})])
            .contains("member Mo: capital_account is required")
    );
    assert!(members(vec![member("Mo", "lots", "3001")]).contains("member Mo: share"));
    assert!(members(vec![member("Mo", "150%", "3001")]).contains("member Mo: share"));
    assert!(members(vec![member("Mo", "0", "3001")]).contains("member Mo: share"));

    // Together.
    assert!(
        members(vec![member("Mo", "1", "3001"), member("Mo", "1", "3002")])
            .contains("member Mo is listed twice")
    );
    assert!(
        members(vec![
            member("Mo", "50%", "3001"),
            member("Tobin", "50%", "3001")
        ])
        .contains("members Mo and Tobin share capital account 3001")
    );
    let short = members(vec![
        member("Mo", "50%", "3001"),
        member("Tobin", "40%", "3002"),
    ]);
    assert_eq!(short, "member shares sum to 0.90, not 100%");
    let mut crossed = ok.clone();
    crossed[0]["distribution_account"] = json!("3002");
    assert!(
        members(crossed.to_vec())
            .contains("Mo's distributions account 3002 is Tobin's capital account")
    );

    // Joint filers.
    let mut selfish = ok.clone();
    selfish[0]["files_jointly_with"] = json!("Mo");
    assert!(members(selfish.to_vec()).contains("files_jointly_with names themself"));
    let mut stranger = ok.clone();
    stranger[0]["files_jointly_with"] = json!("Pat");
    assert!(members(stranger.to_vec()).contains("files_jointly_with \"Pat\" isn't a member"));
    let mut one_sided = ok.clone();
    one_sided[0]["files_jointly_with"] = json!("Tobin");
    assert!(members(one_sided.to_vec()).contains("set it on both"));

    // And both sides saying so, with a distributions account, is kept in a
    // canonical spelling.
    let mut both = ok.clone();
    both[0]["files_jointly_with"] = json!("Tobin");
    both[1]["files_jointly_with"] = json!("Mo");
    both[0]["distribution_account"] = json!("3901");
    let kept = checked("company.members", Value::Array(both.to_vec())).unwrap();
    assert_eq!(kept[0]["share"], json!("0.5"));
    assert_eq!(kept[0]["distribution_account"], json!("3901"));
    assert_eq!(kept[0]["files_jointly_with"], json!("Tobin"));
    assert_eq!(kept[1]["files_jointly_with"], json!("Mo"));
}

/// Members as party shares see them: a member whose name matches no party
/// with the member role, or whose capital account the chart lacks, is left
/// out; a distributions account the chart lacks is none; the live party of
/// a name wins over an archived one.
#[test]
fn party_members_leave_out_who_cannot_be_derived() {
    let mut l = Ledger::new(Currency::USD);
    let cap_mo = l
        .add_account("3001", "Capital Mo", AccountType::Equity, None, false)
        .unwrap();
    let cap_pat = l
        .add_account("3002", "Capital Pat", AccountType::Equity, None, false)
        .unwrap();
    let dist_mo = l
        .add_account("3901", "Draws Mo", AccountType::Equity, None, false)
        .unwrap();
    let old_mo = l.add_party("Mo", &[Role::Member]).unwrap();
    l.update_party(old_mo, PartyField::Archived(true)).unwrap();
    let mo = l.add_party("MO", &[Role::Member]).unwrap();
    let _pat = l.add_party("Pat", &[Role::Employee]).unwrap();
    let _lee = l.add_party("Lee", &[Role::Member]).unwrap();

    let m = |name: &str, share: &str, capital: &str, dist: Option<&str>| Member {
        name: name.into(),
        share: share.parse().unwrap(),
        capital_account: capital.into(),
        distribution_account: dist.map(str::to_string),
        files_jointly_with: None,
    };
    let c = Company {
        members: vec![
            m("mo", "0.4", "3001", Some("3901")),
            m("Pat", "0.3", "3002", None), // a vendor, not a member
            m("Lee", "0.2", "3999", None), // capital account not in the chart
            m("Nobody", "0.1", "3002", Some("39")), // no such party
        ],
        ..Company::default()
    };
    let got = c.party_members(&l);
    assert_eq!(got.len(), 1);
    assert_eq!(
        got[0].party, mo,
        "the live party of the name, not the archived one"
    );
    assert_eq!(
        (got[0].capital, got[0].distributions),
        (cap_mo, Some(dist_mo))
    );

    let c = Company {
        members: vec![m("mo", "1", "3001", Some("3777"))],
        ..Company::default()
    };
    assert_eq!(
        c.party_members(&l)[0].distributions,
        None,
        "a code the chart lacks is none"
    );
    let _ = cap_pat;
}

// ---------------------------------------------------------------- payment methods

/// INVARIANTS 1, 7: `invoicing.payment_methods` is at most eight objects of
/// `{kind, label, lines, link?, qr?}`; the kind is one of the list, lines
/// are strings, a flag is true or false or the words a file writes. Every
/// refusal names the method (1-based).
#[test]
fn payment_methods_are_refused_for_shape_and_flags() {
    let methods = |v: Vec<Value>| refused("invoicing.payment_methods", Value::Array(v));
    let ach = |extra: Value| {
        let mut m = json!({"kind": "ach", "label": "Bank", "lines": ["Routing 021000021"]});
        for (k, v) in extra.as_object().unwrap() {
            m[k] = v.clone();
        }
        m
    };
    let nine: Vec<Value> = (0..9).map(|_| ach(json!({}))).collect();
    assert!(methods(nine).contains("at most 8 payment methods"));
    assert!(methods(vec![json!("ach")]).contains("method 1: expected {kind, label, lines}"));
    assert!(
        methods(vec![ach(json!({"colour": "red"}))]).contains("method 1: unknown field \"colour\"")
    );
    assert!(methods(vec![ach(json!({"kind": "barter"}))]).contains("method 1: kind is one of"));
    assert!(methods(vec![ach(json!({"kind": 3}))]).contains("method 1: kind is one of"));
    assert!(methods(vec![ach(json!({"label": ""}))]).contains("method 1 label is blank"));
    let mut no_lines = ach(json!({}));
    no_lines.as_object_mut().unwrap().remove("lines");
    assert!(methods(vec![no_lines]).contains("method 1: lines is a list of strings"));
    assert!(methods(vec![ach(json!({"lines": [4]}))]).contains("method 1: each line is a string"));
    assert!(methods(vec![ach(json!({})), ach(json!({"link": 4}))]).contains("method 2 link"));
    assert!(methods(vec![ach(json!({"link": "  "}))]).contains("method 1 link is blank"));
    assert!(
        methods(vec![ach(json!({"qr": "yes"}))])
            .contains("method 1 qr \"yes\": expected true or false")
    );
    assert!(methods(vec![ach(json!({"qr": 1}))]).contains("expected true or false"));

    // The kept form: kind lowercased, a flag as a bool whatever it was
    // written as, optional parts only when present.
    let kept = checked(
        "invoicing.payment_methods",
        json!([
            {"kind": "ACH", "label": " Bank ", "lines": ["Routing 021000021"], "qr": "true"},
            {"kind": "link", "label": "Pay online", "lines": [], "link": "https://pay.example/inv", "qr": false, "extra": null}
        ]),
    );
    // `extra` is an unknown field even when null.
    assert!(kept.is_err());
    let kept = checked(
        "invoicing.payment_methods",
        json!([
            {"kind": "ACH", "label": " Bank ", "lines": ["Routing 021000021"], "qr": "true"},
            {"kind": "link", "label": "Pay online", "lines": [], "link": "https://pay.example/inv", "qr": "false"},
            {"kind": "check", "label": "Cheque", "lines": ["Payable to Wanderland"], "qr": null, "link": null}
        ]),
    )
    .unwrap();
    assert_eq!(
        kept[0],
        json!({"kind": "ach", "label": "Bank", "lines": ["Routing 021000021"], "qr": true})
    );
    assert_eq!(kept[1]["qr"], json!(false));
    assert_eq!(kept[1]["link"], json!("https://pay.example/inv"));
    assert_eq!(
        kept[2],
        json!({"kind": "check", "label": "Cheque", "lines": ["Payable to Wanderland"]})
    );
}

/// INVARIANT 3 / BS-entry-4: replay takes any value of the right shape.
/// For payment methods the shape is the list of objects with a kind and a
/// label (strings), lines (a list of strings), a link string and a flag
/// that is a bool or null; for members, shares in (0, 1]; for amounts, a
/// decimal at all; for the bar, one `AttestationBar::parse` reads.
#[test]
fn replay_takes_any_value_of_the_right_shape() {
    let shaped = |name: &str, v: Value| shape(key(name).unwrap(), &v);
    // Payment methods.
    let method = json!({"kind": "weird-kind-from-the-future", "label": "L", "lines": ["x"], "link": "u", "qr": true});
    assert!(shaped("invoicing.payment_methods", json!([method.clone()])).is_ok());
    assert!(shaped("invoicing.payment_methods", json!("nope")).is_err());
    assert!(
        shaped("invoicing.payment_methods", json!([5]))
            .unwrap_err()
            .contains("method 1: expected an object")
    );
    for missing in ["kind", "label", "lines"] {
        let mut m = method.clone();
        m.as_object_mut().unwrap().remove(missing);
        let e = shaped("invoicing.payment_methods", json!([m])).unwrap_err();
        assert!(e.contains(&format!("method 1: no {missing}")), "{e}");
    }
    let mut bad_line = method.clone();
    bad_line["lines"] = json!([1]);
    assert!(shaped("invoicing.payment_methods", json!([bad_line])).is_err());
    let mut bad_link = method.clone();
    bad_link["link"] = json!(1);
    assert!(shaped("invoicing.payment_methods", json!([bad_link])).is_err());
    let mut null_link = method.clone();
    null_link["link"] = Value::Null;
    null_link["qr"] = Value::Null;
    assert!(shaped("invoicing.payment_methods", json!([null_link])).is_ok());
    let mut bad_qr = method.clone();
    bad_qr["qr"] = json!("yes");
    assert!(
        shaped("invoicing.payment_methods", json!([bad_qr]))
            .unwrap_err()
            .contains("method 1 qr: expected true or false")
    );

    // Members: a share past (0, 1] could overflow the figures that
    // multiply by it, so even replay refuses it.
    let over = json!([{"name": "Mo", "share": "1.5", "capital_account": "3001"}]);
    assert!(
        shaped("company.members", over)
            .unwrap_err()
            .contains("outside (0, 1]")
    );
    let zero = json!([{"name": "Mo", "share": "0", "capital_account": "3001"}]);
    assert!(shaped("company.members", zero).is_err());
    let odd_total = json!([{"name": "Mo", "share": "0.4", "capital_account": "3001"}]);
    assert!(
        shaped("company.members", odd_total).is_ok(),
        "the total is not replay's to refuse"
    );
    assert!(
        shaped("company.members", json!([{"name": "Mo"}]))
            .unwrap_err()
            .contains("company.members 1: bad share")
    );
    assert!(
        shaped(
            "company.members",
            json!([{"share": "1", "capital_account": "3"}])
        )
        .unwrap_err()
        .contains("bad name")
    );
    assert!(
        shaped("company.members", json!([{"name": "Mo", "share": "1"}]))
            .unwrap_err()
            .contains("bad capital_account")
    );

    // Amounts, counts, the bar, code lists.
    assert!(
        shaped("policy.evidence.threshold", json!("-3.00")).is_ok(),
        "negative is for new commands to refuse"
    );
    assert!(
        shaped("policy.evidence.threshold", json!("lots"))
            .unwrap_err()
            .contains("is not a decimal")
    );
    assert!(shaped("policy.attestation_bar", json!("effort=high")).is_ok());
    assert!(
        shaped("policy.attestation_bar", json!("effort=wild"))
            .unwrap_err()
            .starts_with("policy.attestation_bar: ")
    );
    assert!(shaped("books.cash_accounts", json!(["1000", "x"])).is_ok());
    assert!(shaped("books.cash_accounts", json!([1000])).is_err());
    assert!(shaped("invoicing.unsent_days", json!(3)).is_ok());
}

// ---------------------------------------------------------------- other kinds

/// INVARIANT 1 and 6: a month start is zero-padded; payment terms keep one
/// spelling; the attestation bar is kept as `describe` spells it; a time
/// zone is one the database knows. A refusal names the key.
#[test]
fn month_terms_bar_and_zone_keep_one_spelling() {
    assert_eq!(
        checked("company.fiscal_year_start", json!("7-1")).unwrap(),
        json!("07-01")
    );
    assert!(!refused("company.fiscal_year_start", json!("07-15")).is_empty());
    assert!(!refused("company.fiscal_year_start", json!("13-01")).is_empty());

    assert_eq!(
        checked("invoicing.default_terms", json!("net 30")).unwrap(),
        json!("Net 30")
    );
    assert!(
        refused("invoicing.default_terms", json!("whenever"))
            .starts_with("invoicing.default_terms: ")
    );

    assert_eq!(
        checked(
            "policy.attestation_bar",
            json!("verifiers=3, adversarial=false")
        )
        .unwrap(),
        json!("effort=high verifiers=3 adversarial=false")
    );
    assert!(
        refused("policy.attestation_bar", json!("verifiers=0"))
            .starts_with("policy.attestation_bar: ")
    );

    assert!(checked("company.timezone", json!("America/Los_Angeles")).is_ok());
    assert!(!refused("company.timezone", json!("Mars/Olympus_Mons")).is_empty());
    // "Today" in an unknown zone is the computer's, never a panic.
    assert_eq!(today_in(Some("Mars/Olympus_Mons")), today_in(None));
    assert!(today_in(Some("America/Los_Angeles")) >= d(2026, 11, 9));
}

/// INVARIANT 8: an invoice number's prefix is 1 to 12 visible ASCII
/// characters, no spaces.
#[test]
fn an_invoice_prefix_is_short_and_visible() {
    assert_eq!(
        checked("invoicing.number_prefix", json!("WL-")).unwrap(),
        json!("WL-")
    );
    for bad in ["WANDERLAND-2026-", "A B", "É-"] {
        assert!(
            refused("invoicing.number_prefix", json!(bad)).contains("1 to 12 visible ASCII"),
            "{bad}"
        );
    }
    assert!(!refused("invoicing.number_prefix", json!("")).is_empty());
}

// ---------------------------------------------------------------- a whole change

/// INVARIANT 2: a change is all or nothing. A key listed twice, an unknown
/// key (a newer version's), a value of the wrong kind, a source without a
/// name, refuse the whole command; changes that change nothing are left
/// out; `null` clears a key; applying the checked changes then agrees.
#[test]
fn a_change_is_all_or_nothing_and_leaves_out_what_changes_nothing() {
    let mut s = settings(&[("company.name", json!("Wanderland Labs LLC"))]);
    let before = s.clone();

    let twice = [
        change("company.name", json!("A")),
        change("company.name", json!("B")),
    ];
    assert_eq!(
        s.check_changes(&twice, &src()).unwrap_err(),
        "company.name is changed twice"
    );
    let unknown = [change("company.nme", json!("A"))];
    assert!(
        s.check_changes(&unknown, &src())
            .unwrap_err()
            .contains("unknown setting \"company.nme\"")
    );
    let half_bad = [
        change("company.name", json!("Wanderland Labs, Inc.")),
        change("invoicing.first_number", json!("zero")),
    ];
    assert!(s.check_changes(&half_bad, &src()).is_err());
    let mut nameless = src();
    nameless.decided_by = " ".into();
    assert!(
        s.check_changes(&[change("company.name", json!("A"))], &nameless)
            .is_err()
    );
    assert_eq!(s, before, "a refused change changes nothing");

    // The same name again is no change; a clear of an unset key is none.
    let same = [change("company.name", json!("Wanderland Labs LLC"))];
    assert!(s.check_changes(&same, &src()).unwrap().is_empty());
    let clear_unset = [Change {
        key: "company.cpa".into(),
        value: None,
    }];
    assert!(s.check_changes(&clear_unset, &src()).unwrap().is_empty());

    // A real change and a clear, together: both are made, in canonical form.
    let both = [
        change("company.cpa", json!("  Pat Accountant  ")),
        Change {
            key: "company.name".into(),
            value: Some(Value::Null),
        },
    ];
    let made = s.check_changes(&both, &src()).unwrap();
    assert_eq!(made.len(), 2);
    s.apply(&made, &src(), 2).unwrap();
    assert_eq!(s.company("USD").name, "Unnamed company");
}

// ---------------------------------------------------------------- the readers

/// INVARIANT 5: unset keys mean the defaults; a set key means its word.
/// `policy.cpa_package_requires_audit` has three states (unset, warn,
/// refuse).
#[test]
fn readers_give_the_default_unset_and_the_word_when_set() {
    let unset = Settings::default();
    assert_eq!(unset.schedule_convention(), ScheduleConvention::Daily);
    assert_eq!(unset.parties(), PartyPolicy::Suggest);
    assert_eq!(unset.require_tax_category(), TaxCategoryPolicy::Warn);
    assert_eq!(unset.approval_rules(), ApprovalMode::Off);
    assert_eq!(
        unset.attestation_bar().describe(),
        "effort=high verifiers=2 adversarial=true"
    );
    assert_eq!(unset.cpa_package_requires_audit(), None);
    assert_eq!(unset.backup_git_remote(), None);
    assert_eq!(unset.backup_folder(), None);
    assert_eq!(unset.receipt_threshold(), None);

    let set = settings(&[
        ("books.schedule_convention", json!("monthly_even")),
        ("policy.parties", json!("require")),
        ("policy.require_tax_category", json!("refuse")),
        ("policy.approval_rules", json!("enforce")),
        (
            "policy.attestation_bar",
            json!("effort=max verifiers=4 adversarial=false"),
        ),
        ("policy.cpa_package_requires_audit", json!("refuse")),
        ("backup.git_remote", json!("git@git.example:books.git")),
        ("backup.folder", json!(BACKUP_FOLDER)),
    ]);
    assert_eq!(set.schedule_convention(), ScheduleConvention::MonthlyEven);
    assert_eq!(set.parties(), PartyPolicy::Require);
    assert_eq!(set.require_tax_category(), TaxCategoryPolicy::Refuse);
    assert_eq!(set.approval_rules(), ApprovalMode::Enforce);
    assert_eq!(
        set.attestation_bar().describe(),
        "effort=max verifiers=4 adversarial=false"
    );
    assert_eq!(set.cpa_package_requires_audit(), Some(true));
    assert_eq!(set.backup_git_remote(), Some("git@git.example:books.git"));
    assert_eq!(set.backup_folder(), Some(BACKUP_FOLDER));

    let warn = settings(&[("policy.cpa_package_requires_audit", json!("warn"))]);
    assert_eq!(warn.cpa_package_requires_audit(), Some(false));
}

/// The company the settings describe: a lenient fiscal year start that a
/// later version may have written, cash basis, a tax form from the
/// classification when none is set, members as written.
#[test]
fn the_company_is_read_from_the_settings() {
    let s = settings(&[
        ("company.name", json!("Wanderland Labs LLC")),
        ("company.fiscal_year_start", json!("07-01")),
        ("books.accounting_basis", json!("cash")),
        ("company.tax_classification", json!("partnership")),
        (
            "company.members",
            json!([member("Mo", "50%", "3001"), member("Tobin", "50%", "3002")]),
        ),
    ]);
    let c = s.company("USD");
    assert_eq!(c.name, "Wanderland Labs LLC");
    assert_eq!(c.fiscal_year_start, (7, 1));
    assert_eq!(c.basis, "Cash");
    assert!(c.tax_form.is_some());
    assert_eq!(c.members.len(), 2);
    assert_eq!(c.members[0].share, Decimal::new(5, 1));

    // Tax form set outright wins over the classification.
    let s = settings(&[
        ("company.tax_form", json!("1120")),
        ("company.tax_classification", json!("partnership")),
    ]);
    assert_eq!(
        s.company("USD").tax_form,
        ledger_core::TaxForm::parse("1120")
    );
}

/// What a setting looks like to a reader: a list of payment methods by
/// label, other lists joined, an identity number masked unless the reader
/// is a report, a plain value as it is.
#[test]
fn display_shows_what_a_reader_may_see() {
    let methods = key("invoicing.payment_methods").unwrap();
    let v = json!([
        {"kind": "ach", "label": "Bank", "lines": []},
        {"kind": "link", "label": "Pay online", "lines": []}
    ]);
    assert_eq!(display(methods, &v, false), "Bank, Pay online");
    let codes = key("books.cash_accounts").unwrap();
    assert_eq!(
        display(codes, &json!(["1000", "1010"]), false),
        "1000, 1010"
    );
    let ein = key("company.ein").unwrap();
    let masked = display(ein, &json!("12-3456789"), false);
    assert!(!masked.contains("3456"), "{masked}");
    assert!(masked.ends_with("6789"), "{masked}");
    assert_eq!(display(ein, &json!("12-3456789"), true), "12-3456789");
    assert_eq!(
        display(key("invoicing.unsent_days").unwrap(), &json!(3), false),
        "3"
    );
}
