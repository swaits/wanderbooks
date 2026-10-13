//! The accounting kernel: the arithmetic every bookkeeping invariant rests
//! on, in one place.
//!
//! Each function here is small, pure, and generic over [`Amount`]. The
//! engine runs it on [`Decimal`]; the Kani harnesses in `proofs.rs` run the
//! very same code on machine integers and prove it correct for every input
//! within their bounds; and the Lean model in `verify/lean` proves the
//! laws it implements hold for books of any size. See
//! `docs/verification.md` for which proof covers which law.
//!
//! The laws, in the order they appear (each traced to its authority in
//! `docs/spec/rules.md`):
//!
//! * a line's signed amount is `+amount` for a debit, `-amount` for a credit
//!   (B-2);
//! * an entry is acceptable when it has two or more lines, every amount is
//!   positive and in range, and its debits equal its credits exactly (B-1,
//!   B-2, B-21);
//! * an acceptable entry nets to zero, account by account summed (B-1);
//! * a reversal flips every side, so it is acceptable exactly when the
//!   original is, and the two together net every account to zero (B-9);
//! * an account's balance is its movement read on its normal side, so the
//!   balances satisfy the accounting equation whenever movements net to
//!   zero (B-5, B-7);
//! * a trial balance puts each movement in exactly one column, so its
//!   columns differ by the net of all movements (B-6);
//! * a closing line zeroes the account it closes (B-11);
//! * an allocation's parts sum to the whole, to the last unit, by largest
//!   remainder (B-13).

use crate::{AccountType, Side};
use rust_decimal::Decimal;

/// What the kernel needs of a money amount: exact, ordered, and checked
/// arithmetic that reports overflow instead of rounding or wrapping.
pub trait Amount: Copy + Ord {
    /// Zero.
    const ZERO: Self;
    /// `self + rhs`, or `None` when it cannot be represented exactly.
    fn checked_add(self, rhs: Self) -> Option<Self>;
    /// `-self`, or `None` when it cannot be represented.
    fn checked_neg(self) -> Option<Self>;
    /// True when the amount is small and precise enough for a journal line
    /// (see [`MAX_LINE_AMOUNT`] and [`MAX_LINE_SCALE`] for [`Decimal`]).
    fn in_line_range(self) -> bool;

    /// `self - rhs`, or `None` when it cannot be represented exactly.
    fn checked_sub(self, rhs: Self) -> Option<Self> {
        self.checked_add(rhs.checked_neg()?)
    }
}

/// The largest amount one journal line may carry: one quadrillion units of
/// the base currency. Far beyond any real transaction, and far enough
/// below [`Decimal::MAX`] that no realistic sum of lines can overflow.
pub const MAX_LINE_AMOUNT: Decimal = Decimal::from_parts(0xA4C6_8000, 0x0003_8D7E, 0, false, 0);

/// The most decimal places a journal line may carry. Cents need two;
/// foreign-currency lines (amount times rate) need more; ten keeps every
/// sum of lines exact as long as the total stays under 7.9 quintillion.
pub const MAX_LINE_SCALE: u32 = 10;

impl Amount for Decimal {
    const ZERO: Self = Decimal::ZERO;
    fn checked_add(self, rhs: Self) -> Option<Self> {
        Decimal::checked_add(self, rhs)
    }
    fn checked_neg(self) -> Option<Self> {
        Some(-self)
    }
    fn in_line_range(self) -> bool {
        self.abs() <= MAX_LINE_AMOUNT && self.normalize().scale() <= MAX_LINE_SCALE
    }
}

impl Amount for i64 {
    const ZERO: Self = 0;
    fn checked_add(self, rhs: Self) -> Option<Self> {
        i64::checked_add(self, rhs)
    }
    fn checked_neg(self) -> Option<Self> {
        i64::checked_neg(self)
    }
    fn in_line_range(self) -> bool {
        // Scaled down with the same headroom as Decimal's range: lines
        // this size can be summed many times over without overflow.
        self.unsigned_abs() <= (1 << 40)
    }
}

/// Why the kernel rejected an entry's lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineError<T> {
    /// Fewer than two lines.
    TooFewLines,
    /// A line with a zero amount.
    ZeroAmount,
    /// A line with a negative amount: direction is the side's job.
    NegativeAmount,
    /// A line too large or too precise for a journal line.
    OutOfRange,
    /// Debits and credits differ.
    Unbalanced {
        /// Sum of debit lines.
        debits: T,
        /// Sum of credit lines.
        credits: T,
    },
}

