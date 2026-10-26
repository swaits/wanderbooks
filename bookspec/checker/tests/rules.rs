//! The rule catalogue: identifiers, the index table, and the examples
//! behind each rule.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use bookspec_checker::{Area, Claims, RuleId, Status, covered_rules, rules};

fn repo_path(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

/// Every rule B-1 to B-45 has exactly one identifier, its area's number
/// counts from one without a gap, and the identifier spells the area.
#[test]
fn every_b_rule_has_one_id() {
    let mut seen = BTreeSet::new();
    let mut by_area: BTreeMap<Area, Vec<u32>> = BTreeMap::new();
    for b in 1..=45 {
        let rule = bookspec_checker::rule(b).unwrap_or_else(|| panic!("no rule B-{b}"));
        assert_eq!(rule.id.area(), rule.area, "B-{b}: the id's area");
        assert_eq!(
            rule.id.as_str().split('-').next(),
            Some("BS"),
            "B-{b}: the identifier's prefix"
        );
        assert!(seen.insert(rule.id.clone()), "B-{b}: the id is used twice");
        assert_eq!(RuleId::parse(rule.id.as_str()).unwrap(), rule.id);
        let n: u32 = rule
            .id
            .as_str()
            .rsplit_once('-')
            .unwrap()
            .1
            .parse()
            .unwrap();
        by_area.entry(rule.area).or_default().push(n);
    }
    assert_eq!(seen.len(), 45);
    for (area, numbers) in by_area {
        let mut want: Vec<u32> = (1..=numbers.len() as u32).collect();
        let mut got = numbers;
        got.sort_unstable();
        want.sort_unstable();
        assert_eq!(got, want, "{}: numbers count from one", area.key());
    }
}

/// The index table is the catalogue: one row per rule, the same id, area
/// and status.
#[test]
fn the_index_matches_the_catalogue() {
    let text = std::fs::read_to_string(repo_path("../spec/rules-index.md"))
        .expect("bookspec/spec/rules-index.md");
    let mut rows = BTreeMap::new();
    for line in text.lines() {
        let Some(rest) = line.strip_prefix("| B-") else {
            continue;
        };
        let fields: Vec<&str> = line.split('|').map(str::trim).collect();
        // "", "B-n", "`BS-...`", area, status, rule, examples, ""
        assert_eq!(fields.len(), 8, "the table has six columns: {line}");
        let b: u8 = rest
            .split_once(' ')
            .unwrap()
            .0
            .parse()
            .unwrap_or_else(|_| panic!("{line}"));
        let id = fields[2].trim_matches('`');
        let area = fields[3];
        let status = fields[4];
        assert!(
            rows.insert(b, (id.to_string(), area.to_string(), status.to_string()))
                .is_none()
        );
    }
    for rule in rules() {
        let (id, area, status) = rows
            .get(&rule.b)
            .unwrap_or_else(|| panic!("the index lists B-{}", rule.b));
        assert_eq!(id, rule.id.as_str(), "B-{}", rule.b);
        assert_eq!(area, rule.area.key(), "B-{}", rule.b);
        let want = rule.status.map_or("—", |s| s.key());
        assert_eq!(status, want, "B-{}", rule.b);
    }
    assert_eq!(rows.len(), 45, "the index lists every rule once");
}

/// Every rule either shows an example or is allocated and not yet stated;
/// no example id is invented, and every example file is listed.
#[test]
fn every_example_is_listed_under_a_rule() {
    let mut files: Vec<String> = std::fs::read_dir(repo_path("../examples"))
        .expect("bookspec/examples")
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "toml"))
        .map(|p| p.file_stem().unwrap().to_string_lossy().into_owned())
        .collect();
    files.sort();
    let listed: BTreeSet<&str> = rules()
        .iter()
        .flat_map(|r| r.examples.iter().copied())
        .collect();
    for f in &files {
        assert!(listed.contains(f.as_str()), "{f} is listed under no rule");
    }
    for id in &listed {
        assert!(
            files.iter().any(|f| f == id),
            "the catalogue lists {id}, which is no example"
        );
    }
    for rule in rules() {
        if rule.status.is_none() {
            assert!(
                !rule.has_examples(),
                "B-{} is not yet stated, so it has no example",
                rule.b
            );
        }
    }
}

/// A rule with evidence is claimed by an engine; one without is not.
#[test]
fn a_claimed_rule_has_an_example() {
    let covered = covered_rules();
    assert!(!covered.is_empty());
    for rule in covered {
        assert!(rule.has_examples(), "{}", rule.id);
        assert!(
            matches!(
                rule.status,
                Some(Status::Authority | Status::Convention | Status::Choice)
            ),
            "{}",
            rule.id
        );
    }
}

/// An identifier has one spelling: `BS-<area>-<n>`, no leading zero.
#[test]
fn an_id_has_one_spelling() {
    assert_eq!(RuleId::parse("BS-entry-1").unwrap().as_str(), "BS-entry-1");
    for bad in [
        "entry-1",
        "BS-nothing-1",
        "BS-entry",
        "BS-entry-0",
        "BS-entry-01",
        "BS-entry-x",
        "BS-entry-1-2",
    ] {
        assert!(RuleId::parse(bad).is_err(), "{bad:?}");
    }
}

/// A choice rule's option is declared by the engine, not by the spec.
#[test]
fn a_choice_rule_declares_its_option() {
    let rule = bookspec_checker::rule(13).unwrap();
    assert_eq!(rule.status, Some(Status::Authority));
    let choice = bookspec_checker::rule(31).unwrap();
    assert_eq!(choice.status, Some(Status::Choice));
    let claims = Claims::new([choice.id.clone()]).with_choice(choice.id.clone(), "refuse_blank");
    assert_eq!(claims.choice(&choice.id), Some("refuse_blank"));
    assert_eq!(claims.choice(&rule.id), None);
}
