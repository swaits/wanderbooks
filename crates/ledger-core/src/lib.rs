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
pub mod assets;
pub mod error;
pub mod journal;
pub mod ledger;
pub mod money;
pub mod reports;

pub use account::{Account, AccountId, AccountType, ChartOfAccounts, Side};
pub use assets::{
    Asset, AssetId, AssetStatus, DepreciationMethod, DisposalAmounts, NewAsset, ScheduleLine,
    accumulated_depreciation, book_value, depreciation_schedule, disposal_amounts,
};
pub use error::LedgerError;
pub use journal::{EntryId, EntryStatus, JournalEntry, JournalLine};
pub use ledger::Ledger;
pub use money::{Currency, Money};
pub use reports::{
    BalanceSheet, CashFlowLine, CashFlowParams, CashFlowStatement, ProfitAndLoss, TrialBalance,
    TrialBalanceLine, cash_flow,
};
