//! Spec-derived tests for control-role assignment: the engine's roles
//! (receivables, revenue, payables, general expense, retained earnings, the
//! seven payroll accounts) assigned to the chart's own accounts.
//!
//! Oracles are `docs/spec/rules.md` B-4 (the five elements) and B-23
//! (control roles), the spec block on `Ledger::assign_role`, the doc
//! comments of `Ledger::role_account`, `role_assignment`,
//! `check_assign_role`, `control_account`, `ControlAccount::key`, the spec
//! on `Ledger::check_controls` and `renumber_account`, and the error
//! variants `UnknownRole`, `RoleTaken`, `RoleHoldsBalance`,
//! `ControlAccountMismatch`. Expected values are worked by hand from those,
//! never read back from the code under test.

use std::collections::BTreeSet;
use std::str::FromStr;

use chrono::NaiveDate;
use ledger_core::{
    AccountId, AccountType, CloseTo, ControlAccount, Currency, DocumentLine, EntryId,
    GENERAL_EXPENSES, Ledger, LedgerError, Money, PAYABLES, PAYROLL, PayrollLine, RECEIVABLES,
    RETAINED_EARNINGS, REVENUE, Side, control_account, control_accounts,
};
use proptest::prelude::*;
use rust_decimal::Decimal;

// ---------------------------------------------------------------- helpers

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).expect("decimal literal")
}

fn usd(s: &str) -> Money {
    Money::new(dec(s), Currency::USD)
}

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).expect("valid date")
}

fn line(desc: &str, amount: &str) -> DocumentLine {
    DocumentLine {
        description: desc.to_string(),
        amount: dec(amount),
    }
}

/// The twelve role keys, as the spec lists them (B-23 and `UnknownRole`).
const KEYS: [&str; 12] = [
    "receivables",
    "revenue",
    "payables",
    "general_expense",
    "retained_earnings",
    "salary_expense",
    "payroll_tax_expense",
    "federal_withholding",
    "state_withholding",
    "social_security_payable",
    "medicare_payable",
    "other_deductions_payable",
];

fn role(key: &str) -> &'static ControlAccount {
    control_account(key).expect("known role")
}

/// A role is a balance sheet one when its type is asset, liability, or
/// equity (assign_role SCENARIO 4).
fn balance_sheet(c: &ControlAccount) -> bool {
    matches!(
        c.account_type,
        AccountType::Asset | AccountType::Liability | AccountType::Equity
    )
}

/// A ledger with only a cash account (1000).
fn books() -> (Ledger, AccountId) {
    let mut l = Ledger::new(Currency::USD);
    let cash = l
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .expect("cash account");
    (l, cash)
}

fn add(l: &mut Ledger, code: &str, name: &str, t: AccountType) -> AccountId {
    l.add_account(code, name, t, None, false).expect("account")
}

fn acct(l: &Ledger, code: &str) -> Option<AccountId> {
    l.chart().find_by_code(code).map(|a| a.id)
}

fn code_of(l: &Ledger, id: AccountId) -> String {
    l.chart().get(id).expect("known account").code.clone()
}

fn bal(l: &Ledger, id: AccountId) -> Decimal {
    l.account_balance(id).expect("known account")
}

/// Draft, add lines, post. Panics on any refusal (setup only).
fn post(l: &mut Ledger, date: NaiveDate, lines: &[(AccountId, Side, &str)]) -> EntryId {
    let id = l.try_create_entry(date, "test entry").unwrap();
    for (a, s, amt) in lines {
        l.add_line(id, *a, *s, usd(amt)).unwrap();
    }
    l.post_entry(id).unwrap();
    id
}

/// The (code, side, amount) lines of an entry, sorted.
fn lines_by_code(l: &Ledger, e: EntryId) -> Vec<(String, Side, Decimal)> {
    let mut v: Vec<_> = l
        .get_entry(e)
        .expect("entry exists")
        .lines()
        .iter()
        .map(|x| (code_of(l, x.account), x.side, x.amount.amount))
        .collect();
    v.sort_by(|a, b| format!("{a:?}").cmp(&format!("{b:?}")));
    v
}

fn ent(code: &str, side: Side, amt: &str) -> (String, Side, Decimal) {
    (code.to_string(), side, dec(amt))
}

fn sorted(mut v: Vec<(String, Side, Decimal)>) -> Vec<(String, Side, Decimal)> {
    v.sort_by(|a, b| format!("{a:?}").cmp(&format!("{b:?}")));
    v
}

/// Every posted entry, rendered, for "posted entries never change".
fn entries(l: &Ledger) -> Vec<String> {
    (0..l.next_entry_id() + 2)
        .filter_map(|id| l.get_entry(id))
        .map(|e| {
            format!(
                "E{} {} {:?} {:?} {:?}",
                e.id(),
                e.date(),
                e.description(),
                e.status(),
                e.lines()
                    .iter()
                    .map(|x| (x.account, x.side, x.amount.amount))
                    .collect::<Vec<_>>()
            )
        })
        .collect()
}

/// Everything observable about the books, deterministically ordered, so a
/// refusal can be checked to leave them unchanged. Includes every role's
/// assignment and resolution.
fn fingerprint(l: &Ledger) -> String {
    let mut out = String::new();
    let mut accts: Vec<_> = l
        .chart()
        .all()
        .map(|a| {
            format!(
                "{}|{}|{}|{:?}|{}|{:?}",
                a.id, a.code, a.name, a.account_type, a.placeholder, a.parent
            )
        })
        .collect();
    accts.sort();
    out.push_str(&accts.join("\n"));
    out.push_str(&format!(
        "\nnext: acct {} entry {} inv {} bill {} cust {} vend {} period {} run {}\n",
        l.next_account_id(),
        l.next_entry_id(),
        l.next_invoice_id(),
        l.next_bill_id(),
        l.next_customer_id(),
        l.next_vendor_id(),
        l.next_period_id(),
        l.next_payroll_run_id()
    ));
    for e in entries(l) {
        out.push_str(&e);
        out.push('\n');
    }
    for id in 0..l.next_invoice_id() + 2 {
        if let Some(i) = l.get_invoice(id) {
            out.push_str(&format!("{i:?}\n"));
        }
    }
    for id in 0..l.next_bill_id() + 2 {
        if let Some(b) = l.get_bill(id) {
            out.push_str(&format!("{b:?}\n"));
        }
    }
    for p in l.list_periods() {
        out.push_str(&format!("{p:?}\n"));
    }
    for r in l.list_payroll_runs() {
        out.push_str(&format!("{r:?}\n"));
    }
    for c in control_accounts() {
        out.push_str(&format!(
            "role {} = {:?} -> {:?}\n",
            c.key,
            l.role_assignment(c),
            l.role_account(c)
        ));
    }
    out
}

