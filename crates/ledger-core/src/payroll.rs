//! Payroll: accounting-only payroll runs (no tax tables).
//!
//! A payroll run posts one balanced entry: gross wages hit Salary & Wage
//! Expense, the employer share of FICA hits Payroll Tax Expense, employee
//! withholdings and deductions accumulate in the 2110–2150 payable accounts,
//! and net pay leaves cash. A later [`crate::Ledger::remit_payroll_taxes`]
//! clears the payables when the tax payments go out.
//!
//! There is no tax law here: every withholding number is supplied by the
//! caller. All amounts are exact [`rust_decimal::Decimal`], never floats.

use crate::{AccountId, Currency, EntryId, LedgerError, Money, Side};
use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// Opaque payroll run handle. Created by [`crate::Ledger`]; never forged.
pub type PayrollRunId = u64;

/// One employee's payroll line: gross pay and every withholding and
/// deduction. All amounts must be non-negative; the engine rejects
/// negatives, and rejects the line when withholdings exceed gross.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PayrollLine {
    /// Employee name, for the run record.
    pub employee_name: String,
    /// Gross pay for the period.
    pub gross_pay: Decimal,
    /// Federal income tax withheld.
    pub federal_withholding: Decimal,
    /// State income tax withheld.
    pub state_withholding: Decimal,
    /// Employee share of Social Security tax.
    pub ss_employee: Decimal,
    /// Employee share of Medicare tax.
    pub medicare_employee: Decimal,
    /// Employer share of Social Security tax.
    pub ss_employer: Decimal,
    /// Employer share of Medicare tax.
    pub medicare_employer: Decimal,
    /// Other deductions (retirement, insurance, garnishments, ...).
    pub other_deductions: Decimal,
}

impl PayrollLine {
    /// Gross minus all employee withholdings and deductions. Must be >= 0:
    /// you cannot withhold more than the employee earned.
    pub fn net_pay(&self) -> Decimal {
        self.gross_pay
            - self.federal_withholding
            - self.state_withholding
            - self.ss_employee
            - self.medicare_employee
            - self.other_deductions
    }
}

/// A posted payroll run: the inputs plus the journal entry it posted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PayrollRun {
    /// Run id (the allocator's).
    pub id: PayrollRunId,
    /// Pay date: the date of the posted entry.
    pub date: NaiveDate,
    /// One line per employee.
    pub lines: Vec<PayrollLine>,
    /// The posted journal entry.
    pub entry_id: EntryId,
}

/// The chart accounts a payroll run posts to, resolved by code.
#[derive(Clone, Copy, Debug)]
pub struct PayrollAccounts {
    /// `6100` Salary & Wage Expense.
    pub salary_expense: AccountId,
    /// `6110` Payroll Tax Expense.
    pub payroll_tax_expense: AccountId,
    /// `2110` Federal Withholding Payable.
    pub federal_payable: AccountId,
    /// `2120` State Withholding Payable.
    pub state_payable: AccountId,
    /// `2130` SS Tax Payable.
    pub ss_payable: AccountId,
    /// `2140` Medicare Tax Payable.
    pub medicare_payable: AccountId,
    /// `2150` Other Deductions Payable.
    pub other_payable: AccountId,
}

/// Validate a payroll run's lines without mutating: non-empty, no negative
/// amounts, no negative net pay. Journaling stores run this before writing
/// anything, so a doomed run never leaves journal lines behind.
pub fn validate_payroll_lines(lines: &[PayrollLine]) -> Result<(), LedgerError> {
    // No wages at all is no payroll: refused before anything changes
    // (found by the spec-blind tests, 2026-10-13: an all-zero run passed
    // validation, then failed at post after creating accounts).
    if lines.is_empty() || lines.iter().all(|l| l.gross_pay.is_zero()) {
        return Err(LedgerError::EmptyPayroll);
    }
    for line in lines {
        for amount in [
            line.gross_pay,
            line.federal_withholding,
            line.state_withholding,
            line.ss_employee,
            line.medicare_employee,
            line.ss_employer,
            line.medicare_employer,
            line.other_deductions,
        ] {
            if amount < Decimal::ZERO {
                return Err(LedgerError::NegativePayrollAmount(amount));
            }
        }
        if line.net_pay() < Decimal::ZERO {
            return Err(LedgerError::NegativeNetPay(line.employee_name.clone()));
        }
    }
    Ok(())
}

