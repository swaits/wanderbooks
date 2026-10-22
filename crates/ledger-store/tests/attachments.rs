//! Spec-derived tests for attachment links, statements, and
//! reconciliations in the store.
//!
//! Oracles: the doc comments on `Store::link_attachment`,
//! `unlink_attachment`, `attachment_links`, `attachments_for`,
//! `record_statement`, `statement`, `statements`, `complete_reconciliation`,
//! `void_reconciliation`, `reconciliations`, `withdraw_attachment`; the
//! `Subject`, `Statement`, `Reconciliation` types; the "Attachments" and
//! "Reconciliations" sections of docs/parties-and-attachments.md;
//! `Store::verify` (a fresh replay equals the live state); and the store
//! errors (UnknownSubject, AlreadyLinked, NotLinkedTo, InvalidStatement,
//! NotReconciled{difference}, ReconciliationRefused). Expected values are
//! worked by hand from those texts, not from the method bodies.

use chrono::NaiveDate;
use ledger_core::arap::DocumentLine;
use ledger_core::{AccountId, AccountType, Currency, EntryId, Money, NewAsset, Side};
use ledger_store::journal::read_and_verify_journal;
use ledger_store::store::{Reconciliation, Statement, Subject};
use ledger_store::{Attachment, JOURNAL_FILE, Store, StoreError, db_file};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::str::FromStr;

// ---------------------------------------------------------------- helpers

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
    let dir = std::env::temp_dir().join(format!("wb-attach-{name}-{}", std::process::id()));
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

/// The journal's (type, payload) pairs, verified.
fn journal(dir: &Path) -> Vec<(String, Value)> {
    read_and_verify_journal(&dir.join(JOURNAL_FILE))
        .unwrap()
        .into_iter()
        .map(|l| (l.line_type, l.payload))
        .collect()
}

fn journal_bytes(dir: &Path) -> Vec<u8> {
    std::fs::read(dir.join(JOURNAL_FILE)).unwrap()
}

/// Run `f`; it must journal exactly the lines of types `tys`, in order.
/// Returns the payloads.
fn events<T>(
    s: &mut Store,
    dir: &Path,
    tys: &[&str],
    f: impl FnOnce(&mut Store) -> T,
) -> (T, Vec<Value>) {
    let seq = s.journal_seq();
    let len = journal(dir).len();
    let out = f(s);
    assert_eq!(
        s.journal_seq(),
        seq + tys.len() as u64,
        "{tys:?}: line count"
    );
    let j = journal(dir);
    assert_eq!(j.len(), len + tys.len(), "{tys:?}: lines on disk");
    let new = &j[len..];
    let types: Vec<&str> = new.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(types, tys);
    (out, new.iter().map(|(_, p)| p.clone()).collect())
}

/// Run `f`; it must journal exactly one line, of type `ty`. Returns that
/// line's payload.
fn one_event<T>(
    s: &mut Store,
    dir: &Path,
    ty: &str,
    f: impl FnOnce(&mut Store) -> T,
) -> (T, Value) {
    let (out, mut ps) = events(s, dir, &[ty], f);
    (out, ps.remove(0))
}

/// Run `f`; it must fail and journal nothing. Returns the error.
fn refused_nothing_journaled<T: std::fmt::Debug>(
    s: &mut Store,
    dir: &Path,
    what: &str,
    f: impl FnOnce(&mut Store) -> Result<T, StoreError>,
) -> StoreError {
    let seq = s.journal_seq();
    let bytes = journal_bytes(dir);
    let before = s.ledger().clone();
    let r = f(s);
    assert_eq!(s.journal_seq(), seq, "{what}: journaled");
    assert_eq!(journal_bytes(dir), bytes, "{what}: journal changed");
    assert!(*s.ledger() == before, "{what}: ledger changed");
    match r {
        Ok(v) => panic!("{what}: expected refusal, got {v:?}"),
        Err(e) => e,
    }
}

fn delete_db(dir: &Path) {
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let _ = std::fs::remove_file(dir.join(format!("{}{suffix}", db_file())));
    }
}

fn ids(v: &[Attachment]) -> Vec<u64> {
    let mut v: Vec<u64> = v.iter().map(|a| a.id).collect();
    v.sort();
    v
}

fn sorted<T: Ord + Clone>(v: &[T]) -> Vec<T> {
    let mut v = v.to_vec();
    v.sort();
    v
}

