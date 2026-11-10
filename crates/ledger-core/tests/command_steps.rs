//! `docs/command-steps.md` lists every command on `Ledger`, with the step
//! of bookspec's Lean model it is.
//!
//! bookspec proves its laws for every state a `Step` reaches. A command
//! that changes the books and is no step is outside those proofs, so the
//! page has to name every command, and a new one has to be placed (or
//! written down as not covered) before it ships. This test reads the
//! source and the page and fails when they differ.

use std::collections::{BTreeMap, BTreeSet};

const SOURCE: &str = include_str!("../src/ledger.rs");
const PAGE: &str = include_str!("../../../docs/command-steps.md");

/// The steps of the two models (`Bookspec.Step`, `Bookspec.Documents.Step`),
/// and `none` for a command that is no step.
const STEPS: &[&str] = &[
    "addAccount",
    "draft",
    "post",
    "reverse",
    "openPeriod",
    "close",
    "closeToRetained",
    "discard",
    "record",
    "document",
    "doc.reverse",
    "writeOff",
    "assign",
    "release",
    "markSent",
    "none",
];

/// The commands that are no step of the model. Adding one here is
/// admitting a hole in what the proofs cover: the page says what closes it.
const NO_STEP: &[&str] = &["renumber_account"];

/// Every `pub fn` of `impl Ledger` that takes `&mut self`, by name.
fn commands() -> BTreeSet<String> {
    let start = SOURCE
        .find("\nimpl Ledger {")
        .expect("ledger.rs has one `impl Ledger {`");
    // The block ends at the first line that is a lone closing brace.
    let body = &SOURCE[start + 1..];
    let end = body.find("\n}\n").expect("the impl block closes");
    let lines: Vec<&str> = body[..end].lines().collect();
    let mut found = BTreeSet::new();
    for (i, line) in lines.iter().enumerate() {
        let Some(rest) = line.strip_prefix("    pub fn ") else {
            continue;
        };
        let name: String = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        // The signature runs to the line that opens the body.
        let signature: String = lines[i..]
            .iter()
            .take_while(|l| !l.trim_end().ends_with('{'))
            .chain(lines[i..].iter().find(|l| l.trim_end().ends_with('{')))
            .copied()
            .collect::<Vec<_>>()
            .join(" ");
        if signature.contains("&mut self") {
            found.insert(name);
        }
    }
    found
}

/// The page's table: command to its steps.
fn rows() -> BTreeMap<String, Vec<String>> {
    let table = PAGE
        .split("## Every command")
        .nth(1)
        .expect("the page has the section `## Every command`");
    let mut rows = BTreeMap::new();
    for line in table.lines() {
        let cells: Vec<&str> = line.split('|').map(str::trim).collect();
        // `| `name` | steps | note |` splits into ["", name, steps, note, ""].
        if cells.len() < 4 || !cells[1].starts_with('`') {
            continue;
        }
        let name = cells[1].trim_matches('`').to_string();
        let steps = cells[2].split(',').map(|s| s.trim().to_string()).collect();
        assert!(
            rows.insert(name.clone(), steps).is_none(),
            "docs/command-steps.md lists `{name}` twice"
        );
    }
    rows
}

/// Every command has a row, and every row a command.
#[test]
fn every_command_is_on_the_page() {
    let commands = commands();
    assert!(
        commands.len() > 100,
        "found only {} commands: the reader of ledger.rs is broken",
        commands.len()
    );
    let listed: BTreeSet<String> = rows().into_keys().collect();
    let missing: Vec<_> = commands.difference(&listed).collect();
    let stale: Vec<_> = listed.difference(&commands).collect();
    assert!(
        missing.is_empty() && stale.is_empty(),
        "docs/command-steps.md and `impl Ledger` differ.\n\
         Commands with no row (add one, with the model's step it is, or a \
         new `Step` in bookspec): {missing:?}\n\
         Rows with no command (remove them): {stale:?}"
    );
}

/// Every row names steps the model has.
#[test]
fn every_row_names_a_step_of_the_model() {
    for (command, steps) in rows() {
        assert!(!steps.is_empty(), "`{command}` names no step");
        for step in &steps {
            assert!(
                STEPS.contains(&step.as_str()),
                "`{command}` names `{step}`, which is no step of the model"
            );
        }
    }
}

/// The commands that are no step are the ones written down, and no others.
#[test]
fn the_commands_outside_the_model_are_the_ones_named() {
    let outside: BTreeSet<String> = rows()
        .into_iter()
        .filter(|(_, steps)| steps.iter().any(|s| s == "none"))
        .map(|(command, _)| command)
        .collect();
    let named: BTreeSet<String> = NO_STEP.iter().map(|s| s.to_string()).collect();
    assert_eq!(
        outside, named,
        "a command marked `none` on the page must be in NO_STEP, and the \
         page's \"No step\" section must say what closes it"
    );
    for (command, steps) in rows() {
        assert!(
            steps.len() == 1 || !steps.iter().any(|s| s == "none"),
            "`{command}` is `none` and a step at once"
        );
    }
}
