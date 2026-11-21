//! One company's window: its sidebar, top bar and current page, with
//! everything that belongs to it alone (its loader, its books, where it
//! is, its dock). The app holds one per open tab (`app.rs`).
//!
//! ```text
//! GOAL: a company's tab holds all of that company's viewer state and
//!       none of another's: tabs are independent windows on their books.
//! SOURCE: plan/p15-app-and-relay.md 3.2 (a `CompanyView` per tab: loader,
//!         books, nav, page state, receipts, export menu, owner menu,
//!         dock); D-78 (one top-level tab per open company).
//! INVARIANTS:
//!   1. Everything a company's page remembers (navigation, filters,
//!      periods, the open receipt, the dock, the export and owner menus,
//!      its loader and its owner handle) is a field of its view; no two
//!      views share a loader, a dock, a menu or a path-keyed handle.
//!   2. Every egui id a view makes is scoped by the view's own id (the app
//!      draws a view inside `push_id`), so egui's memory (scroll offsets,
//!      collapsed headers, focus, remembered widths) never carries from
//!      one company's tab into another's. A panel's id is not scoped by
//!      `push_id` (egui keys a panel's size by the id it is given), so
//!      the view's panels (sidebar, rail, top bar) are given ids made from
//!      the view's own. What is not scoped is the app's, on purpose, and
//!      says so: the assistant dock's panel (one height, one layout in
//!      every tab: dock.rs INVARIANT 9), the tab bar's panel, the colour
//!      scheme's palette (one scheme for the window), and the footer and
//!      link rects published at fixed ids for the tests, which nothing
//!      reads back (`tests/tabs.rs` says each).
//!   3. A view that is not shown still follows its journal and takes the
//!      agent's `books.show` (its own page moves, nothing else does), so
//!      showing it later is current at once.
//!   4. Dropping a view ends everything it started: its loader and its
//!      watcher and viewer presence, its receipts' workers, an export,
//!      the dock's start and its agent's process. They are all counted in
//!      one registry of the view (jobs.rs), which the drop tells to stop
//!      and which says when the last has ended (`jobs_watch`); the
//!      viewer's presence on the books is let go at the drop, not when a
//!      refresh in flight is over. One exception, by decision: a job that
//!      changes the books (an owner's restore or format upgrade, owner.rs
//!      INVARIANT 6) is never abandoned: while one runs the view cannot
//!      be closed (`closing_is_held`), nor the window (app.rs).
//!   5. What the view keeps for next time is filed under the data root
//!      and the books' id (`kept_scope`), with the books' identity (the
//!      SHA-256 of the journal's first line, which no other books share)
//!      once they load; a stored state whose identity is not the loaded
//!      books' is another set of books' and is discarded (`check_identity`).
//! ```

use crate::books::Books;
use crate::data::{Loader, Source, Update};
use crate::jobs::Jobs;
use crate::nav::{Nav, Route, Section};
use crate::owner::Owner;
use crate::pages;
use crate::receipts::Receipts;
use crate::scheme_menu::SchemeMenu;
use crate::theme::{self, pal};
use egui::{Align, CornerRadius, Frame, Key, Layout, Margin, RichText, Sense, Stroke, Ui, vec2};
use egui_phosphor::regular as icon;
use std::sync::Arc;

/// Window widths below this get the icon rail instead of the sidebar.
pub const COMPACT_BELOW: f32 = 1000.0;

/// Per-page state that survives navigation.
#[derive(Default)]
pub struct PageState {
    pub transactions: pages::transactions::State,
    pub reports: pages::reports::State,
    pub accounts: pages::accounts::State,
    pub assets: pages::assets::State,
    pub parties: pages::parties::State,
    pub projects: pages::projects::State,
    pub documents: pages::documents::State,
    pub invoices: pages::invoices::State,
    pub journal: pages::journal::State,
}

/// What a page can ask of the shell.
pub struct Shell<'a> {
    pub receipts: &'a mut Receipts,
    pub state: &'a mut PageState,
    pub go: Option<Route>,
    /// Seconds since the last live update arrived (for "new" highlights).
    pub since_update: f64,
}

impl Shell<'_> {
    pub fn go(&mut self, r: Route) {
        self.go = Some(r);
    }
}

/// Everything that belongs to one open company: the tab's whole state.
pub struct CompanyView {
    /// What tells this company's views apart: where its books are, as a
    /// real path where it can be told. Two views never have the same.
    key: String,
    /// The scope of every egui id this view makes (INVARIANT 2).
    id: egui::Id,
    /// The books' folder, if they are read from one.
    data_dir: Option<std::path::PathBuf>,
    loader: Option<Loader>,
    books: Option<Arc<Books>>,
    error: Option<String>,
    nav: Nav,
    receipts: Receipts,
    state: PageState,
    palette: crate::palette::Palette,
    updated_at: f64,
    live_label: bool,
    /// The export menu (menu.rs).
    menu: crate::menu::Menu,
    /// The owner's Books menu (owner.rs).
    owner_menu: crate::owner::OwnerMenu,
    /// The agent dock along the bottom (dock.rs).
    dock: crate::dock::Dock,
    /// What this view asks to do the owner's actions (INVARIANT 1).
    owner: Option<Arc<dyn Owner>>,
    /// Books arrived while the view wasn't shown: their receipts are
    /// prefetched when it is.
    prefetch_due: bool,
    /// Every thread this view started (INVARIANT 4): the loader, the
    /// receipts' workers, an export, an owner's job, the dock's start and
    /// its agent's reaper. Told to stop when the view drops (`Drop`).
    jobs: Jobs,
    /// What identifies these books, taken from the books themselves once
    /// they load, as it was kept (until then) or is now (INVARIANT 5).
    identity: Option<String>,
    /// The identity of the books as last loaded (None while there is no
    /// journal line to take it from).
    loaded_identity: Option<String>,
    /// What the window knows of this company's assistants, given by the app
    /// each frame (connections.rs): the state, who is connected, the keys.
    assist: crate::connections::Status,
    /// The Connect page's own state (connect.rs).
    connect: crate::connect::State,
}

impl Drop for CompanyView {
    /// The tab is closing: every thread it started is told to stop before
    /// anything else of the view goes (an export that hasn't saved yet
    /// saves nothing; a thumbnail not written is not written).
    fn drop(&mut self) {
        self.jobs.stop();
    }
}

/// The key that tells a company's views apart: its books' folder, as the
/// system really has it.
pub(crate) fn key_of(data_dir: &std::path::Path) -> String {
    ledger_store::root::canonical(data_dir)
        .unwrap_or_else(|_| data_dir.to_path_buf())
        .display()
        .to_string()
}

impl CompanyView {
    fn bare(key: String, data_dir: Option<std::path::PathBuf>, ctx: &egui::Context) -> Self {
        let id = egui::Id::new(("company", &key));
        let jobs = Jobs::new();
        let spawner = jobs.spawner();
        Self {
            key,
            id,
            data_dir,
            loader: None,
            books: None,
            error: None,
            nav: Nav::new(Route::Overview),
            receipts: Receipts::new(ctx, &spawner),
            state: PageState::default(),
            palette: crate::palette::Palette::scoped(id),
            updated_at: -1e9,
            live_label: false,
            menu: crate::menu::Menu::in_tab(spawner.clone()),
            owner_menu: crate::owner::OwnerMenu::scoped(id, spawner),
            dock: Default::default(),
            owner: crate::owner::current(),
            prefetch_due: false,
            jobs,
            identity: None,
            loaded_identity: None,
            assist: crate::connections::Status::off(""),
            connect: Default::default(),
        }
    }

    /// The live view of the books in `source`: its own loader follows
    /// their journal for as long as the view lives.
    pub fn open(ctx: &egui::Context, source: Source) -> Self {
        let mut view = Self::bare(key_of(&source.data_dir), Some(source.data_dir.clone()), ctx);
        view.receipts.thumb_dir = crate::receipts::thumb_dir();
        view.loader = Some(Loader::spawn_in(source, ctx.clone(), &view.jobs.spawner()));
        view
    }

    /// A view of fixed books (tests and screenshots).
    pub fn with_books(ctx: &egui::Context, books: Books, start: Route) -> Self {
        let mut view = Self::bare(key_of(&books.data_dir), Some(books.data_dir.clone()), ctx);
        view.receipts.prefetch(&books);
        view.loaded_identity = identity_of(&books);
        view.books = Some(Arc::new(books));
        view.nav = Nav::new(start);
        view
    }

    /// This view asking `owner` for the owner's actions, whatever the
    /// program gave the viewer (tests give each view its own).
    pub fn with_owner(mut self, owner: Arc<dyn Owner>) -> Self {
        self.owner = Some(owner);
        self
    }

    /// [`Self::with_owner`], for a view already in the tabs (tests).
    pub fn set_owner(&mut self, owner: Arc<dyn Owner>) {
        self.owner = Some(owner);
    }

    /// The dock as it was last left (the app's preferences).
    pub fn with_dock(mut self, dock: crate::dock::Dock) -> Self {
        self.dock = dock;
        self
    }

    /// What the window knows of this company's assistants, as of this
    /// frame (the app gives it before it draws the view).
    pub fn set_assist(&mut self, status: crate::connections::Status) {
        self.assist = status;
    }

    /// Open the Connect page (the first run does, once: app.rs).
    pub fn open_connect(&mut self) {
        self.nav.go(Route::Connect);
    }

    /// What this view was last given of its assistants (tests).
    pub fn assist(&self) -> &crate::connections::Status {
        &self.assist
    }

    /// The Connect page's state (tests).
    pub fn connect_state(&self) -> &crate::connect::State {
        &self.connect
    }

    /// A revoke on the Connect page just ended: who is connected must be
    /// read again (taken once).
    pub fn take_roster_refresh(&mut self) -> bool {
        self.connect.take_refresh()
    }

    /// What tells this company's view apart from another's.
    pub fn key(&self) -> &str {
        &self.key
    }

    /// The scope of this view's egui ids.
    pub fn id(&self) -> egui::Id {
        self.id
    }

    /// The books' id in the data root (their folder's name), if they are
    /// in one.
    pub fn book_id(&self) -> Option<String> {
        let dir = self.data_dir.as_deref()?;
        ledger_store::root::inside_a_root(dir)
            .then(|| dir.file_name()?.to_str().map(str::to_string))
            .flatten()
    }

