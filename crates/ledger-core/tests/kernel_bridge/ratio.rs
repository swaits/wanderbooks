//! U6 bridge properties: percentages and ratio splits (kernel sweep).
//!
//! `proofs/ratio.rs` proves `percent_of` and `split_ratio` on `i64`
//! (whole eight, unit step; a 1 : 2 split); the engine runs the same code
//! on `Decimal`. These properties check that the two instantiations agree
//! where the integer quotient is exact, that `Decimal` agrees with an
//! independent exact `i128` reference (integer numerator and denominator,
//! rounded by mode, sharing no code with the kernel and doing no `Decimal`
//! division at the reference), that a split's parts sum on every input,
//! and the edge table: a zero whole, a negative whole, `Decimal::MAX`,
//! scale 28, and a zero ratio, which must be `None` or exact, never a
//! panic.
//!
//! What rounds, stated plainly (the K-U10 item): `percent_of` divides
//! first (`Decimal`'s own division, exact where it terminates and
//! otherwise rounded at 28 significant digits), then multiplies by 100
//! (a 96-bit product that can round), then rounds once to the step in
//! exact integer arithmetic. `split_ratio` multiplies before dividing
//! (`share_of`), so `Decimal`'s product can round first. For every figure
//! the call sites produce (lines are capped at 10^15), both operations are
//! exact and the reference agrees; at `Decimal`'s mantissa edge the
//! reference and the engine can differ, which is K-U10's to settle.

use ledger_core::kernel::{self, Rounding};
use proptest::prelude::*;
use rust_decimal::Decimal;

fn mode(value: u8) -> Rounding {
    match value {
        0 => Rounding::HalfAwayFromZero,
        1 => Rounding::HalfToEven,
        2 => Rounding::TowardZero,
        _ => Rounding::TowardNegativeInfinity,
    }
}

fn step(places: u32) -> Decimal {
    Decimal::try_from_i128_with_scale(1, places).expect("a step")
}

fn power(exp: u32) -> Option<i128> {
    10i128.checked_pow(exp)
}

/// The nearest integer to `num / den` in `mode`, as plain integer
/// arithmetic: no `Decimal` division and no kernel code.
fn rounded_quotient(num: i128, den: i128, mode: Rounding) -> Option<i128> {
    if den <= 0 {
        return None;
    }
    let negative = num < 0;
    let magnitude = num.unsigned_abs();
    let divisor = den.unsigned_abs();
    let floor = magnitude / divisor;
    let remainder = magnitude % divisor;
    let twice = remainder.checked_mul(2)?;
    let up = match mode {
        Rounding::TowardZero => false,
        Rounding::TowardNegativeInfinity => negative && remainder != 0,
        Rounding::HalfAwayFromZero => twice >= divisor,
        Rounding::HalfToEven => twice > divisor || (twice == divisor && !floor.is_multiple_of(2)),
    };
    let count = if up { floor.checked_add(1)? } else { floor };
    let count = i128::try_from(count).ok()?;
    Some(if negative { -count } else { count })
}

/// The exact percent reference: `100 × part ÷ whole`, rounded to a whole
/// multiple of `step` in `mode`, from the four mantissas and scales.
fn percent_reference(
    part_mantissa: i128,
    part_scale: u32,
    whole_mantissa: i128,
    whole_scale: u32,
    step_mantissa: i128,
    step_scale: u32,
    mode: Rounding,
) -> Option<Decimal> {
    if whole_mantissa <= 0 || step_mantissa <= 0 {
        return None;
    }
    // 100 x (pm 10^-ps) / (wm 10^-ws) / (sm 10^-ss)
    //   = (100 pm 10^(ws + ss)) / (wm 10^ps sm).
    let exponent = whole_scale.checked_add(step_scale)?;
    let numerator = 100i128
        .checked_mul(part_mantissa)?
        .checked_mul(power(exponent)?)?;
    let denominator = whole_mantissa
        .checked_mul(power(part_scale)?)?
        .checked_mul(step_mantissa)?;
    let count = rounded_quotient(numerator, denominator, mode)?;
    let mantissa = count.checked_mul(step_mantissa)?;
    Decimal::try_from_i128_with_scale(mantissa, step_scale).ok()
}

