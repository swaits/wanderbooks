//! Documents whose lines' sum is past exact arithmetic are refused, never a
//! panic (kernel sweep unit K-U4; BS-entry-3, BS-entry-4).
//!
//! A posted entry's lines are bounded by the line cap at the moment it
//! posts, restored or new (`entry.posted` validates), so a report's sums
//! over posted lines cannot reach the edge; a document's lines are the
//! figures that can be restored "of any size", and are what this guards.
//!
//! ```text
//! GOAL: a document whose lines sum past exact arithmetic is refused with
//!       a ledger error and journals nothing.
//! SOURCE: BS-entry-4 (a replayed line is accepted of any
//!         size), BS-entry-3 (amounts are exact); bookspec BS-entry-3,
//!         BS-entry-4.
//! ```

use chrono::NaiveDate;
use ledger_core::Currency;
use ledger_core::arap::DocumentLine;
use ledger_store::{Store, StoreError};
use rust_decimal::Decimal;
use std::path::PathBuf;

struct TestDir(PathBuf);

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn test_dir(name: &str) -> TestDir {
    let dir = std::env::temp_dir().join(format!(
        "wanderbooks-store-exact-totals-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    TestDir(dir)
}

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

/// Store::create_invoice / create_bill: lines whose sum is past exact
/// arithmetic are refused (the raw .sum() ran before the checked
/// document_total), and the books are untouched.
#[test]
fn a_document_of_enormous_lines_is_refused_not_a_panic() {
    let dir = test_dir("documents");
    let mut s = Store::create(&dir.0, Currency::USD).unwrap();
    let customer = s.add_customer("Cedar").unwrap();
    let vendor = s.add_vendor("Pine").unwrap();
    let line = |amount: Decimal| DocumentLine {
        description: "x".into(),
        amount,
        class_id: None,
        account: None,
        quantity: None,
        unit_price: None,
    };
    let lines = || vec![line(Decimal::MAX), line(Decimal::MAX)];
    let before = std::fs::read_to_string(dir.0.join(ledger_store::JOURNAL_FILE)).unwrap();
    let invoice = s.create_invoice(customer, d(2026, 1, 2), d(2026, 2, 1), lines());
    assert!(matches!(invoice, Err(StoreError::Ledger(_))), "{invoice:?}");
    let bill = s.create_bill(vendor, d(2026, 1, 2), d(2026, 2, 1), lines());
    assert!(matches!(bill, Err(StoreError::Ledger(_))), "{bill:?}");
    let after = std::fs::read_to_string(dir.0.join(ledger_store::JOURNAL_FILE)).unwrap();
    assert_eq!(before, after, "a refused document journals nothing");
}