    /// The name the books carry, once they are loaded (and it isn't
    /// blank).
    pub fn name(&self) -> Option<String> {
        self.books
            .as_ref()
            .map(|b| b.company_name().trim().to_string())
            .filter(|n| !n.is_empty())
    }

    /// The tab's title: the name the books carry, else the id of the
    /// books, else the folder's name.
    pub fn title(&self) -> String {
        self.name()
            .or_else(|| self.book_id())
            .or_else(|| self.folder_name())
            .unwrap_or_else(|| "Company".to_string())
    }

    /// The books' folder's name: a demo's or a copy's folder is
    /// `books-data`, which its parent names.
    fn folder_name(&self) -> Option<String> {
        let dir = self.data_dir.as_deref()?;
        let name = dir.file_name()?.to_str()?;
        let parent = dir.parent().and_then(|p| p.file_name()?.to_str());
        Some(match (name, parent) {
            ("books-data", Some(p)) => p.to_string(),
            _ => name.to_string(),
        })
    }

    /// What this company's badge hue is chosen from: the id of its books
    /// (theme.rs `hue_of`), else the name of their folder, which is that
    /// id for books in a data root: a function of where the books are
    /// kept and never of the name they carry, so renaming a company
    /// leaves its colour.
    pub fn hue_key(&self) -> String {
        self.book_id()
            .or_else(|| self.folder_name())
            .unwrap_or_default()
    }

    /// The status card's heading: this company's badge and name.
    fn card_head(&self, ctx: &egui::Context) -> CardHead {
        CardHead {
            initial: self.initial(),
            title: self.title(),
            hue: theme::badge_hue(ctx, &self.hue_key()),
        }
    }

    /// The letter on this company's badge: its title's first, upper case.
    fn initial(&self) -> char {
        self.title()
            .chars()
            .next()
            .map(|c| c.to_uppercase().next().unwrap_or(c))
            .unwrap_or('W')
    }

    /// The books as last loaded (tests).
    pub fn books(&self) -> Option<&Arc<Books>> {
        self.books.as_ref()
    }

    /// Why the books couldn't be read, if they couldn't (tests).
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Whether this view follows a journal on a loader of its own.
    pub fn follows_journal(&self) -> bool {
        self.loader.is_some()
    }

    /// Whether its loader's thread has ended (tests; false while it runs
    /// and for a view with no loader).
    pub fn loader_finished(&self) -> bool {
        self.loader.as_ref().is_some_and(Loader::finished)
    }

    /// A handle that says when this view's loader's thread has ended,
    /// which outlives the view (tests).
    pub fn loader_watch(&self) -> Option<crate::data::LoaderWatch> {
        self.loader.as_ref().map(Loader::watch)
    }

    /// A handle that says when every thread this view started has ended,
    /// which outlives the view: its loader, its receipts' workers, its
    /// exports and owner's jobs, the dock's start and its agent's reaper
    /// (INVARIANT 4; tests).
    pub fn jobs_watch(&self) -> crate::jobs::Watch {
        self.jobs.watch()
    }

    /// What this view's threads hold its tab open for, if a job that
    /// changes the books runs (INVARIANT 4): the words the tab's close
    /// control shows.
    pub fn closing_is_held(&self) -> Option<String> {
        self.jobs.holding()
    }

    /// Tell this view's threads to stop, as its close does; the signal a
    /// thread of it was given (tests).
    pub fn stop_signal(&self) -> crate::jobs::Stop {
        self.jobs.spawner().stop_signal()
    }

    /// Start `work` as this view's export job would be started (tests: a
    /// job whose steps the test controls).
    pub fn start_export_for_tests(
        &mut self,
        ctx: &egui::Context,
        what: &str,
        work: Box<dyn FnOnce(&crate::jobs::Stop) -> crate::menu::Done + Send>,
    ) {
        self.menu.start_with(ctx, what.to_string(), work);
    }

    /// The agent dock (tests).
    pub fn dock_mut(&mut self) -> &mut crate::dock::Dock {
        &mut self.dock
    }

    /// The agent dock (read).
    pub fn dock_ref(&self) -> &crate::dock::Dock {
        &self.dock
    }

    /// Navigate (tests).
    pub fn goto(&mut self, r: Route) {
        self.nav.go(r);
    }

    /// Where the view is.
    pub fn route(&self) -> &Route {
        &self.nav.current
    }

    /// Label a fixed-books view "Live", as it reads when following a
    /// journal (screenshots of what a running app looks like).
    pub fn show_live_label(&mut self) {
        self.live_label = true;
    }

    /// Page state (tests).
    pub fn state_mut(&mut self) -> &mut PageState {
        &mut self.state
    }

    /// Page state (read).
    pub fn state(&self) -> &PageState {
        &self.state
    }

    /// Keep thumbnails on disk in `dir` (the live viewer keeps them in
    /// the user's cache directory, `wanderbooks/thumbs`); tests point it
    /// at their own.
    pub fn keep_thumbs_in(&mut self, dir: std::path::PathBuf) {
        self.receipts.thumb_dir = Some(dir);
        self.receipts.forget_thumbs();
        if let Some(b) = self.books.clone() {
            self.receipts.prefetch(&b);
        }
    }

    /// The size in pixels of a file's thumbnail, once drawn (tests).
    pub fn thumb_size(&self, sha: &str) -> Option<[usize; 2]> {
        self.receipts.thumb_size(sha)
    }

    /// Block until every receipt on screen has rendered (screenshots).
    pub fn settle(&mut self, ctx: &egui::Context) {
        self.receipts.settle(ctx);
    }

    /// Take what the loader has sent since the last frame (INVARIANT 3:
    /// whether or not the view is shown). `shown`: its receipts are
    /// worked on only then.
    pub fn pump(&mut self, ctx: &egui::Context, shown: bool) {
        let now = ctx.input(|i| i.time);
        let updates = self.loader.as_ref().map(|l| l.poll()).unwrap_or_default();
        for u in updates {
            match u {
                Update::Books(b) => {
                    if self.books.is_some() {
                        self.updated_at = now;
                    }
                    if shown {
                        self.receipts.prefetch(&b);
                    } else {
                        self.prefetch_due = true;
                    }
                    self.loaded_identity = identity_of(&b);
                    self.books = Some(b);
                    self.error = None;
                }
                Update::Error(e) => self.error = Some(e),
                Update::Show(target) => self.show(target),
            }
        }
        if shown {
            if std::mem::take(&mut self.prefetch_due)
                && let Some(b) = self.books.clone()
            {
                self.receipts.prefetch(&b);
            }
            self.receipts.pump(ctx);
        }
    }

    /// The dock along the bottom, for books of a data root with an owner
    /// to start the agent (dock.rs).
    fn draw_dock(&mut self, ui: &mut Ui) {
        if !crate::dock::offered() {
            return;
        }
        let (Some(owner), Some(books)) = (self.owner.clone(), self.books.clone()) else {
            return;
        };
        if let Some(book) = crate::owner::book_id(&books) {
            self.dock.show(ui, &owner, &book, &self.jobs.spawner());
        }
    }

    /// The window: the sidebar, the top bar, the dock and the page, with
    /// every id in them scoped to this view (INVARIANT 2).
    ///
    /// The app's tab bar is already across the top of `ui` (`tabs.rs`):
    /// this window is what is below it, and the sidebar's height is the
    /// window's less the bar's (`tests/sidenav.rs` adds the bar's height
    /// to the windows it sizes).
    pub fn ui(&mut self, ui: &mut Ui, schemes: &mut SchemeMenu) {
        let id = self.id;
        ui.push_id(id, |ui| self.window(ui, schemes));
    }

