//! Conventions, decisions, open items and approvals refuse what their
//! rules refuse, and a refusal changes nothing (plan P14 unit V-D2;
//! coverage found `conventions.rs` at 78% of lines, the gap being its
//! validation refusals).
//!
//! ```text
//! GOAL: a company's way of working is kept in the books checked, sourced
//!       and whole: a bad id, text, template, rule, decision, open item or
//!       approval is refused with a reason, and nothing is journaled.
//! SOURCE: WB-ledger-5 (conventions, decisions, open items and approvals
//!         are kept in the journal); WB-ledger-6 (no full TIN anywhere);
//!         WB-ledger-9 (a template splits an amount exactly);
//!         WB-ledger-10 (approval rules, enforced only on request);
//!         BS-entry-4 (replay takes what was journaled);
//!         conventions.rs INVARIANTS 1 to 6.
//! ```
//!
//! Every refusal here is asked of the public checks the store calls before
//! it journals (`check_*`), and the books are compared to a clone taken
//! before: a check changes nothing, and the `apply_*` commands that refuse
//! on replay change nothing either.

use chrono::NaiveDate;
use ledger_core::conventions::{
    self, Approval, ConventionKind, DecisionSource, NewConvention, NewDecision, NewOpenItem,
};
use ledger_core::settings::{Change, Source};
use ledger_core::{
    AccountType, Currency, EntryId, Ledger, LedgerError, Money, PartyShare, Share, Side,
};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::str::FromStr;

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

fn src() -> Source {
    Source {
        decided_by: "Owner".into(),
        decided_on: d(2026, 11, 9),
        citation: None,
        note: None,
    }
}

/// The message of an InvalidConvention refusal; any other error fails.
fn why<T: std::fmt::Debug>(r: Result<T, LedgerError>) -> String {
    match r {
        Err(LedgerError::InvalidConvention(m)) => m,
        other => panic!("expected an InvalidConvention refusal, got {other:?}"),
    }
}

fn convention(id: &str, kind: ConventionKind, structured: Option<Value>) -> NewConvention {
    NewConvention {
        id: id.into(),
        kind,
        title: "Software subscriptions".into(),
        body_md: "How we book them.".into(),
        structured,
        source: src(),
    }
}

fn tline(account: &str, side: &str, share: &str) -> Value {
    json!({"account": account, "side": side, "share": share})
}

fn two_line_template(id: &str) -> NewConvention {
    convention(
        id,
        ConventionKind::PostingTemplate,
        Some(json!({"lines": [tline("6000", "debit", "1"), tline("1000", "credit", "1")]})),
    )
}

fn rule(id: &str, structured: Value) -> NewConvention {
    convention(id, ConventionKind::ApprovalRule, Some(structured))
}

struct Books {
    l: Ledger,
    expense: u64,
    bank: u64,
}

fn books() -> Books {
    let mut l = Ledger::new(Currency::USD);
    let expense = l
        .add_account("6000", "Software", AccountType::Expense, None, false)
        .unwrap();
    let bank = l
        .add_account("1000", "Bank", AccountType::Asset, None, false)
        .unwrap();
    l.add_account("5000", "Costs (heading)", AccountType::Expense, None, true)
        .unwrap();
    Books { l, expense, bank }
}

fn set_convention(l: &mut Ledger, c: &NewConvention, seq: u64) {
    let c = l.check_convention(c).unwrap();
    l.apply_convention(c, seq).unwrap();
}

fn settle(l: &mut Ledger, key: &str, value: Value) {
    let changes = [Change {
        key: key.into(),
        value: Some(value),
    }];
    let made = l.check_settings(&changes, &src()).unwrap();
    l.apply_settings(&made, &src(), 1).unwrap();
}

// ---------------------------------------------------------------- INVARIANT 1: ids

/// INVARIANT 1 and WB-ledger-6: an id is a slug of 1 to 64 lowercase
/// letters, digits and hyphens, not starting with a hyphen, and never one
/// that reads as a full TIN.
#[test]
fn a_convention_id_is_a_slug_and_never_a_tin() {
    let b = books();
    let before = b.l.clone();
    let id_error = |id: &str| why(b.l.check_convention(&two_line_template(id)));
    for bad in [
        "",
        "Upper",
        "under_score",
        "-leading",
        "has space",
        &"a".repeat(65),
    ] {
        let m = id_error(bad);
        assert!(
            m.contains("1 to 64 lowercase letters, digits and hyphens"),
            "{bad:?}: {m}"
        );
    }
    assert!(id_error("123-45-6789").contains("full TIN"));
    for ok in ["a", "0", "software-2026", &"a".repeat(64)] {
        assert!(b.l.check_convention(&two_line_template(ok)).is_ok(), "{ok}");
    }
    assert_eq!(b.l, before, "a check changes nothing");
}

