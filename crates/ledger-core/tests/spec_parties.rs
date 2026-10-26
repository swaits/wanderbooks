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
use ledger_core::parties::{check_text, has_full_tin, name_key, redact_full_tins};
use ledger_core::{
    AccountId, AccountType, Currency, EntryId, Ledger, Money, Party, PartyField, PartyId,
    PartyShare, PayrollLine, Role, Side, W4Facts, W9Facts,
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
        email: None,
        default_terms_as_customer: None,
        default_terms_as_vendor: None,
        address: None,
        notes: None,
        archived: false,
        w9: None,
        w4: None,
        tin: None,
        vendor: None,
        customer: None,
        creditor_of: BTreeSet::new(),
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

/// Oracle: `has_full_tin` doc ("a run of nine digits, with at most three
/// separators (space, hyphen, dot, slash, underscore, a Unicode dash or
/// no-break space) between neighbors, a run of spaces counting once; or a
/// TIN's 3-2-4 or 2-7 groups with commas or parentheses between them") and
/// INVARIANT 1.
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
        ("12  3456789", true), // two spaces
        ("12--3456789", true), // two hyphens
        ("12 -3456789", true), // space and hyphen between neighbors
        ("241.40.4915", true), // dots
        ("wrong one, was 241.40.4915", true),
        ("291/12/2846", true),               // slashes
        ("291_12_2846", true),               // underscores
        ("Bob 291 - 12 - 2846", true),       // three separators
        ("291\u{2013}12\u{2013}2846", true), // en dashes
        ("291\u{a0}12\u{a0}2846", true),     // no-break spaces
        ("12    3456789", true),             // a run of spaces counts once
        ("291, 12, 2846", true),             // commas between a TIN's groups
        ("291,12,2846", true),
        ("12,3456789", true),    // an EIN's 2-7 groups
        ("(291) 12-2846", true), // parentheses around a group
        ("12\t3456789", false),  // a tab is not a space
        ("12345678", false),     // eight digits
        ("123-45-678", false),
        ("1234", false),
        ("", false),
        ("Acme LLC", false),
        ("Suite 100, 84101", false), // 3 + 5 digits, separated by ", "
        ("1,234,567.89", false),     // an amount: comma groups of three
        ("123,456,789", false),      // a comma-grouped number, not a TIN
        ("(123) 456-7890", false),   // a phone number: 3-3-4
        ("-123456789-", true),
    ];
    for (text, want) in cases {
        assert_eq!(has_full_tin(text), *want, "has_full_tin({text:?})");
    }
}

/// Oracle: `redact_full_tins` doc: every run `has_full_tin` finds has its
/// digits masked, separators kept; other text is untouched.
#[test]
fn redact_full_tins_masks_every_full_tin() {
    assert_eq!(
        redact_full_tins("unknown party \"291-12-2846\""),
        "unknown party \"•••-••-••••\""
    );
    assert_eq!(
        redact_full_tins("was 241.40.4915 and 12-3456789, call 801-555"),
        "was •••.••.•••• and ••-•••••••, call 801-555"
    );
    assert_eq!(
        redact_full_tins("invalid type: integer `291122846`"),
        "invalid type: integer `•••••••••`"
    );
    assert_eq!(redact_full_tins("party 12 of 3"), "party 12 of 3");
    for t in ["291 - 12 - 2846", "２９１１２２８４６", "1234567890"] {
        assert!(!has_full_tin(&redact_full_tins(t)), "{t}");
    }
}

