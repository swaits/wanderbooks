//! Entries bridge properties: the small amount-generic pieces the
//! statement of cash flows builds on.
//!
//! `kernel::take`, `entry_movements` and `flows_of` at `i64` cents (what
//! Kani proves, law 10) against the same on `Decimal` (what the engine
//! runs), on random lines.

use super::{arb_cents, dec};
use ledger_core::Side;
use ledger_core::kernel::{self, Flow};
use proptest::prelude::*;

fn arb_flow() -> impl Strategy<Value = Flow> {
    prop_oneof![
        Just(Flow::Cash),
        Just(Flow::Operating),
        Just(Flow::Investing),
        Just(Flow::Financing),
    ]
}

fn arb_side() -> impl Strategy<Value = Side> {
    prop_oneof![Just(Side::Debit), Just(Side::Credit)]
}

fn arb_flow_lines() -> impl Strategy<Value = Vec<(Flow, Side, i64)>> {
    prop::collection::vec((arb_flow(), arb_side(), arb_cents()), 0..6)
}

fn to_dec(lines: &[(Flow, Side, i64)]) -> Vec<(Flow, Side, rust_decimal::Decimal)> {
    lines.iter().map(|&(f, s, a)| (f, s, dec(a))).collect()
}

fn arb_direction() -> impl Strategy<Value = kernel::Direction> {
    prop_oneof![
        Just(kernel::Direction::Paid),
        Just(kernel::Direction::Received),
        Just(kernel::Direction::Neither),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    /// `kernel::take` takes the same part of a movement at both types.
    #[test]
    fn take_agrees(remaining in arb_cents(), movement in arb_cents()) {
        prop_assert_eq!(
            kernel::take(remaining, movement).map(dec),
            kernel::take(dec(remaining), dec(movement))
        );
    }

    /// `kernel::entry_movements` nets the cash and each section the same.
    #[test]
    fn entry_movements_agree(lines in arb_flow_lines()) {
        let i = kernel::entry_movements(&lines)
            .map(|(cash, moved)| (dec(cash), moved.map(dec)));
        let d = kernel::entry_movements(&to_dec(&lines));
        prop_assert_eq!(i, d);
    }

    /// `kernel::flows_of` splits the cash the same, and refuses the same
    /// lines that do not net to zero.
    #[test]
    fn flows_of_agree(
        cash in arb_cents(),
        moved in (arb_cents(), arb_cents(), arb_cents())
            .prop_map(|(a, b, c)| [a, b, c]),
    ) {
        let i = kernel::flows_of(cash, moved);
        let d = kernel::flows_of(dec(cash), moved.map(dec));
        prop_assert_eq!(
            i.map(|f| (dec(f.cash), f.flows.map(dec), f.noncash.map(dec))),
            d.map(|f| (f.cash, f.flows, f.noncash))
        );
    }

    /// `kernel::catch_ups` gives every suffix's catch-up at both types.
    #[test]
    fn catch_ups_agree(
        pairs in prop::collection::vec((arb_cents(), arb_cents()), 0..6),
    ) {
        let scheduled: Vec<i64> = pairs.iter().map(|&(s, _)| s).collect();
        let posted: Vec<i64> = pairs.iter().map(|&(_, p)| p).collect();
        let mut i = vec![0i64; scheduled.len()];
        let mut d = vec![dec(0); scheduled.len()];
        let ri = kernel::catch_ups(&scheduled, &posted, &mut i);
        let rd = kernel::catch_ups(
            &scheduled.iter().map(|&c| dec(c)).collect::<Vec<_>>(),
            &posted.iter().map(|&c| dec(c)).collect::<Vec<_>>(),
            &mut d,
        );
        prop_assert_eq!(ri, rd);
        prop_assert_eq!(i.into_iter().map(dec).collect::<Vec<_>>(), d);
    }

    /// `kernel::unrecovered_cost` is the same shortfall at both types.
    #[test]
    fn unrecovered_cost_agree(revenue in arb_cents(), expenses in arb_cents()) {
        prop_assert_eq!(
            kernel::unrecovered_cost(revenue, expenses).map(dec),
            kernel::unrecovered_cost(dec(revenue), dec(expenses))
        );
    }

    /// `kernel::Direction::of` classifies the same sign at both types, and
    /// each case keeps its meaning (law 23).
    #[test]
    fn direction_of_agrees(cents in arb_cents()) {
        let i = kernel::Direction::of(cents);
        let d = kernel::Direction::of(dec(cents));
        prop_assert_eq!(i, d);
        if cents > 0 {
            prop_assert_eq!(i, kernel::Direction::Paid);
        } else if cents < 0 {
            prop_assert_eq!(i, kernel::Direction::Received);
        } else {
            prop_assert_eq!(i, kernel::Direction::Neither);
        }
    }

    /// `kernel::reverse_target` is the exact opposite at both types, and
    /// each refuses the same out-of-range inputs (law 24).
    #[test]
    fn reverse_target_agrees(
        amount in arb_cents(),
        cash in arb_cents(),
        in_kind in arb_cents(),
        direction in arb_direction(),
    ) {
        let t = kernel::PartyTarget {
            amount,
            direction,
            cash,
            in_kind,
        };
        let d = kernel::PartyTarget {
            amount: dec(amount),
            direction,
            cash: dec(cash),
            in_kind: dec(in_kind),
        };
        let as_dec = |r: kernel::PartyTarget<i64>| kernel::PartyTarget {
            amount: dec(r.amount),
            direction: r.direction,
            cash: dec(r.cash),
            in_kind: dec(r.in_kind),
        };
        prop_assert_eq!(
            kernel::reverse_target(t).map(as_dec),
            kernel::reverse_target(d)
        );
    }

    /// `kernel::part_weight` is the same weight at i32, i64 and i128, and
    /// `None` exactly for a part no calendar month has (laws 26-28).
    #[test]
    fn part_weight_agrees(month_days in 0u32..=40, days in 0u32..=40) {
        let part = kernel::MonthPart { days, month_days };
        let w32: Option<i32> = kernel::part_weight(part);
        let w64: Option<i64> = kernel::part_weight(part);
        let w128: Option<i128> = kernel::part_weight(part);
        prop_assert_eq!(w64, w32.map(i64::from));
        prop_assert_eq!(w128, w64.map(i128::from));
        if part.is_valid() {
            prop_assert_eq!(
                w64,
                Some(i64::from(days) * i64::from(kernel::MONTH_L / month_days))
            );
        } else {
            prop_assert_eq!(w64, None);
        }
    }
}
