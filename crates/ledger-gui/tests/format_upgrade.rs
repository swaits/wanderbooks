//! The viewer and the format upgrade (owner.rs INVARIANT 5, plan 3.9,
//! D-39): books waiting for one show a standing notice above every page,
//! and the go-ahead is asked as the viewer opens them, with the owner
//! passphrase, handed to the owner the program gives the viewer (the
//! server checks it). "Not now" changes nothing; a refusal is said in the
//! dialog, which stays; books this version made, and older books dirs,
//! are never asked; the viewer writes nothing of the books.

use chrono::NaiveDate;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ledger_core::Currency;
use ledger_gui::App;
use ledger_gui::books::Books;
use ledger_gui::data::{self, Source};
use ledger_gui::nav::Route;
use ledger_gui::owner::{Agent, Owner, RestorePlan};
use ledger_store::{JournalFormat, Store};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

/// The owner passphrase the fake owner takes.
const PASS: &str = "correct horse battery";
/// The passphrase on which the fake owner's worker dies, answering nothing.
const DIES: &str = "the worker dies on this one";

/// An owner that says what it was asked, and takes only PASS.
#[derive(Default)]
struct Fake {
    asked: Mutex<Vec<String>>,
}

impl Owner for Fake {
    fn close_books(&self) -> Result<String, String> {
        Err("not in these tests".into())
    }
    fn new_books(&self, _id: &str, _currency: &str) -> Result<String, String> {
        Err("not in these tests".into())
    }
    fn back_up_now(&self, _book: &str) -> Result<Vec<String>, String> {
        Err("not in these tests".into())
    }
    fn plan_restore(
        &self,
        _source: &str,
        _id: Option<String>,
        _replace: bool,
    ) -> Result<RestorePlan, String> {
        Err("not in these tests".into())
    }
    fn restore(&self, _plan: &RestorePlan) -> Result<Vec<String>, String> {
        Err("not in these tests".into())
    }
    fn connect(&self, _agent: Agent, _book: &str) -> Result<Vec<String>, String> {
        Err("not in these tests".into())
    }
    fn upgrade_format(&self, book: &str, passphrase: &str) -> Result<Vec<String>, String> {
        // Slow enough to be seen at work.
        std::thread::sleep(std::time::Duration::from_millis(300));
        // A worker that dies on the way: no answer comes back.
        assert_ne!(passphrase, DIES, "the owner's worker died");
        self.asked
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(format!("upgrade {book} {passphrase}"));
        if passphrase == PASS {
            Ok(vec![
                "Backed up first, and proven whole with the vault and keys: the books as they were, through event 7 (backup 0123456789ab).".into(),
                format!("Upgraded the books \"{book}\" to this version's journal format (journal format 2): now at event 8 (c4d3e2f1a0b9)."),
            ])
        } else {
            Err("that isn't the owner passphrase: nothing was changed".into())
        }
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

fn asked(f: &Fake) -> Vec<String> {
    f.asked.lock().unwrap().clone()
}

/// Books `acme`: in a data root (`root`) or an older books dir, in journal
/// format 1 (as an older version wrote them) or this version's (`old`).
fn books(root: bool, old: bool) -> (tempfile::TempDir, PathBuf, Books) {
    let dir = tempfile::Builder::new()
        .prefix("wb-gui-upgrade-")
        .tempdir()
        .unwrap();
    let book = if root {
        std::fs::write(dir.path().join(ledger_store::root::MARKER_FILE), b"").unwrap();
        dir.path().join("books").join("acme")
    } else {
        dir.path().join("books-data")
    };
    std::fs::create_dir_all(&book).unwrap();
    let format = if old {
        JournalFormat::default()
    } else {
        ledger_store::format::current()
    };
    drop(Store::create_in_format(&book, Currency::USD, &format).unwrap());
    let b = load(&book);
    assert_eq!(b.format_upgrade.is_some(), old);
    (dir, book, b)
}

/// The books in `book`, as the viewer loads them.
fn load(book: &Path) -> Books {
    data::load_once(&Source {
        data_dir: book.to_path_buf(),
        today: Some(NaiveDate::from_ymd_opt(2026, 10, 20).unwrap()),
    })
    .unwrap()
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
    h.run_steps(4);
    h
}

const ASKS: &str = "Upgrade these books' format?";
const NOTICE: &str = "These books wait for a format upgrade.";

/// Type into the dialog's passphrase field (the last text field).
fn type_passphrase(h: &mut Harness<'static, App>, text: &str) {
    let field = h
        .get_all_by_role(egui::accesskit::Role::PasswordInput)
        .last()
        .expect("a passphrase field");
    field.focus();
    h.run_steps(2);
    h.get_all_by_role(egui::accesskit::Role::PasswordInput)
        .last()
        .unwrap()
        .type_text(text);
    h.run_steps(2);
}

/// Run until the owner was asked `n` things, and the outcome shows.
fn settle(h: &mut Harness<'static, App>, f: &Fake, n: usize) {
    for _ in 0..300 {
        h.run_steps(2);
        if asked(f).len() >= n {
            h.run_steps(6);
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("the owner wasn't asked: {:?}", asked(f));
}

/// D-39: asked as the viewer opens the books, and at every open until
/// given; "Not now" changes nothing and leaves the standing notice, whose
/// button asks again; nothing of the books is written.
#[test]
fn the_go_ahead_is_asked_at_every_open_with_a_standing_notice() {
    let _alone = alone();
    let f = fake();
    let (_dir, book, b) = books(true, true);
    let before = files(&book);
    let n = asked(&f).len();
    let mut h = harness(b);
    assert!(h.query_by_label(ASKS).is_some(), "asked as they open");
    assert!(
        h.query_by_label_contains(NOTICE).is_some(),
        "the standing notice"
    );
    assert!(h.query_by_label("Upgrade").is_some());
    assert!(
        h.query_by_label_contains("journal format 1. The upgrade adds journal format 2")
            .is_some()
    );
    h.get_by_label("Not now").click();
    h.run_steps(4);
    assert!(h.query_by_label(ASKS).is_none(), "not now: gone");
    assert!(
        h.query_by_label_contains(NOTICE).is_some(),
        "the notice stays"
    );
    h.run_steps(10);
    assert!(h.query_by_label(ASKS).is_none(), "asked once each open");
    // The notice's button asks again.
    h.get_by_label("Upgrade…").click();
    h.run_steps(4);
    assert!(h.query_by_label(ASKS).is_some());
    h.get_by_label("Not now").click();
    h.run_steps(4);
    // So does the Books menu.
    h.get_by_label_contains(" Books").click();
    h.run_steps(3);
    h.get_by_label_contains("Upgrade the books' format").click();
    h.run_steps(4);
    assert!(h.query_by_label(ASKS).is_some());
    drop(h);
    // Opened again: asked again.
    let h = harness(load(&book));
    assert!(h.query_by_label(ASKS).is_some(), "asked at the next open");
    drop(h);
    assert_eq!(asked(&f).len(), n, "declining asked the owner nothing");
    let after: Vec<_> = files(&book)
        .into_iter()
        .filter(|(p, _)| !p.starts_with(book.join("ops")))
        .collect();
    let before: Vec<_> = before
        .into_iter()
        .filter(|(p, _)| !p.starts_with(book.join("ops")))
        .collect();
    assert_eq!(after, before, "the viewer wrote nothing");
}

/// The go-ahead: nothing is sent without a passphrase; the one typed
/// goes to the owner as typed (which the server checks); a refusal is said
/// in the dialog, which stays, its field emptied; the right one closes it
/// and what was done is said.
#[test]
fn the_passphrase_goes_to_the_owner_and_a_refusal_is_said() {
    let _alone = alone();
    let f = fake();
    let (_dir, _book, b) = books(true, true);
    let mut h = harness(b);
    let n = asked(&f).len();
    h.get_by_label("Upgrade").click();
    h.run_steps(4);
    assert_eq!(asked(&f).len(), n, "nothing typed, nothing sent");
    type_passphrase(&mut h, "not the passphrase at all");
    h.get_by_label("Upgrade").click();
    h.run_steps(2);
    assert!(
        h.query_by_label_contains("Backing up, then upgrading")
            .is_some()
    );
    settle(&mut h, &f, n + 1);
    assert_eq!(asked(&f)[n], "upgrade acme not the passphrase at all");
    assert!(h.query_by_label(ASKS).is_some(), "the dialog stays");
    assert!(
        h.query_by_label("that isn't the owner passphrase: nothing was changed")
            .is_some()
    );
    type_passphrase(&mut h, PASS);
    h.get_by_label("Upgrade").click();
    settle(&mut h, &f, n + 2);
    assert_eq!(asked(&f)[n + 1], format!("upgrade acme {PASS}"));
    assert!(h.query_by_label(ASKS).is_none(), "done: the dialog closes");
    assert!(
        h.query_by_label_contains("Format upgrade: Backed up first")
            .is_some()
    );
}

/// An upgrade whose worker ends without an answer (it died on the way) is
/// said in the dialog, which can then be tried again or closed: the
/// dialog, which holds the whole window, is never left at work for good.
#[test]
fn an_upgrade_that_ends_without_an_answer_never_holds_the_window() {
    let _alone = alone();
    let _ = fake();
    let (_dir, _book, b) = books(true, true);
    let mut h = harness(b);
    type_passphrase(&mut h, DIES);
    h.get_by_label("Upgrade").click();
    h.run_steps(2);
    assert!(
        h.query_by_label_contains("Backing up, then upgrading")
            .is_some()
    );
    let said = "it ended without an answer";
    for _ in 0..300 {
        h.run_steps(2);
        if h.query_by_label_contains(said).is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    // The dialog grew by what it says: settled where it now stands before
    // anything in it is clicked (as `settle` does).
    h.run_steps(6);
    assert!(h.query_by_label_contains(said).is_some(), "said");
    assert!(
        h.query_by_label_contains("Backing up, then upgrading")
            .is_none(),
        "no longer at work"
    );
    assert!(h.query_by_label(ASKS).is_some(), "the dialog stays");
    h.get_by_label("Not now").click();
    h.run_steps(4);
    assert!(h.query_by_label(ASKS).is_none(), "and closes");
}

/// Books this version made wait for nothing, and older books dirs (no
/// owner's server for them) are never asked: no notice, no dialog, no
/// menu item.
#[test]
fn books_in_this_versions_format_and_older_books_dirs_are_never_asked() {
    let _alone = alone();
    let _ = fake();
    for (root, old) in [(true, false), (false, true), (false, false)] {
        let (_dir, _book, b) = books(root, old);
        let mut h = harness(b);
        assert!(h.query_by_label(ASKS).is_none(), "{root} {old}");
        assert!(h.query_by_label_contains(NOTICE).is_none(), "{root} {old}");
        if root {
            h.get_by_label_contains(" Books").click();
            h.run_steps(3);
            assert!(
                h.query_by_label_contains("Upgrade the books' format")
                    .is_none()
            );
        }
    }
}

/// Screenshots of the notice and the dialog, for eyes: set `SHOT_DIR`.
#[test]
fn screenshots() {
    let Ok(out) = std::env::var("SHOT_DIR") else {
        return;
    };
    let _alone = alone();
    let out = PathBuf::from(out);
    std::fs::create_dir_all(&out).unwrap();
    let f = fake();
    let (_dir, _book, b) = books(true, true);
    let mut h = harness(b);
    let shot = |h: &mut Harness<'static, App>, name: &str| {
        h.run_steps(3);
        h.render()
            .unwrap()
            .save(out.join(format!("{name}.png")))
            .unwrap();
    };
    shot(&mut h, "upgrade-ask");
    type_passphrase(&mut h, "not the passphrase at all");
    let n = asked(&f).len();
    h.get_by_label("Upgrade").click();
    h.run_steps(2);
    shot(&mut h, "upgrade-working");
    settle(&mut h, &f, n + 1);
    shot(&mut h, "upgrade-refused");
    type_passphrase(&mut h, DIES);
    h.get_by_label("Upgrade").click();
    for _ in 0..300 {
        h.run_steps(2);
        if h.query_by_label_contains("ended without an answer")
            .is_some()
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    shot(&mut h, "upgrade-no-answer");
    h.get_by_label("Not now").click();
    shot(&mut h, "upgrade-notice");
    h.get_by_label_contains(" Books").click();
    shot(&mut h, "upgrade-menu");
    h.get_by_label_contains("Upgrade the books' format").click();
    h.run_steps(3);
    type_passphrase(&mut h, PASS);
    let n = asked(&f).len();
    h.get_by_label("Upgrade").click();
    settle(&mut h, &f, n + 1);
    shot(&mut h, "upgrade-done");
}