/// The auto-posted payroll entry's lines, in posting order: Dr 6100 gross,
/// Dr 6110 the employer FICA share, Cr 2110–2150 the payables, Cr cash net
/// pay. Zero totals are skipped — the engine rejects zero-amount lines.
///
/// Shared by [`crate::Ledger::run_payroll`] and the journaling store, so the
/// journaled entry and the posted entry are built from the same math. The
/// plan always balances: debits are gross + employer FICA, credits are the
/// payables + net, and net is defined as gross minus employee withholdings.
pub fn payroll_entry_plan(
    lines: &[PayrollLine],
    accounts: PayrollAccounts,
    cash_account: AccountId,
    base: Currency,
) -> Vec<(AccountId, Side, Money)> {
    let sum = |f: fn(&PayrollLine) -> Decimal| lines.iter().map(f).sum::<Decimal>();
    let gross = sum(|l| l.gross_pay);
    let employer = sum(|l| l.ss_employer + l.medicare_employer);
    let federal = sum(|l| l.federal_withholding);
    let state = sum(|l| l.state_withholding);
    let ss = sum(|l| l.ss_employee + l.ss_employer);
    let medicare = sum(|l| l.medicare_employee + l.medicare_employer);
    let other = sum(|l| l.other_deductions);
    let net = sum(|l| l.net_pay());
    let mut plan = Vec::new();
    let mut push = |account: AccountId, side: Side, amount: Decimal| {
        if !amount.is_zero() {
            plan.push((account, side, Money::new(amount, base)));
        }
    };
    push(accounts.salary_expense, Side::Debit, gross);
    push(accounts.payroll_tax_expense, Side::Debit, employer);
    push(accounts.federal_payable, Side::Credit, federal);
    push(accounts.state_payable, Side::Credit, state);
    push(accounts.ss_payable, Side::Credit, ss);
    push(accounts.medicare_payable, Side::Credit, medicare);
    push(accounts.other_payable, Side::Credit, other);
    push(cash_account, Side::Credit, net);
    plan
}

