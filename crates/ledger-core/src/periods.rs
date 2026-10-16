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
//! Once closed, [`Ledger::try_create_entry`](crate::Ledger::try_create_entry) and [`Ledger::post_entry`](crate::Ledger::post_entry) reject
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

/// Where a period close puts the period's net income, and which owner
/// accounts (draws, distributions, dividends) it zeroes into equity too.
#[derive(Clone, Debug, PartialEq)]
pub enum CloseTo {
    /// The Retained Earnings account ([`RETAINED_EARNINGS_CODE`], created if
    /// missing): the corporate convention. `sweep` lists dividend or
    /// distribution accounts to close into it.
    RetainedEarnings {
        /// Equity accounts zeroed into Retained Earnings.
        sweep: Vec<crate::AccountId>,
    },
    /// One equity account, e.g. a sole proprietor's capital, with `sweep`
    /// (e.g. the owner's draws) zeroed into it.
    Account {
        /// The equity account receiving the net.
        into: crate::AccountId,
        /// Equity accounts zeroed into it.
        sweep: Vec<crate::AccountId>,
    },
    /// Members' capital accounts by share (shares sum to 1): the
    /// partnership convention, where each member's capital account carries
    /// their allocated earnings and absorbs their distributions. Odd cents
    /// go by largest remainder ([`allocate`]), cumulatively: a run of
    /// member closes with the same members and shares gives each member,
    /// in total, their share of the run's total to the cent, so rounding
    /// never piles up close after close (rules.md B-13; since 2026-10-13).
    Members(Vec<MemberClose>),
    /// [`CloseTo::Members`] rounding each close on its own (largest
    /// remainder), the rule for closes journaled on 2026-10-13 before the
    /// cumulative one. Only replay of those journals produces it.
    MembersPerClose(Vec<MemberClose>),
    /// [`CloseTo::Members`] under the rounding closes used before
    /// 2026-10-13 ([`allocate_last_absorbs`]). Only replay of those journals
    /// produces it; new closes use `Members`.
    MembersLastAbsorbs(Vec<MemberClose>),
}

/// One member in a partnership close.
#[derive(Clone, Debug, PartialEq)]
pub struct MemberClose {
    /// The member's capital account.
    pub capital: crate::AccountId,
    /// Their share of net income.
    pub share: rust_decimal::Decimal,
    /// Their distributions account, zeroed into their capital, if they
    /// have one.
    pub distributions: Option<crate::AccountId>,
}

/// Split `amount` by `shares` (which sum to 1) to the cent, by largest
/// remainder: every part is its exact share rounded toward zero to the
/// cent, and the leftover cents go one each to the largest remainders,
/// ties to the earlier member ([`crate::kernel::apportion`]). The parts
/// always sum to `amount` exactly, and no member is favored by position
/// (owner, 2026-10-13). A loss splits as the negation of the same gain.
pub fn allocate(
    amount: rust_decimal::Decimal,
    shares: &[rust_decimal::Decimal],
) -> Vec<rust_decimal::Decimal> {
    use rust_decimal::{Decimal, RoundingStrategy::ToZero};
    let magnitude = amount.abs();
    let exact: Vec<Decimal> = shares.iter().map(|s| magnitude * s).collect();
    let mut parts: Vec<Decimal> = exact
        .iter()
        .map(|e| e.round_dp_with_strategy(2, ToZero))
        .collect();
    let remainders: Vec<Decimal> = exact.iter().zip(&parts).map(|(e, p)| e - p).collect();
    crate::kernel::apportion(magnitude, &mut parts, &remainders, Decimal::new(1, 2))
        .expect("shares sum to 1, so the shortfall is under one cent per member");
    if amount.is_sign_negative() {
        for p in &mut parts {
            *p = -*p;
        }
    }
    parts
}

/// The rule closes used before 2026-10-13: each share rounded half away
/// from zero to the cent, the last absorbing the rounding. Kept only so
/// closes journaled under it replay exactly ([`CloseTo::MembersLastAbsorbs`]).
pub fn allocate_last_absorbs(
    amount: rust_decimal::Decimal,
    shares: &[rust_decimal::Decimal],
) -> Vec<rust_decimal::Decimal> {
    use rust_decimal::RoundingStrategy::MidpointAwayFromZero;
    let mut parts: Vec<_> = shares
        .iter()
        .map(|s| (amount * s).round_dp_with_strategy(2, MidpointAwayFromZero))
        .collect();
    crate::kernel::plug_last(amount, &mut parts).expect("allocated amounts are in range");
    parts
}

/// Code of the Retained Earnings account that closing entries credit (or
/// debit), created on the first close if the chart lacks it.
pub const RETAINED_EARNINGS_CODE: &str = "3999";

/// A named accounting period: a month, quarter, or year.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
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
    /// The order it closed in among all closes (1 for the first), once
    /// closed.
    #[serde(default)]
    pub close_seq: Option<u64>,
    /// What a cumulative members close allocated, once closed that way.
    #[serde(default)]
    pub member_close: Option<MemberCloseRecord>,
}

/// A cumulative members close ([`CloseTo::Members`]): who shared, and the
/// net it closed. The next close with the same members continues the run.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct MemberCloseRecord {
    /// (capital account, share), in the order given.
    pub members: Vec<(crate::AccountId, rust_decimal::Decimal)>,
    /// The net income this close allocated (negative for a loss).
    pub net: rust_decimal::Decimal,
}

impl Period {
    /// True when `date` falls inside the period (inclusive).
    pub fn contains(&self, date: NaiveDate) -> bool {
        date >= self.start && date <= self.end
    }
}

/// Validate a period's date range before creation.
/// [`validate_period_order`], and both ends inside the calendar the books
/// keep (crate::calendar; P0 review round 3, S1). For new periods; replay
/// checks only the order (rules.md B-28).
pub fn validate_period_range(start: NaiveDate, end: NaiveDate) -> Result<(), LedgerError> {
    crate::calendar::check(start)?;
    crate::calendar::check(end)?;
    validate_period_order(start, end)
}

/// The check every journaled `period.created` passed when it was written
/// (the rule through 2026-10-16): the end is not before the start.
pub fn validate_period_order(start: NaiveDate, end: NaiveDate) -> Result<(), LedgerError> {
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
        let re = l.chart().find_by_code(RETAINED_EARNINGS_CODE).unwrap().id;
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
