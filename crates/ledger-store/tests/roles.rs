//! Spec-derived tests for control-role assignment in the store.
//!
//! Oracles: `docs/spec/rules.md` B-23 ("the assignment is journaled, so
//! replay never depends on a setting outside the journal"), the doc comment
//! on `Store::assign_role` (journaled as `role.assigned`, with the account
//! that filled the role before), and the `Ledger::assign_role` spec (a
//! refusal leaves the books unchanged). Expected values are worked by hand.

use chrono::NaiveDate;
use ledger_core::{
    AccountId, AccountType, CloseTo, Currency, DocumentLine, EntryId, Ledger, LedgerError, Money,
    RECEIVABLES, Side, control_accounts,
};
use ledger_store::journal::read_and_verify_journal;
use ledger_store::{JOURNAL_FILE, JournalWriter, Store, StoreError};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::str::FromStr;

/// A test's dir, removed when the test ends, pass or panic.
struct TestDir(PathBuf);

impl std::ops::Deref for TestDir {
    type Target = std::path::Path;
    fn deref(&self) -> &std::path::Path {
        &self.0
    }
}

impl AsRef<std::path::Path> for TestDir {
    fn as_ref(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn test_dir(name: &str) -> TestDir {
    let dir = std::env::temp_dir().join(format!("wb-roles-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    TestDir(dir)
}

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

fn line(desc: &str, amount: &str) -> DocumentLine {
    DocumentLine {
        description: desc.to_string(),
        amount: dec(amount),
    }
}

/// The journal's (type, payload) pairs, verified.
fn journal(dir: &Path) -> Vec<(String, Value)> {
    read_and_verify_journal(&dir.join(JOURNAL_FILE))
        .unwrap()
        .into_iter()
        .map(|l| (l.line_type, l.payload))
        .collect()
}

fn roles(l: &Ledger) -> Vec<(&'static str, Option<AccountId>, Option<AccountId>)> {
    control_accounts()
        .map(|c| (c.key, l.role_assignment(c), l.role_account(c)))
        .collect()
}

fn code_of(l: &Ledger, id: AccountId) -> String {
    l.chart().get(id).unwrap().code.clone()
}

fn entry_codes(l: &Ledger, e: EntryId) -> Vec<(String, Side, Decimal)> {
    let mut v: Vec<_> = l
        .get_entry(e)
        .unwrap()
        .lines()
        .iter()
        .map(|x| (code_of(l, x.account), x.side, x.amount.amount))
        .collect();
    v.sort_by(|a, b| format!("{a:?}").cmp(&format!("{b:?}")));
    v
}

/// Every entry and every balance, for "replays identically".
fn state(l: &Ledger) -> String {
    let mut out = String::new();
    for id in 0..l.next_entry_id() + 2 {
        if let Some(e) = l.get_entry(id) {
            out.push_str(&format!(
                "E{} {} {:?} {:?} {:?}\n",
                e.id(),
                e.date(),
                e.description(),
                e.status(),
                e.lines()
                    .iter()
                    .map(|x| (x.account, x.side, x.amount.amount))
                    .collect::<Vec<_>>()
            ));
        }
    }
    let mut accts: Vec<_> = l
        .chart()
        .all()
        .map(|a| {
            format!(
                "{} {} {:?} {}",
                a.id,
                a.code,
                a.account_type,
                l.account_balance(a.id).unwrap()
            )
        })
        .collect();
    accts.sort();
    out.push_str(&accts.join("\n"));
    for p in l.list_periods() {
        out.push_str(&format!("\n{p:?}"));
    }
    out.push_str(&format!("\n{:?}", roles(l)));
    out
}

/// Oracle: Store::assign_role doc: journaled as `role.assigned` with role,
/// account_id, and the account that filled the role before.
#[test]
fn assign_role_is_journaled_as_role_assigned() {
    let dir = test_dir("journaled");
    let mut s = Store::create(&dir, Currency::USD).unwrap();
    let ar = s
        .add_account("1100", "Trade A/R", AccountType::Asset, None, false)
        .unwrap();
    let old = s
        .add_account(
            "1200",
            "Accounts Receivable",
            AccountType::Asset,
            None,
            false,
        )
        .unwrap();
    let seq = s.journal_seq();
    s.assign_role("receivables", Some(ar)).unwrap();
    assert_eq!(s.journal_seq(), seq + 1);
    let j = journal(&dir);
    let (t, p) = j.last().unwrap();
    assert_eq!(t, "role.assigned");
    assert_eq!(p["role"], json!("receivables"));
    assert_eq!(p["account_id"], json!(ar));
    // Before: the account at 1200 filled it by code.
    assert_eq!(p["previous_account_id"], json!(old));

    // Back to the code: account_id is null, the previous is 1100.
    s.assign_role("receivables", None).unwrap();
    let j = journal(&dir);
    let (t, p) = j.last().unwrap();
    assert_eq!(t, "role.assigned");
    assert_eq!(p["role"], json!("receivables"));
    assert!(p["account_id"].is_null());
    assert_eq!(p["previous_account_id"], json!(ar));
    assert_eq!(s.ledger().role_account(&RECEIVABLES), Some(old));
}

/// Oracle: a role with no account at its code yet had no previous account.
#[test]
fn a_first_assignment_with_nothing_at_the_code_has_no_previous() {
    let dir = test_dir("no-previous");
    let mut s = Store::create(&dir, Currency::USD).unwrap();
    let sales = s
        .add_account("4100", "Sales", AccountType::Revenue, None, false)
        .unwrap();
    s.assign_role("revenue", Some(sales)).unwrap();
    let j = journal(&dir);
    let (t, p) = j.last().unwrap();
    assert_eq!(t, "role.assigned");
    assert_eq!(p["account_id"], json!(sales));
    assert!(p["previous_account_id"].is_null());
}

/// Oracle: Ledger::assign_role INVARIANT 5 through the store: a refusal
/// journals nothing and changes nothing.
#[test]
fn a_refused_assignment_journals_nothing() {
    let dir = test_dir("refused");
    let mut s = Store::create(&dir, Currency::USD).unwrap();
    let exp = s
        .add_account("6500", "Misc", AccountType::Expense, None, false)
        .unwrap();
    let ph = s
        .add_account("1150", "A/R group", AccountType::Asset, None, true)
        .unwrap();
    let bytes = std::fs::read(dir.join(JOURNAL_FILE)).unwrap();
    let seq = s.journal_seq();
    let before = state(s.ledger());

    let e = s.assign_role("nope", Some(exp)).unwrap_err();
    assert!(matches!(e, StoreError::Ledger(LedgerError::UnknownRole(ref k)) if k == "nope"));
    let e = s.assign_role("receivables", Some(exp)).unwrap_err();
    assert!(matches!(
        e,
        StoreError::Ledger(LedgerError::ControlAccountMismatch { .. })
    ));
    let e = s.assign_role("receivables", Some(ph)).unwrap_err();
    assert!(matches!(
        e,
        StoreError::Ledger(LedgerError::ControlAccountMismatch { .. })
    ));
    let e = s.assign_role("receivables", Some(9999)).unwrap_err();
    assert!(matches!(
        e,
        StoreError::Ledger(LedgerError::UnknownAccount(9999))
    ));
    s.assign_role("general_expense", Some(exp)).unwrap();
    let bytes_after_ok = std::fs::read(dir.join(JOURNAL_FILE)).unwrap();
    assert!(bytes_after_ok.len() > bytes.len());
    let seq_ok = s.journal_seq();
    assert_eq!(seq_ok, seq + 1);
    let mid = state(s.ledger());
    assert_ne!(mid, before);
    let e = s.assign_role("salary_expense", Some(exp)).unwrap_err();
    assert!(matches!(
        e,
        StoreError::Ledger(LedgerError::RoleTaken { .. })
    ));
    assert_eq!(s.journal_seq(), seq_ok);
    assert_eq!(
        std::fs::read(dir.join(JOURNAL_FILE)).unwrap(),
        bytes_after_ok
    );
    assert_eq!(state(s.ledger()), mid);
}

/// Oracle: B-23 (replay never depends on a setting outside the journal):
/// reopening gives the same assignments, and later commands still post to
/// the assigned accounts.
#[test]
fn assignments_replay_on_reopen_and_steer_later_invoices() {
    let dir = test_dir("replay");
    let mut s = Store::create(&dir, Currency::USD).unwrap();
    let cash = s
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    let ar = s
        .add_account("1100", "Trade A/R", AccountType::Asset, None, false)
        .unwrap();
    let sales = s
        .add_account("4100", "Sales", AccountType::Revenue, None, false)
        .unwrap();
    let misc = s
        .add_account("6500", "Misc", AccountType::Expense, None, false)
        .unwrap();
    s.assign_role("receivables", Some(ar)).unwrap();
    s.assign_role("revenue", Some(sales)).unwrap();
    s.assign_role("general_expense", Some(misc)).unwrap();
    // Assigned then returned: replay must end with it unassigned.
    s.assign_role("general_expense", None).unwrap();
    let before = roles(s.ledger());
    drop(s);

    let mut s = Store::open(&dir).unwrap();
    assert_eq!(roles(s.ledger()), before);
    assert_eq!(s.ledger().role_assignment(&RECEIVABLES), Some(ar));

    let cust = s.add_customer("Acme").unwrap();
    let inv = s
        .create_invoice(
            cust,
            date(2026, 3, 1),
            date(2026, 3, 31),
            vec![line("w", "42.00")],
        )
        .unwrap();
    let e = s.ledger().get_invoice(inv).unwrap().entry_id;
    let mut want = vec![
        ("1100".to_string(), Side::Debit, dec("42.00")),
        ("4100".to_string(), Side::Credit, dec("42.00")),
    ];
    want.sort_by(|a, b| format!("{a:?}").cmp(&format!("{b:?}")));
    assert_eq!(entry_codes(s.ledger(), e), want);
    s.receive_payment(inv, date(2026, 3, 9), dec("42.00"), cash)
        .unwrap();
    assert!(s.ledger().chart().find_by_code("1200").is_none());
    assert!(s.ledger().chart().find_by_code("4000").is_none());
    let after = state(s.ledger());
    drop(s);

    let s = Store::open(&dir).unwrap();
    assert_eq!(state(s.ledger()), after);
}

/// Oracle: B-23 + close_period: a close to retained earnings after the
/// role is assigned goes to the assigned account, and replays identically.
#[test]
fn a_close_to_assigned_retained_earnings_replays_identically() {
    let dir = test_dir("close");
    let mut s = Store::create(&dir, Currency::USD).unwrap();
    let cash = s
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    let rev = s
        .add_account("4500", "Fees", AccountType::Revenue, None, false)
        .unwrap();
    let re = s
        .add_account(
            "3100",
            "Accumulated Earnings",
            AccountType::Equity,
            None,
            false,
        )
        .unwrap();
    s.assign_role("retained_earnings", Some(re)).unwrap();
    let e = s.create_entry(date(2026, 1, 5), "fees").unwrap();
    s.add_line(
        e,
        cash,
        Side::Debit,
        Money::new(dec("310.00"), Currency::USD),
    )
    .unwrap();
    s.add_line(
        e,
        rev,
        Side::Credit,
        Money::new(dec("310.00"), Currency::USD),
    )
    .unwrap();
    s.post_entry(e).unwrap();
    let p = s
        .create_period("2026-01", date(2026, 1, 1), date(2026, 1, 31))
        .unwrap();
    let closing = s
        .close_period_to(p, &CloseTo::RetainedEarnings { sweep: vec![] })
        .unwrap()
        .expect("closing entry");
    let mut want = vec![
        ("4500".to_string(), Side::Debit, dec("310.00")),
        ("3100".to_string(), Side::Credit, dec("310.00")),
    ];
    want.sort_by(|a, b| format!("{a:?}").cmp(&format!("{b:?}")));
    assert_eq!(entry_codes(s.ledger(), closing), want);
    assert!(s.ledger().chart().find_by_code("3999").is_none());
    let before = state(s.ledger());
    drop(s);

    let s = Store::open(&dir).unwrap();
    assert_eq!(state(s.ledger()), before);
    assert_eq!(s.ledger().account_balance(re).unwrap(), dec("310.00"));
}

/// Oracle: a journal is the system of record; a `role.assigned` line whose
/// account_id isn't an account id (an integer) is corruption, refused on
/// open. The line is appended with `JournalWriter`, so the hash chain is
/// valid and only the payload is wrong.
#[test]
fn a_role_assigned_line_with_a_non_integer_account_is_corrupt() {
    for bad in [json!("1100"), json!(1.5), json!(-3), json!({"id": 1})] {
        let dir = test_dir("corrupt");
        let mut s = Store::create(&dir, Currency::USD).unwrap();
        s.add_account("1100", "Trade A/R", AccountType::Asset, None, false)
            .unwrap();
        drop(s);
        let mut w = JournalWriter::open(&dir.join(JOURNAL_FILE)).unwrap();
        w.append(
            "role.assigned",
            json!({"role": "receivables", "account_id": bad, "previous_account_id": null}),
        )
        .unwrap();
        drop(w);
        match Store::open(&dir) {
            Err(StoreError::JournalCorrupt(_)) => {}
            Err(other) => panic!("account_id {bad}: expected JournalCorrupt, got {other:?}"),
            Ok(_) => panic!("account_id {bad}: a corrupt role.assigned line opened"),
        }
    }
}
