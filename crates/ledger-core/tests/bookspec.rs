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

use std::borrow::Borrow;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use chrono::{Datelike, NaiveDate};
use ledger_core::calendar::{PeriodDefaults, period_defaults};
use ledger_core::parties::{Basis, MoneyAccounts};
use ledger_core::settings::{Company, Source};
use ledger_core::tax_capital::TaxCapitalRow;
use ledger_core::{
    AccountType, AssetId, CashFlowParams, CashFlowSection, CashFlowStatement, CloseTo, Currency,
    Dated1099, Decision1099, DocumentLine, EntryId, EntryStatus, ExemptReason, Form1099,
    InstrumentId, InstrumentKind, Ledger, LedgerError, MemberCapital, MemberClose, Money, NewAsset,
    NewSchedule, PartyId, PartyMember, PartyShare, PaymentAccounts, ReconcilingKind, Role,
    ScheduleConvention, ScheduleId, ScheduleKind, Share, Side, TinFacts, TinType, VendorId,
    W9Facts, cash_flow, members_equity, profit_and_loss_for_period, trial_balance_as_of,
    vendor_1099_report,
};
use ledger_store::{DepreciationRun, DisposalPreview, Store, StoreError};
use rust_decimal::Decimal;

use bookspec_checker::{
    AccountSpec, Area, BasisSpec, ChangeSpec, Claims, DocumentLineSpec, Engine, Event, Example,
    Expect, Query, Refusal, RuleId, ScheduleSpec, Setup, Values,
};
use ledger_import::IifField;
use ledger_package::export_iif;
use serde_json::Value;

/// A JSON object: the shape every figure query answers in.
type JsonMap = serde_json::Map<String, Value>;

/// The `AccountType` an example's account type names.
fn account_type(a: &AccountSpec, id: &str) -> AccountType {
    match a.kind.as_str() {
        "asset" => AccountType::Asset,
        "liability" => AccountType::Liability,
        "equity" => AccountType::Equity,
        "revenue" => AccountType::Revenue,
        "expense" => AccountType::Expense,
        other => panic!("{id}: account type {other:?}"),
    }
}

/// The period `[from, to]` a period query covers, each date omitted
/// resolved by the one rule (rules.md B-30) on `setup.today`; panics on
/// `as_of`, on an omitted date without `setup.today`, and on a period the
/// rule refuses.
fn period(ex: &Example, x: &Expect) -> (NaiveDate, NaiveDate) {
    let q = x.query();
    resolved_q_period(ex, &q).unwrap_or_else(|e| panic!("{}: {}: {e}", ex.id, x.query))
}

/// [`period`], or the rule's refusal.
fn resolved_period(ex: &Example, x: &Expect) -> Result<(NaiveDate, NaiveDate), LedgerError> {
    resolved_q_period(ex, &x.query())
}

/// A query's period, or the rule's refusal.
fn resolved_q_period(ex: &Example, q: &Query) -> Result<(NaiveDate, NaiveDate), LedgerError> {
    let id = &ex.id;
    assert!(
        q.as_of.is_none(),
        "{id}: {} covers a period (from, to), not as_of",
        q.name
    );
    let from = q.from.as_deref().map(|s| date(s, id));
    let to = q.to.as_deref().map(|s| date(s, id));
    // A `period` query asks the rule itself, both dates given or not.
    if let (Some(from), Some(to)) = (from, to)
        && q.name != "period"
    {
        assert!(from <= to, "{id}: {} from {from} after to {to}", q.name);
        return Ok((from, to));
    }
    defaults(ex, &q.name).statement(from, to)
}

/// The day an as-of query is made as of (omitted: `setup.today`, rules.md
/// B-30); panics on `from` or `to`.
fn day(ex: &Example, x: &Expect) -> NaiveDate {
    let q = x.query();
    resolved_q_day(ex, &q).unwrap_or_else(|e| panic!("{}: {}: {e}", ex.id, x.query))
}

/// A query's day, or the rule's refusal.
fn resolved_q_day(ex: &Example, q: &Query) -> Result<NaiveDate, LedgerError> {
    let id = &ex.id;
    assert!(
        q.from.is_none() && q.to.is_none(),
        "{id}: {} is as of a day (as_of), not a period",
        q.name
    );
    match q.as_of.as_deref() {
        Some(s) => Ok(date(s, id)),
        None => defaults(ex, &q.name).as_of(None).pipe(Ok),
    }
}

/// Rust has no pipe; this keeps the day rule a single expression.
trait Pipe: Sized {
    fn pipe<T>(self, f: impl FnOnce(Self) -> T) -> T {
        f(self)
    }
}
impl<T> Pipe for T {}
/// What an omitted date means for `ex` (rules.md B-30): its fiscal year
/// on `setup.today`, which `query` needs.
fn defaults(ex: &Example, query: &str) -> PeriodDefaults {
    let today = ex.setup.today.as_deref().unwrap_or_else(|| {
        panic!(
            "{}: {query} omits a date: setup.today says what it means (rules.md B-30)",
            ex.id
        )
    });
    let company = Company {
        fiscal_year_start: fiscal_year_start(ex),
        ..Company::default()
    };
    period_defaults(&company, date(today, &format!("{} today", ex.id)))
}

/// `setup.fiscal_year_start`, MM-DD, as (month, day).
fn fiscal_year_start(ex: &Example) -> (u32, u32) {
    let s = &ex.setup.fiscal_year_start;
    let (m, d) = s
        .split_once('-')
        .unwrap_or_else(|| panic!("{}: fiscal_year_start {s:?}", ex.id));
    let md = (
        m.parse()
            .unwrap_or_else(|_| panic!("{}: fiscal_year_start {s:?}", ex.id)),
        d.parse()
            .unwrap_or_else(|_| panic!("{}: fiscal_year_start {s:?}", ex.id)),
    );
    assert!(
        NaiveDate::from_ymd_opt(2001, md.0, md.1).is_some(),
        "{}: fiscal_year_start {s:?}",
        ex.id
    );
    md
}

/// A refusal in the examples' words (bookspec/README.md), engine-neutral:
/// what was refused, never the engine's message.
fn refusal(e: &LedgerError) -> String {
    match e {
        LedgerError::InvalidDateRange { .. } => "end_before_start",
        LedgerError::StartAfterToday { .. } => "start_after_today",
        LedgerError::EmptyAccountName => "blank",
        LedgerError::AccountNameUnchanged { .. } => "unchanged",
        LedgerError::DuplicateAccountName { .. } => "sibling_has_it",
        LedgerError::AccountNameHoldsTin => "holds_a_tin",
        LedgerError::InvalidAccountName(_) => "not_one_line",
        LedgerError::DuplicateAccountCode(_) => "code_taken",
        LedgerError::TaxCategoryRequired { .. } => "tax_category_required",
        LedgerError::DepreciationPastDisposal(_) => "depreciation_past_disposal",
        LedgerError::AssetEntryIrreversible(..) => "entry_irreversible",
        LedgerError::AssetAlreadyDisposed(_) => "already_disposed",
        LedgerError::InvalidDepreciationPeriod(_) => "invalid_depreciation_period",
        LedgerError::InvalidSchedule(_) => "invalid_schedule",
        LedgerError::UntaggedControlLine { .. } => "untagged_control_line",
        LedgerError::UnbalancedEntry { .. } => "unbalanced",
        LedgerError::TooFewLines => "too_few_lines",
        LedgerError::NegativeAmountLine => "negative_amount",
        LedgerError::ZeroAmountLine => "zero_amount",
        LedgerError::FinerThanMinorUnit { .. } => "finer_than_minor_unit",
        LedgerError::ParentTypeMismatch { .. } => "parent_type_mismatch",
        LedgerError::EntryAlreadyReversed(..) => "already_reversed",
        LedgerError::ClosedPeriod(..) => "period_closed",
        other => return format!("another refusal: {other}"),
    }
    .to_string()
}

/// [`refusal`] of a command the books refused: the engine's reason, or the
/// store's own (none is in the examples' words).
fn store_refusal(e: &StoreError) -> String {
    match e {
        StoreError::Ledger(e) => refusal(e),
        other => format!("another refusal: {other}"),
    }
}

fn examples_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bookspec/examples")
}

fn date(s: &str, what: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap_or_else(|e| panic!("{what}: {s:?}: {e}"))
}

/// An exact decimal string: digits, an optional sign and point; never a
/// float's exponent.
fn exact(s: &str, what: &str) -> Decimal {
    assert!(
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_digit() || c == '.' || c == '-')
            && s.matches('.').count() <= 1,
        "{what}: {s:?} is not an exact decimal string"
    );
    s.parse()
        .unwrap_or_else(|e| panic!("{what}: {s:?} is not a decimal: {e}"))
}

/// An example's books: its setup, then its events as they are applied,
/// kept by the engine's own store (each command journaled as the books
/// journal it), in a directory of their own that goes when they do.
struct Books<'a> {
    ex: &'a Example,
    store: Store,
    /// Where the store keeps them.
    _dir: tempfile::TempDir,
    currency: Currency,
    /// The members, with their shares (zero when the example gives none).
    members: Vec<MemberCapital>,
    /// The members who are parties (`setup.parties` with the member
    /// role), as the settings give them to party shares and the 1099.
    party_members: Vec<PartyMember>,
    /// The directory's parties, by name.
    parties: BTreeMap<String, PartyId>,
    /// The payees' vendor records, by name.
    payees: BTreeMap<String, VendorId>,
    /// The vendors bills came from, by name (a payee's record if one).
    vendors: BTreeMap<String, VendorId>,
    /// The customers invoices went to, by name.
    customers: BTreeMap<String, u64>,
    /// The payment methods, by label.
    instruments: BTreeMap<String, InstrumentId>,
    /// The fixed assets of `setup.assets`, by id.
    assets: BTreeMap<String, AssetId>,
    /// The projects of `setup.projects`, by name.
    projects: BTreeMap<String, ledger_core::ClassId>,
    /// The schedules of `setup.schedules` and `register_schedule` events,
    /// by id.
    schedules: BTreeMap<String, ScheduleId>,
    /// The entries each event applied so far posted, in order: event N's
    /// at N - 1.
    posted: Vec<Vec<EntryId>>,
    /// Each `run_depreciation` event's run, by event.
    runs: BTreeMap<usize, DepreciationRun>,
    /// What each disposal took, by asset id, as the engine computed it
    /// just before posting it.
    disposals: BTreeMap<String, DisposalPreview>,
}

/// A store command's result, unwrapped: the example's books take it.
fn took<T>(r: Result<T, StoreError>, what: &str) -> T {
    r.unwrap_or_else(|e| panic!("{what}: {e}"))
}

/// Who an example's decisions and records are made by.
fn bookspec_source(id: &str) -> Source {
    Source {
        decided_by: "bookspec".into(),
        decided_on: date("2026-01-01", id),
        citation: None,
        note: None,
    }
}

/// A payee's 1099 decision in an example's words (bookspec/README.md).
fn decision(ex: &str, what: &str, kind: Option<&str>, reason: Option<&str>) -> Decision1099 {
    match what {
        "not_reportable" => {
            // Exactly as the README spells it: the engine's parse forgives
            // case and spaces, an example doesn't.
            let kind = kind
                .and_then(ExemptReason::parse)
                .filter(|k| Some(k.key()) == kind)
                .unwrap_or_else(|| {
                    panic!("{ex}: not_reportable needs a reason_kind, got {kind:?}")
                });
            let reason = reason.unwrap_or_else(|| panic!("{ex}: not_reportable needs a reason"));
            Decision1099::not_reportable(kind, reason).unwrap_or_else(|e| panic!("{ex}: {e}"))
        }
        "undetermined" | "1099-NEC" | "1099-MISC" => {
            assert!(
                kind.is_none() && reason.is_none(),
                "{ex}: only not_reportable takes reason_kind and reason"
            );
            match Form1099::parse(what) {
                Some(f) if what.starts_with("1099-") => Decision1099::Files(f),
                _ => Decision1099::Undetermined,
            }
        }
        other => panic!("{ex}: decision {other:?}"),
    }
}

/// The example's directory (`setup.parties`), by name: each party with
/// its roles (a vendor or customer with its record), the liabilities it
/// is the creditor of, and its TIN and W-9 facts' last four.
fn add_parties(ex: &Example, store: &mut Store) -> BTreeMap<String, PartyId> {
    let mut out = BTreeMap::new();
    for p in &ex.setup.parties {
        let what = format!("{} party {}", ex.id, p.name);
        let roles: Vec<Role> = p
            .roles
            .iter()
            .map(|r| Role::parse(r).unwrap_or_else(|| panic!("{what}: role {r:?}")))
            .collect();
        let record = |r: Role| roles.contains(&r);
        let others: Vec<Role> = roles
            .iter()
            .copied()
            .filter(|r| !matches!(r, Role::Vendor | Role::Customer))
            .collect();
        let id = match (record(Role::Vendor), record(Role::Customer)) {
            (true, true) => panic!("{what}: a vendor or a customer, not both"),
            (true, false) => {
                let v = took(store.add_vendor(&p.name), &what);
                store.ledger().party_of_vendor(v).expect("a vendor's party")
            }
            (false, true) => {
                let c = took(store.add_customer(&p.name), &what);
                store
                    .ledger()
                    .party_of_customer(c)
                    .expect("a customer's party")
            }
            (false, false) => took(store.add_party(&p.name, &others), &what),
        };
        if record(Role::Vendor) || record(Role::Customer) {
            for r in &others {
                took(store.set_party_role(id, *r, true), &what);
            }
        }
        for code in &p.creditor_of {
            let account = store
                .ledger()
                .chart()
                .find_by_code(code)
                .unwrap_or_else(|| panic!("{what}: no account {code}"))
                .id;
            took(
                store.set_party_creditor(id, account, true),
                &format!("{what}: creditor of {code}"),
            );
        }
        assert!(
            !p.tin_purged || p.tin_last4.is_some(),
            "{what}: tin_purged needs tin_last4"
        );
        if let Some(last4) = &p.tin_last4 {
            let facts = TinFacts {
                tin_type: TinType::Ssn,
                last4: last4.clone(),
                vault_record: (!p.tin_purged).then(|| "0".repeat(64)),
                purged: p.tin_purged.then(|| "purged: retention ended".into()),
            };
            took(store.set_party_tin(id, Some(facts)), &what);
        }
        if let Some(last4) = &p.w9_tin_last4 {
            let facts = W9Facts {
                received: date("2026-01-01", &what),
                entity_type: "individual/sole proprietor".into(),
                exempt_payee_code: None,
                fatca_code: None,
                tin_last4: last4.clone(),
                kept_at: "wanderbooks vault".into(),
                vault_record: None,
            };
            took(store.set_party_w9(id, Some(facts)), &what);
        }
        assert!(out.insert(p.name.clone(), id).is_none(), "{what}: twice");
    }
    out
}

/// An asset's basis facts as the books record them.
fn basis(b: &BasisSpec, what: &str) -> ledger_core::AssetBasis {
    let amount = |s: &Option<String>| s.as_deref().map(|v| exact(v, what));
    ledger_core::AssetBasis {
        rule: ledger_core::BasisRule::parse(&b.rule)
            .unwrap_or_else(|| panic!("{what}: basis rule {:?}", b.rule)),
        contributor_adjusted_basis: amount(&b.contributor_adjusted_basis),
        fmv_at_contribution: amount(&b.fmv_at_contribution),
        sec704c_method: b.sec704c_method.as_deref().map(|m| {
            ledger_core::Sec704cMethod::parse(m)
                .unwrap_or_else(|| panic!("{what}: §704(c) method {m:?}"))
        }),
        source: None,
    }
}

/// A schedule as the books register it.
fn new_schedule(l: &Ledger, s: &ScheduleSpec, currency: Currency, what: &str) -> NewSchedule {
    let account = |code: &str| {
        l.chart()
            .find_by_code(code)
            .unwrap_or_else(|| panic!("{what}: no account {code}"))
            .id
    };
    NewSchedule {
        description: s.description.clone(),
        kind: match s.kind.as_str() {
            "prepaid_expense" => ScheduleKind::PrepaidExpense,
            "deferred_revenue" => ScheduleKind::DeferredRevenue,
            other => panic!("{what}: schedule kind {other:?}"),
        },
        total: Money::new(exact(&s.total, what), currency),
        start: date(&s.start, what),
        months: s.months,
        balance_account: account(&s.balance_account),
        pl_account: account(&s.pl_account),
        convention: ScheduleConvention::parse(&s.convention)
            .unwrap_or_else(|| panic!("{what}: schedule convention {:?}", s.convention)),
    }
}

impl<'a> Books<'a> {
    /// The books an example sets up, before any event: its shape checked,
    /// its accounts, fixed assets, schedules, parties, members, payees,
    /// their dated decisions, and payment methods.
    fn blank(ex: &'a Example) -> Self {
        let currency = Currency::new(&ex.setup.currency).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let store = took(
            Store::create(&dir.path().join("books"), currency),
            &format!("{}: the books", ex.id),
        );
        Books {
            ex,
            store,
            _dir: dir,
            currency,
            members: Vec::new(),
            party_members: Vec::new(),
            parties: BTreeMap::new(),
            payees: BTreeMap::new(),
            vendors: BTreeMap::new(),
            customers: BTreeMap::new(),
            instruments: BTreeMap::new(),
            assets: BTreeMap::new(),
            projects: BTreeMap::new(),
            schedules: BTreeMap::new(),
            posted: Vec::new(),
            runs: BTreeMap::new(),
            disposals: BTreeMap::new(),
        }
    }

