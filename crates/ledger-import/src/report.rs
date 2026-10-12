//! The presentation model every statement renderer (XLSX, PDF) draws from.
//!
//! Core computes the statements; this module lays them out as rows —
//! sections, account lines, subtotals, totals — once, so the formats can't
//! drift apart. Amounts stay exact `Decimal`s here; [`format_amount`] is the
//! one place they become text ("1,234.56", "(207.68)").

use chrono::NaiveDate;
use ledger_core::reports::{
    BalanceSheetStatement, CashFlowLine, CashFlowStatement, ProfitAndLossStatement, StatementLine,
};
use rust_decimal::Decimal;
use serde::Serialize;

/// Who the report is for and exactly what it was prepared from.
#[derive(Clone, Debug, Serialize)]
pub struct ReportMeta {
    /// Company name for the header, e.g. "Wanderland Labs LLC".
    pub company: String,
    /// Sequence number of the last journal line the report reflects.
    pub journal_seq: u64,
}

/// What a row is, which decides how renderers style it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RowKind {
    /// A section heading ("Revenue"); no amount.
    Section,
    /// One account or one cash-flow line, indented under its section.
    Line,
    /// A section total ("Total revenue").
    Subtotal,
    /// A statement total ("Net income").
    Total,
    /// Vertical space between sections.
    Spacer,
}

/// One row of a statement.
#[derive(Clone, Debug, Serialize)]
pub struct Row {
    /// Row kind.
    pub kind: RowKind,
    /// Label text.
    pub label: String,
    /// Amount, when the row has one.
    pub amount: Option<Decimal>,
}

/// A laid-out financial statement.
#[derive(Clone, Debug, Serialize)]
pub struct Statement {
    /// "Profit and Loss", "Balance Sheet", "Statement of Cash Flows".
    pub title: String,
    /// "October 4 – October 11, 2026" or "As of October 11, 2026".
    pub period: String,
    /// The rows, top to bottom.
    pub rows: Vec<Row>,
}

fn row(kind: RowKind, label: impl Into<String>, amount: Option<Decimal>) -> Row {
    Row {
        kind,
        label: label.into(),
        amount,
    }
}

fn section(label: &str) -> Row {
    row(RowKind::Section, label, None)
}

fn spacer() -> Row {
    row(RowKind::Spacer, "", None)
}

fn account_lines(lines: &[StatementLine]) -> impl Iterator<Item = Row> + '_ {
    lines.iter().map(|l| {
        row(
            RowKind::Line,
            format!("{} {}", l.code, l.name),
            Some(l.amount),
        )
    })
}

fn flow_lines(lines: &[CashFlowLine]) -> impl Iterator<Item = Row> + '_ {
    lines
        .iter()
        .map(|l| row(RowKind::Line, l.label.clone(), Some(l.amount)))
}

/// Lay out a profit and loss statement.
pub fn profit_and_loss(pl: &ProfitAndLossStatement) -> Statement {
    let mut rows = vec![section("Revenue")];
    rows.extend(account_lines(&pl.revenue));
    rows.push(row(
        RowKind::Subtotal,
        "Total revenue",
        Some(pl.total_revenue),
    ));
    rows.push(spacer());
    rows.push(section("Expenses"));
    rows.extend(account_lines(&pl.expenses));
    rows.push(row(
        RowKind::Subtotal,
        "Total expenses",
        Some(pl.total_expenses),
    ));
    rows.push(spacer());
    rows.push(row(RowKind::Total, "Net income", Some(pl.net_income())));
    Statement {
        title: "Profit and Loss".to_string(),
        period: period_label(pl.start, pl.end),
        rows,
    }
}

/// Lay out a balance sheet.
pub fn balance_sheet(bs: &BalanceSheetStatement) -> Statement {
    let mut rows = vec![section("Assets")];
    rows.extend(account_lines(&bs.assets));
    rows.push(row(
        RowKind::Subtotal,
        "Total assets",
        Some(bs.total_assets),
    ));
    rows.push(spacer());
    rows.push(section("Liabilities"));
    rows.extend(account_lines(&bs.liabilities));
    rows.push(row(
        RowKind::Subtotal,
        "Total liabilities",
        Some(bs.total_liabilities),
    ));
    rows.push(spacer());
    rows.push(section("Equity"));
    rows.extend(account_lines(&bs.equity));
    rows.push(row(
        RowKind::Line,
        "Current earnings",
        Some(bs.current_earnings),
    ));
    rows.push(row(
        RowKind::Subtotal,
        "Total equity",
        Some(bs.total_equity),
    ));
    rows.push(spacer());
    rows.push(row(
        RowKind::Total,
        "Total liabilities and equity",
        Some(bs.total_liabilities + bs.total_equity),
    ));
    Statement {
        title: "Balance Sheet".to_string(),
        period: format!("As of {}", long_date(bs.as_of)),
        rows,
    }
}

