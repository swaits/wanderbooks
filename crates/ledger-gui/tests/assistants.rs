//! The window and the assistants connected to its companies (connections.rs,
//! connect.rs, app.rs; plan/p15-app-and-relay.md 3.3; WB-assist-1 to
//! WB-assist-4): the three states, "Ask your assistant", the Connect page,
//! and the warning before a close.
//!
//! ```text
//! GOAL: a person sees, for each company, whether an assistant is connected
//!       and whether it is working, can connect or end one from a page,
//!       asks for changes through an assistant, and is warned before
//!       closing what an assistant is using; and none of it ever waits on
//!       the books server.
//! SOURCE: plan/p15-app-and-relay.md 3.3 (three states, "Ask your
//!         assistant", the Connect page), D-79 (closing with assistants
//!         connected warns, then closes), 3.1 (the sessions request);
//!         WB-assist-1, WB-assist-2, WB-assist-3, WB-assist-4.
//! INVARIANTS:
//!   1. Each state, from the server's own answer: none, connected (with the
//!      program's name), working (a call running); the last thing done in
//!      words; a company's state is its own.
//!   2. A server that is slow, gone or silent is "not known", never
//!      guessed, and no frame waits for it: one request is out at a time.
//!   3. "Ask your assistant" copies one ready sentence naming the company,
//!      says where to connect one when none is, and is not there when the
//!      window runs no server.
//!   4. The Connect page lists the keys with who is on each, what each may
//!      do, shows the exact command for a tool, and revokes a key only
//!      after asking once, through the owner.
//!   5. Closing the window or a tab warns when an assistant is connected
//!      and only then; it closes when the person says so, composing with
//!      "Closing your books"; one dialog at a time; a server that does not
//!      answer does not hold a close.
//!   6. The state mark of a tab, the top bar's state and the pages look as
//!      they were blessed (`tests/snapshots/assist_*.shapes`,
//!      `TABS_BLESS=1`).
//! ```

use chrono::{DateTime, Utc};
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ledger_core::{AccountType, Currency};
use ledger_gui::connections::Link;
use ledger_gui::data::Source;
use ledger_gui::nav::Route;
use ledger_gui::owner::{
    Agent, Assistant, LastCall, Owner, RestorePlan, Roster, TokenRow, WebApproval, WebRequest,
};
use ledger_gui::{App, Start};
use ledger_store::root::{BookId, Root};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

fn now() -> DateTime<Utc> {
    "2026-11-21T10:30:00Z".parse().unwrap()
}