/// Assert `f` refuses and leaves the books unchanged; return the error.
fn refuses(l: &mut Ledger, key: &str, account: Option<AccountId>) -> LedgerError {
    let before = fingerprint(l);
    let err = l
        .assign_role(key, account)
        .expect_err("assignment should be refused");
    assert_eq!(fingerprint(l), before, "a refusal changed the books");
    err
}

/// $5,000 gross; FICA at the 2026 rates (SS 6.2% = 310.00 each share,
/// Medicare 1.45% = 72.50 each share).
fn alice() -> PayrollLine {
    PayrollLine {
        employee_name: "Alice".to_string(),
        gross_pay: dec("5000.00"),
        federal_withholding: dec("600.00"),
        state_withholding: dec("250.00"),
        ss_employee: dec("310.00"),
        medicare_employee: dec("72.50"),
        ss_employer: dec("310.00"),
        medicare_employer: dec("72.50"),
        other_deductions: dec("200.00"),
    }
}

// ------------------------------------------------------------ the roles

/// Oracle: B-23 names the roles and their default codes; `control_account`
/// finds a role by its `key`; `UnknownRole` lists the twelve keys.
#[test]
fn every_role_has_its_key_default_code_and_type() {
    use AccountType::*;
    let expected = [
        ("receivables", "1200", Asset),
        ("revenue", "4000", Revenue),
        ("payables", "2000", Liability),
        ("general_expense", "6999", Expense),
        ("retained_earnings", "3999", Equity),
        ("salary_expense", "6100", Expense),
        ("payroll_tax_expense", "6110", Expense),
        ("federal_withholding", "2110", Liability),
        ("state_withholding", "2120", Liability),
        ("social_security_payable", "2130", Liability),
        ("medicare_payable", "2140", Liability),
        ("other_deductions_payable", "2150", Liability),
    ];
    for (key, code, t) in expected {
        let c = role(key);
        assert_eq!(c.key, key);
        assert_eq!(c.code, code, "{key}");
        assert_eq!(c.account_type, t, "{key}");
    }
    let keys: BTreeSet<_> = control_accounts().map(|c| c.key).collect();
    assert_eq!(keys, KEYS.into_iter().collect::<BTreeSet<_>>());
    assert_eq!(RECEIVABLES.key, "receivables");
    assert_eq!(REVENUE.key, "revenue");
    assert_eq!(PAYABLES.key, "payables");
    assert_eq!(GENERAL_EXPENSES.key, "general_expense");
    assert_eq!(RETAINED_EARNINGS.key, "retained_earnings");
    assert_eq!(PAYROLL.len(), 7);
    assert!(control_account("nope").is_none());
}

/// Oracle: `role_account` is "the one assigned to it, else the account at
/// the role's code (None when there is none yet)"; `role_assignment` is
/// None while the role uses its code.
#[test]
fn a_fresh_role_resolves_to_its_code_or_nothing() {
    let (mut l, _) = books();
    for c in control_accounts() {
        assert_eq!(l.role_assignment(c), None);
        assert_eq!(l.role_account(c), None, "{}", c.key);
    }
    let ar = add(&mut l, "1200", "A/R", AccountType::Asset);
    assert_eq!(l.role_account(&RECEIVABLES), Some(ar));
    assert_eq!(l.role_assignment(&RECEIVABLES), None);
}

// ------------------------------------------------- INVARIANT 1: steering

/// Oracle: assign_role INVARIANT 1 + create_invoice / receive_payment
/// (Dr receivables / Cr revenue; Dr cash / Cr receivables).
#[test]
fn invoices_and_payments_post_to_assigned_receivables_and_revenue() {
    let (mut l, cash) = books();
    let ar = add(&mut l, "1100", "Trade Receivables", AccountType::Asset);
    let sales = add(&mut l, "4100", "Sales", AccountType::Revenue);
    l.assign_role("receivables", Some(ar)).unwrap();
    l.assign_role("revenue", Some(sales)).unwrap();
    assert_eq!(l.role_assignment(&RECEIVABLES), Some(ar));
    assert_eq!(l.role_account(&RECEIVABLES), Some(ar));
    assert_eq!(l.role_account(&REVENUE), Some(sales));

    let cust = l.add_customer("Acme").unwrap();
    let inv = l
        .create_invoice(
            cust,
            d(2026, 3, 1),
            d(2026, 3, 31),
            vec![line("work", "300.00"), line("more", "50.25")],
        )
        .unwrap();
    let e = l.get_invoice(inv).unwrap().entry_id;
    assert_eq!(
        lines_by_code(&l, e),
        sorted(vec![
            ent("1100", Side::Debit, "350.25"),
            ent("4100", Side::Credit, "350.25"),
        ])
    );
    let p = l
        .receive_payment(inv, d(2026, 3, 15), dec("100.00"), cash)
        .unwrap();
    assert_eq!(
        lines_by_code(&l, p),
        sorted(vec![
            ent("1000", Side::Debit, "100.00"),
            ent("1100", Side::Credit, "100.00"),
        ])
    );
    assert_eq!(bal(&l, ar), dec("250.25"));
    assert_eq!(bal(&l, sales), dec("350.25"));
    assert_eq!(acct(&l, "1200"), None, "nothing created at 1200");
    assert_eq!(acct(&l, "4000"), None, "nothing created at 4000");
}

