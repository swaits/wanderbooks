//! The specification text (`bookspec/spec/*.md`, P13 step 4): every rule
//! has its section, its statement, its pinned sources, its symbolic form
//! and its examples, and every citation resolves.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use bookspec_checker::{Rule, rules};

fn spec_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../spec")
}

fn sources_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/spec/sources")
}

/// A rule's section as the text gives it.
#[derive(Default, Debug)]
struct Section {
    file: String,
    heading: String,
    fields: BTreeMap<String, Vec<String>>,
}

/// Every `### BS-...` section of the area files, by its identifier.
fn sections() -> BTreeMap<String, Section> {
    let mut out = BTreeMap::new();
    let mut files: Vec<PathBuf> = std::fs::read_dir(spec_dir())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .collect();
    files.sort();
    for f in files {
        let name = f.file_name().unwrap().to_string_lossy().into_owned();
        let text = std::fs::read_to_string(&f).unwrap();
        let mut current: Option<(String, Section)> = None;
        let mut field = String::new();
        for line in text.lines() {
            if let Some(h) = line.strip_prefix("### BS-") {
                if let Some((id, s)) = current.take() {
                    assert!(out.insert(id.clone(), s).is_none(), "{id} twice");
                }
                let id = format!("BS-{}", h.split(':').next().unwrap());
                current = Some((
                    id,
                    Section {
                        file: name.clone(),
                        heading: h.to_string(),
                        ..Section::default()
                    },
                ));
                continue;
            }
            let Some((_, s)) = current.as_mut() else {
                continue;
            };
            if let Some(rest) = line.strip_prefix("- **") {
                let (key, value) = rest.split_once(":**").expect("a field: - **Name:** ...");
                field = key.to_string();
                let e = s.fields.entry(field.clone()).or_default();
                if !value.trim().is_empty() {
                    e.push(value.trim().to_string());
                }
            } else if let Some(item) = line.strip_prefix("  - ") {
                s.fields
                    .entry(field.clone())
                    .or_default()
                    .push(item.to_string());
            }
        }
        if let Some((id, s)) = current.take() {
            assert!(out.insert(id.clone(), s).is_none(), "{id} twice");
        }
    }
    out
}

fn one<'a>(s: &'a Section, id: &str, field: &str) -> &'a str {
    let v = s
        .fields
        .get(field)
        .unwrap_or_else(|| panic!("{id}: no {field}"));
    assert_eq!(v.len(), 1, "{id}: {field} is one line");
    &v[0]
}