/// The other side.
pub fn flip(side: Side) -> Side {
    match side {
        Side::Debit => Side::Credit,
        Side::Credit => Side::Debit,
    }
}

/// A line's signed amount: positive for a debit, negative for a credit.
pub fn signed<T: Amount>(side: Side, amount: T) -> Option<T> {
    match side {
        Side::Debit => Some(amount),
        Side::Credit => amount.checked_neg(),
    }
}

/// Total debits and total credits of some lines.
pub fn totals<T: Amount>(lines: impl IntoIterator<Item = (Side, T)>) -> Option<(T, T)> {
    let mut debits = T::ZERO;
    let mut credits = T::ZERO;
    for (side, amount) in lines {
        match side {
            Side::Debit => debits = debits.checked_add(amount)?,
            Side::Credit => credits = credits.checked_add(amount)?,
        }
    }
    Some((debits, credits))
}

/// The net of some lines: debits minus credits.
pub fn net<T: Amount>(lines: impl IntoIterator<Item = (Side, T)>) -> Option<T> {
    let mut total = T::ZERO;
    for (side, amount) in lines {
        total = total.checked_add(signed(side, amount)?)?;
    }
    Some(total)
}

/// Check an entry's lines: two or more, every amount positive and within
/// line range, debits equal to credits. Accounts and currencies are the
/// caller's to check; this is the arithmetic.
pub fn check_lines<T: Amount>(lines: &[(Side, T)]) -> Result<(), LineError<T>> {
    if lines.len() < 2 {
        return Err(LineError::TooFewLines);
    }
    for &(_, amount) in lines {
        if amount == T::ZERO {
            return Err(LineError::ZeroAmount);
        }
        if amount < T::ZERO {
            return Err(LineError::NegativeAmount);
        }
        if !amount.in_line_range() {
            return Err(LineError::OutOfRange);
        }
    }
    let (debits, credits) = totals(lines.iter().copied()).ok_or(LineError::OutOfRange)?;
    if debits != credits {
        return Err(LineError::Unbalanced { debits, credits });
    }
    Ok(())
}

/// Net movement per account (debits minus credits), in order of first
/// appearance.
pub fn movements<K: Copy + Eq, T: Amount>(
    lines: impl IntoIterator<Item = (K, Side, T)>,
) -> Option<Vec<(K, T)>> {
    let lines: Vec<_> = lines.into_iter().collect();
    let Some(&(first, _, _)) = lines.first() else {
        return Some(Vec::new());
    };
    // At most one account per line.
    let mut out = vec![(first, T::ZERO); lines.len()];
    let n = movements_into(&lines, &mut out)?;
    out.truncate(n);
    Some(out)
}

/// [`movements`] into a caller's buffer, without allocating: returns how
/// many accounts it wrote, or `None` on overflow or if `out` is too small.
pub fn movements_into<K: Copy + Eq, T: Amount>(
    lines: &[(K, Side, T)],
    out: &mut [(K, T)],
) -> Option<usize> {
    let mut n = 0;
    for &(account, side, amount) in lines {
        let delta = signed(side, amount)?;
        match out[..n].iter_mut().find(|(k, _)| *k == account) {
            Some((_, total)) => *total = total.checked_add(delta)?,
            None => {
                *out.get_mut(n)? = (account, delta);
                n += 1;
            }
        }
    }
    Some(n)
}

/// The lines of a reversing entry: every side flipped, accounts and
/// amounts unchanged.
pub fn reversal<K: Copy, T: Amount>(lines: &[(K, Side, T)]) -> Vec<(K, Side, T)> {
    lines.iter().map(|&line| reverse_line(line)).collect()
}

/// One line of a reversing entry: the side flipped, account and amount
/// unchanged.
pub fn reverse_line<K, T: Amount>((account, side, amount): (K, Side, T)) -> (K, Side, T) {
    (account, flip(side), amount)
}

/// An account's balance read on its normal side: positive is a normal
/// balance, negative a contra balance.
pub fn normal_balance<T: Amount>(account_type: AccountType, movement: T) -> Option<T> {
    match account_type.normal_side() {
        Side::Debit => Some(movement),
        Side::Credit => movement.checked_neg(),
    }
}

