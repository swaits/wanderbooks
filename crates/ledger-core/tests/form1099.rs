//! 1099 tracking. Numbers computed by hand.
//!
//! 2026 payments:
//! - Dana (1099-NEC, W-9): bill 1,500 paid by check + 700 direct = 2,200,
//!   over the 2,000 threshold. A 300 payment on 2025-12-30 counts in 2025.
//! - Ace Rentals (1099-MISC): 1,200 twice, one reversed = 1,200, under.
//! - Sam (1099-NEC): 5,000 by credit card = 0 cash: the card processor's
//!   1099-K covers it.
//! - Bigco (no 1099): 9,000, listed but never reportable.

use chrono::NaiveDate;
use ledger_core::arap::DocumentLine;
use ledger_core::form1099::default_threshold;
use ledger_core::{
    AccountType, Currency, Form1099, Ledger, LedgerError, Money, Side, vendor_1099_report,
};
use rust_decimal::Decimal;
use std::str::FromStr;

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn books() -> Ledger {
    let mut l = Ledger::new(Currency::USD);
    let bank = l
        .add_account("1000", "Bank", AccountType::Asset, None, false)
        .unwrap();
    let card = l
        .add_account("2100", "Credit Card", AccountType::Liability, None, false)
        .unwrap();
    let contract = l
        .add_account("6000", "Contract Labor", AccountType::Expense, None, false)
        .unwrap();
    let rent = l
        .add_account("6600", "Rent", AccountType::Expense, None, false)
        .unwrap();
    let dana = l.add_vendor("Dana").unwrap();
    let ace = l.add_vendor("Ace Rentals").unwrap();
    let sam = l.add_vendor("Sam").unwrap();
    let bigco = l.add_vendor("Bigco").unwrap();
    l.set_vendor_1099(dana, Some(Form1099::Nec), true).unwrap();
    l.set_vendor_1099(ace, Some(Form1099::Misc), false).unwrap();
    l.set_vendor_1099(sam, Some(Form1099::Nec), true).unwrap();
    let pay = |l: &mut Ledger, date, vendor, dr, cr, amt: &str| {
        let e = l.create_entry(date, "payment");
        let m = Money::new(dec(amt), Currency::USD);
        l.add_line(e, dr, Side::Debit, m).unwrap();
        l.add_line(e, cr, Side::Credit, m).unwrap();
        l.post_entry(e).unwrap();
        l.set_entry_vendor(e, Some(vendor)).unwrap();
        e
    };
    let bill = l
        .create_bill(
            dana,
            d(2026, 3, 1),
            d(2026, 3, 31),
            vec![DocumentLine {
                description: "Design work".into(),
                amount: dec("1500"),
            }],
        )
        .unwrap();
    l.pay_bill(bill, d(2026, 3, 20), dec("1500"), bank).unwrap();
    pay(&mut l, d(2026, 6, 1), dana, contract, bank, "700");
    pay(&mut l, d(2025, 12, 30), dana, contract, bank, "300");
    pay(&mut l, d(2026, 1, 1), ace, rent, bank, "1200");
    let wrong = pay(&mut l, d(2026, 2, 1), ace, rent, bank, "1200");
    l.reverse_entry(wrong, d(2026, 2, 2), "duplicate").unwrap();
    pay(&mut l, d(2026, 4, 1), sam, contract, card, "5000");
    pay(&mut l, d(2026, 5, 1), bigco, contract, bank, "9000");
    // Untagged spending is not anyone's 1099.
    let e = l.create_entry(d(2026, 7, 1), "untagged");
    let m = Money::new(dec("50"), Currency::USD);
    l.add_line(e, contract, Side::Debit, m).unwrap();
    l.add_line(e, bank, Side::Credit, m).unwrap();
    l.post_entry(e).unwrap();
    l
}

#[test]
fn cash_paid_per_vendor_and_who_is_reportable() {
    let l = books();
    let r = vendor_1099_report(&l, 2026, None).unwrap();
    assert_eq!(r.threshold, dec("2000"));
    let rows: Vec<(&str, Decimal, bool)> = r
        .vendors
        .iter()
        .map(|v| (v.name.as_str(), v.paid, v.reportable))
        .collect();
    assert_eq!(
        rows,
        [
            ("Ace Rentals", dec("1200"), false),
            ("Bigco", dec("9000"), false),
            ("Dana", dec("2200"), true),
            ("Sam", dec("0"), false),
        ]
    );
    let ace = &r.vendors[0];
    assert_eq!(ace.form, Some(Form1099::Misc));
    assert!(!ace.tin_on_file);
}

