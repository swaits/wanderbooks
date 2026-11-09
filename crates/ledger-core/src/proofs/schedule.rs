//! U5 proof harnesses: equal monthly parts (kernel sweep).
//!
//! `kernel::even_parts` on `i64`: the total a symbolic whole number of the
//! smallest unit, the months and the step fixed constants. A symbolic
//! month count or step was not used: `even_parts` divides by the months
//! and `round_step` by the step, and CBMC's bit-blasted division by a
//! symbolic `i64` did not finish in K-U2 (`docs/proofs/k-u2.md`). The
//! month counts below are one (nothing but the plug), two (the smallest
//! split, and an exact tie at an even total), three (the schedule the
//! tests are written on, a quotient that never ends) and seven (a count
//! that divides few totals); the steps are one, an even step (a tie
//! exists), an odd one, and two decimal steps. The bridge carries the
//! same laws to every `Decimal` total, month count and step, and
//! bookspec's Lean (`Bookspec.Schedule`, law 43) proves them of the
//! exact-rational model for any number of months.
//!
//! ```text
//! GOAL: an equal-months schedule books exactly its total across its
//!       months, each month but the last a whole multiple of the unit and
//!       within half a unit of the quotient, the last taking the rest.
//! SOURCE: BS-prepaid-1 (a schedule recognizes exactly its total over its
//!         months), BS-prepaid-2 (equal months), BS-prepaid-4 (the
//!         currency's minor unit is the step), BS-ledger-1 (whole smallest
//!         units).
//! INVARIANTS:
//!   1. The parts sum to the total exactly, whatever the rounding.
//!   2. Every part but the last equals `round_half_even(total / months)`
//!      to the step, so each is a whole multiple of it.
//!   3. The last part is the total less the others, which is the first
//!      part plus what the others left (`total - months x each`).
//!   4. The first part is within half a step of the quotient the integer
//!      division gives, and `months x each` within half a step per month
//!      (plus the truncation, under one unit a month) of the total. The
//!      `Decimal` quotient keeps its fraction, so the bridge checks the
//!      strict half-step bound against the exact total / months.
//!   5. No months, or a step that is not positive, is None.
//! SCENARIOS:
//!   1. IF 1,000 over 3 months at one: SHOULD be 333, 333, 334.
//!   2. IF 5 over 2 months at one: SHOULD be 2, 3, the half to the even
//!      multiple.
//!   3. IF no months: SHOULD be None.
//! ```
//!
//! Bounds: totals -1,000 to 1,000, months {1, 2, 3, 7}, steps {1, 2, 5,
//! 10, 100}. The laws are about the shape of the split, not the
//! magnitude.

use crate::kernel::{self, Rounding};
use kani::cover;

/// The fixed steps: one, an even step (a tie exists), an odd step, a
/// decimal step, and a step well above the totals' quotients.
const STEPS: [i64; 5] = [1, 2, 5, 10, 100];

/// A total the harnesses explore: -1,000 to 1,000 units.
fn any_total() -> i64 {
    let total: i64 = kani::any();
    kani::assume((-1_000..=1_000).contains(&total));
    total
}

/// The split of `total` into `M` months at `step`.
fn split<const M: usize>(total: i64, step: i64) -> [i64; M] {
    let mut parts = [0i64; M];
    kernel::even_parts(total, step, &mut parts).expect("no overflow under these bounds");
    parts
}

/// The parts sum to the total, whatever the rounding left.
#[kani::proof]
#[kani::unwind(10)]
fn even_parts_sum() {
    let total = any_total();
    for step in STEPS {
        let a = split::<1>(total, step);
        let b = split::<2>(total, step);
        let c = split::<3>(total, step);
        let d = split::<7>(total, step);
        assert_eq!(a.iter().sum::<i64>(), total);
        assert_eq!(b.iter().sum::<i64>(), total);
        assert_eq!(c.iter().sum::<i64>(), total);
        assert_eq!(d.iter().sum::<i64>(), total);
    }
    cover!(
        split::<3>(total, 1)[2] != split::<3>(total, 1)[0],
        "the last part takes a unit the others left"
    );
    cover!(
        split::<3>(total, 10)[2] != split::<3>(total, 10)[0],
        "the last part takes what a coarse step left"
    );
    cover!(total < 0, "a negative total");
}

