//! A party named the creditor of a liability (rules.md B-35, P3 defect
//! 24), in the store: journaled as `party.creditor_set {id, account_id,
//! on}`, which needs the journal feature `party_creditors`: books this
//! version made declared it when they were created; books an older
//! version wrote refuse it until their format upgrade (rules.md B-27).
//!
//! Oracles: `Store::set_party_creditor`'s doc, `format::KNOWN_FEATURES`
//! and `PARTY_CREDITORS`, `Store::ensure_feature`'s doc (refused,
//! journaling nothing, unless the books declared it; books that never use
//! it never change), and `Ledger::set_party_creditor`'s SCENARIOS.
//! Expected values by hand.

use chrono::NaiveDate;
use ledger_core::{AccountType, Currency, Money, Side};
use ledger_store::format::{FORMAT_SET, KNOWN_FEATURES, PARTY_CREDITORS, current};
use ledger_store::journal::read_and_verify_journal;
use ledger_store::{
    JOURNAL_FILE, JournalFormat, JournalWriter, Snapshot, Store, StoreError, db_file,
};
use rust_decimal::Decimal;
use serde_json::{Value, json};
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
    let dir = std::env::temp_dir().join(format!("wb-creditors-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    TestDir(dir)
}

fn journal(dir: &Path) -> Vec<(String, Value)> {
    read_and_verify_journal(&dir.join(JOURNAL_FILE))
        .unwrap()
        .into_iter()
        .map(|l| (l.line_type, l.payload))
        .collect()
}

fn bytes(dir: &Path) -> Vec<u8> {
    std::fs::read(dir.join(JOURNAL_FILE)).unwrap()
}

/// Books with a card, the members' financing account, a member, and a
/// posted entry.
fn books(dir: &Path) -> (Store, u64, u64, u64, u64) {
    books_in(dir, &current())
}

/// [`books`] in the journal format `format`: format 1
/// ([`JournalFormat::default`]) is books an older version wrote.
fn books_in(dir: &Path, format: &JournalFormat) -> (Store, u64, u64, u64, u64) {
    let mut s = Store::create_in_format(dir, Currency::USD, format).unwrap();
    let cash = s
        .add_account("1000", "Checking", AccountType::Asset, None, false)
        .unwrap();
    let card = s
        .add_account("2050", "Business Card", AccountType::Liability, None, false)
        .unwrap();
    let due = s
        .add_account(
            "2100",
            "Due to Members",
            AccountType::Liability,
            None,
            false,
        )
        .unwrap();
    let capital = s
        .add_account(
            "3000",
            "Capital — Tobin Harlow",
            AccountType::Equity,
            None,
            false,
        )
        .unwrap();
    let e = s
        .create_entry(NaiveDate::from_ymd_opt(2026, 1, 2).unwrap(), "Contribution")
        .unwrap();
    let m = Money::new(Decimal::from(500), Currency::USD);
    s.add_line(e, cash, Side::Debit, m).unwrap();
    s.add_line(e, capital, Side::Credit, m).unwrap();
    s.post_entry(e).unwrap();
    let issuer = s
        .add_party("Harbor Bank", &[ledger_core::Role::FinancialInstitution])
        .unwrap();
    let tobin = s
        .add_party("Tobin Harlow", &[ledger_core::Role::Member])
        .unwrap();
    (s, card, due, issuer, tobin)
}

/// The feature is one this version reads.
#[test]
fn party_creditors_is_a_known_feature() {
    assert_eq!(PARTY_CREDITORS, "party_creditors");
    assert!(KNOWN_FEATURES.contains(&PARTY_CREDITORS));
}

/// Store::set_party_creditor doc: books this version made declared the
/// feature when they were created (their second line, the only
/// declaration), so each creditor named journals one line,
/// party.creditor_set, and nothing else; the books replay to the same
/// directory, through the store and the viewer's snapshot alike.
#[test]
fn naming_a_creditor_journals_one_line_each_time() {
    let dir = test_dir("declare");
    let (mut s, card, due, issuer, tobin) = books(&dir);
    let j = journal(&dir);
    assert_eq!(j[1].0, FORMAT_SET);
    assert!(
        j[1].1["features"]
            .as_array()
            .unwrap()
            .contains(&json!("party_creditors"))
    );
    let before = j.len();
    s.set_party_creditor(issuer, card, true).unwrap();
    let j = journal(&dir);
    assert_eq!(j.len(), before + 1);
    assert_eq!(
        j[before],
        (
            "party.creditor_set".to_string(),
            json!({"id": issuer, "account_id": card, "on": true})
        )
    );
    s.set_party_creditor(tobin, due, true).unwrap();
    s.set_party_creditor(tobin, due, false).unwrap();
    let j = journal(&dir);
    assert_eq!(j.len(), before + 3, "one line each");
    assert_eq!(
        j[before + 2].1,
        json!({"id": tobin, "account_id": due, "on": false})
    );
    assert_eq!(
        j.iter().filter(|(t, _)| t == FORMAT_SET).count(),
        1,
        "nothing declared on the way"
    );
    assert_eq!(s.journal_format(), &current());
    s.set_party_creditor(tobin, due, true).unwrap();
    let live = s.ledger().clone();
    s.verify().unwrap();
    drop(s);
    let s = Store::open(&dir).unwrap();
    assert_eq!(s.ledger().creditors_of(card), vec![issuer]);
    assert_eq!(s.ledger().creditors_of(due), vec![tobin]);
    assert!(*s.ledger() == live);
    drop(s);
    let snap = Snapshot::load(&dir).unwrap();
    assert!(*snap.ledger() == live);
    assert_eq!(snap.journal_format(), &current());
    // Rebuilt from the journal alone.
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(dir.join(format!("{}{suffix}", db_file())));
    }
    let s = Store::open(&dir).unwrap();
    assert!(*s.ledger() == live);
}

