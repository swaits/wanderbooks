//! The books' own record of how they are kept (journal format 2): settings,
//! conventions, decisions, open items, approvals, through the store.
//!
//! ```text
//! GOAL: what is journaled is what every reader sees, always.
//! SOURCE: plan/served.md I-3, P1 tests (settings_replay_is_pure,
//!         approval_rules_off_by_default).
//! ```

use chrono::NaiveDate;
use ledger_core::conventions::{ConventionKind, NewConvention, NewDecision, NewOpenItem};
use ledger_core::settings::{Change, Source};
use ledger_core::{AccountType, Currency, Money, Side};
use ledger_store::{Snapshot, Store, StoreError};
use proptest::prelude::*;
use rust_decimal::Decimal;
use serde_json::{Value, json};
use std::path::PathBuf;

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
    let dir = std::env::temp_dir().join(format!(
        "wanderbooks-store-settings-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    TestDir(dir)
}

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn src(by: &str) -> Source {
    Source {
        decided_by: by.into(),
        decided_on: d(2026, 10, 17),
        citation: None,
        note: None,
    }
}

fn set(k: &str, v: Value) -> Change {
    Change {
        key: k.into(),
        value: Some(v),
    }
}

/// Books with a small chart.
fn books(name: &str) -> (TestDir, Store) {
    let dir = test_dir(name);
    let mut s = Store::create(&dir, Currency::USD).unwrap();
    for (code, name, t) in [
        ("1000", "Bank", AccountType::Asset),
        ("2100", "Due to Members", AccountType::Liability),
        ("3000", "Capital A", AccountType::Equity),
        ("3001", "Capital B", AccountType::Equity),
        ("6000", "Software", AccountType::Expense),
    ] {
        s.add_account(code, name, t, None, false).unwrap();
    }
    (dir, s)
}

fn template(id: &str) -> NewConvention {
    NewConvention {
        id: id.into(),
        kind: ConventionKind::PostingTemplate,
        title: "Joint contribution".into(),
        body_md: "Cash in from both members, half each.".into(),
        structured: Some(json!({"lines": [
            {"account": "1000", "side": "debit", "share": "1"},
            {"account": "3000", "side": "credit", "share": "0.5"},
            {"account": "3001", "side": "credit", "share": "0.5"}
        ]})),
        source: src("A"),
    }
}

/// Every reader of the journal (a reopened store, the viewer's snapshot)
/// holds what the live store holds, and the store verifies.
fn assert_every_reader_agrees(dir: &std::path::Path, s: &Store) {
    s.verify().unwrap();
    let snap = Snapshot::load(dir).unwrap();
    assert_eq!(snap.ledger().settings(), s.ledger().settings());
    assert_eq!(snap.ledger().conventions(), s.ledger().conventions());
    assert!(snap.ledger().differs_from(s.ledger()).is_empty());
}

/// Settings need journal format 2 (format gate): books this version made
/// declared it as they were created, so the first change journals only
/// itself; a change to what is already set journals nothing. Books an
/// older version wrote in format 1 refuse the change, journaling nothing,
/// until their format upgrade (the owner's go-ahead, D-39); then it is
/// made, after the one declaration.
#[test]
fn a_change_needs_journal_format_2() {
    let (dir, mut s) = books("format");
    assert_eq!(s.journal_format().format, 2);
    let before = s.head().0;
    s.change_settings(&[set("company.name", json!("Acme LLC"))], &src("A"))
        .unwrap();
    assert_eq!(s.head().0, before + 1, "the change alone");
    s.change_settings(
        &[set("company.timezone", json!("America/Denver"))],
        &src("A"),
    )
    .unwrap();
    assert_eq!(s.head().0, before + 2);
    let at = s.head().0;
    let made = s
        .change_settings(&[set("company.name", json!(" Acme LLC "))], &src("A"))
        .unwrap();
    assert!(made.is_empty());
    assert_eq!(s.head().0, at, "nothing to change, nothing journaled");
    let r = s.ledger().settings().get("company.name").unwrap();
    assert_eq!((r.seq, r.source.decided_by.as_str()), (before + 1, "A"));
    // Books in format 1, as an older version wrote them.
    let old = test_dir("format-old");
    let mut o =
        Store::create_in_format(&old, Currency::USD, &ledger_store::JournalFormat::default())
            .unwrap();
    let at = o.head().0;
    for refused in [
        o.change_settings(&[set("company.name", json!("Acme LLC"))], &src("A")),
        o.adopt_settings(&[], &src("A")),
    ] {
        assert!(
            matches!(refused, Err(StoreError::FormatUpgradePending { .. })),
            "{refused:?}"
        );
    }
    assert_eq!(o.head().0, at, "refused: nothing journaled");
    assert!(!o.ledger().settings().is_journaled());
    o.upgrade_format().unwrap().expect("upgraded");
    o.change_settings(&[set("company.name", json!("Acme LLC"))], &src("A"))
        .unwrap();
    assert_eq!(o.head().0, at + 2, "the declaration, then the change");
    assert_every_reader_agrees(&old, &o);
    drop(s);
    let s = Store::open(&dir).unwrap();
    assert_eq!(s.journal_format().format, 2);
    assert_eq!(s.ledger().settings().timezone(), Some("America/Denver"));
    assert_every_reader_agrees(&dir, &s);
}

/// A refused change journals nothing: an unknown key, a bad value, or
/// settings that wouldn't hold together.
#[test]
fn refused_changes_journal_nothing() {
    let (_dir, mut s) = books("refused");
    let at = s.head().0;
    for changes in [
        vec![set("no.such.key", json!("x"))],
        vec![set("policy.parties", json!("always"))],
        vec![
            set("company.name", json!("Fine")),
            set("company.duns", json!("123")),
        ],
        vec![
            set(
                "company.members",
                json!([{"name": "A", "share": "50%", "capital_account": "3000"},
                       {"name": "B", "share": "50%", "capital_account": "3001"}]),
            ),
            set("books.close_into", json!("3000")),
        ],
        vec![set("company.address", json!("SSN 123-45-6789"))],
    ] {
        assert!(
            s.change_settings(&changes, &src("A")).is_err(),
            "{changes:?}"
        );
        assert_eq!(s.head().0, at, "{changes:?}");
        assert!(!s.ledger().settings().is_journaled());
    }
}

/// Approval rules are recorded but block nothing until the books enforce
/// them; then an entry a rule covers posts only once its approver approved
/// it as it stands (crate::conventions INVARIANT 3).
#[test]
fn approval_rules_off_by_default() {
    let (dir, mut s) = books("approvals");
    s.set_convention(&NewConvention {
        id: "over-500".into(),
        kind: ConventionKind::ApprovalRule,
        title: "Big spends".into(),
        body_md: "Anything over 500 needs B's yes.".into(),
        structured: Some(json!({"approver": "B", "amount_over": "500"})),
        source: src("A"),
    })
    .unwrap();
    let draft = |s: &mut Store, amount: i64| {
        let e = s.create_entry(d(2026, 10, 1), "Laptop").unwrap();
        let m = Money::new(Decimal::from(amount), Currency::USD);
        let a = |s: &Store, c: &str| s.ledger().chart().find_by_code(c).unwrap().id;
        let (x, b) = (a(s, "6000"), a(s, "1000"));
        s.add_line(e, x, Side::Debit, m).unwrap();
        s.add_line(e, b, Side::Credit, m).unwrap();
        e
    };
    // Off (the default): nothing is missing, and it posts.
    let e = draft(&mut s, 900);
    assert!(s.ledger().approvals_missing(e).unwrap().is_empty());
    s.post_entry(e).unwrap();
    // Enforced: missing until B approves; small entries aren't covered.
    s.change_settings(&[set("policy.approval_rules", json!("enforce"))], &src("A"))
        .unwrap();
    let e = draft(&mut s, 900);
    assert_eq!(
        s.ledger().approvals_missing(e).unwrap(),
        vec![("over-500".to_string(), "B".to_string())]
    );
    let small = draft(&mut s, 100);
    assert!(s.ledger().approvals_missing(small).unwrap().is_empty());
    // Approved by someone else: still missing. By B (any case): done.
    s.approve_entry(e, "A", None, &src("A")).unwrap();
    assert_eq!(s.ledger().approvals_missing(e).unwrap().len(), 1);
    s.approve_entry(e, "b", Some("fine"), &src("B")).unwrap();
    assert!(s.ledger().approvals_missing(e).unwrap().is_empty());
    // A line added after the approval voids it.
    let bank = s.ledger().chart().find_by_code("1000").unwrap().id;
    let sw = s.ledger().chart().find_by_code("6000").unwrap().id;
    let one = Money::new(Decimal::ONE, Currency::USD);
    s.add_line(e, sw, Side::Debit, one).unwrap();
    s.add_line(e, bank, Side::Credit, one).unwrap();
    assert_eq!(s.ledger().approvals_missing(e).unwrap().len(), 1);
    // A retired rule blocks nothing.
    s.retire_convention("over-500", "No longer needed.", &src("A"))
        .unwrap();
    assert!(s.ledger().approvals_missing(e).unwrap().is_empty());
    // Approvals are for drafts only.
    s.post_entry(e).unwrap();
    assert!(matches!(
        s.approve_entry(e, "B", None, &src("B")),
        Err(StoreError::Ledger(
            ledger_core::LedgerError::EntryAlreadyPosted(_)
        ))
    ));
    assert_every_reader_agrees(&dir, &s);
}

/// A template drafts lines that sum on each side to the amount exactly,
/// odd cents to the larger remainder; a refused draft journals nothing.
#[test]
fn templates_draft_exactly_or_not_at_all() {
    let (dir, mut s) = books("templates");
    s.set_convention(&template("contribution")).unwrap();
    let e = s
        .draft_from_template(
            "contribution",
            d(2026, 10, 2),
            Decimal::new(10001, 2),
            None,
            &Default::default(),
        )
        .unwrap();
    let entry = s.ledger().get_entry(e).unwrap();
    assert_eq!(entry.description(), "Joint contribution");
    let amounts: Vec<String> = entry
        .lines()
        .iter()
        .map(|l| l.amount.amount.to_string())
        .collect();
    assert_eq!(amounts, ["100.01", "50.01", "50.00"]);
    let at = s.head().0;
    for (id, amount) in [
        ("contribution", Decimal::new(1, 2)),
        ("contribution", Decimal::new(1, 3)),
        ("contribution", Decimal::ZERO),
        ("nope", Decimal::ONE),
    ] {
        assert!(
            s.draft_from_template(id, d(2026, 10, 2), amount, None, &Default::default())
                .is_err(),
            "{id} {amount}"
        );
        assert_eq!(s.head().0, at, "{id} {amount}: nothing journaled");
    }
    // A template must name accounts in the chart.
    let mut bad = template("bad");
    bad.structured = Some(json!({"lines": [
        {"account": "9999", "side": "debit", "share": "1"},
        {"account": "1000", "side": "credit", "share": "1"}]}));
    assert!(s.set_convention(&bad).is_err());
    assert_eq!(s.head().0, at);
    assert_every_reader_agrees(&dir, &s);
}

/// Review of P1 (2026-10-17): approvals cover what posts at once, an
/// approval is of the draft as it stands (its classes too), a rule's
/// account can't be renumbered out from under it, and a placeholder can't
/// be named.
#[test]
fn approvals_cover_every_post() {
    let (dir, mut s) = books("approvals-every");
    s.add_account("6900", "Heading", AccountType::Expense, None, true)
        .unwrap();
    s.change_settings(&[set("policy.approval_rules", json!("enforce"))], &src("A"))
        .unwrap();
    let rule = |accounts: Value| NewConvention {
        id: "big".into(),
        kind: ConventionKind::ApprovalRule,
        title: "Big spends".into(),
        body_md: "Over 500 needs B.".into(),
        structured: Some(json!({"approver": "B", "accounts": accounts, "amount_over": "500"})),
        source: src("A"),
    };
    assert!(
        s.set_convention(&rule(json!(["6900"]))).is_err(),
        "a heading"
    );
    s.set_convention(&rule(json!(["6000"]))).unwrap();
    // Renumbering an account a rule names is refused.
    let sw = s.ledger().chart().find_by_code("6000").unwrap().id;
    assert!(s.renumber_account(sw, "6010").is_err());
    // A rule over any account: a bill over it can't post at once.
    s.set_convention(&NewConvention {
        id: "any-big".into(),
        ..rule(json!([]))
    })
    .unwrap();
    let v = s.add_vendor("Acme").unwrap();
    let at = s.head().0;
    let line = |n: i64| ledger_core::DocumentLine {
        description: "work".into(),
        amount: Decimal::from(n),
    };
    assert!(matches!(
        s.create_bill(v, d(2026, 10, 1), d(2026, 10, 31), vec![line(600)]),
        Err(StoreError::Ledger(
            ledger_core::LedgerError::ApprovalNeedsDraft(_)
        ))
    ));
    assert_eq!(s.head().0, at, "nothing journaled");
    s.create_bill(v, d(2026, 10, 1), d(2026, 10, 31), vec![line(100)])
        .unwrap();
    // A class set after the approval voids it.
    let e = s.create_entry(d(2026, 10, 2), "Laptop").unwrap();
    let m = Money::new(Decimal::from(900), Currency::USD);
    let bank = s.ledger().chart().find_by_code("1000").unwrap().id;
    s.add_line(e, sw, Side::Debit, m).unwrap();
    s.add_line(e, bank, Side::Credit, m).unwrap();
    assert!(matches!(
        s.post_entry(e),
        Err(StoreError::Ledger(
            ledger_core::LedgerError::ApprovalRequired { .. }
        ))
    ));
    s.approve_entry(e, "B", None, &src("B")).unwrap();
    let class = s.add_class("Rondo").unwrap();
    s.set_line_class(e, 0, Some(class)).unwrap();
    assert_eq!(s.ledger().approvals_missing(e).unwrap().len(), 2);
    s.approve_entry(e, "B", None, &src("B")).unwrap();
    assert!(s.ledger().approvals_missing(e).unwrap().is_empty());
    s.post_entry(e).unwrap();
    assert_every_reader_agrees(&dir, &s);
}

/// The fiscal year can't move under a closed period; restating the same
/// year is no change.
#[test]
fn the_fiscal_year_stays_under_closed_periods() {
    let (_dir, mut s) = books("fiscal-year");
    let p = s
        .create_period("2026-01", d(2026, 1, 1), d(2026, 1, 31))
        .unwrap();
    s.close_period(p).unwrap();
    assert!(
        s.change_settings(
            &[set("company.fiscal_year_start", json!("07-01"))],
            &src("A")
        )
        .is_err()
    );
    s.change_settings(
        &[set("company.fiscal_year_start", json!("01-01"))],
        &src("A"),
    )
    .unwrap();
}

/// Books moving a settings file in take the fiscal year they were kept
/// under, closed periods or not (review round 2: a July year couldn't be
/// imported once a month was closed); an ordinary change still can't.
#[test]
fn moving_a_file_in_takes_its_fiscal_year() {
    let (_dir, mut s) = books("fiscal-adopt");
    let p = s
        .create_period("2026-07", d(2026, 7, 1), d(2026, 7, 31))
        .unwrap();
    s.close_period(p).unwrap();
    let july = [set("company.fiscal_year_start", json!("07-01"))];
    assert!(s.change_settings(&july, &src("A")).is_err());
    s.adopt_settings(&july, &src("A")).unwrap();
    assert_eq!(
        s.ledger().settings().company("USD").fiscal_year_start,
        (7, 1)
    );
    // Once the books keep their own settings, adopting is an ordinary change.
    assert!(
        s.adopt_settings(
            &[set("company.fiscal_year_start", json!("01-01"))],
            &src("A")
        )
        .is_err()
    );
}

/// A rule this version can't read is a newer version's on replay;
/// changing the party after an approval voids it.
#[test]
fn rules_fail_closed_and_parties_count() {
    let (dir, s) = books("rule-unreadable");
    drop(s);
    let mut w = ledger_store::JournalWriter::open(&dir.join(ledger_store::JOURNAL_FILE)).unwrap();
    w.append(
        "convention.set",
        json!({"id": "r", "kind": "approval_rule", "title": "T", "body_md": "B",
               "structured": {"approver": "B", "amount_over": "5", "currency": "EUR"},
               "source": {"decided_by": "A", "decided_on": "2026-10-17"}}),
    )
    .unwrap();
    drop(w);
    assert!(matches!(
        Store::open(&dir),
        Err(StoreError::NewerJournal { .. })
    ));
    let (_dir, mut s) = books("party-voids");
    s.change_settings(&[set("policy.approval_rules", json!("enforce"))], &src("A"))
        .unwrap();
    s.set_convention(&NewConvention {
        id: "any".into(),
        kind: ConventionKind::ApprovalRule,
        title: "Any".into(),
        body_md: "Everything over 1 needs B.".into(),
        structured: Some(json!({"approver": "B", "amount_over": "1"})),
        source: src("A"),
    })
    .unwrap();
    let e = s.create_entry(d(2026, 10, 2), "Laptop").unwrap();
    let m = Money::new(Decimal::from(9), Currency::USD);
    let (x, b) = (
        s.ledger().chart().find_by_code("6000").unwrap().id,
        s.ledger().chart().find_by_code("1000").unwrap().id,
    );
    s.add_line(e, x, Side::Debit, m).unwrap();
    s.add_line(e, b, Side::Credit, m).unwrap();
    s.approve_entry(e, "B", None, &src("B")).unwrap();
    assert!(s.ledger().approvals_missing(e).unwrap().is_empty());
    let party = s
        .add_party("Acme", &[ledger_core::Role::Contractor])
        .unwrap();
    s.set_entry_party(e, Some(party)).unwrap();
    assert_eq!(s.ledger().approvals_missing(e).unwrap().len(), 1);
}

/// A template's amount is at most a line's.
#[test]
fn a_template_amount_is_at_most_a_lines() {
    let (_dir, mut s) = books("template-bound");
    s.set_convention(&template("c")).unwrap();
    let over = ledger_core::kernel::MAX_LINE_AMOUNT + Decimal::ONE;
    assert!(
        s.draft_from_template("c", d(2026, 10, 2), over, None, &Default::default())
            .is_err()
    );
}

/// A setting key or convention kind this version doesn't know is a newer
/// version's, never damage (rules.md B-27).
#[test]
fn a_newer_key_or_kind_is_newer() {
    for (event, payload) in [
        (
            "settings.set",
            json!({"changes": [{"key": "policy.capitalization", "value": "x"}],
                   "source": {"decided_by": "A", "decided_on": "2026-10-17"}}),
        ),
        (
            "convention.set",
            json!({"id": "w", "kind": "workpaper", "title": "T", "body_md": "B",
                   "source": {"decided_by": "A", "decided_on": "2026-10-17"}}),
        ),
    ] {
        let (dir, s) = books(&format!("newer-{event}"));
        drop(s);
        let mut w =
            ledger_store::JournalWriter::open(&dir.join(ledger_store::JOURNAL_FILE)).unwrap();
        w.append(event, payload).unwrap();
        drop(w);
        let newer = |e: StoreError| matches!(e, StoreError::NewerJournal { .. });
        assert!(newer(Store::open(&dir).err().expect("refused")), "{event}");
        assert!(
            newer(Snapshot::load(&dir).err().expect("refused")),
            "{event}"
        );
    }
}

/// A template slot takes the account each draft names: every slot named,
/// nothing else, an account in the chart that takes postings.
#[test]
fn template_slots_take_the_account_each_draft_names() {
    let (dir, mut s) = books("slots");
    s.add_account("6900", "Other", AccountType::Expense, None, true)
        .unwrap();
    s.set_convention(&NewConvention {
        id: "member-paid".into(),
        kind: ConventionKind::PostingTemplate,
        title: "Expense a member paid personally".into(),
        body_md: "Dr the expense / Cr Due to Members.".into(),
        structured: Some(json!({"lines": [
            {"account": "{expense}", "side": "debit", "share": "1"},
            {"account": "2100", "side": "credit", "share": "1"}]})),
        source: src("A"),
    })
    .unwrap();
    let slots = |pairs: &[(&str, &str)]| {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect::<std::collections::BTreeMap<_, _>>()
    };
    let at = s.head().0;
    for bad in [
        slots(&[]),
        slots(&[("expense", "9999")]),
        slots(&[("expense", "6900")]),
        slots(&[("expense", "6000"), ("other", "6000")]),
    ] {
        assert!(
            s.draft_from_template(
                "member-paid",
                d(2026, 10, 4),
                Decimal::new(1999, 2),
                None,
                &bad
            )
            .is_err(),
            "{bad:?}"
        );
        assert_eq!(s.head().0, at, "{bad:?}");
    }
    let e = s
        .draft_from_template(
            "member-paid",
            d(2026, 10, 4),
            Decimal::new(1999, 2),
            None,
            &slots(&[("expense", "6000")]),
        )
        .unwrap();
    let codes: Vec<String> = s
        .ledger()
        .get_entry(e)
        .unwrap()
        .lines()
        .iter()
        .map(|l| s.ledger().chart().get(l.account).unwrap().code.clone())
        .collect();
    assert_eq!(codes, ["6000", "2100"]);
    // A malformed slot is refused when the template is set.
    let mut bad = template("bad-slot");
    bad.structured = Some(json!({"lines": [
        {"account": "{Expense}", "side": "debit", "share": "1"},
        {"account": "1000", "side": "credit", "share": "1"}]}));
    assert!(s.set_convention(&bad).is_err());
    assert_every_reader_agrees(&dir, &s);
}

/// Decisions and open items: ids in order, supersession once, evidence a
/// document still in the books.
#[test]
fn decisions_and_open_items() {
    let (dir, mut s) = books("decisions");
    let (doc, _, _) = s
        .file_document("legal", "oa.pdf", "application/pdf", b"%PDF-1.4 oa", None)
        .unwrap();
    let decision = |supersedes: Option<u64>, evidence: Option<u64>| NewDecision {
        subject: "Materiality".into(),
        decision_md: "Expense under 2,500.".into(),
        decided_by: vec!["A".into(), "B".into()],
        decided_on: d(2026, 10, 11),
        evidence_attachment: evidence,
        supersedes,
        citation: Some("Treas. Reg. 1.263(a)-1(f)".into()),
    };
    assert_eq!(s.record_decision(&decision(None, Some(doc))).unwrap(), 1);
    assert_eq!(s.record_decision(&decision(Some(1), None)).unwrap(), 2);
    assert!(s.record_decision(&decision(Some(1), None)).is_err());
    assert!(s.record_decision(&decision(Some(9), None)).is_err());
    s.withdraw_attachment(doc, "filed twice").unwrap();
    assert!(matches!(
        s.record_decision(&decision(None, Some(doc))),
        Err(StoreError::AttachmentWithdrawn(_))
    ));
    let item = NewOpenItem {
        title: "Mail the IRS letter".into(),
        detail_md: None,
        due: Some(d(2026, 11, 1)),
        source: src("A"),
    };
    assert_eq!(s.raise_open_item(&item).unwrap(), 1);
    s.resolve_open_item(1, "Mailed.", &src("A")).unwrap();
    assert!(s.resolve_open_item(1, "Again.", &src("A")).is_err());
    assert!(s.resolve_open_item(7, "No such.", &src("A")).is_err());
    assert_every_reader_agrees(&dir, &s);
}

/// One command of the books' record, valid or not.
#[derive(Clone, Debug)]
enum Op {
    Set(&'static str, Value),
    Clear(&'static str),
    Convention(u8, bool),
    Retire(u8),
    Decide(Option<u64>),
    Raise,
    Resolve(u64),
    Template(i64),
    Post(i64),
}

fn op() -> impl Strategy<Value = Op> {
    let values = prop_oneof![
        Just(("company.name", json!("Acme LLC"))),
        Just(("company.name", json!(""))),
        Just(("company.timezone", json!("America/Denver"))),
        Just(("company.timezone", json!("Mars/Base"))),
        Just(("policy.evidence.threshold", json!("0.00"))),
        Just(("policy.evidence.threshold", json!("-1"))),
        Just(("books.cash_accounts", json!(["1000"]))),
        Just(("books.close_into", json!("3000"))),
        Just((
            "company.members",
            json!([{"name": "A", "share": "50%", "capital_account": "3000"},
                   {"name": "B", "share": "50%", "capital_account": "3001"}])
        )),
        Just(("policy.approval_rules", json!("enforce"))),
        Just(("company.ein", json!("12-3456789"))),
    ];
    prop_oneof![
        values.prop_map(|(k, v)| Op::Set(k, v)),
        prop_oneof![
            Just("company.name"),
            Just("books.close_into"),
            Just("company.members")
        ]
        .prop_map(Op::Clear),
        (0u8..4, any::<bool>()).prop_map(|(i, t)| Op::Convention(i, t)),
        (0u8..4).prop_map(Op::Retire),
        proptest::option::of(0u64..4).prop_map(Op::Decide),
        Just(Op::Raise),
        (0u64..4).prop_map(Op::Resolve),
        (-1i64..300).prop_map(Op::Template),
        (1i64..100_000).prop_map(Op::Post),
    ]
}

/// What a record of how the books are kept must leave alone: every
/// figure (the canonical report snapshot) and every draft.
fn figures(s: &Store) -> (Value, Vec<ledger_core::JournalEntry>) {
    let l = s.ledger();
    (
        ledger_core::reports::snapshot(l).unwrap(),
        (1..l.next_entry_id())
            .filter_map(|id| l.get_entry(id).cloned())
            .collect(),
    )
}

fn run(s: &mut Store, op: &Op) {
    // A record (anything but a post or a template's draft) changes no
    // figure and no entry (rules.md B-29; Lean Step.record_changes_no_figure).
    let records = !matches!(op, Op::Post(_) | Op::Template(_));
    let before = records.then(|| figures(s));
    run_op(s, op);
    if let Some(before) = before {
        assert_eq!(figures(s), before, "{op:?} changed a figure or an entry");
    }
}

fn run_op(s: &mut Store, op: &Op) {
    let _ = match op {
        Op::Post(cents) => {
            let m = Money::new(Decimal::new(*cents, 2), Currency::USD);
            let (x, b) = (
                s.ledger().chart().find_by_code("6000").unwrap().id,
                s.ledger().chart().find_by_code("1000").unwrap().id,
            );
            s.create_entry(d(2026, 10, 9), "Software").and_then(|e| {
                s.add_line(e, x, Side::Debit, m)?;
                s.add_line(e, b, Side::Credit, m)?;
                s.post_entry(e)
            })
        }
        Op::Set(k, v) => s
            .change_settings(&[set(k, v.clone())], &src("A"))
            .map(|_| ()),
        Op::Clear(k) => s
            .change_settings(
                &[Change {
                    key: (*k).into(),
                    value: None,
                }],
                &src("A"),
            )
            .map(|_| ()),
        Op::Convention(i, as_template) => {
            let c = if *as_template {
                template(&format!("c{i}"))
            } else {
                NewConvention {
                    id: format!("c{i}"),
                    kind: ConventionKind::Note,
                    title: format!("Note {i}"),
                    body_md: "Words.".into(),
                    structured: None,
                    source: src("B"),
                }
            };
            s.set_convention(&c).map(|_| ())
        }
        Op::Retire(i) => s.retire_convention(&format!("c{i}"), "Done.", &src("A")),
        Op::Decide(sup) => s
            .record_decision(&NewDecision {
                subject: "S".into(),
                decision_md: "D".into(),
                decided_by: vec!["A".into()],
                decided_on: d(2026, 10, 1),
                evidence_attachment: None,
                supersedes: *sup,
                citation: None,
            })
            .map(|_| ()),
        Op::Raise => s
            .raise_open_item(&NewOpenItem {
                title: "T".into(),
                detail_md: None,
                due: None,
                source: src("A"),
            })
            .map(|_| ()),
        Op::Resolve(i) => s.resolve_open_item(*i, "Done.", &src("A")),
        Op::Template(cents) => {
            let id = (0..4)
                .map(|i| format!("c{i}"))
                .find(|id| {
                    s.ledger()
                        .conventions()
                        .convention(id)
                        .is_some_and(|c| c.template().is_some() && c.retired.is_none())
                })
                .unwrap_or_else(|| "c0".into());
            s.draft_from_template(
                &id,
                d(2026, 10, 3),
                Decimal::new(*cents, 2),
                None,
                &Default::default(),
            )
            .and_then(|e| {
                let approver = if cents % 2 == 0 { "A" } else { "B" };
                s.approve_entry(e, approver, None, &src(approver))
                    .map(|_| ())
            })
        }
    };
}

proptest! {
    // 256 on every check (about 25 s); the plan's 2,000 run by hand
    // (PROPTEST_CASES=2000), passed 2026-10-17.
    #![proptest_config(ProptestConfig::with_cases(
        std::env::var("PROPTEST_CASES").ok().and_then(|n| n.parse().ok()).unwrap_or(256)
    ))]

    /// settings_replay_is_pure (plan/served.md P1): after any sequence of
    /// the books' record commands, valid or refused, the live store, the
    /// store reopened from its journal, and the viewer's snapshot hold the
    /// same settings and conventions, and the store verifies (its database
    /// row for row included).
    #[test]
    fn settings_replay_is_pure(ops in proptest::collection::vec(op(), 1..40)) {
        let name = format!("pure-{}", rand_tag(&ops));
        let (dir, mut s) = books(&name);
        for o in &ops {
            run(&mut s, o);
        }
        assert_every_reader_agrees(&dir, &s);
        let live_settings = s.ledger().settings().clone();
        let live_conventions = s.ledger().conventions().clone();
        drop(s);
        let reopened = Store::open(&dir).unwrap();
        prop_assert_eq!(reopened.ledger().settings(), &live_settings);
        prop_assert_eq!(reopened.ledger().conventions(), &live_conventions);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// A short tag for a case's directory (cases run one after another).
fn rand_tag(ops: &[Op]) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    format!("{ops:?}").hash(&mut h);
    format!("{:x}", h.finish())
}

/// The books' record as a reader holds it, as plain JSON: what the
/// p1_record fixture's record.json says this version replayed.
fn record(l: &ledger_core::Ledger) -> Value {
    let source = |s: &Source| {
        json!({"decided_by": s.decided_by, "decided_on": s.decided_on.to_string(),
               "citation": s.citation, "note": s.note})
    };
    let closing = |c: &Option<ledger_core::conventions::Closing>| {
        c.as_ref()
            .map(|c| json!({"text": c.text, "source": source(&c.source), "seq": c.seq}))
    };
    let k = l.conventions();
    json!({
        "settings": l.settings().iter().map(|(key, r)| json!({
            "key": key.name, "value": r.value, "seq": r.seq, "source": source(&r.source)
        })).collect::<Vec<_>>(),
        "settings_last_seq": l.settings().last_seq(),
        "conventions": k.conventions().map(|c| json!({
            "id": c.id, "kind": c.kind.key(), "title": c.title, "body_md": c.body_md,
            "structured": c.structured, "source": source(&c.source), "seq": c.seq,
            "revision": c.revision, "retired": closing(&c.retired)
        })).collect::<Vec<_>>(),
        "decisions": k.decisions().map(|d| json!({
            "id": d.id, "subject": d.new.subject, "decision_md": d.new.decision_md,
            "decided_by": d.new.decided_by, "decided_on": d.new.decided_on.to_string(),
            "evidence_attachment": d.new.evidence_attachment, "supersedes": d.new.supersedes,
            "citation": d.new.citation, "seq": d.seq, "superseded_by": d.superseded_by
        })).collect::<Vec<_>>(),
        "open_items": k.open_items().map(|i| json!({
            "id": i.id, "title": i.new.title, "detail_md": i.new.detail_md,
            "due": i.new.due.map(|d| d.to_string()), "source": source(&i.new.source),
            "seq": i.seq, "resolved": closing(&i.resolved)
        })).collect::<Vec<_>>(),
        "approvals": (1..l.next_entry_id()).flat_map(|e| k.approvals(e).iter().map(move |a| json!({
            "entry": e, "approver": a.approver, "fingerprint": a.fingerprint, "note": a.note,
            "source": source(&a.source), "seq": a.seq
        }))).collect::<Vec<_>>(),
    })
}

/// What expected.json records of a ledger: the same JSON as
/// tests/store.rs `old_books_figures` (the old-books contract).
fn old_books_figures(l: &ledger_core::Ledger) -> Value {
    let entries: Vec<_> = (1..l.next_entry_id())
        .filter_map(|id| l.get_entry(id))
        .collect();
    let parties: Vec<_> = (1..l.next_entry_id())
        .map(|id| (id, l.entry_parties(id)))
        .filter(|(_, p)| !p.is_empty())
        .collect();
    json!({
        "snapshot": ledger_core::reports::snapshot(l).map_err(|e| e.to_string()),
        "entries": entries,
        "assets": l.registered_assets().collect::<Vec<_>>(),
        "periods": l.list_periods(),
        "schedules": l.schedules().collect::<Vec<_>>(),
        "recurring": l.recurring_entries().collect::<Vec<_>>(),
        "invoices": l.invoices(),
        "bills": l.bills(),
        "payroll_runs": l.list_payroll_runs(),
        "entry_parties": parties,
    })
}

fn fixture() -> PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/old-books/p1_record")
}

