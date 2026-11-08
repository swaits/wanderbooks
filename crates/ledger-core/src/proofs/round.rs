//! U2 proof harnesses: the rounding primitive (kernel sweep).
//!
//! `kernel::round_step` on `i64` money in the smallest unit: the result is
//! a whole multiple of the step; the half modes are nearest; each mode
//! breaks a tie as it says; rounding twice is rounding once; and the sign
//! never flips (so no zero is ever a negative zero).
//!
//! The steps are the fixed constants below rather than a symbolic `step`:
//! a symbolic divisor makes CBMC's bit-blasted division intractable (one
//! harness did not finish in 30 minutes). They cover one, an even step, an
//! odd step, a non-power-of-ten, a decimal step and a larger step; the
//! bridge's exact-reference property carries the same laws to every
//! `Decimal` step, and bookspec's Lean proves the law for any step. The
//! amount is symbolic up to [`VALUE`].

use crate::kernel::{Rounding, round_step};
use kani::cover;

/// Every amount the harnesses explore is within this magnitude.
const VALUE: i64 = 1 << 12;
/// The fixed steps: one, evens (a tie exists), odds, a non-power of ten,
/// a decimal step, and a step well above one.
const STEPS: [i64; 6] = [1, 2, 3, 7, 10, 101];
/// The even steps, where an exact half exists.
const EVEN_STEPS: [i64; 4] = [2, 4, 10, 100];

fn any_mode() -> Rounding {
    let mode: u8 = kani::any();
    kani::assume(mode < 4);
    match mode {
        0 => Rounding::HalfAwayFromZero,
        1 => Rounding::HalfToEven,
        2 => Rounding::TowardZero,
        _ => Rounding::TowardNegativeInfinity,
    }
}

fn any_half_mode() -> Rounding {
    if kani::any() {
        Rounding::HalfAwayFromZero
    } else {
        Rounding::HalfToEven
    }
}

fn any_value() -> i64 {
    let value: i64 = kani::any();
    kani::assume(value >= -VALUE && value <= VALUE);
    value
}

fn check_multiple(value: i64, step: i64, mode: Rounding) {
    let rounded = round_step(value, step, mode).expect("no overflow under these bounds");
    assert_eq!(rounded % step, 0, "a whole multiple of the step");
    if value > 0 {
        assert!(rounded >= 0, "a positive amount never rounds below zero");
    } else if value < 0 {
        assert!(rounded <= 0, "a negative amount never rounds above zero");
    } else {
        assert_eq!(rounded, 0, "zero rounds to zero");
    }
}

/// The result is a whole multiple of the step, and it never crosses zero:
/// a positive amount rounds to a non-negative amount, a negative one to a
/// non-positive amount, and zero to zero.
#[kani::proof]
#[kani::unwind(8)]
fn round_step_is_multiple() {
    let value = any_value();
    let mode = any_mode();
    for step in STEPS {
        check_multiple(value, step, mode);
        cover!(
            step > 1 && value % step != 0,
            "a non-multiple rounded to a step above one"
        );
    }
    cover!(value.abs() % 101 > 1, "a non-multiple of a larger step");
}

fn check_nearest(value: i64, step: i64, mode: Rounding) {
    let rounded = round_step(value, step, mode).expect("no overflow under these bounds");
    let distance = (rounded - value).abs();
    assert!(
        2 * distance <= step,
        "nearest: at most half a step away, a tie at exactly half"
    );
}

/// The half modes are nearest: the result is within half a step of the
/// amount, ties included.
#[kani::proof]
#[kani::unwind(8)]
fn round_step_is_nearest() {
    let value = any_value();
    let mode = any_half_mode();
    for step in STEPS {
        check_nearest(value, step, mode);
    }
    cover!(value % 10 != 0, "a value that actually moves");
    cover!(value.abs() % 10 == 5, "an exact tie (the mode decides it)");
    cover!(value.abs() % 10 != 0 && value.abs() % 10 != 5, "a non-tie");
}

/// At an exact tie every mode picks the multiple its definition names.
/// The tie is built as `whole x step + step/2` (and its negation) with an
/// even fixed step, so it is exact in `i64` and both signs are reached.
#[kani::proof]
#[kani::unwind(8)]
fn round_step_ties() {
    let whole: i64 = kani::any();
    kani::assume(whole >= 0 && whole <= 64);
    let negative: bool = kani::any();
    for step in EVEN_STEPS {
        let half = step / 2;
        let magnitude = whole * step + half;
        let value = if negative { -magnitude } else { magnitude };

        let up = whole + 1;
        let even = if whole % 2 == 0 { whole } else { up };
        let cases = [
            (Rounding::HalfAwayFromZero, up),
            (Rounding::HalfToEven, even),
            (Rounding::TowardZero, whole),
            (
                Rounding::TowardNegativeInfinity,
                if negative { up } else { whole },
            ),
        ];
        for (mode, pick) in cases {
            let want = if negative {
                -(pick * step)
            } else {
                pick * step
            };
            assert_eq!(
                round_step(value, step, mode),
                Some(want),
                "each mode's tie rule"
            );
        }
    }
    cover!(!negative, "a positive tie");
    cover!(negative, "a negative tie");
    cover!(whole % 2 == 1, "HalfToEven rounds an odd floor up");
    cover!(whole % 2 == 0, "HalfToEven keeps an even floor");
}

