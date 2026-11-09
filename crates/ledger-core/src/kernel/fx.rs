//! U1: foreign-currency conversion, the booking of a foreign amount at
//! the transaction-date rate (kernel sweep, `plan/kernel-sweep-audit.md`
//! section 3). The one place foreign x rate becomes booked money: the
//! product is computed exactly and rounded once, by [`round_step`], to
//! the base currency's smallest unit. No call site multiplies by a rate
//! itself.

use super::round::{Rounding, round_step};
use crate::Currency;
use crate::kernel::Amount;
use rust_decimal::Decimal;

/// The booked base amount for a foreign amount at a rate: `foreign x
/// rate` computed exactly, rounded once to a whole multiple of `step`
/// (the base currency's minor unit) in `mode`.
///
/// The rate's precision is carried by the value itself: a `Decimal`
/// rate keeps its own scale (1.0834567 is exact), and the product is the
/// `T::checked_mul` product before the single rounding. At `i64` (the
/// Kani model) amounts and the rate are plain integers in the smallest
/// unit, and the same law is proved of the product.
///
/// **The precondition that makes the product exact (D-75).** On
/// `Decimal`, `checked_mul` is exact only while the product keeps its
/// full scale: that scale (the two scales added) must be within 28, and
/// the product's mantissa (the two mantissas multiplied) must fit
/// `Decimal`'s 96 bits, under 7.9e28. A foreign amount and a rate of at
/// most [`MAX_LINE_SCALE`](super::MAX_LINE_SCALE) (10) places each,
/// trailing zeros not counted, meet the first half: a product of at most
/// 20 places, under 28. `Ledger::add_line_fx` enforces that bound on
/// every new line (never on replay). The second half is not enforced:
/// a product of 20 places past about 7.9e8 needs more than 96 bits, and
/// `Decimal` then drops places (see below). Under both halves the
/// product is the exact one and the result is rounded exactly once.
/// Outside them `Decimal::checked_mul` keeps the behaviours it has
/// always had, which this function inherits and does not hide: a
/// product that does not fit is rounded by `Decimal` (to 28 places, or
/// to the places 96 bits hold) before this function rounds it (a second
/// rounding, so a result can sit a unit off the exact product's:
/// 0.99999999999999999999999999 x 0.005 is 0.00499999999999999999999999995
/// exactly but 0.005 as held, and books 0.01; 1050000010.0000000001 x
/// 9.9999999999 is 10500000099.89 to the cent but 10500000099.895 as
/// held, and books 10500000099.90), and a product below 1e-28 (1e-28 x
/// 1e-28) underflows to zero. Invariants 2 and 3 are stated for
/// products that are exact; invariant 1 (a whole multiple of the step)
/// holds regardless. The bridge pins both behaviours.
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
///      integer n. None when step <= 0, when rate <= 0, or when T's
///      checked_mul cannot hold the product (the exact product, under
///      the precondition above) or the rounded result cannot be held.
///   2. Under the precondition (both halves: the product is exact), in
///      HalfAwayFromZero (the mode add_line_fx books in) r is within
///      half a step of the exact product foreign x rate, an exact half
///      going to the multiple farther from zero.
///   3. Under the precondition, positive foreign and rate give a
///      non-negative r, and a positive r whenever the exact product
///      reaches half a step (a smaller product rounds to zero, which
///      the caller refuses as it does today). The sign never crosses
///      zero.
///   4. On Decimal the value equals the pre-kernel computation
///      (checked_mul, then round_minor half away from zero) exactly;
///      fx_booked_minor also keeps its scale.
/// SCENARIOS:
///   1. IF 100.00 at 1.0834567 to the cent: SHOULD book 108.35.
///   2. IF 10.00 at 151.237 to the yen (step 1): SHOULD book 1512.
///   3. IF the rate is zero or negative: SHOULD be None, never a guess.
///   4. IF foreign x rate overflows: SHOULD be None, never a panic.
///   5. IF foreign is 0.99999999999999999999999999 (26 places) and
///      the rate 0.005 to the cent: outside the precondition, books
///      0.01 where the exact product is under half a cent BECAUSE
///      Decimal rounded the product first; the engine refuses such an
///      input (D-75), and the bridge pins this behaviour.
///   6. IF foreign 1050000010.0000000001 at 9.9999999999 to the cent
///      (ten places each, a product past 96 bits): outside the
///      precondition's second half, books 10500000099.90 where the
///      exact product rounds to 10500000099.89; the bridge pins this
///      open edge.
/// ```
pub fn fx_booked<T: Amount>(foreign: T, rate: T, step: T, mode: Rounding) -> Option<T> {
    if rate <= T::ZERO {
        return None;
    }
    let exact = foreign.checked_mul(rate)?;
    round_step(exact, step, mode)
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
        return foreign.checked_mul(rate);
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
