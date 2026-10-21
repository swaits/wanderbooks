//! Projects (rules.md B-37): a class with its facts; its figures are the
//! revenue and expense lines tagged to it.
//!
//! ```text
//! GOAL: a project's record holds only what its rules allow, and its
//!       figures are exactly its tagged lines, from one function.
//! SOURCE: docs/spec/rules.md B-37 (job costing; ASC 450-30-25-1 and
//!         340-40-25-5 for on-spec work), B-11 (closes left out).
//! ```

use chrono::NaiveDate;
use ledger_core::conventions::NewDecision;
use ledger_core::{
    AccountType, ClassId, Currency, Ledger, LedgerError, Money, ProjectFacts, ProjectKind,
    ProjectStatus, Side, project_entries, project_figures, project_to_date,
};
use rust_decimal::Decimal;

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn cents(c: i64) -> Decimal {
    Decimal::new(c, 2)
}

fn usd(c: i64) -> Money {
    Money::new(cents(c), Currency::USD)
}

/// Cash 1000, equipment 1200, revenue 4000, contractors 6000, rent 6200.
struct Books {
    l: Ledger,
    cash: u64,
    equipment: u64,
    revenue: u64,
    contractors: u64,
    rent: u64,
}

fn books() -> Books {
    let mut l = Ledger::new(Currency::USD);
    let mut add = |code: &str, t| l.add_account(code, code, t, None, false).unwrap();
    let cash = add("1000", AccountType::Asset);
    let equipment = add("1200", AccountType::Asset);
    let revenue = add("4000", AccountType::Revenue);
    let contractors = add("6000", AccountType::Expense);
    let rent = add("6200", AccountType::Expense);
    Books {
        l,
        cash,
        equipment,
        revenue,
        contractors,
        rent,
    }
}

impl Books {
    /// Post `dr`/`cr` for `c` cents on `date`, tagging the debit line
    /// (`on_debit`) or the credit line with `class`.
    fn post(
        &mut self,
        date: NaiveDate,
        dr: u64,
        cr: u64,
        c: i64,
        class: Option<(ClassId, bool)>,
    ) -> u64 {
        let e = self.l.create_entry(date, "e");
        self.l.add_line(e, dr, Side::Debit, usd(c)).unwrap();
        self.l.add_line(e, cr, Side::Credit, usd(c)).unwrap();
        if let Some((k, on_debit)) = class {
            self.l
                .set_line_class(e, if on_debit { 0 } else { 1 }, Some(k))
                .unwrap();
        }
        self.l.post_entry(e).unwrap();
        e
    }
}

fn on_spec() -> ProjectFacts {
    ProjectFacts {
        kind: Some(ProjectKind::OnSpec),
        ..Default::default()
    }
}

#[test]
fn project_facts_default_for_a_bare_class() {
    let mut b = books();
    let c = b.l.add_class("Harbor").unwrap();
    let f = b.l.project_facts(c).unwrap();
    assert_eq!(f, ProjectFacts::default());
    assert_eq!(f.status, ProjectStatus::Active);
    assert_eq!(f.kind, None);
    assert!(b.l.project_facts(99).is_none());
}

