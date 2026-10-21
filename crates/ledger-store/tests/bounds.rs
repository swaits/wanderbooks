//! The store refuses what the books can't hold, before it journals
//! anything, never a panic (P0 review round 3, S1: a panic stops the books
//! server). The bounds are for new commands only: books an earlier version
//! wrote past them open exactly as written (rules.md B-28; P0 review round
//! 4), each bound's pair here or in tests/fixtures/old-books.
//!
//! ```text
//! GOAL: no command reaches an overflow, a date chrono can't hold, or a
//!       run without end; no journal line an earlier version wrote is
//!       refused for a bound added after it.
//! SOURCE: DECISION (P0 review round 3, 2026-10-16, S1; round 4,
//!         2026-10-17); ledger_core calendar.rs and kernel.rs
//!         MAX_LINE_AMOUNT.
//! ```

use chrono::NaiveDate;
use ledger_core::kernel::MAX_LINE_AMOUNT;
use ledger_core::{AccountType, Currency, Frequency, Money, NewRecurring, Side};
use ledger_store::{JournalWriter, MAX_RECURRING_RUN, Statement, Store, StoreError};
use rust_decimal::Decimal;
use serde_json::json;
use std::path::PathBuf;

/// A test's dir, removed when the test ends, pass or panic.
struct TestDir(PathBuf);

impl std::ops::Deref for TestDir {
    type Target = std::path::Path;
    fn deref(&self) -> &std::path::Path {
        &self.0
    }
}

impl AsRef<std::path::Path> for TestDir {
    fn as_ref(&self) -> &std::path::Path {
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
        "wanderbooks-store-bounds-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    TestDir(dir)
}

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

/// The old-books fixture `name` (tests/fixtures/old-books: written by main
/// 188c90ea), opened from a copy (kept while the guard lives).
fn open_old_books(name: &str) -> (TestDir, Store) {
    let from = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/old-books")
        .join(name);
    let dir = test_dir(&format!("old-{name}"));
    std::fs::copy(
        from.join(ledger_store::JOURNAL_FILE),
        dir.join(ledger_store::JOURNAL_FILE),
    )
    .unwrap();
    let blobs = from.join("blobs");
    if blobs.exists() {
        for e in walk(&blobs) {
            let to = dir.join(e.strip_prefix(&from).unwrap());
            std::fs::create_dir_all(to.parent().unwrap()).unwrap();
            std::fs::copy(&e, to).unwrap();
        }
    }
    let store = Store::open(&dir).unwrap_or_else(|e| panic!("{name}: {e}"));
    (dir, store)
}

/// Every file under `dir`.
fn walk(dir: &std::path::Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else {
            out.push(p);
        }
    }
    out
}

/// A statement's balances are no larger than a line, and its period is in
/// the calendar; nothing is journaled for one that isn't.
#[test]
fn a_statement_the_books_cant_hold_is_refused() {
    let dir = test_dir("statement");
    let mut s = Store::create(&dir, Currency::USD).unwrap();
    let cash = s
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    let (att, _, _) = s
        .file_document("statement", "s.pdf", "application/pdf", b"%PDF s", None)
        .unwrap();
    let st = |closing: Decimal, end: NaiveDate| Statement {
        attachment_id: att,
        account_id: cash,
        period_start: d(2026, 1, 1),
        period_end: end,
        opening_balance: Decimal::ZERO,
        closing_balance: closing,
    };
    let journal = std::fs::read(dir.join(ledger_store::JOURNAL_FILE)).unwrap();
    for bad in [
        st(-Decimal::MAX.round_dp(2), d(2026, 1, 31)),
        st(MAX_LINE_AMOUNT + Decimal::ONE, d(2026, 1, 31)),
        st(Decimal::ONE, d(10000, 1, 1)),
    ] {
        assert!(
            matches!(
                s.record_statement(bad),
                Err(StoreError::InvalidStatement(_))
            ),
            "refused"
        );
    }
    assert_eq!(
        std::fs::read(dir.join(ledger_store::JOURNAL_FILE)).unwrap(),
        journal
    );
    s.record_statement(st(-MAX_LINE_AMOUNT, d(2026, 1, 31)))
        .unwrap();
    // A statement an earlier version recorded past both bounds opens, its
    // balances as recorded.
    let (_dir, old) = open_old_books("x_statement_max");
    let recorded = old.statements(None).unwrap();
    assert_eq!(
        recorded[0].closing_balance,
        "792281625142643375935439503.35".parse::<Decimal>().unwrap()
    );
}

