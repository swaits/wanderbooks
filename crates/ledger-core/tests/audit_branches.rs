//! The audit's branches (plan P14 unit V-D2): the words, orders and
//! thresholds of `audit.rs` that `tests/audit.rs`'s one-seed-per-code
//! test does not reach. Coverage found `audit.rs` at 80% of lines and 61%
//! of branches.
//!
//! ```text
//! GOAL: an audit says exactly what is wrong, in a fixed order, from the
//!       ledger alone, and an attestation counts only against the bar the
//!       books set.
//! SOURCE: BS-ledger-13; WB-ledger-16; BS-statement-1 (the statements an
//!         audit reads); BS-arap-2 (control accounts); BS-partner-1 (the
//!         cumulative split a close must keep); BS-entry-5 (evidence);
//!         DECISION D-9 (the bar), D-47 (a reversal's evidence is its
//!         original's).
//! ```
//!
//! The "cannot be made" arms for equation, control and register (a
//! statement that overflows exact arithmetic) are not reachable through a
//! posted book: a line is capped at one quadrillion units
//! (`kernel::MAX_LINE_AMOUNT`) so no realistic sum overflows. They stay
//! unexercised here and are named in the unit's report.

use std::collections::BTreeSet;

use chrono::NaiveDate;
use ledger_core::arap::{ControlCheck, DocAttention, UnattachedLine};
use ledger_core::audit::{
    self, AttestationBar, AuditInput, CODES, Effort, Finding, Method, Scope, Severity, Subject,
};
use ledger_core::cashflow::CashFlowFooting;
use ledger_core::periods::MemberSplit;
use ledger_core::settings::{Change, Settings, Source};
use ledger_core::{
    AccountType, CloseTo, Currency, DocRef, DocStatus, EntryId, Ledger, MemberClose, Money, Side,
};
use rust_decimal::Decimal;
use serde_json::json;
use std::str::FromStr;

fn d(m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, m, day).unwrap()
}

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap()
}

fn usd(s: &str) -> Money {
    Money::new(dec(s), Currency::USD)
}

fn post(l: &mut Ledger, date: NaiveDate, dr: u64, cr: u64, amt: &str) -> EntryId {
    let e = l.create_entry(date, "x");
    l.add_line(e, dr, Side::Debit, usd(amt)).unwrap();
    l.add_line(e, cr, Side::Credit, usd(amt)).unwrap();
    l.post_entry(e).unwrap();
    e
}

struct Books {
    l: Ledger,
    cash: u64,
    expense: u64,
    revenue: u64,
}

fn books() -> Books {
    let mut l = Ledger::new(Currency::USD);
    let mut a = |code: &str, name: &str, t| l.add_account(code, name, t, None, false).unwrap();
    let cash = a("1000", "Bank", AccountType::Asset);
    a("3000", "Capital", AccountType::Equity);
    let revenue = a("4000", "Sales", AccountType::Revenue);
    let expense = a("6000", "Supplies", AccountType::Expense);
    l.set_account_tax(revenue, Some("gross_receipts"), None)
        .unwrap();
    l.set_account_tax(expense, Some("supplies"), None).unwrap();
    Books {
        l,
        cash,
        expense,
        revenue,
    }
}

fn run(
    l: &Ledger,
    settings: &Settings,
    receipts: &BTreeSet<EntryId>,
    through: NaiveDate,
) -> Vec<Finding> {
    audit::statements(
        l,
        &AuditInput {
            settings,
            receipts,
            cash_flow: None,
            settings_problems: &[],
            undecided_1099: &[],
        },
        through,
    )
}

fn codes(f: &[Finding]) -> Vec<&'static str> {
    f.iter().map(|f| f.code).collect()
}

// ---------------------------------------------------------------- the words

/// BS-ledger-13: a finding's severity comes from its code's row; a code no
/// row names is a Failure (a check the table forgot is never softened).
/// Failures sort before warnings before info.
#[test]
fn a_finding_takes_its_codes_severity_and_an_unknown_code_is_a_failure() {
    for row in CODES {
        assert_eq!(
            Finding::new(row.code, Subject::Books, "x").severity,
            row.severity,
            "{}",
            row.code
        );
    }
    assert_eq!(
        Finding::new("not_a_code", Subject::Books, "x").severity,
        Severity::Failure
    );
    assert!(Severity::Failure < Severity::Warning && Severity::Warning < Severity::Info);
    assert_eq!(
        [Severity::Failure, Severity::Warning, Severity::Info].map(Severity::key),
        ["failure", "warning", "info"]
    );
}

