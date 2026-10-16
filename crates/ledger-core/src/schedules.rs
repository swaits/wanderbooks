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

/// The last day of `d`'s month: in chrono's last month (December 262142),
/// NaiveDate::MAX, never a panic.
pub fn month_end(d: NaiveDate) -> NaiveDate {
    d.with_day(1)
        .and_then(|first| first.checked_add_months(Months::new(1)))
        .and_then(|next| next.checked_sub_days(Days::new(1)))
        .unwrap_or(NaiveDate::MAX)
}

/// The smallest total whose plan isn't computed exactly in cents (10^26):
/// 26 integer digits and 2 decimals fill Decimal's 28 significant digits.
pub const EXACT_LIMIT: Decimal =
    Decimal::from_parts(0xe400_0000, 0xdcc8_0cd2, 0x0052_b7d2, false, 0);

/// The schedule's monthly postings: (month-end date, amount). Equal
/// amounts to the cent, the last month absorbing rounding, so they sum to
/// the total exactly.
///
/// ```text
/// INVARIANTS:
///   1. The amounts sum to `total` exactly; all but the last are equal.
///   2. NEVER panics: a schedule an earlier version journaled whose months
///      run past chrono's last day is refused with DateOutOfRange, and one
///      whose amounts exact arithmetic can't hold with TooLargeToCompute
///      (rules.md B-28). `months` is at least 1 (validate_schedule_shape).
///   3. NEVER rounds silently: a total of EXACT_LIMIT or more is refused
///      with TooLargeToCompute. Below it every amount in cents, and every
///      sum of them (none exceeds the total), fits Decimal's 28 significant
///      digits, so INVARIANT 1 holds exactly; at it, Decimal's checked
///      operations round instead of failing.
/// ```
///
/// Replay is unaffected: a posted schedule month passed the line limit
/// (kernel::MAX_LINE_AMOUNT), so no journaled schedule.posted belongs to a
/// schedule with a total anywhere near EXACT_LIMIT.
pub fn amortization_plan(
    total: Decimal,
    start: NaiveDate,
    months: u32,
) -> Result<Vec<(NaiveDate, Decimal)>, LedgerError> {
    let too_large = || LedgerError::TooLargeToCompute(format!("a {months}-month plan of {total}"));
    if total.abs() >= EXACT_LIMIT {
        return Err(too_large());
    }
    let each = total
        .checked_div(Decimal::from(months))
        .ok_or_else(too_large)?
        .round_dp(2);
    let first = start
        .with_day(1)
        .ok_or(LedgerError::DateOutOfRange(start))?;
    (0..months)
        .map(|i| {
            let month = first
                .checked_add_months(Months::new(i))
                .ok_or(LedgerError::DateOutOfRange(start))?;
            let amount = if i + 1 == months {
                each.checked_mul(Decimal::from(months - 1))
                    .and_then(|before| total.checked_sub(before))
                    .ok_or_else(too_large)?
            } else {
                each
            };
            Ok((month_end(month), amount))
        })
        .collect()
}

/// Validate a new schedule against the chart: [`validate_schedule_shape`],
/// the total in range and in cents, and its first and last months inside
/// the calendar the books keep (crate::calendar; P0 review round 3, S1),
/// so every month end of the plan is a date.
pub fn validate_schedule(new: &NewSchedule, chart: &ChartOfAccounts) -> Result<(), LedgerError> {
    validate_schedule_shape(new, chart)?;
    crate::calendar::check(new.start)?;
    if !new
        .start
        .checked_add_months(Months::new(new.months - 1))
        .is_some_and(crate::calendar::in_calendar)
    {
        return Err(LedgerError::InvalidSchedule(format!(
            "{} months from {} run past 9999-12-31, the last day the books keep",
            new.months, new.start
        )));
    }
    // Every month's posting derives from the total: one no line could
    // carry, or finer than a cent, is refused here, not at a month end
    // (found by the store's refusal property, 2026-10-13).
    if new.total.amount > crate::kernel::MAX_LINE_AMOUNT {
        return Err(LedgerError::AmountOutOfRange);
    }
    crate::ledger::check_minor_units(new.total)
}