    fn window(&mut self, ui: &mut Ui, schemes: &mut SchemeMenu) {
        self.handle_keys(ui);
        let compact = ui.available_width() < COMPACT_BELOW;
        self.sidebar(ui, schemes, compact);
        self.topbar(ui);
        self.draw_dock(ui);
        let p = pal(ui.ctx());
        let since_update = ui.input(|i| i.time) - self.updated_at;
        if since_update < 6.0 {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(250));
        }
        egui::CentralPanel::no_frame()
            .frame(Frame::new().fill(p.bg))
            .show(ui, |ui| {
                if let Some(err) = &self.error {
                    Frame::new()
                        .fill(p.negative_soft)
                        .inner_margin(Margin::symmetric(20, 8))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.label(
                                RichText::new(format!("{} {err}", icon::WARNING)).color(p.negative),
                            );
                        });
                }
                let Some(books) = self.books.clone() else {
                    ui.centered_and_justified(|ui| {
                        ui.label(RichText::new("Opening the books…").color(p.muted));
                    });
                    return;
                };
                // Books waiting for a format upgrade: said above every page
                // until it is done (owner.rs INVARIANT 5).
                self.owner_menu.notice(ui, self.owner.clone(), &books);
                let route = self.nav.current.clone();
                // What "Ask your assistant" on a page is about.
                crate::connect::set_context(
                    ui.ctx(),
                    crate::connect::AskContext {
                        company: self.assist.company.clone(),
                        serves: self.assist.serves,
                        link: self.assist.link,
                    },
                );
                if route == Route::Connect {
                    // The Connect page keeps its own state (connect.rs).
                    match self.book_id() {
                        Some(book) => {
                            let spawner = self.jobs.spawner();
                            self.connect.show(
                                ui,
                                &self.assist,
                                &book,
                                self.owner.clone(),
                                &spawner,
                            );
                        }
                        None => self.nav.go(Route::Company),
                    }
                } else {
                    let mut shell = Shell {
                        receipts: &mut self.receipts,
                        state: &mut self.state,
                        go: None,
                        since_update,
                    };
                    pages::show(ui, &books, &route, &mut shell);
                    if let Some(to) = shell.go.take() {
                        self.nav.go(to);
                    }
                }
                if crate::connect::take_go(ui.ctx()) {
                    self.nav.go(Route::Connect);
                }
                // A web assistant asking to connect is asked about over
                // whatever page this is (the Connect page has its own card).
                if route != Route::Connect {
                    let spawner = self.jobs.spawner();
                    self.connect
                        .web_prompt(ui, &self.assist, &self.owner.clone(), &spawner);
                }
                if let Some(to) = self.palette.show(ui.ctx(), &books) {
                    self.nav.go(to);
                }
            });
    }

    /// Go where the agent asked (`books.show`).
    ///
    /// ```text
    /// GOAL: the viewer shows what the agent was told it shows.
    /// SOURCE: BS-period-3 (one default period; a start after the end is
    ///         refused, never swapped); plan/served.md P3 defect 15.
    /// INVARIANTS:
    ///   1. A target's dates are taken as the server resolved them, an
    ///      omitted one (an earlier writer's) by the same rule
    ///      (Books::periods), never swapped, clamped or guessed.
    ///   2. A target the viewer can't honour (an unknown page, report,
    ///      split or comparison; text that isn't a date; a start after
    ///      the end; an account, asset or party not in the books) is
    ///      ignored whole: the page, range and split stay as they were.
    /// ```
    pub fn show(&mut self, target: ledger_store::ops::ViewerTarget) {
        use crate::nav::ReportKind;
        use ledger_store::ops::ViewerTarget as T;
        let Some(b) = self.books.clone() else {
            return;
        };
        // Some(None): omitted; None: text that isn't a date.
        let date = |s: &Option<String>| match s.as_deref() {
            None => Some(None),
            Some(d) => d.parse::<chrono::NaiveDate>().ok().map(Some),
        };
        let dates = |start: &Option<String>, end: &Option<String>| {
            b.periods().statement(date(start)?, date(end)?).ok()
        };
        let route = match target {
            T::Page { page } => match page.as_str() {
                "overview" => Route::Overview,
                "cash" => Route::Cash,
                "profit" => Route::Profit,
                "transactions" => Route::Transactions,
                "reports" => Route::Reports(ReportKind::ProfitAndLoss),
                "accounts" => Route::Accounts,
                "assets" => Route::Assets,
                "parties" => Route::Parties,
                "projects" => Route::Projects,
                "documents" => Route::Documents,
                "company" => Route::Company,
                "journal" => Route::Journal,
                _ => return,
            },
            T::Entry { entry_id } => Route::Entry(entry_id),
            T::Account { code, start, end } => {
                use crate::pages::accounts::RegisterRange;
                let Some(a) = b.ledger.chart().find_by_code(&code) else {
                    return;
                };
                let Some((s, e)) = dates(&start, &end) else {
                    return;
                };
                self.state.accounts.range = if start.is_none() && end.is_none() {
                    RegisterRange::Default
                } else {
                    RegisterRange::Dates(s, e)
                };
                Route::Account(a.id)
            }
            T::Report {
                report,
                start,
                end,
                columns,
                compare,
            } => {
                use crate::pages::reports::{Columns, Compare, Range};
                let columns = match columns.as_deref() {
                    None | Some("total") => Columns::Total,
                    Some("month") => Columns::Month,
                    Some("quarter") => Columns::Quarter,
                    Some("year") => Columns::Year,
                    Some(_) => return,
                };
                let compare = match compare.as_deref() {
                    None => Compare::None,
                    Some("prior_period") => Compare::PriorPeriod,
                    Some("prior_year") => Compare::PriorYear,
                    Some(_) => return,
                };
                let Some((s, e)) = dates(&start, &end) else {
                    return;
                };
                let kind = match report.as_str() {
                    "profit_and_loss" => ReportKind::ProfitAndLoss,
                    "balance_sheet" => ReportKind::BalanceSheet,
                    "cash_flow" => ReportKind::CashFlow,
                    "members_equity" => ReportKind::MembersEquity,
                    "trial_balance" => ReportKind::TrialBalance,
                    "general_ledger" => ReportKind::GeneralLedger,
                    "tax_summary" => ReportKind::TaxSummary,
                    "vendor_1099" => ReportKind::Vendor1099,
                    "ar_aging" => ReportKind::ArAging,
                    "ap_aging" => ReportKind::ApAging,
                    "fixed_asset_register" => ReportKind::FixedAssetRegister,
                    _ => return,
                };
                self.state.reports.columns = columns;
                self.state.reports.compare = compare;
                // The dates as sent, by a preset's name when they are one
                // (the default first: "Year to date"), else Custom.
                self.state.reports.range = std::iter::once(Range::default())
                    .chain(Range::PRESETS)
                    .find(|r| r.dates(&b) == (s, e))
                    .unwrap_or(Range::Custom(s, e));
                Route::Reports(kind)
            }
            T::Document { attachment_id } => Route::Document(attachment_id),
            T::Asset { asset_id } => {
                if b.ledger.get_asset(asset_id).is_none() {
                    return;
                }
                Route::Asset(asset_id)
            }
            T::Party { party_id } => {
                if b.ledger.get_party(party_id).is_none() {
                    return;
                }
                Route::Party(party_id)
            }
            T::Project { class_id } => {
                if b.ledger.get_class(class_id).is_none() {
                    return;
                }
                Route::Project(class_id)
            }
            T::Reconciliation { id } => {
                // Its own page: statement, balance, the lines it cleared.
                if b.reconciliation(id).is_none() {
                    return;
                }
                Route::Reconciliation(id)
            }
            T::Search { query } => {
                let st = &mut self.state.transactions;
                st.query = query;
                st.filter = Default::default();
                st.account = None;
                Route::Transactions
            }
        };
        self.nav.go(route);
    }

    fn handle_keys(&mut self, ui: &Ui) {
        // The agent in the dock has the keyboard while it is clicked on.
        if self.dock.has_keyboard() {
            return;
        }
        let (back, fwd, palette) = ui.input(|i| {
            (
                (i.modifiers.alt && i.key_pressed(Key::ArrowLeft))
                    || i.pointer.button_pressed(egui::PointerButton::Extra1),
                (i.modifiers.alt && i.key_pressed(Key::ArrowRight))
                    || i.pointer.button_pressed(egui::PointerButton::Extra2),
                i.modifiers.command && i.key_pressed(Key::K),
            )
        });
        if back {
            self.nav.back();
        }
        if fwd {
            self.nav.forward();
        }
        if palette {
            self.palette.open();
        }
    }

    /// The sidebar: full below [`COMPACT_BELOW`] window widths, an icon
    /// rail (names in tooltips) when the window is narrower.
    fn sidebar(&mut self, ui: &mut Ui, schemes: &mut SchemeMenu, compact: bool) {
        if compact {
            return self.rail(ui, schemes);
        }
        let p = pal(ui.ctx());
        let books = self.books.clone();
        let head = self.card_head(ui.ctx());
        egui::Panel::left(ui.id().with("sidebar"))
            .exact_size(224.0)
            .resizable(false)
            .frame(
                Frame::new()
                    .fill(p.sidebar)
                    .inner_margin(Margin::symmetric(12, 16)),
            )
            .show(ui, |ui| {
                top_slot(ui);
                let current = self.nav.current.section();
                let attention = books.as_ref().map(|b| b.attention.len()).unwrap_or(0);
                // Short windows give way in steps (Fit): the status card
                // folds, then the rows tighten, then the links scroll.
                // The footer's heights are the ones it was drawn at.
                let fold = 3.0
                    * (ui.ctx().fonts_mut(|f| f.row_height(&theme::regular(11.5)))
                        + ui.spacing().item_spacing.y);
                let memo = FooterMemo::load(ui, "sidebar");
                let heights = memo.heights(SIDEBAR_FOOTER, fold, SIDEBAR_TIGHTEN);
                let fit = fit(ui.available_height(), heights, &SIDEBAR_ROWS, slack(ui));
                ui.add_space(if fit.tight {
                    HEAD_GAP_TIGHT
                } else {
                    SIDEBAR_ROWS.head_gap
                });
                let footer_h = heights[fit.form()];
                // At least one link always shows; below that the footer
                // gives way (footer_layout).
                let links_h = (ui.available_height() - footer_h - FOOTER_GAP).max(fit.row);
                let nav = &mut self.nav;
                links(ui, "sidebar", fit, links_h, current, |ui, s| {
                    let on = s == current;
                    let (rect, resp) =
                        ui.allocate_exact_size(vec2(ui.available_width(), fit.row), Sense::click());
                    // Named for assistive tech (and the tests).
                    resp.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, s.title())
                    });
                    if on {
                        ui.painter()
                            .rect_filled(rect, CornerRadius::same(8), p.surface);
                        ui.painter().rect_stroke(
                            rect,
                            CornerRadius::same(8),
                            Stroke::new(1.0, p.border),
                            egui::StrokeKind::Inside,
                        );
                    } else if resp.hovered() {
                        ui.painter()
                            .rect_filled(rect, CornerRadius::same(8), p.hover);
                    }
                    let color = if on { p.text } else { p.muted };
                    ui.painter().text(
                        rect.left_center() + vec2(10.0, 0.0),
                        egui::Align2::LEFT_CENTER,
                        s.icon(),
                        egui::FontId::proportional(16.0),
                        if on { p.accent } else { p.muted },
                    );
                    ui.painter().text(
                        rect.left_center() + vec2(36.0, 0.0),
                        egui::Align2::LEFT_CENTER,
                        s.title(),
                        if on {
                            theme::semibold(13.5)
                        } else {
                            theme::regular(13.5)
                        },
                        color,
                    );
                    if s == Section::Overview && attention > 0 {
                        let badge = egui::Rect::from_center_size(
                            rect.right_center() - vec2(18.0, 0.0),
                            vec2(22.0, 18.0),
                        );
                        ui.painter()
                            .rect_filled(badge, CornerRadius::same(9), p.warning_soft);
                        ui.painter().text(
                            badge.center(),
                            egui::Align2::CENTER_CENTER,
                            attention.to_string(),
                            theme::semibold(11.0),
                            p.warning,
                        );
                    }
                    if resp.clicked() {
                        nav.go(s.home());
                    }
                    resp
                });
                let footer = footer_layout(ui, footer_h, Align::Min, |ui| {
                    if fit.tight {
                        tighten(ui);
                    }
                    schemes.menu(ui, false);
                    if let Some(b) = &books {
                        ui.add_space(2.0);
                        self.menu.ui(ui, b, false);
                        self.owner_menu.ui(ui, self.owner.clone(), b, false);
                    }
                    ui.add_space(6.0);
                    if let Some(b) = &books {
                        live_status(
                            ui,
                            b,
                            ui.input(|i| i.time) - self.updated_at,
                            self.loader.is_some() || self.live_label,
                            fit.folded,
                            &head,
                        );
                    }
                });
                memo.measured(ui, "sidebar", fit.form(), footer.response.rect);
            });
    }

    /// The sidebar as a narrow rail of icons, for narrow windows.
    fn rail(&mut self, ui: &mut Ui, schemes: &mut SchemeMenu) {
        let p = pal(ui.ctx());
        let books = self.books.clone();
        let head = self.card_head(ui.ctx());
        egui::Panel::left(ui.id().with("rail"))
            .exact_size(RAIL_W)
            .resizable(false)
            .frame(
                Frame::new()
                    .fill(p.sidebar)
                    .inner_margin(Margin::symmetric(10, 16)),
            )
            .show(ui, |ui| {
                top_slot(ui);
                let current = self.nav.current.section();
                let attention = books.as_ref().map(|b| b.attention.len()).unwrap_or(0);
                // As the sidebar's, with no card to fold.
                let memo = FooterMemo::load(ui, "rail");
                let heights = memo.heights(RAIL_FOOTER, 0.0, RAIL_TIGHTEN);
                let fit = fit(ui.available_height(), heights, &RAIL_ROWS, slack(ui));
                ui.add_space(if fit.tight {
                    HEAD_GAP_TIGHT
                } else {
                    RAIL_ROWS.head_gap
                });
                let footer_h = heights[fit.form()];
                let links_h = (ui.available_height() - footer_h - FOOTER_GAP).max(fit.row);
                let nav = &mut self.nav;
                links(ui, "rail", fit, links_h, current, |ui, s| {
                    let on = s == current;
                    let (rect, resp) =
                        ui.allocate_exact_size(vec2(ui.available_width(), fit.row), Sense::click());
                    resp.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, s.title())
                    });
                    if on {
                        ui.painter()
                            .rect_filled(rect, CornerRadius::same(8), p.surface);
                        ui.painter().rect_stroke(
                            rect,
                            CornerRadius::same(8),
                            Stroke::new(1.0, p.border),
                            egui::StrokeKind::Inside,
                        );
                    } else if resp.hovered() {
                        ui.painter()
                            .rect_filled(rect, CornerRadius::same(8), p.hover);
                    }
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        s.icon(),
                        egui::FontId::proportional(18.0),
                        if on { p.accent } else { p.muted },
                    );
                    let tip = if s == Section::Overview && attention > 0 {
                        ui.painter()
                            .circle_filled(rect.center() + vec2(9.0, -8.0), 4.0, p.warning);
                        format!("{} · {attention} need attention", s.title())
                    } else {
                        s.title().to_string()
                    };
                    let resp = resp.on_hover_text(tip);
                    if resp.clicked() {
                        nav.go(s.home());
                    }
                    resp
                });
                let footer = footer_layout(ui, footer_h, Align::Center, |ui| {
                    if fit.tight {
                        tighten(ui);
                    }
                    schemes.menu(ui, true);
                    if let Some(b) = &books {
                        self.menu.ui(ui, b, true);
                        self.owner_menu.ui(ui, self.owner.clone(), b, true);
                    }
                    ui.add_space(6.0);
                    if let Some(b) = &books {
                        let live = self.loader.is_some() || self.live_label;
                        let checks = checks(b);
                        let ok = checks.iter().all(|c| c.0);
                        // The company's badge, its live dot on the badge's
                        // corner (a warning there when a check failed); its
                        // name, state, event and checks on hover.
                        let (r, resp) =
                            ui.allocate_exact_size(vec2(RAIL_BADGE, RAIL_BADGE), Sense::hover());
                        head.paint(ui, r, 6);
                        let corner = r.right_bottom() - vec2(1.0, 1.0);
                        ui.painter().circle_filled(corner, 6.0, p.sidebar);
                        if ok {
                            let pulse = (ui.input(|i| i.time) - self.updated_at < 2.0) as u8 as f32;
                            ui.painter().circle_filled(
                                corner,
                                4.0 + pulse,
                                if live { p.positive } else { p.faint },
                            );
                        } else {
                            ui.painter().text(
                                corner,
                                egui::Align2::CENTER_CENTER,
                                icon::WARNING_CIRCLE,
                                egui::FontId::proportional(13.0),
                                p.negative,
                            );
                        }
                        let mut tip = format!(
                            "{}\n{} · event {}",
                            head.title,
                            if live { "Live" } else { "Snapshot" },
                            b.seq()
                        );
                        for (ok, text, _) in &checks {
                            tip.push_str(&format!(
                                "\n{} {text}",
                                if *ok {
                                    icon::SEAL_CHECK
                                } else {
                                    icon::WARNING_CIRCLE
                                }
                            ));
                        }
                        resp.on_hover_text(tip);
                    }
                });
                memo.measured(ui, "rail", fit.form(), footer.response.rect);
            });
    }

    /// Back, forward, where you are, today, and search. Squeezed, the
    /// search shrinks to its icon, the date goes, and the crumb truncates:
    /// nothing ever overlaps.
    fn topbar(&mut self, ui: &mut Ui) {
        let p = pal(ui.ctx());
        egui::Panel::top(ui.id().with("topbar"))
            .exact_size(52.0)
            .frame(
                Frame::new()
                    .fill(p.bg)
                    .inner_margin(Margin::symmetric(16, 10))
                    .stroke(Stroke::new(1.0, p.border)),
            )
            .show(ui, |ui| {
                let width = ui.available_width();
                ui.horizontal_centered(|ui| {
                    let nav_btn = |ui: &mut Ui, glyph: &str, enabled: bool, tip: &str| {
                        ui.add_enabled(
                            enabled,
                            egui::Button::new(RichText::new(glyph).size(16.0))
                                .frame(false)
                                .min_size(vec2(28.0, 28.0)),
                        )
                        .on_hover_text(tip)
                        .clicked()
                    };
                    if nav_btn(ui, icon::ARROW_LEFT, self.nav.can_back(), "Back (Alt+←)") {
                        self.nav.back();
                    }
                    if nav_btn(
                        ui,
                        icon::ARROW_RIGHT,
                        self.nav.can_forward(),
                        "Forward (Alt+→)",
                    ) {
                        self.nav.forward();
                    }
                    ui.add_space(6.0);
                    // Right side first, so the crumb gets only what's left.
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let (text, min_w) = if width >= 640.0 {
                            (
                                format!(
                                    "{}  Search everything…      Ctrl K",
                                    icon::MAGNIFYING_GLASS
                                ),
                                260.0,
                            )
                        } else if width >= 420.0 {
                            (format!("{}  Search", icon::MAGNIFYING_GLASS), 0.0)
                        } else {
                            (icon::MAGNIFYING_GLASS.to_string(), 30.0)
                        };
                        let search = egui::Button::new(
                            RichText::new(text)
                                .font(theme::regular(13.0))
                                .color(p.faint),
                        )
                        .fill(p.surface)
                        .stroke(Stroke::new(1.0, p.border))
                        .corner_radius(CornerRadius::same(8))
                        .min_size(vec2(min_w, 30.0));
                        if ui
                            .add(search)
                            .on_hover_text("Search everything (Ctrl K)")
                            .clicked()
                        {
                            self.palette.open();
                        }
                        if let Some(b) = &self.books
                            && width >= 860.0
                        {
                            ui.label(
                                RichText::new(format!("Today {}", crate::fmt::date(b.today)))
                                    .font(theme::regular(12.5))
                                    .color(p.muted),
                            );
                        }
                        // The assistants: its shape and words, then what it
                        // last did while there is room (connect.rs).
                        let room = if width >= 1100.0 {
                            Some(crate::connect::Room::Full)
                        } else if width >= 780.0 {
                            Some(crate::connect::Room::Word)
                        } else if width >= 520.0 {
                            Some(crate::connect::Room::Icon)
                        } else {
                            None
                        };
                        if let Some(room) = room
                            && crate::connect::chip(ui, &self.assist, room)
                        {
                            self.nav.go(Route::Connect);
                        }
                        ui.add_space(8.0);
                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                            let crumbs =
                                crate::palette::crumbs(self.books.as_deref(), &self.nav.current);
                            // Narrow, the last crumb alone says where you are.
                            let skip = if ui.available_width() >= 260.0 {
                                0
                            } else {
                                crumbs.len() - 1
                            };
                            let last = crumbs.len() - 1;
                            let mut go = None;
                            for (i, (text, to)) in crumbs.iter().enumerate().skip(skip) {
                                if i > skip && ui.available_width() >= 260.0 - 80.0 {
                                    ui.label(RichText::new(icon::CARET_RIGHT).color(p.faint));
                                }
                                let rich = if i == last && i > 0 {
                                    RichText::new(text).font(theme::semibold(13.5))
                                } else {
                                    RichText::new(text)
                                        .font(theme::regular(13.5))
                                        .color(p.muted)
                                };
                                let label = egui::Label::new(rich).truncate();
                                match to {
                                    Some(to) => {
                                        let r = ui.add(label.sense(egui::Sense::click()));
                                        if r.hovered() {
                                            ui.ctx()
                                                .set_cursor_icon(egui::CursorIcon::PointingHand);
                                        }
                                        if r.on_hover_text(format!("Go to {text}")).clicked() {
                                            go = Some(to.clone());
                                        }
                                    }
                                    None => {
                                        ui.add(label);
                                    }
                                }
                            }
                            if let Some(to) = go {
                                self.nav.go(to);
                            }
                        });
                    });
                });
            });
    }
}