/// BS-ledger-13: what a finding is about, in words.
#[test]
fn a_subject_describes_itself_in_words() {
    let cases = [
        (Subject::Books, "books"),
        (Subject::Account(4), "account 4"),
        (Subject::Entry(9), "entry 9"),
        (Subject::Invoice(2), "invoice 2"),
        (Subject::Bill(3), "bill 3"),
        (Subject::Period(1), "period 1"),
        (Subject::Schedule(5), "schedule 5"),
        (Subject::Attachment(8), "attachment 8"),
        (Subject::Vendor("Dana Co".into()), "vendor Dana Co"),
        (Subject::Named("audit.log".into()), "audit.log"),
    ];
    for (subject, words) in cases {
        assert_eq!(subject.describe(), words);
    }
}

/// BS-ledger-13, INVARIANT 4 (a check that cannot be made is a finding):
/// the statement of cash flows is tried by the caller. Nothing tried says
/// nothing; one that foots says nothing; one that does not foot, and one
/// that could not be made, each say so with the figures or the reason.
#[test]
fn the_cash_flow_check_says_what_it_found() {
    assert_eq!(audit::check_cash_flow(None), None);
    let foots = Ok(CashFlowFooting {
        sections_sum: dec("40.00"),
        net_change: dec("40.00"),
        difference: Decimal::ZERO,
    });
    assert_eq!(audit::check_cash_flow(Some(&foots)), None);

    let off = Ok(CashFlowFooting {
        sections_sum: dec("40.00"),
        net_change: dec("45.00"),
        difference: dec("-5.00"),
    });
    let f = audit::check_cash_flow(Some(&off)).unwrap();
    assert_eq!(
        (f.code, f.severity),
        ("cash_flow_unfooted", Severity::Failure)
    );
    assert_eq!(
        f.message,
        "the sections sum to 40.00 but the cash accounts changed by 45.00"
    );

    let unmade: Result<CashFlowFooting, String> = Err("no cash accounts named".into());
    let f = audit::check_cash_flow(Some(&unmade)).unwrap();
    assert_eq!(f.code, "cash_flow_unfooted");
    assert_eq!(
        f.message,
        "the statement of cash flows cannot be made: no cash accounts named"
    );
}

/// BS-arap-2 as an audit reads it: a control account whose balance differs
/// from its documents is a failure naming the account and the figures; a
/// line carrying no document is a warning on its entry; an overpaid bill is
/// a warning on the bill (and an invoice on the invoice); a document in
/// another status needing attention says nothing here.
#[test]
fn control_accounts_report_differences_unattached_lines_and_overpaid_documents() {
    let clean = ControlCheck {
        role: "payables".into(),
        account: 12,
        balance: dec("200.00"),
        documents_total: dec("200.00"),
        unattached: vec![],
        difference: Decimal::ZERO,
        attention: vec![],
    };
    assert!(audit::check_control(std::slice::from_ref(&clean)).is_empty());

    let off = ControlCheck {
        difference: dec("3.50"),
        balance: dec("203.50"),
        ..clean.clone()
    };
    let f = audit::check_control(&[off]);
    assert_eq!(codes(&f), ["control_difference"]);
    assert_eq!(f[0].subject, Subject::Account(12));
    assert_eq!(
        f[0].message,
        "the payables account's balance 203.50 differs from its documents 200.00 by 3.50"
    );

    let loose = ControlCheck {
        unattached: vec![UnattachedLine {
            entry: 31,
            line: 1,
            date: d(3, 4),
            amount: dec("75.00"),
        }],
        ..clean.clone()
    };
    let f = audit::check_control(&[loose]);
    assert_eq!(codes(&f), ["unattached_control_line"]);
    assert_eq!(f[0].subject, Subject::Entry(31));
    assert!(f[0].message.contains("line 1 of entry 31"));
    assert!(f[0].message.contains("75.00 on the payables account"));

    let attention = |doc, status| DocAttention {
        doc,
        status,
        total: dec("90.00"),
        outstanding: dec("-10.00"),
    };
    let overpaid = ControlCheck {
        attention: vec![
            attention(DocRef::Bill(6), DocStatus::Overpaid),
            attention(DocRef::Invoice(2), DocStatus::Overpaid),
            attention(DocRef::Bill(7), DocStatus::Unattached),
        ],
        ..clean
    };
    let f = audit::check_control(&[overpaid]);
    assert_eq!(codes(&f), ["document_overpaid", "document_overpaid"]);
    assert_eq!(f[0].subject, Subject::Bill(6));
    assert_eq!(f[1].subject, Subject::Invoice(2));
    assert!(f.iter().all(|x| x.severity == Severity::Warning));
}

