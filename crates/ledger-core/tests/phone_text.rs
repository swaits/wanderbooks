//! A telephone or fax number is kept, and a tax id's printed shapes are not
//! (BS-partner-2, WB-partner-15; docs/spec/claims/partner.md, the CHOICE on
//! BS-partner-2 as amended 2026-11-21).
//!
//! Oracle: what a phone number is. It is ten digits or more with a country
//! and an area code, so the strict reading of a party's fields (any run of
//! nine digits) refused every phone number there was; a phone field
//! refuses only what reads as a Social Security number (3-2-4) or an EIN
//! (2-7). A postal address likewise keeps a ZIP+4, which is nine digits and
//! no tax id. The fake ids are `000-00-0000` and `00-0000000`; no real one
//! appears here.

use ledger_core::arap::{DocumentLine, InvoiceFields, PartySnapshot};
use ledger_core::parties::{check_address, check_phone, has_full_tin, has_full_tin_in_address};
use ledger_core::{Currency, Ledger, LedgerError, PartyField};
use proptest::prelude::*;
use rust_decimal::Decimal;

/// What a phone number looks like in the world.
const PHONES: &[&str] = &[
    "2065550142",
    "206-555-0142",
    "206.555.0142",
    "206 555 0142",
    "(206) 555-0142",
    "(206)555-0142",
    "+1 206 555 0142",
    "+1 (206) 555-0142",
    "+1-206-555-0142",
    "1-800-555-0142",
    "+44 20 7946 0958",
    "+44 (0)20 7946 0958",
    "020 7946 0958",
    "+49 30 12345678",
    "+33 1 23 45 67 89",
    "+81 3-1234-5678",
    "+91 98765 43210",
    "612345678",
    "206-555-0142 x1234",
    "206-555-0142 ext. 12",
    "(206) 555-0142, ext 7",
    "503-555-0100",
    "555-0142",
    "",
];

const SSN: &str = "000-00-0000";
const EIN: &str = "00-0000000";

#[test]
fn an_ordinary_phone_number_is_kept() {
    for p in PHONES {
        assert_eq!(check_phone("phone", p), Ok(()), "{p:?}");
    }
}

#[test]
fn an_ssn_or_an_ein_in_a_phone_field_is_refused_in_words_that_never_say_the_number() {
    for t in [
        SSN,
        EIN,
        "000 00 0000",
        "000.00.0000",
        "ssn 000-00-0000",
        "206-555-0142 / 000-00-0000",
        "fax 00-0000000",
        "\u{ff10}\u{ff10}\u{ff10}-\u{ff10}\u{ff10}-\u{ff10}\u{ff10}\u{ff10}\u{ff10}",
    ] {
        let why = check_phone("the bill-to phone", t).unwrap_err();
        assert!(
            why.starts_with("the bill-to phone looks like it holds a tax id"),
            "{t:?}: {why}"
        );
        assert!(!why.contains("0000"), "{why}");
    }
}

#[test]
fn a_phone_field_keeps_a_phone_a_party_s_other_fields_would_refuse() {
    // The strict reading of the other party fields still refuses these.
    for p in ["206-555-0142", "+1 206 555 0142", "2065550142", "612345678"] {
        assert!(has_full_tin(p), "{p}: strict");
        assert!(check_phone("phone", p).is_ok(), "{p}: phone");
    }
}

#[test]
fn a_zip_plus_4_is_kept_in_an_address_and_a_tax_id_is_not() {
    for a in [
        "98101-1234",
        "400 Pine St\nSeattle, WA 98101-1234",
        "400 Pine St, Seattle WA 98101-1234, USA",
        "Suite 100, 84101",
        "PO Box 12, Salt Lake City UT 84101-0012",
        "98101-1234\n84047-1234",
    ] {
        assert_eq!(check_address("address", a), Ok(()), "{a:?}");
    }
    for a in [
        SSN,
        EIN,
        "400 Pine St 123456789",
        "123 456 789",
        "98101-12345",     // six digits after: nine, then one more
        "198101-1234",     // six before
        "98101-1234 5678", // a longer run
        "98101 1234",      // not a ZIP+4's hyphen
        "400 Pine St 000-00-0000, Seattle",
    ] {
        assert!(check_address("address", a).is_err(), "{a:?}");
        assert!(has_full_tin_in_address(a), "{a:?}");
    }
}

