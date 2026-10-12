//! Tax categories and the tax summary. Expected numbers computed by hand.
//!
//! The fixture: a partnership's first quarter.
//!
//! | account               | category              | % | book    | deductible |
//! |-----------------------|-----------------------|---|---------|------------|
//! | 4000 Sales            | gross_receipts        |   | 10,000  | 10,000     |
//! | 4100 Refunds (exp.)   | returns_allowances    |   | 250     | (250)      |
//! | 6000 Platform fees    | commissions_fees      |   | 1,500   | 1,500      |
//! | 6100 Meals            | meals                 |50%| 333.33  | 166.67     |
//! | 6200 Penalties        | nondeductible         | 0%| 40      | 0          |
//! | 6300 Software         | other_deductions      |   | 600     | 600        |
//! | 6400 Partner pay      | guaranteed_payments   |   | 2,000   | 2,000      |
//! | 6500 Misc             | (none)                |   | 75      | uncategorized |

use chrono::NaiveDate;
use ledger_core::tax::{TaxForm, check_category, tax_summary};
use ledger_core::{AccountType, Currency, Ledger, LedgerError, Money, Side};
use rust_decimal::Decimal;
use std::str::FromStr;

fn d(m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, m, day).unwrap()
}

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

fn books() -> Ledger {
    let mut l = Ledger::new(Currency::USD);
    let cash = l
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    let mut acct =
        |code: &str, name: &str, t: AccountType, cat: Option<&str>, pct: Option<&str>| {
            let id = l.add_account(code, name, t, None, false).unwrap();
            if let Some(c) = cat {
                l.set_account_tax(id, Some(c), pct.map(dec)).unwrap();
            }
            id
        };
    let sales = acct(
        "4000",
        "Sales",
        AccountType::Revenue,
        Some("gross_receipts"),
        None,
    );
    let refunds = acct(
        "4100",
        "Refunds",
        AccountType::Expense,
        Some("returns_allowances"),
        None,
    );
    let fees = acct(
        "6000",
        "Platform fees",
        AccountType::Expense,
        Some("commissions_fees"),
        None,
    );
    let meals = acct("6100", "Meals", AccountType::Expense, Some("meals"), None);
    let pen = acct(
        "6200",
        "Penalties",
        AccountType::Expense,
        Some("nondeductible"),
        None,
    );
    let soft = acct(
        "6300",
        "Software",
        AccountType::Expense,
        Some("other_deductions"),
        None,
    );
    let gp = acct(
        "6400",
        "Partner pay",
        AccountType::Expense,
        Some("guaranteed_payments"),
        None,
    );
    let misc = acct("6500", "Misc", AccountType::Expense, None, None);
    let mut post = |dr, cr, amt: &str| {
        let e = l.create_entry(d(2, 1), "x");
        l.add_line(e, dr, Side::Debit, Money::new(dec(amt), Currency::USD))
            .unwrap();
        l.add_line(e, cr, Side::Credit, Money::new(dec(amt), Currency::USD))
            .unwrap();
        l.post_entry(e).unwrap();
    };
    post(cash, sales, "10000");
    for (a, amt) in [
        (refunds, "250"),
        (fees, "1500"),
        (meals, "333.33"),
        (pen, "40"),
        (soft, "600"),
        (gp, "2000"),
        (misc, "75"),
    ] {
        post(a, cash, amt);
    }
    l
}

