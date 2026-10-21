//! The bookspec examples (`bookspec/examples/*.toml`, P13, D-34), checked
//! against the engine on every run, so they can't rot. This is the one
//! place every example is loaded.
//!
//! Each example is a setup, events and expected values worked by hand from
//! its cited source (`bookspec/README.md` gives the shape). Every example
//! must load, whatever it asks or whichever appendix it is in: its shape,
//! its accounts, fixed assets, parties, members, payees, payees' dated
//! 1099 decisions and payment methods, every event (an entry, a reversal,
//! a close or a bill, posting entry #N as event N, with the parties,
//! payee and payment method it is tagged with) and every change that
//! posts no entry (a rename, a policy). The queries the core answers
//! (`cash_flow`, `trial_balance`, `profit_and_loss`, `vendor_1099_paid`,
//! `vendor_1099`, `entry`, `balances`, `members_equity`, `k1`,
//! `party_part`, `party_totals`, `party_tin_last4`, `period`, `day`,
//! `calendar_year`, `account_names`, `uncategorized_accounts`) are then
//! checked value by value, and the commands it tries
//! (`rename_account`, `add_account`, `set_tax_category`) accepted or
//! refused on a copy of the books, each after the event it names (`after`)
//! or the last; the formats appendix's (`iif_lines`, `iif_changes`) are
//! checked by ledger-package (`the_bookspec_iif_example_matches_the_engine`),
//! and a query no crate answers is refused. A key the shape doesn't have
//! is refused too, never ignored.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use chrono::NaiveDate;
use ledger_core::calendar::{PeriodDefaults, period_defaults};
use ledger_core::parties::{Basis, MoneyAccounts};
use ledger_core::settings::{Company, Source};
use ledger_core::{
    AccountType, CashFlowParams, CashFlowSection, CashFlowStatement, CloseTo, Currency, Dated1099,
    Decision1099, DocumentLine, EntryId, ExemptReason, Form1099, InstrumentId, InstrumentKind,
    Ledger, LedgerError, MemberCapital, MemberClose, Money, NewAsset, PartyId, PartyMember,
    PartyShare, PaymentAccounts, ReconcilingKind, Role, Share, Side, TinFacts, TinType, VendorId,
    W9Facts, cash_flow, members_equity, profit_and_loss_for_period, trial_balance_as_of,
    vendor_1099_report,
};
use rust_decimal::Decimal;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Example {
    id: String,
    title: String,
    rule: String,
    status: String,
    sources: Vec<String>,
    tags: Vec<String>,
    /// `"formats"`: an interchange format rather than an accounting rule
    /// (P13's appendix of formats); absent for the rules themselves.
    #[serde(default)]
    appendix: Option<String>,
    setup: Setup,
    events: Vec<Event>,
    /// Changes that post no entry, each made after the event it names.
    #[serde(default)]
    changes: Vec<ChangeSpec>,
    expect: Vec<Expect>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Setup {
    currency: String,
    fiscal_year_start: String,
    basis: String,
    rounding: String,
    accounts: Vec<AccountSpec>,
    #[serde(default)]
    cash_accounts: Vec<String>,
    #[serde(default)]
    financing_accounts: Vec<String>,
    #[serde(default)]
    investing_accounts: Vec<String>,
    #[serde(default)]
    fixed_assets: Vec<FixedAsset>,
    /// The company's directory: parties other than a payee's vendor
    /// record, and the payees' when the example names their roles.
    #[serde(default)]
    parties: Vec<PartySpec>,
    #[serde(default)]
    members: Vec<MemberSpec>,
    #[serde(default)]
    payees: Vec<PayeeSpec>,
    /// A payee's 1099 decision from a date on (rules.md B-19, D-40).
    #[serde(default)]
    status_changes: Vec<StatusChangeSpec>,
    /// The ways the company (or a member, for it) pays (rules.md B-19).
    #[serde(default)]
    instruments: Vec<InstrumentSpec>,
    /// The day the books are asked on, in the company's time zone: what
    /// an omitted date means (rules.md B-30). A query that omits a date
    /// needs it.
    #[serde(default)]
    today: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AccountSpec {
    code: String,
    name: String,
    #[serde(rename = "type")]
    kind: String,
    /// The parent account's code, declared before it.
    #[serde(default)]
    parent: Option<String>,
    /// A header account: it groups others and takes no postings.
    #[serde(default)]
    header: bool,
    /// Its tax category, a key of the catalog (rules.md B-32).
    #[serde(default)]
    tax_category: Option<String>,
}

impl AccountSpec {
    fn account_type(&self, id: &str) -> AccountType {
        match self.kind.as_str() {
            "asset" => AccountType::Asset,
            "liability" => AccountType::Liability,
            "equity" => AccountType::Equity,
            "revenue" => AccountType::Revenue,
            "expense" => AccountType::Expense,
            other => panic!("{id}: account type {other:?}"),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FixedAsset {
    cost_account: String,
    accumulated_account: String,
    expense_account: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MemberSpec {
    /// The member's name: a party's in `setup.parties` (with the member
    /// role) when the example asks about the member as a party.
    name: String,
    capital_account: String,
    /// Their share of income, an exact decimal fraction; every member has
    /// one or none does, and the shares sum to 1. A close to the members
    /// needs them.
    #[serde(default)]
    share: Option<String>,
    /// Their distributions account (rules.md B-13), when they have one.
    #[serde(default)]
    distributions_account: Option<String>,
}

/// A payee marked for an information return from the beginning (rules.md
/// B-19): a vendor record, the party's of that name in `setup.parties`
/// when there is one with the vendor role.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PayeeSpec {
    name: String,
    form: String,
}

/// One party in the directory (rules.md B-26, B-35).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PartySpec {
    name: String,
    /// Role keys (`member`, `vendor`, `customer`, `contractor`,
    /// `financial_institution`, ...); `vendor` and `customer` give the
    /// party its record (one of them at most).
    roles: Vec<String>,
    /// The liabilities, by code, the company owes the party itself
    /// (rules.md B-35 item 6).
    #[serde(default)]
    creditor_of: Vec<String>,
    /// Its TIN facts' last four digits (an SSN's).
    #[serde(default)]
    tin_last4: Option<String>,
    /// Its W-9 facts' last four digits (a 1099 role's).
    #[serde(default)]
    w9_tin_last4: Option<String>,
    /// The vault's copy of the TIN purged, the facts kept (needs
    /// `tin_last4`).
    #[serde(default)]
    tin_purged: bool,
}

/// A payee's 1099 decision from `effective` on (rules.md B-19, D-40).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StatusChangeSpec {
    payee: String,
    effective: String,
    /// `"1099-NEC"`, `"1099-MISC"`, `"not_reportable"` (with `reason_kind`
    /// and `reason`) or `"undetermined"`.
    decision: String,
    #[serde(default)]
    reason_kind: Option<String>,
    #[serde(default)]
    reason: Option<String>,
}

/// A payment method (rules.md B-19).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InstrumentSpec {
    label: String,
    /// `bank`, `business_card`, `member_card`, `member_cash_or_check` or
    /// `platform`.
    kind: String,
    /// The account it pays through, by code.
    account: String,
    #[serde(default)]
    last4: Option<String>,
    /// The party it belongs to, by name (informational).
    #[serde(default)]
    owner: Option<String>,
}

/// A party named on an entry, with its share: an `amount`, a `percent`,
/// or neither (derived from the entry: rules.md B-35).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ShareSpec {
    party: String,
    #[serde(default)]
    amount: Option<String>,
    #[serde(default)]
    percent: Option<String>,
}

/// One event: an entry (`lines`), a reversal (`reverses`), a close
/// (`close`) or a bill (`bill`), exactly one of them.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Event {
    date: String,
    description: String,
    #[serde(default)]
    lines: Vec<LineSpec>,
    reverses: Option<usize>,
    #[serde(default)]
    close: Option<CloseSpec>,
    #[serde(default)]
    bill: Option<BillSpec>,
    /// The payee (in `setup.payees`) the entry paid.
    #[serde(default)]
    payee: Option<String>,
    /// The parties (in `setup.parties`) the entry is with, and their
    /// shares.
    #[serde(default)]
    parties: Vec<ShareSpec>,
    /// The payment method (an `setup.instruments` label) it was paid with.
    #[serde(default)]
    instrument: Option<String>,
}

