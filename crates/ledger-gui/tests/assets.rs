//! Fixed assets in the viewer: the register and each asset's schedule are
//! ledger-core's numbers exactly; what's posted comes from the journal;
//! search, books.show, the journal, and "Needs attention" know assets.

use chrono::{Datelike, Days, Months, NaiveDate};
use egui_kittest::Harness;
use ledger_core::{
    AccountType, AssetId, Currency, EntryId, Money, NewAsset, Side, depreciation_schedule,
    fixed_asset_register,
};
use ledger_gui::books::Books;
use ledger_gui::nav::{ReportKind, Route};
use ledger_gui::pages::assets::{MonthStatus, notices, schedule};
use ledger_gui::pages::journal::describe;
use ledger_gui::{App, Source, attention, data, fmt, palette};
use ledger_store::Store;
use ledger_store::viewer::ViewerTarget;
use proptest::prelude::*;
use rust_decimal::Decimal;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

static DIRS: AtomicUsize = AtomicUsize::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        let n = DIRS.fetch_add(1, Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!(
            "ledger-gui-assets-{tag}-{}-{n}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        TempDir(d)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn usd(cents: i64) -> Money {
    Money::new(Decimal::new(cents, 2), Currency::USD)
}

fn load(dir: &Path, today: NaiveDate) -> Books {
    data::load_once(&Source {
        data_dir: dir.join("books-data"),
        books_toml: None,
        today: Some(today),
    })
    .unwrap()
}

struct Fixture {
    _dir: TempDir,
    books: Books,
    /// The store's own ledger (the oracle's input), reopened from disk.
    oracle: ledger_core::Ledger,
    laptop: AssetId,
    monitor: AssetId,
    old: AssetId,
    laptop_purchase: EntryId,
}

const TODAY: (i32, u32, u32) = (2026, 10, 15);

fn today() -> NaiveDate {
    d(TODAY.0, TODAY.1, TODAY.2)
}

/// A laptop (tagged, serial, depreciation through July), a monitor (no
/// tag, through August), and an old laptop retagged, its serial cleared,
/// and sold in June.
fn fixture(tag: &str) -> Fixture {
    let dir = TempDir::new(tag);
    let data = dir.0.join("books-data");
    let mut s = Store::create(&data, Currency::USD).unwrap();
    let cash = s
        .add_account("1000", "Checking", AccountType::Asset, None, false)
        .unwrap();
    let equip = s
        .add_account("1500", "Equipment", AccountType::Asset, None, false)
        .unwrap();
    let accum = s
        .add_account(
            "1510",
            "Accumulated Depreciation",
            AccountType::Asset,
            None,
            false,
        )
        .unwrap();
    let capital = s
        .add_account("3000", "Capital", AccountType::Equity, None, false)
        .unwrap();
    let dep = s
        .add_account("6500", "Depreciation", AccountType::Expense, None, false)
        .unwrap();
    let gain = s
        .add_account("7000", "Gain on sale", AccountType::Revenue, None, false)
        .unwrap();
    let buy = |s: &mut Store, date, what: &str, cents| {
        let e = s.create_entry(date, what).unwrap();
        s.add_line(e, equip, Side::Debit, usd(cents)).unwrap();
        s.add_line(e, cash, Side::Credit, usd(cents)).unwrap();
        s.post_entry(e).unwrap();
        e
    };
    let e = s.create_entry(d(2026, 1, 1), "Owner contribution").unwrap();
    s.add_line(e, cash, Side::Debit, usd(1_000_000)).unwrap();
    s.add_line(e, capital, Side::Credit, usd(1_000_000))
        .unwrap();
    s.post_entry(e).unwrap();
    let register = |s: &mut Store, what: &str, date, cents, salvage, months| {
        s.register_asset(NewAsset {
            description: what.into(),
            purchase_date: date,
            cost: usd(cents),
            salvage_value: usd(salvage),
            useful_life_months: months,
            cost_account: equip,
            accum_depr_account: accum,
            expense_account: dep,
        })
        .unwrap()
    };
    let laptop_purchase = buy(&mut s, d(2026, 1, 10), "Laptop", 300_000);
    let laptop = register(&mut s, "Laptop", d(2026, 1, 10), 300_000, 0, 36);
    buy(&mut s, d(2026, 2, 1), "Monitor", 120_000);
    let monitor = register(&mut s, "Monitor", d(2026, 2, 1), 120_000, 0, 12);
    buy(&mut s, d(2026, 1, 1), "Old laptop", 60_000);
    let old = register(&mut s, "Old laptop", d(2026, 1, 1), 60_000, 10_000, 24);
    s.set_asset_tag(laptop, "FL-0001").unwrap();
    s.set_asset_serial(laptop, Some("SN-123")).unwrap();
    s.set_asset_tag(old, "OLD-1").unwrap();
    s.set_asset_tag(old, "FL-0009").unwrap();
    s.set_asset_serial(old, Some("X1")).unwrap();
    s.set_asset_serial(old, None).unwrap();
    for m in 1..=7 {
        s.run_depreciation(laptop, &format!("2026-{m:02}")).unwrap();
    }
    for m in 2..=8 {
        s.run_depreciation(monitor, &format!("2026-{m:02}"))
            .unwrap();
    }
    for m in 1..=3 {
        s.run_depreciation(old, &format!("2026-{m:02}")).unwrap();
    }
    s.dispose_asset(old, d(2026, 6, 30), usd(30_000), cash, gain)
        .unwrap();
    drop(s);
    let oracle = Store::open(&data).unwrap().ledger().clone();
    Fixture {
        books: load(&dir.0, today()),
        _dir: dir,
        oracle,
        laptop,
        monitor,
        old,
        laptop_purchase,
    }
}

#[test]
fn the_register_is_ledger_cores_register() {
    let f = fixture("register");
    let b = &f.books;
    for as_of in [
        d(2025, 12, 31),
        d(2026, 1, 10),
        d(2026, 3, 31),
        d(2026, 6, 29),
        d(2026, 6, 30),
        d(2026, 7, 31),
        d(2026, 9, 30),
        today(),
        d(2030, 1, 1),
    ] {
        let mine = b.asset_register(as_of);
        let oracle = fixed_asset_register(&f.oracle, as_of);
        assert_eq!(mine, oracle, "register on {as_of}");
    }
    let r = b.asset_register(today());
    assert_eq!(r.rows.len(), 3);
    let old = r.rows.iter().find(|x| x.asset_id == f.old).unwrap();
    assert_eq!(
        old.disposed,
        Some((d(2026, 6, 30), Decimal::new(30_000, 2)))
    );
    assert_eq!(
        ledger_gui::pages::assets::status_text(old.disposed),
        "Disposed Jun 30, 2026 for $300.00"
    );
}

#[test]
fn depreciation_due_is_the_register_through_the_last_finished_month() {
    let f = fixture("due");
    let b = &f.books;
    let due = b.depreciation_due(today());
    assert_eq!(due.through, d(2026, 9, 30));
    assert_eq!(
        due.amount,
        fixed_asset_register(&f.oracle, d(2026, 9, 30)).unposted_depreciation()
    );
    assert_eq!(due.periods, vec!["2026-08", "2026-09"]);
    // Independently: the laptop's August and September, the monitor's
    // September, straight from the schedules.
    let sched = |id| depreciation_schedule(f.oracle.get_asset(id).unwrap(), d(2026, 9, 30));
    let month = |id, p: &str| {
        sched(id)
            .into_iter()
            .find(|l| l.period == p)
            .unwrap()
            .amount
    };
    assert_eq!(
        due.amount,
        month(f.laptop, "2026-08") + month(f.laptop, "2026-09") + month(f.monitor, "2026-09")
    );
    // A month's last day counts that month as finished.
    assert_eq!(b.depreciation_due(d(2026, 7, 31)).through, d(2026, 7, 31));
    assert_eq!(
        b.depreciation_due(d(2026, 7, 31)).periods,
        Vec::<String>::new()
    );

    // The notices name the due amount exactly, and the month under way.
    let r = b.asset_register(today());
    let n = notices(b, &r);
    assert!(n[0].problem);
    assert!(
        n[0].text.contains(&format!(
            "Depreciation not yet posted through Sep 30, 2026: {}",
            fmt::dollars(due.amount)
        )),
        "{:?}",
        n[0].text
    );
    assert!(n[0].text.contains("ask the bookkeeper to run depreciation"));
    let accruing = r.unposted_depreciation() - due.amount;
    assert!(accruing > Decimal::ZERO);
    assert!(n[1].text.contains(&fmt::dollars(accruing)));
    assert!(!n[1].problem);
}

#[test]
fn an_assets_schedule_and_trail() {
    let f = fixture("schedule");
    let b = &f.books;
    let lines = schedule(b, f.laptop);
    let asset = f.oracle.get_asset(f.laptop).unwrap();
    let oracle = depreciation_schedule(asset, NaiveDate::MAX);
    assert_eq!(
        oracle.len(),
        37,
        "36 months from Jan 10: 37 calendar months"
    );
    assert_eq!(
        lines.iter().map(|(l, _)| l.clone()).collect::<Vec<_>>(),
        oracle
    );
    let trail = b.asset_trail(f.laptop);
    for (l, st) in &lines {
        let want = match l.period.as_str() {
            p if p <= "2026-07" => MonthStatus::Posted(trail.depreciation[p].0),
            "2026-08" | "2026-09" => MonthStatus::Due,
            "2026-10" => MonthStatus::ThisMonth,
            _ => MonthStatus::Scheduled,
        };
        assert_eq!(*st, want, "{}", l.period);
    }
    // Posted amounts are the schedule's, and the purchase is found.
    for (period, (_, amount)) in &trail.depreciation {
        let l = oracle.iter().find(|l| &l.period == period).unwrap();
        assert_eq!(*amount, l.amount);
    }
    assert_eq!(trail.purchase_entry, Some(f.laptop_purchase));
    assert_eq!(b.entry_assets.get(&f.laptop_purchase), Some(&f.laptop));
    assert_eq!(trail.entries().len(), 1 + 7);
    // The sold laptop: its disposal entry, and a final depreciation.
    let old = b.asset_trail(f.old);
    let disposal = old.disposal_entry.expect("disposal entry");
    assert_eq!(b.entry(disposal).unwrap().origin, "Asset disposal");
    assert!(old.depreciation.contains_key("2026-06"));
    assert_eq!(b.entry_assets.get(&disposal), Some(&f.old));
    assert_eq!(b.asset_label(f.laptop), "FL-0001 Laptop");
    assert_eq!(b.asset_label(f.monitor), "Monitor");
}

#[test]
fn search_finds_assets_by_tag_serial_and_description() {
    let f = fixture("search");
    let b = &f.books;
    for q in ["FL-0001", "fl-0001", " Fl-0001 "] {
        let items = palette::search(b, q);
        assert_eq!(items[0].route, Route::Asset(f.laptop), "{q}");
        assert_eq!(items[0].score, 2000);
    }
    let first_asset = |q: &str| {
        palette::search(b, q)
            .into_iter()
            .find(|i| matches!(i.route, Route::Asset(_)))
            .map(|i| i.route)
    };
    assert_eq!(first_asset("sn-123"), Some(Route::Asset(f.laptop)));
    assert_eq!(
        palette::search(b, "SN-123")[0].route,
        Route::Asset(f.laptop)
    );
    assert_eq!(first_asset("monitor"), Some(Route::Asset(f.monitor)));
    assert_eq!(first_asset("fl-0009"), Some(Route::Asset(f.old)));
    // Everything else still found as before.
    assert!(
        palette::search(b, "checking")
            .iter()
            .any(|i| matches!(i.route, Route::Account(_)))
    );
    assert!(
        palette::search(b, "")
            .iter()
            .any(|i| i.route == Route::Assets)
    );
    assert!(
        palette::search(b, "fixed asset register")
            .iter()
            .any(|i| i.route == Route::Reports(ReportKind::FixedAssetRegister))
    );
    assert_eq!(
        palette::describe(b, &Route::Asset(f.laptop)),
        "FL-0001 Laptop"
    );
}

#[test]
fn journal_text_for_asset_events() {
    let f = fixture("journal");
    let b = &f.books;
    let texts: Vec<(String, Option<Route>, Option<Route>)> = b
        .journal
        .iter()
        .filter(|l| l.line_type.starts_with("asset."))
        .filter_map(|l| describe(b, l))
        .map(|e| (e.text, e.route, e.also))
        .collect();
    let has = |t: &str, route: Option<Route>| {
        assert!(
            texts.iter().any(|(x, r, _)| x == t && *r == route),
            "no {t:?} ({route:?}) in {texts:#?}"
        );
    };
    has("Registered asset Laptop", Some(Route::Asset(f.laptop)));
    has("Tagged Laptop FL-0001", Some(Route::Asset(f.laptop)));
    has(
        "Retagged Old laptop from OLD-1 to FL-0009",
        Some(Route::Asset(f.old)),
    );
    has(
        "Serial number of Laptop set to SN-123",
        Some(Route::Asset(f.laptop)),
    );
    has(
        "Serial number of Old laptop cleared",
        Some(Route::Asset(f.old)),
    );
    let disposal = b.asset_trail(f.old).disposal_entry;
    has(
        "Disposed Old laptop on 2026-06-30 for $300.00",
        disposal.map(Route::Entry),
    );
    let (jan, _) = b.asset_trail(f.laptop).depreciation["2026-01"];
    let dep = texts
        .iter()
        .find(|(x, _, _)| x.starts_with("Depreciation posted: Laptop 2026-01"))
        .expect("depreciation text");
    assert_eq!(dep.1, Some(Route::Entry(jan)));
    assert_eq!(dep.2, Some(Route::Asset(f.laptop)));
}

#[test]
fn attention_asks_for_depreciation_and_tags() {
    let f = fixture("attention");
    let b = &f.books;
    let items = attention::find(b);
    let due = b.depreciation_due(today());
    let dep = items
        .iter()
        .find(|a| a.title == "Depreciation not posted through Sep 30, 2026")
        .expect("depreciation item");
    assert_eq!(dep.target, Some(Route::Assets));
    assert!(dep.prompt.starts_with("In wanderbooks,"));
    assert!(
        dep.prompt
            .contains("books.run_depreciation for 2026-08, 2026-09")
    );
    assert!(dep.prompt.contains(&fmt::dollars(due.amount)));
    let tag = items
        .iter()
        .find(|a| a.title == "No tag: Monitor")
        .expect("tag item");
    assert_eq!(tag.target, Some(Route::Asset(f.monitor)));
    assert!(tag.prompt.contains("books.label_asset"));
    // Tagged, disposed, and agreeing assets ask for nothing.
    assert!(
        !items
            .iter()
            .any(|a| a.title.contains("Laptop") && a.title.starts_with("No tag"))
    );
    assert!(!items.iter().any(|a| a.title.contains("not in the")));
    for a in &items {
        assert!(a.prompt.contains("wanderbooks"), "{:?}", a.prompt);
    }

    // Caught up and labeled: nothing about assets.
    let data = f._dir.0.join("books-data");
    let mut s = Store::open(&data).unwrap();
    s.run_depreciation(f.laptop, "2026-08").unwrap();
    s.run_depreciation(f.laptop, "2026-09").unwrap();
    s.run_depreciation(f.monitor, "2026-09").unwrap();
    s.set_asset_tag(f.monitor, "FL-0002").unwrap();
    drop(s);
    let b = load(&f._dir.0, today());
    let titles: Vec<String> = attention::find(&b).into_iter().map(|a| a.title).collect();
    assert!(
        !titles
            .iter()
            .any(|t| t.contains("Depreciation") || t.starts_with("No tag")),
        "{titles:?}"
    );
    assert_eq!(b.depreciation_due(today()).amount, Decimal::ZERO);
}

#[test]
fn unregistered_equipment_is_flagged() {
    let f = fixture("unregistered");
    let data = f._dir.0.join("books-data");
    let mut s = Store::open(&data).unwrap();
    let equip = s.ledger().chart().find_by_code("1500").unwrap().id;
    let cash = s.ledger().chart().find_by_code("1000").unwrap().id;
    let e = s.create_entry(d(2026, 9, 2), "Printer").unwrap();
    s.add_line(e, equip, Side::Debit, usd(45_000)).unwrap();
    s.add_line(e, cash, Side::Credit, usd(45_000)).unwrap();
    s.post_entry(e).unwrap();
    drop(s);
    let b = load(&f._dir.0, today());
    let r = b.asset_register(today());
    assert_eq!(r.unregistered_cost(), Decimal::new(45_000, 2));
    assert!(
        notices(&b, &r)
            .iter()
            .any(|n| n.problem && n.text.starts_with("$450.00 in the asset cost accounts"))
    );
    assert!(
        attention::find(&b)
            .iter()
            .any(|a| a.title == "$450.00 of equipment not in the asset register")
    );
}

#[test]
fn books_show_opens_assets() {
    let f = fixture("show");
    let laptop = f.laptop;
    let books = f.books;
    let mut h = Harness::builder()
        .with_size(egui::vec2(1440.0, 920.0))
        .build_eframe(|cc| App::with_books(&cc.egui_ctx, books, false, Route::Overview));
    h.state_mut().show(ViewerTarget::Page {
        page: "assets".into(),
    });
    h.run_steps(2);
    assert_eq!(h.state_mut().route(), &Route::Assets);
    h.state_mut().show(ViewerTarget::Asset { asset_id: laptop });
    h.run_steps(2);
    assert_eq!(h.state_mut().route(), &Route::Asset(laptop));
    h.state_mut().show(ViewerTarget::Report {
        report: "fixed_asset_register".into(),
        start: None,
        end: Some("2026-06-30".into()),
        columns: None,
        compare: None,
    });
    h.run_steps(2);
    assert_eq!(
        h.state_mut().route(),
        &Route::Reports(ReportKind::FixedAssetRegister)
    );
    assert!(matches!(
        h.state_mut().state_mut().reports.range,
        ledger_gui::pages::reports::Range::Custom(_, e) if e == d(2026, 6, 30)
    ));
    // An unknown asset is ignored, not a crash.
    h.state_mut().show(ViewerTarget::Asset { asset_id: 999 });
    h.run_steps(1);
    assert_eq!(
        h.state_mut().route(),
        &Route::Reports(ReportKind::FixedAssetRegister)
    );
}

/// The asset pages render at every width without panicking, and with no
/// assets at all.
#[test]
fn asset_pages_render() {
    let f = fixture("render");
    let routes = [
        Route::Assets,
        Route::Asset(f.laptop),
        Route::Asset(f.monitor),
        Route::Asset(f.old),
        Route::Asset(999),
        Route::Reports(ReportKind::FixedAssetRegister),
    ];
    for width in [420.0, 900.0, 1440.0] {
        for route in &routes {
            let books = load(&f._dir.0, today());
            let mut h = Harness::builder()
                .with_size(egui::vec2(width, 900.0))
                .build_eframe(|cc| App::with_books(&cc.egui_ctx, books, true, route.clone()));
            h.run_steps(3);
        }
    }
    let empty = TempDir::new("empty");
    drop(Store::create(&empty.0.join("books-data"), Currency::USD).unwrap());
    for route in [
        Route::Assets,
        Route::Reports(ReportKind::FixedAssetRegister),
    ] {
        let books = load(&empty.0, today());
        let mut h = Harness::builder()
            .with_size(egui::vec2(1280.0, 800.0))
            .build_eframe(|cc| App::with_books(&cc.egui_ctx, books, false, route.clone()));
        h.run_steps(3);
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(12))]

    /// On random assets with depreciation run for random months, the
    /// register on any date is ledger-core's, and what's due is the
    /// register through the last finished month.
    #[test]
    fn register_and_due_match_the_oracle(
        assets in proptest::collection::vec(
            (0u64..400, 1i64..2_000_000, 0i64..50, 1u32..60, 0u32..30),
            1..5,
        ),
        today_off in 0u64..900,
        probe in 0u64..1200,
    ) {
        let dir = TempDir::new("prop");
        let data = dir.0.join("books-data");
        let mut s = Store::create(&data, Currency::USD).unwrap();
        let cash = s.add_account("1000", "Checking", AccountType::Asset, None, false).unwrap();
        let equip = s.add_account("1500", "Equipment", AccountType::Asset, None, false).unwrap();
        let accum = s.add_account("1510", "Accumulated", AccountType::Asset, None, false).unwrap();
        let dep = s.add_account("6500", "Depreciation", AccountType::Expense, None, false).unwrap();
        let base = d(2025, 1, 1);
        for (i, (off, cents, salvage_pct, months, run)) in assets.iter().enumerate() {
            let date = base + Days::new(*off);
            let e = s.create_entry(date, &format!("Buy {i}")).unwrap();
            s.add_line(e, equip, Side::Debit, usd(*cents)).unwrap();
            s.add_line(e, cash, Side::Credit, usd(*cents)).unwrap();
            s.post_entry(e).unwrap();
            let id = s.register_asset(NewAsset {
                description: format!("Asset {i}"),
                purchase_date: date,
                cost: usd(*cents),
                salvage_value: usd(cents * salvage_pct / 100),
                useful_life_months: *months,
                cost_account: equip,
                accum_depr_account: accum,
                expense_account: dep,
            }).unwrap();
            // Depreciation for the first `run` months of its life.
            for k in 0..*run {
                let m = date.with_day0(0).unwrap() + Months::new(k);
                let _ = s.run_depreciation(id, &m.format("%Y-%m").to_string());
            }
        }
        drop(s);
        let oracle = Store::open(&data).unwrap().ledger().clone();
        let today = base + Days::new(today_off);
        let b = load(&dir.0, today);
        let at = base + Days::new(probe);
        prop_assert_eq!(b.asset_register(at), fixed_asset_register(&oracle, at));
        let due = b.depreciation_due(today);
        prop_assert!(due.through <= today);
        prop_assert_eq!(due.through.succ_opt().unwrap().day0(), 0);
        prop_assert!(today - due.through < chrono::TimeDelta::days(32));
        prop_assert_eq!(
            due.amount,
            fixed_asset_register(&oracle, due.through).unposted_depreciation()
        );
        // Every period listed is unposted and in some schedule.
        for p in &due.periods {
            let found = b.ledger.registered_assets().any(|a| {
                depreciation_schedule(a, due.through).iter().any(|l| &l.period == p)
                    && !b.asset_trail(a.id).depreciation.contains_key(p)
            });
            prop_assert!(found, "{} listed but not due", p);
        }
    }
}

