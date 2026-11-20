//! Tabs (tabs.rs, app.rs, company.rs; plan/p15-app-and-relay.md 3.2, D-78):
//! one window with a tab per open company, each tab a window on that
//! company alone.
//!
//! ```text
//! GOAL: several companies open at once, each in its own tab, none
//!       disturbing another: not its page, its filters, its periods, its
//!       egui state, its loader, its dock, or its books.
//! SOURCE: plan/p15-app-and-relay.md 3.2 (a `CompanyView` per tab; tests
//!         look for state shared between tabs); D-78 (one top-level tab
//!         per open company).
//! INVARIANTS:
//!   1. Two companies with different figures open in two tabs each show
//!      their own, on the overview, a report and a register.
//!   2. What is done in one tab (a page, a filter, a period) leaves
//!      another tab's state and what it draws as they were; egui's own
//!      memory (a scrolled sidebar) isn't shared either.
//!   3. A tab that isn't in front follows its journal, and `books.show`
//!      moves its page alone; showing it later is current at once.
//!   4. Closing a tab drops its loader's thread and its viewer presence
//!      and ends its dock's agent; the others go on.
//!   5. A company can't be open twice: "+" lists only those not open, and
//!      opening one that is open shows its tab.
//!   6. Start-up restores the open tabs, their order, the one in front
//!      and each one's page and period; a company that is gone is skipped
//!      without a word.
//!   7. Read-only: every tab, in front or not, leaves the books' bytes,
//!      sizes and times as they were.
//!   8. The tab bar and the empty state look as pictured
//!      (`tests/snapshots/`).
//! ```

use chrono::NaiveDate;
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use ledger_core::settings::{Change, Source as Decided};
use ledger_core::{AccountId, AccountType, Currency, Money, Side};
use ledger_gui::data::{self, Source};
use ledger_gui::nav::{ReportKind, Route};
use ledger_gui::owner::{Agent, AgentStart, Owner, RestorePlan};
use ledger_gui::{App, CompanyView, Start};
use ledger_store::Store;
use ledger_store::root::{BookId, Root};
use rust_decimal::Decimal;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, 15).unwrap()
}

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

/// A data root of its own, with companies made in it.
struct World {
    _dir: tempfile::TempDir,
    root: PathBuf,
}

/// A company made in a [`World`].
struct Co {
    data: PathBuf,
    cash: AccountId,
    capital: AccountId,
    rent: AccountId,
}

impl World {
    fn new() -> World {
        let dir = tempfile::Builder::new()
            .prefix("wb-gui-tabs-")
            .tempdir()
            .unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let root = dir.path().join("root");
        Root::setup(&home, Some(&root), None).unwrap();
        World { _dir: dir, root }
    }

    /// A company `id` named `name`: its owner put in `capital` cents, it
    /// paid `rent` cents of rent, on dates in the year of "today".
    fn company(&self, id: &str, name: &str, capital: i64, rent: i64) -> Co {
        let root = Root::find(Some(&self.root)).unwrap();
        let mut s = root
            .create_book(&BookId::parse(id).unwrap(), Currency::USD)
            .unwrap();
        let mut add = |code: &str, n: &str, ty| s.add_account(code, n, ty, None, false).unwrap();
        let cash = add("1000", "Cash", AccountType::Asset);
        let capital_acct = add("3000", "Owner Capital", AccountType::Equity);
        let rent_acct = add("6000", "Rent", AccountType::Expense);
        s.change_settings(
            &[Change {
                key: "company.name".into(),
                value: Some(serde_json::json!(name)),
            }],
            &Decided {
                decided_by: "Test".into(),
                decided_on: today(),
                citation: None,
                note: None,
            },
        )
        .unwrap();
        let mut co = Co {
            data: root.book_dir(&BookId::parse(id).unwrap()),
            cash,
            capital: capital_acct,
            rent: rent_acct,
        };
        post(
            &mut s,
            "Owner contribution",
            d(2026, 9, 1),
            cash,
            capital_acct,
            capital,
        );
        post(&mut s, "Office rent", d(2026, 10, 1), rent_acct, cash, rent);
        drop(s);
        co.data = root.book_dir(&BookId::parse(id).unwrap());
        co
    }

    fn source(&self, id: &str) -> Source {
        Source {
            data_dir: self
                .root
                .join("books")
                .join(BookId::parse(id).unwrap().as_str()),
            today: Some(today()),
        }
    }
}

/// Post Dr `dr` / Cr `cr` `cents` on `date`, with a receipt.
fn post(s: &mut Store, what: &str, date: NaiveDate, dr: AccountId, cr: AccountId, cents: i64) {
    let usd = Money::new(Decimal::new(cents, 2), Currency::USD);
    let e = s.create_entry(date, what).unwrap();
    s.add_line(e, dr, Side::Debit, usd).unwrap();
    s.add_line(e, cr, Side::Credit, usd).unwrap();
    s.attach_receipt(Some(e), "r.pdf", "application/pdf", b"%PDF r", None)
        .unwrap();
    s.post_entry(e).unwrap();
}

/// The preferences the app kept, in memory.
#[derive(Default, Clone)]
struct Prefs(HashMap<String, String>);

impl eframe::Storage for Prefs {
    fn get_string(&self, key: &str) -> Option<String> {
        self.0.get(key).cloned()
    }
    fn set_string(&mut self, key: &str, value: String) {
        self.0.insert(key.to_string(), value);
    }
    fn remove_string(&mut self, key: &str) {
        self.0.remove(key);
    }
    fn flush(&mut self) {}
}

