//! Property tests for the ledger engine: invariants that must hold for
//! *every* input, not just the hand-picked examples in `invariants.rs`.
//!
//! These are the properties mutation testing will try to break: if a
//! mutant survives the example tests but breaks a property, the property
//! catches it.

use chrono::NaiveDate;
use ledger_core::{
    AccountType, Currency, Ledger, Money, NewAsset, Side, accumulated_scheduled_through,
    depreciation_schedule,
    reports::{
        balance_sheet, balance_sheet_as_of, profit_and_loss_for_period, trial_balance,
        unclosed_earnings,
    },
    scheduled_book_value,
};
use proptest::prelude::*;
use rust_decimal::Decimal;

fn arb_account_type() -> impl Strategy<Value = AccountType> {
    prop_oneof![
        Just(AccountType::Asset),
        Just(AccountType::Liability),
        Just(AccountType::Equity),
        Just(AccountType::Revenue),
        Just(AccountType::Expense),
    ]
}

/// A random chart: 2..=6 accounts with unique codes, no placeholders.
fn arb_chart() -> impl Strategy<Value = Vec<(String, String, AccountType)>> {
    prop::collection::vec((1u32..9999u32, arb_account_type()), 2..=6).prop_map(|v| {
        v.into_iter()
            .enumerate()
            .map(|(i, (_, t))| (format!("{:04}", 1000 + i), format!("Account {i}"), t))
            .collect()
    })
}

#[derive(Clone, Debug)]
struct GenEntry {
    lines: Vec<(u64, Side, Decimal)>,
    date: NaiveDate,
}

/// A random *balanced* entry over `num_accounts` (1-based ids): 2..=4
/// random lines plus a plug line for the imbalance. Amounts are whole
/// cents, so the plug is always exact.
fn arb_entry(num_accounts: usize) -> impl Strategy<Value = GenEntry> {
    (
        prop::collection::vec(
            (
                1u64..=(num_accounts as u64),
                prop_oneof![Just(Side::Debit), Just(Side::Credit)],
                1u32..1_000_000u32,
            ),
            2..=4,
        ),
        1u32..=12u32,
        1u32..=28u32,
    )
        .prop_map(|(raw, month, day)| {
            let mut lines: Vec<(u64, Side, Decimal)> = raw
                .into_iter()
                .map(|(a, s, cents)| (a, s, Decimal::new(cents as i64, 2)))
                .collect();
            let net: Decimal = lines
                .iter()
                .map(|(_, s, amt)| match s {
                    Side::Debit => *amt,
                    Side::Credit => -*amt,
                })
                .sum();
            if net != Decimal::ZERO {
                let (side, amt) = if net > Decimal::ZERO {
                    (Side::Credit, net)
                } else {
                    (Side::Debit, -net)
                };
                lines.push((1, side, amt));
            }
            GenEntry {
                lines,
                date: NaiveDate::from_ymd_opt(2026, month, day).unwrap(),
            }
        })
}

fn arb_ops() -> impl Strategy<Value = (Vec<(String, String, AccountType)>, Vec<GenEntry>)> {
    arb_chart().prop_flat_map(|chart| {
        let n = chart.len();
        (Just(chart), prop::collection::vec(arb_entry(n), 0..=10))
    })
}

