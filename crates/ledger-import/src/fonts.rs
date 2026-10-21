//! The fonts the books print in, bundled so nothing depends on what's
//! installed: Inter for the text, and a Noto fallback for each script
//! Inter lacks, so a vendor or an account named in Chinese, Japanese,
//! Korean, Arabic, Hebrew, Thai or Hindi prints as written, never as
//! empty boxes. Each font's provenance and license is in `fonts/`
//! (`Inter-LICENSE.txt`, `NotoSansCJK-LICENSE.txt`, `NotoSans-LICENSE.txt`;
//! all SIL OFL 1.1).
//!
//! ```text
//! GOAL: every name prints as the books keep it, in the PDFs and in the
//!       viewer, from one copy of each font in the binary.
//! SOURCE: DECISION (lead, P3 review round 2, 2026-10-20: CJK, Arabic,
//!         Hebrew, Thai and Devanagari; no emoji, a colour emoji font
//!         being about 10 MB).
//! INVARIANTS:
//!   1. The PDFs (crate::pdf, crate::asset_register, crate::labels) set
//!      their text in Inter, then try FALLBACKS in order, as each template
//!      names them (`font: ("Inter", "Noto Sans CJK SC", ...)`).
//!   2. The viewer (ledger-gui) registers FALLBACKS, these same bytes,
//!      after its own fonts: each font is in the binary once.
//! ```

/// Inter, the PDFs' text face, in the three weights the templates use.
pub static INTER: [&[u8]; 3] = [
    include_bytes!("../fonts/Inter-Regular.ttf"),
    include_bytes!("../fonts/Inter-SemiBold.ttf"),
    include_bytes!("../fonts/Inter-Bold.ttf"),
];

/// The fallbacks for the scripts Inter lacks, each its family name and
/// its bytes, in the order tried: Noto Sans CJK SC (a subset: every
/// character from U+1100 on that the face draws and Inter doesn't: Hangul,
/// kana, the CJK ideographs, fullwidth forms), Noto Sans Arabic UI (its
/// letters keep their dots, so a PDF's text reads back as written:
/// `fonts/NotoSans-LICENSE.txt`), Noto Sans Hebrew, Noto Sans Thai and
/// Noto Sans Devanagari (each whole, its static Regular weight). Emoji
/// have none.
pub static FALLBACKS: [(&str, &[u8]); 5] = [
    (
        "Noto Sans CJK SC",
        include_bytes!("../fonts/NotoSansCJKsc-Regular-subset.otf"),
    ),
    (
        "Noto Sans Arabic UI",
        include_bytes!("../fonts/NotoSansArabicUI-Regular.ttf"),
    ),
    (
        "Noto Sans Hebrew",
        include_bytes!("../fonts/NotoSansHebrew-Regular.ttf"),
    ),
    (
        "Noto Sans Thai",
        include_bytes!("../fonts/NotoSansThai-Regular.ttf"),
    ),
    (
        "Noto Sans Devanagari",
        include_bytes!("../fonts/NotoSansDevanagari-Regular.ttf"),
    ),
];

/// Every font the PDFs use (INVARIANT 1), as Typst takes them: the
/// bundled bytes themselves, not a copy of them.
pub(crate) fn typst_fonts() -> Vec<typst::foundations::Bytes> {
    INTER
        .iter()
        .copied()
        .chain(FALLBACKS.iter().map(|(_, bytes)| *bytes))
        .map(typst::foundations::Bytes::new)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// INVARIANT 1: every template names Inter, then every fallback, in
    /// FALLBACKS' order; and each font's family is the one it is named
    /// by.
    #[test]
    fn every_template_names_every_fallback_in_order() {
        let mut want = String::from("font: (\"Inter\"");
        for (family, _) in &FALLBACKS {
            want.push_str(&format!(", \"{family}\""));
        }
        want.push(')');
        for (name, t) in [
            ("statement.typ", include_str!("../templates/statement.typ")),
            (
                "asset_register.typ",
                include_str!("../templates/asset_register.typ"),
            ),
            ("labels.typ", include_str!("../templates/labels.typ")),
        ] {
            assert!(t.contains(&want), "{name} names {want}");
        }
        let book = typst::text::FontBook::from_fonts(
            typst_fonts()
                .into_iter()
                .flat_map(|b| typst::text::Font::iter(b).collect::<Vec<_>>())
                .collect::<Vec<_>>()
                .iter(),
        );
        for (family, _) in &FALLBACKS {
            assert!(
                book.contains_family(&family.to_lowercase()),
                "{family} is a family the fonts hold"
            );
        }
        assert!(book.contains_family("inter"));
    }
}
