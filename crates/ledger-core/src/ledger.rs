//! The ledger: the command surface of the engine.
//!
//! A [`Ledger`] owns the chart of accounts and the append-only log of posted
//! journal entries. Every mutation is a method on `Ledger` — create, add
//! line, post, reverse — and every method validates before it applies. There
//! is no other way to change the books, which is what makes the ledger an
//! audit trail and what will make the future oracle harness trustworthy.

use crate::{
    AccountId, AccountType, Asset, AssetId, AssetStatus, ChartOfAccounts, Currency, EntryId,
    EntryStatus, JournalEntry, JournalLine, LedgerError, Money, NewAsset, Side, assets,
};
use chrono::{Months, NaiveDate};
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
    /// Fixed-asset register: master data, journaled like accounts.
    assets: HashMap<AssetId, Asset>,
    next_account_id: AccountId,
    next_entry_id: EntryId,
    next_asset_id: AssetId,
}

impl Ledger {
    /// Open a new, empty ledger denominated in `base_currency`.
    pub fn new(base_currency: Currency) -> Self {
        Self {
            base_currency,
            chart: ChartOfAccounts::default(),
            entries: HashMap::new(),
            order: Vec::new(),
            assets: HashMap::new(),
            next_account_id: 1,
            next_entry_id: 1,
            next_asset_id: 1,
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

    /// The id the next fixed asset registered will receive.
    ///
    /// See [`Self::next_account_id`].
    pub fn next_asset_id(&self) -> AssetId {
        self.next_asset_id
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

    // ------------------------------------------------------------------
    // Fixed assets
    // ------------------------------------------------------------------

    /// Pure pre-validation for [`Self::register_asset`]: every check that
    /// does not mutate, so journaling stores can validate before journaling.
    pub fn validate_new_asset(&self, new: &NewAsset) -> Result<(), LedgerError> {
        if new.description.trim().is_empty() {
            return Err(LedgerError::EmptyAssetDescription);
        }
        if new.cost.amount <= Decimal::ZERO {
            return Err(LedgerError::InvalidAssetCost(new.cost.amount));
        }
        if new.cost.currency != self.base_currency {
            return Err(LedgerError::CurrencyMismatch {
                expected: self.base_currency.code().to_string(),
                found: new.cost.currency.code().to_string(),
            });
        }
        if new.salvage_value.amount < Decimal::ZERO || new.salvage_value.amount >= new.cost.amount {
            return Err(LedgerError::InvalidSalvageValue(new.salvage_value.amount));
        }
        if new.salvage_value.currency != self.base_currency {
            return Err(LedgerError::CurrencyMismatch {
                expected: self.base_currency.code().to_string(),
                found: new.salvage_value.currency.code().to_string(),
            });
        }
        if new.useful_life_months < 1 {
            return Err(LedgerError::InvalidUsefulLife(new.useful_life_months));
        }
        // The schedule walks every month of the asset's life; a purchase
        // date near the end of the calendar would overflow date arithmetic
        // (and hang or panic the schedule). Reject it up front.
        if new
            .purchase_date
            .checked_add_months(Months::new(new.useful_life_months))
            .is_none()
        {
            return Err(LedgerError::AssetDateOverflow);
        }
        assets::validate_asset_accounts(
            &self.chart,
            new.cost_account,
            new.accum_depr_account,
            new.expense_account,
        )
    }

    /// Register a fixed asset. Validates the economics (positive cost,
    /// salvage below cost, at least one month of life, base currency) and
    /// the account links (asset/asset/expense types, three distinct
    /// accounts).
    pub fn register_asset(&mut self, new: NewAsset) -> Result<AssetId, LedgerError> {
        self.validate_new_asset(&new)?;
        let id = self.next_asset_id;
        self.next_asset_id += 1;
        self.assets.insert(
            id,
            Asset {
                id,
                description: new.description,
                purchase_date: new.purchase_date,
                cost: new.cost,
                salvage_value: new.salvage_value,
                useful_life_months: new.useful_life_months,
                method: crate::DepreciationMethod::StraightLine,
                cost_account: new.cost_account,
                accum_depr_account: new.accum_depr_account,
                expense_account: new.expense_account,
                status: AssetStatus::Active,
            },
        );
        Ok(id)
    }

    /// Look up a fixed asset.
    pub fn get_asset(&self, id: AssetId) -> Option<&Asset> {
        self.assets.get(&id)
    }

    /// All registered assets, in registration order.
    pub fn registered_assets(&self) -> impl Iterator<Item = &Asset> + '_ {
        let mut ids: Vec<AssetId> = self.assets.keys().copied().collect();
        ids.sort_unstable();
        ids.into_iter().filter_map(|id| self.assets.get(&id))
    }

    /// Mark an asset disposed. The disposal *entry* is posted through the
    /// normal machinery by the caller; this records the lifecycle change.
    pub fn dispose_asset(
        &mut self,
        id: AssetId,
        date: NaiveDate,
        proceeds: Money,
    ) -> Result<(), LedgerError> {
        if proceeds.currency != self.base_currency {
            return Err(LedgerError::CurrencyMismatch {
                expected: self.base_currency.code().to_string(),
                found: proceeds.currency.code().to_string(),
            });
        }
        let asset = self.assets.get(&id).ok_or(LedgerError::UnknownAsset(id))?;
        // Pure math validates activeness, proceeds sign, and date order.
        crate::disposal_amounts(asset, date, proceeds)?;
        self.assets.get_mut(&id).expect("asset exists").status =
            AssetStatus::Disposed { date, proceeds };
        Ok(())
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
