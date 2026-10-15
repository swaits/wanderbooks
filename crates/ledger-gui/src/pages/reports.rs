//! Financial statements and the accountant's reports, over any range,
//! split into months, quarters, or years, compared with a prior period.
//! Every amount drills down to the account's transactions for exactly the
//! dates behind it. Statements come from `ledger-core`; this module lays
//! them out.

use crate::app::Shell;
use crate::books::Books;
use crate::fmt;
use crate::nav::{ReportKind, Route};
use crate::theme::{self, pal};
use crate::widgets;
use chrono::{Datelike, Days, Months, NaiveDate};
use egui::{CornerRadius, RichText, Sense, Stroke, Ui, vec2};
use egui_phosphor::regular as icon;
use ledger_core::analysis::{Interval, intervals};
use ledger_core::reports::{
    CashFlowParams, balance_sheet_as_of, cash_flow, general_ledger, members_equity,
    profit_and_loss_for_period, trial_balance_as_of,
};
use ledger_core::{AccountId, AccountType, MemberCapital, tax_summary, vendor_1099_report};
use rust_decimal::Decimal;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Range {
    ThisMonth,
    LastMonth,
    ThisQuarter,
    #[default]
    YearToDate,
    LastYear,
    Last12Months,
    AllTime,
    Custom(NaiveDate, NaiveDate),
}

impl Range {
    pub fn dates(self, b: &Books) -> (NaiveDate, NaiveDate) {
        let t = b.today;
        let ms = b.month_start();
        match self {
            Range::ThisMonth => (ms, t),
            Range::LastMonth => (ms - Months::new(1), ms - Days::new(1)),
            Range::ThisQuarter => (
                ledger_core::analysis::interval_start(t, Interval::Quarter),
                t,
            ),
            Range::YearToDate => b.fiscal_ytd(),
            Range::LastYear => {
                let (s, _) = b.fiscal_ytd();
                (s - Months::new(12), s - Days::new(1))
            }
            Range::Last12Months => (ms - Months::new(11), t),
            Range::AllTime => (b.kpis.first_entry.unwrap_or(t).min(t), t),
            Range::Custom(s, e) => (s, e),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Range::ThisMonth => "This month",
            Range::LastMonth => "Last month",
            Range::ThisQuarter => "This quarter",
            Range::YearToDate => "Year to date",
            Range::LastYear => "Last year",
            Range::Last12Months => "Last 12 months",
            Range::AllTime => "All time",
            Range::Custom(..) => "Custom",
        }
    }

