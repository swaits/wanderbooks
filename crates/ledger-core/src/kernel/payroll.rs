//! U7: payroll arithmetic, withholding and employer taxes (kernel sweep).
//!
//! Net pay is gross less every employee withholding and deduction, exact
//! to the last unit. `payroll::payroll_entry_plan` keeps its account
//! mapping in `payroll.rs` and builds its totals from [`net_pay`] and
//! [`super::total`]/[`super::total_try`]; the plan it builds is checked
//! by [`super::check_lines`] before it posts, as every entry is. There
//! is no tax law here: every withholding number is supplied by the
//! caller, and this file only subtracts what it is given.

use super::Amount;

/// Gross pay less every deduction, in the order given (`gross` less
/// `d1`, less `d2`, and so on), each subtraction exact. `None` the
/// moment a running difference cannot be represented, never a wrapped,
/// rounded or panicking one.
///
/// ```text
/// GOAL: one definition of net pay: gross less the employee's
///       withholdings and deductions, exactly, or refused.
/// SOURCE: BS-entry-3 (amounts are exact; a sum of amounts is their
///         exact sum, so a difference of amounts is their exact
///         difference), BS-entry-4 (every payroll figure a command
///         takes, and every figure an engine derives, is computed with
///         checked arithmetic and refused past exact arithmetic),
///         BS-entry-1 (the payroll plan this feeds balances: debits
///         equal credits); IRS Pub. 15 (docs/spec/sources/irs-payroll.md:
///         the withholdings are the caller's numbers, never a table here).
/// INVARIANTS:
///   1. Some(n) is the exact mathematical `gross` less the sum of the
///      deductions (every running difference was representable, so no
///      step lost anything). No rounding, ever.
///   2. net_pay(gross, []) = Some(gross).
///   3. When the deductions sum to at most `gross`, the net is the
///      gross less that sum and is not negative; the engine refuses a
///      payroll line whose net is negative, so a posted run's net pay
///      is never below zero. A net below zero is still a value here:
///      the refusal is the caller's rule, not arithmetic's.
///   4. None on overflow; NEVER a rounded or wrapped result, never a
///      panic. The fold is left to right, so [MAX, MAX, -MAX] from a
///      zero gross is None while the same figures summed first would
///      cancel: each step must be exact, as `total`'s fold is.
/// SCENARIOS:
///   1. IF gross 5,000.00 and withholdings 750.00, 250.00, 310.00,
///      72.50 and 100.00: SHOULD be 3,517.50.
///   2. IF the withholdings together exceed gross: SHOULD be the exact
///      negative difference, for the caller to refuse as negative net
///      pay, never a clamped zero.
///   3. IF a running difference is past the amount's range: SHOULD be
///      None, never a panic.
/// ```
pub fn net_pay<T: Amount>(gross: T, deductions: &[T]) -> Option<T> {
    let mut net = gross;
    for &deduction in deductions {
        net = net.checked_sub(deduction)?;
    }
    Some(net)
}
