//! bookspec's conformance runner: it drives any [`Engine`] through the
//! examples and reports, per rule, pass, fail or not claimed
//! (`plan/served.md` 8.2 P13 step 2).
//!
//! The runner is the only judge. An engine answers; the runner compares the
//! answer with the example's expected values, and with the refusal the
//! example names when the engine refuses. The engine is never asked
//! whether its own answer is right.

use std::collections::BTreeMap;

use bookspec_checker::{Claims, Engine, Example, Expect, Query, Rule, RuleId, Values, rules};

/// What the runner says about one rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The engine claims the rule and every example that demonstrates it
    /// passed.
    Pass,
    /// The engine claims the rule and an example failed, or a claim has no
    /// example behind it.
    Fail,
    /// The engine does not claim the rule.
    NotClaimed,
}

impl Outcome {
    /// The outcome as the words a report prints.
    pub fn key(self) -> &'static str {
        match self {
            Outcome::Pass => "pass",
            Outcome::Fail => "fail",
            Outcome::NotClaimed => "not claimed",
        }
    }
}

/// One example's outcome.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExampleReport {
    /// The example's id.
    pub id: String,
    /// Why it failed, if it did.
    pub failure: Option<String>,
}

/// The runner's report: what it found of each rule, and each example.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// Every rule of the catalogue the runner was given, and its outcome.
    pub rules: BTreeMap<RuleId, Outcome>,
    /// Every example, in the order run.
    pub examples: Vec<ExampleReport>,
    /// Failures that belong to no rule: an example no rule lists, a rule
    /// claimed but unknown to the catalogue.
    pub unclaimed: Vec<String>,
}

impl Report {
    /// True when every claimed rule passed and nothing else failed.
    pub fn passed(&self) -> bool {
        self.unclaimed.is_empty()
            && self.rules.values().all(|o| *o != Outcome::Fail)
            && self.examples.iter().all(|e| e.failure.is_none())
    }

    /// Every failure, with its rule's outcome where it has one.
    pub fn failures(&self) -> Vec<String> {
        let mut out = self.unclaimed.clone();
        for (rule, outcome) in &self.rules {
            if *outcome == Outcome::Fail {
                let ids: Vec<&str> = rules()
                    .iter()
                    .find(|r| &r.id == rule)
                    .map(|r| r.examples.to_vec())
                    .unwrap_or_default();
                out.push(format!("{rule} failed (examples: {})", ids.join(", ")));
            }
        }
        for e in &self.examples {
            if let Some(f) = &e.failure {
                out.push(f.clone());
            }
        }
        out
    }
}

/// Run `examples` against engines made by `make`, one fresh engine per
/// example, and report per rule against `catalog` (normally
/// [`bookspec_checker::rules()`]).
pub fn run<'a, E>(
    examples: &'a [Example],
    catalog: &[Rule],
    mut make: impl FnMut(&'a Example) -> E,
) -> Report
where
    E: Engine + 'a,
{
    let mut out = Report::default();
    let mut claims = Claims::default();
    for ex in examples {
        let mut engine = make(ex);
        let declared = engine.claims();
        claims.rules.extend(declared.rules);
        for (rule, choice) in declared.choices {
            claims.choices.entry(rule).or_insert(choice);
        }
        let failure = run_example(ex, &mut engine).err();
        out.examples.push(ExampleReport {
            id: ex.id.clone(),
            failure,
        });
    }
    for rule in catalog {
        let outcome = if !claims.claims(&rule.id) {
            Outcome::NotClaimed
        } else if !rule.has_examples() {
            out.unclaimed
                .push(format!("{} is claimed but has no example", rule.id));
            Outcome::Fail
        } else {
            let failed = rule.examples.iter().any(|id| {
                out.examples
                    .iter()
                    .find(|e| &e.id == id)
                    .is_none_or(|e| e.failure.is_some())
            });
            if failed { Outcome::Fail } else { Outcome::Pass }
        };
        out.rules.insert(rule.id.clone(), outcome);
    }
    for rule in &claims.rules {
        if !catalog.iter().any(|r| &r.id == rule) {
            out.unclaimed
                .push(format!("{rule} is claimed but is not in the catalogue"));
        }
    }
    for ex in examples {
        if !catalog.iter().any(|r| r.examples.contains(&ex.id.as_str())) {
            out.unclaimed
                .push(format!("example {} is listed under no rule", ex.id));
        }
    }
    out
}

