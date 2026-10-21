//! Spec-blind tests for `ledger-import`.
//!
//! Every expected value here comes from the written specs (the ```text
//! GOAL/SOURCE/INVARIANTS/SCENARIOS blocks and the public doc comments) and
//! the sources they cite (docs/spec/sources/intuit-bank-csv.md,
//! intuit-iif.md, xlsx-numbers.md, irs-tax-lines.md), never from the
//! implementation. Each test names the clause it checks.
//!
//! The XLSX checks open the workbook with a small zip reader and DEFLATE
//! decoder written here (RFC 1951), so the cells themselves are the
//! evidence, not the writer's word.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use chrono::{Datelike, NaiveDate};
use ledger_core::reports::{balance_sheet_as_of, profit_and_loss_for_period};
use ledger_core::{
    AccountId, AccountType, CashFlowParams, Currency, EntryId, JournalEntry, Ledger, MemberCapital,
    Money, Side, cash_flow, general_ledger, members_equity, trial_balance_as_of,
};
use ledger_import::chart_csv::ChartCsvError;
use ledger_import::report::{self, Member, ReportMeta, Row, RowKind, Statement, YearEarnings};
use ledger_import::{
    BankCsvError, BankTransaction, DateOrder, EntityType, Iif, IifAccount, IifChange, IifError,
    IifField, LeftOut, MEMO_MAX, NAME_MAX, NUMBER_MAX, QbAccountType, XlsxReport, chart_template,
    format_amount, iif_accounts, journal_to_iif, parse_bank_csv, parse_bank_csv_with,
    parse_chart_csv, report_to_xlsx, statement_pdf,
};
use proptest::prelude::*;
use rust_decimal::{Decimal, RoundingStrategy};

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn dec(s: &str) -> Decimal {
    s.parse().unwrap()
}

fn cents(c: i64) -> Decimal {
    Decimal::new(c, 2)
}

fn usd(c: i64) -> Money {
    Money::new(cents(c), Currency::USD)
}

fn meta() -> ReportMeta {
    ReportMeta {
        company: "Spec Test LLC".into(),
        basis: "Accrual".into(),
        prepared_on: d(2026, 10, 13),
        journal_seq: 42,
    }
}

/// The chart every random book uses: every account type, a subaccount, a
/// name holding a colon, non-ASCII punctuation and letters, two members'
/// capital and distributions accounts, and one equity account that isn't a
/// member's (it feeds "Other changes").
const CHART: &[(&str, &str, AccountType, Option<&str>)] = &[
    ("1000", "Cash", AccountType::Asset, None),
    ("1010", "Reserve: Savings", AccountType::Asset, Some("1000")),
    ("1100", "Receivables", AccountType::Asset, None),
    ("1500", "Equipment", AccountType::Asset, None),
    ("2000", "Card", AccountType::Liability, None),
    ("2100", "Loan", AccountType::Liability, None),
    ("3000", "Capital — Ada", AccountType::Equity, None),
    ("3001", "Capital — Grace", AccountType::Equity, None),
    ("3100", "Distributions — Ada", AccountType::Equity, None),
    ("3101", "Distributions — Grace", AccountType::Equity, None),
    ("3900", "Other equity", AccountType::Equity, None),
    ("4000", "Sales", AccountType::Revenue, None),
    ("4100", "Interest “income”", AccountType::Revenue, None),
    ("6000", "Rent", AccountType::Expense, None),
    ("6010", "Café meals", AccountType::Expense, None),
    ("6020", "Fees", AccountType::Expense, None),
];
const OTHER_EQUITY: usize = 10;

/// One random balanced entry: a day offset from 2025-07-01, distinct
/// accounts (the first `k` debited, the rest credited), the debit amounts in
/// cents, and seeds that split their total across the credits.
#[derive(Clone, Debug)]
struct EntrySpec {
    day: i64,
    accts: Vec<usize>,
    k: usize,
    debits: Vec<i64>,
    seeds: Vec<u64>,
    memo: String,
}

fn entry_spec() -> impl Strategy<Value = EntrySpec> {
    (
        0i64..549,
        proptest::sample::subsequence((0..CHART.len()).collect::<Vec<_>>(), 2..=4),
        any::<u64>(),
        proptest::collection::vec(100i64..10_000_000_000, 3),
        proptest::collection::vec(any::<u64>(), 3),
        "[A-Za-z0-9 :\t\n\r#*$é—“”’]{0,16}",
    )
        .prop_map(|(day, mut accts, shuffle, debits, seeds, memo)| {
            // Rotate so either side may hold any account.
            let r = (shuffle % accts.len() as u64) as usize;
            accts.rotate_left(r);
            let k = 1 + (shuffle / 7 % (accts.len() as u64 - 1)) as usize;
            EntrySpec {
                day,
                accts,
                k,
                debits,
                seeds,
                memo,
            }
        })
}

struct Books {
    ledger: Ledger,
    ids: Vec<AccountId>,
}

fn build_books(specs: &[EntrySpec], other_equity: bool) -> Books {
    let mut l = Ledger::new(Currency::USD);
    let mut ids = Vec::new();
    for (code, name, ty, parent) in CHART {
        let p = parent.map(|pc| l.chart().find_by_code(pc).unwrap().id);
        ids.push(l.add_account(code, name, *ty, p, false).unwrap());
    }
    for s in specs {
        if !other_equity && s.accts.contains(&OTHER_EQUITY) {
            continue;
        }
        let date = d(2025, 7, 1) + chrono::Duration::days(s.day);
        let e = l.create_entry(date, &s.memo);
        let dr = &s.accts[..s.k];
        let cr = &s.accts[s.k..];
        let mut total = 0i64;
        for (i, a) in dr.iter().enumerate() {
            l.add_line(e, ids[*a], Side::Debit, usd(s.debits[i]))
                .unwrap();
            total += s.debits[i];
        }
        let m = cr.len();
        let mut remaining = total;
        for (i, a) in cr.iter().enumerate() {
            let part = if i + 1 == m {
                remaining
            } else {
                let max = remaining - (m - 1 - i) as i64;
                1 + (s.seeds[i] % max as u64) as i64
            };
            remaining -= part;
            l.add_line(e, ids[*a], Side::Credit, usd(part)).unwrap();
        }
        l.post_entry(e).unwrap();
    }
    Books { ledger: l, ids }
}

fn members() -> Vec<Member> {
    vec![
        Member {
            name: "Ada".into(),
            capital_code: "3000".into(),
            distribution_code: Some("3100".into()),
        },
        Member {
            name: "Grace".into(),
            capital_code: "3001".into(),
            distribution_code: Some("3101".into()),
        },
    ]
}

fn member_capitals(b: &Books) -> Vec<MemberCapital> {
    let half = dec("0.5");
    vec![
        MemberCapital {
            name: "Ada".into(),
            capital_account: b.ids[6],
            distribution_account: Some(b.ids[8]),
            share: half,
        },
        MemberCapital {
            name: "Grace".into(),
            capital_account: b.ids[7],
            distribution_account: Some(b.ids[9]),
            share: half,
        },
    ]
}

fn amt(r: &Row, col: usize) -> Decimal {
    r.amounts
        .get(col)
        .copied()
        .flatten()
        .unwrap_or(Decimal::ZERO)
}

fn find<'a>(st: &'a Statement, kind: RowKind, label: &str) -> Option<&'a Row> {
    st.rows.iter().find(|r| r.kind == kind && r.label == label)
}

/// Footing, the reader's check: every subtotal that closes a section opened
/// by a Section heading equals the sum of the Line rows printed in that
/// section, column by column. Returns the (section, subtotal) pairs it
/// checked.
fn assert_sections_foot(st: &Statement) -> Vec<(String, String)> {
    let mut checked = Vec::new();
    let mut section: Option<String> = None;
    let mut sums: Vec<Decimal> = vec![Decimal::ZERO; st.columns.len()];
    for r in &st.rows {
        match r.kind {
            RowKind::Section => {
                section = Some(r.label.clone());
                sums = vec![Decimal::ZERO; st.columns.len()];
            }
            RowKind::Line => {
                for (c, s) in sums.iter_mut().enumerate() {
                    *s += amt(r, c);
                }
            }
            RowKind::Subtotal | RowKind::Total => {
                if let Some(sec) = section.take() {
                    for (c, s) in sums.iter().enumerate() {
                        if st.percent_from.is_some_and(|p| c >= p) {
                            continue;
                        }
                        assert_eq!(
                            amt(r, c),
                            *s,
                            "{}: lines of {sec:?} don't foot to {:?} (col {c})",
                            st.title,
                            r.label
                        );
                    }
                    checked.push((sec, r.label.clone()));
                }
                sums = vec![Decimal::ZERO; st.columns.len()];
            }
            // A detail's amount is part of the line above it, not of the
            // section (cash flow kinds).
            RowKind::Subheading | RowKind::Detail | RowKind::Spacer => {}
        }
    }
    checked
}

/// The Line rows of the section headed `label`.
fn section_lines<'a>(st: &'a Statement, label_contains: &str) -> Vec<&'a Row> {
    let mut out = Vec::new();
    let mut inside = false;
    for r in &st.rows {
        match r.kind {
            RowKind::Section => inside = r.label.contains(label_contains),
            RowKind::Subtotal | RowKind::Total => inside = false,
            RowKind::Line if inside => out.push(r),
            _ => {}
        }
    }
    out
}

// ---------------------------------------------------------------------------
// report::format_amount
// ---------------------------------------------------------------------------

fn unformat(s: &str) -> Decimal {
    let neg = s.starts_with('(') && s.ends_with(')');
    let body: String = s
        .trim_start_matches('(')
        .trim_end_matches(')')
        .chars()
        .filter(|c| *c != ',')
        .collect();
    let v: Decimal = body.parse().unwrap_or_else(|_| panic!("unparsable {s:?}"));
    if neg { -v } else { v }
}

fn assert_accounting_shape(s: &str) {
    assert!(!s.contains('-'), "{s:?}: a negative must be in parentheses");
    let inner = s.trim_start_matches('(').trim_end_matches(')');
    assert_eq!(
        s.starts_with('('),
        s.ends_with(')'),
        "{s:?}: unbalanced parentheses"
    );
    let (int, frac) = inner.split_once('.').expect("a decimal point");
    assert_eq!(frac.len(), 2, "{s:?}: exactly two decimals");
    assert!(frac.chars().all(|c| c.is_ascii_digit()));
    let groups: Vec<&str> = int.split(',').collect();
    assert!(
        !groups[0].is_empty() && groups[0].len() <= 3,
        "{s:?}: bad first group"
    );
    assert!(groups[0].chars().all(|c| c.is_ascii_digit()));
    assert!(
        groups.len() == 1 || !groups[0].starts_with('0'),
        "{s:?}: leading zero before a comma"
    );
    for g in &groups[1..] {
        assert_eq!(g.len(), 3, "{s:?}: thousands groups are three digits");
        assert!(g.chars().all(|c| c.is_ascii_digit()));
    }
}

/// format_amount INVARIANTS 1-3: two decimals rounded half away from zero,
/// commas by thousands, negatives in parentheses; never "(0.00)"; stripping
/// commas and parentheses gives back the amount rounded to the cent.
#[test]
fn format_amount_examples_from_the_spec() {
    assert_eq!(format_amount(dec("1234.56")), "1,234.56");
    assert_eq!(format_amount(dec("-318.42")), "(318.42)");
    assert_eq!(format_amount(Decimal::ZERO), "0.00");
    // INVARIANT 2's own finding: -0.004 printed "(0.00)".
    assert_eq!(format_amount(dec("-0.004")), "0.00");
    assert_eq!(format_amount(dec("-0.00")), "0.00");
    // Half away from zero, both signs.
    assert_eq!(format_amount(dec("0.005")), "0.01");
    assert_eq!(format_amount(dec("-0.005")), "(0.01)");
    assert_eq!(format_amount(dec("2.675")), "2.68");
    assert_eq!(format_amount(dec("-1234567.895")), "(1,234,567.90)");
    assert_eq!(format_amount(dec("999.995")), "1,000.00");
    assert_eq!(format_amount(dec("100000")), "100,000.00");
    assert_eq!(format_amount(dec("12")), "12.00");
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    /// format_amount INVARIANTS 1-3 over random amounts of any scale.
    #[test]
    fn format_amount_round_trips_to_the_cent(
        m in -100_000_000_000_000_000_000i128..100_000_000_000_000_000_000i128,
        scale in 0u32..=8,
    ) {
        let x = Decimal::from_i128_with_scale(m, scale);
        let s = format_amount(x);
        assert_accounting_shape(&s);
        let rounded = x.round_dp_with_strategy(2, RoundingStrategy::MidpointAwayFromZero);
        prop_assert_eq!(unformat(&s), rounded);
        prop_assert!(s != "(0.00)");
        prop_assert_eq!(s.starts_with('('), rounded < Decimal::ZERO);
    }
}

// ---------------------------------------------------------------------------
// report::statement (finishing marks)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
enum RowGen {
    Section,
    Subheading,
    Line(Vec<i64>),
    Subtotal(Vec<i64>),
    Spacer,
}

fn row_gen(cols: usize) -> impl Strategy<Value = RowGen> {
    prop_oneof![
        Just(RowGen::Section),
        Just(RowGen::Subheading),
        proptest::collection::vec(-1_000_000i64..1_000_000, cols).prop_map(RowGen::Line),
        proptest::collection::vec(-1_000_000i64..1_000_000, cols).prop_map(RowGen::Subtotal),
        Just(RowGen::Spacer),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]

    /// statement INVARIANTS 1-3: currency signs exactly on subtotals and
    /// totals; a rule above every total, and above a subtotal only when a
    /// line or subtotal sits above it in its section; rows, labels, and
    /// amounts kept exactly.
    #[test]
    fn statement_finishing_marks(
        cols in 1usize..=3,
        gens in proptest::collection::vec(row_gen(3), 0..30),
        flags in proptest::collection::vec(any::<(bool, bool)>(), 31),
        with_total in any::<bool>(),
    ) {
        let mut rows: Vec<Row> = gens
            .iter()
            .enumerate()
            .map(|(i, g)| {
                let (kind, amounts) = match g {
                    RowGen::Section => (RowKind::Section, vec![]),
                    RowGen::Subheading => (RowKind::Subheading, vec![]),
                    RowGen::Spacer => (RowKind::Spacer, vec![]),
                    RowGen::Line(a) => (RowKind::Line, a[..cols].iter().map(|c| Some(cents(*c))).collect()),
                    RowGen::Subtotal(a) => (RowKind::Subtotal, a[..cols].iter().map(|c| Some(cents(*c))).collect()),
                };
                Row { kind, label: format!("row {i} *#$["), amounts, currency: flags[i].0, rule_above: flags[i].1 }
            })
            .collect();
        if with_total {
            rows.push(Row {
                kind: RowKind::Total,
                label: "Grand total".into(),
                amounts: vec![Some(cents(1)); cols],
                currency: false,
                rule_above: false,
            });
        }
        let col_names: Vec<String> = (0..cols).map(|c| format!("C{c}")).collect();
        let col_refs: Vec<&str> = col_names.iter().map(String::as_str).collect();
        let st = report::statement("Title", "Period".into(), &col_refs, rows.clone());
        prop_assert_eq!(&st.title, "Title");
        prop_assert_eq!(&st.period, "Period");
        prop_assert_eq!(&st.columns, &col_names);
        prop_assert_eq!(st.rows.len(), rows.len());
        let mut amounts_in_section = false;
        for (got, want) in st.rows.iter().zip(&rows) {
            prop_assert_eq!(got.kind, want.kind);
            prop_assert_eq!(&got.label, &want.label);
            prop_assert_eq!(&got.amounts, &want.amounts);
            let is_sum = matches!(got.kind, RowKind::Subtotal | RowKind::Total);
            prop_assert_eq!(got.currency, is_sum, "currency on {:?}", got.label);
            match got.kind {
                RowKind::Total => prop_assert!(got.rule_above),
                RowKind::Subtotal => prop_assert_eq!(got.rule_above, amounts_in_section, "rule on {:?}", got.label),
                _ => {}
            }
            match got.kind {
                RowKind::Section => amounts_in_section = false,
                RowKind::Line | RowKind::Subtotal => amounts_in_section = true,
                _ => {}
            }
        }
    }
}

/// statement INVARIANT 2: a subtotal straight under its heading (nothing to
/// sum) gets no rule.
#[test]
fn statement_subtotal_with_nothing_above_has_no_rule() {
    let row = |kind, label: &str, a: Vec<Option<Decimal>>| Row {
        kind,
        label: label.into(),
        amounts: a,
        currency: false,
        rule_above: true,
    };
    let st = report::statement(
        "T",
        "P".into(),
        &["Total"],
        vec![
            row(RowKind::Section, "Investing", vec![]),
            row(
                RowKind::Subtotal,
                "Net investing",
                vec![Some(Decimal::ZERO)],
            ),
        ],
    );
    assert!(!st.rows[1].rule_above);
    assert!(st.rows[1].currency);
    assert!(!st.rows[0].currency);
}

// ---------------------------------------------------------------------------
// The five statements, from random books
// ---------------------------------------------------------------------------

fn books_strategy() -> impl Strategy<Value = (Vec<EntrySpec>, bool, i64)> {
    (
        proptest::collection::vec(entry_spec(), 0..25),
        any::<bool>(),
        0i64..365,
    )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// profit_and_loss INVARIANTS 1-2: revenue lines foot to "Total revenue",
    /// expense lines to "Total expenses", "Net income (loss)" = revenue -
    /// expenses; one "CODE Name" line per account in the statement's order.
    #[test]
    fn pl_foots_as_printed((specs, oe, end_off) in books_strategy()) {
        let b = build_books(&specs, oe);
        let end = d(2026, 1, 1) + chrono::Duration::days(end_off);
        let pl = profit_and_loss_for_period(&b.ledger, d(2026, 1, 1), end, None).unwrap();
        let st = report::profit_and_loss(&pl);
        assert_sections_foot(&st);
        let rev = find(&st, RowKind::Subtotal, "Total revenue").map_or(Decimal::ZERO, |r| amt(r, 0));
        let exp = find(&st, RowKind::Subtotal, "Total expenses").map_or(Decimal::ZERO, |r| amt(r, 0));
        let net = find(&st, RowKind::Total, "Net income (loss)").expect("a net income total");
        prop_assert_eq!(amt(net, 0), rev - exp);
        prop_assert_eq!(rev, pl.total_revenue);
        prop_assert_eq!(exp, pl.total_expenses);
        // INVARIANT 2: one line per account, "CODE Name", in order.
        let want: Vec<(String, Decimal)> = pl.revenue.iter().chain(&pl.expenses)
            .map(|l| (format!("{} {}", l.code, l.name), l.amount)).collect();
        let got: Vec<(String, Decimal)> = st.rows.iter().filter(|r| r.kind == RowKind::Line)
            .map(|r| (r.label.clone(), amt(r, 0))).collect();
        prop_assert_eq!(got, want);
    }

    /// balance_sheet INVARIANTS 1-3 (with and without members): every
    /// section foots; total L+E = total liabilities + total equity = total
    /// assets; members get "Members' equity" with each capital account
    /// listed, otherwise "Retained earnings" and the year's "Net income
    /// (loss)" (prose: RE plus earlier years' unclosed earnings; the year's
    /// unclosed income).
    #[test]
    fn balance_sheet_foots_and_balances((specs, oe, end_off) in books_strategy()) {
        let b = build_books(&specs, oe);
        let as_of = d(2026, 1, 1) + chrono::Duration::days(end_off);
        let bs = balance_sheet_as_of(&b.ledger, as_of).unwrap();
        let ytd = profit_and_loss_for_period(&b.ledger, d(2026, 1, 1), as_of, None).unwrap().net_income();
        // No closes in these books: the whole year is unclosed.
        let ye = YearEarnings { net_income_ytd: ytd, unclosed: ytd };
        for with_members in [false, true] {
            let ms = if with_members { members() } else { vec![] };
            let st = report::balance_sheet(&bs, ye, &ms);
            let checked = assert_sections_foot(&st);
            let (eq_section, eq_total_label, total_label) = if with_members {
                ("Members' equity", "Total members' equity", "Total liabilities and members' equity")
            } else {
                ("Equity", "Total equity", "Total liabilities and equity")
            };
            prop_assert!(checked.iter().any(|(s, _)| s == eq_section), "{:?}", checked);
            let ta = amt(find(&st, RowKind::Subtotal, "Total assets").expect("Total assets"), 0);
            let tl = amt(find(&st, RowKind::Subtotal, "Total liabilities").expect("Total liabilities"), 0);
            let te = checked.iter().find(|(s, _)| s == eq_section).map(|(_, t)| t.clone()).unwrap();
            prop_assert_eq!(&te, eq_total_label);
            let te = amt(find(&st, RowKind::Subtotal, &te).unwrap(), 0);
            let tle = amt(find(&st, RowKind::Total, total_label).expect("L+E total"), 0);
            prop_assert_eq!(tle, tl + te);
            prop_assert_eq!(tle, ta);
            prop_assert_eq!(ta, bs.total_assets);
            prop_assert_eq!(te, bs.total_equity);
            // INVARIANT 3 and the prose on the earnings lines.
            let eq_lines = section_lines(&st, eq_section);
            let earnings = |label: &str| eq_lines.iter().find(|r| r.label == label).map_or(Decimal::ZERO, |r| amt(r, 0));
            let prior_label = if with_members { "Undistributed earnings, prior years" } else { "Retained earnings" };
            prop_assert_eq!(earnings("Net income (loss)"), ytd);
            prop_assert_eq!(earnings(prior_label), bs.current_earnings - ytd);
            for l in &bs.equity {
                prop_assert!(eq_lines.iter().any(|r| r.label == format!("{} {}", l.code, l.name) && amt(r, 0) == l.amount),
                    "equity account {} {} not listed", l.code, l.name);
            }
        }
    }

    /// cash_flow INVARIANTS 1-5 (direct method, rules.md B-14, D-35): the
    /// operating class rows foot to "Net cash from operating activities",
    /// each other section's kind rows to theirs, and each kind's
    /// transaction details to its kind's row; the three sum to
    /// "Net increase (decrease) in cash"; beginning cash plus it is "Cash
    /// at end of period"; net income and the reconciling items foot to the
    /// schedule's subtotal, which is operating's; every row is the
    /// engine's, kind for kind and label for label (none made up from a
    /// sign); the noncash schedule lists every disclosure, or "None".
    #[test]
    fn cash_flow_foots_and_reconciles((specs, oe, end_off) in books_strategy()) {
        let b = build_books(&specs, oe);
        let end = d(2026, 1, 1) + chrono::Duration::days(end_off);
        let cf = cash_flow(&b.ledger, &CashFlowParams {
            start: d(2026, 1, 1),
            end,
            cash_accounts: vec![b.ids[0], b.ids[1]],
            financing_accounts: vec![b.ids[5]],
            investing_accounts: vec![b.ids[3]],
            members: member_capitals(&b),
        }).unwrap();
        let st = report::cash_flow(&cf);
        let checked = assert_sections_foot(&st);
        prop_assert_eq!(checked.len(), 4, "three sections and the schedule: {:?}", checked);
        let op = find(&st, RowKind::Subtotal, "Net cash from operating activities").expect("operating subtotal");
        let three: Decimal = checked[..3].iter().map(|(_, t)| amt(st.rows.iter().find(|r| &r.label == t).unwrap(), 0)).sum();
        let net = st.rows.iter().find(|r| r.label == "Net increase (decrease) in cash").expect("net change row");
        prop_assert_eq!(amt(net, 0), three);
        let begin = st.rows.iter().find(|r| r.kind == RowKind::Line && r.label.to_lowercase().contains("beginning")).expect("beginning cash");
        let endrow = st.rows.iter().find(|r| r.label == "Cash at end of period").expect("ending cash");
        prop_assert_eq!(amt(begin, 0) + amt(net, 0), amt(endrow, 0));
        // Ties to the core numbers it lays out.
        prop_assert_eq!(amt(op, 0), cf.cash_from_operations);
        prop_assert_eq!(amt(begin, 0), cf.cash_beginning);
        prop_assert_eq!(amt(endrow, 0), cf.cash_ending);
        // INVARIANT 3: the schedule closes on operating's figure.
        let (sched, sched_total) = &checked[3];
        prop_assert!(sched.starts_with("Reconciliation of net income"));
        let sched_row = st.rows.iter().filter(|r| &r.label == sched_total).nth(1).expect("schedule subtotal");
        prop_assert_eq!(amt(sched_row, 0), cf.cash_from_operations);
        // INVARIANT 4: every row is the engine's, in its order.
        let ops: Vec<(String, Decimal)> = section_lines(&st, "Operating").iter().map(|r| (r.label.clone(), amt(r, 0))).collect();
        let want: Vec<(String, Decimal)> = cf.operating_by_class().into_iter().map(|(c, a)| (c.label().to_string(), a)).collect();
        prop_assert_eq!(ops, want);
        // Investing and financing: each kind a line with its total (D-35),
        // its transactions listed under it as details that sum to it.
        for (sec, name) in [(ledger_core::CashFlowSection::Investing, "Investing"), (ledger_core::CashFlowSection::Financing, "Financing")] {
            let mut want = Vec::new();
            let mut kind = None;
            for l in cf.lines(sec) {
                if kind != Some(&l.kind) {
                    let total: Decimal = cf.lines(sec).iter().filter(|x| x.kind == l.kind).map(|x| x.amount).sum();
                    want.push((RowKind::Line, l.kind.clone(), Some(total)));
                    kind = Some(&l.kind);
                }
                want.push((RowKind::Detail, l.label.clone(), Some(l.amount)));
            }
            let mut got = Vec::new();
            let mut inside = false;
            for r in &st.rows {
                match r.kind {
                    RowKind::Section => inside = r.label.starts_with(name),
                    RowKind::Subtotal => inside = false,
                    RowKind::Line | RowKind::Detail | RowKind::Subheading if inside => {
                        got.push((r.kind, r.label.clone(), r.amounts.first().copied().flatten()));
                    }
                    _ => {}
                }
            }
            prop_assert_eq!(got, want);
        }
        prop_assert!(st.rows.iter().all(|r| !r.label.starts_with("Member ")));
        // INVARIANT 5: the noncash schedule.
        let noncash = section_lines(&st, "Noncash");
        if cf.noncash_disclosures.is_empty() {
            prop_assert_eq!(noncash.len(), 1);
            prop_assert_eq!(&noncash[0].label, "None");
        } else {
            let got: Vec<(String, Decimal)> = noncash.iter().map(|r| (r.label.clone(), amt(r, 0))).collect();
            let want: Vec<(String, Decimal)> = cf.noncash_disclosures.iter().map(|n| (n.label.clone(), n.amount)).collect();
            prop_assert_eq!(got, want);
        }
    }

    /// trial_balance INVARIANTS 1-2: each column's lines foot to its total,
    /// the totals are equal (balanced books), and a line shows its balance
    /// on one side only, the other blank.
    #[test]
    fn trial_balance_foots((specs, oe, end_off) in books_strategy()) {
        let b = build_books(&specs, oe);
        let as_of = d(2026, 1, 1) + chrono::Duration::days(end_off);
        let tb = trial_balance_as_of(&b.ledger, as_of);
        let st = report::trial_balance(&tb, as_of);
        prop_assert_eq!(&st.columns, &vec!["Debit".to_string(), "Credit".to_string()]);
        let lines: Vec<&Row> = st.rows.iter().filter(|r| r.kind == RowKind::Line).collect();
        prop_assert_eq!(lines.len(), tb.lines.len());
        for (r, l) in lines.iter().zip(&tb.lines) {
            prop_assert_eq!(r.amounts.len(), 2);
            prop_assert!(r.amounts[0].is_some() != r.amounts[1].is_some(), "one side only: {:?}", r);
            prop_assert_eq!(amt(r, 0), l.debit);
            prop_assert_eq!(amt(r, 1), l.credit);
        }
        let total = st.rows.iter().find(|r| r.kind == RowKind::Total).expect("a total row");
        let dr: Decimal = lines.iter().map(|r| amt(r, 0)).sum();
        let cr: Decimal = lines.iter().map(|r| amt(r, 1)).sum();
        prop_assert_eq!(amt(total, 0), dr);
        prop_assert_eq!(amt(total, 1), cr);
        prop_assert_eq!(dr, cr);
    }

    /// members_equity INVARIANTS 1-3: in every column beginning +
    /// contributions + net income + distributions (printed negative) + other
    /// = ending; members' columns sum to Total on every row; "Other changes"
    /// appears only when some column has any.
    #[test]
    fn members_equity_rolls_forward((specs, oe, end_off) in books_strategy()) {
        let b = build_books(&specs, oe);
        let end = d(2026, 1, 1) + chrono::Duration::days(end_off);
        let me = members_equity(&b.ledger, d(2026, 1, 1), end, &member_capitals(&b)).unwrap();
        let st = report::members_equity(&me);
        prop_assert_eq!(st.columns.len(), 3);
        prop_assert_eq!(&st.columns[0], "Ada");
        prop_assert_eq!(&st.columns[1], "Grace");
        let with_amounts: Vec<&Row> = st.rows.iter().filter(|r| r.amounts.iter().any(Option::is_some)).collect();
        let (ending, flows) = with_amounts.split_last().expect("rows");
        for c in 0..3 {
            let sum: Decimal = flows.iter().map(|r| amt(r, c)).sum();
            prop_assert_eq!(sum, amt(ending, c), "column {} doesn't roll forward", c);
        }
        for r in &with_amounts {
            prop_assert_eq!(amt(r, 0) + amt(r, 1), amt(r, 2), "row {:?} doesn't cross-foot", r.label);
        }
        let cols: Vec<_> = me.members.iter().chain(std::iter::once(&me.total)).collect();
        let dist = st.rows.iter().find(|r| r.label.starts_with("Distributions")).expect("distributions row");
        for (c, col) in cols.iter().enumerate() {
            prop_assert_eq!(amt(flows[0], c), col.beginning);
            prop_assert_eq!(amt(ending, c), col.ending);
            prop_assert_eq!(amt(dist, c), -col.distributions, "distributions print as reductions");
        }
        let any_other = cols.iter().any(|c| !c.other.is_zero());
        prop_assert_eq!(st.rows.iter().any(|r| r.label.starts_with("Other changes")), any_other);
    }
}