/// BS-partner-1 (the cumulative rule): a member's running total of the
/// closes may be at most one minor unit from its exact share; exactly one
/// is fine, one cent and a bit more is drift, in either direction.
#[test]
fn a_close_split_drifts_only_beyond_a_minor_unit() {
    let m = |running: &str, exact: &str| MemberSplit {
        capital: 3001,
        share: dec("0.3"),
        part: dec("0.32"),
        running: dec(running),
        exact: dec(exact),
    };
    let unit = dec("0.01");
    assert!(audit::check_split(1, "Jan", &[m("0.32", "0.315")], unit).is_empty());
    assert!(audit::check_split(1, "Jan", &[m("0.32", "0.31")], unit).is_empty());
    assert!(audit::check_split(1, "Jan", &[m("0.31", "0.32")], unit).is_empty());
    let f = audit::check_split(
        4,
        "Jan 2026",
        &[m("0.31", "0.3"), m("0.28", "0.3"), m("0.33", "0.3")],
        unit,
    );
    assert_eq!(
        f.len(),
        2,
        "the first is within the unit; the other two are not"
    );
    assert!(f.iter().all(|x| x.code == "close_split_drifts"));
    assert_eq!(f[0].subject, Subject::Period(4));
    assert_eq!(
        f[0].message,
        "after the close of Jan 2026, account 3001 holds 0.28 of the closes where its exact share is 0.3"
    );
}

/// BS-partner-1 read by the audit end to end: a members close that splits
/// 1.05 among 0.3 / 0.3 / 0.4 cumulatively leaves every member within a
/// cent of their exact share, so the closed period raises nothing and the
/// books' own checks stay quiet.
#[test]
fn a_members_close_the_audit_walks_raises_nothing() {
    let mut b = books();
    let caps: Vec<u64> = ["3001", "3002", "3003"]
        .iter()
        .map(|c| {
            b.l.add_account(c, c, AccountType::Equity, None, false)
                .unwrap()
        })
        .collect();
    post(&mut b.l, d(1, 5), b.cash, b.revenue, "1.05");
    let jan = b.l.create_period("2026-01", d(1, 1), d(1, 31)).unwrap();
    let members = caps
        .iter()
        .zip(["0.3", "0.3", "0.4"])
        .map(|(&capital, s)| MemberClose {
            capital,
            share: dec(s),
            distributions: None,
        })
        .collect();
    b.l.close_period_to(jan, &CloseTo::Members(members))
        .unwrap();
    // 0.315, 0.315, 0.42 to the cent, odd cent by largest remainder.
    let parts: Vec<Decimal> = caps
        .iter()
        .map(|&a| b.l.account_balance(a).unwrap())
        .collect();
    assert_eq!(parts.iter().sum::<Decimal>(), dec("1.05"));
    let f = run(&b.l, &Settings::default(), &all_receipts(&b.l), d(12, 31));
    assert!(
        f.iter().all(|x| x.severity == Severity::Info),
        "{:?}",
        codes(&f)
    );
    // Through a date before the close the period is not read at all.
    let before = run(&b.l, &Settings::default(), &all_receipts(&b.l), d(1, 20));
    assert!(!codes(&before).contains(&"close_split_drifts"));
}

fn all_receipts(l: &Ledger) -> BTreeSet<EntryId> {
    l.posted_entries().map(|e| e.id()).collect()
}

// ---------------------------------------------------------------- evidence

fn set(l: &mut Ledger, key: &str, value: serde_json::Value) {
    let src = Source {
        decided_by: "Owner".into(),
        decided_on: d(11, 9),
        citation: None,
        note: None,
    };
    let changes = [Change {
        key: key.into(),
        value: Some(value),
    }];
    let made = l.check_settings(&changes, &src).unwrap();
    l.apply_settings(&made, &src, 1).unwrap();
}

