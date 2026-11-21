//! Words that come from outside, made safe to show.
//!
//! ```text
//! GOAL: the relay and a certificate authority are strangers: anything
//!       they send that reaches the window is short, plain and visibly
//!       quoted, so it can neither fill the window, nor hide behind
//!       invisible characters, nor pass for the app's own words.
//! SOURCE: WB-link-5 (the states in words); plan p15-app-and-relay.md 5
//!         (the relay and the authorities are not trusted).
//! INVARIANTS:
//!   1. The window's words are the app's own, chosen from the kind of
//!      failure; outside text is only ever a quoted detail at the end.
//!   2. A detail has no control character and no invisible one (the
//!      zero-width, direction-changing and tag characters, the line and
//!      paragraph separators), runs of white space are one space, and it
//!      is at most [`DETAIL_CHARS`] characters, cut with an ellipsis.
//!   3. Cleaning twice is the same as cleaning once.
//! SCENARIOS:
//!   1. A long text with a newline, a bell and a right-to-left override
//!      comes out short, on one line and with none of them.
//!   2. Text that is already clean and short comes out as it went in.
//! ```

/// The most characters of outside text shown (INVARIANT 2).
pub const DETAIL_CHARS: usize = 120;

/// Whether a character is one that must never be shown: a control
/// character, or one that draws nothing or changes how the text around it
/// reads.
fn hidden(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{00AD}'
                | '\u{034F}'
                | '\u{061C}'
                | '\u{115F}'
                | '\u{1160}'
                | '\u{17B4}'
                | '\u{17B5}'
                | '\u{180B}'..='\u{180F}'
                | '\u{200B}'..='\u{200F}'
                | '\u{2028}'..='\u{202E}'
                | '\u{2060}'..='\u{206F}'
                | '\u{3164}'
                | '\u{FE00}'..='\u{FE0F}'
                | '\u{FEFF}'
                | '\u{FFA0}'
                | '\u{FFF0}'..='\u{FFFB}'
                | '\u{1BCA0}'..='\u{1BCA3}'
                | '\u{1D173}'..='\u{1D17A}'
                | '\u{E0000}'..='\u{E0FFF}'
        )
}

/// `text` with the hidden characters gone, white space made single spaces,
/// and at most `max` characters (INVARIANT 2).
pub fn clean(text: &str, max: usize) -> String {
    let mut out = String::new();
    let mut count = 0usize;
    let mut space = false;
    for c in text.chars() {
        // A line break or tab is white space, not a character to drop: the
        // words either side of it stay apart.
        if c.is_whitespace() || matches!(c, '\n' | '\r' | '\t') {
            space = !out.is_empty();
            continue;
        }
        if hidden(c) {
            continue;
        }
        if count >= max {
            out.push('\u{2026}');
            return out;
        }
        if space {
            out.push(' ');
            count += 1;
            space = false;
        }
        out.push(c);
        count += 1;
    }
    out
}

/// Outside text as a short quoted detail (INVARIANT 1): empty when there is
/// nothing left to show.
pub fn detail(text: &str) -> String {
    let c = clean(text, DETAIL_CHARS);
    if c.is_empty() {
        String::new()
    } else {
        format!("\u{201c}{c}\u{201d}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_text_with_tricks_comes_out_short_and_plain() {
        let nasty = format!(
            "bad\u{7}\nnews\u{202e}gnp.exe\u{200b} {}",
            "word ".repeat(2000)
        );
        let d = detail(&nasty);
        assert!(d.chars().count() <= DETAIL_CHARS + 3, "{}", d.len());
        assert!(d.starts_with("\u{201c}bad news"), "{d}");
        assert!(d.ends_with("\u{2026}\u{201d}"), "{d}");
        assert!(
            !d.chars()
                .any(|c| hidden(c) || (c.is_whitespace() && c != ' '))
        );
        assert_eq!(clean(&clean(&nasty, 50), 50), clean(&nasty, 50));
    }

    #[test]
    fn short_clean_text_is_unchanged_and_nothing_is_nothing() {
        assert_eq!(
            detail("too many requests"),
            "\u{201c}too many requests\u{201d}"
        );
        assert_eq!(detail("\u{200b}\u{202e}  \n"), "");
        assert_eq!(clean("  a   b  ", 10), "a b");
    }
}