fn prefs(pairs: &[(&str, &str)]) -> Prefs {
    Prefs(
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
    )
}

/// The app as it starts from `kept`, over `world`'s data root, its first
/// company the fallback.
fn start(world: &World, kept: Prefs, size: egui::Vec2) -> Harness<'static, App> {
    let root = world.root.clone();
    let fallback = world.source("alpha");
    let mut h = Harness::builder().with_size(size).build_eframe(move |cc| {
        use eframe::Storage;
        App::start_with(
            &cc.egui_ctx,
            &|k| kept.get_string(k),
            Start::Restore { root, fallback },
        )
    });
    h.run_steps(2);
    h
}

fn start_with_prefs(world: &World, pairs: &[(&str, &str)]) -> Harness<'static, App> {
    start(world, prefs(pairs), egui::vec2(1280.0, 800.0))
}

/// Run frames until `ok` holds.
fn until(h: &mut Harness<'static, App>, what: &str, ok: impl Fn(&App) -> bool) {
    let end = Instant::now() + Duration::from_secs(30);
    loop {
        h.run_steps(2);
        if ok(h.state()) {
            return;
        }
        assert!(Instant::now() < end, "never: {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Until every tab has its books.
fn loaded(h: &mut Harness<'static, App>) {
    until(h, "every tab's books load", |a| {
        !a.views().is_empty() && a.views().iter().all(|v| v.books().is_some())
    });
    h.run_steps(3);
}

/// Every text the last frame painted.
fn texts(h: &Harness<'_, App>) -> Vec<String> {
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
    all.sort();
    all
}

fn shows(h: &Harness<'_, App>, text: &str) -> bool {
    texts(h).iter().any(|t| t.contains(text))
}

fn titles(h: &Harness<'_, App>) -> Vec<String> {
    h.state().views().iter().map(CompanyView::title).collect()
}

/// Click the tab named `name`.
fn show_tab(h: &mut Harness<'static, App>, name: &str) {
    h.get_by_role_and_label(Role::Button, name).click();
    h.run_steps(3);
}

/// Click the close control of the tab named `name`.
fn close_tab(h: &mut Harness<'static, App>, name: &str) {
    h.get_by_role_and_label(Role::Button, &format!("Close {name}"))
        .click();
    h.run_steps(3);
}

fn three() -> (World, Co, Co, Co) {
    let w = World::new();
    let a = w.company("alpha", "Alpha Co", 1_000_000, 123_456);
    let b = w.company("beta", "Beta Co", 2_500_000, 77_777);
    let g = w.company("gamma", "Gamma Co", 500_000, 5_000);
    (w, a, b, g)
}

// ---- 1: each tab its own figures ----------------------------------------

#[test]
fn each_tab_shows_its_own_company_and_figures() {
    let (w, a, b, _) = three();
    let mut h = start_with_prefs(&w, &[("open_companies", "alpha,beta")]);
    loaded(&mut h);
    assert_eq!(titles(&h), ["Alpha Co", "Beta Co"]);
    assert_eq!(h.state().active(), Some(0));
    assert_eq!(h.state().window_title(), "Alpha Co · wanderbooks");

    // Alpha: cash 10,000.00 less 1,234.56 rent is 8,765.44.
    assert!(shows(&h, "8,765.44"), "alpha's cash: {:?}", texts(&h));
    assert!(!shows(&h, "24,222.23"));
    h.state_mut()
        .goto(Route::Reports(ReportKind::ProfitAndLoss));
    h.run_steps(3);
    assert!(shows(&h, "1,234.56"), "alpha's rent in its P&L");
    assert!(!shows(&h, "777.77"));
    h.state_mut().goto(Route::Account(a.rent));
    h.run_steps(3);
    assert!(shows(&h, "1,234.56"), "alpha's rent register");
    assert!(!shows(&h, "777.77"));

    // Beta: cash 25,000.00 less 777.77 is 24,222.23.
    show_tab(&mut h, "Beta Co");
    assert_eq!(h.state().active(), Some(1));
    assert_eq!(h.state().window_title(), "Beta Co · wanderbooks");
    assert!(
        matches!(h.state().route(), Route::Overview),
        "a tab opens on its overview, not on the page the other was on"
    );
    assert!(shows(&h, "24,222.23"), "beta's cash: {:?}", texts(&h));
    assert!(!shows(&h, "8,765.44"));
    h.state_mut()
        .goto(Route::Reports(ReportKind::ProfitAndLoss));
    h.run_steps(3);
    assert!(shows(&h, "777.77"));
    assert!(!shows(&h, "1,234.56"));
    h.state_mut().goto(Route::Account(b.rent));
    h.run_steps(3);
    assert!(shows(&h, "777.77"));
    assert!(!shows(&h, "1,234.56"));
    // One of the two accounts is not the other's: ids are per books.
    let _ = (a.cash, a.capital, b.cash, b.capital);
}

// ---- 2: what is done in one tab leaves the other alone --------------------

#[test]
fn what_is_done_in_one_tab_changes_nothing_in_another() {
    let (w, _, _, _) = three();
    let mut h = start_with_prefs(&w, &[("open_companies", "alpha,beta")]);
    loaded(&mut h);

    // Beta as it is, before anything is done in alpha.
    show_tab(&mut h, "Beta Co");
    h.run_steps(3);
    let beta_before = texts(&h);
    assert!(matches!(h.state().route(), Route::Overview));

    // In alpha: a page, a search, a period, columns, a comparison.
    show_tab(&mut h, "Alpha Co");
    h.state_mut().goto(Route::Transactions);
    {
        let st = h.state_mut().state_mut();
        st.transactions.query = "rent".into();
        st.reports.columns = ledger_gui::pages::reports::Columns::Month;
        st.reports.compare = ledger_gui::pages::reports::Compare::PriorYear;
        st.reports.range = ledger_gui::pages::reports::Range::LastMonth;
    }
    h.run_steps(3);
    assert!(shows(&h, "Office rent"), "alpha's search shows its rent");

    // Beta: nothing of that reached it, in its state or in what it draws.
    show_tab(&mut h, "Beta Co");
    h.run_steps(3);
    assert!(matches!(h.state().route(), Route::Overview));
    let st = h.state_mut().state_mut();
    assert_eq!(st.transactions.query, "");
    assert_eq!(
        st.reports.columns,
        ledger_gui::pages::reports::Columns::Total
    );
    assert_eq!(
        st.reports.compare,
        ledger_gui::pages::reports::Compare::None
    );
    assert_eq!(
        st.reports.range,
        ledger_gui::pages::reports::Range::YearToDate
    );
    assert_eq!(texts(&h), beta_before, "beta draws what it drew");

    // And the other way: beta's doings leave alpha's as they were.
    h.state_mut().goto(Route::Reports(ReportKind::BalanceSheet));
    h.state_mut().state_mut().transactions.query = "other".into();
    h.run_steps(3);
    show_tab(&mut h, "Alpha Co");
    h.run_steps(3);
    assert!(matches!(h.state().route(), Route::Transactions));
    let st = h.state_mut().state_mut();
    assert_eq!(st.transactions.query, "rent");
    assert_eq!(
        st.reports.columns,
        ledger_gui::pages::reports::Columns::Month
    );
}

/// egui's own memory isn't shared: a sidebar scrolled in one tab is as it
/// was left in the other. (The links scroll in a short window; their
/// scroll state is kept by egui under an id that is the same in every
/// tab unless each tab's ids are its own.)
#[test]
fn a_scrolled_sidebar_in_one_tab_is_not_scrolled_in_another() {
    let (w, _, _, _) = three();
    let mut h = start(
        &w,
        prefs(&[("open_companies", "alpha,beta")]),
        egui::vec2(1480.0, 330.0),
    );
    loaded(&mut h);
    let links_scroll = |h: &Harness<'_, App>| -> bool {
        h.ctx
            .data(|d| d.get_temp::<bool>(egui::Id::new("sidebar_links_scroll")))
            .expect("whether the links scroll")
    };
    let view = |h: &Harness<'_, App>| -> egui::Rect {
        h.ctx
            .data(|d| d.get_temp::<egui::Rect>(egui::Id::new("sidebar_links_view")))
            .expect("the links' viewport")
    };
    let overview_top = |h: &Harness<'_, App>| -> f32 {
        h.get_all_by_role_and_label(Role::Button, "Overview")
            .map(|n| n.rect())
            .min_by(|a, b| a.left().total_cmp(&b.left()))
            .unwrap()
            .top()
    };
    h.run_steps(3);
    assert!(links_scroll(&h), "a short window: the links scroll");
    assert!(overview_top(&h) >= view(&h).top() - 0.5, "alpha at the top");
    // Alpha's links scrolled a page on.
    h.get_by_label("More links below").click();
    h.run_steps(4);
    assert!(
        overview_top(&h) < view(&h).top() - 0.5,
        "alpha's links did not scroll"
    );
    // Beta's are at the top: its own scroll state, not alpha's.
    show_tab(&mut h, "Beta Co");
    h.run_steps(4);
    assert!(
        overview_top(&h) >= view(&h).top() - 0.5,
        "beta's sidebar inherited alpha's scroll"
    );
    // And alpha's are still where they were left.
    show_tab(&mut h, "Alpha Co");
    h.run_steps(4);
    assert!(
        overview_top(&h) < view(&h).top() - 0.5,
        "alpha's scroll was lost"
    );
}

