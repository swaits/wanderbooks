//! The rounding primitive: money rounded to a whole multiple of a unit,
//! once, in a named mode. The kernel sweep's U2 (plan/kernel-sweep-audit.md
//! section 3) routes every `round_dp*`/`rescale` on money through it, so
//! the engine has one definition of "round" and no call site invents a
//! second.

use crate::Currency;
use crate::kernel::Amount;
use rust_decimal::Decimal;

/// How a value is rounded to a step. Only the modes the engine uses today
/// are here; a new mode is a new match arm in each [`Amount`]
/// instantiation, never a silent default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rounding {
    /// Nearest multiple; an exact half rounds away from zero
    /// (2.5 -> 3, -2.5 -> -3). The FX booking rule (B-3).
    HalfAwayFromZero,
    /// Nearest multiple; an exact half rounds to the even multiple
    /// (2.5 -> 2, 3.5 -> 4). Rust's `Decimal::round_dp`.
    HalfToEven,
    /// Down in magnitude, toward zero (2.9 -> 2, -2.9 -> -2).
    TowardZero,
    /// Down, toward negative infinity (2.9 -> 2, -2.9 -> -3).
    TowardNegativeInfinity,
}

/// `amount` rounded to a whole multiple of `step`, in `mode`.
///
/// ```text
/// GOAL: one rounding primitive: money is rounded to a whole multiple of a
///       unit, once, with the mode named at the call.
/// SOURCE: docs/spec/claims/ledger.md B-3 (a ledger in one money holds no
///         fractions of its smallest unit; FX books the base currency's
///         minor unit, half away from zero), claims/entry.md B-21 (exact
///         amounts in the books; round only where a rule says to), and
///         claims/ledger.md B-32; the call sites' own rules (B-13, B-14,
///         B-15, B-16, B-19, B-35, B-36, B-37); owner decision D-69 (the
///         laws live once, in bookspec's Lean).
/// INVARIANTS:
///   1. Some(r) is a whole multiple of step: r = n x step for an integer
///      n, with step > 0. None when step <= 0 or nothing can hold the
///      result.
///   2. HalfAwayFromZero and HalfToEven are nearest: |r - amount| <=
///      step/2; at exactly half, the mode's tie rule picks the multiple.
///   3. TowardZero never increases magnitude: |r| <= |amount| and
///      |r - amount| < step.
///   4. TowardNegativeInfinity is the floor: r <= amount < r + step.
///   5. Idempotent: round(round(x)) = round(x) for the same step and mode.
///   6. The symmetric modes (HalfAwayFromZero, HalfToEven, TowardZero) are
///      odd: round(-x) = -round(x). No mode ever flips the sign (a
///      non-zero amount never rounds to the other side of zero).
///   7. Never a negative zero: a result that is zero is +0. Never a panic;
///      overflow and an impossible step are None.
/// SCENARIOS:
///   1. IF 1.005 to the cent, HalfAwayFromZero: SHOULD be 1.01.
///   2. IF 1.005 to the cent, HalfToEven: SHOULD be 1.00 (100 is even).
///   3. IF -1.005 to the cent, TowardNegativeInfinity: SHOULD be -1.01.
///   4. IF 0.004 to the cent, either half mode: SHOULD be 0 (not -0).
///   5. IF the step is zero or negative: SHOULD be None, never a guess.
/// ```
pub fn round_step<T: Amount>(amount: T, step: T, mode: Rounding) -> Option<T> {
    amount.checked_round_to_step(step, mode)
}

