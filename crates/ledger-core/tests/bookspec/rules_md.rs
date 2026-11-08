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
        if rule.b == 0 {
            // Born after the B-n numbering ended (P14 unit S-C7): no index
            // row and no B-n section; its pick is recorded in the claims
            // file of its parent.
            assert!(
                all.contains(&format!("**Pick ({}):**", rule.id)),
                "{} carries its pick in its claims file",
                rule.id
            );
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
