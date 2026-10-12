//! QuickBooks-compatible import/export for wanderbooks.
//!
//! - [`bank_csv`]: parse QBO bank/credit-card upload CSVs.
//! - [`chart_csv`]: parse chart-of-accounts CSVs.
//! - [`iif`]: generate IIF journal entries for QuickBooks Desktop.
//! - [`xlsx`]: generate XLSX reports.
//! - [`pdf`]: generate PDF financial statements (CPA handoff).
//!
//! All money stays `Decimal`; no floats in money paths.

pub mod bank_csv;
pub mod chart_csv;
pub mod iif;
pub mod pdf;
pub mod xlsx;

pub use bank_csv::{BankTransaction, parse_bank_csv};
pub use chart_csv::{ChartRow, parse_chart_csv};
pub use iif::{IifEntry, journal_to_iif};
pub use pdf::{PdfError, balance_sheet_pdf, cash_flow_pdf, profit_and_loss_pdf};
pub use xlsx::{XlsxReport, report_to_xlsx};
