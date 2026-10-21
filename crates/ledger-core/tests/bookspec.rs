//! The bookspec examples (`bookspec/examples/*.toml`, P13, D-34), checked
//! against the engine on every run, so they can't rot. This is the one
//! place every example is loaded.
//!
//! Each example is a setup, events and expected values worked by hand from
//! its cited source (`bookspec/README.md` gives the shape). Every example
//! must load, whatever it asks or whichever appendix it is in: its shape,
//! its accounts, fixed assets, members and payees, and every event, an
//! entry, a reversal or a close, posting entry #N as event N. The queries
//! the core answers (`cash_flow`, `trial_balance`, `profit_and_loss`,
//! `vendor_1099_paid`, `entry`, `balances`, `members_equity`) are then
//! checked value by value, each after the event it names (`after`) or the
//! last; the formats appendix's (`iif_lines`, `iif_changes`) are checked by
//! ledger-package (`the_bookspec_iif_example_matches_the_engine`), and a
//! query no crate answers is refused. A key the shape doesn't have is
//! refused too, never ignored.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use chrono::NaiveDate;
use ledger_core::{
    AccountType, CashFlowParams, CashFlowSection, CashFlowStatement, CloseTo, Currency, EntryId,
    Form1099, Ledger, MemberCapital, MemberClose, Money, NewAsset, PaymentAccounts,
    ReconcilingKind, Side, VendorId, cash_flow, members_equity, profit_and_loss_for_period,
    trial_balance_as_of, vendor_1099_report_with_members,
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
    #[serde(default)]
    members: Vec<MemberSpec>,
    #[serde(default)]
    payees: Vec<PayeeSpec>,
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
    name: String,
    capital_account: String,
    /// Their share of income, an exact decimal fraction; every member has
    /// one or none does, and the shares sum to 1. A close to the members
    /// needs them.
    #[serde(default)]
    share: Option<String>,
}

/// A payee marked for an information return (rules.md B-19).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PayeeSpec {
    name: String,
    form: String,
}

/// One event: an entry (`lines`), a reversal (`reverses`) or a close
/// (`close`), exactly one of them.
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
    /// The payee (in `setup.payees`) the entry paid.
    #[serde(default)]
    payee: Option<String>,
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
    /// The entry an `entry` query asks about.
    entry: Option<EntryId>,
    /// The books after event N, not after the last.
    after: Option<usize>,
    values: Values,
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
    /// The figures a figure query expects; panics on lines or rows.
    fn figures(&self, id: &str) -> &BTreeMap<String, String> {
        match &self.values {
            Values::Figures(f) => f,
            _ => panic!("{id}: {} expects figures (a table)", self.query),
        }
    }

    /// The period `[from, to]` a period query covers; panics on `as_of`.
    fn period(&self, id: &str) -> (NaiveDate, NaiveDate) {
        assert!(
            self.as_of.is_none(),
            "{id}: {} covers a period (from, to), not as_of",
            self.query
        );
        let from = date(
            self.from
                .as_deref()
                .unwrap_or_else(|| panic!("{id}: {} needs from", self.query)),
            id,
        );
        let to = date(
            self.to
                .as_deref()
                .unwrap_or_else(|| panic!("{id}: {} needs to", self.query)),
            id,
        );
        assert!(from <= to, "{id}: {} from {from} after to {to}", self.query);
        (from, to)
    }

    /// The day an as-of query is made as of; panics on `from` or `to`.
    fn day(&self, id: &str) -> NaiveDate {
        assert!(
            self.from.is_none() && self.to.is_none(),
            "{id}: {} is as of a day (as_of), not a period",
            self.query
        );
        date(
            self.as_of
                .as_deref()
                .unwrap_or_else(|| panic!("{id}: {} needs as_of", self.query)),
            id,
        )
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

/// An example's books: its setup, then its events as they are applied.
struct Books<'a> {
    ex: &'a Example,
    l: Ledger,
    currency: Currency,
    /// The members, with their shares (zero when the example gives none).
    members: Vec<MemberCapital>,
    /// The payees' vendor records, by name.
    payees: BTreeMap<String, VendorId>,
}

impl<'a> Books<'a> {
    /// The books an example sets up, before any event: its shape checked,
    /// its accounts, fixed assets, members and payees.
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
        let (m, d) = s
            .fiscal_year_start
            .split_once('-')
            .unwrap_or_else(|| panic!("{}: fiscal_year_start {:?}", ex.id, s.fiscal_year_start));
        assert!(
            NaiveDate::from_ymd_opt(2001, m.parse().unwrap(), d.parse().unwrap()).is_some(),
            "{}: fiscal_year_start",
            ex.id
        );
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
            let t = match a.kind.as_str() {
                "asset" => AccountType::Asset,
                "liability" => AccountType::Liability,
                "equity" => AccountType::Equity,
                "revenue" => AccountType::Revenue,
                "expense" => AccountType::Expense,
                other => panic!("{}: account type {other:?}", ex.id),
            };
            // A parent is declared before its subaccounts.
            let parent = a.parent.as_deref().map(|p| id(&l, p));
            l.add_account(&a.code, &a.name, t, parent, false)
                .unwrap_or_else(|e| panic!("{}: account {}: {e}", ex.id, a.code));
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
                distribution_account: None,
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
        let mut payees = BTreeMap::new();
        for p in &s.payees {
            let form = Form1099::parse(&p.form)
                .unwrap_or_else(|| panic!("{}: payee {}: form {:?}", ex.id, p.name, p.form));
            let v = l
                .add_vendor(&p.name)
                .unwrap_or_else(|e| panic!("{}: payee {}: {e}", ex.id, p.name));
            l.set_vendor_1099(v, Some(form), false).unwrap();
            assert!(
                payees.insert(p.name.clone(), v).is_none(),
                "{}: payee {} twice",
                ex.id,
                p.name
            );
        }
        Books {
            ex,
            l,
            currency,
            members,
            payees,
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
            e.payee.is_none() || (e.reverses.is_none() && e.close.is_none()),
            "{ex} event {n}: a reversal takes its original's payee, and a close has none"
        );
        let posted = match (&e.close, e.reverses, e.lines.is_empty()) {
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
            _ => panic!("{ex} event {n}: an event is one of lines, reverses or close"),
        };
        assert_eq!(posted, n, "{ex}: event {n} must be entry #{n}");
        if let Some(p) = &e.payee {
            let v = *self
                .payees
                .get(p)
                .unwrap_or_else(|| panic!("{ex} event {n}: payee {p:?} not in setup"));
            self.l
                .set_entry_vendor(n, Some(v))
                .unwrap_or_else(|err| panic!("{ex} event {n}: {err}"));
        }
    }
}

