//! U5 bridge properties: equal monthly parts (kernel sweep).
//!
//! `proofs/schedule.rs` proves `even_parts` on `i64` (whole units, fixed
//! months and steps); the engine runs the same code on `Decimal`. These
//! properties check that the two instantiations agree where the integer
//! quotient is exact, that `Decimal` agrees with an independent exact
//! rational reference (an `i128` numerator over an `i128` denominator,
//! rounded half to even, sharing no code with the kernel and no `Decimal`
//! division) at the steps of USD (0.01), JPY (1) and KWD (0.001), that the
//! laws of the proofs hold for any month count (the Kani harnesses fix
//! it), and the edge table: `Decimal::MAX`, scale 28, and no months, which
//! must be `None`, never a panic.
//!
//! What rounds, stated plainly (the K-U10 item): `even_parts` divides with
//! `Decimal`'s own division, exact where the quotient terminates and
//! otherwise rounded at 28 significant digits, and then rounds that
//! quotient to the step in exact integer arithmetic. For a total whose
//! quotient fits the 28 digits to well below the step (every total up to
//! about 10^15, which the properties use), that is the exact rational
//! rounded once, and the reference agrees. For a total near `Decimal::MAX`
//! the quotient itself carries a half-unit of error, so the part is not
//! within half a step of the exact quotient: the parts still sum to the
//! total (the plug is exact), and `the_edges` pins what happens.

use ledger_core::kernel;
use proptest::prelude::*;
use rust_decimal::Decimal;

/// The exact rational reference: `tm x 10^-ts` over `months`, each part
/// the quotient rounded half to even to `places` places, the last the
/// total less the others. Plain `i128` integers: the count is
/// `round_half_even(tm x 10^(places - ts) / months)`, found by integer
/// division and its remainder. `None` when a number leaves `i128` or the
/// answer leaves `Decimal`'s 96 bits.
fn reference(tm: i128, ts: u32, months: u32, places: u32) -> Option<(Decimal, Decimal)> {
    let m = i128::from(months);
    let (num, den) = if places >= ts {
        (tm.checked_mul(10i128.checked_pow(places - ts)?)?, m)
    } else {
        (tm, m.checked_mul(10i128.checked_pow(ts - places)?)?)
    };
    let negative = num < 0;
    let magnitude = num.unsigned_abs();
    let den = den.unsigned_abs();
    let floor = magnitude / den;
    let twice = (magnitude % den).checked_mul(2)?;
    let up = twice > den || (twice == den && floor % 2 == 1);
    let count = i128::try_from(if up { floor + 1 } else { floor }).ok()?;
    let count = if negative { -count } else { count };
    // Both at the wider of the two scales.
    let scale = ts.max(places);
    let total = tm.checked_mul(10i128.checked_pow(scale - ts)?)?;
    let each = count.checked_mul(10i128.checked_pow(scale - places)?)?;
    let last = total.checked_sub(each.checked_mul(m - 1)?)?;
    Some((
        Decimal::try_from_i128_with_scale(each, scale).ok()?,
        Decimal::try_from_i128_with_scale(last, scale).ok()?,
    ))
}

/// `even_parts` on `Decimal`: the parts, or None.
fn decimal_parts(total: Decimal, step: Decimal, months: usize) -> Option<Vec<Decimal>> {
    let mut parts = vec![Decimal::ZERO; months];
    kernel::even_parts(total, step, &mut parts)?;
    Some(parts)
}

/// `even_parts` on `i64`: the parts, or None.
fn integer_parts(total: i64, step: i64, months: usize) -> Option<Vec<i64>> {
    let mut parts = vec![0i64; months];
    kernel::even_parts(total, step, &mut parts)?;
    Some(parts)
}

