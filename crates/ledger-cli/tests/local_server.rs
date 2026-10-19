//! `wanderbooks server`: every book in the data root on the root's local
//! sockets (ledger-mcp local.rs). Oracles: the INVARIANTS of local.rs and
//! admin.rs, and plan 3.9 (closing: "no new requests, the one in progress
//! finishes, every writer lock is released").
#![cfg(unix)]

use ledger_mcp::admin::{self, Request, Response};
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, ClientConfig};
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

type Client = RunningService<RoleClient, ClientConfig>;

/// A home with a data root made by `wanderbooks setup`.
struct Home {
    dir: tempfile::TempDir,
}

impl Home {
    fn new(tag: &str) -> Self {
        let dir = tempfile::Builder::new()
            .prefix(&format!("wb-srv-{tag}-"))
            .tempdir()
            .unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_wanderbooks"))
            .args(["setup", "--home"])
            .arg(dir.path())
            .env_remove("XDG_DATA_HOME")
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        Self { dir }
    }

    fn root(&self) -> PathBuf {
        self.dir.path().join(".local/share/wanderbooks")
    }

    fn run(&self) -> PathBuf {
        self.root().join("run")
    }

    fn admin(&self, request: Request) -> Response {
        admin::call(&self.run().join("admin.sock"), request).unwrap()
    }

    /// The text of every answer must not name the root (local.rs
    /// INVARIANT 7).
    fn assert_hidden(&self, text: &str) {
        let root = self.root().display().to_string();
        assert!(!text.contains(&root), "names the data root: {text}");
        assert!(
            !text.contains(&self.dir.path().display().to_string()),
            "names the home: {text}"
        );
    }

    fn create(&self, id: &str, currency: &str) {
        match self.admin(Request::Create {
            id: id.into(),
            currency: currency.into(),
        }) {
            Response::Created { id: made, problem } => {
                assert_eq!(made, id);
                assert_eq!(problem, None);
            }
            other => panic!("{other:?}"),
        }
    }
}

/// A running `wanderbooks server`, ended (kill) when dropped.
struct Server {
    child: Child,
}

impl Server {
    /// Start one and wait for its ready line.
    fn start(home: &Home) -> Self {
        // Its environment is the test home's alone: the default import
        // folder and vault key are there, never the real ones.
        let mut child = Command::new(env!("CARGO_BIN_EXE_wanderbooks"))
            .arg("server")
            .arg("--root")
            .arg(home.root())
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", home.dir.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        assert_eq!(line, "ready\n", "the server didn't say ready");
        Self { child }
    }

    /// Close it as the host does (stdin ends) and wait for it to exit.
    fn close(mut self) -> std::process::ExitStatus {
        drop(self.child.stdin.take());
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status;
            }
            assert!(Instant::now() < deadline, "the server didn't close");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// An HTTP client for the server's MCP socket.
fn mcp_http(home: &Home) -> reqwest::Client {
    // The short path is good in this process only, while it lives: kept.
    let short = ledger_store::root::SocketPath::new(&home.run(), "mcp.sock").unwrap();
    let path = short.path().to_path_buf();
    std::mem::forget(short);
    reqwest::Client::builder()
        .unix_socket(path)
        .build()
        .unwrap()
}

async fn connect(home: &Home, book: &str) -> Result<Client, String> {
    let http = mcp_http(home);
    let transport = StreamableHttpClientTransport::with_client(
        http,
        StreamableHttpClientTransportConfig::with_uri(format!("http://localhost/b/{book}/mcp")),
    );
    ClientConfig::default()
        .serve(transport)
        .await
        .map_err(|e| e.to_string())
}

/// Call a tool: Ok(structured result) or Err(the error text).
async fn call(c: &Client, tool: &str, args: Value) -> Result<Value, String> {
    let args = args.as_object().cloned().unwrap_or_default();
    // A client whose server went away may wait forever: a call that
    // takes this long counts as refused.
    let r = tokio::time::timeout(
        Duration::from_secs(20),
        c.call_tool(CallToolRequestParams::new(tool.to_string()).with_arguments(args)),
    )
    .await
    .map_err(|_| "no answer".to_string())?
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
            .unwrap_or_else(|| serde_json::from_str(&text).unwrap_or(Value::String(text))))
    }
}