/// Every part but the last is the same whole multiple of the step: the
/// quotient rounded half to even.
#[kani::proof]
#[kani::unwind(10)]
fn even_parts_all_but_last_equal() {
    let total = any_total();
    for step in STEPS {
        let each = kernel::round_step(total / 3, step, Rounding::HalfToEven)
            .expect("no overflow under these bounds");
        let c = split::<3>(total, step);
        assert_eq!(c[0], each, "each is the rounded quotient");
        assert_eq!(c[1], c[0], "all but the last are equal");
        assert_eq!(c[0] % step, 0, "a whole multiple of the step");
        let d = split::<7>(total, step);
        for i in 1..6 {
            assert_eq!(d[i], d[0]);
        }
        assert_eq!(d[0] % step, 0);
    }
    cover!(
        total / 3 % 10 == 5 && total > 0,
        "a quotient exactly on a tie at step 10 (rounds to the even multiple)"
    );
    cover!(total % 3 != 0, "a quotient that does not terminate");
}

/// The last part is the total less the others: the first part plus the
/// rounding the others left.
#[kani::proof]
#[kani::unwind(10)]
fn even_parts_last_absorbs() {
    let total = any_total();
    for step in STEPS {
        let c = split::<3>(total, step);
        assert_eq!(
            c[2],
            total - 2 * c[0],
            "the last is the total less the others"
        );
        assert_eq!(
            c[2] - c[0],
            total - 3 * c[0],
            "it is each plus what was left"
        );
        let d = split::<7>(total, step);
        assert_eq!(d[6], total - 6 * d[0]);
        let one = split::<1>(total, step);
        assert_eq!(one[0], total, "a single month is the whole total");
    }
    cover!(
        split::<3>(total, 100)[2] - split::<3>(total, 100)[0] > 50,
        "a last part far from the others under a coarse step"
    );
}

/// The first part is within half a step of the quotient, and the parts
/// are within half a step a month of the total (plus truncation).
#[kani::proof]
#[kani::unwind(10)]
fn even_parts_within_half_step() {
    let total = any_total();
    for step in STEPS {
        let c = split::<3>(total, step);
        let quotient = total / 3;
        assert!(
            2 * (c[0] - quotient).abs() <= step,
            "within half a step of the quotient"
        );
        // Against the exact total / 3: the quotient truncated, under one
        // unit a month.
        assert!(
            2 * (3 * c[0] - total).abs() <= 3 * step + 2 * 2,
            "months x each within half a step a month, plus truncation"
        );
        let d = split::<7>(total, step);
        assert!(2 * (d[0] - total / 7).abs() <= step);
        assert!(2 * (7 * d[0] - total).abs() <= 7 * step + 2 * 6);
    }
    cover!(
        2 * (split::<3>(total, 10)[0] - total / 3).abs() == 10,
        "an exact tie at step 10, taken at exactly half a step"
    );
    cover!(
        split::<3>(total, 2)[0] != total / 3,
        "a quotient moved to the step"
    );
}

/// No months and a step that is not positive are None, never a guess.
#[kani::proof]
#[kani::unwind(4)]
fn even_parts_refuses_no_months_and_a_bad_step() {
    let total = any_total();
    let mut none: [i64; 0] = [];
    assert_eq!(kernel::even_parts(total, 1, &mut none), None);
    let mut parts = [0i64; 3];
    assert_eq!(kernel::even_parts(total, 0, &mut parts), None);
    assert_eq!(kernel::even_parts(total, -5, &mut parts), None);
    cover!(total != 0, "a total no step can place");
}
