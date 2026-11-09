//! The sub-reference checks (unit S-E3).
//!
//! Unit S-E rewrote every `B-n` citation to its bookspec id and kept the
//! sub-reference after it ("item 7", "rule 11", "INVARIANT 2") as it was,
//! but those numbers counted parts of the retired rules.md text, not of
//! bookspec's records. S-E3 mapped each by content, and this check keeps
//! them true: every `BS-<area>-<n> statement k`, `invariant k` or
//! `scenario k` in any tracked file outside `plan/` names a part its
//! pinned record has (k from 1 to the length of that list), and the old
//! vocabulary (`item`, `rule`, `INVARIANT`, `SCENARIO`, `r`, or any
//! other word that is not exactly `statement`, `invariant` or
//! `scenario`) after a bookspec id, followed by a number, fails.
//!
//! A reference may be a list ("statements 5, 11 and 13"), a range
//! ("statements 27-30"), parenthesised, wrapped across lines behind a
//! comment prefix, or continued on the same id ("invariant 2 and
//! statement 11"); each number is one reference checked. The number
//! checked is floored and printed. This file is not scanned: its
//! self-tests quote the forms they must refuse.

use crate::*;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// How many references the scan must find, at least. Raise it, in the
/// same commit, when references are added; it never falls.
const CHECKED_FLOOR: usize = 320;

/// The tracked file this check does not scan: its own self-tests quote
/// the forms it refuses.
const SELF: &str = "crates/ledger-core/tests/bookspec/sub_refs.rs";

/// The parts a record has: its statements, invariants and scenarios.
#[derive(Clone, Copy)]
struct Parts {
    statements: usize,
    invariants: usize,
    scenarios: usize,
}

fn record_parts() -> BTreeMap<String, Parts> {
    bookspec_checker::records()
        .iter()
        .map(|r| {
            (
                r.id.to_string(),
                Parts {
                    statements: r.statement.len(),
                    invariants: r.invariants.len(),
                    scenarios: r.scenarios.len(),
                },
            )
        })
        .collect()
}

/// The text with each line's comment prefix and indentation dropped and
/// the lines joined by one space, so a reference wrapped across lines
/// reads as one; and the byte offset where each line starts in it.
fn flatten(text: &str) -> (String, Vec<usize>) {
    let mut flat = String::with_capacity(text.len());
    let mut starts = Vec::new();
    for line in text.lines() {
        starts.push(flat.len());
        let mut rest = line.trim_start();
        for prefix in ["///", "//!", "//", "#", "*", ">"] {
            if let Some(after) = rest.strip_prefix(prefix) {
                rest = after.trim_start();
                break;
            }
        }
        flat.push_str(rest.trim_end());
        flat.push(' ');
    }
    (flat, starts)
}

fn line_at(starts: &[usize], offset: usize) -> usize {
    starts.partition_point(|&s| s <= offset)
}

/// The kind a word names: `Some(true)` for the new vocabulary, exactly
/// `statement`, `invariant`, `scenario` (or the plural); `Some(false)`
/// for any old or miscased word; `None` for a word that is no part word.
fn part_word(word: &str) -> Option<(Option<char>, bool)> {
    let lower = word.to_ascii_lowercase();
    let singular = lower.strip_suffix('s').unwrap_or(&lower);
    let kind = match singular {
        "statement" => 's',
        "invariant" => 'i',
        "scenario" => 'c',
        "item" | "rule" => 'o',
        _ if lower == "r" => 'o',
        _ => return None,
    };
    let new = kind != 'o'
        && matches!(
            word,
            "statement" | "statements" | "invariant" | "invariants" | "scenario" | "scenarios"
        );
    Some((if kind == 'o' { None } else { Some(kind) }, new))
}

fn is_id_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_'
}

/// The numbers after a part word: `5`, `5, 11 and 13`, `27-30`.
/// Returns each (low, high) and the chars consumed.
fn number_list(tail: &[char], start: usize) -> (Vec<(usize, usize)>, usize) {
    let mut at = start;
    let mut out = Vec::new();
    loop {
        // One element: digits, optionally `-` or an en dash and digits.
        let mut j = at;
        while j < tail.len() && tail[j].is_ascii_digit() {
            j += 1;
        }
        let len = j - at;
        if len == 0 || (!out.is_empty() && len > 3) || len > 4 {
            break;
        }
        let low: usize = tail[at..j].iter().collect::<String>().parse().unwrap_or(0);
        let mut high = low;
        let mut end = j;
        if end + 1 < tail.len()
            && (tail[end] == '-' || tail[end] == '–')
            && tail[end + 1].is_ascii_digit()
        {
            let mut k = end + 1;
            while k < tail.len() && tail[k].is_ascii_digit() {
                k += 1;
            }
            if k - (end + 1) <= 3 {
                high = tail[end + 1..k]
                    .iter()
                    .collect::<String>()
                    .parse()
                    .unwrap_or(0);
                end = k;
            }
        }
        // A date or a number glued to letters is no part number.
        if !out.is_empty()
            && end < tail.len()
            && (tail[end].is_ascii_alphanumeric()
                || (tail[end] == '-' && end + 1 < tail.len() && tail[end + 1].is_ascii_digit()))
        {
            break;
        }
        out.push((low, high));
        at = end;
        // A separator and another number: `, `, ` and `, `, and `, ` or `.
        let mut k = at;
        let mut sep = false;
        for word in [", and ", ", ", " and ", " or "] {
            let w: Vec<char> = word.chars().collect();
            if tail.len() >= k + w.len() && tail[k..k + w.len()] == w[..] {
                k += w.len();
                sep = true;
                break;
            }
        }
        if !sep || k >= tail.len() || !tail[k].is_ascii_digit() {
            break;
        }
        at = k;
    }
    (out, at)
}