/// `amount` rounded to `places` decimal places in `mode`, exactly as
/// `Decimal::round_dp_with_strategy` does today: an amount carrying no
/// more places than `places` is returned unchanged, and otherwise the
/// result carries exactly `places` places. This is the behaviour-preserving
/// wrapper the call sites use while U2 proves `round_step` underneath
/// them; it is not a second rounding rule.
pub fn round_places(amount: Decimal, places: u32, mode: Rounding) -> Option<Decimal> {
    if amount.is_zero() {
        // Keep round_dp's scale, but never its negative zero.
        return Decimal::try_from_i128_with_scale(0, amount.scale().min(places)).ok();
    }
    if amount.scale() <= places {
        return Some(amount);
    }
    let step = Decimal::try_from_i128_with_scale(1, places).ok()?;
    let rounded = round_step(amount, step, mode)?;
    if rounded.is_zero() {
        // Keep the places (a report prints "0.00"), never a negative zero.
        return Decimal::try_from_i128_with_scale(0, places).ok();
    }
    let mut rounded = rounded;
    if rounded.scale() != places {
        // Round_dp_with_strategy returns the target scale even when the
        // value is a whole multiple (1.200 at two places is 1.20), and
        // round_step returns an exact multiple unchanged.
        rounded.rescale(places);
    }
    Some(rounded)
}

/// `amount` rounded to whole `currency` minor units in `mode`: cents for
/// USD, whole yen for JPY, thousandths for KWD. A currency ISO 4217 gives
/// no minor unit (gold, SDRs) is returned exactly, as the FX booking rule
/// has always done (B-3).
pub fn round_minor(amount: Decimal, currency: Currency, mode: Rounding) -> Option<Decimal> {
    match currency.minor_units() {
        Some(places) => round_places(amount, places, mode),
        None => Some(amount),
    }
}

/// The `Decimal` instantiation of [`Amount::checked_round_to_step`]: exact
/// integer arithmetic on the two mantissas and scales, so no intermediate
/// `amount / step` is ever rounded by `Decimal`'s 28-digit division. The
/// quotient is rounded in integer space and multiplied by the step's
/// mantissa once; reduction by trailing zeros (as in
/// `checked_div_round_cent`) makes the result fit Decimal's 96-bit
/// mantissa when the value allows it.
pub(crate) fn round_decimal_to_step(
    amount: Decimal,
    step: Decimal,
    mode: Rounding,
) -> Option<Decimal> {
    if step <= Decimal::ZERO {
        return None;
    }
    if amount.is_zero() {
        return Some(Decimal::ZERO);
    }
    let negative = amount.is_sign_negative();
    let a = amount.mantissa().unsigned_abs();
    let b = step.mantissa().unsigned_abs();
    let exponent = i64::from(step.scale()) - i64::from(amount.scale());
    // An exact multiple is already rounded. That case also covers the one
    // the integer quotient cannot name: a step far finer than the amount
    // (1e28 as 1e-20 steps is a 48-digit count, but the value is 1e28).
    if is_exact_multiple(a, b, exponent) {
        return Some(amount);
    }

    let (mut floor, round_up) = if exponent >= 0 {
        let exp = u32::try_from(exponent).ok()?;
        let power = 10u128.checked_pow(exp)?;
        let whole = a / b;
        let mut remainder = a % b;
        let mut fraction = 0u128;
        for _ in 0..exp {
            remainder = remainder.checked_mul(10)?;
            fraction = fraction.checked_mul(10)?.checked_add(remainder / b)?;
            remainder %= b;
        }
        let floor = whole.checked_mul(power)?.checked_add(fraction)?;
        let twice = remainder.checked_mul(2)?;
        let round_up = match mode {
            Rounding::TowardZero => false,
            Rounding::TowardNegativeInfinity => negative && remainder != 0,
            Rounding::HalfAwayFromZero => twice >= b,
            Rounding::HalfToEven => twice > b || (twice == b && !floor.is_multiple_of(2)),
        };
        (floor, round_up)
    } else {
        let places = u32::try_from(-exponent).ok()?;
        let power = 10u128.checked_pow(places)?;
        let quotient = a / b;
        let remainder = a % b;
        let digit = quotient % power;
        let floor = quotient / power;
        let twice = digit.checked_mul(2)?;
        let round_up = match mode {
            Rounding::TowardZero => false,
            Rounding::TowardNegativeInfinity => negative && (digit != 0 || remainder != 0),
            Rounding::HalfAwayFromZero => twice >= power,
            Rounding::HalfToEven => {
                twice > power || (twice == power && (remainder != 0 || !floor.is_multiple_of(2)))
            }
        };
        (floor, round_up)
    };

    if round_up {
        floor = floor.checked_add(1)?;
    }
    let mut mantissa = floor.checked_mul(b)?;
    if mantissa == 0 {
        // Never a negative zero: money never prints "-0".
        return Some(Decimal::ZERO);
    }
    let max = Decimal::MAX.mantissa().unsigned_abs();
    let mut scale = step.scale();
    while mantissa > max {
        if scale == 0 || !mantissa.is_multiple_of(10) {
            return None;
        }
        mantissa /= 10;
        scale -= 1;
    }
    let signed = i128::try_from(mantissa).ok()?;
    let signed = if negative { -signed } else { signed };
    Decimal::try_from_i128_with_scale(signed, scale).ok()
}

