use crate::*;

/// `docs/spec/claims/*.md` carry every stated rule's id under its heading,
/// `docs/spec/rules.md` indexes every rule to its claims file, and neither
/// calls a choice a decision (P14 unit S-D0; P13 step 3).
#[test]
pub(crate) fn rules_md_carries_every_id() {
    let spec = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/spec");
    let index = std::fs::read_to_string(spec.join("rules.md")).expect("docs/spec/rules.md");

    let mut claims = std::fs::read_dir(spec.join("claims"))
        .expect("docs/spec/claims")
        .map(|entry| entry.expect("a claims entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "md"))
        .collect::<Vec<_>>();
    claims.sort();
    assert!(!claims.is_empty(), "docs/spec/claims holds the rule text");

    let mut all = String::new();
    for path in &claims {
        let text = std::fs::read_to_string(path)
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        all.push_str(&text);
        all.push('\n');
    }

    for rule in bookspec_checker::rules().iter() {
        if rule.is_retired() {
            continue;
        }
        // Every non-retired CHOICE record wanderbooks claims -- whether
        // it carries a B number or was split out of another rule (P14
        // unit S-C7) -- has its pick recorded in its claims file, and
        // the engine's adapter declares the same option, which the
        // record names, so no CHOICE can go undeclared.
        if rule.status == Some(bookspec_checker::Status::Choice) {
            let marker = format!("**Pick ({}):** `", rule.id);
            let at = all
                .find(&marker)
                .unwrap_or_else(|| panic!("{} carries its pick in its claims file", rule.id));
            let option = all[at + marker.len()..].split('`').next().unwrap();
            assert!(
                rule.record().options.iter().any(|o| o.name == option),
                "{}'s pick {option:?} is one of its options",
                rule.id
            );
            assert!(
                engine::PICKS
                    .iter()
                    .any(|&(area, n, o)| RuleId::new(area, n) == rule.id && o == option),
                "{}'s pick {option:?} is declared in the adapter's claims()",
                rule.id
            );
        }
        if rule.b == 0 {
            // A rule born after the B numbering has no row in the index.
            continue;
        }
        assert!(
            index.contains(&format!("| B-{} | `{}` |", rule.b, rule.id)),
            "rules.md indexes B-{} to {}",
            rule.b,
            rule.id
        );
        if rule.status.is_some() {
            assert!(
                all.contains(&format!("**bookspec:** `{}`", rule.id)),
                "B-{} carries {} in its claims file",
                rule.b,
                rule.id
            );
        }
    }
    assert!(
        !index.contains("DECISION") && !all.contains("DECISION"),
        "the rules speak bookspec's CHOICE"
    );
}
