//! The bookspec examples about closes to the members
//! (`bookspec/examples/members-close.*.toml`; P13, D-34; P3 defect 17),
//! checked against the engine on every run, so they can't rot.
//!
//! Each example is a setup, events and expected values worked by hand from
//! its cited source. Its shape is bookspec's (`bookspec/README.md`) with
//! what closes need:
//!
//! - `[[setup.members]]` takes `share`, an exact decimal fraction (the
//!   shares sum to 1), beside `name` and `capital_account`;
//! - an event may be a close instead of an entry: `close = { start, to,
//!   rounding? }` closes the period from `start` through the event's
//!   `date` to `"members"` (by share, in the order listed) or
//!   `"retained_earnings"`; `rounding = "per-close"` makes a members close
//!   split its own net alone (books from an engine that rounded each close
//!   by itself), and the default is the cumulative rule. A close is an
//!   event like any other: event N posts entry #N, so a close must have
//!   something to close;
//! - an `[[expect]]` may take `after = N`: the books after event N, not
//!   after the last;
//! - three queries: `entry` (`entry = N`: its lines, `"<code>.debit"` and
//!   `"<code>.credit"` summing each account's lines on that side, every
//!   line of the entry named), `balances` (`as_of`: each account named,
//!   `"<code>"`, its balance on its normal side, so a member's capital
//!   after a loss is negative), and `members_equity` (`from`, `to`: the
//!   statement of changes in members' equity, `"<name>.beginning"`,
//!   `.contributions`, `.net_income`, `.distributions`, `.other`,
//!   `.ending`).
//!
//! A key the shape doesn't have is refused, never ignored.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::NaiveDate;
use ledger_core::{
    AccountType, CloseTo, Currency, Ledger, MemberCapital, MemberClose, Money, Side,
    members_equity, reports::trial_balance_as_of,
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
    members: Vec<MemberSpec>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AccountSpec {
    code: String,
    name: String,
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MemberSpec {
    name: String,
    capital_account: String,
    share: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Event {
    date: String,
    description: String,
    #[serde(default)]
    lines: Vec<LineSpec>,
    #[serde(default)]
    reverses: Option<u64>,
    #[serde(default)]
    close: Option<CloseSpec>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LineSpec {
    account: String,
    side: String,
    amount: String,
}

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
    #[serde(default)]
    from: Option<String>,
    #[serde(default)]
    to: Option<String>,
    #[serde(default)]
    as_of: Option<String>,
    #[serde(default)]
    entry: Option<u64>,
    #[serde(default)]
    after: Option<usize>,
    values: BTreeMap<String, String>,
}

fn examples() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bookspec/examples");
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("members-close.") && n.ends_with(".toml"))
        })
        .collect();
    paths.sort();
    paths
}

fn date(s: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap_or_else(|_| panic!("date {s:?}"))
}

/// An exact decimal string, never a float.
fn exact(s: &str) -> Decimal {
    assert!(
        !s.contains(['e', 'E']) && s.trim() == s,
        "{s:?} is not a decimal string"
    );
    s.parse()
        .unwrap_or_else(|_| panic!("{s:?} is not a decimal"))
}

fn account_type(kind: &str) -> AccountType {
    match kind {
        "asset" => AccountType::Asset,
        "liability" => AccountType::Liability,
        "equity" => AccountType::Equity,
        "revenue" => AccountType::Revenue,
        "expense" => AccountType::Expense,
        other => panic!("account type {other:?}"),
    }
}

fn side(s: &str) -> Side {
    match s {
        "debit" => Side::Debit,
        "credit" => Side::Credit,
        other => panic!("side {other:?}"),
    }
}

struct Books<'a> {
    ex: &'a Example,
    l: Ledger,
    currency: Currency,
}

