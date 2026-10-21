//! The format upgrade (plan/served.md 3.9, D-39; rules.md B-27): books an
//! older wanderbooks wrote open on their old journal format, refuse only
//! what needs the new one, and move to this version's format in one
//! journaled step; books this version makes are in its format from their
//! first lines.
//!
//! ```text
//! GOAL: no book is moved past what its owner's older version reads
//!       without the owner's go-ahead, and an upgrade changes nothing of the
//!       books but what may be written from then on.
//! SOURCE: plan/served.md 3.9 ("Format changes are the server's gate, not
//!         the updater's"); D-39; rules.md B-27.
//! INVARIANTS:
//!   1. New books declare this version's format, every feature it knows,
//!      as they are created: they wait for no upgrade.
//!   2. Older books open, in the writer and the viewer, without a byte of
//!      their journal changing, and say what upgrade they wait for.
//!   3. Before the upgrade a write that needs the new format is refused,
//!      journaling nothing; a write that needs nothing new works.
//!   4. The upgrade is one journal line declaring the whole format; asked
//!      again, it journals nothing.
//!   5. Every book an earlier version wrote (tests/fixtures/old-books)
//!      upgrades, verifies as before and replays to exactly the figures
//!      that version computed, in the writer and the viewer.
//!   6. Books declaring a feature this version doesn't know are still
//!      refused as newer: the upgrade never weakens the gate.
//! ```

use chrono::NaiveDate;
use ledger_core::settings::{Change, Source};
use ledger_core::{AccountType, Currency, Money, Side};
use ledger_store::format::{FORMAT_SET, current};
use ledger_store::{JOURNAL_FILE, JournalFormat, JournalWriter, Snapshot, Store, StoreError};
use rust_decimal::Decimal;
use serde_json::json;
use std::path::{Path, PathBuf};

/// A test's dir, removed when the test ends, pass or panic.
struct TestDir(PathBuf);

