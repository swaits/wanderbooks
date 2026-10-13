//! Screenshots of every screen, for eyes. Set `SHOT_BOOKS` to a books dir
//! (containing `books-data/` and `wanderbooks.toml`) and `SHOT_DIR` to
//! where the PNGs go; without them this test does nothing.

use egui_kittest::Harness;
use ledger_gui::nav::{ReportKind, Route};
use ledger_gui::{App, Source, data};
use std::path::PathBuf;

#[test]
fn screenshots() {
    let Ok(out) = std::env::var("SHOT_DIR") else {
        return;
    };
    let fixed_today: Option<chrono::NaiveDate> = std::env::var("SHOT_TODAY")
        .ok()
        .and_then(|d| d.parse().ok());
    let books = match std::env::var("SHOT_BOOKS") {
        Ok(b) => PathBuf::from(b),
        Err(_) => {
            let d = fixed_today
                .unwrap_or_else(|| chrono::NaiveDate::from_ymd_opt(2026, 10, 12).unwrap());
            let dir =
                std::env::temp_dir().join(format!("ledger-gui-shots-demo-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            ledger_gui::demo::build(&dir, d).unwrap();
            dir
        }
    };
    let out = PathBuf::from(out);
    std::fs::create_dir_all(&out).unwrap();
    let source = Source {
        data_dir: books.join("books-data"),
        books_toml: Some(books.join("wanderbooks.toml")),
        today: fixed_today.or_else(|| {
            std::env::var("SHOT_BOOKS")
                .is_err()
                .then(|| chrono::NaiveDate::from_ymd_opt(2026, 10, 12).unwrap())
        }),
    };
    let only = std::env::var("SHOT_ONLY").ok();
    let dark = std::env::var("SHOT_DARK").is_ok();
    let first_entry = data::load_once(&source)
        .unwrap()
        .entries
        .last()
        .map(|e| e.id)
        .unwrap_or(1);
    let routes: Vec<(&str, Route)> = vec![
        ("overview", Route::Overview),
        ("transactions", Route::Transactions),
        (
            "entry",
            Route::Entry(
                std::env::var("SHOT_ENTRY")
                    .ok()
                    .and_then(|e| e.parse().ok())
                    .unwrap_or(first_entry),
            ),
        ),
        ("cash", Route::Cash),
        ("profit", Route::Profit),
        ("pnl", Route::Reports(ReportKind::ProfitAndLoss)),
        ("balance", Route::Reports(ReportKind::BalanceSheet)),
        ("accounts", Route::Accounts),
        ("documents", Route::Documents),
        ("company", Route::Company),
        ("journal", Route::Journal),
    ];
    for (name, route) in routes {
        if only
            .as_deref()
            .is_some_and(|o| !o.split(',').any(|x| x == name))
        {
            continue;
        }
        let books = data::load_once(&source).unwrap();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(
                1440.0,
                std::env::var("SHOT_HEIGHT")
                    .ok()
                    .and_then(|h| h.parse().ok())
                    .unwrap_or(920.0),
            ))
            .build_eframe(|cc| App::with_books(&cc.egui_ctx, books, dark, route.clone()));
        harness.run_steps(4);
        let ctx = harness.ctx.clone();
        harness.state_mut().settle(&ctx);
        harness.run_steps(4);
        let img = harness.render().unwrap();
        img.save(out.join(format!("{name}.png"))).unwrap();
    }
}
