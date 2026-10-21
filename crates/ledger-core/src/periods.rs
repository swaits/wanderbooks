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
    /// go by largest remainder, cumulatively from the first members close
    /// with these members and shares ([`allocate_cumulative`]): each close
    /// gives a member their share of everything closed to the members so
    /// far, to the currency's minor unit, less what earlier closes gave
    /// them, so no member is ever a minor unit from their exact share
    /// (rules.md B-13; D-10). What new closes are made with; the books
    /// journal it as [`CloseTo::MembersCumulative`].
    Members(Vec<MemberClose>),
    /// [`CloseTo::Members`] as journaled: the count of members closes
    /// starts at the close numbered `since_seq` (its
    /// [`Period::close_seq`]). Replay produces it and checks the start is
    /// the one the books derive.
    MembersCumulative {
        /// The members and shares.
        members: Vec<MemberClose>,
        /// The close the count starts at: the first members close with
        /// these members and shares since the last with others.
        since_seq: u64,
    },
    /// The run rule of 2026-10-13 to P3 (journaled `"cumulative"`): a
    /// back-to-back run of members closes with the same members and shares
    /// in the same order splits cumulatively, and any other close ends the
    /// run. Only replay of those journals produces it.
    MembersRun(Vec<MemberClose>),
    /// [`CloseTo::Members`] rounding each close on its own (largest
    /// remainder), the rule for closes journaled on 2026-10-13 before the
    /// cumulative one. Only replay of those journals produces it.
    MembersPerClose(Vec<MemberClose>),
    /// [`CloseTo::Members`] under the rounding closes used before
    /// 2026-10-13 ([`allocate_last_absorbs`]). Only replay of those journals
    /// produces it; new closes use `Members`.
    MembersLastAbsorbs(Vec<MemberClose>),
}

impl CloseTo {
    /// The members a close splits the net among, under whichever rule;
    /// `None` for a close to retained earnings or one account.
    pub fn members(&self) -> Option<&[MemberClose]> {
        match self {
            CloseTo::RetainedEarnings { .. } | CloseTo::Account { .. } => None,
            CloseTo::Members(members)
            | CloseTo::MembersCumulative { members, .. }
            | CloseTo::MembersRun(members)
            | CloseTo::MembersPerClose(members)
            | CloseTo::MembersLastAbsorbs(members) => Some(members),
        }
    }
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
/// Closes journaled under the rules of 2026-10-13 replay with it, in cents
/// whatever the currency; new closes split with [`allocate_cumulative`].
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

/// What one members close gives each member: [`allocate`]'s split of the
/// running `total` (everything closed to these members and shares,
/// this close included), to `places` decimal places
/// ([`crate::Currency::allocation_places`]) instead of cents, less what
/// earlier closes gave each member (`given`, in the order of `shares`).
/// Each member's running total is then one split of the running total
/// (rules.md B-13; [`crate::kernel::apportion_cumulative`]). A loss is
/// the negation of the same gain: it splits |total| against `given`
/// negated, then negates.
///
/// ```text
/// GOAL: no member's running share of earnings is ever a minor unit from
///       their exact share, whatever earlier closes gave them.
/// SOURCE: docs/spec/rules.md B-13 (26 U.S.C. §704(a); Treas. Reg.
///         §1.704-1(b)(2)(iv)(b); largest remainder: DECISION, owner
///         2026-10-13; from the first close: D-10); B-3 (the minor unit).
/// INVARIANTS:
///   1. The parts MUST sum to total − Σ given exactly.
///   2. given[i] + part[i] MUST be member i's largest-remainder split of
///      total to `places` (ties to the earlier member): one split.
///   3. So when total is a whole number of units of `places` (every posted
///      line is at its currency's allocation_places: its minor unit, B-3,
///      or for a currency without one the finest a line carries),
///      |given[i] + part[i] − total × share[i]| is under one unit (Lean
///      allocW_drift, cumulative_drift; Kani
///      cumulative_close_never_drifts_a_unit_at_*).
///   4. NEVER rounds a product: if |total| × share isn't exact in Decimal
///      (a share of many decimal places times a large total), refuse
///      (TooLargeToCompute): a rounded product could rank the remainders
///      wrong, and the bound above assumes exact ones (P3 review R-5).
/// SCENARIOS:
///   1. IF 101 yen splits 0.5 / 0.25 / 0.25 with nothing given: SHOULD be
///      51 / 25 / 25 (exact 50.5 / 25.25 / 25.25; the yen to the largest
///      remainder), never fractions of a yen.
///   2. IF 0.03 was given as A 0.03 / B 0.00 (50/50) and the total is now
///      0.04: SHOULD be A −0.01, B +0.02.
///   3. IF the total is a loss: SHOULD be the negation of the same gain
///      against the negated given.
///   4. IF the shares and given don't pair up, or places exceeds what a
///      line may carry: SHOULD refuse.
/// ```
pub fn allocate_cumulative(
    total: rust_decimal::Decimal,
    shares: &[rust_decimal::Decimal],
    given: &[rust_decimal::Decimal],
    places: u32,
) -> Result<Vec<rust_decimal::Decimal>, LedgerError> {
    use rust_decimal::{Decimal, RoundingStrategy::ToZero};
    if shares.len() != given.len() {
        return Err(LedgerError::InvalidMembers(format!(
            "{} shares but {} earlier parts",
            shares.len(),
            given.len()
        )));
    }
    if places > crate::kernel::MAX_LINE_SCALE {
        return Err(LedgerError::TooLargeToCompute(format!(
            "a split to {places} decimal places"
        )));
    }
    let loss = total < Decimal::ZERO;
    let magnitude = total.abs().normalize();
    let given: Vec<Decimal> = given.iter().map(|&g| if loss { -g } else { g }).collect();
    let mut parts = Vec::with_capacity(shares.len());
    let mut remainders = Vec::with_capacity(shares.len());
    for share in shares {
        let share = share.normalize();
        // Exact exactly when Decimal kept every digit: a product it had to
        // round comes back with fewer places than its factors carry (a
        // zero product, which it returns without places, is exact).
        let exact = magnitude
            .checked_mul(share)
            .filter(|e| e.is_zero() || e.scale() == magnitude.scale() + share.scale())
            .ok_or_else(|| {
                LedgerError::TooLargeToCompute(format!(
                    "a member's share {share} of {magnitude}, to its last digit,"
                ))
            })?;
        let floor = exact.round_dp_with_strategy(places, ToZero);
        parts.push(floor);
        remainders.push(exact - floor);
    }
    crate::kernel::apportion_cumulative(
        magnitude,
        &mut parts,
        &remainders,
        Decimal::new(1, places),
        &given,
    )
    .ok_or_else(|| LedgerError::TooLargeToCompute(format!("the members' shares of {total}")))?;
    if loss {
        for p in parts.iter_mut().filter(|p| !p.is_zero()) {
            *p = -*p;
        }
    }
    Ok(parts)
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
    /// What a close under the run rule ([`CloseTo::MembersRun`], journaled
    /// `"cumulative"`) allocated, once closed that way. Only those closes
    /// have one: its serialized form is what earlier versions wrote, and
    /// stays so (rules.md B-28). Every members close's split, under any
    /// rule, is [`Period::members_closed`].
    #[serde(default)]
    pub member_close: Option<MemberCloseRecord>,
    /// What a members close (any rule) split, once closed to the members.
    /// Derived on replay; never serialized, so a period reads as earlier
    /// versions wrote it.
    #[serde(skip)]
    pub members_closed: Option<MembersClosed>,
}

/// A run-rule members close ([`CloseTo::MembersRun`]): who shared, and the
/// net it closed. The next run-rule close with the same members continues
/// the run.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct MemberCloseRecord {
    /// (capital account, share), in the order given.
    pub members: Vec<(crate::AccountId, rust_decimal::Decimal)>,
    /// The net income this close allocated (negative for a loss).
    pub net: rust_decimal::Decimal,
}