    /// The books an example sets up, before any event: its shape checked,
    /// its accounts, fixed assets, schedules, parties, members, payees,
    /// their dated decisions, and payment methods.
    fn setup(&mut self) {
        let ex = self.ex;
        let s = &ex.setup;
        let currency = self.currency;
        assert!(
            ex.appendix.as_deref().is_none_or(|a| a == "formats"),
            "{}: appendix {:?}",
            ex.id,
            ex.appendix
        );
        assert!(
            matches!(s.basis.as_str(), "accrual" | "cash"),
            "{}: basis {:?}",
            ex.id,
            s.basis
        );
        fiscal_year_start(ex);
        if let Some(today) = &s.today {
            date(today, &format!("{} today", ex.id));
        }
        assert!(
            matches!(
                s.rounding.as_str(),
                "half-even" | "half-away-from-zero" | "largest-remainder"
            ),
            "{}: rounding {:?}",
            ex.id,
            s.rounding
        );
        // The engine splits a close among the members by largest
        // remainder (rules.md B-13): an example closing to the members
        // says so.
        assert!(
            s.rounding == "largest-remainder"
                || !ex
                    .events
                    .iter()
                    .any(|e| e.close.as_ref().is_some_and(|c| c.to == "members")),
            "{}: a close to the members rounds by largest-remainder",
            ex.id
        );
        let id = |l: &Ledger, code: &str| {
            l.chart()
                .find_by_code(code)
                .unwrap_or_else(|| panic!("{}: no account {code}", ex.id))
                .id
        };
        for a in &s.accounts {
            let what = format!("{}: account {}", ex.id, a.code);
            // A parent is declared before its subaccounts.
            let parent = a.parent.as_deref().map(|p| id(self.store.ledger(), p));
            let account = took(
                self.store
                    .add_account(&a.code, &a.name, account_type(a, &ex.id), parent, a.header),
                &what,
            );
            if let Some(c) = &a.tax_category {
                took(self.store.set_account_tax(account, Some(c), None), &what);
            }
        }
        let first = ex
            .events
            .iter()
            .find_map(|e| e.date.as_deref())
            .map_or(date("2026-01-01", "start"), |d| date(d, "date"));
        for fa in &s.fixed_assets {
            let new = NewAsset {
                description: format!("Fixed asset on {}", fa.cost_account),
                in_service_date: first,
                cost: Money::new(Decimal::ONE, currency),
                salvage_value: Money::new(Decimal::ZERO, currency),
                useful_life_months: 12,
                cost_account: id(self.store.ledger(), &fa.cost_account),
                accum_depr_account: id(self.store.ledger(), &fa.accumulated_account),
                expense_account: id(self.store.ledger(), &fa.expense_account),
            };
            took(
                self.store.register_asset(new),
                &format!("{}: fixed asset", ex.id),
            );
        }
        let mut assets = BTreeMap::new();
        for a in &s.assets {
            let what = format!("{}: asset {}", ex.id, a.id);
            assert_eq!(a.method, "straight_line", "{what}: method");
            let new = NewAsset {
                description: a.description.clone(),
                in_service_date: date(&a.in_service, &what),
                cost: Money::new(exact(&a.cost, &what), currency),
                salvage_value: Money::new(exact(&a.salvage, &what), currency),
                useful_life_months: a.life_months,
                cost_account: id(self.store.ledger(), &a.cost_account),
                accum_depr_account: id(self.store.ledger(), &a.accumulated_account),
                expense_account: id(self.store.ledger(), &a.expense_account),
            };
            let asset = took(self.store.register_asset(new), &what);
            assert!(
                assets.insert(a.id.clone(), asset).is_none(),
                "{what}: id used twice"
            );
            if let Some(b) = &a.basis {
                took(self.store.set_asset_basis(asset, basis(b, &what)), &what);
            }
            assert!(
                a.acquired_by.is_none_or(|n| n >= 1 && n <= ex.events.len()),
                "{what}: acquired_by = {:?}: events count from 1, to {}",
                a.acquired_by,
                ex.events.len()
            );
        }
        let mut schedules = BTreeMap::new();
        for sc in &s.schedules {
            let what = format!("{}: schedule {}", ex.id, sc.id);
            let new = new_schedule(self.store.ledger(), sc, currency, &what);
            let schedule = took(self.store.register_schedule(new), &what);
            assert!(
                schedules.insert(sc.id.clone(), schedule).is_none(),
                "{what}: id used twice"
            );
        }
        let shared = s.members.iter().filter(|m| m.share.is_some()).count();
        assert!(
            shared == 0 || shared == s.members.len(),
            "{}: every member has a share, or none does",
            ex.id
        );
        let members: Vec<MemberCapital> = s
            .members
            .iter()
            .map(|m| MemberCapital {
                name: m.name.clone(),
                capital_account: id(self.store.ledger(), &m.capital_account),
                distribution_account: m
                    .distributions_account
                    .as_deref()
                    .map(|c| id(self.store.ledger(), c)),
                share: m.share.as_deref().map_or(Decimal::ZERO, |x| {
                    let share = exact(x, &format!("{} {} share", ex.id, m.name));
                    assert!(share > Decimal::ZERO, "{}: {}'s share", ex.id, m.name);
                    share
                }),
            })
            .collect();
        if shared > 0 {
            let total: Decimal = members.iter().map(|m| m.share).sum();
            assert_eq!(total, Decimal::ONE, "{}: the shares sum to 1", ex.id);
        }
        let parties = add_parties(ex, &mut self.store);
        // A member who is a party (by name, with the member role), as the
        // settings' members are (settings::Company::party_members).
        let party_members: Vec<PartyMember> = members
            .iter()
            .filter_map(|m| {
                let p = self.store.ledger().party_by_name(&m.name)?;
                p.roles.contains(&Role::Member).then_some(PartyMember {
                    party: p.id,
                    share: m.share,
                    capital: m.capital_account,
                    distributions: m.distribution_account,
                })
            })
            .collect();
        let source = bookspec_source(&ex.id);
        let mut payees = BTreeMap::new();
        for p in &s.payees {
            let what = format!("{}: payee {}", ex.id, p.name);
            let form = Form1099::parse(&p.form)
                .filter(|f| f.label() == p.form)
                .unwrap_or_else(|| panic!("{what}: form {:?}", p.form));
            // A payee in the directory with the vendor role is that
            // party's vendor record.
            let known = parties
                .get(&p.name)
                .and_then(|&id| self.store.ledger().get_party(id)?.vendor);
            let v = match known {
                Some(v) => v,
                None => took(self.store.add_vendor(&p.name), &what),
            };
            let dated = Dated1099::from_beginning(Decision1099::Files(form), Some(source.clone()));
            took(self.store.set_vendor_1099(v, &dated, Some(false)), &what);
            assert!(
                payees.insert(p.name.clone(), v).is_none(),
                "{}: payee {} twice",
                ex.id,
                p.name
            );
        }
        for c in &s.status_changes {
            let what = format!("{} status change of {}", ex.id, c.payee);
            let &v = payees
                .get(&c.payee)
                .unwrap_or_else(|| panic!("{what}: payee not in setup"));
            let dated = Dated1099 {
                effective: Some(date(&c.effective, &what)),
                decision: decision(
                    &what,
                    &c.decision,
                    c.reason_kind.as_deref(),
                    c.reason.as_deref(),
                ),
                source: Some(source.clone()),
            };
            took(self.store.set_vendor_1099(v, &dated, Some(false)), &what);
        }
        let mut projects = BTreeMap::new();
        for p in &s.projects {
            let what = format!("{}: project {}", ex.id, p.name);
            let facts = ledger_core::ProjectFacts {
                kind: p.kind.as_deref().map(|k| {
                    ledger_core::ProjectKind::parse(k)
                        .unwrap_or_else(|| panic!("{what}: kind {k:?}"))
                }),
                status: p.status.as_deref().map_or(Default::default(), |k| {
                    ledger_core::ProjectStatus::parse(k)
                        .unwrap_or_else(|| panic!("{what}: status {k:?}"))
                }),
                party: p.party.as_ref().map(|n| {
                    *parties
                        .get(n)
                        .unwrap_or_else(|| panic!("{what}: party {n:?} not in setup.parties"))
                }),
                ..Default::default()
            };
            let id = took(self.store.add_project(&p.name, &facts), &what);
            assert!(
                projects.insert(p.name.clone(), id).is_none(),
                "{what}: named twice"
            );
        }
        let mut instruments = BTreeMap::new();
        for i in &s.instruments {
            let what = format!("{} instrument {}", ex.id, i.label);
            let kind = InstrumentKind::parse(&i.kind)
                .filter(|k| k.key() == i.kind)
                .unwrap_or_else(|| panic!("{what}: kind {:?}", i.kind));
            let account = id(self.store.ledger(), &i.account);
            let owner = i.owner.as_ref().map(|o| {
                *parties
                    .get(o)
                    .unwrap_or_else(|| panic!("{what}: owner {o:?} not in setup.parties"))
            });
            let id = took(
                self.store
                    .add_instrument(kind, &i.label, i.last4.as_deref(), owner, account),
                &what,
            );
            assert!(
                instruments.insert(i.label.clone(), id).is_none(),
                "{what}: twice"
            );
        }
        self.members = members;
        self.party_members = party_members;
        self.parties = parties;
        self.vendors = payees.clone();
        self.customers = BTreeMap::new();
        self.payees = payees;
        self.instruments = instruments;
        self.assets = assets;
        self.projects = projects;
        self.schedules = schedules;
    }

    /// The books an example sets up, already loaded.
    fn new(ex: &'a Example) -> Self {
        let mut books = Self::blank(ex);
        books.setup();
        books
    }

    /// The books as they stand.
    fn l(&self) -> &Ledger {
        self.store.ledger()
    }

    fn id(&self, code: &str) -> ledger_core::AccountId {
        self.l()
            .chart()
            .find_by_code(code)
            .unwrap_or_else(|| panic!("{}: no account {code}", self.ex.id))
            .id
    }

    /// The fixed asset `setup.assets` names `name`.
    fn asset(&self, name: &str, what: &str) -> AssetId {
        *self
            .assets
            .get(name)
            .unwrap_or_else(|| panic!("{what}: no asset {name:?} in setup.assets"))
    }

    /// The entry event `event` (counting from 1) posted: an event applied
    /// already, that posted exactly one.
    fn entry_of(&self, event: usize, what: &str) -> EntryId {
        assert!(
            event >= 1 && event <= self.posted.len(),
            "{what}: event {event}: events count from 1, to the last one applied ({})",
            self.posted.len()
        );
        match self.posted[event - 1].as_slice() {
            [e] => *e,
            other => panic!(
                "{what}: event {event} posted {} entries: only an event that posts one entry names it",
                other.len()
            ),
        }
    }

    /// Event `n` (counting from 1): accepted, or refused for the reason it
    /// gives and changing nothing; the entries it posted kept as event
    /// n's, tagged with the parties, payee and payment method it names.
    /// Event `n`'s kind; panics on a shape the schema refuses.
    fn event_kind(&self, n: usize, e: &Event) -> &'static str {
        let kinds = [
            ("lines", !e.lines.is_empty()),
            ("reverses", e.reverses.is_some()),
            ("close", e.close.is_some()),
            ("bill", e.bill.is_some()),
            ("invoice", e.invoice.is_some()),
            ("write_off", e.write_off.is_some()),
            ("run_depreciation", e.run_depreciation.is_some()),
            ("dispose_asset", e.dispose_asset.is_some()),
            ("run_schedules", e.run_schedules.is_some()),
            ("register_schedule", e.register_schedule.is_some()),
        ];
        match kinds.iter().filter(|(_, given)| *given).collect::<Vec<_>>()[..] {
            [(kind, _)] => kind,
            _ => panic!(
                "{} event {n}: an event is one of lines, reverses, close, bill, invoice, write_off, run_depreciation, dispose_asset, run_schedules or register_schedule",
                self.ex.id
            ),
        }
    }

    /// The shape the event must have, checked before it is taken: what the
    /// schema says and what the loader's own words say.
    fn check_event_shape(&self, n: usize, e: &Event, kind: &str) {
        let ex = self.ex.id.as_str();
        // An entry of the example's own words: dated and described. A
        // disposal is dated; the other commands date their own entries,
        // and every command describes its own.
        let own = matches!(kind, "lines" | "reverses" | "close" | "bill" | "invoice");
        let dated = own || matches!(kind, "dispose_asset" | "write_off");
        assert_eq!(
            e.date.is_some(),
            dated,
            "{ex} event {n}: {kind} {}",
            if dated {
                "is dated (date)"
            } else {
                "dates its own entries: no date"
            }
        );
        assert_eq!(
            e.description.is_some(),
            own,
            "{ex} event {n}: {kind} {}",
            if own {
                "is described (description)"
            } else {
                "describes its own entries: no description"
            }
        );
        assert!(
            !e.older || kind == "lines",
            "{ex} event {n}: older is an entry's lines, as an earlier version wrote them"
        );
        assert!(
            e.payee.is_none() || kind == "lines",
            "{ex} event {n}: a payee is an entry's: a reversal takes its original's, a bill names its vendor, an invoice its customer, a close and a command have none"
        );
        assert!(
            (e.parties.is_empty() && e.instrument.is_none()) || kind != "reverses",
            "{ex} event {n}: a reversal takes its original's parties and payment method"
        );
        assert!(
            (e.parties.is_empty() && e.instrument.is_none()) || own,
            "{ex} event {n}: {kind} is tagged with no parties or payment method"
        );
        assert!(
            e.refused.is_none() || !matches!(kind, "close" | "bill" | "invoice"),
            "{ex} event {n}: refused is for a reversal, an entry or a command, which the books take whole or not at all"
        );
        assert!(
            !(e.older && e.refused.is_some()),
            "{ex} event {n}: an entry posted as an earlier version wrote it (older) can't also be refused"
        );
        assert!(
            e.write_off.is_none() || kind == "write_off",
            "{ex} event {n}: a write_off is its own event"
        );
    }

    /// Take event `n` as the books take it, keep what it posted and tag it
    /// with the parties, payee and payment method the example names; the
    /// engine's answer, never compared with what the example expects.
    fn apply_event(&mut self, n: usize, e: &Event) -> Result<(), StoreError> {
        let kind = self.event_kind(n, e);
        self.check_event_shape(n, e, kind);
        let what = format!("{} event {n}", self.ex.id);
        let when = e.date.as_deref().map(|d| date(d, &what));
        let description = e.description.as_deref().unwrap_or_default();
        let first = self.l().next_entry_id();
        let result = self.take(n, e, kind, when, description, &what);
        let next = self.l().next_entry_id();
        let posted: Vec<EntryId> = (first..next).collect();
        self.posted.push(posted);
        result?;
        // The assets this event's entry put on the books.
        for a in self
            .ex
            .setup
            .assets
            .iter()
            .filter(|a| a.acquired_by == Some(n))
        {
            let entry = self.entry_of(n, &format!("{what}: asset {}'s acquired_by", a.id));
            let asset = self.asset(&a.id, &what);
            took(self.store.set_asset_acquisition(asset, entry), &what);
        }
        if !e.parties.is_empty() {
            let entry = self.entry_of(n, &what);
            let shares: Vec<PartyShare> = e
                .parties
                .iter()
                .map(|sh| {
                    let party = *self
                        .parties
                        .get(&sh.party)
                        .unwrap_or_else(|| panic!("{what}: party {:?} not in setup", sh.party));
                    let share = match (&sh.amount, &sh.percent) {
                        (None, None) => Share::Derived,
                        (Some(a), None) => Share::Amount(exact(a, &self.ex.id)),
                        (None, Some(p)) => Share::Percent(exact(p, &self.ex.id)),
                        _ => panic!("{what}: a share is an amount or a percent"),
                    };
                    PartyShare { party, share }
                })
                .collect();
            took(self.store.set_entry_parties(entry, &shares), &what);
        }
        // The payee after the parties: a payee's party must be among
        // several named (the vendor tag follows the parties).
        if let Some(p) = &e.payee {
            let v = *self
                .payees
                .get(p)
                .unwrap_or_else(|| panic!("{what}: payee {p:?} not in setup"));
            let entry = self.entry_of(n, &what);
            took(self.store.set_entry_vendor(entry, Some(v)), &what);
        }
        if let Some(label) = &e.instrument {
            let i = *self
                .instruments
                .get(label)
                .unwrap_or_else(|| panic!("{what}: payment method {label:?} not in setup"));
            let entry = self.entry_of(n, &what);
            took(self.store.set_entry_instrument(entry, Some(i)), &what);
        }
        Ok(())
    }

    /// Apply event `n` as the example says and check it: accepted, or
    /// refused for the reason it gives and changing nothing.
    fn apply(&mut self, n: usize, e: &Event) {
        let ex = self.ex.id.as_str();
        let kind = self.event_kind(n, e);
        let what = format!("{ex} event {n}");
        let own = matches!(kind, "lines" | "reverses" | "close" | "bill" | "invoice");
        let head = self.store.head();
        let first = self.l().next_entry_id();
        let result = self.apply_event(n, e);
        match (result, &e.refused) {
            (Ok(()), None) => {}
            (Err(err), Some(want)) => {
                assert_eq!(store_refusal(&err), *want, "{what}: {kind} refused ({err})");
                assert_eq!(
                    self.store.head(),
                    head,
                    "{what}: a refused {kind} changes nothing"
                );
            }
            (Err(err), None) => panic!("{what}: {kind}: {err}"),
            (Ok(()), Some(want)) => panic!("{what}: {kind} accepted, expected refused: {want}"),
        }
        let next = self.l().next_entry_id();
        let posted: Vec<EntryId> = (first..next).collect();
        assert_eq!(
            &posted,
            &self.posted[n - 1],
            "{what}: what the event posted is kept"
        );
        for &id in &posted {
            assert_eq!(
                self.l().get_entry(id).map(|x| x.status()),
                Some(EntryStatus::Posted),
                "{what}: entry {id}, which it made, is posted"
            );
        }
        if (own || kind == "write_off") && e.refused.is_none() {
            assert_eq!(posted.len(), 1, "{what}: {kind} posts one entry");
        }
    }

    /// An invoice's or bill's lines: `amount` (one line, described as the
    /// event) or `lines`, exactly one; each project a `setup.projects`
    /// name.
    fn document_lines(
        &self,
        amount: &Option<String>,
        lines: &[DocumentLineSpec],
        description: &str,
        what: &str,
    ) -> Vec<DocumentLine> {
        let positive = |s: &str| {
            let a = exact(s, what);
            assert!(a > Decimal::ZERO, "{what}: amounts are positive");
            a
        };
        match (amount, lines) {
            (Some(a), []) => vec![DocumentLine {
                description: description.to_string(),
                amount: positive(a),
                class_id: None,
                account: None,
                quantity: None,
                unit_price: None,
            }],
            (None, [_, ..]) => lines
                .iter()
                .map(|l| DocumentLine {
                    description: l.description.clone().unwrap_or_else(|| description.into()),
                    amount: positive(&l.amount),
                    class_id: l.project.as_ref().map(|p| {
                        *self.projects.get(p).unwrap_or_else(|| {
                            panic!("{what}: project {p:?} not in setup.projects")
                        })
                    }),
                    account: None,
                    quantity: None,
                    unit_price: None,
                })
                .collect(),
            _ => panic!("{what}: a document has an amount or lines, exactly one"),
        }
    }

    /// Event `n`'s command, given to the books (`kind`, as [`Self::apply`]
    /// found it): whatever it posts is posted, or the books refuse it.
    fn take(
        &mut self,
        n: usize,
        e: &Event,
        kind: &str,
        when: Option<NaiveDate>,
        description: &str,
        what: &str,
    ) -> Result<(), StoreError> {
        let money = |s: &str| {
            let amount = exact(s, what);
            // Only a refused entry carries an amount the books refuse
            // (the schema checks it).
            assert!(
                amount > Decimal::ZERO || e.refused.is_some(),
                "{what}: amounts are positive"
            );
            amount
        };
        let day = || when.expect("checked: dated");
        match kind {
            "lines" if e.refused.is_some() => {
                // The books take an entry whole or not at all, so a
                // refused one leaves no draft behind: it is tried on a
                // clone, which is how the engine refuses it.
                let mut probe = self.store.ledger().clone();
                let entry = probe.create_entry(day(), description);
                for (i, line) in e.lines.iter().enumerate() {
                    let side = match line.side.as_str() {
                        "debit" => Side::Debit,
                        "credit" => Side::Credit,
                        other => panic!("{what}: side {other:?}"),
                    };
                    let amount = Money::new(money(&line.amount), self.currency);
                    let account = self.id(&line.account);
                    probe.add_line(entry, account, side, amount)?;
                    if let Some(p) = &line.project {
                        let class = *self.projects.get(p).unwrap_or_else(|| {
                            panic!("{what}: project {p:?} not in setup.projects")
                        });
                        probe.set_line_class(entry, i, Some(class))?;
                    }
                }
                Ok(probe.post_entry(entry)?)
            }
            "lines" => {
                let post = |this: &mut Self| -> Result<(), StoreError> {
                    let entry = this.store.create_entry(day(), description)?;
                    for (i, line) in e.lines.iter().enumerate() {
                        let side = match line.side.as_str() {
                            "debit" => Side::Debit,
                            "credit" => Side::Credit,
                            other => panic!("{what}: side {other:?}"),
                        };
                        let amount = Money::new(money(&line.amount), this.currency);
                        let account = this.id(&line.account);
                        this.store.add_line(entry, account, side, amount)?;
                        // Tagged as books.add_line tags it: after the line.
                        if let Some(p) = &line.project {
                            let class = *this.projects.get(p).unwrap_or_else(|| {
                                panic!("{what}: project {p:?} not in setup.projects")
                            });
                            this.store.set_line_class(entry, i, Some(class))?;
                        }
                    }
                    this.store.post_entry(entry)
                };
                if e.older {
                    ledger_core::parties::replaying(|| post(self))
                } else {
                    post(self)
                }
            }
            "reverses" => {
                let of = e.reverses.expect("checked: a reversal");
                // Events count from 1, as entries do: `reverses = 1` is
                // the first event (bookspec/README.md).
                assert!(of >= 1, "{what}: reverses = {of}: events count from 1");
                assert!(of < n, "{what}: reverses a later event");
                let entry = self.entry_of(of, what);
                self.store
                    .reverse_entry(entry, day(), description)
                    .map(|_| ())
            }
            "close" => {
                let c = e.close.as_ref().expect("checked: a close");
                let start = date(&c.start, what);
                let p = self
                    .store
                    .create_period(&format!("event {n}"), start, day())?;
                let members: Vec<MemberClose> = self
                    .members
                    .iter()
                    .map(|m| MemberClose {
                        capital: m.capital_account,
                        share: m.share,
                        distributions: None,
                    })
                    .collect();
                if c.to == "members" {
                    assert!(
                        !members.is_empty() && members.iter().all(|m| m.share > Decimal::ZERO),
                        "{what}: a close to the members needs their shares"
                    );
                }
                let to = match (c.to.as_str(), c.rounding.as_deref()) {
                    ("members", None) => CloseTo::Members(members),
                    ("members", Some("per-close")) => CloseTo::MembersPerClose(members),
                    ("retained_earnings", None) => CloseTo::RetainedEarnings { sweep: vec![] },
                    other => panic!("{what}: close {other:?}"),
                };
                let closed = self.store.close_period_to(p, &to)?;
                assert!(closed.is_some(), "{what}: the close has nothing to close");
                Ok(())
            }
            "bill" => {
                let b = e.bill.as_ref().expect("checked: a bill");
                let lines = self.document_lines(&b.amount, &b.lines, description, what);
                let vendor = match self.vendors.get(&b.vendor) {
                    Some(&v) => v,
                    None => {
                        let v = self.store.add_vendor(&b.vendor)?;
                        self.vendors.insert(b.vendor.clone(), v);
                        v
                    }
                };
                self.store
                    .create_bill(vendor, day(), day(), lines)
                    .map(|_| ())
            }
            "invoice" => {
                let i = e.invoice.as_ref().expect("checked: an invoice");
                let lines = self.document_lines(&i.amount, &i.lines, description, what);
                let customer = match self.customers.get(&i.customer) {
                    Some(&c) => c,
                    None => {
                        let c = self.store.add_customer(&i.customer)?;
                        self.customers.insert(i.customer.clone(), c);
                        c
                    }
                };
                self.store
                    .create_invoice(customer, day(), day(), lines)
                    .map(|_| ())
            }
            "write_off" => {
                let w = e.write_off.as_ref().expect("checked: a write-off");
                let invoice_entry = self.entry_of(w.invoice, what);
                let invoice = self
                    .store
                    .ledger()
                    .invoices()
                    .into_iter()
                    .find(|i| i.entry_id == invoice_entry)
                    .map(|i| i.id)
                    .unwrap_or_else(|| panic!("{what}: event {} posted no invoice", w.invoice));
                let amount = match &w.amount {
                    Some(a) => exact(a, what),
                    None => {
                        self.store
                            .ledger()
                            .invoice_state(invoice, day())
                            .expect("the invoice's figures")
                            .expect("dated by its own day")
                            .outstanding
                    }
                };
                // Each write-off's own notice: identical bytes would be one
                // record (plan P6 step 4), and a receipt evidences one entry
                // (B-38), so a second write-off with the first's letter is
                // refused before anything is journaled.
                let letter = format!(
                    "%PDF-1.4\nFinal demand for invoice {invoice}: {}\n%%EOF\n",
                    w.reason
                );
                self.store
                    .write_off_invoice(
                        invoice,
                        day(),
                        amount,
                        &w.reason,
                        Some(ledger_store::WriteOffEvidence::File {
                            file_name: "collection-letter.pdf",
                            mime_type: "application/pdf",
                            bytes: letter.as_bytes(),
                            description: Some("Evidence of worthlessness"),
                        }),
                    )
                    .map(drop)
            }
            "run_depreciation" => {
                let r = e.run_depreciation.as_ref().expect("checked: a run");
                let asset = self.asset(&r.asset, what);
                let run = self.store.run_depreciation(asset, &r.period)?;
                self.runs.insert(n, run);
                Ok(())
            }
            "dispose_asset" => {
                let d = e.dispose_asset.as_ref().expect("checked: a disposal");
                let asset = self.asset(&d.asset, what);
                let proceeds = exact(&d.proceeds, what);
                assert!(
                    proceeds >= Decimal::ZERO,
                    "{what}: proceeds aren't negative"
                );
                let proceeds = Money::new(proceeds, self.currency);
                let (into, gain_loss) =
                    (self.id(&d.proceeds_account), self.id(&d.gain_loss_account));
                // What the disposal takes, as the engine computes it before
                // posting; the disposal itself refused whenever its
                // computation is.
                let preview = self.store.dispose_preview(asset, day(), proceeds);
                let disposed = self
                    .store
                    .dispose_asset(asset, day(), proceeds, into, gain_loss);
                match (preview, disposed) {
                    (Ok(p), Ok(_)) => {
                        assert!(
                            self.disposals.insert(d.asset.clone(), p).is_none(),
                            "{what}: asset {} disposed of twice",
                            d.asset
                        );
                        Ok(())
                    }
                    (Err(p), Ok(_)) => {
                        panic!("{what}: disposed of, though its computation was refused: {p}")
                    }
                    (_, Err(err)) => Err(err),
                }
            }
            "run_schedules" => {
                let r = e.run_schedules.as_ref().expect("checked: a run");
                let through = date(&r.through, what);
                self.store.run_schedules(through).map(|_| ())
            }
            "register_schedule" => {
                let sc = e.register_schedule.as_ref().expect("checked: a schedule");
                let new = new_schedule(self.l(), sc, self.currency, what);
                let schedule = self.store.register_schedule(new)?;
                assert!(
                    self.schedules.insert(sc.id.clone(), schedule).is_none(),
                    "{what}: schedule id {} used twice",
                    sc.id
                );
                Ok(())
            }
            other => unreachable!("{other} is no event"),
        }
    }
}

