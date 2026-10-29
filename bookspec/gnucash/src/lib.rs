//! GnuCash as a second bookspec engine (`plan/served.md` P13 step 5).
//!
//! The adapter keeps an example's chart and entries as the oracle's JSON
//! books (`oracle/gnucash/books.py`: accounts, entries, as-of days) and
//! asks GnuCash 5.10, in its pinned container, for each balance. GnuCash
//! is a cross-check, never the authority (`docs/oracle.md`, "Who is
//! right"): where it differs from an example, the difference is
//! adjudicated from the sources and recorded, not conformed to.
//!
//! Only what GnuCash itself answers is claimed: the balance queries
//! (`trial_balance`, `balances`), by the rules whose examples ask nothing
//! else. GnuCash never refuses an entry (it books the difference to an
//! `Imbalance` account), so a refused event is *accepted* here, as it
//! would be by a GnuCash user, and the runner reports the difference.

use std::collections::BTreeMap;
use std::io::Write;
use std::process::{Command, Stdio};
use std::str::FromStr;

use bookspec_checker::{
    Area, ChangeSpec, Claims, Engine, Event, Query, Refusal, RuleId, Setup, fallback_refusal,
};
use rust_decimal::Decimal;
use serde_json::{Value, json};

/// The oracle container (`just oracle` builds it from `oracle/gnucash`).
pub const IMAGE: &str = "wanderbooks-gnucash-oracle";

/// The rules this adapter claims: those whose examples ask only for
/// balances, which GnuCash answers itself.
pub fn claimed() -> Vec<RuleId> {
    vec![
        RuleId::new(Area::Entry, 1),
        RuleId::new(Area::Entry, 2),
        RuleId::new(Area::Entry, 3),
        RuleId::new(Area::Ledger, 1),
        RuleId::new(Area::Ledger, 3),
        RuleId::new(Area::Statement, 1),
        RuleId::new(Area::Statement, 2),
    ]
}

/// One account of the book handed to GnuCash.
struct Account {
    code: String,
    kind: &'static str,
    normal_debit: bool,
}

/// GnuCash, as bookspec's `Engine`.
#[derive(Default)]
pub struct GnuCash {
    accounts: Vec<Account>,
    entries: Vec<Value>,
    /// Each event's entry, by event number (1-based), for reversals.
    event_entries: BTreeMap<usize, usize>,
    events: usize,
}

impl GnuCash {
    /// An empty engine.
    pub fn new() -> Self {
        Self::default()
    }

    fn id(&self, code: &str) -> Option<usize> {
        self.accounts
            .iter()
            .position(|a| a.code == code)
            .map(|i| i + 1)
    }

    fn book(&self, as_of: &str) -> Value {
        let accounts: Vec<Value> = self
            .accounts
            .iter()
            .enumerate()
            .map(|(i, a)| json!({"id": i + 1, "code": a.code, "name": a.code, "type": a.kind}))
            .collect();
        json!({"accounts": accounts, "entries": self.entries, "as_of": [as_of]})
    }

    /// GnuCash's own balance of every account as of a day: debits
    /// positive, exact decimal strings, by account code.
    fn balances(&self, as_of: &str) -> Result<BTreeMap<String, Decimal>, Refusal> {
        let mut child = Command::new("docker")
            .args(["run", "--rm", "-i", IMAGE])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| unavailable(format!("docker: {e}")))?;
        let books = json!([self.book(as_of)]);
        child
            .stdin
            .take()
            .unwrap()
            .write_all(books.to_string().as_bytes())
            .map_err(|e| unavailable(format!("docker stdin: {e}")))?;
        let out = child
            .wait_with_output()
            .map_err(|e| unavailable(format!("docker: {e}")))?;
        if !out.status.success() {
            return Err(unavailable(
                "the GnuCash oracle failed; build it with `just oracle`".into(),
            ));
        }
        let results: Value = serde_json::from_slice(&out.stdout)
            .map_err(|e| unavailable(format!("oracle output: {e}")))?;
        let at = &results[0]["balances"][as_of];
        let mut map = BTreeMap::new();
        for (i, a) in self.accounts.iter().enumerate() {
            let v = at[(i + 1).to_string()]
                .as_str()
                .ok_or_else(|| unavailable(format!("no balance for {}", a.code)))?;
            map.insert(
                a.code.clone(),
                Decimal::from_str(v).map_err(|e| unavailable(e.to_string()))?,
            );
        }
        Ok(map)
    }
}

