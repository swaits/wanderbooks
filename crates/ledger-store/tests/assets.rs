//! Fixed-asset store tests: journaling, idempotency, disposal, and verify.

use chrono::NaiveDate;
use ledger_core::{AccountType, Currency, Money, NewAsset, Side};
use ledger_store::Store;
use rust_decimal::Decimal;
use tempfile::tempdir;

fn usd(amount: &str) -> Money {
    Money::new(amount.parse().unwrap(), Currency::USD)
}

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

fn open_store() -> (tempfile::TempDir, Store) {
    let dir = tempdir().unwrap();
    let store = Store::create(dir.path(), Currency::USD).unwrap();
    (dir, store)
}

fn asset_accounts(store: &mut Store) -> (u64, u64, u64, u64, u64) {
    let cost = store
        .add_account("1500", "Equipment", AccountType::Asset, None, false)
        .unwrap();
    let accum = store
        .add_account(
            "1501",
            "Accumulated Depreciation",
            AccountType::Asset,
            None,
            false,
        )
        .unwrap();
    let expense = store
        .add_account(
            "6100",
            "Depreciation Expense",
            AccountType::Expense,
            None,
            false,
        )
        .unwrap();
    let bank = store
        .add_account("1000", "Bank", AccountType::Asset, None, false)
        .unwrap();
    let gain = store
        .add_account(
            "4900",
            "Gain on Disposal",
            AccountType::Revenue,
            None,
            false,
        )
        .unwrap();
    (cost, accum, expense, bank, gain)
}

fn van(store: &mut Store, dollars: u64, cost_acct: u64, accum_acct: u64, expense_acct: u64) -> u64 {
    store
        .register_asset(NewAsset {
            description: "Sprinter van".to_string(),
            purchase_date: date(2026, 1, 15),
            cost: usd(&dollars.to_string()),
            salvage_value: usd("0"),
            useful_life_months: 60,
            cost_account: cost_acct,
            accum_depr_account: accum_acct,
            expense_account: expense_acct,
        })
        .unwrap()
}

#[test]
fn asset_survives_reopen_with_stable_ids() {
    let (dir, mut store) = open_store();
    let (cost, accum, expense, _, _) = asset_accounts(&mut store);
    let id = van(&mut store, 60_000, cost, accum, expense);
    drop(store);

    let mut store = Store::open(dir.path()).unwrap();
    let asset = store.ledger().get_asset(id).expect("asset must survive");
    assert_eq!(asset.description, "Sprinter van");
    assert_eq!(asset.cost.amount, Decimal::from(60_000));
    // Allocator stability: the next registration continues the sequence.
    let id2 = van(&mut store, 1_000, cost, accum, expense);
    assert_eq!(id2, id + 1);
    store.verify().unwrap();
}

#[test]
fn depreciation_run_is_idempotent() {
    let (_dir, mut store) = open_store();
    let (cost, accum, expense, _, _) = asset_accounts(&mut store);
    let id = van(&mut store, 60_000, cost, accum, expense);

    let run1 = store.run_depreciation(id, "2026-01").unwrap();
    assert!(!run1.already_posted);
    assert_eq!(run1.amount, Decimal::new(54839, 2)); // 1000 * 17/31

    // Second run of the same period is a no-op returning the same entry.
    let run2 = store.run_depreciation(id, "2026-01").unwrap();
    assert!(run2.already_posted);
    assert_eq!(run2.entry_id, run1.entry_id);
    assert_eq!(run2.amount, run1.amount);

    // A later period posts its own entry.
    let run3 = store.run_depreciation(id, "2026-02").unwrap();
    assert!(!run3.already_posted);
    assert_eq!(run3.amount, Decimal::from(1_000));

    // The depreciation entries hit the right accounts.
    let entry = store.ledger().get_entry(run3.entry_id).unwrap();
    assert!(entry.lines().iter().any(|l| l.account == expense
        && l.side == Side::Debit
        && l.amount.amount == Decimal::from(1_000)));
    store.verify().unwrap();
}