impl Books<'_> {
    fn id(&self, code: &str) -> ledger_core::AccountId {
        self.l
            .chart()
            .find_by_code(code)
            .unwrap_or_else(|| panic!("{}: no account {code}", self.ex.id))
            .id
    }

    fn members(&self) -> Vec<MemberCapital> {
        self.ex
            .setup
            .members
            .iter()
            .map(|m| MemberCapital {
                name: m.name.clone(),
                capital_account: self.id(&m.capital_account),
                distribution_account: None,
                share: exact(&m.share),
            })
            .collect()
    }

    /// Event `n` (1-based), which must post entry #n.
    fn apply(&mut self, n: u64, e: &Event) {
        let ex = &self.ex.id;
        let day = date(&e.date);
        let posted = match (&e.close, e.reverses, e.lines.is_empty()) {
            (Some(c), None, true) => {
                let p = self
                    .l
                    .create_period(&format!("event {n}"), date(&c.start), day)
                    .unwrap();
                let members: Vec<MemberClose> = self
                    .members()
                    .iter()
                    .map(|m| MemberClose {
                        capital: m.capital_account,
                        share: m.share,
                        distributions: None,
                    })
                    .collect();
                let to = match (c.to.as_str(), c.rounding.as_deref()) {
                    ("members", None) => CloseTo::Members(members),
                    ("members", Some("per-close")) => CloseTo::MembersPerClose(members),
                    ("retained_earnings", None) => CloseTo::RetainedEarnings { sweep: vec![] },
                    other => panic!("{ex}: close {other:?}"),
                };
                self.l
                    .close_period_to(p, &to)
                    .unwrap()
                    .unwrap_or_else(|| panic!("{ex}: event {n} closes nothing"))
            }
            (None, Some(k), true) => {
                assert!(k >= 1 && k < n, "{ex}: event {n} reverses {k}");
                self.l.reverse_entry(k, day, &e.description).unwrap()
            }
            (None, None, false) => {
                let id = self.l.create_entry(day, &e.description);
                for line in &e.lines {
                    let account = self.id(&line.account);
                    let money = Money::new(exact(&line.amount), self.currency);
                    self.l
                        .add_line(id, account, side(&line.side), money)
                        .unwrap();
                }
                self.l.post_entry(id).unwrap();
                id
            }
            _ => panic!("{ex}: event {n} is one of lines, reverses or close"),
        };
        assert_eq!(posted, n, "{ex}: event {n} posted entry {posted}");
    }

    /// What `q` asks, for each of its keys.
    fn answer(&self, q: &Expect) -> BTreeMap<String, Decimal> {
        let ex = &self.ex.id;
        match q.query.as_str() {
            "entry" => {
                let id = q
                    .entry
                    .unwrap_or_else(|| panic!("{ex}: entry query without entry"));
                let mut out = BTreeMap::new();
                for line in self.l.get_entry(id).unwrap().lines() {
                    let code = &self.l.chart().get(line.account).unwrap().code;
                    let side = match line.side {
                        Side::Debit => "debit",
                        Side::Credit => "credit",
                    };
                    *out.entry(format!("{code}.{side}")).or_default() += line.amount.amount;
                }
                out
            }
            "balances" => {
                let as_of = date(q.as_of.as_deref().expect("balances take as_of"));
                let tb = trial_balance_as_of(&self.l, as_of);
                q.values
                    .keys()
                    .map(|code| {
                        let account = self.l.chart().get(self.id(code)).unwrap();
                        let (dr, cr) = tb
                            .lines
                            .iter()
                            .find(|l| &l.code == code)
                            .map_or((Decimal::ZERO, Decimal::ZERO), |l| (l.debit, l.credit));
                        let normal = match account.account_type.normal_side() {
                            Side::Debit => dr - cr,
                            Side::Credit => cr - dr,
                        };
                        (code.clone(), normal)
                    })
                    .collect()
            }
            "members_equity" => {
                let from = date(q.from.as_deref().expect("members_equity takes from"));
                let to = date(q.to.as_deref().expect("members_equity takes to"));
                let st = members_equity(&self.l, from, to, &self.members()).unwrap();
                let mut out = BTreeMap::new();
                for m in &st.members {
                    for (field, v) in [
                        ("beginning", m.beginning),
                        ("contributions", m.contributions),
                        ("net_income", m.net_income),
                        ("distributions", m.distributions),
                        ("other", m.other),
                        ("ending", m.ending),
                    ] {
                        out.insert(format!("{}.{field}", m.name), v);
                    }
                }
                out
            }
            other => panic!("{ex}: query {other:?}"),
        }
    }

    fn check(&self, q: &Expect) {
        let ex = &self.ex.id;
        let got = self.answer(q);
        for (key, want) in &q.values {
            let got = got
                .get(key)
                .unwrap_or_else(|| panic!("{ex}: {} has no {key}", q.query));
            assert_eq!(*got, exact(want), "{ex}: {} {key}", q.query);
        }
        if q.query == "entry" {
            // Every line named: the entry is exactly what is expected.
            let named: Vec<&String> = q.values.keys().collect();
            let lines: Vec<&String> = got.keys().collect();
            assert_eq!(lines, named, "{ex}: entry {:?}'s lines", q.entry);
        }
    }
}

/// Every members-close example loads, and the engine computes every value
/// it expects, after the event it names or the last.
#[test]
fn the_members_close_examples_hold() {
    let paths = examples();
    assert!(paths.len() >= 4, "{} examples", paths.len());
    for path in paths {
        let text = std::fs::read_to_string(&path).unwrap();
        let ex: Example =
            toml::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert_eq!(
            path.file_stem().unwrap().to_str().unwrap(),
            ex.id,
            "an example's file is named for its id"
        );
        assert!(!ex.title.is_empty() && !ex.rule.is_empty() && !ex.sources.is_empty());
        assert!(!ex.tags.is_empty());
        assert!(["AUTHORITY", "CONVENTION", "CHOICE"].contains(&ex.status.as_str()));
        assert_eq!(
            (ex.setup.basis.as_str(), ex.setup.rounding.as_str()),
            ("accrual", "largest-remainder"),
            "{}",
            ex.id
        );
        assert!(ex.setup.fiscal_year_start.len() == 5, "{}", ex.id);
        let shares: Decimal = ex.setup.members.iter().map(|m| exact(&m.share)).sum();
        assert_eq!(shares, Decimal::ONE, "{}: shares", ex.id);
        let currency = Currency::new(&ex.setup.currency).unwrap();
        let mut books = Books {
            ex: &ex,
            l: Ledger::new(currency),
            currency,
        };
        for a in &ex.setup.accounts {
            books
                .l
                .add_account(&a.code, &a.name, account_type(&a.kind), None, false)
                .unwrap();
        }
        for q in &ex.expect {
            assert!(
                q.after.is_none_or(|n| n >= 1 && n <= ex.events.len()),
                "{}: after {:?}",
                ex.id,
                q.after
            );
        }
        for (i, e) in ex.events.iter().enumerate() {
            books.apply(i as u64 + 1, e);
            for q in ex.expect.iter().filter(|q| q.after == Some(i + 1)) {
                books.check(q);
            }
        }
        for q in ex.expect.iter().filter(|q| q.after.is_none()) {
            books.check(q);
        }
    }
}
