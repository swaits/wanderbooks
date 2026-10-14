//! The presentation model every statement renderer (XLSX, PDF) draws from.
//!
//! Core computes the statements; this module lays them out once — title,
//! period, column headers, and rows (sections, account lines, subtotals,
//! totals) — so the formats can't drift apart. Amounts stay exact
//! `Decimal`s here; [`format_amount`] and [`amount_parts`] are the only
//! places they become text.
//!
//! Presentation follows US small-business convention (QuickBooks-style):
//! a currency sign on subtotals and totals, a rule above every sum, a
//! double rule under statement totals, "Net income (loss)", and — for a
//! multi-member LLC — "Members' equity" with each member named.

use chrono::{Datelike, NaiveDate};
use ledger_core::RETAINED_EARNINGS_CODE;
use ledger_core::reports::{
    BalanceSheetStatement, CashFlowLine, CashFlowStatement, MembersEquityStatement,
    ProfitAndLossStatement, StatementLine, TrialBalance,
};
use rust_decimal::Decimal;
use serde::Serialize;

/// Who the report is for and exactly what it was prepared from.
#[derive(Clone, Debug, Serialize)]
pub struct ReportMeta {
    /// Company name for the header, e.g. "Wanderland Labs LLC".
    pub company: String,
    /// Accounting basis for the footer: "Accrual" or "Cash".
    pub basis: String,
    /// The day the report was prepared.
    pub prepared_on: NaiveDate,
    /// Sequence number of the last journal line the report reflects.
    pub journal_seq: u64,
}

impl ReportMeta {
    /// "Accrual basis · Prepared Sunday, October 11, 2026 · Journal seq 114".
    pub fn footer(&self) -> String {
        format!(
            "{} basis · Prepared {}, {} · Journal seq {}",
            self.basis,
            self.prepared_on.format("%A"),
            long_date(self.prepared_on),
            self.journal_seq
        )
    }
}

/// A member of a multi-member LLC, for naming capital accounts on reports.
#[derive(Clone, Debug)]
pub struct Member {
    /// The member's name.
    pub name: String,
    /// Code of their capital account.
    pub capital_code: String,
    /// Code of their distributions account, if they have one.
    pub distribution_code: Option<String>,
}

/// What a row is, which decides how renderers style it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RowKind {
    /// A section heading ("Revenue"); no amounts.
    Section,
    /// An explanatory heading inside a section; no amounts.
    Subheading,
    /// One account or one flow, indented under its section.
    Line,
    /// A section total ("Total revenue").
    Subtotal,
    /// A statement total ("Net income (loss)"); double rule beneath.
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
    /// One amount per column; `None` leaves the cell blank.
    pub amounts: Vec<Option<Decimal>>,
    /// Print the currency sign with the amounts: subtotals and totals.
    pub currency: bool,
    /// Draw a rule above the row: the "sum of the above" line. Totals
    /// always have one; a subtotal only when amounts sit above it.
    pub rule_above: bool,
}

/// A laid-out financial statement.
#[derive(Clone, Debug, Serialize)]
pub struct Statement {
    /// "Profit and Loss", "Balance Sheet", ...
    pub title: String,
    /// "October 4 – October 11, 2026" or "As of October 11, 2026".
    pub period: String,
    /// Amount column headings ("Total"; "Debit", "Credit"; member names).
    pub columns: Vec<String>,
    /// The rows, top to bottom.
    pub rows: Vec<Row>,
    /// Columns from this index on hold percentages ("12.5%"), not money:
    /// a comparison's "%" column, a P&L's "% of income".
    pub percent_from: Option<usize>,
}

fn row(kind: RowKind, label: impl Into<String>, amounts: Vec<Option<Decimal>>) -> Row {
    Row {
        kind,
        label: label.into(),
        amounts,
        currency: false,
        rule_above: false,
    }
}

