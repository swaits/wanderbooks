//! The live loader. A background thread owns the read-only [`Snapshot`],
//! watches the data directory, and on every change applies the new journal
//! lines and sends a fresh [`Books`] to the UI, then asks for a repaint.
//! A change made by the agent in another window shows up within a frame or
//! two. Filesystem events are the trigger; a slow poll backs them up in case
//! an event is missed (network drives, odd editors).

use crate::books::{Books, Integrity};
use chrono::NaiveDate;
use ledger_config::BooksSettings;
use ledger_store::{Snapshot, StoreError};
use notify::{RecursiveMode, Watcher};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::{Duration, SystemTime};

/// Where the books come from.
#[derive(Clone, Debug)]
pub struct Source {
    /// The `books-data` directory (journal and blobs).
    pub data_dir: PathBuf,
    /// The company's `wanderbooks.toml`, if any.
    pub books_toml: Option<PathBuf>,
    /// Fix "today" (tests, screenshots); otherwise the local date.
    pub today: Option<NaiveDate>,
}

pub enum Update {
    Books(Arc<Books>),
    Error(String),
    /// The agent asked the viewer to show something (`books.show`).
    Show(ledger_store::viewer::ViewerTarget),
}

pub struct Loader {
    rx: Receiver<Update>,
}

impl Loader {
    pub fn spawn(source: Source, ctx: egui::Context) -> Loader {
        let (tx, rx) = channel();
        std::thread::Builder::new()
            .name("books-loader".into())
            .spawn(move || run(source, ctx, tx))
            .expect("spawn loader");
        Loader { rx }
    }

    /// Every update since the last call, in order (books before the
    /// navigation that refers to them).
    pub fn poll(&self) -> Vec<Update> {
        self.rx.try_iter().collect()
    }

    /// Block until the first update (tests, headless).
    pub fn wait(&self, timeout: Duration) -> Option<Update> {
        self.rx.recv_timeout(timeout).ok()
    }
}

/// Load once, synchronously (tests and one-shot tools).
pub fn load_once(source: &Source) -> Result<Books, String> {
    let snap = Snapshot::load(&source.data_dir).map_err(|e| {
        problem(&e, || {
            format!("Can't read the books in {}", source.data_dir.display())
        })
    })?;
    let integrity = Integrity::check(&snap, &mut HashSet::new());
    let settings = settings(source, &snap)?;
    let today = today(source, &settings);
    Books::build(&snap, settings, today, None, integrity)
}

/// What the viewer says when the books can't be read. Books from a newer
/// wanderbooks aren't broken, so they get the plain "update" message on its
/// own, never a failure framing; anything else is `context: error`.
pub fn problem(e: &StoreError, context: impl FnOnce() -> String) -> String {
    match e {
        StoreError::NewerJournal { .. } => {
            let text = e.to_string();
            let mut chars = text.chars();
            chars
                .next()
                .map(|c| c.to_uppercase().chain(chars).collect())
                .unwrap_or_default()
        }
        _ => format!("{}: {e}", context()),
    }
}

/// Today: as fixed for tests, else in the company's time zone, else the
/// computer's.
fn today(source: &Source, settings: &BooksSettings) -> NaiveDate {
    source
        .today
        .unwrap_or_else(|| ledger_core::settings::today_in(settings.timezone.as_deref()))
}

/// The settings: the books' own when they keep them (the settings file is
/// then never read, as the server never reads it), else the settings
/// file's.
fn settings(source: &Source, snap: &Snapshot) -> Result<BooksSettings, String> {
    let books = snap.ledger().settings();
    if books.is_journaled() {
        return Ok(ledger_config::codec::books_settings(
            books,
            snap.ledger().base_currency().code(),
        ));
    }
    match &source.books_toml {
        Some(p) if p.exists() => ledger_config::load(p),
        _ => Ok(BooksSettings::default()),
    }
}

fn mtime(p: &Option<PathBuf>) -> Option<SystemTime> {
    p.as_ref()
        .and_then(|p| std::fs::metadata(p).ok())
        .and_then(|m| m.modified().ok())
}

