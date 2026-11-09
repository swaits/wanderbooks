//! The audit workpaper's findings for one small book, hand-verified
//! (BS-ledger-13; plan/golden-worksheets/audit-workpaper.md).
//!
//! The fixture is a bank, prepaid insurance, capital, two revenue and two
//! expense accounts, a 12-month prepaid plan, and a vendor paid from the
//! bank. It seeds five defects in five check areas (`draft_open`,
//! `evidence_missing`, `schedule_behind`, `tax_category_missing`,
//! `vendor_1099_undecided`) and leaves every other check silent: the trial
//! balance balances, the equation holds, the P&L articulates, the cash
//! flow foots, no control account or document exists, no close has run, no
//! asset is registered, and the entries the receipt policy reaches have
//! their receipts. The `vendor_1099_undecided` input is derived from the
//! books the way `ledger-mcp`'s `audit_findings` does it, not handed in.
//!
//! The golden holds the workpaper's statements data — scope, through,
//! summary and findings — not the store's hashes, the figure head or the
//! programme's version (see the worksheet's "Scope of this golden").

use std::collections::BTreeSet;

use chrono::{Datelike, NaiveDate};
use ledger_core::audit::{self, AuditInput, Severity};
use ledger_core::settings::{Company, Settings};
use ledger_core::{
    AccountType, Currency, Ledger, Money, NewSchedule, ScheduleConvention, ScheduleKind, Side,
    Status1099,
};
use rust_decimal::Decimal;

fn d(month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, month, day).unwrap()
}

fn usd(amount: &str) -> Money {
    Money::new(amount.parse::<Decimal>().unwrap(), Currency::USD)
}

/// Post a balanced entry in one call and return its id.
fn post(
    ledger: &mut Ledger,
    date: NaiveDate,
    description: &str,
    lines: &[(u64, Side, &str)],
) -> u64 {
    let id = ledger.create_entry(date, description);
    for (account, side, amount) in lines {
        ledger.add_line(id, *account, *side, usd(amount)).unwrap();
    }
    ledger.post_entry(id).unwrap();
    id
}

/// The worksheet's fixture: accounts in creation order (ids 1–7), then the
/// opening, the unreceipted purchase, the prepaid payment, a draft, and the
/// vendor's payment (entry 5, after the draft so the draft stays entry 4).
fn audit_company() -> Ledger {
    let mut ledger = Ledger::new(Currency::USD);
    let bank = ledger
        .add_account("1000", "Bank", AccountType::Asset, None, false)
        .unwrap();
    let prepaid = ledger
        .add_account("1300", "Prepaid Insurance", AccountType::Asset, None, false)
        .unwrap();
    let capital = ledger
        .add_account("3000", "Capital", AccountType::Equity, None, false)
        .unwrap();
    let sales = ledger
        .add_account("4000", "Sales", AccountType::Revenue, None, false)
        .unwrap();
    let other_income = ledger
        .add_account("4100", "Other income", AccountType::Revenue, None, false)
        .unwrap();
    let supplies = ledger
        .add_account("6000", "Supplies", AccountType::Expense, None, false)
        .unwrap();
    let insurance = ledger
        .add_account("6100", "Insurance", AccountType::Expense, None, false)
        .unwrap();
    // Every Revenue/Expense account but 4100 gets a category: 4100 is the
    // seeded tax-category defect and nothing else is uncategorized.
    for (account, category) in [
        (sales, "gross_receipts"),
        (supplies, "supplies"),
        (insurance, "insurance"),
    ] {
        ledger
            .set_account_tax(account, Some(category), None)
            .unwrap();
    }

    // 1. Opening: 3,000.00 of capital into the bank.
    post(
        &mut ledger,
        d(1, 1),
        "Opening",
        &[
            (bank, Side::Debit, "3000.00"),
            (capital, Side::Credit, "3000.00"),
        ],
    );
    // 2. Printer paper, 40.00, no receipt: the evidence defect.
    post(
        &mut ledger,
        d(1, 5),
        "Printer paper",
        &[
            (supplies, Side::Debit, "40.00"),
            (bank, Side::Credit, "40.00"),
        ],
    );
    // 3. A year of insurance paid up front, with its receipt. The plan is
    //    never posted, so its months are the schedule defect.
    post(
        &mut ledger,
        d(1, 1),
        "Prepaid insurance",
        &[
            (prepaid, Side::Debit, "1200.00"),
            (bank, Side::Credit, "1200.00"),
        ],
    );
    // 4. A draft dated before the audit, never posted: the draft defect.
    let draft = ledger.create_entry(d(2, 10), "Stamps");
    ledger
        .add_line(draft, supplies, Side::Debit, usd("5.00"))
        .unwrap();
    ledger
        .add_line(draft, bank, Side::Credit, usd("5.00"))
        .unwrap();

    // 5. 900.00 of supplies paid from the bank to a vendor (a party of its
    //    own), with its receipt; the vendor's 1099 decision is left
    //    undecided: the 1099 defect.
    let vendor = ledger.add_vendor("Tobin Harlow Supply").unwrap();
    assert!(ledger.party_of_vendor(vendor).is_some());
    let paid = post(
        &mut ledger,
        d(3, 10),
        "Supplies from Tobin Harlow",
        &[
            (supplies, Side::Debit, "900.00"),
            (bank, Side::Credit, "900.00"),
        ],
    );
    ledger.set_entry_vendor(paid, Some(vendor)).unwrap();

    ledger
        .register_schedule(NewSchedule {
            description: "Prepaid insurance".into(),
            kind: ScheduleKind::PrepaidExpense,
            total: usd("1200.00"),
            start: d(1, 1),
            months: 12,
            balance_account: prepaid,
            pl_account: insurance,
            convention: ScheduleConvention::MonthlyEven,
        })
        .unwrap();

    let _ = other_income;
    ledger
}