/// Every event the store can journal reads as a sentence in the journal
/// and recent activity, never as its raw type: the event types come from
/// the store's own source (its replay arms), so a new event without
/// journal text fails here. (asset.acquisition_linked showed raw,
/// 2026-10-15.)
#[test]
fn every_journal_event_has_plain_language_text() {
    let src = include_str!("../../ledger-store/src/store.rs");
    let mut kinds: Vec<&str> = src
        .lines()
        .flat_map(|l| l.split('"').skip(1).step_by(2))
        .filter(|t| {
            let mut parts = t.split('.');
            matches!(
                (parts.next(), parts.next(), parts.next()),
                (Some(a), Some(b), None)
                    if !a.is_empty() && !b.is_empty()
                        && a.chars().chain(b.chars()).all(|c| c.is_ascii_lowercase() || c == '_')
            )
        })
        .filter(|t| src.contains(&format!("\"{t}\" =>")) || src.contains(&format!("\"{t}\" |")))
        .collect();
    kinds.sort_unstable();
    kinds.dedup();
    assert!(kinds.len() > 30, "found only {kinds:?}");
    let f = fixture("every-event");
    for kind in kinds {
        let line = ledger_store::JournalLine {
            seq: 1,
            ts: "2026-10-15T00:00:00Z".into(),
            line_type: kind.to_string(),
            payload: serde_json::json!({}),
            prev: String::new(),
        };
        let e = ledger_gui::pages::journal::describe_any(&f.books, &line);
        assert_ne!(e.text, kind, "{kind} has no journal text");
    }
}
