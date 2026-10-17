//! Conventions, approvals and templates in the engine, value by value:
//! slots, summaries, template bounds, the approval fingerprint (journaled,
//! so its meaning is fixed forever), which entries a rule covers, and
//! when a convention is a revision at all.
//!
//! ```text
//! GOAL: every bound and every rule says exactly what it means; no more,
//!       no less.
//! SOURCE: plan/served.md P1; P1 mutation run (2026-10-17).
//! ```

use chrono::NaiveDate;
use ledger_core::conventions::{self, Convention, ConventionKind, NewConvention};
use ledger_core::settings::{Change, Source};
use ledger_core::{AccountType, Currency, EntryId, Ledger, LedgerError, Money, Side};
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
        decided_by: "A".into(),
        decided_on: d(2026, 10, 17),
        citation: None,
        note: None,
    }
}

fn convention(id: &str, kind: ConventionKind, structured: Option<Value>) -> NewConvention {
    NewConvention {
        id: id.into(),
        kind,
        title: "Title".into(),
        body_md: "Body.".into(),
        structured,
        source: src(),
    }
}

fn template(lines: Vec<Value>) -> NewConvention {
    convention(
        "t",
        ConventionKind::PostingTemplate,
        Some(json!({ "lines": lines })),
    )
}

fn line(account: &str, side: &str, share: &str) -> Value {
    json!({"account": account, "side": side, "share": share})
}

fn refusal(c: &NewConvention) -> String {
    conventions::check_new(c).expect_err("refused")
}

/// A slot is a name in braces: lowercase letters, digits and `_`, 1 to 32
/// of them.
#[test]
fn a_slot_is_a_short_lowercase_name_in_braces() {
    for (account, want) in [
        ("{expense}", Some("expense")),
        ("{a_1}", Some("a_1")),
        ("{_}", Some("_")),
        ("{7}", Some("7")),
        (
            "{abcdefghijklmnopqrstuvwxyz012345}",
            Some("abcdefghijklmnopqrstuvwxyz012345"),
        ),
        ("{abcdefghijklmnopqrstuvwxyz0123456}", None),
        ("{}", None),
        ("{Expense}", None),
        ("{a-b}", None),
        ("{a b}", None),
        ("expense", None),
        ("{expense", None),
        ("6000", None),
    ] {
        assert_eq!(conventions::slot(account), want, "{account}");
    }
}

/// A summary is the first line with text, trimmed, at most 160
/// characters, with an ellipsis only when it was cut.
#[test]
fn a_summary_is_the_first_line_with_text() {
    let summary = |body: &str| {
        Convention {
            id: "c".into(),
            kind: ConventionKind::Note,
            title: "T".into(),
            body_md: body.into(),
            structured: None,
            source: src(),
            seq: 1,
            revision: 1,
            retired: None,
        }
        .summary()
    };
    assert_eq!(summary("\n   \n  First line  \nsecond"), "First line");
    assert_eq!(summary(""), "");
    assert_eq!(summary("\n \n"), "");
    let exactly = "é".repeat(160);
    assert_eq!(summary(&exactly), exactly, "160 characters: whole");
    let over = "é".repeat(161);
    assert_eq!(summary(&over), format!("{exactly}…"), "161: cut");
    assert_eq!(summary(&format!("  {exactly}  ")), exactly, "trimmed first");
}