fn build_ledger(
    chart: &[(String, String, AccountType)],
    entries: &[GenEntry],
) -> (Ledger, Vec<u64>) {
    let mut ledger = Ledger::new(Currency::USD);
    for (code, name, typ) in chart {
        ledger.add_account(code, name, *typ, None, false).unwrap();
    }
    let mut ids = Vec::new();
    for e in entries {
        let id = ledger.create_entry(e.date, "prop");
        for (a, s, amt) in &e.lines {
            ledger
                .add_line(id, *a, *s, Money::new(*amt, Currency::USD))
                .unwrap();
        }
        ledger.post_entry(id).unwrap();
        ids.push(id);
    }
    (ledger, ids)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// The trial balance always balances, no matter the operations.
    #[test]
    fn prop_trial_balance_balances((chart, entries) in arb_ops()) {
        let (ledger, _) = build_ledger(&chart, &entries);
        let tb = trial_balance(&ledger);
        prop_assert_eq!(tb.total_debits, tb.total_credits);
        let debit_sum: Decimal = tb.lines.iter().map(|l| l.debit).sum();
        let credit_sum: Decimal = tb.lines.iter().map(|l| l.credit).sum();
        prop_assert_eq!(debit_sum, tb.total_debits);
        prop_assert_eq!(credit_sum, tb.total_credits);
    }

    /// The accounting equation holds for every ledger state:
    /// assets == liabilities + equity (with current earnings folded in).
    #[test]
    fn prop_balance_sheet_balances((chart, entries) in arb_ops()) {
        let (ledger, _) = build_ledger(&chart, &entries);
        let bs = balance_sheet(&ledger).unwrap();
        prop_assert_eq!(bs.assets, bs.liabilities + bs.equity);
    }

    /// Period P&Ls partition the year: split anywhere, the two halves sum
    /// to the whole, and the whole is the life-of-ledger P&L.
    #[test]
    fn prop_period_pnl_partitions((chart, entries) in arb_ops(), split in 0u32..365) {
        let (ledger, _) = build_ledger(&chart, &entries);
        let jan1 = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let dec31 = NaiveDate::from_ymd_opt(2026, 12, 31).unwrap();
        let cut = jan1 + chrono::Days::new(split as u64);
        let first = profit_and_loss_for_period(&ledger, jan1, cut, None).unwrap();
        let rest = if cut < dec31 {
            Some(profit_and_loss_for_period(&ledger, cut.succ_opt().unwrap(), dec31, None).unwrap())
        } else {
            None
        };
        let whole = profit_and_loss_for_period(&ledger, jan1, dec31, None).unwrap();
        let (r2, e2) = rest.map_or((Decimal::ZERO, Decimal::ZERO), |p| (p.total_revenue, p.total_expenses));
        prop_assert_eq!(first.total_revenue + r2, whole.total_revenue);
        prop_assert_eq!(first.total_expenses + e2, whole.total_expenses);
        let all = unclosed_earnings(&ledger).unwrap();
        prop_assert_eq!((whole.total_revenue, whole.total_expenses), (all.revenue, all.expenses));
    }

    /// The accounting equation holds as of every date, and a period's net
    /// income is exactly the change in current earnings across it.
    #[test]
    fn prop_balance_sheet_as_of_balances((chart, entries) in arb_ops(), a in 0u32..365, b in 0u32..365) {
        let (ledger, _) = build_ledger(&chart, &entries);
        let jan1 = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let (lo, hi) = (a.min(b), a.max(b));
        let start = jan1 + chrono::Days::new(lo as u64);
        let end = jan1 + chrono::Days::new(hi as u64);
        let before = balance_sheet_as_of(&ledger, start.pred_opt().unwrap()).unwrap();
        let after = balance_sheet_as_of(&ledger, end).unwrap();
        prop_assert!(before.balanced());
        prop_assert!(after.balanced());
        let pnl = profit_and_loss_for_period(&ledger, start, end, None).unwrap();
        prop_assert_eq!(after.current_earnings - before.current_earnings, pnl.net_income());
        // Past the last entry, as-of agrees with the life-of-ledger sheet.
        let all = balance_sheet(&ledger).unwrap();
        let last = balance_sheet_as_of(&ledger, NaiveDate::from_ymd_opt(2026, 12, 31).unwrap()).unwrap();
        prop_assert_eq!(
            (last.total_assets, last.total_liabilities, last.total_equity),
            (all.assets, all.liabilities, all.equity)
        );
    }

    /// Every posted entry balances on its own, exactly.
    #[test]
    fn prop_posted_entries_balance((chart, entries) in arb_ops()) {
        let (ledger, ids) = build_ledger(&chart, &entries);
        for id in ids {
            let e = ledger.get_entry(id).unwrap();
            let debits: Decimal = e.lines().iter()
                .filter(|l| l.side == Side::Debit)
                .map(|l| l.amount.amount).sum();
            let credits: Decimal = e.lines().iter()
                .filter(|l| l.side == Side::Credit)
                .map(|l| l.amount.amount).sum();
            prop_assert_eq!(debits, credits);
        }
    }

    /// Reversing every entry returns the trial balance to zero:
    /// reversals are exact inverses.
    #[test]
    fn prop_reversals_restore_zero((chart, entries) in arb_ops()) {
        let (mut ledger, ids) = build_ledger(&chart, &entries);
        for id in &ids {
            ledger.reverse_entry(*id, NaiveDate::from_ymd_opt(2026, 12, 31).unwrap(), "reversal").unwrap();
        }
        let tb = trial_balance(&ledger);
        prop_assert_eq!(tb.total_debits, Decimal::ZERO);
        prop_assert_eq!(tb.total_credits, Decimal::ZERO);
    }

    /// Posted entries are immutable: serializing them before and after
    /// further operations yields identical bytes.
    #[test]
    fn prop_posted_entries_immutable((chart, entries) in arb_ops(), extra in prop::collection::vec(arb_entry(2), 0..=5)) {
        prop_assume!(chart.len() >= 2);
        let (mut ledger, ids) = build_ledger(&chart, &entries);
        let before: Vec<String> = ids.iter().map(|id| {
            serde_json::to_string(&serde_json::to_value(ledger.get_entry(*id).unwrap()).unwrap()).unwrap()
        }).collect();
        // More operations: entries sized for a 2-account chart would break
        // on bigger charts, so rebuild them against this chart.
        let n = chart.len();
        let extra_entries: Vec<GenEntry> = extra.into_iter().map(|mut e| {
            for (a, _, _) in e.lines.iter_mut() {
                *a = (*a % n as u64) + 1;
            }
            // Rebalance after remapping accounts (net may have changed).
            let net: Decimal = e.lines.iter().map(|(_, s, amt)| match s {
                Side::Debit => *amt, Side::Credit => -*amt,
            }).sum();
            if net != Decimal::ZERO {
                let (side, amt) = if net > Decimal::ZERO { (Side::Credit, net) } else { (Side::Debit, -net) };
                e.lines.push((1, side, amt));
            }
            e
        }).collect();
        for e in &extra_entries {
            let id = ledger.create_entry(e.date, "extra");
            for (a, s, amt) in &e.lines {
                ledger.add_line(id, *a, *s, Money::new(*amt, Currency::USD)).unwrap();
            }
            ledger.post_entry(id).unwrap();
        }
        for id in &ids {
            ledger.reverse_entry(*id, NaiveDate::from_ymd_opt(2026, 12, 31).unwrap(), "rev").unwrap();
        }
        let after: Vec<String> = ids.iter().map(|id| {
            serde_json::to_string(&serde_json::to_value(ledger.get_entry(*id).unwrap()).unwrap()).unwrap()
        }).collect();
        prop_assert_eq!(before, after);
    }

    /// A journal entry survives a JSON round-trip unchanged.
    #[test]
    fn prop_entry_json_roundtrip((chart, entries) in arb_ops()) {
        let (ledger, ids) = build_ledger(&chart, &entries);
        for id in ids {
            let e = ledger.get_entry(id).unwrap();
            let v = serde_json::to_value(e).unwrap();
            let s = serde_json::to_string(&v).unwrap();
            let v2: serde_json::Value = serde_json::from_str(&s).unwrap();
            prop_assert_eq!(v, v2);
        }
    }

    /// Depreciation schedules are exact: the lifetime total is precisely
    /// cost minus salvage, book value plus accumulated always equals cost,
    /// and amounts are never negative.
    #[test]
    fn prop_depreciation_schedule_exact(
        cost_cents in 100u64..10_000_000u64,
        salvage_cents in 0u64..5_000_000u64,
        life in 1u32..360u32,
        y in 2000i32..2027i32,
        m in 1u32..=12u32,
        d in 1u32..=28u32,
    ) {
        let cost_cents = cost_cents.max(salvage_cents + 1);
        let cost = Decimal::new(cost_cents as i64, 2);
        let salvage = Decimal::new(salvage_cents as i64, 2);
        let mut ledger = Ledger::new(Currency::USD);
        let ca = ledger.add_account("1500", "Equipment", AccountType::Asset, None, false).unwrap();
        let aa = ledger.add_account("1501", "Accum", AccountType::Asset, None, false).unwrap();
        let ex = ledger.add_account("6100", "Depr", AccountType::Expense, None, false).unwrap();
        let id = ledger.register_asset(NewAsset {
            description: "prop".to_string(),
            in_service_date: NaiveDate::from_ymd_opt(y, m, d).unwrap(),
            cost: Money::new(cost, Currency::USD),
            salvage_value: Money::new(salvage, Currency::USD),
            useful_life_months: life,
            cost_account: ca,
            accum_depr_account: aa,
            expense_account: ex,
        }).unwrap();
        let asset = ledger.get_asset(id).unwrap();
        let far = asset.in_service_date.checked_add_months(chrono::Months::new(life + 24)).unwrap();
        let sched = depreciation_schedule(asset, far).unwrap();
        // The lifetime total is exactly cost minus salvage: never a penny off.
        let total: Decimal = sched.iter().map(|l| l.amount).sum();
        prop_assert_eq!(total, cost - salvage);
        for l in &sched {
            prop_assert!(l.amount >= Decimal::ZERO);
            prop_assert_eq!(l.book_value + l.accumulated, cost);
        }
        // Accumulated is monotone non-decreasing.
        for w in sched.windows(2) {
            prop_assert!(w[1].accumulated >= w[0].accumulated);
        }
        // Book value at any date matches the schedule.
        if let Some(last) = sched.last() {
            prop_assert_eq!(scheduled_book_value(asset, far).unwrap(), last.book_value);
            prop_assert_eq!(accumulated_scheduled_through(asset, far).unwrap(), last.accumulated);
        }
    }
}