async fn chart(c: &Client) {
    for (code, name, ty) in [
        ("1000", "Cash", "asset"),
        ("3000", "Capital", "equity"),
        ("6000", "Supplies", "expense"),
    ] {
        call(
            c,
            "books.add_account",
            json!({"code": code, "name": name, "account_type": ty}),
        )
        .await
        .unwrap();
    }
}

/// Draft a receipted $1.00 + n entry; post it. Ok(entry id) or Err(why).
async fn post_one(c: &Client, n: u64) -> Result<u64, String> {
    let e = call(
        c,
        "books.draft_entry",
        json!({"date": "2026-03-01", "description": format!("entry {n}")}),
    )
    .await?;
    let id = e["entry_id"].as_u64().unwrap();
    let amount = format!("{}.{:02}", 1 + n / 100, n % 100);
    for (account, side) in [("6000", "debit"), ("1000", "credit")] {
        call(
            c,
            "books.add_line",
            json!({"entry_id": id, "account": account, "side": side, "amount": amount}),
        )
        .await?;
    }
    call(
        c,
        "books.attach_receipt",
        json!({"entry_id": id, "file_name": format!("r{n}.txt"),
               "mime_type": "text/plain", "content_base64": "cmVjZWlwdA=="}),
    )
    .await?;
    call(
        c,
        "books.post_entry",
        json!({"entry_id": id, "confirmed": true, "no_party_reason": "test"}),
    )
    .await?;
    Ok(id)
}

