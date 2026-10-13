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
    fn line_range_agrees(cents in arb_cents()) {
        prop_assert_eq!(cents.in_line_range(), dec(cents).in_line_range());
    }
}
