//! Each projection schema in a file of its own (ledger_store::db_file_for;
//! plan/served.md 3.9: before the owner consents to a format upgrade, the
//! way back to an older version is always there; rules.md B-27).
//!
//! ```text
//! GOAL: an older version, gone back to, finds the books it can open,
//!       however far a newer one moved the projection.
//! INVARIANTS:
//!   1. This version opens or creates only its own schema's file (the
//!      legacy `wanderbooks.db` only when that already holds its schema);
//!      its own missing, it is built from the journal.
//!   2. A file of any other schema, older or newer, is never migrated,
//!      written, refused or deleted: it is left byte for byte, and still
//!      whole for its own schema.
//!   3. A damaged file of this version's own schema is kept aside and
//!      rebuilt, as ever.
//!   4. Which file is in use, and which others are there, is said.
//! ```
//!
//! The P2-shaped fixture (tests/fixtures/p2-books) is a books dir as P2
//! left it: a format 1 journal and its projection in `wanderbooks.db`, at
//! schema 12. While this version's schema is 12 too, it is used as it is;
//! once a schema of P3's moves past it, it must be left byte for byte. A
//! legacy file one schema older than this version's is made here too, so
//! the second case runs on every version.

use chrono::NaiveDate;
use ledger_core::{AccountType, Currency, Money, Side};
use ledger_store::schema::{MIGRATIONS, SCHEMA_VERSION};
use ledger_store::{
    DiskCheck, JOURNAL_FILE, JournalFormat, LEGACY_DB_FILE, Snapshot, Store, db_file, db_file_for,
};
use rust_decimal::Decimal;
use std::path::{Path, PathBuf};

/// The schema P2 kept in `wanderbooks.db`, the last before each schema had
/// a file of its own.
const P2_SCHEMA: u32 = 12;

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
        "wanderbooks-schema-files-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    TestDir(dir)
}

fn usd(n: i64) -> Money {
    Money::new(Decimal::from(n), Currency::USD)
}

/// Books with a chart and one posted entry (cash 500), closed again, in
/// journal format `format`.
fn books(dir: &Path, format: &JournalFormat) {
    let mut s = Store::create_in_format(dir, Currency::USD, format).unwrap();
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
}

/// The cash balance the books say.
fn cash(s: &Store) -> Decimal {
    let id = s.ledger().chart().find_by_code("1000").unwrap().id;
    s.ledger().account_balance(id).unwrap()
}

/// A file's bytes, or None.
fn bytes(path: &Path) -> Option<Vec<u8>> {
    std::fs::read(path).ok()
}

/// What a projection file says of itself, read without writing it: its
/// schema, the journal lines it applied, and SQLite's integrity check.
fn holds(path: &Path) -> (u32, u64, String) {
    let uri = format!("file:{}?mode=ro&immutable=1", path.display());
    let conn = rusqlite::Connection::open_with_flags(
        uri,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )
    .unwrap();
    let meta = |k: &str| -> String {
        conn.query_row("SELECT value FROM meta WHERE key=?1", [k], |r| r.get(0))
            .unwrap()
    };
    let check: String = conn
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .unwrap();
    (
        meta("schema_version").parse().unwrap(),
        meta("last_applied_seq").parse().unwrap(),
        check,
    )
}

/// A copy of tests/fixtures/p2-books.
fn p2_books() -> TestDir {
    let from = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/p2-books");
    let dir = test_dir("p2");
    for name in [JOURNAL_FILE, LEGACY_DB_FILE] {
        std::fs::copy(from.join(name), dir.join(name)).unwrap();
    }
    dir
}

/// How tests/fixtures/p2-books was written: by this code while its schema
/// was P2's (12, and every line applied as P2 applies it), its projection
/// then given P2's file name. Run with WB_WRITE_P2_FIXTURE=DIR.
#[test]
fn write_the_p2_fixture() {
    let Ok(out) = std::env::var("WB_WRITE_P2_FIXTURE") else {
        return;
    };
    assert_eq!(
        SCHEMA_VERSION, P2_SCHEMA,
        "only a P2-schema version writes it"
    );
    let out = PathBuf::from(out);
    books(&out, &JournalFormat::default());
    std::fs::rename(out.join(db_file()), out.join(LEGACY_DB_FILE)).unwrap();
    let _ = std::fs::remove_file(out.join(ledger_store::LOCK_FILE));
    let _ = std::fs::remove_dir(out.join("blobs"));
}

