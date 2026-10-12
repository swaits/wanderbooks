//! PDF financial statements for the CPA handoff package.
//!
//! Simple, correct, readable — not pretty. Uses printpdf (MIT) with built-in
//! Helvetica. Amounts are exact decimals rendered as strings; no floats.

use crate::report::{ReportMeta, RowKind, Statement, format_amount};
use printpdf::{
    BuiltinFont, Color, Mm, Op, PdfDocument, PdfFontHandle, PdfPage, PdfSaveOptions, Point, Pt,
    Rgb, TextItem,
};
use thiserror::Error;

/// PDF rendering errors.
#[derive(Debug, Error)]
pub enum PdfError {
    /// printpdf failed to save.
    #[error("pdf save failed: {0}")]
    Save(String),
}

fn text_ops(
    title: &str,
    subtitle: &str,
    rows: &[(String, String)],
    totals: &[(String, String)],
) -> Vec<Op> {
    let mut ops = vec![
        Op::StartTextSection,
        Op::SetTextCursor {
            pos: Point::new(Mm(20.0), Mm(280.0)),
        },
        Op::SetFont {
            font: PdfFontHandle::Builtin(BuiltinFont::HelveticaBold),
            size: Pt(18.0),
        },
        Op::SetLineHeight { lh: Pt(22.0) },
        Op::SetFillColor {
            col: Color::Rgb(Rgb {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                icc_profile: None,
            }),
        },
        Op::ShowText {
            items: vec![TextItem::Text(title.to_string())],
        },
        Op::AddLineBreak,
        Op::SetFont {
            font: PdfFontHandle::Builtin(BuiltinFont::Helvetica),
            size: Pt(11.0),
        },
        Op::SetLineHeight { lh: Pt(14.0) },
        Op::ShowText {
            items: vec![TextItem::Text(subtitle.to_string())],
        },
        Op::AddLineBreak,
        Op::AddLineBreak,
    ];

    for (label, amount) in rows {
        // Label left, amount right-ish via padding. Monospace would be
        // nicer; Helvetica with padding is readable enough for a CPA file.
        let line = format!("{:<50} {:>14}", label, amount);
        ops.push(Op::ShowText {
            items: vec![TextItem::Text(line)],
        });
        ops.push(Op::AddLineBreak);
    }

    if !totals.is_empty() {
        ops.push(Op::AddLineBreak);
        ops.push(Op::SetFont {
            font: PdfFontHandle::Builtin(BuiltinFont::HelveticaBold),
            size: Pt(11.0),
        });
        for (label, amount) in totals {
            let line = format!("{:<50} {:>14}", label, amount);
            ops.push(Op::ShowText {
                items: vec![TextItem::Text(line)],
            });
            ops.push(Op::AddLineBreak);
        }
    }

    ops.push(Op::EndTextSection);
    ops
}

fn save_pdf(title: &str, ops: Vec<Op>) -> Result<Vec<u8>, PdfError> {
    let mut doc = PdfDocument::new(title);
    let page = PdfPage::new(Mm(210.0), Mm(297.0), ops);
    let bytes = doc
        .with_pages(vec![page])
        .save(&PdfSaveOptions::default(), &mut Vec::new());
    Ok(bytes)
}

/// Render a laid-out statement (see [`crate::report`]) as a PDF.
pub fn statement_pdf(st: &Statement, meta: &ReportMeta) -> Result<Vec<u8>, PdfError> {
    let mut rows = Vec::new();
    let mut totals = Vec::new();
    for r in &st.rows {
        let amount = r.amount.map(format_amount).unwrap_or_default();
        match r.kind {
            RowKind::Spacer => rows.push((String::new(), String::new())),
            RowKind::Section => rows.push((r.label.to_uppercase(), String::new())),
            RowKind::Line => rows.push((format!("  {}", r.label), amount)),
            RowKind::Subtotal | RowKind::Total => rows.push((r.label.clone(), amount)),
        }
    }
    totals.push((
        format!("Prepared from journal through seq {}", meta.journal_seq),
        String::new(),
    ));
    let ops = text_ops(
        &format!("{} — {}", meta.company, st.title),
        &st.period,
        &rows,
        &totals,
    );
    save_pdf(&st.title, ops)
}
