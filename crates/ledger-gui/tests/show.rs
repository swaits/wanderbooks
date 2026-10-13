//! books.show requests drive the viewer to the right place.

use egui_kittest::Harness;
use ledger_gui::nav::{ReportKind, Route};
use ledger_gui::{App, Source, data};
use ledger_store::viewer::ViewerTarget;

#[test]
fn show_requests_navigate_the_viewer() {
    let today = chrono::NaiveDate::from_ymd_opt(2026, 10, 12).unwrap();
    let dir = std::env::temp_dir().join(format!("ledger-gui-show-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    ledger_gui::demo::build(&dir, today).unwrap();
    let source = Source {
        data_dir: dir.join("books-data"),
        books_toml: Some(dir.join("wanderbooks.toml")),
        today: Some(today),
    };
    let books = data::load_once(&source).unwrap();
    let rent = books.ledger.chart().find_by_code("6200").unwrap().id;
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

    // Unknown accounts are ignored, not crashes.
    h.state_mut().show(ViewerTarget::Account {
        code: "9999".into(),
        start: None,
        end: None,
    });
    assert_eq!(h.state_mut().route(), &Route::Transactions);
    let _ = std::fs::remove_dir_all(&dir);
}
