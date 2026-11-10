//! U8 bridge properties: control and aging (kernel sweep).
//!
//! `kernel::running_bounds` and `kernel::bounds_kept` at `i64` cents (what
//! Kani proves, law 34) against the same on `Decimal` (what the engine
//! runs), on random documents. And K-U8's three identities,
//! `kernel::aging_total`, `control_difference` and `reconciliation_total`,
//! at `i64` (`src/proofs/control.rs`) against `Decimal`, and `Decimal`
//! against an independent exact `i128` reference, with the edges that must
//! be `None`.

use super::{arb_cents, dec};
use ledger_core::kernel;
use proptest::prelude::*;
use rust_decimal::Decimal;

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

/// An exact reference: the sum with signs, in `i128`, `None` when any
/// running step leaves what `Decimal` holds at scale 2 (a 96-bit
/// mantissa). Independent of the kernel and of `Decimal` arithmetic.
fn ref_chain(terms: &[(i128, bool)]) -> Option<i128> {
    const LIMIT: i128 = (1i128 << 96) - 1;
    let mut acc = 0i128;
    for &(v, plus) in terms {
        acc = if plus { acc + v } else { acc - v };
        if acc.abs() > LIMIT {
            return None;
        }
    }
    Some(acc)
}

/// Cents anywhere in `Decimal`'s scale-2 mantissa range, edges included.
fn arb_wide_cents() -> impl Strategy<Value = i128> {
    let limit: i128 = (1i128 << 96) - 1;
    prop_oneof![
        arb_cents().prop_map(i128::from),
        (-limit..=limit),
        Just(limit),
        Just(-limit),
        Just(limit - 1),
    ]
}

fn dec_wide(cents: i128) -> Decimal {
    Decimal::from_i128_with_scale(cents, 2)
}

