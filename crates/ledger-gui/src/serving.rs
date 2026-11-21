//! The window's line to the books server (plan P15 3.1; D-77, D-78).
//!
//! ```text
//! GOAL: a tab is a company open in the books server, and the server is
//!       the app's child: it starts with the companies the window
//!       restored, a tab opened or closed opens or closes that company in
//!       it, and closing the window closes it, in view and in bounded
//!       time. What fails is said to the person, never swallowed.
//! SOURCE: plan/p15-app-and-relay.md 3.1 (the app starts the server and
//!         closes it by closing its input; opening or closing a tab is
//!         the server opening or closing that book), 3.2, D-77, D-78,
//!         WB-app-2, WB-app-4.
//! INVARIANTS:
//!   1. Nothing here touches the UI thread's time: every request to the
//!      server runs on a thread of its own, and the window only reads
//!      the notices those threads leave.
//!   2. A company's requests are one queue, in the order they were made
//!      (a tab closed and opened again is a close, then an open); no
//!      company's request waits for another company's, so closing one
//!      company's tab, which makes its last backup, delays no other.
//!   3. No request is made before the server has started, and none after
//!      a start that failed (the person was told why, once).
//!   4. A company the window opened at the start is not opened again; a
//!      company whose open failed is asked for again the next time its
//!      tab opens.
//!   5. A failed request, a close whose last backup wasn't whole, and a
//!      server that has exited on its own are each a notice, in words,
//!      once; the window shows them until they are dismissed.
//!   6. The server is closed once: by `begin_stop` (the window asked to
//!      close, with the wait in view), else when this is dropped. Neither
//!      returns before the owner's bounded close has, and neither kills
//!      anything.
//!   7. Without an owner that hosts a server (the viewer on its own, the
//!      demo, tests) it does nothing at all.
//! ```

use crate::owner::Owner;
use std::collections::{BTreeSet, HashMap};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

/// Where the server's start stands (INVARIANT 3).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Gate {
    Starting,
    Up,
    Down,
}

/// Where the server's close stands (INVARIANT 6).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Closing {
    Not,
    Running,
    Done,
}

enum Cmd {
    Open,
    Close,
}

#[derive(Default)]
struct State {
    /// The companies the server has open, or is being asked to have.
    wanted: BTreeSet<String>,
    /// What the person has not been told yet.
    notices: Vec<String>,
    /// One queue per company (INVARIANT 2).
    lanes: HashMap<String, Sender<Cmd>>,
    /// The server's exit has been said.
    exit_said: bool,
}

struct Inner {
    owner: Arc<dyn Owner>,
    gate: Mutex<Gate>,
    gate_changed: Condvar,
    closing: Mutex<Closing>,
    closing_changed: Condvar,
    state: Mutex<State>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Inner {
    fn note(&self, words: String) {
        lock(&self.state).notices.push(words);
    }

    /// Wait for the start to have come out: whether the server is up.
    fn up(&self) -> bool {
        let mut gate = lock(&self.gate);
        while *gate == Gate::Starting {
            gate = self
                .gate_changed
                .wait(gate)
                .unwrap_or_else(|e| e.into_inner());
        }
        *gate == Gate::Up
    }

    /// Close the server, once; the others wait for that one to be done.
    fn close_server(&self) {
        {
            let mut closing = lock(&self.closing);
            if *closing != Closing::Not {
                while *closing != Closing::Done {
                    closing = self
                        .closing_changed
                        .wait(closing)
                        .unwrap_or_else(|e| e.into_inner());
                }
                return;
            }
            *closing = Closing::Running;
        }
        match self.owner.serve_stop() {
            Ok(None) => {}
            Ok(Some(words)) => {
                // The window may be gone before anyone reads a notice.
                eprintln!("wanderbooks: {words}");
                self.note(words);
            }
            Err(why) => self.note(format!(
                "The books server could not be closed cleanly: {why}"
            )),
        }
        *lock(&self.closing) = Closing::Done;
        self.closing_changed.notify_all();
    }
}

/// The window's line to the server; one per window.
pub struct Serving {
    inner: Option<Arc<Inner>>,
}

impl Serving {
    /// No server: nothing is asked of any (INVARIANT 7).
    pub fn off() -> Self {
        Self { inner: None }
    }

    /// Start the server with the companies `open` open, if `owner` hosts
    /// one; the start runs on a thread of its own (INVARIANT 1).
    pub fn start(owner: Option<Arc<dyn Owner>>, open: Vec<String>) -> Self {
        let Some(owner) = owner.filter(|o| o.serves()) else {
            return Self::off();
        };
        let inner = Arc::new(Inner {
            owner,
            gate: Mutex::new(Gate::Starting),
            gate_changed: Condvar::new(),
            closing: Mutex::new(Closing::Not),
            closing_changed: Condvar::new(),
            state: Mutex::new(State {
                wanted: open.iter().cloned().collect(),
                ..State::default()
            }),
        });
        let starting = inner.clone();
        let spawned = std::thread::Builder::new()
            .name("serving-start".into())
            .spawn(move || {
                let came = match starting.owner.serve_start(&open) {
                    Ok(()) => Gate::Up,
                    Err(why) => {
                        starting.note(format!("Assistants can't reach your companies: {why}"));
                        Gate::Down
                    }
                };
                *lock(&starting.gate) = came;
                starting.gate_changed.notify_all();
            });
        if spawned.is_err() {
            inner.note(
                "Assistants can't reach your companies: the books server could not be started."
                    .into(),
            );
            *lock(&inner.gate) = Gate::Down;
        }
        Self { inner: Some(inner) }
    }

