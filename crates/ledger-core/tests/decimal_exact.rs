//! `rust_decimal` checked against exact rational arithmetic.
//!
//! The engine's money paths trust `Decimal`: 96-bit integers scaled by a
//! power of ten. `docs/verification.md` ("What is and isn't proved") says
//! so, and Kani cannot explore `Decimal` arithmetic directly (one symbolic
//! add runs for more than ten minutes). This file checks the claim on the
//! operations the books rely on, against an independent exact reference:
//! [`num_rational::BigRational`], where every result is exact by
//! construction.
//!
//! The ranges are the books' own (`kernel::MAX_LINE_AMOUNT` and
//! `kernel::MAX_LINE_SCALE`): a line holds at most one quadrillion units
//! (1e15) with at most ten decimal places, a sum at most 1,000 such lines,
//! and an FX rate at most ten decimal places times such an amount. Every
//! property compares the `Decimal` answer to the rational one *exactly*;
//! the rounding operations compare to the correctly rounded rational.
//!
//! Exact only up to the sums the books reach. Near `Decimal::MAX` the add
//! rounds dust away (`MAX + 0.4` gives `Some(MAX)`, found by unit K-U4 and
//! pinned below), and a product whose scale can't fit 28 places is rounded
//! (the reason for decision D-75's ten-place bound on foreign amounts and
//! rates). These are `rust_decimal`'s own behaviours, pinned here as
//! such; the kernel's `Amount for Decimal` refuses both (K-U10, checked
//! in `tests/kernel_bridge/decimal_exact.rs`).
//!
//! This is a test of the dependency, not of the engine. When a property
//! fails, the finding is `rust_decimal`'s, and the failing case stays here
//! as a regression named for the defect.

use num_bigint::BigInt;
use num_rational::BigRational;
use proptest::prelude::*;
use rust_decimal::{Decimal, RoundingStrategy};
use std::str::FromStr;

// ------------------------------------------------------------- helpers

/// The repo's usual case count: `PROPTEST_CASES`, default 256.
fn cases() -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|n| n.parse().ok())
        .unwrap_or(256)
}

/// 10^n as a `BigInt`.
fn pow10(n: u32) -> BigInt {
    BigInt::from(10u8).pow(n)
}

/// `d` as an exact rational: mantissa / 10^scale.
fn rat(d: Decimal) -> BigRational {
    BigRational::new(BigInt::from(d.mantissa()), pow10(d.scale()))
}

/// The integer nearest `q` under `strategy`, where `q` may be fractional.
/// `BigInt` division truncates toward zero, so `q` is the truncation and
/// `rem` carries the sign of the dividend.
fn round_int(q: &BigRational, strategy: RoundingStrategy) -> BigInt {
    let n = q.numer();
    let d = q.denom();
    let truncated = n / d;
    let rem = n - &truncated * d;
    let zero = BigInt::from(0);
    let negative = rem < zero;
    let magnitude = if negative { -&rem } else { rem.clone() };
    let twice = magnitude * 2;
    let away = if negative { -1 } else { 1 };
    match strategy {
        RoundingStrategy::ToZero => truncated,
        RoundingStrategy::ToNegativeInfinity => {
            if negative {
                truncated - 1
            } else {
                truncated
            }
        }
        RoundingStrategy::ToPositiveInfinity => {
            if negative || rem == zero {
                truncated
            } else {
                truncated + 1
            }
        }
        RoundingStrategy::MidpointAwayFromZero => {
            if twice >= *d {
                truncated + away
            } else {
                truncated
            }
        }
        RoundingStrategy::MidpointTowardZero => {
            if twice > *d {
                truncated + away
            } else {
                truncated
            }
        }
        RoundingStrategy::MidpointNearestEven => {
            if twice > *d || (twice == *d && truncated.bit(0)) {
                truncated + away
            } else {
                truncated
            }
        }
        // `Decimal` has no such strategy today; the match stays exhaustive.
        _ => unreachable!("unknown round strategy"),
    }
}

/// `r` rounded to `scale` decimal places under `strategy`, exactly.
fn round_rat(r: &BigRational, scale: u32, strategy: RoundingStrategy) -> BigRational {
    let factor = pow10(scale);
    let q = r * BigRational::from_integer(factor.clone());
    BigRational::new(round_int(&q, strategy), factor)
}

