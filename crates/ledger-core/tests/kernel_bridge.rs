//! The bridge between the proofs and the engine.
//!
//! The Kani harnesses (`src/proofs.rs`) prove the accounting kernel on
//! `i64` money in the smallest unit; the engine runs the same kernel on
//! `Decimal`. These properties check the two instantiations agree, value
//! for value, on cents: whatever Kani proved of the `i64` kernel holds of
//! the `Decimal` one for the same amounts.

use ledger_core::kernel::{self, Amount, LineError};
use ledger_core::{AccountType, Side};
use proptest::prelude::*;
use rust_decimal::Decimal;

/// Cents within both instantiations' line range.
const MAX_CENTS: i64 = 1 << 40;

fn dec(cents: i64) -> Decimal {
    Decimal::new(cents, 2)
}

fn arb_side() -> impl Strategy<Value = Side> {
    prop_oneof![Just(Side::Debit), Just(Side::Credit)]
}

fn arb_type() -> impl Strategy<Value = AccountType> {
    prop_oneof![
        Just(AccountType::Asset),
        Just(AccountType::Liability),
        Just(AccountType::Equity),
        Just(AccountType::Revenue),
        Just(AccountType::Expense),
    ]
}

fn arb_cents() -> impl Strategy<Value = i64> {
    prop_oneof![
        -MAX_CENTS..=MAX_CENTS,
        // Small amounts collide, so balanced entries and zeros show up.
        -3i64..=3,
    ]
}

/// Lines over four accounts. Balanced entries are rare at random, so half
/// the time the last line plugs the difference.
fn arb_lines() -> impl Strategy<Value = Vec<(u8, Side, i64)>> {
    (
        prop::collection::vec((0u8..4, arb_side(), arb_cents()), 0..7),
        any::<bool>(),
    )
        .prop_map(|(mut lines, plug)| {
            if plug && !lines.is_empty() {
                let net: i64 = lines[..lines.len() - 1]
                    .iter()
                    .map(|&(_, s, a)| if s == Side::Debit { a } else { -a })
                    .sum();
                let last = lines.len() - 1;
                if net.abs() > MAX_CENTS {
                    // Out of both ranges, where the two differ by design.
                    return lines;
                }
                lines[last].1 = if net > 0 { Side::Credit } else { Side::Debit };
                lines[last].2 = net.abs();
            }
            lines
        })
}

fn to_dec(lines: &[(u8, Side, i64)]) -> Vec<(u8, Side, Decimal)> {
    lines.iter().map(|&(k, s, a)| (k, s, dec(a))).collect()
}

fn sides<T: Copy>(lines: &[(u8, Side, T)]) -> Vec<(Side, T)> {
    lines.iter().map(|&(_, s, a)| (s, a)).collect()
}