/// Oracle: assign_role INVARIANT 1 + create_bill / pay_bill
/// (Dr general expense / Cr payables; Dr payables / Cr cash).
#[test]
fn bills_and_payments_post_to_assigned_payables_and_general_expense() {
    let (mut l, cash) = books();
    let ap = add(&mut l, "2010", "Trade Payables", AccountType::Liability);
    let misc = add(&mut l, "6500", "Misc Expense", AccountType::Expense);
    l.assign_role("payables", Some(ap)).unwrap();
    l.assign_role("general_expense", Some(misc)).unwrap();

    let v = l.add_vendor("Supplier").unwrap();
    let bill = l
        .create_bill(v, d(2026, 4, 1), d(2026, 4, 30), vec![line("x", "80.00")])
        .unwrap();
    let e = l.get_bill(bill).unwrap().entry_id;
    assert_eq!(
        lines_by_code(&l, e),
        sorted(vec![
            ent("6500", Side::Debit, "80.00"),
            ent("2010", Side::Credit, "80.00"),
        ])
    );
    let p = l
        .pay_bill(bill, d(2026, 4, 10), dec("80.00"), cash)
        .unwrap();
    assert_eq!(
        lines_by_code(&l, p),
        sorted(vec![
            ent("2010", Side::Debit, "80.00"),
            ent("1000", Side::Credit, "80.00"),
        ])
    );
    assert_eq!(bal(&l, ap), Decimal::ZERO);
    assert_eq!(bal(&l, misc), dec("80.00"));
    assert_eq!(acct(&l, "2000"), None);
    assert_eq!(acct(&l, "6999"), None);
}

/// Oracle: assign_role INVARIANT 1 + run_payroll INVARIANT 1 (Dr salary
/// gross, Dr payroll tax employer FICA, Cr the withholdings, Cr cash net),
/// payroll_tax_payables (the withholding accounts' balances, in role
/// order), and remit_payroll_taxes INVARIANT 1 (zero every payable).
#[test]
fn payroll_and_remittance_post_to_all_seven_assigned_accounts() {
    let (mut l, cash) = books();
    let picks = [
        ("salary_expense", "5100", AccountType::Expense),
        ("payroll_tax_expense", "5110", AccountType::Expense),
        ("federal_withholding", "2210", AccountType::Liability),
        ("state_withholding", "2220", AccountType::Liability),
        ("social_security_payable", "2230", AccountType::Liability),
        ("medicare_payable", "2240", AccountType::Liability),
        ("other_deductions_payable", "2250", AccountType::Liability),
    ];
    let mut ids = Vec::new();
    for (key, code, t) in picks {
        let id = add(&mut l, code, key, t);
        l.assign_role(key, Some(id)).unwrap();
        assert_eq!(l.role_account(role(key)), Some(id));
        ids.push(id);
    }
    // Fund cash so net pay has somewhere to come from (not required by the
    // spec; keeps the books ordinary).
    let cap = add(&mut l, "3000", "Capital", AccountType::Equity);
    post(
        &mut l,
        d(2026, 1, 2),
        &[
            (cash, Side::Debit, "10000.00"),
            (cap, Side::Credit, "10000.00"),
        ],
    );

    let run = l.run_payroll(d(2026, 2, 27), vec![alice()], cash).unwrap();
    let e = l.get_payroll_run(run).unwrap().entry_id;
    // Net = 5000 - 600 - 250 - 310 - 72.50 - 200 = 3567.50.
    // Employer FICA = 310 + 72.50 = 382.50. SS payable 620, Medicare 145.
    assert_eq!(
        lines_by_code(&l, e),
        sorted(vec![
            ent("5100", Side::Debit, "5000.00"),
            ent("5110", Side::Debit, "382.50"),
            ent("2210", Side::Credit, "600.00"),
            ent("2220", Side::Credit, "250.00"),
            ent("2230", Side::Credit, "620.00"),
            ent("2240", Side::Credit, "145.00"),
            ent("2250", Side::Credit, "200.00"),
            ent("1000", Side::Credit, "3567.50"),
        ])
    );
    assert_eq!(
        l.payroll_tax_payables(),
        vec![
            (ids[2], dec("600.00")),
            (ids[3], dec("250.00")),
            (ids[4], dec("620.00")),
            (ids[5], dec("145.00")),
            (ids[6], dec("200.00")),
        ]
    );
    let r = l.remit_payroll_taxes(d(2026, 3, 15), cash).unwrap();
    assert_eq!(
        lines_by_code(&l, r),
        sorted(vec![
            ent("2210", Side::Debit, "600.00"),
            ent("2220", Side::Debit, "250.00"),
            ent("2230", Side::Debit, "620.00"),
            ent("2240", Side::Debit, "145.00"),
            ent("2250", Side::Debit, "200.00"),
            ent("1000", Side::Credit, "1815.00"),
        ])
    );
    for id in &ids[2..] {
        assert_eq!(bal(&l, *id), Decimal::ZERO);
    }
    assert!(l.payroll_tax_payables().is_empty());
    for code in ["6100", "6110", "2110", "2120", "2130", "2140", "2150"] {
        assert_eq!(acct(&l, code), None, "nothing created at {code}");
    }
}

