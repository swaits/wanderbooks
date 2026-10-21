//! Books an earlier wanderbooks wrote open in the viewer, and every page
//! renders over them, never a panic (rules.md B-28: posted is forever; a
//! figure the books can't compute is said, never guessed).
//!
//! ```text
//! GOAL: the viewer shows books written under older rules, whatever they
//!       hold that the commands now refuse.
//! SOURCE: rules.md B-28; DECISION (P0 review round 4: replay accepts what
//!         main 188c90ea accepted, and derived figures refuse rather than
//!         panic).
//! INVARIANTS:
//!   1. Every journal in ledger-store's tests/fixtures/old-books (written
//!      through main's Store API: dates before 0001 and past 9999 out to
//!      chrono's edges, draft and recurring lines past the line limit up
//!      to Decimal's largest, shares and contributor bases near Decimal's
//!      largest, sub-cent document and payroll lines, statement balances
//!      and an exchange rate past any sense), and one written under the
//!      rules before 2026-10-13 (a draft line on an account that doesn't
//!      exist, a child account of another type, periods that partly
//!      overlap), loads as the viewer loads it (data::load_once).
//!   2. Over each, every page renders: every route the books have ids
//!      for, every report kind over this and last year, all time, and
//!      custom ranges out to chrono's edges, split every way and compared
//!      every way, the
//!      party pages over every period, the asset register and account
//!      registers on chrono's first and last days.
//! ```

use chrono::NaiveDate;
use egui_kittest::Harness;
use ledger_gui::books::{Books, PartyPeriod};
use ledger_gui::nav::{ReportKind, Route};
use ledger_gui::pages::accounts::RegisterRange;
use ledger_gui::pages::reports::{Columns, Compare, Range};
use ledger_gui::{App, Source, data};
use serde_json::json;
use std::path::{Path, PathBuf};

const TODAY: (i32, u32, u32) = (2026, 10, 17);

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