fn doc(s: &mut Store, name: &str) -> u64 {
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

struct Fx {
    cash: AccountId,
    capital: AccountId,
    expense: AccountId,
    card: AccountId,
    revenue: AccountId,
    /// 2026-01-01 Dr cash 500.00 (line 0) / Cr capital 500.00 (line 1).
    e1: EntryId,
    /// 2026-01-15 Dr expense 120.00 (line 0) / Cr cash 120.00 (line 1).
    e2: EntryId,
    /// 2026-02-03 Dr expense 30.00 (line 0) / Cr cash 30.00 (line 1).
    e3: EntryId,
    /// 2026-01-10 Dr expense 75.00 (line 0) / Cr card 75.00 (line 1).
    e4: EntryId,
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

fn fixture(dir: &Path) -> (Store, Fx) {
    let mut s = Store::create(dir, Currency::USD).unwrap();
    let cash = s
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    let card = s
        .add_account("2050", "Card", AccountType::Liability, None, false)
        .unwrap();
    let capital = s
        .add_account("3000", "Owner Capital", AccountType::Equity, None, false)
        .unwrap();
    let revenue = s
        .add_account("4000", "App Revenue", AccountType::Revenue, None, false)
        .unwrap();
    let expense = s
        .add_account("6000", "Software", AccountType::Expense, None, false)
        .unwrap();
    let e1 = post(&mut s, date(2026, 1, 1), "Opening", cash, capital, "500.00");
    let e2 = post(&mut s, date(2026, 1, 15), "Tool", expense, cash, "120.00");
    let e3 = post(&mut s, date(2026, 2, 3), "Tool 2", expense, cash, "30.00");
    let e4 = post(
        &mut s,
        date(2026, 1, 10),
        "Card buy",
        expense,
        card,
        "75.00",
    );
    (
        s,
        Fx {
            cash,
            capital,
            expense,
            card,
            revenue,
            e1,
            e2,
            e3,
            e4,
        },
    )
}

fn stmt(
    attachment_id: u64,
    account_id: AccountId,
    start: NaiveDate,
    end: NaiveDate,
    opening: &str,
    closing: &str,
) -> Statement {
    Statement {
        attachment_id,
        account_id,
        period_start: start,
        period_end: end,
        opening_balance: dec(opening),
        closing_balance: dec(closing),
    }
}

/// January's cash statement: 0.00 -> 380.00 (500.00 in, 120.00 out).
fn jan_cash(s: &mut Store, fx: &Fx, name: &str) -> u64 {
    let a = doc(s, name);
    s.record_statement(stmt(
        a,
        fx.cash,
        date(2026, 1, 1),
        date(2026, 1, 31),
        "0.00",
        "380.00",
    ))
    .unwrap();
    a
}

fn clear_jan(s: &mut Store, fx: &Fx) {
    s.mark_cleared(fx.e1, 0, true).unwrap();
    s.mark_cleared(fx.e2, 1, true).unwrap();
}

fn subject_json(subject: Subject) -> Value {
    match subject {
        Subject::Company => json!({"kind": "company"}),
        s => json!({"kind": s.kind(), "id": s.id()}),
    }
}

/// One of every subject kind, created in the fixture's store.
fn every_subject(s: &mut Store, fx: &Fx) -> Vec<Subject> {
    let cost = s
        .add_account("1500", "Equipment", AccountType::Asset, None, false)
        .unwrap();
    let accum = s
        .add_account("1501", "Accum Depr", AccountType::Asset, None, false)
        .unwrap();
    let depr = s
        .add_account("6400", "Depreciation", AccountType::Expense, None, false)
        .unwrap();
    let prepaid = s
        .add_account("1300", "Prepaid", AccountType::Asset, None, false)
        .unwrap();
    let asset = s
        .register_asset(NewAsset {
            description: "Laptop".into(),
            in_service_date: date(2026, 1, 5),
            cost: usd("3000.00"),
            salvage_value: usd("0"),
            useful_life_months: 36,
            cost_account: cost,
            accum_depr_account: accum,
            expense_account: depr,
        })
        .unwrap();
    let party = s.add_party("Dev Shop", &[]).unwrap();
    let cust = s.add_customer("Acme").unwrap();
    let invoice = s
        .create_invoice(
            cust,
            date(2026, 3, 1),
            date(2026, 3, 31),
            vec![DocumentLine {
                description: "App".into(),
                amount: dec("100.00"),
                class_id: None,
            }],
        )
        .unwrap();
    let vend = s.add_vendor("Dana").unwrap();
    let bill = s
        .create_bill(
            vend,
            date(2026, 3, 1),
            date(2026, 3, 31),
            vec![DocumentLine {
                description: "Design".into(),
                amount: dec("200.00"),
                class_id: None,
            }],
        )
        .unwrap();
    let period = s
        .create_period("2026-01", date(2026, 1, 1), date(2026, 1, 31))
        .unwrap();
    let run = s
        .run_payroll(
            date(2026, 3, 27),
            vec![ledger_core::PayrollLine {
                employee_name: "Pat".into(),
                employee: None,
                gross_pay: dec("1000.00"),
                federal_withholding: dec("100.00"),
                state_withholding: dec("0"),
                ss_employee: dec("62.00"),
                medicare_employee: dec("14.50"),
                ss_employer: dec("62.00"),
                medicare_employer: dec("14.50"),
                other_deductions: dec("0"),
            }],
            fx.cash,
        )
        .unwrap();
    let schedule = s
        .register_schedule(ledger_core::NewSchedule {
            description: "Insurance".into(),
            kind: ledger_core::ScheduleKind::PrepaidExpense,
            total: usd("1200.00"),
            start: date(2026, 1, 1),
            months: 12,
            balance_account: prepaid,
            pl_account: fx.expense,
            convention: ledger_core::ScheduleConvention::MonthlyEven,
        })
        .unwrap();
    clear_jan(s, fx);
    let st = jan_cash(s, fx, "subject-stmt.pdf");
    let rec = s.complete_reconciliation(st).unwrap();
    vec![
        Subject::Entry(fx.e2),
        Subject::Account(fx.revenue),
        Subject::Asset(asset),
        Subject::Party(party),
        Subject::Invoice(invoice),
        Subject::Bill(bill),
        Subject::Period(period),
        Subject::PayrollRun(run),
        Subject::Schedule(schedule),
        Subject::Reconciliation(rec.id),
        Subject::Company,
    ]
}

// ---------------------------------------------------------------- linking

/// Oracle: link_attachment ("another entry, an account, an asset, a party,
/// an invoice, a bill, a period, a payroll run, a schedule, a
/// reconciliation, or the company"; journaled as `attachment.linked
/// {attachment_id, subject}`); Subject's serde (tag "kind", content "id",
/// snake_case); attachment_links; attachments_for.
#[test]
fn links_to_every_subject_kind() {
    let dir = test_dir("every-kind");
    let (mut s, fx) = fixture(&dir);
    let subjects = every_subject(&mut s, &fx);
    let a = doc(&mut s, "contract.pdf");
    for &subject in &subjects {
        let (r, p) = one_event(&mut s, &dir, "attachment.linked", |s| {
            s.link_attachment(a, subject)
        });
        r.unwrap();
        assert_eq!(p["attachment_id"], json!(a), "{subject:?}");
        assert_eq!(p["subject"], subject_json(subject), "{subject:?}");
        assert!(
            ids(&s.attachments_for(subject).unwrap()).contains(&a),
            "attachments_for({subject:?}) lacks the file"
        );
    }
    // Filed by file_document: no filing entry, so just the links.
    assert_eq!(sorted(&s.attachment_links(a).unwrap()), sorted(&subjects));
    s.verify().unwrap();
}

/// Oracle: link_attachment SCENARIO 1 ("the subject doesn't exist: SHOULD
/// refuse"), INVARIANT 3 (a refusal journals nothing); UnknownSubject.
#[test]
fn refuses_unknown_subjects() {
    let dir = test_dir("unknown-subject");
    let (mut s, _) = fixture(&dir);
    let a = doc(&mut s, "x.pdf");
    let missing = 9_999;
    for subject in [
        Subject::Entry(missing),
        Subject::Account(missing),
        Subject::Asset(missing),
        Subject::Party(missing),
        Subject::Invoice(missing),
        Subject::Bill(missing),
        Subject::Period(missing),
        Subject::PayrollRun(missing),
        Subject::Schedule(missing),
        Subject::Reconciliation(missing),
    ] {
        let e = refused_nothing_journaled(&mut s, &dir, &format!("{subject:?}"), |s| {
            s.link_attachment(a, subject)
        });
        assert!(
            matches!(e, StoreError::UnknownSubject(_)),
            "{subject:?}: {e:?}"
        );
    }
    assert!(s.attachment_links(a).unwrap().is_empty());
}

/// Oracle: link_attachment SCENARIO 1 (unknown or withdrawn attachment).
#[test]
fn refuses_unknown_or_withdrawn_attachment() {
    let dir = test_dir("link-bad-attachment");
    let (mut s, _) = fixture(&dir);
    let e = refused_nothing_journaled(&mut s, &dir, "unknown", |s| {
        s.link_attachment(9_999, Subject::Company)
    });
    assert!(matches!(e, StoreError::UnknownAttachment(9_999)), "{e:?}");
    let a = doc(&mut s, "gone.pdf");
    s.withdraw_attachment(a, "wrong file").unwrap();
    let e = refused_nothing_journaled(&mut s, &dir, "withdrawn", |s| {
        s.link_attachment(a, Subject::Company)
    });
    assert!(
        matches!(e, StoreError::AttachmentWithdrawn(x) if x == a),
        "{e:?}"
    );
}

/// Oracle: link_attachment SCENARIO 2 (already linked to that subject, or
/// filed on that entry: refuse); AlreadyLinked(id, subject words);
/// attachment_links ("its filing entry, if any, first").
#[test]
fn refuses_duplicate_links_and_the_filing_entry() {
    let dir = test_dir("dup-link");
    let (mut s, fx) = fixture(&dir);
    let a = doc(&mut s, "agreement.pdf");
    s.link_attachment(a, Subject::Account(fx.cash)).unwrap();
    let e = refused_nothing_journaled(&mut s, &dir, "dup account", |s| {
        s.link_attachment(a, Subject::Account(fx.cash))
    });
    assert!(
        matches!(&e, StoreError::AlreadyLinked(x, w) if *x == a && *w == Subject::Account(fx.cash).describe()),
        "{e:?}"
    );
    s.link_attachment(a, Subject::Company).unwrap();
    let e = refused_nothing_journaled(&mut s, &dir, "dup company", |s| {
        s.link_attachment(a, Subject::Company)
    });
    assert!(matches!(e, StoreError::AlreadyLinked(..)), "{e:?}");

    let (r, _, _) = s
        .attach_receipt(Some(fx.e1), "r.pdf", "application/pdf", b"%PDF r", None)
        .unwrap();
    let e = refused_nothing_journaled(&mut s, &dir, "filing entry", |s| {
        s.link_attachment(r, Subject::Entry(fx.e1))
    });
    assert!(
        matches!(e, StoreError::AlreadyLinked(x, _) if x == r),
        "{e:?}"
    );

    // The same receipt is evidence for a second entry, without refiling.
    s.link_attachment(r, Subject::Entry(fx.e2)).unwrap();
    s.link_attachment(r, Subject::Account(fx.cash)).unwrap();
    let links = s.attachment_links(r).unwrap();
    assert_eq!(links[0], Subject::Entry(fx.e1), "filing entry first");
    assert_eq!(
        sorted(&links[1..]),
        sorted(&[Subject::Entry(fx.e2), Subject::Account(fx.cash)])
    );
    assert_eq!(
        ids(&s.attachments_for(Subject::Entry(fx.e1)).unwrap()),
        vec![r]
    );
    assert_eq!(
        ids(&s.attachments_for(Subject::Entry(fx.e2)).unwrap()),
        vec![r]
    );
    s.verify().unwrap();
}

/// Oracle: link_attachment INVARIANT 2 ("an entry's receipts are the files
/// filed on it and the files linked to it"); attachments_for ("for an
/// entry: filed on it or linked to it"); docs "A receipt is an attachment
/// linked to an entry". The store is policy-free, so the receipt rule's
/// count is what attachments_for(Entry) returns.
#[test]
fn a_document_linked_to_an_entry_is_its_receipt() {
    let dir = test_dir("doc-as-receipt");
    let (mut s, fx) = fixture(&dir);
    assert!(s.attachments_for(Subject::Entry(fx.e2)).unwrap().is_empty());
    let a = doc(&mut s, "invoice.pdf");
    s.link_attachment(a, Subject::Entry(fx.e2)).unwrap();
    assert_eq!(
        ids(&s.attachments_for(Subject::Entry(fx.e2)).unwrap()),
        vec![a]
    );
    let (r, _, _) = s
        .attach_receipt(
            Some(fx.e2),
            "paid.pdf",
            "application/pdf",
            b"%PDF paid",
            None,
        )
        .unwrap();
    assert_eq!(
        ids(&s.attachments_for(Subject::Entry(fx.e2)).unwrap()),
        sorted(&[a, r])
    );
    // Other entries are untouched.
    assert!(s.attachments_for(Subject::Entry(fx.e3)).unwrap().is_empty());

    // A draft supported only by a linked document.
    let d = s.create_entry(date(2026, 1, 20), "Draft").unwrap();
    s.add_line(d, fx.expense, Side::Debit, usd("10.00"))
        .unwrap();
    s.add_line(d, fx.cash, Side::Credit, usd("10.00")).unwrap();
    let b = doc(&mut s, "draft-support.pdf");
    s.link_attachment(b, Subject::Entry(d)).unwrap();
    assert_eq!(ids(&s.attachments_for(Subject::Entry(d)).unwrap()), vec![b]);
    s.post_entry(d).unwrap();
    assert_eq!(ids(&s.attachments_for(Subject::Entry(d)).unwrap()), vec![b]);
    s.verify().unwrap();
}

// ---------------------------------------------------------------- unlinking

/// Oracle: unlink_attachment SCENARIOS 1–2 (blank reason or not linked:
/// refuse; otherwise `attachment.unlinked {attachment_id, subject,
/// reason}`); ReasonRequired; NotLinkedTo; "the file stays filed".
#[test]
fn unlink_needs_a_reason_and_a_link() {
    let dir = test_dir("unlink");
    let (mut s, fx) = fixture(&dir);
    let a = doc(&mut s, "misfiled.pdf");
    s.link_attachment(a, Subject::Entry(fx.e2)).unwrap();
    for blank in ["", "   "] {
        let e = refused_nothing_journaled(&mut s, &dir, &format!("blank {blank:?}"), |s| {
            s.unlink_attachment(a, Subject::Entry(fx.e2), blank)
        });
        assert!(matches!(e, StoreError::ReasonRequired), "{e:?}");
    }
    let e = refused_nothing_journaled(&mut s, &dir, "not linked", |s| {
        s.unlink_attachment(a, Subject::Entry(fx.e3), "wrong entry")
    });
    assert!(
        matches!(&e, StoreError::NotLinkedTo(x, w) if *x == a && *w == Subject::Entry(fx.e3).describe()),
        "{e:?}"
    );
    let e = refused_nothing_journaled(&mut s, &dir, "unknown attachment", |s| {
        s.unlink_attachment(9_999, Subject::Entry(fx.e2), "why")
    });
    let _ = e;

    let (r, p) = one_event(&mut s, &dir, "attachment.unlinked", |s| {
        s.unlink_attachment(a, Subject::Entry(fx.e2), "belongs to e3")
    });
    r.unwrap();
    assert_eq!(p["attachment_id"], json!(a));
    assert_eq!(p["subject"], subject_json(Subject::Entry(fx.e2)));
    assert_eq!(p["reason"], json!("belongs to e3"));
    assert!(s.attachment_links(a).unwrap().is_empty());
    assert!(s.attachments_for(Subject::Entry(fx.e2)).unwrap().is_empty());
    // Still filed, and it can go where it belongs.
    assert!(ids(&s.list_attachments(None).unwrap()).contains(&a));
    s.link_attachment(a, Subject::Entry(fx.e3)).unwrap();
    assert_eq!(s.attachment_links(a).unwrap(), vec![Subject::Entry(fx.e3)]);
    // Unlinking twice: no longer linked.
    s.unlink_attachment(a, Subject::Entry(fx.e3), "done")
        .unwrap();
    let e = refused_nothing_journaled(&mut s, &dir, "twice", |s| {
        s.unlink_attachment(a, Subject::Entry(fx.e3), "again")
    });
    assert!(matches!(e, StoreError::NotLinkedTo(..)), "{e:?}");
    s.verify().unwrap();
}

/// Oracle: unlink_attachment ("a receipt filed on an entry comes off with
/// `unlink_receipt` instead"; "isn't linked to the subject (through this
/// table): SHOULD refuse").
#[test]
fn unlink_attachment_does_not_take_a_filed_receipt_off() {
    let dir = test_dir("unlink-filed");
    let (mut s, fx) = fixture(&dir);
    let (r, _, _) = s
        .attach_receipt(Some(fx.e1), "r.pdf", "application/pdf", b"%PDF r1", None)
        .unwrap();
    refused_nothing_journaled(&mut s, &dir, "filed receipt", |s| {
        s.unlink_attachment(r, Subject::Entry(fx.e1), "wrong")
    });
    assert_eq!(
        ids(&s.attachments_for(Subject::Entry(fx.e1)).unwrap()),
        vec![r]
    );
}

// ---------------------------------------------------------------- withdrawal

/// Oracle: withdraw_attachment SCENARIO 2 ("IF it is linked to any subject
/// (link_attachment): SHOULD refuse (AlreadyLinked, naming the first
/// subject)"). Linked through link_attachment rather than filed on the entry.
#[test]
fn withdraw_while_linked_to_an_entry_is_already_linked() {
    let dir = test_dir("withdraw-variant");
    let (mut s, fx) = fixture(&dir);
    let a = doc(&mut s, "dup.pdf");
    s.link_attachment(a, Subject::Entry(fx.e2)).unwrap();
    let e = refused_nothing_journaled(&mut s, &dir, "linked", |s| {
        s.withdraw_attachment(a, "duplicate")
    });
    assert!(
        matches!(&e, StoreError::AlreadyLinked(x, sub) if *x == a && *sub == Subject::Entry(fx.e2).describe()),
        "{e:?}"
    );
}

/// Oracle: withdraw_attachment SCENARIO 2 ("linked to any subject: SHOULD
/// refuse"), SCENARIO 3 (blank reason), SCENARIO 4, and
/// "can never be linked again"; attachments_for ("not withdrawn").
#[test]
fn withdraw_refuses_while_linked_to_an_entry_and_works_after_unlink() {
    let dir = test_dir("withdraw");
    let (mut s, fx) = fixture(&dir);
    let a = doc(&mut s, "dup.pdf");
    s.link_attachment(a, Subject::Entry(fx.e2)).unwrap();
    refused_nothing_journaled(&mut s, &dir, "linked", |s| {
        s.withdraw_attachment(a, "duplicate")
    });
    s.unlink_attachment(a, Subject::Entry(fx.e2), "duplicate")
        .unwrap();
    refused_nothing_journaled(&mut s, &dir, "blank reason", |s| {
        s.withdraw_attachment(a, " ")
    });
    let (r, p) = one_event(&mut s, &dir, "attachment.withdrawn", |s| {
        s.withdraw_attachment(a, "duplicate")
    });
    r.unwrap();
    assert_eq!(p["attachment_id"], json!(a));
    let listed = s
        .list_attachments(None)
        .unwrap()
        .into_iter()
        .find(|x| x.id == a)
        .unwrap();
    assert_eq!(listed.withdrawn_reason.as_deref(), Some("duplicate"));
    assert!(s.attachments_for(Subject::Entry(fx.e2)).unwrap().is_empty());
    let e = refused_nothing_journaled(&mut s, &dir, "relink", |s| {
        s.link_attachment(a, Subject::Entry(fx.e2))
    });
    assert!(
        matches!(e, StoreError::AttachmentWithdrawn(x) if x == a),
        "{e:?}"
    );
    refused_nothing_journaled(&mut s, &dir, "withdraw twice", |s| {
        s.withdraw_attachment(a, "again")
    });
    s.verify().unwrap();
}

// ---------------------------------------------------------------- statements

/// Oracle: record_statement INVARIANT 1 (`statement.recorded
/// {attachment_id, account_id, period_start, period_end, opening_balance,
/// closing_balance}` then `attachment.linked` to the account); statement;
/// statements ("optionally one account's, by period end").
#[test]
fn record_statement_journals_and_links_to_the_account() {
    let dir = test_dir("statement");
    let (mut s, fx) = fixture(&dir);
    let a = doc(&mut s, "jan.pdf");
    let st = stmt(
        a,
        fx.cash,
        date(2026, 1, 1),
        date(2026, 1, 31),
        "0.00",
        "380.00",
    );
    let (r, ps) = events(
        &mut s,
        &dir,
        &["statement.recorded", "attachment.linked"],
        |s| s.record_statement(st.clone()),
    );
    r.unwrap();
    let p = &ps[0];
    assert_eq!(p["attachment_id"], json!(a));
    assert_eq!(p["account_id"], json!(fx.cash));
    assert_eq!(p["period_start"], json!("2026-01-01"));
    assert_eq!(p["period_end"], json!("2026-01-31"));
    let money = |v: &Value| dec(v.as_str().unwrap_or(&v.to_string()));
    assert_eq!(money(&p["opening_balance"]), dec("0"));
    assert_eq!(money(&p["closing_balance"]), dec("380.00"));
    assert_eq!(ps[1]["attachment_id"], json!(a));
    assert_eq!(ps[1]["subject"], subject_json(Subject::Account(fx.cash)));

    assert_eq!(s.statement(a).unwrap(), Some(st.clone()));
    assert_eq!(
        s.attachment_links(a).unwrap(),
        vec![Subject::Account(fx.cash)]
    );
    assert!(ids(&s.attachments_for(Subject::Account(fx.cash)).unwrap()).contains(&a));

    // A plain document records no statement.
    let plain = doc(&mut s, "plain.pdf");
    assert_eq!(s.statement(plain).unwrap(), None);

    // A card (liability) statement, and February's cash one recorded
    // before January's in time order of calls but sorted by period end.
    let feb = doc(&mut s, "feb.pdf");
    let st_feb = stmt(
        feb,
        fx.cash,
        date(2026, 2, 1),
        date(2026, 2, 28),
        "380.00",
        "350.00",
    );
    let c = doc(&mut s, "card.pdf");
    let st_card = stmt(
        c,
        fx.card,
        date(2026, 1, 1),
        date(2026, 1, 31),
        "0.00",
        "75.00",
    );
    s.record_statement(st_card.clone()).unwrap();
    s.record_statement(st_feb.clone()).unwrap();
    assert_eq!(
        s.statements(Some(fx.cash)).unwrap(),
        vec![st.clone(), st_feb.clone()]
    );
    assert_eq!(s.statements(Some(fx.card)).unwrap(), vec![st_card.clone()]);
    assert!(s.statements(Some(fx.capital)).unwrap().is_empty());
    let all = s.statements(None).unwrap();
    assert_eq!(all.len(), 3);
    assert_eq!(all.last().unwrap(), &st_feb, "by period end");
    s.verify().unwrap();
}

/// Oracle: record_statement SCENARIO 1 (unknown or withdrawn attachment,
/// already records a statement, unknown account or not an asset or
/// liability, period backwards, a balance with more than two decimal
/// places: refuse, journaling nothing); InvalidStatement.
#[test]
fn record_statement_refusals_journal_nothing() {
    let dir = test_dir("statement-refusals");
    let (mut s, fx) = fixture(&dir);
    let a = doc(&mut s, "stmt.pdf");
    let jan = |acct| {
        stmt(
            a,
            acct,
            date(2026, 1, 1),
            date(2026, 1, 31),
            "0.00",
            "380.00",
        )
    };

    for acct in [fx.capital, fx.revenue, fx.expense] {
        let e = refused_nothing_journaled(&mut s, &dir, &format!("account {acct}"), |s| {
            s.record_statement(jan(acct))
        });
        assert!(
            matches!(e, StoreError::InvalidStatement(_)),
            "{acct}: {e:?}"
        );
    }
    refused_nothing_journaled(&mut s, &dir, "unknown account", |s| {
        s.record_statement(jan(9_999))
    });
    let e = refused_nothing_journaled(&mut s, &dir, "backwards", |s| {
        s.record_statement(stmt(
            a,
            fx.cash,
            date(2026, 1, 31),
            date(2026, 1, 1),
            "0",
            "380.00",
        ))
    });
    assert!(matches!(e, StoreError::InvalidStatement(_)), "{e:?}");
    for (open, close) in [("0.001", "380.00"), ("0.00", "380.005")] {
        let e = refused_nothing_journaled(&mut s, &dir, &format!("sub-cent {open} {close}"), |s| {
            s.record_statement(stmt(
                a,
                fx.cash,
                date(2026, 1, 1),
                date(2026, 1, 31),
                open,
                close,
            ))
        });
        assert!(matches!(e, StoreError::InvalidStatement(_)), "{e:?}");
    }
    let e = refused_nothing_journaled(&mut s, &dir, "unknown attachment", |s| {
        s.record_statement(stmt(
            9_999,
            fx.cash,
            date(2026, 1, 1),
            date(2026, 1, 31),
            "0",
            "1",
        ))
    });
    assert!(matches!(e, StoreError::UnknownAttachment(9_999)), "{e:?}");
    let w = doc(&mut s, "withdrawn.pdf");
    s.withdraw_attachment(w, "wrong month").unwrap();
    let e = refused_nothing_journaled(&mut s, &dir, "withdrawn", |s| {
        s.record_statement(stmt(
            w,
            fx.cash,
            date(2026, 1, 1),
            date(2026, 1, 31),
            "0",
            "1",
        ))
    });
    assert!(
        matches!(e, StoreError::AttachmentWithdrawn(x) if x == w),
        "{e:?}"
    );

    // Nothing was recorded by the refusals.
    assert_eq!(s.statement(a).unwrap(), None);
    assert!(s.statements(None).unwrap().is_empty());

    // A one-day period does not run backwards; a liability is allowed.
    s.record_statement(stmt(
        a,
        fx.card,
        date(2026, 1, 31),
        date(2026, 1, 31),
        "0",
        "75.00",
    ))
    .unwrap();
    let e = refused_nothing_journaled(&mut s, &dir, "duplicate", |s| {
        s.record_statement(jan(fx.cash))
    });
    let _ = e;
    assert_eq!(s.statements(None).unwrap().len(), 1);
    s.verify().unwrap();
}

// ---------------------------------------------------------------- reconciliations

/// Oracle: complete_reconciliation SCENARIO 2 (NotReconciled naming the
/// difference); NotReconciled's doc ("cleared balance less the statement
/// balance"); Ledger::reconciliation_report (cleared lines dated on or
/// before the statement date).
#[test]
fn refuses_with_the_exact_difference() {
    let dir = test_dir("not-reconciled");
    let (mut s, fx) = fixture(&dir);
    let a = jan_cash(&mut s, &fx, "jan.pdf");
    // Nothing cleared: 0.00 - 380.00.
    let e = refused_nothing_journaled(&mut s, &dir, "none cleared", |s| {
        s.complete_reconciliation(a)
    });
    assert!(
        matches!(e, StoreError::NotReconciled { difference } if difference == dec("-380.00")),
        "{e:?}"
    );
    // Only the deposit: 500.00 - 380.00.
    s.mark_cleared(fx.e1, 0, true).unwrap();
    let e = refused_nothing_journaled(&mut s, &dir, "deposit only", |s| {
        s.complete_reconciliation(a)
    });
    assert!(
        matches!(e, StoreError::NotReconciled { difference } if difference == dec("120.00")),
        "{e:?}"
    );
    // A February line, cleared, is after the statement date and doesn't count.
    s.mark_cleared(fx.e3, 1, true).unwrap();
    let e = refused_nothing_journaled(&mut s, &dir, "feb cleared", |s| {
        s.complete_reconciliation(a)
    });
    assert!(
        matches!(e, StoreError::NotReconciled { difference } if difference == dec("120.00")),
        "{e:?}"
    );
    // Clearing the non-cash side of an entry doesn't move the cash balance.
    s.mark_cleared(fx.e2, 0, true).unwrap();
    let e = refused_nothing_journaled(&mut s, &dir, "expense side cleared", |s| {
        s.complete_reconciliation(a)
    });
    assert!(
        matches!(e, StoreError::NotReconciled { difference } if difference == dec("120.00")),
        "{e:?}"
    );
    assert!(s.reconciliations(None).unwrap().is_empty());
}

/// Oracle: complete_reconciliation SCENARIO 1 (no statement: refuse).
#[test]
fn refuses_without_a_statement() {
    let dir = test_dir("no-statement");
    let (mut s, fx) = fixture(&dir);
    clear_jan(&mut s, &fx);
    let plain = doc(&mut s, "plain.pdf");
    refused_nothing_journaled(&mut s, &dir, "plain doc", |s| {
        s.complete_reconciliation(plain)
    });
    refused_nothing_journaled(&mut s, &dir, "unknown", |s| {
        s.complete_reconciliation(9_999)
    });
}

/// Oracle: complete_reconciliation INVARIANTS 1, 2, 4 (records the cleared
/// lines on the account dated on or before the statement date, links the
/// statement to it; journaled as `reconciliation.completed`); the docs'
/// payload `{id, account_id, statement_date, statement_balance, cleared,
/// attachment_id}`; Reconciliation; reconciliations.
#[test]
fn completes_when_the_cleared_balance_matches() {
    let dir = test_dir("reconciled");
    let (mut s, fx) = fixture(&dir);
    let a = jan_cash(&mut s, &fx, "jan.pdf");
    clear_jan(&mut s, &fx);
    // Cleared but out of scope: another account's line, and a cash line
    // after the statement date.
    s.mark_cleared(fx.e1, 1, true).unwrap();
    s.mark_cleared(fx.e3, 1, true).unwrap();
    // 500.00 - 120.00 = 380.00: reconciled.
    let seq = s.journal_seq();
    let len = journal(&dir).len();
    let rec = s.complete_reconciliation(a).unwrap();
    let new: Vec<(String, Value)> = journal(&dir)[len..].to_vec();
    assert_eq!(s.journal_seq(), seq + new.len() as u64);
    let completed: Vec<&Value> = new
        .iter()
        .filter(|(t, _)| t == "reconciliation.completed")
        .map(|(_, p)| p)
        .collect();
    assert_eq!(completed.len(), 1, "one reconciliation.completed: {new:?}");
    for (t, _) in &new {
        assert!(
            t == "reconciliation.completed" || t == "attachment.linked",
            "unexpected line {t}"
        );
    }

    assert_eq!(rec.account_id, fx.cash);
    assert_eq!(rec.statement_date, date(2026, 1, 31));
    assert_eq!(rec.statement_balance, dec("380.00"));
    assert_eq!(rec.attachment_id, a);
    assert_eq!(rec.voided, None);
    assert_eq!(sorted(&rec.cleared), sorted(&[(fx.e1, 0), (fx.e2, 1)]));

    let p = completed[0];
    assert_eq!(p["id"], json!(rec.id));
    assert_eq!(p["account_id"], json!(fx.cash));
    assert_eq!(p["statement_date"], json!("2026-01-31"));
    assert_eq!(p["attachment_id"], json!(a));
    let mut cleared: Vec<(u64, u64)> = p["cleared"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| (x[0].as_u64().unwrap(), x[1].as_u64().unwrap()))
        .collect();
    cleared.sort();
    assert_eq!(cleared, vec![(fx.e1, 0), (fx.e2, 1)]);

    let links = s.attachment_links(a).unwrap();
    assert!(links.contains(&Subject::Account(fx.cash)), "{links:?}");
    assert!(
        links.contains(&Subject::Reconciliation(rec.id)),
        "{links:?}"
    );
    assert_eq!(
        ids(&s.attachments_for(Subject::Reconciliation(rec.id)).unwrap()),
        vec![a]
    );
    assert_eq!(s.reconciliations(Some(fx.cash)).unwrap(), vec![rec.clone()]);
    assert_eq!(s.reconciliations(None).unwrap(), vec![rec.clone()]);
    assert!(s.reconciliations(Some(fx.card)).unwrap().is_empty());
    s.verify().unwrap();
}

/// Oracle: Statement ("closing balance, in the account's normal
/// direction"); complete_reconciliation INVARIANT 1. A card (liability)
/// owing 75.00 states 75.00.
#[test]
fn reconciles_a_liability_in_its_normal_direction() {
    let dir = test_dir("liability");
    let (mut s, fx) = fixture(&dir);
    let c = doc(&mut s, "card.pdf");
    s.record_statement(stmt(
        c,
        fx.card,
        date(2026, 1, 1),
        date(2026, 1, 31),
        "0.00",
        "75.00",
    ))
    .unwrap();
    s.mark_cleared(fx.e4, 1, true).unwrap();
    let rec = s.complete_reconciliation(c).unwrap();
    assert_eq!(rec.account_id, fx.card);
    assert_eq!(rec.cleared, vec![(fx.e4, 1)]);
    assert_eq!(rec.statement_balance, dec("75.00"));
}

/// Oracle: complete_reconciliation INVARIANT 3 / SCENARIO 3 ("a
/// reconciliation not voided is dated on or after it: SHOULD refuse");
/// ReconciliationRefused; reconciliations ("by date").
#[test]
fn reconciliations_go_forward() {
    let dir = test_dir("forward");
    let (mut s, fx) = fixture(&dir);
    clear_jan(&mut s, &fx);
    let jan = jan_cash(&mut s, &fx, "jan.pdf");
    let r1 = s.complete_reconciliation(jan).unwrap();

    // Same date, another copy of the statement.
    let jan2 = jan_cash(&mut s, &fx, "jan-again.pdf");
    let e = refused_nothing_journaled(&mut s, &dir, "same date", |s| {
        s.complete_reconciliation(jan2)
    });
    assert!(matches!(e, StoreError::ReconciliationRefused(_)), "{e:?}");
    // Earlier date: cleared through 01-20 is also 380.00, but it's backwards.
    let mid = doc(&mut s, "mid.pdf");
    s.record_statement(stmt(
        mid,
        fx.cash,
        date(2026, 1, 1),
        date(2026, 1, 20),
        "0",
        "380.00",
    ))
    .unwrap();
    let e = refused_nothing_journaled(&mut s, &dir, "earlier", |s| s.complete_reconciliation(mid));
    assert!(matches!(e, StoreError::ReconciliationRefused(_)), "{e:?}");
    // The same January statement, again.
    let e = refused_nothing_journaled(&mut s, &dir, "jan again", |s| {
        s.complete_reconciliation(jan)
    });
    assert!(matches!(e, StoreError::ReconciliationRefused(_)), "{e:?}");

    // Another account's sequence is its own.
    let c = doc(&mut s, "card.pdf");
    s.record_statement(stmt(
        c,
        fx.card,
        date(2026, 1, 1),
        date(2026, 1, 15),
        "0",
        "75.00",
    ))
    .unwrap();
    s.mark_cleared(fx.e4, 1, true).unwrap();
    s.complete_reconciliation(c).unwrap();

    // February goes forward: 380.00 - 30.00 = 350.00.
    s.mark_cleared(fx.e3, 1, true).unwrap();
    let feb = doc(&mut s, "feb.pdf");
    s.record_statement(stmt(
        feb,
        fx.cash,
        date(2026, 2, 1),
        date(2026, 2, 28),
        "380.00",
        "350.00",
    ))
    .unwrap();
    let r2 = s.complete_reconciliation(feb).unwrap();
    assert_ne!(r2.id, r1.id);
    assert_eq!(
        sorted(&r2.cleared),
        sorted(&[(fx.e1, 0), (fx.e2, 1), (fx.e3, 1)])
    );
    let cash_recs = s.reconciliations(Some(fx.cash)).unwrap();
    assert_eq!(cash_recs, vec![r1, r2]);
    assert_eq!(s.reconciliations(None).unwrap().len(), 3);
    s.verify().unwrap();
}

/// Oracle: void_reconciliation ("with the reason (to redo it)";
/// `reconciliation.voided {id, reason}`); complete_reconciliation
/// INVARIANT 3 (only reconciliations not voided bound the next);
/// Reconciliation.voided.
#[test]
fn void_and_redo() {
    let dir = test_dir("void");
    let (mut s, fx) = fixture(&dir);
    clear_jan(&mut s, &fx);
    let jan = jan_cash(&mut s, &fx, "jan.pdf");
    let r1 = s.complete_reconciliation(jan).unwrap();

    for blank in ["", "  "] {
        refused_nothing_journaled(&mut s, &dir, &format!("blank {blank:?}"), |s| {
            s.void_reconciliation(r1.id, blank)
        });
    }
    refused_nothing_journaled(&mut s, &dir, "unknown", |s| {
        s.void_reconciliation(r1.id + 1_000, "why")
    });
    let (r, p) = one_event(&mut s, &dir, "reconciliation.voided", |s| {
        s.void_reconciliation(r1.id, "cleared the wrong line")
    });
    r.unwrap();
    assert_eq!(p["id"], json!(r1.id));
    assert_eq!(p["reason"], json!("cleared the wrong line"));
    refused_nothing_journaled(&mut s, &dir, "double void", |s| {
        s.void_reconciliation(r1.id, "again")
    });
    let listed = s.reconciliations(Some(fx.cash)).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].voided.as_deref(), Some("cleared the wrong line"));
    assert_eq!(
        Reconciliation {
            voided: None,
            ..listed[0].clone()
        },
        r1,
        "voiding changes nothing else"
    );

    // Redo the same statement.
    let r2 = s.complete_reconciliation(jan).unwrap();
    assert_ne!(r2.id, r1.id);
    assert_eq!(r2.voided, None);
    assert_eq!(r2.statement_date, date(2026, 1, 31));
    assert_eq!(sorted(&r2.cleared), sorted(&[(fx.e1, 0), (fx.e2, 1)]));
    let mut got: Vec<u64> = s
        .reconciliations(Some(fx.cash))
        .unwrap()
        .iter()
        .map(|r| r.id)
        .collect();
    got.sort();
    assert_eq!(got, sorted(&[r1.id, r2.id]));
    assert!(
        s.attachment_links(jan)
            .unwrap()
            .contains(&Subject::Reconciliation(r2.id))
    );
    // And the redo bounds the next one again.
    let e = refused_nothing_journaled(&mut s, &dir, "after redo", |s| {
        s.complete_reconciliation(jan)
    });
    assert!(matches!(e, StoreError::ReconciliationRefused(_)), "{e:?}");
    s.verify().unwrap();
}

