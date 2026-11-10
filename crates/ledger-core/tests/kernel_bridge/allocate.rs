//! U3 bridge properties: proportional shares and weighted splits (kernel
//! sweep).
//!
//! `proofs/allocate.rs` proves the split on `i64` with fixed weights and
//! totals; the engine runs the same code on `Decimal`. These properties
//! check that both instantiations agree, that the `Decimal` split agrees
//! with an independent exact `i128` reference (integer products and
//! remainders, sharing no code with the kernel) for weights summing to one
//! and to arbitrary totals, that each leftover policy matches its own
//! reference, and the edges: zero weights, one member, many members,
//! `Decimal::MAX` and scale 28, each `None` or exact, never a panic.

use ledger_core::kernel::{
    Leftover, Rounding, round_places, round_step, share_floor_with, share_of,
    split_by_weights_into, split_by_weights_with,
};
use proptest::prelude::*;
use rust_decimal::Decimal;

use super::dec;

fn parse(text: &str) -> Decimal {
    text.parse().expect("a decimal literal")
}

/// An independent exact reference for a largest-remainder split, all in
/// the smallest unit: floor each exact share to `step`, then hand the
/// shortfall out by largest remainder (ties to the earlier part). No
/// kernel code and no `Decimal`.
fn reference(whole: i128, weights: &[i128], total: i128, step: i128) -> Option<Vec<i128>> {
    if total == 0 {
        return Some(vec![0; weights.len()]);
    }
    if total < 0 || step <= 0 {
        return None;
    }
    let mut parts = Vec::with_capacity(weights.len());
    let mut rems = Vec::with_capacity(weights.len());
    for &w in weights {
        let num = whole.checked_mul(w)?;
        let part = num.div_euclid(total).div_euclid(step).checked_mul(step)?;
        parts.push(part);
        rems.push(num.checked_sub(part.checked_mul(total)?)?);
    }
    let mut shortfall = whole.checked_sub(parts.iter().sum::<i128>())?;
    if shortfall < 0 {
        return None;
    }
    let mut units = 0usize;
    while shortfall >= step {
        shortfall -= step;
        units += 1;
        if units == parts.len() {
            return None;
        }
    }
    for i in 0..parts.len() {
        let rank = (0..rems.len())
            .filter(|&j| rems[j] > rems[i] || (rems[j] == rems[i] && j < i))
            .count();
        let mut add = 0i128;
        if rank < units {
            add += step;
        }
        if rank == 0 {
            add += shortfall;
        }
        parts[i] = parts[i].checked_add(add)?;
    }
    Some(parts)
}

/// The first/last-absorbs reference: floor each share to `step`, then give
/// the first or last part the whole less the others.
fn reference_absorb(
    whole: i128,
    weights: &[i128],
    total: i128,
    step: i128,
    first: bool,
) -> Option<Vec<i128>> {
    if total == 0 {
        return Some(vec![0; weights.len()]);
    }
    let mut parts = Vec::with_capacity(weights.len());
    for &w in weights {
        let num = whole.checked_mul(w)?;
        parts.push(num.div_euclid(total).div_euclid(step).checked_mul(step)?);
    }
    if parts.is_empty() {
        return Some(parts);
    }
    let others: i128 = if first {
        parts[1..].iter().sum()
    } else {
        parts[..parts.len() - 1].iter().sum()
    };
    let absorbed = whole.checked_sub(others)?;
    if first {
        parts[0] = absorbed;
    } else {
        *parts.last_mut()? = absorbed;
    }
    Some(parts)
}

/// The exact floor to `step` and its remainder, in integers.
fn reference_floor(whole: i128, weight: i128, total: i128, step: i128) -> (i128, i128) {
    let num = whole * weight;
    let part = (num / total / step) * step;
    (part, num - part * total)
}

