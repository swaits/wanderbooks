//! The window: a tab bar with one tab per open company, and the company
//! in front (company.rs).
//!
//! ```text
//! GOAL: one window, a tab for each company that is open, each tab a
//!       window on that company alone; what the app keeps of its own
//!       (the color scheme, the dock's preferences) is kept once.
//! SOURCE: plan/p15-app-and-relay.md 3.2 (tabs: `App` holds the per-app
//!         state and a `CompanyView` per tab); D-78 (one top-level tab per
//!         open company); 3.1 (a tab is a company open in the books
//!         server the app runs); WB-app-2, WB-app-4.
//! INVARIANTS:
//!   1. One view per open company, no company twice: opening one that is
//!      open shows its tab instead, and start-up opens each company of the
//!      kept list once.
//!   2. Closing a tab drops its view, and with it its loader (its
//!      watcher and its viewer presence), its receipts' workers and its
//!      dock's agent; the other tabs are as they were.
//!   3. Start: a company named on the command line is the only tab;
//!      otherwise the companies open when the app last closed, in their
//!      order, the same one in front; a company that is gone is skipped
//!      without a word, and when none remains the data root's first opens.
//!      A session started with a company named never writes the list of
//!      open companies, whatever it opens or closes: a plain start's list
//!      is not replaced by a one-tab session's. (Each company's own page
//!      and period are kept in both.)
//!   4. What is kept for next time is kept for a company of a data root,
//!      by the root and the id: the open companies (`open_companies.<R>`,
//!      the ids of their books, comma separated) and the one in front
//!      (`active_company.<R>`), `R` being a hash of the root's canonical
//!      path (`company::path_hash`); for each company its page, its
//!      report's period, columns and comparison, its name as last known
//!      and the identity of its books (`company.<R>.<id>.…`; books in a
//!      folder outside any root: `company.dir.<hash of the folder>.…`);
//!      and, under their own keys, the scheme and its filter (`scheme`,
//!      `scheme_filter`, `dark`) and the dock's preferences (`dock_open`,
//!      `dock_agent`, the front tab's: the app's). Another root holding a
//!      company of the same id, or the same id made again, is not this
//!      company: a stored state is used only for the books it was kept
//!      for, which each company's identity says once it loads
//!      (company.rs INVARIANT 5): a stored identity that is not the
//!      loaded books' discards the stored state and the remembered name,
//!      and they are kept anew for these books. Files written before the
//!      keys carried a root (`open_companies`, `company.<id>.…`) are
//!      read without error and not used.
//!   5. The window's title names the company in front.
//!   7. While a job that changes the books runs in any tab (an owner's
//!      restore or format upgrade: owner.rs INVARIANT 6), the window is
//!      not closed and that tab's close control is off, saying what is
//!      running; a request to close the window is cancelled and a dialog
//!      says what is running and offers only to wait; it goes when the
//!      job ends, and its result is shown in the tab. A read-only job
//!      (an export, a backup) is stopped with its tab, with no question.
//!   6. The viewer stays read-only: a tab not in front reads what a tab
//!      in front reads, and writes nothing either.
//!   8. A tab is a company open in the books server the program runs for
//!      the window, when it runs one (serving.rs, WB-app-2): the
//!      companies restored at the start are opened with the server, a
//!      company opened later when its tab opens, and closed when its tab
//!      closes. What fails, and a server that stops on its own, is a
//!      notice under the tab bar, in words, until dismissed. Where the
//!      session list and the connection states will go (A4) is
//!      `Self::connection_slot`.
//!   9. Asked to close, a window with a server stays up, saying it is
//!      closing the books, until the server has exited or its bounded
//!      close has run out (and then says what is left); a close is never
//!      begun while a job that changes the books runs (7) (WB-app-4).
//!      When the close says something was not whole (a company's last
//!      backup, a server that exited badly), the window stays up showing
//!      it, with a "Close anyway" button; closing again is not that.
//!      A server still starting is said under the tab bar, and one whose
//!      start failed has a "Try again" button there (WB-app-2).
//! ```

use crate::company::CompanyView;
use crate::data::Source;
use crate::nav::Route;
use crate::scheme_menu::SchemeMenu;
use crate::serving::Serving;
use crate::tabs::{Listed, Tab, TabAction, TabBar};
use crate::theme::{self, pal};
use egui::{Frame, RichText, Ui};
use std::collections::HashMap;
use std::path::PathBuf;

pub use crate::company::{COMPACT_BELOW, PageState, Shell};
use crate::company::{kept_scope_of, path_hash};