/// True when `a x 10^exponent` (or `a / 10^-exponent`) is a whole multiple
/// of `b`, without ever materialising the product: only remainders below
/// `b` (under Decimal's 96-bit mantissa) are multiplied by ten.
fn is_exact_multiple(a: u128, b: u128, exponent: i64) -> bool {
    if exponent >= 0 {
        let mut remainder = a % b;
        for _ in 0..exponent {
            if remainder == 0 {
                return true;
            }
            remainder = (remainder * 10) % b;
        }
        remainder == 0
    } else {
        let places = u32::try_from(-exponent).unwrap_or(u32::MAX);
        let Some(power) = 10u128.checked_pow(places) else {
            return false;
        };
        if !a.is_multiple_of(power) {
            return false;
        }
        (a / power).is_multiple_of(b)
    }
}

/// The `i64` instantiation: a machine integer is already a whole smallest
/// unit, so the step is a whole number of them (a Kani model, never money
/// with a fractional unit).
pub(crate) fn round_i64_to_step(value: i64, step: i64, mode: Rounding) -> Option<i64> {
    if step <= 0 {
        return None;
    }
    let negative = value < 0;
    let magnitude = value.unsigned_abs();
    let divisor = step.unsigned_abs();
    let floor = magnitude / divisor;
    let remainder = magnitude % divisor;
    let twice = remainder.checked_mul(2)?;
    let round_up = match mode {
        Rounding::TowardZero => false,
        Rounding::TowardNegativeInfinity => negative && remainder != 0,
        Rounding::HalfAwayFromZero => twice >= divisor,
        Rounding::HalfToEven => twice > divisor || (twice == divisor && !floor.is_multiple_of(2)),
    };
    let count = if round_up {
        floor.checked_add(1)?
    } else {
        floor
    };
    let magnitude = count.checked_mul(divisor)?;
    let limit = if negative {
        1u64 << 63
    } else {
        i64::MAX as u64
    };
    if magnitude > limit {
        return None;
    }
    let signed = magnitude as i64;
    Some(if negative { -signed } else { signed })
}

/// The `i32` instantiation, the proof model's smaller type: the same rule
/// in 32 bits.
pub(crate) fn round_i32_to_step(value: i32, step: i32, mode: Rounding) -> Option<i32> {
    if step <= 0 {
        return None;
    }
    let negative = value < 0;
    let magnitude = value.unsigned_abs();
    let divisor = step.unsigned_abs();
    let floor = magnitude / divisor;
    let remainder = magnitude % divisor;
    let twice = remainder.checked_mul(2)?;
    let round_up = match mode {
        Rounding::TowardZero => false,
        Rounding::TowardNegativeInfinity => negative && remainder != 0,
        Rounding::HalfAwayFromZero => twice >= divisor,
        Rounding::HalfToEven => twice > divisor || (twice == divisor && !floor.is_multiple_of(2)),
    };
    let count = if round_up {
        floor.checked_add(1)?
    } else {
        floor
    };
    let magnitude = count.checked_mul(divisor)?;
    let limit = if negative {
        1u32 << 31
    } else {
        i32::MAX as u32
    };
    if magnitude > limit {
        return None;
    }
    let signed = magnitude as i32;
    Some(if negative { -signed } else { signed })
}
