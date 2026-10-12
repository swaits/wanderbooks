//! Generate XLSX reports with `rust_xlsxwriter` (MIT).
//!
//! Money is converted to f64 ONLY at this presentation boundary — never in
//! a money path. All accounting math stays `Decimal`; the conversion happens
//! when writing a cell, rounded to cents. Cells stay numeric (so a CPA can
//! sum them) with an accounting number format: thousands separators,
//! negatives in parentheses.

use crate::report::{ReportMeta, RowKind, Statement};
use ledger_core::Ledger;
use ledger_core::reports::TrialBalance;
use rust_decimal::Decimal;
use rust_xlsxwriter::{Format, FormatBorder, Workbook, Worksheet};
use thiserror::Error;

/// Which report to render.
#[derive(Clone, Debug)]
pub enum XlsxReport<'a> {
    /// Trial balance lines.
    TrialBalance(&'a TrialBalance),
    /// A laid-out financial statement (P&L, balance sheet, cash flow); see
    /// [`crate::report`].
    Statement(&'a Statement),
    /// General ledger: every posted journal line. Needs the ledger for
    /// account names and class names.
    GeneralLedger(&'a Ledger),
}

/// XLSX errors.
#[derive(Debug, Error)]
pub enum XlsxError {
    /// The writer failed.
    #[error("xlsx write failed: {0}")]
    Write(String),
}

impl From<rust_xlsxwriter::XlsxError> for XlsxError {
    fn from(e: rust_xlsxwriter::XlsxError) -> Self {
        XlsxError::Write(e.to_string())
    }
}

const MONEY: &str = "#,##0.00;(#,##0.00)";

/// Company name, title, and period across the top; returns the next row.
fn header(
    sheet: &mut Worksheet,
    meta: &ReportMeta,
    title: &str,
    period: Option<&str>,
) -> Result<u32, XlsxError> {
    let bold = Format::new().set_bold();
    sheet.write_string_with_format(0, 0, &meta.company, &bold.clone().set_font_size(14))?;
    sheet.write_string_with_format(1, 0, title, &bold)?;
    let mut row = 2;
    if let Some(period) = period {
        sheet.write_string(row, 0, period)?;
        row += 1;
    }
    Ok(row + 1)
}

/// "Prepared from journal through seq N" under the last row.
fn footer(sheet: &mut Worksheet, meta: &ReportMeta, row: u32) -> Result<(), XlsxError> {
    let note = Format::new().set_italic().set_font_color("#666666");
    sheet.write_string_with_format(
        row + 1,
        0,
        format!("Prepared from journal through seq {}", meta.journal_seq),
        &note,
    )?;
    Ok(())
}

/// Render a report to XLSX bytes.
pub fn report_to_xlsx(report: &XlsxReport<'_>, meta: &ReportMeta) -> Result<Vec<u8>, XlsxError> {
    let mut workbook = Workbook::new();
    let sheet = workbook.add_worksheet();
    let bold = Format::new().set_bold();
    let money = Format::new().set_num_format(MONEY);
    let money_bold = money.clone().set_bold();

    match report {
        XlsxReport::TrialBalance(tb) => {
            sheet.set_name("Trial Balance")?;
            let mut row = header(sheet, meta, "Trial Balance", None)?;
            for (i, h) in ["Code", "Account", "Debit", "Credit"].iter().enumerate() {
                sheet.write_string_with_format(row, i as u16, *h, &bold)?;
            }
            row += 1;
            for line in &tb.lines {
                sheet.write_string(row, 0, &line.code)?;
                sheet.write_string(row, 1, &line.name)?;
                sheet.write_number_with_format(row, 2, dec_to_f64(line.debit), &money)?;
                sheet.write_number_with_format(row, 3, dec_to_f64(line.credit), &money)?;
                row += 1;
            }
            let total = money_bold.clone().set_border_top(FormatBorder::Thin);
            sheet.write_string_with_format(row, 1, "Total", &bold)?;
            sheet.write_number_with_format(row, 2, dec_to_f64(tb.total_debits), &total)?;
            sheet.write_number_with_format(row, 3, dec_to_f64(tb.total_credits), &total)?;
            sheet.set_column_width(1, 40)?;
            sheet.set_column_width(2, 14)?;
            sheet.set_column_width(3, 14)?;
            footer(sheet, meta, row + 1)?;
        }
        XlsxReport::Statement(st) => {
            sheet.set_name(&st.title)?;
            let mut row = header(sheet, meta, &st.title, Some(&st.period))?;
            let line_label = Format::new().set_indent(1);
            let subtotal = money_bold.clone().set_border_top(FormatBorder::Thin);
            let total = money_bold
                .clone()
                .set_border_top(FormatBorder::Thin)
                .set_border_bottom(FormatBorder::Double);
            for r in &st.rows {
                let amount = r.amount.map(dec_to_f64);
                match r.kind {
                    RowKind::Spacer => {}
                    RowKind::Section => {
                        sheet.write_string_with_format(row, 0, &r.label, &bold)?;
                    }
                    RowKind::Line => {
                        sheet.write_string_with_format(row, 0, &r.label, &line_label)?;
                        if let Some(a) = amount {
                            sheet.write_number_with_format(row, 1, a, &money)?;
                        }
                    }
                    RowKind::Subtotal => {
                        sheet.write_string_with_format(row, 0, &r.label, &bold)?;
                        if let Some(a) = amount {
                            sheet.write_number_with_format(row, 1, a, &subtotal)?;
                        }
                    }
                    RowKind::Total => {
                        sheet.write_string_with_format(row, 0, &r.label, &bold)?;
                        if let Some(a) = amount {
                            sheet.write_number_with_format(row, 1, a, &total)?;
                        }
                    }
                }
                row += 1;
            }
            sheet.set_column_width(0, 60)?;
            sheet.set_column_width(1, 16)?;
            footer(sheet, meta, row)?;
        }
        XlsxReport::GeneralLedger(ledger) => {
            sheet.set_name("General Ledger")?;
            let mut row = header(sheet, meta, "General Ledger", None)?;
            for (i, h) in [
                "Date",
                "Entry",
                "Description",
                "Account Code",
                "Account Name",
                "Class",
                "Debit",
                "Credit",
            ]
            .iter()
            .enumerate()
            {
                sheet.write_string_with_format(row, i as u16, *h, &bold)?;
            }
            row += 1;
            for entry in ledger.posted_entries() {
                for line in entry.lines() {
                    let account = ledger.chart().get(line.account);
                    let (code, name) = match account {
                        Some(a) => (a.code.as_str(), a.name.as_str()),
                        None => ("?", "?"),
                    };
                    let class_name = line
                        .class_id
                        .and_then(|cid| ledger.get_class(cid))
                        .map(|c| c.name.as_str())
                        .unwrap_or("");
                    let (debit, credit) = match line.side {
                        ledger_core::Side::Debit => (line.amount.amount, Decimal::ZERO),
                        ledger_core::Side::Credit => (Decimal::ZERO, line.amount.amount),
                    };
                    sheet.write_string(row, 0, entry.date().to_string())?;
                    sheet.write_number(row, 1, entry.id() as f64)?;
                    sheet.write_string(row, 2, entry.description())?;
                    sheet.write_string(row, 3, code)?;
                    sheet.write_string(row, 4, name)?;
                    sheet.write_string(row, 5, class_name)?;
                    sheet.write_number_with_format(row, 6, dec_to_f64(debit), &money)?;
                    sheet.write_number_with_format(row, 7, dec_to_f64(credit), &money)?;
                    row += 1;
                }
            }
            sheet.set_column_width(2, 50)?;
            sheet.set_column_width(4, 32)?;
            footer(sheet, meta, row)?;
        }
    }

    Ok(workbook.save_to_buffer()?)
}

/// Decimal to f64 at the presentation boundary. Rounded to cents first.
fn dec_to_f64(d: Decimal) -> f64 {
    use rust_decimal::prelude::ToPrimitive;
    d.round_dp(2).to_f64().unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ledger_core::reports::TrialBalanceLine;

    #[test]
    fn xlsx_is_a_zip() {
        let tb = TrialBalance {
            lines: vec![TrialBalanceLine {
                code: "1000".to_string(),
                name: "Cash".to_string(),
                debit: Decimal::new(10000, 2),
                credit: Decimal::ZERO,
            }],
            total_debits: Decimal::new(10000, 2),
            total_credits: Decimal::new(10000, 2),
        };
        let meta = ReportMeta {
            company: "Test Co".to_string(),
            journal_seq: 7,
        };
        let bytes = report_to_xlsx(&XlsxReport::TrialBalance(&tb), &meta).unwrap();
        // XLSX is a ZIP: starts with PK.
        assert_eq!(&bytes[0..2], b"PK");
        assert!(bytes.len() > 1000);
    }
}
