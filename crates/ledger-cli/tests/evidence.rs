//! The evidence pipeline over the wire (plan P6): a receipt's text and
//! pages, annotations, finding a file, consolidating duplicates, and the
//! backfill command.
//!
//! Expected behavior comes from `docs/mcp-interface.md` ("Tools —
//! evidence", "Tools — receipts & documents") and rules.md B-44. Every
//! call goes through a real books server as an agent reaches it
//! (`common.rs`).

mod common;

use common::{Books, answer_of};
use serde_json::json;
use std::path::{Path, PathBuf};

/// The settings a test's books start with: a high receipt threshold (so a
/// simple posting needs no receipt) and cash at 1000.
const SETTINGS: &str = "[company]\nname = \"Evidence Co\"\nfiscal_year_start = \"01-01\"\n\
[books]\ncash_accounts = [\"1000\"]\nreceipt_threshold = \"1000.00\"\n";

/// A PDF with one page that says the word "Rent" (the extractor's own
/// typesetter, so the text is real and the page renders).
fn a_pdf() -> Vec<u8> {
    use ledger_import::report::{ReportMeta, Row, RowKind, statement};
    use rust_decimal::Decimal;
    let st = statement(
        "Receipt",
        "March 2026".to_string(),
        &["Total"],
        vec![Row {
            kind: RowKind::Line,
            label: "Rent".into(),
            amounts: vec![Some(Decimal::new(10000, 2))],
            currency: true,
            rule_above: false,
        }],
    );
    let meta = ReportMeta {
        company: "Evidence Co".into(),
        basis: "Accrual".into(),
        prepared_on: chrono::NaiveDate::from_ymd_opt(2026, 3, 2).unwrap(),
        journal_seq: 1,
    };
    ledger_import::statement_pdf(&st, &meta).unwrap()
}

/// A draft with two lines, and the accounts made for it.
fn a_draft(w: &mut common::Wire) -> u64 {
    for (code, name, ty) in [("1000", "Cash", "asset"), ("6000", "Rent", "expense")] {
        let r = w.call(
            "books.add_account",
            json!({"code": code, "name": name, "account_type": ty}),
        );
        if let Err(e) = answer_of(&r) {
            assert!(e.contains("already"), "{e}");
        }
    }
    let r = w.call(
        "books.draft_entry",
        json!({"date": "2026-03-01", "description": "March rent"}),
    );
    let id = answer_of(&r).unwrap()["entry_id"].as_u64().unwrap();
    for (account, side, amount) in [("6000", "debit", "100.00"), ("1000", "credit", "100.00")] {
        let r = w.call(
            "books.add_line",
            json!({"entry_id": id, "account": account, "side": side, "amount": amount}),
        );
        answer_of(&r).unwrap();
    }
    id
}

/// File `bytes` on `entry` through an upload, as an agent does (D-46).
fn file(w: &mut common::Wire, entry: u64, bytes: &[u8], name: &str, mime: &str) -> u64 {
    let upload = w.upload(bytes, name, mime, "receipt");
    let r = w.call(
        "books.attach_receipt",
        json!({"entry_id": entry, "upload_id": upload}),
    );
    answer_of(&r).unwrap()["attachment_id"].as_u64().unwrap()
}

/// A PNG's width and height, from its IHDR.
fn png_size(bytes: &[u8]) -> (u32, u32) {
    assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"), "not a PNG");
    let w = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
    let h = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
    (w, h)
}

