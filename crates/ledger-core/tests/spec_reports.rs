//! Spec-derived tests for the reports (`reports.rs`).
//!
//! Written from the spec blocks (GOAL / SOURCE / INVARIANTS / SCENARIOS) and
//! `docs/spec/rules.md` / `docs/requirements.md` only, never from the
//! implementation. Every expected value is either a literal computed by hand
//! from the spec or a law (property) that pins the answer without calling the
//! function under test to compute it. Where the random-book tests need a
//! reference value they use an independent model: the plain sum of the
//! generated lines, which is what every report is defined over.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{Days, NaiveDate};
use ledger_core::reports::{profit_and_loss_by_class, snapshot, trial_balance, unclosed_earnings};
use ledger_core::{
    AccountId, AccountType, BalanceSheetStatement, CashFlowParams, CashFlowStatement, ClassId,
    CloseTo, Currency, EntryId, GeneralLedger, Ledger, MemberCapital, MemberClose,
    MemberEquityColumn, MembersEquityStatement, Money, NewAsset, ProfitAndLossStatement, Side,
    StatementLine, TrialBalance, balance_sheet_as_of, cash_flow, general_ledger, members_equity,
    profit_and_loss_for_period, trial_balance_as_of,
};
use proptest::prelude::*;
use rust_decimal::Decimal;
use serde_json::Value;

const USD: Currency = Currency::USD;
const DR: Side = Side::Debit;
const CR: Side = Side::Credit;

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn c(cents: i64) -> Decimal {
    Decimal::new(cents, 2)
}

fn usd(cents: i64) -> Money {
    Money::new(c(cents), USD)
}

fn post(l: &mut Ledger, date: NaiveDate, desc: &str, lines: &[(AccountId, Side, i64)]) -> EntryId {
    let e = l.create_entry(date, desc);
    for &(a, s, cents) in lines {
        l.add_line(e, a, s, usd(cents)).unwrap();
    }
    l.post_entry(e).unwrap();
    e
}

fn draft(l: &mut Ledger, date: NaiveDate, lines: &[(AccountId, Side, i64)]) -> EntryId {
    let e = l.create_entry(date, "draft");
    for &(a, s, cents) in lines {
        l.add_line(e, a, s, usd(cents)).unwrap();
    }
    e
}

fn tb_lines(tb: &TrialBalance) -> Vec<(String, Decimal, Decimal)> {
    tb.lines
        .iter()
        .map(|l| (l.code.clone(), l.debit, l.credit))
        .collect()
}

fn sl(lines: &[StatementLine]) -> Vec<(String, Decimal)> {
    lines.iter().map(|l| (l.code.clone(), l.amount)).collect()
}

fn sl_nonzero(lines: &[StatementLine]) -> Vec<(String, Decimal)> {
    lines
        .iter()
        .filter(|l| !l.amount.is_zero())
        .map(|l| (l.code.clone(), l.amount))
        .collect()
}

fn s(code: &str, cents: i64) -> (String, Decimal) {
    (code.to_string(), c(cents))
}

fn debit_normal(t: AccountType) -> bool {
    matches!(t, AccountType::Asset | AccountType::Expense)
}

type PnlKey = (Lines, Lines, Decimal, Decimal);

fn pnl_key(p: &ProfitAndLossStatement) -> PnlKey {
    (
        sl(&p.revenue),
        sl(&p.expenses),
        p.total_revenue,
        p.total_expenses,
    )
}

fn bs_key(b: &BalanceSheetStatement) -> String {
    format!(
        "{:?}|{:?}|{:?}|{}|{}|{}|{}",
        sl(&b.assets),
        sl(&b.liabilities),
        sl(&b.equity),
        b.current_earnings,
        b.total_assets,
        b.total_liabilities,
        b.total_equity
    )
}

fn gl_key(g: &GeneralLedger) -> String {
    format!("{:?}", g.accounts)
}

fn cf_key(cf: &CashFlowStatement) -> String {
    format!("{cf:?}")
}

// ---------------------------------------------------------------------------
// A fixed example company for the scenario tests.
// ---------------------------------------------------------------------------

struct Co {
    l: Ledger,
    cash: AccountId,
    ar: AccountId,
    equip: AccountId,
    accum: AccountId,
    ap: AccountId,
    loan: AccountId,
    cap_a: AccountId,
    cap_b: AccountId,
    other_eq: AccountId,
    rev: AccountId,
    rev2: AccountId,
    gain: AccountId,
    exp: AccountId,
    exp2: AccountId,
    depr: AccountId,
}

fn company() -> Co {
    use AccountType::*;
    let mut l = Ledger::new(USD);
    let header = l
        .add_account("6900", "Operating Expenses", Expense, None, true)
        .unwrap();
    let cash = l.add_account("1000", "Cash", Asset, None, false).unwrap();
    let ar = l
        .add_account("1100", "Accounts Receivable", Asset, None, false)
        .unwrap();
    let equip = l
        .add_account("1500", "Equipment", Asset, None, false)
        .unwrap();
    let accum = l
        .add_account("1510", "Accumulated Depreciation", Asset, None, false)
        .unwrap();
    let ap = l
        .add_account("2000", "Accounts Payable", Liability, None, false)
        .unwrap();
    let loan = l
        .add_account("2500", "Loan Payable", Liability, None, false)
        .unwrap();
    let cap_a = l
        .add_account("3000", "Capital A", Equity, None, false)
        .unwrap();
    let cap_b = l
        .add_account("3001", "Capital B", Equity, None, false)
        .unwrap();
    let other_eq = l
        .add_account("3500", "Other Equity", Equity, None, false)
        .unwrap();
    let rev = l
        .add_account("4000", "Sales", Revenue, None, false)
        .unwrap();
    let rev2 = l
        .add_account("4100", "Services", Revenue, None, false)
        .unwrap();
    let gain = l
        .add_account("4900", "Gain on Disposal", Revenue, None, false)
        .unwrap();
    let exp = l
        .add_account("6000", "Supplies", Expense, Some(header), false)
        .unwrap();
    let exp2 = l
        .add_account("6100", "Rent", Expense, Some(header), false)
        .unwrap();
    let depr = l
        .add_account("6200", "Depreciation Expense", Expense, None, false)
        .unwrap();
    Co {
        l,
        cash,
        ar,
        equip,
        accum,
        ap,
        loan,
        cap_a,
        cap_b,
        other_eq,
        rev,
        rev2,
        gain,
        exp,
        exp2,
        depr,
    }
}

fn members_50_50(co: &Co) -> Vec<MemberCapital> {
    vec![
        MemberCapital {
            name: "A".into(),
            capital_account: co.cap_a,
            distribution_account: None,
            share: c(50),
        },
        MemberCapital {
            name: "B".into(),
            capital_account: co.cap_b,
            distribution_account: None,
            share: c(50),
        },
    ]
}

fn col(name: &str, b: i64, ct: i64, ni: i64, di: i64, o: i64, e: i64) -> MemberEquityColumn {
    MemberEquityColumn {
        name: name.into(),
        beginning: c(b),
        contributions: c(ct),
        net_income: c(ni),
        distributions: c(di),
        other: c(o),
        ending: c(e),
    }
}

// ===========================================================================
// trial_balance / trial_balance_as_of — examples
// ===========================================================================

/// Oracle: TB SCENARIO 1 — no entries posted by `as_of` gives an empty TB
/// with zero totals (also when the only entries are dated after `as_of`).
#[test]
fn tb_as_of_with_nothing_posted_is_empty_with_zero_totals() {
    let mut co = company();
    let tb = trial_balance_as_of(&co.l, d(2026, 6, 30));
    assert!(tb.lines.is_empty());
    assert_eq!(tb.total_debits, Decimal::ZERO);
    assert_eq!(tb.total_credits, Decimal::ZERO);
    assert!(tb.balanced());

    let (cash, rev) = (co.cash, co.rev);
    post(
        &mut co.l,
        d(2026, 7, 1),
        "later",
        &[(cash, DR, 100), (rev, CR, 100)],
    );
    let tb = trial_balance_as_of(&co.l, d(2026, 6, 30));
    assert!(tb.lines.is_empty());
    assert_eq!(tb.total_debits, Decimal::ZERO);
    assert_eq!(tb.total_credits, Decimal::ZERO);
}

/// Oracle: TB SCENARIO 2 — an asset debited 100 and credited 30 shows debit
/// 70, credit 0 (hand-computed: 1000 Dr 70, 4000 Cr 100, 6000 Dr 30).
#[test]
fn tb_as_of_asset_debited_100_credited_30_shows_debit_70() {
    let mut co = company();
    let (cash, rev, exp) = (co.cash, co.rev, co.exp);
    post(
        &mut co.l,
        d(2026, 1, 5),
        "sale",
        &[(cash, DR, 10000), (rev, CR, 10000)],
    );
    post(
        &mut co.l,
        d(2026, 1, 10),
        "supplies",
        &[(exp, DR, 3000), (cash, CR, 3000)],
    );
    let tb = trial_balance_as_of(&co.l, d(2026, 1, 31));
    assert_eq!(
        tb_lines(&tb),
        vec![
            ("1000".to_string(), c(7000), c(0)),
            ("4000".to_string(), c(0), c(10000)),
            ("6000".to_string(), c(3000), c(0)),
        ]
    );
    assert_eq!(tb.total_debits, c(10000));
    assert_eq!(tb.total_credits, c(10000));
    assert!(tb.balanced());
}

/// Oracle: TB INVARIANT 2 — NEVER list a placeholder (header 6900 with
/// active children) or a zero balance (an account debited and credited
/// equally).
#[test]
fn tb_never_lists_placeholders_or_zero_balances() {
    let mut co = company();
    let (cash, rev, exp, exp2) = (co.cash, co.rev, co.exp, co.exp2);
    post(
        &mut co.l,
        d(2026, 2, 1),
        "sale",
        &[(cash, DR, 5000), (rev, CR, 5000)],
    );
    post(
        &mut co.l,
        d(2026, 2, 2),
        "refund",
        &[(rev, DR, 5000), (cash, CR, 5000)],
    );
    post(
        &mut co.l,
        d(2026, 2, 3),
        "x",
        &[(exp, DR, 100), (exp2, CR, 100)],
    );
    let tb = trial_balance_as_of(&co.l, d(2026, 12, 31));
    assert_eq!(
        tb_lines(&tb),
        vec![
            ("6000".to_string(), c(100), c(0)),
            ("6100".to_string(), c(0), c(100)),
        ]
    );
    assert!(tb.lines.iter().all(|l| l.code != "6900"));
}

/// Oracle: TB INVARIANT 4 — NEVER count a draft (balanced or not).
#[test]
fn tb_never_counts_drafts() {
    let mut co = company();
    let (cash, rev, exp) = (co.cash, co.rev, co.exp);
    post(
        &mut co.l,
        d(2026, 3, 1),
        "sale",
        &[(cash, DR, 2500), (rev, CR, 2500)],
    );
    draft(&mut co.l, d(2026, 3, 2), &[(exp, DR, 900), (cash, CR, 900)]);
    draft(&mut co.l, d(2026, 3, 2), &[(exp, DR, 900)]);
    let tb = trial_balance_as_of(&co.l, d(2026, 3, 31));
    assert_eq!(
        tb_lines(&tb),
        vec![
            ("1000".to_string(), c(2500), c(0)),
            ("4000".to_string(), c(0), c(2500)),
        ]
    );
    assert_eq!(tb_lines(&trial_balance(&co.l)), tb_lines(&tb));
}

/// Oracle: TB INVARIANT 4 — closing entries dated on or before `as_of`
/// count, so after a close it is the post-closing TB (hand-computed: Jan
/// revenue 100, expense 30 -> 1000 Dr 70, 3999 Cr 70). The day before the
/// closing date it is still the pre-closing TB.
#[test]
fn tb_after_close_is_post_closing_trial_balance() {
    let mut co = company();
    let (cash, rev, exp) = (co.cash, co.rev, co.exp);
    post(
        &mut co.l,
        d(2026, 1, 5),
        "sale",
        &[(cash, DR, 10000), (rev, CR, 10000)],
    );
    post(
        &mut co.l,
        d(2026, 1, 10),
        "supplies",
        &[(exp, DR, 3000), (cash, CR, 3000)],
    );
    let p =
        co.l.create_period("2026-01", d(2026, 1, 1), d(2026, 1, 31))
            .unwrap();
    co.l.close_period(p).unwrap();

    let pre = trial_balance_as_of(&co.l, d(2026, 1, 30));
    assert_eq!(
        tb_lines(&pre),
        vec![
            ("1000".to_string(), c(7000), c(0)),
            ("4000".to_string(), c(0), c(10000)),
            ("6000".to_string(), c(3000), c(0)),
        ]
    );
    let post_close = trial_balance_as_of(&co.l, d(2026, 1, 31));
    assert_eq!(
        tb_lines(&post_close),
        vec![
            ("1000".to_string(), c(7000), c(0)),
            ("3999".to_string(), c(0), c(7000)),
        ]
    );
    assert_eq!(post_close.total_debits, c(7000));
    assert_eq!(post_close.total_credits, c(7000));
    // TB INVARIANT 1 (trial_balance): equals as-of on or after the last date.
    assert_eq!(tb_lines(&trial_balance(&co.l)), tb_lines(&post_close));
}