fn mode(path: &Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_book_is_served_on_its_own_route() {
    let home = Home::new("routes");
    let server = Server::start(&home);
    for sock in ["mcp.sock", "admin.sock"] {
        assert_eq!(mode(&home.run().join(sock)), 0o600, "{sock}");
    }
    assert_eq!(home.admin(Request::List), Response::Books(vec![]));
    home.create("acme", "USD");
    home.create("beta", "EUR");
    match home.admin(Request::List) {
        Response::Books(books) => {
            let ids: Vec<_> = books.iter().map(|b| (b.id.as_str(), b.open)).collect();
            assert_eq!(ids, [("acme", true), ("beta", true)]);
        }
        other => panic!("{other:?}"),
    }
    // Each route reaches its own books.
    let acme = connect(&home, "acme").await.unwrap();
    let beta = connect(&home, "beta").await.unwrap();
    chart(&acme).await;
    let a = call(&acme, "books.chart", json!({})).await.unwrap();
    let b = call(&beta, "books.chart", json!({})).await.unwrap();
    assert_ne!(a, b, "both routes answered from the same books");
    assert!(a.to_string().contains("Supplies"), "{a}");
    assert!(!b.to_string().contains("Supplies"), "{b}");
    let id = post_one(&acme, 1).await.unwrap();
    let entry = call(&acme, "books.get_entry", json!({"entry_id": id}))
        .await
        .unwrap();
    home.assert_hidden(&entry.to_string());
    let start = call(&beta, "books.start", json!({})).await.unwrap();
    home.assert_hidden(&start.to_string());
    assert!(start.to_string().contains("EUR"), "{start}");
    // A route for no books says so, and names no folder.
    let err = connect(&home, "nobody").await.unwrap_err();
    home.assert_hidden(&err);
    drop((acme, beta));
    assert!(server.close().success());
}

/// local.rs INVARIANT 1: a second server waits for the first (an old
/// one still finishing after its host died), touching nothing, and takes
/// over when it is gone; told to close first, it just goes.
#[test]
fn a_second_server_waits_for_the_first() {
    let home = Home::new("second");
    let first = Server::start(&home);
    home.create("acme", "USD");
    let spawn = || {
        Command::new(env!("CARGO_BIN_EXE_wanderbooks"))
            .arg("server")
            .arg("--root")
            .arg(home.root())
            .env_clear()
            .env("HOME", home.dir.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap()
    };
    // Told to close while waiting: it goes, having done nothing.
    let mut quitter = spawn();
    std::thread::sleep(Duration::from_millis(300));
    assert!(quitter.try_wait().unwrap().is_none(), "it didn't wait");
    drop(quitter.stdin.take());
    assert!(wait_exit(&mut quitter, 10).is_some_and(|s| s.success()));
    // The first is untouched: its sockets still answer.
    assert!(matches!(home.admin(Request::List), Response::Books(b) if b.len() == 1));
    // Another waits, then takes over once the first has closed.
    let mut second = spawn();
    std::thread::sleep(Duration::from_millis(300));
    assert!(second.try_wait().unwrap().is_none());
    assert!(first.close().success());
    let mut line = String::new();
    BufReader::new(second.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    assert_eq!(line, "ready\n");
    assert!(matches!(home.admin(Request::List), Response::Books(b) if b.len() == 1 && b[0].open));
    drop(second.stdin.take());
    assert!(wait_exit(&mut second, 20).is_some_and(|s| s.success()));
}

#[test]
fn closing_lets_go_of_every_book_and_removes_the_sockets() {
    let home = Home::new("close");
    let server = Server::start(&home);
    home.create("acme", "USD");
    home.create("beta", "USD");
    assert!(server.close().success());
    for sock in ["mcp.sock", "admin.sock"] {
        assert!(!home.run().join(sock).exists(), "{sock} left behind");
    }
    // Every writer lock released: the books open, and verify.
    for id in ["acme", "beta"] {
        let mut store = ledger_store::Store::open(&home.root().join("books").join(id)).unwrap();
        store.verify_and_heal().unwrap();
    }
    // And a new server opens them all again.
    let again = Server::start(&home);
    assert!(
        matches!(home.admin(Request::List), Response::Books(b) if b.len() == 2 && b.iter().all(|b| b.open))
    );
    assert!(again.close().success());
}

#[test]
fn signals_close_the_server_cleanly() {
    for signal in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
        let home = Home::new("signal");
        let mut server = Server::start(&home);
        home.create("acme", "USD");
        // SAFETY: the pid is our own child's.
        assert_eq!(unsafe { libc::kill(server.child.id() as i32, signal) }, 0);
        let status = server.child.wait().unwrap();
        assert!(status.success(), "signal {signal}: {status:?}");
        assert!(ledger_store::Store::open(&home.root().join("books/acme")).is_ok());
    }
}

/// The server half of `books_close_cleanly` (plan P2 tests): closing
/// during a burst of posts. Every post either completed (and is in the
/// books) or was refused (and isn't); the books verify at the next open.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn closing_during_a_burst_of_posts_loses_nothing() {
    let home = Home::new("burst");
    let server = Server::start(&home);
    home.create("acme", "USD");
    let setup = connect(&home, "acme").await.unwrap();
    chart(&setup).await;
    drop(setup);
    let mut tasks = Vec::new();
    for n in 0..4u64 {
        let c = connect(&home, "acme").await.unwrap();
        tasks.push(tokio::spawn(async move {
            // Until the close turns it away: it can't finish first.
            let mut posted = Vec::new();
            for k in 0.. {
                match post_one(&c, n * 100_000 + k).await {
                    Ok(id) => posted.push(id),
                    Err(why) => return (posted, why),
                }
            }
            unreachable!()
        }));
    }
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let closed = tokio::task::spawn_blocking(move || server.close())
        .await
        .unwrap();
    assert!(closed.success());
    let mut posted = Vec::new();
    for t in tasks {
        let (theirs, why) = t.await.unwrap();
        // Each client was stopped by the close, mid-burst.
        assert!(
            !theirs.is_empty(),
            "a client posted nothing before the close"
        );
        assert!(
            why.contains("closing") || why.contains("no answer") || why.contains("protocol"),
            "stopped by something else: {why}"
        );
        posted.extend(theirs);
    }
    let mut store = ledger_store::Store::open(&home.root().join("books/acme")).unwrap();
    store.verify_and_heal().unwrap();
    let mut in_books: Vec<u64> = store.ledger().posted_entries().map(|e| e.id()).collect();
    in_books.sort_unstable();
    posted.sort_unstable();
    assert_eq!(
        in_books, posted,
        "a post was lost or made without its answer"
    );
}

#[test]
fn the_admin_socket_refuses_bad_requests_plainly() {
    let home = Home::new("admin");
    let server = Server::start(&home);
    home.create("acme", "USD");
    for (request, why) in [
        (
            Request::Create {
                id: "acme".into(),
                currency: "USD".into(),
            },
            "already books called",
        ),
        (
            Request::Create {
                id: "Bad Id".into(),
                currency: "USD".into(),
            },
            "can't name books",
        ),
        (
            Request::Create {
                id: "new".into(),
                currency: "dollars".into(),
            },
            "isn't a currency",
        ),
        (
            Adopt {
                confirmed: true,
                ..Adopt::new("old", &home.root().join("books/acme"))
            }
            .request(),
            "inside the books' private folder",
        ),
    ] {
        match home.admin(request) {
            Response::Refused(text) => {
                assert!(text.contains(why), "{text}");
                home.assert_hidden(&text);
            }
            other => panic!("{other:?}"),
        }
    }
    // A vault without its key, or a key without its vault.
    let old = tempfile::Builder::new()
        .prefix("wb-srv-badv-")
        .tempdir()
        .unwrap();
    let data = old_books(old.path());
    for vault in [
        (Some(old.path().join("vault")), None),
        (None, Some(old.path().join("vault.key"))),
    ] {
        let r = Request::Adopt {
            id: "old".into(),
            source: data.clone(),
            settings_toml: None,
            config_json: None,
            vault_dir: vault.0,
            vault_key: vault.1,
            confirmed: false,
            expect_head: None,
            skip_unknown: false,
        };
        match home.admin(r) {
            Response::Refused(why) => assert!(why.contains("comes with its key"), "{why}"),
            other => panic!("{other:?}"),
        }
    }
    // Raw lines: not JSON, an unknown operation, another version, none.
    let sock = home.run().join("admin.sock");
    for (line, why) in [
        ("hello\n", "isn't understood"),
        (
            "{\"proto\":1,\"op\":\"delete\",\"id\":\"acme\"}\n",
            "isn't understood",
        ),
        ("{\"proto\":9,\"op\":\"list\"}\n", "another version"),
        ("{\"proto\":0,\"op\":\"list\"}\n", "another version"),
        ("{\"proto\":1,\"op\":\"list\"}", "cut off"),
    ] {
        let answer = raw_admin(&sock, line);
        assert!(answer.contains(why), "{line:?}: {answer}");
    }
    assert!(server.close().success());
}

/// Send `line` to the admin socket as is; the answer's text.
fn raw_admin(sock: &Path, line: &str) -> String {
    use std::io::{Read, Write};
    let mut s = std::os::unix::net::UnixStream::connect(sock).unwrap();
    s.write_all(line.as_bytes()).unwrap();
    s.shutdown(std::net::Shutdown::Write).unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).unwrap();
    out
}