fn step_of(places: u32) -> Decimal {
    Decimal::try_from_i128_with_scale(1, places).unwrap()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(3000))]

    /// The Kani model (whole cents, an integer step in cents) and the
    /// engine (dollars) agree on the same plan wherever the integer
    /// quotient is exact, so truncation moves nothing: the shared case.
    #[test]
    fn i64_and_decimal_agree_where_the_quotient_is_exact(
        each_cents in -(1i64 << 30)..=(1i64 << 30),
        months in 1usize..=600,
        step_cents in prop_oneof![Just(1i64), Just(5), Just(10), Just(100)],
    ) {
        let total_cents = each_cents * months as i64;
        let ours = integer_parts(total_cents, step_cents, months).unwrap();
        let theirs = decimal_parts(
            Decimal::new(total_cents, 2),
            Decimal::new(step_cents, 2),
            months,
        )
        .unwrap();
        prop_assert_eq!(
            ours.iter().map(|&c| Decimal::new(c, 2)).collect::<Vec<_>>(),
            theirs
        );
    }

    /// A single month is the whole total, even finer than the step (the
    /// plug takes everything): on both types.
    #[test]
    fn one_month_is_the_whole_total(
        total in -(1i64 << 40)..=(1i64 << 40),
        step in prop_oneof![Just(1i64), Just(2), Just(10), Just(100)],
        ts in 0u32..=6,
    ) {
        let mut one = [0i64];
        prop_assert_eq!(kernel::even_parts(total, step, &mut one), Some(()));
        prop_assert_eq!(one, [total]);
        let exact = Decimal::new(total, ts);
        let mut one = [Decimal::ZERO];
        prop_assert_eq!(kernel::even_parts(exact, step_of(2), &mut one), Some(()));
        prop_assert_eq!(one, [exact]);
    }

    /// The proofs' laws for any month count (two or more) on `i64`: the parts sum to the
    /// total, all but the last are one whole multiple of the step within
    /// half a step of the (truncated) quotient, and the last is the total
    /// less the others.
    #[test]
    fn the_i64_laws_hold_for_any_month_count(
        total in -(1i64 << 40)..=(1i64 << 40),
        months in 2usize..=600,
        step in prop_oneof![Just(1i64), Just(2), Just(5), Just(10), Just(100), Just(1000)],
    ) {
        let parts = integer_parts(total, step, months).unwrap();
        prop_assert_eq!(parts.iter().sum::<i64>(), total);
        let each = parts[0];
        prop_assert!(parts[..months - 1].iter().all(|&p| p == each));
        prop_assert_eq!(each % step, 0);
        prop_assert!(2 * (each - total / months as i64).abs() <= step);
        prop_assert_eq!(parts[months - 1], total - each * (months as i64 - 1));
    }

    /// The exact reference, for totals of every scale 0 to 6 and up to 15
    /// digits (so the quotient fits `Decimal`'s 28 digits to well below
    /// the step), 1 to 600 months, and the steps of JPY, USD and KWD
    /// (and a four-place unit): `Decimal`'s parts are the reference's,
    /// value for value, for either sign.
    #[test]
    fn decimal_matches_the_exact_reference(
        tm in -(1_000_000_000_000_000i128)..=1_000_000_000_000_000i128,
        ts in 0u32..=6,
        months in 2u32..=600,
        places in prop_oneof![Just(0u32), Just(2u32), Just(3u32), Just(4u32)],
    ) {
        let total = Decimal::try_from_i128_with_scale(tm, ts).unwrap();
        let want = reference(tm, ts, months, places).expect("fits");
        let parts = decimal_parts(total, step_of(places), months as usize).unwrap();
        prop_assert_eq!(parts[0], want.0);
        prop_assert!(parts[..months as usize - 1].iter().all(|&p| p == want.0));
        prop_assert_eq!(*parts.last().unwrap(), want.1);
    }

    /// The laws on `Decimal` for any month count: the parts sum to the
    /// total exactly, all but the last are equal and whole multiples of
    /// the step, and `months x each` is within half a step a month of the
    /// total (so `each` is within half a step of the exact quotient,
    /// strictly: no truncation here).
    #[test]
    fn the_decimal_laws_hold_for_any_month_count(
        tm in 0i128..=1_000_000_000_000_000i128,
        ts in 0u32..=6,
        months in 2usize..=600,
        places in prop_oneof![Just(0u32), Just(2u32), Just(3u32)],
    ) {
        let total = Decimal::try_from_i128_with_scale(tm, ts).unwrap();
        let step = step_of(places);
        let parts = decimal_parts(total, step, months).unwrap();
        prop_assert_eq!(parts.iter().copied().sum::<Decimal>(), total);
        let each = parts[0];
        prop_assert!(parts[..months - 1].iter().all(|&p| p == each));
        prop_assert_eq!((each / step).fract(), Decimal::ZERO);
        let off = (each * Decimal::from(months as u32) - total).abs();
        prop_assert!(off * Decimal::TWO <= Decimal::from(months as u32) * step);
        // The last is `each` plus what the others left.
        let last = *parts.last().unwrap();
        prop_assert_eq!(last, total - each * Decimal::from(months as u32 - 1));
    }
}

