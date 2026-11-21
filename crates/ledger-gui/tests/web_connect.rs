//! The web half of the Connect page (plan P15 unit B3; WB-assist-6,
//! WB-app-6): the switch, the progress in plain words, the address with
//! Copy, the steps, every state the page can be in, and the page opening
//! by itself, once, for a person who has never connected an assistant.
//!
//! ```text
//! GOAL: a person with no technical background turns web assistants on,
//!       watches it get an address and a certificate in words, copies the
//!       address, follows steps that were tried (and is told plainly where
//!       none were), sees what is wrong in words with nothing lost
//!       locally, turns it off, and can get a new address after being
//!       asked; and a first run shows the Connect page once.
//! SOURCE: plan/p15-app-and-relay.md 3.3 ("Allow web assistants" per
//!         company, off by default; the address with Copy; the exact steps
//!         for Claude and for ChatGPT; the relay unreachable in words, with
//!         nothing lost locally; turning off keeps the name; a new address
//!         behind a confirmation), 5 (the plan's research: ChatGPT's two
//!         pages disagree, so no steps are written); WB-assist-6,
//!         WB-app-6.
//! INVARIANTS:
//!   1. Off by default: the switch is shown off, with no address and no
//!      steps, and an owner with no web assistants (`web_state` none)
//!      shows no switch at all.
//!   2. The switch calls the owner and nothing else; the state is the
//!      owner's, read every frame.
//!   3. The steps for Claude are exactly the three in `claude_steps`;
//!      ChatGPT and any other assistant are told "we haven't tested this
//!      yet", never given steps nobody tried.
//!   4. A problem that mends itself says the window and the local
//!      assistants are not affected; one that needs the person does not
//!      say that and says what to do.
//!   5. A new address is asked for twice: the first click only asks.
//!   6. The Connect page opens by itself once, when the window's server
//!      says no assistant was ever connected, and never again; with any
//!      key, session, approval or waiting request it does not open.
//!   7. Once it is connected, the three progress steps are one line,
//!      "Connected"; they are shown while connecting and when one is the
//!      trouble. The page does not say that nobody can read the traffic:
//!      the relay cannot, the assistant's own company reads what it is
//!      sent.
//! SCENARIOS:
//!   1. See the tests below; `web_screenshots` (SHOT_DIR) draws each state
//!      light and dark.
//! ```

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use ledger_core::{AccountType, Currency};
use ledger_gui::connect::{CHATGPT_NOTE, OTHER_NOTE, Progress, claude_steps, web_progress};
use ledger_gui::data::Source;
use ledger_gui::nav::Route;
use ledger_gui::owner::{Owner, RestorePlan, Roster, TokenRow, WebConnect, WebStep};
use ledger_gui::{App, Start};
use ledger_store::root::{BookId, Root};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const CONNECTOR: &str =
    "https://k3j9x0q7m2z8c5v1b4n6a9s2d.ledger-accrual-tally-mint.wanderbooks.link/b/alpha/mcp";

struct World {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

impl World {
    fn new(companies: &[(&str, &str)]) -> World {
        let dir = tempfile::Builder::new()
            .prefix("wb-gui-web-")
            .tempdir()
            .unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let root = dir.path().join("root");
        Root::setup(&home, Some(&root), None).unwrap();
        let found = Root::find(Some(&root)).unwrap();
        for (id, name) in companies {
            let mut s = found
                .create_book(&BookId::parse(id).unwrap(), Currency::USD)
                .unwrap();
            s.add_account("1000", "Cash", AccountType::Asset, None, false)
                .unwrap();
            s.change_settings(
                &[ledger_core::settings::Change {
                    key: "company.name".into(),
                    value: Some(serde_json::json!(name)),
                }],
                &ledger_core::settings::Source {
                    decided_by: "Test".into(),
                    decided_on: chrono::NaiveDate::from_ymd_opt(2026, 10, 15).unwrap(),
                    citation: None,
                    note: None,
                },
            )
            .unwrap();
            drop(s);
        }
        World { _dir: dir, root }
    }

    fn source(&self, id: &str) -> Source {
        Source {
            data_dir: self.root.join("books").join(id),
            today: chrono::NaiveDate::from_ymd_opt(2026, 10, 15),
        }
    }
}

/// An owner that serves, has web assistants in whatever state a test sets,
/// and says what it was asked.
struct Web {
    state: Mutex<Option<WebConnect>>,
    roster: Mutex<Roster>,
    log: Mutex<Vec<String>>,
}

impl Web {
    fn new(state: Option<WebConnect>) -> Arc<Web> {
        Arc::new(Web {
            state: Mutex::new(state),
            roster: Mutex::new(Roster::default()),
            log: Mutex::new(vec![]),
        })
    }