/// An adoption request, built up.
#[derive(Clone, Default)]
struct Adopt {
    id: String,
    source: PathBuf,
    settings_toml: Option<String>,
    vault: Option<(PathBuf, PathBuf)>,
    confirmed: bool,
    expect_head: Option<(u64, String)>,
    skip_unknown: bool,
}

impl Adopt {
    fn new(id: &str, source: &Path) -> Self {
        Self {
            id: id.into(),
            source: source.to_path_buf(),
            ..Self::default()
        }
    }

    /// Confirmed, at the head a preview read.
    fn at(mut self, head: &(u64, String)) -> Self {
        self.confirmed = true;
        self.expect_head = Some(head.clone());
        self
    }

    fn request(self) -> Request {
        let (vault_dir, vault_key) = match self.vault {
            Some((d, k)) => (Some(d), Some(k)),
            None => (None, None),
        };
        Request::Adopt {
            id: self.id,
            source: self.source,
            settings_toml: self.settings_toml,
            config_json: None,
            vault_dir,
            vault_key,
            confirmed: self.confirmed,
            expect_head: self.expect_head,
            skip_unknown: self.skip_unknown,
        }
    }
}

/// Old books in a project folder, as P1 kept them: a data dir with no
/// settings of its own, and its settings file.
fn old_books(dir: &Path) -> PathBuf {
    let data = dir.join("books-data");
    let store = ledger_store::Store::create(&data, ledger_core::Currency::USD).unwrap();
    drop(store);
    data
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn adoption_previews_then_adopts_and_serves() {
    let home = Home::new("adopt");
    let server = Server::start(&home);
    let old = tempfile::Builder::new()
        .prefix("wb-srv-old-")
        .tempdir()
        .unwrap();
    let data = old_books(old.path());
    let toml = "[company]\nname = \"Acme Widgets LLC\"\n".to_string();
    let plan = Adopt {
        settings_toml: Some(toml.clone()),
        ..Adopt::new("acme", &data)
    };
    let journal_before = std::fs::read(data.join("journal.jsonl")).unwrap();
    let preview = match home.admin(plan.clone().request()) {
        Response::Preview(p) => p,
        other => panic!("{other:?}"),
    };
    assert!(!preview.keeps_settings);
    assert!(
        preview
            .settings
            .iter()
            .any(|s| s.key == "company.name" && s.value == Some(json!("Acme Widgets LLC"))),
        "{preview:?}"
    );
    // A preview writes nothing: no books, the source as it was.
    assert_eq!(home.admin(Request::List), Response::Books(vec![]));
    assert_eq!(
        std::fs::read(data.join("journal.jsonl")).unwrap(),
        journal_before
    );
    let adopted = match home.admin(plan.at(&preview.source_head).request()) {
        Response::Adopted(a) => a,
        other => panic!("{other:?}"),
    };
    assert_eq!(adopted.id, "acme");
    assert_eq!(adopted.source_head, preview.source_head);
    assert_eq!(adopted.settings_imported, preview.settings.len());
    assert_eq!(
        std::fs::read(data.join("journal.jsonl")).unwrap(),
        journal_before
    );
    // Served at once, with its settings.
    let c = connect(&home, "acme").await.unwrap();
    let settings = call(&c, "books.settings", json!({})).await.unwrap();
    assert!(
        settings.to_string().contains("Acme Widgets LLC"),
        "{settings}"
    );
    home.assert_hidden(&settings.to_string());
    let verify = call(&c, "books.verify", json!({})).await.unwrap();
    home.assert_hidden(&verify.to_string());
    drop(c);
    assert!(server.close().success());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_book_in_the_root_keeps_its_vault_and_key_inside_its_own_folder() {
    let home = Home::new("vault");
    let server = Server::start(&home);
    home.create("acme", "USD");
    let c = connect(&home, "acme").await.unwrap();
    call(
        &c,
        "books.add_party",
        json!({"name": "Jane Contractor", "roles": []}),
    )
    .await
    .unwrap();
    call(
        &c,
        "books.set_tin",
        json!({"party": "Jane Contractor", "tin": "241-40-4915", "tin_type": "ssn"}),
    )
    .await
    .unwrap();
    let book = home.root().join("books/acme");
    assert_eq!(mode(&book.join("keys")), 0o700);
    assert_eq!(mode(&book.join("keys/legacy.key")), 0o600);
    assert_eq!(mode(&book.join("vault")), 0o700);
    assert!(
        std::fs::read_dir(book.join("vault")).unwrap().count() > 0,
        "the TIN wasn't sealed into the book's vault"
    );
    // A file with TINs is never written into the root.
    let err = call(
        &c,
        "books.vendor_1099",
        json!({"year": 2026, "include_tins": true,
               "save_csv_to": home.root().join("x.csv").display().to_string()}),
    )
    .await
    .unwrap_err();
    home.assert_hidden(&err);
    assert!(!home.root().join("x.csv").exists());
    drop(c);
    assert!(server.close().success());
}

/// An MCP session over raw HTTP: initialized, its session id.
async fn raw_session(http: &reqwest::Client, book: &str) -> String {
    let url = format!("http://localhost/b/{book}/mcp");
    let r = http
        .post(&url)
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .body(
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
                "protocolVersion": "2025-06-18", "capabilities": {},
                "clientInfo": {"name": "test", "version": "0"}}})
            .to_string(),
        )
        .send()
        .await
        .unwrap();
    let session = r.headers()["mcp-session-id"].to_str().unwrap().to_string();
    let _ = r.text().await;
    http.post(&url)
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("mcp-session-id", &session)
        .header("mcp-protocol-version", "2025-06-18")
        .body(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}).to_string())
        .send()
        .await
        .unwrap();
    session
}