/// The rounding rule a members close was made under, as journaled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemberRule {
    /// The last member absorbed the rounding (closes before 2026-10-13;
    /// [`CloseTo::MembersLastAbsorbs`]).
    LastAbsorbs,
    /// Largest remainder, each close on its own (2026-10-13;
    /// [`CloseTo::MembersPerClose`]).
    PerClose,
    /// Cumulative over a back-to-back run (2026-10-13 to P3;
    /// [`CloseTo::MembersRun`]).
    Run,
    /// Cumulative from the members close numbered `since_seq`
    /// ([`CloseTo::Members`], [`CloseTo::MembersCumulative`]).
    Cumulative {
        /// The close the count starts at.
        since_seq: u64,
    },
}

/// A members close's split, under any rule ([`Period::members_closed`]).
#[derive(Clone, Debug, PartialEq)]
pub struct MembersClosed {
    /// (capital account, share), in the close's order.
    pub members: Vec<(crate::AccountId, rust_decimal::Decimal)>,
    /// The net income this close allocated (negative for a loss).
    pub net: rust_decimal::Decimal,
    /// The income it credited each member's capital account (debited for
    /// a negative part), in the same order: their share of `net` plus any
    /// true-up. All zero when the close posted no entry.
    pub parts: Vec<rust_decimal::Decimal>,
    /// The rule it was made under.
    pub rule: MemberRule,
}

/// One member's part of a close under the cumulative rule
/// ([`crate::Ledger::member_close_split`]).
#[derive(Clone, Debug, PartialEq)]
pub struct MemberSplit {
    /// The member's capital account.
    pub capital: crate::AccountId,
    /// Their share.
    pub share: rust_decimal::Decimal,
    /// What this close credited them (negative: debited), any true-up of
    /// earlier rounding included.
    pub part: rust_decimal::Decimal,
    /// What the count's closes have credited them in all, this one
    /// included.
    pub running: rust_decimal::Decimal,
    /// Their exact share of everything the count has closed, this close
    /// included (share × total, unrounded): `running` is within one minor
    /// unit of it once a close of the count has posted under the
    /// cumulative rule (until then it still carries the rounding of closes
    /// made under an earlier rule, which that close trues up).
    pub exact: rust_decimal::Decimal,
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
    use rust_decimal::Decimal;

