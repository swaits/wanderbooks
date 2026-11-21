//! The window and the books server it runs (serving.rs, app.rs;
//! plan/p15-app-and-relay.md 3.1; WB-app-2, WB-app-4): a tab is a company
//! open in the server, and closing the window closes the server in view.
//!
//! ```text
//! GOAL: the server is started with the companies the window restored;
//!       a tab opened or closed opens or closes that company in it; what
//!       fails is said in words; closing the window closes the server and
//!       says so while it waits, and the wait is the owner's bounded one.
//! SOURCE: plan/p15-app-and-relay.md 3.1 (a tab is a company open in the
//!         server; the app closes it by closing its input; the close
//!         dialog shows the wait), D-77, D-78, WB-app-2, WB-app-4.
//! INVARIANTS:
//!   1. The server starts once, with the restored companies, and none of
//!      them is asked to open again.
//!   2. A tab opened is `serve_open`, a tab closed is `serve_close`, for
//!      that company, in the order the tabs were opened and closed.
//!   3. A start that failed is a notice, and nothing is asked of the
//!      server after it.
//!   4. A close that wasn't whole, a company that wouldn't open, and a
//!      server that exited on its own are notices, in words.
//!   5. Asked to close, the window stays up saying it is closing the books
//!      until the owner's close has returned; the owner is asked to close
//!      once, however many times it is asked.
//!   6. A window with no owner that hosts a server asks nothing of any.
//! ```

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ledger_core::Currency;
use ledger_gui::data::Source;
use ledger_gui::owner::{Agent, Owner, RestorePlan};
use ledger_gui::serving::Serving;
use ledger_gui::{App, Start};
use ledger_store::root::{BookId, Root};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// A data root of its own, with companies made in it.
struct World {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

impl World {
    fn new(ids: &[&str]) -> World {
        let dir = tempfile::Builder::new()
            .prefix("wb-gui-serving-")
            .tempdir()
            .unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let root = dir.path().join("root");
        Root::setup(&home, Some(&root), None).unwrap();
        let found = Root::find(Some(&root)).unwrap();
        for id in ids {
            drop(
                found
                    .create_book(&BookId::parse(id).unwrap(), Currency::USD)
                    .unwrap(),
            );
        }
        World { _dir: dir, root }
    }

    fn source(&self, id: &str) -> Source {
        Source {
            data_dir: self.root.join("books").join(id),
            today: None,
        }
    }

    /// The kept preferences that restore these companies as tabs.
    fn kept(&self, open: &[&str]) -> HashMap<String, String> {
        HashMap::from([(ledger_gui::app::open_key(&self.root), open.join(","))])
    }

    fn start(&self, open: &[&str], owner: &Arc<Fake>) -> App {
        let kept = self.kept(open);
        let owner: Arc<dyn Owner> = owner.clone();
        App::start_with_owner(
            &egui::Context::default(),
            &|k| kept.get(k).cloned(),
            Start::Restore {
                root: self.root.clone(),
                fallback: self.source(open.first().copied().unwrap_or("alpha")),
            },
            Some(owner),
        )
    }
}

/// An owner that hosts a server of its own, recording what it is asked.
#[derive(Default)]
struct Fake {
    /// What was asked, in order: "start alpha,beta", "open gamma", "close
    /// beta", "stop".
    log: Mutex<Vec<String>>,
    /// What starting says, if not well.
    start_says: Mutex<Option<String>>,
    /// What closing each company says, if not plainly well.
    close_says: Mutex<HashMap<String, Result<Option<String>, String>>>,
    /// What opening each company says, if not well.
    open_says: Mutex<HashMap<String, String>>,
    /// How long the close of the server takes, and what it says after.
    stop_takes: Mutex<Duration>,
    stop_says: Mutex<Option<String>>,
    /// Why the server is gone, once it is.
    exited: Mutex<Option<String>>,
}

impl Fake {
    fn new() -> Arc<Fake> {
        Arc::new(Fake::default())
    }