/// Oracle: assign_role INVARIANT 1 + close_period INVARIANT 2 (credit
/// equity with the net income) under CloseTo::RetainedEarnings.
#[test]
fn a_close_to_retained_earnings_goes_to_the_assigned_equity_account() {
    let (mut l, cash) = books();
    let re = add(&mut l, "3100", "Accumulated Earnings", AccountType::Equity);
    let rev = add(&mut l, "4500", "Fees", AccountType::Revenue);
    let exp = add(&mut l, "6500", "Rent", AccountType::Expense);
    l.assign_role("retained_earnings", Some(re)).unwrap();
    post(
        &mut l,
        d(2026, 1, 5),
        &[(cash, Side::Debit, "900.00"), (rev, Side::Credit, "900.00")],
    );
    post(
        &mut l,
        d(2026, 1, 20),
        &[(exp, Side::Debit, "250.00"), (cash, Side::Credit, "250.00")],
    );
    let p = l
        .create_period("2026-01", d(2026, 1, 1), d(2026, 1, 31))
        .unwrap();
    let closing = l
        .close_period_to(p, &CloseTo::RetainedEarnings { sweep: vec![] })
        .unwrap()
        .expect("a closing entry");
    assert_eq!(
        lines_by_code(&l, closing),
        sorted(vec![
            ent("4500", Side::Debit, "900.00"),
            ent("6500", Side::Credit, "250.00"),
            ent("3100", Side::Credit, "650.00"),
        ])
    );
    assert_eq!(bal(&l, re), dec("650.00"));
    assert_eq!(acct(&l, "3999"), None, "nothing created at 3999");
}

/// Oracle: assign_role "or (None) return it to its code"; INVARIANT 1
/// "after None, to the account at the role's code (created when first
/// needed, as before)".
#[test]
fn none_returns_a_role_to_its_code() {
    let (mut l, _) = books();
    let ar = add(&mut l, "1100", "Trade Receivables", AccountType::Asset);
    let sales = add(&mut l, "4100", "Sales", AccountType::Revenue);
    l.assign_role("receivables", Some(ar)).unwrap();
    l.assign_role("revenue", Some(sales)).unwrap();
    // Neither holds a balance yet, so both may move.
    l.assign_role("receivables", None).unwrap();
    l.assign_role("revenue", None).unwrap();
    assert_eq!(l.role_assignment(&RECEIVABLES), None);
    assert_eq!(l.role_account(&RECEIVABLES), None);
    assert_eq!(l.role_account(&REVENUE), None);

    let cust = l.add_customer("Acme").unwrap();
    let inv = l
        .create_invoice(
            cust,
            d(2026, 3, 1),
            d(2026, 3, 31),
            vec![line("w", "10.00")],
        )
        .unwrap();
    let e = l.get_invoice(inv).unwrap().entry_id;
    assert_eq!(
        lines_by_code(&l, e),
        sorted(vec![
            ent("1200", Side::Debit, "10.00"),
            ent("4000", Side::Credit, "10.00"),
        ])
    );
    assert_eq!(l.role_account(&RECEIVABLES), acct(&l, "1200"));
    assert_eq!(bal(&l, ar), Decimal::ZERO);
    assert_eq!(bal(&l, sales), Decimal::ZERO);
}

/// Oracle: None returns the role to an existing right-typed account at its
/// code, and later postings go there.
#[test]
fn none_returns_a_role_to_an_existing_account_at_its_code() {
    let (mut l, _) = books();
    let exp = add(&mut l, "6999", "General", AccountType::Expense);
    let other = add(&mut l, "6500", "Other", AccountType::Expense);
    l.assign_role("general_expense", Some(other)).unwrap();
    l.assign_role("general_expense", None).unwrap();
    assert_eq!(l.role_account(&GENERAL_EXPENSES), Some(exp));
    let v = l.add_vendor("V").unwrap();
    let b = l
        .create_bill(v, d(2026, 2, 1), d(2026, 2, 1), vec![line("x", "5.00")])
        .unwrap();
    let e = l.get_bill(b).unwrap().entry_id;
    assert!(lines_by_code(&l, e).contains(&ent("6999", Side::Debit, "5.00")));
}

// --------------------------------------------------- SCENARIO 1: unknowns

/// Oracle: SCENARIO 1 (unknown role) + `UnknownRole`; INVARIANT 5.
#[test]
fn an_unknown_role_is_refused() {
    let (mut l, cash) = books();
    for key in ["", "Receivables", "receivable", "ar", "salary"] {
        assert_eq!(
            refuses(&mut l, key, Some(cash)),
            LedgerError::UnknownRole(key.to_string())
        );
        assert_eq!(
            refuses(&mut l, key, None),
            LedgerError::UnknownRole(key.to_string())
        );
        assert!(l.check_assign_role(key, None).is_err());
    }
}

/// Oracle: SCENARIO 1 (unknown account); INVARIANT 5.
#[test]
fn an_unknown_account_is_refused() {
    let (mut l, _) = books();
    let missing = l.next_account_id() + 7;
    for key in KEYS {
        assert_eq!(
            refuses(&mut l, key, Some(missing)),
            LedgerError::UnknownAccount(missing)
        );
    }
}

// ----------------------------------------- SCENARIO 2: type / placeholder

/// Oracle: SCENARIO 2 + INVARIANT 3 + B-4: every role refuses an account of
/// each other type with ControlAccountMismatch naming the account, the
/// role, the type needed and the type found.
#[test]
fn a_wrong_typed_account_is_refused_for_every_role() {
    use AccountType::*;
    let (mut l, _) = books();
    let types = [Asset, Liability, Equity, Revenue, Expense];
    let mut pool = Vec::new();
    for (i, t) in types.into_iter().enumerate() {
        let code = format!("{}", 8000 + i);
        let id = add(&mut l, &code, &format!("Pool {t:?}"), t);
        pool.push((id, code, t));
    }
    for c in control_accounts() {
        for (id, code, t) in &pool {
            if *t == c.account_type {
                continue;
            }
            match refuses(&mut l, c.key, Some(*id)) {
                LedgerError::ControlAccountMismatch {
                    code: got_code,
                    name,
                    role,
                    expected,
                    found,
                    placeholder,
                } => {
                    assert_eq!(&got_code, code);
                    assert_eq!(name, format!("Pool {t:?}"));
                    assert_eq!(role, c.role);
                    assert_eq!(expected, c.account_type);
                    assert_eq!(found, *t);
                    assert!(!placeholder);
                }
                other => panic!("{}: expected ControlAccountMismatch, got {other:?}", c.key),
            }
        }
    }
}