    const PRESETS: [Range; 7] = [
        Range::ThisMonth,
        Range::LastMonth,
        Range::ThisQuarter,
        Range::YearToDate,
        Range::LastYear,
        Range::Last12Months,
        Range::AllTime,
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Columns {
    #[default]
    Total,
    Month,
    Quarter,
    Year,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Compare {
    #[default]
    None,
    PriorPeriod,
    PriorYear,
}

#[derive(Default)]
pub struct State {
    pub range: Range,
    pub columns: Columns,
    pub compare: Compare,
    /// A PDF being typeset in the background.
    pdf_job: Option<std::sync::mpsc::Receiver<Result<std::path::PathBuf, String>>>,
    /// How the last PDF went, and when (to fade the message).
    pdf_done: Option<(Result<std::path::PathBuf, String>, f64)>,
}

/// What a PDF of the report on screen needs besides the table.
struct PdfInfo {
    title: String,
    period: String,
    meta: ledger_import::ReportMeta,
}

impl PdfInfo {
    fn new(b: &Books, title: &str, period: &str) -> Self {
        PdfInfo {
            title: title.to_string(),
            period: period.to_string(),
            meta: ledger_import::ReportMeta {
                company: b.company_name().to_string(),
                basis: b.settings.company.basis.clone(),
                prepared_on: b.today,
                journal_seq: b.seq(),
            },
        }
    }
}

// ----------------------------------------------------------------------
// The table model every statement renders through.

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Section,
    Line,
    Subtotal,
    Grand,
    Note,
}

#[derive(Clone, Copy)]
struct Drill {
    account: AccountId,
    start: NaiveDate,
    end: NaiveDate,
}

#[derive(Clone)]
struct Cell {
    value: Option<Decimal>,
    drill: Option<Drill>,
}

impl Cell {
    fn none() -> Self {
        Cell {
            value: None,
            drill: None,
        }
    }
    fn of(v: Decimal) -> Self {
        Cell {
            value: Some(v),
            drill: None,
        }
    }
}

struct Row {
    kind: Kind,
    label: String,
    detail: String,
    cells: Vec<Cell>,
}

struct Table {
    columns: Vec<String>,
    rows: Vec<Row>,
    /// Format cells as percentages from this column index on.
    pct_from: Option<usize>,
}

impl Table {
    fn new(columns: Vec<String>) -> Self {
        Table {
            columns,
            rows: Vec::new(),
            pct_from: None,
        }
    }
    fn section(&mut self, label: &str) {
        self.rows.push(Row {
            kind: Kind::Section,
            label: label.into(),
            detail: String::new(),
            cells: Vec::new(),
        });
    }
    fn row(&mut self, kind: Kind, label: impl Into<String>, cells: Vec<Cell>) {
        self.rows.push(Row {
            kind,
            label: label.into(),
            detail: String::new(),
            cells,
        });
    }
    fn note(&mut self, text: impl Into<String>) {
        self.rows.push(Row {
            kind: Kind::Note,
            label: text.into(),
            detail: String::new(),
            cells: Vec::new(),
        });
    }

    /// Add "Change" and "%" columns comparing column 0 to column 1.
    fn add_change(&mut self) {
        self.columns.push("Change".into());
        self.columns.push("%".into());
        self.pct_from = Some(self.columns.len() - 1);
        for r in &mut self.rows {
            if r.cells.len() < 2 {
                continue;
            }
            let (a, b) = (r.cells[0].value, r.cells[1].value);
            let diff = match (a, b) {
                (Some(a), Some(b)) => Some(a - b),
                (Some(a), None) => Some(a),
                (None, Some(b)) => Some(-b),
                _ => None,
            };
            let pct = match (diff, b) {
                (Some(d), Some(b)) if !b.is_zero() => {
                    Some((d / b.abs() * Decimal::from(100)).round_dp(1))
                }
                _ => None,
            };
            r.cells.push(Cell {
                value: diff,
                drill: None,
            });
            r.cells.push(Cell {
                value: pct,
                drill: None,
            });
        }
    }

    /// Add a "% of income" column: each line as a share of total income.
    fn add_share_of_income(&mut self) {
        let income = self
            .rows
            .iter()
            .find(|r| r.kind == Kind::Subtotal && r.label == "Total income")
            .and_then(|r| r.cells.first())
            .and_then(|c| c.value)
            .unwrap_or_default();
        if income.is_zero() {
            return;
        }
        self.columns.push("% of income".into());
        self.pct_from = Some(self.columns.len() - 1);
        for r in &mut self.rows {
            if let Some(v) = r.cells.first().and_then(|c| c.value) {
                r.cells
                    .push(Cell::of((v / income * Decimal::from(100)).round_dp(1)));
            }
        }
    }

    /// The table as a statement for the PDF typesetter: sections spaced
    /// apart, sums ruled and signed, notes as italic subheadings. Detail
    /// lines (the accounts under a tax line) don't carry over.
    fn statement(&self, title: &str, period: &str) -> ledger_import::Statement {
        use ledger_import::{Row as PdfRow, RowKind};
        let mut rows = Vec::new();
        for r in &self.rows {
            let kind = match r.kind {
                Kind::Section => RowKind::Section,
                Kind::Line => RowKind::Line,
                Kind::Subtotal => RowKind::Subtotal,
                Kind::Grand => RowKind::Total,
                Kind::Note => RowKind::Subheading,
            };
            if matches!(kind, RowKind::Section | RowKind::Subheading) && !rows.is_empty() {
                rows.push(PdfRow {
                    kind: RowKind::Spacer,
                    label: String::new(),
                    amounts: Vec::new(),
                    currency: false,
                    rule_above: false,
                });
            }
            // Icon glyphs (private use) aren't in the PDF's font.
            let label: String = r
                .label
                .chars()
                .filter(|c| !('\u{E000}'..='\u{F8FF}').contains(c))
                .collect();
            rows.push(PdfRow {
                kind,
                label: label.trim().to_string(),
                amounts: r.cells.iter().map(|c| c.value).collect(),
                currency: false,
                rule_above: false,
            });
        }
        let columns: Vec<&str> = self.columns.iter().map(String::as_str).collect();
        let mut st = ledger_import::report::statement(title, period.to_string(), &columns, rows);
        st.percent_from = self.pct_from;
        st
    }

    fn tsv(&self) -> String {
        let mut out = String::from("\t");
        out.push_str(&self.columns.join("\t"));
        out.push('\n');
        for r in &self.rows {
            out.push_str(&r.label);
            for c in &r.cells {
                out.push('\t');
                if let Some(v) = c.value {
                    out.push_str(&v.round_dp(2).to_string());
                }
            }
            out.push('\n');
        }
        out
    }
}

fn col_title(s: NaiveDate, e: NaiveDate, columns: Columns) -> String {
    match columns {
        Columns::Month => s.format("%b %Y").to_string(),
        Columns::Quarter => format!("Q{} {}", (s.month() - 1) / 3 + 1, s.year()),
        Columns::Year => s.year().to_string(),
        Columns::Total => {
            if s.year() == e.year() {
                format!("{} – {}", fmt::date_short(s), fmt::date_short(e))
            } else {
                format!("{} – {}", fmt::month(s), fmt::month(e))
            }
        }
    }
}

/// The column ranges for a range and split (with the prior period second
/// when comparing a single column).
fn column_ranges(st: &State, b: &Books) -> Vec<(NaiveDate, NaiveDate, String)> {
    let (s, e) = st.range.dates(b);
    let split = match st.columns {
        Columns::Total => None,
        Columns::Month => Some(Interval::Month),
        Columns::Quarter => Some(Interval::Quarter),
        Columns::Year => Some(Interval::Year),
    };
    match split {
        None => {
            let mut v = vec![(s, e, col_title(s, e, Columns::Total))];
            if let Some((ps, pe)) = prior(st, s, e) {
                v.push((ps, pe, col_title(ps, pe, Columns::Total)));
                // Same days a year apart read the same: say which year.
                if v[0].2 == v[1].2 {
                    v[0].2 = format!("{}, {}", v[0].2, e.year());
                    v[1].2 = format!("{}, {}", v[1].2, pe.year());
                }
            }
            v
        }
        Some(i) => intervals(s, e, i)
            .into_iter()
            .map(|(a, z)| (a, z, col_title(a, z, st.columns)))
            .collect(),
    }
}

fn prior(st: &State, s: NaiveDate, e: NaiveDate) -> Option<(NaiveDate, NaiveDate)> {
    match st.compare {
        Compare::None => None,
        Compare::PriorYear => Some((s - Months::new(12), e - Months::new(12))),
        Compare::PriorPeriod => {
            let days = (e - s).num_days() + 1;
            let pe = s - Days::new(1);
            Some((pe - Days::new(days as u64 - 1), pe))
        }
    }
}

type Cols = [(NaiveDate, NaiveDate, String)];

fn to_jiff(d: NaiveDate) -> jiff::civil::Date {
    jiff::civil::Date::new(d.year() as i16, d.month() as i8, d.day() as i8).expect("valid date")
}

fn from_jiff(d: jiff::civil::Date) -> NaiveDate {
    NaiveDate::from_ymd_opt(i32::from(d.year()), d.month() as u32, d.day() as u32)
        .expect("valid date")
}

// ----------------------------------------------------------------------
// Builders.

fn pnl(b: &Books, cols: &Cols, total: bool) -> Table {
    let mut cols = cols.to_vec();
    if total && cols.len() > 1 {
        cols.push((cols[0].0, cols[cols.len() - 1].1, "Total".into()));
    }
    let stmts: Vec<_> = cols
        .iter()
        .map(|(s, e, _)| profit_and_loss_for_period(&b.ledger, *s, *e, None).ok())
        .collect();
    let mut t = Table::new(cols.iter().map(|c| c.2.clone()).collect());
    let section = |t: &mut Table,
                   title: &str,
                   pick: &dyn Fn(
        &ledger_core::ProfitAndLossStatement,
    ) -> &Vec<ledger_core::StatementLine>,
                   total_label: &str| {
        let mut accounts: BTreeMap<String, String> = BTreeMap::new();
        for s in stmts.iter().flatten() {
            for l in pick(s) {
                accounts.insert(l.code.clone(), l.name.clone());
            }
        }
        t.section(title);
        let mut totals = vec![Decimal::ZERO; cols.len()];
        for (code, name) in &accounts {
            let id = b.ledger.chart().find_by_code(code).map(|a| a.id);
            let cells = cols
                .iter()
                .zip(&stmts)
                .enumerate()
                .map(|(i, ((s, e, _), st))| {
                    let v = st
                        .as_ref()
                        .and_then(|st| pick(st).iter().find(|l| &l.code == code))
                        .map(|l| l.amount)
                        .unwrap_or_default();
                    totals[i] += v;
                    Cell {
                        value: Some(v),
                        drill: id.map(|account| Drill {
                            account,
                            start: *s,
                            end: *e,
                        }),
                    }
                })
                .collect();
            t.row(Kind::Line, format!("{code} {name}"), cells);
        }
        t.row(
            Kind::Subtotal,
            total_label,
            totals.iter().map(|v| Cell::of(*v)).collect(),
        );
        totals
    };
    let rev = section(&mut t, "Income", &|s| &s.revenue, "Total income");
    let exp = section(&mut t, "Expenses", &|s| &s.expenses, "Total expenses");
    t.row(
        Kind::Grand,
        "Net income",
        rev.iter().zip(&exp).map(|(r, e)| Cell::of(r - e)).collect(),
    );
    t
}

fn balance_sheet(b: &Books, dates: &[(NaiveDate, String)]) -> Table {
    let stmts: Vec<_> = dates
        .iter()
        .map(|(d, _)| balance_sheet_as_of(&b.ledger, *d).ok())
        .collect();
    let start = b.kpis.first_entry.unwrap_or(b.today);
    let mut t = Table::new(dates.iter().map(|d| d.1.clone()).collect());
    let section = |t: &mut Table,
                   title: &str,
                   pick: &dyn Fn(
        &ledger_core::BalanceSheetStatement,
    ) -> &Vec<ledger_core::StatementLine>| {
        let mut accounts: BTreeMap<String, String> = BTreeMap::new();
        for s in stmts.iter().flatten() {
            for l in pick(s) {
                accounts.insert(l.code.clone(), l.name.clone());
            }
        }
        t.section(title);
        for (code, name) in &accounts {
            let id = b.ledger.chart().find_by_code(code).map(|a| a.id);
            let cells = dates
                .iter()
                .zip(&stmts)
                .map(|((d, _), st)| Cell {
                    value: Some(
                        st.as_ref()
                            .and_then(|st| pick(st).iter().find(|l| &l.code == code))
                            .map(|l| l.amount)
                            .unwrap_or_default(),
                    ),
                    drill: id.map(|account| Drill {
                        account,
                        start: start.min(*d),
                        end: *d,
                    }),
                })
                .collect();
            t.row(Kind::Line, format!("{code} {name}"), cells);
        }
    };
    let total = |f: &dyn Fn(&ledger_core::BalanceSheetStatement) -> Decimal| -> Vec<Cell> {
        stmts
            .iter()
            .map(|s| Cell {
                value: s.as_ref().map(f),
                drill: None,
            })
            .collect()
    };
    section(&mut t, "Assets", &|s| &s.assets);
    t.row(Kind::Subtotal, "Total assets", total(&|s| s.total_assets));
    section(&mut t, "Liabilities", &|s| &s.liabilities);
    t.row(
        Kind::Subtotal,
        "Total liabilities",
        total(&|s| s.total_liabilities),
    );
    section(&mut t, "Equity", &|s| &s.equity);
    t.row(
        Kind::Line,
        "Net income not yet closed",
        total(&|s| s.current_earnings),
    );
    t.row(Kind::Subtotal, "Total equity", total(&|s| s.total_equity));
    t.row(
        Kind::Grand,
        "Total liabilities and equity",
        total(&|s| s.total_liabilities + s.total_equity),
    );
    if stmts
        .iter()
        .flatten()
        .all(|s| s.total_assets == s.total_liabilities + s.total_equity)
    {
        t.note(format!(
            "{} Assets equal liabilities plus equity in every column.",
            icon::CHECK
        ));
    }
    t
}

/// Financing accounts: from `[books] financing_accounts`, else liabilities
/// whose names say they are debt.
fn financing_accounts(b: &Books) -> Vec<AccountId> {
    if !b.financing_accounts.is_empty() {
        return b.financing_accounts.clone();
    }
    b.ledger
        .chart()
        .all()
        .filter(|a| {
            let n = a.name.to_lowercase();
            a.account_type == AccountType::Liability
                && (n.contains("loan")
                    || n.contains("note payable")
                    || n.contains("line of credit"))
        })
        .map(|a| a.id)
        .collect()
}

fn cash_flow_table(b: &Books, cols: &Cols, total: bool) -> Table {
    let mut cols = cols.to_vec();
    if total && cols.len() > 1 {
        cols.push((cols[0].0, cols[cols.len() - 1].1, "Total".into()));
    }
    let fin = financing_accounts(b);
    let stmts: Vec<_> = cols
        .iter()
        .map(|(s, e, _)| {
            cash_flow(
                &b.ledger,
                &CashFlowParams {
                    start: *s,
                    end: *e,
                    cash_accounts: b.cash_accounts.clone(),
                    financing_accounts: fin.clone(),
                    investing_accounts: b.investing_accounts.clone(),
                },
            )
            .ok()
        })
        .collect();
    let mut t = Table::new(cols.iter().map(|c| c.2.clone()).collect());
    let val = |f: &dyn Fn(&ledger_core::CashFlowStatement) -> Decimal| -> Vec<Cell> {
        stmts
            .iter()
            .map(|s| Cell {
                value: s.as_ref().map(f),
                drill: None,
            })
            .collect()
    };
    let lines = |t: &mut Table,
                 pick: &dyn Fn(
        &ledger_core::CashFlowStatement,
    ) -> &Vec<ledger_core::CashFlowLine>| {
        let mut labels: Vec<(String, Option<String>)> = Vec::new();
        for s in stmts.iter().flatten() {
            for l in pick(s) {
                if !labels.iter().any(|(x, _)| x == &l.label) {
                    labels.push((l.label.clone(), l.account.clone()));
                }
            }
        }
        for (label, account) in labels {
            let id = account
                .as_deref()
                .and_then(|c| b.ledger.chart().find_by_code(c))
                .map(|a| a.id);
            let cells = cols
                .iter()
                .zip(&stmts)
                .map(|((s, e, _), st)| Cell {
                    value: Some(
                        st.as_ref()
                            .and_then(|st| pick(st).iter().find(|l| l.label == label))
                            .map(|l| l.amount)
                            .unwrap_or_default(),
                    ),
                    drill: id.map(|account| Drill {
                        account,
                        start: *s,
                        end: *e,
                    }),
                })
                .collect();
            t.row(Kind::Line, label, cells);
        }
    };
    t.section("Operating activities");
    t.row(Kind::Line, "Net income", val(&|s| s.net_income));
    t.row(
        Kind::Line,
        "Depreciation (non-cash)",
        val(&|s| s.depreciation),
    );
    lines(&mut t, &|s| &s.working_capital);
    t.row(
        Kind::Subtotal,
        "Cash from operations",
        val(&|s| s.cash_from_operations),
    );
    t.section("Investing activities");
    lines(&mut t, &|s| &s.investing);
    t.row(
        Kind::Subtotal,
        "Cash from investing",
        val(&|s| s.cash_from_investing),
    );
    t.section("Financing activities");
    lines(&mut t, &|s| &s.financing);
    t.row(
        Kind::Subtotal,
        "Cash from financing",
        val(&|s| s.cash_from_financing),
    );
    t.row(
        Kind::Grand,
        "Net change in cash",
        val(&|s| s.net_change_in_cash),
    );
    t.row(Kind::Line, "Cash at beginning", val(&|s| s.cash_beginning));
    t.row(Kind::Line, "Cash at end", val(&|s| s.cash_ending));
    let names: Vec<String> = b
        .cash_accounts
        .iter()
        .map(|a| b.account_label(*a))
        .collect();
    t.note(format!("Cash accounts: {}.", names.join(", ")));
    t
}

fn trial_balance(b: &Books, as_of: NaiveDate) -> Table {
    let tb = trial_balance_as_of(&b.ledger, as_of);
    let start = b.kpis.first_entry.unwrap_or(as_of).min(as_of);
    let mut t = Table::new(vec!["Debit".into(), "Credit".into()]);
    for l in &tb.lines {
        let id = b.ledger.chart().find_by_code(&l.code).map(|a| a.id);
        let d = id.map(|account| Drill {
            account,
            start,
            end: as_of,
        });
        let cell = |v: Decimal| Cell {
            value: (!v.is_zero()).then_some(v),
            drill: d,
        };
        t.row(
            Kind::Line,
            format!("{} {}", l.code, l.name),
            vec![cell(l.debit), cell(l.credit)],
        );
    }
    t.row(
        Kind::Grand,
        "Total",
        vec![Cell::of(tb.total_debits), Cell::of(tb.total_credits)],
    );
    if tb.total_debits == tb.total_credits {
        t.note(format!("{} Debits equal credits.", icon::CHECK));
    }
    t
}

fn owners_equity(b: &Books, s: NaiveDate, e: NaiveDate) -> Table {
    let company = &b.settings.company;
    let members: Vec<MemberCapital> = company
        .members
        .iter()
        .filter_map(|m| {
            Some(MemberCapital {
                name: m.name.clone(),
                capital_account: b.ledger.chart().find_by_code(&m.capital_account)?.id,
                distribution_account: m
                    .distribution_account
                    .as_deref()
                    .and_then(|c| b.ledger.chart().find_by_code(c))
                    .map(|a| a.id),
                share: m.share,
            })
        })
        .collect();
    if !members.is_empty()
        && let Ok(me) = members_equity(&b.ledger, s, e, &members)
    {
        let mut cols: Vec<String> = me.members.iter().map(|m| m.name.clone()).collect();
        cols.push("Total".into());
        let mut t = Table::new(cols);
        let all: Vec<&ledger_core::MemberEquityColumn> = me
            .members
            .iter()
            .chain(std::iter::once(&me.total))
            .collect();
        let row = |t: &mut Table,
                   kind,
                   label: &str,
                   f: &dyn Fn(&ledger_core::MemberEquityColumn) -> Decimal| {
            t.row(kind, label, all.iter().map(|m| Cell::of(f(m))).collect());
        };
        row(
            &mut t,
            Kind::Line,
            &format!("Balance, {}", fmt::date(s - Days::new(1))),
            &|m| m.beginning,
        );
        row(&mut t, Kind::Line, "Contributions", &|m| m.contributions);
        row(&mut t, Kind::Line, "Share of net income", &|m| m.net_income);
        row(&mut t, Kind::Line, "Distributions", &|m| -m.distributions);
        row(&mut t, Kind::Line, "Other", &|m| m.other);
        row(
            &mut t,
            Kind::Grand,
            &format!("Balance, {}", fmt::date(e)),
            &|m| m.ending,
        );
        let shares: Vec<String> = members
            .iter()
            .map(|m| format!("{} {}%", m.name, (m.share * Decimal::from(100)).normalize()))
            .collect();
        t.note(format!("Ownership: {}.", shares.join(", ")));
        return t;
    }
    // Single owner or a corporation: every equity account's movement.
    let mut t = Table::new(vec!["Beginning".into(), "Change".into(), "Ending".into()]);
    let eq: Vec<AccountId> =
        ledger_core::analysis::accounts_of_type(&b.ledger, &[AccountType::Equity]);
    let (mut tb, mut te) = (Decimal::ZERO, Decimal::ZERO);
    for a in eq {
        let begin = b.balance_on(a, s - Days::new(1));
        let end = b.balance_on(a, e);
        tb += begin;
        te += end;
        let d = Some(Drill {
            account: a,
            start: s,
            end: e,
        });
        t.row(
            Kind::Line,
            b.account_label(a),
            vec![
                Cell::of(begin),
                Cell {
                    value: Some(end - begin),
                    drill: d,
                },
                Cell::of(end),
            ],
        );
    }
    let ni_begin = balance_sheet_as_of(&b.ledger, s - Days::new(1))
        .map(|bs| bs.current_earnings)
        .unwrap_or_default();
    let ni_end = balance_sheet_as_of(&b.ledger, e)
        .map(|bs| bs.current_earnings)
        .unwrap_or_default();
    t.row(
        Kind::Line,
        "Net income not yet closed",
        vec![
            Cell::of(ni_begin),
            Cell::of(ni_end - ni_begin),
            Cell::of(ni_end),
        ],
    );
    t.row(
        Kind::Grand,
        "Total equity",
        vec![
            Cell::of(tb + ni_begin),
            Cell::of(te + ni_end - tb - ni_begin),
            Cell::of(te + ni_end),
        ],
    );
    t
}

fn tax(b: &Books, s: NaiveDate, e: NaiveDate) -> Table {
    let mut t = Table::new(vec!["Book".into(), "Tax".into()]);
    let Ok(ts) = tax_summary(&b.ledger, s, e, b.settings.company.tax_form) else {
        return t;
    };
    let sec = |t: &mut Table, title: &str, lines: &[ledger_core::TaxSummaryLine]| {
        if lines.is_empty() {
            return;
        }
        t.section(title);
        for l in lines {
            t.rows.push(Row {
                kind: Kind::Line,
                label: l.label.clone(),
                detail: l.accounts.join(", "),
                cells: vec![Cell::of(l.book), Cell::of(l.tax)],
            });
        }
    };
    sec(&mut t, "Income", &ts.income);
    sec(&mut t, "Cost of goods sold", &ts.cost_of_goods);
    sec(&mut t, "Deductions", &ts.deductions);
    let net = if ts.uncategorized.is_empty() {
        "Net"
    } else {
        "Net of categorized accounts"
    };
    t.row(
        Kind::Grand,
        net,
        vec![Cell::of(ts.book_net()), Cell::of(ts.tax_net())],
    );
    if !ts.separately_stated.is_empty() {
        sec(
            &mut t,
            "Separately stated to owners (Schedule K)",
            &ts.separately_stated,
        );
    }
    if !ts.uncategorized.is_empty() {
        t.section("Not categorized");
        for u in &ts.uncategorized {
            let id = u
                .account
                .split_whitespace()
                .next()
                .and_then(|c| b.ledger.chart().find_by_code(c))
                .map(|a| a.id);
            t.row(
                Kind::Line,
                &u.account,
                vec![
                    Cell {
                        value: Some(u.book),
                        drill: id.map(|account| Drill {
                            account,
                            start: s,
                            end: e,
                        }),
                    },
                    Cell::none(),
                ],
            );
        }
    }
    let form = ts.form.map(fmt::tax_form).unwrap_or("not set");
    t.note(format!("Return: {form}. Deductible percentages applied (meals 50%, penalties 0%). A worksheet for the CPA, not a filing."));
    t
}

fn aging(rows: Vec<ledger_core::AgingRow>, as_of: NaiveDate) -> Table {
    let mut t = Table::new(vec![
        "Current".into(),
        "1–30".into(),
        "31–60".into(),
        "61–90".into(),
        "Over 90".into(),
        "Total".into(),
    ]);
    let mut tot = [Decimal::ZERO; 6];
    for r in &rows {
        let v = [
            r.current,
            r.days_30,
            r.days_60,
            r.days_90,
            r.days_90_plus,
            r.total(),
        ];
        for (i, x) in v.iter().enumerate() {
            tot[i] += x;
        }
        t.row(
            Kind::Line,
            &r.name,
            v.iter()
                .map(|x| Cell {
                    value: (!x.is_zero()).then_some(*x),
                    drill: None,
                })
                .collect(),
        );
    }
    t.row(
        Kind::Grand,
        "Total",
        tot.iter().map(|x| Cell::of(*x)).collect(),
    );
    t.note(format!("Days past due as of {}.", fmt::date(as_of)));
    t
}

/// The fixed asset register on `as_of`: ledger-core's
/// `fixed_asset_register`, laid out, with its reconciliation to the
/// general ledger and what that says (the Assets page's notices). Each
/// asset's federal tax basis sits beside its book figures: contributor's
/// basis, tax basis, and built-in gain (loss), the register row's own,
/// with its rule (and citation) in the detail; the tax basis total is the
/// register's.
fn asset_register(b: &Books, as_of: NaiveDate) -> Table {
    let r = b.asset_register(as_of);
    let mut t = Table::new(vec![
        "Cost".into(),
        "Accumulated".into(),
        "Book value".into(),
        "Contributor's basis".into(),
        "Tax basis".into(),
        "Built-in gain (loss)".into(),
    ]);
    let none3 = || vec![Cell::none(), Cell::none(), Cell::none()];
    if r.rows.is_empty() {
        t.note(format!(
            "No fixed asset was in service by {}.",
            fmt::date(as_of)
        ));
        return t;
    }
    t.section("Assets");
    for x in &r.rows {
        let label = match &x.tag {
            Some(tag) => format!("{tag} · {}", x.description),
            None => format!("{} (no tag)", x.description),
        };
        let mut detail = Vec::new();
        if let Some(s) = &x.serial {
            detail.push(format!("serial {s}"));
        }
        detail.push(format!("in service {}", fmt::date(x.in_service)));
        detail.push(fmt::count(x.useful_life_months as usize, "month", "months"));
        if !x.salvage.is_zero() {
            detail.push(format!("salvage {}", fmt::dollars(x.salvage)));
        }
        let tax = super::assets::TaxFacts::of_row(x);
        detail.push(match tax.rule {
            Some(rule) => format!(
                "tax basis: {}",
                ledger_import::asset_register::rule_cited(rule)
            ),
            None => "tax basis not recorded".into(),
        });
        let opt = |v: Option<Decimal>| Cell {
            value: v,
            drill: None,
        };
        let cells = match x.disposed {
            Some(_) => {
                detail.push(format!(
                    "{}; cost {}",
                    super::assets::status_text(x.disposed),
                    fmt::dollars(x.cost)
                ));
                [none3(), none3()].concat()
            }
            None => vec![
                Cell::of(x.cost),
                Cell::of(x.accumulated),
                Cell::of(x.book_value),
                opt(tax.contributor_basis),
                opt(tax.tax_basis),
                opt(tax.built_in_gain),
            ],
        };
        t.rows.push(Row {
            kind: Kind::Line,
            label,
            detail: detail.join(" · "),
            cells,
        });
    }
    t.row(
        Kind::Grand,
        "Total held",
        vec![
            Cell::of(r.total_cost),
            Cell::of(r.total_accumulated),
            Cell::of(r.total_book_value),
            Cell::none(),
            Cell::of(r.total_tax_basis),
            Cell::none(),
        ],
    );
    t.section("Reconciliation to the books");
    let start = b.kpis.first_entry.unwrap_or(as_of).min(as_of);
    let held: Vec<_> = r
        .rows
        .iter()
        .filter_map(|x| b.ledger.get_asset(x.asset_id))
        .collect();
    // One account behind a figure drills down to it.
    let only = |ids: Vec<AccountId>| -> Option<Drill> {
        let mut ids = ids;
        ids.sort_unstable();
        ids.dedup();
        (ids.len() == 1).then(|| Drill {
            account: ids[0],
            start,
            end: as_of,
        })
    };
    let cost_drill = only(held.iter().map(|a| a.cost_account).collect());
    let accum_drill = only(held.iter().map(|a| a.accum_depr_account).collect());
    t.row(
        Kind::Line,
        "In the books",
        vec![
            Cell {
                value: Some(r.ledger_cost),
                drill: cost_drill,
            },
            Cell {
                value: Some(r.ledger_accumulated),
                drill: accum_drill,
            },
            Cell::of(r.ledger_cost - r.ledger_accumulated),
            Cell::none(),
            Cell::none(),
            Cell::none(),
        ],
    );
    t.row(
        Kind::Subtotal,
        "Register less books",
        vec![
            Cell::of(-r.unregistered_cost()),
            Cell::of(r.unposted_depreciation()),
            Cell::of(-r.unregistered_cost() - r.unposted_depreciation()),
            Cell::none(),
            Cell::none(),
            Cell::none(),
        ],
    );
    for n in super::assets::notices(b, &r) {
        t.note(n.text);
    }
    t.note(
        "Cost is what the company paid or, for a member's contribution, its fair market value then; the tax basis follows its rule and can differ. Built-in gain (loss) is that value less the contributor's basis (IRC §704(c)). Tax depreciation starts from the tax basis — the CPA's.",
    );
    t
}

// ----------------------------------------------------------------------
// The page.

/// The reports, grouped as the side list shows them.
const GROUPS: [(&str, &[ReportKind]); 3] = [
    (
        "Statements",
        &[
            ReportKind::ProfitAndLoss,
            ReportKind::BalanceSheet,
            ReportKind::CashFlow,
            ReportKind::MembersEquity,
        ],
    ),
    (
        "Ledgers",
        &[
            ReportKind::TrialBalance,
            ReportKind::GeneralLedger,
            ReportKind::FixedAssetRegister,
        ],
    ),
    (
        "Tax and aging",
        &[
            ReportKind::TaxSummary,
            ReportKind::Vendor1099,
            ReportKind::ArAging,
            ReportKind::ApAging,
        ],
    ),
];

/// Narrower than this, the side list of reports becomes a picker above
/// the report.
const SIDE_LIST_MIN: f32 = 760.0;

pub fn show(ui: &mut Ui, b: &Books, kind: ReportKind, shell: &mut Shell<'_>) {
    let p = pal(ui.ctx());
    let rect = ui.available_rect_before_wrap();
    if rect.width() < SIDE_LIST_MIN {
        super::page(ui, "report", |ui| report(ui, b, kind, shell, true));
        return;
    }
    let nav_w = 210.0;
    let nav_rect = egui::Rect::from_min_size(rect.min, vec2(nav_w, rect.height()));
    ui.painter().line_segment(
        [nav_rect.right_top(), nav_rect.right_bottom()],
        Stroke::new(1.0, p.border),
    );
    let mut nav = ui.new_child(egui::UiBuilder::new().max_rect(nav_rect.shrink2(vec2(12.0, 24.0))));
    nav.label(
        RichText::new("REPORTS")
            .font(theme::semibold(11.5))
            .color(p.faint),
    );
    nav.add_space(4.0);
    for (g, kinds) in GROUPS {
        nav.add_space(10.0);
        nav.label(RichText::new(g).font(theme::regular(12.0)).color(p.muted));
        nav.add_space(2.0);
        for k in kinds {
            let on = *k == kind;
            let (r, resp) = nav.allocate_exact_size(vec2(nav_w - 24.0, 30.0), Sense::click());
            if on {
                nav.painter()
                    .rect_filled(r, CornerRadius::same(7), p.surface);
                nav.painter().rect_stroke(
                    r,
                    CornerRadius::same(7),
                    Stroke::new(1.0, p.border),
                    egui::StrokeKind::Inside,
                );
            } else if resp.hovered() {
                nav.painter().rect_filled(r, CornerRadius::same(7), p.hover);
            }
            nav.painter().text(
                r.left_center() + vec2(10.0, 0.0),
                egui::Align2::LEFT_CENTER,
                k.title(),
                if on {
                    theme::semibold(13.0)
                } else {
                    theme::regular(13.0)
                },
                if on { p.text } else { p.muted },
            );
            if resp.clicked() {
                shell.go(Route::Reports(*k));
            }
        }
    }
    let body = egui::Rect::from_min_max(egui::pos2(nav_rect.right() + 1.0, rect.top()), rect.max);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body));
    child.set_clip_rect(body);
    super::page(&mut child, "report", |ui| report(ui, b, kind, shell, false));
    ui.allocate_rect(rect, Sense::hover());
}

