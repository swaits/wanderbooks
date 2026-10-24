//! Files that carry full TINs never enter the books (rules.md B-26;
//! tin_files.rs).
//!
//! ```text
//! GOAL: no file holding a full TIN can be filed as a receipt or document
//!       (books.get_receipt would hand its bytes to any agent), however it
//!       comes back: the W-9 books.set_w9 read, books.get_w9's copy, a CPA
//!       package or a file from one, or text with a TIN in it.
//! SOURCE: rules.md B-26; DECISION (P0 review round 2, 2026-10-16).
//! INVARIANTS (oracle):
//!   1. Every file the server read or wrote with full TINs is refused by
//!      every way a file enters the books (books.attach_receipt and
//!      books.file_document, its bytes uploaded as an agent uploads them:
//!      D-46), from any session and after the server restarts.
//!   2. Text holding a full TIN is refused, whoever made it: one the vault
//!      holds, in every common rendering (nine digits; 3-2-4 joined by
//!      hyphens, spaces or dots; 2-7 joined by a hyphen or a space), and,
//!      with or without the vault, a number printed as an SSN or EIN is
//!      (123-45-6789, 12-3456789) standing alone.
//!   3. Text probing for a TIN (more than two TIN-shaped numbers ending
//!      in a last four the books know) is refused alike, byte for byte,
//!      whether or not one of them is a TIN the vault holds; one or two
//!      such numbers are checked against the vault.
//!   4. A refusal stores nothing.
//!   5. Ordinary text goes in: a bank export with routing numbers, long
//!      references, and dates next to amounts (DECISION, owner,
//!      2026-10-16: bank exports are filed for reconciliation).
//! ```

use common::{Client, call};
use serde_json::{Value, json};
use std::io::Read;
use std::path::{Path, PathBuf};

mod common;

const SSN: &str = "291-12-2846";
const MEMBER_SSN: &str = "830-83-5531";

/// The refusal for a file the server read or wrote with TINs.
const TIN_FILE: &str = "read or wrote with full TINs";
/// The refusal for text holding a TIN the vault holds.
const TIN_HELD: &str = "a full TIN the books keep in the TIN vault";
/// The refusal for text probing for a TIN.
const TIN_PROBE: &str = "more than two numbers shaped like a TIN";
/// The refusal for text holding a number printed as an SSN or EIN is.
const TIN_PRINTED: &str = "printed the way an SSN or EIN is";

/// The test's books (common.rs: a home, its server, the books in its
/// root), their folder, a folder for files on their way in, and one
/// outside the root for files saved; the settings: a member, so the CPA
/// package has a K-1 worksheet.
struct Books {
    served: common::Books,
    dir: PathBuf,
    inbox: PathBuf,
    out: PathBuf,
    /// Each upload a refusal left unused, by its bytes' SHA-256: named
    /// again by the next call (files.rs INVARIANT 1), so the refusals
    /// never pile up past a token's MAX_LIVE.
    left: std::sync::Mutex<std::collections::HashMap<String, String>>,
}

fn books(tag: &str) -> Books {
    let tag = tag.to_string();
    // Made on a thread of its own: the settings go in through a session
    // with a runtime of its own (common.rs), never inside the test's.
    let served = std::thread::spawn(move || {
        common::Books::with(
            &format!("tin-files-{tag}"),
            "USD",
            Some(
                "[company]\nname = \"Tin Co\"\n[books]\nreceipt_threshold = \"0.00\"\ncash_accounts = [\"1000\"]\n\
                 [[company.members]]\nname = \"Mo\"\nshare = \"100%\"\ncapital_account = \"3000\"\n",
            ),
        )
    })
    .join()
    .unwrap();
    let base = served.home.dir.path().to_path_buf();
    let (inbox, out) = (base.join("inbox"), base.join("out"));
    for d in [&inbox, &out] {
        std::fs::create_dir_all(d).unwrap();
    }
    Books {
        dir: served.dir(),
        served,
        inbox,
        out,
        left: Default::default(),
    }
}