    /// Whether a server is being hosted for this window.
    pub fn is_on(&self) -> bool {
        self.inner.is_some()
    }

    /// A tab for the company `id` opened: ask the server to open it, unless
    /// it has it (INVARIANT 4).
    pub fn open(&self, id: &str) {
        let Some(inner) = &self.inner else { return };
        let fresh = lock(&inner.state).wanted.insert(id.to_string());
        if fresh {
            Self::send(inner, id, Cmd::Open);
        }
    }

    /// The tab of the company `id` closed: ask the server to close it.
    pub fn close(&self, id: &str) {
        let Some(inner) = &self.inner else { return };
        let had = lock(&inner.state).wanted.remove(id);
        if had {
            Self::send(inner, id, Cmd::Close);
        }
    }

    /// Whether the server has, or is being asked to have, `id` open
    /// (tests).
    pub fn wants(&self, id: &str) -> bool {
        self.inner
            .as_ref()
            .is_some_and(|i| lock(&i.state).wanted.contains(id))
    }

    /// Queue `cmd` for the company `id` on its own thread, made the first
    /// time (INVARIANT 2).
    fn send(inner: &Arc<Inner>, id: &str, cmd: Cmd) {
        let mut state = lock(&inner.state);
        let mut cmd = cmd;
        if let Some(lane) = state.lanes.get(id) {
            match lane.send(cmd) {
                Ok(()) => return,
                // Its thread is gone: the command comes back, for a new one.
                Err(back) => cmd = back.0,
            }
        }
        let (tx, rx) = channel::<Cmd>();
        let worker = inner.clone();
        let book = id.to_string();
        let started = std::thread::Builder::new()
            .name(format!("serving-{id}"))
            .spawn(move || {
                for cmd in rx {
                    // INVARIANT 3.
                    if !worker.up() {
                        continue;
                    }
                    match cmd {
                        Cmd::Open => match worker.owner.serve_open(&book) {
                            Ok(None) => {}
                            Ok(Some(words)) => worker.note(words),
                            Err(why) => {
                                lock(&worker.state).wanted.remove(&book);
                                worker.note(why);
                            }
                        },
                        Cmd::Close => match worker.owner.serve_close(&book) {
                            Ok(None) => {}
                            Ok(Some(words)) => worker.note(words),
                            Err(why) => worker.note(format!(
                                "{book} could not be closed to your assistants: {why}"
                            )),
                        },
                    }
                }
            });
        match started {
            Ok(_) => {
                let _ = tx.send(cmd);
                state.lanes.insert(id.to_string(), tx);
            }
            Err(_) => {
                drop(state);
                inner.note(format!(
                    "Assistants may not be able to reach {id}: a thread could not be started."
                ));
            }
        }
    }

    /// What the person has not been told yet (INVARIANT 5), and, once,
    /// that the server exited on its own.
    pub fn take_notices(&self) -> Vec<String> {
        let Some(inner) = &self.inner else {
            return Vec::new();
        };
        let closing = *lock(&inner.closing) != Closing::Not;
        let mut state = lock(&inner.state);
        if !state.exit_said
            && !closing
            && let Some(why) = inner.owner.serve_exited()
        {
            state.exit_said = true;
            state.notices.push(format!(
                "{why}. Assistants can't reach your companies until Wanderbooks is started again."
            ));
        }
        std::mem::take(&mut state.notices)
    }

    /// The window was asked to close: close the server on a thread of its
    /// own, so the window can say so while it does (INVARIANT 6).
    pub fn begin_stop(&self) {
        let Some(inner) = &self.inner else { return };
        if *lock(&inner.closing) != Closing::Not {
            return;
        }
        let inner = inner.clone();
        let spawned = std::thread::Builder::new()
            .name("serving-stop".into())
            .spawn({
                let inner = inner.clone();
                move || inner.close_server()
            });
        if spawned.is_err() {
            inner.close_server();
        }
    }

    /// Whether the close has begun.
    pub fn is_closing(&self) -> bool {
        self.inner
            .as_ref()
            .is_some_and(|i| *lock(&i.closing) != Closing::Not)
    }

    /// Whether the close is done (or there is no server to close).
    pub fn is_closed(&self) -> bool {
        self.inner
            .as_ref()
            .is_none_or(|i| *lock(&i.closing) == Closing::Done)
    }
}

impl Drop for Serving {
    /// The window is gone without having been asked to close first (a
    /// signal, a test): the server is closed here, in bounded time, with
    /// a line on stderr (INVARIANT 6).
    fn drop(&mut self) {
        let Some(inner) = &self.inner else { return };
        if *lock(&inner.closing) == Closing::Not {
            eprintln!("wanderbooks: closing the books before it exits");
        }
        inner.close_server();
    }
}