fn report(ui: &mut Ui, b: &Books, kind: ReportKind, shell: &mut Shell<'_>, picker: bool) {
    let p = pal(ui.ctx());
    let point_in_time = matches!(
        kind,
        ReportKind::TrialBalance
            | ReportKind::ArAging
            | ReportKind::ApAging
            | ReportKind::FixedAssetRegister
    ) || (kind == ReportKind::BalanceSheet
        && shell.state.reports.columns == Columns::Total);
    let splittable = matches!(
        kind,
        ReportKind::ProfitAndLoss | ReportKind::BalanceSheet | ReportKind::CashFlow
    );
    let comparable = matches!(
        kind,
        ReportKind::ProfitAndLoss
            | ReportKind::BalanceSheet
            | ReportKind::CashFlow
            | ReportKind::TaxSummary
    );
    let st = &mut shell.state.reports;

    // Controls. Each starts a new row when it won't fit on this one, by
    // its known width: a control that overflows would widen the page.
    let mut pick = None;
    let row_w = ui.available_width();
    let mut used = 0.0;
    let mut room = |ui: &mut Ui, w: f32| {
        if used > 0.0 && used + 8.0 + w > row_w {
            ui.end_row();
            used = 0.0;
        } else if used > 0.0 {
            ui.add_space(8.0);
            used += 8.0;
        }
        used += w;
    };
    ui.horizontal_wrapped(|ui| {
        if picker {
            room(ui, 214.0);
            egui::ComboBox::from_id_salt("report_kind")
                .selected_text(RichText::new(format!(
                    "{}  {}",
                    icon::FILE_TEXT,
                    kind.title()
                )))
                .width(190.0)
                .show_ui(ui, |ui| {
                    for (g, kinds) in GROUPS {
                        ui.label(RichText::new(g).font(theme::regular(12.0)).color(p.muted));
                        for k in kinds {
                            if ui.selectable_label(*k == kind, k.title()).clicked() {
                                pick = Some(*k);
                            }
                        }
                    }
                });
        }
        room(ui, 194.0);
        egui::ComboBox::from_id_salt("range")
            .selected_text(RichText::new(format!(
                "{}  {}",
                icon::CALENDAR_BLANK,
                st.range.label()
            )))
            .width(170.0)
            .show_ui(ui, |ui| {
                for r in Range::PRESETS {
                    ui.selectable_value(&mut st.range, r, r.label());
                }
                let (s, e) = st.range.dates(b);
                if ui
                    .selectable_label(matches!(st.range, Range::Custom(..)), "Custom…")
                    .clicked()
                {
                    st.range = Range::Custom(s, e);
                }
            });
        if let Range::Custom(s, e) = st.range {
            room(ui, 250.0);
            let (mut js, mut je) = (to_jiff(s), to_jiff(e));
            ui.add(
                egui_extras::DatePickerButton::new(&mut js)
                    .id_salt("from")
                    .calendar_week(false),
            );
            ui.label(RichText::new("to").color(p.muted));
            ui.add(
                egui_extras::DatePickerButton::new(&mut je)
                    .id_salt("to")
                    .calendar_week(false),
            );
            let (s, e) = (from_jiff(js), from_jiff(je));
            st.range = Range::Custom(s.min(e), e.max(s));
        }
        if splittable {
            room(ui, 270.0);
            widgets::segmented(
                ui,
                &mut st.columns,
                &[
                    (Columns::Total, "Total"),
                    (Columns::Month, "Months"),
                    (Columns::Quarter, "Quarters"),
                    (Columns::Year, "Years"),
                ],
            );
        }
        if comparable && (st.columns == Columns::Total || !splittable) {
            room(ui, 160.0);
            egui::ComboBox::from_id_salt("compare")
                .selected_text(match st.compare {
                    Compare::None => "No comparison",
                    Compare::PriorPeriod => "vs prior period",
                    Compare::PriorYear => "vs prior year",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut st.compare, Compare::None, "No comparison");
                    ui.selectable_value(&mut st.compare, Compare::PriorPeriod, "vs prior period");
                    ui.selectable_value(&mut st.compare, Compare::PriorYear, "vs prior year");
                });
        }
    });
    ui.add_space(18.0);

    let (s, e) = st.range.dates(b);
    let state = State {
        range: st.range,
        columns: if splittable {
            st.columns
        } else {
            Columns::Total
        },
        compare: if comparable && (st.columns == Columns::Total || !splittable) {
            st.compare
        } else {
            Compare::None
        },
        ..Default::default()
    };
    let cols = column_ranges(&state, b);
    let compare = state.compare != Compare::None;
    let subtitle = if point_in_time {
        format!("As of {}", fmt::date(e))
    } else {
        format!("{} – {}", fmt::date(s), fmt::date(e))
    };

    ui.label(RichText::new(kind.title()).font(theme::bold(26.0)));
    ui.label(
        RichText::new(format!(
            "{} · {} · {} basis",
            b.company_name(),
            subtitle,
            b.settings.company.basis
        ))
        .font(theme::regular(14.0))
        .color(p.muted),
    );
    ui.add_space(14.0);

    let table = match kind {
        ReportKind::ProfitAndLoss => {
            let mut t = pnl(b, &cols, state.columns != Columns::Total);
            if compare {
                t.add_change();
            } else if cols.len() == 1 {
                t.add_share_of_income();
            }
            Some(t)
        }
        ReportKind::BalanceSheet => {
            let dates: Vec<(NaiveDate, String)> = if state.columns == Columns::Total {
                cols.iter()
                    .map(|(_, e, _)| (*e, format!("As of {}", fmt::date_short(*e))))
                    .collect()
            } else {
                cols.iter()
                    .map(|(_, e, title)| (*e, title.clone()))
                    .collect()
            };
            let mut t = balance_sheet(b, &dates);
            if compare {
                t.add_change();
            }
            Some(t)
        }
        ReportKind::CashFlow => {
            let mut t = cash_flow_table(b, &cols, state.columns != Columns::Total);
            if compare {
                t.add_change();
            }
            Some(t)
        }
        ReportKind::TrialBalance => Some(trial_balance(b, e)),
        ReportKind::FixedAssetRegister => Some(asset_register(b, e)),
        ReportKind::MembersEquity => Some(owners_equity(b, s, e)),
        ReportKind::TaxSummary => {
            let mut t = tax(b, s, e);
            if compare && let Some((ps, pe)) = prior(&state, s, e) {
                let prev = tax(b, ps, pe);
                // Side by side: the tax column now and then.
                let mut m = Table::new(vec![
                    cols[0].2.clone(),
                    cols.get(1).map(|c| c.2.clone()).unwrap_or_default(),
                ]);
                for r in &t.rows {
                    let other = prev
                        .rows
                        .iter()
                        .find(|x| x.label == r.label && x.kind == r.kind)
                        .and_then(|x| x.cells.get(1))
                        .cloned()
                        .unwrap_or(Cell::none());
                    let cells = if r.cells.is_empty() {
                        Vec::new()
                    } else {
                        vec![r.cells.get(1).cloned().unwrap_or(Cell::none()), other]
                    };
                    m.rows.push(Row {
                        kind: r.kind,
                        label: r.label.clone(),
                        detail: r.detail.clone(),
                        cells,
                    });
                }
                m.add_change();
                t = m;
            }
            Some(t)
        }
        ReportKind::ArAging => Some(aging(b.ledger.ar_aging(e), e)),
        ReportKind::ApAging => Some(aging(b.ledger.ap_aging(e), e)),
        ReportKind::Vendor1099 => {
            vendor_1099(ui, b, e.year(), shell);
            None
        }
        ReportKind::GeneralLedger => {
            gl(ui, b, s, e, shell);
            None
        }
    };
    if let Some(t) = table {
        render(ui, &t, &PdfInfo::new(b, kind.title(), &subtitle), shell);
    }
    if let Some(k) = pick {
        shell.go(Route::Reports(k));
    }
}

