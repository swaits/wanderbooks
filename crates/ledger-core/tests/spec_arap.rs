//! Spec-derived tests for A/R and A/P: customers, vendors, invoices, bills,
//! payments, aging, and the control accounts they post to.
//!
//! Oracles are the spec blocks in the `ledger-core` interface (doc comments
//! on `Ledger::create_invoice`, `create_bill`, `receive_payment`, `pay_bill`,
//! `arap::aging_bucket`, the `ControlAccount` / `ControlAccountMismatch`
//! docs) and `docs/spec/rules.md` B-18. Expected values are computed by hand
//! from those, never from the code under test.

use std::str::FromStr;

use chrono::{Duration, NaiveDate};
use ledger_core::arap::{aging_bucket, derive_status, validate_due_date, validate_lines};
use ledger_core::{
    AccountId, AccountType, AgingRow, Bill, Currency, DocumentLine, DocumentStatus, EntryStatus,
    Invoice, Ledger, LedgerError, Side,
};
use proptest::prelude::*;
use rust_decimal::Decimal;

// ---------------------------------------------------------------- helpers

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).expect("decimal literal")
}

fn cents(c: i64) -> Decimal {
    Decimal::new(c, 2)
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

/// A ledger with only a cash account (1000), which the payment commands
/// need as their cash side.
fn books() -> (Ledger, AccountId) {
    let mut l = Ledger::new(Currency::USD);
    let cash = l
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .expect("cash account");
    (l, cash)
}

fn acct(l: &Ledger, code: &str) -> Option<AccountId> {
    l.chart().find_by_code(code).map(|a| a.id)
}

fn bal(l: &Ledger, code: &str) -> Decimal {
    match acct(l, code) {
        Some(id) => l.account_balance(id).expect("known account"),
        None => Decimal::ZERO,
    }
}

/// Everything observable about the books, deterministically ordered, so a
/// refusal can be checked to leave them unchanged.
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
        "\nnext: acct {} entry {} inv {} bill {} cust {} vend {}\n",
        l.next_account_id(),
        l.next_entry_id(),
        l.next_invoice_id(),
        l.next_bill_id(),
        l.next_customer_id(),
        l.next_vendor_id()
    ));
    for id in 0..l.next_entry_id() + 2 {
        if let Some(e) = l.get_entry(id) {
            out.push_str(&format!(
                "E{} {} {:?} {:?} {:?} {:?}\n",
                e.id(),
                e.date(),
                e.description(),
                e.status(),
                e.lines()
                    .iter()
                    .map(|x| (x.account, x.side, x.amount.amount))
                    .collect::<Vec<_>>(),
                l.entry_vendor(id)
            ));
        }
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
    for c in l.list_customers() {
        out.push_str(&format!("{c:?}\n"));
    }
    for v in l.list_vendors() {
        out.push_str(&format!("{v:?}\n"));
    }
    out
}

/// Expected bucket index from rules.md B-18, written independently:
/// 0 Current (on or before due), 1: 1-30, 2: 31-60, 3: 61-90, 4: 90+.
fn expected_bucket(due: NaiveDate, as_of: NaiveDate) -> usize {
    let days = (as_of - due).num_days();
    match days {
        i64::MIN..=0 => 0,
        1..=30 => 1,
        31..=60 => 2,
        61..=90 => 3,
        _ => 4,
    }
}

fn buckets(r: &AgingRow) -> [Decimal; 5] {
    [r.current, r.days_30, r.days_60, r.days_90, r.days_90_plus]
}

fn status_law(paid: Decimal, total: Decimal) -> DocumentStatus {
    if paid.is_zero() {
        DocumentStatus::Open
    } else if paid < total {
        DocumentStatus::Partial
    } else {
        DocumentStatus::Paid
    }
}

/// The (account, side, amount) lines of an entry.
fn entry_lines(l: &Ledger, id: u64) -> Vec<(AccountId, Side, Decimal)> {
    l.get_entry(id)
        .expect("entry exists")
        .lines()
        .iter()
        .map(|x| (x.account, x.side, x.amount.amount))
        .collect()
}

/// Net debit-minus-credit per account within one entry.
fn net_for(lines: &[(AccountId, Side, Decimal)], account: AccountId) -> Decimal {
    lines
        .iter()
        .filter(|(a, _, _)| *a == account)
        .map(|(_, s, amt)| match s {
            Side::Debit => *amt,
            Side::Credit => -*amt,
        })
        .sum()
}

// ------------------------------------------------- Invoice / Bill methods

/// Oracle: Invoice doc: total() is "Total of all lines", balance() is
/// "Amount still owed" = total - amount_paid.
#[test]
fn invoice_total_is_sum_of_lines_and_balance_is_total_minus_paid() {
    let inv = Invoice {
        id: 7,
        customer_id: 1,
        date: d(2026, 3, 1),
        due_date: d(2026, 3, 31),
        lines: vec![line("a", "100.10"), line("b", "0.05"), line("c", "49.85")],
        amount_paid: dec("60.00"),
        status: DocumentStatus::Partial,
        entry_id: 1,
    };
    assert_eq!(inv.total(), dec("150.00"));
    assert_eq!(inv.balance(), dec("90.00"));
}

/// Oracle: Bill doc: total() is the sum of lines, balance() total - paid.
#[test]
fn bill_total_is_sum_of_lines_and_balance_is_total_minus_paid() {
    let bill = Bill {
        id: 3,
        vendor_id: 2,
        date: d(2026, 3, 1),
        due_date: d(2026, 3, 1),
        lines: vec![line("x", "19.99"), line("y", "0.01")],
        amount_paid: dec("20.00"),
        status: DocumentStatus::Paid,
        entry_id: 9,
    };
    assert_eq!(bill.total(), dec("20.00"));
    assert_eq!(bill.balance(), Decimal::ZERO);
}

// ------------------------------------------------------ pure validators

/// Oracle: validate_lines doc: "at least one" line.
#[test]
fn validate_lines_refuses_no_lines() {
    assert_eq!(validate_lines(&[]), Err(LedgerError::EmptyDocumentLines));
}

/// Oracle: validate_lines doc: "all positive amounts" (zero is not positive).
#[test]
fn validate_lines_refuses_zero_and_negative_amounts() {
    assert_eq!(
        validate_lines(&[line("ok", "1.00"), line("zero", "0")]),
        Err(LedgerError::InvalidDocumentLineAmount(Decimal::ZERO))
    );
    assert_eq!(
        validate_lines(&[line("neg", "-0.01")]),
        Err(LedgerError::InvalidDocumentLineAmount(dec("-0.01")))
    );
}

