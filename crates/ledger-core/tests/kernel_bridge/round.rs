//! U2 bridge properties: the rounding primitive (kernel sweep).
//!
//! `proofs/round.rs` proves `round_step` on `i64` money in the smallest
//! unit; the engine runs the same code on `Decimal`. These properties check
//! that both instantiations agree, that `Decimal` agrees with an
//! independent integer reference on amounts of up to ten decimal places,
//! that the primitive reproduces `round_dp_with_strategy` exactly (the
//! behaviour U2 preserves), and the edge table: halves both ways, a
//! sub-half amount, a binary-looking 1.005, scale 28 and `Decimal::MAX`.

use ledger_core::kernel::{Rounding, round_step};
use proptest::prelude::*;
use rust_decimal::Decimal;
use rust_decimal::RoundingStrategy;

fn parse(text: &str) -> Decimal {
    text.parse().expect("a decimal literal")
}

fn any_mode() -> impl Strategy<Value = Rounding> {
    prop_oneof![
        Just(Rounding::HalfAwayFromZero),
        Just(Rounding::HalfToEven),
        Just(Rounding::TowardZero),
        Just(Rounding::TowardNegativeInfinity),
    ]
}

fn strategy(mode: Rounding) -> RoundingStrategy {
    match mode {
        Rounding::HalfAwayFromZero => RoundingStrategy::MidpointAwayFromZero,
        Rounding::HalfToEven => RoundingStrategy::MidpointNearestEven,
        Rounding::TowardZero => RoundingStrategy::ToZero,
        Rounding::TowardNegativeInfinity => RoundingStrategy::ToNegativeInfinity,
    }
}

/// An independent integer reference: `value x 10^-scale` rounded to a
/// whole multiple of `10^-places`, in `mode`, using only `i128` division
/// and remainder. Shares no code with the kernel.
fn reference(value: i128, scale: u32, places: u32, mode: Rounding) -> Option<Decimal> {
    let common = scale.max(places);
    let amount = value.checked_mul(10i128.checked_pow(common - scale)?)?;
    let divisor = 10i128.checked_pow(common - places)?;
    let negative = amount < 0;
    let magnitude = amount.unsigned_abs();
    let divisor = u128::try_from(divisor).ok()?;
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
    let count = i128::try_from(count).ok()?;
    Decimal::try_from_i128_with_scale(if negative { -count } else { count }, places).ok()
}

/// An independent integer reference for any step `mantissa x 10^-step_scale`
/// (not only a power of ten): both operands go to their common scale as
/// `i128`, the quotient and remainder come from `i128` division, and the
/// result `count x step` is reduced by trailing zeros to the smallest scale
/// that holds it. Shares no code with the kernel, and no digit loop.
fn reference_step(
    value: i128,
    scale: u32,
    mantissa: i128,
    step_scale: u32,
    mode: Rounding,
) -> Option<Decimal> {
    let common = scale.max(step_scale);
    let amount = value.checked_mul(10i128.checked_pow(common - scale)?)?;
    let step = mantissa.checked_mul(10i128.checked_pow(common - step_scale)?)?;
    let negative = amount < 0;
    let magnitude = amount.unsigned_abs();
    let step = u128::try_from(step).ok()?;
    let floor = magnitude / step;
    let remainder = magnitude % step;
    let twice = remainder.checked_mul(2)?;
    let round_up = match mode {
        Rounding::TowardZero => false,
        Rounding::TowardNegativeInfinity => negative && remainder != 0,
        Rounding::HalfAwayFromZero => twice >= step,
        Rounding::HalfToEven => twice > step || (twice == step && !floor.is_multiple_of(2)),
    };
    let count = if round_up {
        floor.checked_add(1)?
    } else {
        floor
    };
    let mut result = i128::try_from(count.checked_mul(step)?).ok()?;
    let mut places = common;
    while places > 0 && result % 10 == 0 && result != 0 {
        result /= 10;
        places -= 1;
    }
    if result == 0 {
        places = 0;
    }
    let result = if negative { -result } else { result };
    Decimal::try_from_i128_with_scale(result, places).ok()
}

/// Rounds `value x 10^-scale` to the step `mantissa x 10^-step_scale` with
/// the kernel and with the reference, and asserts they are the same value.
fn assert_matches_reference(
    value: i128,
    scale: u32,
    mantissa: i128,
    step_scale: u32,
    mode: Rounding,
) -> Result<(), TestCaseError> {
    let amount = Decimal::try_from_i128_with_scale(value, scale).expect("in Decimal's range");
    let step = Decimal::try_from_i128_with_scale(mantissa, step_scale).expect("in range");
    let want = reference_step(value, scale, mantissa, step_scale, mode);
    prop_assert!(want.is_some(), "the reference fits for these ranges");
    prop_assert_eq!(round_step(amount, step, mode), want);
    Ok(())
}

