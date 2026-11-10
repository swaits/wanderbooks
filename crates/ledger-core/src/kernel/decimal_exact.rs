//! The exactness check behind `impl Amount for Decimal` (K-U10): a sum or
//! a product `rust_decimal` returns is the exact one, or the kernel
//! refuses it.
//!
//! `rust_decimal`'s `checked_add` and `checked_mul` return `None` only
//! when the integer part overflows. When the exact result merely needs
//! more than 96 bits of mantissa, or a scale past 28, they *round* it and
//! return `Some` (found by K-U1, K-U4, K-U8: `MAX + 0.4` is `Some(MAX)`;
//! `0.01 + 792281625142643375935439503.35` is `…503.4`;
//! `1050000010.0000000001 x 9.9999999999` is `10500000099.895`). The
//! kernel's contract is "exact or `None`" (BS-ledger-1: a product the
//! arithmetic cannot hold MUST be refused), so each operation here runs
//! `rust_decimal`'s and then checks, in integers wide enough that nothing
//! can overflow, that the figure it returned equals the exact result as a
//! rational. It never builds a rounded figure of its own, and a result
//! that is exact is returned as `rust_decimal` gave it, scale and all, so
//! nothing exact changes.
//!
//! A `Decimal` is `m x 10^-s` with `|m| < 2^96` and `s <= 28`. Every
//! quantity compared below is at most `2^96 x 10^28 < 2^190`, or a
//! product `2^96 x 2^96 = 2^192`, so 256 bits hold all of them.

use rust_decimal::Decimal;

/// An unsigned 256-bit integer: `hi x 2^128 + lo`. The field order makes
/// the derived `Ord` the numeric order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Wide {
    hi: u128,
    lo: u128,
}

impl Wide {
    const ZERO: Wide = Wide { hi: 0, lo: 0 };

    fn from_u128(lo: u128) -> Wide {
        Wide { hi: 0, lo }
    }

    /// `a x b`, in full.
    fn mul_wide(a: u128, b: u128) -> Wide {
        const HALF: u128 = (1 << 64) - 1;
        let (a1, a0) = (a >> 64, a & HALF);
        let (b1, b0) = (b >> 64, b & HALF);
        let p00 = a0 * b0;
        let p01 = a0 * b1;
        let p10 = a1 * b0;
        let p11 = a1 * b1;
        // The middle column: three 64-bit terms, under 3 x 2^64.
        let mid = (p00 >> 64) + (p01 & HALF) + (p10 & HALF);
        Wide {
            lo: (mid << 64) | (p00 & HALF),
            hi: p11 + (p01 >> 64) + (p10 >> 64) + (mid >> 64),
        }
    }

    fn checked_add(self, other: Wide) -> Option<Wide> {
        let (lo, carry) = self.lo.overflowing_add(other.lo);
        let hi = self
            .hi
            .checked_add(other.hi)?
            .checked_add(u128::from(carry))?;
        Some(Wide { hi, lo })
    }

    /// `self x k`, `None` past 256 bits.
    fn checked_mul_small(self, k: u128) -> Option<Wide> {
        let low = Wide::mul_wide(self.lo, k);
        let hi = self.hi.checked_mul(k)?.checked_add(low.hi)?;
        Some(Wide { hi, lo: low.lo })
    }

    /// `(self / 10, self % 10)`.
    fn div_rem_10(self) -> (Wide, u128) {
        const HALF: u128 = (1 << 64) - 1;
        let hi = self.hi / 10;
        let carry = self.hi % 10;
        let upper = (carry << 64) | (self.lo >> 64);
        let lower = ((upper % 10) << 64) | (self.lo & HALF);
        (
            Wide {
                hi,
                lo: ((upper / 10) << 64) | (lower / 10),
            },
            lower % 10,
        )
    }

    /// The value as a `u128`, when it fits.
    fn to_u128(self) -> Option<u128> {
        (self.hi == 0).then_some(self.lo)
    }

    /// `self x 10^k`, `None` past 256 bits.
    fn checked_scale_up(self, mut k: u32) -> Option<Wide> {
        let mut value = self;
        while k > 0 {
            let step = k.min(38);
            value = value.checked_mul_small(10u128.pow(step))?;
            k -= step;
        }
        Some(value)
    }
}