// ---------------------------------------------------------------- replay

#[derive(Debug, PartialEq)]
struct View {
    /// Attachment has no PartialEq: compared by its Debug form.
    attachments: Vec<String>,
    links: Vec<(u64, Vec<Subject>)>,
    for_subjects: Vec<(Subject, Vec<u64>)>,
    statements: Vec<Statement>,
    statement_of: Vec<(u64, Option<Statement>)>,
    reconciliations: Vec<Reconciliation>,
}

fn view(s: &Store, subjects: &[Subject]) -> View {
    let attachments = s.list_attachments(None).unwrap();
    View {
        links: attachments
            .iter()
            .map(|a| (a.id, s.attachment_links(a.id).unwrap()))
            .collect(),
        for_subjects: subjects
            .iter()
            .map(|&x| (x, ids(&s.attachments_for(x).unwrap())))
            .collect(),
        statements: s.statements(None).unwrap(),
        statement_of: attachments
            .iter()
            .map(|a| (a.id, s.statement(a.id).unwrap()))
            .collect(),
        reconciliations: s.reconciliations(None).unwrap(),
        attachments: attachments.iter().map(|a| format!("{a:?}")).collect(),
    }
}

/// Oracle: Store::open (the ledger is the replay of every journal line;
/// a missing database is rebuilt from the journal); Store::verify.
#[test]
fn everything_survives_replay_and_verifies() {
    let dir = test_dir("replay");
    let (mut s, fx) = fixture(&dir);
    let mut subjects = every_subject(&mut s, &fx);
    let a = doc(&mut s, "multi.pdf");
    for &x in &subjects {
        s.link_attachment(a, x).unwrap();
    }
    s.unlink_attachment(a, Subject::Company, "not formation")
        .unwrap();
    let (r, _, _) = s
        .attach_receipt(Some(fx.e3), "r.pdf", "application/pdf", b"%PDF r3", None)
        .unwrap();
    s.link_attachment(r, Subject::Entry(fx.e2)).unwrap();
    let w = doc(&mut s, "gone.pdf");
    s.link_attachment(w, Subject::Entry(fx.e1)).unwrap();
    s.unlink_attachment(w, Subject::Entry(fx.e1), "wrong")
        .unwrap();
    s.withdraw_attachment(w, "wrong").unwrap();
    // every_subject reconciled January; void it, redo, then February.
    let first = s.reconciliations(Some(fx.cash)).unwrap()[0].clone();
    s.void_reconciliation(first.id, "redo").unwrap();
    let redo = s.complete_reconciliation(first.attachment_id).unwrap();
    s.mark_cleared(fx.e3, 1, true).unwrap();
    let feb = doc(&mut s, "feb.pdf");
    s.record_statement(stmt(
        feb,
        fx.cash,
        date(2026, 2, 1),
        date(2026, 2, 28),
        "380.00",
        "350.00",
    ))
    .unwrap();
    let r_feb = s.complete_reconciliation(feb).unwrap();
    let c = doc(&mut s, "card.pdf");
    s.record_statement(stmt(
        c,
        fx.card,
        date(2026, 1, 1),
        date(2026, 1, 31),
        "0",
        "75.00",
    ))
    .unwrap();
    subjects.extend([
        Subject::Entry(fx.e1),
        Subject::Entry(fx.e3),
        Subject::Account(fx.cash),
        Subject::Account(fx.card),
        Subject::Reconciliation(first.id),
        Subject::Reconciliation(redo.id),
        Subject::Reconciliation(r_feb.id),
    ]);
    s.verify().unwrap();
    let before = view(&s, &subjects);
    assert_eq!(before.statements.len(), 3);
    assert_eq!(before.reconciliations.len(), 3);
    drop(s);

    let s = Store::open(&dir).unwrap();
    assert_eq!(view(&s, &subjects), before, "reopen");
    s.verify().unwrap();
    drop(s);

    delete_db(&dir);
    let s = Store::open(&dir).unwrap();
    assert_eq!(view(&s, &subjects), before, "full replay");
    s.verify().unwrap();
}