impl Books<'_> {
    /// Make a change that posts no entry (after event `after`).
    /// Make a change that posts no entry (after event `after`), or say why
    /// the books refuse it.
    fn try_change(&mut self, c: &ChangeSpec) -> Result<(), Refusal> {
        let ex = &self.ex.id.clone();
        let after = c.after;
        let what = format!("{ex} change after {after}");
        match (&c.rename, &c.require_tax_category, &c.correct_asset) {
            (Some(r), None, None) => {
                let account = self.id(&r.code);
                self.store
                    .rename_account(account, &r.name, None)
                    .map(|_| ())
                    .map_err(|e| store_refusal_obj(&e))
            }
            (None, None, Some(f)) => {
                let id = self.asset(&f.asset, &what);
                let a = self.l().get_asset(id).expect("registered").clone();
                let amount = |s: &Option<String>, was: Money| {
                    s.as_deref()
                        .map_or(was, |v| Money::new(exact(v, &what), self.currency))
                };
                let fix = ledger_core::AssetCorrection {
                    cost: amount(&f.cost, a.cost),
                    salvage_value: amount(&f.salvage, a.salvage_value),
                    useful_life_months: f.life_months.unwrap_or(a.useful_life_months),
                    in_service_date: f
                        .in_service
                        .as_deref()
                        .map_or(a.in_service_date, |d| date(d, &what)),
                };
                self.store
                    .correct_asset(id, fix, &f.reason)
                    .map_err(|e| store_refusal_obj(&e))
            }
            (None, Some(policy), None) => {
                assert!(
                    matches!(policy.as_str(), "warn" | "refuse"),
                    "{what}: require_tax_category {policy:?}"
                );
                use ledger_core::settings::Change;
                let change = Change {
                    key: "policy.require_tax_category".into(),
                    value: Some(serde_json::json!(policy)),
                };
                self.store
                    .change_settings(&[change], &bookspec_source(ex))
                    .map(|_| ())
                    .map_err(|e| store_refusal_obj(&e))
            }
            _ => panic!("{what}: a change is one of rename, require_tax_category or correct_asset"),
        }
    }

    /// Make a change that posts no entry, checked: the books must take it.
    fn change(&mut self, c: &ChangeSpec) {
        let after = c.after;
        let what = format!("{} change after {after}", self.ex.id);
        self.try_change(c)
            .unwrap_or_else(|r| panic!("{what}: {}", r.reason));
    }
}

/// The refusal a store error is reported as: the examples' words when the
/// engine's reason is one of them, the engine's own otherwise.
fn store_refusal_obj(e: &StoreError) -> Refusal {
    match e {
        StoreError::Ledger(e) => Refusal::of_reason(refusal(e)),
        other => Refusal::of_reason(format!("another refusal: {other}")),
    }
}

/// The worker's refusal for an engine error the examples' words don't
/// name.
fn engine_refusal(rule: RuleId, e: impl std::fmt::Display) -> Refusal {
    Refusal::new(rule, format!("another refusal: {e}"))
}

