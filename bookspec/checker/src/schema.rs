//! The shape of a bookspec example: the TOML schema, the parser, and the
//! rules a well-formed example follows whatever engine runs it.
//!
//! One file per example, named for its `id` (`bookspec/README.md` gives the
//! shape in prose; this module is its executable form). Every structure
//! refuses a key it does not know (`deny_unknown_fields`): a typo is an
//! error, never silently ignored. [`Example::from_toml`] parses and then
//! checks the shape; an engine's own adapter only has to execute it.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::rules::rule_for_refusal;

/// One bookspec example: a setup, its events and changes, and what it
/// expects of the books.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Example {
    /// Stable, never reused; the file is named for it.
    pub id: String,
    /// What the example shows, in words.
    pub title: String,
    /// The normative sentence the example demonstrates: what MUST (or MUST
    /// NOT) hold.
    pub rule: String,
    /// What the expected values rest on: `AUTHORITY`, `CONVENTION` or
    /// `CHOICE`.
    pub status: String,
    /// Exact citations, non-empty.
    pub sources: Vec<String>,
    /// Applicability tags, non-empty.
    pub tags: Vec<String>,
    /// `"formats"`: an interchange format's example rather than an
    /// accounting rule's (P13's appendix of formats); absent otherwise.
    #[serde(default)]
    pub appendix: Option<String>,
    /// The books before the first event.
    pub setup: Setup,
    /// The events, in order, counting from 1.
    pub events: Vec<Event>,
    /// Changes that post no entry, each made after the event it names.
    #[serde(default)]
    pub changes: Vec<ChangeSpec>,
    /// What is expected of the books, at least one.
    pub expect: Vec<Expect>,
}

/// The books an example sets up, before any event.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Setup {
    /// ISO 4217 currency code.
    pub currency: String,
    /// `MM-DD`.
    pub fiscal_year_start: String,
    /// `accrual` or `cash`.
    pub basis: String,
    /// How a figure is rounded to the currency's minor unit: `half-even`,
    /// `half-away-from-zero` or `largest-remainder`.
    pub rounding: String,
    /// The chart, parents before their subaccounts.
    pub accounts: Vec<AccountSpec>,
    /// The cash accounts (ASC 230-10-45-10 to 45-15).
    #[serde(default)]
    pub cash_accounts: Vec<String>,
    /// The financing accounts.
    #[serde(default)]
    pub financing_accounts: Vec<String>,
    /// The investing accounts.
    #[serde(default)]
    pub investing_accounts: Vec<String>,
    /// Accounts that hold property and equipment, for an example whose
    /// individual assets do not matter.
    #[serde(default)]
    pub fixed_assets: Vec<FixedAsset>,
    /// Fixed assets registered, with their figures (B-15).
    #[serde(default)]
    pub assets: Vec<AssetSpec>,
    /// Prepaid-expense and deferred-revenue schedules (B-16).
    #[serde(default)]
    pub schedules: Vec<ScheduleSpec>,
    /// The company's directory (B-26).
    #[serde(default)]
    pub parties: Vec<PartySpec>,
    /// The members of a multi-member LLC (B-13).
    #[serde(default)]
    pub members: Vec<MemberSpec>,
    /// Payees marked for an information return (B-19).
    #[serde(default)]
    pub payees: Vec<PayeeSpec>,
    /// A payee's 1099 decision from a date on (B-19, D-40).
    #[serde(default)]
    pub status_changes: Vec<StatusChangeSpec>,
    /// The ways the company (or a member, for it) pays (B-19).
    #[serde(default)]
    pub instruments: Vec<InstrumentSpec>,
    /// The projects (B-37).
    #[serde(default)]
    pub projects: Vec<ProjectSpec>,
    /// The day the books are asked on: what an omitted date means (B-30).
    #[serde(default)]
    pub today: Option<String>,
}

/// A project (B-37): its name, and its kind, status and party if given.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectSpec {
    /// Its name, once each.
    pub name: String,
    /// `internal`, `client`, `on_spec` or `other`.
    #[serde(default)]
    pub kind: Option<String>,
    /// `active`, `paused`, `finished` or `abandoned`.
    #[serde(default)]
    pub status: Option<String>,
    /// A `setup.parties` name.
    #[serde(default)]
    pub party: Option<String>,
}

/// One account of the chart.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountSpec {
    /// Its code.
    pub code: String,
    /// Its name.
    pub name: String,
    /// `asset`, `liability`, `equity`, `revenue` or `expense`.
    #[serde(rename = "type")]
    pub kind: String,
    /// Its parent's code, declared before it.
    #[serde(default)]
    pub parent: Option<String>,
    /// A header groups other accounts and takes no postings.
    #[serde(default)]
    pub header: bool,
    /// Its tax category, a key of the catalog (B-32).
    #[serde(default)]
    pub tax_category: Option<String>,
}

/// Accounts that hold property and equipment, for an example no asset's
/// own figures matter to.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FixedAsset {
    /// The cost account.
    pub cost_account: String,
    /// The accumulated depreciation account.
    pub accumulated_account: String,
    /// The depreciation expense account.
    pub expense_account: String,
}

/// A fixed asset registered before the first event (B-15).
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetSpec {
    /// Its name in this example.
    pub id: String,
    /// What it is.
    pub description: String,
    /// Its cost, an exact decimal string.
    pub cost: String,
    /// Its salvage value.
    pub salvage: String,
    /// Its useful life in months, above zero.
    pub life_months: u32,
    /// The day depreciation starts, `YYYY-MM-DD`.
    pub in_service: String,
    /// `straight_line`, the one method the books have.
    pub method: String,
    /// The cost account.
    pub cost_account: String,
    /// The accumulated depreciation account.
    pub accumulated_account: String,
    /// The depreciation expense account.
    pub expense_account: String,
    /// The event whose entry put it on the books (B-15 rule 7).
    #[serde(default)]
    pub acquired_by: Option<usize>,
    /// How the company got it and the facts its tax figures follow from
    /// (B-25).
    #[serde(default)]
    pub basis: Option<BasisSpec>,
}

/// An asset's basis facts (B-25).
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BasisSpec {
    /// `purchase`, `carryover_723` or `personal_use_conversion`.
    pub rule: String,
    /// A contribution's contributor's adjusted basis.
    #[serde(default)]
    pub contributor_adjusted_basis: Option<String>,
    /// A contribution's fair market value.
    #[serde(default)]
    pub fmv_at_contribution: Option<String>,
    /// The section 704(c) method, if chosen.
    #[serde(default)]
    pub sec704c_method: Option<String>,
}

/// A prepaid expense or deferred revenue recognized over its months
/// (B-16), in `setup.schedules` or registered by an event.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduleSpec {
    /// Its name in this example.
    pub id: String,
    /// `prepaid_expense` or `deferred_revenue`.
    pub kind: String,
    /// What it is.
    pub description: String,
    /// The whole amount, an exact decimal string.
    pub total: String,
    /// The day it starts, `YYYY-MM-DD`.
    pub start: String,
    /// Its months, above zero.
    pub months: u32,
    /// `daily` or `monthly_even`.
    pub convention: String,
    /// The balance-sheet account it sits in.
    pub balance_account: String,
    /// The profit-and-loss account it recognizes to.
    pub pl_account: String,
}