/// The engine's split at `places`, largest remainder, through
/// `round_places` (the call sites' scale-preserving rounding).
fn decimal_split(
    whole: Decimal,
    weights: &[Decimal],
    total: Decimal,
    places: u32,
    policy: Leftover,
) -> Option<Vec<Decimal>> {
    split_by_weights_with(
        whole,
        weights,
        total,
        Decimal::new(1, places),
        |x| round_places(x, places, Rounding::TowardZero),
        policy,
    )
}

/// A Decimal that is a whole number of cents, as that count.
fn cents(parts: &[Decimal]) -> Vec<i128> {
    parts
        .iter()
        .map(|d| (d * Decimal::from(100)).normalize().mantissa())
        .collect()
}

/// A Decimal that is a whole number, as that integer.
fn integer(d: Decimal) -> i128 {
    d.normalize().mantissa()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    /// The `i64` kernel (the Kani model, in cents) and the `Decimal`
    /// kernel (the engine, in dollars) agree, value for value, where the
    /// shares divide exactly: the `i64` quotient is then the same exact
    /// share, so the largest-remainder recipient is the same. (Where a
    /// share does not divide, the `i64` quotient truncates and the
    /// Decimal one keeps the fraction; the property below carries the
    /// laws Kani proves, which do not depend on the recipient.)
    #[test]
    fn i64_and_decimal_agree(k in 0i64..=100_000) {
        let whole = k * 10;
        let weights = [2i64, 3, 5];
        let narrow = split_by_weights_with(
            whole,
            &weights,
            10,
            1,
            |x| round_step(x, 1, Rounding::TowardZero),
            Leftover::LargestRemainder,
        )
        .unwrap();
        let wide = decimal_split(
            dec(whole),
            &[dec(2), dec(3), dec(5)],
            dec(10),
            2,
            Leftover::LargestRemainder,
        )
        .unwrap();
        prop_assert_eq!(narrow.into_iter().map(dec).collect::<Vec<_>>(), wide);
    }

    /// The laws Kani proves on `i64` hold on `Decimal` for every whole,
    /// remainder or not: the parts sum to the whole, each is within one
    /// step of its exact share (scaled by the total), and none exceeds
    /// the whole.
    #[test]
    fn decimal_split_satisfies_the_kani_laws(whole in 0i64..=1_000_000) {
        let parts = decimal_split(
            dec(whole),
            &[dec(2), dec(3), dec(5)],
            dec(10),
            2,
            Leftover::LargestRemainder,
        )
        .unwrap();
        prop_assert_eq!(parts.iter().sum::<Decimal>(), dec(whole));
        for (i, &p) in parts.iter().enumerate() {
            prop_assert!(p >= Decimal::ZERO && p <= dec(whole));
            let drift = p * Decimal::from(10) - dec(whole) * Decimal::from([2, 3, 5][i]);
            let bound = Decimal::new(1, 1);
            prop_assert!(drift > -bound && drift < bound);
        }
    }

    /// The exact `i128` reference: weights over an arbitrary total (2/3/5
    /// of 10), the engine's cents.
    #[test]
    fn decimal_matches_the_exact_reference(whole in 0i64..=1_000_000) {
        let want = reference(i128::from(whole), &[2, 3, 5], 10, 1).unwrap();
        let got = decimal_split(
            dec(whole),
            &[dec(2), dec(3), dec(5)],
            dec(10),
            2,
            Leftover::LargestRemainder,
        )
        .unwrap();
        prop_assert_eq!(cents(&got), want);
    }

    /// The exact reference with weights summing to one (0.5 / 0.3 / 0.2):
    /// the total is one, so the exact share is the product, and the
    /// reference sees the same weights as 5/3/2 of 10.
    #[test]
    fn decimal_matches_the_exact_reference_weights_sum_to_one(whole in 0i64..=1_000_000) {
        let want = reference(i128::from(whole), &[5, 3, 2], 10, 1).unwrap();
        let got = decimal_split(
            dec(whole),
            &[parse("0.5"), parse("0.3"), parse("0.2")],
            Decimal::ONE,
            2,
            Leftover::LargestRemainder,
        )
        .unwrap();
        prop_assert_eq!(cents(&got), want);
    }

    /// First and last absorb: the same floors, the first or last part
    /// taking the whole less the others.
    #[test]
    fn leftovers_at_each_policy(whole in 0i64..=1_000_000) {
        let weights = [2i64, 3, 5];
        let first = split_by_weights_with(
            whole,
            &weights,
            10,
            1,
            |x| round_step(x, 1, Rounding::TowardZero),
            Leftover::FirstAbsorbs,
        )
        .unwrap();
        let last = split_by_weights_with(
            whole,
            &weights,
            10,
            1,
            |x| round_step(x, 1, Rounding::TowardZero),
            Leftover::LastAbsorbs,
        )
        .unwrap();
        prop_assert_eq!(
            first.iter().map(|&p| i128::from(p)).collect::<Vec<_>>(),
            reference_absorb(i128::from(whole), &[2, 3, 5], 10, 1, true).unwrap()
        );
        prop_assert_eq!(
            last.iter().map(|&p| i128::from(p)).collect::<Vec<_>>(),
            reference_absorb(i128::from(whole), &[2, 3, 5], 10, 1, false).unwrap()
        );
        prop_assert_eq!(first.iter().sum::<i64>(), whole);
        prop_assert_eq!(last.iter().sum::<i64>(), whole);
    }

    /// Zero weights: a positive total hands the whole out by largest
    /// remainder while it fits one unit per part, and refuses past that;
    /// a zero total is zeros. One member takes everything; many members
    /// agree with the reference.
    #[test]
    fn zero_weights_one_member_and_many(
        whole in 0i64..=100_000,
        members in 1usize..=10,
    ) {
        let zeros = vec![Decimal::ZERO; 3];
        let got = decimal_split(dec(whole), &zeros, dec(10), 2, Leftover::LargestRemainder);
        let want = reference(i128::from(whole), &[0, 0, 0], 10, 1);
        prop_assert_eq!(got.map(|p| cents(&p)), want);

        let one = decimal_split(dec(whole), &[Decimal::ONE], Decimal::ONE, 2, Leftover::LargestRemainder).unwrap();
        prop_assert_eq!(one, vec![dec(whole)]);

        let weights = vec![Decimal::ONE; members];
        let want = reference(i128::from(whole), &vec![1i128; members], members as i128, 1).unwrap();
        let got = decimal_split(dec(whole), &weights, Decimal::from(members), 2, Leftover::LargestRemainder).unwrap();
        prop_assert_eq!(cents(&got), want);
    }

    /// The exact reference ranks the exact remainders, and so does the
    /// kernel (K-U10): with any weights, ties included (equal weights,
    /// the 4/1/1 shape, a whole that leaves equal remainders), the split
    /// is the reference's, at whole units and at the cent.
    #[test]
    fn largest_remainder_ranks_like_the_exact_reference(
        whole in 0i64..=100_000,
        weights in prop::collection::vec(1i64..=9, 2..=5),
    ) {
        let total: i64 = weights.iter().sum();
        let decimal_weights: Vec<Decimal> = weights.iter().copied().map(Decimal::from).collect();
        let wide: Vec<i128> = weights.iter().map(|&w| i128::from(w)).collect();
        for places in [0u32, 2] {
            let got = decimal_split(
                Decimal::new(whole, places),
                &decimal_weights,
                Decimal::from(total),
                places,
                Leftover::LargestRemainder,
            )
            .unwrap();
            let want = reference(i128::from(whole), &wide, i128::from(total), 1).unwrap();
            let got = if places == 0 { got.iter().map(|&p| integer(p)).collect() } else { cents(&got) };
            prop_assert_eq!(got, want);
        }
    }

    /// `share_floor_with` is the exact floor of the rational, even when
    /// the division does not terminate (total 3, 7, 11): the quotient's
    /// 28-digit rounding is corrected by multiplication.
    #[test]
    fn share_floor_matches_the_exact_reference(
        whole in 0i64..=1_000_000,
        weight in 0i64..=1_000,
        total in 1i64..=1_000,
        step in 1i64..=100,
    ) {
        let want = reference_floor(i128::from(whole), i128::from(weight), i128::from(total), i128::from(step));
        let got = share_floor_with(
            Decimal::from(whole),
            Decimal::from(weight),
            Decimal::from(total),
            Decimal::from(step),
            |x| round_step(x, Decimal::from(step), Rounding::TowardZero),
        )
        .unwrap();
        prop_assert_eq!((integer(got.0), integer(got.1)), want);
    }
}