impl Books<'_> {
    /// A refusal of a question the example did not fill in.
    fn missing(&self, what: &str) -> Refusal {
        Refusal::new(
            RuleId::new(Area::Ledger, 1),
            format!("{} names its {what}", "the query"),
        )
    }

    /// The period a query covers, or the rule's refusal.
    fn q_period(&self, q: &Query) -> Result<(NaiveDate, NaiveDate), Refusal> {
        resolved_q_period(self.ex, q).map_err(|e| Refusal::of_reason(refusal(&e)))
    }

    /// The day an as-of query is made as of.
    fn q_day(&self, q: &Query) -> Result<NaiveDate, Refusal> {
        resolved_q_day(self.ex, q).map_err(|e| Refusal::of_reason(refusal(&e)))
    }

    /// The event whose entry `entry` is, counting from 1.
    fn event_of(&self, entry: EntryId) -> Option<usize> {
        self.posted
            .iter()
            .position(|entries| entries.contains(&entry))
            .map(|i| i + 1)
    }

    /// The entry event `n` posted.
    fn entry_or_refuse(&self, n: usize) -> Result<EntryId, Refusal> {
        assert!(n >= 1 && n <= self.posted.len(), "event {n} not applied");
        match self.posted[n - 1].as_slice() {
            [entry] => Ok(*entry),
            other => Err(Refusal::new(
                RuleId::new(Area::Ledger, 1),
                format!("event {n} posted {} entries", other.len()),
            )),
        }
    }

    /// The account `code` names.
    fn account_or_refuse(&self, code: &str) -> Result<ledger_core::AccountId, Refusal> {
        self.l()
            .chart()
            .find_by_code(code)
            .map(|a| a.id)
            .ok_or_else(|| Refusal::new(RuleId::new(Area::Ledger, 1), format!("no account {code}")))
    }

    /// The payment accounts the example names.
    fn payment_accounts(&self) -> Result<PaymentAccounts, Refusal> {
        let ex = &self.ex;
        Ok(PaymentAccounts {
            cash: codes(self.l(), &ex.setup.cash_accounts, &ex.id),
            financing: codes(self.l(), &ex.setup.financing_accounts, &ex.id),
        })
    }

    /// The engine's answer to one query, as the runner compares it: exact
    /// decimal strings, never floats; dates `YYYY-MM-DD`.
    fn answer(&self, q: &Query) -> Result<Value, Refusal> {
        match q.name.as_str() {
            "cash_flow" => self.answer_cash_flow(q),
            "trial_balance" => self.answer_trial_balance(q),
            "profit_and_loss" => self.answer_profit_and_loss(q),
            "entry" => self.answer_entry(q),
            "balances" => self.answer_balances(q),
            "members_equity" => self.answer_members_equity(q),
            "k1" => self.answer_k1(q),
            "k1_tax_basis" => self.answer_k1_tax_basis(q),
            "k1_reconciliation" => self.answer_k1_reconciliation(q),
            "vendor_1099_paid" => self.answer_vendor_1099_paid(q),
            "vendor_1099" => self.answer_vendor_1099(q),
            "party_part" => self.answer_party_part(q),
            "party_totals" => self.answer_party_totals(q),
            "party_tin_last4" => self.answer_party_tin_last4(q),
            "project" => self.answer_project(q),
            "run" => self.answer_run(q),
            "disposal" => self.answer_disposal(q),
            "depreciation_schedule" => self.answer_depreciation_schedule(q),
            "schedule_plan" => self.answer_schedule_plan(q),
            "register_row" => self.answer_register_row(q),
            "register_totals" => self.answer_register_totals(q),
            "asset_basis" => self.answer_asset_basis(q),
            "period" => {
                let (from, to) = self.q_period(q)?;
                Ok(json_figures([
                    ("from".to_string(), from.to_string()),
                    ("to".to_string(), to.to_string()),
                ]))
            }
            "day" => Ok(json_figures([(
                "as_of".to_string(),
                self.q_day(q)?.to_string(),
            )])),
            "calendar_year" => Ok(json_figures([(
                "year".to_string(),
                defaults(self.ex, &q.name).calendar_year().to_string(),
            )])),
            "account_names" => Ok(json_figures(
                self.l()
                    .chart()
                    .all()
                    .map(|a| (a.code.clone(), a.name.clone())),
            )),
            "uncategorized_accounts" => Ok(Value::Array(
                ledger_core::tax::uncategorized_accounts(self.l())
                    .into_iter()
                    .map(|a| Value::String(format!("{} {}", a.code, a.name)))
                    .collect(),
            )),
            "iif_lines" | "iif_changes" => self.answer_iif(q),
            "rename_account" | "add_account" | "set_tax_category" => self.answer_command(q),
            other => Err(Refusal::new(
                RuleId::new(Area::Ledger, 1),
                format!("no answer for query {other:?}"),
            )),
        }
    }

    fn answer_cash_flow(&self, q: &Query) -> Result<Value, Refusal> {
        let (start, end) = self.q_period(q)?;
        let l = self.l();
        let params = CashFlowParams {
            start,
            end,
            cash_accounts: codes(l, &self.ex.setup.cash_accounts, &self.ex.id),
            financing_accounts: codes(l, &self.ex.setup.financing_accounts, &self.ex.id),
            investing_accounts: codes(l, &self.ex.setup.investing_accounts, &self.ex.id),
            members: self.members.clone(),
        };
        let cf = cash_flow(l, &params)
            .map_err(|e| engine_refusal(RuleId::new(Area::Statement, 3), e))?;
        let mut m = JsonMap::new();
        let put = |m: &mut JsonMap, key: &str, d: Decimal| {
            m.insert(key.to_string(), Value::String(figure(d)));
        };
        put(&mut m, "net_change", cf.net_change_in_cash);
        put(&mut m, "cash_beginning", cf.cash_beginning);
        put(&mut m, "cash_ending", cf.cash_ending);
        put(&mut m, "difference", cf.reconciles.difference);
        put(&mut m, "net_income", cf.operating_reconciliation.net_income);
        m.insert(
            "noncash.count".to_string(),
            Value::String(cf.noncash_disclosures.len().to_string()),
        );
        for (name, section) in [
            ("operating", CashFlowSection::Operating),
            ("investing", CashFlowSection::Investing),
            ("financing", CashFlowSection::Financing),
        ] {
            put(&mut m, name, cf.total(section));
            let lines = cf.lines(section);
            let entries: BTreeSet<EntryId> = lines.iter().map(|l| l.entry).collect();
            m.insert(
                format!("{name}.count"),
                Value::String(entries.len().to_string()),
            );
            for l in lines {
                let Some(n) = self.event_of(l.entry) else {
                    continue;
                };
                let key = format!("{name}.{n}");
                let amount = m
                    .get(&key)
                    .and_then(Value::as_str)
                    .and_then(|s| s.parse::<Decimal>().ok())
                    .unwrap_or_default();
                m.insert(key, Value::String(figure(amount + l.amount)));
                let key = format!("{name}.{n}.on_behalf");
                let amount = m
                    .get(&key)
                    .and_then(Value::as_str)
                    .and_then(|s| s.parse::<Decimal>().ok())
                    .unwrap_or_default();
                m.insert(key, Value::String(figure(amount + l.on_behalf)));
            }
        }
        for d in &cf.noncash_disclosures {
            if let Some(n) = self.event_of(d.entry) {
                let key = format!("noncash.{n}");
                let amount = m
                    .get(&key)
                    .and_then(Value::as_str)
                    .and_then(|s| s.parse::<Decimal>().ok())
                    .unwrap_or_default();
                m.insert(key, Value::String(figure(amount + d.amount)));
            }
        }
        for (class, amount) in cf.operating_by_class() {
            let letter = ["a", "b", "c", "d", "e", "f", "g"]
                .into_iter()
                .find(|l| operating_class(l) == Some(class))
                .expect("a class has its letter");
            put(&mut m, &format!("class.{letter}"), amount);
        }
        for line in &cf.operating_reconciliation.lines {
            put(&mut m, &reconciliation_key(line), line.amount);
        }
        Ok(Value::Object(m))
    }

    fn answer_trial_balance(&self, q: &Query) -> Result<Value, Refusal> {
        let as_of = self.q_day(q)?;
        let tb = trial_balance_as_of(self.l(), as_of);
        if !tb.balanced() {
            return Err(Refusal::new(
                RuleId::new(Area::Statement, 1),
                "the trial balance balances",
            ));
        }
        Ok(json_figures(self.ex.setup.accounts.iter().map(|a| {
            let (dr, cr) = tb
                .lines
                .iter()
                .find(|t| t.code == a.code)
                .map_or((Decimal::ZERO, Decimal::ZERO), |t| (t.debit, t.credit));
            (a.code.clone(), figure(dr - cr))
        })))
    }

    fn answer_profit_and_loss(&self, q: &Query) -> Result<Value, Refusal> {
        let (start, end) = self.q_period(q)?;
        let pl = profit_and_loss_for_period(self.l(), start, end, None)
            .map_err(|e| engine_refusal(RuleId::new(Area::Statement, 3), e))?;
        let mut m = JsonMap::new();
        for a in self
            .ex
            .setup
            .accounts
            .iter()
            .filter(|a| matches!(a.kind.as_str(), "revenue" | "expense"))
        {
            let amount: Decimal = pl
                .revenue
                .iter()
                .chain(&pl.expenses)
                .filter(|s| s.code == a.code)
                .map(|s| s.amount)
                .sum();
            m.insert(a.code.clone(), Value::String(figure(amount)));
        }
        m.insert(
            "net_income".to_string(),
            Value::String(figure(pl.net_income())),
        );
        Ok(Value::Object(m))
    }

    fn answer_entry(&self, q: &Query) -> Result<Value, Refusal> {
        let n = q.entry.ok_or_else(|| self.missing("entry"))?;
        let entry = self
            .l()
            .get_entry(self.entry_or_refuse(n)?)
            .expect("an entry an event posted");
        let mut got: BTreeMap<String, Decimal> = BTreeMap::new();
        for line in entry.lines() {
            let code = &self.l().chart().get(line.account).unwrap().code;
            let side = match line.side {
                Side::Debit => "debit",
                Side::Credit => "credit",
            };
            *got.entry(format!("{code}.{side}")).or_default() += line.amount.amount;
        }
        Ok(json_figures(got.into_iter().map(|(k, v)| (k, figure(v)))))
    }

    fn answer_balances(&self, q: &Query) -> Result<Value, Refusal> {
        let as_of = self.q_day(q)?;
        let tb = trial_balance_as_of(self.l(), as_of);
        let mut m = JsonMap::new();
        for account in self.l().chart().all() {
            let (dr, cr) = tb
                .lines
                .iter()
                .find(|t| t.code == account.code)
                .map_or((Decimal::ZERO, Decimal::ZERO), |t| (t.debit, t.credit));
            let got = match account.account_type.normal_side() {
                Side::Debit => dr - cr,
                Side::Credit => cr - dr,
            };
            m.insert(account.code.clone(), Value::String(figure(got)));
        }
        Ok(Value::Object(m))
    }

    fn answer_members_equity(&self, q: &Query) -> Result<Value, Refusal> {
        let (from, to) = self.q_period(q)?;
        let st = members_equity(self.l(), from, to, &self.members)
            .map_err(|e| engine_refusal(RuleId::new(Area::Partner, 1), e))?;
        let mut m = JsonMap::new();
        for member in &st.members {
            for (column, v) in [
                ("beginning", member.beginning),
                ("contributions", member.contributions),
                ("net_income", member.net_income),
                ("distributions", member.distributions),
                ("other", member.other),
                ("ending", member.ending),
            ] {
                m.insert(
                    format!("{}.{column}", member.name),
                    Value::String(figure(v)),
                );
            }
        }
        Ok(Value::Object(m))
    }

    fn answer_k1(&self, q: &Query) -> Result<Value, Refusal> {
        let (from, to) = self.q_period(q)?;
        let cash = codes(self.l(), &self.ex.setup.cash_accounts, &self.ex.id);
        let rows = ledger_core::k1::k1_worksheet(self.l(), from, to, &self.members, &cash)
            .map_err(|e| engine_refusal(RuleId::new(Area::Partner, 1), e))?;
        let mut m = JsonMap::new();
        for r in &rows {
            for (column, v) in [
                ("beginning", r.beginning),
                ("contributions_cash", r.contributions_cash),
                ("contributions_property", r.contributions_property),
                ("net_income", r.net_income),
                ("distributions", r.distributions),
                ("other", r.other),
                ("ending", r.ending),
            ] {
                m.insert(format!("{}.{column}", r.name), Value::String(figure(v)));
            }
        }
        Ok(Value::Object(m))
    }

    fn tax_rows(&self, from: NaiveDate, to: NaiveDate) -> Result<Vec<TaxCapitalRow>, Refusal> {
        let cash = codes(self.l(), &self.ex.setup.cash_accounts, &self.ex.id);
        ledger_core::tax_capital::tax_basis_capital(self.l(), from, to, &self.members, &cash)
            .map(|r| r.rows)
            .map_err(|e| engine_refusal(RuleId::new(Area::Partner, 4), e))
    }

    fn answer_k1_tax_basis(&self, q: &Query) -> Result<Value, Refusal> {
        let (from, to) = self.q_period(q)?;
        let mut m = JsonMap::new();
        for row in self.tax_rows(from, to)? {
            let name = &row.book.name;
            m.insert(
                format!("{name}.contributions_cash"),
                Value::String(figure(row.contributions_cash)),
            );
            for (line, tax) in [
                ("beginning", &row.beginning),
                ("contributions_property", &row.contributions_property),
                ("net_income", &row.net_income),
                ("other", &row.other),
                ("distributions", &row.distributions),
                ("ending", &row.ending),
            ] {
                m.insert(
                    format!("{name}.{line}"),
                    Value::String(tax.tax().map_or("open".to_string(), figure)),
                );
                m.insert(
                    format!("{name}.known_{line}"),
                    Value::String(figure(tax.known)),
                );
            }
        }
        Ok(Value::Object(m))
    }

    fn answer_k1_reconciliation(&self, q: &Query) -> Result<Value, Refusal> {
        let (from, to) = self.q_period(q)?;
        let mut m = JsonMap::new();
        for row in self.tax_rows(from, to)? {
            let r = &row.reconciliation;
            let total = r.total().ok_or_else(|| {
                Refusal::new(
                    RuleId::new(Area::Partner, 4),
                    "the reconciliation's total is too large",
                )
            })?;
            for (cause, v) in RECONCILING_CAUSES.into_iter().zip([
                r.beginning,
                r.built_in_gain_704c,
                r.property_without_basis,
                r.property_distributed,
                total,
            ]) {
                m.insert(
                    format!("{}.{cause}", row.book.name),
                    Value::String(figure(v)),
                );
            }
        }
        Ok(Value::Object(m))
    }

    fn answer_vendor_1099_paid(&self, q: &Query) -> Result<Value, Refusal> {
        let (from, to) = self.q_period(q)?;
        let year = chrono::Datelike::year(&to);
        let l = self.l();
        let sources = self.payment_accounts()?;
        let r = vendor_1099_report(l, year, None, &sources, &self.party_members, Some(to))
            .map_err(|e| engine_refusal(RuleId::new(Area::Tax1099, 1), e))?;
        let _ = from;
        let mut m = JsonMap::new();
        for (name, vendor) in &self.payees {
            let paid: Decimal = r
                .vendors
                .iter()
                .filter(|v| v.vendor_id == *vendor)
                .map(|v| v.paid)
                .sum();
            m.insert(name.clone(), Value::String(figure(paid)));
        }
        Ok(Value::Object(m))
    }

    fn answer_vendor_1099(&self, q: &Query) -> Result<Value, Refusal> {
        let (_, to) = self.q_period(q)?;
        let year = chrono::Datelike::year(&to);
        let name = q.payee.as_deref().ok_or_else(|| self.missing("payee"))?;
        let v = *self.payees.get(name).ok_or_else(|| {
            Refusal::new(
                RuleId::new(Area::Tax1099, 1),
                format!("payee {name:?} not in setup"),
            )
        })?;
        let sources = self.payment_accounts()?;
        let r = vendor_1099_report(
            self.l(),
            year,
            None,
            &sources,
            &self.party_members,
            Some(to),
        )
        .map_err(|e| engine_refusal(RuleId::new(Area::Tax1099, 1), e))?;
        let line = r.vendors.iter().find(|l| l.vendor_id == v).ok_or_else(|| {
            Refusal::new(
                RuleId::new(Area::Tax1099, 1),
                format!("the 1099 worksheet has no line for {name}"),
            )
        })?;
        Ok(json_figures([
            ("status".to_string(), line.status.key().to_string()),
            ("threshold".to_string(), figure(r.threshold)),
            ("paid".to_string(), figure(line.paid)),
            ("excluded_card".to_string(), figure(line.excluded_card)),
            ("undetermined".to_string(), figure(line.undetermined)),
            ("not_reportable".to_string(), figure(line.not_reportable)),
        ]))
    }

    fn answer_party_part(&self, q: &Query) -> Result<Value, Refusal> {
        let event = q.entry.ok_or_else(|| self.missing("entry"))?;
        let n = self.entry_or_refuse(event)?;
        let name = q.party.as_deref().ok_or_else(|| self.missing("party"))?;
        let party = *self.parties.get(name).ok_or_else(|| {
            Refusal::new(
                RuleId::new(Area::Partner, 3),
                format!("party {name:?} not in setup"),
            )
        })?;
        let ex = &self.ex;
        let r = ledger_core::shares::resolve(
            self.l(),
            n,
            self.l().entry_parties(n),
            &party_money(ex, self.l()),
            &self.party_members,
        )
        .map_err(|why| {
            Refusal::new(
                RuleId::new(Area::Partner, 3),
                format!("entry {n}'s shares: {why}"),
            )
        })?;
        let part = r.part(party).ok_or_else(|| {
            Refusal::new(
                RuleId::new(Area::Partner, 3),
                format!("entry {n} doesn't name {name:?}"),
            )
        })?;
        Ok(json_figures([
            ("resolved".to_string(), figure(part.amount)),
            (
                "direction".to_string(),
                r.target.direction.key().to_string(),
            ),
            ("cash".to_string(), figure(part.cash)),
            ("in_kind".to_string(), figure(part.in_kind)),
        ]))
    }

    fn answer_party_totals(&self, q: &Query) -> Result<Value, Refusal> {
        let (from, to) = self.q_period(q)?;
        let basis = q
            .basis
            .as_deref()
            .ok_or_else(|| {
                Refusal::new(
                    RuleId::new(Area::Partner, 3),
                    "party_totals names its basis",
                )
            })
            .and_then(|k| {
                Basis::parse(k).filter(|b| b.key() == k).ok_or_else(|| {
                    Refusal::new(RuleId::new(Area::Partner, 3), format!("basis {k:?}"))
                })
            })?;
        let name = q.party.as_deref().ok_or_else(|| self.missing("party"))?;
        let party = *self.parties.get(name).ok_or_else(|| {
            Refusal::new(
                RuleId::new(Area::Partner, 3),
                format!("party {name:?} not in setup"),
            )
        })?;
        let ex = &self.ex;
        let a = ledger_core::parties::activity_with(
            self.l(),
            party,
            (from, to),
            &party_money(ex, self.l()),
            &self.party_members,
        );
        if !a.unresolved.is_empty() {
            return Err(Refusal::new(
                RuleId::new(Area::Partner, 3),
                format!("{name:?}'s entries don't resolve"),
            ));
        }
        let (paid, received) = a.totals(basis);
        Ok(json_figures([
            ("paid".to_string(), figure(paid)),
            ("received".to_string(), figure(received)),
        ]))
    }

    fn answer_party_tin_last4(&self, q: &Query) -> Result<Value, Refusal> {
        let name = q.party.as_deref().ok_or_else(|| self.missing("party"))?;
        let party = *self.parties.get(name).ok_or_else(|| {
            Refusal::new(
                RuleId::new(Area::Partner, 2),
                format!("party {name:?} not in setup"),
            )
        })?;
        let got = self
            .l()
            .get_party(party)
            .and_then(|p| p.tin_last4())
            .unwrap_or_default();
        Ok(json_figures([("tin_last4".to_string(), got.to_string())]))
    }

    fn answer_project(&self, q: &Query) -> Result<Value, Refusal> {
        let name = q
            .project
            .as_deref()
            .ok_or_else(|| self.missing("project"))?;
        let id = *self.projects.get(name).ok_or_else(|| {
            Refusal::new(
                RuleId::new(Area::Statement, 5),
                format!("project {name:?} not in setup.projects"),
            )
        })?;
        let (from, to) = self.q_period(q)?;
        let f = ledger_core::project_figures(self.l(), id, from, to)
            .map_err(|e| engine_refusal(RuleId::new(Area::Statement, 5), e))?;
        let opt = |d: Option<Decimal>| d.map_or("none".to_string(), figure);
        Ok(json_figures([
            ("revenue".to_string(), figure(f.period.total_revenue)),
            ("expenses".to_string(), figure(f.period.total_expenses)),
            ("net".to_string(), figure(f.period.net_income())),
            ("to_date.revenue".to_string(), figure(f.to_date.revenue)),
            ("to_date.expenses".to_string(), figure(f.to_date.expenses)),
            ("to_date.net".to_string(), figure(f.to_date.net())),
            (
                "unrecovered_cost".to_string(),
                opt(f.to_date.unrecovered_cost),
            ),
        ]))
    }

    fn answer_run(&self, q: &Query) -> Result<Value, Refusal> {
        let n = q.event.ok_or_else(|| self.missing("event"))?;
        let run = self.runs.get(&n).ok_or_else(|| {
            Refusal::new(
                RuleId::new(Area::Asset, 1),
                format!("event {n} is no run_depreciation (or not yet run)"),
            )
        })?;
        let posted = if run.already_posted {
            Decimal::ZERO
        } else {
            run.amount
        };
        let caught_up = (posted - run.scheduled).max(Decimal::ZERO);
        Ok(json_figures([
            ("amount".to_string(), figure(posted)),
            ("caught_up".to_string(), figure(caught_up)),
        ]))
    }

    fn answer_disposal(&self, q: &Query) -> Result<Value, Refusal> {
        let name = q.asset.as_deref().ok_or_else(|| self.missing("asset"))?;
        let p = self.disposals.get(name).ok_or_else(|| {
            Refusal::new(
                RuleId::new(Area::Asset, 1),
                format!("asset {name:?} wasn't disposed of"),
            )
        })?;
        Ok(json_figures([
            (
                "final_depreciation".to_string(),
                figure(p.final_depreciation),
            ),
            (
                "catch_up_depreciation".to_string(),
                figure(p.catch_up_depreciation),
            ),
            (
                "excess_depreciation".to_string(),
                figure(p.excess_depreciation),
            ),
            ("accumulated".to_string(), figure(p.accumulated)),
            ("book_value".to_string(), figure(p.book_value)),
            ("gain_loss".to_string(), figure(p.gain_loss)),
        ]))
    }

    fn answer_depreciation_schedule(&self, q: &Query) -> Result<Value, Refusal> {
        let as_of = self.q_day(q)?;
        let name = q.asset.as_deref().ok_or_else(|| self.missing("asset"))?;
        let asset = self
            .l()
            .get_asset(self.asset(name, name))
            .expect("a registered asset");
        let lines = ledger_core::depreciation_schedule(asset, as_of)
            .map_err(|e| engine_refusal(RuleId::new(Area::Asset, 1), e))?;
        let mut m = JsonMap::new();
        for l in &lines {
            m.insert(l.period.clone(), Value::String(figure(l.amount)));
        }
        // A month the schedule doesn't reach is 0.00: from acquisition
        // (never later than in-service) through the day asked as of.
        let start = self
            .l()
            .asset_acquired_date(asset.id)
            .unwrap_or(asset.in_service_date);
        let (mut year, mut month) = (start.year(), start.month());
        while (year, month) <= (as_of.year(), as_of.month()) {
            m.entry(format!("{year:04}-{month:02}"))
                .or_insert_with(|| Value::String("0.00".to_string()));
            if month == 12 {
                year += 1;
                month = 1;
            } else {
                month += 1;
            }
        }
        let accumulated = ledger_core::accumulated_scheduled_through(asset, as_of)
            .map_err(|e| engine_refusal(RuleId::new(Area::Asset, 1), e))?;
        let book_value = ledger_core::scheduled_book_value(asset, as_of)
            .map_err(|e| engine_refusal(RuleId::new(Area::Asset, 1), e))?;
        m.insert(
            "accumulated".to_string(),
            Value::String(figure(accumulated)),
        );
        m.insert("book_value".to_string(), Value::String(figure(book_value)));
        Ok(Value::Object(m))
    }

    fn answer_schedule_plan(&self, q: &Query) -> Result<Value, Refusal> {
        let name = q
            .schedule
            .as_deref()
            .ok_or_else(|| self.missing("schedule"))?;
        let id = *self.schedules.get(name).ok_or_else(|| {
            Refusal::new(
                RuleId::new(Area::Prepaid, 1),
                format!("no schedule {name:?}"),
            )
        })?;
        let plan = self
            .l()
            .get_schedule(id)
            .expect("a registered schedule")
            .plan()
            .map_err(|e| engine_refusal(RuleId::new(Area::Prepaid, 1), e))?;
        let mut m = JsonMap::new();
        let mut total = Decimal::ZERO;
        for (d, amount) in &plan {
            m.insert(d.to_string(), Value::String(figure(*amount)));
            total += *amount;
        }
        m.insert("total".to_string(), Value::String(figure(total)));
        Ok(Value::Object(m))
    }

    fn answer_register_row(&self, q: &Query) -> Result<Value, Refusal> {
        let as_of = self.q_day(q)?;
        let name = q.asset.as_deref().ok_or_else(|| self.missing("asset"))?;
        let id = self.asset(name, name);
        let r = ledger_core::fixed_asset_register(self.l(), as_of)
            .map_err(|e| engine_refusal(RuleId::new(Area::Asset, 3), e))?;
        let row = r
            .rows
            .iter()
            .find(|row| row.asset_id == id)
            .ok_or_else(|| {
                Refusal::new(
                    RuleId::new(Area::Asset, 3),
                    "not on the register (acquired later)",
                )
            })?;
        Ok(json_figures([
            ("acquired".to_string(), row.acquired_date.to_string()),
            ("in_service".to_string(), row.in_service_date.to_string()),
            ("cost".to_string(), figure(row.cost)),
            (
                "accumulated_scheduled".to_string(),
                figure(row.accumulated_scheduled),
            ),
            (
                "accumulated_posted".to_string(),
                figure(row.accumulated_posted),
            ),
            (
                "unposted".to_string(),
                figure(row.accumulated_scheduled - row.accumulated_posted),
            ),
            ("book_value".to_string(), figure(row.book_value)),
        ]))
    }

    fn answer_register_totals(&self, q: &Query) -> Result<Value, Refusal> {
        let as_of = self.q_day(q)?;
        let r = ledger_core::fixed_asset_register(self.l(), as_of)
            .map_err(|e| engine_refusal(RuleId::new(Area::Asset, 3), e))?;
        Ok(json_figures([
            ("total_cost".to_string(), figure(r.total_cost)),
            (
                "total_accumulated_scheduled".to_string(),
                figure(r.total_accumulated_scheduled),
            ),
            (
                "total_accumulated_posted".to_string(),
                figure(r.total_accumulated_posted),
            ),
            ("total_book_value".to_string(), figure(r.total_book_value)),
            (
                "total_depreciable_basis".to_string(),
                figure(r.total_depreciable_basis),
            ),
            (
                "total_tax_basis_723".to_string(),
                figure(r.total_tax_basis_723),
            ),
            ("missing_basis".to_string(), r.missing_basis.to_string()),
            (
                "unregistered_cost".to_string(),
                figure(r.unregistered_cost()),
            ),
            (
                "unposted_depreciation".to_string(),
                figure(r.unposted_depreciation()),
            ),
            (
                "unattributed_accumulated".to_string(),
                figure(r.unattributed_accumulated()),
            ),
        ]))
    }

    fn answer_asset_basis(&self, q: &Query) -> Result<Value, Refusal> {
        let name = q.asset.as_deref().ok_or_else(|| self.missing("asset"))?;
        let a = self
            .l()
            .get_asset(self.asset(name, name))
            .expect("a registered asset");
        let f = a
            .basis_figures()
            .map_err(|e| engine_refusal(RuleId::new(Area::Asset, 4), e))?
            .ok_or_else(|| Refusal::new(RuleId::new(Area::Asset, 4), "no basis recorded"))?;
        let opt = |d: Option<Decimal>| d.map_or("none".to_string(), figure);
        let rule = a.basis.as_ref().expect("a basis recorded").rule;
        Ok(json_figures([
            ("rule".to_string(), rule.key().to_string()),
            (
                "contributor_adjusted_basis".to_string(),
                opt(a.basis.as_ref().and_then(|b| b.contributor_adjusted_basis)),
            ),
            (
                "fmv_at_contribution".to_string(),
                opt(a.fmv_at_contribution()),
            ),
            ("tax_basis_723".to_string(), opt(f.tax_basis_723)),
            ("depreciable_basis".to_string(), figure(f.depreciable_basis)),
            ("built_in_gain_704c".to_string(), opt(f.built_in_gain_704c)),
            ("sec704c_1c".to_string(), a.sec704c_1c().to_string()),
        ]))
    }

    fn answer_iif(&self, q: &Query) -> Result<Value, Refusal> {
        let as_of = self.q_day(q)?;
        let iif = export_iif(self.l(), &self.ex.setup.cash_accounts, as_of)
            .map_err(|e| engine_refusal(RuleId::new(Area::Format, 2), e))?;
        match q.name.as_str() {
            "iif_lines" => {
                let mut lines: Vec<&str> = iif.text.split("\r\n").collect();
                if lines.last() == Some(&"") {
                    lines.pop();
                }
                Ok(Value::Array(
                    lines
                        .into_iter()
                        .map(|l| Value::String(l.to_string()))
                        .collect(),
                ))
            }
            _ => Ok(Value::Array(
                iif.changes
                    .iter()
                    .map(|c| {
                        let what = match &c.field {
                            IifField::AccountName { code } => format!("name {code}"),
                            IifField::Memo { entry } => format!("memo {entry}"),
                            IifField::AccountNumber | IifField::AccountNumberLeftOut { .. } => {
                                "number".to_string()
                            }
                            IifField::AtTopLevel { code, .. } => format!("top level {code}"),
                            IifField::ClassName { class } => format!("class {class}"),
                        };
                        Value::Array(vec![
                            Value::String(what),
                            Value::String(c.books.clone()),
                            Value::String(c.written.clone()),
                        ])
                    })
                    .collect(),
            )),
        }
    }

    fn answer_command(&self, q: &Query) -> Result<Value, Refusal> {
        let ex = &self.ex;
        let mut copy = self.l().clone();
        let result: Result<(), LedgerError> = match q.name.as_str() {
            "rename_account" => {
                let code = q.code.as_deref().ok_or_else(|| self.missing("account"))?;
                let name = q.new_name.as_deref().ok_or_else(|| self.missing("name"))?;
                let account = self.account_or_refuse(code)?;
                copy.rename_account(account, name)
            }
            "add_account" => {
                let a = q.account.as_ref().ok_or_else(|| self.missing("account"))?;
                let kind = account_type(a, &ex.id);
                let parent = a
                    .parent
                    .as_deref()
                    .map(|p| self.account_or_refuse(p))
                    .transpose()?;
                copy.add_account(&a.code, &a.name, kind, parent, a.header)
                    .and_then(|id| match &a.tax_category {
                        Some(c) => copy.set_account_tax(id, Some(c), None),
                        None => Ok(()),
                    })
                    .and_then(|()| {
                        self.l().check_tax_category_required(
                            &a.code,
                            &a.name,
                            kind,
                            a.header,
                            a.tax_category.as_deref(),
                        )
                    })
            }
            "set_tax_category" => {
                let code = q.code.as_deref().ok_or_else(|| self.missing("account"))?;
                let account = self.account_or_refuse(code)?;
                let category = q.category.as_deref();
                copy.set_account_tax(account, category, None)
                    .and_then(|()| self.l().check_tax_category_kept(account, category))
            }
            other => unreachable!("{other} is no command"),
        };
        result
            .map(|()| Value::Bool(true))
            .map_err(|e| Refusal::of_reason(refusal(&e)))
    }
}

/// The `Engine` adapter: the wanderbooks implementation over `Store`, as
/// the bookspec runner drives any engine (`plan/served.md` P13 step 2).
impl Engine for Books<'_> {
    fn load(&mut self, setup: &Setup) -> Result<(), Refusal> {
        if setup != &self.ex.setup {
            return Err(Refusal::new(
                RuleId::new(Area::Ledger, 1),
                "the setup handed over is not the example's",
            ));
        }
        self.setup();
        Ok(())
    }

    fn apply(&mut self, event: &Event) -> Result<(), Refusal> {
        let n = self.posted.len() + 1;
        self.apply_event(n, event)
            .map_err(|e| store_refusal_obj(&e))
    }

    fn change(&mut self, change: &ChangeSpec) -> Result<(), Refusal> {
        self.try_change(change)
    }

    fn query(&self, q: &Query) -> Result<Value, Refusal> {
        self.answer(q)
    }

    fn claims(&self) -> Claims {
        Claims::new(
            bookspec_checker::covered_rules()
                .into_iter()
                .map(|r| r.id.clone()),
        )
    }
}

/// A JSON object of named strings (every figure an exact decimal string,
/// never a number).
fn json_figures<I>(figures: I) -> Value
where
    I: IntoIterator<Item = (String, String)>,
{
    Value::Object(
        figures
            .into_iter()
            .map(|(k, v)| (k, Value::String(v)))
            .collect(),
    )
}

/// The books an example sets up, with every event and change applied and
/// nothing checked.
fn load(ex: &Example) -> Books<'_> {
    let mut b = Books::new(ex);
    for (i, e) in ex.events.iter().enumerate() {
        b.apply(i + 1, e);
        for c in ex.changes.iter().filter(|c| c.after == i + 1) {
            b.change(c);
        }
    }
    b
}

