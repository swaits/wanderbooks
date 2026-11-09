//! U7 bridge properties: payroll arithmetic (kernel sweep).
//!
//! `proofs/payroll.rs` proves `kernel::net_pay` on `i64` (five
//! deductions, a two-employee plan through `check_lines`); the engine
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
/// mantissas over their common power of ten, in `i128` throughout.
fn net_reference(gross: (i128, u32), deductions: &[(i128, u32)]) -> Option<Decimal> {
    let scale = deductions
        .iter()
        .map(|&(_, s)| s)
        .max()
        .unwrap_or(0)
        .max(gross.1);
    let mut net: i128 = gross.0.checked_mul(10i128.checked_pow(scale - gross.1)?)?;
    for &(mantissa, s) in deductions {
        net = net.checked_sub(mantissa.checked_mul(10i128.checked_pow(scale - s)?)?)?;
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
        prop_assert_eq!(kernel::net_pay(gross, &ds), want);
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
