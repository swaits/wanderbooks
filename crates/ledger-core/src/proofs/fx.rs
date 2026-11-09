//! U1 proof harnesses: foreign-currency conversion (kernel sweep).
//!
//! `kernel::fx_booked` on `i64`: amounts and the rate are plain
//! integers (the rate's precision is the value's own, as a `Decimal`
//! rate's is its scale), the step a fixed constant. A symbolic step was
//! not used: `round_step`'s harnesses found CBMC's bit-blasted division
//! by a symbolic `i64` intractable (K-U2), and `fx_booked` rounds through
//! `round_step`, so the steps are the fixed set below — an even step (a
//! tie exists), odd steps, and a step well above one. The bridge's
//! exact-reference property carries the same laws to every `Decimal`
//! step, and bookspec's Lean proves them for any step.

use crate::kernel::{Rounding, fx_booked};
use kani::cover;

/// The fixed steps: one, evens (a tie exists), an odd step, a decimal
/// step, and a step well above the amounts' products.
const STEPS: [i64; 5] = [1, 2, 5, 10, 100];

fn any_foreign() -> i64 {
    let value: i64 = kani::any();
    kani::assume(value >= -256 && value <= 256);
    value
}

fn any_rate() -> i64 {
    let value: i64 = kani::any();
    kani::assume(value >= 1 && value <= 16);
    value
}

/// The result is a whole multiple of the minor unit.
#[kani::proof]
#[kani::unwind(8)]
fn fx_booked_is_whole_unit() {
    let foreign = any_foreign();
    let rate = any_rate();
    for step in STEPS {
        let booked = fx_booked(foreign, rate, step, Rounding::HalfAwayFromZero)
            .expect("no overflow under these bounds");
        assert_eq!(booked % step, 0, "a whole multiple of the step");
    }
    cover!(
        (foreign * rate).rem_euclid(10) != 0,
        "a non-zero remainder rounded to the step"
    );
    cover!(foreign < 0, "a negative foreign amount");
}

/// The result is within half a unit of the exact product, a tie at
/// exactly half included.
#[kani::proof]
#[kani::unwind(8)]
fn fx_booked_within_half_unit() {
    let foreign = any_foreign();
    let rate = any_rate();
    let exact = foreign * rate;
    for step in STEPS {
        let booked = fx_booked(foreign, rate, step, Rounding::HalfAwayFromZero)
            .expect("no overflow under these bounds");
        let distance = (booked - exact).abs();
        assert!(2 * distance <= step, "within half a unit of the product");
    }
    cover!(exact.rem_euclid(10) != 0, "a non-zero remainder");
    cover!(exact.rem_euclid(10) == 5, "an exact tie at step 10");
    cover!(
        exact.rem_euclid(2) == 1,
        "an exact tie at step 2 (half of an even step)"
    );
}

/// Positive foreign and rate book a non-negative amount, and a
/// positive one once the product reaches half a unit (a smaller
/// product rounds to zero, which `add_line_fx` refuses).
#[kani::proof]
#[kani::unwind(8)]
fn fx_booked_positive_for_positive() {
    let foreign: i64 = kani::any();
    kani::assume(foreign >= 1 && foreign <= 256);
    let rate = any_rate();
    let exact = foreign * rate;
    for step in STEPS {
        let booked = fx_booked(foreign, rate, step, Rounding::HalfAwayFromZero)
            .expect("no overflow under these bounds");
        assert!(booked >= 0, "never negative for positive inputs");
        if 2 * exact >= step {
            assert!(booked > 0, "positive once the product reaches half a unit");
        }
    }
    cover!(exact >= 100, "a product past the largest step");
    cover!(2 * exact < 100, "a product under half the largest step");
}

/// Overflow and a non-positive rate are refused, never a panic or a
/// wrapped product.
#[kani::proof]
fn fx_booked_refuses_overflow() {
    assert_eq!(
        fx_booked(i64::MAX, 2, 1, Rounding::HalfAwayFromZero),
        None,
        "a product past i64 is refused"
    );
    assert_eq!(
        fx_booked(100, i64::MAX, 1, Rounding::HalfAwayFromZero),
        None,
        "a huge rate is refused the same way"
    );
    let foreign = any_foreign();
    let rate: i64 = kani::any();
    kani::assume(rate <= 0);
    for step in STEPS {
        assert_eq!(
            fx_booked(foreign, rate, step, Rounding::HalfAwayFromZero),
            None,
            "a non-positive rate is refused"
        );
    }
    cover!(rate < 0, "a negative rate");
    cover!(rate == 0, "a zero rate");
}
