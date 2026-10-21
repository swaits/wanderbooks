//! books.show requests drive the viewer to the right place.

use egui_kittest::Harness;
use ledger_gui::nav::{ReportKind, Route};
use ledger_gui::{App, Source, data};
use ledger_store::ops::ViewerTarget;

/// A test's dir, removed when the test ends, pass or panic.
struct TestDir(std::path::PathBuf);

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn show_requests_navigate_the_viewer() {
    let today = chrono::NaiveDate::from_ymd_opt(2026, 10, 12).unwrap();
    let dir = std::env::temp_dir().join(format!("ledger-gui-show-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let _cleanup = TestDir(dir.clone());
    ledger_gui::demo::build(&dir, today).unwrap();
    let source = Source {
        data_dir: dir.join("books-data"),
        today: Some(today),
    };
    let books = data::load_once(&source).unwrap();
    let rent = books.ledger.chart().find_by_code("6200").unwrap().id;
    let laptop = books.ledger.asset_by_tag("js-0001").unwrap().id;
    let mut h = Harness::builder()
        .with_size(egui::vec2(1440.0, 920.0))
        .build_eframe(|cc| App::with_books(&cc.egui_ctx, books, true, Route::Overview));

    // "Compare this quarter's P&L with the same quarter last year, by month."
    h.state_mut().show(ViewerTarget::Report {
        report: "profit_and_loss".into(),
        start: Some("2026-07-01".into()),
        end: Some("2026-09-30".into()),
        columns: Some("month".into()),
        compare: Some("prior_year".into()),
    });
    h.run_steps(2);
    assert_eq!(
        h.state_mut().route(),
        &Route::Reports(ReportKind::ProfitAndLoss)
    );
    let st = &h.state_mut().state_mut().reports;
    assert_eq!(
        st.range,
        ledger_gui::pages::reports::Range::Custom(
            chrono::NaiveDate::from_ymd_opt(2026, 7, 1).unwrap(),
            chrono::NaiveDate::from_ymd_opt(2026, 9, 30).unwrap()
        )
    );
    assert_eq!(st.columns, ledger_gui::pages::reports::Columns::Month);

    // "Show me the rent account for the summer."
    h.state_mut().show(ViewerTarget::Account {
        code: "6200".into(),
        start: Some("2026-06-01".into()),
        end: Some("2026-08-31".into()),
    });
    h.run_steps(2);
    assert_eq!(h.state_mut().route(), &Route::Account(rent));

    // "Find every Adobe charge."
    h.state_mut().show(ViewerTarget::Search {
        query: "adobe".into(),
    });
    h.run_steps(2);
    assert_eq!(h.state_mut().route(), &Route::Transactions);
    assert_eq!(h.state_mut().state_mut().transactions.query, "adobe");

    // "Show me the asset register", "the laptop", "the register at June 30".
    h.state_mut().show(ViewerTarget::Page {
        page: "assets".into(),
    });
    h.run_steps(2);
    assert_eq!(h.state_mut().route(), &Route::Assets);
    h.state_mut().show(ViewerTarget::Asset { asset_id: laptop });
    h.run_steps(2);
    assert_eq!(h.state_mut().route(), &Route::Asset(laptop));
    h.state_mut().show(ViewerTarget::Report {
        report: "fixed_asset_register".into(),
        start: None,
        end: Some("2026-06-30".into()),
        columns: None,
        compare: None,
    });
    h.run_steps(2);
    assert_eq!(
        h.state_mut().route(),
        &Route::Reports(ReportKind::FixedAssetRegister)
    );
    assert!(matches!(
        h.state_mut().state_mut().reports.range,
        ledger_gui::pages::reports::Range::Custom(_, e)
            if e == chrono::NaiveDate::from_ymd_opt(2026, 6, 30).unwrap()
    ));
    // An unknown asset is ignored.
    h.state_mut().show(ViewerTarget::Asset { asset_id: 999 });
    h.run_steps(1);
    assert_eq!(
        h.state_mut().route(),
        &Route::Reports(ReportKind::FixedAssetRegister)
    );
    h.state_mut().show(ViewerTarget::Search {
        query: "adobe".into(),
    });
    h.run_steps(2);

    // Unknown accounts are ignored, not crashes.
    h.state_mut().show(ViewerTarget::Account {
        code: "9999".into(),
        start: None,
        end: None,
    });
    assert_eq!(h.state_mut().route(), &Route::Transactions);
}

/// The demo books (calendar fiscal year) in a viewer, at `today`.
fn demo(
    tag: &str,
    today: chrono::NaiveDate,
) -> (TestDir, Harness<'static, App>, ledger_gui::books::Books) {
    demo_sized(tag, today, 920.0)
}

/// [`demo`] in a window `height` tall (the register's date row sits below
/// an account's cards: tall enough to click it).
fn demo_sized(
    tag: &str,
    today: chrono::NaiveDate,
    height: f32,
) -> (TestDir, Harness<'static, App>, ledger_gui::books::Books) {
    let dir = std::env::temp_dir().join(format!("ledger-gui-show-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let guard = TestDir(dir.clone());
    ledger_gui::demo::build(&dir, today).unwrap();
    let source = Source {
        data_dir: dir.join("books-data"),
        today: Some(today),
    };
    let books = data::load_once(&source).unwrap();
    let copy = data::load_once(&source).unwrap();
    let h = Harness::builder()
        .with_size(egui::vec2(1440.0, height))
        .build_eframe(move |cc| App::with_books(&cc.egui_ctx, books, true, Route::Overview));
    (guard, h, copy)
}

fn day(y: i32, m: u32, d: u32) -> chrono::NaiveDate {
    chrono::NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

/// Defect 15, repros 1.1.1-1.1.2 (rules.md B-30): a target the viewer
/// can't honour is ignored whole, never swapped or guessed: a start after
/// the end (an earlier writer's, or a hand-edited ops/show.jsonl), text
/// that isn't a date, an unknown split or comparison. The page, the
/// report's range and split, and the register's range stay as they were.
#[test]
fn show_never_swaps_an_inverted_target_defect_15() {
    use ledger_gui::pages::accounts::RegisterRange;
    use ledger_gui::pages::reports::{Columns, Compare, Range};
    let (_dir, mut h, _) = demo("inverted", day(2026, 10, 20));
    h.state_mut().state_mut().reports.range = Range::LastMonth;
    h.state_mut().state_mut().reports.columns = Columns::Quarter;
    h.state_mut().state_mut().accounts.range = RegisterRange::All;
    let report =
        |start: Option<&str>, end: Option<&str>, columns: Option<&str>, compare: Option<&str>| {
            ViewerTarget::Report {
                report: "profit_and_loss".into(),
                start: start.map(Into::into),
                end: end.map(Into::into),
                columns: columns.map(Into::into),
                compare: compare.map(Into::into),
            }
        };
    for target in [
        report(Some("2026-12-31"), Some("2026-01-01"), None, None),
        report(Some("2027-01-01"), None, None, None),
        report(Some("January"), None, None, None),
        report(None, Some("2026-13-01"), None, None),
        report(None, None, Some("weekly"), None),
        report(None, None, None, Some("prior_decade")),
        ViewerTarget::Account {
            code: "1000".into(),
            start: Some("2026-12-31".into()),
            end: Some("2026-01-01".into()),
        },
        ViewerTarget::Account {
            code: "1000".into(),
            start: Some("2027-01-01".into()),
            end: None,
        },
    ] {
        h.state_mut().show(target.clone());
        h.run_steps(2);
        assert_eq!(h.state_mut().route(), &Route::Overview, "{target:?}");
        let st = h.state_mut().state_mut();
        assert_eq!(st.reports.range, Range::LastMonth, "{target:?}");
        assert_eq!(st.reports.columns, Columns::Quarter, "{target:?}");
        assert_eq!(st.reports.compare, Compare::None, "{target:?}");
        assert_eq!(st.accounts.range, RegisterRange::All, "{target:?}");
    }
}

/// Defect 16, repro 1.1.4 (rules.md B-30): a report target with only an
/// end covers that end's fiscal year to it (2025-01-01 to 2025-06-30, not
/// the swapped 2025-06-30 to 2026-01-01); with no dates, the default (the
/// fiscal year to date, named "Year to date"); dates that are a preset's
/// show under its name; an account target with no dates opens the
/// register on the default.
#[test]
fn show_report_with_only_end_uses_that_years_start_defect_16() {
    use ledger_gui::pages::accounts::RegisterRange;
    use ledger_gui::pages::reports::Range;
    let today = day(2026, 10, 20);
    let (_dir, mut h, books) = demo("only-end", today);
    let report = |start: Option<&str>, end: Option<&str>| ViewerTarget::Report {
        report: "profit_and_loss".into(),
        start: start.map(Into::into),
        end: end.map(Into::into),
        columns: None,
        compare: None,
    };
    h.state_mut().show(report(None, Some("2025-06-30")));
    h.run_steps(2);
    assert_eq!(
        h.state_mut().state_mut().reports.range,
        Range::Custom(day(2025, 1, 1), day(2025, 6, 30))
    );
    h.state_mut().state_mut().reports.range = Range::LastMonth;
    h.state_mut().show(report(None, None));
    h.run_steps(2);
    assert_eq!(h.state_mut().state_mut().reports.range, Range::YearToDate);
    assert_eq!(Range::YearToDate.dates(&books), (day(2026, 1, 1), today));
    // What books.show sends for no dates: the same, by name.
    h.state_mut()
        .show(report(Some("2026-01-01"), Some("2026-10-20")));
    h.run_steps(2);
    assert_eq!(h.state_mut().state_mut().reports.range, Range::YearToDate);
    h.state_mut()
        .show(report(Some("2026-09-01"), Some("2026-09-30")));
    h.run_steps(2);
    assert_eq!(h.state_mut().state_mut().reports.range, Range::LastMonth);
    // Only a start: through today.
    h.state_mut().show(report(Some("2026-03-15"), None));
    h.run_steps(2);
    assert_eq!(
        h.state_mut().state_mut().reports.range,
        Range::Custom(day(2026, 3, 15), today)
    );
    // An account with no dates (an earlier writer's): the default.
    h.state_mut().state_mut().accounts.range = RegisterRange::All;
    h.state_mut().show(ViewerTarget::Account {
        code: "1000".into(),
        start: None,
        end: None,
    });
    h.run_steps(2);
    assert_eq!(
        h.state_mut().state_mut().accounts.range,
        RegisterRange::Default
    );
    assert_eq!(
        RegisterRange::Default.dates(&books),
        (day(2026, 1, 1), today)
    );
}

/// Settled in design (P3, defect 16): an account's register opens on the
/// fiscal year to date with "Show all dates" one click away: after
/// books.show of an account (which sends the resolved dates), the button
/// shows every date, the books' first entry through today; from there,
/// "Fiscal year to date" goes back.
#[test]
fn show_all_dates_is_one_click_from_a_shown_register() {
    use egui_kittest::kittest::Queryable;
    use ledger_gui::pages::accounts::RegisterRange;
    let today = day(2026, 10, 20);
    let (_dir, mut h, books) = demo_sized("all-dates", today, 3000.0);
    let id = books.ledger.chart().find_by_code("1000").unwrap().id;
    h.state_mut().show(ViewerTarget::Account {
        code: "1000".into(),
        start: Some("2026-01-01".into()),
        end: Some("2026-10-20".into()),
    });
    h.run_steps(2);
    assert_eq!(h.state_mut().route(), &Route::Account(id));
    assert_eq!(
        h.state_mut().state_mut().accounts.range,
        RegisterRange::Dates(day(2026, 1, 1), today)
    );
    h.get_by_label("Show all dates").click();
    h.run_steps(2);
    assert_eq!(h.state_mut().state_mut().accounts.range, RegisterRange::All);
    let first = books.kpis.first_entry.unwrap();
    assert_eq!(RegisterRange::All.dates(&books), (first.min(today), today));
    assert!(
        first < day(2026, 1, 1),
        "the demo's books began before this year"
    );
    h.get_by_label("Fiscal year to date").click();
    h.run_steps(2);
    assert_eq!(
        h.state_mut().state_mut().accounts.range,
        RegisterRange::Default
    );
}
