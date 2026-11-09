//! U3 proof harnesses: proportional shares and weighted splits (kernel
//! sweep).
//!
//! The divisor is fixed in every harness: CBMC's bit-blasted division by
//! a symbolic `i64` did not finish in 30 minutes in K-U2
//! (`docs/proofs/k-u2.md`), and every split here divides by a total or a
//! step. The fixed weights and totals cover three different weights
//! (2/3/5), a zero weight and a total of one; the steps used are 1 and
//! 10. The bridge carries the same laws to every `Decimal` weight and
//! step, and bookspec's Lean proves them of the exact-share model for any
//! number of parts.
//!
//! ```text
//! GOAL: the parts of a weighted split sum to the whole exactly, each
//!       part is within one step of its exact share (largest remainder),
//!       no part exceeds the whole, and a zero total or all-zero weights
//!       is the callers' answer rather than a division by zero.
//! SOURCE: BS-partner-1 (partners share by the agreement; largest
//!         remainder), BS-1099-1 (a payment's buckets), BS-statement-3
//!         rule 5 (the government's part), BS-arap-3 (a document line's
//!         exact amount), BS-ledger-1 (whole smallest units).
//! INVARIANTS:
//!   1. The parts sum to the whole, for every policy.
//!   2. Under largest remainder each part is within one unit (the step)
//!      of its exact share: `-total < part x total - whole x weight <=
//!      total` at step one. The upper bound is inclusive on `i64` (its
//!      division truncates); the bridge checks the strict `Decimal`
//!      bound.
//!   3. No part is negative or above the whole for non-negative weights
//!      that do not exceed the total.
//!   4. A zero total gives zeros; all-zero weights hand the whole out by
//!      largest remainder, and refuse once the shortfall needs as many
//!      units as there are parts.
//!   5. Overflow is `None`, never a panic.
//! SCENARIOS:
//!   1. IF 101 by 1/1/1 to the unit: SHOULD be 34/34/33 (the rounded
//!      remainders tie, so the units go to the earlier parts), never
//!      33/34/34.
//!   2. IF a quotient rounds up across the step: `share_floor_with`
//!      SHOULD still give the exact floor, by multiplication.
//!   3. IF the product `whole x weight` is past `i64`: SHOULD be None.
//! ```
//!
//! Bounds: amounts up to 1,000, weights up to 100, totals fixed (10 and
//! 0), steps 1 and 10. The laws are about the shape of the split, not the
//! magnitude; Lean proves them of the exact-share model at any size, and
//! the bridge checks the `Decimal` engine, whose ranking uses the rounded
//! quotient.

use crate::kernel::{self, Leftover, Rounding, share_floor_with, share_of, split_by_weights_into};

/// The number of parts the fixed-array harnesses explore.
const N: usize = 3;

/// The tied weights: 20% / 30% / 50%, so the remainders differ.
const WEIGHTS: [i64; N] = [2, 3, 5];
/// Their total.
const TOTAL: i64 = 10;

/// Split `whole` by the fixed weights at the fixed total, unit and mode.
fn split(whole: i64, unit: i64, policy: Leftover) -> Option<([i64; N], [i64; N])> {
    let mut parts = [0i64; N];
    let mut remainders = [0i64; N];
    split_by_weights_into(
        whole,
        &WEIGHTS,
        TOTAL,
        unit,
        |x| kernel::round_step(x, unit, Rounding::TowardZero),
        policy,
        &mut parts,
        &mut remainders,
    )?;
    Some((parts, remainders))
}

/// A whole the split harnesses explore: 0 to 1,000.
fn any_whole() -> i64 {
    let whole: i64 = kani::any();
    kani::assume((0..=1_000).contains(&whole));
    whole
}

/// The parts sum to the whole exactly, by largest remainder.
#[kani::proof]
fn split_by_weights_sums() {
    let whole = any_whole();
    let (parts, _) = split(whole, 1, Leftover::LargestRemainder).expect("in range");
    let sum = parts[0] + parts[1] + parts[2];
    assert_eq!(sum, whole);
    kani::cover!(whole % TOTAL != 0, "a shortfall handed out");
    kani::cover!(parts[2] > parts[1], "the largest weight takes more");
}

/// Each part is within one step of its share: scaled by the total,
/// `part x total - whole x weight` is above `-total` and at most `total`.
///
/// The upper bound is inclusive on the `i64` model: its division
/// truncates, so a part that divides exactly can still take a unit when
/// another part's truncation left one over (`whole = 5`, weights 2/3/5:
/// the first's 1 becomes 2, exactly one step above its truncated share).
/// The `Decimal` quotient keeps the fraction, a part with no remainder
/// never takes a unit, and the bridge checks the strict bound there
/// (`decimal_split_satisfies_the_kani_laws`); Lean's `shares_of_near`
/// proves it for the exact-share model at any size.
#[kani::proof]
fn split_by_weights_within_one_step() {
    let whole = any_whole();
    let (parts, _) = split(whole, 1, Leftover::LargestRemainder).expect("in range");
    for i in 0..N {
        let drift = parts[i] * TOTAL - whole * WEIGHTS[i];
        assert!(-TOTAL < drift && drift <= TOTAL);
    }
    kani::cover!(
        parts
            .iter()
            .enumerate()
            .any(|(i, &p)| p * TOTAL != whole * WEIGHTS[i]),
        "a part off its exact share by the rounding"
    );
    kani::cover!(
        parts
            .iter()
            .enumerate()
            .any(|(i, &p)| p * TOTAL - whole * WEIGHTS[i] == TOTAL),
        "a part exactly one step above its truncated share"
    );
}