/// The `cash_flow` query's value for `key` (bookspec/README.md), or None
/// for a key it doesn't know. A key's `<N>` is event N, whose entry
/// `entry_of` gives.
fn cash_flow_value(
    cf: &CashFlowStatement,
    key: &str,
    entry_of: &dyn Fn(usize) -> EntryId,
) -> Option<Decimal> {
    let sections = [
        ("operating", CashFlowSection::Operating),
        ("investing", CashFlowSection::Investing),
        ("financing", CashFlowSection::Financing),
    ];
    let count = |n: usize| Decimal::from(n as u64);
    Some(match key {
        "net_change" => cf.net_change_in_cash,
        "cash_beginning" => cf.cash_beginning,
        "cash_ending" => cf.cash_ending,
        "difference" => cf.reconciles.difference,
        "net_income" => cf.operating_reconciliation.net_income,
        "noncash.count" => count(cf.noncash_disclosures.len()),
        _ => {
            if let Some(&(_, s)) = sections.iter().find(|(k, _)| *k == key) {
                return Some(cf.total(s));
            }
            let (head, tail) = key.split_once('.')?;
            if head == "noncash" {
                let n = entry_of(tail.parse().ok()?);
                return Some(
                    cf.noncash_disclosures
                        .iter()
                        .filter(|d| d.entry == n)
                        .map(|d| d.amount)
                        .sum(),
                );
            }
            if head == "class" {
                // `class.<a..g>`: the operating class's total (ASC
                // 230-10-45-25's letters).
                let c = operating_class(tail)?;
                return Some(
                    cf.operating_by_class()
                        .into_iter()
                        .filter(|(k, _)| *k == c)
                        .map(|(_, a)| a)
                        .sum(),
                );
            }
            if head == "reconciliation" {
                let (kind, account) = match tail.split_once('.') {
                    Some((k, a)) => (k, Some(a)),
                    None => (tail, None),
                };
                let kind = match kind {
                    "depreciation" => ReconcilingKind::Depreciation,
                    "investing_gain_loss" => ReconcilingKind::InvestingGainLoss,
                    "noncash_through" => ReconcilingKind::NoncashThrough,
                    "other_noncash" => ReconcilingKind::OtherNoncash,
                    "balance_change" => ReconcilingKind::OperatingBalanceChange,
                    _ => return None,
                };
                return Some(
                    cf.operating_reconciliation
                        .lines
                        .iter()
                        .filter(|l| l.kind == kind && l.account.as_deref() == account)
                        .map(|l| l.amount)
                        .sum(),
                );
            }
            let &(_, s) = sections.iter().find(|(k, _)| *k == head)?;
            let lines = cf.lines(s);
            if tail == "count" {
                let entries: BTreeSet<EntryId> = lines.iter().map(|l| l.entry).collect();
                return Some(count(entries.len()));
            }
            // `<section>.<N>`, or `<section>.<N>.on_behalf`: of event N's
            // entry's cash there, what a member or lender handled for the
            // company.
            let (n, on_behalf) = match tail.split_once('.') {
                Some((n, "on_behalf")) => (n, true),
                Some(_) => return None,
                None => (tail, false),
            };
            let n = entry_of(n.parse().ok()?);
            lines
                .iter()
                .filter(|l| l.entry == n)
                .map(|l| if on_behalf { l.on_behalf } else { l.amount })
                .sum()
        }
    })
}

/// The operating class ASC 230-10-45-25's letter names.
fn operating_class(letter: &str) -> Option<ledger_core::OperatingClass> {
    use ledger_core::OperatingClass::*;
    Some(match letter {
        "a" => FromCustomers,
        "b" => InterestAndDividendsReceived,
        "c" => OtherReceipts,
        "d" => ToSuppliersAndEmployees,
        "e" => InterestPaid,
        "f" => IncomeTaxesPaid,
        "g" => OtherPayments,
        _ => return None,
    })
}

/// The reconciliation key a line answers to.
fn reconciliation_key(l: &ledger_core::ReconcilingLine) -> String {
    let kind = match l.kind {
        ReconcilingKind::Depreciation => "depreciation",
        ReconcilingKind::InvestingGainLoss => "investing_gain_loss",
        ReconcilingKind::NoncashThrough => "noncash_through",
        ReconcilingKind::OtherNoncash => "other_noncash",
        ReconcilingKind::OperatingBalanceChange => "balance_change",
    };
    match &l.account {
        Some(a) => format!("reconciliation.{kind}.{a}"),
        None => format!("reconciliation.{kind}"),
    }
}

/// Check one `cash_flow` expectation, every value it gives.
fn check_cash_flow(b: &Books<'_>, x: &Expect) {
    let (ex, l, members) = (b.ex, b.l(), &b.members);
    let (start, end) = period(ex, x);
    let values = x.figures(&ex.id);
    let params = CashFlowParams {
        start,
        end,
        cash_accounts: codes(l, &ex.setup.cash_accounts, &ex.id),
        financing_accounts: codes(l, &ex.setup.financing_accounts, &ex.id),
        investing_accounts: codes(l, &ex.setup.investing_accounts, &ex.id),
        members: members.to_vec(),
    };
    let cf = cash_flow(l, &params).unwrap_or_else(|e| panic!("{}: {e}", ex.id));
    for (key, want) in values {
        let want = exact(want, &format!("{} {key}", ex.id));
        let entry_of = |n: usize| b.entry_of(n, &format!("{} cash_flow {key}", ex.id));
        let got = cash_flow_value(&cf, key, &entry_of)
            .unwrap_or_else(|| panic!("{}: unknown cash_flow key {key:?}", ex.id));
        assert_eq!(
            got, want,
            "{} [{} .. {}] {key}",
            ex.id, params.start, params.end
        );
    }
    // A count given with entries listed: those are all the entries.
    for section in ["operating", "investing", "financing", "noncash"] {
        let listed: BTreeSet<&str> = values
            .keys()
            .filter_map(|k| k.strip_prefix(section)?.strip_prefix('.'))
            .filter(|t| *t != "count")
            .map(|t| t.split_once('.').map_or(t, |(n, _)| n))
            .collect();
        if let Some(n) = values.get(&format!("{section}.count"))
            && !listed.is_empty()
        {
            assert_eq!(
                Decimal::from(listed.len() as u64),
                exact(n, &ex.id),
                "{}: {section}.count against the entries listed",
                ex.id
            );
        }
    }
    // Class keys given: every class the statement has is one.
    if values.keys().any(|k| k.starts_with("class.")) {
        for (c, amount) in cf.operating_by_class() {
            let letter = ["a", "b", "c", "d", "e", "f", "g"]
                .into_iter()
                .find(|l| operating_class(l) == Some(c))
                .unwrap();
            assert!(
                values.contains_key(&format!("class.{letter}")),
                "{}: the statement has class.{letter} ({amount}), the example doesn't",
                ex.id
            );
        }
    }
    // Reconciliation keys given: every line the statement has is one.
    if values.keys().any(|k| k.starts_with("reconciliation.")) {
        for line in &cf.operating_reconciliation.lines {
            let key = reconciliation_key(line);
            assert!(
                values.contains_key(&key),
                "{}: the statement has {key} ({}), the example doesn't",
                ex.id,
                line.amount
            );
        }
    }
}

/// The accounts the codes name.
fn codes(l: &Ledger, cs: &[String], id: &str) -> Vec<ledger_core::AccountId> {
    cs.iter()
        .map(|c| {
            l.chart()
                .find_by_code(c)
                .unwrap_or_else(|| panic!("{id}: no account {c}"))
                .id
        })
        .collect()
}

/// Every key is one of `keys`, and every one of `keys` is given: a figure
/// query lists everything it covers, so an example can't leave a figure
/// unchecked by leaving it out.
fn exactly(ex: &Example, x: &Expect, keys: &BTreeSet<String>) {
    let given: BTreeSet<String> = x.figures(&ex.id).keys().cloned().collect();
    assert_eq!(
        &given, keys,
        "{}: {} lists exactly what it covers",
        ex.id, x.query
    );
}

/// Check one `trial_balance` expectation: each account's balance as of
/// the day, debits less credits (a credit balance negative), every
/// account listed (rules.md B-6, B-33).
fn check_trial_balance(ex: &Example, l: &Ledger, x: &Expect) {
    let as_of = day(ex, x);
    let tb = trial_balance_as_of(l, as_of);
    assert!(tb.balanced(), "{}: the trial balance balances", ex.id);
    let keys = ex.setup.accounts.iter().map(|a| a.code.clone()).collect();
    exactly(ex, x, &keys);
    for (code, want) in x.figures(&ex.id) {
        let want = exact(want, &format!("{} {code}", ex.id));
        let got: Decimal = tb
            .lines
            .iter()
            .filter(|t| &t.code == code)
            .map(|t| t.debit - t.credit)
            .sum();
        assert_eq!(got, want, "{} trial balance as of {as_of}: {code}", ex.id);
    }
}

/// Check one `profit_and_loss` expectation: each revenue and expense
/// account's activity over the period on its normal side, closing entries
/// left out, and `net_income`, revenue less expenses; every revenue and
/// expense account listed (rules.md B-14, B-33).
fn check_profit_and_loss(ex: &Example, l: &Ledger, x: &Expect) {
    let (start, end) = period(ex, x);
    let pl = profit_and_loss_for_period(l, start, end, None)
        .unwrap_or_else(|e| panic!("{}: {e}", ex.id));
    let mut keys: BTreeSet<String> = ex
        .setup
        .accounts
        .iter()
        .filter(|a| matches!(a.kind.as_str(), "revenue" | "expense"))
        .map(|a| a.code.clone())
        .collect();
    keys.insert("net_income".to_string());
    exactly(ex, x, &keys);
    for (key, want) in x.figures(&ex.id) {
        let want = exact(want, &format!("{} {key}", ex.id));
        let got = if key == "net_income" {
            pl.net_income()
        } else {
            pl.revenue
                .iter()
                .chain(&pl.expenses)
                .filter(|s| &s.code == key)
                .map(|s| s.amount)
                .sum()
        };
        assert_eq!(
            got, want,
            "{} profit and loss [{start} .. {end}]: {key}",
            ex.id
        );
    }
}

/// The 1099 report a `vendor_1099_paid` or `vendor_1099` expectation
/// asks for: the calendar year of `to`, from January 1 through `to`, out
/// of the cash and financing accounts, an entry's shares resolved with
/// the members who are parties (rules.md B-19, B-33).
fn report_1099(b: &Books<'_>, x: &Expect) -> (NaiveDate, NaiveDate, ledger_core::Vendor1099Report) {
    let (ex, l) = (b.ex, b.l());
    // A calendar-year count, never the fiscal default (rules.md B-30):
    // both dates given.
    assert!(
        x.from.is_some() && x.to.is_some(),
        "{}: {} gives from and to",
        ex.id,
        x.query
    );
    let (from, to) = period(ex, x);
    let year = chrono::Datelike::year(&to);
    assert_eq!(
        Some(from),
        NaiveDate::from_ymd_opt(year, 1, 1),
        "{}: a 1099 counts a calendar year: from is January 1 of to's year",
        ex.id
    );
    assert!(
        !ex.setup.cash_accounts.is_empty(),
        "{}: {} needs cash_accounts (what payees are paid from)",
        ex.id,
        x.query
    );
    let sources = PaymentAccounts {
        cash: codes(l, &ex.setup.cash_accounts, &ex.id),
        financing: codes(l, &ex.setup.financing_accounts, &ex.id),
    };
    let r = vendor_1099_report(l, year, None, &sources, &b.party_members, Some(to))
        .unwrap_or_else(|e| panic!("{}: {e}", ex.id));
    (from, to, r)
}

/// Check one `vendor_1099_paid` expectation: each payee's payments that
/// count toward its 1099 ([`report_1099`]), a payment card's left out,
/// every payee listed (rules.md B-19, B-33).
fn check_vendor_1099_paid(b: &Books<'_>, x: &Expect) {
    let ex = b.ex;
    let (from, to, r) = report_1099(b, x);
    exactly(ex, x, &b.payees.keys().cloned().collect());
    for (name, want) in x.figures(&ex.id) {
        let want = exact(want, &format!("{} {name}", ex.id));
        let got: Decimal = r
            .vendors
            .iter()
            .filter(|v| v.vendor_id == b.payees[name])
            .map(|v| v.paid)
            .sum();
        assert_eq!(got, want, "{} 1099 paid [{from} .. {to}]: {name}", ex.id);
    }
}

/// Check one `vendor_1099` expectation: one payee's line of the 1099
/// worksheet ([`report_1099`]): `status` (`reportable`,
/// `under_threshold`, `not_reportable`, `undetermined`), the `threshold`
/// applied, and what was `paid` (counts toward its 1099),
/// `excluded_card` (a card or payment network's, Form 1099-K),
/// `undetermined` (its payment method not recorded) and `not_reportable`
/// (paid while decided not reportable); every one given (rules.md B-19).
fn check_vendor_1099(b: &Books<'_>, x: &Expect) {
    let ex = b.ex;
    let name = x
        .payee
        .as_deref()
        .unwrap_or_else(|| panic!("{}: vendor_1099 names its payee", ex.id));
    let v = *b
        .payees
        .get(name)
        .unwrap_or_else(|| panic!("{}: payee {name:?} not in setup", ex.id));
    let (from, to, r) = report_1099(b, x);
    let line = r
        .vendors
        .iter()
        .find(|l| l.vendor_id == v)
        .unwrap_or_else(|| panic!("{}: the 1099 worksheet has no line for {name}", ex.id));
    let keys = [
        "status",
        "threshold",
        "paid",
        "excluded_card",
        "undetermined",
        "not_reportable",
    ];
    exactly(ex, x, &keys.iter().map(|k| k.to_string()).collect());
    let what = |k: &str| format!("{} 1099 [{from} .. {to}] {name}: {k}", ex.id);
    for (key, want) in x.figures(&ex.id) {
        if key == "status" {
            assert_eq!(line.status.key(), want, "{}", what(key));
            continue;
        }
        let got = match key.as_str() {
            "threshold" => r.threshold,
            "paid" => line.paid,
            "excluded_card" => line.excluded_card,
            "undetermined" => line.undetermined,
            _ => line.not_reportable,
        };
        assert_eq!(got, exact(want, &what(key)), "{}", what(key));
    }
}

/// The money party shares resolve against: the cash accounts (every
/// asset account but receivables when none are named), the cards and the
/// financing accounts (rules.md B-35 item 5).
fn party_money(ex: &Example, l: &Ledger) -> MoneyAccounts {
    ledger_core::parties::money_accounts(
        l,
        &codes(l, &ex.setup.cash_accounts, &ex.id),
        &codes(l, &ex.setup.financing_accounts, &ex.id),
    )
}

/// The party an expectation names.
fn expect_party(b: &Books<'_>, x: &Expect) -> PartyId {
    let name = x
        .party
        .as_deref()
        .unwrap_or_else(|| panic!("{}: {} names its party", b.ex.id, x.query));
    *b.parties
        .get(name)
        .unwrap_or_else(|| panic!("{}: party {name:?} not in setup.parties", b.ex.id))
}

/// Check one `party_part` expectation: entry N's part for the party
/// (rules.md B-35): `resolved`, signed as a line on the party's own
/// account (debit-positive: paid or given positive, received negative),
/// the entry's `direction` (`paid`, `received` or `neither`), and what
/// of it moved as money (`cash`) and as property (`in_kind`); every one
/// given.
fn check_party_part(b: &Books<'_>, x: &Expect) {
    let (ex, l) = (b.ex, b.l());
    assert!(
        x.from.is_none() && x.to.is_none() && x.as_of.is_none(),
        "{}: party_part asks about one entry, no dates",
        ex.id
    );
    let event = x
        .entry
        .unwrap_or_else(|| panic!("{}: a party_part query names its event", ex.id));
    let n = b.entry_of(event, &format!("{} party_part", ex.id));
    let party = expect_party(b, x);
    let r = ledger_core::shares::resolve(
        l,
        n,
        l.entry_parties(n),
        &party_money(ex, l),
        &b.party_members,
    )
    .unwrap_or_else(|why| panic!("{}: entry {n}'s shares: {why}", ex.id));
    let part = r
        .part(party)
        .unwrap_or_else(|| panic!("{}: entry {n} doesn't name {:?}", ex.id, x.party));
    let keys = ["resolved", "direction", "cash", "in_kind"];
    exactly(ex, x, &keys.iter().map(|k| k.to_string()).collect());
    let what = |k: &str| format!("{} entry {n} {:?}: {k}", ex.id, x.party);
    for (key, want) in x.figures(&ex.id) {
        if key == "direction" {
            assert_eq!(r.target.direction.key(), want, "{}", what(key));
            continue;
        }
        let got = match key.as_str() {
            "resolved" => part.amount,
            "cash" => part.cash,
            _ => part.in_kind,
        };
        assert_eq!(got, exact(want, &what(key)), "{}", what(key));
    }
}

/// Check one `party_totals` expectation: what was `paid` to and
/// `received` from the party from `from` through `to` on `basis` (`cash`:
/// money; `all`: money and property in kind), each entry's part on its
/// root original's side (rules.md B-35 item 4); both given.
fn check_party_totals(b: &Books<'_>, x: &Expect) {
    let (ex, l) = (b.ex, b.l());
    assert!(
        x.from.is_some() && x.to.is_some(),
        "{}: party_totals gives from and to",
        ex.id
    );
    let (from, to) = period(ex, x);
    let basis = x
        .basis
        .as_deref()
        .map(|k| {
            Basis::parse(k)
                .filter(|b| b.key() == k)
                .unwrap_or_else(|| panic!("{}: basis {k:?}", ex.id))
        })
        .unwrap_or_else(|| panic!("{}: party_totals names its basis", ex.id));
    let party = expect_party(b, x);
    let a = ledger_core::parties::activity_with(
        l,
        party,
        (from, to),
        &party_money(ex, l),
        &b.party_members,
    );
    assert!(
        a.unresolved.is_empty(),
        "{}: {:?}'s entries {:?} don't resolve",
        ex.id,
        x.party,
        a.unresolved
    );
    let (paid, received) = a.totals(basis);
    exactly(ex, x, &["paid".to_string(), "received".to_string()].into());
    let f = x.figures(&ex.id);
    let what = |k: &str| {
        format!(
            "{} {:?} [{from} .. {to}] {}: {k}",
            ex.id,
            x.party,
            basis.key()
        )
    };
    assert_eq!(paid, exact(&f["paid"], &what("paid")), "{}", what("paid"));
    assert_eq!(
        received,
        exact(&f["received"], &what("received")),
        "{}",
        what("received")
    );
}

/// Check one `project` expectation (rules.md B-37): the project's
/// `revenue`, `expenses` and `net` for the period (B-30 resolving omitted
/// dates), the same `to_date` (every tagged line through `to`), and its
/// `unrecovered_cost` (`"none"` unless it is on spec); every one given.
/// Through ledger_core::project_figures, as the tools and the viewer.
fn check_project(b: &Books<'_>, x: &Expect) {
    let (ex, l) = (b.ex, b.l());
    let name = x
        .project
        .as_ref()
        .unwrap_or_else(|| panic!("{}: a project query names its project", ex.id));
    let id = *b
        .projects
        .get(name)
        .unwrap_or_else(|| panic!("{}: project {name:?} not in setup.projects", ex.id));
    let (from, to) = period(ex, x);
    let f =
        ledger_core::project_figures(l, id, from, to).unwrap_or_else(|e| panic!("{}: {e}", ex.id));
    let keys: BTreeSet<String> = [
        "revenue",
        "expenses",
        "net",
        "to_date.revenue",
        "to_date.expenses",
        "to_date.net",
        "unrecovered_cost",
    ]
    .iter()
    .map(|k| k.to_string())
    .collect();
    exactly(ex, x, &keys);
    let what = |k: &str| format!("{} project {name:?} [{from} .. {to}]: {k}", ex.id);
    for (key, want) in x.figures(&ex.id) {
        let got = match key.as_str() {
            "revenue" => Some(f.period.total_revenue),
            "expenses" => Some(f.period.total_expenses),
            "net" => Some(f.period.net_income()),
            "to_date.revenue" => Some(f.to_date.revenue),
            "to_date.expenses" => Some(f.to_date.expenses),
            "to_date.net" => Some(f.to_date.net()),
            _ => f.to_date.unrecovered_cost,
        };
        match (want.as_str(), got) {
            ("none", None) => {}
            ("none", Some(g)) => panic!("{}: {g}, expected none", what(key)),
            (w, Some(g)) => assert_eq!(g, exact(w, &what(key)), "{}", what(key)),
            (w, None) => panic!("{}: none, expected {w}", what(key)),
        }
    }
}

/// Check one `party_tin_last4` expectation: the last four of the party's
/// TIN the books keep (its TIN facts', else its W-9's; `""` for none:
/// rules.md B-26).
fn check_party_tin_last4(b: &Books<'_>, x: &Expect) {
    let ex = b.ex;
    assert!(
        x.from.is_none() && x.to.is_none() && x.as_of.is_none(),
        "{}: party_tin_last4 takes no dates",
        ex.id
    );
    let party = expect_party(b, x);
    exactly(ex, x, &["tin_last4".to_string()].into());
    let got = b
        .l()
        .get_party(party)
        .and_then(|p| p.tin_last4())
        .unwrap_or_default();
    assert_eq!(
        got,
        x.figures(&ex.id)["tin_last4"],
        "{}: {:?}'s TIN last four",
        ex.id,
        x.party
    );
}