/// One run of the recurring entries makes at most MAX_RECURRING_RUN
/// drafts: a far-off date is refused whole, journaling nothing.
#[test]
fn a_recurring_run_has_an_end() {
    let dir = test_dir("recurring");
    let mut s = Store::create(&dir, Currency::USD).unwrap();
    let cash = s
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    let rent = s
        .add_account("6000", "Rent", AccountType::Expense, None, false)
        .unwrap();
    let one = Money::new(Decimal::ONE, Currency::USD);
    s.register_recurring(NewRecurring {
        description: "Rent".into(),
        lines: vec![(rent, Side::Debit, one), (cash, Side::Credit, one)],
        frequency: Frequency::Monthly,
        start: d(2026, 1, 1),
        end: None,
    })
    .unwrap();
    let journal = std::fs::read(dir.join(ledger_store::JOURNAL_FILE)).unwrap();
    let err = s.run_recurring(d(9999, 12, 31)).unwrap_err().to_string();
    assert!(err.contains("more than 1000 at once"), "{err}");
    assert_eq!(
        std::fs::read(dir.join(ledger_store::JOURNAL_FILE)).unwrap(),
        journal
    );
    // Exactly MAX_RECURRING_RUN goes through: 1000 months from 2026-01.
    let last = d(2026, 1, 1)
        .checked_add_months(chrono::Months::new(MAX_RECURRING_RUN as u32 - 1))
        .unwrap();
    assert_eq!(s.run_recurring(last).unwrap().len(), MAX_RECURRING_RUN);
    assert!(s.run_recurring(last).unwrap().is_empty());
    // One run an earlier version made of more than that opens, every
    // draft as generated.
    let (_dir, old) = open_old_books("x_recurring_run_1044");
    assert_eq!(old.ledger().next_entry_id(), 1045);
    old.verify().unwrap();
}

/// A disposal's proceeds are a line of its entry: no larger than one.
#[test]
fn disposal_proceeds_larger_than_a_line_are_refused() {
    let dir = test_dir("dispose");
    let mut s = Store::create(&dir, Currency::USD).unwrap();
    let mut add = |code: &str, ty| s.add_account(code, code, ty, None, false).unwrap();
    let equipment = add("1500", AccountType::Asset);
    let accum = add("1501", AccountType::Asset);
    let depreciation = add("6100", AccountType::Expense);
    let usd = |a: Decimal| Money::new(a, Currency::USD);
    let id = s
        .register_asset(ledger_core::NewAsset {
            description: "Laptop".into(),
            in_service_date: d(2026, 1, 1),
            cost: usd(Decimal::new(240000, 2)),
            salvage_value: usd(Decimal::ZERO),
            useful_life_months: 24,
            cost_account: equipment,
            accum_depr_account: accum,
            expense_account: depreciation,
        })
        .unwrap();
    for proceeds in [Decimal::MAX, MAX_LINE_AMOUNT + Decimal::ONE] {
        assert!(matches!(
            s.dispose_preview(id, d(2026, 6, 30), usd(proceeds)),
            Err(StoreError::Ledger(
                ledger_core::LedgerError::AmountOutOfRange
            ))
        ));
    }
    assert!(matches!(
        s.dispose_preview(id, NaiveDate::MAX, usd(Decimal::ONE)),
        Err(StoreError::Ledger(
            ledger_core::LedgerError::DateOutOfRange(_)
        ))
    ));
    s.dispose_preview(id, d(2026, 6, 30), usd(MAX_LINE_AMOUNT))
        .unwrap();
    // A period past the calendar is refused, never a panic.
    for period in ["999999-01", "10000-01", "262142-12"] {
        assert!(s.run_depreciation(id, period).is_err(), "{period}");
    }
    // Depreciation an earlier version posted for 10000-01 opens.
    let (_dir, old) = open_old_books("x_depreciation_year10000");
    assert_eq!(old.ledger().get_entry(2).unwrap().date(), d(10000, 1, 31));
}