fn run(source: Source, ctx: egui::Context, tx: Sender<Update>) {
    let send = |u: Update| {
        let _ = tx.send(u);
        ctx.request_repaint();
    };
    let mut snap = loop {
        match Snapshot::load(&source.data_dir) {
            Ok(s) => break s,
            Err(e) => {
                send(Update::Error(problem(&e, || {
                    format!("Can't read the books in {}", source.data_dir.display())
                })));
                std::thread::sleep(Duration::from_secs(2));
            }
        }
    };
    let mut verified: HashSet<String> = HashSet::new();
    // Build the picture; also return the database check it was built with.
    let mut build = |snap: &Snapshot, prev: Option<u64>| -> (Update, ledger_store::DiskCheck) {
        let integrity = Integrity::check(snap, &mut verified);
        let disk = integrity.disk.clone();
        let update = match settings(&source, snap).and_then(|s| {
            let today = today(&source, &s);
            Books::build(snap, s, today, prev, integrity)
        }) {
            Ok(b) => Update::Books(Arc::new(b)),
            Err(e) => Update::Error(e),
        };
        (update, disk)
    };
    let (first, disk) = build(&snap, None);
    let mut last_disk = Some(disk);
    send(first);

    // Requests to show something: apply new ones, and on start one made in
    // the last two minutes (the agent said "open the viewer"); never an
    // old one.
    let started = chrono::Utc::now();
    let mut last_show = 0u64;
    let mut check_show = |send: &dyn Fn(Update)| {
        if let Some(cmd) = ledger_store::viewer::read(&source.data_dir)
            && cmd.seq != last_show
        {
            let first_look = last_show == 0;
            last_show = cmd.seq;
            let recent = chrono::DateTime::parse_from_rfc3339(&cmd.at)
                .is_ok_and(|t| started.signed_duration_since(t) < chrono::Duration::minutes(2));
            if !first_look || recent {
                send(Update::Show(cmd.target));
            }
        }
    };
    check_show(&send);

    let (ev_tx, ev_rx) = channel();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if res.is_ok() {
            let _ = ev_tx.send(());
        }
    })
    .ok();
    if let Some(w) = watcher.as_mut() {
        let _ = w.watch(&source.data_dir, RecursiveMode::NonRecursive);
        let cmd = ledger_store::viewer::command_path(&source.data_dir);
        if let Some(dir) = cmd.parent() {
            let _ = std::fs::create_dir_all(dir);
            let _ = w.watch(dir, RecursiveMode::NonRecursive);
        }
        if let Some(t) = &source.books_toml
            && let Some(dir) = t.parent()
        {
            let _ = w.watch(dir, RecursiveMode::NonRecursive);
        }
    }
    let mut toml_mtime = mtime(&source.books_toml);
    let day_now = |snap: &Snapshot| {
        settings(&source, snap)
            .map(|s| today(&source, &s))
            .unwrap_or_else(|_| today(&source, &BooksSettings::default()))
    };
    let mut day = day_now(&snap);
    loop {
        match ev_rx.recv_timeout(Duration::from_secs(3)) {
            Ok(()) => {
                // Let a burst of writes settle, then take them all at once.
                std::thread::sleep(Duration::from_millis(30));
                while ev_rx.try_recv().is_ok() {}
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                std::thread::sleep(Duration::from_secs(3));
            }
        }
        let before = snap.journal_seq();
        let grew = match snap.refresh() {
            Ok(n) => n > 0,
            Err(e) => {
                send(Update::Error(problem(&e, || {
                    "Reading the journal failed".into()
                })));
                continue;
            }
        };
        let toml_now = mtime(&source.books_toml);
        let new_day = day_now(&snap);
        // The writer journals first and updates its database right after:
        // a "behind" database usually catches up a moment later, so look
        // again rather than leave a stale warning up.
        let disk_changed = !grew
            && last_disk
                .as_ref()
                .is_some_and(|d| *d != ledger_store::DiskCheck::Matches && snap.check_disk() != *d);
        if grew || disk_changed || toml_now != toml_mtime || new_day != day {
            toml_mtime = toml_now;
            day = new_day;
            let (update, disk) = build(&snap, grew.then_some(before));
            last_disk = Some(disk);
            send(update);
        }
        check_show(&send);
    }
}