/// A posting template has 2 to 50 lines, each share in (0, 1], each
/// side summing to exactly 1; a refusal names the line.
#[test]
fn template_bounds_are_exact() {
    // 50 lines: one debit, 49 credits summing to 1.
    let mut fifty = vec![line("6000", "debit", "1"), line("1000", "credit", "0.04")];
    fifty.extend((0..48).map(|_| line("1000", "credit", "0.02")));
    assert_eq!(fifty.len(), 50);
    conventions::check_new(&template(fifty)).expect("50 lines");
    let mut fifty_one = vec![line("6000", "debit", "1")];
    fifty_one.extend((0..50).map(|_| line("1000", "credit", "0.02")));
    assert!(refusal(&template(fifty_one)).contains("2 to 50 lines"));
    assert!(refusal(&template(vec![])).contains("2 to 50 lines"));
    // One line can't balance: refused by its count, not its shares.
    assert!(refusal(&template(vec![line("6000", "debit", "1")])).contains("2 to 50 lines"));
    conventions::check_new(&template(vec![
        line("6000", "debit", "1"),
        line("1000", "credit", "1"),
    ]))
    .expect("2 lines");
    // A zero share is no share.
    let e = refusal(&template(vec![
        line("6000", "debit", "1"),
        line("6000", "debit", "0"),
        line("1000", "credit", "1"),
    ]));
    assert!(e.starts_with("template line 2: share \"0\""), "{e}");
    // Over 1, or below zero, even when the side still sums to 1.
    let e = refusal(&template(vec![
        line("6000", "debit", "1.5"),
        line("6000", "debit", "-0.5"),
        line("1000", "credit", "1"),
    ]));
    assert!(e.starts_with("template line 1: share \"1.5\""), "{e}");
    let e = refusal(&template(vec![
        line("6000", "debit", "1"),
        line("1000", "credit", "1.5"),
        line("1000", "credit", "-0.5"),
    ]));
    assert!(e.starts_with("template line 2: share \"1.5\""), "{e}");
    let e = refusal(&template(vec![
        line("6000", "debit", "1"),
        line("1000", "credit", "0.5"),
    ]));
    assert!(e.contains("sum to exactly 1"), "{e}");
}

