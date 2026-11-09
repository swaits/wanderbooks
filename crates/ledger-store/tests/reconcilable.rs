//! Which accounts the monthly close reconciles, and when one is due
//! (`ledger_store::reconcilable`; plan P14 unit V-D2). The server's
//! `books.digest` and the viewer's Needs attention both read this module,
//! so the store's own suite checks it: coverage found it at 0% there.
//!
//! ```text
//! GOAL: the monthly close's reconciliation step can't be forgotten, and
//!       the digest and the viewer never disagree on what is due.
//! SOURCE: docs/parties-and-attachments.md "Reconciliations" (Needs
//!         attention: an account with activity not reconciled through last
//!         month's end); CONVENTION (bank and card statements close
//!         monthly), both as the module's own spec block states them.
//!         No BS- or WB- id covers the module yet; the report of this
//!         unit says so.
//! ```
//!
//! Every test builds real books in a `Store`, records real statements and
//! completes real reconciliations through the public API, then asks the
//! module. Expected values are worked by hand from the two invariants of
//! the module's spec block.

use chrono::NaiveDate;
use ledger_core::{AccountId, AccountType, Currency, EntryId, InstrumentKind, Money, Side};
use ledger_store::Store;
use ledger_store::reconcilable::{
    CASH_WORDS, card_accounts, cash_accounts, last_month_end, reconcilable_accounts,
    reconciled_through, reconciliation_due,
};
use ledger_store::store::{Reconciliation, Statement};
use rust_decimal::Decimal;
use std::path::PathBuf;
use std::str::FromStr;

/// A test's dir, removed when the test ends, pass or panic.
struct TestDir(PathBuf);

