//! Journal entries: the atomic unit of double-entry bookkeeping.
//!
//! An entry is a set of lines that must balance exactly: total debits equal
//! total credits. Entries are born as drafts, validated, then posted. Posting
//! is one-way; a posted entry can only be corrected by a reversing entry.

use crate::{AccountId, ChartOfAccounts, Currency, LedgerError, Money, Side};
use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Opaque journal entry handle. Created by [`crate::Ledger`]; never forged.
pub type EntryId = u64;

/// Lifecycle of a journal entry. `Draft -> Posted` is one-way.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum EntryStatus {
    /// Being built. Lines may be added; nothing has hit the ledger.
    Draft,
    /// Validated and applied. Immutable from here on.
    Posted,
}

/// One debit or credit line of a journal entry.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JournalLine {
    /// The account this line posts to.
    pub account: AccountId,
    /// Which side of the entry this line is on.
    pub side: Side,
    /// The exact amount. Never zero (rejected at validation).
    pub amount: Money,
}

/// A balanced set of journal lines with a date and a description.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JournalEntry {
    id: EntryId,
    date: NaiveDate,
    description: String,
    lines: Vec<JournalLine>,
    status: EntryStatus,
    reversal_of: Option<EntryId>,
}

impl JournalEntry {
    /// Create a draft entry. Only [`crate::Ledger`] does this; the id comes
    /// from the ledger's allocator so ids are unique per ledger.
    pub(crate) fn draft(id: EntryId, date: NaiveDate, description: String) -> Self {
        Self {
            id,
            date,
            description,
            lines: Vec::new(),
            status: EntryStatus::Draft,
            reversal_of: None,
        }
    }

    /// The entry's id.
    pub fn id(&self) -> EntryId {
        self.id
    }

    /// The entry's accounting date.
    pub fn date(&self) -> NaiveDate {
        self.date
    }

    /// Human description / memo.
    pub fn description(&self) -> &str {
        &self.description
    }

    /// The entry's lines, in the order they were added.
    pub fn lines(&self) -> &[JournalLine] {
        &self.lines
    }

    /// Draft or posted.
    pub fn status(&self) -> EntryStatus {
        self.status
    }

    /// If this entry reverses another, the reversed entry's id.
    pub fn reversal_of(&self) -> Option<EntryId> {
        self.reversal_of
    }

    /// Append a line. Only while drafted; only via [`crate::Ledger`].
    pub(crate) fn push_line(&mut self, line: JournalLine) {
        self.lines.push(line);
    }

    /// Mark posted. Only via [`crate::Ledger`], after [`Self::validate`].
    pub(crate) fn mark_posted(&mut self) {
        self.status = EntryStatus::Posted;
    }

    /// Link this entry as the reversal of another. Only via [`crate::Ledger`].
    pub(crate) fn mark_reversal_of(&mut self, original: EntryId) {
        self.reversal_of = Some(original);
    }

    /// Validate the entry against the chart of accounts. This is the heart of
    /// R-1: at least two lines, no zero amounts, no placeholder or unknown
    /// accounts, one currency throughout, and debits exactly equal to credits.
    pub fn validate(&self, chart: &ChartOfAccounts) -> Result<(), LedgerError> {
        if self.lines.len() < 2 {
            return Err(LedgerError::TooFewLines);
        }
        let mut debits = Decimal::ZERO;
        let mut credits = Decimal::ZERO;
        let mut currency: Option<Currency> = None;
        for line in &self.lines {
            let account = chart
                .get(line.account)
                .ok_or(LedgerError::UnknownAccount(line.account))?;
            if account.placeholder {
                return Err(LedgerError::PlaceholderAccount(account.code.clone()));
            }
            if line.amount.is_zero() {
                return Err(LedgerError::ZeroAmountLine);
            }
            match currency {
                None => currency = Some(line.amount.currency),
                Some(c) if c != line.amount.currency => {
                    return Err(LedgerError::CurrencyMismatch {
                        expected: c.code().to_string(),
                        found: line.amount.currency.code().to_string(),
                    });
                }
                Some(_) => {}
            }
            if account.currency != line.amount.currency {
                return Err(LedgerError::CurrencyMismatch {
                    expected: account.currency.code().to_string(),
                    found: line.amount.currency.code().to_string(),
                });
            }
            match line.side {
                Side::Debit => debits += line.amount.amount,
                Side::Credit => credits += line.amount.amount,
            }
        }
        if debits != credits {
            return Err(LedgerError::UnbalancedEntry { debits, credits });
        }
        Ok(())
    }

    /// Net movement per account within this entry: debits minus credits.
    /// Sums to zero across accounts for any valid entry.
    pub fn movements(&self) -> HashMap<AccountId, Decimal> {
        let mut out = HashMap::new();
        for line in &self.lines {
            let delta = match line.side {
                Side::Debit => line.amount.amount,
                Side::Credit => -line.amount.amount,
            };
            *out.entry(line.account).or_insert(Decimal::ZERO) += delta;
        }
        out
    }
}
