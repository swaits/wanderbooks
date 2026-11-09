//! U7 bridge properties: payroll arithmetic (kernel sweep).
//!
//! `proofs/payroll.rs` proves `kernel::net_pay` on `i64` (five
//! deductions, a one-employee plan through `check_lines`, and `None`
//! exactly when a prefix overflows); the engine
//! runs the same code on `Decimal`. These properties check that the two
//! instantiations agree, that `Decimal` agrees with an independent exact
//! `i128` reference (mantissas over a common power of ten, sharing no
//! code with the kernel), and the edge table: a gross or a running
//! difference past the type's range is `None`, never a panic.

use ledger_core::kernel;
use proptest::prelude::*;
use rust_decimal::Decimal;

fn dec(cents: i64) -> Decimal {
    Decimal::new(cents, 2)
}

/// The exact net reference: gross less the deductions, from the
/// mantissas over their common power of ten, in `i128` throughout. Like
/// the kernel's fold it requires every running difference to be
/// representable, each at the scale it has so far (the largest scale of
/// the gross and the deductions folded in): a prefix past `Decimal`'s
/// range is `None` even if later deductions would bring it back.
fn net_reference(gross: (i128, u32), deductions: &[(i128, u32)]) -> Option<Decimal> {
    let scale = deductions
        .iter()
        .map(|&(_, s)| s)
        .max()
        .unwrap_or(0)
        .max(gross.1);
    let at_common = |(m, s): (i128, u32)| m.checked_mul(10i128.checked_pow(scale - s)?);
    let mut net: i128 = at_common(gross)?;
    let mut so_far = gross.1;
    let representable = |net: i128, so_far: u32| {
        let own = net / 10i128.pow(scale - so_far);
        Decimal::try_from_i128_with_scale(own, so_far).is_ok()
    };
    if !representable(net, so_far) {
        return None;
    }
    for &(mantissa, s) in deductions {
        net = net.checked_sub(at_common((mantissa, s))?)?;
        so_far = so_far.max(s);
        if !representable(net, so_far) {
            return None;
        }
    }
    Decimal::try_from_i128_with_scale(net, scale).ok()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    /// The `i64` kernel (cents) and the `Decimal` kernel (dollars) give
    /// the same net for the same gross and five deductions.
    #[test]
    fn i64_and_decimal_net_pay_agree(
        gross in -1_000_000_000i64..=1_000_000_000,
        deductions in prop::collection::vec(-100_000_000i64..=100_000_000, 5),
    ) {
        let i = kernel::net_pay(gross, &deductions);
        let d = kernel::net_pay(
            dec(gross),
            &deductions.iter().copied().map(dec).collect::<Vec<_>>(),
        );
        prop_assert_eq!(i.map(dec), d);
        prop_assert_eq!(i.map(|n| dec(n).to_string()), d.map(|n| n.to_string()));
    }

    /// `Decimal`'s net is the exact difference, for a gross and
    /// deductions of every sign and scale.
    #[test]
    fn decimal_net_pay_matches_the_exact_reference(
        gross_mantissa in -1_000_000_000i128..=1_000_000_000i128,
        gross_scale in 0u32..=4,
        deductions in prop::collection::vec(
            (-1_000_000_000i128..=1_000_000_000i128, 0u32..=4),
            0..=5,
        ),
    ) {
        let gross = Decimal::try_from_i128_with_scale(gross_mantissa, gross_scale).unwrap();
        let ds: Vec<Decimal> = deductions
            .iter()
            .map(|&(m, s)| Decimal::try_from_i128_with_scale(m, s).unwrap())
            .collect();
        let want = net_reference((gross_mantissa, gross_scale), &deductions);
        // `Decimal` equality ignores scale: compare the rendering too, so
        // the reference's scale (the common one) is the kernel's.
        let got = kernel::net_pay(gross, &ds);
        prop_assert_eq!(got.map(|n| n.to_string()), want.map(|n| n.to_string()));
        prop_assert_eq!(got, want);
    }

    /// Near `Decimal`'s range the reference also overflows, so the
    /// property meets `None` (and the in-range answers beside it), not
    /// only the edge table: the kernel and the reference agree on which
    /// is which.
    #[test]
    fn decimal_net_pay_near_the_range_agrees_with_the_reference(
        gross_offset in 0i128..=3,
        gross_negative in any::<bool>(),
        deductions in prop::collection::vec((0i128..=3, any::<bool>()), 0..=5),
    ) {
        let max = Decimal::MAX.mantissa();
        let with_sign = |offset: i128, negative: bool| {
            if negative { -(max - offset) } else { max - offset }
        };
        let g = with_sign(gross_offset, gross_negative);
        let ds_m: Vec<(i128, u32)> = deductions
            .iter()
            .map(|&(o, n)| (with_sign(o, n), 0u32))
            .collect();
        let gross = Decimal::try_from_i128_with_scale(g, 0).unwrap();
        let ds: Vec<Decimal> = ds_m
            .iter()
            .map(|&(m, s)| Decimal::try_from_i128_with_scale(m, s).unwrap())
            .collect();
        let want = net_reference((g, 0), &ds_m);
        let got = kernel::net_pay(gross, &ds);
        prop_assert_eq!(got.map(|n| n.to_string()), want.map(|n| n.to_string()));
        prop_assert_eq!(got, want);
    }

    /// When the deductions do not exceed gross, the net is the gross
    /// less their sum and is not negative, at both instantiations.
    #[test]
    fn decimal_net_pay_nonnegative_when(
        gross in 0i64..=1_000_000_000,
        deductions in prop::collection::vec(0i64..=100_000_000, 5),
    ) {
        let deducted: i64 = deductions.iter().sum();
        prop_assume!(deducted <= gross);
        let net = kernel::net_pay(
            dec(gross),
            &deductions.iter().copied().map(dec).collect::<Vec<_>>(),
        )
        .expect("in range");
        prop_assert_eq!(net, dec(gross - deducted));
        prop_assert_eq!(net.to_string(), dec(gross - deducted).to_string());
        prop_assert!(net >= Decimal::ZERO);
    }
}