/// members_equity INVARIANT 3, the absent case: books with no other equity
/// movement print no "Other changes" row.
#[test]
fn members_equity_without_other_changes_has_no_other_row() {
    let b = build_books(&[], false);
    let mut l = b.ledger;
    let e = l.create_entry(d(2026, 2, 1), "contribution");
    l.add_line(e, b.ids[0], Side::Debit, usd(10_000)).unwrap();
    l.add_line(e, b.ids[6], Side::Credit, usd(10_000)).unwrap();
    l.post_entry(e).unwrap();
    let b = Books {
        ledger: l,
        ids: b.ids,
    };
    let me = members_equity(
        &b.ledger,
        d(2026, 1, 1),
        d(2026, 12, 31),
        &member_capitals(&b),
    )
    .unwrap();
    let st = report::members_equity(&me);
    assert!(!st.rows.iter().any(|r| r.label.starts_with("Other changes")));
}

// ---------------------------------------------------------------------------
// bank_csv
// ---------------------------------------------------------------------------

fn group(int: u64) -> String {
    let s = int.to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// A positive cent amount in one of the spellings bank exports use:
/// plain, thousands commas, a dollar sign.
fn spell_pos(c: i64, style: u8) -> String {
    let (i, f) = (c as u64 / 100, c as u64 % 100);
    match style % 4 {
        0 => format!("{i}.{f:02}"),
        1 => format!("{}.{f:02}", group(i)),
        2 => format!("${}.{f:02}", group(i)),
        _ => format!("${i}.{f:02}"),
    }
}

/// A signed amount for the 3-column layout: negatives as "-" or in
/// parentheses.
fn spell_signed(c: i64, style: u8) -> String {
    if c >= 0 {
        return spell_pos(c, style);
    }
    let p = spell_pos(-c, style);
    match (style / 4) % 3 {
        0 if !p.starts_with('$') => format!("-{p}"),
        _ => format!("({p})"),
    }
}

#[derive(Clone, Debug)]
struct Txn {
    date: NaiveDate,
    desc: String,
    cents: i64,
    style: u8,
}

fn txn() -> impl Strategy<Value = Txn> {
    (
        0i64..3000,
        "[A-Za-z0-9][A-Za-z0-9 ,\"&'#]{0,18}[A-Za-z0-9]",
        prop_oneof![1i64..100_000_000_000, -100_000_000_000i64..0],
        any::<u8>(),
    )
        .prop_map(|(day, desc, cents, style)| Txn {
            date: d(2020, 1, 1) + chrono::Duration::days(day),
            desc,
            cents,
            style,
        })
}

#[derive(Clone, Copy, Debug)]
enum DateStyle {
    Iso,
    MonthFirst,
    DayFirst,
}

fn spell_date(dt: NaiveDate, s: DateStyle) -> String {
    match s {
        DateStyle::Iso => dt.format("%Y-%m-%d").to_string(),
        DateStyle::MonthFirst => format!("{}/{}/{}", dt.month(), dt.day(), dt.year()),
        DateStyle::DayFirst => format!("{}/{}/{}", dt.day(), dt.month(), dt.year()),
    }
}

/// Write `rows` as CSV text (the csv crate quotes as needed).
fn csv_text(rows: &[Vec<String>]) -> String {
    let mut w = csv::WriterBuilder::new().flexible(true).from_writer(vec![]);
    for r in rows {
        w.write_record(r).unwrap();
    }
    String::from_utf8(w.into_inner().unwrap()).unwrap()
}

fn expected(txns: &[Txn]) -> Vec<BankTransaction> {
    txns.iter()
        .map(|t| BankTransaction {
            date: t.date,
            description: t.desc.clone(),
            amount: cents(t.cents),
        })
        .collect()
}

/// Make the file settle its own order: one date with a day over 12.
fn settle(txns: &mut Vec<Txn>) {
    if !txns.iter().any(|t| t.date.day() > 12) {
        txns.push(Txn {
            date: d(2026, 1, 25),
            desc: "Settler".into(),
            cents: 1234,
            style: 0,
        });
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]

    /// bank_csv INVARIANT 1 (3 columns: payment negative, deposit positive),
    /// INVARIANT 2 (zero rows skipped), INVARIANT 3 (a part over 12 settles
    /// the order; a caller order that contradicts it is refused), and the
    /// CONVENTION ($, thousands commas, parentheses; optional header).
    #[test]
    fn bank_csv_three_columns_round_trip(
        mut txns in proptest::collection::vec(txn(), 1..30),
        style in 0u8..3,
        header in any::<bool>(),
        zeros in proptest::collection::vec((0usize..40, 0u8..3), 0..4),
    ) {
        let ds = [DateStyle::Iso, DateStyle::MonthFirst, DateStyle::DayFirst][style as usize];
        if !matches!(ds, DateStyle::Iso) { settle(&mut txns); }
        let mut rows: Vec<Vec<String>> = txns.iter()
            .map(|t| vec![spell_date(t.date, ds), t.desc.clone(), spell_signed(t.cents, t.style)])
            .collect();
        for (at, z) in &zeros {
            let zero = ["0.00", "0", "$0.00"][*z as usize];
            rows.insert(at % (rows.len() + 1), vec![spell_date(d(2026, 1, 28), ds), "zero".into(), zero.into()]);
        }
        if header { rows.insert(0, vec!["Date".into(), "Description".into(), "Amount".into()]); }
        let text = csv_text(&rows);
        let want = expected(&txns);
        prop_assert_eq!(parse_bank_csv(&text).unwrap(), want.clone());
        match ds {
            DateStyle::Iso => {}
            DateStyle::MonthFirst => {
                prop_assert_eq!(parse_bank_csv_with(&text, Some(DateOrder::MonthFirst)).unwrap(), want);
                prop_assert!(parse_bank_csv_with(&text, Some(DateOrder::DayFirst)).is_err());
            }
            DateStyle::DayFirst => {
                prop_assert_eq!(parse_bank_csv_with(&text, Some(DateOrder::DayFirst)).unwrap(), want);
                prop_assert!(parse_bank_csv_with(&text, Some(DateOrder::MonthFirst)).is_err());
            }
        }
    }

    /// bank_csv INVARIANT 1 (4 columns: money out third, money in fourth,
    /// each positive with the other blank, read by position whatever the
    /// header says) and INVARIANT 2 (a zero row, written blank or as 0.00,
    /// is skipped).
    #[test]
    fn bank_csv_four_columns_round_trip(
        txns in proptest::collection::vec(txn(), 1..30),
        header in 0u8..3,
        zeros in proptest::collection::vec((0usize..40, any::<bool>()), 0..4),
    ) {
        let mut rows: Vec<Vec<String>> = txns.iter()
            .map(|t| {
                let a = spell_pos(t.cents.abs(), t.style);
                let (out, inn) = if t.cents < 0 { (a, String::new()) } else { (String::new(), a) };
                vec![spell_date(t.date, DateStyle::Iso), t.desc.clone(), out, inn]
            })
            .collect();
        for (at, blank) in &zeros {
            let z = if *blank { "" } else { "0.00" };
            rows.insert(at % (rows.len() + 1), vec!["2026-01-28".into(), "zero".into(), z.into(), "".into()]);
        }
        match header {
            1 => rows.insert(0, vec!["Date".into(), "Description".into(), "Credit".into(), "Debit".into()]),
            2 => rows.insert(0, vec!["Date".into(), "Description".into(), "Debit".into(), "Credit".into()]),
            _ => {}
        }
        prop_assert_eq!(parse_bank_csv(&csv_text(&rows)).unwrap(), expected(&txns));
    }

    /// bank_csv INVARIANT 3: a file whose slash dates all read both ways
    /// needs the caller's order, and then reads in it.
    #[test]
    fn bank_csv_ambiguous_dates_need_the_callers_order(
        days in proptest::collection::vec((1u32..=12, 1u32..=12), 1..10),
        month_first in any::<bool>(),
    ) {
        let mut days = days;
        days.push((3, 4)); // at least one date that reads both ways
        let order = if month_first { DateOrder::MonthFirst } else { DateOrder::DayFirst };
        let rows: Vec<Vec<String>> = days.iter()
            .map(|(m, dd)| {
                let dt = d(2026, *m, *dd);
                let ds = if month_first { DateStyle::MonthFirst } else { DateStyle::DayFirst };
                vec![spell_date(dt, ds), "x".into(), "1.00".into()]
            })
            .collect();
        let text = csv_text(&rows);
        prop_assert!(matches!(parse_bank_csv(&text), Err(BankCsvError::DateOrderUnknown(_))));
        let got = parse_bank_csv_with(&text, Some(order)).unwrap();
        let want: Vec<NaiveDate> = days.iter().map(|(m, dd)| d(2026, *m, *dd)).collect();
        prop_assert_eq!(got.iter().map(|t| t.date).collect::<Vec<_>>(), want);
    }
}

/// bank_csv SOURCE: Intuit's own 3-column and 4-column example tables,
/// transcribed in intuit-bank-csv.md (payment -100.00, deposit 200.00; in 4
/// columns the payment sits under Credit, the deposit under Debit).
#[test]
fn bank_csv_intuit_examples() {
    let three = "Date,Description,Amount\n1/1/2018,Example of a payment,-100.00\n1/1/2018,Example of a deposit,200.00\n";
    let four = "Date,Description,Credit,Debit\n1/1/2018,Example of a payment,100.00,\n1/1/2018,Example of a deposit,,200.00\n";
    let want = vec![
        BankTransaction {
            date: d(2018, 1, 1),
            description: "Example of a payment".into(),
            amount: dec("-100.00"),
        },
        BankTransaction {
            date: d(2018, 1, 1),
            description: "Example of a deposit".into(),
            amount: dec("200.00"),
        },
    ];
    for order in [DateOrder::DayFirst, DateOrder::MonthFirst] {
        assert_eq!(parse_bank_csv_with(three, Some(order)).unwrap(), want);
        assert_eq!(parse_bank_csv_with(four, Some(order)).unwrap(), want);
    }
}

/// bank_csv INVARIANT 3 scenarios: a part over 12 settles the order for the
/// whole file (so 3/4 follows it); a file settling both ways is refused;
/// ISO dates need no order.
#[test]
fn bank_csv_date_order_scenarios() {
    let day_first = "13/4/2026,a,1.00\n3/4/2026,b,2.00\n";
    let got = parse_bank_csv(day_first).unwrap();
    assert_eq!(got[0].date, d(2026, 4, 13));
    assert_eq!(got[1].date, d(2026, 4, 3));

    let month_first = "4/13/2026,a,1.00\n3/4/2026,b,2.00\n";
    let got = parse_bank_csv(month_first).unwrap();
    assert_eq!(got[0].date, d(2026, 4, 13));
    assert_eq!(got[1].date, d(2026, 3, 4));

    let both = "13/4/2026,a,1.00\n4/13/2026,b,2.00\n";
    assert!(matches!(
        parse_bank_csv(both),
        Err(BankCsvError::DatesDisagree(_, _))
    ));
    assert!(parse_bank_csv_with(both, Some(DateOrder::MonthFirst)).is_err());
    assert!(parse_bank_csv_with(both, Some(DateOrder::DayFirst)).is_err());

    let ambiguous = "3/4/2026,a,1.00\n";
    assert!(matches!(
        parse_bank_csv(ambiguous),
        Err(BankCsvError::DateOrderUnknown(_))
    ));
    assert_eq!(
        parse_bank_csv_with(ambiguous, Some(DateOrder::MonthFirst)).unwrap()[0].date,
        d(2026, 3, 4)
    );
    assert_eq!(
        parse_bank_csv_with(ambiguous, Some(DateOrder::DayFirst)).unwrap()[0].date,
        d(2026, 4, 3)
    );

    let iso = "2026-03-04,a,1.00\n";
    assert_eq!(parse_bank_csv(iso).unwrap()[0].date, d(2026, 3, 4));
}

fn posix_mismatches(years: impl Iterator<Item = i32>) -> Vec<(i32, NaiveDate)> {
    years
        .filter_map(|yy| {
            let text = format!("1/15/{yy:02},a,1.00\n");
            let got = parse_bank_csv(&text).unwrap()[0].date;
            let want = d(if yy >= 69 { 1900 + yy } else { 2000 + yy }, 1, 15);
            (got != want).then_some((yy, got))
        })
        .collect()
}

/// bank_csv INVARIANT 4: a two-digit year follows POSIX (69-99 are 1900s,
/// 00-68 2000s): every year but the 69 boundary.
#[test]
fn bank_csv_two_digit_years_follow_posix() {
    assert_eq!(posix_mismatches((0..69).chain(70..100)), vec![]);
}

/// bank_csv INVARIANT 4 at its boundary: "69" is 1969.
#[test]
fn bank_csv_two_digit_year_69_is_1969() {
    assert_eq!(posix_mismatches(69..70), vec![]);
}

/// bank_csv INVARIANTS 2 and 5: never a fraction of a cent, a double
/// negative ("(-5)", "--5"), or both of a 4-column row's amounts.
#[test]
fn bank_csv_refuses_what_it_must() {
    for bad in [
        "1.005",
        "-0.001",
        "$1,234.567",
        "(-5)",
        "--5",
        "(-5.00)",
        "--5.00",
    ] {
        let text = format!("2026-01-05,x,\"{bad}\"\n");
        assert!(
            matches!(parse_bank_csv(&text), Err(BankCsvError::BadAmount(_))),
            "accepted {bad:?}: {:?}",
            parse_bank_csv(&text)
        );
    }
    let both = "2026-01-05,x,5.00,6.00\n";
    assert!(
        parse_bank_csv(both).is_err(),
        "accepted both 4-column amounts"
    );
    let both_hdr = "Date,Description,Credit,Debit\n2026-01-05,x,5.00,6.00\n";
    assert!(parse_bank_csv(both_hdr).is_err());
}

/// bank_csv INVARIANT 2: a zero row is skipped (3-col zero, 4-col blank
/// cells); the rest keep file order.
#[test]
fn bank_csv_zero_rows_are_skipped() {
    let three = "Date,Description,Amount\n2026-01-01,a,0.00\n2026-01-02,b,-3.50\n2026-01-03,c,0\n";
    let got = parse_bank_csv(three).unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].amount, dec("-3.50"));
    let four = "2026-01-01,a,,\n2026-01-02,b,,3.50\n2026-01-03,c,0.00,\n";
    let got = parse_bank_csv(four).unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].amount, dec("3.50"));
}

/// bank_csv INVARIANT 1: neither 3 nor 4 columns is refused with the count.
#[test]
fn bank_csv_wrong_column_count() {
    assert_eq!(
        parse_bank_csv("2026-01-01,a\n"),
        Err(BankCsvError::BadColumnCount(2))
    );
    assert_eq!(
        parse_bank_csv("2026-01-01,a,1.00,,x\n"),
        Err(BankCsvError::BadColumnCount(5))
    );
}

/// bank_csv prose: the header is the first non-blank row when its first cell
/// says "date"; leading blank lines don't count.
#[test]
fn bank_csv_header_after_blank_lines() {
    let text = "\n\nDate,Description,Amount\n2026-02-03,Coffee,\"($1,234.56)\"\n";
    let got = parse_bank_csv(text).unwrap();
    assert_eq!(
        got,
        vec![BankTransaction {
            date: d(2026, 2, 3),
            description: "Coffee".into(),
            amount: dec("-1234.56"),
        }]
    );
}

// ---------------------------------------------------------------------------
// iif
// ---------------------------------------------------------------------------

fn qb(a: &ledger_core::Account) -> QbAccountType {
    match a.account_type {
        AccountType::Asset if a.code.starts_with("10") => QbAccountType::Bank,
        AccountType::Asset if a.code == "1500" => QbAccountType::FixedAsset,
        AccountType::Asset => QbAccountType::OtherCurrentAsset,
        AccountType::Liability if a.code == "2000" => QbAccountType::CreditCard,
        AccountType::Liability => QbAccountType::OtherCurrentLiability,
        AccountType::Equity => QbAccountType::Equity,
        AccountType::Revenue => QbAccountType::Income,
        AccountType::Expense => QbAccountType::Expense,
    }
}

