//! Percentages and ratio splits: `part ÷ whole × 100` rounded to a step,
//! and an amount split `a : b` with one side rounded and the other exact.
//! The kernel sweep's U6 (`plan/kernel-sweep-audit.md` section 3): the
//! contribution margin the projects page and the tools print, and the
//! government's part of an operating payment (BS-statement-3 statement 8).
//!
//! U3 already took the cash-flow government split's share through
//! [`super::share_of`]; [`split_ratio`] names the whole shape it has --
//! the rounded part and the exact rest -- so the sum is proved, not
//! assumed. Division and multiplication are not exact at `Decimal`'s
//! edges (the 28-digit quotient and the 96-bit product): see
//! [`percent_of`]'s spec and `docs/proofs/k-u6.md` for what rounds
//! (K-U10).

use super::{Amount, Rounding, round_step, share_of};

/// `part / whole × 100`, rounded to a whole multiple of `step` in `mode`;
/// `None` when `whole` is zero or negative (a percent of nothing, or of a
/// negative base, means nothing), when `step` is not positive, or when a
/// quotient or the product is not representable.
///
/// ```text
/// GOAL: one definition of a percentage: the exact ratio times 100,
///       rounded once, at the step and mode the caller names.
/// SOURCE: docs/spec/claims/statement.md BS-statement-5 statement 2 (a
///         project's net is its revenue less its expenses) and
///         WB-statement-7 (that net as a percent of revenue: one place,
///         half to even, none without revenue), with BS-entry-3 (amounts
///         are exact; round only where a rule says to).
/// INVARIANTS:
///   1. Some(r) is round_step(part / whole × 100, step, mode), computed
///      division first and then the multiplication, the order every call
///      site used before U6. For Decimal the quotient is exact while it
///      fits 28 significant digits and rounds past them; the product by
///      100 is exact or refused, never rounded (K-U10).
///      For a machine integer the division truncates toward zero.
///   2. None when whole <= 0, step <= 0, or a quotient, product or
///      rounded result is not representable. Never a panic.
///   3. When the division and the product are exact (every posted
///      amount: the line cap is 10^15), r is the exact ratio times 100
///      rounded once, so |r − exact| <= step/2 for the two half modes.
///      At Decimal's mantissa edge the quotient's own rounding can move
///      r by one more step (K-U10, pinned in the bridge).
///   4. The percent carries the part's sign: a zero part is zero, a
///      positive part with a positive whole is not negative, a negative
///      part not positive.
/// SCENARIOS:
///   1. IF 32,000 of 50,000: SHOULD be 64 at one place (the ratio 64%).
///   2. IF 125 of 10,000 to one place, half to even: SHOULD be 1.2, not
///      1.3.
///   3. IF the whole is zero or negative: SHOULD be None, not a division
///      by zero and not a percent of a negative base.
///   4. IF the part is past the amount's range: SHOULD be None, never a
///      panic.
/// ```
pub fn percent_of<T: Amount>(part: T, whole: T, step: T, mode: Rounding) -> Option<T> {
    if whole <= T::ZERO {
        return None;
    }
    let hundred = T::from_u32(100)?;
    let ratio = part.checked_div(whole)?.checked_mul(hundred)?;
    round_step(ratio, step, mode)
}

/// `amount` split as `a` to `b`: the `a` side is `amount × a ÷ (a + b)`
/// rounded to a whole multiple of `step` in `mode`, the `b` side the
/// exact rest, so the two always sum to `amount`; `None` for a negative
/// share, a non-positive `a + b`, a non-positive step, or an amount the
/// type cannot hold.
///
/// ```text
/// GOAL: one definition of a ratio split: the named side rounded once,
///       the other side exact, and the two summing to the whole.
/// SOURCE: docs/spec/claims/statement.md BS-statement-3 statement 8 (of a
///         payment to a government, the government's part is rounded
///         half to even to the base currency's places and the rest is
///         exact, so the two always sum to the payment), BS-partner-1
///         (a share of a whole by the parties' ratio), and BS-entry-3
///         (round only where a rule says to).
/// INVARIANTS:
///   1. Some((part, rest)): part = round_step(amount × a ÷ (a + b), step,
///      mode), the same share U3's `share_of` computes, rounded once;
///      rest = amount − part exactly, so part + rest = amount to the
///      last digit and no unit is lost or invented.
///   2. part is a whole multiple of step. With 0 <= a <= a + b and
///      amount >= 0, 0 <= part <= amount; with a = 0 the part is zero
///      and the rest is the whole.
///   3. None when a or b is negative, a + b <= 0, step <= 0, or a share,
///      part or rest is not representable. Never a panic: a zero ratio is
///      a refusal, not a division by zero.
///   4. The split is odd in the amount for the symmetric modes: the parts
///      of −amount are the parts of amount negated. The rest keeps the
///      amount's sign whenever the part does.
///   5. What rounds is the share's own arithmetic: for Decimal the
///      product and quotient round at 96 bits and 28 digits (K-U10), and
///      for a machine integer the division truncates; the rest is an
///      exact checked subtraction either way.
///   6. The part's scale is the share's when it is already a whole
///      multiple of the step, and the step's otherwise; a caller that
///      needs a fixed scale rescales, as `contribution_margin_pct` does
///      for its one-place display.
/// SCENARIOS:
///   1. IF −10.00 split 1 : 2 to the cent, half to even: SHOULD be
///      (−3.33, −6.67), and the two sum to −10.00.
///   2. IF 0.05 split 1 : 1 to the cent, half to even: SHOULD be (0.02,
///      0.03), never 0.03 and 0.02.
///   3. IF 0.07 split 1 : 1 to the cent, half to even: SHOULD be (0.04,
///      0.03).
///   4. IF a and b are both zero: SHOULD be None, never a division by
///      zero.
/// ```
pub fn split_ratio<T: Amount>(amount: T, a: T, b: T, step: T, mode: Rounding) -> Option<(T, T)> {
    if a < T::ZERO || b < T::ZERO {
        return None;
    }
    let whole = a.checked_add(b)?;
    if whole <= T::ZERO {
        return None;
    }
    let part = round_step(share_of(amount, a, whole)?, step, mode)?;
    let rest = amount.checked_sub(part)?;
    // A checked subtraction can hand back a negative zero (the negated
    // subtrahend keeps the sign bit); money's zero is always +0.
    let rest = if rest == T::ZERO { T::ZERO } else { rest };
    Some((part, rest))
}
