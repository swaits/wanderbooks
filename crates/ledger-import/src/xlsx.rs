//! Generate XLSX reports with `rust_xlsxwriter` (MIT).
//!
//! Money is converted to f64 ONLY at this presentation boundary — never in
//! a money path. All accounting math stays `Decimal`; the conversion happens
//! when writing a cell, rounded to cents.

use ledger_core::reports::{BalanceSheet, CashFlowStatement, ProfitAndLoss, TrialBalance};
use rust_decimal::Decimal;
use rust_xlsxwriter::{Format, Workbook};
use thiserror::Error;

/// Which report to render.
#[derive(Clone, Debug)]
pub enum XlsxReport<'a> {
    /// Trial balance lines.
    TrialBalance(&'a TrialBalance),
    /// Profit and loss.
    ProfitAndLoss(&'a ProfitAndLoss, String, String),
    /// Balance sheet.
    BalanceSheet(&'a BalanceSheet, String),
    /// Cash flow statement.
    CashFlow(&'a CashFlowStatement),
}

/// XLSX errors.
#[derive(Debug, Error)]
pub enum XlsxError {
    /// The writer failed.
    #[error("xlsx write failed: {0}")]
    Write(String),
}

/// Render a report to XLSX bytes.
pub fn report_to_xlsx(report: &XlsxReport<'_>) -> Result<Vec<u8>, XlsxError> {
    let mut workbook = Workbook::new();
    let sheet = workbook.add_worksheet();
    let bold = Format::new().set_bold();
    let money_fmt = Format::new().set_num_format("#,##0.00");

    let mut row: u32 = 0;
    match report {
        XlsxReport::TrialBalance(tb) => {
            sheet.write_string(row, 0, "Trial Balance").unwrap();
            row += 1;
            for (i, h) in ["Code", "Account", "Debit", "Credit"].iter().enumerate() {
                sheet
                    .write_string_with_format(row, i as u16, *h, &bold)
                    .unwrap();
            }
            row += 1;
            for line in &tb.lines {
                sheet.write_string(row, 0, &line.code).unwrap();
                sheet.write_string(row, 1, &line.name).unwrap();
                sheet
                    .write_number_with_format(row, 2, dec_to_f64(line.debit), &money_fmt)
                    .unwrap();
                sheet
                    .write_number_with_format(row, 3, dec_to_f64(line.credit), &money_fmt)
                    .unwrap();
                row += 1;
            }
        }
        XlsxReport::ProfitAndLoss(pl, start, end) => {
            sheet
                .write_string(row, 0, format!("Profit and Loss ({start} to {end})"))
                .unwrap();
            row += 2;
            sheet.write_string(row, 0, "Revenue").unwrap();
            sheet
                .write_number_with_format(row, 1, dec_to_f64(pl.revenue), &money_fmt)
                .unwrap();
            row += 1;
            sheet.write_string(row, 0, "Expenses").unwrap();
            sheet
                .write_number_with_format(row, 1, dec_to_f64(pl.expenses), &money_fmt)
                .unwrap();
            row += 1;
            sheet
                .write_string_with_format(row, 0, "Net income", &bold)
                .unwrap();
            let net = pl.revenue - pl.expenses;
            sheet
                .write_number_with_format(row, 1, dec_to_f64(net), &money_fmt)
                .unwrap();
        }
        XlsxReport::BalanceSheet(bs, as_of) => {
            sheet
                .write_string(row, 0, format!("Balance Sheet (as of {as_of})"))
                .unwrap();
            row += 2;
            for (label, val) in [
                ("Assets", bs.assets),
                ("Liabilities", bs.liabilities),
                ("Equity", bs.equity),
            ] {
                sheet.write_string(row, 0, label).unwrap();
                sheet
                    .write_number_with_format(row, 1, dec_to_f64(val), &money_fmt)
                    .unwrap();
                row += 1;
            }
        }
        XlsxReport::CashFlow(cf) => {
            sheet
                .write_string(row, 0, format!("Cash Flow ({} to {})", cf.start, cf.end))
                .unwrap();
            row += 2;
            sheet
                .write_string_with_format(row, 0, "Operating activities", &bold)
                .unwrap();
            row += 1;
            sheet.write_string(row, 0, "Net income").unwrap();
            sheet
                .write_number_with_format(row, 1, dec_to_f64(cf.net_income), &money_fmt)
                .unwrap();
            row += 1;
            sheet.write_string(row, 0, "Depreciation").unwrap();
            sheet
                .write_number_with_format(row, 1, dec_to_f64(cf.depreciation), &money_fmt)
                .unwrap();
            row += 1;
            for line in &cf.working_capital {
                sheet.write_string(row, 0, &line.label).unwrap();
                sheet
                    .write_number_with_format(row, 1, dec_to_f64(line.amount), &money_fmt)
                    .unwrap();
                row += 1;
            }
            sheet
                .write_string_with_format(row, 0, "Cash from operations", &bold)
                .unwrap();
            sheet
                .write_number_with_format(row, 1, dec_to_f64(cf.cash_from_operations), &money_fmt)
                .unwrap();
            row += 2;
            sheet
                .write_string_with_format(row, 0, "Investing activities", &bold)
                .unwrap();
            row += 1;
            for line in &cf.investing {
                sheet.write_string(row, 0, &line.label).unwrap();
                sheet
                    .write_number_with_format(row, 1, dec_to_f64(line.amount), &money_fmt)
                    .unwrap();
                row += 1;
            }
            sheet
                .write_string_with_format(row, 0, "Cash from investing", &bold)
                .unwrap();
            sheet
                .write_number_with_format(row, 1, dec_to_f64(cf.cash_from_investing), &money_fmt)
                .unwrap();
            row += 2;
            sheet
                .write_string_with_format(row, 0, "Financing activities", &bold)
                .unwrap();
            row += 1;
            for line in &cf.financing {
                sheet.write_string(row, 0, &line.label).unwrap();
                sheet
                    .write_number_with_format(row, 1, dec_to_f64(line.amount), &money_fmt)
                    .unwrap();
                row += 1;
            }
            sheet
                .write_string_with_format(row, 0, "Cash from financing", &bold)
                .unwrap();
            sheet
                .write_number_with_format(row, 1, dec_to_f64(cf.cash_from_financing), &money_fmt)
                .unwrap();
            row += 2;
            sheet
                .write_string_with_format(row, 0, "Net change in cash", &bold)
                .unwrap();
            sheet
                .write_number_with_format(row, 1, dec_to_f64(cf.net_change_in_cash), &money_fmt)
                .unwrap();
        }
    }

    workbook
        .save_to_buffer()
        .map_err(|e| XlsxError::Write(e.to_string()))
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
        let bytes = report_to_xlsx(&XlsxReport::TrialBalance(&tb)).unwrap();
        // XLSX is a ZIP: starts with PK.
        assert_eq!(&bytes[0..2], b"PK");
        assert!(bytes.len() > 1000);
    }
}
