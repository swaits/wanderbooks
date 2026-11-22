//! How a picture of the window is checked, for the tests that compare one
//! (`assistants.rs`, `tabs.rs`): as the shapes the window paints, never as
//! pixels (another renderer rasterizes the same shapes differently).
//!
//! A baseline `tests/snapshots/<name>.shapes` is the FNV-1a hash of the
//! shapes' text on its first line, then the text itself, one shape to a
//! line. The hash is what is compared; the text is only there so that a
//! mismatch can say *which* shape differs and where (a hash alone said only
//! that something did, and a machine that cannot be reached, such as
//! GitHub's Windows one, then took a guess and a rerun to learn more).
//! The text costs about a hundred kilobytes a picture in the tree (it
//! compresses well, git keeps it at a tenth of that).
//!
//! `TABS_BLESS=1` rewrites a baseline; blessing is for a change that was
//! checked by eye or by hand, never to make a failure go away.

use std::path::Path;

/// How many differing shapes a mismatch prints.
const SHOWN: usize = 30;
/// How much of one shape's text a mismatch prints on each side.
const WIDTH: usize = 700;
/// How much of a shape's start a mismatch always prints.
const HEAD: usize = 260;

/// FNV-1a over the lines' bytes (no separator between lines).
pub fn hash_of(lines: &[String]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for line in lines {
        for b in line.bytes() {
            hash ^= b as u64;
            hash = hash.wrapping_mul(0x100_0000_01b3);
        }
    }
    hash
}

/// The first `HEAD` bytes of `s`, on a char edge.
fn head(s: &str) -> String {
    let mut to = HEAD.min(s.len());
    while !s.is_char_boundary(to) {
        to += 1;
    }
    format!("{}{}", &s[..to], if to < s.len() { "…" } else { "" })
}

/// A slice of `s` of about `WIDTH` bytes around byte `at`, on char edges.
fn around(s: &str, at: usize) -> String {
    let mut from = at.saturating_sub(WIDTH / 4);
    while !s.is_char_boundary(from) {
        from -= 1;
    }
    let mut to = (from + WIDTH).min(s.len());
    while !s.is_char_boundary(to) {
        to += 1;
    }
    format!(
        "{}{}{}",
        if from > 0 { "…" } else { "" },
        &s[from..to],
        if to < s.len() { "…" } else { "" }
    )
}

/// The first 30 differing lines of two texts, in words.
pub fn differences(want: &[&str], got: &[String]) -> String {
    let mut out = format!("{} shapes expected, {} painted\n", want.len(), got.len());
    let mut shown = 0;
    for i in 0..want.len().max(got.len()) {
        let (w, g) = (want.get(i).copied(), got.get(i).map(String::as_str));
        if w == g {
            continue;
        }
        if shown == SHOWN {
            out.push_str("… and more\n");
            break;
        }
        shown += 1;
        out.push_str(&format!("shape {i}:\n"));
        match (w, g) {
            (Some(w), Some(g)) => {
                // The head of a shape names it (its kind, where, and for a
                // text which text); then, where they first differ, a slice.
                let at = w.bytes().zip(g.bytes()).take_while(|(a, b)| a == b).count();
                out.push_str(&format!(
                    "  first differs at byte {at}\n  want: {}\n  got:  {}\n",
                    head(w),
                    head(g)
                ));
                if at > HEAD {
                    out.push_str(&format!(
                        "  from there:\n  want: {}\n  got:  {}\n",
                        around(w, at),
                        around(g, at)
                    ));
                }
            }
            (Some(w), None) => out.push_str(&format!("  want: {}\n  got:  (nothing)\n", head(w))),
            (None, Some(g)) => out.push_str(&format!("  want: (nothing)\n  got:  {}\n", head(g))),
            (None, None) => unreachable!(),
        }
    }
    out
}

/// Check the shapes `lines` of the picture `name` against
/// `tests/snapshots/<name>.shapes` (or write it, with `TABS_BLESS=1`).
pub fn check(name: &str, lines: &[String]) {
    let file = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/snapshots")
        .join(format!("{name}.shapes"));
    let hash = format!("{:016x}", hash_of(lines));
    if std::env::var("TABS_BLESS").is_ok() {
        let mut text = format!("{hash}\n");
        for line in lines {
            text.push_str(line);
            text.push('\n');
        }
        std::fs::write(&file, text).unwrap();
        return;
    }
    let baseline = std::fs::read_to_string(&file)
        .unwrap_or_else(|e| panic!("{}: {e} (TABS_BLESS=1 writes it)", file.display()));
    let mut parts = baseline.lines();
    let want_hash = parts.next().unwrap_or("").trim();
    if hash == want_hash {
        return;
    }
    let want: Vec<&str> = parts.collect();
    let said = if want.is_empty() {
        "the baseline holds no text of the shapes to compare with (bless it)".to_string()
    } else {
        differences(&want, lines)
    };
    panic!("the shapes of {name} changed: want {want_hash}, got {hash}\n{said}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mismatch_names_the_shapes_that_differ_and_no_more_than_thirty() {
        let want: Vec<String> = (0..50)
            .map(|i| format!("Text pos=[{i}.0 1.0] text=\"é{i}\""))
            .collect();
        let mut got = want.clone();
        got[3] = "Text pos=[3.0 9.0] text=\"é3\"".into();
        let said = differences(&want.iter().map(String::as_str).collect::<Vec<_>>(), &got);
        assert!(said.contains("50 shapes expected, 50 painted"), "{said}");
        assert!(
            said.contains("shape 3:") && said.contains("pos=[3.0 9.0]"),
            "{said}"
        );
        assert!(!said.contains("shape 4:"), "{said}");
        // Every shape differs: thirty are shown, then it says there are more.
        let all: Vec<String> = want.iter().map(|w| format!("{w} x")).collect();
        let said = differences(&want.iter().map(String::as_str).collect::<Vec<_>>(), &all);
        assert_eq!(said.matches("\nshape ").count(), 30, "{said}");
        assert!(said.contains("… and more"), "{said}");
        // A shape missing from one side is said so; a long shape is cut on a char edge.
        let long = "é".repeat(2000);
        let said = differences(&[long.as_str()], &[]);
        assert!(said.contains("got:  (nothing)"), "{said}");
    }
}
