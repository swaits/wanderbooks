//! U8 proof harnesses: control and aging (kernel sweep).
//!
//! Empty in U0: the unit adds its Kani harnesses here; `just prove` scans
//! every file in `proofs/`, so a harness needs no change to the module
//! root or the recipe.

use super::any_amount;
use crate::kernel::{self, Amount};

/// An aging row's total is exactly the sum of its five buckets, for
/// **any** five `i64` figures (the edge included): `Some(t)` means `t` is
/// the true sum (checked in `i128`, which holds any five `i64`s), and
/// whenever every bucket is within a journal line's range the total
/// exists. Five fixed figures, no `Vec`; no bound is fixed beyond the
/// five buckets an aging row has.
#[kani::proof]
fn aging_total_is_sum() {
    let b: [i64; 5] = core::array::from_fn(|_| kani::any());
    let spec: i128 = b.iter().map(|&v| i128::from(v)).sum();
    let got = kernel::aging_total(b[0], b[1], b[2], b[3], b[4]);
    if let Some(t) = got {
        assert_eq!(i128::from(t), spec);
        kani::cover!(t != 0, "a nonzero total exists");
    }
    // `None` only on overflow: whenever every running prefix fits in
    // `i64` (not only inside the line-range box), the total exists.
    let mut run: i128 = 0;
    let mut fits = true;
    for &v in &b {
        run += i128::from(v);
        fits &= i64::try_from(run).is_ok();
    }
    if fits {
        assert!(got.is_some(), "every prefix fits, so the total exists");
        kani::cover!(run != 0, "a nonzero total from fitting prefixes");
    }
    if b.iter().all(|v| v.in_line_range()) {
        assert!(got.is_some(), "in-range buckets always total");
        kani::cover!(
            b.iter().all(|&v| v != 0) && got.is_some(),
            "five nonzero buckets"
        );
    }
    // The all-in-range case on its own: the total is the sum, for a
    // positive owing and negative (credit) buckets alike.
    let a: [i64; 5] = core::array::from_fn(|_| any_amount());
    let spec: i128 = a.iter().map(|&v| i128::from(v)).sum();
    let t = kernel::aging_total(a[0], a[1], a[2], a[3], a[4]).expect("in-range figures total");
    assert_eq!(i128::from(t), spec);
    kani::cover!(
        a.iter().any(|&v| v < 0) && a.iter().any(|&v| v > 0),
        "mixed signs"
    );
}

/// The control difference is `balance - documents - unattached`, exactly,
/// and it vanishes exactly when the balance is the documents plus the
/// unattached lines: the BS-arap-2 identity. For any three `i64`s: `Some(d)`
/// means `d` is the true difference (checked in `i128`); a balance that is
/// the sum of the other two (when that sum fits) differs by `Some(0)`; and
/// `Some(0)` means the balance is that sum.
#[kani::proof]
fn control_difference_identity() {
    let (balance, docs, unattached): (i64, i64, i64) = (kani::any(), kani::any(), kani::any());
    let spec = i128::from(balance) - i128::from(docs) - i128::from(unattached);
    let got = kernel::control_difference(balance, docs, unattached);
    // `None` only on overflow: whenever both steps fit in `i64`, it exists.
    let first = i128::from(balance) - i128::from(docs);
    if i64::try_from(first).is_ok() && i64::try_from(spec).is_ok() {
        assert!(got.is_some(), "both steps fit, so the difference exists");
        kani::cover!(spec != 0, "a nonzero difference from fitting steps");
    }
    if let Some(d) = got {
        assert_eq!(i128::from(d), spec);
        if d == 0 {
            assert_eq!(
                i128::from(balance),
                i128::from(docs) + i128::from(unattached)
            );
            kani::cover!(docs != 0 && unattached != 0, "agrees with both parts");
        } else {
            kani::cover!(true, "a nonzero difference");
        }
    }
    // The agreeing books: a balance of the documents plus the unattached
    // lines (in range, so the sum fits) has no difference.
    let (docs, unattached) = (any_amount(), any_amount());
    let balance = docs + unattached;
    assert_eq!(
        kernel::control_difference(balance, docs, unattached),
        Some(0)
    );
    kani::cover!(
        docs > 0 && unattached < 0,
        "documents with a credit unattached"
    );
}

/// The reconciliation total is `beginning + built_in_gain + without_basis
/// - distributed`, exactly, for **any** four `i64`s when it exists
/// (checked in `i128`); and whenever the four are within a journal line's
/// range it exists.
#[kani::proof]
fn reconciliation_total_identity() {
    let f: [i64; 4] = core::array::from_fn(|_| kani::any());
    let spec = i128::from(f[0]) + i128::from(f[1]) + i128::from(f[2]) - i128::from(f[3]);
    let got = kernel::reconciliation_total(f[0], f[1], f[2], f[3]);
    if let Some(t) = got {
        assert_eq!(i128::from(t), spec);
        kani::cover!(t != 0, "a nonzero total");
    }
    // `None` only on overflow: whenever every running prefix fits in
    // `i64`, the total exists.
    let p1 = i128::from(f[0]) + i128::from(f[1]);
    let p2 = p1 + i128::from(f[2]);
    if [p1, p2, spec].iter().all(|&p| i64::try_from(p).is_ok()) {
        assert!(got.is_some(), "every prefix fits, so the total exists");
        kani::cover!(spec != 0, "a nonzero total from fitting prefixes");
    }
    if f.iter().all(|v| v.in_line_range()) {
        assert!(got.is_some());
    }
}
