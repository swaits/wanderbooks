//! The ledger: the command surface of the engine.
//!
//! A [`Ledger`] owns the chart of accounts and the append-only log of posted
//! journal entries. Every mutation is a method on `Ledger` — create, add
//! line, post, reverse — and every method validates before it applies. There
//! is no other way to change the books, which is what makes the ledger an
//! audit trail and what will make the future oracle harness trustworthy.

use crate::schedules::{
    self, AmortizationSchedule, NewRecurring, NewSchedule, RecurringEntry, RecurringId, ScheduleId,
};
use crate::{
    Account, AccountId, AccountType, AgingRow, Asset, AssetId, AssetStatus, Bill, BillId,
    ChartOfAccounts, Class, ClassId, Currency, Customer, CustomerId, DocumentLine, DocumentStatus,
    EntryId, EntryStatus, Invoice, InvoiceId, JournalEntry, JournalLine, LedgerError, Money,
    NewAsset, Period, PeriodId, PeriodStatus, Side, Vendor, VendorId, arap, assets, kernel,
    payroll::{
        PayrollAccounts, PayrollLine, PayrollRun, PayrollRunId, payroll_entry_plan,
        payroll_remit_plan, validate_payroll_lines,
    },
    periods,
};
use chrono::{Months, NaiveDate};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// An account the engine posts to by code: invoices, bills, payroll, and
/// closes find (or create) these. See [`Ledger::check_controls`].
#[derive(Clone, Copy, Debug)]
pub struct ControlAccount {
    /// The code the engine looks for.
    pub code: &'static str,
    /// The name it gives the account when it creates it.
    pub name: &'static str,
    /// What the account is for, in a refusal message.
    pub role: &'static str,
    /// The type the role needs.
    pub account_type: AccountType,
}

const fn control(
    code: &'static str,
    name: &'static str,
    role: &'static str,
    account_type: AccountType,
) -> ControlAccount {
    ControlAccount {
        code,
        name,
        role,
        account_type,
    }
}

/// A control account; see [`ControlAccount`].
pub const RECEIVABLES: ControlAccount = control(
    "1200",
    "Accounts Receivable",
    "accounts receivable",
    AccountType::Asset,
);
/// A control account; see [`ControlAccount`].
pub const REVENUE: ControlAccount =
    control("4000", "Revenue", "invoice revenue", AccountType::Revenue);
