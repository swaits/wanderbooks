use crate::*;

/// `docs/spec/rules.md` carries every stated rule's id under its heading,
/// and no longer calls a choice a decision (P13 step 3).
#[test]
pub(crate) fn rules_md_carries_every_id() {
    let text = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/spec/rules.md"),
    )
    .expect("docs/spec/rules.md");
    for rule in bookspec_checker::rules()
        .iter()
        .filter(|r| r.status.is_some())
    {
        assert!(
            text.contains(&format!("**bookspec:** `{}`", rule.id)),
            "B-{} carries {}",
            rule.b,
            rule.id
        );
    }
    assert!(
        !text.contains("DECISION"),
        "rules.md speaks bookspec's CHOICE"
    );
}