impl std::ops::Deref for TestDir {
    type Target = std::path::Path;
    fn deref(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn test_dir(name: &str) -> TestDir {
    let dir = std::env::temp_dir().join(format!("wb-reconcilable-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    TestDir(dir)
}

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

fn usd(s: &str) -> Money {
    Money::new(dec(s), Currency::USD)
}

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

fn post(
    s: &mut Store,
    d: NaiveDate,
    desc: &str,
    dr: AccountId,
    cr: AccountId,
    amt: &str,
) -> EntryId {
    let e = s.create_entry(d, desc).unwrap();
    s.add_line(e, dr, Side::Debit, usd(amt)).unwrap();
    s.add_line(e, cr, Side::Credit, usd(amt)).unwrap();
    s.post_entry(e).unwrap();
    e
}

fn account(s: &mut Store, code: &str, name: &str, t: AccountType, placeholder: bool) -> AccountId {
    s.add_account(code, name, t, None, placeholder).unwrap()
}

fn file(s: &mut Store, name: &str) -> u64 {
    s.file_document(
        "accounting",
        name,
        "application/pdf",
        format!("%PDF {name}").as_bytes(),
        Some(name),
    )
    .unwrap()
    .0
}

fn statement(
    s: &mut Store,
    name: &str,
    account: AccountId,
    start: NaiveDate,
    end: NaiveDate,
    opening: &str,
    closing: &str,
) -> u64 {
    let attachment_id = file(s, name);
    s.record_statement(Statement {
        attachment_id,
        account_id: account,
        period_start: start,
        period_end: end,
        opening_balance: dec(opening),
        closing_balance: dec(closing),
    })
    .unwrap();
    attachment_id
}

/// What the module reads: the live ledger, statements and reconciliations.
fn due(s: &Store, account: AccountId, today: NaiveDate) -> Option<NaiveDate> {
    reconciliation_due(
        s.ledger(),
        &s.reconciliations(None).unwrap(),
        account,
        today,
    )
}

fn names(s: &Store, ids: &[AccountId]) -> Vec<String> {
    ids.iter()
        .map(|&i| s.ledger().chart().get(i).unwrap().name.clone())
        .collect()
}

// ---------------------------------------------------------------- the cash accounts

/// DEFINITIONS, cash accounts: with none configured, the asset accounts
/// (not placeholders) whose names say they hold cash, in chart order. A
/// name is matched in any case; an expense called "Bank Fees", a
/// placeholder group and an asset that holds no cash are not cash.
#[test]
fn cash_accounts_are_the_assets_whose_names_say_cash() {
    let dir = test_dir("cash-by-name");
    let mut s = Store::create(&dir, Currency::USD).unwrap();
    let checking = account(
        &mut s,
        "1000",
        "Business CHECKING",
        AccountType::Asset,
        false,
    );
    let _group = account(
        &mut s,
        "1050",
        "Cash and Bank (group)",
        AccountType::Asset,
        true,
    );
    let _equipment = account(&mut s, "1500", "Equipment", AccountType::Asset, false);
    let _fees = account(&mut s, "6500", "Bank Fees", AccountType::Expense, false);
    let _loan = account(&mut s, "2500", "Bank Loan", AccountType::Liability, false);
    let savings = account(&mut s, "1010", "Savings", AccountType::Asset, false);
    let petty = account(&mut s, "1020", "Petty cash box", AccountType::Asset, false);
    let mm = account(
        &mut s,
        "1030",
        "Money Market fund",
        AccountType::Asset,
        false,
    );

    assert_eq!(CASH_WORDS.len(), 5);
    let mut found = cash_accounts(s.ledger(), &[]);
    // Which accounts, not in which order: the order is the finding below.
    found.sort_unstable();
    assert_eq!(found, vec![checking, savings, petty, mm]);
    assert_eq!(
        names(&s, &found),
        [
            "Business CHECKING",
            "Savings",
            "Petty cash box",
            "Money Market fund"
        ]
    );
}

/// FINDING (V-D2): the cash accounts found by name come out in the order
/// the chart's `HashMap` yields them (`ChartOfAccounts::all` says "order
/// unspecified"), not in chart order as `card_accounts` does (ascending
/// account id). `books.digest`'s `unreconciled_accounts` lists accounts in
/// this order, so with two or more cash accounts the digest's list can
/// differ between runs of the same books. The module's INVARIANT 1 names
/// the order of the three groups, not within one, so this is a gap in the
/// rule rather than a broken one; the expectation here is the order
/// `card_accounts` already keeps.
#[test]
#[ignore = "finding: cash_accounts yields name-matched accounts in HashMap order, so books.digest's unreconciled_accounts order is not stable; expected ascending account id as card_accounts keeps"]
fn cash_accounts_come_in_chart_order() {
    let dir = test_dir("cash-order");
    let mut s = Store::create(&dir, Currency::USD).unwrap();
    let mut expected = Vec::new();
    for (i, name) in [
        "Checking",
        "Savings",
        "Petty cash",
        "Money market",
        "Bank two",
        "Cash three",
    ]
    .iter()
    .enumerate()
    {
        expected.push(account(
            &mut s,
            &format!("10{i}0"),
            name,
            AccountType::Asset,
            false,
        ));
    }
    // Six accounts in a HashMap come out in any of 720 orders; ascending is one.
    assert_eq!(cash_accounts(s.ledger(), &[]), expected);
}

/// DEFINITIONS, cash accounts: `[books] cash_accounts` wins when it is set,
/// resolved by code in the order given; a code the chart lacks is dropped,
/// and a cash-named account that is not configured is not cash.
#[test]
fn configured_cash_accounts_win_by_code() {
    let dir = test_dir("cash-configured");
    let mut s = Store::create(&dir, Currency::USD).unwrap();
    let checking = account(&mut s, "1000", "Checking", AccountType::Asset, false);
    let drawer = account(&mut s, "1100", "Till", AccountType::Asset, false);
    let _savings = account(&mut s, "1010", "Savings", AccountType::Asset, false);

    let configured = vec!["1100".to_string(), "9999".to_string(), "1000".to_string()];
    assert_eq!(
        cash_accounts(s.ledger(), &configured),
        vec![drawer, checking]
    );
    // Only an unknown code configured: none, and the names are NOT then consulted.
    assert!(cash_accounts(s.ledger(), &["9999".to_string()]).is_empty());
}

// ---------------------------------------------------------------- the card accounts

/// DEFINITIONS, card accounts: while the books record no business-card
/// payment method, the liabilities whose names say "card" and not "gift"
/// (ledger_core::parties::card_accounts); once one is recorded, the
/// recorded cards are the cards and the names are no longer consulted.
#[test]
fn card_accounts_follow_the_recorded_method_else_the_name() {
    let dir = test_dir("cards");
    let mut s = Store::create(&dir, Currency::USD).unwrap();
    let corp_card = account(
        &mut s,
        "2050",
        "Corporate Card",
        AccountType::Liability,
        false,
    );
    let _gift = account(
        &mut s,
        "2060",
        "Gift Card Liability",
        AccountType::Liability,
        false,
    );
    let amex = account(
        &mut s,
        "2070",
        "Amex Platinum",
        AccountType::Liability,
        false,
    );
    let _asset_card = account(&mut s, "1900", "Card Reader", AccountType::Asset, false);

    assert_eq!(card_accounts(s.ledger()), vec![corp_card]);

    s.add_instrument(
        InstrumentKind::BusinessCard,
        "Amex Platinum",
        Some("1234"),
        None,
        amex,
    )
    .unwrap();
    assert_eq!(
        card_accounts(s.ledger()),
        vec![amex],
        "the recorded method decides; the name 'Corporate Card' no longer does"
    );
}

// ---------------------------------------------------------------- INVARIANT 1

/// INVARIANT 1: the accounts the books reconcile are the cash accounts,
/// then the card accounts, then every other liability with a recorded
/// statement, each once. A card is expected before its first statement; a
/// liability named otherwise joins once its first statement is recorded; an
/// asset with a statement but no cash name never does.
#[test]
fn reconcilable_accounts_are_cash_then_cards_then_liabilities_with_statements() {
    let dir = test_dir("reconcilable");
    let mut s = Store::create(&dir, Currency::USD).unwrap();
    let checking = account(&mut s, "1000", "Checking", AccountType::Asset, false);
    let equipment = account(&mut s, "1500", "Equipment", AccountType::Asset, false);
    let card = account(&mut s, "2050", "Visa Card", AccountType::Liability, false);
    let amex = account(&mut s, "2070", "Amex", AccountType::Liability, false);
    let capital = account(&mut s, "3000", "Capital", AccountType::Equity, false);
    post(
        &mut s,
        date(2026, 1, 2),
        "Open",
        checking,
        capital,
        "1000.00",
    );
    post(
        &mut s,
        date(2026, 1, 3),
        "Laptop",
        equipment,
        capital,
        "500.00",
    );
    post(
        &mut s,
        date(2026, 1, 4),
        "Amex buy",
        equipment,
        amex,
        "50.00",
    );

    let cash = cash_accounts(s.ledger(), &[]);
    let all = |s: &Store| reconcilable_accounts(s.ledger(), &cash, &s.statements(None).unwrap());
    // No statements: cash, then the card by name; the Amex is not yet.
    assert_eq!(all(&s), vec![checking, card]);

    // Statements for the Amex (a liability), the equipment (an asset) and
    // the card (already expected): only the Amex joins, and the card once.
    statement(
        &mut s,
        "amex.pdf",
        amex,
        date(2026, 1, 1),
        date(2026, 1, 31),
        "0.00",
        "50.00",
    );
    statement(
        &mut s,
        "equip.pdf",
        equipment,
        date(2026, 1, 1),
        date(2026, 1, 31),
        "0.00",
        "550.00",
    );
    statement(
        &mut s,
        "visa.pdf",
        card,
        date(2026, 1, 1),
        date(2026, 1, 31),
        "0.00",
        "0.00",
    );
    assert_eq!(all(&s), vec![checking, card, amex]);

    // The cash list comes first even when a statement is for a cash account.
    statement(
        &mut s,
        "chk.pdf",
        checking,
        date(2026, 1, 1),
        date(2026, 1, 31),
        "0.00",
        "1000.00",
    );
    assert_eq!(all(&s), vec![checking, card, amex]);
}

// ---------------------------------------------------------------- INVARIANT 2

/// The last day of the month before today's, across a leap year and a
/// year boundary: the due date the digest and the viewer both print.
#[test]
fn last_month_end_is_the_last_day_of_the_month_before() {
    assert_eq!(last_month_end(date(2026, 3, 15)), date(2026, 2, 28));
    assert_eq!(last_month_end(date(2024, 3, 1)), date(2024, 2, 29));
    assert_eq!(last_month_end(date(2026, 1, 1)), date(2025, 12, 31));
    assert_eq!(last_month_end(date(2026, 1, 31)), date(2025, 12, 31));
    assert_eq!(last_month_end(date(2026, 12, 31)), date(2026, 11, 30));
}

/// INVARIANT 2, the newest reconciliation not voided: the date an account
/// is reconciled through ignores voided ones and other accounts', and is
/// None when there is none.
#[test]
fn reconciled_through_is_the_newest_standing_reconciliation() {
    let rec = |id, account_id, d: NaiveDate, voided: Option<&str>| Reconciliation {
        id,
        account_id,
        statement_date: d,
        statement_balance: Decimal::ZERO,
        attachment_id: id,
        cleared: vec![],
        voided: voided.map(str::to_string),
    };
    let recs = vec![
        rec(1, 7, date(2026, 1, 31), None),
        rec(2, 7, date(2026, 2, 28), Some("wrong statement")),
        rec(3, 8, date(2026, 3, 31), None),
        rec(4, 7, date(2025, 12, 31), None),
    ];
    assert_eq!(reconciled_through(&recs, 7), Some(date(2026, 1, 31)));
    assert_eq!(reconciled_through(&recs, 8), Some(date(2026, 3, 31)));
    assert_eq!(reconciled_through(&recs, 9), None);
    assert_eq!(reconciled_through(&[], 7), None);
}

/// INVARIANT 2 end to end: an account is due iff it has a posted line dated
/// on or before last month's end and no standing reconciliation reaches
/// it. Worked by hand, with today = 2026-03-10 (so through = 2026-02-28):
///
/// - checking has January activity and nothing reconciled: due through Feb 28;
/// - reconciled through Jan 31 (statement 0.00 -> 380.00): Jan 31 < Feb 28, still due;
/// - reconciled through Feb 28 (380.00 -> 350.00 after the 30.00 February
///   purchase): not due;
/// - voiding that reconciliation makes it due again.
#[test]
fn an_account_is_due_until_a_reconciliation_reaches_last_months_end() {
    let dir = test_dir("due");
    let mut s = Store::create(&dir, Currency::USD).unwrap();
    let checking = account(&mut s, "1000", "Checking", AccountType::Asset, false);
    let capital = account(&mut s, "3000", "Capital", AccountType::Equity, false);
    let software = account(&mut s, "6000", "Software", AccountType::Expense, false);
    let e1 = post(
        &mut s,
        date(2026, 1, 1),
        "Opening",
        checking,
        capital,
        "500.00",
    );
    let e2 = post(
        &mut s,
        date(2026, 1, 15),
        "Tool",
        software,
        checking,
        "120.00",
    );
    let e3 = post(
        &mut s,
        date(2026, 2, 3),
        "Tool 2",
        software,
        checking,
        "30.00",
    );
    let today = date(2026, 3, 10);
    let through = date(2026, 2, 28);

    assert_eq!(due(&s, checking, today), Some(through));

    s.mark_cleared(e1, 0, true).unwrap();
    s.mark_cleared(e2, 1, true).unwrap();
    let jan = statement(
        &mut s,
        "jan.pdf",
        checking,
        date(2026, 1, 1),
        date(2026, 1, 31),
        "0.00",
        "380.00",
    );
    let rec_jan = s.complete_reconciliation(jan).unwrap();
    assert_eq!(rec_jan.statement_date, date(2026, 1, 31));
    assert_eq!(
        due(&s, checking, today),
        Some(through),
        "reconciled through Jan 31 is short of Feb 28"
    );

    s.mark_cleared(e3, 1, true).unwrap();
    let feb = statement(
        &mut s,
        "feb.pdf",
        checking,
        date(2026, 2, 1),
        date(2026, 2, 28),
        "380.00",
        "350.00",
    );
    let rec_feb = s.complete_reconciliation(feb).unwrap();
    assert_eq!(
        due(&s, checking, today),
        None,
        "reconciled through last month's end"
    );
    // The same books asked in April are due for March again.
    assert_eq!(due(&s, checking, date(2026, 4, 2)), Some(date(2026, 3, 31)));
    // A reconciliation dated after last month's end also satisfies it.
    assert_eq!(due(&s, checking, date(2026, 2, 10)), None);

    s.void_reconciliation(rec_feb.id, "statement was the wrong month")
        .unwrap();
    assert_eq!(
        due(&s, checking, today),
        Some(through),
        "a voided reconciliation reaches nothing; January's still stands but is short"
    );
    assert_eq!(
        reconciled_through(&s.reconciliations(None).unwrap(), checking),
        Some(date(2026, 1, 31))
    );
}

/// INVARIANT 2, "None otherwise (including activity only this month)": an
/// account whose only posted line is dated this month is not due; one
/// dated exactly last month's last day is; a draft line counts for
/// nothing; an account with no lines at all is not due.
#[test]
fn only_posted_activity_through_last_months_end_makes_an_account_due() {
    let dir = test_dir("activity");
    let mut s = Store::create(&dir, Currency::USD).unwrap();
    let a_this_month = account(&mut s, "1000", "Checking", AccountType::Asset, false);
    let a_edge = account(&mut s, "1010", "Savings", AccountType::Asset, false);
    let a_draft = account(&mut s, "1020", "Money Market", AccountType::Asset, false);
    let a_idle = account(&mut s, "1030", "Cash Box", AccountType::Asset, false);
    let capital = account(&mut s, "3000", "Capital", AccountType::Equity, false);
    let today = date(2026, 3, 10);

    post(
        &mut s,
        date(2026, 3, 1),
        "March only",
        a_this_month,
        capital,
        "10.00",
    );
    post(
        &mut s,
        date(2026, 2, 28),
        "Last day of Feb",
        a_edge,
        capital,
        "10.00",
    );
    let draft = s.create_entry(date(2026, 1, 5), "Not posted").unwrap();
    s.add_line(draft, a_draft, Side::Debit, usd("10.00"))
        .unwrap();
    s.add_line(draft, capital, Side::Credit, usd("10.00"))
        .unwrap();

    assert_eq!(due(&s, a_this_month, today), None);
    assert_eq!(due(&s, a_edge, today), Some(date(2026, 2, 28)));
    assert_eq!(due(&s, a_draft, today), None);
    assert_eq!(due(&s, a_idle, today), None);
    // Asked before March began, the edge account has no February behind it
    // yet: its last day is not on or before January's end.
    assert_eq!(due(&s, a_edge, date(2026, 2, 5)), None);
}