/// Where the viewer keeps the companies that were open (the ids of their
/// books, comma separated), and the one in front: under these names, then
/// the data root (`open_key`, `active_key`; INVARIANT 4).
pub const OPEN_KEY: &str = "open_companies";
pub const ACTIVE_KEY: &str = "active_company";

/// The key the open companies of the data root at `root` are kept under.
pub fn open_key(root: &std::path::Path) -> String {
    format!("{OPEN_KEY}.{}", path_hash(root))
}

/// The key the company in front, of the data root at `root`, is kept under.
pub fn active_key(root: &std::path::Path) -> String {
    format!("{ACTIVE_KEY}.{}", path_hash(root))
}

/// The key `thing` ("page", "range", "columns", "compare", "identity",
/// "name") of the company whose books are in `data_dir` is kept under.
pub fn company_key(data_dir: &std::path::Path, thing: &str) -> String {
    stored(&kept_scope_of(data_dir), thing)
}

fn stored(scope: &str, thing: &str) -> String {
    format!("company.{scope}.{thing}")
}

/// How the app starts.
pub enum Start {
    /// This company alone: the one named on the command line, or the demo.
    Only(Source),
    /// The companies kept from last time, of the data root `root`; if none
    /// of them is there any more, `fallback` (the root's first).
    Restore { root: PathBuf, fallback: Source },
}

pub struct App {
    /// The color scheme and its menu: the app's, whatever tab is shown.
    schemes: SchemeMenu,
    /// One view per open company, in the order of the tabs.
    views: Vec<CompanyView>,
    /// The tab in front, an index of `views` (0 when there are none).
    active: usize,
    /// The tab bar's memory.
    tabs: TabBar,
    /// The data root the companies are in: "+" lists its companies.
    root: Option<PathBuf>,
    /// "Today", fixed (tests, screenshots) or not.
    today: Option<chrono::NaiveDate>,
    /// What was kept for companies not open now (and for those that are,
    /// as they were last closed), by key.
    kept: HashMap<String, String>,
    /// Each company's name as last known, by the id of its books: kept for
    /// the tab's title before the books load and for the "+" list, which
    /// doesn't read every company's journal for it.
    names: HashMap<String, String>,
    /// The dock's preferences as they were kept, for a dock made while no
    /// tab has one.
    dock_prefs: (Option<String>, Option<String>),
    /// The title last given the window.
    title: String,
    /// The first-run page, while there are no books to show (welcome.rs).
    welcome: Option<crate::welcome::Welcome>,
    /// What a first run opens its books with: the data root named, and
    /// "today".
    first_run_source: (Option<PathBuf>, Option<chrono::NaiveDate>),
    /// The window was asked to close while a job that changes the books
    /// ran, and the dialog that says so is up (INVARIANT 7).
    close_dialog: bool,
    /// Whether the list of open companies is written when the app saves
    /// (INVARIANT 3: not after a start that named a company).
    writes_open_list: bool,
    /// The window asked to close and the server's close is being waited
    /// for: the window closes when it is done (INVARIANT 9).
    closing_window: bool,
    /// The books server the program runs for the window, if it runs one:
    /// a tab is a company open in it (INVARIANT 8).
    serving: Serving,
    /// What the server work has said that the person has not dismissed.
    notices: Vec<String>,
}

/// What a window is called when no company is in front.
const TITLE: &str = "wanderbooks";

impl App {
    /// The live app over one company's books directory.
    pub fn new(cc: &eframe::CreationContext<'_>, source: Source) -> Self {
        Self::start(cc, Start::Only(source))
    }

    /// The live app, started as `start` says (INVARIANT 3).
    pub fn start(cc: &eframe::CreationContext<'_>, start: Start) -> Self {
        let get = |key: &str| cc.storage.and_then(|s| s.get_string(key));
        Self::start_with(&cc.egui_ctx, &get, start)
    }

    /// [`Self::start`], with the viewer's kept preferences read by `get`
    /// (tests give them without a window).
    pub fn start_with(
        ctx: &egui::Context,
        get: &dyn Fn(&str) -> Option<String>,
        start: Start,
    ) -> Self {
        Self::start_with_owner(ctx, get, start, crate::owner::current())
    }

