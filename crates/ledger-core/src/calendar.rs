//! The calendar the books keep: four-digit years, 0001-01-01 through
//! 9999-12-31.
//!
//! ```text
//! GOAL: no date the books hold, or derive (a schedule's last month, an
//!       asset's last day of depreciation, a fiscal year's end), is one
//!       date arithmetic can't reach, or one a report can't print.
//! SOURCE: CONVENTION: ISO 8601's basic calendar dates have four-digit
//!         years (0000-9999; year 0 is no Gregorian year, so 0001 is the
//!         first), as do the spreadsheet and PDF formats the books export
//!         to; DECISION (P0 review round 3, 2026-10-16, finding S1): a date
//!         outside them is refused, never a crash.
//! INVARIANTS:
//!   1. in_calendar(d) iff FIRST_DAY <= d <= LAST_DAY.
//!   2. Every date from the calendar plus a year or 600 months less than
//!      ten thousand years is a date chrono represents (its range is about
//!      +-262,000 years), so arithmetic on calendar dates never overflows.
//! ```

use crate::LedgerError;
use chrono::NaiveDate;

/// The first day the books keep.
pub const FIRST_DAY: NaiveDate = match NaiveDate::from_ymd_opt(1, 1, 1) {
    Some(d) => d,
    None => panic!("0001-01-01 is a date"),
};

/// The last day the books keep.
pub const LAST_DAY: NaiveDate = match NaiveDate::from_ymd_opt(9999, 12, 31) {
    Some(d) => d,
    None => panic!("9999-12-31 is a date"),
};

/// Whether `d` is in the calendar the books keep (INVARIANT 1).
pub fn in_calendar(d: NaiveDate) -> bool {
    (FIRST_DAY..=LAST_DAY).contains(&d)
}

/// `d`, if it is in the calendar; else [`LedgerError::DateOutOfRange`].
pub fn check(d: NaiveDate) -> Result<NaiveDate, LedgerError> {
    if in_calendar(d) {
        Ok(d)
    } else {
        Err(LedgerError::DateOutOfRange(d))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_calendar_is_four_digit_years() {
        assert!(in_calendar(FIRST_DAY));
        assert!(in_calendar(LAST_DAY));
        assert!(!in_calendar(FIRST_DAY.pred_opt().unwrap()));
        assert!(!in_calendar(LAST_DAY.succ_opt().unwrap()));
        assert!(!in_calendar(NaiveDate::MAX));
        assert!(!in_calendar(NaiveDate::MIN));
        assert_eq!(FIRST_DAY.to_string(), "0001-01-01");
        assert_eq!(LAST_DAY.to_string(), "9999-12-31");
        assert_eq!(check(LAST_DAY), Ok(LAST_DAY));
        assert_eq!(
            check(NaiveDate::MAX),
            Err(LedgerError::DateOutOfRange(NaiveDate::MAX))
        );
    }

    /// INVARIANT 2: the arithmetic the books do on calendar dates (a year
    /// on, 600 months on, a day back) stays inside chrono's range.
    #[test]
    fn arithmetic_on_calendar_dates_never_overflows() {
        for d in [FIRST_DAY, LAST_DAY] {
            assert!(d.checked_add_months(chrono::Months::new(600)).is_some());
            assert!(d.checked_sub_months(chrono::Months::new(600)).is_some());
            assert!(NaiveDate::from_ymd_opt(chrono::Datelike::year(&d) + 1, 1, 1).is_some());
            assert!(NaiveDate::from_ymd_opt(chrono::Datelike::year(&d) - 1, 12, 31).is_some());
            assert!(d.pred_opt().is_some() && d.succ_opt().is_some());
        }
    }
}