/// True when `r` can be written as `m / 10^s` with a 96-bit `m` and
/// `s <= 28`: the set of values `Decimal` represents exactly.
fn representable(r: &BigRational) -> bool {
    let max = BigInt::from(Decimal::MAX.mantissa());
    (0..=Decimal::MAX_SCALE).any(|s| {
        let scaled = r * BigRational::from_integer(pow10(s));
        let numer = scaled.numer();
        let magnitude = if *numer < BigInt::from(0) {
            -numer.clone()
        } else {
            numer.clone()
        };
        scaled.is_integer() && magnitude <= max
    })
}

/// A non-negative amount up to one quadrillion units with up to ten
/// decimal places (the worst case: mantissa to 10^25).
fn line_amount() -> impl Strategy<Value = Decimal> {
    (0u32..=10).prop_flat_map(|scale| {
        let cap = 10i128.pow(15 + scale);
        (0i128..=cap, Just(scale))
            .prop_map(move |(mantissa, _)| Decimal::from_i128_with_scale(mantissa, scale))
    })
}

/// A signed line amount (the books use negative amounts for reversals and
/// policy limits, and `Decimal` must be exact either side of zero).
fn signed_line_amount() -> impl Strategy<Value = Decimal> {
    (line_amount(), any::<bool>())
        .prop_map(|(d, negative)| if negative { -d } else { d }.normalize())
}

/// A rate small enough that a full-range line amount times it still has a
/// mantissa below 2^96 - 1: at most 7,900 at ten places.
fn tiny_rate() -> impl Strategy<Value = Decimal> {
    (0u32..=10).prop_flat_map(|scale| {
        (0i128..=7_900, Just(scale))
            .prop_map(move |(mantissa, _)| Decimal::from_i128_with_scale(mantissa, scale))
    })
}

/// An FX rate with up to ten decimal places, no larger than 1,000: the
/// realistic band for one unit of a foreign currency in base units.
fn fx_rate() -> impl Strategy<Value = Decimal> {
    (0u32..=10).prop_flat_map(|scale| {
        let cap = 10i128.pow(3 + scale);
        (0i128..=cap, Just(scale))
            .prop_map(move |(mantissa, _)| Decimal::from_i128_with_scale(mantissa, scale))
    })
}

/// A `Decimal` of at most `digits` significant integer digits at every
/// scale 0..=28 (for the parsing round trips).
fn any_scale_value() -> impl Strategy<Value = Decimal> {
    (0u32..=Decimal::MAX_SCALE).prop_flat_map(|scale| {
        let cap = 10i128.pow(15);
        (0i128..=cap, Just(scale))
            .prop_map(move |(mantissa, _)| Decimal::from_i128_with_scale(mantissa, scale))
    })
}

// ---------------------------------------------------------- properties