/// INVARIANT 1: setting a live id revises it (the revision counts up and
/// the source is the latest); its kind can't change; a retired id is never
/// reused; saying exactly what is already said is refused.
#[test]
fn revising_a_convention_is_one_way_and_a_retired_id_is_gone_for_good() {
    let mut b = books();
    set_convention(&mut b.l, &two_line_template("subs"), 1);
    let first = b.l.conventions().convention("subs").unwrap().clone();
    assert_eq!((first.revision, first.seq), (1, 1));

    // The same text again says nothing new.
    let m = why(b.l.check_convention(&two_line_template("subs")));
    assert_eq!(m, "convention subs already says exactly that");

    // A different body is a revision.
    let mut revised = two_line_template("subs");
    revised.body_md = "Booked on the 1st.".into();
    set_convention(&mut b.l, &revised, 2);
    let now = b.l.conventions().convention("subs").unwrap();
    assert_eq!((now.revision, now.seq), (2, 2));
    assert_eq!(now.body_md, "Booked on the 1st.");

    // Its kind can't change.
    let policy = convention("subs", ConventionKind::Policy, None);
    let m = why(b.l.check_convention(&policy));
    assert!(
        m.contains("is a posting_template; its kind can't change"),
        "{m}"
    );
    // The replay path refuses the same two things.
    let mut replayed = b.l.clone();
    let e = replayed.apply_convention(policy.clone(), 3).unwrap_err();
    assert!(matches!(&e, LedgerError::InvalidConvention(m) if m.contains("its kind can't change")));
    assert_eq!(replayed, b.l, "a refused apply changes nothing");

    // Retire it: the id is never reused.
    let reason =
        b.l.check_retire_convention("subs", "superseded", &src())
            .unwrap();
    b.l.apply_retire_convention("subs", reason, src(), 4)
        .unwrap();
    assert!(
        b.l.conventions()
            .convention("subs")
            .unwrap()
            .retired
            .is_some()
    );
    let m = why(b.l.check_convention(&revised));
    assert!(m.contains("is retired; ids are never reused"), "{m}");
    let before = b.l.clone();
    let e = b.l.apply_convention(revised, 5).unwrap_err();
    assert!(matches!(&e, LedgerError::InvalidConvention(m) if m.contains("ids are never reused")));
    assert_eq!(b.l, before);
}

// ---------------------------------------------------------------- INVARIANT 2: templates

/// INVARIANT 2 and WB-ledger-9: a posting template is two to fifty lines,
/// each share a decimal in (0, 1] and each side summing to exactly 1; a
/// slot is a lowercase name in braces; a template needs its structure and
/// a convention that is not a template or a rule takes none.
#[test]
fn a_posting_template_is_refused_unless_each_side_sums_to_one() {
    let b = books();
    let refused = |structured: Option<Value>| {
        why(b.l.check_convention(&convention(
            "t",
            ConventionKind::PostingTemplate,
            structured,
        )))
    };
    let lines = |ls: Vec<Value>| Some(json!({ "lines": ls }));

    assert!(refused(None).contains("a posting template needs structured"));
    assert!(refused(Some(json!({"lines": "x"}))).starts_with("posting template: "));
    assert!(refused(Some(json!({"lines": [], "extra": 1}))).starts_with("posting template: "));
    assert!(refused(lines(vec![tline("6000", "debit", "1")])).contains("2 to 50 lines"));
    let fifty_one: Vec<Value> = (0..51).map(|_| tline("6000", "debit", "0.02")).collect();
    assert!(refused(lines(fifty_one)).contains("2 to 50 lines"));
    // A share outside (0, 1], or not a number.
    for bad in ["0", "-0.5", "1.5", "half", ""] {
        let m = refused(lines(vec![
            tline("6000", "debit", bad),
            tline("1000", "credit", "1"),
        ]));
        assert!(m.contains("template line 1: share"), "{bad:?}: {m}");
    }
    // Shares that don't sum to 1 on a side.
    let m = refused(lines(vec![
        tline("6000", "debit", "0.6"),
        tline("1000", "credit", "1"),
    ]));
    assert!(m.contains("debits 0.6, credits 1"), "{m}");
    let m = refused(lines(vec![
        tline("6000", "debit", "1"),
        tline("1000", "credit", "0.5"),
        tline("1000", "credit", "0.6"),
    ]));
    assert!(m.contains("credits 1.1"), "{m}");
    // A bad slot; a good one.
    let m = refused(lines(vec![
        tline("{Expense}", "debit", "1"),
        tline("1000", "credit", "1"),
    ]));
    assert!(m.contains("template line 1: slot"), "{m}");
    assert!(
        b.l.check_convention(&convention(
            "t",
            ConventionKind::PostingTemplate,
            lines(vec![
                tline("{expense}", "debit", "1"),
                tline("1000", "credit", "1")
            ]),
        ))
        .is_ok()
    );
    // A side that is neither.
    let m = refused(lines(vec![
        tline("6000", "left", "1"),
        tline("1000", "credit", "1"),
    ]));
    assert!(m.contains("debit or credit"), "{m}");
    // A blank description.
    let m = refused(Some(json!({
        "description": "  ",
        "lines": [tline("6000", "debit", "1"), tline("1000", "credit", "1")]
    })));
    assert!(m.contains("template description is blank"), "{m}");
    // A non-template takes no structure; a rule needs one.
    let m = why(b
        .l
        .check_convention(&convention("p", ConventionKind::Policy, Some(json!({})))));
    assert!(
        m.contains("a policy convention takes no structured part"),
        "{m}"
    );
    let m = why(b
        .l
        .check_convention(&convention("r", ConventionKind::ApprovalRule, None)));
    assert!(m.contains("an approval rule needs structured"), "{m}");
}

