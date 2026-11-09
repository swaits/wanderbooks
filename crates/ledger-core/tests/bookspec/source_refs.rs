//! The source-citation checks (unit S-E, tightened in review).
//!
//! Every spec block's source clause -- the line that introduces it plus
//! its continuation lines in the same comment block, up to the next
//! blank comment line or the next uppercase heading -- names a rule or
//! a source that exists: a bookspec id the pinned bookspec has, a
//! product-rule id in `docs/spec/claims/`, a repo path that exists, a
//! code reference that names a real item, or a declared external or
//! free-text citation. Path-like and code-reference tokens are checked
//! even when a rule id is also cited; they never fall through to the
//! free-text branch. Clauses that pass only through the free-text
//! branch are counted, and that count is a floor-style constant: it
//! changes only on purpose, in the same commit as the constant.
//!
//! A bookspec id whose record is retired is no source: the retired
//! record states what the rule was, not what governs now. It is kept
//! only in a historical mention that names a live successor in the same
//! clause ("formerly BS-ledger-13, now WB-ledger-16"), so a reader can
//! follow the rule forward without the code resting on a retired id.
//!
//! The second check flags any retired rule number (the old wanderbooks
//! numbering, a word-boundary capital letter, hyphen and a number from
//! 1 to 45) in every tracked text file, outside the excluded index and
//! claims files. No occurrence is kept (unit S-E2, 2026-11-08: program
//! output names bookspec ids too); a future deliberate keep is
//! allow-listed below with its exact count per file, and the count
//! failing in either direction means the list changes only on purpose.
//! Scanning the whole file text, not line by line, also catches a
//! reference wrapped across lines, filename at the end of one line and
//! the number at the start of the next behind a comment prefix.

use crate::*;

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

fn collect_all(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == "target" || name == ".git" {
            continue;
        }
        if path.is_dir() {
            collect_all(&path, out);
        } else {
            out.push(path);
        }
    }
}