/// The "Save PDF" button (busy while one is being made). Also collects a
/// finished job. True when clicked.
fn pdf_button(ui: &mut Ui, st: &mut State) -> bool {
    let p = pal(ui.ctx());
    if let Some(rx) = &st.pdf_job {
        if let Ok(done) = rx.try_recv() {
            st.pdf_job = None;
            st.pdf_done = Some((done, ui.input(|i| i.time)));
        } else {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
    }
    let busy = st.pdf_job.is_some();
    let label = if busy {
        format!("{} Making PDF…", icon::HOURGLASS)
    } else {
        format!("{} Save PDF", icon::FILE_PDF)
    };
    ui.add_enabled(
        !busy,
        egui::Button::new(
            RichText::new(label)
                .font(theme::regular(12.0))
                .color(p.muted),
        )
        .frame(false),
    )
    .on_hover_text(format!(
        "Typesets this report as it's set up here and saves it to {}, then opens it",
        crate::export::downloads_dir().display()
    ))
    .clicked()
}

/// Where the last PDF went (or why it didn't), for a few seconds.
fn pdf_status(ui: &mut Ui, st: &State) {
    let p = pal(ui.ctx());
    let Some((done, at)) = &st.pdf_done else {
        return;
    };
    if ui.input(|i| i.time) - at >= 8.0 {
        return;
    }
    let (text, color) = match done {
        Ok(path) => (
            format!("{} Saved {}", icon::CHECK, path.display()),
            p.positive,
        ),
        Err(e) => (
            format!("{} PDF failed: {e}", icon::WARNING_CIRCLE),
            p.negative,
        ),
    };
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
        ui.add(
            egui::Label::new(RichText::new(text).font(theme::regular(11.5)).color(color))
                .truncate(),
        );
    });
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_millis(500));
}