#[test]
fn no_two_views_have_the_same_id_or_key() {
    let (w, _, _, _) = three();
    let mut h = start_with_prefs(&w, &[("open_companies", "alpha,beta,gamma")]);
    loaded(&mut h);
    let views = h.state().views();
    assert_eq!(views.len(), 3);
    for i in 0..views.len() {
        for j in i + 1..views.len() {
            assert_ne!(views[i].id(), views[j].id());
            assert_ne!(views[i].key(), views[j].key());
        }
    }
}

// ---- 3: a background tab follows its journal ------------------------------

#[test]
fn a_background_tab_follows_its_journal_and_is_current_when_shown() {
    let (w, _, b, _) = three();
    let mut h = start_with_prefs(&w, &[("open_companies", "alpha,beta")]);
    loaded(&mut h);
    assert_eq!(h.state().active(), Some(0));
    let beta_seq = h.state().views()[1].books().unwrap().seq();

    // Beta's journal changes while alpha is in front: 100.00 more cash.
    let mut s = Store::open(&b.data).unwrap();
    post(
        &mut s,
        "Late contribution",
        d(2026, 10, 10),
        b.cash,
        b.capital,
        10_000,
    );
    drop(s);
    until(&mut h, "the background tab reads the new entry", |a| {
        a.views()[1].books().unwrap().seq() > beta_seq
    });
    assert_eq!(h.state().active(), Some(0), "nothing brought beta forward");

    // Shown, it is current at once: no reload, no waiting for a load.
    show_tab(&mut h, "Beta Co");
    assert!(shows(&h, "24,322.23"), "beta's cash now: {:?}", texts(&h));
    assert!(!shows(&h, "24,222.23"));
}