/// The bounds are for new commands only (rules.md B-28: posted is
/// forever). A journal line an earlier version wrote past them (a date
/// after 9999, up to chrono's last day, as chrono writes it: "+10000-01-01";
/// a draft or recurring line past the line limit) opens, in the writer and the viewer alike, exactly as written. A
/// line whose figures exact arithmetic can't hold at all (a recurring
/// template whose debits overflow Decimal: no version could write it, and
/// main panicked replaying it) is refused, never a panic.
#[test]
fn journal_lines_an_earlier_version_wrote_past_the_bounds_open() {
    type Line = (&'static str, serde_json::Value);
    let accounts = || -> Vec<Line> {
        vec![
            (
                "account.added",
                json!({"id": 1, "code": "6000", "name": "Rent", "account_type": "Expense",
                       "parent": null, "placeholder": false, "currency": "USD"}),
            ),
            (
                "account.added",
                json!({"id": 2, "code": "1000", "name": "Cash", "account_type": "Asset",
                       "parent": null, "placeholder": false, "currency": "USD"}),
            ),
        ]
    };
    let recurring = |debit: String| -> Line {
        (
            "recurring.registered",
            json!({"id": 1, "description": "x", "frequency": "monthly",
                   "start": "+262142-01-01",
                   "lines": [
                       {"account": 1, "side": "debit", "amount": debit, "currency": "USD"},
                       {"account": 1, "side": "debit", "amount": "1", "currency": "USD"},
                       {"account": 2, "side": "credit", "amount": "1", "currency": "USD"}]}),
        )
    };
    let big = (MAX_LINE_AMOUNT * Decimal::TWO).to_string();
    let mut past_the_bounds = accounts();
    past_the_bounds.extend([
        (
            "entry.created",
            json!({"id": 1, "date": "+262142-12-31", "description": "far"}),
        ),
        (
            "entry.created",
            json!({"id": 2, "date": "+10000-01-01", "description": "far"}),
        ),
        (
            "entry.line_added",
            json!({"id": 2, "account": 1, "side": "debit", "amount": big, "currency": "USD"}),
        ),
    ]);
    let mut balanced = past_the_bounds.clone();
    balanced.push((
        "recurring.registered",
        json!({"id": 1, "description": "x", "frequency": "monthly", "start": "0000-01-01",
               "lines": [
                   {"account": 1, "side": "debit", "amount": big, "currency": "USD"},
                   {"account": 2, "side": "credit", "amount": big, "currency": "USD"}]}),
    ));
    let mut overflow = accounts();
    overflow.push(recurring(Decimal::MAX.to_string()));
    for (n, (lines, opens)) in [(balanced, true), (overflow, false)]
        .into_iter()
        .enumerate()
    {
        let dir = test_dir(&format!("journal-{n}"));
        let mut w = JournalWriter::open(&dir.join(ledger_store::JOURNAL_FILE)).unwrap();
        w.append("store.created", json!({"base_currency": "USD"}))
            .unwrap();
        for (t, p) in lines {
            w.append(t, p).unwrap();
        }
        drop(w);
        if opens {
            let store = Store::open(&dir).unwrap_or_else(|e| panic!("case {n}: {e}"));
            store.verify().unwrap();
            let l = store.ledger();
            assert_eq!(l.get_entry(1).unwrap().date(), NaiveDate::MAX);
            assert_eq!(l.get_entry(2).unwrap().date(), d(10000, 1, 1));
            assert_eq!(
                l.get_entry(2).unwrap().lines()[0].amount.amount,
                MAX_LINE_AMOUNT * Decimal::TWO
            );
            let r = l.recurring_entries().next().unwrap();
            assert_eq!(r.start, d(0, 1, 1));
            drop(store);
            ledger_store::Snapshot::load(&dir).unwrap_or_else(|e| panic!("case {n}: {e}"));
        } else {
            let err = Store::open(&dir)
                .err()
                .unwrap_or_else(|| panic!("case {n}: the writer opened it"));
            assert!(
                err.to_string().contains("too large to compute"),
                "case {n}: {err}"
            );
            assert!(
                ledger_store::Snapshot::load(&dir).is_err(),
                "case {n}: the viewer loaded it"
            );
        }
    }
}