    fn set(&self, s: WebConnect) {
        *self.state.lock().unwrap() = Some(s);
    }

    fn said(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }
}

impl Owner for Web {
    fn serves(&self) -> bool {
        true
    }
    fn serve_start(&self, _open: &[String]) -> Result<(), String> {
        Ok(())
    }
    fn serve_open(&self, _book: &str) -> Result<Option<String>, String> {
        Ok(None)
    }
    fn serve_close(&self, _book: &str) -> Result<Option<String>, String> {
        Ok(None)
    }
    fn serve_stop(&self) -> Result<Option<String>, String> {
        Ok(None)
    }
    fn serve_roster(&self) -> Result<Roster, String> {
        Ok(self.roster.lock().unwrap().clone())
    }
    fn command_line(&self, words: &[&str]) -> Result<String, String> {
        Ok(format!("wanderbooks {}", words.join(" ")))
    }
    fn web_state(&self, _book: &str) -> Option<WebConnect> {
        self.state.lock().unwrap().clone()
    }
    fn web_turn_on(&self, book: &str) -> Result<String, String> {
        self.log.lock().unwrap().push(format!("on {book}"));
        Ok("Web assistants are on.".into())
    }
    fn web_turn_off(&self, book: &str) -> Result<String, String> {
        self.log.lock().unwrap().push(format!("off {book}"));
        Ok("Web assistants are off.".into())
    }
    fn web_new_address(&self, book: &str) -> Result<String, String> {
        self.log.lock().unwrap().push(format!("new {book}"));
        Ok("Your new address is ready.".into())
    }
    fn new_books(&self, _id: &str, _currency: &str) -> Result<String, String> {
        Err("not here".into())
    }
    fn back_up_now(&self, _book: &str) -> Result<Vec<String>, String> {
        Err("not here".into())
    }
    fn plan_restore(
        &self,
        _source: &str,
        _id: Option<String>,
        _replace: bool,
    ) -> Result<RestorePlan, String> {
        Err("not here".into())
    }
    fn restore(&self, _plan: &RestorePlan) -> Result<Vec<String>, String> {
        Err("not here".into())
    }
    fn connect(
        &self,
        _agent: ledger_gui::owner::Agent,
        _book: &str,
    ) -> Result<Vec<String>, String> {
        Err("not here".into())
    }
    fn upgrade_format(&self, _book: &str, _passphrase: &str) -> Result<Vec<String>, String> {
        Err("not here".into())
    }
}

fn off() -> WebConnect {
    WebConnect {
        on: false,
        step: WebStep::Off,
        address: None,
        connector: None,
        words: "Web assistants are off.".into(),
        problem: None,
        needs_person: false,
        kept: false,
    }
}

fn on(step: WebStep) -> WebConnect {
    WebConnect {
        on: true,
        step,
        address: Some(CONNECTOR.trim_end_matches("/b/alpha/mcp").into()),
        connector: Some(CONNECTOR.into()),
        words: String::new(),
        problem: None,
        needs_person: false,
        kept: true,
    }
}

fn token() -> TokenRow {
    TokenRow {
        id: "t1".into(),
        book: "alpha".into(),
        name: "agent".into(),
        scopes: vec!["read".into()],
        created: "2026-11-01T09:00:00Z".into(),
    }
}

fn start(w: &World, owner: &Arc<Web>, kept: &[(&str, &str)], dark: bool) -> Harness<'static, App> {
    let mut map: HashMap<String, String> = kept
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    map.insert(ledger_gui::app::open_key(&w.root), "alpha".into());
    map.insert("dark".into(), dark.to_string());
    let (root, fallback) = (w.root.clone(), w.source("alpha"));
    let for_app: Arc<dyn Owner> = owner.clone();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1280.0, 1900.0))
        .build_eframe(move |cc| {
            let mut app = App::start_with_owner(
                &cc.egui_ctx,
                &|k| map.get(k).cloned(),
                Start::Restore { root, fallback },
                Some(for_app),
            );
            app.set_now("2026-11-21T10:30:00Z".parse().unwrap());
            app
        });
    let o: Arc<dyn Owner> = owner.clone();
    h.state_mut().view_mut(0).unwrap().set_owner(o);
    until(&mut h, "the books load", |a| {
        a.views().iter().all(|v| v.books().is_some())
    });
    h
}