// ===========================================================================
// profit_and_loss_for_period — examples
// ===========================================================================

/// Oracle: P&L INVARIANTS 1-2 — a line per revenue/expense account with its
/// normal-side movement in [start, end] inclusive; balance-sheet accounts
/// and entries outside the range never counted (hand-computed: 4000 70,
/// 4100 250, 6000 45, 6100 60, NI 215).
#[test]
fn pnl_period_is_a_line_per_account_within_the_inclusive_range() {
    let mut co = company();
    let (cash, ar, ap, rev, rev2, exp, exp2) =
        (co.cash, co.ar, co.ap, co.rev, co.rev2, co.exp, co.exp2);
    post(
        &mut co.l,
        d(2026, 1, 31),
        "before",
        &[(cash, DR, 99900), (rev, CR, 99900)],
    );
    post(
        &mut co.l,
        d(2026, 2, 1),
        "sale",
        &[(cash, DR, 10000), (rev, CR, 10000)],
    );
    post(
        &mut co.l,
        d(2026, 2, 10),
        "refund",
        &[(rev, DR, 3000), (cash, CR, 3000)],
    );
    post(
        &mut co.l,
        d(2026, 2, 14),
        "service",
        &[(ar, DR, 25000), (rev2, CR, 25000)],
    );
    post(
        &mut co.l,
        d(2026, 2, 20),
        "supplies",
        &[(exp, DR, 4000), (cash, CR, 4000)],
    );
    post(
        &mut co.l,
        d(2026, 2, 28),
        "rent",
        &[(exp2, DR, 6000), (ap, CR, 6000)],
    );
    post(
        &mut co.l,
        d(2026, 2, 28),
        "more",
        &[(exp, DR, 500), (cash, CR, 500)],
    );
    post(
        &mut co.l,
        d(2026, 3, 1),
        "after",
        &[(exp, DR, 99900), (cash, CR, 99900)],
    );

    let p = profit_and_loss_for_period(&co.l, d(2026, 2, 1), d(2026, 2, 28), None).unwrap();
    assert_eq!(p.start, d(2026, 2, 1));
    assert_eq!(p.end, d(2026, 2, 28));
    assert_eq!(sl(&p.revenue), vec![s("4000", 7000), s("4100", 25000)]);
    assert_eq!(sl(&p.expenses), vec![s("6000", 4500), s("6100", 6000)]);
    assert_eq!(p.total_revenue, c(32000));
    assert_eq!(p.total_expenses, c(10500));
    assert_eq!(p.net_income(), c(21500));
}

/// Oracle: StatementLine contract — a contra movement is negative (a
/// revenue refund alone in March reads -20; NI -20).
#[test]
fn pnl_period_contra_movement_is_negative() {
    let mut co = company();
    let (cash, rev) = (co.cash, co.rev);
    post(
        &mut co.l,
        d(2026, 2, 1),
        "sale",
        &[(cash, DR, 5000), (rev, CR, 5000)],
    );
    post(
        &mut co.l,
        d(2026, 3, 5),
        "refund",
        &[(rev, DR, 2000), (cash, CR, 2000)],
    );
    let p = profit_and_loss_for_period(&co.l, d(2026, 3, 1), d(2026, 3, 31), None).unwrap();
    assert_eq!(sl(&p.revenue), vec![s("4000", -2000)]);
    assert!(p.expenses.is_empty());
    assert_eq!(p.total_revenue, c(-2000));
    assert_eq!(p.net_income(), c(-2000));
}

/// Oracle: P&L SCENARIO 1 — a closed month still reports its earnings
/// (revenue 100, expenses 30, NI 70), and the closing entry is NEVER counted.
#[test]
fn pnl_closed_month_still_reports_its_earnings() {
    let mut co = company();
    let (cash, rev, exp) = (co.cash, co.rev, co.exp);
    post(
        &mut co.l,
        d(2026, 1, 5),
        "sale",
        &[(cash, DR, 10000), (rev, CR, 10000)],
    );
    post(
        &mut co.l,
        d(2026, 1, 10),
        "supplies",
        &[(exp, DR, 3000), (cash, CR, 3000)],
    );
    let p =
        co.l.create_period("2026-01", d(2026, 1, 1), d(2026, 1, 31))
            .unwrap();
    assert!(co.l.close_period(p).unwrap().is_some());
    let pl = profit_and_loss_for_period(&co.l, d(2026, 1, 1), d(2026, 1, 31), None).unwrap();
    assert_eq!(sl(&pl.revenue), vec![s("4000", 10000)]);
    assert_eq!(sl(&pl.expenses), vec![s("6000", 3000)]);
    assert_eq!(pl.net_income(), c(7000));
    let year = profit_and_loss_for_period(&co.l, d(2026, 1, 1), d(2026, 12, 31), None).unwrap();
    assert_eq!(year.net_income(), c(7000));
}

/// Oracle: P&L SCENARIO 2 (DESIGN, docs/testing.md #1) — start after end
/// refuses.
#[test]
fn pnl_period_start_after_end_refuses() {
    let co = company();
    assert!(profit_and_loss_for_period(&co.l, d(2026, 3, 2), d(2026, 3, 1), None).is_err());
}

/// Oracle: P&L INVARIANT 2 — NEVER count a draft.
#[test]
fn pnl_period_never_counts_drafts() {
    let mut co = company();
    let (cash, rev, exp) = (co.cash, co.rev, co.exp);
    post(
        &mut co.l,
        d(2026, 4, 1),
        "sale",
        &[(cash, DR, 1234), (rev, CR, 1234)],
    );
    draft(&mut co.l, d(2026, 4, 2), &[(exp, DR, 500), (cash, CR, 500)]);
    draft(&mut co.l, d(2026, 4, 2), &[(rev, CR, 500)]);
    let p = profit_and_loss_for_period(&co.l, d(2026, 4, 1), d(2026, 4, 30), None).unwrap();
    assert_eq!(sl(&p.revenue), vec![s("4000", 1234)]);
    assert!(p.expenses.is_empty());
}

/// Builds the class example: Retail sale 100 + supplies 20, Wholesale sale
/// 50, untagged supplies 30 (its cash line tagged "Ghost"), and a class
/// "Unused" with no lines. Classes are added Wholesale-first to test
/// sorting by name.
fn class_books() -> (Co, ClassId, ClassId) {
    let mut co = company();
    let (cash, rev, exp) = (co.cash, co.rev, co.exp);
    let wholesale = co.l.add_class("Wholesale").unwrap();
    let retail = co.l.add_class("Retail").unwrap();
    let _unused = co.l.add_class("Unused").unwrap();
    let ghost = co.l.add_class("Ghost").unwrap();

    let e = draft(
        &mut co.l,
        d(2026, 2, 5),
        &[(cash, DR, 10000), (rev, CR, 10000)],
    );
    co.l.set_line_class(e, 1, Some(retail)).unwrap();
    co.l.post_entry(e).unwrap();
    let e = draft(
        &mut co.l,
        d(2026, 2, 6),
        &[(cash, DR, 5000), (rev, CR, 5000)],
    );
    co.l.set_line_class(e, 1, Some(wholesale)).unwrap();
    co.l.post_entry(e).unwrap();
    let e = draft(
        &mut co.l,
        d(2026, 2, 7),
        &[(exp, DR, 3000), (cash, CR, 3000)],
    );
    co.l.set_line_class(e, 1, Some(ghost)).unwrap();
    co.l.post_entry(e).unwrap();
    let e = draft(
        &mut co.l,
        d(2026, 2, 8),
        &[(exp, DR, 2000), (cash, CR, 2000)],
    );
    co.l.set_line_class(e, 0, Some(retail)).unwrap();
    co.l.post_entry(e).unwrap();
    // A tagged draft never counts.
    let e = draft(&mut co.l, d(2026, 2, 9), &[(exp, DR, 777), (cash, CR, 777)]);
    co.l.set_line_class(e, 0, Some(retail)).unwrap();
    (co, retail, wholesale)
}

/// Oracle: P&L INVARIANT 5 — with a class, count only lines tagged with it
/// (hand-computed: Retail rev 100 exp 20; Wholesale rev 50; all rev 150
/// exp 50).
#[test]
fn pnl_period_with_class_counts_only_tagged_lines() {
    let (co, retail, wholesale) = class_books();
    let (a, b) = (d(2026, 2, 1), d(2026, 2, 28));
    let r = profit_and_loss_for_period(&co.l, a, b, Some(retail)).unwrap();
    assert_eq!(sl(&r.revenue), vec![s("4000", 10000)]);
    assert_eq!(sl(&r.expenses), vec![s("6000", 2000)]);
    let w = profit_and_loss_for_period(&co.l, a, b, Some(wholesale)).unwrap();
    assert_eq!(sl(&w.revenue), vec![s("4000", 5000)]);
    assert!(sl_nonzero(&w.expenses).is_empty());
    assert_eq!(w.total_expenses, Decimal::ZERO);
    let all = profit_and_loss_for_period(&co.l, a, b, None).unwrap();
    assert_eq!(sl(&all.revenue), vec![s("4000", 15000)]);
    assert_eq!(sl(&all.expenses), vec![s("6000", 5000)]);
}

// ===========================================================================
// profit_and_loss_by_class — examples
// ===========================================================================

/// Oracle: by-class doc — one row per class with P&L lines, sorted by name,
/// unclassified last; classes with no P&L lines (Unused; Ghost tags only a
/// cash line) get no row; rows sum to the whole P&L (hand-computed).
#[test]
fn pnl_by_class_rows_sorted_by_name_unclassified_last() {
    let (co, _, _) = class_books();
    let rows = profit_and_loss_by_class(&co.l).unwrap();
    let got: Vec<(Option<String>, Decimal, Decimal)> = rows
        .iter()
        .map(|r| {
            (
                r.class.as_ref().map(|k| k.name.clone()),
                r.profit_and_loss.revenue,
                r.profit_and_loss.expenses,
            )
        })
        .collect();
    assert_eq!(
        got,
        vec![
            (Some("Retail".to_string()), c(10000), c(2000)),
            (Some("Wholesale".to_string()), c(5000), c(0)),
            (None, c(0), c(3000)),
        ]
    );
    let whole = unclosed_earnings(&co.l).unwrap();
    assert_eq!(whole.revenue, c(15000));
    assert_eq!(whole.expenses, c(5000));
}

/// Oracle: by-class doc — no unclassified row when every P&L line is tagged
/// (an untagged cash line is not a P&L line).
#[test]
fn pnl_by_class_has_no_unclassified_row_when_all_pnl_lines_tagged() {
    let mut co = company();
    let (cash, rev) = (co.cash, co.rev);
    let k = co.l.add_class("Only").unwrap();
    let e = draft(&mut co.l, d(2026, 2, 5), &[(cash, DR, 100), (rev, CR, 100)]);
    co.l.set_line_class(e, 1, Some(k)).unwrap();
    co.l.post_entry(e).unwrap();
    let rows = profit_and_loss_by_class(&co.l).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].class.as_ref().map(|k| k.name.as_str()),
        Some("Only")
    );
    assert_eq!(rows[0].profit_and_loss.revenue, c(100));
}

// ===========================================================================
// balance_sheet_as_of — examples
// ===========================================================================

/// Oracle: BS SCENARIO 1 — nothing posted by `as_of`: empty and balanced.
#[test]
fn bs_with_nothing_posted_is_empty_and_balanced() {
    let mut co = company();
    let (cash, cap) = (co.cash, co.cap_a);
    post(
        &mut co.l,
        d(2026, 5, 1),
        "later",
        &[(cash, DR, 100), (cap, CR, 100)],
    );
    let b = balance_sheet_as_of(&co.l, d(2026, 4, 30)).unwrap();
    assert_eq!(b.as_of, d(2026, 4, 30));
    assert!(b.assets.is_empty() && b.liabilities.is_empty() && b.equity.is_empty());
    assert_eq!(b.current_earnings, Decimal::ZERO);
    assert_eq!(b.total_assets, Decimal::ZERO);
    assert_eq!(b.total_liabilities, Decimal::ZERO);
    assert_eq!(b.total_equity, Decimal::ZERO);
    assert!(b.balanced());
}