    fn said(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }

    fn note(&self, what: String) {
        self.log.lock().unwrap().push(what);
    }
}

impl Owner for Fake {
    fn serves(&self) -> bool {
        true
    }
    fn serve_start(&self, open: &[String]) -> Result<(), String> {
        self.note(format!("start {}", open.join(",")));
        match self.start_says.lock().unwrap().clone() {
            Some(why) => Err(why),
            None => Ok(()),
        }
    }
    fn serve_open(&self, book: &str) -> Result<Option<String>, String> {
        self.note(format!("open {book}"));
        match self.open_says.lock().unwrap().remove(book) {
            Some(why) => Err(why),
            None => Ok(None),
        }
    }
    fn serve_close(&self, book: &str) -> Result<Option<String>, String> {
        self.note(format!("close {book}"));
        self.close_says
            .lock()
            .unwrap()
            .remove(book)
            .unwrap_or(Ok(None))
    }
    fn serve_stop(&self) -> Result<Option<String>, String> {
        self.note("stop".into());
        std::thread::sleep(*self.stop_takes.lock().unwrap());
        Ok(self.stop_says.lock().unwrap().clone())
    }
    fn serve_exited(&self) -> Option<String> {
        self.exited.lock().unwrap().clone()
    }
    fn close_books(&self) -> Result<String, String> {
        Err("not here".into())
    }
    fn new_books(&self, _: &str, _: &str) -> Result<String, String> {
        Err("not here".into())
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
    fn connect(&self, _: Agent, _: &str) -> Result<Vec<String>, String> {
        Err("not here".into())
    }
    fn upgrade_format(&self, _: &str, _: &str) -> Result<Vec<String>, String> {
        Err("not here".into())
    }
}

/// Wait until `ok` holds (the requests run on threads of their own).
fn until(what: &str, mut ok: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(30);
    while !ok() {
        assert!(Instant::now() < end, "never: {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// How long to wait for something that must NOT happen.
fn a_while() {
    std::thread::sleep(Duration::from_millis(300));
}

fn tab_of(app: &App, id: &str) -> usize {
    app.views()
        .iter()
        .position(|v| v.book_id().as_deref() == Some(id))
        .unwrap_or_else(|| panic!("no tab for {id}"))
}

#[test]
fn the_companies_the_window_restores_are_the_servers_to_open() {
    let w = World::new(&["alpha", "beta", "gamma"]);
    let owner = Fake::new();
    let app = w.start(&["alpha", "beta"], &owner);
    until("the server started", || !owner.said().is_empty());
    a_while();
    // Started once, with exactly the restored companies open; neither is
    // asked to open again.
    assert_eq!(owner.said(), vec!["start alpha,beta".to_string()]);
    assert_eq!(app.views().len(), 2);
    drop(app);
}

#[test]
fn a_tab_opening_asks_the_server_to_open_that_company() {
    let w = World::new(&["alpha", "gamma"]);
    let owner = Fake::new();
    let ctx = egui::Context::default();
    let mut app = w.start(&["alpha"], &owner);
    app.open(&ctx, w.source("gamma"));
    until("gamma asked", || {
        owner.said().contains(&"open gamma".to_string())
    });
    assert_eq!(owner.said()[0], "start alpha");
    assert!(app.serving().wants("gamma"));
    // A company already open shows its tab and asks nothing more.
    app.open(&ctx, w.source("gamma"));
    a_while();
    assert_eq!(
        owner.said().iter().filter(|s| *s == "open gamma").count(),
        1
    );
    drop(app);
}

#[test]
fn a_tab_closing_asks_the_server_to_close_that_company() {
    let w = World::new(&["alpha", "beta"]);
    let owner = Fake::new();
    let ctx = egui::Context::default();
    let mut app = w.start(&["alpha", "beta"], &owner);
    let beta = tab_of(&app, "beta");
    app.close(beta);
    until("beta closed", || {
        owner.said().contains(&"close beta".to_string())
    });
    assert!(!app.serving().wants("beta"));
    assert!(app.serving().wants("alpha"), "the other stays open");
    // Closed and opened again: a close, then an open, in that order.
    app.open(&ctx, w.source("beta"));
    until("beta opened again", || {
        owner.said().contains(&"open beta".to_string())
    });
    let said = owner.said();
    let close = said.iter().position(|s| s == "close beta").unwrap();
    let open = said.iter().position(|s| s == "open beta").unwrap();
    assert!(close < open, "{said:?}");
    assert!(
        !said.contains(&"close alpha".to_string()),
        "closing one tab closes no other: {said:?}"
    );
    drop(app);
}

#[test]
fn a_close_that_was_not_whole_is_said_to_the_person() {
    let w = World::new(&["alpha", "beta"]);
    let owner = Fake::new();
    owner.close_says.lock().unwrap().insert(
        "beta".into(),
        Ok(Some(
            "beta is closed to your assistants, but its last backup wasn't whole: the backup folder is gone."
                .into(),
        )),
    );
    owner.close_says.lock().unwrap().insert(
        "alpha".into(),
        Err("the books server isn't answering".into()),
    );
    let mut app = w.start(&["alpha", "beta"], &owner);
    let (beta, alpha) = (tab_of(&app, "beta"), tab_of(&app, "alpha"));
    app.close(beta);
    until("a notice about beta", || {
        app.notices().iter().any(|n| n.contains("wasn't whole"))
    });
    // A refused close is said too, naming the company.
    let alpha = if alpha > beta { alpha - 1 } else { alpha };
    app.close(alpha);
    until("a notice about alpha", || {
        app.notices()
            .iter()
            .any(|n| n.contains("alpha could not be closed") && n.contains("isn't answering"))
    });
    // A company that would not open for assistants is said too, and asked
    // for again the next time its tab opens.
    owner.open_says.lock().unwrap().insert(
        "beta".into(),
        "beta couldn't be opened for your assistants: x".into(),
    );
    let ctx = egui::Context::default();
    app.open(&ctx, w.source("beta"));
    until("a notice about the open", || {
        app.notices()
            .iter()
            .any(|n| n.contains("couldn't be opened"))
    });
    until("beta no longer wanted", || !app.serving().wants("beta"));
    drop(app);
}

#[test]
fn a_refused_start_is_said_and_nothing_is_asked_of_the_server() {
    let w = World::new(&["alpha", "gamma"]);
    let owner = Fake::new();
    *owner.start_says.lock().unwrap() = Some("Another program has these books open".into());
    let ctx = egui::Context::default();
    let mut app = w.start(&["alpha"], &owner);
    until("the refusal is said", || {
        app.notices()
            .iter()
            .any(|n| n.contains("Another program has these books open"))
    });
    app.open(&ctx, w.source("gamma"));
    app.close(tab_of(&app, "alpha"));
    a_while();
    assert_eq!(owner.said(), vec!["start alpha".to_string()]);
    drop(app);
}

#[test]
fn a_server_that_exits_on_its_own_is_said_once() {
    let w = World::new(&["alpha"]);
    let owner = Fake::new();
    let mut app = w.start(&["alpha"], &owner);
    until("started", || !owner.said().is_empty());
    assert!(app.notices().is_empty());
    *owner.exited.lock().unwrap() = Some("the books server stopped (exit status: 1)".into());
    let said: Vec<String> = app.notices().to_vec();
    assert_eq!(said.len(), 1, "{said:?}");
    assert!(
        said[0].contains("exit status: 1") && said[0].contains("can't reach your companies"),
        "{said:?}"
    );
    assert_eq!(app.notices().len(), 1, "said once, not every frame");
    drop(app);
}

#[test]
fn closing_the_window_stops_the_server_and_waits_for_it() {
    let w = World::new(&["alpha"]);
    let owner = Fake::new();
    *owner.stop_takes.lock().unwrap() = Duration::from_millis(600);
    let kept = w.kept(&["alpha"]);
    let (root, fallback) = (w.root.clone(), w.source("alpha"));
    let for_app: Arc<dyn Owner> = owner.clone();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1000.0, 700.0))
        .build_eframe(move |cc| {
            App::start_with_owner(
                &cc.egui_ctx,
                &|k| kept.get(k).cloned(),
                Start::Restore { root, fallback },
                Some(for_app),
            )
        });
    h.run_steps(3);
    // The window is asked to close.
    h.input_mut()
        .viewports
        .entry(egui::ViewportId::ROOT)
        .or_default()
        .events
        .push(egui::ViewportEvent::Close);
    h.run_steps(2);
    until("the close began", || h.state().serving().is_closing());
    h.run_steps(2);
    assert!(
        h.query_by_label_contains("Closing your books").is_some(),
        "the window says it is closing the books while it waits"
    );
    assert!(!h.state().serving().is_closed(), "the wait is not over");
    // Asked again meanwhile: the owner is still asked once.
    h.input_mut()
        .viewports
        .entry(egui::ViewportId::ROOT)
        .or_default()
        .events
        .push(egui::ViewportEvent::Close);
    h.run_steps(2);
    until("the close is done", || h.state().serving().is_closed());
    h.run_steps(3);
    assert!(
        h.query_by_label_contains("Closing your books").is_none(),
        "the message goes when the wait does"
    );
    assert_eq!(
        owner.said().iter().filter(|s| *s == "stop").count(),
        1,
        "{:?}",
        owner.said()
    );
    drop(h);
    assert_eq!(
        owner.said().iter().filter(|s| *s == "stop").count(),
        1,
        "dropping the window asks no second time: {:?}",
        owner.said()
    );
}

#[test]
fn a_server_that_will_not_exit_is_left_closing_and_the_window_closes() {
    let w = World::new(&["alpha"]);
    let owner = Fake::new();
    *owner.stop_says.lock().unwrap() = Some("The books are still closing in the background".into());
    let serving = Serving::start(Some(owner.clone() as Arc<dyn Owner>), vec!["alpha".into()]);
    until("started", || !owner.said().is_empty());
    serving.begin_stop();
    until("the close returned", || serving.is_closed());
    // The wait ended with the owner's word; the window can close, and the
    // person is told what is left.
    let notices = serving.take_notices();
    assert!(
        notices.iter().any(|n| n.contains("still closing")),
        "{notices:?}"
    );
    drop(serving);
    drop(w);
}

#[test]
fn a_window_with_no_server_asks_nothing_of_any() {
    let w = World::new(&["alpha", "gamma"]);
    let ctx = egui::Context::default();
    // An owner that does not host a server: the default.
    struct Plain;
    impl Owner for Plain {
        fn close_books(&self) -> Result<String, String> {
            Err("not here".into())
        }
        fn new_books(&self, _: &str, _: &str) -> Result<String, String> {
            Err("not here".into())
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
        fn connect(&self, _: Agent, _: &str) -> Result<Vec<String>, String> {
            Err("not here".into())
        }
        fn upgrade_format(&self, _: &str, _: &str) -> Result<Vec<String>, String> {
            Err("not here".into())
        }
    }
    let kept = w.kept(&["alpha"]);
    let mut app = App::start_with_owner(
        &ctx,
        &|k| kept.get(k).cloned(),
        Start::Restore {
            root: w.root.clone(),
            fallback: w.source("alpha"),
        },
        Some(Arc::new(Plain)),
    );
    assert!(!app.serving().is_on());
    app.open(&ctx, w.source("gamma"));
    app.close(0);
    assert!(app.notices().is_empty());
    assert!(app.serving().is_closed(), "nothing to close");
    // And no owner at all.
    let app = App::start_with_owner(
        &ctx,
        &|k| kept.get(k).cloned(),
        Start::Restore {
            root: w.root.clone(),
            fallback: w.source("alpha"),
        },
        None,
    );
    assert!(!app.serving().is_on());
}
