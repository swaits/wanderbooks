//! U4 proof harnesses: exact totals (kernel sweep).
//!
//! `kernel::total`, `total_of` and `total_try` on `i64` (the engine runs
//! the same code on `Decimal`; `tests/kernel_bridge/total.rs` checks the
//! two agree). Fixed arrays throughout: no symbolic-length `Vec`, no
//! `copy_from_slice` (model-checking/kani#4904).

use super::{N, any_amount, any_side, wide};
use crate::Side;
use crate::kernel::{self, Amount};

/// Most figures in each of the two runs `total_append` joins.
const A: usize = 3;

fn any_count(max: usize) -> usize {
    let n: usize = kani::any();
    kani::assume(n <= max);
    n
}

/// The exact sum of some `i64`s, in `i128`, the specification.
fn spec_sum(values: &[i64]) -> i128 {
    values.iter().map(|&v| i128::from(v)).sum()
}

/// No figures total to zero, for `total`, `total_of` and `total_try`.
#[kani::proof]
fn total_empty_is_zero() {
    let none: [i64; 0] = [];
    assert_eq!(kernel::total(none), Some(0));
    assert_eq!(kernel::total_of(none, |v: i64| v), Some(0));
    assert_eq!(kernel::total_try(none, Some), Some(0));
}

/// A run appended to another totals to the sum of their totals: for any
/// in-range runs `a` and `b`, `total(a ++ b) = total(a) + total(b)`, and
/// all three are `Some` (the combined run never leaves the range).
#[kani::proof]
#[kani::unwind(8)]
fn total_append() {
    let a_values: [i64; A] = core::array::from_fn(|_| any_amount());
    let b_values: [i64; A] = core::array::from_fn(|_| any_amount());
    let a = &a_values[..any_count(A)];
    let b = &b_values[..any_count(A)];
    // The concatenation, into a fixed array (no growth, no copy_from_slice).
    let mut both = [0i64; 2 * A];
    let mut at = 0;
    for &v in a.iter().chain(b.iter()) {
        both[at] = v;
        at += 1;
    }
    let ta = kernel::total(a.iter().copied()).expect("in-range amounts never overflow");
    let tb = kernel::total(b.iter().copied()).expect("in-range amounts never overflow");
    let tab = kernel::total(both[..at].iter().copied()).expect("in-range amounts never overflow");
    assert_eq!(Some(tab), ta.checked_add(tb));
    kani::cover!(
        !a.is_empty() && !b.is_empty() && tab != 0,
        "two non-empty runs"
    );
}

/// A total is exact whenever it exists, for *any* `i64` figures (the edge
/// included): `Some(s)` means `s` is the true sum, and `total` agrees with
/// a plain left-to-right checked fold.
#[kani::proof]
#[kani::unwind(5)]
fn total_is_sound() {
    let all: [i64; 4] = core::array::from_fn(|_| kani::any());
    let values = &all[..any_count(4)];
    if let Some(s) = kernel::total(values.iter().copied()) {
        assert_eq!(i128::from(s), spec_sum(values));
        kani::cover!(values.len() > 1 && s != 0, "a nonempty exact total");
    }
}

/// A total over a mapped figure is the sum of the mapping: with the line's
/// signed amount as the map, `total_of` is `net` (the signed sum of the
/// lines), and it is the exact sum of the signed amounts.
#[kani::proof]
#[kani::unwind(6)]
fn total_of_is_map_sum() {
    let lines: [(Side, i64); N] = core::array::from_fn(|_| (any_side(), any_amount()));
    let used = &lines[..any_count(N)];
    let by_map = kernel::total_of(used.iter().copied(), |(side, amount)| {
        kernel::signed(side, amount).expect("an in-range amount negates")
    })
    .expect("in-range amounts never overflow");
    let spec: i64 = used.iter().map(|&(s, a)| wide(s, a)).sum();
    assert_eq!(by_map, spec);
    assert_eq!(Some(by_map), kernel::net(used.iter().copied()));
    kani::cover!(used.len() > 1 && by_map != 0, "a nonzero signed sum");
}

/// A total over a fallible mapping (a difference of two figures) is the sum
/// of the differences, and `None` when a difference is not representable.
#[kani::proof]
#[kani::unwind(6)]
fn total_try_is_map_sum() {
    let pairs: [(i64, i64); N] = core::array::from_fn(|_| (any_amount(), any_amount()));
    let used = &pairs[..any_count(N)];
    let got = kernel::total_try(used.iter().copied(), |(c, d)| c.checked_sub(d))
        .expect("in-range differences never overflow");
    let spec: i64 = used.iter().map(|&(c, d)| c - d).sum();
    assert_eq!(got, spec);
    kani::cover!(used.len() > 1 && got != 0, "a nonzero sum of differences");

    // A mapping that fails fails the whole total, wherever it is.
    let failing = kernel::total_try(used.iter().copied(), |_| None::<i64>);
    assert_eq!(failing.is_none(), !used.is_empty());
}

/// Overflow is `None`, never a wrap: for any two `i64`s, their total
/// exists exactly when the true sum fits, and the overflow case is reached.
#[kani::proof]
fn total_overflow_is_none() {
    let a: i64 = kani::any();
    let b: i64 = kani::any();
    let true_sum = i128::from(a) + i128::from(b);
    let fits = true_sum >= i128::from(i64::MIN) && true_sum <= i128::from(i64::MAX);
    let got = kernel::total([a, b]);
    assert_eq!(got.is_some(), fits);
    if let Some(s) = got {
        assert_eq!(i128::from(s), true_sum);
    }
    kani::cover!(!fits && true_sum > 0, "overflow past the maximum");
    kani::cover!(!fits && true_sum < 0, "overflow past the minimum");
    kani::cover!(fits && a != 0 && b != 0, "an ordinary sum");
}