    /// [`Self::start_with`], with the program's owner given (tests give
    /// one that hosts a server of its own): the server it hosts starts
    /// with the companies this start opens (INVARIANT 8).
    pub fn start_with_owner(
        ctx: &egui::Context,
        get: &dyn Fn(&str) -> Option<String>,
        start: Start,
        owner: Option<std::sync::Arc<dyn crate::owner::Owner>>,
    ) -> Self {
        let mut app = Self::without_books(ctx, get);
        match start {
            Start::Only(source) => {
                app.today = source.today;
                app.writes_open_list = false;
                app.root = ledger_store::root::root_of_book_dir(&source.data_dir)
                    .map(std::path::Path::to_path_buf);
                app.remember_all(get);
                app.remember_scope(get, &kept_scope_of(&source.data_dir));
                // A company of a data root is served; the demo's is not.
                let id = crate::owner::book_id_of_dir(&source.data_dir);
                app.serving = Serving::start(owner, id.into_iter().collect());
                app.open(ctx, source);
            }
            Start::Restore { root, fallback } => {
                app.today = fallback.today;
                app.root = Some(root.clone());
                app.remember_all(get);
                let there = list_books(&root);
                let wanted: Vec<String> = get(&open_key(&root))
                    .map(|v| v.split(',').map(str::to_string).collect())
                    .unwrap_or_default();
                // The companies this start opens, in order (INVARIANT 3):
                // the server starts with exactly these open.
                let mut opened = Vec::new();
                let mut sources = Vec::new();
                for id in wanted {
                    // Gone, or listed twice: skipped, with no word.
                    if !there.contains(&id) || opened.contains(&id) {
                        continue;
                    }
                    let Ok(book) = ledger_store::root::BookId::parse(&id) else {
                        continue;
                    };
                    let Ok(r) = ledger_store::root::Root::find(Some(&root)) else {
                        break;
                    };
                    sources.push(Source {
                        data_dir: r.book_dir(&book),
                        today: app.today,
                    });
                    opened.push(id);
                }
                app.serving = Serving::start(owner, opened.clone());
                for source in sources {
                    app.open(ctx, source);
                }
                if app.views.is_empty() {
                    app.open(ctx, fallback);
                } else {
                    // The one in front last time, else the first.
                    app.active = get(&active_key(&root))
                        .and_then(|front| opened.iter().position(|id| *id == front))
                        .unwrap_or(0);
                    // The dock left open was the front tab's.
                    let front = app.active;
                    for (i, v) in app.views.iter_mut().enumerate() {
                        if i != front {
                            v.dock_mut().open = false;
                        }
                    }
                }
            }
        }
        app
    }

    /// The app on a first run: no books yet, so the page that sets them up
    /// (welcome.rs), which opens them once they are made. `root` is the data
    /// root named on the command line, if any.
    pub fn first_run(
        cc: &eframe::CreationContext<'_>,
        root: Option<PathBuf>,
        today: Option<chrono::NaiveDate>,
        state: crate::owner::FirstRun,
    ) -> Self {
        let mut app = Self::new_without_books(cc);
        app.welcome = Some(crate::welcome::Welcome::new(state));
        app.first_run_source = (root, today);
        app
    }

    /// An app with no company open: the scheme kept from last time (before
    /// schemes, a light/dark choice; else the system's mode: schemes
    /// INVARIANT 4) and the dock as it was left.
    pub fn new_without_books(cc: &eframe::CreationContext<'_>) -> Self {
        let get = |key: &str| cc.storage.and_then(|s| s.get_string(key));
        Self::without_books(&cc.egui_ctx, &get)
    }

    fn without_books(ctx: &egui::Context, get: &dyn Fn(&str) -> Option<String>) -> Self {
        let schemes = SchemeMenu::restored(get, ctx.global_style().visuals.dark_mode);
        let mut app = Self::bare(ctx, schemes);
        app.dock_prefs = (get("dock_open"), get("dock_agent"));
        app
    }

    /// The first-run page, if it is up (tests).
    pub fn welcome(&self) -> Option<&crate::welcome::Welcome> {
        self.welcome.as_ref()
    }

    /// An app showing fixed books (tests and screenshots): one tab.
    pub fn with_books(
        ctx: &egui::Context,
        books: crate::books::Books,
        dark: bool,
        start: Route,
    ) -> Self {
        let mut app = Self::bare(ctx, SchemeMenu::new(crate::schemes::own(dark)));
        app.add_view(CompanyView::with_books(ctx, books, start));
        app
    }