/// INVARIANT 2: the template's accounts must be in the chart and take
/// postings, a slot excepted (a draft names it).
#[test]
fn a_templates_accounts_are_checked_against_the_chart() {
    let b = books();
    let t = |a: &str| {
        convention(
            "t",
            ConventionKind::PostingTemplate,
            Some(json!({"lines": [tline(a, "debit", "1"), tline("1000", "credit", "1")]})),
        )
    };
    assert_eq!(
        why(b.l.check_convention(&t("7777"))),
        "template account 7777 is not in the chart"
    );
    assert!(why(b.l.check_convention(&t("5000"))).contains("is a placeholder"));
    assert!(b.l.check_convention(&t("{anything}")).is_ok());
    assert!(b.l.check_convention(&t("6000")).is_ok());
}

/// INVARIANT 3 and WB-ledger-10: a rule names an approver and limits
/// itself by accounts that take postings, an amount at least zero, or
/// both; accounts are listed once.
#[test]
fn an_approval_rule_needs_an_approver_and_a_limit() {
    let b = books();
    let refused = |s: Value| why(b.l.check_convention(&rule("r", s)));
    assert!(
        refused(json!({"approver": "Steve"}))
            .contains("covers some accounts, entries over an amount, or both")
    );
    assert!(refused(json!({"approver": "  ", "amount_over": "10"})).contains("approver is blank"));
    for bad in ["-1", "ten", "", "99999999999999999"] {
        let m = refused(json!({"approver": "Steve", "amount_over": bad}));
        assert!(
            m.contains("must be a decimal at least zero"),
            "{bad:?}: {m}"
        );
    }
    assert!(
        refused(json!({"approver": "Steve", "accounts": ["9999"]}))
            .contains("approval rule account 9999 is not in the chart")
    );
    assert!(
        refused(json!({"approver": "Steve", "accounts": ["5000"]}))
            .contains("is a placeholder (a heading)")
    );
    assert!(refused(json!({"approver": "Steve", "bogus": 1})).starts_with("approval rule: "));

    // Duplicates collapse and an amount is kept in its canonical text.
    let ok =
        b.l.check_convention(&rule(
            "r",
            json!({"approver": "Steve", "accounts": ["6000", "6000"], "amount_over": "25.50"}),
        ))
        .unwrap();
    assert_eq!(
        ok.structured.unwrap(),
        json!({"approver": "Steve", "accounts": ["6000"], "amount_over": "25.50"})
    );
}

/// INVARIANT 6 and WB-ledger-6: no convention text holds a full TIN, a
/// control character or more than its length; its source is checked like
/// a setting's.
#[test]
fn convention_text_is_bounded_and_holds_no_tin() {
    let b = books();
    let with = |f: fn(&mut NewConvention)| {
        let mut c = two_line_template("t");
        f(&mut c);
        why(b.l.check_convention(&c))
    };
    assert!(with(|c| c.title = "  ".into()).contains("title is blank"));
    assert!(with(|c| c.title = "x".repeat(201)).contains("title is longer than 200"));
    assert!(with(|c| c.title = "bad\nline".into()).contains("title holds a line break"));
    assert!(with(|c| c.body_md = "EIN 12-3456789 on file".into()).contains("body"));
    assert!(with(|c| c.body_md = "x".repeat(20_001)).contains("body is longer than 20000"));
    assert!(with(|c| c.source.decided_by = " ".into()).contains("decided_by is blank"));
    assert!(
        with(|c| c.source.decided_on = NaiveDate::MAX)
            .contains("outside the calendar the books keep")
    );
    assert!(with(|c| c.source.citation = Some("x".repeat(2_001))).contains("citation"));
    assert!(with(|c| c.source.note = Some("123-45-6789".into())).contains("note"));
}

// ---------------------------------------------------------------- source corrections

