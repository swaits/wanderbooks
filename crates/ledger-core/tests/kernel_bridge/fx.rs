//! U1 bridge properties: foreign-currency conversion (kernel sweep).
//!
//! `proofs/fx.rs` proves `fx_booked` on `i64` (plain integers, fixed
//! steps); the engine runs the same code on `Decimal`. These properties
//! check that both instantiations agree, that `Decimal` agrees with an
//! independent exact rational reference (integer mantissas over powers
//! of ten, sharing no code with the kernel) for foreign amounts and
//! rates of up to ten places into base currencies of 0, 2 and 3 minor
//! units (JPY, USD, KWD), and the edge table: `Decimal::MAX`, scale 28,
//! half-unit ties of both signs, and a product that overflows (None,
//! never a panic).

use ledger_core::Currency;
use ledger_core::kernel::{Rounding, fx_booked, fx_booked_minor};
use proptest::prelude::*;
use rust_decimal::Decimal;

fn parse(text: &str) -> Decimal {
    text.parse().expect("a decimal literal")
}

/// An independent exact reference: the product of `fm x 10^-fsc` and
/// `rm x 10^-rsc` (both exact `i128` integers), rounded once half away
/// from zero to `places` places. No kernel code, no `Decimal` division.
fn reference(fm: i128, fsc: u32, rm: i128, rsc: u32, places: u32) -> Option<Decimal> {
    let product = fm.checked_mul(rm)?;
    let scale = fsc + rsc;
    let negative = product < 0;
    let magnitude = product.unsigned_abs();
    let count = if scale >= places {
        let divisor = 10u128.checked_pow(scale - places)?;
        let floor = magnitude / divisor;
        let remainder = magnitude % divisor;
        if remainder.checked_mul(2)? >= divisor {
            floor.checked_add(1)?
        } else {
            floor
        }
    } else {
        magnitude.checked_mul(10u128.checked_pow(places - scale)?)?
    };
    let count = i128::try_from(count).ok()?;
    Decimal::try_from_i128_with_scale(if negative { -count } else { count }, places).ok()
}

