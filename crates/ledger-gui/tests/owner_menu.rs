//! The viewer's Books menu (owner.rs, plan P2 "Viewer menu", D-32): each
//! item hands its work to the owner the program gives the viewer, off the
//! UI thread, and says what came of it; a restore is previewed first; the
//! viewer itself writes nothing of the books; without an owner, or for an
//! older books dir, the menu isn't there.

use chrono::NaiveDate;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ledger_core::Currency;
use ledger_gui::App;
use ledger_gui::books::Books;
use ledger_gui::data::{self, Source};
use ledger_gui::nav::Route;
use ledger_gui::owner::{Agent, Owner, RestorePlan};
use ledger_store::Store;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

/// An owner that does nothing but say what it was asked.
#[derive(Default)]
struct Fake {
    asked: Mutex<Vec<String>>,
}

impl Fake {
    fn said(&self, what: String) {
        self.asked
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(what);
    }
}

impl Owner for Fake {
    fn close_books(&self) -> Result<String, String> {
        self.said("close".into());
        Ok("The books are closing.".into())
    }
    fn new_books(&self, id: &str, currency: &str) -> Result<String, String> {
        self.said(format!("new {id} {currency}"));
        Ok(format!("Made new books \"{id}\" ({currency})."))
    }
    fn back_up_now(&self, book: &str) -> Result<Vec<String>, String> {
        // Slow enough to be seen at work.
        std::thread::sleep(std::time::Duration::from_millis(300));
        self.said(format!("back up {book}"));
        Err("Not backed up to the backup folder: the backup folder isn't there (is its disk connected?)".into())
    }
    fn plan_restore(
        &self,
        source: &str,
        id: Option<String>,
        replace: bool,
    ) -> Result<RestorePlan, String> {
        std::thread::sleep(std::time::Duration::from_millis(300));
        self.said(format!("plan {source} {id:?} {replace}"));
        Ok(RestorePlan {
            source: source.to_string(),
            id: id.unwrap_or_else(|| "acme".into()),
            replace,
            head: (7, "ab".repeat(32)),
            lines: vec!["The backup at the disk is whole: through event 7.".into()],
        })
    }
    fn restore(&self, plan: &RestorePlan) -> Result<Vec<String>, String> {
        self.said(format!(
            "restore {} {} {}",
            plan.source, plan.id, plan.head.0
        ));
        Ok(vec![format!("Restored as \"{}\".", plan.id)])
    }
    fn connect(&self, agent: Agent, book: &str) -> Result<Vec<String>, String> {
        self.said(format!("connect {agent:?} {book}"));
        Ok(vec!["Claude Code reaches the books.".into()])
    }
}

/// One test at a time: they share the one owner and its record of calls.
fn alone() -> std::sync::MutexGuard<'static, ()> {
    static ONE: Mutex<()> = Mutex::new(());
    ONE.lock().unwrap_or_else(|e| e.into_inner())
}

/// The one owner of this test binary (the viewer takes one, once).
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
/// an older books dir's.
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

/// Every file under `dir`, with its bytes.
fn files(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            out.extend(files(&p));
        } else {
            out.push((p.clone(), std::fs::read(&p).unwrap()));
        }
    }
    out.sort();
    out
}

fn harness(b: Books) -> Harness<'static, App> {
    let mut h = Harness::builder()
        .with_size(egui::vec2(1200.0, 900.0))
        .build_eframe(|cc| App::with_books(&cc.egui_ctx, b, false, Route::Overview));
    h.run_steps(3);
    h
}

