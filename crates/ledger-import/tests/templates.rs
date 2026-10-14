//! The starter charts, checked against the engine they'll run in
//! (templates.rs INVARIANTS 1-3).

use ledger_core::tax::{TaxForm, check_category};
use ledger_core::{
    AccountType, ControlAccount, Currency, GENERAL_EXPENSES, Ledger, PAYABLES, PAYROLL,
    RECEIVABLES, RETAINED_EARNINGS, REVENUE,
};
use ledger_import::{EntityType, chart_template};

fn ty(s: &str) -> AccountType {
    match s {
        "asset" => AccountType::Asset,
        "liability" => AccountType::Liability,
        "equity" => AccountType::Equity,
        "revenue" => AccountType::Revenue,
        "expense" => AccountType::Expense,
        other => panic!("bad type {other}"),
    }
}

const ENTITIES: [(EntityType, TaxForm); 4] = [
    (EntityType::SoleProprietorship, TaxForm::ScheduleC),
    (EntityType::Partnership, TaxForm::Form1065),
    (EntityType::SCorporation, TaxForm::Form1120S),
    (EntityType::CCorporation, TaxForm::Form1120),
];

/// INVARIANT 1: every template imports into fresh books, every tax
/// category valid for the entity's return.
#[test]
fn every_template_imports_with_categories_valid_for_its_return() {
    let owners = ["Ada".to_string(), "Grace".to_string(), "Linus".to_string()];
    for (entity, form) in ENTITIES {
        let t = chart_template(entity, &owners).unwrap();
        let mut l = Ledger::new(Currency::USD);
        for r in &t.rows {
            let parent = r
                .parent_code
                .as_ref()
                .map(|p| l.chart().find_by_code(p).expect("parent first").id);
            let id = l
                .add_account(&r.code, &r.name, ty(&r.account_type), parent, false)
                .unwrap_or_else(|e| panic!("{entity:?} {}: {e}", r.code));
            if let Some(cat) = &r.tax_category {
                check_category(cat, ty(&r.account_type), Some(form))
                    .unwrap_or_else(|e| panic!("{entity:?} {} {cat}: {e}", r.code));
                l.set_account_tax(id, Some(cat), None).unwrap();
            }
        }
    }
}

/// INVARIANT 2: an account at a control account's code is that control
/// account: same type and name (the engine posts by code).
#[test]
fn no_template_puts_another_account_at_a_control_code() {
    let controls: Vec<&ControlAccount> = [
        &RECEIVABLES,
        &REVENUE,
        &PAYABLES,
        &GENERAL_EXPENSES,
        &RETAINED_EARNINGS,
    ]
    .into_iter()
    .chain(PAYROLL.iter())
    .collect();
    let owners = ["Ada".to_string(), "Grace".to_string()];
    for (entity, _) in ENTITIES {
        let t = chart_template(entity, &owners).unwrap();
        for c in &controls {
            if let Some(r) = t.rows.iter().find(|r| r.code == c.code) {
                assert_eq!(ty(&r.account_type), c.account_type, "{entity:?} {}", c.code);
                // Revenue's control is named generically; a template may
                // call its 4000 "Sales". Every other control keeps its name.
                if c.code != REVENUE.code {
                    assert_eq!(r.name, c.name, "{entity:?} {}", c.code);
                }
            }
        }
    }
}

/// INVARIANT 3: a partnership has a capital and a distributions account
/// per partner, and shares summing to exactly 100%.
#[test]
fn a_partnership_template_names_every_partner_and_shares_sum_to_100() {
    let owners: Vec<String> = ["A", "B", "C"].iter().map(|s| s.to_string()).collect();
    let t = chart_template(EntityType::Partnership, &owners).unwrap();
    for name in &owners {
        assert!(t.rows.iter().any(|r| r.name == format!("Capital — {name}")));
        assert!(
            t.rows
                .iter()
                .any(|r| r.name == format!("Distributions — {name}"))
        );
    }
    let total: rust_decimal::Decimal = t
        .settings_toml
        .lines()
        .filter_map(|l| {
            l.strip_prefix("share = \"")
                .and_then(|s| s.strip_suffix("%\""))
        })
        .map(|s| s.parse::<rust_decimal::Decimal>().unwrap())
        .sum();
    assert_eq!(total, rust_decimal::Decimal::from(100));
}