/// Check an IIF document against iif.rs INVARIANTS 1-6 and the layout,
/// for `entries` written with `accounts`.
fn check_iif(text: &str, accounts: &[IifAccount], entries: &[&JournalEntry]) {
    // INVARIANT 4: ASCII text, every line ending CR LF, and no other CR or
    // LF, tab or control character but the tabs between fields.
    assert!(text.is_ascii(), "non-ASCII in IIF");
    assert!(text.ends_with("\r\n"), "the last line ends CR LF");
    let lines: Vec<&str> = text.strip_suffix("\r\n").unwrap().split("\r\n").collect();
    for l in &lines {
        assert!(
            l.bytes().all(|b| b == b'\t' || (0x20..0x7F).contains(&b)),
            "a control character (a bare CR or LF?) inside a line: {l:?}"
        );
    }
    let keywords = [
        "!ACCNT", "ACCNT", "!TRNS", "!SPL", "!ENDTRNS", "TRNS", "SPL", "ENDTRNS",
    ];
    let mut headers: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut declared: HashSet<String> = HashSet::new();
    let mut seen_trns_header = false;
    let mut txns: Vec<Vec<Vec<&str>>> = Vec::new();
    let mut current: Option<Vec<Vec<&str>>> = None;
    for l in &lines {
        let f: Vec<&str> = l.split('\t').collect();
        let kw = f[0];
        // INVARIANT 4: no line break inside a field means every line starts
        // with a keyword.
        assert!(keywords.contains(&kw), "line starts with {kw:?}: {l:?}");
        if let Some(h) = kw.strip_prefix('!') {
            headers.insert(h, f.clone());
            if kw == "!TRNS" {
                seen_trns_header = true;
            }
            continue;
        }
        // INVARIANT 4: no tab inside a field: as many fields as the header.
        let h = headers
            .get(kw)
            .unwrap_or_else(|| panic!("{kw} before its header"));
        assert_eq!(
            f.len(),
            h.len(),
            "{kw} has {} fields, header {}: {l:?}",
            f.len(),
            h.len()
        );
        match kw {
            "ACCNT" => {
                // INVARIANT 1: every account precedes the transactions,
                // and its parent precedes it.
                assert!(!seen_trns_header, "ACCNT after !TRNS");
                let name = f[h.iter().position(|c| *c == "NAME").unwrap()];
                if let Some((parent, _)) = name.rsplit_once(':') {
                    assert!(
                        declared.contains(parent),
                        "{name:?} before its parent {parent:?}"
                    );
                }
                declared.insert(name.to_string());
            }
            "TRNS" => {
                assert!(current.is_none(), "TRNS inside a transaction");
                current = Some(vec![f]);
            }
            "SPL" => current.as_mut().expect("SPL outside TRNS").push(f),
            "ENDTRNS" => txns.push(current.take().expect("ENDTRNS without TRNS")),
            _ => unreachable!(),
        }
    }
    assert!(current.is_none(), "unterminated transaction");
    assert_eq!(
        declared.len(),
        accounts.len(),
        "every account declared once"
    );
    // INVARIANT 6: no two NAMEs alike, even ignoring case.
    let lower: HashSet<String> = declared.iter().map(|n| n.to_ascii_lowercase()).collect();
    assert_eq!(lower.len(), declared.len(), "two NAMEs alike: {declared:?}");
    let col = |kw: &str, name: &str| -> usize {
        headers[kw]
            .iter()
            .position(|c| *c == name)
            .unwrap_or_else(|| panic!("{kw} lacks {name}"))
    };
    // ACCNT rows carry each account's path, QuickBooks type, and number.
    let accnt_rows: Vec<Vec<&str>> = lines
        .iter()
        .map(|l| l.split('\t').collect::<Vec<_>>())
        .filter(|f| f[0] == "ACCNT")
        .collect();
    for a in accounts {
        let row = accnt_rows
            .iter()
            .find(|f| f[col("ACCNT", "NAME")] == a.path)
            .unwrap_or_else(|| panic!("{} not declared", a.path));
        assert_eq!(row[col("ACCNT", "ACCNTTYPE")], a.qb_type.keyword());
        // INVARIANT 2: a subaccount has its parent's QuickBooks type.
        if let Some((parent, _)) = a.path.rsplit_once(':') {
            let p = accounts.iter().find(|p| p.path == parent).unwrap();
            assert_eq!(p.qb_type, a.qb_type, "{:?} under {parent:?}", a.path);
        }
        // Its number, or none where QuickBooks can't hold it (INVARIANT 6;
        // check_changes says which).
        let accnum = row[col("ACCNT", "ACCNUM")];
        assert!(
            accnum == a.number || accnum.is_empty(),
            "{accnum:?} for {:?}",
            a.number
        );
    }
    let path_of: HashMap<AccountId, &str> =
        accounts.iter().map(|a| (a.id, a.path.as_str())).collect();
    assert_eq!(txns.len(), entries.len(), "one transaction per entry");
    for (t, e) in txns.iter().zip(entries) {
        let mut sum = Decimal::ZERO;
        let mut got: Vec<(String, Decimal)> = Vec::new();
        for (j, f) in t.iter().enumerate() {
            let kw = if j == 0 { "TRNS" } else { "SPL" };
            // INVARIANT 3: GENERAL JOURNAL, dated M/D/YYYY, numbered with
            // the entry's id on every line.
            assert_eq!(f[col(kw, "TRNSTYPE")], "GENERAL JOURNAL");
            let dt = e.date();
            assert_eq!(
                f[col(kw, "DATE")],
                format!("{}/{}/{}", dt.month(), dt.day(), dt.year())
            );
            assert_eq!(
                f[col(kw, "DOCNUM")],
                e.id().to_string(),
                "DOCNUM is the entry id"
            );
            let acct = f[col(kw, "ACCNT")];
            // INVARIANT 1/5: every account used is declared.
            assert!(declared.contains(acct), "{acct:?} used but not declared");
            let a: Decimal = f[col(kw, "AMOUNT")].parse().unwrap();
            assert_eq!(a.scale(), 2, "two decimal places: {}", f[col(kw, "AMOUNT")]);
            sum += a;
            got.push((acct.to_string(), a));
        }
        // INVARIANT 3: debits positive, credits negative, sum zero.
        assert_eq!(sum, Decimal::ZERO, "entry {} doesn't sum to zero", e.id());
        let mut want: Vec<(String, Decimal)> = e
            .lines()
            .iter()
            .map(|l| {
                let a = l.amount.amount;
                let signed = if l.side == Side::Debit { a } else { -a };
                (path_of[&l.account].to_string(), signed)
            })
            .collect();
        got.sort();
        want.sort();
        assert_eq!(got, want, "entry {} lines", e.id());
    }
}

/// Each transaction's MEMO as the file writes it, in file order.
fn iif_memos(text: &str) -> Vec<String> {
    let header: Vec<&str> = text
        .split("\r\n")
        .find(|l| l.starts_with("!TRNS\t"))
        .unwrap()
        .split('\t')
        .collect();
    let at = header.iter().position(|c| *c == "MEMO").unwrap();
    text.split("\r\n")
        .filter(|l| l.starts_with("TRNS\t"))
        .map(|l| l.split('\t').nth(at).unwrap().to_string())
        .collect()
}

/// Each account's ACCNUM as the file writes it, by NAME.
fn iif_numbers(text: &str) -> HashMap<String, String> {
    text.split("\r\n")
        .filter(|l| l.starts_with("ACCNT\t"))
        .map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            (f[1].to_string(), f[3].to_string())
        })
        .collect()
}

/// iif INVARIANTS 6 and 7 against the file: every ACCNUM is at most
/// NUMBER_MAX characters and no other's (ignoring case), and is the
/// account's number unless that is too long or shared, when it is blank;
/// `changes` lists, in file order, each account whose own level or number
/// the file writes differently from the books (a number left out with
/// why), then each entry whose memo it writes differently, with both
/// texts; and nothing else. Its warnings say every change, one line each.
fn check_changes(iif: &Iif, accounts: &[IifAccount], entries: &[&JournalEntry]) {
    let written = iif_numbers(&iif.text);
    let mut seen: HashSet<String> = HashSet::new();
    for n in written.values().filter(|n| !n.is_empty()) {
        assert!(n.len() <= NUMBER_MAX, "ACCNUM {n:?} too long");
        assert!(seen.insert(n.to_ascii_lowercase()), "ACCNUM {n:?} twice");
    }
    let alike = |n: &str| {
        accounts
            .iter()
            .filter(|a| a.number.eq_ignore_ascii_case(n))
            .count()
    };
    let mut want: Vec<IifChange> = Vec::new();
    for a in accounts {
        let own = a.path.rsplit(':').next().unwrap();
        if own != a.name {
            want.push(IifChange {
                field: IifField::AccountName {
                    code: a.code.clone(),
                },
                books: a.name.clone(),
                written: own.to_string(),
            });
        }
        if let Some((parent, parent_type)) = &a.apart_from {
            want.push(IifChange {
                field: IifField::AtTopLevel {
                    code: a.code.clone(),
                    own: a.qb_type,
                    parent: *parent_type,
                },
                books: parent.clone(),
                written: String::new(),
            });
        }
        let left_out = if a.number.len() > NUMBER_MAX {
            Some(LeftOut::TooLong)
        } else if !a.number.is_empty() && alike(&a.number) > 1 {
            Some(LeftOut::Shared(a.number.clone()))
        } else {
            None
        };
        let accnum = &written[&a.path];
        match left_out {
            Some(why) => {
                assert_eq!(accnum, "", "{:?} left out", a.number);
                want.push(IifChange {
                    field: IifField::AccountNumberLeftOut { why },
                    books: a.code.clone(),
                    written: String::new(),
                });
            }
            None => {
                assert_eq!(accnum, &a.number);
                if a.number != a.code {
                    want.push(IifChange {
                        field: IifField::AccountNumber,
                        books: a.code.clone(),
                        written: a.number.clone(),
                    });
                }
            }
        }
    }
    for (e, memo) in entries.iter().zip(iif_memos(&iif.text)) {
        assert!(memo.len() <= MEMO_MAX, "entry {}'s memo too long", e.id());
        if memo != e.description() {
            want.push(IifChange {
                field: IifField::Memo { entry: e.id() },
                books: e.description().to_string(),
                written: memo,
            });
        }
    }
    assert_eq!(iif.changes, want);
    check_warnings(iif);
}

/// A string Rust's `{:?}` printed at the start of `s`, read back, and
/// what follows it.
fn read_quoted(s: &str) -> Option<(String, &str)> {
    let mut chars = s.strip_prefix('"')?.char_indices();
    let mut out = String::new();
    while let Some((i, c)) = chars.next() {
        match c {
            '"' => return Some((out, &s[i + 2..])),
            '\\' => match chars.next()?.1 {
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                '0' => out.push('\0'),
                'u' => {
                    let mut hex = String::new();
                    chars.next()?; // '{'
                    for (_, h) in chars.by_ref() {
                        if h == '}' {
                            break;
                        }
                        hex.push(h);
                    }
                    out.push(char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?);
                }
                other => out.push(other),
            },
            c => out.push(c),
        }
    }
    None
}

/// iif INVARIANT 7's warnings: one line each, never a full TIN; each
/// account name or number change its own line, as the change prints; each
/// memo change named in one line at least, and every memo line true of
/// each entry it names: `"from" is written "to" in the memos of entries
/// ...` with `from` in the books' memo and `to` in the file's, or the line
/// of memos cut to MEMO_MAX naming exactly those cut. A line with a masked
/// TIN-like run is taken as it is.
fn check_warnings(iif: &Iif) {
    let lines = iif.warnings();
    let memos: HashMap<u64, &IifChange> = iif
        .changes
        .iter()
        .filter_map(|c| match c.field {
            IifField::Memo { entry } => Some((entry, c)),
            _ => None,
        })
        .collect();
    let mut named: HashSet<u64> = HashSet::new();
    for l in &lines {
        assert!(!l.chars().any(char::is_control), "one line: {l:?}");
        assert!(!ledger_core::parties::has_full_tin(l), "a TIN in {l:?}");
        let ids = l
            .rsplit_once(" the memo of entry ")
            .or_else(|| l.rsplit_once(" the memos of entries "))
            .or_else(|| l.split_once("the memo of entry "))
            .or_else(|| l.split_once("the memos of entries "));
        let Some((_, ids)) = ids else {
            continue;
        };
        let list: String = ids
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == ',' || *c == ' ')
            .collect();
        let ids: Vec<u64> = list
            .trim_end()
            .split(", ")
            .map(|t| t.parse().unwrap())
            .collect();
        assert!(
            !ids.is_empty() && ids.windows(2).all(|w| w[0] < w[1]),
            "{l:?}"
        );
        named.extend(&ids);
        if l.contains('\u{2022}') {
            continue;
        }
        if l.starts_with("the memo") {
            let cut: Vec<u64> = memos
                .iter()
                .filter(|(_, c)| c.written.len() == MEMO_MAX)
                .map(|(e, _)| *e)
                .collect();
            for e in &ids {
                assert!(cut.contains(e), "entry {e} isn't cut: {l:?}");
            }
            continue;
        }
        let (from, rest) = read_quoted(l).unwrap_or_else(|| panic!("{l:?}"));
        let rest = rest.strip_prefix(" is written ").unwrap();
        let (to, rest) = read_quoted(rest).unwrap();
        assert!(rest.starts_with(" in the memo"), "{l:?}");
        assert!(!from.is_empty() && from != to, "{l:?}");
        for e in &ids {
            let c = memos[e];
            assert!(
                c.books.contains(&from),
                "{from:?} not in entry {e}'s {:?}",
                c.books
            );
            assert!(
                c.written.contains(&to),
                "{to:?} not in entry {e}'s {:?}",
                c.written
            );
        }
    }
    for c in &iif.changes {
        match c.field {
            IifField::Memo { entry } => {
                assert!(
                    named.contains(&entry),
                    "entry {entry}'s memo isn't in {lines:?}"
                )
            }
            _ => assert!(lines.contains(&c.to_string()), "{c} isn't in {lines:?}"),
        }
    }
}

/// Whether `s` is text INVARIANT 4 writes as itself: printable ASCII
/// without a double quote.
fn plain(s: &str) -> bool {
    s.bytes().all(|b| (0x20..0x7F).contains(&b) && b != b'"')
}

/// The colon path an account should have: its own name and its parents',
/// highest first (each with any colon replaced, so the depth is exact).
fn depth(ledger: &Ledger, mut id: AccountId) -> usize {
    let mut n = 0;
    while let Some(p) = ledger.chart().get(id).unwrap().parent {
        n += 1;
        id = p;
    }
    n
}

/// Text for names and memos, mixing what INVARIANT 4 keeps (ASCII), what
/// it transliterates (Latin-1 and other letters, curly quotes, dashes, a
/// no-break space, a fullwidth colon, Chinese, Japanese, Korean, emoji,
/// combining marks, invisible format characters), what it writes '?'
/// (controls, private use) and what it writes as a space (tabs and every
/// kind of line break).
fn any_text() -> impl Strategy<Value = String> {
    "[A-Za-z0-9 :()#$\t\n\r\u{B}\u{C}\u{85}\u{2028}\u{2029}é—–“”’\u{A0}Ł北京서울あ😀☕\u{301}\u{200B}\u{FEFF}\u{1}\u{7F}\u{9F}\u{E000}\u{FF1A}]{0,12}"
}

