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

// ---------------------------------------------------------------------------
// Cash flow statement (indirect method, R-FUTURE-3).
// ---------------------------------------------------------------------------

/// One labeled cash flow line. Positive is an inflow, negative an outflow.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CashFlowLine {
    /// Human label, e.g. `"Increase in Accounts Receivable"` or account code/name.
    pub label: String,
    /// Signed amount: positive = cash in, negative = cash out.
    pub amount: Decimal,
}

/// Parameters for [`cash_flow`].
#[derive(Clone, Debug)]
pub struct CashFlowParams {
    /// Period start, inclusive.
    pub start: chrono::NaiveDate,
    /// Period end, inclusive.
    pub end: chrono::NaiveDate,
    /// Accounts treated as cash. Their net change is the reconciling total.
    pub cash_accounts: Vec<crate::AccountId>,
    /// Liability accounts treated as financing (debt). Default empty: all
    /// liabilities are operating working capital, all equity is financing.
    pub financing_accounts: Vec<crate::AccountId>,
}

/// Cash flow statement, indirect method.
///
/// Starts from period net income, adds back non-cash depreciation, adjusts
/// for working-capital changes, then reports investing (fixed-asset
/// purchases at cost, disposal proceeds) and financing (equity changes,
/// designated debt accounts) flows. The three sections always sum to the
/// net change in the designated cash accounts.
///
/// Disposals are identified structurally: a posted entry that credits a
/// registered fixed-asset cost account and debits its linked accumulated
/// depreciation account. The gain/loss in that entry is removed from
/// operating and the proceeds are shown in investing.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CashFlowStatement {
    /// Period start, inclusive.
    pub start: chrono::NaiveDate,
    /// Period end, inclusive.
    pub end: chrono::NaiveDate,
    /// Period net income (revenue minus expenses).
    pub net_income: Decimal,
    /// Non-cash depreciation added back to net income.
    pub depreciation: Decimal,
    /// Working-capital adjustments (non-cash balance-sheet changes).
    pub working_capital: Vec<CashFlowLine>,
    /// Net cash from operating activities.
    pub cash_from_operations: Decimal,
    /// Investing flows: asset purchases and disposal proceeds.
    pub investing: Vec<CashFlowLine>,
    /// Net cash from investing activities.
    pub cash_from_investing: Decimal,
    /// Financing flows: equity changes and designated debt.
    pub financing: Vec<CashFlowLine>,
    /// Net cash from financing activities.
    pub cash_from_financing: Decimal,
    /// Net change in cash: the three sections summed.
    pub net_change_in_cash: Decimal,
    /// Cash balance at period start.
    pub cash_beginning: Decimal,
    /// Cash balance at period end.
    pub cash_ending: Decimal,
}