/// Oracle: `check_text` refuses what `has_full_tin` finds, so a purge
/// reason or note quoting the old number with dots is refused (B-26: no
/// party field may hold a nine-digit number).
#[test]
fn check_text_refuses_a_dotted_tin() {
    assert!(check_text("purged", "wrong one, was 241.40.4915").is_err());
    assert!(check_text("no_party_reason", "ssn 291.12.2846").is_err());
    assert!(check_text("name", "Bob 291 - 12 - 2846").is_err());
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

/// Oracle: `has_full_tin` doc (P3 review round 2; P7): past ASCII, every
/// form of the five separators Unicode has separates (every Zs space, every
/// Dash character, the compatibility forms of the dot, slash and
/// underscore; `docs/spec/sources/unicode-tin-text.md`, Unicode 17.0),
/// and invisible characters (Default_Ignorable_Code_Point) don't count
/// toward the gap. A run of spaces counts once, however long, and a comma
/// or a parenthesis separates a TIN's 3-2-4 or 2-7 groups (ASCII only).
/// A TIN typed in fullwidth forms, Arabic-Indic digits, with an en dash, a
/// non-breaking hyphen, a Hebrew maqaf, any space, a comma or a
/// parenthesis, or with right-to-left marks around its hyphens is still a
/// full TIN; line breaks, commas in an ungrouped run, tabs and a fourth
/// other visible separator still end a run. (Before P3, "１２３－４５－６７８９"
/// passed: the fullwidth hyphen-minus wasn't a separator, nor the
/// ideographic or thin space.)
#[test]
fn has_full_tin_reads_every_script_and_separator() {
    let tins = [
        // Fullwidth digits and hyphen-minus.
        "\u{FF11}\u{FF12}\u{FF13}\u{FF0D}\u{FF14}\u{FF15}\u{FF0D}\u{FF16}\u{FF17}\u{FF18}\u{FF19}",
        "291\u{FF0D}12\u{FF0D}2846", // fullwidth hyphen-minus
        "291\u{FE63}12\u{FE63}2846", // small hyphen-minus
        // Arabic-Indic, Extended Arabic-Indic, Devanagari (en dashes).
        "\u{661}\u{662}\u{663}-\u{664}\u{665}-\u{666}\u{667}\u{668}\u{669}",
        "\u{6F1}\u{6F2}\u{6F3}\u{6F4}\u{6F5}\u{6F6}\u{6F7}\u{6F8}\u{6F9}",
        "\u{967}\u{968}\u{969}\u{2013}\u{96A}\u{96B}\u{2013}\u{96C}\u{96D}\u{96E}\u{96F}",
        "291\u{2013}12\u{2013}2846",                 // en dash
        "291\u{2011}12\u{2011}2846",                 // non-breaking hyphen
        "291\u{2012}12\u{2012}2846",                 // figure dash
        "291\u{2015}12\u{2015}2846",                 // horizontal bar
        "291\u{5BE}12\u{5BE}2846",                   // Hebrew maqaf
        "291\u{301C}12\u{301C}2846",                 // wave dash
        "291\u{A0}12\u{A0}2846",                     // no-break space
        "291\u{3000}12\u{3000}2846",                 // ideographic space
        "291\u{2009}12\u{2009}2846",                 // thin space
        "291\u{202F}12\u{202F}2846",                 // narrow no-break space
        "291 \u{2014} 12 \u{2014} 2846",             // three: space, em dash, space
        "12\u{3000}\u{3000}\u{3000}\u{3000}3456789", // a run of spaces counts once
        "291\u{FF0E}12\u{FF0E}2846",                 // fullwidth full stop
        "291\u{2024}12\u{2024}2846",                 // one dot leader
        "291\u{FF0F}12\u{FF0F}2846",                 // fullwidth solidus
        "291\u{FF3F}12\u{FF3F}2846",                 // fullwidth low line
        "291,12,2846",                               // commas between the groups
        "291, 12, 2846",                             // a comma and a space
        "(291) 12-2846",                             // parentheses around a group
        "12,3456789",                                // an EIN's groups
        // Zero-width spaces between every digit.
        "2\u{200B}9\u{200B}1\u{200B}1\u{200B}2\u{200B}2\u{200B}8\u{200B}4\u{200B}6",
        "291\u{200F}-\u{200F}12\u{200F}-\u{200F}2846", // right-to-left marks around hyphens
        "291\u{200E}\u{200E}\u{200E}\u{200E} 12 2846", // invisible marks don't count
        "291\u{AD}12\u{AD}2846",                       // soft hyphens
        "291\u{2060}122846",                           // word joiner
        "\u{FEFF}291122846",
    ];
    for t in tins {
        assert!(has_full_tin(t), "has_full_tin({t:?})");
        let masked = redact_full_tins(t);
        assert!(!has_full_tin(&masked), "{t:?} masked: {masked:?}");
        assert_eq!(
            masked.chars().count(),
            t.chars().count(),
            "only digits change: {masked:?}"
        );
    }
    assert_eq!(
        redact_full_tins(
            "ref \u{FF11}\u{FF12}\u{FF13}\u{FF0D}\u{FF14}\u{FF15}\u{FF0D}\u{FF16}\u{FF17}\u{FF18}\u{FF19}"
        ),
        "ref •••\u{FF0D}••\u{FF0D}••••"
    );
    assert_eq!(
        redact_full_tins("291\u{200F}-\u{200F}12-2846 ok"),
        "•••\u{200F}-\u{200F}••-•••• ok"
    );
    let not = [
        "12 \u{2014} \u{200B}-3456789", // four visible, an invisible one among them
        "291\u{2028}12\u{2028}2846",    // line separators end a run
        "291\u{2029}12\u{2029}2846",    // paragraph separators
        "291\u{85}12\u{85}2846",        // next line
        "291\u{FF0C}12\u{FF0C}2846",    // fullwidth commas are not a group's comma
        "291\u{3001}12\u{3001}2846",    // ideographic commas either
        "1,234,567.89",                 // an amount: comma groups of three
        "(123) 456-7890",               // a phone number: 3-3-4
        "\u{FF11}\u{FF12}\u{FF13}\u{FF14}\u{FF15}\u{FF16}\u{FF17}\u{FF18}", // eight digits
        "291\u{200B}12284",             // eight, invisibly joined
    ];
    for t in not {
        assert!(!has_full_tin(t), "has_full_tin({t:?})");
    }
}

/// The reading before P3 review round 2, the oracle for what P7 must
/// never narrow: P7 only widens what reads as a TIN (commas and
/// parentheses between a TIN's groups, any run of spaces counting once).
fn has_full_tin_before(text: &str) -> bool {
    let sep = |c: char| {
        matches!(
            c,
            ' ' | '-' | '.' | '/' | '_' | '\u{a0}' | '\u{2010}'..='\u{2014}' | '\u{2212}'
        )
    };
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if !chars[i].is_numeric() {
            i += 1;
            continue;
        }
        let (mut digits, mut last, mut j) = (1, i, i + 1);
        while j < chars.len() {
            if chars[j].is_numeric() {
                digits += 1;
                last = j;
                j += 1;
                continue;
            }
            let gap = chars[j..].iter().take_while(|c| sep(**c)).count();
            if (1..=3).contains(&gap) && chars.get(j + gap).is_some_and(|c| c.is_numeric()) {
                j += gap;
            } else {
                break;
            }
        }
        if digits >= 9 {
            return true;
        }
        i = last + 1;
    }
    false
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    /// What `has_full_tin` found before P7 it still finds: the fix only
    /// widens the reading (commas and parentheses between a TIN's groups,
    /// any run of spaces counting once), never narrows it.
    #[test]
    fn has_full_tin_never_forgets_a_tin(
        any in "[0-9 ./_\t\n,a\u{A0}\u{2010}-\u{2015}\u{2212}\u{FF0D}\u{3000}\u{200B}\u{200F}\u{FF10}-\u{FF19}\u{660}-\u{669}\u{2028}-]{0,40}",
    ) {
        if has_full_tin_before(&any) {
            prop_assert!(has_full_tin(&any), "{:?}", any);
        }
    }
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
    assert_eq!(all.len(), 10, "ten distinct roles");
    for r in [
        Role::Vendor,
        Role::Contractor,
        Role::Customer,
        Role::Employee,
        Role::Member,
        Role::PayoutPlatform,
        Role::TaxAgency,
        Role::GovernmentAgency,
        Role::FinancialInstitution,
        Role::Landlord,
    ] {
        assert!(all.contains(&r));
        assert_eq!(Role::parse(r.key()), Some(r));
    }
    assert_eq!(Role::PayoutPlatform.key(), "payout_platform");
    assert_eq!(Role::TaxAgency.key(), "tax_agency");
    assert_eq!(Role::GovernmentAgency.key(), "government_agency");
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
    let p = l.add_party("Mo", &[]).unwrap();
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

/// Oracle: name_payroll_parties doc: a payroll entry names the employees
/// it paid as its parties, each with their net pay (one employee: the whole
/// entry), live and on replay alike; a line naming no employee leaves the
/// entry untagged; a line paying nothing names no one.
#[test]
fn a_payroll_entry_names_the_employees_it_paid() {
    let (mut l, fx) = fixture();
    let alice = l.add_party("Alice", &[Role::Employee]).unwrap();
    let bob = l.add_party("Bob", &[Role::Employee]).unwrap();
    let day = date(2026, 2, 27);
    let net = d("823.50");
    let run = l
        .run_payroll(day, vec![pay_line("Alice", Some(alice))], fx.cash)
        .unwrap();
    let entry = l.get_payroll_run(run).unwrap().entry_id;
    assert_eq!(l.entry_parties(entry), &[PartyShare::whole(alice)]);
    // Alice twice (her two lines one share) and Bob: exact amounts.
    let lines = vec![
        pay_line("Alice", Some(alice)),
        pay_line("Bob", Some(bob)),
        pay_line("Alice", Some(alice)),
    ];
    let run = l.run_payroll(day, lines.clone(), fx.cash).unwrap();
    let entry = l.get_payroll_run(run).unwrap().entry_id;
    let amounts = |a: Decimal, b: Decimal| {
        vec![
            PartyShare {
                party: alice,
                share: ledger_core::Share::Amount(a),
            },
            PartyShare {
                party: bob,
                share: ledger_core::Share::Amount(b),
            },
        ]
    };
    assert_eq!(l.entry_parties(entry), amounts(net + net, net).as_slice());
    // The same run replayed names them the same way.
    let replayed = l.create_entry(day, "Payroll replayed");
    let hosting = l.chart().find_by_code("6050").unwrap().id;
    l.add_line(replayed, hosting, Side::Debit, usd("1.00"))
        .unwrap();
    l.add_line(replayed, fx.cash, Side::Credit, usd("1.00"))
        .unwrap();
    l.post_entry(replayed).unwrap();
    let id = l.next_payroll_run_id();
    l.replay_payroll_run(id, day, lines, replayed).unwrap();
    assert_eq!(
        l.entry_parties(replayed),
        amounts(net + net, net).as_slice()
    );
    // A line naming no employee: the entry is left for the agent to tag.
    let run = l
        .run_payroll(
            day,
            vec![pay_line("Alice", Some(alice)), pay_line("Old", None)],
            fx.cash,
        )
        .unwrap();
    let entry = l.get_payroll_run(run).unwrap().entry_id;
    assert!(l.entry_parties(entry).is_empty());
    // A line that pays nothing (all withheld) names no one, so Bob's run
    // is his alone.
    let mut nothing = pay_line("Old", None);
    nothing.federal_withholding = d("923.50");
    let run = l
        .run_payroll(day, vec![nothing, pay_line("Bob", Some(bob))], fx.cash)
        .unwrap();
    let entry = l.get_payroll_run(run).unwrap().entry_id;
    assert_eq!(l.entry_parties(entry), &[PartyShare::whole(bob)]);
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

/// Oracle: parties::entry_flow DEFINITIONS (across): one liability paying
/// down two (a member paying both cards from their own funds): across is
/// taken from the debited liabilities in order of first appearance, each
/// at most what is left of it, until across is used up.
#[test]
fn entry_flow_takes_across_in_order_until_used_up() {
    use ledger_core::parties::{entry_flow, money_accounts};
    use ledger_core::{AccountType, Currency, Ledger, Money, Side};
    let mut l = Ledger::new(Currency::USD);
    let mut acct = |code: &str, name: &str, t| l.add_account(code, name, t, None, false).unwrap();
    let bank = acct("1000", "Bank", AccountType::Asset);
    let card_a = acct("2050", "Card A", AccountType::Liability);
    let card_b = acct("2060", "Card B", AccountType::Liability);
    let members = acct("2100", "Due to Members", AccountType::Liability);
    let capital = acct("3000", "Capital", AccountType::Equity);
    let fees = acct("6000", "Fees", AccountType::Expense);
    let money = money_accounts(&l, &[bank], &[members]);
    let e = l.create_entry(NaiveDate::from_ymd_opt(2026, 5, 1).unwrap(), "x");
    for (a, side, amt) in [
        (card_a, Side::Debit, "30.00"),
        (card_b, Side::Debit, "20.00"),
        (fees, Side::Debit, "5.00"),
        (members, Side::Credit, "25.00"),
        (bank, Side::Credit, "15.00"),
        (capital, Side::Credit, "15.00"),
    ] {
        l.add_line(e, a, side, Money::new(d(amt), Currency::USD))
            .unwrap();
    }
    let f = entry_flow(l.get_entry(e).unwrap(), &money).unwrap();
    // down = min(liab_dr 50, cash_cr 15) = 15: card A's first 15. Left:
    // card A 15, card B 20, members -25; across = min(35, 25) = 25: card
    // A's 15, then 10 of card B's 20, and nothing more.
    assert_eq!(f.net, d("30.00"));
    assert_eq!(
        f.what_for,
        vec![
            (card_a, d("30.00")),
            (fees, d("5.00")),
            (capital, d("-15.00")),
            (card_b, d("10.00"))
        ]
    );
}

/// Oracle: shares::target DEFINITIONS, case 3 (rules.md B-35): members
/// named on an entry moving no money split what moved equity, E (here
/// −4.00 of a draft not yet balanced), received, in kind (the equipment's
/// 10.00, at most |E|); case 5: with a party named who isn't a member,
/// the total debits, direction neither, counting nothing.
#[test]
fn a_share_target_without_money_is_the_capital_or_the_debits() {
    use ledger_core::parties::money_accounts;
    use ledger_core::shares::{Direction, Target, target};
    use ledger_core::{AccountType, Currency, Ledger, Money, Side};
    let mut l = Ledger::new(Currency::USD);
    let bank = l
        .add_account("1000", "Bank", AccountType::Asset, None, false)
        .unwrap();
    let equip = l
        .add_account("1500", "Equipment", AccountType::Asset, None, false)
        .unwrap();
    let capital = l
        .add_account("3000", "Capital", AccountType::Equity, None, false)
        .unwrap();
    let member = l.add_party("Tobin Harlow", &[Role::Member]).unwrap();
    let vendor = l.add_party("Riley Chen", &[Role::Contractor]).unwrap();
    let money = money_accounts(&l, &[bank], &[]);
    let e = l.create_entry(NaiveDate::from_ymd_opt(2026, 5, 1).unwrap(), "x");
    l.add_line(e, equip, Side::Debit, Money::new(d("10.00"), Currency::USD))
        .unwrap();
    l.add_line(
        e,
        capital,
        Side::Credit,
        Money::new(d("4.00"), Currency::USD),
    )
    .unwrap();
    assert_eq!(
        target(&l, e, &[PartyShare::whole(member)], &money),
        Ok(Target {
            amount: d("-4.00"),
            direction: Direction::Received,
            cash: Decimal::ZERO,
            in_kind: d("-4.00"),
            counts_as: Direction::Received,
        })
    );
    let both = [PartyShare::whole(member), PartyShare::whole(vendor)];
    assert_eq!(
        target(&l, e, &both, &money),
        Ok(Target {
            amount: d("10.00"),
            direction: Direction::Neither,
            cash: Decimal::ZERO,
            in_kind: Decimal::ZERO,
            counts_as: Direction::Neither,
        })
    );
}

/// Oracle: shares::resolve INVARIANT 10: members named on an entry that
/// touches their capital share it by their capital lines, credits less
/// debits.
#[test]
fn derived_member_shares_net_their_capital_lines() {
    use ledger_core::parties::money_accounts;
    use ledger_core::{AccountType, Currency, Ledger, Money, Side};
    let mut l = Ledger::new(Currency::USD);
    let mut acct = |code: &str, t| l.add_account(code, code, t, None, false).unwrap();
    let bank = acct("1000", AccountType::Asset);
    let cap_a = acct("3000", AccountType::Equity);
    let cap_b = acct("3001", AccountType::Equity);
    let a = l.add_party("A", &[Role::Member]).unwrap();
    let b = l.add_party("B", &[Role::Member]).unwrap();
    let money = money_accounts(&l, &[bank], &[]);
    let member = |party, capital| ledger_core::PartyMember {
        party,
        share: d("0.5"),
        capital,
        distributions: None,
    };
    let members = [member(a, cap_a), member(b, cap_b)];
    let e = l.create_entry(NaiveDate::from_ymd_opt(2026, 5, 1).unwrap(), "x");
    for (acct, side, amt) in [
        (bank, Side::Debit, "100.00"),
        (cap_a, Side::Credit, "100.00"),
        (cap_a, Side::Debit, "30.00"),
        (cap_b, Side::Credit, "30.00"),
    ] {
        l.add_line(e, acct, side, Money::new(d(amt), Currency::USD))
            .unwrap();
    }
    let shares = [PartyShare::whole(a), PartyShare::whole(b)].map(|mut s| {
        s.share = ledger_core::Share::Derived;
        s
    });
    let r = ledger_core::shares::resolve(&l, e, &shares, &money, &members).unwrap();
    assert_eq!(
        r.parts
            .iter()
            .map(|p| (p.party, p.amount))
            .collect::<Vec<_>>(),
        vec![(a, d("-70.00")), (b, d("-30.00"))]
    );
}

/// Oracle: parties::entry_flow SCENARIOS 1–6, worked by hand; what_for
/// always sums to net.
#[test]
fn entry_flow_scenarios() {
    use ledger_core::parties::{entry_flow, money_accounts};
    use ledger_core::{AccountType, Currency, Ledger, Money, Side};
    use rust_decimal::Decimal;
    let mut l = Ledger::new(Currency::USD);
    let mut acct = |code: &str, name: &str, t| l.add_account(code, name, t, None, false).unwrap();
    let bank = acct("1000", "Bank", AccountType::Asset);
    let savings = acct("1010", "Savings", AccountType::Asset);
    let equip = acct("1500", "Equipment", AccountType::Asset);
    let card = acct("2050", "Business Card", AccountType::Liability);
    let members = acct("2100", "Due to Members", AccountType::Liability);
    let loan = acct("2500", "Loan", AccountType::Liability);
    let accrued = acct("2200", "Accrued Expenses", AccountType::Liability);
    let capital = acct("3000", "Capital", AccountType::Equity);
    let exp = acct("6000", "Expenses", AccountType::Expense);
    let money = money_accounts(&l, &[bank, savings], &[members, loan]);
    let d = chrono::NaiveDate::from_ymd_opt(2026, 5, 1).unwrap();
    let mut case = |lines: &[(u64, Side, &str)]| {
        let e = l.create_entry(d, "x");
        for (a, side, amt) in lines {
            l.add_line(
                e,
                *a,
                *side,
                Money::new(amt.parse().unwrap(), Currency::USD),
            )
            .unwrap();
        }
        let f = entry_flow(l.get_entry(e).unwrap(), &money).unwrap();
        let sum: Decimal = f.what_for.iter().map(|(_, v)| *v).sum();
        assert_eq!(sum, f.net, "what_for sums to net: {f:?}");
        f
    };
    use Side::{Credit as C, Debit as D};
    let dec = |s: &str| s.parse::<Decimal>().unwrap();
    // 1. Expense from the bank, a card, a member.
    for src in [bank, card, members] {
        let f = case(&[(exp, D, "10.00"), (src, C, "10.00")]);
        assert_eq!(f.net, dec("10.00"));
        assert_eq!(f.what_for, vec![(exp, dec("10.00"))]);
    }
    // 2. Reimbursing members; paying the card.
    for liab in [members, card] {
        let f = case(&[(liab, D, "412.30"), (bank, C, "412.30")]);
        assert_eq!(f.net, dec("412.30"));
        assert_eq!(f.what_for, vec![(liab, dec("412.30"))]);
    }
    // 3. A loan.
    let f = case(&[(bank, D, "1000.00"), (loan, C, "1000.00")]);
    assert_eq!(f.net, dec("-1000.00"));
    // 4. Refunds to the bank and to the card.
    assert_eq!(
        case(&[(bank, D, "5.00"), (exp, C, "5.00")]).net,
        dec("-5.00")
    );
    assert_eq!(
        case(&[(card, D, "5.00"), (exp, C, "5.00")]).net,
        dec("-5.00")
    );
    // 5. Own-account transfer.
    let f = case(&[(savings, D, "50.00"), (bank, C, "50.00")]);
    assert_eq!(f.net, Decimal::ZERO);
    assert!(f.what_for.is_empty());
    // 6. Property contributed.
    assert_eq!(
        case(&[(equip, D, "750.00"), (capital, C, "750.00")]).net,
        Decimal::ZERO
    );
    // Split payment: part bank, part card.
    let f = case(&[(exp, D, "100.00"), (bank, C, "60.00"), (card, C, "40.00")]);
    assert_eq!(f.net, dec("100.00"));
    // 7. A member pays the card bill from their own funds, or a loan pays
    // it off: the issuer is paid, for the card, as from the bank.
    for funder in [members, loan] {
        let f = case(&[(card, D, "300.00"), (funder, C, "300.00")]);
        assert_eq!(f.net, dec("300.00"));
        assert_eq!(f.what_for, vec![(card, dec("300.00"))]);
    }
    // Part bank, part a member.
    let f = case(&[
        (card, D, "300.00"),
        (bank, C, "200.00"),
        (members, C, "100.00"),
    ]);
    assert_eq!(f.net, dec("300.00"));
    assert_eq!(f.what_for, vec![(card, dec("300.00"))]);
    // 4. A charge and its refund on one card offset there: no card paid
    // down, 80.00 paid by card.
    let f = case(&[
        (exp, D, "100.00"),
        (card, C, "100.00"),
        (card, D, "20.00"),
        (exp, C, "20.00"),
    ]);
    assert_eq!(f.net, dec("80.00"));
    assert_eq!(f.what_for, vec![(exp, dec("80.00"))]);
    // Paying the card from the bank while the issuer charges a fee on it:
    // 300.00 cash plus the 50.00 fee by card.
    let f = case(&[
        (card, D, "300.00"),
        (bank, C, "300.00"),
        (exp, D, "50.00"),
        (card, C, "50.00"),
    ]);
    assert_eq!(f.net, dec("350.00"));
    // 8 (defect 24). An accrual through a liability that isn't money
    // moves none; its settlement is the payment, counted once.
    let f = case(&[(exp, D, "40.00"), (accrued, C, "40.00")]);
    assert_eq!(f.net, Decimal::ZERO);
    let f = case(&[(accrued, D, "40.00"), (bank, C, "40.00")]);
    assert_eq!(f.net, dec("40.00"));
    assert_eq!(f.what_for, vec![(accrued, dec("40.00"))]);
}

// ------------------------------------------------------- party shares
//
// Oracles: the spec block of `ledger_core::shares` (DEFINITIONS,
// INVARIANTS 1–10, SCENARIOS 1–5), `Ledger::set_entry_parties` (INVARIANTS
// 1–4), `parties::activity_with` (INVARIANTS 1–4), and
// docs/parties-and-attachments.md "Shares". Values worked by hand.

mod shares {
    use super::{d, date, refused, usd};
    use ledger_core::parties::{activity_with, money_accounts};
    use ledger_core::shares::{check_shares, resolve};
    use ledger_core::{
        AccountId, AccountType, Currency, EntryId, Ledger, PartyId, PartyMember, PartyShare, Role,
        Share, Side,
    };
    use proptest::prelude::*;
    use rust_decimal::Decimal;

    struct Books {
        l: Ledger,
        bank: AccountId,
        due: AccountId,
        cap_s: AccountId,
        cap_k: AccountId,
        equip: AccountId,
        exp: AccountId,
        tobin: PartyId,
        mo: PartyId,
        acme: PartyId,
        members: Vec<PartyMember>,
    }

    /// Quillmere 1000, Due to Shareholders 2100, capital 3000 and 3001,
    /// equipment 1500, an expense 6050; members Tobin and Morgan at
    /// 50/50; a party Acme (no records).
    fn books() -> Books {
        let mut l = Ledger::new(Currency::USD);
        let add = |l: &mut Ledger, c: &str, t| l.add_account(c, c, t, None, false).unwrap();
        let bank = add(&mut l, "1000", AccountType::Asset);
        let equip = add(&mut l, "1500", AccountType::Asset);
        let due = add(&mut l, "2100", AccountType::Liability);
        let cap_s = add(&mut l, "3000", AccountType::Equity);
        let cap_k = add(&mut l, "3001", AccountType::Equity);
        let exp = add(&mut l, "6050", AccountType::Expense);
        let tobin = l.add_party("Tobin Harlow", &[Role::Member]).unwrap();
        let mo = l.add_party("Morgan Ellis", &[Role::Member]).unwrap();
        let acme = l.add_party("Acme", &[Role::Contractor]).unwrap();
        let half = d("0.5");
        let members = vec![
            PartyMember {
                party: tobin,
                share: half,
                capital: cap_s,
                distributions: None,
            },
            PartyMember {
                party: mo,
                share: half,
                capital: cap_k,
                distributions: None,
            },
        ];
        Books {
            l,
            bank,
            due,
            cap_s,
            cap_k,
            equip,
            exp,
            tobin,
            mo,
            acme,
            members,
        }
    }

    fn post(l: &mut Ledger, lines: &[(AccountId, Side, &str)]) -> EntryId {
        let e = l.try_create_entry(date(2026, 5, 1), "joint").unwrap();
        for (a, s, v) in lines {
            l.add_line(e, *a, *s, usd(v)).unwrap();
        }
        l.post_entry(e).unwrap();
        e
    }

    fn derived(ps: &[PartyId]) -> Vec<PartyShare> {
        ps.iter().map(|&p| PartyShare::whole(p)).collect()
    }

    fn amounts(ps: &[(PartyId, &str)]) -> Vec<PartyShare> {
        ps.iter()
            .map(|&(party, v)| PartyShare {
                party,
                share: Share::Amount(d(v)),
            })
            .collect()
    }

    fn percents(ps: &[(PartyId, &str)]) -> Vec<PartyShare> {
        ps.iter()
            .map(|&(party, v)| PartyShare {
                party,
                share: Share::Percent(d(v)),
            })
            .collect()
    }

    /// The money accounts: the bank, and 2100 as the members' financing
    /// (`books.financing_accounts`).
    fn money(b: &Books) -> ledger_core::parties::MoneyAccounts {
        money_accounts(&b.l, &[b.bank], &[b.due])
    }

    fn resolved(b: &Books, e: EntryId) -> Result<Vec<(PartyId, Decimal)>, String> {
        resolve(&b.l, e, b.l.entry_parties(e), &money(b), &b.members)
            .map(|r| r.parts.iter().map(|p| (p.party, p.amount)).collect())
    }

    fn year() -> (chrono::NaiveDate, chrono::NaiveDate) {
        (date(2026, 1, 1), date(2026, 12, 31))
    }

    /// (paid, received) on basis cash.
    fn paid_received(b: &Books, p: PartyId) -> (Decimal, Decimal) {
        let a = activity_with(&b.l, p, year(), &money(b), &b.members);
        (a.paid, a.received)
    }

    /// (paid, received) on basis all.
    fn paid_received_all(b: &Books, p: PartyId) -> (Decimal, Decimal) {
        activity_with(&b.l, p, year(), &money(b), &b.members)
            .totals(ledger_core::parties::Basis::All)
    }

    /// SCENARIO 1: a reimbursement to both members, derived, 50/50.
    #[test]
    fn reimbursing_both_members_splits_by_member_share() {
        let mut b = books();
        let e = post(
            &mut b.l,
            &[
                (b.due, Side::Debit, "100.00"),
                (b.bank, Side::Credit, "100.00"),
            ],
        );
        b.l.set_entry_parties(e, &derived(&[b.tobin, b.mo]))
            .unwrap();
        assert_eq!(
            resolved(&b, e).unwrap(),
            vec![(b.tobin, d("50.00")), (b.mo, d("50.00"))]
        );
        assert_eq!(b.l.entry_party(e), None, "two parties: no sole party");
        assert!(b.l.entry_names_party(e, b.mo));
        assert_eq!(paid_received(&b, b.tobin), (d("50.00"), Decimal::ZERO));
        assert_eq!(paid_received(&b, b.mo), (d("50.00"), Decimal::ZERO));
    }

    /// SCENARIO 2: a joint contribution derives from the capital lines.
    #[test]
    fn joint_contribution_derives_from_capital_lines() {
        let mut b = books();
        let e = post(
            &mut b.l,
            &[
                (b.bank, Side::Debit, "300.00"),
                (b.cap_s, Side::Credit, "200.00"),
                (b.cap_k, Side::Credit, "100.00"),
            ],
        );
        b.l.set_entry_parties(e, &derived(&[b.tobin, b.mo]))
            .unwrap();
        assert_eq!(
            resolved(&b, e).unwrap(),
            vec![(b.tobin, d("-200.00")), (b.mo, d("-100.00"))]
        );
        assert_eq!(paid_received(&b, b.tobin), (Decimal::ZERO, d("200.00")));
        assert_eq!(paid_received(&b, b.mo), (Decimal::ZERO, d("100.00")));
    }

    /// SCENARIO 3 (defect 14, rules.md B-35): property contributed moves
    /// no money; each member's part is what moved their capital, signed
    /// as their own account shows it: received, in kind. Basis cash
    /// counts nothing; basis all counts it. (Before P3 this read +500.00
    /// each, the sign of a payment.)
    #[test]
    fn property_contribution_resolves_against_the_total() {
        let mut b = books();
        let e = post(
            &mut b.l,
            &[
                (b.equip, Side::Debit, "1000.00"),
                (b.cap_s, Side::Credit, "500.00"),
                (b.cap_k, Side::Credit, "500.00"),
            ],
        );
        b.l.set_entry_parties(e, &derived(&[b.tobin, b.mo]))
            .unwrap();
        assert_eq!(
            resolved(&b, e).unwrap(),
            vec![(b.tobin, d("-500.00")), (b.mo, d("-500.00"))]
        );
        let r = resolve(&b.l, e, b.l.entry_parties(e), &money(&b), &b.members).unwrap();
        assert_eq!(r.target.direction, ledger_core::shares::Direction::Received);
        assert!(
            r.parts
                .iter()
                .all(|p| p.cash.is_zero() && p.in_kind == d("-500.00"))
        );
        assert_eq!(paid_received(&b, b.tobin), (Decimal::ZERO, Decimal::ZERO));
        assert_eq!(paid_received_all(&b, b.tobin), (Decimal::ZERO, d("500.00")));
        assert_eq!(paid_received_all(&b, b.mo), (Decimal::ZERO, d("500.00")));
    }

    /// INVARIANT 10: capital lines that don't account for the whole
    /// movement of equity (here another equity account's line) are
    /// refused, and the entry is listed as unresolved.
    #[test]
    fn capital_lines_short_of_the_target_are_refused() {
        let mut b = books();
        let other =
            b.l.add_account("3500", "Opening Equity", AccountType::Equity, None, false)
                .unwrap();
        let e = post(
            &mut b.l,
            &[
                (b.bank, Side::Debit, "300.00"),
                (b.cap_s, Side::Credit, "200.00"),
                (other, Side::Credit, "100.00"),
            ],
        );
        b.l.set_entry_parties(e, &derived(&[b.tobin, b.mo]))
            .unwrap();
        assert!(resolved(&b, e).is_err());
        let a = activity_with(&b.l, b.tobin, year(), &money(&b), &b.members);
        assert!(a.unresolved.contains(&e));
        assert_eq!((a.paid, a.received), (Decimal::ZERO, Decimal::ZERO));
    }

    /// rules.md B-35 item 3 (capital first): a members' entry counts what
    /// moved their capital, not other money on it: 300.00 in, 200.00 of it
    /// Tobin's capital and 100.00 an expense refunded, is Tobin's 200.00
    /// received (cash) and nothing of Morgan's. (Before P3 the 300.00 net
    /// was the target and the capital lines fell short of it: refused.)
    #[test]
    fn a_members_entry_counts_what_moved_their_capital() {
        let mut b = books();
        let e = post(
            &mut b.l,
            &[
                (b.bank, Side::Debit, "300.00"),
                (b.cap_s, Side::Credit, "200.00"),
                (b.exp, Side::Credit, "100.00"),
            ],
        );
        b.l.set_entry_parties(e, &derived(&[b.tobin, b.mo]))
            .unwrap();
        assert_eq!(
            resolved(&b, e).unwrap(),
            vec![(b.tobin, d("-200.00")), (b.mo, Decimal::ZERO)]
        );
        assert_eq!(paid_received(&b, b.tobin), (Decimal::ZERO, d("200.00")));
        assert_eq!(paid_received(&b, b.mo), (Decimal::ZERO, Decimal::ZERO));
    }

    /// SCENARIO 4: percent rounding, remainder to the first party.
    #[test]
    fn percent_rounding_gives_the_remainder_to_the_first() {
        let mut b = books();
        let c = b.l.add_party("Third", &[]).unwrap();
        let e = post(
            &mut b.l,
            &[
                (b.exp, Side::Debit, "100.00"),
                (b.bank, Side::Credit, "100.00"),
            ],
        );
        b.l.set_entry_parties(
            e,
            &percents(&[(b.acme, "33.33"), (b.tobin, "33.33"), (c, "33.34")]),
        )
        .unwrap();
        assert_eq!(
            resolved(&b, e).unwrap(),
            vec![(b.acme, d("33.33")), (b.tobin, d("33.33")), (c, d("33.34"))]
        );
        b.l.set_entry_parties(
            e,
            &percents(&[(b.acme, "33.333"), (b.tobin, "33.333"), (c, "33.334")]),
        )
        .unwrap();
        assert_eq!(
            resolved(&b, e).unwrap(),
            vec![(b.acme, d("33.34")), (b.tobin, d("33.33")), (c, d("33.33"))]
        );
    }

    /// SCENARIO 5 and INVARIANT 7: amounts must sum to what moved.
    #[test]
    fn amounts_must_sum_to_the_flow() {
        let mut b = books();
        let e = post(
            &mut b.l,
            &[
                (b.exp, Side::Debit, "100.00"),
                (b.bank, Side::Credit, "100.00"),
            ],
        );
        b.l.set_entry_parties(e, &amounts(&[(b.tobin, "60.00"), (b.mo, "30.00")]))
            .unwrap();
        assert!(resolved(&b, e).is_err());
        b.l.set_entry_parties(e, &amounts(&[(b.tobin, "70.00"), (b.mo, "30.00")]))
            .unwrap();
        assert_eq!(
            resolved(&b, e).unwrap(),
            vec![(b.tobin, d("70.00")), (b.mo, d("30.00"))]
        );
        assert_eq!(paid_received(&b, b.tobin), (d("70.00"), Decimal::ZERO));
    }

    /// INVARIANT 10: several derived shares need members.
    #[test]
    fn derived_needs_members_for_several_parties() {
        let mut b = books();
        let e = post(
            &mut b.l,
            &[
                (b.exp, Side::Debit, "10.00"),
                (b.bank, Side::Credit, "10.00"),
            ],
        );
        b.l.set_entry_parties(e, &derived(&[b.tobin, b.acme]))
            .unwrap();
        assert!(resolved(&b, e).is_err());
        // With no members passed (plain activity), two members can't derive.
        b.l.set_entry_parties(e, &derived(&[b.tobin, b.mo]))
            .unwrap();
        assert!(resolve(&b.l, e, b.l.entry_parties(e), &money(&b), &[]).is_err());
        // A sole party needs no members.
        b.l.set_entry_parties(e, &derived(&[b.acme])).unwrap();
        assert_eq!(resolved(&b, e).unwrap(), vec![(b.acme, d("10.00"))]);
    }

    /// INVARIANT 10: income shares rescale to the members named.
    #[test]
    fn member_shares_rescale_to_those_named() {
        let mut b = books();
        let third = b.l.add_party("Third Member", &[Role::Member]).unwrap();
        let cap_t =
            b.l.add_account("3002", "Capital T", AccountType::Equity, None, false)
                .unwrap();
        b.members[0].share = d("0.5");
        b.members[1].share = d("0.3");
        b.members.push(PartyMember {
            party: third,
            share: d("0.2"),
            capital: cap_t,
            distributions: None,
        });
        let e = post(
            &mut b.l,
            &[
                (b.due, Side::Debit, "80.00"),
                (b.bank, Side::Credit, "80.00"),
            ],
        );
        b.l.set_entry_parties(e, &derived(&[b.tobin, b.mo]))
            .unwrap();
        assert_eq!(
            resolved(&b, e).unwrap(),
            vec![(b.tobin, d("50.00")), (b.mo, d("30.00"))]
        );
    }

    /// INVARIANTS 1–5 of shares and set_entry_parties 1: refusals change
    /// nothing.
    #[test]
    fn bad_shares_are_refused_changing_nothing() {
        let mut b = books();
        let e = post(
            &mut b.l,
            &[
                (b.exp, Side::Debit, "10.00"),
                (b.bank, Side::Credit, "10.00"),
            ],
        );
        let (s, k) = (b.tobin, b.mo);
        refused(&mut b.l, "duplicate", |l| {
            l.set_entry_parties(e, &derived(&[s, s]))
        });
        refused(&mut b.l, "unknown party", |l| {
            l.set_entry_parties(e, &derived(&[s, 999]))
        });
        refused(&mut b.l, "unknown entry", |l| {
            l.set_entry_parties(999, &derived(&[s]))
        });
        refused(&mut b.l, "mixed kinds", |l| {
            l.set_entry_parties(
                e,
                &[
                    PartyShare {
                        party: s,
                        share: Share::Amount(d("5.00")),
                    },
                    PartyShare::whole(k),
                ],
            )
        });
        refused(&mut b.l, "percents sum to 99", |l| {
            l.set_entry_parties(e, &percents(&[(s, "50"), (k, "49")]))
        });
        refused(&mut b.l, "percent over 100", |l| {
            l.set_entry_parties(e, &percents(&[(s, "101"), (k, "-1")]))
        });
        refused(&mut b.l, "finer than a cent", |l| {
            l.set_entry_parties(e, &amounts(&[(s, "5.005"), (k, "4.995")]))
        });
        refused(&mut b.l, "zero amount", |l| {
            l.set_entry_parties(e, &amounts(&[(s, "10.00"), (k, "0")]))
        });
        let v1 = b.l.add_vendor("Vendor One").unwrap();
        let v2 = b.l.add_vendor("Vendor Two").unwrap();
        let (p1, p2) = (
            b.l.party_of_vendor(v1).unwrap(),
            b.l.party_of_vendor(v2).unwrap(),
        );
        refused(&mut b.l, "two vendors", |l| {
            l.set_entry_parties(e, &amounts(&[(p1, "5.00"), (p2, "5.00")]))
        });
        // The checker agrees on its own.
        assert!(check_shares(&percents(&[(s, "100")]), |_| true).is_ok());
        assert!(check_shares(&percents(&[(s, "60"), (k, "40.0")]), |_| true).is_ok());
    }

    /// set_entry_parties INVARIANT 2: the vendor tag follows the one party
    /// with a vendor record; clearing clears it.
    #[test]
    fn vendor_tag_follows_the_one_vendor_party() {
        let mut b = books();
        let e = post(
            &mut b.l,
            &[
                (b.exp, Side::Debit, "10.00"),
                (b.bank, Side::Credit, "10.00"),
            ],
        );
        let v = b.l.add_vendor("Vendor One").unwrap();
        let pv = b.l.party_of_vendor(v).unwrap();
        b.l.set_entry_parties(e, &amounts(&[(pv, "6.00"), (b.tobin, "4.00")]))
            .unwrap();
        assert_eq!(b.l.entry_vendor(e), Some(v));
        b.l.set_entry_parties(e, &derived(&[b.tobin, b.mo]))
            .unwrap();
        assert_eq!(b.l.entry_vendor(e), None);
        b.l.set_entry_parties(e, &[]).unwrap();
        assert!(b.l.entry_parties(e).is_empty());
    }

    /// Single-party tagging is one derived share; INVARIANT 3: naming a
    /// party clears the no-party mark, and the mark is refused while
    /// parties are named.
    #[test]
    fn single_party_and_no_party_mark() {
        let mut b = books();
        let e = post(
            &mut b.l,
            &[
                (b.exp, Side::Debit, "10.00"),
                (b.bank, Side::Credit, "10.00"),
            ],
        );
        b.l.set_entry_party(e, Some(b.acme)).unwrap();
        assert_eq!(b.l.entry_parties(e), &[PartyShare::whole(b.acme)]);
        assert_eq!(b.l.entry_party(e), Some(b.acme));
        b.l.set_entry_party(e, None).unwrap();
        b.l.set_entry_no_party(e, "own transfer").unwrap();
        b.l.set_entry_parties(e, &derived(&[b.tobin, b.mo]))
            .unwrap();
        assert_eq!(b.l.entry_no_party(e), None);
        refused(&mut b.l, "no-party while named", |l| {
            l.set_entry_no_party(e, "none")
        });
    }

    /// set_entry_no_party INVARIANT 1 and SCENARIO 3: a marked transfer's
    /// reversal carries the mark; marking a reversed entry marks the
    /// whole chain; naming a party on any of them clears it on all.
    #[test]
    fn a_no_party_mark_follows_the_reversal_chain() {
        let mut b = books();
        let own = |b: &mut Books| {
            let (due, bank) = (b.due, b.bank);
            post(
                &mut b.l,
                &[(due, Side::Debit, "10.00"), (bank, Side::Credit, "10.00")],
            )
        };
        let e = own(&mut b);
        b.l.set_entry_no_party(e, "transfer between our own accounts")
            .unwrap();
        let r = b.l.reverse_entry(e, date(2026, 5, 2), "undo").unwrap();
        assert_eq!(
            b.l.entry_no_party(r),
            Some("transfer between our own accounts")
        );
        // Marked after it was reversed: the whole chain.
        let e2 = own(&mut b);
        let r2 = b.l.reverse_entry(e2, date(2026, 5, 2), "undo").unwrap();
        b.l.set_entry_no_party(r2, "accrual").unwrap();
        assert_eq!(b.l.entry_no_party(e2), Some("accrual"));
        assert_eq!(b.l.entry_no_party(r2), Some("accrual"));
        // A party named on the original clears both.
        b.l.set_entry_party(e2, Some(b.acme)).unwrap();
        assert_eq!(b.l.entry_no_party(e2), None);
        assert_eq!(b.l.entry_no_party(r2), None);
        refused(&mut b.l, "chain names a party", |l| {
            l.set_entry_no_party(r2, "none")
        });
    }

    /// set_entry_vendor SCENARIO 3: tagging a vendor names its party and
    /// clears a no-party mark, so the entry never carries both; clearing
    /// the vendor afterwards leaves no stale mark to satisfy "require".
    #[test]
    fn tagging_a_vendor_clears_a_no_party_mark() {
        let mut b = books();
        let e = b.l.try_create_entry(date(2026, 5, 1), "hosting").unwrap();
        b.l.add_line(e, b.exp, Side::Debit, usd("12.00")).unwrap();
        b.l.add_line(e, b.bank, Side::Credit, usd("12.00")).unwrap();
        b.l.set_entry_no_party(e, "none really").unwrap();
        let v = b.l.add_vendor("Namecheap").unwrap();
        let pv = b.l.party_of_vendor(v).unwrap();
        b.l.set_entry_vendor(e, Some(v)).unwrap();
        assert_eq!(b.l.entry_parties(e), &[PartyShare::whole(pv)]);
        assert_eq!(b.l.entry_no_party(e), None);
        b.l.set_entry_vendor(e, None).unwrap();
        assert!(b.l.entry_parties(e).is_empty());
        assert_eq!(b.l.entry_no_party(e), None);
    }

    /// Reversals carry every share; the pair nets to zero for each party.
    #[test]
    fn reversal_carries_all_shares() {
        let mut b = books();
        let e = post(
            &mut b.l,
            &[
                (b.exp, Side::Debit, "90.00"),
                (b.bank, Side::Credit, "90.00"),
            ],
        );
        let shares = amounts(&[(b.tobin, "60.00"), (b.mo, "30.00")]);
        b.l.set_entry_parties(e, &shares).unwrap();
        let r = b.l.reverse_entry(e, date(2026, 5, 2), "undo").unwrap();
        assert_eq!(b.l.entry_parties(r), shares.as_slice());
        assert_eq!(
            resolved(&b, r).unwrap(),
            vec![(b.tobin, d("-60.00")), (b.mo, d("-30.00"))]
        );
        assert_eq!(paid_received(&b, b.tobin), (Decimal::ZERO, Decimal::ZERO));
        assert_eq!(paid_received(&b, b.mo), (Decimal::ZERO, Decimal::ZERO));
    }

    /// Plain `activity` is activity_with no members: a sole party still
    /// counts the whole net.
    #[test]
    fn activity_counts_a_sole_party_in_full() {
        let mut b = books();
        let e = post(
            &mut b.l,
            &[
                (b.exp, Side::Debit, "12.34"),
                (b.bank, Side::Credit, "12.34"),
            ],
        );
        b.l.set_entry_party(e, Some(b.acme)).unwrap();
        let a = ledger_core::parties::activity(&b.l, b.acme, year(), &money(&b));
        assert_eq!(a.paid, d("12.34"));
        assert!(a.entries.contains(&e));
    }

    /// activity_with INVARIANT 2: a reversal of a reversal reinstates the
    /// payment on its root original's side: paid, never a negative
    /// received.
    #[test]
    fn reversing_a_reversal_reinstates_the_payment_as_paid() {
        let mut b = books();
        let a = post(
            &mut b.l,
            &[
                (b.exp, Side::Debit, "100.00"),
                (b.bank, Side::Credit, "100.00"),
            ],
        );
        b.l.set_entry_party(a, Some(b.acme)).unwrap();
        let r1 = b.l.reverse_entry(a, date(2026, 5, 2), "undo").unwrap();
        let r2 = b.l.reverse_entry(r1, date(2026, 5, 3), "redo").unwrap();
        let act = ledger_core::parties::activity(&b.l, b.acme, year(), &money(&b));
        assert_eq!(act.entries, std::collections::BTreeSet::from([a, r1, r2]));
        assert_eq!((act.paid, act.received), (d("100.00"), Decimal::ZERO));
        // A third level takes it back out again, still on the paid side.
        let _r3 =
            b.l.reverse_entry(r2, date(2026, 5, 4), "undo again")
                .unwrap();
        let act = ledger_core::parties::activity(&b.l, b.acme, year(), &money(&b));
        assert_eq!((act.paid, act.received), (Decimal::ZERO, Decimal::ZERO));
    }

    fn cents() -> impl Strategy<Value = Decimal> {
        (1i64..10_000_000).prop_map(|c| Decimal::new(c, 2))
    }

    proptest! {
        /// INVARIANT 6: resolved shares sum to the target exactly, each
        /// with its sign, for percents (any split summing to 100, to
        /// three decimals), member-derived splits, and capital lines.
        #[test]
        fn resolved_shares_sum_exactly(
            total in cents(),
            refund in any::<bool>(),
            cuts in proptest::collection::vec(1u32..100_000, 1..6),
            member_share in 1u32..1000,
        ) {
            let mut b = books();
            let (dr, cr) = if refund { (b.bank, b.exp) } else { (b.exp, b.bank) };
            let t = total.to_string();
            let e = post(&mut b.l, &[(dr, Side::Debit, &t), (cr, Side::Credit, &t)]);
            let target = if refund { -total } else { total };
            // Percents: cut 100.000 at sorted points.
            let mut points: Vec<u32> = cuts.iter().map(|c| c % 100_000).collect();
            points.sort_unstable();
            points.dedup();
            points.retain(|&p| p > 0);
            let mut bounds = vec![0u32];
            bounds.extend(points);
            bounds.push(100_000);
            let mut ps = Vec::new();
            for (i, w) in bounds.windows(2).enumerate() {
                let party = b.l.add_party(&format!("P{i}"), &[]).unwrap();
                ps.push((party, Decimal::new(i64::from(w[1] - w[0]), 3)));
            }
            let shares: Vec<PartyShare> = ps
                .iter()
                .map(|&(party, p)| PartyShare { party, share: Share::Percent(p) })
                .collect();
            b.l.set_entry_parties(e, &shares).unwrap();
            let r = resolved(&b, e).unwrap();
            prop_assert_eq!(r.iter().map(|(_, v)| *v).sum::<Decimal>(), target);
            prop_assert!(r.iter().all(|(_, v)| v.is_zero() || v.is_sign_negative() == refund));
            prop_assert!(r.iter().all(|(_, v)| v.normalize().scale() <= 2));
            // Member-derived with uneven income shares.
            let s = Decimal::new(i64::from(member_share), 3);
            b.members[0].share = s;
            b.members[1].share = Decimal::ONE - s;
            b.l.set_entry_parties(e, &derived(&[b.tobin, b.mo])).unwrap();
            let r = resolved(&b, e).unwrap();
            prop_assert_eq!(r.iter().map(|(_, v)| *v).sum::<Decimal>(), target);
            // Amounts that sum: split at the first party's resolved part.
            let first = r[0].1.abs();
            let rest = total - first;
            if !first.is_zero() && !rest.is_zero() {
                b.l.set_entry_parties(e, &[
                    PartyShare { party: b.tobin, share: Share::Amount(first) },
                    PartyShare { party: b.mo, share: Share::Amount(rest) },
                ]).unwrap();
                let r = resolved(&b, e).unwrap();
                prop_assert_eq!(r.iter().map(|(_, v)| *v).sum::<Decimal>(), target);
            }
        }

        /// Capital-line derivation: two members contribute any amounts in
        /// one entry; each gets their own line, summing to the flow.
        #[test]
        fn capital_line_shares_sum_exactly(a in cents(), c in cents()) {
            let mut b = books();
            let total = a + c;
            let e = post(&mut b.l, &[
                (b.bank, Side::Debit, &total.to_string()),
                (b.cap_s, Side::Credit, &a.to_string()),
                (b.cap_k, Side::Credit, &c.to_string()),
            ]);
            b.l.set_entry_parties(e, &derived(&[b.tobin, b.mo])).unwrap();
            let r = resolved(&b, e).unwrap();
            prop_assert_eq!(r, vec![(b.tobin, -a), (b.mo, -c)]);
            let sa = activity_with(&b.l, b.tobin, year(), &money(&b), &b.members);
            let sk = activity_with(&b.l, b.mo, year(), &money(&b), &b.members);
            prop_assert_eq!(sa.received + sk.received, total);
        }
    }
}