fn map_err(e: LineError<i64>) -> LineError<Decimal> {
    match e {
        LineError::TooFewLines => LineError::TooFewLines,
        LineError::ZeroAmount => LineError::ZeroAmount,
        LineError::NegativeAmount => LineError::NegativeAmount,
        LineError::OutOfRange => LineError::OutOfRange,
        LineError::Unbalanced { debits, credits } => LineError::Unbalanced {
            debits: dec(debits),
            credits: dec(credits),
        },
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    #[test]
    fn check_lines_agrees(lines in arb_lines()) {
        let i = kernel::check_lines(&sides(&lines)).map_err(map_err);
        let d = kernel::check_lines(&sides(&to_dec(&lines)));
        prop_assert_eq!(i, d);
    }

    #[test]
    fn sums_agree(lines in arb_lines()) {
        let (i, d) = (sides(&lines), sides(&to_dec(&lines)));
        let ti = kernel::totals(i.iter().copied()).unwrap();
        let td = kernel::totals(d.iter().copied()).unwrap();
        prop_assert_eq!((dec(ti.0), dec(ti.1)), td);
        prop_assert_eq!(
            kernel::net(i.iter().copied()).map(dec),
            kernel::net(d.iter().copied())
        );
        let mi: Vec<_> = kernel::movements(lines.iter().copied())
            .unwrap()
            .into_iter()
            .map(|(k, m)| (k, dec(m)))
            .collect();
        prop_assert_eq!(Some(mi), kernel::movements(to_dec(&lines)));
    }

    #[test]
    fn reversal_agrees(lines in arb_lines()) {
        let ri: Vec<_> = kernel::reversal(&lines)
            .into_iter()
            .map(|(k, s, a)| (k, s, dec(a)))
            .collect();
        prop_assert_eq!(ri, kernel::reversal(&to_dec(&lines)));
    }

    #[test]
    fn balances_agree(t in arb_type(), m in arb_cents()) {
        prop_assert_eq!(kernel::normal_balance(t, m).map(dec), kernel::normal_balance(t, dec(m)));
        let ci = kernel::columns(m).unwrap();
        prop_assert_eq!(Some((dec(ci.0), dec(ci.1))), kernel::columns(dec(m)));
        prop_assert_eq!(
            kernel::closing_line(m).map(|(s, a)| (s, dec(a))),
            kernel::closing_line(dec(m))
        );
        prop_assert_eq!(
            kernel::credit_line(m).map(|(s, a)| (s, dec(a))),
            kernel::credit_line(dec(m))
        );
    }

    #[test]
    fn plug_last_agrees(whole in arb_cents(), parts in prop::collection::vec(arb_cents(), 0..5)) {
        let mut pi = parts.clone();
        let mut pd: Vec<Decimal> = parts.iter().copied().map(dec).collect();
        kernel::plug_last(whole, &mut pi).unwrap();
        kernel::plug_last(dec(whole), &mut pd).unwrap();
        prop_assert_eq!(pi.into_iter().map(dec).collect::<Vec<_>>(), pd);
    }

    #[test]
    fn apportion_agrees(
        parts in prop::collection::vec(arb_cents(), 0..5),
        rems in prop::collection::vec(0i64..100, 5),
        short in 0i64..500,
    ) {
        let n = parts.len();
        let whole: i64 = parts.iter().sum::<i64>() + short;
        let mut pi = parts.clone();
        let mut pd: Vec<Decimal> = parts.iter().copied().map(dec).collect();
        let ri = &rems[..n];
        let rd: Vec<Decimal> = ri.iter().copied().map(dec).collect();
        let oi = kernel::apportion(whole, &mut pi, ri, 100);
        let od = kernel::apportion(dec(whole), &mut pd, &rd, dec(100));
        prop_assert_eq!(oi.is_some(), od.is_some());
        if oi.is_some() {
            prop_assert_eq!(pi.into_iter().map(dec).collect::<Vec<_>>(), pd);
        }
    }

    #[test]
    fn line_range_agrees(cents in arb_cents()) {
        prop_assert_eq!(cents.in_line_range(), dec(cents).in_line_range());
    }

    #[test]
    fn apportion_cumulative_agrees(
        parts in prop::collection::vec(arb_cents(), 0..5),
        rems in prop::collection::vec(0i64..100, 5),
        given in prop::collection::vec(arb_cents(), 5),
        short in 0i64..500,
        unpaired in any::<bool>(),
    ) {
        let n = parts.len();
        let whole: i64 = parts.iter().sum::<i64>() + short;
        // Past both line ranges the two differ by design (i64's is 2^40
        // units, Decimal's a quadrillion).
        prop_assume!(whole.unsigned_abs() <= MAX_CENTS as u64);
        let mut pi = parts.clone();
        let mut pd: Vec<Decimal> = parts.iter().copied().map(dec).collect();
        let ri = &rems[..n];
        let rd: Vec<Decimal> = ri.iter().copied().map(dec).collect();
        // Sometimes one `given` too many: both refuse.
        let gi = &given[..(n + usize::from(unpaired)).min(5)];
        let gd: Vec<Decimal> = gi.iter().copied().map(dec).collect();
        let oi = kernel::apportion_cumulative(whole, &mut pi, ri, 100, gi);
        let od = kernel::apportion_cumulative(dec(whole), &mut pd, &rd, dec(100), &gd);
        prop_assert_eq!(oi.is_some(), od.is_some());
        prop_assert_eq!(pi.into_iter().map(dec).collect::<Vec<_>>(), pd);
    }

    /// periods::allocate_cumulative, on Decimal shares w_i / W (exact:
    /// W is 100 or 10,000), against its integer model at cents: floors
    /// |T|·w_i div W, remainders |T|·w_i mod W, kernel::apportion_cumulative
    /// at i64, a loss as the negated gain (the model Kani's
    /// cumulative_close_never_drifts_a_unit_at_* and Lean's allocW prove).
    #[test]
    fn allocate_cumulative_agrees_with_the_integer_model(
        four_places in any::<bool>(),
        cuts in prop::collection::vec(0i64..=10_000, 0..3),
        closed in -10_000_000i64..10_000_000,
        net in -10_000_000i64..10_000_000,
        spread in prop::collection::vec(-5_000i64..5_000, 3),
    ) {
        // Weights summing to W: the gaps between sorted cut points.
        let big_w: i64 = if four_places { 10_000 } else { 100 };
        let mut points: Vec<i64> = cuts.iter().map(|c| c % (big_w + 1)).collect();
        points.push(0);
        points.push(big_w);
        points.sort_unstable();
        let weights: Vec<i64> = points.windows(2).map(|w| w[1] - w[0]).filter(|&w| w > 0).collect();
        let n = weights.len();
        let scale = if four_places { 4 } else { 2 };
        let shares: Vec<Decimal> = weights.iter().map(|&w| Decimal::new(w, scale)).collect();
        // What earlier closes gave: anything summing to what they closed.
        let mut given: Vec<i64> = spread[..n - 1].to_vec();
        given.push(closed - given.iter().sum::<i64>());
        let total = closed + net;
        let got = ledger_core::periods::allocate_cumulative(
            dec(total),
            &shares,
            &given.iter().copied().map(dec).collect::<Vec<_>>(),
            2,
        )
        .unwrap();
        let magnitude = total.abs();
        let mut parts: Vec<i64> = weights.iter().map(|w| magnitude * w / big_w).collect();
        let rems: Vec<i64> = weights.iter().map(|w| magnitude * w % big_w).collect();
        let against: Vec<i64> = given.iter().map(|&g| if total < 0 { -g } else { g }).collect();
        kernel::apportion_cumulative(magnitude, &mut parts, &rems, 1, &against).unwrap();
        let want: Vec<Decimal> = parts.iter().map(|&p| dec(if total < 0 { -p } else { p })).collect();
        prop_assert_eq!(&got, &want);
        prop_assert_eq!(got.iter().sum::<Decimal>(), dec(net));
        for ((g, p), w) in given.iter().zip(&got).zip(&weights) {
            let drift = (dec(*g) + p) * Decimal::from(big_w) - dec(total) * Decimal::from(*w);
            prop_assert!(drift.abs() < Decimal::from(big_w) / Decimal::from(100), "{drift}");
        }
    }
}