/// Wait for a child to exit, at most `secs`.
fn wait_exit(child: &mut Child, secs: u64) -> Option<std::process::ExitStatus> {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait().unwrap() {
            return Some(status);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    None
}

/// Review 3a, finding 1: a tool whose client hung up still runs to its
/// end before the server lets go of the books (SharedStore INVARIANT 3).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_call_whose_client_hung_up_finishes_before_the_close() {
    let home = Home::new("hangup");
    let mut server = Server::start(&home);
    home.create("acme", "USD");
    let http = mcp_http(&home);
    let session = raw_session(&http, "acme").await;
    // A large document: its filing takes long enough to close mid-way.
    let mut bytes = b"%PDF-1.4\n".to_vec();
    bytes.extend((0..20_000_000u32).map(|i| (i % 251) as u8));
    use base64_engine::encode;
    let call = json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {
        "name": "books.file_document", "arguments": {
            "category": "formation", "file_name": "big.pdf",
            "mime_type": "application/pdf", "content_base64": encode(&bytes)}}});
    let response = http
        .post("http://localhost/b/acme/mcp")
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("mcp-session-id", &session)
        .header("mcp-protocol-version", "2025-06-18")
        .body(call.to_string())
        .send()
        .await
        .unwrap();
    // Hang up with the headers read (the call is running by then), then
    // close.
    drop(response);
    drop(http);
    tokio::time::sleep(Duration::from_millis(200)).await;
    drop(server.child.stdin.take());
    let status = tokio::task::spawn_blocking(move || {
        let s = wait_exit(&mut server.child, 120);
        (s, server)
    })
    .await
    .unwrap();
    assert!(status.0.is_some_and(|s| s.success()), "{:?}", status.0);
    // The document is in the books whole, or not at all; never half.
    let mut store = ledger_store::Store::open(&home.root().join("books/acme")).unwrap();
    store.verify_and_heal().unwrap();
    drop(store);
    let snap = ledger_store::Snapshot::load(&home.root().join("books/acme")).unwrap();
    // It ran to its end before the server let go: filed whole.
    let filed = snap.attachments(None).unwrap();
    assert_eq!(filed.len(), 1, "the call was cut off");
    assert_eq!(snap.attachment_bytes(filed[0].id).unwrap(), bytes);
}