/// A bill from `vendor` for `amount`, dated and due on the event's date:
/// Dr General Expenses (6999), Cr Accounts Payable (2000), each created
/// with its fixed name when first needed, as the engine creates a control
/// role's account (rules.md B-32).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BillSpec {
    vendor: String,
    amount: String,
}

/// A change to the books that posts no entry, made right after event
/// `after` (counting from 1) once the expectations asked after that event
/// are checked: a rename (`rename`) or the tax-category policy
/// (`require_tax_category`), exactly one of them.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChangeSpec {
    after: usize,
    #[serde(default)]
    rename: Option<RenameSpec>,
    /// `policy.require_tax_category`: "warn" (the books' default) or
    /// "refuse" (rules.md B-32).
    #[serde(default)]
    require_tax_category: Option<String>,
}

/// Rename the account `code` to `name` (rules.md B-31).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RenameSpec {
    code: String,
    name: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LineSpec {
    account: String,
    side: String,
    amount: String,
}

/// A close of the period from `start` through the event's date, to
/// `"members"` (by share, in the order listed) or `"retained_earnings"`;
/// `rounding = "per-close"`: a members close split on its own (books from
/// an engine that rounded each close by itself), else the cumulative rule
/// (rules.md B-13).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CloseSpec {
    start: String,
    to: String,
    #[serde(default)]
    rounding: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Expect {
    query: String,
    from: Option<String>,
    to: Option<String>,
    as_of: Option<String>,
    /// The entry an `entry` or `party_part` query asks about.
    entry: Option<EntryId>,
    /// The party a `party_part`, `party_totals` or `party_tin_last4`
    /// query asks about (in `setup.parties`).
    party: Option<String>,
    /// `party_totals`' basis: `"cash"` or `"all"` (rules.md B-35 item 4).
    basis: Option<String>,
    /// The payee a `vendor_1099` query asks about (in `setup.payees`).
    payee: Option<String>,
    /// The books after event N, not after the last.
    after: Option<usize>,
    /// The account a `rename_account` or `set_tax_category` command
    /// names, by code.
    code: Option<String>,
    /// The new name a `rename_account` command gives.
    name: Option<String>,
    /// The category a `set_tax_category` command sets (absent: it clears
    /// the account's).
    category: Option<String>,
    /// The account an `add_account` command adds.
    account: Option<AccountSpec>,
    /// A command the engine accepts: `true` (never `false`: a command is
    /// accepted or `refused`).
    accepted: Option<bool>,
    /// A command, or a `period`, the engine refuses: why, in the words
    /// of [`refusal`].
    refused: Option<String>,
    #[serde(default)]
    values: Option<Values>,
}

