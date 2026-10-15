//! Spec-derived store tests for asset tags and serial numbers (rules.md
//! B-24): journaling (`asset.tagged` {asset_id, tag, previous_tag},
//! `asset.serial_set` {asset_id, serial | null}), replay on reopen, the v9
//! SQLite projection agreeing with the journal (`Store::verify`), books
//! written before tags existed, refusals journaling nothing, and corrupt
//! `asset.serial_set` lines refused on open; and tax basis (B-25:
//! `asset.tax_basis_set` {asset_id, rule, contributor_basis | null}, the
//! v10 migration, corrupt payloads).
//!
//! Expectations come from the doc comments on `Store::set_asset_tag`,
//! `Store::set_asset_serial`, `Store::open`, `Store::verify`,
//! `Ledger::set_asset_tag` and the schema v9 migration comment.

use chrono::NaiveDate;
use ledger_core::{AccountType, AssetId, Currency, LedgerError, Money, NewAsset};
use ledger_store::journal::read_and_verify_journal;
use ledger_store::{DB_FILE, JOURNAL_FILE, JournalWriter, Store, StoreError};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

fn test_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("wb-asset-tags-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
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
            purchase_date: date(2026, 1, 1),
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
fn fixture(name: &str) -> (PathBuf, Store, Accts, AssetId, AssetId) {
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
            std::fs::remove_file(dir.join(DB_FILE)).unwrap();
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
    let _ = std::fs::remove_file(dir.join(DB_FILE));
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
    let _ = std::fs::remove_file(dir.join(DB_FILE));
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

/// Downgrade the database to schema v8 (no assets.tag / assets.serial, no
/// asset_tags_log, and none of v10's tax basis columns), as an older
/// wanderbooks left it.
fn downgrade_db_to_v8(dir: &Path) {
    downgrade_db_to_v9(dir);
    let conn = rusqlite::Connection::open(dir.join(DB_FILE)).unwrap();
    conn.execute_batch(
        "ALTER TABLE assets DROP COLUMN tag;
         ALTER TABLE assets DROP COLUMN serial;
         ALTER TABLE assets DROP COLUMN acquisition_entry;
         DROP TABLE asset_tags_log;
         UPDATE meta SET value = '8' WHERE key = 'schema_version';",
    )
    .unwrap();
}

/// An existing v8 database migrates to v9 on open and verifies.
#[test]
fn a_v8_database_migrates_and_verifies() {
    let (dir, s, _, x, y) = fixture("v8");
    drop(s);
    downgrade_db_to_v8(&dir);
    let mut s = Store::open(&dir).expect("a v8 database opens");
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

/// A v8 database behind a journal that has tag lines (Store::open SCENARIO
/// 7: apply the rest) migrates, applies them, and verifies.
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
        let _ = std::fs::remove_file(dir.join(DB_FILE));
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
    std::fs::remove_file(dir.join(DB_FILE)).unwrap();
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
    std::fs::remove_file(dir.join(DB_FILE)).unwrap();
    let s = Store::open(&dir).unwrap();
    assert_eq!(
        s.ledger().get_asset(x).unwrap().description,
        "Laptop, contributed jointly"
    );
    s.verify().unwrap();
}

// ------------------------------------------------- tax basis (rules.md B-25)
//
// Expectations from `Store::set_asset_tax_basis` (journaled as
// `asset.tax_basis_set`), `Ledger::set_asset_tax_basis` INVARIANTS and
// SCENARIOS, and the schema v10 migration comment (tax_rule and
// contributor_basis as TEXT, exact decimal).

fn tax(rule: ledger_core::TaxBasisRule, basis: Option<&str>) -> ledger_core::TaxBasis {
    ledger_core::TaxBasis {
        rule,
        contributor_basis: basis.map(|b| b.parse().unwrap()),
    }
}

fn tax_of(s: &Store, id: AssetId) -> Option<ledger_core::TaxBasis> {
    s.ledger().get_asset(id).unwrap().tax
}

/// Journaled as `asset.tax_basis_set` {asset_id, rule, contributor_basis}:
/// the basis a decimal string, or null for a purchase.
#[test]
fn tax_basis_journals_asset_tax_basis_set() {
    use ledger_core::TaxBasisRule::*;
    let (dir, mut s, _, x, y) = fixture("journal-tax");
    s.set_asset_tax_basis(x, tax(Cost, None)).unwrap();
    s.set_asset_tax_basis(y, tax(PersonalUseConversion, Some("1500.25")))
        .unwrap();
    s.set_asset_tax_basis(x, tax(Carryover, Some("800.00")))
        .unwrap();
    let j = journal(&dir);
    let set: Vec<&Value> = j
        .iter()
        .filter(|(t, _)| t == "asset.tax_basis_set")
        .map(|(_, p)| p)
        .collect();
    assert_eq!(set.len(), 3);
    assert_eq!(
        set[0],
        &json!({"asset_id": x, "rule": "cost", "contributor_basis": null})
    );
    assert_eq!(set[1]["asset_id"], json!(y));
    assert_eq!(set[1]["rule"], "personal_use_conversion");
    let b = set[1]["contributor_basis"]
        .as_str()
        .expect("a decimal string");
    assert_eq!(
        b.parse::<rust_decimal::Decimal>().unwrap(),
        "1500.25".parse().unwrap()
    );
    assert_eq!(set[2]["rule"], "carryover");
    let b = set[2]["contributor_basis"]
        .as_str()
        .expect("a decimal string");
    assert_eq!(
        b.parse::<rust_decimal::Decimal>().unwrap(),
        "800.00".parse().unwrap()
    );
    // Set again replaces (the journal keeps both).
    assert_eq!(tax_of(&s, x), Some(tax(Carryover, Some("800.00"))));
    assert_eq!(
        s.ledger().get_asset(y).unwrap().tax_basis(),
        Some("1200.00".parse().unwrap()),
        "lesser of the 1200.00 book cost and the 1500.25 basis"
    );
    s.verify().unwrap();
}

/// A refused tax basis (Ledger::set_asset_tax_basis INVARIANT 3) journals
/// nothing and changes nothing, now or on reopen.
#[test]
fn a_refused_tax_basis_journals_nothing() {
    use ledger_core::TaxBasisRule::*;
    let (dir, mut s, _, x, _) = fixture("refused-tax");
    s.set_asset_tax_basis(x, tax(Carryover, Some("100.00")))
        .unwrap();
    let lines = journal(&dir).len();
    let before = s.ledger().clone();
    for bad in [
        tax(Cost, Some("100.00")),
        tax(Carryover, None),
        tax(PersonalUseConversion, None),
        tax(Carryover, Some("-0.01")),
        tax(PersonalUseConversion, Some("10.001")),
    ] {
        assert!(
            matches!(
                s.set_asset_tax_basis(x, bad),
                Err(StoreError::Ledger(LedgerError::InvalidTaxBasis(_)))
            ),
            "{bad:?}"
        );
    }
    assert!(matches!(
        s.set_asset_tax_basis(x + 99, tax(Cost, None)),
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

/// Tax basis replays on reopen, with the database and rebuilt from the
/// journal alone, and verifies each time.
#[test]
fn tax_basis_replays_on_reopen() {
    use ledger_core::TaxBasisRule::*;
    let (dir, mut s, a, x, y) = fixture("replay-tax");
    let z = laptop(&mut s, &a, "Laptop C");
    s.set_asset_tax_basis(x, tax(Carryover, Some("900.00")))
        .unwrap();
    s.set_asset_tax_basis(x, tax(PersonalUseConversion, Some("2000.00")))
        .unwrap();
    s.set_asset_tax_basis(y, tax(Cost, None)).unwrap();
    s.verify().unwrap();
    let live = s.ledger().clone();
    drop(s);
    for rebuild in [false, true] {
        if rebuild {
            std::fs::remove_file(dir.join(DB_FILE)).unwrap();
        }
        let s = Store::open(&dir).unwrap();
        s.verify().unwrap();
        assert_eq!(s.ledger(), &live, "rebuild={rebuild}");
        assert_eq!(
            tax_of(&s, x),
            Some(tax(PersonalUseConversion, Some("2000.00")))
        );
        assert_eq!(tax_of(&s, y), Some(tax(Cost, None)));
        assert_eq!(tax_of(&s, z), None);
        let ax = s.ledger().get_asset(x).unwrap();
        assert_eq!(ax.tax_basis(), Some("1200.00".parse().unwrap()));
        assert_eq!(ax.built_in_gain(), Some("-800.00".parse().unwrap()));
        drop(s);
    }
}

/// Downgrade the database to schema v10 (none of v11's attachment links,
/// statements, reconciliations, or payroll_lines.employee_id).
fn downgrade_db_to_v10(dir: &Path) {
    let conn = rusqlite::Connection::open(dir.join(DB_FILE)).unwrap();
    conn.execute_batch(
        "DROP TABLE attachment_links;
         DROP TABLE statements;
         DROP TABLE reconciliations;
         DROP TABLE reconciliation_lines;
         ALTER TABLE payroll_lines DROP COLUMN employee_id;
         UPDATE meta SET value = '10' WHERE key = 'schema_version';",
    )
    .unwrap();
}

/// An existing v10 database migrates on open and verifies.
#[test]
fn a_v10_database_migrates_and_verifies() {
    let (dir, mut s, _, x, _) = fixture("v10");
    s.set_asset_tag(x, "FL-0001").unwrap();
    drop(s);
    downgrade_db_to_v10(&dir);
    let s = Store::open(&dir).expect("a v10 database opens");
    s.verify().unwrap();
    assert_eq!(tag_of(&s, x).as_deref(), Some("FL-0001"));
}

/// Downgrade the database to schema v9 (no assets.tax_rule /
/// assets.contributor_basis), as an older wanderbooks left it.
fn downgrade_db_to_v9(dir: &Path) {
    downgrade_db_to_v10(dir);
    let conn = rusqlite::Connection::open(dir.join(DB_FILE)).unwrap();
    conn.execute_batch(
        "ALTER TABLE assets DROP COLUMN tax_rule;
         ALTER TABLE assets DROP COLUMN contributor_basis;
         UPDATE meta SET value = '9' WHERE key = 'schema_version';",
    )
    .unwrap();
}

/// An existing v9 database migrates on open, verifies, and takes tax bases.
#[test]
fn a_v9_database_migrates_and_verifies() {
    use ledger_core::TaxBasisRule::*;
    let (dir, mut s, _, x, y) = fixture("v9");
    s.set_asset_tag(x, "FL-0001").unwrap();
    drop(s);
    downgrade_db_to_v9(&dir);
    let mut s = Store::open(&dir).expect("a v9 database opens");
    s.verify().unwrap();
    assert_eq!(tag_of(&s, x).as_deref(), Some("FL-0001"));
    assert_eq!(tax_of(&s, x), None);
    s.set_asset_tax_basis(y, tax(Carryover, Some("640.00")))
        .unwrap();
    s.verify().unwrap();
    drop(s);
    let s = Store::open(&dir).unwrap();
    s.verify().unwrap();
    assert_eq!(tax_of(&s, y), Some(tax(Carryover, Some("640.00"))));
}

/// A v9 database behind a journal with a tax basis line migrates, applies
/// it (Store::open SCENARIO 7), and verifies.
#[test]
fn a_v9_database_behind_a_journal_with_tax_basis_catches_up() {
    use ledger_core::TaxBasisRule::*;
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
    s.verify().unwrap();
    assert_eq!(tax_of(&s, x), Some(tax(Carryover, Some("300.00"))));
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
        let _ = std::fs::remove_file(dir.join(DB_FILE));
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
        purchase_date: date(2026, 1, 1),
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
    std::fs::remove_file(dir.join(DB_FILE)).unwrap();
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