impl Books {
    /// An upload of `bytes` named `name` on `c` (the books' token's
    /// session), as an agent uploads them: one a refusal left unused if
    /// there is one, else a new one (purpose `purpose`). Its id.
    async fn upload(&self, c: &Client, bytes: &[u8], name: &str, purpose: &str) -> String {
        let sha = ledger_store::journal::sha256_hex(bytes);
        let key = format!("{purpose} {sha}");
        if let Some(id) = self.left.lock().unwrap().get(&key) {
            return id.clone();
        }
        let token = self.served.home.token(common::BOOK);
        let id = common::upload_on(
            c,
            &self.served.home.run(),
            common::BOOK,
            &token,
            bytes,
            (name, "application/octet-stream", purpose),
        )
        .await;
        self.left.lock().unwrap().insert(key, id.clone());
        id
    }

    /// The upload of `bytes` for `purpose` was filed: used up.
    fn spent(&self, bytes: &[u8], purpose: &str) {
        let key = format!("{purpose} {}", ledger_store::journal::sha256_hex(bytes));
        self.left.lock().unwrap().remove(&key);
    }
}

/// A new session with the books.
async fn connect(b: &Books) -> Client {
    common::connect(&b.served.home, common::BOOK).await.unwrap()
}

async fn ok(c: &Client, tool: &str, args: Value) -> Value {
    call(c, tool, args)
        .await
        .unwrap_or_else(|e| panic!("{tool}: {e}"))
}

/// The bytes of a file a tool handed over (a `HandedFile`): fetched by its
/// download, matching the size and SHA-256 it states.
async fn handed(b: &Books, file: &Value) -> Vec<u8> {
    let token = b.served.home.token(common::BOOK);
    common::fetch_export(&b.served.home, common::BOOK, &token, file).await
}

/// Every way the file at `path` (named `name`) could enter the books is
/// refused with `why`, and nothing is stored: the refusals, tool by tool.
async fn refused_everywhere(
    c: &Client,
    b: &Books,
    path: &Path,
    name: &str,
    why: &str,
) -> Vec<String> {
    let bytes = std::fs::read(path).unwrap();
    // One upload, refused by each tool and so left for the next.
    let upload = b.upload(c, &bytes, name, "receipt").await;
    let journal = b.dir.join(ledger_store::JOURNAL_FILE);
    let before = std::fs::read(&journal).unwrap();
    let mut refusals = Vec::new();
    for (tool, args) in [
        (
            "books.attach_receipt",
            json!({"upload_id": upload, "mime_type": "application/octet-stream"}),
        ),
        (
            "books.file_document",
            json!({"category": "tax", "upload_id": upload, "mime_type": "application/pdf"}),
        ),
    ] {
        let err = call(c, tool, args).await.expect_err(tool);
        assert!(err.contains(why), "{tool} {name}: {err}");
        refusals.push(err);
    }
    assert_eq!(
        std::fs::read(&journal).unwrap(),
        before,
        "a refusal stored something"
    );
    refusals
}

