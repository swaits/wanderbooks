//! Accounting periods and period close.
//!
//! A [`Period`] is a named date range (a month, a quarter, a year). Periods
//! start open; closing one is a one-way operation that:
//!
//! 1. Rejects the close if any draft entries have dates inside the period.
//! 2. Posts a closing entry dated at period end: revenue accounts are
//!    debited to zero, expense accounts are credited to zero, and the net
//!    difference lands in Retained Earnings (created as `3999` if missing).
//! 3. Marks the period closed.
//!
//! Once closed, [`Ledger::create_entry`] and [`Ledger::post_entry`] reject
//! any entry dated inside the period. Corrections to closed periods go
//! through the *next* open period (or a reversal dated in an open period).

use crate::LedgerError;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

/// Opaque period handle. Created by [`crate::Ledger`]; never forged.
pub type PeriodId = u64;

/// Lifecycle of an accounting period. `Open -> Closed` is one-way.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum PeriodStatus {
    /// Accepting entries.
    Open,
    /// Closed. Entries dated inside are rejected.
    Closed,
}

/// A named accounting period: a month, quarter, or year.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Period {
    /// Opaque handle.
    pub id: PeriodId,
    /// Human name, e.g. `"2026-10"` or `"Q4 2026"`.
    pub name: String,
    /// First date in the period (inclusive).
    pub start: NaiveDate,
    /// Last date in the period (inclusive).
    pub end: NaiveDate,
    /// Open or closed.
    pub status: PeriodStatus,
    /// The closing entry's id, once closed.
    pub closing_entry: Option<u64>,
}

impl Period {
    /// True when `date` falls inside the period (inclusive).
    pub fn contains(&self, date: NaiveDate) -> bool {
        date >= self.start && date <= self.end
    }
}

/// Validate a period's date range before creation.
pub fn validate_period_range(start: NaiveDate, end: NaiveDate) -> Result<(), LedgerError> {
    if end < start {
        return Err(LedgerError::InvalidPeriodRange {
            start: start.to_string(),
            end: end.to_string(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Currency, Ledger};

    fn period_ledger() -> Ledger {
        let mut l = Ledger::new(Currency::USD);
        l.add_account("4000", "Revenue", crate::AccountType::Revenue, None, false)
            .unwrap();
        l.add_account("6000", "Expenses", crate::AccountType::Expense, None, false)
            .unwrap();
        l.add_account("1000", "Cash", crate::AccountType::Asset, None, false)
            .unwrap();
        l
    }

    fn post_sale(l: &mut Ledger, date: NaiveDate, amount: &str) {
        let e = l.create_entry(date, "sale");
        let rev = l.chart().find_by_code("4000").unwrap().id;
        let cash = l.chart().find_by_code("1000").unwrap().id;
        l.add_line(
            e,
            cash,
            crate::Side::Debit,
            crate::Money::new(amount.parse().unwrap(), Currency::USD),
        )
        .unwrap();
        l.add_line(
            e,
            rev,
            crate::Side::Credit,
            crate::Money::new(amount.parse().unwrap(), Currency::USD),
        )
        .unwrap();
        l.post_entry(e).unwrap();
    }

    #[test]
    fn close_zeroes_revenue_and_expense_to_retained_earnings() {
        let mut l = period_ledger();
        let start = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2026, 10, 31).unwrap();
        let pid = l.create_period("2026-10", start, end).unwrap();

        post_sale(
            &mut l,
            NaiveDate::from_ymd_opt(2026, 10, 15).unwrap(),
            "1000.00",
        );
        // An expense: debit expense, credit cash.
        let e = l.create_entry(NaiveDate::from_ymd_opt(2026, 10, 20).unwrap(), "rent");
        let exp = l.chart().find_by_code("6000").unwrap().id;
        let cash = l.chart().find_by_code("1000").unwrap().id;
        let m = crate::Money::new("400.00".parse().unwrap(), Currency::USD);
        l.add_line(e, exp, crate::Side::Debit, m).unwrap();
        l.add_line(e, cash, crate::Side::Credit, m).unwrap();
        l.post_entry(e).unwrap();

        l.close_period(pid).unwrap();

        // Revenue and expense are zeroed.
        let rev = l.chart().find_by_code("4000").unwrap().id;
        let exp = l.chart().find_by_code("6000").unwrap().id;
        assert_eq!(l.account_balance(rev).unwrap(), rust_decimal::Decimal::ZERO);
        assert_eq!(l.account_balance(exp).unwrap(), rust_decimal::Decimal::ZERO);
        // Retained earnings holds the net: 1000 - 400 = 600 (credit-normal).
        let re = l.chart().find_by_code("3999").unwrap().id;
        assert_eq!(
            l.account_balance(re).unwrap(),
            "600".parse::<rust_decimal::Decimal>().unwrap()
        );
        // Period is closed.
        assert_eq!(l.get_period(pid).unwrap().status, PeriodStatus::Closed);
    }

    #[test]
    fn close_rejects_when_already_closed() {
        let mut l = period_ledger();
        let pid = l
            .create_period(
                "2026-10",
                NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
                NaiveDate::from_ymd_opt(2026, 10, 31).unwrap(),
            )
            .unwrap();
        l.close_period(pid).unwrap();
        assert_eq!(
            l.close_period(pid).unwrap_err(),
            LedgerError::PeriodAlreadyClosed(pid)
        );
    }

    #[test]
    fn close_rejects_with_drafts_in_period() {
        let mut l = period_ledger();
        let pid = l
            .create_period(
                "2026-10",
                NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
                NaiveDate::from_ymd_opt(2026, 10, 31).unwrap(),
            )
            .unwrap();
        // A draft dated inside the period blocks the close.
        l.create_entry(NaiveDate::from_ymd_opt(2026, 10, 15).unwrap(), "draft");
        assert_eq!(
            l.close_period(pid).unwrap_err(),
            LedgerError::DraftsInPeriod(pid)
        );
    }

    #[test]
    fn closed_period_rejects_new_entries() {
        let mut l = period_ledger();
        let pid = l
            .create_period(
                "2026-10",
                NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
                NaiveDate::from_ymd_opt(2026, 10, 31).unwrap(),
            )
            .unwrap();
        l.close_period(pid).unwrap();
        // create_entry with a date in the closed period fails.
        let err = l.try_create_entry(NaiveDate::from_ymd_opt(2026, 10, 15).unwrap(), "late");
        assert!(matches!(err, Err(LedgerError::ClosedPeriod(_, _))));
    }

    #[test]
    fn invalid_range_rejected() {
        let mut l = period_ledger();
        let err = l.create_period(
            "bad",
            NaiveDate::from_ymd_opt(2026, 10, 31).unwrap(),
            NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
        );
        assert!(matches!(err, Err(LedgerError::InvalidPeriodRange { .. })));
    }
}
