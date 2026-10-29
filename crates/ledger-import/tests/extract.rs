//! The one extractor (plan P6 step 1): a PDF's text and pages, and an
//! XLSX's cells as exact decimal strings.
//!
//! Expected behavior comes from `crates/ledger-reader/src/lib.rs`'s
//! INVARIANTS, which come from plan/served.md P6 step 1 and D-15, D-32 and
//! D-53.

use std::io::{Cursor, Write};

use ledger_import::extract::{ExtractError, MAX_BYTES, page_png, pdf_text, text_of, xlsx_text};
use ledger_import::report::{ReportMeta, Row, RowKind, statement};
use ledger_import::statement_pdf;
use proptest::prelude::*;
use rust_decimal::Decimal;

fn a_pdf() -> Vec<u8> {
    let st = statement(
        "Receipt",
        "March 2026".to_string(),
        &["Total"],
        vec![Row {
            kind: RowKind::Line,
            label: "Rent".into(),
            amounts: vec![Some(Decimal::new(10000, 2))],
            currency: true,
            rule_above: false,
        }],
    );
    let meta = ReportMeta {
        company: "Fernhill Labs LLC".into(),
        basis: "Accrual".into(),
        prepared_on: chrono::NaiveDate::from_ymd_opt(2026, 3, 2).unwrap(),
        journal_seq: 1,
    };
    statement_pdf(&st, &meta).unwrap()
}

/// An XLSX holding one sheet whose cells are `values` (numbers as written,
/// not through a float).
fn xlsx_of(values: &[String]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    writer
        .start_file("xl/worksheets/sheet1.xml", options)
        .unwrap();
    let cells: String = values
        .iter()
        .enumerate()
        .map(|(i, v)| format!(r#"<c r="A{}"><v>{v}</v></c>"#, i + 1))
        .collect();
    writer
        .write_all(
            format!(
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1">{cells}</row></sheetData></worksheet>"#
            )
            .as_bytes(),
        )
        .unwrap();
    writer.finish().unwrap().into_inner()
}

/// INVARIANT 1: the same bytes give the same text and the same PNG bytes,
/// every run.
#[test]
fn extraction_deterministic() {
    let pdf = a_pdf();
    let first = pdf_text(&pdf).unwrap();
    assert_eq!(first, pdf_text(&pdf).unwrap(), "the text");
    assert!(first.contains("Rent"), "{first}");
    let first_png = page_png(&pdf, 0, 600).unwrap();
    assert_eq!(first_png, page_png(&pdf, 0, 600).unwrap(), "the page");
    assert!(first_png.starts_with(b"\x89PNG\r\n\x1a\n"), "a PNG");
    // A plain text file reads back exactly; an image has no text (D-15).
    assert_eq!(
        text_of(b"date,amount\n2026-03-01,12.50\n", "text/csv", "bank.csv").as_deref(),
        Some("date,amount\n2026-03-01,12.50\n")
    );
    assert_eq!(text_of(b"\x89PNG\r\n\x1a\n", "image/png", "r.png"), None);
}

// INVARIANT 2: an XLSX cell's number is the exact decimal string the
// sheet XML holds, never a binary float.
proptest! {
    #[test]
    fn xlsx_cells_are_decimal_strings(values in proptest::collection::vec(
        (-1_000_000_000_000i64..1_000_000_000_000i64, 0u32..=6u32), 1..12)) {
        let cells: Vec<String> = values
            .iter()
            .map(|(mantissa, scale)| Decimal::new(*mantissa, *scale).to_string())
            .collect();
        let text = xlsx_text(&xlsx_of(&cells)).unwrap();
        let got: Vec<&str> = text.split('\t').collect();
        prop_assert_eq!(got, cells.iter().map(String::as_str).collect::<Vec<_>>());
    }
}

/// INVARIANT 4 and D-32: a file the reader can't read is an error that
/// says so, not a panic out of the server.
#[test]
fn a_pdf_that_crashes_the_reader_is_failed_not_fatal() {
    for bytes in [
        &b"not a pdf at all"[..],
        &b"%PDF-1.7\n1 0 obj << /Type /Catalog"[..],
        &b""[..],
    ] {
        assert!(
            matches!(pdf_text(bytes), Err(ExtractError::Pdf(_))),
            "{bytes:?}"
        );
    }
    // The input cap is a refusal too, before the reader runs.
    let too_big = vec![0u8; MAX_BYTES + 1];
    assert!(matches!(
        pdf_text(&too_big),
        Err(ExtractError::TooLarge { .. })
    ));
    assert_eq!(text_of(&too_big, "application/pdf", "x.pdf"), None);
    // No page past the last, and no zero-sized page.
    let pdf = a_pdf();
    assert!(matches!(
        page_png(&pdf, 30, 600),
        Err(ExtractError::PageOutOfRange { .. })
    ));
    assert!(matches!(page_png(&pdf, 0, 0), Err(ExtractError::NoPixels)));
}