/// Build the indirect-method cash flow statement for a period.
///
/// Pure function of the ledger: net income plus non-cash depreciation plus
/// working-capital changes gives operating cash flow; fixed-asset purchases
/// and disposal proceeds give investing; equity and designated debt give
/// financing. The three sections sum to the net change in `cash_accounts`.
pub fn cash_flow(
    ledger: &Ledger,
    params: &CashFlowParams,
) -> Result<CashFlowStatement, LedgerError> {
    use std::collections::{HashMap, HashSet};

    if params.end < params.start {
        return Err(LedgerError::InvalidDateRange {
            start: params.start.to_string(),
            end: params.end.to_string(),
        });
    }

    // Fixed-asset account roles from the register.
    let mut cost_accounts: HashSet<crate::AccountId> = HashSet::new();
    let mut accum_accounts: HashSet<crate::AccountId> = HashSet::new();
    let mut depr_expense_accounts: HashSet<crate::AccountId> = HashSet::new();
    // cost_account -> accum_account, for disposal identification.
    let mut cost_to_accum: HashMap<crate::AccountId, crate::AccountId> = HashMap::new();
    for asset in ledger.registered_assets() {
        cost_accounts.insert(asset.cost_account);
        accum_accounts.insert(asset.accum_depr_account);
        depr_expense_accounts.insert(asset.expense_account);
        cost_to_accum.insert(asset.cost_account, asset.accum_depr_account);
    }

    let cash_set: HashSet<crate::AccountId> = params.cash_accounts.iter().copied().collect();
    let financing_set: HashSet<crate::AccountId> =
        params.financing_accounts.iter().copied().collect();

    // Movement helpers: debits minus credits over a date predicate.
    let movement_where = |account: crate::AccountId, pred: &dyn Fn(chrono::NaiveDate) -> bool| {
        let mut total = Decimal::ZERO;
        for entry in ledger.posted_entries() {
            if !pred(entry.date()) {
                continue;
            }
            for line in entry.lines() {
                if line.account == account {
                    match line.side {
                        crate::Side::Debit => total += line.amount.amount,
                        crate::Side::Credit => total -= line.amount.amount,
                    }
                }
            }
        }
        total
    };
    let begin_movement = |a: crate::AccountId| movement_where(a, &|d| d < params.start);
    let period_movement =
        |a: crate::AccountId| movement_where(a, &|d| d >= params.start && d <= params.end);

    // Normal-side balance from a movement.
    let normal = |account: crate::AccountId, movement: Decimal| -> Decimal {
        let acct = ledger.chart().get(account).expect("chart account exists");
        match acct.account_type.normal_side() {
            crate::Side::Debit => movement,
            crate::Side::Credit => -movement,
        }
    };

    // --- Net income and depreciation (period P&L) ---
    let mut net_income = Decimal::ZERO;
    let mut depreciation = Decimal::ZERO;
    for account in ledger.chart().all() {
        if account.placeholder {
            continue;
        }
        let pm = period_movement(account.id);
        if pm.is_zero() {
            continue;
        }
        match account.account_type {
            AccountType::Revenue => net_income += -pm, // revenue movement is credits
            AccountType::Expense => {
                net_income -= pm;
                if depr_expense_accounts.contains(&account.id) {
                    depreciation += pm;
                }
            }
            _ => {}
        }
    }

    // --- Classify balance-sheet accounts ---
    let mut working_capital: Vec<CashFlowLine> = Vec::new();
    let mut investing: Vec<CashFlowLine> = Vec::new();
    let mut financing: Vec<CashFlowLine> = Vec::new();

    // Account display helper.
    let display = |account: crate::AccountId| -> String {
        let a = ledger.chart().get(account).expect("chart account exists");
        format!("{} {}", a.code, a.name)
    };

    for account in ledger.chart().all() {
        if account.placeholder {
            continue;
        }
        let id = account.id;
        if cash_set.contains(&id) || cost_accounts.contains(&id) || accum_accounts.contains(&id) {
            continue; // cash reconciles; fixed assets are investing (gross, below)
        }
        let begin = normal(id, begin_movement(id));
        let end = normal(id, period_movement(id) + begin_movement(id));
        let delta = end - begin;
        if delta.is_zero() {
            continue;
        }
        // Cash effect of the balance change: for both assets and
        // liabilities, it is minus the movement (debits - credits).
        let movement_delta = period_movement(id);
        let cash_effect = -movement_delta;
        let (label, line) = match account.account_type {
            AccountType::Asset => {
                let dir = if delta > Decimal::ZERO {
                    "Increase in"
                } else {
                    "Decrease in"
                };
                (format!("{dir} {}", display(id)), cash_effect)
            }
            AccountType::Liability if financing_set.contains(&id) => {
                let dir = if delta > Decimal::ZERO {
                    "Proceeds from"
                } else {
                    "Repayment of"
                };
                (format!("{dir} {}", display(id)), cash_effect)
            }
            AccountType::Liability => {
                let dir = if delta > Decimal::ZERO {
                    "Increase in"
                } else {
                    "Decrease in"
                };
                (format!("{dir} {}", display(id)), cash_effect)
            }
            AccountType::Equity => {
                let dir = if delta > Decimal::ZERO {
                    "Owner contributions"
                } else {
                    "Owner distributions"
                };
                (format!("{dir} — {}", display(id)), cash_effect)
            }
            _ => continue, // revenue/expense handled via net income
        };
        let target = match account.account_type {
            AccountType::Equity => &mut financing,
            AccountType::Liability if financing_set.contains(&id) => &mut financing,
            _ => &mut working_capital,
        };
        target.push(CashFlowLine {
            label,
            amount: line,
        });
    }

    // --- Investing, gross: purchases and disposal proceeds ---
    // A disposal entry credits a cost account and debits its linked accum.
    let mut disposal_gain_loss = Decimal::ZERO; // +gain / -loss, to remove from operating
    for entry in ledger.posted_entries() {
        let d = entry.date();
        if d < params.start || d > params.end {
            continue;
        }
        // Is this a disposal? Credit to a cost account + debit to its accum.
        let mut disposed: Option<(crate::AccountId, Decimal, Decimal)> = None;
        for line in entry.lines() {
            if line.side == crate::Side::Credit
                && let Some(&accum) = cost_to_accum.get(&line.account)
            {
                // Confirm the linked accum account is debited in this entry.
                let accum_debit: Decimal = entry
                    .lines()
                    .iter()
                    .filter(|l| l.account == accum && l.side == crate::Side::Debit)
                    .map(|l| l.amount.amount)
                    .sum();
                if !accum_debit.is_zero() {
                    disposed = Some((line.account, line.amount.amount, accum_debit));
                    break;
                }
            }
        }
        if let Some((cost_acct, cost_amount, accum_debit)) = disposed {
            let accum_acct = cost_to_accum[&cost_acct];
            let mut proceeds = Decimal::ZERO;
            let mut proceeds_label = String::new();
            let mut gain = Decimal::ZERO;
            let mut loss = Decimal::ZERO;
            for line in entry.lines() {
                if line.account == cost_acct || line.account == accum_acct {
                    continue;
                }
                let acct = ledger
                    .chart()
                    .get(line.account)
                    .expect("chart account exists");
                match (acct.account_type, line.side) {
                    (AccountType::Asset, crate::Side::Debit) => {
                        proceeds += line.amount.amount;
                        proceeds_label = display(line.account);
                    }
                    (AccountType::Revenue, crate::Side::Credit) => gain += line.amount.amount,
                    (AccountType::Expense, crate::Side::Debit) => loss += line.amount.amount,
                    _ => {}
                }
            }
            let book_value = cost_amount - accum_debit;
            // Sanity: the entry balances, so proceeds + accum debit = cost + gain - loss.
            debug_assert!((proceeds + accum_debit - cost_amount - gain + loss).is_zero());
            let _ = book_value;
            disposal_gain_loss += gain - loss;
            investing.push(CashFlowLine {
                label: format!("Sale of fixed assets — {proceeds_label}"),
                amount: proceeds,
            });
            continue;
        }
        // Not a disposal: debits to cost accounts are purchases.
        for line in entry.lines() {
            if cost_accounts.contains(&line.account) && line.side == crate::Side::Debit {
                investing.push(CashFlowLine {
                    label: format!("Purchase of fixed assets — {}", display(line.account)),
                    amount: -line.amount.amount,
                });
            }
        }
    }

    // Remove disposal gains/losses from operating (they live in investing).
    if !disposal_gain_loss.is_zero() {
        working_capital.push(CashFlowLine {
            label: "Less: gain on sale of fixed assets (in investing)".to_string(),
            amount: -disposal_gain_loss,
        });
    }

    let cash_from_operations: Decimal =
        net_income + depreciation + working_capital.iter().map(|l| l.amount).sum::<Decimal>();
    let cash_from_investing: Decimal = investing.iter().map(|l| l.amount).sum();
    let cash_from_financing: Decimal = financing.iter().map(|l| l.amount).sum();

    // --- Cash reconciliation ---
    let mut cash_beginning = Decimal::ZERO;
    let mut cash_ending = Decimal::ZERO;
    for &id in &params.cash_accounts {
        cash_beginning += normal(id, begin_movement(id));
        cash_ending += normal(id, begin_movement(id) + period_movement(id));
    }
    let net_change_in_cash = cash_ending - cash_beginning;

    Ok(CashFlowStatement {
        start: params.start,
        end: params.end,
        net_income,
        depreciation,
        working_capital,
        cash_from_operations,
        investing,
        cash_from_investing,
        financing,
        cash_from_financing,
        net_change_in_cash,
        cash_beginning,
        cash_ending,
    })
}
