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
/// returned exactly (the FX rule, B-3).
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
