//! Spec-derived tests for a party's taxpayer-ID facts in the store.
//!
//! Oracles: `Store::set_party_tin` ("journaled as `party.tin_set`" {id,
//! facts}), the `ledger_core::parties` spec (INVARIANTS 1, 2, 2a: any role,
//! never a full TIN, the TIN and W-9 facts agree), and `Store::verify` (a
//! fresh replay equals the live ledger). Values worked by hand.

use chrono::NaiveDate;
use ledger_core::{Currency, Role, TinFacts, TinType, W9Facts};
use ledger_store::journal::read_and_verify_journal;
use ledger_store::{JOURNAL_FILE, JournalWriter, Store, db_file};
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
    let dir = std::env::temp_dir().join(format!("wb-party-tin-{name}-{}", std::process::id()));
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

fn tin(last4: &str) -> TinFacts {
    TinFacts {
        tin_type: TinType::Ssn,
        last4: last4.into(),
        vault_record: Some("ab".repeat(32)),
        purged: None,
    }
}

fn w9(last4: &str) -> W9Facts {
    W9Facts {
        received: NaiveDate::from_ymd_opt(2026, 1, 5).unwrap(),
        entity_type: "individual/sole proprietor".into(),
        exempt_payee_code: None,
        fatca_code: None,
        tin_last4: last4.into(),
        kept_at: "wanderbooks vault".into(),
        vault_record: None,
    }
}

fn delete_db(dir: &Path) {
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let _ = std::fs::remove_file(dir.join(format!("{}{suffix}", db_file())));
    }
}

/// Any role, journaled as one `party.tin_set {id, facts}` line, replayed
/// on open (with and without the database), and cleared with null.
#[test]
fn tin_facts_journal_and_replay_for_any_role() {
    let dir = test_dir("replay");
    let mut s = Store::create(&dir, Currency::USD).unwrap();
    let member = s.add_party("Mo", &[Role::Member]).unwrap();
    let nobody = s.add_party("Plain", &[]).unwrap();
    s.set_party_tin(member, Some(tin("4915"))).unwrap();
    s.set_party_tin(nobody, Some(tin("1234"))).unwrap();
    s.set_party_tin(nobody, None).unwrap();
    let j = journal(&dir);
    let tins: Vec<&(String, Value)> = j.iter().filter(|(t, _)| t == "party.tin_set").collect();
    assert_eq!(tins.len(), 3);
    assert_eq!(tins[0].1["id"], json!(member));
    assert_eq!(tins[0].1["facts"]["last4"], "4915");
    assert_eq!(tins[0].1["facts"]["tin_type"], "ssn");
    assert!(tins[2].1["facts"].is_null());
    s.verify().unwrap();
    drop(s);
    for without_db in [false, true] {
        if without_db {
            delete_db(&dir);
        }
        let s = Store::open(&dir).unwrap();
        assert_eq!(s.ledger().get_party(member).unwrap().tin, Some(tin("4915")));
        assert_eq!(s.ledger().get_party(nobody).unwrap().tin, None);
    }
}

/// The TIN and W-9 facts never disagree (INVARIANT 2a), and a refusal
/// journals nothing.
#[test]
fn tin_and_w9_facts_agree() {
    let dir = test_dir("agree");
    let mut s = Store::create(&dir, Currency::USD).unwrap();
    let c = s.add_party("Acme", &[Role::Contractor]).unwrap();
    s.set_party_w9(c, Some(w9("2846"))).unwrap();
    let before = journal(&dir).len();
    assert!(s.set_party_tin(c, Some(tin("1111"))).is_err());
    assert!(s.set_party_tin(c, Some(tin("12"))).is_err(), "four digits");
    assert!(
        s.set_party_tin(
            c,
            Some(TinFacts {
                purged: Some("291122846".into()),
                ..tin("2846")
            })
        )
        .is_err(),
        "never a full TIN"
    );
    assert!(s.set_party_tin(99, Some(tin("2846"))).is_err());
    assert_eq!(journal(&dir).len(), before);
    s.set_party_tin(c, Some(tin("2846"))).unwrap();
    assert!(s.set_party_w9(c, Some(w9("1111"))).is_err());
    // Cleared first, the W-9 may change, then the TIN facts follow.
    s.set_party_tin(c, None).unwrap();
    s.set_party_w9(c, Some(w9("1111"))).unwrap();
    s.set_party_tin(c, Some(tin("1111"))).unwrap();
    s.verify().unwrap();
}

/// A `party.tin_set` line that can't replay is refused on open.
#[test]
fn corrupt_tin_payloads_are_refused_on_open() {
    for (i, facts) in [
        json!({"tin_type": "passport", "last4": "1234"}),
        json!({"tin_type": "ssn", "last4": "123456789"}),
        json!({"tin_type": "ssn", "last4": "1234", "vault_record": "nothex"}),
        json!({"last4": "1234"}),
    ]
    .into_iter()
    .enumerate()
    {
        let dir = test_dir(&format!("corrupt-{i}"));
        let mut s = Store::create(&dir, Currency::USD).unwrap();
        let id = s.add_party("A", &[Role::Member]).unwrap();
        drop(s);
        let mut w = JournalWriter::open(&dir.join(JOURNAL_FILE)).unwrap();
        w.append("party.tin_set", json!({"id": id, "facts": facts}))
            .unwrap();
        drop(w);
        assert!(Store::open(&dir).is_err(), "{facts} opened");
        delete_db(&dir);
        assert!(Store::open(&dir).is_err(), "{facts} opened without the db");
    }
}