/// Oracle: SCENARIO 2 + INVARIANT 3 ("always postable").
#[test]
fn a_placeholder_account_is_refused() {
    let (mut l, _) = books();
    let ph = l
        .add_account(
            "1150",
            "Receivables (group)",
            AccountType::Asset,
            None,
            true,
        )
        .unwrap();
    match refuses(&mut l, "receivables", Some(ph)) {
        LedgerError::ControlAccountMismatch {
            expected,
            found,
            placeholder,
            ..
        } => {
            assert_eq!(expected, AccountType::Asset);
            assert_eq!(found, AccountType::Asset);
            assert!(placeholder);
        }
        other => panic!("expected ControlAccountMismatch, got {other:?}"),
    }
}

/// Oracle: SCENARIO 2 "(or, for None, the account at the role's code)":
/// returning a role to a code that holds a wrong-typed account is refused.
#[test]
fn none_is_refused_when_the_default_code_holds_a_wrong_typed_account() {
    // An imported chart with an expense at 1200, role never assigned.
    let (mut l, _) = books();
    add(&mut l, "1200", "Postage", AccountType::Expense);
    match refuses(&mut l, "receivables", None) {
        LedgerError::ControlAccountMismatch {
            expected, found, ..
        } => {
            assert_eq!(expected, AccountType::Asset);
            assert_eq!(found, AccountType::Expense);
        }
        other => panic!("expected ControlAccountMismatch, got {other:?}"),
    }

    // The role assigned elsewhere, then 1200 taken by a wrong type (an
    // assignment frees the code, B-23 / check_renumber_account): None is
    // refused, and the assignment stays.
    let (mut l, _) = books();
    let ar = add(&mut l, "1100", "Trade Receivables", AccountType::Asset);
    l.assign_role("receivables", Some(ar)).unwrap();
    let post_exp = add(&mut l, "6400", "Postage", AccountType::Expense);
    l.renumber_account(post_exp, "1200").unwrap();
    assert!(matches!(
        refuses(&mut l, "receivables", None),
        LedgerError::ControlAccountMismatch { .. }
    ));
    assert_eq!(l.role_account(&RECEIVABLES), Some(ar));

    // A placeholder at the code is refused for None too.
    let (mut l, _) = books();
    l.add_account("6999", "Expenses", AccountType::Expense, None, true)
        .unwrap();
    assert!(matches!(
        refuses(&mut l, "general_expense", None),
        LedgerError::ControlAccountMismatch {
            placeholder: true,
            ..
        }
    ));
}

// ------------------------------------------------ SCENARIO 3: RoleTaken

/// Oracle: SCENARIO 3 + INVARIANT 2: an account assigned to one role can't
/// take another.
#[test]
fn one_account_cannot_fill_two_assigned_roles() {
    let (mut l, _) = books();
    let liab = add(&mut l, "2500", "Payables to all", AccountType::Liability);
    l.assign_role("payables", Some(liab)).unwrap();
    match refuses(&mut l, "federal_withholding", Some(liab)) {
        LedgerError::RoleTaken { code, name, role } => {
            assert_eq!(code, "2500");
            assert_eq!(name, "Payables to all");
            assert_eq!(role, PAYABLES.role);
        }
        other => panic!("expected RoleTaken, got {other:?}"),
    }
    let exp = add(&mut l, "6500", "Wages", AccountType::Expense);
    l.assign_role("salary_expense", Some(exp)).unwrap();
    assert!(matches!(
        refuses(&mut l, "payroll_tax_expense", Some(exp)),
        LedgerError::RoleTaken { .. }
    ));
    assert!(matches!(
        refuses(&mut l, "general_expense", Some(exp)),
        LedgerError::RoleTaken { .. }
    ));
}

/// Oracle: SCENARIO 3 + `role_account` ("else the account at the role's
/// code"): an account filling a role only by its default code fills it.
#[test]
fn an_account_at_another_roles_default_code_is_taken() {
    let (mut l, _) = books();
    let ge = add(&mut l, "6999", "General Expenses", AccountType::Expense);
    match refuses(&mut l, "salary_expense", Some(ge)) {
        LedgerError::RoleTaken { code, role, .. } => {
            assert_eq!(code, "6999");
            assert_eq!(role, GENERAL_EXPENSES.role);
        }
        other => panic!("expected RoleTaken, got {other:?}"),
    }
    // Once general expense moves elsewhere, 6999 fills no role and is free.
    let g = add(&mut l, "6998", "Sundry", AccountType::Expense);
    l.assign_role("general_expense", Some(g)).unwrap();
    l.assign_role("salary_expense", Some(ge)).unwrap();
    assert_eq!(l.role_account(PAYROLL.first().unwrap()), Some(ge));
}

/// Oracle: SCENARIO 3 applies to None too ("that account" is the one at the
/// role's code): returning a role to its code while another role is
/// assigned the account there is refused.
#[test]
fn none_is_refused_when_its_codes_account_fills_another_role() {
    let (mut l, _) = books();
    let at_6100 = add(&mut l, "6100", "Salaries", AccountType::Expense);
    let wages = add(&mut l, "5100", "Wages", AccountType::Expense);
    l.assign_role("salary_expense", Some(wages)).unwrap();
    l.assign_role("general_expense", Some(at_6100)).unwrap();
    match refuses(&mut l, "salary_expense", None) {
        LedgerError::RoleTaken { code, role, .. } => {
            assert_eq!(code, "6100");
            assert_eq!(role, GENERAL_EXPENSES.role);
        }
        other => panic!("expected RoleTaken, got {other:?}"),
    }
}

// --------------------------------------------- SCENARIO 4: balances move