/// WB-ledger-5: a convention's source can be corrected alone, with a
/// reason: the text, revision and line stay. It is refused for an unknown
/// or retired convention, a changed text, a changed kind, the same source,
/// a blank reason.
#[test]
fn a_conventions_source_is_corrected_alone_and_with_a_reason() {
    let mut b = books();
    set_convention(&mut b.l, &two_line_template("subs"), 1);
    let mut fixed = two_line_template("subs");
    fixed.source.decided_by = "Steve".into();
    fixed.source.citation = Some("Operating agreement 4.2".into());

    let (source, reason) =
        b.l.check_convention_source(&fixed, "  named the wrong person ")
            .unwrap();
    assert_eq!(source, fixed.source);
    assert_eq!(reason, "named the wrong person");

    let refuse = |l: &Ledger, c: &NewConvention, r: &str| why(l.check_convention_source(c, r));
    assert_eq!(refuse(&b.l, &fixed, " "), "reason is blank");
    assert!(refuse(&b.l, &fixed, "123-45-6789").starts_with("reason"));
    assert!(refuse(&b.l, &two_line_template("subs"), "same").contains("already has that source"));
    let mut other_text = fixed.clone();
    other_text.body_md = "different".into();
    assert!(refuse(&b.l, &other_text, "r").contains("changes more than its source"));
    let mut other_kind = fixed.clone();
    other_kind.kind = ConventionKind::Policy;
    other_kind.structured = None;
    assert!(refuse(&b.l, &other_kind, "r").contains("its kind can't change"));
    let mut unknown = fixed.clone();
    unknown.id = "nope".into();
    assert!(refuse(&b.l, &unknown, "r").contains("unknown convention \"nope\""));

    let before = b.l.clone();
    assert!(matches!(
        b.l.apply_convention_source("nope", fixed.source.clone()),
        Err(LedgerError::InvalidConvention(m)) if m.contains("unknown convention")
    ));
    assert_eq!(b.l, before);

    b.l.apply_convention_source("subs", source).unwrap();
    let c = b.l.conventions().convention("subs").unwrap();
    assert_eq!(c.source.decided_by, "Steve");
    assert_eq!((c.revision, c.body_md.as_str()), (1, "How we book them."));

    // Retired: refused.
    let reason = b.l.check_retire_convention("subs", "gone", &src()).unwrap();
    b.l.apply_retire_convention("subs", reason, src(), 2)
        .unwrap();
    let mut again = fixed.clone();
    again.source.decided_by = "Third".into();
    assert!(refuse(&b.l, &again, "r").contains("is retired"));
}

/// WB-ledger-5: retiring needs a known live convention, a good source and
/// a reason; the replay path refuses an unknown or already retired id.
#[test]
fn a_convention_is_retired_once_with_a_reason() {
    let mut b = books();
    set_convention(&mut b.l, &two_line_template("subs"), 1);
    assert!(why(b.l.check_retire_convention("nope", "r", &src())).contains("unknown convention"));
    assert!(why(b.l.check_retire_convention("subs", " ", &src())).contains("reason is blank"));
    let mut bad = src();
    bad.decided_by = "".into();
    assert!(why(b.l.check_retire_convention("subs", "r", &bad)).contains("decided_by is blank"));

    let reason =
        b.l.check_retire_convention("subs", "no longer used", &src())
            .unwrap();
    assert_eq!(reason, "no longer used");
    b.l.apply_retire_convention("subs", reason.clone(), src(), 2)
        .unwrap();
    assert!(why(b.l.check_retire_convention("subs", "again", &src())).contains("already retired"));
    let before = b.l.clone();
    assert!(
        b.l.apply_retire_convention("subs", reason.clone(), src(), 3)
            .is_err()
    );
    assert!(
        b.l.apply_retire_convention("nope", reason, src(), 3)
            .is_err()
    );
    assert_eq!(b.l, before);
    let closing =
        b.l.conventions()
            .convention("subs")
            .unwrap()
            .retired
            .clone()
            .unwrap();
    assert_eq!((closing.text.as_str(), closing.seq), ("no longer used", 2));
}

// ---------------------------------------------------------------- decisions

fn decision(subject: &str) -> NewDecision {
    NewDecision {
        subject: subject.into(),
        decision_md: "We keep cash-basis books for 2026.".into(),
        decided_by: vec!["Steve".into(), "Tobin".into()],
        decided_on: d(2026, 11, 1),
        evidence_attachment: None,
        supersedes: None,
        citation: Some("Minutes, 2026-11-01".into()),
        projects: vec![],
    }
}

