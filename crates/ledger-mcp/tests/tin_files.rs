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
//!      books.file_document, its bytes given inline), from any session and
//!      after the server restarts.
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

use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, ClientConfig};
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::StreamableHttpClientTransport;
use serde_json::{Value, json};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

type Client = RunningService<RoleClient, ClientConfig>;

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

struct Server(Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// The books dir, a folder for files on their way in, a folder outside
/// both, and the settings (a member, so the CPA package has a K-1
/// worksheet).
struct Books {
    dir: PathBuf,
    inbox: PathBuf,
    out: PathBuf,
    toml: PathBuf,
}

/// The books dir and the folders beside it go when the test ends, pass or
/// panic.
impl Drop for Books {
    fn drop(&mut self) {
        if let Some(base) = self.dir.parent() {
            let _ = std::fs::remove_dir_all(base);
        }
    }
}

fn books(tag: &str) -> Books {
    let base =
        std::env::temp_dir().join(format!("ledger-mcp-tin-files-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let dir = base.join("books");
    let inbox = base.join("inbox");
    let out = base.join("out");
    for d in [&dir, &inbox, &out, &base.join("config")] {
        std::fs::create_dir_all(d).unwrap();
    }
    let toml = dir.join("wanderbooks.toml");
    std::fs::write(
        &toml,
        format!(
            "[company]\nname = \"Tin Co\"\n[books]\nreceipt_threshold = \"0.00\"\ncash_accounts = [\"1000\"]\n\
             [[company.members]]\nname = \"Mo\"\nshare = \"100%\"\ncapital_account = \"3000\"\n\
             [vault]\nkey_file = \"{}\"\n",
            base.join("config/vault.key").display()
        ),
    )
    .unwrap();
    Books {
        dir,
        inbox,
        out,
        toml,
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// Start the server over HTTP on these books; every client is a session
/// of its own.
fn serve(b: &Books) -> (Server, String) {
    let addr = format!("127.0.0.1:{}", free_port());
    let base = b.dir.parent().unwrap();
    let child = Command::new(env!("CARGO_BIN_EXE_ledger-mcp"))
        .arg("--data-dir")
        .arg(b.dir.join("books-data"))
        .arg("--books-toml")
        .arg(&b.toml)
        .arg("--http")
        .arg(&addr)
        .env("XDG_CONFIG_HOME", base.join("config"))
        .env("XDG_RUNTIME_DIR", base.join("config"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    (Server(child), format!("http://{addr}/mcp"))
}

async fn connect(url: &str) -> Client {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let transport = StreamableHttpClientTransport::from_uri(url.to_string());
        match ClientConfig::default().serve(transport).await {
            Ok(c) => return c,
            Err(e) if Instant::now() > deadline => panic!("connect: {e}"),
            Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
        }
    }
}

/// Call a tool: Ok(structured result) or Err(the tool's error text).
async fn call(c: &Client, tool: &str, args: Value) -> Result<Value, String> {
    let args = args.as_object().cloned().unwrap_or_default();
    let r = c
        .call_tool(CallToolRequestParams::new(tool.to_string()).with_arguments(args))
        .await
        .map_err(|e| format!("protocol: {e}"))?;
    let text = r
        .content
        .iter()
        .find_map(|c| c.as_text().map(|t| t.text.clone()))
        .unwrap_or_default();
    if r.is_error == Some(true) {
        Err(text)
    } else {
        Ok(r.structured_content
            .unwrap_or_else(|| serde_json::from_str(&text).unwrap()))
    }
}

async fn ok(c: &Client, tool: &str, args: Value) -> Value {
    call(c, tool, args)
        .await
        .unwrap_or_else(|e| panic!("{tool}: {e}"))
}

/// `bytes` as a tool takes them inline: base64, and their SHA-256 (hex).
fn inline(bytes: &[u8]) -> (String, String) {
    use base64::Engine;
    (
        base64::engine::general_purpose::STANDARD.encode(bytes),
        ledger_store::journal::sha256_hex(bytes),
    )
}

/// The bytes of a file a tool handed over (a `HandedFile`): inline, as on
/// books served from a --data-dir, matching the size and SHA-256 it
/// states.
fn handed(file: &Value) -> Vec<u8> {
    use base64::Engine;
    assert!(file.get("download").is_none(), "{file}");
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(file["content_base64"].as_str().expect("content_base64"))
        .unwrap();
    assert_eq!(file["size_bytes"], json!(bytes.len()), "{file}");
    assert_eq!(
        file["sha256"],
        json!(ledger_store::journal::sha256_hex(&bytes)),
        "{file}"
    );
    bytes
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
    let (content, sha) = inline(&bytes);
    let journal = b.dir.join("books-data").join(ledger_store::JOURNAL_FILE);
    let before = std::fs::read(&journal).unwrap();
    let mut refusals = Vec::new();
    for (tool, args) in [
        (
            "books.attach_receipt",
            json!({"file_name": name, "mime_type": "application/octet-stream",
                   "content_base64": content, "sha256": sha}),
        ),
        (
            "books.file_document",
            json!({"category": "tax", "file_name": name, "mime_type": "application/pdf",
                   "content_base64": content, "sha256": sha}),
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
/// TIN that the server never saw is refused, sent as base64.
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
    let (w9_content, w9_sha) = inline(&std::fs::read(&w9).unwrap());
    let (server, url) = serve(&b);
    let one = connect(&url).await;
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
               "tin": SSN, "tin_type": "ssn", "w9_content_base64": w9_content, "w9_sha256": w9_sha}),
    )
    .await;
    ok(
        &one,
        "books.set_tin",
        json!({"party": "Mo", "tin": MEMBER_SSN, "tin_type": "ssn"}),
    )
    .await;
    // A bank statement filed in the books: the package carries it, and it
    // is no TIN file (its routing numbers are not TINs).
    accepted_everywhere(&one, "statement.csv", BANK_CSV).await;
    // What the server hands over with TINs, from the first session, saved
    // outside the books as the human would save it.
    let copy = b.out.join("w9-copy.pdf");
    let got = ok(&one, "books.get_w9", json!({"party": "Acme"})).await;
    std::fs::write(&copy, handed(&got["file"])).unwrap();
    let csv = b.out.join("1099.csv");
    let got = ok(
        &one,
        "books.vendor_1099",
        json!({"year": 2026, "include_tins": true, "csv": true}),
    )
    .await;
    std::fs::write(&csv, handed(&got["csv"])).unwrap();
    let zip = b.out.join("cpa.zip");
    let got = ok(
        &one,
        "books.export_cpa_package",
        json!({"cash_accounts": ["1000"], "as_of": "2026-12-31"}),
    )
    .await;
    assert_eq!(got["tins_included"], true, "{got}");
    std::fs::write(&zip, handed(&got["file"])).unwrap();
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
    let two = connect(&url).await;
    for (path, name) in &files {
        refused_everywhere(&two, &b, path, name, TIN_FILE).await;
    }
    // Text with a TIN the server never wrote: refused by its text.
    let fresh = b.out.join("payees.csv");
    std::fs::write(&fresh, format!("name,ssn\nDana,{}\n", "187-05-1907")).unwrap();
    refused_everywhere(&two, &b, &fresh, "payees.csv", TIN_PRINTED).await;
    // An ordinary file still goes in, every way.
    let (fine, fine_sha) = inline(b"%PDF-1.4\n\xff lunch receipt, order 12-34\n");
    ok(
        &two,
        "books.attach_receipt",
        json!({"file_name": "lunch.pdf", "mime_type": "application/pdf",
               "content_base64": fine, "sha256": fine_sha}),
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
    accepted_everywhere(&two, "statement-again.csv", BANK_CSV).await;
    drop((one, two));
    drop(server);

    // A server started afresh on the same books refuses them all too.
    let (_server, url) = serve(&b);
    let three = connect(&url).await;
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
    let (_server, url) = serve(&b);
    let c = connect(&url).await;
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
    let w9_args = |party: &str, path: &Path| {
        let (content, sha) = inline(&std::fs::read(path).unwrap());
        json!({"party": party, "received_on": "2026-01-03", "entity_type": "Individual",
               "tin": SSN, "tin_type": "ssn", "w9_content_base64": content, "w9_sha256": sha})
    };
    for (party, path, why) in [
        ("Nobody", &w9, "unknown party"),
        ("Mo", &w9, "1099"),
        ("Acme", &notes, "PDF, PNG, or JPEG"),
    ] {
        let err = call(&c, "books.set_w9", w9_args(party, path))
            .await
            .expect_err("refused");
        assert!(err.contains(why), "{party}: {err}");
    }
    // Neither file was marked: each is filed as any document is.
    for (name, bytes) in [
        ("w9-scan.pdf", w9_bytes.clone()),
        ("notes.txt", std::fs::read(&notes).unwrap()),
    ] {
        let (content, sha) = inline(&bytes);
        let out = ok(
            &c,
            "books.file_document",
            json!({"category": "other", "file_name": name, "mime_type": "application/pdf",
                   "content_base64": content, "sha256": sha}),
        )
        .await;
        assert!(out["attachment_id"].is_u64(), "{name}: {out}");
    }
    // A set_w9 that succeeds marks it.
    ok(&c, "books.set_w9", w9_args("Acme", &w9)).await;
    let (content, sha) = inline(&w9_bytes);
    for (tool, args) in [
        (
            "books.file_document",
            json!({"category": "other", "file_name": "again.pdf", "mime_type": "application/pdf",
                   "content_base64": content, "sha256": sha}),
        ),
        (
            "books.attach_receipt",
            json!({"file_name": "again.pdf", "mime_type": "application/pdf",
                   "content_base64": content, "sha256": sha}),
        ),
    ] {
        let err = call(&c, tool, args).await.expect_err("marked");
        assert!(err.contains(TIN_FILE), "{tool}: {err}");
    }
}

/// Every way a file enters the books takes the text `name` holding
/// `text`, and the journal shows it.
async fn accepted_everywhere(c: &Client, name: &str, text: &str) {
    let (content, sha) = inline(text.as_bytes());
    for (tool, args) in [
        (
            "books.attach_receipt",
            json!({"file_name": name, "mime_type": "text/csv", "content_base64": content, "sha256": sha}),
        ),
        (
            "books.file_document",
            json!({"category": "import_source", "file_name": name, "mime_type": "text/csv",
                   "content_base64": content, "sha256": sha}),
        ),
    ] {
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
    let (server, url) = serve(&b);
    let c = connect(&url).await;
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
    accepted_everywhere(&c, "bank.csv", BANK_CSV).await;
    drop(c);
    drop(server);

    // The vault locked (its key gone): the printed shapes are still
    // refused, and the bank export still goes in; the vault's TIN written
    // bare is no longer recognized.
    std::fs::remove_file(b.dir.parent().unwrap().join("config/vault.key")).unwrap();
    let (_server, url) = serve(&b);
    let c = connect(&url).await;
    let path = b.out.join("payees-0.csv");
    refused_everywhere(&c, &b, &path, "payees-0.csv", TIN_PRINTED).await;
    let path = b.out.join("members-1.csv");
    refused_everywhere(&c, &b, &path, "members-1.csv", TIN_PRINTED).await;
    accepted_everywhere(
        &c,
        "bank-2.csv",
        &format!("{BANK_CSV}2026-10-06,x,1.00,1.00,1\n"),
    )
    .await;
    accepted_everywhere(&c, "members-0.csv", "member,tin\nKat,830835531\n").await;
}

/// INVARIANT 3b through the tools: the company's own EIN, as its
/// settings hold it, goes in printed 2-7 (formation papers, a business
/// plan) by every filing tool; another EIN, or the same nine digits
/// printed as an SSN, is still refused.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_company_s_own_ein_goes_in_and_no_other() {
    let b = books("own-ein");
    let toml = std::fs::read_to_string(&b.toml).unwrap();
    std::fs::write(
        &b.toml,
        toml.replacen(
            "name = \"Tin Co\"\n",
            "name = \"Tin Co\"\nein = \"12-3456789\"\n",
            1,
        ),
    )
    .unwrap();
    let (_server, url) = serve(&b);
    let c = connect(&url).await;
    accepted_everywhere(
        &c,
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
    let (_server, url) = serve(&b);
    let c = connect(&url).await;
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
    let (server, url) = serve(&b);
    let c = connect(&url).await;
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
    accepted_everywhere(&c, "one.csv", &std::fs::read_to_string(&one).unwrap()).await;
    accepted_everywhere(&c, "bank.csv", BANK_CSV).await;
    drop(c);
    drop(server);

    // The vault locked: the same refusals, word for word.
    std::fs::remove_file(b.dir.parent().unwrap().join("config/vault.key")).unwrap();
    let (_server, url) = serve(&b);
    let c = connect(&url).await;
    assert_eq!(
        probes(&c).await,
        open,
        "the refusal tells whether the vault is open"
    );
}