impl std::ops::Deref for TestDir {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn test_dir(name: &str) -> TestDir {
    let dir = std::env::temp_dir().join(format!(
        "wanderbooks-format-upgrade-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    TestDir(dir)
}

fn journal(dir: &Path) -> Vec<u8> {
    std::fs::read(dir.join(JOURNAL_FILE)).unwrap()
}

fn lines(dir: &Path) -> Vec<serde_json::Value> {
    String::from_utf8(journal(dir))
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

/// The payload this version's format is declared with.
fn this_versions_declaration() -> serde_json::Value {
    let c = current();
    json!({"format": c.format, "features": c.features})
}

fn usd(n: i64) -> Money {
    Money::new(Decimal::from(n), Currency::USD)
}

/// Books as an older version made them (journal format 1), with a chart
/// and one posted entry.
fn old_books(dir: &Path) -> Store {
    let mut s = Store::create_in_format(dir, Currency::USD, &JournalFormat::default()).unwrap();
    let cash = s
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    let cap = s
        .add_account("3000", "Capital", AccountType::Equity, None, false)
        .unwrap();
    let e = s
        .create_entry(NaiveDate::from_ymd_opt(2026, 3, 1).unwrap(), "Capital in")
        .unwrap();
    s.add_line(e, cash, Side::Debit, usd(500)).unwrap();
    s.add_line(e, cap, Side::Credit, usd(500)).unwrap();
    s.post_entry(e).unwrap();
    s
}

fn a_setting() -> (Vec<Change>, Source) {
    (
        vec![Change {
            key: "company.name".into(),
            value: Some(json!("Fernhill Labs LLC")),
        }],
        Source {
            decided_by: "Avery Quill".into(),
            decided_on: NaiveDate::from_ymd_opt(2026, 10, 20).unwrap(),
            citation: None,
            note: None,
        },
    )
}

/// INVARIANT 1: new books are in this version's format from their second
/// line on, and wait for no upgrade, in the writer and the viewer.
#[test]
fn new_books_are_in_this_versions_format_from_their_creation() {
    let dir = test_dir("new");
    let s = Store::create(&dir, Currency::USD).unwrap();
    assert_eq!(s.journal_format(), &current());
    assert_eq!(s.journal_format().upgrade(), None);
    drop(s);
    let l = lines(&dir);
    assert_eq!(l.len(), 2, "store.created, then the format");
    assert_eq!(l[0]["type"], "store.created");
    assert_eq!(l[1]["type"], FORMAT_SET);
    assert_eq!(l[1]["payload"], this_versions_declaration());
    let s = Store::open(&dir).unwrap();
    assert_eq!(s.journal_format().upgrade(), None);
    s.verify().unwrap();
    let snap = Snapshot::load(&dir).unwrap();
    assert_eq!(snap.journal_format(), &current());
}

/// INVARIANTS 2 to 4: older books open without a byte changing and say
/// what they wait for; a change that needs the new format is refused,
/// journaling nothing, while one that needs nothing new is made; the
/// upgrade is one line, the whole format at once, and asked again does
/// nothing; then the change is made.
#[test]
fn a_change_needing_the_new_format_waits_for_the_upgrade() {
    let dir = test_dir("waits");
    drop(old_books(&dir));
    let before = journal(&dir);
    let mut s = Store::open(&dir).unwrap();
    assert_eq!(journal(&dir), before, "opening wrote nothing");
    let waiting = s.journal_format().upgrade().expect("they wait");
    assert_eq!(waiting.from, JournalFormat::default());
    assert_eq!(waiting.to, current());
    let snap = Snapshot::load(&dir).unwrap();
    assert_eq!(snap.journal_format().upgrade(), Some(waiting.clone()));
    // Refused, saying why, with nothing journaled or applied.
    let (changes, source) = a_setting();
    match s.change_settings(&changes, &source) {
        Err(e @ StoreError::FormatUpgradePending { .. }) => {
            assert!(e.to_string().contains("format_upgrade_pending"), "{e}");
        }
        other => panic!("expected FormatUpgradePending, got {other:?}"),
    }
    assert_eq!(journal(&dir), before);
    assert!(!s.ledger().settings().is_journaled());
    for refused in [
        s.set_convention(&ledger_core::conventions::NewConvention {
            id: "close-ask".into(),
            kind: ledger_core::conventions::ConventionKind::CloseStep,
            title: "Ask about cards".into(),
            body_md: "Ask.".into(),
            structured: None,
            source: source.clone(),
        })
        .map(drop),
        s.raise_open_item(&ledger_core::conventions::NewOpenItem {
            title: "Find the lease".into(),
            detail_md: None,
            due: None,
            source: source.clone(),
        })
        .map(drop),
    ] {
        assert!(
            matches!(refused, Err(StoreError::FormatUpgradePending { .. })),
            "{refused:?}"
        );
    }
    assert_eq!(journal(&dir), before);
    // What needs nothing new is made as ever.
    let supplies = s
        .add_account("6000", "Supplies", AccountType::Expense, None, false)
        .unwrap();
    let cash = s.ledger().chart().find_by_code("1000").unwrap().id;
    let e = s
        .create_entry(NaiveDate::from_ymd_opt(2026, 3, 2).unwrap(), "Pens")
        .unwrap();
    s.add_line(e, supplies, Side::Debit, usd(12)).unwrap();
    s.add_line(e, cash, Side::Credit, usd(12)).unwrap();
    s.post_entry(e).unwrap();
    assert_eq!(s.journal_format(), &JournalFormat::default());
    let figures = ledger_core::reports::snapshot(s.ledger()).unwrap();
    // The upgrade: one line, the whole format.
    let mid = journal(&dir);
    let n = lines(&dir).len();
    let up = s.upgrade_format().unwrap().expect("upgraded");
    assert_eq!(up, waiting);
    assert_eq!(s.journal_format(), &current());
    let after = lines(&dir);
    assert_eq!(after.len(), n + 1, "one line");
    assert_eq!(after[n]["type"], FORMAT_SET);
    assert_eq!(after[n]["payload"], this_versions_declaration());
    assert!(journal(&dir).starts_with(&mid), "nothing before it changed");
    assert_eq!(
        ledger_core::reports::snapshot(s.ledger()).unwrap(),
        figures,
        "the books read as before"
    );
    // Again: nothing.
    let upgraded = journal(&dir);
    assert_eq!(s.upgrade_format().unwrap(), None);
    assert_eq!(journal(&dir), upgraded);
    // Now the change is made.
    s.change_settings(&changes, &source).unwrap();
    assert!(s.ledger().settings().is_journaled());
    s.verify().unwrap();
    drop(s);
    let s = Store::open(&dir).unwrap();
    assert_eq!(s.journal_format(), &current());
    assert_eq!(s.journal_format().upgrade(), None);
    s.verify().unwrap();
    let snap = Snapshot::load(&dir).unwrap();
    assert_eq!(snap.journal_format(), &current());
    assert_eq!(
        lines(&dir)
            .iter()
            .filter(|l| l["type"] == FORMAT_SET)
            .count(),
        1,
        "the format moved exactly once"
    );
}

/// The books an earlier wanderbooks wrote (tests/fixtures/old-books; see
/// tests/store.rs `old_books`).
fn old_book_fixtures() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/old-books");
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    assert!(dirs.len() >= 29, "{} old books", dirs.len());
    dirs
}

/// A fixture's journal and stored files in a fresh dir of their own.
fn fixture_copy(fixture: &Path) -> TestDir {
    fn copy(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for e in std::fs::read_dir(from).unwrap() {
            let e = e.unwrap();
            if e.file_name() == "expected.json" || e.file_name() == "record.json" {
                continue;
            }
            if e.file_type().unwrap().is_dir() {
                copy(&e.path(), &to.join(e.file_name()));
            } else {
                std::fs::copy(e.path(), to.join(e.file_name())).unwrap();
            }
        }
    }
    let name = fixture.file_name().unwrap().to_string_lossy();
    let dir = test_dir(&format!("fixture-{name}"));
    copy(fixture, &dir);
    dir
}

/// What expected.json records of a ledger (tests/store.rs
/// `old_books_figures`, field for field).
fn figures(l: &ledger_core::Ledger) -> serde_json::Value {
    let entries: Vec<_> = (1..l.next_entry_id())
        .filter_map(|id| l.get_entry(id))
        .collect();
    let parties: Vec<_> = (1..l.next_entry_id())
        .map(|id| (id, l.entry_parties(id)))
        .filter(|(_, p)| !p.is_empty())
        .collect();
    json!({
        "snapshot": ledger_core::reports::snapshot(l).map_err(|e| e.to_string()),
        "entries": entries,
        "assets": l.registered_assets().collect::<Vec<_>>(),
        "periods": l.list_periods(),
        "schedules": l.schedules().collect::<Vec<_>>(),
        "recurring": l.recurring_entries().collect::<Vec<_>>(),
        "invoices": l.invoices(),
        "bills": l.bills(),
        "payroll_runs": l.list_payroll_runs(),
        "entry_parties": parties,
    })
}

/// INVARIANTS 2, 4 and 5: every book an earlier version wrote opens
/// without a byte changing, upgrades in one line (or, already in this
/// version's format, waits for nothing), verifies as it did, and replays
/// to exactly the figures that version computed: in the writer, again
/// after reopening, and in the viewer.
#[test]
fn every_book_an_earlier_version_wrote_upgrades_and_still_verifies() {
    let mut upgraded = 0;
    for fixture in old_book_fixtures() {
        let name = fixture.file_name().unwrap().to_string_lossy().to_string();
        let expected: serde_json::Value =
            serde_json::from_slice(&std::fs::read(fixture.join("expected.json")).unwrap()).unwrap();
        let dir = fixture_copy(&fixture);
        let before = journal(&dir);
        let mut s = Store::open(&dir).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(journal(&dir), before, "{name}: opening wrote");
        let waiting = s.journal_format().upgrade();
        let declared = before
            .split(|b| *b == b'\n')
            .filter(|l| !l.is_empty())
            .any(|l| serde_json::from_slice::<serde_json::Value>(l).unwrap()["type"] == FORMAT_SET);
        if !declared {
            assert!(waiting.is_some(), "{name}: format 1 books wait");
        }
        let up = s.upgrade_format().unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(up, waiting, "{name}");
        let after = journal(&dir);
        assert!(after.starts_with(&before), "{name}: the old lines changed");
        let added = after[before.len()..]
            .split(|b| *b == b'\n')
            .filter(|l| !l.is_empty())
            .count();
        assert_eq!(added, usize::from(up.is_some()), "{name}: one line");
        if up.is_some() {
            upgraded += 1;
        }
        assert_eq!(s.journal_format(), &current(), "{name}");
        let verify = s.verify().map(|_| ()).map_err(|e| e.to_string());
        assert_eq!(json!(verify), expected["verify"], "{name}: verify");
        let mut got = figures(s.ledger());
        got["verify"] = expected["verify"].clone();
        assert_eq!(got, expected, "{name}: the writer's figures");
        drop(s);
        let s = Store::open(&dir).unwrap_or_else(|e| panic!("{name}: reopen: {e}"));
        assert_eq!(s.journal_format().upgrade(), None, "{name}");
        let mut got = figures(s.ledger());
        got["verify"] = expected["verify"].clone();
        assert_eq!(got, expected, "{name}: reopened");
        drop(s);
        let snap = Snapshot::load(&dir).unwrap_or_else(|e| panic!("{name}: viewer: {e}"));
        assert_eq!(snap.journal_format(), &current(), "{name}");
        let mut got = figures(snap.ledger());
        got["verify"] = expected["verify"].clone();
        assert_eq!(got, expected, "{name}: the viewer's figures");
    }
    assert!(upgraded >= 28, "only {upgraded} fixtures were format 1");
}

/// INVARIANT 6: books that declare a feature this version doesn't know
/// (a newer version wrote them) are refused as newer, in the writer and
/// the viewer, and never touched: there is nothing to upgrade.
#[test]
fn books_declaring_a_feature_this_version_doesnt_know_stay_refused_as_newer() {
    let dir = test_dir("newer");
    drop(old_books(&dir));
    {
        let mut w = JournalWriter::open(&dir.join(JOURNAL_FILE)).unwrap();
        w.append(
            FORMAT_SET,
            json!({"format": 2, "features": ["from_a_newer_version"]}),
        )
        .unwrap();
        w.append("account.added", json!({})).unwrap();
    }
    let before = journal(&dir);
    for refused in [Store::open(&dir).map(drop), Snapshot::load(&dir).map(drop)] {
        match refused {
            Err(StoreError::NewerJournal { need, .. }) => {
                assert_eq!(need, "journal features from_a_newer_version")
            }
            other => panic!("expected NewerJournal, got {other:?}"),
        }
    }
    assert_eq!(journal(&dir), before);
}
