//! Chart of accounts: the five account types and the account registry.
//!
//! Every account has exactly one of five types. The type determines the
//! account's *normal balance side*: assets and expenses are debit-normal,
//! liabilities, equity, and revenue are credit-normal. Reports are built by
//! grouping on type, so getting this right is the whole game.

use crate::{Currency, LedgerError};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Opaque account handle. Created by [`ChartOfAccounts`]; never forged.
pub type AccountId = u64;

/// The five kinds of accounts. This list is closed: all of double-entry
/// bookkeeping is expressed with these.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum AccountType {
    /// What the business owns or is owed: cash, receivables, equipment.
    Asset,
    /// What the business owes: payables, loans.
    Liability,
    /// The owners' claim: capital, retained earnings.
    Equity,
    /// Increases to equity from operations: sales, fees.
    Revenue,
    /// Decreases to equity from operations: rent, wages, supplies.
    Expense,
}

/// Debit or credit: the two sides of every journal line.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Side {
    /// Left side. Increases assets and expenses.
    Debit,
    /// Right side. Increases liabilities, equity, and revenue.
    Credit,
}

impl AccountType {
    /// The side that increases an account of this type — its normal balance.
    pub fn normal_side(&self) -> Side {
        match self {
            AccountType::Asset | AccountType::Expense => Side::Debit,
            AccountType::Liability | AccountType::Equity | AccountType::Revenue => Side::Credit,
        }
    }

    /// True for the three types that appear on the balance sheet.
    pub fn is_balance_sheet(&self) -> bool {
        matches!(
            self,
            AccountType::Asset | AccountType::Liability | AccountType::Equity
        )
    }
}

/// A single account in the chart of accounts.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Account {
    /// Opaque handle.
    pub id: AccountId,
    /// Human-meaningful code, e.g. `"1000"`. Unique within the chart.
    pub code: String,
    /// Display name, e.g. `"Cash"`.
    pub name: String,
    /// One of the five types. Determines reporting and normal side.
    pub account_type: AccountType,
    /// Hierarchy link, e.g. `"1010"` under `"1000"`. `None` for top-level.
    pub parent: Option<AccountId>,
    /// Header accounts organize the chart and take no postings.
    pub placeholder: bool,
    /// Day one: always the ledger's base currency (see R-5).
    pub currency: Currency,
}

/// The registry of accounts. Owns id allocation for accounts it holds.
#[derive(Clone, Debug, Default)]
pub struct ChartOfAccounts {
    accounts: HashMap<AccountId, Account>,
    by_code: HashMap<String, AccountId>,
}

impl ChartOfAccounts {
    /// Register an account. Codes must be unique and non-empty; a parent, if
    /// given, must already exist.
    pub fn add(
        &mut self,
        id: AccountId,
        code: &str,
        name: &str,
        account_type: AccountType,
        parent: Option<AccountId>,
        placeholder: bool,
        currency: Currency,
    ) -> Result<(), LedgerError> {
        if code.is_empty() {
            return Err(LedgerError::EmptyAccountCode);
        }
        if self.by_code.contains_key(code) {
            return Err(LedgerError::DuplicateAccountCode(code.to_string()));
        }
        if let Some(p) = parent {
            if !self.accounts.contains_key(&p) {
                return Err(LedgerError::UnknownParentAccount(p));
            }
        }
        self.by_code.insert(code.to_string(), id);
        self.accounts.insert(
            id,
            Account {
                id,
                code: code.to_string(),
                name: name.to_string(),
                account_type,
                parent,
                placeholder,
                currency,
            },
        );
        Ok(())
    }

    /// Look up an account by id.
    pub fn get(&self, id: AccountId) -> Option<&Account> {
        self.accounts.get(&id)
    }

    /// Look up an account by code.
    pub fn find_by_code(&self, code: &str) -> Option<&Account> {
        self.by_code.get(code).and_then(|id| self.accounts.get(id))
    }

    /// Iterate over all accounts (order unspecified; sort by code for display).
    pub fn all(&self) -> impl Iterator<Item = &Account> {
        self.accounts.values()
    }

    /// Number of accounts in the chart.
    pub fn len(&self) -> usize {
        self.accounts.len()
    }

    /// True when the chart holds no accounts.
    pub fn is_empty(&self) -> bool {
        self.accounts.is_empty()
    }
}