/// The remittance entry's lines: Dr each payable with a non-zero balance,
/// Cr cash for the total. Balances are normal-side (positive = owed), so
/// the plan balances by construction.
pub fn payroll_remit_plan(
    payables: &[(AccountId, Decimal)],
    cash_account: AccountId,
    base: Currency,
) -> Vec<(AccountId, Side, Money)> {
    let mut plan = Vec::new();
    let mut total = Decimal::ZERO;
    for (account, balance) in payables {
        if balance.is_zero() {
            continue;
        }
        plan.push((*account, Side::Debit, Money::new(*balance, base)));
        total += *balance;
    }
    if !total.is_zero() {
        plan.push((cash_account, Side::Credit, Money::new(total, base)));
    }
    plan
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AccountType, Ledger};

    fn d(cents: i64) -> Decimal {
        Decimal::new(cents, 2)
    }

    fn line(name: &str) -> PayrollLine {
        PayrollLine {
            employee_name: name.to_string(),
            gross_pay: d(500_000),          // 5000.00
            federal_withholding: d(75_000), // 750.00
            state_withholding: d(25_000),   // 250.00
            ss_employee: d(31_000),         // 310.00
            medicare_employee: d(7_250),    // 72.50
            ss_employer: d(31_000),
            medicare_employer: d(7_250),
            other_deductions: d(10_000), // 100.00
        }
    }

    fn ledger_with_cash() -> (Ledger, AccountId) {
        let mut ledger = Ledger::new(Currency::USD);
        let cash = ledger
            .add_account("1000", "Cash", AccountType::Asset, None, false)
            .unwrap();
        (ledger, cash)
    }

    #[test]
    fn net_pay_math() {
        let l = line("Ada");
        // 5000 - 750 - 250 - 310 - 72.50 - 100 = 3517.50
        assert_eq!(l.net_pay(), d(351_750));
    }

    #[test]
    fn validation_rejects_empty_negative_and_overwithheld() {
        assert_eq!(validate_payroll_lines(&[]), Err(LedgerError::EmptyPayroll));
        let mut bad = line("Bob");
        bad.ss_employee = d(-1);
        assert_eq!(
            validate_payroll_lines(std::slice::from_ref(&bad)),
            Err(LedgerError::NegativePayrollAmount(d(-1)))
        );
        let mut over = line("Cat");
        over.other_deductions = d(99_999_999);
        assert!(over.net_pay() < Decimal::ZERO);
        assert_eq!(
            validate_payroll_lines(std::slice::from_ref(&over)),
            Err(LedgerError::NegativeNetPay("Cat".to_string()))
        );
        assert!(validate_payroll_lines(&[line("Ada")]).is_ok());
    }

    #[test]
    fn entry_plan_balances_and_skips_zeros() {
        let cash = ledger_with_cash().1;
        let accounts = PayrollAccounts {
            salary_expense: 101,
            payroll_tax_expense: 102,
            federal_payable: 103,
            state_payable: 104,
            ss_payable: 105,
            medicare_payable: 106,
            other_payable: 107,
        };
        // No state withholding and no other deductions: those credits vanish.
        let mut l = line("Ada");
        l.state_withholding = Decimal::ZERO;
        l.other_deductions = Decimal::ZERO;
        let plan = payroll_entry_plan(&[l], accounts, cash, Currency::USD);
        assert_eq!(plan.len(), 6); // gross, employer tax, fed, ss, medicare, cash
        let mut debits = Decimal::ZERO;
        let mut credits = Decimal::ZERO;
        for (_, side, amount) in &plan {
            match side {
                Side::Debit => debits += amount.amount,
                Side::Credit => credits += amount.amount,
            }
        }
        assert_eq!(debits, credits);
        assert!(debits > Decimal::ZERO);
        for (_, _, amount) in &plan {
            assert!(!amount.amount.is_zero());
        }
    }

    #[test]
    fn run_payroll_posts_balanced_entry_and_records_run() {
        let (mut ledger, cash) = ledger_with_cash();
        let date = NaiveDate::from_ymd_opt(2026, 10, 9).unwrap();
        let id = ledger
            .run_payroll(date, vec![line("Ada"), line("Bob")], cash)
            .unwrap();
        assert_eq!(id, 1);
        assert_eq!(ledger.next_payroll_run_id(), 2);
        let run = ledger.get_payroll_run(id).unwrap();
        assert_eq!(run.date, date);
        assert_eq!(run.lines.len(), 2);
        let entry = ledger.get_entry(run.entry_id).unwrap();
        assert_eq!(entry.status(), crate::EntryStatus::Posted);
        // Debits equal credits, exactly.
        let mut debits = Decimal::ZERO;
        let mut credits = Decimal::ZERO;
        for l in entry.lines() {
            match l.side {
                Side::Debit => debits += l.amount.amount,
                Side::Credit => credits += l.amount.amount,
            }
        }
        assert_eq!(debits, credits);
        // Gross x2 = 10000 hits Salary & Wage Expense (debit-normal).
        let salary = ledger.chart().find_by_code("6100").unwrap().id;
        assert_eq!(ledger.account_balance(salary).unwrap(), d(1_000_000));
        // Net x2 = 7035 leaves cash (debit-normal): -7035.
        assert_eq!(ledger.account_balance(cash).unwrap(), -d(703_500));
        // The seven payroll accounts were auto-created.
        for code in ["6100", "6110", "2110", "2120", "2130", "2140", "2150"] {
            assert!(
                ledger.chart().find_by_code(code).is_some(),
                "missing {code}"
            );
        }
        // Runs list in id order.
        let id2 = ledger.run_payroll(date, vec![line("Cat")], cash).unwrap();
        let ids: Vec<_> = ledger.list_payroll_runs().iter().map(|r| r.id).collect();
        assert_eq!(ids, vec![id, id2]);
    }

    #[test]
    fn run_payroll_rejects_closed_period() {
        let (mut ledger, cash) = ledger_with_cash();
        let date = NaiveDate::from_ymd_opt(2026, 9, 30).unwrap();
        let pid = ledger
            .create_period("Sep", NaiveDate::from_ymd_opt(2026, 9, 1).unwrap(), date)
            .unwrap();
        ledger.close_period(pid).unwrap();
        let err = ledger.run_payroll(date, vec![line("Ada")], cash);
        assert_eq!(err, Err(LedgerError::ClosedPeriod(pid, date.to_string())));
    }

    #[test]
    fn run_payroll_rejects_bad_inputs() {
        let (mut ledger, cash) = ledger_with_cash();
        let date = NaiveDate::from_ymd_opt(2026, 10, 9).unwrap();
        assert_eq!(
            ledger.run_payroll(date, vec![], cash),
            Err(LedgerError::EmptyPayroll)
        );
        let mut bad = line("Ada");
        bad.gross_pay = d(-100);
        assert_eq!(
            ledger.run_payroll(date, vec![bad], cash),
            Err(LedgerError::NegativePayrollAmount(d(-100)))
        );
        assert_eq!(
            ledger.run_payroll(date, vec![line("Ada")], 999),
            Err(LedgerError::UnknownAccount(999))
        );
    }

    #[test]
    fn remit_clears_payables() {
        let (mut ledger, cash) = ledger_with_cash();
        let date = NaiveDate::from_ymd_opt(2026, 10, 9).unwrap();
        ledger.run_payroll(date, vec![line("Ada")], cash).unwrap();
        // Withholdings owed: 750 + 250 + (310+310) + (72.50+72.50) + 100 = 1865.
        let fed = ledger.chart().find_by_code("2110").unwrap().id;
        assert_eq!(ledger.account_balance(fed).unwrap(), d(75_000));
        // Nothing owed before any payroll: fresh ledger remits nothing.
        let (mut fresh, fresh_cash) = ledger_with_cash();
        assert_eq!(
            fresh.remit_payroll_taxes(date, fresh_cash),
            Err(LedgerError::NothingToRemit)
        );
        // Remit: payables go to zero, cash drops by the total.
        let cash_before = ledger.account_balance(cash).unwrap();
        let entry = ledger.remit_payroll_taxes(date, cash).unwrap();
        let posted = ledger.get_entry(entry).unwrap();
        assert_eq!(posted.status(), crate::EntryStatus::Posted);
        for code in ["2110", "2120", "2130", "2140", "2150"] {
            let id = ledger.chart().find_by_code(code).unwrap().id;
            assert_eq!(
                ledger.account_balance(id).unwrap(),
                Decimal::ZERO,
                "payable {code} not cleared"
            );
        }
        assert_eq!(
            ledger.account_balance(cash).unwrap(),
            cash_before - d(186_500)
        );
        // Second remit finds nothing left.
        assert_eq!(
            ledger.remit_payroll_taxes(date, cash),
            Err(LedgerError::NothingToRemit)
        );
    }

    #[test]
    fn remit_rejects_closed_period() {
        let (mut ledger, cash) = ledger_with_cash();
        let date = NaiveDate::from_ymd_opt(2026, 9, 30).unwrap();
        ledger
            .run_payroll(
                NaiveDate::from_ymd_opt(2026, 9, 15).unwrap(),
                vec![line("Ada")],
                cash,
            )
            .unwrap();
        let pid = ledger
            .create_period("Sep", NaiveDate::from_ymd_opt(2026, 9, 1).unwrap(), date)
            .unwrap();
        ledger.close_period(pid).unwrap();
        assert_eq!(
            ledger.remit_payroll_taxes(date, cash),
            Err(LedgerError::ClosedPeriod(pid, date.to_string()))
        );
    }

    #[test]
    fn replay_payroll_run_checks_id() {
        let (mut ledger, _cash) = ledger_with_cash();
        let date = NaiveDate::from_ymd_opt(2026, 10, 9).unwrap();
        let entry = ledger.create_entry(date, "x");
        let err = ledger.replay_payroll_run(7, date, vec![line("Ada")], entry);
        assert_eq!(err, Err(LedgerError::ReplayIdMismatch(7, 1)));
        // The allocator did not move on failure.
        assert_eq!(ledger.next_payroll_run_id(), 1);
        ledger
            .replay_payroll_run(1, date, vec![line("Ada")], entry)
            .unwrap();
        assert_eq!(ledger.next_payroll_run_id(), 2);
        assert!(ledger.get_payroll_run(1).is_some());
        assert!(ledger.get_payroll_run(2).is_none());
    }
}
