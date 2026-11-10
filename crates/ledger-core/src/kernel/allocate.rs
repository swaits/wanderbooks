//! Proportional shares and weighted splits: `whole × weight ÷ total`,
//! rounded to a step, the leftover handed out by a named policy. The
//! kernel sweep's U3 (`plan/kernel-sweep-audit.md` section 3): the six
//! local copies of this arithmetic (the K-1/1065 worksheet, the 1099
//! worksheet, the parties' shares, the conventions' template split and the
//! statement of cash flows' government split) call one implementation.
//!
//! Division is not exact: [`share_of`] is the amount type's checked
//! division, so a `Decimal` quotient is rounded at 28 significant digits
//! (and a machine integer's is truncated), never an exact rational.
//! Multiplication is not exact either at `Decimal`'s 96-bit mantissa:
//! `checked_mul_rounded` rounds the product there, and this module uses
//! it by name wherever a rounded product is the figure meant (the
//! kernel's `checked_mul` is exact or refused, K-U10, and would refuse a
//! share by a percent of 27 places). Every invariant below is
//! stated of that computation, and of the parts after the named leftover
//! policy, which do sum to the whole exactly. See `docs/proofs/k-u3.md`
//! for the edges where a product's or a quotient's rounding can change a
//! rounded part (Decimal's edge, K-U10).

use super::{Amount, apportion, plug_last};

/// What a split does with the units its rounded parts leave over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Leftover {
    /// Each part is rounded down; the units left over go one each to the
    /// largest remainders. The remainders are exact: `whole x weight -
    /// part x total`, an integer comparison, never the rounded quotient's
    /// difference, so an exact tie goes to the earlier part (the Lean
    /// model's rank, bookspec's `Apportion`). [`apportion`].
    LargestRemainder,
    /// The last part becomes the whole less the others. [`plug_last`].
    LastAbsorbs,
    /// The first part becomes its rounded share plus the shortfall (the
    /// whole less every rounded part), so its scale is the shortfall
    /// addition's; the value is the whole less the others.
    FirstAbsorbs,
}

/// One member's share of `whole`: `whole × weight ÷ total`.
///
/// ```text
/// GOAL: one definition of "share = whole x weight / total", the arithmetic
///       every proportional split in the engine rests on.
/// SOURCE: BS-partner-1 (partners share by the agreement; capital accounts
///         carry the allocations), BS-partner-3 (a party's part of an
///         entry), BS-partner-4 (the K-1 worksheet), BS-1099-1 (a payment
///         split by the parties' shares), BS-statement-3 statement 8 (the
///         government's part of a payment), BS-arap-3 (a document line's
///         exact amount).
/// INVARIANTS:
///   1. Some(s) is the amount type's `checked_mul_rounded` of whole and
///      weight, then its checked division by total. For Decimal the
///      product and quotient are exact while they fit 96 bits and scale
///      28, and are rounded at that edge (rust_decimal's checked
///      operations: a share by a percent of 27 places is a rounded figure
///      by construction, and is computed so, to the digit; K-U10 left
///      this one product rounding, because the quotient rounds anyway);
///      for a machine integer the division truncates toward zero. This is
///      the computation the six call sites made before U3, to the digit.
///   2. None when total is zero, or when the product or quotient is not
///      representable. Never a panic, never a wrapped amount.
///   3. When total is not zero and whole x weight is representable, the
///      product and the division are the two steps that can round: either
///      `Decimal` operation rounds at its own edge. A `Some` is always the
///      amount type's quotient of its product, never a guessed one.
/// SCENARIOS:
///   1. IF 100 by 40 of 100: SHOULD be 40.
///   2. IF 1 by 1 of 3: SHOULD be 0.333... (28 significant digits), not an
///      exact rational; the split's rounding makes the part exact to its
///      step.
///   3. IF the product or the quotient is past the amount's range: SHOULD
///      be None.
/// ```
pub fn share_of<T: Amount>(whole: T, weight: T, total: T) -> Option<T> {
    whole.checked_mul_rounded(weight)?.checked_div(total)
}

