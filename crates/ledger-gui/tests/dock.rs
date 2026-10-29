//! The agent dock (dock.rs, plan P11b, D-27): it runs the chosen agent in
//! the book's books home, the agent's output reaches the panel, keys and
//! Ctrl-C reach the agent, a resize redraws; an agent that isn't installed
//! is offered through its vendor's installer; the dock writes nothing of
//! the books. Linux and macOS (a PTY); Windows is not offered.

#![cfg(unix)]

use chrono::NaiveDate;
use egui::{Key, Modifiers};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ledger_core::Currency;
use ledger_gui::App;
use ledger_gui::data::{self, Source};
use ledger_gui::nav::Route;
use ledger_gui::owner::{Agent, AgentStart, Installer, Owner, RestorePlan};
use ledger_store::Store;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// An owner that only answers `agent_start`, and remembers what it was asked.
#[derive(Default)]
struct Fake {
    answer: Mutex<Option<Result<AgentStart, String>>>,
    asked: Mutex<Vec<String>>,
}

impl Owner for Fake {
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
    fn agent_start(&self, agent: Agent, book: &str) -> Result<AgentStart, String> {
        self.asked.lock().unwrap().push(format!("{agent:?} {book}"));
        self.answer
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_else(|| Err("no answer set".into()))
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

/// Books in a data root, named `acme`.
fn books() -> (tempfile::TempDir, PathBuf, ledger_gui::books::Books) {
    let dir = tempfile::Builder::new()
        .prefix("wb-gui-dock-")
        .tempdir()
        .unwrap();
    std::fs::write(dir.path().join(ledger_store::root::MARKER_FILE), b"").unwrap();
    let book = dir.path().join("books").join("acme");
    std::fs::create_dir_all(&book).unwrap();
    drop(Store::create(&book, Currency::USD).unwrap());
    let b = data::load_once(&Source {
        data_dir: book.clone(),
        today: NaiveDate::from_ymd_opt(2026, 10, 15),
    })
    .unwrap();
    (dir, book, b)
}

/// A stand-in `claude`: says where it is, answers a typed line, and reports
/// Ctrl-C and a resize (`stty size`) without ending.
fn fake_claude(dir: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join("claude");
    std::fs::write(
        &path,
        r#"#!/bin/bash
echo "fake-agent in $(basename "$(pwd)")"
trap 'echo got-interrupt' INT
trap 'echo "size $(stty size)"' WINCH
echo ready
while true; do
  if read -r -t 0.1 line; then echo "you-said: $line"; fi
done
"#,
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn harness(b: ledger_gui::books::Books) -> Harness<'static, App> {
    let mut h = Harness::builder()
        .with_size(egui::vec2(1200.0, 900.0))
        .build_eframe(|cc| App::with_books(&cc.egui_ctx, b, false, Route::Overview));
    h.run_steps(3);
    h
}

/// SHOT_DIR set: the window as drawn, to look at.
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

/// Run until the dock's screen holds `want`.
fn until(h: &mut Harness<'static, App>, want: &str) -> String {
    let end = Instant::now() + Duration::from_secs(20);
    loop {
        h.run_steps(2);
        let screen = h.state_mut().dock_mut().screen().unwrap_or_default();
        if screen.contains(want) {
            return screen;
        }
        assert!(
            Instant::now() < end,
            "the dock never showed {want:?}; it shows:\n{screen}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn dock_runs_the_chosen_agent() {
    let _alone = alone();
    let f = fake();
    let tools = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let home = home.path().join("acme");
    std::fs::create_dir_all(&home).unwrap();
    *f.answer.lock().unwrap() = Some(Ok(AgentStart::Ready {
        label: "Claude Code".into(),
        program: fake_claude(tools.path()),
        args: vec![],
        dir: home.clone(),
        env: vec![("TERM".into(), "xterm-256color".into())],
    }));
    let (_dir, book, b) = books();
    let before = std::fs::read(book.join("journal.jsonl")).ok();
    let mut h = harness(b);
    assert!(
        h.state_mut().dock_mut().screen().is_none(),
        "nothing runs until the dock is opened"
    );

    // Opened: the chosen agent (Claude Code, the default) is started for
    // this book, in its books home; its output reaches the panel.
    h.state_mut().dock_mut().open = true;
    let screen = until(&mut h, "ready");
    assert!(f.asked.lock().unwrap().contains(&"Claude acme".to_string()));
    assert!(screen.contains("fake-agent in acme"), "{screen}");

    // Typed text reaches it (and the terminal shows it, as a terminal does).
    h.event(egui::Event::Text("hello".into()));
    h.key_press(Key::Enter);
    until(&mut h, "you-said: hello");

    // Ctrl-C reaches the agent as an interrupt, and it goes on.
    h.key_press_modifiers(Modifiers::CTRL, Key::C);
    until(&mut h, "got-interrupt");

    // A resize redraws: the program is told the new size.
    let before_size = h.state_mut().dock_mut().cells().unwrap();
    h.set_size(egui::vec2(800.0, 900.0));
    h.run_steps(4);
    let after = h.state_mut().dock_mut().cells().unwrap();
    assert!(
        after.cols < before_size.cols,
        "{before_size:?} -> {after:?}"
    );
    until(&mut h, &format!("size {} {}", after.rows, after.cols));

    shot(&mut h, "dock-running");

    // Closing the window ends the agent with it (the session is dropped).
    drop(h);
    // The dock wrote nothing of the books.
    assert_eq!(std::fs::read(book.join("journal.jsonl")).ok(), before);
}

#[test]
fn agent_missing_offers_the_installer() {
    let _alone = alone();
    let f = fake();
    *f.answer.lock().unwrap() = Some(Ok(AgentStart::Missing(Installer {
        label: "Claude Code".into(),
        command: "curl -fsSL https://claude.ai/install.sh | bash".into(),
        page: "https://code.claude.com/docs/en/setup".into(),
    })));
    let (_dir, _book, b) = books();
    let mut h = harness(b);
    h.state_mut().dock_mut().open = true;
    for _ in 0..100 {
        h.run_steps(2);
        if h.query_by_label_contains("isn't installed here").is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    h.get_by_label_contains("Claude Code isn't installed here");
    h.get_by_label_contains("curl -fsSL https://claude.ai/install.sh | bash");
    h.get_by_label_contains("Copy");
    shot(&mut h, "dock-missing");
    // Nothing runs; nothing was installed by the app.
    assert!(h.state_mut().dock_mut().screen().is_none());
}

#[test]
fn a_chosen_agent_and_an_open_dock_are_kept() {
    let _alone = alone();
    fake();
    // Preferences: the agent and whether it is open are the viewer's.
    let kept = |k: &str| match k {
        "dock_agent" => Some("codex".to_string()),
        "dock_open" => Some("true".to_string()),
        _ => None,
    };
    let dock = ledger_gui::dock::Dock::restored(kept);
    assert_eq!(dock.agent, Agent::Codex);
    assert!(dock.open);
    let mut seen = Vec::new();
    dock.keep(&mut |k, v| seen.push((k.to_string(), v)));
    assert!(seen.contains(&("dock_agent".to_string(), "codex".to_string())));
    assert!(seen.contains(&("dock_open".to_string(), "true".to_string())));
    let none = ledger_gui::dock::Dock::restored(|_| None);
    assert_eq!(none.agent, Agent::Claude, "Claude Code is the default");
    assert!(!none.open);
}