/// By-hand worksheet: `plan/golden-worksheets/audit-workpaper.md`.
#[test]
fn golden_audit_workpaper() {
    let ledger = audit_company();
    let settings = Settings::default();
    // Entries 1, 3 and 5 carry receipts; entry 2 is the evidence defect.
    let receipts: BTreeSet<u64> = [1, 3, 5].into_iter().collect();
    let through = d(3, 31);

    // The books name the bank (1000) as their one cash account
    // (`books.cash_accounts`); the package reads the roles from there.
    let company = Company::default();
    let cash_accounts = vec!["1000".to_string()];
    let package = ledger_package::Settings {
        company: &company,
        cash_accounts: &cash_accounts,
        financing_accounts: &[],
        investing_accounts: &[],
    };

    // The statement of cash flows, from the first entry through `through`,
    // with the roles: as `ledger-mcp`'s `audit_findings` tries it.
    let first = ledger
        .posted_entries()
        .filter(|e| e.date() <= through)
        .map(|e| e.date())
        .min()
        .expect("the fixture posts entries");
    let statement = ledger_package::cash_flow_for(
        &ledger,
        &package,
        Some(&first.to_string()),
        Some(&through.to_string()),
        &[],
        &[],
        &[],
        through,
    );
    let cash = Some(statement.clone().map(|s| s.reconciles));

    // Vendors paid in a way that could need a 1099 and undecided: the same
    // derivation as `ledger-mcp`'s `audit_findings`, from the ledger.
    let report =
        ledger_package::vendor_1099_report(&ledger, &package, through.year(), None, Some(through));
    let undecided_1099: Vec<(String, Decimal)> = report
        .as_ref()
        .map(|r| {
            r.vendors
                .iter()
                .filter(|v| v.decision_needed)
                .map(|v| (v.name.clone(), v.paid))
                .collect()
        })
        .unwrap_or_default();

    // The worksheet's hand values for the two derived inputs (see "The
    // vendor's 1099 and the cash flow, by hand"): the statement foots at
    // +860.00 (operating -2,140.00, financing +3,000.00); the vendor's
    // report is 900.00 of undecided payments against the 2026 threshold
    // of 2,000.00, so under threshold, yet a decision is asked for.
    let statement = statement.unwrap();
    assert_eq!(statement.cash_from_operations, usd("-2140.00").amount);
    assert_eq!(statement.cash_from_investing, Decimal::ZERO);
    assert_eq!(statement.cash_from_financing, usd("3000.00").amount);
    assert_eq!(statement.cash_beginning, Decimal::ZERO);
    assert_eq!(statement.cash_ending, usd("860.00").amount);
    let footing = cash.as_ref().unwrap().as_ref().unwrap();
    assert_eq!(footing.sections_sum, usd("860.00").amount);
    assert_eq!(footing.net_change, usd("860.00").amount);
    assert_eq!(footing.difference, Decimal::ZERO);
    let report = report.unwrap();
    assert_eq!(report.threshold, usd("2000.00").amount);
    assert_eq!(report.vendors.len(), 1);
    assert_eq!(report.vendors[0].status, Status1099::UnderThreshold);
    assert_eq!(
        undecided_1099,
        [("Tobin Harlow Supply".to_string(), usd("900.00").amount)]
    );

    let findings = audit::statements(
        &ledger,
        &AuditInput {
            settings: &settings,
            receipts: &receipts,
            cash_flow: cash.as_ref(),
            settings_problems: &[],
            undecided_1099: &undecided_1099,
        },
        through,
    );

    let count = |severity: Severity| findings.iter().filter(|f| f.severity == severity).count();
    let summary = format!(
        "{} failure(s), {} warning(s), {} note(s)",
        count(Severity::Failure),
        count(Severity::Warning),
        count(Severity::Info)
    );

    // The workpaper's statements data: what `books.audit` files, minus the
    // fields that are not hand-verifiable (hashes, figure head, build
    // version) or not findings (company name, the static check catalogue).
    let got = serde_json::json!({
        "scope": "month",
        "through": through.to_string(),
        "summary": summary,
        "findings": serde_json::to_value(&findings).unwrap(),
    });
    let want: serde_json::Value =
        serde_json::from_str(include_str!("golden/audit_workpaper.json")).unwrap();
    assert_eq!(got, want);
}