/// A random chart for the name properties: account i under an earlier
/// one or at the top, its name from a pool that makes clashes (the same
/// name twice, differing only by case, a fullwidth colon against a
/// hyphen, two private-use names that are both '?', Chinese against its
/// transliteration, a name that is another's " (code)" form, names past
/// NAME_MAX that are alike once cut, and one that is another's cut form)
/// or from [`any_text`].
fn chart_strategy() -> impl Strategy<Value = Vec<(String, Option<usize>)>> {
    let name = prop_oneof![
        proptest::sample::select(vec![
            "Software",
            "software",
            "Ratio\u{FF1A}Ops",
            "Ratio-Ops",
            "\u{E000}",
            "\u{E001}",
            "北京",
            "Bei Jing",
            "Software (6001)",
            " Rent ",
            "Professional services and consulting fees",
            "Professional services and consulting costs",
            "Professional services and consu",
            "Professional services an (6002)",
        ])
        .prop_map(String::from),
        any_text(),
    ];
    proptest::collection::vec((name, any::<u64>(), any::<bool>()), 1..12).prop_map(|v| {
        v.into_iter()
            .enumerate()
            .map(|(i, (n, pick, top))| {
                let parent = (i > 0 && !top).then(|| (pick % i as u64) as usize);
                (n, parent)
            })
            .collect()
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// iif INVARIANTS 1-7 over random books with memos of every kind:
    /// accounts declared before the transactions, colon paths; TRNS/SPL/
    /// ENDTRNS, GENERAL JOURNAL, M/D/YYYY, DOCNUM the entry id, debits
    /// positive and credits negative summing to zero; ASCII lines ending CR
    /// LF, no tab or line break inside a field; plain ASCII written as
    /// itself; every change listed.
    #[test]
    fn iif_round_trips_random_books(
        (specs, oe, _) in books_strategy(),
        memos in proptest::collection::vec(any_text(), 25),
    ) {
        let mut specs = specs;
        for (s, m) in specs.iter_mut().zip(memos) {
            s.memo = m;
        }
        let b = build_books(&specs, oe);
        let accounts = iif_accounts(&b.ledger, qb);
        prop_assert_eq!(accounts.len(), b.ledger.chart().len());
        let entries: Vec<&JournalEntry> = b.ledger.posted_entries().collect();
        let iif = journal_to_iif(&accounts, &entries).unwrap();
        check_iif(&iif.text, &accounts, &entries);
        check_changes(&iif, &accounts, &entries);
        // INVARIANT 4: a memo of printable ASCII is written as itself.
        for (e, memo) in entries.iter().zip(iif_memos(&iif.text)) {
            if plain(e.description()) {
                prop_assert_eq!(memo.as_str(), e.description());
            }
        }
        // INVARIANT 2: a subaccount's NAME is its path, parents first, one
        // colon per level, the parent's path as its prefix.
        for a in &accounts {
            let acct = b.ledger.chart().get(a.id).unwrap();
            prop_assert_eq!(&a.number, &acct.code);
            prop_assert_eq!(a.qb_type, qb(acct));
            prop_assert_eq!(a.path.matches(':').count(), depth(&b.ledger, a.id), "{:?}", a.path);
            if let Some(p) = acct.parent {
                let pp = &accounts.iter().find(|x| x.id == p).unwrap().path;
                prop_assert!(a.path.starts_with(&format!("{pp}:")), "{:?} under {:?}", a.path, pp);
            }
        }
    }

    /// iif INVARIANTS 2, 6 and 7 over random charts full of clashing
    /// names: every NAME differs from every other ignoring case, has one
    /// colon per level under its parent's NAME, and each level is the
    /// account's own name (or that name followed by " (ACCNUM)" where it
    /// would clash), trimmed, never empty; a clash that remains is refused,
    /// naming accounts whose NAMEs are alike; every change is listed.
    #[test]
    fn iif_names_never_clash_and_every_change_is_listed(chart in chart_strategy()) {
        let mut l = Ledger::new(Currency::USD);
        let mut ids = Vec::new();
        // Codes: most four digits; some with a colon (its tag can't add a
        // level), some past NUMBER_MAX (left out of ACCNUM), and one with a
        // space before account 2's (the same number once trimmed: both
        // left out, and tagged alike).
        let code_of = |i: usize| match i {
            8 => " 6002".to_string(),
            i if i % 5 == 3 => format!("{}:{i}", 60 + i),
            i if i % 7 == 6 => format!("1234567{i}"),
            i => format!("{}", 6000 + i),
        };
        for (i, (name, parent)) in chart.iter().enumerate() {
            // A blank name is refused by the books; it can't reach the file.
            let name = if name.trim().is_empty() { "x" } else { name.as_str() };
            let parent = parent.map(|p| ids[p]);
            // Replayed, as books written before P3 hold them: a sibling's
            // name, a tab, nine digits (rules.md B-31 refuses them in a
            // new name, never in a journaled one).
            ids.push(l.replay_add_account(&code_of(i), name, AccountType::Expense, parent, false).unwrap());
        }
        let accounts = iif_accounts(&l, |_| QbAccountType::Expense);
        match journal_to_iif(&accounts, &[]) {
            Ok(iif) => {
                check_iif(&iif.text, &accounts, &[]);
                check_changes(&iif, &accounts, &[]);
                for a in &accounts {
                    let acct = l.chart().get(a.id).unwrap();
                    prop_assert_eq!(a.path.matches(':').count(), depth(&l, a.id), "{:?}", a.path);
                    if let Some(p) = acct.parent {
                        let pp = &accounts.iter().find(|x| x.id == p).unwrap().path;
                        prop_assert!(a.path.starts_with(&format!("{pp}:")), "{:?} under {:?}", a.path, pp);
                    }
                    let own = a.path.rsplit(':').next().unwrap();
                    prop_assert!(!own.is_empty() && own == own.trim(), "{:?}", own);
                    prop_assert!(own.len() <= NAME_MAX, "{:?}", own);
                    // A plain name written as itself, cut to NAME_MAX, or
                    // so cut to make room for its tag, unless it clashed.
                    let name = acct.name.as_str();
                    let tag = format!(" ({})", acct.code.trim().replace(':', "-"));
                    if plain(name) && !name.contains(':') && name == name.trim() {
                        let base = own.strip_suffix(tag.as_str()).unwrap_or(own);
                        prop_assert!(
                            own == name.get(..NAME_MAX).unwrap_or(name).trim_end()
                                || (own.ends_with(tag.as_str()) && name.starts_with(base)),
                            "{:?} for {:?}", own, name
                        );
                    }
                }
            }
            Err(IifError::NameCollision { name, codes }) => {
                // Only account 8, whose code is account 2's once trimmed,
                // can be tagged like another.
                let alike: Vec<&str> = accounts
                    .iter()
                    .filter(|a| field_eq(&a.path, &name))
                    .map(|a| a.code.as_str())
                    .collect();
                prop_assert_eq!(&alike, &codes);
                prop_assert_eq!(codes.iter().map(|c| c.trim()).collect::<HashSet<_>>().len(), 1);
            }
            Err(e) => prop_assert!(false, "{}", e),
        }
    }
}

/// Whether two NAMEs are alike to QuickBooks (ignoring ASCII case).
fn field_eq(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// iif INVARIANT 2: "Reserve: Savings" under "Cash" is one level down, its
/// own colon replaced; INVARIANT 4: what isn't ASCII is transliterated
/// (curly quotes straight and single, an em dash two hyphens, "é" "e"),
/// and each such name is listed (INVARIANT 7).
#[test]
fn iif_paths_and_transliteration() {
    let b = build_books(&[], true);
    let accounts = iif_accounts(&b.ledger, qb);
    let path = |code: &str| {
        let id = b.ledger.chart().find_by_code(code).unwrap().id;
        accounts.iter().find(|a| a.id == id).unwrap().path.clone()
    };
    assert_eq!(path("1010"), "Cash:Reserve- Savings");
    for a in &accounts {
        assert!(plain(&a.path), "{:?}", a.path);
    }
    // Curly quotes straight, and a double quote single (QuickBooks reads
    // it as a text delimiter).
    assert_eq!(path("4100"), "Interest 'income'");
    assert_eq!(path("3000"), "Capital -- Ada");
    assert_eq!(path("6010"), "Cafe meals");
    let iif = journal_to_iif(&accounts, &[]).unwrap();
    let changed: Vec<String> = iif.changes.iter().map(ToString::to_string).collect();
    assert_eq!(
        changed,
        [
            "account 1010's name \"Reserve: Savings\" is written \"Reserve- Savings\"",
            "account 3000's name \"Capital — Ada\" is written \"Capital -- Ada\"",
            "account 3001's name \"Capital — Grace\" is written \"Capital -- Grace\"",
            "account 3100's name \"Distributions — Ada\" is written \"Distributions -- Ada\"",
            "account 3101's name \"Distributions — Grace\" is written \"Distributions -- Grace\"",
            "account 4100's name \"Interest “income”\" is written \"Interest 'income'\"",
            "account 6010's name \"Café meals\" is written \"Cafe meals\"",
        ]
    );
}

/// iif layout example (module docs): a two-line sale on 1/15/2026 writes
/// TRNS Cash 1000.00 then SPL Revenue -1000.00, DOCNUM 1 (it is entry 1),
/// memo "Sale", each line's tabs matching its header.
#[test]
fn iif_module_layout_example() {
    let mut l = Ledger::new(Currency::USD);
    let cash = l
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    let rev = l
        .add_account("4000", "Revenue", AccountType::Revenue, None, false)
        .unwrap();
    let e = l.create_entry(d(2026, 1, 15), "Sale");
    assert_eq!(e, 1);
    l.add_line(e, cash, Side::Debit, usd(100_000)).unwrap();
    l.add_line(e, rev, Side::Credit, usd(100_000)).unwrap();
    l.post_entry(e).unwrap();
    let accounts = iif_accounts(&l, |a| {
        if a.code == "1000" {
            QbAccountType::Bank
        } else {
            QbAccountType::Income
        }
    });
    let entries: Vec<&JournalEntry> = l.posted_entries().collect();
    let iif = journal_to_iif(&accounts, &entries).unwrap();
    let lines: Vec<&str> = iif.text.lines().map(|l| l.trim_end_matches('\r')).collect();
    assert_eq!(lines[0], "!ACCNT\tNAME\tACCNTTYPE\tACCNUM");
    assert!(lines.contains(&"ACCNT\tCash\tBANK\t1000"));
    assert!(lines.contains(&"ACCNT\tRevenue\tINC\t4000"));
    assert!(lines.contains(&"TRNS\t\tGENERAL JOURNAL\t1/15/2026\tCash\t\t1000.00\t1\tSale"));
    assert!(lines.contains(&"SPL\t\tGENERAL JOURNAL\t1/15/2026\tRevenue\t\t-1000.00\t1\tSale"));
    check_iif(&iif.text, &accounts, &entries);
    assert!(iif.changes.is_empty());
}

/// iif INVARIANT 4: a memo holding tabs, line breaks, and non-ASCII still
/// writes one ASCII line per record with the header's field count.
#[test]
fn iif_memo_with_tabs_and_breaks() {
    let mut l = Ledger::new(Currency::USD);
    let cash = l
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    let rev = l
        .add_account("4000", "Revenue", AccountType::Revenue, None, false)
        .unwrap();
    let e = l.create_entry(d(2026, 3, 9), "a\tb\nc\r\nd — “é” ’");
    l.add_line(e, cash, Side::Debit, usd(1)).unwrap();
    l.add_line(e, rev, Side::Credit, usd(1)).unwrap();
    l.post_entry(e).unwrap();
    let accounts = iif_accounts(&l, |_| QbAccountType::Bank);
    let entries: Vec<&JournalEntry> = l.posted_entries().collect();
    let iif = journal_to_iif(&accounts, &entries).unwrap();
    check_iif(&iif.text, &accounts, &entries);
    check_changes(&iif, &accounts, &entries);
}

/// iif INVARIANT 5: an entry whose account wasn't declared is refused,
/// naming the account.
#[test]
fn iif_refuses_an_undeclared_account() {
    let spec = EntrySpec {
        day: 200,
        accts: vec![0, 11],
        k: 1,
        debits: vec![500, 0, 0],
        seeds: vec![0, 0, 0],
        memo: "x".into(),
    };
    let b = build_books(&[spec], false);
    let mut accounts = iif_accounts(&b.ledger, qb);
    let sales = b.ids[11];
    accounts.retain(|a| a.id != sales);
    let entries: Vec<&JournalEntry> = b.ledger.posted_entries().collect();
    let eid: EntryId = entries[0].id();
    assert_eq!(
        journal_to_iif(&accounts, &entries),
        Err(IifError::UndeclaredAccount {
            entry: eid,
            account: sales
        })
    );
}

/// A ledger with Cash 1000, Revenue 4000 and Fees 6000.
fn small_ledger() -> (Ledger, AccountId, AccountId, AccountId) {
    let mut l = Ledger::new(Currency::USD);
    let cash = l
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    let rev = l
        .add_account("4000", "Revenue", AccountType::Revenue, None, false)
        .unwrap();
    let fees = l
        .add_account("6000", "Fees", AccountType::Expense, None, false)
        .unwrap();
    (l, cash, rev, fees)
}

/// Defect 7 (P3; DOCNUM was the transaction's place in the file, so a
/// discarded draft, a post out of order or the as_of filter broke the
/// trace from QuickBooks to the books). iif INVARIANT 3: DOCNUM is the
/// entry's id, on the TRNS and every SPL. Entry 1 is a draft never
/// posted; 2, 3 and 4 are posted in the order 3, 2, 4, and dated so that
/// date order (4, 3, 2), posting order and id order all differ. The file
/// had numbered them 1, 2, 3.
#[test]
fn iif_docnum_is_the_entry_id() {
    let (mut l, cash, rev, fees) = small_ledger();
    let draft = l.create_entry(d(2026, 1, 10), "Never posted");
    l.add_line(draft, fees, Side::Debit, usd(500)).unwrap();
    l.add_line(draft, cash, Side::Credit, usd(500)).unwrap();
    let b = l.create_entry(d(2026, 3, 1), "March sale");
    l.add_line(b, cash, Side::Debit, usd(20_000)).unwrap();
    l.add_line(b, rev, Side::Credit, usd(20_000)).unwrap();
    let c = l.create_entry(d(2026, 2, 1), "February sale, less a fee");
    l.add_line(c, cash, Side::Debit, usd(9_700)).unwrap();
    l.add_line(c, fees, Side::Debit, usd(300)).unwrap();
    l.add_line(c, rev, Side::Credit, usd(10_000)).unwrap();
    let e4 = l.create_entry(d(2026, 1, 20), "January fee");
    l.add_line(e4, fees, Side::Debit, usd(1_000)).unwrap();
    l.add_line(e4, cash, Side::Credit, usd(1_000)).unwrap();
    assert_eq!((draft, b, c, e4), (1, 2, 3, 4));
    for e in [c, b, e4] {
        l.post_entry(e).unwrap();
    }
    let accounts = iif_accounts(&l, |_| QbAccountType::Bank);
    let entries: Vec<&JournalEntry> = l.posted_entries().collect();
    assert_eq!(
        entries.iter().map(|e| e.id()).collect::<Vec<_>>(),
        [3, 2, 4]
    );
    let iif = journal_to_iif(&accounts, &entries).unwrap();
    check_iif(&iif.text, &accounts, &entries);
    // Each record's keyword and DOCNUM, in file order.
    let docnums: Vec<(&str, &str)> = iif
        .text
        .split("\r\n")
        .filter(|l| l.starts_with("TRNS\t") || l.starts_with("SPL\t"))
        .map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            (f[0], f[7])
        })
        .collect();
    assert_eq!(
        docnums,
        [
            ("TRNS", "3"),
            ("SPL", "3"),
            ("SPL", "3"),
            ("TRNS", "2"),
            ("SPL", "2"),
            ("TRNS", "4"),
            ("SPL", "4"),
        ]
    );
}

/// Plan D-36 (P3, defect 7): the file is plain ASCII, every name and memo
/// past ASCII transliterated and listed. Accented letters, Chinese,
/// emoji, a fullwidth colon and fullwidth digits, a tab inside a name,
/// controls, a decomposed accent and a private-use character, each with
/// its exact written form (deunicode 1.6.0's table); a memo of plain ASCII
/// is neither changed nor listed. Before, every one of these characters
/// was '?' (and "—" a single '-'), and nothing said so.
#[test]
fn iif_writes_plain_ascii_and_lists_every_change() {
    let (mut l, cash, rev, _) = small_ledger();
    for (code, name) in [
        ("6010", "Café meals"),
        ("6020", "北京 Office"),
        ("6030", "Ratio\u{FF1A}Ops"),
        ("6040", "Desk\trent"),
        ("6050", "René 😀"),
        ("６０７０", "Wide number"),
    ] {
        // Replayed: a tab in a name is refused in a new one (rules.md
        // B-31), never in one books written before P3 hold.
        l.replay_add_account(code, name, AccountType::Expense, None, false)
            .unwrap();
    }
    let memos = [
        "Lunch at Café “Zoë” — 北京\t☕\n",
        "Control\u{1}char\u{7F}",
        "Plain ASCII memo",
        "Cafe\u{301}",
        "\u{E000}x",
    ];
    for m in memos {
        let e = l.create_entry(d(2026, 4, 1), m);
        l.add_line(e, cash, Side::Debit, usd(100)).unwrap();
        l.add_line(e, rev, Side::Credit, usd(100)).unwrap();
        l.post_entry(e).unwrap();
    }
    let accounts = iif_accounts(&l, |_| QbAccountType::Expense);
    let entries: Vec<&JournalEntry> = l.posted_entries().collect();
    let iif = journal_to_iif(&accounts, &entries).unwrap();
    check_iif(&iif.text, &accounts, &entries);
    check_changes(&iif, &accounts, &entries);
    let accnt: Vec<&str> = iif
        .text
        .split("\r\n")
        .filter(|l| l.starts_with("ACCNT\t"))
        .collect();
    assert_eq!(
        accnt,
        [
            "ACCNT\tCash\tEXP\t1000",
            "ACCNT\tRevenue\tEXP\t4000",
            "ACCNT\tFees\tEXP\t6000",
            "ACCNT\tCafe meals\tEXP\t6010",
            "ACCNT\tBei Jing Office\tEXP\t6020",
            "ACCNT\tRatio-Ops\tEXP\t6030",
            "ACCNT\tDesk rent\tEXP\t6040",
            "ACCNT\tRene grinning\tEXP\t6050",
            "ACCNT\tWide number\tEXP\t6070",
        ]
    );
    assert_eq!(
        iif_memos(&iif.text),
        [
            "Lunch at Cafe 'Zoe' -- Bei Jing coffee ",
            "Control?char?",
            "Plain ASCII memo",
            "Cafe",
            "?x",
        ]
    );
    let changed: Vec<String> = iif.changes.iter().map(ToString::to_string).collect();
    assert_eq!(
        changed,
        [
            "account 6010's name \"Café meals\" is written \"Cafe meals\"",
            "account 6020's name \"北京 Office\" is written \"Bei Jing Office\"",
            "account 6030's name \"Ratio：Ops\" is written \"Ratio-Ops\"",
            "account 6040's name \"Desk\\trent\" is written \"Desk rent\"",
            "account 6050's name \"René 😀\" is written \"Rene grinning\"",
            "account number \"６０７０\" is written \"6070\"",
            "entry 1's memo \"Lunch at Café “Zoë” — 北京\\t☕\\n\" is written \"Lunch at Cafe 'Zoe' -- Bei Jing coffee \"",
            "entry 2's memo \"Control\\u{1}char\\u{7f}\" is written \"Control?char?\"",
            "entry 4's memo \"Cafe\\u{301}\" is written \"Cafe\"",
            "entry 5's memo \"\\u{e000}x\" is written \"?x\"",
        ]
    );
}

/// Plan D-36: every line of the file ends CR LF, as in Intuit's own
/// example file (sources/intuit-iif.md), and no CR or LF stands alone.
#[test]
fn iif_every_line_ends_cr_lf() {
    let (mut l, cash, rev, _) = small_ledger();
    let e = l.create_entry(d(2026, 5, 1), "Sale\r\nsplit\rover\nlines");
    l.add_line(e, cash, Side::Debit, usd(100)).unwrap();
    l.add_line(e, rev, Side::Credit, usd(100)).unwrap();
    l.post_entry(e).unwrap();
    let accounts = iif_accounts(&l, |_| QbAccountType::Bank);
    let entries: Vec<&JournalEntry> = l.posted_entries().collect();
    let text = journal_to_iif(&accounts, &entries).unwrap().text;
    let crlf = text.matches("\r\n").count();
    assert_eq!(crlf, text.matches('\n').count(), "every LF follows a CR");
    assert_eq!(
        crlf,
        text.matches('\r').count(),
        "every CR comes before an LF"
    );
    // Three headers and three accounts, three more headers, TRNS, SPL,
    // ENDTRNS.
    assert_eq!(crlf, 1 + 3 + 3 + 3);
    assert!(text.ends_with("ENDTRNS\r\n"));
    assert_eq!(iif_memos(&text), ["Sale  split over lines"]);
}

/// iif INVARIANT 6 (P3 review R2): QuickBooks knows an account by its
/// NAME, so two that would share one would be merged. Two top-level
/// "Software", "software" by case, two "Licenses" under one parent, a
/// fullwidth colon against a hyphen, two private-use names (both '?'):
/// each written with " (ACCNUM)", the subaccounts of a renamed parent
/// following it, and listed. A name that is another's tagged form is
/// tagged in turn; a match left when the codes themselves are written
/// alike is refused, naming both.
#[test]
fn iif_distinct_accounts_never_share_a_name() {
    let mut l = Ledger::new(Currency::USD);
    // Replayed, as books written before P3 hold them: siblings of one
    // name (rules.md B-31 refuses one in a new name, never in a journaled
    // one), which the file must still keep apart.
    let mut add = |code: &str, name: &str, parent: Option<&str>| {
        let p = parent.map(|c| l.chart().find_by_code(c).unwrap().id);
        l.replay_add_account(code, name, AccountType::Expense, p, false)
            .unwrap();
    };
    add("6100", "Software", None);
    add("6110", "Licenses", Some("6100"));
    add("6200", "Software", None);
    add("6210", "Licenses", Some("6200"));
    add("6300", "software", None);
    add("6400", "Subscriptions", None);
    add("6410", "Licenses", Some("6400"));
    add("6420", "Licenses", Some("6400"));
    add("6500", "Ratio\u{FF1A}Ops", None);
    add("6600", "Ratio-Ops", None);
    add("6700", "\u{E000}", None);
    add("6800", "\u{E001}", None);
    let accounts = iif_accounts(&l, |_| QbAccountType::Expense);
    let paths: Vec<(&str, &str)> = accounts
        .iter()
        .map(|a| (a.number.as_str(), a.path.as_str()))
        .collect();
    assert_eq!(
        paths,
        [
            ("6100", "Software (6100)"),
            ("6110", "Software (6100):Licenses"),
            ("6200", "Software (6200)"),
            ("6210", "Software (6200):Licenses"),
            ("6300", "software (6300)"),
            ("6400", "Subscriptions"),
            ("6410", "Subscriptions:Licenses (6410)"),
            ("6420", "Subscriptions:Licenses (6420)"),
            ("6500", "Ratio-Ops (6500)"),
            ("6600", "Ratio-Ops (6600)"),
            ("6700", "? (6700)"),
            ("6800", "? (6800)"),
        ]
    );
    let iif = journal_to_iif(&accounts, &[]).unwrap();
    check_iif(&iif.text, &accounts, &[]);
    check_changes(&iif, &accounts, &[]);
    assert_eq!(iif.changes.len(), 9, "{:?}", iif.changes);
    assert_eq!(
        iif.changes[0].to_string(),
        "account 6100's name \"Software\" is written \"Software (6100)\""
    );
    // A third account already named like the first's tagged form: tagged
    // in its turn, so no two match.
    l.add_account("6900", "Software (6100)", AccountType::Expense, None, false)
        .unwrap();
    let accounts = iif_accounts(&l, |_| QbAccountType::Expense);
    let path = |code: &str| {
        accounts
            .iter()
            .find(|a| a.code == code)
            .map(|a| a.path.clone())
            .unwrap()
    };
    assert_eq!(path("6100"), "Software (6100)");
    assert_eq!(path("6900"), "Software (6100) (6900)");
    let iif = journal_to_iif(&accounts, &[]).unwrap();
    check_iif(&iif.text, &accounts, &[]);
    check_changes(&iif, &accounts, &[]);
    // Two "Software" whose codes are written alike (a fullwidth digit):
    // tagged alike, so the match remains, and is refused naming both.
    let mut l = Ledger::new(Currency::USD);
    for code in ["6100", "6\u{FF11}00"] {
        l.replay_add_account(code, "Software", AccountType::Expense, None, false)
            .unwrap();
    }
    let accounts = iif_accounts(&l, |_| QbAccountType::Expense);
    let err = journal_to_iif(&accounts, &[]).unwrap_err();
    assert_eq!(
        err,
        IifError::NameCollision {
            name: "Software (6100)".into(),
            codes: vec!["6100".into(), "6\u{FF11}00".into()],
        }
    );
    assert_eq!(
        err.to_string(),
        "accounts 6100, 6\u{FF11}00 would have the same name in QuickBooks, \"Software (6100)\": rename one"
    );
}

/// iif INVARIANT 2 (P3 review round 2): QuickBooks gives a subaccount
/// its parent's type ("The parent and subaccount must be the same account
/// type", Intuit, sources/intuit-iif.md), so a bank account under a
/// "Bank accounts" header that is no bank, or an asset's cost account
/// under a "Fixed assets" header, would import as the wrong type or not at
/// all. Each is written at the top level, keeping its own type, its own
/// subaccounts with it, and listed; one that then matches a top-level
/// account's name is told apart by its code; a subaccount of its parent's
/// type stays under it. Before, every subaccount was written under its
/// parent whatever their types.
#[test]
fn iif_a_subaccount_of_another_type_stands_at_the_top_level() {
    let mut l = Ledger::new(Currency::USD);
    let mut add = |code: &str, name: &str, parent: Option<&str>| {
        let p = parent.map(|c| l.chart().find_by_code(c).unwrap().id);
        l.add_account(code, name, AccountType::Asset, p, false)
            .unwrap()
    };
    add("1000", "Bank accounts", None);
    let checking = add("1010", "Checking", Some("1000"));
    let sweep = add("1011", "Sweep", Some("1010"));
    add("1020", "Petty cash", Some("1000"));
    add("1500", "Fixed assets", None);
    let laptop = add("1510", "Laptop", Some("1500"));
    add("1600", "Laptop", None);
    let e = l.create_entry(d(2026, 3, 1), "Laptop bought");
    l.add_line(e, laptop, Side::Debit, usd(150_000)).unwrap();
    l.add_line(e, sweep, Side::Credit, usd(100_000)).unwrap();
    l.add_line(e, checking, Side::Credit, usd(50_000)).unwrap();
    l.post_entry(e).unwrap();
    let accounts = iif_accounts(&l, |a| match a.code.as_str() {
        "1010" | "1011" => QbAccountType::Bank,
        "1510" => QbAccountType::FixedAsset,
        _ => QbAccountType::OtherCurrentAsset,
    });
    let entries: Vec<&JournalEntry> = l.posted_entries().collect();
    let iif = journal_to_iif(&accounts, &entries).unwrap();
    check_iif(&iif.text, &accounts, &entries);
    check_changes(&iif, &accounts, &entries);
    let lines: Vec<&str> = iif
        .text
        .split("\r\n")
        .filter(|l| l.starts_with("ACCNT\t") || l.starts_with("TRNS\t") || l.starts_with("SPL\t"))
        .collect();
    assert_eq!(
        lines,
        [
            "ACCNT\tBank accounts\tOCASSET\t1000",
            "ACCNT\tChecking\tBANK\t1010",
            "ACCNT\tChecking:Sweep\tBANK\t1011",
            "ACCNT\tBank accounts:Petty cash\tOCASSET\t1020",
            "ACCNT\tFixed assets\tOCASSET\t1500",
            "ACCNT\tLaptop (1510)\tFIXASSET\t1510",
            "ACCNT\tLaptop (1600)\tOCASSET\t1600",
            "TRNS\t\tGENERAL JOURNAL\t3/1/2026\tLaptop (1510)\t\t1500.00\t1\tLaptop bought",
            "SPL\t\tGENERAL JOURNAL\t3/1/2026\tChecking:Sweep\t\t-1000.00\t1\tLaptop bought",
            "SPL\t\tGENERAL JOURNAL\t3/1/2026\tChecking\t\t-500.00\t1\tLaptop bought",
        ]
    );
    assert_eq!(
        iif.warnings(),
        [
            "account 1010's name is written at the top level, not under account 1000's: QuickBooks gives a subaccount its parent's type, and it is BANK, its parent OCASSET",
            "account 1510's name \"Laptop\" is written \"Laptop (1510)\"",
            "account 1510's name is written at the top level, not under account 1500's: QuickBooks gives a subaccount its parent's type, and it is FIXASSET, its parent OCASSET",
            "account 1600's name \"Laptop\" is written \"Laptop (1600)\"",
        ]
    );
    assert!(iif.receivable_payable.is_empty());
}

/// iif INVARIANT 1 (P3 review round 2): QuickBooks reads a subaccount's
/// NAME as its parent's and its own, so each parent is declared before its
/// subaccounts, else the accounts go in the order of their codes. A parent
/// coded after its subaccounts ("6000 Office" over "5900 Supplies" over
/// "5800 Paper") had been declared after them.
#[test]
fn iif_declares_each_parent_before_its_subaccounts() {
    let mut l = Ledger::new(Currency::USD);
    let mut add = |code: &str, name: &str, parent: Option<&str>| {
        let p = parent.map(|c| l.chart().find_by_code(c).unwrap().id);
        l.add_account(code, name, AccountType::Expense, p, false)
            .unwrap();
    };
    add("6000", "Office", None);
    add("5900", "Supplies", Some("6000"));
    add("5800", "Paper", Some("5900"));
    add("5000", "Rent", None);
    add("6100", "Software", None);
    add("5950", "Toner", Some("5900"));
    let accounts = iif_accounts(&l, |_| QbAccountType::Expense);
    let iif = journal_to_iif(&accounts, &[]).unwrap();
    check_iif(&iif.text, &accounts, &[]);
    let names: Vec<&str> = iif
        .text
        .split("\r\n")
        .filter_map(|l| l.strip_prefix("ACCNT\t"))
        .map(|l| l.split('\t').next().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "Rent",
            "Office",
            "Office:Supplies",
            "Office:Supplies:Paper",
            "Office:Supplies:Toner",
            "Software",
        ]
    );
    assert!(iif.changes.is_empty());
}

/// P3 review R12: a change prints as one line, whatever the books' text
/// holds: its line breaks and controls are escaped.
#[test]
fn iif_change_warnings_are_one_line_each() {
    let (mut l, cash, rev, _) = small_ledger();
    let e = l.create_entry(d(2026, 6, 1), "a\nb\u{1}c\u{2028}d");
    l.add_line(e, cash, Side::Debit, usd(100)).unwrap();
    l.add_line(e, rev, Side::Credit, usd(100)).unwrap();
    l.post_entry(e).unwrap();
    let accounts = iif_accounts(&l, |_| QbAccountType::Bank);
    let entries: Vec<&JournalEntry> = l.posted_entries().collect();
    let iif = journal_to_iif(&accounts, &entries).unwrap();
    assert_eq!(
        iif.changes
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        ["entry 1's memo \"a\\nb\\u{1}c\\u{2028}d\" is written \"a b?c d\""]
    );
}

/// QuickBooks Desktop's limits (Intuit, "Character limitations for fields
/// in QuickBooks": account name 31, account number 7, memo 4,095;
/// sources/intuit-iif.md). iif INVARIANT 2: a level past 31 characters is
/// cut there, and the clash check runs on the cut names, so two alike once
/// cut are tagged with their codes (cut to make room), as is one whose cut
/// form is another's own name; a subaccount follows its cut parent.
/// INVARIANT 6: an account number past 7 characters, or written like
/// another's, is left out of ACCNUM. INVARIANT 3: a memo past 4,095
/// characters is cut. Every one listed, in the exact words.
#[test]
fn iif_fits_quickbooks_desktop_limits() {
    let mut l = Ledger::new(Currency::USD);
    let mut add = |code: &str, name: &str, parent: Option<&str>| {
        let p = parent.map(|c| l.chart().find_by_code(c).unwrap().id);
        l.add_account(code, name, AccountType::Expense, p, false)
            .unwrap()
    };
    let fees = add("6100", "Professional services and consulting fees", None);
    add("6110", "Legal", Some("6100"));
    add("6200", "Professional services and consulting costs", None);
    add("6300", "Professional services and consu", None);
    add("6400", "Office supplies and small equipment", None);
    add("12345678", "Suspense", None);
    // Seven characters, the most: kept; two alike but for case: left out.
    add("1234567", "Escrow", None);
    add("CLR0001", "Clearing", None);
    add("clr0001", "Clearing, old", None);
    add("6500", "Bank fees", None);
    add(" 6500", "Card fees", None);
    let cash = add("1000", "Cash", None);
    let long_memo = format!("{}\u{2014}tail", "x".repeat(MEMO_MAX - 1));
    let e = l.create_entry(d(2026, 7, 1), &long_memo);
    l.add_line(e, fees, Side::Debit, usd(100)).unwrap();
    l.add_line(e, cash, Side::Credit, usd(100)).unwrap();
    l.post_entry(e).unwrap();
    let accounts = iif_accounts(&l, |_| QbAccountType::Expense);
    let entries: Vec<&JournalEntry> = l.posted_entries().collect();
    let iif = journal_to_iif(&accounts, &entries).unwrap();
    check_iif(&iif.text, &accounts, &entries);
    check_changes(&iif, &accounts, &entries);
    let accnt: Vec<&str> = iif
        .text
        .split("\r\n")
        .filter(|l| l.starts_with("ACCNT\t"))
        .collect();
    assert_eq!(
        accnt,
        [
            "ACCNT\tCard fees\tEXP\t",
            "ACCNT\tCash\tEXP\t1000",
            "ACCNT\tEscrow\tEXP\t1234567",
            "ACCNT\tSuspense\tEXP\t",
            "ACCNT\tProfessional services an (6100)\tEXP\t6100",
            "ACCNT\tProfessional services an (6100):Legal\tEXP\t6110",
            "ACCNT\tProfessional services an (6200)\tEXP\t6200",
            "ACCNT\tProfessional services an (6300)\tEXP\t6300",
            "ACCNT\tOffice supplies and small equip\tEXP\t6400",
            "ACCNT\tBank fees\tEXP\t",
            "ACCNT\tClearing\tEXP\t",
            "ACCNT\tClearing, old\tEXP\t",
        ]
    );
    for l in &accnt {
        let name = l.split('\t').nth(1).unwrap();
        assert!(
            name.split(':').all(|level| level.len() <= NAME_MAX),
            "{name:?}"
        );
    }
    let memo = &iif_memos(&iif.text)[0];
    assert_eq!(memo.len(), MEMO_MAX);
    assert_eq!(*memo, format!("{}-", "x".repeat(MEMO_MAX - 1)));
    assert_eq!(
        iif.warnings(),
        [
            "account number \" 6500\" is left out: another account's number would also be written \"6500\"".to_string(),
            "account number \"12345678\" is left out: QuickBooks Desktop holds 7 characters at most".to_string(),
            "account 6100's name \"Professional services and consulting fees\" is written \"Professional services an (6100)\"".to_string(),
            "account 6200's name \"Professional services and consulting costs\" is written \"Professional services an (6200)\"".to_string(),
            "account 6300's name \"Professional services and consu\" is written \"Professional services an (6300)\"".to_string(),
            "account 6400's name \"Office supplies and small equipment\" is written \"Office supplies and small equip\"".to_string(),
            "account number \"6500\" is left out: another account's number would also be written \"6500\"".to_string(),
            "account number \"CLR0001\" is left out: another account's number would also be written \"CLR0001\"".to_string(),
            "account number \"clr0001\" is left out: another account's number would also be written \"clr0001\"".to_string(),
            format!("\"\u{2014}\" is written \"-\" in the memo of entry {e}"),
            format!("the memo of entry {e} is cut to 4095 characters, the most QuickBooks Desktop holds"),
        ]
    );
    // A NAME built some other way, a level past the limit: refused.
    let mut bad = accounts.clone();
    bad[1].path = "x".repeat(NAME_MAX + 1);
    assert_eq!(
        journal_to_iif(&bad, &[]).unwrap_err(),
        IifError::NameTooLong {
            code: bad[1].code.clone(),
            name: "x".repeat(NAME_MAX + 1),
        }
    );
    assert_eq!((NAME_MAX, NUMBER_MAX, MEMO_MAX), (31, 7, 4095));
}