/// INVARIANT 1: the W-9 books.set_w9 read (binary, no TIN in its text), a
/// books.get_w9 copy of it, a 1099 worksheet and a CPA package with TINs,
/// and the K-1 worksheet extracted from that package are each refused,
/// from another session and after a restart; INVARIANT 2: a CSV holding a
/// TIN that the server never saw is refused, sent by upload.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_tin_bearing_file_is_refused_from_every_session_and_after_a_restart() {
    let b = books("sessions");
    // A W-9 scan: binary bytes, no TIN a text scan would see.
    let w9 = b.inbox.join("w9.pdf");
    std::fs::write(
        &w9,
        b"%PDF-1.4\n\xff\xfe\x00 scanned W-9 \x01\x02\x03\n%%EOF\n",
    )
    .unwrap();
    let one = connect(&b).await;
    let w9_upload = b
        .upload(&one, &std::fs::read(&w9).unwrap(), "w9.pdf", "w9")
        .await;
    for (code, name, ty) in [
        ("1000", "Cash", "asset"),
        ("3000", "Capital", "equity"),
        ("6000", "Design", "expense"),
    ] {
        ok(
            &one,
            "books.add_account",
            json!({"code": code, "name": name, "account_type": ty}),
        )
        .await;
    }
    ok(
        &one,
        "books.add_party",
        json!({"name": "Acme", "roles": ["contractor"]}),
    )
    .await;
    ok(
        &one,
        "books.add_party",
        json!({"name": "Mo", "roles": ["member"]}),
    )
    .await;
    ok(
        &one,
        "books.set_w9",
        json!({"party": "Acme", "received_on": "2026-01-03", "entity_type": "Individual",
               "tin": SSN, "tin_type": "ssn", "w9_upload_id": w9_upload}),
    )
    .await;
    b.spent(&std::fs::read(&w9).unwrap(), "w9");
    ok(
        &one,
        "books.set_tin",
        json!({"party": "Mo", "tin": MEMBER_SSN, "tin_type": "ssn"}),
    )
    .await;
    // A bank statement filed in the books: the package carries it, and it
    // is no TIN file (its routing numbers are not TINs).
    accepted_everywhere(&one, &b, "statement.csv", BANK_CSV).await;
    // What the server hands over with TINs, from the first session, saved
    // outside the books as the human would save it.
    let copy = b.out.join("w9-copy.pdf");
    let got = ok(&one, "books.get_w9", json!({"party": "Acme"})).await;
    std::fs::write(&copy, handed(&b, &got["file"]).await).unwrap();
    let csv = b.out.join("1099.csv");
    let got = ok(
        &one,
        "books.vendor_1099",
        json!({"year": 2026, "include_tins": true, "csv": true}),
    )
    .await;
    std::fs::write(&csv, handed(&b, &got["csv"]).await).unwrap();
    let zip = b.out.join("cpa.zip");
    let got = ok(
        &one,
        "books.export_cpa_package",
        json!({"cash_accounts": ["1000"], "as_of": "2026-12-31"}),
    )
    .await;
    assert_eq!(got["tins_included"], true, "{got}");
    std::fs::write(&zip, handed(&b, &got["file"]).await).unwrap();
    let k1 = b.out.join("k1-members.csv");
    {
        let mut archive = zip::ZipArchive::new(std::fs::File::open(&zip).unwrap()).unwrap();
        let mut member = archive.by_name("k1-members.csv").unwrap();
        let mut bytes = Vec::new();
        member.read_to_end(&mut bytes).unwrap();
        assert!(
            String::from_utf8_lossy(&bytes).contains(MEMBER_SSN),
            "the K-1 worksheet holds the TIN"
        );
        std::fs::write(&k1, bytes).unwrap();
    }
    let files = [
        (w9.clone(), "receipt.pdf"),
        (copy.clone(), "invoice.pdf"),
        (csv.clone(), "statement.csv"),
        (zip.clone(), "bundle.zip"),
        (k1.clone(), "k1-members.csv"),
    ];
    // A second session refuses each.
    let two = connect(&b).await;
    for (path, name) in &files {
        refused_everywhere(&two, &b, path, name, TIN_FILE).await;
    }
    // Text with a TIN the server never wrote: refused by its text.
    let fresh = b.out.join("payees.csv");
    std::fs::write(&fresh, format!("name,ssn\nDana,{}\n", "187-05-1907")).unwrap();
    refused_everywhere(&two, &b, &fresh, "payees.csv", TIN_PRINTED).await;
    // An ordinary file still goes in, every way.
    let fine = b
        .upload(
            &two,
            b"%PDF-1.4\n\xff lunch receipt, order 12-34\n",
            "lunch.pdf",
            "receipt",
        )
        .await;
    ok(
        &two,
        "books.attach_receipt",
        json!({"upload_id": fine, "mime_type": "application/pdf"}),
    )
    .await;
    // The books' own files the package carried are not marked: the
    // statement still goes in.
    {
        let archive = zip::ZipArchive::new(std::fs::File::open(&zip).unwrap()).unwrap();
        assert!(
            archive
                .file_names()
                .filter_map(Result::ok)
                .any(|n| n.starts_with("documents/") && n.ends_with("statement.csv")),
            "the package carries the statement"
        );
    }
    accepted_everywhere(&two, &b, "statement-again.csv", BANK_CSV).await;
    drop((one, two));
    b.served.restart();

    // A server started afresh on the same books refuses them all too.
    let three = connect(&b).await;
    for (path, name) in &files {
        refused_everywhere(&three, &b, path, name, TIN_FILE).await;
    }
    // The registry holds hashes only.
    let registry = std::fs::read_to_string(b.dir.join("vault/tin-files.sha256")).unwrap();
    assert!(registry.lines().count() >= files.len(), "{registry}");
    assert!(
        registry
            .lines()
            .all(|l| l.len() == 64 && l.chars().all(|c| c.is_ascii_hexdigit())),
        "{registry}"
    );
}

