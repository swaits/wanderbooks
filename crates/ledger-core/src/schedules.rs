//! Things that happen on a schedule.
//!
//! Two kinds, both master data owned by [`crate::Ledger`] and journaled:
//!
//! - **Amortization schedules** spread a prepaid expense or deferred revenue
//!   evenly over the months it covers: a year of insurance paid up front is
//!   an asset expensed 1/12 a month; a year of subscription revenue
//!   collected up front is a liability earned 1/12 a month. Like
//!   depreciation, the schedule is the evidence for its postings, so they
//!   need no receipts. Each month posts once (idempotent runs).
//! - **Recurring entries** (rent, subscriptions, payroll) generate *drafts*
//!   on their dates, never posted entries: the agent attaches the actual
//!   receipt and posts through the normal preview, so the receipt policy is
//!   never bypassed.

use crate::{AccountId, AccountType, ChartOfAccounts, LedgerError, Money, Side};
use chrono::{Datelike, Days, Months, NaiveDate};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Opaque handle for an amortization schedule.
pub type ScheduleId = u64;
/// Opaque handle for a recurring entry.
pub type RecurringId = u64;
/// One line of a template entry: account, side, amount.
pub type TemplateLine = (AccountId, Side, Money);

/// What an amortization schedule recognizes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleKind {
    /// A prepaid expense: each month debits the expense account and
    /// credits the prepaid (asset) account.
    PrepaidExpense,
    /// Deferred (unearned) revenue: each month debits the deferred revenue
    /// (liability) account and credits the revenue account.
    DeferredRevenue,
}

/// An amortization schedule.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AmortizationSchedule {
    /// Opaque handle.
    pub id: ScheduleId,
    /// What it is, e.g. "2027 general liability insurance".
    pub description: String,
    /// Prepaid expense or deferred revenue.
    pub kind: ScheduleKind,
    /// The amount to spread.
    pub total: Money,
    /// Any date in the first month covered.
    pub start: NaiveDate,
    /// Months covered.
    pub months: u32,
    /// The balance-sheet account: prepaid (asset) or deferred revenue
    /// (liability).
    pub balance_account: AccountId,
    /// The P&L account: expense or revenue.
    pub pl_account: AccountId,
    /// Months already posted, as their month-end dates.
    pub posted: BTreeSet<NaiveDate>,
}

/// Parameters for a new schedule.
#[derive(Clone, Debug)]
pub struct NewSchedule {
    /// What it is.
    pub description: String,
    /// Prepaid expense or deferred revenue.
    pub kind: ScheduleKind,
    /// The amount to spread (positive).
    pub total: Money,
    /// Any date in the first month covered.
    pub start: NaiveDate,
    /// Months covered (at least 1).
    pub months: u32,
    /// Prepaid (asset) or deferred revenue (liability) account.
    pub balance_account: AccountId,
    /// Expense or revenue account.
    pub pl_account: AccountId,
}

/// The last day of `d`'s month.
pub fn month_end(d: NaiveDate) -> NaiveDate {
    let first = NaiveDate::from_ymd_opt(d.year(), d.month(), 1).expect("valid");
    first + Months::new(1) - Days::new(1)
}

/// The schedule's monthly postings: (month-end date, amount). Equal
/// amounts to the cent, the last month absorbing rounding, so they sum to
/// the total exactly.
pub fn amortization_plan(
    total: Decimal,
    start: NaiveDate,
    months: u32,
) -> Vec<(NaiveDate, Decimal)> {
    let each = (total / Decimal::from(months)).round_dp(2);
    let first = NaiveDate::from_ymd_opt(start.year(), start.month(), 1).expect("valid");
    (0..months)
        .map(|i| {
            let date = month_end(first + Months::new(i));
            let amount = if i + 1 == months {
                total - each * Decimal::from(months - 1)
            } else {
                each
            };
            (date, amount)
        })
        .collect()
}