    /// An app with no company open, wearing `schemes`.
    fn bare(ctx: &egui::Context, schemes: SchemeMenu) -> Self {
        theme::install_fonts(ctx);
        theme::apply_scheme(ctx, schemes.in_force());
        egui_extras::install_image_loaders(ctx);
        Self {
            schemes,
            views: Vec::new(),
            active: 0,
            tabs: TabBar::default(),
            root: None,
            today: None,
            kept: HashMap::new(),
            names: HashMap::new(),
            dock_prefs: (None, None),
            title: TITLE.to_string(),
            welcome: None,
            first_run_source: (None, None),
            close_dialog: false,
            writes_open_list: true,
            closing_window: false,
            serving: Serving::off(),
            notices: Vec::new(),
        }
    }

    // ---- the tabs --------------------------------------------------------

    /// The views, in the order of the tabs.
    pub fn views(&self) -> &[CompanyView] {
        &self.views
    }

    /// Tab `i`'s view (tests).
    pub fn view_mut(&mut self, i: usize) -> Option<&mut CompanyView> {
        self.views.get_mut(i)
    }

    /// The tab in front, if any is open.
    pub fn active(&self) -> Option<usize> {
        (!self.views.is_empty()).then_some(self.active)
    }

    /// The view in front, if any is open.
    pub fn active_view(&self) -> Option<&CompanyView> {
        self.views.get(self.active)
    }

    /// The view in front, if any is open.
    pub fn active_view_mut(&mut self) -> Option<&mut CompanyView> {
        self.views.get_mut(self.active)
    }

    /// Bring tab `i` to the front; an index that isn't a tab changes
    /// nothing.
    pub fn activate(&mut self, i: usize) {
        if i < self.views.len() && i != self.active {
            self.active = i;
            self.tabs.reveal();
        }
    }

    /// The jobs that change the books, running now in any tab, as the
    /// window is held open for them: each tab's name and what runs there
    /// (INVARIANT 7).
    pub fn holding(&self) -> Vec<(String, String)> {
        self.views
            .iter()
            .filter_map(|v| Some((self.title_of(v), v.closing_is_held()?)))
            .collect()
    }

    /// Whether the dialog that says the window is held open is up (tests).
    pub fn close_dialog_is_up(&self) -> bool {
        self.close_dialog
    }