/// One member of a multi-member LLC (B-13).
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemberSpec {
    /// The member's name; a party's, with the member role, when the
    /// example asks about the member as a party.
    pub name: String,
    /// Their capital account.
    pub capital_account: String,
    /// Their share of income, an exact decimal fraction above zero; every
    /// member has one or none does, and the shares sum to 1.
    #[serde(default)]
    pub share: Option<String>,
    /// Their distributions account (B-13).
    #[serde(default)]
    pub distributions_account: Option<String>,
}

/// A payee marked for an information return from the beginning (B-19).
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PayeeSpec {
    /// Its name, once each.
    pub name: String,
    /// `1099-NEC` or `1099-MISC`.
    pub form: String,
}

/// One party in the directory (B-26, B-35).
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartySpec {
    /// Its name, once each.
    pub name: String,
    /// Role keys; `vendor` and `customer` give the party its record (one
    /// of them at most).
    pub roles: Vec<String>,
    /// The liabilities, by code, the company owes the party itself
    /// (B-35 item 6).
    #[serde(default)]
    pub creditor_of: Vec<String>,
    /// Its TIN facts' last four (an SSN's).
    #[serde(default)]
    pub tin_last4: Option<String>,
    /// Its W-9 facts' last four (a 1099 role's).
    #[serde(default)]
    pub w9_tin_last4: Option<String>,
    /// The vault's copy of the TIN purged, the facts kept (needs
    /// `tin_last4`).
    #[serde(default)]
    pub tin_purged: bool,
}

/// A payee's 1099 decision from `effective` on (B-19, D-40).
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatusChangeSpec {
    /// A `setup.payees` name.
    pub payee: String,
    /// `YYYY-MM-DD`.
    pub effective: String,
    /// `1099-NEC`, `1099-MISC`, `undetermined`, or `not_reportable` (with
    /// `reason_kind` and `reason`).
    pub decision: String,
    /// Why it is not reportable, from the closed list.
    #[serde(default)]
    pub reason_kind: Option<String>,
    /// The facts, in words.
    #[serde(default)]
    pub reason: Option<String>,
}

/// A payment method (B-19).
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstrumentSpec {
    /// Its label, once each.
    pub label: String,
    /// `bank`, `business_card`, `member_card`, `member_cash_or_check` or
    /// `platform`.
    pub kind: String,
    /// The account it pays through, by code.
    pub account: String,
    /// The card's or account's last four, if known.
    #[serde(default)]
    pub last4: Option<String>,
    /// The party it belongs to, by name (informational).
    #[serde(default)]
    pub owner: Option<String>,
}

/// A party named on an entry, with its share (B-35).
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShareSpec {
    /// A `setup.parties` name.
    pub party: String,
    /// Its share as an exact decimal string.
    #[serde(default)]
    pub amount: Option<String>,
    /// Its share as an exact decimal fraction.
    #[serde(default)]
    pub percent: Option<String>,
}

/// One event: an entry, a reversal, a close, a bill, an invoice, a
/// command that posts what is due, or a schedule's registration.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Event {
    /// The day of an entry, a reversal, a close, a bill, an invoice or a
    /// disposal; the other commands date their own entries.
    #[serde(default)]
    pub date: Option<String>,
    /// An entry's, a reversal's, a close's or a bill's; a command writes
    /// its own.
    #[serde(default)]
    pub description: Option<String>,
    /// An entry's lines.
    #[serde(default)]
    pub lines: Vec<LineSpec>,
    /// The event (1-based) whose entry this one reverses.
    #[serde(default)]
    pub reverses: Option<usize>,
    /// A close of a period.
    #[serde(default)]
    pub close: Option<CloseSpec>,
    /// A bill.
    #[serde(default)]
    pub bill: Option<BillSpec>,
    /// An invoice.
    #[serde(default)]
    pub invoice: Option<InvoiceSpec>,
    /// A bad-debt write-off of an earlier invoice.
    #[serde(default)]
    pub write_off: Option<WriteOffSpec>,
    /// A depreciation run.
    #[serde(default)]
    pub run_depreciation: Option<RunSpec>,
    /// A disposal.
    #[serde(default)]
    pub dispose_asset: Option<DisposeSpec>,
    /// A run of every schedule due through a day.
    #[serde(default)]
    pub run_schedules: Option<RunSchedulesSpec>,
    /// A schedule registered by an event.
    #[serde(default)]
    pub register_schedule: Option<ScheduleSpec>,
    /// The engine refuses the event, for this reason in the examples'
    /// words (`refusal_reasons`); it changes nothing.
    #[serde(default)]
    pub refused: Option<String>,
    /// The payee (a `setup.payees` name) the entry paid.
    #[serde(default)]
    pub payee: Option<String>,
    /// The parties (a `setup.parties` name) the entry is with, and their
    /// shares.
    #[serde(default)]
    pub parties: Vec<ShareSpec>,
    /// The payment method (a `setup.instruments` label) it was paid with.
    #[serde(default)]
    pub instrument: Option<String>,
    /// An entry as an earlier version could write it and this version's
    /// commands refuse (the control guard, B-38): posted as replay posts
    /// it, since books that hold it must still read right.
    #[serde(default)]
    pub older: bool,
}

impl Event {
    /// Which of the event's kinds it is, or why it is not exactly one.
    pub fn kind(&self) -> Result<EventKind, String> {
        let kinds = [
            (EventKind::Lines, !self.lines.is_empty()),
            (EventKind::Reverses, self.reverses.is_some()),
            (EventKind::Close, self.close.is_some()),
            (EventKind::Bill, self.bill.is_some()),
            (EventKind::Invoice, self.invoice.is_some()),
            (EventKind::WriteOff, self.write_off.is_some()),
            (EventKind::RunDepreciation, self.run_depreciation.is_some()),
            (EventKind::DisposeAsset, self.dispose_asset.is_some()),
            (EventKind::RunSchedules, self.run_schedules.is_some()),
            (
                EventKind::RegisterSchedule,
                self.register_schedule.is_some(),
            ),
        ];
        let given: Vec<EventKind> = kinds
            .iter()
            .filter_map(|(kind, given)| given.then_some(*kind))
            .collect();
        match given.as_slice() {
            [one] => Ok(*one),
            _ => Err(
                "an event is one of lines, reverses, close, bill, invoice, run_depreciation, \
                 dispose_asset, run_schedules or register_schedule"
                    .to_string(),
            ),
        }
    }
}

/// The kind of an event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    /// An entry, by its `lines`.
    Lines,
    /// A reversal of an earlier event's entry.
    Reverses,
    /// A close of a period.
    Close,
    /// A bill.
    Bill,
    /// An invoice.
    Invoice,
    /// A bad-debt write-off of an earlier invoice (rules.md B-41).
    WriteOff,
    /// A depreciation run.
    RunDepreciation,
    /// A fixed asset's disposal.
    DisposeAsset,
    /// Every schedule's months due through a day.
    RunSchedules,
    /// A schedule's registration (posts nothing).
    RegisterSchedule,
}