/// The books an example sets up, with every event applied and nothing
/// checked.
fn load(ex: &Example) -> Books<'_> {
    let mut b = Books::new(ex);
    for (i, e) in ex.events.iter().enumerate() {
        b.apply(i as EntryId + 1, e);
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
    let (start, end) = x.period(&ex.id);
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
    let as_of = x.day(&ex.id);
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
    let (start, end) = x.period(&ex.id);
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

/// Check one `vendor_1099_paid` expectation: each payee's payments out of
/// the cash (and financing) accounts from January 1 through `to`, a
/// payment card's left out, every payee listed (rules.md B-19, B-33).
fn check_vendor_1099_paid(
    ex: &Example,
    l: &Ledger,
    payees: &BTreeMap<String, VendorId>,
    x: &Expect,
) {
    let (from, to) = x.period(&ex.id);
    let year = chrono::Datelike::year(&to);
    assert_eq!(
        Some(from),
        NaiveDate::from_ymd_opt(year, 1, 1),
        "{}: a 1099 counts a calendar year: from is January 1 of to's year",
        ex.id
    );
    assert!(
        !ex.setup.cash_accounts.is_empty(),
        "{}: vendor_1099_paid needs cash_accounts (what payees are paid from)",
        ex.id
    );
    let sources = PaymentAccounts {
        cash: codes(l, &ex.setup.cash_accounts, &ex.id),
        financing: codes(l, &ex.setup.financing_accounts, &ex.id),
    };
    let r = vendor_1099_report_with_members(l, year, Some(to), None, &sources, &[])
        .unwrap_or_else(|e| panic!("{}: {e}", ex.id));
    exactly(ex, x, &payees.keys().cloned().collect());
    for (name, want) in x.figures(&ex.id) {
        let want = exact(want, &format!("{} {name}", ex.id));
        let got: Decimal = r
            .vendors
            .iter()
            .filter(|v| v.vendor_id == payees[name])
            .map(|v| v.paid)
            .sum();
        assert_eq!(got, want, "{} 1099 paid [{from} .. {to}]: {name}", ex.id);
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
    x.day(&ex.id);
    assert!(
        x.after.is_none(),
        "{}: {}: a format query is of the books after every event",
        ex.id,
        x.query
    );
    match (x.query.as_str(), &x.values) {
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
    let as_of = x.day(&ex.id);
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
    let (from, to) = x.period(&ex.id);
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

/// Check one expectation against the books as they stand: a query the
/// core answers, value by value; a format query's shape, its values left
/// to the test [`FORMAT_QUERIES`] names (returned); a query no crate
/// answers is refused.
fn check(b: &Books<'_>, x: &Expect) -> Option<&'static str> {
    let (ex, l) = (b.ex, &b.l);
    assert!(
        !x.values.is_empty(),
        "{}: {} expects nothing",
        ex.id,
        x.query
    );
    assert!(
        x.entry.is_none() || x.query == "entry",
        "{}: only an entry query names an entry",
        ex.id
    );
    match x.query.as_str() {
        "cash_flow" => check_cash_flow(ex, l, &b.members, x),
        "trial_balance" => check_trial_balance(ex, l, x),
        "profit_and_loss" => check_profit_and_loss(ex, l, x),
        "vendor_1099_paid" => check_vendor_1099_paid(ex, l, &b.payees, x),
        "entry" => check_entry(ex, l, x),
        "balances" => check_balances(ex, l, x),
        "members_equity" => check_members_equity(ex, l, &b.members, x),
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
/// on the books after event N, the rest after the last. Gives each query
/// checked, with the test of another crate that checks its values, if
/// any.
fn run(ex: &Example) -> Vec<(String, Option<&'static str>)> {
    for x in &ex.expect {
        assert!(
            x.after.is_none_or(|n| n >= 1 && n <= ex.events.len()),
            "{}: after = {:?}: events count from 1, to {}",
            ex.id,
            x.after,
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
    // least, the as-of and 1099 examples', the members closes' and the
    // formats appendix's.
    for (q, at_least) in [
        ("cash_flow", 6),
        ("trial_balance", 2),
        ("profit_and_loss", 2),
        ("vendor_1099_paid", 3),
        ("entry", 8),
        ("balances", 4),
        ("members_equity", 5),
        ("iif_lines", 1),
        ("iif_changes", 1),
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
