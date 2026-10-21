//! Fixed assets in the viewer: the register and each asset's schedule are
//! ledger-core's numbers exactly; what's posted comes from the journal;
//! search, books.show, the journal, and "Needs attention" know assets.

use chrono::{Datelike, Days, Months, NaiveDate};
use egui_kittest::Harness;
use ledger_core::{
    AccountType, AssetBasis, AssetId, BasisRule, Currency, EntryId, Money, NewAsset, Side,
    depreciation_schedule, fixed_asset_register,
};
use ledger_gui::books::Books;
use ledger_gui::nav::{ReportKind, Route};
use ledger_gui::pages::assets::{MonthStatus, TaxFacts, missing_tax_notice, notices, schedule};
use ledger_gui::pages::journal::describe;
use ledger_gui::{App, Source, attention, data, fmt, palette};
use ledger_store::Store;
use ledger_store::ops::ViewerTarget;
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

/// A laptop (tagged, serial, depreciation through July; tax basis first
/// recorded as a carryover, then corrected to cost), a monitor (no tag, no
/// tax basis, through August), and an old laptop retagged, its serial
/// cleared, converted from personal use worth less than its basis (a
/// built-in loss), and sold in June.
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
            in_service_date: date,
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
    let given = |rule, adjusted: i64, fmv: i64| {
        AssetBasis::contribution(rule, Decimal::new(adjusted, 2), Decimal::new(fmv, 2))
    };
    s.set_asset_basis(laptop, given(BasisRule::Carryover723, 200_000, 300_000))
        .unwrap();
    s.set_asset_basis(laptop, AssetBasis::purchase()).unwrap();
    s.set_asset_basis(old, given(BasisRule::PersonalUseConversion, 70_000, 60_000))
        .unwrap();
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
        let mine = b.asset_register(as_of).unwrap();
        let oracle = fixed_asset_register(&f.oracle, as_of).unwrap();
        assert_eq!(mine, oracle, "register on {as_of}");
    }
    let r = b.asset_register(today()).unwrap();
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
    let due = b.depreciation_due(today()).unwrap();
    assert_eq!(due.through, d(2026, 9, 30));
    assert_eq!(
        due.amount,
        fixed_asset_register(&f.oracle, d(2026, 9, 30))
            .unwrap()
            .unposted_depreciation()
    );
    assert_eq!(due.periods, vec!["2026-08", "2026-09"]);
    // Independently: the laptop's August and September, the monitor's
    // September, straight from the schedules.
    let sched =
        |id| depreciation_schedule(f.oracle.get_asset(id).unwrap(), d(2026, 9, 30)).unwrap();
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
    assert_eq!(
        b.depreciation_due(d(2026, 7, 31)).unwrap().through,
        d(2026, 7, 31)
    );
    assert_eq!(
        b.depreciation_due(d(2026, 7, 31)).unwrap().periods,
        Vec::<String>::new()
    );

    // The notices name the due amount exactly, and the month under way.
    let r = b.asset_register(today()).unwrap();
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
    let lines = schedule(b, f.laptop).unwrap();
    let asset = f.oracle.get_asset(f.laptop).unwrap();
    let oracle = depreciation_schedule(asset, NaiveDate::MAX).unwrap();
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
    let due = b.depreciation_due(today()).unwrap();
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
    assert_eq!(b.depreciation_due(today()).unwrap().amount, Decimal::ZERO);
}