/// A phone field and an address on the party, through the engine.
#[test]
fn a_party_keeps_any_phone_and_a_zip_plus_4_and_stays_strict_elsewhere() {
    let mut l = Ledger::new(Currency::USD);
    let party = l.add_party("Harbor Coffee", &[]).unwrap();
    for p in PHONES {
        l.update_party(party, PartyField::Phone(Some((*p).to_string())))
            .unwrap();
    }
    l.update_party(
        party,
        PartyField::BillingAddress(Some("1 Main St, Seattle WA 98101-1234".into())),
    )
    .unwrap();
    l.update_party(
        party,
        PartyField::ShippingAddress(Some("2 Dock Rd, Seattle WA 98101-1234".into())),
    )
    .unwrap();
    l.update_party(
        party,
        PartyField::Address(Some("3 Pier St, Seattle WA 98101-1234".into())),
    )
    .unwrap();
    for bad in [SSN, EIN] {
        let before = l.clone();
        let r = l.update_party(party, PartyField::Phone(Some(bad.into())));
        assert!(matches!(r, Err(LedgerError::InvalidParty(_))), "{r:?}");
        assert_eq!(l, before, "a refusal changes nothing");
    }
    // Every other field of a party is still strict.
    for f in [
        PartyField::Contact(Some("Ana 123456789".into())),
        PartyField::Notes(Some("tax id 123456789".into())),
        PartyField::Address(Some("1 Main St 123456789".into())),
        PartyField::Email(Some("123456789@example.com".into())),
    ] {
        let r = l.update_party(party, f.clone());
        assert!(
            matches!(r, Err(LedgerError::InvalidParty(_))),
            "{f:?}: {r:?}"
        );
    }
}

fn snap(phone: &str) -> PartySnapshot {
    PartySnapshot {
        name: "Harbor Coffee Roasters, Inc.".into(),
        address: Some("455 Wharf Street\nPortland, OR 97209-1234".into()),
        email: None,
        phone: Some(phone.into()),
        contact: None,
        billing_address: None,
        shipping_address: None,
        billing_email: None,
    }
}

fn invoice_with(
    l: &mut Ledger,
    bill_to: PartySnapshot,
    seller: PartySnapshot,
) -> Result<(), LedgerError> {
    let c = l.add_customer("Harbor Coffee").unwrap();
    let d = chrono::NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
    l.create_invoice_with(
        c,
        d,
        d,
        vec![DocumentLine {
            description: "Consulting".into(),
            amount: Decimal::from(10),
            class_id: None,
            account: None,
            quantity: None,
            unit_price: None,
        }],
        InvoiceFields {
            bill_to: Some(bill_to),
            seller: Some(seller),
            ..Default::default()
        },
    )
    .map(|_| ())
}

/// The invoice's bill-to and seller snapshots, through the engine.
#[test]
fn an_invoice_prints_any_phone_and_refuses_a_tax_id_in_one() {
    for p in PHONES.iter().filter(|p| !p.is_empty()) {
        let mut l = Ledger::new(Currency::USD);
        let r = invoice_with(&mut l, snap(p), snap(p));
        assert!(r.is_ok(), "{p:?}: {r:?}");
    }
    for bad in [SSN, EIN] {
        let mut l = Ledger::new(Currency::USD);
        let r = invoice_with(&mut l, snap(bad), snap("206-555-0142"));
        assert!(
            matches!(r, Err(LedgerError::InvalidParty(_))),
            "{bad}: {r:?}"
        );
        let mut l = Ledger::new(Currency::USD);
        let r = invoice_with(&mut l, snap("206-555-0142"), snap(bad));
        assert!(
            matches!(r, Err(LedgerError::FreeTextHoldsTin(_))),
            "{bad}: {r:?}"
        );
    }
}