/// The largest whole multiple of `step` not above `whole × weight ÷ total`,
/// with the exact remainder scaled by `total`.
///
/// ```text
/// GOAL: the exact floor of a proportional share to a step, without
///       trusting a rounded quotient, and the exact remainder that ranks
///       the parts of a largest-remainder split.
/// SOURCE: BS-partner-1, BS-1099-1 (a payment split into its buckets to
///         the cent; partners share by the agreement),
///         BS-entry-3 (amounts are exact; round only where a rule says
///         to).
/// INVARIANTS:
///   1. whole and weight are non-negative (the callers pass magnitudes).
///      Provided `part x total` and `(part + step) x total` are exact
///      (inside Decimal's 96-bit mantissa; true for every posted line),
///      Some((part, remainder)): part is a whole multiple of step,
///      part x total <= whole x weight, and (part + step) x total >
///      whole x weight. So part is the exact floor of the share to step,
///      whatever the quotient's rounding did. At Decimal's mantissa edge
///      the comparison product itself rounds, and the correction can keep
///      a step the exact rational would not (K-U10; pinned in the bridge).
///   2. remainder = whole x weight - part x total, in [0, step x total)
///      when total and step are positive: the exact remainder below the
///      next step, scaled by total, never the rounded quotient's
///      difference (exact when the products are; the edge above inherits
///      the product's rounding).
///   3. None when step or total is not positive, a product is not
///      representable, or the correction leaves the range; never a panic.
///      The correction takes at most [`MAX_CORRECTION_STEPS`] steps in
///      each direction and returns None past the cap: a malformed input
///      (one whose product's rounding put the estimate many steps away)
///      refuses rather than runs without end.
/// SCENARIOS:
///   1. IF 60 x 33.33 / 100 to the cent: SHOULD be (19.99, 80), since
///      60 x 33.33 = 1999.80 and 19.99 x 100 = 1999.
///   2. IF the quotient rounds up across a step (Decimal's 28 digits at
///      the edge), while the comparison products are exact: SHOULD still
///      be the exact floor, the correction taking the step back.
/// ```
pub fn share_floor_with<T: Amount>(
    whole: T,
    weight: T,
    total: T,
    step: T,
    round: impl Fn(T) -> Option<T>,
) -> Option<(T, T)> {
    if total <= T::ZERO || step <= T::ZERO {
        return None;
    }
    let num = whole.checked_mul_rounded(weight)?;
    let quotient = num.checked_div(total)?;
    let mut part = round(quotient)?;
    // The quotient may have rounded across the exact floor: take whole
    // steps back until part x total <= num, then forward while one more
    // step still fits. The comparisons multiply, never divide; they are
    // exact while the product fits Decimal's mantissa (the edge is
    // K-U10, pinned in the bridge). Both directions are bounded so a
    // quotient rounded far across the floor refuses instead of hanging.
    let mut steps = 0;
    while part.checked_mul_rounded(total)? > num {
        if steps == MAX_CORRECTION_STEPS {
            return None;
        }
        steps += 1;
        part = part.checked_sub(step)?;
    }
    steps = 0;
    loop {
        if steps == MAX_CORRECTION_STEPS {
            return None;
        }
        let next = part.checked_add(step)?;
        if next.checked_mul_rounded(total)? > num {
            break;
        }
        part = next;
        steps += 1;
    }
    let remainder = num.checked_sub(part.checked_mul_rounded(total)?)?;
    Some((part, remainder))
}

/// The most correction steps [`share_floor_with`] takes in each direction.
/// The first estimate is at most one step off in practice (half-away's
/// rounding of a quotient whose product is exact); a few thousand leaves
/// room for any quotient rounding that still describes the same figure.
/// A share further away is a malformed input, and the function refuses it
/// rather than loop without end.
const MAX_CORRECTION_STEPS: usize = 4096;