proptest! {
    #![proptest_config(ProptestConfig::with_cases(cases()))]

    /// Addition is exact over the books' line range.
    #[test]
    fn prop_checked_add_is_exact(a in signed_line_amount(), b in signed_line_amount()) {
        let sum = a.checked_add(b).expect("a line-range sum always fits");
        prop_assert_eq!(rat(sum), rat(a) + rat(b));
    }

    /// Subtraction is exact over the books' line range.
    #[test]
    fn prop_checked_sub_is_exact(a in signed_line_amount(), b in signed_line_amount()) {
        let difference = a.checked_sub(b).expect("a line-range difference always fits");
        prop_assert_eq!(rat(difference), rat(a) - rat(b));
    }

    /// A sum of up to 1,000 line amounts equals the rational sum exactly.
    /// The worst case (1,000 × 1e15 at ten places) needs a 29-digit
    /// mantissa (10^28), inside `Decimal`'s 96 bits; this checks it.
    #[test]
    fn prop_sum_of_many_lines_is_exact(
        amounts in proptest::collection::vec(line_amount(), 0..=1000)
    ) {
        let mut total = Decimal::ZERO;
        let mut expected = BigRational::from_integer(BigInt::from(0));
        for a in &amounts {
            total = total.checked_add(*a).expect("line-range sums stay exact");
            expected += rat(*a);
        }
        prop_assert!(representable(&expected), "1,000 line-range amounts always fit");
        prop_assert_eq!(rat(total), expected);
    }

    /// Multiplication (a quantity times a unit price, an FX amount times
    /// its rate) is exact when the true product fits: here both mantissas
    /// multiply to at most 10^28, below `Decimal`'s 2^96 - 1.
    #[test]
    fn prop_checked_mul_is_exact(
        amount in (0i128..=10i128.pow(15)).prop_flat_map(|m| {
            (0u32..=10).prop_map(move |s| Decimal::from_i128_with_scale(m, s))
        }),
        rate in fx_rate(),
    ) {
        let product = amount.checked_mul(rate).expect("mantissas multiply below 2^96");
        prop_assert_eq!(rat(product), rat(amount) * rat(rate));
    }

    /// Multiplication stays exact at the top of the line range: a full
    /// quadrillion at ten places (a 26-digit mantissa) times a small rate
    /// (at most 7,900 at ten places). The product's mantissa stays below
    /// 2^96 - 1, so no rounding may happen.
    #[test]
    fn prop_checked_mul_at_the_line_range_is_exact(
        amount in line_amount(),
        rate in tiny_rate(),
    ) {
        let product = amount.checked_mul(rate).expect("mantissas multiply below 2^96");
        prop_assert_eq!(rat(product), rat(amount) * rat(rate));
        prop_assert!(representable(&(rat(amount) * rat(rate))));
    }

    /// `round_dp_with_strategy` is the correctly rounded rational for
    /// every strategy the engine uses, at every scale 0..=28.
    #[test]
    fn prop_round_dp_matches_correct_rounding(
        amount in signed_line_amount(),
        dp in 0u32..=Decimal::MAX_SCALE,
        strategy in prop_oneof![
            Just(RoundingStrategy::MidpointAwayFromZero),
            Just(RoundingStrategy::MidpointNearestEven),
            Just(RoundingStrategy::ToZero),
            Just(RoundingStrategy::ToNegativeInfinity),
        ],
    ) {
        let rounded = amount.round_dp_with_strategy(dp, strategy);
        prop_assert_eq!(rat(rounded), round_rat(&rat(amount), dp, strategy));
    }

    /// `rescale` rounds with `MidpointAwayFromZero`, as its documentation
    /// says, and never changes the value when scaling up.
    #[test]
    fn prop_rescale_matches_half_away_rounding(
        amount in signed_line_amount(),
        scale in 0u32..=Decimal::MAX_SCALE,
    ) {
        let mut rescaled = amount;
        rescaled.rescale(scale);
        prop_assert_eq!(
            rat(rescaled),
            round_rat(&rat(amount), scale, RoundingStrategy::MidpointAwayFromZero)
        );
    }

    /// `normalize` strips trailing zeros and changes no value.
    #[test]
    fn prop_normalize_preserves_value(amount in signed_line_amount()) {
        let normalized = amount.normalize();
        prop_assert_eq!(rat(normalized), rat(amount));
        prop_assert_eq!(normalized.normalize(), normalized, "normalize is idempotent");
        if !normalized.is_zero() && normalized.scale() > 0 {
            prop_assert_ne!(normalized.mantissa() % 10, 0, "no trailing zero survives");
        }
    }

    /// Ordering and equality across scales agree with the rationals: 1.0
    /// equals 1.00, and `cmp` follows value, not representation.
    #[test]
    fn prop_comparison_matches_rationals(a in signed_line_amount(), b in signed_line_amount()) {
        prop_assert_eq!(a.cmp(&b), rat(a).cmp(&rat(b)));
        prop_assert_eq!(a == b, rat(a) == rat(b));
        prop_assert_eq!(a.partial_cmp(&b), Some(rat(a).cmp(&rat(b))));
    }

    /// The same value written at any two scales compares equal: 1.0 is
    /// 1.00, 1.000 and so on.
    #[test]
    fn prop_equal_across_scales(
        mantissa in 0i128..=10i128.pow(15),
        scale in 0u32..=Decimal::MAX_SCALE,
        extra in 0u32..=12,
    ) {
        let target = (scale + extra).min(Decimal::MAX_SCALE);
        let extra = target - scale;
        let a = Decimal::from_i128_with_scale(mantissa, scale);
        let b = Decimal::from_i128_with_scale(mantissa * 10i128.pow(extra), target);
        prop_assert_eq!(a, b);
        prop_assert_eq!(a.cmp(&b), std::cmp::Ordering::Equal);
    }

    /// `from_str` and `to_string` round-trip at every scale 0..=28,
    /// against a string built independently from the mantissa and scale.
    #[test]
    fn prop_from_str_to_string_round_trips(amount in any_scale_value()) {
        let text = amount.to_string();
        let parsed = Decimal::from_str(&text).expect("our own rendering parses");
        prop_assert_eq!(parsed, amount);
        prop_assert_eq!(parsed.scale(), amount.scale());
        prop_assert_eq!(parsed.to_string(), text);

        // Build the plain decimal rendering from the parts, without
        // `to_string`, and parse that too: the same value, same digits.
        let mantissa = amount.mantissa();
        let scale = amount.scale();
        let digits = mantissa.unsigned_abs().to_string();
        let body = if scale == 0 {
            digits
        } else {
            let scale = scale as usize;
            let padded = if digits.len() <= scale {
                format!("{}{}", "0".repeat(scale + 1 - digits.len()), digits)
            } else {
                digits
            };
            let split = padded.len() - scale;
            format!("{}.{}", &padded[..split], &padded[split..])
        };
        let body = if mantissa.is_negative() { format!("-{body}") } else { body };
        let parsed = Decimal::from_str(&body).expect("a plain decimal parses");
        prop_assert_eq!(rat(parsed), rat(amount));
        prop_assert_eq!(parsed.scale(), amount.scale());
    }
}

