//! The viewer is read-only: it never writes into the books.
//!
//! ```text
//! GOAL: Tobin, Mo, and the CPA can open the books in the viewer
//!       without any chance of changing them.
//! SOURCE: DECISION (AGENTS.md: "read-only and live ... It never writes;
//!         changes go through you").
//! INVARIANTS:
//!   1. With every file and folder of the books made read-only on disk,
//!      the viewer still loads, follows live, and renders every page
//!      (receipts included), and reports the books whole.
//!   2. Afterwards every file holds the same bytes, size, and
//!      modification time, and no file has appeared or gone; on writable
//!      books too (no SQLite -wal or -shm created beside the database).
//! ```

use egui_kittest::Harness;
use ledger_gui::data::{self, Loader, Update};
use ledger_gui::nav::{ReportKind, Route};
use ledger_gui::{App, Source};
use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

type Tree = BTreeMap<PathBuf, (Vec<u8>, u64, SystemTime)>;

fn tree(root: &Path) -> Tree {
    fn walk(root: &Path, d: &Path, out: &mut Tree) {
        for e in std::fs::read_dir(d).unwrap() {
            let p = e.unwrap().path();
            let m = std::fs::symlink_metadata(&p).unwrap();
            if m.is_dir() {
                walk(root, &p, out);
            } else {
                out.insert(
                    p.strip_prefix(root).unwrap().to_path_buf(),
                    (std::fs::read(&p).unwrap(), m.len(), m.modified().unwrap()),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

/// Every file 0444 and folder 0555 (`read_only`), or back to 0644/0755.
fn set_read_only(root: &Path, read_only: bool) {
    fn walk(p: &Path, ro: bool) {
        let m = std::fs::symlink_metadata(p).unwrap();
        if m.is_dir() {
            if !ro {
                std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            for e in std::fs::read_dir(p).unwrap() {
                walk(&e.unwrap().path(), ro);
            }
            if ro {
                std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o555)).unwrap();
            }
        } else {
            let mode = if ro { 0o444 } else { 0o644 };
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(mode)).unwrap();
        }
    }
    walk(root, read_only);
}

/// Makes the books writable again when dropped, so a failing or
/// interrupted run never leaves a folder no one can delete.
struct Writable(PathBuf);
impl Drop for Writable {
    fn drop(&mut self) {
        if self.0.exists() {
            set_read_only(&self.0, false);
        }
    }
}

/// Folders left read-only by a killed run (no destructor ran): made
/// writable and removed once their process is gone.
fn sweep_leftovers() {
    let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else {
        return;
    };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let Some(pid) = name.strip_prefix("ledger-gui-ro-") else {
            continue;
        };
        if Path::new("/proc").join(pid).exists() {
            continue; // still running (or the pid was reused): leave it
        }
        set_read_only(&e.path(), false);
        let _ = std::fs::remove_dir_all(e.path());
    }
}

#[test]
fn the_viewer_never_writes_into_the_books() {
    sweep_leftovers();
    let dir = std::env::temp_dir().join(format!("ledger-gui-ro-{}", std::process::id()));
    if dir.exists() {
        set_read_only(&dir, false);
        std::fs::remove_dir_all(&dir).unwrap();
    }
    let today = chrono::NaiveDate::from_ymd_opt(2026, 10, 12).unwrap();
    ledger_gui::demo::build(&dir, today).unwrap();
    let source = Source {
        data_dir: dir.join("books-data"),
        books_toml: Some(dir.join("wanderbooks.toml")),
        today: Some(today),
    };
    let _restore = Writable(dir.clone());
    set_read_only(&dir, true);
    let before = tree(&dir);

    // Load once; the books are whole (the database matches the journal).
    let books = data::load_once(&source).expect("loads from read-only books");
    assert!(
        matches!(books.integrity.disk, ledger_store::DiskCheck::Matches),
        "read-only books reported as {:?}",
        books.integrity.disk
    );
    assert!(books.integrity.file_failures.is_empty());
    let entry = books.entries.last().map(|e| e.id).unwrap();
    let cash = books.cash_accounts[0];
    let doc = books.attachments.last().map(|a| a.id).unwrap();
    let event = books.seq();
    let asset = books
        .ledger
        .registered_assets()
        .next()
        .map(|a| a.id)
        .unwrap();
    drop(books);

    // Follow live for a moment.
    let loader = Loader::spawn(source.clone(), egui::Context::default());
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    let mut live = false;
    while std::time::Instant::now() < deadline {
        match loader.wait(Duration::from_millis(200)) {
            Some(Update::Books(_)) => {
                live = true;
                break;
            }
            Some(Update::Error(e)) => panic!("live loader on read-only books: {e}"),
            _ => {}
        }
    }
    assert!(live, "the live loader never delivered the books");

    // Render every page.
    let routes = [
        Route::Overview,
        Route::Transactions,
        Route::Entry(entry),
        Route::Cash,
        Route::Profit,
        Route::Reports(ReportKind::ProfitAndLoss),
        Route::Reports(ReportKind::BalanceSheet),
        Route::Reports(ReportKind::CashFlow),
        Route::Reports(ReportKind::TrialBalance),
        Route::Reports(ReportKind::GeneralLedger),
        Route::Reports(ReportKind::TaxSummary),
        Route::Reports(ReportKind::Vendor1099),
        Route::Reports(ReportKind::ArAging),
        Route::Reports(ReportKind::ApAging),
        Route::Reports(ReportKind::MembersEquity),
        Route::Reports(ReportKind::FixedAssetRegister),
        Route::Accounts,
        Route::Account(cash),
        Route::Assets,
        Route::Asset(asset),
        Route::Document(doc),
        Route::JournalEvent(event),
        Route::Documents,
        Route::Company,
        Route::Journal,
    ];
    for route in routes {
        let books = data::load_once(&source).unwrap();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1280.0, 800.0))
            .build_eframe(|cc| App::with_books(&cc.egui_ctx, books, false, route.clone()));
        harness.run_steps(3);
        let ctx = harness.ctx.clone();
        harness.state_mut().settle(&ctx);
        harness.run_steps(2);
    }

    let after = tree(&dir);
    set_read_only(&dir, false);
    let changed: Vec<_> = before
        .keys()
        .chain(after.keys())
        .filter(|k| before.get(*k) != after.get(*k))
        .collect();
    assert!(changed.is_empty(), "the viewer changed {changed:?}");

    // On writable books too: loading and checking creates nothing (no
    // SQLite -wal or -shm beside the database).
    let before = tree(&dir);
    let books = data::load_once(&source).unwrap();
    assert!(matches!(
        books.integrity.disk,
        ledger_store::DiskCheck::Matches
    ));
    drop(books);
    let after = tree(&dir);
    assert_eq!(
        before.keys().collect::<Vec<_>>(),
        after.keys().collect::<Vec<_>>(),
        "the viewer created or removed files in writable books"
    );
    assert!(before == after, "the viewer changed writable books");
    let _ = std::fs::remove_dir_all(&dir);
}
