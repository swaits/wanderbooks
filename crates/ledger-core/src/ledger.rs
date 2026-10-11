//! The ledger: the command surface of the engine.
//!
//! A [`Ledger`] owns the chart of accounts and the append-only log of posted
//! journal entries. Every mutation is a method on `Ledger` — create, add
//! line, post, reverse — and every method validates before it applies. There
//! is no other way to change the books, which is what makes the ledger an
//! audit trail and what will make the future oracle harness trustworthy.

use crate::{
    AccountId, AccountType, AgingRow, Asset, AssetId, AssetStatus, Bill, BillId, ChartOfAccounts,
    Currency, Customer, CustomerId, DocumentLine, DocumentStatus, EntryId, EntryStatus, Invoice,
    InvoiceId, JournalEntry, JournalLine, LedgerError, Money, NewAsset, Period, PeriodId,
    PeriodStatus, Side, Vendor, VendorId, arap, assets, periods,
};
use chrono::{Months, NaiveDate};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A posted line that has not cleared, for the reconciliation report.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UnclearedLine {
    /// Entry id.
    pub entry_id: EntryId,
    /// Index within the entry.
    pub line_index: usize,
    /// Entry date.
    pub date: NaiveDate,
    /// Entry description.
    pub description: String,
    /// Line side.
    pub side: Side,
    /// Line amount (base currency).
    pub amount: Decimal,
}