impl EventKind {
    /// The key the example spells it with.
    pub fn key(self) -> &'static str {
        match self {
            EventKind::Lines => "lines",
            EventKind::Reverses => "reverses",
            EventKind::Close => "close",
            EventKind::Bill => "bill",
            EventKind::Invoice => "invoice",
            EventKind::WriteOff => "write_off",
            EventKind::RunDepreciation => "run_depreciation",
            EventKind::DisposeAsset => "dispose_asset",
            EventKind::RunSchedules => "run_schedules",
            EventKind::RegisterSchedule => "register_schedule",
        }
    }

    /// True when the example gives the event's day (a command that dates
    /// its own entries takes none).
    pub fn is_dated(self) -> bool {
        matches!(
            self,
            EventKind::Lines
                | EventKind::Reverses
                | EventKind::Close
                | EventKind::Bill
                | EventKind::Invoice
                | EventKind::WriteOff
                | EventKind::DisposeAsset
        )
    }

    /// True when the example describes the event (a command writes its own
    /// descriptions).
    pub fn is_described(self) -> bool {
        matches!(
            self,
            EventKind::Lines
                | EventKind::Reverses
                | EventKind::Close
                | EventKind::Bill
                | EventKind::Invoice
        )
    }

    /// True when the books take it whole or not at all, so `refused` may
    /// be given: a reversal, an entry or a command, never a close, a bill
    /// or an invoice (those are corrected by a reversal). An entry posts
    /// whole or not at all, so a hand line the control guard refuses is
    /// one too.
    pub fn may_refuse(self) -> bool {
        !matches!(
            self,
            EventKind::Close | EventKind::Bill | EventKind::Invoice
        )
    }

    /// True when the example may tag it with parties and a payment method.
    pub fn may_tag(self) -> bool {
        !matches!(
            self,
            EventKind::Reverses
                | EventKind::WriteOff
                | EventKind::RunDepreciation
                | EventKind::DisposeAsset
                | EventKind::RunSchedules
                | EventKind::RegisterSchedule
        )
    }
}

/// A depreciation run of `asset` for `period` (`YYYY-MM`), dated at the
/// month's end (B-15).
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunSpec {
    /// A `setup.assets` id.
    pub asset: String,
    /// `YYYY-MM`.
    pub period: String,
}

/// A fixed asset's disposal (B-15).
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DisposeSpec {
    /// A `setup.assets` id.
    pub asset: String,
    /// What it sold for, zero allowed.
    pub proceeds: String,
    /// The account the proceeds were received into.
    pub proceeds_account: String,
    /// The gain-or-loss account.
    pub gain_loss_account: String,
}

/// Every schedule's months due through `through` (B-16).
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunSchedulesSpec {
    /// `YYYY-MM-DD`.
    pub through: String,
}

/// A bill from `vendor` (B-38).
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BillSpec {
    /// The vendor's name.
    pub vendor: String,
    /// One line's amount, exactly one of `amount` and `lines`.
    #[serde(default)]
    pub amount: Option<String>,
    /// Its lines.
    #[serde(default)]
    pub lines: Vec<DocumentLineSpec>,
}

/// An invoice to `customer` (B-38).
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvoiceSpec {
    /// The customer's name.
    pub customer: String,
    /// One line's amount, exactly one of `amount` and `lines`.
    #[serde(default)]
    pub amount: Option<String>,
    /// Its lines.
    #[serde(default)]
    pub lines: Vec<DocumentLineSpec>,
}

/// A write-off of the invoice an earlier event posted (rules.md B-41;
/// plan P4b S7): the invoice's event, what to write off and why.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WriteOffSpec {
    /// The event (1-based) whose invoice this writes off: an `invoice`
    /// event before it.
    pub invoice: usize,
    /// How much to write off, an exact decimal string above zero; left
    /// out, all the invoice owes on the day.
    #[serde(default)]
    pub amount: Option<String>,
    /// Why it will not be collected (collection correspondence, a
    /// bankruptcy), one line.
    pub reason: String,
}

/// One line of an invoice or a bill.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentLineSpec {
    /// Its amount, an exact decimal string above zero.
    pub amount: String,
    /// What it is (the event's description when left out).
    #[serde(default)]
    pub description: Option<String>,
    /// The project it belongs to.
    #[serde(default)]
    pub project: Option<String>,
}

/// A change to the books that posts no entry, made right after event
/// `after` once that event's expectations are checked: exactly one of
/// `rename`, `require_tax_category` and `correct_asset`.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChangeSpec {
    /// The event after which it is made (1 to the number of events).
    pub after: usize,
    /// A rename.
    #[serde(default)]
    pub rename: Option<RenameSpec>,
    /// The tax-category policy: `warn` (the books' default) or `refuse`
    /// (B-32).
    #[serde(default)]
    pub require_tax_category: Option<String>,
    /// An asset's correction.
    #[serde(default)]
    pub correct_asset: Option<CorrectSpec>,
}

/// A registered asset's economics corrected for `reason`: each figure
/// given replaces the asset's; the rest stay.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CorrectSpec {
    /// A `setup.assets` id.
    pub asset: String,
    /// Its new cost.
    #[serde(default)]
    pub cost: Option<String>,
    /// Its new salvage value.
    #[serde(default)]
    pub salvage: Option<String>,
    /// Its new useful life, above zero.
    #[serde(default)]
    pub life_months: Option<u32>,
    /// Its new in-service day.
    #[serde(default)]
    pub in_service: Option<String>,
    /// Why it is corrected.
    pub reason: String,
}

/// Rename the account `code` to `name` (B-31).
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenameSpec {
    /// The account's code.
    pub code: String,
    /// Its new name.
    pub name: String,
}

/// One line of an entry.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LineSpec {
    /// The account's code.
    pub account: String,
    /// `debit` or `credit`.
    pub side: String,
    /// The amount, an exact decimal string above zero.
    pub amount: String,
    /// The project the line is tagged with.
    #[serde(default)]
    pub project: Option<String>,
}

/// A close of the period from `start` through the event's date (B-11,
/// B-13).
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CloseSpec {
    /// `YYYY-MM-DD`.
    pub start: String,
    /// `members` or `retained_earnings`.
    pub to: String,
    /// `per-close`: a members close split on its own, not cumulatively.
    #[serde(default)]
    pub rounding: Option<String>,
}

/// One expectation: a query, the dates or things it asks about, and what
/// it expects.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Expect {
    /// The query's name (`QUERIES`).
    pub query: String,
    /// A period query's first day, `YYYY-MM-DD`.
    #[serde(default)]
    pub from: Option<String>,
    /// A period query's last day.
    #[serde(default)]
    pub to: Option<String>,
    /// An as-of query's day.
    #[serde(default)]
    pub as_of: Option<String>,
    /// The event whose entry an `entry` or `party_part` query asks about.
    #[serde(default)]
    pub entry: Option<usize>,
    /// The `run_depreciation` event a `run` query asks about.
    #[serde(default)]
    pub event: Option<usize>,
    /// The fixed asset a `disposal`, `depreciation_schedule`,
    /// `register_row` or `asset_basis` query asks about.
    #[serde(default)]
    pub asset: Option<String>,
    /// The schedule a `schedule_plan` query asks about.
    #[serde(default)]
    pub schedule: Option<String>,
    /// The party a `party_part`, `party_totals` or `party_tin_last4` query
    /// asks about.
    #[serde(default)]
    pub party: Option<String>,
    /// The project a `project` query asks about.
    #[serde(default)]
    pub project: Option<String>,
    /// `party_totals`' basis: `cash` or `all` (B-35 item 4).
    #[serde(default)]
    pub basis: Option<String>,
    /// The payee a `vendor_1099` query asks about.
    #[serde(default)]
    pub payee: Option<String>,
    /// The books after event N, not after the last (a format query may
    /// not name it).
    #[serde(default)]
    pub after: Option<usize>,
    /// The account a command names, by code.
    #[serde(default)]
    pub code: Option<String>,
    /// The new name a `rename_account` command gives.
    #[serde(default)]
    pub name: Option<String>,
    /// The category a `set_tax_category` command sets (absent: it clears
    /// the account's).
    #[serde(default)]
    pub category: Option<String>,
    /// The account an `add_account` command adds.
    #[serde(default)]
    pub account: Option<AccountSpec>,
    /// A command the engine accepts: `true` (never `false`).
    #[serde(default)]
    pub accepted: Option<bool>,
    /// A command, or a `period`, the engine refuses: why, in the
    /// examples' words.
    #[serde(default)]
    pub refused: Option<String>,
    /// What the query expects.
    #[serde(default)]
    pub values: Option<Values>,
}