/// Oracle: BS INVARIANTS 1, 3, 4, 5 — hand-computed statement: assets 1450
/// (cash 1250, AR 200), loan 300, capital 500 + 500, current earnings 150,
/// total equity 1150; contra accumulated depreciation reads negative; no
/// revenue or expense lines.
#[test]
fn bs_lines_totals_and_current_earnings_by_hand() {
    let mut co = company();
    let (cash, ar, loan, cap_a, cap_b, rev, exp, accum, depr) = (
        co.cash, co.ar, co.loan, co.cap_a, co.cap_b, co.rev, co.exp, co.accum, co.depr,
    );
    post(
        &mut co.l,
        d(2026, 1, 2),
        "contributions",
        &[(cash, DR, 100000), (cap_a, CR, 50000), (cap_b, CR, 50000)],
    );
    post(
        &mut co.l,
        d(2026, 1, 3),
        "loan",
        &[(cash, DR, 30000), (loan, CR, 30000)],
    );
    post(
        &mut co.l,
        d(2026, 1, 4),
        "invoice",
        &[(ar, DR, 20000), (rev, CR, 20000)],
    );
    post(
        &mut co.l,
        d(2026, 1, 5),
        "supplies",
        &[(exp, DR, 5000), (cash, CR, 5000)],
    );
    post(
        &mut co.l,
        d(2026, 1, 6),
        "depr",
        &[(depr, DR, 1000), (accum, CR, 1000)],
    );
    let b = balance_sheet_as_of(&co.l, d(2026, 1, 31)).unwrap();
    assert_eq!(
        sl(&b.assets),
        vec![s("1000", 125000), s("1100", 20000), s("1510", -1000)]
    );
    assert_eq!(sl(&b.liabilities), vec![s("2500", 30000)]);
    assert_eq!(sl(&b.equity), vec![s("3000", 50000), s("3001", 50000)]);
    assert_eq!(b.current_earnings, c(14000));
    assert_eq!(b.total_assets, c(144000));
    assert_eq!(b.total_liabilities, c(30000));
    assert_eq!(b.total_equity, c(114000));
    assert!(b.balanced());
}

/// Oracle: BS INVARIANTS 2-3 — closing entries count: after closing January
/// (rev 100 Jan 10, exp 30 Jan 20) 3999 holds 70 and current earnings is
/// only February's 40; mid-January (before the closing date) current
/// earnings is 100 and there is no 3999 line.
#[test]
fn bs_after_close_moves_earnings_to_retained_earnings() {
    let mut co = company();
    let (cash, rev, exp) = (co.cash, co.rev, co.exp);
    post(
        &mut co.l,
        d(2026, 1, 10),
        "sale",
        &[(cash, DR, 10000), (rev, CR, 10000)],
    );
    post(
        &mut co.l,
        d(2026, 1, 20),
        "supplies",
        &[(exp, DR, 3000), (cash, CR, 3000)],
    );
    let p =
        co.l.create_period("2026-01", d(2026, 1, 1), d(2026, 1, 31))
            .unwrap();
    co.l.close_period(p).unwrap();
    post(
        &mut co.l,
        d(2026, 2, 3),
        "sale",
        &[(cash, DR, 4000), (rev, CR, 4000)],
    );

    let mid = balance_sheet_as_of(&co.l, d(2026, 1, 15)).unwrap();
    assert!(sl_nonzero(&mid.equity).is_empty());
    assert_eq!(mid.current_earnings, c(10000));

    let jan = balance_sheet_as_of(&co.l, d(2026, 1, 31)).unwrap();
    assert_eq!(sl(&jan.equity), vec![s("3999", 7000)]);
    assert_eq!(jan.current_earnings, Decimal::ZERO);
    assert!(jan.balanced());

    let feb = balance_sheet_as_of(&co.l, d(2026, 2, 28)).unwrap();
    assert_eq!(sl(&feb.equity), vec![s("3999", 7000)]);
    assert_eq!(feb.current_earnings, c(4000));
    assert_eq!(feb.total_equity, c(11000));
    assert_eq!(feb.total_assets, c(11000));
}

/// Oracle: BS doc ("every posted entry") — drafts never counted.
#[test]
fn bs_never_counts_drafts() {
    let mut co = company();
    let (cash, cap, exp) = (co.cash, co.cap_a, co.exp);
    post(
        &mut co.l,
        d(2026, 1, 2),
        "in",
        &[(cash, DR, 1000), (cap, CR, 1000)],
    );
    draft(&mut co.l, d(2026, 1, 3), &[(exp, DR, 400), (cash, CR, 400)]);
    let b = balance_sheet_as_of(&co.l, d(2026, 1, 31)).unwrap();
    assert_eq!(sl(&b.assets), vec![s("1000", 1000)]);
    assert_eq!(b.current_earnings, Decimal::ZERO);
}

// ===========================================================================
// general_ledger — examples
// ===========================================================================

/// Oracle: GL INVARIANTS 1-3 — hand-computed February ledger: cash begins
/// 1000, Feb 3 credit 40 (split "6000 Supplies") -> 960, Feb 10 debit 300
/// (split "-Split-") -> 1260; Sales gets credit 200 split "1000 Cash";
/// Capital A is listed for its beginning balance alone; Rent (only March
/// activity) is not listed.
#[test]
fn gl_hand_computed_february() {
    let mut co = company();
    let (cash, cap_a, rev, rev2, exp, exp2) = (co.cash, co.cap_a, co.rev, co.rev2, co.exp, co.exp2);
    post(
        &mut co.l,
        d(2026, 1, 5),
        "Contribution",
        &[(cash, DR, 100000), (cap_a, CR, 100000)],
    );
    let e1 = post(
        &mut co.l,
        d(2026, 2, 3),
        "Supplies",
        &[(exp, DR, 4000), (cash, CR, 4000)],
    );
    let e2 = post(
        &mut co.l,
        d(2026, 2, 10),
        "Sales",
        &[(cash, DR, 30000), (rev, CR, 20000), (rev2, CR, 10000)],
    );
    post(
        &mut co.l,
        d(2026, 3, 1),
        "Rent",
        &[(exp2, DR, 1000), (cash, CR, 1000)],
    );
    draft(
        &mut co.l,
        d(2026, 2, 15),
        &[(exp2, DR, 500), (cash, CR, 500)],
    );

    let g = general_ledger(&co.l, d(2026, 2, 1), d(2026, 2, 28)).unwrap();
    assert_eq!(g.start, d(2026, 2, 1));
    assert_eq!(g.end, d(2026, 2, 28));
    let codes: Vec<&str> = g.accounts.iter().map(|a| a.code.as_str()).collect();
    assert_eq!(codes, vec!["1000", "3000", "4000", "4100", "6000"]);

    let cashacct = &g.accounts[0];
    assert_eq!(cashacct.name, "Cash");
    assert_eq!(cashacct.account_type, AccountType::Asset);
    assert_eq!(cashacct.beginning_balance, c(100000));
    let p: Vec<(NaiveDate, EntryId, &str, &str, Decimal, Decimal, Decimal)> = cashacct
        .postings
        .iter()
        .map(|p| {
            (
                p.date,
                p.entry_id,
                p.description.as_str(),
                p.split.as_str(),
                p.debit,
                p.credit,
                p.balance,
            )
        })
        .collect();
    assert_eq!(
        p,
        vec![
            (
                d(2026, 2, 3),
                e1,
                "Supplies",
                "6000 Supplies",
                c(0),
                c(4000),
                c(96000)
            ),
            (
                d(2026, 2, 10),
                e2,
                "Sales",
                "-Split-",
                c(30000),
                c(0),
                c(126000)
            ),
        ]
    );
    assert_eq!(cashacct.total_debits, c(30000));
    assert_eq!(cashacct.total_credits, c(4000));
    assert_eq!(cashacct.ending_balance, c(126000));

    let capital = &g.accounts[1];
    assert_eq!(capital.beginning_balance, c(100000));
    assert!(capital.postings.is_empty());
    assert_eq!(capital.ending_balance, c(100000));

    let sales = &g.accounts[2];
    assert_eq!(sales.beginning_balance, Decimal::ZERO);
    assert_eq!(sales.postings.len(), 1);
    assert_eq!(sales.postings[0].split, "1000 Cash");
    assert_eq!(sales.postings[0].credit, c(20000));
    assert_eq!(sales.postings[0].balance, c(20000));
    assert_eq!(sales.ending_balance, c(20000));
}

/// Oracle: GL INVARIANT 1 — postings in date order (then entry), running
/// balance follows that order, even when entries were posted out of date
/// order; a liability's balance runs on its credit side.
#[test]
fn gl_postings_in_date_order_with_normal_side_running_balance() {
    let mut co = company();
    let (cash, loan) = (co.cash, co.loan);
    let late = post(
        &mut co.l,
        d(2026, 5, 20),
        "draw 2",
        &[(cash, DR, 700), (loan, CR, 700)],
    );
    let early = post(
        &mut co.l,
        d(2026, 5, 5),
        "draw 1",
        &[(cash, DR, 300), (loan, CR, 300)],
    );
    let pay = post(
        &mut co.l,
        d(2026, 5, 20),
        "repay",
        &[(loan, DR, 100), (cash, CR, 100)],
    );
    let g = general_ledger(&co.l, d(2026, 5, 1), d(2026, 5, 31)).unwrap();
    let l = g.accounts.iter().find(|a| a.code == "2500").unwrap();
    let got: Vec<(EntryId, Decimal)> = l.postings.iter().map(|p| (p.entry_id, p.balance)).collect();
    assert!(late < pay);
    assert_eq!(got, vec![(early, c(300)), (late, c(1000)), (pay, c(900))]);
    assert_eq!(l.ending_balance, c(900));
}

/// Oracle: GL doc — closing entries are postings like any other (January
/// close shows on Sales as a debit of 100, ending 0).
#[test]
fn gl_shows_closing_entries_as_postings() {
    let mut co = company();
    let (cash, rev) = (co.cash, co.rev);
    post(
        &mut co.l,
        d(2026, 1, 10),
        "sale",
        &[(cash, DR, 10000), (rev, CR, 10000)],
    );
    let p =
        co.l.create_period("2026-01", d(2026, 1, 1), d(2026, 1, 31))
            .unwrap();
    let close = co.l.close_period(p).unwrap().unwrap();
    let g = general_ledger(&co.l, d(2026, 1, 1), d(2026, 1, 31)).unwrap();
    let sales = g.accounts.iter().find(|a| a.code == "4000").unwrap();
    assert_eq!(sales.postings.len(), 2);
    assert_eq!(sales.postings[1].entry_id, close);
    assert_eq!(sales.postings[1].debit, c(10000));
    assert_eq!(sales.ending_balance, Decimal::ZERO);
    let re = g.accounts.iter().find(|a| a.code == "3999").unwrap();
    assert_eq!(re.ending_balance, c(10000));
}

// ===========================================================================
// members_equity — examples
// ===========================================================================

/// Oracle: ME SCENARIO 1 — shares that don't sum to 1, or aren't positive,
/// refuse.
#[test]
fn me_refuses_bad_shares() {
    let co = company();
    let (a, b) = (d(2026, 1, 1), d(2026, 12, 31));
    for (sa, sb) in [(60, 60), (100, 0), (-50, 150), (40, 50)] {
        let mut m = members_50_50(&co);
        m[0].share = c(sa);
        m[1].share = c(sb);
        assert!(members_equity(&co.l, a, b, &m).is_err(), "shares {sa} {sb}");
    }
}

/// Oracle: ME INVARIANTS 1-3b — hand-computed roll-forward, cumulative
/// allocation (docs/testing.md #7; K-1 Item L: one running capital
/// balance), odd cents by largest remainder, ties to the earlier member.
/// Undistributed before Feb 1: 100.01 -> A 50.01, B 50.00. Through Mar 31:
/// 100.01 + 301.01 - 100.00 = 301.02 -> 150.51 each. Endings: A 950.00 +
/// 150.51 = 1100.51, B 1000.00 + 150.51 = 1150.51. Incomes (the
/// remainder): A 1100.51 - 1050.01 + 50.00 = 100.50, B 100.51; sum 201.01,
/// the period's income. Total ending 2251.02 = BS equity. (Splitting the
/// period's income on its own would give A 150.52 and B 150.50 to date:
/// two cents off half each.)
#[test]
fn me_hand_computed_roll_forward_with_largest_remainder() {
    let mut co = company();
    let (cash, cap_a, cap_b, rev, exp) = (co.cash, co.cap_a, co.cap_b, co.rev, co.exp);
    post(
        &mut co.l,
        d(2026, 1, 5),
        "A in",
        &[(cash, DR, 100000), (cap_a, CR, 100000)],
    );
    post(
        &mut co.l,
        d(2026, 1, 20),
        "Jan sale",
        &[(cash, DR, 10001), (rev, CR, 10001)],
    );
    post(
        &mut co.l,
        d(2026, 2, 2),
        "B in",
        &[(cash, DR, 100000), (cap_b, CR, 100000)],
    );
    post(
        &mut co.l,
        d(2026, 2, 10),
        "Feb sale",
        &[(cash, DR, 30101), (rev, CR, 30101)],
    );
    post(
        &mut co.l,
        d(2026, 2, 15),
        "supplies",
        &[(exp, DR, 10000), (cash, CR, 10000)],
    );
    post(
        &mut co.l,
        d(2026, 3, 1),
        "A draw",
        &[(cap_a, DR, 5000), (cash, CR, 5000)],
    );

    let st = members_equity(&co.l, d(2026, 2, 1), d(2026, 3, 31), &members_50_50(&co)).unwrap();
    assert_eq!(st.start, d(2026, 2, 1));
    assert_eq!(st.end, d(2026, 3, 31));
    assert_eq!(
        st.members,
        vec![
            col("A", 105001, 0, 10050, 5000, 0, 110051),
            col("B", 5000, 100000, 10051, 0, 0, 115051),
        ]
    );
    assert_eq!(
        st.total,
        col("Total", 110001, 100000, 20101, 5000, 0, 225102)
    );
    let bs = balance_sheet_as_of(&co.l, d(2026, 3, 31)).unwrap();
    assert_eq!(bs.total_equity, c(225102));
}