/// Rounding twice is rounding once, in every mode.
#[kani::proof]
#[kani::unwind(8)]
fn round_step_is_idempotent() {
    let value = any_value();
    let mode = any_mode();
    for step in STEPS {
        let once = round_step(value, step, mode).expect("no overflow under these bounds");
        assert_eq!(round_step(once, step, mode), Some(once));
    }
}

/// No negative zero and no sign flip: zero in gives zero out, and a result
/// that reaches zero only does so from an amount that rounds there.
#[kani::proof]
#[kani::unwind(8)]
fn round_step_never_negative_zero() {
    let value = any_value();
    let mode = any_mode();
    for step in STEPS {
        let rounded = round_step(value, step, mode).expect("no overflow under these bounds");
        if value == 0 {
            assert_eq!(rounded, 0);
        }
        if rounded == 0 {
            assert!(
                value.abs() < step,
                "a zero is a rounding to the zero multiple"
            );
        }
        assert!(
            value == 0 || rounded == 0 || (rounded > 0) == (value > 0),
            "the sign never flips"
        );
    }
    cover!(
        value % 3 != 0 || value % 10 != 0,
        "a non-zero amount rounded to another multiple"
    );
}

/// Toward zero never grows the magnitude and lands less than a step from
/// the amount: `|r| <= |x|` and `|r - x| < step`, for every amount (ties
/// are not special: both modes pick the same multiple whether or not the
/// amount is exactly between two).
#[kani::proof]
#[kani::unwind(8)]
fn round_step_toward_zero_bounds() {
    let value = any_value();
    for step in STEPS {
        let rounded =
            round_step(value, step, Rounding::TowardZero).expect("no overflow under these bounds");
        assert!(rounded.abs() <= value.abs(), "magnitude never grows");
        assert!((rounded - value).abs() < step, "less than a step away");
    }
    cover!(value % 10 != 0 && value > 0, "a positive non-multiple");
    cover!(value % 10 != 0 && value < 0, "a negative non-multiple");
    cover!(
        value.abs() % 10 != 0 && value.abs() % 10 != 5,
        "a non-tie, a non-multiple of ten"
    );
}

/// Toward negative infinity is the floor: `r <= x < r + step`, for every
/// amount of either sign.
#[kani::proof]
#[kani::unwind(8)]
fn round_step_toward_neg_inf_is_floor() {
    let value = any_value();
    for step in STEPS {
        let rounded = round_step(value, step, Rounding::TowardNegativeInfinity)
            .expect("no overflow under these bounds");
        assert!(rounded <= value, "never above the amount");
        assert!(value < rounded + step, "less than a step below it");
    }
    cover!(value % 10 != 0 && value > 0, "a positive non-multiple");
    cover!(
        value % 10 != 0 && value < 0,
        "a negative non-multiple (the floor moves away from zero)"
    );
    cover!(
        value.abs() % 10 != 0 && value.abs() % 10 != 5,
        "a non-tie, a non-multiple of ten"
    );
}

/// No mode flips the sign: a non-negative amount rounds to a non-negative
/// one, and a non-positive amount to a non-positive one.
#[kani::proof]
#[kani::unwind(8)]
fn round_step_sign_preserved() {
    let value = any_value();
    let mode = any_mode();
    for step in STEPS {
        let rounded = round_step(value, step, mode).expect("no overflow under these bounds");
        if value >= 0 {
            assert!(rounded >= 0, "a non-negative amount stays non-negative");
        }
        if value <= 0 {
            assert!(rounded <= 0, "a non-positive amount stays non-positive");
        }
    }
    cover!(value > 0, "a positive amount");
    cover!(value < 0, "a negative amount");
    cover!(value == 0, "zero");
}

/// A step of zero or less is refused, never guessed at.
#[kani::proof]
fn round_step_refuses_nonpositive_step() {
    let value = any_value();
    let mode = any_mode();
    for step in [0i64, -1, -7, i64::MIN] {
        assert_eq!(round_step(value, step, mode), None);
    }
    cover!(value > 0, "a positive amount with a refused step");
}
