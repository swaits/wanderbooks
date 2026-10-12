//! `ledger-core`: canonical double-entry bookkeeping semantics.
//!
//! This crate is the engine. Everything else in the workspace — persistence,
//! import, the oracle harness, the UI — builds on top of it and never
//! reimplements accounting logic.
//!
//! Design rules (see `AGENTS.md` at the repo root):
//!
//! * Money is [`Money`]: an exact [`rust_decimal::Decimal`] plus an ISO 4217
//!   currency code. Binary floats never touch money paths.
//! * A [`Ledger`] is single-currency for now (see `docs/requirements.md` R-5).
//! * All mutations go through [`Ledger`] commands (`create_entry`, `add_line`,
//!   `post_entry`, `reverse_entry`). Posted entries are immutable; corrections
//!   are reversing entries.
//! * Reports ([`trial_balance`], [`balance_sheet`], [`profit_and_loss`]) are
//!   pure functions of a ledger, which is what makes the semantic oracle
//!   (`docs/oracle.md`) possible.

pub mod account;
pub mod arap;
pub mod assets;
pub mod classes;
pub mod error;
pub mod journal;
pub mod ledger;
pub mod money;
pub mod payroll;
pub mod periods;
pub mod reports;
pub mod schedules;
pub mod tax;

pub use account::{Account, AccountId, AccountType, ChartOfAccounts, Side};
pub use arap::{
    AgingRow, Bill, BillId, Customer, CustomerId, DocumentLine, DocumentStatus, Invoice, InvoiceId,
    Vendor, VendorId,
};
pub use assets::{
    Asset, AssetId, AssetStatus, DepreciationMethod, DisposalAmounts, NewAsset, ScheduleLine,
    accumulated_depreciation, book_value, depreciation_schedule, disposal_amounts,
};
pub use classes::{Class, ClassId};
pub use error::LedgerError;
pub use journal::{EntryId, EntryStatus, JournalEntry, JournalLine};
pub use ledger::{Ledger, ReconciliationReport, UnclearedLine};
pub use money::{Currency, Money};
pub use payroll::{PayrollLine, PayrollRun, PayrollRunId};
pub use periods::{CloseTo, MemberClose, Period, PeriodId, PeriodStatus, RETAINED_EARNINGS_CODE};
pub use reports::{
    BalanceSheet, BalanceSheetStatement, CashFlowLine, CashFlowParams, CashFlowStatement,
    ClassProfitAndLoss, GeneralLedger, LedgerAccount, LedgerPosting, MemberCapital,
    MemberEquityColumn, MembersEquityStatement, ProfitAndLoss, ProfitAndLossStatement,
    StatementLine, TrialBalance, TrialBalanceLine, balance_sheet_as_of, cash_flow, general_ledger,
    members_equity, profit_and_loss_for_period, trial_balance_as_of,
};
pub use schedules::{
    AmortizationSchedule, Frequency, NewRecurring, NewSchedule, RecurringEntry, RecurringId,
    ScheduleId, ScheduleKind, TemplateLine,
};
pub use tax::{TaxCategory, TaxForm, TaxKind, TaxSummary, TaxSummaryLine, tax_summary};