/// The README and the export's warnings group the memos (lead's call,
/// 2026-10-20): each thing written differently in them is one line with
/// every entry it is in, so a hundred depreciation memos with an em dash
/// make one line, not a hundred; a run of changed characters is one thing
/// ("北京" is "Bei Jing"); the lines come in the order first met, the
/// entries in order, each once; account names and numbers stay one line
/// each, first. No change is left out.
#[test]
fn iif_warnings_group_the_memos_by_what_is_written_differently() {
    let (mut l, cash, _, fees) = small_ledger();
    l.add_account(
        "6200",
        "Caf\u{E9} \u{2014} north",
        AccountType::Expense,
        None,
        false,
    )
    .unwrap();
    let post = |l: &mut Ledger, memo: &str| {
        let e = l.create_entry(d(2026, 2, 1), memo);
        l.add_line(e, fees, Side::Debit, usd(100)).unwrap();
        l.add_line(e, cash, Side::Credit, usd(100)).unwrap();
        l.post_entry(e).unwrap();
        e
    };
    let mut depreciation = Vec::new();
    for m in 1..=100 {
        depreciation.push(post(
            &mut l,
            &format!("Depreciation \u{2014} Laptop \u{2014} {m}"),
        ));
    }
    let beijing = post(&mut l, "Trip to \u{5317}\u{4EAC} \u{2014} visa");
    let plain_one = post(&mut l, "Plain memo");
    let zoe = post(&mut l, "Lunch, \"Zo\u{EB}\"\nteam \u{5317}\u{4EAC}");
    let accounts = iif_accounts(&l, |_| QbAccountType::Expense);
    let entries: Vec<&JournalEntry> = l.posted_entries().collect();
    let iif = journal_to_iif(&accounts, &entries).unwrap();
    check_changes(&iif, &accounts, &entries);
    assert_eq!(iif.changes.len(), 1 + 100 + 1 + 1, "one per name and memo");
    assert!(
        !iif.changes
            .iter()
            .any(|c| c.field == IifField::Memo { entry: plain_one })
    );
    let mut dashes = depreciation.clone();
    dashes.push(beijing);
    let list = |ids: &[u64]| {
        ids.iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    };
    assert_eq!(
        iif.warnings(),
        [
            "account 6200's name \"Café — north\" is written \"Cafe -- north\"".to_string(),
            format!(
                "\"—\" is written \"--\" in the memos of entries {}",
                list(&dashes)
            ),
            format!("\"北京\" is written \"Bei Jing\" in the memos of entries {beijing}, {zoe}"),
            format!("\"\\\"\" is written \"'\" in the memo of entry {zoe}"),
            // "ë", the quote and the line break are side by side: one run.
            format!("\"ë\\\"\\n\" is written \"e' \" in the memo of entry {zoe}"),
        ]
    );
}

/// INVARIANT 7: no warning holds a full TIN (rules.md B-26), though entry
/// descriptions and account names aren't checked for one: a memo's TIN
/// written with en dashes shows only the dashes; one in fullwidth digits,
/// one in Arabic-Indic digits, and an account name holding one, are
/// masked, digit by digit, and a line with digits masked says so; so is
/// the change's own line. So are a name and a memo that hold a TIN only
/// once written (digits a tab apart, which the file writes a space apart).
#[test]
fn iif_warnings_never_hold_a_tin() {
    let (mut l, cash, rev, _) = small_ledger();
    // Replayed, as books written before P3 hold them: a name holding a
    // TIN, or a tab, is refused in a new name (rules.md B-31), never in a
    // journaled one, so the file must still mask it.
    l.replay_add_account(
        "6300",
        "Loan from 123-45-6789 \u{2014} member",
        AccountType::Liability,
        None,
        false,
    )
    .unwrap();
    // A TIN only once written: its digits a tab apart, which the file
    // writes a space apart.
    l.replay_add_account(
        "6400",
        "Loan 123\t45\t6789",
        AccountType::Liability,
        None,
        false,
    )
    .unwrap();
    for memo in [
        "Paid SSN 123\u{2013}45\u{2013}6789",
        "Paid \u{FF11}\u{FF12}\u{FF13}\u{FF0D}\u{FF14}\u{FF15}\u{FF0D}\u{FF16}\u{FF17}\u{FF18}\u{FF19}",
        // Arabic-Indic digits, spaced em dashes: a TIN as the books keep it,
        // though " -- " is too wide a gap once written.
        "Ref \u{661}\u{662}\u{663} \u{2014} \u{664}\u{665} \u{2014} \u{666}\u{667}\u{668}\u{669}",
        // Arabic-Indic digits a tab apart: a TIN only once written.
        "Ref \u{661}\u{662}\u{663}\t\u{664}\u{665}\t\u{666}\u{667}\u{668}\u{669}",
    ] {
        let e = l.create_entry(d(2026, 6, 1), memo);
        l.add_line(e, cash, Side::Debit, usd(100)).unwrap();
        l.add_line(e, rev, Side::Credit, usd(100)).unwrap();
        l.post_entry(e).unwrap();
    }
    let accounts = iif_accounts(&l, |_| QbAccountType::Bank);
    let entries: Vec<&JournalEntry> = l.posted_entries().collect();
    let iif = journal_to_iif(&accounts, &entries).unwrap();
    check_changes(&iif, &accounts, &entries);
    assert_eq!(
        iif.warnings(),
        [
            "account 6300's name \"Loan from •••-••-•••• — member\" is written \"Loan from •••-••-•••• -- member\"",
            "account 6400's name \"Loan •••\\t••\\t••••\" is written \"Loan ••• •• ••••\"",
            "\"–\" is written \"-\" in the memo of entry 1",
            "\"•••－••－••••\" is written \"•••-••-••••\" in the memo of entry 2 (digits masked: may be a TIN)",
            "\"•••\" is written \"•••\" in the memo of entry 3 (digits masked: may be a TIN)",
            "\"—\" is written \"--\" in the memo of entry 3",
            "\"••\" is written \"••\" in the memo of entry 3 (digits masked: may be a TIN)",
            "\"••••\" is written \"••••\" in the memo of entry 3 (digits masked: may be a TIN)",
            "\"•••\\t••\\t••••\" is written \"••• •• ••••\" in the memo of entry 4 (digits masked: may be a TIN)",
        ]
    );
    assert_eq!(
        iif.changes[3].to_string(),
        "entry 2's memo \"Paid •••－••－••••\" is written \"Paid •••-••-••••\""
    );
    for c in &iif.changes {
        assert!(!ledger_core::parties::has_full_tin(&c.to_string()), "{c}");
    }
}

/// P3 review R7: the server records a package file holding a full TIN
/// (ledger_core::parties::has_full_tin) so it can't be filed back in. A
/// memo's TIN separated by en dashes or no-break spaces stays a TIN in
/// the file: they become ASCII hyphens and spaces, never '?'.
#[test]
fn iif_keeps_a_tin_readable_to_the_package_scan() {
    let (mut l, cash, rev, _) = small_ledger();
    for memo in [
        "Paid SSN 123\u{2013}45\u{2013}6789",
        "Paid SSN 123\u{A0}45\u{A0}6789",
    ] {
        assert!(ledger_core::parties::has_full_tin(memo));
        let e = l.create_entry(d(2026, 6, 1), memo);
        l.add_line(e, cash, Side::Debit, usd(100)).unwrap();
        l.add_line(e, rev, Side::Credit, usd(100)).unwrap();
        l.post_entry(e).unwrap();
    }
    let accounts = iif_accounts(&l, |_| QbAccountType::Bank);
    let entries: Vec<&JournalEntry> = l.posted_entries().collect();
    let iif = journal_to_iif(&accounts, &entries).unwrap();
    let memos = iif_memos(&iif.text);
    assert_eq!(memos, ["Paid SSN 123-45-6789", "Paid SSN 123 45 6789"]);
    for m in &memos {
        assert!(ledger_core::parties::has_full_tin(m), "{m}");
    }
}

/// QbAccountType::keyword: the IIF ACCNTTYPE keywords the doc comments name.
#[test]
fn qb_account_type_keywords() {
    use QbAccountType::*;
    let want = [
        (Bank, "BANK"),
        (AccountsReceivable, "AR"),
        (OtherCurrentAsset, "OCASSET"),
        (FixedAsset, "FIXASSET"),
        (OtherAsset, "OASSET"),
        (AccountsPayable, "AP"),
        (CreditCard, "CCARD"),
        (OtherCurrentLiability, "OCLIAB"),
        (LongTermLiability, "LTLIAB"),
        (Equity, "EQUITY"),
        (Income, "INC"),
        (CostOfGoodsSold, "COGS"),
        (Expense, "EXP"),
        (OtherIncome, "EXINC"),
        (OtherExpense, "EXEXP"),
    ];
    for (t, k) in want {
        assert_eq!(t.keyword(), k);
    }
}

// ---------------------------------------------------------------------------
// XLSX: a minimal zip reader and DEFLATE decoder (RFC 1951)
// ---------------------------------------------------------------------------

struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
    buf: u32,
    n: u32,
}

impl Bits<'_> {
    fn need(&mut self, k: u32) -> u32 {
        while self.n < k {
            self.buf |= (self.data[self.pos] as u32) << self.n;
            self.pos += 1;
            self.n += 8;
        }
        let v = if k == 0 {
            0
        } else {
            self.buf & ((1u32 << k) - 1)
        };
        self.buf = if k >= 32 { 0 } else { self.buf >> k };
        self.n -= k;
        v
    }
}

struct Huff {
    counts: [i32; 16],
    symbols: Vec<u16>,
}

impl Huff {
    fn new(lengths: &[u8]) -> Huff {
        let mut counts = [0i32; 16];
        for &l in lengths {
            counts[l as usize] += 1;
        }
        counts[0] = 0;
        let mut offs = [0usize; 16];
        for i in 1..16 {
            offs[i] = offs[i - 1] + counts[i - 1] as usize;
        }
        let mut symbols = vec![0u16; lengths.len()];
        for (s, &l) in lengths.iter().enumerate() {
            if l != 0 {
                symbols[offs[l as usize]] = s as u16;
                offs[l as usize] += 1;
            }
        }
        Huff { counts, symbols }
    }

    fn decode(&self, b: &mut Bits) -> u16 {
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for len in 1..16 {
            code |= b.need(1) as i32;
            let count = self.counts[len];
            if code - count < first {
                return self.symbols[(index + (code - first)) as usize];
            }
            index += count;
            first += count;
            first <<= 1;
            code <<= 1;
        }
        panic!("bad huffman code");
    }
}

const LBASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LEXT: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const DBASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DEXT: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];

fn inflate(data: &[u8]) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    let mut b = Bits {
        data,
        pos: 0,
        buf: 0,
        n: 0,
    };
    loop {
        let last = b.need(1);
        match b.need(2) {
            0 => {
                b.buf = 0;
                b.n = 0;
                let len = u16::from_le_bytes([data[b.pos], data[b.pos + 1]]) as usize;
                b.pos += 4;
                out.extend_from_slice(&data[b.pos..b.pos + len]);
                b.pos += len;
            }
            t @ (1 | 2) => {
                let (lit, dist) = if t == 1 {
                    let mut l = [0u8; 288];
                    for (i, x) in l.iter_mut().enumerate() {
                        *x = match i {
                            0..=143 => 8,
                            144..=255 => 9,
                            256..=279 => 7,
                            _ => 8,
                        };
                    }
                    (Huff::new(&l), Huff::new(&[5u8; 30]))
                } else {
                    let hlit = b.need(5) as usize + 257;
                    let hdist = b.need(5) as usize + 1;
                    let hclen = b.need(4) as usize + 4;
                    const ORDER: [usize; 19] = [
                        16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
                    ];
                    let mut cl = [0u8; 19];
                    for &o in ORDER.iter().take(hclen) {
                        cl[o] = b.need(3) as u8;
                    }
                    let clh = Huff::new(&cl);
                    let mut lens: Vec<u8> = Vec::new();
                    while lens.len() < hlit + hdist {
                        let sym = clh.decode(&mut b);
                        match sym {
                            0..=15 => lens.push(sym as u8),
                            16 => {
                                let prev = *lens.last().unwrap();
                                for _ in 0..3 + b.need(2) {
                                    lens.push(prev);
                                }
                            }
                            17 => lens.extend(std::iter::repeat_n(0, 3 + b.need(3) as usize)),
                            _ => lens.extend(std::iter::repeat_n(0, 11 + b.need(7) as usize)),
                        }
                    }
                    (Huff::new(&lens[..hlit]), Huff::new(&lens[hlit..]))
                };
                loop {
                    let sym = lit.decode(&mut b) as usize;
                    if sym < 256 {
                        out.push(sym as u8);
                    } else if sym == 256 {
                        break;
                    } else {
                        let i = sym - 257;
                        let len = LBASE[i] as usize + b.need(LEXT[i] as u32) as usize;
                        let ds = dist.decode(&mut b) as usize;
                        let dd = DBASE[ds] as usize + b.need(DEXT[ds] as u32) as usize;
                        let start = out.len() - dd;
                        for k in 0..len {
                            out.push(out[start + k]);
                        }
                    }
                }
            }
            _ => panic!("bad block type"),
        }
        if last == 1 {
            return out;
        }
    }
}

fn le16(b: &[u8], at: usize) -> usize {
    u16::from_le_bytes([b[at], b[at + 1]]) as usize
}
fn le32(b: &[u8], at: usize) -> usize {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]) as usize
}