/// The viewer's tax figures are ledger-core's: each asset's
/// (Asset::basis_figures), each register row's, and the register's totals
/// and missing count, on every date.
#[test]
fn basis_figures_are_ledger_cores() {
    let f = fixture("tax");
    let b = &f.books;
    for id in [f.laptop, f.monitor, f.old] {
        let core = f.oracle.get_asset(id).unwrap();
        let mine = TaxFacts::of(b.ledger.get_asset(id).unwrap());
        let figures = core.basis_figures().unwrap();
        assert_eq!(mine.rule, core.basis.as_ref().map(|x| x.rule), "asset {id}");
        assert_eq!(
            mine.contributor_adjusted_basis,
            core.basis
                .as_ref()
                .and_then(|x| x.contributor_adjusted_basis)
        );
        assert_eq!(mine.tax_basis_723, figures.and_then(|x| x.tax_basis_723));
        assert_eq!(mine.depreciable_basis, figures.map(|x| x.depreciable_basis));
        assert_eq!(
            mine.built_in_gain_704c,
            figures.and_then(|x| x.built_in_gain_704c)
        );
        assert_eq!(mine.fmv_at_contribution, core.fmv_at_contribution());
    }
    // Independently: the laptop's basis is its cost; the old laptop's
    // §723 basis is the member's $700.00, its depreciable basis its value
    // ($600.00, the lesser), a $100.00 built-in loss.
    let laptop = TaxFacts::of(b.ledger.get_asset(f.laptop).unwrap());
    assert_eq!(laptop.rule, Some(BasisRule::Purchase));
    assert_eq!(laptop.depreciable_basis, Some(Decimal::new(300_000, 2)));
    assert_eq!(laptop.tax_basis_723, None);
    assert_eq!(laptop.built_in_gain_704c, None);
    let old = TaxFacts::of(b.ledger.get_asset(f.old).unwrap());
    assert_eq!(old.tax_basis_723, Some(Decimal::new(70_000, 2)));
    assert_eq!(old.depreciable_basis, Some(Decimal::new(60_000, 2)));
    assert_eq!(old.built_in_gain_704c, Some(Decimal::new(-10_000, 2)));
    assert_eq!(
        old.summary(),
        "Personal-use conversion · §723 basis $700.00"
    );
    assert_eq!(
        old.detail(),
        "Depreciable basis $600.00: Personal-use conversion (Treas. Reg. §1.168(i)-4(b)); §723 basis $700.00; value at contribution $600.00; built-in loss $100.00."
    );
    assert_eq!(
        TaxFacts::of(b.ledger.get_asset(f.monitor).unwrap()).detail(),
        "Tax basis not recorded."
    );
    assert_eq!(
        TaxFacts::of(b.ledger.get_asset(f.monitor).unwrap()),
        TaxFacts {
            rule: None,
            contributor_adjusted_basis: None,
            fmv_at_contribution: None,
            fmv_recorded: false,
            tax_basis_723: None,
            depreciable_basis: None,
            built_in_gain_704c: None,
            sec704c_method: None,
        }
    );
    for as_of in [d(2026, 1, 31), d(2026, 6, 30), today()] {
        let r = b.asset_register(as_of).unwrap();
        let oracle = fixed_asset_register(&f.oracle, as_of).unwrap();
        assert_eq!(
            r.total_depreciable_basis, oracle.total_depreciable_basis,
            "{as_of}"
        );
        assert_eq!(r.total_tax_basis_723, oracle.total_tax_basis_723, "{as_of}");
        assert_eq!(r.missing_basis, oracle.missing_basis, "{as_of}");
        for x in &r.rows {
            let a = f.oracle.get_asset(x.asset_id).unwrap();
            let facts = TaxFacts::of_row(x);
            assert_eq!(facts.depreciable_basis, a.depreciable_basis());
            assert_eq!(facts.built_in_gain_704c, a.built_in_gain_704c());
            assert_eq!(facts, TaxFacts::of(a));
        }
    }
    // Today: the laptop's $3,000.00 (the monitor has none; the old laptop
    // is gone), and one missing, said in the notices.
    let r = b.asset_register(today()).unwrap();
    assert_eq!(r.total_depreciable_basis, Decimal::new(300_000, 2));
    assert_eq!(r.missing_basis, 1);
    let want = missing_tax_notice(1).unwrap();
    assert!(want.starts_with("Tax basis not recorded for 1 asset"));
    assert!(want.contains("books.set_asset_tax_basis"));
    assert!(notices(b, &r).iter().any(|n| n.problem && n.text == want));
    assert_eq!(missing_tax_notice(0), None);
    assert!(
        missing_tax_notice(2)
            .unwrap()
            .starts_with("Tax basis not recorded for 2 assets")
    );
}

