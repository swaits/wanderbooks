//! An invoice's or bill's lines in the journal (rules.md B-37 item 8,
//! B-27): a line naming its project carries `class_id` (journal feature
//! `document_line_projects`), and the entry's revenue or expense line for
//! that project is tagged by `entry.line_class_set` right after its
//! `entry.line_added`. A document naming no project is journaled exactly
//! as before, and books that haven't declared the feature refuse one that
//! names a project, journaling nothing.
//!
//! ```text
//! GOAL: a document's projects are journaled once, replay exactly, and
//!       never reach books whose owner hasn't consented to the format.
//! SOURCE: docs/spec/rules.md B-37 item 8, B-27, B-28; plan/served.md
//!         D-39 (the owner's upgrade).
//! ```

use chrono::NaiveDate;
use ledger_core::arap::DocumentLine;
use ledger_core::{ClassId, Currency, ProjectFacts};
use ledger_store::format::{DOCUMENT_LINE_PROJECTS, PROJECTS, current};
use ledger_store::{JOURNAL_FILE, JournalWriter, Snapshot, Store, StoreError};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

struct TestDir(PathBuf);

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn test_dir(name: &str) -> TestDir {
    let dir = std::env::temp_dir().join(format!(
        "wanderbooks-document-projects-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    TestDir(dir)
}

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn cents(c: i64) -> Decimal {
    Decimal::new(c, 2)
}

fn line(what: &str, c: i64, class: Option<ClassId>) -> DocumentLine {
    DocumentLine {
        description: what.into(),
        amount: cents(c),
        class_id: class,
        account: None,
        quantity: None,
        unit_price: None,
    }
}

/// Every journal line, as (type, payload).
fn lines(dir: &Path) -> Vec<(String, Value)> {
    std::fs::read_to_string(dir.join(JOURNAL_FILE))
        .unwrap()
        .lines()
        .map(|l| {
            let v: Value = serde_json::from_str(l).unwrap();
            (
                v["type"].as_str().unwrap().to_string(),
                v["payload"].clone(),
            )
        })
        .collect()
}

/// The lines journaled from `from` on, as (type, payload).
fn since(dir: &Path, from: usize) -> Vec<(String, Value)> {
    lines(dir).split_off(from)
}

fn project(s: &mut Store, name: &str) -> ClassId {
    s.add_project(name, &ProjectFacts::default()).unwrap()
}

#[test]
fn a_tagged_invoice_journals_each_projects_line_and_replays() {
    let dir = test_dir("invoice");
    let mut s = Store::create(&dir.0, Currency::USD).unwrap();
    let zeta = project(&mut s, "Zeta");
    let harbor = project(&mut s, "Harbor");
    let c = s.add_customer("Cedar").unwrap();
    // The control accounts first, so what follows is the invoice alone.
    s.create_invoice(c, d(2026, 1, 2), d(2026, 2, 1), vec![line("x", 100, None)])
        .unwrap();
    let from = lines(&dir.0).len();
    let inv = s
        .create_invoice(
            c,
            d(2026, 3, 2),
            d(2026, 4, 1),
            vec![
                line("Design", 300_000, Some(zeta)),
                line("Hosting", 20_000, Some(harbor)),
                line("Fee", 10_000, None),
            ],
        )
        .unwrap();
    let e = s.ledger().get_invoice(inv).unwrap().entry_id;
    let (ar, rev) = (
        s.ledger().chart().find_by_code("1200").unwrap().id,
        s.ledger().chart().find_by_code("4000").unwrap().id,
    );
    let added = |account: u64, side: &str, amount: &str| {
        (
            "entry.line_added".to_string(),
            json!({"id": e, "account": account, "side": side, "amount": amount, "currency": "USD"}),
        )
    };
    let class_set = |index: u64, class: ClassId| {
        (
            "entry.line_class_set".to_string(),
            json!({"entry_id": e, "line_index": index, "class_id": class}),
        )
    };
    let got = since(&dir.0, from);
    assert_eq!(
        got[..7],
        [
            (
                "entry.created".to_string(),
                json!({"id": e, "date": "2026-03-02", "description": format!("Invoice {inv}")})
            ),
            added(ar, "debit", "3300.00"),
            added(rev, "credit", "3000.00"),
            class_set(1, zeta),
            added(rev, "credit", "200.00"),
            class_set(2, harbor),
            added(rev, "credit", "100.00"),
        ]
    );
    assert_eq!(got[7].0, "entry.posted");
    assert_eq!(
        got[7].1["lines"],
        json!([
            {"account": ar, "side": "debit", "amount": "3300.00", "currency": "USD"},
            {"account": rev, "side": "credit", "amount": "3000.00", "currency": "USD", "class_id": zeta},
            {"account": rev, "side": "credit", "amount": "200.00", "currency": "USD", "class_id": harbor},
            {"account": rev, "side": "credit", "amount": "100.00", "currency": "USD"},
        ])
    );
    assert_eq!(got[8].0, "invoice.created");
    assert_eq!(
        got[8].1["lines"],
        json!([
            {"description": "Design", "amount": "3000.00", "class_id": zeta},
            {"description": "Hosting", "amount": "200.00", "class_id": harbor},
            {"description": "Fee", "amount": "100.00"},
        ])
    );
    assert_eq!(got.len(), 9);
    // Live, replayed, projected and the viewer's snapshot all agree.
    s.verify().unwrap();
    let live = s.ledger().clone();
    drop(s);
    let s = Store::open(&dir.0).unwrap();
    assert!(s.ledger().differs_from(&live).is_empty());
    assert_eq!(
        s.ledger().get_invoice(inv).unwrap().lines[1].class_id,
        Some(harbor)
    );
    let snap = Snapshot::load(&dir.0).unwrap();
    assert!(snap.ledger().differs_from(&live).is_empty());
}

#[test]
fn a_tagged_bill_tags_its_expense_lines_never_the_payable() {
    let dir = test_dir("bill");
    let mut s = Store::create(&dir.0, Currency::USD).unwrap();
    let zeta = project(&mut s, "Zeta");
    let v = s.add_vendor("Dana").unwrap();
    let bill = s
        .create_bill(
            v,
            d(2026, 3, 9),
            d(2026, 4, 8),
            vec![
                line("Misc", 2_500, None),
                line("Illustration", 180_000, Some(zeta)),
            ],
        )
        .unwrap();
    let e = s.ledger().get_bill(bill).unwrap().entry_id;
    let tags: Vec<Option<ClassId>> = s
        .ledger()
        .get_entry(e)
        .unwrap()
        .lines()
        .iter()
        .map(|l| l.class_id)
        .collect();
    assert_eq!(tags, vec![None, Some(zeta), None]);
    let set: Vec<Value> = lines(&dir.0)
        .into_iter()
        .filter(|(t, _)| t == "entry.line_class_set")
        .map(|(_, p)| p)
        .collect();
    assert_eq!(
        set,
        vec![json!({"entry_id": e, "line_index": 1, "class_id": zeta})]
    );
    // Its payment is balance-sheet only: no tag journaled.
    let cash = s
        .add_account("1000", "Cash", ledger_core::AccountType::Asset, None, false)
        .unwrap();
    let before = lines(&dir.0).len();
    s.pay_bill(bill, d(2026, 4, 1), cents(182_500), cash)
        .unwrap();
    assert!(since(&dir.0, before).iter().all(|(t, p)| {
        t != "entry.line_class_set"
            && p.get("class_id").is_none()
            && p["lines"]
                .as_array()
                .is_none_or(|ls| ls.iter().all(|l| l.get("class_id").is_none()))
    }));
    s.verify().unwrap();
}

/// A document naming no project is journaled exactly as every version
/// before the feature journaled one: no `class_id` anywhere, no
/// `entry.line_class_set`, one revenue (expense) line.
/// Reversing a tagged invoice's entry journals one `entry.reversed`, whose
/// replay (Ledger::reverse_entry) carries each line's project; live and
/// replayed books agree that its projects net to zero.
#[test]
fn a_reversed_tagged_invoice_replays_with_its_projects_at_zero() {
    let dir = test_dir("reversed");
    let mut s = Store::create(&dir.0, Currency::USD).unwrap();
    let zeta = project(&mut s, "Zeta");
    let harbor = project(&mut s, "Harbor");
    let c = s.add_customer("Cedar").unwrap();
    let inv = s
        .create_invoice(
            c,
            d(2026, 3, 2),
            d(2026, 4, 1),
            vec![
                line("Design", 300_000, Some(zeta)),
                line("Hosting", 20_000, Some(harbor)),
            ],
        )
        .unwrap();
    let e = s.ledger().get_invoice(inv).unwrap().entry_id;
    let from = lines(&dir.0).len();
    let r = s.reverse_entry(e, d(2026, 3, 31), "Reverse").unwrap();
    let types: Vec<String> = since(&dir.0, from).into_iter().map(|(t, _)| t).collect();
    assert_eq!(types, ["entry.reversed"]);
    let tags = |s: &Store| -> Vec<Option<ClassId>> {
        s.ledger()
            .get_entry(r)
            .unwrap()
            .lines()
            .iter()
            .map(|l| l.class_id)
            .collect()
    };
    assert_eq!(tags(&s), [None, Some(zeta), Some(harbor)]);
    let zero = |s: &Store, p: ClassId| {
        let t = ledger_core::project_to_date(s.ledger(), p, d(2026, 12, 31)).unwrap();
        (t.revenue, t.expenses) == (Decimal::ZERO, Decimal::ZERO)
    };
    assert!(zero(&s, zeta) && zero(&s, harbor));
    s.verify().unwrap();
    let live = s.ledger().clone();
    drop(s);
    let s = Store::open(&dir.0).unwrap();
    assert!(s.ledger().differs_from(&live).is_empty());
    assert_eq!(tags(&s), [None, Some(zeta), Some(harbor)]);
    assert!(zero(&s, zeta) && zero(&s, harbor));
}

#[test]
fn an_untagged_document_journals_as_before() {
    let dir = test_dir("untagged");
    let mut s = Store::create(&dir.0, Currency::USD).unwrap();
    project(&mut s, "Zeta");
    let c = s.add_customer("Cedar").unwrap();
    let v = s.add_vendor("Dana").unwrap();
    let from = lines(&dir.0).len();
    s.create_invoice(
        c,
        d(2026, 1, 5),
        d(2026, 2, 4),
        vec![line("A", 10_000, None), line("B", 2_550, None)],
    )
    .unwrap();
    s.create_bill(v, d(2026, 1, 6), d(2026, 2, 5), vec![line("C", 999, None)])
        .unwrap();
    let got = since(&dir.0, from);
    let types: Vec<&str> = got.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(
        types,
        [
            "account.added",
            "account.added",
            "entry.created",
            "entry.line_added",
            "entry.line_added",
            "entry.posted",
            "invoice.created",
            "account.added",
            "account.added",
            "entry.created",
            "entry.line_added",
            "entry.line_added",
            "entry.posted",
            "bill.created",
        ]
    );
    let text = got
        .iter()
        .map(|(_, p)| p.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!text.contains("class_id"), "{text}");
    assert_eq!(
        got[6].1["lines"],
        json!([{"description": "A", "amount": "100.00"}, {"description": "B", "amount": "25.50"}])
    );
}

/// The books the live server keeps until the owner consents again: this
/// version's format without `document_line_projects` (they declared
/// `projects`). A document naming a project is refused, journaling
/// nothing (not even its control accounts); one naming none is taken;
/// after the upgrade both are.
#[test]
fn a_document_naming_a_project_waits_for_the_feature() {
    let dir = test_dir("gate");
    let mut format = current();
    format.features.retain(|f| f != DOCUMENT_LINE_PROJECTS);
    assert!(format.features.iter().any(|f| f == PROJECTS));
    let mut s = Store::create_in_format(&dir.0, Currency::USD, &format).unwrap();
    let zeta = project(&mut s, "Zeta");
    let c = s.add_customer("Cedar").unwrap();
    let v = s.add_vendor("Dana").unwrap();
    let head = s.head();
    let waits = |r: Result<u64, StoreError>| match r {
        Err(StoreError::FormatUpgradePending { need }) => {
            assert_eq!(need, "the journal feature document_line_projects")
        }
        other => panic!("expected FormatUpgradePending, got {other:?}"),
    };
    let tagged = || {
        vec![
            line("Design", 50_000, None),
            line("Build", 1_000, Some(zeta)),
        ]
    };
    waits(s.create_invoice(c, d(2026, 3, 2), d(2026, 4, 1), tagged()));
    waits(s.create_bill(v, d(2026, 3, 2), d(2026, 4, 1), tagged()));
    assert_eq!(s.head(), head, "nothing journaled");
    assert!(s.ledger().chart().find_by_code("1200").is_none());
    // Untagged: as before, accepted.
    s.create_invoice(c, d(2026, 3, 2), d(2026, 4, 1), vec![line("A", 100, None)])
        .unwrap();
    assert!(s.upgrade_format().unwrap().is_some());
    let inv = s
        .create_invoice(c, d(2026, 3, 3), d(2026, 4, 2), tagged())
        .unwrap();
    s.create_bill(v, d(2026, 3, 3), d(2026, 4, 2), tagged())
        .unwrap();
    assert_eq!(
        s.ledger().get_invoice(inv).unwrap().lines[1].class_id,
        Some(zeta)
    );
    s.verify().unwrap();
}

/// Invoices and bills an earlier version journaled replay unchanged: no
/// project on their lines, the entry as it was, and opening the books
/// writes nothing.
#[test]
fn old_invoices_and_bills_replay_unchanged() {
    let dir = test_dir("old");
    let mut w = JournalWriter::open(&dir.0.join(JOURNAL_FILE)).unwrap();
    for (t, p) in [
        ("store.created", json!({"base_currency": "USD"})),
        (
            "account.added",
            json!({"account_type": "Asset", "code": "1200", "currency": "USD", "id": 1, "name": "Accounts Receivable", "parent": null, "placeholder": false}),
        ),
        (
            "account.added",
            json!({"account_type": "Revenue", "code": "4000", "currency": "USD", "id": 2, "name": "Revenue", "parent": null, "placeholder": false}),
        ),
        (
            "account.added",
            json!({"account_type": "Expense", "code": "6999", "currency": "USD", "id": 3, "name": "General Expenses", "parent": null, "placeholder": false}),
        ),
        (
            "account.added",
            json!({"account_type": "Liability", "code": "2000", "currency": "USD", "id": 4, "name": "Accounts Payable", "parent": null, "placeholder": false}),
        ),
        ("class.added", json!({"id": 1, "name": "Zeta"})),
        ("customer.added", json!({"id": 1, "name": "Cedar"})),
        ("vendor.added", json!({"id": 1, "name": "Dana"})),
        (
            "entry.created",
            json!({"date": "2026-03-01", "description": "Invoice 1", "id": 1}),
        ),
        (
            "entry.line_added",
            json!({"account": 1, "amount": "125.50", "currency": "USD", "id": 1, "side": "debit"}),
        ),
        (
            "entry.line_added",
            json!({"account": 2, "amount": "125.50", "currency": "USD", "id": 1, "side": "credit"}),
        ),
        (
            "entry.posted",
            json!({"date": "2026-03-01", "description": "Invoice 1", "id": 1, "reversal_of": null,
            "lines": [{"account": 1, "amount": "125.50", "currency": "USD", "side": "debit"},
                      {"account": 2, "amount": "125.50", "currency": "USD", "side": "credit"}]}),
        ),
        (
            "invoice.created",
            json!({"customer_id": 1, "date": "2026-03-01", "due_date": "2026-03-31", "entry_id": 1, "id": 1,
            "lines": [{"amount": "100.00", "description": "a"}, {"amount": "25.50", "description": "b"}]}),
        ),
        (
            "entry.created",
            json!({"date": "2026-03-02", "description": "Bill 1", "id": 2}),
        ),
        (
            "entry.line_added",
            json!({"account": 3, "amount": "40.00", "currency": "USD", "id": 2, "side": "debit"}),
        ),
        (
            "entry.line_added",
            json!({"account": 4, "amount": "40.00", "currency": "USD", "id": 2, "side": "credit"}),
        ),
        (
            "entry.posted",
            json!({"date": "2026-03-02", "description": "Bill 1", "id": 2, "reversal_of": null,
            "lines": [{"account": 3, "amount": "40.00", "currency": "USD", "side": "debit"},
                      {"account": 4, "amount": "40.00", "currency": "USD", "side": "credit"}]}),
        ),
        (
            "bill.created",
            json!({"vendor_id": 1, "date": "2026-03-02", "due_date": "2026-04-01", "entry_id": 2, "id": 1,
            "lines": [{"amount": "40.00", "description": "c"}]}),
        ),
    ] {
        w.append(t, p).unwrap();
    }
    drop(w);
    let bytes = std::fs::read(dir.0.join(JOURNAL_FILE)).unwrap();
    let s = Store::open(&dir.0).unwrap();
    s.verify().unwrap();
    let inv = s.ledger().get_invoice(1).unwrap();
    assert_eq!(
        inv.lines,
        vec![line("a", 10_000, None), line("b", 2_550, None)]
    );
    assert_eq!(
        s.ledger().get_bill(1).unwrap().lines,
        vec![line("c", 4_000, None)]
    );
    for e in [1, 2] {
        let entry = s.ledger().get_entry(e).unwrap();
        assert_eq!(entry.lines().len(), 2);
        assert!(entry.lines().iter().all(|l| l.class_id.is_none()));
    }
    assert!(ledger_core::project_entries(s.ledger(), 1).is_empty());
    drop(s);
    assert_eq!(std::fs::read(dir.0.join(JOURNAL_FILE)).unwrap(), bytes);
}

/// A document line's `class_id` that isn't a class id is damage, never
/// silently no project; null is none, and an id is taken as journaled
/// (replay checks no line, B-28).
#[test]
fn a_document_lines_class_id_that_isnt_an_id_is_corrupt() {
    let journal = |dir: &Path, class: Value| {
        let mut w = JournalWriter::open(&dir.join(JOURNAL_FILE)).unwrap();
        w.append("store.created", json!({"base_currency": "USD"}))
            .unwrap();
        w.append("customer.added", json!({"id": 1, "name": "Cedar"}))
            .unwrap();
        w.append(
            "invoice.created",
            json!({"customer_id": 1, "date": "2026-03-01", "due_date": "2026-03-31", "entry_id": 1, "id": 1,
                   "lines": [{"amount": "1.00", "description": "a", "class_id": class}]}),
        )
        .unwrap();
    };
    for (fine, want) in [(json!(null), None), (json!(7), Some(7))] {
        let dir = test_dir("fine-class");
        journal(&dir.0, fine);
        let s = Store::open(&dir.0).unwrap();
        assert_eq!(s.ledger().get_invoice(1).unwrap().lines[0].class_id, want);
    }
    for bad in [
        json!("1"),
        json!(-1),
        json!(1.5),
        json!(u64::from(u32::MAX) + 1),
    ] {
        let dir = test_dir("bad-class");
        journal(&dir.0, bad.clone());
        assert!(
            matches!(Store::open(&dir.0), Err(StoreError::JournalCorrupt(_))),
            "{bad}"
        );
    }
}
