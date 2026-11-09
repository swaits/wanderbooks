use crate::engine::unpicked_option;
use crate::*;

/// The `Engine` adapter over `Store` runs every example through the
/// runner, and every rule the adapter claims passes (`plan/served.md` P13
/// step 2). The loader above proves the same figures with its own checks;
/// this proves the trait path, which is what another engine supplies.
#[test]
pub(crate) fn the_runner_reports_every_claimed_rule_green() {
    let mut examples = Vec::new();
    for (name, text) in bookspec_checker::examples() {
        examples.push(Example::from_toml(text).unwrap_or_else(|e| panic!("{name}: {e}")));
    }
    assert!(examples.len() >= 56, "the seed has at least 56 examples");
    let report = bookspec_runner::run(&examples, bookspec_checker::rules(), Books::blank);
    assert!(
        report.passed(),
        "the runner is not green:\n{}",
        report.failures().join("\n")
    );
    // An example for an option wanderbooks did not pick is not run; every
    // other one is, and passes.
    let skipped = report.skipped();
    let unpicked: Vec<&str> = examples
        .iter()
        .filter(|ex| unpicked_option(ex).is_some())
        .map(|ex| ex.id.as_str())
        .collect();
    assert_eq!(
        skipped.len(),
        unpicked.len(),
        "the runner skips exactly the examples for unpicked options: {skipped:?}"
    );
    for id in &unpicked {
        assert!(
            skipped.iter().any(|s| s.starts_with(&format!("{id}:"))),
            "{id} is skipped"
        );
    }
    let ran = report.examples.len() - skipped.len();
    let passed = report
        .examples
        .iter()
        .filter(|e| e.skipped.is_none() && e.failure.is_none())
        .count();
    eprintln!(
        "bookspec runner: {} examples, {ran} run, {passed} passed, {} skipped (an option wanderbooks did not pick)",
        report.examples.len(),
        skipped.len()
    );
    for s in &skipped {
        eprintln!("  skipped {s}");
    }
    assert_eq!(ran, passed, "every example run passed");
    for rule in bookspec_checker::covered_rules() {
        assert_eq!(
            report.rules.get(&rule.id),
            Some(&bookspec_runner::Outcome::Pass),
            "{}",
            rule.id
        );
    }
    assert!(
        report
            .rules
            .values()
            .any(|o| *o == bookspec_runner::Outcome::NotClaimed),
        "a rule with no example is not claimed"
    );
}

#[test]
pub(crate) fn every_bookspec_example_holds() {
    let files = bookspec_checker::examples();
    let mut ids = BTreeSet::new();
    let mut checked: BTreeMap<String, usize> = BTreeMap::new();
    let mut skipped = Vec::new();
    for (name, text) in &files {
        let ex = Example::from_toml(text).unwrap_or_else(|e| panic!("{name}: {e}"));
        let stem = name.strip_suffix(".toml").unwrap();
        assert_eq!(ex.id, stem, "{name}: the file is named for its id");
        assert!(ids.insert(ex.id.clone()), "{}: id used twice", ex.id);
        assert!(
            !ex.title.is_empty() && !ex.rule.is_empty() && !ex.tags.is_empty(),
            "{}: title, rule and tags",
            ex.id
        );
        assert!(
            matches!(ex.status.as_str(), "AUTHORITY" | "CONVENTION" | "CHOICE"),
            "{}: status {:?}",
            ex.id,
            ex.status
        );
        assert!(!ex.sources.is_empty(), "{}: every rule is sourced", ex.id);
        assert!(!ex.expect.is_empty(), "{}: something to expect", ex.id);
        // The examples for an option wanderbooks did not pick are for
        // another engine; they are read (above) and not run.
        if let Some(why) = unpicked_option(&ex) {
            skipped.push(format!("{} {why}", ex.id));
            continue;
        }
        for (q, by) in run(&ex) {
            if let Some(by) = by {
                eprintln!("{}: {q} is checked by {by}", ex.id);
            }
            *checked.entry(q).or_insert(0) += 1;
        }
    }
    // Each query the examples use ran: the cash flow's seven examples at
    // least, the as-of and 1099 examples', the members closes', the
    // formats appendix's, and P3's default period, rename, tax category,
    // parties, 1099, depreciation, schedules, asset basis and K-1 tax-basis
    // examples'.
    for (q, at_least) in [
        ("cash_flow", 7),
        ("trial_balance", 2),
        ("profit_and_loss", 2),
        ("vendor_1099_paid", 4),
        ("vendor_1099", 10),
        ("entry", 8),
        ("balances", 4),
        ("members_equity", 10),
        ("k1", 3),
        ("party_part", 28),
        ("party_totals", 23),
        ("party_tin_last4", 5),
        ("iif_lines", 1),
        ("iif_changes", 1),
        ("period", 6),
        ("day", 1),
        ("calendar_year", 1),
        ("account_names", 2),
        ("uncategorized_accounts", 2),
        ("rename_account", 5),
        ("add_account", 4),
        ("set_tax_category", 2),
        ("run", 4),
        ("disposal", 5),
        ("depreciation_schedule", 5),
        ("schedule_plan", 3),
        ("register_row", 5),
        ("register_totals", 1),
        ("asset_basis", 6),
        ("k1_tax_basis", 4),
        ("k1_reconciliation", 2),
        ("project", 6),
        ("aging", 4),
        ("document_dates", 8),
    ] {
        let n = checked.get(q).copied().unwrap_or(0);
        assert!(n >= at_least, "{q} ran {n} times, fewer than {at_least}");
    }
    eprintln!(
        "bookspec examples: {} files, {} run, {} skipped (an option wanderbooks did not pick): {skipped:?}",
        files.len(),
        files.len() - skipped.len(),
        skipped.len()
    );
    assert_eq!(
        skipped.len(),
        3,
        "the three unpicked-option examples: {skipped:?}"
    );
}