fn unavailable(why: String) -> Refusal {
    fallback_refusal(
        RuleId::new(Area::Ledger, 1),
        format!("the GnuCash oracle: {why}"),
    )
}

fn not_supported(what: &str) -> Refusal {
    fallback_refusal(
        RuleId::new(Area::Ledger, 1),
        format!("the GnuCash adapter does not take {what}"),
    )
}

impl Engine for GnuCash {
    fn load(&mut self, setup: &Setup) -> Result<(), Refusal> {
        for a in &setup.accounts {
            let (kind, normal_debit) = match a.kind.as_str() {
                "asset" => ("Asset", true),
                "liability" => ("Liability", false),
                "equity" => ("Equity", false),
                "revenue" => ("Revenue", false),
                "expense" => ("Expense", true),
                other => return Err(not_supported(&format!("account type {other}"))),
            };
            self.accounts.push(Account {
                code: a.code.clone(),
                kind,
                normal_debit,
            });
        }
        if setup.currency != "USD" {
            return Err(not_supported("a currency other than USD"));
        }
        Ok(())
    }

    fn apply(&mut self, event: &Event) -> Result<(), Refusal> {
        self.events += 1;
        let n = self.events;
        let date = event
            .date
            .clone()
            .ok_or_else(|| not_supported("an undated event"))?;
        let description = event.description.clone().unwrap_or_default();
        if !event.lines.is_empty() {
            let mut lines = Vec::new();
            for l in &event.lines {
                let account = self
                    .id(&l.account)
                    .ok_or_else(|| not_supported("an unknown account"))?;
                lines.push(json!({"account": account, "side": l.side, "amount": l.amount}));
            }
            self.entries
                .push(json!({"date": date, "description": description, "lines": lines}));
            self.event_entries.insert(n, self.entries.len() - 1);
            return Ok(());
        }
        if let Some(of) = event.reverses {
            // GnuCash has no reversal link; a user reverses by writing the
            // opposite transaction, which is what is done here.
            let idx = *self
                .event_entries
                .get(&of)
                .ok_or_else(|| not_supported("a reversal of an event that posted nothing"))?;
            let original = self.entries[idx].clone();
            let flipped: Vec<Value> = original["lines"]
                .as_array()
                .unwrap()
                .iter()
                .map(|l| {
                    let side = if l["side"] == "debit" {
                        "credit"
                    } else {
                        "debit"
                    };
                    json!({"account": l["account"], "side": side, "amount": l["amount"]})
                })
                .collect();
            self.entries
                .push(json!({"date": date, "description": description, "lines": flipped}));
            self.event_entries.insert(n, self.entries.len() - 1);
            return Ok(());
        }
        Err(not_supported("this kind of event"))
    }

    fn query(&self, q: &Query) -> Result<Value, Refusal> {
        let as_of = q
            .as_of
            .clone()
            .ok_or_else(|| not_supported("a query without as_of"))?;
        match q.name.as_str() {
            "trial_balance" | "balances" => {
                let balances = self.balances(&as_of)?;
                let mut out = serde_json::Map::new();
                for a in &self.accounts {
                    let b = balances[&a.code];
                    let mut shown = if q.name == "balances" && !a.normal_debit {
                        -b
                    } else {
                        b
                    };
                    if shown.is_zero() {
                        shown = Decimal::ZERO;
                    }
                    if shown.scale() < 2 {
                        shown.rescale(2);
                    }
                    out.insert(a.code.clone(), Value::String(shown.to_string()));
                }
                Ok(Value::Object(out))
            }
            other => Err(not_supported(&format!("the {other} query"))),
        }
    }

    fn claims(&self) -> Claims {
        Claims::new(claimed())
    }

    fn change(&mut self, _change: &ChangeSpec) -> Result<(), Refusal> {
        Err(not_supported("a change"))
    }
}