/// Oracle: validate_lines doc: one or more positive lines are valid.
#[test]
fn validate_lines_accepts_positive_lines() {
    assert_eq!(validate_lines(&[line("a", "0.01")]), Ok(()));
    assert_eq!(
        validate_lines(&[line("a", "5"), line("b", "1000000")]),
        Ok(())
    );
}

/// Oracle: validate_due_date doc: due date "not before" the document date
/// (equal is allowed).
#[test]
fn validate_due_date_allows_same_day_and_later_refuses_earlier() {
    assert_eq!(validate_due_date(d(2026, 5, 1), d(2026, 5, 1)), Ok(()));
    assert_eq!(validate_due_date(d(2026, 5, 1), d(2026, 6, 1)), Ok(()));
    assert!(matches!(
        validate_due_date(d(2026, 5, 1), d(2026, 4, 30)),
        Err(LedgerError::DueBeforeDate(_))
    ));
}

/// Oracle: DocumentStatus docs: Open = nothing paid, Partial = partially
/// paid, Paid = fully paid.
#[test]
fn derive_status_examples() {
    assert_eq!(derive_status(dec("0"), dec("100")), DocumentStatus::Open);
    assert_eq!(
        derive_status(dec("0.01"), dec("100")),
        DocumentStatus::Partial
    );
    assert_eq!(
        derive_status(dec("99.99"), dec("100")),
        DocumentStatus::Partial
    );
    assert_eq!(
        derive_status(dec("100.00"), dec("100")),
        DocumentStatus::Paid
    );
}

// ------------------------------------------------------------ aging_bucket

/// Oracle: aging_bucket SCENARIO 1 and B-18 DECISION: due today is Current.
#[test]
fn aging_due_today_is_current() {
    let r = aging_bucket(d(2026, 10, 13), d(2026, 10, 13), dec("42.00"));
    assert_eq!(
        buckets(&r),
        [dec("42.00"), dec("0"), dec("0"), dec("0"), dec("0")]
    );
}

/// Oracle: aging_bucket SCENARIO 1: before the due date is Current.
#[test]
fn aging_not_yet_due_is_current() {
    let r = aging_bucket(d(2026, 12, 31), d(2026, 10, 13), dec("5"));
    assert_eq!(r.current, dec("5"));
    assert_eq!(r.total(), dec("5"));
}

/// Oracle: aging_bucket SCENARIOS 2-5 at each edge (1, 30, 31, 60, 61, 90,
/// 91 days past due), B-18.
#[test]
fn aging_boundaries_by_days_past_due() {
    let due = d(2026, 1, 1);
    let cases: [(i64, usize); 8] = [
        (1, 1),
        (30, 1),
        (31, 2),
        (60, 2),
        (61, 3),
        (90, 3),
        (91, 4),
        (400, 4),
    ];
    for (days, idx) in cases {
        let r = aging_bucket(due, due + Duration::days(days), dec("10.00"));
        let mut want = [Decimal::ZERO; 5];
        want[idx] = dec("10.00");
        assert_eq!(buckets(&r), want, "{days} days past due");
    }
}