/// A scratch directory, removed when dropped.
struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn temp(tag: &str) -> TempDir {
    let dir = std::env::temp_dir().join(format!("ledger-gui-old-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("books-data")).unwrap();
    TempDir(dir)
}

/// Every file of a fixture's books (journal and blobs, never the expected
/// figures beside them) copied into `to`.
fn copy_books(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let name = e.file_name();
        if name == "expected.json" {
            continue;
        }
        if e.file_type().unwrap().is_dir() {
            copy_books(&e.path(), &to.join(&name));
        } else {
            std::fs::copy(e.path(), to.join(&name)).unwrap();
        }
    }
}

/// The settings the old books are viewed with: two members (the shares
/// fixtures' parties), the cash account, a fiscal year from July.
const SETTINGS: &str = "[company]\nname = \"Old Books Co\"\nfiscal_year_start = \"07-01\"\ntax_form = \"1065\"\n\
     [[company.members]]\nname = \"Member A\"\nshare = \"50%\"\ncapital_account = \"3000\"\n\
     [[company.members]]\nname = \"Member B\"\nshare = \"50%\"\ncapital_account = \"3001\"\n\
     [books]\nreceipt_threshold = \"25.00\"\ncash_accounts = [\"1000\"]\n";

/// [`SETTINGS`] journaled into the books in `dir`, as the server adopts a
/// settings file, unless they already keep their own (P1's books): the
/// viewer reads settings only from the journal (plan 3.6). As adoption
/// does, the books move to this version's journal format first (an
/// adopted copy is new books of this version).
fn write_settings(dir: &Path) {
    let mut store = ledger_store::Store::open(&dir.join("books-data")).unwrap();
    if store.ledger().settings().is_journaled() {
        return;
    }
    store.upgrade_format().unwrap();
    let import =
        ledger_config::codec::import(SETTINGS, None, store.ledger().settings(), "USD").unwrap();
    store
        .adopt_settings(
            &import.changes,
            &ledger_core::settings::Source {
                decided_by: "the test".into(),
                decided_on: d(2026, 10, 17),
                citation: None,
                note: None,
            },
        )
        .unwrap();
}

/// The fixtures whose index is `shard` mod `of`, one scratch books
/// directory each.
fn old_books(shard: usize, of: usize) -> Vec<(String, TempDir)> {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../ledger-store/tests/fixtures/old-books");
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    assert!(dirs.len() >= 14, "{} old books", dirs.len());
    let mut out: Vec<(String, TempDir)> = dirs
        .iter()
        .enumerate()
        .filter(|(i, _)| i % of == shard)
        .map(|(_, dir)| {
            let name = dir.file_name().unwrap().to_string_lossy().to_string();
            let t = temp(&name);
            copy_books(dir, &t.0.join("books-data"));
            write_settings(&t.0);
            (name, t)
        })
        .collect();
    if shard == of - 1 {
        out.push(("older_rules".into(), older_rules()));
    }
    out
}

/// What a wanderbooks before 2026-10-13 could have written: a child
/// account of another type, partially overlapping periods, a draft line
/// on an account that doesn't exist, a foreign line not rounded to the
/// cent (ledger-store's books_written_under_older_rules_still_open).
fn older_rules() -> TempDir {
    let t = temp("older-rules");
    let data = t.0.join("books-data");
    let mut store = ledger_store::Store::create(&data, ledger_core::Currency::USD).unwrap();
    let cash = store
        .add_account("1000", "Cash", ledger_core::AccountType::Asset, None, true)
        .unwrap();
    drop(store);
    let mut w = ledger_store::JournalWriter::open(&data.join(ledger_store::JOURNAL_FILE)).unwrap();
    for (line_type, payload) in [
        (
            "account.added",
            json!({"id": 2, "code": "1010", "name": "Rent", "account_type": "Expense",
                   "parent": cash, "placeholder": false, "currency": "USD"}),
        ),
        (
            "period.created",
            json!({"id": 1, "name": "2026-01", "start": "2026-01-01", "end": "2026-01-31"}),
        ),
        (
            "period.created",
            json!({"id": 2, "name": "straddle", "start": "2026-01-15", "end": "2026-02-15"}),
        ),
        (
            "entry.created",
            json!({"id": 1, "date": "2026-03-01", "description": "draft"}),
        ),
        (
            "entry.line_added",
            json!({"id": 1, "account": 999, "side": "debit", "amount": "10.00", "currency": "USD"}),
        ),
        (
            "entry.line_added",
            json!({"id": 1, "account": cash, "side": "credit", "amount": "108.34567",
                   "currency": "USD", "foreign_currency": "EUR", "foreign_amount": "100.00",
                   "fx_rate": "1.0834567"}),
        ),
    ] {
        w.append(line_type, payload).unwrap();
    }
    drop(w);
    write_settings(&t.0);
    t
}

/// Every route the books have something for, and one past each.
fn routes(b: &Books) -> Vec<Route> {
    let mut out = vec![
        Route::Overview,
        Route::Transactions,
        Route::Cash,
        Route::Profit,
        Route::Accounts,
        Route::Assets,
        Route::Parties,
        Route::Projects,
        Route::Documents,
        Route::Company,
        Route::Journal,
    ];
    out.extend(ReportKind::ALL.map(Route::Reports));
    out.extend(b.ledger.list_classes().iter().map(|c| Route::Project(c.id)));
    out.extend((1..=b.ledger.next_entry_id()).map(Route::Entry));
    out.extend(b.ledger.chart().all().map(|a| Route::Account(a.id)));
    out.extend(b.ledger.registered_assets().map(|a| Route::Asset(a.id)));
    out.extend(b.ledger.parties().map(|p| Route::Party(p.id)));
    out.extend(b.attachments.iter().map(|a| Route::Document(a.id)));
    out.extend(
        b.reconciliations
            .iter()
            .map(|r| Route::Reconciliation(r.id)),
    );
    out.extend((1..=b.seq()).map(Route::JournalEvent));
    out
}

/// Render `route` (a panic is in its first frame).
fn show(h: &mut Harness<'_, App>, route: Route) {
    h.state_mut().goto(route);
    h.run_steps(1);
}

/// How many shards the old books are rendered in, side by side.
const SHARDS: usize = 8;

/// INVARIANTS 1-2, in eight shards run side by side.
#[test]
fn every_page_renders_over_books_an_earlier_version_wrote_0() {
    every_page_renders(0, SHARDS);
}

#[test]
fn every_page_renders_over_books_an_earlier_version_wrote_1() {
    every_page_renders(1, SHARDS);
}

#[test]
fn every_page_renders_over_books_an_earlier_version_wrote_2() {
    every_page_renders(2, SHARDS);
}

#[test]
fn every_page_renders_over_books_an_earlier_version_wrote_3() {
    every_page_renders(3, SHARDS);
}

#[test]
fn every_page_renders_over_books_an_earlier_version_wrote_4() {
    every_page_renders(4, SHARDS);
}

#[test]
fn every_page_renders_over_books_an_earlier_version_wrote_5() {
    every_page_renders(5, SHARDS);
}

#[test]
fn every_page_renders_over_books_an_earlier_version_wrote_6() {
    every_page_renders(6, SHARDS);
}

#[test]
fn every_page_renders_over_books_an_earlier_version_wrote_7() {
    every_page_renders(7, SHARDS);
}

/// Every page over the old books whose index is `shard` mod `of`.
fn every_page_renders(shard: usize, of: usize) {
    let today = d(TODAY.0, TODAY.1, TODAY.2);
    // OLD_BOOKS_ONLY=<parts of names, comma-separated>: only those books
    // (debugging, and mutation runs).
    let only = std::env::var("OLD_BOOKS_ONLY").ok();
    for (name, dir) in old_books(shard, of) {
        if only
            .as_deref()
            .is_some_and(|o| !o.split(',').any(|part| name.contains(part)))
        {
            continue;
        }
        let source = Source {
            data_dir: dir.0.join("books-data"),
            today: Some(today),
        };
        let books = data::load_once(&source)
            .unwrap_or_else(|e| panic!("{name}: the viewer can't load it: {e}"));
        let routes = routes(&books);
        let parties: Vec<_> = books.ledger.parties().map(|p| p.id).collect();
        let accounts: Vec<_> = books.ledger.chart().all().map(|a| a.id).collect();
        let edges = [
            NaiveDate::MIN,
            d(1, 1, 1),
            today,
            d(9999, 12, 31),
            NaiveDate::MAX,
        ];
        let mut h = Harness::builder()
            .with_size(egui::vec2(1280.0, 800.0))
            .build_eframe(|cc| App::with_books(&cc.egui_ctx, books, false, Route::Overview));
        h.run_steps(2);
        for route in routes {
            show(&mut h, route);
        }
        // Every report over the ranges the books' own dates reach (the
        // other presets are today's, the same in any books), split and
        // compared every way.
        let mut ranges = vec![Range::YearToDate, Range::LastYear, Range::AllTime];
        for (s, e) in [
            (NaiveDate::MIN, NaiveDate::MAX),
            (d(1, 1, 1), d(9999, 12, 31)),
            (d(9999, 1, 1), NaiveDate::MAX),
            (NaiveDate::MIN, d(1, 12, 31)),
        ] {
            ranges.push(Range::Custom(s, e));
        }
        for kind in ReportKind::ALL {
            // The views a report has: split (P&L, balance sheet, cash
            // flow), or compared (those three and the tax summary, on one
            // column), else one.
            let splittable = matches!(
                kind,
                ReportKind::ProfitAndLoss | ReportKind::BalanceSheet | ReportKind::CashFlow
            );
            let comparable = splittable || kind == ReportKind::TaxSummary;
            let mut views = vec![(Columns::Total, Compare::None)];
            if comparable {
                views.push((Columns::Total, Compare::PriorPeriod));
                views.push((Columns::Total, Compare::PriorYear));
            }
            if splittable {
                views.extend(
                    [Columns::Month, Columns::Quarter, Columns::Year].map(|c| (c, Compare::None)),
                );
            }
            for range in &ranges {
                for &(columns, compare) in &views {
                    let st = &mut h.state_mut().state_mut().reports;
                    st.range = *range;
                    st.columns = columns;
                    st.compare = compare;
                    show(&mut h, Route::Reports(kind));
                }
            }
        }
        // Every party over every period.
        for period in [
            PartyPeriod::ThisYear,
            PartyPeriod::LastYear,
            PartyPeriod::AllTime,
            PartyPeriod::Year(NaiveDate::MIN.format("%Y").to_string().parse().unwrap()),
            PartyPeriod::Year(0),
            PartyPeriod::Year(10000),
            PartyPeriod::Year(262142),
        ] {
            h.state_mut().state_mut().parties.period = period;
            show(&mut h, Route::Parties);
            for &p in &parties {
                show(&mut h, Route::Party(p));
            }
        }
        // The asset register and account registers at chrono's edges.
        for at in edges {
            h.state_mut().state_mut().assets.as_of = Some(at);
            show(&mut h, Route::Assets);
            h.state_mut().state_mut().accounts.range = RegisterRange::Dates(NaiveDate::MIN, at);
            for &a in &accounts {
                show(&mut h, Route::Account(a));
            }
        }
        eprintln!("{name}: every page rendered");
    }
}

/// One fixture's books, staged as [`old_books`] stages them.
fn stage(name: &str) -> TempDir {
    let from = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../ledger-store/tests/fixtures/old-books")
        .join(name);
    let t = temp(&format!("said-{name}"));
    copy_books(&from, &t.0.join("books-data"));
    write_settings(&t.0);
    t
}

fn load(dir: &TempDir) -> Books {
    data::load_once(&Source {
        data_dir: dir.0.join("books-data"),
        today: Some(d(TODAY.0, TODAY.1, TODAY.2)),
    })
    .unwrap()
}

/// A figure the books can't compute is said, never a number: a draft of
/// Decimal's largest lines has no total; a schedule past chrono's last day
/// and a register whose contributor bases are past exact arithmetic are
/// each a Problem in "Needs attention".
#[test]
fn figures_that_cant_be_computed_are_said() {
    let dir = stage("x_draft_max_lines");
    let b = load(&dir);
    let draft = b.entry(1).unwrap();
    assert_eq!(draft.amount, None);
    assert_eq!(
        ledger_gui::fmt::dollars_or(draft.amount),
        "too large to compute"
    );
    assert!(
        b.attention
            .iter()
            .any(|a| a.detail.contains("too large to compute")),
        "{:?}",
        b.attention
    );

    let dir = stage("x_schedule_chrono_end");
    let b = load(&dir);
    let item = b
        .attention
        .iter()
        .find(|a| a.title.starts_with("Amortization schedule"))
        .unwrap_or_else(|| panic!("{:?}", b.attention));
    assert_eq!(item.severity, ledger_gui::books::Severity::Problem);
    assert!(item.title.ends_with("can't be computed"), "{}", item.title);

    let dir = stage("x_tax_basis_max");
    let b = load(&dir);
    assert!(matches!(
        b.asset_register(b.today),
        Err(ledger_core::LedgerError::TooLargeToCompute(_))
    ));
    assert!(
        b.attention
            .iter()
            .any(|a| a.title == "The fixed asset register can't be computed"),
        "{:?}",
        b.attention
    );
}

/// Books that keep their own settings (written under P1) show them, as
/// the server reads them (plan/served.md P1, Accept; 3.6): never the
/// settings the other old books are viewed with (another name, a July
/// fiscal year, a $25 receipt rule).
#[test]
fn books_that_keep_their_settings_show_them() {
    let dir = stage("p1_record");
    let b = load(&dir);
    assert_eq!(b.settings.company.name, "Fixture Labs LLC");
    assert_eq!(b.settings.company.fiscal_year_start, (1, 1));
    assert_eq!(b.receipt_threshold, rust_decimal::Decimal::ZERO);
    assert_eq!(b.settings.timezone, None, "the fixture cleared it");
    assert_eq!(b.settings.company.members.len(), 2);
    assert!(
        b.settings
            .company
            .profile
            .iter()
            .any(|(k, v)| k == "ein" && v == "98-7814321"),
        "the viewer is the human's: the EIN in full"
    );
    // Every new event has its own sentence in the journal page.
    for line in b.journal.iter().filter(|l| {
        [
            "settings.set",
            "convention.set",
            "convention.retired",
            "decision.recorded",
            "open_item.raised",
            "open_item.resolved",
            "entry.approved",
        ]
        .contains(&l.line_type.as_str())
    }) {
        let e = ledger_gui::pages::journal::describe(&b, line).expect("described");
        assert_ne!(e.text, line.line_type, "{} has no sentence", line.line_type);
    }
}

/// Books that keep no settings in their journal are read as their server
/// reads them (plan 3.6): an older books dir from its settings file; a
/// book of the data root never from a file, so with the defaults.
#[test]
fn books_that_keep_no_settings_read_their_file_only_outside_a_root() {
    let from = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../ledger-store/tests/fixtures/old-books")
        .join("shares_big");
    // An older books dir: its server reads wanderbooks.toml, so the viewer
    // does too.
    let t = temp("said-file");
    copy_books(&from, &t.0.join("books-data"));
    std::fs::write(t.0.join("wanderbooks.toml"), SETTINGS).unwrap();
    let b = load(&t);
    assert_eq!(b.settings.company.name, "Old Books Co");
    assert!(!b.settings.company.members.is_empty());
    // The same books in a data root: no file is read, whatever lies
    // beside them; they show the defaults until their settings are in
    // their journal.
    let t = temp("said-defaults");
    std::fs::write(t.0.join(ledger_store::root::MARKER_FILE), b"").unwrap();
    copy_books(&from, &t.0.join("books-data"));
    std::fs::write(t.0.join("wanderbooks.toml"), SETTINGS).unwrap();
    let b = load(&t);
    assert_eq!(
        b.settings,
        ledger_config::codec::books_settings(&ledger_core::settings::Settings::default(), "USD")
    );
}

/// An older books dir with no settings file but a `config.json`: its
/// receipt rule is the server's, from that file (the regression round 2
/// of slice 6's review found).
#[test]
fn an_older_books_dirs_config_json_sets_its_receipt_rule() {
    let from = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../ledger-store/tests/fixtures/old-books")
        .join("shares_big");
    let t = temp("said-config");
    copy_books(&from, &t.0.join("books-data"));
    std::fs::write(
        t.0.join("books-data/config.json"),
        r#"{"receipt_policy": {"require_receipts_above": "0.00"}}"#,
    )
    .unwrap();
    assert_eq!(load(&t).receipt_threshold, rust_decimal::Decimal::ZERO);
    // In a data root, no file: the books' own rule (the default).
    let t = temp("said-config-root");
    std::fs::write(t.0.join(ledger_store::root::MARKER_FILE), b"").unwrap();
    copy_books(&from, &t.0.join("books-data"));
    std::fs::write(
        t.0.join("books-data/config.json"),
        r#"{"receipt_policy": {"require_receipts_above": "0.00"}}"#,
    )
    .unwrap();
    assert_ne!(load(&t).receipt_threshold, rust_decimal::Decimal::ZERO);
}

/// rules.md B-37: the classes an earlier version wrote read as projects
/// with the default facts (active, kind not recorded), listed by name,
/// their figures ledger-core's.
#[test]
fn old_books_classes_read_as_projects() {
    let mut seen = 0;
    // Copies of their own: the shards above copy the same fixtures under
    // the fixtures' names, side by side.
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../ledger-store/tests/fixtures/old-books");
    let mut fixtures: Vec<PathBuf> = std::fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir())
        .collect();
    fixtures.sort();
    let copies = fixtures.iter().map(|f| {
        let name = f.file_name().unwrap().to_string_lossy().to_string();
        let t = temp(&format!("projects-{name}"));
        copy_books(f, &t.0.join("books-data"));
        write_settings(&t.0);
        (name, t)
    });
    for (name, dir) in copies {
        let b = data::load_once(&ledger_gui::Source {
            data_dir: dir.0.join("books-data"),
            today: Some(d(2026, 10, 12)),
        })
        .unwrap();
        let listed = ledger_gui::pages::projects::listed(&b);
        assert_eq!(listed.len(), b.ledger.list_classes().len(), "{name}");
        for p in &listed {
            seen += 1;
            assert_eq!(p.facts, ledger_core::ProjectFacts::default(), "{name}");
            assert_eq!(
                p.to_date,
                ledger_core::project_to_date(&b.ledger, p.id, b.today).ok(),
                "{name}"
            );
        }
        let names: Vec<String> = listed.iter().map(|p| p.name.to_lowercase()).collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted, "{name}: alphabetical");
    }
    // No fixture holds a class: books with the lines an earlier version
    // wrote for one (class.added, a line tagged with it), and nothing more.
    let t = temp("old-class");
    let data = t.0.join("books-data");
    let mut store = ledger_store::Store::create(&data, ledger_core::Currency::USD).unwrap();
    let cash = store
        .add_account("1000", "Cash", ledger_core::AccountType::Asset, None, false)
        .unwrap();
    let rev = store
        .add_account(
            "4000",
            "Sales",
            ledger_core::AccountType::Revenue,
            None,
            false,
        )
        .unwrap();
    let class = store.add_class("Harbor app").unwrap();
    let e = store.create_entry(d(2026, 3, 1), "sale").unwrap();
    let usd =
        |c| ledger_core::Money::new(rust_decimal::Decimal::new(c, 2), ledger_core::Currency::USD);
    store
        .add_line(e, cash, ledger_core::Side::Debit, usd(5000))
        .unwrap();
    store
        .add_line(e, rev, ledger_core::Side::Credit, usd(5000))
        .unwrap();
    store.set_line_class(e, 1, Some(class)).unwrap();
    store.post_entry(e).unwrap();
    let journal = std::fs::read_to_string(data.join(ledger_store::JOURNAL_FILE)).unwrap();
    assert!(!journal.contains("project.set"), "nothing new was written");
    drop(store);
    write_settings(&t.0);
    let b = data::load_once(&ledger_gui::Source {
        data_dir: data,
        today: Some(d(2026, 10, 12)),
    })
    .unwrap();
    let listed = ledger_gui::pages::projects::listed(&b);
    assert_eq!(listed.len(), 1);
    seen += 1;
    assert_eq!(listed[0].name, "Harbor app");
    assert_eq!(listed[0].facts, ledger_core::ProjectFacts::default());
    let t = listed[0].to_date.as_ref().unwrap();
    assert_eq!(t.revenue, rust_decimal::Decimal::new(5000, 2));
    assert_eq!(t.unrecovered_cost, None);
    assert!(seen > 0);
}
