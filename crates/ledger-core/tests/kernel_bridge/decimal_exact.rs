//! K-U10 bridge properties: `impl Amount for Decimal`'s add, subtract and
//! multiply are exact or `None`, never rounded (BS-ledger-1: a product the
//! arithmetic cannot hold MUST be refused; BS-entry-4).
//!
//! Kani works on `i64`, which never rounds, so the claim rests here: the
//! kernel's `Decimal` result is compared with `rust_decimal`'s own
//! (the "old" behaviour) and with an independent exact reference
//! (`num_rational::BigRational`, sharing no code with the kernel) over
//! the whole mantissa (to 2^96 - 1) and scale (0 to 28) range, edges
//! included. The law, per operation:
//!
//! 1. `Some(r)` is the exact result: `r` equals the rational one.
//! 2. Where the old result was exact, the kernel returns it unchanged,
//!    compared as strings (value and scale): nothing exact moves.
//! 3. Where the old result was rounded, the kernel returns `None`.
//! 4. Where the old result was `None`, so is the kernel's.
//!
//! And over the posted range (BS-entry-4's line cap and ten places) the
//! kernel equals the old for every input.

use ledger_core::kernel::{Amount, MAX_LINE_AMOUNT, MAX_LINE_SCALE};
use num_bigint::BigInt;
use num_rational::BigRational;
use proptest::prelude::*;
use rust_decimal::Decimal;

fn rat(d: Decimal) -> BigRational {
    BigRational::new(
        BigInt::from(d.mantissa()),
        BigInt::from(10u8).pow(d.scale()),
    )
}

fn shown(d: Option<Decimal>) -> String {
    d.map_or("None".into(), |d| format!("{d} (scale {})", d.scale()))
}

/// What the kernel must return, given what `rust_decimal` returned and
/// the exact result: the old figure if it is exact, else `None`.
fn expected(old: Option<Decimal>, exact: &BigRational) -> Option<Decimal> {
    old.filter(|o| rat(*o) == *exact)
}

/// Any `Decimal`: mantissas across the whole 96 bits (small, near the
/// top, trailing zeros) at every scale.
fn arb_decimal() -> impl Strategy<Value = Decimal> {
    let top = (1i128 << 96) - 1;
    (
        prop_oneof![
            -1_000i128..=1_000,
            -1_000_000_000_000i128..=1_000_000_000_000,
            -top..=top,
            (top - 1_000)..=top,
            -top..=-(top - 1_000),
            (-top / 1_000_000..=top / 1_000_000).prop_map(|m| m * 1_000_000),
            Just(top),
            Just(-top),
            Just(0),
        ],
        0u32..=28,
    )
        .prop_map(|(m, s)| Decimal::from_i128_with_scale(m, s))
}