/// The mantissas and scales of the finer-step tests: a step is any
/// `mantissa x 10^-scale`; a tie needs an even mantissa, so a tie row
/// builds its own.
fn step_parts() -> impl Strategy<Value = (i128, u32)> {
    (1i128..=1000, 0u32..=10)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    /// The `i64` kernel (the Kani model) and the `Decimal` kernel (the
    /// engine) agree on the same money in cents.
    #[test]
    fn i64_and_decimal_agree(
        value in -(1i64 << 40)..=(1i64 << 40),
        step in 1i64..=(1i64 << 20),
        mode in any_mode(),
    ) {
        let ours = round_step(value, step, mode).map(|r| Decimal::new(r, 2));
        prop_assert_eq!(
            ours,
            round_step(Decimal::new(value, 2), Decimal::new(step, 2), mode)
        );
    }

    /// The exact reference above, on books of up to ten decimal places and
    /// steps from one to a ten-thousandth: the `Decimal` primitive is the
    /// integer rounding, carried out exactly.
    #[test]
    fn decimal_matches_the_integer_reference(
        value in -(1i128 << 60)..=(1i128 << 60),
        scale in 0u32..=10,
        places in 0u32..=4,
        mode in any_mode(),
    ) {
        let amount = Decimal::try_from_i128_with_scale(value, scale).expect("in Decimal's range");
        let step = Decimal::new(1, places);
        prop_assert_eq!(
            round_step(amount, step, mode),
            reference(value, scale, places, mode)
        );
    }

    /// Steps that are not powers of ten: a mantissa 1..1000 at scales
    /// 0..10 against amounts at scales 0..28, every mode, checked against
    /// the independent reference. When the step carries more places than
    /// the amount, the kernel's digit loop runs with a mantissa above one
    /// (the finer-step branch); otherwise the coarser-step branch does.
    #[test]
    fn decimal_matches_the_reference_for_any_step(
        value in -(1i128 << 60)..=(1i128 << 60),
        scale in 0u32..=28,
        (mantissa, step_scale) in step_parts(),
        mode in any_mode(),
    ) {
        assert_matches_reference(value, scale, mantissa, step_scale, mode)?;
    }

    /// Small amounts, so exact multiples are common: the same comparison.
    #[test]
    fn decimal_matches_the_reference_for_any_step_on_small_amounts(
        value in -5000i128..=5000,
        scale in 0u32..=12,
        (mantissa, step_scale) in step_parts(),
        mode in any_mode(),
    ) {
        assert_matches_reference(value, scale, mantissa, step_scale, mode)?;
    }

    /// Exact ties: the amount is an odd number of half-steps, written with
    /// up to three trailing zeros removed (so the step often has more
    /// places than the amount). Every mode's tie rule against the reference.
    #[test]
    fn decimal_matches_the_reference_at_ties(
        q in 0i128..=10_000,
        half in 1i128..=100,
        tens in 0u32..=3,
        step_scale in 0u32..=10,
        strip in 0u32..=3,
        negative in any::<bool>(),
        mode in any_mode(),
    ) {
        let half = half * 10i128.pow(tens);
        let mantissa = 2 * half;
        let mut value = (2 * q + 1) * half;
        let mut scale = step_scale;
        for _ in 0..strip {
            if scale > 0 && value % 10 == 0 {
                value /= 10;
                scale -= 1;
            }
        }
        let value = if negative { -value } else { value };
        assert_matches_reference(value, scale, mantissa, step_scale, mode)?;
    }

    /// Behaviour preservation: with a power-of-ten step, `round_step` is
    /// exactly `round_dp_with_strategy`, the call sites' old primitive.
    #[test]
    fn round_step_is_round_dp_for_a_power_of_ten(
        value in -(1i128 << 60)..=(1i128 << 60),
        scale in 0u32..=10,
        places in 0u32..=10,
        mode in any_mode(),
    ) {
        let amount = Decimal::try_from_i128_with_scale(value, scale).expect("in Decimal's range");
        let mut want = amount.round_dp_with_strategy(places, strategy(mode));
        // The kernel never returns a negative zero; Decimal's rounding can.
        if want.is_zero() && want.is_sign_negative() {
            want = Decimal::ZERO;
        }
        prop_assert_eq!(round_step(amount, Decimal::new(1, places), mode), Some(want));
    }

    /// And `round_places` matches it to the character: same value, same
    /// scale, the call sites' exact output. (A negative zero becomes +0.)
    #[test]
    fn round_places_matches_round_dp_to_the_character(
        value in -(1i128 << 60)..=(1i128 << 60),
        scale in 0u32..=10,
        places in 0u32..=10,
        mode in any_mode(),
    ) {
        let amount = Decimal::try_from_i128_with_scale(value, scale).expect("in Decimal's range");
        let got = ledger_core::kernel::round_places(amount, places, mode).unwrap();
        let want = amount.round_dp_with_strategy(places, strategy(mode));
        if want.is_zero() {
            prop_assert!(got.is_zero());
            prop_assert!(!got.is_sign_negative());
        } else {
            prop_assert_eq!(got.to_string(), want.to_string());
        }
    }

    /// `rescale_exact` changes how an amount is written, never the
    /// amount. Over Decimal's whole range, against whole-number
    /// arithmetic that shares nothing with it: it refuses exactly when
    /// the amount is no whole multiple of 10^-places, and what it returns
    /// is the same value with the same sign, at `places` places or, where
    /// 96 bits cannot hold it that wide, as many as they can.
    #[test]
    fn rescale_exact_never_changes_the_value(
        mantissa in prop_oneof![
            -(1i128 << 60)..=(1i128 << 60),
            -MAX_MANTISSA..=MAX_MANTISSA,
            (0u32..=27, -9i128..=9).prop_map(|(zeros, digit)| digit * 10i128.pow(zeros)),
        ],
        scale in 0u32..=28,
        places in 0u32..=28,
    ) {
        use num_bigint::BigInt;
        let amount = Decimal::try_from_i128_with_scale(mantissa, scale).expect("in Decimal's range");
        let whole_multiple = places >= scale || mantissa % 10i128.pow(scale - places) == 0;
        let got = ledger_core::kernel::rescale_exact(amount, places);
        prop_assert_eq!(got.is_some(), whole_multiple, "{} at {} places", amount, places);
        if let Some(written) = got {
            // The same number: both mantissas brought to 28 places.
            let at_28 = |m: i128, s: u32| BigInt::from(m) * BigInt::from(10u8).pow(28 - s);
            prop_assert_eq!(at_28(written.mantissa(), written.scale()), at_28(mantissa, scale));
            prop_assert_eq!(written.is_sign_negative(), amount.is_sign_negative());
            let no_room = written.mantissa().unsigned_abs() * 10 > MAX_MANTISSA.unsigned_abs();
            prop_assert!(
                written.scale() == places || (written.scale() < places && no_room),
                "{} written as {} at {} places", amount, written, places
            );
        }
    }
}