/// What a query expects: figures by key (every figure an exact decimal
/// string), or, for a file format, its lines or its rows of fields.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum Values {
    /// A table of named values, every one a string.
    Figures(BTreeMap<String, String>),
    /// A list of lines.
    Lines(Vec<String>),
    /// A list of rows, each a list of fields.
    Rows(Vec<Vec<String>>),
}

impl Values {
    /// True when it holds nothing.
    pub fn is_empty(&self) -> bool {
        match self {
            Values::Figures(f) => f.is_empty(),
            Values::Lines(l) => l.is_empty(),
            Values::Rows(r) => r.is_empty(),
        }
    }
}

impl Expect {
    /// The values the query expects; panics when it gives none.
    pub fn values(&self, id: &str) -> &Values {
        self.values
            .as_ref()
            .unwrap_or_else(|| panic!("{id}: {} expects values", self.query))
    }

    /// The figures the query expects; panics on lines or rows.
    pub fn figures(&self, id: &str) -> &BTreeMap<String, String> {
        match self.values(id) {
            Values::Figures(f) => f,
            _ => panic!("{id}: {} expects figures (a table)", self.query),
        }
    }

    /// The question this expectation asks, as the engine receives it.
    pub fn query(&self) -> Query {
        Query {
            name: self.query.clone(),
            from: self.from.clone(),
            to: self.to.clone(),
            as_of: self.as_of.clone(),
            entry: self.entry,
            event: self.event,
            asset: self.asset.clone(),
            schedule: self.schedule.clone(),
            party: self.party.clone(),
            project: self.project.clone(),
            basis: self.basis.clone(),
            payee: self.payee.clone(),
            after: self.after,
            code: self.code.clone(),
            new_name: self.name.clone(),
            category: self.category.clone(),
            account: self.account.clone(),
        }
    }
}

/// The question an expectation asks: everything but the answer.
#[derive(Clone, Debug, PartialEq)]
pub struct Query {
    /// The query's name.
    pub name: String,
    /// A period's first day (raw, `YYYY-MM-DD`).
    pub from: Option<String>,
    /// A period's last day.
    pub to: Option<String>,
    /// An as-of query's day.
    pub as_of: Option<String>,
    /// The event whose entry is asked about.
    pub entry: Option<usize>,
    /// The event a `run` query asks about.
    pub event: Option<usize>,
    /// The asset asked about.
    pub asset: Option<String>,
    /// The schedule asked about.
    pub schedule: Option<String>,
    /// The party asked about.
    pub party: Option<String>,
    /// The project asked about.
    pub project: Option<String>,
    /// `party_totals`' basis.
    pub basis: Option<String>,
    /// The payee asked about.
    pub payee: Option<String>,
    /// The books after event N (the runner has already applied them).
    pub after: Option<usize>,
    /// The account a command names.
    pub code: Option<String>,
    /// The new name a rename command gives.
    pub new_name: Option<String>,
    /// The category a set-tax-category command sets.
    pub category: Option<String>,
    /// The account an add-account command adds.
    pub account: Option<AccountSpec>,
}

/// The queries every engine must answer: the figure queries first, then
/// the calendar ones, then the commands (tried on a copy of the books),
/// then the formats appendix's.
pub const QUERIES: &[&str] = &[
    "cash_flow",
    "trial_balance",
    "profit_and_loss",
    "entry",
    "balances",
    "members_equity",
    "k1",
    "k1_tax_basis",
    "k1_reconciliation",
    "vendor_1099_paid",
    "vendor_1099",
    "party_part",
    "party_totals",
    "party_tin_last4",
    "period",
    "day",
    "calendar_year",
    "account_names",
    "uncategorized_accounts",
    "run",
    "disposal",
    "depreciation_schedule",
    "schedule_plan",
    "register_row",
    "register_totals",
    "asset_basis",
    "project",
    "rename_account",
    "add_account",
    "set_tax_category",
    "iif_lines",
    "iif_changes",
];

/// The commands an example may try: each on a copy of the books, so it
/// changes nothing checked after it.
pub const COMMANDS: &[&str] = &["rename_account", "add_account", "set_tax_category"];

/// The formats appendix's queries.
pub const FORMAT_QUERIES: &[&str] = &["iif_lines", "iif_changes"];

/// The queries whose figures may be text rather than an exact decimal
/// string: a status, a direction, a date, a year, a name, a rule name,
/// `"none"`, `"open"`, `"true"` or `"false"`, and the register's dates
/// beside its figures.
pub const TEXT_VALUED: &[&str] = &[
    "vendor_1099",
    "party_part",
    "party_tin_last4",
    "period",
    "day",
    "calendar_year",
    "register_row",
    "project",
    "asset_basis",
    "k1_tax_basis",
    "account_names",
];

/// The queries whose example lists every value it covers, so a value left
/// out of an example is an error. The others name the figures they show:
/// `cash_flow`, the statements, the register, a schedule's plan and an
/// asset's basis.
pub const COMPLETE_QUERIES: &[&str] = &[
    "trial_balance",
    "profit_and_loss",
    "entry",
    "vendor_1099_paid",
    "vendor_1099",
    "party_part",
    "party_totals",
    "party_tin_last4",
    "period",
    "day",
    "calendar_year",
    "account_names",
    "uncategorized_accounts",
    "run",
    "disposal",
    "project",
];

impl Query {
    /// True when it is a command, tried on a copy of the books.
    pub fn is_command(&self) -> bool {
        COMMANDS.contains(&self.name.as_str())
    }

    /// True when it is a formats appendix's query.
    pub fn is_format(&self) -> bool {
        FORMAT_QUERIES.contains(&self.name.as_str())
    }

    /// True when the example must list every value the query covers.
    pub fn is_complete(&self) -> bool {
        COMPLETE_QUERIES.contains(&self.name.as_str())
    }
}

/// Every refusal reason the specification names (bookspec/README.md), each
/// the reason of exactly one rule (see [`crate::rules::rule_for_refusal`]).
pub fn refusal_reasons() -> impl Iterator<Item = &'static str> {
    [
        "end_before_start",
        "start_after_today",
        "blank",
        "unchanged",
        "sibling_has_it",
        "holds_a_tin",
        "not_one_line",
        "code_taken",
        "tax_category_required",
        "depreciation_past_disposal",
        "entry_irreversible",
        "already_disposed",
        "invalid_depreciation_period",
        "invalid_schedule",
        "untagged_control_line",
    ]
    .into_iter()
}

/// Why an example is not well formed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SchemaError {
    /// The example's id, when it has one.
    pub id: String,
    /// What is wrong.
    pub message: String,
}

impl fmt::Display for SchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.id.is_empty() {
            write!(f, "{}", self.message)
        } else {
            write!(f, "{}: {}", self.id, self.message)
        }
    }
}

impl std::error::Error for SchemaError {}

impl SchemaError {
    fn new(id: &str, message: impl Into<String>) -> Self {
        Self {
            id: id.to_string(),
            message: message.into(),
        }
    }
}