/// `Decimal`'s answer against the exact reference: the same figure at
/// scale 2, or `None` where the reference is `None`. K-U8's finding
/// (the K-U4 finding again) was that where a step's exact result did not
/// fit the 96-bit mantissa at scale 2, `rust_decimal` did not refuse but
/// rounded the result to a coarser scale; K-U10 closed it, so a figure
/// that is not the exact one is a failure. One shape is not a rounding:
/// an exact sum whose last digit is a zero fits a coarser scale (so the
/// reference's cents overflow at that step and `Decimal`'s figure does
/// not, and a later step may bring it back to scale 2); it must equal
/// the exact value.
fn exact_or_refused(
    got: Option<Decimal>,
    want: Option<i128>,
    exact: i128,
) -> Result<(), TestCaseError> {
    match (got, want) {
        (Some(g), Some(w)) => prop_assert_eq!((g.mantissa(), g.scale()), (w, 2)),
        (None, None) => {}
        (Some(g), None) => {
            let cents = g
                .mantissa()
                .checked_mul(10i128.pow(2u32.saturating_sub(g.scale())));
            prop_assert!(
                g.scale() <= 2 && cents == Some(exact),
                "{g} is not the exact {exact} cents"
            );
        }
        (None, Some(w)) => prop_assert!(false, "refused an exact {w}"),
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    /// The aging total is the same cents at `i64` and `Decimal`, and the
    /// plain sum.
    #[test]
    fn aging_total_agrees(b in prop::array::uniform5(arb_cents())) {
        let i = kernel::aging_total(b[0], b[1], b[2], b[3], b[4]);
        let d = kernel::aging_total(dec(b[0]), dec(b[1]), dec(b[2]), dec(b[3]), dec(b[4]));
        prop_assert_eq!(i.map(dec), d);
        prop_assert_eq!(i, Some(b.iter().sum::<i64>()));
    }

    /// The control difference agrees at both types, is the plain
    /// difference, and is zero for agreeing books.
    #[test]
    fn control_difference_agrees(
        balance in arb_cents(), docs in arb_cents(), unattached in arb_cents(),
    ) {
        let i = kernel::control_difference(balance, docs, unattached);
        let d = kernel::control_difference(dec(balance), dec(docs), dec(unattached));
        prop_assert_eq!(i.map(dec), d);
        prop_assert_eq!(i, Some(balance - docs - unattached));
        prop_assert_eq!(
            kernel::control_difference(dec(docs + unattached), dec(docs), dec(unattached)),
            Some(Decimal::ZERO)
        );
    }

    /// The reconciliation total agrees at both types, and is the plain
    /// chain.
    #[test]
    fn reconciliation_total_agrees(f in prop::array::uniform4(arb_cents())) {
        let i = kernel::reconciliation_total(f[0], f[1], f[2], f[3]);
        let d = kernel::reconciliation_total(dec(f[0]), dec(f[1]), dec(f[2]), dec(f[3]));
        prop_assert_eq!(i.map(dec), d);
        prop_assert_eq!(i, Some(f[0] + f[1] + f[2] - f[3]));
    }

    /// `Decimal` against the exact `i128` reference, over the whole
    /// mantissa range: the same figure, or `None` exactly when a step
    /// leaves the range; never a panic.
    #[test]
    fn decimal_matches_i128_reference(f in prop::array::uniform5(arb_wide_cents())) {
        let (a, b, c, d, e) = (f[0], f[1], f[2], f[3], f[4]);
        let got = kernel::aging_total(dec_wide(a), dec_wide(b), dec_wide(c), dec_wide(d), dec_wide(e));
        let want = ref_chain(&[(a, true), (b, true), (c, true), (d, true), (e, true)]);
        exact_or_refused(got, want, a + b + c + d + e)?;

        let got = kernel::control_difference(dec_wide(a), dec_wide(b), dec_wide(c));
        let want = ref_chain(&[(a, true), (b, false), (c, false)]);
        exact_or_refused(got, want, a - b - c)?;

        let got = kernel::reconciliation_total(dec_wide(a), dec_wide(b), dec_wide(c), dec_wide(d));
        let want = ref_chain(&[(a, true), (b, true), (c, true), (d, false)]);
        exact_or_refused(got, want, a + b + c - d)?;
    }
}

/// The edges: what is representable is exact, and what is not is `None`.
#[test]
fn control_identity_edges() {
    let (max, min, one, zero) = (Decimal::MAX, Decimal::MIN, Decimal::ONE, Decimal::ZERO);

    // Aging: every bucket at the edge alone, past it, and cancelling.
    assert_eq!(kernel::aging_total(max, zero, zero, zero, zero), Some(max));
    assert_eq!(kernel::aging_total(min, zero, zero, zero, zero), Some(min));
    assert_eq!(kernel::aging_total(max, one, zero, zero, zero), None);
    assert_eq!(kernel::aging_total(zero, zero, zero, zero, max), Some(max));
    assert_eq!(kernel::aging_total(max, max, zero, zero, zero), None);
    assert_eq!(kernel::aging_total(min, min, zero, zero, zero), None);
    assert_eq!(kernel::aging_total(max, zero, zero, zero, min), Some(zero));
    assert_eq!(
        kernel::aging_total(zero, zero, zero, zero, zero),
        Some(zero)
    );

    // Control: balance past either edge, documents or unattached that
    // push it past, and a difference that lands exactly on an edge.
    assert_eq!(kernel::control_difference(max, zero, zero), Some(max));
    assert_eq!(kernel::control_difference(min, one, zero), None);
    assert_eq!(kernel::control_difference(min, zero, one), None);
    assert_eq!(kernel::control_difference(max, min, zero), None);
    assert_eq!(kernel::control_difference(zero, min, zero), Some(max));
    assert_eq!(kernel::control_difference(zero, zero, min), Some(max));
    assert_eq!(kernel::control_difference(max, max, zero), Some(zero));
    assert_eq!(kernel::control_difference(zero, max, min), Some(zero));

    // Reconciliation: each step, each sign.
    assert_eq!(kernel::reconciliation_total(max, one, zero, zero), None);
    assert_eq!(kernel::reconciliation_total(max, zero, one, zero), None);
    assert_eq!(kernel::reconciliation_total(min, zero, zero, one), None);
    assert_eq!(
        kernel::reconciliation_total(zero, zero, zero, min),
        Some(max)
    );
    assert_eq!(
        kernel::reconciliation_total(max, zero, zero, max),
        Some(zero)
    );
    assert_eq!(kernel::reconciliation_total(max, one, zero, one), None);
    assert_eq!(
        kernel::reconciliation_total(max, zero, min, zero),
        Some(zero)
    );
}

/// The laptop (K-1 scenario 1): two members each at -124.20, as the
/// worksheet's own figures give it.
#[test]
fn reconciliation_laptop() {
    let gain = Decimal::new(-12420, 2);
    assert_eq!(
        kernel::reconciliation_total(Decimal::ZERO, gain, Decimal::ZERO, Decimal::ZERO),
        Some(gain)
    );
}

/// K-U8's finding, closed by K-U10: the dust the K-U4 finding saw at
/// `Decimal::MAX` was wider than that. Two cents-scale figures whose exact
/// sum needs more than 96 bits at scale 2 summed to a rounded figure at
/// scale 1, not to `None`: 0.01 plus the largest scale-2 figure is
/// 792281625142643375935439503.36, which `rust_decimal` returns as
/// 792281625142643375935439503.4. The kernel now refuses it.
#[test]
fn a_sum_past_exact_arithmetic_is_refused() {
    let top = Decimal::from_i128_with_scale((1i128 << 96) - 1, 2);
    let got = kernel::aging_total(
        Decimal::new(1, 2),
        Decimal::ZERO,
        Decimal::ZERO,
        Decimal::ZERO,
        top,
    );
    assert_eq!(got, None);
    // The old chain's figure, for the record: rounded.
    assert_eq!(
        Decimal::new(1, 2)
            .checked_add(top)
            .map(|g| (g.mantissa(), g.scale())),
        Some((7922816251426433759354395034, 1))
    );
}

/// Any `Decimal`, scale and sign included, mantissas up to the whole
/// 96 bits (so sums reach the overflow and rounding edges).
fn arb_decimal() -> impl Strategy<Value = Decimal> {
    (
        prop_oneof![
            -1_000_000_000i128..=1_000_000_000,
            -((1i128 << 96) - 1)..=((1i128 << 96) - 1),
            Just((1i128 << 96) - 1),
            Just(-((1i128 << 96) - 1)),
        ],
        0u32..=28,
    )
        .prop_map(|(m, s)| Decimal::from_i128_with_scale(m, s))
}

fn rat(d: Decimal) -> num_rational::BigRational {
    num_rational::BigRational::new(
        num_bigint::BigInt::from(d.mantissa()),
        num_bigint::BigInt::from(10u8).pow(d.scale()),
    )
}

/// `rust_decimal`'s `a + b` (or `a - b`) as it returned it, only when it
/// is the exact result: the reference the kernel's checked sum must equal.
fn exact_only(got: Option<Decimal>, a: Decimal, b: Decimal, add: bool) -> Option<Decimal> {
    let exact = if add {
        rat(a) + rat(b)
    } else {
        rat(a) - rat(b)
    };
    got.filter(|g| rat(*g) == exact)
}

fn shown(d: Option<Decimal>) -> String {
    d.map_or("None".into(), |d| format!("{d} (scale {})", d.scale()))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(20000))]

    /// The routed sites give the figures, scales and refusals the
    /// hand-written chains they replaced did (K-U8's compatibility check),
    /// compared as strings, except that a step whose result `rust_decimal`
    /// had to round is now refused (K-U10): the old `AgingRow::total`, the
    /// old `control_check` difference, the old `Reconciliation::total`,
    /// each step filtered through an exact rational reference.
    #[test]
    fn kernel_matches_the_replaced_chains(f in prop::array::uniform5(arb_decimal())) {
        let old_aging = [f[1], f[2], f[3], f[4]]
            .into_iter()
            .try_fold(f[0], |a, b| exact_only(Decimal::checked_add(a, b), a, b, true));
        prop_assert_eq!(
            shown(kernel::aging_total(f[0], f[1], f[2], f[3], f[4])),
            shown(old_aging)
        );
        let old_diff = exact_only(Decimal::checked_sub(f[0], f[1]), f[0], f[1], false)
            .and_then(|d| exact_only(Decimal::checked_sub(d, f[2]), d, f[2], false));
        prop_assert_eq!(
            shown(kernel::control_difference(f[0], f[1], f[2])),
            shown(old_diff)
        );
        let old_recon = exact_only(Decimal::checked_add(f[0], f[1]), f[0], f[1], true)
            .and_then(|x| exact_only(Decimal::checked_add(x, f[2]), x, f[2], true))
            .and_then(|x| exact_only(Decimal::checked_sub(x, f[3]), x, f[3], false));
        prop_assert_eq!(
            shown(kernel::reconciliation_total(f[0], f[1], f[2], f[3])),
            shown(old_recon)
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    /// The worksheet's identity (BS-partner-4), at both types: a book row
    /// and a tax row that differ only by the three causes differ at the
    /// ending by `reconciliation_total`. (Kani did not finish this one at
    /// nine terms, 500 s, nor at seven, 110 s; Lean's
    /// `Control.reconciliation_identity` carries it at any size, and this
    /// property checks it on the engine's own type.)
    #[test]
    fn reconciliation_worksheet_identity(
        f in prop::array::uniform5(arb_cents()),
        g in prop::array::uniform5(arb_cents()),
    ) {
        let (bb, tb, cash, prop_book, income) = (f[0], f[1], f[2], f[3], f[4]);
        let (other, dist_book, gain, without, distributed) = (g[0], g[1], g[2], g[3], g[4]);
        let book = bb + cash + prop_book + income + other - dist_book;
        let tax = tb + cash + (prop_book - gain - without) + income + other
            - (dist_book - distributed);
        prop_assert_eq!(
            kernel::reconciliation_total(bb - tb, gain, without, distributed),
            Some(book - tax)
        );
        prop_assert_eq!(
            kernel::reconciliation_total(dec(bb - tb), dec(gain), dec(without), dec(distributed)),
            Some(dec(book - tax))
        );
    }
}