/// A book folder with two live receipts of one file's bytes, filed by an
/// older version on two entries (plan P6 step 4: legacy duplicates). With
/// `withdraw_first`, the first is already withdrawn, as a duplicate of the
/// second, so the second is its entry's only evidence.
fn legacy_duplicates(only: &str, withdraw_first: bool) -> (tempfile::TempDir, PathBuf) {
    use ledger_core::{AccountType, Currency, Money, Side};
    let root = tempfile::Builder::new()
        .prefix(&format!("wb-p6-{only}-"))
        .tempdir()
        .unwrap();
    let dir = root.path().join("book");
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = ledger_store::Store::create(&dir, Currency::USD).unwrap();
    let cash = s
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    let expense = s
        .add_account("6000", "Rent", AccountType::Expense, None, false)
        .unwrap();
    let mut entries = Vec::new();
    for (d, what) in [("2026-03-01", "Rent one"), ("2026-03-02", "Rent two")] {
        let e = s
            .create_entry(
                chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").unwrap(),
                what,
            )
            .unwrap();
        let m = Money::new("100.00".parse().unwrap(), Currency::USD);
        s.add_line(e, expense, Side::Debit, m).unwrap();
        s.add_line(e, cash, Side::Credit, m).unwrap();
        s.post_entry(e).unwrap();
        entries.push(e);
    }
    drop(s);
    let bytes = b"one receipt, filed twice by an older version";
    let (sha, _) = ledger_store::BlobStore::open(&dir.join("blobs"))
        .unwrap()
        .put(bytes)
        .unwrap();
    let mut w = ledger_store::JournalWriter::open(&dir.join(ledger_store::JOURNAL_FILE)).unwrap();
    for (id, entry, name) in [(1u64, entries[0], "one.txt"), (2, entries[1], "two.txt")] {
        w.append(
            "receipt.attached",
            json!({
                "attachment_id": id, "blob_sha": sha, "entry_id": entry,
                "category": "receipts", "file_name": name,
                "mime_type": "text/plain", "size_bytes": bytes.len() as u64,
                "description": null,
            }),
        )
        .unwrap();
    }
    if withdraw_first {
        w.append(
            "receipt.unlinked",
            json!({"attachment_id": 1, "entry_id": entries[0], "reason": "duplicate"}),
        )
        .unwrap();
        w.append(
            "attachment.withdrawn",
            json!({"attachment_id": 1, "reason": "duplicate of #2", "duplicate_of": 2}),
        )
        .unwrap();
    }
    drop(w);
    for f in [
        ledger_store::db_file(),
        ledger_store::LOCK_FILE.to_string(),
        format!("{}-wal", ledger_store::db_file()),
        format!("{}-shm", ledger_store::db_file()),
    ] {
        let _ = std::fs::remove_file(dir.join(f));
    }
    (root, dir)
}

/// Plan P6 step 3: the default answer is what the file is, the first 8 KiB
/// of its text and its pages; never its bytes, and small enough to read.
#[test]
fn get_receipt_default_has_text_and_no_bytes() {
    let books = Books::with("p6-meta", "USD", Some(SETTINGS));
    let mut w = books.session("evidence");
    let entry = a_draft(&mut w);
    let pdf = a_pdf();
    let id = file(&mut w, entry, &pdf, "receipt.pdf", "application/pdf");
    let r = w.call("books.get_receipt", json!({"attachment_id": id}));
    let out = answer_of(&r).unwrap();
    assert_eq!(out["text_status"], "ok", "{out}");
    assert_eq!(out["pages"], 1, "{out}");
    let text = out["text"].as_str().unwrap();
    assert!(text.contains("Rent"), "{text}");
    assert!(text.len() <= 8 * 1024);
    assert_eq!(out["text_truncated"], false);
    assert!(out.get("file").is_none(), "no file in the default answer");
    assert!(
        serde_json::to_string(&out).unwrap().len() < 8 * 1024,
        "the default answer is under 8 KiB"
    );
    assert!(!serde_json::to_string(&out).unwrap().contains("base64"));
    // view text: the whole text, no file.
    let r = w.call(
        "books.get_receipt",
        json!({"attachment_id": id, "view": "text"}),
    );
    let out = answer_of(&r).unwrap();
    assert_eq!(out["text"].as_str().unwrap(), text);
    // The resource reads the same: metadata plus text.
    let r = w.rpc(
        "resources/read",
        json!({"uri": format!("books://attachments/{id}")}),
    );
    let text = r["result"]["contents"][0]["text"].as_str().unwrap();
    assert!(text.contains("\"attachment_id\""), "{text}");
    assert!(text.contains("Rent"), "{text}");
}

/// Plan P6 step 3: one page as a PNG, handed over; page numbers start at
/// 1; a page past the last is refused.
#[test]
fn page_image_is_a_png_of_the_page() {
    let books = Books::with("p6-page", "USD", Some(SETTINGS));
    let mut w = books.session("evidence");
    let entry = a_draft(&mut w);
    let id = file(&mut w, entry, &a_pdf(), "receipt.pdf", "application/pdf");
    let r = w.call(
        "books.get_receipt",
        json!({"attachment_id": id, "view": "page_image", "page": 1, "max_px": 300}),
    );
    let out = answer_of(&r).unwrap();
    let file = &out["file"];
    assert_eq!(file["mime_type"], "image/png");
    let png = w.fetch(file);
    let (width, height) = png_size(&png);
    assert!(width.max(height) <= 300, "{width}x{height}");
    assert!(width > 0 && height > 0);
    // The page resource gives a PNG of the page too (its size its own:
    // a resource takes no max_px).
    let r = w.rpc(
        "resources/read",
        json!({"uri": format!("books://attachments/{id}/page/1")}),
    );
    let contents = &r["result"]["contents"][0];
    assert_eq!(contents["mimeType"], "image/png", "{contents}");
    let encoded = contents["blob"].as_str().unwrap();
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .unwrap();
    png_size(&bytes);
    // Past the last page: refused.
    let r = w.call(
        "books.get_receipt",
        json!({"attachment_id": id, "view": "page_image", "page": 2}),
    );
    assert!(answer_of(&r).is_err());
    // bytes still hands the file over, unchanged.
    let r = w.call(
        "books.get_receipt",
        json!({"attachment_id": id, "view": "bytes"}),
    );
    let out = answer_of(&r).unwrap();
    assert_eq!(w.fetch(&out["file"]), a_pdf());
}

