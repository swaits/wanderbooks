//! The specification's own checker (`plan/served.md` P13 step 5):
//! every example's expected figures derived from its rule's symbolic
//! form, by an engine-free reference model with exact decimals.
//!
//! The runner asks an *engine* whether it agrees with an example. This
//! module asks whether the *example* agrees with the specification: it
//! replays the example's events on the smallest ledger the rules of
//! `bookspec/spec/*.md` describe (entries, reversals, a close, a bill, an
//! invoice) and computes each expectation it can from the symbolic forms
//! (`sum` of signed values, normal balances, the accounting equation,
//! net income). An expectation whose figure disagrees with the
//! derivation, or an event whose refusal the rules do not justify, fails
//! the specification's build, before any engine is consulted.
//!
//! What the model does not cover (depreciation, schedules, members,
//! 1099s, formats, the cash-flow sections) is not derived here: an
//! expectation is [`Derived::Skipped`] with the reason, never silently
//! passed, and the example is still checked by the engines.

use std::collections::BTreeMap;
use std::str::FromStr;

use chrono::NaiveDate;
use rust_decimal::Decimal;

use crate::engine::RuleId;
use crate::rules::Area;
use crate::schema::{Event, Example, Expect, Values};

/// What the derivation found of one expectation or event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Derived {
    /// The specification derives the example's figures, and they agree.
    Agrees {
        /// The rule whose symbolic form derived them.
        rule: RuleId,
    },
    /// The specification derives different figures, or does not justify
    /// a refusal: the example is wrong.
    Disagrees {
        /// The rule whose symbolic form was applied.
        rule: RuleId,
        /// What differs, in words.
        why: String,
    },
    /// The reference model does not cover it.
    Skipped {
        /// Why not.
        why: String,
    },
}

/// The derivation of one example: one result per event that was refused
/// and per expectation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Derivation {
    /// The example's id.
    pub example: String,
    /// What was derived, in the order met.
    pub results: Vec<(String, Derived)>,
}

impl Derivation {
    /// The disagreements, each as a sentence.
    pub fn disagreements(&self) -> Vec<String> {
        self.results
            .iter()
            .filter_map(|(what, d)| match d {
                Derived::Disagrees { rule, why } => {
                    Some(format!("{}: {what}: {why} ({rule})", self.example))
                }
                _ => None,
            })
            .collect()
    }

    /// How many results agree.
    pub fn agreed(&self) -> usize {
        self.results
            .iter()
            .filter(|(_, d)| matches!(d, Derived::Agrees { .. }))
            .count()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Side {
    Debit,
    Credit,
}

#[derive(Clone, Debug)]
struct Line {
    account: String,
    side: Side,
    amount: Decimal,
}

#[derive(Clone, Debug)]
struct Posted {
    date: NaiveDate,
    lines: Vec<Line>,
    closing: bool,
    reversed: bool,
}

/// The reference model: posted entries, the closed periods, the types.
struct Model {
    types: BTreeMap<String, String>,
    entries: Vec<Posted>,
    /// Event number (from 1) to the entry it posted.
    event_entry: BTreeMap<usize, usize>,
    closed_through: Vec<NaiveDate>,
    minor_digits: u32,
}

fn sign(side: Side) -> Decimal {
    match side {
        Side::Debit => Decimal::ONE,
        Side::Credit => -Decimal::ONE,
    }
}

fn dec(s: &str) -> Option<Decimal> {
    Decimal::from_str(s).ok()
}

fn date(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
}

fn rule(area: Area, n: u32) -> RuleId {
    RuleId::new(area, n)
}

/// The accounts the books create for a document or a close, with their
/// types (rules.md B-23's default roles).
fn role_type(code: &str) -> Option<&'static str> {
    match code {
        "1200" => Some("asset"),
        "2000" => Some("liability"),
        "3999" => Some("equity"),
        "4000" => Some("revenue"),
        "6999" => Some("expense"),
        _ => None,
    }
}

impl Model {
    fn new(ex: &Example) -> Self {
        let mut types: BTreeMap<String, String> = ex
            .setup
            .accounts
            .iter()
            .map(|a| (a.code.clone(), a.kind.clone()))
            .collect();
        for code in ["1200", "2000", "3999", "4000", "6999"] {
            types
                .entry(code.to_string())
                .or_insert_with(|| role_type(code).unwrap().to_string());
        }
        // ISO 4217: the currency's minor-unit digits (two unless listed).
        let minor_digits = match ex.setup.currency.as_str() {
            "JPY" | "KRW" | "VND" | "CLP" | "ISK" => 0,
            "BHD" | "KWD" | "OMR" | "JOD" => 3,
            _ => 2,
        };
        Self {
            types,
            entries: Vec::new(),
            event_entry: BTreeMap::new(),
            closed_through: Vec::new(),
            minor_digits,
        }
    }