/// The keys a view keeps its things under (after `company.<scope>.`; the
/// company's name is kept by the app beside them, under `name`).
pub(crate) const KEPT_KEYS: [&str; 5] = ["page", "range", "columns", "compare", "identity"];

/// A short, stable name for a place on disk: FNV-1a (64 bits) of its
/// canonical path's bytes, in twelve hex digits. Stable across runs and
/// versions (it is a key in a file); not a secret.
pub fn path_hash(path: &std::path::Path) -> String {
    let real = ledger_store::root::canonical(path).unwrap_or_else(|_| path.to_path_buf());
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in real.as_os_str().as_encoded_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{:012x}", h & 0xffff_ffff_ffff)
}

/// What a company's kept things are filed under: for books in a data
/// root, the root (a hash of its canonical path) and the books' id,
/// `<root hash>.<id>`; for books in a folder outside any root, that
/// folder (`dir.<hash of its canonical path>`). Two roots' companies of
/// one id are two companies.
pub fn kept_scope_of(data_dir: &std::path::Path) -> String {
    match (
        ledger_store::root::root_of_book_dir(data_dir),
        data_dir.file_name().and_then(|n| n.to_str()),
    ) {
        (Some(root), Some(id)) => format!("{}.{id}", path_hash(root)),
        _ => format!("dir.{}", path_hash(data_dir)),
    }
}

