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
//! * Reports ([`trial_balance`](reports::trial_balance), [`balance_sheet`](reports::balance_sheet), [`unclosed_earnings`](reports::unclosed_earnings)) are
//!   pure functions of a ledger, which is what makes the semantic oracle
//!   (`docs/oracle.md`) possible.

pub mod account;
pub mod analysis;
pub mod arap;
pub mod assets;
pub mod classes;
pub mod error;
pub mod form1099;
pub mod journal;
pub mod kernel;
pub mod ledger;
pub mod money;
pub mod payouts;
pub mod payroll;
pub mod periods;
#[cfg(kani)]
mod proofs;
pub mod reports;
pub mod rules;
pub mod schedules;
pub mod tax;

pub use account::{Account, AccountId, AccountType, ChartOfAccounts, Side};
pub use arap::{
    AgingRow, Bill, BillId, Customer, CustomerId, DocumentLine, DocumentStatus, Invoice, InvoiceId,
    Vendor, VendorId,
};
pub use assets::{
    Asset, AssetId, AssetStatus, DepreciationMethod, DisposalAmounts, NewAsset, ScheduleLine,
    TaxBasis, TaxBasisRule, accumulated_depreciation, book_value, depreciation_schedule,
    disposal_amounts,
};
pub use classes::{Class, ClassId};
pub use error::LedgerError;
pub use form1099::{Form1099, Vendor1099Line, Vendor1099Report, vendor_1099_report};
pub use journal::{EntryId, EntryStatus, JournalEntry, JournalLine};
pub use ledger::{
    ControlAccount, GENERAL_EXPENSES, Ledger, PAYABLES, PAYROLL, RECEIVABLES, RETAINED_EARNINGS,
    REVENUE, ReconciliationReport, UnclearedLine, control_account, control_accounts,
};
pub use money::{Currency, Money};
pub use payouts::{Payout, PayoutAdjustment, PayoutLine, PayoutTotals, payout_plan};
pub use payroll::{PayrollLine, PayrollRun, PayrollRunId};
pub use periods::{
    CloseTo, MemberClose, MemberCloseRecord, Period, PeriodId, PeriodStatus, RETAINED_EARNINGS_CODE,
};
pub use reports::{
    AssetRegister, AssetRegisterRow, BalanceSheet, BalanceSheetStatement, CashFlowLine,
    CashFlowParams, CashFlowStatement, ClassProfitAndLoss, GeneralLedger, LedgerAccount,
    LedgerPosting, MemberCapital, MemberEquityColumn, MembersEquityStatement, ProfitAndLoss,
    ProfitAndLossStatement, StatementLine, TrialBalance, TrialBalanceLine, balance_sheet_as_of,
    cash_flow, fixed_asset_register, general_ledger, members_equity, profit_and_loss_for_period,
    trial_balance_as_of,
};
pub use schedules::{
    AmortizationSchedule, Frequency, NewRecurring, NewSchedule, RecurringEntry, RecurringId,
    ScheduleId, ScheduleKind, TemplateLine,
};
pub use tax::{TaxCategory, TaxForm, TaxKind, TaxSummary, TaxSummaryLine, tax_summary};
