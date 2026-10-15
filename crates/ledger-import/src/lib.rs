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
//!
//! All money stays `Decimal`; no floats in money paths.

pub mod asset_register;
pub mod bank_csv;
pub mod chart_csv;
pub mod iif;
pub mod labels;
pub mod pdf;
pub mod report;
pub mod templates;
pub mod xlsx;

pub use asset_register::{asset_register_pdf, asset_register_xlsx};
pub use bank_csv::{BankCsvError, BankTransaction, DateOrder, parse_bank_csv, parse_bank_csv_with};
pub use chart_csv::{ChartRow, parse_chart_csv};
pub use iif::{IifAccount, IifError, QbAccountType, iif_accounts, journal_to_iif};
pub use labels::{AssetLabel, LabelSheet, asset_labels_pdf};
pub use pdf::{PdfError, statement_pdf};
pub use report::{Member, ReportMeta, Row, RowKind, Statement, YearEarnings, format_amount};
pub use templates::{ChartTemplate, EntityType, chart_template};
pub use xlsx::{XlsxReport, report_to_xlsx};
