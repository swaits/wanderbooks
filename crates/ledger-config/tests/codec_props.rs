//! The settings file codec over settings built at random from valid and
//! refused changes (plan/served.md P1: toml_round_trip_zero_events).
//!
//! ```text
//! INVARIANTS (crate::codec):
//!   1. An export imports into the same books as no change at all.
//!   2. An export imports into empty books as exactly those settings,
//!      identity values aside (exported masked: they stay unset).
//!   3. The export reads, as a settings file, as the books do.
//! ```

use chrono::NaiveDate;
use ledger_config::codec;
use ledger_core::settings::{Change, Settings, Source};
use proptest::prelude::*;
use serde_json::{Value, json};

fn src() -> Source {
    Source {
        decided_by: "Pat Rivera".into(),
        decided_on: NaiveDate::from_ymd_opt(2026, 10, 17).unwrap(),
        citation: Some("Operating agreement §4.1\nsecond line".into()),
        note: None,
    }
}

fn candidates() -> Vec<(&'static str, Value)> {
    vec![
        ("company.name", json!("Acme \"Widgets\" LLC")),
        ("company.name", json!("Ünïcödé Holdings")),
        ("company.entity_type", json!("LLC, multi-member")),
        ("company.tax_classification", json!("S corporation")),
        ("company.tax_classification", json!("partnership")),
        ("company.tax_form", json!("1065")),
        ("company.fiscal_year_start", json!("07-01")),
        ("company.timezone", json!("America/Denver")),
        ("company.formation_state", json!("UT")),
        ("company.formed", json!("2026-10-04")),
        ("company.state_entity_number", json!("14745117-0160")),
        ("company.ein", json!("28-8525893")),
        ("company.irs_name_control", json!("waln")),
        ("company.duns", json!("149-976-972")),
        ("company.address", json!("1 Main St\nSuite 2, Midvale, UT")),
        ("company.registered_agent", json!("Pat Rivera")),
        ("company.partnership_representative", json!("Pat Rivera")),
        ("company.cpa", json!("Casey Rowe")),
        (
            "company.members",
            json!([{"name": "A", "share": "50%", "capital_account": "3000",
                    "distribution_account": "3100", "files_jointly_with": "B"},
                   {"name": "B", "share": "0.5", "capital_account": "3001",
                    "files_jointly_with": "A"}]),
        ),
        (
            "company.members",
            json!([{"name": "Solo", "share": "100%", "capital_account": "3000"}]),
        ),
        ("books.accounting_basis", json!("cash")),
        ("books.cash_accounts", json!(["1000", "1010"])),
        ("books.financing_accounts", json!(["2100"])),
        ("books.investing_accounts", json!(["1400"])),
        ("books.close_into", json!("3000")),
        ("books.close_sweep", json!(["3100"])),
        ("books.asset_tag_prefix", json!("FL")),
        ("books.schedule_convention", json!("monthly_even")),
        ("books.schedule_convention", json!("daily")),
        ("policy.evidence.threshold", json!("0.00")),
        ("policy.evidence.threshold", json!("25.5")),
        ("policy.evidence.required_accounts", json!(["6200"])),
        ("policy.evidence.exempt_accounts", json!(["6999", "6998"])),
        ("policy.parties", json!("require")),
        ("policy.approval_rules", json!("enforce")),
        // P4b S4.2: invoicing, bills and contact details.
        ("invoicing.number_prefix", json!("ACME-")),
        ("invoicing.first_number", json!(1000)),
        ("invoicing.default_terms", json!("2/10 net 30")),
        ("invoicing.default_terms", json!("Net 15 EOM")),
        (
            "invoicing.payment_methods",
            json!([{"kind": "ach", "label": "Bank transfer",
                    "lines": ["Routing 123456789", "Account 000123456789"]},
                   {"kind": "link", "label": "Card", "lines": [],
                    "link": "https://pay.example/acme", "qr": true}]),
        ),
        ("invoicing.footer", json!("Thank you for your business.")),
        ("invoicing.unsent_days", json!(2)),
        ("invoicing.reminder_days", json!(14)),
        ("bills.remind_days", json!(7)),
        ("bills.discount_warn_days", json!(3)),
        ("company.email", json!("billing@acme.example")),
        ("company.website", json!("https://acme.example")),
    ]
}

fn build(picks: &[(usize, bool)]) -> Settings {
    let pool = candidates();
    let mut s = Settings::default();
    for (seq, (i, clear)) in picks.iter().enumerate() {
        let (k, v) = &pool[i % pool.len()];
        let change = Change {
            key: (*k).into(),
            value: (!clear).then(|| v.clone()),
        };
        // Refused changes (settings that wouldn't hold together) are
        // skipped, as the books skip them.
        if let Ok(made) = s.check_changes(&[change], &src()) {
            s.apply(&made, &src(), seq as u64 + 1).unwrap();
        }
    }
    s
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn exports_round_trip(picks in proptest::collection::vec((0usize..64, prop::bool::weighted(0.15)), 0..30)) {
        let books = build(&picks);
        let text = codec::export(&books, "USD");
        // INVARIANT 3: the export reads as a settings file.
        let file = ledger_config::parse(&text)
            .unwrap_or_else(|e| panic!("the export reads as a settings file: {e}\n{text}"));
        prop_assert_eq!(&file.company.name, &books.company("USD").name);
        // INVARIANT 1.
        let same = codec::import(&text, None, &books, "USD")
            .unwrap_or_else(|e| panic!("{e}\n{text}"));
        prop_assert!(same.changes.is_empty(), "{:?}\n{}", same.changes, text);
        prop_assert!(same.unknown.is_empty(), "{:?}", same.unknown);
        // INVARIANT 2.
        let fresh = codec::import(&text, None, &Settings::default(), "USD")
            .unwrap_or_else(|e| panic!("{e}\n{text}"));
        let mut other = Settings::default();
        other.apply(&fresh.changes, &src(), 1).unwrap();
        for (k, r) in books.iter() {
            if k.name == "company.ein" {
                prop_assert!(other.get(k.name).is_none());
            } else {
                prop_assert_eq!(other.get(k.name).map(|o| &o.value), Some(&r.value), "{}", k.name);
            }
        }
        prop_assert_eq!(other.iter().count() + usize::from(books.get("company.ein").is_some()), books.iter().count());
    }
}