/// Base64 for the tests (standard alphabet, padded).
mod base64_engine {
    pub fn encode(bytes: &[u8]) -> String {
        const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::with_capacity(bytes.len() * 4 / 3 + 4);
        for c in bytes.chunks(3) {
            let n = (u32::from(c[0]) << 16)
                | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
                | u32::from(*c.get(2).unwrap_or(&0));
            for (i, shift) in [18, 12, 6, 0].iter().enumerate() {
                out.push(if i <= c.len() {
                    T[((n >> shift) & 63) as usize] as char
                } else {
                    '='
                });
            }
        }
        out
    }
}

/// Send the start of a request that never finishes.
fn stall(home: &Home) -> std::os::unix::net::UnixStream {
    use std::io::Write;
    let short = ledger_store::root::SocketPath::new(&home.run(), "mcp.sock").unwrap();
    let mut s = std::os::unix::net::UnixStream::connect(short.path()).unwrap();
    s.write_all(
        b"POST /b/acme/mcp HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\n\
          Accept: application/json, text/event-stream\r\nContent-Length: 1000\r\n\r\n{\"jsonrpc\"",
    )
    .unwrap();
    s
}

/// Review 3a, finding 2: a request that never finishes arriving doesn't
/// hold the books open; a second signal stops the wait for it at once.
#[test]
fn a_stalled_request_doesnt_hold_the_books_open() {
    let home = Home::new("stall");
    let mut server = Server::start(&home);
    home.create("acme", "USD");
    let _held = stall(&home);
    std::thread::sleep(Duration::from_millis(200));
    let started = Instant::now();
    drop(server.child.stdin.take());
    let status = wait_exit(&mut server.child, 40).expect("the server never closed");
    assert!(status.success());
    assert!(
        started.elapsed() < Duration::from_secs(30),
        "{:?}",
        started.elapsed()
    );

    let mut server = Server::start(&home);
    let _held = stall(&home);
    std::thread::sleep(Duration::from_millis(200));
    let started = Instant::now();
    let pid = server.child.id() as i32;
    // SAFETY: our own child.
    unsafe { libc::kill(pid, libc::SIGTERM) };
    std::thread::sleep(Duration::from_millis(300));
    // SAFETY: our own child.
    unsafe { libc::kill(pid, libc::SIGTERM) };
    let status = wait_exit(&mut server.child, 20).expect("a second signal didn't hurry it");
    assert!(status.success());
    assert!(
        started.elapsed() < Duration::from_secs(9),
        "{:?}",
        started.elapsed()
    );
    // And the admin socket, idle, doesn't hold it either.
    let mut server = Server::start(&home);
    let short = ledger_store::root::SocketPath::new(&home.run(), "admin.sock").unwrap();
    let _idle = std::os::unix::net::UnixStream::connect(short.path()).unwrap();
    let started = Instant::now();
    drop(server.child.stdin.take());
    assert!(wait_exit(&mut server.child, 20).is_some_and(|s| s.success()));
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
}

