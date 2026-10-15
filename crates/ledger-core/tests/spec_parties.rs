//! Spec-derived tests for the party directory.
//!
//! Oracles: the spec block of `ledger_core::parties` (INVARIANTS 1–4), the
//! doc comments of `has_full_tin`, `check_text`, `W9Facts`/`W4Facts` and
//! their `check`, `Role::gets_1099`, `Party::label`/`all_roles`,
//! `name_key`, and the `Ledger` party methods (`add_party`,
//! `update_party`, `set_party_role`, `set_party_w9`/`w4`,
//! `set_entry_party`, `set_entry_vendor`, the vendor/customer auto-link,
//! `validate_payroll`), plus `docs/parties-and-attachments.md` "Parties".
//! Every expected value is worked by hand from those texts; nothing is read
//! back from the code under test and then asserted.

use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

use chrono::NaiveDate;
use ledger_core::parties::{check_text, has_full_tin, name_key};
use ledger_core::{
    AccountId, AccountType, Currency, EntryId, Ledger, Money, Party, PartyField, PartyId,
    PayrollLine, Role, Side, W4Facts, W9Facts,
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

struct Fx {
    cash: AccountId,
    posted: EntryId,
    draft: EntryId,
}

/// A USD ledger with cash, capital, an expense, one posted entry
/// (Dr 1000 / Cr 3000 $100.00) and one draft entry.
fn fixture() -> (Ledger, Fx) {
    let mut l = Ledger::new(Currency::USD);
    let cash = l
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    let capital = l
        .add_account("3000", "Owner Capital", AccountType::Equity, None, false)
        .unwrap();
    let expense = l
        .add_account(
            "6050",
            "Domain & Hosting",
            AccountType::Expense,
            None,
            false,
        )
        .unwrap();
    let posted = l.try_create_entry(date(2026, 1, 1), "Opening").unwrap();
    l.add_line(posted, cash, Side::Debit, usd("100.00"))
        .unwrap();
    l.add_line(posted, capital, Side::Credit, usd("100.00"))
        .unwrap();
    l.post_entry(posted).unwrap();
    let draft = l.try_create_entry(date(2026, 1, 2), "Hosting").unwrap();
    l.add_line(draft, expense, Side::Debit, usd("12.00"))
        .unwrap();
    l.add_line(draft, cash, Side::Credit, usd("12.00")).unwrap();
    (
        l,
        Fx {
            cash,
            posted,
            draft,
        },
    )
}

fn w9(last4: &str) -> W9Facts {
    W9Facts {
        received: date(2026, 3, 1),
        entity_type: "individual/sole proprietor".to_string(),
        exempt_payee_code: None,
        fatca_code: None,
        tin_last4: last4.to_string(),
        kept_at: "wanderbooks vault".to_string(),
        vault_record: None,
    }
}

fn w4(state: &str) -> W4Facts {
    W4Facts {
        received: date(2026, 3, 1),
        state: state.to_string(),
        kept_at: "CPA portal".to_string(),
    }
}

fn party(l: &Ledger, id: PartyId) -> &Party {
    l.get_party(id).expect("party exists")
}

/// Run `f` on `l`; it must fail and leave `l` equal to before.
fn refused<T: std::fmt::Debug, E>(
    l: &mut Ledger,
    what: &str,
    f: impl FnOnce(&mut Ledger) -> Result<T, E>,
) {
    let before = l.clone();
    let r = f(l);
    assert!(r.is_err(), "{what}: expected a refusal, got {:?}", r.ok());
    assert!(*l == before, "{what}: a refusal changed the ledger");
}

fn bare_party(name: &str) -> Party {
    Party {
        id: 1,
        name: name.to_string(),
        display_name: None,
        roles: BTreeSet::new(),
        website: None,
        address: None,
        notes: None,
        archived: false,
        w9: None,
        w4: None,
        vendor: None,
        customer: None,
    }
}

fn pay_line(name: &str, employee: Option<PartyId>) -> PayrollLine {
    PayrollLine {
        employee_name: name.to_string(),
        employee,
        gross_pay: d("1000.00"),
        federal_withholding: d("100.00"),
        state_withholding: d("0"),
        ss_employee: d("62.00"),
        medicare_employee: d("14.50"),
        ss_employer: d("62.00"),
        medicare_employer: d("14.50"),
        other_deductions: d("0"),
    }
}

// ------------------------------------------------- has_full_tin truth table

/// Oracle: `has_full_tin` doc ("a run of nine digits, with at most one
/// space or hyphen between neighbors") and INVARIANT 1.
#[test]
fn has_full_tin_truth_table() {
    let cases: &[(&str, bool)] = &[
        ("123456789", true),
        ("123-45-6789", true), // SSN
        ("12-3456789", true),  // EIN
        ("1234 5678 9", true), // single spaces
        ("123 45-6789", true), // mixed single separators
        ("1-2-3-4-5-6-7-8-9", true),
        ("TIN 123456789 on file", true), // inside longer text
        ("abc123456789def", true),
        ("12  3456789", false), // two spaces break the run: 2 + 7 digits
        ("12--3456789", false), // two hyphens
        ("12 -3456789", false), // space and hyphen between neighbors
        ("12\t3456789", false), // a tab is not a space
        ("12345678", false),    // eight digits
        ("123-45-678", false),
        ("1234", false),
        ("", false),
        ("Acme LLC", false),
        ("Suite 100, 84101", false), // 3 + 5 digits, separated by ", "
        ("-123456789-", true),
    ];
    for (text, want) in cases {
        assert_eq!(has_full_tin(text), *want, "has_full_tin({text:?})");
    }
}

/// Oracle: INVARIANT 1 says "any run of nine digits"; a run of ten digits
/// holds a run of nine.
#[test]
fn has_full_tin_ten_digits_hold_nine() {
    assert!(has_full_tin("1234567890"));
    assert!(has_full_tin("801-555-12345"));
}

/// Oracle: INVARIANT 1 ("No party field holds a full taxpayer
/// identification number"). Full-width digits are Unicode decimal digits;
/// a TIN typed with them is still a full TIN.
#[test]
fn has_full_tin_unicode_digits() {
    assert!(has_full_tin("１２３４５６７８９"));
    assert!(has_full_tin("١٢٣٤٥٦٧٨٩")); // Arabic-Indic digits
}

// ---------------------------------------------------------------- check_text

/// Oracle: `check_text` doc: not a full TIN, at most 500 characters, no
/// control characters.
#[test]
fn check_text_rules() {
    assert!(check_text("notes", "Pays by ACH, net 30").is_ok());
    assert!(check_text("notes", "").is_ok());
    assert!(check_text("notes", &"a".repeat(500)).is_ok());
    assert!(check_text("notes", &"a".repeat(501)).is_err());
    // Characters, not bytes: 500 two-byte characters are 500 characters.
    assert!(check_text("notes", &"é".repeat(500)).is_ok());
    assert!(check_text("notes", &"é".repeat(501)).is_err());
    assert!(check_text("notes", "nul\0").is_err());
    assert!(check_text("notes", "EIN 12-3456789").is_err());
    assert!(check_text("notes", "last four 6789").is_ok());
    let why = check_text("notes", "123456789").unwrap_err();
    assert!(!why.is_empty(), "a refusal says why");
}

// ---------------------------------------------------------------- facts

/// Oracle: `W9Facts.tin_last4` doc ("exactly four digits") and
/// INVARIANT 1.
#[test]
fn w9_check_tin_last4_is_exactly_four_digits() {
    assert!(w9("6789").check().is_ok());
    assert!(w9("0000").check().is_ok());
    for bad in ["", "678", "56789", "67a9", "67 9", "123456789", "6789 "] {
        assert!(w9(bad).check().is_err(), "tin_last4 {bad:?}");
    }
}

/// Oracle: INVARIANT 1: no party field (the W-9 facts' text included)
/// holds a full TIN.
#[test]
fn w9_check_refuses_full_tin_in_any_text() {
    let mut f = w9("6789");
    f.entity_type = "LLC 12-3456789".to_string();
    assert!(f.check().is_err());
    let mut f = w9("6789");
    f.kept_at = "drawer 123456789".to_string();
    assert!(f.check().is_err());
    let mut f = w9("6789");
    f.exempt_payee_code = Some("123-45-6789".to_string());
    assert!(f.check().is_err());
    let mut f = w9("6789");
    f.fatca_code = Some("123456789".to_string());
    assert!(f.check().is_err());
    let mut f = w9("6789");
    f.exempt_payee_code = Some("5".to_string());
    f.fatca_code = Some("A".to_string());
    assert!(f.check().is_ok());
}

/// Oracle: `W4Facts.state` doc ("two letters") and INVARIANT 1.
#[test]
fn w4_check_state_two_letters() {
    assert!(w4("UT").check().is_ok());
    assert!(w4("CA").check().is_ok());
    for bad in ["", "U", "UTA", "U1", "12", "U "] {
        assert!(w4(bad).check().is_err(), "state {bad:?}");
    }
    let mut f = w4("UT");
    f.kept_at = "SSN 123-45-6789".to_string();
    assert!(f.check().is_err());
}

// ---------------------------------------------------------------- roles

/// Oracle: `Role::gets_1099` doc and INVARIANT 2 ("a 1099 role
/// (contractor or landlord)").
#[test]
fn gets_1099_is_contractor_and_landlord_only() {
    for r in Role::ALL {
        let want = matches!(r, Role::Contractor | Role::Landlord);
        assert_eq!(r.gets_1099(), want, "{r:?}");
    }
}

/// Oracle: `Role::ALL` ("every role"), `Role::key` (wire name, e.g.
/// `payout_platform`), `Role::parse` ("the role named key").
#[test]
fn role_keys_round_trip() {
    let all: BTreeSet<Role> = Role::ALL.into_iter().collect();
    assert_eq!(all.len(), 9, "nine distinct roles");
    for r in [
        Role::Vendor,
        Role::Contractor,
        Role::Customer,
        Role::Employee,
        Role::Member,
        Role::PayoutPlatform,
        Role::TaxAgency,
        Role::FinancialInstitution,
        Role::Landlord,
    ] {
        assert!(all.contains(&r));
        assert_eq!(Role::parse(r.key()), Some(r));
    }
    assert_eq!(Role::PayoutPlatform.key(), "payout_platform");
    assert_eq!(Role::TaxAgency.key(), "tax_agency");
    assert_eq!(Role::FinancialInstitution.key(), "financial_institution");
    assert_eq!(Role::Contractor.key(), "contractor");
    assert_eq!(Role::parse("boss"), None);
    assert_eq!(Role::parse(""), None);
}

// ---------------------------------------------------------------- Party

/// Oracle: `Party::label` doc: the display name, else the name.
#[test]
fn label_prefers_display_name() {
    let mut p = bare_party("Anthropic, PBC");
    assert_eq!(p.label(), "Anthropic, PBC");
    p.display_name = Some("Anthropic".to_string());
    assert_eq!(p.label(), "Anthropic");
}

/// Oracle: `Party::all_roles` doc: hand roles plus vendor and customer
/// when it has those records.
#[test]
fn all_roles_adds_record_roles() {
    let mut p = bare_party("Acme");
    p.roles.insert(Role::Contractor);
    assert_eq!(p.all_roles(), BTreeSet::from([Role::Contractor]));
    p.vendor = Some(7);
    assert_eq!(
        p.all_roles(),
        BTreeSet::from([Role::Contractor, Role::Vendor])
    );
    p.customer = Some(3);
    assert_eq!(
        p.all_roles(),
        BTreeSet::from([Role::Contractor, Role::Vendor, Role::Customer])
    );
    p.vendor = None;
    assert_eq!(
        p.all_roles(),
        BTreeSet::from([Role::Contractor, Role::Customer])
    );
}

/// Oracle: `name_key` doc: case and surrounding space ignored (inner
/// space is not surrounding space).
#[test]
fn name_key_ignores_case_and_surrounding_space_only() {
    assert_eq!(name_key("Acme"), name_key("  ACME "));
    assert_eq!(name_key("acme llc"), name_key("Acme LLC\t"));
    assert_ne!(name_key("Acme"), name_key("Ac me"));
    assert_ne!(name_key("Acme"), name_key("Acme Inc"));
}

// ---------------------------------------------------------------- add_party

/// Oracle: add_party GOAL; parties() "every party, in id order";
/// party_by_name ignoring case and surrounding space.
#[test]
fn add_party_creates_a_record() {
    let (mut l, _) = fixture();
    let p = l
        .add_party("Internal Revenue Service", &[Role::TaxAgency])
        .unwrap();
    let got = party(&l, p);
    assert_eq!(got.id, p);
    assert_eq!(got.name, "Internal Revenue Service");
    assert_eq!(got.roles, BTreeSet::from([Role::TaxAgency]));
    assert!(!got.archived);
    assert_eq!(got.w9, None);
    assert_eq!(got.w4, None);
    assert_eq!(got.vendor, None);
    assert_eq!(got.customer, None);
    assert_eq!(
        l.party_by_name("  internal revenue SERVICE ").map(|x| x.id),
        Some(p)
    );
    let q = l
        .add_party("Utah State Tax Commission", &[Role::TaxAgency])
        .unwrap();
    assert_ne!(p, q);
    let ids: Vec<PartyId> = l.parties().map(|x| x.id).collect();
    let mut sorted = ids.clone();
    sorted.sort();
    assert_eq!(ids, sorted, "parties() is in id order");
    assert!(ids.contains(&p) && ids.contains(&q));
}

/// Oracle: add_party SCENARIO 1.
#[test]
fn add_party_refuses_blank_tin_or_taken_names() {
    let (mut l, _) = fixture();
    l.add_party("Apple", &[Role::PayoutPlatform]).unwrap();
    refused(&mut l, "blank", |l| l.add_party("", &[]));
    refused(&mut l, "spaces", |l| l.add_party("   ", &[]));
    refused(&mut l, "TIN", |l| l.add_party("Bob 123-45-6789", &[]));
    refused(&mut l, "same name", |l| l.add_party("Apple", &[]));
    refused(&mut l, "case", |l| l.add_party("APPLE", &[]));
    refused(&mut l, "space", |l| l.add_party("  apple ", &[]));
    assert!(l.check_add_party("Apple", &[]).is_err());
    assert!(l.check_add_party("Google", &[]).is_ok());
}

/// Oracle: add_party SCENARIO 2.
#[test]
fn add_party_refuses_vendor_and_customer_roles() {
    let (mut l, _) = fixture();
    refused(&mut l, "vendor", |l| l.add_party("Acme", &[Role::Vendor]));
    refused(&mut l, "customer", |l| {
        l.add_party("Acme", &[Role::Member, Role::Customer])
    });
    assert!(l.check_add_party("Acme", &[Role::Vendor]).is_err());
}

/// Oracle: INVARIANT 3 ("among parties not archived"): an archived
/// party's name is free again.
#[test]
fn add_party_may_reuse_an_archived_name() {
    let (mut l, _) = fixture();
    let old = l.add_party("Landlord Co", &[Role::Landlord]).unwrap();
    l.update_party(old, PartyField::Archived(true)).unwrap();
    let new = l.add_party("landlord co", &[Role::Landlord]).unwrap();
    assert_ne!(old, new);
    assert_eq!(l.party_by_name("Landlord Co").map(|p| p.id), Some(new));
    assert!(party(&l, old).archived);
}

/// Oracle: check_add_party is "the validation behind add_party": it
/// agrees with add_party and changes nothing.
#[test]
fn check_add_party_agrees_with_add_party() {
    let (mut l, _) = fixture();
    l.add_party("Taken", &[]).unwrap();
    for (name, roles) in [
        ("Taken", vec![]),
        ("Fresh", vec![Role::Member]),
        ("", vec![]),
        ("Fresh2", vec![Role::Customer]),
        ("x 123456789", vec![]),
    ] {
        let before = l.clone();
        let check = l.check_add_party(name, &roles);
        assert!(l == before);
        let mut trial = l.clone();
        let real = trial.add_party(name, &roles);
        assert_eq!(check.is_ok(), real.is_ok(), "{name:?}");
    }
}

// ---------------------------------------------------------------- update_party

/// Oracle: PartyField docs; update_party "change one of a party's facts".
#[test]
fn update_party_changes_one_fact() {
    let (mut l, _) = fixture();
    let p = l.add_party("Anthropic, PBC", &[Role::Contractor]).unwrap();
    l.update_party(p, PartyField::DisplayName(Some("Anthropic".into())))
        .unwrap();
    assert_eq!(party(&l, p).display_name.as_deref(), Some("Anthropic"));
    assert_eq!(party(&l, p).label(), "Anthropic");
    l.update_party(p, PartyField::Website(Some("https://anthropic.com".into())))
        .unwrap();
    l.update_party(
        p,
        PartyField::Address(Some("548 Market St, San Francisco".into())),
    )
    .unwrap();
    l.update_party(p, PartyField::Notes(Some("API billing".into())))
        .unwrap();
    let got = party(&l, p);
    assert_eq!(got.website.as_deref(), Some("https://anthropic.com"));
    assert_eq!(got.address.as_deref(), Some("548 Market St, San Francisco"));
    assert_eq!(got.notes.as_deref(), Some("API billing"));
    assert_eq!(got.name, "Anthropic, PBC");
    l.update_party(p, PartyField::DisplayName(None)).unwrap();
    assert_eq!(party(&l, p).display_name, None);
    assert_eq!(party(&l, p).label(), "Anthropic, PBC");
    l.update_party(p, PartyField::Website(None)).unwrap();
    assert_eq!(party(&l, p).website, None);
    l.update_party(p, PartyField::Name("Anthropic PBC".into()))
        .unwrap();
    assert_eq!(party(&l, p).name, "Anthropic PBC");
    assert_eq!(l.party_by_name("anthropic pbc").map(|x| x.id), Some(p));
    assert!(l.party_by_name("Anthropic, PBC").is_none());
    l.update_party(p, PartyField::Archived(true)).unwrap();
    assert!(party(&l, p).archived);
    assert!(
        l.party_by_name("Anthropic PBC").is_none(),
        "party_by_name skips archived"
    );
    l.update_party(p, PartyField::Archived(false)).unwrap();
    assert!(!party(&l, p).archived);
}

/// Oracle: update_party SCENARIO 1.
#[test]
fn update_party_refusals_change_nothing() {
    let (mut l, _) = fixture();
    let a = l.add_party("Alpha", &[]).unwrap();
    l.add_party("Beta", &[]).unwrap();
    let unknown = l.next_party_id() + 10;
    refused(&mut l, "unknown", |l| {
        l.update_party(unknown, PartyField::Notes(None))
    });
    refused(&mut l, "TIN in notes", |l| {
        l.update_party(a, PartyField::Notes(Some("SSN 123-45-6789".into())))
    });
    refused(&mut l, "TIN in website", |l| {
        l.update_party(a, PartyField::Website(Some("x.com/123456789".into())))
    });
    refused(&mut l, "TIN in address", |l| {
        l.update_party(a, PartyField::Address(Some("12-3456789 Main".into())))
    });
    refused(&mut l, "TIN in display name", |l| {
        l.update_party(a, PartyField::DisplayName(Some("123456789".into())))
    });
    refused(&mut l, "TIN in name", |l| {
        l.update_party(a, PartyField::Name("Alpha 123456789".into()))
    });
    refused(&mut l, "too long", |l| {
        l.update_party(a, PartyField::Notes(Some("n".repeat(501))))
    });
    refused(&mut l, "control char", |l| {
        l.update_party(a, PartyField::Notes(Some("a\u{7}b".into())))
    });
    refused(&mut l, "blank name", |l| {
        l.update_party(a, PartyField::Name("  ".into()))
    });
    refused(&mut l, "taken name", |l| {
        l.update_party(a, PartyField::Name("Beta".into()))
    });
    refused(&mut l, "taken name, case/space", |l| {
        l.update_party(a, PartyField::Name(" BETA ".into()))
    });
    assert!(
        l.check_update_party(a, &PartyField::Name("beta".into()))
            .is_err()
    );
    assert!(
        l.check_update_party(a, &PartyField::Name("Gamma".into()))
            .is_ok()
    );
}

/// Oracle: INVARIANT 3: un-archiving a party whose name another live party
/// now holds would make two live parties share a name.
#[test]
fn unarchiving_into_a_taken_name_is_refused() {
    let (mut l, _) = fixture();
    let old = l.add_party("Acme", &[]).unwrap();
    l.update_party(old, PartyField::Archived(true)).unwrap();
    l.add_party("ACME", &[]).unwrap();
    refused(&mut l, "unarchive", |l| {
        l.update_party(old, PartyField::Archived(false))
    });
}

/// Oracle: INVARIANT 3 counts only parties not archived: renaming onto an
/// archived party's name is fine.
#[test]
fn rename_onto_archived_name_is_allowed() {
    let (mut l, _) = fixture();
    let old = l.add_party("Acme", &[]).unwrap();
    l.update_party(old, PartyField::Archived(true)).unwrap();
    let b = l.add_party("Beta", &[]).unwrap();
    l.update_party(b, PartyField::Name("acme".into())).unwrap();
    assert_eq!(l.party_by_name("Acme").map(|p| p.id), Some(b));
}

// ---------------------------------------------------------------- roles + facts

/// Oracle: set_party_role SCENARIO 1.
#[test]
fn set_party_role_refuses_record_roles() {
    let (mut l, _) = fixture();
    let p = l.add_party("Acme", &[]).unwrap();
    for on in [true, false] {
        refused(&mut l, "vendor", |l| l.set_party_role(p, Role::Vendor, on));
        refused(&mut l, "customer", |l| {
            l.set_party_role(p, Role::Customer, on)
        });
    }
    let unknown = l.next_party_id() + 5;
    refused(&mut l, "unknown party", |l| {
        l.set_party_role(unknown, Role::Member, true)
    });
}

/// Oracle: "give a party a role, or take one away".
#[test]
fn set_party_role_gives_and_takes() {
    let (mut l, _) = fixture();
    let p = l.add_party("Kat", &[]).unwrap();
    l.set_party_role(p, Role::Member, true).unwrap();
    l.set_party_role(p, Role::Employee, true).unwrap();
    assert_eq!(
        party(&l, p).roles,
        BTreeSet::from([Role::Member, Role::Employee])
    );
    l.set_party_role(p, Role::Member, false).unwrap();
    assert_eq!(party(&l, p).roles, BTreeSet::from([Role::Employee]));
}

/// Oracle: INVARIANT 2 and set_party_w9 SCENARIO 1: W-9 facts need a 1099
/// role.
#[test]
fn w9_needs_a_1099_role() {
    let (mut l, _) = fixture();
    let p = l.add_party("Dev Contractor", &[Role::Member]).unwrap();
    refused(&mut l, "no 1099 role", |l| {
        l.set_party_w9(p, Some(w9("6789")))
    });
    assert!(l.check_set_party_w9(p, Some(&w9("6789"))).is_err());
    l.set_party_role(p, Role::Contractor, true).unwrap();
    refused(&mut l, "bad facts", |l| l.set_party_w9(p, Some(w9("67"))));
    l.set_party_w9(p, Some(w9("6789"))).unwrap();
    assert_eq!(party(&l, p).w9, Some(w9("6789")));
    let unknown = l.next_party_id() + 3;
    refused(&mut l, "unknown", |l| {
        l.set_party_w9(unknown, Some(w9("6789")))
    });
    l.set_party_w9(p, None).unwrap();
    assert_eq!(party(&l, p).w9, None);

    let landlord = l.add_party("Office Landlord", &[Role::Landlord]).unwrap();
    l.set_party_w9(landlord, Some(w9("1111"))).unwrap();
    assert_eq!(party(&l, landlord).w9, Some(w9("1111")));
}

/// Oracle: set_party_role SCENARIO 2 (and INVARIANT 2).
#[test]
fn removing_the_last_1099_role_with_w9_is_refused() {
    let (mut l, _) = fixture();
    let p = l
        .add_party("Both", &[Role::Contractor, Role::Landlord])
        .unwrap();
    l.set_party_w9(p, Some(w9("6789"))).unwrap();
    // Another 1099 role remains: allowed.
    l.set_party_role(p, Role::Contractor, false).unwrap();
    // The last one: refused.
    refused(&mut l, "last 1099 role", |l| {
        l.set_party_role(p, Role::Landlord, false)
    });
    assert!(l.check_set_party_role(p, Role::Landlord, false).is_err());
    // Clear the facts first, then it goes.
    l.set_party_w9(p, None).unwrap();
    l.set_party_role(p, Role::Landlord, false).unwrap();
    assert!(party(&l, p).roles.is_empty());
}

/// Oracle: INVARIANT 2 (W-4 facts need employee) and set_party_role
/// SCENARIO 2.
#[test]
fn w4_needs_employee() {
    let (mut l, _) = fixture();
    let p = l.add_party("Alice", &[Role::Contractor]).unwrap();
    refused(&mut l, "no employee role", |l| {
        l.set_party_w4(p, Some(w4("UT")))
    });
    assert!(l.check_set_party_w4(p, Some(&w4("UT"))).is_err());
    l.set_party_role(p, Role::Employee, true).unwrap();
    refused(&mut l, "bad state", |l| l.set_party_w4(p, Some(w4("Utah"))));
    l.set_party_w4(p, Some(w4("UT"))).unwrap();
    assert_eq!(party(&l, p).w4, Some(w4("UT")));
    refused(&mut l, "drop employee with W-4", |l| {
        l.set_party_role(p, Role::Employee, false)
    });
    l.set_party_w4(p, None).unwrap();
    l.set_party_role(p, Role::Employee, false).unwrap();
    assert_eq!(party(&l, p).roles, BTreeSet::from([Role::Contractor]));
}

// ---------------------------------------------------------------- auto-link

/// Oracle: the auto-link doc ("the party not archived with that name ...
/// that has no such record yet, else a new party"); party_of_vendor /
/// party_of_customer; all_roles.
#[test]
fn vendor_and_customer_with_the_same_name_share_one_party() {
    let (mut l, _) = fixture();
    let v = l.add_vendor("Acme").unwrap();
    let pv = l.party_of_vendor(v).expect("vendor has a party");
    assert_eq!(party(&l, pv).vendor, Some(v));
    assert!(party(&l, pv).all_roles().contains(&Role::Vendor));
    let c = l.add_customer("  acme ").unwrap();
    let pc = l.party_of_customer(c).expect("customer has a party");
    assert_eq!(pc, pv, "same name: one party");
    let got = party(&l, pv);
    assert_eq!(got.vendor, Some(v));
    assert_eq!(got.customer, Some(c));
    assert!(got.all_roles().contains(&Role::Customer));
    assert!(got.all_roles().contains(&Role::Vendor));
    assert_eq!(l.parties().count(), 1);
}

/// Oracle: the auto-link doc: a hand-added party with the name and no
/// vendor record absorbs the new vendor.
#[test]
fn hand_added_party_absorbs_vendor_ignoring_case_and_space() {
    let (mut l, _) = fixture();
    let p = l.add_party("Gamma Design", &[Role::Contractor]).unwrap();
    let v = l.add_vendor(" GAMMA design ").unwrap();
    assert_eq!(l.party_of_vendor(v), Some(p));
    assert_eq!(party(&l, p).vendor, Some(v));
    assert_eq!(
        party(&l, p).all_roles(),
        BTreeSet::from([Role::Contractor, Role::Vendor])
    );
}

/// Oracle: the auto-link doc ("not archived").
#[test]
fn archived_parties_do_not_absorb() {
    let (mut l, _) = fixture();
    let p = l.add_party("Beta", &[]).unwrap();
    l.update_party(p, PartyField::Archived(true)).unwrap();
    let v = l.add_vendor("Beta").unwrap();
    let pv = l.party_of_vendor(v).unwrap();
    assert_ne!(pv, p);
    assert_eq!(party(&l, p).vendor, None);
    assert!(!party(&l, pv).archived);
}

/// Oracle: the auto-link doc ("that has no such record yet, else a new
/// party").
#[test]
fn a_second_vendor_with_the_same_name_gets_a_new_party() {
    let (mut l, _) = fixture();
    let v1 = l.add_vendor("Delta").unwrap();
    let v2 = l.add_vendor("Delta").unwrap();
    assert_ne!(v1, v2);
    let p1 = l.party_of_vendor(v1).unwrap();
    let p2 = l.party_of_vendor(v2).unwrap();
    assert_ne!(p1, p2);
    assert_eq!(party(&l, p1).vendor, Some(v1));
    assert_eq!(party(&l, p2).vendor, Some(v2));
    assert_eq!(
        party(&l, p2).name,
        "Delta (2)",
        "INVARIANT 3: names stay unique"
    );
}

/// Oracle: INVARIANT 3 ("Names are unique ... among parties not
/// archived") applied after the auto-link's "else a new party".
#[test]
fn invariant_3_holds_after_a_second_same_name_vendor() {
    let (mut l, _) = fixture();
    l.add_vendor("Delta").unwrap();
    let r = l.add_vendor("Delta");
    if r.is_ok() {
        let live: Vec<String> = l
            .parties()
            .filter(|p| !p.archived)
            .map(|p| name_key(&p.name))
            .collect();
        let set: BTreeSet<&String> = live.iter().collect();
        assert_eq!(set.len(), live.len(), "live names unique: {live:?}");
    }
}

/// Oracle: INVARIANT 4 after mixed auto-linking.
#[test]
fn every_record_links_to_exactly_one_party() {
    let (mut l, _) = fixture();
    l.add_party("Echo", &[]).unwrap();
    for n in ["Echo", "Foxtrot", "echo", "Golf"] {
        l.add_vendor(n).unwrap();
    }
    for n in ["Golf", "Hotel", "ECHO"] {
        l.add_customer(n).unwrap();
    }
    check_invariants(&l);
}

/// Oracle: check_text doc ("no control characters except newline"):
/// addresses run several lines; a tab is refused.
#[test]
fn check_text_newline_yes_tab_no() {
    assert!(check_text("address", "line one\nline two").is_ok());
    assert!(check_text("notes", "tab\there").is_err());
}

/// Oracle: INVARIANT 1 ("No party field holds a full taxpayer
/// identification number: any run of nine digits ... is refused"): the
/// party the auto-link makes for a vendor is a party; its name is a party
/// field.
#[test]
fn add_vendor_name_with_full_tin_is_refused() {
    let (mut l, _) = fixture();
    let before = l.clone();
    if l.add_vendor("Bob 123-45-6789").is_ok() {
        assert!(
            l.parties().all(|p| !has_full_tin(&p.name)),
            "a party holds a full TIN"
        );
    } else {
        assert!(l == before);
    }
    let mut l = before;
    if l.add_customer("Carol 123456789").is_ok() {
        assert!(l.parties().all(|p| !has_full_tin(&p.name)));
    }
}

// ---------------------------------------------------------------- entry tags

/// Oracle: set_entry_party doc: a party with a vendor record also tags the
/// entry's vendor; metadata, so posted entries may be tagged and no amount
/// changes.
#[test]
fn set_entry_party_tags_the_vendor() {
    let (mut l, fx) = fixture();
    let v = l.add_vendor("Hosting Co").unwrap();
    let p = l.party_of_vendor(v).unwrap();
    let cash_before = l.account_balance(fx.cash).unwrap();
    l.set_entry_party(fx.posted, Some(p)).unwrap();
    assert_eq!(l.entry_party(fx.posted), Some(p));
    assert_eq!(l.entry_vendor(fx.posted), Some(v));
    assert_eq!(l.account_balance(fx.cash).unwrap(), cash_before);
    l.set_entry_party(fx.draft, Some(p)).unwrap();
    assert_eq!(l.entry_vendor(fx.draft), Some(v));
    l.set_entry_party(fx.posted, None).unwrap();
    assert_eq!(l.entry_party(fx.posted), None);
}

/// Oracle: set_entry_party doc ("as set_entry_vendor does the other way
/// round") and the task's "set_entry_vendor now also tags the party".
#[test]
fn set_entry_vendor_tags_the_party() {
    let (mut l, fx) = fixture();
    let v = l.add_vendor("Registrar").unwrap();
    let p = l.party_of_vendor(v).unwrap();
    l.set_entry_vendor(fx.posted, Some(v)).unwrap();
    assert_eq!(l.entry_vendor(fx.posted), Some(v));
    assert_eq!(l.entry_party(fx.posted), Some(p));
}

/// Oracle: docs "an entry has one counterparty": retagging an entry with a
/// party that has no vendor record leaves no other party's vendor tag.
#[test]
fn retagging_to_a_party_without_vendor_drops_the_old_vendor() {
    let (mut l, fx) = fixture();
    let v = l.add_vendor("Old Vendor").unwrap();
    let irs = l.add_party("IRS", &[Role::TaxAgency]).unwrap();
    l.set_entry_vendor(fx.posted, Some(v)).unwrap();
    l.set_entry_party(fx.posted, Some(irs)).unwrap();
    assert_eq!(l.entry_party(fx.posted), Some(irs));
    assert_eq!(l.entry_vendor(fx.posted), None);
}

/// Oracle: set_entry_vendor SCENARIO 1 and "refusals change nothing".
#[test]
fn entry_tag_refusals() {
    let (mut l, fx) = fixture();
    let p = l.add_party("Someone", &[]).unwrap();
    let missing_entry = l.next_entry_id() + 9;
    let missing_party = l.next_party_id() + 9;
    refused(&mut l, "unknown entry", |l| {
        l.set_entry_party(missing_entry, Some(p))
    });
    refused(&mut l, "unknown party", |l| {
        l.set_entry_party(fx.posted, Some(missing_party))
    });
    refused(&mut l, "unknown vendor", |l| {
        l.set_entry_vendor(fx.posted, Some(9999))
    });
    refused(&mut l, "unknown entry (vendor)", |l| {
        l.set_entry_vendor(missing_entry, None)
    });
}

// ---------------------------------------------------------------- payroll

/// Oracle: validate_payroll: "a line naming an employee needs a party with
/// the employee role"; payroll.rs: runs from before carry the name only.
#[test]
fn payroll_employee_must_have_the_employee_role() {
    let (mut l, fx) = fixture();
    let alice = l.add_party("Alice", &[Role::Employee]).unwrap();
    let bob = l.add_party("Bob", &[Role::Contractor]).unwrap();
    let unknown = l.next_party_id() + 4;
    let day = date(2026, 2, 27);
    assert!(
        l.validate_payroll(day, &[pay_line("Alice", Some(alice))], fx.cash)
            .is_ok()
    );
    assert!(
        l.validate_payroll(day, &[pay_line("Old", None)], fx.cash)
            .is_ok()
    );
    assert!(
        l.validate_payroll(day, &[pay_line("Bob", Some(bob))], fx.cash)
            .is_err()
    );
    assert!(
        l.validate_payroll(day, &[pay_line("Ghost", Some(unknown))], fx.cash)
            .is_err()
    );
    assert!(
        l.validate_payroll(
            day,
            &[pay_line("Alice", Some(alice)), pay_line("Bob", Some(bob))],
            fx.cash
        )
        .is_err()
    );
    refused(&mut l, "run with non-employee", |l| {
        l.run_payroll(day, vec![pay_line("Bob", Some(bob))], fx.cash)
    });
    let run = l
        .run_payroll(day, vec![pay_line("Alice", Some(alice))], fx.cash)
        .unwrap();
    let rec = l.get_payroll_run(run).unwrap();
    assert_eq!(rec.lines[0].employee, Some(alice));
}

// ---------------------------------------------------------------- invariants

/// INVARIANTS 1–4 plus the vendor/party tag sync, over the whole ledger.
fn check_invariants(l: &Ledger) {
    let parties: Vec<&Party> = l.parties().collect();
    // 1. No full TIN in any party field; tin_last4 four digits.
    for p in &parties {
        let mut texts = vec![p.name.clone()];
        texts.extend(p.display_name.clone());
        texts.extend(p.website.clone());
        texts.extend(p.address.clone());
        texts.extend(p.notes.clone());
        if let Some(f) = &p.w9 {
            texts.push(f.entity_type.clone());
            texts.push(f.kept_at.clone());
            texts.extend(f.exempt_payee_code.clone());
            texts.extend(f.fatca_code.clone());
            assert!(
                f.tin_last4.len() == 4 && f.tin_last4.chars().all(|c| c.is_ascii_digit()),
                "INV1 tin_last4 {:?}",
                f.tin_last4
            );
        }
        if let Some(f) = &p.w4 {
            texts.push(f.state.clone());
            texts.push(f.kept_at.clone());
        }
        for t in texts {
            assert!(!has_full_tin(&t), "INV1: party {} holds {t:?}", p.id);
        }
        // 2.
        if p.w9.is_some() {
            assert!(p.roles.iter().any(|r| r.gets_1099()), "INV2 w9 on {}", p.id);
        }
        if p.w4.is_some() {
            assert!(p.roles.contains(&Role::Employee), "INV2 w4 on {}", p.id);
        }
    }
    // 3.
    let mut live: BTreeMap<String, Vec<&Party>> = BTreeMap::new();
    for p in parties.iter().filter(|p| !p.archived) {
        live.entry(name_key(&p.name)).or_default().push(p);
    }
    for (k, group) in &live {
        assert!(
            group.len() == 1,
            "INV3: live name {k:?} shared by {:?}",
            group.iter().map(|p| p.id).collect::<Vec<_>>()
        );
    }
    // 4.
    for v in l.list_vendors() {
        let owners: Vec<PartyId> = parties
            .iter()
            .filter(|p| p.vendor == Some(v.id))
            .map(|p| p.id)
            .collect();
        assert_eq!(owners.len(), 1, "INV4 vendor {}", v.id);
        assert_eq!(l.party_of_vendor(v.id), Some(owners[0]));
    }
    for c in l.list_customers() {
        let owners: Vec<PartyId> = parties
            .iter()
            .filter(|p| p.customer == Some(c.id))
            .map(|p| p.id)
            .collect();
        assert_eq!(owners.len(), 1, "INV4 customer {}", c.id);
        assert_eq!(l.party_of_customer(c.id), Some(owners[0]));
    }
    // Tag sync: an entry's vendor tag belongs to its party.
    for e in 0..l.next_entry_id() {
        if let Some(v) = l.entry_vendor(e) {
            assert_eq!(
                l.entry_party(e),
                l.party_of_vendor(v),
                "entry {e}: vendor tag and party tag disagree"
            );
        }
    }
}

// ---------------------------------------------------------------- proptest

const NAMES: &[&str] = &[
    "Acme",
    " acme ",
    "ACME",
    "Beta",
    "beta",
    "Gamma",
    "",
    "  ",
    "Tin 123456789",
    "Delta",
];

/// Indices into NAMES used for vendor/customer records (all of them).
const RECORD_NAMES: &[usize] = &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9];