    /// A request to close the window while a job that changes the books
    /// runs is cancelled and the dialog that says why is put up; the
    /// dialog goes when the job ends (INVARIANT 7).
    fn guard_close(&mut self, ctx: &egui::Context) {
        let held = self.holding();
        if held.is_empty() {
            self.close_dialog = false;
            return;
        }
        if ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.close_dialog = true;
        }
        if !self.close_dialog {
            return;
        }
        egui::Modal::new(egui::Id::new("wanderbooks-close-held")).show(ctx, |ui| {
            ui.set_max_width(460.0);
            ui.heading("The books are being changed");
            ui.add_space(4.0);
            ui.label(
                "The window stays open until this is done, so that nothing is cut off half way:",
            );
            for (company, what) in &held {
                ui.label(format!("{company}: {what}"));
            }
            ui.add_space(6.0);
            if ui.button("Wait for it to finish").clicked() {
                self.close_dialog = false;
            }
        });
    }

    /// Asked to close, a window with a server stays up, saying it is
    /// closing the books, until the server has exited or its bounded
    /// close has run out (INVARIANT 9). A job that changes the books holds
    /// the window first (`guard_close`), and no close begins meanwhile.
    fn guard_server_close(&mut self, ctx: &egui::Context) {
        if !self.serving.is_on() || !self.holding().is_empty() {
            return;
        }
        let asked = ctx.input(|i| i.viewport().close_requested());
        // What the close said was not whole is read before the window goes
        // (INVARIANT 9): asking again is not reading it.
        let report = self.serving.close_report();
        if asked && (!self.serving.is_closed() || !report.is_empty()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.serving.begin_stop();
            self.closing_window = true;
        }
        if !self.closing_window {
            return;
        }
        if self.serving.is_closed() {
            if report.is_empty() {
                self.closing_window = false;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            }
            let mut close_anyway = false;
            egui::Modal::new(egui::Id::new("wanderbooks-closed-with-problems")).show(ctx, |ui| {
                ui.set_max_width(520.0);
                ui.heading("Your books did not close cleanly");
                ui.add_space(4.0);
                for line in &report {
                    ui.label(line);
                    ui.add_space(2.0);
                }
                ui.add_space(6.0);
                if ui.button("Close anyway").clicked() {
                    close_anyway = true;
                }
            });
            if close_anyway {
                self.serving.acknowledge_close();
                self.closing_window = false;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            return;
        }
        egui::Modal::new(egui::Id::new("wanderbooks-closing-books")).show(ctx, |ui| {
            ui.set_max_width(460.0);
            ui.heading("Closing your books");
            ui.add_space(4.0);
            ui.label(
                "Finishing what is running and making each company's last backup. \
                 This can take a minute; the window closes by itself.",
            );
            ui.add_space(6.0);
            ui.spinner();
        });
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }

    /// The notices from the server work, shown under the tab bar until
    /// dismissed (INVARIANT 8).
    fn show_notices(&mut self, ui: &mut Ui) {
        self.notices.extend(self.serving.take_notices());
        let starting = self.serving.is_starting();
        let down = self.serving.is_down();
        if self.notices.is_empty() && !starting && !down {
            return;
        }
        let p = pal(ui.ctx());
        let mut dismiss = None;
        let mut retry = false;
        egui::Panel::top("server_notices")
            .resizable(false)
            .frame(
                Frame::new()
                    .fill(p.warning_soft)
                    .inner_margin(egui::Margin::symmetric(12, 6)),
            )
            .show(ui, |ui| {
                if starting {
                    ui.horizontal_wrapped(|ui| {
                        ui.spinner();
                        ui.label(
                            RichText::new(
                                "Starting the books server for your assistants: another program may still be finishing with the books.",
                            )
                            .color(p.text),
                        );
                    });
                }
                for (i, words) in self.notices.iter().enumerate() {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(words).color(p.text));
                        if down && ui.button("Try again").clicked() {
                            retry = true;
                        }
                        if ui.button("Dismiss").clicked() {
                            dismiss = Some(i);
                        }
                    });
                }
                if down && self.notices.is_empty() && ui.button("Try again").clicked() {
                    retry = true;
                }
            });
        if retry {
            self.serving.retry();
        }
        if let Some(i) = dismiss {
            self.notices.remove(i);
        }
        if starting {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(500));
        }
    }

    /// What the server work has said that has not been dismissed (tests).
    pub fn notices(&mut self) -> &[String] {
        self.notices.extend(self.serving.take_notices());
        &self.notices
    }

    /// The server work of the window (tests).
    pub fn serving(&self) -> &Serving {
        &self.serving
    }

    /// Where the connection states and the list of who is connected will
    /// be shown (plan 3.3, unit A4); nothing is drawn yet.
    fn connection_slot(&mut self, _ui: &mut Ui) {}

    /// The window's title now.
    pub fn window_title(&self) -> String {
        match self.active_view() {
            Some(v) => format!("{} · {TITLE}", self.title_of(v)),
            None => TITLE.to_string(),
        }
    }

    /// Put `view` in the tabs and bring it to the front; the index of its
    /// tab. A company already open isn't opened twice (INVARIANT 1): its
    /// own tab comes to the front and `view` is dropped, ending what it
    /// started.
    pub fn add_view(&mut self, view: CompanyView) -> usize {
        if let Some(i) = self.views.iter().position(|v| v.key() == view.key()) {
            self.activate(i);
            return i;
        }
        let at = self.views.len();
        // What was kept for it (last run's, or from when it was closed
        // earlier in this one) comes back.
        let mut view = view;
        if let Some(scope) = view.kept_scope() {
            view.restore(&|k| self.kept.get(&stored(&scope, k)).cloned());
        }
        self.views.push(view);
        self.active = at;
        self.tabs.reveal();
        at
    }

    /// Open the company of `source` in a tab (or show its tab, if it is
    /// open): the index of its tab.
    pub fn open(&mut self, ctx: &egui::Context, source: Source) -> usize {
        if self.root.is_none() {
            self.root = ledger_store::root::root_of_book_dir(&source.data_dir)
                .map(std::path::Path::to_path_buf);
        }
        let key = crate::company::key_of(&source.data_dir);
        if let Some(i) = self.views.iter().position(|v| v.key() == key) {
            self.activate(i);
            return i;
        }
        let view = CompanyView::open(ctx, source).with_dock(self.new_dock());
        let at = self.add_view(view);
        // A tab is a company open in the server (INVARIANT 8): one this
        // start opened is open in it already.
        if let Some(id) = self.views[at].book_id() {
            self.serving.open(&id);
        }
        at
    }

    /// Close tab `i`: its view is dropped (INVARIANT 2). The tab after it,
    /// else the one before, comes to the front when it was the front one.
    /// A tab with a job that changes the books under way is not closed
    /// (INVARIANT 7).
    pub fn close(&mut self, i: usize) {
        if i >= self.views.len() {
            return;
        }
        // A job that changes the books is never abandoned: its tab stays.
        if self.views[i].closing_is_held().is_some() {
            return;
        }
        let view = self.views.remove(i);
        if let Some(scope) = view.kept_scope() {
            view.keep(&mut |k, v| {
                self.kept.insert(stored(&scope, k), v);
            });
        }
        // The company is closed in the server with its tab (INVARIANT 8).
        if let Some(id) = view.book_id() {
            self.serving.close(&id);
        }
        drop(view);
        if i < self.active || self.active >= self.views.len() {
            self.active = self.active.saturating_sub(1);
        }
        self.tabs.reveal();
    }

    /// A dock for a view just made: the front tab's agent and closed, else
    /// as kept (the first view of the app takes the dock as it was left).
    fn new_dock(&self) -> crate::dock::Dock {
        match self.active_view() {
            // The agent chosen, but closed: opening a company must not
            // start an agent in it that nobody asked for.
            Some(v) => crate::dock::Dock::closed_with(v.dock_ref().agent),
            None => {
                let (open, agent) = self.dock_prefs.clone();
                crate::dock::Dock::restored(|key| match key {
                    "dock_open" => open.clone(),
                    "dock_agent" => agent.clone(),
                    _ => None,
                })
            }
        }
    }

    /// Everything kept for the companies of the data root, so a company
    /// opened later comes back as it was.
    fn remember_all(&mut self, get: &dyn Fn(&str) -> Option<String>) {
        let Some(root) = self.root.clone() else {
            return;
        };
        let root_hash = path_hash(&root);
        for id in list_books(&root) {
            if let Some(name) = get(&stored(&format!("{root_hash}.{id}"), "name")) {
                self.names.insert(id.clone(), name);
            }
            self.remember_scope(get, &format!("{root_hash}.{id}"));
        }
    }

    /// What was kept under `scope` (a company's), for the view that opens
    /// for it.
    fn remember_scope(&mut self, get: &dyn Fn(&str) -> Option<String>, scope: &str) {
        for k in crate::company::KEPT_KEYS {
            let key = stored(scope, k);
            if let Some(v) = get(&key) {
                self.kept.insert(key, v);
            }
        }
    }

    /// Open the books `id` of the data root named, as the first-run page
    /// was asked to: the books come in as the loader reads them.
    fn open_first_books(&mut self, ctx: &egui::Context, id: &str) {
        let (root, today) = self.first_run_source.clone();
        match crate::book_in_root(root.as_deref(), Some(id)) {
            Ok(data_dir) => {
                self.today = today;
                // The first company of a first run: the server starts with
                // it (INVARIANT 8), if the program runs one.
                if !self.serving.is_on() {
                    self.serving = Serving::start(crate::owner::current(), vec![id.to_string()]);
                }
                self.open(ctx, Source { data_dir, today });
                self.welcome = None;
            }
            Err(why) => {
                if let Some(w) = self.welcome.as_mut() {
                    w.refused(why);
                }
            }
        }
    }

    /// The data root's companies that aren't open, with the names last
    /// known (reading the ids is all it does).
    fn not_open(&self) -> impl Fn() -> Vec<Listed> + use<> {
        let root = self.root.clone();
        let open: Vec<String> = self.views.iter().filter_map(CompanyView::book_id).collect();
        let names = self.names.clone();
        move || {
            root.as_deref()
                .map(list_books)
                .unwrap_or_default()
                .into_iter()
                .filter(|id| !open.contains(id))
                .map(|id| Listed {
                    name: names.get(&id).cloned(),
                    id,
                })
                .collect()
        }
    }

    /// Books that are not the books a company's kept state was kept for
    /// (their identity differs: made again under the id, or restored over
    /// it): the kept state and the remembered name are dropped, to be
    /// kept anew for these books (company.rs INVARIANT 5).
    fn check_identities(&mut self) {
        let mut stale = Vec::new();
        for v in &mut self.views {
            if v.check_identity() {
                stale.push((v.kept_scope(), v.book_id()));
            }
        }
        for (scope, id) in stale {
            if let Some(scope) = scope {
                let prefix = format!("company.{scope}.");
                self.kept.retain(|k, _| !k.starts_with(&prefix));
            }
            if let Some(id) = id {
                self.names.remove(&id);
            }
        }
    }

    /// Note the name of every company whose books are loaded.
    fn learn_names(&mut self) {
        for v in &self.views {
            if let (Some(id), Some(name)) = (v.book_id(), v.name())
                && self.names.get(&id) != Some(&name)
            {
                self.names.insert(id, name);
            }
        }
    }

    /// The name last known for the company with books `id` (tests).
    pub fn remembered_name(&self, id: &str) -> Option<&str> {
        self.names.get(id).map(String::as_str)
    }

    /// A view's tab title: the name its books carry, else the name last
    /// known, else the id.
    fn title_of(&self, v: &CompanyView) -> String {
        v.name()
            .or_else(|| self.names.get(&v.book_id()?).cloned())
            .unwrap_or_else(|| v.title())
    }

    /// Tab `i`'s title as the bar shows it (tests).
    pub fn tab_title(&self, i: usize) -> Option<String> {
        self.views.get(i).map(|v| self.title_of(v))
    }

    /// Do what the tab bar was asked.
    fn act(&mut self, ctx: &egui::Context, action: TabAction) {
        match action {
            TabAction::Show(i) => self.activate(i),
            TabAction::Close(i) => self.close(i),
            TabAction::Open(id) => {
                let Some(root) = self.root.clone() else {
                    return;
                };
                let Ok(book) = ledger_store::root::BookId::parse(&id) else {
                    return;
                };
                let Ok(r) = ledger_store::root::Root::find(Some(&root)) else {
                    return;
                };
                let source = Source {
                    data_dir: r.book_dir(&book),
                    today: self.today,
                };
                self.open(ctx, source);
            }
        }
    }

    // ---- the front tab, for tests and screenshots -------------------------

    /// The agent dock of the front tab (tests).
    ///
    /// # Panics
    /// With no tab open.
    pub fn dock_mut(&mut self) -> &mut crate::dock::Dock {
        self.views[self.active].dock_mut()
    }

    /// Navigate the front tab (tests).
    pub fn goto(&mut self, r: Route) {
        if let Some(v) = self.active_view_mut() {
            v.goto(r);
        }
    }

    /// Wear the color scheme kept as `key` ("Family/Variant"; tests).
    pub fn set_scheme(&mut self, ctx: &egui::Context, key: &str) {
        self.schemes.set_scheme(ctx, key);
    }

    /// The color scheme in force, as kept ("Family/Variant").
    pub fn scheme_key(&self) -> String {
        self.schemes.scheme_key()
    }

    /// The scheme being worn ("Family/Variant"): a previewed one while
    /// its row is hovered (tests).
    pub fn shown_scheme_key(&self) -> String {
        self.schemes.shown_scheme_key()
    }

    /// Keep thumbnails on disk in `dir` for the front tab (tests).
    pub fn keep_thumbs_in(&mut self, dir: PathBuf) {
        if let Some(v) = self.active_view_mut() {
            v.keep_thumbs_in(dir);
        }
    }

    /// The size in pixels of a file's thumbnail in the front tab, once
    /// drawn (tests).
    pub fn thumb_size(&self, sha: &str) -> Option<[usize; 2]> {
        self.active_view().and_then(|v| v.thumb_size(sha))
    }

    /// Block until every receipt on screen in the front tab has rendered
    /// (screenshots).
    pub fn settle(&mut self, ctx: &egui::Context) {
        if let Some(v) = self.active_view_mut() {
            v.settle(ctx);
        }
    }

    /// Go where the agent asked, in the front tab (`books.show`; tests).
    pub fn show(&mut self, target: ledger_store::ops::ViewerTarget) {
        if let Some(v) = self.active_view_mut() {
            v.show(target);
        }
    }

    /// Where the front tab is (tests).
    pub fn route(&self) -> &Route {
        match self.active_view() {
            Some(v) => v.route(),
            None => &Route::Overview,
        }
    }

    /// Label the front tab "Live", as it reads when following a journal
    /// (screenshots of what a running app looks like).
    pub fn show_live_label(&mut self) {
        if let Some(v) = self.active_view_mut() {
            v.show_live_label();
        }
    }

    /// The front tab's page state (tests).
    ///
    /// # Panics
    /// With no tab open.
    pub fn state_mut(&mut self) -> &mut PageState {
        self.views[self.active].state_mut()
    }

    /// The window with no company in it.
    fn empty(&self, ui: &mut Ui) {
        let p = pal(ui.ctx());
        egui::CentralPanel::no_frame()
            .frame(Frame::new().fill(p.bg))
            .show(ui, |ui| {
                ui.centered_and_justified(|ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space((ui.available_height() / 2.0 - 40.0).max(0.0));
                        ui.label(RichText::new("No company is open").font(theme::semibold(18.0)));
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new("Open a company with the + at the top of the window.")
                                .font(theme::regular(14.0))
                                .color(p.muted),
                        );
                    });
                });
            });
    }
}

