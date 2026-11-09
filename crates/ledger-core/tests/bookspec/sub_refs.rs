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
//! `scenario`) after a bookspec id fails, whether a number follows it or
//! the word stands bare (`BS-arap-3 SCENARIOS: ...`, on the id's line).
//!
//! A product rule's id (`WB-<area>-<n>`) takes the same check against the
//! parts its own claim states: the numbered `INVARIANTS:` and
//! `SCENARIOS:` lists of its spec block in `docs/spec/claims/*.md`. A
//! product rule with no such list takes no sub-number, and has no
//! statements (those are bookspec's).
//!
//! A bare `Rule n` or `rule n` is refused in a file that is the cash-flow
//! statement's or a party's throughout, and elsewhere on a line that
//! cites `BS-statement-3` or `BS-partner-3`: bookspec unit S-C8 dropped
//! those two records' old labels, which had drifted from the statements'
//! positions, and a reference is now `BS-statement-3 statement k` (or
//! `BS-partner-3 statement k`). An ordinary use of the word, with no
//! number or away from a citation, is not a label.
//!
//! A reference may be a list ("statements 5, 11 and 13"), a range
//! ("statements 27-30"), parenthesised, wrapped across lines behind a
//! comment prefix, backticked after the id (`` `BS-asset-1` statements
//! 23-24 ``), or continued on the same id ("invariant 2 and statement
//! 11"); each number, each member of a range, is one reference checked.
//! The count is floored and printed. This file is not scanned: its
//! self-tests quote the forms they must refuse.

use crate::*;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// How many references the scan must find, at least: exactly the number
/// found when it was last set, so none can vanish silently. Raise it, in
/// the same commit, when references are added; it never falls.
const CHECKED_FLOOR: usize = 515;

/// The tracked file this check does not scan: its own self-tests quote
/// the forms it refuses.
const SELF: &str = "crates/ledger-core/tests/bookspec/sub_refs.rs";

/// The files that are the cash-flow statement's or a party's throughout.
/// BS-statement-3 and BS-partner-3 used to open their statements with
/// "Rule n" labels that drifted from their positions; bookspec unit S-C8
/// dropped them, and every reference is now a position. A numbered
/// "rule n" in one of these files is that old label, so it is refused
/// even when the line does not name the record. Elsewhere a bare label is
/// refused only beside a citation of one of those two records, so an
/// ordinary use of the word ("this rule holds", "the run rule 0.02") is
/// left alone.
const OLD_LABEL_FILES: &[&str] = &[
    "crates/ledger-core/src/cashflow.rs",
    "crates/ledger-core/tests/spec_cash_flow.rs",
    "crates/ledger-core/tests/spec_reports.rs",
    "crates/ledger-core/src/parties.rs",
    "crates/ledger-core/tests/spec_parties.rs",
    "crates/ledger-core/tests/spec_party_totals.rs",
    "crates/ledger-core/src/shares.rs",
    "crates/ledger-core/src/k1.rs",
    "crates/ledger-cli/tests/spec_parties.rs",
    "crates/ledger-cli/tests/spec_party_totals.rs",
];

/// The byte offsets of bare old labels: `Rule n`, `rule n`, `rules n`,
/// with a number after. A label straight after a bookspec id
/// ("BS-arap-2 rule 3") is the id scan's finding, not this one's.
fn bare_old_labels(flat: &str) -> Vec<usize> {
    let chars: Vec<char> = flat.chars().collect();
    let offsets: Vec<usize> = flat.char_indices().map(|(b, _)| b).collect();
    let mut out = Vec::new();
    for i in 0..chars.len() {
        if i > 0 && is_id_char(chars[i - 1]) {
            continue;
        }
        if chars[i] != 'r' && chars[i] != 'R' {
            continue;
        }
        let head: String = chars[i..(i + 4).min(chars.len())].iter().collect();
        if head != "rule" && head != "Rule" {
            continue;
        }
        let mut j = i + 4;
        if matches!(chars.get(j), Some('s' | 'S')) {
            j += 1;
        }
        // At least one space, then a number.
        let start = j;
        while chars.get(j) == Some(&' ') {
            j += 1;
        }
        if j > start && chars.get(j).is_some_and(|c| c.is_ascii_digit()) {
            out.push(offsets[i]);
        }
    }
    out
}