/// The bank reconciliation report.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReconciliationReport {
    /// Account id reconciled.
    pub account: AccountId,
    /// Sum of all posted lines on the account (book balance).
    pub book_balance: Decimal,
    /// Sum of cleared lines dated on or before the statement date.
    pub cleared_balance: Decimal,
    /// The bank statement's balance.
    pub statement_balance: Decimal,
    /// cleared_balance - statement_balance. Zero means reconciled.
    pub difference: Decimal,
    /// Posted, uncleared lines dated on or before the statement date.
    pub uncleared_lines: Vec<UnclearedLine>,
}

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
    /// Accounting periods.
    periods: HashMap<PeriodId, Period>,
    /// Customers for A/R.
    customers: HashMap<CustomerId, Customer>,
    /// Vendors for A/P.
    vendors: HashMap<VendorId, Vendor>,
    /// Sales invoices.
    invoices: HashMap<InvoiceId, Invoice>,
    /// Vendor bills.
    bills: HashMap<BillId, Bill>,
    /// Cleared flags for bank reconciliation: (entry_id, line_index) -> cleared.
    /// Posted entries are immutable; this side table holds the workflow flag.
    cleared: HashMap<(EntryId, usize), bool>,
    /// Historical FX rates for reference: (currency, date) -> rate (foreign -> base).
    fx_rates: HashMap<(Currency, NaiveDate), Decimal>,
    next_account_id: AccountId,
    next_entry_id: EntryId,
    next_asset_id: AssetId,
    next_period_id: PeriodId,
    next_customer_id: CustomerId,
    next_vendor_id: VendorId,
    next_invoice_id: InvoiceId,
    next_bill_id: BillId,
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
            periods: HashMap::new(),
            customers: HashMap::new(),
            vendors: HashMap::new(),
            invoices: HashMap::new(),
            bills: HashMap::new(),
            cleared: HashMap::new(),
            fx_rates: HashMap::new(),
            next_account_id: 1,
            next_entry_id: 1,
            next_asset_id: 1,
            next_period_id: 1,
            next_customer_id: 1,
            next_vendor_id: 1,
            next_invoice_id: 1,
            next_bill_id: 1,
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

    /// The id the next accounting period created will receive.
    ///
    /// See [`Self::next_account_id`].
    pub fn next_period_id(&self) -> PeriodId {
        self.next_period_id
    }

    /// The id the next customer added will receive.
    ///
    /// See [`Self::next_account_id`].
    pub fn next_customer_id(&self) -> CustomerId {
        self.next_customer_id
    }

    /// The id the next vendor added will receive.
    ///
    /// See [`Self::next_account_id`].
    pub fn next_vendor_id(&self) -> VendorId {
        self.next_vendor_id
    }

    /// The id the next invoice created will receive.
    ///
    /// See [`Self::next_account_id`].
    pub fn next_invoice_id(&self) -> InvoiceId {
        self.next_invoice_id
    }

    /// The id the next bill created will receive.
    ///
    /// See [`Self::next_account_id`].
    pub fn next_bill_id(&self) -> BillId {
        self.next_bill_id
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
    ///
    /// This is the unchecked constructor: replay and internal callers
    /// (closing entries) use it. For the checked version that rejects
    /// dates in closed periods, see [`Self::try_create_entry`].
    pub fn create_entry(&mut self, date: NaiveDate, description: &str) -> EntryId {
        let id = self.next_entry_id;
        self.next_entry_id += 1;
        self.entries
            .insert(id, JournalEntry::draft(id, date, description.to_string()));
        id
    }

    /// Start a draft journal entry, rejecting dates that fall in a closed
    /// period. This is the entry point for new work; [`Self::create_entry`]
    /// is for replay and internal use.
    pub fn try_create_entry(
        &mut self,
        date: NaiveDate,
        description: &str,
    ) -> Result<EntryId, LedgerError> {
        if let Some(pid) = self.closed_period_containing(date) {
            return Err(LedgerError::ClosedPeriod(pid, date.to_string()));
        }
        Ok(self.create_entry(date, description))
    }

    /// The id of a closed period containing `date`, if any.
    ///
    /// Pure query. Journaling stores use it to pre-validate dates before
    /// journaling, so a journal write is never followed by a failed mutation.
    pub fn closed_period_containing(&self, date: NaiveDate) -> Option<PeriodId> {
        self.periods
            .values()
            .find(|p| p.status == PeriodStatus::Closed && p.contains(date))
            .map(|p| p.id)
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
            foreign_currency: None,
            foreign_amount: None,
        });
        Ok(())
    }

    /// Add a foreign-currency line to a draft entry. `foreign_amount` is the
    /// amount in `foreign_currency`; `fx_rate` converts foreign -> base
    /// (base = foreign * rate). The stored `amount` is the base-currency
    /// equivalent; the foreign original is kept for reference. Rates are
    /// exact Decimals, never floats.
    pub fn add_line_fx(
        &mut self,
        entry: EntryId,
        account: AccountId,
        side: Side,
        foreign_currency: Currency,
        foreign_amount: Decimal,
        fx_rate: Decimal,
    ) -> Result<(), LedgerError> {
        if foreign_amount <= Decimal::ZERO {
            return Err(LedgerError::InvalidDocumentLineAmount(foreign_amount));
        }
        if fx_rate <= Decimal::ZERO {
            return Err(LedgerError::InvalidFxRate(fx_rate));
        }
        if foreign_currency == self.base_currency {
            return Err(LedgerError::CurrencyMismatch {
                expected: "foreign currency".to_string(),
                found: foreign_currency.code().to_string(),
            });
        }
        let base_amount = foreign_amount * fx_rate;
        if base_amount <= Decimal::ZERO {
            return Err(LedgerError::InvalidFxRate(fx_rate));
        }
        let base = self.base_currency;
        let e = self.draft_mut(entry)?;
        e.push_line(JournalLine {
            account,
            side,
            amount: Money::new(base_amount, base),
            foreign_currency: Some(foreign_currency),
            foreign_amount: Some(foreign_amount),
        });
        Ok(())
    }

    /// Validate and post a draft entry. One-way: a posted entry is immutable.
    ///
    /// Rejects entries dated in a closed period.
    pub fn post_entry(&mut self, entry: EntryId) -> Result<(), LedgerError> {
        let snapshot = self
            .entries
            .get(&entry)
            .ok_or(LedgerError::UnknownEntry(entry))?
            .clone();
        if snapshot.status() == EntryStatus::Posted {
            return Err(LedgerError::EntryAlreadyPosted(entry));
        }
        // Closed periods reject new postings.
        if let Some(pid) = self.closed_period_containing(snapshot.date()) {
            return Err(LedgerError::ClosedPeriod(pid, snapshot.date().to_string()));
        }
        // Clone so validation borrows end before the mutable mark.
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

    // ------------------------------------------------------------------
    // Accounting periods
    // ------------------------------------------------------------------

    /// Create an accounting period. The date range must be valid
    /// (end >= start). Returns the period id.
    pub fn create_period(
        &mut self,
        name: &str,
        start: NaiveDate,
        end: NaiveDate,
    ) -> Result<PeriodId, LedgerError> {
        periods::validate_period_range(start, end)?;
        let id = self.next_period_id;
        self.next_period_id += 1;
        self.periods.insert(
            id,
            Period {
                id,
                name: name.to_string(),
                start,
                end,
                status: PeriodStatus::Open,
                closing_entry: None,
            },
        );
        Ok(id)
    }

    /// Look up a period by id.
    pub fn get_period(&self, id: PeriodId) -> Option<&Period> {
        self.periods.get(&id)
    }

    /// All periods, in creation order.
    pub fn list_periods(&self) -> Vec<&Period> {
        let mut ids: Vec<PeriodId> = self.periods.keys().copied().collect();
        ids.sort_unstable();
        ids.into_iter()
            .filter_map(|id| self.periods.get(&id))
            .collect()
    }

    /// Close a period. This:
    /// 1. Rejects if the period is already closed.
    /// 2. Rejects if any draft entries have dates inside the period.
    /// 3. Posts a closing entry dated at period end: revenue accounts are
    ///    debited to zero, expense accounts are credited to zero, and the
    ///    net goes to Retained Earnings (account `3999`, created if missing).
    /// 4. Marks the period closed.
    ///
    /// Returns the closing entry's id, or `None` if there was nothing to
    /// close (no revenue/expense movement).
    pub fn close_period(&mut self, id: PeriodId) -> Result<Option<EntryId>, LedgerError> {
        let period = self
            .periods
            .get(&id)
            .ok_or(LedgerError::UnknownPeriod(id))?
            .clone();
        if period.status == PeriodStatus::Closed {
            return Err(LedgerError::PeriodAlreadyClosed(id));
        }
        // No drafts may have dates inside the period.
        let has_drafts = self
            .entries
            .values()
            .any(|e| e.status() == EntryStatus::Draft && period.contains(e.date()));
        if has_drafts {
            return Err(LedgerError::DraftsInPeriod(id));
        }

        // Collect revenue and expense accounts with non-zero movement.
        let mut closings: Vec<(AccountId, Side, Decimal)> = Vec::new();
        let mut accounts: Vec<_> = self.chart.all().collect();
        accounts.sort_by(|a, b| a.code.cmp(&b.code));
        for account in accounts {
            match account.account_type {
                AccountType::Revenue | AccountType::Expense => {
                    let movement = self.account_movement(account.id);
                    if movement.is_zero() {
                        continue;
                    }
                    // To zero the account: post the opposite of its net movement.
                    let (side, amount) = if movement > Decimal::ZERO {
                        (Side::Credit, movement)
                    } else {
                        (Side::Debit, -movement)
                    };
                    closings.push((account.id, side, amount));
                }
                _ => {}
            }
        }

        let closing_entry = if closings.is_empty() {
            None
        } else {
            // Ensure Retained Earnings exists.
            let re_id = match self.chart.find_by_code("3999") {
                Some(a) => a.id,
                None => self.add_account(
                    "3999",
                    "Retained Earnings",
                    AccountType::Equity,
                    None,
                    false,
                )?,
            };
            // Sum the closing lines to find the net for RE.
            let mut debits = Decimal::ZERO;
            let mut credits = Decimal::ZERO;
            for (_, side, amount) in &closings {
                match side {
                    Side::Debit => debits += *amount,
                    Side::Credit => credits += *amount,
                }
            }
            let entry = self.create_entry(period.end, &format!("Close period {}", period.name));
            for (account, side, amount) in closings {
                self.add_line(entry, account, side, Money::new(amount, self.base_currency))?;
            }
            // Balance with Retained Earnings.
            if debits > credits {
                // Profit: credit RE.
                self.add_line(
                    entry,
                    re_id,
                    Side::Credit,
                    Money::new(debits - credits, self.base_currency),
                )?;
            } else if credits > debits {
                // Loss: debit RE.
                self.add_line(
                    entry,
                    re_id,
                    Side::Debit,
                    Money::new(credits - debits, self.base_currency),
                )?;
            }
            self.post_entry(entry)?;
            Some(entry)
        };

        self.periods.get_mut(&id).expect("period exists").status = PeriodStatus::Closed;
        self.periods
            .get_mut(&id)
            .expect("period exists")
            .closing_entry = closing_entry;
        Ok(closing_entry)
    }

    /// Pure pre-close check for journaling stores: verifies that
    /// [`Self::close_period`] would succeed — the period exists and is open,
    /// no draft entries are dated inside it, and the closing entry's date
    /// (period end) is not inside another closed period — and returns the id
    /// the closing entry will receive, or `None` when the close posts no
    /// entry (no revenue/expense movement). [`Self::close_period`] repeats
    /// these checks before mutating; this exists so stores can journal the
    /// outcome before mutating, keeping a failed write from leaving a
    /// journal line behind.
    pub fn preview_close_period(&self, id: PeriodId) -> Result<Option<EntryId>, LedgerError> {
        let period = self
            .periods
            .get(&id)
            .ok_or(LedgerError::UnknownPeriod(id))?;
        if period.status == PeriodStatus::Closed {
            return Err(LedgerError::PeriodAlreadyClosed(id));
        }
        let has_drafts = self
            .entries
            .values()
            .any(|e| e.status() == EntryStatus::Draft && period.contains(e.date()));
        if has_drafts {
            return Err(LedgerError::DraftsInPeriod(id));
        }
        if let Some(other) = self.closed_period_containing(period.end)
            && other != id
        {
            return Err(LedgerError::ClosedPeriod(other, period.end.to_string()));
        }
        // The close posts an entry exactly when some revenue or expense
        // account has non-zero movement — the same predicate close_period
        // uses to decide. The entry id is the next one the allocator hands
        // out; account creation (e.g. Retained Earnings) does not consume
        // entry ids.
        let any_movement = self.chart.all().any(|a| {
            matches!(a.account_type, AccountType::Revenue | AccountType::Expense)
                && !self.account_movement(a.id).is_zero()
        });
        Ok(any_movement.then_some(self.next_entry_id))
    }

    // ------------------------------------------------------------------
    // Accounts receivable / payable
    // ------------------------------------------------------------------

    /// Add a customer. Returns its id.
    pub fn add_customer(&mut self, name: &str) -> Result<CustomerId, LedgerError> {
        if name.trim().is_empty() {
            return Err(LedgerError::EmptyAssetDescription);
        }
        let id = self.next_customer_id;
        self.next_customer_id += 1;
        self.customers.insert(
            id,
            Customer {
                id,
                name: name.to_string(),
            },
        );
        Ok(id)
    }

    /// Add a vendor. Returns its id.
    pub fn add_vendor(&mut self, name: &str) -> Result<VendorId, LedgerError> {
        if name.trim().is_empty() {
            return Err(LedgerError::EmptyAssetDescription);
        }
        let id = self.next_vendor_id;
        self.next_vendor_id += 1;
        self.vendors.insert(
            id,
            Vendor {
                id,
                name: name.to_string(),
            },
        );
        Ok(id)
    }

    /// Look up a customer.
    pub fn get_customer(&self, id: CustomerId) -> Option<&Customer> {
        self.customers.get(&id)
    }

    /// Look up a vendor.
    pub fn get_vendor(&self, id: VendorId) -> Option<&Vendor> {
        self.vendors.get(&id)
    }

    /// All customers, in id order.
    pub fn list_customers(&self) -> Vec<&Customer> {
        let mut ids: Vec<CustomerId> = self.customers.keys().copied().collect();
        ids.sort_unstable();
        ids.into_iter()
            .filter_map(|id| self.customers.get(&id))
            .collect()
    }

    /// All vendors, in id order.
    pub fn list_vendors(&self) -> Vec<&Vendor> {
        let mut ids: Vec<VendorId> = self.vendors.keys().copied().collect();
        ids.sort_unstable();
        ids.into_iter()
            .filter_map(|id| self.vendors.get(&id))
            .collect()
    }

    /// Ensure an account exists by code, creating it with the given type
    /// and name if missing. Returns the account id.
    fn ensure_account(&mut self, code: &str, name: &str, account_type: AccountType) -> AccountId {
        if let Some(a) = self.chart.find_by_code(code) {
            return a.id;
        }
        self.add_account(code, name, account_type, None, false)
            .expect("fresh code cannot fail")
    }

    /// Create a sales invoice. Auto-posts: Dr `1200` Accounts Receivable,
    /// Cr `4000` Revenue. The invoice date must not fall in a closed period.
    pub fn create_invoice(
        &mut self,
        customer_id: CustomerId,
        date: NaiveDate,
        due_date: NaiveDate,
        lines: Vec<DocumentLine>,
    ) -> Result<InvoiceId, LedgerError> {
        if !self.customers.contains_key(&customer_id) {
            return Err(LedgerError::UnknownCustomer(customer_id));
        }
        arap::validate_lines(&lines)?;
        arap::validate_due_date(date, due_date)?;
        if let Some(pid) = self.closed_period_containing(date) {
            return Err(LedgerError::ClosedPeriod(pid, date.to_string()));
        }
        let total: Decimal = lines.iter().map(|l| l.amount).sum();
        let ar = self.ensure_account("1200", "Accounts Receivable", AccountType::Asset);
        let revenue = self.ensure_account("4000", "Revenue", AccountType::Revenue);
        let entry = self.create_entry(date, &format!("Invoice {}", self.next_invoice_id));
        let money = Money::new(total, self.base_currency);
        self.add_line(entry, ar, Side::Debit, money)?;
        self.add_line(entry, revenue, Side::Credit, money)?;
        self.post_entry(entry)?;
        let id = self.next_invoice_id;
        self.next_invoice_id += 1;
        self.invoices.insert(
            id,
            Invoice {
                id,
                customer_id,
                date,
                due_date,
                lines,
                amount_paid: Decimal::ZERO,
                status: DocumentStatus::Open,
                entry_id: entry,
            },
        );
        Ok(id)
    }

    /// Create a vendor bill. Auto-posts: Dr `6999` General Expenses,
    /// Cr `2000` Accounts Payable. The bill date must not fall in a closed
    /// period.
    pub fn create_bill(
        &mut self,
        vendor_id: VendorId,
        date: NaiveDate,
        due_date: NaiveDate,
        lines: Vec<DocumentLine>,
    ) -> Result<BillId, LedgerError> {
        if !self.vendors.contains_key(&vendor_id) {
            return Err(LedgerError::UnknownVendor(vendor_id));
        }
        arap::validate_lines(&lines)?;
        arap::validate_due_date(date, due_date)?;
        if let Some(pid) = self.closed_period_containing(date) {
            return Err(LedgerError::ClosedPeriod(pid, date.to_string()));
        }
        let total: Decimal = lines.iter().map(|l| l.amount).sum();
        let expense = self.ensure_account("6999", "General Expenses", AccountType::Expense);
        let ap = self.ensure_account("2000", "Accounts Payable", AccountType::Liability);
        let entry = self.create_entry(date, &format!("Bill {}", self.next_bill_id));
        let money = Money::new(total, self.base_currency);
        self.add_line(entry, expense, Side::Debit, money)?;
        self.add_line(entry, ap, Side::Credit, money)?;
        self.post_entry(entry)?;
        let id = self.next_bill_id;
        self.next_bill_id += 1;
        self.bills.insert(
            id,
            Bill {
                id,
                vendor_id,
                date,
                due_date,
                lines,
                amount_paid: Decimal::ZERO,
                status: DocumentStatus::Open,
                entry_id: entry,
            },
        );
        Ok(id)
    }

    /// Look up an invoice.
    pub fn get_invoice(&self, id: InvoiceId) -> Option<&Invoice> {
        self.invoices.get(&id)
    }

    /// Look up a bill.
    pub fn get_bill(&self, id: BillId) -> Option<&Bill> {
        self.bills.get(&id)
    }

    /// Record a customer payment against an invoice. Posts: Dr Cash,
    /// Cr A/R. Updates the invoice's paid amount and status.
    pub fn receive_payment(
        &mut self,
        invoice_id: InvoiceId,
        date: NaiveDate,
        amount: Decimal,
        cash_account: AccountId,
    ) -> Result<EntryId, LedgerError> {
        if amount <= Decimal::ZERO {
            return Err(LedgerError::InvalidPaymentAmount(amount));
        }
        let invoice = self
            .invoices
            .get(&invoice_id)
            .ok_or(LedgerError::UnknownInvoice(invoice_id))?
            .clone();
        let balance = invoice.balance();
        if amount > balance {
            return Err(LedgerError::Overpayment(amount, balance));
        }
        if let Some(pid) = self.closed_period_containing(date) {
            return Err(LedgerError::ClosedPeriod(pid, date.to_string()));
        }
        let ar = self.ensure_account("1200", "Accounts Receivable", AccountType::Asset);
        let entry = self.create_entry(date, &format!("Payment on invoice {}", invoice_id));
        let money = Money::new(amount, self.base_currency);
        self.add_line(entry, cash_account, Side::Debit, money)?;
        self.add_line(entry, ar, Side::Credit, money)?;
        self.post_entry(entry)?;
        let inv = self.invoices.get_mut(&invoice_id).expect("invoice exists");
        inv.amount_paid += amount;
        inv.status = arap::derive_status(inv.amount_paid, inv.total());
        Ok(entry)
    }

    /// Pay a vendor bill. Posts: Dr A/P, Cr Cash. Updates the bill's paid
    /// amount and status.
    pub fn pay_bill(
        &mut self,
        bill_id: BillId,
        date: NaiveDate,
        amount: Decimal,
        cash_account: AccountId,
    ) -> Result<EntryId, LedgerError> {
        if amount <= Decimal::ZERO {
            return Err(LedgerError::InvalidPaymentAmount(amount));
        }
        let bill = self
            .bills
            .get(&bill_id)
            .ok_or(LedgerError::UnknownBill(bill_id))?
            .clone();
        let balance = bill.balance();
        if amount > balance {
            return Err(LedgerError::Overpayment(amount, balance));
        }
        if let Some(pid) = self.closed_period_containing(date) {
            return Err(LedgerError::ClosedPeriod(pid, date.to_string()));
        }
        let ap = self.ensure_account("2000", "Accounts Payable", AccountType::Liability);
        let entry = self.create_entry(date, &format!("Payment on bill {}", bill_id));
        let money = Money::new(amount, self.base_currency);
        self.add_line(entry, ap, Side::Debit, money)?;
        self.add_line(entry, cash_account, Side::Credit, money)?;
        self.post_entry(entry)?;
        let b = self.bills.get_mut(&bill_id).expect("bill exists");
        b.amount_paid += amount;
        b.status = arap::derive_status(b.amount_paid, b.total());
        Ok(entry)
    }

    /// Record an invoice without posting: replay support for journaling
    /// stores. The invoice's entry was already rebuilt from its own
    /// `entry.created` / `entry.line_added` / `entry.posted` events; this
    /// only restores the invoice record. The id must be the next invoice
    /// id — anything else means the journal and the allocator disagree.
    #[doc(hidden)]
    pub fn replay_invoice(
        &mut self,
        id: InvoiceId,
        customer_id: CustomerId,
        date: NaiveDate,
        due_date: NaiveDate,
        lines: Vec<DocumentLine>,
        entry_id: EntryId,
    ) -> Result<(), LedgerError> {
        if id != self.next_invoice_id {
            return Err(LedgerError::ReplayIdMismatch(id, self.next_invoice_id));
        }
        self.next_invoice_id += 1;
        self.invoices.insert(
            id,
            Invoice {
                id,
                customer_id,
                date,
                due_date,
                lines,
                amount_paid: Decimal::ZERO,
                status: DocumentStatus::Open,
                entry_id,
            },
        );
        Ok(())
    }

    /// Record a vendor bill without posting: replay support for journaling
    /// stores. See [`Self::replay_invoice`].
    #[doc(hidden)]
    pub fn replay_bill(
        &mut self,
        id: BillId,
        vendor_id: VendorId,
        date: NaiveDate,
        due_date: NaiveDate,
        lines: Vec<DocumentLine>,
        entry_id: EntryId,
    ) -> Result<(), LedgerError> {
        if id != self.next_bill_id {
            return Err(LedgerError::ReplayIdMismatch(id, self.next_bill_id));
        }
        self.next_bill_id += 1;
        self.bills.insert(
            id,
            Bill {
                id,
                vendor_id,
                date,
                due_date,
                lines,
                amount_paid: Decimal::ZERO,
                status: DocumentStatus::Open,
                entry_id,
            },
        );
        Ok(())
    }

    /// Apply an invoice payment without posting: replay support for
    /// journaling stores. The payment entry was already rebuilt from its
    /// own entry events; this only moves the invoice's paid amount and
    /// status. `entry_id` is the journaled payment entry, carried for
    /// audit; the invoice does not link payment entries.
    #[doc(hidden)]
    pub fn replay_invoice_payment(
        &mut self,
        invoice_id: InvoiceId,
        amount: Decimal,
        entry_id: EntryId,
    ) -> Result<(), LedgerError> {
        let _ = entry_id;
        let inv = self
            .invoices
            .get_mut(&invoice_id)
            .ok_or(LedgerError::UnknownInvoice(invoice_id))?;
        inv.amount_paid += amount;
        inv.status = arap::derive_status(inv.amount_paid, inv.total());
        Ok(())
    }

    /// Apply a bill payment without posting: replay support for journaling
    /// stores. See [`Self::replay_invoice_payment`].
    #[doc(hidden)]
    pub fn replay_bill_payment(
        &mut self,
        bill_id: BillId,
        amount: Decimal,
        entry_id: EntryId,
    ) -> Result<(), LedgerError> {
        let _ = entry_id;
        let bill = self
            .bills
            .get_mut(&bill_id)
            .ok_or(LedgerError::UnknownBill(bill_id))?;
        bill.amount_paid += amount;
        bill.status = arap::derive_status(bill.amount_paid, bill.total());
        Ok(())
    }

    /// A/R aging as of a date: one row per customer with an outstanding
    /// balance, bucketed by days past due.
    pub fn ar_aging(&self, as_of: NaiveDate) -> Vec<AgingRow> {
        let mut by_customer: HashMap<CustomerId, AgingRow> = HashMap::new();
        for inv in self.invoices.values() {
            let balance = inv.balance();
            if balance.is_zero() {
                continue;
            }
            let row = arap::aging_bucket(inv.due_date, as_of, balance);
            let entry = by_customer
                .entry(inv.customer_id)
                .or_insert_with(|| AgingRow {
                    name: self
                        .customers
                        .get(&inv.customer_id)
                        .map(|c| c.name.clone())
                        .unwrap_or_default(),
                    current: Decimal::ZERO,
                    days_30: Decimal::ZERO,
                    days_60: Decimal::ZERO,
                    days_90: Decimal::ZERO,
                    days_90_plus: Decimal::ZERO,
                });
            entry.current += row.current;
            entry.days_30 += row.days_30;
            entry.days_60 += row.days_60;
            entry.days_90 += row.days_90;
            entry.days_90_plus += row.days_90_plus;
        }
        let mut rows: Vec<AgingRow> = by_customer.into_values().collect();
        rows.sort_by(|a, b| a.name.cmp(&b.name));
        rows
    }

    /// A/P aging as of a date: one row per vendor with an outstanding
    /// balance, bucketed by days past due.
    pub fn ap_aging(&self, as_of: NaiveDate) -> Vec<AgingRow> {
        let mut by_vendor: HashMap<VendorId, AgingRow> = HashMap::new();
        for bill in self.bills.values() {
            let balance = bill.balance();
            if balance.is_zero() {
                continue;
            }
            let row = arap::aging_bucket(bill.due_date, as_of, balance);
            let entry = by_vendor.entry(bill.vendor_id).or_insert_with(|| AgingRow {
                name: self
                    .vendors
                    .get(&bill.vendor_id)
                    .map(|v| v.name.clone())
                    .unwrap_or_default(),
                current: Decimal::ZERO,
                days_30: Decimal::ZERO,
                days_60: Decimal::ZERO,
                days_90: Decimal::ZERO,
                days_90_plus: Decimal::ZERO,
            });
            entry.current += row.current;
            entry.days_30 += row.days_30;
            entry.days_60 += row.days_60;
            entry.days_90 += row.days_90;
            entry.days_90_plus += row.days_90_plus;
        }
        let mut rows: Vec<AgingRow> = by_vendor.into_values().collect();
        rows.sort_by(|a, b| a.name.cmp(&b.name));
        rows
    }

    // ------------------------------------------------------------------
    // Bank reconciliation
    // ------------------------------------------------------------------

    /// Mark a posted entry's line as cleared (or uncleared). The cleared
    /// flag is a workflow marker, not accounting data: posted entries stay
    /// immutable, and the flag lives in a side table.
    pub fn mark_cleared(
        &mut self,
        entry_id: EntryId,
        line_index: usize,
        cleared: bool,
    ) -> Result<(), LedgerError> {
        let entry = self
            .entries
            .get(&entry_id)
            .ok_or(LedgerError::UnknownEntry(entry_id))?;
        if entry.status() != EntryStatus::Posted {
            return Err(LedgerError::ClearedOnDraft(entry_id));
        }
        if line_index >= entry.lines().len() {
            return Err(LedgerError::LineIndexOutOfRange(line_index, entry_id));
        }
        if cleared {
            self.cleared.insert((entry_id, line_index), true);
        } else {
            self.cleared.remove(&(entry_id, line_index));
        }
        Ok(())
    }

    /// True if the given posted line is marked cleared.
    pub fn is_cleared(&self, entry_id: EntryId, line_index: usize) -> bool {
        self.cleared
            .get(&(entry_id, line_index))
            .copied()
            .unwrap_or(false)
    }

    /// Reconcile a bank/cash account against a statement. Compares the
    /// cleared balance (posted lines marked cleared, dated on or before
    /// the statement date) to the statement balance, and lists every
    /// uncleared line so the human can see what's outstanding.
    pub fn reconciliation_report(
        &self,
        account: AccountId,
        statement_date: NaiveDate,
        statement_balance: Decimal,
    ) -> Result<ReconciliationReport, LedgerError> {
        let acct = self
            .chart
            .get(account)
            .ok_or(LedgerError::UnknownAccount(account))?;
        let normal = acct.account_type.normal_side();
        let mut book = Decimal::ZERO;
        let mut cleared = Decimal::ZERO;
        let mut uncleared_lines = Vec::new();
        for entry in self.posted_entries() {
            for (idx, line) in entry.lines().iter().enumerate() {
                if line.account != account {
                    continue;
                }
                // Signed in the account's normal side: positive = normal balance.
                let signed = match (line.side, normal) {
                    (Side::Debit, Side::Debit) => line.amount.amount,
                    (Side::Credit, Side::Credit) => line.amount.amount,
                    (Side::Debit, Side::Credit) => -line.amount.amount,
                    (Side::Credit, Side::Debit) => -line.amount.amount,
                };
                book += signed;
                if entry.date() > statement_date {
                    continue;
                }
                if self.is_cleared(entry.id(), idx) {
                    cleared += signed;
                } else {
                    uncleared_lines.push(UnclearedLine {
                        entry_id: entry.id(),
                        line_index: idx,
                        date: entry.date(),
                        description: entry.description().to_string(),
                        side: line.side,
                        amount: line.amount.amount,
                    });
                }
            }
        }
        // Sort uncleared by date for a readable report.
        uncleared_lines.sort_by(|a, b| a.date.cmp(&b.date).then(a.entry_id.cmp(&b.entry_id)));
        Ok(ReconciliationReport {
            account,
            book_balance: if book.is_zero() { Decimal::ZERO } else { book },
            cleared_balance: if cleared.is_zero() {
                Decimal::ZERO
            } else {
                cleared
            },
            statement_balance,
            difference: cleared - statement_balance,
            uncleared_lines,
        })
    }

    // ------------------------------------------------------------------
    // Foreign exchange
    // ------------------------------------------------------------------

    /// Record a historical FX rate for reference: how much base currency
    /// one unit of `currency` buys on `date`. Lines store their own rate at
    /// entry time; this table is for lookup and audit, not conversion.
    pub fn set_fx_rate(
        &mut self,
        currency: Currency,
        date: NaiveDate,
        rate: Decimal,
    ) -> Result<(), LedgerError> {
        if rate <= Decimal::ZERO {
            return Err(LedgerError::InvalidFxRate(rate));
        }
        if currency == self.base_currency {
            return Err(LedgerError::CurrencyMismatch {
                expected: "foreign currency".to_string(),
                found: currency.code().to_string(),
            });
        }
        self.fx_rates.insert((currency, date), rate);
        Ok(())
    }

    /// Look up a recorded FX rate.
    pub fn get_fx_rate(&self, currency: Currency, date: NaiveDate) -> Option<Decimal> {
        self.fx_rates.get(&(currency, date)).copied()
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
