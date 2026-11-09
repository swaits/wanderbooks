//! The ledger's small edits and their refusals (plan P14 unit V-D2):
//! renumbering an account, a party's fields, an asset's description and
//! disposal, a vendor's W-9 flag, the fiscal year under closed periods,
//! annotations and settlements of evidence. Each is a command whose
//! refusal arms the core's own suite did not reach.
//!
//! ```text
//! GOAL: an edit the books can stand is made; one they cannot is refused
//!       with its reason and changes nothing.
//! SOURCE: BS-ledger-19 (refuse rather than renumber); WB-ledger-10 (an
//!         account an approval rule names is not renumbered); BS-partner-8
//!         and WB-partner-4 (no full TIN in the books' text); BS-asset-1
//!         (disposal); BS-period-2 (closed periods) with the fiscal-year
//!         rule of settings.rs (IRS Form 1128); WB-ledger-15 (annotations
//!         and `entry.settles`).
//! ```

use chrono::NaiveDate;
use ledger_core::evidence::{Annotation, AnnotationFields};
use ledger_core::settings::{Change, Source};
use ledger_core::{
    AccountType, Currency, Ledger, LedgerError, Money, NewAsset, PartyField, Role, Side,
};
use rust_decimal::Decimal;
use serde_json::json;
use std::str::FromStr;

fn d(m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, m, day).unwrap()
}

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

fn usd(s: &str) -> Money {
    Money::new(dec(s), Currency::USD)
}

fn src() -> Source {
    Source {
        decided_by: "Owner".into(),
        decided_on: d(11, 9),
        citation: None,
        note: None,
    }
}

/// Run `f`, which must be refused; the ledger must be as it was.
fn refused<T: std::fmt::Debug>(
    l: &mut Ledger,
    f: impl FnOnce(&mut Ledger) -> Result<T, LedgerError>,
) -> LedgerError {
    let before = l.clone();
    let e = f(l).expect_err("the command should be refused");
    assert_eq!(*l, before, "a refusal changes nothing");
    e
}

// ---------------------------------------------------------------- renumbering

/// BS-ledger-19, WB-ledger-10: an account is renumbered to a code that is
/// free, not blank; not while an approval rule names its code (the rule
/// would cover nothing); not onto a control role's default code when its
/// type or placeholder-ness does not fit the role.
#[test]
fn an_account_is_renumbered_only_where_the_books_can_stand_it() {
    let mut l = Ledger::new(Currency::USD);
    let rent = l
        .add_account("6100", "Rent", AccountType::Expense, None, false)
        .unwrap();
    let bank = l
        .add_account("1000", "Bank", AccountType::Asset, None, false)
        .unwrap();

    assert_eq!(
        l.renumber_account(99, "7000").unwrap_err(),
        LedgerError::UnknownAccount(99)
    );
    assert_eq!(
        refused(&mut l, |l| l.renumber_account(rent, "  ")),
        LedgerError::EmptyAccountCode
    );
    assert_eq!(
        refused(&mut l, |l| l.renumber_account(rent, "1000")),
        LedgerError::DuplicateAccountCode("1000".into())
    );
    // 1200 is the receivables role's default code: an expense can't take it.
    assert!(matches!(
        refused(&mut l, |l| l.renumber_account(rent, "1200")),
        LedgerError::ControlAccountMismatch {
            expected: AccountType::Asset,
            found: AccountType::Expense,
            ..
        }
    ));

    // An approval rule names 6100: the account stays put until the rule is revised.
    let rule = ledger_core::conventions::NewConvention {
        id: "rent-approval".into(),
        kind: ledger_core::conventions::ConventionKind::ApprovalRule,
        title: "Rent needs Steve".into(),
        body_md: "Rent over nothing needs approval.".into(),
        structured: Some(json!({"approver": "Steve", "accounts": ["6100"]})),
        source: src(),
    };
    let checked = l.check_convention(&rule).unwrap();
    l.apply_convention(checked, 1).unwrap();
    let e = refused(&mut l, |l| l.renumber_account(rent, "6150"));
    assert!(
        matches!(&e, LedgerError::InvalidConvention(m)
        if m.contains("approval rule rent-approval names account 6100")),
        "{e:?}"
    );
    // An account the rule does not name moves freely.
    l.renumber_account(bank, "1010").unwrap();
    assert_eq!(l.chart().get(bank).unwrap().code, "1010");
    assert!(l.chart().find_by_code("1000").is_none());
}

// ---------------------------------------------------------------- parties

