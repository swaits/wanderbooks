//! The viewer's update notice and its "Check for updates" menu item
//! (owner.rs, P11a step 4): a staged update the owner reports shows a
//! standing notice with an "Update now" button; the menu can look for one;
//! accepting hands the work to the owner, off the UI thread.

use chrono::NaiveDate;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ledger_core::Currency;
use ledger_gui::App;
use ledger_gui::books::Books;
use ledger_gui::data::{self, Source};
use ledger_gui::nav::Route;
use ledger_gui::owner::{Owner, RestorePlan, UpdateWaiting};
use ledger_store::Store;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

/// An owner with an update that waits once it has been "found".
#[derive(Default)]
struct Fake {
    asked: Mutex<Vec<String>>,
    staged: Mutex<bool>,
}

impl Fake {
    fn said(&self, what: &str) {
        self.asked
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(what.into());
    }
}

impl Owner for Fake {
    fn close_books(&self) -> Result<String, String> {
        Err("not in these tests".into())
    }
    fn new_books(&self, _: &str, _: &str) -> Result<String, String> {
        Err("not in these tests".into())
    }
    fn back_up_now(&self, _: &str) -> Result<Vec<String>, String> {
        Err("not in these tests".into())
    }
    fn plan_restore(&self, _: &str, _: Option<String>, _: bool) -> Result<RestorePlan, String> {
        Err("not in these tests".into())
    }
    fn restore(&self, _: &RestorePlan) -> Result<Vec<String>, String> {
        Err("not in these tests".into())
    }
    fn connect(&self, _: ledger_gui::owner::Agent, _: &str) -> Result<Vec<String>, String> {
        Err("not in these tests".into())
    }
    fn upgrade_format(&self, _: &str, _: &str) -> Result<Vec<String>, String> {
        Err("not in these tests".into())
    }
    fn update_waiting(&self) -> Option<UpdateWaiting> {
        (*self.staged.lock().unwrap()).then(|| UpdateWaiting {
            version: "9.1.0".into(),
            staged_at: "2026-10-30T08:00:00Z".into(),
        })
    }
    fn check_for_updates(&self) -> Result<Vec<String>, String> {
        self.said("check");
        *self.staged.lock().unwrap() = true;
        Ok(vec!["Version 9.1.0 is downloaded and checked.".into()])
    }
    fn apply_update(&self) -> Result<Vec<String>, String> {
        self.said("apply");
        *self.staged.lock().unwrap() = false;
        Ok(vec!["Version 9.1.0 is installed.".into()])
    }
}

fn alone() -> std::sync::MutexGuard<'static, ()> {
    static ONE: Mutex<()> = Mutex::new(());
    ONE.lock().unwrap_or_else(|e| e.into_inner())
}

fn fake() -> Arc<Fake> {
    static FAKE: OnceLock<Arc<Fake>> = OnceLock::new();
    FAKE.get_or_init(|| {
        let f = Arc::new(Fake::default());
        ledger_gui::owner::set_owner(f.clone());
        f
    })
    .clone()
}

fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, 15).unwrap()
}

/// Books in a data root (`root`: a folder holding the root's marker), or
/// outside one (as the demo's are).
fn books(root: bool) -> (tempfile::TempDir, PathBuf, Books) {
    let dir = tempfile::Builder::new()
        .prefix("wb-gui-owner-")
        .tempdir()
        .unwrap();
    let book = if root {
        std::fs::write(dir.path().join(ledger_store::root::MARKER_FILE), b"").unwrap();
        dir.path().join("books").join("acme")
    } else {
        dir.path().join("books-data")
    };
    std::fs::create_dir_all(&book).unwrap();
    drop(Store::create(&book, Currency::USD).unwrap());
    let b = data::load_once(&Source {
        data_dir: book.clone(),
        today: Some(today()),
    })
    .unwrap();
    (dir, book, b)
}

fn harness(b: Books) -> Harness<'static, App> {
    let mut h = Harness::builder()
        .with_size(egui::vec2(1200.0, 900.0))
        .build_eframe(|cc| App::with_books(&cc.egui_ctx, b, false, Route::Overview));
    h.run_steps(3);
    h
}

fn wait_for(h: &mut Harness<'static, App>, f: &Fake, n: usize) {
    for _ in 0..200 {
        h.run_steps(2);
        if f.asked.lock().unwrap().len() >= n {
            h.run_steps(4);
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("the owner wasn't asked: {:?}", f.asked.lock().unwrap());
}

#[test]
fn the_menu_checks_for_updates_and_a_waiting_update_shows_a_notice_that_accepts() {
    let _alone = alone();
    let f = fake();
    let (_dir, _book, b) = books(true);
    let mut h = harness(b);

    // Nothing waits: no notice.
    assert!(h.query_by_label_contains("is ready.").is_none());

    // "Check for updates" asks the owner to look; what it finds shows.
    h.get_by_label_contains(" Books").click();
    h.run_steps(3);
    h.get_by_label_contains("Check for updates").click();
    wait_for(&mut h, &f, 1);
    assert_eq!(f.asked.lock().unwrap()[0], "check");
    // The job says so, and the notice says so too: either is the same
    // version, so ask for at least one rather than exactly one.
    assert!(
        h.query_all_by_label_contains("downloaded and checked")
            .next()
            .is_some()
    );

    // The standing notice names the version; accepting hands it to the
    // owner, and the notice goes once it is installed.
    for _ in 0..60 {
        h.run_steps(2);
        if h.query_by_label_contains("Version 9.1.0 of wanderbooks is ready")
            .is_some()
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(
        h.query_by_label_contains("Version 9.1.0 of wanderbooks is ready")
            .is_some(),
        "no notice"
    );
    h.get_by_label("Update now").click();
    wait_for(&mut h, &f, 2);
    assert_eq!(f.asked.lock().unwrap()[1], "apply");
    for _ in 0..60 {
        h.run_steps(2);
        if h.query_by_label_contains("is ready.").is_none() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(h.query_by_label_contains("is ready.").is_none());
}

#[test]
fn books_outside_a_data_root_have_no_update_notice() {
    let _alone = alone();
    let f = fake();
    *f.staged.lock().unwrap() = true;
    let (_dir, _book, b) = books(false);
    let h = harness(b);
    assert!(h.query_by_label_contains("is ready.").is_none());
    *f.staged.lock().unwrap() = false;
}