/// BS-entry-5, INVARIANT 3 of the audit (only entries on or before the
/// date count): an uncovered entry dated after `through` is not yet a
/// finding, and one with a receipt never is.
#[test]
fn evidence_is_missing_only_through_the_date_and_only_without_a_receipt() {
    let mut b = books();
    let early = post(&mut b.l, d(2, 1), b.expense, b.cash, "40.00");
    let late = post(&mut b.l, d(6, 1), b.expense, b.cash, "40.00");
    let settings = Settings::default();
    let none = BTreeSet::new();

    let f = run(&b.l, &settings, &none, d(3, 31));
    assert_eq!(codes(&f), ["evidence_missing"]);
    assert_eq!(f[0].subject, Subject::Entry(early));
    assert!(
        f[0].message
            .contains("strictly greater than the 25.00 threshold")
    );

    let f = run(&b.l, &settings, &none, d(12, 31));
    assert_eq!(codes(&f), ["evidence_missing", "evidence_missing"]);

    let have = BTreeSet::from([early]);
    let f = run(&b.l, &settings, &have, d(12, 31));
    assert_eq!(f.len(), 1);
    assert_eq!(f[0].subject, Subject::Entry(late));
}

/// D-47 as an audit reads it: a reversal the policy WOULD cover, with no
/// receipt of its own, is an Info finding (its original's evidence stands
/// for it) and is not a failure; under a policy that would not cover it
/// (a threshold above its debits) it says nothing.
#[test]
fn a_reversal_without_its_own_receipt_is_info_and_only_when_the_policy_would_cover_it() {
    let mut b = books();
    let big = post(&mut b.l, d(2, 1), b.expense, b.cash, "500.00");
    let rev = b.l.reverse_entry(big, d(2, 2), "Returned").unwrap();
    let settings = Settings::default();
    // The original has its receipt; the reversal has none.
    let have = BTreeSet::from([big]);
    let f = run(&b.l, &settings, &have, d(12, 31));
    assert_eq!(codes(&f), ["legacy_reversal_exempt"]);
    assert_eq!(f[0].severity, Severity::Info);
    assert_eq!(f[0].subject, Subject::Entry(rev));
    assert_eq!(
        f[0].message,
        format!("entry {rev} reverses entry {big}: its evidence is the original's")
    );
    // With a receipt of its own, nothing.
    let both = BTreeSet::from([big, rev]);
    assert!(run(&b.l, &settings, &both, d(12, 31)).is_empty());

    // A policy whose threshold is above the reversal's debits would not
    // have covered it: nothing to say about the reversal (nor the original).
    set(&mut b.l, "policy.evidence.threshold", json!("1000.00"));
    assert!(run(&b.l, b.l.settings(), &have, d(12, 31)).is_empty());
}

/// BS-ledger-13 SCENARIO 2: a draft dated on or before the date is exactly
/// `draft_open`; one dated after is not mentioned.
#[test]
fn a_draft_is_open_only_through_the_date() {
    let mut b = books();
    let e1 = b.l.create_entry(d(3, 3), "March draft");
    let e2 = b.l.create_entry(d(9, 9), "September draft");
    for e in [e1, e2] {
        b.l.add_line(e, b.expense, Side::Debit, usd("1.00"))
            .unwrap();
        b.l.add_line(e, b.cash, Side::Credit, usd("1.00")).unwrap();
    }
    let f = run(&b.l, &Settings::default(), &BTreeSet::new(), d(4, 30));
    assert_eq!(codes(&f), ["draft_open"]);
    assert_eq!(f[0].subject, Subject::Entry(e1));
    assert_eq!(
        f[0].message,
        "draft 1 (2026-03-03) \"March draft\" is not posted"
    );
}

/// BS-ledger-13: the facts the caller reads from around the books are
/// passed through as findings: a settings problem as it was worded, a
/// vendor paid in a way that could need a 1099 by name and amount.
#[test]
fn the_callers_facts_become_findings_in_order() {
    let b = books();
    let settings = Settings::default();
    let receipts = BTreeSet::new();
    let problems = ["books.cash_accounts names 1099, which the chart lacks".to_string()];
    let undecided = [("Dana Co".to_string(), dec("1250.00"))];
    let f = audit::statements(
        &b.l,
        &AuditInput {
            settings: &settings,
            receipts: &receipts,
            cash_flow: None,
            settings_problems: &problems,
            undecided_1099: &undecided,
        },
        d(12, 31),
    );
    assert_eq!(codes(&f), ["settings_problem", "vendor_1099_undecided"]);
    assert_eq!(f[0].message, problems[0]);
    assert_eq!(f[1].subject, Subject::Vendor("Dana Co".into()));
    assert_eq!(
        f[1].message,
        "Dana Co was paid 1250.00 in a way that could need a 1099 and has no decision"
    );
}

// ---------------------------------------------------------------- the bar (D-9)