/// Decimal's largest mantissa: 96 bits.
const MAX_MANTISSA: i128 = (1i128 << 96) - 1;

/// `rescale_exact` at the edges, each row checked by hand: the places a
/// figure prints with, a zero's sign, and the two refusals.
#[test]
fn rescale_exact_at_the_edges() {
    use ledger_core::kernel::rescale_exact;
    let text =
        |amount: &str, places: u32| rescale_exact(parse(amount), places).map(|d| d.to_string());
    assert_eq!(text("100", 1).as_deref(), Some("100.0"));
    assert_eq!(text("0.250", 2).as_deref(), Some("0.25"));
    assert_eq!(text("0", 2).as_deref(), Some("0.00"));
    assert_eq!(text("12.5", 1).as_deref(), Some("12.5"));
    assert_eq!(text("-3.10", 1).as_deref(), Some("-3.1"));
    // A digit would be lost: refused, where `Decimal::rescale` rounds.
    assert_eq!(text("0.255", 2), None);
    assert_eq!(text("-0.001", 2), None);
    assert_eq!(text("1", 29), None);
    // A negative zero keeps its sign and takes the places (a schedule's
    // posted month is compared as text on replay).
    let minus_zero = rescale_exact(-Decimal::ZERO, 2).unwrap();
    assert!(minus_zero.is_zero() && minus_zero.is_sign_negative());
    assert_eq!(minus_zero.scale(), 2);
    // No room for two more places in 96 bits: as many as fit, the value
    // untouched.
    let wide = Decimal::from_i128_with_scale(MAX_MANTISSA, 0);
    assert_eq!(rescale_exact(wide, 2), Some(wide));
    assert_eq!(rescale_exact(wide, 2).unwrap().scale(), 0);
    let nearly = Decimal::from_i128_with_scale(MAX_MANTISSA / 10, 0);
    assert_eq!(rescale_exact(nearly, 2).unwrap().scale(), 1);
}