/// Split `whole` by `weights` over `total`, each part the exact share
/// rounded by `round`, the leftover by `policy`, written into `parts` (and
/// `remainders`, a scratch buffer for the largest-remainder policy).
///
/// ```text
/// GOAL: split a whole by weights without drift: the parts sum to the
///       whole exactly, each is within one step of the share the kernel
///       computes (the largest-remainder policy), and no part exceeds the
///       whole.
/// SOURCE: BS-partner-1 (partners share by the agreement; the largest
///         remainder is the owner's choice, 2026-10-13), BS-partner-3 (a
///         party's part of an entry), BS-partner-4 (the K-1 worksheet's
///         shares), BS-1099-1 (a payment's buckets), BS-statement-3 statement 8
///         (the government's part), BS-arap-3 (a document line's exact
///         amount), BS-ledger-1 (a whole number of smallest units).
/// INVARIANTS:
///   1. With total not zero and at least one weight, the parts sum to
///      whole exactly, whatever the policy: the largest-remainder policy
///      hands out the shortfall by [`apportion`], the first/last-absorbs
///      policies give the first or last part the whole less the others
///      ([`plug_last`] for the last). An empty weight list is an empty
///      split (the callers' existing answer).
///   2. With total not zero, every part is the share `share_of` computes,
///      rounded by `round`; under the largest-remainder policy each part
///      is within one step of that share (`part x total - whole x weight`
///      is above `-total` and at most `total` when the step is one unit;
///      the upper bound is strict when the division keeps the fraction, as
///      `Decimal` does, and a machine integer's truncating division can
///      put a part exactly one step above, never more). That share is the
///      exact rational while the division and the comparison products are
///      exact; at Decimal's mantissa edge the quotient's rounding can put
///      a part more than one step from the exact rational (K-U10, pinned
///      in the bridge). Under the first/last-absorbs policies only the
///      absorbing part may be further, by the rounding of all the others.
///      The largest-remainder ranking compares the exact remainders
///      `whole x weight - part x total` (the quantity `share_floor_with`
///      returns, scaled by total; the sign flipped for a negative total),
///      never the rounded quotients' differences. An exact tie goes to the
///      earlier part, whatever `Decimal`'s 28th digit would have said
///      (K-U10, the owner's decision; exact while the products are, as
///      above).
///   3. With total not zero, no part exceeds the whole when every weight
///      is in [0, total] and the whole is not negative; no part is
///      negative when the weights and the whole are not.
///   4. total of zero is the callers' "nothing to split by": every part is
///      zero (k1's and shares::split_first's existing answer), never a
///      division by zero.
///   5. None when the slices' lengths differ, a share or a part is not
///      representable, the unit is not positive for the largest-remainder
///      policy, or apportion refuses (the shortfall is not rounding).
///      Never a panic. On None the contents of the buffers are not
///      promised.
/// SCENARIOS:
///   1. IF 100 by 1/1/1 to the cent by largest remainder: SHOULD be
///      33.34, 33.33, 33.33 (the exact remainders tie, so the cent goes
///      to the earlier part).
///   2. IF 0.01 by 1/1 to the cent by largest remainder: SHOULD be
///      0.01, 0.00.
///   3. IF 10 by weights 2/3/5 by first-absorbs: SHOULD be the rounded
///      shares with the first taking the whole less the others.
///   4. IF 20.00 by 4/1/1 over 6 to the cent by largest remainder: SHOULD
///      be 13.34, 3.33, 3.33. The exact remainders (0.02, 0.02, 0.02 over
///      6, as 80 - 79.98, 20 - 19.98) tie, so the cent goes to the first.
///      The rounded quotients (13.33...3 keeps one fewer fractional digit
///      than 3.33...3 at 28 significant digits) would have put the
///      second's remainder above the first's.
///   5. IF the total is zero, or every weight is zero: SHOULD be zeros (a
///      zero total) or the largest-remainder handout of the whole (a
///      positive total with zero weights, as the conventions' template
///      split has always done).
/// ```
#[allow(clippy::too_many_arguments)] // one argument per split input and buffer
pub fn split_by_weights_into<T: Amount>(
    whole: T,
    weights: &[T],
    total: T,
    unit: T,
    round: impl Fn(T) -> Option<T>,
    policy: Leftover,
    parts: &mut [T],
    remainders: &mut [T],
) -> Option<()> {
    if parts.len() != weights.len() || remainders.len() != weights.len() {
        return None;
    }
    if total == T::ZERO {
        // Nothing to split by: the call sites' existing answer.
        parts.fill(T::ZERO);
        return Some(());
    }
    // The exact shares first, so a failure leaves parts untouched until
    // every share is known; the buffer doubles as the largest-remainder
    // scratch below.
    for i in 0..weights.len() {
        remainders[i] = share_of(whole, weights[i], total)?;
    }
    for i in 0..weights.len() {
        parts[i] = round(remainders[i])?;
    }
    match policy {
        Leftover::LargestRemainder => {
            if unit <= T::ZERO {
                return None;
            }
            // Rank by the exact remainder, never the rounded quotient's
            // difference: `whole x weight - part x total`, the exact
            // rational's fraction scaled by `total`, the same quantity
            // `share_floor_with` returns. A negative total flips the
            // scale's sign, so the order is the share's own. The two
            // products are the rounding ones: each is exact while it fits
            // Decimal's mantissa (every split of posted amounts by whole
            // weights or basis points), and where a weight of 27 places
            // makes a product round, the split is still made, ranked by
            // the nearest figures Decimal holds, not refused. Their
            // difference is exact: the subtraction refuses otherwise.
            for i in 0..weights.len() {
                let exact = whole
                    .checked_mul_rounded(weights[i])?
                    .checked_sub(parts[i].checked_mul_rounded(total)?)?;
                remainders[i] = if total < T::ZERO {
                    T::ZERO.checked_sub(exact)?
                } else {
                    exact
                };
            }
            apportion(whole, parts, remainders, unit)
        }
        Leftover::LastAbsorbs => plug_last(whole, parts),
        Leftover::FirstAbsorbs => {
            // The old `split_first`: the first part takes its rounded share
            // plus the shortfall (the whole less every rounded part). The
            // value is the whole less the others, but its scale is the sum
            // and the addition's, as the call site has always produced.
            let mut sum = T::ZERO;
            for &p in parts.iter() {
                sum = sum.checked_add(p)?;
            }
            let Some(first) = parts.first_mut() else {
                return Some(());
            };
            let left = whole.checked_sub(sum)?;
            *first = first.checked_add(left)?;
            Some(())
        }
    }
}

/// [`split_by_weights_into`] allocating its two buffers: the shape the
/// call sites use.
pub fn split_by_weights_with<T: Amount>(
    whole: T,
    weights: &[T],
    total: T,
    unit: T,
    round: impl Fn(T) -> Option<T>,
    policy: Leftover,
) -> Option<Vec<T>> {
    let mut parts = vec![T::ZERO; weights.len()];
    let mut remainders = vec![T::ZERO; weights.len()];
    split_by_weights_into(
        whole,
        weights,
        total,
        unit,
        round,
        policy,
        &mut parts,
        &mut remainders,
    )?;
    Some(parts)
}