/// What a query expects: figures by key (every figure an exact decimal
/// string), or, for a file format, its lines or its rows of fields.
#[derive(Deserialize)]
#[serde(untagged)]
enum Values {
    Figures(BTreeMap<String, String>),
    Lines(Vec<String>),
    Rows(Vec<Vec<String>>),
}

impl Values {
    fn is_empty(&self) -> bool {
        match self {
            Values::Figures(f) => f.is_empty(),
            Values::Lines(l) => l.is_empty(),
            Values::Rows(r) => r.is_empty(),
        }
    }
}

impl Expect {
    /// The values a query expects; panics when it gives none.
    fn values(&self, id: &str) -> &Values {
        self.values
            .as_ref()
            .unwrap_or_else(|| panic!("{id}: {} expects values", self.query))
    }

    /// The figures a figure query expects; panics on lines or rows.
    fn figures(&self, id: &str) -> &BTreeMap<String, String> {
        match self.values(id) {
            Values::Figures(f) => f,
            _ => panic!("{id}: {} expects figures (a table)", self.query),
        }
    }

    /// The period `[from, to]` a period query covers, each date omitted
    /// resolved by the one rule (rules.md B-30) on `setup.today`; panics
    /// on `as_of`, on an omitted date without `setup.today`, and on a
    /// period the rule refuses.
    fn period(&self, ex: &Example) -> (NaiveDate, NaiveDate) {
        self.resolved_period(ex)
            .unwrap_or_else(|e| panic!("{}: {}: {e}", ex.id, self.query))
    }