/// Oracle: SCENARIO 4 + B-23 ("moves only when its account holds no
/// balance, so the bookkeeper first moves the balance with an entry").
#[test]
fn receivables_cannot_move_while_holding_a_balance_but_can_once_moved_out() {
    let (mut l, cash) = books();
    let cust = l.add_customer("Acme").unwrap();
    let inv = l
        .create_invoice(
            cust,
            d(2026, 3, 1),
            d(2026, 3, 31),
            vec![line("w", "120.00")],
        )
        .unwrap();
    let old = acct(&l, "1200").expect("1200 created by the invoice");
    let new = add(&mut l, "1100", "Trade Receivables", AccountType::Asset);
    match refuses(&mut l, "receivables", Some(new)) {
        LedgerError::RoleHoldsBalance {
            role,
            code,
            balance,
            ..
        } => {
            assert_eq!(role, RECEIVABLES.role);
            assert_eq!(code, "1200");
            assert_eq!(balance, dec("120.00"));
        }
        other => panic!("expected RoleHoldsBalance, got {other:?}"),
    }
    // Move the balance with an entry, then the role may move.
    post(
        &mut l,
        d(2026, 3, 2),
        &[(new, Side::Debit, "120.00"), (old, Side::Credit, "120.00")],
    );
    l.assign_role("receivables", Some(new)).unwrap();
    assert_eq!(l.role_account(&RECEIVABLES), Some(new));
    // The open invoice is now controlled by 1100: its payment credits it.
    let p = l
        .receive_payment(inv, d(2026, 3, 20), dec("120.00"), cash)
        .unwrap();
    assert!(lines_by_code(&l, p).contains(&ent("1100", Side::Credit, "120.00")));
    assert_eq!(bal(&l, new), Decimal::ZERO);
    assert_eq!(bal(&l, old), Decimal::ZERO);
}

/// Oracle: SCENARIO 4 for each balance-sheet role kind (asset, liability,
/// equity), and for None as a move too.
#[test]
fn every_balance_sheet_role_holds_while_its_account_has_a_balance() {
    let (mut l, cash) = books();
    let mut held = Vec::new();
    for c in control_accounts().filter(|c| balance_sheet(c)) {
        let cur = add(&mut l, c.code, c.name, c.account_type);
        // Give it a balance against cash.
        let (dr, cr) = if c.account_type == AccountType::Asset {
            (cur, cash)
        } else {
            (cash, cur)
        };
        post(
            &mut l,
            d(2026, 1, 3),
            &[(dr, Side::Debit, "7.00"), (cr, Side::Credit, "7.00")],
        );
        held.push((c, cur));
    }
    for (i, (c, cur)) in held.iter().enumerate() {
        let target = add(&mut l, &format!("{}", 9100 + i), "Target", c.account_type);
        assert!(
            matches!(
                refuses(&mut l, c.key, Some(target)),
                LedgerError::RoleHoldsBalance { .. }
            ),
            "{}",
            c.key
        );
        // Assigning the account it already resolves to is not a move.
        l.assign_role(c.key, Some(*cur)).unwrap();
        assert_eq!(l.role_assignment(c), Some(*cur));
        // None would move it nowhere new (the code holds the same account):
        // accepted.
        l.assign_role(c.key, None).unwrap();
        assert_eq!(l.role_account(c), Some(*cur));
    }
}

/// Oracle: SCENARIO 4 (None is a move when the role is assigned away from
/// its code and the assigned account holds a balance).
#[test]
fn none_is_refused_while_the_assigned_balance_sheet_account_holds_a_balance() {
    let (mut l, cash) = books();
    let ap = add(&mut l, "2010", "Trade Payables", AccountType::Liability);
    l.assign_role("payables", Some(ap)).unwrap();
    let v = l.add_vendor("V").unwrap();
    let exp = add(&mut l, "6500", "Stuff", AccountType::Expense);
    l.assign_role("general_expense", Some(exp)).unwrap();
    l.create_bill(v, d(2026, 2, 1), d(2026, 2, 28), vec![line("x", "40.00")])
        .unwrap();
    assert!(matches!(
        refuses(&mut l, "payables", None),
        LedgerError::RoleHoldsBalance { .. }
    ));
    let _ = cash;
}

/// Oracle: SCENARIO 4 "Revenue and expense roles move freely"; B-23
/// "earlier postings stay where they are"; INVARIANT 4.
#[test]
fn revenue_and_expense_roles_move_even_with_a_balance() {
    let (mut l, cash) = books();
    let cust = l.add_customer("Acme").unwrap();
    let ar = add(&mut l, "1200", "A/R", AccountType::Asset);
    let _ = ar;
    l.create_invoice(
        cust,
        d(2026, 3, 1),
        d(2026, 3, 31),
        vec![line("w", "60.00")],
    )
    .unwrap();
    let rev_old = acct(&l, "4000").expect("revenue created");
    let v = l.add_vendor("V").unwrap();
    let ap = add(&mut l, "2000", "A/P", AccountType::Liability);
    l.create_bill(v, d(2026, 3, 2), d(2026, 3, 2), vec![line("x", "15.00")])
        .unwrap();
    let exp_old = acct(&l, "6999").expect("general expense created");
    let _ = (ap, cash);
    let before = entries(&l);

    let rev_new = add(&mut l, "4100", "Sales", AccountType::Revenue);
    let exp_new = add(&mut l, "6500", "Misc", AccountType::Expense);
    l.assign_role("revenue", Some(rev_new)).unwrap();
    l.assign_role("general_expense", Some(exp_new)).unwrap();
    assert_eq!(entries(&l), before, "posted entries unchanged");
    assert_eq!(bal(&l, rev_old), dec("60.00"));
    assert_eq!(bal(&l, exp_old), dec("15.00"));

    l.create_invoice(cust, d(2026, 4, 1), d(2026, 4, 30), vec![line("w", "5.00")])
        .unwrap();
    l.create_bill(v, d(2026, 4, 2), d(2026, 4, 2), vec![line("x", "3.00")])
        .unwrap();
    assert_eq!(bal(&l, rev_old), dec("60.00"));
    assert_eq!(bal(&l, rev_new), dec("5.00"));
    assert_eq!(bal(&l, exp_old), dec("15.00"));
    assert_eq!(bal(&l, exp_new), dec("3.00"));

    // Payroll expense roles move freely too.
    let run = l.run_payroll(d(2026, 4, 30), vec![alice()], cash).unwrap();
    let _ = run;
    let wages_old = acct(&l, "6100").unwrap();
    let wages_new = add(&mut l, "5100", "Wages", AccountType::Expense);
    l.assign_role("salary_expense", Some(wages_new)).unwrap();
    assert_eq!(bal(&l, wages_old), dec("5000.00"));
    // ... and back to its code, though 6100 holds a balance.
    l.assign_role("salary_expense", None).unwrap();
    assert_eq!(l.role_account(&PAYROLL[0]), Some(wages_old));
}

