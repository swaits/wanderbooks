//! Spec-blind oracle for `ledger-store`.
//!
//! Every expectation here is derived from the written specs (the ```text
//! GOAL / SOURCE / INVARIANTS / SCENARIOS blocks in the doc comments of
//! `store.rs`, `snapshot.rs`, `journal.rs`, `blob.rs`, and the design docs
//! `docs/receipt-store.md` and `docs/requirements.md`), never from the
//! implementation. A test marked `#[ignore = "FINDING: ..."]` follows the
//! spec and fails against the code.

use chrono::NaiveDate;
use ledger_core::{AccountId, AccountType, Currency, EntryId, Money, NewAsset, Side};
use ledger_store::journal::repair_torn_tail;
use ledger_store::{BlobStore, JOURNAL_FILE, Snapshot, Store, StoreError, db_file};
use rust_decimal::Decimal;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------- helpers

const GENESIS: &str = "0000000000000000000000000000000000000000000000000000000000000000";

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
    let dir = std::env::temp_dir().join(format!("wb-spec-store-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    TestDir(dir)
}

fn usd(amount: u64) -> Money {
    Money::new(Decimal::from(amount), Currency::USD)
}

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

fn journal_path(dir: &Path) -> PathBuf {
    dir.join(JOURNAL_FILE)
}

fn jbytes(dir: &Path) -> Vec<u8> {
    std::fs::read(journal_path(dir)).unwrap()
}

/// The journal's lines, raw (without the newline).
fn raw_lines(dir: &Path) -> Vec<Vec<u8>> {
    let bytes = jbytes(dir);
    let mut out = Vec::new();
    let mut start = 0;
    for (i, b) in bytes.iter().enumerate() {
        if *b == b'\n' {
            out.push(bytes[start..i].to_vec());
            start = i + 1;
        }
    }
    if start < bytes.len() {
        out.push(bytes[start..].to_vec());
    }
    out
}

fn parsed_lines(dir: &Path) -> Vec<Value> {
    raw_lines(dir)
        .iter()
        .map(|l| serde_json::from_slice(l).unwrap())
        .collect()
}

/// Canonical JSON per the spec: keys sorted recursively, no whitespace.
fn canon(v: &Value) -> String {
    match v {
        Value::Object(m) => {
            let sorted: BTreeMap<&String, &Value> = m.iter().collect();
            let parts: Vec<String> = sorted
                .iter()
                .map(|(k, v)| format!("{}:{}", serde_json::to_string(k).unwrap(), canon(v)))
                .collect();
            format!("{{{}}}", parts.join(","))
        }
        Value::Array(a) => {
            let parts: Vec<String> = a.iter().map(canon).collect();
            format!("[{}]", parts.join(","))
        }
        other => serde_json::to_string(other).unwrap(),
    }
}

fn sha_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    h.finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>()
}

fn line_types(dir: &Path) -> Vec<String> {
    parsed_lines(dir)
        .iter()
        .map(|l| l["type"].as_str().unwrap().to_string())
        .collect()
}