/// A decimal and a second one at a nearby scale or the same: the
/// rescaling edge of an add is where the rounding lives.
fn arb_pair() -> impl Strategy<Value = (Decimal, Decimal)> {
    prop_oneof![
        (arb_decimal(), arb_decimal()),
        (arb_decimal(), 0u32..=28, -1_000i128..=1_000)
            .prop_map(|(a, s, m)| (a, Decimal::from_i128_with_scale(m, s))),
        (arb_decimal(), -1_000i128..=1_000)
            .prop_map(|(a, m)| (a, Decimal::from_i128_with_scale(m, a.scale()))),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(30000))]

    /// Add: laws 1 to 4, over the whole range.
    #[test]
    fn add_is_exact_or_none((a, b) in arb_pair()) {
        let got = Amount::checked_add(a, b);
        let old = Decimal::checked_add(a, b);
        let exact = rat(a) + rat(b);
        if let Some(r) = got {
            prop_assert_eq!(rat(r), exact.clone(), "{} + {} is rounded", a, b);
        }
        prop_assert_eq!(shown(got), shown(expected(old, &exact)));
    }

    /// Subtract: the same laws, against `rust_decimal`'s own subtraction
    /// (so a zero stays a plain zero).
    #[test]
    fn sub_is_exact_or_none((a, b) in arb_pair()) {
        let got = Amount::checked_sub(a, b);
        let old = Decimal::checked_sub(a, b);
        let exact = rat(a) - rat(b);
        if let Some(r) = got {
            prop_assert_eq!(rat(r), exact.clone(), "{} - {} is rounded", a, b);
        }
        prop_assert_eq!(shown(got), shown(expected(old, &exact)));
    }

    /// Multiply: the same laws.
    #[test]
    fn mul_is_exact_or_none((a, b) in arb_pair()) {
        let got = Amount::checked_mul(a, b);
        let old = Decimal::checked_mul(a, b);
        let exact = rat(a) * rat(b);
        if let Some(r) = got {
            prop_assert_eq!(rat(r), exact.clone(), "{} x {} is rounded", a, b);
        }
        prop_assert_eq!(shown(got), shown(expected(old, &exact)));
    }

    /// Whatever the kernel refuses, `rust_decimal` either refused or
    /// rounded: the kernel never refuses an exact figure `rust_decimal`
    /// held (restating laws 2 and 3 as the refusal's only two causes).
    #[test]
    fn a_refusal_is_an_overflow_or_a_rounding((a, b) in arb_pair()) {
        for (got, old, exact) in [
            (Amount::checked_add(a, b), Decimal::checked_add(a, b), rat(a) + rat(b)),
            (Amount::checked_mul(a, b), Decimal::checked_mul(a, b), rat(a) * rat(b)),
        ] {
            if got.is_none() {
                prop_assert!(old.is_none_or(|o| rat(o) != exact));
            }
        }
    }
}

/// A figure a posted line can carry (BS-entry-4): at most
/// `MAX_LINE_AMOUNT`, at most `MAX_LINE_SCALE` places once trailing zeros
/// are dropped; stored at any raw scale that still holds it.
fn arb_line_amount() -> impl Strategy<Value = Decimal> {
    (0u32..=MAX_LINE_SCALE, any::<bool>()).prop_flat_map(|(scale, negative)| {
        let cap = 10i128.pow(15 + scale);
        (0i128..=cap, 0u32..=28).prop_map(move |(m, want)| {
            let m = if negative { -m } else { m };
            // Up to `want` extra trailing zeros: the same value at a raw
            // scale of up to 28, as far as 96 bits hold it.
            let mut pad = 0;
            while pad < want
                && scale + pad < 28
                && m.checked_mul(10i128.pow(pad + 1))
                    .is_some_and(|p| p.unsigned_abs() < 1u128 << 96)
            {
                pad += 1;
            }
            Decimal::from_i128_with_scale(m * 10i128.pow(pad), scale + pad)
        })
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(20000))]

    /// Over the posted range, add and subtract equal `rust_decimal`'s, to
    /// the character, and are never `None`.
    #[test]
    fn posted_range_sums_are_unchanged(a in arb_line_amount(), b in arb_line_amount()) {
        prop_assert!(a.abs() <= MAX_LINE_AMOUNT && b.abs() <= MAX_LINE_AMOUNT);
        prop_assert!(a.normalize().scale() <= MAX_LINE_SCALE);
        let add = Amount::checked_add(a, b);
        let sub = Amount::checked_sub(a, b);
        prop_assert!(add.is_some() && sub.is_some());
        prop_assert_eq!(shown(add), shown(Decimal::checked_add(a, b)));
        prop_assert_eq!(shown(sub), shown(Decimal::checked_sub(a, b)));
    }

    /// Over the posted range, an FX product (a foreign amount and a
    /// rate of at most ten places each, D-75) is the old product to the
    /// character whenever its mantissa product fits 96 bits (the exact
    /// product is then representable), and `None` where it does not and
    /// the old one rounded.
    #[test]
    fn posted_range_fx_products_are_unchanged_or_refused(
        fm in 0i128..=10i128.pow(25),
        fs in 0u32..=MAX_LINE_SCALE,
        rm in 0i128..=10i128.pow(13),
        rs in 0u32..=MAX_LINE_SCALE,
    ) {
        let foreign = Decimal::from_i128_with_scale(fm, fs);
        let rate = Decimal::from_i128_with_scale(rm, rs);
        let got = Amount::checked_mul(foreign, rate);
        let old = Decimal::checked_mul(foreign, rate);
        let fits = fm.checked_mul(rm).is_some_and(|p| p < (1i128 << 96));
        if fits {
            prop_assert!(got.is_some());
            prop_assert_eq!(shown(got), shown(old));
        } else {
            // Past 96 bits at the full scale: the old product was rounded
            // unless the product's own trailing zeros made it fit.
            prop_assert_eq!(shown(got), shown(expected(old, &(rat(foreign) * rat(rate)))));
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1500))]

    /// A running total of posted lines (1e18 at the worst, ten places: a
    /// 29-digit mantissa) is unchanged at every step.
    #[test]
    fn posted_range_running_totals_are_unchanged(
        lines in proptest::collection::vec(arb_line_amount(), 0..=100)
    ) {
        let mut old = Decimal::ZERO;
        let mut new = Decimal::ZERO;
        for l in lines {
            old = Decimal::checked_add(old, l).expect("posted sums fit");
            new = Amount::checked_add(new, l).expect("posted sums stay exact");
            prop_assert_eq!(shown(Some(new)), shown(Some(old)));
        }
    }
}