/// Oracle: ME INVARIANT 4 — an equity change through a non-member account
/// (3500 credited 10) goes in "other", allocated by share (5 / 5), never
/// hidden; ending ties to BS equity 1010.
#[test]
fn me_unexplained_equity_change_goes_in_other() {
    let mut co = company();
    let (cash, cap_a, other) = (co.cash, co.cap_a, co.other_eq);
    post(
        &mut co.l,
        d(2026, 1, 5),
        "A in",
        &[(cash, DR, 100000), (cap_a, CR, 100000)],
    );
    post(
        &mut co.l,
        d(2026, 2, 3),
        "grant",
        &[(cash, DR, 1000), (other, CR, 1000)],
    );
    let st = members_equity(&co.l, d(2026, 2, 1), d(2026, 2, 28), &members_50_50(&co)).unwrap();
    assert_eq!(
        st.members,
        vec![
            col("A", 100000, 0, 0, 0, 500, 100500),
            col("B", 0, 0, 0, 0, 500, 500)
        ]
    );
    assert_eq!(st.total, col("Total", 100000, 0, 0, 0, 1000, 101000));
}

/// Oracle: ME INVARIANTS 1-3 with a members close inside / before the
/// period: the close's credits to capital are allocated earnings, not
/// contributions (hand-computed: Jan income 100.01 closes A 50.01 / B
/// 50.00).
///
/// ASSUMED: a closing entry's credit to a member's capital account is not
/// reported as a contribution (B-11: a close moves earnings), so January's
/// statement shows contributions 1000 / 0, income 50.01 / 50.00.
#[test]
fn me_after_members_close_does_not_count_close_as_contribution() {
    let mut co = company();
    let (cash, cap_a, cap_b, rev) = (co.cash, co.cap_a, co.cap_b, co.rev);
    post(
        &mut co.l,
        d(2026, 1, 5),
        "A in",
        &[(cash, DR, 100000), (cap_a, CR, 100000)],
    );
    post(
        &mut co.l,
        d(2026, 1, 20),
        "Jan sale",
        &[(cash, DR, 10001), (rev, CR, 10001)],
    );
    let p =
        co.l.create_period("2026-01", d(2026, 1, 1), d(2026, 1, 31))
            .unwrap();
    let to = CloseTo::Members(vec![
        MemberClose {
            capital: cap_a,
            share: c(50),
            distributions: None,
        },
        MemberClose {
            capital: cap_b,
            share: c(50),
            distributions: None,
        },
    ]);
    co.l.close_period_to(p, &to).unwrap();
    post(
        &mut co.l,
        d(2026, 2, 10),
        "Feb sale",
        &[(cash, DR, 1000), (rev, CR, 1000)],
    );

    let bs_jan = balance_sheet_as_of(&co.l, d(2026, 1, 31)).unwrap();
    assert_eq!(sl(&bs_jan.equity), vec![s("3000", 105001), s("3001", 5000)]);

    let m = members_50_50(&co);
    let jan = members_equity(&co.l, d(2026, 1, 1), d(2026, 1, 31), &m).unwrap();
    assert_eq!(
        jan.members,
        vec![
            col("A", 0, 100000, 5001, 0, 0, 105001),
            col("B", 0, 0, 5000, 0, 0, 5000)
        ]
    );
    let feb = members_equity(&co.l, d(2026, 2, 1), d(2026, 2, 28), &m).unwrap();
    assert_eq!(
        feb.members,
        vec![
            col("A", 105001, 0, 500, 0, 0, 105501),
            col("B", 5000, 0, 500, 0, 0, 5500)
        ]
    );
    assert_eq!(feb.total.ending, c(111001));
}

fn three_members(co: &Co) -> Vec<MemberCapital> {
    vec![
        MemberCapital {
            name: "A".into(),
            capital_account: co.cap_a,
            distribution_account: None,
            share: c(50),
        },
        MemberCapital {
            name: "B".into(),
            capital_account: co.cap_b,
            distribution_account: None,
            share: c(30),
        },
        MemberCapital {
            name: "C".into(),
            capital_account: co.other_eq,
            distribution_account: None,
            share: c(20),
        },
    ]
}

/// Oracle: ME INVARIANT 3 (largest remainder, DECISION 2026-10-13) —
/// income 0.09 at 50/30/20: exact 0.045/0.027/0.018, floors 0.04/0.02/0.01,
/// two leftover cents to the largest remainders (C .008, B .007): 0.04,
/// 0.03, 0.02. (Last-absorbs would give 0.05/0.03/0.01.) A loss of 0.09
/// splits as the negation.
#[test]
fn me_three_members_largest_remainder_gain_and_loss() {
    let mut co = company();
    let (cash, rev, exp) = (co.cash, co.rev, co.exp);
    post(
        &mut co.l,
        d(2026, 2, 10),
        "sale",
        &[(cash, DR, 9), (rev, CR, 9)],
    );
    post(
        &mut co.l,
        d(2026, 3, 10),
        "cost",
        &[(exp, DR, 9), (cash, CR, 9)],
    );
    let m = three_members(&co);
    let feb = members_equity(&co.l, d(2026, 2, 1), d(2026, 2, 28), &m).unwrap();
    let ni: Vec<Decimal> = feb.members.iter().map(|x| x.net_income).collect();
    assert_eq!(ni, vec![c(4), c(3), c(2)]);
    assert_eq!(feb.total.net_income, c(9));
    let mar = members_equity(&co.l, d(2026, 3, 1), d(2026, 3, 31), &m).unwrap();
    let ni: Vec<Decimal> = mar.members.iter().map(|x| x.net_income).collect();
    assert_eq!(ni, vec![c(-4), c(-3), c(-2)]);
    // March begins with February's unclosed earnings, split the same way.
    let beg: Vec<Decimal> = mar.members.iter().map(|x| x.beginning).collect();
    assert_eq!(beg, vec![c(4), c(3), c(2)]);
    assert_eq!(mar.total.ending, Decimal::ZERO);
}

/// Oracle: MemberCapital doc — a member's distribution account is part of
/// their capital on every statement: A contributes 1000, draws 30 through
/// 3010 -> distributions 30, ending 970 = BS equity.
#[test]
fn me_distribution_account_counts_as_distributions() {
    let mut co = company();
    let (cash, cap_a) = (co.cash, co.cap_a);
    let draws =
        co.l.add_account("3010", "Distributions A", AccountType::Equity, None, false)
            .unwrap();
    post(
        &mut co.l,
        d(2026, 1, 5),
        "A in",
        &[(cash, DR, 100000), (cap_a, CR, 100000)],
    );
    post(
        &mut co.l,
        d(2026, 2, 5),
        "A draw",
        &[(draws, DR, 3000), (cash, CR, 3000)],
    );
    let mut m = members_50_50(&co);
    m[0].distribution_account = Some(draws);
    let st = members_equity(&co.l, d(2026, 2, 1), d(2026, 2, 28), &m).unwrap();
    assert_eq!(
        st.members,
        vec![
            col("A", 100000, 0, 0, 3000, 0, 97000),
            col("B", 0, 0, 0, 0, 0, 0)
        ]
    );
    assert_eq!(
        balance_sheet_as_of(&co.l, d(2026, 2, 28))
            .unwrap()
            .total_equity,
        c(97000)
    );
}

/// Oracle: ME doc — earnings not yet distributed include retained earnings:
/// after a January close into 3999 (100.01), February begins A 1000 +
/// 50.01, B 50.00 (largest remainder, tie to the earlier), and 3999 is not
/// "other".
#[test]
fn me_beginning_includes_retained_earnings_by_share() {
    let mut co = company();
    let (cash, cap_a, rev) = (co.cash, co.cap_a, co.rev);
    post(
        &mut co.l,
        d(2026, 1, 5),
        "A in",
        &[(cash, DR, 100000), (cap_a, CR, 100000)],
    );
    post(
        &mut co.l,
        d(2026, 1, 20),
        "Jan sale",
        &[(cash, DR, 10001), (rev, CR, 10001)],
    );
    let p =
        co.l.create_period("2026-01", d(2026, 1, 1), d(2026, 1, 31))
            .unwrap();
    co.l.close_period(p).unwrap();
    let st = members_equity(&co.l, d(2026, 2, 1), d(2026, 2, 28), &members_50_50(&co)).unwrap();
    assert_eq!(
        st.members,
        vec![
            col("A", 105001, 0, 0, 0, 0, 105001),
            col("B", 5000, 0, 0, 0, 0, 5000)
        ]
    );
}

/// Oracle: CF INVARIANT 1 — a transfer between two designated cash
/// accounts is no cash flow at all (net change 0, every section 0).
#[test]
fn cf_transfer_between_cash_accounts_is_not_a_flow() {
    let mut co = company();
    let (cash, ar, cap_a) = (co.cash, co.ar, co.cap_a);
    post(
        &mut co.l,
        d(2026, 1, 5),
        "A in",
        &[(cash, DR, 100000), (cap_a, CR, 100000)],
    );
    post(
        &mut co.l,
        d(2026, 2, 5),
        "sweep",
        &[(ar, DR, 40000), (cash, CR, 40000)],
    );
    let cf = cash_flow(
        &co.l,
        &CashFlowParams {
            start: d(2026, 2, 1),
            end: d(2026, 2, 28),
            cash_accounts: vec![cash, ar],
            financing_accounts: vec![],
            investing_accounts: vec![],
        },
    )
    .unwrap();
    assert_eq!(cf.net_change_in_cash, Decimal::ZERO);
    assert_eq!(cf.cash_from_operations, Decimal::ZERO);
    assert_eq!(cf.cash_from_investing, Decimal::ZERO);
    assert_eq!(cf.cash_from_financing, Decimal::ZERO);
    assert_eq!(cf.cash_beginning, c(100000));
    assert_eq!(cf.cash_ending, c(100000));
}

// ===========================================================================
// cash_flow — examples
// ===========================================================================

fn cf_example_books() -> Co {
    let mut co = company();
    let (cash, ar, ap, loan, cap_a, cap_b, rev, exp) = (
        co.cash, co.ar, co.ap, co.loan, co.cap_a, co.cap_b, co.rev, co.exp,
    );
    post(
        &mut co.l,
        d(2026, 1, 5),
        "A in",
        &[(cash, DR, 100000), (cap_a, CR, 100000)],
    );
    post(
        &mut co.l,
        d(2026, 2, 2),
        "invoice",
        &[(ar, DR, 50000), (rev, CR, 50000)],
    );
    post(
        &mut co.l,
        d(2026, 2, 9),
        "collect",
        &[(cash, DR, 20000), (ar, CR, 20000)],
    );
    post(
        &mut co.l,
        d(2026, 2, 10),
        "bill",
        &[(exp, DR, 8000), (ap, CR, 8000)],
    );
    post(
        &mut co.l,
        d(2026, 2, 11),
        "cash exp",
        &[(exp, DR, 2000), (cash, CR, 2000)],
    );
    post(
        &mut co.l,
        d(2026, 2, 12),
        "loan",
        &[(cash, DR, 30000), (loan, CR, 30000)],
    );
    post(
        &mut co.l,
        d(2026, 2, 13),
        "B in",
        &[(cash, DR, 10000), (cap_b, CR, 10000)],
    );
    co
}

fn sorted_amounts(lines: &[ledger_core::CashFlowLine]) -> Vec<Decimal> {
    let mut v: Vec<Decimal> = lines
        .iter()
        .map(|l| l.amount)
        .filter(|a| !a.is_zero())
        .collect();
    v.sort();
    v
}

