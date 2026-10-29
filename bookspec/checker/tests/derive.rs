//! The specification's own checker: every example's expected figures are
//! derived from its rules' symbolic forms, engine-free (P13 step 5).

use std::path::{Path, PathBuf};

use bookspec_checker::{Derived, Example, derive};

fn examples() -> Vec<Example> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples");
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "toml"))
        .collect();
    files.sort();
    files
        .iter()
        .map(|f| Example::from_toml(&std::fs::read_to_string(f).unwrap()).unwrap())
        .collect()
}

/// No example's figure disagrees with what its rule derives.
#[test]
fn no_example_disagrees_with_its_derivation() {
    let mut bad = Vec::new();
    let mut agreed = 0;
    for ex in examples() {
        let d = derive(&ex);
        bad.extend(d.disagreements());
        agreed += d.agreed();
    }
    assert!(
        bad.is_empty(),
        "the examples and the rules disagree:\n{}",
        bad.join("\n")
    );
    assert!(
        agreed >= 30,
        "the reference model derived only {agreed} results"
    );
}

/// The nine examples written for the ledger's own laws are derived in
/// full: not one of their results is skipped.
#[test]
fn the_law_examples_are_derived_in_full() {
    for id in [
        "entry-an-unbalanced-entry-is-refused",
        "entry-a-line-amount-is-positive",
        "ledger-a-fraction-of-a-cent-is-refused",
        "statement-the-books-balance-and-the-equation-holds",
        "ledger-a-mistake-is-reversed-never-erased",
        "period-a-close-sweeps-the-year-into-capital",
        "entry-cents-add-exactly",
    ] {
        let ex = examples().into_iter().find(|e| e.id == id).unwrap();
        let d = derive(&ex);
        assert!(!d.results.is_empty(), "{id}");
        for (what, r) in &d.results {
            assert!(matches!(r, Derived::Agrees { .. }), "{id}: {what}: {r:?}");
        }
    }
}

/// An example whose expectation is wrong fails the specification's build:
/// the same example with one cent moved is caught, and so is a refusal the
/// rules do not justify.
#[test]
fn a_wrong_expectation_is_caught() {
    let ex = examples()
        .into_iter()
        .find(|e| e.id == "statement-the-books-balance-and-the-equation-holds")
        .unwrap();
    let text = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../examples/statement-the-books-balance-and-the-equation-holds.toml"),
    )
    .unwrap();
    assert!(derive(&ex).disagreements().is_empty());
    let wrong = text.replacen("\"1000\" = \"6550.25\"", "\"1000\" = \"6550.26\"", 1);
    let d = derive(&Example::from_toml(&wrong).unwrap());
    assert!(
        d.disagreements().iter().any(|m| m.contains("6550.26")),
        "{:?}",
        d.disagreements()
    );
    // A refusal the rules do not justify: the entry balances.
    let ex = examples()
        .into_iter()
        .find(|e| e.id == "entry-an-unbalanced-entry-is-refused")
        .unwrap();
    let text = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../examples/entry-an-unbalanced-entry-is-refused.toml"),
    )
    .unwrap();
    assert!(derive(&ex).disagreements().is_empty());
    let wrong = text.replacen("amount = \"90.00\"", "amount = \"100.00\"", 1);
    let d = derive(&Example::from_toml(&wrong).unwrap());
    assert!(
        d.disagreements()
            .iter()
            .any(|m| m.contains("equal credits")),
        "{:?}",
        d.disagreements()
    );
}