/// Lay out a cash flow statement (indirect method).
pub fn cash_flow(cf: &CashFlowStatement) -> Statement {
    let mut rows = vec![section("Operating activities")];
    rows.push(row(RowKind::Line, "Net income", Some(cf.net_income)));
    if !cf.depreciation.is_zero() {
        rows.push(row(
            RowKind::Line,
            "Depreciation (non-cash)",
            Some(cf.depreciation),
        ));
    }
    rows.extend(flow_lines(&cf.working_capital));
    rows.push(row(
        RowKind::Subtotal,
        "Net cash from operating activities",
        Some(cf.cash_from_operations),
    ));
    rows.push(spacer());
    rows.push(section("Investing activities"));
    rows.extend(flow_lines(&cf.investing));
    rows.push(row(
        RowKind::Subtotal,
        "Net cash from investing activities",
        Some(cf.cash_from_investing),
    ));
    rows.push(spacer());
    rows.push(section("Financing activities"));
    rows.extend(flow_lines(&cf.financing));
    rows.push(row(
        RowKind::Subtotal,
        "Net cash from financing activities",
        Some(cf.cash_from_financing),
    ));
    rows.push(spacer());
    rows.push(row(
        RowKind::Total,
        "Net change in cash",
        Some(cf.net_change_in_cash),
    ));
    rows.push(row(
        RowKind::Line,
        "Cash at beginning of period",
        Some(cf.cash_beginning),
    ));
    rows.push(row(
        RowKind::Total,
        "Cash at end of period",
        Some(cf.cash_ending),
    ));
    Statement {
        title: "Statement of Cash Flows".to_string(),
        period: period_label(cf.start, cf.end),
        rows,
    }
}

/// "October 11, 2026".
pub fn long_date(d: NaiveDate) -> String {
    d.format("%B %-d, %Y").to_string()
}

/// "October 4 – October 11, 2026", or with both years when they differ.
pub fn period_label(start: NaiveDate, end: NaiveDate) -> String {
    use chrono::Datelike;
    if start.year() == end.year() {
        format!("{} – {}", start.format("%B %-d"), long_date(end))
    } else {
        format!("{} – {}", long_date(start), long_date(end))
    }
}

/// Exact accounting format: thousands separators, two decimals, negatives
/// in parentheses. "1,234.56", "(207.68)", "0.00". Never a float.
pub fn format_amount(d: Decimal) -> String {
    let negative = d.is_sign_negative() && !d.is_zero();
    let fixed = format!("{:.2}", d.abs());
    let (int, frac) = fixed.split_once('.').expect("two decimals");
    let mut grouped = String::new();
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(c);
    }
    if negative {
        format!("({grouped}.{frac})")
    } else {
        format!("{grouped}.{frac}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn dec(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap()
    }

    #[test]
    fn amounts_format_exactly() {
        assert_eq!(format_amount(dec("0")), "0.00");
        assert_eq!(format_amount(dec("-0")), "0.00");
        assert_eq!(format_amount(dec("5.8")), "5.80");
        assert_eq!(format_amount(dec("207.68")), "207.68");
        assert_eq!(format_amount(dec("-207.68")), "(207.68)");
        assert_eq!(format_amount(dec("1000")), "1,000.00");
        assert_eq!(format_amount(dec("1234567.891")), "1,234,567.89");
        assert_eq!(format_amount(dec("-100000")), "(100,000.00)");
    }

    #[test]
    fn periods_read_like_english() {
        let d = |y, m, dd| NaiveDate::from_ymd_opt(y, m, dd).unwrap();
        assert_eq!(
            period_label(d(2026, 10, 4), d(2026, 10, 11)),
            "October 4 – October 11, 2026"
        );
        assert_eq!(
            period_label(d(2026, 12, 1), d(2027, 1, 31)),
            "December 1, 2026 – January 31, 2027"
        );
    }
}
