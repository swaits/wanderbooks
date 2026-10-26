//! The runner's own tests, with an engine that only pretends: the runner
//! must judge the answer, never the engine's belief about it.

use bookspec_checker::{
    Area, ChangeSpec, Claims, Engine, Event, Example, Query, Refusal, Rule, RuleId, Setup, Status,
};
use bookspec_runner::Outcome;

const EXAMPLE: &str = r#"
id = "fake.example"
title = "A fake engine answers"
rule = "The chart's names MUST be the account_names the example expects."
status = "AUTHORITY"
sources = ["bookspec/README.md"]
tags = ["runner"]

events = []

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

[[expect]]
query = "account_names"

[expect.values]
"1000" = "Cash"
"3000" = "Capital"
"#;

fn example() -> Example {
    Example::from_toml(EXAMPLE).expect("the fake example is well formed")
}

fn catalog() -> Vec<Rule> {
    vec![Rule {
        b: 1,
        id: RuleId::new(Area::Entry, 1),
        area: Area::Entry,
        title: "Every entry is a double entry: debits equal credits",
        status: Some(Status::Authority),
        examples: &["fake.example"],
    }]
}

/// The rules an engine that claims the catalogue's example states.
fn claim() -> Claims {
    Claims::new([RuleId::new(Area::Entry, 1)])
}

struct Fake {
    claims: Claims,
    names: serde_json::Value,
    accept: bool,
    refuse_query: Option<Refusal>,
    change_ok: bool,
}

impl Default for Fake {
    fn default() -> Self {
        Self {
            claims: claim(),
            names: serde_json::json!({"1000": "Cash", "3000": "Capital"}),
            accept: true,
            refuse_query: None,
            change_ok: true,
        }
    }
}

impl Engine for Fake {
    fn load(&mut self, _setup: &Setup) -> Result<(), Refusal> {
        Ok(())
    }

    fn apply(&mut self, event: &Event) -> Result<(), Refusal> {
        if let Some(reason) = &event.refused {
            return Err(Refusal::of_reason(reason));
        }
        if self.accept {
            Ok(())
        } else {
            Err(Refusal::of_reason("entry_irreversible"))
        }
    }

    fn query(&self, q: &Query) -> Result<serde_json::Value, Refusal> {
        if let Some(r) = &self.refuse_query {
            return Err(r.clone());
        }
        match q.name.as_str() {
            "account_names" => Ok(self.names.clone()),
            other => Err(Refusal::new(
                RuleId::new(Area::Entry, 1),
                format!("the fake engine answers no {other}"),
            )),
        }
    }

    fn claims(&self) -> Claims {
        self.claims.clone()
    }

    fn change(&mut self, _change: &ChangeSpec) -> Result<(), Refusal> {
        if self.change_ok {
            Ok(())
        } else {
            Err(Refusal::of_reason("unchanged"))
        }
    }
}

/// An engine that answers what the example expects passes, and its rule is
/// claimed and passed.
#[test]
fn a_passing_engine_passes() {
    let examples = [example()];
    let report = bookspec_runner::run(&examples, &catalog(), |_| Fake::default());
    assert!(report.passed(), "{:?}", report.failures());
    assert_eq!(
        report.rules.get(&RuleId::new(Area::Entry, 1)),
        Some(&Outcome::Pass)
    );
}

/// The runner judges the answer: a wrong figure fails the rule, whatever
/// the engine says about itself.
#[test]
fn a_wrong_answer_fails() {
    let examples = [example()];
    let report = bookspec_runner::run(&examples, &catalog(), |_| Fake {
        names: serde_json::json!({"1000": "Coin", "3000": "Capital"}),
        ..Fake::default()
    });
    assert!(!report.passed());
    assert_eq!(
        report.rules.get(&RuleId::new(Area::Entry, 1)),
        Some(&Outcome::Fail)
    );
    assert!(
        report.failures().iter().any(|f| f.contains("1000")),
        "{:?}",
        report.failures()
    );
}

/// An engine that claims nothing is not claimed, not failed; and the
/// example it does not claim is still run (its answer is compared).
#[test]
fn an_engine_that_claims_nothing_is_not_claimed() {
    let examples = [example()];
    let report = bookspec_runner::run(&examples, &catalog(), |_| Fake {
        claims: Claims::default(),
        ..Fake::default()
    });
    assert!(report.passed(), "{:?}", report.failures());
    assert_eq!(
        report.rules.get(&RuleId::new(Area::Entry, 1)),
        Some(&Outcome::NotClaimed)
    );
}

/// A claim with no example behind it is a failure: a claim needs evidence.
#[test]
fn a_claim_without_an_example_fails() {
    let mut catalog = catalog();
    catalog[0].examples = &[];
    let examples = [example()];
    let report = bookspec_runner::run(&examples, &catalog, |_| Fake::default());
    assert!(!report.passed());
    assert_eq!(
        report.rules.get(&RuleId::new(Area::Entry, 1)),
        Some(&Outcome::Fail)
    );
}

/// A query the engine refuses fails, and says which rule it refused under.
#[test]
fn a_refused_query_fails() {
    let examples = [example()];
    let report = bookspec_runner::run(&examples, &catalog(), |_| Fake {
        refuse_query: Some(Refusal::new(
            RuleId::new(Area::Ledger, 1),
            "not implemented",
        )),
        ..Fake::default()
    });
    assert!(!report.passed());
    let failures = report.failures().join("\n");
    assert!(failures.contains("not implemented"), "{failures}");
    assert!(failures.contains("BS-ledger-1"), "{failures}");
}

/// An example no rule lists is a failure: it proves nothing.
#[test]
fn an_example_listed_under_no_rule_fails() {
    let examples = [example()];
    let report = bookspec_runner::run(&examples, &[], |_| Fake::default());
    assert!(!report.passed());
    assert!(
        report
            .failures()
            .iter()
            .any(|f| f.contains("listed under no rule")),
        "{:?}",
        report.failures()
    );
}

/// A change the engine refuses fails the example.
#[test]
fn a_refused_change_fails() {
    let text = EXAMPLE.replace("events = []\n", "").replace(
        "[[expect]]",
        "[[events]]\ndate = \"2026-01-02\"\ndescription = \"A\"\nlines = [\n  { account = \"1000\", side = \"debit\", amount = \"1.00\" },\n  { account = \"3000\", side = \"credit\", amount = \"1.00\" },\n]\n\n[[changes]]\nafter = 1\nrequire_tax_category = \"refuse\"\n\n[[expect]]",
    );
    let examples = [Example::from_toml(&text).unwrap()];
    let report = bookspec_runner::run(&examples, &catalog(), |_| Fake {
        change_ok: false,
        ..Fake::default()
    });
    assert!(!report.passed());
    assert!(
        report.failures().iter().any(|f| f.contains("unchanged")),
        "{:?}",
        report.failures()
    );
}