/// Books an older version wrote (format 1, declaring nothing) refuse a
/// creditor while they wait for their format upgrade, journaling nothing,
/// not the line and not the feature; once the owner upgrades them, the
/// same command journals its one line.
#[test]
fn a_creditor_in_older_books_waits_for_the_format_upgrade() {
    let dir = test_dir("older");
    let (mut s, card, _, issuer, _) = books_in(&dir, &JournalFormat::default());
    let was = bytes(&dir);
    assert!(matches!(
        s.set_party_creditor(issuer, card, true),
        Err(StoreError::FormatUpgradePending { need }) if need.contains("party_creditors")
    ));
    assert_eq!(bytes(&dir), was, "nothing journaled");
    assert!(s.ledger().creditors_of(card).is_empty());
    assert!(s.upgrade_format().unwrap().is_some());
    let before = journal(&dir).len();
    s.set_party_creditor(issuer, card, true).unwrap();
    let j = journal(&dir);
    assert_eq!(j.len(), before + 1);
    assert_eq!(j[before].0, "party.creditor_set");
    s.verify().unwrap();
    assert_eq!(s.ledger().creditors_of(card), vec![issuer]);
}

/// A refusal journals nothing: not the line, and not the feature either.
#[test]
fn a_refused_creditor_journals_nothing() {
    let dir = test_dir("refused");
    let (mut s, _, _, issuer, _) = books(&dir);
    let cash = s.ledger().chart().find_by_code("1000").unwrap().id;
    let was = bytes(&dir);
    let format = s.journal_format().clone();
    for (party, account) in [(issuer, cash), (999, cash), (issuer, 999)] {
        assert!(s.set_party_creditor(party, account, true).is_err());
        assert_eq!(bytes(&dir), was);
    }
    assert_eq!(s.journal_format(), &format);
}

/// Books that never name a creditor are never changed by the feature:
/// opening, verifying and rebuilding them leaves the journal
/// byte-identical, in the format it was in, this version's or an older
/// one's (which declares nothing and stays so).
#[test]
fn books_without_creditors_are_never_changed_by_the_feature() {
    for (name, format) in [
        ("untouched", current()),
        ("untouched-old", JournalFormat::default()),
    ] {
        let dir = test_dir(name);
        let (s, ..) = books_in(&dir, &format);
        assert_eq!(s.journal_format(), &format);
        drop(s);
        let was = bytes(&dir);
        let mut s = Store::open(&dir).unwrap();
        s.verify().unwrap();
        s.rebuild().unwrap();
        assert_eq!(s.journal_format(), &format, "{name}");
        drop(s);
        assert_eq!(bytes(&dir), was, "{name}");
        let j = journal(&dir);
        assert!(j.iter().all(|(t, _)| t != "party.creditor_set"), "{name}");
        assert!(
            j.iter().filter(|(t, _)| t == FORMAT_SET).count() <= 1,
            "{name}: nothing declared but at creation"
        );
    }
}

/// A line that can't replay is refused on open, with the database and
/// without it: `on` not a bool, an unknown account, an account that isn't
/// a liability.
#[test]
fn corrupt_creditor_lines_are_refused_on_open() {
    for (name, payload) in [
        (
            "on-not-bool",
            json!({"id": 1, "account_id": 2, "on": "yes"}),
        ),
        (
            "unknown-account",
            json!({"id": 1, "account_id": 99, "on": true}),
        ),
        (
            "not-a-liability",
            json!({"id": 1, "account_id": 1, "on": true}),
        ),
    ] {
        let dir = test_dir(name);
        let (mut s, card, ..) = books(&dir);
        // Declare the feature properly first, so only the line is wrong.
        let issuer = s.ledger().party_by_name("Harbor Bank").unwrap().id;
        s.set_party_creditor(issuer, card, true).unwrap();
        drop(s);
        let mut w = JournalWriter::open(&dir.join(JOURNAL_FILE)).unwrap();
        w.append("party.creditor_set", payload.clone()).unwrap();
        drop(w);
        assert!(Store::open(&dir).is_err(), "{name}: {payload} opened");
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(dir.join(format!("{}{suffix}", db_file())));
        }
        assert!(
            Store::open(&dir).is_err(),
            "{name}: {payload} opened without the db"
        );
    }
}
