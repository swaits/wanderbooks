//! PDF financial statements, typeset with Typst.
//!
//! [`crate::report`] lays a statement out as rows; this module hands those
//! rows to `templates/statement.typ` as JSON data (`sys.inputs.data`) and
//! renders the PDF. Amounts arrive already split by
//! [`crate::report::amount_parts`] — Typst never does arithmetic on money,
//! and labels are data, never markup. The fonts are bundled, so output
//! doesn't depend on what's installed, and it is deterministic: the same
//! statement and report metadata render to the same bytes. Inter sets
//! the text; the Noto fallbacks ([`crate::fonts`]) draw what Inter can't:
//! Chinese, Japanese, Korean, Arabic, Hebrew, Thai and Devanagari, so a
//! vendor or an account named in them prints as written, right-to-left
//! where it is written so, never as empty boxes. (Emoji have no font
//! here: an emoji prints as an empty box.)

use crate::report::{ReportMeta, RowKind, Statement, amount_parts};
use std::sync::OnceLock;
use thiserror::Error;
use typst::foundations::{Dict, IntoValue};
use typst_as_lib::{TypstEngine, TypstTemplateMainFile};
use typst_layout::PagedDocument;

/// PDF rendering errors.
#[derive(Debug, Error)]
pub enum PdfError {
    /// Typst failed to compile the statement.
    #[error("pdf typesetting failed: {0}")]
    Compile(String),
    /// The PDF writer failed.
    #[error("pdf save failed: {0}")]
    Save(String),
    /// No labels to print.
    #[error("no labels to print")]
    NoLabels,
    /// The skip would start past the first sheet.
    #[error("skip {skip} is out of range: a sheet has {per_sheet} labels, so skip must be 0 to {}", per_sheet - 1)]
    SkipOutOfRange {
        /// The requested skip.
        skip: usize,
        /// Labels per sheet.
        per_sheet: usize,
    },
    /// A tag could not be encoded as a QR code (too long).
    #[error("qr code failed: {0}")]
    Qr(String),
}

static TEMPLATE: &str = include_str!("../templates/statement.typ");

/// The engine, built once: parsing the bundled fonts is the expensive part.
fn engine() -> &'static TypstEngine<TypstTemplateMainFile> {
    static ENGINE: OnceLock<TypstEngine<TypstTemplateMainFile>> = OnceLock::new();
    ENGINE.get_or_init(|| {
        TypstEngine::builder()
            .main_file(TEMPLATE)
            .fonts(crate::fonts::typst_fonts())
            .build()
    })
}

