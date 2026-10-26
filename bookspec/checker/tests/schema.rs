//! The example schema: what a well-formed bookspec example is, and what
//! the parser refuses.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use bookspec_checker::{
    Example, Query, Refusal, TEXT_VALUED, Values, refusal_reasons, rule_for_refusal,
};

fn examples_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples")
}

/// A minimal well-formed example, and the text with one line replaced.
const BASE: &str = r#"
id = "schema.test"
title = "A test"
rule = "Every entry MUST balance."
status = "AUTHORITY"
sources = ["bookspec/README.md"]
tags = ["schema"]

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
description = "Contributed"
lines = [
  { account = "1000", side = "debit", amount = "100.00" },
  { account = "3000", side = "credit", amount = "100.00" },
]

[[expect]]
query = "trial_balance"
as_of = "2026-01-02"
[expect.values]
"1000" = "100.00"
"3000" = "-100.00"
"#;

fn base() -> Example {
    Example::from_toml(BASE).expect("the base example is well formed")
}

/// Every example of the seed parses and follows the schema: its closed
/// lists, its dates, its amounts and its references.
#[test]
fn every_seed_example_parses_and_validates() {
    let mut files: Vec<PathBuf> = std::fs::read_dir(examples_dir())
        .expect("bookspec/examples")
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "toml"))
        .collect();
    files.sort();
    assert!(files.len() >= 56, "the seed has at least 56 examples");
    let mut ids = BTreeSet::new();
    for f in &files {
        let text = std::fs::read_to_string(f).unwrap();
        let ex = Example::from_toml(&text).unwrap_or_else(|e| panic!("{}: {e}", f.display()));
        let stem = f.file_stem().unwrap().to_string_lossy();
        assert_eq!(ex.id, stem, "{}: the file is named for its id", f.display());
        assert!(ids.insert(ex.id.clone()), "{}: id used twice", ex.id);
    }
}

/// A key the schema does not have is refused, never ignored.
#[test]
fn an_unknown_key_is_refused() {
    let text = BASE.replace("cash_accounts = ", "not_a_key = ");
    let text = format!("{text}\ncash_account = []\n");
    assert!(Example::from_toml(&text).is_err());
}