/// INVARIANT 4 and 6: a decision names who decided and when, holds no TIN
/// and no unbounded text; its projects exist and are named once; it
/// supersedes at most one live decision.
#[test]
fn a_decision_is_refused_for_text_projects_and_supersession() {
    let mut b = books();
    let refused = |f: fn(&mut NewDecision)| {
        let mut x = decision("Basis");
        f(&mut x);
        b.l.check_decision(&x).unwrap_err()
    };
    let msg = |e: LedgerError| match e {
        LedgerError::InvalidConvention(m) => m,
        other => panic!("{other:?}"),
    };
    assert!(msg(refused(|x| x.decided_by.clear())).contains("names at least one person"));
    assert!(msg(refused(|x| x.decided_by = vec!["  ".into()])).contains("decided_by is blank"));
    assert!(
        msg(refused(|x| x.decided_on = NaiveDate::MIN))
            .contains("outside the calendar the books keep")
    );
    assert!(msg(refused(|x| x.subject = "\t".into())).contains("subject is blank"));
    assert!(msg(refused(|x| x.decision_md = "SSN 123-45-6789".into())).contains("decision"));
    assert!(msg(refused(|x| x.citation = Some("x".repeat(2_001)))).contains("citation"));
    assert_eq!(
        refused(|x| x.projects = vec![99]),
        LedgerError::UnknownClass(99)
    );
    let p = b.l.add_class("Launch").unwrap();
    let q = b.l.add_class("Maintenance").unwrap();
    let mut twice = decision("Basis");
    twice.projects = vec![p, q, p];
    assert!(
        msg(b.l.check_decision(&twice).unwrap_err())
            .contains(&format!("project {p} is named twice"))
    );
    let mut ok = decision("Basis");
    ok.projects = vec![p, q];
    let (id, kept) = b.l.check_decision(&ok).unwrap();
    assert_eq!(id, 1);
    assert_eq!(kept.projects, vec![p, q]);

    // Supersession.
    let mut sup = decision("Basis, again");
    sup.supersedes = Some(7);
    assert!(msg(b.l.check_decision(&sup).unwrap_err()).contains("decision 7 doesn't exist"));
    b.l.apply_decision(1, ok, 10).unwrap();
    sup.supersedes = Some(1);
    let (id2, kept) = b.l.check_decision(&sup).unwrap();
    assert_eq!(id2, 2);
    b.l.apply_decision(id2, kept, 11).unwrap();
    assert_eq!(
        b.l.conventions().decision(1).unwrap().superseded_by,
        Some(2)
    );
    // A decision already superseded can't be again, by check or by replay.
    let mut third = decision("Basis, thrice");
    third.supersedes = Some(1);
    assert!(msg(b.l.check_decision(&third).unwrap_err()).contains("already superseded by 2"));
    let before = b.l.clone();
    assert!(b.l.apply_decision(3, third.clone(), 12).is_err());
    third.supersedes = Some(42);
    assert!(b.l.apply_decision(3, third, 12).is_err());
    // An id that isn't the next one is refused (ids are allocated in order).
    assert!(b.l.apply_decision(9, decision("Out of order"), 13).is_err());
    assert_eq!(b.l, before, "refused applies change nothing");
}

/// WB-ledger-5: a decision's source is corrected alone with a reason; the
/// same source, an unknown decision, a bad source or a blank reason are
/// refused.
#[test]
fn a_decisions_source_is_corrected_alone_and_with_a_reason() {
    let mut b = books();
    let (id, kept) = b.l.check_decision(&decision("Basis")).unwrap();
    b.l.apply_decision(id, kept, 1).unwrap();
    let fixed = DecisionSource {
        decided_by: vec!["Steve".into()],
        decided_on: d(2026, 11, 2),
        citation: None,
    };
    assert!(why(b.l.check_decision_source(9, &fixed, "r")).contains("decision 9 doesn't exist"));
    let mut empty = fixed.clone();
    empty.decided_by.clear();
    assert!(why(b.l.check_decision_source(id, &empty, "r")).contains("names at least one person"));
    let same = decision("Basis").source();
    assert!(why(b.l.check_decision_source(id, &same, "r")).contains("already has that source"));
    assert!(why(b.l.check_decision_source(id, &fixed, " ")).contains("reason is blank"));

    let (source, reason) = b.l.check_decision_source(id, &fixed, " typo ").unwrap();
    assert_eq!(reason, "typo");
    let before = b.l.clone();
    assert!(b.l.apply_decision_source(9, source.clone()).is_err());
    assert_eq!(b.l, before);
    b.l.apply_decision_source(id, source).unwrap();
    let kept = b.l.conventions().decision(id).unwrap();
    assert_eq!(kept.new.decided_by, vec!["Steve".to_string()]);
    assert_eq!(kept.new.decided_on, d(2026, 11, 2));
    assert_eq!(kept.new.citation, None);
    assert_eq!(kept.new.subject, "Basis", "only the source moved");
}

// ---------------------------------------------------------------- open items

fn item(title: &str) -> NewOpenItem {
    NewOpenItem {
        title: title.into(),
        detail_md: Some("Waiting on the CPA.".into()),
        due: Some(d(2026, 12, 15)),
        source: src(),
    }
}