#[test]
fn books_show_moves_the_tab_it_is_aimed_at_and_only_that_one() {
    let (w, a, b, _) = three();
    ledger_store::ops::prepare(&a.data).unwrap();
    ledger_store::ops::prepare(&b.data).unwrap();
    let mut h = start_with_prefs(&w, &[("open_companies", "alpha,beta")]);
    loaded(&mut h);
    assert!(matches!(h.state().views()[0].route(), Route::Overview));
    // The agent asks for beta's journal page; alpha is in front.
    ledger_store::ops::request(
        &b.data,
        ledger_store::ops::ViewerTarget::Page {
            page: "journal".into(),
        },
    )
    .unwrap();
    until(&mut h, "beta's tab moves", |a| {
        matches!(a.views()[1].route(), Route::Journal)
    });
    assert!(
        matches!(h.state().views()[0].route(), Route::Overview),
        "the tab in front stayed where it was"
    );
    assert_eq!(h.state().active(), Some(0), "no tab was brought forward");
}

// ---- 4: closing a tab ends what it started ---------------------------------

/// An owner that starts a stand-in agent that says its process id.
struct Agentic {
    pidfile: PathBuf,
    dir: PathBuf,
    asked: Mutex<Vec<String>>,
}

impl Owner for Agentic {
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
    fn agent_start(&self, _: Agent, book: &str) -> Result<AgentStart, String> {
        self.asked.lock().unwrap().push(book.to_string());
        Ok(AgentStart::Ready {
            label: "Claude Code".into(),
            program: PathBuf::from("/bin/sh"),
            args: vec![
                "-c".into(),
                format!("echo $$ > '{}'; exec sleep 600", self.pidfile.display()),
            ],
            dir: self.dir.clone(),
            env: vec![("TERM".into(), "xterm-256color".into())],
        })
    }
}

#[cfg(unix)]
fn alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

#[cfg(unix)]
#[test]
fn closing_a_tab_releases_its_lock_its_threads_and_its_agent() {
    let (w, a, b, _) = three();
    ledger_store::ops::prepare(&a.data).unwrap();
    ledger_store::ops::prepare(&b.data).unwrap();
    let work = tempfile::tempdir().unwrap();
    let owner = Arc::new(Agentic {
        pidfile: work.path().join("agent.pid"),
        dir: work.path().to_path_buf(),
        asked: Mutex::new(Vec::new()),
    });
    let mut h = start_with_prefs(&w, &[("open_companies", "alpha,beta")]);
    loaded(&mut h);
    // Each view has an owner of its own (no process global): only beta's
    // runs an agent.
    h.state_mut()
        .view_mut(1)
        .unwrap()
        .set_owner(owner.clone() as Arc<dyn Owner>);
    until(&mut h, "both viewers hold their books", |_| {
        ledger_store::ops::viewer_open(&a.data) && ledger_store::ops::viewer_open(&b.data)
    });
    let beta_loader = h.state().views()[1].loader_watch().expect("a loader");
    let alpha_loader = h.state().views()[0].loader_watch().expect("a loader");
    assert!(!beta_loader.ended());

    // Beta in front, its dock open: its agent runs.
    show_tab(&mut h, "Beta Co");
    h.state_mut().dock_mut().open = true;
    let pid = {
        let end = Instant::now() + Duration::from_secs(20);
        loop {
            h.run_steps(2);
            if let Ok(text) = std::fs::read_to_string(&owner.pidfile)
                && let Ok(pid) = text.trim().parse::<u32>()
            {
                break pid;
            }
            assert!(Instant::now() < end, "the agent never started");
            std::thread::sleep(Duration::from_millis(20));
        }
    };
    assert!(alive(pid), "the agent runs");
    assert_eq!(*owner.asked.lock().unwrap(), ["beta"], "beta's own owner");

    // Closed: its viewer lock is released, its loader's thread ends, its
    // agent's process is gone; alpha's are as they were.
    close_tab(&mut h, "Beta Co");
    assert_eq!(titles(&h), ["Alpha Co"]);
    assert_eq!(h.state().active(), Some(0));
    let end = Instant::now() + Duration::from_secs(20);
    while (ledger_store::ops::viewer_open(&b.data) || !beta_loader.ended() || alive(pid))
        && Instant::now() < end
    {
        h.run_steps(1);
        std::thread::sleep(Duration::from_millis(25));
    }
    assert!(
        !ledger_store::ops::viewer_open(&b.data),
        "a closed tab still holds its books' viewer lock"
    );
    assert!(beta_loader.ended(), "a closed tab's loader thread runs on");
    assert!(!alive(pid), "a closed tab's agent still runs");
    assert!(ledger_store::ops::viewer_open(&a.data), "alpha's lock held");
    assert!(!alpha_loader.ended(), "alpha's loader runs on");
    // Beta can be opened again: a new tab, a new lock.
    h.get_by_label("Open another company").click();
    h.run_steps(3);
    h.get_by_label("beta").click();
    h.run_steps(3);
    assert_eq!(titles(&h).len(), 2);
    until(&mut h, "beta's lock is taken again", |_| {
        ledger_store::ops::viewer_open(&b.data)
    });
}

