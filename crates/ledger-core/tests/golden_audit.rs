//! The audit workpaper's findings for one small book, hand-verified
//! (rules.md B-45; plan/golden-worksheets/audit-workpaper.md).
//!
//! The fixture is a bank, prepaid insurance, capital, two revenue and two
//! expense accounts, and a 12-month prepaid plan. It seeds five defects in
//! five check areas (`draft_open`, `evidence_missing`, `schedule_behind`,
//! `tax_category_missing`, `vendor_1099_undecided`) and leaves every other
//! check silent: the trial balance balances, the equation holds, the P&L
//! articulates, no control account or document exists, no close has run, no
//! asset is registered, and the entries the receipt policy reaches have
//! their receipts.
//!
//! The golden holds the workpaper's statements data — scope, through,
//! summary and findings — not the store's hashes, the figure head or the
//! programme's version (see the worksheet's "Scope of this golden").

use std::collections::BTreeSet;

use chrono::NaiveDate;
use ledger_core::audit::{self, AuditInput, Severity};
use ledger_core::settings::Settings;
use ledger_core::{
    AccountType, Currency, Ledger, Money, NewSchedule, ScheduleConvention, ScheduleKind, Side,
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
/// opening, the unreceipted purchase, the prepaid payment, and a draft.
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

    // 1. Opening: 2,000.00 of capital into the bank.
    post(
        &mut ledger,
        d(1, 1),
        "Opening",
        &[
            (bank, Side::Debit, "2000.00"),
            (capital, Side::Credit, "2000.00"),
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
    // Entries 1 and 3 carry receipts; entry 2 is the evidence defect.
    let receipts: BTreeSet<u64> = [1, 3].into_iter().collect();
    let undecided_1099 = [("Tobin Harlow Supply".to_string(), usd("900.00").amount)];
    let through = d(3, 31);

    let findings = audit::statements(
        &ledger,
        &AuditInput {
            settings: &settings,
            receipts: &receipts,
            cash_flow: None,
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