// ---------------------------------------------------------------------------
// Member splits (periods::allocate): conservation, a cent of each exact
// share, fairness, and sign symmetry. Rank 2: laws, not recomputation.
// ---------------------------------------------------------------------------

/// 1 to 5 positive shares summing to exactly 1 (the last takes the rest).
fn arb_shares() -> impl Strategy<Value = Vec<Decimal>> {
    prop::collection::vec(1u32..1000, 1..=5).prop_map(|weights| {
        let total: u32 = weights.iter().sum();
        let mut shares: Vec<Decimal> = weights
            .iter()
            .map(|w| Decimal::from(*w) / Decimal::from(total))
            .collect();
        let rest: Decimal = shares[..shares.len() - 1].iter().sum();
        *shares.last_mut().unwrap() = Decimal::ONE - rest;
        shares
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    #[test]
    fn prop_split_parts_sum_to_the_whole_within_a_cent_of_each_share(
        cents in -10_000_000_000i64..10_000_000_000, shares in arb_shares(),
    ) {
        let amount = Decimal::new(cents, 2);
        let parts = ledger_core::periods::allocate(amount, &shares);
        prop_assert_eq!(parts.len(), shares.len());
        prop_assert_eq!(parts.iter().copied().sum::<Decimal>(), amount);
        for (p, s) in parts.iter().zip(&shares) {
            prop_assert!((*p - amount * s).abs() < Decimal::new(1, 2), "{p} vs share {s}");
            prop_assert!(p.scale() <= 2 || p.normalize().scale() <= 2, "{p} is not whole cents");
        }
    }

    #[test]
    fn prop_split_never_rounds_up_a_smaller_remainder_over_a_larger(
        cents in 0i64..10_000_000_000, shares in arb_shares(),
    ) {
        let amount = Decimal::new(cents, 2);
        let parts = ledger_core::periods::allocate(amount, &shares);
        let exact: Vec<Decimal> = shares.iter().map(|s| amount * s).collect();
        let floor = |e: &Decimal| e.round_dp_with_strategy(2, rust_decimal::RoundingStrategy::ToZero);
        for i in 0..parts.len() {
            for j in 0..parts.len() {
                let (ri, rj) = (exact[i] - floor(&exact[i]), exact[j] - floor(&exact[j]));
                let (up_i, up_j) = (parts[i] > floor(&exact[i]), parts[j] > floor(&exact[j]));
                if ri > rj {
                    prop_assert!(up_i || !up_j, "member {j} rounded up over member {i}");
                }
            }
        }
    }

    #[test]
    fn prop_split_of_a_loss_is_the_negated_split_of_the_gain(
        cents in 0i64..10_000_000_000, shares in arb_shares(),
    ) {
        let amount = Decimal::new(cents, 2);
        let gain = ledger_core::periods::allocate(amount, &shares);
        let loss = ledger_core::periods::allocate(-amount, &shares);
        prop_assert_eq!(loss, gain.iter().map(|p| -*p).collect::<Vec<_>>());
    }
}

/// A fiscal year start as the books may hold one: any month and day 2001
/// has (a new setting is the first of a month; replay takes any day every
/// year has, rules.md B-28).
fn arb_fiscal_start() -> impl Strategy<Value = (u32, u32)> {
    (1u32..=12, 1u32..=31).prop_filter("a day 2001 has", |&(m, d)| {
        NaiveDate::from_ymd_opt(2001, m, d).is_some()
    })
}

/// A day in the books' calendar, 0001-01-01 through 9999-12-31.
fn arb_calendar_day() -> impl Strategy<Value = NaiveDate> {
    let span = (ledger_core::calendar::LAST_DAY - ledger_core::calendar::FIRST_DAY).num_days();
    prop_oneof![
        // Near the edges as often as anywhere: year 1 under a July year
        // starts in year 0 (clamped), 9999-12-31 is the last day.
        (0i64..800).prop_map(|n| ledger_core::calendar::FIRST_DAY + chrono::Days::new(n as u64)),
        (0i64..800).prop_map(|n| ledger_core::calendar::LAST_DAY - chrono::Days::new(n as u64)),
        (0i64..=span).prop_map(|n| ledger_core::calendar::FIRST_DAY + chrono::Days::new(n as u64)),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(5000))]

    /// rules.md B-30, PeriodDefaults INVARIANTS 1-5, for every fiscal
    /// year start the books hold, every today in the calendar, and every
    /// mix of given and omitted dates: the resolved dates are the given
    /// ones or the defaults, never swapped; Err exactly when the resolved
    /// start is after the resolved end, as InvalidDateRange when both
    /// were given and StartAfterToday when the end was defaulted.
    #[test]
    fn prop_period_defaults_resolve(
        md in arb_fiscal_start(),
        today in arb_calendar_day(),
        start in prop::option::of(arb_calendar_day()),
        end in prop::option::of(arb_calendar_day()),
    ) {
        use ledger_core::calendar::{FIRST_DAY, fiscal_year_start_on, period_defaults};
        use chrono::Datelike;
        let company = ledger_core::settings::Company {
            fiscal_year_start: md,
            ..Default::default()
        };
        let p = period_defaults(&company, today);
        let want_end = end.unwrap_or(today);
        let fy = fiscal_year_start_on(md, want_end);
        let want_start = start.unwrap_or(fy);
        // The fiscal year start of the end: on or before it, md in its
        // own year, and the next one after the end (unless year 0's,
        // clamped to the calendar's first day).
        prop_assert!(fy <= want_end && fy >= FIRST_DAY);
        let next = NaiveDate::from_ymd_opt(fy.year() + 1, md.0, md.1).unwrap();
        if (fy.month(), fy.day()) == md {
            prop_assert!(want_end < next, "{fy} {want_end}");
            let this = NaiveDate::from_ymd_opt(want_end.year(), md.0, md.1).unwrap();
            prop_assert_eq!(fy, if this <= want_end { this } else {
                NaiveDate::from_ymd_opt(want_end.year() - 1, md.0, md.1).unwrap()
            });
        } else {
            // Clamped: the year began in year 0.
            prop_assert_eq!(fy, FIRST_DAY);
            prop_assert!(want_end < NaiveDate::from_ymd_opt(1, md.0, md.1).unwrap());
        }
        match p.statement(start, end) {
            Ok((s, e)) => {
                prop_assert!(want_start <= want_end);
                prop_assert_eq!((s, e), (want_start, want_end));
            }
            Err(err) => {
                prop_assert!(want_start > want_end);
                let (s, e) = (want_start.to_string(), want_end.to_string());
                let expected = if end.is_some() {
                    ledger_core::LedgerError::InvalidDateRange { start: s, end: e }
                } else {
                    ledger_core::LedgerError::StartAfterToday { start: s, today: e }
                };
                prop_assert_eq!(err, expected);
            }
        }
        prop_assert_eq!(p.statement(None, None), Ok(p.statement_range()));
        prop_assert_eq!(p.as_of(None), today);
        prop_assert_eq!(p.as_of(start), start.unwrap_or(today));
        prop_assert_eq!(p.calendar_year(), today.year());
    }
}

// ---------------------------------------------------------------------------
// P3: depreciation by the day through the final month (defect 22), and
// schedules spread by day (defect 9). Each law is checked against arithmetic
// written here, never read back from the code.
// ---------------------------------------------------------------------------

use chrono::Datelike;
use ledger_core::schedules::{amortization_plan, daily_plan, month_end};
use ledger_core::{AssetStatus, disposal_amounts};

mod p3 {
    use super::*;
    use chrono::Months;

    /// A USD asset on a ledger, as a command registers it.
    pub fn asset_on(
        cost_cents: u64,
        salvage_cents: u64,
        life: u32,
        start: NaiveDate,
    ) -> (Ledger, ledger_core::AssetId) {
        let mut ledger = Ledger::new(Currency::USD);
        let ca = ledger
            .add_account("1500", "Equipment", AccountType::Asset, None, false)
            .unwrap();
        let aa = ledger
            .add_account("1501", "Accum", AccountType::Asset, None, false)
            .unwrap();
        let ex = ledger
            .add_account("6100", "Depr", AccountType::Expense, None, false)
            .unwrap();
        let id = ledger
            .register_asset(NewAsset {
                description: "prop".to_string(),
                in_service_date: start,
                cost: Money::new(Decimal::new(cost_cents as i64, 2), Currency::USD),
                salvage_value: Money::new(Decimal::new(salvage_cents as i64, 2), Currency::USD),
                useful_life_months: life,
                cost_account: ca,
                accum_depr_account: aa,
                expense_account: ex,
            })
            .unwrap();
        (ledger, id)
    }

    pub fn arb_day() -> impl Strategy<Value = NaiveDate> {
        (2000i32..=2100, 1u32..=12, 1u32..=31).prop_map(|(y, m, d)| {
            (1..=d)
                .rev()
                .find_map(|d| NaiveDate::from_ymd_opt(y, m, d))
                .unwrap()
        })
    }

    /// The days in `d`'s month.
    pub fn days_in(d: NaiveDate) -> u32 {
        month_end(d).day()
    }

    /// The last day of `d`'s month.
    pub fn month_end_of(d: NaiveDate) -> NaiveDate {
        month_end(d)
    }

    /// Before P3: the per-month running total in Decimal, rounded half away
    /// from zero to the cent, at each month end of the life (the final
    /// month's plug left out: it only ever set the life's end to cost −
    /// salvage, as P3 does). A copy of assets.rs as it was, the oracle for
    /// "no posted month moves".
    pub fn legacy_month_ends(
        depreciable: Decimal,
        life: u32,
        start: NaiveDate,
    ) -> Vec<(NaiveDate, Decimal)> {
        let end = start.checked_add_months(Months::new(life)).unwrap();
        let rate = depreciable / Decimal::from(life);
        let mut out = Vec::new();
        let mut exact = Decimal::ZERO;
        let mut month = start.with_day(1).unwrap();
        while month < end {
            let next = month.checked_add_months(Months::new(1)).unwrap();
            let held = (next.min(end) - start.max(month)).num_days();
            exact += rate * Decimal::from(held) / Decimal::from(days_in(month));
            out.push((
                month_end(month),
                exact.round_dp_with_strategy(
                    2,
                    rust_decimal::RoundingStrategy::MidpointAwayFromZero,
                ),
            ));
            month = next;
        }
        out
    }

    /// The exact cumulative charge through the end of the life's `i`th
    /// month (not the last), in cents, as a fraction: (numerator,
    /// denominator) of D × (months held, each days held over days in the
    /// month) / life, in whole MONTH_L-ths.
    pub fn exact_through(
        depreciable_cents: i128,
        life: u32,
        start: NaiveDate,
        through: NaiveDate,
    ) -> (i128, i128) {
        const L: i128 = 377_580;
        let mut elapsed = 0i128;
        let mut month = start.with_day(1).unwrap();
        while month <= through {
            let next = month.checked_add_months(Months::new(1)).unwrap();
            let from = start.max(month);
            let to = next.min(through.succ_opt().unwrap());
            elapsed += i128::from((to - from).num_days() as i32) * (L / i128::from(days_in(month)));
            month = next;
        }
        (depreciable_cents * elapsed, i128::from(life) * L)
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]

    /// depreciation_schedule INVARIANT 5 (rules.md B-15; defect 22): accumulated depreciation never falls,
    /// never exceeds cost − salvage, is 0 before the in-service date and
    /// cost − salvage from the life's last day on; each schedule's lines
    /// sum to its accumulated and none is negative. Sampled through: every
    /// day of the first and final months and of two months between, a
    /// dozen month ends, and the days around the life (P3 depreciation
    /// review R9: stepping every day of 600-month lives is P12's).
    #[test]
    fn prop_depreciation_is_monotone_and_bounded(
        cost_cents in 2u64..=100_000_000_000,
        salvage_frac in 0u64..1000,
        life in 1u32..=600,
        start in p3::arb_day(),
        picks in prop::collection::vec(0u32..600, 14),
    ) {
        let salvage_cents = (cost_cents - 1) * salvage_frac / 1000;
        let (ledger, id) = p3::asset_on(cost_cents, salvage_cents, life, start);
        let a = ledger.get_asset(id).unwrap();
        let dep = Decimal::new((cost_cents - salvage_cents) as i64, 2);
        let end = start.checked_add_months(chrono::Months::new(life)).unwrap();
        let last_day = end.pred_opt().unwrap();
        let month_of = |k: u32| start.with_day(1).unwrap().checked_add_months(chrono::Months::new(k)).unwrap();
        let mut days: Vec<NaiveDate> = vec![
            start.pred_opt().unwrap(), start, last_day, end, end + chrono::Days::new(400),
        ];
        let months = life;
        for k in [0, months, picks[0] % (months + 1), picks[1] % (months + 1)] {
            let m = month_of(k);
            days.extend((0..p3::days_in(m)).map(|i| m + chrono::Days::new(u64::from(i))));
        }
        days.extend(picks[2..].iter().map(|&k| p3::month_end_of(month_of(k % (months + 1)))));
        days.sort();
        days.dedup();
        let mut prev = Decimal::ZERO;
        for day in days {
            let s = depreciation_schedule(a, day).unwrap();
            let acc = accumulated_scheduled_through(a, day).unwrap();
            prop_assert_eq!(s.iter().map(|l| l.amount).sum::<Decimal>(), acc);
            prop_assert!(s.iter().all(|l| l.amount >= Decimal::ZERO));
            prop_assert!(acc >= prev, "{}: {} < {}", day, acc, prev);
            prop_assert!(acc <= dep);
            if day < start {
                prop_assert_eq!(acc, Decimal::ZERO);
            }
            if day >= last_day {
                prop_assert_eq!(acc, dep);
            }
            prev = acc;
        }
    }

    /// Defect 22, reproduction B as a law: a disposed asset's schedule
    /// shows exactly what its disposal computed, for any disposal day in or
    /// after the life, and is unchanged through any later date.
    #[test]
    fn prop_disposed_schedule_equals_what_disposal_computed(
        cost_cents in 2u64..=100_000_000_000,
        salvage_frac in 0u64..1000,
        life in 1u32..=600,
        start in p3::arb_day(),
        after in 0u64..20_000,
        later in 0u64..1_000,
    ) {
        let salvage_cents = (cost_cents - 1) * salvage_frac / 1000;
        let (mut ledger, id) = p3::asset_on(cost_cents, salvage_cents, life, start);
        let on = start + chrono::Days::new(after);
        let proceeds = Money::new(Decimal::ONE, Currency::USD);
        let want = disposal_amounts(ledger.get_asset(id).unwrap(), on, proceeds).unwrap();
        if ledger_core::calendar::check(on).is_err() {
            return Ok(());
        }
        ledger.dispose_asset(id, on, proceeds).unwrap();
        let a = ledger.get_asset(id).unwrap();
        let disposed = matches!(a.status, AssetStatus::Disposed { .. });
        prop_assert!(disposed);
        let full = depreciation_schedule(a, NaiveDate::MAX).unwrap();
        prop_assert_eq!(full.last().map_or(Decimal::ZERO, |l| l.accumulated), want.accumulated);
        prop_assert_eq!(
            accumulated_scheduled_through(a, on + chrono::Days::new(later)).unwrap(),
            want.accumulated
        );
        prop_assert_eq!(scheduled_book_value(a, on).unwrap(), want.book_value);
    }

    /// B-15 rule 3 (defect 22), against arithmetic written here: through
    /// any day k of the life's final month (its d days), accumulated is C +
    /// (D − C) × k / d rounded half up to the cent, C the exact charge of
    /// the months before (D × each month's days held over its days, over
    /// the life), D cost less salvage; on its last day exactly D. So the
    /// final month is prorated by its days, never plugged.
    #[test]
    fn prop_the_final_month_takes_the_remainder_by_its_days(
        cost_cents in 2u64..=100_000_000_000,
        salvage_frac in 0u64..1000,
        life in 1u32..=600,
        start in p3::arb_day(),
    ) {
        let salvage_cents = (cost_cents - 1) * salvage_frac / 1000;
        let (ledger, id) = p3::asset_on(cost_cents, salvage_cents, life, start);
        let a = ledger.get_asset(id).unwrap();
        let dep = i128::from(cost_cents - salvage_cents);
        let end = start.checked_add_months(chrono::Months::new(life)).unwrap();
        let last_day = end.pred_opt().unwrap();
        let final_first = last_day.with_day(1).unwrap().max(start);
        let d = i128::from((end - final_first).num_days() as i32);
        let (num, den) = p3::exact_through(dep, life, start, final_first.pred_opt().unwrap());
        for k in 1..=d {
            let through = final_first + chrono::Days::new((k - 1) as u64);
            // C + (D − C) k / d = (num d + (D den − num) k) / (den d).
            let (x, y) = (num * d + (dep * den - num) * k, den * d);
            let want = if k == d {
                dep
            } else {
                x / y + i128::from(2 * (x % y) >= y)
            };
            prop_assert_eq!(
                accumulated_scheduled_through(a, through).unwrap(),
                Decimal::new(want as i64, 2),
                "{} of {}", through, last_day
            );
        }
    }

    /// B-15 rule 4 (depreciation_schedule INVARIANT 6; P3 depreciation brief 2.3, R1, R8): through every
    /// month end of the life, the schedule is the pre-P3 running total
    /// (USD, a two-place currency), except where the exact cumulative is a
    /// half-cent tie, which the old 28-digit Decimal quotients could round
    /// either way: there the new figure is the exact one, half a cent up.
    /// So no posted month moves, but a tie's cent.
    #[test]
    fn prop_month_end_schedules_match_the_legacy_arithmetic_except_half_cent_ties(
        cost_cents in 2u64..=100_000_000_000,
        salvage_frac in 0u64..1000,
        life in 1u32..=600,
        start in p3::arb_day(),
    ) {
        let salvage_cents = (cost_cents - 1) * salvage_frac / 1000;
        let (ledger, id) = p3::asset_on(cost_cents, salvage_cents, life, start);
        let a = ledger.get_asset(id).unwrap();
        let dep_cents = i128::from(cost_cents - salvage_cents);
        let dep = Decimal::new(dep_cents as i64, 2);
        let legacy = p3::legacy_month_ends(dep, life, start);
        let new = depreciation_schedule(a, NaiveDate::MAX).unwrap();
        prop_assert_eq!(legacy.len(), new.len());
        let n = new.len();
        for (i, ((at, old), line)) in legacy.iter().zip(&new).enumerate() {
            if i + 1 == n {
                // The final month: cost − salvage, then (the plug) as now.
                prop_assert_eq!(line.accumulated, dep);
                continue;
            }
            if line.accumulated != *old {
                let (num, den) = p3::exact_through(dep_cents, life, start, *at);
                // A half-cent tie: num/den = q + 1/2 exactly.
                prop_assert_eq!(2 * (num % den), den, "{}: {} vs {}, not a tie", at, line.accumulated, old);
                let up = Decimal::new(((num / den) + 1) as i64, 2);
                prop_assert_eq!(line.accumulated, up);
                prop_assert_eq!((line.accumulated - *old).abs(), Decimal::new(1, 2));
            }
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    /// daily_plan INVARIANTS 1-3 (defect 9): the plan sums to the total
    /// exactly, every amount is > 0, the dates are distinct month ends
    /// (ascending) from the start's month through the last day's; from the
    /// 1st with at least a cent a month, there are `months` postings, each
    /// but the last within a cent of total/months, and the last takes the
    /// rest.
    #[test]
    fn prop_daily_plan_sums_exactly_and_lands_on_month_ends(
        total_cents in 1i64..=100_000_000_000,
        start in p3::arb_day(),
        months in 1u32..=600,
        on_the_first in any::<bool>(),
    ) {
        let start = if on_the_first { start.with_day(1).unwrap() } else { start };
        let total = Money::new(Decimal::new(total_cents, 2), Currency::USD);
        let plan = daily_plan(total, start, months).unwrap();
        let last_day = start.checked_add_months(chrono::Months::new(months)).unwrap().pred_opt().unwrap();
        prop_assert_eq!(plan.iter().map(|(_, a)| *a).sum::<Decimal>(), total.amount);
        prop_assert!(plan.iter().all(|(_, a)| *a > Decimal::ZERO));
        prop_assert!(plan.windows(2).all(|w| w[0].0 < w[1].0));
        for (d, _) in &plan {
            prop_assert_eq!(*d, month_end(*d));
            prop_assert!(*d >= month_end(start) && *d <= month_end(last_day));
        }
        if on_the_first && total_cents >= i64::from(months) {
            prop_assert_eq!(plan.len() as u32, months);
            let even = total.amount / Decimal::from(months);
            for (_, a) in &plan[..plan.len() - 1] {
                prop_assert!((*a - even).abs() <= Decimal::new(1, 2), "{} vs {}", a, even);
            }
            let others: Decimal = plan[..plan.len() - 1].iter().map(|(_, a)| *a).sum();
            prop_assert_eq!(plan.last().unwrap().1, total.amount - others);
        }
    }

    /// The equal-months plan is unchanged forever (a posted month's replay
    /// re-derives it): a copy of amortization_plan as it was before P3.
    #[test]
    fn prop_monthly_even_plan_is_unchanged(
        total_cents in 1i64..=100_000_000_000,
        start in p3::arb_day(),
        months in 1u32..=600,
    ) {
        let total = Decimal::new(total_cents, 2);
        let each = (total / Decimal::from(months)).round_dp(2);
        let first = start.with_day(1).unwrap();
        let want: Vec<(NaiveDate, Decimal)> = (0..months)
            .map(|i| {
                let m = first.checked_add_months(chrono::Months::new(i)).unwrap();
                let a = if i + 1 == months { total - each * Decimal::from(months - 1) } else { each };
                (month_end(m), a)
            })
            .collect();
        prop_assert_eq!(amortization_plan(total, start, months).unwrap(), want);
    }
}

/// The half-cent tie the P3 depreciation review (R8) names: 1,000.03 over
/// 36 months from 2026-01-01. Through month 18 the exact figure is
/// 100,003 × 18/36 = 50,001.5 cents, a tie: P3 rounds it up, 500.02, and
/// month 19 (100,003 × 19/36 = 52,779.36 cents) is 527.79. The old
/// running total in 28-digit Decimal lands a hair under the tie and gives
/// 500.01, so that cent moves from month 19 to month 18 (the change-log
/// case); every other month end is the same, and the life ends at
/// 1,000.03 either way.
#[test]
fn a_half_cent_tie_rounds_up_exactly() {
    let start = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let (ledger, id) = p3::asset_on(100_003, 0, 36, start);
    let new = depreciation_schedule(ledger.get_asset(id).unwrap(), NaiveDate::MAX).unwrap();
    assert_eq!(new[17].accumulated, Decimal::new(50_002, 2));
    assert_eq!(new[18].accumulated, Decimal::new(52_779, 2));
    assert_eq!(new[35].accumulated, Decimal::new(100_003, 2));
    let old = p3::legacy_month_ends(Decimal::new(100_003, 2), 36, start);
    let moved: Vec<usize> = (0..35)
        .filter(|&i| old[i].1 != new[i].accumulated)
        .collect();
    assert_eq!(moved, [17], "only the tie moves");
    assert_eq!(old[17].1, Decimal::new(50_001, 2));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]

    /// assets::run_amounts INVARIANT 1 (P3 review round 2: it had no test
    /// but the viewer's): what it says a run of each month posts, and what
    /// the books carry past the schedule then, is run_amount's for that
    /// month, over any runs and reversals (each in its own month, a
    /// reversal negative), through any month.
    #[test]
    fn prop_run_amounts_are_each_months_run_amount(
        cost_cents in 2u64..=10_000_000,
        life in 1u32..=24,
        start in p3::arb_day(),
        effects in prop::collection::vec((0u32..30, -500_000i64..=500_000), 0..8),
        through in 0u32..36,
    ) {
        let (ledger, id) = p3::asset_on(cost_cents, 0, life, start);
        let a = ledger.get_asset(id).unwrap();
        let first = start.with_day(1).unwrap();
        let ym = |i: u32| {
            let d = first.checked_add_months(chrono::Months::new(i)).unwrap();
            (chrono::Datelike::year(&d), chrono::Datelike::month(&d))
        };
        let posted: Vec<ledger_core::PostedRun> = effects
            .iter()
            .map(|&(i, cents)| {
                let (year, month) = ym(i);
                ledger_core::PostedRun { year, month, amount: Decimal::new(cents, 2) }
            })
            .collect();
        let rows = ledger_core::run_amounts(a, ym(through), &posted).unwrap();
        prop_assert!(!rows.is_empty());
        prop_assert_eq!((rows[0].0, rows[0].1), ym(0));
        for (k, &(y, m, r)) in rows.iter().enumerate() {
            prop_assert_eq!((y, m), ym(k as u32), "the months in turn");
            let one = ledger_core::run_amount(a, y, m, &posted).unwrap();
            prop_assert_eq!(r, one, "{}-{:02}", y, m);
        }
    }
}

fn arb_basis_rule() -> impl Strategy<Value = ledger_core::BasisRule> {
    prop_oneof![
        Just(ledger_core::BasisRule::Purchase),
        Just(ledger_core::BasisRule::Carryover723),
        Just(ledger_core::BasisRule::PersonalUseConversion),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(5000))]

    /// kernel::basis_figures INVARIANTS 1-4 on Decimal money (rules.md
    /// B-25): a purchase depreciates from cost and has no §723 basis or
    /// built-in gain; a contribution's §723 basis is the adjusted basis,
    /// its built-in gain fair market value less that basis, negative
    /// exactly when the basis exceeds the value; carryover depreciates
    /// from the basis, personal use from the lesser (at most each, equal
    /// to one).
    #[test]
    fn prop_basis_figures_laws(
        rule in arb_basis_rule(),
        cost in 1i64..=100_000_000_000,
        adjusted in 0i64..=100_000_000_000,
        fmv in 0i64..=100_000_000_000,
    ) {
        use ledger_core::BasisRule::*;
        let (cost, adjusted, fmv) = (Decimal::new(cost, 2), Decimal::new(adjusted, 2), Decimal::new(fmv, 2));
        let f = ledger_core::kernel::basis_figures(rule, cost, adjusted, fmv).unwrap();
        match rule {
            Purchase => {
                prop_assert_eq!(f.depreciable_basis, cost);
                prop_assert_eq!(f.tax_basis_723, None);
                prop_assert_eq!(f.built_in_gain_704c, None);
            }
            Carryover723 | PersonalUseConversion => {
                prop_assert_eq!(f.tax_basis_723, Some(adjusted));
                let g = f.built_in_gain_704c.unwrap();
                prop_assert_eq!(g, fmv - adjusted);
                prop_assert_eq!(g < Decimal::ZERO, adjusted > fmv);
                if rule == Carryover723 {
                    prop_assert_eq!(f.depreciable_basis, adjusted);
                } else {
                    prop_assert!(f.depreciable_basis <= adjusted && f.depreciable_basis <= fmv);
                    prop_assert!(f.depreciable_basis == adjusted || f.depreciable_basis == fmv);
                }
            }
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// rules.md B-37 item 3: for any tags and any period, the projects'
    /// figures plus the untagged lines' are the period's profit and loss
    /// (no line counted twice or lost), each through
    /// ledger_core::project_figures.
    #[test]
    fn prop_project_nets_partition_the_pnl(
        entries in prop::collection::vec(
            (0u32..120, 1i64..1_000_000, any::<bool>(), 0u32..4, 0u32..4),
            1..20,
        ),
        from in 0u32..120,
        len in 0u32..120,
    ) {
        let base = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let mut l = Ledger::new(Currency::USD);
        let cash = l.add_account("1000", "Cash", AccountType::Asset, None, false).unwrap();
        let rev = l.add_account("4000", "Revenue", AccountType::Revenue, None, false).unwrap();
        let exp = l.add_account("6000", "Costs", AccountType::Expense, None, false).unwrap();
        let classes: Vec<u32> = (0..3).map(|i| l.add_class(&format!("P{i}")).unwrap()).collect();
        for (day, c, income, tag_a, tag_b) in entries {
            let e = l.create_entry(base + chrono::Days::new(u64::from(day)), "e");
            let amt = Money::new(Decimal::new(c, 2), Currency::USD);
            let (dr, cr) = if income { (cash, rev) } else { (exp, cash) };
            l.add_line(e, dr, Side::Debit, amt).unwrap();
            l.add_line(e, cr, Side::Credit, amt).unwrap();
            // 3 means untagged; both lines may carry any project.
            for (i, t) in [tag_a, tag_b].into_iter().enumerate() {
                if let Some(&k) = classes.get(t as usize) {
                    l.set_line_class(e, i, Some(k)).unwrap();
                }
            }
            l.post_entry(e).unwrap();
        }
        let start = base + chrono::Days::new(u64::from(from));
        let end = start + chrono::Days::new(u64::from(len));
        let all = profit_and_loss_for_period(&l, start, end, None).unwrap();
        let mut rev_sum = Decimal::ZERO;
        let mut exp_sum = Decimal::ZERO;
        for &k in &classes {
            let f = ledger_core::project_figures(&l, k, start, end).unwrap();
            rev_sum += f.period.total_revenue;
            exp_sum += f.period.total_expenses;
            let months: Decimal = f.months.iter().map(|m| m.net()).sum();
            prop_assert_eq!(months, f.period.net_income());
        }
        // The untagged lines: every line with no class.
        let mut untagged_rev = Decimal::ZERO;
        let mut untagged_exp = Decimal::ZERO;
        for e in l.posted_entries() {
            if e.date() < start || e.date() > end {
                continue;
            }
            for line in e.lines().iter().filter(|x| x.class_id.is_none()) {
                let a = line.amount.amount;
                if line.account == rev {
                    untagged_rev += if line.side == Side::Credit { a } else { -a };
                } else if line.account == exp {
                    untagged_exp += if line.side == Side::Debit { a } else { -a };
                }
            }
        }
        prop_assert_eq!(rev_sum + untagged_rev, all.total_revenue);
        prop_assert_eq!(exp_sum + untagged_exp, all.total_expenses);
    }

    /// rules.md B-37 item 7: the snapshot's contribution is
    /// project_to_date's net, revenue less direct costs, whatever the
    /// tags and sides (a refund debits revenue, a reimbursement credits an
    /// expense below zero): the accounts sum to the totals, each month's
    /// net to date is project_to_date through its end (the last is the
    /// net to date), and a margin exists exactly when revenue is above
    /// zero, with the contribution's sign.
    #[test]
    fn prop_project_contribution_is_the_net_to_date(
        entries in prop::collection::vec(
            (0u32..400, 1i64..1_000_000, 0u32..4, any::<bool>(), 0u32..3, 0u32..3),
            0..20,
        ),
        from in 0u32..400,
        len in 0u32..400,
    ) {
        let base = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let mut l = Ledger::new(Currency::USD);
        let cash = l.add_account("1000", "Cash", AccountType::Asset, None, false).unwrap();
        let rev = l.add_account("4000", "Revenue", AccountType::Revenue, None, false).unwrap();
        let exp = l.add_account("6000", "Costs", AccountType::Expense, None, false).unwrap();
        let soft = l.add_account("6100", "Software", AccountType::Expense, None, false).unwrap();
        let classes: Vec<u32> = (0..2).map(|i| l.add_class(&format!("P{i}")).unwrap()).collect();
        let accounts = [cash, rev, exp, soft];
        // Each tagged revenue or expense line: (date, class).
        let mut tagged = Vec::new();
        for (day, c, other, debit_first, tag_a, tag_b) in entries {
            let date = base + chrono::Days::new(u64::from(day));
            let e = l.create_entry(date, "e");
            let amt = Money::new(Decimal::new(c, 2), Currency::USD);
            let other = accounts[other as usize];
            let (dr, cr) = if debit_first { (other, cash) } else { (cash, other) };
            l.add_line(e, dr, Side::Debit, amt).unwrap();
            l.add_line(e, cr, Side::Credit, amt).unwrap();
            for (i, t) in [tag_a, tag_b].into_iter().enumerate() {
                if let Some(&k) = classes.get(t as usize) {
                    l.set_line_class(e, i, Some(k)).unwrap();
                    if [dr, cr][i] != cash {
                        tagged.push((date, k));
                    }
                }
            }
            l.post_entry(e).unwrap();
        }
        let start = base + chrono::Days::new(u64::from(from));
        let end = start + chrono::Days::new(u64::from(len));
        for &k in &classes {
            let t = ledger_core::project_to_date(&l, k, end).unwrap();
            let lines = |v: &[ledger_core::StatementLine]| v.iter().map(|x| x.amount).sum::<Decimal>();
            prop_assert_eq!(lines(&t.revenue_by_account), t.revenue);
            prop_assert_eq!(lines(&t.expenses_by_account), t.expenses);
            prop_assert_eq!(t.net(), t.revenue - t.expenses);
            match t.margin_pct() {
                None => prop_assert!(t.revenue <= Decimal::ZERO),
                Some(m) => {
                    prop_assert!(t.revenue > Decimal::ZERO);
                    prop_assert_eq!(m.scale(), 1);
                    // Its sign is the contribution's (a tiny one rounds to 0.0).
                    if t.net() >= Decimal::ZERO {
                        prop_assert!(m >= Decimal::ZERO);
                    } else {
                        prop_assert!(m <= Decimal::ZERO);
                    }
                }
            }
            // Whether anything is tagged counts lines, even ones netting
            // to zero, never accounts left with a balance.
            let count = |from: NaiveDate| {
                tagged.iter().filter(|(d, c)| *c == k && *d >= from && *d <= end).count()
            };
            prop_assert_eq!(t.revenue_lines + t.expenses_lines, count(NaiveDate::MIN));
            let f = ledger_core::project_figures(&l, k, start, end).unwrap();
            prop_assert_eq!(f.period_lines, count(start));
            prop_assert_eq!(&f.to_date, &t);
            prop_assert_eq!(f.months_net_to_date.len(), f.months.len());
            for (m, running) in f.months.iter().zip(&f.months_net_to_date) {
                let at = ledger_core::project_to_date(&l, k, m.end).unwrap();
                prop_assert_eq!(*running, at.net());
            }
            if let Some(last) = f.months_net_to_date.last() {
                prop_assert_eq!(*last, t.net());
            }
            prop_assert_eq!(
                f.period_margin_pct(),
                ledger_core::contribution_margin_pct(f.period.net_income(), f.period.total_revenue)
            );
        }
    }

    /// rules.md B-37 item 3: project_entries codes its own signs, so the
    /// posted entries' project amounts must sum to project_to_date's net
    /// (with no entry dated after the day), whatever the tags, the sides,
    /// a tagged balance-sheet line, or drafts beside them.
    #[test]
    fn prop_project_entries_sum_to_the_net_to_date(
        entries in prop::collection::vec(
            (0u32..120, 1i64..1_000_000, 0u32..3, any::<bool>(), 0u32..3, 0u32..3, any::<bool>()),
            1..20,
        ),
    ) {
        let base = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let mut l = Ledger::new(Currency::USD);
        let cash = l.add_account("1000", "Cash", AccountType::Asset, None, false).unwrap();
        let rev = l.add_account("4000", "Revenue", AccountType::Revenue, None, false).unwrap();
        let exp = l.add_account("6000", "Costs", AccountType::Expense, None, false).unwrap();
        let classes: Vec<u32> = (0..2).map(|i| l.add_class(&format!("P{i}")).unwrap()).collect();
        let accounts = [cash, rev, exp];
        for (day, c, other, debit_first, tag_a, tag_b, post) in entries {
            let e = l.create_entry(base + chrono::Days::new(u64::from(day)), "e");
            let amt = Money::new(Decimal::new(c, 2), Currency::USD);
            // Cash against any account, on either side: a refund debits
            // revenue, a reimbursement credits an expense.
            let other = accounts[other as usize];
            let (dr, cr) = if debit_first { (other, cash) } else { (cash, other) };
            l.add_line(e, dr, Side::Debit, amt).unwrap();
            l.add_line(e, cr, Side::Credit, amt).unwrap();
            for (i, t) in [tag_a, tag_b].into_iter().enumerate() {
                if let Some(&k) = classes.get(t as usize) {
                    l.set_line_class(e, i, Some(k)).unwrap();
                }
            }
            if post {
                l.post_entry(e).unwrap();
            }
        }
        let through = base + chrono::Days::new(200);
        for &k in &classes {
            let posted: Decimal = ledger_core::project_entries(&l, k)
                .into_iter()
                .filter(|(e, _)| {
                    l.get_entry(*e).unwrap().status() == ledger_core::EntryStatus::Posted
                })
                .map(|(_, a)| a)
                .sum();
            let t = ledger_core::project_to_date(&l, k, through).unwrap();
            prop_assert_eq!(posted, t.net());
        }
    }
}