/// A data root of its own, with companies made in it.
struct World {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

impl World {
    fn new(companies: &[(&str, &str)]) -> World {
        let dir = tempfile::Builder::new()
            .prefix("wb-gui-assist-")
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

/// An owner that hosts a server and answers who is connected as it is told.
#[derive(Default)]
struct Fake {
    log: Mutex<Vec<String>>,
    roster: Mutex<Option<Result<Roster, String>>>,
    /// While closed, the roster request does not return.
    gate: Mutex<bool>,
    gate_changed: Condvar,
    asked: Mutex<u32>,
    exited: Mutex<Option<String>>,
    stop_takes: Mutex<Duration>,
}

impl Fake {
    fn new() -> Arc<Fake> {
        let f = Fake::default();
        *f.gate.lock().unwrap() = true;
        Arc::new(f)
    }

    fn said(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }

    fn note(&self, s: String) {
        self.log.lock().unwrap().push(s);
    }

    fn answer(&self, r: Result<Roster, String>) {
        *self.roster.lock().unwrap() = Some(r);
    }

    fn close_gate(&self) {
        *self.gate.lock().unwrap() = false;
    }

    fn open_gate(&self) {
        *self.gate.lock().unwrap() = true;
        self.gate_changed.notify_all();
    }
}

impl Owner for Fake {
    fn serves(&self) -> bool {
        true
    }
    fn serve_start(&self, open: &[String]) -> Result<(), String> {
        self.note(format!("start {}", open.join(",")));
        Ok(())
    }
    fn serve_open(&self, book: &str) -> Result<Option<String>, String> {
        self.note(format!("open {book}"));
        Ok(None)
    }
    fn serve_close(&self, book: &str) -> Result<Option<String>, String> {
        self.note(format!("close {book}"));
        Ok(None)
    }
    fn serve_stop(&self) -> Result<Option<String>, String> {
        self.note("stop".into());
        std::thread::sleep(*self.stop_takes.lock().unwrap());
        Ok(None)
    }
    fn serve_exited(&self) -> Option<String> {
        self.exited.lock().unwrap().clone()
    }
    fn serve_roster(&self) -> Result<Roster, String> {
        *self.asked.lock().unwrap() += 1;
        let mut open = self.gate.lock().unwrap();
        let end = Instant::now() + Duration::from_secs(60);
        while !*open {
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err("the books server isn't answering".into());
            }
            open = self.gate_changed.wait_timeout(open, left).unwrap().0;
        }
        drop(open);
        self.roster
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_else(|| Ok(Roster::default()))
    }
    fn serve_revoke(&self, book: &str, name: &str, _id: &str) -> Result<String, String> {
        self.note(format!("revoke {book} {name}"));
        if let Some(Ok(r)) = self.roster.lock().unwrap().as_mut() {
            r.tokens.retain(|t| !(t.book == book && t.name == name));
            r.assistants
                .retain(|a| !(a.book == book && a.token == name));
        }
        Ok(format!("Ended {name:?}'s access to {book}."))
    }
    fn web_decide(&self, id: &str, approve: bool, post: bool) -> Result<String, String> {
        self.note(format!("web_decide {id} {approve} {post}"));
        // As the server does: it is no longer waiting, and an allowed one
        // is a connection.
        if let Some(Ok(r)) = self.roster.lock().unwrap().as_mut() {
            let gone: Vec<_> = r
                .web_pending
                .iter()
                .filter(|p| p.id == id)
                .cloned()
                .collect();
            r.web_pending.retain(|p| p.id != id);
            if approve {
                for p in gone {
                    r.web_approvals.push(WebApproval {
                        id: format!("apr_{id}"),
                        book: p.book,
                        label: p.client_name,
                        token_id: "tok_1".into(),
                        may_post: post,
                        last_used: None,
                    });
                }
            }
        }
        Ok("done".into())
    }
    fn web_set_post(&self, approval: &str, post: bool) -> Result<String, String> {
        self.note(format!("web_set_post {approval} {post}"));
        if let Some(Ok(r)) = self.roster.lock().unwrap().as_mut() {
            for a in &mut r.web_approvals {
                if a.id == approval {
                    a.may_post = post;
                }
            }
        }
        Ok("done".into())
    }
    fn command_line(&self, words: &[&str]) -> Result<String, String> {
        Ok(format!("/opt/wanderbooks {}", words.join(" ")))
    }
    fn make_key(&self, book: &str) -> Result<bool, String> {
        self.note(format!("key {book}"));
        Ok(true)
    }
    fn connect(&self, agent: Agent, book: &str) -> Result<Vec<String>, String> {
        self.note(format!("connect {} {book}", agent.key()));
        Ok(vec![format!("{} reaches the books.", agent.label())])
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
    fn upgrade_format(&self, _: &str, _: &str) -> Result<Vec<String>, String> {
        Err("not here".into())
    }
}

fn session(book: &str, token: &str, program: &str, running: u32, last: Option<&str>) -> Assistant {
    Assistant {
        book: book.into(),
        token: token.into(),
        client_name: Some(program.into()),
        client_version: Some("2.1".into()),
        model: None,
        connected_at: "2026-11-21T10:00:00Z".into(),
        running,
        last: last.map(|tool| LastCall {
            tool: tool.into(),
            outcome: "ok".into(),
            at: "2026-11-21T10:29:50Z".into(),
        }),
    }
}

fn token(book: &str, name: &str) -> TokenRow {
    TokenRow {
        id: format!("tok_{book}_{name}"),
        book: book.into(),
        name: name.into(),
        scopes: ["books:read", "books:write", "books:post"]
            .map(String::from)
            .to_vec(),
        created: "2026-11-01T09:00:00Z".into(),
    }
}

fn roster(assistants: Vec<Assistant>, tokens: Vec<TokenRow>) -> Result<Roster, String> {
    Ok(Roster {
        assistants,
        tokens,
        ..Roster::default()
    })
}

/// The app over `ids` as tabs (the first in front), served by `owner`.
fn start(w: &World, ids: &[&str], owner: &Arc<Fake>) -> Harness<'static, App> {
    let kept: HashMap<String, String> =
        HashMap::from([(ledger_gui::app::open_key(&w.root), ids.join(","))]);
    let (root, fallback) = (w.root.clone(), w.source(ids[0]));
    let for_app: Arc<dyn Owner> = owner.clone();
    // Tall, so the whole Connect page is on screen to be clicked.
    let mut h = Harness::builder()
        .with_size(egui::vec2(1280.0, 1700.0))
        .build_eframe(move |cc| {
            let mut app = App::start_with_owner(
                &cc.egui_ctx,
                &|k| kept.get(k).cloned(),
                Start::Restore { root, fallback },
                Some(for_app),
            );
            app.set_now(now());
            app
        });
    let o: Arc<dyn Owner> = owner.clone();
    for i in 0..ids.len() {
        h.state_mut().view_mut(i).unwrap().set_owner(o.clone());
    }
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

/// Read the server's answer again: forget the last, ask, wait, draw.
fn refresh(h: &mut Harness<'static, App>) {
    h.state().connections().forget();
    h.run_steps(2);
    assert!(h.state().connections().wait_idle(Duration::from_secs(10)));
    h.run_steps(3);
}

fn links(h: &Harness<'_, App>) -> Vec<Link> {
    h.state().views().iter().map(|v| v.assist().link).collect()
}

fn ask_to_close(h: &mut Harness<'static, App>) {
    h.input_mut()
        .viewports
        .entry(egui::ViewportId::ROOT)
        .or_default()
        .events
        .push(egui::ViewportEvent::Close);
    h.run_steps(3);
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

fn wait_for(what: &str, mut ok: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(30);
    while !ok() {
        assert!(Instant::now() < end, "never: {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

// ---- 1: the three states ----------------------------------------------------

#[test]
fn each_state_is_read_from_the_servers_answer_and_is_the_companys_own() {
    let w = World::new(&[("alpha", "Wanderland Labs"), ("beta", "Fernhill Studio")]);
    let owner = Fake::new();
    owner.answer(roster(vec![], vec![]));
    let mut h = start(&w, &["alpha", "beta"], &owner);
    refresh(&mut h);
    // No assistant: said in words, and what to do.
    assert_eq!(links(&h), [Link::None, Link::None]);
    let a = h.state().views()[0].assist().clone();
    assert_eq!(a.word(), "No assistant connected");
    assert_eq!(a.detail(), "Connect an assistant");
    assert!(h.query_by_label("No assistant connected").is_some());

    // One connected to alpha, idle: its name, and the last thing done.
    owner.answer(roster(
        vec![session(
            "alpha",
            "agent",
            "claude-code",
            0,
            Some("books.post_entry"),
        )],
        vec![token("alpha", "agent")],
    ));
    refresh(&mut h);
    assert_eq!(links(&h), [Link::Connected, Link::None], "beta is its own");
    let a = h.state().views()[0].assist().clone();
    assert_eq!(a.word(), "Claude Code connected");
    assert_eq!(a.detail(), "Posted an entry, 10 seconds ago");
    assert!(h.query_by_label("Claude Code connected").is_some());

    // A call running: working.
    owner.answer(roster(
        vec![session(
            "alpha",
            "agent",
            "claude-code",
            1,
            Some("books.post_entry"),
        )],
        vec![token("alpha", "agent")],
    ));
    refresh(&mut h);
    assert_eq!(links(&h)[0], Link::Working);
    let a = h.state().views()[0].assist().clone();
    assert_eq!(a.word(), "Claude Code is working");
    assert_eq!(a.detail(), "Last: posted an entry, 10 seconds ago");

    // Two programs on one company.
    owner.answer(roster(
        vec![
            session("alpha", "agent", "claude-code", 0, None),
            session("alpha", "codex", "codex-mcp-client", 0, None),
        ],
        vec![],
    ));
    refresh(&mut h);
    assert_eq!(
        h.state().views()[0].assist().word(),
        "Claude Code and Codex connected"
    );
    drop(h);
}

// ---- 2: slow, gone, silent -------------------------------------------------

#[test]
fn a_slow_server_is_never_waited_for_and_one_request_is_out_at_a_time() {
    let w = World::new(&[("alpha", "Wanderland Labs")]);
    let owner = Fake::new();
    owner.close_gate();
    owner.answer(roster(
        vec![session("alpha", "agent", "claude-code", 0, None)],
        vec![],
    ));
    let began = Instant::now();
    let mut h = start(&w, &["alpha"], &owner);
    let frames = Instant::now();
    h.run_steps(30);
    assert!(
        frames.elapsed() < Duration::from_secs(10),
        "frames did not wait for the server: {:?}",
        frames.elapsed()
    );
    let _ = began;
    // Not known, said so and not guessed; one request, still out.
    assert_eq!(links(&h), [Link::Unknown]);
    assert!(h.state().connections().is_asking());
    wait_for("the request began", || *owner.asked.lock().unwrap() >= 1);
    assert_eq!(
        *owner.asked.lock().unwrap(),
        1,
        "never a second one beside it"
    );
    // Answered at last: known.
    owner.open_gate();
    assert!(h.state().connections().wait_idle(Duration::from_secs(10)));
    h.run_steps(3);
    assert_eq!(links(&h), [Link::Connected]);
    drop(h);
}

#[test]
fn a_server_that_is_gone_or_refuses_is_not_known_and_the_window_goes_on() {
    let w = World::new(&[("alpha", "Wanderland Labs")]);
    let owner = Fake::new();
    owner.answer(Err("the books server didn't answer (BrokenPipe)".into()));
    let mut h = start(&w, &["alpha"], &owner);
    refresh(&mut h);
    assert_eq!(links(&h), [Link::Unknown]);
    assert!(h.state().connections().trouble().is_some());
    // The server exits on its own: said as down, still drawing.
    *owner.exited.lock().unwrap() = Some("The books server stopped".into());
    h.run_steps(4);
    assert!(h.state().views()[0].assist().down);
    assert!(
        h.query_by_label_contains("Assistants can't connect now")
            .is_some()
    );
    // The company's own pages still draw.
    h.state_mut().goto(Route::Transactions);
    h.run_steps(3);
    assert_eq!(h.state().route(), &Route::Transactions);
    assert!(
        h.query_all_by_label_contains("Transactions")
            .next()
            .is_some()
    );
    drop(h);
}

// ---- 3: Ask your assistant ---------------------------------------------------

#[test]
fn ask_your_assistant_copies_one_ready_sentence() {
    let w = World::new(&[("alpha", "Wanderland Labs")]);
    let owner = Fake::new();
    owner.answer(roster(vec![], vec![]));
    let mut h = start(&w, &["alpha"], &owner);
    refresh(&mut h);
    h.state_mut().goto(Route::Transactions);
    h.run_steps(3);
    let below = h.get_by_label_contains("recorded, newest first").rect();
    h.get_by_role_and_label(Role::Button, "Ask your assistant")
        .click();
    h.step();
    assert_eq!(
        copied(&h),
        ["In my Wanderbooks books for Wanderland Labs, please record a new transaction."]
    );
    h.run_steps(2);
    assert_eq!(
        h.get_by_label_contains("recorded, newest first").rect(),
        below,
        "the feedback is a popover: the page does not move"
    );
    // With none connected it says so, and where to connect one.
    assert!(
        h.query_by_label_contains("No assistant is connected yet.")
            .is_some()
    );
    h.get_by_label("Connect an assistant").click();
    h.run_steps(3);
    assert_eq!(h.state().route(), &Route::Connect);

    // With one connected, only what was copied.
    owner.answer(roster(
        vec![session("alpha", "agent", "claude-code", 0, None)],
        vec![],
    ));
    refresh(&mut h);
    h.state_mut().goto(Route::Receivables);
    h.run_steps(3);
    h.get_by_role_and_label(Role::Button, "Ask your assistant")
        .click();
    h.step();
    assert_eq!(
        copied(&h),
        ["In my Wanderbooks books for Wanderland Labs, please create a new invoice."]
    );
    h.run_steps(2);
    assert!(
        h.query_by_label_contains("No assistant is connected yet.")
            .is_none()
    );
    drop(h);
}

#[test]
fn a_window_with_no_server_offers_no_assistant() {
    let w = World::new(&[("alpha", "Wanderland Labs")]);
    let source = w.source("alpha");
    let mut h = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .build_eframe(move |cc| {
            App::with_books(
                &cc.egui_ctx,
                ledger_gui::data::load_once(&source).unwrap(),
                false,
                Route::Transactions,
            )
        });
    h.run_steps(4);
    assert!(h.query_by_label("Ask your assistant").is_none());
    assert!(h.query_by_label("No assistant connected").is_none());
}

// ---- 4: the Connect page ---------------------------------------------------

#[test]
fn the_top_bar_state_opens_the_connect_page_which_lists_and_revokes() {
    let w = World::new(&[("alpha", "Wanderland Labs")]);
    let owner = Fake::new();
    owner.answer(roster(
        vec![session(
            "alpha",
            "agent",
            "claude-code",
            0,
            Some("books.draft_entry"),
        )],
        vec![token("alpha", "agent"), token("alpha", "web")],
    ));
    let mut h = start(&w, &["alpha"], &owner);
    refresh(&mut h);
    h.get_by_label("Claude Code connected").click();
    h.run_steps(3);
    assert_eq!(h.state().route(), &Route::Connect);
    // A row per tool, and the keys with who is on each.
    for tool in ["Claude Code", "Codex", "Another assistant"] {
        assert!(
            h.query_all_by_label(tool).next().is_some(),
            "a row for {tool}"
        );
    }
    assert!(h.query_by_label_contains("Key “agent”").is_some());
    assert!(h.query_by_label_contains("Key “web”").is_some());
    assert!(
        h.query_by_label_contains("Claude Code 2.1, connected 30 minutes ago")
            .is_some(),
        "the assistant's name and version, and when it connected"
    );
    assert!(
        h.query_by_label_contains("Not connected. Can read the books, draft entries and post them")
            .is_some()
    );
    // The exact command for a tool with no button of its own, and for
    // Claude Code once its by-hand part is opened.
    assert_eq!(
        h.query_all_by_label("/opt/wanderbooks mcp --book alpha")
            .count(),
        1,
        "the program and its arguments, once, on the card for any other assistant"
    );
    h.get_all_by_label("Or do it yourself, in a terminal")
        .next()
        .unwrap()
        .click();
    h.run_steps(3);
    assert!(
        h.query_by_label(
            "claude mcp add --scope user wanderbooks-alpha -- /opt/wanderbooks mcp --book alpha"
        )
        .is_some()
    );

    // Revoke asks once, and does nothing until it is said again.
    h.get_all_by_role_and_label(Role::Button, "Revoke")
        .next()
        .unwrap()
        .click();
    h.run_steps(3);
    assert!(!owner.said().iter().any(|s| s.starts_with("revoke")));
    h.get_by_role_and_label(Role::Button, "Keep it").click();
    h.run_steps(2);
    assert!(!owner.said().iter().any(|s| s.starts_with("revoke")));
    h.get_all_by_role_and_label(Role::Button, "Revoke")
        .next()
        .unwrap()
        .click();
    h.run_steps(2);
    h.get_by_role_and_label(Role::Button, "Yes, revoke").click();
    h.run_steps(2);
    wait_for("the revoke ran", || {
        owner.said().contains(&"revoke alpha agent".to_string())
    });
    until(&mut h, "the page says it", |a| {
        a.views()[0].connect_state().said().is_some()
    });
    // Who is connected is read again: the key and its session are gone.
    until(&mut h, "the key is gone", |a| {
        a.views()[0]
            .assist()
            .tokens
            .iter()
            .all(|t| t.name != "agent")
    });
    assert!(h.query_by_label_contains("Key “agent”").is_none());
    assert!(h.query_by_label_contains("Key “web”").is_some());
    drop(h);
}

#[test]
fn the_connect_buttons_do_what_connect_does() {
    let w = World::new(&[("alpha", "Wanderland Labs")]);
    let owner = Fake::new();
    owner.answer(roster(vec![], vec![]));
    let mut h = start(&w, &["alpha"], &owner);
    refresh(&mut h);
    h.state_mut().goto(Route::Connect);
    h.run_steps(3);
    h.get_by_role_and_label(Role::Button, "Connect Codex")
        .click();
    h.run_steps(2);
    wait_for("connect ran", || {
        owner.said().contains(&"connect codex alpha".to_string())
    });
    until(&mut h, "it said what came of it", |a| {
        a.views()[0].connect_state().said().is_some()
    });
    h.run_steps(2);
    assert!(
        h.query_by_label_contains("Codex reaches the books.")
            .is_some()
    );
    until(&mut h, "the page is idle", |a| {
        !a.views()[0].connect_state().busy()
    });
    h.run_steps(2);
    h.get_by_role_and_label(Role::Button, "Connect Claude Code")
        .click();
    h.run_steps(2);
    wait_for("connect ran for Claude Code", || {
        owner.said().contains(&"connect claude alpha".to_string())
    });
    h.get_by_role_and_label(Role::Button, "Make the key")
        .click();
    h.run_steps(2);
    wait_for("the key was made", || {
        owner.said().contains(&"key alpha".to_string())
    });
    drop(h);
}

// ---- 5: the warning before a close ---------------------------------------------

fn connected_alpha(owner: &Fake) {
    owner.answer(roster(
        vec![session("alpha", "agent", "claude-code", 1, None)],
        vec![token("alpha", "agent")],
    ));
}

#[test]
fn closing_the_window_with_an_assistant_connected_warns_and_then_closes_if_confirmed() {
    let w = World::new(&[("alpha", "Wanderland Labs"), ("beta", "Fernhill Studio")]);
    let owner = Fake::new();
    *owner.stop_takes.lock().unwrap() = Duration::from_millis(400);
    connected_alpha(&owner);
    let mut h = start(&w, &["alpha", "beta"], &owner);
    refresh(&mut h);
    ask_to_close(&mut h);
    assert!(h.state().window_warning_is_up());
    assert!(h.query_by_label("Assistants are connected").is_some());
    assert!(
        h.query_by_label_contains("Wanderland Labs: Claude Code is working")
            .is_some()
    );
    // Only the company that has one is named; the books are not closing.
    assert!(h.query_by_label_contains("Fernhill Studio:").is_none());
    assert!(h.query_by_label_contains("Closing your books").is_none());
    assert!(!owner.said().contains(&"stop".to_string()));
    // Keep it open: nothing closes, and nothing is up.
    h.get_by_role_and_label(Role::Button, "Keep it open")
        .click();
    h.run_steps(3);
    assert!(!h.state().assistant_warning_is_up());
    assert!(!owner.said().contains(&"stop".to_string()));
    // Asked again, and confirmed: the books close, in view, with no second
    // dialog beside it.
    ask_to_close(&mut h);
    assert!(h.state().window_warning_is_up());
    h.get_by_role_and_label(Role::Button, "Close Wanderbooks")
        .click();
    h.run_steps(2);
    assert!(!h.state().assistant_warning_is_up());
    // The window's close command comes back as a close request (egui-winit
    // does that; the harness does not).
    ask_to_close(&mut h);
    until(&mut h, "the close began", |a| a.serving().is_closing());
    assert!(!h.state().assistant_warning_is_up(), "not warned twice");
    assert!(
        h.query_by_label_contains("Closing your books").is_some()
            || h.state().serving().is_closed()
    );
    assert!(h.query_by_label("Assistants are connected").is_none());
    until(&mut h, "the close is done", |a| a.serving().is_closed());
    assert_eq!(
        owner.said().iter().filter(|s| *s == "stop").count(),
        1,
        "{:?}",
        owner.said()
    );
    drop(h);
}

#[test]
fn closing_the_window_with_no_assistant_connected_does_not_warn() {
    let w = World::new(&[("alpha", "Wanderland Labs")]);
    let owner = Fake::new();
    owner.answer(roster(vec![], vec![token("alpha", "agent")]));
    let mut h = start(&w, &["alpha"], &owner);
    refresh(&mut h);
    ask_to_close(&mut h);
    assert!(!h.state().assistant_warning_is_up());
    until(&mut h, "the close began", |a| a.serving().is_closing());
    assert!(h.query_by_label("Assistants are connected").is_none());
    drop(h);
}

#[test]
fn a_server_that_does_not_answer_does_not_hold_a_close() {
    let w = World::new(&[("alpha", "Wanderland Labs")]);
    let owner = Fake::new();
    owner.close_gate();
    let mut h = start(&w, &["alpha"], &owner);
    h.run_steps(5);
    assert_eq!(links(&h), [Link::Unknown]);
    ask_to_close(&mut h);
    assert!(!h.state().assistant_warning_is_up(), "it cannot be told");
    until(&mut h, "the close began", |a| a.serving().is_closing());
    owner.open_gate();
    drop(h);
}

#[test]
fn closing_a_tab_with_an_assistant_connected_warns_and_only_then() {
    let w = World::new(&[("alpha", "Wanderland Labs"), ("beta", "Fernhill Studio")]);
    let owner = Fake::new();
    connected_alpha(&owner);
    let mut h = start(&w, &["alpha", "beta"], &owner);
    refresh(&mut h);
    // Beta has none: its tab closes at once.
    h.get_by_label("Close Fernhill Studio").click();
    h.run_steps(3);
    assert_eq!(h.state().views().len(), 1);
    assert!(!h.state().assistant_warning_is_up());
    // Alpha has one: asked first.
    h.get_by_label("Close Wanderland Labs").click();
    h.run_steps(3);
    assert!(h.state().assistant_warning_is_up());
    assert!(!h.state().window_warning_is_up());
    assert!(h.query_by_label("Close Wanderland Labs?").is_some());
    assert_eq!(h.state().views().len(), 1);
    h.get_by_role_and_label(Role::Button, "Keep it open")
        .click();
    h.run_steps(3);
    assert_eq!(h.state().views().len(), 1);
    assert!(!h.state().assistant_warning_is_up());
    h.get_by_label("Close Wanderland Labs").click();
    h.run_steps(3);
    h.get_by_role_and_label(Role::Button, "Close the company")
        .click();
    h.run_steps(3);
    assert_eq!(h.state().views().len(), 0);
    wait_for("the company closed in the server", || {
        owner.said().contains(&"close alpha".to_string())
    });
    drop(h);
}

// ---- 6: how it looks ----------------------------------------------------------

fn describe(shape: &egui::epaint::Shape) -> String {
    use egui::epaint::Shape;
    match shape {
        Shape::Text(t) => {
            let mut out = format!(
                "Text pos={:?} fallback={:?} angle={:?} text={:?}",
                t.pos, t.fallback_color, t.angle, t.galley.job.text,
            );
            for placed in &t.galley.rows {
                out.push_str(&format!(" row@{:?} size={:?}", placed.pos, placed.row.size));
                for g in &placed.row.glyphs {
                    out.push_str(&format!(" {}@{:?}+{:?}", g.chr, g.pos, g.advance_width));
                }
            }
            out
        }
        Shape::Vec(v) => v.iter().map(describe).collect::<Vec<_>>().join(" | "),
        Shape::Mesh(m) => {
            let mut out = String::from("Mesh");
            for v in &m.vertices {
                out.push_str(&format!(" v{:?}{:?}", v.pos, v.color));
            }
            out
        }
        other => format!("{other:?}"),
    }
}

/// A hash of every shape the window paints where it can be seen.
fn shapes_of(h: &Harness<'_, App>) -> u64 {
    let screen = h.ctx.content_rect();
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for s in h
        .output()
        .shapes
        .iter()
        .filter(|s| s.clip_rect.intersects(screen))
    {
        let line = format!("{:?} {}", s.clip_rect, describe(&s.shape));
        for b in line.bytes() {
            hash ^= b as u64;
            hash = hash.wrapping_mul(0x100_0000_01b3);
        }
    }
    hash
}

/// The shapes painted are the check (`tests/snapshots/<name>.shapes`;
/// `TABS_BLESS=1` writes them): never pixels.
fn picture(h: &Harness<'_, App>, name: &str) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots");
    let file = dir.join(format!("{name}.shapes"));
    let shapes = format!("{:016x}", shapes_of(h));
    if std::env::var("TABS_BLESS").is_ok() {
        std::fs::write(&file, format!("{shapes}\n")).unwrap();
    } else {
        let want = std::fs::read_to_string(&file)
            .unwrap_or_else(|e| panic!("{}: {e} (TABS_BLESS=1 writes it)", file.display()));
        assert_eq!(shapes, want.trim(), "the shapes of {name} changed");
    }
}

/// A window of two companies in the three states (alpha: working, beta:
/// none, gamma: connected), the front one `front`.
fn three_states(dark: bool, front: usize, size: egui::Vec2) -> (World, Harness<'static, App>) {
    let w = World::new(&[
        ("alpha", "Wanderland Labs LLC"),
        ("beta", "Fernhill Studio"),
        ("gamma", "Oak & Ash Carpentry"),
    ]);
    let owner = Fake::new();
    owner.answer(roster(
        vec![
            session("alpha", "agent", "claude-code", 1, Some("books.post_entry")),
            session(
                "gamma",
                "agent",
                "codex-mcp-client",
                0,
                Some("books.draft_entry"),
            ),
        ],
        vec![token("alpha", "agent"), token("gamma", "agent")],
    ));
    let kept: HashMap<String, String> = HashMap::from([
        (
            ledger_gui::app::open_key(&w.root),
            "alpha,beta,gamma".to_string(),
        ),
        ("dark".to_string(), dark.to_string()),
    ]);
    let (root, fallback) = (w.root.clone(), w.source("alpha"));
    let for_app: Arc<dyn Owner> = owner.clone();
    let mut h = Harness::builder().with_size(size).build_eframe(move |cc| {
        let mut app = App::start_with_owner(
            &cc.egui_ctx,
            &|k| kept.get(k).cloned(),
            Start::Restore { root, fallback },
            Some(for_app),
        );
        app.set_now(now());
        app
    });
    let o: Arc<dyn Owner> = owner;
    for i in 0..3 {
        h.state_mut().view_mut(i).unwrap().set_owner(o.clone());
    }
    until(&mut h, "the books load", |a| {
        a.views().iter().all(|v| v.books().is_some())
    });
    refresh(&mut h);
    h.state_mut().activate(front);
    h.run_steps(4);
    (w, h)
}

#[test]
fn the_state_marks_the_top_bar_and_the_pages_look_as_blessed() {
    for dark in [false, true] {
        let mode = if dark { "dark" } else { "light" };
        let size = egui::vec2(1280.0, 800.0);
        // alpha in front: working; the tabs show all three marks.
        let (_w, mut h) = three_states(dark, 0, size);
        picture(&h, &format!("assist_working_{mode}"));
        h.state_mut().activate(1);
        h.run_steps(4);
        picture(&h, &format!("assist_none_{mode}"));
        h.state_mut().activate(2);
        h.run_steps(4);
        picture(&h, &format!("assist_connected_{mode}"));
        h.state_mut().goto(Route::Connect);
        h.run_steps(4);
        picture(&h, &format!("assist_connect_page_{mode}"));
        h.state_mut().activate(1);
        h.state_mut().goto(Route::Receivables);
        h.run_steps(4);
        picture(&h, &format!("assist_ask_{mode}"));
    }
}

/// The three states are three different shapes, whatever the colour (plugs
/// apart, plugs together, and an arc that is drawn, not a glyph).
#[test]
fn the_three_states_are_three_different_shapes() {
    let (_w, h) = three_states(false, 0, egui::vec2(1280.0, 800.0));
    assert_eq!(links(&h), [Link::Working, Link::None, Link::Connected]);
    let (none, connected, working) = (
        Link::None.glyph(),
        Link::Connected.glyph(),
        Link::Working.glyph(),
    );
    assert!(none.is_some() && connected.is_some());
    assert_ne!(none, connected, "apart and together are two glyphs");
    assert_eq!(working, None, "working is an arc, drawn, and no glyph");
    assert_eq!(
        Link::Unknown.glyph(),
        None,
        "nothing is drawn when not known"
    );
}

/// Pictures of the main states for a person to look at
/// (`SHOT_DIR=… cargo test -p ledger-gui --test assistants -- --ignored pictures`).
#[test]
#[ignore]
fn pictures() {
    let out = PathBuf::from(std::env::var("SHOT_DIR").expect("SHOT_DIR"));
    std::fs::create_dir_all(&out).unwrap();
    let save = |h: &mut Harness<'_, App>, name: &str| {
        h.render()
            .unwrap()
            .save(out.join(format!("{name}.png")))
            .unwrap();
    };
    for dark in [false, true] {
        let mode = if dark { "dark" } else { "light" };
        let size = egui::vec2(1440.0, 920.0);
        let (_w, mut h) = three_states(dark, 0, size);
        h.state_mut().goto(Route::Transactions);
        h.run_steps(4);
        save(&mut h, &format!("1-working-{mode}"));
        h.state_mut().activate(1);
        h.run_steps(4);
        save(&mut h, &format!("2-none-{mode}"));
        h.state_mut().activate(2);
        h.run_steps(4);
        save(&mut h, &format!("3-connected-{mode}"));
        h.state_mut().activate(1);
        h.state_mut().goto(Route::Receivables);
        h.run_steps(3);
        h.get_by_role_and_label(Role::Button, "Ask your assistant")
            .click();
        h.run_steps(3);
        save(&mut h, &format!("4-ask-your-assistant-{mode}"));
        h.state_mut().goto(Route::Connect);
        h.run_steps(4);
        save(&mut h, &format!("5-connect-none-{mode}"));
        h.state_mut().activate(0);
        h.state_mut().goto(Route::Connect);
        h.run_steps(4);
        save(&mut h, &format!("6-connect-working-{mode}"));
        // The window asked to close with an assistant connected.
        ask_to_close(&mut h);
        save(&mut h, &format!("7-close-warning-{mode}"));
        // A short window, as the owner's laptop has.
        let (_w2, mut small) = three_states(dark, 0, egui::vec2(1479.0, 519.0));
        save(&mut small, &format!("8-short-window-{mode}"));
        // The Connect page tall enough to show "Another assistant".
        let (_w3, mut tall) = three_states(dark, 1, egui::vec2(1440.0, 1300.0));
        tall.state_mut().goto(Route::Connect);
        tall.run_steps(4);
        save(&mut tall, &format!("9-connect-another-{mode}"));
    }
}

// ---- 7: a web assistant asks to connect (plan P15 D-83, unit B1) --------------

fn asking(id: &str, name: &str, known_host: bool) -> WebRequest {
    WebRequest {
        id: id.into(),
        book: "alpha".into(),
        client_name: name.into(),
        known_host,
        code: format!("482 {}", 100 + id.len()),
        redirect: "https://claude.ai/api/mcp/auth_callback".into(),
        redirect_host: "claude.ai".into(),
        from: "203.0.113.9".into(),
        wants_post: false,
        expires_in_secs: 240,
    }
}

fn with_pending(owner: &Fake, pending: Vec<WebRequest>, approvals: Vec<WebApproval>) {
    owner.answer(Ok(Roster {
        web_pending: pending,
        web_approvals: approvals,
        ..Roster::default()
    }));
}

#[test]
fn a_web_assistant_asking_is_put_to_the_person_over_any_page_in_plain_words() {
    let w = World::new(&[("alpha", "Wanderland Labs")]);
    let owner = Fake::new();
    with_pending(&owner, vec![asking("p_1", "Claude", true)], vec![]);
    let mut h = start(&w, &["alpha"], &owner);
    refresh(&mut h);
    // On the company's own page, not the Connect page.
    assert_ne!(h.state().route(), &Route::Connect);
    // Where it is sent back to first, in neutral words: a host says nothing
    // of who asks, so nothing is called a known product, and the one
    // address of a vendor is not warned about, nor praised.
    assert!(
        h.query_by_label_contains(
            "An assistant that will be sent back to claude.ai wants to connect to Wanderland Labs. Allow?"
        )
        .is_some()
    );
    assert!(h.query_by_label_contains("Claude on the web").is_none());
    assert!(
        h.query_by_label_contains("doesn't know this address")
            .is_none()
    );
    // What it calls itself, after the address, as its own word.
    assert!(h.query_by_label_contains("It calls itself").is_some());
    // The code to match with the one the browser's page shows.
    assert!(h.query_by_label_contains("Code 482 103").is_some());
    assert!(
        h.query_by_label_contains("Allow only if the page in your browser shows this same code")
            .is_some()
    );
    // Who, where it goes, from where: in words, nothing about OAuth.
    assert!(
        h.query_all_by_label_contains("The request came from the internet address 203.0.113.9")
            .next()
            .is_some()
    );
    assert!(
        h.query_by_label_contains("can't post entries unless you tick")
            .is_some()
    );
    for word in ["OAuth", "token", "MCP"] {
        assert!(
            h.query_by_label_contains(word).is_none(),
            "{word} is not a word for the person"
        );
    }
    // Nothing is decided by looking at it.
    assert!(!owner.said().iter().any(|s| s.starts_with("web_decide")));
    h.get_by_role_and_label(Role::Button, "Allow").click();
    h.run_steps(2);
    wait_for("the answer was given", || {
        owner
            .said()
            .contains(&"web_decide p_1 true false".to_string())
    });
    // Posting stayed off. The question goes away once the server stops
    // listing it.
    refresh(&mut h);
    assert!(h.query_by_label_contains("wants to connect").is_none());
    drop(h);
}

#[test]
fn refusing_and_allowing_with_posting_are_their_own_answers() {
    let w = World::new(&[("alpha", "Wanderland Labs")]);
    let owner = Fake::new();
    with_pending(
        &owner,
        vec![asking("p_7", "Someone's assistant", false)],
        vec![],
    );
    let mut h = start(&w, &["alpha"], &owner);
    refresh(&mut h);
    // An address that isn't a known vendor's keeps its warning.
    assert!(
        h.query_by_label_contains("doesn't know this address")
            .is_some()
    );
    h.get_by_role_and_label(Role::Button, "Don't allow").click();
    h.run_steps(2);
    wait_for("the refusal was given", || {
        owner
            .said()
            .contains(&"web_decide p_7 false false".to_string())
    });
    refresh(&mut h);
    assert!(h.query_by_label_contains("wants to connect").is_none());
    // Another asks; this time posting is ticked, then allowed.
    with_pending(&owner, vec![asking("p_8", "Claude", true)], vec![]);
    refresh(&mut h);
    h.get_by_label("Also let it post entries").click();
    h.run_steps(2);
    h.get_by_role_and_label(Role::Button, "Allow").click();
    h.run_steps(2);
    wait_for("the answer with posting was given", || {
        owner
            .said()
            .contains(&"web_decide p_8 true true".to_string())
    });
    drop(h);
}

#[test]
fn waiting_requests_each_show_their_own_code_and_only_a_few_are_drawn() {
    let w = World::new(&[("alpha", "Wanderland Labs")]);
    let owner = Fake::new();
    let mut all = Vec::new();
    for (i, id) in ["p_a", "p_b", "p_c", "p_d", "p_e"].iter().enumerate() {
        let mut r = asking(id, "Claude", true);
        // Never merged: each has its own code.
        r.code = format!("{}{}{} 6{}{}", i, i, i, i, i);
        // The later the id, the newer it is (more time left).
        r.expires_in_secs = 100 + 10 * i as u64;
        all.push(r);
    }
    with_pending(&owner, all, vec![]);
    let mut h = start(&w, &["alpha"], &owner);
    refresh(&mut h);
    // The newest three, each with its own code; the rest are counted.
    for shown in ["444 644", "333 633", "222 622"] {
        assert!(
            h.query_by_label_contains(&format!("Code {shown}"))
                .is_some(),
            "{shown}"
        );
    }
    for hidden in ["111 611", "000 600"] {
        assert!(
            h.query_by_label_contains(&format!("Code {hidden}"))
                .is_none(),
            "{hidden}"
        );
    }
    // Three cards (their titles end in the question); the window has its own title.
    assert_eq!(h.query_all_by_label_contains("Allow?").count(), 3);
    assert!(h.query_by_label_contains("2 more are waiting").is_some());
    drop(h);
}

#[test]
fn a_request_that_ran_out_goes_away_and_posting_is_turned_on_and_off_on_the_connect_page() {
    let w = World::new(&[("alpha", "Wanderland Labs")]);
    let owner = Fake::new();
    with_pending(&owner, vec![asking("p_2", "Claude", true)], vec![]);
    let mut h = start(&w, &["alpha"], &owner);
    refresh(&mut h);
    assert!(
        h.query_all_by_label_contains("wants to connect")
            .next()
            .is_some()
    );
    // The server turned it away unanswered: it is not listed any more, and
    // the window never keeps what the server has dropped.
    with_pending(&owner, vec![], vec![]);
    refresh(&mut h);
    assert!(h.query_by_label_contains("wants to connect").is_none());
    // A connection that was allowed shows on the Connect page, with what
    // it may do, and the person raises and lowers it.
    with_pending(
        &owner,
        vec![],
        vec![WebApproval {
            id: "apr_1".into(),
            book: "alpha".into(),
            label: "Claude on the web".into(),
            token_id: "tok_1".into(),
            may_post: false,
            last_used: None,
        }],
    );
    refresh(&mut h);
    h.state_mut().view_mut(0).unwrap().goto(Route::Connect);
    h.run_steps(3);
    assert!(h.query_by_label_contains("Assistants on the web").is_some());
    assert!(
        h.query_by_label_contains("Can read and draft entries. It can't post them.")
            .is_some()
    );
    h.get_by_role_and_label(Role::Button, "Let it post entries")
        .click();
    h.run_steps(2);
    wait_for("posting was turned on", || {
        owner
            .said()
            .contains(&"web_set_post apr_1 true".to_string())
    });
    refresh(&mut h);
    assert!(
        h.query_by_label_contains("Can read, draft and post entries.")
            .is_some()
    );
    h.get_by_role_and_label(Role::Button, "Stop letting it post")
        .click();
    h.run_steps(2);
    wait_for("posting was turned off", || {
        owner
            .said()
            .contains(&"web_set_post apr_1 false".to_string())
    });
    drop(h);
}