/// Typeset `t` in the background, save it to Downloads, and open it.
fn start_pdf(ctx: &egui::Context, st: &mut State, t: &Table, pdf: &PdfInfo) {
    let statement = t.statement(&pdf.title, &pdf.period);
    let meta = pdf.meta.clone();
    let name = format!("{} — {} — {}", meta.company, pdf.title, pdf.period);
    let (tx, rx) = std::sync::mpsc::channel();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let done = ledger_import::statement_pdf(&statement, &meta)
            .map_err(|e| e.to_string())
            .and_then(|bytes| crate::export::save_and_open(&name, "pdf", &bytes));
        let _ = tx.send(done);
        ctx.request_repaint();
    });
    st.pdf_job = Some(rx);
    st.pdf_done = None;
}

/// The general ledger as a table for the PDF: each account a section, a
/// line per posting (date, entry, description, the other side), its
/// beginning balance, and totals with the ending balance.
fn gl_table(g: &ledger_core::reports::GeneralLedger) -> Table {
    let mut t = Table::new(vec!["Debit".into(), "Credit".into(), "Balance".into()]);
    let amt = |v: Decimal| {
        if v.is_zero() {
            Cell::none()
        } else {
            Cell::of(v)
        }
    };
    for a in g
        .accounts
        .iter()
        .filter(|a| !a.postings.is_empty() || !a.beginning_balance.is_zero())
    {
        t.section(&format!("{} {}", a.code, a.name));
        t.row(
            Kind::Line,
            "Beginning balance",
            vec![Cell::none(), Cell::none(), Cell::of(a.beginning_balance)],
        );
        for post in &a.postings {
            t.row(
                Kind::Line,
                format!(
                    "{} · #{} · {} · {}",
                    fmt::date_short(post.date),
                    post.entry_id,
                    post.description,
                    post.split
                ),
                vec![amt(post.debit), amt(post.credit), Cell::of(post.balance)],
            );
        }
        t.row(
            Kind::Subtotal,
            "Totals and ending balance",
            vec![
                Cell::of(a.total_debits),
                Cell::of(a.total_credits),
                Cell::of(a.ending_balance),
            ],
        );
    }
    t
}

