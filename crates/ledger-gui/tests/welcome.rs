//! The first-run page (welcome.rs, plan P11b): with no books yet, the viewer
//! sets up the private folder, asks for the owner passphrase, makes the first
//! books, connects an agent and opens the books, each through the owner and
//! none needing a terminal.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ledger_core::Currency;
use ledger_gui::App;
use ledger_gui::owner::{Agent, FirstRun, Owner, RestorePlan};
use ledger_store::root::{BookId, Root};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

/// An owner over a real data root in a scratch folder, that records what it
/// was asked (the first run's steps), and refuses what it should.
#[derive(Default)]
struct Fake {
    place: Mutex<Option<(PathBuf, PathBuf)>>,
    asked: Mutex<Vec<String>>,
}

impl Fake {
    fn reset(&self, home: PathBuf, root: PathBuf) {
        *self.place.lock().unwrap() = Some((home, root));
        self.asked.lock().unwrap().clear();
    }
    fn said(&self, what: String) {
        self.asked.lock().unwrap().push(what);
    }
    fn root_path(&self) -> (PathBuf, PathBuf) {
        self.place.lock().unwrap().clone().unwrap()
    }
}

impl Owner for Fake {
    fn close_books(&self) -> Result<String, String> {
        Err("not here".into())
    }
    fn new_books(&self, id: &str, currency: &str) -> Result<String, String> {
        self.said(format!("new {id} {currency}"));
        let (_, root) = self.root_path();
        let root = Root::find(Some(&root)).map_err(|e| e.to_string())?;
        let id = BookId::parse(id).map_err(|e| e.to_string())?;
        root.create_book(&id, Currency::new(currency).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        Ok(format!("Made new books \"{id}\" ({currency})."))
    }
    fn back_up_now(&self, _: &str) -> Result<Vec<String>, String> {
        Err("not here".into())
    }
    fn plan_restore(&self, _: &str, _: Option<String>, _: bool) -> Result<RestorePlan, String> {
        Err("not here".into())
    }
    fn restore(&self, _: &RestorePlan) -> Result<Vec<String>, String> {
        Err("not here".into())
    }
    fn connect(&self, agent: Agent, book: &str) -> Result<Vec<String>, String> {
        self.said(format!("connect {agent:?} {book}"));
        match agent {
            Agent::Claude => Ok(vec!["Claude Code reaches the books.".into()]),
            Agent::Codex => Err("Codex isn't installed here".into()),
        }
    }
    fn upgrade_format(&self, _: &str, _: &str) -> Result<Vec<String>, String> {
        Err("not here".into())
    }
    fn first_run(&self) -> FirstRun {
        let (_, root) = self.root_path();
        let Ok(found) = Root::find(Some(&root)) else {
            return FirstRun::default();
        };
        FirstRun {
            root: true,
            books: found
                .list_books()
                .unwrap()
                .iter()
                .map(|b| b.as_str().to_string())
                .collect(),
            passphrase_set: ledger_store::passphrase::is_set(&found.auth_dir().unwrap()).unwrap(),
        }
    }
    fn set_up(&self) -> Result<Vec<String>, String> {
        self.said("set up".into());
        let (home, root) = self.root_path();
        Root::setup(&home, Some(&root), None).map_err(|e| e.to_string())?;
        Ok(vec!["Made the books' private folder.".into()])
    }
    fn set_passphrase(&self, new: &str) -> Result<(), String> {
        self.said(format!("passphrase of {} characters", new.chars().count()));
        let (_, root) = self.root_path();
        let found = Root::find(Some(&root)).map_err(|e| e.to_string())?;
        ledger_store::passphrase::set(&found.auth_dir().unwrap(), new, None)
            .map_err(|e| e.to_string())
    }
    fn hold(&self, book: &str) {
        self.said(format!("hold {book}"));
    }
}

/// One test at a time: they share the one owner.
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

fn harness(root: PathBuf, state: FirstRun) -> Harness<'static, App> {
    let mut h = Harness::builder()
        .with_size(egui::vec2(1000.0, 1100.0))
        .build_eframe(move |cc| {
            App::first_run(
                cc,
                Some(root.clone()),
                chrono::NaiveDate::from_ymd_opt(2026, 10, 15),
                state.clone(),
            )
        });
    h.run_steps(3);
    h
}

/// Run until `f` holds (a step's worker answered).
fn until(h: &mut Harness<'static, App>, what: &str, f: impl Fn(&Harness<'static, App>) -> bool) {
    for _ in 0..300 {
        h.run_steps(2);
        if f(h) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    shot(h, "welcome-stuck");
    panic!("never: {what}");
}

fn shot(h: &mut Harness<'static, App>, name: &str) {
    if let Ok(out) = std::env::var("SHOT_DIR") {
        let out = PathBuf::from(out);
        std::fs::create_dir_all(&out).unwrap();
        h.render()
            .unwrap()
            .save(out.join(format!("{name}.png")))
            .unwrap();
    }
}

fn scratch() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let dir = tempfile::Builder::new()
        .prefix("wb-gui-welcome-")
        .tempdir()
        .unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let root = dir.path().join("data");
    (dir, home, root)
}