/// A movement in trial-balance columns: `(debit, credit)`, one of them
/// zero, both non-negative.
pub fn columns<T: Amount>(movement: T) -> Option<(T, T)> {
    if movement > T::ZERO {
        Some((movement, T::ZERO))
    } else if movement == T::ZERO {
        // Never a negative zero: money never prints "-0".
        Some((T::ZERO, T::ZERO))
    } else {
        Some((T::ZERO, movement.checked_neg()?))
    }
}

/// The line that zeroes an account with this movement: the opposite side,
/// for the same amount. `None` for a zero movement (nothing to close).
pub fn closing_line<T: Amount>(movement: T) -> Option<(Side, T)> {
    if movement > T::ZERO {
        Some((Side::Credit, movement))
    } else if movement < T::ZERO {
        Some((Side::Debit, movement.checked_neg()?))
    } else {
        None
    }
}

/// The line that credits `credit` to an account (a negative credit is a
/// debit). `None` for zero. This is [`closing_line`]: crediting `c` and
/// zeroing a movement of `c` are the same line.
pub fn credit_line<T: Amount>(credit: T) -> Option<(Side, T)> {
    closing_line(credit)
}

/// Make rounded parts sum to the whole: the last part becomes `whole`
/// minus the others, absorbing every unit of rounding. Returns `None` on
/// overflow, leaving `parts` unchanged.
pub fn plug_last<T: Amount>(whole: T, parts: &mut [T]) -> Option<()> {
    let Some((last, rest)) = parts.split_last_mut() else {
        return Some(());
    };
    let mut others = T::ZERO;
    for &p in rest.iter() {
        others = others.checked_add(p)?;
    }
    *last = whole.checked_sub(others)?;
    Some(())
}