/// Run until the owner was asked `n` things, and the outcome shows.
fn settle(h: &mut Harness<'static, App>, f: &Fake, n: usize) {
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

fn open_menu(h: &mut Harness<'static, App>) {
    h.get_by_label_contains(" Books").click();
    h.run_steps(3);
}

#[test]
fn the_books_menu_hands_each_action_to_the_owner_and_writes_nothing() {
    let _alone = alone();
    let f = fake();
    let (_dir, book, b) = books(true);
    let before = files(&book);
    let mut h = harness(b);
    let start = f.asked.lock().unwrap().len();

    // Back up now: done by the owner; its trouble said, in its words.
    open_menu(&mut h);
    h.get_by_label_contains("Back up now").click();
    settle(&mut h, &f, start + 1);
    assert_eq!(f.asked.lock().unwrap()[start], "back up acme");
    assert!(h.query_by_label_contains("is its disk connected").is_some());

    // Connect Claude Code.
    open_menu(&mut h);
    h.get_by_label_contains("Connect Claude Code").click();
    settle(&mut h, &f, start + 2);
    assert_eq!(f.asked.lock().unwrap()[start + 1], "connect Claude acme");

    // New books: asked for, then made.
    let page_fields = h
        .query_all_by_role(egui::accesskit::Role::TextInput)
        .count();
    open_menu(&mut h);
    h.get_by_label_contains("New books").click();
    h.run_steps(3);
    h.get_all_by_role(egui::accesskit::Role::TextInput)
        .nth(page_fields)
        .unwrap()
        .focus();
    h.run_steps(2);
    h.get_all_by_role(egui::accesskit::Role::TextInput)
        .nth(page_fields)
        .unwrap()
        .type_text("beta");
    h.run_steps(2);
    h.get_by_label("Make them").click();
    settle(&mut h, &f, start + 3);
    assert_eq!(f.asked.lock().unwrap()[start + 2], "new beta USD");

    // Restore: previewed (the backup checked), then done for that plan.
    open_menu(&mut h);
    h.get_by_label_contains("Restore…").click();
    h.run_steps(3);
    h.get_all_by_role(egui::accesskit::Role::TextInput)
        .nth(page_fields)
        .unwrap()
        .focus();
    h.run_steps(2);
    h.get_all_by_role(egui::accesskit::Role::TextInput)
        .nth(page_fields)
        .unwrap()
        .type_text("/media/disk/acme.git");
    h.run_steps(2);
    assert!(
        h.query_by_label("Restore it").is_none(),
        "not before it is checked"
    );
    h.get_by_label("Check it").click();
    settle(&mut h, &f, start + 4);
    assert_eq!(
        f.asked.lock().unwrap()[start + 3],
        "plan /media/disk/acme.git None false"
    );
    assert!(
        h.query_by_label_contains("is whole: through event 7")
            .is_some()
    );
    h.get_by_label("Restore it").click();
    settle(&mut h, &f, start + 5);
    assert_eq!(
        f.asked.lock().unwrap()[start + 4],
        "restore /media/disk/acme.git acme 7"
    );

    // Close the books: asked first; cancelled, nothing; then done.
    open_menu(&mut h);
    h.get_by_label_contains("Close the books").click();
    h.run_steps(3);
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert_eq!(f.asked.lock().unwrap().len(), start + 5);
    open_menu(&mut h);
    h.get_by_label_contains("Close the books").click();
    h.run_steps(3);
    h.get_by_label("Close the books").click();
    settle(&mut h, &f, start + 6);
    assert_eq!(f.asked.lock().unwrap()[start + 5], "close");

    // The viewer wrote nothing of the books.
    drop(h);
    let after: Vec<_> = files(&book)
        .into_iter()
        .filter(|(p, _)| !p.starts_with(book.join("ops")))
        .collect();
    let before: Vec<_> = before
        .into_iter()
        .filter(|(p, _)| !p.starts_with(book.join("ops")))
        .collect();
    assert_eq!(after, before);
}

/// Whether the widget is disabled.
fn disabled(node: &egui_kittest::Node<'_>) -> bool {
    use egui_kittest::kittest::NodeT;
    node.accesskit_node().is_disabled()
}

#[test]
fn nothing_is_changed_while_it_is_at_work() {
    let _alone = alone();
    let f = fake();
    let (_dir, _book, b) = books(true);
    let mut h = harness(b);
    // A backup under way: the menu waits for it.
    let n = f.asked.lock().unwrap().len();
    open_menu(&mut h);
    h.get_by_label_contains("Back up now").click();
    h.run_steps(2);
    assert!(
        disabled(&h.get_by_label_contains(" Books")),
        "the menu works on while busy"
    );
    settle(&mut h, &f, n + 1);
    for _ in 0..50 {
        h.run_steps(2);
        if !disabled(&h.get_by_label_contains(" Books")) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(!disabled(&h.get_by_label_contains(" Books")));
    // A restore being checked: what it asks can't change under it.
    let page_fields = h
        .query_all_by_role(egui::accesskit::Role::TextInput)
        .count();
    open_menu(&mut h);
    h.get_by_label_contains("Restore…").click();
    h.run_steps(3);
    h.get_all_by_role(egui::accesskit::Role::TextInput)
        .nth(page_fields)
        .unwrap()
        .focus();
    h.run_steps(2);
    h.get_all_by_role(egui::accesskit::Role::TextInput)
        .nth(page_fields)
        .unwrap()
        .type_text("/media/disk/acme.git");
    h.run_steps(2);
    h.get_by_label("Check it").click();
    h.run_steps(2);
    for field in h
        .get_all_by_role(egui::accesskit::Role::TextInput)
        .skip(page_fields)
    {
        assert!(
            disabled(&field),
            "a field could change while it was checked"
        );
    }
    let n = f.asked.lock().unwrap().len();
    settle(&mut h, &f, n + 1);
    assert!(h.query_by_label("Restore it").is_some());
    // Edited after the check: checked again before it can be restored.
    h.get_all_by_role(egui::accesskit::Role::TextInput)
        .nth(page_fields)
        .unwrap()
        .focus();
    h.run_steps(2);
    h.get_all_by_role(egui::accesskit::Role::TextInput)
        .nth(page_fields)
        .unwrap()
        .type_text("x");
    h.run_steps(3);
    assert!(h.query_by_label("Restore it").is_none());
    assert!(h.query_by_label("Check it").is_some());
}

#[test]
fn an_older_books_dir_has_no_books_menu() {
    let _alone = alone();
    let _ = fake();
    let (_dir, _book, b) = books(false);
    let h = harness(b);
    assert!(h.query_by_label_contains(" Books").is_none());
}

/// Screenshots of the menu and its dialogs, for eyes: set `SHOT_DIR`.
#[test]
fn screenshots() {
    let Ok(out) = std::env::var("SHOT_DIR") else {
        return;
    };
    let _alone = alone();
    let out = PathBuf::from(out);
    std::fs::create_dir_all(&out).unwrap();
    let f = fake();
    let (_dir, _book, b) = books(true);
    let mut h = harness(b);
    let shot = |h: &mut Harness<'static, App>, name: &str| {
        h.run_steps(3);
        h.render()
            .unwrap()
            .save(out.join(format!("{name}.png")))
            .unwrap();
    };
    open_menu(&mut h);
    shot(&mut h, "menu");
    h.get_by_label_contains("Back up now").click();
    let n = f.asked.lock().unwrap().len();
    settle(&mut h, &f, n + 1);
    shot(&mut h, "backed-up");
    let page_fields = h
        .query_all_by_role(egui::accesskit::Role::TextInput)
        .count();
    open_menu(&mut h);
    h.get_by_label_contains("New books").click();
    shot(&mut h, "new-books");
    h.get_by_label("Cancel").click();
    open_menu(&mut h);
    h.get_by_label_contains("Restore…").click();
    h.run_steps(3);
    h.get_all_by_role(egui::accesskit::Role::TextInput)
        .nth(page_fields)
        .unwrap()
        .focus();
    h.run_steps(2);
    h.get_all_by_role(egui::accesskit::Role::TextInput)
        .nth(page_fields)
        .unwrap()
        .type_text("/media/disk/acme.git");
    shot(&mut h, "restore-ask");
    h.get_by_label("Check it").click();
    let n = f.asked.lock().unwrap().len();
    settle(&mut h, &f, n + 1);
    shot(&mut h, "restore-plan");
    h.get_by_label("Cancel").click();
    open_menu(&mut h);
    h.get_by_label_contains("Close the books").click();
    shot(&mut h, "close");
}