/// The hand-checked table: the shapes the six call sites produce, at the
/// scale they produce them.
#[test]
fn split_at_the_call_sites_scales() {
    // k1's property split: 401 by credits 750/250 to cents.
    let got = decimal_split(
        Decimal::from(401),
        &[Decimal::from(750), Decimal::from(250)],
        Decimal::from(1000),
        2,
        Leftover::LargestRemainder,
    )
    .unwrap();
    assert_eq!(got, vec![parse("300.75"), parse("100.25")]);

    // A cent split 50/50 (the tied cent to the first).
    let got = decimal_split(
        dec(1),
        &[parse("0.5"), parse("0.5")],
        Decimal::ONE,
        2,
        Leftover::LargestRemainder,
    )
    .unwrap();
    assert_eq!(got, vec![parse("0.01"), parse("0.00")]);

    // Whole yen: 101 by 51/51, the tied yen to the first.
    let got = decimal_split(
        Decimal::from(101),
        &[Decimal::from(51), Decimal::from(51)],
        Decimal::from(102),
        0,
        Leftover::LargestRemainder,
    )
    .unwrap();
    assert_eq!(got, vec![Decimal::from(51), Decimal::from(50)]);

    // The exact split with no rounding: amount x share, the last plugs.
    let got = split_by_weights_with(
        Decimal::from(7),
        &[parse("0.5"), parse("0.5")],
        Decimal::ONE,
        Decimal::ONE,
        Some,
        Leftover::LastAbsorbs,
    )
    .unwrap();
    assert_eq!(got, vec![parse("3.5"), parse("3.5")]);
}

