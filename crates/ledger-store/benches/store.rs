//! Benchmarks for the store's hot paths (rank 6: a baseline, never
//! correctness; no budget, by decision, 2026-10-13).
//!
//! `cargo bench -p ledger-store --bench store`. Books of 10k two-line
//! entries, on a real disk (under `target/`), since every journal append
//! is synced and a RAM disk would hide that: opening them (verify the hash
//! chain, replay, catch the database up), what the viewer does to load
//! them, a full verify, and posting one more entry (four synced appends).

use chrono::NaiveDate;
use criterion::{Criterion, criterion_group, criterion_main};
use ledger_core::{AccountType, Currency, Money, Side};
use ledger_store::{Snapshot, Store};
use rust_decimal::Decimal;
use std::path::{Path, PathBuf};

const ENTRIES: u32 = 10_000;

fn post(store: &mut Store, n: u32, a: u64, b: u64) {
    let day = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap() + chrono::Days::new(u64::from(n % 365));
    let e = store.create_entry(day, "bench").unwrap();
    let m = Money::new(Decimal::new(i64::from(n % 50_000) + 1, 2), Currency::USD);
    store.add_line(e, a, Side::Debit, m).unwrap();
    store.add_line(e, b, Side::Credit, m).unwrap();
    store.post_entry(e).unwrap();
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let target = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &target);
        } else {
            std::fs::copy(e.path(), target).unwrap();
        }
    }
}

/// 10k entries over 40 accounts, built in a temp dir (fast) and copied to
/// disk under target/ (honest syncs). Built once per run.
fn books() -> PathBuf {
    let disk = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/bench-books");
    let _ = std::fs::remove_dir_all(&disk);
    let tmp = tempfile::tempdir().unwrap();
    {
        let mut store = Store::create(tmp.path(), Currency::USD).unwrap();
        let types = [
            AccountType::Asset,
            AccountType::Liability,
            AccountType::Equity,
            AccountType::Revenue,
            AccountType::Expense,
        ];
        let ids: Vec<u64> = (0..40)
            .map(|i| {
                store
                    .add_account(&format!("{}", 1000 + i), "a", types[i % 5], None, false)
                    .unwrap()
            })
            .collect();
        for n in 0..ENTRIES {
            let a = ids[(n as usize * 7) % 40];
            let b = ids[(n as usize * 13 + 1) % 40];
            if a != b {
                post(&mut store, n, a, b);
            }
        }
    }
    let _ = std::fs::remove_file(tmp.path().join(ledger_store::LOCK_FILE));
    copy_dir(tmp.path(), &disk);
    disk
}

fn bench(c: &mut Criterion) {
    let dir = books();
    let mut g = c.benchmark_group("store_10k");
    g.sample_size(10);
    g.bench_function("open", |b| b.iter(|| Store::open(&dir).unwrap()));
    g.bench_function("viewer_load", |b| b.iter(|| Snapshot::load(&dir).unwrap()));
    g.bench_function("verify", |b| {
        let store = Store::open(&dir).unwrap();
        b.iter(|| store.verify().unwrap())
    });
    g.bench_function("post_one_entry", |b| {
        let mut store = Store::open(&dir).unwrap();
        let mut n = ENTRIES;
        b.iter(|| {
            n += 1;
            post(&mut store, n, 1, 2);
        })
    });
    g.finish();
}

criterion_group!(benches, bench);
criterion_main!(benches);