/// The journal names each basis figure for what it is: the §723 basis,
/// the depreciable basis, the built-in gain or loss; a pre-P3
/// `asset.tax_basis_set` too, its value read as the book cost.
#[test]
fn journal_text_names_each_basis_figure() {
    let f = fixture("tax-journal");
    let b = &f.books;
    let texts: Vec<(String, Option<Route>)> = b
        .journal
        .iter()
        .filter(|l| l.line_type == "asset.basis_set")
        .filter_map(|l| describe(b, l))
        .map(|e| (e.text, e.route))
        .collect();
    assert_eq!(
        texts,
        vec![
            (
                "Tax basis of FL-0001: carryover, value $3,000.00, contributor's adjusted basis $2,000.00: §723 basis $2,000.00, depreciable basis $2,000.00, built-in gain $1,000.00"
                    .to_string(),
                Some(Route::Asset(f.laptop))
            ),
            (
                "Tax basis of FL-0001: purchase: depreciable basis $3,000.00".to_string(),
                Some(Route::Asset(f.laptop))
            ),
            (
                "Tax basis of FL-0009: personal-use conversion, value $600.00, contributor's adjusted basis $700.00: §723 basis $700.00, depreciable basis $600.00, built-in loss $100.00"
                    .to_string(),
                Some(Route::Asset(f.old))
            ),
        ]
    );
    // A line an earlier version wrote: rule by its old name, no value.
    let data = f._dir.0.join("books-data");
    let mut w = ledger_store::JournalWriter::open(&data.join(ledger_store::JOURNAL_FILE)).unwrap();
    w.append(
        "asset.tax_basis_set",
        serde_json::json!({"asset_id": f.monitor, "rule": "personal_use_conversion", "contributor_basis": "1500.00"}),
    )
    .unwrap();
    drop(w);
    let b = load(&f._dir.0, today());
    let legacy: Vec<String> = b
        .journal
        .iter()
        .filter(|l| l.line_type == "asset.tax_basis_set")
        .filter_map(|l| describe(&b, l))
        .map(|e| e.text)
        .collect();
    assert_eq!(
        legacy,
        [
            "Tax basis of Monitor: personal-use conversion, contributor's adjusted basis $1,500.00: §723 basis $1,500.00, depreciable basis $1,200.00, built-in loss $300.00"
        ]
    );
}