fn one(kind: RowKind, label: impl Into<String>, amount: Decimal) -> Row {
    row(kind, label, vec![Some(amount)])
}

fn section(label: &str) -> Row {
    row(RowKind::Section, label, vec![])
}

fn subheading(label: &str) -> Row {
    row(RowKind::Subheading, label, vec![])
}

fn spacer() -> Row {
    row(RowKind::Spacer, "", vec![])
}

/// Finish a statement: currency signs on sums, rules above them.
pub fn statement(title: &str, period: String, columns: &[&str], mut rows: Vec<Row>) -> Statement {
    let mut above: Option<RowKind> = None;
    for r in &mut rows {
        r.currency = matches!(r.kind, RowKind::Subtotal | RowKind::Total);
        r.rule_above = match r.kind {
            RowKind::Total => true,
            RowKind::Subtotal => matches!(above, Some(RowKind::Line | RowKind::Subtotal)),
            _ => false,
        };
        if !matches!(r.kind, RowKind::Spacer | RowKind::Subheading) {
            above = Some(r.kind);
        }
    }
    Statement {
        title: title.to_string(),
        period,
        columns: columns.iter().map(|c| c.to_string()).collect(),
        rows,
        percent_from: None,
    }
}

fn account_lines(lines: &[StatementLine]) -> impl Iterator<Item = Row> + '_ {
    lines
        .iter()
        .map(|l| one(RowKind::Line, format!("{} {}", l.code, l.name), l.amount))
}

/// Lay out a profit and loss statement.
pub fn profit_and_loss(pl: &ProfitAndLossStatement) -> Statement {
    let mut rows = vec![section("Revenue")];
    rows.extend(account_lines(&pl.revenue));
    rows.push(one(RowKind::Subtotal, "Total revenue", pl.total_revenue));
    rows.push(spacer());
    rows.push(section("Expenses"));
    rows.extend(account_lines(&pl.expenses));
    rows.push(one(RowKind::Subtotal, "Total expenses", pl.total_expenses));
    rows.push(spacer());
    rows.push(one(RowKind::Total, "Net income (loss)", pl.net_income()));
    statement(
        "Profit and Loss",
        period_label(pl.start, pl.end),
        &["Total"],
        rows,
    )
}

/// The fiscal year's earnings, as the balance sheet presents them.
#[derive(Clone, Copy, Debug)]
pub struct YearEarnings {
    /// Net income (loss) for the fiscal year through the as-of date.
    pub net_income_ytd: Decimal,
    /// The part of it not yet closed into equity accounts.
    pub unclosed: Decimal,
}

