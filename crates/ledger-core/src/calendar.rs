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
use crate::kernel::MonthPart;
use crate::settings::Company;
use chrono::{Datelike, Months, NaiveDate};

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

/// The month and day a fiscal year starts on, as one every year has: the
/// books' `company.fiscal_year_start` always is (a new value is the first
/// of a month; a journaled one is a day 2001 has, and 2001 is no leap
/// year). Any other counts as January 1.
fn year_start(md: (u32, u32)) -> (u32, u32) {
    if NaiveDate::from_ymd_opt(2001, md.0, md.1).is_some() {
        md
    } else {
        (1, 1)
    }
}

/// The first day of the fiscal year that holds `d` (a fiscal year
/// starting on month-day `md`), before any clamp: in year 0 for a day
/// early in year 1, which chrono holds and the books' calendar doesn't.
pub(crate) fn fiscal_year_start_unclamped(md: (u32, u32), d: NaiveDate) -> NaiveDate {
    let (m, day) = year_start(md);
    // A day every year has (year_start), in d's year or the one before:
    // chrono holds both for every date it holds but its very first year,
    // where the year before falls back to d itself.
    let on = |y: i32| NaiveDate::from_ymd_opt(y, m, day).unwrap_or(d);
    let this = on(d.year());
    if this <= d { this } else { on(d.year() - 1) }
}

/// The fiscal year that holds `d`, `(first day, last day)`, unclamped
/// (`Company::fiscal_year_containing`): the last day is the day before
/// the next start, so the two can't disagree.
pub(crate) fn fiscal_year_unclamped(md: (u32, u32), d: NaiveDate) -> (NaiveDate, NaiveDate) {
    let start = fiscal_year_start_unclamped(md, d);
    let (m, day) = year_start(md);
    let end = NaiveDate::from_ymd_opt(start.year() + 1, m, day)
        .and_then(|next| next.pred_opt())
        .unwrap_or(NaiveDate::MAX);
    (start, end)
}

/// The first day of the fiscal year that holds `d`, for a fiscal year
/// starting on month-day `md` (`company.fiscal_year_start`), clamped to
/// [`FIRST_DAY`].
///
/// ```text
/// GOAL: the books' annual period: the one fact the fiscal year to date,
///       the CPA package's year, and every default period derive from.
/// SOURCE: IRS Pub. 538 (docs/spec/sources/irs-periods-and-methods.md):
///         "A tax year is an annual accounting period for keeping records
///         and reporting income and expenses"; rules.md B-28 (no date
///         outside the calendar).
/// INVARIANTS:
///   1. For d in the calendar: FIRST_DAY <= result <= d, and d is before
///      the same month-day a year after the result (unless clamped).
///   2. The result is md in d's year when that is on or before d, else md
///      in the year before; clamped to FIRST_DAY only when that is year
///      0 (a non-January fiscal year, d early in year 1).
///   3. NEVER panics, for any md and any chrono date.
/// ```
pub fn fiscal_year_start_on(md: (u32, u32), d: NaiveDate) -> NaiveDate {
    fiscal_year_start_unclamped(md, d).max(FIRST_DAY)
}

/// What an omitted date means, by the kind of report (rules.md B-30): the
/// text every tool description and date parameter shows, so none can say
/// other than [`PeriodDefaults`] does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PeriodKind {
    /// A report over a period (`start`, `end`): profit and loss, cash
    /// flow, tax summary, members' equity, the general ledger and
    /// journal, an account's register.
    Statement,
    /// A report at an instant (`as_of`): balance sheet, trial balance,
    /// aging, the fixed asset register, the CPA package.
    AsOf,
    /// A count by calendar year (`year`): 1099s and party totals.
    CalendarYear,
}

impl PeriodKind {
    /// The one sentence a tool's description gives for its dates.
    pub const fn describe(self) -> &'static str {
        match self {
            PeriodKind::Statement => {
                "Dates are YYYY-MM-DD. Omitted, the period is the fiscal year to date (company.fiscal_year_start): end defaults to today, start to the first day of the fiscal year that holds end. A start after the end is refused."
            }
            PeriodKind::AsOf => "as_of is YYYY-MM-DD; omitted, today in the company's time zone.",
            PeriodKind::CalendarYear => {
                "year defaults to this calendar year (1099s and party totals count by calendar year)."
            }
        }
    }
}