/// What identifies a set of books, from the books themselves: the SHA-256
/// of the journal's first line (its canonical bytes: the very value the
/// second line's `prev` repeats). Every set of books has its own first
/// line (its timestamp, its first event); books made again under an old
/// id, or restored over it from another set, do not share it. None until
/// the journal has a line.
pub fn identity_of(books: &Books) -> Option<String> {
    books
        .journal
        .first()
        .map(|l| ledger_store::journal::sha256_hex(l.to_canonical().as_bytes()))
}

impl CompanyView {
    /// What this company's kept things are filed under ([`kept_scope_of`]),
    /// when its books are read from a folder.
    pub fn kept_scope(&self) -> Option<String> {
        self.data_dir.as_deref().map(kept_scope_of)
    }

    /// The books' identity as kept, or as last loaded ([`identity_of`]).
    pub fn identity(&self) -> Option<&str> {
        self.identity.as_deref()
    }

    /// Compare the identity kept with the loaded books' (INVARIANT 5).
    /// First load with nothing kept: the books' identity is taken and
    /// nothing changes. Kept, and the same: nothing changes. Kept, and
    /// not the same: these are other books than the kept state was of
    /// (made again under the id, or restored over it), so the state kept
    /// is dropped, this view starts as for new books, and true is
    /// returned: the caller drops what else it kept for them (their
    /// name).
    pub fn check_identity(&mut self) -> bool {
        let Some(now) = self.loaded_identity.clone() else {
            return false;
        };
        let changed = self.identity.as_ref().is_some_and(|kept| *kept != now);
        if changed {
            self.nav = Nav::new(Route::Overview);
            self.state.reports = Default::default();
        }
        self.identity = Some(now);
        changed
    }

    /// What a person expects to come back to: the page and the report's
    /// period, columns and comparison. (A detail page comes back as its
    /// list: the entry or the party may not be there any more.)
    pub fn keep(&self, set: &mut dyn FnMut(&str, String)) {
        use crate::pages::reports::{Columns, Compare, Range};
        set("page", route_key(&self.nav.current));
        let r = &self.state.reports;
        set(
            "range",
            match r.range {
                Range::ThisMonth => "this-month".to_string(),
                Range::LastMonth => "last-month".to_string(),
                Range::ThisQuarter => "this-quarter".to_string(),
                Range::YearToDate => "year-to-date".to_string(),
                Range::LastYear => "last-year".to_string(),
                Range::Last12Months => "last-12-months".to_string(),
                Range::AllTime => "all-time".to_string(),
                Range::Custom(s, e) => format!("custom:{s}..{e}"),
            },
        );
        set(
            "columns",
            match r.columns {
                Columns::Total => "total",
                Columns::Month => "month",
                Columns::Quarter => "quarter",
                Columns::Year => "year",
            }
            .to_string(),
        );
        set(
            "compare",
            match r.compare {
                Compare::None => "none",
                Compare::PriorPeriod => "prior-period",
                Compare::PriorYear => "prior-year",
            }
            .to_string(),
        );
        if let Some(identity) = &self.identity {
            set("identity", identity.clone());
        }
    }

    /// Take back what [`Self::keep`] kept (`get` reads a key; what isn't
    /// there, or isn't understood, is left as it is).
    pub fn restore(&mut self, get: &dyn Fn(&str) -> Option<String>) {
        use crate::pages::reports::{Columns, Compare, Range};
        self.identity = get("identity");
        if let Some(route) = get("page").and_then(|k| route_from_key(&k)) {
            self.nav = Nav::new(route);
        }
        let r = &mut self.state.reports;
        if let Some(v) = get("range") {
            let range = match v.as_str() {
                "this-month" => Some(Range::ThisMonth),
                "last-month" => Some(Range::LastMonth),
                "this-quarter" => Some(Range::ThisQuarter),
                "year-to-date" => Some(Range::YearToDate),
                "last-year" => Some(Range::LastYear),
                "last-12-months" => Some(Range::Last12Months),
                "all-time" => Some(Range::AllTime),
                other => other.strip_prefix("custom:").and_then(|d| {
                    let (s, e) = d.split_once("..")?;
                    let (s, e) = (s.parse().ok()?, e.parse().ok()?);
                    // A start after the end is never kept (BS-period-3).
                    (s <= e).then_some(Range::Custom(s, e))
                }),
            };
            if let Some(range) = range {
                r.range = range;
            }
        }
        match get("columns").as_deref() {
            Some("total") => r.columns = Columns::Total,
            Some("month") => r.columns = Columns::Month,
            Some("quarter") => r.columns = Columns::Quarter,
            Some("year") => r.columns = Columns::Year,
            _ => {}
        }
        match get("compare").as_deref() {
            Some("none") => r.compare = Compare::None,
            Some("prior-period") => r.compare = Compare::PriorPeriod,
            Some("prior-year") => r.compare = Compare::PriorYear,
            _ => {}
        }
    }
}

/// The page, as kept: a list page by name, a report by its kind; a page
/// of one thing (an entry, a party) by the list it is in.
fn route_key(r: &Route) -> String {
    match r {
        Route::Reports(k) => format!("reports:{}", report_key(*k)),
        other => section_key(other.section()).to_string(),
    }
}

fn route_from_key(key: &str) -> Option<Route> {
    if let Some(kind) = key.strip_prefix("reports:") {
        return crate::nav::ReportKind::ALL
            .into_iter()
            .find(|k| report_key(*k) == kind)
            .map(Route::Reports);
    }
    Section::ALL
        .into_iter()
        .find(|s| section_key(*s) == key)
        .map(Section::home)
}

fn section_key(s: Section) -> &'static str {
    match s {
        Section::Overview => "overview",
        Section::Cash => "cash",
        Section::Profit => "profit",
        Section::Transactions => "transactions",
        Section::Reports => "reports",
        Section::Accounts => "accounts",
        Section::Assets => "assets",
        Section::Parties => "parties",
        Section::Projects => "projects",
        Section::Invoices => "invoices",
        Section::Documents => "documents",
        Section::Company => "company",
        Section::Journal => "journal",
    }
}

fn report_key(k: crate::nav::ReportKind) -> &'static str {
    use crate::nav::ReportKind as K;
    match k {
        K::ProfitAndLoss => "profit-and-loss",
        K::BalanceSheet => "balance-sheet",
        K::CashFlow => "cash-flow",
        K::TrialBalance => "trial-balance",
        K::GeneralLedger => "general-ledger",
        K::MembersEquity => "members-equity",
        K::TaxSummary => "tax-summary",
        K::ArAging => "ar-aging",
        K::ApAging => "ap-aging",
        K::Vendor1099 => "vendor-1099",
        K::FixedAssetRegister => "fixed-asset-register",
    }
}

/// Width of the icon rail.
const RAIL_W: f32 = 60.0;
/// The footer's height (the status box and the menus under the sidebar's
/// links; the rail's equivalent) before it is first measured. After the
/// first frame its drawn heights are used ([`FooterMemo`]).
const SIDEBAR_FOOTER: f32 = 192.0;
const RAIL_FOOTER: f32 = 112.0;
/// How much shorter the footer is tightened ([`tighten`]), before it is
/// measured so.
const SIDEBAR_TIGHTEN: f32 = 36.0;
const RAIL_TIGHTEN: f32 = 36.0;

/// Space kept between the last link and the footer.
const FOOTER_GAP: f32 = 8.0;

/// Tightened: the space between links, and under the top slot. The slot
/// draws nothing (`top_slot`), and the panel's margin is the first link's
/// room under the tab bar, so both gaps are nothing today.
const GAP_MIN: f32 = 2.0;
const HEAD_GAP: f32 = 0.0;
const HEAD_GAP_TIGHT: f32 = 0.0;

/// A panel's links: how many, their height and spacing, the least height
/// they tighten to, the gap above them, and whether its status card folds.
struct Rows {
    n: usize,
    row: f32,
    gap: f32,
    row_min: f32,
    head_gap: f32,
    foldable: bool,
}

impl Rows {
    fn height(&self, row: f32, gap: f32) -> f32 {
        let n = self.n as f32;
        n * row + (n - 1.0) * gap
    }
}

const SIDEBAR_ROWS: Rows = Rows {
    n: Section::ALL.len(),
    row: 32.0,
    gap: 6.0,
    // 22, not 24, since Invoices & Bills made 13 links (2026-10-24): so
    // they still fit the owner's 1479x519 window unscrolled.
    row_min: 22.0,
    head_gap: HEAD_GAP,
    foldable: true,
};

const RAIL_ROWS: Rows = Rows {
    n: Section::ALL.len(),
    row: 36.0,
    gap: 6.0,
    // As low as the sidebar's: at 28 the rail scrolled in the owner's
    // window (review, 2026-10-22), and at 24 once there were 13 links.
    row_min: 22.0,
    head_gap: HEAD_GAP,
    foldable: false,
};

/// How a panel's links and footer share the window's height (owner,
/// 2026-10-22: at 1479x519 half the links hid behind the status box).
/// Each step is taken only when the one before isn't enough, so where
/// everything fits nothing changes.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Fit {
    /// 1. The status card folded to its summary line.
    folded: bool,
    /// 2. The rows tightened: the footer's menus, the gap under the
    ///    header, and the links' spacing, then their height.
    tight: bool,
    row: f32,
    gap: f32,
    /// 3. Even tight they don't fit: the links scroll.
    scroll: bool,
}

impl Fit {
    /// Which of the footer's three forms it draws: full, folded, tight.
    fn form(&self) -> usize {
        if self.tight {
            2
        } else if self.folded {
            1
        } else {
            0
        }
    }
}