/// Review 3a, finding 12: told to close while opening, it closes cleanly.
#[test]
fn a_signal_while_opening_still_closes_cleanly() {
    let home = Home::new("early");
    let server = Server::start(&home);
    home.create("acme", "USD");
    assert!(server.close().success());
    let mut child = Command::new(env!("CARGO_BIN_EXE_wanderbooks"))
        .args(["server", "--root"])
        .arg(home.root())
        .env_clear()
        .env("HOME", home.dir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    // SAFETY: our own child.
    unsafe { libc::kill(child.id() as i32, libc::SIGTERM) };
    let status = wait_exit(&mut child, 20).unwrap();
    // Either caught (exit 0) or, if it came before the handlers, the
    // default (killed by the signal) with nothing written half-way.
    assert!(status.success() || status.code().is_none(), "{status:?}");
    let mut store = ledger_store::Store::open(&home.root().join("books/acme")).unwrap();
    store.verify_and_heal().unwrap();
}

/// After kill -9 the next server replaces the stale sockets and serves.
#[test]
fn a_killed_server_is_replaced_cleanly() {
    let home = Home::new("kill9");
    let mut server = Server::start(&home);
    home.create("acme", "USD");
    server.child.kill().unwrap();
    server.child.wait().unwrap();
    assert!(home.run().join("mcp.sock").exists(), "stale socket left");
    let again = Server::start(&home);
    assert!(matches!(home.admin(Request::List), Response::Books(b) if b.len() == 1 && b[0].open));
    assert!(again.close().success());
}

/// Review 3a, findings 3 and 4: nothing written into the root, and a
/// vault folder made by the filing guard is private, so TINs can follow.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn nothing_is_written_in_the_root_and_the_vault_stays_private() {
    let home = Home::new("cpa");
    let server = Server::start(&home);
    home.create("acme", "USD");
    let c = connect(&home, "acme").await.unwrap();
    let book = home.root().join("books/acme");
    let err = call(
        &c,
        "books.export_cpa_package",
        json!({"as_of": "2026-10-18", "include_tins": false,
               "save_to": book.display().to_string()}),
    )
    .await
    .unwrap_err();
    home.assert_hidden(&err);
    let zips: Vec<_> = std::fs::read_dir(&book)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().ends_with(".zip"))
        .collect();
    assert!(zips.is_empty(), "a package was written into the root");
    // A TIN file written outside first: the vault folder it records into
    // is made private, and storing a TIN then works.
    let out = tempfile::Builder::new()
        .prefix("wb-srv-out-")
        .tempdir()
        .unwrap();
    let _ = call(
        &c,
        "books.vendor_1099",
        json!({"year": 2026, "include_tins": true,
               "save_csv_to": out.path().join("1099.csv").display().to_string()}),
    )
    .await;
    if book.join("vault").exists() {
        assert_eq!(mode(&book.join("vault")), 0o700);
    }
    call(
        &c,
        "books.add_party",
        json!({"name": "Jane Contractor", "roles": []}),
    )
    .await
    .unwrap();
    call(
        &c,
        "books.set_tin",
        json!({"party": "Jane Contractor", "tin": "241-40-4915", "tin_type": "ssn"}),
    )
    .await
    .unwrap();
    assert_eq!(mode(&book.join("vault")), 0o700);
    // books.export_settings works for books that read no file.
    call(&c, "books.export_settings", json!({})).await.unwrap();
    drop(c);
    assert!(server.close().success());
}

