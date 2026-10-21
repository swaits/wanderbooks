//! Spec-derived store tests for asset tags and serial numbers (rules.md
//! B-24): journaling (`asset.tagged` {asset_id, tag, previous_tag},
//! `asset.serial_set` {asset_id, serial | null}), replay on reopen, the v9
//! SQLite projection agreeing with the journal (`Store::verify`), books
//! written before tags existed, refusals journaling nothing, and corrupt
//! `asset.serial_set` lines refused on open; and tax basis (B-25:
//! `asset.basis_set`, pre-P3 `asset.tax_basis_set` {asset_id, rule,
//! contributor_basis | null} replayed, the v10 schema and v13's, corrupt
//! and newer payloads). A database at an older schema under this
//! version's file name is rebuilt, never migrated, and P2's own file is
//! left as it is (each schema keeps a file of its own: rules.md B-27,
//! tests/schema_files.rs).
//!
//! Expectations come from the doc comments on `Store::set_asset_tag`,
//! `Store::set_asset_serial`, `Store::open`, `Store::verify`,
//! `Ledger::set_asset_tag` and the schema v9 migration comment.

use chrono::NaiveDate;
use ledger_core::{AccountType, AssetId, Currency, LedgerError, Money, NewAsset};
use ledger_store::journal::read_and_verify_journal;
use ledger_store::{JOURNAL_FILE, JournalWriter, Store, StoreError, db_file, db_file_for};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// A test's dir, removed when the test ends, pass or panic.
struct TestDir(PathBuf);

impl std::ops::Deref for TestDir {
    type Target = std::path::Path;
    fn deref(&self) -> &std::path::Path {
        &self.0
    }
}

