//! Screenshots of every screen, for eyes. Set `SHOT_BOOKS` to a books dir
//! (containing `books-data/` and `wanderbooks.toml`) and `SHOT_DIR` to
//! where the PNGs go; without them this test does nothing.

use egui_kittest::Harness;
use ledger_gui::nav::{ReportKind, Route};
use ledger_gui::{App, Source, data};
use std::path::PathBuf;

/// A test's dir, removed when the test ends, pass or panic.
struct TestDir(PathBuf);

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn screenshots() {
    let Ok(out) = std::env::var("SHOT_DIR") else {
        return;
    };
    let fixed_today: Option<chrono::NaiveDate> = std::env::var("SHOT_TODAY")
        .ok()
        .and_then(|d| d.parse().ok());
    // Demo books (no SHOT_BOOKS) go when the run ends, pass or panic.
    let mut _demo = None;
    let books = match std::env::var("SHOT_BOOKS") {
        Ok(b) => PathBuf::from(b),
        Err(_) => {
            let d = fixed_today
                .unwrap_or_else(|| chrono::NaiveDate::from_ymd_opt(2026, 10, 12).unwrap());
            let dir =
                std::env::temp_dir().join(format!("ledger-gui-shots-demo-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            _demo = Some(TestDir(dir.clone()));
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
    let loaded = data::load_once(&source).unwrap();
    let first_entry = loaded.entries.last().map(|e| e.id).unwrap_or(1);
    let first_cash = loaded.cash_accounts.first().copied().unwrap_or(1);
    let last_doc = loaded.attachments.last().map(|a| a.id).unwrap_or(1);
    let last_event = loaded.seq();
    let first_asset = loaded
        .ledger
        .registered_assets()
        .next()
        .map(|a| a.id)
        .unwrap_or(1);
    // A party with the most to show: W-9 facts first, else any.
    let party = std::env::var("SHOT_PARTY")
        .ok()
        .and_then(|p| p.parse().ok())
        .or_else(|| {
            loaded
                .ledger
                .parties()
                .find(|p| p.w9.is_some() && p.vendor.is_some())
                .or_else(|| loaded.ledger.parties().next())
                .map(|p| p.id)
        })
        .unwrap_or(1);
    let reconciliation = loaded.reconciliations.last().map(|r| r.id).unwrap_or(1);
    let voided = loaded
        .reconciliations
        .iter()
        .find(|r| r.voided.is_some())
        .map_or(reconciliation, |r| r.id);
    // A document linked to more than one subject, else the last.
    let linked_doc = loaded
        .attachments
        .iter()
        .rev()
        .find(|a| loaded.links_of(a.id).len() > 1)
        .map_or(last_doc, |a| a.id);
    drop(loaded);
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
        ("cashflow", Route::Reports(ReportKind::CashFlow)),
        ("trial", Route::Reports(ReportKind::TrialBalance)),
        ("gl", Route::Reports(ReportKind::GeneralLedger)),
        ("tax", Route::Reports(ReportKind::TaxSummary)),
        ("v1099", Route::Reports(ReportKind::Vendor1099)),
        ("ar", Route::Reports(ReportKind::ArAging)),
        (
            "asset-register",
            Route::Reports(ReportKind::FixedAssetRegister),
        ),
        ("accounts", Route::Accounts),
        ("account", Route::Account(first_cash)),
        ("assets", Route::Assets),
        ("asset", Route::Asset(first_asset)),
        ("parties", Route::Parties),
        ("party", Route::Party(party)),
        ("document", Route::Document(last_doc)),
        ("document-linked", Route::Document(linked_doc)),
        ("reconciliation", Route::Reconciliation(reconciliation)),
        ("reconciliation-voided", Route::Reconciliation(voided)),
        ("event", Route::JournalEvent(last_event)),
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
                std::env::var("SHOT_WIDTH")
                    .ok()
                    .and_then(|w| w.parse().ok())
                    .unwrap_or(1440.0),
                std::env::var("SHOT_HEIGHT")
                    .ok()
                    .and_then(|h| h.parse().ok())
                    .unwrap_or(920.0),
            ))
            .build_eframe(|cc| App::with_books(&cc.egui_ctx, books, dark, route.clone()));
        if std::env::var("SHOT_LIVE").is_ok() {
            harness.state_mut().show_live_label();
        }
        harness.run_steps(4);
        let ctx = harness.ctx.clone();
        harness.state_mut().settle(&ctx);
        harness.run_steps(4);
        let img = harness.render().unwrap();
        img.save(out.join(format!("{name}.png"))).unwrap();
    }
}
