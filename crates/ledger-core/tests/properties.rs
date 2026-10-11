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
    reports::{balance_sheet, trial_balance},
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
