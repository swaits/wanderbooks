//! The ledger: the command surface of the engine.
//!
//! A [`Ledger`] owns the chart of accounts and the append-only log of posted
//! journal entries. Every mutation is a method on `Ledger` — create, add
//! line, post, reverse — and every method validates before it applies. There
//! is no other way to change the books, which is what makes the ledger an
//! audit trail and what will make the future oracle harness trustworthy.

use crate::{
    AccountId, AccountType, ChartOfAccounts, Currency, EntryId, EntryStatus, JournalEntry,
    JournalLine, LedgerError, Money, Side,
};
use chrono::NaiveDate;
use rust_decimal::Decimal;
use std::collections::HashMap;

/// The books. Single-currency (see R-5); owns id allocation.
#[derive(Debug)]
pub struct Ledger {
    base_currency: Currency,
    chart: ChartOfAccounts,
    entries: HashMap<EntryId, JournalEntry>,
    /// Entry ids in posting order: the append-only log.
    order: Vec<EntryId>,
    next_account_id: AccountId,
    next_entry_id: EntryId,
}

impl Ledger {
    /// Open a new, empty ledger denominated in `base_currency`.
    pub fn new(base_currency: Currency) -> Self {
        Self {
            base_currency,
            chart: ChartOfAccounts::default(),
            entries: HashMap::new(),
            order: Vec::new(),
            next_account_id: 1,
            next_entry_id: 1,
        }
    }

    /// The id the next account added will receive.
    ///
    /// Exposed for journaling stores that record the id before mutating.
    /// The value is only stable if no other mutation intervenes.
    pub fn next_account_id(&self) -> AccountId {
        self.next_account_id
    }

    /// The id the next journal entry created will receive.
    ///
    /// See [`Self::next_account_id`].
    pub fn next_entry_id(&self) -> EntryId {
        self.next_entry_id
    }

    /// The currency every account and line in this ledger uses.
    pub fn base_currency(&self) -> Currency {
        self.base_currency
    }

    /// Read-only view of the chart of accounts.
    pub fn chart(&self) -> &ChartOfAccounts {
        &self.chart
    }

    /// Add an account to the chart. Returns its id.
    pub fn add_account(
        &mut self,
        code: &str,
        name: &str,
        account_type: AccountType,
        parent: Option<AccountId>,
        placeholder: bool,
    ) -> Result<AccountId, LedgerError> {
        let id = self.next_account_id;
        self.next_account_id += 1;
        self.chart.add(
            id,
            code,
            name,
            account_type,
            parent,
            placeholder,
            self.base_currency,
        )?;
        Ok(id)
    }

    /// Start a draft journal entry. Returns its id.
    pub fn create_entry(&mut self, date: NaiveDate, description: &str) -> EntryId {
        let id = self.next_entry_id;
        self.next_entry_id += 1;
        self.entries
            .insert(id, JournalEntry::draft(id, date, description.to_string()));
        id
    }

    fn draft_mut(&mut self, entry: EntryId) -> Result<&mut JournalEntry, LedgerError> {
        let e = self
            .entries
            .get_mut(&entry)
            .ok_or(LedgerError::UnknownEntry(entry))?;
        if e.status() != EntryStatus::Draft {
            return Err(LedgerError::PostedEntryImmutable);
        }
        Ok(e)
    }

    /// Add a line to a draft entry. The amount must be in the ledger's base
    /// currency; drafts reject anything else rather than converting.
    pub fn add_line(
        &mut self,
        entry: EntryId,
        account: AccountId,
        side: Side,
        amount: Money,
    ) -> Result<(), LedgerError> {
        if amount.currency != self.base_currency {
            return Err(LedgerError::CurrencyMismatch {
                expected: self.base_currency.code().to_string(),
                found: amount.currency.code().to_string(),
            });
        }
        let e = self.draft_mut(entry)?;
        e.push_line(JournalLine {
            account,
            side,
            amount,
        });
        Ok(())
    }

    /// Validate and post a draft entry. One-way: a posted entry is immutable.
    pub fn post_entry(&mut self, entry: EntryId) -> Result<(), LedgerError> {
        let status = self
            .entries
            .get(&entry)
            .ok_or(LedgerError::UnknownEntry(entry))?
            .status();
        if status == EntryStatus::Posted {
            return Err(LedgerError::EntryAlreadyPosted(entry));
        }
        // Clone so validation borrows end before the mutable mark.
        let snapshot = self
            .entries
            .get(&entry)
            .ok_or(LedgerError::UnknownEntry(entry))?
            .clone();
        snapshot.validate(&self.chart)?;
        self.entries
            .get_mut(&entry)
            .ok_or(LedgerError::UnknownEntry(entry))?
            .mark_posted();
        self.order.push(entry);
        Ok(())
    }

    /// Look up an entry by id (draft or posted).
    pub fn get_entry(&self, entry: EntryId) -> Option<&JournalEntry> {
        self.entries.get(&entry)
    }

    /// Post a reversing entry: same lines with sides flipped, linked back to
    /// the original via `reversal_of`. The books never forget; they correct.
    pub fn reverse_entry(
        &mut self,
        entry: EntryId,
        date: NaiveDate,
        description: &str,
    ) -> Result<EntryId, LedgerError> {
        let original = self
            .entries
            .get(&entry)
            .ok_or(LedgerError::UnknownEntry(entry))?
            .clone();
        if original.status() != EntryStatus::Posted {
            return Err(LedgerError::EntryNotPosted(entry));
        }
        let id = self.create_entry(date, description);
        for line in original.lines() {
            let flipped = match line.side {
                Side::Debit => Side::Credit,
                Side::Credit => Side::Debit,
            };
            self.add_line(id, line.account, flipped, line.amount)?;
        }
        self.entries
            .get_mut(&id)
            .ok_or(LedgerError::UnknownEntry(id))?
            .mark_reversal_of(entry);
        self.post_entry(id)?;
        Ok(id)
    }

    /// Posted entries in posting order: the append-only log.
    pub fn posted_entries(&self) -> impl Iterator<Item = &JournalEntry> + '_ {
        self.order.iter().filter_map(|id| self.entries.get(id))
    }

    /// Signed net movement of an account across all posted entries:
    /// debits minus credits. Positive means net debit movement.
    pub fn account_movement(&self, account: AccountId) -> Decimal {
        let mut total = Decimal::ZERO;
        for entry in self.posted_entries() {
            for line in entry.lines() {
                if line.account == account {
                    match line.side {
                        Side::Debit => total += line.amount.amount,
                        Side::Credit => total -= line.amount.amount,
                    }
                }
            }
        }
        total
    }

    /// Balance expressed in the account's normal side: positive means a
    /// normal balance, negative means a contra balance.
    pub fn account_balance(&self, account: AccountId) -> Result<Decimal, LedgerError> {
        let account = self
            .chart
            .get(account)
            .ok_or(LedgerError::UnknownAccount(account))?;
        let movement = self.account_movement(account.id);
        let balance = match account.account_type.normal_side() {
            Side::Debit => movement,
            Side::Credit => -movement,
        };
        // Never leak negative zero into reports: it is a representation
        // artifact of negating an exact zero, and money must never print "-0".
        Ok(if balance.is_zero() {
            Decimal::ZERO
        } else {
            balance
        })
    }
}