/// The K-U4 canary, flipped: a sum within one unit of `Decimal::MAX`
/// that `rust_decimal` rounds is refused, and the exact ones stand.
#[test]
fn a_sum_past_exact_arithmetic_is_refused_at_the_maximum() {
    let max = Decimal::MAX;
    assert_eq!(Decimal::checked_add(max, Decimal::new(4, 1)), Some(max));
    assert_eq!(Amount::checked_add(max, Decimal::new(4, 1)), None);
    assert_eq!(Amount::checked_add(max, Decimal::new(5, 1)), None);
    assert_eq!(Amount::checked_add(max, Decimal::ONE), None);
    assert_eq!(Amount::checked_add(max, Decimal::ZERO), Some(max));
    assert_eq!(
        Amount::checked_add(max, -Decimal::ONE),
        Some(max - Decimal::ONE)
    );
    assert_eq!(Amount::checked_sub(Decimal::MIN, Decimal::new(4, 1)), None);
}

/// The K-U8 canary, flipped: `0.01 + 792281625142643375935439503.35`.
#[test]
fn a_sum_past_exact_arithmetic_is_refused_near_the_top() {
    let top = Decimal::from_i128_with_scale((1i128 << 96) - 1, 2);
    let cent = Decimal::new(1, 2);
    assert_eq!(
        shown(Decimal::checked_add(cent, top)),
        "792281625142643375935439503.4 (scale 1)",
        "rust_decimal rounds the exact ...503.36"
    );
    assert_eq!(Amount::checked_add(cent, top), None);
    assert_eq!(Amount::checked_add(top, cent), None);
    // The sums that are exact stay.
    assert_eq!(
        Amount::checked_add(-cent, top),
        Decimal::checked_add(-cent, top)
    );
}

