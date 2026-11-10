//! The one rounding for presentation: an amount rounded for display,
//! never for the books. Every renderer that shortens an amount to a
//! fixed number of places goes through [`round_display`], so a display
//! change never changes a booked figure and a booked figure never
//! changes because of a display change.
//!
//! ```text
//! GOAL: one definition of display rounding for every renderer.
//! SOURCE: CONVENTION (US accounting presentation: an amount shown to a
//!         fixed number of places rounds half away from zero, as
//!         crate::report::format_amount has always done).
//! INVARIANTS:
//!   1. round_display(amount, places) is amount rounded to `places`
//!      decimal places, half away from zero, through the kernel's one
//!      rounding primitive (ledger_core::kernel::round_places).
//!   2. Never a negative zero: a result that is zero is "0.00", never
//!      "-0.00" and never "(0.00)".
//!   3. Display only: the result is for printing. Callers never book it.
//! SCENARIOS:
//!   1. IF 1.005 to two places: SHOULD be 1.01 (and -1.005, -1.01).
//!   2. IF -0.004 to two places: SHOULD be 0.00, never a negative zero.
//!   3. IF 2.5 to zero places: SHOULD be 3 (and -2.5, -3).
//! ```

use ledger_core::kernel::Rounding;
use rust_decimal::Decimal;

/// `amount` rounded to `places` decimal places for display: half away
/// from zero, never a negative zero. See the module spec.
pub fn round_display(amount: Decimal, places: u32) -> Decimal {
    match ledger_core::kernel::round_places(amount, places, Rounding::HalfAwayFromZero) {
        Some(r) => r,
        // Rounding to fewer places cannot grow an amount past Decimal's
        // range in practice; past exact arithmetic, show the amount
        // itself (with no negative zero) rather than a wrong figure.
        None if amount.is_zero() => Decimal::ZERO,
        None => amount,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn dec(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap()
    }

    #[test]
    fn half_away_from_zero() {
        assert_eq!(round_display(dec("1.005"), 2), dec("1.01"));
        assert_eq!(round_display(dec("-1.005"), 2), dec("-1.01"));
        assert_eq!(round_display(dec("2.5"), 0), dec("3"));
        assert_eq!(round_display(dec("-2.5"), 0), dec("-3"));
        assert_eq!(round_display(dec("1.004"), 2), dec("1.00"));
        assert_eq!(round_display(dec("1234567.891"), 2), dec("1234567.89"));
    }

    #[test]
    fn never_a_negative_zero() {
        for s in ["-0.004", "-0.00", "-0", "0"] {
            let r = round_display(dec(s), 2);
            assert_eq!(r, Decimal::ZERO, "{s}");
            assert!(!r.is_sign_negative(), "{s}");
        }
        assert_eq!(round_display(dec("-0.004"), 2).to_string(), "0.00");
        assert_eq!(round_display(dec("-0.4"), 0).to_string(), "0");
    }
}