// -------------------------------------------------------- edge cases

const MAX_MANTISSA: i128 = 79_228_162_514_264_337_593_543_950_335;

/// `Decimal::MAX`'s mantissa is 28 digits; a line's worst-case sum is 28
/// digits too, so the headroom is stated, not assumed.
#[test]
fn a_thousand_full_lines_fit_below_the_decimal_maximum() {
    let line = Decimal::from_i128_with_scale(10i128.pow(25), 10);
    assert_eq!(line, Decimal::from(1_000_000_000_000_000i64));
    let total = line * Decimal::from(1000u32);
    assert_eq!(total, Decimal::from(1_000_000_000_000_000_000i64));
    assert!(total < Decimal::MAX);
    assert_eq!(Decimal::from_i128_with_scale(MAX_MANTISSA, 0), Decimal::MAX);
}

#[test]
fn the_largest_mantissa_at_scale_zero_round_trips() {
    let text = "79228162514264337593543950335";
    let d = Decimal::from_str(text).unwrap();
    assert_eq!(d, Decimal::MAX);
    assert_eq!(d.scale(), 0);
    assert_eq!(d.to_string(), text);
}

#[test]
fn scale_28_round_trips_at_both_ends() {
    for (mantissa, text) in [
        (1i128, "0.0000000000000000000000000001"),
        (-1i128, "-0.0000000000000000000000000001"),
        (MAX_MANTISSA, "7.9228162514264337593543950335"),
        (-MAX_MANTISSA, "-7.9228162514264337593543950335"),
    ] {
        let d = Decimal::from_i128_with_scale(mantissa, 28);
        assert_eq!(d.to_string(), text);
        let parsed = Decimal::from_str(text).unwrap();
        assert_eq!(parsed, d);
        assert_eq!(parsed.scale(), 28);
        assert_eq!(parsed.to_string(), text);
    }
}

#[test]
fn negative_ties_round_away_from_zero() {
    for text in ["-0.5", "-1.5", "-2.5", "-0.125", "-0.135"] {
        let d = Decimal::from_str(text).unwrap();
        let expected = round_rat(&rat(d), 0, RoundingStrategy::MidpointAwayFromZero);
        assert_eq!(
            rat(d.round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero)),
            expected,
            "{text}"
        );
        let expected = round_rat(&rat(d), 2, RoundingStrategy::MidpointAwayFromZero);
        assert_eq!(
            rat(d.round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero)),
            expected,
            "{text}"
        );
    }
    // 0.005 is a tie at two places, away from zero on both signs.
    assert_eq!(
        Decimal::from_str("0.005")
            .unwrap()
            .round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero),
        Decimal::from_str("0.01").unwrap()
    );
    assert_eq!(
        Decimal::from_str("-0.005")
            .unwrap()
            .round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero),
        Decimal::from_str("-0.01").unwrap()
    );
}