/// INVARIANT 4 and 6: an open item has a titled, sourced, bounded text and
/// a due date inside the calendar; ids are allocated in order; an item
/// resolves once, with a resolution; its source is corrected alone.
#[test]
fn an_open_item_is_raised_resolved_once_and_its_source_corrected() {
    let mut b = books();
    let mut e = item(" ");
    assert!(why(b.l.check_open_item(&e)).contains("title is blank"));
    e = item("Ask CPA");
    e.due = Some(NaiveDate::MAX);
    assert!(why(b.l.check_open_item(&e)).contains("outside the calendar the books keep"));
    e = item("Ask CPA");
    e.source.decided_by = "".into();
    assert!(why(b.l.check_open_item(&e)).contains("decided_by is blank"));
    e = item("Ask CPA");
    e.detail_md = Some("SSN 123-45-6789".into());
    assert!(why(b.l.check_open_item(&e)).contains("detail"));

    let (id, kept) = b.l.check_open_item(&item("  Ask CPA  ")).unwrap();
    assert_eq!((id, kept.title.as_str()), (1, "Ask CPA"));
    let before = b.l.clone();
    assert!(
        b.l.apply_open_item(5, kept.clone(), 1).is_err(),
        "not the next id"
    );
    assert_eq!(b.l, before);
    b.l.apply_open_item(id, kept, 1).unwrap();
    assert_eq!(b.l.conventions().next_open_item_id(), 2);

    // Resolution.
    assert!(why(b.l.check_resolve_item(9, "done", &src())).contains("unknown open item 9"));
    assert!(why(b.l.check_resolve_item(id, " ", &src())).contains("resolution is blank"));
    let mut bad = src();
    bad.decided_on = NaiveDate::MAX;
    assert!(
        why(b.l.check_resolve_item(id, "done", &bad))
            .contains("outside the calendar the books keep")
    );
    let text =
        b.l.check_resolve_item(id, " Filed 2026-12-01 ", &src())
            .unwrap();
    assert_eq!(text, "Filed 2026-12-01");
    assert!(b.l.apply_resolve_item(9, text.clone(), src(), 2).is_err());
    b.l.apply_resolve_item(id, text.clone(), src(), 2).unwrap();
    assert!(why(b.l.check_resolve_item(id, "again", &src())).contains("already resolved"));
    let before = b.l.clone();
    assert!(b.l.apply_resolve_item(id, text, src(), 3).is_err());
    assert_eq!(b.l, before);

    // Source correction.
    let mut fixed = src();
    fixed.decided_by = "Tobin".into();
    assert!(why(b.l.check_open_item_source(9, &fixed, "r")).contains("unknown open item 9"));
    assert!(why(b.l.check_open_item_source(id, &src(), "r")).contains("already has that source"));
    assert!(why(b.l.check_open_item_source(id, &fixed, "")).contains("reason is blank"));
    let mut bad = fixed.clone();
    bad.decided_by = "".into();
    assert!(why(b.l.check_open_item_source(id, &bad, "r")).contains("decided_by is blank"));
    let (source, reason) =
        b.l.check_open_item_source(id, &fixed, "wrong name")
            .unwrap();
    assert_eq!(reason, "wrong name");
    assert!(b.l.apply_open_item_source(9, source.clone()).is_err());
    b.l.apply_open_item_source(id, source).unwrap();
    assert_eq!(
        b.l.conventions()
            .open_item(id)
            .unwrap()
            .new
            .source
            .decided_by,
        "Tobin"
    );
}

// ---------------------------------------------------------------- approvals

fn draft(l: &mut Ledger, accounts: (u64, u64), amount: &str) -> EntryId {
    let e = l.create_entry(d(2026, 11, 2), "Spend");
    let m = Money::new(dec(amount), Currency::USD);
    l.add_line(e, accounts.0, Side::Debit, m).unwrap();
    l.add_line(e, accounts.1, Side::Credit, m).unwrap();
    e
}

/// WB-ledger-10: an approval is of a draft that exists and is not posted,
/// by a named approver, with a bounded note and a good source.
#[test]
fn an_approval_is_refused_for_a_posted_or_unknown_entry_and_bad_text() {
    let mut b = books();
    let e = draft(&mut b.l, (b.expense, b.bank), "50.00");
    assert_eq!(
        b.l.check_approval(99, "Steve", None, &src()).unwrap_err(),
        LedgerError::UnknownEntry(99)
    );
    assert!(why(b.l.check_approval(e, "  ", None, &src())).contains("approver is blank"));
    assert!(
        why(b
            .l
            .check_approval(e, "Steve", Some("SSN 123-45-6789"), &src()))
        .contains("note")
    );
    let mut bad = src();
    bad.decided_by = "".into();
    assert!(why(b.l.check_approval(e, "Steve", None, &bad)).contains("decided_by is blank"));

    let a =
        b.l.check_approval(e, " Steve ", Some(" fine "), &src())
            .unwrap();
    assert_eq!(
        (a.approver.as_str(), a.note.as_deref()),
        ("Steve", Some("fine"))
    );
    assert!(!a.fingerprint.is_empty());
    assert_eq!(
        b.l.apply_approval(99, a.clone()).unwrap_err(),
        LedgerError::UnknownEntry(99)
    );
    b.l.apply_approval(e, a).unwrap();
    assert_eq!(b.l.conventions().approvals(e).len(), 1);

    b.l.post_entry(e).unwrap();
    assert_eq!(
        b.l.check_approval(e, "Steve", None, &src()).unwrap_err(),
        LedgerError::EntryAlreadyPosted(e)
    );
}

