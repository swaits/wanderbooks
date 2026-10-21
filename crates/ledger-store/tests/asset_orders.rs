//! Every order of depreciation runs, reversals, re-runs, re-instatements,
//! disposals and closed periods, on one asset: exhaustive over short
//! sequences, and random over longer ones (P3 review round 2).
//!
//! ```text
//! GOAL: whatever a human does, and in whatever order, the books never
//!       carry an asset's depreciation past its schedule at a month end,
//!       never carry a month twice, take exactly cost less salvage over
//!       the life, and dispose at proceeds less the book value of the
//!       days held.
//! SOURCE: docs/spec/rules.md B-9, B-10, B-15; plan/served.md P3 (settled
//!         in design: a skipped month is caught up in the month the run is
//!         made); Ledger::dispose_asset INVARIANT 1.
//! INVARIANTS (checked after every command, refused or not):
//!   1. A refused command journals nothing.
//!   2. At every month end before the disposal (all of them, while the
//!      asset is active), the depreciation expense is the accumulated
//!      depreciation credit, and at most the schedule through that day.
//!   3. No month has two runs in effect.
//!   4. Disposed on D: at the end of D's month and in the end, the
//!      expense is the schedule through D, accumulated depreciation 0.00,
//!      the cost cleared, and the gain proceeds less (cost less that).
//!   5. Active, then run month by month past the life (a closed month
//!      refused): the expense is exactly cost less salvage, and every
//!      month end still within the schedule.
//!   6. The books replay to the same figures (verify, at the search's
//!      leaves and each random sequence's end).
//! ```
//!
//! The asset: 3,000.00, no salvage, 3 months from 2026-01-16, so January
//! holds 16 of its 31 days (516.13), February and March 1,000.00 each, and
//! April 1 to 15 the rest (483.87): rounding in the first and last months.

use chrono::NaiveDate;
use ledger_core::{AccountType, AssetStatus, Currency, Money, NewAsset};
use ledger_store::{JOURNAL_FILE, Store, db_file};
use proptest::prelude::*;
use rust_decimal::Decimal;
use std::path::{Path, PathBuf};

/// A test's dir, removed when the test ends, pass or panic.
struct TestDir(PathBuf);

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn test_dir(name: &str) -> TestDir {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "wanderbooks-orders-{name}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    TestDir(dir)
}

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

fn usd(cents: i64) -> Money {
    Money::new(Decimal::new(cents, 2), Currency::USD)
}

fn month_end(m: u32) -> NaiveDate {
    ledger_core::schedules::month_end(day(2026, m, 1))
}

/// The books' accounts, by code.
const COST: &str = "1500";
const ACCUM: &str = "1501";
const EXPENSE: &str = "6100";
const GAIN: &str = "4900";
const CASH: &str = "1000";
const PROCEEDS: i64 = 100_000;

/// One command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    /// Run 2026-`m`.
    Run(u32),
    /// Reverse the `n`th asset entry made so far (a run, a reversal, the
    /// disposal or its final depreciation), on a day.
    Reverse(usize, NaiveDate),
    /// Dispose of the asset on a day for 1,000.00.
    Dispose(NaiveDate),
    /// Close 2026-`m`.
    Close(u32),
}

/// Books with the asset, a period per month of 2026, and the asset's id.
fn books(dir: &Path) -> u64 {
    let mut s = Store::create(dir, Currency::USD).unwrap();
    let mut add = |code: &str, t| s.add_account(code, code, t, None, false).unwrap();
    let cash = add(CASH, AccountType::Asset);
    let cost = add(COST, AccountType::Asset);
    let accum = add(ACCUM, AccountType::Asset);
    let expense = add(EXPENSE, AccountType::Expense);
    add(GAIN, AccountType::Revenue);
    add("3000", AccountType::Equity);
    let _ = cash;
    for m in 1..=12 {
        s.create_period(&format!("2026-{m:02}"), day(2026, m, 1), month_end(m))
            .unwrap();
    }
    s.register_asset(NewAsset {
        description: "Asset".into(),
        purchase_date: day(2026, 1, 16),
        cost: usd(300_000),
        salvage_value: usd(0),
        useful_life_months: 3,
        cost_account: cost,
        accum_depr_account: accum,
        expense_account: expense,
    })
    .unwrap()
}

/// The entries the asset's commands made, in order: what a reversal picks
/// from. Read from the journal so a copy of the books knows them.
fn asset_entries(store: &Store) -> Vec<u64> {
    let mut out = Vec::new();
    let text = std::fs::read_to_string(store.data_dir().join(JOURNAL_FILE)).unwrap();
    for line in text.lines() {
        let v: serde_json::Value = serde_json::from_str(line).unwrap();
        let p = &v["payload"];
        let id = match v["type"].as_str().unwrap() {
            "asset.depreciation_posted" | "asset.disposed" => p["entry_id"].as_u64(),
            "entry.reversed" => p["id"].as_u64(),
            _ => None,
        };
        out.extend(id);
    }
    out
}