#[test]
fn one_point_zero_equals_one_point_zero_zero() {
    let a = Decimal::from_str("1.0").unwrap();
    let b = Decimal::from_str("1.00").unwrap();
    assert_eq!(a, b);
    assert_eq!(a.cmp(&b), std::cmp::Ordering::Equal);
    assert_ne!(a.to_string(), b.to_string(), "the scales still differ");
}

#[test]
fn checked_add_and_sub_return_none_past_the_maximum() {
    assert_eq!(Decimal::MAX.checked_add(Decimal::ONE), None);
    assert_eq!(Decimal::MIN.checked_sub(Decimal::ONE), None);
    assert_eq!(
        Decimal::MAX.checked_add(-Decimal::ONE),
        Some(Decimal::MAX - Decimal::ONE)
    );
}

/// Inexact at the edge: within one unit of `Decimal::MAX`, `checked_add`
/// rounds the fraction away instead of refusing (K-U4's finding). Pinned
/// so the day `rust_decimal` makes it exact, this test says so; the
/// kernel's `Amount::checked_add` refuses it (K-U10).
#[test]
fn checked_add_rounds_dust_away_next_to_the_maximum() {
    let dust = Decimal::new(4, 1);
    assert_eq!(Decimal::MAX.checked_add(dust), Some(Decimal::MAX));
    assert_eq!(Decimal::MAX.checked_add(Decimal::new(5, 1)), None);
}

#[test]
fn checked_mul_returns_none_when_the_integer_part_cannot_fit() {
    assert_eq!(Decimal::MAX.checked_mul(Decimal::from(2u32)), None);
    assert_eq!(Decimal::MIN.checked_mul(Decimal::from(2u32)), None);
}

/// When the *scale* cannot fit, `checked_mul` does not fail: it rounds the
/// product to 28 decimal places, ties to even (`ops::mul`'s reduction
/// path), and returns `Some`. 5e-28 × 0.5 is 2.5e-28, a tie between 2e-28
/// and 3e-28 at scale 28, and the even one wins.
#[test]
fn checked_mul_rounds_when_only_the_scale_does_not_fit() {
    let a = Decimal::from_i128_with_scale(5, 28);
    let half = Decimal::from_str("0.5").unwrap();
    assert_eq!(
        a.checked_mul(half),
        Some(Decimal::from_i128_with_scale(2, 28))
    );

    // 15e-28 × 0.5 = 7.5e-28: a tie, and 8 is even.
    let a = Decimal::from_i128_with_scale(15, 28);
    assert_eq!(
        a.checked_mul(half),
        Some(Decimal::from_i128_with_scale(8, 28))
    );
}

// ------------------------------------------------- engine reachability

/// The engine's FX path (`Ledger::add_line_fx`) multiplies a foreign
/// amount by a rate, and only bounds the *base* result afterwards. The
/// foreign amount and the rate are any positive `Decimal`, so a product
/// whose scale does not fit is reachable through the public API: that
/// product is rounded (the test above), not refused. Products whose
/// integer part does not fit do return `None`, which the engine maps to
/// `AmountOutOfRange`. This test states the reachability, not a claim
/// that the rounded case is wrong: the base amount is then rounded to the
/// currency's minor unit, far coarser than the lost places.
#[test]
fn the_fx_product_rounding_is_reachable_from_public_decimal_arguments() {
    let foreign = Decimal::from_i128_with_scale(15, 28); // 1.5e-27
    let rate = Decimal::from_str("0.5").unwrap();
    assert_eq!(
        foreign.checked_mul(rate),
        Some(Decimal::from_i128_with_scale(8, 28)),
        "a scale-reduced product, not None"
    );
    assert_eq!(Decimal::MAX.checked_mul(Decimal::MAX), None);
}