#[test]
fn set_project_checks_every_field() {
    let mut b = books();
    let c = b.l.add_class("Harbor").unwrap();
    b.l.add_class("Other").unwrap();
    let ok = ProjectFacts::default();
    let refused =
        |l: &Ledger, name: &str, f: ProjectFacts| l.check_set_project(c, name, &f).is_err();
    // The class exists.
    assert_eq!(
        b.l.check_set_project(99, "x", &ok),
        Err(LedgerError::UnknownClass(99))
    );
    // Names: not blank, one line, not another's (ignoring capitals), the
    // project's own name fine.
    assert!(refused(&b.l, "  ", ok.clone()));
    assert!(refused(&b.l, &"x".repeat(201), ok.clone()));
    assert!(refused(&b.l, "two\nlines", ok.clone()));
    assert_eq!(
        b.l.check_set_project(c, " other ", &ok),
        Err(LedgerError::DuplicateClassName("other".into()))
    );
    assert_eq!(b.l.check_set_project(c, "HARBOR", &ok).unwrap().0, "HARBOR");
    // No full TIN in any text.
    for f in [
        ProjectFacts {
            description: Some("EIN 12-3456789".into()),
            ..ok.clone()
        },
        ProjectFacts {
            notes: Some("SSN 123-45-6789".into()),
            ..ok.clone()
        },
    ] {
        assert!(refused(&b.l, "Harbor", f));
    }
    assert!(refused(&b.l, "123-45-6789", ok.clone()));
    // ended_on only when over, never before started_on.
    let ended = |status, start: Option<NaiveDate>, end| ProjectFacts {
        status,
        started_on: start,
        ended_on: Some(end),
        ..Default::default()
    };
    assert!(refused(
        &b.l,
        "Harbor",
        ended(ProjectStatus::Active, None, d(2026, 1, 1))
    ));
    assert!(refused(
        &b.l,
        "Harbor",
        ended(ProjectStatus::Paused, None, d(2026, 1, 1))
    ));
    assert!(refused(
        &b.l,
        "Harbor",
        ended(ProjectStatus::Finished, Some(d(2026, 2, 1)), d(2026, 1, 31))
    ));
    assert!(!refused(
        &b.l,
        "Harbor",
        ended(ProjectStatus::Finished, Some(d(2026, 2, 1)), d(2026, 2, 1))
    ));
    assert!(!refused(
        &b.l,
        "Harbor",
        ended(ProjectStatus::Abandoned, None, d(2026, 2, 1))
    ));
    // Links: at most eight, one address each, none twice.
    let links = |v: Vec<&str>| ProjectFacts {
        links: v.into_iter().map(String::from).collect(),
        ..Default::default()
    };
    assert!(refused(&b.l, "Harbor", links(vec!["https://a.example"; 2])));
    assert!(refused(&b.l, "Harbor", links(vec!["a b"])));
    assert!(refused(&b.l, "Harbor", links(vec![""])));
    let nine: Vec<String> = (0..9).map(|i| format!("https://{i}.example")).collect();
    assert!(refused(
        &b.l,
        "Harbor",
        links(nine.iter().map(String::as_str).collect())
    ));
    assert!(!refused(
        &b.l,
        "Harbor",
        links(nine[..8].iter().map(String::as_str).collect())
    ));
    // The party is in the directory.
    assert!(refused(
        &b.l,
        "Harbor",
        ProjectFacts {
            party: Some(7),
            ..Default::default()
        }
    ));
    let p = b.l.add_party("Harbor Co", &[]).unwrap();
    let (name, facts) =
        b.l.check_set_project(
            c,
            "  Harbor  ",
            &ProjectFacts {
                party: Some(p),
                description: Some("  the app  ".into()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(
        (name.as_str(), facts.description.as_deref()),
        ("Harbor", Some("the app"))
    );
}

#[test]
fn set_project_renames_the_class_and_moves_no_figure() {
    let mut b = books();
    let c = b.l.add_class("Harbor").unwrap();
    b.post(d(2026, 3, 1), b.cash, b.revenue, 10_000, Some((c, false)));
    let before = project_to_date(&b.l, c, d(2026, 12, 31)).unwrap();
    b.l.set_project(c, "Harbor app", on_spec()).unwrap();
    assert_eq!(b.l.get_class(c).unwrap().name, "Harbor app");
    let after = project_to_date(&b.l, c, d(2026, 12, 31)).unwrap();
    assert_eq!(
        (before.revenue, before.expenses),
        (after.revenue, after.expenses)
    );
    assert_eq!(
        b.l.project_facts(c).unwrap().kind,
        Some(ProjectKind::OnSpec)
    );
}

#[test]
fn add_class_refuses_a_duplicate_name() {
    let mut b = books();
    b.l.add_class("Harbor").unwrap();
    let next = b.l.next_class_id();
    assert_eq!(
        b.l.add_class("  harbor "),
        Err(LedgerError::DuplicateClassName("harbor".into()))
    );
    assert_eq!(b.l.next_class_id(), next, "a refusal allocates nothing");
    // Trimmed as kept.
    let c = b.l.add_class("  Dock  ").unwrap();
    assert_eq!(b.l.get_class(c).unwrap().name, "Dock");
}

#[test]
fn names_compare_one_way_for_the_duplicate_check_and_the_lookup() {
    let mut b = books();
    let c = b.l.add_class("Éclair").unwrap();
    // The duplicate check and the lookup fold the same: capitals in any
    // script, surrounding space.
    assert_eq!(
        b.l.add_class("éCLAIR"),
        Err(LedgerError::DuplicateClassName("éCLAIR".into()))
    );
    for name in ["éclair", "ÉCLAIR", " Éclair ", "\téclair"] {
        assert_eq!(
            b.l.find_class_by_name(name).map(|x| x.id),
            Some(c),
            "{name:?}"
        );
    }
    assert!(b.l.find_class_by_name("eclair").is_none());
}

#[test]
fn old_books_duplicates_resolve_to_the_lowest_id_and_change_freely() {
    let mut l = Ledger::new(Currency::USD);
    // Replay never checks names, so an earlier version's duplicates load.
    l.replay_class(1, "Alpha".into()).unwrap();
    l.replay_class(2, "alpha".into()).unwrap();
    l.replay_class(3, "Harbor".into()).unwrap();
    for _ in 0..20 {
        assert_eq!(l.find_class_by_name("ALPHA").map(|c| c.id), Some(1));
    }
    let paused = ProjectFacts {
        status: ProjectStatus::Paused,
        ..Default::default()
    };
    // Each takes a fact-only change under its own name.
    assert_eq!(l.check_set_project(1, "Alpha", &paused).unwrap().0, "Alpha");
    assert_eq!(l.check_set_project(2, "alpha", &paused).unwrap().0, "alpha");
    // Its own name up to capitals and space is still its own.
    assert_eq!(
        l.check_set_project(2, " Alpha ", &paused).unwrap().0,
        "Alpha"
    );
    // A rename onto another project's name is still refused.
    assert_eq!(
        l.check_set_project(2, "harbor", &paused),
        Err(LedgerError::DuplicateClassName("harbor".into()))
    );
}

#[test]
fn an_unchanged_name_is_kept_exactly_and_not_checked_again() {
    let mut l = Ledger::new(Currency::USD);
    // An earlier version kept names untrimmed, and wrote rules since
    // tightened (a nine-digit job number) without checking them.
    l.replay_class(1, "Retail ".into()).unwrap();
    l.replay_class(2, "Job 123456789".into()).unwrap();
    let ok = ProjectFacts::default();
    assert_eq!(l.check_set_project(1, "Retail ", &ok).unwrap().0, "Retail ");
    assert_eq!(
        l.check_set_project(2, "Job 123456789", &ok).unwrap().0,
        "Job 123456789"
    );
    // A new name gets every rule.
    assert!(l.check_set_project(2, "Job 223456789", &ok).is_err());
    assert_eq!(l.check_set_project(1, "Retail", &ok).unwrap().0, "Retail");
}

#[test]
fn a_name_of_digits_only_is_refused() {
    let mut b = books();
    let c = b.l.add_class("Harbor").unwrap();
    for name in ["1", " 42 ", "0007"] {
        assert!(
            matches!(b.l.add_class(name), Err(LedgerError::InvalidProject(_))),
            "{name:?}"
        );
        assert!(
            b.l.check_set_project(c, name, &ProjectFacts::default())
                .is_err(),
            "{name:?}"
        );
    }
    assert!(b.l.add_class("Job 42").is_ok());
}

#[test]
fn a_link_is_read_as_a_web_address_for_a_full_tin() {
    let mut b = books();
    let c = b.l.add_class("Harbor").unwrap();
    let link = |l: &str| {
        b.l.check_set_project(
            c,
            "Harbor",
            &ProjectFacts {
                links: vec![l.to_string()],
                ..Default::default()
            },
        )
    };
    // An id in a path or query is no TIN.
    for ok in [
        "https://apps.apple.com/us/app/harbor/id284882215",
        "https://store.example/listing/284882215",
        "https://store.example/app?id=284882215&x=1",
        "https://drive.example/file/d/284882215abc/view",
    ] {
        assert!(link(ok).is_ok(), "{ok}");
    }
    // A printed SSN or EIN, or nine digits standing alone, is.
    for bad in [
        "https://files.example/123-45-6789",
        "https://files.example/ein-12-3456789.pdf",
        "https://files.example/a.123456789.pdf",
        "123456789",
    ] {
        assert!(
            matches!(link(bad), Err(LedgerError::InvalidProject(_))),
            "{bad}"
        );
    }
}

#[test]
fn replay_set_project_takes_what_the_text_rules_would_refuse() {
    let mut b = books();
    let c = b.l.add_class("Harbor").unwrap();
    // An end with no end status, and a nine-link list: what a later rule
    // might refuse, an earlier version may have journaled.
    let odd = ProjectFacts {
        ended_on: Some(d(2026, 1, 1)),
        links: (0..9).map(|i| format!("l{i}")).collect(),
        ..Default::default()
    };
    assert!(b.l.check_set_project(c, "Harbor", &odd).is_err());
    b.l.replay_set_project(c, "Harbor".into(), odd.clone())
        .unwrap();
    assert_eq!(b.l.project_facts(c), Some(odd));
    // What replay can't do without is still refused.
    assert!(
        b.l.replay_set_project(9, "x".into(), ProjectFacts::default())
            .is_err()
    );
    assert!(
        b.l.replay_set_project(
            c,
            "x".into(),
            ProjectFacts {
                party: Some(42),
                ..Default::default()
            }
        )
        .is_err()
    );
}

#[test]
fn project_figures_are_its_tagged_lines() {
    let mut b = books();
    let alpha = b.l.add_class("Alpha").unwrap();
    let beta = b.l.add_class("Beta").unwrap();
    b.post(
        d(2026, 2, 3),
        b.cash,
        b.revenue,
        500_000,
        Some((alpha, false)),
    );
    b.post(
        d(2026, 2, 10),
        b.contractors,
        b.cash,
        180_000,
        Some((alpha, true)),
    );
    b.post(
        d(2026, 3, 10),
        b.contractors,
        b.cash,
        8_000,
        Some((beta, true)),
    );
    b.post(d(2026, 3, 28), b.rent, b.cash, 150_000, None);
    let f = project_figures(&b.l, alpha, d(2026, 1, 1), d(2026, 3, 31)).unwrap();
    assert_eq!(f.period.total_revenue, cents(500_000));
    assert_eq!(f.period.total_expenses, cents(180_000));
    assert_eq!(f.period.revenue.len(), 1);
    assert_eq!(f.period.expenses[0].code, "6000");
    assert_eq!(f.months.len(), 3);
    assert_eq!(f.months[1].net(), cents(320_000));
    assert!(
        f.months[2].net().is_zero(),
        "March is Beta's and the rent's"
    );
    let total: Decimal = f.months.iter().map(|m| m.net()).sum();
    assert_eq!(total, f.period.net_income());
    assert_eq!(
        project_figures(&b.l, 99, d(2026, 1, 1), d(2026, 3, 31)).err(),
        Some(LedgerError::UnknownClass(99))
    );
}

#[test]
fn project_to_date_ignores_the_start_and_the_projects_dates() {
    let mut b = books();
    let c = b.l.add_class("Harbor").unwrap();
    b.l.set_project(
        c,
        "Harbor",
        ProjectFacts {
            started_on: Some(d(2026, 6, 1)),
            ..Default::default()
        },
    )
    .unwrap();
    // A cost before kickoff is still the project's.
    b.post(
        d(2025, 11, 5),
        b.contractors,
        b.cash,
        20_000,
        Some((c, true)),
    );
    b.post(
        d(2026, 7, 5),
        b.contractors,
        b.cash,
        10_000,
        Some((c, true)),
    );
    let f = project_figures(&b.l, c, d(2026, 7, 1), d(2026, 7, 31)).unwrap();
    assert_eq!(f.period.total_expenses, cents(10_000));
    assert_eq!(f.to_date.expenses, cents(30_000));
    assert_eq!(f.to_date.through, d(2026, 7, 31));
    assert_eq!(
        project_to_date(&b.l, c, d(2025, 12, 31)).unwrap().expenses,
        cents(20_000)
    );
}

#[test]
fn a_tagged_balance_sheet_line_counts_nothing() {
    let mut b = books();
    let c = b.l.add_class("Beta").unwrap();
    let e = b.post(d(2026, 3, 5), b.equipment, b.cash, 90_000, Some((c, true)));
    let t = project_to_date(&b.l, c, d(2026, 12, 31)).unwrap();
    assert_eq!((t.revenue, t.expenses), (Decimal::ZERO, Decimal::ZERO));
    // Listed among its entries, with nothing of the project's.
    assert_eq!(project_entries(&b.l, c), vec![(e, Decimal::ZERO)]);
}

#[test]
fn closing_entries_are_left_out_of_project_figures() {
    let mut b = books();
    let c = b.l.add_class("Alpha").unwrap();
    b.post(d(2025, 6, 1), b.cash, b.revenue, 40_000, Some((c, false)));
    let p =
        b.l.create_period("2025", d(2025, 1, 1), d(2025, 12, 31))
            .unwrap();
    let close = b.l.close_period(p).unwrap();
    assert!(close.is_some());
    let f = project_figures(&b.l, c, d(2025, 1, 1), d(2025, 12, 31)).unwrap();
    assert_eq!(f.period.total_revenue, cents(40_000));
    assert_eq!(f.to_date.revenue, cents(40_000));
}

#[test]
fn a_reversal_nets_a_project_to_zero() {
    let mut b = books();
    let c = b.l.add_class("Alpha").unwrap();
    let e = b.post(
        d(2026, 3, 12),
        b.contractors,
        b.cash,
        180_000,
        Some((c, true)),
    );
    let r =
        b.l.reverse_entry(e, d(2026, 3, 13), "billed twice")
            .unwrap();
    let t = project_to_date(&b.l, c, d(2026, 12, 31)).unwrap();
    assert!(t.expenses.is_zero());
    let entries = project_entries(&b.l, c);
    assert_eq!(entries, vec![(r, cents(180_000)), (e, cents(-180_000))]);
}

#[test]
fn unrecovered_cost_only_for_on_spec_and_never_negative() {
    let mut b = books();
    let spec = b.l.add_class("Harbor app").unwrap();
    b.l.set_project(spec, "Harbor app", on_spec()).unwrap();
    let client = b.l.add_class("Client").unwrap();
    b.post(
        d(2026, 4, 2),
        b.contractors,
        b.cash,
        146_000,
        Some((spec, true)),
    );
    b.post(
        d(2026, 4, 2),
        b.contractors,
        b.cash,
        5_000,
        Some((client, true)),
    );
    let at = |b: &Books, k| project_to_date(&b.l, k, d(2026, 12, 31)).unwrap();
    assert_eq!(at(&b, spec).unrecovered_cost, Some(cents(146_000)));
    assert_eq!(at(&b, client).unrecovered_cost, None);
    b.post(
        d(2026, 6, 15),
        b.cash,
        b.revenue,
        100_000,
        Some((spec, false)),
    );
    assert_eq!(at(&b, spec).unrecovered_cost, Some(cents(46_000)));
    b.post(
        d(2026, 7, 1),
        b.cash,
        b.revenue,
        70_000,
        Some((spec, false)),
    );
    assert_eq!(at(&b, spec).unrecovered_cost, Some(Decimal::ZERO));
    assert_eq!(at(&b, spec).net(), cents(24_000));
}

#[test]
fn projects_of_party_lists_them_by_name() {
    let mut b = books();
    let p = b.l.add_party("Riley Chen", &[]).unwrap();
    let with = |l: &mut Ledger, name: &str| {
        let c = l.add_class(name).unwrap();
        l.set_project(
            c,
            name,
            ProjectFacts {
                party: Some(p),
                ..Default::default()
            },
        )
        .unwrap();
        c
    };
    let zeta = with(&mut b.l, "Zeta");
    let alpha = with(&mut b.l, "alpha");
    b.l.add_class("Unrelated").unwrap();
    assert_eq!(b.l.projects_of_party(p), vec![alpha, zeta]);
    assert!(b.l.projects_of_party(p + 1).is_empty());
}

#[test]
fn check_decision_refuses_an_unknown_or_repeated_project() {
    let mut b = books();
    let c = b.l.add_class("Harbor").unwrap();
    let decision = |projects| NewDecision {
        subject: "Harbor".into(),
        decision_md: "Build it.".into(),
        decided_by: vec!["Tobin Harlow".into()],
        decided_on: d(2026, 4, 1),
        evidence_attachment: None,
        supersedes: None,
        citation: None,
        projects,
    };
    assert_eq!(
        b.l.check_decision(&decision(vec![9])).err(),
        Some(LedgerError::UnknownClass(9))
    );
    assert!(b.l.check_decision(&decision(vec![c, c])).is_err());
    assert_eq!(
        b.l.check_decision(&decision(vec![c])).unwrap().1.projects,
        vec![c]
    );
}

#[test]
fn differs_from_names_projects() {
    let mut a = books();
    let c = a.l.add_class("Harbor").unwrap();
    let before = a.l.clone();
    a.l.set_project(c, "Harbor", on_spec()).unwrap();
    assert_eq!(a.l.differs_from(&before), vec!["projects"]);
}