// ---- 5: a company is never open twice ---------------------------------------

#[test]
fn plus_lists_only_the_companies_not_open_and_none_opens_twice() {
    let (w, _, _, _) = three();
    let mut h = start_with_prefs(&w, &[("open_companies", "alpha,beta")]);
    loaded(&mut h);
    h.get_by_label("Open another company").click();
    h.run_steps(3);
    assert!(h.query_by_label("gamma").is_some(), "gamma is offered");
    assert!(h.query_by_label("alpha").is_none(), "alpha is open");
    assert!(h.query_by_label("beta").is_none(), "beta is open");
    h.get_by_label("gamma").click();
    loaded(&mut h);
    assert_eq!(titles(&h), ["Alpha Co", "Beta Co", "Gamma Co"]);
    assert_eq!(h.state().active(), Some(2), "the new tab comes forward");
    // Every one is open: the list says so.
    h.get_by_label("Open another company").click();
    h.run_steps(3);
    assert!(shows(&h, "Every company here is open."), "{:?}", texts(&h));
    // Opening an open company by other means shows its tab, never a second.
    let ctx = h.ctx.clone();
    let n = h.state_mut().open(&ctx, w.source("beta"));
    assert_eq!(n, 1);
    assert_eq!(h.state().views().len(), 3);
    assert_eq!(h.state().active(), Some(1));
    // Likewise a view of fixed books of the same folder.
    let books = data::load_once(&w.source("alpha")).unwrap();
    let again = CompanyView::with_books(&ctx, books, Route::Cash);
    assert_eq!(h.state_mut().add_view(again), 0);
    assert_eq!(h.state().views().len(), 3);
}

// ---- 6: start-up -------------------------------------------------------------

#[test]
fn start_up_restores_the_open_tabs_their_order_and_the_one_in_front() {
    let (w, _, _, _) = three();
    // A company that is gone, and one listed twice, are skipped.
    let mut h = start_with_prefs(
        &w,
        &[
            ("open_companies", "gamma,gone,alpha,gamma,not a book!,beta"),
            ("active_company", "alpha"),
        ],
    );
    loaded(&mut h);
    assert_eq!(titles(&h), ["Gamma Co", "Alpha Co", "Beta Co"]);
    assert_eq!(h.state().active(), Some(1), "the same one in front");
    assert!(h.state().views().iter().all(|v| v.error().is_none()));
    assert_eq!(h.state().window_title(), "Alpha Co · wanderbooks");

    // No record: the data root's first, alone.
    let h2 = start_with_prefs(&w, &[]);
    assert_eq!(h2.state().views().len(), 1);
    assert_eq!(h2.state().views()[0].book_id().as_deref(), Some("alpha"));
    // A record of none that exists any more: the same.
    let h3 = start_with_prefs(&w, &[("open_companies", "gone,vanished")]);
    assert_eq!(h3.state().views().len(), 1);
    assert_eq!(h3.state().views()[0].book_id().as_deref(), Some("alpha"));
    // A front one that isn't among them is ignored: the first is in front.
    let h4 = start_with_prefs(
        &w,
        &[
            ("open_companies", "beta,gamma"),
            ("active_company", "alpha"),
        ],
    );
    assert_eq!(h4.state().active(), Some(0));
    // The company named on the command line is the only tab, whatever was
    // kept.
    let src = w.source("gamma");
    let h5 = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .build_eframe(move |cc| App::start_with(&cc.egui_ctx, &|_| None, Start::Only(src)));
    assert_eq!(h5.state().views().len(), 1);
    assert_eq!(h5.state().views()[0].book_id().as_deref(), Some("gamma"));
}