    fn dec(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    fn decs(v: &[&str]) -> Vec<Decimal> {
        v.iter().map(|s| dec(s)).collect()
    }

    /// allocate_cumulative scenario 1: 101 yen at 0.5 / 0.25 / 0.25 is
    /// exactly 50.5 / 25.25 / 25.25; floors 50 / 25 / 25 leave one yen,
    /// which goes to the largest remainder (0.5): 51 / 25 / 25. Never a
    /// fraction of a yen. A dinar splits to thousandths.
    #[test]
    fn allocate_cumulative_splits_to_the_currencys_minor_unit() {
        let shares = decs(&["0.5", "0.25", "0.25"]);
        let none = decs(&["0", "0", "0"]);
        assert_eq!(
            allocate_cumulative(dec("101"), &shares, &none, 0).unwrap(),
            decs(&["51", "25", "25"])
        );
        // KWD: 0.101 at 50/50 is 0.0505 each; floors 0.050 / 0.050 and
        // the tied thousandth to the first.
        let half = decs(&["0.5", "0.5"]);
        assert_eq!(
            allocate_cumulative(dec("0.101"), &half, &decs(&["0", "0"]), 3).unwrap(),
            decs(&["0.051", "0.050"])
        );
        // Cents, as allocate splits them.
        assert_eq!(
            allocate_cumulative(dec("0.07"), &half, &decs(&["0", "0"]), 2).unwrap(),
            allocate(dec("0.07"), &half)
        );
    }

    /// allocate_cumulative scenarios 2 and 3: three cents given A 0.03 /
    /// B 0.00 and a running 0.04 give A −0.01, B +0.02; a loss of the
    /// same is the negation against the negated given.
    #[test]
    fn allocate_cumulative_trues_up_and_mirrors_a_loss() {
        let half = decs(&["0.5", "0.5"]);
        assert_eq!(
            allocate_cumulative(dec("0.04"), &half, &decs(&["0.03", "0"]), 2).unwrap(),
            decs(&["-0.01", "0.02"])
        );
        assert_eq!(
            allocate_cumulative(dec("-0.04"), &half, &decs(&["-0.03", "0"]), 2).unwrap(),
            decs(&["0.01", "-0.02"])
        );
        // Given that sums to the earlier total: the parts sum to the new
        // amount (0.05 − 0.03 = 0.02); running totals 0.03 / 0.02.
        let parts = allocate_cumulative(dec("0.05"), &half, &decs(&["0.01", "0.02"]), 2).unwrap();
        assert_eq!(parts, decs(&["0.02", "0.00"]));
        assert_eq!(parts.iter().sum::<Decimal>(), dec("0.02"));
    }

    /// allocate_cumulative INVARIANT 4 (P3 review R-5): a share of 28
    /// decimal places times 100.00 has 30, more than Decimal keeps, so the
    /// product would be rounded: refused, never split from a rounded
    /// product. Four places (33.33%) are exact at any line amount.
    #[test]
    fn allocate_cumulative_refuses_a_product_it_cannot_compute_exactly() {
        let third = dec("0.3333333333333333333333333333");
        let last = Decimal::ONE - third - third;
        let shares = [third, third, last];
        let none = decs(&["0", "0", "0"]);
        assert!(matches!(
            allocate_cumulative(dec("100.00"), &shares, &none, 2),
            Err(LedgerError::TooLargeToCompute(_))
        ));
        let shares = decs(&["0.3333", "0.3333", "0.3334"]);
        let max = crate::kernel::MAX_LINE_AMOUNT - dec("0.01");
        let parts = allocate_cumulative(max, &shares, &none, 2).unwrap();
        assert_eq!(parts.iter().sum::<Decimal>(), max);
        // Trailing zeros are not digits: "0.50" and "0.5" split alike.
        assert_eq!(
            allocate_cumulative(dec("0.01"), &decs(&["0.50", "0.50"]), &decs(&["0", "0"]), 2)
                .unwrap(),
            decs(&["0.01", "0.00"])
        );
    }

    /// allocate_cumulative scenario 4.
    #[test]
    fn allocate_cumulative_refuses_unpaired_inputs() {
        assert!(matches!(
            allocate_cumulative(dec("1"), &decs(&["1"]), &[], 2),
            Err(LedgerError::InvalidMembers(_))
        ));
        assert!(matches!(
            allocate_cumulative(dec("1"), &decs(&["1"]), &decs(&["0"]), 11),
            Err(LedgerError::TooLargeToCompute(_))
        ));
    }

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
