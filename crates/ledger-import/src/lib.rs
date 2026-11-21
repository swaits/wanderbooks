//! QuickBooks-compatible import/export for wanderbooks.
//!
//! - [`bank_csv`]: parse QBO bank/credit-card upload CSVs.
//! - [`chart_csv`]: parse chart-of-accounts CSVs.
//! - [`iif`]: generate IIF journal entries for QuickBooks Desktop.
//! - [`report`]: the statement layout every renderer draws from.
//! - [`xlsx`]: generate XLSX reports.
//! - [`pdf`]: generate PDF financial statements (CPA handoff).
//! - [`asset_register`]: the fixed asset register as XLSX and PDF.
//! - [`labels`]: print asset label sheets with QR codes.
//! - [`invoice_pdf`](mod@invoice_pdf): an invoice as PDF.
//! - [`fonts`]: the bundled fonts the PDFs and the viewer share.
//!
//! All money stays `Decimal`; no floats in money paths.

pub mod asset_register;
pub mod bank_csv;
pub mod chart_csv;
/// The one reader, now the `ledger-reader` crate (D-61), kept at its old path.
pub use ledger_reader as extract;
pub mod fonts;
pub mod format;
pub mod iif;
pub mod invoice_pdf;
pub mod labels;
pub mod pdf;
pub mod report;
pub mod templates;
pub mod workpaper;
pub mod xlsx;

/// The line of `text` (the first is 1) a CSV record starts on, counted from
/// its byte offset: the csv reader reports a record's position from where
/// it began looking, before it skipped any blank lines, and a row is named
/// by the line a person sees it on.
pub(crate) fn line_of(text: &str, record: &csv::StringRecord, fallback: usize) -> u64 {
    record.position().map_or(fallback as u64, |p| {
        let bytes = text.as_bytes();
        let mut at = usize::try_from(p.byte())
            .unwrap_or(usize::MAX)
            .min(bytes.len());
        while bytes.get(at).is_some_and(|b| matches!(b, b'\n' | b'\r')) {
            at += 1;
        }
        bytes[..at].iter().filter(|b| **b == b'\n').count() as u64 + 1
    })
}

pub use asset_register::{asset_register_pdf, asset_register_xlsx};
pub use bank_csv::{
    BankCsvError, BankTransaction, DateOrder, MaskedBankImport, MaskedCell, parse_bank_csv,
    parse_bank_csv_masked, parse_bank_csv_with,
};
pub use chart_csv::{ChartRow, parse_chart_csv};
pub use extract::{ExtractError, page_count, page_png, pdf_lines, pdf_text, text_of, xlsx_text};
pub use format::round_display;
pub use iif::{
    Iif, IifAccount, IifChange, IifClass, IifError, IifField, LeftOut, MEMO_MAX, NAME_MAX,
    NUMBER_MAX, QbAccountType, iif_accounts, iif_classes, journal_to_iif,
};
pub use invoice_pdf::{
    InvoiceDoc, InvoiceItem, InvoiceParty, InvoiceStamp, InvoiceVersion, PaymentMethodDoc,
    invoice_pdf,
};
pub use labels::{AssetLabel, LabelSheet, asset_labels_pdf};
pub use pdf::{PdfError, statement_pdf};
pub use report::{Member, ReportMeta, Row, RowKind, Statement, YearEarnings, format_amount};
pub use templates::{ChartTemplate, EntityType, chart_template};
pub use workpaper::{Workpaper, WorkpaperCheck, WorkpaperFinding, workpaper_pdf};
pub use xlsx::{XlsxReport, report_to_xlsx};
