//! Time series over the books, for dashboards and interval-column reports.
//!
//! Statements answer "what was true for this range"; these answer "how did
//! it change": a range split into months, quarters, or years; balances of a
//! set of accounts on a series of dates; profit and loss per interval. They
//! are built from the same posted entries and statements as everything
//! else, so a number on a chart always ties to a number on a statement.

use crate::reports::profit_and_loss_for_period;
use crate::{AccountId, AccountType, Ledger, LedgerError, Side};
use chrono::{Datelike, Days, Months, NaiveDate};
use rust_decimal::Decimal;
use std::collections::HashSet;

/// How to split a date range into columns.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Interval {
    /// Calendar months.
    Month,
    /// Calendar quarters.
    Quarter,
    /// Calendar years.
    Year,
}

impl Interval {
    fn months(self) -> u32 {
        match self {
            Interval::Month => 1,
            Interval::Quarter => 3,
            Interval::Year => 12,
        }
    }
}

/// The first day of the calendar interval containing `d`.
pub fn interval_start(d: NaiveDate, interval: Interval) -> NaiveDate {
    let month0 = (d.month0() / interval.months()) * interval.months();
    NaiveDate::from_ymd_opt(d.year(), month0 + 1, 1).expect("valid")
}

/// `[start, end]` split on calendar interval boundaries, first and last
/// pieces clipped to the range. Empty when `end < start`.
pub fn intervals(
    start: NaiveDate,
    end: NaiveDate,
    interval: Interval,
) -> Vec<(NaiveDate, NaiveDate)> {
    let mut out = Vec::new();
    let mut from = interval_start(start, interval);
    while from <= end {
        let next = from + Months::new(interval.months());
        let to = (next - Days::new(1)).min(end);
        out.push((from.max(start), to));
        from = next;
    }
    out
}

/// The combined normal-side balance of `accounts` at the end of each date
/// in `dates` (which must be ascending). Assets and expenses count debits
/// as positive; liabilities, equity, and revenue count credits. Mixing
/// types is allowed but rarely meaningful.
pub fn balance_series(
    ledger: &Ledger,
    accounts: &[AccountId],
    dates: &[NaiveDate],
) -> Vec<Decimal> {
    let set: HashSet<AccountId> = accounts.iter().copied().collect();
    let mut postings: Vec<(NaiveDate, Decimal)> = Vec::new();
    for entry in ledger.posted_entries() {
        for line in entry.lines() {
            if !set.contains(&line.account) {
                continue;
            }
            let Some(account) = ledger.chart().get(line.account) else {
                continue;
            };
            let signed = if line.side == account.account_type.normal_side() {
                line.amount.amount
            } else {
                -line.amount.amount
            };
            postings.push((entry.date(), signed));
        }
    }
    postings.sort_by_key(|(d, _)| *d);
    let mut out = Vec::with_capacity(dates.len());
    let mut running = Decimal::ZERO;
    let mut i = 0;
    for date in dates {
        while i < postings.len() && postings[i].0 <= *date {
            running += postings[i].1;
            i += 1;
        }
        out.push(running);
    }
    out
}

/// Every date `accounts` moved, with the combined balance at the end of
/// that day: the exact shape of a balance-over-time chart.
pub fn balance_history(ledger: &Ledger, accounts: &[AccountId]) -> Vec<(NaiveDate, Decimal)> {
    let set: HashSet<AccountId> = accounts.iter().copied().collect();
    let mut dates: Vec<NaiveDate> = ledger
        .posted_entries()
        .filter(|e| e.lines().iter().any(|l| set.contains(&l.account)))
        .map(|e| e.date())
        .collect();
    dates.sort();
    dates.dedup();
    let balances = balance_series(ledger, accounts, &dates);
    dates.into_iter().zip(balances).collect()
}

/// One interval's profit and loss.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntervalPnl {
    /// First day.
    pub start: NaiveDate,
    /// Last day.
    pub end: NaiveDate,
    /// Total revenue.
    pub revenue: Decimal,
    /// Total expenses.
    pub expenses: Decimal,
}