/// Largest-remainder apportionment: make rounded-down `parts` sum to
/// `whole`, handing out the shortfall one `unit` at a time, largest
/// `remainders` first (ties to the earlier part). Any final fraction of a
/// unit (when `whole` isn't a whole number of units) goes to the first
/// part in that order.
///
/// ```text
/// GOAL: split a whole into parts, each a whole number of units where
///       possible, summing to the whole exactly, without favoring anyone.
/// SOURCE: docs/spec/rules.md B-13 (shares: 26 U.S.C. §704(a); cent
///         rounding: DECISION, owner 2026-10-13), B-21 (exact totals).
/// INVARIANTS:
///   1. MUST make the parts sum to `whole` exactly.
///   2. MUST change each part by zero or one unit (the first in order
///      may also take the sub-unit fraction).
///   3. NEVER gives a unit to a part while a part with a strictly larger
///      remainder goes without.
/// SCENARIOS:
///   1. IF the shortfall is negative, or `n` units or more (the parts
///      weren't rounded down from shares of `whole`):
///      - SHOULD return None, leaving `parts` unchanged
///      BECAUSE apportioning only corrects rounding.
///   2. IF $100.00 splits three equal ways:
///      - SHOULD be 33.34, 33.33, 33.33
///      - NEVER the same partner absorbing every odd cent by position
///      BECAUSE the cent goes to the largest remainder, ties to the first
///      (owner, 2026-10-13).
/// ```
///
/// Allocation-free and O(n^2): it ranks each part by counting the parts
/// ahead of it, so the model checker can prove it outright.
pub fn apportion<T: Amount>(whole: T, parts: &mut [T], remainders: &[T], unit: T) -> Option<()> {
    if parts.len() != remainders.len() || unit <= T::ZERO {
        return None;
    }
    if parts.is_empty() {
        // Nothing to give the whole to.
        return (whole == T::ZERO).then_some(());
    }
    let mut shortfall = whole;
    for &p in parts.iter() {
        shortfall = shortfall.checked_sub(p)?;
    }
    if shortfall < T::ZERO {
        return None;
    }
    // Whole units short, by subtraction: no division in `Amount`. A
    // shortfall of a unit per part or more isn't rounding; refuse it.
    let mut units = 0;
    while shortfall >= unit {
        shortfall = shortfall.checked_sub(unit)?;
        units += 1;
        if units == parts.len() {
            return None;
        }
    }
    // A part's rank: the parts ahead of it (larger remainder, or equal and
    // earlier).
    let rank = |i: usize| {
        (0..remainders.len())
            .filter(|&j| remainders[j] > remainders[i] || (remainders[j] == remainders[i] && j < i))
            .count()
    };
    for (i, part) in parts.iter_mut().enumerate() {
        let r = rank(i);
        let mut add = T::ZERO;
        if r < units {
            add = unit;
        }
        if r == 0 {
            add = add.checked_add(shortfall)?;
        }
        *part = part.checked_add(add)?;
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn max_line_amount_is_one_quadrillion() {
        assert_eq!(MAX_LINE_AMOUNT, Decimal::from(1_000_000_000_000_000u64));
        assert!(MAX_LINE_AMOUNT.in_line_range());
        assert!(!(MAX_LINE_AMOUNT + Decimal::ONE).in_line_range());
        assert!("0.0000000001".parse::<Decimal>().unwrap().in_line_range());
        assert!(!"0.00000000001".parse::<Decimal>().unwrap().in_line_range());
        // Trailing zeros are not precision.
        assert!("1.000000000000".parse::<Decimal>().unwrap().in_line_range());
    }

    /// Regression (Kani `apportion_is_exact`, 2026-10-13): a shortfall of
    /// two units and a bit, with one part, was accepted and handed almost
    /// two units to that part. Spec scenario 1: more than one unit short
    /// per part is refused.
    #[test]
    fn apportion_refuses_a_shortfall_of_a_unit_per_part_or_more() {
        let mut parts = [0i64];
        assert_eq!(apportion(1805, &mut parts, &[0], 903), None);
        assert_eq!(parts, [0]);
        let mut parts = [0i64, 0];
        assert_eq!(apportion(1806, &mut parts, &[0, 0], 903), None);
        // Just under: accepted.
        let mut parts = [0i64, 0];
        assert_eq!(apportion(1805, &mut parts, &[1, 0], 903), Some(()));
        assert_eq!(parts, [1805, 0]);
    }

    /// The i64 model's line range: up to 2^40 either way, inclusive.
    #[test]
    fn i64_line_range_is_two_to_the_fortieth() {
        assert!((1i64 << 40).in_line_range());
        assert!((-(1i64 << 40)).in_line_range());
        assert!(!((1i64 << 40) + 1).in_line_range());
        assert!(!i64::MIN.in_line_range());
    }

    /// B-9: a reversal flips every side and keeps accounts and amounts, so
    /// with the original it nets every account to zero.
    #[test]
    fn a_reversal_flips_every_line_and_nets_to_zero() {
        let lines = [
            (1u8, Side::Debit, 70i64),
            (2, Side::Credit, 50),
            (3, Side::Credit, 20),
        ];
        let rev = reversal(&lines);
        assert_eq!(rev.len(), lines.len());
        for (o, r) in lines.iter().zip(&rev) {
            assert_eq!((r.0, r.1, r.2), (o.0, flip(o.1), o.2));
        }
        let both = movements(lines.iter().chain(&rev).copied()).unwrap();
        assert_eq!(both, vec![(1, 0), (2, 0), (3, 0)]);
    }

    /// Spec scenario 1's preconditions: parts and remainders pair up, and
    /// the unit is positive; otherwise nothing is apportioned.
    #[test]
    fn apportion_refuses_mismatched_inputs_or_a_nonpositive_unit() {
        let mut parts = [0i64, 0];
        assert_eq!(apportion(0, &mut parts, &[0], 1), None);
        assert_eq!(parts, [0, 0]);
        let mut none: [i64; 0] = [];
        assert_eq!(apportion(0, &mut none, &[], 0), None);
        assert_eq!(apportion(0, &mut none, &[], -1), None);
    }

    /// With no parts, only a zero whole is fully apportioned.
    #[test]
    fn apportion_to_no_parts_accepts_only_zero() {
        let mut none: [i64; 0] = [];
        assert_eq!(apportion(0, &mut none, &[], 1), Some(()));
        assert_eq!(apportion(5, &mut none, &[], 1), None);
    }

    /// A zero movement's columns are both a plain zero, never "-0" (found
    /// by mutation triage, 2026-10-13: it returned a negative-zero credit).
    #[test]
    fn a_zero_movement_has_no_negative_zero_column() {
        let (d, c) = columns(Decimal::ZERO).unwrap();
        assert_eq!(
            (d.to_string(), c.to_string()),
            ("0".to_string(), "0".to_string())
        );
    }

    #[test]
    fn credit_line_sides() {
        assert_eq!(credit_line(5i64), Some((Side::Credit, 5)));
        assert_eq!(credit_line(-5i64), Some((Side::Debit, 5)));
        assert_eq!(credit_line(0i64), None);
    }
}