    /// [`Self::period`], or the rule's refusal.
    fn resolved_period(&self, ex: &Example) -> Result<(NaiveDate, NaiveDate), LedgerError> {
        let id = &ex.id;
        assert!(
            self.as_of.is_none(),
            "{id}: {} covers a period (from, to), not as_of",
            self.query
        );
        let from = self.from.as_deref().map(|s| date(s, id));
        let to = self.to.as_deref().map(|s| date(s, id));
        // A `period` query asks the rule itself, both dates given or not.
        if let (Some(from), Some(to)) = (from, to)
            && self.query != "period"
        {
            assert!(from <= to, "{id}: {} from {from} after to {to}", self.query);
            return Ok((from, to));
        }
        defaults(ex, &self.query).statement(from, to)
    }

    /// The day an as-of query is made as of (omitted: `setup.today`,
    /// rules.md B-30); panics on `from` or `to`.
    fn day(&self, ex: &Example) -> NaiveDate {
        let id = &ex.id;
        assert!(
            self.from.is_none() && self.to.is_none(),
            "{id}: {} is as of a day (as_of), not a period",
            self.query
        );
        match self.as_of.as_deref() {
            Some(s) => date(s, id),
            None => defaults(ex, &self.query).as_of(None),
        }
    }
}

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
        other => return format!("another refusal: {other}"),
    }
    .to_string()
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

/// An example's books: its setup, then its events as they are applied.
struct Books<'a> {
    ex: &'a Example,
    l: Ledger,
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
    /// The payment methods, by label.
    instruments: BTreeMap<String, InstrumentId>,
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
            let kind = kind.and_then(ExemptReason::parse).unwrap_or_else(|| {
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
fn add_parties(ex: &Example, l: &mut Ledger) -> BTreeMap<String, PartyId> {
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
                let v = l
                    .add_vendor(&p.name)
                    .unwrap_or_else(|e| panic!("{what}: {e}"));
                l.party_of_vendor(v).expect("a vendor's party")
            }
            (false, true) => {
                let c = l
                    .add_customer(&p.name)
                    .unwrap_or_else(|e| panic!("{what}: {e}"));
                l.party_of_customer(c).expect("a customer's party")
            }
            (false, false) => l
                .add_party(&p.name, &others)
                .unwrap_or_else(|e| panic!("{what}: {e}")),
        };
        if record(Role::Vendor) || record(Role::Customer) {
            for r in &others {
                l.set_party_role(id, *r, true)
                    .unwrap_or_else(|e| panic!("{what}: {e}"));
            }
        }
        for code in &p.creditor_of {
            let account = l
                .chart()
                .find_by_code(code)
                .unwrap_or_else(|| panic!("{what}: no account {code}"))
                .id;
            l.set_party_creditor(id, account, true)
                .unwrap_or_else(|e| panic!("{what}: creditor of {code}: {e}"));
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
            l.set_party_tin(id, Some(facts))
                .unwrap_or_else(|e| panic!("{what}: {e}"));
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
            l.set_party_w9(id, Some(facts))
                .unwrap_or_else(|e| panic!("{what}: {e}"));
        }
        assert!(out.insert(p.name.clone(), id).is_none(), "{what}: twice");
    }
    out
}