impl Example {
    /// Parse an example from its TOML text and check its shape.
    pub fn from_toml(text: &str) -> Result<Self, SchemaError> {
        let ex: Self = toml::from_str(text).map_err(|e| SchemaError::new("", e.to_string()))?;
        ex.validate()?;
        Ok(ex)
    }

    /// Check the example's shape: its closed lists, its dates and amounts,
    /// its references and its events' and expectations' forms.
    pub fn validate(&self) -> Result<(), SchemaError> {
        let id = self.id.as_str();
        let e = |m: String| SchemaError::new(id, m);
        if id.is_empty() {
            return Err(SchemaError::new("", "an example has an id"));
        }
        if self.title.is_empty() || self.rule.is_empty() {
            return Err(e("an example has a title and a rule".into()));
        }
        if self.sources.is_empty() || self.tags.is_empty() {
            return Err(e("an example is sourced and tagged".into()));
        }
        if !matches!(self.status.as_str(), "AUTHORITY" | "CONVENTION" | "CHOICE") {
            return Err(e(format!("status {:?}", self.status)));
        }
        if let Some(a) = &self.appendix
            && a != "formats"
        {
            return Err(e(format!("appendix {a:?}")));
        }
        if self.expect.is_empty() {
            return Err(e("an example expects something".into()));
        }
        self.setup.validate(id)?;

        let mut codes = BTreeSet::new();
        for a in &self.setup.accounts {
            codes.insert(a.code.as_str());
        }
        for a in &self.setup.assets {
            for c in [&a.cost_account, &a.accumulated_account, &a.expense_account] {
                if !codes.contains(c.as_str()) {
                    return Err(e(format!("asset {}: no account {c}", a.id)));
                }
            }
            if let Some(n) = a.acquired_by
                && (n < 1 || n > self.events.len())
            {
                return Err(e(format!(
                    "asset {}: acquired_by {n}: events count from 1, to {}",
                    a.id,
                    self.events.len()
                )));
            }
        }
        for s in &self.setup.schedules {
            s.validate(id, &codes)?;
        }
        let parties: BTreeSet<&str> = self.setup.parties.iter().map(|p| p.name.as_str()).collect();
        let projects: BTreeSet<&str> = self
            .setup
            .projects
            .iter()
            .map(|p| p.name.as_str())
            .collect();
        let payees: BTreeSet<&str> = self.setup.payees.iter().map(|p| p.name.as_str()).collect();
        let instruments: BTreeSet<&str> = self
            .setup
            .instruments
            .iter()
            .map(|i| i.label.as_str())
            .collect();
        for (n, ev) in self.events.iter().enumerate() {
            let n = n + 1;
            let what = |m: String| e(format!("event {n}: {m}"));
            let kind = ev.kind().map_err(what)?;
            if ev.date.is_some() != kind.is_dated() {
                return Err(what(if kind.is_dated() {
                    format!("{} is dated (date)", kind.key())
                } else {
                    format!("{} dates its own entries: no date", kind.key())
                }));
            }
            if let Some(d) = &ev.date {
                check_date(d, &what)?;
            }
            if ev.description.is_some() != kind.is_described() {
                return Err(what(if kind.is_described() {
                    format!("{} is described (description)", kind.key())
                } else {
                    format!("{} describes its own entries: no description", kind.key())
                }));
            }
            if ev.older && kind != EventKind::Lines {
                return Err(what(
                    "older is an entry's lines, as an earlier version wrote them".into(),
                ));
            }
            if ev.older && ev.refused.is_some() {
                return Err(what(
                    "an entry posted as an earlier version wrote it (older) can't also be \
                     refused"
                        .into(),
                ));
            }
            if ev.payee.is_some() && kind != EventKind::Lines {
                return Err(what(
                    "a payee is an entry's: a reversal takes its original's, a bill names \
                     its vendor, an invoice its customer, a close and a command have none"
                        .into(),
                ));
            }
            if (!ev.parties.is_empty() || ev.instrument.is_some()) && !kind.may_tag() {
                return Err(what(format!(
                    "{} is tagged with no parties or payment method",
                    kind.key()
                )));
            }
            if ev.refused.is_some() && !kind.may_refuse() {
                return Err(what(
                    "refused is for a reversal or a command, which the books take whole or \
                     not at all"
                        .into(),
                ));
            }
            if let Some(r) = &ev.refused
                && rule_for_refusal(r).is_none()
            {
                return Err(what(format!("{r:?} is no refusal the specification names")));
            }
            if let Some(p) = &ev.payee
                && !payees.contains(p.as_str())
            {
                return Err(what(format!("payee {p:?} not in setup.payees")));
            }
            if let Some(i) = &ev.instrument
                && !instruments.contains(i.as_str())
            {
                return Err(what(format!(
                    "payment method {i:?} not in setup.instruments"
                )));
            }
            if let Some(of) = ev.reverses {
                if of < 1 {
                    return Err(what(format!("reverses = {of}: events count from 1")));
                }
                if of >= n {
                    return Err(what(format!("reverses a later event ({of})")));
                }
            }
            for line in &ev.lines {
                if !codes.contains(line.account.as_str()) {
                    return Err(what(format!("no account {}", line.account)));
                }
                if !matches!(line.side.as_str(), "debit" | "credit") {
                    return Err(what(format!("side {:?}", line.side)));
                }
                // A refused entry may carry the amount the books refuse (zero or
                // negative, B-2); an accepted one carries a positive amount.
                check_amount(&line.amount, ev.refused.is_none(), &what)?;
                if let Some(p) = &line.project
                    && !projects.contains(p.as_str())
                {
                    return Err(what(format!("project {p:?} not in setup.projects")));
                }
            }
            for sh in &ev.parties {
                if !parties.contains(sh.party.as_str()) {
                    return Err(what(format!("party {:?} not in setup.parties", sh.party)));
                }
                match (&sh.amount, &sh.percent) {
                    (None, None) => {}
                    (Some(a), None) => check_amount(a, false, &what)?,
                    (None, Some(p)) => check_amount(p, false, &what)?,
                    _ => return Err(what("a share is an amount or a percent".into())),
                }
            }
            if let Some(c) = &ev.close {
                check_date(&c.start, &what)?;
                if !matches!(c.to.as_str(), "members" | "retained_earnings") {
                    return Err(what(format!("close to {:?}", c.to)));
                }
                if let Some(r) = &c.rounding
                    && !(r == "per-close" && c.to == "members")
                {
                    return Err(what(format!("close rounding {r:?}")));
                }
            }
            if let Some(b) = &ev.bill {
                check_document(&b.amount, &b.lines, &projects, &what)?;
                if b.vendor.is_empty() {
                    return Err(what("a bill names its vendor".into()));
                }
            }
            if let Some(i) = &ev.invoice {
                check_document(&i.amount, &i.lines, &projects, &what)?;
                if i.customer.is_empty() {
                    return Err(what("an invoice names its customer".into()));
                }
            }
            if let Some(w) = &ev.write_off {
                if w.invoice < 1 || w.invoice >= n {
                    return Err(what(format!(
                        "write_off names event {}: an earlier invoice",
                        w.invoice
                    )));
                }
                if self.events[w.invoice - 1].invoice.is_none() {
                    return Err(what(format!(
                        "write_off names event {}, which is no invoice",
                        w.invoice
                    )));
                }
                if let Some(a) = &w.amount {
                    check_amount(a, true, &what)?;
                }
                if w.reason.trim().is_empty() {
                    return Err(what("a write-off says why it will not be collected".into()));
                }
            }
            if let Some(r) = &ev.run_depreciation {
                check_month(&r.period, &what)?;
                if !self.setup.assets.iter().any(|a| a.id == r.asset) {
                    return Err(what(format!("no asset {:?} in setup.assets", r.asset)));
                }
            }
            if let Some(d) = &ev.dispose_asset {
                if !self.setup.assets.iter().any(|a| a.id == d.asset) {
                    return Err(what(format!("no asset {:?} in setup.assets", d.asset)));
                }
                check_amount(&d.proceeds, false, &what)?;
                for c in [&d.proceeds_account, &d.gain_loss_account] {
                    if !codes.contains(c.as_str()) {
                        return Err(what(format!("no account {c}")));
                    }
                }
            }
            if let Some(r) = &ev.run_schedules {
                check_date(&r.through, &what)?;
            }
            if let Some(s) = &ev.register_schedule
                && ev.refused.is_none()
            {
                s.validate(id, &codes)?;
            }
        }
        for c in &self.changes {
            let what = |m: String| e(format!("change after {}: {m}", c.after));
            if c.after < 1 || c.after > self.events.len() {
                return Err(what(format!(
                    "after = {}: events count from 1, to {}",
                    c.after,
                    self.events.len()
                )));
            }
            let given = [
                c.rename.is_some(),
                c.require_tax_category.is_some(),
                c.correct_asset.is_some(),
            ];
            if given.iter().filter(|x| **x).count() != 1 {
                return Err(what(
                    "a change is one of rename, require_tax_category or correct_asset".into(),
                ));
            }
            if let Some(r) = &c.rename {
                if !codes.contains(r.code.as_str()) {
                    return Err(what(format!("no account {}", r.code)));
                }
                if r.name.trim().is_empty() {
                    return Err(what("a rename names the account".into()));
                }
            }
            if let Some(p) = &c.require_tax_category
                && !matches!(p.as_str(), "warn" | "refuse")
            {
                return Err(what(format!("require_tax_category {p:?}")));
            }
            if let Some(f) = &c.correct_asset {
                if !self.setup.assets.iter().any(|a| a.id == f.asset) {
                    return Err(what(format!("no asset {:?} in setup.assets", f.asset)));
                }
                for a in [&f.cost, &f.salvage].into_iter().flatten() {
                    check_amount(a, false, &what)?;
                }
                if let Some(d) = &f.in_service {
                    check_date(d, &what)?;
                }
                if f.life_months == Some(0) {
                    return Err(what("life_months is above zero".into()));
                }
                if f.reason.trim().is_empty() {
                    return Err(what("a correction gives its reason".into()));
                }
            }
        }
        for x in &self.expect {
            let q = x.query.as_str();
            let what = |m: String| e(format!("{q}: {m}"));
            if !QUERIES.contains(&q) {
                return Err(e(format!("no query answers {q:?}")));
            }
            if let Some(n) = x.after
                && (n < 1 || n > self.events.len())
            {
                return Err(what(format!(
                    "after = {n}: events count from 1, to {}",
                    self.events.len()
                )));
            }
            if x.after.is_some() && FORMAT_QUERIES.contains(&q) {
                return Err(what(
                    "a format query is of the books after every event".into(),
                ));
            }
            if FORMAT_QUERIES.contains(&q) && self.appendix.as_deref() != Some("formats") {
                return Err(what(
                    "a format query belongs to an example in appendix \"formats\"".into(),
                ));
            }
            for d in [&x.from, &x.to, &x.as_of].into_iter().flatten() {
                check_date(d, &what)?;
            }
            only(x, &what)?;
            if COMMANDS.contains(&q) {
                if x.values.is_some() || (x.accepted == Some(true)) == x.refused.is_some() {
                    return Err(what(
                        "a command is accepted = true, or refused, and shows no values".into(),
                    ));
                }
                if x.accepted == Some(false) {
                    return Err(what("a command is accepted = true, or refused".into()));
                }
                if let Some(r) = &x.refused
                    && rule_for_refusal(r).is_none()
                {
                    return Err(what(format!("{r:?} is no refusal the specification names")));
                }
                if x.account.as_ref().is_some_and(|a| a.code.trim().is_empty()) {
                    return Err(what("add_account names the account's code".into()));
                }
            } else {
                match (&x.values, &x.refused) {
                    (Some(v), None) if !v.is_empty() => {}
                    (None, Some(r)) => {
                        if q != "period" {
                            return Err(what(
                                "only a period may be refused instead of answering".into(),
                            ));
                        }
                        if rule_for_refusal(r).is_none() {
                            return Err(what(format!(
                                "{r:?} is no refusal the specification names"
                            )));
                        }
                    }
                    _ => return Err(what("expects values (a period: values or refused)".into())),
                }
            }
            if let Some(v) = &x.values {
                match v {
                    Values::Figures(f) => {
                        for (k, val) in f {
                            if k.is_empty() {
                                return Err(what("a figure's key is empty".into()));
                            }
                            if !TEXT_VALUED.contains(&q) && !is_exact_decimal(val) {
                                return Err(what(format!(
                                    "{k}: {val:?} is no exact decimal string"
                                )));
                            }
                        }
                    }
                    Values::Lines(l) => {
                        if !matches!(q, "iif_lines" | "uncategorized_accounts") {
                            return Err(what("expects figures (a table), not lines".into()));
                        }
                        if l.is_empty() {
                            return Err(what("an empty list expects nothing".into()));
                        }
                    }
                    Values::Rows(rows) => {
                        if q != "iif_changes" {
                            return Err(what("expects figures (a table), not rows".into()));
                        }
                        if rows.iter().any(|r| r.len() != 3) {
                            return Err(what("an iif_changes row has three fields".into()));
                        }
                    }
                }
            }
            if FORMAT_QUERIES.contains(&q) && x.values.is_none() {
                return Err(what("a format query expects its lines or rows".into()));
            }
        }
        Ok(())
    }
}