/// The exact first-part reference: `amount × a ÷ (a + b)`, rounded to a
/// whole multiple of `step` in `mode`.
fn split_first_reference(
    amount_mantissa: i128,
    amount_scale: u32,
    a: i128,
    b: i128,
    step_mantissa: i128,
    step_scale: u32,
    mode: Rounding,
) -> Option<Decimal> {
    let whole = a.checked_add(b)?;
    if a < 0 || b < 0 || whole <= 0 || step_mantissa <= 0 {
        return None;
    }
    // (am 10^-as) x a / whole / (sm 10^-ss)
    //   = (am a 10^ss) / (10^as whole sm).
    let numerator = amount_mantissa
        .checked_mul(a)?
        .checked_mul(power(step_scale)?)?;
    let denominator = power(amount_scale)?
        .checked_mul(whole)?
        .checked_mul(step_mantissa)?;
    let count = rounded_quotient(numerator, denominator, mode)?;
    let mantissa = count.checked_mul(step_mantissa)?;
    Decimal::try_from_i128_with_scale(mantissa, step_scale).ok()
}

fn arb_mode() -> impl Strategy<Value = Rounding> {
    (0u8..4).prop_map(mode)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(3000))]

    /// `Decimal`'s percent is the exact ratio times 100 rounded once, for
    /// parts and wholes of every sign and scale, every mode and step.
    #[test]
    fn decimal_percent_matches_the_exact_reference(
        part_mantissa in -(1_000_000_000i128)..=1_000_000_000i128,
        part_scale in 0u32..=4,
        whole_mantissa in 1i128..=1_000_000_000i128,
        whole_scale in 0u32..=4,
        step_mantissa in 1i128..=10_000i128,
        step_scale in 0u32..=2,
        mode in arb_mode(),
    ) {
        let part = Decimal::try_from_i128_with_scale(part_mantissa, part_scale).unwrap();
        let whole = Decimal::try_from_i128_with_scale(whole_mantissa, whole_scale).unwrap();
        let step = Decimal::try_from_i128_with_scale(step_mantissa, step_scale).unwrap();
        let want = percent_reference(
            part_mantissa, part_scale, whole_mantissa, whole_scale,
            step_mantissa, step_scale, mode,
        );
        prop_assert_eq!(kernel::percent_of(part, whole, step, mode), want);
    }

    /// The Kani model (whole percent points) and the engine (a `Decimal`
    /// percent) agree wherever the integer quotient is exact, so
    /// truncation moves nothing: `part = whole x factor` is `100 x
    /// factor` percent exactly.
    #[test]
    fn i64_and_decimal_percent_agree_where_the_quotient_is_exact(
        whole in 1i64..=1_000,
        factor in -1_000i64..=1_000,
        mode in arb_mode(),
    ) {
        let part = whole * factor;
        let ours = kernel::percent_of(part, whole, 1, mode).unwrap();
        let theirs = kernel::percent_of(
            Decimal::from(part),
            Decimal::from(whole),
            Decimal::ONE,
            mode,
        )
        .unwrap();
        prop_assert_eq!(Decimal::from(ours), theirs);
    }

    /// A split's parts sum to the whole on every input, the rounded side
    /// is a whole multiple of the step, and the rest is exact.
    #[test]
    fn decimal_split_sums(
        amount_mantissa in -(1_000_000_000_000i128)..=1_000_000_000_000i128,
        amount_scale in 0u32..=6,
        a in 0i128..=1_000,
        b in 0i128..=1_000,
        step_mantissa in 1i128..=10_000i128,
        step_scale in 0u32..=3,
        mode in arb_mode(),
    ) {
        prop_assume!(a + b > 0);
        let amount = Decimal::try_from_i128_with_scale(amount_mantissa, amount_scale).unwrap();
        let step = Decimal::try_from_i128_with_scale(step_mantissa, step_scale).unwrap();
        let (part, rest) =
            kernel::split_ratio(amount, Decimal::from(a), Decimal::from(b), step, mode).unwrap();
        prop_assert_eq!(part + rest, amount);
        prop_assert!((part % step).is_zero(), "the rounded side is a whole multiple");
        if rest.is_zero() {
            prop_assert!(!rest.is_sign_negative(), "a zero rest is +0");
        }
        if a == 0 {
            prop_assert!(part.is_zero());
            prop_assert_eq!(rest, amount);
        }
    }

    /// `Decimal`'s first part is the exact ratio rounded once, for every
    /// amount, ratio and step.
    #[test]
    fn decimal_split_matches_the_exact_reference(
        amount_mantissa in -(1_000_000_000i128)..=1_000_000_000i128,
        amount_scale in 0u32..=4,
        a in 0i128..=100,
        b in 0i128..=100,
        step_mantissa in 1i128..=1_000i128,
        step_scale in 0u32..=3,
        mode in arb_mode(),
    ) {
        prop_assume!(a + b > 0);
        let amount = Decimal::try_from_i128_with_scale(amount_mantissa, amount_scale).unwrap();
        let step = Decimal::try_from_i128_with_scale(step_mantissa, step_scale).unwrap();
        let want = split_first_reference(
            amount_mantissa, amount_scale, a, b, step_mantissa, step_scale, mode,
        );
        let (part, rest) =
            kernel::split_ratio(amount, Decimal::from(a), Decimal::from(b), step, mode).unwrap();
        prop_assert_eq!(Some(part), want);
        prop_assert_eq!(rest, amount - part);
    }

    /// The Kani model in cents and the engine in dollars agree on a
    /// 1 : 2 split wherever the integer quotient is exact (`amount` a
    /// multiple of three).
    #[test]
    fn i64_and_decimal_split_agree_where_the_quotient_is_exact(
        quotient in -100_000i64..=100_000,
        mode in arb_mode(),
    ) {
        let amount_cents = 3 * quotient;
        let (part_cents, rest_cents) = kernel::split_ratio(amount_cents, 1, 2, 1, mode).unwrap();
        let (part, rest) = kernel::split_ratio(
            Decimal::new(amount_cents, 2),
            Decimal::ONE,
            Decimal::from(2),
            step(2),
            mode,
        )
        .unwrap();
        prop_assert_eq!(part, Decimal::new(part_cents, 2));
        prop_assert_eq!(rest, Decimal::new(rest_cents, 2));
    }
}