fn code_id(store: &Store, code: &str) -> u64 {
    store.ledger().chart().find_by_code(code).unwrap().id
}

/// An account's movement (debit positive) through the end of `on`, its
/// closing entries left out: what was charged to it, closed or not.
fn balance(store: &Store, code: &str, on: NaiveDate) -> Decimal {
    let l = store.ledger();
    let id = code_id(store, code);
    let closing = l.closing_entry_ids();
    l.posted_entries()
        .filter(|e| e.date() <= on && !closing.contains(&e.id()))
        .flat_map(|e| e.lines().iter())
        .filter(|x| x.account == id)
        .map(|x| match x.side {
            ledger_core::Side::Debit => x.amount.amount,
            ledger_core::Side::Credit => -x.amount.amount,
        })
        .sum()
}

/// Apply one command; Ok(true) when it changed the books.
fn apply(store: &mut Store, asset: u64, op: Op) -> bool {
    let head = store.head();
    let done = match op {
        Op::Run(m) => store
            .run_depreciation(asset, &format!("2026-{m:02}"))
            .map(|r| r.entry_id.is_some() && !r.already_posted),
        Op::Reverse(n, on) => {
            let entries = asset_entries(store);
            match entries.get(n) {
                Some(&e) => store.reverse_entry(e, on, "reversed").map(|_| true),
                None => Ok(false),
            }
        }
        Op::Dispose(on) => {
            let (cash, gain) = (code_id(store, CASH), code_id(store, GAIN));
            store
                .dispose_asset(asset, on, usd(PROCEEDS), cash, gain)
                .map(|_| true)
        }
        Op::Close(m) => {
            let id = store
                .ledger()
                .list_periods()
                .into_iter()
                .find(|p| p.name == format!("2026-{m:02}"))
                .unwrap()
                .id;
            store.close_period(id).map(|_| true)
        }
    };
    match done {
        Ok(changed) => {
            if !changed {
                assert_eq!(store.head(), head, "{op:?} changed nothing yet journaled");
            }
            changed
        }
        Err(_) => {
            assert_eq!(store.head(), head, "{op:?} was refused yet journaled");
            false
        }
    }
}

/// The month ends checked: from before the asset through after the life.
fn ends() -> Vec<NaiveDate> {
    std::iter::once(day(2025, 12, 31))
        .chain((1..=12).map(month_end))
        .chain([NaiveDate::MAX])
        .collect()
}

/// Invariants 2 to 4 (and 6, when `verify`).
fn check(store: &Store, asset: u64, ops: &[Op], verify: bool) {
    let a = store.ledger().get_asset(asset).unwrap().clone();
    let mut active = a.clone();
    active.status = AssetStatus::Active;
    let sched = |on: NaiveDate| ledger_core::accumulated_depreciation(&active, on).unwrap();
    // INVARIANT 3: one run in effect per month.
    let text = std::fs::read_to_string(store.data_dir().join(JOURNAL_FILE)).unwrap();
    let mut live: Vec<String> = Vec::new();
    for line in text.lines() {
        let v: serde_json::Value = serde_json::from_str(line).unwrap();
        if v["type"] == "asset.depreciation_posted"
            && store
                .ledger()
                .in_effect(v["payload"]["entry_id"].as_u64().unwrap())
        {
            let p = v["payload"]["period"].as_str().unwrap().to_string();
            assert!(!live.contains(&p), "{ops:?}: two runs in effect for {p}");
            live.push(p);
        }
    }
    let disposed = match a.status {
        AssetStatus::Disposed { date, .. } => Some(date),
        AssetStatus::Active => None,
    };
    for e in ends() {
        let x = balance(store, EXPENSE, e);
        let acc = balance(store, ACCUM, e);
        if disposed.is_none_or(|d| e < d) {
            assert_eq!(x, -acc, "{ops:?}: expense and accumulated apart on {e}");
            assert!(
                x <= sched(e),
                "{ops:?}: {x} carried on {e}, past the schedule's {}",
                sched(e)
            );
        }
    }
    if let Some(d) = disposed {
        let target = sched(d);
        for e in [ledger_core::schedules::month_end(d), NaiveDate::MAX] {
            assert_eq!(
                balance(store, EXPENSE, e),
                target,
                "{ops:?}: expense on {e}"
            );
            assert_eq!(
                balance(store, ACCUM, e),
                Decimal::ZERO,
                "{ops:?}: accum on {e}"
            );
            // No purchase entry here: the disposal credits the whole cost.
            assert_eq!(
                balance(store, COST, e),
                -a.cost.amount,
                "{ops:?}: cost on {e}"
            );
            let gain = Decimal::new(PROCEEDS, 2) - (a.cost.amount - target);
            assert_eq!(balance(store, GAIN, e), -gain, "{ops:?}: gain on {e}");
        }
    }
    if verify {
        store.verify().unwrap();
    }
}