/// Oracle: CF INVARIANTS 1, 2, 4 — hand-computed February (indirect): NI
/// 400; AR up 300 (-300), AP up 80 (+80); CFO 180; financing loan 300 +
/// contribution 100 = 400; net change 580 = cash 1000 -> 1580.
///
/// Working capital is one line per account that changed; financing is gross per
/// account (docs/testing.md #8): here each financing account moved one way only.
#[test]
fn cf_hand_computed_with_designated_debt() {
    let co = cf_example_books();
    let params = CashFlowParams {
        start: d(2026, 2, 1),
        end: d(2026, 2, 28),
        cash_accounts: vec![co.cash],
        financing_accounts: vec![co.loan],
        investing_accounts: vec![],
    };
    let cf = cash_flow(&co.l, &params).unwrap();
    assert_eq!(cf.start, d(2026, 2, 1));
    assert_eq!(cf.end, d(2026, 2, 28));
    assert_eq!(cf.net_income, c(40000));
    assert_eq!(cf.depreciation, Decimal::ZERO);
    assert_eq!(
        sorted_amounts(&cf.working_capital),
        vec![c(-30000), c(8000)]
    );
    assert_eq!(cf.cash_from_operations, c(18000));
    assert_eq!(cf.cash_from_investing, Decimal::ZERO);
    assert_eq!(sorted_amounts(&cf.financing), vec![c(10000), c(30000)]);
    assert_eq!(cf.cash_from_financing, c(40000));
    assert_eq!(cf.net_change_in_cash, c(58000));
    assert_eq!(cf.cash_beginning, c(100000));
    assert_eq!(cf.cash_ending, c(158000));
}

/// Oracle: CashFlowParams doc — by default all liabilities are operating
/// working capital (the loan moves to operating: CFO 480, financing 100).
#[test]
fn cf_undesignated_liability_is_working_capital() {
    let co = cf_example_books();
    let params = CashFlowParams {
        start: d(2026, 2, 1),
        end: d(2026, 2, 28),
        cash_accounts: vec![co.cash],
        financing_accounts: vec![],
        investing_accounts: vec![],
    };
    let cf = cash_flow(&co.l, &params).unwrap();
    assert_eq!(cf.cash_from_operations, c(48000));
    assert_eq!(cf.cash_from_financing, c(10000));
    assert_eq!(cf.net_change_in_cash, c(58000));
}

/// Oracle: CF INVARIANT 5 / B-14 — closing entries never counted: after
/// closing February into retained earnings the February statement is
/// unchanged (NI still 400, 3999 not in financing).
#[test]
fn cf_never_counts_closing_entries() {
    let mut co = cf_example_books();
    let params = CashFlowParams {
        start: d(2026, 2, 1),
        end: d(2026, 2, 28),
        cash_accounts: vec![co.cash],
        financing_accounts: vec![co.loan],
        investing_accounts: vec![],
    };
    let before = cash_flow(&co.l, &params).unwrap();
    let p1 =
        co.l.create_period("2026-01", d(2026, 1, 1), d(2026, 1, 31))
            .unwrap();
    co.l.close_period(p1).unwrap();
    let p2 =
        co.l.create_period("2026-02", d(2026, 2, 1), d(2026, 2, 28))
            .unwrap();
    co.l.close_period(p2).unwrap();
    let after = cash_flow(&co.l, &params).unwrap();
    assert_eq!(after.net_income, c(40000));
    assert_eq!(cf_key(&after), cf_key(&before));
}

/// Oracle: CF INVARIANTS 1-3 — fixed-asset purchase in investing,
/// depreciation added back, disposal gain moved out of operating with the
/// proceeds in investing (hand-computed: Feb NI -100, depr 100, CFO 0,
/// investing -1200; Mar NI -50, depr 100, CFO 0, investing +1050).
///
/// ASSUMED: depreciation is recognised as credits to a registered asset's
/// accumulated-depreciation account against expense, and the disposal is
/// recognised structurally from the entry alone (dispose_asset not called).
#[test]
fn cf_fixed_asset_purchase_depreciation_and_disposal() {
    let mut co = company();
    let (cash, cap_a, equip, accum, depr, gain) =
        (co.cash, co.cap_a, co.equip, co.accum, co.depr, co.gain);
    post(
        &mut co.l,
        d(2026, 1, 5),
        "A in",
        &[(cash, DR, 1000000), (cap_a, CR, 1000000)],
    );
    post(
        &mut co.l,
        d(2026, 2, 1),
        "buy",
        &[(equip, DR, 120000), (cash, CR, 120000)],
    );
    co.l.register_asset(NewAsset {
        description: "Laptop".into(),
        purchase_date: d(2026, 2, 1),
        cost: usd(120000),
        salvage_value: usd(0),
        useful_life_months: 12,
        cost_account: equip,
        accum_depr_account: accum,
        expense_account: depr,
    })
    .unwrap();
    post(
        &mut co.l,
        d(2026, 2, 28),
        "depr Feb",
        &[(depr, DR, 10000), (accum, CR, 10000)],
    );
    post(
        &mut co.l,
        d(2026, 3, 31),
        "depr Mar",
        &[(depr, DR, 10000), (accum, CR, 10000)],
    );
    post(
        &mut co.l,
        d(2026, 3, 31),
        "sell",
        &[
            (cash, DR, 105000),
            (accum, DR, 20000),
            (equip, CR, 120000),
            (gain, CR, 5000),
        ],
    );
    let feb = cash_flow(
        &co.l,
        &CashFlowParams {
            start: d(2026, 2, 1),
            end: d(2026, 2, 28),
            cash_accounts: vec![cash],
            financing_accounts: vec![],
            investing_accounts: vec![],
        },
    )
    .unwrap();
    assert_eq!(feb.net_income, c(-10000));
    assert_eq!(feb.depreciation, c(10000));
    assert_eq!(feb.cash_from_operations, Decimal::ZERO);
    assert_eq!(feb.cash_from_investing, c(-120000));
    assert_eq!(feb.cash_from_financing, Decimal::ZERO);
    assert_eq!(feb.net_change_in_cash, c(-120000));

    let mar = cash_flow(
        &co.l,
        &CashFlowParams {
            start: d(2026, 3, 1),
            end: d(2026, 3, 31),
            cash_accounts: vec![cash],
            financing_accounts: vec![],
            investing_accounts: vec![],
        },
    )
    .unwrap();
    assert_eq!(mar.net_income, c(-5000));
    assert_eq!(mar.depreciation, c(10000));
    assert_eq!(mar.cash_from_operations, Decimal::ZERO);
    assert_eq!(mar.cash_from_investing, c(105000));
    assert_eq!(mar.net_change_in_cash, c(105000));
    assert_eq!(mar.cash_ending, c(985000));
}

// ===========================================================================
// snapshot — examples
// ===========================================================================

fn has_float(v: &Value) -> bool {
    match v {
        Value::Number(n) => n.is_f64(),
        Value::Array(a) => a.iter().any(has_float),
        Value::Object(o) => o.values().any(has_float),
        _ => false,
    }
}

fn decimal_strings(v: &Value, out: &mut Vec<Decimal>) {
    match v {
        Value::String(s) => {
            if let Ok(x) = s.parse::<Decimal>() {
                out.push(x);
            }
        }
        Value::Array(a) => a.iter().for_each(|x| decimal_strings(x, out)),
        Value::Object(o) => o.values().for_each(|x| decimal_strings(x, out)),
        _ => {}
    }
}

/// Oracle: snapshot INVARIANTS 1-2 — no floats; amounts are decimal strings
/// (the hand-computed 70.00 cash balance appears as a string); drafts do
/// not change it; it is deterministic.
#[test]
fn snapshot_uses_decimal_strings_and_ignores_drafts() {
    let mut co = company();
    let (cash, rev, exp) = (co.cash, co.rev, co.exp);
    post(
        &mut co.l,
        d(2026, 1, 5),
        "sale",
        &[(cash, DR, 10000), (rev, CR, 10000)],
    );
    post(
        &mut co.l,
        d(2026, 1, 10),
        "supplies",
        &[(exp, DR, 3001), (cash, CR, 3001)],
    );
    let snap = snapshot(&co.l).unwrap();
    assert!(!has_float(&snap));
    let mut amts = Vec::new();
    decimal_strings(&snap, &mut amts);
    assert!(amts.contains(&c(6999)), "{snap}");
    assert!(amts.contains(&c(3001)), "{snap}");
    draft(&mut co.l, d(2026, 1, 11), &[(exp, DR, 5), (cash, CR, 5)]);
    assert_eq!(snapshot(&co.l).unwrap(), snap);
    assert_eq!(snapshot(&co.l).unwrap(), snap);
}

// ===========================================================================
// Random balanced books for the properties.
// ===========================================================================

const N: usize = 10;
const CASH: usize = 0;
const LOAN: usize = 3;
const CAP_A: usize = 4;
const CAP_B: usize = 5;

/// The random chart: (code, name, type, child of the 6900 header).
const RCHART: [(&str, &str, AccountType, bool); N] = [
    ("1000", "Cash", AccountType::Asset, false),
    ("1100", "Receivables", AccountType::Asset, false),
    ("2000", "Payables", AccountType::Liability, false),
    ("2500", "Loan", AccountType::Liability, false),
    ("3000", "Capital A", AccountType::Equity, false),
    ("3001", "Capital B", AccountType::Equity, false),
    ("4000", "Sales", AccountType::Revenue, false),
    ("4100", "Services", AccountType::Revenue, false),
    ("6000", "Supplies", AccountType::Expense, true),
    ("6100", "Rent", AccountType::Expense, true),
];

fn rtype(code: &str) -> Option<AccountType> {
    RCHART.iter().find(|r| r.0 == code).map(|r| r.2)
}

#[derive(Clone, Debug)]
struct Gen {
    day: u32,
    mode: u8,
    picks: [usize; 3],
    x: i64,
    y: i64,
    /// Per-line class choice: 0 untagged, 1 class A, 2 class B.
    classes: [u8; 3],
}

impl Gen {
    fn date(&self) -> NaiveDate {
        day(self.day)
    }

    /// (chart index, side, cents) for each line; accounts distinct.
    fn legs(&self, scale: i64) -> Vec<(usize, Side, i64)> {
        let mut pool: Vec<usize> = (0..N).collect();
        let a = pool.remove(self.picks[0] % pool.len());
        let b = pool.remove(self.picks[1] % pool.len());
        let cc = pool.remove(self.picks[2] % pool.len());
        let (x, y) = (self.x * scale, self.y * scale);
        match self.mode {
            0 => vec![(a, DR, x), (b, CR, x)],
            1 => vec![(a, DR, x), (b, DR, y), (cc, CR, x + y)],
            _ => vec![(a, CR, x), (b, CR, y), (cc, DR, x + y)],
        }
    }
}

fn day(n: u32) -> NaiveDate {
    NaiveDate::from_yo_opt(2026, n + 1).unwrap()
}

fn prev(date: NaiveDate) -> NaiveDate {
    date.checked_sub_days(Days::new(1)).unwrap()
}

fn month_end(month: u32) -> NaiveDate {
    if month == 12 {
        d(2026, 12, 31)
    } else {
        prev(d(2026, month + 1, 1))
    }
}

fn arb_gen() -> impl Strategy<Value = Gen> {
    (
        0u32..365,
        0u8..3,
        (0usize..N, 0usize..N, 0usize..N),
        1i64..=1_000_000,
        1i64..=1_000_000,
        (0u8..3, 0u8..3, 0u8..3),
    )
        .prop_map(|(day, mode, (p0, p1, p2), x, y, (k0, k1, k2))| Gen {
            day,
            mode,
            picks: [p0, p1, p2],
            x,
            y,
            classes: [k0, k1, k2],
        })
}

fn arb_books() -> impl Strategy<Value = Vec<Gen>> {
    prop::collection::vec(arb_gen(), 0..18)
}

struct Books {
    l: Ledger,
    ids: [AccountId; N],
    entries: Vec<EntryId>,
    class_a: ClassId,
    class_b: ClassId,
}

fn build_ordered(gens: &[Gen], scale: i64, order: &[usize], tag: bool) -> Books {
    let mut l = Ledger::new(USD);
    let header = l
        .add_account(
            "6900",
            "Operating Expenses",
            AccountType::Expense,
            None,
            true,
        )
        .unwrap();
    let mut ids = [0; N];
    for &i in order {
        let (code, name, t, child) = RCHART[i];
        ids[i] = l
            .add_account(code, name, t, child.then_some(header), false)
            .unwrap();
    }
    let class_a = l.add_class("Alpha").unwrap();
    let class_b = l.add_class("Beta").unwrap();
    let mut entries = Vec::new();
    for g in gens {
        let e = l.create_entry(g.date(), "gen");
        for (n, (i, side, cents)) in g.legs(scale).into_iter().enumerate() {
            l.add_line(e, ids[i], side, usd(cents)).unwrap();
            if tag {
                let k = match g.classes[n] {
                    1 => Some(class_a),
                    2 => Some(class_b),
                    _ => None,
                };
                l.set_line_class(e, n, k).unwrap();
            }
        }
        l.post_entry(e).unwrap();
        entries.push(e);
    }
    Books {
        l,
        ids,
        entries,
        class_a,
        class_b,
    }
}

fn build(gens: &[Gen]) -> Books {
    build_ordered(gens, 1, &(0..N).collect::<Vec<_>>(), false)
}

fn build_scaled(gens: &[Gen], k: i64) -> Books {
    build_ordered(gens, k, &(0..N).collect::<Vec<_>>(), false)
}

fn members_close(b: &Books, share_a: Decimal) -> CloseTo {
    CloseTo::Members(vec![
        MemberClose {
            capital: b.ids[CAP_A],
            share: share_a,
            distributions: None,
        },
        MemberClose {
            capital: b.ids[CAP_B],
            share: Decimal::ONE - share_a,
            distributions: None,
        },
    ])
}