/// A percent of no whole is `None`, the whole being zero or negative,
/// and no ratio to split is `None` too: no division by zero, never a
/// panic.
#[test]
fn percent_of_refuses_no_whole() {
    for mode in [
        Rounding::HalfAwayFromZero,
        Rounding::HalfToEven,
        Rounding::TowardZero,
        Rounding::TowardNegativeInfinity,
    ] {
        assert_eq!(
            kernel::percent_of(Decimal::ONE, Decimal::ZERO, step(1), mode),
            None
        );
        assert_eq!(
            kernel::percent_of(Decimal::ONE, -Decimal::ONE, step(1), mode),
            None
        );
        assert_eq!(
            kernel::percent_of(Decimal::ONE, Decimal::ONE, Decimal::ZERO, mode),
            None
        );
        assert_eq!(
            kernel::percent_of(Decimal::ONE, Decimal::ONE, -step(1), mode),
            None
        );
        assert_eq!(
            kernel::split_ratio(Decimal::ONE, Decimal::ZERO, Decimal::ZERO, step(2), mode),
            None
        );
        assert_eq!(
            kernel::split_ratio(Decimal::ONE, Decimal::ONE, Decimal::from(2), -step(2), mode),
            None
        );
        assert_eq!(
            kernel::split_ratio(Decimal::ONE, -Decimal::ONE, Decimal::from(2), step(2), mode),
            None
        );
        assert_eq!(
            kernel::split_ratio(
                Decimal::ONE,
                Decimal::ONE,
                Decimal::from(2),
                Decimal::ZERO,
                mode
            ),
            None
        );
    }
}

