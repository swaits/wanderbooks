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

/// What a posting template drafts: its description, and the lines
/// `(account, side, amount)` ([`Ledger::template_lines`]).
pub type TemplateDraft = (Option<String>, Vec<(AccountId, Side, Money)>);

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
    /// The project facts set for classes (rules.md B-37); a class not here
    /// has the defaults.
    projects: std::collections::BTreeMap<ClassId, crate::classes::ProjectFacts>,
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
    /// The payment methods ([`crate::instruments`]), by id.
    instruments: std::collections::BTreeMap<crate::InstrumentId, crate::Instrument>,
    next_instrument_id: crate::InstrumentId,
    /// The payment method each entry was paid with, for 1099 reporting.
    entry_instruments: HashMap<EntryId, crate::InstrumentId>,
    /// The document each receivables or payables line belongs to, by
    /// (entry, line index) (rules.md B-38, plan P4b.3): written by the
    /// document commands in the step that posts the line, derived on
    /// replay from the document events, and copied onto a reversal's
    /// mirror line. Every figure about a document is read from these.
    line_docs: std::collections::BTreeMap<(EntryId, usize), arap::DocTag>,
    /// Every payment event recorded against a document, each amount once,
    /// whether its entry stands and whether its line could be tagged:
    /// what a version before document standing counts as paid
    /// (`payments_recorded`; plan P4b.7, `document_standing`).
    payment_events: std::collections::BTreeMap<arap::DocRef, Vec<Decimal>>,
    /// Indexes derived from the fields above, kept as they change so the
    /// figures read from them stay linear (none is state of its own):
    /// each document's tagged lines (`line_docs` by document)...
    doc_index:
        std::collections::BTreeMap<arap::DocRef, std::collections::BTreeSet<(EntryId, usize)>>,
    /// ... the accounts holding a document's line, and whether an
    /// invoice's (receivables) or a bill's (payables)...
    doc_accounts: std::collections::BTreeSet<(AccountId, bool)>,
    /// ... each posted entry's place in `order`...
    order_pos: HashMap<EntryId, usize>,
    /// ... and the posted entry reversing each reversed entry (the first
    /// posted, as `reversal_chain` reads it).
    reversed_by: HashMap<EntryId, EntryId>,
    /// Control roles assigned an account ([`Ledger::assign_role`]), by
    /// [`ControlAccount::key`]. A role not here uses its code.
    roles: std::collections::BTreeMap<&'static str, AccountId>,
    /// Every asset tag ever held ([`assets::tag_key`] → the asset that
    /// held it): a tag never moves to another asset.
    asset_tags: std::collections::BTreeMap<String, AssetId>,
    /// Each asset's depreciation runs (`asset.depreciation_posted`), in
    /// the order recorded: what the books carry for it
    /// ([`Ledger::accumulated_posted`]).
    asset_runs: std::collections::BTreeMap<AssetId, Vec<assets::DepreciationPosting>>,
    /// The party directory ([`crate::parties`]).
    parties: std::collections::BTreeMap<crate::PartyId, crate::Party>,
    next_party_id: crate::PartyId,
    /// The parties each entry is with (its counterparties), each with
    /// their share ([`crate::shares`]); never an empty list.
    entry_parties: HashMap<EntryId, Vec<crate::PartyShare>>,
    /// Entries marked as having no counterparty, with why.
    entry_no_party: HashMap<EntryId, String>,
    /// The settings the journal holds ([`crate::settings`]).
    settings: crate::settings::Settings,
    /// Conventions, decisions, open items and approvals
    /// ([`crate::conventions`]).
    conventions: crate::conventions::Conventions,
    /// The facts recorded about each filed file ([`crate::evidence`],
    /// `attachment.annotated`), by attachment id.
    annotations: std::collections::BTreeMap<u64, crate::evidence::Annotation>,
    /// What each entry settles (`entry.settles`, plan P6): the one record
    /// a member-paid entry's "reimbursed" is derived from (IZ-18).
    entry_settles: std::collections::BTreeMap<EntryId, Vec<EntryId>>,
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
            projects: std::collections::BTreeMap::new(),
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
            instruments: std::collections::BTreeMap::new(),
            next_instrument_id: 1,
            entry_instruments: HashMap::new(),
            line_docs: std::collections::BTreeMap::new(),
            payment_events: std::collections::BTreeMap::new(),
            doc_index: std::collections::BTreeMap::new(),
            doc_accounts: std::collections::BTreeSet::new(),
            order_pos: HashMap::new(),
            reversed_by: HashMap::new(),
            roles: std::collections::BTreeMap::new(),
            asset_tags: std::collections::BTreeMap::new(),
            asset_runs: std::collections::BTreeMap::new(),
            parties: std::collections::BTreeMap::new(),
            next_party_id: 1,
            entry_parties: HashMap::new(),
            entry_no_party: HashMap::new(),
            settings: crate::settings::Settings::default(),
            conventions: crate::conventions::Conventions::default(),
            annotations: std::collections::BTreeMap::new(),
            entry_settles: std::collections::BTreeMap::new(),
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
    /// checks added on 2026-10-13, or the calendar bound and the
    /// equal-months start on the 1st (D-37) added since, for replaying
    /// journals written before them. Never for new work.
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
                convention: new.convention,
                postings: Default::default(),
            },
        );
        Ok(id)
    }

    /// Whether a schedule's month was ever posted (and so, if it isn't
    /// posted now, a reversal un-posted it): posting it again needs a
    /// journal an older version refuses (store feature `month_reposting`).
    pub fn schedule_month_posted_before(&self, id: ScheduleId, month_end: NaiveDate) -> bool {
        self.schedules
            .get(&id)
            .and_then(|s| s.postings.get(&month_end))
            .is_some_and(|p| !p.is_empty())
    }

    /// The latest date after a schedule month's end that one of its
    /// postings was reversed (or re-instated) on, if any: posting the month
    /// again on its own date would have the books carry it twice from then
    /// until that date, so it is left for a human
    /// ([`Self::preview_schedule_month`], [`Self::schedule_month_hold`]).
    ///
    /// ```text
    /// GOAL: a schedule month is never recognized twice at a month end.
    /// SOURCE: docs/spec/rules.md B-9 (a correction is dated when it is
    ///         made), B-16; P3 depreciation review R3 (a reversed month is
    ///         due again: when the reversal is the month's own).
    /// ```
    pub fn schedule_month_reversed_after(
        &self,
        id: ScheduleId,
        month_end: NaiveDate,
    ) -> Option<NaiveDate> {
        self.schedules
            .get(&id)?
            .postings
            .get(&month_end)?
            .iter()
            .flat_map(|&e| {
                self.reversal_chain(e)
                    .into_iter()
                    .skip_while(move |&c| c != e)
                    .skip(1)
            })
            .filter_map(|c| self.entries.get(&c).map(JournalEntry::date))
            .filter(|d| *d > month_end)
            .max()
    }

    /// Why a run of schedules leaves a due month for a human, if it must:
    /// it is dated in a closed period (which takes no new entries), or its
    /// posting was reversed after the month ended
    /// ([`Self::schedule_month_reversed_after`]).
    pub fn schedule_month_hold(&self, id: ScheduleId, month_end: NaiveDate) -> Option<String> {
        if let Some(p) = self.closed_period_containing(month_end) {
            return Some(format!(
                "{month_end} is in closed period {p}: book it in an open one by hand, or not at all"
            ));
        }
        self.schedule_month_reversed_after(id, month_end).map(|r| {
            format!(
                "its posting was reversed on {r}, after the month ended: posting it again on {month_end} would recognize the month twice until then; book it by hand in an open period, or not at all"
            )
        })
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
        let (date, amount) = s
            .plan()?
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
        if let Some(r) = self.schedule_month_reversed_after(id, date) {
            return Err(LedgerError::InvalidSchedule(format!(
                "schedule {id}'s {date} posting was reversed on {r}, after its month ended: posting it again on {date} would recognize the month twice until then (book it by hand in an open period, or not at all)"
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
        let s = self.schedules.get_mut(&id).expect("previewed");
        s.posted.insert(date);
        s.postings.entry(date).or_default().push(entry);
        Ok(entry)
    }

    /// Register a recurring entry. Returns its id. A template naming a
    /// receivables or payables account is refused (UntaggedControlLine,
    /// the control guard: plan P4b.3), since every draft it made would be.
    pub fn register_recurring(&mut self, new: NewRecurring) -> Result<RecurringId, LedgerError> {
        schedules::validate_recurring(&new, &self.chart)?;
        if let Some((i, (account, ..))) = new
            .lines
            .iter()
            .enumerate()
            .find(|(_, (a, ..))| self.control_role_of(*a).is_some())
        {
            return Err(LedgerError::UntaggedControlLine {
                account: self.account_label(*account),
                at: format!("line {i} of the recurring template"),
            });
        }
        self.insert_recurring(new)
    }

    /// [`Self::register_recurring`] with the checks every journaled
    /// `recurring.registered` passed when it was written
    /// (schedules::validate_recurring_journaled), for replay only
    /// (rules.md B-28). Never for new work.
    pub fn replay_register_recurring(
        &mut self,
        new: NewRecurring,
    ) -> Result<RecurringId, LedgerError> {
        schedules::validate_recurring_journaled(&new, &self.chart)?;
        self.insert_recurring(new)
    }

    /// Put a checked recurring entry in under the next id.
    fn insert_recurring(&mut self, new: NewRecurring) -> Result<RecurringId, LedgerError> {
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
            // The template's lines as journaled: no size bound (rules.md
            // B-28); a draft too large to post is refused at its post.
            if let Err(e) = self.add_line_journaled(entry, account, side, money, false) {
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
            ("projects", self.projects == other.projects),
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
            ("instruments", self.instruments == other.instruments),
            (
                "next_instrument_id",
                self.next_instrument_id == other.next_instrument_id,
            ),
            (
                "entry_instruments",
                self.entry_instruments == other.entry_instruments,
            ),
            ("line_docs", self.line_docs == other.line_docs),
            (
                "payment_events",
                self.payment_events == other.payment_events,
            ),
            ("roles", self.roles == other.roles),
            ("asset_tags", self.asset_tags == other.asset_tags),
            ("asset_runs", self.asset_runs == other.asset_runs),
            ("parties", self.parties == other.parties),
            ("next_party_id", self.next_party_id == other.next_party_id),
            ("entry_parties", self.entry_parties == other.entry_parties),
            (
                "entry_no_party",
                self.entry_no_party == other.entry_no_party,
            ),
            ("settings", self.settings == other.settings),
            ("conventions", self.conventions == other.conventions),
            ("annotations", self.annotations == other.annotations),
            ("entry_settles", self.entry_settles == other.entry_settles),
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
    /// GOAL: register an account under a unique code and a name no
    ///       sibling has.
    /// SOURCE: rules.md B-4 (the five elements); R-4; DECISION (owner,
    ///         2026-10-13): a child has its parent's type; rules.md B-31
    ///         (names: QuickBooks matches accounts by name,
    ///         docs/spec/sources/intuit-iif.md; no full TIN in what every
    ///         report prints, B-26; DECISION, owner, 2026-10-20).
    /// SCENARIOS:
    ///   1. IF the code is empty or taken: SHOULD refuse.
    ///   2. IF the parent doesn't exist: SHOULD refuse.
    ///   3. IF the parent is another type:
    ///      - SHOULD refuse with ParentTypeMismatch
    ///      - NEVER add the account
    ///      BECAUSE a child subdivides its parent.
    ///   4. IF the name is blank: SHOULD refuse with EmptyAccountName.
    ///   5. IF an account under the same parent (or, for a top-level
    ///      account, another top-level one) has the name, ignoring
    ///      capitals and surrounding space: SHOULD refuse with
    ///      DuplicateAccountName.
    ///   6. IF the name holds a run of nine digits (a full SSN, EIN or
    ///      account number): SHOULD refuse with AccountNameHoldsTin.
    ///   7. IF the name, trimmed, is longer than MAX_ACCOUNT_NAME
    ///      characters, or holds a line break, tab or other control
    ///      character: SHOULD refuse with InvalidAccountName
    ///      BECAUSE every report, export and the IIF print it on one line.
    ///   8. The name is kept trimmed, as a rename keeps it.
    /// ```
    ///
    /// Replay of journals written before these rules uses
    /// [`Self::replay_add_account`]; the accounts the engine creates for a
    /// control role use [`Self::add_control_account`].
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
        let name = name.trim();
        if name.is_empty() {
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
        if self.chart.find_by_code(code).is_some() {
            return Err(LedgerError::DuplicateAccountCode(code.to_string()));
        }
        self.check_account_name(None, name, parent)?;
        self.replay_add_account(code, name, account_type, parent, placeholder)
    }

    /// Create the account a control role posts to, at its code with its
    /// name (invoices, bills, payroll and closes create them when first
    /// needed). The engine's own names: never refused for a sibling's
    /// name, and never for a missing tax category (rules.md B-32), so an
    /// invoice is never refused over a chart label.
    pub fn add_control_account(&mut self, c: &ControlAccount) -> Result<AccountId, LedgerError> {
        self.replay_add_account(c.code, c.name, c.account_type, None, false)
    }

    /// The name rules for a new name (a new account's, or a rename's),
    /// given trimmed and not blank: one line of at most
    /// [`crate::account::MAX_ACCOUNT_NAME`] characters, no run of nine
    /// digits, and none of `account`'s siblings under `parent` has it,
    /// ignoring capitals and surrounding space ([`Self::add_account`]
    /// SCENARIOS 5-7). `account` is the one being renamed, never its own
    /// sibling.
    fn check_account_name(
        &self,
        account: Option<AccountId>,
        name: &str,
        parent: Option<AccountId>,
    ) -> Result<(), LedgerError> {
        let max = crate::account::MAX_ACCOUNT_NAME;
        if name.chars().count() > max {
            return Err(LedgerError::InvalidAccountName(format!(
                "is longer than {max} characters: every report prints it on one line"
            )));
        }
        if name.chars().any(char::is_control) {
            return Err(LedgerError::InvalidAccountName(
                "holds a line break, tab or other control character: every report prints it on one line".into(),
            ));
        }
        if crate::parties::has_full_tin(name) {
            return Err(LedgerError::AccountNameHoldsTin);
        }
        let key = crate::parties::name_key(name);
        let mut siblings: Vec<&Account> = self
            .chart
            .all()
            .filter(|a| a.parent == parent && Some(a.id) != account)
            .filter(|a| crate::parties::name_key(&a.name) == key)
            .collect();
        siblings.sort_by_key(|a| a.id);
        match siblings.first() {
            Some(a) => Err(LedgerError::DuplicateAccountName {
                name: name.trim().to_string(),
                sibling: format!("{} {}", a.code, a.name),
            }),
            None => Ok(()),
        }
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
    /// SOURCE: DECISION (owner, 2026-10-14: "fix the account number");
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

    /// Give an account a new name: `name`, trimmed.
    ///
    /// ```text
    /// GOAL: let the human rename an account without touching its history.
    /// SOURCE: rules.md B-31; DECISION (owner, plan/served.md 4.9 and P3
    ///         "Settled in design", 2026-10-20); rules.md B-8 (posted is
    ///         forever: entries name accounts by id, so a name is a label).
    /// INVARIANTS:
    ///   1. Only the name changes: the account's id, code, type, parent,
    ///      placeholder flag, tax category, and every entry posted to it
    ///      stay as they were, so no entry, balance or statement amount
    ///      changes; every report, past periods included, prints the new
    ///      name.
    ///   2. A refusal MUST leave the books unchanged.
    ///   3. The rules are the command's: replay takes any journaled name
    ///      for an account that exists (Self::replay_rename_account,
    ///      rules.md B-28).
    /// SCENARIOS:
    ///   1. IF the account is unknown: SHOULD refuse with UnknownAccount.
    ///   2. IF the name is blank: SHOULD refuse with EmptyAccountName.
    ///   3. IF the name, trimmed, is the one it has: SHOULD refuse with
    ///      AccountNameUnchanged (nothing to journal).
    ///   4. IF another account under the same parent has it, ignoring
    ///      capitals and surrounding space: SHOULD refuse with
    ///      DuplicateAccountName.
    ///   5. IF it holds a run of nine digits: SHOULD refuse with
    ///      AccountNameHoldsTin.
    ///   6. IF it is longer than MAX_ACCOUNT_NAME characters or holds a
    ///      control character: SHOULD refuse with InvalidAccountName (the
    ///      rules a new account's name meets, Self::add_account).
    ///   7. Approval rules, roles, settings and closed periods never
    ///      block it: they name accounts by code or id, and a name is no
    ///      posting.
    /// ```
    pub fn rename_account(&mut self, account: AccountId, name: &str) -> Result<(), LedgerError> {
        self.check_rename_account(account, name)?;
        self.chart.set_name(account, name.trim())
    }

    /// The validation behind [`Self::rename_account`], for stores that
    /// journal before mutating.
    pub fn check_rename_account(&self, account: AccountId, name: &str) -> Result<(), LedgerError> {
        let a = self
            .chart
            .get(account)
            .ok_or(LedgerError::UnknownAccount(account))?;
        let name = name.trim();
        if name.is_empty() {
            return Err(LedgerError::EmptyAccountName);
        }
        if name == a.name {
            return Err(LedgerError::AccountNameUnchanged {
                code: a.code.clone(),
                name: a.name.clone(),
            });
        }
        self.check_account_name(Some(account), name, a.parent)
    }

    /// Replay a journaled rename: the account must exist; the name is
    /// taken as journaled, whatever later versions refuse in a new one
    /// (rules.md B-28).
    pub fn replay_rename_account(
        &mut self,
        account: AccountId,
        name: &str,
    ) -> Result<(), LedgerError> {
        self.chart.set_name(account, name)
    }

    /// Check a rename's reason before it is journaled: one to
    /// [`crate::settings::MAX_LONG_TEXT`] characters, line breaks and tabs
    /// allowed but no other control character, and no full TIN (rules.md
    /// B-26). Returns it trimmed.
    pub fn check_rename_reason(reason: &str) -> Result<String, LedgerError> {
        let r = crate::settings::long_text("reason", reason, crate::settings::MAX_LONG_TEXT)
            .map_err(LedgerError::InvalidRenameReason)?;
        if crate::parties::has_full_tin(&r) {
            return Err(LedgerError::InvalidRenameReason(
                "reason holds nine digits in a row, which read as a full SSN or EIN: write only the last four".into(),
            ));
        }
        Ok(r)
    }

    /// Whether `policy.require_tax_category` lets an account be created
    /// with `category` (rules.md B-32): under `refuse`, a revenue or
    /// expense account that isn't a header needs one. The command's check
    /// only, never replay's (rules.md B-28); the accounts the engine
    /// creates for a control role never meet it.
    pub fn check_tax_category_required(
        &self,
        code: &str,
        name: &str,
        account_type: AccountType,
        placeholder: bool,
        category: Option<&str>,
    ) -> Result<(), LedgerError> {
        if self.settings.require_tax_category() == crate::settings::TaxCategoryPolicy::Refuse
            && crate::tax::needs_category(account_type, placeholder)
            && category.is_none()
        {
            return Err(LedgerError::TaxCategoryRequired {
                account: format!("{} {}", code.trim(), name.trim()),
                clearing: false,
            });
        }
        Ok(())
    }

    /// Whether `policy.require_tax_category` lets `account`'s category be
    /// set to `category` (rules.md B-32): under `refuse`, an account that
    /// needs one ([`crate::tax::account_needs_category`]) keeps one. The
    /// command's check only, never replay's.
    pub fn check_tax_category_kept(
        &self,
        account: AccountId,
        category: Option<&str>,
    ) -> Result<(), LedgerError> {
        let a = self
            .chart
            .get(account)
            .ok_or(LedgerError::UnknownAccount(account))?;
        if self.settings.require_tax_category() == crate::settings::TaxCategoryPolicy::Refuse
            && category.is_none()
            && crate::tax::account_needs_category(a)
        {
            return Err(LedgerError::TaxCategoryRequired {
                account: format!("{} {}", a.code, a.name),
                clearing: a.tax_category.is_some(),
            });
        }
        Ok(())
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
        // An approval rule names accounts by code: renumbering one it names
        // would leave the rule covering nothing (or, later, another
        // account). It is revised first (crate::conventions INVARIANT 3).
        if let Some((rule, _)) = self
            .conventions
            .approval_rules()
            .find(|(_, r)| r.as_ref().is_none_or(|r| r.accounts.contains(&a.code)))
        {
            return Err(LedgerError::InvalidConvention(format!(
                "approval rule {} names account {}: retire it (books.retire_convention) or revise it without that account (books.set_convention), renumber, then set the rule again with the new code",
                rule.id, a.code
            )));
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
    ///         DECISION (owner, 2026-10-15): roles are journaled, not
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
    ///      - For receivables and payables: DocumentRoleHoldsBalance
    ///        instead, which names no hand entry (the control guard
    ///        refuses one, B-38): their balance moves with its documents,
    ///        a later command (plan P4b S7). Only on their control account
    ///        (Self::control_role_of): the account at the default code,
    ///        before any document posted there, is the chart's own, and
    ///        its balance no document's.
    ///   5. IF the role already resolves to that account: SHOULD accept
    ///      (the assignment is recorded).
    ///   6. IF the role is receivables or payables and the account it
    ///      would move to holds a posted line that is no document's:
    ///      - SHOULD refuse (RoleAccountHasHandLines), naming the entry
    ///      - NEVER make a hand line a control account's
    ///      BECAUSE no command adds an untagged control line (B-38
    ///      INVARIANT 1). Replay never refuses (B-28).
    ///   An account the role moves off keeps its documents' lines, and
    ///   stays guarded while it holds them (Self::control_role_of).
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
        let documents = c.key == RECEIVABLES.key || c.key == PAYABLES.key;
        // SCENARIO 6: the new account's lines must all be documents'.
        if documents
            && !crate::parties::is_replaying()
            && let Some(a) = target
            && now != Some(a.id)
            && let Some(entry) = self.posted_entries().find_map(|e| {
                e.lines()
                    .iter()
                    .enumerate()
                    .any(|(i, l)| l.account == a.id && !self.line_docs.contains_key(&(e.id(), i)))
                    .then_some(e.id())
            })
        {
            return Err(LedgerError::RoleAccountHasHandLines {
                role: c.role.to_string(),
                code: a.code.clone(),
                name: a.name.clone(),
                entry,
            });
        }
        let balance_sheet = !matches!(c.account_type, AccountType::Revenue | AccountType::Expense);
        // A document role's balance is its documents' only on its control
        // account (Self::control_role_of): the account at its default
        // code before any document posted there is a chart's own.
        let controls = |current: AccountId| {
            !documents
                || self
                    .control_role_of(current)
                    .is_some_and(|r| r.key == c.key)
        };
        if let Some(current) = now
            && balance_sheet
            && now != target.map(|a| a.id)
            && controls(current)
        {
            let balance = self.account_balance(current).unwrap_or(Decimal::ZERO);
            if !balance.is_zero() {
                let a = self.chart.get(current).expect("a role's account exists");
                if documents {
                    return Err(LedgerError::DocumentRoleHoldsBalance {
                        role: c.role.to_string(),
                        code: a.code.clone(),
                        name: a.name.clone(),
                        balance,
                    });
                }
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
    ///   0. IF the date is outside the calendar the books keep
    ///      (crate::calendar): SHOULD refuse (DateOutOfRange).
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
        crate::calendar::check(date)?;
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
        self.add_line_journaled(entry, account, side, amount, true)
    }

    /// [`Self::add_line`]'s checks: the account, the minor unit, and (when
    /// `bounded`, for new work) the size. A draft line larger than any
    /// posting could carry is refused when it is added, not only at the
    /// post (P0 review round 3, S1); a recurring entry's draft is generated
    /// with the checks its template passed when it was journaled
    /// (`bounded` false), so a journaled `recurring.generated` always
    /// replays (rules.md B-28).
    fn add_line_journaled(
        &mut self,
        entry: EntryId,
        account: AccountId,
        side: Side,
        amount: Money,
        bounded: bool,
    ) -> Result<(), LedgerError> {
        self.check_line_account(account)?;
        self.check_hand_line(entry, account)?;
        check_minor_units(amount)?;
        if bounded {
            check_line_size(amount.amount)?;
        }
        self.replay_add_line(entry, account, side, amount)
    }

    /// The document role an account is the control account of, if any:
    /// receivables or payables (rules.md B-23, B-38; a later control role
    /// joins them). An account is one when the role is assigned to it
    /// ([`Self::assign_role`]), or when it holds a line of one of the
    /// role's documents (the role's default code, once a document has
    /// posted there; an account the role has moved off, whose documents'
    /// lines stay on it).
    ///
    /// ```text
    /// GOAL: guard exactly the accounts whose balance is documents, and
    ///       no other: a chart's own account at a role's default code
    ///       (1200 Equipment, a card at 2000) is anyone's until a document
    ///       posts there.
    /// SOURCE: docs/spec/rules.md B-23, B-38 INVARIANT 1.
    /// INVARIANTS:
    ///   1. Some(role) when the role is assigned to the account, or the
    ///      account holds a line tagged with one of the role's documents
    ///      (an invoice's for receivables, a bill's for payables).
    ///   2. None for every other account, the default code's included.
    /// ```
    pub fn control_role_of(&self, account: AccountId) -> Option<&'static ControlAccount> {
        [(&RECEIVABLES, true), (&PAYABLES, false)]
            .into_iter()
            .find(|&(c, invoices)| {
                self.roles.get(c.key) == Some(&account)
                    || self.doc_accounts.contains(&(account, invoices))
            })
            .map(|(c, _)| c)
    }

    /// The accounts that are a document role's control accounts
    /// ([`Self::control_role_of`]), in account order.
    fn document_role_accounts(&self, c: &ControlAccount) -> Vec<AccountId> {
        let invoices = c.key == RECEIVABLES.key;
        let mut out: Vec<AccountId> = self
            .doc_accounts
            .iter()
            .filter(|&&(_, i)| i == invoices)
            .map(|&(a, _)| a)
            .collect();
        if let Some(&a) = self.roles.get(c.key)
            && !out.contains(&a)
        {
            out.push(a);
            out.sort_unstable();
        }
        out
    }

    /// Refuse a line by hand on a control account (the control guard,
    /// plan P4b.3): such a line is the document commands' alone, so the
    /// account always equals its documents. Replay never refuses (B-28).
    fn check_hand_line(&self, entry: EntryId, account: AccountId) -> Result<(), LedgerError> {
        if crate::parties::is_replaying() || self.control_role_of(account).is_none() {
            return Ok(());
        }
        let line = self.entries.get(&entry).map_or(0, |e| e.lines().len());
        Err(LedgerError::UntaggedControlLine {
            account: self.account_label(account),
            at: format!("line {line} of entry {entry}"),
        })
    }

    /// An account as a refusal names it: `1200 "Accounts Receivable"`.
    fn account_label(&self, account: AccountId) -> String {
        self.chart.get(account).map_or_else(
            || account.to_string(),
            |a| format!("{} \"{}\"", a.code, a.name),
        )
    }

    /// A document command's line on a control account: [`Self::add_line`]'s
    /// checks but the control guard's, then tagged with its document in
    /// the same step (so the guard at the post finds it tagged).
    fn add_tagged_line(
        &mut self,
        entry: EntryId,
        account: AccountId,
        side: Side,
        amount: Money,
        tag: arap::DocTag,
    ) -> Result<(), LedgerError> {
        self.check_line_account(account)?;
        check_minor_units(amount)?;
        check_line_size(amount.amount)?;
        self.replay_add_line(entry, account, side, amount)?;
        let idx = self.entries[&entry].lines().len() - 1;
        self.tag_line(entry, idx, tag);
        Ok(())
    }

    /// Tag a ledger line with its document, keeping the indexes read from
    /// the tags (`doc_index`, `doc_accounts`) in step. The only way a tag
    /// is written; tags are never removed (a refused command's are rolled
    /// back with the rest of the ledger by [`Self::atomically`]).
    fn tag_line(&mut self, entry: EntryId, idx: usize, tag: arap::DocTag) {
        if let Some(l) = self.entries.get(&entry).and_then(|e| e.lines().get(idx)) {
            self.doc_accounts
                .insert((l.account, matches!(tag.doc, arap::DocRef::Invoice(_))));
        }
        self.doc_index
            .entry(tag.doc)
            .or_default()
            .insert((entry, idx));
        self.line_docs.insert((entry, idx), tag);
    }

    /// The control guard at the post (plan P4b.3): every line of `entry` on
    /// a receivables or payables account carries its document, but the
    /// mirror of an untagged line in a reversal that takes it off the
    /// books (reversing an old hand line is how it is cleaned up).
    /// Replay never refuses (B-28).
    ///
    /// ```text
    /// GOAL: no command adds a line on a control account that is no
    ///       document's; one only takes a legacy one away.
    /// SOURCE: docs/spec/rules.md B-38 INVARIANT 1 and item 3 (the
    ///         exemption).
    /// INVARIANTS:
    ///   1. A tagged line passes.
    ///   2. An untagged line passes only as the mirror of an untagged line
    ///      on the same account in the entry it reverses, when that entry
    ///      stands at an even place in its reversal chain (the hand line
    ///      itself, or a reversal of its reversal): the mirror removes the
    ///      line. Reversing the hand line's reversal would put it back, and
    ///      is refused.
    /// ```
    fn check_control_lines(&self, entry: &JournalEntry) -> Result<(), LedgerError> {
        if crate::parties::is_replaying() {
            return Ok(());
        }
        for (i, l) in entry.lines().iter().enumerate() {
            if self.control_role_of(l.account).is_none()
                || self.line_docs.contains_key(&(entry.id(), i))
            {
                continue;
            }
            let removes_untagged = entry.reversal_of().is_some_and(|o| {
                self.entries
                    .get(&o)
                    .and_then(|o| o.lines().get(i))
                    .is_some_and(|ol| ol.account == l.account)
                    && !self.line_docs.contains_key(&(o, i))
                    && self
                        .reversal_chain(o)
                        .iter()
                        .position(|&e| e == o)
                        .is_some_and(|at| at % 2 == 0)
            });
            if !removes_untagged {
                return Err(LedgerError::UntaggedControlLine {
                    account: self.account_label(l.account),
                    at: format!("line {i} of entry {}", entry.id()),
                });
            }
        }
        Ok(())
    }

    /// A line's account must exist and take postings, checked when the
    /// line is added (owner, 2026-10-13); posting checks again.
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

    /// [`Self::add_line`] without the account check added on 2026-10-13 or
    /// the size bound added in P0, for replaying journals written before
    /// them (rules.md B-28): a draft line replays as journaled, and its post
    /// refuses it if it is out of range. Never for new work.
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
    ///         4217; rounding: DECISION (owner, 2026-10-13).
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
        self.check_hand_line(entry, account)?;
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
        // smallest unit (rules.md B-3; DECISION, owner 2026-10-13, after
        // the GnuCash cross-check found the difference).
        let base_amount = match self.base_currency.minor_units() {
            Some(dp) => exact
                .round_dp_with_strategy(dp, rust_decimal::RoundingStrategy::MidpointAwayFromZero),
            None => exact,
        };
        if base_amount <= Decimal::ZERO {
            return Err(LedgerError::InvalidFxRate(fx_rate));
        }
        check_line_size(base_amount)?;
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
    /// amount as booked (rounded or not, by the rule of its day, and of any
    /// size: the bound is the command's, rules.md B-28) and the foreign
    /// original. For replay only; new work uses [`Self::add_line_fx`].
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
    ///   6. IF a line is on a receivables or payables account (the role's
    ///      account now) and carries no document:
    ///      - SHOULD refuse (UntaggedControlLine), naming the document
    ///        tools
    ///      - NEVER refuse the mirror of an untagged line in a reversal
    ///        BECAUSE a control account must equal its documents (B-23,
    ///        B-38): only the document commands post there, and reversing
    ///        an old hand line is how it is cleaned up. Replay never
    ///        refuses (B-28).
    /// ```
    pub fn post_entry(&mut self, entry: EntryId) -> Result<(), LedgerError> {
        if let Some(e) = self.entries.get(&entry) {
            for line in e.lines() {
                check_minor_units(line.amount)?;
            }
            if e.status() == EntryStatus::Draft {
                self.check_control_lines(e)?;
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
        self.order_pos.insert(entry, self.order.len());
        self.order.push(entry);
        if let Some(o) = snapshot.reversal_of() {
            self.reversed_by.entry(o).or_insert(entry);
        }
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
    ///   5. IF the entry reverses a schedule month's posting (so reversing
    ///      it re-instates that posting) and the month has been posted
    ///      again since:
    ///      - SHOULD refuse (InvalidSchedule), naming the entry that posts
    ///        it now
    ///      - NEVER recognize one month of a schedule twice
    ///      BECAUSE a reversed month is due again and may have been run
    ///      again (P3 depreciation review R3).
    ///   6. IF the entry carries a document's lines (an invoice's or a
    ///      bill's own entry, a payment, or a reversal of one) and `date`
    ///      is before the entry's own date:
    ///      - SHOULD refuse (DocumentReversalBeforeOriginal)
    ///      BECAUSE a correction comes after what it corrects (B-9; plan
    ///      P4b.3, "Dating rules").
    ///   7. IF reversing it would leave a document it carries owing less
    ///      than nothing or more than its total at some date (a void
    ///      under a payment that stands; re-instating a payment beside
    ///      another), where it didn't before:
    ///      - SHOULD refuse (DocumentBounds), naming the entries to
    ///        reverse first
    ///      - NEVER refuse reversing a payment, which only reopens its
    ///        document
    ///      BECAUSE 0 <= outstanding <= total at every date (B-38
    ///      INVARIANT 2). Replay never refuses (B-28).
    /// ```
    pub fn check_reversal(&self, entry: EntryId, date: NaiveDate) -> Result<(), LedgerError> {
        let original = self
            .entries
            .get(&entry)
            .ok_or(LedgerError::UnknownEntry(entry))?;
        if original.status() != EntryStatus::Posted {
            return Err(LedgerError::EntryNotPosted(entry));
        }
        if let Some(&by) = self.reversed_by.get(&entry) {
            return Err(LedgerError::EntryAlreadyReversed(entry, by));
        }
        if self.closing_entry_ids().contains(&entry) {
            return Err(LedgerError::ClosingEntryIrreversible(entry));
        }
        if let Some(pid) = self.closed_period_containing(date) {
            return Err(LedgerError::ClosedPeriod(pid, date.to_string()));
        }
        // SCENARIO 5. `entry` ends its chain (it isn't reversed), so
        // reversing it flips whether the chain's original stands.
        if let Some(top) = self.schedule_chain_top(entry)
            && !self.in_effect(top)
            && let Some((s, month)) = self.schedule_month_of(top)
            && let Some(now) = s.postings[&month]
                .iter()
                .find(|&&e| e != top && self.in_effect(e))
        {
            return Err(LedgerError::InvalidSchedule(format!(
                "reversing entry {entry} would post schedule {}'s {month} a second time: entry {now} posts it now (reverse that one instead)",
                s.id
            )));
        }
        if !crate::parties::is_replaying() {
            self.check_document_reversal(original, date)?;
        }
        Ok(())
    }

    /// [`Self::check_reversal`] SCENARIOS 6 and 7: a reversal of an entry
    /// carrying documents' lines.
    fn check_document_reversal(
        &self,
        original: &JournalEntry,
        date: NaiveDate,
    ) -> Result<(), LedgerError> {
        let mut moves: Vec<(arap::DocRef, Decimal)> = Vec::new();
        for (i, l) in original.lines().iter().enumerate() {
            if let Some(tag) = self.line_docs.get(&(original.id(), i)) {
                // The mirror moves the document by the opposite.
                let by = -tag.doc.counted(l.side, l.amount.amount);
                match moves.iter_mut().find(|(d, _)| *d == tag.doc) {
                    Some((_, m)) => *m += by,
                    None => moves.push((tag.doc, by)),
                }
            }
        }
        let Some(&(first, _)) = moves.first() else {
            return Ok(());
        };
        if date < original.date() {
            return Err(LedgerError::DocumentReversalBeforeOriginal {
                entry: original.id(),
                document: first.to_string(),
                original: original.date(),
                date,
            });
        }
        for (doc, by) in moves {
            self.check_item_bounds(doc, &[(date, by)])?;
        }
        Ok(())
    }

    /// The original an entry's reversal chain starts from (the entry itself
    /// when it reverses nothing), when some schedule month was ever posted:
    /// books without one skip the chain walk.
    fn schedule_chain_top(&self, entry: EntryId) -> Option<EntryId> {
        if self.schedules.values().all(|s| s.postings.is_empty()) {
            return None;
        }
        Some(self.reversal_chain(entry).first().copied().unwrap_or(entry))
    }

    /// Whether a posted entry's effect stands: nothing reverses it, or what
    /// reverses it is itself reversed, and so on (an even number of
    /// reversals after it in its chain). A draft or an unknown id stands
    /// for nothing.
    ///
    /// ```text
    /// SOURCE: docs/spec/rules.md B-9 (a mistake is corrected by an
    ///         opposite entry), B-10 (reversed at most once; reversing the
    ///         reversal re-instates it).
    /// ```
    pub fn in_effect(&self, entry: EntryId) -> bool {
        self.stands_on(entry, NaiveDate::MAX)
    }

    /// Whether a posted entry's effect stands as of a day: it is dated on
    /// or before `as_of`, and of the reversals after it in its chain, an
    /// even number are dated on or before `as_of` (none, or one reversed
    /// again, and so on). A draft or an unknown id stands for nothing.
    ///
    /// ```text
    /// GOAL: say whether an entry counts on a given day, so a document's
    ///       status (a void) and its aging are right as of any date.
    /// SOURCE: docs/spec/rules.md B-9, B-10, B-38; plan/served.md P4b.3
    ///         ("Standing as of a date").
    /// INVARIANTS:
    ///   1. stands_on(e, NaiveDate::MAX) is in_effect(e).
    ///   2. NEVER true for an entry dated after `as_of`, a draft, or an
    ///      unknown id.
    ///   3. A reversal dated after `as_of` doesn't count against it, nor
    ///      does any later link of its chain (a re-reversal an older
    ///      journal dated before the reversal it undoes counts from that
    ///      reversal's date).
    /// SCENARIOS:
    ///   1. IF a payment is reversed on the 25th: SHOULD stand as of the
    ///      22nd, and not as of the 31st.
    /// ```
    pub fn stands_on(&self, entry: EntryId, as_of: NaiveDate) -> bool {
        if self
            .entries
            .get(&entry)
            .is_none_or(|e| e.status() != EntryStatus::Posted || e.date() > as_of)
        {
            return false;
        }
        let chain = self.reversal_chain(entry);
        let Some(i) = chain.iter().position(|&e| e == entry) else {
            return false;
        };
        // A reversal counts only once what it reverses counts: an older
        // journal may date a re-reversal before the reversal it undoes,
        // and it can't undo what hasn't happened yet.
        chain[i + 1..]
            .iter()
            .take_while(|r| self.entries.get(r).is_some_and(|r| r.date() <= as_of))
            .count()
            .is_multiple_of(2)
    }

    /// The schedule and month an entry posted, if it posted one (a
    /// schedule month's posting; not its reversal).
    pub fn schedule_posting(&self, entry: EntryId) -> Option<(ScheduleId, NaiveDate)> {
        self.schedule_month_of(entry).map(|(s, m)| (s.id, m))
    }

    /// The schedule and month an entry posted, if it posted one.
    fn schedule_month_of(&self, entry: EntryId) -> Option<(&AmortizationSchedule, NaiveDate)> {
        self.schedules.values().find_map(|s| {
            s.postings
                .iter()
                .find(|(_, es)| es.contains(&entry))
                .map(|(m, _)| (s, *m))
        })
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

    /// [`Self::reverse_entry`] as a journal recorded it: without the
    /// checks later versions added for documents (a reversal's date, a
    /// document's bounds, the control guard), for replaying journals
    /// written before them (rules.md B-28). Never for new work.
    pub fn replay_reverse_entry(
        &mut self,
        entry: EntryId,
        date: NaiveDate,
        description: &str,
    ) -> Result<EntryId, LedgerError> {
        crate::parties::replaying(|| self.reverse_entry(entry, date, description))
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
            // The mirror of each line, whatever its account: the control
            // guard at the post below exempts the mirror of an untagged
            // control line, and a tagged one carries its tag.
            self.check_line_account(line.account)?;
            self.replay_add_line(id, line.account, kernel::flip(line.side), line.amount)?;
            // The reversal offsets the original line for line: carry the
            // class so divisional P&L nets correctly, and the document so
            // it moves by exactly the opposite (B-38 INVARIANT 3).
            if line.class_id.is_some() {
                self.set_line_class(id, idx, line.class_id)?;
            }
            if let Some(tag) = self.line_docs.get(&(entry, idx)).copied() {
                self.tag_line(id, idx, tag);
            }
        }
        self.entries
            .get_mut(&id)
            .ok_or(LedgerError::UnknownEntry(id))?
            .mark_reversal_of(entry);
        self.post_entry(id)?;
        // A reversed payment un-pays the vendor: carry the tags.
        if let Some(v) = self.entry_vendor(entry) {
            self.entry_vendors.insert(id, v);
        }
        // Paid the same way: its 1099 buckets net to zero too.
        if let Some(i) = self.entry_instrument(entry) {
            self.entry_instruments.insert(id, i);
        }
        // Every party and share: the reversal undoes each one's part.
        if let Some(p) = self.entry_parties.get(&entry).cloned() {
            self.entry_parties.insert(id, p);
        }
        // An entry with no counterparty is reversed with none either
        // (set_entry_no_party INVARIANT 1).
        if let Some(why) = self.entry_no_party.get(&entry).cloned() {
            self.entry_no_party.insert(id, why);
        }
        // A reversed schedule month is un-posted, so it is due again; a
        // reversed reversal posts it back (P3 depreciation review R3).
        if let Some(top) = self.schedule_chain_top(entry)
            && let Some((sid, month)) = self.schedule_month_of(top).map(|(s, m)| (s.id, m))
        {
            let posted = self.schedules[&sid].postings[&month]
                .iter()
                .any(|&e| self.in_effect(e));
            let s = self.schedules.get_mut(&sid).expect("found");
            if posted {
                s.posted.insert(month);
            } else {
                s.posted.remove(&month);
            }
        }
        self.refresh_docs_of(id);
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
    /// [`Self::validate_new_asset_journaled`]; the in-service date and the
    /// life's last day of depreciation inside the calendar the books keep
    /// (crate::calendar): a life whose last day is after 9999-12-31 is
    /// refused like one past chrono's last day (P0 review round 3, S1; P3:
    /// the bound is the last day held, so a life ending on 9999-12-31 is
    /// taken); and the cost and salvage in the currency's minor unit
    /// (rules.md B-3: every depreciation and disposal line derives from
    /// them, and a line finer than a cent could never post).
    pub fn validate_new_asset(&self, new: &NewAsset) -> Result<(), LedgerError> {
        self.check_new_asset(new, true)
    }

    /// The checks every journaled `asset.registered` and `asset.corrected`
    /// passed when it was written (the rule through 2026-10-16):
    /// [`Self::validate_new_asset`] without the calendar bound or the minor
    /// unit, the end of the life only representable. Replay applies
    /// exactly these (rules.md B-28).
    pub fn validate_new_asset_journaled(&self, new: &NewAsset) -> Result<(), LedgerError> {
        self.check_new_asset(new, false)
    }

    /// The checks of [`Self::validate_new_asset`] (`strict`) or
    /// [`Self::validate_new_asset_journaled`], in one order.
    fn check_new_asset(&self, new: &NewAsset, strict: bool) -> Result<(), LedgerError> {
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
        if strict {
            check_minor_units(new.cost)?;
            check_minor_units(new.salvage_value)?;
        }
        // The schedule walks every month of the asset's life; a purchase
        // date near the end of the calendar would overflow date arithmetic
        // (and hang or panic the schedule). Reject it up front. New ones
        // keep the in-service date and the life's last day (the day before
        // its exclusive end) inside the calendar the books keep, so every
        // month of the life is a month the books can post.
        if strict {
            crate::calendar::check(new.in_service_date)?;
        }
        let end = new
            .in_service_date
            .checked_add_months(Months::new(new.useful_life_months));
        if !end
            .is_some_and(|end| !strict || end.pred_opt().is_some_and(crate::calendar::in_calendar))
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
        self.insert_asset(new)
    }

    /// [`Self::register_asset`] with the checks every journaled
    /// `asset.registered` passed when it was written
    /// ([`Self::validate_new_asset_journaled`]), for replay only (rules.md
    /// B-28). Never for new work.
    pub fn replay_register_asset(&mut self, new: NewAsset) -> Result<AssetId, LedgerError> {
        self.validate_new_asset_journaled(&new)?;
        self.insert_asset(new)
    }

    /// Put a checked asset on the register under the next id.
    fn insert_asset(&mut self, new: NewAsset) -> Result<AssetId, LedgerError> {
        let id = self.next_asset_id;
        self.next_asset_id += 1;
        self.assets.insert(
            id,
            Asset {
                id,
                description: new.description,
                in_service_date: new.in_service_date,
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
                basis: None,
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
    /// SOURCE: DECISION (owner, 2026-10-15).
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
    /// SOURCE: DECISION (owner, 2026-10-15).
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
    /// purchase, or a member's contribution of it. The entry's date is the
    /// asset's acquired date ([`Self::asset_acquired_date`]).
    ///
    /// ```text
    /// GOAL: from an asset, its source document and capital entry; from
    ///       the entry, the asset; and the day the company acquired it.
    /// SOURCE: DECISION (owner, 2026-10-15); rules.md B-15 (P3, settled in
    ///         design 2026-10-20: an asset's acquired date comes from the
    ///         entry it is linked to).
    /// INVARIANTS:
    ///   1. The linked entry is posted, is not a reversal, has not been
    ///      reversed, and debits the asset's cost account.
    ///   2. The linked entry is dated on or before the asset's in-service
    ///      date, and on or before its disposal: the register then holds
    ///      the asset exactly while the books carry its cost (B-24).
    ///   3. A refusal MUST leave the books unchanged.
    /// SCENARIOS:
    ///   1. IF the asset or entry is unknown, or the entry fails 1 or 2:
    ///      SHOULD refuse with AcquisitionEntryMismatch (or UnknownAsset /
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

    /// [`Self::set_asset_acquisition`] with the checks every journaled
    /// `asset.acquisition_linked` passed when it was written (INVARIANT 1,
    /// without the dates of INVARIANT 2), for replay only (rules.md B-28).
    pub fn replay_set_asset_acquisition(
        &mut self,
        id: AssetId,
        entry: EntryId,
    ) -> Result<(), LedgerError> {
        self.check_asset_acquisition_journaled(id, entry)?;
        self.assets.get_mut(&id).expect("checked").acquired_by = Some(entry);
        Ok(())
    }

    /// Restore a link recorded earlier (a projection read-back): it was
    /// checked when made, and a later correction (reversing that entry)
    /// doesn't undo it; it only becomes a link to fix, by linking the
    /// replacement. Checks only that the asset and entry exist. (Re-checking
    /// against today's books refused to open them once a linked entry was
    /// reversed, 2026-10-15.)
    pub fn restore_asset_acquisition(
        &mut self,
        id: AssetId,
        entry: EntryId,
    ) -> Result<(), LedgerError> {
        if !self.entries.contains_key(&entry) {
            return Err(LedgerError::UnknownEntry(entry));
        }
        self.assets
            .get_mut(&id)
            .ok_or(LedgerError::UnknownAsset(id))?
            .acquired_by = Some(entry);
        Ok(())
    }

    /// Whether an asset's acquisition link points at an entry that has
    /// since been reversed (to relink the replacement).
    pub fn acquisition_reversed(&self, id: AssetId) -> bool {
        self.assets
            .get(&id)
            .and_then(|a| a.acquired_by)
            .is_some_and(|e| self.entries.values().any(|x| x.reversal_of() == Some(e)))
    }

    /// The day the company acquired an asset: its acquisition entry's date,
    /// or its in-service date when no entry is linked (every asset
    /// registered before P3 without a link), never later than the
    /// in-service date. `None` for an unknown asset.
    ///
    /// ```text
    /// GOAL: the register holds an asset from the day the books carry its
    ///       cost, which can be before it serves (B-24).
    /// SOURCE: rules.md B-15, B-24; P3, settled in design (2026-10-20):
    ///         "an asset's acquired date comes from the entry it is linked
    ///         to".
    /// INVARIANTS:
    ///   1. acquired = min(in_service_date, the linked entry's date); the
    ///      in-service date with no link. One source of truth: no stored
    ///      date to disagree with the entry.
    /// ```
    pub fn asset_acquired_date(&self, id: AssetId) -> Option<NaiveDate> {
        let a = self.assets.get(&id)?;
        Some(
            a.acquired_by
                .and_then(|e| self.entries.get(&e))
                .map_or(a.in_service_date, |e| e.date().min(a.in_service_date)),
        )
    }

    /// The validation behind [`Self::set_asset_acquisition`]: INVARIANTS 1
    /// and 2.
    pub fn check_asset_acquisition(&self, id: AssetId, entry: EntryId) -> Result<(), LedgerError> {
        self.check_asset_acquisition_journaled(id, entry)?;
        let asset = self.assets.get(&id).expect("checked");
        let date = self.entries.get(&entry).expect("checked").date();
        let why = if date > asset.in_service_date {
            Some(format!(
                "it is dated {date}, after the asset's in-service date {} (an asset is acquired before it serves: correct the in-service date with books.correct_asset, or the entry)",
                asset.in_service_date
            ))
        } else {
            match &asset.status {
                AssetStatus::Disposed { date: gone, .. } if date > *gone => Some(format!(
                    "it is dated {date}, after the asset's disposal on {gone}"
                )),
                _ => None,
            }
        };
        match why {
            Some(why) => Err(LedgerError::AcquisitionEntryMismatch {
                asset: id,
                entry,
                why,
            }),
            None => Ok(()),
        }
    }

    /// INVARIANT 1 of [`Self::set_asset_acquisition`]: the checks every
    /// journaled `asset.acquisition_linked` passed.
    fn check_asset_acquisition_journaled(
        &self,
        id: AssetId,
        entry: EntryId,
    ) -> Result<(), LedgerError> {
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
    /// SOURCE: DECISION (owner, 2026-10-15).
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

    /// Record how the company got an asset and the facts its tax figures
    /// follow from ([`assets::AssetBasis`]): the rule, and for a member's
    /// contribution the member's adjusted basis, the fair market value at
    /// contribution, the §704(c) method if chosen, and the source. Book
    /// figures don't change.
    ///
    /// ```text
    /// GOAL: the register carries the tax figures beside book value, each
    ///       named for what it is, so book and tax never get confused and
    ///       the CPA sees both.
    /// SOURCE: docs/spec/rules.md B-25; docs/spec/sources/
    ///         irs-partnership-basis.md; DECISION D-41 (owner, 2026-10-20).
    /// INVARIANTS:
    ///   1. The record passes AssetBasis::check: a purchase takes no
    ///      contribution facts; a contribution needs the adjusted basis and
    ///      the value at contribution, each non-negative, in cents, at
    ///      most one quadrillion.
    ///   2. Only facts are kept: the figures are derived when read
    ///      (Asset::basis_figures), under the rule in force.
    ///   3. A refusal MUST leave the books unchanged.
    /// SCENARIOS:
    ///   1. IF the asset is unknown, or 1 fails: SHOULD refuse with
    ///      UnknownAsset or InvalidTaxBasis.
    ///   2. IF set again: the new record replaces the old (the journal
    ///      keeps both).
    /// ```
    pub fn set_asset_basis(
        &mut self,
        id: AssetId,
        basis: assets::AssetBasis,
    ) -> Result<(), LedgerError> {
        self.check_asset_basis(id, &basis)?;
        self.assets.get_mut(&id).expect("checked").basis = Some(basis);
        Ok(())
    }

    /// The validation behind [`Self::set_asset_basis`].
    pub fn check_asset_basis(
        &self,
        id: AssetId,
        basis: &assets::AssetBasis,
    ) -> Result<(), LedgerError> {
        if !self.assets.contains_key(&id) {
            return Err(LedgerError::UnknownAsset(id));
        }
        basis.check().map_err(LedgerError::InvalidTaxBasis)
    }

    /// [`Self::set_asset_basis`] with the checks every journaled
    /// `asset.basis_set` passed (AssetBasis::check_journaled: the facts fit
    /// the rule, none negative, no bound on size), for replay and the
    /// projection's read-back only (rules.md B-28). The figures the line
    /// carries are for its readers: replay never recomputes them, so a
    /// later change to how a figure is derived never refuses books an
    /// earlier version wrote. Never for new work.
    pub fn replay_set_asset_basis(
        &mut self,
        id: AssetId,
        basis: assets::AssetBasis,
    ) -> Result<(), LedgerError> {
        let asset = self
            .assets
            .get_mut(&id)
            .ok_or(LedgerError::UnknownAsset(id))?;
        basis
            .check_journaled()
            .map_err(LedgerError::InvalidTaxBasis)?;
        asset.basis = Some(basis);
        Ok(())
    }

    /// Replay a pre-P3 `asset.tax_basis_set` (rule and contributor basis,
    /// no value at contribution, which reads as the book cost): the checks
    /// it passed when written (AssetBasis::check_legacy_journaled), for
    /// replay and the projection's read-back only (rules.md B-28). Never for
    /// new work.
    pub fn replay_set_asset_tax_basis(
        &mut self,
        id: AssetId,
        rule: assets::BasisRule,
        contributor_basis: Option<Decimal>,
    ) -> Result<(), LedgerError> {
        let asset = self
            .assets
            .get_mut(&id)
            .ok_or(LedgerError::UnknownAsset(id))?;
        let basis = assets::AssetBasis {
            rule,
            contributor_adjusted_basis: contributor_basis,
            fmv_at_contribution: None,
            sec704c_method: None,
            source: None,
        };
        basis
            .check_legacy_journaled()
            .map_err(LedgerError::InvalidTaxBasis)?;
        asset.basis = Some(basis);
        Ok(())
    }

    /// Correct a registered asset's economics (cost, salvage, useful life,
    /// in-service date) when it was registered wrong. Its accounts, tag,
    /// serial, links, and basis record stay.
    ///
    /// ```text
    /// GOAL: fix a registration mistake without re-registering (which would
    ///       burn its tag, since tags are never reused).
    /// SOURCE: DECISION (owner, 2026-10-15).
    /// INVARIANTS:
    ///   1. The corrected figures pass every check a new registration does
    ///      (validate_new_asset).
    ///   2. Only an active asset is corrected; a store also refuses once any
    ///      depreciation for it is posted (the posted entries would disagree
    ///      with the schedule; correct with entries instead).
    ///   3. The in-service date is never before the linked acquisition
    ///      entry's date (set_asset_acquisition INVARIANT 2).
    ///   4. A contribution's value at contribution and built-in gain stay
    ///      as recorded: they were fixed when it was contributed (Treas.
    ///      Reg. §1.704-3(a)(3)(ii)), whatever the book cost becomes.
    ///   5. A refusal MUST leave the books unchanged.
    /// ```
    pub fn correct_asset(
        &mut self,
        id: AssetId,
        fix: assets::AssetCorrection,
    ) -> Result<(), LedgerError> {
        self.check_correct_asset(id, &fix)?;
        self.apply_asset_correction(id, fix);
        Ok(())
    }

    /// [`Self::correct_asset`] with the checks every journaled
    /// `asset.corrected` passed when it was written
    /// ([`Self::validate_new_asset_journaled`]), for replay only (rules.md
    /// B-28). Never for new work.
    pub fn replay_correct_asset(
        &mut self,
        id: AssetId,
        fix: assets::AssetCorrection,
    ) -> Result<(), LedgerError> {
        let corrected = self.corrected_asset(id, &fix)?;
        self.validate_new_asset_journaled(&corrected)?;
        self.apply_asset_correction(id, fix);
        Ok(())
    }

    /// Write checked corrected figures onto the asset.
    fn apply_asset_correction(&mut self, id: AssetId, fix: assets::AssetCorrection) {
        let a = self.assets.get_mut(&id).expect("checked");
        a.cost = fix.cost;
        a.salvage_value = fix.salvage_value;
        a.useful_life_months = fix.useful_life_months;
        a.in_service_date = fix.in_service_date;
    }

    /// The validation behind [`Self::correct_asset`] (INVARIANTS 1-3).
    pub fn check_correct_asset(
        &self,
        id: AssetId,
        fix: &assets::AssetCorrection,
    ) -> Result<(), LedgerError> {
        self.validate_new_asset(&self.corrected_asset(id, fix)?)?;
        if let Some(linked) = self
            .assets
            .get(&id)
            .and_then(|a| a.acquired_by)
            .and_then(|e| self.entries.get(&e))
            .map(|e| e.date())
            && fix.in_service_date < linked
        {
            return Err(LedgerError::InServiceBeforeAcquisition {
                asset: id,
                in_service: fix.in_service_date.to_string(),
                acquired: linked.to_string(),
            });
        }
        Ok(())
    }

    /// The active asset `id` as `fix` would leave it, as a registration to
    /// check.
    fn corrected_asset(
        &self,
        id: AssetId,
        fix: &assets::AssetCorrection,
    ) -> Result<NewAsset, LedgerError> {
        let a = self.assets.get(&id).ok_or(LedgerError::UnknownAsset(id))?;
        if a.status != AssetStatus::Active {
            return Err(LedgerError::AssetAlreadyDisposed(id));
        }
        Ok(NewAsset {
            description: a.description.clone(),
            in_service_date: fix.in_service_date,
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

    /// Whether a depreciation run can be recorded, for a run a store
    /// journals now: the asset is known; the period is a month, "YYYY-MM";
    /// the amount is positive and in cents; and the entry is posted and
    /// credits the asset's accumulated depreciation account by exactly the
    /// amount.
    ///
    /// ```text
    /// GOAL: a run the books record is the entry that posted it, no more
    ///       and no less, so accumulated_posted is what the books carry.
    /// SOURCE: rules.md B-15 rules 6 and 8, B-24; B-28 (new commands are bounded; replay checks
    ///         only what every journaled run passed:
    ///         record_depreciation_run).
    /// ```
    pub fn check_depreciation_run(
        &self,
        asset: AssetId,
        period: &str,
        entry: EntryId,
        amount: Decimal,
    ) -> Result<(), LedgerError> {
        let a = self
            .assets
            .get(&asset)
            .ok_or(LedgerError::UnknownAsset(asset))?;
        let bad = |why: String| LedgerError::InvalidDepreciationRun { asset, why };
        let month_ok = period.len() == 7
            && period.as_bytes()[4] == b'-'
            && NaiveDate::parse_from_str(&format!("{period}-01"), "%Y-%m-%d").is_ok();
        if !month_ok {
            return Err(bad(format!("period {period:?} isn't a month (YYYY-MM)")));
        }
        if amount <= Decimal::ZERO || amount.normalize().scale() > 2 {
            return Err(bad(format!(
                "amount {amount} isn't a positive number of cents"
            )));
        }
        let e = self
            .entries
            .get(&entry)
            .ok_or(LedgerError::UnknownEntry(entry))?;
        if e.status() != crate::EntryStatus::Posted {
            return Err(bad(format!("entry {entry} isn't posted")));
        }
        let credited = kernel::net(
            e.lines()
                .iter()
                .filter(|l| l.account == a.accum_depr_account)
                .map(|l| (kernel::flip(l.side), l.amount.amount)),
        )
        .ok_or_else(|| bad("its entry's lines are too large".into()))?;
        if credited != amount {
            return Err(bad(format!(
                "entry {entry} credits accumulated depreciation {credited}, not {amount}"
            )));
        }
        Ok(())
    }

    /// Record a depreciation run (`asset.depreciation_posted`). Checks only
    /// that the asset and the entry exist: every run an earlier version
    /// journaled replays, whatever its period or amount (rules.md B-28; a
    /// store checks new runs first, [`Self::check_depreciation_run`]). A
    /// second line for the same asset, period and entry (a journal edited
    /// by hand; a store never writes one) is the run already recorded: one
    /// entry is carried once, as the projection keys it (P3 review,
    /// 2026-10-21: replay counted it twice, the reload once).
    pub fn record_depreciation_run(
        &mut self,
        asset: AssetId,
        period: &str,
        entry: EntryId,
        amount: Decimal,
    ) -> Result<(), LedgerError> {
        if !self.assets.contains_key(&asset) {
            return Err(LedgerError::UnknownAsset(asset));
        }
        if !self.entries.contains_key(&entry) {
            return Err(LedgerError::UnknownEntry(entry));
        }
        let runs = self.asset_runs.entry(asset).or_default();
        if runs.iter().any(|r| r.entry == entry && r.period == period) {
            return Ok(());
        }
        runs.push(assets::DepreciationPosting {
            period: period.to_string(),
            entry,
            amount,
        });
        Ok(())
    }

    /// An asset's depreciation runs, in the order recorded.
    pub fn depreciation_runs(&self, asset: AssetId) -> &[assets::DepreciationPosting] {
        self.asset_runs.get(&asset).map_or(&[], Vec::as_slice)
    }

    /// Each posted reversal's original: original entry → its reversal (an
    /// entry is reversed at most once, B-10). Read once for many runs
    /// ([`assets::depreciation_effects`], [`Self::accumulated_posted_all`]),
    /// never per run: the viewer asks it of every asset.
    pub(crate) fn posted_reversals(&self) -> HashMap<EntryId, EntryId> {
        self.posted_entries()
            .filter_map(|e| e.reversal_of().map(|orig| (orig, e.id())))
            .collect()
    }

    /// An asset's runs as the catch-up rule reads them: (entry, amount),
    /// in the order recorded ([`assets::depreciation_effects`]).
    pub fn depreciation_run_entries(&self, asset: AssetId) -> Vec<(EntryId, Decimal)> {
        self.depreciation_runs(asset)
            .iter()
            .map(|r| (r.entry, r.amount))
            .collect()
    }

    /// What the books carry for an asset's depreciation, entry by entry,
    /// each in its own month: every run and every reversal of one
    /// ([`assets::depreciation_effects`] of its runs).
    pub fn depreciation_effects_of(&self, asset: AssetId) -> Vec<assets::PostedRun> {
        assets::depreciation_effects(self, &self.depreciation_run_entries(asset))
    }

    /// Accumulated depreciation the books carry for an asset on `as_of`:
    /// what its runs carry through that day, entry by entry
    /// ([`assets::depreciation_effects`]' reading, by day); zero once it
    /// is disposed (the disposal entry cleared it).
    ///
    /// ```text
    /// GOAL: book value is cost less what the books carry, never the
    ///       schedule's figure.
    /// SOURCE: rules.md B-24 (the register reconciles to the general
    ///         ledger); B-8, B-9, B-10 (a reversal undoes what it reverses,
    ///         and reversing the reversal reinstates it); B-15 (what the
    ///         catch-up rule reads, the same entries).
    /// INVARIANTS:
    ///   1. Disposed on or before as_of: 0.
    ///   2. Otherwise Σ, over the asset's runs and every entry of each
    ///      one's reversal chain the books carry (assets::run_chain_entries:
    ///      the run +amount, its reversal −amount, that reversal's
    ///      reversal +amount, ...), of those dated on or before as_of. So
    ///      it moves exactly as the trial balance does, even for a
    ///      reversal dated before its run, and agrees with
    ///      depreciation_effects summed through any month's end.
    ///   3. NEVER panics: a sum past exact arithmetic is refused
    ///      (TooLargeToCompute).
    /// ```
    pub fn accumulated_posted(
        &self,
        id: AssetId,
        as_of: NaiveDate,
    ) -> Result<Decimal, LedgerError> {
        self.accumulated_posted_with(id, as_of, &self.posted_reversals())
    }

    /// [`Self::accumulated_posted`] for every registered asset, the
    /// reversal links read once (for the register and the viewer).
    pub fn accumulated_posted_all(
        &self,
        as_of: NaiveDate,
    ) -> Result<std::collections::BTreeMap<AssetId, Decimal>, LedgerError> {
        let reversals = self.posted_reversals();
        self.assets
            .keys()
            .map(|&id| Ok((id, self.accumulated_posted_with(id, as_of, &reversals)?)))
            .collect()
    }

    fn accumulated_posted_with(
        &self,
        id: AssetId,
        as_of: NaiveDate,
        reversals: &HashMap<EntryId, EntryId>,
    ) -> Result<Decimal, LedgerError> {
        let asset = self.assets.get(&id).ok_or(LedgerError::UnknownAsset(id))?;
        if matches!(asset.status, AssetStatus::Disposed { date, .. } if date <= as_of) {
            return Ok(Decimal::ZERO);
        }
        self.carried_between(id, NaiveDate::MIN, as_of, reversals)
    }

    /// Depreciation the books posted for an asset through entries dated in
    /// `[from, to]` (its runs and their reversals, as
    /// [`Self::accumulated_posted`] counts them, the disposal aside): the
    /// period's depreciation expense from its runs.
    pub fn depreciation_posted_between(
        &self,
        id: AssetId,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Decimal, LedgerError> {
        if !self.assets.contains_key(&id) {
            return Err(LedgerError::UnknownAsset(id));
        }
        self.carried_between(id, from, to, &self.posted_reversals())
    }

    /// What the entries of the asset's runs' chains dated in `[from, to]`
    /// carry, summed exactly.
    fn carried_between(
        &self,
        id: AssetId,
        from: NaiveDate,
        to: NaiveDate,
        reversals: &HashMap<EntryId, EntryId>,
    ) -> Result<Decimal, LedgerError> {
        let too_large =
            || LedgerError::TooLargeToCompute(format!("asset {id}'s posted depreciation"));
        assets::run_chain_entries(self, &self.depreciation_run_entries(id), reversals)
            .into_iter()
            .filter(|(d, _)| *d >= from && *d <= to)
            .try_fold(Decimal::ZERO, |t, (_, a)| t.checked_add(a))
            .ok_or_else(too_large)
    }

    /// The run the books carry for an asset's month, if any: the first one
    /// recorded for `period` whose entry is in effect ([`Self::in_effect`]:
    /// posted, and an even number of posted reversals after it). A month
    /// whose run was reversed has none, so it can be run again; one whose
    /// reversal was itself reversed has its run back. At most one run of a
    /// month is in effect: a store refuses re-instating a run whose month
    /// was run again.
    pub fn live_depreciation_run(
        &self,
        id: AssetId,
        period: &str,
    ) -> Option<&assets::DepreciationPosting> {
        self.depreciation_runs(id)
            .iter()
            .find(|run| run.period == period && self.in_effect(run.entry))
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
        self.check_dispose_asset(id, date, proceeds, true)?;
        self.assets.get_mut(&id).expect("asset exists").status =
            AssetStatus::Disposed { date, proceeds };
        Ok(())
    }

    /// [`Self::dispose_asset`] with the checks every journaled
    /// `asset.disposed` passed when it was written (no bound on the
    /// proceeds or the date beyond the disposal's own), for replay only
    /// (rules.md B-28). Never for new work.
    pub fn replay_dispose_asset(
        &mut self,
        id: AssetId,
        date: NaiveDate,
        proceeds: Money,
    ) -> Result<(), LedgerError> {
        self.check_dispose_asset(id, date, proceeds, false)?;
        self.assets.get_mut(&id).expect("asset exists").status =
            AssetStatus::Disposed { date, proceeds };
        Ok(())
    }

    /// The checks of [`Self::dispose_asset`] (`strict`) or
    /// [`Self::replay_dispose_asset`].
    fn check_dispose_asset(
        &self,
        id: AssetId,
        date: NaiveDate,
        proceeds: Money,
        strict: bool,
    ) -> Result<(), LedgerError> {
        if proceeds.currency != self.base_currency {
            return Err(LedgerError::CurrencyMismatch {
                expected: self.base_currency.code().to_string(),
                found: proceeds.currency.code().to_string(),
            });
        }
        let asset = self.assets.get(&id).ok_or(LedgerError::UnknownAsset(id))?;
        // Activeness and the proceeds' sign; no depreciation is computed,
        // so a journaled disposal replays even where the figure can't be
        // (rules.md B-28).
        assets::check_disposal(asset, proceeds)?;
        if strict {
            // Never before the company acquired it. A command's bound, not
            // replay's: the acquired date needs the link, which a
            // projection read-back restores after the disposal (rules.md
            // B-28).
            let acquired = self.asset_acquired_date(id).expect("known asset");
            if date < acquired {
                return Err(LedgerError::DisposalBeforeAcquisition {
                    date: date.to_string(),
                    acquired: acquired.to_string(),
                });
            }
            // Proceeds are a line of the disposal entry, and its date one
            // the books keep (P0 review round 3, S1).
            if !kernel::Amount::in_line_range(proceeds.amount) {
                return Err(LedgerError::AmountOutOfRange);
            }
            crate::calendar::check(date)?;
        }
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
    ///         DECISION (owner, 2026-10-13): periods nest, never
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
    /// 2026-10-13 or the calendar bound added in P0 (only the order is
    /// checked, periods::validate_period_order), for replaying journals
    /// written before them (rules.md B-28).
    pub fn replay_create_period(
        &mut self,
        name: &str,
        start: NaiveDate,
        end: NaiveDate,
    ) -> Result<PeriodId, LedgerError> {
        periods::validate_period_order(start, end)?;
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
                members_closed: None,
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

    /// Whether `entry` is a period's closing entry ([`Self::closing_entry_ids`]
    /// without building the set: O(periods)).
    pub fn is_closing_entry(&self, entry: EntryId) -> bool {
        self.periods
            .values()
            .any(|p| p.closing_entry == Some(entry))
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
    ///      everything closed to these members and shares so far, to the
    ///      currency's minor unit (odd units by largest remainder, ties to
    ///      the member listed first), less what earlier closes gave them
    ///      (rules.md B-13; D-10), and their own distribution account. The
    ///      count starts at the first members close, or at the first since
    ///      the members or shares last changed (`Ledger::members_regime`);
    ///      closes to anything else are skipped. A close that posts an
    ///      entry may carry a one-time true-up of rounding by closes made
    ///      under an earlier rule (the journaled `MembersRun`,
    ///      `MembersPerClose` and `MembersLastAbsorbs`, which replay as
    ///      they were made).
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
    ///      among members by share, to the currency's minor unit, by largest
    ///      remainder, cumulatively).
    ///   3. Entries dated after the period MUST stay untouched (their income
    ///      belongs to their own period).
    ///   4. A sweep account (draws, distributions, dividends) MUST end at zero,
    ///      its balance moved into its target.
    ///   5. NEVER post a closing entry when there is nothing to close.
    ///   6. After a members close that posts, each member's total from the
    ///      count's closes MUST be one largest-remainder split of everything
    ///      they closed: under one minor unit from their exact share. A
    ///      members close that posts nothing credits no one (any true-up
    ///      waits for the next close that posts).
    ///   7. NEVER changes a posted close: earlier closes replay under the
    ///      rule they were journaled with.
    /// SCENARIOS:
    ///   1. IF the period is unknown or already closed: SHOULD refuse.
    ///   2. IF a draft is dated inside it: SHOULD refuse (DraftsInPeriod).
    ///   3. IF a later period is already closed: SHOULD refuse (CloseOutOfOrder).
    ///   4. IF a target or sweep account isn't a distinct equity account, or
    ///      member shares aren't positive and summing to 1: SHOULD refuse.
    ///   5. IF closing to retained earnings and 3999 exists but isn't a
    ///      postable equity account: SHOULD refuse (ControlAccountMismatch).
    ///   6. IF a journaled MembersCumulative names a start other than the
    ///      one the books derive: SHOULD refuse (InvalidMembers)
    ///      BECAUSE replay would split differently from the journal.
    ///   7. IF 50/50 members closed −287.53 in January (A −143.77, B
    ///      −143.76), February closed to retained earnings, and March closes
    ///      −287.53 to the members: SHOULD give A −143.76, B −143.77, so
    ///      each has −287.53 (the count skips February's close).
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
        let close_seq = self.next_close_seq();

        // (account, amount to credit; negative debits): the net to its
        // targets, and each sweep account's balance into its target.
        let mut equity: Vec<(AccountId, Decimal)> = Vec::new();
        let mut sweep_lines: Vec<(AccountId, AccountId, Decimal)> = Vec::new();
        let mut members_closed = None;
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
            _ => {
                let members = to.members().expect("every other target is members");
                // The sweeps first: whether an entry posts decides whether
                // the parts may carry a true-up. Their lines still follow
                // the parts in the entry.
                for m in members {
                    if let Some(d) = m.distributions {
                        sweep(d, m.capital, self);
                    }
                }
                let posts = !closings.is_empty() || !sweep_lines.is_empty();
                let (parts, rule) = self.members_split(to, net, posts, close_seq)?;
                for (m, part) in members.iter().zip(&parts) {
                    equity.push((m.capital, *part));
                }
                members_closed = Some(periods::MembersClosed {
                    members: member_key(members),
                    net,
                    parts,
                    rule,
                });
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
        p.close_seq = Some(close_seq);
        p.member_close = match to {
            periods::CloseTo::MembersRun(members) => Some(periods::MemberCloseRecord {
                members: member_key(members),
                net,
            }),
            _ => None,
        };
        p.members_closed = members_closed;
        Ok(closing_entry)
    }

    /// The number the next close takes ([`Period::close_seq`]): one more
    /// than the closes so far.
    fn next_close_seq(&self) -> u64 {
        self.periods
            .values()
            .filter(|p| p.status == PeriodStatus::Closed)
            .count() as u64
            + 1
    }

    /// What a members close (`to`, closing `net`; `posts` when its entry
    /// posts; numbered `close_seq`) credits each member, in `to`'s order,
    /// and the rule it is made under: the rule journaled for replayed
    /// closes, else the cumulative rule (rules.md B-13): each member's
    /// split of the count's running total, to the base currency's minor
    /// unit, less what the count's earlier closes gave them. A close that
    /// posts nothing credits nothing, so a true-up of earlier rounding
    /// rides only on an entry that posts (INVARIANT 5 stays true).
    fn members_split(
        &self,
        to: &periods::CloseTo,
        net: Decimal,
        posts: bool,
        close_seq: u64,
    ) -> Result<(Vec<Decimal>, periods::MemberRule), LedgerError> {
        use periods::{CloseTo, MemberRule};
        let members = to.members().expect("a members close");
        let shares: Vec<Decimal> = members.iter().map(|m| m.share).collect();
        Ok(match to {
            CloseTo::MembersLastAbsorbs(_) => (
                periods::allocate_last_absorbs(net, &shares),
                MemberRule::LastAbsorbs,
            ),
            CloseTo::MembersPerClose(_) => (periods::allocate(net, &shares), MemberRule::PerClose),
            CloseTo::MembersRun(_) => {
                // The run rule: what the run gives each member with this
                // close, less what it gave them before.
                let before = self.member_close_run(&member_key(members));
                let after = periods::allocate(before + net, &shares);
                let prior = periods::allocate(before, &shares);
                let parts = after.iter().zip(&prior).map(|(a, p)| a - p).collect();
                (parts, MemberRule::Run)
            }
            _ => {
                let key = member_key(members);
                let regime = self.members_regime(&key, None);
                let since_seq = regime.since_seq.unwrap_or(close_seq);
                let parts = if posts {
                    regime.allocate(
                        &key,
                        regime.closed + net,
                        &regime.given,
                        self.base_currency.allocation_places(),
                    )?
                } else {
                    vec![Decimal::ZERO; members.len()]
                };
                (parts, MemberRule::Cumulative { since_seq })
            }
        })
    }

    /// The members regime a close with `members` ((capital account, share)
    /// pairs) belongs to (rules.md B-13), counting only closes whose
    /// period ends on or before `through` (all when `None`): walking the
    /// closes newest first by [`Period::close_seq`], members closes under
    /// any rule with the same SET of (capital, share) pairs (order and
    /// distribution accounts ignored; 0.5 is 0.50), skipping closes to
    /// anything else (they allocate nothing to members), and stopping at
    /// the first members close with another set. A change of members or
    /// shares starts a new count (26 U.S.C. §706(d)(1); Treas. Reg.
    /// §1.706-4(a)(3)(vii): each segment its own distributive share
    /// period); income closed under the old shares is never re-split.
    ///
    /// The date filter keeps a prefix of the closes in close order because
    /// period ends rise with close order: a close is refused while a later
    /// period is closed (CloseOutOfOrder) or when its end lies in a closed
    /// period (`preview_close_period_to`), pinned by
    /// `period_ends_rise_with_close_order`.
    ///
    /// As of a day no count with `members` holds yet, the earnings not
    /// closed by then go to the count the next members close begins, when
    /// it is these members': its first close's order is the regime's
    /// [`Regime::order`], so their tied units go as they will once it
    /// counts them, and a reorder of the members in the settings since
    /// moves no cent on those days either.
    pub(crate) fn members_regime(
        &self,
        members: &[(AccountId, Decimal)],
        through: Option<NaiveDate>,
    ) -> Regime {
        let mut regime = self.regime_where(members, |p| through.is_none_or(|d| p.end <= d));
        if let Some(day) = through
            && regime.order.is_empty()
        {
            let next = self
                .periods
                .values()
                .filter(|p| p.status == PeriodStatus::Closed && p.end > day)
                .filter_map(|p| Some((p.close_seq, p.members_closed.as_ref()?)))
                .min_by_key(|&(seq, _)| seq);
            if let Some((_, m)) = next
                && same_member_set(&m.members, members)
            {
                regime.order.clone_from(&m.members);
            }
        }
        regime
    }

    /// [`Self::members_regime`] over the closed periods `include` keeps.
    fn regime_where(
        &self,
        members: &[(AccountId, Decimal)],
        include: impl Fn(&Period) -> bool,
    ) -> Regime {
        let mut closed: Vec<&Period> = self
            .periods
            .values()
            .filter(|p| p.status == PeriodStatus::Closed && include(p))
            .collect();
        closed.sort_by_key(|p| std::cmp::Reverse(p.close_seq));
        let mut regime = Regime {
            since_seq: None,
            closed: Decimal::ZERO,
            given: vec![Decimal::ZERO; members.len()],
            order: Vec::new(),
        };
        for p in closed {
            let Some(m) = &p.members_closed else {
                continue;
            };
            if !same_member_set(&m.members, members) {
                break;
            }
            regime.since_seq = p.close_seq;
            regime.order.clone_from(&m.members);
            regime.closed += m.net;
            for ((capital, _), part) in m.members.iter().zip(&m.parts) {
                if let Some(i) = members.iter().position(|(c, _)| c == capital) {
                    regime.given[i] += *part;
                }
            }
        }
        regime
    }

    /// A closed members period's split under the cumulative rule
    /// (books.close_period's `members`): the close its count starts at,
    /// and per member, in the close's order, what this close credited
    /// them, their running total from the count's closes through it, and
    /// their exact share of everything the count closed through it. `None`
    /// for a period not closed to members, or closed under an earlier
    /// rule.
    pub fn member_close_split(&self, id: PeriodId) -> Option<(u64, Vec<periods::MemberSplit>)> {
        let p = self.periods.get(&id)?;
        let m = p.members_closed.as_ref()?;
        let periods::MemberRule::Cumulative { since_seq } = m.rule else {
            return None;
        };
        let seq = p.close_seq?;
        let regime = self.regime_where(&m.members, |q| q.close_seq <= Some(seq));
        let split = m
            .members
            .iter()
            .zip(&m.parts)
            .zip(&regime.given)
            .map(
                |((&(capital, share), &part), &running)| periods::MemberSplit {
                    capital,
                    share,
                    part,
                    running,
                    exact: share * regime.closed,
                },
            )
            .collect();
        Some((since_seq, split))
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
        Ok(match self.chart.find_by_code(RETAINED_EARNINGS.code) {
            Some(a) => a.id,
            None => self.add_control_account(&RETAINED_EARNINGS)?,
        })
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
            _ => {
                let members = to.members().expect("every other target is members");
                if let periods::CloseTo::MembersCumulative { since_seq: 0, .. } = to {
                    return Err(LedgerError::InvalidMembers(
                        "a count of members closes starts at close 1 or later, not 0".into(),
                    ));
                }
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
    /// target is valid (for members: their split computes exactly, and a
    /// journaled start is the one the books derive) — and returns the id
    /// the closing entry will receive,
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
        let (closings, net) = self.income_closings(period.end);
        let sweeps: Vec<AccountId> = match to {
            periods::CloseTo::RetainedEarnings { sweep }
            | periods::CloseTo::Account { sweep, .. } => sweep.clone(),
            _ => to
                .members()
                .expect("every other target is members")
                .iter()
                .filter_map(|m| m.distributions)
                .collect(),
        };
        let any = !closings.is_empty()
            || sweeps
                .iter()
                .any(|a| !self.movement_through(*a, period.end).is_zero());
        if to.members().is_some() {
            // The journaled start must be the derived one, and the split
            // must be computable: a close this refuses journals nothing.
            let (_, rule) = self.members_split(to, net, any, self.next_close_seq())?;
            if let (
                periods::CloseTo::MembersCumulative { since_seq, .. },
                periods::MemberRule::Cumulative { since_seq: derived },
            ) = (to, rule)
                && *since_seq != derived
            {
                return Err(LedgerError::InvalidMembers(format!(
                    "this close was journaled counting the members' closes from close {since_seq}, but the books count them from close {derived}"
                )));
            }
        }
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
        // Its party takes this name, so the name obeys party rules (no
        // full TIN; parties INVARIANT 1).
        crate::parties::check_text("name", name).map_err(LedgerError::InvalidParty)?;
        let id = self.next_customer_id;
        self.next_customer_id += 1;
        self.customers.insert(
            id,
            Customer {
                id,
                name: name.to_string(),
            },
        );
        self.link_record_party(name, false, id);
        Ok(id)
    }

    /// Add a vendor. Returns its id.
    ///
    /// ```text
    /// GOAL: a vendor record, linked to its party in the directory.
    /// SOURCE: docs/parties-and-attachments.md; crate::shares INVARIANT 5
    ///         (at most one party on an entry has a vendor record);
    ///         Self::set_entry_parties INVARIANT 2 (the vendor tag follows
    ///         the one party with a vendor record).
    /// INVARIANTS:
    ///   1. The record links to the live party of that name without one,
    ///      else a new party (Self::link_record_party).
    ///   2. IF that party is named on an entry beside another party with a
    ///      vendor record: SHOULD refuse, changing nothing (the entry would
    ///      name two vendors). Record each vendor's part as its own entry
    ///      first.
    ///   3. Every entry naming that party with no vendor tag is tagged with
    ///      the new vendor, as set_entry_parties would have tagged it.
    /// ```
    pub fn add_vendor(&mut self, name: &str) -> Result<VendorId, LedgerError> {
        if name.trim().is_empty() {
            return Err(LedgerError::EmptyAssetDescription);
        }
        // Its party takes this name, so the name obeys party rules (no
        // full TIN; parties INVARIANT 1).
        crate::parties::check_text("name", name).map_err(LedgerError::InvalidParty)?;
        if let Some(p) = self.record_party_match(name, true) {
            let mut ids: Vec<EntryId> = self.entry_parties.keys().copied().collect();
            ids.sort_unstable();
            if let Some(e) = ids.into_iter().find(|e| {
                let named = self.entry_parties(*e);
                named.iter().any(|s| s.party == p)
                    && named
                        .iter()
                        .any(|s| s.party != p && self.parties[&s.party].vendor.is_some())
            }) {
                return Err(LedgerError::InvalidParty(format!(
                    "party {p} ({}) is named on entry {e} beside another party with a vendor record; a 1099 counts a payment toward one vendor, so record each vendor's part as its own entry before adding this vendor",
                    self.parties[&p].name
                )));
            }
        }
        let id = self.next_vendor_id;
        self.next_vendor_id += 1;
        self.vendors.insert(
            id,
            Vendor {
                id,
                name: name.to_string(),
                decisions_1099: Vec::new(),
                tin_on_file: false,
            },
        );
        self.link_record_party(name, true, id);
        // INVARIANT 3: the tag follows the party's new vendor record.
        if let Some(p) = self.party_of_vendor(id) {
            let untagged: Vec<EntryId> = self
                .entry_parties
                .iter()
                .filter(|(e, named)| {
                    named.iter().any(|s| s.party == p) && !self.entry_vendors.contains_key(e)
                })
                .map(|(e, _)| *e)
                .collect();
            for e in untagged {
                self.entry_vendors.insert(e, id);
            }
        }
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

    /// Check a 1099 decision for a vendor before it is recorded
    /// ([`crate::form1099::check_new_decision`]): the vendor exists, the
    /// decision, its source and its date are sound, and a dated decision
    /// records a change. Returns what recording it would do.
    pub fn check_vendor_1099(
        &self,
        id: VendorId,
        new: &crate::form1099::Dated1099,
    ) -> Result<crate::form1099::Applied, LedgerError> {
        let v = self
            .vendors
            .get(&id)
            .ok_or(LedgerError::UnknownVendor(id))?;
        crate::form1099::check_new_decision(&v.decisions_1099, new)
            .map_err(LedgerError::Invalid1099)
    }

    /// Record a vendor's 1099 decision from the date it takes effect (or
    /// from the beginning), with who decided, and whether its W-9 is on
    /// file, checked ([`Self::check_vendor_1099`]).
    ///
    /// ```text
    /// GOAL: "no 1099" is a decision kept with its reason and who made it,
    ///       never the same as "never decided" (plan defects 18, 23), and
    ///       each payment takes the decision in force on its date (D-40).
    /// SOURCE: docs/spec/rules.md B-19; crate::form1099::plan_decision.
    /// SCENARIOS:
    ///   1. IF the vendor is unknown, or the decision fails its checks:
    ///      SHOULD refuse, changing nothing.
    ///   2. Otherwise SHOULD record it (crate::form1099::apply_decision)
    ///      and set the W-9 flag, returning what it did.
    /// ```
    pub fn set_vendor_1099(
        &mut self,
        id: VendorId,
        new: crate::form1099::Dated1099,
        tin_on_file: bool,
    ) -> Result<crate::form1099::Applied, LedgerError> {
        self.check_vendor_1099(id, &new)?;
        self.replay_vendor_1099(id, new, tin_on_file)
    }

    /// [`Self::set_vendor_1099`] as journaled: the vendor must exist, and
    /// the decision is recorded as the line says, without the checks a
    /// new decision passes (rules.md B-28). A line written before
    /// decisions were kept replays as a decision from the beginning with
    /// no source (a null as undetermined: plan 5.5).
    pub fn replay_vendor_1099(
        &mut self,
        id: VendorId,
        new: crate::form1099::Dated1099,
        tin_on_file: bool,
    ) -> Result<crate::form1099::Applied, LedgerError> {
        let v = self
            .vendors
            .get_mut(&id)
            .ok_or(LedgerError::UnknownVendor(id))?;
        let applied = crate::form1099::apply_decision(&mut v.decisions_1099, new);
        v.tin_on_file = tin_on_file;
        Ok(applied)
    }

    /// Mark whether a vendor's W-9 is on file, keeping its 1099 decisions
    /// and who made them (P3 review R8). Returns whether it changed.
    pub fn set_vendor_w9_flag(&mut self, id: VendorId, on_file: bool) -> Result<bool, LedgerError> {
        let v = self
            .vendors
            .get_mut(&id)
            .ok_or(LedgerError::UnknownVendor(id))?;
        let changed = v.tin_on_file != on_file;
        v.tin_on_file = on_file;
        Ok(changed)
    }

    // ------------------------------------------------------------------
    // Payment methods (crate::instruments)
    // ------------------------------------------------------------------

    /// The id the next payment method gets.
    pub fn next_instrument_id(&self) -> crate::InstrumentId {
        self.next_instrument_id
    }

    /// Check a new payment method (crate::instruments INVARIANTS 1-4).
    /// Returns its label, trimmed.
    pub fn check_add_instrument(
        &self,
        label: &str,
        last4: Option<&str>,
        owner_party: Option<crate::PartyId>,
        account: AccountId,
    ) -> Result<String, LedgerError> {
        let label =
            crate::instruments::check_label(label).map_err(LedgerError::InvalidInstrument)?;
        let key = crate::instruments::label_key(&label);
        if let Some(i) = self
            .instruments
            .values()
            .find(|i| crate::instruments::label_key(&i.label) == key)
        {
            return Err(LedgerError::InvalidInstrument(format!(
                "payment method {} is already named {:?}: give this one another name",
                i.id, i.label
            )));
        }
        if let Some(l) = last4 {
            crate::instruments::check_last4(l).map_err(LedgerError::InvalidInstrument)?;
        }
        if let Some(p) = owner_party
            && !self.parties.contains_key(&p)
        {
            return Err(LedgerError::UnknownParty(p));
        }
        let a = self
            .chart
            .get(account)
            .ok_or(LedgerError::UnknownAccount(account))?;
        if a.placeholder {
            return Err(LedgerError::PlaceholderAccount(a.code.clone()));
        }
        if !matches!(a.account_type, AccountType::Asset | AccountType::Liability) {
            return Err(LedgerError::InvalidInstrument(format!(
                "account {} {:?} is {:?}: a payment method pays through an asset (a bank account) or a liability (a card, or what's owed to the members)",
                a.code, a.name, a.account_type
            )));
        }
        let control = [&crate::PAYABLES, &crate::RECEIVABLES]
            .into_iter()
            .find(|c| self.role_account(c) == Some(account));
        if let Some(c) = control {
            return Err(LedgerError::InvalidInstrument(format!(
                "account {} is the {} control account, which no payment is made from: name the bank, card, or members' account the money left",
                a.code, c.role
            )));
        }
        Ok(label)
    }

    /// Add a payment method ([`Self::check_add_instrument`]). Returns its
    /// id.
    pub fn add_instrument(
        &mut self,
        kind: crate::InstrumentKind,
        label: &str,
        last4: Option<&str>,
        owner_party: Option<crate::PartyId>,
        account: AccountId,
    ) -> Result<crate::InstrumentId, LedgerError> {
        let label = self.check_add_instrument(label, last4, owner_party, account)?;
        let id = self.next_instrument_id;
        self.next_instrument_id += 1;
        self.instruments.insert(
            id,
            crate::Instrument {
                id,
                kind,
                label,
                last4: last4.map(str::to_string),
                owner_party,
                account,
            },
        );
        Ok(id)
    }

    /// Every payment method, in id order.
    pub fn instruments(&self) -> impl Iterator<Item = &crate::Instrument> + '_ {
        self.instruments.values()
    }

    /// One payment method.
    pub fn get_instrument(&self, id: crate::InstrumentId) -> Option<&crate::Instrument> {
        self.instruments.get(&id)
    }

    /// The liability accounts that are company cards: the account of each
    /// business-card payment method that is a liability (a credit card's
    /// balance; a debit card draws on an asset and isn't one). A member's
    /// card pays through the members' financing account, and a platform
    /// through its own balance: neither is a card account. For whatever
    /// needs to tell a card from other liabilities (party money, the cash
    /// flow's sections), from what the books record rather than an
    /// account's name.
    pub fn card_accounts(&self) -> std::collections::BTreeSet<AccountId> {
        self.instruments
            .values()
            .filter(|i| i.kind == crate::InstrumentKind::BusinessCard)
            .filter(|i| {
                self.chart
                    .get(i.account)
                    .is_some_and(|a| a.account_type == AccountType::Liability)
            })
            .map(|i| i.account)
            .collect()
    }

    /// The payment method with this label (ignoring case and surrounding
    /// space).
    pub fn instrument_by_label(&self, label: &str) -> Option<&crate::Instrument> {
        let key = crate::instruments::label_key(label);
        self.instruments
            .values()
            .find(|i| crate::instruments::label_key(&i.label) == key)
    }

    /// The payment method an entry is tagged with.
    pub fn entry_instrument(&self, entry: EntryId) -> Option<crate::InstrumentId> {
        self.entry_instruments.get(&entry).copied()
    }

    /// Tag an entry (draft or posted) with the payment method it was paid
    /// with, or clear the tag.
    ///
    /// ```text
    /// GOAL: the 1099 worksheet knows how each payment was made.
    /// SOURCE: docs/spec/rules.md B-19; crate::instruments.
    /// SCENARIOS:
    ///   1. IF the entry or the payment method is unknown: SHOULD refuse.
    ///   2. Tagging MUST change no amount (it's metadata, allowed on posted
    ///      entries, like the vendor tag).
    ///   3. The entry's reversal chain carries the same tag, so a payment
    ///      and its reversal always sit in the same bucket and net to zero.
    /// ```
    pub fn set_entry_instrument(
        &mut self,
        entry: EntryId,
        instrument: Option<crate::InstrumentId>,
    ) -> Result<(), LedgerError> {
        if !self.entries.contains_key(&entry) {
            return Err(LedgerError::UnknownEntry(entry));
        }
        if let Some(i) = instrument
            && !self.instruments.contains_key(&i)
        {
            return Err(LedgerError::UnknownInstrument(i));
        }
        for e in self.reversal_chain(entry) {
            match instrument {
                Some(i) => self.entry_instruments.insert(e, i),
                None => self.entry_instruments.remove(&e),
            };
        }
        Ok(())
    }

    /// Tag an entry (draft or posted) with the vendor it paid or was billed
    /// by, or clear the tag. The tag is reporting metadata, like a receipt
    /// link: it changes no amounts, so posted entries may be tagged.
    ///
    /// ```text
    /// GOAL: tag an entry with the vendor it paid, for 1099s.
    /// SOURCE: docs/spec/rules.md B-19; Self::set_entry_parties
    ///         INVARIANTS 2 and 5 (the vendor tag follows the parties, never
    ///         the other way round, on an entry naming several).
    /// SCENARIOS:
    ///   1. IF the entry or vendor is unknown: SHOULD refuse.
    ///   2. Tagging MUST change no amount (it's metadata, allowed on posted
    ///      entries).
    ///   3. On an entry naming no party or one: Some(v) makes v's party the
    ///      entry's one party (all of it) and clears a no-party mark
    ///      (set_entry_parties INVARIANT 3: an entry never names a party
    ///      and carries the mark at once); None clears the party too when
    ///      it was the vendor's, and leaves no mark.
    ///   4. On an entry naming several parties, Some(v) with v's party
    ///      among them sets the tag and keeps every party and share as
    ///      they are. IF v's party is not among them (or v has none):
    ///      SHOULD refuse, changing nothing: naming v alone would drop the
    ///      others' shares and count all of it toward v's 1099. The list
    ///      is Self::set_entry_parties's to change.
    ///   5. On an entry naming several parties, one of them the tagged
    ///      vendor's party, None SHOULD refuse, changing nothing: the tag
    ///      follows that party (set_entry_parties INVARIANT 2), so the
    ///      1099 would drop a payment the party still shows. Drop the party
    ///      with Self::set_entry_parties instead.
    ///   6. The entry's reversal chain (an entry, its reversal, theirs)
    ///      carries the same tags (set_entry_parties INVARIANT 5).
    /// ```
    pub fn set_entry_vendor(
        &mut self,
        entry: EntryId,
        vendor: Option<VendorId>,
    ) -> Result<(), LedgerError> {
        self.check_set_entry_vendor(entry, vendor)?;
        for e in self.reversal_chain(entry) {
            self.tag_entry_vendor(e, vendor);
        }
        Ok(())
    }

    /// The validation behind [`Self::set_entry_vendor`] (SCENARIOS 1, 4,
    /// 5).
    pub fn check_set_entry_vendor(
        &self,
        entry: EntryId,
        vendor: Option<VendorId>,
    ) -> Result<(), LedgerError> {
        if !self.entries.contains_key(&entry) {
            return Err(LedgerError::UnknownEntry(entry));
        }
        let named = self.entry_parties(entry);
        match vendor {
            Some(v) => {
                if !self.vendors.contains_key(&v) {
                    return Err(LedgerError::UnknownVendor(v));
                }
                let p = self.party_of_vendor(v);
                if named.len() >= 2 && !named.iter().any(|s| Some(s.party) == p) {
                    return Err(LedgerError::InvalidParty(format!(
                        "entry {entry} names {} parties and vendor {v}'s party isn't one of them; tagging the vendor would drop their shares: name the parties with books.set_entry_party (parties) instead",
                        named.len()
                    )));
                }
            }
            None => {
                if named.len() >= 2
                    && let Some(p) = self
                        .entry_vendor(entry)
                        .and_then(|v| self.party_of_vendor(v))
                    && named.iter().any(|s| s.party == p)
                {
                    return Err(LedgerError::InvalidParty(format!(
                        "entry {entry} names party {p}, whose vendor record is its vendor tag, among {} parties; the tag follows that party: drop it from the parties with books.set_entry_party (parties) instead",
                        named.len()
                    )));
                }
            }
        }
        Ok(())
    }

    /// One entry's half of [`Self::set_entry_vendor`], checked already.
    fn tag_entry_vendor(&mut self, entry: EntryId, vendor: Option<VendorId>) {
        match vendor {
            Some(v) => {
                self.entry_vendors.insert(entry, v);
                // Several parties: the list stays (SCENARIO 4).
                if self.entry_parties(entry).len() < 2
                    && let Some(p) = self.party_of_vendor(v)
                {
                    self.entry_parties
                        .insert(entry, vec![crate::PartyShare::whole(p)]);
                }
                if self.entry_parties.contains_key(&entry) {
                    self.entry_no_party.remove(&entry);
                }
            }
            None => {
                // The party tag goes too only when it was the vendor's.
                if let Some(v) = self.entry_vendors.remove(&entry)
                    && self.party_of_vendor(v).is_some()
                    && self.entry_party(entry) == self.party_of_vendor(v)
                {
                    self.entry_parties.remove(&entry);
                }
            }
        }
    }

    /// An entry's reversal chain, in order: the original it reverses (and
    /// so on up), itself, the entry reversing it (and so on down). An
    /// entry reversed by none and reversing none is its own chain. Each
    /// entry is reversed at most once ([`Self::check_reversal`]).
    pub fn reversal_chain(&self, entry: EntryId) -> Vec<EntryId> {
        let mut top = entry;
        while let Some(o) = self.entries.get(&top).and_then(JournalEntry::reversal_of) {
            if !self.entries.contains_key(&o) || o == entry {
                break;
            }
            top = o;
        }
        let mut chain = vec![top];
        let mut cur = top;
        while let Some(&r) = self.reversed_by.get(&cur) {
            if chain.contains(&r) {
                break;
            }
            chain.push(r);
            cur = r;
        }
        chain
    }

    /// Tag an entry with a vendor and that vendor's party (bills, bill
    /// payments): one counterparty, both tags.
    fn tag_vendor(&mut self, entry: EntryId, vendor: VendorId) {
        self.entry_vendors.insert(entry, vendor);
        if let Some(p) = self.party_of_vendor(vendor) {
            self.entry_parties
                .insert(entry, vec![crate::PartyShare::whole(p)]);
        }
    }

    /// Tag an entry with a customer's party (invoices, invoice payments).
    fn tag_customer(&mut self, entry: EntryId, customer: CustomerId) {
        if let Some(p) = self.party_of_customer(customer) {
            self.entry_parties
                .insert(entry, vec![crate::PartyShare::whole(p)]);
        }
    }

    /// Link a new vendor or customer record to its party: the party not
    /// archived with that name (ignoring case and surrounding space) that
    /// has no such record yet, else a new party (named "Name (2)", "(3)",
    /// ... when a live party already has the name). Deterministic, so replay
    /// rebuilds the same directory from the same records.
    fn link_record_party(&mut self, name: &str, vendor: bool, record: u64) {
        let id = match self.record_party_match(name, vendor) {
            Some(id) => id,
            None => {
                let id = self.next_party_id;
                self.next_party_id += 1;
                // Names are unique among live parties (parties INVARIANT 3):
                // a second record of the same name gets "Name (2)", and so on.
                let base = name.trim();
                let taken = |n: &str| {
                    let k = crate::parties::name_key(n);
                    self.parties
                        .values()
                        .any(|p| !p.archived && crate::parties::name_key(&p.name) == k)
                };
                let mut party_name = base.to_string();
                let mut n = 2;
                while taken(&party_name) {
                    party_name = format!("{base} ({n})");
                    n += 1;
                }
                self.parties.insert(
                    id,
                    crate::Party {
                        id,
                        name: party_name,
                        display_name: None,
                        roles: std::collections::BTreeSet::new(),
                        website: None,
                        email: None,
                        default_terms_as_customer: None,
                        default_terms_as_vendor: None,
                        address: None,
                        notes: None,
                        archived: false,
                        w9: None,
                        w4: None,
                        tin: None,
                        vendor: None,
                        customer: None,
                        creditor_of: std::collections::BTreeSet::new(),
                    },
                );
                id
            }
        };
        let p = self.parties.get_mut(&id).expect("found or made above");
        if vendor {
            p.vendor = Some(record);
        } else {
            p.customer = Some(record);
        }
    }

    /// The existing party a new vendor (or customer) record named `name`
    /// links to ([`Self::link_record_party`]): the party not archived with
    /// that name and no such record yet.
    fn record_party_match(&self, name: &str, vendor: bool) -> Option<crate::PartyId> {
        let key = crate::parties::name_key(name);
        let slot = |p: &crate::Party| if vendor { p.vendor } else { p.customer };
        self.parties
            .values()
            .find(|p| !p.archived && crate::parties::name_key(&p.name) == key && slot(p).is_none())
            .map(|p| p.id)
    }

    /// The party a vendor record belongs to.
    pub fn party_of_vendor(&self, vendor: VendorId) -> Option<crate::PartyId> {
        self.parties
            .values()
            .find(|p| p.vendor == Some(vendor))
            .map(|p| p.id)
    }

    /// The party a customer record belongs to.
    pub fn party_of_customer(&self, customer: CustomerId) -> Option<crate::PartyId> {
        self.parties
            .values()
            .find(|p| p.customer == Some(customer))
            .map(|p| p.id)
    }

    /// One party.
    pub fn get_party(&self, id: crate::PartyId) -> Option<&crate::Party> {
        self.parties.get(&id)
    }

    /// Every party, in id order.
    pub fn parties(&self) -> impl Iterator<Item = &crate::Party> + '_ {
        self.parties.values()
    }

    /// The party (not archived) named `name`, ignoring case and surrounding
    /// space.
    pub fn party_by_name(&self, name: &str) -> Option<&crate::Party> {
        let key = crate::parties::name_key(name);
        self.parties
            .values()
            .find(|p| !p.archived && crate::parties::name_key(&p.name) == key)
    }

    /// The sole party an entry is with: Some only when it names exactly
    /// one ([`Self::entry_parties`] lists them all).
    pub fn entry_party(&self, entry: EntryId) -> Option<crate::PartyId> {
        match self.entry_parties.get(&entry).map(Vec::as_slice) {
            Some([one]) => Some(one.party),
            _ => None,
        }
    }

    /// Every party an entry is with, each with their share, in the order
    /// named (empty when it names none). Resolve the shares to amounts with
    /// [`crate::shares::resolve`].
    pub fn entry_parties(&self, entry: EntryId) -> &[crate::PartyShare] {
        self.entry_parties
            .get(&entry)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    /// Whether an entry names `party`, alone or among others.
    pub fn entry_names_party(&self, entry: EntryId, party: crate::PartyId) -> bool {
        self.entry_parties(entry).iter().any(|s| s.party == party)
    }

    /// The next party id the directory will assign.
    pub fn next_party_id(&self) -> crate::PartyId {
        self.next_party_id
    }

    /// Add a party.
    ///
    /// ```text
    /// GOAL: a record for anyone the company deals with.
    /// SOURCE: crate::parties (INVARIANTS 1, 3); DECISION (owner,
    ///         2026-10-15).
    /// SCENARIOS:
    ///   1. IF the name is blank, holds a full TIN, or names a party not
    ///      archived: SHOULD refuse, changing nothing.
    ///   2. IF a role is vendor or customer: SHOULD refuse (those follow
    ///      their records; add_vendor / add_customer make them).
    /// ```
    pub fn add_party(
        &mut self,
        name: &str,
        roles: &[crate::Role],
    ) -> Result<crate::PartyId, LedgerError> {
        self.check_add_party(name, roles)?;
        let id = self.next_party_id;
        self.next_party_id += 1;
        self.parties.insert(
            id,
            crate::Party {
                id,
                name: name.trim().to_string(),
                display_name: None,
                roles: roles.iter().copied().collect(),
                website: None,
                email: None,
                default_terms_as_customer: None,
                default_terms_as_vendor: None,
                address: None,
                notes: None,
                archived: false,
                w9: None,
                w4: None,
                tin: None,
                vendor: None,
                customer: None,
                creditor_of: std::collections::BTreeSet::new(),
            },
        );
        Ok(id)
    }

    /// The validation behind [`Self::add_party`].
    pub fn check_add_party(&self, name: &str, roles: &[crate::Role]) -> Result<(), LedgerError> {
        let bad = |why: String| Err(LedgerError::InvalidParty(why));
        if name.trim().is_empty() {
            return bad("a party needs a name".into());
        }
        crate::parties::check_text("name", name).map_err(LedgerError::InvalidParty)?;
        if self.party_by_name(name).is_some() {
            return bad(format!("a party named {:?} already exists", name.trim()));
        }
        if roles
            .iter()
            .any(|r| matches!(r, crate::Role::Vendor | crate::Role::Customer))
        {
            return bad(
                "vendor and customer roles come with their records: add_vendor or add_customer"
                    .into(),
            );
        }
        Ok(())
    }

    /// Change one of a party's facts.
    ///
    /// ```text
    /// SOURCE: crate::parties (INVARIANTS 1, 3).
    /// SCENARIOS:
    ///   1. IF the party is unknown, a text holds a full TIN or is too long,
    ///      or a new name is blank or taken: SHOULD refuse, changing
    ///      nothing.
    /// ```
    pub fn update_party(
        &mut self,
        id: crate::PartyId,
        field: crate::PartyField,
    ) -> Result<(), LedgerError> {
        self.check_update_party(id, &field)?;
        let p = self.parties.get_mut(&id).expect("checked");
        use crate::PartyField::*;
        match field {
            Name(n) => {
                // Its records carry the same name: aging, bills, and the
                // 1099 worksheet show what the party is called now.
                p.name = n.trim().to_string();
                let (name, vendor, customer) = (p.name.clone(), p.vendor, p.customer);
                if let Some(v) = vendor.and_then(|v| self.vendors.get_mut(&v)) {
                    v.name = name.clone();
                }
                if let Some(c) = customer.and_then(|c| self.customers.get_mut(&c)) {
                    c.name = name;
                }
            }
            DisplayName(v) => p.display_name = v,
            Website(v) => p.website = v,
            Address(v) => p.address = v,
            Notes(v) => p.notes = v,
            Archived(b) => p.archived = b,
            Email(v) => p.email = v,
            DefaultTermsAsCustomer(v) => p.default_terms_as_customer = v,
            DefaultTermsAsVendor(v) => p.default_terms_as_vendor = v,
        }
        Ok(())
    }

    /// The validation behind [`Self::update_party`].
    pub fn check_update_party(
        &self,
        id: crate::PartyId,
        field: &crate::PartyField,
    ) -> Result<(), LedgerError> {
        use crate::PartyField::*;
        let p = self.parties.get(&id).ok_or(LedgerError::UnknownParty(id))?;
        let text = |f: &str, v: &Option<String>| match v {
            Some(t) => crate::parties::check_text(f, t).map_err(LedgerError::InvalidParty),
            None => Ok(()),
        };
        match field {
            Name(n) => {
                if n.trim().is_empty() {
                    return Err(LedgerError::InvalidParty("a party needs a name".into()));
                }
                crate::parties::check_text("name", n).map_err(LedgerError::InvalidParty)?;
                if let Some(other) = self.party_by_name(n)
                    && other.id != id
                {
                    return Err(LedgerError::InvalidParty(format!(
                        "a party named {:?} already exists",
                        n.trim()
                    )));
                }
            }
            DisplayName(v) => text("display_name", v)?,
            Website(v) => text("website", v)?,
            Address(v) => text("address", v)?,
            Notes(v) => text("notes", v)?,
            Archived(false) => {
                if let Some(other) = self.party_by_name(&p.name)
                    && other.id != id
                {
                    return Err(LedgerError::InvalidParty(format!(
                        "another party is named {:?}; rename one first",
                        p.name
                    )));
                }
            }
            Archived(true) => {}
            Email(v) => {
                if let Some(e) = v {
                    crate::parties::check_email("email", e).map_err(LedgerError::InvalidParty)?;
                }
            }
            // Terms are typed and already canonical (`crate::Terms`), so
            // there is nothing here a string could get wrong.
            DefaultTermsAsCustomer(_) | DefaultTermsAsVendor(_) => {}
        }
        Ok(())
    }

    /// Give a party a role, or take one away.
    ///
    /// ```text
    /// SOURCE: crate::parties (INVARIANT 2).
    /// SCENARIOS:
    ///   1. IF the role is vendor or customer: SHOULD refuse (they follow
    ///      the records).
    ///   2. IF taking away a 1099 role leaves W-9 facts with no 1099 role,
    ///      or taking away employee leaves W-4 facts: SHOULD refuse (clear
    ///      the facts first).
    /// ```
    pub fn set_party_role(
        &mut self,
        id: crate::PartyId,
        role: crate::Role,
        on: bool,
    ) -> Result<(), LedgerError> {
        self.check_set_party_role(id, role, on)?;
        let p = self.parties.get_mut(&id).expect("checked");
        if on {
            p.roles.insert(role);
        } else {
            p.roles.remove(&role);
        }
        Ok(())
    }

    /// The validation behind [`Self::set_party_role`].
    pub fn check_set_party_role(
        &self,
        id: crate::PartyId,
        role: crate::Role,
        on: bool,
    ) -> Result<(), LedgerError> {
        let p = self.parties.get(&id).ok_or(LedgerError::UnknownParty(id))?;
        if matches!(role, crate::Role::Vendor | crate::Role::Customer) {
            return Err(LedgerError::InvalidParty(
                "vendor and customer roles follow their records".into(),
            ));
        }
        if !on {
            let mut left = p.roles.clone();
            left.remove(&role);
            if p.w9.is_some() && !left.iter().any(|r| r.gets_1099()) {
                return Err(LedgerError::InvalidParty(
                    "clear the W-9 facts before removing the last 1099 role".into(),
                ));
            }
            if p.w4.is_some() && role == crate::Role::Employee {
                return Err(LedgerError::InvalidParty(
                    "clear the W-4 facts before removing the employee role".into(),
                ));
            }
        }
        Ok(())
    }

    /// Record (or clear) a party's W-9 facts.
    ///
    /// ```text
    /// SOURCE: crate::parties (INVARIANTS 1, 2).
    /// SCENARIOS:
    ///   1. IF the party has no 1099 role (contractor, landlord), or the
    ///      facts' form is wrong (W9Facts::check): SHOULD refuse.
    /// ```
    pub fn set_party_w9(
        &mut self,
        id: crate::PartyId,
        facts: Option<crate::W9Facts>,
    ) -> Result<(), LedgerError> {
        self.check_set_party_w9(id, facts.as_ref())?;
        self.parties.get_mut(&id).expect("checked").w9 = facts;
        Ok(())
    }

    /// The validation behind [`Self::set_party_w9`].
    pub fn check_set_party_w9(
        &self,
        id: crate::PartyId,
        facts: Option<&crate::W9Facts>,
    ) -> Result<(), LedgerError> {
        let p = self.parties.get(&id).ok_or(LedgerError::UnknownParty(id))?;
        if let Some(f) = facts {
            if !p.roles.iter().any(|r| r.gets_1099()) {
                return Err(LedgerError::InvalidParty(
                    "W-9 facts need a 1099 role (contractor or landlord)".into(),
                ));
            }
            f.check().map_err(LedgerError::InvalidParty)?;
            if let Some(t) = &p.tin
                && t.last4 != f.tin_last4
            {
                return Err(LedgerError::InvalidParty(format!(
                    "the party's TIN on file ends in {}, the W-9 facts say {}: they must agree (record the W-9's TIN with it)",
                    t.last4, f.tin_last4
                )));
            }
        }
        Ok(())
    }

    /// Record (or clear) a party's taxpayer-ID facts: the TIN's type, last
    /// four, and vault record, never the number.
    ///
    /// ```text
    /// SOURCE: crate::parties (INVARIANTS 1, 2, 2a); DECISION (owner,
    ///         2026-10-15): every member's and payee's TIN in the vault.
    /// SCENARIOS:
    ///   1. Any role may have them (members need theirs for K-1s).
    ///   2. IF the facts' form is wrong (TinFacts::check): SHOULD refuse.
    ///   3. IF the party's W-9 facts name another last four: SHOULD refuse
    ///      (the two never disagree).
    /// ```
    pub fn set_party_tin(
        &mut self,
        id: crate::PartyId,
        facts: Option<crate::TinFacts>,
    ) -> Result<(), LedgerError> {
        self.check_set_party_tin(id, facts.as_ref())?;
        self.parties.get_mut(&id).expect("checked").tin = facts;
        Ok(())
    }

    /// The validation behind [`Self::set_party_tin`].
    pub fn check_set_party_tin(
        &self,
        id: crate::PartyId,
        facts: Option<&crate::TinFacts>,
    ) -> Result<(), LedgerError> {
        let p = self.parties.get(&id).ok_or(LedgerError::UnknownParty(id))?;
        if let Some(f) = facts {
            f.check().map_err(LedgerError::InvalidParty)?;
            if let Some(w) = &p.w9
                && w.tin_last4 != f.last4
            {
                return Err(LedgerError::InvalidParty(format!(
                    "the party's W-9 facts say the TIN ends in {}, not {}: a new TIN for a W-9 payee comes with its W-9 (books.set_w9)",
                    w.tin_last4, f.last4
                )));
            }
        }
        Ok(())
    }

    /// Name a party the creditor of a liability account the company owes
    /// it itself on (`on`), or stop (`!on`): the card it issued, the loan
    /// it made, what the members advanced (each member).
    ///
    /// ```text
    /// GOAL: the books know whose each card, loan and advance is, so a
    ///       credit to it never counts as paying that party (a fee on its
    ///       own card, interest on its own loan, a distribution still owed
    ///       to a member: a promise, not a payment).
    /// SOURCE: crate::parties INVARIANT 5; rules.md B-35 (DECISION, owner,
    ///         2026-10-20: "a liability credit never counts as paying that
    ///         liability's own creditor").
    /// SCENARIOS:
    ///   1. IF the party or the account is unknown: SHOULD refuse.
    ///   2. IF the account isn't a liability, is a placeholder, or is
    ///      payables or a payroll withholding (owed to many, through their
    ///      own records): SHOULD refuse, changing nothing.
    ///   3. Naming it twice, or stopping one never named, changes nothing.
    ///   4. Several parties may be one account's creditors (both members,
    ///      for what the members advanced).
    /// ```
    pub fn set_party_creditor(
        &mut self,
        id: crate::PartyId,
        account: AccountId,
        on: bool,
    ) -> Result<(), LedgerError> {
        self.check_set_party_creditor(id, account, on)?;
        let p = self.parties.get_mut(&id).expect("checked");
        if on {
            p.creditor_of.insert(account);
        } else {
            p.creditor_of.remove(&account);
        }
        Ok(())
    }

    /// The validation behind [`Self::set_party_creditor`].
    pub fn check_set_party_creditor(
        &self,
        id: crate::PartyId,
        account: AccountId,
        on: bool,
    ) -> Result<(), LedgerError> {
        if !self.parties.contains_key(&id) {
            return Err(LedgerError::UnknownParty(id));
        }
        let a = self
            .chart
            .get(account)
            .ok_or(LedgerError::UnknownAccount(account))?;
        if !on {
            return Ok(());
        }
        if a.account_type != AccountType::Liability {
            return Err(LedgerError::InvalidParty(format!(
                "account {} isn't a liability: a party is the creditor only of what the company owes it (a card, a loan, what the members advanced)",
                a.code
            )));
        }
        if a.placeholder {
            return Err(LedgerError::InvalidParty(format!(
                "account {} is a placeholder: name the creditor of the account under it that is posted to",
                a.code
            )));
        }
        let many = PAYROLL[2..]
            .iter()
            .chain([&PAYABLES])
            .any(|c| self.role_account(c) == Some(account));
        if many {
            return Err(LedgerError::InvalidParty(format!(
                "account {} is owed to many parties through their own records (bills, payroll): it has no one creditor",
                a.code
            )));
        }
        Ok(())
    }

    /// The parties named the creditor of `account`, by id
    /// ([`Self::set_party_creditor`]).
    pub fn creditors_of(&self, account: AccountId) -> Vec<crate::PartyId> {
        self.parties
            .values()
            .filter(|p| p.creditor_of.contains(&account))
            .map(|p| p.id)
            .collect()
    }

    /// Record (or clear) a party's W-4 facts.
    pub fn set_party_w4(
        &mut self,
        id: crate::PartyId,
        facts: Option<crate::W4Facts>,
    ) -> Result<(), LedgerError> {
        self.check_set_party_w4(id, facts.as_ref())?;
        self.parties.get_mut(&id).expect("checked").w4 = facts;
        Ok(())
    }

    /// The validation behind [`Self::set_party_w4`].
    pub fn check_set_party_w4(
        &self,
        id: crate::PartyId,
        facts: Option<&crate::W4Facts>,
    ) -> Result<(), LedgerError> {
        let p = self.parties.get(&id).ok_or(LedgerError::UnknownParty(id))?;
        if let Some(f) = facts {
            if !p.roles.contains(&crate::Role::Employee) {
                return Err(LedgerError::InvalidParty(
                    "W-4 facts need the employee role".into(),
                ));
            }
            f.check().map_err(LedgerError::InvalidParty)?;
        }
        Ok(())
    }

    /// Tag an entry with the party it was with, or clear the tag: one
    /// party with all of it ([`Self::set_entry_parties`] with one derived
    /// share). A party with a vendor record also tags the entry's vendor
    /// (for 1099s), as [`Self::set_entry_vendor`] does the other way round.
    /// Metadata: no amount changes, so posted entries may be tagged.
    pub fn set_entry_party(
        &mut self,
        entry: EntryId,
        party: Option<crate::PartyId>,
    ) -> Result<(), LedgerError> {
        let shares: Vec<crate::PartyShare> =
            party.into_iter().map(crate::PartyShare::whole).collect();
        self.set_entry_parties(entry, &shares)
    }

    /// Name the parties an entry was with, each with their share, or
    /// clear them (an empty list). Metadata: no amount changes, so posted
    /// entries may be tagged.
    ///
    /// ```text
    /// GOAL: one entry with several counterparties (both members, two
    ///       contractors), apportioned exactly.
    /// SOURCE: crate::shares (INVARIANTS 1–5); DECISION (owner,
    ///         2026-10-15).
    /// INVARIANTS:
    ///   1. The shares are checked as crate::shares::check_shares says, and
    ///      at most one party has a vendor record; refused otherwise,
    ///      changing nothing. Whether they resolve against the entry's lines
    ///      depends on settings (cash accounts, members) core doesn't hold:
    ///      callers check crate::shares::resolve before journaling.
    ///   2. The entry's vendor tag follows the one party with a vendor
    ///      record; with none, it is cleared.
    ///   3. Naming any party clears a no-party mark.
    ///   4. entry_party(entry) is Some only for exactly one party.
    ///   5. An entry and its reversal (and theirs: Self::reversal_chain)
    ///      name the same parties, shares, and vendor tag: tagging any one
    ///      of them tags the whole chain, so each party's part of an entry
    ///      and its reversal still nets to zero (crate::shares INVARIANT 6;
    ///      form1099 SCENARIO 6), however it is re-apportioned later.
    /// SCENARIOS:
    ///   1. IF the entry or a party is unknown, a party is named twice, or
    ///      two parties have vendor records: SHOULD refuse.
    ///   2. Both members, no shares: two derived shares.
    ///   3. A reversed $1,000.00 payment named 600/400, then re-apportioned
    ///      700/300: the reversal is 700/300 too, and the pair nets to 0.00
    ///      for each party.
    /// ```
    pub fn set_entry_parties(
        &mut self,
        entry: EntryId,
        shares: &[crate::PartyShare],
    ) -> Result<(), LedgerError> {
        let vendor = self.check_set_entry_parties(entry, shares)?;
        self.apply_entry_parties(entry, shares, vendor);
        Ok(())
    }

    /// [`Self::set_entry_parties`] with the checks every journaled
    /// `entry.parties_set` passed when it was written
    /// (crate::shares::check_shares_journaled: no upper bound on an
    /// amount), for replay only (rules.md B-28). Never for new work.
    pub fn replay_set_entry_parties(
        &mut self,
        entry: EntryId,
        shares: &[crate::PartyShare],
    ) -> Result<(), LedgerError> {
        if !self.entries.contains_key(&entry) {
            return Err(LedgerError::UnknownEntry(entry));
        }
        let vendor = self.party_shares_vendor(shares, false)?;
        self.apply_entry_parties(entry, shares, vendor);
        Ok(())
    }

    /// Record checked shares on the entry's reversal chain
    /// ([`Self::set_entry_parties`] INVARIANTS 2-5).
    fn apply_entry_parties(
        &mut self,
        entry: EntryId,
        shares: &[crate::PartyShare],
        vendor: Option<VendorId>,
    ) {
        for e in self.reversal_chain(entry) {
            if shares.is_empty() {
                self.entry_parties.remove(&e);
                self.entry_vendors.remove(&e);
                continue;
            }
            self.entry_parties.insert(e, shares.to_vec());
            self.entry_no_party.remove(&e);
            match vendor {
                Some(v) => {
                    self.entry_vendors.insert(e, v);
                }
                None => {
                    self.entry_vendors.remove(&e);
                }
            }
        }
    }

    /// The validation behind [`Self::set_entry_parties`]: returns the
    /// vendor record the entry's vendor tag follows, if any.
    pub fn check_set_entry_parties(
        &self,
        entry: EntryId,
        shares: &[crate::PartyShare],
    ) -> Result<Option<VendorId>, LedgerError> {
        if !self.entries.contains_key(&entry) {
            return Err(LedgerError::UnknownEntry(entry));
        }
        self.check_party_shares(shares)
    }

    /// The checks [`Self::check_set_entry_parties`] makes of the shares
    /// themselves, for an entry not yet created: returns the vendor record
    /// the vendor tag would follow, if any.
    pub fn check_party_shares(
        &self,
        shares: &[crate::PartyShare],
    ) -> Result<Option<VendorId>, LedgerError> {
        self.party_shares_vendor(shares, true)
    }

    /// [`Self::check_party_shares`], with crate::shares::check_shares when
    /// `strict` (new work) or check_shares_journaled (replay).
    fn party_shares_vendor(
        &self,
        shares: &[crate::PartyShare],
        strict: bool,
    ) -> Result<Option<VendorId>, LedgerError> {
        if let Some(s) = shares.iter().find(|s| !self.parties.contains_key(&s.party)) {
            return Err(LedgerError::UnknownParty(s.party));
        }
        let known = |p| self.parties.contains_key(&p);
        if strict {
            crate::shares::check_shares(shares, known)
        } else {
            crate::shares::check_shares_journaled(shares, known)
        }
        .map_err(LedgerError::InvalidParty)?;
        let vendors: Vec<(crate::PartyId, VendorId)> = shares
            .iter()
            .filter_map(|s| self.parties[&s.party].vendor.map(|v| (s.party, v)))
            .collect();
        if let [_, _, ..] = vendors.as_slice() {
            return Err(LedgerError::InvalidParty(format!(
                "parties {} have vendor records; a 1099 counts a payment toward one vendor, so record each vendor's part as its own entry",
                vendors
                    .iter()
                    .map(|(p, _)| p.to_string())
                    .collect::<Vec<_>>()
                    .join(" and ")
            )));
        }
        Ok(vendors.first().map(|(_, v)| *v))
    }

    /// Mark an entry as having no counterparty, with why (a transfer
    /// between the company's own accounts, an accrual, a correction), so a
    /// books policy that requires parties can tell "none" from "not yet".
    ///
    /// ```text
    /// SOURCE: docs/parties-and-attachments.md "Counterparties"; DECISION
    ///         (owner, 2026-10-15).
    /// INVARIANTS:
    ///   1. An entry and its reversal (and theirs: Self::reversal_chain)
    ///      carry the same mark, as they name the same parties
    ///      (set_entry_parties INVARIANT 5): marking any one marks the
    ///      chain, and reverse_entry copies the mark to the reversal.
    /// SCENARIOS:
    ///   1. IF the entry is unknown, already names a party, or the reason
    ///      is blank or fails the party text rules: SHOULD refuse.
    ///   2. Tagging the entry with a party later clears the mark
    ///      (set_entry_parties, set_entry_party, set_entry_vendor), on the
    ///      whole chain.
    ///   3. A transfer between the company's own accounts marked, then
    ///      reversed: the reversal is marked with the same reason.
    /// ```
    pub fn set_entry_no_party(&mut self, entry: EntryId, reason: &str) -> Result<(), LedgerError> {
        self.check_set_entry_no_party(entry, reason)?;
        for e in self.reversal_chain(entry) {
            self.entry_no_party.insert(e, reason.trim().to_string());
        }
        Ok(())
    }

    /// The validation behind [`Self::set_entry_no_party`].
    pub fn check_set_entry_no_party(
        &self,
        entry: EntryId,
        reason: &str,
    ) -> Result<(), LedgerError> {
        if !self.entries.contains_key(&entry) {
            return Err(LedgerError::UnknownEntry(entry));
        }
        if let Some(e) = self
            .reversal_chain(entry)
            .into_iter()
            .find(|e| self.entry_parties.contains_key(e))
        {
            return Err(LedgerError::InvalidParty(if e == entry {
                format!("entry {entry} names a party; clear it first")
            } else {
                format!("entry {entry}'s reversal chain entry {e} names a party; clear it first")
            }));
        }
        if reason.trim().is_empty() {
            return Err(LedgerError::InvalidParty(
                "say why the entry has no counterparty".into(),
            ));
        }
        crate::parties::check_text("reason", reason).map_err(LedgerError::InvalidParty)
    }

    /// Why an entry has no counterparty, when it was marked so.
    pub fn entry_no_party(&self, entry: EntryId) -> Option<&str> {
        self.entry_no_party.get(&entry).map(String::as_str)
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
            None => self.ensure_account(c),
        }
    }

    /// Ensure a control role's account exists at its code, creating it
    /// (with the role's name and type: [`Self::add_control_account`]) if
    /// missing. Returns the account id.
    fn ensure_account(&mut self, c: &ControlAccount) -> AccountId {
        if let Some(a) = self.chart.find_by_code(c.code) {
            return a.id;
        }
        self.add_control_account(c).expect("fresh code cannot fail")
    }

    /// A document's revenue, expense or asset side on its draft `entry`:
    /// one line on each part's account, or on `default_account` when the
    /// part names none ([`arap::document_parts`]), each tagged with its
    /// project (a project the books don't have: UnknownClass, and the
    /// caller's `atomically` leaves the books unchanged). Every account is
    /// checked for the document's kind first
    /// ([`Self::check_document_line_account`]).
    fn add_document_parts(
        &mut self,
        entry: EntryId,
        default_account: AccountId,
        side: Side,
        kind: arap::DocumentKind,
        parts: &[(arap::DocumentPartKey, Decimal)],
    ) -> Result<(), LedgerError> {
        for &((account, class), amount) in parts {
            let account = account.unwrap_or(default_account);
            self.check_document_line_account(kind, account)?;
            self.add_line(entry, account, side, Money::new(amount, self.base_currency))?;
            if class.is_some() {
                let last = self
                    .get_entry(entry)
                    .map_or(0, |e| e.lines().len())
                    .saturating_sub(1);
                self.set_line_class(entry, last, class)?;
            }
        }
        Ok(())
    }

    /// A document line's account must be one its kind can post to (plan
    /// P4b S4.1, A-10): an invoice line takes a revenue account, or a
    /// liability holding no role (2400 Deferred Revenue, a contract
    /// liability); a bill line takes an expense account, or an asset that
    /// is not cash and holds no role (1500 Equipment, 1300 Prepaid).
    /// Placeholders and the receivables and payables control accounts are
    /// refused. Checked when a line is added and when the entry posts.
    ///
    /// ```text
    /// GOAL: a document's lines go where its kind can put them, never to a
    ///       control, a cash account or a header.
    /// SOURCE: docs/spec/rules.md B-39; plan P4b S4.1 ("Line accounts"),
    ///         A-10 (every bill line names its account);
    ///         ASC 606-10-45-2 (deferred revenue is a liability).
    /// INVARIANTS:
    ///   1. An invoice line: a revenue account, or a liability that holds
    ///      no role.
    ///   2. A bill line: an expense account, or an asset that is not cash
    ///      (`Ledger::account_is_cash`) and holds no role.
    ///   3. A placeholder, or an account that is a document control
    ///      (receivables or payables), is refused for both.
    ///   4. A refusal names the account, the kind and the rule.
    /// ```
    pub fn check_document_line_account(
        &self,
        kind: arap::DocumentKind,
        account: AccountId,
    ) -> Result<(), LedgerError> {
        self.check_line_account(account)?;
        let a = self.chart.get(account).expect("checked above");
        let kind_name = match kind {
            arap::DocumentKind::Invoice => "an invoice",
            arap::DocumentKind::Bill => "a bill",
        };
        let refuse = |why: &str| {
            Err(LedgerError::DocumentLineAccount {
                code: a.code.clone(),
                name: a.name.clone(),
                kind: kind_name,
                why: why.to_string(),
            })
        };
        if self.control_role_of(account).is_some() {
            return refuse(
                "it is a receivables or payables control account; its balance is its documents'",
            );
        }
        match kind {
            arap::DocumentKind::Invoice => match a.account_type {
                AccountType::Revenue => Ok(()),
                AccountType::Liability if !self.holds_any_role(account) => Ok(()),
                _ => refuse(
                    "an invoice line takes a revenue account, or a liability holding no role (deferred revenue)",
                ),
            },
            arap::DocumentKind::Bill => {
                if a.account_type == AccountType::Expense {
                    return Ok(());
                }
                if a.account_type == AccountType::Asset
                    && !self.account_is_cash(account)
                    && !self.holds_any_role(account)
                {
                    return Ok(());
                }
                refuse(
                    "a bill line takes an expense account, or an asset that is not cash and holds no role (equipment, prepaid)",
                )
            }
        }
    }

    /// Whether `account` is one of the books' cash accounts: the codes in
    /// `books.cash_accounts` when set, else (as `ledger-store`'s
    /// reconciliation reads them) the asset accounts whose names say they
    /// hold cash.
    fn account_is_cash(&self, account: AccountId) -> bool {
        let Some(a) = self.chart.get(account) else {
            return false;
        };
        let configured = self.settings.cash_accounts();
        if !configured.is_empty() {
            return configured.iter().any(|c| c == &a.code);
        }
        const CASH_WORDS: [&str; 5] = ["cash", "bank", "checking", "savings", "money market"];
        let name = a.name.to_lowercase();
        a.account_type == AccountType::Asset
            && !a.placeholder
            && CASH_WORDS.iter().any(|w| name.contains(w))
    }

    /// Whether `account` fills any of the books' control roles (assigned,
    /// or the account at the role's code and type).
    fn holds_any_role(&self, account: AccountId) -> bool {
        control_accounts().any(|c| self.holds_role(c, account))
    }

    /// Create a sales invoice. Auto-posts: Dr `1200` Accounts Receivable,
    /// Cr each line's revenue account (the revenue role, `4000`, when a
    /// line names none). The invoice date must not fall in a closed period.
    ///
    /// ```text
    /// GOAL: bill a customer and recognize the revenue (accrual basis).
    /// SOURCE: IRS Pub. 538, accrual method (docs/spec/sources/irs-periods-and-methods.md:
    ///         "report income in the year it is earned"); ASC 606-10-25-23;
    ///         control accounts: Ledger::check_controls; rules.md B-38, B-39.
    /// INVARIANTS:
    ///   1. MUST post one balanced entry: Dr 1200 receivables, Cr the lines'
    ///      accounts, for the sum of the lines, dated the invoice date: one
    ///      line per (account, project) part the lines name, the parts
    ///      naming neither as one (arap::document_parts), each revenue line
    ///      tagged with its project; the receivable line never (rules.md
    ///      B-37 item 8).
    ///   2. The receivable line MUST carry the invoice (Origin), in the
    ///      step that posts it: the invoice starts Open, owing its total.
    ///   3. A refusal MUST leave the books unchanged.
    /// SCENARIOS:
    ///   1. IF the customer is unknown, a line amount isn't positive, the due
    ///      date is before the invoice date, a line names a project the
    ///      books don't have, or the date is in a closed period: SHOULD
    ///      refuse.
    /// ```
    pub fn create_invoice(
        &mut self,
        customer_id: CustomerId,
        date: NaiveDate,
        due_date: NaiveDate,
        lines: Vec<DocumentLine>,
    ) -> Result<InvoiceId, LedgerError> {
        self.create_invoice_with(
            customer_id,
            date,
            due_date,
            lines,
            arap::InvoiceFields::default(),
        )
    }

    /// [`Self::create_invoice`] with the document's new fields (number,
    /// terms, snapshots, references: plan P4b S4.1). The fields are stored
    /// on the invoice at issue and never change. All `None` is exactly
    /// [`Self::create_invoice`].
    pub fn create_invoice_with(
        &mut self,
        customer_id: CustomerId,
        date: NaiveDate,
        due_date: NaiveDate,
        lines: Vec<DocumentLine>,
        fields: arap::InvoiceFields,
    ) -> Result<InvoiceId, LedgerError> {
        self.atomically(|l| l.create_invoice_steps(customer_id, date, due_date, lines, fields))
    }

    /// The steps of [`Self::create_invoice`], run inside [`Self::atomically`].
    fn create_invoice_steps(
        &mut self,
        customer_id: CustomerId,
        date: NaiveDate,
        due_date: NaiveDate,
        lines: Vec<DocumentLine>,
        fields: arap::InvoiceFields,
    ) -> Result<InvoiceId, LedgerError> {
        if !self.customers.contains_key(&customer_id) {
            return Err(LedgerError::UnknownCustomer(customer_id));
        }
        arap::validate_lines(&lines)?;
        arap::validate_due_date(date, due_date)?;
        if let Some(number) = &fields.number
            && self
                .invoices
                .values()
                .any(|i| i.number.as_deref() == Some(number.as_str()))
        {
            return Err(LedgerError::DuplicateInvoiceNumber(number.clone()));
        }
        if let Some(pid) = self.closed_period_containing(date) {
            return Err(LedgerError::ClosedPeriod(pid, date.to_string()));
        }
        let total = arap::document_total(&lines, "the invoice")?;
        let parts = arap::document_parts(&lines, "the invoice")?;
        self.check_controls(&[RECEIVABLES, REVENUE])?;
        let ar = self.ensure_control(&RECEIVABLES);
        let revenue = self.ensure_control(&REVENUE);
        let id = self.next_invoice_id;
        let entry = self.create_entry(date, &format!("Invoice {id}"));
        self.add_tagged_line(
            entry,
            ar,
            Side::Debit,
            Money::new(total, self.base_currency),
            arap::DocTag {
                doc: arap::DocRef::Invoice(id),
                movement: arap::Movement::Origin,
            },
        )?;
        self.add_document_parts(
            entry,
            revenue,
            Side::Credit,
            arap::DocumentKind::Invoice,
            &parts,
        )?;
        self.post_entry(entry)?;
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
                owed: Decimal::ZERO,
                status: DocumentStatus::Open,
                entry_id: entry,
                number: fields.number,
                terms: fields.terms,
                bill_to: fields.bill_to,
                seller: fields.seller,
                customer_ref: fields.customer_ref,
                memo: fields.memo,
                sends: Vec::new(),
            },
        );
        self.tag_customer(entry, customer_id);
        self.refresh_doc(arap::DocRef::Invoice(id));
        Ok(id)
    }

    /// Create a vendor bill. Auto-posts: Dr each line's expense or asset
    /// account (general expenses, `6999`, when a line names none), Cr
    /// `2000` Accounts Payable. The bill date must not fall in a closed
    /// period.
    ///
    /// ```text
    /// GOAL: record a vendor's bill as an expense or asset owed (accrual
    ///       basis).
    /// SOURCE: IRS Pub. 538, accrual method ("deduct ... expenses in the year
    ///         incurred"); control accounts: Ledger::check_controls;
    ///         rules.md B-38, B-39 (plan P4b S4.1, A-10).
    /// INVARIANTS:
    ///   1. MUST post one balanced entry: Dr the lines' accounts, Cr 2000
    ///      payables, for the sum of the lines, dated the bill date, tagged
    ///      with the vendor (1099s): one line per (account, project) part
    ///      the lines name, the parts naming neither as one
    ///      (arap::document_parts), each tagged with its project; the
    ///      payable line never (rules.md B-37 item 8).
    ///   2. The payable line MUST carry the bill (Origin), in the step that
    ///      posts it.
    ///   3. A refusal MUST leave the books unchanged.
    /// SCENARIOS:
    ///   1. IF the vendor is unknown, a line amount isn't positive, the due date
    ///      is before the bill date, a line names a project the books don't
    ///      have, or the date is in a closed period: SHOULD refuse.
    /// ```
    pub fn create_bill(
        &mut self,
        vendor_id: VendorId,
        date: NaiveDate,
        due_date: NaiveDate,
        lines: Vec<DocumentLine>,
    ) -> Result<BillId, LedgerError> {
        self.create_bill_with(
            vendor_id,
            date,
            due_date,
            lines,
            arap::BillFields::default(),
        )
    }

    /// [`Self::create_bill`] with the bill's new fields (the vendor's
    /// invoice number, terms, receipt date, memo: plan P4b S4.1). The
    /// fields are stored on the bill at issue and never change. All `None`
    /// is exactly [`Self::create_bill`].
    pub fn create_bill_with(
        &mut self,
        vendor_id: VendorId,
        date: NaiveDate,
        due_date: NaiveDate,
        lines: Vec<DocumentLine>,
        fields: arap::BillFields,
    ) -> Result<BillId, LedgerError> {
        self.atomically(|l| l.create_bill_steps(vendor_id, date, due_date, lines, fields))
    }

    /// The steps of [`Self::create_bill`], run inside [`Self::atomically`].
    fn create_bill_steps(
        &mut self,
        vendor_id: VendorId,
        date: NaiveDate,
        due_date: NaiveDate,
        lines: Vec<DocumentLine>,
        fields: arap::BillFields,
    ) -> Result<BillId, LedgerError> {
        if !self.vendors.contains_key(&vendor_id) {
            return Err(LedgerError::UnknownVendor(vendor_id));
        }
        arap::validate_lines(&lines)?;
        arap::validate_due_date(date, due_date)?;
        if let Some(received_on) = fields.received_on {
            crate::calendar::check(received_on)?;
        }
        if let Some(vendor_ref) = fields.vendor_ref.as_deref() {
            self.check_vendor_ref(vendor_id, vendor_ref, fields.duplicate_reason.as_deref())?;
        }
        if let Some(pid) = self.closed_period_containing(date) {
            return Err(LedgerError::ClosedPeriod(pid, date.to_string()));
        }
        let total = arap::document_total(&lines, "the bill")?;
        let parts = arap::document_parts(&lines, "the bill")?;
        self.check_controls(&[GENERAL_EXPENSES, PAYABLES])?;
        let expense = self.ensure_control(&GENERAL_EXPENSES);
        let ap = self.ensure_control(&PAYABLES);
        let id = self.next_bill_id;
        let entry = self.create_entry(date, &format!("Bill {id}"));
        self.add_document_parts(
            entry,
            expense,
            Side::Debit,
            arap::DocumentKind::Bill,
            &parts,
        )?;
        self.add_tagged_line(
            entry,
            ap,
            Side::Credit,
            Money::new(total, self.base_currency),
            arap::DocTag {
                doc: arap::DocRef::Bill(id),
                movement: arap::Movement::Origin,
            },
        )?;
        self.post_entry(entry)?;
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
                owed: Decimal::ZERO,
                status: DocumentStatus::Open,
                entry_id: entry,
                vendor_ref: fields.vendor_ref,
                received_on: fields.received_on,
                terms: fields.terms,
                memo: fields.memo,
            },
        );
        self.tag_vendor(entry, vendor_id);
        self.refresh_doc(arap::DocRef::Bill(id));
        Ok(id)
    }

    /// The number the next invoice is issued under (A-2, B-40): `prefix`,
    /// then `first_number` plus the count of numbered invoices issued so
    /// far, zero-padded to four digits and wider past 9,999 (never
    /// truncated). Stored at issue, never recomputed; a void keeps its
    /// number, so the series never reuses one; a legacy invoice
    /// (`number: None`) is not counted.
    pub fn next_invoice_number(&self, prefix: &str, first_number: u64) -> String {
        let issued = self
            .invoices
            .values()
            .filter(|i| i.number.is_some())
            .count() as u64;
        format!("{prefix}{:04}", first_number.saturating_add(issued))
    }

    /// Refuse a vendor invoice number that already names a standing bill
    /// of the same vendor (B-40), unless `duplicate_reason` says why it is
    /// entered again. Case, spaces and hyphens are ignored: "ACME-123",
    /// "acme 123" and "Acme123" are the same number.
    pub fn check_vendor_ref(
        &self,
        vendor: VendorId,
        vendor_ref: &str,
        duplicate_reason: Option<&str>,
    ) -> Result<(), LedgerError> {
        if duplicate_reason.is_some_and(|r| !r.trim().is_empty()) {
            return Ok(());
        }
        let key = normalize_vendor_ref(vendor_ref);
        if key.is_empty() {
            return Ok(());
        }
        for bill in self.bills.values() {
            if bill.vendor_id != vendor || !self.in_effect(bill.entry_id) {
                continue;
            }
            if let Some(existing) = &bill.vendor_ref
                && normalize_vendor_ref(existing) == key
            {
                return Err(LedgerError::DuplicateVendorRef {
                    vendor,
                    vendor_ref: vendor_ref.to_string(),
                });
            }
        }
        Ok(())
    }

    /// Look up an invoice.
    pub fn get_invoice(&self, id: InvoiceId) -> Option<&Invoice> {
        self.invoices.get(&id)
    }

    /// Look up a bill.
    pub fn get_bill(&self, id: BillId) -> Option<&Bill> {
        self.bills.get(&id)
    }

    /// Every invoice, by id.
    pub fn invoices(&self) -> Vec<&Invoice> {
        let mut v: Vec<&Invoice> = self.invoices.values().collect();
        v.sort_by_key(|i| i.id);
        v
    }

    /// Every bill, by id.
    pub fn bills(&self) -> Vec<&Bill> {
        let mut v: Vec<&Bill> = self.bills.values().collect();
        v.sort_by_key(|b| b.id);
        v
    }

    /// Every document: the invoices by id, then the bills by id.
    pub fn documents(&self) -> Vec<arap::DocRef> {
        self.invoices()
            .into_iter()
            .map(|i| arap::DocRef::Invoice(i.id))
            .chain(self.bills().into_iter().map(|b| arap::DocRef::Bill(b.id)))
            .collect()
    }

    /// The document a ledger line belongs to, if it carries one (rules.md
    /// B-38): `(entry, line index)`.
    pub fn line_doc(&self, entry: EntryId, line: usize) -> Option<arap::DocTag> {
        self.line_docs.get(&(entry, line)).copied()
    }

    /// A document's date, due date, own entry and total; refused for an
    /// unknown document (UnknownInvoice, UnknownBill) or a total past
    /// exact arithmetic.
    fn doc_header(
        &self,
        doc: arap::DocRef,
    ) -> Result<(NaiveDate, NaiveDate, EntryId, Decimal), LedgerError> {
        match doc {
            arap::DocRef::Invoice(id) => {
                let i = self
                    .invoices
                    .get(&id)
                    .ok_or(LedgerError::UnknownInvoice(id))?;
                Ok((i.date, i.due_date, i.entry_id, i.total()?))
            }
            arap::DocRef::Bill(id) => {
                let b = self.bills.get(&id).ok_or(LedgerError::UnknownBill(id))?;
                Ok((b.date, b.due_date, b.entry_id, b.total()?))
            }
        }
    }

    /// Every posted ledger line tagged with `doc`, in date order (posting
    /// order within a day): its origin, payments, and the mirrors of any
    /// of them reversed.
    pub fn doc_lines(&self, doc: arap::DocRef) -> Vec<arap::DocLine> {
        let Some(tagged) = self.doc_index.get(&doc) else {
            return Vec::new();
        };
        let mut out: Vec<arap::DocLine> = tagged
            .iter()
            .filter_map(|&(entry, line)| {
                let e = self.entries.get(&entry)?;
                if e.status() != EntryStatus::Posted {
                    return None;
                }
                let l = e.lines().get(line)?;
                Some(arap::DocLine {
                    entry,
                    line,
                    date: e.date(),
                    account: l.account,
                    movement: self.line_docs.get(&(entry, line))?.movement,
                    amount: doc.counted(l.side, l.amount.amount),
                    reversal: e.reversal_of().is_some(),
                })
            })
            .collect();
        out.sort_by_key(|l| (l.date, self.order_pos.get(&l.entry).copied(), l.line));
        out
    }

    /// What a document is as of a day, read from the ledger lines tagged
    /// with it (rules.md B-38; plan P4b.3, "Derived figures"). `None` for
    /// a document dated after `as_of` (it wasn't issued yet).
    ///
    /// ```text
    /// GOAL: every figure about a document (paid, outstanding, status,
    ///       overdue) is read from its tagged lines, so a reversal, a void
    ///       or a bounced payment is right by construction.
    /// SOURCE: docs/spec/rules.md B-38 (INVARIANTS 1-4), B-9, B-10, B-18;
    ///         ASC 310-10 (receivables), ASC 405-20-40-1 (payables).
    /// INVARIANTS:
    ///   1. outstanding = the signed sum of every posted line tagged with
    ///      it dated on or before `as_of` (a debit to receivables counts
    ///      plus for an invoice; a credit to payables for a bill): the
    ///      very lines the control account sums.
    ///   2. paid (discounted, written off) = minus the same sum over its
    ///      Payment (Discount, WriteOff) lines: a reversed payment counts
    ///      from its reversal's date, never after.
    ///   3. status = arap::doc_status(its own entry stands on `as_of`, its
    ///      own entry's control line carries it, ...).
    ///   4. overdue_days > 0 only while something is owed past the due date.
    ///   5. A pure function of (the books, as_of); NEVER panics:
    ///      TooLargeToCompute past exact arithmetic.
    /// SCENARIOS:
    ///   1. IF an invoice of 100 was paid 100 and the payment reversed:
    ///      SHOULD be Open, owing 100 (scenario 1 of B-38).
    ///   2. IF its own entry was reversed with nothing paid: SHOULD be Void,
    ///      owing nothing.
    ///   3. IF asked between a payment and its reversal: SHOULD be Paid.
    /// ```
    pub fn doc_state(
        &self,
        doc: arap::DocRef,
        as_of: NaiveDate,
    ) -> Result<Option<arap::DocState>, LedgerError> {
        let (date, due_date, entry, total) = self.doc_header(doc)?;
        if date > as_of {
            return Ok(None);
        }
        let too_large = || LedgerError::TooLargeToCompute(format!("{doc}'s figures"));
        let add = |a: Decimal, b: Decimal| a.checked_add(b).ok_or_else(too_large);
        let (mut outstanding, mut paid, mut discounted, mut written_off) =
            (Decimal::ZERO, Decimal::ZERO, Decimal::ZERO, Decimal::ZERO);
        let lines = self.doc_lines(doc);
        let origin_tagged = lines
            .iter()
            .any(|l| l.entry == entry && l.movement == arap::Movement::Origin);
        for l in lines.into_iter().filter(|l| l.date <= as_of) {
            outstanding = add(outstanding, l.amount)?;
            match l.movement {
                arap::Movement::Payment => paid = add(paid, -l.amount)?,
                arap::Movement::Discount => discounted = add(discounted, -l.amount)?,
                arap::Movement::WriteOff => written_off = add(written_off, -l.amount)?,
                arap::Movement::Origin | arap::Movement::Adopted | arap::Movement::RoleMove => {}
            }
        }
        let status = arap::doc_status(
            self.stands_on(entry, as_of),
            origin_tagged,
            total,
            outstanding,
            written_off,
        );
        let overdue_days = if outstanding > Decimal::ZERO && as_of > due_date {
            (as_of - due_date).num_days()
        } else {
            0
        };
        Ok(Some(arap::DocState {
            total,
            paid,
            discounted,
            written_off,
            outstanding,
            status,
            overdue_days,
        }))
    }

    /// [`Self::doc_state`] of an invoice.
    pub fn invoice_state(
        &self,
        id: InvoiceId,
        as_of: NaiveDate,
    ) -> Result<Option<arap::DocState>, LedgerError> {
        self.doc_state(arap::DocRef::Invoice(id), as_of)
    }

    /// [`Self::doc_state`] of a bill.
    pub fn bill_state(
        &self,
        id: BillId,
        as_of: NaiveDate,
    ) -> Result<Option<arap::DocState>, LedgerError> {
        self.doc_state(arap::DocRef::Bill(id), as_of)
    }

    /// The payments a document's events recorded, each once, standing or
    /// not, tagged or not: every `invoice.payment_received` or `bill.paid`
    /// naming it (live or replayed). What a version before document
    /// standing (plan P4b S1) counts as paid, so the store knows when a
    /// payment needs that feature. Refused (TooLargeToCompute), never a
    /// panic, past exact arithmetic: replayed amounts are any size (B-28).
    pub fn payments_recorded(&self, doc: arap::DocRef) -> Result<Decimal, LedgerError> {
        self.payment_events
            .get(&doc)
            .map_or(&[][..], Vec::as_slice)
            .iter()
            .try_fold(Decimal::ZERO, |sum, &a| sum.checked_add(a))
            .ok_or_else(|| LedgerError::TooLargeToCompute(format!("{doc}'s payments")))
    }

    /// The payment entries that stand against a document now: each
    /// payment (an entry reversing nothing) that isn't reversed, once, in
    /// entry order. What a void waits on, and what a bounds refusal names
    /// to reverse first; never a payment's reversal, which stands too but
    /// re-instating the payment is not the remedy.
    pub fn standing_payments(&self, doc: arap::DocRef) -> Vec<EntryId> {
        let mut out: Vec<EntryId> = self
            .doc_lines(doc)
            .iter()
            .filter(|l| {
                l.movement == arap::Movement::Payment && !l.reversal && self.in_effect(l.entry)
            })
            .map(|l| l.entry)
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// A document's movements by date, `extra` ones added: `(date, the
    /// day's movements summed)`, in date order, over the dates in `grid`
    /// (each date once; a date with nothing moving is zero).
    fn doc_deltas(
        &self,
        doc: arap::DocRef,
        extra: &[(NaiveDate, Decimal)],
        grid: &[NaiveDate],
    ) -> Result<Vec<Decimal>, LedgerError> {
        let mut by_date: std::collections::BTreeMap<NaiveDate, Decimal> =
            grid.iter().map(|&d| (d, Decimal::ZERO)).collect();
        let too_large = || LedgerError::TooLargeToCompute(format!("{doc}'s figures"));
        let moves = self
            .doc_lines(doc)
            .into_iter()
            .map(|l| (l.date, l.amount))
            .chain(extra.iter().copied());
        for (date, amount) in moves {
            let day = by_date.entry(date).or_default();
            *day = day.checked_add(amount).ok_or_else(too_large)?;
        }
        Ok(by_date.into_values().collect())
    }

    /// Refuse a step that would move `doc` by `extra` (dated movements)
    /// if, at some date, it would then owe less than nothing or more than
    /// its total where it didn't before ([`kernel::bounds_kept`]; rules.md
    /// B-38 INVARIANT 2). Books an earlier version left out of bounds
    /// (a void under a payment that stands) take any step that makes no
    /// date worse, so the remedy (reversing the payment) is accepted.
    fn check_item_bounds(
        &self,
        doc: arap::DocRef,
        extra: &[(NaiveDate, Decimal)],
    ) -> Result<(), LedgerError> {
        let (_, _, _, total) = self.doc_header(doc)?;
        let mut grid: Vec<NaiveDate> = self
            .doc_lines(doc)
            .iter()
            .map(|l| l.date)
            .chain(extra.iter().map(|&(d, _)| d))
            .collect();
        grid.sort();
        grid.dedup();
        let before = self.doc_deltas(doc, &[], &grid)?;
        let after = self.doc_deltas(doc, extra, &grid)?;
        let (i, below) = match kernel::bounds_kept(total, &before, &after) {
            kernel::Bounds::Within => return Ok(()),
            kernel::Bounds::Below(i) => (i, true),
            kernel::Bounds::Above(i) => (i, false),
            kernel::Bounds::Overflow => {
                return Err(LedgerError::TooLargeToCompute(format!("{doc}'s figures")));
            }
        };
        let owed: Decimal = after[..=i].iter().sum();
        let on = grid[i];
        let payments: Vec<String> = self
            .standing_payments(doc)
            .iter()
            .map(|e| format!("entry {e}"))
            .collect();
        let standing = if payments.is_empty() {
            String::new()
        } else {
            format!(" (payments standing: {})", payments.join(", "))
        };
        Err(LedgerError::DocumentBounds(if below {
            format!(
                "{doc} would owe {owed} on {on}, less than nothing: more would be paid against it than it is for{standing}. Reverse the payment first (books.reverse_entry)"
            )
        } else {
            format!(
                "{doc} would owe {owed} on {on}, more than its total {total}{standing}: reverse the other payment first (books.reverse_entry), then this one's reversal"
            )
        }))
    }

    /// What can still be paid on `doc` by a payment dated `date`: the
    /// least it owes on any day from then on (from its own date, for a
    /// payment dated before it), so no later day is overpaid.
    fn payment_room(&self, doc: arap::DocRef, date: NaiveDate) -> Result<Decimal, LedgerError> {
        let (doc_date, ..) = self.doc_header(doc)?;
        let from = date.max(doc_date);
        let lines = self.doc_lines(doc);
        let too_large = || LedgerError::TooLargeToCompute(format!("{doc}'s figures"));
        let mut running = Decimal::ZERO;
        for l in lines.iter().filter(|l| l.date <= from) {
            running = running.checked_add(l.amount).ok_or_else(too_large)?;
        }
        let mut room = running;
        for l in lines.iter().filter(|l| l.date > from) {
            running = running.checked_add(l.amount).ok_or_else(too_large)?;
            room = room.min(running);
        }
        Ok(room)
    }

    /// A payment's deposit or payer account: a postable asset or liability
    /// that holds no role (a bank, a card, a member's payable), never a
    /// control, revenue or expense account (plan P4b.3, "Payment
    /// accounts"). Receivables and payables are held as the control guard
    /// reads them ([`Self::control_role_of`]); the other roles by
    /// [`Self::holds_role`], so a chart's own card at 2000, or a bank at a
    /// payroll role's code, pays.
    fn check_payment_account(&self, account: AccountId) -> Result<(), LedgerError> {
        self.check_line_account(account)?;
        let a = self.chart.get(account).expect("checked above");
        let refuse = |why: String| {
            Err(LedgerError::PaymentAccount {
                code: a.code.clone(),
                name: a.name.clone(),
                why,
            })
        };
        if !matches!(a.account_type, AccountType::Asset | AccountType::Liability) {
            return refuse(format!(
                "it is {} account, not cash, a card or a payable",
                match a.account_type {
                    AccountType::Equity => "an equity",
                    AccountType::Revenue => "a revenue",
                    _ => "an expense",
                }
            ));
        }
        if let Some(c) = self.control_role_of(account).or_else(|| {
            control_accounts().find(|c| {
                c.key != RECEIVABLES.key && c.key != PAYABLES.key && self.holds_role(c, account)
            })
        }) {
            return refuse(format!("it holds the {} role", c.role));
        }
        Ok(())
    }

    /// Whether `account` fills role `c`: assigned to it, or (unassigned)
    /// the account at the role's code, of the role's type and postable,
    /// so the commands posting the role post there. An account of
    /// another type at the code fills nothing (the commands refuse it:
    /// [`Self::check_controls`]).
    fn holds_role(&self, c: &ControlAccount, account: AccountId) -> bool {
        match self.roles.get(c.key) {
            Some(&a) => a == account,
            None => self.chart.find_by_code(c.code).is_some_and(|a| {
                a.id == account && a.account_type == c.account_type && !a.placeholder
            }),
        }
    }

    /// Record a customer payment against an invoice. Posts: Dr Cash,
    /// Cr A/R ([`Self::receive_payment_with`], with no payment method or
    /// reference).
    pub fn receive_payment(
        &mut self,
        invoice_id: InvoiceId,
        date: NaiveDate,
        amount: Decimal,
        cash_account: AccountId,
    ) -> Result<EntryId, LedgerError> {
        self.receive_payment_with(
            invoice_id,
            date,
            amount,
            cash_account,
            &arap::PaymentDetails::default(),
        )
    }

    /// Record a customer payment against an invoice, with its payment
    /// method and reference. Posts: Dr the deposit account, Cr A/R.
    ///
    /// ```text
    /// GOAL: record a customer's payment against an invoice.
    /// SOURCE: IRS Pub. 538 (accrual); CONVENTION (A/R workflow); rules.md
    ///         B-38; plan/served.md P4b.3 ("Payment accounts"), P4b.6.
    /// INVARIANTS:
    ///   1. MUST post Dr the deposit account, Cr 1200 receivables, for the
    ///      amount; the receivables line carries the invoice (Payment), so
    ///      what it owes falls by the amount from the payment's date.
    ///   2. The payment method, when given, tags the entry in the same
    ///      step (D-40); the reference is in its description.
    ///   3. A refusal MUST leave the books unchanged.
    /// SCENARIOS:
    ///   1. IF the amount isn't positive: SHOULD refuse.
    ///   2. IF it exceeds what's still owed on any day from its date on:
    ///      - SHOULD refuse (Overpayment), saying what can be paid
    ///      - NEVER let a standing payment exceed the total
    ///      - a reversed payment no longer counts, so the right one is
    ///        accepted after the wrong one is reversed (B-38 scenario 5)
    ///   3. IF the deposit account is a control, revenue, expense or
    ///      equity account, or holds any role: SHOULD refuse
    ///      (PaymentAccount; B-38 scenario 8).
    ///   4. IF dated before the invoice: SHOULD refuse (PaymentBeforeDocument).
    /// ```
    pub fn receive_payment_with(
        &mut self,
        invoice_id: InvoiceId,
        date: NaiveDate,
        amount: Decimal,
        cash_account: AccountId,
        details: &arap::PaymentDetails,
    ) -> Result<EntryId, LedgerError> {
        self.atomically(|l| {
            l.payment_steps(
                arap::DocRef::Invoice(invoice_id),
                date,
                amount,
                cash_account,
                details,
            )
        })
    }

    /// Pay a vendor bill. Posts: Dr A/P, Cr Cash ([`Self::pay_bill_with`],
    /// with no payment method or reference).
    pub fn pay_bill(
        &mut self,
        bill_id: BillId,
        date: NaiveDate,
        amount: Decimal,
        cash_account: AccountId,
    ) -> Result<EntryId, LedgerError> {
        self.pay_bill_with(
            bill_id,
            date,
            amount,
            cash_account,
            &arap::PaymentDetails::default(),
        )
    }

    /// Pay a vendor bill, with its payment method and reference. Posts:
    /// Dr A/P, Cr the payer account.
    ///
    /// ```text
    /// GOAL: record paying a vendor's bill.
    /// SOURCE: IRS Pub. 538 (accrual); CONVENTION (A/P workflow); rules.md
    ///         B-38; plan/served.md P4b.3, P4b.6.
    /// INVARIANTS:
    ///   1. MUST post Dr 2000 payables, Cr the payer account, for the
    ///      amount, tagged with the vendor; the payables line carries the
    ///      bill (Payment).
    ///   2. The payment method, when given, tags the entry in the same
    ///      step (a card-paid bill is left out of 1099-NEC, D-40).
    ///   3. A refusal MUST leave the books unchanged.
    /// SCENARIOS:
    ///   1. IF the amount isn't positive, or exceeds what's still owed on
    ///      any day from its date on: SHOULD refuse.
    ///   2. IF dated before the bill: SHOULD refuse (PaymentBeforeDocument);
    ///      the bill's own date is fine (DECISION, owner 2026-10-13).
    ///   3. IF the payer account isn't a bank, card or payable that holds
    ///      no role: SHOULD refuse (PaymentAccount).
    /// ```
    pub fn pay_bill_with(
        &mut self,
        bill_id: BillId,
        date: NaiveDate,
        amount: Decimal,
        cash_account: AccountId,
        details: &arap::PaymentDetails,
    ) -> Result<EntryId, LedgerError> {
        self.atomically(|l| {
            l.payment_steps(
                arap::DocRef::Bill(bill_id),
                date,
                amount,
                cash_account,
                details,
            )
        })
    }

    /// The steps of a payment ([`Self::receive_payment_with`],
    /// [`Self::pay_bill_with`]), run inside [`Self::atomically`].
    fn payment_steps(
        &mut self,
        doc: arap::DocRef,
        date: NaiveDate,
        amount: Decimal,
        account: AccountId,
        details: &arap::PaymentDetails,
    ) -> Result<EntryId, LedgerError> {
        if amount <= Decimal::ZERO {
            return Err(LedgerError::InvalidPaymentAmount(amount));
        }
        let (doc_date, ..) = self.doc_header(doc)?;
        let room = self.payment_room(doc, date)?;
        if amount > room {
            return Err(LedgerError::Overpayment(amount, room));
        }
        if let Some(pid) = self.closed_period_containing(date) {
            return Err(LedgerError::ClosedPeriod(pid, date.to_string()));
        }
        if date < doc_date {
            return Err(LedgerError::PaymentBeforeDocument {
                date,
                document_date: doc_date,
            });
        }
        details.check()?;
        if let Some(i) = details.instrument
            && !self.instruments.contains_key(&i)
        {
            return Err(LedgerError::UnknownInstrument(i));
        }
        let role = match doc {
            arap::DocRef::Invoice(_) => RECEIVABLES,
            arap::DocRef::Bill(_) => PAYABLES,
        };
        self.check_controls(&[role])?;
        // Every check before the entry exists: a refusal leaves no draft.
        self.check_payment_account(account)?;
        let control = self.ensure_control(&role);
        let money = Money::new(amount, self.base_currency);
        let tag = arap::DocTag {
            doc,
            movement: arap::Movement::Payment,
        };
        let entry = match doc {
            arap::DocRef::Invoice(id) => {
                let entry = self.create_entry(
                    date,
                    &details.description(&format!("Payment on invoice {id}")),
                );
                self.add_line(entry, account, Side::Debit, money)?;
                self.add_tagged_line(entry, control, Side::Credit, money, tag)?;
                entry
            }
            arap::DocRef::Bill(id) => {
                let entry =
                    self.create_entry(date, &details.description(&format!("Payment on bill {id}")));
                self.add_tagged_line(entry, control, Side::Debit, money, tag)?;
                self.add_line(entry, account, Side::Credit, money)?;
                entry
            }
        };
        self.post_entry(entry)?;
        match doc {
            arap::DocRef::Invoice(id) => {
                let customer = self.invoices[&id].customer_id;
                self.tag_customer(entry, customer);
            }
            arap::DocRef::Bill(id) => {
                let vendor = self.bills[&id].vendor_id;
                self.tag_vendor(entry, vendor);
            }
        }
        if details.instrument.is_some() {
            self.set_entry_instrument(entry, details.instrument)?;
        }
        self.payment_events.entry(doc).or_default().push(amount);
        self.refresh_doc(doc);
        Ok(entry)
    }

    /// Void a document: reverse its own entry (B-9), described "Void
    /// invoice 7: reason". It keeps its id (and, later, its number); its
    /// status as of the void's date on is Void. Plain
    /// [`Self::reverse_entry`] of the same entry gives the same result.
    ///
    /// ```text
    /// GOAL: a document that should never have existed (a duplicate, the
    ///       wrong customer) is undone by an opposite entry, never erased.
    /// SOURCE: docs/spec/rules.md B-9, B-10, B-12, B-38 (scenarios 2, 3);
    ///         plan/served.md P4b.5, P4b.6; ASC 250 (an immaterial error is
    ///         corrected in the period found).
    /// INVARIANTS:
    ///   1. MUST post exactly the reversal reverse_entry would, of the
    ///      entry whose reversal voids it (its own entry, or the last
    ///      reversal in its chain when it was voided and re-instated).
    ///   2. A refusal MUST leave the books unchanged.
    /// SCENARIOS:
    ///   1. IF the reason is blank, more than one line, or longer than 200
    ///      characters: SHOULD refuse (InvalidVoid).
    ///   2. IF it is void already: SHOULD refuse (InvalidVoid), naming the
    ///      reversal (reverse that to re-instate it).
    ///   3. IF a payment stands against it: SHOULD refuse (DocumentBounds),
    ///      naming the payment to reverse first.
    ///   4. IF dated before the document, or in a closed period: SHOULD
    ///      refuse (check_reversal).
    /// ```
    pub fn void_document(
        &mut self,
        doc: arap::DocRef,
        date: NaiveDate,
        reason: &str,
    ) -> Result<EntryId, LedgerError> {
        let (target, description) = self.check_void_document(doc, date, reason)?;
        self.reverse_entry(target, date, &description)
    }

    /// Every refusal of [`Self::void_document`], without touching the
    /// books: the entry to reverse and the reversal's description.
    pub fn check_void_document(
        &self,
        doc: arap::DocRef,
        date: NaiveDate,
        reason: &str,
    ) -> Result<(EntryId, String), LedgerError> {
        let reason = reason.trim();
        if reason.is_empty()
            || reason.contains(['\n', '\r'])
            || reason.chars().count() > arap::MAX_REFERENCE_CHARS
        {
            return Err(LedgerError::InvalidVoid(format!(
                "a void needs its reason: one line of at most {} characters",
                arap::MAX_REFERENCE_CHARS
            )));
        }
        let (_, _, origin, _) = self.doc_header(doc)?;
        let chain = self.reversal_chain(origin);
        let target = *chain.last().unwrap_or(&origin);
        if !self.in_effect(origin) {
            return Err(LedgerError::InvalidVoid(format!(
                "{doc} is void already: entry {target} reversed it (reverse that entry to re-instate it)"
            )));
        }
        self.check_reversal(target, date)?;
        Ok((target, format!("Void {doc}: {reason}")))
    }

    /// The control accounts' tie-out as of a day (plan P4b.10,
    /// `books.control_check`): for each control account
    /// ([`Self::control_role_of`]: assigned receivables or payables, or
    /// holding a document's lines), its balance, what its documents owe
    /// on it, its lines that carry no document, and (on the role's first
    /// account) the role's documents needing attention.
    ///
    /// ```text
    /// GOAL: show that receivables and payables equal their documents, and
    ///       list every line that is no document's.
    /// SOURCE: docs/spec/rules.md B-23, B-38 INVARIANT 1:
    ///         GL(A, D) = Σ_docs outstanding(d, D) + unattached(A, D).
    /// INVARIANTS:
    ///   1. balance sums every posted line on the account dated on or
    ///      before `as_of`; documents_total sums the tagged ones through
    ///      their documents (Self::doc_lines); unattached lists the rest,
    ///      each reversal chain once, at what its lines net to as of the
    ///      day (a reversed hand line nets to nothing and is not listed).
    ///   2. difference = balance - documents_total - Σ unattached is zero
    ///      in every books (an identity: both sides sum the same lines).
    ///   3. attention lists each document of the role issued by `as_of`
    ///      whose state then is out of bounds (a credit balance, which
    ///      aging leaves out, or more owed than its total) or Unattached:
    ///      what books an earlier version wrote can hold (plan P4b.14
    ///      item 3), never what these commands write.
    ///   4. NEVER panics: TooLargeToCompute past exact arithmetic.
    /// ```
    pub fn control_check(&self, as_of: NaiveDate) -> Result<Vec<arap::ControlCheck>, LedgerError> {
        let too_large = || LedgerError::TooLargeToCompute("a control account's balance".into());
        // Each control account (Self::control_role_of), and the role
        // whose documents it holds: a debit counts plus on receivables, a
        // credit on payables.
        let mut accounts: Vec<(AccountId, &'static ControlAccount)> = [&RECEIVABLES, &PAYABLES]
            .into_iter()
            .flat_map(|c| {
                self.document_role_accounts(c)
                    .into_iter()
                    .map(move |a| (a, c))
            })
            .collect();
        let mut doc_totals: HashMap<AccountId, Decimal> = HashMap::new();
        let mut attention: HashMap<&'static str, Vec<arap::DocAttention>> = HashMap::new();
        for doc in self.documents() {
            let role = match doc {
                arap::DocRef::Invoice(_) => &RECEIVABLES,
                arap::DocRef::Bill(_) => &PAYABLES,
            };
            for l in self.doc_lines(doc).into_iter().filter(|l| l.date <= as_of) {
                let t = doc_totals.entry(l.account).or_default();
                *t = t.checked_add(l.amount).ok_or_else(too_large)?;
            }
            if let Some(st) = self.doc_state(doc, as_of)?
                && (st.out_of_bounds() || st.status == arap::DocStatus::Unattached)
            {
                attention
                    .entry(role.key)
                    .or_default()
                    .push(arap::DocAttention {
                        doc,
                        status: st.status,
                        total: st.total,
                        outstanding: st.outstanding,
                    });
            }
        }
        // A role whose documents carry no line anywhere (each unattached)
        // still gets its row, on the account the role posts to now.
        for c in [&RECEIVABLES, &PAYABLES] {
            if attention.contains_key(c.key)
                && !accounts.iter().any(|&(_, r)| r.key == c.key)
                && let Some(a) = self.role_account(c)
            {
                accounts.push((a, c));
            }
        }
        let counted = |role: &ControlAccount, side: Side, amount: Decimal| {
            let plus = if role.key == RECEIVABLES.key {
                Side::Debit
            } else {
                Side::Credit
            };
            if side == plus { amount } else { -amount }
        };
        let mut out = Vec::new();
        for (account, role) in accounts {
            let mut balance = Decimal::ZERO;
            let mut tops: Vec<(EntryId, usize)> = Vec::new();
            for e in self.posted_entries().filter(|e| e.date() <= as_of) {
                for (i, l) in e.lines().iter().enumerate() {
                    if l.account != account {
                        continue;
                    }
                    balance = balance
                        .checked_add(counted(role, l.side, l.amount.amount))
                        .ok_or_else(too_large)?;
                    if !self.line_docs.contains_key(&(e.id(), i)) {
                        let top = self.reversal_chain(e.id())[0];
                        if !tops.contains(&(top, i)) {
                            tops.push((top, i));
                        }
                    }
                }
            }
            let mut unattached = Vec::new();
            let mut unattached_sum = Decimal::ZERO;
            for (top, i) in tops {
                // The chain's lines dated by `as_of`, each the mirror of
                // the one before: they net to k times the top's line.
                let mut net = Decimal::ZERO;
                for (n, e) in self.reversal_chain(top).into_iter().enumerate() {
                    let Some(e) = self.entries.get(&e).filter(|e| e.date() <= as_of) else {
                        continue;
                    };
                    let Some(l) = e.lines().get(i) else {
                        continue;
                    };
                    debug_assert!(n == 0 || e.reversal_of().is_some());
                    net = net
                        .checked_add(counted(role, l.side, l.amount.amount))
                        .ok_or_else(too_large)?;
                }
                if !net.is_zero() {
                    unattached_sum = unattached_sum.checked_add(net).ok_or_else(too_large)?;
                    unattached.push(arap::UnattachedLine {
                        entry: top,
                        line: i,
                        date: self.entries[&top].date(),
                        amount: net,
                    });
                }
            }
            let documents_total = doc_totals.get(&account).copied().unwrap_or_default();
            let difference = balance
                .checked_sub(documents_total)
                .and_then(|d| d.checked_sub(unattached_sum))
                .ok_or_else(too_large)?;
            out.push(arap::ControlCheck {
                role: role.key.to_string(),
                account,
                balance,
                documents_total,
                unattached,
                difference,
                attention: attention.remove(role.key).unwrap_or_default(),
            });
        }
        Ok(out)
    }

    /// Refresh a document's record figures ([`Invoice::amount_paid`],
    /// [`Invoice::owed`], [`Invoice::status`]) from its tagged lines as of
    /// the end of the books: what was settled (paid, discounted, written
    /// off, as they stand), what is owed, and the three-word status of
    /// what was settled. A void settles nothing, so it never reads as
    /// paid. Never refuses: a document whose figures can't be computed
    /// keeps the last ones (its state refuses them, `doc_state`).
    fn refresh_doc(&mut self, doc: arap::DocRef) {
        let Ok(Some(st)) = self.doc_state(doc, NaiveDate::MAX) else {
            return;
        };
        let Some(settled) = st
            .paid
            .checked_add(st.discounted)
            .and_then(|s| s.checked_add(st.written_off))
        else {
            return;
        };
        // Nothing settled is zero as the record always started it (a
        // sum's scale would show as "0.00"); what is owed keeps the
        // lines' scale, as the total less the settled did ("0.00").
        let settled = if settled.is_zero() {
            Decimal::ZERO
        } else {
            settled
        };
        let owed = st.outstanding;
        let status = arap::derive_status(settled, st.total);
        match doc {
            arap::DocRef::Invoice(id) => {
                if let Some(i) = self.invoices.get_mut(&id) {
                    (i.amount_paid, i.owed, i.status) = (settled, owed, status);
                }
            }
            arap::DocRef::Bill(id) => {
                if let Some(b) = self.bills.get_mut(&id) {
                    (b.amount_paid, b.owed, b.status) = (settled, owed, status);
                }
            }
        }
    }

    /// [`Self::refresh_doc`] for every document `entry`'s lines carry.
    fn refresh_docs_of(&mut self, entry: EntryId) {
        let docs: Vec<arap::DocRef> = self
            .line_docs
            .range((entry, 0)..(entry + 1, 0))
            .map(|(_, t)| t.doc)
            .collect();
        for d in docs {
            self.refresh_doc(d);
        }
    }

    /// On replay, tag a document's control line in `entry` (plan P4b.3,
    /// "How tags get set"): the first line, in line order, on the role's
    /// account as the role stands at this point of the journal, on the
    /// side the movement posts (the owed side for the document's own
    /// line, the other for a payment), that carries no document yet and
    /// is for `amount` (any amount when `None`: a document's own entry
    /// has one such line). An entry with
    /// no such line stays untagged, and its line is listed by
    /// [`Self::control_check`]: replay never refuses (B-28).
    fn tag_replayed(
        &mut self,
        entry: EntryId,
        role: &ControlAccount,
        amount: Option<Decimal>,
        tag: arap::DocTag,
    ) {
        let (Some(account), Some(e)) = (self.role_account(role), self.entries.get(&entry)) else {
            return;
        };
        // The side the movement posts: a document's own line raises what
        // it owes, a payment lowers it. An older payment deposited into
        // the control account itself (Dr 1200 / Cr 1200, plan P4b.2 gap
        // 5) has a line on each side; only the right one is the payment.
        let side = match tag.movement {
            arap::Movement::Origin => tag.doc.owed_side(),
            _ => kernel::flip(tag.doc.owed_side()),
        };
        let at = e.lines().iter().enumerate().position(|(i, l)| {
            l.account == account
                && l.side == side
                && !self.line_docs.contains_key(&(entry, i))
                && amount.is_none_or(|a| a == l.amount.amount)
        });
        if let Some(i) = at {
            // A document's own entry has one control line: two would be no
            // entry an earlier version wrote, so neither is tagged.
            let lines_on = e.lines().iter().filter(|l| l.account == account).count();
            if amount.is_some() || lines_on == 1 {
                self.tag_line(entry, i, tag);
            }
        }
    }

    /// Record an invoice without posting: replay support for journaling
    /// stores. The invoice's entry was already rebuilt from its own
    /// `entry.created` / `entry.line_added` / `entry.posted` events; this
    /// restores the invoice record and tags its receivable line (the one
    /// line on the receivables role's account as it stands at this point
    /// of the journal). The id must be the next invoice id — anything else
    /// means the journal and the allocator disagree.
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
        // Restored exactly as journaled: replay checks no line or date
        // (none did when it was written; rules.md B-28). Figures derived
        // from them (the total, aging) refuse what they can't compute.
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
                owed: Decimal::ZERO,
                status: DocumentStatus::Open,
                entry_id,
                number: None,
                terms: None,
                bill_to: None,
                seller: None,
                customer_ref: None,
                memo: None,
                sends: Vec::new(),
            },
        );
        self.tag_customer(entry_id, customer_id);
        let doc = arap::DocRef::Invoice(id);
        self.tag_replayed(
            entry_id,
            &RECEIVABLES,
            None,
            arap::DocTag {
                doc,
                movement: arap::Movement::Origin,
            },
        );
        self.refresh_doc(doc);
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
        // Restored exactly as journaled: replay checks no line or date
        // (none did when it was written; rules.md B-28). Figures derived
        // from them (the total, aging) refuse what they can't compute.
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
                owed: Decimal::ZERO,
                status: DocumentStatus::Open,
                entry_id,
                vendor_ref: None,
                received_on: None,
                terms: None,
                memo: None,
            },
        );
        self.tag_vendor(entry_id, vendor_id);
        let doc = arap::DocRef::Bill(id);
        self.tag_replayed(
            entry_id,
            &PAYABLES,
            None,
            arap::DocTag {
                doc,
                movement: arap::Movement::Origin,
            },
        );
        self.refresh_doc(doc);
        Ok(())
    }

    /// Apply an invoice payment without posting: replay support for
    /// journaling stores. The payment entry was already rebuilt from its
    /// own entry events; this tags its receivables line with the invoice
    /// (the first untagged one for `amount`, so several payment events
    /// naming one entry tag its lines in order: the ordinal rule, plan
    /// P4b.3). Nothing is bounded on replay (rules.md B-28).
    #[doc(hidden)]
    pub fn replay_invoice_payment(
        &mut self,
        invoice_id: InvoiceId,
        amount: Decimal,
        entry_id: EntryId,
    ) -> Result<(), LedgerError> {
        let customer = self
            .invoices
            .get(&invoice_id)
            .ok_or(LedgerError::UnknownInvoice(invoice_id))?
            .customer_id;
        let doc = arap::DocRef::Invoice(invoice_id);
        self.tag_replayed(
            entry_id,
            &RECEIVABLES,
            Some(amount),
            arap::DocTag {
                doc,
                movement: arap::Movement::Payment,
            },
        );
        self.tag_customer(entry_id, customer);
        self.payment_events.entry(doc).or_default().push(amount);
        self.refresh_doc(doc);
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
        let vendor = self
            .bills
            .get(&bill_id)
            .ok_or(LedgerError::UnknownBill(bill_id))?
            .vendor_id;
        let doc = arap::DocRef::Bill(bill_id);
        self.tag_replayed(
            entry_id,
            &PAYABLES,
            Some(amount),
            arap::DocTag {
                doc,
                movement: arap::Movement::Payment,
            },
        );
        self.tag_vendor(entry_id, vendor);
        self.payment_events.entry(doc).or_default().push(amount);
        self.refresh_doc(doc);
        Ok(())
    }

    /// A/R aging as of a date: one row per customer with an outstanding
    /// balance, bucketed by days past due. Refused (TooLargeToCompute),
    /// never a panic, when a sum is past exact arithmetic: documents are
    /// restored as journaled (rules.md B-28).
    ///
    /// ```text
    /// GOAL: what each customer owes as of a day, by how overdue.
    /// SOURCE: docs/spec/rules.md B-18, B-38 INVARIANT 5.
    /// INVARIANTS:
    ///   1. Each invoice issued by `as_of` that owes something then
    ///      (Self::doc_state's outstanding) is in exactly one bucket, at
    ///      that amount: the buckets partition what the invoices owe.
    ///   2. A void invoice, a paid one, and one dated after `as_of` are
    ///      left out; a payment reversed after `as_of` still counts as
    ///      paid, one reversed by then doesn't.
    ///   3. One owing less than nothing (a credit balance, in books an
    ///      earlier version left out of bounds) is left out too: it is
    ///      owed back, not receivable (ASC 310-10), and
    ///      Self::control_check lists it. So the buckets sum to what the
    ///      invoices owing something owe: the documents' total less those
    ///      credit balances.
    /// ```
    pub fn ar_aging(&self, as_of: NaiveDate) -> Result<Vec<AgingRow>, LedgerError> {
        self.aging(as_of, true)
    }

    /// A/P aging as of a date: one row per vendor with an outstanding
    /// balance, bucketed by days past due; as [`Self::ar_aging`] is.
    pub fn ap_aging(&self, as_of: NaiveDate) -> Result<Vec<AgingRow>, LedgerError> {
        self.aging(as_of, false)
    }

    /// [`Self::ar_aging`] (`invoices`) or [`Self::ap_aging`].
    fn aging(&self, as_of: NaiveDate, invoices: bool) -> Result<Vec<AgingRow>, LedgerError> {
        let mut by_party: HashMap<u64, AgingRow> = HashMap::new();
        for doc in self.documents() {
            let (party, name) = match doc {
                arap::DocRef::Invoice(id) if invoices => {
                    let c = self.invoices[&id].customer_id;
                    (c, self.customers.get(&c).map(|c| c.name.clone()))
                }
                arap::DocRef::Bill(id) if !invoices => {
                    let v = self.bills[&id].vendor_id;
                    (v, self.vendors.get(&v).map(|v| v.name.clone()))
                }
                _ => continue,
            };
            let Some(st) = self.doc_state(doc, as_of)? else {
                continue;
            };
            // Nothing owed, or a credit balance (Overpaid, or a void under
            // a payment that stands): no receivable or payable to age; the
            // control check lists the credit balances.
            if st.outstanding <= Decimal::ZERO {
                continue;
            }
            let (_, due_date, ..) = self.doc_header(doc)?;
            let row = arap::aging_bucket(due_date, as_of, st.outstanding);
            by_party
                .entry(party)
                .or_insert_with(|| AgingRow {
                    name: name.unwrap_or_default(),
                    current: Decimal::ZERO,
                    days_30: Decimal::ZERO,
                    days_60: Decimal::ZERO,
                    days_90: Decimal::ZERO,
                    days_90_plus: Decimal::ZERO,
                })
                .add(&row)?;
        }
        let mut rows: Vec<AgingRow> = by_party.into_values().collect();
        rows.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(rows)
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
        // A statement says what an account holds: no more than a line may
        // move, so the difference is exact (P0 review round 3, S1).
        if !kernel::Amount::in_line_range(statement_balance) {
            return Err(LedgerError::ValueOutOfRange(format!(
                "statement balance {statement_balance}"
            )));
        }
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
            difference: cleared.checked_sub(statement_balance).ok_or_else(|| {
                LedgerError::TooLargeToCompute("the reconciliation's difference".into())
            })?,
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
        // A line that names its employee names a party with that role.
        for line in lines {
            if let Some(id) = line.employee {
                let p = self.parties.get(&id).ok_or(LedgerError::UnknownParty(id))?;
                if !p.roles.contains(&crate::Role::Employee) {
                    return Err(LedgerError::InvalidParty(format!(
                        "{} isn't an employee (give the party the employee role first)",
                        p.label()
                    )));
                }
            }
        }
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
    ///   3. When every line with net pay names its employee party, the entry
    ///      names those parties (Self::entry_parties), each with its net pay
    ///      (summed over its lines): the whole of it for one employee, else
    ///      an exact amount each, so each employee's books.party counts what
    ///      they were paid. Lines named by employee_name only (or shares
    ///      Self::set_entry_parties would refuse) leave the entry naming no
    ///      party (the agent tags it).
    /// SCENARIOS:
    ///   1. IF any amount is negative, gross is zero, or withholdings exceed
    ///      gross: SHOULD refuse.
    ///   2. Pat (net 1,500.00) and Sam (net 900.00), both parties: the entry
    ///      names Pat 1,500.00 and Sam 900.00.
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
            payroll_entry_plan(&lines, accounts, cash_account, self.base_currency)?
        {
            self.add_line(entry, account, side, amount)?;
        }
        self.post_entry(entry)?;
        self.name_payroll_parties(entry, &lines)?;
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

    /// [`Self::run_payroll`] INVARIANT 3: name the employees a payroll
    /// entry paid as its parties, each with their net pay. Live and on
    /// replay ([`Self::replay_payroll_run`]) alike.
    fn name_payroll_parties(
        &mut self,
        entry: EntryId,
        lines: &[PayrollLine],
    ) -> Result<(), LedgerError> {
        let mut paid: Vec<(crate::PartyId, Decimal)> = Vec::new();
        let mut unnamed = false;
        for l in lines {
            // A run is restored as journaled (rules.md B-28): its figures
            // are checked, and a net past exact arithmetic refused.
            let net = l.net_pay()?;
            if net.is_zero() {
                continue;
            }
            match l.employee {
                Some(p) => match paid.iter_mut().find(|(x, _)| *x == p) {
                    Some((_, v)) => {
                        *v = v.checked_add(net).ok_or_else(|| {
                            LedgerError::TooLargeToCompute(format!("party {p}'s net pay"))
                        })?
                    }
                    None => paid.push((p, net)),
                },
                None => unnamed = true,
            }
        }
        if !unnamed && !paid.is_empty() {
            let shares: Vec<crate::PartyShare> = match paid.as_slice() {
                [(p, _)] => vec![crate::PartyShare::whole(*p)],
                _ => paid
                    .iter()
                    .map(|&(party, v)| crate::PartyShare {
                        party,
                        share: crate::Share::Amount(v),
                    })
                    .collect(),
            };
            // Shares set_entry_parties would refuse (two employees with
            // vendor records) leave the entry for the agent to tag.
            if self.check_set_entry_parties(entry, &shares).is_ok() {
                self.set_entry_parties(entry, &shares)?;
            }
        }
        Ok(())
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
        // Restored exactly as journaled: the run's figures are checked by
        // the command, not by replay (rules.md B-28).
        self.next_payroll_run_id += 1;
        if self.entries.contains_key(&entry_id) {
            self.name_payroll_parties(entry_id, &lines)?;
        }
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
    // Settings, conventions, decisions, open items, approvals
    // ------------------------------------------------------------------

    /// The settings the journal holds ([`crate::settings`]).
    pub fn settings(&self) -> &crate::settings::Settings {
        &self.settings
    }

    /// Check a settings change before it is journaled: every value
    /// canonical, changes that change nothing left out, the result whole
    /// (crate::settings INVARIANTS 1, 2, 4), and the fiscal year never
    /// moved under a closed period. Changes nothing.
    pub fn check_settings(
        &self,
        changes: &[crate::settings::Change],
        source: &crate::settings::Source,
    ) -> Result<Vec<crate::settings::Change>, LedgerError> {
        self.check_settings_adopting(changes, source, false)
    }

    /// [`Self::check_settings`] for books moving a settings file in
    /// (`adopting`): their first `settings.set` takes the fiscal year they
    /// were kept under (the import proves it is the file's), so it moves
    /// nothing, closed periods or not.
    pub fn check_settings_adopting(
        &self,
        changes: &[crate::settings::Change],
        source: &crate::settings::Source,
        adopting: bool,
    ) -> Result<Vec<crate::settings::Change>, LedgerError> {
        let made = self
            .settings
            .check_changes(changes, source)
            .map_err(LedgerError::InvalidSetting)?;
        // The fiscal year can't move under closed periods: they were
        // closed by it, and the year's earnings would restate (a tax year
        // changes only with the IRS's consent, Form 1128). Books moving a
        // settings file in take the year they were kept under.
        if !(adopting && !self.settings.is_journaled())
            && made.iter().any(|c| c.key == "company.fiscal_year_start")
        {
            let mut next = self.settings.clone();
            next.apply(&made, source, 0)
                .map_err(LedgerError::InvalidSetting)?;
            let (before, after) = (
                self.settings.company("").fiscal_year_start,
                next.company("").fiscal_year_start,
            );
            if before != after
                && self
                    .periods
                    .values()
                    .any(|p| p.status == crate::PeriodStatus::Closed)
            {
                return Err(LedgerError::InvalidSetting(format!(
                    "company.fiscal_year_start can't change from {:02}-{:02} to {:02}-{:02}: periods are closed under it (a changed tax year needs the IRS's consent, Form 1128; ask the CPA)",
                    before.0, before.1, after.0, after.1
                )));
            }
        }
        Ok(made)
    }

    /// Apply a journaled settings change (`settings.set` at `seq`): as
    /// checked when made, or as replayed (shape only, rules.md B-28).
    pub fn apply_settings(
        &mut self,
        changes: &[crate::settings::Change],
        source: &crate::settings::Source,
        seq: u64,
    ) -> Result<(), LedgerError> {
        self.settings
            .apply(changes, source, seq)
            .map_err(LedgerError::InvalidSetting)
    }

    /// Conventions, decisions, open items and approvals.
    pub fn conventions(&self) -> &crate::conventions::Conventions {
        &self.conventions
    }

    /// The facts recorded about the books' files, by attachment id
    /// (`attachment.annotated`, plan P6 step 5).
    pub fn annotations(&self) -> impl Iterator<Item = &crate::evidence::Annotation> {
        self.annotations.values()
    }

    /// The facts recorded about one file, if any.
    pub fn annotation(&self, attachment_id: u64) -> Option<&crate::evidence::Annotation> {
        self.annotations.get(&attachment_id)
    }

    /// The next receipt number: one past the highest one annotated, or 1
    /// when none is (plan P6 step 5: engine numbers run from the max).
    pub fn next_receipt_number(&self) -> u32 {
        self.annotations
            .values()
            .filter_map(|a| a.fields.receipt_number)
            .max()
            .map_or(1, |n| n.saturating_add(1))
    }

    /// Record what a human read off a file, replacing an earlier
    /// annotation of it (plan P6 step 5; INVARIANTS 1 and 2 of
    /// [`crate::evidence`]).
    pub fn annotate_attachment(
        &mut self,
        annotation: crate::evidence::Annotation,
    ) -> Result<(), LedgerError> {
        let annotation = annotation.checked()?;
        self.annotations
            .insert(annotation.attachment_id, annotation);
        Ok(())
    }

    /// [`Self::annotate_attachment`] as journaled (`attachment.annotated`),
    /// for replay only (rules.md B-28).
    pub fn replay_annotate_attachment(
        &mut self,
        annotation: crate::evidence::Annotation,
    ) -> Result<(), LedgerError> {
        self.annotate_attachment(annotation)
    }

    /// The entries `entry` records settling (`entry.settles`, plan P6).
    pub fn entry_settles(&self, entry: EntryId) -> &[EntryId] {
        self.entry_settles.get(&entry).map_or(&[], Vec::as_slice)
    }

    /// Whether any entry records settling `entry`: the one way a
    /// member-paid entry reads as reimbursed (IZ-18: no FIFO guess).
    pub fn settles_entry(&self, entry: EntryId) -> bool {
        self.entry_settles.values().any(|s| s.contains(&entry))
    }

    /// Record which entries `entry` settles (a reimbursement naming what
    /// it pays back, plan P6 step 5). INVARIANT 3: every entry exists,
    /// none is `entry` itself, none is named twice.
    pub fn set_entry_settles(
        &mut self,
        entry: EntryId,
        settles: Vec<EntryId>,
    ) -> Result<(), LedgerError> {
        if !self.entries.contains_key(&entry) {
            return Err(LedgerError::UnknownEntry(entry));
        }
        let mut seen = std::collections::BTreeSet::new();
        for s in &settles {
            if *s == entry {
                return Err(LedgerError::InvalidEvidence(
                    "an entry doesn't settle itself".into(),
                ));
            }
            if !self.entries.contains_key(s) {
                return Err(LedgerError::UnknownEntry(*s));
            }
            if !seen.insert(*s) {
                return Err(LedgerError::InvalidEvidence(format!(
                    "entry {s} is settled twice by entry {entry}"
                )));
            }
        }
        self.entry_settles.insert(entry, settles);
        Ok(())
    }

    /// [`Self::set_entry_settles`] as journaled (`entry.settles`), for
    /// replay only (rules.md B-28).
    pub fn replay_set_entry_settles(
        &mut self,
        entry: EntryId,
        settles: Vec<EntryId>,
    ) -> Result<(), LedgerError> {
        self.set_entry_settles(entry, settles)
    }

    /// Check a convention before it is journaled: its id, text and
    /// structure (crate::conventions::check_new), and that every account a
    /// posting template or approval rule names is in the chart (a
    /// template's, one that takes postings). Returns it as it will be
    /// journaled; changes nothing.
    pub fn check_convention(
        &self,
        c: &crate::conventions::NewConvention,
    ) -> Result<crate::conventions::NewConvention, LedgerError> {
        let bad = LedgerError::InvalidConvention;
        let c = crate::conventions::check_new(c).map_err(bad)?;
        let old = self.conventions.convention(&c.id);
        if old.is_some_and(|o| o.retired.is_some()) {
            return Err(bad(format!(
                "convention {} is retired; ids are never reused: choose a new one",
                c.id
            )));
        }
        if let Some(o) = old.filter(|o| o.kind != c.kind) {
            return Err(bad(format!(
                "convention {} is a {}; its kind can't change (retire it and set a new id)",
                c.id,
                o.kind.key()
            )));
        }
        let probe = crate::conventions::Convention {
            id: c.id.clone(),
            kind: c.kind,
            title: c.title.clone(),
            body_md: c.body_md.clone(),
            structured: c.structured.clone(),
            source: c.source.clone(),
            seq: 0,
            revision: 1,
            retired: None,
        };
        if let Some(t) = probe.template() {
            for l in t
                .lines
                .iter()
                .filter(|l| crate::conventions::slot(&l.account).is_none())
            {
                let a = self.chart.find_by_code(&l.account).ok_or_else(|| {
                    bad(format!(
                        "template account {} is not in the chart",
                        l.account
                    ))
                })?;
                if a.placeholder {
                    return Err(bad(format!(
                        "template account {} is a placeholder; templates post to accounts that take postings",
                        l.account
                    )));
                }
            }
        }
        if let Some(r) = probe.approval_rule() {
            for code in &r.accounts {
                match self.chart.find_by_code(code) {
                    None => {
                        return Err(bad(format!(
                            "approval rule account {code} is not in the chart"
                        )));
                    }
                    Some(a) if a.placeholder => {
                        return Err(bad(format!(
                            "approval rule account {code} is a placeholder (a heading), which no line posts to: name the accounts under it"
                        )));
                    }
                    Some(_) => {}
                }
            }
        }
        if old.is_some_and(|o| {
            o.title == c.title && o.body_md == c.body_md && o.structured == c.structured
        }) {
            return Err(bad(format!(
                "convention {} already says exactly that",
                c.id
            )));
        }
        Ok(c)
    }

    /// Apply a journaled `convention.set` at `seq` (checked when made;
    /// replay takes it as journaled).
    pub fn apply_convention(
        &mut self,
        c: crate::conventions::NewConvention,
        seq: u64,
    ) -> Result<(), LedgerError> {
        self.conventions
            .apply_convention(c, seq)
            .map_err(LedgerError::InvalidConvention)
    }

    /// Check a retirement before it is journaled: the convention is live,
    /// the reason and source are sound.
    pub fn check_retire_convention(
        &self,
        id: &str,
        reason: &str,
        source: &crate::settings::Source,
    ) -> Result<String, LedgerError> {
        let bad = LedgerError::InvalidConvention;
        source.check().map_err(bad)?;
        let reason = crate::settings::long_text("reason", reason, crate::settings::MAX_LONG_TEXT)
            .map_err(bad)?;
        match self.conventions.convention(id) {
            None => Err(bad(format!(
                "unknown convention {id:?}: books.conventions lists them"
            ))),
            Some(c) if c.retired.is_some() => {
                Err(bad(format!("convention {id} is already retired")))
            }
            Some(_) => Ok(reason),
        }
    }

    /// Apply a journaled `convention.retired` at `seq`.
    pub fn apply_retire_convention(
        &mut self,
        id: &str,
        reason: String,
        source: crate::settings::Source,
        seq: u64,
    ) -> Result<(), LedgerError> {
        self.conventions
            .apply_retire(id, reason, source, seq)
            .map_err(LedgerError::InvalidConvention)
    }

    /// Check a decision before it is journaled; returns it as it will be
    /// journaled, under the id it will get. Changes nothing. (Its
    /// evidence attachment is the store's to check.)
    pub fn check_decision(
        &self,
        d: &crate::conventions::NewDecision,
    ) -> Result<(u64, crate::conventions::NewDecision), LedgerError> {
        let bad = LedgerError::InvalidConvention;
        let d = crate::conventions::check_decision(d).map_err(bad)?;
        for (i, p) in d.projects.iter().enumerate() {
            if !self.classes.contains_key(p) {
                return Err(LedgerError::UnknownClass(*p));
            }
            if d.projects[..i].contains(p) {
                return Err(bad(format!("project {p} is named twice")));
            }
        }
        if let Some(old) = d.supersedes {
            match self.conventions.decision(old) {
                None => {
                    return Err(bad(format!(
                        "decision {old} doesn't exist: books.decisions lists them"
                    )));
                }
                Some(o) if o.superseded_by.is_some() => {
                    return Err(bad(format!(
                        "decision {old} was already superseded by {}",
                        o.superseded_by.unwrap_or_default()
                    )));
                }
                Some(_) => {}
            }
        }
        Ok((self.conventions.next_decision_id(), d))
    }

    /// Apply a journaled `decision.recorded` under `id` at `seq`.
    pub fn apply_decision(
        &mut self,
        id: u64,
        d: crate::conventions::NewDecision,
        seq: u64,
    ) -> Result<(), LedgerError> {
        self.conventions
            .apply_decision(id, d, seq)
            .map_err(LedgerError::InvalidConvention)
    }

    /// Check an open item before it is journaled; returns it, under the
    /// id it will get.
    pub fn check_open_item(
        &self,
        i: &crate::conventions::NewOpenItem,
    ) -> Result<(u64, crate::conventions::NewOpenItem), LedgerError> {
        let i = crate::conventions::check_open_item(i).map_err(LedgerError::InvalidConvention)?;
        Ok((self.conventions.next_open_item_id(), i))
    }

    /// Apply a journaled `open_item.raised` under `id` at `seq`.
    pub fn apply_open_item(
        &mut self,
        id: u64,
        i: crate::conventions::NewOpenItem,
        seq: u64,
    ) -> Result<(), LedgerError> {
        self.conventions
            .apply_raise(id, i, seq)
            .map_err(LedgerError::InvalidConvention)
    }

    /// Check a resolution before it is journaled: the item is open, the
    /// text and source are sound.
    pub fn check_resolve_item(
        &self,
        id: u64,
        resolution: &str,
        source: &crate::settings::Source,
    ) -> Result<String, LedgerError> {
        let bad = LedgerError::InvalidConvention;
        source.check().map_err(bad)?;
        let text =
            crate::settings::long_text("resolution", resolution, crate::settings::MAX_LONG_TEXT)
                .map_err(bad)?;
        match self.conventions.open_item(id) {
            None => Err(bad(format!(
                "unknown open item {id}: books.open_items lists them"
            ))),
            Some(i) if i.resolved.is_some() => {
                Err(bad(format!("open item {id} is already resolved")))
            }
            Some(_) => Ok(text),
        }
    }

    /// Apply a journaled `open_item.resolved` at `seq`.
    pub fn apply_resolve_item(
        &mut self,
        id: u64,
        resolution: String,
        source: crate::settings::Source,
        seq: u64,
    ) -> Result<(), LedgerError> {
        self.conventions
            .apply_resolve(id, resolution, source, seq)
            .map_err(LedgerError::InvalidConvention)
    }

    /// Check an approval before it is journaled: `entry` is a draft, the
    /// approver is named, the note and source are sound. Returns the
    /// approval as it will be journaled (its fingerprint is the draft's
    /// now: crate::conventions INVARIANT 3).
    pub fn check_approval(
        &self,
        entry: EntryId,
        approver: &str,
        note: Option<&str>,
        source: &crate::settings::Source,
    ) -> Result<crate::conventions::Approval, LedgerError> {
        let bad = LedgerError::InvalidConvention;
        source.check().map_err(bad)?;
        let e = self
            .entries
            .get(&entry)
            .ok_or(LedgerError::UnknownEntry(entry))?;
        if e.status() == EntryStatus::Posted {
            return Err(LedgerError::EntryAlreadyPosted(entry));
        }
        Ok(crate::conventions::Approval {
            approver: crate::settings::line_text("approver", approver, crate::settings::MAX_TEXT)
                .map_err(bad)?,
            fingerprint: self.approval_fingerprint(e),
            note: note
                .map(|n| crate::settings::long_text("note", n, crate::settings::MAX_LONG_TEXT))
                .transpose()
                .map_err(bad)?,
            source: source.clone(),
            seq: 0,
        })
    }

    /// Apply a journaled `entry.approved` at `seq` (the entry must exist).
    pub fn apply_approval(
        &mut self,
        entry: EntryId,
        approval: crate::conventions::Approval,
    ) -> Result<(), LedgerError> {
        if !self.entries.contains_key(&entry) {
            return Err(LedgerError::UnknownEntry(entry));
        }
        self.conventions.apply_approval(entry, approval);
        Ok(())
    }

    /// An entry's fingerprint for approvals, with who it is with
    /// (crate::conventions::fingerprint).
    fn approval_fingerprint(&self, e: &JournalEntry) -> String {
        crate::conventions::fingerprint(
            e,
            self.entry_parties
                .get(&e.id())
                .map_or(&[][..], Vec::as_slice),
            self.entry_no_party.get(&e.id()).map(String::as_str),
        )
    }

    /// The approvals `entry` still needs before it posts: each live
    /// approval rule covering it whose approver hasn't approved it as it
    /// stands, as `(rule id, approver)`. Empty under
    /// `policy.approval_rules = off` (crate::conventions INVARIANT 3).
    pub fn approvals_missing(&self, entry: EntryId) -> Result<Vec<(String, String)>, LedgerError> {
        if self.settings.approval_rules() == crate::settings::ApprovalMode::Off {
            return Ok(Vec::new());
        }
        let e = self
            .entries
            .get(&entry)
            .ok_or(LedgerError::UnknownEntry(entry))?;
        let codes: Vec<&str> = e
            .lines()
            .iter()
            .filter_map(|l| self.chart.get(l.account).map(|a| a.code.as_str()))
            .collect();
        let debits = e
            .lines()
            .iter()
            .filter(|l| l.side == Side::Debit)
            .try_fold(Decimal::ZERO, |t, l| t.checked_add(l.amount.amount))
            .ok_or_else(|| LedgerError::TooLargeToCompute(format!("entry {entry}'s debits")))?;
        let approvals = self.conventions.approvals(entry);
        let key = crate::parties::name_key;
        let mut missing = Vec::new();
        for (c, r) in self.conventions.approval_rules() {
            // A rule this version can't read covers everything (fail
            // closed): no approver it names can be matched.
            let Some(r) = r else {
                missing.push((c.id.clone(), "an approver this version can't read".into()));
                continue;
            };
            let by_account =
                r.accounts.is_empty() || r.accounts.iter().any(|a| codes.contains(&a.as_str()));
            let by_amount = r
                .amount_over
                .as_deref()
                .and_then(crate::settings::decimal)
                .is_none_or(|over| debits > over);
            let covered = by_account && by_amount;
            let now = self.approval_fingerprint(e);
            let approved = approvals
                .iter()
                .any(|a| key(&a.approver) == key(&r.approver) && a.fingerprint == now);
            if covered && !approved {
                missing.push((c.id.clone(), r.approver.clone()));
            }
        }
        Ok(missing)
    }

    /// The lines a posting template drafts for `amount` (crate::
    /// conventions INVARIANT 2): `(account, side, amount)`, each side
    /// summing to `amount` exactly, and the template's description.
    /// `slots` names the account (by code) for each of the template's
    /// slots (`{expense}` → `"expense"`): every slot, and nothing else.
    pub fn template_lines(
        &self,
        template_id: &str,
        amount: Decimal,
        slots: &std::collections::BTreeMap<String, String>,
    ) -> Result<TemplateDraft, LedgerError> {
        let bad = LedgerError::InvalidConvention;
        let c = self.conventions.convention(template_id).ok_or_else(|| {
            bad(format!(
                "unknown convention {template_id:?}: books.conventions lists the templates"
            ))
        })?;
        if c.retired.is_some() {
            return Err(bad(format!("template {template_id} is retired")));
        }
        let t = c.template().ok_or_else(|| {
            bad(format!(
                "convention {template_id} is a {}, not a posting template",
                c.kind.key()
            ))
        })?;
        let wanted: std::collections::BTreeSet<&str> = t
            .lines
            .iter()
            .filter_map(|l| crate::conventions::slot(&l.account))
            .collect();
        let missing: Vec<&str> = wanted
            .iter()
            .copied()
            .filter(|w| !slots.contains_key(*w))
            .collect();
        if !missing.is_empty() {
            return Err(bad(format!(
                "template {template_id} needs an account for {}: pass accounts {{{}}}",
                missing.join(", "),
                missing
                    .iter()
                    .map(|m| format!("\"{m}\": \"<code>\""))
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }
        if let Some(extra) = slots.keys().find(|k| !wanted.contains(k.as_str())) {
            return Err(bad(format!(
                "template {template_id} has no slot {extra:?}{}",
                if wanted.is_empty() {
                    String::new()
                } else {
                    format!(
                        "; its slots: {}",
                        wanted.iter().copied().collect::<Vec<_>>().join(", ")
                    )
                }
            )));
        }
        if amount <= Decimal::ZERO {
            return Err(bad(format!("amount {amount} must be above zero")));
        }
        if amount > crate::kernel::MAX_LINE_AMOUNT {
            return Err(LedgerError::AmountOutOfRange);
        }
        check_minor_units(Money::new(amount, self.base_currency))?;
        let places = self.base_currency.minor_units();
        let mut out = Vec::new();
        for side in [Side::Debit, Side::Credit] {
            let lines: Vec<_> = t.lines.iter().filter(|l| l.side == side).collect();
            let shares: Vec<Decimal> = lines
                .iter()
                .map(|l| crate::settings::decimal(&l.share).unwrap_or_default())
                .collect();
            let parts = crate::conventions::split(amount, &shares, places).ok_or_else(|| {
                LedgerError::TooLargeToCompute(format!("template {template_id} at {amount}"))
            })?;
            for (l, part) in lines.iter().zip(parts) {
                if part <= Decimal::ZERO {
                    return Err(bad(format!(
                        "{amount} is too small to split by template {template_id}: account {}'s part would be {part}",
                        l.account
                    )));
                }
                let code = match crate::conventions::slot(&l.account) {
                    Some(name) => slots[name].as_str(),
                    None => l.account.as_str(),
                };
                let a = self
                    .chart
                    .find_by_code(code)
                    .ok_or_else(|| bad(format!("template account {code} is not in the chart")))?;
                if a.placeholder {
                    return Err(bad(format!(
                        "account {code} is a placeholder (a heading); a template posts to accounts that take postings"
                    )));
                }
                out.push((a.id, side, Money::new(part, self.base_currency)));
            }
        }
        Ok((t.description, out))
    }

    // ------------------------------------------------------------------
    // Classes
    // ------------------------------------------------------------------

    /// Add a class (division) for line-level P&L tracking: a project with
    /// only its name (rules.md B-37). Returns its id.
    ///
    /// ```text
    /// GOAL: a new class under a name no other class has.
    /// SOURCE: docs/spec/rules.md B-37 item 1.
    /// SCENARIOS:
    ///   1. IF the name is blank: SHOULD refuse (EmptyClassName).
    ///   2. IF it is longer than a line, holds a control character or a
    ///      full TIN: SHOULD refuse.
    ///   3. IF another class has it, ignoring capitals and surrounding
    ///      space: SHOULD refuse (DuplicateClassName). Replay never checks
    ///      (replay_class), so old books holding a duplicate still open.
    ///   4. IF it is digits only: SHOULD refuse (it would read as an id).
    /// ```
    pub fn add_class(&mut self, name: &str) -> Result<ClassId, LedgerError> {
        let name = self.check_class_name(name, None)?;
        let id = self.next_class_id;
        self.next_class_id += 1;
        self.classes.insert(id, Class { id, name });
        Ok(id)
    }

    /// A class name, trimmed, if a class (other than `except`) may take it.
    ///
    /// ```text
    /// GOAL: one rule for every name a command gives a class.
    /// SOURCE: docs/spec/rules.md B-37 item 1; B-26 (no full TIN).
    /// SCENARIOS:
    ///   1. IF `except` is a class and the name is its own, exactly: SHOULD
    ///      keep it as stored, unchecked, so a fact-only change never
    ///      renames (an earlier version stored names untrimmed) and never
    ///      refuses a name the books already hold.
    ///   2. IF it is the class's own name up to capitals and surrounding
    ///      space: SHOULD check its text but not its uniqueness, so old
    ///      books holding a duplicate (replay never checks) can still
    ///      change either class.
    ///   3. IF it is digits only: SHOULD refuse, since a project is named
    ///      by id or name and such a name would read as an id.
    ///   4. IF another class has it, ignoring capitals and surrounding
    ///      space: SHOULD refuse (DuplicateClassName).
    /// ```
    fn check_class_name(&self, name: &str, except: Option<ClassId>) -> Result<String, LedgerError> {
        let current = except.and_then(|id| self.classes.get(&id));
        if let Some(c) = current
            && c.name == name
        {
            return Ok(c.name.clone());
        }
        if name.trim().is_empty() {
            return Err(LedgerError::EmptyClassName);
        }
        let name = crate::settings::line_text("project name", name, crate::settings::MAX_TEXT)
            .map_err(LedgerError::InvalidProject)?;
        let folded = fold_class_name(&name);
        if current.is_some_and(|c| fold_class_name(&c.name) == folded) {
            return Ok(name);
        }
        if name.chars().all(|c| c.is_ascii_digit()) {
            return Err(LedgerError::InvalidProject(format!(
                "project name {name:?} is digits only, so it would read as a project id: add a word (\"Job {name}\")"
            )));
        }
        if self
            .classes
            .values()
            .any(|c| Some(c.id) != except && fold_class_name(&c.name) == folded)
        {
            return Err(LedgerError::DuplicateClassName(name));
        }
        Ok(name)
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

    /// Find a class by name, ignoring capitals (in any script) and
    /// surrounding space: the fold the duplicate check uses. Among old
    /// books' duplicates (replay never checks), the lowest id.
    pub fn find_class_by_name(&self, name: &str) -> Option<&Class> {
        let folded = fold_class_name(name);
        self.classes
            .values()
            .filter(|c| fold_class_name(&c.name) == folded)
            .min_by_key(|c| c.id)
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

    /// A class's project facts: the ones set, or the defaults for a class
    /// that has none (every class an earlier version wrote). `None` for no
    /// such class.
    pub fn project_facts(&self, id: ClassId) -> Option<crate::classes::ProjectFacts> {
        self.classes.get(&id)?;
        Some(self.projects.get(&id).cloned().unwrap_or_default())
    }

    /// The projects for or with a party, by name (ignoring capitals).
    pub fn projects_of_party(&self, party: crate::PartyId) -> Vec<ClassId> {
        let mut ids: Vec<ClassId> = self
            .projects
            .iter()
            .filter(|(_, f)| f.party == Some(party))
            .map(|(id, _)| *id)
            .filter(|id| self.classes.contains_key(id))
            .collect();
        ids.sort_by_key(|id| {
            let c = &self.classes[id];
            (c.name.to_lowercase(), c.id)
        });
        ids
    }

    /// Check a whole project record before it is journaled: returns the
    /// trimmed name and the normalized facts. Changes nothing.
    ///
    /// ```text
    /// GOAL: refuse any project record the books can't take, before a
    ///       line is written.
    /// SOURCE: docs/spec/rules.md B-37 item 1 (and B-26: no full TIN).
    /// SCENARIOS:
    ///   1. IF the class doesn't exist: SHOULD refuse (UnknownClass).
    ///   2. IF the name is blank, too long, or another class has it
    ///      (ignoring capitals; the class itself excepted): SHOULD refuse.
    ///   3. IF a fact breaks its rule (ProjectFacts::check): SHOULD refuse.
    ///   4. IF the party is not in the directory: SHOULD refuse.
    /// ```
    pub fn check_set_project(
        &self,
        id: ClassId,
        name: &str,
        facts: &crate::classes::ProjectFacts,
    ) -> Result<(String, crate::classes::ProjectFacts), LedgerError> {
        if !self.classes.contains_key(&id) {
            return Err(LedgerError::UnknownClass(id));
        }
        let name = self.check_class_name(name, Some(id))?;
        let facts = facts.check().map_err(LedgerError::InvalidProject)?;
        if let Some(p) = facts.party
            && self.get_party(p).is_none()
        {
            return Err(LedgerError::InvalidProject(format!(
                "party {p} is not in the directory: books.parties lists them"
            )));
        }
        Ok((name, facts))
    }

    /// Apply a project record after [`Ledger::check_set_project`]: rename
    /// the class, keep the facts.
    pub fn set_project(
        &mut self,
        id: ClassId,
        name: &str,
        facts: crate::classes::ProjectFacts,
    ) -> Result<(), LedgerError> {
        let (name, facts) = self.check_set_project(id, name, &facts)?;
        self.put_project(id, name, facts);
        Ok(())
    }

    /// Replay a journaled project record: only what replay can't do
    /// without (the class and the party exist), never the text rules, so a
    /// rule tightened later never breaks old books (as
    /// `AssetBasis::check_journaled`, rules.md B-25).
    #[doc(hidden)]
    pub fn replay_set_project(
        &mut self,
        id: ClassId,
        name: String,
        facts: crate::classes::ProjectFacts,
    ) -> Result<(), LedgerError> {
        if !self.classes.contains_key(&id) {
            return Err(LedgerError::UnknownClass(id));
        }
        if let Some(p) = facts.party
            && self.get_party(p).is_none()
        {
            return Err(LedgerError::InvalidProject(format!(
                "party {p} is not in the directory"
            )));
        }
        self.put_project(id, name, facts);
        Ok(())
    }

    fn put_project(&mut self, id: ClassId, name: String, facts: crate::classes::ProjectFacts) {
        if let Some(c) = self.classes.get_mut(&id) {
            c.name = name;
        }
        if facts.is_default() {
            self.projects.remove(&id);
        } else {
            self.projects.insert(id, facts);
        }
    }
}

/// A class name as names compare (rules.md B-37 item 1): trimmed, in
/// lower case (Unicode's, so `Éclair` and `éCLAIR` are one name).
fn fold_class_name(name: &str) -> String {
    name.trim().to_lowercase()
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

/// Refuse a new line larger than any posting could carry
/// (kernel::MAX_LINE_AMOUNT; P0 review round 3, S1). Only the size: the
/// minor unit is check_minor_units', and the post checks the full range.
fn check_line_size(amount: Decimal) -> Result<(), LedgerError> {
    if amount.abs() > kernel::MAX_LINE_AMOUNT {
        return Err(LedgerError::AmountOutOfRange);
    }
    Ok(())
}

/// A members close's (capital account, share) list, in order: what a run
/// of run-rule closes must share in that order, and a count of cumulative
/// closes as a set ([`same_member_set`]).
fn member_key(members: &[periods::MemberClose]) -> Vec<(AccountId, Decimal)> {
    members.iter().map(|m| (m.capital, m.share)).collect()
}

/// Two members closes' (capital account, share) lists name the same
/// members with the same shares, in any order (rules.md B-13: a reorder of
/// the members is not a change of members). Shares compare by value, so
/// 0.5 is 0.50.
fn same_member_set(a: &[(AccountId, Decimal)], b: &[(AccountId, Decimal)]) -> bool {
    let sorted = |v: &[(AccountId, Decimal)]| {
        let mut v = v.to_vec();
        v.sort_unstable();
        v
    };
    a.len() == b.len() && sorted(a) == sorted(b)
}

/// A vendor invoice number as the duplicate check compares it: lower case,
/// with spaces and hyphens removed ("ACME-123" and "acme 123" are the
/// same number; rules.md B-40).
fn normalize_vendor_ref(vendor_ref: &str) -> String {
    vendor_ref
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .flat_map(char::to_lowercase)
        .collect()
}

/// A members regime ([`Ledger::members_regime`]): the members closes with
/// one set of members and shares since the last close with another.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Regime {
    /// The close the count starts at (its [`Period::close_seq`]); `None`
    /// when no members close with this set is counted yet.
    pub since_seq: Option<u64>,
    /// The net those closes closed, together (negative for a loss).
    pub closed: Decimal,
    /// What those closes credited each member, in the order asked for, by
    /// capital account.
    pub given: Vec<Decimal>,
    /// The members as the count's first close listed them: the order its
    /// tied odd units go in. As of a day before any close of a count, the
    /// count the next members close begins, when it is these members'
    /// ([`Ledger::members_regime`]). Empty when there is none.
    pub order: Vec<(AccountId, Decimal)>,
}

impl Regime {
    /// [`periods::allocate_cumulative`] of `total` among `members` against
    /// `given` (both in `members`' order), the tied odd units to the member
    /// the count's first close listed first ([`Regime::order`]; `members`'
    /// own order when no close has begun the count), the parts back in `members`'
    /// order. So a count's ties go to the same member whatever order later
    /// closes, statements or the settings list the members in, and a
    /// reorder of the members moves no cent (rules.md B-13, D-38).
    pub(crate) fn allocate(
        &self,
        members: &[(AccountId, Decimal)],
        total: Decimal,
        given: &[Decimal],
        places: u32,
    ) -> Result<Vec<Decimal>, LedgerError> {
        let order = if self.order.is_empty() {
            members
        } else {
            &self.order
        };
        let index: Vec<usize> = order
            .iter()
            .map(|(capital, _)| {
                members
                    .iter()
                    .position(|(c, _)| c == capital)
                    .expect("a count's members are the members asked for")
            })
            .collect();
        let shares: Vec<Decimal> = index.iter().map(|&i| members[i].1).collect();
        let given: Vec<Decimal> = index.iter().map(|&i| given[i]).collect();
        let split = periods::allocate_cumulative(total, &shares, &given, places)?;
        let mut parts = vec![Decimal::ZERO; members.len()];
        for (&i, part) in index.iter().zip(split) {
            parts[i] = part;
        }
        Ok(parts)
    }
}