// ---------------------------------------------- SCENARIO 5 / INVARIANT 4

/// Oracle: SCENARIO 5: re-assigning the account the role resolves to is
/// accepted and recorded, balance or not.
#[test]
fn reassigning_the_account_a_role_already_resolves_to_is_accepted() {
    let (mut l, _) = books();
    let cust = l.add_customer("Acme").unwrap();
    l.create_invoice(cust, d(2026, 3, 1), d(2026, 3, 31), vec![line("w", "9.00")])
        .unwrap();
    let ar = acct(&l, "1200").unwrap();
    assert_eq!(l.role_assignment(&RECEIVABLES), None);
    l.assign_role("receivables", Some(ar)).unwrap();
    assert_eq!(l.role_assignment(&RECEIVABLES), Some(ar), "recorded");
    l.assign_role("receivables", Some(ar)).unwrap();
    assert_eq!(l.role_assignment(&RECEIVABLES), Some(ar));
    // check_assign_role agrees and does not mutate.
    let before = fingerprint(&l);
    assert_eq!(
        l.check_assign_role("receivables", Some(ar)).unwrap().key,
        "receivables"
    );
    assert_eq!(fingerprint(&l), before);
}

/// Oracle: INVARIANT 4: an assignment changes no posted entry.
#[test]
fn posted_entries_are_unchanged_by_assignments() {
    let (mut l, cash) = books();
    let cust = l.add_customer("Acme").unwrap();
    let inv = l
        .create_invoice(
            cust,
            d(2026, 3, 1),
            d(2026, 3, 31),
            vec![line("w", "75.00")],
        )
        .unwrap();
    l.receive_payment(inv, d(2026, 3, 5), dec("75.00"), cash)
        .unwrap();
    let before = entries(&l);
    let balances: Vec<_> = l.chart().all().map(|a| (a.id, bal(&l, a.id))).collect();
    // 1200 is back to zero, so receivables may move; revenue moves freely.
    let ar = add(&mut l, "1100", "Trade A/R", AccountType::Asset);
    let sales = add(&mut l, "4100", "Sales", AccountType::Revenue);
    l.assign_role("receivables", Some(ar)).unwrap();
    l.assign_role("revenue", Some(sales)).unwrap();
    assert_eq!(entries(&l), before);
    for (id, b) in balances {
        assert_eq!(bal(&l, id), b);
    }
}

/// Oracle: check_assign_role is "the validation behind assign_role": it
/// agrees with assign_role on accept/refuse and changes nothing.
#[test]
fn check_assign_role_agrees_and_does_not_mutate() {
    let (mut l, _) = books();
    let ar = add(&mut l, "1100", "Trade A/R", AccountType::Asset);
    let exp = add(&mut l, "6500", "Misc", AccountType::Expense);
    let before = fingerprint(&l);
    assert!(l.check_assign_role("receivables", Some(ar)).is_ok());
    assert!(matches!(
        l.check_assign_role("receivables", Some(exp)),
        Err(LedgerError::ControlAccountMismatch { .. })
    ));
    assert_eq!(fingerprint(&l), before);
}

// ------------------------------------------- renumbering and the codes

/// Oracle: B-23 + check_renumber_account ("an assigned role frees its
/// code"); renumber_account SCENARIO 2 for an unassigned role's code.
#[test]
fn an_assigned_role_frees_its_code_for_renumbering() {
    let (mut l, _) = books();
    let ar = add(&mut l, "1100", "Trade A/R", AccountType::Asset);
    let postage = add(&mut l, "6400", "Postage", AccountType::Expense);
    let hosting = add(&mut l, "6450", "Hosting", AccountType::Expense);

    // Revenue is unassigned: its code still refuses an expense.
    let before = fingerprint(&l);
    assert!(matches!(
        l.renumber_account(hosting, "4000"),
        Err(LedgerError::ControlAccountMismatch { .. })
    ));
    assert_eq!(fingerprint(&l), before);
    // Receivables unassigned: 1200 refuses an expense too.
    assert!(matches!(
        l.renumber_account(postage, "1200"),
        Err(LedgerError::ControlAccountMismatch { .. })
    ));

    l.assign_role("receivables", Some(ar)).unwrap();
    l.renumber_account(postage, "1200").unwrap();
    assert_eq!(code_of(&l, postage), "1200");
    // The role still resolves to its assignment and invoices go there.
    assert_eq!(l.role_account(&RECEIVABLES), Some(ar));
    let cust = l.add_customer("Acme").unwrap();
    let inv = l
        .create_invoice(cust, d(2026, 3, 1), d(2026, 3, 31), vec![line("w", "1.00")])
        .unwrap();
    let e = l.get_invoice(inv).unwrap().entry_id;
    assert!(lines_by_code(&l, e).contains(&ent("1100", Side::Debit, "1.00")));
    assert_eq!(bal(&l, postage), Decimal::ZERO);
    // An unassigned role's free code (payables, 2000) still refuses the
    // expense. (4000 now holds the invoice's revenue account.)
    assert!(matches!(
        l.renumber_account(hosting, "2000"),
        Err(LedgerError::ControlAccountMismatch { .. })
    ));
    assert_eq!(code_of(&l, hosting), "6450");
}