/// The edges: `Decimal::MAX`, a product past it, a zero total, scale 28.
/// Each is `None` or exact, never a panic.
#[test]
fn decimal_edges_are_none_or_exact() {
    assert_eq!(share_of(Decimal::MAX, Decimal::from(2), Decimal::ONE), None);
    assert_eq!(
        share_of(Decimal::MAX, Decimal::ONE, Decimal::MAX),
        Some(Decimal::ONE)
    );
    assert_eq!(share_of(Decimal::ONE, Decimal::ONE, Decimal::ZERO), None);

    assert_eq!(
        share_floor_with(
            Decimal::MAX,
            Decimal::from(2),
            Decimal::ONE,
            Decimal::ONE,
            Some
        ),
        None
    );
    assert_eq!(
        split_by_weights_with(
            Decimal::MAX,
            &[Decimal::from(2)],
            Decimal::ONE,
            Decimal::ONE,
            Some,
            Leftover::LargestRemainder
        ),
        None
    );

    // The finest amount: the quotient may not be representable, but the
    // split is never a panic and, when it is Some, still sums.
    let finest = parse("0.0000000000000000000000000001");
    let got = split_by_weights_with(
        finest,
        &[Decimal::ONE, Decimal::ONE],
        Decimal::from(2),
        finest,
        Some,
        Leftover::LargestRemainder,
    );
    if let Some(parts) = got {
        assert_eq!(parts.iter().sum::<Decimal>(), finest);
    }

    // A zero total is zeros, whatever the weights.
    assert_eq!(
        split_by_weights_with(
            dec(5),
            &[dec(1), dec(2)],
            Decimal::ZERO,
            Decimal::ONE,
            |x| round_step(x, Decimal::ONE, Rounding::TowardZero),
            Leftover::LargestRemainder
        ),
        Some(vec![Decimal::ZERO, Decimal::ZERO])
    );
}

