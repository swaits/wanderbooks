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
/// closes find (or create) these; a command refuses one of the wrong type
/// (`LedgerError::ControlAccountMismatch`).
#[derive(Clone, Copy, Debug)]
pub struct ControlAccount {
    /// The role's stable name, for assigning it an account
    /// ([`Ledger::assign_role`]): `receivables`, `payables`, ...
    pub key: &'static str,
    /// The code the engine looks for when the role has no account assigned.
    pub code: &'static str,
    /// The name it gives the account when it creates it.
    pub name: &'static str,
    /// What the account is for, in a refusal message.
    pub role: &'static str,
    /// The type the role needs.
    pub account_type: AccountType,
}

const fn control(
    key: &'static str,
    code: &'static str,
    name: &'static str,
    role: &'static str,
    account_type: AccountType,
) -> ControlAccount {
    ControlAccount {
        key,
        code,
        name,
        role,
        account_type,
    }
}

/// A control account; see [`ControlAccount`].
pub const RECEIVABLES: ControlAccount = control(
    "receivables",
    "1200",
    "Accounts Receivable",
    "accounts receivable",
    AccountType::Asset,
);
/// A control account; see [`ControlAccount`].
pub const REVENUE: ControlAccount = control(
    "revenue",
    "4000",
    "Revenue",
    "invoice revenue",
    AccountType::Revenue,
);
/// A control account; see [`ControlAccount`].
pub const PAYABLES: ControlAccount = control(
    "payables",
    "2000",
    "Accounts Payable",
    "accounts payable",
    AccountType::Liability,
);
/// A control account; see [`ControlAccount`].
pub const GENERAL_EXPENSES: ControlAccount = control(
    "general_expense",
    "6999",
    "General Expenses",
    "bill expense",
    AccountType::Expense,
);
/// A control account; see [`ControlAccount`].
pub const RETAINED_EARNINGS: ControlAccount = control(
    "retained_earnings",
    periods::RETAINED_EARNINGS_CODE,
    "Retained Earnings",
    "retained earnings",
    AccountType::Equity,
);
/// Every control account the engine posts to by code.
pub fn control_accounts() -> impl Iterator<Item = &'static ControlAccount> {
    [
        &RECEIVABLES,
        &REVENUE,
        &PAYABLES,
        &GENERAL_EXPENSES,
        &RETAINED_EARNINGS,
    ]
    .into_iter()
    .chain(PAYROLL.iter())
}

/// The control role named `key` (see [`ControlAccount::key`]).
pub fn control_account(key: &str) -> Option<&'static ControlAccount> {
    control_accounts().find(|c| c.key == key)
}

