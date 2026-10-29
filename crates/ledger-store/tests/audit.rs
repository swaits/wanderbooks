//! Audits and attestations in the store (plan P8; rules.md B-45): the
//! `audits` feature, the figure head, idempotent runs, attestations that
//! rest on a head, and the integrity findings only the store can see.

use chrono::NaiveDate;
use ledger_core::audit::{Effort, Method, Scope};
use ledger_core::{AccountType, Currency, Money, Side};
use ledger_store::audit::{self, Attested, BackupStanding, Claim, NewAuditRun, backup_findings};
use ledger_store::format::{AUDITS, JournalFormat};
use ledger_store::{BlobStore, JOURNAL_FILE, JournalWriter, Store, StoreError};
use rust_decimal::Decimal;
use serde_json::json;
use std::path::PathBuf;

struct TestDir(PathBuf);

impl std::ops::Deref for TestDir {
    type Target = std::path::Path;
    fn deref(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn test_dir(name: &str) -> TestDir {
    let dir = std::env::temp_dir().join(format!("wb-audit-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    TestDir(dir)
}

fn d(m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, m, day).unwrap()
}

fn usd(s: &str) -> Money {
    Money::new(s.parse().unwrap(), Currency::USD)
}

struct Fx {
    bank: u64,
    capital: u64,
    supplies: u64,
    /// A posted 40.00 purchase.
    entry: u64,
}

/// A bank, capital and an expense account, and a posted 40.00 purchase on
/// the 5th.
fn books(dir: &std::path::Path, format: Option<&JournalFormat>) -> (Store, Fx) {
    let mut s = match format {
        Some(f) => Store::create_in_format(dir, Currency::USD, f).unwrap(),
        None => Store::create(dir, Currency::USD).unwrap(),
    };
    let bank = s
        .add_account("1000", "Bank", AccountType::Asset, None, false)
        .unwrap();
    let capital = s
        .add_account("3000", "Capital", AccountType::Equity, None, false)
        .unwrap();
    let supplies = s
        .add_account("6000", "Supplies", AccountType::Expense, None, false)
        .unwrap();
    let e = s.create_entry(d(1, 1), "Opening").unwrap();
    s.add_line(e, bank, Side::Debit, usd("500.00")).unwrap();
    s.add_line(e, capital, Side::Credit, usd("500.00")).unwrap();
    s.post_entry(e).unwrap();
    let entry = s.create_entry(d(1, 5), "Printer paper").unwrap();
    s.add_line(entry, supplies, Side::Debit, usd("40.00"))
        .unwrap();
    s.add_line(entry, bank, Side::Credit, usd("40.00")).unwrap();
    s.post_entry(entry).unwrap();
    (
        s,
        Fx {
            bank,
            capital,
            supplies,
            entry,
        },
    )
}

fn run_of(s: &Store, through: NaiveDate, scope: Scope) -> NewAuditRun {
    NewAuditRun {
        through,
        through_seq: s.figure_head().unwrap(),
        scope,
        tb_hash: "tb".into(),
        findings_sha256: "fs".into(),
        workpaper_attachment: None,
        workpaper_pdf: None,
    }
}

fn claim(effort: &str) -> Claim {
    Claim {
        by: "verifier-1".into(),
        method: Method {
            model: Some("opus".into()),
            reasoning_effort: Effort::parse(effort),
            adversarial: Some(true),
            verifier_agents: Some(2),
        },
        note: None,
    }
}

fn journal_bytes(dir: &std::path::Path) -> Vec<u8> {
    std::fs::read(dir.join(JOURNAL_FILE)).unwrap()
}

/// R-A: every write of the feature waits for the owner's upgrade,
/// journaling nothing; afterwards it is taken and the books replay to
/// what was live.
#[test]
fn audit_writes_wait_for_audits() {
    let dir = test_dir("waits");
    let (mut s, fx) = books(&dir, Some(&JournalFormat::default()));
    assert!(!s.journal_format().has_feature(AUDITS));
    let (att, _, _) = s
        .attach_receipt(Some(fx.entry), "r.txt", "text/plain", b"paper 40.00", None)
        .unwrap();
    let before = journal_bytes(&dir);
    let new = run_of(&s, d(1, 31), Scope::Month);
    for r in [
        s.record_audit_run(new.clone()).map(|_| ()),
        s.attest_receipt(
            att,
            fx.entry,
            "40.00".parse().unwrap(),
            d(1, 5),
            claim("high"),
        )
        .map(|_| ()),
        s.review_entry(fx.entry, claim("high")).map(|_| ()),
    ] {
        match r {
            Err(StoreError::FormatUpgradePending { need }) => {
                assert_eq!(need, "the journal feature audits")
            }
            other => panic!("expected FormatUpgradePending, got {other:?}"),
        }
    }
    assert_eq!(journal_bytes(&dir), before, "a refusal journals nothing");
    s.upgrade_format().unwrap();
    let run = s
        .record_audit_run(run_of(&s, d(1, 31), Scope::Month))
        .unwrap();
    s.review_entry(fx.entry, claim("high")).unwrap();
    s.verify().unwrap();
    drop(s);
    let s = Store::open(&dir).unwrap();
    s.verify().unwrap();
    assert_eq!(s.audit_runs().unwrap(), vec![run]);
    assert_eq!(s.attestations().unwrap().len(), 1);
}

/// INVARIANT 1 and 2: filing a workpaper and journaling the run do not
/// move the figure head; an entry does; the same audit asked again
/// returns the first run and journals nothing.
#[test]
fn a_run_is_idempotent_on_the_figure_head() {
    let dir = test_dir("idem");
    let (mut s, fx) = books(&dir, None);
    let head = s.figure_head().unwrap();
    let (wp, _, _) = s
        .file_document("workpaper", "w.json", "application/json", b"{}", None)
        .unwrap();
    assert_eq!(s.figure_head().unwrap(), head, "a filing is not a figure");
    let mut new = run_of(&s, d(1, 31), Scope::Month);
    new.workpaper_attachment = Some(wp);
    let first = s.record_audit_run(new.clone()).unwrap();
    assert_eq!(first.id, 1);
    assert_eq!(first.through_seq, head);
    assert_eq!(s.figure_head().unwrap(), head, "a run is not a figure");
    let bytes = journal_bytes(&dir);
    let again = s.record_audit_run(new).unwrap();
    assert_eq!(again, first);
    assert_eq!(journal_bytes(&dir), bytes, "asking again journals nothing");
    assert!(s.audit_is_current(&first).unwrap());
    // Another scope is another run.
    let other = s
        .record_audit_run(run_of(&s, d(1, 31), Scope::Full))
        .unwrap();
    assert_eq!(other.id, 2);
    // An entry moves the head: the run no longer stands.
    let e = s.create_entry(d(2, 1), "Stamps").unwrap();
    s.add_line(e, fx.supplies, Side::Debit, usd("3.00"))
        .unwrap();
    s.add_line(e, fx.bank, Side::Credit, usd("3.00")).unwrap();
    s.post_entry(e).unwrap();
    assert!(s.figure_head().unwrap() > head);
    assert!(!s.audit_is_current(&first).unwrap());
    s.verify().unwrap();
}

/// A receipt reading that differs from the entry's is recorded, marked,
/// and never counts; one that agrees is not marked. The receipt must be on
/// the entry.
#[test]
fn amount_mismatch_never_counts() {
    let dir = test_dir("mismatch");
    let (mut s, fx) = books(&dir, None);
    let (att, _, _) = s
        .attach_receipt(Some(fx.entry), "r.txt", "text/plain", b"paper", None)
        .unwrap();
    let other = s
        .attach_receipt(None, "x.txt", "text/plain", b"stray", None)
        .unwrap()
        .0;
    let amount = |t: &str| -> Decimal { t.parse().unwrap() };
    let disagrees = |a: &ledger_store::audit::Attestation| match a.what {
        Attested::Receipt { disagrees, .. } => disagrees,
        _ => panic!("not a receipt"),
    };
    let ok = s
        .attest_receipt(att, fx.entry, amount("40.00"), d(1, 5), claim("high"))
        .unwrap();
    assert!(!disagrees(&ok));
    let wrong_amount = s
        .attest_receipt(att, fx.entry, amount("41.00"), d(1, 5), claim("high"))
        .unwrap();
    assert!(disagrees(&wrong_amount));
    let wrong_date = s
        .attest_receipt(att, fx.entry, amount("40.00"), d(1, 6), claim("high"))
        .unwrap();
    assert!(disagrees(&wrong_date));
    // Refused: a receipt not on the entry, a draft, an unknown entry.
    assert!(matches!(
        s.attest_receipt(other, fx.entry, amount("40.00"), d(1, 5), claim("high")),
        Err(StoreError::AuditRefused(_))
    ));
    let draft = s.create_entry(d(2, 2), "Draft").unwrap();
    assert!(matches!(
        s.review_entry(draft, claim("high")),
        Err(StoreError::AuditRefused(_))
    ));
    assert!(s.review_entry(999, claim("high")).is_err());
    // All three kinds replay, in order.
    s.verify().unwrap();
    drop(s);
    let s = Store::open(&dir).unwrap();
    let got = s.attestations().unwrap();
    assert_eq!(got.len(), 3);
    assert!(got.windows(2).all(|w| w[0].seq < w[1].seq));
    assert_eq!(got[0].method.reasoning_effort, Some(Effort::High));
    assert_eq!(got[0].by, "verifier-1");
}

/// INVARIANT 3: a period is attested only on a run at the current head
/// that covers it; an entry since makes the run stale and the attestation
/// is refused until the audit is run again.
#[test]
fn stale_head_period_attestation_refused() {
    let dir = test_dir("stale");
    let (mut s, fx) = books(&dir, None);
    let jan = s.create_period("2026-01", d(1, 1), d(1, 31)).unwrap();
    // No audit yet.
    assert!(matches!(
        s.attest_period(jan, claim("high")),
        Err(StoreError::AuditRefused(_))
    ));
    // An audit through before the period's end does not cover it.
    s.record_audit_run(run_of(&s, d(1, 15), Scope::Month))
        .unwrap();
    assert!(matches!(
        s.attest_period(jan, claim("high")),
        Err(StoreError::AuditRefused(_))
    ));
    let run = s
        .record_audit_run(run_of(&s, d(1, 31), Scope::Month))
        .unwrap();
    let (a, on) = s.attest_period(jan, claim("high")).unwrap();
    assert_eq!(on.id, run.id);
    assert_eq!(
        a.what,
        Attested::Period {
            period_id: jan,
            audit_run_id: run.id
        }
    );
    // An entry since: the run is stale.
    let e = s.create_entry(d(2, 1), "Stamps").unwrap();
    s.add_line(e, fx.supplies, Side::Debit, usd("3.00"))
        .unwrap();
    s.add_line(e, fx.capital, Side::Credit, usd("3.00"))
        .unwrap();
    s.post_entry(e).unwrap();
    let head = s.head();
    match s.attest_period(jan, claim("high")) {
        Err(StoreError::AuditRefused(why)) => assert!(why.contains("books.audit"), "{why}"),
        other => panic!("expected a refusal, got {other:?}"),
    }
    assert_eq!(s.head(), head, "a refusal journals nothing");
    s.verify().unwrap();
}

/// The integrity table: each code seeded alone.
#[test]
fn integrity_detects_each_seeded_defect() {
    // A whole book finds nothing.
    {
        let dir = test_dir("whole");
        let (s, _) = books(&dir, None);
        assert_eq!(s.integrity_findings(), vec![]);
    }
    let seed = |code: &str| -> Vec<&'static str> {
        let dir = test_dir(&format!("seed-{code}"));
        let (mut s, fx) = books(&dir, None);
        let found = match code {
            "blob_missing" | "blob_damaged" | "extraction_blob_missing" => {
                let (_, sha, _) = s
                    .attach_receipt(Some(fx.entry), "r.txt", "text/plain", b"paper 40.00", None)
                    .unwrap();
                let target = if code == "extraction_blob_missing" {
                    // The text the extractor read, content-addressed too.
                    let x = s.extractions().unwrap().pop().expect("an extraction");
                    x.text_sha256.expect("text")
                } else {
                    sha
                };
                let path = dir.join("blobs").join(&target[..2]).join(&target);
                let path = if path.exists() {
                    path
                } else {
                    // The store's own layout.
                    walk(&dir.join("blobs"), &target).expect("the blob file")
                };
                if code == "blob_damaged" {
                    std::fs::write(&path, b"tampered").unwrap();
                } else {
                    std::fs::remove_file(&path).unwrap();
                }
                s.integrity_findings()
            }
            "duplicate_blob" => {
                drop(s);
                let bytes = b"filed twice by an older version";
                let (sha, _) = BlobStore::open(&dir.join("blobs"))
                    .unwrap()
                    .put(bytes)
                    .unwrap();
                let mut w = JournalWriter::open(&dir.join(JOURNAL_FILE)).unwrap();
                for (id, name) in [(1, "one.pdf"), (2, "two.pdf")] {
                    w.append(
                        "receipt.attached",
                        json!({
                            "attachment_id": id, "blob_sha": sha, "entry_id": null,
                            "category": "receipts", "file_name": name,
                            "mime_type": "application/pdf", "size_bytes": bytes.len() as u64,
                            "description": null,
                        }),
                    )
                    .unwrap();
                }
                drop(w);
                Store::open(&dir).unwrap().integrity_findings()
            }
            "access_log_damaged" => {
                std::fs::write(
                    dir.join("access.jsonl"),
                    b"{\"not\": \"a chained record\"}\n",
                )
                .unwrap();
                s.integrity_findings()
            }
            "projection_differs" => {
                let db = ledger_store::database_in_use(&dir);
                let conn = rusqlite::Connection::open(&db).unwrap();
                conn.execute("DELETE FROM lines", []).unwrap();
                drop(conn);
                s.integrity_findings()
            }
            "journal_damaged" => {
                let path = dir.join(JOURNAL_FILE);
                let text = std::fs::read_to_string(&path).unwrap();
                std::fs::write(&path, text.replacen("Printer paper", "Printer papers", 1)).unwrap();
                s.integrity_findings()
            }
            "backup_stale" => backup_findings(&[BackupStanding {
                target: "folder".into(),
                stale: true,
                last_error: None,
                drill_error: None,
            }]),
            "drill_failed" => backup_findings(&[BackupStanding {
                target: "folder".into(),
                stale: false,
                last_error: None,
                drill_error: Some("restore did not reach the head".into()),
            }]),
            other => panic!("no seed for {other}"),
        };
        found.iter().map(|f| f.code).collect()
    };
    for row in audit::CODES {
        let got = seed(row.code);
        assert!(got.contains(&row.code), "{}: found {got:?}", row.code);
        // The seed implies nothing else but its own consequences: a
        // missing or damaged file is found once; a damaged journal can
        // also differ from its replay.
        for c in &got {
            assert!(
                *c == row.code
                    || (row.code == "journal_damaged" && *c == "projection_differs")
                    // verify reads a file's text too, so a damaged or missing
                    // stored file is also a replay that cannot be made.
                    || (row.code.starts_with("blob_") || row.code == "extraction_blob_missing")
                        && [
                            "blob_missing",
                            "blob_damaged",
                            "extraction_blob_missing",
                            "projection_differs",
                        ]
                            .contains(c),
                "{}: also found {c}",
                row.code
            );
        }
    }
    // No backups set up is the owner's standing choice, not a finding.
    assert!(backup_findings(&[]).is_empty());
}

fn walk(dir: &std::path::Path, name: &str) -> Option<PathBuf> {
    for e in std::fs::read_dir(dir).ok()? {
        let p = e.ok()?.path();
        if p.is_dir() {
            if let Some(f) = walk(&p, name) {
                return Some(f);
            }
        } else if p.file_name().is_some_and(|n| n == name) {
            return Some(p);
        }
    }
    None
}

/// 8.0.6 item 2: the books the version before P8 wrote
/// (`old-books/p8_before`) open to the same journal, have no audit run or
/// claim, are whole (nothing for the integrity checks to find), refuse
/// the new writes until the owner's upgrade, and take them after it.
#[test]
fn books_the_version_before_p8_wrote_wait_for_the_upgrade() {
    let from =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/old-books/p8_before");
    let dir = test_dir("p8-before");
    std::fs::create_dir_all(dir.join("blobs")).unwrap();
    std::fs::copy(from.join(JOURNAL_FILE), dir.join(JOURNAL_FILE)).unwrap();
    fn copy(from: &std::path::Path, to: &std::path::Path) {
        std::fs::create_dir_all(to).unwrap();
        for e in std::fs::read_dir(from).unwrap() {
            let e = e.unwrap();
            if e.file_type().unwrap().is_dir() {
                copy(&e.path(), &to.join(e.file_name()));
            } else {
                std::fs::copy(e.path(), to.join(e.file_name())).unwrap();
            }
        }
    }
    copy(&from.join("blobs"), &dir.join("blobs"));
    let mut s = Store::open(&dir).unwrap();
    s.verify().unwrap();
    assert!(!s.journal_format().has_feature(AUDITS));
    assert!(s.audit_runs().unwrap().is_empty());
    assert!(s.attestations().unwrap().is_empty());
    assert_eq!(s.integrity_findings(), vec![]);
    // The head is the last figure line; the filings after it don't move it.
    let head = s.figure_head().unwrap();
    assert!(head > 0 && head <= s.head().0);
    let jan = s.ledger().list_periods()[0].id;
    let before = journal_bytes(&dir);
    assert!(matches!(
        s.record_audit_run(run_of(&s, d(1, 31), Scope::Month)),
        Err(StoreError::FormatUpgradePending { .. })
    ));
    assert!(matches!(
        s.attest_period(jan, claim("high")),
        Err(StoreError::FormatUpgradePending { .. })
    ));
    assert_eq!(journal_bytes(&dir), before, "refused: nothing journaled");
    s.upgrade_format().unwrap();
    let run = s
        .record_audit_run(run_of(&s, d(1, 31), Scope::Month))
        .unwrap();
    s.attest_period(jan, claim("high")).unwrap();
    assert_eq!(s.figure_head().unwrap(), head, "claims are not figures");
    s.verify().unwrap();
    drop(s);
    let s = Store::open(&dir).unwrap();
    s.verify().unwrap();
    assert_eq!(s.audit_runs().unwrap(), vec![run]);
}

/// Done-when 2: books with an unfooted cash flow, a posted entry with no
/// receipt, an open draft and a duplicate file give exactly those four
/// findings: the statements' and the store's, together.
#[test]
fn four_defects_give_exactly_four_findings() {
    use ledger_core::audit::{AuditInput, statements};
    let dir = test_dir("four");
    let (mut s, fx) = {
        let mut s = Store::create(&dir, Currency::USD).unwrap();
        let bank = s
            .add_account("1000", "Bank", AccountType::Asset, None, false)
            .unwrap();
        let card = s
            .add_account("2050", "Card", AccountType::Liability, None, false)
            .unwrap();
        let capital = s
            .add_account("3000", "Capital", AccountType::Equity, None, false)
            .unwrap();
        let supplies = s
            .add_account_with_tax(
                "6000",
                "Supplies",
                AccountType::Expense,
                None,
                false,
                Some(("supplies", None)),
            )
            .unwrap();
        let e = s.create_entry(d(1, 1), "Opening").unwrap();
        s.add_line(e, bank, Side::Debit, usd("500.00")).unwrap();
        s.add_line(e, capital, Side::Credit, usd("500.00")).unwrap();
        s.post_entry(e).unwrap();
        s.attach_receipt(Some(e), "open.txt", "text/plain", b"deposit 500.00", None)
            .unwrap();
        // A purchase over the threshold with no receipt: posted through the
        // primitive, as a hand entry made before the rule was tightened.
        let bare = s.create_entry(d(1, 5), "Printer paper").unwrap();
        s.add_line(bare, supplies, Side::Debit, usd("40.00"))
            .unwrap();
        s.add_line(bare, bank, Side::Credit, usd("40.00")).unwrap();
        s.post_entry(bare).unwrap();
        // An open draft.
        let draft = s.create_entry(d(1, 9), "Stamps").unwrap();
        s.add_line(draft, supplies, Side::Debit, usd("5.00"))
            .unwrap();
        s.add_line(draft, bank, Side::Credit, usd("5.00")).unwrap();
        (s, (bank, card, bare))
    };
    // A duplicate: two records an older version filed for one file.
    drop(s);
    let bytes = b"filed twice by an older version";
    let (sha, _) = BlobStore::open(&dir.join("blobs"))
        .unwrap()
        .put(bytes)
        .unwrap();
    let mut w = JournalWriter::open(&dir.join(JOURNAL_FILE)).unwrap();
    for (id, name) in [(10, "one.pdf"), (11, "two.pdf")] {
        w.append(
            "receipt.attached",
            json!({
                "attachment_id": id, "blob_sha": sha, "entry_id": null,
                "category": "receipts", "file_name": name,
                "mime_type": "application/pdf", "size_bytes": bytes.len() as u64,
                "description": null,
            }),
        )
        .unwrap();
    }
    drop(w);
    s = Store::open(&dir).unwrap();
    let (bank, card, _) = fx;
    // The statement of cash flows with a liability named as cash: the
    // engine refuses it, and that refusal is the finding.
    let params = ledger_core::CashFlowParams {
        start: d(1, 1),
        end: d(1, 31),
        cash_accounts: vec![bank, card],
        financing_accounts: vec![],
        investing_accounts: vec![],
        members: vec![],
    };
    let made = ledger_core::cash_flow(s.ledger(), &params)
        .map(|c| c.reconciles)
        .map_err(|e| e.to_string());
    assert!(made.is_err(), "a liability is not a cash account");
    let receipts = s.entries_with_receipts().unwrap();
    let mut found = statements(
        s.ledger(),
        &AuditInput {
            settings: s.ledger().settings(),
            receipts: &receipts,
            cash_flow: Some(&made),
            settings_problems: &[],
            undecided_1099: &[],
        },
        d(1, 31),
    );
    found.extend(s.integrity_findings());
    ledger_core::audit::sort(&mut found);
    let mut codes: Vec<&str> = found.iter().map(|f| f.code).collect();
    codes.sort_unstable();
    assert_eq!(
        codes,
        [
            "cash_flow_unfooted",
            "draft_open",
            "duplicate_blob",
            "evidence_missing"
        ],
        "{found:#?}"
    );
}