#[test]
fn depreciation_rejects_period_outside_life() {
    let (_dir, mut store) = open_store();
    let (cost, accum, expense, _, _) = asset_accounts(&mut store);
    let id = van(&mut store, 60_000, cost, accum, expense);
    // Before purchase.
    assert!(store.run_depreciation(id, "2025-12").is_err());
    // After the 60-month life ends (2031-01).
    assert!(store.run_depreciation(id, "2031-06").is_err());
    // Malformed period.
    assert!(store.run_depreciation(id, "2026-13").is_err());
    assert!(store.run_depreciation(id, "january").is_err());
    // Unknown asset.
    assert!(store.run_depreciation(999, "2026-01").is_err());
}

#[test]
fn disposal_posts_balanced_entry_and_marks_asset() {
    let (_dir, mut store) = open_store();
    let (cost, accum, expense, bank, gain) = asset_accounts(&mut store);
    let id = van(&mut store, 60_000, cost, accum, expense);
    store.run_depreciation(id, "2026-01").unwrap();
    store.run_depreciation(id, "2026-02").unwrap();

    // Book value after two months: 60000 − (548.39 + 1000) = 58451.61.
    // Sell for 60,000 → gain of 1548.39.
    let entry_id = store
        .dispose_asset(id, date(2026, 2, 28), usd("60000"), bank, gain)
        .unwrap();
    let entry = store.ledger().get_entry(entry_id).unwrap();
    let debits: Decimal = entry
        .lines()
        .iter()
        .filter(|l| l.side == Side::Debit)
        .map(|l| l.amount.amount)
        .sum();
    let credits: Decimal = entry
        .lines()
        .iter()
        .filter(|l| l.side == Side::Credit)
        .map(|l| l.amount.amount)
        .sum();
    assert_eq!(debits, credits, "disposal entry must balance");
    assert_eq!(debits, Decimal::new(154839, 2) + Decimal::from(60_000));

    // Asset is marked disposed; further depreciation is refused.
    assert!(matches!(
        store.ledger().get_asset(id).unwrap().status,
        ledger_core::AssetStatus::Disposed { .. }
    ));
    assert!(store.run_depreciation(id, "2026-03").is_err());
    assert!(
        store
            .dispose_asset(id, date(2026, 3, 1), usd("1"), bank, gain)
            .is_err()
    );
    store.verify().unwrap();
}

#[test]
fn disposal_at_loss_uses_debit_side() {
    let (_dir, mut store) = open_store();
    let (cost, accum, expense, bank, _) = asset_accounts(&mut store);
    let loss = store
        .add_account(
            "6900",
            "Loss on Disposal",
            AccountType::Expense,
            None,
            false,
        )
        .unwrap();
    let id = van(&mut store, 60_000, cost, accum, expense);
    // Dispose 2026-01-31 with no prior depreciation runs: the final-period
    // depreciation (Jan, 548.39) is auto-posted first. Book value 59451.61,
    // sell for 50,000 → loss 9451.61.
    let entry_id = store
        .dispose_asset(id, date(2026, 1, 31), usd("50000"), bank, loss)
        .unwrap();
    let entry = store.ledger().get_entry(entry_id).unwrap();
    let loss_line = entry.lines().iter().find(|l| l.account == loss).unwrap();
    assert_eq!(loss_line.side, Side::Debit);
    assert_eq!(loss_line.amount.amount, Decimal::new(945161, 2));
    // The auto-posted depreciation entry exists for the disposal month.
    let posted = store
        .ledger()
        .posted_entries()
        .filter(|e| e.description().contains("(final)"))
        .count();
    assert_eq!(posted, 1);
    store.verify().unwrap();
}

#[test]
fn rejected_asset_registration_journals_nothing() {
    let (_dir, mut store) = open_store();
    let (cost, accum, expense, _, _) = asset_accounts(&mut store);
    let bad = NewAsset {
        description: "Bad".to_string(),
        purchase_date: date(2026, 1, 1),
        cost: usd("1000"),
        salvage_value: usd("1000"), // salvage >= cost: invalid
        useful_life_months: 12,
        cost_account: cost,
        accum_depr_account: accum,
        expense_account: expense,
    };
    assert!(store.register_asset(bad).is_err());
    // Journal holds only the setup lines; no asset.registered.
    let events = store.events_since(0, 100).unwrap();
    assert!(events.iter().all(|e| e.event_type != "asset.registered"));
    // And the allocator did not advance.
    let id = van(&mut store, 1_000, cost, accum, expense);
    assert_eq!(id, 1);
}
