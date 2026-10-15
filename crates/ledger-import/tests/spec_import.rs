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
    BankCsvError, BankTransaction, DateOrder, EntityType, IifAccount, IifError, QbAccountType,
    XlsxReport, chart_template, format_amount, iif_accounts, journal_to_iif, parse_bank_csv,
    parse_bank_csv_with, parse_chart_csv, report_to_xlsx, statement_pdf,
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
            RowKind::Subheading | RowKind::Spacer => {}
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
    assert_eq!(format_amount(dec("-207.68")), "(207.68)");
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

    /// cash_flow INVARIANTS 1-3: operating lines (net income, depreciation,
    /// working capital) foot to "Net cash from operating activities",
    /// investing and financing to theirs; the three sum to "Net increase
    /// (decrease) in cash"; beginning cash plus it is "Cash at end of
    /// period"; a member's capital/distributions line is "Member
    /// contributions — Name" (cash in) or "Member distributions — Name"
    /// (cash out).
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
        }).unwrap();
        let ms = members();
        let st = report::cash_flow(&cf, &ms);
        let checked = assert_sections_foot(&st);
        prop_assert_eq!(checked.len(), 3, "three sections: {:?}", checked);
        let op = find(&st, RowKind::Subtotal, "Net cash from operating activities").expect("operating subtotal");
        prop_assert!(checked.iter().any(|(_, t)| t == &op.label));
        let three: Decimal = checked.iter().map(|(_, t)| amt(st.rows.iter().find(|r| &r.label == t).unwrap(), 0)).sum();
        let net = st.rows.iter().find(|r| r.label == "Net increase (decrease) in cash").expect("net change row");
        prop_assert_eq!(amt(net, 0), three);
        let begin = st.rows.iter().find(|r| r.kind == RowKind::Line && r.label.to_lowercase().contains("beginning")).expect("beginning cash");
        let endrow = st.rows.iter().find(|r| r.label == "Cash at end of period").expect("ending cash");
        prop_assert_eq!(amt(begin, 0) + amt(net, 0), amt(endrow, 0));
        // Ties to the core numbers it lays out.
        prop_assert_eq!(amt(op, 0), cf.cash_from_operations);
        prop_assert_eq!(amt(begin, 0), cf.cash_beginning);
        prop_assert_eq!(amt(endrow, 0), cf.cash_ending);
        // INVARIANT 3: members named on their capital/distributions lines.
        let mut fin: Vec<(String, Decimal)> = section_lines(&st, "Financing").iter().map(|r| (r.label.clone(), amt(r, 0))).collect();
        for line in &cf.financing {
            let Some(code) = &line.account else { continue };
            let Some(m) = ms.iter().find(|m| &m.capital_code == code || m.distribution_code.as_ref() == Some(code)) else { continue };
            let want = if line.amount > Decimal::ZERO {
                format!("Member contributions — {}", m.name)
            } else {
                format!("Member distributions — {}", m.name)
            };
            let pos = fin.iter().position(|(l, a)| *l == want && *a == line.amount);
            prop_assert!(pos.is_some(), "no {:?} {} among {:?}", want, line.amount, fin);
            fin.remove(pos.unwrap());
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

/// Check an IIF document against iif.rs INVARIANTS 1-4 and the layout,
/// for `entries` written with `accounts`.
fn check_iif(text: &str, accounts: &[IifAccount], entries: &[&JournalEntry]) {
    // INVARIANT 4: ASCII only.
    assert!(text.is_ascii(), "non-ASCII in IIF");
    let lines: Vec<&str> = text
        .split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .filter(|l| !l.is_empty())
        .collect();
    for l in &lines {
        assert!(!l.contains('\r'), "stray CR inside a line: {l:?}");
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
                // INVARIANT 1: every account precedes the transactions.
                assert!(!seen_trns_header, "ACCNT after !TRNS");
                let name = f[h.iter().position(|c| *c == "NAME").unwrap()];
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
        assert_eq!(row[col("ACCNT", "ACCNUM")], a.number);
    }
    let path_of: HashMap<AccountId, &str> =
        accounts.iter().map(|a| (a.id, a.path.as_str())).collect();
    assert_eq!(txns.len(), entries.len(), "one transaction per entry");
    for (i, (t, e)) in txns.iter().zip(entries).enumerate() {
        let mut sum = Decimal::ZERO;
        let mut got: Vec<(String, Decimal)> = Vec::new();
        for (j, f) in t.iter().enumerate() {
            let kw = if j == 0 { "TRNS" } else { "SPL" };
            // INVARIANT 3: GENERAL JOURNAL, dated M/D/YYYY.
            assert_eq!(f[col(kw, "TRNSTYPE")], "GENERAL JOURNAL");
            let dt = e.date();
            assert_eq!(
                f[col(kw, "DATE")],
                format!("{}/{}/{}", dt.month(), dt.day(), dt.year())
            );
            assert_eq!(f[col(kw, "DOCNUM")], (i + 1).to_string(), "DOCNUM from 1");
            let acct = f[col(kw, "ACCNT")];
            // INVARIANT 1/5: every account used is declared.
            assert!(declared.contains(acct), "{acct:?} used but not declared");
            let a: Decimal = f[col(kw, "AMOUNT")].parse().unwrap();
            assert_eq!(a.scale(), 2, "two decimal places: {}", f[col(kw, "AMOUNT")]);
            sum += a;
            got.push((acct.to_string(), a));
        }
        // INVARIANT 3: debits positive, credits negative, sum zero.
        assert_eq!(
            sum,
            Decimal::ZERO,
            "transaction {} doesn't sum to zero",
            i + 1
        );
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

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// iif INVARIANTS 1-4 over random books: accounts declared before the
    /// transactions; colon paths; TRNS/SPL/ENDTRNS, GENERAL JOURNAL,
    /// M/D/YYYY, debits positive and credits negative summing to zero;
    /// ASCII only, no tab or line break inside a field.
    #[test]
    fn iif_round_trips_random_books((specs, oe, _) in books_strategy()) {
        let b = build_books(&specs, oe);
        let accounts = iif_accounts(&b.ledger, qb);
        prop_assert_eq!(accounts.len(), b.ledger.chart().len());
        let entries: Vec<&JournalEntry> = b.ledger.posted_entries().collect();
        let text = journal_to_iif(&accounts, &entries).unwrap();
        check_iif(&text, &accounts, &entries);
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
}

/// iif INVARIANT 2: "Reserve: Savings" under "Cash" is one level down, its
/// own colon replaced; INVARIANT 4: common punctuation is transliterated
/// (curly quotes, em dash), anything else becomes '?'.
#[test]
fn iif_paths_and_transliteration() {
    let b = build_books(&[], true);
    let accounts = iif_accounts(&b.ledger, qb);
    let path = |code: &str| {
        let id = b.ledger.chart().find_by_code(code).unwrap().id;
        accounts.iter().find(|a| a.id == id).unwrap().path.clone()
    };
    let reserve = path("1010");
    assert!(reserve.starts_with("Cash:"), "{reserve:?}");
    assert_eq!(reserve.matches(':').count(), 1, "{reserve:?}");
    assert!(reserve.ends_with("Savings"));
    for a in &accounts {
        assert!(a.path.is_ascii(), "{:?}", a.path);
    }
    // Curly quotes are common punctuation: transliterated, not '?'.
    assert_eq!(path("4100"), "Interest \"income\"");
    // The em dash too.
    let cap = path("3000");
    assert!(
        cap.starts_with("Capital ") && cap.ends_with(" Ada"),
        "{cap:?}"
    );
    assert!(!cap.contains('?'), "em dash became '?': {cap:?}");
    // A letter with no ASCII form is not punctuation: '?'.
    assert_eq!(path("6010"), "Caf? meals");
}

/// iif layout example (module docs): a two-line sale on 1/15/2026 writes
/// TRNS Cash 1000.00 then SPL Revenue -1000.00, DOCNUM 1, memo "Sale", each
/// line's tabs matching its header.
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
    let text = journal_to_iif(&accounts, &entries).unwrap();
    let lines: Vec<&str> = text.lines().map(|l| l.trim_end_matches('\r')).collect();
    assert_eq!(lines[0], "!ACCNT\tNAME\tACCNTTYPE\tACCNUM");
    assert!(lines.contains(&"ACCNT\tCash\tBANK\t1000"));
    assert!(lines.contains(&"ACCNT\tRevenue\tINC\t4000"));
    assert!(lines.contains(&"TRNS\t\tGENERAL JOURNAL\t1/15/2026\tCash\t\t1000.00\t1\tSale"));
    assert!(lines.contains(&"SPL\t\tGENERAL JOURNAL\t1/15/2026\tRevenue\t\t-1000.00\t1\tSale"));
    check_iif(&text, &accounts, &entries);
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
    let text = journal_to_iif(&accounts, &entries).unwrap();
    check_iif(&text, &accounts, &entries);
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
            line("plain", "-207.68"),
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
        },
    )
    .unwrap();
    let me = members_equity(&b.ledger, s, e, &member_capitals(&b)).unwrap();
    for st in [
        report::profit_and_loss(&pl),
        report::balance_sheet(&bs, ye, &members()),
        report::cash_flow(&cf, &members()),
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

/// The cash flow's "Adjustments" subheading and depreciation line appear
/// only when there is something to adjust.
#[test]
fn cash_flow_adjustments_appear_only_when_there_are_some() {
    let mk = |depreciation: Decimal, wc: Vec<ledger_core::CashFlowLine>| {
        ledger_core::CashFlowStatement {
            start: d(2026, 1, 1),
            end: d(2026, 1, 31),
            net_income: cents(1_000),
            depreciation,
            cash_from_operations: cents(1_000)
                + depreciation
                + wc.iter().map(|l| l.amount).sum::<Decimal>(),
            working_capital: wc,
            investing: vec![],
            cash_from_investing: Decimal::ZERO,
            financing: vec![],
            cash_from_financing: Decimal::ZERO,
            net_change_in_cash: Decimal::ZERO,
            cash_beginning: Decimal::ZERO,
            cash_ending: Decimal::ZERO,
        }
    };
    let has = |st: &Statement, p: &str| st.rows.iter().any(|r| r.label.starts_with(p));
    let plain = report::cash_flow(&mk(Decimal::ZERO, vec![]), &[]);
    assert!(!has(&plain, "Adjustments") && !has(&plain, "Depreciation"));
    let dep = report::cash_flow(&mk(cents(100), vec![]), &[]);
    assert!(has(&dep, "Adjustments") && has(&dep, "Depreciation"));
    let wc = report::cash_flow(
        &mk(
            Decimal::ZERO,
            vec![ledger_core::CashFlowLine {
                label: "Increase in AR".into(),
                amount: cents(-50),
                account: None,
            }],
        ),
        &[],
    );
    assert!(has(&wc, "Adjustments") && !has(&wc, "Depreciation"));
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

/// iif INVARIANT 4, exactly: a tab or line break becomes a space, dashes
/// '-', curly quotes straight, a no-break space a space, anything else '?'.
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
    let text = journal_to_iif(&accounts, &entries).unwrap();
    let trns = text.lines().find(|l| l.starts_with("TRNS\t")).unwrap();
    assert_eq!(trns.rsplit('\t').next(), Some("a b c  d --- \"?\" '' x y"));
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
/// from the register on both counts.
fn asset_register_fixture() -> ledger_core::AssetRegister {
    use ledger_core::AssetRegisterRow;
    let row = |id, tag: Option<&str>, desc: &str, serial: Option<&str>, cost, acc, disposed| {
        AssetRegisterRow {
            asset_id: id,
            tag: tag.map(Into::into),
            description: desc.into(),
            serial: serial.map(Into::into),
            in_service: d(2026, 1, 15),
            cost: cents(cost),
            salvage: cents(10000),
            useful_life_months: 36,
            accumulated: cents(acc),
            book_value: cents(cost - acc),
            disposed,
        }
    };
    ledger_core::AssetRegister {
        as_of: d(2026, 10, 15),
        rows: vec![
            row(
                1,
                Some("WL-0001"),
                "Framework Laptop 13",
                Some("FRAN123"),
                250000,
                52778,
                None,
            ),
            row(2, None, "Desk", None, 120000, 25000, None),
            {
                let mut r = row(
                    3,
                    Some("WL-0003"),
                    "Old monitor",
                    None,
                    40000,
                    0,
                    Some((d(2026, 9, 30), cents(4500))),
                );
                r.book_value = Decimal::ZERO;
                r
            },
        ],
        total_cost: cents(370000),
        total_accumulated: cents(77778),
        total_book_value: cents(292222),
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

/// asset_register INVARIANTS 1-3 and 5, read back from the workbook's XML:
/// header, headings, one row per asset in order (disposed marked), the
/// register's own totals, and the reconciliation under Cost and
/// Accumulated depreciation.
#[test]
fn asset_register_xlsx_cells() {
    let r = asset_register_fixture();
    let x = ledger_import::asset_register_xlsx(&r, &meta()).unwrap();
    let c = sheet_cells(&x);
    assert_eq!(text_cell(&c, (1, 0)), "Spec Test LLC");
    assert_eq!(text_cell(&c, (2, 0)), "Fixed Asset Register");
    assert_eq!(text_cell(&c, (3, 0)), "As of October 15, 2026");
    let heads: Vec<String> = (0..10).map(|i| text_cell(&c, (5, i))).collect();
    assert_eq!(
        heads,
        [
            "TAG",
            "DESCRIPTION",
            "SERIAL",
            "IN SERVICE",
            "COST",
            "SALVAGE",
            "LIFE (MONTHS)",
            "ACCUMULATED DEPRECIATION",
            "BOOK VALUE",
            "STATUS"
        ]
    );
    assert_eq!(text_cell(&c, (6, 0)), "WL-0001");
    assert_eq!(text_cell(&c, (6, 1)), "Framework Laptop 13");
    assert_eq!(text_cell(&c, (6, 2)), "FRAN123");
    assert_eq!(text_cell(&c, (6, 3)), "2026-01-15");
    assert_eq!(number_cell(&c, (6, 4)), dec("2500.00"));
    assert_eq!(number_cell(&c, (6, 5)), dec("100.00"));
    assert_eq!(number_cell(&c, (6, 6)), dec("36"));
    assert_eq!(number_cell(&c, (6, 7)), dec("527.78"));
    assert_eq!(number_cell(&c, (6, 8)), dec("1972.22"));
    assert_eq!(text_cell(&c, (6, 9)), "Active");
    assert_eq!(text_cell(&c, (7, 1)), "Desk");
    assert_eq!(text_cell(&c, (8, 0)), "WL-0003");
    assert_eq!(text_cell(&c, (8, 9)), "Disposed 2026-09-30, proceeds 45.00");
    assert_eq!(number_cell(&c, (8, 8)), Decimal::ZERO);
    // Totals: the register's (held assets only), not a re-sum of the rows.
    assert_eq!(text_cell(&c, (9, 0)), "Total, assets held");
    assert_eq!(number_cell(&c, (9, 4)), dec("3700.00"));
    assert_eq!(number_cell(&c, (9, 7)), dec("777.78"));
    assert_eq!(number_cell(&c, (9, 8)), dec("2922.22"));
    assert!(!c.contains_key(&(9, 5)), "no salvage total");
    // Reconciliation.
    assert_eq!(text_cell(&c, (11, 0)), "Reconciliation to the books");
    let recon = [
        (12, "Cost per register", 4, "3700.00"),
        (13, "Cost per books", 4, "4000.00"),
        (14, "Difference (unregistered cost)", 4, "300.00"),
        (16, "Accumulated depreciation per schedule", 7, "777.78"),
        (17, "Accumulated depreciation per books", 7, "700.00"),
        (18, "Difference (not yet posted)", 7, "77.78"),
    ];
    for (row, label, col, amt) in recon {
        assert_eq!(text_cell(&c, (row, 0)), label);
        assert_eq!(number_cell(&c, (row, col)), dec(amt), "{label}");
    }
    let footer = c
        .values()
        .any(|v| matches!(v, Cell::Text(t) if t.contains("Journal seq 42")));
    assert!(footer, "footer present");
}

/// asset_register INVARIANT 4: an empty register says so, still totals,
/// and still reconciles (the books may carry unregistered cost).
#[test]
fn asset_register_xlsx_empty() {
    let r = ledger_core::AssetRegister {
        as_of: d(2026, 10, 15),
        rows: vec![],
        total_cost: Decimal::ZERO,
        total_accumulated: Decimal::ZERO,
        total_book_value: Decimal::ZERO,
        ledger_cost: cents(99900),
        ledger_accumulated: Decimal::ZERO,
    };
    let c = sheet_cells(&ledger_import::asset_register_xlsx(&r, &meta()).unwrap());
    assert_eq!(text_cell(&c, (6, 0)), "No fixed assets");
    assert_eq!(text_cell(&c, (7, 0)), "Total, assets held");
    assert_eq!(number_cell(&c, (7, 4)), Decimal::ZERO);
    assert_eq!(text_cell(&c, (12, 0)), "Difference (unregistered cost)");
    assert_eq!(number_cell(&c, (12, 4)), dec("999.00"));
}