/// INVARIANTS 1, 2 and 4, on a books dir exactly as P2 left it: with this
/// version's schema P2's, its file is this version's and is used as it is
/// (no rebuild); past it, the file is left byte for byte and whole for its
/// schema, and this version builds its own from the journal. Either way the
/// books open to the same figures and verify.
#[test]
fn a_p2_books_dirs_database_is_used_at_its_schema_and_left_alone_past_it() {
    let dir = p2_books();
    let legacy = dir.join(LEGACY_DB_FILE);
    let before = bytes(&legacy).unwrap();
    let lines = std::fs::read_to_string(dir.join(JOURNAL_FILE))
        .unwrap()
        .lines()
        .count() as u64;
    assert_eq!(holds(&legacy), (P2_SCHEMA, lines, "ok".into()));
    let s = Store::open(&dir).unwrap();
    assert_eq!(cash(&s), Decimal::from(500));
    s.verify().unwrap();
    if SCHEMA_VERSION == P2_SCHEMA {
        // Its schema: used as it is, nothing rebuilt or made beside it.
        assert_eq!(s.database_file(), LEGACY_DB_FILE);
        assert!(s.other_databases().is_empty());
        drop(s);
        assert!(!dir.join(db_file()).exists(), "no rebuild for nothing");
    } else {
        assert_eq!(s.database_file(), db_file());
        assert_eq!(s.other_databases(), [LEGACY_DB_FILE]);
        drop(s);
        assert_eq!(bytes(&legacy).unwrap(), before, "P2's file, byte for byte");
        assert_eq!(holds(&legacy), (P2_SCHEMA, lines, "ok".into()), "and whole");
        let s = Store::open(&dir).unwrap();
        assert_eq!(cash(&s), Decimal::from(500));
        drop(s);
        assert_eq!(bytes(&legacy).unwrap(), before);
    }
}

/// INVARIANTS 1, 2 and 4, on every version: a legacy file one schema older
/// than this version's (as P2's is to P3's) is never migrated or written.
/// It stays byte for byte, and whole for its own schema; this version's
/// file is built from the journal and caught up as the books move on; the
/// report names the one in use and the one left alone; the viewer checks
/// the one in use.
#[test]
fn an_older_schemas_file_is_left_byte_for_byte() {
    let dir = test_dir("older");
    books(&dir, &JournalFormat::default());
    let own = dir.join(db_file());
    let legacy = dir.join(LEGACY_DB_FILE);
    // The older version's file: its schema, its own copy of the books.
    let older = SCHEMA_VERSION - 1;
    {
        let conn = rusqlite::Connection::open(&legacy).unwrap();
        for m in &MIGRATIONS[..older as usize] {
            conn.execute_batch(m).unwrap();
        }
        for (k, v) in [
            ("schema_version", older.to_string()),
            ("base_currency", "USD".into()),
            ("last_applied_seq", "0".into()),
        ] {
            conn.execute(
                "INSERT OR REPLACE INTO meta(key, value) VALUES (?1, ?2)",
                [k, v.as_str()],
            )
            .unwrap();
        }
    }
    std::fs::remove_file(&own).unwrap();
    let before = bytes(&legacy).unwrap();
    let mut s = Store::open(&dir).unwrap();
    assert_eq!(s.database_file(), db_file());
    assert_eq!(s.other_databases(), [LEGACY_DB_FILE]);
    assert_eq!(cash(&s), Decimal::from(500));
    let report = s.verify_and_heal().unwrap();
    assert_eq!(report.healed, None);
    assert_eq!(report.database, db_file());
    assert_eq!(report.other_databases, [LEGACY_DB_FILE]);
    // The books move on: this version's file follows, the other doesn't.
    s.add_account("6000", "Fees", AccountType::Expense, None, false)
        .unwrap();
    let head = s.head().0;
    drop(s);
    assert_eq!(bytes(&legacy).unwrap(), before, "never migrated or written");
    assert!(!dir.join(format!("{LEGACY_DB_FILE}-wal")).exists());
    assert_eq!(
        holds(&legacy),
        (older, 0, "ok".into()),
        "whole, at its schema"
    );
    assert_eq!(holds(&own), (SCHEMA_VERSION, head, "ok".into()));
    let snap = Snapshot::load(&dir).unwrap();
    assert_eq!(snap.check_disk(), DiskCheck::Matches);
    let s = Store::open(&dir).unwrap();
    s.verify().unwrap();
    drop(s);
    assert_eq!(bytes(&legacy).unwrap(), before);
}