/// The fit for `avail` points under the header, the footer being
/// `footer` high in each form ([`Fit::form`]), overrunning by `slack`
/// points ([`slack`]) taken as fitting.
fn fit(avail: f32, footer: [f32; 3], g: &Rows, slack: f32) -> Fit {
    let full = g.height(g.row, g.gap);
    let at_ease = Fit {
        folded: false,
        tight: false,
        row: g.row,
        gap: g.gap,
        scroll: false,
    };
    // The links fit when what they overrun is under half a pixel
    // (`slack`, points), which no one sees: at 1.25 the rows' sum runs a
    // hair over whole points, and without it the card folded where main
    // drew everything (review, 2026-10-22).
    let fits = |footer: f32| avail - g.head_gap - FOOTER_GAP - footer + slack >= full;
    if fits(footer[0]) {
        return at_ease;
    }
    if g.foldable && fits(footer[1]) {
        return Fit {
            folded: true,
            ..at_ease
        };
    }
    let tight = Fit {
        folded: g.foldable,
        tight: true,
        ..at_ease
    };
    let room = avail - HEAD_GAP_TIGHT - FOOTER_GAP - footer[2];
    let n = g.n as f32;
    // The space between the links gives first, then the links.
    let gap = ((room - n * g.row) / (n - 1.0)).floor();
    if gap >= GAP_MIN {
        return Fit {
            gap: gap.min(g.gap),
            ..tight
        };
    }
    let row = ((room - (n - 1.0) * GAP_MIN) / n).floor();
    if row >= g.row_min {
        return Fit {
            row: row.min(g.row),
            gap: GAP_MIN,
            ..tight
        };
    }
    Fit {
        row: g.row_min,
        gap: GAP_MIN,
        scroll: true,
        ..tight
    }
}

/// Half a pixel, in points: links overrunning their room by less are
/// drawn whole, with no cue.
fn slack(ui: &Ui) -> f32 {
    0.5 / ui.ctx().pixels_per_point()
}

/// The footer's height in each form, as last drawn; a form not drawn yet
/// is estimated from one that was.
#[derive(Clone, Copy, Debug, Default)]
struct FooterMemo([Option<f32>; 3]);

impl FooterMemo {
    /// Scoped by the panel it measures, which is the view's own
    /// (INVARIANT 2 of the module).
    fn id(ui: &Ui, key: &str) -> egui::Id {
        ui.id().with((key, "footer_memo"))
    }

    fn load(ui: &Ui, key: &str) -> FooterMemo {
        ui.ctx()
            .data(|d| d.get_temp(Self::id(ui, key)))
            .unwrap_or_default()
    }

    /// Each form's height: `guess` full before anything was drawn, a
    /// form not drawn `fold` (the card's checks) and `tighten` shorter
    /// than the one before it.
    fn heights(&self, guess: f32, fold: f32, tighten: f32) -> [f32; 3] {
        let off = [0.0, -fold, -fold - tighten];
        let anchor = (0..3)
            .find_map(|i| self.0[i].map(|h| h - off[i]))
            .unwrap_or(guess);
        std::array::from_fn(|i| self.0[i].unwrap_or(anchor + off[i]))
    }

    /// Keep form `form`'s drawn height for the next frame (the footer's
    /// content changing moves every form alike), the footer's rect for
    /// the tests, and draw again at once when it changed.
    fn measured(mut self, ui: &Ui, key: &str, form: usize, rect: egui::Rect) {
        let h = rect.height();
        let changed = match self.0[form] {
            Some(old) if (old - h).abs() <= 0.5 => false,
            Some(old) => {
                for v in self.0.iter_mut().flatten() {
                    *v += h - old;
                }
                true
            }
            None => {
                self.0[form] = Some(h);
                true
            }
        };
        ui.ctx().data_mut(|d| {
            d.insert_temp(Self::id(ui, key), self);
            d.insert_temp(egui::Id::new(format!("{key}_footer_rect")), rect);
            d.insert_temp(egui::Id::new(format!("{key}_footer_h")), h);
        });
        if changed {
            ui.ctx().request_repaint();
        }
    }
}

/// The footer's menus tightened, as the links are.
fn tighten(ui: &mut Ui) {
    let s = ui.spacing_mut();
    s.item_spacing.y = GAP_MIN;
    s.interact_size.y = 22.0;
    s.button_padding.y = 2.0;
}

/// Height of the fade over a clipped edge of the links.
const FADE: f32 = 28.0;

/// A panel's links, `row` draws one: at most `max_h` high, scrolling
/// when they don't fit, then with a fade and a chevron at each clipped
/// edge, and the current page's link scrolled into view whenever the
/// page changes. What it drew is kept for the tests
/// (`{key}_links_view`, `{key}_links_scroll`).
fn links(
    ui: &mut Ui,
    key: &str,
    fit: Fit,
    max_h: f32,
    current: Section,
    mut row: impl FnMut(&mut Ui, Section) -> egui::Response,
) {
    // Last frame's page, viewport height and scrolling: the current link
    // is scrolled into view when they change while the links scroll.
    let shown_id = ui.id().with((key, "links_shown"));
    let shown = ui
        .ctx()
        .data(|d| d.get_temp::<(Section, i32, bool)>(shown_id));
    let now = (current, max_h.round() as i32);
    let reveal = fit.scroll && shown != Some((now.0, now.1, true));
    let out = egui::ScrollArea::vertical()
        .id_salt(format!("{key}_links"))
        .max_height(max_h)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = fit.gap;
            for s in Section::ALL {
                let resp = row(ui, s);
                if reveal && s == current {
                    resp.scroll_to_me(Some(Align::Center));
                }
            }
        });
    let view = out.inner_rect;
    let content = out.content_size.y;
    let offset = out.state.offset.y;
    let scrolls = content > view.height() + slack(ui);
    ui.ctx().data_mut(|d| {
        d.insert_temp(shown_id, (now.0, now.1, scrolls));
        d.insert_temp(egui::Id::new(format!("{key}_links_view")), view);
        d.insert_temp(egui::Id::new(format!("{key}_links_scroll")), scrolls);
    });
    if !scrolls {
        return;
    }
    let p = pal(ui.ctx());
    let painter = ui.painter().with_clip_rect(view);
    let above = offset > 0.5;
    let below = offset + view.height() < content - 0.5;
    for (clipped, down) in [(above, false), (below, true)] {
        if !clipped {
            continue;
        }
        let h = FADE.min(view.height() / 2.0);
        let (band, edge_y) = if down {
            (
                egui::Rect::from_min_max(egui::pos2(view.left(), view.bottom() - h), view.max),
                view.bottom() - 8.0,
            )
        } else {
            (
                egui::Rect::from_min_max(view.min, egui::pos2(view.right(), view.top() + h)),
                view.top() + 8.0,
            )
        };
        let (top, bottom) = if down {
            (egui::Color32::TRANSPARENT, p.sidebar)
        } else {
            (p.sidebar, egui::Color32::TRANSPARENT)
        };
        let mut mesh = egui::Mesh::default();
        mesh.colored_vertex(band.left_top(), top);
        mesh.colored_vertex(band.right_top(), top);
        mesh.colored_vertex(band.left_bottom(), bottom);
        mesh.colored_vertex(band.right_bottom(), bottom);
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(1, 3, 2);
        painter.add(egui::Shape::mesh(mesh));
        // The sidebar's at the edge's right end, clear of the links'
        // names. The rail's has no room beside its icons (there it read
        // as a caret on one): centred on the edge, on a pill of its own
        // that covers the faded icon under it.
        let narrow = view.width() < 80.0;
        let chevron = if narrow {
            egui::Rect::from_center_size(
                egui::pos2(
                    view.center().x,
                    if down {
                        view.bottom() - 7.0
                    } else {
                        view.top() + 7.0
                    },
                ),
                vec2(26.0, 14.0),
            )
        } else {
            egui::Rect::from_center_size(egui::pos2(view.right() - 14.0, edge_y), vec2(24.0, 16.0))
        };
        let label = if down {
            "More links below"
        } else {
            "More links above"
        };
        let resp = ui
            .interact(chevron, ui.id().with((key, down)), Sense::click())
            .on_hover_text(label);
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
        if narrow {
            painter.rect(
                chevron,
                CornerRadius::same(7),
                if resp.hovered() { p.hover } else { p.sidebar },
                Stroke::new(1.0, p.border),
                egui::StrokeKind::Inside,
            );
        }
        painter.text(
            chevron.center(),
            egui::Align2::CENTER_CENTER,
            if down {
                icon::CARET_DOWN
            } else {
                icon::CARET_UP
            },
            egui::FontId::proportional(if narrow { 11.0 } else { 14.0 }),
            if resp.hovered() { p.text } else { p.muted },
        );
        if resp.clicked() {
            // A page of links on, keeping one link of the last in view.
            let step = (view.height() - fit.row - fit.gap).max(fit.row);
            let mut state = out.state;
            state.offset.y = (offset + if down { step } else { -step })
                .clamp(0.0, (content - view.height()).max(0.0));
            state.store(ui.ctx(), out.id);
            ui.ctx().request_repaint();
        }
    }
}

/// The sidebar's footer, bottom up: on the panel's floor when it fits
/// under the links, else right under them, running off the bottom of the
/// window, so it gives way and never draws over a link.
fn footer_layout<R>(
    ui: &mut Ui,
    footer_h: f32,
    align: Align,
    add: impl FnOnce(&mut Ui) -> R,
) -> egui::InnerResponse<R> {
    if ui.available_height() >= footer_h {
        ui.with_layout(Layout::bottom_up(align), add)
    } else {
        let w = ui.available_width();
        ui.allocate_ui_with_layout(vec2(w, footer_h), Layout::bottom_up(align), add)
    }
}

/// The top of the sidebar (and of the rail). It stays empty, on purpose: the
/// links must all fit at 1479x519 (`tests/sidenav.rs`), so nothing is added
/// above them. The assistant state is in the top bar instead, where it costs
/// no height (`topbar`, `connect::chip`).
fn top_slot(_ui: &mut Ui) {}

/// What the status card says of its company: the heading row's badge
/// (its hue, its letter) and name.
struct CardHead {
    initial: char,
    title: String,
    hue: egui::Color32,
}