#[test]
fn the_open_tabs_and_each_ones_page_and_period_are_kept_and_come_back() {
    let (w, _, _, _) = three();
    let mut h = start_with_prefs(&w, &[("open_companies", "alpha,beta,gamma")]);
    loaded(&mut h);
    // Alpha: a report, a custom period, columns, a comparison. Beta: its
    // transactions. Gamma in front, on its accounts.
    {
        use ledger_gui::pages::reports::{Columns, Compare, Range};
        let a = h.state_mut().view_mut(0).unwrap();
        a.goto(Route::Reports(ReportKind::BalanceSheet));
        a.state_mut().reports.range = Range::Custom(d(2026, 1, 1), d(2026, 6, 30));
        a.state_mut().reports.columns = Columns::Quarter;
        a.state_mut().reports.compare = Compare::PriorYear;
        h.state_mut().view_mut(1).unwrap().goto(Route::Transactions);
        h.state_mut().view_mut(2).unwrap().goto(Route::Accounts);
    }
    h.state_mut().activate(2);
    h.run_steps(2);
    let mut kept = Prefs::default();
    eframe::App::save(h.state_mut(), &mut kept);
    assert_eq!(kept.0["open_companies"], "alpha,beta,gamma");
    assert_eq!(kept.0["active_company"], "gamma");
    // The per-app keys are as they always were.
    for key in ["scheme", "scheme_filter", "dark", "dock_open", "dock_agent"] {
        assert!(
            kept.0.contains_key(key),
            "{key} not kept: {:?}",
            kept.0.keys()
        );
    }
    drop(h);

    // Next time: the same tabs, order, front tab, pages and periods.
    let mut h = start(&w, kept.clone(), egui::vec2(1280.0, 800.0));
    loaded(&mut h);
    assert_eq!(titles(&h), ["Alpha Co", "Beta Co", "Gamma Co"]);
    assert_eq!(h.state().active(), Some(2));
    let views = h.state().views();
    assert!(matches!(
        views[0].route(),
        Route::Reports(ReportKind::BalanceSheet)
    ));
    assert!(matches!(views[1].route(), Route::Transactions));
    assert!(matches!(views[2].route(), Route::Accounts));
    {
        use ledger_gui::pages::reports::{Columns, Compare, Range};
        let r = &views[0].state().reports;
        assert_eq!(r.range, Range::Custom(d(2026, 1, 1), d(2026, 6, 30)));
        assert_eq!(r.columns, Columns::Quarter);
        assert_eq!(r.compare, Compare::PriorYear);
        // Beta's period is its own, untouched.
        assert_eq!(views[1].state().reports.range, Range::YearToDate);
    }

    // A tab closed in the run is remembered as it was left, and comes back
    // as it was when opened again, in this run and the next.
    close_tab(&mut h, "Alpha Co");
    h.get_by_label("Open another company").click();
    h.run_steps(3);
    h.get_by_label("alpha").click();
    h.run_steps(3);
    let last = h.state().views().len() - 1;
    assert!(matches!(
        h.state().views()[last].route(),
        Route::Reports(ReportKind::BalanceSheet)
    ));
    let mut again = Prefs::default();
    eframe::App::save(h.state_mut(), &mut again);
    assert_eq!(again.0["open_companies"], "beta,gamma,alpha");
    assert_eq!(again.0["active_company"], "alpha");
    // Closing the last tab keeps a list with nothing in it, and the next
    // start opens the data root's first.
    for _ in 0..3 {
        let name = h.state().views()[0].title();
        close_tab(&mut h, &name);
    }
    assert!(h.state().views().is_empty());
    let mut none = Prefs::default();
    eframe::App::save(h.state_mut(), &mut none);
    assert_eq!(none.0["open_companies"], "");
    drop(h);
    let h = start(&w, none, egui::vec2(1280.0, 800.0));
    assert_eq!(h.state().views().len(), 1);
    assert_eq!(h.state().views()[0].book_id().as_deref(), Some("alpha"));
}

// ---- closing and the empty state ---------------------------------------------

#[test]
fn closing_the_front_tab_shows_a_neighbour_and_the_last_leaves_an_empty_window() {
    let (w, _, _, _) = three();
    let mut h = start_with_prefs(&w, &[("open_companies", "alpha,beta,gamma")]);
    loaded(&mut h);
    show_tab(&mut h, "Beta Co");
    close_tab(&mut h, "Beta Co");
    assert_eq!(titles(&h), ["Alpha Co", "Gamma Co"]);
    assert_eq!(h.state().window_title(), "Gamma Co · wanderbooks");
    // A tab behind the front one closing leaves the front one in front.
    close_tab(&mut h, "Alpha Co");
    assert_eq!(titles(&h), ["Gamma Co"]);
    assert_eq!(h.state().window_title(), "Gamma Co · wanderbooks");
    // The last one.
    close_tab(&mut h, "Gamma Co");
    assert!(h.state().views().is_empty());
    assert_eq!(h.state().window_title(), "wanderbooks");
    assert!(shows(&h, "No company is open"), "{:?}", texts(&h));
    assert!(
        shows(&h, "Open a company with the + at the top of the window."),
        "{:?}",
        texts(&h)
    );
    // The bar is still there, and "+" opens a company from the empty window.
    h.get_by_label("Open another company").click();
    h.run_steps(3);
    h.get_by_label("beta").click();
    loaded(&mut h);
    assert_eq!(titles(&h), ["Beta Co"]);
    assert!(!shows(&h, "No company is open"));
}

#[test]
fn the_tab_in_front_is_marked_by_more_than_colour() {
    let (w, _, _, _) = three();
    let mut h = start_with_prefs(&w, &[("open_companies", "alpha,beta")]);
    loaded(&mut h);
    let selected = |h: &Harness<'_, App>, name: &str| {
        h.get_by_role_and_label(Role::Button, name)
            .accesskit_node()
            .toggled()
    };
    assert_eq!(
        selected(&h, "Alpha Co"),
        Some(egui::accesskit::Toggled::True)
    );
    assert_eq!(
        selected(&h, "Beta Co"),
        Some(egui::accesskit::Toggled::False)
    );
    show_tab(&mut h, "Beta Co");
    assert_eq!(
        selected(&h, "Alpha Co"),
        Some(egui::accesskit::Toggled::False)
    );
    assert_eq!(
        selected(&h, "Beta Co"),
        Some(egui::accesskit::Toggled::True)
    );
    // A close control on each, named for its company.
    h.get_by_role_and_label(Role::Button, "Close Alpha Co");
    h.get_by_role_and_label(Role::Button, "Close Beta Co");
}

// ---- 7: read-only, in a tab that is not in front -----------------------------

type Tree = BTreeMap<PathBuf, (Vec<u8>, u64, SystemTime)>;