// ------------------------------------------- withdrawn and voided statements

/// Oracle: complete_reconciliation SCENARIOS 1 and 4 (a withdrawn
/// statement file: AttachmentWithdrawn, before anything is journaled);
/// statements ("a withdrawn file retires its statement"). The statement's
/// account link is taken off and the file withdrawn; confirming then must
/// record no reconciliation, and the statement is no longer listed.
#[test]
fn a_withdrawn_statement_records_no_reconciliation() {
    let dir = test_dir("withdrawn-statement");
    let (mut s, fx) = fixture(&dir);
    let a = jan_cash(&mut s, &fx, "jan.pdf");
    clear_jan(&mut s, &fx);
    s.unlink_attachment(a, Subject::Account(fx.cash), "filed wrong")
        .unwrap();
    s.withdraw_attachment(a, "filed wrong").unwrap();
    let e = refused_nothing_journaled(&mut s, &dir, "withdrawn", |s| s.complete_reconciliation(a));
    assert!(
        matches!(e, StoreError::AttachmentWithdrawn(x) if x == a),
        "{e:?}"
    );
    assert!(s.reconciliations(None).unwrap().is_empty());
    assert_eq!(s.statement(a).unwrap(), None);
    assert!(s.statements(Some(fx.cash)).unwrap().is_empty());
    s.verify().unwrap();
}