#[test]
fn the_summary_groups_by_category_and_applies_deductible_percents() {
    let l = books();
    let ts = tax_summary(&l, d(1, 1), d(3, 31), Some(TaxForm::Form1065)).unwrap();
    let rows = |lines: &[ledger_core::TaxSummaryLine]| -> Vec<(String, Decimal, Decimal)> {
        lines
            .iter()
            .map(|l| (l.category.clone(), l.book, l.tax))
            .collect()
    };
    // Refunds (an expense account) reduce receipts as negative income.
    assert_eq!(
        rows(&ts.income),
        [
            ("gross_receipts".into(), dec("10000"), dec("10000")),
            ("returns_allowances".into(), dec("-250"), dec("-250")),
        ]
    );
    // Catalog order; meals at 50% (333.33 → 166.665 → 166.67, half away
    // from zero); penalties at 0%.
    assert_eq!(
        rows(&ts.deductions),
        [
            ("commissions_fees".into(), dec("1500"), dec("1500")),
            ("guaranteed_payments".into(), dec("2000"), dec("2000")),
            ("meals".into(), dec("333.33"), dec("166.67")),
            ("other_deductions".into(), dec("600"), dec("600")),
            ("nondeductible".into(), dec("40"), dec("0")),
        ]
    );
    assert_eq!(ts.uncategorized.len(), 1);
    assert_eq!(ts.uncategorized[0].account, "6500 Misc");
    assert_eq!(ts.uncategorized[0].book, dec("75"));
    // Book: 9,750 − 4,473.33 = 5,276.67. Tax: 9,750 − 4,266.67 = 5,483.33.
    assert_eq!(ts.book_net(), dec("5276.67"));
    assert_eq!(ts.tax_net(), dec("5483.33"));
}

#[test]
fn an_override_beats_the_category_default() {
    let mut l = books();
    let meals = l.chart().find_by_code("6100").unwrap().id;
    // A company party is 100% deductible: override the 50% default.
    l.set_account_tax(meals, Some("meals"), Some(dec("100")))
        .unwrap();
    let ts = tax_summary(&l, d(1, 1), d(3, 31), None).unwrap();
    let m = ts
        .deductions
        .iter()
        .find(|x| x.category == "meals")
        .unwrap();
    assert_eq!(m.tax, dec("333.33"));
}

#[test]
fn categories_must_fit_the_account_and_the_return() {
    let mut l = books();
    let sales = l.chart().find_by_code("4000").unwrap().id;
    let meals = l.chart().find_by_code("6100").unwrap().id;
    let bad = |r: Result<(), LedgerError>| matches!(r, Err(LedgerError::InvalidTaxCategory(_)));
    assert!(
        bad(l.set_account_tax(sales, Some("meals"), None)),
        "deduction on revenue"
    );
    assert!(
        bad(l.set_account_tax(meals, Some("no_such"), None)),
        "unknown key"
    );
    assert!(
        bad(l.set_account_tax(meals, Some("meals"), Some(dec("101")))),
        "over 100%"
    );
    assert!(
        bad(l.set_account_tax(meals, None, Some(dec("50")))),
        "percent without category"
    );
    // Guaranteed payments exist only on a partnership return.
    assert!(
        check_category(
            "guaranteed_payments",
            AccountType::Expense,
            Some(TaxForm::Form1065)
        )
        .is_ok()
    );
    assert!(
        check_category(
            "guaranteed_payments",
            AccountType::Expense,
            Some(TaxForm::ScheduleC)
        )
        .is_err()
    );
    assert!(
        check_category(
            "officer_compensation",
            AccountType::Expense,
            Some(TaxForm::Form1120S)
        )
        .is_ok()
    );
    // Clearing works.
    l.set_account_tax(meals, None, None).unwrap();
    assert!(l.chart().get(meals).unwrap().tax_category.is_none());
}

#[test]
fn forms_parse_from_common_spellings() {
    for (s, f) in [
        ("schedule_c", TaxForm::ScheduleC),
        ("Schedule C", TaxForm::ScheduleC),
        ("1065", TaxForm::Form1065),
        ("Form 1065", TaxForm::Form1065),
        ("1120s", TaxForm::Form1120S),
        ("1120-S", TaxForm::Form1120S),
        ("1120", TaxForm::Form1120),
    ] {
        assert_eq!(TaxForm::parse(s), Some(f), "{s}");
    }
    assert_eq!(TaxForm::parse("990"), None);
}
