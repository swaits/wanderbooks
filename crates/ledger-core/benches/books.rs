//! Benchmarks for the hot paths (rank 6: a budget, never correctness).
//!
//! `cargo bench -p ledger-core --bench books`. Books of 1k and 10k entries
//! over a 40-account chart: posting, cloning (the store clones the ledger
//! to rehearse every command), trial balance, balance sheet, and a
//! period's P&L. Baselines are recorded in docs/testing.md; there is no
//! budget yet, so nothing gates on them.

use chrono::NaiveDate;
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use ledger_core::reports::{balance_sheet_as_of, profit_and_loss_for_period, trial_balance};
use ledger_core::{AccountType, Currency, Ledger, Money, Side};
use rust_decimal::Decimal;
use std::hint::black_box;

const TYPES: [AccountType; 5] = [
    AccountType::Asset,
    AccountType::Liability,
    AccountType::Equity,
    AccountType::Revenue,
    AccountType::Expense,
];

/// `entries` balanced two-line entries across 2026 over 40 accounts.
fn books(entries: u64) -> Ledger {
    let mut l = Ledger::new(Currency::USD);
    let ids: Vec<u64> = (0..40)
        .map(|i| {
            l.add_account(
                &format!("{}", 1000 + i),
                "a",
                TYPES[i as usize % 5],
                None,
                false,
            )
            .unwrap()
        })
        .collect();
    let mut x: u64 = 0x9e37_79b9;
    for n in 0..entries {
        x = x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        let date =
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap() + chrono::Days::new(n * 365 / entries);
        let e = l.create_entry(date, "bench");
        let m = Money::new(Decimal::new((x % 1_000_000) as i64 + 1, 2), Currency::USD);
        l.add_line(e, ids[(x >> 8) as usize % 40], Side::Debit, m)
            .unwrap();
        l.add_line(e, ids[(x >> 20) as usize % 40], Side::Credit, m)
            .unwrap();
        l.post_entry(e).unwrap();
    }
    l
}

fn bench(c: &mut Criterion) {
    let end = NaiveDate::from_ymd_opt(2026, 12, 31).unwrap();
    for n in [1_000u64, 10_000] {
        let l = books(n);
        let mut g = c.benchmark_group(format!("{n} entries"));
        g.sample_size(20);
        g.bench_function(BenchmarkId::new("clone (store rehearsal)", n), |b| {
            b.iter(|| black_box(l.clone()))
        });
        g.bench_function(BenchmarkId::new("post one more entry", n), |b| {
            b.iter_batched(
                || l.clone(),
                |mut l| {
                    let e = l.create_entry(end, "one more");
                    let m = Money::new(Decimal::new(100, 2), Currency::USD);
                    l.add_line(e, 1, Side::Debit, m).unwrap();
                    l.add_line(e, 2, Side::Credit, m).unwrap();
                    l.post_entry(e).unwrap();
                    l
                },
                criterion::BatchSize::LargeInput,
            )
        });
        g.bench_function(BenchmarkId::new("trial balance", n), |b| {
            b.iter(|| black_box(trial_balance(&l)))
        });
        g.bench_function(BenchmarkId::new("balance sheet", n), |b| {
            b.iter(|| black_box(balance_sheet_as_of(&l, end).unwrap()))
        });
        g.bench_function(BenchmarkId::new("one month's P&L", n), |b| {
            let start = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
            let stop = NaiveDate::from_ymd_opt(2026, 6, 30).unwrap();
            b.iter(|| black_box(profit_and_loss_for_period(&l, start, stop, None).unwrap()))
        });
        g.finish();
    }
}

criterion_group!(benches, bench);
criterion_main!(benches);