/// INVARIANT 1, the other way: a books.set_w9 that is refused (an unknown
/// party, a party without a 1099 role, a file that is no PDF, PNG or
/// JPEG) never marks the file it names: the same bytes are then filed as
/// an ordinary document. Once a set_w9 with the file succeeds, they are
/// refused everywhere.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refused_set_w9_never_marks_its_file() {
    let b = books("w9-refused");
    let w9 = b.inbox.join("w9.pdf");
    let w9_bytes = b"%PDF-1.4\n\xff\xfe\x00 a scanned W-9 \x04\x05\n%%EOF\n".to_vec();
    std::fs::write(&w9, &w9_bytes).unwrap();
    let notes = b.inbox.join("notes.txt");
    std::fs::write(&notes, b"Acme sent their W-9 on the 3rd.\n").unwrap();
    let c = connect(&b).await;
    for (code, name, ty) in [("1000", "Cash", "asset"), ("3000", "Capital", "equity")] {
        ok(
            &c,
            "books.add_account",
            json!({"code": code, "name": name, "account_type": ty}),
        )
        .await;
    }
    for (name, role) in [("Acme", "contractor"), ("Mo", "member")] {
        ok(
            &c,
            "books.add_party",
            json!({"name": name, "roles": [role]}),
        )
        .await;
    }
    let w9_args = |party: &str, upload: &str| {
        json!({"party": party, "received_on": "2026-01-03", "entity_type": "Individual",
               "tin": SSN, "tin_type": "ssn", "w9_upload_id": upload})
    };
    for (party, path, why) in [
        ("Nobody", &w9, "unknown party"),
        ("Mo", &w9, "1099"),
        ("Acme", &notes, "PDF, PNG, or JPEG"),
    ] {
        let bytes = std::fs::read(path).unwrap();
        let upload = b.upload(&c, &bytes, "w9.pdf", "w9").await;
        let err = call(&c, "books.set_w9", w9_args(party, &upload))
            .await
            .expect_err("refused");
        assert!(err.contains(why), "{party}: {err}");
    }
    // Neither file was marked: each is filed as any document is.
    let mut filed = Vec::new();
    for (name, bytes) in [
        ("w9-scan.pdf", w9_bytes.clone()),
        ("notes.txt", std::fs::read(&notes).unwrap()),
    ] {
        let upload = b.upload(&c, &bytes, name, "document").await;
        let out = ok(
            &c,
            "books.file_document",
            json!({"category": "other", "upload_id": upload, "mime_type": "application/pdf"}),
        )
        .await;
        b.spent(&bytes, "document");
        assert!(out["attachment_id"].is_u64(), "{name}: {out}");
        filed.push(out["attachment_id"].clone());
    }
    // A set_w9 that succeeds marks it.
    let upload = b.upload(&c, &w9_bytes, "w9.pdf", "w9").await;
    ok(&c, "books.set_w9", w9_args("Acme", &upload)).await;
    b.spent(&w9_bytes, "w9");
    // The scan filed before is withdrawn, so its bytes are no attachment
    // begin_upload names: they are uploaded, and refused.
    ok(
        &c,
        "books.withdraw_attachment",
        json!({"attachment_id": filed[0], "reason": "a W-9: it belongs in the vault"}),
    )
    .await;
    let upload = b.upload(&c, &w9_bytes, "again.pdf", "document").await;
    for (tool, args) in [
        (
            "books.file_document",
            json!({"category": "other", "upload_id": upload, "mime_type": "application/pdf"}),
        ),
        (
            "books.attach_receipt",
            json!({"upload_id": upload, "mime_type": "application/pdf"}),
        ),
    ] {
        let err = call(&c, tool, args).await.expect_err("marked");
        assert!(err.contains(TIN_FILE), "{tool}: {err}");
    }
}