fn render(ui: &mut Ui, t: &Table, pdf: &PdfInfo, shell: &mut Shell<'_>) {
    let p = pal(ui.ctx());
    let tsv = t.tsv();
    let mut go: Option<Drill> = None;
    let st = &mut shell.state.reports;
    let mut save_pdf = false;
    widgets::card(ui, |ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
            save_pdf = pdf_button(ui, st);
            ui.add_space(8.0);
            if ui
                .add(
                    egui::Button::new(
                        RichText::new(format!("{} Copy for a spreadsheet", icon::COPY))
                            .font(theme::regular(12.0))
                            .color(p.muted),
                    )
                    .frame(false),
                )
                .on_hover_text("Copies the table as tab-separated values")
                .clicked()
            {
                ui.ctx().copy_text(tsv.clone());
            }
        });
        pdf_status(ui, st);
        let n = t.columns.len().max(1) as f32;
        let base: f32 = if t.columns.len() > 6 { 112.0 } else { 136.0 };
        // Narrow, the amount columns give up their slack before the table
        // scrolls sideways.
        let col_w = ((ui.available_width() - 180.0) / n).clamp(100.0, base);
        let cols_w = col_w * n;
        egui::ScrollArea::horizontal()
            .id_salt("report_table")
            .show(ui, |ui| {
                // Labels get what's left, but never so much that the eye loses
                // the line between a label and its amount.
                let label_w = (ui.available_width() - cols_w).clamp(180.0, 480.0);
                let width = label_w + cols_w;
                ui.set_min_width(width);
                let (hr, _) = ui.allocate_exact_size(vec2(width, 30.0), Sense::hover());
                for (i, c) in t.columns.iter().enumerate() {
                    let x = hr.left() + label_w + col_w * (i as f32 + 1.0) - 4.0;
                    ui.painter().text(
                        egui::pos2(x, hr.center().y),
                        egui::Align2::RIGHT_CENTER,
                        c,
                        theme::semibold(12.5),
                        p.muted,
                    );
                }
                ui.painter().line_segment(
                    [hr.left_bottom(), hr.right_bottom()],
                    Stroke::new(1.0, p.border),
                );
                for (ri, r) in t.rows.iter().enumerate() {
                    let h = match r.kind {
                        Kind::Section => 36.0,
                        Kind::Grand => 38.0,
                        Kind::Note => 32.0,
                        _ => {
                            if r.detail.is_empty() {
                                27.0
                            } else {
                                40.0
                            }
                        }
                    };
                    let (rr, resp) = ui.allocate_exact_size(vec2(width, h), Sense::hover());
                    if matches!(r.kind, Kind::Line) && resp.hovered() {
                        ui.painter().rect_filled(rr, CornerRadius::same(4), p.hover);
                    }
                    let (font, color, indent) = match r.kind {
                        Kind::Section => (theme::semibold(13.5), p.text, 0.0),
                        Kind::Line => (theme::regular(13.5), p.text, 18.0),
                        Kind::Subtotal => (theme::semibold(13.5), p.text, 0.0),
                        Kind::Grand => (theme::bold(14.5), p.text, 0.0),
                        Kind::Note => (theme::regular(12.0), p.muted, 0.0),
                    };
                    let y = match r.kind {
                        Kind::Section => rr.bottom() - 11.0,
                        _ if !r.detail.is_empty() => rr.center().y - 7.0,
                        _ => rr.center().y,
                    };
                    // Labels stop short of the first amount column (notes,
                    // which have none, run the full width).
                    let label_room = if r.cells.is_empty() {
                        width - indent - 8.0
                    } else {
                        label_w - indent - 12.0
                    };
                    widgets::clipped_text(
                        ui.painter(),
                        egui::pos2(rr.left() + indent + 4.0, y),
                        &r.label,
                        font.clone(),
                        color,
                        label_room,
                    );
                    if !r.detail.is_empty() {
                        widgets::clipped_text(
                            ui.painter(),
                            egui::pos2(rr.left() + indent + 4.0, rr.center().y + 9.0),
                            &r.detail,
                            theme::regular(11.5),
                            p.faint,
                            label_room,
                        );
                    }
                    for (i, c) in r.cells.iter().enumerate() {
                        let Some(v) = c.value else {
                            continue;
                        };
                        let is_pct = t.pct_from.is_some_and(|f| i >= f);
                        let text = if is_pct {
                            format!("{}%", v.normalize())
                        } else {
                            fmt::accounting(v)
                        };
                        let x1 = rr.left() + label_w + col_w * (i as f32 + 1.0) - 4.0;
                        let cell = egui::Rect::from_min_max(
                            egui::pos2(x1 - col_w + 8.0, rr.top()),
                            egui::pos2(x1, rr.bottom()),
                        );
                        let copy = fmt::plain(&text);
                        let cresp = ui.interact(
                            cell,
                            ui.id().with(("cell", ri, i)),
                            if c.drill.is_some() || copy.is_some() {
                                Sense::click()
                            } else {
                                Sense::hover()
                            },
                        );
                        // An amount copies as its plain decimal: a click
                        // where the cell doesn't drill down, the right-click
                        // menu where it does.
                        if let Some(v) = &copy {
                            if c.drill.is_some() {
                                cresp.context_menu(|ui| widgets::copy_item(ui, "Copy amount", v));
                            } else {
                                if cresp.hovered() {
                                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                }
                                if cresp.clicked() {
                                    widgets::copy_now(ui, cresp.id, v);
                                }
                                let note = if widgets::recently_copied(ui, cresp.id) {
                                    "Copied"
                                } else {
                                    "Click to copy"
                                };
                                cresp.clone().on_hover_text(note);
                            }
                        }
                        let hovered = c.drill.is_some() && cresp.hovered();
                        let vcolor = if hovered { p.accent } else { color };
                        let vfont = if matches!(r.kind, Kind::Line) {
                            theme::regular(13.5)
                        } else {
                            font.clone()
                        };
                        let g = ui.painter().layout_no_wrap(text, vfont, vcolor);
                        let pos = egui::pos2(x1 - g.size().x, y - g.size().y / 2.0);
                        if hovered {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                            ui.painter().line_segment(
                                [
                                    egui::pos2(pos.x, pos.y + g.size().y),
                                    egui::pos2(x1, pos.y + g.size().y),
                                ],
                                Stroke::new(1.0, p.accent),
                            );
                        }
                        ui.painter().galley(pos, g, vcolor);
                        let rule_x0 = x1 - col_w + 28.0;
                        if matches!(r.kind, Kind::Subtotal) {
                            ui.painter().line_segment(
                                [
                                    egui::pos2(rule_x0, rr.top() + 1.0),
                                    egui::pos2(x1, rr.top() + 1.0),
                                ],
                                Stroke::new(1.0, p.faint),
                            );
                        }
                        if matches!(r.kind, Kind::Grand) {
                            let painter = ui.painter();
                            painter.line_segment(
                                [
                                    egui::pos2(rule_x0, rr.top() + 2.0),
                                    egui::pos2(x1, rr.top() + 2.0),
                                ],
                                Stroke::new(1.0, p.text),
                            );
                            let yb = rr.bottom() - 5.0;
                            painter.line_segment(
                                [egui::pos2(rule_x0, yb), egui::pos2(x1, yb)],
                                Stroke::new(1.0, p.text),
                            );
                            painter.line_segment(
                                [egui::pos2(rule_x0, yb + 2.5), egui::pos2(x1, yb + 2.5)],
                                Stroke::new(1.0, p.text),
                            );
                        }
                        if let Some(d) = c.drill {
                            let cresp = cresp.on_hover_text(format!(
                                "Show the transactions behind this ({} – {})",
                                fmt::date(d.start),
                                fmt::date(d.end)
                            ));
                            if cresp.clicked() {
                                go = Some(d);
                            }
                        }
                    }
                }
            });
    });
    if save_pdf {
        start_pdf(ui.ctx(), st, t, pdf);
    }
    if let Some(d) = go {
        shell.state.accounts.range = Some((d.start, d.end));
        shell.go(Route::Account(d.account));
    }
}