impl Setup {
    /// Check the setup's shape: its closed lists, its dates, its accounts
    /// and its directory.
    pub fn validate(&self, id: &str) -> Result<(), SchemaError> {
        let e = |m: String| SchemaError::new(id, m);
        if self.currency.len() != 3 || !self.currency.bytes().all(|b| b.is_ascii_uppercase()) {
            return Err(e(format!("currency {:?}", self.currency)));
        }
        if !is_month_day(&self.fiscal_year_start) {
            return Err(e(format!("fiscal_year_start {:?}", self.fiscal_year_start)));
        }
        if !matches!(self.basis.as_str(), "accrual" | "cash") {
            return Err(e(format!("basis {:?}", self.basis)));
        }
        if !matches!(
            self.rounding.as_str(),
            "half-even" | "half-away-from-zero" | "largest-remainder"
        ) {
            return Err(e(format!("rounding {:?}", self.rounding)));
        }
        if let Some(t) = &self.today {
            check_date(t, &e)?;
        }
        let mut codes: BTreeSet<&str> = BTreeSet::new();
        for a in &self.accounts {
            if a.code.trim().is_empty() || a.name.trim().is_empty() {
                return Err(e("an account has a code and a name".into()));
            }
            if !matches!(
                a.kind.as_str(),
                "asset" | "liability" | "equity" | "revenue" | "expense"
            ) {
                return Err(e(format!("account {}: type {:?}", a.code, a.kind)));
            }
            if !codes.insert(a.code.as_str()) {
                return Err(e(format!("account {}: code used twice", a.code)));
            }
            if let Some(p) = &a.parent
                && !codes.contains(p.as_str())
            {
                return Err(e(format!(
                    "account {}: parent {p} is not declared before it",
                    a.code
                )));
            }
        }
        for fa in &self.fixed_assets {
            for c in [
                &fa.cost_account,
                &fa.accumulated_account,
                &fa.expense_account,
            ] {
                if !codes.contains(c.as_str()) {
                    return Err(e(format!("fixed asset: no account {c}")));
                }
            }
        }
        let mut ids = BTreeSet::new();
        for a in &self.assets {
            if a.id.is_empty() || a.description.is_empty() {
                return Err(e("an asset has an id and a description".into()));
            }
            if !ids.insert(a.id.as_str()) {
                return Err(e(format!("asset {}: id used twice", a.id)));
            }
            if a.method != "straight_line" {
                return Err(e(format!("asset {}: method {:?}", a.id, a.method)));
            }
            check_amount(&a.cost, false, &e)?;
            check_amount(&a.salvage, false, &e)?;
            if a.life_months == 0 {
                return Err(e(format!("asset {}: life_months is above zero", a.id)));
            }
            check_date(&a.in_service, &e)?;
            if let Some(b) = &a.basis {
                if !matches!(
                    b.rule.as_str(),
                    "purchase" | "carryover_723" | "personal_use_conversion"
                ) {
                    return Err(e(format!("asset {}: basis rule {:?}", a.id, b.rule)));
                }
                for x in [&b.contributor_adjusted_basis, &b.fmv_at_contribution]
                    .into_iter()
                    .flatten()
                {
                    check_amount(x, false, &e)?;
                }
                if let Some(m) = &b.sec704c_method
                    && !matches!(
                        m.as_str(),
                        "traditional" | "traditional_with_curative" | "remedial"
                    )
                {
                    return Err(e(format!("asset {}: section 704(c) method {m:?}", a.id)));
                }
            }
        }
        let mut schedule_ids = BTreeSet::new();
        for s in &self.schedules {
            s.validate(id, &codes)?;
            if !schedule_ids.insert(s.id.as_str()) {
                return Err(e(format!("schedule {}: id used twice", s.id)));
            }
        }
        let mut party_names = BTreeSet::new();
        for p in &self.parties {
            if p.name.trim().is_empty() {
                return Err(e("a party has a name".into()));
            }
            if !party_names.insert(p.name.as_str()) {
                return Err(e(format!("party {}: named twice", p.name)));
            }
            if p.roles.iter().any(|r| !ROLES.contains(&r.as_str())) {
                return Err(e(format!("party {}: roles {:?}", p.name, p.roles)));
            }
            if p.roles
                .iter()
                .filter(|r| *r == "vendor" || *r == "customer")
                .count()
                > 1
            {
                return Err(e(format!(
                    "party {}: a vendor or a customer, not both",
                    p.name
                )));
            }
            for c in &p.creditor_of {
                if !codes.contains(c.as_str()) {
                    return Err(e(format!("party {}: no account {c}", p.name)));
                }
            }
            for l in [&p.tin_last4, &p.w9_tin_last4].into_iter().flatten() {
                if l.len() != 4 || !l.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(e(format!("party {}: last four {l:?}", p.name)));
                }
            }
            if p.tin_purged && p.tin_last4.is_none() {
                return Err(e(format!("party {}: tin_purged needs tin_last4", p.name)));
            }
        }
        let mut member_names = BTreeSet::new();
        let shared = self.members.iter().filter(|m| m.share.is_some()).count();
        if shared != 0 && shared != self.members.len() {
            return Err(e("every member has a share, or none does".into()));
        }
        let mut total = Decimal::ZERO;
        for m in &self.members {
            if m.name.trim().is_empty() || !member_names.insert(m.name.as_str()) {
                return Err(e(format!("member {:?} is named twice", m.name)));
            }
            if !codes.contains(m.capital_account.as_str()) {
                return Err(e(format!(
                    "member {}: no account {}",
                    m.name, m.capital_account
                )));
            }
            if let Some(d) = &m.distributions_account
                && !codes.contains(d.as_str())
            {
                return Err(e(format!("member {}: no account {d}", m.name)));
            }
            if let Some(s) = &m.share {
                let share = decimal(s, &e)?;
                if share <= Decimal::ZERO {
                    return Err(e(format!("member {}: share {s:?} is above zero", m.name)));
                }
                total += share;
            }
        }
        if shared > 0 && total != Decimal::ONE {
            return Err(e("the shares sum to 1".into()));
        }
        let mut payee_names = BTreeSet::new();
        for p in &self.payees {
            if p.name.trim().is_empty() || !payee_names.insert(p.name.as_str()) {
                return Err(e(format!("payee {:?} is named twice", p.name)));
            }
            if !matches!(p.form.as_str(), "1099-NEC" | "1099-MISC") {
                return Err(e(format!("payee {}: form {:?}", p.name, p.form)));
            }
        }
        for c in &self.status_changes {
            if !payee_names.contains(c.payee.as_str()) {
                return Err(e(format!(
                    "status change: payee {:?} not in setup",
                    c.payee
                )));
            }
            check_date(&c.effective, &e)?;
            match c.decision.as_str() {
                "not_reportable" => {
                    if !c
                        .reason_kind
                        .as_deref()
                        .is_some_and(|k| EXEMPT_REASONS.contains(&k))
                    {
                        return Err(e(format!(
                            "status change of {}: not_reportable needs a reason_kind",
                            c.payee
                        )));
                    }
                    if c.reason.as_deref().is_none_or(|r| r.trim().is_empty()) {
                        return Err(e(format!(
                            "status change of {}: not_reportable needs a reason",
                            c.payee
                        )));
                    }
                }
                "undetermined" | "1099-NEC" | "1099-MISC" => {
                    if c.reason_kind.is_some() || c.reason.is_some() {
                        return Err(e(format!(
                            "status change of {}: only not_reportable takes reason_kind and reason",
                            c.payee
                        )));
                    }
                }
                other => {
                    return Err(e(format!(
                        "status change of {}: decision {other:?}",
                        c.payee
                    )));
                }
            }
        }
        let mut labels = BTreeSet::new();
        for i in &self.instruments {
            if i.label.trim().is_empty() || !labels.insert(i.label.as_str()) {
                return Err(e(format!("payment method {:?} is labelled twice", i.label)));
            }
            if !matches!(
                i.kind.as_str(),
                "bank" | "business_card" | "member_card" | "member_cash_or_check" | "platform"
            ) {
                return Err(e(format!("payment method {}: kind {:?}", i.label, i.kind)));
            }
            if !codes.contains(i.account.as_str()) {
                return Err(e(format!(
                    "payment method {}: no account {}",
                    i.label, i.account
                )));
            }
            if let Some(o) = &i.owner
                && !party_names.contains(o.as_str())
            {
                return Err(e(format!(
                    "payment method {}: owner {o:?} not in setup.parties",
                    i.label
                )));
            }
            for c in [&i.last4].into_iter().flatten() {
                check_last4(c, &e)?;
            }
        }
        let mut project_names = BTreeSet::new();
        for p in &self.projects {
            if p.name.trim().is_empty() || !project_names.insert(p.name.as_str()) {
                return Err(e(format!("project {:?} is named twice", p.name)));
            }
            if let Some(k) = &p.kind
                && !matches!(k.as_str(), "internal" | "client" | "on_spec" | "other")
            {
                return Err(e(format!("project {}: kind {k:?}", p.name)));
            }
            if let Some(s) = &p.status
                && !matches!(s.as_str(), "active" | "paused" | "finished" | "abandoned")
            {
                return Err(e(format!("project {}: status {s:?}", p.name)));
            }
            if let Some(t) = &p.party
                && !party_names.contains(t.as_str())
            {
                return Err(e(format!(
                    "project {}: party {t:?} not in setup.parties",
                    p.name
                )));
            }
        }
        Ok(())
    }
}