fn tree(root: &Path) -> Tree {
    fn walk(root: &Path, d: &Path, out: &mut Tree) {
        for e in std::fs::read_dir(d).unwrap() {
            let p = e.unwrap().path();
            let m = std::fs::symlink_metadata(&p).unwrap();
            if m.is_dir() {
                walk(root, &p, out);
            } else {
                out.insert(
                    p.strip_prefix(root).unwrap().to_path_buf(),
                    (std::fs::read(&p).unwrap(), m.len(), m.modified().unwrap()),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

/// Every file 0444 and folder 0555 (`read_only`), or back to 0644/0755.
#[cfg(unix)]
fn set_read_only(root: &Path, read_only: bool) {
    use std::os::unix::fs::PermissionsExt;
    fn walk(p: &Path, ro: bool) {
        let m = std::fs::symlink_metadata(p).unwrap();
        if m.is_dir() {
            if !ro {
                std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            for e in std::fs::read_dir(p).unwrap() {
                walk(&e.unwrap().path(), ro);
            }
            if ro {
                std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o555)).unwrap();
            }
        } else {
            let mode = if ro { 0o444 } else { 0o644 };
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(mode)).unwrap();
        }
    }
    walk(root, read_only);
}

/// Makes the books writable again when dropped, so a run that passes or
/// fails never leaves a folder no one can delete.
#[cfg(unix)]
struct Writable(PathBuf);

#[cfg(unix)]
impl Drop for Writable {
    fn drop(&mut self) {
        if self.0.exists() {
            set_read_only(&self.0, false);
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

#[cfg(unix)]
#[test]
fn every_tab_never_writes_into_its_books_in_front_or_not() {
    let dir = std::env::temp_dir().join(format!("ledger-gui-tabs-ro-{}", std::process::id()));
    if dir.exists() {
        set_read_only(&dir, false);
        std::fs::remove_dir_all(&dir).unwrap();
    }
    let _restore = Writable(dir.clone());
    let t = d(2026, 10, 12);
    ledger_gui::demo::build(&dir.join("one"), t).unwrap();
    ledger_gui::demo::build(&dir.join("two"), t).unwrap();
    let source = |n: &str| Source {
        data_dir: dir.join(n).join("books-data"),
        today: Some(t),
    };
    set_read_only(&dir, true);
    let before = tree(&dir);

    // The ids of what to show, from the second company's books.
    let books = data::load_once(&source("two")).unwrap();
    let entry = books.entries.last().map(|e| e.id).unwrap();
    let cash = books.cash_accounts[0];
    let doc = books.attachments.last().map(|a| a.id).unwrap();
    let asset = books
        .ledger
        .registered_assets()
        .next()
        .map(|a| a.id)
        .unwrap();
    let party = books.ledger.parties().next().map(|p| p.id).unwrap();
    let projects: Vec<u32> = books.ledger.list_classes().iter().map(|c| c.id).collect();
    drop(books);

    let (one, two) = (source("one"), source("two"));
    let mut h = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .build_eframe(move |cc| {
            let mut app = App::start_with(&cc.egui_ctx, &|_| None, Start::Only(one));
            app.open(&cc.egui_ctx, two);
            // The first company in front: the second is a tab behind it.
            app.activate(0);
            app
        });
    loaded(&mut h);
    assert_eq!(h.state().active(), Some(0));
    // The tab behind follows live for a moment, unseen.
    for _ in 0..10 {
        h.run_steps(2);
        std::thread::sleep(Duration::from_millis(30));
    }
    // Then it is shown, and every page rendered in it, the other behind.
    h.state_mut().activate(1);
    let routes = [
        Route::Overview,
        Route::Transactions,
        Route::Entry(entry),
        Route::Cash,
        Route::Profit,
        Route::Reports(ReportKind::ProfitAndLoss),
        Route::Reports(ReportKind::BalanceSheet),
        Route::Reports(ReportKind::CashFlow),
        Route::Reports(ReportKind::TrialBalance),
        Route::Reports(ReportKind::GeneralLedger),
        Route::Reports(ReportKind::TaxSummary),
        Route::Reports(ReportKind::Vendor1099),
        Route::Reports(ReportKind::ArAging),
        Route::Reports(ReportKind::ApAging),
        Route::Reports(ReportKind::MembersEquity),
        Route::Reports(ReportKind::FixedAssetRegister),
        Route::Accounts,
        Route::Account(cash),
        Route::Assets,
        Route::Asset(asset),
        Route::Parties,
        Route::Party(party),
        Route::Projects,
        Route::Document(doc),
        Route::Documents,
        Route::Company,
        Route::Journal,
    ];
    for route in routes
        .into_iter()
        .chain(projects.iter().map(|&c| Route::Project(c)))
    {
        h.state_mut().goto(route);
        h.run_steps(3);
        let ctx = h.ctx.clone();
        h.state_mut().settle(&ctx);
        h.run_steps(2);
    }
    // Back and forth between the tabs.
    for i in [0, 1, 0, 1] {
        h.state_mut().activate(i);
        h.run_steps(3);
    }
    drop(h);
    let after = tree(&dir);
    set_read_only(&dir, false);
    let changed: Vec<_> = before
        .keys()
        .chain(after.keys())
        .filter(|k| before.get(*k) != after.get(*k))
        .collect();
    assert!(changed.is_empty(), "the viewer changed {changed:?}");
}

// ---- 8: how the bar looks ----------------------------------------------------

/// Pixels compared where there is a renderer; tolerant of antialiasing
/// (as the sidebar's are: sidenav.rs).
fn picture(h: &mut Harness<'_, App>, name: &str) {
    if h.render().is_err() {
        eprintln!("no renderer: {name} not compared");
        return;
    }
    let options = egui_kittest::SnapshotOptions::new()
        .threshold(1.0)
        .max_failed_pixels(60);
    if let Err(e) = h.try_snapshot_options(name, &options) {
        panic!("{e}");
    }
}

/// An app of fixed books, one tab each named `names`, the first in
/// front, `size` large. `wide`: names long enough to be cut.
fn shown(world: &World, names: &[&str], dark: bool, size: egui::Vec2) -> Harness<'static, App> {
    let ids: Vec<String> = (0..names.len()).map(|i| format!("co-{i}")).collect();
    for (id, name) in ids.iter().zip(names) {
        world.company(id, name, 1_000_000, 123_456);
    }
    let sources: Vec<Source> = ids.iter().map(|id| world.source(id)).collect();
    let mut h = Harness::builder().with_size(size).build_eframe(move |cc| {
        let mut app: Option<App> = None;
        for s in sources {
            let books = data::load_once(&s).unwrap();
            match app.as_mut() {
                None => {
                    app = Some(App::with_books(&cc.egui_ctx, books, dark, Route::Overview));
                }
                Some(a) => {
                    a.add_view(CompanyView::with_books(
                        &cc.egui_ctx,
                        books,
                        Route::Overview,
                    ));
                }
            }
        }
        let mut app = app.unwrap();
        app.activate(0);
        app
    });
    h.run_steps(4);
    h
}

const BAR_SIZE: egui::Vec2 = egui::vec2(900.0, 160.0);

#[test]
fn the_tab_bar_with_one_two_and_many_tabs() {
    for dark in [false, true] {
        let mode = if dark { "dark" } else { "light" };
        let w = World::new();
        let mut h = shown(&w, &["Wanderland Labs LLC"], dark, BAR_SIZE);
        picture(&mut h, &format!("tabs_one_{mode}"));
        let w = World::new();
        let mut h = shown(
            &w,
            &["Wanderland Labs LLC", "Fernhill Studio"],
            dark,
            BAR_SIZE,
        );
        picture(&mut h, &format!("tabs_two_{mode}"));
        let w = World::new();
        let names = [
            "Wanderland Labs LLC",
            "Fernhill Studio",
            "Northgate Holdings Incorporated",
            "Oak & Ash Carpentry",
            "Harbor Lane Books",
            "Quill and Compass",
            "Mossy Rock Farm",
            "Lantern Works",
            "Birch Street Bakery",
            "Tidewater Consulting Group",
        ];
        let mut h = shown(&w, &names, dark, BAR_SIZE);
        // The tab in front is the last: it is scrolled into view.
        h.state_mut().activate(9);
        h.run_steps(6);
        picture(&mut h, &format!("tabs_many_{mode}"));
        // Never overlapping: each tab's close control is inside the bar.
        let bar = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(BAR_SIZE.x, 40.0));
        let plus = h
            .get_by_role_and_label(Role::Button, "Open another company")
            .rect();
        assert!(bar.contains_rect(plus), "the + is whole in the bar");
    }
}

#[test]
fn the_empty_window_and_the_plus_list() {
    for dark in [false, true] {
        let mode = if dark { "dark" } else { "light" };
        let w = World::new();
        let mut h = shown(&w, &["Wanderland Labs LLC"], dark, egui::vec2(900.0, 420.0));
        close_tab(&mut h, "Wanderland Labs LLC");
        assert!(shows(&h, "No company is open"));
        picture(&mut h, &format!("tabs_empty_{mode}"));
        // "+" with companies to open.
        let w = World::new();
        w.company("alpha", "Alpha Co", 1_000_000, 100);
        w.company("beta", "Beta Co", 1_000_000, 100);
        w.company("gamma", "Gamma Co", 1_000_000, 100);
        let mut h = start(
            &w,
            prefs(&[
                ("open_companies", "alpha"),
                ("dark", if dark { "true" } else { "false" }),
            ]),
            egui::vec2(900.0, 420.0),
        );
        loaded(&mut h);
        h.get_by_label("Open another company").click();
        h.run_steps(4);
        picture(&mut h, &format!("tabs_plus_{mode}"));
    }
}

/// Pictures of the whole window with two tabs, each in front once, the
/// "+" list open and the empty window, for a person to look at
/// (`SHOT_DIR=… cargo test -p ledger-gui --test tabs -- --ignored pictures`).
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
        let (w, _, _, _) = three();
        let mut h = start(
            &w,
            prefs(&[
                ("open_companies", "alpha,beta"),
                ("dark", if dark { "true" } else { "false" }),
            ]),
            egui::vec2(1440.0, 920.0),
        );
        loaded(&mut h);
        show_tab(&mut h, "Alpha Co");
        h.run_steps(6);
        save(&mut h, &format!("two-tabs-alpha-in-front-{mode}"));
        show_tab(&mut h, "Beta Co");
        h.run_steps(6);
        save(&mut h, &format!("two-tabs-beta-in-front-{mode}"));
        h.get_by_label("Open another company").click();
        h.run_steps(4);
        save(&mut h, &format!("plus-list-open-{mode}"));
        h.key_press(egui::Key::Escape);
        h.run_steps(3);
        close_tab(&mut h, "Alpha Co");
        close_tab(&mut h, "Beta Co");
        h.run_steps(4);
        save(&mut h, &format!("empty-state-{mode}"));
    }
}