    /// Signed sum of an account's lines dated on or before `through`
    /// (debit positive), counting entries that stand as written: the
    /// reversal is its own entry, so the original is always counted.
    fn signed(
        &self,
        code: &str,
        from: Option<NaiveDate>,
        through: NaiveDate,
        closings: bool,
    ) -> Decimal {
        self.entries
            .iter()
            .filter(|e| {
                e.date <= through && from.is_none_or(|f| e.date >= f) && (closings || !e.closing)
            })
            .flat_map(|e| e.lines.iter())
            .filter(|l| l.account == code)
            .map(|l| sign(l.side) * l.amount)
            .sum()
    }

    fn is_debit_normal(&self, code: &str) -> bool {
        matches!(
            self.types.get(code).map(String::as_str),
            Some("asset") | Some("expense")
        )
    }

    fn on_normal_side(&self, code: &str, signed: Decimal) -> Decimal {
        if self.is_debit_normal(code) {
            signed
        } else {
            -signed
        }
    }

    /// Why the rules refuse this entry, if they do: (reason, rule).
    fn refusal_justified(&self, e: &Event, reason: &str) -> Result<RuleId, String> {
        let on = |side: &str| -> Vec<Decimal> {
            e.lines
                .iter()
                .filter(|l| l.side == side)
                .filter_map(|l| dec(&l.amount))
                .collect()
        };
        let d: Decimal = on("debit").into_iter().sum();
        let c: Decimal = on("credit").into_iter().sum();
        let amounts: Vec<Decimal> = e.lines.iter().filter_map(|l| dec(&l.amount)).collect();
        match reason {
            "unbalanced" if d != c => Ok(rule(Area::Entry, 1)),
            "unbalanced" => Err(format!("debits {d} equal credits {c}")),
            "too_few_lines" if e.lines.len() < 2 => Ok(rule(Area::Entry, 1)),
            "negative_amount" if amounts.iter().any(|a| *a < Decimal::ZERO) => {
                Ok(rule(Area::Entry, 2))
            }
            "zero_amount" if amounts.iter().any(|a| a.is_zero()) => Ok(rule(Area::Entry, 2)),
            "finer_than_minor_unit"
                if amounts
                    .iter()
                    .any(|a| a.normalize().scale() > self.minor_digits) =>
            {
                Ok(rule(Area::Ledger, 1))
            }
            "period_closed" => {
                let day = e.date.as_deref().and_then(date).ok_or("no date")?;
                if self.closed_through.iter().any(|end| day <= *end) {
                    Ok(rule(Area::Period, 2))
                } else {
                    Err(format!("{day} is in no closed period"))
                }
            }
            "already_reversed" => {
                let of = e.reverses.ok_or("no reversal named")?;
                let idx = self.event_entry.get(&of).ok_or("names no entry")?;
                if self.entries[*idx].reversed {
                    Ok(rule(Area::Ledger, 6))
                } else {
                    Err(format!("event {of} was not yet reversed"))
                }
            }
            other => Err(format!("{other:?} is not derived by the reference model")),
        }
    }