/// WB-partner-4 / BS-partner-8: a party's fields are text the books print,
/// so none holds a full TIN, a control character or more than 500
/// characters; a name is not blank or another party's; an archived party
/// returns only if no live party took its name; emails are addresses.
#[test]
fn a_party_field_is_checked_before_it_changes() {
    let mut l = Ledger::new(Currency::USD);
    let dana = l.add_party("Dana Co", &[Role::Employee]).unwrap();
    let pat = l.add_party("Pat Co", &[Role::Employee]).unwrap();
    let bad = |e: LedgerError| match e {
        LedgerError::InvalidParty(m) => m,
        other => panic!("{other:?}"),
    };

    assert_eq!(
        l.update_party(99, PartyField::Archived(true)).unwrap_err(),
        LedgerError::UnknownParty(99)
    );
    assert!(
        bad(refused(&mut l, |l| l
            .update_party(dana, PartyField::Name("  ".into()))))
        .contains("needs a name")
    );
    assert!(
        bad(refused(&mut l, |l| l
            .update_party(dana, PartyField::Name("123-45-6789".into()))))
        .contains("taxpayer identification")
    );
    assert!(
        bad(refused(&mut l, |l| l
            .update_party(dana, PartyField::Name(" pat co ".into()))))
        .contains("already exists")
    );
    // The name it already has is its own.
    l.update_party(dana, PartyField::Name("Dana Co".into()))
        .unwrap();
    l.update_party(dana, PartyField::Name("Dana & Sons".into()))
        .unwrap();
    assert_eq!(l.get_party(dana).unwrap().name, "Dana & Sons");

    for field in [
        PartyField::DisplayName(Some("EIN 12-3456789".into())),
        PartyField::Website(Some("x".repeat(501))),
        PartyField::Address(Some("1 Main\u{7}St".into())),
        PartyField::Notes(Some("SSN 123-45-6789".into())),
        PartyField::Phone(Some("123-45-6789".into())),
        PartyField::Contact(Some("x".repeat(501))),
        PartyField::BillingAddress(Some("123-45-6789".into())),
        PartyField::ShippingAddress(Some("123-45-6789".into())),
    ] {
        let f = format!("{field:?}");
        assert!(
            !bad(refused(&mut l, |l| l.update_party(dana, field))).is_empty(),
            "{f}"
        );
    }
    for field in [
        PartyField::Email(Some("not an email".into())),
        PartyField::BillingEmail(Some("a b@x".into())),
    ] {
        let m = bad(refused(&mut l, |l| l.update_party(dana, field)));
        assert!(m.contains("email"), "{m}");
    }
    // Good values pass and are kept; `None` clears.
    l.update_party(dana, PartyField::Email(Some("dana@example.com".into())))
        .unwrap();
    l.update_party(
        dana,
        PartyField::BillingEmail(Some("ap@example.com".into())),
    )
    .unwrap();
    l.update_party(dana, PartyField::Phone(Some("+1 555 0100".into())))
        .unwrap();
    l.update_party(dana, PartyField::Notes(None)).unwrap();
    assert_eq!(
        l.get_party(dana).unwrap().email.as_deref(),
        Some("dana@example.com")
    );

    // Archive Pat, take the name, and Pat cannot come back until one is renamed.
    l.update_party(pat, PartyField::Archived(true)).unwrap();
    let taker = l.add_party("Pat Co", &[Role::Employee]).unwrap();
    let m = bad(refused(&mut l, |l| {
        l.update_party(pat, PartyField::Archived(false))
    }));
    assert!(
        m.contains("another party is named \"Pat Co\"; rename one first"),
        "{m}"
    );
    l.update_party(taker, PartyField::Name("Pat Co II".into()))
        .unwrap();
    l.update_party(pat, PartyField::Archived(false)).unwrap();
    assert!(!l.get_party(pat).unwrap().archived);
}

/// A vendor's W-9 flag reports whether it changed.
#[test]
fn a_vendors_w9_flag_says_whether_it_moved() {
    let mut l = Ledger::new(Currency::USD);
    let v = l.add_vendor("Tobin Harlow Supply").unwrap();
    assert_eq!(
        l.set_vendor_w9_flag(99, true).unwrap_err(),
        LedgerError::UnknownVendor(99)
    );
    assert!(l.set_vendor_w9_flag(v, true).unwrap());
    assert!(!l.set_vendor_w9_flag(v, true).unwrap(), "already on file");
    assert!(l.set_vendor_w9_flag(v, false).unwrap());
}

// ---------------------------------------------------------------- assets