/// Every file under `dir` (relative path → bytes).
fn tree(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, d: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for e in std::fs::read_dir(d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(root, &p, out);
            } else {
                out.insert(
                    p.strip_prefix(root).unwrap().to_path_buf(),
                    std::fs::read(&p).unwrap(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

fn file_names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

fn remove_db(dir: &Path) {
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(dir.join(format!("{}{suffix}", db_file())));
    }
}

/// True when `needle` appears as a string or number anywhere in `v`.
fn contains_value(v: &Value, needle: &str) -> bool {
    match v {
        Value::String(s) => s == needle,
        Value::Number(n) => n.to_string() == needle,
        Value::Array(a) => a.iter().any(|x| contains_value(x, needle)),
        Value::Object(m) => m.values().any(|x| contains_value(x, needle)),
        _ => false,
    }
}

struct Books {
    store: Store,
    cash: AccountId,
    equity: AccountId,
    rent: AccountId,
    e1: EntryId,
    e2: EntryId,
}

/// Small books: three accounts, two posted entries, no receipts.
fn books(dir: &Path) -> Books {
    let mut store = Store::create(dir, Currency::USD).unwrap();
    let cash = store
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    let equity = store
        .add_account("3000", "Owner Capital", AccountType::Equity, None, false)
        .unwrap();
    let rent = store
        .add_account("6000", "Rent Expense", AccountType::Expense, None, false)
        .unwrap();
    let e1 = store
        .create_entry(date(2026, 1, 2), "Owner investment")
        .unwrap();
    store.add_line(e1, cash, Side::Debit, usd(10_000)).unwrap();
    store
        .add_line(e1, equity, Side::Credit, usd(10_000))
        .unwrap();
    store.post_entry(e1).unwrap();
    let e2 = store
        .create_entry(date(2026, 1, 15), "January rent")
        .unwrap();
    store.add_line(e2, rent, Side::Debit, usd(800)).unwrap();
    store.add_line(e2, cash, Side::Credit, usd(800)).unwrap();
    store.post_entry(e2).unwrap();
    Books {
        store,
        cash,
        equity,
        rent,
        e1,
        e2,
    }
}

/// Run a command that must be refused; assert the journal is byte-identical
/// and the ledger unchanged afterwards. Returns the error.
fn refused<T: std::fmt::Debug>(
    dir: &Path,
    store: &mut Store,
    f: impl FnOnce(&mut Store) -> Result<T, StoreError>,
) -> StoreError {
    let before = jbytes(dir);
    let ledger_before = store.ledger().clone();
    let seq_before = store.journal_seq();
    let atts_before = format!("{:?}", store.list_attachments(None).unwrap());
    let err = f(store).expect_err("command should have been refused");
    assert_eq!(jbytes(dir), before, "refused command changed the journal");
    assert_eq!(
        store.ledger(),
        &ledger_before,
        "refused command changed the ledger"
    );
    assert_eq!(store.journal_seq(), seq_before);
    assert_eq!(
        format!("{:?}", store.list_attachments(None).unwrap()),
        atts_before,
        "refused command changed the projection's attachments"
    );
    err
}

/// Assert the chain spec: whole lines, gapless seq, prev chaining.
fn assert_chain(dir: &Path) {
    let bytes = jbytes(dir);
    assert_eq!(
        bytes.last(),
        Some(&b'\n'),
        "journal must end with a whole line"
    );
    let raws = raw_lines(dir);
    let mut prev = GENESIS.to_string();
    for (i, raw) in raws.iter().enumerate() {
        let v: Value = serde_json::from_slice(raw).unwrap();
        assert_eq!(v["seq"].as_u64(), Some(i as u64 + 1), "seq gap at line {i}");
        assert_eq!(
            v["prev"].as_str(),
            Some(prev.as_str()),
            "prev broken at seq {}",
            i + 1
        );
        let c = canon(&v);
        assert_eq!(
            c.as_bytes(),
            raw.as_slice(),
            "writer must emit canonical form (seq {})",
            i + 1
        );
        prev = sha_hex(c.as_bytes());
    }
}

// ------------------------------------------------- module INVARIANT 1

/// Oracle: store module INVARIANT 1 — a refused command leaves the journal
/// byte-identical and the ledger unchanged (unbalanced post).
#[test]
fn inv1_refused_unbalanced_post_leaves_journal_identical() {
    let dir = test_dir("inv1-post");
    let mut b = books(&dir);
    let e = b.store.create_entry(date(2026, 2, 1), "lopsided").unwrap();
    b.store.add_line(e, b.cash, Side::Debit, usd(5)).unwrap();
    b.store.add_line(e, b.equity, Side::Credit, usd(4)).unwrap();
    refused(&dir, &mut b.store, |s| s.post_entry(e));
}

/// Oracle: store module INVARIANT 1 — refused commands of several kinds
/// leave the journal byte-identical.
#[test]
fn inv1_refused_commands_of_many_kinds_leave_journal_identical() {
    let dir = test_dir("inv1-many");
    let mut b = books(&dir);
    let (cash, e1) = (b.cash, b.e1);
    // Line on an unknown entry.
    refused(&dir, &mut b.store, |s| {
        s.add_line(9999, cash, Side::Debit, usd(1))
    });
    // Line on an unknown account.
    let d = b.store.create_entry(date(2026, 2, 2), "draft").unwrap();
    refused(&dir, &mut b.store, |s| {
        s.add_line(d, 9999, Side::Debit, usd(1))
    });
    // Post an unknown entry.
    refused(&dir, &mut b.store, |s| s.post_entry(9999));
    // Re-post an already posted entry.
    refused(&dir, &mut b.store, |s| s.post_entry(e1));
    // Duplicate account code.
    refused(&dir, &mut b.store, |s| {
        s.add_account("1000", "Cash again", AccountType::Asset, None, false)
    });
    // Reverse an unknown entry.
    refused(&dir, &mut b.store, |s| {
        s.reverse_entry(9999, date(2026, 3, 1), "nope")
    });
    // Receipt commands.
    refused(&dir, &mut b.store, |s| s.link_receipt(4242, e1));
    refused(&dir, &mut b.store, |s| s.unlink_receipt(4242, "why"));
    refused(&dir, &mut b.store, |s| s.withdraw_attachment(4242, "why"));
    refused(&dir, &mut b.store, |s| {
        s.attach_receipt(Some(9999), "r.pdf", "application/pdf", b"x", None)
    });
}

// ------------------------------------------------- module INVARIANT 2

/// Oracle: store module INVARIANT 2 — an accepted command is journaled and
/// applied to SQLite after: the projection's event mirror matches the
/// journal line for line once each command returns.
#[test]
fn inv2_accepted_commands_are_journaled_and_applied() {
    let dir = test_dir("inv2");
    let mut b = books(&dir);
    let n_before = raw_lines(&dir).len();
    b.store
        .add_account("6100", "Software", AccountType::Expense, None, false)
        .unwrap();
    let after = parsed_lines(&dir);
    assert!(after.len() > n_before, "accepted command must be journaled");
    assert_eq!(after.last().unwrap()["type"], "account.added");
    assert_eq!(b.store.journal_seq(), after.len() as u64);
    let events = b.store.events_since(0, 1_000_000).unwrap();
    let types: Vec<String> = events.iter().map(|e| e.event_type.clone()).collect();
    assert_eq!(
        types,
        line_types(&dir),
        "projection must mirror the journal"
    );
    for (ev, line) in events.iter().zip(after.iter()) {
        assert_eq!(Some(ev.seq), line["seq"].as_u64());
        assert_eq!(ev.payload, line["payload"]);
    }
}

// ------------------------------------------------- module INVARIANT 3

/// Oracle: store module INVARIANT 3 + docs/receipt-store.md "Line format" —
/// whole lines, gapless 1-based seq, each prev the SHA-256 of the previous
/// line's canonical JSON, genesis 64 zeros.
#[test]
fn inv3_journal_is_whole_gapless_and_chained() {
    let dir = test_dir("inv3");
    let mut b = books(&dir);
    let (aid, _, _) = b
        .store
        .attach_receipt(
            Some(b.e2),
            "rent.pdf",
            "application/pdf",
            b"rent pdf",
            Some("rent"),
        )
        .unwrap();
    b.store.unlink_receipt(aid, "wrong file").unwrap();
    b.store.withdraw_attachment(aid, "superseded").unwrap();
    b.store
        .file_document(
            "legal",
            "oa.pdf",
            "application/pdf",
            b"operating agreement",
            None,
        )
        .unwrap();
    b.store
        .reverse_entry(b.e2, date(2026, 1, 31), "oops")
        .unwrap();
    assert_chain(&dir);
    // Fields per the line format.
    for l in parsed_lines(&dir) {
        let obj = l.as_object().unwrap();
        for k in ["seq", "ts", "type", "payload", "prev"] {
            assert!(obj.contains_key(k), "line missing {k}: {l}");
        }
    }
    drop(b);
    // Still chained after reopen + more writes.
    let mut s = Store::open(&dir).unwrap();
    s.add_account("6200", "Travel", AccountType::Expense, None, false)
        .unwrap();
    assert_chain(&dir);
}

// ------------------------------------------------- module INVARIANT 4

/// Oracle: store module INVARIANT 4 — replaying the journal rebuilds
/// exactly the live ledger (verify passes; a reopen's ledger equals it).
#[test]
fn inv4_replay_rebuilds_exactly_the_live_ledger() {
    let dir = test_dir("inv4");
    let mut b = books(&dir);
    b.store
        .attach_receipt(Some(b.e1), "dep.pdf", "application/pdf", b"deposit", None)
        .unwrap();
    let draft = b.store.create_entry(date(2026, 3, 3), "a draft").unwrap();
    b.store
        .add_line(draft, b.rent, Side::Debit, usd(7))
        .unwrap();
    b.store
        .reverse_entry(b.e2, date(2026, 2, 1), "reverse rent")
        .unwrap();
    b.store.verify().unwrap();
    let live = b.store.ledger().clone();
    drop(b);
    let reopened = Store::open(&dir).unwrap();
    assert_eq!(reopened.ledger(), &live);
    reopened.verify().unwrap();
}

// ------------------------------------------------- module INVARIANT 5

/// Oracle: store module INVARIANT 5 — a multi-step command (a disposal,
/// which auto-posts depreciation first) refused anywhere writes nothing.
#[test]
fn inv5_refused_disposal_writes_nothing() {
    let dir = test_dir("inv5");
    let mut b = books(&dir);
    let equip = b
        .store
        .add_account("1500", "Equipment", AccountType::Asset, None, false)
        .unwrap();
    let accum = b
        .store
        .add_account(
            "1510",
            "Accumulated Depreciation",
            AccountType::Asset,
            None,
            false,
        )
        .unwrap();
    let depr = b
        .store
        .add_account(
            "6500",
            "Depreciation Expense",
            AccountType::Expense,
            None,
            false,
        )
        .unwrap();
    let asset = b
        .store
        .register_asset(NewAsset {
            description: "Laptop".into(),
            in_service_date: date(2026, 1, 1),
            cost: usd(3_600),
            salvage_value: usd(0),
            useful_life_months: 36,
            cost_account: equip,
            accum_depr_account: accum,
            expense_account: depr,
        })
        .unwrap();
    // Gain/loss and proceeds accounts that do not exist: refused, after the
    // depreciation step would have run.
    let cash = b.cash;
    refused(&dir, &mut b.store, |s| {
        s.dispose_asset(asset, date(2026, 6, 15), usd(100), cash, 9999)
    });
    refused(&dir, &mut b.store, |s| {
        s.dispose_asset(asset, date(2026, 6, 15), usd(100), 9998, cash)
    });
    // Unknown asset.
    refused(&dir, &mut b.store, |s| {
        s.dispose_asset(777, date(2026, 6, 15), usd(100), cash, cash)
    });
    b.store.verify().unwrap();
}

// ------------------------------------------------- module INVARIANT 6

/// Oracle: store module INVARIANT 6 — never delete or rewrite a journal
/// line or a stored file: every earlier journal is a prefix of later ones,
/// and a withdrawn file's bytes stay stored.
#[test]
fn inv6_journal_only_grows_and_files_stay() {
    let dir = test_dir("inv6");
    let mut b = books(&dir);
    let mut prev = jbytes(&dir);
    let (aid, sha, _) = b
        .store
        .attach_receipt(Some(b.e2), "r.pdf", "application/pdf", b"keep me", None)
        .unwrap();
    let blob = dir
        .join("blobs")
        .join(&sha[..2])
        .join(&sha[2..4])
        .join(&sha);
    let check = |prev: &mut Vec<u8>| {
        let now = jbytes(&dir);
        assert!(now.starts_with(prev), "journal was rewritten");
        *prev = now;
    };
    check(&mut prev);
    b.store.unlink_receipt(aid, "wrong").unwrap();
    check(&mut prev);
    b.store.withdraw_attachment(aid, "retired").unwrap();
    check(&mut prev);
    b.store.rebuild().unwrap();
    check(&mut prev);
    b.store.verify_and_heal().unwrap();
    check(&mut prev);
    assert_eq!(std::fs::read(&blob).unwrap(), b"keep me");
    assert_eq!(b.store.attachment_bytes(aid).unwrap(), b"keep me");
}

// ------------------------------------------------- lock_books

/// Oracle: lock_books SCENARIO 1 — a second writer on the same books (in
/// this process) is refused with Locked, saying so; it never extends the
/// journal.
#[test]
fn lock_second_writer_refused() {
    let dir = test_dir("lock");
    let b = books(&dir);
    let before = jbytes(&dir);
    match Store::open(&dir) {
        Err(StoreError::Locked(msg)) => assert!(!msg.trim().is_empty(), "Locked must say so"),
        Err(e) => panic!("expected Locked, got {e:?}"),
        Ok(_) => panic!("two writers opened the same books"),
    }
    assert_eq!(jbytes(&dir), before);
    drop(b);
    // Released on drop.
    Store::open(&dir).unwrap();
}

// ------------------------------------------------- Store::create

/// Oracle: Store::create INVARIANT 1 — the first line is `store.created`
/// with the base currency, seq 1, chained from 64 zeros.
#[test]
fn create_first_line_is_store_created() {
    let dir = test_dir("create-first");
    let s = Store::create(&dir, Currency::USD).unwrap();
    let lines = parsed_lines(&dir);
    assert!(!lines.is_empty());
    let first = &lines[0];
    assert_eq!(first["type"], "store.created");
    assert_eq!(first["seq"], 1);
    assert_eq!(first["prev"], GENESIS);
    assert!(
        contains_value(&first["payload"], "USD"),
        "payload lacks base currency: {first}"
    );
    assert_eq!(s.base_currency(), Currency::USD);
    assert_chain(&dir);
}

/// Oracle: Store::create INVARIANT 2 — the writer lock is held from
/// creation on.
#[test]
fn create_holds_the_lock() {
    let dir = test_dir("create-lock");
    let _s = Store::create(&dir, Currency::USD).unwrap();
    assert!(matches!(Store::open(&dir), Err(StoreError::Locked(_))));
}

/// Oracle: Store::create SCENARIO 1 — refuses where a journal exists; never
/// overwrites or appends.
#[test]
fn create_refuses_existing_books() {
    let dir = test_dir("create-exists");
    drop(books(&dir));
    let before = tree(&dir);
    let r = Store::create(&dir, Currency::USD);
    assert!(
        matches!(r, Err(StoreError::AlreadyExists(_))),
        "got {:?}",
        r.err()
    );
    assert_eq!(tree(&dir), before, "create touched existing books");
    // A bare journal file alone also counts.
    let dir2 = test_dir("create-exists-bare");
    std::fs::write(journal_path(&dir2), jbytes(&dir)).unwrap();
    let before2 = jbytes(&dir2);
    assert!(Store::create(&dir2, Currency::USD).is_err());
    assert_eq!(jbytes(&dir2), before2);
}

// ------------------------------------------------- Store::open

/// Oracle: Store::open SCENARIO 1 — no journal: refuse (NotFound).
#[test]
fn open_without_journal_is_not_found() {
    let dir = test_dir("open-nf");
    let r = Store::open(&dir);
    assert!(
        matches!(r, Err(StoreError::NotFound(_))),
        "got {:?}",
        r.err()
    );
    assert!(
        !journal_path(&dir).exists(),
        "open must not create a journal"
    );
}

/// Oracle: Store::open SCENARIO 2 — another writer: refuse (Locked).
#[test]
fn open_while_open_is_locked() {
    let dir = test_dir("open-locked");
    let _b = books(&dir);
    assert!(matches!(Store::open(&dir), Err(StoreError::Locked(_))));
}

/// Oracle: Store::open INVARIANT 1 — the lock is taken before anything is
/// read: with another writer holding the books, even a damaged journal
/// answers Locked (nothing was read).
#[test]
fn open_takes_lock_before_reading() {
    let dir = test_dir("open-lock-first");
    let _b = books(&dir);
    let good = jbytes(&dir);
    std::fs::write(journal_path(&dir), b"garbage, not a journal\n").unwrap();
    let r = Store::open(&dir);
    std::fs::write(journal_path(&dir), &good).unwrap();
    assert!(matches!(r, Err(StoreError::Locked(_))), "got {:?}", r.err());
}

/// Oracle: Store::open INVARIANTS 2 and 3 — the ledger is the replay of
/// every line, and the projection ends caught up to the head.
#[test]
fn open_replays_and_catches_up() {
    let dir = test_dir("open-replay");
    let b = books(&dir);
    let live = b.store.ledger().clone();
    drop(b);
    let s = Store::open(&dir).unwrap();
    assert_eq!(s.ledger(), &live);
    let n = raw_lines(&dir).len() as u64;
    assert_eq!(s.journal_seq(), n);
    let evs = s.events_since(0, 1_000_000).unwrap();
    assert_eq!(evs.last().map(|e| e.seq), Some(n));
    s.verify().unwrap();
}

/// Oracle: Store::open SCENARIO 3 — a cut-off last line is set aside,
/// truncated, and journaled as `journal.tail_repaired`.
#[test]
fn open_sets_aside_a_torn_tail() {
    let dir = test_dir("open-torn");
    drop(books(&dir));
    let complete = jbytes(&dir);
    let n = raw_lines(&dir).len();
    let torn: &[u8] = b"{\"payload\":{\"code\":\"9";
    let mut with_tail = complete.clone();
    with_tail.extend_from_slice(torn);
    std::fs::write(journal_path(&dir), &with_tail).unwrap();

    let s = Store::open(&dir).unwrap();
    let after = jbytes(&dir);
    assert!(
        after.starts_with(&complete),
        "complete lines must be untouched"
    );
    let lines = parsed_lines(&dir);
    assert_eq!(lines.len(), n + 1);
    assert_eq!(lines[n]["type"], "journal.tail_repaired");
    assert_chain(&dir);
    // Saved beside the journal.
    let saved: Vec<String> = file_names(&dir)
        .into_iter()
        .filter(|f| f.starts_with(&format!("{JOURNAL_FILE}.torn.")))
        .collect();
    assert_eq!(saved.len(), 1, "files: {:?}", file_names(&dir));
    assert_eq!(std::fs::read(dir.join(&saved[0])).unwrap(), torn);
    let repairs = s.tail_repairs().unwrap();
    assert_eq!(repairs.len(), 1);
    assert_eq!(repairs[0].1, torn.len() as u64);
    s.verify().unwrap();
}

/// Oracle: Store::open SCENARIO 4 — a complete line that fails its hash is
/// refused, never repaired or skipped.
#[test]
fn open_refuses_a_damaged_complete_line() {
    let dir = test_dir("open-hash");
    drop(books(&dir));
    let good = jbytes(&dir);
    let s = String::from_utf8(good.clone()).unwrap();
    assert!(s.contains("Owner investment"));
    let bad = s.replacen("Owner investment", "Owner investmenT", 1);
    std::fs::write(journal_path(&dir), bad.as_bytes()).unwrap();
    let r = Store::open(&dir);
    assert!(r.is_err(), "a damaged line must refuse to open");
    assert_eq!(
        jbytes(&dir),
        bad.as_bytes(),
        "open must not repair a complete line"
    );
}

/// Oracle: Store::open SCENARIO 4 — a complete line that fails to parse
/// is refused.
#[test]
fn open_refuses_an_unparseable_complete_line() {
    let dir = test_dir("open-parse");
    drop(books(&dir));
    let mut raws = raw_lines(&dir);
    raws[2] = b"this is not json".to_vec();
    let mut bytes = raws.join(&b'\n');
    bytes.push(b'\n');
    std::fs::write(journal_path(&dir), &bytes).unwrap();
    assert!(Store::open(&dir).is_err());
    assert_eq!(jbytes(&dir), bytes);
}

/// Oracle: Store::open SCENARIO 4 — a bad `seq` is refused even when the
/// hash chain is recomputed to be intact.
#[test]
fn open_refuses_a_seq_gap_with_an_intact_chain() {
    let dir = test_dir("open-seq");
    drop(books(&dir));
    let mut lines = parsed_lines(&dir);
    // Renumber from line 3 on (skipping a seq), then re-chain.
    for (i, l) in lines.iter_mut().enumerate().skip(2) {
        l["seq"] = Value::from(i as u64 + 2);
    }
    let mut prev = GENESIS.to_string();
    let mut out = Vec::new();
    for l in lines.iter_mut() {
        l["prev"] = Value::from(prev.clone());
        let c = canon(l);
        prev = sha_hex(c.as_bytes());
        out.extend_from_slice(c.as_bytes());
        out.push(b'\n');
    }
    std::fs::write(journal_path(&dir), &out).unwrap();
    assert!(Store::open(&dir).is_err(), "a seq gap must refuse to open");
    assert_eq!(jbytes(&dir), out);
}

/// Oracle: Store::open SCENARIO 5 — a missing database is rebuilt from the
/// journal.
#[test]
fn open_rebuilds_a_missing_database() {
    let dir = test_dir("open-nodb");
    let b = books(&dir);
    let live = b.store.ledger().clone();
    drop(b);
    remove_db(&dir);
    let s = Store::open(&dir).unwrap();
    assert!(dir.join(db_file()).exists());
    assert_eq!(s.ledger(), &live);
    assert_eq!(
        s.events_since(0, 1_000_000).unwrap().len(),
        raw_lines(&dir).len()
    );
    s.verify().unwrap();
}

/// Oracle: Store::open SCENARIO 6 — a database failing the integrity check
/// is kept aside for forensics and rebuilt from the journal.
#[test]
fn open_keeps_a_corrupt_database_aside_and_rebuilds() {
    let dir = test_dir("open-corruptdb");
    let b = books(&dir);
    let live = b.store.ledger().clone();
    drop(b);
    remove_db(&dir);
    let garbage: Vec<u8> = b"definitely not an sqlite database "
        .iter()
        .cycle()
        .take(8192)
        .copied()
        .collect();
    std::fs::write(dir.join(db_file()), &garbage).unwrap();
    let journal_before = jbytes(&dir);
    let s = Store::open(&dir).unwrap();
    assert_eq!(s.ledger(), &live);
    s.verify().unwrap();
    // Kept aside: some other file in the directory holds the garbage.
    let kept = tree(&dir)
        .into_iter()
        .any(|(p, bytes)| p != Path::new(&db_file()) && bytes == garbage);
    assert!(
        kept,
        "corrupt db not kept aside; files: {:?}",
        file_names(&dir)
    );
    assert_eq!(
        jbytes(&dir),
        journal_before,
        "rebuilding the db must not touch the journal"
    );
}

/// Oracle: Store::open SCENARIO 7 — a database behind the journal gets the
/// rest applied.
#[test]
fn open_catches_up_a_database_behind_the_journal() {
    let dir = test_dir("open-behind");
    drop(books(&dir));
    let old_db = std::fs::read(dir.join(db_file())).unwrap();
    let old_wal = std::fs::read(dir.join(format!("{}-wal", db_file()))).ok();
    let mut s = Store::open(&dir).unwrap();
    s.add_account("6300", "Meals", AccountType::Expense, None, false)
        .unwrap();
    let (aid, _, _) = s
        .attach_receipt(None, "x.pdf", "application/pdf", b"later", None)
        .unwrap();
    let live = s.ledger().clone();
    drop(s);
    remove_db(&dir);
    std::fs::write(dir.join(db_file()), &old_db).unwrap();
    if let Some(w) = old_wal {
        std::fs::write(dir.join(format!("{}-wal", db_file())), w).unwrap();
    }
    let s = Store::open(&dir).unwrap();
    assert_eq!(s.ledger(), &live);
    assert!(
        s.list_attachments(None)
            .unwrap()
            .iter()
            .any(|a| a.id == aid)
    );
    assert_eq!(
        s.events_since(0, 1_000_000).unwrap().len(),
        raw_lines(&dir).len()
    );
    s.verify().unwrap();
}

// ------------------------------------------------- journal::repair_torn_tail

/// Oracle: repair_torn_tail SCENARIO 1 — a journal ending in a newline is
/// left alone.
#[test]
fn repair_torn_tail_noop_on_whole_journal() {
    let dir = test_dir("rtt-noop");
    drop(books(&dir));
    let before = tree(&dir);
    let r = repair_torn_tail(&journal_path(&dir)).unwrap();
    assert!(r.is_none());
    assert_eq!(tree(&dir), before);
}

/// Oracle: repair_torn_tail SCENARIO 2 — bytes after the last newline are
/// saved beside the journal, truncated away, and reported.
#[test]
fn repair_torn_tail_sets_aside_and_reports() {
    let dir = test_dir("rtt-tail");
    drop(books(&dir));
    let complete = jbytes(&dir);
    let torn: &[u8] = b"{\"seq\":99,\"ty";
    let mut bytes = complete.clone();
    bytes.extend_from_slice(torn);
    std::fs::write(journal_path(&dir), &bytes).unwrap();
    let t = repair_torn_tail(&journal_path(&dir))
        .unwrap()
        .expect("tail reported");
    assert_eq!(t.bytes, torn.len() as u64);
    assert_eq!(t.sha256, sha_hex(torn));
    assert_eq!(std::fs::read(&t.saved_as).unwrap(), torn);
    let parent = t.saved_as.parent().unwrap().canonicalize().unwrap();
    assert_eq!(
        parent,
        dir.canonicalize().unwrap(),
        "saved beside the journal"
    );
    assert_eq!(jbytes(&dir), complete);
}

/// Oracle: repair_torn_tail SCENARIO 3 — never touch a complete line, even
/// a damaged one.
#[test]
fn repair_torn_tail_never_touches_complete_lines() {
    let dir = test_dir("rtt-damaged");
    drop(books(&dir));
    let s = String::from_utf8(jbytes(&dir)).unwrap();
    let damaged = s.replacen("January rent", "January renT", 1);
    std::fs::write(journal_path(&dir), damaged.as_bytes()).unwrap();
    let _ = repair_torn_tail(&journal_path(&dir));
    assert_eq!(jbytes(&dir), damaged.as_bytes());
    // Damaged line plus a torn tail: only the tail goes.
    let mut with_tail = damaged.clone().into_bytes();
    with_tail.extend_from_slice(b"{\"partial");
    std::fs::write(journal_path(&dir), &with_tail).unwrap();
    let _ = repair_torn_tail(&journal_path(&dir));
    let after = jbytes(&dir);
    assert!(
        after == damaged.as_bytes() || after == with_tail,
        "a complete line was changed"
    );
    assert!(after.starts_with(damaged.as_bytes()));
}

// ------------------------------------------------- Store::attach_receipt

/// Oracle: Store::attach_receipt INVARIANT 1 — bytes stored under their
/// SHA-256 (at blobs/<aa>/<bb>/<hex>) and read back identical.
#[test]
fn attach_stores_under_sha256() {
    let dir = test_dir("attach-sha");
    let mut b = books(&dir);
    let bytes = b"%PDF-1.4 rent receipt";
    let (aid, sha, dedup) = b
        .store
        .attach_receipt(
            Some(b.e2),
            "rent.pdf",
            "application/pdf",
            bytes,
            Some("rent"),
        )
        .unwrap();
    assert_eq!(sha, sha_hex(bytes));
    assert!(!dedup);
    let p = dir
        .join("blobs")
        .join(&sha[..2])
        .join(&sha[2..4])
        .join(&sha);
    assert_eq!(std::fs::read(&p).unwrap(), bytes);
    assert_eq!(b.store.attachment_bytes(aid).unwrap(), bytes);
    assert_eq!(b.store.verify_blobs().unwrap(), 1);
}

/// Oracle: Store::attach_receipt INVARIANTS 2 and 3, as plan P6 step 4
/// changes them — the same bytes filed twice share one stored file and one
/// live attachment, the second says deduplicated, and that attachment is
/// linked to the second subject (its name, type and description are the
/// first filing's). Records an older version filed twice stay as written
/// (`legacy_duplicates_replay_as_written`).
#[test]
fn same_bytes_are_one_attachment_linked_to_both() {
    let dir = test_dir("attach-dedup");
    let mut b = books(&dir);
    let bytes = b"same bytes";
    let (a1, s1, d1) = b
        .store
        .attach_receipt(
            Some(b.e1),
            "one.pdf",
            "application/pdf",
            bytes,
            Some("first"),
        )
        .unwrap();
    let (a2, s2, d2) = b
        .store
        .attach_receipt(Some(b.e2), "two.png", "image/png", bytes, Some("second"))
        .unwrap();
    assert_eq!((a1, a2), (a1, a1), "one attachment");
    assert_eq!(s1, s2);
    assert!(!d1);
    assert!(d2);
    let blob_files: Vec<_> = tree(&dir)
        .into_keys()
        .filter(|p| p.starts_with("blobs"))
        .collect();
    assert_eq!(blob_files.len(), 1, "one stored file: {blob_files:?}");
    let all = b.store.list_attachments(None).unwrap();
    assert_eq!(all.len(), 1, "one record: {all:?}");
    let one = all.iter().find(|a| a.id == a1).expect("listed");
    assert_eq!(one.entry_id, Some(b.e1));
    assert_eq!(
        (one.file_name.as_str(), one.mime_type.as_str()),
        ("one.pdf", "application/pdf")
    );
    assert_eq!(one.description.as_deref(), Some("first"));
    assert_eq!(one.size_bytes, bytes.len() as u64);
    assert_eq!(one.blob_sha, s1);
    // The second subject is linked, and counts as its receipt.
    assert!(
        b.store
            .attachment_links(a1)
            .unwrap()
            .contains(&ledger_store::Subject::Entry(b.e2))
    );
    assert_eq!(
        b.store
            .attachments_for(ledger_store::Subject::Entry(b.e2))
            .unwrap()
            .len(),
        1
    );
    // One filing line: the second added a link, not a record.
    let attached: Vec<Value> = parsed_lines(&dir)
        .into_iter()
        .filter(|l| l["type"] == "receipt.attached")
        .collect();
    assert_eq!(attached.len(), 1);
    for needle in ["one.pdf", "application/pdf", "first", s1.as_str()] {
        assert!(
            contains_value(&attached[0]["payload"], needle),
            "{needle} missing from {}",
            attached[0]
        );
    }
    assert_eq!(b.store.attachment_bytes(a1).unwrap(), bytes);
}

/// Oracle: Store::attach_receipt INVARIANT 3, as plan P6 step 4 changes
/// it — one file, one record: the first filing's own name and type stay,
/// in the writer and in the viewer.
#[test]
fn the_first_filings_name_and_type_stay() {
    let dir = test_dir("attach-dedup-name");
    let mut b = books(&dir);
    let bytes = b"same bytes, two names";
    let (a1, _, _) = b
        .store
        .attach_receipt(Some(b.e1), "one.pdf", "application/pdf", bytes, None)
        .unwrap();
    let (a2, _, d2) = b
        .store
        .attach_receipt(Some(b.e2), "two.png", "image/png", bytes, None)
        .unwrap();
    assert_eq!((a1, a2), (a1, a1));
    assert!(d2);
    let all = b.store.list_attachments(None).unwrap();
    assert_eq!(all.len(), 1);
    let one = all.iter().find(|a| a.id == a1).unwrap();
    assert_eq!(
        (one.file_name.as_str(), one.mime_type.as_str()),
        ("one.pdf", "application/pdf")
    );
    let snap = Snapshot::load(&dir).unwrap();
    let s1 = snap
        .attachments(None)
        .unwrap()
        .into_iter()
        .find(|a| a.id == a1)
        .unwrap();
    assert_eq!(
        (s1.file_name.as_str(), s1.mime_type.as_str()),
        ("one.pdf", "application/pdf")
    );
}

/// Oracle: Store::attach_receipt SCENARIO 1 — an entry_id naming no entry
/// is refused; no attachment points at nothing.
#[test]
fn attach_to_unknown_entry_refused() {
    let dir = test_dir("attach-unknown");
    let mut b = books(&dir);
    refused(&dir, &mut b.store, |s| {
        s.attach_receipt(Some(9999), "r.pdf", "application/pdf", b"orphan", None)
    });
    assert!(b.store.list_attachments(None).unwrap().is_empty());
    assert!(b.store.list_attachments(Some(9999)).unwrap().is_empty());
}

/// Oracle: Store::attach_receipt SCENARIO 2 — entry_id None files it
/// unlinked, to link later.
#[test]
fn attach_unlinked_then_link() {
    let dir = test_dir("attach-none");
    let mut b = books(&dir);
    let (aid, _, _) = b
        .store
        .attach_receipt(None, "later.pdf", "application/pdf", b"later", None)
        .unwrap();
    let a = b.store.list_attachments(None).unwrap();
    assert_eq!(a.len(), 1);
    assert_eq!(a[0].id, aid);
    assert_eq!(a[0].entry_id, None);
    b.store.link_receipt(aid, b.e2).unwrap();
    assert_eq!(b.store.list_attachments(Some(b.e2)).unwrap()[0].id, aid);
}

// ------------------------------------------------- Store::link_receipt

fn filed_unlinked(b: &mut Books, bytes: &[u8]) -> u64 {
    b.store
        .attach_receipt(None, "f.pdf", "application/pdf", bytes, None)
        .unwrap()
        .0
}

/// Oracle: Store::link_receipt SCENARIO 1 — an unknown attachment is
/// refused.
#[test]
fn link_unknown_attachment_refused() {
    let dir = test_dir("link-unknown");
    let mut b = books(&dir);
    let e = b.e1;
    let err = refused(&dir, &mut b.store, |s| s.link_receipt(4242, e));
    assert!(
        matches!(err, StoreError::UnknownAttachment(4242)),
        "got {err:?}"
    );
}

/// Oracle: Store::link_receipt SCENARIO 2 — a withdrawn attachment is
/// refused (AttachmentWithdrawn).
#[test]
fn link_withdrawn_refused() {
    let dir = test_dir("link-withdrawn");
    let mut b = books(&dir);
    let aid = filed_unlinked(&mut b, b"w");
    b.store.withdraw_attachment(aid, "not needed").unwrap();
    let e = b.e1;
    let err = refused(&dir, &mut b.store, |s| s.link_receipt(aid, e));
    assert!(
        matches!(err, StoreError::AttachmentWithdrawn(x) if x == aid),
        "got {err:?}"
    );
}

/// Oracle: Store::link_receipt SCENARIO 3 — an attachment linked to any
/// entry (the same one or another) is refused (AttachmentLinked).
#[test]
fn link_already_linked_refused() {
    let dir = test_dir("link-linked");
    let mut b = books(&dir);
    let (aid, _, _) = b
        .store
        .attach_receipt(Some(b.e1), "r.pdf", "application/pdf", b"l", None)
        .unwrap();
    let (e1, e2) = (b.e1, b.e2);
    let err = refused(&dir, &mut b.store, |s| s.link_receipt(aid, e2));
    assert!(
        matches!(err, StoreError::AttachmentLinked(x, y) if x == aid && y == e1),
        "got {err:?}"
    );
    let err = refused(&dir, &mut b.store, |s| s.link_receipt(aid, e1));
    assert!(
        matches!(err, StoreError::AttachmentLinked(_, _)),
        "got {err:?}"
    );
}

/// Oracle: Store::link_receipt SCENARIO 4 — an unknown entry is refused.
#[test]
fn link_unknown_entry_refused() {
    let dir = test_dir("link-noentry");
    let mut b = books(&dir);
    let aid = filed_unlinked(&mut b, b"u");
    refused(&dir, &mut b.store, |s| s.link_receipt(aid, 9999));
    assert_eq!(b.store.list_attachments(None).unwrap()[0].entry_id, None);
}

/// Oracle: Store::link_receipt SCENARIO 5 — otherwise `receipt.linked` is
/// journaled and the attachment is linked.
#[test]
fn link_journals_receipt_linked() {
    let dir = test_dir("link-ok");
    let mut b = books(&dir);
    let aid = filed_unlinked(&mut b, b"ok");
    let n = raw_lines(&dir).len();
    b.store.link_receipt(aid, b.e2).unwrap();
    let lines = parsed_lines(&dir);
    assert_eq!(lines.len(), n + 1);
    let last = &lines[n];
    assert_eq!(last["type"], "receipt.linked");
    assert!(contains_value(&last["payload"], &aid.to_string()));
    assert!(contains_value(&last["payload"], &b.e2.to_string()));
    let a = b.store.list_attachments(Some(b.e2)).unwrap();
    assert_eq!(a.len(), 1);
    assert_eq!(a[0].id, aid);
    b.store.verify().unwrap();
}

// ------------------------------------------------- Store::unlink_receipt

/// Oracle: Store::unlink_receipt SCENARIO 1 — unknown, withdrawn, or not
/// linked: refused.
#[test]
fn unlink_refusals() {
    let dir = test_dir("unlink-refuse");
    let mut b = books(&dir);
    let err = refused(&dir, &mut b.store, |s| s.unlink_receipt(4242, "why"));
    assert!(
        matches!(err, StoreError::UnknownAttachment(4242)),
        "got {err:?}"
    );
    let loose = filed_unlinked(&mut b, b"loose");
    let err = refused(&dir, &mut b.store, |s| s.unlink_receipt(loose, "why"));
    assert!(
        matches!(err, StoreError::AttachmentNotLinked(x) if x == loose),
        "got {err:?}"
    );
    let gone = filed_unlinked(&mut b, b"gone");
    b.store.withdraw_attachment(gone, "retired").unwrap();
    refused(&dir, &mut b.store, |s| s.unlink_receipt(gone, "why"));
}

/// Oracle: Store::unlink_receipt SCENARIO 2 — a blank reason is refused.
#[test]
fn unlink_blank_reason_refused() {
    let dir = test_dir("unlink-blank");
    let mut b = books(&dir);
    let (aid, _, _) = b
        .store
        .attach_receipt(Some(b.e2), "r.pdf", "application/pdf", b"b", None)
        .unwrap();
    for reason in ["", "   ", "\t\n"] {
        let err = refused(&dir, &mut b.store, |s| s.unlink_receipt(aid, reason));
        assert!(
            matches!(err, StoreError::ReasonRequired),
            "{reason:?}: got {err:?}"
        );
    }
    assert_eq!(b.store.list_attachments(Some(b.e2)).unwrap().len(), 1);
}

/// Oracle: Store::unlink_receipt SCENARIO 3 — journals `receipt.unlinked`
/// with the entry and reason, returns the entry; the file stays stored and
/// filed.
#[test]
fn unlink_journals_and_keeps_the_file() {
    let dir = test_dir("unlink-ok");
    let mut b = books(&dir);
    let (aid, _, _) = b
        .store
        .attach_receipt(Some(b.e2), "r.pdf", "application/pdf", b"unlink me", None)
        .unwrap();
    let n = raw_lines(&dir).len();
    let e = b.store.unlink_receipt(aid, "wrong receipt").unwrap();
    assert_eq!(e, b.e2);
    let lines = parsed_lines(&dir);
    assert_eq!(lines.len(), n + 1);
    let last = &lines[n];
    assert_eq!(last["type"], "receipt.unlinked");
    assert!(contains_value(&last["payload"], &b.e2.to_string()));
    assert!(contains_value(&last["payload"], &aid.to_string()));
    assert!(contains_value(&last["payload"], "wrong receipt"));
    assert!(b.store.list_attachments(Some(b.e2)).unwrap().is_empty());
    let all = b.store.list_attachments(None).unwrap();
    let a = all.iter().find(|a| a.id == aid).expect("still filed");
    assert_eq!(a.entry_id, None);
    assert_eq!(b.store.attachment_bytes(aid).unwrap(), b"unlink me");
    // Can be linked again.
    b.store.link_receipt(aid, b.e1).unwrap();
    b.store.verify().unwrap();
}

// ------------------------------------------------- Store::withdraw_attachment

/// Oracle: Store::withdraw_attachment SCENARIO 1 — unknown or already
/// withdrawn: refused.
#[test]
fn withdraw_unknown_or_twice_refused() {
    let dir = test_dir("withdraw-refuse");
    let mut b = books(&dir);
    let err = refused(&dir, &mut b.store, |s| s.withdraw_attachment(4242, "why"));
    assert!(
        matches!(err, StoreError::UnknownAttachment(4242)),
        "got {err:?}"
    );
    let aid = filed_unlinked(&mut b, b"twice");
    b.store.withdraw_attachment(aid, "first").unwrap();
    refused(&dir, &mut b.store, |s| s.withdraw_attachment(aid, "second"));
}

/// Oracle: Store::withdraw_attachment SCENARIO 2 — a linked attachment is
/// refused (AttachmentLinked).
#[test]
fn withdraw_linked_refused() {
    let dir = test_dir("withdraw-linked");
    let mut b = books(&dir);
    let (aid, _, _) = b
        .store
        .attach_receipt(Some(b.e1), "r.pdf", "application/pdf", b"linked", None)
        .unwrap();
    let e1 = b.e1;
    let err = refused(&dir, &mut b.store, |s| s.withdraw_attachment(aid, "why"));
    assert!(
        matches!(err, StoreError::AttachmentLinked(x, y) if x == aid && y == e1),
        "got {err:?}"
    );
}

/// Oracle: Store::withdraw_attachment SCENARIO 3 — a blank reason is
/// refused.
#[test]
fn withdraw_blank_reason_refused() {
    let dir = test_dir("withdraw-blank");
    let mut b = books(&dir);
    let aid = filed_unlinked(&mut b, b"blank");
    for reason in ["", "  ", "\n"] {
        let err = refused(&dir, &mut b.store, |s| s.withdraw_attachment(aid, reason));
        assert!(
            matches!(err, StoreError::ReasonRequired),
            "{reason:?}: got {err:?}"
        );
    }
}

/// Oracle: Store::withdraw_attachment SCENARIO 4 — journals
/// `attachment.withdrawn`; its bytes still verify and read; it can never be
/// linked again.
#[test]
fn withdraw_journals_and_bytes_still_read() {
    let dir = test_dir("withdraw-ok");
    let mut b = books(&dir);
    let aid = filed_unlinked(&mut b, b"retire me");
    let (doc, _, _) = b
        .store
        .file_document("tax", "1065.pdf", "application/pdf", b"tax doc", None)
        .unwrap();
    let n = raw_lines(&dir).len();
    b.store.withdraw_attachment(aid, "duplicate").unwrap();
    b.store.withdraw_attachment(doc, "filed in error").unwrap();
    let lines = parsed_lines(&dir);
    assert_eq!(lines.len(), n + 2);
    for (l, (id, why)) in lines[n..]
        .iter()
        .zip([(aid, "duplicate"), (doc, "filed in error")])
    {
        assert_eq!(l["type"], "attachment.withdrawn");
        assert!(contains_value(&l["payload"], &id.to_string()));
        assert!(contains_value(&l["payload"], why));
    }
    assert_eq!(b.store.attachment_bytes(aid).unwrap(), b"retire me");
    assert_eq!(b.store.attachment_bytes(doc).unwrap(), b"tax doc");
    assert_eq!(b.store.verify_blobs().unwrap(), 2);
    let e = b.e1;
    refused(&dir, &mut b.store, |s| s.link_receipt(aid, e));
    b.store.verify().unwrap();
}

/// Oracle: Store::withdraw_attachment SCENARIO 4 — the store's listing
/// marks it withdrawn (with the reason) and the active set omits it.
/// (Adjudicated 2026-10-13: the store lists everything, marked; the MCP
/// listing hides withdrawn files unless asked.)
#[test]
fn withdraw_listings_omit_it() {
    let dir = test_dir("withdraw-listing");
    let mut b = books(&dir);
    let keep = filed_unlinked(&mut b, b"keep");
    let aid = filed_unlinked(&mut b, b"omit");
    b.store.withdraw_attachment(aid, "retired").unwrap();
    let ids: Vec<u64> = b
        .store
        .list_attachments(None)
        .unwrap()
        .iter()
        .filter(|a| a.withdrawn_at.is_none())
        .map(|a| a.id)
        .collect();
    assert_eq!(ids, vec![keep]);
    let all = b.store.list_attachments(None).unwrap();
    let withdrawn = all
        .iter()
        .find(|a| a.id == aid)
        .expect("still listed, marked");
    assert!(withdrawn.withdrawn_at.is_some());
    assert_eq!(withdrawn.withdrawn_reason.as_deref(), Some("retired"));
}

// ------------------------------------------------- Store::verify

/// Oracle: Store::verify INVARIANT 1 — verify changes nothing.
#[test]
fn verify_changes_nothing() {
    let dir = test_dir("verify-nochange");
    let mut b = books(&dir);
    b.store
        .attach_receipt(Some(b.e1), "r.pdf", "application/pdf", b"v", None)
        .unwrap();
    let journal = jbytes(&dir);
    let names = file_names(&dir);
    let ledger = b.store.ledger().clone();
    let seq = b.store.journal_seq();
    let atts = format!("{:?}", b.store.list_attachments(None).unwrap());
    let events = b.store.events_since(0, 1_000_000).unwrap().len();
    b.store.verify().unwrap();
    b.store.verify().unwrap();
    assert_eq!(jbytes(&dir), journal);
    assert_eq!(file_names(&dir), names);
    assert_eq!(b.store.ledger(), &ledger);
    assert_eq!(b.store.journal_seq(), seq);
    assert_eq!(
        format!("{:?}", b.store.list_attachments(None).unwrap()),
        atts
    );
    assert_eq!(b.store.events_since(0, 1_000_000).unwrap().len(), events);
}

/// Oracle: Store::verify INVARIANT 2 — a projection table that differs from
/// a fresh projection fails with VerifyMismatch, naming what differs.
#[test]
fn verify_detects_projection_divergence() {
    let dir = test_dir("verify-diverge");
    let mut b = books(&dir);
    b.store
        .attach_receipt(Some(b.e1), "r.pdf", "application/pdf", b"d", None)
        .unwrap();
    {
        let c = rusqlite::Connection::open(dir.join(db_file())).unwrap();
        let n = c.execute("DELETE FROM attachments", []).unwrap();
        assert_eq!(n, 1);
    }
    let journal = jbytes(&dir);
    match b.store.verify() {
        Err(StoreError::VerifyMismatch(msg)) => assert!(!msg.trim().is_empty()),
        other => panic!("expected VerifyMismatch, got {other:?}"),
    }
    assert_eq!(jbytes(&dir), journal);
}

/// Oracle: Store::verify INVARIANT 2 — verify fails unless the chain
/// verifies (journal damaged under a live store).
#[test]
fn verify_detects_a_broken_chain() {
    let dir = test_dir("verify-chain");
    let b = books(&dir);
    let s = String::from_utf8(jbytes(&dir)).unwrap();
    let bad = s.replacen("January rent", "January renT", 1);
    std::fs::write(journal_path(&dir), bad.as_bytes()).unwrap();
    assert!(b.store.verify().is_err());
    assert_eq!(jbytes(&dir), bad.as_bytes());
}

// ------------------------------------------------- Store::verify_and_heal

/// Oracle: Store::verify_and_heal SCENARIO 1 — clean books report the event
/// count and head hash, healed: none.
#[test]
fn heal_clean_books_reports_nothing_healed() {
    let dir = test_dir("heal-clean");
    let mut b = books(&dir);
    let journal = jbytes(&dir);
    let r = b.store.verify_and_heal().unwrap();
    let lines = parsed_lines(&dir);
    assert_eq!(r.events, lines.len() as u64);
    assert_eq!(r.head, sha_hex(canon(lines.last().unwrap()).as_bytes()));
    assert_eq!(r.healed, None);
    assert_eq!(jbytes(&dir), journal);
}

/// Oracle: Store::verify_and_heal SCENARIO 2 — a diverged projection is
/// copied aside, rebuilt from the journal, verified again, with why.
#[test]
fn heal_rebuilds_a_diverged_projection() {
    let dir = test_dir("heal-diverged");
    let mut b = books(&dir);
    let (aid, _, _) = b
        .store
        .attach_receipt(Some(b.e1), "r.pdf", "application/pdf", b"h", None)
        .unwrap();
    {
        let c = rusqlite::Connection::open(dir.join(db_file())).unwrap();
        c.execute("DELETE FROM attachments", []).unwrap();
    }
    let journal = jbytes(&dir);
    let names_before = file_names(&dir);
    let r = b.store.verify_and_heal().unwrap();
    let why = r.healed.expect("must report why it healed");
    assert!(!why.trim().is_empty());
    assert_eq!(r.events, raw_lines(&dir).len() as u64);
    assert_eq!(jbytes(&dir), journal, "healing must not touch the journal");
    let names_after = file_names(&dir);
    assert!(
        names_after.iter().any(|n| !names_before.contains(n)),
        "the divergent db was not kept aside: {names_before:?} -> {names_after:?}"
    );
    b.store.verify().unwrap();
    assert_eq!(b.store.list_attachments(None).unwrap()[0].id, aid);
}

/// Oracle: Store::verify_and_heal SCENARIO 3 — a journal that fails is
/// refused, never touched.
#[test]
fn heal_refuses_a_broken_journal() {
    let dir = test_dir("heal-journal");
    let mut b = books(&dir);
    let s = String::from_utf8(jbytes(&dir)).unwrap();
    let bad = s.replacen("Owner investment", "Owner investmenT", 1);
    std::fs::write(journal_path(&dir), bad.as_bytes()).unwrap();
    assert!(b.store.verify_and_heal().is_err());
    assert_eq!(jbytes(&dir), bad.as_bytes());
}

// ------------------------------------------------- Snapshot::refresh

/// Oracle: Snapshot::refresh SCENARIO 1 and INVARIANT 1 — nothing appended:
/// returns 0, changes nothing, never writes to the data directory.
#[test]
fn refresh_nothing_appended_returns_zero_and_writes_nothing() {
    let dir = test_dir("snap-zero");
    drop(books(&dir));
    let before = tree(&dir);
    let mut snap = Snapshot::load(&dir).unwrap();
    let seq = snap.journal_seq();
    let head = snap.head_hash().to_string();
    let ledger = snap.ledger().clone();
    assert_eq!(snap.refresh().unwrap(), 0);
    assert_eq!(snap.refresh().unwrap(), 0);
    assert_eq!(snap.journal_seq(), seq);
    assert_eq!(snap.head_hash(), head);
    assert_eq!(snap.ledger(), &ledger);
    assert_eq!(tree(&dir), before, "snapshot wrote to the data directory");
}

/// Oracle: Snapshot::refresh INVARIANTS 2 and 3 — picks up lines the live
/// writer appended; afterwards the ledger equals a replay of the lines read.
#[test]
fn refresh_follows_the_live_writer() {
    let dir = test_dir("snap-follow");
    let mut b = books(&dir);
    let mut snap = Snapshot::load(&dir).unwrap();
    assert_eq!(snap.ledger(), b.store.ledger());
    let n0 = raw_lines(&dir).len();
    b.store
        .add_account("6100", "Software", AccountType::Expense, None, false)
        .unwrap();
    b.store
        .attach_receipt(Some(b.e1), "r.pdf", "application/pdf", b"snap", None)
        .unwrap();
    let n1 = raw_lines(&dir).len();
    assert_eq!(snap.refresh().unwrap(), n1 - n0);
    assert_eq!(snap.journal_seq(), n1 as u64);
    assert_eq!(snap.journal().len(), n1);
    assert_eq!(snap.ledger(), b.store.ledger());
    let last = parsed_lines(&dir).pop().unwrap();
    assert_eq!(snap.head_hash(), sha_hex(canon(&last).as_bytes()));
    assert_eq!(snap.attachments(None).unwrap().len(), 1);
    assert_eq!(snap.refresh().unwrap(), 0);
    // Equal to a fresh replay.
    let fresh = Snapshot::load(&dir).unwrap();
    assert_eq!(snap.ledger(), fresh.ledger());
}

/// Oracle: Snapshot::refresh INVARIANT 2 — a line still being appended (no
/// newline yet) waits for the next refresh; once completed, it applies.
#[test]
fn refresh_waits_for_a_partial_line() {
    let dir = test_dir("snap-partial");
    let mut b = books(&dir);
    b.store
        .add_account("6100", "Software", AccountType::Expense, None, false)
        .unwrap();
    let full_ledger = b.store.ledger().clone();
    drop(b);
    let full = jbytes(&dir);
    let raws = raw_lines(&dir);
    let last = raws.last().unwrap().clone();
    let prefix_len = full.len() - last.len() - 1;
    let prefix = full[..prefix_len].to_vec();
    std::fs::write(journal_path(&dir), &prefix).unwrap();
    let mut snap = Snapshot::load(&dir).unwrap();
    let n = raws.len() as u64 - 1;
    assert_eq!(snap.journal_seq(), n);
    let before_ledger = snap.ledger().clone();

    // Half the line, no newline.
    let half = last.len() / 2;
    let mut partial = prefix.clone();
    partial.extend_from_slice(&last[..half]);
    std::fs::write(journal_path(&dir), &partial).unwrap();
    let tree_before = tree(&dir);
    assert_eq!(snap.refresh().unwrap(), 0);
    assert_eq!(snap.journal_seq(), n);
    assert_eq!(snap.ledger(), &before_ledger);
    // The whole line, still without its newline.
    let mut no_nl = prefix.clone();
    no_nl.extend_from_slice(&last);
    std::fs::write(journal_path(&dir), &no_nl).unwrap();
    assert_eq!(
        snap.refresh().unwrap(),
        0,
        "a line without its newline is not complete"
    );
    assert_eq!(snap.journal_seq(), n);
    assert_eq!(tree(&dir).len(), tree_before.len(), "refresh wrote a file");
    // Completed.
    std::fs::write(journal_path(&dir), &full).unwrap();
    assert_eq!(snap.refresh().unwrap(), 1);
    assert_eq!(snap.journal_seq(), n + 1);
    assert_eq!(snap.ledger(), &full_ledger);
    assert_eq!(jbytes(&dir), full, "refresh must never write");
}

/// Oracle: Snapshot::refresh SCENARIO 2 — the file shrank (a restore, a
/// repaired tail): reload from scratch.
#[test]
fn refresh_reloads_when_the_file_shrinks() {
    let dir = test_dir("snap-shrink");
    let mut b = books(&dir);
    let short = jbytes(&dir);
    let short_ledger = b.store.ledger().clone();
    let short_n = raw_lines(&dir).len() as u64;
    b.store
        .add_account("6100", "Software", AccountType::Expense, None, false)
        .unwrap();
    drop(b);
    let mut snap = Snapshot::load(&dir).unwrap();
    assert_eq!(snap.journal_seq(), short_n + 1);
    std::fs::write(journal_path(&dir), &short).unwrap();
    snap.refresh().unwrap();
    assert_eq!(snap.journal_seq(), short_n);
    assert_eq!(snap.ledger(), &short_ledger);
    assert_eq!(snap.journal().len() as u64, short_n);
}

/// Oracle: Snapshot::refresh SCENARIO 2 — the journal no longer chains (a
/// different books' journal restored in place): reload from scratch.
#[test]
fn refresh_reloads_when_the_chain_changes() {
    let dir = test_dir("snap-rechain");
    drop(books(&dir));
    let mut snap = Snapshot::load(&dir).unwrap();
    // Other books, longer, with a different history.
    let other = test_dir("snap-rechain-other");
    let mut o = Store::create(&other, Currency::USD).unwrap();
    for (code, name) in [("1001", "Bank"), ("6100", "Software"), ("6200", "Travel")] {
        o.add_account(code, name, AccountType::Expense, None, false)
            .unwrap();
    }
    for _ in 0..20 {
        o.add_class(&format!("class {}", o.journal_seq())).unwrap();
    }
    drop(o);
    let other_bytes = jbytes(&other);
    let ours = jbytes(&dir);
    assert!(other_bytes.len() > ours.len());
    assert!(
        !other_bytes.starts_with(&ours),
        "other books must not extend ours"
    );
    std::fs::write(journal_path(&dir), &other_bytes).unwrap();
    snap.refresh().unwrap();
    let fresh = Snapshot::load(&dir).unwrap();
    assert_eq!(snap.journal_seq(), fresh.journal_seq());
    assert_eq!(snap.head_hash(), fresh.head_hash());
    assert_eq!(snap.ledger(), fresh.ledger());
    let last = parsed_lines(&dir).pop().unwrap();
    assert_eq!(snap.head_hash(), sha_hex(canon(&last).as_bytes()));
}

/// Oracle: Snapshot::refresh SCENARIO 3 — a complete new line that fails
/// its parse is refused.
#[test]
fn refresh_refuses_an_unparseable_new_line() {
    let dir = test_dir("snap-badparse");
    drop(books(&dir));
    let mut snap = Snapshot::load(&dir).unwrap();
    let mut bytes = jbytes(&dir);
    bytes.extend_from_slice(b"not a journal line\n");
    std::fs::write(journal_path(&dir), &bytes).unwrap();
    assert!(snap.refresh().is_err());
    assert_eq!(jbytes(&dir), bytes, "refresh must never write");
}

/// Oracle: Snapshot::refresh SCENARIO 3 — a complete new line that fails
/// its hash is refused.
#[test]
fn refresh_refuses_a_bad_hash_new_line() {
    let dir = test_dir("snap-badhash");
    let mut b = books(&dir);
    b.store
        .add_account("6100", "Software", AccountType::Expense, None, false)
        .unwrap();
    drop(b);
    let full = jbytes(&dir);
    let raws = raw_lines(&dir);
    let last = raws.last().unwrap().clone();
    let prefix = full[..full.len() - last.len() - 1].to_vec();
    std::fs::write(journal_path(&dir), &prefix).unwrap();
    let mut snap = Snapshot::load(&dir).unwrap();
    // The real next line, its content altered: it no longer hashes to what
    // a following line would expect, and its own prev is fine, so alter the
    // prev instead to break the chain onto what the snapshot holds.
    let mut v: Value = serde_json::from_slice(&last).unwrap();
    v["prev"] = Value::from("ab".repeat(32));
    let mut bytes = prefix.clone();
    bytes.extend_from_slice(canon(&v).as_bytes());
    bytes.push(b'\n');
    std::fs::write(journal_path(&dir), &bytes).unwrap();
    assert!(snap.refresh().is_err());
    assert_eq!(jbytes(&dir), bytes);
}

// ------------------------------------------------- blob::path_for

/// Oracle: blob::path_for INVARIANTS 1 and 2 — only a 64-hex-digit id
/// names a blob; nothing resolves outside the blob directory, whatever the
/// input (observed through the public exists/get).
#[test]
fn blob_ids_are_only_sha256_hex() {
    let dir = test_dir("blob-ids");
    let root = dir.join("blobs");
    let blobs = BlobStore::open(&root).unwrap();
    let (sha, dedup) = blobs.put(b"blob bytes").unwrap();
    assert!(!dedup);
    assert_eq!(sha, sha_hex(b"blob bytes"));
    assert!(blobs.exists(&sha));
    assert_eq!(blobs.get(&sha).unwrap(), b"blob bytes");
    // Files outside the blob root that traversal would reach.
    std::fs::write(dir.join("secret"), b"outside").unwrap();
    let shard = root.join(&sha[..2]).join(&sha[2..4]);
    std::fs::write(shard.join("short"), b"not a blob").unwrap();
    let bad: Vec<String> = vec![
        String::new(),
        "a".into(),
        "abc".into(),
        sha[..63].to_string(),
        format!("{sha}0"),
        "../secret".into(),
        "../../secret".into(),
        "/etc/passwd".into(),
        format!("{}/{}/short", &sha[..2], &sha[2..4]),
        format!("../{}", &sha[3..]),
        "g".repeat(64),
        "é".repeat(32),
        "é".repeat(64),
        format!("{}/..", &sha[..61]),
        "..".repeat(32),
    ];
    for id in &bad {
        assert!(!blobs.exists(id), "{id:?} named a blob");
        assert!(blobs.get(id).is_err(), "{id:?} was served");
    }
    let ro = BlobStore::open_read_only(&root);
    for id in &bad {
        assert!(!ro.exists(id), "{id:?} named a blob (read-only)");
        assert!(ro.get(id).is_err());
    }
    assert!(ro.exists(&sha));
}