fn currency_for(places: u32) -> Currency {
    match places {
        0 => Currency::JPY,
        2 => Currency::USD,
        _ => Currency::new("KWD").unwrap(),
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    /// The `i64` kernel (the Kani model) and the `Decimal` kernel (the
    /// engine) agree on the same plain-integer conversion.
    #[test]
    fn i64_and_decimal_agree(
        foreign in -(1i64 << 20)..=(1i64 << 20),
        rate in 1i64..=(1i64 << 12),
        step in 1i64..=(1i64 << 12),
    ) {
        let ours = fx_booked(foreign, rate, step, Rounding::HalfAwayFromZero)
            .map(Decimal::from);
        prop_assert_eq!(
            ours,
            fx_booked(
                Decimal::from(foreign),
                Decimal::from(rate),
                Decimal::from(step),
                Rounding::HalfAwayFromZero,
            )
        );
    }

    /// The exact reference above, on foreign amounts and rates of up
    /// to ten places, booked into 0, 2 and 3 minor units: the
    /// `Decimal` kernel is the exact product rounded once, both
    /// through a bare step and through the currency wrapper.
    #[test]
    fn decimal_matches_the_exact_reference(
        fm in -(1i128 << 40)..=(1i128 << 40),
        fsc in 0u32..=10,
        rm in 1i128..=(1i128 << 40),
        rsc in 0u32..=10,
        places in prop_oneof![Just(0u32), Just(2u32), Just(3u32)],
    ) {
        let foreign = Decimal::try_from_i128_with_scale(fm, fsc).expect("in range");
        let rate = Decimal::try_from_i128_with_scale(rm, rsc).expect("in range");
        let want = reference(fm, fsc, rm, rsc, places);
        prop_assert!(want.is_some(), "the reference fits for these ranges");
        let step = Decimal::try_from_i128_with_scale(1, places).unwrap();
        prop_assert_eq!(
            fx_booked(foreign, rate, step, Rounding::HalfAwayFromZero),
            want
        );
        prop_assert_eq!(
            fx_booked_minor(foreign, rate, currency_for(places), Rounding::HalfAwayFromZero),
            want
        );
    }
}

/// The edge table, each row checked by hand.
#[test]
fn fx_booked_at_the_edges() {
    let cent = parse("0.01");
    // The ledger's own examples (BS-ledger-1): to the cent, to the yen.
    assert_eq!(
        fx_booked(
            parse("100.00"),
            parse("1.0834567"),
            cent,
            Rounding::HalfAwayFromZero
        ),
        Some(parse("108.35"))
    );
    assert_eq!(
        fx_booked_minor(
            parse("10.00"),
            parse("151.237"),
            Currency::JPY,
            Rounding::HalfAwayFromZero
        ),
        Some(parse("1512"))
    );
    // A thousandths currency (KWD), an exact half thousandth away.
    assert_eq!(
        fx_booked_minor(
            parse("1"),
            parse("1.2345"),
            Currency::new("KWD").unwrap(),
            Rounding::HalfAwayFromZero
        ),
        Some(parse("1.235"))
    );
    // Half-unit ties, both signs (the kernel books the magnitude).
    assert_eq!(
        fx_booked(parse("1"), parse("0.125"), cent, Rounding::HalfAwayFromZero),
        Some(parse("0.13"))
    );
    assert_eq!(
        fx_booked(
            parse("-1"),
            parse("0.125"),
            cent,
            Rounding::HalfAwayFromZero
        ),
        Some(parse("-0.13"))
    );
    // Scale 28: the finest amount, and a product that rounds to zero.
    let finest = parse("0.0000000000000000000000000001");
    assert_eq!(
        fx_booked(finest, Decimal::ONE, cent, Rounding::HalfAwayFromZero),
        Some(Decimal::ZERO)
    );
    // Decimal's own multiplication underflows the finest product to
    // zero (checked_mul holds it as 0); the kernel keeps that exactly,
    // as the pre-kernel computation did.
    assert_eq!(
        fx_booked(finest, finest, Decimal::ONE, Rounding::HalfAwayFromZero),
        Some(Decimal::ZERO)
    );
    // Decimal's largest value converts at rate one; at rate two the
    // product cannot be held: None, never a panic.
    assert_eq!(
        fx_booked(
            Decimal::MAX,
            Decimal::ONE,
            Decimal::ONE,
            Rounding::HalfAwayFromZero
        ),
        Some(Decimal::MAX)
    );
    assert_eq!(
        fx_booked(
            Decimal::MAX,
            Decimal::from(2),
            cent,
            Rounding::HalfAwayFromZero
        ),
        None
    );
    assert_eq!(
        fx_booked_minor(
            Decimal::MAX,
            Decimal::from(2),
            Currency::USD,
            Rounding::HalfAwayFromZero
        ),
        None
    );
    // A non-positive rate and a non-positive step are refused.
    assert_eq!(
        fx_booked(
            parse("100"),
            Decimal::ZERO,
            cent,
            Rounding::HalfAwayFromZero
        ),
        None
    );
    assert_eq!(
        fx_booked(
            parse("100"),
            parse("-1.5"),
            cent,
            Rounding::HalfAwayFromZero
        ),
        None
    );
    assert_eq!(
        fx_booked(
            parse("100"),
            Decimal::ONE,
            Decimal::ZERO,
            Rounding::HalfAwayFromZero
        ),
        None
    );
    // The booked scale is the old computation's: an exact product with
    // trailing zeros past the cents books at exactly the minor places,
    // and one already at the cents keeps its scale.
    let booked = fx_booked_minor(
        parse("1.50"),
        parse("1.00"),
        Currency::USD,
        Rounding::HalfAwayFromZero,
    )
    .unwrap();
    assert_eq!(booked.to_string(), "1.50");
    let booked = fx_booked_minor(
        parse("108.3"),
        Decimal::ONE,
        Currency::USD,
        Rounding::HalfAwayFromZero,
    )
    .unwrap();
    assert_eq!(booked.to_string(), "108.3");
    // A currency with no minor unit books the exact product.
    let gold = Currency::new("XAU").unwrap();
    assert_eq!(
        fx_booked_minor(
            parse("1.5"),
            parse("1.23456789"),
            gold,
            Rounding::HalfAwayFromZero
        ),
        Some(parse("1.851851835"))
    );
}