/// The absorbing parts keep the scale the pre-U3 code produced. Decimal's
/// equality ignores scale, so these compare the strings: the old
/// `shares::split_first` added the shortfall to the first rounded share
/// (4860 by 6/0.7/0.5 gave `4050`, not `4050.00`), and 902 by
/// 1/0.000001/0.000001 gave `902.00` there; the kernel now computes the
/// same expression, not the algebraically equal `whole − Σ rest`.
#[test]
fn the_absorbing_parts_keep_the_old_scale() {
    let text = |parts: &[Decimal]| parts.iter().map(Decimal::to_string).collect::<Vec<_>>();
    let weights = [parse("6"), parse("0.7"), parse("0.5")];
    let got = split_by_weights_with(
        parse("4860"),
        &weights,
        parse("7.2"),
        parse("0.01"),
        |x| round_places(x, 2, Rounding::TowardZero),
        Leftover::FirstAbsorbs,
    )
    .unwrap();
    assert_eq!(text(&got), ["4050", "472.50", "337.50"]);

    let weights = [Decimal::ONE, parse("0.000001"), parse("0.000001")];
    let got = split_by_weights_with(
        parse("902"),
        &weights,
        parse("1.000002"),
        parse("0.01"),
        |x| round_places(x, 2, Rounding::TowardZero),
        Leftover::FirstAbsorbs,
    )
    .unwrap();
    assert_eq!(text(&got), ["902.00", "0.00", "0.00"]);

    // LastAbsorbs is the old `plug_last` (and the no-places conventions
    // split): the last part is the whole less the others, same scale.
    let got = split_by_weights_with(
        parse("4860"),
        &weights[..1],
        Decimal::ONE,
        Decimal::ONE,
        Some,
        Leftover::LastAbsorbs,
    )
    .unwrap();
    assert_eq!(text(&got), ["4860"]);
    let got = split_by_weights_with(
        parse("7"),
        &[parse("0.5"), parse("0.5")],
        Decimal::ONE,
        Decimal::ONE,
        Some,
        Leftover::LastAbsorbs,
    )
    .unwrap();
    assert_eq!(text(&got), ["3.5", "3.5"]);
}

/// The buffer API and the allocating one agree, including scale: the
/// bridge calls `split_by_weights_into` itself (fixed arrays), as the
/// traceability gate requires.
#[test]
fn split_into_matches_the_allocating_split() {
    let weights = [parse("6"), parse("0.7"), parse("0.5")];
    let mut parts = [Decimal::ZERO; 3];
    let mut remainders = [Decimal::ZERO; 3];
    split_by_weights_into(
        parse("4860"),
        &weights,
        parse("7.2"),
        parse("0.01"),
        |x| round_places(x, 2, Rounding::TowardZero),
        Leftover::FirstAbsorbs,
        &mut parts,
        &mut remainders,
    )
    .unwrap();
    let with = split_by_weights_with(
        parse("4860"),
        &weights,
        parse("7.2"),
        parse("0.01"),
        |x| round_places(x, 2, Rounding::TowardZero),
        Leftover::FirstAbsorbs,
    )
    .unwrap();
    assert_eq!(
        parts.iter().map(Decimal::to_string).collect::<Vec<_>>(),
        with.iter().map(Decimal::to_string).collect::<Vec<_>>()
    );
}