/// Lay out a balance sheet.
///
/// Without members (corporate convention), equity ends with "Retained
/// earnings" — the Retained Earnings account plus earlier years' earnings
/// not yet closed — and the fiscal year's "Net income (loss)".
///
/// With members (a partnership), each member's capital account already
/// carries the earnings closed into it, so equity ends with only what
/// hasn't been allocated: "Undistributed earnings, prior years" (if any)
/// and this year's unclosed "Net income (loss)". Equity is "Members'
/// equity".
pub fn balance_sheet(
    bs: &BalanceSheetStatement,
    earnings: YearEarnings,
    members: &[Member],
) -> Statement {
    let equity_name = if members.is_empty() {
        "Equity"
    } else {
        "Members' equity"
    };
    let mut rows = vec![section("Assets")];
    rows.extend(account_lines(&bs.assets));
    rows.push(one(RowKind::Subtotal, "Total assets", bs.total_assets));
    rows.push(spacer());
    rows.push(section("Liabilities"));
    rows.extend(account_lines(&bs.liabilities));
    rows.push(one(
        RowKind::Subtotal,
        "Total liabilities",
        bs.total_liabilities,
    ));
    rows.push(spacer());
    rows.push(section(equity_name));
    if members.is_empty() {
        let (retained, other): (Vec<_>, Vec<_>) = bs
            .equity
            .iter()
            .partition(|l| l.code == RETAINED_EARNINGS_CODE);
        rows.extend(
            other
                .iter()
                .map(|l| one(RowKind::Line, format!("{} {}", l.code, l.name), l.amount)),
        );
        let retained: Decimal = retained.iter().map(|l| l.amount).sum::<Decimal>()
            + bs.current_earnings
            - earnings.net_income_ytd;
        if !retained.is_zero() {
            rows.push(one(RowKind::Line, "Retained earnings", retained));
        }
        rows.push(one(
            RowKind::Line,
            "Net income (loss)",
            earnings.net_income_ytd,
        ));
    } else {
        rows.extend(account_lines(&bs.equity));
        let prior = bs.current_earnings - earnings.unclosed;
        if !prior.is_zero() {
            rows.push(one(
                RowKind::Line,
                "Undistributed earnings, prior years",
                prior,
            ));
        }
        rows.push(one(RowKind::Line, "Net income (loss)", earnings.unclosed));
    }
    rows.push(one(
        RowKind::Subtotal,
        format!("Total {}", equity_name.to_lowercase()),
        bs.total_equity,
    ));
    rows.push(spacer());
    rows.push(one(
        RowKind::Total,
        format!("Total liabilities and {}", equity_name.to_lowercase()),
        bs.total_liabilities + bs.total_equity,
    ));
    statement(
        "Balance Sheet",
        format!("As of {}", long_date(bs.as_of)),
        &["Total"],
        rows,
    )
}

/// Cash flow lines, naming members on their capital accounts.
fn flow_lines<'a>(
    lines: &'a [CashFlowLine],
    members: &'a [Member],
) -> impl Iterator<Item = Row> + 'a {
    lines.iter().map(move |l| {
        let member = members.iter().find(|m| {
            let code = l.account.as_deref();
            code == Some(m.capital_code.as_str()) || code == m.distribution_code.as_deref()
        });
        let label = match member {
            Some(m) if l.amount >= Decimal::ZERO => format!("Member contributions — {}", m.name),
            Some(m) => format!("Member distributions — {}", m.name),
            None => l.label.clone(),
        };
        one(RowKind::Line, label, l.amount)
    })
}

/// Lay out a cash flow statement (indirect method).
pub fn cash_flow(cf: &CashFlowStatement, members: &[Member]) -> Statement {
    let mut rows = vec![section("Operating activities")];
    rows.push(one(RowKind::Line, "Net income (loss)", cf.net_income));
    if !cf.depreciation.is_zero() || !cf.working_capital.is_empty() {
        rows.push(subheading(
            "Adjustments to reconcile net income (loss) to net cash from operating activities:",
        ));
    }
    if !cf.depreciation.is_zero() {
        rows.push(one(RowKind::Line, "Depreciation", cf.depreciation));
    }
    rows.extend(flow_lines(&cf.working_capital, members));
    rows.push(one(
        RowKind::Subtotal,
        "Net cash from operating activities",
        cf.cash_from_operations,
    ));
    rows.push(spacer());
    rows.push(section("Investing activities"));
    rows.extend(flow_lines(&cf.investing, members));
    rows.push(one(
        RowKind::Subtotal,
        "Net cash from investing activities",
        cf.cash_from_investing,
    ));
    rows.push(spacer());
    rows.push(section("Financing activities"));
    rows.extend(flow_lines(&cf.financing, members));
    rows.push(one(
        RowKind::Subtotal,
        "Net cash from financing activities",
        cf.cash_from_financing,
    ));
    rows.push(spacer());
    rows.push(one(
        RowKind::Subtotal,
        "Net increase (decrease) in cash",
        cf.net_change_in_cash,
    ));
    rows.push(one(
        RowKind::Line,
        "Cash at beginning of period",
        cf.cash_beginning,
    ));
    rows.push(one(RowKind::Total, "Cash at end of period", cf.cash_ending));
    statement(
        "Statement of Cash Flows",
        period_label(cf.start, cf.end),
        &["Total"],
        rows,
    )
}