fn asset_books() -> (Ledger, u64, ledger_core::AssetId) {
    let mut l = Ledger::new(Currency::USD);
    let mut a = |code: &str, name: &str, t| l.add_account(code, name, t, None, false).unwrap();
    let cash = a("1000", "Bank", AccountType::Asset);
    let equipment = a("1500", "Equipment", AccountType::Asset);
    let accum = a("1510", "Accumulated Depreciation", AccountType::Asset);
    let depr = a("6900", "Depreciation", AccountType::Expense);
    let capital = a("3000", "Capital", AccountType::Equity);
    let e = l.create_entry(d(1, 2), "Fund");
    l.add_line(e, cash, Side::Debit, usd("5000.00")).unwrap();
    l.add_line(e, capital, Side::Credit, usd("5000.00"))
        .unwrap();
    l.post_entry(e).unwrap();
    let id = l
        .register_asset(NewAsset {
            description: "Laptop".into(),
            in_service_date: d(1, 15),
            cost: usd("1200.00"),
            salvage_value: usd("0.00"),
            useful_life_months: 12,
            cost_account: equipment,
            accum_depr_account: accum,
            expense_account: depr,
        })
        .unwrap();
    (l, cash, id)
}

/// An asset's description is edited to something, never nothing, for an
/// asset that exists; the figures stay.
#[test]
fn an_assets_description_is_never_blank() {
    let (mut l, _, id) = asset_books();
    assert_eq!(
        l.set_asset_description(99, "x").unwrap_err(),
        LedgerError::UnknownAsset(99)
    );
    assert_eq!(
        refused(&mut l, |l| l.set_asset_description(id, "  ")),
        LedgerError::EmptyAssetDescription
    );
    let cost = l.get_asset(id).unwrap().cost;
    l.set_asset_description(id, "MacBook Pro 16\"").unwrap();
    assert_eq!(l.get_asset(id).unwrap().description, "MacBook Pro 16\"");
    assert_eq!(l.get_asset(id).unwrap().cost, cost);
}

/// BS-asset-1: a disposal is refused for proceeds in another currency,
/// negative proceeds, an unknown asset, a day before the asset was
/// acquired, a second disposal; a disposal done is of an active asset
/// once.
#[test]
fn a_disposal_is_refused_unless_the_asset_is_active_and_the_proceeds_make_sense() {
    let (mut l, _, id) = asset_books();
    let eur = Money::new(dec("100.00"), Currency::EUR);
    assert!(matches!(
        refused(&mut l, |l| l.dispose_asset(id, d(6, 1), eur)),
        LedgerError::CurrencyMismatch { .. }
    ));
    assert_eq!(
        refused(&mut l, |l| l.dispose_asset(99, d(6, 1), usd("1.00"))),
        LedgerError::UnknownAsset(99)
    );
    assert_eq!(
        refused(&mut l, |l| l.dispose_asset(id, d(6, 1), usd("-1.00"))),
        LedgerError::NegativeProceeds(dec("-1.00"))
    );
    l.dispose_asset(id, d(6, 1), usd("700.00")).unwrap();
    assert_eq!(
        refused(&mut l, |l| l.dispose_asset(id, d(7, 1), usd("1.00"))),
        LedgerError::AssetAlreadyDisposed(id)
    );
}

// ---------------------------------------------------------------- the fiscal year

fn set_checked(
    l: &mut Ledger,
    key: &str,
    value: serde_json::Value,
    adopting: bool,
) -> Result<(), LedgerError> {
    let made = l.check_settings_adopting(
        &[Change {
            key: key.into(),
            value: Some(value),
        }],
        &src(),
        adopting,
    )?;
    l.apply_settings(&made, &src(), 1)
}

/// Settings rule on the fiscal year: it can't move under a closed period
/// (the year's earnings would restate; a tax year changes only with the
/// IRS's consent, Form 1128). With no period closed it moves freely; a
/// settings file being adopted for the first time (the books' settings not
/// yet journaled) brings its year in whole.
#[test]
fn the_fiscal_year_does_not_move_under_a_closed_period() {
    let mut l = Ledger::new(Currency::USD);
    let cash = l
        .add_account("1000", "Bank", AccountType::Asset, None, false)
        .unwrap();
    let sales = l
        .add_account("4000", "Sales", AccountType::Revenue, None, false)
        .unwrap();
    // No period closed: free.
    set_checked(&mut l, "company.fiscal_year_start", json!("07-01"), false).unwrap();
    set_checked(&mut l, "company.fiscal_year_start", json!("01-01"), false).unwrap();

    let e = l.create_entry(d(1, 10), "Sale");
    l.add_line(e, cash, Side::Debit, usd("100.00")).unwrap();
    l.add_line(e, sales, Side::Credit, usd("100.00")).unwrap();
    l.post_entry(e).unwrap();
    let p = l.create_period("Jan", d(1, 1), d(1, 31)).unwrap();
    l.close_period(p).unwrap();

    let before = l.clone();
    let e = set_checked(&mut l, "company.fiscal_year_start", json!("07-01"), false).unwrap_err();
    assert!(
        matches!(&e, LedgerError::InvalidSetting(m)
        if m.contains("can't change from 01-01 to 07-01") && m.contains("Form 1128")),
        "{e:?}"
    );
    assert_eq!(l, before);
    // The same year again is no change, and other settings move freely.
    set_checked(&mut l, "company.fiscal_year_start", json!("01-01"), false).unwrap();
    set_checked(&mut l, "company.name", json!("Wanderland Labs LLC"), false).unwrap();
}