/// Close January through `month` as one period (if `month` is given).
fn close_through(b: &mut Books, month: Option<u32>, to: &CloseTo) {
    if let Some(m) = month {
        let p =
            b.l.create_period("closed", d(2026, 1, 1), month_end(m))
                .unwrap();
        b.l.close_period_to(p, to).unwrap();
    }
}

fn re_close() -> CloseTo {
    CloseTo::RetainedEarnings { sweep: vec![] }
}

/// Independent model: debits minus credits per account code over the
/// generated lines dated in [from, to] (closing entries are not generated
/// lines). Optionally only lines whose class choice matches.
fn model(
    gens: &[Gen],
    from: NaiveDate,
    to: NaiveDate,
    class: Option<u8>,
) -> BTreeMap<&'static str, Decimal> {
    let mut m = BTreeMap::new();
    for g in gens {
        if g.date() < from || g.date() > to {
            continue;
        }
        for (n, (i, side, cents)) in g.legs(1).into_iter().enumerate() {
            if class.is_some_and(|k| g.classes[n] != k) {
                continue;
            }
            let v = if side == DR { c(cents) } else { -c(cents) };
            *m.entry(RCHART[i].0).or_insert(Decimal::ZERO) += v;
        }
    }
    m
}

fn early() -> NaiveDate {
    d(2025, 1, 1)
}

/// Model P&L lines: (revenue lines, expense lines) on the normal side,
/// nonzero only.
type Lines = Vec<(String, Decimal)>;
fn model_pnl(m: &BTreeMap<&'static str, Decimal>) -> (Lines, Lines) {
    let mut rev = Vec::new();
    let mut exp = Vec::new();
    for (code, v) in m {
        match rtype(code) {
            Some(AccountType::Revenue) if !v.is_zero() => rev.push((code.to_string(), -*v)),
            Some(AccountType::Expense) if !v.is_zero() => exp.push((code.to_string(), *v)),
            _ => {}
        }
    }
    (rev, exp)
}

fn model_ni(m: &BTreeMap<&'static str, Decimal>) -> Decimal {
    let (r, e) = model_pnl(m);
    r.iter().map(|x| x.1).sum::<Decimal>() - e.iter().map(|x| x.1).sum::<Decimal>()
}

fn sum(lines: &[StatementLine]) -> Decimal {
    lines.iter().map(|l| l.amount).sum()
}

fn three_days() -> impl Strategy<Value = (u32, u32, u32)> {
    (0u32..365, 0u32..365, 0u32..365).prop_map(|(a, b, c)| {
        let mut v = [a, b, c];
        v.sort();
        (v[0], v[1], v[2])
    })
}

fn opt_month() -> impl Strategy<Value = Option<u32>> {
    prop::option::of(1u32..=12)
}

// ===========================================================================
// trial_balance / trial_balance_as_of — properties
// ===========================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Oracle: TB INVARIANT 1 / B-6 — total debits equal total credits, and
    /// each total is its column's sum, for any as-of date, with or without a
    /// close.
    #[test]
    fn prop_tb_columns_always_balance(gens in arb_books(), as_of in 0u32..365, month in opt_month()) {
        let mut b = build(&gens);
        close_through(&mut b, month, &re_close());
        for tb in [trial_balance_as_of(&b.l, day(as_of)), trial_balance(&b.l)] {
            prop_assert!(tb.balanced());
            prop_assert_eq!(tb.total_debits, tb.total_credits);
            prop_assert_eq!(tb.total_debits, tb.lines.iter().map(|l| l.debit).sum::<Decimal>());
            prop_assert_eq!(tb.total_credits, tb.lines.iter().map(|l| l.credit).sum::<Decimal>());
        }
    }

    /// Oracle: TB INVARIANTS 2-3 — lines strictly in code order, each in
    /// exactly one column, positive, never a placeholder.
    #[test]
    fn prop_tb_lines_ordered_one_column_no_placeholder(gens in arb_books(), as_of in 0u32..365, month in opt_month()) {
        let mut b = build(&gens);
        close_through(&mut b, month, &re_close());
        let tb = trial_balance_as_of(&b.l, day(as_of));
        for w in tb.lines.windows(2) {
            prop_assert!(w[0].code < w[1].code);
        }
        for l in &tb.lines {
            prop_assert!(l.code != "6900");
            prop_assert!(l.debit >= Decimal::ZERO && l.credit >= Decimal::ZERO);
            prop_assert!(l.debit.is_zero() != l.credit.is_zero(), "{:?}", l);
        }
    }

    /// Oracle: TB INVARIANTS 2-4 against the independent model — exactly the
    /// accounts with a nonzero net of posted lines dated <= as_of, net on
    /// its side.
    #[test]
    fn prop_tb_matches_net_balance_model(gens in arb_books(), as_of in 0u32..365) {
        let b = build(&gens);
        let tb = trial_balance_as_of(&b.l, day(as_of));
        let want: Vec<(String, Decimal, Decimal)> = model(&gens, early(), day(as_of), None)
            .into_iter()
            .filter(|(_, v)| !v.is_zero())
            .map(|(k, v)| if v > Decimal::ZERO { (k.to_string(), v, Decimal::ZERO) } else { (k.to_string(), Decimal::ZERO, -v) })
            .collect();
        prop_assert_eq!(tb_lines(&tb), want);
    }

    /// Oracle: trial_balance INVARIANT 1 — equals trial_balance_as_of on or
    /// after the last entry's date (with or without a close).
    #[test]
    fn prop_tb_full_equals_as_of_last_date(gens in arb_books(), month in opt_month()) {
        let mut b = build(&gens);
        close_through(&mut b, month, &re_close());
        let full = tb_lines(&trial_balance(&b.l));
        prop_assert_eq!(&full, &tb_lines(&trial_balance_as_of(&b.l, d(2026, 12, 31))));
        prop_assert_eq!(&full, &tb_lines(&trial_balance_as_of(&b.l, d(2030, 1, 1))));
        let last = gens.iter().map(|g| g.date()).chain(month.map(month_end)).max();
        if let Some(last) = last {
            prop_assert_eq!(&full, &tb_lines(&trial_balance_as_of(&b.l, last)));
        }
    }

    /// Oracle: metamorphic scaling — every amount times k scales every TB
    /// column by k.
    #[test]
    fn prop_tb_scales_linearly(gens in arb_books(), as_of in 0u32..365, k in 2i64..=7) {
        let one = trial_balance_as_of(&build(&gens).l, day(as_of));
        let many = trial_balance_as_of(&build_scaled(&gens, k).l, day(as_of));
        let kk = Decimal::from(k);
        let want: Vec<_> = tb_lines(&one).into_iter().map(|(c0, a, b)| (c0, a * kk, b * kk)).collect();
        prop_assert_eq!(tb_lines(&many), want);
        prop_assert_eq!(many.total_debits, one.total_debits * kk);
    }

    /// Oracle: B-9 metamorphic — adding an entry and its reversal (same
    /// date) leaves TB and P&L unchanged at every date.
    #[test]
    fn prop_entry_plus_reversal_leaves_tb_and_pnl_unchanged(gens in arb_books(), extra in arb_gen(), as_of in 0u32..365) {
        let mut b = build(&gens);
        let tb0 = tb_lines(&trial_balance_as_of(&b.l, day(as_of)));
        let pl0 = pnl_key(&profit_and_loss_for_period(&b.l, d(2026, 1, 1), day(as_of), None).unwrap());
        let e = b.l.create_entry(extra.date(), "extra");
        for (i, side, cents) in extra.legs(1) {
            b.l.add_line(e, b.ids[i], side, usd(cents)).unwrap();
        }
        b.l.post_entry(e).unwrap();
        b.l.reverse_entry(e, extra.date(), "undo").unwrap();
        prop_assert_eq!(tb_lines(&trial_balance_as_of(&b.l, day(as_of))), tb0);
        let pl1 = profit_and_loss_for_period(&b.l, d(2026, 1, 1), day(as_of), None).unwrap();
        prop_assert_eq!(sl_nonzero(&pl1.revenue), pl0.0.into_iter().filter(|x| !x.1.is_zero()).collect::<Vec<_>>());
        prop_assert_eq!(sl_nonzero(&pl1.expenses), pl0.1.into_iter().filter(|x| !x.1.is_zero()).collect::<Vec<_>>());
        prop_assert_eq!(pl1.total_revenue, pl0.2);
        prop_assert_eq!(pl1.total_expenses, pl0.3);
    }

    /// Oracle: NEVER-count-drafts clauses (TB 4, P&L 2, BS 2, GL 4, CF via
    /// P&L, snapshot) — adding drafts changes no report.
    #[test]
    fn prop_drafts_change_no_report(gens in arb_books(), drafts in prop::collection::vec(arb_gen(), 1..5), (a, _m, e) in three_days()) {
        let mut b = build(&gens);
        let (s0, e0) = (day(a), day(e));
        let cfp = CashFlowParams { start: s0, end: e0, cash_accounts: vec![b.ids[CASH]], financing_accounts: vec![], investing_accounts: vec![] };
        let tb = tb_lines(&trial_balance_as_of(&b.l, e0));
        let pl = pnl_key(&profit_and_loss_for_period(&b.l, s0, e0, None).unwrap());
        let bs = bs_key(&balance_sheet_as_of(&b.l, e0).unwrap());
        let gl = gl_key(&general_ledger(&b.l, s0, e0).unwrap());
        let cf = cf_key(&cash_flow(&b.l, &cfp).unwrap());
        let snap = snapshot(&b.l).unwrap();
        for g in &drafts {
            let id = b.l.create_entry(g.date(), "draft");
            for (n, (i, side, cents)) in g.legs(1).into_iter().enumerate() {
                if g.mode == 0 && n == 1 { continue; } // some drafts unbalanced
                b.l.add_line(id, b.ids[i], side, usd(cents)).unwrap();
            }
        }
        prop_assert_eq!(tb_lines(&trial_balance_as_of(&b.l, e0)), tb);
        prop_assert_eq!(pnl_key(&profit_and_loss_for_period(&b.l, s0, e0, None).unwrap()), pl);
        prop_assert_eq!(bs_key(&balance_sheet_as_of(&b.l, e0).unwrap()), bs);
        prop_assert_eq!(gl_key(&general_ledger(&b.l, s0, e0).unwrap()), gl);
        prop_assert_eq!(cf_key(&cash_flow(&b.l, &cfp).unwrap()), cf);
        prop_assert_eq!(snapshot(&b.l).unwrap(), snap);
    }
}