/// The template's input: the statement with every amount split for
/// alignment.
pub fn template_data(st: &Statement, meta: &ReportMeta) -> serde_json::Value {
    let kind = |k: RowKind| match k {
        RowKind::Section => "section",
        RowKind::Subheading => "subheading",
        RowKind::Line => "line",
        RowKind::Subtotal => "subtotal",
        RowKind::Total => "total",
        RowKind::Spacer => "spacer",
    };
    serde_json::json!({
        "company": meta.company,
        "title": st.title,
        "period": st.period,
        "footer": meta.footer(),
        "columns": st.columns,
        "rows": st.rows.iter().map(|r| serde_json::json!({
            "kind": kind(r.kind),
            "label": r.label,
            "rule": r.rule_above,
            "currency": r.currency,
            "amounts": r.amounts.iter().enumerate().map(|(i, a)| a.map(|a| {
                if st.percent_from.is_some_and(|f| i >= f) {
                    let a = a.round_dp(1);
                    let negative = a.is_sign_negative() && !a.is_zero();
                    return serde_json::json!({
                        "digits": format!("{}%", a.abs().normalize()),
                        "negative": negative,
                        "percent": true,
                    });
                }
                let p = amount_parts(a);
                serde_json::json!({ "digits": p.digits, "negative": p.negative })
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    })
}

/// Typeset a statement into a document (pages, before PDF encoding).
fn typeset(st: &Statement, meta: &ReportMeta) -> Result<PagedDocument, PdfError> {
    let mut inputs = Dict::new();
    inputs.insert(
        "data".into(),
        template_data(st, meta).to_string().into_value(),
    );
    engine()
        .compile_with_input(inputs)
        .output
        .map_err(|e| PdfError::Compile(format!("{e:?}")))
}

/// Render a laid-out statement (see [`crate::report`]) as a PDF.
///
/// ```text
/// GOAL: a statement a CPA can print and file.
/// SOURCE: CONVENTION (layout); docs/spec/sources/irs-records.md (records
///         "capable of being processed", reproducible).
/// INVARIANTS:
///   1. Deterministic: the same statement and metadata render to the same
///      bytes.
///   2. Every label is data, never markup: a name holding Typst syntax
///      ("#panic()", "*", "$", "[") prints as text and NEVER runs.
///   3. Every row reaches the page: template_data carries each row's label,
///      kind, and amounts split by amount_parts, in order.
///   4. Every character prints as itself when a bundled font has it:
///      Chinese, Japanese, Korean, Arabic, Hebrew, Thai and Devanagari
///      from the Noto fallbacks (crate::fonts), Arabic and Hebrew right to
///      left, never an empty box.
/// ```
pub fn statement_pdf(st: &Statement, meta: &ReportMeta) -> Result<Vec<u8>, PdfError> {
    let doc = typeset(st, meta)?;
    typst_pdf::pdf(&doc, &typst_pdf::PdfOptions::default())
        .map_err(|e| PdfError::Save(format!("{e:?}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{Row, profit_and_loss};
    use chrono::NaiveDate;
    use ledger_core::reports::{ProfitAndLossStatement, StatementLine};
    use rust_decimal::Decimal;

    fn meta() -> ReportMeta {
        ReportMeta {
            company: "Test Co".to_string(),
            basis: "Accrual".to_string(),
            prepared_on: NaiveDate::from_ymd_opt(2026, 10, 11).unwrap(),
            journal_seq: 42,
        }
    }

    fn pl() -> ProfitAndLossStatement {
        let line = |code: &str, name: &str, cents| StatementLine {
            code: code.to_string(),
            name: name.to_string(),
            amount: Decimal::new(cents, 2),
        };
        ProfitAndLossStatement {
            start: NaiveDate::from_ymd_opt(2026, 10, 4).unwrap(),
            end: NaiveDate::from_ymd_opt(2026, 10, 11).unwrap(),
            revenue: vec![],
            expenses: vec![
                line("6000", "Organizational Costs", 5900),
                line("6400", "Memberships & Licenses", 10638),
            ],
            total_revenue: Decimal::ZERO,
            total_expenses: Decimal::new(16538, 2),
        }
    }

    /// The exact data the template typesets: hand-checked, like a golden.
    #[test]
    fn template_data_is_exact() {
        let amt = |digits: &str, negative: bool| serde_json::json!([{"digits": digits, "negative": negative}]);
        let data = template_data(&profit_and_loss(&pl()), &meta());
        assert_eq!(
            data,
            serde_json::json!({
                "company": "Test Co",
                "title": "Profit and Loss",
                "period": "October 4 – October 11, 2026",
                "footer": "Accrual basis · Prepared Sunday, October 11, 2026 · Journal seq 42",
                "columns": ["Total"],
                "rows": [
                    {"kind": "section", "label": "Revenue", "rule": false, "currency": false, "amounts": []},
                    {"kind": "subtotal", "label": "Total revenue", "rule": false, "currency": true, "amounts": amt("0.00", false)},
                    {"kind": "spacer", "label": "", "rule": false, "currency": false, "amounts": []},
                    {"kind": "section", "label": "Expenses", "rule": false, "currency": false, "amounts": []},
                    {"kind": "line", "label": "6000 Organizational Costs", "rule": false, "currency": false, "amounts": amt("59.00", false)},
                    {"kind": "line", "label": "6400 Memberships & Licenses", "rule": false, "currency": false, "amounts": amt("106.38", false)},
                    {"kind": "subtotal", "label": "Total expenses", "rule": true, "currency": true, "amounts": amt("165.38", false)},
                    {"kind": "spacer", "label": "", "rule": false, "currency": false, "amounts": []},
                    {"kind": "total", "label": "Net income (loss)", "rule": true, "currency": true, "amounts": amt("165.38", true)},
                ],
            })
        );
    }

    #[test]
    fn renders_a_pdf_deterministically() {
        let st = profit_and_loss(&pl());
        let a = statement_pdf(&st, &meta()).unwrap();
        assert!(a.starts_with(b"%PDF"));
        assert_eq!(a, statement_pdf(&st, &meta()).unwrap());
    }

    /// Labels are data: Typst markup characters print literally, and a
    /// statement too long for one page flows onto more instead of being cut.
    #[test]
    fn markup_is_inert_and_long_statements_paginate() {
        let mut st = profit_and_loss(&pl());
        st.rows = (0..200)
            .map(|i| Row {
                kind: RowKind::Line,
                label: format!("#{i} *not bold* $x$ <label> @ref [link] \\ #panic()"),
                amounts: vec![Some(Decimal::new(i, 0))],
                currency: false,
                rule_above: false,
            })
            .collect();
        let doc = typeset(&st, &meta()).unwrap();
        assert!(doc.pages().len() > 1, "200 rows should span pages");
        assert!(statement_pdf(&st, &meta()).unwrap().starts_with(b"%PDF"));
    }

    /// Each text run on the pages: its text, its font's family, and its
    /// glyphs' ids in that font (0 is the font's empty box, "notdef").
    fn runs(doc: &PagedDocument) -> Vec<(String, String, Vec<u16>)> {
        use typst::layout::{Frame, FrameItem};
        fn walk(frame: &Frame, out: &mut Vec<(String, String, Vec<u16>)>) {
            for (_, item) in frame.items() {
                match item {
                    FrameItem::Group(g) => walk(&g.frame, out),
                    FrameItem::Text(t) => out.push((
                        t.text.to_string(),
                        t.font.font().info().family.clone(),
                        t.glyphs.iter().map(|g| g.id).collect(),
                    )),
                    _ => {}
                }
            }
        }
        let mut out = Vec::new();
        for p in doc.pages() {
            walk(&p.frame, &mut out);
        }
        out
    }

    /// INVARIANT 4: a company, an account and a vendor named in Chinese,
    /// Japanese and Korean print in the CJK fallback, every glyph a real
    /// one; the rest stays Inter. (With Inter alone, every one of these
    /// was glyph 0, an empty box.)
    #[test]
    fn chinese_japanese_and_korean_print_as_written() {
        let mut m = meta();
        m.company = "北京 Trading 株式会社".to_string();
        let mut st = profit_and_loss(&pl());
        st.rows.push(Row {
            kind: RowKind::Line,
            label: "6100 東京 서울 ありがとう ＡＢＣ".into(),
            amounts: vec![Some(Decimal::ONE)],
            currency: false,
            rule_above: false,
        });
        let runs = runs(&typeset(&st, &m).unwrap());
        let mut seen = String::new();
        for (text, family, glyphs) in &runs {
            assert!(
                !glyphs.contains(&0),
                "{text:?} has an empty box in {family}"
            );
            let cjk = text.chars().any(|c| {
                matches!(u32::from(c), 0x1100..=0x11FF | 0x3000..=0x9FFF | 0xAC00..=0xD7AF | 0xFF00..=0xFFEF)
            });
            let ascii = text.is_ascii();
            if cjk {
                assert_eq!(family, "Noto Sans CJK SC", "{text:?}");
                seen.push_str(text);
            } else if ascii {
                assert_eq!(family, "Inter", "{text:?}");
            }
        }
        for want in ["北京", "株式会社", "東京", "서울", "ありがとう", "ＡＢＣ"] {
            assert!(seen.contains(want), "{want} printed: {runs:?}");
        }
    }

    /// INVARIANT 4 for the other bundled scripts: Arabic, Hebrew, Thai and
    /// Devanagari each print in their own Noto fallback, every glyph a
    /// real one (the right-to-left reading is checked on the PDF itself,
    /// `tests/spec_import.rs` `pdf_prints_every_bundled_script_as_written`).
    #[test]
    fn arabic_hebrew_thai_and_devanagari_print_as_written() {
        let mut m = meta();
        m.company = "شركة Example".to_string();
        let mut st = profit_and_loss(&pl());
        for label in ["مكتب", "שלום", "สวัสดี", "नमस्ते"] {
            st.rows.push(Row {
                kind: RowKind::Line,
                label: label.into(),
                amounts: vec![Some(Decimal::ONE)],
                currency: false,
                rule_above: false,
            });
        }
        let runs = runs(&typeset(&st, &m).unwrap());
        let family_of = |c: char| match u32::from(c) {
            0x0600..=0x06FF => Some("Noto Sans Arabic UI"),
            0x0590..=0x05FF => Some("Noto Sans Hebrew"),
            0x0E00..=0x0E7F => Some("Noto Sans Thai"),
            0x0900..=0x097F => Some("Noto Sans Devanagari"),
            _ => None,
        };
        for (text, family, glyphs) in &runs {
            assert!(
                !glyphs.contains(&0),
                "{text:?} has an empty box in {family}"
            );
            if let Some(want) = text.chars().find_map(family_of) {
                assert_eq!(family, want, "{text:?}");
            }
        }
        // Every letter printed in its script's font.
        for c in "شركةمكتبשלוםสวัสดีनमस्ते".chars() {
            let want = family_of(c).unwrap();
            assert!(
                runs.iter().any(|(t, f, _)| f == want && t.contains(c)),
                "{c} printed in {want}: {runs:?}"
            );
        }
    }
}
