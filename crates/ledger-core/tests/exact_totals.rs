//! Totals past exact arithmetic are refused, never a panic (kernel sweep
//! unit K-U4; rules.md B-21, B-28).
//!
//! A replayed journal line is accepted "of any size", so a restored
//! figure can sit at `Decimal::MAX`; every sum the engine makes of such
//! figures must return `TooLargeToCompute` (or skip a check), where a raw
//! `+` / `.sum()` used to crash the process. Each test names the site it
//! guards; before this unit each of them panicked.

use ledger_core::arap::{DocumentLine, document_total};
use ledger_core::audit::check_articulation;
use ledger_core::money::{add_exact, sub_exact, sum_exact, sum_exact_of, sum_exact_try};
use ledger_core::periods::{
    allocate, allocate_last_absorbs, try_allocate, try_allocate_last_absorbs,
};
use ledger_core::{LedgerError, kernel};
use rust_decimal::Decimal;

fn too_large<T: std::fmt::Debug>(r: Result<T, LedgerError>) {
    assert!(
        matches!(r, Err(LedgerError::TooLargeToCompute(_))),
        "expected TooLargeToCompute, got {r:?}"
    );
}

fn dec(s: &str) -> Decimal {
    s.parse().unwrap()
}

#[test]
fn the_engine_sums_refuse_at_the_edge() {
    let max = Decimal::MAX;
    too_large(sum_exact([max, Decimal::ONE], "a column"));
    too_large(sum_exact_of([max, max], |x| x, "a column"));
    too_large(sum_exact_try([(max, -max)], |_| None, "a column"));
    too_large(add_exact(max, Decimal::ONE, "a balance"));
    too_large(sub_exact(-max, Decimal::ONE, "a balance"));
    // And the ordinary case is exact.
    assert_eq!(
        sum_exact([dec("1.10"), dec("2.20"), dec("-3.30")], "a column").unwrap(),
        Decimal::ZERO
    );
    assert_eq!(
        add_exact(dec("0.10"), dec("0.20"), "x").unwrap(),
        dec("0.30")
    );
}

/// arap::document_total (store: create_invoice / create_bill).
#[test]
fn a_documents_lines_past_exact_arithmetic_are_refused() {
    let line = |amount| DocumentLine {
        description: "x".into(),
        amount,
        class_id: None,
        account: None,
        quantity: None,
        unit_price: None,
    };
    let big = [line(Decimal::MAX), line(Decimal::ONE)];
    too_large(document_total(&big, "the invoice"));
    let ok = [line(dec("10.00")), line(dec("5.25"))];
    assert_eq!(document_total(&ok, "the invoice").unwrap(), dec("15.25"));
}

/// audit::check_articulation (current_earnings + closed was a raw +).
#[test]
fn the_articulation_check_skips_what_it_cannot_compute() {
    assert!(check_articulation(Decimal::MAX, Decimal::MAX, Decimal::MAX).is_none());
    // An ordinary mismatch is still reported, and a match is not.
    assert!(check_articulation(dec("10"), dec("4"), dec("5")).is_some());
    assert!(check_articulation(dec("10"), dec("4"), dec("6")).is_none());
}

/// periods::allocate / allocate_last_absorbs (amount * share was raw: a
/// replayed close with a huge journaled amount). The frozen rules'
/// results do not move; only the overflow is an error.
#[test]
fn the_frozen_close_splits_refuse_instead_of_panicking() {
    let two = Decimal::from(2);
    too_large(try_allocate(Decimal::MAX, &[two]));
    too_large(try_allocate_last_absorbs(Decimal::MAX, &[two]));
    too_large(try_allocate(-Decimal::MAX, &[two]));

    // The same split as the infallible rule for ordinary inputs.
    for amount in ["100.00", "0.07", "-33.33", "0.01", "1234567.89"] {
        let shares = [dec("0.5"), dec("0.3"), dec("0.2")];
        let a = dec(amount);
        assert_eq!(try_allocate(a, &shares).unwrap(), allocate(a, &shares));
        assert_eq!(
            try_allocate_last_absorbs(a, &shares).unwrap(),
            allocate_last_absorbs(a, &shares)
        );
    }
}

/// The kernel total over the line cap's edge: ten of the largest allowed
/// line sum exactly (no refusal where the cap says it must work).
#[test]
fn lines_at_the_cap_still_sum_exactly() {
    let cap = kernel::MAX_LINE_AMOUNT;
    let ten = kernel::total(std::iter::repeat_n(cap, 10)).unwrap();
    assert_eq!(ten, cap * Decimal::from(10));
}