#[derive(Clone, Debug)]
enum Cmd {
    AddParty(usize, Vec<Role>),
    Update(usize, PartyField),
    Role(usize, Role, bool),
    W9(usize, Option<W9Facts>),
    W4(usize, Option<W4Facts>),
    EntryParty(usize, Option<usize>),
    EntryVendor(usize, Option<usize>),
    Vendor(usize),
    Customer(usize),
}

fn role_strategy() -> impl Strategy<Value = Role> {
    prop::sample::select(Role::ALL.to_vec())
}

fn text_strategy() -> impl Strategy<Value = Option<String>> {
    prop::option::of(prop::sample::select(vec![
        "ok".to_string(),
        "SSN 123-45-6789".to_string(),
        "x".repeat(501),
        "bell\u{7}".to_string(),
        "Suite 100".to_string(),
    ]))
}

fn field_strategy() -> impl Strategy<Value = PartyField> {
    prop_oneof![
        (0..NAMES.len()).prop_map(|i| PartyField::Name(NAMES[i].to_string())),
        text_strategy().prop_map(PartyField::DisplayName),
        text_strategy().prop_map(PartyField::Website),
        text_strategy().prop_map(PartyField::Address),
        text_strategy().prop_map(PartyField::Notes),
        any::<bool>().prop_map(PartyField::Archived),
    ]
}