/// Every file in a zip, by name, read through the central directory.
fn unzip(z: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let eocd = (0..=z.len() - 22)
        .rev()
        .find(|&i| z[i..i + 4] == [0x50, 0x4b, 0x05, 0x06])
        .expect("end of central directory");
    let count = le16(z, eocd + 10);
    let mut p = le32(z, eocd + 16);
    let mut files = BTreeMap::new();
    for _ in 0..count {
        assert_eq!(&z[p..p + 4], &[0x50, 0x4b, 0x01, 0x02]);
        let method = le16(z, p + 10);
        let csize = le32(z, p + 20);
        let (nl, el, cl) = (le16(z, p + 28), le16(z, p + 30), le16(z, p + 32));
        let local = le32(z, p + 42);
        let name = String::from_utf8(z[p + 46..p + 46 + nl].to_vec()).unwrap();
        let data_at = local + 30 + le16(z, local + 26) + le16(z, local + 28);
        let raw = &z[data_at..data_at + csize];
        let body = match method {
            0 => raw.to_vec(),
            8 => inflate(raw),
            m => panic!("zip method {m}"),
        };
        files.insert(name, body);
        p += 46 + nl + el + cl;
    }
    files
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// Text of every `<t>` inside `xml`, concatenated.
fn t_text(xml: &str) -> String {
    let mut out = String::new();
    let mut rest = xml;
    while let Some(i) = rest.find("<t") {
        let after = &rest[i + 2..];
        if !(after.starts_with('>') || after.starts_with(' ')) {
            rest = after;
            continue;
        }
        let open_end = after.find('>').unwrap();
        if after[..open_end].ends_with('/') {
            rest = &after[open_end + 1..];
            continue;
        }
        let body = &after[open_end + 1..];
        let close = body.find("</t>").unwrap();
        out.push_str(&unescape(&body[..close]));
        rest = &body[close + 4..];
    }
    out
}

#[derive(Debug, Clone, PartialEq)]
enum Cell {
    Num(String),
    Text(String),
}

/// The first worksheet's cells: (row, column index from 0) -> value.
fn sheet_cells(xlsx: &[u8]) -> BTreeMap<(u32, u32), Cell> {
    let files = unzip(xlsx);
    let shared: Vec<String> = files
        .get("xl/sharedStrings.xml")
        .map(|b| {
            let s = String::from_utf8(b.clone()).unwrap();
            s.split("<si>").skip(1).map(t_text).collect()
        })
        .unwrap_or_default();
    let sheet = String::from_utf8(files["xl/worksheets/sheet1.xml"].clone()).unwrap();
    let mut cells = BTreeMap::new();
    let mut rest = sheet.as_str();
    while let Some(i) = rest.find("<c ") {
        let after = &rest[i + 3..];
        let tag_end = after.find('>').unwrap();
        let attrs = &after[..tag_end];
        let attr = |name: &str| -> Option<String> {
            let k = format!("{name}=\"");
            attrs
                .find(&k)
                .map(|p| attrs[p + k.len()..].split('"').next().unwrap().to_string())
        };
        let r = attr("r").unwrap();
        let letters: String = r.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
        let row: u32 = r[letters.len()..].parse().unwrap();
        let col = letters
            .chars()
            .fold(0u32, |a, c| a * 26 + (c as u32 - 'A' as u32 + 1))
            - 1;
        if attrs.ends_with('/') {
            rest = &after[tag_end..];
            continue;
        }
        let body_all = &after[tag_end + 1..];
        let close = body_all.find("</c>").unwrap();
        let body = &body_all[..close];
        let v = body
            .find("<v>")
            .map(|p| unescape(&body[p + 3..p + 3 + body[p + 3..].find("</v>").unwrap()]));
        let cell = match attr("t").as_deref() {
            Some("s") => Some(Cell::Text(
                shared[v.unwrap().parse::<usize>().unwrap()].clone(),
            )),
            Some("inlineStr") | Some("str") => Some(Cell::Text(t_text(body))),
            None | Some("n") => v.map(Cell::Num),
            Some(_) => None,
        };
        if let Some(c) = cell {
            cells.insert((row, col), c);
        }
        rest = &body_all[close..];
    }
    cells
}

fn parse_num(v: &str) -> Decimal {
    if v.contains(['e', 'E']) {
        Decimal::from_scientific(v).unwrap()
    } else {
        v.parse().unwrap()
    }
}

/// A money text cell back to its amount: commas, '$', spaces dropped,
/// parentheses as a minus.
fn parse_money_text(s: &str) -> Option<Decimal> {
    let t = s.trim();
    let neg = t.starts_with('(') && t.ends_with(')');
    let body: String = t
        .chars()
        .filter(|c| !matches!(c, ',' | '$' | ' ' | '(' | ')'))
        .collect();
    let v: Decimal = body.parse().ok()?;
    Some(if neg { -v } else { v })
}

/// Excel shows a whole-cent amount exactly within 15 significant digits
/// (xlsx-numbers.md): definitely safe below 10^13, definitely not when
/// finer than a cent or more than 15 significant digits.
fn excel_safe(x: Decimal) -> Option<bool> {
    let whole_cents = x == x.round_dp(2);
    let sig = x.normalize().mantissa().unsigned_abs().to_string().len();
    if !whole_cents || sig > 15 {
        Some(false)
    } else if x.abs() < Decimal::from(10_000_000_000_000i64) {
        Some(true)
    } else {
        None
    }
}

/// XLSX output and its cells; checks the zip signature (INVARIANT 3).
fn render(st: &Statement) -> BTreeMap<(u32, u32), Cell> {
    let bytes = report_to_xlsx(&XlsxReport::Statement(st), &meta()).unwrap();
    assert_eq!(&bytes[..2], b"PK", "an XLSX is a zip");
    let files = unzip(&bytes);
    assert!(
        files.contains_key("xl/workbook.xml"),
        "a zip holding a workbook"
    );
    sheet_cells(&bytes)
}

/// The spreadsheet row holding `label` in some text cell, after `from`.
fn row_of(cells: &BTreeMap<(u32, u32), Cell>, label: &str, from: u32) -> Option<u32> {
    cells
        .iter()
        .find(|((r, _), c)| *r > from && **c == Cell::Text(label.to_string()))
        .map(|((r, _), _)| *r)
}

/// report_to_xlsx INVARIANTS 1-2 for one statement: every row, in order,
/// with its label; each amount a number cell equal to it when Excel can
/// show it exactly, its exact text when not; no number cell anywhere that
/// Excel would show wrong.
fn check_xlsx(st: &Statement) {
    let cells = render(st);
    for ((r, c), cell) in &cells {
        if let Cell::Num(v) = cell {
            let x = parse_num(v);
            assert!(
                excel_safe(x).unwrap_or(true),
                "number cell {r},{c} = {v} Excel can't show exactly"
            );
        }
    }
    let mut at = 0u32;
    for row in &st.rows {
        if row.label.is_empty() {
            continue;
        }
        let r = row_of(&cells, &row.label, at)
            .unwrap_or_else(|| panic!("row {:?} missing or out of order", row.label));
        at = r;
        let in_row: Vec<&Cell> = cells
            .iter()
            .filter(|((rr, _), c)| *rr == r && **c != Cell::Text(row.label.clone()))
            .map(|(_, c)| c)
            .collect();
        for a in row.amounts.iter().flatten() {
            match excel_safe(*a) {
                Some(true) => assert!(
                    in_row
                        .iter()
                        .any(|c| matches!(c, Cell::Num(v) if parse_num(v) == *a)),
                    "{:?}: {a} not a number cell (row has {in_row:?})",
                    row.label
                ),
                Some(false) => assert!(
                    in_row
                        .iter()
                        .any(|c| matches!(c, Cell::Text(t) if parse_money_text(t) == Some(*a))),
                    "{:?}: {a} not written as its exact text (row has {in_row:?})",
                    row.label
                ),
                None => assert!(
                    in_row.iter().any(|c| match c {
                        Cell::Num(v) => parse_num(v) == *a,
                        Cell::Text(t) => parse_money_text(t) == Some(*a),
                    }),
                    "{:?}: {a} missing",
                    row.label
                ),
            }
        }
        // Nothing else numeric in the row: no rounded stand-in.
        for c in &in_row {
            if let Cell::Num(v) = c {
                let x = parse_num(v);
                assert!(
                    row.amounts.iter().flatten().any(|a| *a == x),
                    "{:?}: number {v} isn't one of its amounts",
                    row.label
                );
            }
        }
    }
}

/// Whole-cent amounts, from small to past 15 significant digits.
fn amount_strategy() -> impl Strategy<Value = Decimal> {
    prop_oneof![
        4 => (-10_000_000_000_000i64..10_000_000_000_000).prop_map(cents),
        1 => (-10_000_000_000_000_000_000i128..10_000_000_000_000_000_000, 0u32..=2)
            .prop_map(|(m, s)| Decimal::from_i128_with_scale(m, s)),
    ]
}

/// Amounts finer than a cent.
fn subcent_strategy() -> impl Strategy<Value = Decimal> {
    (-100_000_000i64..100_000_000, 3u32..=6)
        .prop_map(|(m, s)| Decimal::new(m, s))
        .prop_filter("finer than a cent", |x| *x != x.round_dp(2))
}

fn random_statement(rows: Vec<(u8, Vec<Option<Decimal>>)>) -> Statement {
    let rows: Vec<Row> = rows
        .into_iter()
        .enumerate()
        .map(|(i, (k, a))| {
            let kind = [RowKind::Line, RowKind::Subtotal, RowKind::Section][k as usize];
            let amounts = if kind == RowKind::Section { vec![] } else { a };
            Row {
                kind,
                label: format!("Label {i} & <x>"),
                amounts,
                currency: false,
                rule_above: false,
            }
        })
        .collect();
    report::statement("Random", "As of now".into(), &["A", "B"], rows)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// report_to_xlsx INVARIANTS 1-3 over random statements of whole-cent
    /// amounts, including ones past 15 significant digits (exact text).
    #[test]
    fn xlsx_never_shows_a_wrong_number(
        rows in proptest::collection::vec(
            (0u8..3, proptest::collection::vec(proptest::option::of(amount_strategy()), 2)),
            1..20,
        )
    ) {
        check_xlsx(&random_statement(rows));
    }

    /// report_to_xlsx INVARIANT 1: an amount finer than a cent is "its exact
    /// text", never a rounded number or rounded text.
    #[test]
    fn xlsx_subcent_amounts_are_exact_text(
        rows in proptest::collection::vec(
            (0u8..2, proptest::collection::vec(proptest::option::of(subcent_strategy()), 2)),
            1..10,
        )
    ) {
        check_xlsx(&random_statement(rows));
    }
}

/// report_to_xlsx INVARIANT 1 at the boundaries xlsx-numbers.md derives:
/// 13 digits before the cents show exactly; a 16-significant-digit amount
/// or a fraction of a cent is written as exact text.
#[test]
fn xlsx_precision_boundaries() {
    let line = |label: &str, a: &str| Row {
        kind: RowKind::Line,
        label: label.into(),
        amounts: vec![Some(dec(a))],
        currency: false,
        rule_above: false,
    };
    let st = report::statement(
        "Edges",
        "x".into(),
        &["Total"],
        vec![
            line("max exact", "9999999999999.99"),
            line("neg max exact", "-9999999999999.99"),
            line("sixteen digits", "12345678901234.56"),
            line("neg sixteen digits", "-12345678901234.56"),
            line("plain", "-318.42"),
        ],
    );
    check_xlsx(&st);
}

/// report_to_xlsx INVARIANT 1: a fraction of a cent is written as its exact
/// text ("1.005"), not rounded.
#[test]
fn xlsx_subcent_boundaries() {
    let line = |label: &str, a: &str| Row {
        kind: RowKind::Line,
        label: label.into(),
        amounts: vec![Some(dec(a))],
        currency: false,
        rule_above: false,
    };
    let st = report::statement(
        "Edges",
        "x".into(),
        &["Total"],
        vec![line("half cent", "1.005"), line("tiny", "-0.001")],
    );
    check_xlsx(&st);
}

/// report_to_xlsx over every laid-out statement from real books (rows in
/// order, amounts as numbers) and the general ledger and journal reports
/// (a valid XLSX whose number cells Excel shows exactly).
#[test]
fn xlsx_renders_every_statement_and_ledger_report() {
    let specs: Vec<EntrySpec> = (0..12)
        .map(|i| EntrySpec {
            day: 190 + i * 13,
            accts: vec![
                (i as usize) % CHART.len(),
                (i as usize * 5 + 3) % CHART.len(),
            ],
            k: 1,
            debits: vec![1_234_567 + i * 101, 0, 0],
            seeds: vec![0; 3],
            memo: format!("entry {i}"),
        })
        .filter(|s| s.accts[0] != s.accts[1])
        .collect();
    let b = build_books(&specs, true);
    let (s, e) = (d(2026, 1, 1), d(2026, 12, 31));
    let pl = profit_and_loss_for_period(&b.ledger, s, e, None).unwrap();
    let bs = balance_sheet_as_of(&b.ledger, e).unwrap();
    let ye = YearEarnings {
        net_income_ytd: pl.net_income(),
        unclosed: pl.net_income(),
    };
    let cf = cash_flow(
        &b.ledger,
        &CashFlowParams {
            start: s,
            end: e,
            cash_accounts: vec![b.ids[0]],
            financing_accounts: vec![],
            investing_accounts: vec![],
            members: member_capitals(&b),
        },
    )
    .unwrap();
    let me = members_equity(&b.ledger, s, e, &member_capitals(&b)).unwrap();
    for st in [
        report::profit_and_loss(&pl),
        report::balance_sheet(&bs, ye, &members()),
        report::cash_flow(&cf),
        report::trial_balance(&trial_balance_as_of(&b.ledger, e), e),
        report::members_equity(&me),
    ] {
        check_xlsx(&st);
    }
    let gl = general_ledger(&b.ledger, s, e).unwrap();
    for rep in [
        XlsxReport::GeneralLedger(&gl),
        XlsxReport::GeneralJournal(&b.ledger, s, e),
    ] {
        let bytes = report_to_xlsx(&rep, &meta()).unwrap();
        assert_eq!(&bytes[..2], b"PK");
        for cell in sheet_cells(&bytes).values() {
            if let Cell::Num(v) = cell {
                assert_ne!(excel_safe(parse_num(v)), Some(false), "{v}");
            }
        }
    }
}

// ---------------------------------------------------------------------------
// PDF
// ---------------------------------------------------------------------------

const MARKUP: [&str; 9] = [
    "#panic()",
    "#{1+1}",
    "*bold*",
    "$x$",
    "[",
    "]",
    "\\",
    "#let x = 1; _under_ `raw` <lbl> @ref // c",
    "/* open comment",
];

fn markup_statement() -> Statement {
    let mut rows = vec![Row {
        kind: RowKind::Section,
        label: "Section #panic(\"boom\")".into(),
        amounts: vec![],
        currency: false,
        rule_above: false,
    }];
    for (i, m) in MARKUP.iter().enumerate() {
        rows.push(Row {
            kind: RowKind::Line,
            label: (*m).into(),
            amounts: vec![Some(cents(i as i64 * 12_345 - 40_000))],
            currency: false,
            rule_above: false,
        });
    }
    rows.push(Row {
        kind: RowKind::Subtotal,
        label: "Total $[*]$".into(),
        amounts: vec![Some(dec("-1234567.89"))],
        currency: false,
        rule_above: false,
    });
    rows.push(Row {
        kind: RowKind::Spacer,
        label: String::new(),
        amounts: vec![],
        currency: false,
        rule_above: false,
    });
    rows.push(Row {
        kind: RowKind::Total,
        label: "Net #{1+1}".into(),
        amounts: vec![Some(Decimal::ZERO)],
        currency: false,
        rule_above: false,
    });
    report::statement("Title #panic()", "Period *x*".into(), &["Total ["], rows)
}

fn strings_in(v: &serde_json::Value, out: &mut Vec<String>) {
    match v {
        serde_json::Value::String(s) => out.push(s.clone()),
        serde_json::Value::Array(a) => a.iter().for_each(|x| strings_in(x, out)),
        serde_json::Value::Object(o) => o.values().for_each(|x| strings_in(x, out)),
        _ => {}
    }
}

/// The array in `v` with one element per statement row.
fn rows_array(v: &serde_json::Value, n: usize) -> Option<&Vec<serde_json::Value>> {
    match v {
        serde_json::Value::Array(a) if a.len() == n && a.iter().all(|x| x.is_object()) => Some(a),
        serde_json::Value::Array(a) => a.iter().find_map(|x| rows_array(x, n)),
        serde_json::Value::Object(o) => o.values().find_map(|x| rows_array(x, n)),
        _ => None,
    }
}

/// statement_pdf INVARIANTS 1-3: labels holding Typst syntax render
/// without running (Ok, a PDF), the same input gives the same bytes, and
/// template_data carries each row's label verbatim, its kind, and its
/// amounts split for alignment, in order.
#[test]
fn pdf_is_deterministic_and_labels_are_data() {
    let st = markup_statement();
    let m = ReportMeta {
        company: "#panic() *Co* $x$ [ ] \\".into(),
        ..meta()
    };
    let a = statement_pdf(&st, &m).expect("labels with markup render");
    let b = statement_pdf(&st, &m).expect("again");
    assert!(a.starts_with(b"%PDF"), "a PDF");
    assert_eq!(a, b, "deterministic bytes");

    let data = ledger_import::pdf::template_data(&st, &m);
    let rows = rows_array(&data, st.rows.len()).expect("one element per row");
    for (row, el) in st.rows.iter().zip(rows) {
        let mut s = Vec::new();
        strings_in(el, &mut s);
        if !row.label.is_empty() {
            assert!(
                s.contains(&row.label),
                "label {:?} not verbatim in {el}",
                row.label
            );
        }
        let kind = serde_json::to_value(row.kind).unwrap();
        assert!(
            el.as_object().unwrap().values().any(|v| *v == kind),
            "kind {kind} missing in {el}"
        );
        for x in row.amounts.iter().flatten() {
            let digits = format_amount(x.abs());
            assert!(
                s.contains(&digits),
                "amount digits {digits:?} missing in {el}"
            );
        }
    }
    let mut all = Vec::new();
    strings_in(&data, &mut all);
    assert!(all.contains(&"Title #panic()".to_string()));
}

/// statement_pdf INVARIANT 2: one label per markup string, each rendered
/// alone (so one that would break the parse can't hide behind another).
#[test]
fn pdf_each_markup_label_alone_renders() {
    for m in MARKUP {
        let st = report::statement(
            m,
            m.to_string(),
            &[m],
            vec![Row {
                kind: RowKind::Line,
                label: m.into(),
                amounts: vec![Some(dec("1.00"))],
                currency: false,
                rule_above: false,
            }],
        );
        let bytes = statement_pdf(&st, &meta()).unwrap_or_else(|e| panic!("{m:?}: {e}"));
        assert!(bytes.starts_with(b"%PDF"));
    }
}

/// Every FlateDecode stream of a PDF, inflated (zlib: a two-byte header,
/// then RFC 1951), joined as text: the content, the fonts' ToUnicode maps
/// and the object streams that hold the fonts' dictionaries.
fn pdf_streams(pdf: &[u8]) -> String {
    let find = |from: usize, pat: &[u8]| {
        pdf[from..]
            .windows(pat.len())
            .position(|w| w == pat)
            .map(|i| from + i)
    };
    let mut out = String::new();
    let mut at = 0;
    while let Some(s) = find(at, b"stream") {
        let dict_from = pdf[..s].windows(3).rposition(|w| w == b"obj").unwrap_or(0);
        let mut body = s + b"stream".len();
        if pdf[body..].starts_with(b"\r\n") {
            body += 2;
        } else if pdf[body..].starts_with(b"\n") {
            body += 1;
        } else {
            // "endstream" itself, or a word holding "stream".
            at = body;
            continue;
        }
        let end = find(body, b"endstream").expect("endstream");
        let dict = String::from_utf8_lossy(&pdf[dict_from..s]);
        if dict.contains("/FlateDecode") {
            out.push_str(&String::from_utf8_lossy(&inflate(&pdf[body + 2..end])));
            out.push('\n');
        }
        at = end + b"endstream".len();
    }
    out
}

/// statement_pdf INVARIANT 4: a company, an account and a vendor named in
/// Chinese, Japanese and Korean go into the PDF's text: the CJK font is
/// embedded, and its ToUnicode map gives back every one of their
/// characters, so they copy, search and print as written.
#[test]
fn pdf_keeps_chinese_japanese_and_korean_names() {
    let names = ["北京商贸", "東京事務所", "서울상사", "ありがとう"];
    let st = report::statement(
        "Profit and Loss",
        "2026".to_string(),
        &["Total"],
        names
            .iter()
            .map(|n| Row {
                kind: RowKind::Line,
                label: format!("6100 {n}"),
                amounts: vec![Some(dec("1.00"))],
                currency: false,
                rule_above: false,
            })
            .collect(),
    );
    let m = ReportMeta {
        company: "株式会社 Example".into(),
        ..meta()
    };
    let pdf = statement_pdf(&st, &m).unwrap();
    let text = pdf_streams(&pdf).to_lowercase();
    assert!(text.contains("notosanscjk"), "the CJK font is embedded");
    for c in names.concat().chars().chain("株式会社".chars()) {
        let hex = format!("<{:04x}>", u32::from(c));
        assert!(text.contains(&hex), "{c} ({hex}) maps back to text");
    }
}

/// The text a PDF draws, each line from the top of each page down, its
/// glyphs left to right, each the text its font's ToUnicode map gives
/// back, read with hayro (the viewer's own PDF reader): what a reader
/// copies, searches and speaks.
fn pdf_lines(pdf: &[u8]) -> Vec<Vec<String>> {
    use hayro_interpret::font::GlyphRun;
    use hayro_interpret::hayro_cmap::BfString;
    use hayro_interpret::hayro_syntax::Pdf;
    use hayro_interpret::{
        BlendMode, ClipPath, Context, Device, DrawMode, DrawProps, Image, ImageDrawProps,
        InterpreterCache, InterpreterSettings, SoftMask, interpret_page,
    };
    use kurbo::{Affine, BezPath, Point, Rect};
    /// Each glyph drawn: its baseline (tenths of a point), x, and text.
    struct Glyphs(Vec<(i64, f64, String)>);
    impl<'a> Device<'a> for Glyphs {
        fn draw_path(&mut self, _: &BezPath, _: DrawProps<'a>, _: &DrawMode) {}
        fn push_clip_path(&mut self, _: &ClipPath) {}
        fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
        fn draw_glyph_run(&mut self, run: &GlyphRun<'_, 'a>, props: DrawProps<'a>, _: &DrawMode) {
            for g in run.glyphs() {
                let at = (props.transform * g.transform()) * Point::ZERO;
                let text = match g.as_unicode() {
                    Some(BfString::Char(c)) => c.to_string(),
                    Some(BfString::String(s)) => s,
                    // A mark or a ligature's second glyph: its text is
                    // with the cluster's first.
                    None => String::new(),
                };
                self.0.push(((at.y * 10.0).round() as i64, at.x, text));
            }
        }
        fn draw_image(&mut self, _: Image<'a, '_>, _: ImageDrawProps<'a>) {}
        fn pop_clip(&mut self) {}
        fn pop_transparency_group(&mut self) {}
    }
    let pdf = Pdf::new(pdf.to_vec()).unwrap();
    let cache = InterpreterCache::new();
    let mut out = Vec::new();
    for page in pdf.pages().iter() {
        let mut ctx = Context::new(
            Affine::IDENTITY,
            Rect::new(0.0, 0.0, 1.0, 1.0),
            &cache,
            pdf.xref(),
            InterpreterSettings::default(),
        );
        let mut glyphs = Glyphs(Vec::new());
        interpret_page(page, &mut ctx, &mut glyphs);
        // PDF space runs up the page: the highest baseline first.
        let mut lines: BTreeMap<std::cmp::Reverse<i64>, Vec<(f64, String)>> = BTreeMap::new();
        for (y, x, t) in glyphs.0 {
            lines.entry(std::cmp::Reverse(y)).or_default().push((x, t));
        }
        for (_, mut line) in lines {
            line.sort_by(|a, b| a.0.total_cmp(&b.0));
            out.push(line.into_iter().map(|(_, t)| t).collect());
        }
    }
    out
}

/// statement_pdf INVARIANT 4 (P3 review round 2): a company and accounts
/// named in Arabic, Hebrew, Thai, Devanagari and Chinese print as written
/// in the bundled Noto fallbacks, and the PDF's text reads back as the
/// books keep it, Arabic and Hebrew drawn right to left, so their glyphs
/// read from the right give the words (the Arabic shaped, its lam-alef a
/// ligature, still reads back "سلام"; with Noto Sans Arabic 2.013, whose
/// dots are marks Typst sets apart, it read "شلام" and doubled letters),
/// the others left to right.
/// Before the fallbacks, every one of these was an empty box.
#[test]
fn pdf_prints_every_bundled_script_as_written() {
    let rtl = ["مكتب سلام", "שלום עולם"];
    let ltr = ["สวัสดีครับ", "नमस्ते", "北京商贸"];
    let st = report::statement(
        "Profit and Loss",
        "2026".to_string(),
        &["Total"],
        rtl.iter()
            .chain(&ltr)
            .map(|n| Row {
                kind: RowKind::Line,
                label: n.to_string(),
                amounts: vec![Some(dec("1.00"))],
                currency: false,
                rule_above: false,
            })
            .collect(),
    );
    let m = ReportMeta {
        company: "شركة Example".into(),
        ..meta()
    };
    let pdf = statement_pdf(&st, &m).unwrap();
    let lines = pdf_lines(&pdf);
    let squeeze = |s: &str| s.replace(' ', "");
    let read = |reversed: bool| -> Vec<String> {
        lines
            .iter()
            .map(|l| {
                let mut l = l.clone();
                if reversed {
                    l.reverse();
                }
                squeeze(&l.concat())
            })
            .collect()
    };
    let (left, right) = (read(false), read(true));
    for word in rtl.iter().chain(&["شركة"]) {
        assert!(
            right.iter().any(|l| l.contains(&squeeze(word))),
            "{word} reads right to left: {lines:?}"
        );
        assert!(
            !left.iter().any(|l| l.contains(&squeeze(word))),
            "{word} isn't drawn left to right"
        );
    }
    for word in ltr {
        assert!(
            left.iter().any(|l| l.contains(&squeeze(word))),
            "{word} reads left to right: {lines:?}"
        );
    }
    // The fonts' names (BaseFont), in the PDF's objects or its streams.
    let fonts = format!("{}{}", String::from_utf8_lossy(&pdf), pdf_streams(&pdf)).to_lowercase();
    for f in [
        "notosansarabicui",
        "notosanshebrew",
        "notosansthai",
        "notosansdevanagari",
        "notosanscjk",
    ] {
        assert!(fonts.contains(f), "{f} is embedded");
    }
}

// ---------------------------------------------------------------------------
// chart_csv
// ---------------------------------------------------------------------------

/// parse_chart_csv INVARIANTS 1 and 3: the header in any case, optional
/// fifth tax_category; blank parent/category is None; blank lines skipped.
#[test]
fn chart_csv_header_any_case_and_blanks() {
    let text = "\n\nCODE,Name,TYPE,Parent_Code\n1000,Cash,ASSET,\n\n1010,Checking,asset,1000\n";
    let rows = parse_chart_csv(text).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].code, "1000");
    assert_eq!(rows[0].account_type, "asset");
    assert_eq!(rows[0].parent_code, None);
    assert_eq!(rows[0].tax_category, None);
    assert_eq!(rows[1].parent_code.as_deref(), Some("1000"));

    let text = "code,name,type,parent_code,TAX_CATEGORY\n6000,Rent,Expense,,rent_lease_other\n6010,Misc,expense,,\n";
    let rows = parse_chart_csv(text).unwrap();
    assert_eq!(rows[0].tax_category.as_deref(), Some("rent_lease_other"));
    assert_eq!(rows[0].account_type, "expense");
    assert_eq!(rows[1].tax_category, None);

    assert_eq!(parse_chart_csv("").unwrap(), vec![]);
    assert_eq!(parse_chart_csv("\n\n").unwrap(), vec![]);
}

/// parse_chart_csv INVARIANTS 1-2: anything but the header is BadHeader;
/// wrong width BadColumnCount(found, expected); unknown type BadType; blank
/// code or name Blank with its row (header is row 1).
#[test]
fn chart_csv_refusals() {
    for bad in [
        "code,name,kind,parent_code\n",
        "code,name,type\n",
        "name,code,type,parent_code\n",
        "code,name,type,parent_code,tax_category,extra\n",
        "1000,Cash,asset,\n",
    ] {
        assert_eq!(
            parse_chart_csv(bad),
            Err(ChartCsvError::BadHeader),
            "{bad:?}"
        );
    }
    assert_eq!(
        parse_chart_csv("code,name,type,parent_code\n1000,Cash,asset\n"),
        Err(ChartCsvError::BadColumnCount(3, 4))
    );
    assert_eq!(
        parse_chart_csv("code,name,type,parent_code,tax_category\n1000,Cash,asset,\n"),
        Err(ChartCsvError::BadColumnCount(4, 5))
    );
    assert!(matches!(
        parse_chart_csv("code,name,type,parent_code\n1000,Cash,cash,\n"),
        Err(ChartCsvError::BadType(_))
    ));
    assert_eq!(
        parse_chart_csv("code,name,type,parent_code\n1000,Cash,asset,\n ,X,asset,\n"),
        Err(ChartCsvError::Blank {
            row: 3,
            field: "code"
        })
    );
    assert_eq!(
        parse_chart_csv("code,name,type,parent_code\n1000,,asset,\n"),
        Err(ChartCsvError::Blank {
            row: 2,
            field: "name"
        })
    );
}

proptest! {
    /// parse_chart_csv INVARIANTS 1-3 as a round trip: random rows written
    /// with the csv crate parse back to themselves, type lowercased.
    #[test]
    fn chart_csv_round_trip(
        rows in proptest::collection::vec(
            ("[0-9]{1,6}", "[A-Za-z][A-Za-z ,\"'&—é]{0,15}[A-Za-z]", 0usize..5, any::<bool>(),
             proptest::option::of("[0-9]{1,6}"), proptest::option::of("[a-z_]{1,12}")),
            0..20),
        with_tax in any::<bool>(),
    ) {
        let types = ["asset", "LIABILITY", "Equity", "revenue", "EXPENSE"];
        let mut out = vec![];
        let mut hdr = vec!["code", "name", "type", "parent_code"];
        if with_tax { hdr.push("tax_category"); }
        out.push(hdr.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        for (code, name, t, _, parent, tax) in &rows {
            let mut r = vec![code.clone(), name.clone(), types[*t].into(), parent.clone().unwrap_or_default()];
            if with_tax { r.push(tax.clone().unwrap_or_default()); }
            out.push(r);
        }
        let got = parse_chart_csv(&csv_text(&out)).unwrap();
        prop_assert_eq!(got.len(), rows.len());
        for (g, (code, name, t, _, parent, tax)) in got.iter().zip(&rows) {
            prop_assert_eq!(&g.code, code);
            prop_assert_eq!(&g.name, name);
            prop_assert_eq!(&g.account_type, &types[*t].to_lowercase());
            prop_assert_eq!(&g.parent_code, parent);
            prop_assert_eq!(&g.tax_category, if with_tax { tax } else { &None });
        }
    }
}

// ---------------------------------------------------------------------------
// templates (what tests/templates.rs doesn't already cover)
// ---------------------------------------------------------------------------

fn owners(n: usize) -> Vec<String> {
    (0..n).map(|i| format!("Owner{i}")).collect()
}

const ENTITIES: [EntityType; 4] = [
    EntityType::SoleProprietorship,
    EntityType::Partnership,
    EntityType::SCorporation,
    EntityType::CCorporation,
];

/// templates INVARIANT 1: codes unique, parents before children, valid
/// types, for every entity and any owner count; and the template's CSV is
/// a chart CSV that parses back to exactly its rows (the import path).
#[test]
fn templates_rows_import_and_csv_round_trips() {
    let valid = ["asset", "liability", "equity", "revenue", "expense"];
    for entity in ENTITIES {
        for n in 1..=6 {
            let Ok(t) = chart_template(entity, &owners(n)) else {
                assert!(
                    entity == EntityType::Partnership && n < 2,
                    "{entity:?} with {n} owners refused"
                );
                continue;
            };
            let mut seen = HashSet::new();
            for r in &t.rows {
                assert!(
                    valid.contains(&r.account_type.as_str()),
                    "{}",
                    r.account_type
                );
                if let Some(p) = &r.parent_code {
                    assert!(
                        seen.contains(p),
                        "{entity:?}: {} before its parent {p}",
                        r.code
                    );
                }
                assert!(
                    seen.insert(r.code.clone()),
                    "{entity:?}: duplicate code {}",
                    r.code
                );
            }
            assert_eq!(parse_chart_csv(&t.csv()).unwrap(), t.rows, "{entity:?}");
        }
    }
}

/// templates INVARIANT 3 (and the prose: "at least two for a
/// partnership"): one owner is refused; any number from 2 up gets a capital
/// and a distributions account each, shares summing to exactly 100%.
#[test]
fn templates_partnership_any_owner_count() {
    assert!(chart_template(EntityType::Partnership, &owners(1)).is_err());
    assert!(chart_template(EntityType::Partnership, &[]).is_err());
    for n in 2..=12 {
        let os = owners(n);
        let t = chart_template(EntityType::Partnership, &os).unwrap();
        for o in &os {
            assert_eq!(
                t.rows
                    .iter()
                    .filter(|r| r.name == format!("Capital — {o}"))
                    .count(),
                1
            );
            assert_eq!(
                t.rows
                    .iter()
                    .filter(|r| r.name == format!("Distributions — {o}"))
                    .count(),
                1
            );
        }
        let shares: Vec<Decimal> = t
            .settings_toml
            .lines()
            .filter_map(|l| {
                l.trim()
                    .strip_prefix("share = \"")
                    .and_then(|s| s.strip_suffix("%\""))
            })
            .map(|s| s.parse().unwrap())
            .collect();
        assert_eq!(shares.len(), n, "a share per partner");
        assert!(shares.iter().all(|s| *s > Decimal::ZERO));
        assert_eq!(
            shares.iter().copied().sum::<Decimal>(),
            Decimal::from(100),
            "{n} owners"
        );
    }
}

/// EntityType::parse: the four spellings parse; "llc" alone is ambiguous.
#[test]
fn entity_type_parse() {
    assert_eq!(
        EntityType::parse("sole_proprietorship"),
        Some(EntityType::SoleProprietorship)
    );
    assert_eq!(
        EntityType::parse("partnership"),
        Some(EntityType::Partnership)
    );
    assert_eq!(
        EntityType::parse("s_corporation"),
        Some(EntityType::SCorporation)
    );
    assert_eq!(
        EntityType::parse("c_corporation"),
        Some(EntityType::CCorporation)
    );
    assert_eq!(EntityType::parse("llc"), None);
}

// ---------------------------------------------------------------------
// XLSX general ledger and general journal sheets, header and footer
// (added after the store-style mutation run found them untested).
// ---------------------------------------------------------------------

fn num_at(cells: &BTreeMap<(u32, u32), Cell>, r: u32, c: u32) -> Option<Decimal> {
    match cells.get(&(r, c)) {
        Some(Cell::Num(v)) => Some(parse_num(v)),
        Some(Cell::Text(t)) => parse_money_text(t),
        None => None,
    }
}

fn text_at(cells: &BTreeMap<(u32, u32), Cell>, r: u32, c: u32) -> Option<&str> {
    match cells.get(&(r, c)) {
        Some(Cell::Text(t)) => Some(t.as_str()),
        _ => None,
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

    /// The general ledger sheet: per account a beginning balance, each
    /// posting with its running balance (each step moves it by the posting's
    /// debit minus credit, the same sign throughout an account), and a total
    /// row whose debits and credits foot and whose balance is the last one.
    #[test]
    fn xlsx_general_ledger_runs_and_foots((specs, oe, end_off) in books_strategy()) {
        let b = build_books(&specs, oe);
        let (s, e) = (d(2025, 7, 1), d(2025, 7, 1) + chrono::Duration::days(end_off));
        let gl = general_ledger(&b.ledger, s, e).unwrap();
        let bytes = report_to_xlsx(&XlsxReport::GeneralLedger(&gl), &meta()).unwrap();
        let cells = sheet_cells(&bytes);
        for a in &gl.accounts {
            let name = format!("{} {}", a.code, a.name);
            // The heading is the account's name alone in the first column
            // (the Split column holds account names too).
            let head = cells
                .iter()
                .find(|((_, c), v)| *c == 0 && **v == Cell::Text(name.clone()))
                .map(|((r, _), _)| *r)
                .expect("account heading");
            prop_assert_eq!(text_at(&cells, head + 1, 2), Some("Beginning balance"));
            let mut bal = num_at(&cells, head + 1, 6).unwrap();
            prop_assert_eq!(bal, a.beginning_balance);
            let (mut dr, mut cr) = (Decimal::ZERO, Decimal::ZERO);
            let mut sign: Option<Decimal> = None;
            let mut r = head + 2;
            while text_at(&cells, r, 0) != Some(format!("Total for {name}").as_str()) {
                let debit = num_at(&cells, r, 4).unwrap_or_default();
                let credit = num_at(&cells, r, 5).unwrap_or_default();
                let next = num_at(&cells, r, 6).unwrap();
                let step = debit - credit;
                if !step.is_zero() {
                    let s = (next - bal) / step;
                    prop_assert!(s == Decimal::ONE || s == -Decimal::ONE, "row {}", r);
                    if let Some(prev) = sign { prop_assert_eq!(prev, s); }
                    sign = Some(s);
                }
                dr += debit;
                cr += credit;
                bal = next;
                r += 1;
                prop_assert!(r < head + 10_000, "no total row for {}", name);
            }
            prop_assert_eq!(num_at(&cells, r, 4).unwrap(), dr);
            prop_assert_eq!(num_at(&cells, r, 5).unwrap(), cr);
            prop_assert_eq!(num_at(&cells, r, 6).unwrap(), bal);
            prop_assert_eq!(bal, a.ending_balance);
        }
    }

    /// The general journal sheet: exactly the posted lines dated in the
    /// range (each with its date inside it), and a total row whose debits
    /// and credits foot and are equal.
    #[test]
    fn xlsx_general_journal_holds_the_range_and_foots((specs, oe, end_off) in books_strategy()) {
        let b = build_books(&specs, oe);
        let (s, e) = (d(2025, 7, 10), d(2025, 7, 10) + chrono::Duration::days(end_off / 2));
        let bytes = report_to_xlsx(&XlsxReport::GeneralJournal(&b.ledger, s, e), &meta()).unwrap();
        let cells = sheet_cells(&bytes);
        let want_lines: usize = b
            .ledger
            .posted_entries()
            .filter(|x| x.date() >= s && x.date() <= e)
            .map(|x| x.lines().len())
            .sum();
        let header = row_of(&cells, "DATE", 0).expect("column headings");
        let (mut dr, mut cr, mut lines) = (Decimal::ZERO, Decimal::ZERO, 0usize);
        let mut r = header + 1;
        while text_at(&cells, r, 0) != Some("Total") {
            let date: NaiveDate = text_at(&cells, r, 0).unwrap().parse().unwrap();
            prop_assert!(date >= s && date <= e, "{} outside the range", date);
            dr += num_at(&cells, r, 5).unwrap_or_default();
            cr += num_at(&cells, r, 6).unwrap_or_default();
            lines += 1;
            r += 1;
        }
        prop_assert_eq!(lines, want_lines);
        prop_assert_eq!(num_at(&cells, r, 5).unwrap(), dr);
        prop_assert_eq!(num_at(&cells, r, 6).unwrap(), cr);
        prop_assert_eq!(dr, cr);
    }
}

/// Every sheet names the company, the report, and its period at the top,
/// and ends with the footer (basis, preparation date, journal seq).
#[test]
fn xlsx_sheets_carry_header_and_footer() {
    let b = build_books(&[], false);
    let (s, e) = (d(2025, 7, 1), d(2025, 7, 31));
    let pl = profit_and_loss_for_period(&b.ledger, s, e, None).unwrap();
    let st = report::profit_and_loss(&pl);
    let gl = general_ledger(&b.ledger, s, e).unwrap();
    for (rep, title) in [
        (XlsxReport::Statement(&st), "Profit and Loss"),
        (XlsxReport::GeneralLedger(&gl), "General Ledger"),
        (
            XlsxReport::GeneralJournal(&b.ledger, s, e),
            "General Journal",
        ),
    ] {
        let cells = sheet_cells(&report_to_xlsx(&rep, &meta()).unwrap());
        let texts: Vec<&str> = cells
            .values()
            .filter_map(|c| match c {
                Cell::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect();
        let m = meta();
        for want in [
            m.company.as_str(),
            title,
            &report::period_label(s, e),
            &m.footer(),
        ] {
            prop_assert_contains(&texts, want, title);
        }
    }
}

fn prop_assert_contains(texts: &[&str], want: &str, ctx: &str) {
    assert!(
        texts.iter().any(|t| t.contains(want)),
        "{ctx}: no cell holds {want:?}"
    );
}

/// The tax summary statement's Difference column is blank where book and
/// tax agree and book - tax where they don't (meals at 50%); separately
/// stated items sit in their own section, outside the net.
#[test]
fn tax_summary_difference_and_schedule_k() {
    use ledger_core::tax::{TaxForm, tax_summary};
    let mut l = Ledger::new(Currency::USD);
    let cash = l
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    let mut acct = |code: &str, t, cat: &str| {
        let id = l.add_account(code, code, t, None, false).unwrap();
        l.set_account_tax(id, Some(cat), None).unwrap();
        id
    };
    let sales = acct("4000", AccountType::Revenue, "gross_receipts");
    let interest = acct("4200", AccountType::Revenue, "interest_income");
    let meals = acct("6450", AccountType::Expense, "meals");
    for (dr, cr, c) in [
        (cash, sales, 100_000),
        (cash, interest, 2_500),
        (meals, cash, 10_000),
    ] {
        let e = l.create_entry(d(2026, 2, 1), "x");
        l.add_line(e, dr, Side::Debit, usd(c)).unwrap();
        l.add_line(e, cr, Side::Credit, usd(c)).unwrap();
        l.post_entry(e).unwrap();
    }
    let ts = tax_summary(&l, d(2026, 1, 1), d(2026, 12, 31), Some(TaxForm::Form1065)).unwrap();
    let st = report::tax_summary(&ts);
    let row = |label: &str| {
        st.rows
            .iter()
            .find(|r| r.label.starts_with(label))
            .unwrap_or_else(|| panic!("no row {label}"))
    };
    let sales_row = row("Gross receipts");
    assert_eq!(
        sales_row.amounts,
        vec![Some(cents(100_000)), Some(cents(100_000)), None]
    );
    let meals_row = row("Meals");
    assert_eq!(
        meals_row.amounts,
        vec![Some(cents(10_000)), Some(cents(5_000)), Some(cents(5_000))]
    );
    assert!(
        st.rows
            .iter()
            .any(|r| r.label.starts_with("Separately stated"))
    );
    let net = row("Income less deductions");
    assert_eq!(
        net.amounts[0],
        Some(cents(90_000)),
        "interest is outside the net"
    );
}

/// A gain on business property (rules.md B-32; ledger_core::tax
/// `TaxSummary::form_4797`): on a Schedule C business's summary it is in
/// its own Form 4797 section, at its book amount with no tax amount, and
/// outside the net, never Schedule C income.
#[test]
fn tax_summary_lists_form_4797_apart() {
    use ledger_core::tax::{TaxForm, tax_summary};
    let mut l = Ledger::new(Currency::USD);
    let cash = l
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    let mut acct = |code: &str, t, cat: &str| {
        let id = l.add_account(code, code, t, None, false).unwrap();
        l.set_account_tax(id, Some(cat), None).unwrap();
        id
    };
    let sales = acct("4000", AccountType::Revenue, "gross_receipts");
    let gain = acct("4700", AccountType::Revenue, "form_4797_gain_loss");
    for (cr, c) in [(sales, 100_000), (gain, 7_500)] {
        let e = l.create_entry(d(2026, 2, 1), "x");
        l.add_line(e, cash, Side::Debit, usd(c)).unwrap();
        l.add_line(e, cr, Side::Credit, usd(c)).unwrap();
        l.post_entry(e).unwrap();
    }
    let ts = tax_summary(&l, d(2026, 1, 1), d(2026, 12, 31), Some(TaxForm::ScheduleC)).unwrap();
    let st = report::tax_summary(&ts);
    let at = |label: &str| {
        st.rows
            .iter()
            .position(|r| r.label.starts_with(label))
            .unwrap_or_else(|| panic!("no row {label}"))
    };
    let section = at("Gains and losses on business property (Form 4797)");
    let line = at("Gain or loss on business property (Form 4797) (4700)");
    assert!(line > section && section > at("Income less deductions"));
    assert_eq!(st.rows[line].amounts, vec![Some(cents(7_500)), None, None]);
    assert_eq!(
        st.rows[at("Total income")].amounts[0],
        Some(cents(100_000)),
        "the gain is no Schedule C income"
    );
    assert_eq!(
        st.rows[at("Income less deductions")].amounts,
        vec![Some(cents(100_000)), Some(cents(100_000)), None]
    );
}

/// The reconciliation's "Adjustments:" subheading appears only when there
/// is something to reconcile, and the noncash schedule says "None" when
/// nothing was disclosed (cash_flow INVARIANT 5). A contribution of
/// property is disclosed under its label; depreciation is an adjustment.
#[test]
fn cash_flow_adjustments_appear_only_when_there_are_some() {
    let mut l = Ledger::new(Currency::USD);
    let mut add = |code: &str, t| l.add_account(code, code, t, None, false).unwrap();
    let cash = add("1000", AccountType::Asset);
    let equip = add("1500", AccountType::Asset);
    let accum = add("1510", AccountType::Asset);
    let cap = add("3000", AccountType::Equity);
    let rev = add("4000", AccountType::Revenue);
    let depr = add("6100", AccountType::Expense);
    let post = |l: &mut Ledger, day, desc: &str, dr, cr, n: i64| {
        let e = l.create_entry(d(2026, 1, day), desc);
        l.add_line(e, dr, Side::Debit, usd(n)).unwrap();
        l.add_line(e, cr, Side::Credit, usd(n)).unwrap();
        l.post_entry(e).unwrap();
    };
    post(&mut l, 2, "Sale", cash, rev, 1_000);
    let st = |l: &Ledger| {
        let cf = cash_flow(
            l,
            &CashFlowParams {
                start: d(2026, 1, 1),
                end: d(2026, 1, 31),
                cash_accounts: vec![cash],
                financing_accounts: vec![],
                investing_accounts: vec![],
                members: vec![],
            },
        )
        .unwrap();
        report::cash_flow(&cf)
    };
    let has = |st: &Statement, p: &str| st.rows.iter().any(|r| r.label.starts_with(p));
    let plain = st(&l);
    assert!(!has(&plain, "Adjustments") && !has(&plain, "Depreciation"));
    assert_eq!(section_lines(&plain, "Noncash")[0].label, "None");
    l.register_asset(ledger_core::NewAsset {
        description: "Laptop".into(),
        in_service_date: d(2026, 1, 3),
        cost: usd(900),
        salvage_value: usd(0),
        useful_life_months: 36,
        cost_account: equip,
        accum_depr_account: accum,
        expense_account: depr,
    })
    .unwrap();
    post(&mut l, 3, "Laptop contributed", equip, cap, 900);
    post(&mut l, 31, "Depreciation", depr, accum, 25);
    let dep = st(&l);
    assert!(has(&dep, "Adjustments") && has(&dep, "Depreciation"));
    let noncash = section_lines(&dep, "Noncash");
    assert_eq!(noncash.len(), 1);
    assert_eq!(noncash[0].label, "Laptop contributed (#2)");
    assert_eq!(noncash[0].amounts, vec![Some(cents(900))]);
}

/// A template takes at most 90 owners.
#[test]
fn templates_take_at_most_90_owners() {
    let owners = |n: usize| (0..n).map(|i| format!("P{i}")).collect::<Vec<_>>();
    assert!(chart_template(EntityType::Partnership, &owners(90)).is_ok());
    assert!(chart_template(EntityType::Partnership, &owners(91)).is_err());
}

// ---------------------------------------------------------------------
// XLSX layout and styles (report_to_xlsx INVARIANTS 4-6), read from the
// workbook's own XML: each cell's style index, the style table
// (`cellXfs`: border and alignment), merged ranges, and column widths.
// (Added after the import mutation run found layout untested.)
// ---------------------------------------------------------------------

#[derive(Clone, Debug, Default, PartialEq)]
struct Style {
    top_border: bool,
    halign: Option<String>,
}

fn xml_section<'a>(xml: &'a str, tag: &str) -> &'a str {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let Some(i) = xml.find(&open) else { return "" };
    let j = xml[i..].find(&close).map_or(xml.len(), |j| i + j);
    &xml[i..j]
}

fn attr_of(tag: &str, name: &str) -> Option<String> {
    let k = format!(" {name}=\"");
    tag.find(&k)
        .map(|p| tag[p + k.len()..].split('"').next().unwrap().to_string())
}

/// Excel's "B12" to 1-based (row, 0-based col).
fn cell_ref(r: &str) -> (u32, u32) {
    let letters: String = r.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
    let row: u32 = r[letters.len()..].parse().unwrap();
    let col = letters
        .chars()
        .fold(0u32, |a, c| a * 26 + (c as u32 - 'A' as u32 + 1))
        - 1;
    (row, col)
}

/// Every written cell's style, blank cells included.
fn cell_styles(xlsx: &[u8]) -> BTreeMap<(u32, u32), Style> {
    let files = unzip(xlsx);
    let styles = String::from_utf8(files["xl/styles.xml"].clone()).unwrap();
    let borders: Vec<bool> = xml_section(&styles, "borders")
        .split("<border")
        .skip(1)
        .filter(|b| b.starts_with('>') || b.starts_with(' '))
        .map(|b| {
            let end = b.find("</border>").unwrap_or(b.len());
            b[..end].contains("<top style=")
        })
        .collect();
    let xfs: Vec<Style> = xml_section(&styles, "cellXfs")
        .split("<xf ")
        .skip(1)
        .map(|x| {
            let tag = format!(" {}", &x[..x.find('>').unwrap()]);
            let border = attr_of(&tag, "borderId").map_or(0, |b| b.parse().unwrap());
            Style {
                top_border: borders[border],
                halign: x
                    .find("<alignment")
                    .and_then(|p| attr_of(&x[p..p + x[p..].find('>').unwrap()], "horizontal")),
            }
        })
        .collect();
    let sheet = String::from_utf8(files["xl/worksheets/sheet1.xml"].clone()).unwrap();
    let mut out = BTreeMap::new();
    for c in sheet.split("<c ").skip(1) {
        let tag = format!(" {}", &c[..c.find('>').unwrap()]);
        let s = attr_of(&tag, "s").map_or(0, |s| s.parse().unwrap());
        out.insert(cell_ref(&attr_of(&tag, "r").unwrap()), xfs[s].clone());
    }
    out
}

/// Merged ranges as ((row, col), (row, col)), 1-based rows.
fn merges(xlsx: &[u8]) -> Vec<((u32, u32), (u32, u32))> {
    let files = unzip(xlsx);
    let sheet = String::from_utf8(files["xl/worksheets/sheet1.xml"].clone()).unwrap();
    sheet
        .split("<mergeCell ")
        .skip(1)
        .map(|m| {
            let r = attr_of(&format!(" {m}"), "ref").unwrap();
            let (a, b) = r.split_once(':').unwrap();
            (cell_ref(a), cell_ref(b))
        })
        .collect()
}

/// Column widths by 0-based column.
fn col_widths(xlsx: &[u8]) -> BTreeMap<u32, f64> {
    let files = unzip(xlsx);
    let sheet = String::from_utf8(files["xl/worksheets/sheet1.xml"].clone()).unwrap();
    let mut out = BTreeMap::new();
    for c in sheet.split("<col ").skip(1) {
        let tag = format!(" {}", &c[..c.find('>').unwrap()]);
        let (lo, hi): (u32, u32) = (
            attr_of(&tag, "min").unwrap().parse().unwrap(),
            attr_of(&tag, "max").unwrap().parse().unwrap(),
        );
        let w: f64 = attr_of(&tag, "width").unwrap().parse().unwrap();
        for col in lo..=hi {
            out.insert(col - 1, w);
        }
    }
    out
}

fn rows_used(cells: &BTreeMap<(u32, u32), Cell>) -> BTreeSet<u32> {
    cells.keys().map(|(r, _)| *r).collect()
}

/// The footer is one empty row below the last other content (INVARIANT 4).
fn assert_footer_one_row_below(cells: &BTreeMap<(u32, u32), Cell>, what: &str) {
    let footer = meta().footer();
    let at = row_of(cells, &footer, 0).unwrap_or_else(|| panic!("{what}: no footer"));
    let last = cells
        .iter()
        .filter(|(_, c)| **c != Cell::Text(footer.clone()))
        .map(|((r, _), _)| *r)
        .max()
        .unwrap();
    assert_eq!(
        at,
        last + 2,
        "{what}: footer at row {at}, content ends {last}"
    );
}

/// INVARIANTS 4-5 on a statement with every kind of row: header block,
/// right-aligned headings, content from row 6, a spacer as one empty row,
/// rules exactly where rule_above says (blank amount cells too), the
/// footer one row down, and the label and amount columns sized.
#[test]
fn xlsx_statement_layout_and_rules() {
    let row = |kind, label: &str, amounts: Vec<Option<Decimal>>| Row {
        kind,
        label: label.into(),
        amounts,
        currency: false,
        rule_above: false,
    };
    let st = report::statement(
        "Layout",
        "July 2025".into(),
        &["Book", "Tax"],
        vec![
            row(RowKind::Section, "Income", vec![]),
            row(
                RowKind::Line,
                "Sales",
                vec![Some(cents(100)), Some(cents(100))],
            ),
            row(
                RowKind::Subtotal,
                "Total income",
                vec![Some(cents(100)), None],
            ),
            row(RowKind::Spacer, "", vec![]),
            row(RowKind::Section, "Empty", vec![]),
            row(RowKind::Subtotal, "Total empty", vec![Some(cents(0)), None]),
            row(RowKind::Spacer, "", vec![]),
            row(RowKind::Total, "Net", vec![Some(cents(100)), None]),
        ],
    );
    let rules: HashMap<&str, bool> = st
        .rows
        .iter()
        .map(|r| (r.label.as_str(), r.rule_above))
        .collect();
    assert!(rules["Total income"] && !rules["Total empty"] && rules["Net"]);
    let bytes = report_to_xlsx(&XlsxReport::Statement(&st), &meta()).unwrap();
    let cells = sheet_cells(&bytes);
    let styles = cell_styles(&bytes);
    // Header block over all three columns (A..C), centered.
    let m = meta();
    for (r, want) in [(1, m.company.as_str()), (2, "Layout"), (3, "July 2025")] {
        assert_eq!(text_at(&cells, r, 0), Some(want));
        assert!(merges(&bytes).contains(&((r, 0), (r, 2))), "row {r} merge");
        assert_eq!(styles[&(r, 0)].halign.as_deref(), Some("center"));
    }
    assert!(!rows_used(&cells).contains(&4));
    assert_eq!(text_at(&cells, 5, 1), Some("BOOK"));
    assert_eq!(text_at(&cells, 5, 2), Some("TAX"));
    for c in [1, 2] {
        assert_eq!(styles[&(5, c)].halign.as_deref(), Some("right"));
    }
    // Content from row 6, a spacer is one empty row.
    let at = |label: &str| row_of(&cells, label, 0).unwrap();
    assert_eq!(at("Income"), 6);
    assert_eq!(at("Sales"), 7);
    assert_eq!(at("Total income"), 8);
    assert_eq!(at("Empty"), 10);
    assert_eq!(at("Total empty"), 11);
    assert_eq!(at("Net"), 13);
    assert!(!rows_used(&cells).contains(&9) && !rows_used(&cells).contains(&12));
    // Rules: exactly on rule_above rows, across every amount column.
    for r in &st.rows {
        if r.label.is_empty() {
            continue;
        }
        let rr = at(&r.label);
        for c in [1, 2] {
            let ruled = styles.get(&(rr, c)).is_some_and(|s| s.top_border);
            assert_eq!(ruled, r.rule_above, "{:?} column {c}", r.label);
        }
    }
    assert_footer_one_row_below(&cells, "statement");
    let w = col_widths(&bytes);
    assert!(w[&0] > w[&1], "the label column is the wide one: {w:?}");
    assert!(w.contains_key(&1) && w.contains_key(&2), "{w:?}");
}

/// INVARIANT 5's blank cells: a missing amount on a ruled row (a subtotal
/// under amounts, a total) is a blank cell in that row's ruled amount
/// format; on any other row (a line, a subtotal with nothing above it)
/// it writes no cell at all.
#[test]
fn xlsx_missing_amounts_are_cells_only_on_ruled_rows() {
    let row = |kind, label: &str, amounts: Vec<Option<Decimal>>| Row {
        kind,
        label: label.into(),
        amounts,
        currency: false,
        rule_above: false,
    };
    let st = report::statement(
        "Blanks",
        "July 2025".into(),
        &["Book", "Tax"],
        vec![
            row(RowKind::Section, "Income", vec![]),
            row(RowKind::Line, "Sales", vec![Some(cents(100)), None]),
            row(RowKind::Line, "Fees", vec![Some(cents(-5)), None]),
            row(
                RowKind::Subtotal,
                "Total income",
                vec![Some(cents(95)), None],
            ),
            row(RowKind::Spacer, "", vec![]),
            row(RowKind::Section, "Empty", vec![]),
            row(RowKind::Subtotal, "Total empty", vec![Some(cents(0)), None]),
            row(RowKind::Spacer, "", vec![]),
            row(RowKind::Total, "Net", vec![Some(cents(95)), None]),
        ],
    );
    let ruled: Vec<&str> = st
        .rows
        .iter()
        .filter(|r| r.rule_above)
        .map(|r| r.label.as_str())
        .collect();
    assert_eq!(ruled, ["Total income", "Net"]);
    let bytes = report_to_xlsx(&XlsxReport::Statement(&st), &meta()).unwrap();
    let cells = sheet_cells(&bytes);
    let styles = cell_styles(&bytes);
    for r in st.rows.iter().filter(|r| !r.amounts.is_empty()) {
        let at = row_of(&cells, &r.label, 0).unwrap();
        assert!(cells.contains_key(&(at, 1)), "{:?}: its amount", r.label);
        assert!(!cells.contains_key(&(at, 2)), "{:?}: no value", r.label);
        if r.rule_above {
            let blank = styles.get(&(at, 2));
            assert!(blank.is_some_and(|s| s.top_border), "{:?}", r.label);
            assert_eq!(blank, styles.get(&(at, 1)), "{:?}: as its amount", r.label);
        } else {
            assert!(!styles.contains_key(&(at, 2)), "{:?}: no cell", r.label);
        }
    }
}

/// INVARIANT 4 on the general ledger and journal: amount headings
/// right-aligned and the others not, accounts one empty row apart, and the
/// footer one row below (also with nothing to show).
#[test]
fn xlsx_ledger_layouts() {
    let specs: Vec<EntrySpec> = (0..6)
        .map(|i| EntrySpec {
            day: 190 + i * 3,
            accts: vec![(i as usize) % 3, 3],
            k: 1,
            debits: vec![1_000 + i, 0, 0],
            seeds: vec![0; 3],
            memo: format!("e{i}"),
        })
        .collect();
    let b = build_books(&specs, false);
    let (s, e) = (d(2026, 1, 1), d(2026, 12, 31));
    let gl = general_ledger(&b.ledger, s, e).unwrap();
    assert!(gl.accounts.len() >= 2);
    let empty_gl = general_ledger(&b.ledger, d(2020, 1, 1), d(2020, 1, 31)).unwrap();
    for (rep, right_from, what) in [
        (XlsxReport::GeneralLedger(&gl), 4, "general ledger"),
        (
            XlsxReport::GeneralJournal(&b.ledger, s, e),
            5,
            "general journal",
        ),
        (
            XlsxReport::GeneralLedger(&empty_gl),
            4,
            "empty general ledger",
        ),
    ] {
        let bytes = report_to_xlsx(&rep, &meta()).unwrap();
        let cells = sheet_cells(&bytes);
        let styles = cell_styles(&bytes);
        assert_eq!(text_at(&cells, 5, 0), Some("DATE"), "{what}");
        for c in 0..7 {
            let right = styles[&(5, c)].halign.as_deref() == Some("right");
            assert_eq!(right, c >= right_from, "{what}: heading column {c}");
        }
        assert_footer_one_row_below(&cells, what);
    }
    let cells = sheet_cells(&report_to_xlsx(&XlsxReport::GeneralLedger(&gl), &meta()).unwrap());
    let first = &gl.accounts[0];
    assert_eq!(
        text_at(&cells, 6, 0),
        Some(format!("{} {}", first.code, first.name).as_str()),
        "the first account right under the headings"
    );
    for pair in gl.accounts.windows(2) {
        let name = |a: &ledger_core::reports::LedgerAccount| format!("{} {}", a.code, a.name);
        let total = row_of(&cells, &format!("Total for {}", name(&pair[0])), 0).unwrap();
        assert_eq!(
            text_at(&cells, total + 2, 0),
            Some(name(&pair[1]).as_str()),
            "the next account one empty row down"
        );
        assert!(!rows_used(&cells).contains(&(total + 1)));
    }
}

/// INVARIANT 6: exact text groups thousands in threes, keeps every
/// decimal, and puts a negative in parentheses.
#[test]
fn xlsx_exact_text_groups_thousands() {
    let cases = [
        ("0.001", "0.001"),
        ("999.999", "999.999"),
        ("1000.001", "1,000.001"),
        ("123456.001", "123,456.001"),
        ("-1000.005", "(1,000.005)"),
        ("1234567.125", "1,234,567.125"),
        ("12345678901234.56", "12,345,678,901,234.56"),
        ("-123456789012345.67", "(123,456,789,012,345.67)"),
    ];
    let st = report::statement(
        "Text",
        "x".into(),
        &["Amount"],
        cases
            .iter()
            .map(|(a, _)| Row {
                kind: RowKind::Line,
                label: format!("amount {a}"),
                amounts: vec![Some(dec(a))],
                currency: false,
                rule_above: false,
            })
            .collect(),
    );
    let cells = render(&st);
    for (a, want) in cases {
        let r = row_of(&cells, &format!("amount {a}"), 0).unwrap();
        assert_eq!(text_at(&cells, r, 1), Some(want), "{a}");
    }
}

/// bank_csv INVARIANT 3: a date whose day equals its month reads the same
/// either way, so a file of only those needs no order.
#[test]
fn bank_csv_day_equals_month_needs_no_order() {
    let txns = parse_bank_csv("03/03/2026,A,1.00\n12/12/2026,B,-2.00\n").unwrap();
    assert_eq!(txns[0].date, d(2026, 3, 3));
    assert_eq!(txns[1].date, d(2026, 12, 12));
}

/// iif INVARIANT 4, exactly: a tab or line break is a space, and what
/// isn't ASCII is transliterated (deunicode 1.6.0's table): the en dash
/// and the minus sign '-', the em dash "--", curly quotes straight and
/// single, "é" "e", a no-break space a space. (Before plan D-36 the em
/// dash was one '-' and the "é" '?'.)
#[test]
fn iif_field_transliteration_exact() {
    let mut l = Ledger::new(Currency::USD);
    let cash = l
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    let rev = l
        .add_account("4000", "Revenue", AccountType::Revenue, None, false)
        .unwrap();
    let e = l.create_entry(
        d(2026, 3, 9),
        "a\tb\nc\r\nd \u{2013}\u{2014}\u{2212} \u{201C}\u{e9}\u{201D} \u{2018}\u{2019} x\u{a0}y",
    );
    l.add_line(e, cash, Side::Debit, usd(1)).unwrap();
    l.add_line(e, rev, Side::Credit, usd(1)).unwrap();
    l.post_entry(e).unwrap();
    let accounts = iif_accounts(&l, |_| QbAccountType::Bank);
    let entries: Vec<&JournalEntry> = l.posted_entries().collect();
    let iif = journal_to_iif(&accounts, &entries).unwrap();
    assert_eq!(iif_memos(&iif.text), ["a b c  d ---- 'e' '' x y"]);
}

/// The tax summary shows its cost-of-goods and not-yet-categorized
/// sections when it has those, and omits them when it doesn't.
#[test]
fn tax_summary_optional_sections() {
    use ledger_core::tax::{TaxForm, tax_summary};
    let build = |with: bool| {
        let mut l = Ledger::new(Currency::USD);
        let cash = l
            .add_account("1000", "Cash", AccountType::Asset, None, false)
            .unwrap();
        let sales = l
            .add_account("4000", "Sales", AccountType::Revenue, None, false)
            .unwrap();
        l.set_account_tax(sales, Some("gross_receipts"), None)
            .unwrap();
        let mut pairs = vec![(cash, sales)];
        if with {
            let cogs = l
                .add_account("5000", "Inventory sold", AccountType::Expense, None, false)
                .unwrap();
            l.set_account_tax(cogs, Some("cost_of_goods_sold"), None)
                .unwrap();
            let odd = l
                .add_account("6900", "Unsorted", AccountType::Expense, None, false)
                .unwrap();
            pairs.extend([(cogs, cash), (odd, cash)]);
        }
        for (dr, cr) in pairs {
            let e = l.create_entry(d(2026, 2, 1), "x");
            l.add_line(e, dr, Side::Debit, usd(1_000)).unwrap();
            l.add_line(e, cr, Side::Credit, usd(1_000)).unwrap();
            l.post_entry(e).unwrap();
        }
        let ts = tax_summary(&l, d(2026, 1, 1), d(2026, 12, 31), Some(TaxForm::Form1065)).unwrap();
        report::tax_summary(&ts)
    };
    let has = |st: &Statement, label: &str| st.rows.iter().any(|r| r.label == label);
    let full = build(true);
    assert!(has(&full, "Cost of goods sold"));
    assert!(has(&full, "Not yet categorized"));
    let bare = build(false);
    assert!(!has(&bare, "Cost of goods sold"));
    assert!(!has(&bare, "Not yet categorized"));
}

// ---------------------------------------------------------------------------
// Fixed asset register (ledger_import::asset_register)
// ---------------------------------------------------------------------------

/// A register as ledger_core::fixed_asset_register would return it: two
/// assets held, one disposed (shown, in no total), and books that differ
/// from the register on all three counts. Tax figures: the laptop a
/// member contributed from personal use (worth less than their basis:
/// depreciable basis is its value, a built-in loss), the desk with none
/// recorded, the monitor a carryover recorded before its value was asked
/// (read as its cost) with a built-in gain (disposed, in no total).
/// Hand-worked totals of the held: cost 2,500.00 + 1,200.00 = 3,700.00;
/// scheduled 527.78 + 250.00 = 777.78; posted 500.00 + 250.00 = 750.00;
/// book value 3,700.00 − 750.00 = 2,950.00; §723 basis 2,800.00;
/// depreciable basis 2,500.00. The books: cost 4,000.00 (300.00
/// unregistered), accumulated 700.00 (77.78 not yet posted, −50.00 not
/// from runs).
fn asset_register_fixture() -> ledger_core::AssetRegister {
    use ledger_core::AssetRegisterRow;
    let row =
        |id, tag: Option<&str>, desc: &str, serial: Option<&str>, cost, sched, posted, disposed| {
            AssetRegisterRow {
                asset_id: id,
                tag: tag.map(Into::into),
                description: desc.into(),
                serial: serial.map(Into::into),
                acquired_date: d(2026, 1, 10),
                in_service_date: d(2026, 1, 15),
                cost: cents(cost),
                salvage: cents(10000),
                useful_life_months: 36,
                accumulated_scheduled: cents(sched),
                accumulated_posted: cents(posted),
                book_value: cents(cost - posted),
                disposed,
                basis_rule: None,
                contributor_adjusted_basis: None,
                fmv_at_contribution: None,
                fmv_recorded: false,
                tax_basis_723: None,
                depreciable_basis: None,
                built_in_gain_704c: None,
                sec704c_1c: false,
                sec704c_method: None,
                basis_source: None,
            }
        };
    let taxed = |mut r: AssetRegisterRow, rule, basis: Decimal, fmv: Option<Decimal>| {
        let v = fmv.unwrap_or(r.cost);
        let f = ledger_core::kernel::basis_figures(rule, r.cost, basis, v).unwrap();
        r.basis_rule = Some(rule);
        r.contributor_adjusted_basis = Some(basis);
        r.fmv_at_contribution = Some(v);
        r.fmv_recorded = fmv.is_some();
        r.tax_basis_723 = f.tax_basis_723;
        r.depreciable_basis = Some(f.depreciable_basis);
        r.built_in_gain_704c = f.built_in_gain_704c;
        r.sec704c_1c = f.built_in_gain_704c.is_some_and(|g| g < Decimal::ZERO);
        r
    };
    let mut laptop = taxed(
        row(
            1,
            Some("FL-0001"),
            "Framework Laptop 13",
            Some("FRAN123"),
            250000,
            52778,
            50000,
            None,
        ),
        ledger_core::BasisRule::PersonalUseConversion,
        cents(280000),
        Some(cents(250000)),
    );
    laptop.sec704c_method = Some(ledger_core::Sec704cMethod::Traditional);
    laptop.basis_source = Some(ledger_core::settings::Source {
        decided_by: "Tobin Harlow".into(),
        decided_on: d(2026, 10, 20),
        citation: Some("entry 4".into()),
        note: None,
    });
    ledger_core::AssetRegister {
        as_of: d(2026, 10, 15),
        rows: vec![
            laptop,
            row(2, None, "Desk", None, 120000, 25000, 25000, None),
            {
                let mut r = row(
                    3,
                    Some("FL-0003"),
                    "Old monitor",
                    None,
                    40000,
                    0,
                    0,
                    Some((d(2026, 9, 30), cents(4500))),
                );
                r.book_value = Decimal::ZERO;
                taxed(r, ledger_core::BasisRule::Carryover723, cents(30000), None)
            },
        ],
        total_cost: cents(370000),
        total_accumulated_scheduled: cents(77778),
        total_accumulated_posted: cents(75000),
        total_book_value: cents(295000),
        total_depreciable_basis: cents(250000),
        total_tax_basis_723: cents(280000),
        missing_basis: 1,
        ledger_cost: cents(400000),
        ledger_accumulated: cents(70000),
    }
}

fn number_cell(cells: &BTreeMap<(u32, u32), Cell>, at: (u32, u32)) -> Decimal {
    match cells.get(&at) {
        Some(Cell::Num(v)) => parse_num(v),
        other => panic!("{at:?}: expected a number, got {other:?}"),
    }
}

fn text_cell(cells: &BTreeMap<(u32, u32), Cell>, at: (u32, u32)) -> String {
    match cells.get(&at) {
        Some(Cell::Text(t)) => t.clone(),
        other => panic!("{at:?}: expected text, got {other:?}"),
    }
}

/// asset_register INVARIANTS 1-3, 5 and 7-9, read back from the
/// workbook's XML: header, headings, one row per asset in order (disposed
/// marked) with its acquired and in-service dates, depreciation scheduled
/// and posted, its tax figures, method and source; the register's own
/// totals, and the reconciliation's three differences.
#[test]
fn asset_register_xlsx_cells() {
    let r = asset_register_fixture();
    let x = ledger_import::asset_register_xlsx(&r, &meta()).unwrap();
    let c = sheet_cells(&x);
    assert_eq!(text_cell(&c, (1, 0)), "Spec Test LLC");
    assert_eq!(text_cell(&c, (2, 0)), "Fixed Asset Register");
    assert_eq!(text_cell(&c, (3, 0)), "As of October 15, 2026");
    let heads: Vec<String> = (0..21).map(|i| text_cell(&c, (5, i))).collect();
    assert_eq!(
        heads,
        [
            "TAG",
            "DESCRIPTION",
            "SERIAL",
            "ACQUIRED",
            "IN SERVICE",
            "COST",
            "SALVAGE",
            "LIFE (MONTHS)",
            "DEPRECIATION SCHEDULED",
            "DEPRECIATION POSTED",
            "BOOK VALUE",
            "BASIS RULE",
            "VALUE AT CONTRIBUTION",
            "CONTRIBUTOR'S ADJUSTED BASIS",
            "§723 BASIS",
            "DEPRECIABLE BASIS",
            "BUILT-IN GAIN (LOSS)",
            "ALLOCATION METHOD",
            "BASIS FACTS FROM",
            "VALUE RECORDED",
            "STATUS"
        ]
    );
    assert_eq!(text_cell(&c, (6, 0)), "FL-0001");
    assert_eq!(text_cell(&c, (6, 1)), "Framework Laptop 13");
    assert_eq!(text_cell(&c, (6, 2)), "FRAN123");
    assert_eq!(text_cell(&c, (6, 3)), "2026-01-10");
    assert_eq!(text_cell(&c, (6, 4)), "2026-01-15");
    assert_eq!(number_cell(&c, (6, 5)), dec("2500.00"));
    assert_eq!(number_cell(&c, (6, 6)), dec("100.00"));
    assert_eq!(number_cell(&c, (6, 7)), dec("36"));
    assert_eq!(number_cell(&c, (6, 8)), dec("527.78"));
    assert_eq!(number_cell(&c, (6, 9)), dec("500.00"));
    assert_eq!(number_cell(&c, (6, 10)), dec("2000.00"));
    // Tax figures (INVARIANT 7): rule in words with its citation, value,
    // adjusted basis, §723 basis, depreciable basis (the lesser), the
    // built-in loss as a negative, the method, the source.
    assert_eq!(
        text_cell(&c, (6, 11)),
        "Personal-use conversion (Treas. Reg. §1.168(i)-4(b))"
    );
    assert_eq!(number_cell(&c, (6, 12)), dec("2500.00"));
    assert_eq!(number_cell(&c, (6, 13)), dec("2800.00"));
    assert_eq!(number_cell(&c, (6, 14)), dec("2800.00"));
    assert_eq!(number_cell(&c, (6, 15)), dec("2500.00"));
    assert_eq!(number_cell(&c, (6, 16)), dec("-300.00"));
    assert_eq!(
        text_cell(&c, (6, 17)),
        "Traditional (Treas. Reg. §1.704-3(b))"
    );
    assert_eq!(text_cell(&c, (6, 18)), "Tobin Harlow, 2026-10-20: entry 4");
    assert_eq!(text_cell(&c, (6, 19)), "Yes");
    assert_eq!(text_cell(&c, (6, 20)), "Active");
    // The desk: no tag, no serial, no tax basis recorded.
    assert_eq!(text_cell(&c, (7, 1)), "Desk");
    assert_eq!(number_cell(&c, (7, 5)), dec("1200.00"));
    assert_eq!(number_cell(&c, (7, 8)), dec("250.00"));
    assert_eq!(number_cell(&c, (7, 9)), dec("250.00"));
    assert_eq!(number_cell(&c, (7, 10)), dec("950.00"));
    assert_eq!(text_cell(&c, (7, 11)), "Not recorded");
    for col in 12..=19 {
        assert!(
            !c.contains_key(&(7, col)),
            "desk has nothing in tax column {col}"
        );
    }
    assert_eq!(text_cell(&c, (7, 20)), "Active");
    assert_eq!(text_cell(&c, (8, 0)), "FL-0003");
    assert_eq!(text_cell(&c, (8, 11)), "Carryover (IRC §723)");
    assert_eq!(number_cell(&c, (8, 12)), dec("400.00"));
    assert_eq!(number_cell(&c, (8, 13)), dec("300.00"));
    assert_eq!(number_cell(&c, (8, 14)), dec("300.00"));
    assert_eq!(number_cell(&c, (8, 15)), dec("300.00"));
    assert_eq!(number_cell(&c, (8, 16)), dec("100.00"));
    assert_eq!(text_cell(&c, (8, 19)), "No: read as the book cost");
    assert_eq!(
        text_cell(&c, (8, 20)),
        "Disposed 2026-09-30, proceeds 45.00"
    );
    assert_eq!(number_cell(&c, (8, 10)), Decimal::ZERO);
    // Totals: the register's (held assets only), not a re-sum of the rows.
    assert_eq!(text_cell(&c, (9, 0)), "Total, assets held");
    assert_eq!(number_cell(&c, (9, 5)), dec("3700.00"));
    assert_eq!(number_cell(&c, (9, 8)), dec("777.78"));
    assert_eq!(number_cell(&c, (9, 9)), dec("750.00"));
    assert_eq!(number_cell(&c, (9, 10)), dec("2950.00"));
    assert_eq!(number_cell(&c, (9, 14)), dec("2800.00"));
    assert_eq!(number_cell(&c, (9, 15)), dec("2500.00"));
    for col in [6, 7, 12, 13, 16] {
        assert!(!c.contains_key(&(9, col)), "no total in {col}");
    }
    // INVARIANT 8: the missing tax basis line.
    assert_eq!(text_cell(&c, (10, 0)), "Tax basis not recorded for 1 asset");
    // Reconciliation.
    assert_eq!(text_cell(&c, (12, 0)), "Reconciliation to the books");
    let recon = [
        (13, "Cost per register", 5, "3700.00"),
        (14, "Cost per books", 5, "4000.00"),
        (15, "Difference (unregistered cost)", 5, "300.00"),
        (17, "Accumulated depreciation per schedule", 8, "777.78"),
        (18, "Accumulated depreciation per books", 8, "700.00"),
        (19, "Difference (not yet posted)", 8, "77.78"),
        (
            21,
            "Accumulated depreciation from the assets' runs",
            8,
            "750.00",
        ),
        (22, "Accumulated depreciation per books", 8, "700.00"),
        (23, "Difference (not from runs)", 8, "-50.00"),
    ];
    for (row, label, col, amt) in recon {
        assert_eq!(text_cell(&c, (row, 0)), label);
        assert_eq!(number_cell(&c, (row, col)), dec(amt), "{label}");
    }
    // INVARIANT 9: the notes, numbered as the PDF marks them.
    assert_eq!(text_cell(&c, (25, 0)), "Notes");
    let notes = ledger_import::asset_register::notes();
    for (i, n) in notes.iter().enumerate() {
        let want = if n.mark.is_empty() {
            n.text.clone()
        } else {
            format!("{}. {}", n.mark, n.text)
        };
        assert_eq!(text_cell(&c, (26 + i as u32, 0)), want);
    }
    assert!(
        text_cell(&c, (26, 0))
            .starts_with("Book value is cost less the depreciation the books carry")
    );
    for (row, start) in [
        (27, "1. Purchase (IRC §1012): "),
        (28, "2. Carryover (IRC §723): "),
        (
            29,
            "3. Personal-use conversion (Treas. Reg. §1.168(i)-4(b)): ",
        ),
        (30, "4. Built-in gain (loss), IRC §704(c): "),
        (31, "*. Value at contribution not recorded"),
    ] {
        assert!(text_cell(&c, (row, 0)).starts_with(start), "note {start:?}");
    }
    let footer = c
        .values()
        .any(|v| matches!(v, Cell::Text(t) if t.contains("Journal seq 42")));
    assert!(footer, "footer present");
}

/// asset_register INVARIANTS 2 and 5 as laid out (crate::xlsx INVARIANTS
/// 4-5): the headings over the amounts and over the life in months
/// right-aligned and the others not, and the totals row ruled across
/// every column after its label in the totals format, blank cells and
/// totals alike; the asset rows above it aren't ruled.
#[test]
fn asset_register_xlsx_headings_and_totals_rule() {
    let x = ledger_import::asset_register_xlsx(&asset_register_fixture(), &meta()).unwrap();
    let c = sheet_cells(&x);
    let styles = cell_styles(&x);
    let right_aligned = [5, 6, 7, 8, 9, 10, 12, 13, 14, 15, 16];
    for col in 0..21 {
        let right = styles[&(5, col)].halign.as_deref() == Some("right");
        assert_eq!(right, right_aligned.contains(&col), "heading column {col}");
    }
    assert_eq!(text_cell(&c, (9, 0)), "Total, assets held");
    assert!(!styles[&(9, 0)].top_border, "the label isn't ruled");
    let total = &styles[&(9, 5)];
    assert!(total.top_border, "{total:?}");
    for col in 1..21 {
        assert_eq!(
            styles.get(&(9, col)),
            Some(total),
            "totals row column {col}"
        );
    }
    for row in 6..=8 {
        for col in 0..21 {
            let ruled = styles.get(&(row, col)).is_some_and(|s| s.top_border);
            assert!(!ruled, "asset row {row} column {col}");
        }
    }
}

/// asset_register INVARIANT 4: an empty register says so, still totals,
/// and still reconciles (the books may carry unregistered cost).
#[test]
fn asset_register_xlsx_empty() {
    let r = ledger_core::AssetRegister {
        as_of: d(2026, 10, 15),
        rows: vec![],
        total_cost: Decimal::ZERO,
        total_accumulated_scheduled: Decimal::ZERO,
        total_accumulated_posted: Decimal::ZERO,
        total_book_value: Decimal::ZERO,
        total_depreciable_basis: Decimal::ZERO,
        total_tax_basis_723: Decimal::ZERO,
        missing_basis: 0,
        ledger_cost: cents(99900),
        ledger_accumulated: Decimal::ZERO,
    };
    let c = sheet_cells(&ledger_import::asset_register_xlsx(&r, &meta()).unwrap());
    assert_eq!(number_cell(&c, (7, 14)), Decimal::ZERO);
    assert_eq!(number_cell(&c, (7, 15)), Decimal::ZERO);
    assert!(
        !c.values()
            .any(|v| matches!(v, Cell::Text(t) if t.starts_with("Tax basis not recorded"))),
        "nothing missing"
    );
    assert_eq!(text_cell(&c, (6, 0)), "No fixed assets");
    assert_eq!(text_cell(&c, (7, 0)), "Total, assets held");
    assert_eq!(number_cell(&c, (7, 5)), Decimal::ZERO);
    assert_eq!(text_cell(&c, (12, 0)), "Difference (unregistered cost)");
    assert_eq!(number_cell(&c, (12, 5)), dec("999.00"));
}
