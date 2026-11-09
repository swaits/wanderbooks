//! U1 bridge properties: foreign-currency conversion (kernel sweep).
//!
//! `proofs/fx.rs` proves `fx_booked` on `i64` (plain integers, fixed
//! steps); the engine runs the same code on `Decimal`. These properties
//! check that both instantiations agree, that `Decimal` agrees with an
//! independent exact rational reference (integer mantissas over powers
//! of ten, sharing no code with the kernel) for foreign amounts and
//! rates of every scale D-75 allows, 0 to 10 places each (so products of
//! up to 20 places), into base currencies of 0, 2 and 3 minor units
//! (JPY, USD, KWD), and the edge table: `Decimal::MAX`, scale 28,
//! half-unit ties of both signs, and a product that overflows (None,
//! never a panic).
//!
//! D-75 is what keeps the reference and the engine equal: past ten
//! places, `Decimal` rounds the product to 28 places before the kernel
//! rounds it to the minor unit, and the two disagree
//! (`past_ten_places_the_reference_and_decimal_part_and_the_engine_refuses`).
//! The scale is half of the exactness condition; the other half is that
//! the mantissas' product fits `Decimal`'s 96 bits (`the_mantissa_edge_*`).

use ledger_core::kernel::{Rounding, fx_booked, fx_booked_minor};
use ledger_core::{AccountType, Currency, Ledger, LedgerError, Side};
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
    let mut count = i128::try_from(count).ok()?;
    if negative {
        count = -count;
    }
    // The kernel returns an exact multiple already in range as it is, so a
    // count `places` places would push past 96 bits is compared at a
    // coarser scale (the same value, trailing zeros dropped).
    let mut scale = places;
    loop {
        if let Ok(value) = Decimal::try_from_i128_with_scale(count, scale) {
            return Some(value);
        }
        if scale == 0 || count % 10 != 0 {
            return None;
        }
        count /= 10;
        scale -= 1;
    }
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

    /// The exact reference above, on foreign amounts and rates of every
    /// scale D-75 allows (0 to 10 places each, so products to 20
    /// places), booked into 0, 2 and 3 minor units: the `Decimal`
    /// kernel is the exact product rounded once, both through a bare
    /// step and through the currency wrapper. The mantissas stay under
    /// 2^47 so their product fits `Decimal`'s 96 bits (the other half of
    /// exactness, pinned below).
    #[test]
    fn decimal_matches_the_exact_reference(
        fm in -(1i128 << 47)..=(1i128 << 47),
        fsc in 0u32..=10,
        rm in 1i128..=(1i128 << 47),
        rsc in 0u32..=10,
        places in prop_oneof![Just(0u32), Just(2u32), Just(3u32)],
    ) {
        let foreign = Decimal::try_from_i128_with_scale(fm, fsc).expect("in range");
        let rate = Decimal::try_from_i128_with_scale(rm, rsc).expect("in range");
        let want = reference(fm, fsc, rm, rsc, places);
        // A result `Decimal` cannot hold (a count past 96 bits) is None in
        // both: never a panic, never a wrapped amount.
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

/// A fresh USD ledger with an entry to add lines to.
fn usd_books() -> (Ledger, ledger_core::AccountId, ledger_core::EntryId) {
    let mut ledger = Ledger::new(Currency::USD);
    let cash = ledger
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    let entry = ledger.create_entry(chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(), "fx");
    (ledger, cash, entry)
}

/// The top of D-75's range: ten places each, a product of twenty places,
/// every digit of it kept: the exact product, rounded once.
#[test]
fn ten_places_each_is_exact_through_the_engine() {
    let (mut ledger, cash, entry) = usd_books();
    // 0.9999999999 x 0.0050000001 = 0.00499999999500000001...: let the
    // exact product be 0.00500000009...; a hand-checked row each side of
    // half a cent. 0.0000000001 x 0.0000000001 = 1e-20 books nothing.
    for (foreign, rate, want) in [
        // 0.4999999999 x 0.01 = 0.004999999999: under half a cent.
        ("0.4999999999", "0.0100000000", None),
        // 0.5000000001 x 0.01 = 0.005000000001: over half a cent.
        ("0.5000000001", "0.0100000000", Some("0.01")),
        // 0.9999999999 x 0.5000000001 = 0.50000000004999999999: 0.50.
        ("0.9999999999", "0.5000000001", Some("0.50")),
        // 99999.9999999999 x 99999.9999999999 = 9999999999.99998...: 10000000000.00.
        (
            "99999.9999999999",
            "99999.9999999999",
            Some("10000000000.00"),
        ),
    ] {
        let (f, r) = (parse(foreign), parse(rate));
        let booked = ledger.add_line_fx(entry, cash, Side::Debit, Currency::EUR, f, r);
        match want {
            None => assert!(booked.is_err(), "{foreign} x {rate} books nothing"),
            Some(want) => {
                booked.unwrap_or_else(|e| panic!("{foreign} x {rate}: {e}"));
                let lines = ledger.get_entry(entry).unwrap().lines();
                assert_eq!(lines.last().unwrap().amount.amount, parse(want));
            }
        }
        let (fm, fsc) = (f.mantissa(), f.scale());
        let (rm, rsc) = (r.mantissa(), r.scale());
        assert_eq!(
            fx_booked_minor(f, r, Currency::USD, Rounding::HalfAwayFromZero),
            reference(fm, fsc, rm, rsc, 2),
            "{foreign} x {rate}: kernel equals the exact reference"
        );
    }
}

/// D-75's bound is what keeps the reference and the engine equal: the
/// K-U1 review's input, 0.99999999999999999999999999 (26 places) x 0.005,
/// is 0.00499999999999999999999999995 exactly (under half a cent, so
/// 0.00), but `Decimal` rounds the product to 28 places first and the
/// kernel books 0.01. The engine refuses any foreign amount or rate of
/// eleven or more places before it computes.
#[test]
fn past_ten_places_the_reference_and_decimal_part_and_the_engine_refuses() {
    let foreign = parse("0.99999999999999999999999999");
    let rate = parse("0.005");
    assert_eq!(
        reference(
            foreign.mantissa(),
            foreign.scale(),
            rate.mantissa(),
            rate.scale(),
            2
        ),
        Some(parse("0.00")),
        "the exact product is under half a cent"
    );
    assert_eq!(
        fx_booked_minor(foreign, rate, Currency::USD, Rounding::HalfAwayFromZero),
        Some(parse("0.01")),
        "outside D-75 the kernel inherits Decimal's early rounding"
    );
    let (mut ledger, cash, entry) = usd_books();
    for (foreign, rate) in [
        (foreign, rate),
        (parse("1.00000000001"), Decimal::ONE),
        (Decimal::ONE, parse("1.00000000001")),
    ] {
        assert!(
            matches!(
                ledger.add_line_fx(entry, cash, Side::Debit, Currency::EUR, foreign, rate),
                Err(LedgerError::ValueOutOfRange(_))
            ),
            "{foreign} x {rate} is refused"
        );
    }
    assert!(ledger.get_entry(entry).unwrap().lines().is_empty());
}

/// The other half of exactness (found by this bridge, reported with
/// D-75): ten places each keep the product's scale within 28, but
/// `Decimal`'s mantissa is 96 bits (7.9e28), and a product of twenty
/// places past about 7.9e8 does not fit. `Decimal` then drops places
/// from the product, and the kernel rounds the already rounded value.
/// 1050000010.0000000001 x 9.9999999999 is 10500000099.89499999999999999999
/// exactly, and 10500000099.89 to the cent, but `Decimal` holds
/// 10500000099.895 and the kernel books 10500000099.90. Pinned as it is
/// today, outside the precondition of `fx_booked` (which says the
/// mantissas' product must fit); a unit to refuse it is the owner's call.
#[test]
fn the_mantissa_edge_is_outside_the_precondition_and_pinned() {
    let foreign = parse("1050000010.0000000001");
    let rate = parse("9.9999999999");
    assert_eq!(foreign.scale(), 10);
    assert_eq!(rate.scale(), 10);
    assert_eq!(
        reference(
            foreign.mantissa(),
            foreign.scale(),
            rate.mantissa(),
            rate.scale(),
            2
        ),
        Some(parse("10500000099.89"))
    );
    assert_eq!(
        fx_booked_minor(foreign, rate, Currency::USD, Rounding::HalfAwayFromZero),
        Some(parse("10500000099.90"))
    );
}