/// Check one `k1` expectation: the K-1 worksheet over the period (rules.md
/// B-13; Schedule K-1 Item L), `"<member>.<column>"` for the columns
/// `beginning`, `contributions_cash`, `contributions_property`,
/// `net_income`, `distributions`, `other` and `ending`; the figures named
/// are the example's choice. It needs `cash_accounts` (what is money
/// rather than property).
fn check_k1(ex: &Example, l: &Ledger, members: &[MemberCapital], x: &Expect) {
    let (from, to) = period(ex, x);
    assert!(
        !members.is_empty() && members.iter().all(|m| m.share > Decimal::ZERO),
        "{}: k1 needs the members and their shares",
        ex.id
    );
    assert!(
        !ex.setup.cash_accounts.is_empty(),
        "{}: k1 needs cash_accounts (what is money rather than property)",
        ex.id
    );
    let cash = codes(l, &ex.setup.cash_accounts, &ex.id);
    let rows = ledger_core::k1::k1_worksheet(l, from, to, members, &cash)
        .unwrap_or_else(|e| panic!("{}: {e}", ex.id));
    let mut got = BTreeMap::new();
    for r in &rows {
        for (column, v) in [
            ("beginning", r.beginning),
            ("contributions_cash", r.contributions_cash),
            ("contributions_property", r.contributions_property),
            ("net_income", r.net_income),
            ("distributions", r.distributions),
            ("other", r.other),
            ("ending", r.ending),
        ] {
            got.insert(format!("{}.{column}", r.name), v);
        }
    }
    for (key, want) in x.figures(&ex.id) {
        let want = exact(want, &format!("{} {key}", ex.id));
        let got = got
            .get(key)
            .unwrap_or_else(|| panic!("{}: k1 has no {key}", ex.id));
        assert_eq!(*got, want, "{} K-1 [{from} .. {to}]: {key}", ex.id);
    }
}

/// The formats appendix's queries, and the test (another crate's) that
/// checks them against the engine.
const FORMAT_QUERIES: [(&str, &str); 2] = [
    (
        "iif_lines",
        "ledger-package the_bookspec_iif_example_matches_the_engine",
    ),
    (
        "iif_changes",
        "ledger-package the_bookspec_iif_example_matches_the_engine",
    ),
];

/// A format query's shape, checked here; its values are checked by
/// [`FORMAT_QUERIES`]' test.
fn check_format_shape(ex: &Example, x: &Expect) {
    assert_eq!(
        ex.appendix.as_deref(),
        Some("formats"),
        "{}: {} is a format's: the example is in appendix \"formats\"",
        ex.id,
        x.query
    );
    // ledger-package reads the as_of itself: given, never defaulted.
    assert!(x.as_of.is_some(), "{}: {} gives as_of", ex.id, x.query);
    day(ex, x);
    assert!(
        x.after.is_none(),
        "{}: {}: a format query is of the books after every event",
        ex.id,
        x.query
    );
    match (x.query.as_str(), x.values(&ex.id)) {
        ("iif_lines", Values::Lines(_)) => {}
        ("iif_changes", Values::Rows(rows)) => {
            for r in rows {
                assert_eq!(r.len(), 3, "{}: iif_changes row {r:?}", ex.id);
            }
        }
        (q, _) => panic!("{}: {q}: values of the wrong shape", ex.id),
    }
}

/// Check one `entry` expectation: the lines of event N's entry, `"<code>.debit"` and
/// `"<code>.credit"` each summing that account's lines on that side,
/// every line of the entry named.
fn check_entry(b: &Books<'_>, x: &Expect) {
    let (ex, l) = (b.ex, b.l());
    assert!(
        x.from.is_none() && x.to.is_none() && x.as_of.is_none(),
        "{}: entry asks about one entry, no dates",
        ex.id
    );
    let n = x
        .entry
        .unwrap_or_else(|| panic!("{}: an entry query names its event", ex.id));
    let entry = l
        .get_entry(b.entry_of(n, &format!("{} entry {n}", ex.id)))
        .expect("an entry an event posted");
    let mut got: BTreeMap<String, Decimal> = BTreeMap::new();
    for line in entry.lines() {
        let code = &l.chart().get(line.account).unwrap().code;
        let side = match line.side {
            Side::Debit => "debit",
            Side::Credit => "credit",
        };
        *got.entry(format!("{code}.{side}")).or_default() += line.amount.amount;
    }
    let keys = got.keys().cloned().collect();
    exactly(ex, x, &keys);
    for (key, want) in x.figures(&ex.id) {
        let want = exact(want, &format!("{} {key}", ex.id));
        assert_eq!(got[key], want, "{} entry {n}: {key}", ex.id);
    }
}

/// Check one `balances` expectation: each account named, its balance as
/// of the day on its normal side (so a member's capital after a loss is
/// negative); the accounts named are the example's choice.
fn check_balances(ex: &Example, l: &Ledger, x: &Expect) {
    let as_of = day(ex, x);
    let tb = trial_balance_as_of(l, as_of);
    for (code, want) in x.figures(&ex.id) {
        let want = exact(want, &format!("{} {code}", ex.id));
        let account = l
            .chart()
            .find_by_code(code)
            .unwrap_or_else(|| panic!("{}: no account {code}", ex.id));
        let (dr, cr) = tb
            .lines
            .iter()
            .find(|t| &t.code == code)
            .map_or((Decimal::ZERO, Decimal::ZERO), |t| (t.debit, t.credit));
        let got = match account.account_type.normal_side() {
            Side::Debit => dr - cr,
            Side::Credit => cr - dr,
        };
        assert_eq!(got, want, "{} balance as of {as_of}: {code}", ex.id);
    }
}

/// Check one `members_equity` expectation: the statement of changes in
/// members' equity over the period, `"<member>.<column>"` for the columns
/// `beginning`, `contributions`, `net_income`, `distributions`, `other`
/// and `ending`; the figures named are the example's choice (rules.md
/// B-13).
fn check_members_equity(ex: &Example, l: &Ledger, members: &[MemberCapital], x: &Expect) {
    let (from, to) = period(ex, x);
    assert!(
        !members.is_empty() && members.iter().all(|m| m.share > Decimal::ZERO),
        "{}: members_equity needs the members and their shares",
        ex.id
    );
    let st = members_equity(l, from, to, members).unwrap_or_else(|e| panic!("{}: {e}", ex.id));
    let mut got = BTreeMap::new();
    for m in &st.members {
        for (column, v) in [
            ("beginning", m.beginning),
            ("contributions", m.contributions),
            ("net_income", m.net_income),
            ("distributions", m.distributions),
            ("other", m.other),
            ("ending", m.ending),
        ] {
            got.insert(format!("{}.{column}", m.name), v);
        }
    }
    for (key, want) in x.figures(&ex.id) {
        let want = exact(want, &format!("{} {key}", ex.id));
        let got = got
            .get(key)
            .unwrap_or_else(|| panic!("{}: members_equity has no {key}", ex.id));
        assert_eq!(
            *got, want,
            "{} members' equity [{from} .. {to}]: {key}",
            ex.id
        );
    }
}

/// No dates: a query about one thing the books did, not about a day.
fn undated(ex: &Example, x: &Expect) {
    assert!(
        x.from.is_none() && x.to.is_none() && x.as_of.is_none(),
        "{}: {} takes no dates",
        ex.id,
        x.query
    );
}

/// Check one `run` expectation: what the `run_depreciation` event `event`
/// posted (`amount`: zero when nothing was due, or the month already had
/// its run) and, of it, what it caught up beyond its own month's
/// depreciation (`caught_up`), both listed (rules.md B-15).
fn check_run(b: &Books<'_>, x: &Expect) {
    let ex = b.ex;
    undated(ex, x);
    let n = x
        .event
        .unwrap_or_else(|| panic!("{}: run names its event", ex.id));
    let run = b.runs.get(&n).unwrap_or_else(|| {
        panic!(
            "{}: event {n} is no run_depreciation (or not yet run)",
            ex.id
        )
    });
    let posted = if run.already_posted {
        Decimal::ZERO
    } else {
        run.amount
    };
    let caught_up = (posted - run.scheduled).max(Decimal::ZERO);
    exactly(
        ex,
        x,
        &["amount".to_string(), "caught_up".to_string()].into(),
    );
    for (key, got) in [("amount", posted), ("caught_up", caught_up)] {
        let want = exact(&x.figures(&ex.id)[key], &format!("{} {key}", ex.id));
        assert_eq!(got, want, "{} run of event {n}: {key}", ex.id);
    }
}

/// Check one `disposal` expectation: what the disposal of `asset` took, as
/// the engine computed it when posting it (rules.md B-15), every figure
/// listed: `final_depreciation` (posted in its own entry before the
/// disposal), `catch_up_depreciation` and `excess_depreciation` (charged,
/// and credited back, in the disposal entry), `accumulated` (cleared),
/// `book_value` and `gain_loss` (a gain positive).
fn check_disposal(b: &Books<'_>, x: &Expect) {
    let ex = b.ex;
    undated(ex, x);
    let asset = x
        .asset
        .as_deref()
        .unwrap_or_else(|| panic!("{}: disposal names its asset", ex.id));
    let p = b
        .disposals
        .get(asset)
        .unwrap_or_else(|| panic!("{}: asset {asset:?} wasn't disposed of", ex.id));
    let figures = [
        ("final_depreciation", p.final_depreciation),
        ("catch_up_depreciation", p.catch_up_depreciation),
        ("excess_depreciation", p.excess_depreciation),
        ("accumulated", p.accumulated),
        ("book_value", p.book_value),
        ("gain_loss", p.gain_loss),
    ];
    exactly(ex, x, &figures.iter().map(|(k, _)| k.to_string()).collect());
    for (key, got) in figures {
        let want = exact(&x.figures(&ex.id)[key], &format!("{} {key}", ex.id));
        assert_eq!(got, want, "{} disposal of {asset}: {key}", ex.id);
    }
}

/// A month key, "YYYY-MM", checked.
fn month_key(key: &str, what: &str) {
    assert!(
        key.len() == 7 && NaiveDate::parse_from_str(&format!("{key}-01"), "%Y-%m-%d").is_ok(),
        "{what}: {key:?} is no month (YYYY-MM)"
    );
}

/// Check one `depreciation_schedule` expectation: the schedule of `asset`
/// through `as_of` (rules.md B-15), each month named (`"YYYY-MM"`) its
/// depreciation, 0.00 for a month the schedule doesn't reach;
/// `accumulated` and `book_value` through `as_of`; the example names the
/// figures it shows.
fn check_depreciation_schedule(b: &Books<'_>, x: &Expect) {
    let ex = b.ex;
    let as_of = day(ex, x);
    let name = x
        .asset
        .as_deref()
        .unwrap_or_else(|| panic!("{}: depreciation_schedule names its asset", ex.id));
    let what = format!("{} depreciation_schedule of {name} through {as_of}", ex.id);
    let asset = b
        .l()
        .get_asset(b.asset(name, &what))
        .expect("a registered asset");
    let lines =
        ledger_core::depreciation_schedule(asset, as_of).unwrap_or_else(|e| panic!("{what}: {e}"));
    for (key, want) in x.figures(&ex.id) {
        let want = exact(want, &format!("{what}: {key}"));
        let got = match key.as_str() {
            "accumulated" => ledger_core::accumulated_scheduled_through(asset, as_of),
            "book_value" => ledger_core::scheduled_book_value(asset, as_of),
            month => {
                month_key(month, &what);
                Ok(lines
                    .iter()
                    .find(|l| l.period == month)
                    .map_or(Decimal::ZERO, |l| l.amount))
            }
        }
        .unwrap_or_else(|e| panic!("{what}: {key}: {e}"));
        assert_eq!(got, want, "{what}: {key}");
    }
}

/// Check one `schedule_plan` expectation: what the schedule's plan
/// recognizes at each month end named (`"YYYY-MM-DD"`, 0.00 at a month end
/// the plan doesn't post), and `total`, all it recognizes (rules.md B-16);
/// the example names the figures it shows.
fn check_schedule_plan(b: &Books<'_>, x: &Expect) {
    let ex = b.ex;
    undated(ex, x);
    let name = x
        .schedule
        .as_deref()
        .unwrap_or_else(|| panic!("{}: schedule_plan names its schedule", ex.id));
    let what = format!("{} schedule_plan of {name}", ex.id);
    let id = *b
        .schedules
        .get(name)
        .unwrap_or_else(|| panic!("{what}: no schedule {name:?}"));
    let plan = b
        .l()
        .get_schedule(id)
        .expect("a registered schedule")
        .plan()
        .unwrap_or_else(|e| panic!("{what}: {e}"));
    for (key, want) in x.figures(&ex.id) {
        let want = exact(want, &format!("{what}: {key}"));
        let got: Decimal = if key == "total" {
            plan.iter().map(|(_, a)| *a).sum()
        } else {
            let d = date(key, &what);
            assert_eq!(
                d,
                ledger_core::schedules::month_end(d),
                "{what}: {key} is no month end"
            );
            plan.iter().filter(|(m, _)| *m == d).map(|(_, a)| *a).sum()
        };
        assert_eq!(got, want, "{what}: {key}");
    }
}

/// Check expectations whose values are named figures: each key one of
/// `figures`' (an unknown one refused), its value the figure's text.
fn named<K: Borrow<str> + Ord>(
    ex: &Example,
    x: &Expect,
    what: &str,
    figures: &BTreeMap<K, String>,
) {
    for (key, want) in x.figures(&ex.id) {
        let got = figures
            .get(key.as_str())
            .unwrap_or_else(|| panic!("{what}: no figure {key:?}"));
        assert_eq!(got, want, "{what}: {key}");
    }
}

/// A decimal figure as an example writes it: two places at least, as the
/// books keep money (exact; "0.00", "-14.20").
fn figure(d: Decimal) -> String {
    let mut d = d.normalize();
    if d.scale() < 2 {
        d.rescale(2);
    }
    d.to_string()
}

/// Check one `register_row` expectation: the asset's row of the fixed
/// asset register as of `as_of` (rules.md B-15 rules 7 and 8, B-24):
/// `acquired` and `in_service` (`YYYY-MM-DD`), `cost`,
/// `accumulated_scheduled` (the schedule's), `accumulated_posted` (what
/// the books carry), `unposted` (the one less the other), `book_value`
/// (cost less posted); each 0.00 once disposed; the example names the
/// figures it shows. An asset not on the register by `as_of` (acquired
/// later) is refused.
fn check_register_row(b: &Books<'_>, x: &Expect) {
    let ex = b.ex;
    let as_of = day(ex, x);
    let name = x
        .asset
        .as_deref()
        .unwrap_or_else(|| panic!("{}: register_row names its asset", ex.id));
    let what = format!("{} register_row of {name} as of {as_of}", ex.id);
    let id = b.asset(name, &what);
    let r =
        ledger_core::fixed_asset_register(b.l(), as_of).unwrap_or_else(|e| panic!("{what}: {e}"));
    let row = r
        .rows
        .iter()
        .find(|row| row.asset_id == id)
        .unwrap_or_else(|| panic!("{what}: not on the register (acquired later)"));
    let figures = BTreeMap::from([
        ("acquired", row.acquired_date.to_string()),
        ("in_service", row.in_service_date.to_string()),
        ("cost", figure(row.cost)),
        ("accumulated_scheduled", figure(row.accumulated_scheduled)),
        ("accumulated_posted", figure(row.accumulated_posted)),
        (
            "unposted",
            figure(row.accumulated_scheduled - row.accumulated_posted),
        ),
        ("book_value", figure(row.book_value)),
    ]);
    named(ex, x, &what, &figures);
}

/// Check one `register_totals` expectation: the fixed asset register's
/// totals as of `as_of`, over the assets held (rules.md B-24, B-25):
/// `total_cost`, `total_accumulated_scheduled`, `total_accumulated_posted`,
/// `total_book_value`, `total_depreciable_basis`, `total_tax_basis_723`,
/// `missing_basis` (how many held have none), and how it reconciles to
/// the books: `unregistered_cost`, `unposted_depreciation`,
/// `unattributed_accumulated`; the example names the figures it shows.
fn check_register_totals(b: &Books<'_>, x: &Expect) {
    let ex = b.ex;
    let as_of = day(ex, x);
    let what = format!("{} register_totals as of {as_of}", ex.id);
    let r =
        ledger_core::fixed_asset_register(b.l(), as_of).unwrap_or_else(|e| panic!("{what}: {e}"));
    let figures = BTreeMap::from([
        ("total_cost", figure(r.total_cost)),
        (
            "total_accumulated_scheduled",
            figure(r.total_accumulated_scheduled),
        ),
        (
            "total_accumulated_posted",
            figure(r.total_accumulated_posted),
        ),
        ("total_book_value", figure(r.total_book_value)),
        ("total_depreciable_basis", figure(r.total_depreciable_basis)),
        ("total_tax_basis_723", figure(r.total_tax_basis_723)),
        ("missing_basis", r.missing_basis.to_string()),
        ("unregistered_cost", figure(r.unregistered_cost())),
        ("unposted_depreciation", figure(r.unposted_depreciation())),
        (
            "unattributed_accumulated",
            figure(r.unattributed_accumulated()),
        ),
    ]);
    named(ex, x, &what, &figures);
}

/// Check one `asset_basis` expectation: the asset's basis facts and the
/// tax figures they give (rules.md B-25): `rule`,
/// `contributor_adjusted_basis`, `fmv_at_contribution`, `tax_basis_723`,
/// `depreciable_basis`, `built_in_gain_704c` (a loss negative), each
/// `"none"` where the asset has none (a purchase's §723 basis), and
/// `sec704c_1c` (`"true"`: a built-in loss, which counts only for the
/// contributing member, IRC §704(c)(1)(C)); the example names the figures
/// it shows.
fn check_asset_basis(b: &Books<'_>, x: &Expect) {
    let ex = b.ex;
    undated(ex, x);
    let name = x
        .asset
        .as_deref()
        .unwrap_or_else(|| panic!("{}: asset_basis names its asset", ex.id));
    let what = format!("{} asset_basis of {name}", ex.id);
    let a = b
        .l()
        .get_asset(b.asset(name, &what))
        .expect("a registered asset");
    let f = a
        .basis_figures()
        .unwrap_or_else(|e| panic!("{what}: {e}"))
        .unwrap_or_else(|| panic!("{what}: no basis recorded"));
    let opt = |d: Option<Decimal>| d.map_or("none".to_string(), figure);
    let rule = a.basis.as_ref().expect("a basis recorded").rule;
    let figures = BTreeMap::from([
        ("rule", rule.key().to_string()),
        (
            "contributor_adjusted_basis",
            opt(a.basis.as_ref().and_then(|b| b.contributor_adjusted_basis)),
        ),
        ("fmv_at_contribution", opt(a.fmv_at_contribution())),
        ("tax_basis_723", opt(f.tax_basis_723)),
        ("depreciable_basis", figure(f.depreciable_basis)),
        ("built_in_gain_704c", opt(f.built_in_gain_704c)),
        ("sec704c_1c", a.sec704c_1c().to_string()),
    ]);
    named(ex, x, &what, &figures);
}

/// The tax-basis capital worksheet for `[from, to]` (rules.md B-36), a
/// row a member, as `k1` reads the book one: it needs the members and
/// their shares, and `cash_accounts`.
fn tax_capital_rows(b: &Books<'_>, x: &Expect, what: &str) -> Vec<TaxCapitalRow> {
    let ex = b.ex;
    let (from, to) = period(ex, x);
    assert!(
        !b.members.is_empty() && b.members.iter().all(|m| m.share > Decimal::ZERO),
        "{what}: needs the members and their shares"
    );
    assert!(
        !ex.setup.cash_accounts.is_empty(),
        "{what}: the K-1 worksheet needs cash_accounts"
    );
    let cash = codes(b.l(), &ex.setup.cash_accounts, &ex.id);
    ledger_core::tax_capital::tax_basis_capital(b.l(), from, to, &b.members, &cash)
        .unwrap_or_else(|e| panic!("{what}: {e}"))
        .rows
}