/// How the p1_record fixture was written (2026-10-17, P1): through the
/// Store API of the version that introduced journal format 2, every kind
/// of event it writes, refused commands between them. Run once with
/// `--ignored`; the journal it wrote is frozen in the fixture, and later
/// versions must open it to the same record (books_written_under_p1_open_to_their_record).
#[test]
#[ignore]
fn write_p1_fixture() {
    let (dir, mut s) = books("p1-fixture");
    let a = src("Tobin Harlow");
    s.change_settings(
        &[
            set("company.name", json!("Fixture Labs LLC")),
            set("company.tax_classification", json!("partnership")),
            set("company.ein", json!("98-7814321")),
            set("company.timezone", json!("America/Denver")),
            set("policy.evidence.threshold", json!("0.00")),
            set("books.cash_accounts", json!(["1000"])),
            set("books.financing_accounts", json!(["2100"])),
            set("policy.parties", json!("required")),
            set(
                "company.members",
                json!([{"name": "A", "share": "50%", "capital_account": "3000",
                        "files_jointly_with": "B"},
                       {"name": "B", "share": "0.5", "capital_account": "3001",
                        "files_jointly_with": "A"}]),
            ),
        ],
        &Source {
            note: Some("Imported from wanderbooks.toml.".into()),
            ..a.clone()
        },
    )
    .unwrap();
    s.set_convention(&template("joint-contribution")).unwrap();
    s.set_convention(&NewConvention {
        body_md:
            "Cash in from both members, half each; one entry per member when they pay separately."
                .into(),
        ..template("joint-contribution")
    })
    .unwrap();
    s.set_convention(&NewConvention {
        id: "over-500".into(),
        kind: ConventionKind::ApprovalRule,
        title: "Big spends".into(),
        body_md: "Software over 500 needs B.".into(),
        structured: Some(json!({"approver": "B", "accounts": ["6000"], "amount_over": "500"})),
        source: a.clone(),
    })
    .unwrap();
    s.set_convention(&NewConvention {
        id: "close-ask".into(),
        kind: ConventionKind::CloseStep,
        title: "Ask about personal cards".into(),
        body_md: "Ask whether a member paid anything personally.".into(),
        structured: None,
        source: a.clone(),
    })
    .unwrap();
    s.retire_convention("close-ask", "Folded into the checklist.", &a)
        .unwrap();
    let (doc, _, _) = s
        .file_document(
            "legal",
            "oa.pdf",
            "application/pdf",
            b"%PDF-1.4 fixture",
            None,
        )
        .unwrap();
    s.record_decision(&NewDecision {
        subject: "Materiality".into(),
        decision_md: "Expense tangible items up to 2,500.".into(),
        decided_by: vec!["Tobin Harlow".into(), "B".into()],
        decided_on: d(2026, 10, 11),
        evidence_attachment: Some(doc),
        supersedes: None,
        citation: Some("Treas. Reg. 1.263(a)-1(f)".into()),
    })
    .unwrap();
    s.record_decision(&NewDecision {
        subject: "Materiality".into(),
        decision_md: "Corrected: prepaid services follow the 12-month rule.".into(),
        decided_by: vec!["Tobin Harlow".into()],
        decided_on: d(2026, 10, 16),
        evidence_attachment: None,
        supersedes: Some(1),
        citation: None,
    })
    .unwrap();
    s.raise_open_item(&NewOpenItem {
        title: "Mail the letter".into(),
        detail_md: Some("Signed; mailing pending.".into()),
        due: Some(d(2026, 10, 24)),
        source: a.clone(),
    })
    .unwrap();
    s.raise_open_item(&NewOpenItem {
        title: "Ask the CPA".into(),
        detail_md: None,
        due: None,
        source: a.clone(),
    })
    .unwrap();
    s.resolve_open_item(2, "Asked.", &a).unwrap();
    let e = s
        .draft_from_template(
            "joint-contribution",
            d(2026, 10, 5),
            Decimal::new(100001, 2),
            None,
            &Default::default(),
        )
        .unwrap();
    s.approve_entry(e, "B", Some("ok"), &src("B")).unwrap();
    s.change_settings(
        &[
            set("policy.approval_rules", json!("enforce")),
            Change {
                key: "company.timezone".into(),
                value: None,
            },
        ],
        &a,
    )
    .unwrap();
    s.verify().unwrap();
    let out = fixture();
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    std::fs::copy(
        dir.join(ledger_store::JOURNAL_FILE),
        out.join(ledger_store::JOURNAL_FILE),
    )
    .unwrap();
    // The blob store, as written.
    fn copy(from: &std::path::Path, to: &std::path::Path) {
        std::fs::create_dir_all(to).unwrap();
        for e in std::fs::read_dir(from).unwrap() {
            let e = e.unwrap();
            if e.file_type().unwrap().is_dir() {
                copy(&e.path(), &to.join(e.file_name()));
            } else {
                std::fs::copy(e.path(), to.join(e.file_name())).unwrap();
            }
        }
    }
    copy(&dir.join("blobs"), &out.join("blobs"));
    let mut expected = old_books_figures(s.ledger());
    expected["verify"] = json!(Ok::<(), String>(()));
    std::fs::write(
        out.join("expected.json"),
        serde_json::to_string_pretty(&expected).unwrap() + "\n",
    )
    .unwrap();
    std::fs::write(
        out.join("record.json"),
        serde_json::to_string_pretty(&record(s.ledger())).unwrap() + "\n",
    )
    .unwrap();
}

