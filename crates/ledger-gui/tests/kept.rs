//! The Company page shows how the books are kept: live conventions (not
//! retired ones), standing decisions (not superseded ones), and open items
//! (not resolved ones), each with who decided it and when; an open item
//! past due says so in words. Books that keep their settings but hold no
//! company facts say where the facts will come from.

use chrono::NaiveDate;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ledger_core::Currency;
use ledger_core::conventions::{ConventionKind, NewConvention, NewDecision, NewOpenItem};
use ledger_core::settings::{Change, Source};
use ledger_gui::nav::Route;
use ledger_gui::{App, data, fmt};
use ledger_store::Store;
use std::path::PathBuf;

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        let d = std::env::temp_dir().join(format!("ledger-gui-kept-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        TempDir(d)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn today() -> NaiveDate {
    d(2026, 10, 17)
}

fn src(by: &str, on: NaiveDate) -> Source {
    Source {
        decided_by: by.into(),
        decided_on: on,
        citation: None,
        note: None,
    }
}

fn note(id: &str, title: &str, body: &str, on: NaiveDate) -> NewConvention {
    NewConvention {
        id: id.into(),
        kind: ConventionKind::Policy,
        title: title.into(),
        body_md: body.into(),
        structured: None,
        source: src("Tobin Harlow", on),
    }
}

fn item(title: &str, due: Option<NaiveDate>) -> NewOpenItem {
    NewOpenItem {
        title: title.into(),
        detail_md: None,
        due,
        source: src("Tobin Harlow", d(2026, 10, 16)),
    }
}

/// Renders the Company page of the books in `dir`.
fn company(dir: &TempDir) -> Harness<'static, App> {
    let books = data::load_once(&ledger_gui::Source {
        data_dir: dir.0.join("books-data"),
        today: Some(today()),
    })
    .unwrap();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1440.0, 2400.0))
        .build_eframe(|cc| App::with_books(&cc.egui_ctx, books, false, Route::Company));
    h.run_steps(3);
    h
}

fn shows(h: &Harness<'_, App>, text: &str) -> bool {
    h.query_all_by_label_contains(text).next().is_some()
}

