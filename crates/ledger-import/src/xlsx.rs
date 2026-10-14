//! Generate XLSX reports with `rust_xlsxwriter` (MIT).
//!
//! Money is converted to f64 ONLY at this presentation boundary — never in
//! a money path. All accounting math stays `Decimal`; the conversion happens
//! when writing a cell, rounded to cents. Cells stay numeric (so a CPA can
//! sum them) with accounting number formats: thousands separators,
//! negatives in parentheses with the closing parenthesis hanging (so
//! decimal points align), "$" on subtotals and totals.

use crate::report::{ReportMeta, RowKind, Statement};
use chrono::NaiveDate;
use ledger_core::Ledger;
use ledger_core::reports::GeneralLedger;
use rust_decimal::Decimal;
use rust_xlsxwriter::{Format, FormatAlign, FormatBorder, Workbook, Worksheet};
use thiserror::Error;

/// Which report to render.
#[derive(Clone, Debug)]
pub enum XlsxReport<'a> {
    /// A laid-out statement (P&L, balance sheet, cash flow, trial balance,
    /// members' equity); see [`crate::report`].
    Statement(&'a Statement),
    /// The general ledger: by account, with running balances and splits.
    GeneralLedger(&'a GeneralLedger),
    /// The general journal: every posted entry in `[start, end]`, line by
    /// line, in date order.
    GeneralJournal(&'a Ledger, NaiveDate, NaiveDate),
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

/// Accounting number formats: the `_)` reserves a closing parenthesis's
/// width on positives so decimal points line up with negatives.
const PLAIN: &str = "#,##0.00_);(#,##0.00)";
const DOLLARS: &str = "$#,##0.00_);($#,##0.00)";

struct Formats {
    bold: Format,
    company: Format,
    centered: Format,
    header: Format,
    header_right: Format,
    plain: Format,
    dollars_sum: Format,
    dollars_total: Format,
    line_label: Format,
    sub_label: Format,
    note: Format,
}

impl Formats {
    fn new() -> Self {
        let bold = Format::new().set_bold();
        let ruled = |f: Format| f.set_border_top(FormatBorder::Thin);
        Self {
            company: Format::new()
                .set_font_size(14)
                .set_align(FormatAlign::Center),
            centered: Format::new().set_align(FormatAlign::Center),
            header: bold
                .clone()
                .set_font_size(9)
                .set_border_bottom(FormatBorder::Thin),
            header_right: bold
                .clone()
                .set_font_size(9)
                .set_border_bottom(FormatBorder::Thin)
                .set_align(FormatAlign::Right),
            plain: Format::new().set_num_format(PLAIN),
            dollars_sum: ruled(bold.clone().set_num_format(DOLLARS)),
            dollars_total: ruled(bold.clone().set_num_format(DOLLARS))
                .set_border_bottom(FormatBorder::Double),
            line_label: Format::new().set_indent(1),
            sub_label: Format::new().set_italic().set_indent(1),
            note: Format::new()
                .set_italic()
                .set_font_size(9)
                .set_font_color("#666666")
                .set_align(FormatAlign::Center),
            bold,
        }
    }
}

/// Company, title, and period centered over `width` columns; returns the
/// next free row.
fn header(
    sheet: &mut Worksheet,
    f: &Formats,
    meta: &ReportMeta,
    title: &str,
    period: &str,
    width: u16,
) -> Result<u32, XlsxError> {
    let last = width.saturating_sub(1);
    sheet.merge_range(0, 0, 0, last, &meta.company, &f.company)?;
    sheet.merge_range(
        1,
        0,
        1,
        last,
        title,
        &f.bold.clone().set_align(FormatAlign::Center),
    )?;
    sheet.merge_range(2, 0, 2, last, period, &f.centered)?;
    Ok(4)
}

/// Basis, preparation date, and journal seq under the last row.
fn footer(
    sheet: &mut Worksheet,
    f: &Formats,
    meta: &ReportMeta,
    row: u32,
    width: u16,
) -> Result<(), XlsxError> {
    sheet.merge_range(
        row + 1,
        0,
        row + 1,
        width.saturating_sub(1),
        &meta.footer(),
        &f.note,
    )?;
    Ok(())
}

/// Write an amount. A number cell is a binary double and Excel honors 15
/// significant digits (docs/spec/sources/xlsx-numbers.md), so an amount
/// with more than 13 digits before the cents can't show exactly: such an
/// amount is written as its exact text instead, right-aligned, so the
/// spreadsheet never shows a wrong number.
fn number(
    sheet: &mut Worksheet,
    row: u32,
    col: u16,
    d: Decimal,
    fmt: &Format,
) -> Result<(), XlsxError> {
    match exact_f64(d) {
        Some(x) => sheet.write_number_with_format(row, col, x, fmt)?,
        None => sheet.write_string_with_format(
            row,
            col,
            crate::report::format_amount(d),
            &fmt.clone().set_align(rust_xlsxwriter::FormatAlign::Right),
        )?,
    };
    Ok(())
}

/// Render a report to XLSX bytes.
///
/// ```text
/// GOAL: a statement as a spreadsheet a CPA can sum and check.
/// SOURCE: docs/spec/sources/xlsx-numbers.md (a cell number is a binary
///         double; Excel honors 15 significant digits).
/// INVARIANTS:
///   1. An amount is a number cell, in an accounting format (thousands
///      separators, negatives in parentheses), unless Excel couldn't show
///      it exactly (finer than a cent, or past 15 digits): then its exact
///      text, right-aligned. NEVER a number Excel would show wrong.
///   2. Every statement row is a row, in order, with its label.
///   3. The output is a valid XLSX (a zip holding a workbook).
/// ```
pub fn report_to_xlsx(report: &XlsxReport<'_>, meta: &ReportMeta) -> Result<Vec<u8>, XlsxError> {
    let mut workbook = Workbook::new();
    let sheet = workbook.add_worksheet();
    let f = Formats::new();
    match report {
        XlsxReport::Statement(st) => statement(sheet, &f, meta, st)?,
        XlsxReport::GeneralLedger(gl) => general_ledger(sheet, &f, meta, gl)?,
        XlsxReport::GeneralJournal(ledger, start, end) => {
            general_journal(sheet, &f, meta, ledger, *start, *end)?
        }
    }
    Ok(workbook.save_to_buffer()?)
}

fn statement(
    sheet: &mut Worksheet,
    f: &Formats,
    meta: &ReportMeta,
    st: &Statement,
) -> Result<(), XlsxError> {
    // Excel sheet names: at most 31 characters, no apostrophes at the ends.
    let name: String = st.title.replace('\'', "").chars().take(31).collect();
    sheet.set_name(name.trim())?;
    let width = 1 + st.columns.len() as u16;
    let mut row = header(sheet, f, meta, &st.title, &st.period, width)?;
    sheet.write_string_with_format(row, 0, "", &f.header)?;
    for (i, c) in st.columns.iter().enumerate() {
        sheet.write_string_with_format(row, 1 + i as u16, c.to_uppercase(), &f.header_right)?;
    }
    row += 1;
    let dollars_unruled = f.bold.clone().set_num_format(DOLLARS);
    for r in &st.rows {
        let (label_fmt, amount_fmt) = match r.kind {
            RowKind::Spacer => {
                row += 1;
                continue;
            }
            RowKind::Section => (&f.bold, &f.plain),
            RowKind::Subheading => (&f.sub_label, &f.plain),
            RowKind::Line => (&f.line_label, &f.plain),
            RowKind::Subtotal if r.rule_above => (&f.bold, &f.dollars_sum),
            RowKind::Subtotal => (&f.bold, &dollars_unruled),
            RowKind::Total => (&f.bold, &f.dollars_total),
        };
        sheet.write_string_with_format(row, 0, &r.label, label_fmt)?;
        for (i, a) in r.amounts.iter().enumerate() {
            match a {
                Some(d) => number(sheet, row, 1 + i as u16, *d, amount_fmt)?,
                None if r.rule_above => {
                    sheet.write_blank(row, 1 + i as u16, amount_fmt)?;
                }
                None => {}
            }
        }
        row += 1;
    }
    sheet.set_column_width(0, 58)?;
    for i in 0..st.columns.len() as u16 {
        sheet.set_column_width(1 + i, 16)?;
    }
    footer(sheet, f, meta, row, width)
}

fn general_ledger(
    sheet: &mut Worksheet,
    f: &Formats,
    meta: &ReportMeta,
    gl: &GeneralLedger,
) -> Result<(), XlsxError> {
    sheet.set_name("General Ledger")?;
    let period = crate::report::period_label(gl.start, gl.end);
    let cols = [
        "Date",
        "Entry",
        "Description",
        "Split",
        "Debit",
        "Credit",
        "Balance",
    ];
    let mut row = header(sheet, f, meta, "General Ledger", &period, cols.len() as u16)?;
    for (i, h) in cols.iter().enumerate() {
        let fmt = if i >= 4 { &f.header_right } else { &f.header };
        sheet.write_string_with_format(row, i as u16, h.to_uppercase(), fmt)?;
    }
    row += 1;
    for a in &gl.accounts {
        let name = format!("{} {}", a.code, a.name);
        sheet.write_string_with_format(row, 0, &name, &f.bold)?;
        row += 1;
        sheet.write_string_with_format(row, 2, "Beginning balance", &f.line_label)?;
        number(sheet, row, 6, a.beginning_balance, &f.plain)?;
        row += 1;
        for p in &a.postings {
            sheet.write_string(row, 0, p.date.to_string())?;
            sheet.write_number(row, 1, p.entry_id as f64)?;
            sheet.write_string(row, 2, &p.description)?;
            sheet.write_string(row, 3, &p.split)?;
            if !p.debit.is_zero() {
                number(sheet, row, 4, p.debit, &f.plain)?;
            }
            if !p.credit.is_zero() {
                number(sheet, row, 5, p.credit, &f.plain)?;
            }
            number(sheet, row, 6, p.balance, &f.plain)?;
            row += 1;
        }
        sheet.write_string_with_format(row, 0, format!("Total for {name}"), &f.bold)?;
        for c in 1..4 {
            sheet.write_blank(row, c, &f.dollars_sum)?;
        }
        number(sheet, row, 4, a.total_debits, &f.dollars_sum)?;
        number(sheet, row, 5, a.total_credits, &f.dollars_sum)?;
        number(sheet, row, 6, a.ending_balance, &f.dollars_sum)?;
        row += 2;
    }
    for (c, w) in [
        (0, 12.0),
        (1, 7.0),
        (2, 46.0),
        (3, 34.0),
        (4, 13.0),
        (5, 13.0),
        (6, 13.0),
    ] {
        sheet.set_column_width(c, w)?;
    }
    footer(sheet, f, meta, row, cols.len() as u16)
}

fn general_journal(
    sheet: &mut Worksheet,
    f: &Formats,
    meta: &ReportMeta,
    ledger: &Ledger,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<(), XlsxError> {
    sheet.set_name("General Journal")?;
    let period = crate::report::period_label(start, end);
    let cols = [
        "Date",
        "Entry",
        "Description",
        "Account",
        "Class",
        "Debit",
        "Credit",
    ];
    let mut row = header(
        sheet,
        f,
        meta,
        "General Journal",
        &period,
        cols.len() as u16,
    )?;
    for (i, h) in cols.iter().enumerate() {
        let fmt = if i >= 5 { &f.header_right } else { &f.header };
        sheet.write_string_with_format(row, i as u16, h.to_uppercase(), fmt)?;
    }
    row += 1;
    let mut entries: Vec<_> = ledger
        .posted_entries()
        .filter(|e| e.date() >= start && e.date() <= end)
        .collect();
    entries.sort_by_key(|e| (e.date(), e.id()));
    let (mut debits, mut credits) = (Decimal::ZERO, Decimal::ZERO);
    for entry in entries {
        for line in entry.lines() {
            let account = ledger
                .chart()
                .get(line.account)
                .map(|a| format!("{} {}", a.code, a.name))
                .unwrap_or_else(|| "?".to_string());
            let class = line
                .class_id
                .and_then(|cid| ledger.get_class(cid))
                .map(|c| c.name.as_str())
                .unwrap_or("");
            sheet.write_string(row, 0, entry.date().to_string())?;
            sheet.write_number(row, 1, entry.id() as f64)?;
            sheet.write_string(row, 2, entry.description())?;
            sheet.write_string(row, 3, &account)?;
            sheet.write_string(row, 4, class)?;
            match line.side {
                ledger_core::Side::Debit => {
                    number(sheet, row, 5, line.amount.amount, &f.plain)?;
                    debits += line.amount.amount;
                }
                ledger_core::Side::Credit => {
                    number(sheet, row, 6, line.amount.amount, &f.plain)?;
                    credits += line.amount.amount;
                }
            }
            row += 1;
        }
    }
    sheet.write_string_with_format(row, 0, "Total", &f.bold)?;
    for c in 1..5 {
        sheet.write_blank(row, c, &f.dollars_total)?;
    }
    number(sheet, row, 5, debits, &f.dollars_total)?;
    number(sheet, row, 6, credits, &f.dollars_total)?;
    for (c, w) in [
        (0, 12.0),
        (1, 7.0),
        (2, 50.0),
        (3, 38.0),
        (4, 12.0),
        (5, 13.0),
        (6, 13.0),
    ] {
        sheet.set_column_width(c, w)?;
    }
    footer(sheet, f, meta, row + 1, cols.len() as u16)
}

/// Decimal to f64 at the presentation boundary. Rounded to cents first.
/// The double a cell holds for `d`, when Excel will show it exactly: whole
/// cents, at most 15 significant digits. None otherwise.
fn exact_f64(d: Decimal) -> Option<f64> {
    use rust_decimal::prelude::ToPrimitive;
    let cents = d.round_dp(2);
    if cents != d || cents.abs() > Decimal::new(999_999_999_999_999, 2) {
        return None;
    }
    cents.to_f64()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report;
    use ledger_core::reports::{TrialBalance, TrialBalanceLine};

    proptest::proptest! {
        /// Oracle: docs/spec/sources/xlsx-numbers.md (a cell is a binary
        /// double; Excel honors 15 significant digits). Every whole-cent
        /// amount up to 15 digits comes back exactly when shown to the
        /// cent; anything finer than a cent, or longer, is not written as a
        /// number at all.
        #[test]
        fn a_cent_amount_shows_exactly_or_is_written_as_text(
            cents in -999_999_999_999_999i64..=999_999_999_999_999,
            extra in 1i64..10,
        ) {
            let d = Decimal::new(cents, 2);
            let x = exact_f64(d).expect("15 digits fit");
            proptest::prop_assert_eq!(format!("{x:.2}"), format!("{d:.2}"));
            proptest::prop_assert!(exact_f64(Decimal::new(cents * 10 + extra, 3)).is_none());
            let big = Decimal::new(999_999_999_999_999, 2) + Decimal::new(extra, 2);
            proptest::prop_assert!(exact_f64(big).is_none());
            proptest::prop_assert!(exact_f64(-big).is_none());
        }
    }

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
            basis: "Accrual".to_string(),
            prepared_on: NaiveDate::from_ymd_opt(2026, 10, 11).unwrap(),
            journal_seq: 7,
        };
        let st = report::trial_balance(&tb, NaiveDate::from_ymd_opt(2026, 10, 11).unwrap());
        let bytes = report_to_xlsx(&XlsxReport::Statement(&st), &meta).unwrap();
        // XLSX is a ZIP: starts with PK.
        assert_eq!(&bytes[0..2], b"PK");
        assert!(bytes.len() > 1000);
    }
}