/// `mantissa x 10^k`, as a 256-bit magnitude.
fn scaled(magnitude: u128, k: u32) -> Option<Wide> {
    Wide::from_u128(magnitude).checked_scale_up(k)
}

/// `a + b`: `rust_decimal`'s sum when it is the exact sum, else `None`.
pub(super) fn exact_add(a: Decimal, b: Decimal) -> Option<Decimal> {
    let sum = Decimal::checked_add(a, b)?;
    sum_is_exact(a, b, sum).then_some(sum)
}

/// `a - b`: `rust_decimal`'s difference when it is the exact difference,
/// else `None`.
pub(super) fn exact_sub(a: Decimal, b: Decimal) -> Option<Decimal> {
    let difference = Decimal::checked_sub(a, b)?;
    sum_is_exact(a, -b, difference).then_some(difference)
}

/// `a x b`: `rust_decimal`'s product when it is the exact product, else
/// `None`.
pub(super) fn exact_mul(a: Decimal, b: Decimal) -> Option<Decimal> {
    let product = Decimal::checked_mul(a, b)?;
    product_is_exact(a, b, product).then_some(product)
}

/// Is `a + b == sum` as rationals? At the common scale `s` this is
/// `ma 10^(s-sa) + mb 10^(s-sb) - ms 10^(s-ss) == 0`; the terms are moved
/// by sign so each side is a sum of magnitudes (no signed arithmetic, no
/// overflow: every term is under 2^190). Any failure to compute is "not
/// exact", the safe answer.
fn sum_is_exact(a: Decimal, b: Decimal, sum: Decimal) -> bool {
    let (ma, mb, ms) = (a.mantissa(), b.mantissa(), sum.mantissa());
    let (sa, sb, ss) = (a.scale(), b.scale(), sum.scale());
    if sa == sb && sb == ss {
        // The common case (cents plus cents): no rescaling. Mantissas are
        // under 2^96, so the i128 sum cannot overflow.
        return ma + mb == ms;
    }
    let s = sa.max(sb).max(ss);
    // The terms of a + b + (-sum), by sign.
    let mut positive = Wide::ZERO;
    let mut negative = Wide::ZERO;
    for (mantissa, scale, flip) in [(ma, sa, false), (mb, sb, false), (ms, ss, true)] {
        let Some(term) = scaled(mantissa.unsigned_abs(), s - scale) else {
            return false;
        };
        let counts_negative = (mantissa < 0) != flip;
        let side = if counts_negative {
            &mut negative
        } else {
            &mut positive
        };
        let Some(next) = side.checked_add(term) else {
            return false;
        };
        *side = next;
    }
    positive == negative
}

/// Is `a x b == product` as rationals? The exact product is
/// `ma x mb` at scale `sa + sb` (up to 2^192, at scale up to 56), compared
/// with `mp` at scale `sp` by scaling the side with fewer places up; an
/// overflow past 256 bits means that side is larger than the other, so
/// unequal.
fn product_is_exact(a: Decimal, b: Decimal, product: Decimal) -> bool {
    let (ma, mb, mp) = (a.mantissa(), b.mantissa(), product.mantissa());
    let exact = Wide::mul_wide(ma.unsigned_abs(), mb.unsigned_abs());
    if exact == Wide::ZERO {
        return mp == 0;
    }
    if mp == 0 || ((ma < 0) != (mb < 0)) != (mp < 0) {
        return false;
    }
    let exact_scale = a.scale() + b.scale();
    let held = Wide::from_u128(mp.unsigned_abs());
    if exact_scale >= product.scale() {
        held.checked_scale_up(exact_scale - product.scale()) == Some(exact)
    } else {
        exact.checked_scale_up(product.scale() - exact_scale) == Some(held)
    }
}