/// The parts a record has: its statements, invariants and scenarios.
#[derive(Clone, Copy, Default)]
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

/// The parts each product rule `WB-<area>-<n>` states in
/// `docs/spec/claims/*.md`: under its `**WB-…**` paragraph, the numbered
/// items (`  1. …`) below `INVARIANTS:` and below `SCENARIOS:`. A list
/// must count 1, 2, 3, … : an irregular one is a failure, not a guess.
fn product_rule_parts(root: &Path) -> BTreeMap<String, Parts> {
    let dir = root.join("docs/spec/claims");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("docs/spec/claims exists")
        .map(|e| e.expect("a directory entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .collect();
    files.sort();
    let mut out: BTreeMap<String, Parts> = BTreeMap::new();
    for file in files {
        let text = std::fs::read_to_string(&file).expect("a claims file reads");
        let mut current: Option<String> = None;
        // 'i' under INVARIANTS:, 'c' under SCENARIOS:, none elsewhere.
        let mut mode: Option<char> = None;
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("**WB-") {
                let id: String = rest.chars().take_while(|c| *c != '*').collect();
                current = Some(format!("WB-{id}"));
                out.entry(format!("WB-{id}")).or_default();
                mode = None;
                continue;
            }
            if line.starts_with('#') {
                current = None;
                mode = None;
                continue;
            }
            let Some(id) = &current else { continue };
            if line == "INVARIANTS:" {
                mode = Some('i');
            } else if line == "SCENARIOS:" {
                mode = Some('c');
            } else if line.starts_with("```")
                || line.split_once(':').is_some_and(|(head, _)| {
                    !head.is_empty() && head.chars().all(|c| c.is_ascii_uppercase())
                })
            {
                mode = None;
            } else if let Some(kind) = mode {
                let digits: String = line
                    .strip_prefix("  ")
                    .unwrap_or("")
                    .chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect();
                if !digits.is_empty() && line[2 + digits.len()..].starts_with(". ") {
                    let p = out.get_mut(id).expect("the rule was entered");
                    let count = if kind == 'i' {
                        &mut p.invariants
                    } else {
                        &mut p.scenarios
                    };
                    assert_eq!(
                        digits.parse::<usize>().ok(),
                        Some(*count + 1),
                        "{}: {id}'s numbered list counts 1, 2, 3, ...: {line:?}",
                        file.display()
                    );
                    *count += 1;
                }
            }
        }
    }
    out
}

/// Every id a part can be cited on: bookspec's records and the product
/// rules of the claims.
fn all_parts() -> BTreeMap<String, Parts> {
    let mut parts = record_parts();
    parts.extend(product_rule_parts(&repo_root()));
    parts
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

/// How many references a list of (low, high) numbers is: each number, each
/// member of a range, once. A range that runs backwards is one.
fn count_of(nums: &[(usize, usize)]) -> usize {
    nums.iter()
        .map(|&(low, high)| if high >= low { high - low + 1 } else { 1 })
        .sum()
}

/// Scan `text`; returns the references checked and the problems found.
fn scan(text: &str, at: &str, parts: &BTreeMap<String, Parts>) -> (usize, Vec<String>) {
    let (flat, starts) = flatten(text);
    let mut checked = 0usize;
    let mut problems = Vec::new();
    // The offsets of old labels the id scan already reports.
    let mut consumed: BTreeSet<usize> = BTreeSet::new();
    let mut hits: Vec<(usize, &str)> = ["BS-", "WB-"]
        .iter()
        .flat_map(|prefix| flat.match_indices(prefix).map(|(pos, _)| (pos, *prefix)))
        .collect();
    hits.sort();
    for (pos, prefix) in hits {
        if flat[..pos].chars().next_back().is_some_and(is_id_char) {
            continue;
        }
        let after = &flat[pos + 3..];
        let rest: Vec<char> = after.chars().take(240).collect();
        let offsets: Vec<usize> = after.char_indices().map(|(b, _)| b).take(240).collect();
        // <BS|WB>-<area>-<n>
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
        let id = format!("{prefix}{area}-{number}");
        let line = line_at(&starts, pos);
        // Optional closing backtick, optional 's, optional comma, spaces,
        // optional filler `states`, optional parenthesis.
        while rest.get(i) == Some(&'`') {
            i += 1;
        }
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
        for filler in ["states ", "state "] {
            let w: Vec<char> = filler.chars().collect();
            if rest.len() >= i + w.len() && rest[i..i + w.len()] == w[..] {
                i += w.len();
                break;
            }
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
                // A bare spec-block word (`BS-arap-3 SCENARIOS: ...`) on
                // the id's own line is the old vocabulary with no number.
                let shown_line = offsets
                    .get(word_from)
                    .map(|b| line_at(&starts, pos + 3 + b));
                let upper = word.len() > 1 && word == word.to_ascii_uppercase();
                if first && kind.is_some() && !new && upper && shown_line == Some(line) {
                    problems.push(format!(
                        "{at}:{line}: {id} {word}: the old vocabulary; say `statements`, `invariants` or `scenarios` k-m, as bookspec numbers its parts"
                    ));
                }
                break;
            }
            let (nums, end) = number_list(&rest, i + 1);
            let shown: String = rest[word_from..end].iter().collect();
            if !new {
                if let Some(b) = offsets.get(word_from) {
                    consumed.insert(pos + 3 + b);
                }
                problems.push(format!(
                    "{at}:{line}: {id} {shown}: the old vocabulary; say `statement`, `invariant` or `scenario` k, as bookspec numbers its parts"
                ));
            } else if let Some(p) = parts.get(&id) {
                let (have, name) = match kind {
                    Some('s') => (p.statements, "statements"),
                    Some('i') => (p.invariants, "invariants"),
                    _ => (p.scenarios, "scenarios"),
                };
                checked += count_of(&nums);
                for (low, high) in &nums {
                    if *low < 1 || *high < *low || *high > have {
                        problems.push(format!(
                            "{at}:{line}: {id} {shown}: the record has {have} {name}"
                        ));
                    }
                }
            } else {
                checked += count_of(&nums);
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
    }
    // A bare old label: `Rule 11` or `rule 11` for a record that numbers
    // its parts by position. Refused in a file that is the cash-flow
    // statement's or a party's throughout, and elsewhere only beside a
    // BS-statement-3 or BS-partner-3 citation on the same line, so
    // ordinary uses of the word ("this rule holds", "the run rule 0.02")
    // are not caught.
    let scoped = OLD_LABEL_FILES.contains(&at);
    let lines: Vec<&str> = text.lines().collect();
    for pos in bare_old_labels(&flat) {
        if consumed.contains(&pos) {
            continue;
        }
        let line = line_at(&starts, pos);
        let beside = lines
            .get(line - 1)
            .is_some_and(|l| l.contains("BS-statement-3") || l.contains("BS-partner-3"));
        if scoped || beside {
            let shown: String = flat[pos..]
                .chars()
                .take_while(|c| {
                    c.is_ascii_alphanumeric() || c.is_ascii_whitespace() || matches!(c, '\'' | ',')
                })
                .take(24)
                .collect();
            problems.push(format!(
                "{at}:{line}: {}: a bare old rule label; a record numbers its parts by position, so say `BS-statement-3 statement k` or `BS-partner-3 statement k`",
                shown.trim()
            ));
        }
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
    let parts = all_parts();
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
    scan(text, "t", &all_parts()).1
}

#[test]
pub(crate) fn a_statement_the_record_lacks_fails() {
    // The bounds come from the pinned record, so a record that gains or
    // loses a part moves them with it (BS-entry-1 lost a statement and its
    // scenario in X-H1c; a hard-coded four would have broken here).
    let parts = all_parts();
    let id = "BS-arap-2";
    let p = parts[id];
    let (s, i, c) = (p.statements, p.invariants, p.scenarios);
    assert!(s > 1 && i > 1 && c > 1, "{id} has every kind of part");
    for text in [
        format!("{id} statement {}", s + 50),
        format!("{id} statement {}", s + 1),
        format!("{id} statement 0"),
        format!("{id} invariant {}", i + 1),
        format!("{id} scenario {}", c + 1),
        "BS-nosuch-9 statement 1".to_string(),
    ] {
        assert_eq!(problems_in(&text).len(), 1, "{text}");
    }
    for text in [
        format!("{id} statement {s}"),
        format!("{id} invariant {i}"),
        format!("{id} scenario {c}"),
    ] {
        assert_eq!(scan(&text, "t", &parts), (1, vec![]), "{text}");
    }
}

#[test]
pub(crate) fn lists_ranges_wraps_and_continuations_are_each_checked() {
    let parts = all_parts();
    // Every number of a list or range, in the parentheses a comment gives.
    assert_eq!(
        scan("(BS-arap-2 statements 1, 2 and 4)", "t", &parts),
        (3, vec![])
    );
    assert_eq!(problems_in("(BS-arap-2 statements 1, 2 and 99)").len(), 1);
    assert_eq!(problems_in("BS-arap-2 statements 2-99").len(), 1);
    // A range counts each of its members; a list each of its numbers.
    assert_eq!(scan("BS-arap-2 statements 1-4", "t", &parts), (4, vec![]));
    assert_eq!(
        scan("BS-arap-2 statements 1-2 and 4", "t", &parts),
        (3, vec![])
    );
    assert_eq!(problems_in("BS-arap-2 statements 3-2").len(), 1);
    assert_eq!(problems_in("BS-nosuch-9 statements 1-3").len(), 1);
    assert_eq!(scan("BS-nosuch-9 statements 1-3", "t", &parts).0, 3);
    // A reference wrapped across a comment prefix reads as one.
    assert_eq!(problems_in("/// (BS-arap-2\n/// statement 99)").len(), 1);
    assert_eq!(problems_in("//! BS-arap-2 statement\n//!   99").len(), 1);
    // A continuation on the same id is checked too, and one in the old
    // vocabulary fails.
    assert_eq!(
        scan("BS-arap-2 invariant 1 and statement 3", "t", &parts),
        (2, vec![])
    );
    assert_eq!(
        problems_in("BS-arap-2 invariant 1 and statement 99").len(),
        1
    );
    assert_eq!(problems_in("BS-arap-2 invariant 1 and item 3").len(), 1);
    // A date or a number after a list is no reference.
    assert_eq!(
        scan("BS-arap-2 statement 2, 2026-11-08", "t", &parts),
        (1, vec![])
    );
    // Prose that merely follows an id is no sub-reference.
    assert_eq!(
        scan("BS-arap-2 states that a rule holds", "t", &parts),
        (0, vec![])
    );
    assert_eq!(
        scan("BS-arap-2 rule: debits equal credits", "t", &parts),
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
    // A bare word with no number is no better.
    assert_eq!(problems_in("BS-arap-3 SCENARIOS: Net 30, Net 15").len(), 1);
    assert_eq!(
        problems_in("/// BS-arap-3 INVARIANTS: the due date").len(),
        1
    );
    assert_eq!(problems_in("BS-arap-3 SCENARIO").len(), 1);
    // But only on the id's own line, as the text of a spec block runs on
    // from its SOURCE line to the next heading.
    assert_eq!(
        problems_in("SOURCE: BS-arap-3\nSCENARIOS:\n  1. a").len(),
        0
    );
    assert_eq!(problems_in("SOURCE: BS-arap-3.\nINVARIANTS:").len(), 0);
}

#[test]
pub(crate) fn the_old_vocabulary_fails_after_a_closing_backtick() {
    for old in [
        "`BS-asset-1` rules 23-24",
        "(… are `BS-asset-1` rules 23-24.)",
        "`BS-asset-1` states (rules 25-26: the dates)",
        "`BS-asset-1`'s item 3",
        "`BS-asset-1`, INVARIANT 2",
    ] {
        assert_eq!(problems_in(old).len(), 1, "{old:?} fails");
    }
    // The new vocabulary behind a backtick is checked like any other.
    let parts = all_parts();
    assert_eq!(
        scan("`BS-asset-1` statements 23-24", "t", &parts),
        (2, vec![])
    );
    assert_eq!(
        scan(
            "`BS-asset-1` states (statements 25-26: the dates)",
            "t",
            &parts
        ),
        (2, vec![])
    );
    assert_eq!(problems_in("`BS-asset-1` statements 23-99").len(), 1);
    // Prose after a backticked id is no reference.
    assert_eq!(
        scan("`BS-asset-1` states that a rule holds", "t", &parts),
        (0, vec![])
    );
}

#[test]
pub(crate) fn product_rule_parts_are_the_numbered_lists_of_their_claims() {
    let parts = all_parts();
    // WB-ledger-15 states six invariants and seven scenarios.
    let p = parts
        .get("WB-ledger-15")
        .expect("WB-ledger-15 is a product rule");
    assert_eq!((p.invariants, p.scenarios, p.statements), (6, 7, 0));
    assert_eq!(scan("WB-ledger-15 invariant 3", "t", &parts), (1, vec![]));
    assert_eq!(
        scan("WB-ledger-15 invariants 1-6 and scenario 7", "t", &parts),
        (7, vec![])
    );
    assert_eq!(problems_in("WB-ledger-15 invariant 7").len(), 1);
    assert_eq!(problems_in("WB-ledger-15 scenario 8").len(), 1);
    // A product rule has no statements of bookspec's kind.
    assert_eq!(problems_in("WB-ledger-15 statement 1").len(), 1);
    // The old vocabulary fails here too.
    assert_eq!(problems_in("WB-ledger-15 INVARIANT 3").len(), 1);
    assert_eq!(problems_in("WB-ledger-15 item 2").len(), 1);
    assert_eq!(problems_in("`WB-ledger-15` rule 2").len(), 1);
    // A product rule that states no list takes no sub-number.
    let (id, p) = parts
        .iter()
        .find(|(id, p)| id.starts_with("WB-") && p.invariants == 0 && p.scenarios == 0)
        .expect("some product rule states no list");
    assert_eq!(problems_in(&format!("{id} invariant 1")).len(), 1, "{id}");
    assert_eq!((p.statements, p.invariants), (0, 0));
    // And one that does not exist takes none either.
    assert_eq!(problems_in("WB-nosuch-9 invariant 1").len(), 1);
    // A bare product-rule id is no sub-reference.
    assert_eq!(problems_in("WB-ledger-15").len(), 0);
    assert_eq!(problems_in("the INVARIANT 2 of kernel::settle").len(), 0);
}

#[test]
pub(crate) fn a_bare_old_rule_label_is_refused_in_a_cash_flow_or_party_file() {
    let parts = all_parts();
    let cashflow = "crates/ledger-core/src/cashflow.rs";
    let parties = "crates/ledger-core/src/parties.rs";
    // In a file that is the record's throughout, a bare label fails even
    // when its line does not name the record.
    for text in [
        "/// Rule 11: what a member or lender paid",
        "    // rule 5's classes",
        "/// BS-statement-3 statement 16, the old Rule 11",
        "/// Rules 1, 3 and 4",
    ] {
        assert_eq!(scan(text, cashflow, &parts).1.len(), 1, "{text:?}");
    }
    assert_eq!(scan("/// rule 2: the target", parties, &parts).1.len(), 1);
    assert_eq!(
        scan(
            "/// rule 2: the target",
            "crates/ledger-core/tests/spec_party_totals.rs",
            &parts
        )
        .1
        .len(),
        1
    );
    // Elsewhere, a label beside a citation fails.
    assert_eq!(
        problems_in("BS-statement-3 statement 16; rule 11 was its old label").len(),
        1
    );
    assert_eq!(
        problems_in("BS-partner-3 statement 7; rule 2 was its old label").len(),
        1
    );
    // Ordinary uses of the word are not labels: no number, a number that
    // is not a label, or a bare label far from any citation.
    assert_eq!(scan("/// this rule holds", cashflow, &parts).1.len(), 0);
    assert_eq!(
        scan("/// the rules of the books", parties, &parts).1.len(),
        0
    );
    assert_eq!(
        scan("/// the run rule 0.02 / 0.00", "docs/testing.md", &parts)
            .1
            .len(),
        0
    );
    assert_eq!(
        scan("/// rule 11 is not cited here", "docs/testing.md", &parts)
            .1
            .len(),
        0
    );
    // The new form passes; a bare label straight after an id is the id
    // scan's finding, reported once.
    assert_eq!(
        scan("/// BS-statement-3 statement 16", cashflow, &parts),
        (1, vec![])
    );
    assert_eq!(problems_in("BS-statement-3 rule 16").len(), 1);
}