/// Scan `text`; returns the references checked and the problems found.
fn scan(text: &str, at: &str, parts: &BTreeMap<String, Parts>) -> (usize, Vec<String>) {
    let (flat, starts) = flatten(text);
    let mut checked = 0usize;
    let mut problems = Vec::new();
    for (pos, _) in flat.match_indices("BS-") {
        if flat[..pos].chars().next_back().is_some_and(is_id_char) {
            continue;
        }
        let rest: Vec<char> = flat[pos + 3..].chars().take(240).collect();
        // BS-<area>-<n>
        let mut i = 0;
        while i < rest.len() && (rest[i].is_ascii_lowercase() || rest[i].is_ascii_digit()) {
            i += 1;
        }
        if i == 0 || i >= rest.len() || rest[i] != '-' {
            continue;
        }
        let area: String = rest[..i].iter().collect();
        i += 1;
        let digits_from = i;
        while i < rest.len() && rest[i].is_ascii_digit() {
            i += 1;
        }
        if i == digits_from {
            continue;
        }
        let number: String = rest[digits_from..i].iter().collect();
        let id = format!("BS-{area}-{number}");
        let line = line_at(&starts, pos);
        // Optional 's, optional comma, spaces, optional parenthesis.
        if rest.get(i) == Some(&'\'') && rest.get(i + 1) == Some(&'s') {
            i += 2;
        }
        if rest.get(i) == Some(&',') {
            i += 1;
        }
        let spaces_from = i;
        while rest.get(i) == Some(&' ') {
            i += 1;
        }
        if i == spaces_from {
            continue;
        }
        if rest.get(i) == Some(&'(') {
            i += 1;
        }
        // A reference, then continuations on the same id.
        let mut first = true;
        loop {
            let word_from = i;
            while i < rest.len() && rest[i].is_ascii_alphabetic() {
                i += 1;
            }
            let word: String = rest[word_from..i].iter().collect();
            let Some((kind, new)) = part_word(&word) else {
                break;
            };
            if rest.get(i) != Some(&' ') || !rest.get(i + 1).is_some_and(|c| c.is_ascii_digit()) {
                break;
            }
            let (nums, end) = number_list(&rest, i + 1);
            let shown: String = rest[word_from..end].iter().collect();
            if !new {
                problems.push(format!(
                    "{at}:{line}: {id} {shown}: the old vocabulary; say `statement`, `invariant` or `scenario` k, as bookspec numbers its parts"
                ));
            } else if let Some(p) = parts.get(&id) {
                let (have, name) = match kind {
                    Some('s') => (p.statements, "statements"),
                    Some('i') => (p.invariants, "invariants"),
                    _ => (p.scenarios, "scenarios"),
                };
                for (low, high) in &nums {
                    checked += 1;
                    if *high != *low {
                        checked += 1;
                    }
                    if *low < 1 || *high < 1 || *low > have || *high > have {
                        problems.push(format!(
                            "{at}:{line}: {id} {shown}: the record has {have} {name}"
                        ));
                    }
                }
            } else {
                checked += nums.len();
                problems.push(format!("{at}:{line}: {id} {shown}: no such rule"));
            }
            i = end;
            // `, and`, `and`, `with`, `,` and then another part word.
            let mut k = i;
            let mut moved = false;
            for sep in [", and ", ", ", " and ", " with "] {
                let w: Vec<char> = sep.chars().collect();
                if rest.len() >= k + w.len() && rest[k..k + w.len()] == w[..] {
                    k += w.len();
                    moved = true;
                    break;
                }
            }
            if !moved {
                break;
            }
            i = k;
            first = false;
        }
        let _ = first;
    }
    (checked, problems)
}

