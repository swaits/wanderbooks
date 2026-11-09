//! U5: schedule plans, the monthly charge a schedule books (kernel sweep).
//!
//! `even_parts` is the arithmetic of an equal-months (monthly-even)
//! schedule: the total spread over its months, each month but the last
//! the quotient rounded to the currency's step, the last taking what the
//! others leave. `schedules::amortization_plan` calls it; the date half of
//! a plan (which month end each part books on) stays there.

use super::{Amount, Rounding, plug_last, round_step};

/// Fill `parts` (one slot per month) with the equal-months split of
/// `total`: every part but the last is `total / months` rounded to a whole
/// multiple of `step`, half to even; the last is the total less the
/// others, so the parts sum to `total` exactly. `None` for no months, a
/// step that is not positive, or an amount that cannot be held; `parts`
/// is then unspecified, and the caller discards it.
///
/// ```text
/// GOAL: an equal-months schedule books exactly its total across its
///       months, each month a whole number of the currency's smallest
///       unit, and no month is ever more than a rounding from the rest.
/// SOURCE: BS-prepaid-1 (a schedule recognizes exactly its total over its
///         months, each month rounded to the currency's minor unit), BS-prepaid-2
///         (equal months is one of the two methods an engine picks),
///         BS-prepaid-4 (the unit is the currency's minor unit for a new
///         schedule, the cent for one written before the choice: the
///         caller passes the step, never this function), BS-ledger-1
///         (whole smallest units), BS-entry-3 (exact totals; round only
///         where a rule says to); owner decision D-72; the `plug_last`
///         rule of BS-partner-1 for the remainder.
/// INVARIANTS:
///   1. Some(()) means the parts sum to `total` exactly, and there are as
///      many parts as months.
///   2. Every part but the last is equal: `each = round(total / months)`
///      to `step`, half to even (the schedule rule, frozen by replay), so
///      each is a whole multiple of `step`.
///      A single month has no part but the last: it is the whole total,
///      even one finer than the step (the plug takes it; the cent rule
///      always did).
///   3. `each` is within half a step of the quotient the type's division
///      gives: for `Decimal` that is the exact quotient wherever the
///      division terminates, otherwise the quotient rounded at 28
///      significant digits (`Amount::checked_div`: the K-U10 item,
///      checked against an exact reference in the bridge), and for a
///      machine integer it is the truncated quotient, so the bound against
///      the true quotient is one unit wider. Lean proves the exact-rational
///      law at any size (`Bookspec.Schedule.each_near`).
///   4. The last part is `total - (months - 1) x each`, which is `each`
///      plus the rounding the others left: at most `months x step / 2`
///      from `each` (Lean: `Schedule.last_near`). It is therefore not
///      near the others for a long schedule of a coarse step; it is what
///      makes the sum exact.
///   5. None for no months (`parts` empty), a step <= 0, or overflow;
///      never a panic. The kernel multiplies nothing: the only rounding
///      is `round_step`, which is exact integer arithmetic, and the
///      division's own. The caller keeps `total` below the size Decimal
///      adds exactly (`schedules::EXACT_LIMIT`).
/// SCENARIOS:
///   1. IF 1,000.00 over 3 months at the cent: SHOULD be 333.33, 333.33,
///      333.34
///      BECAUSE the quotient 333.333... rounds down and the last takes
///      the odd cent.
///   2. IF 100,000 yen over 3 months at one yen: SHOULD be 33,333, 33,333,
///      33,334; and 1.000 dinar over 3 at 0.001: 0.333, 0.333, 0.334
///      BECAUSE each month is a whole number of the currency's smallest
///      unit (the cent rule gave 33,333.33 yen and 0.33 dinar).
///   3. IF 0.01 over 3 months at the cent: SHOULD be 0.00, 0.00, 0.01
///      BECAUSE the quotient rounds to nothing and the last takes it all.
///   4. IF there are no months, or the step is zero or negative: SHOULD be
///      None, never a guess.
/// ```
pub fn even_parts<T: Amount>(total: T, step: T, parts: &mut [T]) -> Option<()> {
    let months = u32::try_from(parts.len()).ok().filter(|&m| m > 0)?;
    let quotient = total.checked_div(T::from_u32(months)?)?;
    let each = round_step(quotient, step, Rounding::HalfToEven)?;
    for part in parts.iter_mut() {
        *part = each;
    }
    plug_last(total, parts)
}
