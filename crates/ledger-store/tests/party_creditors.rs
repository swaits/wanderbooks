//! A party named the creditor of a liability (rules.md B-35, P3 defect
//! 24), in the store: journaled as `party.creditor_set {id, account_id,
//! on}`, after the journal declares the feature it needs
//! (`party_creditors`) the first time, and never otherwise.
//!
//! Oracles: `Store::set_party_creditor`'s doc, `format::KNOWN_FEATURES`
//! and `PARTY_CREDITORS`, `Store::ensure_feature`'s doc (nothing journaled
//! when the feature is in force; books that never use it never change),
//! and `Ledger::set_party_creditor`'s SCENARIOS. Expected values by hand.

use chrono::NaiveDate;
use ledger_core::{AccountType, Currency, Money, Side};
use ledger_store::format::{FORMAT_SET, KNOWN_FEATURES, PARTY_CREDITORS};
use ledger_store::journal::read_and_verify_journal;
use ledger_store::{DB_FILE, JOURNAL_FILE, JournalWriter, Snapshot, Store};
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
    let mut s = Store::create(dir, Currency::USD).unwrap();
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

/// Store::set_party_creditor doc: the first one declares the feature (one
/// journal.format_set keeping the format, at least 2, and every feature
/// already declared), then journals party.creditor_set; later ones journal
/// one line each; the books replay to the same directory, through the
/// store and the viewer's snapshot alike.
#[test]
fn naming_a_creditor_declares_its_feature_once() {
    let dir = test_dir("declare");
    let (mut s, card, due, issuer, tobin) = books(&dir);
    let before = journal(&dir).len();
    s.set_party_creditor(issuer, card, true).unwrap();
    let j = journal(&dir);
    assert_eq!(j.len(), before + 2);
    assert_eq!(j[before].0, FORMAT_SET);
    assert_eq!(
        j[before].1,
        json!({"format": 2, "features": ["party_creditors"]})
    );
    assert_eq!(
        j[before + 1],
        (
            "party.creditor_set".to_string(),
            json!({"id": issuer, "account_id": card, "on": true})
        )
    );
    assert_eq!(s.journal_format().features, vec!["party_creditors"]);
    s.set_party_creditor(tobin, due, true).unwrap();
    s.set_party_creditor(tobin, due, false).unwrap();
    let j = journal(&dir);
    assert_eq!(j.len(), before + 4, "the feature is declared once");
    assert_eq!(
        j[before + 3].1,
        json!({"id": tobin, "account_id": due, "on": false})
    );
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
    assert_eq!(snap.journal_format().features, vec!["party_creditors"]);
    // Rebuilt from the journal alone.
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(dir.join(format!("{DB_FILE}{suffix}")));
    }
    let s = Store::open(&dir).unwrap();
    assert!(*s.ledger() == live);
}

/// A refusal journals nothing: not the line, and not the feature either.
#[test]
fn a_refused_creditor_journals_nothing() {
    let dir = test_dir("refused");
    let (mut s, _, _, issuer, _) = books(&dir);
    let cash = s.ledger().chart().find_by_code("1000").unwrap().id;
    let was = bytes(&dir);
    for (party, account) in [(issuer, cash), (999, cash), (issuer, 999)] {
        assert!(s.set_party_creditor(party, account, true).is_err());
        assert_eq!(bytes(&dir), was);
    }
    assert!(s.journal_format().features.is_empty());
}

/// Books that never name a creditor never declare the feature: opening,
/// verifying and rebuilding them leaves the journal byte-identical, in the
/// format it was in.
#[test]
fn books_without_creditors_are_never_changed_by_the_feature() {
    let dir = test_dir("untouched");
    let (s, ..) = books(&dir);
    let format = s.journal_format().clone();
    drop(s);
    let was = bytes(&dir);
    let mut s = Store::open(&dir).unwrap();
    s.verify().unwrap();
    s.rebuild().unwrap();
    assert_eq!(s.journal_format(), &format);
    drop(s);
    assert_eq!(bytes(&dir), was);
    assert!(
        journal(&dir)
            .iter()
            .all(|(t, p)| t != FORMAT_SET || p["features"] == json!([]))
    );
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
            let _ = std::fs::remove_file(dir.join(format!("{DB_FILE}{suffix}")));
        }
        assert!(
            Store::open(&dir).is_err(),
            "{name}: {payload} opened without the db"
        );
    }
}
