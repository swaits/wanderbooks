use crate::*;

/// Every `SOURCE:` line in `crates/*/src` names a rule or a source that
/// exists (unit S-E): a `BS-<area>-<n>` the pinned bookspec has, a
/// `WB-<area>-<n>` in `docs/spec/claims/`, a vendored or repo source
/// path that exists, or an external citation (a statute, an ASC
/// paragraph, an IRS publication, a standard). Declared decisions and
/// plan citations name their document the same way. A `BS-` or `WB-` id
/// that does not exist fails here, so a typo'd id cannot survive.
/// And no retired `rules.md` B-n citation remains outside `plan/` and
/// the two map files (`docs/spec/rules.md`, `docs/spec/claims/`).
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn collect_rs(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// The ids in `text` that start with `prefix` (`BS-` or `WB-`), each cut
/// at the first character that cannot be in an id.
fn ids_in(text: &str, prefix: &str) -> Vec<String> {
    let mut ids = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find(prefix) {
        let tail: String = rest[at..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect();
        // An id is prefix + area + '-' + number; keep only full shapes.
        let body = &tail[prefix.len()..];
        if body
            .rsplit('-')
            .next()
            .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
            && body.contains('-')
        {
            ids.push(tail);
        }
        rest = &rest[at + prefix.len()..];
    }
    ids
}

const EXTERNAL: &[&str] = &[
    "U.S.C.",
    "§",
    "ASC ",
    "IRS",
    "Pub.",
    "Form ",
    "Treas.",
    "Rev. Proc.",
    "IRC",
    "ISO 4217",
    "Unicode",
    "Pacioli",
    "Instructions for",
    "RFC ",
];

fn source_line_ok(
    line: &str,
    root: &Path,
    bs_ids: &BTreeSet<String>,
    wb_ids: &BTreeSet<String>,
    bad: &mut Vec<String>,
    at: &str,
) -> bool {
    let Some(text) = line.split("SOURCE:").nth(1) else {
        return true;
    };
    let mut cited = false;
    for id in ids_in(text, "BS-") {
        cited = true;
        if !bs_ids.contains(&id) {
            bad.push(format!("{at}: unknown bookspec id {id}"));
        }
    }
    for id in ids_in(text, "WB-") {
        cited = true;
        if !wb_ids.contains(&id) {
            bad.push(format!("{at}: unknown product rule {id}"));
        }
    }
    if cited {
        return true;
    }
    // A repo path that exists (vendored sources, docs, the plan).
    for token in text.split_whitespace() {
        let token = token.trim_matches(|c| matches!(c, '`' | '"' | '\'' | ')' | ',' | ';' | '('));
        if token.contains('/')
            && token
                .rsplit('.')
                .next()
                .is_some_and(|e| ["md", "rs", "toml", "json"].contains(&e))
            && root.join(token).exists()
        {
            return true;
        }
    }
    if EXTERNAL.iter().any(|m| text.contains(m)) {
        return true;
    }
    // Declared decisions, requirements, plan citations and named code
    // contracts: each names the document or module that states it.
    let declared = text.contains("DECISION")
        || text.contains("CONVENTION")
        || text.contains("CHOICE")
        || text.contains("::")
        || text.contains("plan ")
        || text.contains("plan/")
        || text.contains("server.rs")
        || text.contains("the module's rule")
        || text.contains("the books.")
        || text.contains("owner")
        || text
            .split(|c: char| !c.is_ascii_alphanumeric() && c != '-')
            .any(|t| {
                (t.starts_with("R-")
                    || t.starts_with("D-")
                    || t.starts_with("I-")
                    || t.starts_with("A-"))
                    && t.len() > 2
                    && t[2..].chars().next().is_some_and(|c| c.is_ascii_digit())
                    && t[2..].chars().all(|c| c.is_ascii_alphanumeric())
            });
    if declared {
        return true;
    }
    bad.push(format!(
        "{at}: SOURCE cites no rule or source that exists: {}",
        line.trim()
    ));
    false
}

#[test]
pub(crate) fn every_source_line_cites_an_existing_rule_or_source() {
    let root = repo_root();
    let bs_ids: BTreeSet<String> = bookspec_checker::rules()
        .iter()
        .map(|r| r.id.to_string())
        .collect();
    assert!(!bs_ids.is_empty(), "the pinned bookspec has rules");

    let claims_dir = root.join("docs/spec/claims");
    let mut wb_ids = BTreeSet::new();
    for entry in std::fs::read_dir(&claims_dir).expect("docs/spec/claims") {
        let path = entry.expect("a claims entry").path();
        if path.extension().is_some_and(|e| e == "md") {
            let text = std::fs::read_to_string(&path).expect("a claims file");
            wb_ids.extend(ids_in(&text, "WB-"));
        }
    }
    assert!(!wb_ids.is_empty(), "the claims files hold product rules");

    let mut files = Vec::new();
    for crate_dir in std::fs::read_dir(root.join("crates")).expect("crates") {
        let src = crate_dir.expect("a crate").path().join("src");
        if src.is_dir() {
            collect_rs(&src, &mut files);
        }
    }
    files.sort();
    assert!(!files.is_empty(), "crates/*/src holds sources");

    let mut bad = Vec::new();
    let mut checked = 0usize;
    for path in &files {
        let text = std::fs::read_to_string(path).expect("a source file");
        for (i, line) in text.lines().enumerate() {
            if !line.contains("SOURCE:") {
                continue;
            }
            checked += 1;
            let at = format!("{}:{}", path.display(), i + 1);
            source_line_ok(line, &root, &bs_ids, &wb_ids, &mut bad, &at);
        }
    }
    assert!(checked > 100, "SOURCE lines were found ({checked})");
    assert!(
        bad.is_empty(),
        "every SOURCE line cites a rule or source that exists:\n{}",
        bad.join("\n")
    );
}

#[test]
pub(crate) fn no_retired_rule_reference_remains() {
    let root = repo_root();
    let mut files = Vec::new();
    for top in ["crates", "docs", "deploy", "scripts", ".github"] {
        let dir = root.join(top);
        if dir.is_dir() {
            collect_text(&dir, &mut files);
        }
    }
    for top in ["AGENTS.md", "README.md"] {
        files.push(root.join(top));
    }
    let map_a = root.join("docs/spec/rules.md");
    let claims = root.join("docs/spec/claims");
    let mut bad = Vec::new();
    for path in files {
        if path == map_a || path.starts_with(&claims) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue; // binary
        };
        for (i, line) in text.lines().enumerate() {
            if retired_ref(line) {
                bad.push(format!("{}:{}: {}", path.display(), i + 1, line.trim()));
            }
        }
    }
    assert!(
        bad.is_empty(),
        "no retired rule reference remains outside the map files:\n{}",
        bad.join("\n")
    );
}

fn collect_text(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_text(&path, out);
        } else {
            out.push(path);
        }
    }
}

/// The retired form: filename, space, B-<digits> (the script that
/// performed the rewrite keeps the backticked form in its docs).
fn retired_ref(line: &str) -> bool {
    let needle = ["rules.md ", "B-"].concat();
    line.find(&needle).is_some_and(|at| {
        line[at + needle.len()..]
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_digit())
    })
}
