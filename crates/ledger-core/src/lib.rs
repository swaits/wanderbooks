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
pub mod calendar;
pub mod cashflow;
pub mod classes;
pub mod conventions;
pub mod error;
pub mod evidence;
pub mod form1099;
pub mod instruments;
pub mod journal;
pub mod k1;
pub mod kernel;
pub mod ledger;
pub mod money;
pub mod parties;
pub mod payouts;
pub mod payroll;
pub mod periods;
#[cfg(kani)]
mod proofs;
pub mod reports;
pub mod rules;
pub mod schedules;
pub mod settings;
pub mod shares;
pub mod tax;
pub mod tax_capital;

pub use account::{Account, AccountId, AccountType, ChartOfAccounts, Side};
pub use arap::{
    AgingDocument, AgingRow, Application, Bill, BillFields, BillId, ControlCheck, Customer,
    CustomerId, DocLine, DocRef, DocState, DocStatus, DocTag, DocumentKind, DocumentLine,
    DocumentPartKey, DocumentStatus, EarlyDiscount, Fee, Invoice, InvoiceFields, InvoiceId,
    Movement, PartySnapshot, PaymentDetails, SendKind, Sent, SentVia, SettleParams, SettlementRow,
    Terms, TermsKind, UnattachedLine, Vendor, VendorId,
};
pub use assets::{
    Asset, AssetBasis, AssetCorrection, AssetId, AssetStatus, BasisRule, DepreciationMethod,
    DepreciationPosting, DisposalAmounts, NewAsset, PostedRun, RunAmount, ScheduleLine,
    Sec704cMethod, accumulated_scheduled_through, catch_up_room, check_disposal,
    depreciation_effects, depreciation_schedule, disposal_amounts, life_end, run_amount,
    run_amounts, scheduled_book_value,
};
pub use cashflow::{
    CashFlowFooting, CashFlowKind, CashFlowLine, CashFlowParams, CashFlowSection,
    CashFlowStatement, CashFlowWarning, CashFlowWarningKind, NoncashDisclosure, OperatingClass,
    OperatingReconciliation, ReconcilingKind, ReconcilingLine, RoleList, RoleProblem, cash_flow,
    role_problems,
};
pub use classes::{Class, ClassId, ProjectFacts, ProjectKind, ProjectStatus};
pub use error::LedgerError;
pub use evidence::{Annotation, AnnotationFields};
pub use form1099::{
    Dated1099, Decision1099, ExemptReason, Form1099, Payment1099, PaymentAccounts, Status1099,
    Vendor1099Line, Vendor1099Report, vendor_1099_report,
};
pub use instruments::{Instrument, InstrumentId, InstrumentKind};
pub use journal::{EntryId, EntryStatus, JournalEntry, JournalLine};
pub use kernel::{EntryFlows, Flow};
pub use ledger::{
    BAD_DEBTS, ControlAccount, GENERAL_EXPENSES, Ledger, PAYABLES, PAYMENT_PROCESSING_FEES,
    PAYROLL, PURCHASE_DISCOUNTS, RECEIVABLES, RETAINED_EARNINGS, REVENUE, ReconciliationReport,
    SALES_ALLOWANCES, UnclearedLine, control_account, control_accounts,
};
pub use money::{Currency, Money};
pub use parties::{Party, PartyField, PartyId, Role, TinFacts, TinType, W4Facts, W9Facts};
pub use payouts::{Payout, PayoutAdjustment, PayoutLine, PayoutTotals, payout_plan};
pub use payroll::{PayrollLine, PayrollRun, PayrollRunId};
pub use periods::{
    CloseTo, MemberClose, MemberCloseRecord, MemberRule, MemberSplit, MembersClosed, Period,
    PeriodId, PeriodStatus, RETAINED_EARNINGS_CODE,
};
pub use reports::{
    AssetRegister, AssetRegisterRow, BalanceSheet, BalanceSheetStatement, ClassProfitAndLoss,
    GeneralLedger, LedgerAccount, LedgerPosting, MemberCapital, MemberEquityColumn,
    MembersEquityStatement, ProfitAndLoss, ProfitAndLossStatement, ProjectFigures, ProjectToDate,
    StatementLine, TrialBalance, TrialBalanceLine, balance_sheet_as_of, contribution_margin_pct,
    fixed_asset_register, general_ledger, members_equity, profit_and_loss_for_period,
    project_entries, project_figures, project_to_date, trial_balance_as_of,
};
pub use schedules::{
    AmortizationSchedule, Frequency, NewRecurring, NewSchedule, RecurringEntry, RecurringId,
    ScheduleConvention, ScheduleId, ScheduleKind, TemplateLine,
};
pub use shares::{PartyMember, PartyShare, Share};
pub use tax::{
    BookOnlyLine, TaxCategory, TaxForm, TaxKind, TaxSummary, TaxSummaryLine, tax_summary,
};
