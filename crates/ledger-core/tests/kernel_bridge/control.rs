//! U8 bridge properties: control and aging (kernel sweep).
//!
//! `kernel::running_bounds` and `kernel::bounds_kept` at `i64` cents (what
//! Kani proves, law 34) against the same on `Decimal` (what the engine
//! runs), on random documents.

use super::{arb_cents, dec};
use ledger_core::kernel;
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    /// The same running balances are within bounds, or leave them at the
    /// same step, at both types.
    #[test]
    fn running_bounds_agree(
        total in arb_cents(),
        deltas in prop::collection::vec(arb_cents(), 0..7),
    ) {
        let wide: Vec<_> = deltas.iter().map(|&c| dec(c)).collect();
        prop_assert_eq!(
            kernel::running_bounds(total, &deltas),
            kernel::running_bounds(dec(total), &wide)
        );
    }

    /// The same step keeps (or breaks) a document's bounds at both types.
    #[test]
    fn bounds_kept_agree(
        total in arb_cents(),
        before in prop::collection::vec(arb_cents(), 0..7),
        after in prop::collection::vec(arb_cents(), 0..7),
    ) {
        let wide_before: Vec<_> = before.iter().map(|&c| dec(c)).collect();
        let wide_after: Vec<_> = after.iter().map(|&c| dec(c)).collect();
        prop_assert_eq!(
            kernel::bounds_kept(total, &before, &after),
            kernel::bounds_kept(dec(total), &wide_before, &wide_after)
        );
    }
}