proptest! {
    /// Any phone built of a country code, an area code and a subscriber
    /// number, in any of the usual groupings, is kept unless it is itself
    /// an SSN's 3-2-4 or an EIN's 2-7 (which a phone's groups never are).
    #[test]
    fn a_phone_of_any_usual_grouping_is_kept(
        cc in "[1-9][0-9]{0,2}",
        area in "[2-9][0-9]{2}",
        ex in "[2-9][0-9]{2}",
        sub in "[0-9]{4}",
        sep in prop::sample::select(vec!["-", " ", ".", ""]),
        shape in 0usize..4,
    ) {
        let p = match shape {
            0 => format!("+{cc} {area}{sep}{ex}{sep}{sub}"),
            1 => format!("({area}) {ex}{sep}{sub}"),
            2 => format!("{area}{sep}{ex}{sep}{sub} x{cc}"),
            _ => format!("+{cc}-{area}{ex}{sub}"),
        };
        prop_assert!(check_phone("phone", &p).is_ok(), "{p}");
    }

    /// An SSN's 3-2-4 and an EIN's 2-7, with any separator, are refused
    /// in a phone field, alone or among words.
    #[test]
    fn a_printed_ssn_or_ein_is_refused_in_a_phone_field(
        a in "[0-9]{3}", b in "[0-9]{2}", c in "[0-9]{4}",
        e in "[0-9]{2}", f in "[0-9]{7}",
        sep in prop::sample::select(vec!["-", " ", "\u{2013}", " - "]),
        pre in prop::sample::select(vec!["", "ssn ", "(", "fax: "]),
        post in prop::sample::select(vec!["", " x12", ")", " thanks"]),
    ) {
        let ssn = format!("{pre}{a}{sep}{b}{sep}{c}{post}");
        let ein = format!("{pre}{e}{sep}{f}{post}");
        prop_assert!(check_phone("phone", &ssn).is_err(), "{ssn}");
        prop_assert!(check_phone("phone", &ein).is_err(), "{ein}");
    }

    /// A ZIP+4 anywhere in an address is kept.
    #[test]
    fn a_zip_plus_4_is_kept_wherever_it_stands(
        z in "[0-9]{5}", p in "[0-9]{4}",
        pre in prop::sample::select(vec!["", "Seattle, WA ", "1 Main St\nSeattle WA ", "Unit 4, "]),
        post in prop::sample::select(vec!["", ", USA", "\nUSA"]),
    ) {
        let a = format!("{pre}{z}-{p}{post}");
        prop_assert!(check_address("address", &a).is_ok(), "{a}");
    }
}

/// The formation state's entity number is a registration number: a
/// Washington UBI is nine digits, so a bare nine digits is kept, and a
/// printed SSN or EIN is not (every other text setting is free text).
#[test]
fn the_state_entity_number_keeps_nine_digits_and_refuses_a_printed_tax_id() {
    let key = ledger_core::settings::key("company.state_entity_number").unwrap();
    for ok in ["603123456", "603 123 456", "14745117-0160", "5012345"] {
        assert!(
            ledger_core::settings::check(key, &serde_json::json!(ok)).is_ok(),
            "{ok}"
        );
    }
    for bad in [SSN, EIN] {
        assert!(ledger_core::settings::check(key, &serde_json::json!(bad)).is_err());
    }
    // Another text setting stays free text: a bare nine digits is refused.
    let agent = ledger_core::settings::key("company.registered_agent").unwrap();
    assert!(ledger_core::settings::check(agent, &serde_json::json!("123456789")).is_err());
}