impl ScheduleSpec {
    fn validate(&self, id: &str, codes: &BTreeSet<&str>) -> Result<(), SchemaError> {
        let e = |m: String| SchemaError::new(id, m);
        if self.id.trim().is_empty() || self.description.trim().is_empty() {
            return Err(e("a schedule has an id and a description".into()));
        }
        if !matches!(self.kind.as_str(), "prepaid_expense" | "deferred_revenue") {
            return Err(e(format!("schedule {}: kind {:?}", self.id, self.kind)));
        }
        check_amount(&self.total, false, &e)?;
        check_date(&self.start, &e)?;
        if self.months == 0 {
            return Err(e(format!("schedule {}: months is above zero", self.id)));
        }
        if !matches!(self.convention.as_str(), "daily" | "monthly_even") {
            return Err(e(format!(
                "schedule {}: convention {:?}",
                self.id, self.convention
            )));
        }
        if self.convention == "monthly_even" {
            let day = self
                .start
                .rsplit_once('-')
                .and_then(|(_, d)| d.parse::<u32>().ok());
            if day != Some(1) {
                return Err(e(format!(
                    "schedule {}: an equal-months schedule starts on the 1st",
                    self.id
                )));
            }
        }
        for c in [&self.balance_account, &self.pl_account] {
            if !codes.contains(c.as_str()) {
                return Err(e(format!("schedule {}: no account {c}", self.id)));
            }
        }
        Ok(())
    }
}