/// Every way a file enters the books takes the text `name` holding
/// `text`, and the journal shows it. The same bytes filed already are
/// withdrawn first (books.begin_upload would name them instead of
/// taking them again), so each tool takes them itself.
async fn accepted_everywhere(c: &Client, b: &Books, name: &str, text: &str) {
    let token = b.served.home.token(common::BOOK);
    for (tool, mut args) in [
        ("books.attach_receipt", json!({"mime_type": "text/csv"})),
        (
            "books.file_document",
            json!({"category": "import_source", "mime_type": "text/csv"}),
        ),
    ] {
        let begun = ok(
            c,
            "books.begin_upload",
            common::upload_args(text.as_bytes(), name, "text/csv", "document"),
        )
        .await;
        if let Some(stored) = begun["existing_attachment_id"].as_u64() {
            ok(
                c,
                "books.withdraw_attachment",
                json!({"attachment_id": stored, "reason": "filed again another way"}),
            )
            .await;
        }
        let upload = common::upload_on(
            c,
            &b.served.home.run(),
            common::BOOK,
            &token,
            text.as_bytes(),
            (name, "text/csv", "document"),
        )
        .await;
        args["upload_id"] = json!(upload);
        let out = ok(c, tool, args).await;
        assert!(out["attachment_id"].is_u64(), "{tool}: {out}");
    }
}

/// A real bank export: ABA routing numbers (nine digits), account and
/// trace numbers, long references, dates run into amounts, a ZIP+4, a
/// phone number, and the number 12345 in a ten-digit run.
const BANK_CSV: &str = "\
Date,Description,Amount,Balance,Reference
2026-10-01,ACH CREDIT APPLE INC 021000021 ID 9878143210123456,1499.00,\"12,345.67\",20261001000123456789
2026-10-02,WIRE IN 026009593 ACCT 4417123456789 TRACE 0210000210012345,250000.00,262345.67,FT26275987814321
2026-10-03 12.50,POS 2026-10-03 801-555-1234 SALT LAKE UT 84101-1234,-12.50,262333.17,1002345678
2026-10-04,ZELLE 5551234567 CONF 1123-45-6789 REF 123-45-67890,-40.00,262293.17,000123456789012
2026-10-05,CHECK 1042 SEQ 026 00 9593 ITEM 112-3456789,-100.00,262193.17,ROUTING 228620358
";