impl<'a> Books<'a> {
    /// The books an example sets up, before any event: its shape checked,
    /// its accounts, fixed assets, parties, members, payees, their dated
    /// decisions, and payment methods.
    fn new(ex: &'a Example) -> Self {
        let s = &ex.setup;
        let currency = Currency::new(&s.currency).unwrap();
        let mut l = Ledger::new(currency);
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
            // A parent is declared before its subaccounts.
            let parent = a.parent.as_deref().map(|p| id(&l, p));
            let account = l
                .add_account(&a.code, &a.name, a.account_type(&ex.id), parent, a.header)
                .unwrap_or_else(|e| panic!("{}: account {}: {e}", ex.id, a.code));
            if let Some(c) = &a.tax_category {
                l.set_account_tax(account, Some(c), None)
                    .unwrap_or_else(|e| panic!("{}: account {}: {e}", ex.id, a.code));
            }
        }
        let first = ex
            .events
            .first()
            .map_or(date("2026-01-01", "start"), |e| date(&e.date, "date"));
        for fa in &s.fixed_assets {
            l.register_asset(NewAsset {
                description: format!("Fixed asset on {}", fa.cost_account),
                purchase_date: first,
                cost: Money::new(Decimal::ONE, currency),
                salvage_value: Money::new(Decimal::ZERO, currency),
                useful_life_months: 12,
                cost_account: id(&l, &fa.cost_account),
                accum_depr_account: id(&l, &fa.accumulated_account),
                expense_account: id(&l, &fa.expense_account),
            })
            .unwrap_or_else(|e| panic!("{}: fixed asset: {e}", ex.id));
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
                capital_account: id(&l, &m.capital_account),
                distribution_account: m.distributions_account.as_deref().map(|c| id(&l, c)),
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
        let parties = add_parties(ex, &mut l);
        // A member who is a party (by name, with the member role), as the
        // settings' members are (settings::Company::party_members).
        let party_members: Vec<PartyMember> = members
            .iter()
            .filter_map(|m| {
                let p = l.party_by_name(&m.name)?;
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
            let form = Form1099::parse(&p.form)
                .filter(|_| p.form.starts_with("1099-"))
                .unwrap_or_else(|| panic!("{}: payee {}: form {:?}", ex.id, p.name, p.form));
            // A payee in the directory with the vendor role is that
            // party's vendor record.
            let v = match parties.get(&p.name).and_then(|&id| l.get_party(id)?.vendor) {
                Some(v) => v,
                None => l
                    .add_vendor(&p.name)
                    .unwrap_or_else(|e| panic!("{}: payee {}: {e}", ex.id, p.name)),
            };
            let dated = Dated1099::from_beginning(Decision1099::Files(form), Some(source.clone()));
            l.set_vendor_1099(v, dated, false)
                .unwrap_or_else(|e| panic!("{}: payee {}: {e}", ex.id, p.name));
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
            l.set_vendor_1099(v, dated, false)
                .unwrap_or_else(|e| panic!("{what}: {e}"));
        }
        let mut instruments = BTreeMap::new();
        for i in &s.instruments {
            let what = format!("{} instrument {}", ex.id, i.label);
            let kind = InstrumentKind::parse(&i.kind)
                .unwrap_or_else(|| panic!("{what}: kind {:?}", i.kind));
            let account = l
                .chart()
                .find_by_code(&i.account)
                .unwrap_or_else(|| panic!("{what}: no account {}", i.account))
                .id;
            let owner = i.owner.as_ref().map(|o| {
                *parties
                    .get(o)
                    .unwrap_or_else(|| panic!("{what}: owner {o:?} not in setup.parties"))
            });
            let id = l
                .add_instrument(kind, &i.label, i.last4.as_deref(), owner, account)
                .unwrap_or_else(|e| panic!("{what}: {e}"));
            assert!(
                instruments.insert(i.label.clone(), id).is_none(),
                "{what}: twice"
            );
        }
        Books {
            ex,
            l,
            currency,
            members,
            party_members,
            parties,
            vendors: payees.clone(),
            payees,
            instruments,
        }
    }

    fn id(&self, code: &str) -> ledger_core::AccountId {
        self.l
            .chart()
            .find_by_code(code)
            .unwrap_or_else(|| panic!("{}: no account {code}", self.ex.id))
            .id
    }

    /// Event `n` (counting from 1), which must post entry #n.
    fn apply(&mut self, n: EntryId, e: &Event) {
        let ex = &self.ex.id;
        let when = date(&e.date, &format!("{ex} event {n}"));
        assert!(
            e.payee.is_none() || (e.reverses.is_none() && e.close.is_none() && e.bill.is_none()),
            "{ex} event {n}: a reversal takes its original's payee, a bill names its vendor, and a close has none"
        );
        assert!(
            (e.parties.is_empty() && e.instrument.is_none()) || e.reverses.is_none(),
            "{ex} event {n}: a reversal takes its original's parties and payment method"
        );
        let posted = match (&e.close, e.reverses, e.lines.is_empty()) {
            (None, None, true) if e.bill.is_some() => {
                let b = e.bill.as_ref().unwrap();
                let amount = exact(&b.amount, ex);
                assert!(
                    amount > Decimal::ZERO,
                    "{ex} event {n}: amounts are positive"
                );
                let vendor = match self.vendors.get(&b.vendor) {
                    Some(&v) => v,
                    None => {
                        let v = self
                            .l
                            .add_vendor(&b.vendor)
                            .unwrap_or_else(|err| panic!("{ex} event {n}: {err}"));
                        self.vendors.insert(b.vendor.clone(), v);
                        v
                    }
                };
                let bill = self
                    .l
                    .create_bill(
                        vendor,
                        when,
                        when,
                        vec![DocumentLine {
                            description: e.description.clone(),
                            amount,
                        }],
                    )
                    .unwrap_or_else(|err| panic!("{ex} event {n}: {err}"));
                self.l.get_bill(bill).expect("the bill just made").entry_id
            }
            (_, _, _) if e.bill.is_some() => {
                panic!("{ex} event {n}: an event is one of lines, reverses, close or bill")
            }
            (Some(c), None, true) => {
                let p = self
                    .l
                    .create_period(
                        &format!("event {n}"),
                        date(&c.start, &format!("{ex} event {n}")),
                        when,
                    )
                    .unwrap_or_else(|err| panic!("{ex} event {n}: {err}"));
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
                        "{ex} event {n}: a close to the members needs their shares"
                    );
                }
                let to = match (c.to.as_str(), c.rounding.as_deref()) {
                    ("members", None) => CloseTo::Members(members),
                    ("members", Some("per-close")) => CloseTo::MembersPerClose(members),
                    ("retained_earnings", None) => CloseTo::RetainedEarnings { sweep: vec![] },
                    other => panic!("{ex} event {n}: close {other:?}"),
                };
                self.l
                    .close_period_to(p, &to)
                    .unwrap_or_else(|err| panic!("{ex} event {n}: {err}"))
                    .unwrap_or_else(|| panic!("{ex} event {n}: the close has nothing to close"))
            }
            (None, Some(of), true) => {
                // Events count from 1, as entries do: `reverses = 1` is
                // the first event (bookspec/README.md).
                assert!(
                    of >= 1,
                    "{ex} event {n}: reverses = {of}: events count from 1"
                );
                assert!(
                    (of as EntryId) < n,
                    "{ex} event {n}: reverses a later event"
                );
                self.l
                    .reverse_entry(of as EntryId, when, &e.description)
                    .unwrap_or_else(|err| panic!("{ex} event {n}: {err}"))
            }
            (None, Some(_), false) => panic!("{ex} event {n}: a reversal has no lines"),
            (None, None, false) => {
                let entry = self.l.create_entry(when, &e.description);
                for line in &e.lines {
                    let side = match line.side.as_str() {
                        "debit" => Side::Debit,
                        "credit" => Side::Credit,
                        other => panic!("{ex} event {n}: side {other:?}"),
                    };
                    let amount = exact(&line.amount, ex);
                    assert!(
                        amount > Decimal::ZERO,
                        "{ex} event {n}: amounts are positive"
                    );
                    let account = self.id(&line.account);
                    self.l
                        .add_line(entry, account, side, Money::new(amount, self.currency))
                        .unwrap_or_else(|err| panic!("{ex} event {n}: {err}"));
                }
                self.l
                    .post_entry(entry)
                    .unwrap_or_else(|err| panic!("{ex} event {n}: {err}"));
                entry
            }
            _ => panic!("{ex} event {n}: an event is one of lines, reverses, close or bill"),
        };
        assert_eq!(posted, n, "{ex}: event {n} must be entry #{n}");
        if !e.parties.is_empty() {
            let shares: Vec<PartyShare> = e
                .parties
                .iter()
                .map(|sh| {
                    let party = *self.parties.get(&sh.party).unwrap_or_else(|| {
                        panic!("{ex} event {n}: party {:?} not in setup", sh.party)
                    });
                    let share = match (&sh.amount, &sh.percent) {
                        (None, None) => Share::Derived,
                        (Some(a), None) => Share::Amount(exact(a, ex)),
                        (None, Some(p)) => Share::Percent(exact(p, ex)),
                        _ => panic!("{ex} event {n}: a share is an amount or a percent"),
                    };
                    PartyShare { party, share }
                })
                .collect();
            self.l
                .set_entry_parties(n, &shares)
                .unwrap_or_else(|err| panic!("{ex} event {n}: {err}"));
        }
        // The payee after the parties: a payee's party must be among
        // several named (the vendor tag follows the parties).
        if let Some(p) = &e.payee {
            let v = *self
                .payees
                .get(p)
                .unwrap_or_else(|| panic!("{ex} event {n}: payee {p:?} not in setup"));
            self.l
                .set_entry_vendor(n, Some(v))
                .unwrap_or_else(|err| panic!("{ex} event {n}: {err}"));
        }
        if let Some(label) = &e.instrument {
            let i = *self
                .instruments
                .get(label)
                .unwrap_or_else(|| panic!("{ex} event {n}: payment method {label:?} not in setup"));
            self.l
                .set_entry_instrument(n, Some(i))
                .unwrap_or_else(|err| panic!("{ex} event {n}: {err}"));
        }
    }
}

