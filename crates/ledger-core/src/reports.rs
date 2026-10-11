//! Reports: pure functions of a [`Ledger`](crate::Ledger).
//!
//! Reports never mutate and never guess. They are what the semantic oracle
//! (`docs/oracle.md`) diffs: the same fixture books through two engines must
//! produce byte-identical [`snapshot`] output.

use crate::{AccountType, Ledger, LedgerError};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// One line of a trial balance.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TrialBalanceLine {
    /// Account code, e.g. `"1000"`.
    pub code: String,
    /// Account name, e.g. `"Cash"`.
    pub name: String,
    /// Debit balance, or zero.
    pub debit: Decimal,
    /// Credit balance, or zero.
    pub credit: Decimal,
}

/// The trial balance: every account with a non-zero balance, debit and credit
/// columns, and column totals. A correct book always has equal totals.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TrialBalance {
    /// Lines sorted by account code. Deterministic order matters for goldens.
    pub lines: Vec<TrialBalanceLine>,
    /// Sum of the debit column.
    pub total_debits: Decimal,
    /// Sum of the credit column.
    pub total_credits: Decimal,
}

impl TrialBalance {
    /// The fundamental check: debits equal credits, exactly.
    pub fn balanced(&self) -> bool {
        self.total_debits == self.total_credits
    }
}

/// Build the trial balance from all posted entries.
pub fn trial_balance(ledger: &Ledger) -> TrialBalance {
    let mut accounts: Vec<_> = ledger.chart().all().collect();
    accounts.sort_by(|a, b| a.code.cmp(&b.code));

    let mut lines = Vec::new();
    let mut total_debits = Decimal::ZERO;
    let mut total_credits = Decimal::ZERO;
    for account in accounts {
        if account.placeholder {
            continue;
        }
        let movement = ledger.account_movement(account.id);
        if movement.is_zero() {
            continue;
        }
        let (debit, credit) = if movement > Decimal::ZERO {
            (movement, Decimal::ZERO)
        } else {
            (Decimal::ZERO, -movement)
        };
        total_debits += debit;
        total_credits += credit;
        lines.push(TrialBalanceLine {
            code: account.code.clone(),
            name: account.name.clone(),
            debit,
            credit,
        });
    }
    TrialBalance {
        lines,
        total_debits,
        total_credits,
    }
}

/// Profit and loss: revenue and expenses for the life of the ledger.
/// (Period filtering is R-FUTURE work; see `docs/requirements.md` R-7.)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProfitAndLoss {
    /// Total revenue (credit-normal balances, reported positive).
    pub revenue: Decimal,
    /// Total expenses (debit-normal balances, reported positive).
    pub expenses: Decimal,
}

impl ProfitAndLoss {
    /// Revenue minus expenses. Positive is profit.
    pub fn net_income(&self) -> Decimal {
        self.revenue - self.expenses
    }
}

/// Build the profit and loss statement from all posted entries.
pub fn profit_and_loss(ledger: &Ledger) -> Result<ProfitAndLoss, LedgerError> {
    let mut revenue = Decimal::ZERO;
    let mut expenses = Decimal::ZERO;
    for account in ledger.chart().all() {
        if account.placeholder {
            continue;
        }
        let balance = ledger.account_balance(account.id)?;
        match account.account_type {
            AccountType::Revenue => revenue += balance,
            AccountType::Expense => expenses += balance,
            _ => {}
        }
    }
    Ok(ProfitAndLoss { revenue, expenses })
}

/// Balance sheet: assets, liabilities, and equity.
///
/// Because day-one ledgers have no closing entries yet (R-7), current
/// earnings (net income to date) are folded into equity so that the
/// accounting equation holds on a live book: assets = liabilities + equity.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BalanceSheet {
    /// Total assets (debit-normal balances, reported positive).
    pub assets: Decimal,
    /// Total liabilities (credit-normal balances, reported positive).
    pub liabilities: Decimal,
    /// Total equity *including* current earnings.
    pub equity: Decimal,
    /// Net income to date, folded into equity above.
    pub current_earnings: Decimal,
}

impl BalanceSheet {
    /// The accounting equation: assets == liabilities + equity, exactly.
    pub fn balanced(&self) -> bool {
        self.assets == self.liabilities + self.equity
    }
}

/// Build the balance sheet from all posted entries.
pub fn balance_sheet(ledger: &Ledger) -> Result<BalanceSheet, LedgerError> {
    let mut assets = Decimal::ZERO;
    let mut liabilities = Decimal::ZERO;
    let mut equity = Decimal::ZERO;
    for account in ledger.chart().all() {
        if account.placeholder {
            continue;
        }
        let balance = ledger.account_balance(account.id)?;
        match account.account_type {
            AccountType::Asset => assets += balance,
            AccountType::Liability => liabilities += balance,
            AccountType::Equity => equity += balance,
            AccountType::Revenue | AccountType::Expense => {}
        }
    }
    let current_earnings = profit_and_loss(ledger)?.net_income();
    equity += current_earnings;
    Ok(BalanceSheet {
        assets,
        liabilities,
        equity,
        current_earnings,
    })
}

/// Render the three core reports as canonical JSON for oracle diffing.
///
/// Amounts are decimal strings (never floats, never locale-formatted), lines
/// are sorted by account code, and object keys are fixed. Two engines fed the
/// same fixture books must produce byte-identical output — that is the whole
/// point (see `docs/oracle.md`).
pub fn snapshot(ledger: &Ledger) -> Result<serde_json::Value, LedgerError> {
    let tb = trial_balance(ledger);
    let pl = profit_and_loss(ledger)?;
    let bs = balance_sheet(ledger)?;
    Ok(serde_json::json!({
        "trial_balance": {
            "lines": tb.lines.iter().map(|l| serde_json::json!({
                "code": l.code,
                "name": l.name,
                "debit": l.debit.to_string(),
                "credit": l.credit.to_string(),
            })).collect::<Vec<_>>(),
            "total_debits": tb.total_debits.to_string(),
            "total_credits": tb.total_credits.to_string(),
        },
        "profit_and_loss": {
            "revenue": pl.revenue.to_string(),
            "expenses": pl.expenses.to_string(),
            "net_income": pl.net_income().to_string(),
        },
        "balance_sheet": {
            "assets": bs.assets.to_string(),
            "liabilities": bs.liabilities.to_string(),
            "equity": bs.equity.to_string(),
            "current_earnings": bs.current_earnings.to_string(),
            "balanced": bs.balanced(),
        },
    }))
}
