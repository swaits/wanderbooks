//! The breadcrumb links back up: every crumb but the page you're on goes
//! where it names.
//!
//! ```text
//! GOAL: from Parties › Stephen Waits, clicking "Parties" goes back to the
//!       directory; deeper trails link each level.
//! SOURCE: DECISION (Stephen, 2026-10-16: "I should be able to click
//!         parties and it takes me back there ... when it goes deeper").
//! INVARIANTS: palette::crumbs 1-3, and
//!   4. Clicking a crumb goes to its route (a history step, so Back
//!      returns).
//! ```

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ledger_gui::nav::{ReportKind, Route, Section};
use ledger_gui::palette::{crumbs, describe};
use ledger_gui::{App, Source, data};

fn demo(tag: &str) -> Source {
    let dir = std::env::temp_dir().join(format!("ledger-gui-crumbs-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let today = chrono::NaiveDate::from_ymd_opt(2026, 10, 12).unwrap();
    ledger_gui::demo::build(&dir, today).unwrap();
    Source {
        data_dir: dir.join("books-data"),
        books_toml: Some(dir.join("wanderbooks.toml")),
        today: Some(today),
    }
}

/// INVARIANTS 1-3 on every kind of page in the demo books.
#[test]
fn crumbs_link_every_level_but_the_last() {
    let b = data::load_once(&demo("unit")).unwrap();
    let party = b.ledger.parties().next().unwrap().id;
    let rec = b.reconciliations.first().expect("the demo reconciles");
    let entry = b.entries.first().unwrap().id;
    let mut routes = vec![
        Route::Party(party),
        Route::Reconciliation(rec.id),
        Route::Entry(entry),
        Route::Reports(ReportKind::ProfitAndLoss),
        Route::Reports(ReportKind::BalanceSheet),
    ];
    routes.extend(Section::ALL.iter().map(|s| s.home()));
    for r in &routes {
        let c = crumbs(Some(&b), r);
        assert_eq!(c[0].0, r.section().title(), "{r:?}");
        assert!(c.last().unwrap().1.is_none(), "{r:?}: the page you're on");
        for (_, to) in &c {
            assert_ne!(to.as_ref(), Some(r), "{r:?} links to itself");
        }
        // Every crumb above is a link, except one naming this very page
        // (Reports on Profit and loss, the Reports section's home).
        for (i, (_, to)) in c[..c.len() - 1].iter().enumerate() {
            let home_here = i == 0 && r.section().home() == *r;
            assert_eq!(to.is_some(), !home_here, "{r:?}: crumb {i}");
        }
        let here = describe(&b, r);
        if !here.is_empty() {
            assert_eq!(c.last().unwrap().0, here);
        }
    }
    assert_eq!(
        crumbs(Some(&b), &Route::Party(party)),
        [
            ("Parties".to_string(), Some(Route::Parties)),
            (b.party_label(party), None),
        ]
    );
    // A reconciliation sits under its account.
    let account = Route::Account(rec.account_id);
    let c = crumbs(Some(&b), &Route::Reconciliation(rec.id));
    assert_eq!(c.len(), 3);
    assert_eq!(c[0].1, Some(Route::Accounts));
    assert_eq!(c[1], (describe(&b, &account), Some(account)));
    // A section's own page is one crumb, not a link.
    assert_eq!(
        crumbs(Some(&b), &Route::Parties),
        [("Parties".to_string(), None)]
    );
    // P&L is the Reports section's home, so "Reports" there isn't a link.
    assert_eq!(
        crumbs(Some(&b), &Route::Reports(ReportKind::ProfitAndLoss))[0].1,
        None
    );
}

/// INVARIANT 4, through the top bar.
#[test]
fn clicking_a_crumb_goes_there() {
    let source = demo("click");
    let b = data::load_once(&source).unwrap();
    let rec = b.reconciliations.first().unwrap().clone();
    let account = describe(&b, &Route::Account(rec.account_id));
    let mut h = Harness::builder()
        .with_size(egui::vec2(1280.0, 900.0))
        .build_eframe(|cc| App::with_books(&cc.egui_ctx, b, false, Route::Reconciliation(rec.id)));
    h.run_steps(3);
    h.get_all_by_label(&account).next().unwrap().click();
    h.run_steps(3);
    assert_eq!(h.state().route(), &Route::Account(rec.account_id));
    h.get_by_label("Accounts").click();
    h.run_steps(3);
    assert_eq!(h.state().route(), &Route::Accounts);
}