/// No part is negative or above the whole (the weights are in [0, total]).
#[kani::proof]
fn split_by_weights_no_part_exceeds() {
    let whole = any_whole();
    let (parts, _) = split(whole, 1, Leftover::LargestRemainder).expect("in range");
    for &p in &parts {
        assert!(0 <= p && p <= whole);
    }
    kani::cover!(parts[1] > 0, "a middle part takes its share");
}

/// First absorbs: the first part is the whole less the others, and no
/// part is above the whole.
#[kani::proof]
fn split_by_weights_first_absorbs() {
    let whole = any_whole();
    let (parts, _) = split(whole, 1, Leftover::FirstAbsorbs).expect("in range");
    assert_eq!(parts[0], whole - parts[1] - parts[2]);
    assert_eq!(parts[0] + parts[1] + parts[2], whole);
    for &p in &parts {
        assert!(0 <= p && p <= whole);
    }
    kani::cover!(parts[1] != 0 && parts[2] != 0, "the others take theirs");
}

/// Last absorbs: the last part is the whole less the others, and no part
/// is above the whole.
#[kani::proof]
fn split_by_weights_last_absorbs() {
    let whole = any_whole();
    let (parts, _) = split(whole, 1, Leftover::LastAbsorbs).expect("in range");
    assert_eq!(parts[2], whole - parts[0] - parts[1]);
    assert_eq!(parts[0] + parts[1] + parts[2], whole);
    for &p in &parts {
        assert!(0 <= p && p <= whole);
    }
    kani::cover!(parts[0] != 0 && parts[1] != 0, "the others take theirs");
}

/// A zero total is zeros; all-zero weights with a positive total hand the
/// whole out by largest remainder, and refuse once the shortfall needs as
/// many units as there are parts.
#[kani::proof]
fn split_by_weights_zero_total_is_zero() {
    let whole = any_whole();
    let mut p = [0i64; N];
    let mut r = [0i64; N];
    split_by_weights_into(
        whole,
        &WEIGHTS,
        0,
        1,
        |x| kernel::round_step(x, 1, Rounding::TowardZero),
        Leftover::LargestRemainder,
        &mut p,
        &mut r,
    )
    .expect("a zero total is not a division by zero");
    assert_eq!(p, [0, 0, 0]);

    let zeros = [0i64; N];
    let mut p = [0i64; N];
    let mut r = [0i64; N];
    let got = split_by_weights_into(
        whole,
        &zeros,
        TOTAL,
        1,
        |x| kernel::round_step(x, 1, Rounding::TowardZero),
        Leftover::LargestRemainder,
        &mut p,
        &mut r,
    );
    assert_eq!(got.is_some(), whole < N as i64);
    if got.is_some() {
        assert_eq!(p[0] + p[1] + p[2], whole);
    }
    kani::cover!(
        whole < N as i64,
        "zero weights, a shortfall within the parts"
    );
}

/// `share_of` is the product and then the division: an overflowing
/// product or a zero total is `None`, never a panic.
#[kani::proof]
fn share_of_refuses_overflow() {
    let whole: i64 = kani::any();
    assert_eq!(share_of(whole, 2, 1), whole.checked_mul(2));
    assert_eq!(share_of(whole, 1, 0), None);
    let big: i64 = kani::any();
    kani::assume(big > i64::MAX / 2);
    assert_eq!(share_of(big, 2, 1), None);
    kani::cover!(big == i64::MAX, "the largest amount");
    kani::cover!(
        whole.checked_mul(2).is_some() && whole != 0,
        "a product in range"
    );
}

/// `share_floor_with` is the exact floor to the step even when the first
/// estimate rounds up across it: the part is a multiple of the step, one
/// more step would pass the share, and the remainder is exact.
#[kani::proof]
#[kani::unwind(4)]
fn share_floor_is_the_exact_floor() {
    let whole: i64 = kani::any();
    kani::assume((0..=100).contains(&whole));
    let weight: i64 = kani::any();
    kani::assume((0..=10).contains(&weight));
    // A step of ten with half-away's first estimate: `whole x weight /
    // total` near a half step (5) rounds up to ten, and the correction
    // takes it back to the exact floor (zero). Smaller bounds than the
    // split harnesses: the correction's multiplications and the
    // symbolic quotient were past an hour at 1,000 x 100 (K-U3 note).
    let step = 10;
    let (part, rem) = share_floor_with(whole, weight, TOTAL, step, |x| {
        kernel::round_step(x, step, Rounding::HalfAwayFromZero)
    })
    .expect("in range");
    let num = whole * weight;
    assert_eq!(part % step, 0);
    assert!(part * TOTAL <= num);
    assert!((part + step) * TOTAL > num);
    assert_eq!(rem, num - part * TOTAL);
    assert!(0 <= rem && rem < step * TOTAL);
    kani::cover!(rem > 0, "a remainder below the next step");
    kani::cover!(part == 0 && num >= 5 * TOTAL, "the estimate corrected down");
}