impl Books<'_> {
    /// Make a change that posts no entry (after event `after`).
    fn change(&mut self, c: &ChangeSpec) {
        let ex = &self.ex.id;
        let after = c.after;
        match (&c.rename, &c.require_tax_category) {
            (Some(r), None) => {
                let account = self.id(&r.code);
                self.l
                    .rename_account(account, &r.name)
                    .unwrap_or_else(|e| panic!("{ex} change after {after}: {e}"));
            }
            (None, Some(policy)) => {
                assert!(
                    matches!(policy.as_str(), "warn" | "refuse"),
                    "{ex} change after {after}: require_tax_category {policy:?}"
                );
                use ledger_core::settings::{Change, Source};
                let source = Source {
                    decided_by: "bookspec".into(),
                    decided_on: date("2026-01-01", ex),
                    citation: None,
                    note: None,
                };
                let changes = self
                    .l
                    .check_settings(
                        &[Change {
                            key: "policy.require_tax_category".into(),
                            value: Some(serde_json::json!(policy)),
                        }],
                        &source,
                    )
                    .unwrap_or_else(|e| panic!("{ex} change after {after}: {e}"));
                self.l
                    .apply_settings(&changes, &source, 1)
                    .unwrap_or_else(|e| panic!("{ex} change after {after}: {e}"));
            }
            _ => panic!(
                "{ex} change after {after}: a change is one of rename or require_tax_category"
            ),
        }
    }
}