/// Lay out a trial balance as of a date: Debit and Credit columns, a zero
/// side left blank.
pub fn trial_balance(tb: &TrialBalance, as_of: NaiveDate) -> Statement {
    let nz = |d: Decimal| if d.is_zero() { None } else { Some(d) };
    let mut rows: Vec<Row> = tb
        .lines
        .iter()
        .map(|l| {
            row(
                RowKind::Line,
                format!("{} {}", l.code, l.name),
                vec![nz(l.debit), nz(l.credit)],
            )
        })
        .collect();
    rows.push(row(
        RowKind::Total,
        "Total",
        vec![Some(tb.total_debits), Some(tb.total_credits)],
    ));
    statement(
        "Trial Balance",
        format!("As of {}", long_date(as_of)),
        &["Debit", "Credit"],
        rows,
    )
}

/// Lay out the statement of changes in members' equity: one column per
/// member and a total. Distributions print as reductions.
pub fn members_equity(me: &MembersEquityStatement) -> Statement {
    let cols: Vec<_> = me
        .members
        .iter()
        .chain(std::iter::once(&me.total))
        .collect();
    let pick = |f: &dyn Fn(&ledger_core::MemberEquityColumn) -> Decimal| {
        cols.iter().map(|c| Some(f(c))).collect::<Vec<_>>()
    };
    let mut rows = vec![
        row(RowKind::Line, "Beginning balance", pick(&|c| c.beginning)),
        row(RowKind::Line, "Contributions", pick(&|c| c.contributions)),
        row(RowKind::Line, "Net income (loss)", pick(&|c| c.net_income)),
        row(RowKind::Line, "Distributions", pick(&|c| -c.distributions)),
    ];
    if !me.total.other.is_zero() {
        rows.push(row(RowKind::Line, "Other changes", pick(&|c| c.other)));
    }
    rows.push(row(RowKind::Total, "Ending balance", pick(&|c| c.ending)));
    let names: Vec<&str> = cols.iter().map(|c| c.name.as_str()).collect();
    statement(
        "Statement of Changes in Members' Equity",
        period_label(me.start, me.end),
        &names,
        rows,
    )
}

