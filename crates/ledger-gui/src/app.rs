//! The window: a tab bar with one tab per open company, and the company
//! in front (company.rs).
//!
//! ```text
//! GOAL: one window, a tab for each company that is open, each tab a
//!       window on that company alone; what the app keeps of its own
//!       (the color scheme, the dock's preferences) is kept once.
//! SOURCE: plan/p15-app-and-relay.md 3.2 (tabs: `App` holds the per-app
//!         state and a `CompanyView` per tab); D-78 (one top-level tab per
//!         open company).
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
//!   4. What is kept for next time: the open companies (`open_companies`,
//!      the ids of their books, comma separated) and the one in front
//!      (`active_company`); for each company its page, its report's
//!      period, columns and comparison (`company.<id>.…`); and, as
//!      before and under the same keys, the scheme and its filter
//!      (`scheme`, `scheme_filter`, `dark`) and the dock (`dock_open`,
//!      `dock_agent`, the front tab's).
//!   5. The window's title names the company in front.
//!   6. The viewer stays read-only: a tab not in front reads what a tab
//!      in front reads, and writes nothing either.
//! ```

use crate::company::CompanyView;
use crate::data::Source;
use crate::nav::Route;
use crate::scheme_menu::SchemeMenu;
use crate::tabs::{Tab, TabAction, TabBar};
use crate::theme::{self, pal};
use egui::{Frame, RichText, Ui};
use std::collections::HashMap;
use std::path::PathBuf;

pub use crate::company::{COMPACT_BELOW, PageState, Shell};

/// Where the viewer keeps the companies that were open (the ids of their
/// books, comma separated), and the one in front.
pub const OPEN_KEY: &str = "open_companies";
pub const ACTIVE_KEY: &str = "active_company";

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
        let mut app = Self::without_books(ctx, get);
        match start {
            Start::Only(source) => {
                app.today = source.today;
                app.root = ledger_store::root::root_of_book_dir(&source.data_dir)
                    .map(std::path::Path::to_path_buf);
                app.remember_all(get);
                app.open(ctx, source);
            }
            Start::Restore { root, fallback } => {
                app.today = fallback.today;
                app.root = Some(root.clone());
                app.remember_all(get);
                let there = list_books(&root);
                let wanted: Vec<String> = get(OPEN_KEY)
                    .map(|v| v.split(',').map(str::to_string).collect())
                    .unwrap_or_default();
                let mut opened = Vec::new();
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
                    let source = Source {
                        data_dir: r.book_dir(&book),
                        today: app.today,
                    };
                    app.open(ctx, source);
                    opened.push(id);
                }
                if app.views.is_empty() {
                    app.open(ctx, fallback);
                } else {
                    // The one in front last time, else the first.
                    app.active = get(ACTIVE_KEY)
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
            dock_prefs: (None, None),
            title: TITLE.to_string(),
            welcome: None,
            first_run_source: (None, None),
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

    /// The window's title now.
    pub fn window_title(&self) -> String {
        match self.active_view() {
            Some(v) => format!("{} · {TITLE}", v.title()),
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
        if let Some(id) = view.book_id() {
            view.restore(&|k| self.kept.get(&format!("company.{id}.{k}")).cloned());
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
        self.add_view(view)
    }

    /// Close tab `i`: its view is dropped (INVARIANT 2). The tab after it,
    /// else the one before, comes to the front when it was the front one.
    pub fn close(&mut self, i: usize) {
        if i >= self.views.len() {
            return;
        }
        let view = self.views.remove(i);
        if let Some(id) = view.book_id() {
            view.keep(&mut |k, v| {
                self.kept.insert(format!("company.{id}.{k}"), v);
            });
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
        for id in list_books(&root) {
            for k in crate::company::KEPT_KEYS {
                let key = format!("company.{id}.{k}");
                if let Some(v) = get(&key) {
                    self.kept.insert(key, v);
                }
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

    /// The ids of the data root's companies that aren't open.
    fn not_open(&self) -> impl Fn() -> Vec<String> + use<> {
        let root = self.root.clone();
        let open: Vec<String> = self.views.iter().filter_map(CompanyView::book_id).collect();
        move || {
            root.as_deref()
                .map(list_books)
                .unwrap_or_default()
                .into_iter()
                .filter(|id| !open.contains(id))
                .collect()
        }
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
        let tabs: Vec<Tab> = self
            .views
            .iter()
            .map(|v| Tab {
                title: v.title(),
                detail: v.book_id().unwrap_or_else(|| v.title()),
            })
            .collect();
        let shown = self.active();
        let others = self.not_open();
        let mut action = None;
        // The bar goes where the front view says (beside its sidebar, which
        // keeps the window's whole height), or across the window when
        // there is no view.
        let mut bar = |ui: &mut Ui| {
            action = self.tabs.show(ui, &tabs, shown, &others);
        };
        let active = self.active;
        match self.views.get_mut(active) {
            Some(v) => v.ui(ui, &mut self.schemes, &mut bar),
            None => {
                bar(ui);
                self.empty(ui);
            }
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
        // The companies open, in order, and the one in front.
        let open: Vec<(String, usize)> = self
            .views
            .iter()
            .enumerate()
            .filter_map(|(i, v)| Some((v.book_id()?, i)))
            .collect();
        storage.set_string(
            OPEN_KEY,
            open.iter()
                .map(|(id, _)| id.as_str())
                .collect::<Vec<_>>()
                .join(","),
        );
        if let Some((id, _)) = open.iter().find(|(_, i)| *i == self.active) {
            storage.set_string(ACTIVE_KEY, id.clone());
        }
        // What was kept for the companies closed, then for those open
        // (their own, as they are now).
        for (k, v) in &self.kept {
            storage.set_string(k, v.clone());
        }
        for v in &self.views {
            if let Some(id) = v.book_id() {
                v.keep(&mut |k, val| storage.set_string(&format!("company.{id}.{k}"), val));
            }
        }
    }
}