/// Oracle: AgingRow::total doc ("Total outstanding") = sum of the buckets.
#[test]
fn aging_row_total_sums_buckets() {
    let r = AgingRow {
        name: "x".into(),
        current: dec("1"),
        days_30: dec("2"),
        days_60: dec("3.5"),
        days_90: dec("4"),
        days_90_plus: dec("0.25"),
    };
    assert_eq!(r.total(), dec("10.75"));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Oracle: aging_bucket INVARIANT 1 (whole balance in exactly one
    /// bucket, others zero) and SCENARIOS 1-5 (which bucket).
    #[test]
    fn prop_aging_one_bucket_matches_spec(
        offset in -400i64..400,
        bal_cents in 1i64..100_000_000,
    ) {
        let due = d(2026, 6, 15);
        let as_of = due + Duration::days(offset);
        let balance = cents(bal_cents);
        let r = aging_bucket(due, as_of, balance);
        let b = buckets(&r);
        let mut want = [Decimal::ZERO; 5];
        want[expected_bucket(due, as_of)] = balance;
        prop_assert_eq!(b, want);
        prop_assert_eq!(r.total(), balance);
    }

    /// Oracle: aging_bucket INVARIANT 2: never moves to a younger bucket as
    /// as_of advances.
    #[test]
    fn prop_aging_only_ages(
        a in -200i64..300,
        step in 0i64..200,
    ) {
        let due = d(2026, 6, 15);
        let r1 = aging_bucket(due, due + Duration::days(a), dec("1"));
        let r2 = aging_bucket(due, due + Duration::days(a + step), dec("1"));
        let idx = |r: &AgingRow| buckets(r).iter().position(|x| !x.is_zero()).unwrap();
        prop_assert!(idx(&r2) >= idx(&r1));
    }

    /// Oracle: DocumentStatus docs (Open / Partial / Paid exactly per paid:
    /// zero / between / equal).
    #[test]
    fn prop_derive_status_law(total_c in 1i64..10_000_000, frac in 0u32..=100) {
        let total = cents(total_c);
        let paid = match frac {
            0 => Decimal::ZERO,
            100 => total,
            f => (total * Decimal::from(f) / Decimal::from(100)).round_dp(2),
        };
        prop_assert_eq!(derive_status(paid, total), status_law(paid, total));
    }

    /// Oracle: validate_lines doc: valid iff non-empty and every amount > 0.
    #[test]
    fn prop_validate_lines_iff_nonempty_and_positive(
        amts in proptest::collection::vec(-1000i64..1000, 0..5)
    ) {
        let lines: Vec<DocumentLine> = amts
            .iter()
            .map(|c| DocumentLine { description: "l".into(), amount: cents(*c) })
            .collect();
        let ok = !amts.is_empty() && amts.iter().all(|c| *c > 0);
        prop_assert_eq!(validate_lines(&lines).is_ok(), ok);
    }
}

// ------------------------------------------------------ customers/vendors

/// Oracle: add_customer doc ("Returns its id") with get_customer /
/// list_customers ("in id order").
#[test]
fn add_customer_and_vendor_are_retrievable_in_id_order() {
    let (mut l, _) = books();
    let a = l.add_customer("Acme").unwrap();
    let b = l.add_customer("Beta").unwrap();
    assert_ne!(a, b);
    assert_eq!(l.get_customer(a).unwrap().name, "Acme");
    assert_eq!(l.get_customer(b).unwrap().name, "Beta");
    let names: Vec<_> = l.list_customers().iter().map(|c| c.name.clone()).collect();
    assert_eq!(names, vec!["Acme", "Beta"]);

    let v = l.add_vendor("Landlord").unwrap();
    let w = l.add_vendor("Hosting").unwrap();
    assert_ne!(v, w);
    assert_eq!(l.get_vendor(v).unwrap().name, "Landlord");
    let vn: Vec<_> = l.list_vendors().iter().map(|x| x.name.clone()).collect();
    assert_eq!(vn, vec!["Landlord", "Hosting"]);
}

/// Oracle: next_customer_id / next_vendor_id docs ("the id the next ...
/// added will receive").
#[test]
fn add_customer_and_vendor_use_the_announced_next_id() {
    let (mut l, _) = books();
    let nc = l.next_customer_id();
    assert_eq!(l.add_customer("A").unwrap(), nc);
    let nv = l.next_vendor_id();
    assert_eq!(l.add_vendor("V").unwrap(), nv);
}

// ------------------------------------------------------------ invoices

/// Oracle: create_invoice INVARIANT 1 (Dr 1200 / Cr 4000 for the sum of the
/// lines, dated the invoice date) and INVARIANT 2 (starts open, nothing
/// paid); control accounts doc: missing 1200/4000 are created.
#[test]
fn create_invoice_posts_dr_receivables_cr_revenue_for_the_line_sum() {
    let (mut l, _) = books();
    let c = l.add_customer("Acme").unwrap();
    assert!(acct(&l, "1200").is_none() && acct(&l, "4000").is_none());
    let next = l.next_invoice_id();
    let id = l
        .create_invoice(
            c,
            d(2026, 3, 1),
            d(2026, 3, 31),
            vec![line("design", "1000.00"), line("hosting", "234.56")],
        )
        .unwrap();
    assert_eq!(id, next);

    let ar = l.chart().find_by_code("1200").expect("1200 created");
    assert_eq!(ar.account_type, AccountType::Asset);
    assert!(!ar.placeholder);
    let rev = l.chart().find_by_code("4000").expect("4000 created");
    assert_eq!(rev.account_type, AccountType::Revenue);
    assert!(!rev.placeholder);

    let inv = l.get_invoice(id).unwrap().clone();
    assert_eq!(inv.customer_id, c);
    assert_eq!(inv.total(), dec("1234.56"));
    assert_eq!(inv.amount_paid, Decimal::ZERO);
    assert_eq!(inv.status, DocumentStatus::Open);
    assert_eq!(inv.balance(), dec("1234.56"));

    let e = l.get_entry(inv.entry_id).unwrap();
    assert_eq!(e.status(), EntryStatus::Posted);
    assert_eq!(e.date(), d(2026, 3, 1));
    let lines = entry_lines(&l, inv.entry_id);
    assert!(lines.iter().all(|(a, _, _)| *a == ar.id || *a == rev.id));
    assert_eq!(net_for(&lines, ar.id), dec("1234.56"));
    assert_eq!(net_for(&lines, rev.id), dec("-1234.56"));

    assert_eq!(bal(&l, "1200"), dec("1234.56"));
    assert_eq!(bal(&l, "4000"), dec("1234.56"));
}

/// Oracle: control accounts doc: an existing postable account of the right
/// type is the one used ("find (or create)"); nothing new is added.
#[test]
fn create_invoice_uses_existing_right_type_controls() {
    let (mut l, _) = books();
    let ar = l
        .add_account("1200", "Trade Debtors", AccountType::Asset, None, false)
        .unwrap();
    let rev = l
        .add_account("4000", "Sales", AccountType::Revenue, None, false)
        .unwrap();
    let n = l.chart().len();
    let c = l.add_customer("Acme").unwrap();
    let id = l
        .create_invoice(c, d(2026, 3, 1), d(2026, 3, 1), vec![line("x", "10")])
        .unwrap();
    assert_eq!(l.chart().len(), n);
    let lines = entry_lines(&l, l.get_invoice(id).unwrap().entry_id);
    assert_eq!(net_for(&lines, ar), dec("10"));
    assert_eq!(net_for(&lines, rev), dec("-10"));
}

/// Oracle: create_invoice SCENARIO 1 (unknown customer) + INVARIANT 3.
#[test]
fn create_invoice_refuses_unknown_customer_books_unchanged() {
    let (mut l, _) = books();
    let before = fingerprint(&l);
    let r = l.create_invoice(999, d(2026, 3, 1), d(2026, 3, 2), vec![line("x", "1")]);
    assert_eq!(r, Err(LedgerError::UnknownCustomer(999)));
    assert_eq!(fingerprint(&l), before);
}

/// Oracle: create_invoice SCENARIO 1 (line amount not positive; no lines,
/// per validate_lines) + INVARIANT 3.
#[test]
fn create_invoice_refuses_bad_lines_books_unchanged() {
    let (mut l, _) = books();
    let c = l.add_customer("Acme").unwrap();
    let before = fingerprint(&l);
    let r = l.create_invoice(
        c,
        d(2026, 3, 1),
        d(2026, 3, 2),
        vec![line("x", "5"), line("y", "0")],
    );
    assert_eq!(
        r,
        Err(LedgerError::InvalidDocumentLineAmount(Decimal::ZERO))
    );
    let r = l.create_invoice(c, d(2026, 3, 1), d(2026, 3, 2), vec![line("x", "-5")]);
    assert_eq!(r, Err(LedgerError::InvalidDocumentLineAmount(dec("-5"))));
    let r = l.create_invoice(c, d(2026, 3, 1), d(2026, 3, 2), vec![]);
    assert_eq!(r, Err(LedgerError::EmptyDocumentLines));
    assert_eq!(fingerprint(&l), before);
}

/// Oracle: create_invoice SCENARIO 1 (due date before invoice date) +
/// INVARIANT 3.
#[test]
fn create_invoice_refuses_due_before_date_books_unchanged() {
    let (mut l, _) = books();
    let c = l.add_customer("Acme").unwrap();
    let before = fingerprint(&l);
    let r = l.create_invoice(c, d(2026, 3, 2), d(2026, 3, 1), vec![line("x", "1")]);
    assert!(matches!(r, Err(LedgerError::DueBeforeDate(_))), "{r:?}");
    assert_eq!(fingerprint(&l), before);
}

/// Closes January 2026 on `l`.
///
/// ASSUMED: closing a period with nothing in it still marks it closed
/// (close_period returns Ok(None)); the precondition is asserted.
fn close_january(l: &mut Ledger) {
    let p = l
        .create_period("2026-01", d(2026, 1, 1), d(2026, 1, 31))
        .unwrap();
    l.close_period(p).unwrap();
    assert!(l.closed_period_containing(d(2026, 1, 15)).is_some());
}

/// Oracle: create_invoice SCENARIO 1 (date in a closed period) + INVARIANT 3.
#[test]
fn create_invoice_refuses_closed_period_books_unchanged() {
    let (mut l, _) = books();
    let c = l.add_customer("Acme").unwrap();
    close_january(&mut l);
    let before = fingerprint(&l);
    let r = l.create_invoice(c, d(2026, 1, 15), d(2026, 2, 15), vec![line("x", "1")]);
    assert!(matches!(r, Err(LedgerError::ClosedPeriod(..))), "{r:?}");
    assert_eq!(fingerprint(&l), before);
}

/// Oracle: create_bill SCENARIO 1 (date in a closed period) + INVARIANT 2.
#[test]
fn create_bill_refuses_closed_period_books_unchanged() {
    let (mut l, _) = books();
    let v = l.add_vendor("Hosting").unwrap();
    close_january(&mut l);
    let before = fingerprint(&l);
    let r = l.create_bill(v, d(2026, 1, 15), d(2026, 2, 15), vec![line("x", "1")]);
    assert!(matches!(r, Err(LedgerError::ClosedPeriod(..))), "{r:?}");
    assert_eq!(fingerprint(&l), before);
}

// ------------------------------------------------------ control accounts

fn expect_mismatch(
    r: Result<u64, LedgerError>,
    code: &str,
    name: &str,
    expected: AccountType,
    found: AccountType,
    placeholder: bool,
) {
    match r {
        Err(LedgerError::ControlAccountMismatch {
            code: c,
            name: n,
            expected: e,
            found: f,
            placeholder: p,
            ..
        }) => {
            assert_eq!(c, code);
            assert_eq!(n, name);
            assert_eq!(e, expected);
            assert_eq!(f, found);
            assert_eq!(p, placeholder);
        }
        other => panic!("expected ControlAccountMismatch for {code}, got {other:?}"),
    }
}

/// Oracle: ControlAccountMismatch doc: 1200 of the wrong type is refused;
/// create_invoice INVARIANT 3: books unchanged (4000 not created either).
#[test]
fn invoice_refuses_wrong_type_receivables_books_unchanged() {
    let (mut l, _) = books();
    l.add_account("1200", "Loan", AccountType::Liability, None, false)
        .unwrap();
    let c = l.add_customer("Acme").unwrap();
    let before = fingerprint(&l);
    let r = l.create_invoice(c, d(2026, 3, 1), d(2026, 3, 1), vec![line("x", "1")]);
    expect_mismatch(
        r,
        "1200",
        "Loan",
        AccountType::Asset,
        AccountType::Liability,
        false,
    );
    assert!(acct(&l, "4000").is_none());
    assert_eq!(fingerprint(&l), before);
}

/// Oracle: ControlAccountMismatch doc: a placeholder 1200 (right type) is
/// refused, with placeholder = true; books unchanged.
#[test]
fn invoice_refuses_placeholder_receivables_books_unchanged() {
    let (mut l, _) = books();
    l.add_account("1200", "Receivables", AccountType::Asset, None, true)
        .unwrap();
    let c = l.add_customer("Acme").unwrap();
    let before = fingerprint(&l);
    let r = l.create_invoice(c, d(2026, 3, 1), d(2026, 3, 1), vec![line("x", "1")]);
    expect_mismatch(
        r,
        "1200",
        "Receivables",
        AccountType::Asset,
        AccountType::Asset,
        true,
    );
    assert_eq!(fingerprint(&l), before);
}

/// Oracle: ControlAccountMismatch doc: 4000 must be a postable Revenue
/// account; books unchanged (1200 not created either).
#[test]
fn invoice_refuses_wrong_type_revenue_books_unchanged() {
    let (mut l, _) = books();
    l.add_account("4000", "Rent", AccountType::Expense, None, false)
        .unwrap();
    let c = l.add_customer("Acme").unwrap();
    let before = fingerprint(&l);
    let r = l.create_invoice(c, d(2026, 3, 1), d(2026, 3, 1), vec![line("x", "1")]);
    expect_mismatch(
        r,
        "4000",
        "Rent",
        AccountType::Revenue,
        AccountType::Expense,
        false,
    );
    assert!(acct(&l, "1200").is_none());
    assert_eq!(fingerprint(&l), before);
}

/// Oracle: ControlAccountMismatch doc: 2000 must be a postable Liability;
/// create_bill INVARIANT 2: books unchanged (6999 not created).
#[test]
fn bill_refuses_wrong_type_payables_books_unchanged() {
    let (mut l, _) = books();
    l.add_account("2000", "Savings", AccountType::Asset, None, false)
        .unwrap();
    let v = l.add_vendor("Hosting").unwrap();
    let before = fingerprint(&l);
    let r = l.create_bill(v, d(2026, 3, 1), d(2026, 3, 1), vec![line("x", "1")]);
    expect_mismatch(
        r,
        "2000",
        "Savings",
        AccountType::Liability,
        AccountType::Asset,
        false,
    );
    assert!(acct(&l, "6999").is_none());
    assert_eq!(fingerprint(&l), before);
}

/// Oracle: ControlAccountMismatch doc: a placeholder 6999 is refused; books
/// unchanged (2000 not created).
#[test]
fn bill_refuses_placeholder_general_expenses_books_unchanged() {
    let (mut l, _) = books();
    l.add_account("6999", "Other", AccountType::Expense, None, true)
        .unwrap();
    let v = l.add_vendor("Hosting").unwrap();
    let before = fingerprint(&l);
    let r = l.create_bill(v, d(2026, 3, 1), d(2026, 3, 1), vec![line("x", "1")]);
    expect_mismatch(
        r,
        "6999",
        "Other",
        AccountType::Expense,
        AccountType::Expense,
        true,
    );
    assert!(acct(&l, "2000").is_none());
    assert_eq!(fingerprint(&l), before);
}

/// Oracle: ControlAccountMismatch doc: 6999 of the wrong type is refused.
#[test]
fn bill_refuses_wrong_type_general_expenses_books_unchanged() {
    let (mut l, _) = books();
    l.add_account("6999", "Misc income", AccountType::Revenue, None, false)
        .unwrap();
    let v = l.add_vendor("Hosting").unwrap();
    let before = fingerprint(&l);
    let r = l.create_bill(v, d(2026, 3, 1), d(2026, 3, 1), vec![line("x", "1")]);
    expect_mismatch(
        r,
        "6999",
        "Misc income",
        AccountType::Expense,
        AccountType::Revenue,
        false,
    );
    assert_eq!(fingerprint(&l), before);
}

// --------------------------------------------------------------- payments

/// Oracle: receive_payment INVARIANTS 1-2 (Dr cash / Cr 1200 for the
/// amount; paid rises; Partial when partly, Paid exactly when fully).
#[test]
fn receive_payment_partial_then_full() {
    let (mut l, cash) = books();
    let c = l.add_customer("Acme").unwrap();
    let id = l
        .create_invoice(c, d(2026, 3, 1), d(2026, 3, 31), vec![line("x", "300.00")])
        .unwrap();
    let ar = acct(&l, "1200").unwrap();

    let e1 = l
        .receive_payment(id, d(2026, 3, 10), dec("100.00"), cash)
        .unwrap();
    let lines = entry_lines(&l, e1);
    assert_eq!(l.get_entry(e1).unwrap().status(), EntryStatus::Posted);
    assert_eq!(l.get_entry(e1).unwrap().date(), d(2026, 3, 10));
    assert!(lines.iter().all(|(a, _, _)| *a == ar || *a == cash));
    assert_eq!(net_for(&lines, cash), dec("100.00"));
    assert_eq!(net_for(&lines, ar), dec("-100.00"));
    let inv = l.get_invoice(id).unwrap();
    assert_eq!(inv.amount_paid, dec("100.00"));
    assert_eq!(inv.status, DocumentStatus::Partial);
    assert_eq!(inv.balance(), dec("200.00"));
    assert_eq!(bal(&l, "1200"), dec("200.00"));
    assert_eq!(bal(&l, "1000"), dec("100.00"));

    l.receive_payment(id, d(2026, 3, 20), dec("200.00"), cash)
        .unwrap();
    let inv = l.get_invoice(id).unwrap();
    assert_eq!(inv.amount_paid, dec("300.00"));
    assert_eq!(inv.status, DocumentStatus::Paid);
    assert_eq!(inv.balance(), Decimal::ZERO);
    assert_eq!(bal(&l, "1200"), Decimal::ZERO);
    assert_eq!(bal(&l, "1000"), dec("300.00"));
}

/// Oracle: receive_payment SCENARIO 1 (amount not positive) — refusal leaves
/// the books unchanged (B-8 general refusal rule, as for post_entry).
#[test]
fn receive_payment_refuses_non_positive_amount() {
    let (mut l, cash) = books();
    let c = l.add_customer("Acme").unwrap();
    let id = l
        .create_invoice(c, d(2026, 3, 1), d(2026, 3, 31), vec![line("x", "50")])
        .unwrap();
    let before = fingerprint(&l);
    assert_eq!(
        l.receive_payment(id, d(2026, 3, 2), Decimal::ZERO, cash),
        Err(LedgerError::InvalidPaymentAmount(Decimal::ZERO))
    );
    assert_eq!(
        l.receive_payment(id, d(2026, 3, 2), dec("-1"), cash),
        Err(LedgerError::InvalidPaymentAmount(dec("-1")))
    );
    assert_eq!(fingerprint(&l), before);
}

/// Oracle: receive_payment SCENARIO 2 (exceeds what's owed: Overpayment;
/// never let paid exceed the total), including after full payment.
#[test]
fn receive_payment_refuses_overpayment() {
    let (mut l, cash) = books();
    let c = l.add_customer("Acme").unwrap();
    let id = l
        .create_invoice(c, d(2026, 3, 1), d(2026, 3, 31), vec![line("x", "50.00")])
        .unwrap();
    l.receive_payment(id, d(2026, 3, 2), dec("20.00"), cash)
        .unwrap();
    let before = fingerprint(&l);
    assert_eq!(
        l.receive_payment(id, d(2026, 3, 3), dec("30.01"), cash),
        Err(LedgerError::Overpayment(dec("30.01"), dec("30.00")))
    );
    assert_eq!(fingerprint(&l), before);
    l.receive_payment(id, d(2026, 3, 3), dec("30.00"), cash)
        .unwrap();
    let before = fingerprint(&l);
    assert_eq!(
        l.receive_payment(id, d(2026, 3, 4), dec("0.01"), cash),
        Err(LedgerError::Overpayment(dec("0.01"), Decimal::ZERO))
    );
    assert_eq!(fingerprint(&l), before);
    assert_eq!(l.get_invoice(id).unwrap().amount_paid, dec("50.00"));
}

/// Oracle: UnknownInvoice / UnknownBill error docs ("the referenced ...
/// does not exist"); a refusal leaves the books unchanged.
#[test]
fn payments_refuse_unknown_documents() {
    let (mut l, cash) = books();
    let before = fingerprint(&l);
    assert_eq!(
        l.receive_payment(77, d(2026, 3, 2), dec("1"), cash),
        Err(LedgerError::UnknownInvoice(77))
    );
    assert_eq!(
        l.pay_bill(78, d(2026, 3, 2), dec("1"), cash),
        Err(LedgerError::UnknownBill(78))
    );
    assert_eq!(fingerprint(&l), before);
}

// ------------------------------------------------------------------ bills

/// Oracle: create_bill INVARIANT 1 (Dr 6999 / Cr 2000 for the sum of the
/// lines, dated the bill date, tagged with the vendor); controls created.
#[test]
fn create_bill_posts_dr_general_expenses_cr_payables_tagged() {
    let (mut l, _) = books();
    let v = l.add_vendor("Hosting").unwrap();
    let id = l
        .create_bill(
            v,
            d(2026, 4, 2),
            d(2026, 5, 2),
            vec![line("vps", "40.00"), line("backup", "9.50")],
        )
        .unwrap();
    let ap = l.chart().find_by_code("2000").expect("2000 created");
    assert_eq!(ap.account_type, AccountType::Liability);
    assert!(!ap.placeholder);
    let ex = l.chart().find_by_code("6999").expect("6999 created");
    assert_eq!(ex.account_type, AccountType::Expense);
    assert!(!ex.placeholder);

    let bill = l.get_bill(id).unwrap().clone();
    assert_eq!(bill.vendor_id, v);
    assert_eq!(bill.total(), dec("49.50"));
    assert_eq!(bill.amount_paid, Decimal::ZERO);
    assert_eq!(bill.status, DocumentStatus::Open);

    let e = l.get_entry(bill.entry_id).unwrap();
    assert_eq!(e.status(), EntryStatus::Posted);
    assert_eq!(e.date(), d(2026, 4, 2));
    let lines = entry_lines(&l, bill.entry_id);
    assert!(lines.iter().all(|(a, _, _)| *a == ap.id || *a == ex.id));
    assert_eq!(net_for(&lines, ex.id), dec("49.50"));
    assert_eq!(net_for(&lines, ap.id), dec("-49.50"));
    assert_eq!(l.entry_vendor(bill.entry_id), Some(v));
    assert_eq!(bal(&l, "2000"), dec("49.50"));
    assert_eq!(bal(&l, "6999"), dec("49.50"));
}

/// Oracle: create_bill SCENARIO 1 (unknown vendor, bad line, due before
/// date) + INVARIANT 2.
#[test]
fn create_bill_refusals_leave_books_unchanged() {
    let (mut l, _) = books();
    let v = l.add_vendor("Hosting").unwrap();
    let before = fingerprint(&l);
    assert_eq!(
        l.create_bill(555, d(2026, 4, 1), d(2026, 4, 1), vec![line("x", "1")]),
        Err(LedgerError::UnknownVendor(555))
    );
    assert_eq!(
        l.create_bill(v, d(2026, 4, 1), d(2026, 4, 1), vec![line("x", "-2")]),
        Err(LedgerError::InvalidDocumentLineAmount(dec("-2")))
    );
    assert_eq!(
        l.create_bill(v, d(2026, 4, 1), d(2026, 4, 1), vec![]),
        Err(LedgerError::EmptyDocumentLines)
    );
    let r = l.create_bill(v, d(2026, 4, 2), d(2026, 4, 1), vec![line("x", "1")]);
    assert!(matches!(r, Err(LedgerError::DueBeforeDate(_))), "{r:?}");
    assert_eq!(fingerprint(&l), before);
}

/// Oracle: pay_bill INVARIANTS 1-2 (Dr 2000 / Cr cash for the amount,
/// tagged with the vendor; paid rises) and DocumentStatus docs.
#[test]
fn pay_bill_partial_then_full() {
    let (mut l, cash) = books();
    let v = l.add_vendor("Hosting").unwrap();
    let id = l
        .create_bill(v, d(2026, 4, 1), d(2026, 4, 30), vec![line("x", "80.00")])
        .unwrap();
    let ap = acct(&l, "2000").unwrap();
    let e = l.pay_bill(id, d(2026, 4, 5), dec("30.00"), cash).unwrap();
    let lines = entry_lines(&l, e);
    assert_eq!(l.get_entry(e).unwrap().status(), EntryStatus::Posted);
    assert_eq!(l.get_entry(e).unwrap().date(), d(2026, 4, 5));
    assert!(lines.iter().all(|(a, _, _)| *a == ap || *a == cash));
    assert_eq!(net_for(&lines, ap), dec("30.00"));
    assert_eq!(net_for(&lines, cash), dec("-30.00"));
    assert_eq!(l.entry_vendor(e), Some(v));
    let b = l.get_bill(id).unwrap();
    assert_eq!(b.amount_paid, dec("30.00"));
    assert_eq!(b.status, DocumentStatus::Partial);
    assert_eq!(bal(&l, "2000"), dec("50.00"));

    l.pay_bill(id, d(2026, 4, 6), dec("50.00"), cash).unwrap();
    let b = l.get_bill(id).unwrap();
    assert_eq!(b.status, DocumentStatus::Paid);
    assert_eq!(b.balance(), Decimal::ZERO);
    assert_eq!(bal(&l, "2000"), Decimal::ZERO);
    assert_eq!(bal(&l, "1000"), dec("-80.00"));
}

/// Oracle: pay_bill SCENARIO 1 (non-positive or exceeding what's owed:
/// refuse); refusal leaves the books unchanged.
#[test]
fn pay_bill_refuses_bad_amounts_books_unchanged() {
    let (mut l, cash) = books();
    let v = l.add_vendor("Hosting").unwrap();
    let id = l
        .create_bill(v, d(2026, 4, 1), d(2026, 4, 30), vec![line("x", "80.00")])
        .unwrap();
    let before = fingerprint(&l);
    assert_eq!(
        l.pay_bill(id, d(2026, 4, 2), Decimal::ZERO, cash),
        Err(LedgerError::InvalidPaymentAmount(Decimal::ZERO))
    );
    assert_eq!(
        l.pay_bill(id, d(2026, 4, 2), dec("-3"), cash),
        Err(LedgerError::InvalidPaymentAmount(dec("-3")))
    );
    assert_eq!(
        l.pay_bill(id, d(2026, 4, 2), dec("80.01"), cash),
        Err(LedgerError::Overpayment(dec("80.01"), dec("80.00")))
    );
    assert_eq!(fingerprint(&l), before);
}

// ------------------------------------------------------------------ aging

/// Oracle: ar_aging doc (one row per customer with an outstanding balance,
/// bucketed by days past due) and B-18 buckets; fully paid customers get no
/// row.
///
/// ASSUMED: rows carry the customer's name and their order is unspecified
/// (the test sorts by name).
#[test]
fn ar_aging_example() {
    let (mut l, cash) = books();
    let a = l.add_customer("Acme").unwrap();
    let b = l.add_customer("Beta").unwrap();
    let z = l.add_customer("Zeta").unwrap();
    let as_of = d(2026, 10, 13);
    // Acme: due today (Current) 100, 45 days late 200 with 50 paid (31-60).
    l.create_invoice(a, d(2026, 9, 13), as_of, vec![line("x", "100")])
        .unwrap();
    let i2 = l
        .create_invoice(
            a,
            d(2026, 8, 1),
            as_of - Duration::days(45),
            vec![line("y", "200")],
        )
        .unwrap();
    l.receive_payment(i2, d(2026, 9, 1), dec("50"), cash)
        .unwrap();
    // Beta: 91 days late (90+) 10.
    l.create_invoice(
        b,
        d(2026, 6, 1),
        as_of - Duration::days(91),
        vec![line("z", "10")],
    )
    .unwrap();
    // Zeta: fully paid, no row.
    let i4 = l
        .create_invoice(z, d(2026, 9, 1), d(2026, 9, 2), vec![line("w", "7")])
        .unwrap();
    l.receive_payment(i4, d(2026, 9, 2), dec("7"), cash)
        .unwrap();

    let mut rows = l.ar_aging(as_of);
    rows.sort_by(|x, y| x.name.cmp(&y.name));
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].name, "Acme");
    assert_eq!(
        buckets(&rows[0]),
        [dec("100"), dec("0"), dec("150"), dec("0"), dec("0")]
    );
    assert_eq!(rows[1].name, "Beta");
    assert_eq!(
        buckets(&rows[1]),
        [dec("0"), dec("0"), dec("0"), dec("0"), dec("10")]
    );
}