// ------------------------------------------------------------- property

#[derive(Clone, Debug)]
enum Op {
    /// Assign role `0` (index into KEYS) the pool account `1`, or None.
    Assign(usize, Option<usize>),
    /// Post a small entry debiting pool account `0` and crediting `1`
    /// (skipped if either is a placeholder or they are equal), so balances
    /// come and go.
    Move(usize, usize, u32),
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        3 => (0..KEYS.len(), proptest::option::of(0..POOL)).prop_map(|(r, a)| Op::Assign(r, a)),
        1 => (0..POOL, 0..POOL, 1u32..500).prop_map(|(a, b, c)| Op::Move(a, b, c)),
    ]
}

const POOL: usize = 24;

/// A pool of accounts of every type: a right-typed postable account at
/// most roles' default codes, two spares of each type, and a placeholder of
/// each type.
fn pool_books() -> (Ledger, Vec<AccountId>) {
    use AccountType::*;
    let mut l = Ledger::new(Currency::USD);
    let mut ids = Vec::new();
    for c in control_accounts().take(7) {
        ids.push(add(&mut l, c.code, c.name, c.account_type));
    }
    for (i, t) in [Asset, Liability, Equity, Revenue, Expense]
        .into_iter()
        .enumerate()
    {
        ids.push(add(&mut l, &format!("{}", 8000 + 10 * i), "spare a", t));
        ids.push(add(&mut l, &format!("{}", 8001 + 10 * i), "spare b", t));
        ids.push(
            l.add_account(&format!("{}", 8002 + 10 * i), "group", t, None, true)
                .unwrap(),
        );
    }
    while ids.len() < POOL {
        let n = ids.len();
        ids.push(add(&mut l, &format!("{}", 8500 + n), "extra", Expense));
    }
    assert_eq!(ids.len(), POOL);
    (l, ids)
}

/// The reasons the spec gives to refuse an assignment, worked from the
/// current books (assign_role SCENARIOS 1–5).
fn predicted_refusals(
    l: &Ledger,
    c: &ControlAccount,
    account: Option<AccountId>,
) -> Vec<&'static str> {
    let mut why = Vec::new();
    let target = match account {
        Some(id) => l.chart().get(id),
        None => l.chart().find_by_code(c.code),
    };
    if let Some(a) = target {
        if a.account_type != c.account_type || a.placeholder {
            why.push("mismatch");
        }
        if control_accounts().any(|o| o.key != c.key && l.role_account(o) == Some(a.id)) {
            why.push("taken");
        }
    }
    let now = l.role_account(c);
    let next = target.map(|a| a.id);
    if balance_sheet(c)
        && next != now
        && let Some(cur) = now
        && !bal(l, cur).is_zero()
    {
        why.push("balance");
    }
    why
}

fn reason(e: &LedgerError) -> &'static str {
    match e {
        LedgerError::ControlAccountMismatch { .. } => "mismatch",
        LedgerError::RoleTaken { .. } => "taken",
        LedgerError::RoleHoldsBalance { .. } => "balance",
        _ => "other",
    }
}

fn assert_role_invariants(l: &Ledger) {
    // INVARIANT 2: no account fills two roles.
    let filled: Vec<AccountId> = control_accounts()
        .filter_map(|c| l.role_account(c))
        .collect();
    let distinct: BTreeSet<_> = filled.iter().copied().collect();
    assert_eq!(filled.len(), distinct.len(), "an account fills two roles");
    // INVARIANT 3: a role's account is postable and of its type.
    for c in control_accounts() {
        if let Some(id) = l.role_account(c) {
            let a = l.chart().get(id).expect("role account exists");
            assert_eq!(a.account_type, c.account_type, "{}", c.key);
            assert!(!a.placeholder, "{}", c.key);
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Oracle: assign_role INVARIANTS 2, 3, 5 and SCENARIOS 2–5, checked
    /// after every command of a random sequence.
    #[test]
    fn role_invariants_hold_over_random_assignments(ops in proptest::collection::vec(op(), 1..40)) {
        let (mut l, pool) = pool_books();
        assert_role_invariants(&l);
        for o in ops {
            match o {
                Op::Assign(r, a) => {
                    let c = role(KEYS[r]);
                    let account = a.map(|i| pool[i]);
                    let why = predicted_refusals(&l, c, account);
                    let before = fingerprint(&l);
                    let entries_before = entries(&l);
                    match l.assign_role(c.key, account) {
                        Ok(()) => {
                            prop_assert!(why.is_empty(), "accepted {o:?} but the spec refuses: {why:?}");
                            prop_assert_eq!(l.role_assignment(c), account);
                            let expect = match account {
                                Some(id) => Some(id),
                                None => acct(&l, c.code),
                            };
                            prop_assert_eq!(l.role_account(c), expect);
                            prop_assert_eq!(entries(&l), entries_before);
                        }
                        Err(e) => {
                            prop_assert_eq!(fingerprint(&l), before, "refusal changed the books");
                            prop_assert!(
                                why.contains(&reason(&e)),
                                "refused {:?} with {:?}; spec predicts {:?}", o, e, why
                            );
                        }
                    }
                }
                Op::Move(a, b, cents) => {
                    let (da, cb) = (pool[a], pool[b]);
                    let ph = |id| l.chart().get(id).unwrap().placeholder;
                    if da == cb || ph(da) || ph(cb) {
                        continue;
                    }
                    let amt = Decimal::new(i64::from(cents), 2).to_string();
                    post(&mut l, d(2026, 1, 1), &[(da, Side::Debit, &amt), (cb, Side::Credit, &amt)]);
                }
            }
            assert_role_invariants(&l);
        }
    }
}
