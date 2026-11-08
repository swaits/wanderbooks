//! A reversal is dated on or after the entry it reverses, for every kind of
//! entry (D-73, 2026-11-08; rules.md B-9; bookspec BS-ledger-5): a
//! correction comes after what it corrects. A journal an earlier version
//! wrote may hold one dated before its original, and it opens as it was
//! (B-28: bounds bind new commands, never replay).
//!
//! ```text
//! GOAL: no new reversal precedes its original; no old one stops a journal
//!       from opening, or moves a figure.
//! SOURCE: docs/spec/rules.md B-9, B-28; plan/served.md D-73.
//! ```

use chrono::NaiveDate;
use ledger_core::reports::trial_balance;
use ledger_core::{AccountType, Currency, LedgerError, Money, Side};
use ledger_store::{JournalWriter, Store, StoreError};
use rust_decimal::Decimal;
use serde_json::json;
use std::path::PathBuf;

struct TestDir(PathBuf);

impl std::ops::Deref for TestDir {
    type Target = std::path::Path;
    fn deref(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn test_dir(name: &str) -> TestDir {
    let dir = std::env::temp_dir().join(format!(
        "wanderbooks-store-revdates-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    TestDir(dir)
}

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn usd(n: i64) -> Money {
    Money::new(Decimal::new(n, 2), Currency::USD)
}

/// Books with one plain posted entry: rent paid in cash, 100.00, on
/// 2026-03-10. Returns (dir, store, rent, cash, entry).
fn books(name: &str) -> (TestDir, Store, u64, u64, u64) {
    let dir = test_dir(name);
    let mut s = Store::create(&dir, Currency::USD).unwrap();
    let rent = s
        .add_account("6000", "Rent", AccountType::Expense, None, false)
        .unwrap();
    let cash = s
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    let e = s.create_entry(d(2026, 3, 10), "March rent").unwrap();
    s.add_line(e, rent, Side::Debit, usd(10_000)).unwrap();
    s.add_line(e, cash, Side::Credit, usd(10_000)).unwrap();
    s.post_entry(e).unwrap();
    (dir, s, rent, cash, e)
}

/// A plain entry's reversal dated before it is refused, by the check the
/// preview uses and by the command alike, and nothing is journaled; the
/// same day and later are taken.
#[test]
fn a_plain_entrys_reversal_is_not_dated_before_it() {
    let (_dir, mut s, rent, cash, e) = books("plain");
    let head = s.head();
    let tb = trial_balance(s.ledger());
    let before = (tb.total_debits, tb.total_credits);
    for early in [d(2026, 3, 9), d(2026, 1, 1)] {
        assert!(matches!(
            s.check_reverse_entry(e, early),
            Err(StoreError::Ledger(LedgerError::ReversalBeforeOriginal { entry, original, date }))
                if entry == e && original == d(2026, 3, 10) && date == early
        ));
        assert!(matches!(
            s.reverse_entry(e, early, "early"),
            Err(StoreError::Ledger(
                LedgerError::ReversalBeforeOriginal { .. }
            ))
        ));
    }
    assert_eq!(s.head(), head, "a refused reversal journals nothing");
    let tb = trial_balance(s.ledger());
    assert_eq!((tb.total_debits, tb.total_credits), before);
    assert_eq!(s.ledger().account_movement(rent), Decimal::new(10_000, 2));
    // The same day is fine, and the books net to nothing.
    s.check_reverse_entry(e, d(2026, 3, 10)).unwrap();
    let r = s.reverse_entry(e, d(2026, 3, 10), "same day").unwrap();
    assert_eq!(s.ledger().account_movement(rent), Decimal::ZERO);
    assert_eq!(s.ledger().account_movement(cash), Decimal::ZERO);
    // Reversing the reversal (re-instating) is a correction of it too: it
    // can't precede the reversal, and may follow it.
    assert!(matches!(
        s.reverse_entry(r, d(2026, 3, 9), "early"),
        Err(StoreError::Ledger(
            LedgerError::ReversalBeforeOriginal { .. }
        ))
    ));
    s.reverse_entry(r, d(2026, 3, 11), "later").unwrap();
    assert_eq!(s.ledger().account_movement(rent), Decimal::new(10_000, 2));
}

/// A journal an earlier version wrote, holding a reversal dated before its
/// original, opens exactly as it was: same figures, chain verified, in the
/// writer and again after a snapshot-less reopen (B-28).
#[test]
fn a_journal_with_an_earlier_dated_reversal_opens_as_it_was() {
    let (dir, s, rent, cash, e) = books("replay");
    let next = s.ledger().next_entry_id();
    drop(s);
    // What the old behaviour journaled: the mirror of the entry, dated
    // 2026-03-01, nine days before it.
    let mut w = JournalWriter::open(&dir.join(ledger_store::JOURNAL_FILE)).unwrap();
    let line = |account: u64, side: &str| json!({"account": account, "side": side, "amount": "100.00", "currency": "USD"});
    w.append(
        "entry.reversed",
        json!({"id": next, "reverses": e, "date": "2026-03-01",
               "description": "backdated by an earlier version",
               "lines": [line(rent, "credit"), line(cash, "debit")]}),
    )
    .unwrap();
    drop(w);

    let store = Store::open(&dir).unwrap();
    store.verify().unwrap();
    let l = store.ledger();
    let r = l.get_entry(next).unwrap();
    assert_eq!(r.date(), d(2026, 3, 1));
    assert_eq!(r.reversal_of(), Some(e));
    assert_eq!(l.account_movement(rent), Decimal::ZERO);
    assert_eq!(l.account_movement(cash), Decimal::ZERO);
    let tb = trial_balance(l);
    assert_eq!(tb.total_debits, tb.total_credits);
    // The old entry stands reversed, and the books still refuse a new
    // backdated reversal of anything.
    assert!(!l.in_effect(e));
    assert!(matches!(
        store.check_reverse_entry(next, d(2026, 2, 1)),
        Err(StoreError::Ledger(
            LedgerError::ReversalBeforeOriginal { .. }
        ))
    ));
}