/// Oracle: ap_aging doc (one row per vendor with an outstanding balance) and
/// B-18 (1-30 and 61-90 buckets).
#[test]
fn ap_aging_example() {
    let (mut l, cash) = books();
    let v = l.add_vendor("Hosting").unwrap();
    let w = l.add_vendor("Paid Up").unwrap();
    let as_of = d(2026, 10, 13);
    l.create_bill(
        v,
        d(2026, 9, 1),
        as_of - Duration::days(30),
        vec![line("a", "12.34")],
    )
    .unwrap();
    l.create_bill(
        v,
        d(2026, 7, 1),
        as_of - Duration::days(61),
        vec![line("b", "1.00")],
    )
    .unwrap();
    let pb = l
        .create_bill(w, d(2026, 9, 1), d(2026, 9, 30), vec![line("c", "5")])
        .unwrap();
    l.pay_bill(pb, d(2026, 9, 5), dec("5"), cash).unwrap();
    let rows = l.ap_aging(as_of);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name, "Hosting");
    assert_eq!(
        buckets(&rows[0]),
        [dec("0"), dec("12.34"), dec("0"), dec("1.00"), dec("0")]
    );
}

// ------------------------------------------------- sequence properties

#[derive(Clone, Debug)]
struct Doc {
    who: usize,
    line_cents: Vec<i64>,
    due_offset: i64,
}