/// The edge table, each row checked by hand.
#[test]
fn round_step_at_the_edges() {
    let cent = parse("0.01");
    // A half, both signs, in every mode.
    assert_eq!(
        round_step(parse("0.5"), cent, Rounding::HalfAwayFromZero),
        Some(parse("0.50"))
    );
    assert_eq!(
        round_step(parse("-0.5"), cent, Rounding::HalfAwayFromZero),
        Some(parse("-0.50"))
    );
    assert_eq!(
        round_step(parse("0.5"), cent, Rounding::HalfToEven),
        Some(parse("0.50"))
    );
    assert_eq!(
        round_step(parse("-0.5"), cent, Rounding::TowardNegativeInfinity),
        Some(parse("-0.50"))
    );
    // Under half a cent: zero, and never a negative zero.
    let tiny = round_step(parse("0.004"), cent, Rounding::HalfAwayFromZero).unwrap();
    assert!(tiny.is_zero());
    assert!(!tiny.is_sign_negative());
    let tiny = round_step(parse("-0.004"), cent, Rounding::HalfToEven).unwrap();
    assert!(tiny.is_zero());
    assert!(!tiny.is_sign_negative());
    // 1.005 is exactly a half at the cent (Decimal carries it exactly).
    assert_eq!(
        round_step(parse("1.005"), cent, Rounding::HalfAwayFromZero),
        Some(parse("1.01"))
    );
    assert_eq!(
        round_step(parse("1.005"), cent, Rounding::HalfToEven),
        Some(parse("1.00"))
    );
    assert_eq!(
        round_step(parse("-1.005"), cent, Rounding::HalfAwayFromZero),
        Some(parse("-1.01"))
    );
    assert_eq!(
        round_step(parse("-1.005"), cent, Rounding::HalfToEven),
        Some(parse("-1.00"))
    );
    assert_eq!(
        round_step(parse("-1.005"), cent, Rounding::TowardNegativeInfinity),
        Some(parse("-1.01"))
    );
    // Scale 28: a whole multiple, and a tie at the last place.
    let last = parse("0.0000000000000000000000000001");
    let five = parse("0.0000000000000000000000000005");
    assert_eq!(round_step(five, last, Rounding::HalfToEven), Some(five));
    let two_last = parse("0.0000000000000000000000000002");
    assert_eq!(
        round_step(five, two_last, Rounding::HalfToEven),
        Some(parse("0.0000000000000000000000000004"))
    );
    assert_eq!(
        round_step(five, two_last, Rounding::HalfAwayFromZero),
        Some(parse("0.0000000000000000000000000006"))
    );
    // Decimal's largest value is a whole multiple of a unit and a cent.
    assert_eq!(
        round_step(Decimal::MAX, Decimal::ONE, Rounding::HalfToEven),
        Some(Decimal::MAX)
    );
    assert_eq!(
        round_step(Decimal::MAX, cent, Rounding::HalfAwayFromZero),
        Some(Decimal::MAX)
    );
    // A step that is not positive is refused.
    assert_eq!(
        round_step(Decimal::ONE, Decimal::ZERO, Rounding::HalfToEven),
        None
    );
    assert_eq!(
        round_step(Decimal::ONE, -Decimal::ONE, Rounding::HalfToEven),
        None
    );
}

/// `round_places` is the behaviour-preserving wrapper the call sites use:
/// it keeps `round_dp_with_strategy`'s scale and its early return.
#[test]
fn round_places_keeps_the_old_scale() {
    use ledger_core::kernel::round_places;
    assert_eq!(
        round_places(parse("1.200"), 2, Rounding::HalfToEven),
        Some(parse("1.20"))
    );
    assert_eq!(
        round_places(parse("1.5"), 2, Rounding::HalfToEven),
        Some(parse("1.5"))
    );
    let zero = round_places(parse("0.004"), 2, Rounding::HalfAwayFromZero).unwrap();
    assert_eq!(zero.to_string(), "0.00");
    assert!(!zero.is_sign_negative());
}

/// `round_minor` picks the currency's own unit; a currency with none is
/// returned exactly (the FX rule, BS-ledger-1).
#[test]
fn round_minor_follows_the_currency() {
    use ledger_core::Currency;
    use ledger_core::kernel::round_minor;
    assert_eq!(
        round_minor(parse("1.2345"), Currency::USD, Rounding::HalfAwayFromZero),
        Some(parse("1.23"))
    );
    assert_eq!(
        round_minor(parse("1234.5"), Currency::JPY, Rounding::HalfAwayFromZero),
        Some(parse("1235"))
    );
    assert_eq!(
        round_minor(
            parse("1.2345"),
            Currency::new("KWD").unwrap(),
            Rounding::HalfAwayFromZero
        ),
        Some(parse("1.235"))
    );
    let gold = Currency::new("XAU").unwrap();
    assert_eq!(
        round_minor(parse("1.23456789"), gold, Rounding::HalfAwayFromZero),
        Some(parse("1.23456789"))
    );
}