// ===========================================================================
// profit_and_loss_for_period — properties
// ===========================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Oracle: P&L INVARIANT 3 — partition: NI[a, c] = NI[a, m] + NI[m+1, c]
    /// (also per-total), with or without a close.
    #[test]
    fn prop_pnl_partitions(gens in arb_books(), (a, m, e) in three_days(), month in opt_month()) {
        prop_assume!(m < e);
        let mut b = build(&gens);
        close_through(&mut b, month, &re_close());
        let whole = profit_and_loss_for_period(&b.l, day(a), day(e), None).unwrap();
        let left = profit_and_loss_for_period(&b.l, day(a), day(m), None).unwrap();
        let right = profit_and_loss_for_period(&b.l, day(m + 1), day(e), None).unwrap();
        prop_assert_eq!(whole.net_income(), left.net_income() + right.net_income());
        prop_assert_eq!(whole.total_revenue, left.total_revenue + right.total_revenue);
        prop_assert_eq!(whole.total_expenses, left.total_expenses + right.total_expenses);
    }

    /// Oracle: P&L INVARIANTS 1-2 against the model — a line per revenue /
    /// expense account with nonzero normal-side movement in the range, in
    /// code order; totals are line sums; NI = revenue - expenses; closing
    /// entries never counted (model has none).
    #[test]
    fn prop_pnl_matches_model_and_excludes_closes((a, _m, e) in three_days(), gens in arb_books(), month in opt_month()) {
        let mut b = build(&gens);
        close_through(&mut b, month, &re_close());
        let p = profit_and_loss_for_period(&b.l, day(a), day(e), None).unwrap();
        let (rev, exp) = model_pnl(&model(&gens, day(a), day(e), None));
        prop_assert_eq!(sl_nonzero(&p.revenue), rev);
        prop_assert_eq!(sl_nonzero(&p.expenses), exp);
        for l in &p.revenue { prop_assert_eq!(rtype(&l.code), Some(AccountType::Revenue)); }
        for l in &p.expenses { prop_assert_eq!(rtype(&l.code), Some(AccountType::Expense)); }
        for w in p.revenue.windows(2) { prop_assert!(w[0].code < w[1].code); }
        for w in p.expenses.windows(2) { prop_assert!(w[0].code < w[1].code); }
        prop_assert_eq!(p.total_revenue, sum(&p.revenue));
        prop_assert_eq!(p.total_expenses, sum(&p.expenses));
        prop_assert_eq!(p.net_income(), p.total_revenue - p.total_expenses);
    }

    /// Oracle: P&L INVARIANT 2 / SCENARIO 1 — closing a period changes no
    /// period P&L (books built twice, one closed).
    #[test]
    fn prop_pnl_unchanged_by_close(gens in arb_books(), month in 1u32..=12, (a, _m, e) in three_days()) {
        let open = build(&gens);
        let mut closed = build(&gens);
        close_through(&mut closed, Some(month), &re_close());
        for (s0, e0) in [(day(a), day(e)), (d(2026, 1, 1), month_end(month)), (d(2026, 1, 1), d(2026, 12, 31))] {
            prop_assert_eq!(
                pnl_key(&profit_and_loss_for_period(&open.l, s0, e0, None).unwrap()),
                pnl_key(&profit_and_loss_for_period(&closed.l, s0, e0, None).unwrap())
            );
        }
    }

    /// Oracle: P&L INVARIANT 4 — with no close in the range, NI[s, e] equals
    /// current earnings at e minus current earnings the day before s.
    #[test]
    fn prop_pnl_equals_change_in_current_earnings(gens in arb_books(), (a, _m, e) in three_days()) {
        let b = build(&gens);
        let p = profit_and_loss_for_period(&b.l, day(a), day(e), None).unwrap();
        let ce_end = balance_sheet_as_of(&b.l, day(e)).unwrap().current_earnings;
        let ce_before = balance_sheet_as_of(&b.l, prev(day(a))).unwrap().current_earnings;
        prop_assert_eq!(p.net_income(), ce_end - ce_before);
    }

    /// Oracle: metamorphic scaling — every amount times k scales every P&L
    /// line and total by k.
    #[test]
    fn prop_pnl_scales_linearly(gens in arb_books(), (a, _m, e) in three_days(), k in 2i64..=7) {
        let kk = Decimal::from(k);
        let one = profit_and_loss_for_period(&build(&gens).l, day(a), day(e), None).unwrap();
        let many = profit_and_loss_for_period(&build_scaled(&gens, k).l, day(a), day(e), None).unwrap();
        let scale = |v: Vec<(String, Decimal)>| v.into_iter().map(|(c0, x)| (c0, x * kk)).collect::<Vec<_>>();
        prop_assert_eq!(sl_nonzero(&many.revenue), scale(sl_nonzero(&one.revenue)));
        prop_assert_eq!(sl_nonzero(&many.expenses), scale(sl_nonzero(&one.expenses)));
        prop_assert_eq!(many.net_income(), one.net_income() * kk);
    }

    /// Oracle: P&L INVARIANT 5 against the model — with a class, the
    /// statement is exactly the lines tagged with that class.
    #[test]
    fn prop_pnl_class_filter_matches_model(gens in arb_books(), (a, _m, e) in three_days()) {
        let b = build_ordered(&gens, 1, &(0..N).collect::<Vec<_>>(), true);
        for (k, id) in [(1u8, b.class_a), (2u8, b.class_b)] {
            let p = profit_and_loss_for_period(&b.l, day(a), day(e), Some(id)).unwrap();
            let (rev, exp) = model_pnl(&model(&gens, day(a), day(e), Some(k)));
            prop_assert_eq!(sl_nonzero(&p.revenue), rev);
            prop_assert_eq!(sl_nonzero(&p.expenses), exp);
        }
    }
}

// ===========================================================================
// profit_and_loss_by_class — properties
// ===========================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Oracle: by-class INVARIANTS 1-2 — conservation: the rows (with
    /// unclassified) sum to the whole P&L, each class row equals the model
    /// of its tagged lines, the unclassified row the untagged lines; rows by
    /// name, unclassified last.
    #[test]
    fn prop_pnl_by_class_rows_sum_to_whole(gens in arb_books()) {
        let b = build_ordered(&gens, 1, &(0..N).collect::<Vec<_>>(), true);
        let rows = profit_and_loss_by_class(&b.l).unwrap();
        let whole = unclosed_earnings(&b.l).unwrap();
        prop_assert_eq!(rows.iter().map(|r| r.profit_and_loss.revenue).sum::<Decimal>(), whole.revenue);
        prop_assert_eq!(rows.iter().map(|r| r.profit_and_loss.expenses).sum::<Decimal>(), whole.expenses);
        let (wr, we) = model_pnl(&model(&gens, early(), d(2030, 1, 1), None));
        prop_assert_eq!(whole.revenue, wr.iter().map(|x| x.1).sum::<Decimal>());
        prop_assert_eq!(whole.expenses, we.iter().map(|x| x.1).sum::<Decimal>());
        for r in &rows {
            let k = match &r.class {
                Some(cl) if cl.id == b.class_a => 1u8,
                Some(cl) if cl.id == b.class_b => 2,
                Some(_) => return Err(TestCaseError::fail("unknown class row")),
                None => 0,
            };
            let (mr, me) = model_pnl(&model(&gens, early(), d(2030, 1, 1), Some(k)));
            prop_assert_eq!(r.profit_and_loss.revenue, mr.iter().map(|x| x.1).sum::<Decimal>());
            prop_assert_eq!(r.profit_and_loss.expenses, me.iter().map(|x| x.1).sum::<Decimal>());
        }
        let names: Vec<Option<String>> = rows.iter().map(|r| r.class.as_ref().map(|k| k.name.clone())).collect();
        let mut tagged: Vec<Option<String>> = names.iter().filter(|n| n.is_some()).cloned().collect();
        tagged.sort();
        let unclassified = names.iter().filter(|n| n.is_none()).count();
        prop_assert!(unclassified <= 1);
        if unclassified == 1 { prop_assert!(names.last().unwrap().is_none()); }
        prop_assert_eq!(&names[..tagged.len()], &tagged[..]);
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Oracle: by-class INVARIANT 1 (conservation) — still holds after a
    /// period close: the rows sum to the whole P&L.
    #[test]
    fn prop_pnl_by_class_conserves_after_close(gens in arb_books(), month in 1u32..=12) {
        let mut b = build_ordered(&gens, 1, &(0..N).collect::<Vec<_>>(), true);
        close_through(&mut b, Some(month), &re_close());
        let rows = profit_and_loss_by_class(&b.l).unwrap();
        let whole = unclosed_earnings(&b.l).unwrap();
        prop_assert_eq!(rows.iter().map(|r| r.profit_and_loss.revenue).sum::<Decimal>(), whole.revenue);
        prop_assert_eq!(rows.iter().map(|r| r.profit_and_loss.expenses).sum::<Decimal>(), whole.expenses);
    }
}

// ===========================================================================
// balance_sheet_as_of — properties
// ===========================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Oracle: BS INVARIANT 1 / B-7 — assets = liabilities + equity (with
    /// current earnings), exactly; totals are line sums; never a revenue or
    /// expense line; with or without a close (RE or members).
    #[test]
    fn prop_bs_balances_on_every_date(gens in arb_books(), as_of in 0u32..365, month in opt_month(), members in any::<bool>()) {
        let mut b = build(&gens);
        let to = if members { members_close(&b, c(50)) } else { re_close() };
        close_through(&mut b, month, &to);
        let s0 = balance_sheet_as_of(&b.l, day(as_of)).unwrap();
        prop_assert!(s0.balanced());
        prop_assert_eq!(s0.total_assets, s0.total_liabilities + s0.total_equity);
        prop_assert_eq!(s0.total_assets, sum(&s0.assets));
        prop_assert_eq!(s0.total_liabilities, sum(&s0.liabilities));
        prop_assert_eq!(s0.total_equity, sum(&s0.equity) + s0.current_earnings);
        for l in s0.assets.iter().chain(&s0.liabilities).chain(&s0.equity) {
            prop_assert!(!matches!(rtype(&l.code), Some(AccountType::Revenue | AccountType::Expense)), "{:?}", l);
            prop_assert!(l.code != "6900");
        }
    }

    /// Oracle: BS INVARIANTS 2, 3, 5 against the model (no close) — each
    /// group's lines are the nonzero normal-side balances in code order;
    /// current earnings is revenue minus expenses through as_of.
    #[test]
    fn prop_bs_lines_match_model(gens in arb_books(), as_of in 0u32..365) {
        let b = build(&gens);
        let s0 = balance_sheet_as_of(&b.l, day(as_of)).unwrap();
        let m = model(&gens, early(), day(as_of), None);
        let group = |t: AccountType| -> Vec<(String, Decimal)> {
            m.iter()
                .filter(|(k, v)| rtype(k) == Some(t) && !v.is_zero())
                .map(|(k, v)| (k.to_string(), if debit_normal(t) { *v } else { -*v }))
                .collect()
        };
        prop_assert_eq!(sl_nonzero(&s0.assets), group(AccountType::Asset));
        prop_assert_eq!(sl_nonzero(&s0.liabilities), group(AccountType::Liability));
        prop_assert_eq!(sl_nonzero(&s0.equity), group(AccountType::Equity));
        prop_assert_eq!(s0.current_earnings, model_ni(&m));
    }

    /// Oracle: BS INVARIANTS 2-3 — current earnings is only income not yet
    /// closed: after a close through E, at as_of >= E it is income from E+1;
    /// before E it is all income to date.
    #[test]
    fn prop_bs_current_earnings_is_unclosed_income(gens in arb_books(), as_of in 0u32..365, month in 1u32..=12) {
        let mut b = build(&gens);
        close_through(&mut b, Some(month), &re_close());
        let s0 = balance_sheet_as_of(&b.l, day(as_of)).unwrap();
        let from = if day(as_of) >= month_end(month) { month_end(month).succ_opt().unwrap() } else { early() };
        prop_assert_eq!(s0.current_earnings, model_ni(&model(&gens, from, day(as_of), None)));
        if day(as_of) >= month_end(month) {
            let re: Decimal = s0.equity.iter().filter(|l| l.code == "3999").map(|l| l.amount).sum();
            prop_assert_eq!(re, model_ni(&model(&gens, early(), month_end(month), None)));
        }
    }

    /// Oracle: metamorphic scaling — every amount times k scales every BS
    /// line and total by k.
    #[test]
    fn prop_bs_scales_linearly(gens in arb_books(), as_of in 0u32..365, k in 2i64..=7) {
        let kk = Decimal::from(k);
        let one = balance_sheet_as_of(&build(&gens).l, day(as_of)).unwrap();
        let many = balance_sheet_as_of(&build_scaled(&gens, k).l, day(as_of)).unwrap();
        let scale = |v: Vec<(String, Decimal)>| v.into_iter().map(|(c0, x)| (c0, x * kk)).collect::<Vec<_>>();
        prop_assert_eq!(sl_nonzero(&many.assets), scale(sl_nonzero(&one.assets)));
        prop_assert_eq!(sl_nonzero(&many.liabilities), scale(sl_nonzero(&one.liabilities)));
        prop_assert_eq!(sl_nonzero(&many.equity), scale(sl_nonzero(&one.equity)));
        prop_assert_eq!(many.current_earnings, one.current_earnings * kk);
        prop_assert_eq!(many.total_assets, one.total_assets * kk);
    }
}