/// Oracle: void_statement SCENARIOS 1–4 and INVARIANT 1; record_statement
/// SCENARIO 2 (a voided statement is recorded afresh on the same file);
/// the docs' `statement.voided {attachment_id, reason}`. A January
/// statement mistyped at 830.00 is voided and recorded again at 380.00,
/// then reconciled; voiding it while that reconciliation stands is
/// refused. Reopening and a full replay agree.
#[test]
fn a_mis_recorded_statement_is_voided_and_recorded_again() {
    let dir = test_dir("void-statement");
    let (mut s, fx) = fixture(&dir);
    let a = doc(&mut s, "jan.pdf");
    let typo = stmt(
        a,
        fx.cash,
        date(2026, 1, 1),
        date(2026, 1, 31),
        "0.00",
        "830.00",
    );
    s.record_statement(typo).unwrap();
    for blank in ["", "  "] {
        let e = refused_nothing_journaled(&mut s, &dir, "blank", |s| s.void_statement(a, blank));
        assert!(matches!(e, StoreError::ReasonRequired), "{e:?}");
    }
    let plain = doc(&mut s, "plain.pdf");
    let e = refused_nothing_journaled(&mut s, &dir, "no statement", |s| {
        s.void_statement(plain, "why")
    });
    assert!(matches!(e, StoreError::InvalidStatement(_)), "{e:?}");
    s.void_statement(a, "closing balance mistyped").unwrap();
    let (kind, payload) = journal(&dir).last().cloned().unwrap();
    assert_eq!(kind, "statement.voided");
    assert_eq!(
        payload,
        json!({"attachment_id": a, "reason": "closing balance mistyped"})
    );
    assert_eq!(s.statement(a).unwrap(), None);
    assert!(s.statements(None).unwrap().is_empty());
    let e = refused_nothing_journaled(&mut s, &dir, "voided twice", |s| {
        s.void_statement(a, "again")
    });
    assert!(matches!(e, StoreError::InvalidStatement(_)), "{e:?}");
    let right = stmt(
        a,
        fx.cash,
        date(2026, 1, 1),
        date(2026, 1, 31),
        "0.00",
        "380.00",
    );
    s.record_statement(right.clone()).unwrap();
    assert_eq!(s.statements(None).unwrap(), vec![right.clone()]);
    clear_jan(&mut s, &fx);
    let rec = s.complete_reconciliation(a).unwrap();
    let e = refused_nothing_journaled(&mut s, &dir, "backs a reconciliation", |s| {
        s.void_statement(a, "why")
    });
    assert!(matches!(e, StoreError::ReconciliationRefused(_)), "{e:?}");
    s.verify().unwrap();
    drop(s);
    delete_db(&dir);
    let s = Store::open(&dir).unwrap();
    assert_eq!(s.statements(None).unwrap(), vec![right]);
    assert_eq!(s.reconciliations(None).unwrap(), vec![rec]);
    s.verify().unwrap();
}

