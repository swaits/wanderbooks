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
use std::time::Duration;

/// Where the books come from: their folder, a book of the data root
/// (`wanderbooks gui ID`; D-44). Their settings are their own, in their
/// journal (plan 3.6), read as the server reads them.
#[derive(Clone, Debug)]
pub struct Source {
    /// The books' folder (journal and blobs).
    pub data_dir: PathBuf,
    /// Fix "today" (tests, screenshots); otherwise the local date.
    pub today: Option<NaiveDate>,
}

pub enum Update {
    Books(Arc<Books>),
    Error(String),
    /// The agent asked the viewer to show something (`books.show`).
    Show(ledger_store::ops::ViewerTarget),
}

pub struct Loader {
    rx: Receiver<Update>,
    /// Set when this is dropped: the loader's thread then ends, letting go
    /// of the books (and the viewer's presence on them).
    gone: Arc<std::sync::atomic::AtomicBool>,
}

impl Drop for Loader {
    fn drop(&mut self) {
        self.gone.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

impl Loader {
    pub fn spawn(source: Source, ctx: egui::Context) -> Loader {
        let (tx, rx) = channel();
        let gone = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stop = gone.clone();
        std::thread::Builder::new()
            .name("books-loader".into())
            .spawn(move || run(source, ctx, tx, &stop))
            .expect("spawn loader");
        Loader { rx, gone }
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
    let settings = settings(&snap);
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

/// The settings: the books' own, from their journal, as the server reads
/// them (the defaults until the first is set).
pub fn settings(snap: &Snapshot) -> BooksSettings {
    ledger_config::codec::books_settings(
        snap.ledger().settings(),
        snap.ledger().base_currency().code(),
    )
}

fn run(
    source: Source,
    ctx: egui::Context,
    tx: Sender<Update>,
    gone: &std::sync::atomic::AtomicBool,
) {
    let gone = || gone.load(std::sync::atomic::Ordering::SeqCst);
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
                if gone() {
                    return;
                }
                std::thread::sleep(Duration::from_secs(2));
            }
        }
    };
    let mut verified: HashSet<String> = HashSet::new();
    // Build the picture; also return the database check it was built with.
    let mut build = |snap: &Snapshot, prev: Option<u64>| -> (Update, ledger_store::DiskCheck) {
        let integrity = Integrity::check(snap, &mut verified);
        let disk = integrity.disk.clone();
        let s = settings(snap);
        let today = today(&source, &s);
        let update = match Books::build(snap, s, today, prev, integrity) {
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
        if let Some(cmd) = ledger_store::ops::read(&source.data_dir)
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
    }
    // The books' `ops/`, once the server has made it (the viewer makes
    // nothing): its show requests watched, and this viewer's presence
    // held there while it runs (ledger_store::ops INVARIANT 3).
    let ops = ledger_store::ops::ops_dir(&source.data_dir);
    let mut ops_watched = false;
    let mut presence: Option<ledger_store::ops::Presence> = None;
    let mut look_at_ops = |watcher: &mut Option<notify::RecommendedWatcher>| {
        if !ops_watched
            && ops.is_dir()
            && let Some(w) = watcher.as_mut()
        {
            ops_watched = w.watch(&ops, RecursiveMode::NonRecursive).is_ok();
        }
        if presence.is_none() {
            presence = ledger_store::ops::Presence::hold(&source.data_dir);
        }
    };
    look_at_ops(&mut watcher);
    let day_now = |snap: &Snapshot| today(&source, &settings(snap));
    let mut day = day_now(&snap);
    loop {
        if gone() {
            return;
        }
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
        look_at_ops(&mut watcher);
        let new_day = day_now(&snap);
        // The writer journals first and updates its database right after:
        // a "behind" database usually catches up a moment later, so look
        // again rather than leave a stale warning up.
        let disk_changed = !grew
            && last_disk
                .as_ref()
                .is_some_and(|d| *d != ledger_store::DiskCheck::Matches && snap.check_disk() != *d);
        if grew || disk_changed || new_day != day {
            day = new_day;
            let (update, disk) = build(&snap, grew.then_some(before));
            last_disk = Some(disk);
            send(update);
        }
        check_show(&send);
    }
}