fn gl(ui: &mut Ui, b: &Books, s: NaiveDate, e: NaiveDate, shell: &mut Shell<'_>) {
    let p = pal(ui.ctx());
    let Ok(g) = general_ledger(&b.ledger, s, e) else {
        return;
    };
    let st = &mut shell.state.reports;
    let mut save_pdf = false;
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
        save_pdf = pdf_button(ui, st);
    });
    pdf_status(ui, st);
    if save_pdf {
        let pdf = PdfInfo::new(
            b,
            ReportKind::GeneralLedger.title(),
            &format!("{} – {}", fmt::date(s), fmt::date(e)),
        );
        start_pdf(ui.ctx(), st, &gl_table(&g), &pdf);
    }
    ui.add_space(6.0);
    let mut go: Option<Route> = None;
    for a in g
        .accounts
        .iter()
        .filter(|a| !a.postings.is_empty() || !a.beginning_balance.is_zero())
    {
        widgets::card(ui, |ui| {
            ui.horizontal(|ui| {
                let r = widgets::link(
                    ui,
                    RichText::new(format!("{} {}", a.code, a.name)).font(theme::semibold(14.0)),
                );
                if r.clicked()
                    && let Some(acct) = b.ledger.chart().find_by_code(&a.code)
                {
                    go = Some(Route::Account(acct.id));
                }
                ui.label(
                    RichText::new(format!("{:?}", a.account_type))
                        .font(theme::regular(12.0))
                        .color(p.muted),
                );
            });
            ui.add_space(6.0);
            // Narrow, the description gives way down to 120 points, then
            // the table scrolls sideways.
            let fixed = [86.0, 52.0, 0.0, 110.0, 110.0, 120.0];
            let w = ui.available_width().max(fixed.iter().sum::<f32>() + 120.0);
            let flex = w - fixed.iter().sum::<f32>();
            widgets::min_width(ui, &format!("gl_{}", a.code), w, |ui| {
                let row = |ui: &mut Ui,
                           vals: [String; 6],
                           strong: bool,
                           color: egui::Color32|
                 -> egui::Response {
                    let (r, resp) = ui.allocate_exact_size(vec2(w, 24.0), Sense::click());
                    resp.context_menu(|ui| {
                        for v in &vals {
                            if let Some(id) = v.strip_prefix('#') {
                                widgets::copy_item(ui, "Copy entry number", &format!("#{id}"));
                            } else if let Some(a) = fmt::plain(v) {
                                widgets::copy_item(ui, &format!("Copy {v}"), &a);
                            }
                        }
                    });
                    if resp.hovered() && !strong {
                        ui.painter().rect_filled(r, CornerRadius::same(4), p.hover);
                    }
                    let painter = ui.painter_at(r);
                    let mut x = r.left();
                    for (i, v) in vals.iter().enumerate() {
                        let cw = if i == 2 { flex } else { fixed[i] };
                        let font = if strong {
                            theme::semibold(12.5)
                        } else {
                            theme::regular(12.5)
                        };
                        if i >= 3 {
                            painter.text(
                                egui::pos2(x + cw - 4.0, r.center().y),
                                egui::Align2::RIGHT_CENTER,
                                v,
                                font,
                                color,
                            );
                        } else {
                            widgets::clipped_text(
                                &painter,
                                egui::pos2(x + 4.0, r.center().y),
                                v,
                                font,
                                color,
                                cw - 8.0,
                            );
                        }
                        x += cw;
                    }
                    resp
                };
                row(
                    ui,
                    [
                        "Date".into(),
                        "#".into(),
                        "Description".into(),
                        "Debit".into(),
                        "Credit".into(),
                        "Balance".into(),
                    ],
                    true,
                    p.muted,
                );
                row(
                    ui,
                    [
                        String::new(),
                        String::new(),
                        "Beginning balance".into(),
                        String::new(),
                        String::new(),
                        fmt::accounting(a.beginning_balance),
                    ],
                    false,
                    p.muted,
                );
                for post in &a.postings {
                    let r = row(
                        ui,
                        [
                            fmt::date_short(post.date),
                            post.entry_id.to_string(),
                            format!("{} · {}", post.description, post.split),
                            if post.debit.is_zero() {
                                String::new()
                            } else {
                                fmt::money(post.debit)
                            },
                            if post.credit.is_zero() {
                                String::new()
                            } else {
                                fmt::money(post.credit)
                            },
                            fmt::accounting(post.balance),
                        ],
                        false,
                        p.text,
                    );
                    if r.clicked() {
                        go = Some(Route::Entry(post.entry_id));
                    }
                }
                row(
                    ui,
                    [
                        String::new(),
                        String::new(),
                        "Totals and ending balance".into(),
                        fmt::money(a.total_debits),
                        fmt::money(a.total_credits),
                        fmt::accounting(a.ending_balance),
                    ],
                    true,
                    p.text,
                );
            });
        });
        ui.add_space(12.0);
    }
    if let Some(r) = go {
        if let Route::Account(_) = r {
            shell.state.accounts.range = Some((s, e));
        }
        shell.go(r);
    }
}