#[test]
fn the_company_page_shows_how_the_books_are_kept() {
    let dir = TempDir::new("cards");
    let mut s = Store::create(&dir.0.join("books-data"), Currency::USD).unwrap();
    s.set_convention(&note(
        "joint-money",
        "Joint money",
        "Owner funds are 50/50.\nUnless Tobin says otherwise.",
        d(2026, 10, 11),
    ))
    .unwrap();
    s.set_convention(&note(
        "old-way",
        "The old way",
        "Gone now.",
        d(2026, 10, 12),
    ))
    .unwrap();
    s.retire_convention("old-way", "replaced", &src("Tobin Harlow", d(2026, 10, 14)))
        .unwrap();
    let first = s
        .record_decision(&NewDecision {
            subject: "Materiality, first cut".into(),
            decision_md: "Expense up to $2,500.".into(),
            decided_by: vec!["Tobin Harlow".into()],
            decided_on: d(2026, 10, 11),
            evidence_attachment: None,
            supersedes: None,
            citation: None,
            projects: vec![],
        })
        .unwrap();
    s.record_decision(&NewDecision {
        subject: "Materiality policy".into(),
        decision_md: "Tangible property up to $2,500; prepaids by term.".into(),
        decided_by: vec!["Tobin Harlow".into(), "Morgan Ellis".into()],
        decided_on: d(2026, 10, 16),
        evidence_attachment: None,
        supersedes: Some(first),
        citation: None,
        projects: vec![],
    })
    .unwrap();
    s.raise_open_item(&item("Mail the IRS letter", Some(d(2026, 10, 16))))
        .unwrap();
    s.raise_open_item(&item("Close October", Some(today())))
        .unwrap();
    s.raise_open_item(&item("Ask about the domains", None))
        .unwrap();
    let done = s
        .raise_open_item(&item("File the EIN letter", None))
        .unwrap();
    s.resolve_open_item(done, "filed", &src("Tobin Harlow", d(2026, 10, 17)))
        .unwrap();
    drop(s);

    let h = company(&dir);
    // Live conventions, their kind and summary (the first line), and who
    // decided them; never a retired one.
    assert!(shows(&h, "How the books are kept"));
    assert!(shows(&h, "Joint money"));
    assert!(shows(&h, "policy"));
    assert!(shows(&h, "Owner funds are 50/50."));
    assert!(
        !shows(&h, "Unless Tobin says otherwise"),
        "a summary is its first line"
    );
    assert!(shows(
        &h,
        &format!("Tobin Harlow · {}", fmt::date(d(2026, 10, 11)))
    ));
    assert!(
        !shows(&h, "The old way"),
        "a retired convention is not shown"
    );
    // The standing decision, by both members; never the one it replaced.
    assert!(shows(&h, "Decisions"));
    assert!(shows(&h, "Materiality policy"));
    assert!(shows(
        &h,
        "Tangible property up to $2,500; prepaids by term."
    ));
    assert!(shows(
        &h,
        &format!(
            "Tobin Harlow and Morgan Ellis · {}",
            fmt::date(d(2026, 10, 16))
        )
    ));
    assert!(
        !shows(&h, "Materiality, first cut"),
        "a superseded decision is not shown"
    );
    // Open items: late in words, due today is not late, no due no date;
    // never a resolved one.
    assert!(shows(&h, "Open items"));
    assert!(shows(&h, "Mail the IRS letter"));
    assert!(shows(
        &h,
        &format!("overdue since {}", fmt::date(d(2026, 10, 16)))
    ));
    assert!(shows(&h, "Close October"));
    assert!(
        h.query_by_label(&format!("due {}", fmt::date(today())))
            .is_some()
    );
    assert!(!shows(&h, &format!("overdue since {}", fmt::date(today()))));
    assert!(shows(&h, "Ask about the domains"));
    assert!(
        !shows(&h, "File the EIN letter"),
        "a resolved item is not shown"
    );
}

#[test]
fn nothing_kept_shows_no_cards() {
    let dir = TempDir::new("none");
    let mut s = Store::create(&dir.0.join("books-data"), Currency::USD).unwrap();
    s.set_convention(&note(
        "old-way",
        "The old way",
        "Gone now.",
        d(2026, 10, 12),
    ))
    .unwrap();
    s.retire_convention("old-way", "replaced", &src("Tobin Harlow", d(2026, 10, 14)))
        .unwrap();
    let only = s.raise_open_item(&item("Done already", None)).unwrap();
    s.resolve_open_item(only, "done", &src("Tobin Harlow", d(2026, 10, 17)))
        .unwrap();
    drop(s);
    let h = company(&dir);
    for card in ["How the books are kept", "Decisions", "Open items"] {
        assert!(!shows(&h, card), "{card} shown with nothing in it");
    }
}

#[test]
fn books_that_keep_their_settings_say_where_facts_come_from() {
    let dir = TempDir::new("facts");
    let mut s = Store::create(&dir.0.join("books-data"), Currency::USD).unwrap();
    // Journaled, with no company fact yet.
    s.change_settings(
        &[Change {
            key: "books.asset_tag_prefix".into(),
            value: Some(serde_json::json!("FL")),
        }],
        &src("Tobin Harlow", today()),
    )
    .unwrap();
    drop(s);
    let h = company(&dir);
    assert!(shows(
        &h,
        "No company facts set yet: the agent records them as you give them."
    ));
    drop(h);
    let mut s = Store::open(&dir.0.join("books-data")).unwrap();
    s.change_settings(
        &[Change {
            key: "company.name".into(),
            value: Some(serde_json::json!("Fernhill Labs LLC")),
        }],
        &src("Tobin Harlow", today()),
    )
    .unwrap();
    drop(s);
    let h = company(&dir);
    assert!(!shows(&h, "No company facts"), "the facts are there");
    assert!(shows(&h, "Fernhill Labs LLC"));
    assert!(shows(&h, &format!("Tobin Harlow · {}", fmt::date(today()))));
}