    /// Post an accepted event; `Err` says what the model cannot post.
    fn post(&mut self, n: usize, e: &Event) -> Result<(), String> {
        let day = e.date.as_deref().and_then(date);
        let lines = |ls: &[crate::schema::LineSpec]| -> Option<Vec<Line>> {
            ls.iter()
                .map(|l| {
                    Some(Line {
                        account: l.account.clone(),
                        side: if l.side == "debit" {
                            Side::Debit
                        } else {
                            Side::Credit
                        },
                        amount: dec(&l.amount)?,
                    })
                })
                .collect()
        };
        let posted = if !e.lines.is_empty() {
            Posted {
                date: day.ok_or("undated")?,
                lines: lines(&e.lines).ok_or("an unreadable amount")?,
                closing: false,
                reversed: false,
            }
        } else if let Some(of) = e.reverses {
            let idx = *self
                .event_entry
                .get(&of)
                .ok_or("reverses an event that posted nothing")?;
            let original = self.entries[idx].clone();
            self.entries[idx].reversed = true;
            Posted {
                date: day.ok_or("undated")?,
                lines: original
                    .lines
                    .iter()
                    .map(|l| Line {
                        account: l.account.clone(),
                        side: match l.side {
                            Side::Debit => Side::Credit,
                            Side::Credit => Side::Debit,
                        },
                        amount: l.amount,
                    })
                    .collect(),
                closing: false,
                reversed: false,
            }
        } else if let Some(b) = &e.bill {
            let total = document_total(
                b.amount.as_deref(),
                &b.lines.iter().map(|l| l.amount.clone()).collect::<Vec<_>>(),
            )?;
            if b.lines.iter().any(|l| l.project.is_some()) {
                return Err("project tags are not modelled".into());
            }
            Posted {
                date: day.ok_or("undated")?,
                lines: vec![
                    Line {
                        account: "6999".into(),
                        side: Side::Debit,
                        amount: total,
                    },
                    Line {
                        account: "2000".into(),
                        side: Side::Credit,
                        amount: total,
                    },
                ],
                closing: false,
                reversed: false,
            }
        } else if let Some(i) = &e.invoice {
            let total = document_total(
                i.amount.as_deref(),
                &i.lines.iter().map(|l| l.amount.clone()).collect::<Vec<_>>(),
            )?;
            if i.lines.iter().any(|l| l.project.is_some()) {
                return Err("project tags are not modelled".into());
            }
            Posted {
                date: day.ok_or("undated")?,
                lines: vec![
                    Line {
                        account: "1200".into(),
                        side: Side::Debit,
                        amount: total,
                    },
                    Line {
                        account: "4000".into(),
                        side: Side::Credit,
                        amount: total,
                    },
                ],
                closing: false,
                reversed: false,
            }
        } else if let Some(c) = &e.close {
            if c.to != "retained_earnings" {
                return Err("a close to the members is not modelled".into());
            }
            let end = day.ok_or("undated")?;
            let mut out = Vec::new();
            let mut net = Decimal::ZERO;
            let codes: Vec<String> = self.types.keys().cloned().collect();
            for code in codes {
                let ty = self.types[&code].as_str();
                if ty != "revenue" && ty != "expense" {
                    continue;
                }
                let s = self.signed(&code, None, end, true);
                if s.is_zero() {
                    continue;
                }
                net -= s; // net income is credit-normal: revenue - expense
                out.push(Line {
                    account: code,
                    side: if s > Decimal::ZERO {
                        Side::Credit
                    } else {
                        Side::Debit
                    },
                    amount: s.abs(),
                });
            }
            if !net.is_zero() {
                out.push(Line {
                    account: "3999".into(),
                    side: if net > Decimal::ZERO {
                        Side::Credit
                    } else {
                        Side::Debit
                    },
                    amount: net.abs(),
                });
            }
            self.closed_through.push(end);
            Posted {
                date: end,
                lines: out,
                closing: true,
                reversed: false,
            }
        } else {
            return Err("an event the reference model does not post".into());
        };
        // The symbolic form of BS-entry-1: what is posted balances.
        let (d, c): (Decimal, Decimal) =
            posted
                .lines
                .iter()
                .fold((Decimal::ZERO, Decimal::ZERO), |(d, c), l| match l.side {
                    Side::Debit => (d + l.amount, c),
                    Side::Credit => (d, c + l.amount),
                });
        if d != c {
            return Err(format!(
                "the example posts an unbalanced entry (debits {d}, credits {c})"
            ));
        }
        self.entries.push(posted);
        self.event_entry.insert(n, self.entries.len() - 1);
        Ok(())
    }
}

fn document_total(amount: Option<&str>, lines: &[String]) -> Result<Decimal, String> {
    match amount {
        Some(a) => dec(a).ok_or_else(|| "an unreadable amount".into()),
        None => lines
            .iter()
            .map(|l| dec(l).ok_or_else(|| "an unreadable amount".to_string()))
            .sum(),
    }
}

/// Derive what the specification says about `ex`.
pub fn derive(ex: &Example) -> Derivation {
    let mut results = Vec::new();
    let mut model = Model::new(ex);
    // The expectations asked after event N are checked right after it; the
    // rest at the end. The model keeps one state per point, so replay.
    let mut usable = true;
    let mut why_not = String::new();
    let unsupported_setup = !ex.setup.assets.is_empty()
        || !ex.setup.schedules.is_empty()
        || !ex.setup.fixed_assets.is_empty()
        || !ex.setup.members.is_empty();
    if unsupported_setup {
        usable = false;
        why_not = "the setup holds assets, schedules or members, which the reference model does not carry".into();
    }
    for (i, e) in ex.events.iter().enumerate() {
        let n = i + 1;
        if usable {
            match &e.refused {
                Some(reason) => match model.refusal_justified(e, reason) {
                    Ok(rule) => results.push((
                        format!("event {n} refused {reason}"),
                        Derived::Agrees { rule },
                    )),
                    Err(why) => results.push((
                        format!("event {n} refused {reason}"),
                        if why.contains("not derived") {
                            Derived::Skipped { why }
                        } else {
                            Derived::Disagrees {
                                rule: rule(Area::Entry, 1),
                                why,
                            }
                        },
                    )),
                },
                None => {
                    if let Err(why) = model.post(n, e) {
                        usable = false;
                        why_not = format!("event {n}: {why}");
                    }
                }
            }
        }
        for x in ex.expect.iter().filter(|x| x.after == Some(n)) {
            results.push(expectation(&model, x, usable, &why_not));
        }
    }
    for x in ex.expect.iter().filter(|x| x.after.is_none()) {
        results.push(expectation(&model, x, usable, &why_not));
    }
    Derivation {
        example: ex.id.clone(),
        results,
    }
}

fn expectation(model: &Model, x: &Expect, usable: bool, why_not: &str) -> (String, Derived) {
    let what = format!(
        "{} {}",
        x.query,
        x.as_of.as_deref().or(x.to.as_deref()).unwrap_or("")
    );
    if !usable {
        return (
            what,
            Derived::Skipped {
                why: why_not.to_string(),
            },
        );
    }
    let Some(Values::Figures(want)) = &x.values else {
        return (
            what,
            Derived::Skipped {
                why: "no figures to derive".into(),
            },
        );
    };
    let derived = match x.query.as_str() {
        "trial_balance" => x.as_of.as_deref().and_then(date).map(|d| {
            (
                rule(Area::Statement, 1),
                want.keys()
                    .map(|k| (k.clone(), model.signed(k, None, d, true)))
                    .collect::<Vec<_>>(),
            )
        }),
        "balances" => x.as_of.as_deref().and_then(date).map(|d| {
            (
                rule(Area::Ledger, 3),
                want.keys()
                    .map(|k| {
                        (
                            k.clone(),
                            model.on_normal_side(k, model.signed(k, None, d, true)),
                        )
                    })
                    .collect(),
            )
        }),
        "profit_and_loss" => {
            let from = x.from.as_deref().and_then(date);
            let to = x.to.as_deref().and_then(date);
            from.zip(to).map(|(f, t)| {
                let mut figs = Vec::new();
                for k in want.keys() {
                    if k == "net_income" {
                        let mut net = Decimal::ZERO;
                        for (code, ty) in &model.types {
                            if ty == "revenue" || ty == "expense" {
                                net += model
                                    .on_normal_side(code, model.signed(code, Some(f), t, false))
                                    * if ty == "revenue" {
                                        Decimal::ONE
                                    } else {
                                        -Decimal::ONE
                                    };
                            }
                        }
                        figs.push((k.clone(), net));
                    } else {
                        figs.push((
                            k.clone(),
                            model.on_normal_side(k, model.signed(k, Some(f), t, false)),
                        ));
                    }
                }
                (rule(Area::Statement, 3), figs)
            })
        }
        "entry" => x.entry.and_then(|n| model.event_entry.get(&n)).map(|idx| {
            let e = &model.entries[*idx];
            let figs = want
                .keys()
                .map(|k| {
                    let (code, side) = k.split_once('.').unwrap_or((k, ""));
                    let v: Decimal = e
                        .lines
                        .iter()
                        .filter(|l| {
                            l.account == code
                                && matches!(
                                    (side, l.side),
                                    ("debit", Side::Debit) | ("credit", Side::Credit)
                                )
                        })
                        .map(|l| l.amount)
                        .sum();
                    (k.clone(), v)
                })
                .collect();
            (rule(Area::Ledger, 4), figs)
        }),
        other => {
            return (
                what,
                Derived::Skipped {
                    why: format!("{other} is not derived by the reference model"),
                },
            );
        }
    };
    let Some((rule, figs)) = derived else {
        return (
            what,
            Derived::Skipped {
                why: "an undated or unresolved query".into(),
            },
        );
    };
    for (k, got) in figs {
        match want.get(&k).and_then(|w| dec(w)) {
            Some(w) if w == got => {}
            Some(w) => {
                return (
                    what,
                    Derived::Disagrees {
                        rule,
                        why: format!("{k}: the example says {w}, the rule derives {got}"),
                    },
                );
            }
            None => {
                return (
                    what,
                    Derived::Disagrees {
                        rule,
                        why: format!("{k}: no exact figure"),
                    },
                );
            }
        }
    }
    // The laws on the whole book, at the day asked: BS-statement-2.
    if let (true, Some(d)) = (
        matches!(x.query.as_str(), "trial_balance" | "balances"),
        x.as_of.as_deref().and_then(date),
    ) {
        let (mut a, mut l, mut e, mut r, mut xp) = (
            Decimal::ZERO,
            Decimal::ZERO,
            Decimal::ZERO,
            Decimal::ZERO,
            Decimal::ZERO,
        );
        let mut net = Decimal::ZERO;
        for (code, ty) in &model.types {
            let s = model.signed(code, None, d, true);
            net += s;
            let n = model.on_normal_side(code, s);
            match ty.as_str() {
                "asset" => a += n,
                "liability" => l += n,
                "equity" => e += n,
                "revenue" => r += n,
                "expense" => xp += n,
                _ => {}
            }
        }
        if !net.is_zero() {
            return (
                what,
                Derived::Disagrees {
                    rule: self::rule(Area::Statement, 1),
                    why: format!("the books do not net to zero ({net})"),
                },
            );
        }
        if a != l + e + (r - xp) {
            return (
                what,
                Derived::Disagrees {
                    rule: self::rule(Area::Statement, 2),
                    why: format!("A {a} != L {l} + E {e} + (R {r} - X {xp})"),
                },
            );
        }
    }
    (what, Derived::Agrees { rule })
}