/// K-U10 (the owner's decision): a split ranks the exact remainders, so an
/// exact tie goes to the earlier part, as the docs and the Lean model say.
/// 20.00 split 4/1/1 over 6 is 13.34/3.33/3.33 (the rounded quotients'
/// ranking gave 13.33/3.34/3.33), and 20 at whole units is 14/3/3 (it gave
/// 13/4/3). Equal weights tie on every odd cent and give it to the first.
#[test]
fn ties_go_to_the_earlier_part() {
    let weights = [Decimal::from(4), Decimal::ONE, Decimal::ONE];
    let got = decimal_split(
        parse("20.00"),
        &weights,
        Decimal::from(6),
        2,
        Leftover::LargestRemainder,
    )
    .unwrap();
    assert_eq!(
        got.iter().map(Decimal::to_string).collect::<Vec<_>>(),
        ["13.34", "3.33", "3.33"]
    );
    let got = decimal_split(
        Decimal::from(20),
        &weights,
        Decimal::from(6),
        0,
        Leftover::LargestRemainder,
    )
    .unwrap();
    assert_eq!(
        got.iter().map(Decimal::to_string).collect::<Vec<_>>(),
        ["14", "3", "3"]
    );
    // A negative total keeps the share's own order: ties still go first.
    let neg = [Decimal::from(-4), Decimal::from(-1), Decimal::from(-1)];
    let got = decimal_split(
        parse("20.00"),
        &neg,
        Decimal::from(-6),
        2,
        Leftover::LargestRemainder,
    )
    .unwrap();
    assert_eq!(
        got.iter().map(Decimal::to_string).collect::<Vec<_>>(),
        ["13.34", "3.33", "3.33"]
    );
}

/// The review's hang: `0.1113 × 5909471247797156928749568911.8 ÷
/// 0.11134381908633486049673` to the cent put the rounded product far
/// past the exact floor, and the correction walked without end. It now
/// refuses past the cap, quickly; the other two buckets keep exact floors.
#[test]
fn share_floor_refuses_past_the_correction_cap() {
    let b = [
        parse("0.1113"),
        parse("0.00004381908373486049673"),
        parse("0.0000000000026"),
    ];
    let v = parse("-5909471247797156928749568911.8");
    let total = b
        .iter()
        .copied()
        .try_fold(Decimal::ZERO, |s, x| s.checked_add(x))
        .unwrap();
    let started = std::time::Instant::now();
    let got = share_floor_with(b[0].abs(), v.abs(), total, parse("0.01"), |x| {
        round_places(x, 2, Rounding::TowardZero)
    });
    assert_eq!(got, None);
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
    for bucket in &b[1..] {
        assert!(
            share_floor_with(bucket.abs(), v.abs(), total, parse("0.01"), |x| {
                round_places(x, 2, Rounding::TowardZero)
            },)
            .is_some()
        );
    }
}

/// The K-U10 finding: at `Decimal`'s mantissa edge, `checked_div`'s
/// rounding can move a share across a step, and `checked_mul`'s rounding
/// hides it from the exact-floor correction. The exact share is
/// `whole / total` just above `whole`; the kernel's quotient rounds up to
/// `whole + 0.01`, so the largest-remainder split refuses (a part above
/// the whole is never emitted), and `share_floor_with` keeps the rounded
/// step because the comparison product rounds to the same figure. No
/// posted line reaches this edge (the line cap is 10^15); it is pinned so
/// a future change to the division or multiplication cannot move it
/// silently.
#[test]
fn finding_the_quotients_rounding_can_cross_a_step() {
    let whole = Decimal::try_from_i128_with_scale(7923000000000000000000000099i128, 2).unwrap();
    let total = Decimal::try_from_i128_with_scale(9999999999999999999999999999i128, 28).unwrap();
    let rounded = Decimal::try_from_i128_with_scale(7923000000000000000000000100i128, 2).unwrap();
    assert_eq!(share_of(whole, Decimal::ONE, total), Some(rounded));
    assert_eq!(
        share_floor_with(whole, Decimal::ONE, total, Decimal::new(1, 2), |x| {
            round_places(x, 2, Rounding::TowardZero)
        })
        .map(|(part, _)| part),
        Some(rounded)
    );
    assert_eq!(
        decimal_split(whole, &[Decimal::ONE], total, 2, Leftover::LargestRemainder),
        None
    );
}