impl IntervalPnl {
    /// Revenue minus expenses.
    pub fn net(&self) -> Decimal {
        self.revenue - self.expenses
    }
}

/// Profit and loss per interval across `[start, end]` (closing entries
/// excluded, as on every period statement).
pub fn pnl_by_interval(
    ledger: &Ledger,
    start: NaiveDate,
    end: NaiveDate,
    interval: Interval,
) -> Result<Vec<IntervalPnl>, LedgerError> {
    intervals(start, end, interval)
        .into_iter()
        .map(|(s, e)| {
            let pl = profit_and_loss_for_period(ledger, s, e, None)?;
            Ok(IntervalPnl {
                start: s,
                end: e,
                revenue: pl.total_revenue,
                expenses: pl.total_expenses,
            })
        })
        .collect()
}

/// Money in and out of `accounts` per interval: (inflows, outflows), each
/// positive. Each entry counts by its net debit (in) or credit (out) to the
/// accounts together: for cash, deposits and payments; a transfer between
/// two of them (checking to savings) is neither; with a credit card in the
/// set, a charge is money out and paying the card moves nothing.
pub fn flows_by_interval(
    ledger: &Ledger,
    accounts: &[AccountId],
    start: NaiveDate,
    end: NaiveDate,
    interval: Interval,
) -> Vec<(NaiveDate, NaiveDate, Decimal, Decimal)> {
    let set: HashSet<AccountId> = accounts.iter().copied().collect();
    let mut out: Vec<(NaiveDate, NaiveDate, Decimal, Decimal)> = intervals(start, end, interval)
        .into_iter()
        .map(|(s, e)| (s, e, Decimal::ZERO, Decimal::ZERO))
        .collect();
    for entry in ledger.posted_entries() {
        let d = entry.date();
        let Some(slot) = out.iter_mut().find(|(s, e, _, _)| *s <= d && d <= *e) else {
            continue;
        };
        // Debits in, credits out, for every account in the set: cash
        // received is a debit to cash, and a charge on a card in the set is
        // a credit to the card.
        let mut net = Decimal::ZERO;
        for line in entry.lines().iter().filter(|l| set.contains(&l.account)) {
            match line.side {
                Side::Debit => net += line.amount.amount,
                Side::Credit => net -= line.amount.amount,
            }
        }
        if net > Decimal::ZERO {
            slot.2 += net;
        } else {
            slot.3 -= net;
        }
    }
    out
}

/// Accounts of the given types, in chart order.
pub fn accounts_of_type(ledger: &Ledger, types: &[AccountType]) -> Vec<AccountId> {
    ledger
        .chart()
        .all()
        .filter(|a| types.contains(&a.account_type))
        .map(|a| a.id)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn intervals_clip_to_the_range() {
        assert_eq!(
            intervals(d(2026, 1, 15), d(2026, 3, 10), Interval::Month),
            [
                (d(2026, 1, 15), d(2026, 1, 31)),
                (d(2026, 2, 1), d(2026, 2, 28)),
                (d(2026, 3, 1), d(2026, 3, 10)),
            ]
        );
        assert_eq!(
            intervals(d(2026, 2, 1), d(2026, 12, 31), Interval::Quarter),
            [
                (d(2026, 2, 1), d(2026, 3, 31)),
                (d(2026, 4, 1), d(2026, 6, 30)),
                (d(2026, 7, 1), d(2026, 9, 30)),
                (d(2026, 10, 1), d(2026, 12, 31)),
            ]
        );
        assert_eq!(
            intervals(d(2025, 6, 1), d(2026, 2, 1), Interval::Year),
            [
                (d(2025, 6, 1), d(2025, 12, 31)),
                (d(2026, 1, 1), d(2026, 2, 1)),
            ]
        );
        assert!(intervals(d(2026, 2, 1), d(2026, 1, 1), Interval::Month).is_empty());
    }
}