/// A period report's `start` parameter, as every schema documents it.
pub const START_DOC: &str = "First day, YYYY-MM-DD. Omitted: the first day of the fiscal year that holds end (company.fiscal_year_start). A start after the end is refused.";
/// A period report's `end` parameter, as every schema documents it.
pub const END_DOC: &str = "Last day, YYYY-MM-DD. Omitted: today in the company's time zone.";
/// A point-in-time report's `as_of` parameter, as every schema documents
/// it.
pub const AS_OF_DOC: &str =
    "As of this day, YYYY-MM-DD. Omitted: today in the company's time zone.";
/// A calendar-year count's `year` parameter, as every schema documents it.
pub const YEAR_DOC: &str =
    "Calendar year. Omitted: this calendar year (1099s and party totals count by calendar year).";

/// The defaults an omitted date takes, today, under the company's fiscal
/// year: the one rule (rules.md B-30) the report tools, the viewer and
/// the tool descriptions share. Built by [`period_defaults`].
///
/// ```text
/// GOAL: one meaning for "no dates", everywhere: a report asked for
///       without dates covers this fiscal year so far.
/// SOURCE: rules.md B-30; IRS Pub. 538 ("an annual accounting period for
///         keeping records and reporting income and expenses"); 26 U.S.C.
///         §6041(a) ("in any calendar year") and §6041A(a)(2) ("during
///         such calendar year") for the calendar-year counts; DECISION
///         (owner, 2026-10-20, plan/served.md P3 "Settled in design"):
///         the fiscal year to date, everywhere.
/// INVARIANTS:
///   1. statement Ok implies start <= end, both in the calendar.
///   2. An omitted end is today; an omitted as_of is today.
///   3. An omitted start is the first day of the fiscal year that holds
///      the resolved end, clamped to FIRST_DAY.
///   4. Never swapped or clamped to fit: Err iff the resolved start is
///      after the resolved end.
///   5. statement_range() == statement(None, None).
/// SCENARIOS:
///   1. IF both dates are given and start > end: SHOULD refuse with
///      InvalidDateRange.
///   2. IF only start is given and it is after today: SHOULD refuse with
///      StartAfterToday, saying the end defaulted to today.
///   3. IF a given date is outside the calendar: SHOULD refuse with
///      DateOutOfRange.
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PeriodDefaults {
    fiscal_year_start: (u32, u32),
    today: NaiveDate,
}

/// The defaults an omitted date takes for `company`, on `today` (in the
/// company's time zone).
pub fn period_defaults(company: &Company, today: NaiveDate) -> PeriodDefaults {
    PeriodDefaults {
        fiscal_year_start: company.fiscal_year_start,
        today,
    }
}

impl PeriodDefaults {
    /// The day the defaults are for.
    pub fn today(&self) -> NaiveDate {
        self.today
    }

    /// A period report with both dates omitted: the first day of the
    /// fiscal year that holds today, through today (INVARIANT 5).
    pub fn statement_range(&self) -> (NaiveDate, NaiveDate) {
        (
            fiscal_year_start_on(self.fiscal_year_start, self.today),
            self.today,
        )
    }

    /// A period report's dates as given (`None`: omitted), resolved
    /// (INVARIANTS 1-4).
    pub fn statement(
        &self,
        start: Option<NaiveDate>,
        end: Option<NaiveDate>,
    ) -> Result<(NaiveDate, NaiveDate), LedgerError> {
        let end_given = end.map(check).transpose()?;
        let start_given = start.map(check).transpose()?;
        let end = end_given.unwrap_or(self.today);
        let start =
            start_given.unwrap_or_else(|| fiscal_year_start_on(self.fiscal_year_start, end));
        if start > end {
            return Err(if end_given.is_some() {
                LedgerError::InvalidDateRange {
                    start: start.to_string(),
                    end: end.to_string(),
                }
            } else {
                LedgerError::StartAfterToday {
                    start: start.to_string(),
                    today: end.to_string(),
                }
            });
        }
        Ok((start, end))
    }

    /// A point-in-time report's day: the one given, else today.
    pub fn as_of(&self, given: Option<NaiveDate>) -> NaiveDate {
        given.unwrap_or(self.today)
    }