/// Old books with a TIN vault, as P1 kept them: the data dir, `vault/`
/// beside it holding one sealed TIN, and its key.
fn old_books_with_vault(dir: &Path) -> (PathBuf, PathBuf, PathBuf) {
    let data = old_books(dir);
    let vault = dir.join("vault");
    let key = dir.join("vault.key");
    let v = ledger_vault::Vault::open(&vault, &key, true).unwrap();
    v.put_tin(
        1,
        &ledger_vault::Tin::new("241-40-4915", ledger_vault::TinType::Ssn).unwrap(),
    )
    .unwrap();
    (data, vault, key)
}

#[test]
fn adoption_checks_the_vault_key_and_the_settings_file() {
    let home = Home::new("adoptv");
    let server = Server::start(&home);
    let old = tempfile::Builder::new()
        .prefix("wb-srv-oldv-")
        .tempdir()
        .unwrap();
    let (data, vault, key) = old_books_with_vault(old.path());
    // Another key doesn't open the records: refused, nothing made.
    let wrong = old.path().join("other.key");
    ledger_vault::Vault::open(&old.path().join("v2"), &wrong, true).unwrap();
    let toml = "[company]\nname = \"Acme\"\n".to_string();
    let with = |key: &Path| Adopt {
        settings_toml: Some(toml.clone()),
        vault: Some((vault.clone(), key.to_path_buf())),
        ..Adopt::new("acme", &data)
    };
    match home.admin(with(&wrong).request()) {
        Response::Refused(why) => assert!(why.contains("doesn't open"), "{why}"),
        other => panic!("{other:?}"),
    }
    // A settings file naming another key file: refused.
    let named = Adopt {
        settings_toml: Some(format!(
            "{toml}[vault]\nkey_file = \"{}\"\n",
            wrong.display()
        )),
        ..with(&key)
    };
    match home.admin(named.request()) {
        Response::Refused(why) => assert!(why.contains("key_file"), "{why}"),
        other => panic!("{other:?}"),
    }
    // Entries the books don't know: shown, then refused unless skipped.
    let unknown = Adopt {
        settings_toml: Some(format!("{toml}bogus_key = 1\n# kept elsewhere\n")),
        ..with(&key)
    };
    let preview = match home.admin(unknown.clone().request()) {
        Response::Preview(p) => p,
        other => panic!("{other:?}"),
    };
    assert_eq!(preview.vault_records, 1);
    assert!(
        preview.unknown.iter().any(|u| u.contains("bogus_key")),
        "{preview:?}"
    );
    assert!(
        preview
            .comments
            .iter()
            .any(|c| c.contains("kept elsewhere")),
        "{preview:?}"
    );
    match home.admin(unknown.clone().at(&preview.source_head).request()) {
        Response::Refused(why) => assert!(why.contains("--skip-unknown"), "{why}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(home.admin(Request::List), Response::Books(vec![]));
    // Changed since the preview: refused.
    {
        let mut s = ledger_store::Store::open(&data).unwrap();
        s.add_account("1000", "Bank", ledger_core::AccountType::Asset, None, false)
            .unwrap();
    }
    let skipping = Adopt {
        skip_unknown: true,
        ..unknown
    };
    // (A child another test is spawning can hold a copy of the old
    // books' lock for an instant after `s` let go: asked again then.)
    let answer = (0..50)
        .map(|_| home.admin(skipping.clone().at(&preview.source_head).request()))
        .find(|r| {
            let busy = matches!(r, Response::Refused(why) if why.contains("open for writing"));
            if busy {
                std::thread::sleep(Duration::from_millis(100));
            }
            !busy
        })
        .expect("the old books stayed locked");
    match answer {
        Response::Refused(why) => assert!(why.contains("changed since the preview"), "{why}"),
        other => panic!("{other:?}"),
    }
    // Previewed again and confirmed: adopted, the vault and its key in.
    let head = match home.admin(skipping.clone().request()) {
        Response::Preview(p) => p.source_head,
        other => panic!("{other:?}"),
    };
    match home.admin(skipping.at(&head).request()) {
        Response::Adopted(a) => {
            assert_eq!(a.vault_files, 1);
            assert!(a.key_copied);
            assert_eq!(a.problem, None);
        }
        other => panic!("{other:?}"),
    }
    let book = home.root().join("books/acme");
    assert_eq!(mode(&book.join("keys/legacy.key")), 0o600);
    assert!(book.join("vault/tin-1.bin").is_file());
    assert!(server.close().success());
}