/// A control account; see [`ControlAccount`].
pub const PAYABLES: ControlAccount = control(
    "2000",
    "Accounts Payable",
    "accounts payable",
    AccountType::Liability,
);
/// A control account; see [`ControlAccount`].
pub const GENERAL_EXPENSES: ControlAccount = control(
    "6999",
    "General Expenses",
    "bill expense",
    AccountType::Expense,
);
/// A control account; see [`ControlAccount`].
pub const RETAINED_EARNINGS: ControlAccount = control(
    periods::RETAINED_EARNINGS_CODE,
    "Retained Earnings",
    "retained earnings",
    AccountType::Equity,
);
/// The payroll accounts, in [`PayrollAccounts`] order.
pub const PAYROLL: [ControlAccount; 7] = [
    control(
        "6100",
        "Salary & Wage Expense",
        "salary expense",
        AccountType::Expense,
    ),
    control(
        "6110",
        "Payroll Tax Expense",
        "payroll tax expense",
        AccountType::Expense,
    ),
    control(
        "2110",
        "Federal Withholding Payable",
        "federal withholding payable",
        AccountType::Liability,
    ),
    control(
        "2120",
        "State Withholding Payable",
        "state withholding payable",
        AccountType::Liability,
    ),
    control(
        "2130",
        "SS Tax Payable",
        "social security tax payable",
        AccountType::Liability,
    ),
    control(
        "2140",
        "Medicare Tax Payable",
        "Medicare tax payable",
        AccountType::Liability,
    ),
    control(
        "2150",
        "Other Deductions Payable",
        "other deductions payable",
        AccountType::Liability,
    ),
];

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
#[derive(Clone, Debug)]
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
    /// Payroll runs.
    payroll_runs: HashMap<PayrollRunId, PayrollRun>,
    /// Classes for divisional P&L tracking.
    classes: HashMap<ClassId, Class>,
    next_account_id: AccountId,
    next_entry_id: EntryId,
    next_asset_id: AssetId,
    next_period_id: PeriodId,
    next_customer_id: CustomerId,
    next_vendor_id: VendorId,
    next_invoice_id: InvoiceId,
    next_bill_id: BillId,
    next_payroll_run_id: PayrollRunId,
    next_class_id: ClassId,
    /// Amortization schedules (prepaid expenses, deferred revenue).
    schedules: std::collections::BTreeMap<ScheduleId, AmortizationSchedule>,
    next_schedule_id: ScheduleId,
    /// Recurring entries.
    recurring: std::collections::BTreeMap<RecurringId, RecurringEntry>,
    next_recurring_id: RecurringId,
    /// Who each entry paid or was billed by, for 1099 reporting.
    entry_vendors: HashMap<EntryId, VendorId>,
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
            payroll_runs: HashMap::new(),
            classes: HashMap::new(),
            next_account_id: 1,
            next_entry_id: 1,
            next_asset_id: 1,
            next_period_id: 1,
            next_customer_id: 1,
            next_vendor_id: 1,
            next_invoice_id: 1,
            next_bill_id: 1,
            next_payroll_run_id: 1,
            next_class_id: 1,
            schedules: std::collections::BTreeMap::new(),
            next_schedule_id: 1,
            recurring: std::collections::BTreeMap::new(),
            next_recurring_id: 1,
            entry_vendors: HashMap::new(),
        }
    }

    // ------------------------------------------------------------------
    // Amortization schedules and recurring entries
    // ------------------------------------------------------------------

    /// Register an amortization schedule. Returns its id.
    pub fn register_schedule(&mut self, new: NewSchedule) -> Result<ScheduleId, LedgerError> {
        schedules::validate_schedule(&new, &self.chart)?;
        let id = self.next_schedule_id;
        self.next_schedule_id += 1;
        self.schedules.insert(
            id,
            AmortizationSchedule {
                id,
                description: new.description,
                kind: new.kind,
                total: new.total,
                start: new.start,
                months: new.months,
                balance_account: new.balance_account,
                pl_account: new.pl_account,
                posted: Default::default(),
            },
        );
        Ok(id)
    }

    /// The id the next schedule will receive (for journaling stores).
    pub fn next_schedule_id(&self) -> ScheduleId {
        self.next_schedule_id
    }

    /// All schedules, by id.
    pub fn schedules(&self) -> impl Iterator<Item = &AmortizationSchedule> + '_ {
        self.schedules.values()
    }

    /// Look up a schedule.
    pub fn get_schedule(&self, id: ScheduleId) -> Option<&AmortizationSchedule> {
        self.schedules.get(&id)
    }

    /// The entry [`Self::post_schedule_month`] would post: (date,
    /// description, lines). Errors if the month isn't due and unposted.
    pub fn preview_schedule_month(
        &self,
        id: ScheduleId,
        month_end: NaiveDate,
    ) -> Result<(NaiveDate, String, Vec<crate::schedules::TemplateLine>), LedgerError> {
        let s = self
            .schedules
            .get(&id)
            .ok_or_else(|| LedgerError::InvalidSchedule(format!("unknown schedule {id}")))?;
        let (date, amount) = schedules::amortization_plan(s.total.amount, s.start, s.months)
            .into_iter()
            .find(|(d, _)| *d == month_end)
            .ok_or_else(|| {
                LedgerError::InvalidSchedule(format!("{month_end} is not a month of schedule {id}"))
            })?;
        if s.posted.contains(&date) {
            return Err(LedgerError::InvalidSchedule(format!(
                "schedule {id} already posted {date}"
            )));
        }
        let money = Money::new(amount, s.total.currency);
        let (debit, credit) = s.accounts();
        let what = match s.kind {
            schedules::ScheduleKind::PrepaidExpense => "Prepaid expense",
            schedules::ScheduleKind::DeferredRevenue => "Deferred revenue",
        };
        Ok((
            date,
            format!("{what}: {} ({})", s.description, date.format("%Y-%m")),
            vec![(debit, Side::Debit, money), (credit, Side::Credit, money)],
        ))
    }

    /// Post one month of a schedule: the entry from
    /// [`Self::preview_schedule_month`], posted and recorded so the month
    /// never posts twice. Returns the entry id.
    pub fn post_schedule_month(
        &mut self,
        id: ScheduleId,
        month_end: NaiveDate,
    ) -> Result<EntryId, LedgerError> {
        let (date, description, lines) = self.preview_schedule_month(id, month_end)?;
        if let Some(p) = self.closed_period_containing(date) {
            return Err(LedgerError::ClosedPeriod(p, date.to_string()));
        }
        let entry = self.create_entry(date, &description);
        for (account, side, money) in lines {
            self.add_line(entry, account, side, money)?;
        }
        self.post_entry(entry)?;
        self.schedules
            .get_mut(&id)
            .expect("previewed")
            .posted
            .insert(date);
        Ok(entry)
    }

    /// Register a recurring entry. Returns its id.
    pub fn register_recurring(&mut self, new: NewRecurring) -> Result<RecurringId, LedgerError> {
        schedules::validate_recurring(&new, &self.chart)?;
        let id = self.next_recurring_id;
        self.next_recurring_id += 1;
        self.recurring.insert(
            id,
            RecurringEntry {
                id,
                description: new.description,
                lines: new.lines,
                frequency: new.frequency,
                start: new.start,
                end: new.end,
                generated: Default::default(),
            },
        );
        Ok(id)
    }

    /// The id the next recurring entry will receive (for journaling stores).
    pub fn next_recurring_id(&self) -> RecurringId {
        self.next_recurring_id
    }

    /// All recurring entries, by id.
    pub fn recurring_entries(&self) -> impl Iterator<Item = &RecurringEntry> + '_ {
        self.recurring.values()
    }

    /// Generate one occurrence of a recurring entry as a *draft* (never
    /// posted: the agent attaches the receipt and posts it). `date` must be
    /// an occurrence not yet generated. Returns the draft's id.
    pub fn generate_recurring(
        &mut self,
        id: RecurringId,
        date: NaiveDate,
    ) -> Result<EntryId, LedgerError> {
        let r = self
            .recurring
            .get(&id)
            .ok_or_else(|| LedgerError::InvalidSchedule(format!("unknown recurring entry {id}")))?;
        if !r.due(date).contains(&date) {
            return Err(LedgerError::InvalidSchedule(format!(
                "{date} is not an ungenerated occurrence of recurring entry {id}"
            )));
        }
        let (description, lines) = (r.description.clone(), r.lines.clone());
        let entry = self.create_entry(date, &description);
        for (account, side, money) in lines {
            self.add_line(entry, account, side, money)?;
        }
        self.recurring
            .get_mut(&id)
            .expect("checked")
            .generated
            .insert(date);
        Ok(entry)
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

    /// The id the next payroll run will receive.
    ///
    /// See [`Self::next_account_id`].
    pub fn next_payroll_run_id(&self) -> PayrollRunId {
        self.next_payroll_run_id
    }

    /// The id the next class added will receive.
    ///
    /// See [`Self::next_account_id`].
    pub fn next_class_id(&self) -> ClassId {
        self.next_class_id
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
        self.chart.add(Account {
            id,
            code: code.to_string(),
            name: name.to_string(),
            account_type,
            parent,
            placeholder,
            currency: self.base_currency,
            tax_category: None,
            deductible_percent: None,
        })?;
        Ok(id)
    }

    /// Set (or clear, with `None`) an account's tax category, and optionally
    /// override the category's deductible percent (0 to 100). The category
    /// must fit the account's type (see [`crate::tax::check_category`]).
    pub fn set_account_tax(
        &mut self,
        account: AccountId,
        category: Option<&str>,
        deductible_percent: Option<Decimal>,
    ) -> Result<(), LedgerError> {
        self.check_account_tax(account, category, deductible_percent)?;
        self.chart
            .set_tax(account, category.map(str::to_string), deductible_percent)
    }

    /// The validation behind [`Self::set_account_tax`], for stores that
    /// journal before mutating.
    pub fn check_account_tax(
        &self,
        account: AccountId,
        category: Option<&str>,
        deductible_percent: Option<Decimal>,
    ) -> Result<(), LedgerError> {
        let a = self
            .chart
            .get(account)
            .ok_or(LedgerError::UnknownAccount(account))?;
        if let Some(key) = category {
            crate::tax::check_category(key, a.account_type, None)?;
        } else if deductible_percent.is_some() {
            return Err(LedgerError::InvalidTaxCategory(
                "a deductible percent needs a tax category".to_string(),
            ));
        }
        if let Some(p) = deductible_percent
            && (p < Decimal::ZERO || p > Decimal::from(100))
        {
            return Err(LedgerError::InvalidTaxCategory(format!(
                "deductible percent {p} must be between 0 and 100"
            )));
        }
        Ok(())
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
            class_id: None,
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
        let base_amount = foreign_amount
            .checked_mul(fx_rate)
            .ok_or(LedgerError::AmountOutOfRange)?;
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
            class_id: None,
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

    /// Pure pre-check for [`Self::reverse_entry`]: every way a reversal can
    /// be refused, checked without touching the books, so journaling stores
    /// can validate before they journal.
    ///
    /// ```text
    /// GOAL: decide whether `entry` may be reversed on `date`.
    /// SOURCE: docs/spec/rules.md B-9 (Pacioli ch. 31, 36), B-10 (DECISION,
    ///         owner 2026-10-13), B-12 (closed periods).
    /// INVARIANTS:
    ///   1. MUST NOT mutate anything.
    ///   2. NEVER accepts a reversal that `reverse_entry` would then refuse.
    /// SCENARIOS:
    ///   1. IF the entry is unknown or still a draft:
    ///      - SHOULD refuse (`UnknownEntry`, `EntryNotPosted`)
    ///      BECAUSE only posted entries are in the books to correct.
    ///   2. IF the entry was already reversed:
    ///      - SHOULD refuse with `EntryAlreadyReversed(entry, by)`
    ///      - NEVER undo it a second time
    ///      BECAUSE a second reversal would undo the original twice
    ///      (owner, 2026-10-13). Reversing the reversal re-instates it.
    ///   3. IF the entry closes a period:
    ///      - SHOULD refuse with `ClosingEntryIrreversible`
    ///      BECAUSE the period would stay closed with its earnings
    ///      unswept (owner, 2026-10-13).
    ///   4. IF `date` falls in a closed period:
    ///      - SHOULD refuse with `ClosedPeriod`
    ///      BECAUSE closed periods take no new postings.
    /// ```
    pub fn check_reversal(&self, entry: EntryId, date: NaiveDate) -> Result<(), LedgerError> {
        let original = self
            .entries
            .get(&entry)
            .ok_or(LedgerError::UnknownEntry(entry))?;
        if original.status() != EntryStatus::Posted {
            return Err(LedgerError::EntryNotPosted(entry));
        }
        if let Some(by) = self
            .posted_entries()
            .find(|e| e.reversal_of() == Some(entry))
        {
            return Err(LedgerError::EntryAlreadyReversed(entry, by.id()));
        }
        if self.closing_entry_ids().contains(&entry) {
            return Err(LedgerError::ClosingEntryIrreversible(entry));
        }
        if let Some(pid) = self.closed_period_containing(date) {
            return Err(LedgerError::ClosedPeriod(pid, date.to_string()));
        }
        Ok(())
    }

    /// Post a reversing entry: same lines with sides flipped, linked back to
    /// the original via `reversal_of`. The books never forget; they correct.
    /// Refused (see [`Self::check_reversal`]) before anything changes.
    pub fn reverse_entry(
        &mut self,
        entry: EntryId,
        date: NaiveDate,
        description: &str,
    ) -> Result<EntryId, LedgerError> {
        self.check_reversal(entry, date)?;
        let original = self
            .entries
            .get(&entry)
            .ok_or(LedgerError::UnknownEntry(entry))?
            .clone();
        let id = self.create_entry(date, description);
        for (idx, line) in original.lines().iter().enumerate() {
            self.add_line(id, line.account, kernel::flip(line.side), line.amount)?;
            // The reversal offsets the original line for line: carry the
            // class so divisional P&L nets correctly.
            if line.class_id.is_some() {
                self.set_line_class(id, idx, line.class_id)?;
            }
        }
        self.entries
            .get_mut(&id)
            .ok_or(LedgerError::UnknownEntry(id))?
            .mark_reversal_of(entry);
        self.post_entry(id)?;
        // A reversed payment un-pays the vendor: carry the tag.
        if let Some(v) = self.entry_vendor(entry) {
            self.entry_vendors.insert(id, v);
        }
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

    /// Ids of every period-closing entry. Period statements (P&L, cash
    /// flow) exclude them: a close moves earnings into retained earnings; it
    /// earns and spends nothing, so counting it would zero out the period.
    pub fn closing_entry_ids(&self) -> std::collections::BTreeSet<EntryId> {
        self.periods
            .values()
            .filter_map(|p| p.closing_entry)
            .collect()
    }

    /// All periods, in creation order.
    pub fn list_periods(&self) -> Vec<&Period> {
        let mut ids: Vec<PeriodId> = self.periods.keys().copied().collect();
        ids.sort_unstable();
        ids.into_iter()
            .filter_map(|id| self.periods.get(&id))
            .collect()
    }

    /// Close a period into Retained Earnings: see [`Self::close_period_to`].
    pub fn close_period(&mut self, id: PeriodId) -> Result<Option<EntryId>, LedgerError> {
        self.close_period_to(id, &periods::CloseTo::RetainedEarnings { sweep: vec![] })
    }

    /// Close a period. This:
    /// 1. Rejects if the period is already closed, if any draft is dated
    ///    inside it, if a later period is already closed (closing out of
    ///    order would sweep the same income twice), or if `to` is invalid.
    /// 2. Posts a closing entry dated at period end that zeroes every revenue
    ///    and expense account's balance *as of the period end* (entries
    ///    dated after it stay open for their own period), puts the net in
    ///    `to`'s target, and zeroes `to`'s sweep accounts (draws,
    ///    distributions, dividends) into their target too:
    ///    - `RetainedEarnings`: account `3999`, created if missing.
    ///    - `Account`: one equity account (a sole proprietor's capital).
    ///    - `Members`: each member's capital account gets their share of
    ///      the net, to the cent (the last absorbs rounding), and their own
    ///      distribution account.
    /// 3. Marks the period closed.
    ///
    /// Returns the closing entry's id, or `None` if there was nothing to
    /// close.
    pub fn close_period_to(
        &mut self,
        id: PeriodId,
        to: &periods::CloseTo,
    ) -> Result<Option<EntryId>, LedgerError> {
        self.preview_close_period_to(id, to)?;
        let period = self.periods.get(&id).expect("previewed").clone();
        let (closings, net) = self.income_closings(period.end);

        // (account, amount to credit; negative debits): the net to its
        // targets, and each sweep account's balance into its target.
        let mut equity: Vec<(AccountId, Decimal)> = Vec::new();
        let mut sweep_lines: Vec<(AccountId, AccountId, Decimal)> = Vec::new();
        let mut sweep = |from: AccountId, into: AccountId, this: &Self| {
            // An equity balance (credit-normal) is −movement; zeroing it
            // moves exactly that much into the target.
            let balance = -this.movement_through(from, period.end);
            if !balance.is_zero() {
                sweep_lines.push((from, into, balance));
            }
        };
        match to {
            periods::CloseTo::RetainedEarnings { sweep: accounts } => {
                let has_work = !closings.is_empty()
                    || accounts
                        .iter()
                        .any(|a| !self.movement_through(*a, period.end).is_zero());
                if has_work {
                    let re = self.retained_earnings_account()?;
                    equity.push((re, net));
                    for a in accounts {
                        sweep(*a, re, self);
                    }
                }
            }
            periods::CloseTo::Account {
                into,
                sweep: accounts,
            } => {
                equity.push((*into, net));
                for a in accounts {
                    sweep(*a, *into, self);
                }
            }
            periods::CloseTo::Members(members) | periods::CloseTo::MembersLastAbsorbs(members) => {
                let shares: Vec<Decimal> = members.iter().map(|m| m.share).collect();
                let parts = match to {
                    periods::CloseTo::MembersLastAbsorbs(_) => {
                        periods::allocate_last_absorbs(net, &shares)
                    }
                    _ => periods::allocate(net, &shares),
                };
                for (m, part) in members.iter().zip(parts) {
                    equity.push((m.capital, part));
                    if let Some(d) = m.distributions {
                        sweep(d, m.capital, self);
                    }
                }
            }
        }

        let closing_entry = if closings.is_empty() && sweep_lines.is_empty() {
            None
        } else {
            let entry = self.create_entry(period.end, &format!("Close period {}", period.name));
            let post = |this: &mut Self, account, credit: Decimal| -> Result<(), LedgerError> {
                match kernel::credit_line(credit) {
                    Some((side, amount)) => {
                        this.add_line(entry, account, side, Money::new(amount, this.base_currency))
                    }
                    None => Ok(()),
                }
            };
            for (account, side, amount) in closings {
                self.add_line(entry, account, side, Money::new(amount, self.base_currency))?;
            }
            for (account, share) in equity {
                post(self, account, share)?;
            }
            // A sweep zeroes the source (opposite its balance) and moves
            // the balance into the target.
            for (from, into, balance) in sweep_lines {
                post(self, from, -balance)?;
                post(self, into, balance)?;
            }
            self.post_entry(entry)?;
            Some(entry)
        };

        let p = self.periods.get_mut(&id).expect("period exists");
        p.status = PeriodStatus::Closed;
        p.closing_entry = closing_entry;
        Ok(closing_entry)
    }

    /// Revenue and expense balances as of `end` as closing lines (code
    /// order), and the net income they carry (revenue debits minus expense
    /// credits).
    fn income_closings(&self, end: NaiveDate) -> (Vec<(AccountId, Side, Decimal)>, Decimal) {
        let mut closings = Vec::new();
        let mut accounts: Vec<_> = self.chart.all().collect();
        accounts.sort_by(|a, b| a.code.cmp(&b.code));
        for account in accounts {
            if !matches!(
                account.account_type,
                AccountType::Revenue | AccountType::Expense
            ) {
                continue;
            }
            // To zero the account: post the opposite of its net movement.
            if let Some((side, amount)) =
                kernel::closing_line(self.movement_through(account.id, end))
            {
                closings.push((account.id, side, amount));
            }
        }
        let net = kernel::net(closings.iter().map(|&(_, side, amount)| (side, amount)))
            .expect("posted lines are in range, so their sums are");
        (closings, net)
    }

    fn retained_earnings_account(&mut self) -> Result<AccountId, LedgerError> {
        Ok(
            match self.chart.find_by_code(periods::RETAINED_EARNINGS_CODE) {
                Some(a) => a.id,
                None => self.add_account(
                    periods::RETAINED_EARNINGS_CODE,
                    "Retained Earnings",
                    AccountType::Equity,
                    None,
                    false,
                )?,
            },
        )
    }

    /// Validate a close target: every account an equity account, targets
    /// and sweeps distinct, members' shares positive and summing to 1.
    fn validate_close_to(&self, to: &periods::CloseTo) -> Result<(), LedgerError> {
        let mut accounts: Vec<AccountId> = Vec::new();
        match to {
            periods::CloseTo::RetainedEarnings { sweep } => accounts.extend(sweep),
            periods::CloseTo::Account { into, sweep } => {
                accounts.push(*into);
                accounts.extend(sweep);
            }
            periods::CloseTo::Members(members) | periods::CloseTo::MembersLastAbsorbs(members) => {
                let total: Decimal = members.iter().map(|m| m.share).sum();
                if members.is_empty()
                    || total != Decimal::ONE
                    || members.iter().any(|m| m.share <= Decimal::ZERO)
                {
                    return Err(LedgerError::InvalidMembers(format!(
                        "member shares must be positive and sum to 1, got {total}"
                    )));
                }
                for m in members {
                    accounts.push(m.capital);
                    accounts.extend(m.distributions);
                }
            }
        }
        let mut seen = std::collections::BTreeSet::new();
        for a in accounts {
            let account = self.chart.get(a).ok_or(LedgerError::UnknownAccount(a))?;
            if account.account_type != AccountType::Equity || !seen.insert(a) {
                return Err(LedgerError::InvalidMembers(format!(
                    "close account {} must be a distinct equity account",
                    account.code
                )));
            }
        }
        Ok(())
    }

    /// Debits minus credits on an account across posted entries dated on or
    /// before `end`.
    fn movement_through(&self, account: AccountId, end: NaiveDate) -> Decimal {
        self.net_of(account, |e| e.date() <= end)
    }

    /// Debits minus credits on an account across the posted entries
    /// `include` accepts.
    fn net_of(&self, account: AccountId, include: impl Fn(&JournalEntry) -> bool) -> Decimal {
        kernel::net(
            self.posted_entries()
                .filter(|e| include(e))
                .flat_map(|e| e.lines())
                .filter(|l| l.account == account)
                .map(|l| (l.side, l.amount.amount)),
        )
        .expect("posted lines are in range, so their sums are")
    }

    /// [`Self::preview_close_period_to`] closing into Retained Earnings.
    pub fn preview_close_period(&self, id: PeriodId) -> Result<Option<EntryId>, LedgerError> {
        self.preview_close_period_to(id, &periods::CloseTo::RetainedEarnings { sweep: vec![] })
    }

    /// Pure pre-close check for journaling stores: verifies that
    /// [`Self::close_period_to`] would succeed — the period exists and is
    /// open, no draft entries are dated inside it, no later period is
    /// closed, the closing date isn't inside another closed period, and the
    /// target is valid — and returns the id the closing entry will receive,
    /// or `None` when the close posts no entry. Stores call this before
    /// journaling so a failed close leaves no journal line behind.
    pub fn preview_close_period_to(
        &self,
        id: PeriodId,
        to: &periods::CloseTo,
    ) -> Result<Option<EntryId>, LedgerError> {
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
        // Periods close in order: a later closed period already swept this
        // one's income, so closing it now would sweep it again.
        if let Some(later) = self
            .periods
            .values()
            .find(|p| p.status == PeriodStatus::Closed && p.end > period.end)
        {
            return Err(LedgerError::CloseOutOfOrder(id, later.id));
        }
        self.validate_close_to(to)?;
        if matches!(to, periods::CloseTo::RetainedEarnings { .. }) {
            self.check_controls(&[RETAINED_EARNINGS])?;
        }
        // An entry posts exactly when there is income to close or a sweep
        // account with a balance. Account creation (Retained Earnings) does
        // not consume entry ids.
        let (closings, _) = self.income_closings(period.end);
        let sweeps: Vec<AccountId> = match to {
            periods::CloseTo::RetainedEarnings { sweep }
            | periods::CloseTo::Account { sweep, .. } => sweep.clone(),
            periods::CloseTo::Members(members) | periods::CloseTo::MembersLastAbsorbs(members) => {
                members.iter().filter_map(|m| m.distributions).collect()
            }
        };
        let any = !closings.is_empty()
            || sweeps
                .iter()
                .any(|a| !self.movement_through(*a, period.end).is_zero());
        Ok(any.then_some(self.next_entry_id))
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
                form_1099: None,
                tin_on_file: false,
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

    /// Set which information return a vendor gets (or none), and whether
    /// their W-9 is on file.
    pub fn set_vendor_1099(
        &mut self,
        id: VendorId,
        form: Option<crate::form1099::Form1099>,
        tin_on_file: bool,
    ) -> Result<(), LedgerError> {
        let v = self
            .vendors
            .get_mut(&id)
            .ok_or(LedgerError::UnknownVendor(id))?;
        v.form_1099 = form;
        v.tin_on_file = tin_on_file;
        Ok(())
    }

    /// Tag an entry (draft or posted) with the vendor it paid or was billed
    /// by, or clear the tag. The tag is reporting metadata, like a receipt
    /// link: it changes no amounts, so posted entries may be tagged.
    pub fn set_entry_vendor(
        &mut self,
        entry: EntryId,
        vendor: Option<VendorId>,
    ) -> Result<(), LedgerError> {
        if !self.entries.contains_key(&entry) {
            return Err(LedgerError::UnknownEntry(entry));
        }
        match vendor {
            Some(v) => {
                if !self.vendors.contains_key(&v) {
                    return Err(LedgerError::UnknownVendor(v));
                }
                self.entry_vendors.insert(entry, v);
            }
            None => {
                self.entry_vendors.remove(&entry);
            }
        }
        Ok(())
    }

    /// The vendor an entry is tagged with.
    pub fn entry_vendor(&self, entry: EntryId) -> Option<VendorId> {
        self.entry_vendors.get(&entry).copied()
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

    /// Check the control accounts a command will post to by code, before
    /// anything changes: each is either missing (it will be created) or a
    /// postable account of the right type.
    ///
    /// ```text
    /// GOAL: never post a control role (receivables, payables, revenue,
    ///       payroll, retained earnings) to an account of the wrong kind.
    /// SOURCE: docs/spec/rules.md B-4 (the elements); DECISION (owner,
    ///         2026-10-13): refuse with a reason, never force the chart's
    ///         numbering.
    /// INVARIANTS:
    ///   1. MUST NOT mutate anything.
    ///   2. NEVER accepts an existing account of another type, or a
    ///      placeholder, for a control role.
    /// SCENARIOS:
    ///   1. IF the code is free: SHOULD accept (the command creates it).
    ///   2. IF the code holds a postable account of the right type: SHOULD
    ///      accept.
    ///   3. IF it holds another type or a placeholder:
    ///      - SHOULD refuse with ControlAccountMismatch, naming the code,
    ///        the account, its type, the role, and the type needed
    ///      - NEVER post to it, and NEVER rename or renumber it
    ///      BECAUSE imported charts use their own numbering; the human
    ///      decides.
    /// ```
    fn check_controls(&self, controls: &[ControlAccount]) -> Result<(), LedgerError> {
        for c in controls {
            if let Some(a) = self.chart.find_by_code(c.code)
                && (a.account_type != c.account_type || a.placeholder)
            {
                return Err(LedgerError::ControlAccountMismatch {
                    code: c.code.to_string(),
                    name: a.name.clone(),
                    role: c.role.to_string(),
                    expected: c.account_type,
                    found: a.account_type,
                    placeholder: a.placeholder,
                });
            }
        }
        Ok(())
    }

    /// Ensure a control account exists, creating it if missing. Callers
    /// run [`Self::check_controls`] first, so an existing account here is
    /// the right type.
    fn ensure_control(&mut self, c: &ControlAccount) -> AccountId {
        self.ensure_account(c.code, c.name, c.account_type)
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
        self.check_controls(&[RECEIVABLES, REVENUE])?;
        let ar = self.ensure_control(&RECEIVABLES);
        let revenue = self.ensure_control(&REVENUE);
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
        self.check_controls(&[GENERAL_EXPENSES, PAYABLES])?;
        let expense = self.ensure_control(&GENERAL_EXPENSES);
        let ap = self.ensure_control(&PAYABLES);
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
        self.entry_vendors.insert(entry, vendor_id);
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
        self.check_controls(&[RECEIVABLES])?;
        let ar = self.ensure_control(&RECEIVABLES);
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
        self.check_controls(&[PAYABLES])?;
        let ap = self.ensure_control(&PAYABLES);
        let entry = self.create_entry(date, &format!("Payment on bill {}", bill_id));
        let money = Money::new(amount, self.base_currency);
        self.add_line(entry, ap, Side::Debit, money)?;
        self.add_line(entry, cash_account, Side::Credit, money)?;
        self.post_entry(entry)?;
        let b = self.bills.get_mut(&bill_id).expect("bill exists");
        b.amount_paid += amount;
        b.status = arap::derive_status(b.amount_paid, b.total());
        self.entry_vendors.insert(entry, bill.vendor_id);
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
        self.entry_vendors.insert(entry_id, vendor_id);
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
        let bill = self
            .bills
            .get_mut(&bill_id)
            .ok_or(LedgerError::UnknownBill(bill_id))?;
        bill.amount_paid += amount;
        bill.status = arap::derive_status(bill.amount_paid, bill.total());
        let vendor = bill.vendor_id;
        self.entry_vendors.insert(entry_id, vendor);
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
        self.net_of(account, |_| true)
    }

    /// Balance expressed in the account's normal side: positive means a
    /// normal balance, negative means a contra balance.
    pub fn account_balance(&self, account: AccountId) -> Result<Decimal, LedgerError> {
        let account = self
            .chart
            .get(account)
            .ok_or(LedgerError::UnknownAccount(account))?;
        let movement = self.account_movement(account.id);
        let balance = kernel::normal_balance(account.account_type, movement)
            .expect("Decimal negation is exact");
        // Never leak negative zero into reports: it is a representation
        // artifact of negating an exact zero, and money must never print "-0".
        Ok(if balance.is_zero() {
            Decimal::ZERO
        } else {
            balance
        })
    }

    // ------------------------------------------------------------------
    // Payroll
    // ------------------------------------------------------------------

    /// Pure pre-validation for [`Self::run_payroll`]: every check that does
    /// not mutate, so journaling stores can validate before journaling.
    pub fn validate_payroll(
        &self,
        date: NaiveDate,
        lines: &[PayrollLine],
        cash_account: AccountId,
    ) -> Result<(), LedgerError> {
        validate_payroll_lines(lines)?;
        self.check_controls(&PAYROLL)?;
        self.chart
            .get(cash_account)
            .ok_or(LedgerError::UnknownAccount(cash_account))?;
        if let Some(pid) = self.closed_period_containing(date) {
            return Err(LedgerError::ClosedPeriod(pid, date.to_string()));
        }
        Ok(())
    }

    /// Resolve the seven payroll chart accounts by code, creating them when
    /// missing. Journaling stores ensure these through their own journaled
    /// path first; this then no-ops and replay stays deterministic.
    fn payroll_accounts(&mut self) -> PayrollAccounts {
        let [salary, tax, federal, state, ss, medicare, other] = PAYROLL;
        PayrollAccounts {
            salary_expense: self.ensure_control(&salary),
            payroll_tax_expense: self.ensure_control(&tax),
            federal_payable: self.ensure_control(&federal),
            state_payable: self.ensure_control(&state),
            ss_payable: self.ensure_control(&ss),
            medicare_payable: self.ensure_control(&medicare),
            other_payable: self.ensure_control(&other),
        }
    }

    /// Run payroll: validate, auto-create the payroll chart accounts, and
    /// post one balanced entry — Dr `6100` Salary & Wage Expense for gross,
    /// Dr `6110` Payroll Tax Expense for the employer FICA share, Cr the
    /// `2110`–`2150` payables for withholdings and deductions, Cr cash for
    /// net pay. Zero totals are skipped, never posted as zero-amount lines.
    ///
    /// Accounting only: there are no tax tables here. Every withholding
    /// number is supplied by the caller.
    pub fn run_payroll(
        &mut self,
        date: NaiveDate,
        lines: Vec<PayrollLine>,
        cash_account: AccountId,
    ) -> Result<PayrollRunId, LedgerError> {
        self.validate_payroll(date, &lines, cash_account)?;
        let accounts = self.payroll_accounts();
        let entry = self.try_create_entry(date, &format!("Payroll {date}"))?;
        for (account, side, amount) in
            payroll_entry_plan(&lines, accounts, cash_account, self.base_currency)
        {
            self.add_line(entry, account, side, amount)?;
        }
        self.post_entry(entry)?;
        let id = self.next_payroll_run_id;
        self.next_payroll_run_id += 1;
        self.payroll_runs.insert(
            id,
            PayrollRun {
                id,
                date,
                lines,
                entry_id: entry,
            },
        );
        Ok(id)
    }

    /// Current balances of the payroll tax payable accounts (`2110`–`2150`),
    /// in code order. Accounts that don't exist yet count as zero and are
    /// skipped, as are zero balances. Balances are normal-side: positive
    /// means owed.
    pub fn payroll_tax_payables(&self) -> Vec<(AccountId, Decimal)> {
        let mut out = Vec::new();
        for code in ["2110", "2120", "2130", "2140", "2150"] {
            let Some(account) = self.chart.find_by_code(code) else {
                continue;
            };
            // The code resolved, so the balance lookup cannot fail.
            let balance = self.account_balance(account.id).unwrap_or(Decimal::ZERO);
            if !balance.is_zero() {
                out.push((account.id, balance));
            }
        }
        out
    }

    /// Remit payroll taxes: pay the `2110`–`2150` payables out of cash.
    /// Posts one entry — Dr each payable for its balance, Cr cash for the
    /// total. Fails with [`LedgerError::NothingToRemit`] when nothing is
    /// owed, and rejects dates in closed periods.
    pub fn remit_payroll_taxes(
        &mut self,
        date: NaiveDate,
        cash_account: AccountId,
    ) -> Result<EntryId, LedgerError> {
        self.chart
            .get(cash_account)
            .ok_or(LedgerError::UnknownAccount(cash_account))?;
        self.check_controls(&PAYROLL)?;
        let payables = self.payroll_tax_payables();
        let total: Decimal = payables.iter().map(|(_, b)| *b).sum();
        if total.is_zero() {
            return Err(LedgerError::NothingToRemit);
        }
        if let Some(pid) = self.closed_period_containing(date) {
            return Err(LedgerError::ClosedPeriod(pid, date.to_string()));
        }
        let entry = self.try_create_entry(date, &format!("Payroll tax remittance {date}"))?;
        for (account, side, amount) in
            payroll_remit_plan(&payables, cash_account, self.base_currency)
        {
            self.add_line(entry, account, side, amount)?;
        }
        self.post_entry(entry)?;
        Ok(entry)
    }

    /// All payroll runs, in id order.
    pub fn list_payroll_runs(&self) -> Vec<&PayrollRun> {
        let mut ids: Vec<PayrollRunId> = self.payroll_runs.keys().copied().collect();
        ids.sort_unstable();
        ids.into_iter()
            .filter_map(|id| self.payroll_runs.get(&id))
            .collect()
    }

    /// Look up a payroll run.
    pub fn get_payroll_run(&self, id: PayrollRunId) -> Option<&PayrollRun> {
        self.payroll_runs.get(&id)
    }

    /// Record a payroll run without posting: replay support for journaling
    /// stores. The run's entry was already rebuilt from its own
    /// `entry.created` / `entry.line_added` / `entry.posted` events; this
    /// only restores the run record. The id must be the next payroll run
    /// id — anything else means the journal and the allocator disagree.
    #[doc(hidden)]
    pub fn replay_payroll_run(
        &mut self,
        id: PayrollRunId,
        date: NaiveDate,
        lines: Vec<PayrollLine>,
        entry_id: EntryId,
    ) -> Result<(), LedgerError> {
        if id != self.next_payroll_run_id {
            return Err(LedgerError::ReplayIdMismatch(id, self.next_payroll_run_id));
        }
        self.next_payroll_run_id += 1;
        self.payroll_runs.insert(
            id,
            PayrollRun {
                id,
                date,
                lines,
                entry_id,
            },
        );
        Ok(())
    }

    // ------------------------------------------------------------------
    // Classes
    // ------------------------------------------------------------------

    /// Add a class (division) for line-level P&L tracking. Returns its id.
    pub fn add_class(&mut self, name: &str) -> Result<ClassId, LedgerError> {
        if name.trim().is_empty() {
            return Err(LedgerError::EmptyClassName);
        }
        let id = self.next_class_id;
        self.next_class_id += 1;
        self.classes.insert(
            id,
            Class {
                id,
                name: name.to_string(),
            },
        );
        Ok(id)
    }

    /// Look up a class.
    pub fn get_class(&self, id: ClassId) -> Option<&Class> {
        self.classes.get(&id)
    }

    /// All classes, in id order.
    pub fn list_classes(&self) -> Vec<&Class> {
        let mut ids: Vec<ClassId> = self.classes.keys().copied().collect();
        ids.sort_unstable();
        ids.into_iter()
            .filter_map(|id| self.classes.get(&id))
            .collect()
    }

    /// Find a class by name, case-insensitive.
    pub fn find_class_by_name(&self, name: &str) -> Option<&Class> {
        self.classes
            .values()
            .find(|c| c.name.eq_ignore_ascii_case(name))
    }

    /// Tag a draft entry's line with a class, or clear it with `None`.
    /// Draft-only: posted entries are immutable, so their classes are
    /// frozen at posting time.
    pub fn set_line_class(
        &mut self,
        entry: EntryId,
        line_index: usize,
        class_id: Option<ClassId>,
    ) -> Result<(), LedgerError> {
        if let Some(cid) = class_id
            && !self.classes.contains_key(&cid)
        {
            return Err(LedgerError::UnknownClass(cid));
        }
        let e = self.draft_mut(entry)?;
        if line_index >= e.lines().len() {
            return Err(LedgerError::LineIndexOutOfRange(line_index, entry));
        }
        e.set_line_class(line_index, class_id);
        Ok(())
    }

    /// Record a class without allocating: replay support for journaling
    /// stores. The id must be the next class id — anything else means the
    /// journal and the allocator disagree.
    #[doc(hidden)]
    pub fn replay_class(&mut self, id: ClassId, name: String) -> Result<(), LedgerError> {
        if id != self.next_class_id {
            return Err(LedgerError::ReplayIdMismatch(
                id as u64,
                self.next_class_id as u64,
            ));
        }
        self.next_class_id += 1;
        self.classes.insert(id, Class { id, name });
        Ok(())
    }
}