    /// A calendar-year count's year when none is given: today's.
    pub fn calendar_year(&self) -> i32 {
        self.today.year()
    }
}

/// The days in a month of a year chrono holds, without stepping to the
/// next month, so chrono's last month (December 262142, whose next month
/// chrono can't hold) has its 31 days too (rules.md B-28: a schedule an
/// earlier version accepted is computed, never a panic).
pub fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        2 if NaiveDate::from_ymd_opt(year, 2, 29).is_some() => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// The first day of the month after `month_start`'s, or `None` past
/// chrono's last day (the month containing NaiveDate::MAX).
pub fn next_month_start(month_start: NaiveDate) -> Option<NaiveDate> {
    month_start.checked_add_months(Months::new(1))
}

/// One calendar month's share of a span of days ([`month_parts`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SpanPart {
    /// The month's year.
    pub year: i32,
    /// The month, 1 to 12.
    pub month: u32,
    /// The span's first day in the month.
    pub first: NaiveDate,
    /// Its days in the month, of the month's days.
    pub part: MonthPart,
    /// True for the span's last month (the one holding its last day).
    pub last: bool,
}

/// The calendar months of the span `[start, end)`, in order, each with the
/// span's days in it: lazily, so a walk through a date reads only the
/// months up to it.
///
/// ```text
/// GOAL: cut a span of days (an asset's life, a schedule's coverage) into
///       the calendar months it touches, for proration by days.
/// SOURCE: docs/spec/rules.md B-15, B-16; kernel::prorate_by_days.
/// INVARIANTS:
///   1. The parts' days sum to the span's days; every part has 1 to its
///      month's days (a valid kernel::MonthPart), and only the last is
///      marked last.
///   2. NEVER steps past chrono's last day: December 262142, whose next
///      month chrono can't hold, still has its 31 days, and its part ends
///      at `end`.
///   3. An empty span (end <= start) has no parts.
/// ```
pub fn month_parts(start: NaiveDate, end: NaiveDate) -> impl Iterator<Item = SpanPart> {
    let mut at = (start < end).then_some(start);
    std::iter::from_fn(move || {
        let first = at?;
        let month_start = first.with_day(1)?;
        let next = next_month_start(month_start);
        // The span ends in this month when its end is no later than the
        // next month's first day (or chrono has no next month).
        let last = next.is_none_or(|n| end <= n);
        let part_end = if last { end } else { next? };
        let days = u32::try_from((part_end - first).num_days()).ok()?;
        at = if last { None } else { next };
        Some(SpanPart {
            year: first.year(),
            month: first.month(),
            first,
            part: MonthPart {
                days,
                month_days: days_in_month(first.year(), first.month()),
            },
            last,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    /// INVARIANT 1, on a span from mid-month with a short final month.
    #[test]
    fn a_span_cuts_into_its_calendar_months() {
        let parts: Vec<_> = month_parts(d(2026, 1, 15), d(2026, 4, 15)).collect();
        let got: Vec<_> = parts
            .iter()
            .map(|p| (p.year, p.month, p.part.days, p.part.month_days, p.last))
            .collect();
        assert_eq!(
            got,
            [
                (2026, 1, 17, 31, false),
                (2026, 2, 28, 28, false),
                (2026, 3, 31, 31, false),
                (2026, 4, 14, 30, true),
            ]
        );
        assert_eq!(parts[0].first, d(2026, 1, 15));
        assert_eq!(parts[3].first, d(2026, 4, 1));
        // A span from the 1st: whole months, the last one marked.
        let whole: Vec<_> = month_parts(d(2028, 1, 1), d(2028, 3, 1))
            .map(|p| (p.part.days, p.part.month_days, p.last))
            .collect();
        assert_eq!(whole, [(31, 31, false), (29, 29, true)]);
        // INVARIANT 3.
        assert_eq!(month_parts(d(2026, 1, 1), d(2026, 1, 1)).count(), 0);
        assert_eq!(month_parts(d(2026, 1, 2), d(2026, 1, 1)).count(), 0);
    }

    /// INVARIANT 2: chrono's last month is walked to the span's end.
    #[test]
    fn the_walk_reaches_chronos_last_month() {
        let y = NaiveDate::MAX.year();
        let parts: Vec<_> = month_parts(d(y, 11, 20), d(y, 12, 15)).collect();
        assert_eq!(parts.len(), 2);
        assert_eq!(
            parts[1].part,
            MonthPart {
                days: 14,
                month_days: 31
            }
        );
        assert!(parts[1].last);
        let to_max: Vec<_> = month_parts(d(y, 12, 1), NaiveDate::MAX).collect();
        assert_eq!(to_max.len(), 1);
        assert_eq!(
            to_max[0].part,
            MonthPart {
                days: 30,
                month_days: 31
            }
        );
        assert_eq!(days_in_month(y, 12), 31);
        assert_eq!(next_month_start(d(y, 12, 1)), None);
    }

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

    fn company(md: (u32, u32)) -> Company {
        Company {
            fiscal_year_start: md,
            ..Company::default()
        }
    }

    /// fiscal_year_start_on INVARIANT 2: md in d's year when on or before
    /// d, else the year before; for a calendar year, a July year and a
    /// December year, on the start day and the day before it.
    #[test]
    fn the_fiscal_year_start_is_the_last_start_on_or_before() {
        for (md, day, want) in [
            ((1, 1), d(2026, 10, 20), d(2026, 1, 1)),
            ((1, 1), d(2026, 1, 1), d(2026, 1, 1)),
            ((1, 1), d(2025, 12, 31), d(2025, 1, 1)),
            ((7, 1), d(2026, 10, 20), d(2026, 7, 1)),
            ((7, 1), d(2026, 7, 1), d(2026, 7, 1)),
            ((7, 1), d(2026, 6, 30), d(2025, 7, 1)),
            ((7, 1), d(2026, 2, 28), d(2025, 7, 1)),
            ((12, 1), d(2026, 11, 30), d(2025, 12, 1)),
            ((12, 1), d(2026, 12, 1), d(2026, 12, 1)),
            ((12, 1), d(2026, 12, 31), d(2026, 12, 1)),
            // A journaled start that isn't the first (replay is lenient).
            ((7, 15), d(2026, 7, 14), d(2025, 7, 15)),
            ((7, 15), d(2026, 7, 15), d(2026, 7, 15)),
            // The calendar's last day.
            ((7, 1), LAST_DAY, d(9999, 7, 1)),
            ((1, 1), LAST_DAY, d(9999, 1, 1)),
        ] {
            assert_eq!(fiscal_year_start_on(md, day), want, "{md:?} {day}");
        }
    }

    /// INVARIANT 2's clamp: a July year holding a day early in year 1
    /// began in year 0, which the books' calendar doesn't keep.
    #[test]
    fn a_fiscal_year_from_year_zero_starts_on_the_first_day() {
        assert_eq!(fiscal_year_start_unclamped((7, 1), d(1, 3, 1)), d(0, 7, 1));
        assert_eq!(fiscal_year_start_on((7, 1), d(1, 3, 1)), FIRST_DAY);
        assert_eq!(fiscal_year_start_on((7, 1), d(1, 7, 1)), d(1, 7, 1));
        assert_eq!(fiscal_year_start_on((1, 1), FIRST_DAY), FIRST_DAY);
    }

    /// INVARIANT 3: no month-day and no chrono date panics; a month-day
    /// no year has counts as January 1.
    #[test]
    fn no_month_day_or_date_panics() {
        assert_eq!(fiscal_year_start_on((2, 30), d(2026, 5, 5)), d(2026, 1, 1));
        assert_eq!(fiscal_year_start_on((13, 1), d(2026, 5, 5)), d(2026, 1, 1));
        assert_eq!(fiscal_year_start_on((0, 0), d(2026, 5, 5)), d(2026, 1, 1));
        for md in [(1, 1), (7, 1), (12, 31), (2, 28)] {
            for day in [NaiveDate::MIN, NaiveDate::MAX, FIRST_DAY, LAST_DAY] {
                let _ = fiscal_year_start_on(md, day);
                let _ = fiscal_year_start_unclamped(md, day);
            }
        }
    }

    /// PeriodDefaults INVARIANTS 2, 3, 5: omitted dates.
    #[test]
    fn omitted_dates_are_the_fiscal_year_to_date() {
        let today = d(2026, 10, 20);
        let p = period_defaults(&company((1, 1)), today);
        assert_eq!(p.statement_range(), (d(2026, 1, 1), today));
        assert_eq!(p.statement(None, None), Ok(p.statement_range()));
        // Only end: that end's fiscal year, to it.
        assert_eq!(
            p.statement(None, Some(d(2025, 6, 30))),
            Ok((d(2025, 1, 1), d(2025, 6, 30)))
        );
        // Only start: through today.
        assert_eq!(
            p.statement(Some(d(2024, 3, 15)), None),
            Ok((d(2024, 3, 15), today))
        );
        let july = period_defaults(&company((7, 1)), today);
        assert_eq!(july.statement_range(), (d(2026, 7, 1), today));
        assert_eq!(
            july.statement(None, Some(d(2026, 6, 30))),
            Ok((d(2025, 7, 1), d(2026, 6, 30)))
        );
        assert_eq!(
            july.statement(None, Some(d(1, 3, 1))),
            Ok((FIRST_DAY, d(1, 3, 1)))
        );
        assert_eq!(
            july.statement(None, Some(LAST_DAY)),
            Ok((d(9999, 7, 1), LAST_DAY))
        );
        assert_eq!(p.as_of(None), today);
        assert_eq!(p.as_of(Some(d(2025, 12, 31))), d(2025, 12, 31));
        assert_eq!(p.calendar_year(), 2026);
        assert_eq!(july.calendar_year(), 2026);
        assert_eq!(p.today(), today);
    }

    /// PeriodDefaults INVARIANT 4 and SCENARIOS 1-3: never swapped.
    #[test]
    fn a_start_after_the_end_is_refused_never_swapped() {
        let today = d(2026, 10, 20);
        let p = period_defaults(&company((1, 1)), today);
        // One day long is a period.
        assert_eq!(p.statement(Some(today), Some(today)), Ok((today, today)));
        assert_eq!(
            p.statement(Some(d(2026, 1, 2)), Some(d(2026, 1, 1))),
            Err(LedgerError::InvalidDateRange {
                start: "2026-01-02".into(),
                end: "2026-01-01".into()
            })
        );
        assert_eq!(
            p.statement(Some(d(2026, 12, 31)), Some(d(2026, 1, 1))),
            Err(LedgerError::InvalidDateRange {
                start: "2026-12-31".into(),
                end: "2026-01-01".into()
            })
        );
        // The end defaulted: the message says so.
        let e = p.statement(Some(d(2027, 1, 1)), None).unwrap_err();
        assert_eq!(
            e,
            LedgerError::StartAfterToday {
                start: "2027-01-01".into(),
                today: "2026-10-20".into()
            }
        );
        assert!(e.to_string().contains("today"), "{e}");
        assert!(e.to_string().contains("pass end"), "{e}");
        assert_eq!(
            p.statement(Some(d(2026, 10, 21)), None).map(|_| ()),
            Err(LedgerError::StartAfterToday {
                start: "2026-10-21".into(),
                today: "2026-10-20".into()
            })
        );
        assert_eq!(p.statement(Some(today), None), Ok((today, today)));
        // Outside the calendar.
        assert_eq!(
            p.statement(None, Some(NaiveDate::MAX)),
            Err(LedgerError::DateOutOfRange(NaiveDate::MAX))
        );
        assert_eq!(
            p.statement(Some(NaiveDate::MIN), Some(today)),
            Err(LedgerError::DateOutOfRange(NaiveDate::MIN))
        );
    }

    /// The descriptions say what the resolver does (B-30), in the words
    /// every tool shows.
    #[test]
    fn the_descriptions_name_the_rule() {
        let s = PeriodKind::Statement.describe();
        assert!(s.contains("fiscal year to date"));
        assert!(s.contains("end defaults to today"));
        assert!(s.contains("A start after the end is refused"));
        assert!(PeriodKind::AsOf.describe().contains("today"));
        assert!(
            PeriodKind::CalendarYear
                .describe()
                .contains("calendar year")
        );
        for text in [
            s,
            PeriodKind::AsOf.describe(),
            PeriodKind::CalendarYear.describe(),
            START_DOC,
            END_DOC,
            AS_OF_DOC,
            YEAR_DOC,
        ] {
            for stale in ["all time", "beginning of time", "first entry"] {
                assert!(!text.contains(stale), "{text}");
            }
        }
    }
}