/// Validate a new schedule against the chart.
pub fn validate_schedule(new: &NewSchedule, chart: &ChartOfAccounts) -> Result<(), LedgerError> {
    let bad = |m: String| Err(LedgerError::InvalidSchedule(m));
    if new.description.trim().is_empty() {
        return bad("description is required".into());
    }
    if new.total.amount <= Decimal::ZERO {
        return bad(format!("total must be positive, got {}", new.total.amount));
    }
    if new.months == 0 || new.months > 600 {
        return bad(format!("months must be 1 to 600, got {}", new.months));
    }
    let (want_balance, want_pl) = match new.kind {
        ScheduleKind::PrepaidExpense => (AccountType::Asset, AccountType::Expense),
        ScheduleKind::DeferredRevenue => (AccountType::Liability, AccountType::Revenue),
    };
    for (id, want) in [
        (new.balance_account, want_balance),
        (new.pl_account, want_pl),
    ] {
        let a = chart.get(id).ok_or(LedgerError::UnknownAccount(id))?;
        if a.account_type != want || a.placeholder {
            return bad(format!(
                "account {} must be a postable {want:?} account for a {:?} schedule",
                a.code, new.kind
            ));
        }
    }
    Ok(())
}

impl AmortizationSchedule {
    /// The postings for one month: (debit account, credit account).
    pub fn accounts(&self) -> (AccountId, AccountId) {
        match self.kind {
            ScheduleKind::PrepaidExpense => (self.pl_account, self.balance_account),
            ScheduleKind::DeferredRevenue => (self.balance_account, self.pl_account),
        }
    }

    /// Months due through `through` and not yet posted.
    pub fn due(&self, through: NaiveDate) -> Vec<(NaiveDate, Decimal)> {
        amortization_plan(self.total.amount, self.start, self.months)
            .into_iter()
            .filter(|(d, _)| *d <= through && !self.posted.contains(d))
            .collect()
    }

    /// The amount not yet recognized.
    pub fn remaining(&self) -> Decimal {
        amortization_plan(self.total.amount, self.start, self.months)
            .into_iter()
            .filter(|(d, _)| !self.posted.contains(d))
            .map(|(_, a)| a)
            .sum()
    }
}

/// How often a recurring entry happens.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Frequency {
    /// Every 7 days.
    Weekly,
    /// Every month, on the start date's day (clamped to month end).
    Monthly,
    /// Every three months.
    Quarterly,
    /// Every year.
    Yearly,
}

impl Frequency {
    /// Parse "weekly", "monthly", "quarterly", "yearly" ("annually").
    pub fn parse(s: &str) -> Option<Frequency> {
        match s.trim().to_lowercase().as_str() {
            "weekly" => Some(Frequency::Weekly),
            "monthly" => Some(Frequency::Monthly),
            "quarterly" => Some(Frequency::Quarterly),
            "yearly" | "annually" | "annual" => Some(Frequency::Yearly),
            _ => None,
        }
    }

    /// The `k`th occurrence from `start`, computed from the start each
    /// time so month-end dates don't drift (Jan 31 → Feb 28 → Mar 31).
    pub fn nth(&self, start: NaiveDate, k: u32) -> Option<NaiveDate> {
        match self {
            Frequency::Weekly => start.checked_add_days(Days::new(7 * k as u64)),
            Frequency::Monthly => start.checked_add_months(Months::new(k)),
            Frequency::Quarterly => start.checked_add_months(Months::new(3 * k)),
            Frequency::Yearly => start.checked_add_months(Months::new(12 * k)),
        }
    }
}

/// A recurring entry: a balanced template and when it happens.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecurringEntry {
    /// Opaque handle.
    pub id: RecurringId,
    /// Description for each generated entry.
    pub description: String,
    /// The balanced lines.
    pub lines: Vec<TemplateLine>,
    /// How often.
    pub frequency: Frequency,
    /// First occurrence.
    pub start: NaiveDate,
    /// Last date an occurrence may fall on, if it ends.
    pub end: Option<NaiveDate>,
    /// Occurrences already generated, by date.
    pub generated: BTreeSet<NaiveDate>,
}

