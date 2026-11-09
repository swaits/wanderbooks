//! U7 proof harnesses: payroll arithmetic (kernel sweep).
//!
//! `kernel::net_pay` on `i64`: net is gross less the deductions exactly,
//! it is never negative when they do not exceed gross, and a payroll
//! plan built from net pay and totals balances through `check_lines`.
//!
//! A payroll line carries five employee deductions (federal, state,
//! Social Security, Medicare, other), so the deduction arrays are fixed
//! at five. The plan harness carries two employees and the plan's eight
//! totals (gross, employer FICA, the five payables, net), each a kernel
//! total of the two lines, in a fixed array of eight `(Side, amount)`
//! lines. Amounts run to a few thousand units: the law is about the
//! shape of the subtraction and the balance, not the magnitude, and
//! every total stays far inside the `i64` line range (2^40) that
//! `check_lines` requires. Lean proves the same laws at any size.
//!
//! ```text
//! GOAL: net pay is gross less the withholdings exactly; it is not
//!       negative when they do not exceed gross; and the payroll plan
//!       (Dr gross + employer FICA, Cr the payables + net) balances.
//! SOURCE: BS-entry-3 (exact amounts), BS-entry-4 (every payroll
//!         figure and every derived figure computed with checked
//!         arithmetic), BS-entry-1 (debits equal credits, the plan
//!         through check_lines).
//! INVARIANTS:
//!   1. net_pay(gross, ds) = gross - sum(ds) whenever it is Some, and
//!      None exactly when a running difference leaves the range.
//!   2. sum(ds) <= gross (ds non-negative) gives a net that is Some and
//!      not negative.
//!   3. The eight plan lines pass check_lines: two or more lines, every
//!      amount positive and in range, debits equal to credits.
//! SCENARIOS:
//!   1. IF the withholdings exceed gross: the net is the exact negative
//!      difference (the engine refuses the line, not the arithmetic).
//!   2. IF a deduction is past the range: SHOULD be None.
//! ```

use crate::Side;
use crate::kernel::{check_lines, net_pay, total};

/// The five employee deductions of one payroll line.
const DEDUCTIONS: usize = 5;

/// One employee's figures for the plan harness: gross, the five
/// withholdings, and the two employer shares.
struct Employee {
    gross: i64,
    deductions: [i64; DEDUCTIONS],
    ss_employer: i64,
    medicare_employer: i64,
}

fn any_employee() -> Employee {
    let gross: i64 = kani::any();
    kani::assume((1..=10_000).contains(&gross));
    let mut deductions = [0i64; DEDUCTIONS];
    for deduction in &mut deductions {
        let d: i64 = kani::any();
        kani::assume((0..=1_000).contains(&d));
        *deduction = d;
    }
    // Net pay is at least one unit, so the plan's cash line is posted.
    kani::assume(total(deductions.iter().copied()).expect("in range") < gross);
    let ss_employer: i64 = kani::any();
    kani::assume((1..=1_000).contains(&ss_employer));
    let medicare_employer: i64 = kani::any();
    kani::assume((1..=1_000).contains(&medicare_employer));
    Employee {
        gross,
        deductions,
        ss_employer,
        medicare_employer,
    }
}

/// Net pay is gross less the deductions: the exact difference of the
/// gross and the deductions' total, for deductions of either sign.
#[kani::proof]
fn net_pay_is_gross_less_deductions() {
    let gross: i64 = kani::any();
    kani::assume((-10_000..=10_000).contains(&gross));
    let mut deductions = [0i64; DEDUCTIONS];
    for deduction in &mut deductions {
        let d: i64 = kani::any();
        kani::assume((-1_000..=1_000).contains(&d));
        *deduction = d;
    }
    let deducted = total(deductions.iter().copied()).expect("in range");
    let net = net_pay(gross, &deductions).expect("in range");
    assert_eq!(net, gross - deducted, "net is gross less the deductions");
    // Past the range the same call is None, never a panic.
    assert!(net_pay(i64::MAX, &[-1]).is_none(), "overflow is refused");
    assert!(net_pay(i64::MIN, &[1]).is_none(), "underflow is refused");
    kani::cover!(deducted > gross, "withholdings exceed gross");
    kani::cover!(net < 0, "a negative net is a value");
}

/// When the (non-negative) deductions do not exceed gross, the net is
/// the gross less their sum and is not negative.
#[kani::proof]
fn net_pay_nonnegative_when() {
    let gross: i64 = kani::any();
    kani::assume((0..=10_000).contains(&gross));
    let mut deductions = [0i64; DEDUCTIONS];
    for deduction in &mut deductions {
        let d: i64 = kani::any();
        kani::assume((0..=2_000).contains(&d));
        *deduction = d;
    }
    let deducted = total(deductions.iter().copied()).expect("in range");
    kani::assume(deducted <= gross);
    let net = net_pay(gross, &deductions).expect("in range");
    assert_eq!(net, gross - deducted, "net is gross less the deductions");
    assert!(net >= 0, "the net is not negative");
    kani::cover!(net == 0, "withholdings can take the whole gross");
    kani::cover!(deducted > 0 && net > 0, "a partial withholding");
}

/// The payroll plan balances through `check_lines`: two employees, the
/// eight totals `payroll_entry_plan` posts (Dr gross, Dr employer FICA,
/// Cr federal, state, Social Security, Medicare and other payables, Cr
/// cash net), every total positive, debits equal to credits exactly.
#[kani::proof]
fn payroll_plan_balances() {
    let employees = [any_employee(), any_employee()];
    let gross = total(employees.iter().map(|e| e.gross)).expect("in range");
    let employer = total(
        employees
            .iter()
            .map(|e| e.ss_employer + e.medicare_employer),
    )
    .expect("in range");
    let federal = total(employees.iter().map(|e| e.deductions[0])).expect("in range");
    let state = total(employees.iter().map(|e| e.deductions[1])).expect("in range");
    let ss = total(employees.iter().map(|e| e.deductions[2] + e.ss_employer)).expect("in range");
    let medicare = total(
        employees
            .iter()
            .map(|e| e.deductions[3] + e.medicare_employer),
    )
    .expect("in range");
    let other = total(employees.iter().map(|e| e.deductions[4])).expect("in range");
    let net = total(
        employees
            .iter()
            .map(|e| net_pay(e.gross, &e.deductions).expect("in range")),
    )
    .expect("in range");
    // The real plan skips a zero total; this harness takes the case
    // where none is zero (gross, employer FICA, both FICA payables and
    // net are positive by construction above).
    kani::assume(federal > 0 && state > 0 && other > 0);
    let plan: [(Side, i64); 8] = [
        (Side::Debit, gross),
        (Side::Debit, employer),
        (Side::Credit, federal),
        (Side::Credit, state),
        (Side::Credit, ss),
        (Side::Credit, medicare),
        (Side::Credit, other),
        (Side::Credit, net),
    ];
    assert!(
        check_lines(&plan).is_ok(),
        "the payroll plan is an acceptable entry"
    );
    kani::cover!(
        employees[0].gross != employees[1].gross,
        "two employees differ"
    );
    kani::cover!(net > gross, "net over two employees exceeds one gross");
}
