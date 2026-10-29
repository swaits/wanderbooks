//! The five laws are Lean theorems in `bookspec/lean` (P13 step 4), with
//! no assumption beyond Lean's standard axioms.

use std::path::{Path, PathBuf};
use std::process::Command;

const LAWS: [&str; 5] = [
    "law_debits_equal_credits",
    "law_accounting_equation",
    "law_trial_balance",
    "law_net_income_articulates",
    "law_cash_flow_is_change_in_cash",
];

fn lean_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../lean")
}

/// Every law is stated in `Laws.lean`, audited in `Audit.lean`, and no
/// file of the library says `sorry`.
#[test]
fn the_five_laws_are_stated_and_audited() {
    let laws = std::fs::read_to_string(lean_dir().join("Bookspec/Laws.lean")).unwrap();
    let audit = std::fs::read_to_string(lean_dir().join("Audit.lean")).unwrap();
    for law in LAWS {
        assert!(laws.contains(&format!("theorem {law}")), "{law} is stated");
        assert!(
            audit.contains(&format!("#print axioms {law}")),
            "{law} is audited"
        );
    }
    for entry in std::fs::read_dir(lean_dir().join("Bookspec")).unwrap() {
        let path = entry.unwrap().path();
        let text = std::fs::read_to_string(&path).unwrap();
        for (i, line) in text.lines().enumerate() {
            let code = line.split("--").next().unwrap();
            assert!(
                !code
                    .split(|c: char| !c.is_alphanumeric() && c != '_')
                    .any(|w| w == "sorry"),
                "{}:{}: sorry",
                path.display(),
                i + 1
            );
        }
    }
}

/// With Lean installed, the library builds and every law depends only on
/// `propext`, `Quot.sound` and `Classical.choice`. Without it the test
/// says so and passes: `mise install` provides Lean (`just prove` is
/// where the machine is known to have it).
#[test]
fn the_laws_use_standard_axioms_only() {
    let built = Command::new("lake")
        .arg("build")
        .current_dir(lean_dir())
        .output();
    let Ok(built) = built else {
        eprintln!("lake is not installed: the Lean build is not checked here");
        return;
    };
    assert!(
        built.status.success(),
        "lake build:\n{}",
        String::from_utf8_lossy(&built.stdout)
    );
    let out = Command::new("lake")
        .args(["env", "lean", "Audit.lean"])
        .current_dir(lean_dir())
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "audit:\n{text}");
    for law in LAWS {
        let line = text
            .lines()
            .find(|l| l.contains(&format!("Bookspec.{law}'")))
            .unwrap_or_else(|| panic!("{law} is not audited:\n{text}"));
        for axiom in line
            .split('[')
            .nth(1)
            .unwrap()
            .trim_end_matches(']')
            .split(", ")
        {
            assert!(
                ["propext", "Quot.sound", "Classical.choice"].contains(&axiom.trim()),
                "{law}: axiom {axiom}"
            );
        }
    }
}