/// The ids in `text` that start with `prefix`, each cut at the first
/// character that cannot be in an id.
fn ids_in(text: &str, prefix: &str) -> Vec<String> {
    let mut ids = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find(prefix) {
        let tail: String = rest[at..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect();
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

/// The number of source clauses checked must stay at or above this.
const CHECKED_FLOOR: usize = 300;
/// Clauses passing only through the declared or external free-text
/// branch. Edit this constant, on purpose, in the same commit that
/// changes the count.
const FREE_TEXT_COUNT: usize = 56;

fn strip_comment(line: &str) -> Option<String> {
    let t = line.trim_start();
    t.strip_prefix("//!")
        .or_else(|| t.strip_prefix("///"))
        .or_else(|| t.strip_prefix("//"))
        .map(|rest| rest.trim().to_string())
}

fn is_heading(content: &str) -> bool {
    let Some((head, _)) = content.split_once(':') else {
        return false;
    };
    !head.is_empty()
        && head.len() <= 32
        && head.chars().filter(|c| c.is_ascii_uppercase()).count() >= 2
        && head
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || " _-/()".contains(c))
}

fn clean_token(token: &str) -> String {
    token
        .trim_start_matches(['`', '"', '\'', '(', '[', '{'])
        .trim_end_matches(['`', '"', '\'', ')', ']', '}', ',', ';', ':', '.'])
        .to_string()
}

fn is_path_like(token: &str) -> bool {
    [".md", ".rs", ".toml", ".json"]
        .iter()
        .any(|e| token.ends_with(e))
}

struct ClauseCtx<'a> {
    root: &'a Path,
    bs_ids: &'a BTreeSet<String>,
    /// The retired records' ids and why each was retired.
    retired_bs: &'a BTreeMap<String, String>,
    wb_ids: &'a BTreeSet<String>,
    identifiers: &'a BTreeSet<String>,
    basenames: &'a BTreeSet<String>,
}

fn check_clause(clause: &str, ctx: &ClauseCtx<'_>, bad: &mut Vec<String>, at: &str) -> bool {
    // Returns true when the clause passes only through the free-text branch.
    let Some(text) = clause.split("SOURCE:").nth(1) else {
        return false;
    };
    let mut errors: Vec<String> = Vec::new();
    let mut cited = false;
    let bs_ids = ids_in(text, "BS-");
    let wb_ids = ids_in(text, "WB-");
    // A live rule named in the same clause is a retired id's successor:
    // the retirement names it, or the claims file does.
    let successor = wb_ids.iter().any(|id| ctx.wb_ids.contains(id))
        || bs_ids
            .iter()
            .any(|id| ctx.bs_ids.contains(id) && !ctx.retired_bs.contains_key(id));
    for id in &bs_ids {
        cited = true;
        if !ctx.bs_ids.contains(id) {
            errors.push(format!("unknown bookspec id {id}"));
        } else if let Some(reason) = ctx.retired_bs.get(id)
            && !successor
        {
            errors.push(format!(
                "retired bookspec id {id} is no source ({reason}); cite the rule that states it now, or name its successor in this clause"
            ));
        }
    }
    for id in &wb_ids {
        cited = true;
        if !ctx.wb_ids.contains(id) {
            errors.push(format!("unknown product rule {id}"));
        }
    }
    let mut has_path = false;
    let mut has_ref = false;
    for raw in text.split_whitespace() {
        let token = clean_token(raw);
        if token.contains("://") {
            continue;
        }
        if is_path_like(&token) {
            let exists = if token.contains('/') {
                ctx.root.join(&token).exists()
            } else {
                ctx.basenames.contains(&token)
            };
            if exists {
                has_path = true;
            } else {
                errors.push(format!("path {token} does not exist"));
            }
        }
        if token.contains("::") {
            let first = token.split("::").next().unwrap_or("");
            if first == "crate" || first.starts_with("ledger_") {
                let last = token.rsplit("::").next().unwrap_or("");
                let ident: String = last
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect();
                if ctx.identifiers.contains(&ident) {
                    has_ref = true;
                } else {
                    errors.push(format!("code reference {token} names nothing real"));
                }
            }
        }
    }
    if !errors.is_empty() {
        for e in errors {
            bad.push(format!("{at}: {e}"));
        }
        return false;
    }
    if cited || has_path || has_ref {
        return false;
    }
    if EXTERNAL.iter().any(|m| text.contains(m)) {
        return true;
    }
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
        "{at}: source clause cites no rule or source that exists"
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

    let retired_bs: BTreeMap<String, String> = bookspec_checker::records()
        .iter()
        .filter(|r| r.is_retired())
        .map(|r| {
            (
                r.id.to_string(),
                r.retired
                    .as_ref()
                    .map(|retired| retired.reason.clone())
                    .unwrap_or_default(),
            )
        })
        .collect();
    assert!(
        !retired_bs.is_empty(),
        "the pinned bookspec has retired rules"
    );

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

    // Identifiers declared anywhere under crates/, and file basenames
    // anywhere in the repo, for the path and code-reference checks.
    let mut crate_files = Vec::new();
    collect_all(&root.join("crates"), &mut crate_files);
    let mut identifiers = BTreeSet::new();
    for path in crate_files
        .iter()
        .filter(|p| p.extension().is_some_and(|e| e == "rs"))
    {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let words: Vec<&str> = text
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .filter(|w| !w.is_empty())
            .collect();
        for pair in words.windows(2) {
            if matches!(
                pair[0],
                "fn" | "struct" | "enum" | "mod" | "const" | "type" | "trait"
            ) {
                identifiers.insert(pair[1].to_string());
            }
        }
    }
    let mut all_files = Vec::new();
    collect_all(&root, &mut all_files);
    let basenames: BTreeSet<String> = all_files
        .iter()
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .collect();

    let mut files = Vec::new();
    for crate_dir in std::fs::read_dir(root.join("crates")).expect("crates") {
        let base = crate_dir.expect("a crate").path();
        for sub in ["src", "tests"] {
            let dir = base.join(sub);
            if dir.is_dir() {
                collect_rs(&dir, &mut files);
            }
        }
    }
    let fuzz_dir = root.join("fuzz");
    if fuzz_dir.is_dir() {
        collect_rs(&fuzz_dir, &mut files);
    }
    files.sort();
    assert!(!files.is_empty(), "crates and fuzz hold sources");

    let mut bad = Vec::new();
    let mut checked = 0usize;
    let mut free_text = 0usize;
    for path in &files {
        let text = std::fs::read_to_string(path).expect("a source file");
        let lines: Vec<&str> = text.lines().collect();
        let mut i = 0usize;
        while i < lines.len() {
            let Some(content) = strip_comment(lines[i]) else {
                i += 1;
                continue;
            };
            if !content.contains("SOURCE:") {
                i += 1;
                continue;
            }
            checked += 1;
            let at = format!("{}:{}", path.display(), i + 1);
            let mut clause = content.clone();
            let mut j = i + 1;
            while j < lines.len() {
                let Some(next) = strip_comment(lines[j]) else {
                    break;
                };
                if next.is_empty() || is_heading(&next) || next.contains("SOURCE:") {
                    break;
                }
                clause.push(' ');
                clause.push_str(&next);
                j += 1;
            }
            let ctx = ClauseCtx {
                root: &root,
                bs_ids: &bs_ids,
                retired_bs: &retired_bs,
                wb_ids: &wb_ids,
                identifiers: &identifiers,
                basenames: &basenames,
            };
            if check_clause(&clause, &ctx, &mut bad, &at) {
                free_text += 1;
            }
            i = j.max(i + 1);
        }
    }
    println!("source clauses checked: {checked}");
    println!("source clauses passing only through the free-text branch: {free_text}");
    assert!(
        checked >= CHECKED_FLOOR,
        "source clauses were found ({checked})"
    );
    assert_eq!(
        free_text, FREE_TEXT_COUNT,
        "free-text source clauses changed on purpose only: edit FREE_TEXT_COUNT in the same commit"
    );
    assert!(
        bad.is_empty(),
        "every source clause cites a rule or source that exists:\n{}",
        bad.join("\n")
    );
}

#[test]
fn a_retired_bookspec_id_is_no_source() {
    let bs_ids: BTreeSet<String> = ["BS-format-1", "BS-ledger-12", "BS-ledger-13", "BS-ledger-7"]
        .into_iter()
        .map(str::to_string)
        .collect();
    let retired: BTreeMap<String, String> = [
        ("BS-format-1", "wholly a product rule"),
        ("BS-ledger-12", "kept as WB-ledger-15"),
        ("BS-ledger-13", "kept as WB-ledger-16"),
    ]
    .into_iter()
    .map(|(id, why)| (id.to_string(), why.to_string()))
    .collect();
    let wb_ids: BTreeSet<String> = ["WB-format-1", "WB-ledger-15", "WB-ledger-16"]
        .into_iter()
        .map(str::to_string)
        .collect();
    let root = repo_root();
    let empty: BTreeSet<String> = BTreeSet::new();
    let ctx = ClauseCtx {
        root: &root,
        bs_ids: &bs_ids,
        retired_bs: &retired,
        wb_ids: &wb_ids,
        identifiers: &empty,
        basenames: &empty,
    };
    let problems = |text: &str| {
        let mut bad = Vec::new();
        check_clause(text, &ctx, &mut bad, "t");
        bad
    };

    // A bare retired id is no source, however much plan or decision
    // text surrounds it.
    assert_eq!(problems("SOURCE: BS-ledger-13.").len(), 1);
    assert_eq!(
        problems("SOURCE: BS-ledger-12; plan/served.md P6.").len(),
        1
    );
    assert_eq!(problems("SOURCE: BS-format-1; D-39.").len(), 1);

    // A historical mention that names a live successor in the same
    // clause is allowed, so the rule can be followed forward.
    assert!(problems("SOURCE: formerly BS-ledger-13, now WB-ledger-16.").is_empty());
    assert!(problems("SOURCE: formerly BS-ledger-12, now WB-ledger-15.").is_empty());
    assert!(problems("SOURCE: formerly BS-format-1, now WB-format-1.").is_empty());
    assert!(
        problems("SOURCE: formerly BS-ledger-12, now BS-ledger-7 and WB-ledger-15.").is_empty()
    );

    // A live rule alone passes as it always did.
    assert!(problems("SOURCE: BS-ledger-7.").is_empty());
}

fn retired_count(text: &str) -> usize {
    let chars: Vec<char> = text.chars().collect();
    let mut count = 0usize;
    let mut i = 0usize;
    while i < chars.len() {
        if chars[i] == 'B'
            && (i == 0 || !(chars[i - 1].is_alphanumeric() || chars[i - 1] == '_'))
            && chars.get(i + 1) == Some(&'-')
        {
            let mut j = i + 2;
            let mut value = 0usize;
            let mut digits = 0usize;
            while j < chars.len() && chars[j].is_ascii_digit() {
                value = value * 10 + chars[j].to_digit(10).unwrap_or(0) as usize;
                digits += 1;
                j += 1;
            }
            if digits > 0 && (1..=45).contains(&value) {
                count += 1;
            }
            i = j.max(i + 1);
        } else {
            i += 1;
        }
    }
    count
}

#[test]
pub(crate) fn no_retired_rule_reference_remains() {
    let root = repo_root();
    // Deliberately kept occurrences: path and exact count. The list is
    // empty (unit S-E2): every citation, program output included, names
    // a bookspec id. A future keep adds its path and count here, on
    // purpose, in that commit.
    let allow: &[(&str, usize)] = &[];
    let output = std::process::Command::new("git")
        .args(["ls-files", "-z"])
        .current_dir(&root)
        .output()
        .expect("git ls-files");
    assert!(output.status.success(), "git ls-files succeeds");
    let listing = String::from_utf8_lossy(&output.stdout);
    let mut bad = Vec::new();
    let mut seen_allow: BTreeSet<&str> = BTreeSet::new();
    for rel in listing.split('\0').filter(|s| !s.is_empty()) {
        if rel.starts_with("plan/")
            || rel == "docs/spec/rules.md"
            || rel.starts_with("docs/spec/claims/")
        {
            continue;
        }
        let path = root.join(rel);
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        if text.contains('\0') {
            continue;
        }
        // Whole-text scan: a reference wrapped across a newline and a
        // comment prefix is still one occurrence of the number.
        let count = retired_count(&text);
        if count == 0 {
            continue;
        }
        match allow.iter().find(|(p, _)| *p == rel) {
            Some((_, expected)) => {
                seen_allow.insert(rel);
                if count != *expected {
                    bad.push(format!(
                        "{rel}: {count} kept occurrences, allow-list says {expected}"
                    ));
                }
            }
            None => bad.push(format!("{rel}: {count} retired rule reference(s)")),
        }
    }
    for (p, _) in allow {
        if !seen_allow.contains(p) {
            bad.push(format!(
                "{p}: allow-listed file has no kept occurrence left"
            ));
        }
    }
    assert!(
        bad.is_empty(),
        "no retired rule reference remains outside the map files and the allow-list:\n{}",
        bad.join("\n")
    );
}
