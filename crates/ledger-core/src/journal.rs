//! Journal entries: the atomic unit of double-entry bookkeeping.
//!
//! An entry is a set of lines that must balance exactly: total debits equal
//! total credits. Entries are born as drafts, validated, then posted. Posting
//! is one-way; a posted entry can only be corrected by a reversing entry.

use crate::kernel::{self, LineError};
use crate::{AccountId, ChartOfAccounts, ClassId, Currency, LedgerError, Money, Side};
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

/// Where a draft entry stands (plan P4, `drafts_v2`): still being built,
/// or finished and waiting on the document that supports it (rules.md
/// B-43). Meaningful while the entry is a draft; a posted entry keeps
/// the state it had and nothing reads it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum DraftState {
    /// Lines are still being added. What every draft starts as.
    #[default]
    Building,
    /// The lines are final and the entry waits for its receipt or other
    /// evidence; `note` says what is awaited (empty: nothing said).
    AwaitingEvidence {
        /// What the draft waits on, in the words of whoever set the state.
        note: String,
    },
}

impl DraftState {
    /// Its key as the journal, the tools and the viewer name it:
    /// `building` or `awaiting_evidence`.
    pub fn key(&self) -> &'static str {
        match self {
            DraftState::Building => "building",
            DraftState::AwaitingEvidence { .. } => "awaiting_evidence",
        }
    }

    /// [`Self::key`] the other way, with the note an awaiting state
    /// carries; `None` for a state this version doesn't know (a newer
    /// version's).
    pub fn parse(key: &str, note: &str) -> Option<DraftState> {
        match key {
            "building" => Some(DraftState::Building),
            "awaiting_evidence" => Some(DraftState::AwaitingEvidence {
                note: note.to_string(),
            }),
            _ => None,
        }
    }

    /// The note an awaiting state carries; empty for building.
    pub fn note(&self) -> &str {
        match self {
            DraftState::Building => "",
            DraftState::AwaitingEvidence { note } => note,
        }
    }
}

/// One debit or credit line of a journal entry.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct JournalLine {
    /// The account this line posts to.
    pub account: AccountId,
    /// Which side of the entry this line is on.
    pub side: Side,
    /// The exact amount, in the ledger's base currency. Never zero (rejected
    /// at validation).
    pub amount: Money,
    /// For foreign-currency lines: the original currency. `None` for base-
    /// currency lines. When set, `amount` is the base-currency equivalent
    /// converted at the line's rate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub foreign_currency: Option<Currency>,
    /// For foreign-currency lines: the original foreign amount. `None` for
    /// base-currency lines.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub foreign_amount: Option<Decimal>,
    /// Divisional tracking: which class (department, location, job) this
    /// line belongs to. `None` means unclassified. Set on drafts only via
    /// [`crate::Ledger::set_line_class`]; posted lines are immutable, so
    /// their classes are frozen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class_id: Option<ClassId>,
}

/// A balanced set of journal lines with a date and a description.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct JournalEntry {
    id: EntryId,
    date: NaiveDate,
    description: String,
    lines: Vec<JournalLine>,
    status: EntryStatus,
    reversal_of: Option<EntryId>,
    /// Where the draft stands (`entry.draft_state_set`); meaningful while
    /// `status` is `Draft`.
    #[serde(default)]
    draft_state: DraftState,
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
            draft_state: DraftState::Building,
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

    /// Where a draft stands (plan P4): building, or awaiting evidence.
    /// Meaningful while the entry is a draft.
    pub fn draft_state(&self) -> &DraftState {
        &self.draft_state
    }

    /// If this entry reverses another, the reversed entry's id.
    pub fn reversal_of(&self) -> Option<EntryId> {
        self.reversal_of
    }

    /// Append a line. Only while drafted; only via [`crate::Ledger`].
    pub(crate) fn push_line(&mut self, line: JournalLine) {
        self.lines.push(line);
    }

    /// Tag a line with a class (or clear it). Only while drafted; only via
    /// [`crate::Ledger::set_line_class`].
    pub(crate) fn set_line_class(&mut self, index: usize, class_id: Option<ClassId>) {
        self.lines[index].class_id = class_id;
    }

    /// Set the draft's state. Only while drafted; only via
    /// [`crate::Ledger::set_draft_state`].
    pub(crate) fn set_draft_state(&mut self, state: DraftState) {
        self.draft_state = state;
    }

    /// Mark posted. Only via [`crate::Ledger`], after [`Self::validate`].
    pub(crate) fn mark_posted(&mut self) {
        self.status = EntryStatus::Posted;
        // A posted entry waits on nothing: its state is the canonical one.
        self.draft_state = DraftState::Building;
    }

    /// Link this entry as the reversal of another. Only via [`crate::Ledger`].
    pub(crate) fn mark_reversal_of(&mut self, original: EntryId) {
        self.reversal_of = Some(original);
    }

    /// Validate the entry against the chart of accounts. This is the heart of
    /// R-1: at least two lines, every amount positive and within line range,
    /// no placeholder or unknown accounts, one currency throughout, and
    /// debits exactly equal to credits. The arithmetic is
    /// [`kernel::check_lines`].
    pub fn validate(&self, chart: &ChartOfAccounts) -> Result<(), LedgerError> {
        let mut currency: Option<Currency> = None;
        for line in &self.lines {
            let account = chart
                .get(line.account)
                .ok_or(LedgerError::UnknownAccount(line.account))?;
            if account.placeholder {
                return Err(LedgerError::PlaceholderAccount(account.code.clone()));
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
        }
        let sides: Vec<(Side, Decimal)> = self
            .lines
            .iter()
            .map(|l| (l.side, l.amount.amount))
            .collect();
        kernel::check_lines(&sides).map_err(|e| match e {
            LineError::TooFewLines => LedgerError::TooFewLines,
            LineError::ZeroAmount => LedgerError::ZeroAmountLine,
            LineError::NegativeAmount => LedgerError::NegativeAmountLine,
            LineError::OutOfRange => LedgerError::AmountOutOfRange,
            LineError::Unbalanced { debits, credits } => {
                LedgerError::UnbalancedEntry { debits, credits }
            }
        })
    }

    /// Net movement per account within this entry: debits minus credits.
    /// Sums to zero across accounts for any valid entry.
    pub fn movements(&self) -> HashMap<AccountId, Decimal> {
        kernel::movements(
            self.lines
                .iter()
                .map(|l| (l.account, l.side, l.amount.amount)),
        )
        .expect("posted lines are in range, so their sums are")
        .into_iter()
        .collect()
    }
}
