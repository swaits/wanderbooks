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
/// exact `Decimal` product before the single rounding, exactly as
/// `Ledger::add_line_fx` computed it before this kernel existed. At
/// `i64` (the Kani model) amounts and the rate are plain integers in
/// the smallest unit, and the same law is proved of the product.
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
///         docs/spec/claims/ledger.md B-3 states the same pick. The
///         rounding itself is kernel::round_step (BS-ledger-1,
///         BS-entry-3); owner decision D-69 (the laws live once, in
///         bookspec's Lean).
/// INVARIANTS:
///   1. Some(r) is a whole multiple of step: r = n x step for an
///      integer n. None when step <= 0, when rate <= 0, or when the
///      exact product or the rounded result cannot be held.
///   2. In HalfAwayFromZero (the mode add_line_fx books in) r is
///      within half a step of the exact product foreign x rate, an
///      exact half going to the multiple farther from zero.
///   3. Positive foreign and rate give a non-negative r, and a
///      positive r whenever the exact product reaches half a step
///      (a smaller product rounds to zero, which the caller refuses
///      as it does today). The sign never crosses zero.
///   4. On Decimal the value equals the pre-kernel computation
///      (checked_mul, then round_minor half away from zero) exactly;
///      fx_booked_minor also keeps its scale.
/// SCENARIOS:
///   1. IF 100.00 at 1.0834567 to the cent: SHOULD book 108.35.
///   2. IF 10.00 at 151.237 to the yen (step 1): SHOULD book 1512.
///   3. IF the rate is zero or negative: SHOULD be None, never a guess.
///   4. IF foreign x rate overflows: SHOULD be None, never a panic.
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
/// has always done (B-3). The result's scale is the pre-kernel
/// computation's: `round_places` keeps an amount already at or under
/// the minor places unchanged, and otherwise carries exactly the minor
/// places (a booked 1.50 prints "1.50", never "1.5000").
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
