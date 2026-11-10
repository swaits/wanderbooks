//! U1: foreign-currency conversion, the booking of a foreign amount at
//! the transaction-date rate (kernel sweep, `plan/kernel-sweep-audit.md`
//! section 3). The one place foreign x rate becomes booked money: the
//! product is computed exactly and rounded once, by [`round_step`](super::round_step), to
//! the base currency's smallest unit. No call site multiplies by a rate
//! itself.

use super::round::Rounding;
use crate::Currency;
use crate::kernel::Amount;
use rust_decimal::Decimal;

/// The booked base amount for a foreign amount at a rate: `foreign x
/// rate` computed exactly, rounded once to a whole multiple of `step`
/// (the base currency's minor unit) in `mode`.
///
/// The rate's precision is carried by the value itself: a `Decimal`
/// rate keeps its own scale (1.0834567 is exact), and the product is the
/// exact one before the single rounding. At `i64` (the Kani model)
/// amounts and the rate are plain integers in the smallest unit, and the
/// same law is proved of the product.
///
/// **The exact product on `Decimal` (D-75, K-U10).** A foreign amount and
/// a rate of at most [`MAX_LINE_SCALE`](super::MAX_LINE_SCALE) (10) places
/// each, trailing zeros not counted, have an exact product of at most 20
/// places, and `Ledger::add_line_fx` enforces that bound on every new
/// line (never on replay). The product's mantissa can still pass
/// `Decimal`'s 96 bits (a product of twenty places past about 7.9e8), and
/// `Decimal::checked_mul` then drops places. `Amount::checked_mul_round_to_step`
/// does not use it for that case: it holds the exact product in 256 bits,
/// cuts it down only as far as `Decimal` needs (remembering whether any
/// cut digit was not zero), and rounds once, so 1050000010.0000000001 x
/// 9.9999999999 (10500000099.89499999999999999999 exactly) books
/// 10500000099.89, where it booked 10500000099.90 through a rounded
/// 10500000099.895. The same holds past ten places: a product below 1e-28
/// (1e-28 x 1e-28) rounds to zero because its exact value does, and
/// 0.99999999999999999999999999 x 0.005 books 0.00 (the engine refuses
/// such an input all the same, D-75). `None` only for a figure too large
/// to hold with its rounding digits (about 7.9e24 at a cent).
///
/// ```text
/// GOAL: book a foreign amount in the ledger's one money: the exact
///       product foreign x rate, rounded once to the base currency's
///       smallest unit, half away from zero.
/// SOURCE: bookspec BS-ledger-1 (one money; a foreign amount at the
///         transaction-date rate, ASC 830-20-30-1; the smallest unit
///         from ISO 4217), and its CHOICE BS-ledger-14
///         "book-in-the-smallest-unit" (the owner, 2026-10-13: book in
///         the smallest unit, half away from zero); wanderbooks
///         docs/spec/claims/ledger.md BS-ledger-1 states the same pick. The
///         rounding itself is kernel::round_step (BS-ledger-1,
///         BS-entry-3); owner decision D-69 (the laws live once, in
///         bookspec's Lean).
/// INVARIANTS:
///   1. Some(r) is a whole multiple of step: r = n x step for an
///      integer n. None when step <= 0, when rate <= 0, or when the
///      rounded result cannot be held.
///   2. In HalfAwayFromZero (the mode add_line_fx books in) r is within
///      half a step of the exact product foreign x rate, an exact half
///      going to the multiple farther from zero: the product is rounded
///      once, from the exact one, however many bits it needs (K-U10).
///   3. Positive foreign and rate give a non-negative r, and a positive
///      r whenever the exact product reaches half a step (a smaller
///      product rounds to zero, which the caller refuses as it does
///      today). The sign never crosses zero.
///   4. On Decimal the value equals the pre-kernel computation
///      (checked_mul, then round_minor half away from zero) exactly
///      wherever that product was exact; where Decimal had rounded the
///      product first, the figure is now the once-rounded one;
///      fx_booked_minor also keeps its scale.
/// SCENARIOS:
///   1. IF 100.00 at 1.0834567 to the cent: SHOULD book 108.35.
///   2. IF 10.00 at 151.237 to the yen (step 1): SHOULD book 1512.
///   3. IF the rate is zero or negative: SHOULD be None, never a guess.
///   4. IF foreign x rate is too large to hold: SHOULD be None, never a
///      panic.
///   5. IF foreign is 0.99999999999999999999999999 (26 places) and
///      the rate 0.005 to the cent: SHOULD book 0.00, the exact product
///      being under half a cent (it booked 0.01 when Decimal rounded the
///      product first); the engine refuses such an input (D-75).
///   6. IF foreign 1050000010.0000000001 at 9.9999999999 to the cent
///      (ten places each, a product past 96 bits): SHOULD book
///      10500000099.89, the exact product's cent (it booked
///      10500000099.90 when Decimal rounded the product first).
/// ```
pub fn fx_booked<T: Amount>(foreign: T, rate: T, step: T, mode: Rounding) -> Option<T> {
    if rate <= T::ZERO {
        return None;
    }
    foreign.checked_mul_round_to_step(rate, step, mode)
}

/// [`fx_booked`] at a currency's minor unit, on `Decimal`: the spelling
/// `Ledger::add_line_fx` books through. A currency ISO 4217 gives no
/// minor unit (gold, SDRs) books the exact product, as `round_minor`
/// has always done (BS-ledger-1). The result's scale is the pre-kernel
/// computation's: `round_places` keeps an amount already at or under
/// the minor places unchanged, and otherwise carries exactly the minor
/// places (a booked 1.50 prints "1.50", never "1.5000").
///
/// SOURCE: BS-ledger-1 (law 41: a foreign amount booked at the
/// transaction-date rate is the exact product rounded once to the base
/// currency's smallest unit).
///
/// INVARIANTS:
///   1. It is `fx_booked` at the currency's minor unit, and keeps that
///      function's scale.
///   2. A currency with no minor unit books the exact product.
///   3. None exactly when `fx_booked` refuses, or on overflow.
pub fn fx_booked_minor(
    foreign: Decimal,
    rate: Decimal,
    currency: Currency,
    mode: Rounding,
) -> Option<Decimal> {
    let Some(places) = currency.minor_units() else {
        if rate <= Decimal::ZERO {
            return None;
        }
        return Amount::checked_mul(foreign, rate);
    };
    let step = Decimal::try_from_i128_with_scale(1, places).ok()?;
    let rounded = fx_booked(foreign, rate, step, mode)?;
    if rounded.is_zero() {
        // The old round_places scale for a zero (the caller refuses it).
        return Decimal::try_from_i128_with_scale(0, places).ok();
    }
    if rounded.scale() != places {
        // fx_booked returns an exact multiple unchanged, trailing zeros
        // and all; the old computation carried exactly the minor places
        // whenever the product was finer than them. The multiple is
        // exact at `places`, so this rescale rounds nothing.
        let exact = foreign.checked_mul(rate)?;
        if exact.scale() > places {
            let mut rounded = rounded;
            rounded.rescale(places);
            return Some(rounded);
        }
    }
    Some(rounded)
}