fn doc_strategy() -> impl Strategy<Value = Doc> {
    (
        0usize..3,
        proptest::collection::vec(1i64..500_000, 1..4),
        0i64..60,
    )
        .prop_map(|(who, line_cents, due_offset)| Doc {
            who,
            line_cents,
            due_offset,
        })
}

/// Payment request: (document index, percent of the remaining balance,
/// 1..=130 — above 100 is an overpayment attempt).
fn pay_strategy() -> impl Strategy<Value = Vec<(usize, u32)>> {
    proptest::collection::vec((0usize..8, 1u32..=130), 0..16)
}

fn doc_lines(doc: &Doc) -> Vec<DocumentLine> {
    doc.line_cents
        .iter()
        .map(|c| DocumentLine {
            description: "item".into(),
            amount: cents(*c),
        })
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Oracles: receive_payment INVARIANT 2 and SCENARIOS 1-2 (paid rises by
    /// the amount, never exceeds total, Overpayment refused with books
    /// unchanged); DocumentStatus docs (status law); Invoice::balance doc;
    /// conservation (1200 balance = sum of open invoice balances, 4000 =
    /// sum of invoice totals, cash = sum received, since every movement on
    /// them comes through invoices and payments); ar_aging doc + B-18
    /// (each open balance in exactly one bucket; buckets sum to open
    /// balances; due today is Current).
    #[test]
    fn prop_invoices_and_payments_conserve(
        docs in proptest::collection::vec(doc_strategy(), 1..6),
        pays in pay_strategy(),
    ) {
        let (mut l, cash) = books();
        let names = ["Alpha", "Bravo", "Charlie"];
        let custs: Vec<_> = names.iter().map(|n| l.add_customer(n).unwrap()).collect();
        let base = d(2026, 3, 1);
        let mut ids = Vec::new();
        let mut expect_paid: Vec<Decimal> = Vec::new();
        let mut totals: Vec<Decimal> = Vec::new();
        for doc in &docs {
            let id = l
                .create_invoice(custs[doc.who], base, base + Duration::days(doc.due_offset), doc_lines(doc))
                .unwrap();
            ids.push(id);
            expect_paid.push(Decimal::ZERO);
            totals.push(doc.line_cents.iter().map(|c| cents(*c)).sum());
        }
        let mut received = Decimal::ZERO;
        for (k, pct) in &pays {
            let i = k % ids.len();
            let remaining = totals[i] - expect_paid[i];
            let amount = if *pct > 100 {
                remaining + cents(i64::from(*pct - 100))
            } else {
                (remaining * Decimal::from(*pct) / Decimal::from(100)).round_dp(2)
            };
            let before = fingerprint(&l);
            let r = l.receive_payment(ids[i], base + Duration::days(5), amount, cash);
            if amount <= Decimal::ZERO {
                prop_assert_eq!(r, Err(LedgerError::InvalidPaymentAmount(amount)));
                prop_assert_eq!(fingerprint(&l), before);
            } else if amount > remaining {
                prop_assert_eq!(r, Err(LedgerError::Overpayment(amount, remaining)));
                prop_assert_eq!(fingerprint(&l), before);
            } else {
                prop_assert!(r.is_ok(), "{:?}", r);
                expect_paid[i] += amount;
                received += amount;
            }
        }

        let mut open_sum = Decimal::ZERO;
        for (i, id) in ids.iter().enumerate() {
            let inv = l.get_invoice(*id).unwrap();
            prop_assert_eq!(inv.total(), totals[i]);
            prop_assert_eq!(inv.amount_paid, expect_paid[i]);
            prop_assert_eq!(inv.balance(), totals[i] - expect_paid[i]);
            prop_assert!(inv.balance() >= Decimal::ZERO);
            prop_assert_eq!(inv.status, status_law(expect_paid[i], totals[i]));
            open_sum += inv.balance();
        }
        prop_assert_eq!(bal(&l, "1200"), open_sum);
        prop_assert_eq!(bal(&l, "4000"), totals.iter().copied().sum::<Decimal>());
        prop_assert_eq!(l.account_balance(cash).unwrap(), received);

        // Aging as of a date spanning every bucket edge in turn.
        for as_of_off in [0i64, 1, 30, 31, 60, 61, 90, 91, 200] {
            let as_of = base + Duration::days(as_of_off);
            let rows = l.ar_aging(as_of);
            let mut want: std::collections::BTreeMap<String, [Decimal; 5]> = Default::default();
            for (i, doc) in docs.iter().enumerate() {
                // Payments (dated base + 5) count only once made: aging
                // as of a date sees what existed then (owner, 2026-10-13).
                let paid = if as_of >= base + Duration::days(5) { expect_paid[i] } else { Decimal::ZERO };
                let open = totals[i] - paid;
                if open.is_zero() {
                    continue;
                }
                let due = base + Duration::days(doc.due_offset);
                want.entry(names[doc.who].to_string()).or_insert([Decimal::ZERO; 5])
                    [expected_bucket(due, as_of)] += open;
            }
            let got: std::collections::BTreeMap<String, [Decimal; 5]> =
                rows.iter().map(|r| (r.name.clone(), buckets(r))).collect();
            prop_assert_eq!(rows.len(), got.len(), "one row per customer");
            prop_assert_eq!(&got, &want);
            let row_sum: Decimal = rows.iter().map(|r| r.total()).sum();
            let want_sum: Decimal = want.values().flatten().copied().sum();
            prop_assert_eq!(row_sum, want_sum);
        }
    }

    /// Oracles: pay_bill INVARIANTS 1-2 and SCENARIO 1; DocumentStatus docs;
    /// Bill::balance doc; conservation (2000 = sum of open bill balances,
    /// 6999 = sum of bill totals, cash = -sum paid); every payment entry
    /// tagged with its vendor; ap_aging doc + B-18.
    #[test]
    fn prop_bills_and_payments_conserve(
        docs in proptest::collection::vec(doc_strategy(), 1..6),
        pays in pay_strategy(),
    ) {
        let (mut l, cash) = books();
        let names = ["Vendor A", "Vendor B", "Vendor C"];
        let vends: Vec<_> = names.iter().map(|n| l.add_vendor(n).unwrap()).collect();
        let base = d(2026, 5, 1);
        let mut ids = Vec::new();
        let mut expect_paid: Vec<Decimal> = Vec::new();
        let mut totals: Vec<Decimal> = Vec::new();
        for doc in &docs {
            let id = l
                .create_bill(vends[doc.who], base, base + Duration::days(doc.due_offset), doc_lines(doc))
                .unwrap();
            prop_assert_eq!(l.entry_vendor(l.get_bill(id).unwrap().entry_id), Some(vends[doc.who]));
            ids.push(id);
            expect_paid.push(Decimal::ZERO);
            totals.push(doc.line_cents.iter().map(|c| cents(*c)).sum());
        }
        let mut paid_out = Decimal::ZERO;
        for (k, pct) in &pays {
            let i = k % ids.len();
            let remaining = totals[i] - expect_paid[i];
            let amount = if *pct > 100 {
                remaining + cents(i64::from(*pct - 100))
            } else {
                (remaining * Decimal::from(*pct) / Decimal::from(100)).round_dp(2)
            };
            let before = fingerprint(&l);
            let r = l.pay_bill(ids[i], base + Duration::days(3), amount, cash);
            if amount <= Decimal::ZERO {
                prop_assert_eq!(r, Err(LedgerError::InvalidPaymentAmount(amount)));
                prop_assert_eq!(fingerprint(&l), before);
            } else if amount > remaining {
                prop_assert!(r.is_err(), "overpayment accepted: {:?}", r);
                prop_assert_eq!(fingerprint(&l), before);
            } else {
                let e = r.unwrap();
                prop_assert_eq!(l.entry_vendor(e), Some(vends[docs[i].who]));
                expect_paid[i] += amount;
                paid_out += amount;
            }
        }

        let mut open_sum = Decimal::ZERO;
        for (i, id) in ids.iter().enumerate() {
            let b = l.get_bill(*id).unwrap();
            prop_assert_eq!(b.total(), totals[i]);
            prop_assert_eq!(b.amount_paid, expect_paid[i]);
            prop_assert_eq!(b.balance(), totals[i] - expect_paid[i]);
            prop_assert_eq!(b.status, status_law(expect_paid[i], totals[i]));
            open_sum += b.balance();
        }
        prop_assert_eq!(bal(&l, "2000"), open_sum);
        prop_assert_eq!(bal(&l, "6999"), totals.iter().copied().sum::<Decimal>());
        prop_assert_eq!(l.account_balance(cash).unwrap(), -paid_out);

        for as_of_off in [0i64, 15, 45, 75, 120] {
            let as_of = base + Duration::days(as_of_off);
            let rows = l.ap_aging(as_of);
            let mut want: std::collections::BTreeMap<String, [Decimal; 5]> = Default::default();
            for (i, doc) in docs.iter().enumerate() {
                // Payments (dated base + 3) count only once made.
                let paid = if as_of >= base + Duration::days(3) { expect_paid[i] } else { Decimal::ZERO };
                let open = totals[i] - paid;
                if open.is_zero() {
                    continue;
                }
                let due = base + Duration::days(doc.due_offset);
                want.entry(names[doc.who].to_string()).or_insert([Decimal::ZERO; 5])
                    [expected_bucket(due, as_of)] += open;
            }
            let got: std::collections::BTreeMap<String, [Decimal; 5]> =
                rows.iter().map(|r| (r.name.clone(), buckets(r))).collect();
            prop_assert_eq!(rows.len(), got.len(), "one row per vendor");
            prop_assert_eq!(&got, &want);
        }
    }
}

// ---------------------------------------------------------------------------
// Decisions (owner, 2026-10-13): no payment before its document; aging
// as of a date sees only what existed by then.
// ---------------------------------------------------------------------------

/// A payment can't apply to an invoice or bill that didn't exist yet (a
/// real prepayment is a deposit, recorded differently). Refused, books
/// unchanged.
#[test]
fn a_payment_never_predates_its_document() {
    let (mut l, cash) = books();
    let c = l.add_customer("Acme").unwrap();
    let v = l.add_vendor("Vend").unwrap();
    let inv = l
        .create_invoice(c, d(2026, 3, 10), d(2026, 4, 10), vec![line("w", "100.00")])
        .unwrap();
    let bill = l
        .create_bill(v, d(2026, 3, 10), d(2026, 4, 10), vec![line("w", "100.00")])
        .unwrap();
    let before = fingerprint(&l);
    assert_eq!(
        l.receive_payment(inv, d(2026, 3, 9), dec("10.00"), cash),
        Err(LedgerError::PaymentBeforeDocument {
            date: d(2026, 3, 9),
            document_date: d(2026, 3, 10),
        })
    );
    assert!(matches!(
        l.pay_bill(bill, d(2026, 3, 1), dec("10.00"), cash),
        Err(LedgerError::PaymentBeforeDocument { .. })
    ));
    assert_eq!(fingerprint(&l), before);
    // The same day is fine.
    l.receive_payment(inv, d(2026, 3, 10), dec("10.00"), cash)
        .unwrap();
}

/// Aging as of a date shows only documents issued by then, less only the
/// payments made by then.
#[test]
fn aging_as_of_a_date_sees_only_what_existed_then() {
    let (mut l, cash) = books();
    let c = l.add_customer("Acme").unwrap();
    let early = l
        .create_invoice(c, d(2026, 1, 5), d(2026, 2, 4), vec![line("a", "300.00")])
        .unwrap();
    l.create_invoice(c, d(2026, 3, 1), d(2026, 3, 31), vec![line("b", "500.00")])
        .unwrap();
    l.receive_payment(early, d(2026, 2, 20), dec("100.00"), cash)
        .unwrap();
    let total = |as_of| -> Decimal {
        l.ar_aging(as_of)
            .iter()
            .map(|r| buckets(r).iter().copied().sum::<Decimal>())
            .sum()
    };
    // Jan 31: only the first invoice exists, nothing paid yet.
    assert_eq!(total(d(2026, 1, 31)), dec("300.00"));
    // Feb 28: paid 100 of it; the March invoice doesn't exist yet.
    assert_eq!(total(d(2026, 2, 28)), dec("200.00"));
    // Mar 15: both.
    assert_eq!(total(d(2026, 3, 15)), dec("700.00"));
    // Before anything was issued: nothing.
    assert!(l.ar_aging(d(2025, 12, 31)).is_empty());
}