/// The party roles the specification names.
pub const ROLES: &[&str] = &[
    "member",
    "vendor",
    "customer",
    "contractor",
    "landlord",
    "employee",
    "financial_institution",
    "payout_platform",
    "tax_agency",
    "government_agency",
];

/// Why a party is not reportable (B-19).
pub const EXEMPT_REASONS: &[&str] = &[
    "corporation",
    "tax_exempt",
    "united_states",
    "state",
    "foreign_government",
    "foreign_person",
    "merchandise",
    "employee",
    "other",
];

/// A date, `YYYY-MM-DD` and a real day.
pub fn is_date(s: &str) -> bool {
    s.len() == 10 && NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok()
}

/// A month, `YYYY-MM`.
pub fn is_month(s: &str) -> bool {
    s.len() == 7 && NaiveDate::parse_from_str(&format!("{s}-01"), "%Y-%m-%d").is_ok()
}

/// An exact decimal string: digits, an optional sign and point; never an
/// exponent, never a float.
pub fn is_exact_decimal(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_digit() || c == '.' || c == '-')
        && s.matches('.').count() <= 1
        && s.parse::<Decimal>().is_ok()
}

fn decimal(s: &str, e: &impl Fn(String) -> SchemaError) -> Result<Decimal, SchemaError> {
    if !is_exact_decimal(s) {
        return Err(e(format!("{s:?} is no exact decimal string")));
    }
    s.parse()
        .map_err(|_| e(format!("{s:?} is no exact decimal string")))
}

fn check_amount(
    s: &str,
    positive: bool,
    e: &impl Fn(String) -> SchemaError,
) -> Result<(), SchemaError> {
    let d = decimal(s, e)?;
    if positive && d <= Decimal::ZERO {
        return Err(e(format!("{s:?}: amounts are positive")));
    }
    Ok(())
}

fn check_date(s: &str, e: &impl Fn(String) -> SchemaError) -> Result<(), SchemaError> {
    if !is_date(s) {
        return Err(e(format!("{s:?} is no date (YYYY-MM-DD)")));
    }
    Ok(())
}

fn check_month(s: &str, e: &impl Fn(String) -> SchemaError) -> Result<(), SchemaError> {
    if !is_month(s) {
        return Err(e(format!("{s:?} is no month (YYYY-MM)")));
    }
    Ok(())
}

fn check_last4(s: &str, e: &impl Fn(String) -> SchemaError) -> Result<(), SchemaError> {
    if s.len() != 4 || !s.bytes().all(|b| b.is_ascii_digit()) {
        return Err(e(format!("last four {s:?}")));
    }
    Ok(())
}

fn check_document(
    amount: &Option<String>,
    lines: &[DocumentLineSpec],
    projects: &BTreeSet<&str>,
    e: &impl Fn(String) -> SchemaError,
) -> Result<(), SchemaError> {
    match (amount, lines) {
        (Some(a), []) => {
            if decimal(a, e)? <= Decimal::ZERO {
                return Err(e("amounts are positive".into()));
            }
        }
        (None, [_, ..]) => {
            for l in lines {
                if decimal(&l.amount, e)? <= Decimal::ZERO {
                    return Err(e("amounts are positive".into()));
                }
                if let Some(p) = &l.project
                    && !projects.contains(p.as_str())
                {
                    return Err(e(format!("project {p:?} not in setup.projects")));
                }
            }
        }
        (Some(_), [_, ..]) => {
            return Err(e("a document has an amount or lines, exactly one".into()));
        }
        (None, []) => return Err(e("a document has an amount or lines, exactly one".into())),
    }
    Ok(())
}

/// A key only the queries that ask for it may name.
fn only(x: &Expect, e: &impl Fn(String) -> SchemaError) -> Result<(), SchemaError> {
    let q = x.query.as_str();
    let checks: [(&str, bool, &[&str]); 14] = [
        ("entry", x.entry.is_some(), &["entry", "party_part"]),
        ("event", x.event.is_some(), &["run"]),
        (
            "asset",
            x.asset.is_some(),
            &[
                "disposal",
                "depreciation_schedule",
                "register_row",
                "asset_basis",
            ],
        ),
        ("schedule", x.schedule.is_some(), &["schedule_plan"]),
        (
            "party",
            x.party.is_some(),
            &["party_part", "party_totals", "party_tin_last4"],
        ),
        ("project", x.project.is_some(), &["project"]),
        ("basis", x.basis.is_some(), &["party_totals"]),
        ("payee", x.payee.is_some(), &["vendor_1099"]),
        (
            "code",
            x.code.is_some(),
            &["rename_account", "set_tax_category"],
        ),
        ("name", x.name.is_some(), &["rename_account"]),
        ("category", x.category.is_some(), &["set_tax_category"]),
        ("account", x.account.is_some(), &["add_account"]),
        ("accepted", x.accepted.is_some(), COMMANDS),
        (
            "refused",
            x.refused.is_some(),
            &[
                "rename_account",
                "add_account",
                "set_tax_category",
                "period",
            ],
        ),
    ];
    for (key, given, queries) in checks {
        if given && !queries.contains(&q) {
            return Err(e(format!("{key} is for {} only", queries.join(" and "))));
        }
    }
    Ok(())
}

fn is_month_day(s: &str) -> bool {
    let Some((m, d)) = s.split_once('-') else {
        return false;
    };
    let (Ok(m), Ok(d)) = (m.parse::<u32>(), d.parse::<u32>()) else {
        return false;
    };
    s.len() == 5 && s.as_bytes()[2] == b'-' && NaiveDate::from_ymd_opt(2001, m, d).is_some()
}
