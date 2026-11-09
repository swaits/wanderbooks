//! U4: totals, exact checked sums for books and reports (kernel sweep).
//!
//! The sum of a collection of money figures, in one place. A raw `Decimal`
//! `+`, `+=` or `.sum()` panics at `Decimal::MAX`; these return `None`
//! instead, so the caller can refuse with `TooLargeToCompute` and never
//! crash a book (BS-entry-4: a replayed journal line is accepted "of
//! any size", so a restored figure is the realistic path to the edge).
//! The signed sum of `(Side, amount)` lines is [`super::net`] and the pair
//! of columns [`super::totals`]; they were here first and stay the one
//! copy of that shape.

use super::Amount;

/// The exact sum of some figures: `None` the moment a running sum cannot
/// be represented, never a wrapped, rounded or panicking one.
///
/// ```text
/// GOAL: a book's or report's total is the exact sum of its figures, or
///       refused; no total is ever rounded, wrapped or a crash.
/// SOURCE: BS-entry-3 (amounts are exact; rounding is
///         confined to returns), BS-entry-4 (replayed lines are accepted "of any
///         size"; no input may reach a panic); bookspec BS-entry-3,
///         BS-entry-4; CONVENTION.
/// INVARIANTS:
///   1. total([]) = Some(0).
///   2. Some(s): s is the exact mathematical sum of the figures (every
///      running sum was representable, so no step lost anything).
///   3. total(a ++ b) = total(a) + total(b) whenever the combined run
///      never leaves the representable range (always for amounts within
///      the line range and a few thousand of them). At the very edge the
///      sum is order dependent by design: [MAX, 1, -1] is None while
///      [MAX, -1, 1] is Some(MAX), because each step must be exact.
///   4. None on overflow; NEVER a rounded or wrapped result, never a
///      panic. (Exactness is `Amount::checked_add`'s: `rust_decimal`
///      rounds sub-unit dust when a sum lands on `Decimal::MAX`; see
///      docs/proofs/k-u4.md, finding 1.)
/// ```
pub fn total<T: Amount>(values: impl IntoIterator<Item = T>) -> Option<T> {
    let mut sum = T::ZERO;
    for value in values {
        sum = sum.checked_add(value)?;
    }
    Some(sum)
}

/// The exact sum of a figure drawn from each item (`f` of each), the shape
/// of `iter().map(f).sum()`: `total(map(f, items))`.
///
/// ```text
/// GOAL: the sum of a mapped field (a row's amount, a member's share) is
///       the exact sum of the mapping, or refused.
/// SOURCE: BS-entry-3, BS-entry-4; bookspec BS-entry-3,
///         BS-entry-4; CONVENTION.
/// INVARIANTS:
///   1. total_of(items, f) = total(f(item) for item in items), exactly.
///   2. None on overflow; NEVER a rounded or wrapped result.
/// ```
pub fn total_of<T: Amount, I>(
    values: impl IntoIterator<Item = I>,
    f: impl FnMut(I) -> T,
) -> Option<T> {
    total(values.into_iter().map(f))
}

/// [`total_of`] for a mapping that can itself fail (a difference of two
/// figures, `c - d`, that may not be representable): `None` when `f` is
/// `None` for any item or the running sum overflows.
///
/// ```text
/// GOAL: the sum of a derived figure (a column's contributions less its
///       distributions) is exact, or refused.
/// SOURCE: BS-entry-3, BS-entry-4; bookspec BS-entry-3,
///         BS-entry-4; CONVENTION.
/// INVARIANTS:
///   1. total_try(items, f) = Some(Σ f(item)) exactly when every f(item)
///      is Some and every running sum is representable; otherwise None.
/// ```
pub fn total_try<T: Amount, I>(
    values: impl IntoIterator<Item = I>,
    mut f: impl FnMut(I) -> Option<T>,
) -> Option<T> {
    let mut sum = T::ZERO;
    for value in values {
        sum = sum.checked_add(f(value)?)?;
    }
    Some(sum)
}