/// Check one `k1_tax_basis` expectation: each member's capital account
/// for `[from, to]` on the tax basis, as Schedule K-1 Item L reports it
/// (rules.md B-36), `"<member>.<line>"` for the lines `beginning`,
/// `contributions_cash`, `contributions_property`, `net_income`, `other`,
/// `distributions` and `ending`, beside `k1`'s book figures; a line the
/// books can't know is `"open"`, with what they know so far as
/// `"<member>.known_<line>"`. The example names the figures it shows.
fn check_k1_tax_basis(b: &Books<'_>, x: &Expect) {
    let ex = b.ex;
    let what = format!("{} k1_tax_basis", ex.id);
    let mut figures = BTreeMap::new();
    for row in tax_capital_rows(b, x, &what) {
        let m = &row.book.name;
        figures.insert(
            format!("{m}.contributions_cash"),
            figure(row.contributions_cash),
        );
        for (line, tax) in [
            ("beginning", &row.beginning),
            ("contributions_property", &row.contributions_property),
            ("net_income", &row.net_income),
            ("other", &row.other),
            ("distributions", &row.distributions),
            ("ending", &row.ending),
        ] {
            figures.insert(
                format!("{m}.{line}"),
                tax.tax().map_or("open".to_string(), figure),
            );
            figures.insert(format!("{m}.known_{line}"), figure(tax.known));
        }
    }
    named(ex, x, &what, &figures);
}

/// The causes `k1_reconciliation` reports, each member's every one.
const RECONCILING_CAUSES: [&str; 5] = [
    "beginning",
    "built_in_gain_704c",
    "property_without_basis",
    "property_distributed",
    "total",
];

/// Check one `k1_reconciliation` expectation: each member's book capital
/// less tax-basis capital at the end of `[from, to]`, by cause (rules.md
/// B-36), `"<member>.<cause>"` for `beginning`, `built_in_gain_704c`
/// (their share of the §704(c) gain or loss at contribution),
/// `property_without_basis`, `property_distributed` and `total`: the
/// example names the members it shows, each with every cause.
fn check_k1_reconciliation(b: &Books<'_>, x: &Expect) {
    let ex = b.ex;
    let what = format!("{} k1_reconciliation", ex.id);
    let mut figures = BTreeMap::new();
    for row in tax_capital_rows(b, x, &what) {
        let r = &row.reconciliation;
        let total = r
            .total()
            .unwrap_or_else(|| panic!("{what}: the total is too large"));
        for (cause, v) in RECONCILING_CAUSES.into_iter().zip([
            r.beginning,
            r.built_in_gain_704c,
            r.property_without_basis,
            r.property_distributed,
            total,
        ]) {
            figures.insert(format!("{}.{cause}", row.book.name), figure(v));
        }
    }
    let named_members: BTreeSet<&str> = x
        .figures(&ex.id)
        .keys()
        .map(|k| k.split_once('.').map_or(k.as_str(), |(m, _)| m))
        .collect();
    for m in named_members {
        for cause in RECONCILING_CAUSES {
            assert!(
                x.figures(&ex.id).contains_key(&format!("{m}.{cause}")),
                "{what}: {m} is shown with every cause: {m}.{cause} is missing"
            );
        }
    }
    named(ex, x, &what, &figures);
}

/// The commands an example may try: each on a copy of the books, so it
/// changes nothing that is checked after it.
const COMMANDS: [&str; 3] = ["rename_account", "add_account", "set_tax_category"];

/// The account `code` names, for a command.
fn command_account(ex: &Example, l: &Ledger, x: &Expect) -> ledger_core::AccountId {
    let code = x
        .code
        .as_deref()
        .unwrap_or_else(|| panic!("{}: {} names its account (code)", ex.id, x.query));
    l.chart()
        .find_by_code(code)
        .unwrap_or_else(|| panic!("{}: no account {code}", ex.id))
        .id
}

/// Every balance, by code: what a change that moves no amount leaves as
/// it was.
fn every_balance(l: &Ledger) -> Vec<(String, Decimal, Decimal)> {
    trial_balance_as_of(l, ledger_core::calendar::LAST_DAY)
        .lines
        .into_iter()
        .map(|t| (t.code, t.debit, t.credit))
        .collect()
}

/// Try one command on a copy of the books, as the books' own command does
/// it (the engine's checks, then the policy's: rules.md B-31, B-32), and
/// check it is accepted or refused for the reason given.
fn check_command(ex: &Example, l: &Ledger, x: &Expect) {
    let mut copy = l.clone();
    let result: Result<(), LedgerError> = match x.query.as_str() {
        "rename_account" => {
            let account = command_account(ex, l, x);
            let name = x
                .name
                .as_deref()
                .unwrap_or_else(|| panic!("{}: rename_account gives the name", ex.id));
            copy.rename_account(account, name).inspect(|()| {
                // A rename changes the label, nothing else (rules.md B-31).
                assert_eq!(
                    copy.chart().get(account).unwrap().name,
                    name.trim(),
                    "{}: rename_account keeps the name trimmed",
                    ex.id
                );
                assert_eq!(
                    every_balance(&copy),
                    every_balance(l),
                    "{}: a rename moves no amount",
                    ex.id
                );
            })
        }
        "add_account" => {
            let a = x
                .account
                .as_ref()
                .unwrap_or_else(|| panic!("{}: add_account gives the account", ex.id));
            let kind = account_type(a, &ex.id);
            let parent = a.parent.as_deref().map(|p| {
                l.chart()
                    .find_by_code(p)
                    .unwrap_or_else(|| panic!("{}: no account {p}", ex.id))
                    .id
            });
            copy.add_account(&a.code, &a.name, kind, parent, a.header)
                .and_then(|id| match &a.tax_category {
                    Some(c) => copy.set_account_tax(id, Some(c), None),
                    None => Ok(()),
                })
                .and_then(|()| {
                    l.check_tax_category_required(
                        &a.code,
                        &a.name,
                        kind,
                        a.header,
                        a.tax_category.as_deref(),
                    )
                })
        }
        "set_tax_category" => {
            let account = command_account(ex, l, x);
            let category = x.category.as_deref();
            copy.set_account_tax(account, category, None)
                .and_then(|()| l.check_tax_category_kept(account, category))
        }
        q => unreachable!("{q} is no command"),
    };
    match (result, &x.refused) {
        (Ok(()), None) => {}
        (Err(e), Some(want)) => {
            assert_eq!(refusal(&e), *want, "{}: {} refused ({e})", ex.id, x.query)
        }
        (Err(e), None) => panic!("{}: {} refused ({e}), expected accepted", ex.id, x.query),
        (Ok(()), Some(want)) => panic!("{}: {} accepted, expected refused: {want}", ex.id, x.query),
    }
}

/// Check one `period` expectation: the period a period report asked with
/// these dates covers, by the one rule (rules.md B-30), as `from` and
/// `to`; or the rule's refusal.
fn check_period(ex: &Example, x: &Expect) {
    match (resolved_period(ex, x), &x.refused) {
        (Ok((from, to)), None) => {
            exactly(ex, x, &["from".to_string(), "to".to_string()].into());
            let f = x.figures(&ex.id);
            assert_eq!(from, date(&f["from"], &ex.id), "{}: period from", ex.id);
            assert_eq!(to, date(&f["to"], &ex.id), "{}: period to", ex.id);
        }
        (Err(e), Some(want)) => assert_eq!(refusal(&e), *want, "{}: period refused ({e})", ex.id),
        (Err(e), None) => panic!("{}: period refused ({e})", ex.id),
        (Ok((from, to)), Some(want)) => {
            panic!("{}: period {from} .. {to}, expected refused: {want}", ex.id)
        }
    }
}

/// Check one `day` expectation: the day a report as of a day is made as
/// of (rules.md B-30), as `as_of`.
fn check_day(ex: &Example, x: &Expect) {
    exactly(ex, x, &["as_of".to_string()].into());
    let want = date(&x.figures(&ex.id)["as_of"], &ex.id);
    assert_eq!(day(ex, x), want, "{}: day", ex.id);
}

/// Check one `calendar_year` expectation: the year a calendar-year count
/// (1099s, party totals) asked without one counts (rules.md B-30).
fn check_calendar_year(ex: &Example, x: &Expect) {
    assert!(
        x.from.is_none() && x.to.is_none() && x.as_of.is_none(),
        "{}: calendar_year takes no dates",
        ex.id
    );
    exactly(ex, x, &["year".to_string()].into());
    let want: i32 = x.figures(&ex.id)["year"]
        .parse()
        .unwrap_or_else(|e| panic!("{}: calendar_year year: {e}", ex.id));
    assert_eq!(
        defaults(ex, &x.query).calendar_year(),
        want,
        "{}: calendar_year",
        ex.id
    );
}

/// Check one `account_names` expectation: each account's name, by code,
/// as the chart holds it and every report prints it (the trial balance
/// and the profit and loss, over every date: past periods too, rules.md
/// B-31); every account listed.
fn check_account_names(ex: &Example, l: &Ledger, x: &Expect) {
    assert!(
        x.from.is_none() && x.to.is_none() && x.as_of.is_none(),
        "{}: account_names takes no dates",
        ex.id
    );
    let keys = l.chart().all().map(|a| a.code.clone()).collect();
    exactly(ex, x, &keys);
    let (first, last) = (
        ledger_core::calendar::FIRST_DAY,
        ledger_core::calendar::LAST_DAY,
    );
    let tb = trial_balance_as_of(l, last);
    let pl = profit_and_loss_for_period(l, first, last, None)
        .unwrap_or_else(|e| panic!("{}: {e}", ex.id));
    for (code, want) in x.figures(&ex.id) {
        let a = l.chart().find_by_code(code).unwrap();
        assert_eq!(&a.name, want, "{}: account {code}'s name", ex.id);
        let printed = tb.lines.iter().map(|t| (&t.code, &t.name)).chain(
            pl.revenue
                .iter()
                .chain(&pl.expenses)
                .map(|s| (&s.code, &s.name)),
        );
        for (c, name) in printed.filter(|(c, _)| *c == code) {
            assert_eq!(name, want, "{}: a report prints {c} as {name:?}", ex.id);
        }
    }
}

/// Check one `uncategorized_accounts` expectation: every account that
/// maps to a line of the return and has no tax category, with activity or
/// not, as `"<code> <name>"`, by code (rules.md B-32).
fn check_uncategorized_accounts(ex: &Example, l: &Ledger, x: &Expect) {
    assert!(
        x.from.is_none() && x.to.is_none() && x.as_of.is_none(),
        "{}: uncategorized_accounts takes no dates",
        ex.id
    );
    let Values::Lines(want) = x.values(&ex.id) else {
        panic!("{}: uncategorized_accounts expects a list", ex.id)
    };
    let got: Vec<String> = ledger_core::tax::uncategorized_accounts(l)
        .into_iter()
        .map(|a| format!("{} {}", a.code, a.name))
        .collect();
    assert_eq!(&got, want, "{}: uncategorized_accounts", ex.id);
}

/// Check one expectation against the books as they stand: a query the
/// core answers, value by value; a format query's shape, its values left
/// to the test [`FORMAT_QUERIES`] names (returned); a query no crate
/// answers is refused.
fn check(b: &Books<'_>, x: &Expect) -> Option<&'static str> {
    let (ex, l) = (b.ex, b.l());
    let q = x.query.as_str();
    let only = |given: bool, key: &str, queries: &[&str]| {
        assert!(
            !given || queries.contains(&q),
            "{}: {q}: {key} is for {} only",
            ex.id,
            queries.join(" and ")
        );
    };
    only(x.entry.is_some(), "entry", &["entry", "party_part"]);
    only(x.event.is_some(), "event", &["run"]);
    only(
        x.asset.is_some(),
        "asset",
        &[
            "disposal",
            "depreciation_schedule",
            "register_row",
            "asset_basis",
        ],
    );
    only(x.schedule.is_some(), "schedule", &["schedule_plan"]);
    only(
        x.party.is_some(),
        "party",
        &["party_part", "party_totals", "party_tin_last4"],
    );
    only(x.basis.is_some(), "basis", &["party_totals"]);
    only(x.payee.is_some(), "payee", &["vendor_1099"]);
    only(
        x.code.is_some(),
        "code",
        &["rename_account", "set_tax_category"],
    );
    only(x.name.is_some(), "name", &["rename_account"]);
    only(x.category.is_some(), "category", &["set_tax_category"]);
    only(x.account.is_some(), "account", &["add_account"]);
    only(x.accepted.is_some(), "accepted", &COMMANDS);
    only(
        x.refused.is_some(),
        "refused",
        &[
            "rename_account",
            "add_account",
            "set_tax_category",
            "period",
        ],
    );
    if COMMANDS.contains(&q) {
        // A command is accepted or refused, and shows no values.
        assert!(
            x.values.is_none() && (x.accepted == Some(true)) != x.refused.is_some(),
            "{}: {q} is a command: accepted = true, or refused, and no values",
            ex.id
        );
        check_command(ex, l, x);
        return None;
    }
    // Anything else shows its values; a period may be refused instead.
    assert!(
        x.refused.is_some() != x.values.as_ref().is_some_and(|v| !v.is_empty()),
        "{}: {q} expects values (a period: values or refused)",
        ex.id
    );
    match q {
        "period" => check_period(ex, x),
        "day" => check_day(ex, x),
        "calendar_year" => check_calendar_year(ex, x),
        "account_names" => check_account_names(ex, l, x),
        "uncategorized_accounts" => check_uncategorized_accounts(ex, l, x),
        "cash_flow" => check_cash_flow(b, x),
        "trial_balance" => check_trial_balance(ex, l, x),
        "profit_and_loss" => check_profit_and_loss(ex, l, x),
        "vendor_1099_paid" => check_vendor_1099_paid(b, x),
        "vendor_1099" => check_vendor_1099(b, x),
        "entry" => check_entry(b, x),
        "run" => check_run(b, x),
        "disposal" => check_disposal(b, x),
        "depreciation_schedule" => check_depreciation_schedule(b, x),
        "schedule_plan" => check_schedule_plan(b, x),
        "register_row" => check_register_row(b, x),
        "register_totals" => check_register_totals(b, x),
        "asset_basis" => check_asset_basis(b, x),
        "k1_tax_basis" => check_k1_tax_basis(b, x),
        "k1_reconciliation" => check_k1_reconciliation(b, x),
        "balances" => check_balances(ex, l, x),
        "members_equity" => check_members_equity(ex, l, &b.members, x),
        "k1" => check_k1(ex, l, &b.members, x),
        "party_part" => check_party_part(b, x),
        "party_totals" => check_party_totals(b, x),
        "party_tin_last4" => check_party_tin_last4(b, x),
        "project" => check_project(b, x),
        q => {
            let &(_, by) = FORMAT_QUERIES
                .iter()
                .find(|(f, _)| *f == q)
                .unwrap_or_else(|| panic!("{}: no crate answers query {q:?}", ex.id));
            check_format_shape(ex, x);
            return Some(by);
        }
    }
    None
}

/// Load an example and check every expectation: each one with `after = N`
/// on the books after event N (before the changes made after it), the
/// rest after the last event and every change. Gives each query checked,
/// with the test of another crate that checks its values, if any.
fn run(ex: &Example) -> Vec<(String, Option<&'static str>)> {
    let afters = ex
        .expect
        .iter()
        .map(|x| x.after)
        .chain(ex.changes.iter().map(|c| Some(c.after)));
    for after in afters {
        assert!(
            after.is_none_or(|n| n >= 1 && n <= ex.events.len()),
            "{}: after = {after:?}: events count from 1, to {}",
            ex.id,
            ex.events.len()
        );
    }
    let mut b = Books::new(ex);
    let mut out = Vec::new();
    for (i, e) in ex.events.iter().enumerate() {
        b.apply(i + 1, e);
        for x in ex.expect.iter().filter(|x| x.after == Some(i + 1)) {
            out.push((x.query.clone(), check(&b, x)));
        }
        for c in ex.changes.iter().filter(|c| c.after == i + 1) {
            b.change(c);
        }
    }
    for x in ex.expect.iter().filter(|x| x.after.is_none()) {
        out.push((x.query.clone(), check(&b, x)));
    }
    // What the books kept replays to the books (the journal, the live
    // books and their projection agree).
    b.store
        .verify()
        .unwrap_or_else(|e| panic!("{}: the books replay as kept: {e}", ex.id));
    out
}

/// The `Engine` adapter over `Store` runs every example through the
/// runner, and every rule the adapter claims passes (`plan/served.md` P13
/// step 2). The loader above proves the same figures with its own checks;
/// this proves the trait path, which is what another engine supplies.
#[test]
fn the_runner_reports_every_claimed_rule_green() {
    let mut files: Vec<PathBuf> = std::fs::read_dir(examples_dir())
        .expect("bookspec/examples")
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "toml"))
        .collect();
    files.sort();
    let mut examples = Vec::new();
    for f in &files {
        let text = std::fs::read_to_string(f).unwrap();
        examples.push(Example::from_toml(&text).unwrap_or_else(|e| panic!("{}: {e}", f.display())));
    }
    assert!(examples.len() >= 56, "the seed has at least 56 examples");
    let report = bookspec_runner::run(&examples, bookspec_checker::rules(), Books::blank);
    assert!(
        report.passed(),
        "the runner is not green:\n{}",
        report.failures().join("\n")
    );
    for rule in bookspec_checker::covered_rules() {
        assert_eq!(
            report.rules.get(&rule.id),
            Some(&bookspec_runner::Outcome::Pass),
            "{}",
            rule.id
        );
    }
    assert!(
        report
            .rules
            .values()
            .any(|o| *o == bookspec_runner::Outcome::NotClaimed),
        "a rule with no example is not claimed"
    );
}

#[test]
fn every_bookspec_example_holds() {
    let mut files: Vec<PathBuf> = std::fs::read_dir(examples_dir())
        .expect("bookspec/examples")
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "toml"))
        .collect();
    files.sort();
    let mut ids = BTreeSet::new();
    let mut checked: BTreeMap<String, usize> = BTreeMap::new();
    for f in &files {
        let text = std::fs::read_to_string(f).unwrap();
        let ex = Example::from_toml(&text).unwrap_or_else(|e| panic!("{}: {e}", f.display()));
        let stem = f.file_stem().unwrap().to_string_lossy();
        assert_eq!(ex.id, stem, "{}: the file is named for its id", f.display());
        assert!(ids.insert(ex.id.clone()), "{}: id used twice", ex.id);
        assert!(
            !ex.title.is_empty() && !ex.rule.is_empty() && !ex.tags.is_empty(),
            "{}: title, rule and tags",
            ex.id
        );
        assert!(
            matches!(ex.status.as_str(), "AUTHORITY" | "CONVENTION" | "CHOICE"),
            "{}: status {:?}",
            ex.id,
            ex.status
        );
        assert!(!ex.sources.is_empty(), "{}: every rule is sourced", ex.id);
        assert!(!ex.expect.is_empty(), "{}: something to expect", ex.id);
        for (q, by) in run(&ex) {
            if let Some(by) = by {
                eprintln!("{}: {q} is checked by {by}", ex.id);
            }
            *checked.entry(q).or_insert(0) += 1;
        }
    }
    // Each query the examples use ran: the cash flow's seven examples at
    // least, the as-of and 1099 examples', the members closes', the
    // formats appendix's, and P3's default period, rename, tax category,
    // parties, 1099, depreciation, schedules, asset basis and K-1 tax-basis
    // examples'.
    for (q, at_least) in [
        ("cash_flow", 7),
        ("trial_balance", 2),
        ("profit_and_loss", 2),
        ("vendor_1099_paid", 4),
        ("vendor_1099", 10),
        ("entry", 8),
        ("balances", 4),
        ("members_equity", 10),
        ("k1", 3),
        ("party_part", 28),
        ("party_totals", 23),
        ("party_tin_last4", 5),
        ("iif_lines", 1),
        ("iif_changes", 1),
        ("period", 6),
        ("day", 1),
        ("calendar_year", 1),
        ("account_names", 2),
        ("uncategorized_accounts", 2),
        ("rename_account", 5),
        ("add_account", 4),
        ("set_tax_category", 2),
        ("run", 4),
        ("disposal", 5),
        ("depreciation_schedule", 5),
        ("schedule_plan", 3),
        ("register_row", 5),
        ("register_totals", 1),
        ("asset_basis", 6),
        ("k1_tax_basis", 4),
        ("k1_reconciliation", 2),
        ("project", 6),
    ] {
        let n = checked.get(q).copied().unwrap_or(0);
        assert!(n >= at_least, "{q} ran {n} times, fewer than {at_least}");
    }
}

/// An inline example: two contributions, then `reverses` on the third
/// event.
fn reversing(reverses: usize) -> Example {
    toml::from_str(&format!(
        r#"
id = "loader.reverses"
title = "reverses counts events from 1"
rule = "An event's reverses names the event it reverses, counting from 1."
status = "CONVENTION"
sources = ["bookspec/README.md"]
tags = ["loader"]

[setup]
currency = "USD"
fiscal_year_start = "01-01"
basis = "accrual"
rounding = "half-even"

[[setup.accounts]]
code = "1000"
name = "Cash"
type = "asset"

[[setup.accounts]]
code = "3000"
name = "Capital"
type = "equity"

[[events]]
date = "2026-01-02"
description = "First"
lines = [
  {{ account = "1000", side = "debit", amount = "100.00" }},
  {{ account = "3000", side = "credit", amount = "100.00" }},
]

[[events]]
date = "2026-01-03"
description = "Second"
lines = [
  {{ account = "1000", side = "debit", amount = "250.00" }},
  {{ account = "3000", side = "credit", amount = "250.00" }},
]

[[events]]
date = "2026-01-04"
description = "Reverse the second"
reverses = {reverses}

[[expect]]
query = "none"
[expect.values]
none = "0"
"#
    ))
    .unwrap()
}