/// Run one example against one engine: load the setup, apply every event
/// (accepted or refused as the example says), check the expectations named
/// after each event, make the changes named after it, then check the rest.
pub fn run_example(ex: &Example, engine: &mut dyn Engine) -> Result<(), String> {
    engine
        .load(&ex.setup)
        .map_err(|r| format!("load refused: {} ({})", r.reason, r.rule))?;
    for (i, event) in ex.events.iter().enumerate() {
        let n = i + 1;
        match (engine.apply(event), &event.refused) {
            (Ok(()), None) => {}
            (Err(r), Some(want)) if &r.reason == want => {}
            (Err(r), Some(want)) => {
                return Err(format!(
                    "event {n}: refused {:?}, expected {want:?} ({})",
                    r.reason, r.rule
                ));
            }
            (Err(r), None) => {
                return Err(format!(
                    "event {n}: refused {:?}, expected accepted ({})",
                    r.reason, r.rule
                ));
            }
            (Ok(()), Some(want)) => {
                return Err(format!("event {n}: accepted, expected refused: {want}"));
            }
        }
        for x in ex.expect.iter().filter(|x| x.after == Some(n)) {
            check(engine, x).map_err(|m| format!("after event {n}: {m}"))?;
        }
        for c in ex.changes.iter().filter(|c| c.after == n) {
            engine
                .change(c)
                .map_err(|r| format!("change after {n}: {} ({})", r.reason, r.rule))?;
        }
    }
    for x in ex.expect.iter().filter(|x| x.after.is_none()) {
        check(engine, x)?;
    }
    Ok(())
}

/// Check one expectation against the engine's answer.
fn check(engine: &dyn Engine, x: &Expect) -> Result<(), String> {
    let q = x.query();
    let answer = engine.query(&q);
    match (answer, &x.refused, x.accepted) {
        (Ok(v), None, Some(true)) => {
            if v == serde_json::Value::Bool(true) {
                Ok(())
            } else {
                Err(format!(
                    "{}: accepted, but answered {v} (a command shows no values)",
                    q.name
                ))
            }
        }
        (Ok(v), None, None) => compare(x, &v),
        (Ok(v), Some(want), _) => Err(format!(
            "{}: answered {v}, expected refused: {want}",
            q.name
        )),
        (Err(r), Some(want), _) if &r.reason == want => Ok(()),
        (Err(r), Some(want), _) => Err(format!(
            "{}: refused {:?}, expected {want:?} ({})",
            q.name, r.reason, r.rule
        )),
        (Err(r), None, _) => Err(format!("{}: refused {:?} ({})", q.name, r.reason, r.rule)),
        (Ok(v), None, Some(false)) => Err(format!(
            "{}: accepted = false is not the examples' vocabulary; answered {v}",
            q.name
        )),
    }
}

/// Compare an answer with what the example expects.
fn compare(x: &Expect, got: &serde_json::Value) -> Result<(), String> {
    let q: Query = x.query();
    let Some(want) = &x.values else {
        return Err(format!("{}: expects values, none given", q.name));
    };
    match want {
        Values::Figures(figures) => {
            let serde_json::Value::Object(map) = got else {
                return Err(format!("{}: answered {got}, expected a table", q.name));
            };
            for (key, want) in figures {
                match map.get(key) {
                    Some(serde_json::Value::String(got)) if got == want => {}
                    Some(got) => {
                        return Err(format!("{}: {key}: got {got}, expected {want:?}", q.name));
                    }
                    None => {
                        return Err(format!("{}: {key} is missing (expected {want:?})", q.name));
                    }
                }
            }
            if q.is_complete() {
                let extra: Vec<&String> =
                    map.keys().filter(|k| !figures.contains_key(*k)).collect();
                let missing: Vec<&String> =
                    figures.keys().filter(|k| !map.contains_key(*k)).collect();
                if !extra.is_empty() || !missing.is_empty() {
                    return Err(format!(
                        "{}: lists exactly what it covers; missing {missing:?}, extra {extra:?}",
                        q.name
                    ));
                }
            }
            Ok(())
        }
        Values::Lines(lines) => {
            let serde_json::Value::Array(got) = got else {
                return Err(format!("{}: answered {got}, expected a list", q.name));
            };
            let got: Vec<&str> = got
                .iter()
                .map(|v| v.as_str().unwrap_or("<not text>"))
                .collect();
            let want: Vec<&str> = lines.iter().map(String::as_str).collect();
            if got == want {
                Ok(())
            } else {
                Err(format!("{}: got {got:?}, expected {want:?}", q.name))
            }
        }
        Values::Rows(rows) => {
            let serde_json::Value::Array(got) = got else {
                return Err(format!("{}: answered {got}, expected rows", q.name));
            };
            let got: Vec<Vec<&str>> = got
                .iter()
                .map(|row| {
                    row.as_array()
                        .map(|r| {
                            r.iter()
                                .map(|v| v.as_str().unwrap_or("<not text>"))
                                .collect()
                        })
                        .unwrap_or_default()
                })
                .collect();
            let want: Vec<Vec<&str>> = rows
                .iter()
                .map(|r| r.iter().map(String::as_str).collect())
                .collect();
            if got == want {
                Ok(())
            } else {
                Err(format!("{}: got {got:?}, expected {want:?}", q.name))
            }
        }
    }
}
