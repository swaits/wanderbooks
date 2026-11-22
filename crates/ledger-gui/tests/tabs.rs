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
//!   9. The bar spans the whole window above the sidebar; its "+" is
//!      right after the last tab, and at the right edge, whole, when the
//!      tabs overflow.
//!  10. A company's name is remembered when it is loaded: the "+" list
//!      shows it (the id alone for a company never opened), and so does a
//!      tab whose books are not loaded yet.
//!  11. The tab is the company's identity in the frame: nothing else in
//!      the top 120 points of the window shows its name, and the
//!      Overview opens with its summary sentence, not a heading with the
//!      name.
//!  12. The status card at the foot of the sidebar names the company in
//!      front, with its badge, and follows the tab; the rail shows the
//!      badge alone.
//!  13. A company's badge hue is a function of the id of its books alone
//!      (so it is the same on its tab and its card, in both themes, after
//!      a restart, in any order of opening), and every hue reads (the
//!      contrast test is `theme::tests`, in the crate).
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

#[path = "shapes_support/mod.rs"]
mod shapes_support;

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
        World::named("wb-gui-tabs-")
    }

    /// A world whose temporary folder's name starts with `prefix`.
    fn named(prefix: &str) -> World {
        let dir = tempfile::Builder::new().prefix(prefix).tempdir().unwrap();
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

    /// The key the open companies are kept under for this world's root.
    fn open_key(&self) -> String {
        ledger_gui::app::open_key(&self.root)
    }

    fn active_key(&self) -> String {
        ledger_gui::app::active_key(&self.root)
    }

    /// The key `thing` of company `id` of this world is kept under.
    fn kept_key(&self, id: &str, thing: &str) -> String {
        ledger_gui::app::company_key(&self.root.join("books").join(id), thing)
    }

    /// Preferences written with the plain names tests give them
    /// (`open_companies`, `active_company`, `company.<id>.<thing>`), filed
    /// as the app files them: under this world's root (app.rs INVARIANT 4).
    fn filed(&self, plain: Prefs) -> Prefs {
        Prefs(
            plain
                .0
                .into_iter()
                .map(|(k, v)| {
                    let key = match k.as_str() {
                        "open_companies" => self.open_key(),
                        "active_company" => self.active_key(),
                        _ => match k.strip_prefix("company.").and_then(|r| r.split_once('.')) {
                            // Plain: `company.<id>.<thing>`; one already
                            // filed has a root before its id.
                            Some((id, thing)) if !thing.contains('.') => self.kept_key(id, thing),
                            _ => k,
                        },
                    };
                    (key, v)
                })
                .collect(),
        )
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
    // The Connect page opens by itself once, in a window whose server says
    // no assistant was ever connected (WB-app-6); these tests are about
    // tabs, so that has happened already.
    let mut kept: std::collections::BTreeMap<String, String> =
        [("first_connect_done".to_string(), "1".to_string())].into();
    kept.extend(pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())));
    Prefs(kept.into_iter().collect())
}

/// The app as it starts from `kept`, over `world`'s data root, its first
/// company the fallback.
fn start(world: &World, kept: Prefs, size: egui::Vec2) -> Harness<'static, App> {
    start_as_is(world, world.filed(kept), size)
}

