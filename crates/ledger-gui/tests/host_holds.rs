//! A start the app's server was refused because a host holds the books (an
//! earlier build's detached host, or `wanderbooks serve`'s): the notice
//! carries a button that closes it, and the line to type is its fallback
//! (serving.rs INVARIANT 3; WB-app-2).
//!
//! ```text
//! GOAL: a person with no command line on their path can let go of a
//!       host that holds the books, from the notice, and the app then
//!       starts its own server.
//! SOURCE: plan/p15-app-and-relay.md 3.1; WB-app-1 (a start is refused,
//!         in words, when another program has the books), WB-app-2.
//! INVARIANTS:
//!   1. A start refused while a host holds the books is a notice that
//!      says what will happen and what to type instead; the button is
//!      offered (`host_blocks`).
//!   2. The button asks the host to close (`serve_close_host`) and, once
//!      it has, starts the server and takes the notice away.
//!   3. A close that fails says why and offers the button again.
//!   4. A start refused for any other reason offers no such button.
//! ```

use ledger_core::Currency;
use ledger_gui::owner::{Agent, Owner, RestorePlan};
use ledger_gui::serving::{HOST_HOLDS, Serving};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// An owner whose start is refused while `host` holds the books.
struct Held {
    host: AtomicBool,
    /// Whether a host is what refused the start (else something else did).
    host_is_why: bool,
    close_says: Mutex<Result<String, String>>,
    log: Mutex<Vec<String>>,
}

impl Held {
    fn new(host_is_why: bool) -> Arc<Held> {
        Arc::new(Held {
            host: AtomicBool::new(true),
            host_is_why,
            close_says: Mutex::new(Ok("The other program let go of the books.".into())),
            log: Mutex::new(Vec::new()),
        })
    }

    fn said(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }
}

impl Owner for Held {
    fn serves(&self) -> bool {
        true
    }
    fn serve_start(&self, _open: &[String]) -> Result<(), String> {
        self.log.lock().unwrap().push("start".into());
        if self.host.load(Ordering::SeqCst) {
            Err("Another program has these books open: close it".into())
        } else {
            Ok(())
        }
    }
    fn serve_host_holds(&self) -> bool {
        self.host_is_why && self.host.load(Ordering::SeqCst)
    }
    fn serve_close_host(&self) -> Result<String, String> {
        self.log.lock().unwrap().push("close host".into());
        let said = self.close_says.lock().unwrap().clone();
        if said.is_ok() {
            self.host.store(false, Ordering::SeqCst);
        }
        said
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

fn until(what: &str, mut ok: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(30);
    while !ok() {
        assert!(Instant::now() < end, "never: {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn started(owner: &Arc<Held>) -> Serving {
    let _ = Currency::USD;
    let serving = Serving::start(Some(owner.clone() as Arc<dyn Owner>), vec![]);
    until("the first start came out", || !serving.is_starting());
    serving
}

#[test]
fn a_start_refused_by_a_host_says_what_the_button_does_and_the_command_to_type() {
    let owner = Held::new(true);
    let serving = started(&owner);
    assert!(serving.host_blocks());
    let notices = serving.take_notices();
    assert_eq!(notices, vec![HOST_HOLDS.to_string()]);
    assert!(
        HOST_HOLDS.contains("Close it") && HOST_HOLDS.contains("`wanderbooks close`"),
        "{HOST_HOLDS}"
    );
}

#[test]
fn the_button_closes_the_host_and_starts_the_server() {
    let owner = Held::new(true);
    let serving = started(&owner);
    // Notices the window shows are kept by it: what it has not shown yet
    // is read here, as it does.
    let mut shown = serving.take_notices();
    assert!(serving.close_host());
    until("the server started after the host closed", || {
        serving.is_up()
    });
    assert_eq!(owner.said(), ["start", "close host", "start"]);
    assert!(!serving.host_blocks());
    shown.extend(serving.take_notices());
    // The refusal is over, and the notice of it goes (a window that
    // already shows it drops it as it learns the start came out).
    assert!(!serving.is_down());
}

#[test]
fn a_close_that_fails_says_why_and_the_button_stays() {
    let owner = Held::new(true);
    *owner.close_says.lock().unwrap() = Err("the other program is still closing".into());
    let serving = started(&owner);
    let _ = serving.take_notices();
    assert!(serving.close_host());
    until("the failure is said", || {
        serving
            .take_notices()
            .iter()
            .any(|n| n.contains("Couldn't close it: the other program is still closing"))
    });
    until("the button is back", || serving.host_blocks());
    assert!(serving.is_down());
    assert_eq!(owner.said(), ["start", "close host"]);
}

#[test]
fn a_start_refused_for_another_reason_offers_no_button() {
    let owner = Held::new(false);
    let serving = started(&owner);
    assert!(!serving.host_blocks());
    assert!(!serving.close_host());
    let notices = serving.take_notices();
    assert_eq!(notices.len(), 1, "{notices:?}");
    assert!(notices[0].starts_with("Assistants can't reach your companies:"));
    assert_eq!(owner.said(), ["start"]);
}

/// The window itself: the notice shows the button, pressing it asks the
/// host to close, and the notice goes once the server has started.
#[test]
fn the_notice_in_the_window_has_the_button_and_it_goes_when_the_server_starts() {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    use ledger_gui::{App, Start};
    use ledger_store::root::{BookId, Root};
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let root = dir.path().join("root");
    Root::setup(&home, Some(&root), None).unwrap();
    let found = Root::find(Some(&root)).unwrap();
    drop(
        found
            .create_book(&BookId::parse("alpha").unwrap(), Currency::USD)
            .unwrap(),
    );
    let fallback = ledger_gui::data::Source {
        data_dir: root.join("books").join("alpha"),
        today: None,
    };
    let owner = Held::new(true);
    let for_app: Arc<dyn Owner> = owner.clone();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1000.0, 700.0))
        .build_eframe(move |cc| {
            App::start_with_owner(
                &cc.egui_ctx,
                &|_| None,
                Start::Restore { root, fallback },
                Some(for_app),
            )
        });
    until("the start was refused", || {
        h.run_steps(2);
        h.query_all_by_label_contains("`wanderbooks close`")
            .next()
            .is_some()
    });
    h.query_all_by_label("Close it")
        .next()
        .expect("the button")
        .click();
    until("the host was asked to close and the server started", || {
        h.run_steps(2);
        {
            // (A tab opening after the refused start tried it again once.)
            let said = owner.said();
            said.iter().filter(|s| *s == "close host").count() == 1
                && said.last().is_some_and(|s| s == "start")
        }
    });
    until("the notice goes", || {
        h.run_steps(2);
        h.query_all_by_label_contains("`wanderbooks close`")
            .next()
            .is_none()
            && h.query_all_by_label("Close it").next().is_none()
    });
}