/// Parameters for a new recurring entry.
#[derive(Clone, Debug)]
pub struct NewRecurring {
    /// Description for each generated entry.
    pub description: String,
    /// The balanced lines.
    pub lines: Vec<TemplateLine>,
    /// How often.
    pub frequency: Frequency,
    /// First occurrence.
    pub start: NaiveDate,
    /// Optional last date.
    pub end: Option<NaiveDate>,
}

/// Validate a new recurring entry: at least two lines, positive amounts,
/// postable accounts, balanced, and an end not before the start.
pub fn validate_recurring(new: &NewRecurring, chart: &ChartOfAccounts) -> Result<(), LedgerError> {
    let bad = |m: String| Err(LedgerError::InvalidSchedule(m));
    if new.description.trim().is_empty() {
        return bad("description is required".into());
    }
    if new.lines.len() < 2 {
        return bad("a recurring entry needs at least two lines".into());
    }
    if new.end.is_some_and(|e| e < new.start) {
        return bad("end is before start".into());
    }
    let (mut debits, mut credits) = (Decimal::ZERO, Decimal::ZERO);
    for (account, side, money) in &new.lines {
        let a = chart
            .get(*account)
            .ok_or(LedgerError::UnknownAccount(*account))?;
        if a.placeholder {
            return bad(format!("account {} is a placeholder", a.code));
        }
        if money.amount <= Decimal::ZERO {
            return bad(format!(
                "line amounts must be positive, got {}",
                money.amount
            ));
        }
        match side {
            Side::Debit => debits += money.amount,
            Side::Credit => credits += money.amount,
        }
    }
    if debits != credits {
        return bad(format!(
            "lines don't balance: debits {debits}, credits {credits}"
        ));
    }
    Ok(())
}

impl RecurringEntry {
    /// Occurrences on or before `through` (and the end), not yet generated.
    pub fn due(&self, through: NaiveDate) -> Vec<NaiveDate> {
        let last = self.end.map_or(through, |e| e.min(through));
        (0..)
            .map_while(|k| self.frequency.nth(self.start, k).filter(|d| *d <= last))
            .filter(|d| !self.generated.contains(d))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn plans_sum_exactly_and_land_on_month_ends() {
        let plan = amortization_plan(Decimal::new(100_000, 2), d(2026, 11, 15), 3);
        // 1,000.00 / 3 = 333.33, 333.33, 333.34.
        assert_eq!(
            plan,
            [
                (d(2026, 11, 30), Decimal::new(33_333, 2)),
                (d(2026, 12, 31), Decimal::new(33_333, 2)),
                (d(2027, 1, 31), Decimal::new(33_334, 2)),
            ]
        );
        let leap = amortization_plan(Decimal::new(1200, 0), d(2028, 2, 1), 1);
        assert_eq!(leap, [(d(2028, 2, 29), Decimal::new(1200, 0))]);
    }

    #[test]
    fn monthly_occurrences_do_not_drift() {
        let f = Frequency::Monthly;
        let dates: Vec<NaiveDate> = (0..4).map(|k| f.nth(d(2026, 1, 31), k).unwrap()).collect();
        assert_eq!(
            dates,
            [
                d(2026, 1, 31),
                d(2026, 2, 28),
                d(2026, 3, 31),
                d(2026, 4, 30)
            ]
        );
        assert_eq!(
            Frequency::Quarterly.nth(d(2026, 1, 15), 3),
            Some(d(2026, 10, 15))
        );
        assert_eq!(
            Frequency::Weekly.nth(d(2026, 1, 1), 2),
            Some(d(2026, 1, 15))
        );
        assert_eq!(Frequency::parse("Annually"), Some(Frequency::Yearly));
    }
}