/// The edge table, each row checked by hand.
#[test]
fn the_edges() {
    let parse = |s: &str| -> Decimal { s.parse().unwrap() };
    let cent = parse("0.01");
    // No months, and a step that is no step: None, never a guess.
    assert_eq!(decimal_parts(parse("100"), cent, 0), None);
    assert_eq!(decimal_parts(parse("100"), Decimal::ZERO, 3), None);
    assert_eq!(decimal_parts(parse("100"), parse("-0.01"), 3), None);
    assert_eq!(integer_parts(100, 0, 3), None);
    assert_eq!(integer_parts(100, 1, 0), None);
    // The three currencies of the plans, by hand: 100,000 yen over 3 is
    // 33,333.33..., to the yen 33,333, the last 100,000 - 66,666 = 33,334;
    // 1,000.00 dollars is 333.33, 333.33, 333.34; 1.000 dinar is 0.333,
    // 0.333, 0.334.
    let plan = |t: &str, step: &str| decimal_parts(parse(t), parse(step), 3).unwrap();
    assert_eq!(
        plan("100000", "1"),
        [parse("33333"), parse("33333"), parse("33334")]
    );
    assert_eq!(
        plan("1000.00", "0.01"),
        [parse("333.33"), parse("333.33"), parse("333.34")]
    );
    assert_eq!(
        plan("1.000", "0.001"),
        [parse("0.333"), parse("0.333"), parse("0.334")]
    );
    // A tie goes to the even multiple (5 / 2 = 2.5 to 2; 7 / 2 = 3.5 to
    // 4), the last taking the rest, which may be the smaller.
    assert_eq!(
        decimal_parts(parse("5"), parse("1"), 2).unwrap(),
        [parse("2"), parse("3")]
    );
    assert_eq!(
        decimal_parts(parse("7"), parse("1"), 2).unwrap(),
        [parse("4"), parse("3")]
    );
    assert_eq!(
        decimal_parts(parse("-7"), parse("1"), 2).unwrap(),
        [parse("-4"), parse("-3")]
    );
    // A total smaller than its months: every other month rounds to
    // nothing and the last takes it all (0.01 over 3 at the cent).
    assert_eq!(
        decimal_parts(parse("0.01"), cent, 3).unwrap(),
        [Decimal::ZERO, Decimal::ZERO, parse("0.01")]
    );
    // Scale 28, the finest Decimal holds: 1e-28 over 2 rounds to nothing
    // at any step, and the last takes the whole.
    let tiny = Decimal::try_from_i128_with_scale(1, 28).unwrap();
    assert_eq!(decimal_parts(tiny, cent, 2).unwrap(), [Decimal::ZERO, tiny]);
    let widest = Decimal::MAX;
    // Decimal::MAX: no panic, and the parts still sum to the total. The
    // answer is not None, because the plug is exact; but the quotient
    // MAX / 2 = ...167.5 has 29 digits, so Decimal's division rounds it to
    // ...168 first, and the part is half a unit from the exact quotient,
    // a hundred steps at the cent: the K-U10 item (the division's, left
    // as it is), which is why the callers refuse totals of 10^26 or more
    // (EXACT_LIMIT). The sum of the parts is exact or refused (K-U10):
    // 599 parts of 1.3e26 at the cent need more than 96 bits at scale 2,
    // and `rust_decimal` used to round that sum.
    for months in [1usize, 2, 3, 7, 600] {
        for step in ["1", "0.01", "0.001"] {
            let parts = decimal_parts(widest, parse(step), months);
            if months == 600 && step != "1" {
                assert_eq!(
                    parts, None,
                    "{months} months at {step}: the sum is not exact"
                );
                continue;
            }
            let parts =
                parts.expect("MAX fits: the quotient is smaller, the last is the remainder");
            assert_eq!(
                kernel::total(parts.iter().copied()),
                Some(widest),
                "{months} months at {step}"
            );
        }
    }
    let halves = decimal_parts(widest, cent, 2).unwrap();
    assert_eq!(halves[0], parse("39614081257132168796771975168"));
    assert_eq!(halves[1], parse("39614081257132168796771975167"));
    // A step as large as the total: MAX / 2 rounds up at 28 digits to a
    // hair over half the step, so the part is the whole step (the same
    // K-U10 half unit) and the last takes nothing.
    assert_eq!(
        decimal_parts(widest, widest, 2),
        Some(vec![widest, Decimal::ZERO])
    );
    assert_eq!(
        decimal_parts(Decimal::MIN, cent, 1),
        Some(vec![Decimal::MIN])
    );
    // The maximum scale the engine accepts for a total, 28 places, over
    // 7 months at the cent: 7.9228162514264337593543950335 / 7 = 1.13...
    let max28 =
        Decimal::try_from_i128_with_scale(79_228_162_514_264_337_593_543_950_335, 28).unwrap();
    let parts = decimal_parts(max28, cent, 7).unwrap();
    assert_eq!(parts[0], parse("1.13"));
    assert_eq!(parts.iter().copied().sum::<Decimal>(), max28);
}
