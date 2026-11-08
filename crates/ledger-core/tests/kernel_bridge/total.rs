//! U4 bridge properties: exact totals (kernel sweep).
//!
//! `kernel::total`, `total_of` and `total_try` on `i64` cents (what Kani
//! proved, `src/proofs/total.rs`) against the same on `Decimal` (what the
//! engine runs), on random lists, plus a table of the `Decimal` edges.

use super::{arb_cents, dec};
use ledger_core::kernel::{self, Amount};
use proptest::prelude::*;
use rust_decimal::Decimal;

fn arb_run() -> impl Strategy<Value = Vec<i64>> {
    prop::collection::vec(arb_cents(), 0..9)
}

fn to_dec(run: &[i64]) -> Vec<Decimal> {
    run.iter().map(|&c| dec(c)).collect()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    /// Both types total a run to the same cents, and to the plain sum.
    #[test]
    fn total_agrees(run in arb_run()) {
        let i = kernel::total(run.iter().copied());
        let d = kernel::total(to_dec(&run));
        prop_assert_eq!(i.map(dec), d);
        prop_assert_eq!(i, Some(run.iter().sum::<i64>()));
    }

    /// A mapped total agrees: the map is applied before the sum, both types.
    #[test]
    fn total_of_agrees(run in arb_run()) {
        let i = kernel::total_of(run.iter().copied(), |c| c * 3);
        let d = kernel::total_of(to_dec(&run), |x| x * Decimal::from(3));
        prop_assert_eq!(i.map(dec), d);
        // total_of is the total of the mapped run.
        prop_assert_eq!(
            kernel::total_of(run.iter().copied(), |c| c * 3),
            kernel::total(run.iter().map(|&c| c * 3))
        );
    }

    /// A fallible mapping agrees too: the difference of each pair.
    #[test]
    fn total_try_agrees(pairs in prop::collection::vec((arb_cents(), arb_cents()), 0..7)) {
        let i = kernel::total_try(pairs.iter().copied(), |(c, d)| c.checked_sub(d));
        let d = kernel::total_try(
            pairs.iter().map(|&(c, d)| (dec(c), dec(d))),
            |(c, d)| Amount::checked_sub(c, d),
        );
        prop_assert_eq!(i.map(dec), d);
        // And a failing mapping fails the whole total at both types.
        prop_assert_eq!(
            kernel::total_try(pairs.iter().copied(), |_| None::<i64>).is_none(),
            !pairs.is_empty()
        );
        prop_assert_eq!(
            kernel::total_try(pairs.iter().map(|&(c, _)| dec(c)), |_| None::<Decimal>).is_none(),
            !pairs.is_empty()
        );
    }

    /// Appending runs adds their totals (no overflow at cents), at both types.
    #[test]
    fn total_append_agrees(a in arb_run(), b in arb_run()) {
        let both: Vec<i64> = a.iter().chain(&b).copied().collect();
        let (ta, tb) = (kernel::total(to_dec(&a)).unwrap(), kernel::total(to_dec(&b)).unwrap());
        prop_assert_eq!(kernel::total(to_dec(&both)), Amount::checked_add(ta, tb));
        let (ia, ib) = (kernel::total(a.iter().copied()).unwrap(), kernel::total(b.iter().copied()).unwrap());
        prop_assert_eq!(kernel::total(both.iter().copied()), ia.checked_add(ib));
    }
}

/// The `Decimal` edges: what is representable is summed exactly, and what is
/// not is `None`, never a panic and never a rounded figure.
#[test]
fn decimal_edge_table() {
    let max = Decimal::MAX;
    let min = Decimal::MIN;
    let one = Decimal::ONE;

    // The empty total, and a single figure at either edge.
    assert_eq!(kernel::total(Vec::<Decimal>::new()), Some(Decimal::ZERO));
    assert_eq!(kernel::total([max]), Some(max));
    assert_eq!(kernel::total([min]), Some(min));

    // One unit past either edge is refused.
    assert_eq!(kernel::total([max, one]), None);
    assert_eq!(kernel::total([min, -one]), None);
    assert_eq!(kernel::total([max, max]), None);
    assert_eq!(kernel::total([min, min]), None);

    // Opposite edges cancel exactly.
    assert_eq!(kernel::total([max, min]), Some(Decimal::ZERO));
    assert_eq!(kernel::total([max, -max]), Some(Decimal::ZERO));
    // Mixed signs that cancel exactly, in the middle of the range.
    let run = [dec(1_00), dec(-1_00), dec(3_33), dec(-3_33), Decimal::ZERO];
    assert_eq!(kernel::total(run), Some(Decimal::ZERO));

    // Each step must be exact, so the edge is order dependent by design:
    // the same three figures, one order representable, the other not.
    assert_eq!(kernel::total([max, -one, one]), Some(max));
    assert_eq!(kernel::total([max, one, -one]), None);
    assert_eq!(kernel::total([one, max, -one]), None);

    // total_of and total_try see the same edges.
    assert_eq!(kernel::total_of([max, one], |x| x), None);
    assert_eq!(
        kernel::total_of([1, 2, 3], Decimal::from),
        Some(Decimal::from(6))
    );
    assert_eq!(
        kernel::total_try([(max, min)], |(a, b)| Amount::checked_sub(a, b)),
        None
    );
    assert_eq!(
        kernel::total_try([(max, max), (min, min)], |(a, b)| Amount::checked_sub(a, b)),
        Some(Decimal::ZERO)
    );
}

/// FINDING (K-U4), pinned as a canary: `rust_decimal` rounds sub-unit dust
/// away when a sum lands on `Decimal::MAX`, so `Amount::checked_add` for
/// `Decimal` is not exact at the very edge (`MAX + 0.4 = MAX`, but `MAX +
/// 0.5` is `None`). The kernel's "exact or `None`" contract (and `total`'s
/// invariant 2) therefore holds for `Decimal` everywhere except figures
/// within one unit of `Decimal::MAX` (~7.9 x 10^28, 10^13 times a journal
/// line's cap). The fix belongs in `impl Amount for Decimal`
/// (`kernel/entries.rs`), not in `total`; when it lands this test fails
/// and is flipped to expect `None`. See docs/proofs/k-u4.md.
#[test]
fn finding_decimal_add_rounds_dust_at_the_edge() {
    let max = Decimal::MAX;
    let dust = Decimal::new(4, 1);
    assert_eq!(
        kernel::total([max, dust]),
        Some(max),
        "rust_decimal rounds the 0.4 away"
    );
    assert_eq!(kernel::total([max, Decimal::new(5, 1)]), None);
}