impl AsRef<std::path::Path> for TestDir {
    fn as_ref(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn test_dir(name: &str) -> TestDir {
    let dir = std::env::temp_dir().join(format!("wb-asset-tags-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    TestDir(dir)
}

fn usd(amount: &str) -> Money {
    Money::new(amount.parse().unwrap(), Currency::USD)
}

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

/// The journal's (type, payload) pairs, verified.
fn journal(dir: &Path) -> Vec<(String, Value)> {
    read_and_verify_journal(&dir.join(JOURNAL_FILE))
        .unwrap()
        .into_iter()
        .map(|l| (l.line_type, l.payload))
        .collect()
}

struct Accts {
    cost: u64,
    accum: u64,
    expense: u64,
    bank: u64,
    gain: u64,
}

fn accounts(store: &mut Store) -> Accts {
    let mut add =
        |code: &str, name: &str, ty| store.add_account(code, name, ty, None, false).unwrap();
    Accts {
        cost: add("1500", "Equipment", AccountType::Asset),
        accum: add("1501", "Accumulated Depreciation", AccountType::Asset),
        expense: add("6200", "Depreciation Expense", AccountType::Expense),
        bank: add("1000", "Bank", AccountType::Asset),
        gain: add("4900", "Gain on Disposal", AccountType::Revenue),
    }
}

fn laptop(store: &mut Store, a: &Accts, name: &str) -> AssetId {
    store
        .register_asset(NewAsset {
            description: name.to_string(),
            in_service_date: date(2026, 1, 1),
            cost: usd("1200.00"),
            salvage_value: usd("0.00"),
            useful_life_months: 12,
            cost_account: a.cost,
            accum_depr_account: a.accum,
            expense_account: a.expense,
        })
        .unwrap()
}

/// A store with two untagged laptops.
fn fixture(name: &str) -> (TestDir, Store, Accts, AssetId, AssetId) {
    let dir = test_dir(name);
    let mut s = Store::create(&dir, Currency::USD).unwrap();
    let a = accounts(&mut s);
    let x = laptop(&mut s, &a, "Laptop A");
    let y = laptop(&mut s, &a, "Laptop B");
    (dir, s, a, x, y)
}

fn tag_of(s: &Store, id: AssetId) -> Option<String> {
    s.ledger().get_asset(id).unwrap().tag.clone()
}

fn serial_of(s: &Store, id: AssetId) -> Option<String> {
    s.ledger().get_asset(id).unwrap().serial.clone()
}

// ------------------------------------------------- journaling

/// Store::set_asset_tag: journaled as `asset.tagged`, with the tag it had.
#[test]
fn tagging_journals_asset_tagged_with_the_previous_tag() {
    let (dir, mut s, _, x, _) = fixture("journal-tag");
    s.set_asset_tag(x, "FL-0001").unwrap();
    s.set_asset_tag(x, "FL-0007").unwrap();
    let j = journal(&dir);
    let tagged: Vec<&Value> = j
        .iter()
        .filter(|(t, _)| t == "asset.tagged")
        .map(|(_, p)| p)
        .collect();
    assert_eq!(tagged.len(), 2);
    assert_eq!(
        tagged[0],
        &json!({"asset_id": x, "tag": "FL-0001", "previous_tag": null})
    );
    assert_eq!(
        tagged[1],
        &json!({"asset_id": x, "tag": "FL-0007", "previous_tag": "FL-0001"})
    );
    assert_eq!(tag_of(&s, x).as_deref(), Some("FL-0007"));
    s.verify().unwrap();
}

/// Store::set_asset_serial: journaled as `asset.serial_set`, serial or null.
#[test]
fn serials_journal_asset_serial_set() {
    let (dir, mut s, _, x, _) = fixture("journal-serial");
    s.set_asset_serial(x, Some("C02XG0FDJGH5")).unwrap();
    s.set_asset_serial(x, None).unwrap();
    let j = journal(&dir);
    let set: Vec<&Value> = j
        .iter()
        .filter(|(t, _)| t == "asset.serial_set")
        .map(|(_, p)| p)
        .collect();
    assert_eq!(
        set,
        vec![
            &json!({"asset_id": x, "serial": "C02XG0FDJGH5"}),
            &json!({"asset_id": x, "serial": null}),
        ]
    );
    assert_eq!(serial_of(&s, x), None);
    s.verify().unwrap();
}

// ------------------------------------------------- refusals journal nothing

/// Ledger::set_asset_tag INVARIANT 3 through the store: a refusal changes
/// nothing, so nothing is journaled.
#[test]
fn a_refused_tag_journals_nothing() {
    let (dir, mut s, _, x, y) = fixture("refused-tag");
    s.set_asset_tag(x, "FL-0001").unwrap();
    s.set_asset_tag(x, "FL-0002").unwrap();
    let lines = journal(&dir).len();
    let before = s.ledger().clone();
    // Taken (current and released), wrong form, unknown asset.
    assert!(matches!(
        s.set_asset_tag(y, "fl-0002"),
        Err(StoreError::Ledger(LedgerError::AssetTagTaken { .. }))
    ));
    assert!(matches!(
        s.set_asset_tag(y, "FL-0001"),
        Err(StoreError::Ledger(LedgerError::AssetTagTaken { .. }))
    ));
    assert!(matches!(
        s.set_asset_tag(y, "no spaces"),
        Err(StoreError::Ledger(LedgerError::InvalidAssetTag { .. }))
    ));
    assert!(s.set_asset_tag(y + 99, "FL-0050").is_err());
    assert_eq!(journal(&dir).len(), lines, "nothing journaled");
    assert_eq!(s.ledger(), &before);
    s.verify().unwrap();
    // And nothing reappears on reopen.
    drop(s);
    let s = Store::open(&dir).unwrap();
    assert_eq!(s.ledger(), &before);
    s.verify().unwrap();
}

#[test]
fn a_refused_serial_journals_nothing() {
    let (dir, mut s, _, x, _) = fixture("refused-serial");
    s.set_asset_serial(x, Some("SN-1")).unwrap();
    let lines = journal(&dir).len();
    let before = s.ledger().clone();
    for bad in ["", " SN", "SN\u{1}"] {
        assert!(matches!(
            s.set_asset_serial(x, Some(bad)),
            Err(StoreError::Ledger(LedgerError::InvalidSerial(_)))
        ));
    }
    assert!(s.set_asset_serial(x + 99, Some("SN-2")).is_err());
    assert_eq!(journal(&dir).len(), lines);
    assert_eq!(s.ledger(), &before);
    s.verify().unwrap();
}

// ------------------------------------------------- replay

/// Tags (with their history) and serials survive a reopen, both with the
/// database present and rebuilt from the journal alone (Store::open
/// SCENARIO 5), and verify passes each time.
#[test]
fn tags_and_serials_replay_on_reopen() {
    let (dir, mut s, a, x, y) = fixture("replay");
    s.set_asset_tag(x, "FL-0001").unwrap();
    s.set_asset_tag(y, "FL-0002").unwrap();
    s.set_asset_tag(x, "Desk/7").unwrap(); // releases FL-0001 (still reserved)
    s.set_asset_tag(x, "FL-0001").unwrap(); // takes it back
    s.set_asset_tag(x, "FL-0003").unwrap();
    s.set_asset_serial(x, Some("SN 123")).unwrap();
    s.set_asset_serial(y, Some("first")).unwrap();
    s.set_asset_serial(y, Some("second")).unwrap();
    let z = laptop(&mut s, &a, "Laptop C");
    s.set_asset_serial(z, Some("gone")).unwrap();
    s.set_asset_serial(z, None).unwrap();
    s.verify().unwrap();
    let live = s.ledger().clone();
    drop(s);

    for rebuild in [false, true] {
        if rebuild {
            std::fs::remove_file(dir.join(db_file())).unwrap();
        }
        let s = Store::open(&dir).unwrap();
        s.verify().unwrap();
        assert_eq!(s.ledger(), &live, "rebuild={rebuild}");
        assert_eq!(tag_of(&s, x).as_deref(), Some("FL-0003"));
        assert_eq!(tag_of(&s, y).as_deref(), Some("FL-0002"));
        assert_eq!(tag_of(&s, z), None);
        assert_eq!(serial_of(&s, x).as_deref(), Some("SN 123"));
        assert_eq!(serial_of(&s, y).as_deref(), Some("second"));
        assert_eq!(serial_of(&s, z), None);
        assert_eq!(s.ledger().asset_by_tag("fl-0003").map(|a| a.id), Some(x));
        assert!(s.ledger().asset_by_tag("Desk/7").is_none());
        // Every tag x ever held stays reserved after replay.
        for t in ["FL-0001", "desk/7", "FL-0003"] {
            assert!(
                matches!(
                    s.ledger().check_asset_tag_for(Some(z), t),
                    Err(LedgerError::AssetTagTaken { asset, .. }) if asset == x
                ),
                "{t} rebuild={rebuild}"
            );
        }
        assert_eq!(s.ledger().next_asset_tag("FL"), "FL-0004");
        // x may still take back what it held.
        s.ledger().check_asset_tag_for(Some(x), "Desk/7").unwrap();
        drop(s);
    }
}

/// Tags on a disposed asset stay reserved through the store and a reopen.
#[test]
fn a_disposed_assets_tag_stays_reserved_after_reopen() {
    let (dir, mut s, a, x, y) = fixture("disposed");
    s.set_asset_tag(x, "FL-0001").unwrap();
    s.set_asset_serial(x, Some("SN-X")).unwrap();
    s.dispose_asset(x, date(2026, 3, 31), usd("500.00"), a.bank, a.gain)
        .unwrap();
    s.verify().unwrap();
    drop(s);
    let mut s = Store::open(&dir).unwrap();
    s.verify().unwrap();
    assert_eq!(tag_of(&s, x).as_deref(), Some("FL-0001"));
    assert_eq!(serial_of(&s, x).as_deref(), Some("SN-X"));
    assert!(matches!(
        s.set_asset_tag(y, "FL-0001"),
        Err(StoreError::Ledger(LedgerError::AssetTagTaken { .. }))
    ));
    s.set_asset_tag(y, "FL-0002").unwrap();
    s.verify().unwrap();
}

/// The store's projection keeps up after mutations without a reopen, and
/// stays right across many retags (verify compares every projection table
/// to a fresh projection of the journal).
#[test]
fn verify_passes_after_many_retags_and_serial_changes() {
    let (dir, mut s, _, x, y) = fixture("many");
    for n in 1..=20u32 {
        let id = if n % 2 == 0 { x } else { y };
        s.set_asset_tag(id, &format!("FL-{n:04}")).unwrap();
        s.set_asset_serial(id, Some(&format!("SN {n}"))).unwrap();
        if n % 5 == 0 {
            s.set_asset_serial(id, None).unwrap();
        }
    }
    s.verify().unwrap();
    assert_eq!(s.ledger().next_asset_tag("FL"), "FL-0021");
    drop(s);
    let s = Store::open(&dir).unwrap();
    s.verify().unwrap();
    assert_eq!(s.ledger().next_asset_tag("FL"), "FL-0021");
}

// ------------------------------------------------- older books

/// Books written before tags existed (no asset.tagged lines) open, verify,
/// and then take tags.
#[test]
fn books_written_before_tags_open_and_verify() {
    let (dir, s, _, x, y) = fixture("older");
    drop(s);
    let _ = std::fs::remove_file(dir.join(db_file()));
    let mut s = Store::open(&dir).expect("older books still open");
    s.verify().unwrap();
    assert_eq!(tag_of(&s, x), None);
    assert_eq!(serial_of(&s, y), None);
    s.set_asset_tag(x, "FL-0001").unwrap();
    s.verify().unwrap();
}

/// What an older wanderbooks wrote, by hand (as tests/store.rs
/// `books_written_under_older_rules_still_open` does): an asset.registered
/// line from before tags, appended with JournalWriter, then the database
/// rebuilt from the journal. It opens, verifies, and takes a tag.
#[test]
fn an_older_asset_registered_line_still_replays() {
    let (dir, s, a, x, y) = fixture("older-line");
    drop(s);
    let mut w = JournalWriter::open(&dir.join(JOURNAL_FILE)).unwrap();
    w.append(
        "asset.registered",
        json!({"id": y + 1, "description": "Old printer", "purchase_date": "2026-02-01",
               "cost": {"amount": "600.00", "currency": "USD"},
               "salvage_value": {"amount": "0.00", "currency": "USD"},
               "useful_life_months": 6, "method": "straight_line",
               "cost_account": a.cost, "accum_depr_account": a.accum,
               "expense_account": a.expense}),
    )
    .unwrap();
    drop(w);
    let _ = std::fs::remove_file(dir.join(db_file()));
    let mut s = Store::open(&dir).expect("older books still open");
    s.verify().unwrap();
    let old = y + 1;
    assert_eq!(tag_of(&s, old), None);
    assert_eq!(serial_of(&s, old), None);
    assert_eq!(tag_of(&s, x), None);
    s.set_asset_tag(old, "FL-0001").unwrap();
    s.set_asset_serial(old, Some("HP-1")).unwrap();
    s.verify().unwrap();
    drop(s);
    let s = Store::open(&dir).unwrap();
    s.verify().unwrap();
    assert_eq!(tag_of(&s, old).as_deref(), Some("FL-0001"));
}

/// Whether this version's database was kept aside (and rebuilt) as the
/// books opened: `wanderbooks.v{N}.corrupt.*` beside it.
fn kept_aside(dir: &Path) -> bool {
    let stem = db_file().trim_end_matches(".db").to_string();
    std::fs::read_dir(dir).unwrap().flatten().any(|e| {
        e.file_name()
            .to_string_lossy()
            .starts_with(&format!("{stem}.corrupt."))
    })
}

/// Downgrade the database to schema v8 (no assets.tag / assets.serial, no
/// asset_tags_log, and none of v10's tax basis columns), as an older
/// wanderbooks left it.
fn downgrade_db_to_v8(dir: &Path) {
    downgrade_db_to_v9(dir);
    let conn = rusqlite::Connection::open(dir.join(db_file())).unwrap();
    conn.execute_batch(
        "ALTER TABLE assets DROP COLUMN tag;
         ALTER TABLE assets DROP COLUMN serial;
         ALTER TABLE assets DROP COLUMN acquisition_entry;
         DROP TABLE asset_tags_log;
         UPDATE meta SET value = '8' WHERE key = 'schema_version';",
    )
    .unwrap();
}

/// A database at schema v8 under this version's own file name (what an
/// older wanderbooks once migrated in place) is never migrated now: each
/// schema keeps a file of its own (rules.md B-27), so this one is damaged,
/// kept aside and rebuilt from the journal as the books open, and verifies.
#[test]
fn a_v8_database_migrates_and_verifies() {
    let (dir, s, _, x, y) = fixture("v8");
    drop(s);
    downgrade_db_to_v8(&dir);
    let mut s = Store::open(&dir).expect("a v8 database opens");
    assert!(kept_aside(&dir), "rebuilt, not migrated");
    s.verify().unwrap();
    s.set_asset_tag(x, "FL-0001").unwrap();
    s.set_asset_serial(y, Some("SN-Y")).unwrap();
    s.verify().unwrap();
    drop(s);
    let s = Store::open(&dir).unwrap();
    s.verify().unwrap();
    assert_eq!(tag_of(&s, x).as_deref(), Some("FL-0001"));
    assert_eq!(serial_of(&s, y).as_deref(), Some("SN-Y"));
}

/// A v8 database behind a journal that has tag lines is rebuilt from the
/// whole journal, the tag lines with it (as above), and verifies.
#[test]
fn a_v8_database_behind_a_journal_with_tags_catches_up() {
    let (dir, s, _, x, _) = fixture("v8-behind");
    drop(s);
    downgrade_db_to_v8(&dir);
    let mut w = JournalWriter::open(&dir.join(JOURNAL_FILE)).unwrap();
    w.append(
        "asset.tagged",
        json!({"asset_id": x, "tag": "FL-0001", "previous_tag": null}),
    )
    .unwrap();
    w.append("asset.serial_set", json!({"asset_id": x, "serial": "SN-1"}))
        .unwrap();
    drop(w);
    let s = Store::open(&dir).expect("opens and catches up");
    assert!(kept_aside(&dir), "rebuilt, not migrated");
    s.verify().unwrap();
    assert_eq!(tag_of(&s, x).as_deref(), Some("FL-0001"));
    assert_eq!(serial_of(&s, x).as_deref(), Some("SN-1"));
}

// ------------------------------------------------- corruption

/// A journal is the system of record; an `asset.serial_set` line whose
/// serial is neither a string nor null is corruption, refused on open. The
/// line is appended with `JournalWriter`, so the chain is valid and only
/// the payload is wrong.
#[test]
fn an_asset_serial_set_line_with_a_non_string_serial_is_corrupt() {
    for bad in [json!(12), json!(true), json!(["SN"]), json!({"s": "SN"})] {
        let (dir, s, _, x, _) = fixture("corrupt-serial");
        drop(s);
        let mut w = JournalWriter::open(&dir.join(JOURNAL_FILE)).unwrap();
        w.append("asset.serial_set", json!({"asset_id": x, "serial": bad}))
            .unwrap();
        drop(w);
        match Store::open(&dir) {
            Err(StoreError::JournalCorrupt(_)) => {}
            Err(other) => panic!("serial {bad}: expected JournalCorrupt, got {other:?}"),
            Ok(_) => panic!("serial {bad}: a corrupt asset.serial_set line opened"),
        }
        // Rebuilding from the journal alone refuses it too.
        let _ = std::fs::remove_file(dir.join(db_file()));
        assert!(Store::open(&dir).is_err(), "serial {bad}: rebuild opened");
    }
}

/// A journaled tag the rules refuse (taken by another asset) can't replay:
/// opening refuses rather than give one tag to two assets.
#[test]
fn a_journaled_tag_taken_by_another_asset_is_refused_on_open() {
    let (dir, mut s, _, x, y) = fixture("corrupt-taken");
    s.set_asset_tag(x, "FL-0001").unwrap();
    drop(s);
    let mut w = JournalWriter::open(&dir.join(JOURNAL_FILE)).unwrap();
    w.append(
        "asset.tagged",
        json!({"asset_id": y, "tag": "fl-0001", "previous_tag": null}),
    )
    .unwrap();
    drop(w);
    assert!(Store::open(&dir).is_err());
}

/// The acquisition link is journaled, replays on reopen, survives a
/// rebuild of the database from the journal, and verifies; a refused link
/// journals nothing.
#[test]
fn the_acquisition_link_replays_and_verifies() {
    let (dir, mut s, a, x, _) = fixture("acquired");
    let e = s.create_entry(date(2026, 1, 1), "bought laptop").unwrap();
    s.add_line(e, a.cost, ledger_core::Side::Debit, usd("1200.00"))
        .unwrap();
    s.add_line(e, a.bank, ledger_core::Side::Credit, usd("1200.00"))
        .unwrap();
    s.post_entry(e).unwrap();
    s.verify().expect("before linking");
    let before = journal(&dir).len();
    assert!(s.set_asset_acquisition(x, 999).is_err());
    assert_eq!(journal(&dir).len(), before, "a refusal journals nothing");
    s.set_asset_acquisition(x, e).unwrap();
    let (kind, p) = journal(&dir).pop().unwrap();
    assert_eq!(kind, "asset.acquisition_linked");
    assert_eq!(p, json!({"asset_id": x, "entry_id": e}));
    s.verify().unwrap();
    drop(s);
    let s = Store::open(&dir).unwrap();
    assert_eq!(s.ledger().get_asset(x).unwrap().acquired_by, Some(e));
    s.verify().unwrap();
    drop(s);
    std::fs::remove_file(dir.join(db_file())).unwrap();
    let s = Store::open(&dir).unwrap();
    assert_eq!(s.ledger().get_asset(x).unwrap().acquired_by, Some(e));
    s.verify().unwrap();
}

/// Regression: verify compared the snapshot's amounts as strings, and a
/// zero total summed in hash-map order could be "0" or "0.00", so books
/// that agreed failed verify at random (found 2026-10-15). Fresh stores
/// (fresh hash seeds) with an entry that nets the assets to zero must all
/// verify.
#[test]
fn verify_compares_amounts_by_value() {
    for n in 0..40 {
        let dir = test_dir(&format!("by-value-{n}"));
        let mut s = Store::create(&dir, Currency::USD).unwrap();
        let a = accounts(&mut s);
        let e = s.create_entry(date(2026, 3, 5), "bought").unwrap();
        s.add_line(e, a.cost, ledger_core::Side::Debit, usd("1200.00"))
            .unwrap();
        s.add_line(e, a.bank, ledger_core::Side::Credit, usd("1200.00"))
            .unwrap();
        s.post_entry(e).unwrap();
        s.verify().unwrap();
        drop(s);
        Store::open(&dir).unwrap().verify().unwrap();
    }
}

/// A description change is journaled with the old one, replays on reopen
/// and from the journal alone, and verifies; a blank one is refused and
/// journals nothing.
#[test]
fn a_description_change_replays_and_verifies() {
    let (dir, mut s, _, x, _) = fixture("described");
    let before = journal(&dir).len();
    assert!(s.set_asset_description(x, "  ").is_err());
    assert_eq!(journal(&dir).len(), before, "a refusal journals nothing");
    s.set_asset_description(x, "Laptop, contributed jointly")
        .unwrap();
    let (kind, p) = journal(&dir).pop().unwrap();
    assert_eq!(kind, "asset.described");
    assert_eq!(p["description"], "Laptop, contributed jointly");
    s.verify().unwrap();
    drop(s);
    std::fs::remove_file(dir.join(db_file())).unwrap();
    let s = Store::open(&dir).unwrap();
    assert_eq!(
        s.ledger().get_asset(x).unwrap().description,
        "Laptop, contributed jointly"
    );
    s.verify().unwrap();
}

// ------------------------------------------------- tax basis (rules.md B-25)
//
// Expectations from `Store::set_asset_basis` (journaled as
// `asset.basis_set`, the feature `asset_basis` declared first),
// `Ledger::set_asset_basis` INVARIANTS and SCENARIOS, the replay of
// pre-P3 `asset.tax_basis_set` lines, and the schema v10 and v13
// migration comments (tax_rule, contributor_basis, fmv_at_contribution as
// TEXT, exact decimal).

fn dec(s: &str) -> rust_decimal::Decimal {
    s.parse().unwrap()
}

fn contribution(
    rule: ledger_core::BasisRule,
    adjusted: &str,
    fmv: &str,
) -> ledger_core::AssetBasis {
    ledger_core::AssetBasis::contribution(rule, dec(adjusted), dec(fmv))
}

/// A pre-P3 record, as `asset.tax_basis_set` replays it.
fn legacy(rule: ledger_core::BasisRule, basis: Option<&str>) -> ledger_core::AssetBasis {
    ledger_core::AssetBasis {
        rule,
        contributor_adjusted_basis: basis.map(dec),
        fmv_at_contribution: None,
        sec704c_method: None,
        source: None,
    }
}

fn basis_of(s: &Store, id: AssetId) -> Option<ledger_core::AssetBasis> {
    s.ledger().get_asset(id).unwrap().basis.clone()
}

/// Journaled as `asset.basis_set` {asset_id, rule, contributor_adjusted_basis,
/// fmv_at_contribution, sec704c_method, source, tax_basis_723,
/// depreciable_basis, built_in_gain_704c}: the facts, and the figures for
/// the line's readers (a purchase's depreciable basis null: the cost,
/// whatever it becomes). The first declares the feature asset_basis.
#[test]
fn a_basis_journals_asset_basis_set() {
    use ledger_core::BasisRule::*;
    let (dir, mut s, _, x, y) = fixture("journal-tax");
    s.set_asset_basis(x, ledger_core::AssetBasis::purchase())
        .unwrap();
    let mut b = contribution(PersonalUseConversion, "1500.25", "1200.00");
    b.sec704c_method = Some(ledger_core::Sec704cMethod::Traditional);
    s.set_asset_basis(y, b.clone()).unwrap();
    let j = journal(&dir);
    let with_feature = |p: &Value| {
        p["features"]
            .as_array()
            .is_some_and(|f| f.contains(&json!("asset_basis")))
    };
    let declared: Vec<usize> = j
        .iter()
        .enumerate()
        .filter(|(_, (t, p))| t == "journal.format_set" && with_feature(p))
        .map(|(i, _)| i)
        .collect();
    let first = j.iter().position(|(t, _)| t == "asset.basis_set").unwrap();
    assert_eq!(declared.len(), 1, "declared once");
    assert!(declared[0] < first, "before the first asset.basis_set");
    let set: Vec<&Value> = j
        .iter()
        .filter(|(t, _)| t == "asset.basis_set")
        .map(|(_, p)| p)
        .collect();
    assert_eq!(set.len(), 2);
    assert_eq!(
        set[0],
        &json!({"asset_id": x, "rule": "purchase", "contributor_adjusted_basis": null,
                "fmv_at_contribution": null, "sec704c_method": null, "source": null,
                "tax_basis_723": null, "depreciable_basis": null, "built_in_gain_704c": null})
    );
    assert_eq!(
        set[1],
        &json!({"asset_id": y, "rule": "personal_use_conversion",
                "contributor_adjusted_basis": "1500.25", "fmv_at_contribution": "1200.00",
                "sec704c_method": "traditional", "source": null,
                "tax_basis_723": "1500.25", "depreciable_basis": "1200.00",
                "built_in_gain_704c": "-300.25"})
    );
    assert_eq!(basis_of(&s, y), Some(b));
    s.verify().unwrap();
}

/// A refused basis (Ledger::set_asset_basis INVARIANT 3) journals nothing
/// (not even the feature declaration) and changes nothing, now or on
/// reopen. A contribution's value is required.
#[test]
fn a_refused_basis_journals_nothing() {
    use ledger_core::BasisRule::*;
    let (dir, mut s, _, x, _) = fixture("refused-tax");
    let lines = journal(&dir).len();
    let before = s.ledger().clone();
    let mut purchase_with_basis = ledger_core::AssetBasis::purchase();
    purchase_with_basis.contributor_adjusted_basis = Some(dec("100.00"));
    let mut no_fmv = contribution(Carryover723, "100.00", "100.00");
    no_fmv.fmv_at_contribution = None;
    for bad in [
        purchase_with_basis,
        no_fmv,
        legacy(PersonalUseConversion, None),
        contribution(Carryover723, "-0.01", "1.00"),
        contribution(PersonalUseConversion, "10.001", "1.00"),
    ] {
        assert!(
            matches!(
                s.set_asset_basis(x, bad.clone()),
                Err(StoreError::Ledger(LedgerError::InvalidTaxBasis(_)))
            ),
            "{bad:?}"
        );
    }
    assert!(matches!(
        s.set_asset_basis(x + 99, ledger_core::AssetBasis::purchase()),
        Err(StoreError::Ledger(LedgerError::UnknownAsset(_)))
    ));
    assert_eq!(journal(&dir).len(), lines, "nothing journaled");
    assert_eq!(s.ledger(), &before);
    s.verify().unwrap();
    drop(s);
    let s = Store::open(&dir).unwrap();
    assert_eq!(s.ledger(), &before);
    s.verify().unwrap();
}

/// A basis replays on reopen, with the database and rebuilt from the
/// journal alone, and in the viewer, and verifies each time (its facts and
/// source included).
#[test]
fn asset_basis_set_journals_replays_and_reloads() {
    use ledger_core::BasisRule::*;
    let (dir, mut s, a, x, y) = fixture("replay-tax");
    let z = laptop(&mut s, &a, "Laptop C");
    s.set_asset_basis(x, contribution(Carryover723, "900.00", "1200.00"))
        .unwrap();
    let mut b = contribution(PersonalUseConversion, "2000.00", "1200.00");
    b.sec704c_method = Some(ledger_core::Sec704cMethod::Remedial);
    b.source = Some(ledger_core::settings::Source {
        decided_by: "Tobin Harlow".into(),
        decided_on: date(2026, 10, 20),
        citation: Some("entry 3".into()),
        note: Some("value as booked".into()),
    });
    s.set_asset_basis(x, b.clone()).unwrap();
    s.set_asset_basis(y, ledger_core::AssetBasis::purchase())
        .unwrap();
    s.verify().unwrap();
    let live = s.ledger().clone();
    drop(s);
    for rebuild in [false, true] {
        if rebuild {
            std::fs::remove_file(dir.join(db_file())).unwrap();
        }
        let s = Store::open(&dir).unwrap();
        s.verify().unwrap();
        assert_eq!(s.ledger(), &live, "rebuild={rebuild}");
        assert_eq!(basis_of(&s, x), Some(b.clone()));
        assert_eq!(basis_of(&s, y), Some(ledger_core::AssetBasis::purchase()));
        assert_eq!(basis_of(&s, z), None);
        let ax = s.ledger().get_asset(x).unwrap();
        assert_eq!(ax.depreciable_basis(), Some(dec("1200.00")));
        assert_eq!(ax.built_in_gain_704c(), Some(dec("-800.00")));
        drop(s);
    }
    let snap = ledger_store::Snapshot::load(&dir).unwrap();
    assert_eq!(snap.ledger(), &live, "the viewer");
}

/// Pre-P3 `asset.tax_basis_set` lines replay to the figures that version
/// gave, with the database and rebuilt from the journal.
#[test]
fn legacy_tax_basis_set_lines_replay_to_todays_figures() {
    use ledger_core::BasisRule::*;
    let (dir, s, _, x, y) = fixture("legacy-tax");
    drop(s);
    let mut w = JournalWriter::open(&dir.join(JOURNAL_FILE)).unwrap();
    w.append(
        "asset.tax_basis_set",
        json!({"asset_id": x, "rule": "personal_use_conversion", "contributor_basis": "1500.25"}),
    )
    .unwrap();
    w.append(
        "asset.tax_basis_set",
        json!({"asset_id": y, "rule": "cost", "contributor_basis": null}),
    )
    .unwrap();
    drop(w);
    for rebuild in [false, true] {
        if rebuild {
            std::fs::remove_file(dir.join(db_file())).unwrap();
        }
        let s = Store::open(&dir).unwrap();
        s.verify().unwrap();
        assert_eq!(
            basis_of(&s, x),
            Some(legacy(PersonalUseConversion, Some("1500.25")))
        );
        assert_eq!(basis_of(&s, y), Some(legacy(Purchase, None)));
        let ax = s.ledger().get_asset(x).unwrap();
        // The lesser of the 1,200.00 book cost and the 1,500.25 basis.
        assert_eq!(ax.depreciable_basis(), Some(dec("1200.00")));
        assert_eq!(ax.tax_basis_723(), Some(dec("1500.25")));
        assert_eq!(ax.built_in_gain_704c(), Some(dec("-300.25")));
        assert!(!ax.fmv_is_recorded());
        drop(s);
    }
}

/// Review R1: a line's figures are for its readers; replay never
/// recomputes them, so a line whose figures disagree with today's formula
/// (a later version's, or a CPA's correction) still replays, and the
/// books show today's figures from its facts.
#[test]
fn a_basis_set_whose_figures_disagree_still_replays() {
    let (dir, s, _, x, _) = fixture("figures-disagree");
    drop(s);
    let mut w = JournalWriter::open(&dir.join(JOURNAL_FILE)).unwrap();
    w.append(
        "journal.format_set",
        json!({"format": 2, "features": ["asset_basis"]}),
    )
    .unwrap();
    w.append(
        "asset.basis_set",
        json!({"asset_id": x, "rule": "personal_use_conversion",
               "contributor_adjusted_basis": "1500.00", "fmv_at_contribution": "1200.00",
               "sec704c_method": null, "source": null,
               "tax_basis_723": "1.00", "depreciable_basis": "1500.00", "built_in_gain_704c": "0"}),
    )
    .unwrap();
    drop(w);
    for rebuild in [false, true] {
        if rebuild {
            std::fs::remove_file(dir.join(db_file())).unwrap();
        }
        let s = Store::open(&dir).unwrap();
        s.verify().unwrap();
        let a = s.ledger().get_asset(x).unwrap();
        assert_eq!(a.depreciable_basis(), Some(dec("1200.00")));
        assert_eq!(a.tax_basis_723(), Some(dec("1500.00")));
        assert_eq!(a.built_in_gain_704c(), Some(dec("-300.00")));
        drop(s);
    }
}

/// A basis rule or §704(c) method this version doesn't know was written
/// by a newer one: NewerJournal, never corrupt (rules.md B-27); a figure
/// or fact that isn't a decimal string (or null) is corrupt.
#[test]
fn an_unknown_basis_rule_or_method_is_newer() {
    let base = json!({"rule": "carryover_723", "contributor_adjusted_basis": "1.00",
        "fmv_at_contribution": "1.00", "sec704c_method": null, "source": null,
        "tax_basis_723": "1.00", "depreciable_basis": "1.00", "built_in_gain_704c": "0.00"});
    let with = |k: &str, v: Value| {
        let mut p = base.clone();
        p[k] = v;
        p
    };
    for (bad, newer) in [
        (with("rule", json!("gift_1015")), true),
        (with("sec704c_method", json!("aggregated")), true),
        (with("rule", json!(7)), false),
        (with("sec704c_method", json!(1)), false),
        (with("fmv_at_contribution", json!(1.5)), false),
        (with("tax_basis_723", json!("one")), false),
        (with("source", json!({"decided_by": 1})), false),
        (with("rule", json!("cost")), true),
    ] {
        let (dir, s, _, x, _) = fixture("newer-basis");
        drop(s);
        let mut p = bad.clone();
        p["asset_id"] = json!(x);
        let mut w = JournalWriter::open(&dir.join(JOURNAL_FILE)).unwrap();
        w.append(
            "journal.format_set",
            json!({"format": 2, "features": ["asset_basis"]}),
        )
        .unwrap();
        w.append("asset.basis_set", p).unwrap();
        drop(w);
        match (Store::open(&dir), newer) {
            (Err(StoreError::NewerJournal { .. }), true) => {}
            (Err(StoreError::JournalCorrupt(_)), false) => {}
            (Err(other), _) => panic!("{bad}: got {other:?}"),
            (Ok(_), _) => panic!("{bad}: opened"),
        }
    }
}

/// Downgrade the database to schema v12 (none of v13's 1099 decisions or
/// payment methods, nor its basis columns; runs keyed by asset and
/// period), as P2 left it.
fn downgrade_db_to_v12(db: &Path) {
    let conn = rusqlite::Connection::open(db).unwrap();
    conn.execute_batch(
        "DROP TABLE vendor_1099_decisions;
         DROP TABLE instruments;
         DROP TABLE entry_instruments;
         ALTER TABLE assets DROP COLUMN fmv_at_contribution;
         ALTER TABLE assets DROP COLUMN sec704c_method;
         ALTER TABLE assets DROP COLUMN basis_source_json;
         CREATE TABLE asset_depreciation_v12(asset_id INTEGER NOT NULL, period TEXT NOT NULL,
           entry_id INTEGER NOT NULL, amount TEXT NOT NULL, PRIMARY KEY (asset_id, period));
         INSERT INTO asset_depreciation_v12 SELECT asset_id, period, entry_id, amount FROM asset_depreciation;
         DROP TABLE asset_depreciation;
         ALTER TABLE asset_depreciation_v12 RENAME TO asset_depreciation;
         UPDATE meta SET value = '12' WHERE key = 'schema_version';",
    )
    .unwrap();
}

/// A database at v13 exactly as it shipped on main before the assets'
/// basis columns (P3 review, 2026-10-21: those columns were first written
/// into v13 itself, so a v13 file built by the earlier commits opened as
/// current and failed to load, "no such column"): its file is another
/// schema's, left byte for byte, and this version builds its own from the
/// journal, with every run, and verifies.
#[test]
fn a_v13_file_built_before_the_basis_columns_is_left_and_rebuilt() {
    use ledger_core::BasisRule::*;
    let (dir, mut s, _, x, y) = fixture("v13-shipped");
    s.run_depreciation(x, "2026-01").unwrap();
    s.set_asset_basis(y, contribution(Carryover723, "640.00", "1200.00"))
        .unwrap();
    drop(s);
    let v13 = dir.join(db_file_for(13));
    std::fs::rename(dir.join(db_file()), &v13).unwrap();
    let conn = rusqlite::Connection::open(&v13).unwrap();
    conn.execute_batch(
        "ALTER TABLE assets DROP COLUMN fmv_at_contribution;
         ALTER TABLE assets DROP COLUMN sec704c_method;
         ALTER TABLE assets DROP COLUMN basis_source_json;
         CREATE TABLE asset_depreciation_v12(asset_id INTEGER NOT NULL, period TEXT NOT NULL,
           entry_id INTEGER NOT NULL, amount TEXT NOT NULL, PRIMARY KEY (asset_id, period));
         INSERT INTO asset_depreciation_v12 SELECT asset_id, period, entry_id, amount FROM asset_depreciation;
         DROP TABLE asset_depreciation;
         ALTER TABLE asset_depreciation_v12 RENAME TO asset_depreciation;
         UPDATE meta SET value = '13' WHERE key = 'schema_version';",
    )
    .unwrap();
    drop(conn);
    let shipped = std::fs::read(&v13).unwrap();
    let s = Store::open(&dir).expect("books with a shipped v13 file open");
    assert_eq!(s.database_file(), db_file());
    s.verify().unwrap();
    assert_eq!(s.ledger().depreciation_runs(x).len(), 1);
    assert_eq!(
        basis_of(&s, y),
        Some(contribution(Carryover723, "640.00", "1200.00"))
    );
    drop(s);
    assert_eq!(std::fs::read(&v13).unwrap(), shipped, "left byte for byte");
}

/// Downgrade the database to schema v10 (none of v11's attachment links,
/// statements, reconciliations, or payroll_lines.employee_id).
fn downgrade_db_to_v10(dir: &Path) {
    downgrade_db_to_v12(&dir.join(db_file()));
    let conn = rusqlite::Connection::open(dir.join(db_file())).unwrap();
    conn.execute_batch(
        "DROP TABLE attachment_links;
         DROP TABLE statements;
         DROP TABLE reconciliations;
         DROP TABLE reconciliation_lines;
         DROP TABLE settings;
         DROP TABLE conventions;
         DROP TABLE decisions;
         DROP TABLE open_items;
         DROP TABLE entry_approvals;
         ALTER TABLE payroll_lines DROP COLUMN employee_id;
         UPDATE meta SET value = '10' WHERE key = 'schema_version';",
    )
    .unwrap();
}

/// P2's database (`wanderbooks.db`, schema v12) is left as it is, byte
/// for byte: this version builds its own file from the journal, its
/// runs table keyed by entry with every run kept and the new columns
/// empty, and it verifies; a basis set after then reloads from it (each
/// schema in a file of its own: rules.md B-27).
#[test]
fn a_p2_database_is_left_and_this_versions_built_from_the_journal() {
    use ledger_core::BasisRule::*;
    let (dir, mut s, _, x, y) = fixture("v12");
    s.run_depreciation(x, "2026-01").unwrap();
    drop(s);
    // As P2 left the books: its schema in the legacy file, none of this
    // version's.
    let legacy = dir.join(ledger_store::LEGACY_DB_FILE);
    std::fs::rename(dir.join(db_file()), &legacy).unwrap();
    downgrade_db_to_v12(&legacy);
    let p2 = std::fs::read(&legacy).unwrap();
    let mut s = Store::open(&dir).expect("P2's books open");
    assert_eq!(s.database_file(), db_file());
    s.verify().unwrap();
    assert_eq!(s.ledger().depreciation_runs(x).len(), 1);
    assert_eq!(basis_of(&s, y), None);
    s.set_asset_basis(y, contribution(Carryover723, "640.00", "1200.00"))
        .unwrap();
    s.verify().unwrap();
    drop(s);
    let s = Store::open(&dir).unwrap();
    s.verify().unwrap();
    assert_eq!(
        basis_of(&s, y),
        Some(contribution(Carryover723, "640.00", "1200.00"))
    );
    assert_eq!(s.ledger().depreciation_runs(x).len(), 1);
    drop(s);
    assert_eq!(
        std::fs::read(&legacy).unwrap(),
        p2,
        "P2's file, byte for byte"
    );
}

/// A database at schema v10 under this version's own file name is rebuilt
/// from the journal as the books open (as the v8 one above), and verifies.
#[test]
fn a_v10_database_migrates_and_verifies() {
    let (dir, mut s, _, x, _) = fixture("v10");
    s.set_asset_tag(x, "FL-0001").unwrap();
    drop(s);
    downgrade_db_to_v10(&dir);
    let s = Store::open(&dir).expect("a v10 database opens");
    assert!(kept_aside(&dir), "rebuilt, not migrated");
    s.verify().unwrap();
    assert_eq!(tag_of(&s, x).as_deref(), Some("FL-0001"));
}

/// Downgrade the database to schema v9 (no assets.tax_rule /
/// assets.contributor_basis), as an older wanderbooks left it.
fn downgrade_db_to_v9(dir: &Path) {
    downgrade_db_to_v10(dir);
    let conn = rusqlite::Connection::open(dir.join(db_file())).unwrap();
    conn.execute_batch(
        "ALTER TABLE assets DROP COLUMN tax_rule;
         ALTER TABLE assets DROP COLUMN contributor_basis;
         UPDATE meta SET value = '9' WHERE key = 'schema_version';",
    )
    .unwrap();
}

/// A database at schema v9 under this version's own file name is rebuilt
/// as the books open (as the v8 one above), verifies, and takes bases.
#[test]
fn a_v9_database_migrates_and_verifies() {
    use ledger_core::BasisRule::*;
    let (dir, mut s, _, x, y) = fixture("v9");
    s.set_asset_tag(x, "FL-0001").unwrap();
    drop(s);
    downgrade_db_to_v9(&dir);
    let mut s = Store::open(&dir).expect("a v9 database opens");
    assert!(kept_aside(&dir), "rebuilt, not migrated");
    s.verify().unwrap();
    assert_eq!(tag_of(&s, x).as_deref(), Some("FL-0001"));
    assert_eq!(basis_of(&s, x), None);
    s.set_asset_basis(y, contribution(Carryover723, "640.00", "1200.00"))
        .unwrap();
    s.verify().unwrap();
    drop(s);
    let s = Store::open(&dir).unwrap();
    s.verify().unwrap();
    assert_eq!(
        basis_of(&s, y),
        Some(contribution(Carryover723, "640.00", "1200.00"))
    );
}

/// A v9 database behind a journal with a tax basis line is rebuilt from
/// the whole journal, that line with it, and verifies.
#[test]
fn a_v9_database_behind_a_journal_with_tax_basis_catches_up() {
    use ledger_core::BasisRule::*;
    let (dir, s, _, x, _) = fixture("v9-behind");
    drop(s);
    downgrade_db_to_v9(&dir);
    let mut w = JournalWriter::open(&dir.join(JOURNAL_FILE)).unwrap();
    w.append(
        "asset.tax_basis_set",
        json!({"asset_id": x, "rule": "carryover", "contributor_basis": "300.00"}),
    )
    .unwrap();
    drop(w);
    let s = Store::open(&dir).expect("opens and catches up");
    assert!(kept_aside(&dir), "rebuilt, not migrated");
    s.verify().unwrap();
    assert_eq!(basis_of(&s, x), Some(legacy(Carryover723, Some("300.00"))));
}

/// An `asset.tax_basis_set` line with an unknown rule, or a contributor
/// basis that isn't a decimal string (or null), is corrupt: refused on
/// open with JournalCorrupt, and on a rebuild from the journal alone.
#[test]
fn an_asset_tax_basis_set_line_with_a_bad_payload_is_corrupt() {
    let bads = [
        json!({"rule": "fmv", "contributor_basis": null}),
        json!({"rule": "Cost", "contributor_basis": null}),
        json!({"rule": "carryover", "contributor_basis": 300}),
        json!({"rule": "carryover", "contributor_basis": 300.5}),
        json!({"rule": "carryover", "contributor_basis": "three hundred"}),
        json!({"rule": "personal_use_conversion", "contributor_basis": ""}),
        json!({"rule": "personal_use_conversion", "contributor_basis": ["300.00"]}),
    ];
    for bad in bads {
        let (dir, s, _, x, _) = fixture("corrupt-tax");
        drop(s);
        let mut p = bad.clone();
        p["asset_id"] = json!(x);
        let mut w = JournalWriter::open(&dir.join(JOURNAL_FILE)).unwrap();
        w.append("asset.tax_basis_set", p).unwrap();
        drop(w);
        match Store::open(&dir) {
            Err(StoreError::JournalCorrupt(_)) => {}
            Err(other) => panic!("{bad}: expected JournalCorrupt, got {other:?}"),
            Ok(_) => panic!("{bad}: a corrupt asset.tax_basis_set line opened"),
        }
        let _ = std::fs::remove_file(dir.join(db_file()));
        match Store::open(&dir) {
            Err(StoreError::JournalCorrupt(_)) => {}
            Err(other) => panic!("{bad} rebuild: expected JournalCorrupt, got {other:?}"),
            Ok(_) => panic!("{bad} rebuild: a corrupt asset.tax_basis_set line opened"),
        }
    }
}

/// A registration mistake is corrected (journaled with the old figures,
/// replayed, verified); a correction needs a reason and valid figures, and
/// is refused once depreciation for the asset is posted.
#[test]
fn an_asset_is_corrected_until_depreciation_is_posted() {
    let (dir, mut s, _, x, _) = fixture("corrected");
    let fix = |cost: &str| ledger_core::AssetCorrection {
        cost: usd(cost),
        salvage_value: usd("0.00"),
        useful_life_months: 12,
        in_service_date: date(2026, 1, 1),
    };
    let before = journal(&dir).len();
    assert!(matches!(
        s.correct_asset(x, fix("1000.00"), " "),
        Err(StoreError::ReasonRequired)
    ));
    assert!(s.correct_asset(x, fix("0.00"), "zero cost").is_err());
    assert_eq!(journal(&dir).len(), before, "refusals journal nothing");
    s.correct_asset(x, fix("1000.00"), "AppleCare+ split out")
        .unwrap();
    let (kind, p) = journal(&dir).pop().unwrap();
    assert_eq!(kind, "asset.corrected");
    assert_eq!(p["cost"]["amount"], "1000.00");
    assert_eq!(p["previous"]["cost"]["amount"], "1200.00");
    s.verify().unwrap();
    drop(s);
    std::fs::remove_file(dir.join(db_file())).unwrap();
    let mut s = Store::open(&dir).unwrap();
    assert_eq!(
        s.ledger().get_asset(x).unwrap().cost.amount.to_string(),
        "1000.00"
    );
    s.verify().unwrap();
    s.run_depreciation(x, "2026-01").unwrap();
    assert!(matches!(
        s.correct_asset(x, fix("900.00"), "too late"),
        Err(StoreError::AssetDepreciationPosted(_))
    ));
}

/// Regression (2026-10-15): reversing an asset's linked acquisition entry
/// (a correction) must not stop the books from opening. The link was valid
/// when made; the books open and verify, and the reversed link is reported
/// for relinking.
#[test]
fn reversing_a_linked_acquisition_still_opens_and_verifies() {
    let (dir, mut s, a, x, _) = fixture("relink");
    let post = |s: &mut Store| {
        let e = s.create_entry(date(2026, 1, 1), "contributed").unwrap();
        s.add_line(e, a.cost, ledger_core::Side::Debit, usd("1200.00"))
            .unwrap();
        s.add_line(e, a.bank, ledger_core::Side::Credit, usd("1200.00"))
            .unwrap();
        s.post_entry(e).unwrap();
        e
    };
    let first = post(&mut s);
    s.set_asset_acquisition(x, first).unwrap();
    s.reverse_entry(first, date(2026, 1, 2), "corrected")
        .unwrap();
    assert!(s.ledger().acquisition_reversed(x));
    s.verify().unwrap();
    drop(s);
    let mut s = Store::open(&dir).expect("books with a reversed link open");
    s.verify().unwrap();
    let second = post(&mut s);
    s.set_asset_acquisition(x, second).unwrap();
    assert!(!s.ledger().acquisition_reversed(x));
    drop(s);
    Store::open(&dir).unwrap().verify().unwrap();
}

// ------------------------------------------------- depreciation runs (rules.md B-24)

/// A run is the books' record of what they carry for an asset
/// (Ledger::accumulated_posted): recorded live, replayed from the journal,
/// reloaded from the database, the same in the viewer, and verify
/// catches a run the database holds differently from the journal (review
/// R5).
#[test]
fn runs_replay_reload_and_verify_catches_a_tampered_run() {
    let (dir, mut s, _, x, _) = fixture("runs");
    s.run_depreciation(x, "2026-01").unwrap();
    s.run_depreciation(x, "2026-02").unwrap();
    let live = s.ledger().depreciation_runs(x).to_vec();
    assert_eq!(live.len(), 2);
    assert_eq!(
        s.ledger().accumulated_posted(x, date(2026, 2, 28)).unwrap(),
        dec("200.00")
    );
    s.verify().unwrap();
    drop(s);
    for rebuild in [false, true] {
        if rebuild {
            std::fs::remove_file(dir.join(db_file())).unwrap();
        }
        let s = Store::open(&dir).unwrap();
        assert_eq!(s.ledger().depreciation_runs(x), live.as_slice());
        s.verify().unwrap();
        drop(s);
    }
    let snap = ledger_store::Snapshot::load(&dir).unwrap();
    assert_eq!(snap.ledger().depreciation_runs(x), live.as_slice());
    let conn = rusqlite::Connection::open(dir.join(db_file())).unwrap();
    conn.execute(
        "UPDATE asset_depreciation SET amount = '99.00' WHERE period = '2026-02'",
        [],
    )
    .unwrap();
    drop(conn);
    let s = Store::open(&dir).unwrap();
    assert!(matches!(s.verify(), Err(StoreError::VerifyMismatch(_))));
}

/// The writer and the viewer read an `asset.depreciation_posted` line the
/// same way: well formed and naming an asset and an entry that exist, it
/// replays (any period an earlier version journaled, rules.md B-28);
/// otherwise both refuse it as corrupt.
#[test]
fn the_viewer_and_writer_agree_on_a_bad_depreciation_posted_payload() {
    for (amount, asset_known, ok) in [
        ("lots", true, false),
        ("1.00", false, false),
        ("0.001", true, true),
    ] {
        let (dir, mut s, a, x, _) = fixture("bad-run");
        let e = s.create_entry(date(2026, 1, 31), "depreciation").unwrap();
        s.add_line(e, a.expense, ledger_core::Side::Debit, usd("1.00"))
            .unwrap();
        s.add_line(e, a.accum, ledger_core::Side::Credit, usd("1.00"))
            .unwrap();
        s.post_entry(e).unwrap();
        drop(s);
        let mut w = JournalWriter::open(&dir.join(JOURNAL_FILE)).unwrap();
        w.append(
            "asset.depreciation_posted",
            json!({"asset_id": if asset_known { x } else { x + 99 }, "period": "10000-01",
                   "entry_id": e, "amount": amount}),
        )
        .unwrap();
        drop(w);
        let _ = std::fs::remove_file(dir.join(db_file()));
        let writer = Store::open(&dir).map(|s| s.ledger().depreciation_runs(x).len());
        let viewer =
            ledger_store::Snapshot::load(&dir).map(|s| s.ledger().depreciation_runs(x).len());
        assert_eq!(writer.is_ok(), ok, "{amount} {asset_known}: {writer:?}");
        assert_eq!(viewer.is_ok(), ok, "{amount} {asset_known}: {viewer:?}");
        if ok {
            assert_eq!(writer.unwrap(), 1);
            assert_eq!(viewer.unwrap(), 1);
        }
    }
}

/// B-27: books an older version wrote (format 1, declaring nothing) refuse
/// a basis while they wait for the owner's format upgrade, a purchase's
/// too, journaling nothing; the feature is never declared on the way (P3
/// review, 2026-10-21).
#[test]
fn a_basis_in_books_pending_their_upgrade_is_refused_and_journals_nothing() {
    use ledger_core::BasisRule::*;
    let dir = test_dir("basis-pending");
    let mut s = Store::create_in_format(
        &dir,
        Currency::USD,
        &ledger_store::format::JournalFormat::default(),
    )
    .unwrap();
    let a = accounts(&mut s);
    let x = laptop(&mut s, &a, "Laptop A");
    let was = std::fs::read(dir.join(JOURNAL_FILE)).unwrap();
    for b in [
        ledger_core::AssetBasis::purchase(),
        contribution(Carryover723, "640.00", "1200.00"),
    ] {
        match s.set_asset_basis(x, b) {
            Err(StoreError::FormatUpgradePending { need }) => {
                assert!(need.contains(ledger_store::format::ASSET_BASIS), "{need:?}")
            }
            other => panic!("expected FormatUpgradePending, got {other:?}"),
        }
    }
    assert_eq!(std::fs::read(dir.join(JOURNAL_FILE)).unwrap(), was);
    assert_eq!(basis_of(&s, x), None);
}