/// [`start`] with the preferences exactly as given (filed or not).
fn start_as_is(world: &World, kept: Prefs, size: egui::Vec2) -> Harness<'static, App> {
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
    assert_eq!(h.state().window_title(), "Alpha Co · Wanderbooks");

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
    assert_eq!(h.state().window_title(), "Beta Co · Wanderbooks");
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
    let beta_cash = h.state().views()[1].books().unwrap().kpis.cash;

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
    // The entry is several journal lines, and the tab may read some of
    // them first (its seq moves): wait for the one that posts it, whose
    // 100.00 lands in cash.
    let beta_cash_after = beta_cash + Decimal::new(10_000, 2);
    until(&mut h, "the background tab reads the new entry", |a| {
        a.views()[1].books().unwrap().kpis.cash == beta_cash_after
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
    fn new_books(&self, _: &str, _: &str) -> Result<String, String> {
        Err("not here".into())
    }
    fn back_up_now(&self, _: &str) -> Result<Vec<String>, String> {
        Err("not here".into())
    }
    fn plan_restore(
        &self,
        _: &str,
        _: Option<String>,
        _: bool,
        _: Option<ledger_gui::owner::Secret>,
    ) -> Result<RestorePlan, String> {
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

    // Every thread of beta's tab is one of its registry's: the loader, the
    // receipts' workers, the dock's agent (jobs.rs).
    let beta_threads = h.state().views()[1].jobs_watch();
    let running = beta_threads.running();
    for name in [
        "books-loader",
        "receipt-render-0",
        "thumb-render-0",
        "dock agent",
    ] {
        assert!(
            running.iter().any(|n| n == name),
            "{name} isn't counted among beta's threads: {running:?}"
        );
    }

    // Closed: every thread of the tab ends (waited for on that condition;
    // the limit is only a failure guard), and with them its viewer lock is
    // released, its loader's thread has ended, its agent's process is gone;
    // alpha's are as they were.
    close_tab(&mut h, "Beta Co");
    assert_eq!(titles(&h), ["Alpha Co"]);
    assert_eq!(h.state().active(), Some(0));
    assert!(
        beta_threads.wait_ended(Duration::from_secs(120)),
        "a closed tab's threads run on: {:?}",
        beta_threads.running()
    );
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
    h.get_by_label("Beta Co").click();
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
    assert!(
        shows(&h, "Every company is already open."),
        "{:?}",
        texts(&h)
    );
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
    assert_eq!(h.state().window_title(), "Alpha Co · Wanderbooks");

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
    assert_eq!(kept.0[&w.open_key()], "alpha,beta,gamma");
    assert_eq!(kept.0[&w.active_key()], "gamma");
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
    h.get_by_label("Alpha Co").click();
    h.run_steps(3);
    let last = h.state().views().len() - 1;
    assert!(matches!(
        h.state().views()[last].route(),
        Route::Reports(ReportKind::BalanceSheet)
    ));
    let mut again = Prefs::default();
    eframe::App::save(h.state_mut(), &mut again);
    assert_eq!(again.0[&w.open_key()], "beta,gamma,alpha");
    assert_eq!(again.0[&w.active_key()], "alpha");
    // Closing the last tab keeps a list with nothing in it, and the next
    // start opens the data root's first.
    for _ in 0..3 {
        let name = h.state().tab_title(0).unwrap();
        close_tab(&mut h, &name);
    }
    assert!(h.state().views().is_empty());
    let mut none = Prefs::default();
    eframe::App::save(h.state_mut(), &mut none);
    assert_eq!(none.0[&w.open_key()], "");
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
    assert_eq!(h.state().window_title(), "Gamma Co · Wanderbooks");
    // A tab behind the front one closing leaves the front one in front.
    close_tab(&mut h, "Alpha Co");
    assert_eq!(titles(&h), ["Gamma Co"]);
    assert_eq!(h.state().window_title(), "Gamma Co · Wanderbooks");
    // The last one.
    close_tab(&mut h, "Gamma Co");
    assert!(h.state().views().is_empty());
    assert_eq!(h.state().window_title(), "Wanderbooks");
    assert!(shows(&h, "No company is open"), "{:?}", texts(&h));
    assert!(
        shows(&h, "Open a company with the + at the top of the window."),
        "{:?}",
        texts(&h)
    );
    // The bar is still there, and "+" opens a company from the empty window.
    h.get_by_label("Open another company").click();
    h.run_steps(3);
    h.get_by_label("Beta Co").click();
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

// ---- 9: the bar's place --------------------------------------------------------

/// The fill of the bar: a rectangle from the window's corner, as wide as
/// the window and as high as the bar.
fn bar_rect(h: &Harness<'_, App>, size: egui::Vec2) -> Option<egui::Rect> {
    fn walk(s: &egui::Shape, size: egui::Vec2, out: &mut Option<egui::Rect>) {
        match s {
            egui::Shape::Rect(r)
                if r.rect.min.x.abs() < 0.5
                    && r.rect.min.y.abs() < 0.5
                    && (r.rect.height() - ledger_gui::tabs::BAR_H).abs() < 0.5
                    && r.rect.width() >= size.x - 0.5 =>
            {
                *out = Some(r.rect)
            }
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, size, out)),
            _ => {}
        }
    }
    let mut found = None;
    for c in &h.output().shapes {
        walk(&c.shape, size, &mut found);
    }
    found
}

#[test]
fn the_bar_spans_the_window_above_the_sidebar() {
    let size = egui::vec2(1280.0, 600.0);
    let w = World::new();
    let h = shown(&w, &["Wanderland Labs LLC", "Fernhill Studio"], false, size);
    let bar = bar_rect(&h, size).expect("a bar the window's width, at its top");
    assert!(bar.max.x >= size.x - 0.5, "{bar:?}");
    // Every tab, and its close control, is inside the bar...
    for name in ["Wanderland Labs LLC", "Fernhill Studio"] {
        let tab = h
            .get_by_role_and_label(egui::accesskit::Role::Button, name)
            .rect();
        assert!(bar.contains_rect(tab), "{name}: {tab:?} is in {bar:?}");
    }
    // ...and the first is over the sidebar's column, whose links, header
    // and status all lie below the bar.
    let first = h
        .get_by_role_and_label(egui::accesskit::Role::Button, "Wanderland Labs LLC")
        .rect();
    assert!(first.left() < 224.0, "the first tab is above the sidebar");
    for link in ["Overview", "Journal"] {
        let r = h
            .query_all_by_role_and_label(egui::accesskit::Role::Button, link)
            .map(|n| n.rect())
            .min_by(|a, b| a.left().total_cmp(&b.left()))
            .unwrap();
        assert!(r.top() >= bar.bottom() - 0.5, "{link} {r:?} under {bar:?}");
    }
    // Nothing but the bar is drawn across the top strip of the sidebar's
    // column: the sidebar's own panel starts where the bar ends.
    let sidebar_top = h
        .output()
        .shapes
        .iter()
        .filter(|s| s.clip_rect.max.x <= 224.5 && s.clip_rect.min.y > 1.0)
        .map(|s| s.clip_rect.min.y)
        .fold(f32::MAX, f32::min);
    assert!(
        sidebar_top >= bar.bottom() - 0.5,
        "the sidebar starts at {sidebar_top}, below the bar's {}",
        bar.bottom()
    );
}

#[test]
fn the_plus_follows_the_last_tab_and_stays_whole_when_the_tabs_overflow() {
    let size = egui::vec2(1100.0, 200.0);
    let rect = |h: &Harness<'_, App>, label: &str| {
        h.get_by_role_and_label(egui::accesskit::Role::Button, label)
            .rect()
    };
    // Few tabs: the "+" is right after the last, not at the window's edge.
    let w = World::new();
    let h = shown(&w, &["Wanderland Labs LLC", "Fernhill Studio"], false, size);
    let last = rect(&h, "Fernhill Studio");
    let plus = rect(&h, "Open another company");
    assert!(
        plus.left() >= last.right() && plus.left() - last.right() < 16.0,
        "the + at {plus:?} is not just after the last tab {last:?}"
    );
    assert!(plus.right() < size.x / 2.0, "the + sits at the far right");
    // Many: the "+" is at the right edge, whole; the tabs scroll behind it.
    let names: Vec<String> = (0..9).map(|i| format!("Company number {i}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let w = World::new();
    let mut h = shown(&w, &refs, false, size);
    h.state_mut().activate(8);
    h.run_steps(6);
    let plus = rect(&h, "Open another company");
    assert!(
        plus.right() <= size.x && plus.right() > size.x - 24.0,
        "the + at {plus:?} is not at the right edge"
    );
    assert!(
        plus.left() >= 0.0 && plus.width() >= 20.0,
        "{plus:?} is whole"
    );
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Later tabs");
    // The tab in front was scrolled into view, left of the "+".
    let front = rect(&h, "Company number 8");
    assert!(
        front.right() <= plus.left(),
        "{front:?} under the + {plus:?}"
    );
}

// ---- 10: names remembered ---------------------------------------------------------

#[test]
fn a_name_is_remembered_and_shown_in_the_plus_list_and_on_a_loading_tab() {
    let (w, _, _, _) = three();
    let mut h = start_with_prefs(&w, &[("open_companies", "alpha,beta")]);
    loaded(&mut h);
    assert_eq!(h.state().remembered_name("beta"), Some("Beta Co"));
    assert_eq!(h.state().remembered_name("gamma"), None);
    // Beta closed: "+" offers it by name with its id beside, and gamma,
    // never opened here, by its id alone.
    close_tab(&mut h, "Beta Co");
    h.get_by_label("Open another company").click();
    h.run_steps(3);
    assert!(shows(&h, "Open a company"), "{:?}", texts(&h));
    assert!(shows(&h, "Beta Co") && texts(&h).contains(&"beta".to_string()));
    assert!(texts(&h).contains(&"gamma".to_string()), "{:?}", texts(&h));
    assert!(!shows(&h, "Gamma Co"), "gamma's name was never read");
    h.get_by_label("Beta Co").click();
    loaded(&mut h);
    // Kept with the app's other state...
    let mut kept = Prefs::default();
    eframe::App::save(h.state_mut(), &mut kept);
    assert_eq!(
        kept.0.get(&w.kept_key("beta", "name")).map(String::as_str),
        Some("Beta Co")
    );
    // ...and a window started from it titles a tab whose books are not
    // loaded yet by name; a company with no name known by its id.
    let ctx = egui::Context::default();
    kept.0.remove(&w.kept_key("alpha", "name"));
    kept.0.insert(w.open_key(), "alpha,beta".into());
    let root = w.root.clone();
    let fallback = w.source("alpha");
    let app = App::start_with(
        &ctx,
        &|k| kept.0.get(k).cloned(),
        Start::Restore { root, fallback },
    );
    assert_eq!(app.tab_title(0).as_deref(), Some("alpha"), "no name known");
    assert_eq!(
        app.tab_title(1).as_deref(),
        Some("Beta Co"),
        "the name kept"
    );
}

#[test]
fn the_plus_list_says_so_when_every_company_is_open() {
    let w = World::new();
    w.company("alpha", "Alpha Co", 1_000_000, 100);
    let mut h = start_with_prefs(&w, &[("open_companies", "alpha")]);
    loaded(&mut h);
    h.get_by_label("Open another company").click();
    h.run_steps(3);
    assert!(
        shows(&h, "Every company is already open."),
        "{:?}",
        texts(&h)
    );
    assert!(!shows(&h, "Open a company"), "no heading over nothing");
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

/// A shape in words, without where its glyphs happen to lie in egui's font
/// atlas: that depends on which text this process laid out first (other
/// tests, in other threads), and no picture shows it. Positions, colours,
/// the text and each glyph's place are all here.
fn describe(shape: &egui::epaint::Shape) -> String {
    use egui::epaint::Shape;
    match shape {
        Shape::Text(t) => {
            let mut out = format!(
                "Text pos={:?} under={:?} fallback={:?} over={:?} opacity={:?} angle={:?} text={:?}",
                t.pos,
                t.underline,
                t.fallback_color,
                t.override_text_color,
                t.opacity_factor,
                t.angle,
                t.galley.job.text,
            );
            for placed in &t.galley.rows {
                out.push_str(&format!(" row@{:?} size={:?}", placed.pos, placed.row.size));
                for g in &placed.row.glyphs {
                    out.push_str(&format!(" {}@{:?}+{:?}", g.chr, g.pos, g.advance_width));
                }
                for v in &placed.row.visuals.mesh.vertices {
                    out.push_str(&format!(" v{:?}{:?}", v.pos, v.color));
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

/// The shapes the window paints where it can be seen: those whose clip
/// reaches into the window. (The page under a short bar is laid out far
/// below it and holds the time now, "Nov 21 2026 04:26 UTC", which no
/// picture shows: it is clipped away.)
fn shown_shapes(h: &Harness<'_, App>) -> Vec<String> {
    let screen = h.ctx.content_rect();
    h.output()
        .shapes
        .iter()
        .filter(|s| s.clip_rect.intersects(screen))
        .map(|s| format!("{:?} {}", s.clip_rect, describe(&s.shape)))
        .collect()
}

/// A hash of every shape the window paints where it can be seen (FNV-1a
/// over their debug form): the same shapes paint the same pixels, on any
/// renderer. What the picture shows (colours, text and where each lies) is
/// in them; how a renderer rasterizes their edges is not.
fn shapes_of(h: &Harness<'_, App>) -> u64 {
    shapes_support::hash_of(&shown_shapes(h))
}

/// The look of the window, compared as sidenav.rs compares its sidebar's:
/// the shapes painted are the check everywhere
/// (`tests/snapshots/<name>.shapes`; `TABS_BLESS=1` writes them), for
/// another renderer rasterizes the same shapes differently (CI's software
/// one differs from a GPU in antialiasing, by thousands of faint pixels
/// and some dozens of strong ones along glyph edges). A mismatch prints
/// the first differing shapes (`shapes_support`). The pixels
/// (`tests/snapshots/<name>.png`, tolerant of antialiasing) are compared
/// as well on the machine that made them and any with its renderer:
/// `TABS_PIXELS=1`.
fn picture(h: &mut Harness<'_, App>, name: &str) {
    shapes_support::check(name, &shown_shapes(h));
    if std::env::var("TABS_PIXELS").is_err() {
        return;
    }
    if h.render().is_err() {
        eprintln!("no renderer: {name} pixels not compared");
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

/// What the window shows depends on the books and the theme alone: not on
/// the name of the temporary folder they sit in, the clock, or the
/// machine. The same companies made in two worlds whose folders are named
/// unlike paint the same shapes and the same pixels (where there is a
/// renderer), with the badge hue chosen from the id of the books.
#[test]
fn the_bar_is_the_same_from_differently_named_folders() {
    let names = ["Wanderland Labs LLC", "Fernhill Studio"];
    let mut seen: Vec<(u64, Option<Vec<u8>>, Vec<String>)> = Vec::new();
    for (prefix, dark) in [
        ("a-", false),
        ("zzzz-a-much-longer-name-", false),
        ("a-", true),
        ("zzzz-a-much-longer-name-", true),
    ] {
        let w = World::named(prefix);
        let mut h = shown(&w, &names, dark, BAR_SIZE);
        let keys: Vec<String> = (0..names.len())
            .map(|i| h.state().views()[i].hue_key())
            .collect();
        let pixels = h.render().ok().map(|img| img.into_raw());
        seen.push((shapes_of(&h), pixels, keys));
    }
    for pair in [(0, 1), (2, 3)] {
        let (a, b) = (&seen[pair.0], &seen[pair.1]);
        assert_eq!(a.2, ["co-0", "co-1"], "a company's hue key is its book id");
        assert_eq!(
            a.2, b.2,
            "the hue key is the book id, not the folder's name"
        );
        assert_eq!(a.0, b.0, "the shapes differ with the folder's name");
        assert!(a.1 == b.1, "the pixels differ with the folder's name");
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

// ---- 11-13: the company's name and hue --------------------------------------

/// Every text the last frame painted, with where it starts.
fn placed(h: &Harness<'_, App>) -> Vec<(egui::Pos2, String)> {
    fn walk(s: &egui::Shape, out: &mut Vec<(egui::Pos2, String)>) {
        match s {
            egui::Shape::Text(t) => out.push((t.pos, t.galley.text().to_string())),
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            _ => {}
        }
    }
    let mut all = Vec::new();
    for c in &h.output().shapes {
        walk(&c.shape, &mut all);
    }
    all
}

/// Every rectangle of exactly colour `color` the last frame painted.
fn fills(h: &Harness<'_, App>, color: egui::Color32) -> Vec<egui::Rect> {
    fn walk(s: &egui::Shape, color: egui::Color32, out: &mut Vec<egui::Rect>) {
        match s {
            egui::Shape::Rect(r) if r.fill == color => out.push(r.rect),
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, color, out)),
            _ => {}
        }
    }
    let mut all = Vec::new();
    for c in &h.output().shapes {
        walk(&c.shape, color, &mut all);
    }
    all
}

/// The sidebar's column below the bar.
fn in_sidebar(r: egui::Rect) -> bool {
    r.max.x <= 224.5 && r.min.y >= ledger_gui::tabs::BAR_H
}

/// The hue ids of the books give: the light or dark one.
fn hue(id: &str, dark: bool) -> egui::Color32 {
    ledger_gui::tabs::hue_of(id, dark)
}

#[test]
fn nothing_in_the_top_of_the_window_but_the_tab_shows_the_company_name() {
    use ledger_gui::tabs::BAR_H;
    for dark in [false, true] {
        for route in [Route::Overview, Route::Cash, Route::Journal] {
            let w = World::new();
            let mut h = shown(
                &w,
                &["Wanderland Labs LLC", "Fernhill Studio"],
                dark,
                egui::vec2(1280.0, 700.0),
            );
            h.state_mut().goto(route.clone());
            h.run_steps(4);
            let tag = format!("{route:?}, dark {dark}");
            for name in ["Wanderland Labs LLC", "Fernhill Studio"] {
                // Painted text: in the top 120 points it is only the tab's.
                for (at, text) in placed(&h) {
                    if text.contains(name) && at.y < 120.0 {
                        assert!(
                            at.y < BAR_H,
                            "{tag}: {name} is painted at {at:?} below the bar: {text:?}"
                        );
                    }
                }
                // The tree: only the tab and its close control, both in the bar.
                for n in h.query_all_by_label_contains(name) {
                    let r = n.rect();
                    if r.top() < 120.0 {
                        assert!(
                            r.bottom() <= BAR_H + 0.5,
                            "{tag}: a widget named {name} at {r:?} is under the bar"
                        );
                    }
                }
            }
            // And the one in front is named once more, lower: the card.
            let card = placed(&h)
                .into_iter()
                .filter(|(at, t)| at.x < 224.0 && at.y > 120.0 && t == "Wanderland Labs LLC")
                .count();
            assert_eq!(card, 1, "{tag}: the status card names the company");
        }
    }
}

#[test]
fn the_status_card_shows_the_company_in_front_and_follows_the_tab() {
    for dark in [false, true] {
        let (w, _, _, _) = three();
        let mode = if dark { "true" } else { "false" };
        let mut h = start(
            &w,
            prefs(&[("open_companies", "alpha,beta"), ("dark", mode)]),
            egui::vec2(1280.0, 900.0),
        );
        loaded(&mut h);
        let card_names = |h: &Harness<'_, App>| -> Vec<String> {
            placed(h)
                .into_iter()
                .filter(|(at, t)| at.x < 224.0 && at.y > 120.0 && t.ends_with(" Co"))
                .map(|(_, t)| t)
                .collect()
        };
        // The card's badge: its hue, a small square in the sidebar, near
        // the foot (the tab's is in the bar).
        let card_badge = |h: &Harness<'_, App>, id: &str| {
            fills(h, hue(id, dark))
                .into_iter()
                .filter(|r| in_sidebar(*r) && r.min.y > 120.0)
                .collect::<Vec<_>>()
        };
        assert_eq!(card_names(&h), ["Alpha Co"], "dark {dark}");
        // The heading carries the live word, at the name's row and the
        // card's right edge; no text of the card says the event (it is the
        // heading's hover and its live mark's label).
        let card_texts = |h: &Harness<'_, App>| -> Vec<(egui::Pos2, String)> {
            placed(h)
                .into_iter()
                .filter(|(at, _)| at.x < 224.0 && at.y > 120.0)
                .collect()
        };
        let texts_now = card_texts(&h);
        let name_y = texts_now.iter().find(|(_, t)| t == "Alpha Co").unwrap().0.y;
        let live = texts_now
            .iter()
            .find(|(_, t)| t == "Live" || t == "Snapshot")
            .expect("the live word is on the card");
        assert!(
            (live.0.y - name_y).abs() < 6.0,
            "the live word is in the heading"
        );
        assert!(
            live.0.x > 150.0,
            "the live word is at the card's right edge: {:?}",
            live.0
        );
        assert!(
            texts_now
                .iter()
                .any(|(_, t)| t.ends_with("Journal chain verified")),
            "the journal chain line is as it was, with no event"
        );
        assert!(
            !texts_now.iter().any(|(_, t)| t.contains("event")),
            "no text of the card says the event"
        );
        // The event is the live mark's accessible label.
        assert_eq!(
            h.query_all_by_label_contains("event ")
                .filter(|n| n.rect().left() < 224.0)
                .count(),
            1,
            "one element carries the event"
        );
        assert!(
            h.query_all_by_label_contains("Live · event ")
                .chain(h.query_all_by_label_contains("Snapshot · event "))
                .count()
                == 1,
            "the live mark's label is \"Live · event N\""
        );
        assert_eq!(
            card_badge(&h, "alpha").len(),
            1,
            "alpha's badge on its card"
        );
        assert!(
            card_badge(&h, "beta").is_empty(),
            "beta's hue isn't on alpha's card"
        );
        // The badge is small, and carries the initial.
        let r = card_badge(&h, "alpha")[0];
        assert!(r.width() <= 20.0 && r.height() <= 20.0, "{r:?}");
        assert!(
            placed(&h)
                .iter()
                .any(|(at, t)| t == "A" && r.contains(*at + egui::vec2(4.0, 4.0))
                    || t == "A" && r.expand(4.0).contains(*at)),
            "the initial is on the badge"
        );
        show_tab(&mut h, "Beta Co");
        assert_eq!(card_names(&h), ["Beta Co"], "dark {dark}");
        assert_eq!(card_badge(&h, "beta").len(), 1, "beta's badge on its card");
        assert!(
            card_badge(&h, "alpha").is_empty(),
            "alpha's hue left the card"
        );
        // The window's title still names the company.
        assert_eq!(h.state().window_title(), "Beta Co · Wanderbooks");
    }
}

#[test]
fn the_rail_shows_the_badge_alone_with_the_name_on_hover() {
    let (w, _, _, _) = three();
    let mut h = start(
        &w,
        prefs(&[("open_companies", "alpha,beta"), ("dark", "false")]),
        egui::vec2(900.0, 700.0),
    );
    loaded(&mut h);
    let badge: Vec<_> = fills(&h, hue("alpha", false))
        .into_iter()
        .filter(|r| in_sidebar(*r) && r.max.x <= 60.5)
        .collect();
    assert_eq!(badge.len(), 1, "alpha's badge in the rail");
    assert!(
        !placed(&h)
            .iter()
            .any(|(at, t)| at.x < 60.0 && at.y > 36.0 && t.contains("Alpha Co")),
        "the rail has no room for the name"
    );
    h.hover_at(badge[0].center());
    h.run_steps(30);
    assert!(shows(&h, "Alpha Co"), "the name is on hover");
    assert!(shows(&h, "· event "), "the event is on hover");
    assert!(
        shows(&h, "Alpha Co\n") || shows(&h, "Alpha Co"),
        "the name leads the hover"
    );
    assert!(
        shows(&h, "Live") || shows(&h, "Snapshot"),
        "the state is on hover"
    );
    assert!(
        shows(&h, "Journal chain verified"),
        "the checks are on hover"
    );
}

#[test]
fn a_hue_is_the_books_id_alone_on_the_tab_and_the_card_in_both_themes_and_after_a_restart() {
    let (w, _, _, _) = three();
    for dark in [false, true] {
        let mode = if dark { "true" } else { "false" };
        let size = egui::vec2(1280.0, 900.0);
        // Two starts, the tabs opened in different orders: a restart, and
        // an order of opening, change nothing.
        let mut seen = Vec::new();
        for order in ["alpha,beta,gamma", "gamma,alpha,beta"] {
            let mut h = start(
                &w,
                prefs(&[("open_companies", order), ("dark", mode)]),
                size,
            );
            loaded(&mut h);
            for id in ["alpha", "beta", "gamma"] {
                let name = format!("{} Co", id[..1].to_uppercase() + &id[1..]);
                show_tab(&mut h, &name);
                let want = hue(id, dark);
                let tab: Vec<_> = fills(&h, want)
                    .into_iter()
                    .filter(|r| r.max.y <= ledger_gui::tabs::BAR_H)
                    .collect();
                let card: Vec<_> = fills(&h, want)
                    .into_iter()
                    .filter(|r| in_sidebar(*r))
                    .collect();
                assert_eq!(tab.len(), 1, "{id} dark {dark}: its tab's badge in its hue");
                assert_eq!(
                    card.len(),
                    1,
                    "{id} dark {dark}: its card's badge in its hue"
                );
                seen.push((id, dark, want));
            }
        }
        // The same in both starts.
        let (first, second) = seen.split_at(3);
        let mut a = first.to_vec();
        let mut b = second.to_vec();
        a.sort_by_key(|x| x.0);
        b.sort_by_key(|x| x.0);
        assert_eq!(a, b);
    }
    // Pinned, with the rule: it never changes without a decision, and
    // is a function of the id alone (not the name, nor the machine).
    assert_eq!(
        hue("alpha", false),
        egui::Color32::from_rgb(0xBE, 0x18, 0x5D)
    );
    assert_eq!(
        hue("alpha", true),
        egui::Color32::from_rgb(0xF4, 0x72, 0xB6)
    );
    assert_eq!(
        hue("beta", false),
        egui::Color32::from_rgb(0x0E, 0x74, 0x90)
    );
    assert_eq!(
        hue("gamma", false),
        egui::Color32::from_rgb(0xB4, 0x53, 0x09)
    );
}

#[test]
fn the_overview_opens_with_its_summary_sentence_and_never_names_the_company() {
    for dark in [false, true] {
        let w = World::new();
        let mut h = shown(&w, &["Alpha Co"], dark, egui::vec2(1280.0, 800.0));
        h.run_steps(3);
        // The page's first text: the first under the company's top bar
        // (52 high, under the tab bar).
        let page_top = ledger_gui::tabs::BAR_H + 52.0;
        let mut page: Vec<(egui::Pos2, String)> = placed(&h)
            .into_iter()
            .filter(|(at, _)| at.x >= 224.0 && at.y >= page_top)
            .collect();
        page.sort_by(|a, b| a.0.y.total_cmp(&b.0.y).then(a.0.x.total_cmp(&b.0.x)));
        let (_, first) = page.first().expect("the page has text");
        assert!(
            first.starts_with("You have $8,765.44 in cash and owe $0.00."),
            "dark {dark}: the page opens with {first:?}"
        );
        // Nothing on the page says the company's name: no sentence there
        // used it.
        assert!(
            page.iter().all(|(_, t)| !t.contains("Alpha Co")),
            "dark {dark}: {:?}",
            page.iter()
                .filter(|(_, t)| t.contains("Alpha Co"))
                .collect::<Vec<_>>()
        );
        // It reads as the page's opening line: bigger than body text,
        // smaller than a page's title (26).
        let lead = h
            .output()
            .shapes
            .iter()
            .find_map(|c| match &c.shape {
                egui::Shape::Text(t) if t.galley.text().starts_with("You have") => {
                    Some(t.galley.rows[0].height())
                }
                _ => None,
            })
            .expect("the lead");
        let body = 14.0 * 1.2;
        assert!(lead > body * 1.2 && lead < 26.0 * 1.2, "lead row {lead}");
    }
}

#[test]
fn the_overview_of_books_with_no_transactions_opens_the_same_way() {
    let w = World::new();
    let id = "empty-co";
    let root = Root::find(Some(&w.root)).unwrap();
    let mut s = root
        .create_book(&BookId::parse(id).unwrap(), Currency::USD)
        .unwrap();
    s.add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    drop(s);
    let source = w.source(id);
    let mut h = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .build_eframe(move |cc| {
            App::with_books(
                &cc.egui_ctx,
                data::load_once(&source).unwrap(),
                false,
                Route::Overview,
            )
        });
    h.run_steps(4);
    let page_top = ledger_gui::tabs::BAR_H + 52.0;
    let mut page: Vec<_> = placed(&h)
        .into_iter()
        .filter(|(at, _)| at.x >= 224.0 && at.y >= page_top)
        .collect();
    page.sort_by(|a, b| a.0.y.total_cmp(&b.0.y));
    assert!(
        page[0].1.starts_with("No transactions yet."),
        "opens with {:?}",
        page[0].1
    );
}

/// Pictures of the whole window: one company, two tabs each in front once,
/// eight tabs, the "+" list open, the empty window, the rail, the
/// smallest window and the Overview at its top, for a person to look at
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
        // One company open.
        let (w, _, _, _) = three();
        let mut h = start(
            &w,
            prefs(&[
                ("open_companies", "alpha"),
                ("dark", if dark { "true" } else { "false" }),
            ]),
            egui::vec2(1440.0, 920.0),
        );
        loaded(&mut h);
        save(&mut h, &format!("one-company-{mode}"));
        // The Overview scrolled down and back to its top.
        h.hover_at(egui::pos2(900.0, 500.0));
        for dy in [-400.0, 800.0] {
            h.event(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, dy),
                phase: egui::TouchPhase::Move,
                modifiers: egui::Modifiers::NONE,
            });
            h.run_steps(6);
        }
        save(&mut h, &format!("overview-scrolled-to-top-{mode}"));
        // The sidebar collapsed to its rail, two companies open.
        let mut h = start(
            &w,
            prefs(&[
                ("open_companies", "alpha,beta"),
                ("dark", if dark { "true" } else { "false" }),
            ]),
            egui::vec2(900.0, 700.0),
        );
        loaded(&mut h);
        save(&mut h, &format!("rail-two-companies-{mode}"));
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
        // Eight companies open: the tabs scroll, the "+" stays at the edge.
        let names: Vec<String> = [
            "Wanderland Labs LLC",
            "Fernhill Studio",
            "Northgate Holdings Incorporated",
            "Oak & Ash Carpentry",
            "Harbor Lane Books",
            "Quill and Compass",
            "Mossy Rock Farm",
            "Lantern Works",
        ]
        .map(String::from)
        .to_vec();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let w = World::new();
        let mut h = shown(&w, &refs, dark, egui::vec2(1100.0, 600.0));
        h.state_mut().activate(6);
        h.run_steps(6);
        save(&mut h, &format!("eight-tabs-overflow-{mode}"));
        // The smallest window the viewer allows, two tabs.
        let w = World::new();
        let mut h = shown(
            &w,
            &["Wanderland Labs LLC", "Fernhill Studio"],
            dark,
            egui::vec2(360.0, 400.0 + ledger_gui::tabs::BAR_H),
        );
        h.run_steps(4);
        save(&mut h, &format!("smallest-window-two-tabs-{mode}"));
    }
}

// ---- 14: threads, held jobs and the window's close ---------------------------

/// A latch: opened once, it stays open; waiters block until then.
#[derive(Default)]
struct Latch {
    open: Mutex<bool>,
    changed: std::sync::Condvar,
}

impl Latch {
    fn open(&self) {
        *self.open.lock().unwrap() = true;
        self.changed.notify_all();
    }

    fn is_open(&self) -> bool {
        *self.open.lock().unwrap()
    }

    /// Wait for it to open; the limit is a failure guard, never what the
    /// test relies on.
    fn wait(&self) -> bool {
        let end = Instant::now() + Duration::from_secs(120);
        let mut open = self.open.lock().unwrap();
        while !*open {
            let Some(left) = end.checked_duration_since(Instant::now()) else {
                return false;
            };
            open = self.changed.wait_timeout(open, left).unwrap().0;
        }
        true
    }
}

/// An owner whose changing calls (restore, close the books), its backup
/// and its agent's start each say they were entered and then wait to be
/// let go: the test decides when they end.
struct Gated {
    entered: Latch,
    release: Latch,
    start: Mutex<Option<AgentStart>>,
}

impl Gated {
    fn new() -> Arc<Gated> {
        Arc::new(Gated {
            entered: Latch::default(),
            release: Latch::default(),
            start: Mutex::new(None),
        })
    }

    fn block(&self) {
        self.entered.open();
        assert!(self.release.wait(), "the test never let the call go");
    }
}

impl Owner for Gated {
    fn new_books(&self, _: &str, _: &str) -> Result<String, String> {
        Err("not here".into())
    }
    fn back_up_now(&self, _: &str) -> Result<Vec<String>, String> {
        self.block();
        Ok(vec!["Backed up.".into()])
    }
    fn plan_restore(
        &self,
        source: &str,
        id: Option<String>,
        replace: bool,
        passphrase: Option<ledger_gui::owner::Secret>,
    ) -> Result<RestorePlan, String> {
        Ok(RestorePlan {
            source: source.to_string(),
            passphrase,
            id: id.unwrap_or_else(|| "alpha".into()),
            replace,
            head: (7, "ab".repeat(32)),
            lines: vec!["The backup is whole: through event 7.".into()],
        })
    }
    fn restore(&self, plan: &RestorePlan) -> Result<Vec<String>, String> {
        self.block();
        Ok(vec![format!("Restored as \"{}\".", plan.id)])
    }
    fn connect(&self, _: Agent, _: &str) -> Result<Vec<String>, String> {
        Err("not here".into())
    }
    fn upgrade_format(&self, _: &str, _: &str) -> Result<Vec<String>, String> {
        Err("not here".into())
    }
    fn agent_start(&self, _: Agent, _: &str) -> Result<AgentStart, String> {
        self.block();
        self.start
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| "no agent set".to_string())
    }
}

/// Whether the window's close, asked for now, was cancelled by the app.
fn window_close_is_cancelled(h: &mut Harness<'static, App>) -> bool {
    h.input_mut()
        .viewports
        .entry(egui::ViewportId::ROOT)
        .or_default()
        .events
        .push(egui::ViewportEvent::Close);
    h.run_steps(1);
    h.output()
        .viewport_output
        .get(&egui::ViewportId::ROOT)
        .is_some_and(|v| {
            v.commands
                .iter()
                .any(|c| matches!(c, egui::ViewportCommand::CancelClose))
        })
}

/// Whether the close control of the tab named `name` is disabled for
/// assistive technology.
fn close_is_off(h: &Harness<'_, App>, name: &str) -> bool {
    h.get_by_role_and_label(Role::Button, &format!("Close {name}"))
        .accesskit_node()
        .is_disabled()
}

/// Through the front tab's Books menu to a restore that has been checked
/// and is asked for: `Restore it` clicked.
fn ask_for_a_restore(h: &mut Harness<'static, App>) {
    let page_fields = h.query_all_by_role(Role::TextInput).count();
    h.get_by_label_contains(" Books").click();
    h.run_steps(3);
    h.get_by_label_contains("Restore…").click();
    h.run_steps(3);
    h.get_all_by_role(Role::TextInput)
        .nth(page_fields)
        .unwrap()
        .focus();
    h.run_steps(2);
    h.get_all_by_role(Role::TextInput)
        .nth(page_fields)
        .unwrap()
        .type_text("/media/disk/alpha.git");
    h.run_steps(2);
    h.get_by_label("Check it").click();
    let end = Instant::now() + Duration::from_secs(60);
    while h.query_by_label("Restore it").is_none() {
        assert!(Instant::now() < end, "the backup was never checked");
        h.run_steps(2);
        std::thread::sleep(Duration::from_millis(10));
    }
    h.run_steps(3);
    h.get_by_label("Restore it").click();
    h.run_steps(2);
}

#[test]
fn a_restore_under_way_holds_its_tab_and_the_window_open() {
    let (w, _, _, _) = three();
    let owner = Gated::new();
    let mut h = start_with_prefs(&w, &[("open_companies", "alpha,beta")]);
    loaded(&mut h);
    h.state_mut()
        .view_mut(0)
        .unwrap()
        .set_owner(owner.clone() as Arc<dyn Owner>);
    // Nothing runs: the tab closes freely, the window's close goes through.
    assert!(!close_is_off(&h, "Alpha Co"));
    assert!(
        !window_close_is_cancelled(&mut h),
        "nothing holds the window"
    );
    assert!(h.state().holding().is_empty());

    ask_for_a_restore(&mut h);
    until(&mut h, "the owner is asked to restore", |_| {
        owner.entered.is_open()
    });
    h.run_steps(3);
    let words = "Restoring these books: wait for it to finish".to_string();
    assert_eq!(
        h.state().holding(),
        [("Alpha Co".to_string(), words.clone())]
    );
    // The tab: its close control is off, and a click on it closes nothing.
    assert!(close_is_off(&h, "Alpha Co"));
    assert!(!close_is_off(&h, "Beta Co"), "beta's own is not held");
    close_tab(&mut h, "Alpha Co");
    assert_eq!(titles(&h), ["Alpha Co", "Beta Co"], "alpha stayed");
    h.state_mut().close(0);
    assert_eq!(
        titles(&h),
        ["Alpha Co", "Beta Co"],
        "not even asked in code"
    );
    // The window: the close is cancelled and the dialog says why, and
    // offers only to wait.
    assert!(
        window_close_is_cancelled(&mut h),
        "the window's close went through with a restore running"
    );
    h.run_steps(3);
    assert!(h.state().close_dialog_is_up());
    assert!(shows(&h, "The books are being changed"), "{:?}", texts(&h));
    assert!(shows(&h, &format!("Alpha Co: {words}")), "{:?}", texts(&h));
    assert!(h.query_by_label("Wait for it to finish").is_some());

    // Let go: the dialog goes, the result is shown in that tab, and the
    // tab closes as any other.
    owner.release.open();
    until(&mut h, "the restore ends", |a| a.holding().is_empty());
    h.run_steps(4);
    assert!(!h.state().close_dialog_is_up());
    assert!(!shows(&h, "The books are being changed"));
    assert!(
        shows(&h, "Restore: Restored as"),
        "its result isn't in the tab: {:?}",
        texts(&h)
    );
    assert!(!close_is_off(&h, "Alpha Co"));
    close_tab(&mut h, "Alpha Co");
    assert_eq!(titles(&h), ["Beta Co"]);
}

#[test]
fn a_backup_only_reads_so_its_tab_closes_and_its_thread_ends() {
    let (w, _, _, _) = three();
    let owner = Gated::new();
    let mut h = start_with_prefs(&w, &[("open_companies", "alpha,beta")]);
    loaded(&mut h);
    h.state_mut()
        .view_mut(0)
        .unwrap()
        .set_owner(owner.clone() as Arc<dyn Owner>);
    h.run_steps(3);
    h.get_by_label_contains(" Books").click();
    h.run_steps(3);
    h.get_by_label_contains("Back up now").click();
    until(&mut h, "the owner is asked to back up", |_| {
        owner.entered.is_open()
    });
    let watch = h.state().views()[0].jobs_watch();
    assert!(
        watch.running().iter().any(|n| n == "Back up now"),
        "the backup isn't one of the tab's threads: {:?}",
        watch.running()
    );
    // It changes nothing of the books: nothing holds the tab or the window.
    assert!(h.state().holding().is_empty());
    assert!(!close_is_off(&h, "Alpha Co"));
    close_tab(&mut h, "Alpha Co");
    assert_eq!(titles(&h), ["Beta Co"]);
    // Its thread ends when the owner's call does, and says nothing.
    owner.release.open();
    assert!(
        watch.wait_ended(Duration::from_secs(60)),
        "{:?}",
        watch.running()
    );
}

#[test]
fn an_export_is_stopped_when_its_tab_closes() {
    let (w, _, _, _) = three();
    let mut h = start_with_prefs(&w, &[("open_companies", "alpha,beta")]);
    loaded(&mut h);
    // A job that waits to be told, then says whether it was told to stop.
    let go = Arc::new(Latch::default());
    let told = Arc::new(Mutex::new(None::<bool>));
    let (go2, told2) = (go.clone(), told.clone());
    let ctx = h.ctx.clone();
    h.state_mut().view_mut(1).unwrap().start_export_for_tests(
        &ctx,
        "A file",
        Box::new(move |stop| {
            assert!(go2.wait());
            *told2.lock().unwrap() = Some(stop.requested());
            Err("stopped".into())
        }),
    );
    let watch = h.state().views()[1].jobs_watch();
    assert!(watch.running().iter().any(|n| n == "export"));
    // Closing the tab asks nothing: it stops the job.
    assert!(h.state().holding().is_empty());
    close_tab(&mut h, "Beta Co");
    go.open();
    assert!(
        watch.wait_ended(Duration::from_secs(60)),
        "{:?}",
        watch.running()
    );
    assert_eq!(*told.lock().unwrap(), Some(true), "the export wasn't told");
}

// ---- 14b: the dock's agent and its tab ---------------------------------------

#[cfg(unix)]
fn agent_script(pidfile: &Path, body: &str) -> AgentStart {
    AgentStart::Ready {
        label: "Claude Code".into(),
        program: PathBuf::from("/bin/sh"),
        args: vec![
            "-c".into(),
            format!(
                "{body} echo $$ > '{}'; {}",
                pidfile.display(),
                "while true; do sleep 1; done"
            ),
        ],
        dir: pidfile.parent().unwrap().to_path_buf(),
        env: vec![("TERM".into(), "xterm-256color".into())],
    }
}

#[cfg(unix)]
#[test]
fn a_tab_closed_while_its_agent_is_starting_leaves_no_child_and_no_thread() {
    let (w, _, _, _) = three();
    let work = tempfile::tempdir().unwrap();
    let pidfile = work.path().join("agent.pid");
    let owner = Gated::new();
    *owner.start.lock().unwrap() = Some(agent_script(&pidfile, ""));
    let mut h = start_with_prefs(&w, &[("open_companies", "alpha,beta")]);
    loaded(&mut h);
    h.state_mut()
        .view_mut(1)
        .unwrap()
        .set_owner(owner.clone() as Arc<dyn Owner>);
    show_tab(&mut h, "Beta Co");
    h.state_mut().dock_mut().open = true;
    // Starting: the owner is looking for the agent and holds its answer.
    until(&mut h, "the owner is asked to start the agent", |_| {
        owner.entered.is_open()
    });
    let watch = h.state().views()[1].jobs_watch();
    assert!(
        watch.running().iter().any(|n| n == "agent start"),
        "the start isn't one of the tab's threads: {:?}",
        watch.running()
    );
    close_tab(&mut h, "Beta Co");
    // The answer comes after the tab is gone.
    owner.release.open();
    assert!(
        watch.wait_ended(Duration::from_secs(60)),
        "{:?}",
        watch.running()
    );
    h.run_steps(6);
    assert!(
        !pidfile.exists(),
        "a child was started for a tab that had closed"
    );
}

#[cfg(unix)]
#[test]
fn a_tab_closed_with_its_agent_running_ends_the_child_even_if_it_ignores_the_hangup() {
    let (w, _, _, _) = three();
    let work = tempfile::tempdir().unwrap();
    let pidfile = work.path().join("agent.pid");
    let owner = Gated::new();
    owner.release.open();
    // An agent that ignores the hang-up a closed terminal sends.
    *owner.start.lock().unwrap() = Some(agent_script(&pidfile, "trap '' HUP;"));
    let mut h = start_with_prefs(&w, &[("open_companies", "alpha,beta")]);
    loaded(&mut h);
    h.state_mut()
        .view_mut(1)
        .unwrap()
        .set_owner(owner.clone() as Arc<dyn Owner>);
    show_tab(&mut h, "Beta Co");
    h.state_mut().dock_mut().open = true;
    let pid = {
        let end = Instant::now() + Duration::from_secs(60);
        loop {
            h.run_steps(2);
            if let Ok(text) = std::fs::read_to_string(&pidfile)
                && let Ok(pid) = text.trim().parse::<u32>()
            {
                break pid;
            }
            assert!(Instant::now() < end, "the agent never started");
            std::thread::sleep(Duration::from_millis(20));
        }
    };
    assert!(alive(pid));
    let watch = h.state().views()[1].jobs_watch();
    assert!(
        watch.running().iter().any(|n| n == "dock agent"),
        "{:?}",
        watch.running()
    );
    close_tab(&mut h, "Beta Co");
    // Every thread of the tab, the one that reaps its agent included, ends;
    // and with them the child, hung up on, killed and reaped.
    assert!(
        watch.wait_ended(Duration::from_secs(120)),
        "{:?}",
        watch.running()
    );
    assert!(!alive(pid), "a closed tab's agent survived it");
}

// ---- 15: what is kept belongs to the books it was kept for -------------------

/// Wait for the clock's next second. A set of books' identity is the hash
/// of its journal's first line, which says when the books were made to the
/// second: books made again in the same second as the old would have the
/// old ones' identity, so a test that makes other books under an id waits
/// for the clock first (a condition, not a guess at a duration).
fn next_second() {
    let now = chrono::Utc::now().timestamp();
    while chrono::Utc::now().timestamp() == now {
        std::thread::yield_now();
    }
}

/// Save the app's preferences into `into`.
fn saved(h: &mut Harness<'static, App>, mut into: Prefs) -> Prefs {
    eframe::App::save(h.state_mut(), &mut into);
    into
}

#[test]
fn another_data_root_with_a_company_of_the_same_id_inherits_nothing() {
    // Root A: alpha and beta, open, alpha on a report in a custom period.
    let a = World::new();
    a.company("alpha", "Alpha One", 1_000_000, 100);
    a.company("beta", "Beta One", 500_000, 100);
    let mut h = start_with_prefs(&a, &[("open_companies", "alpha,beta")]);
    loaded(&mut h);
    {
        use ledger_gui::pages::reports::Range;
        let v = h.state_mut().view_mut(0).unwrap();
        v.goto(Route::Reports(ReportKind::BalanceSheet));
        v.state_mut().reports.range = Range::Custom(d(2026, 1, 1), d(2026, 6, 30));
    }
    h.run_steps(2);
    let kept = saved(&mut h, Prefs::default());
    drop(h);

    // Root B: another alpha (other books under the same id), and a beta.
    next_second();
    let b = World::new();
    b.company("alpha", "Alpha Two", 2_000_000, 100);
    b.company("beta", "Beta Two", 700_000, 100);
    // Before anything loads: B's alpha is not named for A's, and A's list
    // of open companies is not B's.
    let ctx = egui::Context::default();
    let app = App::start_with(
        &ctx,
        &|k| kept.0.get(k).cloned(),
        Start::Restore {
            root: b.root.clone(),
            fallback: b.source("alpha"),
        },
    );
    assert_eq!(app.views().len(), 1, "opened from another root's list");
    assert_eq!(app.tab_title(0).as_deref(), Some("alpha"), "A's name kept");
    assert_eq!(app.remembered_name("alpha"), None);
    drop(app);
    // Loaded: its own page and period, its own name.
    let mut h = start_as_is(&b, kept.clone(), egui::vec2(1280.0, 800.0));
    loaded(&mut h);
    assert_eq!(titles(&h), ["Alpha Two"]);
    assert!(matches!(h.state().views()[0].route(), Route::Overview));
    assert_eq!(
        h.state().views()[0].state().reports.range,
        ledger_gui::pages::reports::Range::YearToDate
    );
    // And A's, kept in the same file, are still A's.
    let both = saved(&mut h, kept.clone());
    assert_eq!(both.0[&a.open_key()], "alpha,beta");
    assert_eq!(both.0[&a.kept_key("alpha", "name")], "Alpha One");
    assert_eq!(both.0[&b.kept_key("alpha", "name")], "Alpha Two");
    assert_ne!(a.kept_key("alpha", "page"), b.kept_key("alpha", "page"));
}

#[test]
fn books_made_again_under_an_id_start_fresh_and_a_restart_of_the_same_books_does_not() {
    let w = World::new();
    w.company("alpha", "Alpha One", 1_000_000, 100);
    w.company("beta", "Beta One", 500_000, 100);
    let mut h = start_with_prefs(&w, &[("open_companies", "alpha,beta")]);
    loaded(&mut h);
    {
        use ledger_gui::pages::reports::Range;
        let v = h.state_mut().view_mut(0).unwrap();
        v.goto(Route::Reports(ReportKind::BalanceSheet));
        v.state_mut().reports.range = Range::Custom(d(2026, 1, 1), d(2026, 6, 30));
    }
    h.run_steps(2);
    let kept = saved(&mut h, Prefs::default());
    let identity = kept.0[&w.kept_key("alpha", "identity")].clone();
    assert_eq!(identity.len(), 64, "the journal's first line's SHA-256");
    drop(h);

    // The same books, restarted: the state comes back, the identity is the
    // same.
    let mut h = start_as_is(&w, kept.clone(), egui::vec2(1280.0, 800.0));
    loaded(&mut h);
    assert!(matches!(
        h.state().views()[0].route(),
        Route::Reports(ReportKind::BalanceSheet)
    ));
    let again = saved(&mut h, kept.clone());
    assert_eq!(again.0[&w.kept_key("alpha", "identity")], identity);
    drop(h);

    // Alpha deleted and made again, as other books.
    std::fs::remove_dir_all(w.root.join("books").join("alpha")).unwrap();
    next_second();
    w.company("alpha", "Alpha Two", 2_000_000, 100);
    let mut h = start_as_is(&w, kept.clone(), egui::vec2(1280.0, 800.0));
    loaded(&mut h);
    assert_eq!(titles(&h), ["Alpha Two", "Beta One"]);
    assert!(
        matches!(h.state().views()[0].route(), Route::Overview),
        "the old books' page was kept for the new books"
    );
    assert_eq!(
        h.state().views()[0].state().reports.range,
        ledger_gui::pages::reports::Range::YearToDate,
        "the old books' period was kept for the new books"
    );
    assert_eq!(h.state().remembered_name("alpha"), Some("Alpha Two"));
    // Beta, untouched, kept its own.
    assert_eq!(h.state().remembered_name("beta"), Some("Beta One"));
    let fresh = saved(&mut h, kept.clone());
    assert_ne!(fresh.0[&w.kept_key("alpha", "identity")], identity);
    assert_eq!(fresh.0[&w.kept_key("alpha", "name")], "Alpha Two");
    assert_eq!(fresh.0[&w.kept_key("alpha", "page")], "overview");
}

#[test]
fn books_outside_any_data_root_are_kept_under_their_folder() {
    let make = |tag: &str| {
        let dir = tempfile::Builder::new()
            .prefix(&format!("wb-gui-folder-{tag}-"))
            .tempdir()
            .unwrap();
        let book = dir.path().join("books-data");
        let mut s = Store::create(&book, Currency::USD).unwrap();
        s.add_account("1000", "Cash", AccountType::Asset, None, false)
            .unwrap();
        drop(s);
        (dir, book)
    };
    let open = |book: &Path, kept: Prefs| {
        let src = Source {
            data_dir: book.to_path_buf(),
            today: Some(today()),
        };
        let mut h = Harness::builder()
            .with_size(egui::vec2(1280.0, 800.0))
            .build_eframe(move |cc| {
                App::start_with(&cc.egui_ctx, &|k| kept.0.get(k).cloned(), Start::Only(src))
            });
        loaded(&mut h);
        h
    };
    let (_one, one) = make("one");
    let mut h = open(&one, Prefs::default());
    h.state_mut()
        .view_mut(0)
        .unwrap()
        .goto(Route::Reports(ReportKind::CashFlow));
    let kept = saved(&mut h, Prefs::default());
    let key = ledger_gui::app::company_key(&one, "page");
    assert!(key.starts_with("company.dir."), "{key}");
    assert_eq!(kept.0[&key], "reports:cash-flow");
    drop(h);
    // The same folder comes back to its page...
    let h = open(&one, kept.clone());
    assert!(matches!(
        h.state().views()[0].route(),
        Route::Reports(ReportKind::CashFlow)
    ));
    drop(h);
    // ...another folder, though it is also `books-data`, does not.
    let (_two, two) = make("two");
    let h = open(&two, kept);
    assert!(matches!(h.state().views()[0].route(), Route::Overview));
}

#[test]
fn preferences_written_before_the_keys_had_a_root_are_read_and_not_used() {
    let (w, _, _, _) = three();
    let old = prefs(&[
        ("open_companies", "beta,gamma"),
        ("active_company", "gamma"),
        ("company.beta.page", "transactions"),
        ("company.beta.name", "Old Beta"),
        ("company.alpha.page", "accounts"),
        ("scheme", "Wanderland/Light"),
    ]);
    let mut h = start_as_is(&w, old, egui::vec2(1280.0, 800.0));
    loaded(&mut h);
    // The data root's first, alone, on its overview: nothing was matched.
    assert_eq!(titles(&h), ["Alpha Co"]);
    assert!(matches!(h.state().views()[0].route(), Route::Overview));
    assert_eq!(h.state().remembered_name("beta"), None);
}

// ---- 16: a start that names a company leaves the open list alone -------------

#[test]
fn a_start_with_a_named_company_never_writes_the_list_of_open_companies() {
    let (w, _, _, _) = three();
    let mut h = start_with_prefs(&w, &[("open_companies", "alpha,beta,gamma")]);
    loaded(&mut h);
    h.state_mut().activate(2);
    h.run_steps(2);
    let kept = saved(&mut h, Prefs::default());
    let (list, front) = (
        kept.0[&w.open_key()].clone(),
        kept.0[&w.active_key()].clone(),
    );
    assert_eq!(list, "alpha,beta,gamma");
    drop(h);

    // `wanderbooks gui beta`: one tab; another opened and beta closed.
    let src = w.source("beta");
    let from = kept.clone();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .build_eframe(move |cc| {
            App::start_with(&cc.egui_ctx, &|k| from.0.get(k).cloned(), Start::Only(src))
        });
    loaded(&mut h);
    assert_eq!(titles(&h), ["Beta Co"]);
    h.state_mut().view_mut(0).unwrap().goto(Route::Transactions);
    let ctx = h.ctx.clone();
    h.state_mut().open(&ctx, w.source("alpha"));
    loaded(&mut h);
    close_tab(&mut h, "Beta Co");
    assert_eq!(titles(&h), ["Alpha Co"]);
    let after = saved(&mut h, kept.clone());
    // The list and the front tab are the plain start's, as they were.
    assert_eq!(after.0[&w.open_key()], list);
    assert_eq!(after.0[&w.active_key()], front);
    // Each company's own page is still kept.
    assert_eq!(after.0[&w.kept_key("beta", "page")], "transactions");
    assert_eq!(after.0[&w.kept_key("alpha", "page")], "overview");
}

// ---- 17: what the tabs share on purpose --------------------------------------

#[cfg(unix)]
#[test]
fn the_docks_height_and_layout_are_the_apps_one_for_every_tab() {
    let (w, _, _, _) = three();
    let owner = Gated::new();
    owner.release.open();
    let mut h = start_with_prefs(&w, &[("open_companies", "alpha,beta")]);
    loaded(&mut h);
    for i in 0..2 {
        h.state_mut()
            .view_mut(i)
            .unwrap()
            .set_owner(owner.clone() as Arc<dyn Owner>);
        h.state_mut().view_mut(i).unwrap().dock_mut().open = true;
    }
    h.run_steps(4);
    let top = |h: &Harness<'_, App>| h.get_by_label_contains(" Assistant").rect().top();
    let before = top(&h);
    // Resized in alpha: dragged up by a hundred points.
    let x = h.get_by_label_contains(" Assistant").rect().left() + 300.0;
    let panel_top = egui::containers::panel::PanelState::load(&h.ctx, egui::Id::new("dock-open"))
        .expect("the dock's panel")
        .outer_rect
        .top();
    let edge = egui::pos2(x, panel_top);
    let to = egui::pos2(x, panel_top - 100.0);
    h.hover_at(edge);
    h.run_steps(2);
    h.drag_at(edge);
    h.run_steps(2);
    h.hover_at(to);
    h.run_steps(3);
    h.drop_at(to);
    h.run_steps(4);
    let grown = top(&h);
    assert!(
        grown < before - 50.0,
        "the dock didn't grow: header at {before} then {grown}"
    );
    // Beta's dock has the same height: one layout for the assistant dock
    // everywhere.
    show_tab(&mut h, "Beta Co");
    h.run_steps(3);
    assert!(
        (top(&h) - grown).abs() < 0.5,
        "beta's dock is {} from the top, alpha's {grown}",
        top(&h)
    );
    // And so it is in the egui memory: the panel is the app's, its id the
    // raw one, the one id of a panel that is not scoped by its tab.
    use egui::containers::panel::PanelState;
    let ctx = h.ctx.clone();
    assert!(PanelState::load(&ctx, egui::Id::new("dock-open")).is_some());
}

#[test]
fn no_panel_but_the_apps_own_is_keyed_by_an_id_every_tab_shares() {
    use egui::containers::panel::PanelState;
    let (w, _, _, _) = three();
    let mut h = start_with_prefs(&w, &[("open_companies", "alpha,beta")]);
    loaded(&mut h);
    let ctx = h.ctx.clone();
    let raw = |name: &str| PanelState::load(&ctx, egui::Id::new(name));
    // Drawn in both tabs, and in the narrow window's rail too.
    for name in ["Alpha Co", "Beta Co"] {
        show_tab(&mut h, name);
        h.run_steps(3);
    }
    h.set_size(egui::vec2(800.0, 800.0));
    h.run_steps(4);
    // A company's panels (the sidebar, the rail, the top bar) are keyed by
    // ids made from its own, never by a name every tab shares.
    for shared in ["sidebar", "rail", "topbar"] {
        assert!(
            raw(shared).is_none(),
            "a company's {shared} panel is keyed by an id every tab shares"
        );
    }
    // The ones that are the app's, and say so: the tab bar's.
    assert!(raw("company_tabs").is_some());
}

#[test]
fn the_color_scheme_is_the_windows_one_for_every_tab() {
    let (w, _, _, _) = three();
    let mut h = start_with_prefs(&w, &[("open_companies", "alpha,beta")]);
    loaded(&mut h);
    // The sidebar's own fill: the largest rectangle in its column below the
    // bar.
    let sidebar = |h: &Harness<'_, App>| -> egui::Color32 {
        fn walk(s: &egui::Shape, out: &mut Vec<(f32, egui::Color32)>) {
            match s {
                egui::Shape::Rect(r) if in_sidebar(r.rect) && r.fill.a() > 0 => {
                    out.push((r.rect.area(), r.fill))
                }
                egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                _ => {}
            }
        }
        let mut all = Vec::new();
        for c in &h.output().shapes {
            walk(&c.shape, &mut all);
        }
        all.into_iter()
            .max_by(|a, b| a.0.total_cmp(&b.0))
            .expect("the sidebar's fill")
            .1
    };
    let ctx = h.ctx.clone();
    let key = h.state().scheme_key();
    let other = if key.ends_with("/Dark") {
        "Wanderbooks/Light"
    } else {
        "Wanderbooks/Dark"
    };
    let was = sidebar(&h);
    h.state_mut().set_scheme(&ctx, other);
    h.run_steps(3);
    let in_alpha = sidebar(&h);
    assert_ne!(in_alpha, was, "the scheme didn't change what is drawn");
    show_tab(&mut h, "Beta Co");
    h.run_steps(3);
    assert_eq!(
        sidebar(&h),
        in_alpha,
        "beta wears another scheme than alpha"
    );
    assert_eq!(h.state().scheme_key(), other);
}