fn cmd_strategy() -> impl Strategy<Value = Cmd> {
    prop_oneof![
        (0..NAMES.len(), prop::collection::vec(role_strategy(), 0..3))
            .prop_map(|(n, r)| Cmd::AddParty(n, r)),
        (0..8usize, field_strategy()).prop_map(|(p, f)| Cmd::Update(p, f)),
        (0..8usize, role_strategy(), any::<bool>()).prop_map(|(p, r, on)| Cmd::Role(p, r, on)),
        (
            0..8usize,
            prop::option::of(prop::sample::select(vec!["6789", "67", "123456789"]))
        )
            .prop_map(|(p, f)| Cmd::W9(p, f.map(w9))),
        (
            0..8usize,
            prop::option::of(prop::sample::select(vec!["UT", "Utah"]))
        )
            .prop_map(|(p, f)| Cmd::W4(p, f.map(w4))),
        (0..3usize, prop::option::of(0..8usize)).prop_map(|(e, p)| Cmd::EntryParty(e, p)),
        (0..3usize, prop::option::of(0..5usize)).prop_map(|(e, v)| Cmd::EntryVendor(e, v)),
        prop::sample::select(RECORD_NAMES.to_vec()).prop_map(Cmd::Vendor),
        prop::sample::select(RECORD_NAMES.to_vec()).prop_map(Cmd::Customer),
    ]
}

