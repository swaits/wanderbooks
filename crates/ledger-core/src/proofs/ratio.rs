//! U6 proof harnesses: percentages and ratio splits (kernel sweep).
//!
//! `kernel::percent_of` and `kernel::split_ratio` on `i64`: a percent is
//! within half a step of the exact ratio, a percent of no whole is `None`,
//! and a ratio split's two parts sum to the whole exactly.
//!
//! The divisor is fixed in every harness. CBMC's bit-blasted division by
//! a symbolic `i64` did not finish in K-U2 (`docs/proofs/k-u2.md`), and
//! both functions divide by a whole. The percent harness fixes the whole
//! at eight and takes the part as a whole multiple of it, so the model's
//! division is exact: `part / 8 = k` and the exact percent is `100 × k`.
//! The model divides before it multiplies (the order the engine's call
//! site always used), and a machine integer's division truncates, so an
//! arbitrary part would lose its fraction before the percent was formed
//! and could be a hundred points from the exact ratio; `Decimal` keeps
//! that fraction, and the bridge checks every fractional ratio there
//! against an exact reference. The step is three, so the harness reaches
//! a percent that is not already a whole multiple (the truncating model
//! cannot represent a fraction of a percent, but it can round one).
//!
//! ```text
//! GOAL: a percent is the exact ratio times 100, rounded to the step; a
//!       percent of a whole that is not positive is refused; a ratio
//!       split's parts sum to the whole exactly.
//! SOURCE: BS-statement-5 statement 2 and WB-statement-7 (the project
//!         margin: net over revenue, one place, half to even, none
//!         without revenue), BS-statement-3 statement 8 (the
//!         government's part rounded and the rest exact, the two summing
//!         to the payment), BS-partner-1 (a share of a whole), and
//!         BS-entry-3 (exact amounts; round only where a rule says to).
//! INVARIANTS:
//!   1. |r − 100 × part / whole| <= step/2 for the two half modes, where
//!      the model's division is exact (the part a whole multiple of the
//!      whole).
//!   2. A zero or negative whole is None for every part and mode; a
//!      non-positive step is None too.
//!   3. split_ratio(amount, 1, 2): the rounded part plus the exact rest
//!      is the amount, and the part is a whole multiple of the step.
//!   4. Never a panic; an unrepresentable result is None.
//! SCENARIOS:
//!   1. IF the exact percent is not a whole multiple of the step: the
//!      half modes take the nearest multiple, a tie to their own rule.
//!   2. IF the whole is zero: SHOULD be None.
//!   3. IF two parts of 1 : 2 are added: SHOULD be the amount, exactly.
//! ```
//!
//! Bounds: parts -512 to 512 by whole multiples of eight, amounts -1,000
//! to 1,000, steps 3 and 1. The laws are about the shape of the
//! arithmetic, not its magnitude; Lean proves the same laws of the exact
//! ratio at any size, and the bridge checks the `Decimal` engine against
//! an independent exact reference.

use crate::kernel::{Rounding, percent_of, split_ratio};

/// The fixed whole the percent harness divides by.
const WHOLE: i64 = 8;
/// The fixed step: three percent points, so a rounded percent moves.
const STEP: i64 = 3;
/// The percent harness's quotient bound: `part = WHOLE × k`.
const QUOTIENT: i64 = 64;

fn any_half_mode() -> Rounding {
    if kani::any() {
        Rounding::HalfAwayFromZero
    } else {
        Rounding::HalfToEven
    }
}

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

fn any_part() -> i64 {
    let part: i64 = kani::any();
    kani::assume((-QUOTIENT * WHOLE..=QUOTIENT * WHOLE).contains(&part));
    part
}

/// The percent is within half a step of the exact ratio: with the part a
/// whole multiple of the whole, `100 × part / whole = 100 × k`, and the
/// two half modes put the percent within half a step of it.
#[kani::proof]
fn percent_of_within_half_step() {
    let k: i64 = kani::any();
    kani::assume((-QUOTIENT..=QUOTIENT).contains(&k));
    let part = WHOLE * k;
    let mode = any_half_mode();
    let r = percent_of(part, WHOLE, STEP, mode).expect("in range");
    let exact = 100 * k;
    assert!(
        2 * (r - exact).abs() <= STEP,
        "within half a step of 100 x part / whole"
    );
    kani::cover!(exact % STEP != 0, "a percent that is not a whole multiple");
    kani::cover!(k < 0, "a negative part keeps its sign");
}

/// A percent of no whole is `None`: zero and negative wholes, and a step
/// that is not positive, refuse for every part and mode.
#[kani::proof]
fn percent_of_zero_whole_is_none() {
    let part = any_part();
    let mode = any_mode();
    assert!(percent_of(part, 0, STEP, mode).is_none(), "zero whole");
    assert!(
        percent_of(part, -WHOLE, STEP, mode).is_none(),
        "negative whole"
    );
    assert!(percent_of(part, WHOLE, 0, mode).is_none(), "zero step");
    kani::cover!(part != 0, "a nonzero part still has no percent of no whole");
}

/// A ratio split's parts sum to the whole exactly, and the rounded side is
/// a whole multiple of the step.
#[kani::proof]
fn split_ratio_sums() {
    let amount: i64 = kani::any();
    kani::assume((-1_000..=1_000).contains(&amount));
    let mode = any_mode();
    let (part, rest) = split_ratio(amount, 1, 2, 1, mode).expect("in range");
    assert_eq!(part + rest, amount, "the two parts sum to the whole");
    kani::cover!(
        part * 3 != amount,
        "the rounded side is not the exact share"
    );
    kani::cover!(amount < 0, "a negative amount keeps its sign");
}