/// INVARIANTS 2-4 through the tools: a member's TIN the vault holds is
/// refused in each rendering by every filing tool; a printed SSN or EIN
/// shape is refused with the vault locked as with it open, while the
/// vault's TIN written bare then goes in (only the shapes are left); a
/// bank export goes in either way.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn text_with_a_vault_tin_or_a_printed_one_is_refused_and_a_bank_export_is_not() {
    let b = books("text");
    let c = connect(&b).await;
    ok(
        &c,
        "books.add_party",
        json!({"name": "Mo", "roles": ["member"]}),
    )
    .await;
    ok(
        &c,
        "books.set_tin",
        json!({"party": "Mo", "tin": MEMBER_SSN, "tin_type": "ssn"}),
    )
    .await;
    let renderings = [
        "830835531",
        "830-83-5531",
        "830 83 5531",
        "830.83.5531",
        "83-0835531",
        "83 0835531",
    ];
    for (n, tin) in renderings.iter().enumerate() {
        let path = b.out.join(format!("members-{n}.csv"));
        std::fs::write(&path, format!("member,tin\nKat,{tin}\n")).unwrap();
        refused_everywhere(&c, &b, &path, &format!("members-{n}.csv"), TIN_HELD).await;
    }
    for (n, printed) in ["Dana,187-05-1907", "Acme LLC,20-5200615"]
        .iter()
        .enumerate()
    {
        let path = b.out.join(format!("payees-{n}.csv"));
        std::fs::write(&path, format!("payee,tin\n{printed}\n")).unwrap();
        refused_everywhere(&c, &b, &path, &format!("payees-{n}.csv"), TIN_PRINTED).await;
    }
    accepted_everywhere(&c, &b, "bank.csv", BANK_CSV).await;
    drop(c);

    // The vault locked (its key gone): the printed shapes are still
    // refused, and the bank export still goes in; the vault's TIN written
    // bare is no longer recognized.
    b.served.stop();
    std::fs::remove_file(b.dir.join("keys/legacy.key")).unwrap();
    b.served.start();
    let c = connect(&b).await;
    let path = b.out.join("payees-0.csv");
    refused_everywhere(&c, &b, &path, "payees-0.csv", TIN_PRINTED).await;
    let path = b.out.join("members-1.csv");
    refused_everywhere(&c, &b, &path, "members-1.csv", TIN_PRINTED).await;
    accepted_everywhere(
        &c,
        &b,
        "bank-2.csv",
        &format!("{BANK_CSV}2026-10-06,x,1.00,1.00,1\n"),
    )
    .await;
    accepted_everywhere(&c, &b, "members-0.csv", "member,tin\nKat,830835531\n").await;
}

/// INVARIANT 3b through the tools: the company's own EIN, as its
/// settings hold it, goes in printed 2-7 (formation papers, a business
/// plan) by every filing tool; another EIN, or the same nine digits
/// printed as an SSN, is still refused.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_company_s_own_ein_goes_in_and_no_other() {
    let b = books("own-ein");
    let c = connect(&b).await;
    call(
        &c,
        "books.set_setting",
        json!({"changes": [{"key": "company.ein", "value": "12-3456789"}],
               "decided_by": "Test Owner"}),
    )
    .await
    .unwrap();
    accepted_everywhere(
        &c,
        &b,
        "plan.md",
        "# Tin Co business plan\n\nEIN 12-3456789, formed 2026-10-04.\n",
    )
    .await;
    for (n, text) in [
        "EIN 12-3456789; our vendor's is 20-5200615",
        "as an SSN: 123-45-6789",
    ]
    .iter()
    .enumerate()
    {
        let path = b.out.join(format!("other-{n}.md"));
        std::fs::write(&path, text).unwrap();
        refused_everywhere(&c, &b, &path, &format!("other-{n}.md"), TIN_PRINTED).await;
    }
}