/// Apply `cmd`, returning whether it was accepted. Party and vendor
/// indices pick among existing ids (or one past them, to hit "unknown").
fn apply(l: &mut Ledger, fx: &Fx, cmd: &Cmd) -> bool {
    let party_ids: Vec<PartyId> = l.parties().map(|p| p.id).collect();
    let pick_party = |i: usize| -> PartyId {
        party_ids
            .get(i)
            .copied()
            .unwrap_or_else(|| l.next_party_id() + i as u64)
    };
    let vendor_ids: Vec<u64> = l.list_vendors().iter().map(|v| v.id).collect();
    let pick_vendor = |i: usize| -> u64 { vendor_ids.get(i).copied().unwrap_or(10_000 + i as u64) };
    let entries = [fx.posted, fx.draft, 9_999];
    match cmd {
        Cmd::AddParty(n, roles) => l.add_party(NAMES[*n], roles).is_ok(),
        Cmd::Update(p, f) => l.update_party(pick_party(*p), f.clone()).is_ok(),
        Cmd::Role(p, r, on) => l.set_party_role(pick_party(*p), *r, *on).is_ok(),
        Cmd::W9(p, f) => l.set_party_w9(pick_party(*p), f.clone()).is_ok(),
        Cmd::W4(p, f) => l.set_party_w4(pick_party(*p), f.clone()).is_ok(),
        Cmd::EntryParty(e, p) => l.set_entry_party(entries[*e], p.map(pick_party)).is_ok(),
        Cmd::EntryVendor(e, v) => l.set_entry_vendor(entries[*e], v.map(pick_vendor)).is_ok(),
        Cmd::Vendor(n) => l.add_vendor(NAMES[*n]).is_ok(),
        Cmd::Customer(n) => l.add_customer(NAMES[*n]).is_ok(),
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Oracle: INVARIANTS 1–4 hold after every accepted command; every
    /// refusal "changes nothing" (the add/update/role SCENARIOS).
    #[test]
    fn random_commands_keep_the_invariants(cmds in prop::collection::vec(cmd_strategy(), 1..40)) {
        let (mut l, fx) = fixture();
        for cmd in &cmds {
            let before = l.clone();
            let ok = apply(&mut l, &fx, cmd);
            if ok {
                check_invariants(&l);
            } else {
                prop_assert!(l == before, "refused {:?} changed the ledger", cmd);
            }
        }
    }
}

/// Oracle: W9Facts.vault_record "SHA-256 of the vault record": a hash is
/// hex, and hex can hold nine digits in a row; that is not a TIN.
#[test]
fn a_vault_record_hash_with_nine_digits_is_accepted() {
    let facts = W9Facts {
        received: chrono::NaiveDate::from_ymd_opt(2026, 10, 15).unwrap(),
        entity_type: "individual/sole proprietor".into(),
        exempt_payee_code: None,
        fatca_code: None,
        tin_last4: "6789".into(),
        kept_at: "wanderbooks vault".into(),
        vault_record: Some(format!("123456789{}", "a".repeat(55))),
    };
    assert_eq!(facts.check(), Ok(()));
}