/// Whitespace and typographic quotes made plain, so a pin copied from a
/// wrapped or curly-quoted source still matches.
fn squash(s: &str) -> String {
    let plain = s
        .replace(['\u{2019}', '\u{2018}'], "'")
        .replace(['\u{201c}', '\u{201d}'], "\"");
    plain.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn backticked(s: &str) -> Vec<&str> {
    s.split('`').skip(1).step_by(2).collect()
}

/// Every one of the 45 rules has one section, in the file of its area,
/// headed with its identifier and B-number.
#[test]
fn every_rule_has_its_section() {
    let sections = sections();
    assert_eq!(sections.len(), 45, "one section per rule");
    for rule in rules() {
        let s = sections
            .get(rule.id.as_str())
            .unwrap_or_else(|| panic!("{}: no section", rule.id));
        assert_eq!(s.file, format!("{}.md", rule.area.key()), "{}", rule.id);
        assert!(
            s.heading.ends_with(&format!("(B-{})", rule.b)),
            "{}: heading {:?} names B-{}",
            rule.id,
            s.heading,
            rule.b
        );
    }
}

/// A stated rule has a status the catalogue agrees with, a normative
/// statement, a symbolic form and applicability; a reserved one says so.
#[test]
fn every_stated_rule_is_complete() {
    let sections = sections();
    for rule in rules() {
        let s = &sections[rule.id.as_str()];
        let id = rule.id.as_str();
        let status = one(s, id, "Status");
        match rule.status {
            Some(st) => {
                assert!(status.starts_with(st.key()), "{id}: status {status:?}");
                let statement = one(s, id, "Statement");
                assert!(
                    ["MUST", "SHOULD", "MAY"]
                        .iter()
                        .any(|w| statement.contains(w)),
                    "{id}: a statement says MUST, SHOULD or MAY"
                );
                for field in ["Symbolic form", "Applicability"] {
                    assert!(
                        one(s, id, field).len() > 10 && !one(s, id, field).starts_with("none"),
                        "{id}: {field}"
                    );
                }
            }
            None => assert!(status.starts_with("reserved"), "{id}: reserved"),
        }
    }
}

/// A section's examples are the catalogue's, exactly; "none yet" says why.
#[test]
fn the_examples_are_the_catalogues() {
    let sections = sections();
    for rule in rules() {
        let s = &sections[rule.id.as_str()];
        let id = rule.id.as_str();
        let line = one(s, id, "Examples");
        let listed: BTreeSet<&str> = backticked(line)
            .into_iter()
            .filter(|b| *b != "none yet")
            .collect();
        let want: BTreeSet<&str> = rule.examples.iter().copied().collect();
        if rule.examples.is_empty() {
            assert!(line.starts_with("none yet"), "{id}: {line}");
            if rule.status.is_some() {
                assert!(
                    line.len() > "none yet. ".len() + 20,
                    "{id}: a stated rule with no example says why"
                );
            }
        } else {
            assert_eq!(listed, want, "{id}: examples");
        }
    }
}

/// Every citation resolves: a vendored source exists, and a verbatim
/// pin appears in it; an external reference names its paragraph.
#[test]
fn every_citation_resolves() {
    let sections = sections();
    let mut vendored = 0;
    let mut external = 0;
    let mut bad: Vec<String> = Vec::new();
    for rule in rules() {
        let s = &sections[rule.id.as_str()];
        let id = rule.id.as_str();
        let sources = s
            .fields
            .get("Sources")
            .unwrap_or_else(|| panic!("{id}: no Sources"));
        if rule.status.is_none() {
            assert_eq!(sources, &vec!["none yet".to_string()], "{id}");
            continue;
        }
        assert!(!sources.is_empty(), "{id}: sourced");
        for item in sources {
            if let Some(rest) = item.strip_prefix("vendored `") {
                let (file, rest) = rest
                    .split_once("`:")
                    .unwrap_or_else(|| panic!("{id}: {item}"));
                let path = sources_dir().join(file.strip_prefix("sources/").unwrap_or(file));
                let text = std::fs::read_to_string(&path)
                    .unwrap_or_else(|_| panic!("{id}: {file} is not vendored"));
                let hay = squash(&text);
                // Every quoted pin is verbatim in the file.
                let quoted: Vec<&str> = rest.split('"').skip(1).step_by(2).collect();
                for q in &quoted {
                    if !hay.contains(&squash(q)) {
                        bad.push(format!("{id}: {q:?} is not in {file}"));
                    }
                }
                if quoted.is_empty() {
                    bad.push(format!("{id}: {item} has no verbatim pin"));
                }
                vendored += 1;
            } else if let Some(rest) = item.strip_prefix("external:") {
                assert!(
                    rest.trim().len() > 20,
                    "{id}: an external reference is exact: {item}"
                );
                external += 1;
            } else {
                panic!("{id}: a source is `vendored` or `external`: {item}");
            }
        }
    }
    assert!(bad.is_empty(), "pins not found:\n{}", bad.join("\n"));
    assert!(vendored >= 30, "vendored citations: {vendored}");
    assert!(external >= 15, "external citations: {external}");
}

/// The first edition's page budget: 60 pages of 55 lines.
#[test]
fn the_edition_fits_its_page_budget() {
    let mut lines = 0;
    for e in std::fs::read_dir(spec_dir()).unwrap() {
        let p = e.unwrap().path();
        if p.extension().is_some_and(|x| x == "md") {
            lines += std::fs::read_to_string(p).unwrap().lines().count();
        }
    }
    assert!(lines <= 60 * 55, "{lines} lines is over 60 pages");
}

/// A stated rule with no example is exactly one an engine cannot claim.
#[test]
fn rules_without_examples_are_not_claimable() {
    let uncovered: Vec<&Rule> = rules().iter().filter(|r| !r.has_examples()).collect();
    let claimable: BTreeSet<_> = bookspec_checker::covered_rules()
        .into_iter()
        .map(|r| r.b)
        .collect();
    for r in uncovered {
        assert!(!claimable.contains(&r.b), "B-{}", r.b);
    }
}