/// bookspec/README.md: `reverses = N` is the Nth event (entry #N),
/// counting from 1. `reverses = 2` reverses the second event, 250.00, not
/// the first (as counting from 0 would read it) nor itself.
#[test]
fn reverses_counts_events_from_one() {
    let ex = reversing(2);
    let b = load(&ex);
    let r = b.l().get_entry(3).unwrap();
    assert_eq!(r.reversal_of(), Some(2));
    let amounts: Vec<Decimal> = r.lines().iter().map(|x| x.amount.amount).collect();
    assert_eq!(amounts, [Decimal::new(25000, 2), Decimal::new(25000, 2)]);
    assert_eq!(r.lines()[0].side, Side::Credit);
}

/// `reverses = 0` names no event: refused, saying events count from 1.
#[test]
#[should_panic(expected = "events count from 1")]
fn reverses_zero_is_refused() {
    load(&reversing(0));
}

/// A key the shape doesn't have (a typo for a setting) is refused, never
/// ignored: an example can't silently run with other roles than it says.
#[test]
fn an_unknown_key_is_refused() {
    let text = std::fs::read_to_string(
        examples_dir().join("cash-flow.reversed-property-contribution.toml"),
    )
    .unwrap()
    .replace("cash_accounts = ", "cash_account = ");
    assert!(toml::from_str::<Example>(&text).is_err());
}

/// The example `file`'s text, changed by `edit`, loaded and its
/// expectations checked as `every_bookspec_example_holds` checks them.
fn run_edited(file: &str, edit: impl Fn(String) -> String) {
    let text = edit(std::fs::read_to_string(examples_dir().join(file)).unwrap());
    let before = std::fs::read_to_string(examples_dir().join(file)).unwrap();
    assert_ne!(text, before, "the edit changed nothing");
    let ex: Example = toml::from_str(&text).unwrap();
    run(&ex);
}

/// A query no crate answers is refused, never skipped: its values would
/// otherwise go unchecked.
#[test]
#[should_panic(expected = "no crate answers query")]
fn an_unknown_query_is_refused() {
    run_edited("as-of-package-stops-at-as-of.toml", |t| {
        t.replacen("query = \"trial_balance\"", "query = \"trial_balances\"", 1)
    });
}

/// A format's query belongs to the formats appendix: an example without
/// `appendix = "formats"` can't ask it.
#[test]
#[should_panic(expected = "appendix \"formats\"")]
fn a_format_query_outside_the_formats_appendix_is_refused() {
    run_edited("iif-quickbooks-journal-file.toml", |t| {
        t.replacen("appendix = \"formats\"\n", "", 1)
    });
}

/// The appendix is "formats" or absent.
#[test]
#[should_panic(expected = "appendix Some(\"format\")")]
fn an_unknown_appendix_is_refused() {
    run_edited("iif-quickbooks-journal-file.toml", |t| {
        t.replacen("appendix = \"formats\"", "appendix = \"format\"", 1)
    });
}

/// An event's payee must be one of the setup's payees.
#[test]
#[should_panic(expected = "not in setup")]
fn a_payee_not_in_the_setup_is_refused() {
    run_edited("1099-worksheet-through-as-of.toml", |t| {
        t.replacen(
            "payee = \"Contractor A\"\nlines",
            "payee = \"Contractor B\"\nlines",
            1,
        )
    });
}

/// A figure query lists everything it covers: a trial balance leaving an
/// account out is refused, so no balance goes unchecked.
#[test]
#[should_panic(expected = "lists exactly what it covers")]
fn a_trial_balance_leaving_an_account_out_is_refused() {
    run_edited("as-of-package-stops-at-as-of.toml", |t| {
        t.replacen(", \"6000\" = \"1200.00\" }", " }", 1)
    });
}

/// A trial balance is as of a day, a profit and loss over a period: the
/// wrong dates are refused, never read some other way.
#[test]
#[should_panic(expected = "is as of a day")]
fn a_trial_balance_over_a_period_is_refused() {
    run_edited("as-of-package-stops-at-as-of.toml", |t| {
        t.replacen(
            "query = \"trial_balance\"\nas_of = \"2026-06-30\"",
            "query = \"trial_balance\"\nfrom = \"2026-01-01\"\nto = \"2026-06-30\"",
            1,
        )
    });
}

/// An `entry` query names every line of the entry: one left out is
/// refused, so the entry is exactly what is expected.
#[test]
#[should_panic(expected = "lists exactly what it covers")]
fn an_entry_leaving_a_line_out_is_refused() {
    run_edited("members-close.true-up-of-a-per-close-split.toml", |t| {
        t.replacen("\"3000.debit\" = \"0.01\"\n", "", 1)
    });
}

/// `after = N` checks the books after event N: the statement to April 30
/// asked before April's close (A 0.03, B 0.01) fails when asked after it
/// (A 0.02, B 0.02).
#[test]
#[should_panic(expected = "members' equity [2026-01-01 .. 2026-04-30]: A.ending")]
fn after_is_the_books_after_that_event() {
    run_edited("members-close.true-up-of-a-per-close-split.toml", |t| {
        t.replacen("after = 7\n", "", 1)
    });
}

/// `after` counts events from 1 up to the last.
#[test]
#[should_panic(expected = "events count from 1")]
fn after_past_the_last_event_is_refused() {
    run_edited("members-close.true-up-of-a-per-close-split.toml", |t| {
        t.replacen("after = 6\n", "after = 9\n", 1)
    });
}

/// A close to the members needs every member's share, and the shares sum
/// to 1.
#[test]
#[should_panic(expected = "the shares sum to 1")]
fn shares_that_dont_sum_to_one_are_refused() {
    run_edited("members-close.true-up-of-a-per-close-split.toml", |t| {
        t.replacen("share = \"0.5\"", "share = \"0.4\"", 1)
    });
}

/// The rounding an example declares is the engine's for what it asks: a
/// members close splits by largest remainder.
#[test]
#[should_panic(expected = "rounds by largest-remainder")]
fn a_members_close_declared_with_another_rounding_is_refused() {
    run_edited("members-close.true-up-of-a-per-close-split.toml", |t| {
        t.replacen(
            "rounding = \"largest-remainder\"",
            "rounding = \"half-even\"",
            1,
        )
    });
}

/// A period's dates are the rule's (rules.md B-30): an example saying the
/// fiscal year to date starts on January 1 under a July fiscal year fails.
#[test]
#[should_panic(expected = "period from")]
fn a_period_other_than_the_rules_is_refused() {
    run_edited("default-period-fiscal-year-to-date.toml", |t| {
        t.replacen(
            "values = { from = \"2026-07-01\", to = \"2026-10-20\" }",
            "values = { from = \"2026-01-01\", to = \"2026-10-20\" }",
            1,
        )
    });
}

/// A refusal is checked for its reason: a start after today is not "the
/// end before the start".
#[test]
#[should_panic(expected = "period refused")]
fn a_refusal_for_another_reason_is_refused() {
    run_edited("default-period-fiscal-year-to-date.toml", |t| {
        t.replacen(
            "refused = \"start_after_today\"",
            "refused = \"end_before_start\"",
            1,
        )
    });
}

/// A query that omits a date needs `setup.today`: without it, nothing
/// says what the omitted date means.
#[test]
#[should_panic(expected = "setup.today says what it means")]
fn an_omitted_date_without_today_is_refused() {
    run_edited("default-period-fiscal-year-to-date.toml", |t| {
        t.replacen("today = \"2026-10-20\"\n", "", 1)
    });
}

/// `account_names` lists every account: one left out is refused.
#[test]
#[should_panic(expected = "lists exactly what it covers")]
fn account_names_leaving_an_account_out_is_refused() {
    run_edited("rename-changes-no-amount.toml", |t| {
        t.replacen(", \"6160\" = \"Hosting\" }", " }", 1)
    });
}

/// A change is made after the event it names, once that event's
/// expectations are checked: asked after the rename, the old name fails.
#[test]
#[should_panic(expected = "account 6150's name")]
fn a_change_is_made_after_its_events_expectations() {
    run_edited("rename-changes-no-amount.toml", |t| {
        t.replacen(
            "\"6150\" = \"Software subscriptions\", \"6160\" = \"Hosting\" }",
            "\"6150\" = \"Software\", \"6160\" = \"Hosting\" }",
            1,
        )
    });
}

/// A command is tried under the policy in force when it is asked: without
/// the change to refuse, the account without a category is accepted.
#[test]
#[should_panic(expected = "accepted, expected refused: tax_category_required")]
fn a_command_runs_under_the_policy_in_force() {
    run_edited("tax-category-required.toml", |t| {
        t.replacen(
            "[[changes]]\nafter = 1\nrequire_tax_category = \"refuse\"\n",
            "",
            1,
        )
    });
}

/// A command is accepted or refused, never both, and shows no values.
#[test]
#[should_panic(expected = "is a command")]
fn a_command_with_values_is_refused() {
    run_edited("tax-category-required.toml", |t| {
        t.replacen(
            "category = \"other_deductions\"\naccepted = true\n",
            "category = \"other_deductions\"\naccepted = true\nvalues = { x = \"1\" }\n",
            1,
        )
    });
}

/// An event's parties must be in the setup's directory.
#[test]
#[should_panic(expected = "not in setup")]
fn a_party_not_in_the_setup_is_refused() {
    run_edited("p3-party-signs-and-reversals.toml", |t| {
        t.replacen(
            "parties = [{ party = \"Tobin Harlow\" }]",
            "parties = [{ party = \"Tobin Harlo\" }]",
            1,
        )
    });
}

/// A party's part is checked for its direction as well as its figures:
/// a contribution read as paid fails.
#[test]
#[should_panic(expected = "entry 2 Some(\"Tobin Harlow\"): direction")]
fn a_party_parts_direction_is_checked() {
    run_edited("p3-party-signs-and-reversals.toml", |t| {
        t.replacen(
            "resolved = \"-400.00\", direction = \"received\"",
            "resolved = \"-400.00\", direction = \"paid\"",
            1,
        )
    });
}

/// A reversal takes its original's parties: naming them again is
/// refused, never read as a second tag.
#[test]
#[should_panic(expected = "takes its original's parties")]
fn a_reversal_naming_parties_is_refused() {
    run_edited("p3-party-signs-and-reversals.toml", |t| {
        t.replacen(
            "reverses = 4\n",
            "reverses = 4\nparties = [{ party = \"Tobin Harlow\" }]\n",
            1,
        )
    });
}

/// Party totals name their basis: cash and all differ for property.
#[test]
#[should_panic(expected = "party_totals names its basis")]
fn party_totals_without_a_basis_are_refused() {
    run_edited("p3-party-signs-and-reversals.toml", |t| {
        t.replacen("basis = \"all\"\n", "", 1)
    });
}

/// A `vendor_1099` line lists every figure it covers: one left out is
/// refused.
#[test]
#[should_panic(expected = "lists exactly what it covers")]
fn a_vendor_1099_line_leaving_a_figure_out_is_refused() {
    run_edited("1099-card-payments-are-excluded.toml", |t| {
        t.replacen(", not_reportable = \"0.00\" }", " }", 1)
    });
}

/// A `vendor_1099` line's status is checked: card payments that would
/// reach the threshold leave the payee under it.
#[test]
#[should_panic(expected = "Harbor Design LLC: status")]
fn a_vendor_1099_status_is_checked() {
    run_edited("1099-card-payments-are-excluded.toml", |t| {
        t.replacen("status = \"under_threshold\"", "status = \"reportable\"", 1)
    });
}

/// A payment method an event names must be in the setup.
#[test]
#[should_panic(expected = "payment method \"Harbor debit\" not in setup")]
fn a_payment_method_not_in_the_setup_is_refused() {
    run_edited("1099-card-payments-are-excluded.toml", |t| {
        t.replacen(
            "instrument = \"Harbor debit card\"",
            "instrument = \"Harbor debit\"",
            1,
        )
    });
}

/// A value from a closed list is spelled exactly as bookspec/README.md
/// spells it: the engine's own parse forgives case and spaces, the
/// loader doesn't (a spec has one spelling).
#[test]
#[should_panic(expected = "form \"1099-nec\"")]
fn a_form_in_another_case_is_refused() {
    run_edited("1099-a-dated-status-applies-by-payment-date.toml", |t| {
        t.replacen("form = \"1099-NEC\"", "form = \"1099-nec\"", 1)
    });
}

#[test]
#[should_panic(expected = "kind \"Business_Card\"")]
fn a_payment_method_kind_in_another_case_is_refused() {
    run_edited("1099-card-payments-are-excluded.toml", |t| {
        t.replacen("kind = \"business_card\"", "kind = \"Business_Card\"", 1)
    });
}

#[test]
#[should_panic(expected = "not_reportable needs a reason_kind")]
fn a_reason_kind_in_another_case_is_refused() {
    run_edited("1099-a-dated-status-applies-by-payment-date.toml", |t| {
        t.replacen(
            "reason_kind = \"corporation\"",
            "reason_kind = \"Corporation\"",
            1,
        )
    });
}

/// An event the example says the engine refuses must be refused: one it
/// takes fails the example (March's run reversed on its own day is taken).
#[test]
#[should_panic(expected = "accepted, expected refused: entry_irreversible")]
fn an_event_the_engine_takes_is_not_refused() {
    run_edited(
        "depreciation-reversal-before-its-run-is-refused.toml",
        |t| t.replacen("date = \"2026-02-10\"", "date = \"2026-03-31\"", 1),
    );
}

/// A refused event is refused for the reason it gives, in the examples'
/// words, never another.
#[test]
#[should_panic(expected = "reverses refused")]
fn an_event_refused_for_another_reason_is_refused() {
    run_edited(
        "depreciation-reversal-before-its-run-is-refused.toml",
        |t| {
            t.replacen(
                "refused = \"entry_irreversible\"",
                "refused = \"depreciation_past_disposal\"",
                1,
            )
        },
    );
}

/// A refused event posts nothing, so nothing can name its entry.
#[test]
#[should_panic(expected = "event 6 posted 0 entries")]
fn a_refused_event_has_no_entry_to_reverse() {
    run_edited(
        "depreciation-reversal-before-its-run-is-refused.toml",
        |t| {
            t.replacen(
                "reverses = 5\n\n# Hand check",
                "reverses = 6\n\n# Hand check",
                1,
            )
        },
    );
}

/// A command dates its own entries: a run given a date is refused, never
/// read as the day it runs.
#[test]
#[should_panic(expected = "dates its own entries: no date")]
fn a_run_given_a_date_is_refused() {
    run_edited("depreciation-catch-up-month.toml", |t| {
        t.replacen(
            "[[events]]\nrun_depreciation",
            "[[events]]\ndate = \"2026-01-31\"\nrun_depreciation",
            1,
        )
    });
}

/// `run` asks about a run_depreciation event, nothing else.
#[test]
#[should_panic(expected = "event 2 is no run_depreciation")]
fn a_run_query_naming_another_event_is_refused() {
    run_edited("depreciation-catch-up-month.toml", |t| {
        t.replacen("event = 4", "event = 2", 1)
    });
}

/// A disposal lists every figure it took: one left out is refused.
#[test]
#[should_panic(expected = "lists exactly what it covers")]
fn a_disposal_leaving_a_figure_out_is_refused() {
    run_edited("depreciation-disposal-mid-month.toml", |t| {
        t.replacen("catch_up_depreciation = \"0.00\"\n", "", 1)
    });
}

/// The example's figures are the engine's, checked: a schedule month off
/// by a cent fails it.
#[test]
#[should_panic(expected = "schedule_plan of SUB: 2026-02-28")]
fn a_schedule_month_off_by_a_cent_fails() {
    run_edited("schedule-daily-deferred-revenue.toml", |t| {
        t.replacen(
            "\"2026-02-28\" = \"333.34\"",
            "\"2026-02-28\" = \"333.33\"",
            1,
        )
    });
}

/// An asset is on the register from its acquired date: a row asked for
/// before it is refused, never shown with figures.
#[test]
#[should_panic(expected = "not on the register (acquired later)")]
fn a_register_row_before_the_asset_was_acquired_is_refused() {
    run_edited("assets-acquired-before-in-service.toml", |t| {
        t.replacen("as_of = \"2026-03-31\"", "as_of = \"2026-03-09\"", 1)
    });
}

/// `acquired_by` names an event whose entry put the asset on the books:
/// an event that posts no entry can't.
#[test]
#[should_panic(expected = "event 2 posted 0 entries")]
fn an_asset_acquired_by_an_event_with_no_entry_is_refused() {
    run_edited("assets-a-reversed-run-is-not-posted.toml", |t| {
        t.replacen("acquired_by = 1", "acquired_by = 2", 1)
            .replacen(
                "[[events]]\nrun_depreciation = { asset = \"printer\", period = \"2026-01\" }",
                "[[events]]\nrun_depreciation = { asset = \"printer\", period = \"2025-12\" }\nrefused = \"invalid_depreciation_period\"",
                1,
            )
    });
}

/// A tax line the books can't know is "open", never a figure: an example
/// giving the book figure for it fails.
#[test]
#[should_panic(expected = "k1_tax_basis: Tobin Harlow.net_income")]
fn a_tax_line_the_books_cant_know_is_open() {
    run_edited("partnership-k1-tax-basis-capital.toml", |t| {
        t.replacen(
            "\"Tobin Harlow.net_income\" = \"open\"",
            "\"Tobin Harlow.net_income\" = \"-385.00\"",
            1,
        )
    });
}

/// A member's reconciliation is shown whole: an example leaving out a
/// cause fails, so a figure can't quietly go unchecked.
#[test]
#[should_panic(expected = "Tobin Harlow.property_distributed is missing")]
fn a_members_reconciliation_shows_every_cause() {
    run_edited("partnership-k1-tax-basis-capital.toml", |t| {
        t.replacen(", \"Tobin Harlow.property_distributed\" = \"0.00\"", "", 1)
    });
}

/// A project named twice in `setup.projects` is refused (names are
/// unique, rules.md B-37).
#[test]
#[should_panic(expected = "a project named")]
fn a_project_named_twice_is_refused() {
    run_edited("projects-net-is-its-tagged-lines.toml", |t| {
        t.replacen("name = \"Beta\"", "name = \"alpha\"", 1)
    });
}

/// A line naming a project the setup doesn't have is refused.
#[test]
#[should_panic(expected = "not in setup.projects")]
fn a_line_naming_an_unknown_project_is_refused() {
    run_edited("projects-net-is-its-tagged-lines.toml", |t| {
        t.replacen("project = \"Beta\" }", "project = \"Gamma\" }", 1)
    });
}

/// A project query lists every value it covers.
#[test]
#[should_panic(expected = "lists exactly what it covers")]
fn a_project_query_leaving_a_value_out_is_refused() {
    run_edited("projects-on-spec-costs-are-expenses.toml", |t| {
        t.replacen("unrecovered_cost = \"1460.00\"\n", "", 1)
    });
}

/// An invoice's line naming a project the setup doesn't have is refused.
#[test]
#[should_panic(expected = "project \"Gamma\" not in setup.projects")]
fn a_document_line_naming_an_unknown_project_is_refused() {
    run_edited(
        "projects-invoice-and-bill-lines-count-by-project.toml",
        |t| t.replacen("project = \"Harbor\" }", "project = \"Gamma\" }", 1),
    );
}

/// A document has an amount or lines, exactly one.
#[test]
#[should_panic(expected = "a document has an amount or lines, exactly one")]
fn a_document_with_an_amount_and_lines_is_refused() {
    run_edited(
        "projects-invoice-and-bill-lines-count-by-project.toml",
        |t| {
            t.replacen(
                "invoice = { customer = \"Cedar Client\", lines",
                "invoice = { customer = \"Cedar Client\", amount = \"1.00\", lines",
                1,
            )
        },
    );
}

/// An invoice's projects count: the example fails when a line's project
/// moves (Zeta's figures are no longer the ones worked by hand).
#[test]
#[should_panic(expected = "projects-invoice-and-bill-lines-count-by-project project \"Zeta\"")]
fn an_invoice_lines_project_is_what_the_project_counts() {
    run_edited(
        "projects-invoice-and-bill-lines-count-by-project.toml",
        |t| {
            t.replacen(
                "{ description = \"Revisions\", amount = \"500.00\", project = \"Zeta\" }",
                "{ description = \"Revisions\", amount = \"500.00\", project = \"Harbor\" }",
                1,
            )
        },
    );
}