/// WB-ledger-10 and INVARIANT 3: approvals bind only when the company
/// enforces them. Off: nothing is missing, whatever the rules. Enforced: a
/// covered draft lacks its approver until that approver (matched by name,
/// however spelled) approved it AS IT STANDS; a rule this version cannot
/// read covers everything and names no approver (fail closed); an unknown
/// entry is refused.
#[test]
fn approvals_are_missing_only_under_enforcement_and_fail_closed() {
    let mut b = books();
    set_convention(
        &mut b.l,
        &rule(
            "big-spend",
            json!({"approver": "Steve", "amount_over": "100"}),
        ),
        1,
    );
    let small = draft(&mut b.l, (b.expense, b.bank), "100.00");
    let big = draft(&mut b.l, (b.expense, b.bank), "100.01");

    assert_eq!(
        b.l.approvals_missing(big).unwrap(),
        vec![],
        "off by default"
    );
    settle(&mut b.l, "policy.approval_rules", json!("enforce"));
    assert_eq!(
        b.l.approvals_missing(99).unwrap_err(),
        LedgerError::UnknownEntry(99)
    );
    assert!(
        b.l.approvals_missing(small).unwrap().is_empty(),
        "exactly the amount is not over it"
    );
    assert_eq!(
        b.l.approvals_missing(big).unwrap(),
        vec![("big-spend".to_string(), "Steve".to_string())]
    );

    // Approved as it stands (name matched loosely): satisfied.
    let a = b.l.check_approval(big, "steve", None, &src()).unwrap();
    b.l.apply_approval(big, a).unwrap();
    assert!(b.l.approvals_missing(big).unwrap().is_empty());
    // A line added after: the approval no longer fits the draft.
    b.l.add_line(
        big,
        b.expense,
        Side::Debit,
        Money::new(dec("1.00"), Currency::USD),
    )
    .unwrap();
    b.l.add_line(
        big,
        b.bank,
        Side::Credit,
        Money::new(dec("1.00"), Currency::USD),
    )
    .unwrap();
    assert_eq!(b.l.approvals_missing(big).unwrap().len(), 1);

    // A rule this version cannot read (a newer version's structure, as
    // replay takes it) covers every entry and names no approver.
    let future = convention(
        "future",
        ConventionKind::ApprovalRule,
        Some(json!({"quorum": 3})),
    );
    b.l.apply_convention(future, 5).unwrap();
    let missing = b.l.approvals_missing(small).unwrap();
    assert_eq!(
        missing,
        vec![(
            "future".to_string(),
            "an approver this version can't read".to_string()
        )]
    );
}

/// WB-ledger-10: a rule that names accounts covers an entry touching one
/// of them; the fingerprint is how an approval notices its draft changed:
/// a party, a share's kind and amount, or a stated reason for having none
/// each change it, and a decimal's spelling does not.
#[test]
fn the_fingerprint_notices_parties_shares_and_reasons() {
    let mut b = books();
    let e = draft(&mut b.l, (b.expense, b.bank), "20.00");
    let entry = b.l.get_entry(e).unwrap();
    let share = |share| PartyShare { party: 1, share };
    let fp =
        |parties: &[PartyShare], none: Option<&str>| conventions::fingerprint(entry, parties, none);

    let plain = fp(&[], None);
    let derived = fp(&[share(Share::Derived)], None);
    let amount = fp(&[share(Share::Amount(dec("12.50")))], None);
    let percent = fp(&[share(Share::Percent(dec("50")))], None);
    let none = fp(&[], Some("company expense"));
    let all = [&plain, &derived, &amount, &percent, &none];
    for (i, x) in all.iter().enumerate() {
        for y in &all[i + 1..] {
            assert_ne!(x, y);
        }
    }
    // Trailing zeros are not a change; another amount is; another party is.
    assert_eq!(amount, fp(&[share(Share::Amount(dec("12.500")))], None));
    assert_ne!(amount, fp(&[share(Share::Amount(dec("12.51")))], None));
    assert_ne!(
        amount,
        fp(
            &[PartyShare {
                party: 2,
                share: Share::Amount(dec("12.50"))
            }],
            None
        )
    );
    assert_eq!(percent, fp(&[share(Share::Percent(dec("50.0")))], None));
    assert_ne!(none, fp(&[], Some("owner expense")));
}

// ---------------------------------------------------------------- drafting from a template