/// INVARIANT 2: a newer schema's file, whether under its own name or the
/// legacy one, is left alone, never refused: the books open on this
/// version's own file (built from the journal if it isn't there).
#[test]
fn a_newer_schemas_file_is_left_alone_not_refused() {
    let dir = test_dir("newer");
    books(&dir, &JournalFormat::default());
    let own = dir.join(db_file());
    // A newer version's files: its own name, and (as no version writes, but
    // one could find) the legacy name holding a newer schema.
    let newer = dir.join(db_file_for(SCHEMA_VERSION + 1));
    let legacy = dir.join(LEGACY_DB_FILE);
    for f in [&newer, &legacy] {
        std::fs::copy(&own, f).unwrap();
        let conn = rusqlite::Connection::open(f).unwrap();
        conn.execute(
            "UPDATE meta SET value=?1 WHERE key='schema_version'",
            [(SCHEMA_VERSION + 1).to_string()],
        )
        .unwrap();
        conn.execute_batch("PRAGMA journal_mode=DELETE").unwrap();
    }
    let (newer_bytes, legacy_bytes) = (bytes(&newer).unwrap(), bytes(&legacy).unwrap());
    // This version's own file there, then not.
    for keep_own in [true, false] {
        if !keep_own {
            std::fs::remove_file(&own).unwrap();
        }
        let s = Store::open(&dir).unwrap();
        assert_eq!(s.database_file(), db_file(), "{keep_own}");
        let mut others = vec![LEGACY_DB_FILE.to_string(), db_file_for(SCHEMA_VERSION + 1)];
        others.sort();
        assert_eq!(s.other_databases(), others);
        assert_eq!(cash(&s), Decimal::from(500));
        s.verify().unwrap();
        drop(s);
        assert_eq!(bytes(&newer).unwrap(), newer_bytes, "{keep_own}");
        assert_eq!(bytes(&legacy).unwrap(), legacy_bytes, "{keep_own}");
    }
}

/// INVARIANT 3: this version's own file, damaged (not a database, or
/// holding another schema though named for this one), is kept aside and
/// rebuilt from the journal; a damaged legacy file is another version's to
/// mend, and left as it is.
#[test]
fn a_damaged_file_of_this_versions_schema_is_rebuilt() {
    let dir = test_dir("damaged");
    books(&dir, &JournalFormat::default());
    let own = dir.join(db_file());
    let kept_aside = |dir: &Path| {
        std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".corrupt."))
            .count()
    };
    // Not a database at all.
    std::fs::write(&own, b"this is not a sqlite database at all").unwrap();
    for wal in ["-wal", "-shm"] {
        let _ = std::fs::remove_file(dir.join(format!("{}{wal}", db_file())));
    }
    let s = Store::open(&dir).unwrap();
    assert_eq!(cash(&s), Decimal::from(500));
    s.verify().unwrap();
    drop(s);
    assert_eq!(kept_aside(&dir), 1);
    assert_eq!(holds(&own).0, SCHEMA_VERSION);
    // Named for this schema, holding another.
    {
        let conn = rusqlite::Connection::open(&own).unwrap();
        conn.execute(
            "UPDATE meta SET value=?1 WHERE key='schema_version'",
            [(SCHEMA_VERSION + 1).to_string()],
        )
        .unwrap();
    }
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let s = Store::open(&dir).unwrap();
    s.verify().unwrap();
    drop(s);
    assert_eq!(kept_aside(&dir), 2);
    assert_eq!(holds(&own).0, SCHEMA_VERSION);
    // A damaged legacy file: not this version's to mend.
    let legacy = dir.join(LEGACY_DB_FILE);
    std::fs::write(&legacy, b"garbage of an older version's").unwrap();
    let s = Store::open(&dir).unwrap();
    assert_eq!(s.database_file(), db_file());
    s.verify().unwrap();
    drop(s);
    assert_eq!(
        bytes(&legacy).unwrap(),
        b"garbage of an older version's".to_vec()
    );
    assert_eq!(kept_aside(&dir), 2);
}