/// The payroll accounts, in [`PayrollAccounts`] order.
pub const PAYROLL: [ControlAccount; 7] = [
    control(
        "salary_expense",
        "6100",
        "Salary & Wage Expense",
        "salary expense",
        AccountType::Expense,
    ),
    control(
        "payroll_tax_expense",
        "6110",
        "Payroll Tax Expense",
        "payroll tax expense",
        AccountType::Expense,
    ),
    control(
        "federal_withholding",
        "2110",
        "Federal Withholding Payable",
        "federal withholding payable",
        AccountType::Liability,
    ),
    control(
        "state_withholding",
        "2120",
        "State Withholding Payable",
        "state withholding payable",
        AccountType::Liability,
    ),
    control(
        "social_security_payable",
        "2130",
        "SS Tax Payable",
        "social security tax payable",
        AccountType::Liability,
    ),
    control(
        "medicare_payable",
        "2140",
        "Medicare Tax Payable",
        "Medicare tax payable",
        AccountType::Liability,
    ),
    control(
        "other_deductions_payable",
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
#[derive(Clone, Debug, PartialEq)]
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
    /// Each invoice's payments (entry, amount), for aging as of a date.
    invoice_payments: HashMap<InvoiceId, Vec<(EntryId, Decimal)>>,
    /// Each bill's payments (entry, amount), for aging as of a date.
    bill_payments: HashMap<BillId, Vec<(EntryId, Decimal)>>,
    /// Control roles assigned an account ([`Ledger::assign_role`]), by
    /// [`ControlAccount::key`]. A role not here uses its code.
    roles: std::collections::BTreeMap<&'static str, AccountId>,
    /// Every asset tag ever held ([`assets::tag_key`] → the asset that
    /// held it): a tag never moves to another asset.
    asset_tags: std::collections::BTreeMap<String, AssetId>,
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
            invoice_payments: HashMap::new(),
            bill_payments: HashMap::new(),
            roles: std::collections::BTreeMap::new(),
            asset_tags: std::collections::BTreeMap::new(),
        }
    }

    // ------------------------------------------------------------------
    // Amortization schedules and recurring entries
    // ------------------------------------------------------------------

    /// Register an amortization schedule. Returns its id.
    pub fn register_schedule(&mut self, new: NewSchedule) -> Result<ScheduleId, LedgerError> {
        schedules::validate_schedule(&new, &self.chart)?;
        self.replay_register_schedule(new)
    }

    /// [`Self::register_schedule`] without the total's range and minor-unit
    /// checks added on 2026-10-13, for replaying journals written before
    /// them. Never for new work.
    pub fn replay_register_schedule(
        &mut self,
        new: NewSchedule,
    ) -> Result<ScheduleId, LedgerError> {
        schedules::validate_schedule_shape(&new, &self.chart)?;
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
        self.atomically(|l| l.post_schedule_month_steps(id, month_end))
    }

    /// The steps of [`Self::post_schedule_month`], run inside [`Self::atomically`].
    fn post_schedule_month_steps(
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
    ///
    /// ```text
    /// GOAL: turn one due occurrence of a recurring entry into a draft.
    /// SOURCE: docs/spec/rules.md B-12; Lean invariant Inv.draftsOpen.
    /// SCENARIOS:
    ///   1. IF the date falls in a closed period:
    ///      - SHOULD refuse (ClosedPeriod)
    ///      - NEVER create a draft there
    ///      BECAUSE a closed period takes no new entries, however they
    ///      are made (found by the store's refusal property, 2026-10-13).
    ///   2. IF the date is not an ungenerated occurrence: SHOULD refuse.
    ///   3. IF a line is refused: SHOULD leave the ledger as it was (no
    ///      half-built draft, no id spent).
    ///   4. Otherwise: SHOULD create a draft with the template's lines and
    ///      mark the occurrence generated.
    /// ```
    pub fn generate_recurring(
        &mut self,
        id: RecurringId,
        date: NaiveDate,
    ) -> Result<EntryId, LedgerError> {
        if let Some(pid) = self.closed_period_containing(date) {
            return Err(LedgerError::ClosedPeriod(pid, date.to_string()));
        }
        self.replay_generate_recurring(id, date)
    }

    /// [`Self::generate_recurring`] without the closed-period check, for
    /// replaying journals written before that check existed.
    pub fn replay_generate_recurring(
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
            if let Err(e) = self.add_line(entry, account, side, money) {
                self.entries.remove(&entry);
                self.next_entry_id = entry;
                return Err(e);
            }
        }
        self.recurring
            .get_mut(&id)
            .expect("checked")
            .generated
            .insert(date);
        Ok(entry)
    }

    /// Occurrences due through `through` that can't be generated because
    /// their date falls in a closed period, as (recurring entry, date).
    /// They need a human decision (book them in an open period by hand,
    /// or not at all).
    pub fn recurring_in_closed_periods(&self, through: NaiveDate) -> Vec<(RecurringId, NaiveDate)> {
        self.recurring
            .values()
            .flat_map(|r| r.due(through).into_iter().map(move |d| (r.id, d)))
            .filter(|(_, d)| self.closed_period_containing(*d).is_some())
            .collect()
    }

    /// The parts of the ledger that differ from `other`, by field name
    /// (empty when equal). For diagnosing a store whose journal replay and
    /// projection disagree.
    pub fn differs_from(&self, other: &Ledger) -> Vec<&'static str> {
        [
            ("base_currency", self.base_currency == other.base_currency),
            ("chart", self.chart == other.chart),
            ("entries", self.entries == other.entries),
            ("order", self.order == other.order),
            ("assets", self.assets == other.assets),
            ("periods", self.periods == other.periods),
            ("customers", self.customers == other.customers),
            ("vendors", self.vendors == other.vendors),
            ("invoices", self.invoices == other.invoices),
            ("bills", self.bills == other.bills),
            ("cleared", self.cleared == other.cleared),
            ("fx_rates", self.fx_rates == other.fx_rates),
            ("payroll_runs", self.payroll_runs == other.payroll_runs),
            ("classes", self.classes == other.classes),
            (
                "next_account_id",
                self.next_account_id == other.next_account_id,
            ),
            ("next_entry_id", self.next_entry_id == other.next_entry_id),
            ("next_asset_id", self.next_asset_id == other.next_asset_id),
            (
                "next_period_id",
                self.next_period_id == other.next_period_id,
            ),
            (
                "next_customer_id",
                self.next_customer_id == other.next_customer_id,
            ),
            (
                "next_vendor_id",
                self.next_vendor_id == other.next_vendor_id,
            ),
            (
                "next_invoice_id",
                self.next_invoice_id == other.next_invoice_id,
            ),
            ("next_bill_id", self.next_bill_id == other.next_bill_id),
            (
                "next_payroll_run_id",
                self.next_payroll_run_id == other.next_payroll_run_id,
            ),
            ("next_class_id", self.next_class_id == other.next_class_id),
            ("schedules", self.schedules == other.schedules),
            (
                "next_schedule_id",
                self.next_schedule_id == other.next_schedule_id,
            ),
            ("recurring", self.recurring == other.recurring),
            (
                "next_recurring_id",
                self.next_recurring_id == other.next_recurring_id,
            ),
            ("entry_vendors", self.entry_vendors == other.entry_vendors),
            (
                "invoice_payments",
                self.invoice_payments == other.invoice_payments,
            ),
            ("bill_payments", self.bill_payments == other.bill_payments),
            ("roles", self.roles == other.roles),
            ("asset_tags", self.asset_tags == other.asset_tags),
        ]
        .into_iter()
        .filter(|(_, same)| !same)
        .map(|(name, _)| name)
        .collect()
    }

    /// Run a command made of several steps as one: if any step is
    /// refused, the ledger is exactly as it was (no half-built draft, no
    /// id spent). Found 2026-10-13: a schedule month whose amount was out
    /// of range left a draft behind.
    fn atomically<T>(
        &mut self,
        steps: impl FnOnce(&mut Self) -> Result<T, LedgerError>,
    ) -> Result<T, LedgerError> {
        let before = self.clone();
        steps(self).inspect_err(|_| *self = before)
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
    ///
    /// ```text
    /// GOAL: register an account under a unique code.
    /// SOURCE: rules.md B-4 (the five elements); R-4; DECISION (Stephen,
    ///         2026-10-13): a child has its parent's type.
    /// SCENARIOS:
    ///   1. IF the code is empty or taken: SHOULD refuse.
    ///   2. IF the parent doesn't exist: SHOULD refuse.
    ///   3. IF the parent is another type:
    ///      - SHOULD refuse with ParentTypeMismatch
    ///      - NEVER add the account
    ///      BECAUSE a child subdivides its parent.
    /// ```
    ///
    /// Replay of journals written before the parent rule uses
    /// [`Self::replay_add_account`].
    pub fn add_account(
        &mut self,
        code: &str,
        name: &str,
        account_type: AccountType,
        parent: Option<AccountId>,
        placeholder: bool,
    ) -> Result<AccountId, LedgerError> {
        // A chart lists each account by number and title; neither may be
        // blank (2026-10-13; the core refused only an empty code, so a code
        // of spaces or an unnamed account got in).
        if code.trim().is_empty() {
            return Err(LedgerError::EmptyAccountCode);
        }
        if name.trim().is_empty() {
            return Err(LedgerError::EmptyAccountName);
        }
        if let Some(p) = parent.and_then(|p| self.chart.get(p))
            && p.account_type != account_type
        {
            return Err(LedgerError::ParentTypeMismatch {
                code: code.to_string(),
                parent_code: p.code.clone(),
                expected: p.account_type,
                found: account_type,
            });
        }
        self.replay_add_account(code, name, account_type, parent, placeholder)
    }

    /// [`Self::add_account`] without the rules added on 2026-10-13 (a
    /// child's type; a blank code or name), for replaying journals written before them. Never
    /// for new work.
    pub fn replay_add_account(
        &mut self,
        code: &str,
        name: &str,
        account_type: AccountType,
        parent: Option<AccountId>,
        placeholder: bool,
    ) -> Result<AccountId, LedgerError> {
        // The id is used only if the account is: a refusal uses up nothing.
        let id = self.next_account_id;
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
        self.next_account_id += 1;
        Ok(id)
    }

    /// Give an account a new number (code).
    ///
    /// ```text
    /// GOAL: let the human renumber an account (say, off a code the engine
    ///       uses for a control role) without touching its history.
    /// SOURCE: DECISION (Stephen, 2026-10-14: "fix the account number");
    ///         control accounts: Ledger::check_controls.
    /// INVARIANTS:
    ///   1. Only the code changes: the account's id, name, type, parent,
    ///      tax category, and every entry posted to it stay as they were
    ///      (entries name accounts by id), so every balance and report
    ///      total is unchanged.
    ///   2. A refusal MUST leave the books unchanged.
    /// SCENARIOS:
    ///   1. IF the account is unknown, or the code is blank or already in
    ///      use (its own included): SHOULD refuse.
    ///   2. IF the code is a control account's and the account is of
    ///      another type, or a placeholder: SHOULD refuse with
    ///      ControlAccountMismatch, as a control command would.
    /// ```
    pub fn renumber_account(&mut self, account: AccountId, code: &str) -> Result<(), LedgerError> {
        self.check_renumber_account(account, code)?;
        self.chart.set_code(account, code)
    }

    /// The validation behind [`Self::renumber_account`], for stores that
    /// journal before mutating.
    pub fn check_renumber_account(
        &self,
        account: AccountId,
        code: &str,
    ) -> Result<(), LedgerError> {
        let a = self
            .chart
            .get(account)
            .ok_or(LedgerError::UnknownAccount(account))?;
        if code.trim().is_empty() {
            return Err(LedgerError::EmptyAccountCode);
        }
        if self.chart.find_by_code(code).is_some() {
            return Err(LedgerError::DuplicateAccountCode(code.to_string()));
        }
        if let Some(c) =
            control_accounts().find(|c| c.code == code && !self.roles.contains_key(c.key))
            && (a.account_type != c.account_type || a.placeholder)
        {
            return Err(LedgerError::ControlAccountMismatch {
                code: code.to_string(),
                name: a.name.clone(),
                role: c.role.to_string(),
                expected: c.account_type,
                found: a.account_type,
                placeholder: a.placeholder,
            });
        }
        Ok(())
    }

    /// The account that fills a control role now: the one assigned to it,
    /// else the account at the role's code (`None` when there is none yet;
    /// the first command that needs it creates it).
    pub fn role_account(&self, c: &ControlAccount) -> Option<AccountId> {
        match self.roles.get(c.key) {
            Some(&id) => Some(id),
            None => self.chart.find_by_code(c.code).map(|a| a.id),
        }
    }

    /// The account assigned to a control role, if any (`None`: the role
    /// uses its code).
    pub fn role_assignment(&self, c: &ControlAccount) -> Option<AccountId> {
        self.roles.get(c.key).copied()
    }

    /// Assign a control role an account of the books' own numbering, or
    /// (`None`) return it to its code. Invoices, bills, payroll, and closes
    /// then post the role there.
    ///
    /// ```text
    /// GOAL: let a chart keep its own numbering: the engine's roles go to
    ///       the accounts the human picks, not only to fixed codes.
    /// SOURCE: docs/spec/rules.md B-4 (the elements), B-23 (control roles);
    ///         DECISION (Stephen, 2026-10-15): roles are journaled, not
    ///         configured, so replay never depends on a file.
    /// INVARIANTS:
    ///   1. After an assignment, every command that posts the role posts it
    ///      to that account; after `None`, to the account at the role's
    ///      code (created when first needed, as before).
    ///   2. No account fills two roles.
    ///   3. A role's account is always postable and of the role's type.
    ///   4. Posted entries never change: an assignment only steers what
    ///      later commands post.
    ///   5. A refusal MUST leave the books unchanged.
    /// SCENARIOS:
    ///   1. IF the role is unknown, or the account is unknown: SHOULD refuse.
    ///   2. IF the account (or, for `None`, the account at the role's
    ///      code) is of another type or a placeholder: SHOULD refuse with
    ///      ControlAccountMismatch.
    ///   3. IF that account already fills another role: SHOULD refuse
    ///      with RoleTaken.
    ///   4. IF the role is a balance sheet one (an asset, liability, or
    ///      equity role) and the account filling it now holds a balance,
    ///      and the role would move: SHOULD refuse with RoleHoldsBalance
    ///      BECAUSE the control account must keep agreeing with what it
    ///      controls (open invoices, bills, taxes withheld); move the
    ///      balance with an entry first. Revenue and expense roles move
    ///      freely.
    ///   5. IF the role already resolves to that account: SHOULD accept
    ///      (the assignment is recorded).
    /// ```
    pub fn assign_role(
        &mut self,
        key: &str,
        account: Option<AccountId>,
    ) -> Result<(), LedgerError> {
        let c = self.check_assign_role(key, account)?;
        match account {
            Some(id) => self.roles.insert(c.key, id),
            None => self.roles.remove(c.key),
        };
        Ok(())
    }

    /// The validation behind [`Self::assign_role`], for stores that journal
    /// before mutating. Returns the role.
    pub fn check_assign_role(
        &self,
        key: &str,
        account: Option<AccountId>,
    ) -> Result<&'static ControlAccount, LedgerError> {
        let c = control_account(key).ok_or_else(|| LedgerError::UnknownRole(key.to_string()))?;
        let target = match account {
            Some(id) => Some(self.chart.get(id).ok_or(LedgerError::UnknownAccount(id))?),
            None => self.chart.find_by_code(c.code),
        };
        if let Some(a) = target {
            if a.account_type != c.account_type || a.placeholder {
                return Err(LedgerError::ControlAccountMismatch {
                    code: a.code.clone(),
                    name: a.name.clone(),
                    role: c.role.to_string(),
                    expected: c.account_type,
                    found: a.account_type,
                    placeholder: a.placeholder,
                });
            }
            if let Some(other) =
                control_accounts().find(|o| o.key != c.key && self.role_account(o) == Some(a.id))
            {
                return Err(LedgerError::RoleTaken {
                    code: a.code.clone(),
                    name: a.name.clone(),
                    role: other.role.to_string(),
                });
            }
        }
        let now = self.role_account(c);
        let balance_sheet = !matches!(c.account_type, AccountType::Revenue | AccountType::Expense);
        if let Some(current) = now
            && balance_sheet
            && now != target.map(|a| a.id)
        {
            let balance = self.account_balance(current).unwrap_or(Decimal::ZERO);
            if !balance.is_zero() {
                let a = self.chart.get(current).expect("a role's account exists");
                return Err(LedgerError::RoleHoldsBalance {
                    role: c.role.to_string(),
                    code: a.code.clone(),
                    name: a.name.clone(),
                    balance,
                });
            }
        }
        Ok(c)
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
    ///
    /// ```text
    /// GOAL: start a draft entry for new work.
    /// SOURCE: docs/spec/rules.md B-12; Lean invariant Inv.draftsOpen.
    /// SCENARIOS:
    ///   1. IF the date falls in a closed period:
    ///      - SHOULD refuse (ClosedPeriod)
    ///      - NEVER create a draft there
    ///      BECAUSE a closed period takes no new entries.
    ///   2. Otherwise: SHOULD return a fresh id, one past the last.
    /// ```
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
        self.check_line_account(account)?;
        check_minor_units(amount)?;
        self.replay_add_line(entry, account, side, amount)
    }

    /// A line's account must exist and take postings, checked when the
    /// line is added (Stephen, 2026-10-13); posting checks again.
    fn check_line_account(&self, account: AccountId) -> Result<(), LedgerError> {
        let a = self
            .chart
            .get(account)
            .ok_or(LedgerError::UnknownAccount(account))?;
        if a.placeholder {
            return Err(LedgerError::PlaceholderAccount(a.code.clone()));
        }
        Ok(())
    }

    /// [`Self::add_line`] without the account check added on 2026-10-13,
    /// for replaying journals written before it. Never for new work.
    pub fn replay_add_line(
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
    /// amount in `foreign_currency`; `fx_rate` converts foreign -> base.
    /// The stored `amount` is foreign * rate rounded to the base currency's
    /// minor unit (cents for USD), half away from zero; the foreign
    /// original is kept for reference. Rates are exact Decimals, never
    /// floats.
    ///
    /// ```text
    /// GOAL: book a foreign-currency amount in the ledger's one money.
    /// SOURCE: docs/spec/rules.md B-3 (Pacioli ch. 36: one money; ASC
    ///         830-20-30-1: at the transaction-date rate); minor units: ISO
    ///         4217; rounding: DECISION (Stephen, 2026-10-13).
    /// INVARIANTS:
    ///   1. The base amount MUST be a whole number of the base currency's
    ///      smallest unit, within half a unit of foreign * rate.
    ///   2. NEVER convert into the base currency itself.
    /// SCENARIOS:
    ///   1. IF 100.00 EUR at 1.0834567 into USD: SHOULD book 108.35.
    ///   2. IF 10.00 USD at 151.237 into JPY: SHOULD book 1512.
    ///   3. IF the rounded amount is zero: SHOULD refuse.
    /// ```
    pub fn add_line_fx(
        &mut self,
        entry: EntryId,
        account: AccountId,
        side: Side,
        foreign_currency: Currency,
        foreign_amount: Decimal,
        fx_rate: Decimal,
    ) -> Result<(), LedgerError> {
        self.check_line_account(account)?;
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
        let exact = foreign_amount
            .checked_mul(fx_rate)
            .ok_or(LedgerError::AmountOutOfRange)?;
        // Booked in the base currency's smallest unit (cents for USD), half
        // away from zero: a ledger in one money holds no fractions of its
        // smallest unit (rules.md B-3; DECISION, Stephen 2026-10-13, after
        // the GnuCash cross-check found the difference).
        let base_amount = match self.base_currency.minor_units() {
            Some(dp) => exact
                .round_dp_with_strategy(dp, rust_decimal::RoundingStrategy::MidpointAwayFromZero),
            None => exact,
        };
        if base_amount <= Decimal::ZERO {
            return Err(LedgerError::InvalidFxRate(fx_rate));
        }
        self.replay_add_line_fx(
            entry,
            account,
            side,
            Money::new(base_amount, self.base_currency),
            foreign_currency,
            foreign_amount,
        )
    }

    /// A foreign-currency line exactly as a journal recorded it: the base
    /// amount as booked (rounded or not, by the rule of its day) and the
    /// foreign original. For replay only; new work uses
    /// [`Self::add_line_fx`].
    pub fn replay_add_line_fx(
        &mut self,
        entry: EntryId,
        account: AccountId,
        side: Side,
        amount: Money,
        foreign_currency: Currency,
        foreign_amount: Decimal,
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
            foreign_currency: Some(foreign_currency),
            foreign_amount: Some(foreign_amount),
            class_id: None,
        });
        Ok(())
    }

    /// Validate and post a draft entry. One-way: a posted entry is immutable.
    ///
    /// Rejects entries dated in a closed period.
    ///
    /// ```text
    /// GOAL: make a draft part of the books, once and for good.
    /// SOURCE: docs/spec/rules.md B-1, B-2, B-3, B-8 (Pacioli ch. 36: write
    ///         only forward), B-12.
    /// INVARIANTS:
    ///   1. MUST post only an entry that passes JournalEntry::validate.
    ///   2. MUST append it to the log; NEVER reorder, change, or remove an
    ///      earlier posted entry.
    ///   3. A refusal MUST leave the books unchanged.
    /// SCENARIOS:
    ///   1. IF the entry is unknown: SHOULD refuse (UnknownEntry).
    ///   2. IF it is already posted:
    ///      - SHOULD refuse (EntryAlreadyPosted)
    ///      - NEVER post it twice
    ///   3. IF it is dated in a closed period: SHOULD refuse (ClosedPeriod).
    ///   4. IF it doesn't balance, has fewer than two lines, a zero, negative,
    ///      or out-of-range amount, or an unknown or placeholder account:
    ///      SHOULD refuse with that reason.
    ///   5. IF a line is finer than its currency's minor unit (a fraction
    ///      of a cent): SHOULD refuse (FinerThanMinorUnit)
    ///      BECAUSE a ledger in one money holds no fractions of its
    ///      smallest unit (B-3); found when a payroll run posted $65.27794
    ///      (the store's refusal property in CI, 2026-10-13).
    /// ```
    pub fn post_entry(&mut self, entry: EntryId) -> Result<(), LedgerError> {
        if let Some(e) = self.entries.get(&entry) {
            for line in e.lines() {
                check_minor_units(line.amount)?;
            }
        }
        self.replay_post_entry(entry)
    }

    /// [`Self::post_entry`] without the minor-unit check added on
    /// 2026-10-13, for replaying journals written before it. Never for new
    /// work.
    pub fn replay_post_entry(&mut self, entry: EntryId) -> Result<(), LedgerError> {
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
    ///         Stephen 2026-10-13), B-12 (closed periods).
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
    ///      (Stephen, 2026-10-13). Reversing the reversal re-instates it.
    ///   3. IF the entry closes a period:
    ///      - SHOULD refuse with `ClosingEntryIrreversible`
    ///      BECAUSE the period would stay closed with its earnings
    ///      unswept (Stephen, 2026-10-13).
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
        self.atomically(|l| l.reverse_entry_steps(entry, date, description))
    }

    /// The steps of [`Self::reverse_entry`], run inside [`Self::atomically`].
    fn reverse_entry_steps(
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
        // Every depreciation and disposal line derives from the cost: one
        // that no line could carry is refused here, not months later.
        if new.cost.amount > kernel::MAX_LINE_AMOUNT {
            return Err(LedgerError::AmountOutOfRange);
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
    ///
    /// ```text
    /// GOAL: put a fixed asset on the register, to depreciate monthly.
    /// SOURCE: docs/spec/rules.md B-15; B-22 (capitalize vs expense).
    /// SCENARIOS:
    ///   1. IF the description is blank, cost isn't positive, salvage is
    ///      negative or not below cost, the useful life is zero months, or the
    ///      accounts aren't the right types: SHOULD refuse.
    /// ```
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
                tag: None,
                serial: None,
                acquired_by: None,
                tax: None,
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

    /// Give an asset a tag (or a new one): the label on the thing itself.
    ///
    /// ```text
    /// GOAL: find an asset in the books from the label on it, and the
    ///       label from the books.
    /// SOURCE: DECISION (Stephen, 2026-10-15).
    /// INVARIANTS:
    ///   1. Tags are unique among all assets, ignoring case.
    ///   2. A tag is never reused by another asset, even after the asset
    ///      that held it was retagged or disposed: an old label never
    ///      points at the wrong thing. The same asset may take back a tag
    ///      it held.
    ///   3. A refusal MUST leave the books unchanged.
    /// SCENARIOS:
    ///   1. IF the asset is unknown: SHOULD refuse.
    ///   2. IF the tag's form is wrong (assets::check_asset_tag): SHOULD
    ///      refuse with InvalidAssetTag.
    ///   3. IF another asset holds or held the tag: SHOULD refuse with
    ///      AssetTagTaken.
    ///   4. IF the asset already has that tag: SHOULD accept (no change).
    /// ```
    pub fn set_asset_tag(&mut self, id: AssetId, tag: &str) -> Result<(), LedgerError> {
        self.check_asset_tag_for(Some(id), tag)?;
        self.asset_tags.insert(assets::tag_key(tag), id);
        self.assets.get_mut(&id).expect("checked").tag = Some(tag.to_string());
        Ok(())
    }

    /// Whether `tag` can go on asset `id` (`None`: on an asset not yet
    /// registered), for stores that journal before mutating.
    pub fn check_asset_tag_for(&self, id: Option<AssetId>, tag: &str) -> Result<(), LedgerError> {
        if let Some(id) = id
            && !self.assets.contains_key(&id)
        {
            return Err(LedgerError::UnknownAsset(id));
        }
        assets::check_asset_tag(tag).map_err(|why| LedgerError::InvalidAssetTag {
            tag: tag.to_string(),
            why,
        })?;
        match self.asset_tags.get(&assets::tag_key(tag)) {
            Some(&holder) if Some(holder) != id => Err(LedgerError::AssetTagTaken {
                tag: tag.to_string(),
                asset: holder,
            }),
            _ => Ok(()),
        }
    }

    /// Record (or, with `None`, clear) an asset's serial number.
    ///
    /// ```text
    /// GOAL: keep the manufacturer's serial with the asset, for warranty,
    ///       insurance, and proving which thing the books mean.
    /// SOURCE: DECISION (Stephen, 2026-10-15).
    /// SCENARIOS:
    ///   1. IF the asset is unknown, or the serial's form is wrong
    ///      (assets::check_serial): SHOULD refuse, changing nothing.
    /// ```
    pub fn set_asset_serial(
        &mut self,
        id: AssetId,
        serial: Option<&str>,
    ) -> Result<(), LedgerError> {
        self.check_asset_serial(id, serial)?;
        self.assets.get_mut(&id).expect("checked").serial = serial.map(str::to_string);
        Ok(())
    }

    /// The validation behind [`Self::set_asset_serial`].
    pub fn check_asset_serial(&self, id: AssetId, serial: Option<&str>) -> Result<(), LedgerError> {
        if !self.assets.contains_key(&id) {
            return Err(LedgerError::UnknownAsset(id));
        }
        if let Some(s) = serial {
            assets::check_serial(s).map_err(LedgerError::InvalidSerial)?;
        }
        Ok(())
    }

    /// Link an asset to the posted entry that put it on the books: its
    /// purchase, or a member's contribution of it.
    ///
    /// ```text
    /// GOAL: from an asset, its source document and capital entry; from
    ///       the entry, the asset.
    /// SOURCE: DECISION (Stephen, 2026-10-15).
    /// INVARIANTS:
    ///   1. The linked entry is posted, is not a reversal, has not been
    ///      reversed, and debits the asset's cost account.
    ///   2. A refusal MUST leave the books unchanged.
    /// SCENARIOS:
    ///   1. IF the asset or entry is unknown, or the entry fails 1: SHOULD
    ///      refuse with AcquisitionEntryMismatch (or UnknownAsset /
    ///      UnknownEntry).
    ///   2. IF a later correction reverses the linked entry: link the
    ///      replacement (a reversed link stays as history in the journal).
    /// ```
    pub fn set_asset_acquisition(
        &mut self,
        id: AssetId,
        entry: EntryId,
    ) -> Result<(), LedgerError> {
        self.check_asset_acquisition(id, entry)?;
        self.assets.get_mut(&id).expect("checked").acquired_by = Some(entry);
        Ok(())
    }

    /// The validation behind [`Self::set_asset_acquisition`].
    pub fn check_asset_acquisition(&self, id: AssetId, entry: EntryId) -> Result<(), LedgerError> {
        let asset = self.assets.get(&id).ok_or(LedgerError::UnknownAsset(id))?;
        let e = self
            .entries
            .get(&entry)
            .ok_or(LedgerError::UnknownEntry(entry))?;
        let why = if e.status() != crate::EntryStatus::Posted {
            Some("it isn't posted")
        } else if e.reversal_of().is_some() {
            Some("it is a reversal")
        } else if self
            .entries
            .values()
            .any(|x| x.reversal_of() == Some(entry))
        {
            Some("it has been reversed")
        } else if !e
            .lines()
            .iter()
            .any(|l| l.account == asset.cost_account && l.side == Side::Debit)
        {
            Some("it doesn't debit the asset's cost account")
        } else {
            None
        };
        match why {
            Some(why) => Err(LedgerError::AcquisitionEntryMismatch {
                asset: id,
                entry,
                why: why.to_string(),
            }),
            None => Ok(()),
        }
    }

    /// Change an asset's description (what it is, as the register and its
    /// label print it). Its accounting doesn't change.
    ///
    /// ```text
    /// GOAL: keep the register's words right when they go stale (a
    ///       corrected entry, a better name).
    /// SOURCE: DECISION (Stephen, 2026-10-15).
    /// SCENARIOS:
    ///   1. IF the asset is unknown, or the description is blank: SHOULD
    ///      refuse, changing nothing.
    /// ```
    pub fn set_asset_description(
        &mut self,
        id: AssetId,
        description: &str,
    ) -> Result<(), LedgerError> {
        self.check_asset_description(id, description)?;
        self.assets.get_mut(&id).expect("checked").description = description.to_string();
        Ok(())
    }

    /// The validation behind [`Self::set_asset_description`].
    pub fn check_asset_description(
        &self,
        id: AssetId,
        description: &str,
    ) -> Result<(), LedgerError> {
        if !self.assets.contains_key(&id) {
            return Err(LedgerError::UnknownAsset(id));
        }
        if description.trim().is_empty() {
            return Err(LedgerError::EmptyAssetDescription);
        }
        Ok(())
    }

    /// Record an asset's federal tax basis: the rule, and for a member's
    /// contribution the member's adjusted basis. Book figures don't change.
    ///
    /// ```text
    /// GOAL: the register carries tax basis beside book value, so book and
    ///       tax never get confused and the CPA sees both.
    /// SOURCE: docs/spec/rules.md B-25; docs/spec/sources/
    ///         irs-partnership-basis.md.
    /// INVARIANTS:
    ///   1. Cost takes no contributor basis; Carryover and
    ///      PersonalUseConversion require one.
    ///   2. A contributor basis is non-negative, in cents at most.
    ///   3. A refusal MUST leave the books unchanged.
    /// SCENARIOS:
    ///   1. IF the asset is unknown, or 1 or 2 fails: SHOULD refuse with
    ///      UnknownAsset or InvalidTaxBasis.
    ///   2. IF set again: the new record replaces the old (the journal
    ///      keeps both).
    /// ```
    pub fn set_asset_tax_basis(
        &mut self,
        id: AssetId,
        tax: assets::TaxBasis,
    ) -> Result<(), LedgerError> {
        self.check_asset_tax_basis(id, tax)?;
        self.assets.get_mut(&id).expect("checked").tax = Some(tax);
        Ok(())
    }

    /// The validation behind [`Self::set_asset_tax_basis`].
    pub fn check_asset_tax_basis(
        &self,
        id: AssetId,
        tax: assets::TaxBasis,
    ) -> Result<(), LedgerError> {
        if !self.assets.contains_key(&id) {
            return Err(LedgerError::UnknownAsset(id));
        }
        tax.check().map_err(LedgerError::InvalidTaxBasis)
    }

    /// Correct a registered asset's economics (cost, salvage, useful life,
    /// in-service date) when it was registered wrong. Its accounts, tag,
    /// serial, links, and tax basis record stay.
    ///
    /// ```text
    /// GOAL: fix a registration mistake without re-registering (which would
    ///       burn its tag, since tags are never reused).
    /// SOURCE: DECISION (Stephen, 2026-10-15).
    /// INVARIANTS:
    ///   1. The corrected figures pass every check a new registration does
    ///      (validate_new_asset).
    ///   2. Only an active asset is corrected; a store also refuses once any
    ///      depreciation for it is posted (the posted entries would disagree
    ///      with the schedule; correct with entries instead).
    ///   3. A refusal MUST leave the books unchanged.
    /// ```
    pub fn correct_asset(
        &mut self,
        id: AssetId,
        fix: assets::AssetCorrection,
    ) -> Result<(), LedgerError> {
        self.check_correct_asset(id, &fix)?;
        let a = self.assets.get_mut(&id).expect("checked");
        a.cost = fix.cost;
        a.salvage_value = fix.salvage_value;
        a.useful_life_months = fix.useful_life_months;
        a.purchase_date = fix.purchase_date;
        Ok(())
    }

    /// The validation behind [`Self::correct_asset`].
    pub fn check_correct_asset(
        &self,
        id: AssetId,
        fix: &assets::AssetCorrection,
    ) -> Result<(), LedgerError> {
        let a = self.assets.get(&id).ok_or(LedgerError::UnknownAsset(id))?;
        if a.status != AssetStatus::Active {
            return Err(LedgerError::AssetAlreadyDisposed(id));
        }
        self.validate_new_asset(&NewAsset {
            description: a.description.clone(),
            purchase_date: fix.purchase_date,
            cost: fix.cost,
            salvage_value: fix.salvage_value,
            useful_life_months: fix.useful_life_months,
            cost_account: a.cost_account,
            accum_depr_account: a.accum_depr_account,
            expense_account: a.expense_account,
        })
    }

    /// The asset whose current tag is `tag`, ignoring case.
    pub fn asset_by_tag(&self, tag: &str) -> Option<&Asset> {
        let id = self.asset_tags.get(&assets::tag_key(tag))?;
        self.assets
            .get(id)
            .filter(|a| a.tag.as_deref().map(assets::tag_key) == Some(assets::tag_key(tag)))
    }

    /// The first free generated tag, `PREFIX-0001`, `PREFIX-0002`, ...:
    /// the lowest number no asset holds or held under that prefix.
    /// Numbers past 9999 widen.
    pub fn next_asset_tag(&self, prefix: &str) -> String {
        (1u64..)
            .map(|n| format!("{prefix}-{n:04}"))
            .find(|t| !self.asset_tags.contains_key(&assets::tag_key(t)))
            .expect("some number is free")
    }

    /// Mark an asset disposed. The disposal *entry* is posted through the
    /// normal machinery by the caller; this records the lifecycle change.
    ///
    /// ```text
    /// GOAL: take an asset off the books when sold or scrapped.
    /// SOURCE: docs/spec/rules.md B-15; CONVENTION (gain or loss on disposal).
    /// INVARIANTS:
    ///   1. The gain (or loss) MUST equal proceeds minus book value at the
    ///      disposal date.
    ///   2. The asset's cost and accumulated depreciation MUST both leave the
    ///      books.
    /// SCENARIOS:
    ///   1. IF the asset is unknown or already disposed: SHOULD refuse.
    /// ```
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
    ///
    /// ```text
    /// GOAL: name a span of days that can later be closed.
    /// SOURCE: rules.md B-12 (IRS Pub. 538: tax years, calendar or fiscal);
    ///         DECISION (Stephen, 2026-10-13): periods nest, never
    ///         partially overlap.
    /// SCENARIOS:
    ///   1. IF end is before start: SHOULD refuse.
    ///   2. IF it lies inside, or around, every period it touches: SHOULD
    ///      accept (a month in a quarter in a year).
    ///   3. IF it partially overlaps a period:
    ///      - SHOULD refuse with PeriodsOverlap
    ///      - NEVER create it, or use up an id
    ///      BECAUSE a span split across two periods could close twice or
    ///      never.
    /// ```
    pub fn create_period(
        &mut self,
        name: &str,
        start: NaiveDate,
        end: NaiveDate,
    ) -> Result<PeriodId, LedgerError> {
        periods::validate_period_range(start, end)?;
        let nested = |a: (NaiveDate, NaiveDate), b: (NaiveDate, NaiveDate)| {
            (a.0 >= b.0 && a.1 <= b.1) || (b.0 >= a.0 && b.1 <= a.1)
        };
        for q in self.list_periods() {
            let touches = start <= q.end && q.start <= end;
            if touches && !nested((start, end), (q.start, q.end)) {
                return Err(LedgerError::PeriodsOverlap {
                    name: name.to_string(),
                    start,
                    end,
                    other: q.name.clone(),
                    other_start: q.start,
                    other_end: q.end,
                });
            }
        }
        self.replay_create_period(name, start, end)
    }

    /// [`Self::create_period`] without the nesting rule added on
    /// 2026-10-13, for replaying journals written before it.
    pub fn replay_create_period(
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
                close_seq: None,
                member_close: None,
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
    ///      the net, to the cent (odd cents by largest remainder; closes
    ///      journaled before 2026-10-13 replay with the last member
    ///      absorbing them), and their own distribution account.
    /// 3. Marks the period closed.
    ///
    /// Returns the closing entry's id, or `None` if there was nothing to
    /// close.
    ///
    /// ```text
    /// GOAL: close a period: zero every revenue and expense account's balance
    ///       as of its last day into equity, and lock it.
    /// SOURCE: docs/spec/rules.md B-11 (Pacioli ch. 27, 34), B-12, B-13 (26
    ///         U.S.C. §704(a)); R-? Periods and close.
    /// INVARIANTS:
    ///   1. After a close, every revenue and expense account MUST have a zero
    ///      balance as of the period's last day.
    ///   2. The closing entry MUST balance, be dated the period's last day, and
    ///      credit equity with exactly the net income through that day (split
    ///      among members by share, to the cent, by largest remainder).
    ///   3. Entries dated after the period MUST stay untouched (their income
    ///      belongs to their own period).
    ///   4. A sweep account (draws, distributions, dividends) MUST end at zero,
    ///      its balance moved into its target.
    ///   5. NEVER post a closing entry when there is nothing to close.
    /// SCENARIOS:
    ///   1. IF the period is unknown or already closed: SHOULD refuse.
    ///   2. IF a draft is dated inside it: SHOULD refuse (DraftsInPeriod).
    ///   3. IF a later period is already closed: SHOULD refuse (CloseOutOfOrder).
    ///   4. IF a target or sweep account isn't a distinct equity account, or
    ///      member shares aren't positive and summing to 1: SHOULD refuse.
    ///   5. IF closing to retained earnings and 3999 exists but isn't a
    ///      postable equity account: SHOULD refuse (ControlAccountMismatch).
    /// ```
    pub fn close_period_to(
        &mut self,
        id: PeriodId,
        to: &periods::CloseTo,
    ) -> Result<Option<EntryId>, LedgerError> {
        self.atomically(|l| l.close_period_to_steps(id, to))
    }

    /// The steps of [`Self::close_period_to`], run inside [`Self::atomically`].
    fn close_period_to_steps(
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
            periods::CloseTo::Members(members)
            | periods::CloseTo::MembersPerClose(members)
            | periods::CloseTo::MembersLastAbsorbs(members) => {
                let shares: Vec<Decimal> = members.iter().map(|m| m.share).collect();
                let parts = match to {
                    periods::CloseTo::MembersLastAbsorbs(_) => {
                        periods::allocate_last_absorbs(net, &shares)
                    }
                    periods::CloseTo::MembersPerClose(_) => periods::allocate(net, &shares),
                    _ => {
                        // Cumulative: what the run gives each member with
                        // this close, less what it gave them before.
                        let before = self.member_close_run(&member_key(members));
                        let after = periods::allocate(before + net, &shares);
                        let prior = periods::allocate(before, &shares);
                        after.iter().zip(&prior).map(|(a, p)| a - p).collect()
                    }
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

        let close_seq = self
            .periods
            .values()
            .filter(|p| p.status == PeriodStatus::Closed)
            .count() as u64
            + 1;
        let p = self.periods.get_mut(&id).expect("period exists");
        p.status = PeriodStatus::Closed;
        p.closing_entry = closing_entry;
        p.close_seq = Some(close_seq);
        p.member_close = match to {
            periods::CloseTo::Members(members) => Some(periods::MemberCloseRecord {
                members: member_key(members),
                net,
            }),
            _ => None,
        };
        Ok(closing_entry)
    }

    /// The net closed by the run of cumulative members closes, with exactly
    /// these members and shares, that ends with the latest close: zero if
    /// the latest close was anything else (or there is none).
    fn member_close_run(&self, members: &[(AccountId, Decimal)]) -> Decimal {
        let mut closed: Vec<&periods::Period> = self
            .periods
            .values()
            .filter(|p| p.status == PeriodStatus::Closed)
            .collect();
        closed.sort_by_key(|p| std::cmp::Reverse(p.close_seq));
        let mut total = Decimal::ZERO;
        for p in closed {
            match &p.member_close {
                Some(r) if r.members == members => total += r.net,
                _ => break,
            }
        }
        total
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
        if let Some(&id) = self.roles.get(RETAINED_EARNINGS.key) {
            return Ok(id);
        }
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
            periods::CloseTo::Members(members)
            | periods::CloseTo::MembersPerClose(members)
            | periods::CloseTo::MembersLastAbsorbs(members) => {
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
            periods::CloseTo::Members(members)
            | periods::CloseTo::MembersPerClose(members)
            | periods::CloseTo::MembersLastAbsorbs(members) => {
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
    ///
    /// ```text
    /// GOAL: tag an entry with the vendor it paid, for 1099s.
    /// SOURCE: docs/spec/rules.md B-19.
    /// SCENARIOS:
    ///   1. IF the entry or vendor is unknown: SHOULD refuse.
    ///   2. Tagging MUST change no amount (it's metadata, allowed on posted
    ///      entries).
    /// ```
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
    /// SOURCE: docs/spec/rules.md B-4 (the elements); DECISION (Stephen,
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
            // An assigned account was checked when it was assigned, and
            // an account's type never changes.
            if self.roles.contains_key(c.key) {
                continue;
            }
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
        match self.roles.get(c.key) {
            Some(&id) => id,
            None => self.ensure_account(c.code, c.name, c.account_type),
        }
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
    ///
    /// ```text
    /// GOAL: bill a customer and recognize the revenue (accrual basis).
    /// SOURCE: IRS Pub. 538, accrual method (docs/spec/sources/irs-periods-and-methods.md:
    ///         "report income in the year it is earned"); ASC 606-10-25-23;
    ///         control accounts: Ledger::check_controls.
    /// INVARIANTS:
    ///   1. MUST post one balanced entry: Dr 1200 receivables, Cr 4000 revenue,
    ///      for the sum of the lines, dated the invoice date.
    ///   2. The invoice MUST start open, with nothing paid.
    ///   3. A refusal MUST leave the books unchanged.
    /// SCENARIOS:
    ///   1. IF the customer is unknown, a line amount isn't positive, the due
    ///      date is before the invoice date, or the date is in a closed
    ///      period: SHOULD refuse.
    /// ```
    pub fn create_invoice(
        &mut self,
        customer_id: CustomerId,
        date: NaiveDate,
        due_date: NaiveDate,
        lines: Vec<DocumentLine>,
    ) -> Result<InvoiceId, LedgerError> {
        self.atomically(|l| l.create_invoice_steps(customer_id, date, due_date, lines))
    }

    /// The steps of [`Self::create_invoice`], run inside [`Self::atomically`].
    fn create_invoice_steps(
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
    ///
    /// ```text
    /// GOAL: record a vendor's bill as an expense owed (accrual basis).
    /// SOURCE: IRS Pub. 538, accrual method ("deduct ... expenses in the year
    ///         incurred"); control accounts: Ledger::check_controls.
    /// INVARIANTS:
    ///   1. MUST post one balanced entry: Dr 6999 general expenses, Cr 2000
    ///      payables, for the sum of the lines, dated the bill date, tagged
    ///      with the vendor (1099s).
    ///   2. A refusal MUST leave the books unchanged.
    /// SCENARIOS:
    ///   1. IF the vendor is unknown, a line amount isn't positive, the due date
    ///      is before the bill date, or the date is in a closed period: SHOULD
    ///      refuse.
    /// ```
    pub fn create_bill(
        &mut self,
        vendor_id: VendorId,
        date: NaiveDate,
        due_date: NaiveDate,
        lines: Vec<DocumentLine>,
    ) -> Result<BillId, LedgerError> {
        self.atomically(|l| l.create_bill_steps(vendor_id, date, due_date, lines))
    }

    /// The steps of [`Self::create_bill`], run inside [`Self::atomically`].
    fn create_bill_steps(
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
    ///
    /// ```text
    /// GOAL: record a customer's payment against an invoice.
    /// SOURCE: IRS Pub. 538 (accrual); CONVENTION (A/R workflow).
    /// INVARIANTS:
    ///   1. MUST post Dr cash, Cr 1200 receivables, for the amount.
    ///   2. The invoice's paid amount MUST rise by the amount; it is Paid
    ///      exactly when fully paid, Partial when partly.
    /// SCENARIOS:
    ///   1. IF the amount isn't positive: SHOULD refuse.
    ///   2. IF it exceeds what's still owed:
    ///      - SHOULD refuse (Overpayment)
    ///      - NEVER let paid exceed the total
    /// ```
    pub fn receive_payment(
        &mut self,
        invoice_id: InvoiceId,
        date: NaiveDate,
        amount: Decimal,
        cash_account: AccountId,
    ) -> Result<EntryId, LedgerError> {
        self.atomically(|l| l.receive_payment_steps(invoice_id, date, amount, cash_account))
    }

    /// The steps of [`Self::receive_payment`], run inside [`Self::atomically`].
    fn receive_payment_steps(
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
        if date < invoice.date {
            return Err(LedgerError::PaymentBeforeDocument {
                date,
                document_date: invoice.date,
            });
        }
        self.check_controls(&[RECEIVABLES])?;
        // Every check before the entry exists: a refusal leaves no draft.
        self.check_line_account(cash_account)?;
        let ar = self.ensure_control(&RECEIVABLES);
        let entry = self.create_entry(date, &format!("Payment on invoice {}", invoice_id));
        let money = Money::new(amount, self.base_currency);
        self.add_line(entry, cash_account, Side::Debit, money)?;
        self.add_line(entry, ar, Side::Credit, money)?;
        self.post_entry(entry)?;
        let inv = self.invoices.get_mut(&invoice_id).expect("invoice exists");
        inv.amount_paid += amount;
        inv.status = arap::derive_status(inv.amount_paid, inv.total());
        self.invoice_payments
            .entry(invoice_id)
            .or_default()
            .push((entry, amount));
        Ok(entry)
    }

    /// Pay a vendor bill. Posts: Dr A/P, Cr Cash. Updates the bill's paid
    /// amount and status.
    ///
    /// ```text
    /// GOAL: record paying a vendor's bill.
    /// SOURCE: IRS Pub. 538 (accrual); CONVENTION (A/P workflow).
    /// INVARIANTS:
    ///   1. MUST post Dr 2000 payables, Cr cash, for the amount, tagged with
    ///      the vendor.
    ///   2. The bill's paid amount MUST rise by the amount.
    /// SCENARIOS:
    ///   1. IF the amount isn't positive, or exceeds what's still owed: SHOULD
    ///      refuse.
    ///   2. IF dated before the bill: SHOULD refuse (PaymentBeforeDocument);
    ///      the bill's own date is fine (DECISION, Stephen 2026-10-13).
    /// ```
    pub fn pay_bill(
        &mut self,
        bill_id: BillId,
        date: NaiveDate,
        amount: Decimal,
        cash_account: AccountId,
    ) -> Result<EntryId, LedgerError> {
        self.atomically(|l| l.pay_bill_steps(bill_id, date, amount, cash_account))
    }

    /// The steps of [`Self::pay_bill`], run inside [`Self::atomically`].
    fn pay_bill_steps(
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
        if date < bill.date {
            return Err(LedgerError::PaymentBeforeDocument {
                date,
                document_date: bill.date,
            });
        }
        self.check_controls(&[PAYABLES])?;
        // Every check before the entry exists: a refusal leaves no draft.
        self.check_line_account(cash_account)?;
        let ap = self.ensure_control(&PAYABLES);
        let entry = self.create_entry(date, &format!("Payment on bill {}", bill_id));
        let money = Money::new(amount, self.base_currency);
        self.add_line(entry, ap, Side::Debit, money)?;
        self.add_line(entry, cash_account, Side::Credit, money)?;
        self.post_entry(entry)?;
        let b = self.bills.get_mut(&bill_id).expect("bill exists");
        b.amount_paid += amount;
        b.status = arap::derive_status(b.amount_paid, b.total());
        self.bill_payments
            .entry(bill_id)
            .or_default()
            .push((entry, amount));
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
        let inv = self
            .invoices
            .get_mut(&invoice_id)
            .ok_or(LedgerError::UnknownInvoice(invoice_id))?;
        inv.amount_paid += amount;
        self.invoice_payments
            .entry(invoice_id)
            .or_default()
            .push((entry_id, amount));
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
        self.bill_payments
            .entry(bill_id)
            .or_default()
            .push((entry_id, amount));
        let vendor = bill.vendor_id;
        self.entry_vendors.insert(entry_id, vendor);
        Ok(())
    }

    /// A/R aging as of a date: one row per customer with an outstanding
    /// balance, bucketed by days past due.
    pub fn ar_aging(&self, as_of: NaiveDate) -> Vec<AgingRow> {
        let mut by_customer: HashMap<CustomerId, AgingRow> = HashMap::new();
        for inv in self.invoices.values() {
            // Only what existed by `as_of`: issued by then, less payments
            // made by then (Stephen, 2026-10-13).
            if inv.date > as_of {
                continue;
            }
            let balance = inv.total() - self.paid_by(self.invoice_payments.get(&inv.id), as_of);
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

    /// What a document's payments add up to through `as_of`: payments are
    /// posted entries, dated.
    fn paid_by(&self, payments: Option<&Vec<(EntryId, Decimal)>>, as_of: NaiveDate) -> Decimal {
        payments
            .into_iter()
            .flatten()
            .filter(|(e, _)| self.get_entry(*e).is_some_and(|e| e.date() <= as_of))
            .map(|(_, amount)| *amount)
            .sum()
    }

    /// A/P aging as of a date: one row per vendor with an outstanding
    /// balance, bucketed by days past due.
    pub fn ap_aging(&self, as_of: NaiveDate) -> Vec<AgingRow> {
        let mut by_vendor: HashMap<VendorId, AgingRow> = HashMap::new();
        for bill in self.bills.values() {
            // Only what existed by `as_of` (see `ar_aging`).
            if bill.date > as_of {
                continue;
            }
            let balance = bill.total() - self.paid_by(self.bill_payments.get(&bill.id), as_of);
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
    ///
    /// ```text
    /// GOAL: mark a posted line as cleared (seen on the bank statement), or not.
    /// SOURCE: CONVENTION (bank reconciliation); R-? Bank reconciliation.
    /// SCENARIOS:
    ///   1. IF the entry is unknown, a draft, or has no such line: SHOULD refuse.
    ///   2. Marking MUST change no amount, no report but the reconciliation.
    /// ```
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
    ///
    /// ```text
    /// GOAL: compare the books with a bank statement.
    /// SOURCE: CONVENTION (bank reconciliation); R-? Bank reconciliation.
    /// INVARIANTS:
    ///   1. book_balance MUST be the account's net of all posted lines.
    ///   2. cleared_balance MUST be the net of cleared lines dated on or
    ///      before the statement date.
    ///   3. difference MUST equal cleared_balance - statement_balance; zero
    ///      means reconciled.
    ///   4. uncleared_lines MUST list exactly the posted, uncleared lines on
    ///      the account dated on or before the statement date.
    /// ```
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
    ///
    /// ```text
    /// GOAL: an account's balance on its normal side.
    /// SOURCE: docs/spec/rules.md B-5.
    /// INVARIANTS:
    ///   1. MUST equal the account's debits minus credits for debit-normal
    ///      types (asset, expense), and credits minus debits otherwise.
    ///   2. NEVER return negative zero.
    /// SCENARIOS:
    ///   1. IF the account is unknown: SHOULD refuse.
    /// ```
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
    ///
    /// ```text
    /// GOAL: record a payroll: wages, the employer's FICA, what is withheld
    ///       and owed, and net pay.
    /// SOURCE: IRS Pub. 15 (docs/spec/sources/irs-payroll.md); R-? Payroll
    ///         (accounting only: the caller supplies every number).
    /// INVARIANTS:
    ///   1. MUST post one balanced entry: Dr 6100 gross wages, Dr 6110 employer
    ///      FICA, Cr 2110-2150 the withholdings and both shares of FICA, Cr cash
    ///      net pay.
    ///   2. Net pay MUST equal gross minus the employee's withholdings.
    /// SCENARIOS:
    ///   1. IF any amount is negative, gross is zero, or withholdings exceed
    ///      gross: SHOULD refuse.
    /// ```
    pub fn run_payroll(
        &mut self,
        date: NaiveDate,
        lines: Vec<PayrollLine>,
        cash_account: AccountId,
    ) -> Result<PayrollRunId, LedgerError> {
        self.atomically(|l| l.run_payroll_steps(date, lines, cash_account))
    }

    /// The steps of [`Self::run_payroll`], run inside [`Self::atomically`].
    fn run_payroll_steps(
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
        for c in &PAYROLL[2..] {
            let Some(account) = self.role_account(c) else {
                continue;
            };
            // The role resolved to an account, so the balance lookup
            // cannot fail.
            let balance = self.account_balance(account).unwrap_or(Decimal::ZERO);
            if !balance.is_zero() {
                out.push((account, balance));
            }
        }
        out
    }

    /// Remit payroll taxes: pay the `2110`–`2150` payables out of cash.
    /// Posts one entry — Dr each payable for its balance, Cr cash for the
    /// total. Fails with [`LedgerError::NothingToRemit`] when nothing is
    /// owed, and rejects dates in closed periods.
    ///
    /// ```text
    /// GOAL: pay the payroll taxes and withholdings owed.
    /// SOURCE: IRS Pub. 15 (deposits).
    /// INVARIANTS:
    ///   1. MUST zero every payroll payable (2110-2150) against cash.
    /// SCENARIOS:
    ///   1. IF nothing is owed: SHOULD refuse (NothingToRemit).
    /// ```
    pub fn remit_payroll_taxes(
        &mut self,
        date: NaiveDate,
        cash_account: AccountId,
    ) -> Result<EntryId, LedgerError> {
        self.atomically(|l| l.remit_payroll_taxes_steps(date, cash_account))
    }

    /// The steps of [`Self::remit_payroll_taxes`], run inside [`Self::atomically`].
    fn remit_payroll_taxes_steps(
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
    ///
    /// ```text
    /// GOAL: tag a draft line with a class (or clear it).
    /// SOURCE: R-? Tracking classes.
    /// SCENARIOS:
    ///   1. IF the entry is posted:
    ///      - SHOULD refuse
    ///      - NEVER change a posted line (B-8)
    ///   2. IF the line or class doesn't exist: SHOULD refuse.
    /// ```
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

/// Refuse an amount finer than its currency's minor unit (rules.md B-3).
/// A currency with no defined minor unit (ISO 4217 "N.A.") is not checked.
pub(crate) fn check_minor_units(amount: Money) -> Result<(), LedgerError> {
    match amount.currency.minor_units() {
        Some(places) if amount.amount.normalize().scale() > places => {
            Err(LedgerError::FinerThanMinorUnit {
                amount: amount.amount,
                currency: amount.currency.code().to_string(),
                places,
            })
        }
        _ => Ok(()),
    }
}

/// A members close's (capital account, share) list, in order: what a run
/// of cumulative closes must share.
fn member_key(members: &[periods::MemberClose]) -> Vec<(AccountId, Decimal)> {
    members.iter().map(|m| (m.capital, m.share)).collect()
}