/// The ids of the books in the data root at `root`, in order; none when
/// it can't be read.
fn list_books(root: &std::path::Path) -> Vec<String> {
    ledger_store::root::Root::find(Some(root))
        .and_then(|r| r.list_books())
        .map(|ids| ids.iter().map(|i| i.as_str().to_string()).collect())
        .unwrap_or_default()
}

impl Drop for App {
    /// The window is closing: every tab's threads are told to stop and
    /// waited for, so a dock's agent that ignored its hang-up is killed and
    /// reaped before the process ends and not left to outlive it. Bounded:
    /// the agent's grace and a little more.
    fn drop(&mut self) {
        let watches: Vec<_> = self.views.iter().map(CompanyView::jobs_watch).collect();
        self.views.clear();
        let limit = crate::dock::AGENT_GRACE + std::time::Duration::from_secs(3);
        for w in watches {
            w.wait_ended(limit);
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        // A first run: the page that sets the books up, until there are some.
        if self.welcome.is_some() && self.views.is_empty() {
            let p = pal(ui.ctx());
            let owner = crate::owner::current();
            let mut open = None;
            if let (Some(w), Some(owner)) = (self.welcome.as_mut(), owner) {
                egui::CentralPanel::no_frame()
                    .frame(Frame::new().fill(p.bg))
                    .show(ui, |ui| w.show(ui, &owner));
                open = w.take_open();
            }
            if let Some(id) = open {
                self.open_first_books(&ui.ctx().clone(), &id);
            }
            return;
        }
        let ctx = ui.ctx().clone();
        // Every view takes what its loader has sent, the one in front
        // and the others alike (views INVARIANT 3).
        for (i, v) in self.views.iter_mut().enumerate() {
            v.pump(&ctx, i == self.active);
        }
        self.guard_close(&ctx);
        self.guard_server_close(&ctx);
        self.check_identities();
        self.learn_names();
        let tabs: Vec<Tab> = self
            .views
            .iter()
            .map(|v| {
                let title = self.title_of(v);
                Tab {
                    held: v.closing_is_held(),
                    detail: match v.book_id() {
                        Some(id) if id != title => format!("{title}\n{id}"),
                        _ => title.clone(),
                    },
                    title,
                    hue_key: v.hue_key(),
                }
            })
            .collect();
        let shown = self.active();
        let others = self.not_open();
        // The bar goes across the whole window, above everything of the
        // company in front: a tab is the top level (D-78).
        let action = self.tabs.show(ui, &tabs, shown, &others);
        self.show_notices(ui);
        self.connection_slot(ui);
        let active = self.active;
        match self.views.get_mut(active) {
            Some(v) => v.ui(ui, &mut self.schemes),
            None => self.empty(ui),
        }
        if let Some(action) = action {
            self.act(&ctx, action);
        }
        // The title names the company in front.
        let title = self.window_title();
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.schemes.keep(&mut |k, v| storage.set_string(k, v));
        // The dock, as the front tab has it, else as it was kept.
        match self.active_view() {
            Some(v) => v.dock_ref().keep(&mut |k, v| storage.set_string(k, v)),
            None => {
                let (open, agent) = self.dock_prefs.clone();
                if let Some(v) = open {
                    storage.set_string("dock_open", v);
                }
                if let Some(v) = agent {
                    storage.set_string("dock_agent", v);
                }
            }
        }
        // The companies open, in order, and the one in front: not after a
        // start that named a company (INVARIANT 3).
        if self.writes_open_list
            && let Some(root) = self.root.clone()
        {
            let open: Vec<(String, usize)> = self
                .views
                .iter()
                .enumerate()
                .filter_map(|(i, v)| Some((v.book_id()?, i)))
                .collect();
            storage.set_string(
                &open_key(&root),
                open.iter()
                    .map(|(id, _)| id.as_str())
                    .collect::<Vec<_>>()
                    .join(","),
            );
            if let Some((id, _)) = open.iter().find(|(_, i)| *i == self.active) {
                storage.set_string(&active_key(&root), id.clone());
            }
        }
        // What was kept for the companies closed, then for those open
        // (their own, as they are now).
        for (k, v) in &self.kept {
            storage.set_string(k, v.clone());
        }
        if let Some(root) = &self.root {
            let root_hash = path_hash(root);
            for (id, name) in &self.names {
                storage.set_string(&stored(&format!("{root_hash}.{id}"), "name"), name.clone());
            }
        }
        for v in &self.views {
            if let Some(scope) = v.kept_scope() {
                v.keep(&mut |k, val| storage.set_string(&stored(&scope, k), val));
            }
        }
    }
}