fn until(h: &mut Harness<'static, App>, what: &str, ok: impl Fn(&App) -> bool) {
    let end = Instant::now() + Duration::from_secs(30);
    loop {
        h.run_steps(2);
        if ok(h.state()) {
            return;
        }
        assert!(Instant::now() < end, "never: {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The Connect page, in front, with the server's answer read.
fn connect_page(w: &World, owner: &Arc<Web>, dark: bool) -> Harness<'static, App> {
    let mut h = start(w, owner, &[("first_connect_done", "1")], dark);
    h.state_mut().view_mut(0).unwrap().open_connect();
    h.state().connections().forget();
    h.run_steps(2);
    assert!(h.state().connections().wait_idle(Duration::from_secs(10)));
    h.run_steps(4);
    h
}

fn texts(h: &Harness<'_, App>) -> String {
    fn walk(s: &egui::Shape, out: &mut Vec<String>) {
        match s {
            egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            _ => {}
        }
    }
    let mut all = Vec::new();
    for c in &h.output().shapes {
        walk(&c.shape, &mut all);
    }
    all.join("\n")
}

fn has(h: &Harness<'_, App>, text: &str) -> bool {
    texts(h).contains(text)
}

fn copied(h: &Harness<'_, App>) -> Vec<String> {
    h.output()
        .platform_output
        .commands
        .iter()
        .filter_map(|c| match c {
            egui::OutputCommand::CopyText(t) => Some(t.clone()),
            _ => None,
        })
        .collect()
}

/// WB-assist-6 INVARIANT 1: off by default, nothing but the switch and
/// words; and no switch at all when the window cannot offer them.
#[test]
fn the_switch_is_off_by_default_and_an_owner_without_web_assistants_has_none() {
    let w = World::new(&[("alpha", "Wanderland Labs")]);
    let owner = Web::new(Some(off()));
    let h = connect_page(&w, &owner, false);
    let switch = h.get_by_role_and_label(Role::CheckBox, "Allow web assistants");
    assert_eq!(
        switch.accesskit_node().toggled(),
        Some(egui::accesskit::Toggled::False)
    );
    assert!(has(
        &h,
        "Off. Nothing on this computer can be reached from the internet."
    ));
    assert!(!has(&h, "The address to give an assistant"));
    assert!(!has(&h, "Claude on the web"));
    assert!(owner.said().is_empty(), "looking asks nothing of the owner");

    let none = Web::new(None);
    let h = connect_page(&w, &none, false);
    assert!(
        h.query_by_role_and_label(Role::CheckBox, "Allow web assistants")
            .is_none()
    );
}

/// WB-assist-6 INVARIANTS 2, 3: turned on, the progress in plain words,
/// then the address with Copy, the three steps for Claude, and for ChatGPT
/// and the rest the plain "we haven't tested this yet".
#[test]
fn turning_it_on_shows_progress_then_the_address_the_steps_and_what_was_not_tried() {
    let w = World::new(&[("alpha", "Wanderland Labs")]);
    let owner = Web::new(Some(off()));
    let mut h = connect_page(&w, &owner, false);
    h.get_by_role_and_label(Role::CheckBox, "Allow web assistants")
        .click();
    until(&mut h, "the owner was asked", |_| !owner.said().is_empty());
    assert_eq!(owner.said(), ["on alpha"]);

    owner.set(on(WebStep::GettingAddress));
    h.run_steps(4);
    assert!(has(&h, "Getting an address\u{2026}"));
    assert!(has(&h, "Getting a certificate"));
    assert!(
        !has(&h, "Add custom connector"),
        "no steps before there is an address that works"
    );

    owner.set(on(WebStep::GettingCertificate));
    h.run_steps(4);
    assert!(has(&h, "Getting a certificate\u{2026}"));

    owner.set(on(WebStep::Connected));
    h.run_steps(4);
    assert!(has(&h, "Connected"));
    // Once it works the three steps are one line: "Connected".
    assert!(!has(&h, "Getting an address"));
    assert!(!has(&h, "Getting a certificate"));
    assert!(has(&h, CONNECTOR));
    for (i, step) in claude_steps().iter().enumerate() {
        assert!(has(&h, &format!("{}. {step}", i + 1)), "{step}");
    }
    assert!(has(&h, CHATGPT_NOTE) && CHATGPT_NOTE.contains("We haven't tested this yet"));
    assert!(has(&h, OTHER_NOTE) && OTHER_NOTE.contains("we haven't tested these yet"));
    // The word MCP is nowhere.
    assert!(!texts(&h).contains("MCP"));
    // Copy puts the address for this company on the clipboard.
    h.get_by_role_and_label(Role::Button, "Copy the address")
        .click();
    h.step();
    assert_eq!(copied(&h), [CONNECTOR]);
    h.run_steps(2);
    assert!(has(&h, "Copied"));
}

/// The three steps are shown while connecting, and when a step is the
/// trouble even though the address works; a clean connection is one line.
#[test]
fn the_steps_collapse_to_one_line_when_connected_and_return_with_a_trouble() {
    let w = World::new(&[("alpha", "Wanderland Labs")]);
    let owner = Web::new(Some(on(WebStep::Connected)));
    let mut h = connect_page(&w, &owner, false);
    h.run_steps(4);
    assert!(has(&h, "Connected"));
    assert!(!has(&h, "Getting an address"));
    assert!(!has(&h, "Getting a certificate"));
    // The certificate could not be renewed: the steps are back, with the
    // trouble.
    owner.set(WebConnect {
        problem: Some("The certificate could not be renewed yet.".into()),
        ..on(WebStep::Connected)
    });
    h.run_steps(4);
    assert!(has(&h, "Getting an address"));
    assert!(has(&h, "Getting a certificate"));
    assert!(has(&h, "The certificate could not be renewed yet."));
    // Connecting again shows them as well.
    owner.set(on(WebStep::Connecting));
    h.run_steps(4);
    assert!(has(&h, "Getting an address"));
}

/// The words of the switch are true about who can read what (plan 5): the
/// relay cannot; the assistant's own company reads what it is sent.
#[test]
fn the_page_says_who_can_read_what_and_not_that_nobody_can() {
    let w = World::new(&[("alpha", "Wanderland Labs")]);
    let owner = Web::new(Some(off()));
    let mut h = connect_page(&w, &owner, false);
    h.run_steps(3);
    let all = texts(&h);
    assert!(!all.contains("nobody can read"));
    assert!(all.contains("The relay that carries the connection can't read it."));
    assert!(all.contains(
        "The assistant's maker (for example Anthropic or OpenAI) sees what you let that assistant read."
    ));
    assert!(all.contains(
        "An assistant on the web still has to be allowed here, by you, each time it connects."
    ));
}

/// The progress words are one function: what is done, what is under way,
/// what waits.
#[test]
fn the_progress_steps_are_in_order() {
    use Progress::*;
    let p = |s| web_progress(s).map(|(_, p)| p);
    assert_eq!(p(WebStep::GettingAddress), [Now, Waiting, Waiting]);
    assert_eq!(p(WebStep::Connecting), [Now, Waiting, Waiting]);
    assert_eq!(p(WebStep::GettingCertificate), [Done, Now, Waiting]);
    assert_eq!(p(WebStep::Connected), [Done, Done, Done]);
    let words: Vec<_> = web_progress(WebStep::Connected).map(|(w, _)| w).to_vec();
    assert_eq!(
        words,
        ["Getting an address", "Getting a certificate", "Connected"]
    );
}

/// WB-assist-6 INVARIANT 4: the relay cannot be reached (nothing is lost,
/// the window and the local assistants work), the clock is wrong (it needs
/// the person), the certificate is waited for.
#[test]
fn what_goes_wrong_is_in_words_and_says_what_is_not_affected() {
    let w = World::new(&[("alpha", "Wanderland Labs")]);
    let owner = Web::new(Some(WebConnect {
        problem: Some("This computer can't reach the internet. Your assistants on this computer still work. Web assistants will connect again as soon as the internet is back.".into()),
        ..on(WebStep::Connecting)
    }));
    let h = connect_page(&w, &owner, false);
    assert!(has(&h, "This computer can't reach the internet."));
    assert!(has(
        &h,
        "Your assistants on this computer, and this window, are not affected."
    ));
    // The local rows are there, above, untouched.
    assert!(has(&h, "On this computer"));

    owner.set(WebConnect {
        problem: Some("This computer's clock looks wrong, so the relay can't be trusted yet. Set the date and time (turn on \"set automatically\"), and Wanderbooks will connect by itself.".into()),
        needs_person: true,
        ..on(WebStep::Connecting)
    });
    let mut h = h;
    h.run_steps(4);
    assert!(has(&h, "clock looks wrong"));
    assert!(!has(&h, "are not affected"));

    owner.set(WebConnect {
        problem: Some(
            "Wanderbooks could not get a certificate yet: the authority says no. It keeps trying."
                .into(),
        ),
        ..on(WebStep::GettingCertificate)
    });
    h.run_steps(4);
    assert!(has(&h, "could not get a certificate yet"));
}

/// WB-assist-6: off calls the owner; a new address is asked for twice.
#[test]
fn off_and_a_new_address_go_through_the_owner_and_a_new_one_is_asked_twice() {
    let w = World::new(&[("alpha", "Wanderland Labs")]);
    let owner = Web::new(Some(on(WebStep::Connected)));
    let mut h = connect_page(&w, &owner, false);
    // The first click only asks.
    h.get_by_label_contains("Get a new address").click();
    h.run_steps(3);
    assert!(has(&h, "The old address stops working at once."));
    assert!(owner.said().is_empty());
    // Keeping the old one does nothing.
    h.get_by_label("Keep the old one").click();
    h.run_steps(3);
    assert!(!has(&h, "The old address stops working at once."));
    assert!(owner.said().is_empty());
    // The second click does it.
    h.get_by_label_contains("Get a new address").click();
    h.run_steps(3);
    h.get_by_label("Get a new address").click();
    until(&mut h, "the owner was asked", |_| !owner.said().is_empty());
    assert_eq!(owner.said(), ["new alpha"]);

    h.get_by_role_and_label(Role::CheckBox, "Allow web assistants")
        .click();
    until(&mut h, "off", |_| owner.said().len() == 2);
    assert_eq!(owner.said()[1], "off alpha");
}

/// WB-app-6: the first time a company is open and no assistant was ever
/// connected, the Connect page opens by itself, once.
#[test]
fn the_connect_page_opens_by_itself_once_for_a_first_run() {
    let w = World::new(&[("alpha", "Wanderland Labs")]);
    let owner = Web::new(Some(off()));
    let mut h = start(&w, &owner, &[], false);
    until(&mut h, "the page opens", |a| a.first_connect_done());
    h.run_steps(3);
    assert_eq!(h.state().route(), &Route::Connect);
    // Local rows first, the web below.
    assert!(has(&h, "On this computer") && has(&h, "On the web"));
    // The person goes elsewhere: it does not open again.
    h.state_mut().view_mut(0).unwrap().goto(Route::Company);
    h.run_steps(6);
    assert_ne!(h.state().route(), &Route::Connect);
}

/// WB-app-6: it is remembered per installation, and not for a person who
/// has connected an assistant, or has one waiting.
#[test]
fn the_connect_page_does_not_open_when_it_was_shown_or_an_assistant_exists() {
    let w = World::new(&[("alpha", "Wanderland Labs")]);
    // Shown before (kept preference).
    let owner = Web::new(Some(off()));
    let mut h = start(&w, &owner, &[("first_connect_done", "1")], false);
    h.run_steps(8);
    assert_ne!(h.state().route(), &Route::Connect);
    // A key exists.
    let owner = Web::new(Some(off()));
    owner.roster.lock().unwrap().tokens.push(token());
    let mut h = start(&w, &owner, &[], false);
    h.state().connections().forget();
    h.run_steps(6);
    assert!(h.state().connections().wait_idle(Duration::from_secs(10)));
    h.run_steps(6);
    assert_ne!(h.state().route(), &Route::Connect);
    assert!(
        h.state().first_connect_done(),
        "an existing assistant ends the first run for good"
    );
}

/// Pictures for eyes: set `SHOT_DIR`. Each state of the web half, light and
/// dark.
#[test]
fn web_screenshots() {
    let Ok(out) = std::env::var("SHOT_DIR") else {
        return;
    };
    let out = PathBuf::from(out);
    std::fs::create_dir_all(&out).unwrap();
    let w = World::new(&[("alpha", "Wanderland Labs")]);
    let states: Vec<(&str, WebConnect, bool)> = vec![
        ("1-off", off(), false),
        ("2-getting-an-address", on(WebStep::GettingAddress), false),
        (
            "3-getting-a-certificate",
            on(WebStep::GettingCertificate),
            false,
        ),
        ("4-connected", on(WebStep::Connected), false),
        (
            "5-no-internet",
            WebConnect {
                problem: Some(ledger_link::Trouble::NoInternet.words()),
                ..on(WebStep::Connecting)
            },
            false,
        ),
        (
            "6-clock-wrong",
            WebConnect {
                problem: Some(ledger_link::Trouble::ClockWrong.words()),
                needs_person: true,
                ..on(WebStep::Connecting)
            },
            false,
        ),
        ("7-new-address", on(WebStep::Connected), true),
    ];
    for dark in [false, true] {
        for (name, state, ask) in &states {
            let owner = Web::new(Some(state.clone()));
            let mut h = connect_page(&w, &owner, dark);
            if *ask {
                h.get_by_label_contains("Get a new address").click();
                h.run_steps(3);
            }
            h.run_steps(3);
            h.render()
                .unwrap()
                .save(out.join(format!(
                    "{name}-{}.png",
                    if dark { "dark" } else { "light" }
                )))
                .unwrap();
        }
    }
}
