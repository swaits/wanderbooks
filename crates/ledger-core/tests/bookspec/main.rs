//! The bookspec examples (`bookspec/examples/*.toml`, P13, D-34), checked
//! against the engine on every run, so they can't rot. This is the one
//! place every example is loaded.
//!
//! Each example is a setup, events and expected values worked by hand from
//! its cited source (`bookspec/README.md` gives the shape). Every example
//! must load, whatever it asks or whichever appendix it is in: its shape,
//! its accounts, fixed assets, schedules, parties, members, payees,
//! payees' dated 1099 decisions and payment methods, every event (an
//! entry, a reversal, a close, a bill, or a command that posts what is
//! due: a depreciation run, a disposal, a schedule run or registration),
//! each accepted or refused as it says, with the parties, payee and
//! payment method it is tagged with, and every change that posts no entry
//! (a rename, a policy, an asset's correction). The books are the
//! engine's own store (`ledger_store::Store`, in a directory of their
//! own), so every event goes through the command the books take it by,
//! journaled, and the books must replay to what was kept
//! (`Store::verify`). The queries the
//! core answers (`cash_flow`, `trial_balance`, `profit_and_loss`,
//! `vendor_1099_paid`, `vendor_1099`, `entry`, `balances`,
//! `members_equity`, `k1`, `k1_tax_basis`, `k1_reconciliation`,
//! `party_part`, `party_totals`, `party_tin_last4`, `period`, `day`,
//! `calendar_year`, `account_names`, `uncategorized_accounts`, `run`,
//! `disposal`, `depreciation_schedule`, `schedule_plan`, `register_row`,
//! `register_totals`, `asset_basis`) are then checked value by value, and
//! the commands it tries (`rename_account`, `add_account`,
//! `set_tax_category`) accepted or refused on a copy of the books, each
//! after the event it names (`after`) or the last; the formats appendix's
//! (`iif_lines`, `iif_changes`) are checked by ledger-package
//! (`the_bookspec_iif_example_matches_the_engine`), and a query no crate
//! answers is refused. A key the shape doesn't have is refused too, never
//! ignored.
//!
//! The adapter is split by area: `common` (the shared vocabulary and the
//! date and refusal rules), `books` (an example's books and their setup),
//! `events` (applying an event), `queries/` (each query family's answers
//! and checks), `engine` (the `Engine` impl), `check` and `run` (the
//! per-expectation and per-example drivers), and the tests by area
//! (`examples`, `loader_tests`, `sources`, `rules_md`). `lean_model` is
//! apart: random command sequences, checked against the Lean model of the
//! ledger (`just lean-model`).

pub(crate) use std::borrow::Borrow;
pub(crate) use std::collections::{BTreeMap, BTreeSet};
pub(crate) use std::path::{Path, PathBuf};

pub(crate) use chrono::{Datelike, NaiveDate};
pub(crate) use ledger_core::calendar::{PeriodDefaults, period_defaults};
pub(crate) use ledger_core::parties::{Basis, MoneyAccounts};
pub(crate) use ledger_core::settings::{Company, Source};
pub(crate) use ledger_core::tax_capital::TaxCapitalRow;
pub(crate) use ledger_core::{
    AccountType, AssetId, BillFields, CashFlowParams, CashFlowSection, CashFlowStatement, CloseTo,
    Currency, Dated1099, Decision1099, DocumentLine, EntryId, EntryStatus, ExemptReason, Form1099,
    InstrumentId, InstrumentKind, InvoiceFields, Ledger, LedgerError, MemberCapital, MemberClose,
    Money, NewAsset, NewSchedule, PartyId, PartyMember, PartyShare, PaymentAccounts,
    ReconcilingKind, Role, ScheduleConvention, ScheduleId, ScheduleKind, Share, Side, Terms,
    TinFacts, TinType, VendorId, W9Facts, cash_flow, members_equity, profit_and_loss_for_period,
    trial_balance_as_of, vendor_1099_report,
};
pub(crate) use ledger_store::{DepreciationRun, DisposalPreview, Store, StoreError};
pub(crate) use rust_decimal::Decimal;

pub(crate) use bookspec_checker::{
    AccountSpec, Area, BasisSpec, ChangeSpec, Claims, DocumentLineSpec, Engine, Event, Example,
    Expect, Query, Refusal, RuleId, ScheduleSpec, Setup, Values,
};
pub(crate) use ledger_import::IifField;
pub(crate) use ledger_package::export_iif;
pub(crate) use serde_json::Value;

pub(crate) use books::*;
pub(crate) use check::check;
pub(crate) use common::*;
pub(crate) use queries::assets::*;
pub(crate) use queries::commands::*;
pub(crate) use queries::dates::*;
pub(crate) use queries::documents::*;
pub(crate) use queries::formats::*;
pub(crate) use queries::parties::*;
pub(crate) use queries::statements::*;
pub(crate) use queries::tax::*;
pub(crate) use run::{load, run};

mod books;
mod check;
mod common;
mod engine;
mod events;
mod examples;
mod lean_model;
mod loader_tests;
mod pin;
mod queries;
mod rules_md;
mod run;
mod source_refs;
mod sources;