/// D-9: a scope's and an effort's words round-trip; case and padding are
/// forgiven; anything else is not a scope or an effort.
#[test]
fn scopes_and_efforts_round_trip_their_words() {
    for s in [Scope::Month, Scope::Annual, Scope::CpaHandoff, Scope::Full] {
        assert_eq!(Scope::parse(s.key()), Some(s));
        assert_eq!(
            Scope::parse(&format!("  {}\n", s.key().to_uppercase())),
            Some(s)
        );
    }
    assert_eq!(Scope::parse("quarter"), None);
    assert_eq!(Scope::parse(""), None);
    for e in [
        Effort::Low,
        Effort::Medium,
        Effort::High,
        Effort::Xhigh,
        Effort::Max,
    ] {
        assert_eq!(Effort::parse(e.key()), Some(e));
        assert_eq!(
            Effort::parse(&format!(" {} ", e.key().to_uppercase())),
            Some(e)
        );
    }
    assert_eq!(Effort::parse("extreme"), None);
    assert!(Effort::Low < Effort::Medium && Effort::Xhigh < Effort::Max);
}

/// D-9: only a month's audit is held to the effort alone; the other scopes
/// are held to the whole bar. Only the CPA hand-off and a full audit are
/// ones the CPA package may rest on.
#[test]
fn scopes_say_which_bar_they_carry_and_which_serve_the_hand_off() {
    let table = [
        (Scope::Month, false, false),
        (Scope::Annual, true, false),
        (Scope::CpaHandoff, true, true),
        (Scope::Full, true, true),
    ];
    for (scope, full, handoff) in table {
        assert_eq!(scope.full_bar(), full, "{scope:?}");
        assert_eq!(scope.serves_handoff(), handoff, "{scope:?}");
    }
}

/// D-9: the bar's setting text. Words in any order, split by spaces or
/// commas, long and short spellings, an omitted word keeps its default; a
/// word that is not `key=value`, a value out of range and an unknown key
/// are each refused with the offending word named.
#[test]
fn the_bar_setting_parses_what_it_names_and_refuses_the_rest() {
    let default = AttestationBar::default();
    assert_eq!(AttestationBar::parse("").unwrap(), default);
    assert_eq!(AttestationBar::parse(" , ,").unwrap(), default);
    let bar =
        AttestationBar::parse("reasoning_effort=max,verifier_agents=64 adversarial=false").unwrap();
    assert_eq!(bar.describe(), "effort=max verifiers=64 adversarial=false");
    assert_eq!(
        AttestationBar::parse("verifiers=1").unwrap().describe(),
        "effort=high verifiers=1 adversarial=true"
    );

    let refused = |text: &str, fragment: &str| {
        let e = AttestationBar::parse(text).unwrap_err();
        assert!(e.contains(fragment), "{text:?}: {e}");
    };
    refused("effort", "expected key=value");
    refused("effort=wild", "is not low, medium, high, xhigh or max");
    refused("verifiers=0", "not a count from 1 to 64");
    refused("verifiers=65", "not a count from 1 to 64");
    refused("verifiers=two", "not a count from 1 to 64");
    refused("verifiers=-1", "not a count from 1 to 64");
    refused("adversarial=maybe", "is not true or false");
    refused("adversarial=True", "is not true or false");
    refused("strict=yes", "is not a part of the bar");
}

/// D-9: what counts, at the edges. The effort is a floor (equal counts),
/// the verifiers a floor (equal counts), the adversarial check is needed
/// only when the bar asks for it, and a month's attestation needs the
/// effort alone however few verifiers it names.
#[test]
fn the_bar_counts_at_its_edges_and_only_asks_for_what_it_sets() {
    let method = |effort, adversarial, agents| Method {
        model: Some("claude".into()),
        reasoning_effort: effort,
        adversarial,
        verifier_agents: agents,
    };
    let bar = AttestationBar::default();
    let at_edge = method(Some(Effort::High), Some(true), Some(2));
    assert!(bar.counts(&at_edge, true, false));
    assert!(bar.counts(&at_edge, false, false));
    // Month: effort alone.
    assert!(bar.counts(&method(Some(Effort::High), None, None), false, false));
    assert!(!bar.counts(
        &method(Some(Effort::Medium), Some(true), Some(9)),
        false,
        false
    ));
    // A bar that does not ask for the adversarial check does not need it,
    // but still needs the verifiers.
    let open = AttestationBar::parse("adversarial=false").unwrap();
    assert!(open.counts(&method(Some(Effort::High), None, Some(2)), true, false));
    assert!(!open.counts(&method(Some(Effort::High), None, Some(1)), true, false));
    // A disagreeing claim never counts, whatever else is true.
    assert!(!open.counts(&at_edge, false, true));
}