/// Lay out the tax summary: book and tax amounts by tax category, the
/// difference where they disagree (meals at 50%, penalties at 0%), and any
/// account still needing a category.
pub fn tax_summary(ts: &ledger_core::TaxSummary) -> Statement {
    let diff = |b: Decimal, t: Decimal| if b == t { None } else { Some(b - t) };
    let lines = |ls: &[ledger_core::TaxSummaryLine]| -> Vec<Row> {
        ls.iter()
            .map(|l| {
                let codes: Vec<&str> = l
                    .accounts
                    .iter()
                    .map(|a| a.split_whitespace().next().unwrap_or(""))
                    .collect();
                row(
                    RowKind::Line,
                    format!("{} ({})", l.label, codes.join(", ")),
                    vec![Some(l.book), Some(l.tax), diff(l.book, l.tax)],
                )
            })
            .collect()
    };
    let total = |kind, label: &str, ls: &[ledger_core::TaxSummaryLine]| {
        let b: Decimal = ls.iter().map(|l| l.book).sum();
        let t: Decimal = ls.iter().map(|l| l.tax).sum();
        row(kind, label, vec![Some(b), Some(t), diff(b, t)])
    };
    let mut rows = vec![section("Income")];
    rows.extend(lines(&ts.income));
    rows.push(total(RowKind::Subtotal, "Total income", &ts.income));
    if !ts.cost_of_goods.is_empty() {
        rows.push(spacer());
        rows.push(section("Cost of goods sold"));
        rows.extend(lines(&ts.cost_of_goods));
        rows.push(total(
            RowKind::Subtotal,
            "Total cost of goods sold",
            &ts.cost_of_goods,
        ));
    }
    rows.push(spacer());
    rows.push(section("Deductions"));
    rows.extend(lines(&ts.deductions));
    rows.push(total(RowKind::Subtotal, "Total deductions", &ts.deductions));
    rows.push(spacer());
    rows.push(row(
        RowKind::Total,
        "Income less deductions",
        vec![
            Some(ts.book_net()),
            Some(ts.tax_net()),
            diff(ts.book_net(), ts.tax_net()),
        ],
    ));
    if !ts.separately_stated.is_empty() {
        rows.push(spacer());
        rows.push(section("Separately stated to owners (Schedule K)"));
        rows.push(subheading(
            "Portfolio interest and charitable contributions; not included above:",
        ));
        rows.extend(lines(&ts.separately_stated));
    }
    if !ts.uncategorized.is_empty() {
        rows.push(spacer());
        rows.push(section("Not yet categorized"));
        rows.push(subheading(
            "Assign each a tax category (books.set_tax_category); not included above:",
        ));
        rows.extend(ts.uncategorized.iter().map(|u| {
            row(
                RowKind::Line,
                u.account.clone(),
                vec![Some(u.book), None, None],
            )
        }));
    }
    let title = match ts.form {
        Some(f) => format!("Tax Summary ({} categories)", f.label()),
        None => "Tax Summary".to_string(),
    };
    statement(
        &title,
        period_label(ts.start, ts.end),
        &["Book", "Tax", "Difference"],
        rows,
    )
}

/// "October 11, 2026".
pub fn long_date(d: NaiveDate) -> String {
    d.format("%B %-d, %Y").to_string()
}

/// "October 4 – October 11, 2026", or with both years when they differ.
pub fn period_label(start: NaiveDate, end: NaiveDate) -> String {
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

/// An amount split for column alignment: the digits, and whether it is a
/// negative (printed in parentheses) or zero (printed as 0.00). Renderers
/// right-align `digits` and reserve the closing parenthesis's width on every
/// row, so decimal points line up whether or not a value is negative.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AmountParts {
    /// "1,234.56" — absolute value, grouped, two decimals.
    pub digits: String,
    /// Below zero.
    pub negative: bool,
}