/// The K-U1 canary, flipped: `1050000010.0000000001 x 9.9999999999` is
/// 10500000099.89499999999999999999 exactly, 20 places, 96 bits short;
/// `rust_decimal` held 10500000099.895 and the engine booked ...99.90.
/// The product is refused.
#[test]
fn a_product_past_exact_arithmetic_is_refused() {
    let foreign: Decimal = "1050000010.0000000001".parse().unwrap();
    let rate: Decimal = "9.9999999999".parse().unwrap();
    let held = Decimal::checked_mul(foreign, rate).unwrap();
    assert_ne!(rat(held), rat(foreign) * rat(rate), "rust_decimal rounded");
    assert_eq!(held.to_string(), "10500000099.895000000000000000");
    assert_eq!(Amount::checked_mul(foreign, rate), None);
    // K-U3's square, reachable below the line cap at ten places.
    let n: Decimal = "123456789.0123456789".parse().unwrap();
    assert_eq!(
        shown(Decimal::checked_mul(n, n)),
        "15241578753238836.750190519988 (scale 12)"
    );
    assert_eq!(Amount::checked_mul(n, n), None);
    // Past scale 28 the old product rounds (or underflows to zero).
    let tiny = Decimal::from_i128_with_scale(1, 28);
    assert_eq!(Decimal::checked_mul(tiny, tiny), Some(Decimal::ZERO));
    assert_eq!(Amount::checked_mul(tiny, tiny), None);
}

/// Exact products with trailing zeros stay, even where the full-scale
/// mantissa would pass 96 bits (a scale test would refuse these): 100 x
/// 0.3333333333333333333333333333 is 33.333...3300 exactly.
#[test]
fn an_exact_product_that_only_looks_wide_is_kept() {
    let third = Decimal::from_i128_with_scale(3_333_333_333_333_333_333_333_333_333, 28);
    let hundred = Decimal::ONE_HUNDRED;
    let got = Amount::checked_mul(third, hundred);
    assert_eq!(shown(got), shown(Decimal::checked_mul(third, hundred)));
    assert!(got.is_some());
    // 10^20 at scale 20 (one) squared: 10^40 mantissa, value 1.
    let one = Decimal::from_i128_with_scale(10i128.pow(20), 20);
    assert_eq!(
        shown(Amount::checked_mul(one, one)),
        shown(Decimal::checked_mul(one, one))
    );
    assert_eq!(
        Amount::checked_mul(one, one).map(rat),
        Some(rat(Decimal::ONE))
    );
}

/// Zero is exact in every shape, and a zero factor never refuses.
#[test]
fn zero_is_exact() {
    let z28 = Decimal::from_i128_with_scale(0, 28);
    let big = Decimal::MAX;
    assert_eq!(
        shown(Amount::checked_mul(z28, big)),
        shown(Decimal::checked_mul(z28, big))
    );
    assert!(Amount::checked_mul(z28, z28).is_some());
    assert_eq!(Amount::checked_add(z28, big), Some(big));
    assert_eq!(
        Amount::checked_add(Decimal::ZERO, z28).map(|d| d.is_zero()),
        Some(true)
    );
}

// ---------------------------------------------------------------- FX

use ledger_core::kernel::Rounding;

/// `q` rounded to an integer in `mode`, exactly (no kernel code).
fn round_rational(q: &BigRational, mode: Rounding) -> BigInt {
    let n = q.numer();
    let d = q.denom();
    let truncated = n / d;
    let rem = n - &truncated * d;
    let zero = BigInt::from(0);
    let negative = rem < zero;
    let twice = if negative { -&rem } else { rem.clone() } * 2;
    let away = if negative { -1 } else { 1 };
    match mode {
        Rounding::TowardZero => truncated,
        Rounding::TowardNegativeInfinity => {
            if negative {
                truncated - 1
            } else {
                truncated
            }
        }
        Rounding::HalfAwayFromZero => {
            if twice >= *d {
                truncated + away
            } else {
                truncated
            }
        }
        Rounding::HalfToEven => {
            if twice > *d || (twice == *d && truncated.bit(0)) {
                truncated + away
            } else {
                truncated
            }
        }
    }
}

fn arb_mode() -> impl Strategy<Value = Rounding> {
    prop_oneof![
        Just(Rounding::TowardZero),
        Just(Rounding::TowardNegativeInfinity),
        Just(Rounding::HalfAwayFromZero),
        Just(Rounding::HalfToEven),
    ]
}

