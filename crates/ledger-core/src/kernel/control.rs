//! U8: control, aging and reconciliation totals (kernel sweep).
//!
//! Three identities the books state by name, each a short chain of exact
//! additions and subtractions: an aging row's five buckets sum to what is
//! owed (BS-arap-1), a control account's balance less its documents and its
//! unattached lines is its difference, which is always zero (BS-arap-2), and
//! a member's book capital less tax capital is their reconciliation total
//! (BS-partner-4). Naming them here puts them under proof rather than
//! leaving each a hand-written sum at its call site. The sums reuse
//! [`super::total`]; the chains of differences are checked one step at a
//! time, so none of them can panic or round.

use super::{Amount, total};

/// What an aging row owes: its five buckets, current, 1-30, 31-60, 61-90
/// and over 90 days, added exactly.
///
/// ```text
/// GOAL: an aging row's total is exactly the sum of its five buckets, so
///       the buckets partition what is owed and no bucket is lost or
///       counted twice in the total.
/// SOURCE: BS-arap-1 (an aging puts each open balance in exactly one
///         bucket, so the buckets sum to the open balances); BS-entry-3,
///         BS-entry-4 (exact, no panic).
/// INVARIANTS:
///   1. Some(t): t = current + b30 + b60 + b90 + b90p, exactly.
///   2. The order of the buckets does not change a total that exists
///      for in-range figures; aging_total of five zeros is Some(0).
///   3. None when a running sum cannot be represented; NEVER a rounded or
///      wrapped figure, never a panic.
/// SCENARIOS:
///   1. IF a customer owes 100.00 current and 40.00 over 90 days past due:
///      - SHOULD total 140.00
///      BECAUSE every bucket counts once.
///   2. IF the buckets are Decimal::MAX and 1 in any two places:
///      - SHOULD be None
///      BECAUSE the sum is not representable.
/// ```
pub fn aging_total<T: Amount>(current: T, b30: T, b60: T, b90: T, b90p: T) -> Option<T> {
    total([current, b30, b60, b90, b90p])
}

/// A control account's difference: its balance less what its documents
/// still owe on it, less its unattached lines.
///
/// ```text
/// GOAL: the control check's difference is the balance less the documents
///       and the unattached lines, exactly; it is zero whenever the
///       balance is those two sums, which it always is, as both sides sum
///       the same lines.
/// SOURCE: BS-arap-2 (a control account's balance equals the sum of its
///         documents plus its unattached lines); BS-ledger-8; BS-entry-4.
/// INVARIANTS:
///   1. Some(d): d = balance - documents - unattached, exactly.
///   2. control_difference(documents + unattached, documents, unattached)
///      = Some(0), and conversely Some(0) means balance = documents +
///      unattached: the difference vanishes exactly when the account
///      agrees with its subledger.
///   3. None when a step cannot be represented; NEVER a rounded or
///      wrapped figure, never a panic.
/// SCENARIOS:
///   1. IF the balance is 150.00, the documents 100.00 and one unattached
///      line 50.00:
///      - SHOULD be 0
///      BECAUSE 150.00 = 100.00 + 50.00.
///   2. IF the balance is 100.00 and the documents 90.00 with nothing
///      unattached:
///      - SHOULD be 10.00
///      BECAUSE the account holds 10.00 no document explains.
/// ```
pub fn control_difference<T: Amount>(balance: T, documents: T, unattached: T) -> Option<T> {
    balance
        .checked_sub(documents)
        .and_then(|d| d.checked_sub(unattached))
}

/// A member's book capital less their tax capital, by cause: the
/// beginning difference, plus the built-in gain, plus property without a
/// basis, less property distributed (all at book).
///
/// ```text
/// GOAL: the reconciliation total is exactly book ending less tax ending
///       as far as the books know it: what contributing and distributing
///       property does to the two capitals is the only way they part.
/// SOURCE: BS-partner-4 (the book-to-tax capital reconciliation: book
///         ending less tax ending = beginning (book less tax) + built-in
///         gain share + property without a basis (at book) - property
///         distributed (at book)); BS-asset-4 (the section 704(c)
///         built-in gain); BS-entry-4.
/// INVARIANTS:
///   1. Some(t): t = beginning + built_in_gain + without_basis -
///      distributed, exactly.
///   2. For any book and tax capital rows that differ only by those
///      causes (tax contributions = book contributions less the gain and
///      the property without a basis; tax distributions = book
///      distributions less what was distributed), book ending less tax
///      ending = reconciliation_total (Lean: Bookspec.Reconcile).
///   3. None when a step cannot be represented; NEVER a rounded or
///      wrapped figure, never a panic.
/// SCENARIOS:
///   1. IF two members jointly contribute a laptop worth 1,440.00 whose
///      basis is 1,688.40 (no other activity):
///      - SHOULD give each a total of -124.20
///      BECAUSE book property is 720.00 and tax 844.20.
/// ```
pub fn reconciliation_total<T: Amount>(
    beginning: T,
    built_in_gain: T,
    without_basis: T,
    distributed: T,
) -> Option<T> {
    beginning
        .checked_add(built_in_gain)?
        .checked_add(without_basis)?
        .checked_sub(distributed)
}