/// Books written under P1's rules (journal format 2) open, in the writer
/// and the viewer, to exactly the record that version replayed: every
/// setting with its source and seq, every convention revision and
/// retirement, decision, open item and approval (plan/served.md 5.1:
/// fixture journals from every phase).
#[test]
fn books_written_under_p1_open_to_their_record() {
    let want: Value =
        serde_json::from_slice(&std::fs::read(fixture().join("record.json")).unwrap()).unwrap();
    let dir = test_dir("p1-open");
    std::fs::copy(
        fixture().join(ledger_store::JOURNAL_FILE),
        dir.join(ledger_store::JOURNAL_FILE),
    )
    .unwrap();
    let s = Store::open(&dir).unwrap();
    s.verify().unwrap();
    assert_eq!(record(s.ledger()), want, "the writer");
    let snap = Snapshot::load(&dir).unwrap();
    assert_eq!(record(snap.ledger()), want, "the viewer");
    assert_eq!(s.journal_format().format, 2);
}

/// Moving a settings file in that changes nothing still marks the books
/// as keeping their own settings (one `settings.set`, no changes); once
/// they do, or for an ordinary change, nothing to change journals
/// nothing.
#[test]
fn an_empty_import_still_moves_the_books_in() {
    let (dir, mut s) = books("empty-adopt");
    let at = s.head().0;
    assert!(s.change_settings(&[], &src("A")).unwrap().is_empty());
    assert_eq!(s.head().0, at, "an ordinary empty change: nothing");
    assert_eq!(s.journal_format().format, 2, "declared at creation");
    assert!(!s.ledger().settings().is_journaled());
    assert!(s.adopt_settings(&[], &src("A")).unwrap().is_empty());
    assert_eq!(s.head().0, at + 1, "the move alone");
    assert!(s.ledger().settings().is_journaled());
    assert_eq!(s.ledger().settings().last_seq(), Some(at + 1));
    for again in [
        s.adopt_settings(&[], &src("A")).unwrap(),
        s.change_settings(&[], &src("A")).unwrap(),
    ] {
        assert!(again.is_empty());
    }
    assert_eq!(s.head().0, at + 1, "moved in once");
    drop(s);
    let s = Store::open(&dir).unwrap();
    assert!(s.ledger().settings().is_journaled());
    assert_every_reader_agrees(&dir, &s);
}

/// A posting template whose structure this version can't read is a newer
/// version's on replay, never skipped (as for an approval rule).
#[test]
fn an_unreadable_template_is_newer() {
    let (dir, s) = books("template-unreadable");
    drop(s);
    let mut w = ledger_store::JournalWriter::open(&dir.join(ledger_store::JOURNAL_FILE)).unwrap();
    w.append(
        "convention.set",
        json!({"id": "t", "kind": "posting_template", "title": "T", "body_md": "B",
               "structured": {"lines": [
                   {"account": "1000", "side": "debit", "share": "1"},
                   {"account": "3000", "side": "credit", "share": "1"}],
                   "currency": "EUR"},
               "source": {"decided_by": "A", "decided_on": "2026-10-17"}}),
    )
    .unwrap();
    drop(w);
    assert!(matches!(
        Store::open(&dir),
        Err(StoreError::NewerJournal { .. })
    ));
}