/// Split an amount for aligned accounting display. Exact: no floats.
pub fn amount_parts(d: Decimal) -> AmountParts {
    let rounded = d.round_dp(2);
    AmountParts {
        digits: format_amount(rounded.abs()),
        negative: rounded.is_sign_negative() && !rounded.is_zero(),
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
    fn amounts_split_for_alignment() {
        let p = amount_parts(dec("-1234.5"));
        assert_eq!((p.digits.as_str(), p.negative), ("1,234.50", true));
        let z = amount_parts(dec("-0.004"));
        assert_eq!((z.digits.as_str(), z.negative), ("0.00", false));
    }

    #[test]
    fn currency_signs_and_rules_go_on_sums() {
        let rows = vec![
            section("Assets"),
            one(RowKind::Line, "a", dec("1")),
            one(RowKind::Line, "b", dec("2")),
            one(RowKind::Subtotal, "t", dec("3")),
            spacer(),
            section("Investing"),
            one(RowKind::Subtotal, "empty", dec("0")),
            one(RowKind::Total, "T", dec("3")),
        ];
        let st = statement("X", String::new(), &["Total"], rows);
        let signs: Vec<bool> = st.rows.iter().map(|r| r.currency).collect();
        assert_eq!(signs, [false, false, false, true, false, false, true, true]);
        let rules: Vec<bool> = st.rows.iter().map(|r| r.rule_above).collect();
        // No rule over an empty section's subtotal; always one over a total.
        assert_eq!(
            rules,
            [false, false, false, true, false, false, false, true]
        );
    }

    fn sl(code: &str, name: &str, a: &str) -> StatementLine {
        StatementLine {
            code: code.into(),
            name: name.into(),
            amount: dec(a),
        }
    }

    fn equity_rows(st: &Statement, from: &str) -> Vec<(String, Option<Decimal>)> {
        st.rows
            .iter()
            .skip_while(|r| r.label != from)
            .map(|r| (r.label.clone(), r.amounts.first().copied().flatten()))
            .collect()
    }

    #[test]
    fn corporate_balance_sheet_splits_retained_earnings_from_this_year() {
        // Prior years closed 1,000 to Retained Earnings; 200 more unclosed
        // from last year; this year's net income is 300 (current earnings
        // 500 = 200 + 300).
        let bs = BalanceSheetStatement {
            as_of: NaiveDate::from_ymd_opt(2027, 3, 31).unwrap(),
            assets: vec![sl("1000", "Cash", "2500")],
            liabilities: vec![],
            equity: vec![
                sl("3000", "Common Stock", "1000"),
                sl("3999", "Retained Earnings", "1000"),
            ],
            current_earnings: dec("500"),
            total_assets: dec("2500"),
            total_liabilities: dec("0"),
            total_equity: dec("2500"),
        };
        let e = YearEarnings {
            net_income_ytd: dec("300"),
            unclosed: dec("300"),
        };
        let st = balance_sheet(&bs, e, &[]);
        let some = |s: &str| Some(dec(s));
        assert_eq!(
            equity_rows(&st, "Equity"),
            [
                ("Equity".into(), None),
                ("3000 Common Stock".into(), some("1000")),
                ("Retained earnings".into(), some("1200")),
                ("Net income (loss)".into(), some("300")),
                ("Total equity".into(), some("2500")),
                ("".into(), None),
                ("Total liabilities and equity".into(), some("2500")),
            ]
        );
    }

    #[test]
    fn partnership_balance_sheet_shows_only_unallocated_earnings() {
        // January (100) was closed into the capital accounts; February and
        // March (200) are this year's unclosed income; 50 from last year
        // was never closed.
        let bs = BalanceSheetStatement {
            as_of: NaiveDate::from_ymd_opt(2027, 3, 31).unwrap(),
            assets: vec![sl("1000", "Cash", "1350")],
            liabilities: vec![],
            equity: vec![
                sl("3000", "Capital — Alice", "550"),
                sl("3001", "Capital — Bob", "550"),
            ],
            current_earnings: dec("250"),
            total_assets: dec("1350"),
            total_liabilities: dec("0"),
            total_equity: dec("1350"),
        };
        let e = YearEarnings {
            net_income_ytd: dec("300"),
            unclosed: dec("200"),
        };
        let members = [
            Member {
                name: "Alice".into(),
                capital_code: "3000".into(),
                distribution_code: None,
            },
            Member {
                name: "Bob".into(),
                capital_code: "3001".into(),
                distribution_code: None,
            },
        ];
        let st = balance_sheet(&bs, e, &members);
        let some = |s: &str| Some(dec(s));
        assert_eq!(
            equity_rows(&st, "Members' equity"),
            [
                ("Members' equity".into(), None),
                ("3000 Capital — Alice".into(), some("550")),
                ("3001 Capital — Bob".into(), some("550")),
                ("Undistributed earnings, prior years".into(), some("50")),
                ("Net income (loss)".into(), some("200")),
                ("Total members' equity".into(), some("1350")),
                ("".into(), None),
                ("Total liabilities and members' equity".into(), some("1350")),
            ]
        );
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
        let meta = ReportMeta {
            company: "X".into(),
            basis: "Accrual".into(),
            prepared_on: d(2026, 10, 11),
            journal_seq: 114,
        };
        assert_eq!(
            meta.footer(),
            "Accrual basis · Prepared Sunday, October 11, 2026 · Journal seq 114"
        );
    }
}