/// INVARIANT 3 through the tools: UTF-16 text written without a
/// byte-order mark (ASCII so written is valid UTF-8, each digit followed
/// or led by a NUL) is read as the text it is, either way round: a vault
/// TIN and a printed SSN in it are refused by every filing tool,
/// `books.file_document` included, and nothing is stored.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn utf16_text_without_a_byte_order_mark_is_scanned_either_way_round() {
    let b = books("utf16");
    let c = connect(&b).await;
    ok(
        &c,
        "books.add_party",
        json!({"name": "Mo", "roles": ["member"]}),
    )
    .await;
    ok(
        &c,
        "books.set_tin",
        json!({"party": "Mo", "tin": MEMBER_SSN, "tin_type": "ssn"}),
    )
    .await;
    let bare = |text: &str, le: bool| -> Vec<u8> {
        text.encode_utf16()
            .flat_map(|u| if le { u.to_le_bytes() } else { u.to_be_bytes() })
            .collect()
    };
    for (le, side) in [(true, "le"), (false, "be")] {
        for (text, why, tag) in [
            ("member,tin\nKat,830835531\n", TIN_HELD, "held"),
            ("payee,tin\nDana,187-05-1907\n", TIN_PRINTED, "printed"),
        ] {
            let bytes = bare(text, le);
            assert!(
                le && bytes[1] == 0 || !le && bytes[0] == 0,
                "NUL-interleaved, no byte-order mark"
            );
            let name = format!("utf16-{side}-{tag}.csv");
            let path = b.out.join(&name);
            std::fs::write(&path, &bytes).unwrap();
            refused_everywhere(&c, &b, &path, &name, why).await;
        }
    }
}

/// INVARIANT 3 through the tools: text with three numbers ending in the
/// last four of a TIN the books know (a member's TIN in the vault, a
/// payee's W-9 kept elsewhere) is refused by every filing tool, byte for
/// byte alike whether one of them is the vault's TIN or none is, with the
/// vault open or locked; one such number goes in unless it is the TIN; a
/// bank export goes in.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn text_probing_for_a_tin_is_refused_alike_by_every_filing_tool() {
    let b = books("probe");
    let c = connect(&b).await;
    ok(
        &c,
        "books.add_party",
        json!({"name": "Mo", "roles": ["member"]}),
    )
    .await;
    ok(
        &c,
        "books.set_tin",
        json!({"party": "Mo", "tin": MEMBER_SSN, "tin_type": "ssn"}),
    )
    .await;
    // A W-9 kept at the CPA's: the books know its last four, the vault
    // holds nothing for it.
    ok(
        &c,
        "books.add_party",
        json!({"name": "Acme", "roles": ["contractor"]}),
    )
    .await;
    ok(
        &c,
        "books.set_w9",
        json!({"party": "Acme", "received_on": "2026-01-03", "entity_type": "C corporation",
               "tin_last4": "7788", "kept_at": "CPA portal"}),
    )
    .await;
    let write = |name: &str, text: &str| {
        let path = b.out.join(name);
        std::fs::write(&path, text).unwrap();
        path
    };
    let with_tin = write(
        "with.csv",
        "member,ref\nKat,830835531\nA,864065531\nB,846825531\n",
    );
    let without = write(
        "without.csv",
        "member,ref\nKat,873685531\nA,864065531\nB,846825531\n",
    );
    let payee = write("payee.csv", "ref\n100007788\n100017788\n100027788\n");
    let one = write("one.csv", "member,ref\nKat,873685531\n");
    let tin = write("tin.csv", "member,ref\nKat,830835531\n");
    let probes = async |c: &Client| {
        let a = refused_everywhere(c, &b, &with_tin, "with.csv", TIN_PROBE).await;
        let z = refused_everywhere(c, &b, &without, "without.csv", TIN_PROBE).await;
        assert_eq!(a, z, "the refusal tells whether the vault's TIN is there");
        refused_everywhere(c, &b, &payee, "payee.csv", TIN_PROBE).await;
        a
    };
    let open = probes(&c).await;
    refused_everywhere(&c, &b, &tin, "tin.csv", TIN_HELD).await;
    accepted_everywhere(&c, &b, "one.csv", &std::fs::read_to_string(&one).unwrap()).await;
    accepted_everywhere(&c, &b, "bank.csv", BANK_CSV).await;
    drop(c);

    // The vault locked: the same refusals, word for word.
    b.served.stop();
    std::fs::remove_file(b.dir.join("keys/legacy.key")).unwrap();
    b.served.start();
    let c = connect(&b).await;
    assert_eq!(
        probes(&c).await,
        open,
        "the refusal tells whether the vault is open"
    );
}