/// New books get this version's own schema's file, named for its schema,
/// and nothing else.
#[test]
fn new_books_get_their_schemas_own_file() {
    let dir = test_dir("new");
    let s = Store::create(&dir, Currency::USD).unwrap();
    assert_eq!(s.database_file(), db_file());
    assert_eq!(db_file(), format!("wanderbooks.v{SCHEMA_VERSION}.db"));
    assert!(s.other_databases().is_empty());
    drop(s);
    assert!(dir.join(db_file()).exists());
    assert!(!dir.join(LEGACY_DB_FILE).exists());
    for (name, is) in [
        ("wanderbooks.db", true),
        ("wanderbooks.v13.db", true),
        ("wanderbooks.v1.db", true),
        ("wanderbooks.v.db", false),
        ("wanderbooks.v13.db-wal", false),
        ("wanderbooks.v13a.db", false),
        ("wanderbooks.v13.corrupt.20261021T000000Z", false),
        ("journal.jsonl", false),
    ] {
        assert_eq!(ledger_store::is_database_file(name), is, "{name}");
    }
}

/// A shipped migration's text never changes (P3 review, 2026-10-21: the
/// assets' v13 columns were first added to the v13 already on main, so a
/// v13 file built before them would open without them). Each schema's
/// file is built once by these scripts; a change is a new migration. The
/// SHA-256 of each shipped one, v1 to v13 (append the next as it ships).
#[test]
fn a_shipped_migration_never_changes() {
    use sha2::{Digest, Sha256};
    const SHIPPED: [&str; 13] = [
        "8b253dded247d8518a7454b1c894a147ac211ee32ea2671dca2c586c5184c046",
        "9fc19f844ed33b8f08eee8437ee3eb038e854737d5febe79ce3ac80abeb18614",
        "215af9a7a54cda382f721488fedb34f68eb36399730de1b23371d44eb16fd79d",
        "7923eb31ab8a08f20b8d7df017814095861478cb796af14994dbb85dbe558c20",
        "95caa91ee3327b5590ad1ddefa76bad189950091cfcd140121f978e0af258e8a",
        "74181a819beb4349808b9a095fbcda0073890fa6f54bc3fa5535c5a6644e35c6",
        "df008d2a7cf832173908d1e33f795b88d272eae9cd23a5ebebe7207871d7076f",
        "d0cd600ef4aa7bc2301e682bace23f3ce311b5f991e5d7bff447a9520ca8e89f",
        "b2e54e8cf47b7f55f42f3fd4876fce5183e229aea08059518a50cbbffdca2db2",
        "8881527992c586f69644276db921e640fb6548ee1d2f7c313a46c53e1e63430e",
        "cc18b31e929b5889c0d09190b3211ccefd9d73df8e1e05c93f9e460097e1553a",
        "dd2a93d760350f2edd9462f4a52888c0645040582739d3a93b17a3195238d501",
        "9aa8a0ea7ea15315a4e38281e3a6d4e14beb26333991ebb2824d9c8cc5a51941",
    ];
    for (i, want) in SHIPPED.iter().enumerate() {
        let got: String = Sha256::digest(MIGRATIONS[i].as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(&got, want, "migration v{} changed after it shipped", i + 1);
    }
    assert_eq!(MIGRATIONS.len() as u32, SCHEMA_VERSION);
}
