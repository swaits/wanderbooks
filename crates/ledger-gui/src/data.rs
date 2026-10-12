//! The live loader. A background thread owns the read-only [`Snapshot`],
//! watches the data directory, and on every change applies the new journal
//! lines and sends a fresh [`Books`] to the UI, then asks for a repaint.
//! A change made by the agent in another window shows up within a frame or
//! two. Filesystem events are the trigger; a slow poll backs them up in case
//! an event is missed (network drives, odd editors).

use crate::books::Books;
use chrono::NaiveDate;
use ledger_config::BooksSettings;
use ledger_store::Snapshot;
use notify::{RecursiveMode, Watcher};
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

    /// The newest update, if any arrived since the last call.
    pub fn poll(&self) -> Option<Update> {
        let mut last = None;
        while let Ok(u) = self.rx.try_recv() {
            last = Some(u);
        }
        last
    }

    /// Block until the first update (tests, headless).
    pub fn wait(&self, timeout: Duration) -> Option<Update> {
        self.rx.recv_timeout(timeout).ok()
    }
}

/// Load once, synchronously (tests and one-shot tools).
pub fn load_once(source: &Source) -> Result<Books, String> {
    let snap = Snapshot::load(&source.data_dir).map_err(|e| e.to_string())?;
    Books::build(&snap, settings(source)?, today(source), None)
}

fn today(source: &Source) -> NaiveDate {
    source
        .today
        .unwrap_or_else(|| chrono::Local::now().date_naive())
}

fn settings(source: &Source) -> Result<BooksSettings, String> {
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
                send(Update::Error(format!(
                    "Can't read the books in {}: {e}",
                    source.data_dir.display()
                )));
                std::thread::sleep(Duration::from_secs(2));
            }
        }
    };
    let build = |snap: &Snapshot, prev: Option<u64>| -> Update {
        match settings(&source).and_then(|s| Books::build(snap, s, today(&source), prev)) {
            Ok(b) => Update::Books(Arc::new(b)),
            Err(e) => Update::Error(e),
        }
    };
    send(build(&snap, None));

    let (ev_tx, ev_rx) = channel();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if res.is_ok() {
            let _ = ev_tx.send(());
        }
    })
    .ok();
    if let Some(w) = watcher.as_mut() {
        let _ = w.watch(&source.data_dir, RecursiveMode::NonRecursive);
        if let Some(t) = &source.books_toml
            && let Some(dir) = t.parent()
        {
            let _ = w.watch(dir, RecursiveMode::NonRecursive);
        }
    }
    let mut toml_mtime = mtime(&source.books_toml);
    let mut day = today(&source);
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
                send(Update::Error(format!("Reading the journal failed: {e}")));
                continue;
            }
        };
        let toml_now = mtime(&source.books_toml);
        let new_day = today(&source);
        if grew || toml_now != toml_mtime || new_day != day {
            toml_mtime = toml_now;
            day = new_day;
            send(build(&snap, grew.then_some(before)));
        }
    }
}