/// FNV-1a, 64 bits, written out here independently of the engine.
fn fnv1a(text: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// The approval fingerprint is journaled with every approval and
/// recomputed at post: its text and hash are fixed forever.
#[test]
fn the_fingerprint_is_fixed_forever() {
    let mut l = Ledger::new(Currency::USD);
    let exp = l
        .add_account("6000", "Software", AccountType::Expense, None, false)
        .unwrap();
    let bank = l
        .add_account("1000", "Bank", AccountType::Asset, None, false)
        .unwrap();
    let e = l.create_entry(d(2026, 10, 2), "Laptop");
    let m = Money::new(dec("900.50"), Currency::USD);
    l.add_line(e, exp, Side::Debit, m).unwrap();
    l.add_line(e, bank, Side::Credit, m).unwrap();
    let entry = l.get_entry(e).unwrap();
    let text = format!(
        "2026-10-02\u{1f}Laptop\
         \u{1e}{exp}\u{1f}D\u{1f}900.5\u{1f}USD\u{1f}\u{1f}\u{1f}\
         \u{1e}{bank}\u{1f}C\u{1f}900.5\u{1f}USD\u{1f}\u{1f}\u{1f}"
    );
    let plain = conventions::fingerprint(entry, &[], None);
    assert_eq!(plain, fnv1a(&text));
    assert_eq!(fnv1a(""), "cbf29ce484222325", "FNV-1a's offset basis");
    assert_eq!(fnv1a("a"), "af63dc4c8601ec8c", "FNV-1a's published value");
    let none = conventions::fingerprint(entry, &[], Some("bank fee"));
    let with_none = format!(
        "2026-10-02\u{1f}Laptop\u{1d}none\u{1f}bank fee\
         \u{1e}{exp}\u{1f}D\u{1f}900.5\u{1f}USD\u{1f}\u{1f}\u{1f}\
         \u{1e}{bank}\u{1f}C\u{1f}900.5\u{1f}USD\u{1f}\u{1f}\u{1f}"
    );
    assert_eq!(none, fnv1a(&with_none));
    assert_ne!(none, plain);
}

/// Books enforcing approval rules, with an expense and a bank account.
fn enforcing() -> (Ledger, ledger_core::AccountId, ledger_core::AccountId) {
    let mut l = Ledger::new(Currency::USD);
    let exp = l
        .add_account("6000", "Software", AccountType::Expense, None, false)
        .unwrap();
    let bank = l
        .add_account("1000", "Bank", AccountType::Asset, None, false)
        .unwrap();
    let on = [Change {
        key: "policy.approval_rules".into(),
        value: Some(json!("enforce")),
    }];
    let made = l.check_settings(&on, &src()).unwrap();
    l.apply_settings(&made, &src(), 1).unwrap();
    (l, exp, bank)
}

fn set_convention(l: &mut Ledger, c: &NewConvention, seq: u64) {
    let c = l.check_convention(c).unwrap();
    l.apply_convention(c, seq).unwrap();
}

fn draft(l: &mut Ledger, lines: &[(ledger_core::AccountId, Side, &str)]) -> EntryId {
    let e = l.create_entry(d(2026, 10, 2), "Spend");
    for (a, side, amount) in lines {
        l.add_line(e, *a, *side, Money::new(dec(amount), Currency::USD))
            .unwrap();
    }
    e
}

/// A rule's amount is "over": an entry whose debits are exactly it isn't
/// covered; one a cent over is. It counts debits, not credits, even on a
/// draft that doesn't balance yet.
#[test]
fn a_rule_covers_debits_strictly_over_its_amount() {
    let (mut l, exp, bank) = enforcing();
    set_convention(
        &mut l,
        &convention(
            "big",
            ConventionKind::ApprovalRule,
            Some(json!({"approver": "B", "amount_over": "500"})),
        ),
        2,
    );
    let want = vec![("big".to_string(), "B".to_string())];
    let at = draft(
        &mut l,
        &[(exp, Side::Debit, "500"), (bank, Side::Credit, "500")],
    );
    assert!(l.approvals_missing(at).unwrap().is_empty(), "exactly 500");
    let over = draft(
        &mut l,
        &[(exp, Side::Debit, "500.01"), (bank, Side::Credit, "500.01")],
    );
    assert_eq!(l.approvals_missing(over).unwrap(), want);
    let debits = draft(&mut l, &[(exp, Side::Debit, "600")]);
    assert_eq!(l.approvals_missing(debits).unwrap(), want, "debits count");
    let credits = draft(&mut l, &[(bank, Side::Credit, "600")]);
    assert!(
        l.approvals_missing(credits).unwrap().is_empty(),
        "credits don't"
    );
}

/// What check_settings returns is what will be journaled.
#[test]
fn check_settings_returns_the_changes_it_will_journal() {
    let l = Ledger::new(Currency::USD);
    let name = Change {
        key: "company.name".into(),
        value: Some(json!("  Acme LLC ")),
    };
    let made = l.check_settings(&[name], &src()).unwrap();
    assert_eq!(
        made,
        [Change {
            key: "company.name".into(),
            value: Some(json!("Acme LLC")),
        }]
    );
    assert!(!l.settings().is_journaled(), "checking changes nothing");
}

/// A convention set again unchanged is refused; a change to its title,
/// its body or its structure alone is a revision.
#[test]
fn any_one_change_is_a_revision() {
    let (mut l, _, _) = enforcing();
    let base = convention(
        "big",
        ConventionKind::ApprovalRule,
        Some(json!({"approver": "B", "amount_over": "500"})),
    );
    set_convention(&mut l, &base, 2);
    let e = l.check_convention(&base).unwrap_err();
    assert!(
        matches!(&e, LedgerError::InvalidConvention(m) if m.contains("already says exactly that")),
        "{e}"
    );
    let title = NewConvention {
        title: "New title".into(),
        ..base.clone()
    };
    let body = NewConvention {
        body_md: "New body.".into(),
        ..base.clone()
    };
    let structured = NewConvention {
        structured: Some(json!({"approver": "B", "amount_over": "600"})),
        ..base.clone()
    };
    for c in [title, body, structured] {
        assert!(l.check_convention(&c).is_ok(), "{c:?}");
    }
    let revised = NewConvention {
        title: "New title".into(),
        ..base.clone()
    };
    set_convention(&mut l, &revised, 3);
    assert_eq!(l.conventions().convention("big").unwrap().revision, 2);
}

/// A template drafts any amount up to a line's most, and refuses above
/// it as out of range.
#[test]
fn a_template_drafts_up_to_a_lines_most() {
    let mut l = Ledger::new(Currency::USD);
    for (code, t) in [("6000", AccountType::Expense), ("1000", AccountType::Asset)] {
        l.add_account(code, code, t, None, false).unwrap();
    }
    set_convention(
        &mut l,
        &template(vec![
            line("6000", "debit", "1"),
            line("1000", "credit", "1"),
        ]),
        1,
    );
    let max = ledger_core::kernel::MAX_LINE_AMOUNT;
    let (_, lines) = l.template_lines("t", max, &BTreeMap::new()).unwrap();
    assert!(lines.iter().all(|(_, _, m)| m.amount == max));
    assert!(matches!(
        l.template_lines("t", max + dec("0.01"), &BTreeMap::new()),
        Err(LedgerError::AmountOutOfRange)
    ));
}