/// The status card's badge, and the rail's.
const CARD_BADGE: f32 = 16.0;
/// The card's vertical padding (its sides' is 10): the footer is short.
const CARD_PAD_Y: i8 = 8;
const RAIL_BADGE: f32 = 24.0;

impl CardHead {
    fn paint(&self, ui: &Ui, rect: egui::Rect, radius: u8) {
        let font = theme::bold(if rect.width() > 20.0 { 13.0 } else { 10.0 });
        theme::paint_badge(ui.painter(), rect, radius, self.initial, self.hue, font);
    }

    /// The card's heading row, one line: the badge, small, and the name,
    /// cut with an ellipsis to the room the live mark leaves; at the
    /// card's right edge that mark, the dot and "Live" (or "Snapshot"),
    /// pulsing for two seconds after an update. The name elides before
    /// the mark is ever clipped. The mark's accessible label is "Live ·
    /// event N". `tip` is the row's hover text (none: the caller's own
    /// covers the row).
    fn row(&self, ui: &mut Ui, d: &CardView, tip: Option<String>) {
        let p = pal(ui.ctx());
        let status = d.status();
        let font = theme::semibold(12.0);
        let word = ui.fonts_mut(|f| {
            f.layout_no_wrap(status.to_string(), font.clone(), p.text)
                .size()
                .x
        });
        let gap = MARK_GAP;
        let dot = 8.0;
        let mark = dot + gap + word;
        let room = (ui.available_width() - CARD_BADGE - 2.0 * gap - mark - 1.0).max(0.0);
        // As tall as its badge, not as a button: the card is short.
        let row = ui.scope(|ui| {
            ui.spacing_mut().interact_size.y = CARD_BADGE;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                let (r, _) = ui.allocate_exact_size(vec2(CARD_BADGE, CARD_BADGE), Sense::hover());
                self.paint(ui, r, 4);
                // The name's room is all kept, so the mark is at the right.
                ui.allocate_ui_with_layout(
                    vec2(room, CARD_BADGE),
                    Layout::left_to_right(Align::Center),
                    |ui| {
                        ui.set_min_width(room);
                        ui.add(
                            egui::Label::new(
                                RichText::new(&self.title).font(theme::semibold(12.5)),
                            )
                            .truncate(),
                        );
                    },
                );
                let (r, _) = ui.allocate_exact_size(vec2(dot, dot), Sense::hover());
                let pulse = (d.since_update < 2.0) as u8 as f32;
                let color = if d.live { p.positive } else { p.faint };
                ui.painter().circle_filled(r.center(), 4.0 + pulse, color);
                let label = format!("{status} · event {}", d.seq);
                ui.add(egui::Label::new(RichText::new(status).font(font)).truncate())
                    .widget_info(|| {
                        egui::WidgetInfo::labeled(egui::WidgetType::Label, true, label.clone())
                    });
            })
        });
        if let Some(tip) = tip {
            ui.interact(row.response.rect, ui.id().with("card_head"), Sense::hover())
                .on_hover_text(tip);
        }
    }
}

/// The least space between the heading's parts.
const MARK_GAP: f32 = 4.0;

/// What the status card shows of its books: the newest event, the
/// journal's head and folder (the copy menu's), and the integrity
/// checks; and how it is drawn: live or a snapshot, how long since the
/// last update.
struct CardView {
    seq: u64,
    head: String,
    data_dir: String,
    checks: Vec<(bool, String, String)>,
    since_update: f64,
    live: bool,
}

impl CardView {
    fn new(b: &Books, since_update: f64, live: bool) -> Self {
        CardView {
            seq: b.seq(),
            head: b.head.clone(),
            data_dir: b.data_dir.display().to_string(),
            checks: checks(b),
            since_update,
            live,
        }
    }

    fn status(&self) -> &'static str {
        if self.live { "Live" } else { "Snapshot" }
    }

    /// The hover text of the heading: the company, "Live · event N",
    /// then each check, one fact a line.
    fn tip(&self, head: &CardHead) -> String {
        let mut tip = format!("{}\n{} · event {}", head.title, self.status(), self.seq);
        for (ok, text, _) in &self.checks {
            tip.push_str(&format!("\n{} {text}", check_glyph(*ok)));
        }
        tip
    }

    fn copy_menu(&self, ui: &mut Ui) {
        crate::widgets::copy_item(ui, "Copy journal head", &self.head);
        crate::widgets::copy_item(ui, "Copy data folder", &self.data_dir);
    }
}

fn check_glyph(ok: bool) -> &'static str {
    if ok {
        icon::SEAL_CHECK
    } else {
        icon::WARNING_CIRCLE
    }
}

/// A check's line: its glyph and name, small, one line, elided.
fn check_line(ui: &mut Ui, ok: bool, text: &str) -> egui::Response {
    let p = pal(ui.ctx());
    ui.add(
        egui::Label::new(
            RichText::new(format!("{} {text}", check_glyph(ok)))
                .font(theme::regular(11.5))
                .color(if ok { p.muted } else { p.negative }),
        )
        .truncate(),
    )
}

/// The status card: the company and whether its books are live in the
/// heading, and the integrity checks. Every text is one line and elides;
/// the event is on the heading's hover and its live mark's label, not on
/// a row. Folded (a short window), the heading alone (and a failed
/// check's line), the checks on hover.
fn live_status(
    ui: &mut Ui,
    b: &Books,
    since_update: f64,
    live: bool,
    folded: bool,
    head: &CardHead,
) {
    status_card(ui, &CardView::new(b, since_update, live), folded, head);
}

fn status_card(ui: &mut Ui, d: &CardView, folded: bool, head: &CardHead) {
    if folded {
        return folded_status(ui, d, head);
    }
    let p = pal(ui.ctx());
    Frame::new()
        .fill(p.surface)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(10, CARD_PAD_Y))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            for (i, (ok, text, tip)) in d.checks.iter().enumerate() {
                let r = check_line(ui, *ok, text).on_hover_text(tip);
                if i == 0 {
                    r.context_menu(|ui| d.copy_menu(ui));
                }
            }
            // The footer is laid out bottom up, so what is added last is
            // the card's top: the heading, over what the card always said.
            head.row(ui, d, Some(d.tip(head)));
        });
}

/// The status card folded. Its heading alone: the badge, the name, and at
/// the right "● Live". A check failed, a line under the heading names it
/// ([`summaries`], the fullest that fits, so it is never cut to
/// "Datab…": review, 2026-10-22). The card is one target: hovered it
/// shows the name, "Live · event N" and the three checks, and a right
/// click copies the journal head or the data folder.
fn folded_status(ui: &mut Ui, d: &CardView, head: &CardHead) {
    let p = pal(ui.ctx());
    let failed: Vec<&str> = d
        .checks
        .iter()
        .filter(|c| !c.0)
        .map(|c| c.1.as_str())
        .collect();
    // The card's inner width: the panel's less the frame's margins.
    let room = ui.available_width() - 2.0 * 10.0 - 2.0;
    let width = |s: &str| {
        ui.fonts_mut(|f| {
            f.layout_no_wrap(s.to_string(), theme::regular(12.0), p.text)
                .size()
                .x
        })
    };
    let all = summaries(&failed);
    let line = (!failed.is_empty()).then(|| {
        all.iter()
            .find(|s| width(s) <= room)
            .unwrap_or_else(|| all.last().expect("a summary"))
    });
    let card = Frame::new()
        .fill(p.surface)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(10, CARD_PAD_Y))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if let Some(line) = line {
                ui.add(
                    egui::Label::new(
                        RichText::new(line)
                            .font(theme::regular(12.0))
                            .color(p.negative),
                    )
                    .truncate(),
                );
            }
            // Bottom up, so last is the top: the heading. Its hover is the
            // card's (below): the card is one target.
            head.row(ui, d, None);
        });
    let resp = ui.interact(
        card.response.rect,
        ui.id().with("folded_status"),
        Sense::click(),
    );
    resp.on_hover_ui(|ui| {
        ui.label(RichText::new(&head.title).font(theme::semibold(12.0)));
        ui.label(
            RichText::new(format!("{} · event {}", d.status(), d.seq))
                .font(theme::regular(11.5))
                .color(p.muted),
        );
        for (ok, text, _) in &d.checks {
            check_line_hover(ui, *ok, text);
        }
    })
    .context_menu(|ui| d.copy_menu(ui));
}

/// A check's line in a tooltip (which has room to say it whole).
fn check_line_hover(ui: &mut Ui, ok: bool, text: &str) {
    let p = pal(ui.ctx());
    ui.label(
        RichText::new(format!("{} {text}", check_glyph(ok)))
            .font(theme::regular(11.5))
            .color(if ok { p.muted } else { p.negative }),
    );
}

/// The folded card's failure line, fullest first, given the checks that
/// `failed`: the names, then a count, then the shortest. A failure is
/// named before anything is shortened; "Live" is the heading's.
fn summaries(failed: &[&str]) -> Vec<String> {
    let named = format!("{} {}", icon::WARNING_CIRCLE, failed.join(", "));
    let counted = match failed.len() {
        1 => format!("{} check failed", icon::WARNING_CIRCLE),
        n => format!("{} {n} checks failed", icon::WARNING_CIRCLE),
    };
    let short = match failed.len() {
        1 => format!("{} failed", icon::WARNING_CIRCLE),
        n => format!("{} {n} failed", icon::WARNING_CIRCLE),
    };
    vec![named, counted, short]
}