/// The books an example sets up, with every event and change applied and
/// nothing checked.
fn load(ex: &Example) -> Books<'_> {
    let mut b = Books::new(ex);
    for (i, e) in ex.events.iter().enumerate() {
        b.apply(i as EntryId + 1, e);
        for c in ex.changes.iter().filter(|c| c.after == i + 1) {
            b.change(c);
        }
    }
    b
}

/// The `cash_flow` query's value for `key` (bookspec/README.md), or None
/// for a key it doesn't know.
fn cash_flow_value(cf: &CashFlowStatement, key: &str) -> Option<Decimal> {
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
                let n: EntryId = tail.parse().ok()?;
                return Some(
                    cf.noncash_disclosures
                        .iter()
                        .filter(|d| d.entry == n)
                        .map(|d| d.amount)
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
            // `<section>.<N>`, or `<section>.<N>.on_behalf`: of entry N's
            // cash there, what a member or lender handled for the company.
            let (n, on_behalf) = match tail.split_once('.') {
                Some((n, "on_behalf")) => (n, true),
                Some(_) => return None,
                None => (tail, false),
            };
            let n: EntryId = n.parse().ok()?;
            lines
                .iter()
                .filter(|l| l.entry == n)
                .map(|l| if on_behalf { l.on_behalf } else { l.amount })
                .sum()
        }
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
fn check_cash_flow(ex: &Example, l: &Ledger, members: &[MemberCapital], x: &Expect) {
    let (start, end) = x.period(ex);
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
        let got = cash_flow_value(&cf, key)
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
    let as_of = x.day(ex);
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
    let (start, end) = x.period(ex);
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
    let (ex, l) = (b.ex, &b.l);
    // A calendar-year count, never the fiscal default (rules.md B-30):
    // both dates given.
    assert!(
        x.from.is_some() && x.to.is_some(),
        "{}: {} gives from and to",
        ex.id,
        x.query
    );
    let (from, to) = x.period(ex);
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
    let (ex, l) = (b.ex, &b.l);
    assert!(
        x.from.is_none() && x.to.is_none() && x.as_of.is_none(),
        "{}: party_part asks about one entry, no dates",
        ex.id
    );
    let n = x
        .entry
        .unwrap_or_else(|| panic!("{}: a party_part query names its entry", ex.id));
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
    let (ex, l) = (b.ex, &b.l);
    assert!(
        x.from.is_some() && x.to.is_some(),
        "{}: party_totals gives from and to",
        ex.id
    );
    let (from, to) = x.period(ex);
    let basis = x
        .basis
        .as_deref()
        .map(|k| Basis::parse(k).unwrap_or_else(|| panic!("{}: basis {k:?}", ex.id)))
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
    let got =
        b.l.get_party(party)
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
    let (from, to) = x.period(ex);
    assert!(
        !members.is_empty() && members.iter().all(|m| m.share > Decimal::ZERO),
        "{}: k1 needs the members and their shares",
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
    x.day(ex);
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

/// Check one `entry` expectation: entry N's lines, `"<code>.debit"` and
/// `"<code>.credit"` each summing that account's lines on that side,
/// every line of the entry named.
fn check_entry(ex: &Example, l: &Ledger, x: &Expect) {
    assert!(
        x.from.is_none() && x.to.is_none() && x.as_of.is_none(),
        "{}: entry asks about one entry, no dates",
        ex.id
    );
    let n = x
        .entry
        .unwrap_or_else(|| panic!("{}: an entry query names its entry", ex.id));
    let entry = l
        .get_entry(n)
        .unwrap_or_else(|| panic!("{}: no entry {n}", ex.id));
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
    let as_of = x.day(ex);
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
    let (from, to) = x.period(ex);
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
            let kind = a.account_type(&ex.id);
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
    match (x.resolved_period(ex), &x.refused) {
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
    assert_eq!(x.day(ex), want, "{}: day", ex.id);
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
    let (ex, l) = (b.ex, &b.l);
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
        "cash_flow" => check_cash_flow(ex, l, &b.members, x),
        "trial_balance" => check_trial_balance(ex, l, x),
        "profit_and_loss" => check_profit_and_loss(ex, l, x),
        "vendor_1099_paid" => check_vendor_1099_paid(b, x),
        "vendor_1099" => check_vendor_1099(b, x),
        "entry" => check_entry(ex, l, x),
        "balances" => check_balances(ex, l, x),
        "members_equity" => check_members_equity(ex, l, &b.members, x),
        "k1" => check_k1(ex, l, &b.members, x),
        "party_part" => check_party_part(b, x),
        "party_totals" => check_party_totals(b, x),
        "party_tin_last4" => check_party_tin_last4(b, x),
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
        b.apply(i as EntryId + 1, e);
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
    out
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
        let ex: Example = toml::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", f.display()));
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
    // Each query the examples use ran: the cash flow's six examples at
    // least, the as-of and 1099 examples', the members closes', the
    // formats appendix's, and P3's default period, rename, tax category,
    // parties and 1099 examples'.
    for (q, at_least) in [
        ("cash_flow", 6),
        ("trial_balance", 2),
        ("profit_and_loss", 2),
        ("vendor_1099_paid", 4),
        ("vendor_1099", 10),
        ("entry", 8),
        ("balances", 4),
        ("members_equity", 10),
        ("k1", 2),
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
    let r = b.l.get_entry(3).unwrap();
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
