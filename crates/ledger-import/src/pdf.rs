//! PDF financial statements, typeset with Typst.
//!
//! [`crate::report`] lays a statement out as rows; this module hands those
//! rows to `templates/statement.typ` as JSON data (`sys.inputs.data`) and
//! renders the PDF. Amounts arrive already split by
//! [`crate::report::amount_parts`] — Typst never does arithmetic on money,
//! and labels are data, never markup. Fonts are bundled (`typst-assets`),
//! so output doesn't depend on what's installed, and it is deterministic:
//! the same statement renders to the same bytes.

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
}

static TEMPLATE: &str = include_str!("../templates/statement.typ");

/// The engine, built once: parsing the bundled fonts is the expensive part.
fn engine() -> &'static TypstEngine<TypstTemplateMainFile> {
    static ENGINE: OnceLock<TypstEngine<TypstTemplateMainFile>> = OnceLock::new();
    ENGINE.get_or_init(|| {
        TypstEngine::builder()
            .main_file(TEMPLATE)
            .fonts(typst_assets::fonts())
            .build()
    })
}

/// The template's input: the statement with every amount formatted.
pub fn template_data(st: &Statement, meta: &ReportMeta) -> serde_json::Value {
    let kind = |k: RowKind| match k {
        RowKind::Section => "section",
        RowKind::Line => "line",
        RowKind::Subtotal => "subtotal",
        RowKind::Total => "total",
        RowKind::Spacer => "spacer",
    };
    serde_json::json!({
        "company": meta.company,
        "title": st.title,
        "period": st.period,
        "footer": format!("Prepared from journal through seq {}", meta.journal_seq),
        "rows": st.rows.iter().map(|r| serde_json::json!({
            "kind": kind(r.kind),
            "label": r.label,
            "rule": r.rule_above,
            "amount": r.amount.map(|a| {
                let p = amount_parts(a);
                serde_json::json!({
                    "digits": p.digits,
                    "negative": p.negative,
                    "zero": p.zero,
                    "currency": r.currency,
                })
            }),
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
        let data = template_data(&profit_and_loss(&pl()), &meta());
        assert_eq!(
            data,
            serde_json::json!({
                "company": "Test Co",
                "title": "Profit and Loss",
                "period": "October 4 – October 11, 2026",
                "footer": "Prepared from journal through seq 42",
                "rows": [
                    {"kind": "section", "label": "Revenue", "rule": false, "amount": null},
                    {"kind": "subtotal", "label": "Total revenue", "rule": false, "amount":
                        {"digits": "", "negative": false, "zero": true, "currency": true}},
                    {"kind": "spacer", "label": "", "rule": false, "amount": null},
                    {"kind": "section", "label": "Expenses", "rule": false, "amount": null},
                    {"kind": "line", "label": "6000 Organizational Costs", "rule": false, "amount":
                        {"digits": "59.00", "negative": false, "zero": false, "currency": true}},
                    {"kind": "line", "label": "6400 Memberships & Licenses", "rule": false, "amount":
                        {"digits": "106.38", "negative": false, "zero": false, "currency": false}},
                    {"kind": "subtotal", "label": "Total expenses", "rule": true, "amount":
                        {"digits": "165.38", "negative": false, "zero": false, "currency": true}},
                    {"kind": "spacer", "label": "", "rule": false, "amount": null},
                    {"kind": "total", "label": "Net income", "rule": true, "amount":
                        {"digits": "165.38", "negative": true, "zero": false, "currency": true}},
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
                amount: Some(Decimal::new(i, 0)),
                currency: false,
                rule_above: false,
            })
            .collect();
        let doc = typeset(&st, &meta()).unwrap();
        assert!(doc.pages().len() > 1, "200 rows should span pages");
        assert!(statement_pdf(&st, &meta()).unwrap().starts_with(b"%PDF"));
    }
}