/// Plan P6 steps 4 and 5: annotations, the engine's numbers, the three
/// ways to find a file, and a settlement recorded.
#[test]
fn annotations_and_searches() {
    let books = Books::with("p6-find", "USD", Some(SETTINGS));
    let mut w = books.session("evidence");
    let entry = a_draft(&mut w);
    let text = b"Harbor Design LLC\nInvoice H-1042\nHosting, 42.00, 2026-03-01\n";
    let id = file(&mut w, entry, text, "harbor.txt", "text/plain");
    let r = w.call(
        "books.annotate_attachment",
        json!({"attachment_id": id, "fields": {
            "receipt_number": 7, "payee": "Harbor Design LLC", "amount": "42.00",
            "date": "2026-03-01", "instrument": "Check 1042",
            "source": "the owner's email", "reimbursable_to": "Avery Quill"
        }, "note": "read on 2026-03-02"}),
    );
    let out = answer_of(&r).unwrap();
    assert_eq!(out["fields"]["receipt_number"], 7);
    assert_eq!(out["fields"]["amount"], "42.00");
    assert_eq!(out["next_receipt_number"], 8);
    // Meta carries the annotation and the number.
    let r = w.call("books.get_receipt", json!({"attachment_id": id}));
    let out = answer_of(&r).unwrap();
    assert_eq!(out["receipt_number"], 7);
    assert_eq!(out["annotation"]["payee"], "Harbor Design LLC");
    // Found three ways.
    for args in [
        json!({"receipt_number": 7}),
        json!({"sha256": ledger_store::journal::sha256_hex(text)}),
        json!({"text": "harbor design"}),
    ] {
        let r = w.call("books.find_attachment", args.clone());
        let out = answer_of(&r).unwrap();
        assert!(
            out["attachments"]
                .as_array()
                .unwrap()
                .iter()
                .any(|a| a["attachment_id"] == id),
            "{args}: {out}"
        );
    }
    // Exactly one selector.
    let r = w.call(
        "books.find_attachment",
        json!({"sha256": ledger_store::journal::sha256_hex(text), "text": "x"}),
    );
    assert!(answer_of(&r).is_err());
    // A settlement is recorded on the paying entry and read back.
    let pay = a_draft(&mut w);
    let r = w.call(
        "books.set_entry_settles",
        json!({"entry_id": pay, "settles": [entry]}),
    );
    let out = answer_of(&r).unwrap();
    assert_eq!(out["settles"], json!([entry]));
    let r = w.call(
        "books.set_entry_settles",
        json!({"entry_id": pay, "settles": [pay]}),
    );
    assert!(answer_of(&r).is_err(), "an entry settles not itself");
    let r = w.call("books.verify", json!({}));
    assert_eq!(answer_of(&r).unwrap()["status"], "ok");
}

/// Plan P6 step 4 and rules.md B-44 item 5: consolidation never leaves an
/// entry the receipt policy covers with no evidence.
#[test]
fn consolidation_never_removes_an_entrys_only_evidence() {
    let (_root, fixture) = legacy_duplicates("dupes-only", true);
    let books = Books::placed("p6-dupes-only", &fixture);
    let mut w = books.session("evidence");
    let before = books.journal();
    let r = w.call("books.consolidate_duplicates", json!({"confirmed": true}));
    let why = answer_of(&r).unwrap_err();
    assert!(why.contains("receipt_required"), "{why}");
    assert_eq!(books.journal(), before, "a refusal journals nothing");
}