#[test]
fn first_run_onboards_with_no_terminal() {
    let _alone = alone();
    let f = fake();
    let (_dir, home, root) = scratch();
    f.reset(home, root.clone());
    let mut h = harness(root.clone(), FirstRun::default());
    shot(&mut h, "welcome-1");

    // 1. The private folder: set up from the page.
    h.get_by_label("Set it up").click();
    until(&mut h, "the folder is made", |h| {
        h.query_by_label_contains("Made. Only your account")
            .is_some()
    });
    assert!(h.state().welcome().unwrap().state().root);

    // 2. The passphrase: typed twice, handed to the owner, gone from the page.
    assert_eq!(
        h.query_all_by_role(egui::accesskit::Role::PasswordInput)
            .count(),
        2,
        "two password fields"
    );
    for i in 0..2 {
        h.get_all_by_role(egui::accesskit::Role::PasswordInput)
            .nth(i)
            .unwrap()
            .focus();
        h.run_steps(2);
        h.get_all_by_role(egui::accesskit::Role::PasswordInput)
            .nth(i)
            .unwrap()
            .type_text("a long enough phrase");
        h.run_steps(2);
    }
    shot(&mut h, "welcome-2");
    h.get_by_label("Set passphrase").click();
    until(&mut h, "the passphrase is set", |h| {
        h.state().welcome().unwrap().state().passphrase_set
    });

    // 3. The first books.
    h.get_all_by_role(egui::accesskit::Role::TextInput)
        .next()
        .unwrap()
        .focus();
    h.run_steps(2);
    h.get_all_by_role(egui::accesskit::Role::TextInput)
        .next()
        .unwrap()
        .type_text("acme");
    h.run_steps(2);
    h.get_by_label("Make the books").click();
    until(&mut h, "the books are made", |h| {
        h.state().welcome().unwrap().state().books == ["acme"]
    });
    shot(&mut h, "welcome-3");

    // 4. An agent: Claude Code connects; Codex isn't installed, and says so.
    h.get_by_label("Connect Codex").click();
    until(&mut h, "Codex is refused", |h| {
        h.query_all_by_label_contains("Codex isn't installed here")
            .next()
            .is_some()
    });
    h.get_by_label("Connect Claude Code").click();
    until(&mut h, "Claude Code is connected", |h| {
        h.query_all_by_label_contains("Claude Code reaches the books")
            .next()
            .is_some()
    });
    shot(&mut h, "welcome-4");

    // 5. Open the books: the viewer shows them.
    h.get_by_label_contains("Open the books").click();
    until(&mut h, "the books are shown", |h| {
        h.query_by_label_contains("Needs attention").is_some()
    });
    assert!(h.state().welcome().is_none(), "the page is gone");
    shot(&mut h, "welcome-5");

    // What the owner was asked, in order, and nothing else.
    let asked = f.asked.lock().unwrap().clone();
    assert_eq!(
        asked,
        [
            "set up",
            "passphrase of 20 characters",
            "new acme USD",
            "hold acme",
            "connect Codex acme",
            "connect Claude acme",
        ]
    );
}

#[test]
fn first_run_starts_at_the_first_step_missing_and_asks_nothing_done_again() {
    let _alone = alone();
    let f = fake();
    let (_dir, home, root) = scratch();
    f.reset(home.clone(), root.clone());
    // Done at a terminal already: the folder and the passphrase.
    f.set_up().unwrap();
    f.set_passphrase("a long enough phrase").unwrap();
    f.asked.lock().unwrap().clear();
    let state = f.first_run();
    assert_eq!(
        state,
        FirstRun {
            root: true,
            books: vec![],
            passphrase_set: true
        }
    );
    let h = harness(root, state);
    // No folder or passphrase to ask for; the books are.
    assert!(h.query_by_label("Set it up").is_none());
    assert!(h.query_by_label("Set passphrase").is_none());
    h.get_by_label("Make the books");
    assert!(f.asked.lock().unwrap().is_empty());
}

#[test]
fn a_passphrase_too_short_or_not_repeated_is_not_sent_and_can_wait() {
    let _alone = alone();
    let f = fake();
    let (_dir, home, root) = scratch();
    f.reset(home, root.clone());
    f.set_up().unwrap();
    f.asked.lock().unwrap().clear();
    let mut h = harness(root, f.first_run());
    h.get_all_by_role(egui::accesskit::Role::PasswordInput)
        .next()
        .unwrap()
        .focus();
    h.run_steps(2);
    h.get_all_by_role(egui::accesskit::Role::PasswordInput)
        .next()
        .unwrap()
        .type_text("short");
    h.run_steps(3);
    h.get_by_label_contains("characters or more, please");
    // Too short: the button does nothing.
    h.get_by_label("Set passphrase").click();
    h.run_steps(3);
    assert!(f.asked.lock().unwrap().is_empty());
    // Not now: the books are asked for next, and nothing was sent.
    h.get_by_label("Not now").click();
    h.run_steps(3);
    h.get_by_label("Make the books");
    assert!(f.asked.lock().unwrap().is_empty());
    assert!(!f.first_run().passphrase_set);
}