/// The status is one of the three the specification uses.
#[test]
fn the_status_is_authority_convention_or_choice() {
    let text = BASE.replace(r#"status = "AUTHORITY""#, r#"status = "DECISION""#);
    let e = Example::from_toml(&text).unwrap_err();
    assert!(e.message.contains("status"), "{e}");
}

/// A line's amount is exact and positive.
#[test]
fn a_line_amount_is_exact_and_positive() {
    let text = BASE.replace(r#"amount = "100.00" }"#, r#"amount = "-100.00" }"#);
    let e = Example::from_toml(&text).unwrap_err();
    assert!(e.message.contains("amounts are positive"), "{e}");
    let text = BASE.replace(r#"amount = "100.00" }"#, r#"amount = "1e2" }"#);
    let e = Example::from_toml(&text).unwrap_err();
    assert!(e.message.contains("exact decimal"), "{e}");
}

/// An event is exactly one of the nine kinds.
#[test]
fn an_event_is_exactly_one_kind() {
    let text = BASE.replace(
        r#"lines = [
  { account = "1000", side = "debit", amount = "100.00" },
  { account = "3000", side = "credit", amount = "100.00" },
]"#,
        r#"lines = [
  { account = "1000", side = "debit", amount = "100.00" },
  { account = "3000", side = "credit", amount = "100.00" },
]
reverses = 1"#,
    );
    let e = Example::from_toml(&text).unwrap_err();
    assert!(e.message.contains("an event is one of"), "{e}");
}

/// `reverses` counts events from 1 and never names itself or a later one.
#[test]
fn reverses_counts_events_from_one() {
    let text = format!(
        "{BASE}\n[[events]]\ndate = \"2026-01-03\"\ndescription = \"Reverse it\"\nreverses = 0\n"
    );
    let e = Example::from_toml(&text).unwrap_err();
    assert!(e.message.contains("events count from 1"), "{e}");
}

/// A members close needs the shares, and they sum to 1.
#[test]
fn the_members_shares_sum_to_one() {
    let text = BASE.replace(
        r#"[[setup.accounts]]
code = "3000""#,
        r#"[[setup.members]]
name = "A"
capital_account = "3000"
share = "0.6"

[[setup.members]]
name = "B"
capital_account = "3000"
share = "0.5"

[[setup.accounts]]
code = "3000""#,
    );
    let e = Example::from_toml(&text).unwrap_err();
    assert!(e.message.contains("shares sum to 1"), "{e}");
}

/// A member's share is an exact decimal fraction above zero.
#[test]
fn a_share_is_a_fraction_above_zero() {
    let text = BASE.replace(
        r#"[[setup.accounts]]
code = "3000""#,
        r#"[[setup.members]]
name = "A"
capital_account = "3000"
share = "1.5"

[[setup.accounts]]
code = "3000""#,
    );
    let e = Example::from_toml(&text).unwrap_err();
    assert!(e.message.contains("share"), "{e}");
}

/// Every refusal reason the specification names belongs to exactly one
/// rule, and a refusal names that rule.
#[test]
fn every_refusal_reason_names_a_rule() {
    for reason in refusal_reasons() {
        let rule = rule_for_refusal(reason).unwrap_or_else(|| panic!("no rule for {reason:?}"));
        assert!(
            !rule.title.contains("allocated"),
            "{reason:?} belongs to a stated rule"
        );
        assert_eq!(Refusal::of_reason(reason).rule, rule.id);
        assert_eq!(Refusal::of_reason(reason).reason, reason);
    }
    assert!(rule_for_refusal("not_a_reason").is_none());
}

/// A query no crate answers is refused.
#[test]
fn an_unknown_query_is_refused() {
    let text = BASE.replace(r#"query = "trial_balance""#, r#"query = "trial_balances""#);
    let e = Example::from_toml(&text).unwrap_err();
    assert!(e.message.contains("no query answers"), "{e}");
}

/// A period may be refused instead of answering; another query may not.
#[test]
fn a_period_may_be_refused() {
    let text = BASE.replace(
        r#"query = "trial_balance"
as_of = "2026-01-02"
[expect.values]
"1000" = "100.00"
"3000" = "-100.00""#,
        r#"query = "trial_balance"
as_of = "2026-01-02"
refused = "end_before_start""#,
    );
    let e = Example::from_toml(&text).unwrap_err();
    assert!(e.message.contains("only a period may be refused"), "{e}");
}

/// A command is accepted or refused, never both, and shows no values.
#[test]
fn a_command_shows_no_values() {
    let text = BASE.replace(
        r#"query = "trial_balance"
as_of = "2026-01-02"
[expect.values]
"1000" = "100.00"
"3000" = "-100.00""#,
        r#"query = "rename_account"
code = "1000"
name = "Cash on hand"
accepted = true
[expect.values]
"1000" = "100.00""#,
    );
    let e = Example::from_toml(&text).unwrap_err();
    assert!(e.message.contains("a command is accepted"), "{e}");
}

/// A text-valued figure query takes words; a figure query takes exact
/// decimal strings.
#[test]
fn a_figure_query_takes_exact_decimals() {
    assert!(TEXT_VALUED.contains(&"vendor_1099"));
    let text = BASE.replace(r#""1000" = "100.00""#, r#""1000" = "100.0.0""#);
    let e = Example::from_toml(&text).unwrap_err();
    assert!(e.message.contains("exact decimal"), "{e}");
}

/// `Query` carries the question and not the answer, and knows which
/// queries list everything they cover.
#[test]
fn a_query_does_not_carry_its_expected_values() {
    let ex = base();
    let x = &ex.expect[0];
    let q: Query = x.query();
    assert_eq!(q.name, "trial_balance");
    assert!(q.is_complete());
    assert!(!q.is_command());
    assert!(!q.is_format());
    assert_eq!(
        x.values("schema.test"),
        &Values::Figures(BTreeMap::from([
            ("1000".to_string(), "100.00".to_string()),
            ("3000".to_string(), "-100.00".to_string()),
        ]))
    );
}

/// A format query belongs to the formats appendix.
#[test]
fn a_format_query_is_of_the_formats_appendix() {
    let text = BASE.replace(
        r#"query = "trial_balance"
as_of = "2026-01-02""#,
        r#"query = "iif_lines"
as_of = "2026-01-02""#,
    );
    let e = Example::from_toml(&text).unwrap_err();
    assert!(e.message.contains("iif_lines"), "{e}");
}

/// The seed's index is the table the catalogue is: B-1 to B-45, one
/// identifier each. The catalogue test reads it too; this checks the ids
/// themselves.
#[test]
fn the_rule_index_lists_every_stated_rule() {
    let index = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../spec/rules-index.md"),
    )
    .expect("bookspec/spec/rules-index.md");
    for b in 1..=45 {
        assert!(
            index.contains(&format!("| B-{b} |")),
            "the index lists B-{b}"
        );
    }
}