/// The edges: `Decimal::MAX`, scale 28, and a tiny whole. Each answer is
/// `None` or exact, never a panic.
#[test]
fn decimal_ratio_at_the_edges() {
    // A part past the type's range: the product by 100 is not
    // representable, so the percent refuses.
    assert_eq!(
        kernel::percent_of(Decimal::MAX, Decimal::ONE, step(1), Rounding::HalfToEven),
        None
    );
    // A tiny part at scale 28 is under a tenth of a percent: zero, never
    // a negative zero.
    let tiny = Decimal::try_from_i128_with_scale(1, 28).unwrap();
    let got = kernel::percent_of(tiny, Decimal::ONE, step(1), Rounding::HalfToEven).unwrap();
    assert!(got.is_zero() && !got.is_sign_negative());
    // A whole at scale 28 whose ratio is exactly one.
    let hundred = kernel::percent_of(tiny, tiny, step(1), Rounding::HalfToEven).unwrap();
    assert_eq!(hundred, Decimal::ONE_HUNDRED);
    // A whole past the type's range leaves a percent under a unit.
    let under = kernel::percent_of(Decimal::ONE, Decimal::MAX, step(1), Rounding::HalfToEven)
        .expect("a tiny percent is zero, not a refusal");
    assert!(under.is_zero());

    // A split at the amount's edge either refuses or its parts sum; it
    // never panics.
    for (a, b) in [(1, 1), (1, 2), (2, 1), (0, 1)] {
        if let Some((part, rest)) = kernel::split_ratio(
            Decimal::MAX,
            Decimal::from(a),
            Decimal::from(b),
            step(2),
            Rounding::HalfToEven,
        ) {
            assert_eq!(part + rest, Decimal::MAX);
        }
    }
    // A zero amount splits into two zero parts, and both are +0: a
    // checked subtraction used to hand the rest back as -0, which equality
    // hides but a sign test does not.
    let (part, rest) = kernel::split_ratio(
        Decimal::ZERO,
        Decimal::ONE,
        Decimal::from(2),
        step(2),
        Rounding::HalfToEven,
    )
    .expect("a zero amount splits");
    assert_eq!((part, rest), (Decimal::ZERO, Decimal::ZERO));
    assert!(part.is_zero() && !part.is_sign_negative());
    assert!(
        rest.is_zero() && !rest.is_sign_negative(),
        "a zero rest is +0"
    );

    // The same for a negative-zero amount, whose sign bit a subtraction can
    // carry: both parts come back +0.
    let negative_zero = -Decimal::ZERO;
    assert!(negative_zero.is_sign_negative());
    let (part, rest) = kernel::split_ratio(
        negative_zero,
        Decimal::ONE,
        Decimal::from(2),
        step(2),
        Rounding::HalfToEven,
    )
    .expect("a negative-zero amount splits");
    assert!(part.is_zero() && !part.is_sign_negative());
    assert!(
        rest.is_zero() && !rest.is_sign_negative(),
        "a zero rest is +0"
    );

    // A zero part over a positive whole is a zero percent, never -0.
    let zero = kernel::percent_of(
        Decimal::ZERO,
        Decimal::from(2),
        step(1),
        Rounding::HalfToEven,
    )
    .expect("a zero part has a percent");
    assert!(zero.is_zero() && !zero.is_sign_negative());
    let zero = kernel::percent_of(
        negative_zero,
        Decimal::from(2),
        step(1),
        Rounding::HalfToEven,
    )
    .expect("a negative-zero part has a percent");
    assert!(zero.is_zero() && !zero.is_sign_negative());
}