// ===========================================================================
// general_ledger — properties
// ===========================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Oracle: GL INVARIANTS 1-2 — each account rolls forward (beginning +
    /// normal-side postings = running balances = ending), totals are posting
    /// sums, postings in range in (date, entry) order, one column each;
    /// ending ties to the TB at `end` and beginning to the TB the day
    /// before `start`; accounts in strict code order, every TB account at
    /// `end` listed.
    #[test]
    fn prop_gl_rolls_forward_and_ties_to_tb(gens in arb_books(), (a, _m, e) in three_days(), month in opt_month()) {
        let mut b = build(&gens);
        close_through(&mut b, month, &re_close());
        let (s0, e0) = (day(a), day(e));
        let g = general_ledger(&b.l, s0, e0).unwrap();
        let tb_end: BTreeMap<String, (Decimal, Decimal)> = trial_balance_as_of(&b.l, e0).lines.into_iter().map(|l| (l.code, (l.debit, l.credit))).collect();
        let tb_beg: BTreeMap<String, (Decimal, Decimal)> = trial_balance_as_of(&b.l, prev(s0)).lines.into_iter().map(|l| (l.code, (l.debit, l.credit))).collect();
        for w in g.accounts.windows(2) { prop_assert!(w[0].code < w[1].code); }
        let listed: BTreeSet<&str> = g.accounts.iter().map(|x| x.code.as_str()).collect();
        for code in tb_end.keys().chain(tb_beg.keys()) { prop_assert!(listed.contains(code.as_str()), "missing {}", code); }
        for acct in &g.accounts {
            let dn = debit_normal(acct.account_type);
            let normal = |dr: Decimal, cr: Decimal| if dn { dr - cr } else { cr - dr };
            let mut bal = acct.beginning_balance;
            let (mut td, mut tc) = (Decimal::ZERO, Decimal::ZERO);
            let mut last: Option<(NaiveDate, EntryId)> = None;
            for p in &acct.postings {
                prop_assert!(p.date >= s0 && p.date <= e0);
                prop_assert!(p.debit >= Decimal::ZERO && p.credit >= Decimal::ZERO);
                prop_assert!(p.debit.is_zero() != p.credit.is_zero());
                if let Some(l) = last { prop_assert!(l <= (p.date, p.entry_id)); }
                last = Some((p.date, p.entry_id));
                bal += normal(p.debit, p.credit);
                prop_assert_eq!(p.balance, bal);
                td += p.debit;
                tc += p.credit;
            }
            prop_assert_eq!(acct.total_debits, td);
            prop_assert_eq!(acct.total_credits, tc);
            prop_assert_eq!(acct.ending_balance, bal);
            let (ed, ec) = tb_end.get(&acct.code).copied().unwrap_or_default();
            prop_assert_eq!(acct.ending_balance, normal(ed, ec));
            let (bd, bc) = tb_beg.get(&acct.code).copied().unwrap_or_default();
            prop_assert_eq!(acct.beginning_balance, normal(bd, bc));
        }
    }

    /// Oracle: GL INVARIANT 3 against the generated entries — each posting
    /// is one generated line in range, with split "CODE Name" of the one
    /// account on the other side, or "-Split-" when that side has several.
    #[test]
    fn prop_gl_postings_and_splits_match_entries(gens in arb_books(), (a, _m, e) in three_days()) {
        let b = build(&gens);
        let (s0, e0) = (day(a), day(e));
        let g = general_ledger(&b.l, s0, e0).unwrap();
        // Expected postings: (code, entry, debit, credit, split).
        let mut want: Vec<(String, EntryId, Decimal, Decimal, String)> = Vec::new();
        for (g0, &id) in gens.iter().zip(&b.entries) {
            if g0.date() < s0 || g0.date() > e0 { continue; }
            let legs = g0.legs(1);
            for &(i, side, cents) in &legs {
                let others: Vec<usize> = legs.iter().filter(|l| l.1 != side).map(|l| l.0).collect();
                let split = if others.len() == 1 { format!("{} {}", RCHART[others[0]].0, RCHART[others[0]].1) } else { "-Split-".to_string() };
                let (dr, cr) = if side == DR { (c(cents), Decimal::ZERO) } else { (Decimal::ZERO, c(cents)) };
                want.push((RCHART[i].0.to_string(), id, dr, cr, split));
            }
        }
        let mut got: Vec<(String, EntryId, Decimal, Decimal, String)> = g.accounts.iter()
            .flat_map(|acct| acct.postings.iter().map(move |p| (acct.code.clone(), p.entry_id, p.debit, p.credit, p.split.clone())))
            .collect();
        want.sort();
        got.sort();
        prop_assert_eq!(got, want);
    }
}

// ===========================================================================
// members_equity — properties
// ===========================================================================

fn me_members(b: &Books, share_a: Decimal) -> Vec<MemberCapital> {
    vec![
        MemberCapital {
            name: "A".into(),
            capital_account: b.ids[CAP_A],
            distribution_account: None,
            share: share_a,
        },
        MemberCapital {
            name: "B".into(),
            capital_account: b.ids[CAP_B],
            distribution_account: None,
            share: Decimal::ONE - share_a,
        },
    ]
}

fn rolls(col: &MemberEquityColumn) -> bool {
    col.beginning + col.contributions + col.net_income - col.distributions + col.other == col.ending
}

fn total_is_sum(st: &MembersEquityStatement) -> bool {
    let f = |g: fn(&MemberEquityColumn) -> Decimal| st.members.iter().map(g).sum::<Decimal>();
    st.total.beginning == f(|x| x.beginning)
        && st.total.contributions == f(|x| x.contributions)
        && st.total.net_income == f(|x| x.net_income)
        && st.total.distributions == f(|x| x.distributions)
        && st.total.other == f(|x| x.other)
        && st.total.ending == f(|x| x.ending)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Oracle: ME INVARIANTS 1-2 — every row (and the total) rolls forward
    /// exactly; the total is the sum of the members; total ending equals BS
    /// equity at `end`, total beginning equals BS equity the day before
    /// `start`; with or without a members close.
    #[test]
    fn prop_me_rolls_forward_and_ties_to_bs(gens in arb_books(), (a, _m, e) in three_days(), pct in 1i64..100, month in opt_month()) {
        let mut b = build(&gens);
        let share_a = Decimal::new(pct, 2);
        let to = members_close(&b, share_a);
        close_through(&mut b, month, &to);
        let st = members_equity(&b.l, day(a), day(e), &me_members(&b, share_a)).unwrap();
        prop_assert_eq!(st.members.len(), 2);
        prop_assert_eq!(st.members[0].name.as_str(), "A");
        prop_assert_eq!(st.members[1].name.as_str(), "B");
        for col in st.members.iter().chain(std::iter::once(&st.total)) { prop_assert!(rolls(col), "{:?}", col); }
        prop_assert!(total_is_sum(&st), "{:?}", st);
        prop_assert_eq!(st.total.ending, balance_sheet_as_of(&b.l, day(e)).unwrap().total_equity);
        prop_assert_eq!(st.total.beginning, balance_sheet_as_of(&b.l, prev(day(a))).unwrap().total_equity);
    }

    /// Oracle: ME INVARIANTS 3-3b — the members' incomes sum to the period's
    /// income (closing entries excluded: the model), and each is within
    /// less than two cents of its exact share: under one cent of rounding
    /// at each end of the cumulative split (docs/testing.md #7: a member's
    /// running balance, what Form 1065 K-1 Item L reports, chains exactly;
    /// see `each_members_capital_chains_across_statements`).
    #[test]
    fn prop_me_period_income_split_by_share(gens in arb_books(), (a, _m, e) in three_days(), pct in 1i64..100, month in opt_month()) {
        let mut b = build(&gens);
        let share_a = Decimal::new(pct, 2);
        let to = members_close(&b, share_a);
        close_through(&mut b, month, &to);
        let st = members_equity(&b.l, day(a), day(e), &me_members(&b, share_a)).unwrap();
        let ni = model_ni(&model(&gens, day(a), day(e), None));
        prop_assert_eq!(st.total.net_income, ni);
        let cent = c(1);
        for (col, share) in st.members.iter().zip([share_a, Decimal::ONE - share_a]) {
            prop_assert!((col.net_income - ni * share).abs() < cent + cent, "{:?} share {}", col, share);
        }
    }
}

// ===========================================================================
// cash_flow — properties
// ===========================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Oracle: CF INVARIANTS 1-2, 4 — operating + investing + financing =
    /// net change = ending - beginning cash (model), NI is the period P&L
    /// (model, closes excluded), section totals are their line sums, CFO =
    /// NI + depreciation + working capital; with or without a close and a
    /// designated loan.
    ///
    /// With no registered fixed assets and no investing accounts (docs/testing.md #8),
    /// depreciation and investing
    /// are zero (receivables are current, so working capital).
    #[test]
    fn prop_cf_sections_sum_to_change_in_cash(gens in arb_books(), (a, _m, e) in three_days(), month in opt_month(), designate in any::<bool>()) {
        let mut b = build(&gens);
        close_through(&mut b, month, &re_close());
        let params = CashFlowParams {
            start: day(a),
            end: day(e),
            cash_accounts: vec![b.ids[CASH]],
            financing_accounts: if designate { vec![b.ids[LOAN]] } else { vec![] },
            investing_accounts: vec![],
        };
        let cf = cash_flow(&b.l, &params).unwrap();
        prop_assert_eq!(cf.cash_from_operations + cf.cash_from_investing + cf.cash_from_financing, cf.net_change_in_cash);
        prop_assert_eq!(cf.cash_ending - cf.cash_beginning, cf.net_change_in_cash);
        let beg = model(&gens, early(), prev(day(a)), None).get("1000").copied().unwrap_or_default();
        let end = model(&gens, early(), day(e), None).get("1000").copied().unwrap_or_default();
        prop_assert_eq!(cf.cash_beginning, beg);
        prop_assert_eq!(cf.cash_ending, end);
        prop_assert_eq!(cf.net_income, model_ni(&model(&gens, day(a), day(e), None)));
        prop_assert_eq!(cf.depreciation, Decimal::ZERO);
        prop_assert_eq!(cf.cash_from_investing, Decimal::ZERO);
        let wc: Decimal = cf.working_capital.iter().map(|l| l.amount).sum();
        prop_assert_eq!(cf.cash_from_operations, cf.net_income + cf.depreciation + wc);
        prop_assert_eq!(cf.cash_from_investing, cf.investing.iter().map(|l| l.amount).sum::<Decimal>());
        prop_assert_eq!(cf.cash_from_financing, cf.financing.iter().map(|l| l.amount).sum::<Decimal>());
    }

    /// Oracle: CF INVARIANT 4 against the model — financing is the change in
    /// equity accounts (other than earnings) plus designated debt; the rest
    /// of the non-cash balance sheet is working capital.
    #[test]
    fn prop_cf_financing_is_equity_plus_designated_debt(gens in arb_books(), (a, _m, e) in three_days(), designate in any::<bool>()) {
        let b = build(&gens);
        let params = CashFlowParams {
            start: day(a),
            end: day(e),
            cash_accounts: vec![b.ids[CASH]],
            financing_accounts: if designate { vec![b.ids[LOAN]] } else { vec![] },
            investing_accounts: vec![],
        };
        let cf = cash_flow(&b.l, &params).unwrap();
        let m = model(&gens, day(a), day(e), None);
        let credit = |code: &str| -m.get(code).copied().unwrap_or_default();
        let mut fin = credit("3000") + credit("3001");
        if designate { fin += credit("2500"); }
        prop_assert_eq!(cf.cash_from_financing, fin);
    }

    /// Oracle: CF INVARIANT 5 — closing a period changes no cash flow
    /// statement (books built twice, one closed).
    #[test]
    fn prop_cf_unchanged_by_close(gens in arb_books(), month in 1u32..=12, (a, _m, e) in three_days()) {
        let open = build(&gens);
        let mut closed = build(&gens);
        close_through(&mut closed, Some(month), &re_close());
        let p = |b: &Books| CashFlowParams { start: day(a), end: day(e), cash_accounts: vec![b.ids[CASH]], financing_accounts: vec![b.ids[LOAN]], investing_accounts: vec![] };
        let x = cash_flow(&open.l, &p(&open)).unwrap();
        let y = cash_flow(&closed.l, &p(&closed)).unwrap();
        prop_assert_eq!(x.net_income, y.net_income);
        prop_assert_eq!(x.cash_from_operations, y.cash_from_operations);
        prop_assert_eq!(x.cash_from_investing, y.cash_from_investing);
        prop_assert_eq!(x.cash_from_financing, y.cash_from_financing);
        prop_assert_eq!(x.net_change_in_cash, y.net_change_in_cash);
    }

    /// Oracle: metamorphic scaling — every amount times k scales every cash
    /// flow total by k.
    #[test]
    fn prop_cf_scales_linearly(gens in arb_books(), (a, _m, e) in three_days(), k in 2i64..=7) {
        let kk = Decimal::from(k);
        let one = build(&gens);
        let many = build_scaled(&gens, k);
        let p = |b: &Books| CashFlowParams { start: day(a), end: day(e), cash_accounts: vec![b.ids[CASH]], financing_accounts: vec![], investing_accounts: vec![] };
        let x = cash_flow(&one.l, &p(&one)).unwrap();
        let y = cash_flow(&many.l, &p(&many)).unwrap();
        prop_assert_eq!(y.net_income, x.net_income * kk);
        prop_assert_eq!(y.cash_from_operations, x.cash_from_operations * kk);
        prop_assert_eq!(y.cash_from_financing, x.cash_from_financing * kk);
        prop_assert_eq!(y.cash_beginning, x.cash_beginning * kk);
        prop_assert_eq!(y.cash_ending, x.cash_ending * kk);
    }
}

// ===========================================================================
// snapshot — properties
// ===========================================================================

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// Oracle: snapshot INVARIANT 1 — identical JSON whatever order the
    /// accounts were added in (ids differ, books the same), with or without
    /// a close.
    #[test]
    fn prop_snapshot_invariant_under_account_order(
        gens in arb_books(),
        order in Just((0..N).collect::<Vec<usize>>()).prop_shuffle(),
        month in opt_month(),
    ) {
        let mut x = build(&gens);
        let mut y = build_ordered(&gens, 1, &order, false);
        close_through(&mut x, month, &re_close());
        close_through(&mut y, month, &re_close());
        prop_assert_eq!(snapshot(&x.l).unwrap(), snapshot(&y.l).unwrap());
    }

    /// Oracle: snapshot INVARIANT 2 — never a float anywhere in the JSON.
    #[test]
    fn prop_snapshot_has_no_floats(gens in arb_books(), month in opt_month()) {
        let mut b = build(&gens);
        close_through(&mut b, month, &re_close());
        let snap = snapshot(&b.l).unwrap();
        prop_assert!(!has_float(&snap), "{}", snap);
    }
}