/// Oracle: mark_cleared SCENARIOS 1–3. A line January's reconciliation
/// recorded as cleared can't be un-cleared while it stands; once it is
/// voided it can. A line dated in January but cleared later (an
/// outstanding item) may still be cleared.
#[test]
fn a_reconciled_line_stays_cleared_until_the_reconciliation_is_voided() {
    let dir = test_dir("reconciled-line");
    let (mut s, fx) = fixture(&dir);
    let a = jan_cash(&mut s, &fx, "jan.pdf");
    clear_jan(&mut s, &fx);
    let rec = s.complete_reconciliation(a).unwrap();
    let e = refused_nothing_journaled(&mut s, &dir, "un-clear", |s| {
        s.mark_cleared(fx.e2, 1, false)
    });
    assert!(matches!(e, StoreError::ReconciliationRefused(_)), "{e:?}");
    assert!(s.ledger().is_cleared(fx.e2, 1));
    // An outstanding January item cleared afterwards: allowed.
    let late = post(
        &mut s,
        date(2026, 1, 30),
        "Check 101",
        fx.expense,
        fx.cash,
        "10.00",
    );
    s.mark_cleared(late, 1, true).unwrap();
    s.mark_cleared(late, 1, false).unwrap();
    s.void_reconciliation(rec.id, "redo").unwrap();
    s.mark_cleared(fx.e2, 1, false).unwrap();
    assert!(!s.ledger().is_cleared(fx.e2, 1));
    s.verify().unwrap();
}

/// Oracle: link_receipt SCENARIO 5 (already linked to that entry: refuse,
/// AlreadyLinked, journaling nothing), the mirror of link_attachment
/// SCENARIO 2. A file linked to an entry can't then be filed on it too.
#[test]
fn a_file_linked_to_an_entry_cannot_also_be_filed_on_it() {
    let dir = test_dir("linked-then-filed");
    let (mut s, fx) = fixture(&dir);
    let (a, _, _) = s
        .attach_receipt(None, "r.pdf", "application/pdf", b"%PDF r", None)
        .unwrap();
    s.link_attachment(a, Subject::Entry(fx.e2)).unwrap();
    let e = refused_nothing_journaled(&mut s, &dir, "filed on too", |s| s.link_receipt(a, fx.e2));
    assert!(
        matches!(e, StoreError::AlreadyLinked(x, _) if x == a),
        "{e:?}"
    );
    // Another entry is fine: filed on one, linked to another.
    s.link_receipt(a, fx.e3).unwrap();
    s.verify().unwrap();
}