fn tracked_text_files(root: &Path) -> Vec<(String, String)> {
    let output = std::process::Command::new("git")
        .args(["ls-files", "-z"])
        .current_dir(root)
        .output()
        .expect("git ls-files");
    assert!(output.status.success(), "git ls-files succeeds");
    let listing = String::from_utf8_lossy(&output.stdout).into_owned();
    let mut files = Vec::new();
    for rel in listing.split('\0').filter(|s| !s.is_empty()) {
        if rel.starts_with("plan/") || rel == SELF {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(root.join(rel)) else {
            continue;
        };
        if text.contains('\0') {
            continue;
        }
        files.push((rel.to_string(), text));
    }
    files
}

#[test]
pub(crate) fn every_sub_reference_names_a_part_its_record_has() {
    let parts = record_parts();
    assert!(!parts.is_empty(), "the pinned bookspec has records");
    let root = repo_root();
    let mut checked = 0usize;
    let mut bad = Vec::new();
    for (rel, text) in tracked_text_files(&root) {
        let (n, problems) = scan(&text, &rel, &parts);
        checked += n;
        bad.extend(problems);
    }
    println!("sub-references checked: {checked}");
    assert!(
        checked >= CHECKED_FLOOR,
        "sub-references were found ({checked}, floor {CHECKED_FLOOR})"
    );
    assert!(
        bad.is_empty(),
        "every sub-reference names a part its record has, in bookspec's words:\n{}",
        bad.join("\n")
    );
}

fn problems_in(text: &str) -> Vec<String> {
    scan(text, "t", &record_parts()).1
}

#[test]
pub(crate) fn a_statement_the_record_lacks_fails() {
    // BS-entry-1 has four statements, four invariants and one scenario.
    assert_eq!(problems_in("BS-entry-1 statement 99").len(), 1);
    assert_eq!(problems_in("BS-entry-1 statement 5").len(), 1);
    assert_eq!(problems_in("BS-entry-1 statement 0").len(), 1);
    assert_eq!(problems_in("BS-entry-1 invariant 5").len(), 1);
    assert_eq!(problems_in("BS-entry-1 scenario 2").len(), 1);
    assert_eq!(problems_in("BS-nosuch-9 statement 1").len(), 1);
    assert_eq!(
        scan("BS-entry-1 statement 4", "t", &record_parts()),
        (1, vec![])
    );
    assert_eq!(
        scan("BS-entry-1 invariant 4", "t", &record_parts()),
        (1, vec![])
    );
    assert_eq!(
        scan("BS-entry-1 scenario 1", "t", &record_parts()),
        (1, vec![])
    );
}

#[test]
pub(crate) fn lists_ranges_wraps_and_continuations_are_each_checked() {
    let parts = record_parts();
    // Every number of a list or range, in the parentheses a comment gives.
    assert_eq!(
        scan("(BS-entry-1 statements 1, 2 and 4)", "t", &parts),
        (3, vec![])
    );
    assert_eq!(problems_in("(BS-entry-1 statements 1, 2 and 9)").len(), 1);
    assert_eq!(problems_in("BS-entry-1 statements 2-9").len(), 1);
    assert_eq!(scan("BS-entry-1 statements 1-4", "t", &parts).0, 2);
    // A reference wrapped across a comment prefix reads as one.
    assert_eq!(problems_in("/// (BS-entry-1\n/// statement 99)").len(), 1);
    assert_eq!(problems_in("//! BS-entry-1 statement\n//!   99").len(), 1);
    // A continuation on the same id is checked too, and one in the old
    // vocabulary fails.
    assert_eq!(
        scan("BS-entry-1 invariant 1 and statement 3", "t", &parts),
        (2, vec![])
    );
    assert_eq!(
        problems_in("BS-entry-1 invariant 1 and statement 30").len(),
        1
    );
    assert_eq!(problems_in("BS-entry-1 invariant 1 and item 3").len(), 1);
    // A date or a number after a list is no reference.
    assert_eq!(
        scan("BS-entry-1 statement 2, 2026-11-08", "t", &parts),
        (1, vec![])
    );
    // Prose that merely follows an id is no sub-reference.
    assert_eq!(
        scan("BS-entry-1 states that a rule holds", "t", &parts),
        (0, vec![])
    );
    assert_eq!(
        scan("BS-entry-1 rule: debits equal credits", "t", &parts),
        (0, vec![])
    );
}

#[test]
pub(crate) fn the_old_vocabulary_fails() {
    for old in [
        "BS-entry-1 item 2",
        "BS-entry-1 items 1 and 2",
        "BS-entry-1 rule 2",
        "BS-entry-1, rule 2",
        "BS-entry-1's rules 1, 2",
        "BS-entry-1 INVARIANT 2",
        "BS-entry-1 INVARIANTS 1-2",
        "BS-entry-1 SCENARIO 1",
        "BS-entry-1 r 2",
        "BS-entry-1 Statement 2",
        "(BS-entry-1 item 2)",
        "// BS-entry-1\n// INVARIANT 2",
    ] {
        assert_eq!(problems_in(old).len(), 1, "{old:?} fails");
    }
    // A product-rule id is no bookspec id, and a bare word is no reference.
    assert_eq!(problems_in("WB-entry-1 item 2").len(), 0);
    assert_eq!(problems_in("the INVARIANT 2 of kernel::settle").len(), 0);
}