/// Invariant 5, on a copy: run every month in turn past the life.
fn check_completion(dir: &Path, asset: u64, ops: &[Op]) {
    let copy = test_dir("complete");
    copy_books(dir, &copy.0);
    let mut store = Store::open(&copy.0).unwrap();
    if store.ledger().get_asset(asset).unwrap().status != AssetStatus::Active {
        return;
    }
    for m in 1..=12 {
        apply(&mut store, asset, Op::Run(m));
    }
    let a = store.ledger().get_asset(asset).unwrap().clone();
    for e in ends() {
        let x = balance(&store, EXPENSE, e);
        assert!(
            x <= ledger_core::accumulated_depreciation(&a, e).unwrap(),
            "{ops:?} then every month: {x} on {e}"
        );
    }
    let whole = a.cost.amount - a.salvage_value.amount;
    assert_eq!(
        balance(&store, EXPENSE, NaiveDate::MAX),
        whole,
        "{ops:?} then every month: the lifetime total"
    );
    assert_eq!(balance(&store, ACCUM, NaiveDate::MAX), -whole, "{ops:?}");
}

/// Copy the books (journal and projection) to `to`, closed.
fn copy_books(from: &Path, to: &Path) {
    for f in [JOURNAL_FILE.to_string(), db_file()] {
        let src = from.join(&f);
        if src.exists() {
            std::fs::copy(&src, to.join(&f)).unwrap();
        }
    }
}

/// Every command at a node: runs of each month, reversals of each asset
/// entry made so far on three days (before, in and after most entries'
/// months), two disposals (mid-month, and in the life's final month), and
/// closes of January and February.
fn alphabet(entries: usize) -> Vec<Op> {
    let mut ops: Vec<Op> = (1..=5).map(Op::Run).collect();
    for n in 0..entries {
        for on in [day(2026, 1, 31), day(2026, 3, 31), day(2026, 5, 2)] {
            ops.push(Op::Reverse(n, on));
        }
    }
    ops.extend([Op::Dispose(day(2026, 2, 15)), Op::Dispose(day(2026, 4, 10))]);
    ops.extend([Op::Close(1), Op::Close(2)]);
    ops
}

/// Depth-first over every sequence of commands up to `depth` long, each
/// node a closed copy of its parent's books.
fn explore(dir: &Path, asset: u64, ops: &mut Vec<Op>, depth: usize, seen: &mut u64) {
    if ops.len() == depth {
        return;
    }
    let entries = asset_entries(&Store::open(dir).unwrap()).len();
    for op in alphabet(entries) {
        let child = test_dir("node");
        copy_books(dir, &child.0);
        let mut store = Store::open(&child.0).unwrap();
        let changed = apply(&mut store, asset, op);
        ops.push(op);
        *seen += 1;
        if changed {
            check(&store, asset, ops, ops.len() == depth);
            drop(store);
            check_completion(&child.0, asset, ops);
            explore(&child.0, asset, ops, depth, seen);
        }
        ops.pop();
    }
}

/// Every sequence of up to three commands (about half a minute; deeper
/// with WB_ORDERS_DEPTH, for a review, not every run: four took a minute
/// and five five minutes, 2026-10-21, both clean).
#[test]
fn every_order_of_runs_reversals_disposals_and_closes_keeps_the_books_right() {
    let depth = std::env::var("WB_ORDERS_DEPTH")
        .ok()
        .and_then(|d| d.parse().ok())
        .unwrap_or(3);
    let root = test_dir("root");
    let asset = books(&root.0);
    let mut seen = 0;
    explore(&root.0, asset, &mut Vec::new(), depth, &mut seen);
    assert!(seen > 0);
}

fn arb_op() -> impl Strategy<Value = Op> {
    let date = (1u32..=6, 1u32..=31)
        .prop_map(|(m, d)| NaiveDate::from_ymd_opt(2026, m, d).unwrap_or_else(|| month_end(m)));
    prop_oneof![
        4 => (1u32..=6).prop_map(Op::Run),
        4 => (0usize..8, date.clone()).prop_map(|(n, on)| Op::Reverse(n, on)),
        1 => date.prop_map(Op::Dispose),
        1 => (1u32..=4).prop_map(Op::Close),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Longer sequences at random, any day of the first half of 2026.
    #[test]
    fn prop_any_sequence_keeps_the_books_right(ops in prop::collection::vec(arb_op(), 1..10)) {
        let root = test_dir("prop");
        let asset = books(&root.0);
        {
            let mut store = Store::open(&root.0).unwrap();
            for (i, &op) in ops.iter().enumerate() {
                apply(&mut store, asset, op);
                check(&store, asset, &ops[..=i], false);
            }
            store.verify().unwrap();
        }
        check_completion(&root.0, asset, &ops);
    }
}
