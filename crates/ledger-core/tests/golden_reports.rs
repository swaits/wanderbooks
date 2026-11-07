//! Golden files for the report rows in `docs/parity.md` that had none:
//! profit and loss for a date range, balance sheet as of a date, trial
//! balance as of a date, and the general ledger by account.
//!
//! The fixture is an **extended copy** of the hand-verified
//! `fixture_company()` in `invariants.rs`. The original has no closing
//! entry, no entry dated after an as-of date, and no multi-leg entry, so it
//! cannot show the exclusions and the `-Split-` case these reports are
//! about. Entries 1–5 are the original fixture unchanged; entries 6–8 are
//! the extension. Every expected number is computed by hand in
//! `plan/golden-worksheets/`; see the worksheet named in each test.

use chrono::NaiveDate;
use ledger_core::reports::profit_and_loss_for_period;
use ledger_core::{AccountId, AccountType, Currency, Ledger, Money, Side};
use rust_decimal::Decimal;

fn date(day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 1, day).unwrap()
}

fn february(day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 2, day).unwrap()
}

fn usd(amount: u64) -> Money {
    Money::new(Decimal::from(amount), Currency::USD)
}

/// Post a balanced entry in one call. Fixtures must be valid by construction.
fn post(ledger: &mut Ledger, date: NaiveDate, description: &str, lines: &[(AccountId, Side, u64)]) {
    // As replay posts it: this fixture holds hand lines on the receivable
    // and payable control accounts (1200, 2000), which books an earlier
    // version wrote still hold and every report must read; the control
    // guard (rules.md B-38) refuses them as new work.
    ledger_core::parties::replaying(|| {
        let id = ledger.create_entry(date, description);
        for (account, side, amount) in lines {
            ledger.add_line(id, *account, *side, usd(*amount)).unwrap();
        }
        ledger.post_entry(id).unwrap();
    })
}

/// The extended January 2026 fixture: the original five entries, then a
/// multi-leg equipment purchase (cash and payable), a January close, and a
/// February entry after every as-of date used here.
fn golden_company() -> Ledger {
    let mut ledger = Ledger::new(Currency::USD);
    let cash = ledger
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    let ar = ledger
        .add_account(
            "1200",
            "Accounts Receivable",
            AccountType::Asset,
            None,
            false,
        )
        .unwrap();
    let equipment = ledger
        .add_account("1500", "Equipment", AccountType::Asset, None, false)
        .unwrap();
    let ap = ledger
        .add_account(
            "2000",
            "Accounts Payable",
            AccountType::Liability,
            None,
            false,
        )
        .unwrap();
    let equity = ledger
        .add_account("3000", "Owner's Equity", AccountType::Equity, None, false)
        .unwrap();
    let revenue = ledger
        .add_account("4000", "Service Revenue", AccountType::Revenue, None, false)
        .unwrap();
    let rent = ledger
        .add_account("6000", "Rent Expense", AccountType::Expense, None, false)
        .unwrap();

    // 1. Owner invests $10,000 cash.
    post(
        &mut ledger,
        date(2),
        "Owner investment",
        &[(cash, Side::Debit, 10_000), (equity, Side::Credit, 10_000)],
    );
    // 2. Buy equipment for $2,500 cash.
    post(
        &mut ledger,
        date(5),
        "Buy equipment",
        &[(equipment, Side::Debit, 2_500), (cash, Side::Credit, 2_500)],
    );
    // 3. Bill a client $4,000.
    post(
        &mut ledger,
        date(10),
        "Invoice client",
        &[(ar, Side::Debit, 4_000), (revenue, Side::Credit, 4_000)],
    );
    // 4. Pay January rent, $800.
    post(
        &mut ledger,
        date(15),
        "January rent",
        &[(rent, Side::Debit, 800), (cash, Side::Credit, 800)],
    );
    // 5. Collect $1,500 of receivables.
    post(
        &mut ledger,
        date(20),
        "Client payment",
        &[(cash, Side::Debit, 1_500), (ar, Side::Credit, 1_500)],
    );
    // 6. Buy more equipment: $1,000 cash and $500 on account (multi-leg, so
    //    its Equipment line's split is "-Split-").
    post(
        &mut ledger,
        date(25),
        "Buy more equipment",
        &[
            (equipment, Side::Debit, 1_500),
            (cash, Side::Credit, 1_000),
            (ap, Side::Credit, 500),
        ],
    );
    // 7. Close January: revenue 4,000 less rent 800 moves 3,200 to retained
    //    earnings, dated at the period end (2026-01-31).
    let period = ledger.create_period("2026-01", date(1), date(31)).unwrap();
    ledger.close_period(period).unwrap();
    // 8. February revenue after every as-of date in these tests.
    post(
        &mut ledger,
        february(5),
        "February service",
        &[(ar, Side::Debit, 600), (revenue, Side::Credit, 600)],
    );

    ledger
}

/// By-hand worksheet: `plan/golden-worksheets/profit-and-loss-period.md`.
#[test]
fn golden_profit_and_loss_period() {
    let ledger = golden_company();
    let report = profit_and_loss_for_period(&ledger, date(1), date(31), None).unwrap();
    let got = serde_json::to_value(&report).unwrap();
    let want: serde_json::Value =
        serde_json::from_str(include_str!("golden/profit_and_loss_period.json")).unwrap();
    assert_eq!(got, want);
}