/// WB-ledger-9: drafting from a template names the accounts for its slots,
/// splits the amount exactly on each side and refuses anything it cannot
/// make whole: an unknown, retired or non-template convention, a missing or
/// surplus slot, an amount that is not above zero, over the line limit, in
/// fractions of a cent, too small to split, or an account the chart lacks
/// or a heading.
#[test]
fn drafting_from_a_template_refuses_what_it_cannot_make_whole() {
    let mut b = books();
    b.l.add_account("1010", "Savings", AccountType::Asset, None, false)
        .unwrap();
    let three_way = convention(
        "split",
        ConventionKind::PostingTemplate,
        Some(json!({"description": "Split purchase", "lines": [
            tline("{expense}", "debit", "0.5"),
            tline("6000", "debit", "0.5"),
            tline("1000", "credit", "0.5"),
            tline("1010", "credit", "0.5"),
        ]})),
    );
    set_convention(&mut b.l, &three_way, 1);
    set_convention(&mut b.l, &convention("note", ConventionKind::Note, None), 2);
    set_convention(&mut b.l, &two_line_template("old"), 3);
    let reason = b.l.check_retire_convention("old", "r", &src()).unwrap();
    b.l.apply_retire_convention("old", reason, src(), 4)
        .unwrap();

    let slots = |pairs: &[(&str, &str)]| -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    };
    let before = b.l.clone();
    let go = |id: &str, amount: &str, s: &BTreeMap<String, String>| {
        b.l.template_lines(id, dec(amount), s)
    };

    assert!(why(go("missing", "10", &slots(&[]))).contains("unknown convention"));
    assert!(why(go("old", "10", &slots(&[]))).contains("template old is retired"));
    assert!(why(go("note", "10", &slots(&[]))).contains("is a note, not a posting template"));
    let m = why(go("split", "10", &slots(&[])));
    assert!(
        m.contains("needs an account for expense: pass accounts {\"expense\": \"<code>\"}"),
        "{m}"
    );
    let m = why(go(
        "split",
        "10",
        &slots(&[("expense", "6000"), ("bank", "1000")]),
    ));
    assert!(
        m.contains("has no slot \"bank\"; its slots: expense"),
        "{m}"
    );
    let m = why(go("old2", "10", &slots(&[])));
    assert!(m.contains("unknown convention"));
    let ok_slots = slots(&[("expense", "6000")]);
    assert!(why(go("split", "0", &ok_slots)).contains("must be above zero"));
    assert!(why(go("split", "-5", &ok_slots)).contains("must be above zero"));
    assert!(matches!(
        go("split", "1000000000000000.01", &ok_slots),
        Err(LedgerError::AmountOutOfRange)
    ));
    assert!(
        go("split", "10.001", &ok_slots).is_err(),
        "a fraction of a cent"
    );
    // 0.01 split in halves: 0.005 each side is under a minor unit, so a part is zero.
    assert!(why(go("split", "0.01", &ok_slots)).contains("too small to split"));
    assert!(why(go("split", "10", &slots(&[("expense", "7777")]))).contains("is not in the chart"));
    assert!(why(go("split", "10", &slots(&[("expense", "5000")]))).contains("is a placeholder"));

    // And made whole: 10.01 in halves is 5.01 / 5.00 by largest remainder
    // on each side, summing to 10.01 exactly.
    let (description, lines) = go("split", "10.01", &ok_slots).unwrap();
    assert_eq!(description.as_deref(), Some("Split purchase"));
    for side in [Side::Debit, Side::Credit] {
        let parts: Vec<Decimal> = lines
            .iter()
            .filter(|(_, s, _)| *s == side)
            .map(|(_, _, m)| m.amount)
            .collect();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts.iter().sum::<Decimal>(), dec("10.01"), "{side:?}");
    }
    assert_eq!(b.l, before, "drafting asks and changes nothing");
}

// ---------------------------------------------------------------- the pure helpers

/// WB-ledger-9: a template's split is exact. With no rounding places,
/// each part is amount × share and the last absorbs any difference; with
/// places, each part is truncated and the leftover goes by largest
/// remainder; either way the parts sum to the amount.
#[test]
fn a_split_sums_to_its_amount_exactly() {
    let thirds = [dec("0.333"), dec("0.333"), dec("0.334")];
    let exact = conventions::split(dec("100.00"), &thirds, None).unwrap();
    assert_eq!(exact.iter().sum::<Decimal>(), dec("100.00"));
    assert_eq!(exact[0], dec("33.300"));
    assert_eq!(exact[1], dec("33.300"));

    let cents = conventions::split(
        dec("100.00"),
        &[dec("0.3333333"), dec("0.3333333"), dec("0.3333334")],
        Some(2),
    )
    .unwrap();
    assert_eq!(cents.iter().sum::<Decimal>(), dec("100.00"));
    assert!(cents.iter().all(|p| p.scale() <= 2));

    let odd = conventions::split(dec("0.05"), &[dec("0.5"), dec("0.5")], Some(2)).unwrap();
    assert_eq!(odd.iter().sum::<Decimal>(), dec("0.05"));
    assert_eq!(odd.iter().copied().max().unwrap(), dec("0.03"));

    // Past exact arithmetic: refused, not a panic.
    assert_eq!(
        conventions::split(Decimal::MAX, &[dec("1.5"), dec("1.5")], None),
        None
    );
}

/// The convention kinds' words round-trip, and an unknown word is none.
#[test]
fn convention_kinds_round_trip_their_words() {
    for k in ConventionKind::ALL {
        assert_eq!(ConventionKind::parse(k.key()), Some(k));
    }
    assert_eq!(ConventionKind::parse("Policy"), None);
    assert_eq!(ConventionKind::parse(""), None);
}

/// `Approval` carries what an approval needs and nothing a refused one
/// would leave behind: constructing one by hand and applying it is how
/// replay restores it.
#[test]
fn replay_restores_an_approval_as_journaled() {
    let mut b = books();
    let e = draft(&mut b.l, (b.expense, b.bank), "5.00");
    let a = Approval {
        approver: "Steve".into(),
        fingerprint: "deadbeef".into(),
        note: None,
        source: src(),
        seq: 4,
    };
    b.l.apply_approval(e, a.clone()).unwrap();
    assert_eq!(b.l.conventions().approvals(e), [a]);
    assert!(b.l.conventions().approvals(e + 100).is_empty());
}