/// [`validate_schedule`] without the total's range and minor-unit checks
/// added on 2026-10-13 or the calendar bound added in P0: the checks every
/// journaled `schedule.registered` passed when it was written, for
/// replaying journals written before them (rules.md B-28).
pub fn validate_schedule_shape(
    new: &NewSchedule,
    chart: &ChartOfAccounts,
) -> Result<(), LedgerError> {
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

    /// Its monthly postings ([`amortization_plan`]); refused, never a
    /// panic, for one that can't be computed.
    pub fn plan(&self) -> Result<Vec<(NaiveDate, Decimal)>, LedgerError> {
        amortization_plan(self.total.amount, self.start, self.months)
    }

    /// Months due through `through` and not yet posted.
    pub fn due(&self, through: NaiveDate) -> Result<Vec<(NaiveDate, Decimal)>, LedgerError> {
        Ok(self
            .plan()?
            .into_iter()
            .filter(|(d, _)| *d <= through && !self.posted.contains(d))
            .collect())
    }

    /// The amount not yet recognized: the sum of the plan's unposted
    /// months.
    pub fn remaining(&self) -> Result<Decimal, LedgerError> {
        self.plan()?
            .into_iter()
            .filter(|(d, _)| !self.posted.contains(d))
            .try_fold(Decimal::ZERO, |sum, (_, a)| sum.checked_add(a))
            .ok_or_else(|| {
                LedgerError::TooLargeToCompute(format!("schedule {}'s remaining amount", self.id))
            })
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

/// Validate a new recurring entry: [`validate_recurring_journaled`], its
/// dates inside the calendar the books keep, and no line larger than a
/// journal line may be (P0 review round 3, S1).
pub fn validate_recurring(new: &NewRecurring, chart: &ChartOfAccounts) -> Result<(), LedgerError> {
    // The size first: a line past the limit is out of range, whatever its
    // sum would be.
    if new
        .lines
        .iter()
        .any(|(_, _, money)| money.amount > crate::kernel::MAX_LINE_AMOUNT)
    {
        return Err(LedgerError::AmountOutOfRange);
    }
    validate_recurring_journaled(new, chart)?;
    crate::calendar::check(new.start)?;
    if let Some(end) = new.end {
        crate::calendar::check(end)?;
    }
    Ok(())
}

/// The checks every journaled `recurring.registered` passed when it was
/// written (the rule through 2026-10-16): at least two lines, positive
/// amounts, postable accounts, balanced, and an end not before the start.
/// Replay applies exactly these (rules.md B-28). Sums too large for exact
/// arithmetic are refused (TooLargeToCompute), never a panic.
pub fn validate_recurring_journaled(
    new: &NewRecurring,
    chart: &ChartOfAccounts,
) -> Result<(), LedgerError> {
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
        let total = match side {
            Side::Debit => &mut debits,
            Side::Credit => &mut credits,
        };
        *total = total
            .checked_add(money.amount)
            .ok_or_else(|| LedgerError::TooLargeToCompute("the recurring entry's total".into()))?;
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
        let plan = amortization_plan(Decimal::new(100_000, 2), d(2026, 11, 15), 3).unwrap();
        // 1,000.00 / 3 = 333.33, 333.33, 333.34.
        assert_eq!(
            plan,
            [
                (d(2026, 11, 30), Decimal::new(33_333, 2)),
                (d(2026, 12, 31), Decimal::new(33_333, 2)),
                (d(2027, 1, 31), Decimal::new(33_334, 2)),
            ]
        );
        let leap = amortization_plan(Decimal::new(1200, 0), d(2028, 2, 1), 1).unwrap();
        assert_eq!(leap, [(d(2028, 2, 29), Decimal::new(1200, 0))]);
    }

    /// A schedule an earlier version journaled, at chrono's last month or
    /// with Decimal's largest total: its plan is computed where it can be,
    /// refused where it can't, never a panic (rules.md B-28).
    #[test]
    fn plans_at_the_edges_are_computed_or_refused() {
        assert_eq!(month_end(NaiveDate::MAX), NaiveDate::MAX);
        assert_eq!(month_end(d(262_142, 12, 1)), NaiveDate::MAX);
        assert_eq!(month_end(d(262_142, 11, 5)), d(262_142, 11, 30));
        assert_eq!(month_end(NaiveDate::MIN), d(NaiveDate::MIN.year(), 1, 31));
        // The last month chrono holds is a month of the plan.
        assert_eq!(
            amortization_plan(Decimal::new(300, 0), d(262_142, 11, 9), 2).unwrap(),
            [
                (d(262_142, 11, 30), Decimal::new(150, 0)),
                (NaiveDate::MAX, Decimal::new(150, 0)),
            ]
        );
        // One month further is none.
        assert_eq!(
            amortization_plan(Decimal::new(300, 0), d(262_142, 11, 9), 3),
            Err(LedgerError::DateOutOfRange(d(262_142, 11, 9)))
        );
        // Past the line limit (replay takes any positive total): exact, the
        // last month the remainder, right up to just under EXACT_LIMIT.
        for big in ["2000000000000000.00", "99999999999999999999999999.99"] {
            let big: Decimal = big.parse().unwrap();
            let plan = amortization_plan(big, d(2026, 1, 1), 600).unwrap();
            assert_eq!(plan.len(), 600);
            assert_eq!(
                plan.iter()
                    .try_fold(Decimal::ZERO, |s, (_, a)| s.checked_add(*a)),
                Some(big)
            );
        }
        // At EXACT_LIMIT and beyond, Decimal would round: refused, never
        // rounded (INVARIANT 3), in either sign.
        assert_eq!(
            EXACT_LIMIT,
            Decimal::from_i128_with_scale(10_i128.pow(26), 0)
        );
        for total in [EXACT_LIMIT, -EXACT_LIMIT, Decimal::MAX, Decimal::MIN] {
            assert!(matches!(
                amortization_plan(total, d(2026, 1, 1), 600),
                Err(LedgerError::TooLargeToCompute(_))
            ));
        }
        let s = AmortizationSchedule {
            id: 1,
            description: "edge".into(),
            kind: ScheduleKind::PrepaidExpense,
            total: Money::new(Decimal::new(300, 0), crate::Currency::USD),
            start: d(262_142, 11, 9),
            months: 3,
            balance_account: 1,
            pl_account: 2,
            posted: BTreeSet::new(),
        };
        assert!(s.due(NaiveDate::MAX).is_err());
        assert!(s.remaining().is_err());
        let s = AmortizationSchedule { months: 2, ..s };
        assert_eq!(s.due(NaiveDate::MAX).unwrap().len(), 2);
        assert_eq!(s.remaining(), Ok(Decimal::new(300, 0)));
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