fn vendor_1099(ui: &mut Ui, b: &Books, year: i32, shell: &mut Shell<'_>) {
    let p = pal(ui.ctx());
    let r = match vendor_1099_report(&b.ledger, year, None) {
        Ok(r) => r,
        Err(e) => {
            // Never guess a threshold (rules.md B-19): say why there's no
            // worksheet, in the engine's words.
            ui.label(RichText::new(e.to_string()).color(p.muted));
            return;
        }
    };
    ui.label(
        RichText::new(format!(
            "Calendar {year}. Threshold {} ($600 through 2025; $2,000 from 2026, indexed after). Counts cash paid on entries tagged with the vendor; card payments are reported by the card processor (1099-K) and drop out.",
            fmt::dollars(r.threshold)
        ))
        .color(p.muted),
    );
    ui.add_space(10.0);
    let mut t = Table::new(vec!["Paid".into()]);
    for v in &r.vendors {
        let form = v.form.map(|f| f.label()).unwrap_or("No 1099");
        let w9 = if v.tin_on_file {
            "W-9 on file"
        } else {
            "no W-9"
        };
        let status = if v.reportable {
            if v.tin_on_file {
                "File a 1099"
            } else {
                "File a 1099 · get a W-9 first"
            }
        } else {
            "Not reportable"
        };
        t.rows.push(Row {
            kind: Kind::Line,
            label: v.name.clone(),
            detail: format!("{form} · {w9} · {status}"),
            cells: vec![Cell::of(v.paid)],
        });
    }
    if r.vendors.is_empty() {
        t.note("No vendors marked for a 1099 and none paid this year.");
    }
    let pdf = PdfInfo::new(
        b,
        ReportKind::Vendor1099.title(),
        &format!("Calendar year {year}"),
    );
    render(ui, &t, &pdf, shell);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fixed asset register report's tax columns are the register's:
    /// each held asset's contributor's basis, tax basis, and built-in gain
    /// (ledger-core's Asset::tax_basis and built_in_gain), the total its
    /// total_tax_basis, and its rule (cited) in the detail.
    #[test]
    fn asset_register_report_carries_tax_basis() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 12).unwrap();
        let dir = std::env::temp_dir().join(format!("ledger-gui-tax-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        crate::demo::build(&dir, today).unwrap();
        let b = crate::data::load_once(&crate::data::Source {
            data_dir: dir.join("books-data"),
            books_toml: Some(dir.join("wanderbooks.toml")),
            today: Some(today),
        })
        .unwrap();
        let r = ledger_core::fixed_asset_register(&b.ledger, today);
        assert!(!r.rows.is_empty(), "the demo has an asset");
        assert_eq!(r.missing_tax_basis, 0, "the demo's asset has a tax basis");
        let t = asset_register(&b, today);
        assert_eq!(
            t.columns,
            [
                "Cost",
                "Accumulated",
                "Book value",
                "Contributor's basis",
                "Tax basis",
                "Built-in gain (loss)"
            ]
        );
        for x in &r.rows {
            let a = b.ledger.get_asset(x.asset_id).unwrap();
            let row = t
                .rows
                .iter()
                .find(|row| row.kind == Kind::Line && row.label.contains(&x.description))
                .expect("a line per asset");
            let v: Vec<Option<Decimal>> = row.cells.iter().map(|c| c.value).collect();
            assert_eq!(
                v,
                [
                    Some(x.cost),
                    Some(x.accumulated),
                    Some(x.book_value),
                    a.tax.and_then(|t| t.contributor_basis),
                    a.tax_basis(),
                    a.built_in_gain(),
                ]
            );
            assert!(row.detail.contains("tax basis: Cost"), "{}", row.detail);
        }
        let total = t.rows.iter().find(|row| row.label == "Total held").unwrap();
        assert_eq!(total.cells[4].value, Some(r.total_tax_basis));
        assert_eq!(total.cells[4].value, r.rows[0].tax_basis);
        assert!(
            t.rows
                .iter()
                .all(|row| row.cells.is_empty() || row.cells.len() == 6)
        );
        assert!(
            t.rows
                .iter()
                .any(|row| row.kind == Kind::Note && row.label.contains("fair market value"))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Every table-backed report typesets as a PDF, in its plain, by-month,
    /// and compared layouts. Set `PDF_DIR` to keep the files for eyes.
    #[test]
    fn reports_typeset_as_pdf() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 12).unwrap();
        let dir = std::env::temp_dir().join(format!("ledger-gui-pdf-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        crate::demo::build(&dir, today).unwrap();
        let b = crate::data::load_once(&crate::data::Source {
            data_dir: dir.join("books-data"),
            books_toml: Some(dir.join("wanderbooks.toml")),
            today: Some(today),
        })
        .unwrap();
        let ytd = |columns, compare| State {
            range: Range::YearToDate,
            columns,
            compare,
            ..Default::default()
        };
        let (s, e) = Range::YearToDate.dates(&b);
        let mut tables: Vec<(&str, Table)> = Vec::new();
        let mut t = pnl(
            &b,
            &column_ranges(&ytd(Columns::Total, Compare::None), &b),
            false,
        );
        t.add_share_of_income();
        tables.push(("pnl", t));
        let cols = column_ranges(&ytd(Columns::Month, Compare::None), &b);
        tables.push(("pnl-months", pnl(&b, &cols, true)));
        let mut t = pnl(
            &b,
            &column_ranges(&ytd(Columns::Total, Compare::PriorYear), &b),
            false,
        );
        t.add_change();
        tables.push(("pnl-vs-prior-year", t));
        tables.push(("balance", balance_sheet(&b, &[(e, "Total".into())])));
        let cols = column_ranges(&ytd(Columns::Total, Compare::None), &b);
        tables.push(("cashflow", cash_flow_table(&b, &cols, false)));
        tables.push(("trial", trial_balance(&b, e)));
        tables.push(("equity", owners_equity(&b, s, e)));
        tables.push(("tax", tax(&b, s, e)));
        tables.push(("ar", aging(b.ledger.ar_aging(e), e)));
        tables.push(("assets", asset_register(&b, e)));
        tables.push(("gl", gl_table(&general_ledger(&b.ledger, s, e).unwrap())));
        let keep = std::env::var_os("PDF_DIR").map(std::path::PathBuf::from);
        for (name, t) in tables {
            let info = PdfInfo::new(&b, name, "Jan 1 – Oct 12, 2026");
            let bytes =
                ledger_import::statement_pdf(&t.statement(&info.title, &info.period), &info.meta)
                    .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(bytes.starts_with(b"%PDF"), "{name}");
            if let Some(d) = &keep {
                std::fs::create_dir_all(d).unwrap();
                std::fs::write(d.join(format!("{name}.pdf")), bytes).unwrap();
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