/// A foreign amount or rate of at most ten places (D-75) and up to
/// `digits` digits of mantissa, so products reach 2^96 and past it.
fn arb_ten_place(digits: u32) -> impl Strategy<Value = Decimal> {
    (0i128..10i128.pow(digits), 0u32..=MAX_LINE_SCALE)
        .prop_map(|(m, s)| Decimal::from_i128_with_scale(m, s))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(30000))]

    /// `checked_mul_round_to_step` is the exact product rounded once, in
    /// every mode, at the steps the books use (one, a cent, a mill),
    /// whether or not the exact product fits 96 bits, ties and signs
    /// included; `None` only for a figure past about 10^23 (the real digits, the
    /// sticky digit and the step's places must fit 96 bits).
    #[test]
    fn the_fx_product_is_rounded_once_from_the_exact_one(
        foreign in arb_ten_place(26),
        rate in arb_ten_place(14),
        places in 0u32..=3,
        mode in arb_mode(),
        negative in any::<bool>(),
    ) {
        let foreign = if negative { -foreign } else { foreign };
        let step = Decimal::from_i128_with_scale(1, places);
        let got = Amount::checked_mul_round_to_step(foreign, rate, step, mode);
        let scale = BigRational::from_integer(BigInt::from(10u8).pow(places));
        let count = round_rational(&(rat(foreign) * rat(rate) * scale.clone()), mode);
        let want = BigRational::from_integer(count) / scale;
        match got {
            Some(g) => prop_assert_eq!(rat(g), want, "{} x {} to {} places, {:?}", foreign, rate, places, mode),
            None => prop_assert!(
                want.numer().magnitude() / want.denom().magnitude()
                    > BigInt::from(10u8).pow(23).magnitude().clone(),
                "{} x {} refused though it is only {}", foreign, rate, want
            ),
        }
    }
}

/// The K-U1 edge, now rounded once: the exact product is
/// 10500000099.89499999999999999999, so the cent is 10500000099.89 (the
/// engine used to book ...99.90, off by a cent, by rounding twice).
#[test]
fn the_k_u1_edge_books_the_cent_the_exact_product_gives() {
    let foreign: Decimal = "1050000010.0000000001".parse().unwrap();
    let rate: Decimal = "9.9999999999".parse().unwrap();
    let cent = Decimal::new(1, 2);
    assert_eq!(
        Amount::checked_mul_round_to_step(foreign, rate, cent, Rounding::HalfAwayFromZero),
        Some("10500000099.89".parse().unwrap())
    );
    // Products straddling a half cent by digits far below the cent, with
    // a 97-bit mantissa (so they cannot take the plain path): the sticky
    // digit puts each on its side of the tie.
    let rate: Decimal = "1.0000000001".parse().unwrap();
    let book = |foreign: &str, mode| {
        Amount::checked_mul_round_to_step(foreign.parse().unwrap(), rate, cent, mode)
    };
    let above = "1000000000.0050000001"; // 1000000000.10500000010050000001
    let below = "1000000000.0049999999"; // 1000000000.10499999990049999999
    assert!(
        Decimal::checked_mul(above.parse().unwrap(), rate)
            .is_some_and(|p| { rat(p) != rat(above.parse().unwrap()) * rat(rate) })
    );
    assert_eq!(
        book(above, Rounding::HalfAwayFromZero),
        Some("1000000000.11".parse().unwrap())
    );
    assert_eq!(
        book(above, Rounding::HalfToEven),
        Some("1000000000.11".parse().unwrap())
    );
    assert_eq!(
        book(above, Rounding::TowardZero),
        Some("1000000000.10".parse().unwrap())
    );
    assert_eq!(
        book(below, Rounding::HalfAwayFromZero),
        Some("1000000000.10".parse().unwrap())
    );
}