/// The exact product `a x b` as a `Decimal` that rounds to a multiple of
/// any step of `step_scale` places (or fewer) the way the exact product
/// would, for a product `Decimal` cannot hold: the case
/// [`exact_mul`] refused.
///
/// The product is held in 256 bits and cut down a digit at a time until it
/// fits `Decimal`, remembering whether any cut digit was not zero (a
/// sticky digit, appended as a final `1`). At least one real digit is
/// kept below the step's own places, so the first digit past the step is
/// real and every later one is only "zero or not": that decides a tie
/// (a half followed by zeros) and the directed modes exactly as the full
/// product would, so the one rounding that follows is the correctly
/// rounded one. `None` when the product is too large to hold with those
/// digits: the figure times 10^(step places + 2) must fit 96 bits, so a
/// figure past about 7.9e26 at a whole-unit step, 7.9e24 at a cent (10^9
/// times a journal line's cap). For the same reason a step of 27 or 28
/// places is always `None` here: `Decimal` ends at 28 places, so there is
/// no place below such a step for the real digit and the sticky one. That
/// refuses some figures whose rounded result `Decimal` could hold (1e-28
/// squared to a step of 1e-27 is zero); a refusal, never a guess, and no
/// currency's step is within twenty places of it (pinned in the bridge:
/// `a_step_of_27_places_refuses_a_product_that_is_not_exact`). A zero
/// product is exact and never gets here.
pub(super) fn mul_for_rounding(a: Decimal, b: Decimal, step_scale: u32) -> Option<Decimal> {
    let negative = (a.mantissa() < 0) != (b.mantissa() < 0);
    let mut digits = Wide::mul_wide(a.mantissa().unsigned_abs(), b.mantissa().unsigned_abs());
    let mut scale = a.scale() + b.scale();
    let mut sticky = false;
    loop {
        // The candidate: the digits kept, then the sticky digit if any.
        let (mantissa, candidate_scale) = if sticky {
            (
                digits
                    .checked_mul_small(10)
                    .and_then(|d| d.checked_add(Wide::from_u128(1)))
                    .and_then(Wide::to_u128),
                scale + 1,
            )
        } else {
            (digits.to_u128(), scale)
        };
        if let Some(m) = mantissa
            && candidate_scale <= Decimal::MAX_SCALE
            && let Ok(m) = i128::try_from(m)
            && let Ok(d) =
                Decimal::try_from_i128_with_scale(if negative { -m } else { m }, candidate_scale)
        {
            return Some(d);
        }
        if scale <= step_scale + 1 {
            return None;
        }
        let (rest, digit) = digits.div_rem_10();
        digits = rest;
        sticky |= digit != 0;
        scale -= 1;
    }
}

#[cfg(test)]
mod tests {
    //! The 256-bit helpers against `num_bigint`, which shares no code with
    //! them: they are private, so the bridge reaches them only through the
    //! operations; these check them directly, edges included.

    use super::*;
    use num_bigint::BigUint;
    use proptest::prelude::*;

    fn big(w: Wide) -> BigUint {
        (BigUint::from(w.hi) << 128u32) + BigUint::from(w.lo)
    }

    fn edge_u128() -> impl Strategy<Value = u128> {
        prop_oneof![
            any::<u128>(),
            Just(0u128),
            Just(1),
            Just(u128::MAX),
            Just(u128::from(u64::MAX)),
            Just(u128::from(u64::MAX) + 1),
            Just((1u128 << 96) - 1),
        ]
    }

    fn arb_wide() -> impl Strategy<Value = Wide> {
        (edge_u128(), edge_u128()).prop_map(|(hi, lo)| Wide { hi, lo })
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(5000))]

        #[test]
        fn mul_wide_is_the_full_product(a in edge_u128(), b in edge_u128()) {
            prop_assert_eq!(big(Wide::mul_wide(a, b)), BigUint::from(a) * BigUint::from(b));
        }

        #[test]
        fn add_and_mul_small_are_exact_or_refuse(w in arb_wide(), o in arb_wide(), k in edge_u128()) {
            let limit = BigUint::from(1u8) << 256u32;
            let sum = big(w) + big(o);
            prop_assert_eq!(w.checked_add(o).map(big), (sum < limit).then_some(sum));
            let product = big(w) * BigUint::from(k);
            prop_assert_eq!(w.checked_mul_small(k).map(big), (product < limit).then_some(product));
        }

        #[test]
        fn div_rem_10_divides(w in arb_wide()) {
            let (q, r) = w.div_rem_10();
            prop_assert_eq!(big(q) * 10u8 + BigUint::from(r), big(w));
            prop_assert!(r < 10);
        }

        #[test]
        fn scale_up_is_a_power_of_ten(m in edge_u128(), k in 0u32..=60) {
            let want = BigUint::from(m) * BigUint::from(10u8).pow(k);
            let limit = BigUint::from(1u8) << 256u32;
            prop_assert_eq!(scaled(m, k).map(big), (want < limit).then_some(want));
        }
    }
}