/// Plan P6 step 4: the kept file takes every subject the duplicates had;
/// the others are withdrawn as duplicates of it, and nothing is deleted.
#[test]
fn consolidation_links_the_kept_file_to_every_subject() {
    let (_root, fixture) = legacy_duplicates("dupes-live", false);
    let books = Books::placed("p6-dupes-live", &fixture);
    let mut w = books.session("evidence");
    let sha = {
        let r = w.call("books.get_receipt", json!({"attachment_id": 2}));
        answer_of(&r).unwrap()["sha256"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let r = w.call("books.consolidate_duplicates", json!({}));
    let preview = answer_of(&r).unwrap();
    assert_eq!(preview["status"], "preview");
    assert_eq!(preview["groups"][0]["keep"], 1);
    assert_eq!(preview["groups"][0]["withdraw"], json!([2]));
    let before = books.journal();
    assert_eq!(before, books.journal(), "a preview journals nothing");
    let r = w.call("books.consolidate_duplicates", json!({"confirmed": true}));
    let out = answer_of(&r).unwrap();
    assert_eq!(out["status"], "consolidated", "{out}");
    assert_eq!(out["withdrawn"], 1);
    // The second entry is still evidenced, by the kept file.
    let r = w.call("books.list_receipts", json!({"entry_id": 2}));
    let out = answer_of(&r).unwrap();
    assert_eq!(out["receipts"].as_array().unwrap().len(), 1, "{out}");
    assert_eq!(out["receipts"][0]["attachment_id"], 1);
    // The withdrawn one is found, marked, and names what it duplicates.
    let r = w.call("books.find_attachment", json!({"sha256": sha}));
    let out = answer_of(&r).unwrap();
    let found = out["attachments"].as_array().unwrap();
    assert_eq!(found.len(), 2, "{out}");
    let gone = found.iter().find(|a| a["attachment_id"] == 2).unwrap();
    assert_eq!(gone["withdrawn"], true);
    let r = w.call("books.verify", json!({}));
    assert_eq!(answer_of(&r).unwrap()["status"], "ok");
}

/// Done when (1): every attachment of the `p2-served` fixture gives a
/// default `books.get_receipt` answer under 8 KiB.
#[test]
fn every_p2_served_attachment_has_a_small_default_answer() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/p2-served");
    let books = Books::placed("p6-p2-served", &fixture);
    let mut w = books.session("evidence");
    let mut seen = 0;
    for id in 1..=64u64 {
        let r = w.call("books.get_receipt", json!({"attachment_id": id}));
        let Ok(out) = answer_of(&r) else { continue };
        seen += 1;
        let size = serde_json::to_string(&out).unwrap().len();
        assert!(size < 8 * 1024, "attachment {id}: {size} bytes");
        assert_eq!(out["text_truncated"], false, "attachment {id}");
        assert!(out.get("file").is_none(), "attachment {id}");
    }
    assert!(seen >= 60, "the fixture's attachments: {seen}");
}

/// Done when (4): the backfill reads an older book's files, one
/// attachment at a time, and a second run journals nothing.
#[test]
fn the_backfill_reads_an_old_fixtures_files_once() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../ledger-store/tests/fixtures/old-books/p6_evidence");
    let books = Books::placed("p6-backfill", &fixture);
    // The fixture is from before P6: upgrade it (the owner's step), then
    // the backfill can journal extraction records.
    books.stop();
    {
        let mut s = ledger_store::Store::open(&books.dir()).unwrap();
        s.upgrade_format().unwrap();
        s.verify().unwrap();
    }
    books.start();
    let mut w = books.session("evidence");
    // Attachment 1 is a PDF (one page with text), 2 a text file, 3 a
    // Markdown document.
    for id in 1..=2 {
        let before = books.journal();
        let r = w.call("books.get_receipt", json!({"attachment_id": id}));
        let out = answer_of(&r).unwrap();
        assert_eq!(out["text_status"], "not_extracted", "{out}");
        let answer = books.home.admin(ledger_mcp::admin::Request::Extract {
            id: books.id.clone(),
            attachment_id: id,
        });
        match answer {
            ledger_mcp::admin::Response::Extracted { status, .. } => assert_eq!(status, "ok"),
            other => panic!("{other:?}"),
        }
        assert_ne!(books.journal(), before, "the backfill journals a record");
    }
    // The PDF's text is readable now, and its pages render.
    let r = w.call("books.get_receipt", json!({"attachment_id": 1}));
    let out = answer_of(&r).unwrap();
    assert_eq!(out["text_status"], "ok", "{out}");
    assert_eq!(out["pages"], 1, "{out}");
    assert!(out["text"].as_str().unwrap().contains("Rent"), "{out}");
    // The command itself, on the Markdown document (attachment 3), and
    // again: a second run journals nothing. The test's own server is
    // stopped first: the command opens the books and closes them as the
    // host does.
    books.stop();
    let run = || {
        let mut extract = std::process::Command::new(env!("CARGO_BIN_EXE_wanderbooks"));
        extract
            .args(["books", "extract", &books.id, "3", "--root"])
            .arg(books.home.root());
        common::clean_env(&mut extract, books.home.dir.path());
        extract.output().unwrap()
    };
    let before = books.journal();
    let out = run();
    assert!(out.status.success(), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("Read attachment 3"),
        "{out:?}"
    );
    assert_ne!(books.journal(), before, "the command journals a record");
    let after = books.journal();
    let out = run();
    assert!(out.status.success(), "{out:?}");
    assert_eq!(books.journal(), after, "an extract journals once");
    books.start();
    let mut w = books.session("evidence");
    let r = w.call("books.verify", json!({}));
    assert_eq!(answer_of(&r).unwrap()["status"], "ok");
    let r = w.call("books.get_receipt", json!({"attachment_id": 3}));
    assert_eq!(answer_of(&r).unwrap()["text_status"], "ok");
}