#[test]
fn years_and_thresholds() {
    let l = books();
    // SOURCE: rules.md B-19 (sources/irs-1099-thresholds.md).
    assert_eq!(default_threshold(2025), Ok(dec("600")));
    assert_eq!(default_threshold(2026), Ok(dec("2000")));
    let y2025 = vendor_1099_report(&l, 2025, None).unwrap();
    let dana = y2025.vendors.iter().find(|v| v.name == "Dana").unwrap();
    assert_eq!(dana.paid, dec("300"));
    assert!(!dana.reportable, "300 < 600");
    // An override: at 1,000, Ace's 1,200 is reportable.
    let r = vendor_1099_report(&l, 2026, Some(dec("1000"))).unwrap();
    assert!(
        r.vendors
            .iter()
            .any(|v| v.name == "Ace Rentals" && v.reportable)
    );
}

#[test]
fn tags_need_real_entries_and_vendors() {
    let mut l = books();
    assert!(matches!(
        l.set_entry_vendor(999, Some(1)),
        Err(LedgerError::UnknownEntry(999))
    ));
    assert!(matches!(
        l.set_entry_vendor(1, Some(999)),
        Err(LedgerError::UnknownVendor(999))
    ));
    // The bill and its payment were tagged automatically.
    assert_eq!(l.entry_vendor(1), Some(1));
    assert_eq!(l.entry_vendor(2), Some(1));
    l.set_entry_vendor(2, None).unwrap();
    assert_eq!(l.entry_vendor(2), None);
    assert_eq!(Form1099::parse("1099-NEC"), Some(Form1099::Nec));
    assert_eq!(Form1099::parse("misc"), Some(Form1099::Misc));
    assert_eq!(Form1099::parse("K"), None);
}

/// Spec (Stephen, 2026-10-13): a rule set by law that wanderbooks doesn't
/// know for a year is never guessed. 26 U.S.C. §6041(h) indexes the $2,000
/// threshold for calendar years after 2026; until the indexed amount is in
/// the rule table, asking for it is an error that says what to do.
#[test]
fn an_unknown_year_threshold_is_never_guessed() {
    let err = default_threshold(2027).unwrap_err();
    let LedgerError::RuleNotKnown {
        rule,
        year,
        known_through,
        spec,
    } = &err
    else {
        panic!("expected RuleNotKnown, got {err:?}");
    };
    assert_eq!(
        (rule.as_str(), *year, *known_through),
        ("1099 reporting threshold", 2027, 2026)
    );
    assert!(spec.ends_with("irs-1099-thresholds.md"), "{spec}");
    let why = err.to_string();
    for needle in [
        "2027",
        "official source",
        "CPA",
        "github.com/swaits/wanderbooks",
    ] {
        assert!(why.contains(needle), "missing {needle:?} in: {why}");
    }
    let l = books();
    assert!(matches!(
        vendor_1099_report(&l, 2027, None),
        Err(LedgerError::RuleNotKnown { .. })
    ));
    // An explicit threshold (from the IRS, or the owner or CPA) works.
    let r = vendor_1099_report(&l, 2027, Some(dec("2100"))).unwrap();
    assert_eq!(r.threshold, dec("2100"));
}

/// Spec: a dated rule is due for review from its review date, and stays
/// due; before it, nothing is stale. The engine has no clock: callers pass
/// today.
#[test]
fn dated_rules_go_stale_on_their_review_date() {
    use ledger_core::rules::{THRESHOLD_1099, stale_rules};
    assert!(stale_rules(d(2026, 10, 13)).is_empty());
    let due = THRESHOLD_1099.review_after();
    assert!(stale_rules(due.pred_opt().unwrap()).is_empty());
    assert_eq!(stale_rules(due).len(), 1);
    assert_eq!(
        stale_rules(d(2027, 1, 1))[0].name,
        "1099 reporting threshold"
    );
    assert_eq!(stale_rules(d(2030, 6, 1)).len(), 1);
}

#[test]
fn forms_have_their_irs_names() {
    assert_eq!(Form1099::Nec.label(), "1099-NEC");
    assert_eq!(Form1099::Misc.label(), "1099-MISC");
    // A label parses back to its form.
    for f in [Form1099::Nec, Form1099::Misc] {
        assert_eq!(Form1099::parse(f.label()), Some(f));
    }
}
