//! Property tests for the ledger engine: invariants that must hold for
//! *every* input, not just the hand-picked examples in `invariants.rs`.
//!
//! These are the properties mutation testing will try to break: if a
//! mutant survives the example tests but breaks a property, the property
//! catches it.

use chrono::NaiveDate;
use ledger_core::{
    AccountType, Currency, Ledger, Money, NewAsset, Side, accumulated_depreciation, book_value,
    depreciation_schedule,
    reports::{
        balance_sheet, balance_sheet_as_of, profit_and_loss_for_period, trial_balance,
        unclosed_earnings,
    },
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
            purchase_date: NaiveDate::from_ymd_opt(y, m, d).unwrap(),
            cost: Money::new(cost, Currency::USD),
            salvage_value: Money::new(salvage, Currency::USD),
            useful_life_months: life,
            cost_account: ca,
            accum_depr_account: aa,
            expense_account: ex,
        }).unwrap();
        let asset = ledger.get_asset(id).unwrap();
        let far = asset.purchase_date.checked_add_months(chrono::Months::new(life + 24)).unwrap();
        let sched = depreciation_schedule(asset, far);
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
            prop_assert_eq!(book_value(asset, far), last.book_value);
            prop_assert_eq!(accumulated_depreciation(asset, far), last.accumulated);
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