/// The integrity checks the sidebar shows: (ok, label, explanation).
fn checks(b: &Books) -> Vec<(bool, String, String)> {
    use ledger_store::DiskCheck;
    let mut out = vec![(
        true,
        "Journal chain verified".to_string(),
        format!(
            "Every journal event's hash chains to the one before it.\nHead: {}\nData: {}",
            b.head,
            b.data_dir.display()
        ),
    )];
    out.push(match &b.integrity.disk {
        DiskCheck::Matches => (true, "Database matches journal".to_string(), "The server's SQLite database matches a fresh projection of the journal, every table, row for row.".to_string()),
        DiskCheck::Behind { applied, journal } => (true, "Database catching up".to_string(), format!("The database has applied {applied} of {journal} journal events; the server applies the rest right after writing them.")),
        DiskCheck::Ahead { .. } => (true, "Viewer catching up".to_string(), "The server has written journal events this view hasn't read yet; it reads them on its next refresh.".to_string()),
        DiskCheck::Missing => (true, "Read-only copy".to_string(), "No server database here: a copy of the books (a CPA's, say), or the server hasn't run yet. The journal and every file are verified either way.".to_string()),
        DiskCheck::Differs(why) => (false, "Database differs".to_string(), format!("{why}. The server rebuilds it from the journal (books.verify, or on its next start).")),
        DiskCheck::Unreadable(why) => (false, "Database unreadable".to_string(), why.clone()),
    });
    let bad = b.integrity.file_failures.len();
    out.push((
        bad == 0,
        if bad == 0 {
            format!("{} files verified", b.integrity.files_ok)
        } else {
            format!("{bad} files damaged")
        },
        "Every stored receipt and document re-hashed against its recorded SHA-256.".into(),
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The footer's heights in the owner's window (measured 2026-10-22).
    const FOOTER: [f32; 3] = [219.0, 147.0, 112.0];

    #[test]
    fn a_window_that_fits_everything_changes_nothing() {
        let f = fit(900.0, FOOTER, &SIDEBAR_ROWS, 0.5);
        assert_eq!(
            (f.folded, f.tight, f.row, f.gap, f.scroll),
            (false, false, 32.0, 6.0, false)
        );
    }

    #[test]
    fn the_sidebar_gives_way_in_order() {
        // The links' height at a row and gap, however many sections there
        // are (12 until 2026-10-24, when Invoices & Bills made 13).
        let links = |row: f32, gap: f32| SIDEBAR_ROWS.height(row, gap);
        // Card folds first.
        let f = fit(
            18.0 + links(32.0, 6.0) + 8.0 + 147.0,
            FOOTER,
            &SIDEBAR_ROWS,
            0.5,
        );
        assert!(f.folded && !f.tight && f.row == 32.0 && f.gap == 6.0);
        // Then the spacing goes, before the rows.
        let f = fit(
            8.0 + links(32.0, 4.0) + 8.0 + 112.0,
            FOOTER,
            &SIDEBAR_ROWS,
            0.5,
        );
        assert!(f.folded && f.tight && f.row == 32.0 && f.gap < 6.0 && f.gap >= GAP_MIN);
        // Then the rows, down to their least.
        let f = fit(
            8.0 + links(28.0, GAP_MIN) + 8.0 + 112.0,
            FOOTER,
            &SIDEBAR_ROWS,
            0.5,
        );
        assert!(f.tight && !f.scroll && f.gap == GAP_MIN && f.row < 32.0 && f.row >= 22.0);
        // Then they scroll.
        let f = fit(300.0, FOOTER, &SIDEBAR_ROWS, 0.5);
        assert!(f.scroll && f.row == 22.0 && f.gap == GAP_MIN);
    }

    #[test]
    fn whatever_fit_is_chosen_the_links_fit_unless_they_scroll() {
        for g in [&SIDEBAR_ROWS, &RAIL_ROWS] {
            let foot = if g.foldable {
                FOOTER
            } else {
                [150.0, 150.0, 120.0]
            };
            let mut last = (0, f32::INFINITY);
            for tenths in (1000..12000).rev() {
                let avail = tenths as f32 / 10.0;
                let f = fit(avail, foot, g, 0.5);
                let head = if f.tight { HEAD_GAP_TIGHT } else { g.head_gap };
                let need = head + g.height(f.row, f.gap) + FOOTER_GAP + foot[f.form()];
                // Within the scroll area's rounding (fit).
                assert!(f.scroll || need <= avail + 0.5, "{avail}: {f:?}");
                assert!(f.row >= g.row_min && f.row <= g.row && f.gap >= GAP_MIN);
                // Shorter windows never step back.
                let step = (f.form() + f.scroll as usize, f.row + f.gap);
                assert!(step.0 >= last.0, "{avail}: {f:?}");
                assert!(step.0 > last.0 || step.1 <= last.1, "{avail}: {f:?}");
                last = step;
            }
        }
    }

    #[test]
    fn a_folded_card_names_a_failure_before_keeping_anything_else() {
        let lines = summaries(&["Database differs"]);
        // Every line says it failed; the name goes only after the full
        // names have.
        assert!(lines.iter().all(|l| l.contains(icon::WARNING_CIRCLE)));
        let named = lines
            .iter()
            .take_while(|l| l.contains("Database differs"))
            .count();
        assert_eq!(named, 1);
        assert!(lines[named..].iter().all(|l| l.contains("failed")));
        // The event is the heading's hover, never on the line.
        assert!(lines.iter().all(|l| !l.contains("event")));
        assert!(summaries(&["a", "b"]).last().unwrap().contains("2 failed"));
    }

    /// The card at the sidebar's width (it is not resizable: 224 less the
    /// panel's margins), with a very long company name and an event of
    /// 1,234,567: every text a single line, so the card is exactly its
    /// rows and its padding, in every state.
    #[test]
    fn the_status_card_never_wraps_in_any_state() {
        use egui_kittest::Harness;
        use std::cell::RefCell;
        use std::rc::Rc;
        let check = |ok: bool, t: &str| (ok, t.to_string(), String::new());
        let passing = [
            vec![
                check(true, "Journal chain verified"),
                check(true, "Database matches journal"),
                check(true, "12345 files verified"),
            ],
            vec![
                check(true, "Journal chain verified"),
                check(true, "Database catching up"),
                check(true, "12345 files verified"),
            ],
            vec![
                check(true, "Journal chain verified"),
                check(true, "Viewer catching up"),
                check(true, "12345 files verified"),
            ],
            vec![
                check(true, "Journal chain verified"),
                check(true, "Read-only copy"),
                check(true, "12345 files verified"),
            ],
        ];
        let failing = [
            vec![
                check(true, "Journal chain verified"),
                check(false, "Database differs"),
                check(true, "12345 files verified"),
            ],
            vec![
                check(true, "Journal chain verified"),
                check(false, "Database unreadable"),
                check(false, "12345 files damaged"),
            ],
        ];
        let long = "Wanderland Labs, the Very Long Named Limited Liability Company of Somewhere";
        for width in [224.0, 200.0] {
            for folded in [false, true] {
                for live in [true, false] {
                    for checks in passing.iter().chain(failing.iter()) {
                        let tag = format!("{width} folded {folded} live {live} {checks:?}");
                        let got = Rc::new(RefCell::new(None));
                        let out = got.clone();
                        let ready = std::cell::Cell::new(false);
                        let view = CardView {
                            seq: 1_234_567,
                            head: "abc".into(),
                            data_dir: "/x".into(),
                            checks: checks.clone(),
                            since_update: 9.0,
                            live,
                        };
                        let mut h =
                            Harness::builder()
                                .with_size(vec2(width, 400.0))
                                .build_ui(move |ui| {
                                    // The fonts are bound a frame after they are
                                    // installed.
                                    if !ready.replace(true) {
                                        crate::theme::install_fonts(ui.ctx());
                                        return;
                                    }
                                    let head = CardHead {
                                        initial: 'W',
                                        title: long.to_string(),
                                        hue: egui::Color32::from_rgb(90, 120, 200),
                                    };
                                    let inner = ui.available_width() - 24.0;
                                    ui.add_space(12.0);
                                    let mut rect = None;
                                    ui.allocate_ui_with_layout(
                                        vec2(inner, 300.0),
                                        Layout::bottom_up(Align::Min),
                                        |ui| {
                                            let before = ui.cursor();
                                            status_card(ui, &view, folded, &head);
                                            rect = Some((before, ui.min_rect()));
                                        },
                                    );
                                    let y = ui.spacing().interact_size.y;
                                    let row = ui.fonts_mut(|f| f.row_height(&theme::regular(11.5)));
                                    let row12 =
                                        ui.fonts_mut(|f| f.row_height(&theme::regular(12.0)));
                                    let gap = ui.spacing().item_spacing.y;
                                    *out.borrow_mut() = Some((rect, y, row, row12, gap));
                                });
                        h.run_steps(3);
                        let (_, interact, row, row12, gap) = got.borrow().unwrap();
                        // Every text the card painted is one row, and none
                        // runs past the sidebar's inner width.
                        let mut texts = 0;
                        let mut lines = Vec::new();
                        fn walk(s: &egui::Shape, out: &mut Vec<(f32, usize, f32)>) {
                            match s {
                                egui::Shape::Text(t) => out.push((
                                    t.galley.size().y,
                                    t.galley.rows.len(),
                                    t.pos.x + t.galley.size().x,
                                )),
                                egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                                _ => {}
                            }
                        }
                        for cs in &h.output().shapes {
                            walk(&cs.shape, &mut lines);
                        }
                        for (height, rows, right) in &lines {
                            texts += 1;
                            assert_eq!(*rows, 1, "{tag}: a text wrapped");
                            assert!(*height <= row12 + 1.0, "{tag}: a text {height} high");
                            assert!(*right <= width - 12.0 + 0.5, "{tag}: a text runs out");
                        }
                        assert!(texts >= 3, "{tag}: the card painted {texts} texts");
                        // The card: its rows and its padding (10 and the
                        // stroke's 1 on each side).
                        let heading = CARD_BADGE;
                        let failed = checks.iter().any(|c| !c.0);
                        let body = if folded {
                            if failed { row12 + gap } else { 0.0 }
                        } else {
                            checks.len() as f32 * (row + gap)
                        };
                        let want = body + heading + 2.0 * CARD_PAD_Y as f32 + 2.0;
                        let (rect, ..) = got.borrow().unwrap();
                        let (_, used) = rect.unwrap();
                        let have = used.height();
                        assert!(
                            (have - want).abs() < 1.0,
                            "{tag}: the card is {have} high, its rows and padding {want} (heading {heading}, row {row}, gap {gap}, interact {interact}, used {used:?})"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn an_unmeasured_footer_form_is_reckoned_from_a_measured_one() {
        let m = FooterMemo([None, Some(150.0), None]);
        assert_eq!(m.heights(172.0, 70.0, 36.0), [220.0, 150.0, 114.0]);
        assert_eq!(
            FooterMemo::default().heights(172.0, 70.0, 36.0),
            [172.0, 102.0, 66.0]
        );
    }
}