/// An asset held with no tax basis asks for one, with a prompt naming
/// the tool and the asset; recording it clears the item.
#[test]
fn attention_asks_for_tax_basis() {
    let f = fixture("tax-attention");
    let items = attention::find(&f.books);
    let tax: Vec<_> = items
        .iter()
        .filter(|a| a.title.starts_with("No tax basis"))
        .collect();
    assert_eq!(tax.len(), 1, "only the monitor: {tax:#?}");
    let item = tax[0];
    assert_eq!(item.title, "No tax basis: Monitor");
    assert_eq!(item.target, Some(Route::Asset(f.monitor)));
    assert!(item.prompt.starts_with(&format!(
        "In wanderbooks, record the tax basis of asset {} (\"Monitor\", book cost $1,200.00) with books.set_asset_tax_basis.",
        f.monitor
    )));
    assert!(item.detail.contains("$1,200.00"));

    let data = f._dir.0.join("books-data");
    let mut s = Store::open(&data).unwrap();
    s.set_asset_tag(f.monitor, "FL-0002").unwrap();
    drop(s);
    let b = load(&f._dir.0, today());
    let item = attention::find(&b)
        .into_iter()
        .find(|a| a.title.starts_with("No tax basis"))
        .expect("still missing");
    assert_eq!(item.title, "No tax basis: FL-0002 Monitor");
    assert!(
        item.prompt
            .starts_with("In wanderbooks, record the tax basis of FL-0002 (")
    );

    let mut s = Store::open(&data).unwrap();
    s.set_asset_basis(
        f.monitor,
        AssetBasis::contribution(
            BasisRule::Carryover723,
            Decimal::new(90_000, 2),
            Decimal::new(120_000, 2),
        ),
    )
    .unwrap();
    drop(s);
    let b = load(&f._dir.0, today());
    assert!(
        !attention::find(&b)
            .iter()
            .any(|a| a.title.starts_with("No tax basis"))
    );
    let r = b.asset_register(today()).unwrap();
    assert_eq!(r.missing_basis, 0);
    assert!(
        !notices(&b, &r)
            .iter()
            .any(|n| n.text.contains("Tax basis not recorded"))
    );
    // $3,000.00 laptop + the monitor's $900.00 carryover.
    assert_eq!(r.total_depreciable_basis, Decimal::new(390_000, 2));
    // Its value is recorded, so nothing asks for it; its $300.00 built-in
    // gain has no §704(c) method, so a method is asked for.
    let items = attention::find(&b);
    assert!(
        !items
            .iter()
            .any(|a| a.title.starts_with("Confirm its value at contribution"))
    );
    let method = items
        .iter()
        .find(|a| a.title.starts_with("Choose a §704(c) method"))
        .expect("asked for a method");
    assert!(method.prompt.contains("books.set_asset_tax_basis"));
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
    let r = b.asset_register(today()).unwrap();
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
                in_service_date: date,
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
        prop_assert_eq!(b.asset_register(at).unwrap(), fixed_asset_register(&oracle, at).unwrap());
        let due = b.depreciation_due(today).unwrap();
        prop_assert!(due.through <= today);
        prop_assert_eq!(due.through.succ_opt().unwrap().day0(), 0);
        prop_assert!(today - due.through < chrono::TimeDelta::days(32));
        prop_assert_eq!(
            due.amount,
            fixed_asset_register(&oracle, due.through).unwrap().unposted_depreciation()
        );
        // Every period listed is unposted and in some schedule.
        for p in &due.periods {
            let found = b.ledger.registered_assets().any(|a| {
                depreciation_schedule(a, due.through).unwrap().iter().any(|l| &l.period == p)
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

/// Books holding asset X of the P3 depreciation brief (12,000.00 over 12
/// months from 2026-01-01, 1,000.00 a month), with the store to post to.
fn asset_x(tag: &str) -> (TempDir, PathBuf, Store, AssetId, (u64, u64)) {
    let dir = TempDir::new(tag);
    let data = dir.0.join("books-data");
    let mut s = Store::create(&data, Currency::USD).unwrap();
    let mut add = |code: &str, t| s.add_account(code, code, t, None, false).unwrap();
    let cash = add("1000", AccountType::Asset);
    let equip = add("1500", AccountType::Asset);
    let accum = add("1510", AccountType::Asset);
    let dep = add("6500", AccountType::Expense);
    let gain = add("7000", AccountType::Revenue);
    let x = s
        .register_asset(NewAsset {
            description: "Asset X".into(),
            in_service_date: d(2026, 1, 1),
            cost: usd(1_200_000),
            salvage_value: usd(0),
            useful_life_months: 12,
            cost_account: equip,
            accum_depr_account: accum,
            expense_account: dep,
        })
        .unwrap();
    (dir, data, s, x, (cash, gain))
}

/// Defect 22 in the viewer: a disposed asset's page shows its schedule
/// through the disposal day, held, as the store posted it. Hand check:
/// January through May run, disposed of 2026-06-15: June is 1,000.00 ×
/// 15/30 = 500.00 (the final depreciation, posted), accumulated 5,500.00,
/// book value 6,500.00; every month posted.
#[test]
fn a_disposed_assets_page_shows_what_it_depreciated_through_its_disposal() {
    let (dir, _data, mut s, x, (cash, gain)) = asset_x("disposed-page");
    for m in 1..=5 {
        s.run_depreciation(x, &format!("2026-{m:02}")).unwrap();
    }
    s.dispose_asset(x, d(2026, 6, 15), usd(700_000), cash, gain)
        .unwrap();
    drop(s);
    let b = load(&dir.0, today());
    let lines = schedule(&b, x).unwrap();
    assert_eq!(lines.len(), 6);
    let (jun, st) = lines.last().unwrap();
    assert_eq!(jun.period, "2026-06");
    assert_eq!(jun.amount, Decimal::new(50_000, 2));
    assert_eq!(jun.accumulated, Decimal::new(550_000, 2));
    assert_eq!(jun.book_value, Decimal::new(650_000, 2));
    assert!(matches!(st, MonthStatus::Posted(_)));
    assert!(
        lines
            .iter()
            .all(|(_, st)| matches!(st, MonthStatus::Posted(_)))
    );
    assert_eq!(b.asset_trail(x).posted(), Decimal::new(550_000, 2));
}

/// The catch-up rule in the viewer (plan/served.md P3, settled in design):
/// a month a later run caught up is Covered, never Due forever; a run
/// reversed within its month is no longer the books', so its month (and
/// one it covered) is Due again, and the books carry what is in effect.
/// Hand check: January and March run (March 2,000.00), February skipped:
/// February Covered; March's run reversed on Mar 31: February and March
/// Due, 1,000.00 carried.
#[test]
fn a_month_a_later_run_caught_up_is_covered_and_a_reversed_one_due() {
    let (dir, data, mut s, x, _) = asset_x("caught-up-page");
    s.run_depreciation(x, "2026-01").unwrap();
    let mar = s.run_depreciation(x, "2026-03").unwrap();
    assert_eq!(mar.amount, Decimal::new(200_000, 2));
    drop(s);
    let b = load(&dir.0, today());
    let st: Vec<MonthStatus> = schedule(&b, x)
        .unwrap()
        .into_iter()
        .map(|(_, s)| s)
        .collect();
    assert!(matches!(st[0], MonthStatus::Posted(_)));
    assert_eq!(st[1], MonthStatus::Covered);
    assert!(matches!(st[2], MonthStatus::Posted(_)));
    assert_eq!(st[3], MonthStatus::Due);
    let due = b.depreciation_due(d(2026, 4, 30)).unwrap();
    assert_eq!(due.periods, vec!["2026-04"]);
    let mut s = Store::open(&data).unwrap();
    s.reverse_entry(mar.entry_id.unwrap(), d(2026, 3, 31), "reversed")
        .unwrap();
    drop(s);
    let b = load(&dir.0, today());
    let st: Vec<MonthStatus> = schedule(&b, x)
        .unwrap()
        .into_iter()
        .map(|(_, s)| s)
        .collect();
    assert_eq!((st[1], st[2]), (MonthStatus::Due, MonthStatus::Due));
    assert_eq!(b.asset_trail(x).posted(), Decimal::new(100_000, 2));
    let due = b.depreciation_due(d(2026, 4, 30)).unwrap();
    assert_eq!(due.periods, vec!["2026-02", "2026-03", "2026-04"]);
}

/// P3 review of the catch-up rule, in the viewer: a run reversed after its
/// month ended carried that month at its end, so the month isn't Due (a
/// run of it would post nothing); the month the reversal is in is, with
/// everything since caught up. Hand check: January and March run (March
/// 2,000.00); March's run reversed on Apr 2: February and March Covered,
/// April Due; through September (today is in October) 9,000.00 is due,
/// less the 1,000.00 carried: 8,000.00 unposted.
#[test]
fn a_run_reversed_after_its_month_leaves_the_reversals_month_due() {
    let (dir, data, mut s, x, _) = asset_x("reversed-later-page");
    s.run_depreciation(x, "2026-01").unwrap();
    let mar = s.run_depreciation(x, "2026-03").unwrap();
    s.reverse_entry(mar.entry_id.unwrap(), d(2026, 4, 2), "reversed")
        .unwrap();
    drop(s);
    let b = load(&dir.0, today());
    let st: Vec<MonthStatus> = schedule(&b, x)
        .unwrap()
        .into_iter()
        .map(|(_, s)| s)
        .collect();
    assert_eq!(
        (st[1], st[2], st[3]),
        (MonthStatus::Covered, MonthStatus::Covered, MonthStatus::Due)
    );
    assert_eq!(
        ledger_gui::pages::assets::unposted(&b, x).unwrap(),
        Decimal::new(800_000, 2)
    );
    let due = b.depreciation_due(d(2026, 4, 30)).unwrap();
    assert_eq!(due.periods, vec!["2026-04"]);
    // The store agrees: March again posts nothing, April 3,000.00.
    let mut s = Store::open(&data).unwrap();
    assert_eq!(s.run_depreciation(x, "2026-03").unwrap().entry_id, None);
    assert_eq!(
        s.run_depreciation(x, "2026-04").unwrap().amount,
        Decimal::new(300_000, 2)
    );
}

/// P3 review: a disposed asset's figures in the viewer are what its
/// disposal took. Hand check: January through June run (6,000.00, June's
/// dated Jun 30), disposed of Jun 15 for 7,000.00: the disposal credits
/// June's last 15 days back (500.00), so the books' depreciation for it is
/// 5,500.00 and its book value at disposal 6,500.00, as books.dispose_asset
/// and books.list_assets say. Disposed the same way after March's run was
/// reversed (on Jun 20), the disposal entry itself charges 500.00 (March's
/// 1,000.00, less June's last 15 days already in June's run): again
/// 5,500.00, and March isn't Due, the disposal took it.
#[test]
fn a_disposed_assets_figures_are_what_its_disposal_took() {
    let (dir, _data, mut s, x, (cash, gain)) = asset_x("disposed-true-up");
    for m in 1..=6 {
        s.run_depreciation(x, &format!("2026-{m:02}")).unwrap();
    }
    s.dispose_asset(x, d(2026, 6, 15), usd(700_000), cash, gain)
        .unwrap();
    drop(s);
    let b = load(&dir.0, today());
    assert_eq!(b.asset_trail(x).posted(), Decimal::new(550_000, 2));
    assert_eq!(b.asset_trail(x).disposal_true_up, Decimal::new(-50_000, 2));
    assert_eq!(
        ledger_gui::pages::assets::unposted(&b, x).unwrap(),
        Decimal::ZERO
    );

    let (dir, _data, mut s, x, (cash, gain)) = asset_x("disposed-catch-up");
    let mut mar = 0;
    for m in 1..=6 {
        let r = s.run_depreciation(x, &format!("2026-{m:02}")).unwrap();
        if m == 3 {
            mar = r.entry_id.unwrap();
        }
    }
    s.reverse_entry(mar, d(2026, 6, 20), "reversed").unwrap();
    let p = s.dispose_preview(x, d(2026, 6, 15), usd(700_000)).unwrap();
    assert_eq!(p.catch_up_depreciation, Decimal::new(50_000, 2));
    s.dispose_asset(x, d(2026, 6, 15), usd(700_000), cash, gain)
        .unwrap();
    drop(s);
    let b = load(&dir.0, today());
    assert_eq!(b.asset_trail(x).posted(), Decimal::new(550_000, 2));
    let st: Vec<MonthStatus> = schedule(&b, x)
        .unwrap()
        .into_iter()
        .map(|(_, s)| s)
        .collect();
    assert_eq!(st[2], MonthStatus::Covered);
    assert!(!st.contains(&MonthStatus::Due));
}

/// A command for [`prop_the_viewer_reads_an_asset_as_the_store_does`].
#[derive(Clone, Copy, Debug)]
enum AssetOp {
    Run(u32),
    Reverse(usize, NaiveDate),
    Dispose(NaiveDate),
}

/// The entries asset commands made, in journal order: runs (the disposal's
/// final depreciation too), reversals, disposals.
fn asset_entries(data: &Path) -> Vec<EntryId> {
    let text = std::fs::read_to_string(data.join(ledger_store::JOURNAL_FILE)).unwrap();
    text.lines()
        .filter_map(|l| {
            let v: serde_json::Value = serde_json::from_str(l).unwrap();
            let p = &v["payload"];
            match v["type"].as_str()? {
                "asset.depreciation_posted" | "asset.disposed" => p["entry_id"].as_u64(),
                "entry.reversed" => p["id"].as_u64(),
                _ => None,
            }
        })
        .collect()
}

/// An account's balance (debit positive) at the end of a day.
fn balance_on(store: &Store, account: u64, on: NaiveDate) -> Decimal {
    let code = &store.ledger().chart().get(account).unwrap().code;
    ledger_core::reports::trial_balance_as_of(store.ledger(), on)
        .lines
        .iter()
        .find(|l| &l.code == code)
        .map_or(Decimal::ZERO, |l| l.debit - l.credit)
}

fn arb_asset_op() -> impl Strategy<Value = AssetOp> {
    let date = (1u32..=6, 1u32..=31).prop_map(|(m, day)| {
        NaiveDate::from_ymd_opt(2026, m, day).unwrap_or_else(|| {
            ledger_core::schedules::month_end(NaiveDate::from_ymd_opt(2026, m, 1).unwrap())
        })
    });
    prop_oneof![
        4 => (1u32..=6).prop_map(AssetOp::Run),
        4 => (0usize..8, date.clone()).prop_map(|(n, on)| AssetOp::Reverse(n, on)),
        1 => date.prop_map(AssetOp::Dispose),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// P3 review round 2: whatever runs, reversals, re-runs,
    /// re-instatements and disposals the books hold, the viewer reads the
    /// asset as the store does. On July 15: what it says the books carry
    /// is the accumulated depreciation account (a disposed asset's, what
    /// its disposal took: the schedule through its day); each finished
    /// month is Posted by the entry a run of it says is already posted,
    /// Due exactly when a run of it would post something, and Covered
    /// otherwise; and what it says is unposted is what the schedule says
    /// through June less what the books carry then.
    #[test]
    fn prop_the_viewer_reads_an_asset_as_the_store_does(
        ops in prop::collection::vec(arb_asset_op(), 1..10)
    ) {
        let (dir, data, mut s, x, (cash, gain)) = asset_x("viewer-orders");
        for op in &ops {
            let _ = match *op {
                AssetOp::Run(m) => s.run_depreciation(x, &format!("2026-{m:02}")).map(|_| ()),
                AssetOp::Reverse(n, on) => match asset_entries(&data).get(n) {
                    Some(&e) => s.reverse_entry(e, on, "reversed").map(|_| ()),
                    None => Ok(()),
                },
                AssetOp::Dispose(on) => s.dispose_asset(x, on, usd(100_000), cash, gain).map(|_| ()),
            };
        }
        let a = s.ledger().get_asset(x).unwrap().clone();
        let accum = a.accum_depr_account;
        let expense = a.expense_account;
        drop(s);
        let today = d(2026, 7, 15);
        let b = load(&dir.0, today);
        let trail = b.asset_trail(x);
        let s = Store::open(&data).unwrap();
        let disposed = a.status != ledger_core::AssetStatus::Active;
        if disposed {
            prop_assert_eq!(
                trail.posted(),
                ledger_core::accumulated_scheduled_through(&a, NaiveDate::MAX).unwrap()
            );
            prop_assert_eq!(balance_on(&s, accum, NaiveDate::MAX), Decimal::ZERO);
        } else {
            prop_assert_eq!(trail.posted(), -balance_on(&s, accum, NaiveDate::MAX));
            let june = d(2026, 6, 30);
            let owed = ledger_core::accumulated_scheduled_through(&a, june).unwrap()
                - balance_on(&s, expense, june);
            prop_assert_eq!(
                ledger_gui::pages::assets::unposted(&b, x).unwrap(),
                owed.max(Decimal::ZERO)
            );
        }
        drop(s);
        let lines = schedule(&b, x).unwrap();
        for (line, st) in lines.iter().filter(|(l, _)| l.period.as_str() < "2026-07") {
            // A run of the month, on a copy of the books.
            let copy = TempDir::new("viewer-orders-copy");
            let to = copy.0.join("books-data");
            std::fs::create_dir_all(&to).unwrap();
            std::fs::copy(data.join(ledger_store::JOURNAL_FILE), to.join(ledger_store::JOURNAL_FILE)).unwrap();
            let mut c = Store::open(&to).unwrap();
            let run = c.run_depreciation(x, &line.period);
            let want = match run {
                Ok(r) if r.already_posted => MonthStatus::Posted(r.entry_id.unwrap()),
                Ok(r) if r.entry_id.is_some() => MonthStatus::Due,
                Ok(_) => MonthStatus::Covered,
                Err(_) => {
                    prop_assert!(disposed, "{} refused: {:?}", line.period, run);
                    MonthStatus::Covered
                }
            };
            prop_assert_eq!(*st, want, "{} ({:?})", line.period, ops);
        }
    }
}

/// P3 review round 2, in the viewer: a run re-instated after its month was
/// run again and that second run reversed is the month's run, as the store
/// reads it. Hand check: January run (A), reversed on Jan 31; run again
/// (A2), reversed on Jan 31; A's reversal reversed on Jan 31: A stands.
/// January is Posted(A), the books carry 1,000.00, and nothing is unposted
/// but February through June (5,000.00). Before this review the viewer read
/// January's latest run (A2, reversed): January Due, 0.00 carried.
#[test]
fn the_viewer_counts_a_run_reinstated_after_its_rerun_was_reversed() {
    let (dir, _data, mut s, x, _) = asset_x("reinstated-page");
    let jan31 = d(2026, 1, 31);
    let a = s.run_depreciation(x, "2026-01").unwrap().entry_id.unwrap();
    let b_rev = s.reverse_entry(a, jan31, "first doubt").unwrap();
    let a2 = s.run_depreciation(x, "2026-01").unwrap().entry_id.unwrap();
    s.reverse_entry(a2, jan31, "second doubt").unwrap();
    s.reverse_entry(b_rev, jan31, "the first was right")
        .unwrap();
    drop(s);
    let b = load(&dir.0, d(2026, 7, 15));
    assert_eq!(b.asset_trail(x).posted(), Decimal::new(100_000, 2));
    let st = schedule(&b, x).unwrap();
    assert_eq!(st[0].1, MonthStatus::Posted(a));
    assert_eq!(
        ledger_gui::pages::assets::unposted(&b, x).unwrap(),
        Decimal::new(500_000, 2)
    );
}