// ---------------------------------------------------------------- evidence on the ledger

/// WB-ledger-15 item 4 and 5: annotations are checked as `Annotation::
/// checked` says before they are kept; a second annotation of the same file
/// replaces the first; receipt numbers run from the highest annotated, so a
/// human's duplicate is not renumbered.
#[test]
fn annotations_replace_by_file_and_receipt_numbers_run_from_the_max() {
    let mut l = Ledger::new(Currency::USD);
    assert_eq!(l.next_receipt_number(), 1);
    let ann = |id: u64, n: Option<u32>, payee: &str| Annotation {
        attachment_id: id,
        fields: AnnotationFields {
            receipt_number: n,
            payee: Some(payee.into()),
            ..Default::default()
        },
        note: None,
    };
    let empty = Annotation {
        attachment_id: 1,
        fields: AnnotationFields::default(),
        note: None,
    };
    let before = l.clone();
    assert!(matches!(
        l.annotate_attachment(empty),
        Err(LedgerError::InvalidEvidence(_))
    ));
    assert!(matches!(
        l.annotate_attachment(ann(1, Some(0), "x")),
        Err(LedgerError::InvalidEvidence(_))
    ));
    assert_eq!(l, before);

    l.annotate_attachment(ann(1, Some(7), " Staples ")).unwrap();
    l.annotate_attachment(ann(2, Some(7), "Costco")).unwrap();
    assert_eq!(
        l.next_receipt_number(),
        8,
        "a duplicate 7 is not renumbered"
    );
    assert_eq!(
        l.annotation(1).unwrap().fields.payee.as_deref(),
        Some("Staples")
    );
    l.annotate_attachment(ann(1, Some(3), "Staples")).unwrap();
    assert_eq!(
        l.annotation(1).unwrap().fields.receipt_number,
        Some(3),
        "replaced"
    );
    assert_eq!(l.annotations().count(), 2);
    l.replay_annotate_attachment(ann(3, Some(u32::MAX), "Big"))
        .unwrap();
    assert_eq!(l.next_receipt_number(), u32::MAX, "saturates, never wraps");
}

/// WB-ledger-15 item 5: `entry.settles` names entries that exist, never
/// the entry itself, never one twice; "reimbursed" is derived from it
/// alone; a refusal changes nothing.
#[test]
fn a_settlement_of_entries_names_existing_distinct_entries_only() {
    let mut l = Ledger::new(Currency::USD);
    let cash = l
        .add_account("1000", "Bank", AccountType::Asset, None, false)
        .unwrap();
    let exp = l
        .add_account("6000", "Meals", AccountType::Expense, None, false)
        .unwrap();
    let mk = |l: &mut Ledger, day| {
        let e = l.create_entry(d(3, day), "x");
        l.add_line(e, exp, Side::Debit, usd("20.00")).unwrap();
        l.add_line(e, cash, Side::Credit, usd("20.00")).unwrap();
        l.post_entry(e).unwrap();
        e
    };
    let (a, b, reimbursement) = (mk(&mut l, 1), mk(&mut l, 2), mk(&mut l, 9));

    assert_eq!(
        refused(&mut l, |l| l.set_entry_settles(99, vec![a])),
        LedgerError::UnknownEntry(99)
    );
    assert!(matches!(
        refused(&mut l, |l| l.set_entry_settles(reimbursement, vec![a, reimbursement])),
        LedgerError::InvalidEvidence(m) if m.contains("doesn't settle itself")
    ));
    assert_eq!(
        refused(&mut l, |l| l.set_entry_settles(reimbursement, vec![a, 98])),
        LedgerError::UnknownEntry(98)
    );
    assert!(matches!(
        refused(&mut l, |l| l.set_entry_settles(reimbursement, vec![a, b, a])),
        LedgerError::InvalidEvidence(m) if m.contains(&format!("entry {a} is settled twice by entry {reimbursement}"))
    ));

    assert!(l.entry_settles(reimbursement).is_empty());
    assert!(!l.settles_entry(a));
    l.set_entry_settles(reimbursement, vec![a, b]).unwrap();
    assert_eq!(l.entry_settles(reimbursement), [a, b]);
    assert!(l.settles_entry(a) && l.settles_entry(b) && !l.settles_entry(reimbursement));
    l.replay_set_entry_settles(reimbursement, vec![b]).unwrap();
    assert!(!l.settles_entry(a), "replaced, not added to");
}