/// The edges: empty deductions, a zero net, and figures past the type's
/// range. Each answer is exact or `None`, never a panic.
#[test]
fn decimal_net_pay_at_the_edges() {
    // No deductions: the net is the gross itself.
    assert_eq!(
        kernel::net_pay(Decimal::new(500_000, 2), &[]),
        Some(Decimal::new(500_000, 2))
    );
    // The payroll scenario: 5,000.00 less 750, 250, 310, 72.50, 100.
    assert_eq!(
        kernel::net_pay(
            Decimal::new(500_000, 2),
            &[
                Decimal::new(75_000, 2),
                Decimal::new(25_000, 2),
                Decimal::new(31_000, 2),
                Decimal::new(7_250, 2),
                Decimal::new(10_000, 2),
            ],
        ),
        Some(Decimal::new(351_750, 2))
    );
    // Withholdings taking the whole gross give exactly zero, and more
    // than the gross give the exact negative difference.
    assert_eq!(
        kernel::net_pay(Decimal::ONE, &[Decimal::ONE]),
        Some(Decimal::ZERO)
    );
    assert_eq!(
        kernel::net_pay(Decimal::ONE, &[Decimal::from(2)]),
        Some(-Decimal::ONE)
    );
    // Past the range, in either direction and mid-fold: None.
    assert_eq!(kernel::net_pay(Decimal::MAX, &[-Decimal::ONE]), None);
    assert_eq!(kernel::net_pay(Decimal::MIN, &[Decimal::ONE]), None);
    assert_eq!(
        kernel::net_pay(Decimal::ZERO, &[Decimal::MAX, Decimal::MAX]),
        None
    );
    // The same edges on the Kani instantiation.
    assert_eq!(kernel::net_pay(i64::MAX, &[-1]), None);
    assert_eq!(kernel::net_pay(i64::MIN, &[1]), None);
    assert_eq!(kernel::net_pay(0i64, &[i64::MAX, i64::MAX]), None);
    assert_eq!(
        kernel::net_pay(5_000i64, &[750, 250, 310, 72, 100]),
        Some(3_518)
    );
}
