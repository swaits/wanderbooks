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
    /// A token for each book, made on first use (all scopes).
    tokens: std::sync::Mutex<std::collections::HashMap<String, String>>,
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
        Self {
            dir,
            tokens: Default::default(),
        }
    }

    /// The token tests use for `book`, made through the admin socket the
    /// first time.
    fn token(&self, book: &str) -> String {
        let mut tokens = self.tokens.lock().unwrap();
        if let Some(t) = tokens.get(book) {
            return t.clone();
        }
        let text = match self.admin(Request::TokenCreate {
            book: book.into(),
            name: "test".into(),
            scopes: ["read", "write", "post", "admin"]
                .map(String::from)
                .to_vec(),
            replace: true,
        }) {
            Response::TokenCreated { text, .. } => text.0,
            other => panic!("{other:?}"),
        };
        tokens.insert(book.to_string(), text.clone());
        text
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
    mcp_http_builder(home).build().unwrap()
}

fn mcp_http_builder(home: &Home) -> reqwest::ClientBuilder {
    // The short path is good in this process only, while it lives: kept.
    let short = ledger_store::root::SocketPath::new(&home.run(), "mcp.sock").unwrap();
    let path = short.path().to_path_buf();
    std::mem::forget(short);
    reqwest::Client::builder().unix_socket(path)
}

/// An HTTP client for the server's MCP socket that sends `book`'s token
/// on every request.
fn mcp_http_for(home: &Home, book: &str) -> reqwest::Client {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::AUTHORIZATION,
        format!("Bearer {}", home.token(book)).parse().unwrap(),
    );
    mcp_http_builder(home)
        .default_headers(headers)
        .build()
        .unwrap()
}

async fn connect(home: &Home, book: &str) -> Result<Client, String> {
    let token = home.token(book);
    connect_with(home, book, Some(token)).await
}

/// [`connect`], with this token (or none).
async fn connect_with(home: &Home, book: &str, token: Option<String>) -> Result<Client, String> {
    let http = mcp_http(home);
    let mut config =
        StreamableHttpClientTransportConfig::with_uri(format!("http://localhost/b/{book}/mcp"));
    if let Some(token) = token {
        config = config.auth_header(token);
    }
    let transport = StreamableHttpClientTransport::with_client(http, config);
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
    let err = connect_with(&home, "nobody", Some(home.token("acme")))
        .await
        .unwrap_err();
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
    let http = mcp_http_for(&home, "acme");
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

// P2 slice 4: tokens, scopes and the access log (local.rs INVARIANT 8,
// ledger-mcp access.rs, scopes.rs; ledger-store auth.rs, access.rs).

impl Home {
    /// A token for `book` named `name` with `scopes`, made (or remade).
    fn token_with(&self, book: &str, name: &str, scopes: &[&str]) -> String {
        match self.admin(Request::TokenCreate {
            book: book.into(),
            name: name.into(),
            scopes: scopes.iter().map(|s| s.to_string()).collect(),
            replace: true,
        }) {
            Response::TokenCreated { text, .. } => text.0,
            other => panic!("{other:?}"),
        }
    }

    fn book_dir(&self, book: &str) -> PathBuf {
        self.root().join("books").join(book)
    }
}

/// A raw initialize on `book`'s route with `bearer` (if any): its status,
/// its refusal header, its session id and its body.
async fn raw_initialize(
    home: &Home,
    book: &str,
    bearer: Option<&str>,
) -> (u16, Option<String>, Option<String>, String) {
    let mut r = mcp_http(home)
        .post(format!("http://localhost/b/{book}/mcp"))
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .body(
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
                "protocolVersion": "2025-06-18", "capabilities": {},
                "clientInfo": {"name": "raw", "version": "0"}}})
            .to_string(),
        );
    if let Some(b) = bearer {
        r = r.bearer_auth(b);
    }
    let r = r.send().await.unwrap();
    let header = |name: &str| {
        r.headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    };
    let refused = header("x-wanderbooks-refused");
    let session = header("mcp-session-id");
    let status = r.status().as_u16();
    if status == 401 {
        assert_eq!(
            header("www-authenticate").as_deref(),
            Some("Bearer realm=\"wanderbooks\"")
        );
    }
    (status, refused, session, r.text().await.unwrap())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_token_opens_only_its_own_books_until_revoked() {
    let home = Home::new("tokens");
    let server = Server::start(&home);
    home.create("acme", "USD");
    home.create("beta", "USD");
    // None, or one that isn't a token: 401, recorded, nothing named.
    for bearer in [None, Some("wb_nonsense"), Some("")] {
        let (status, refused, session, body) = raw_initialize(&home, "acme", bearer).await;
        assert_eq!(status, 401, "{bearer:?}: {body}");
        assert_eq!(refused.as_deref(), Some("unauthorized"));
        assert!(session.is_none());
        assert!(body.contains("token"), "{body}");
        home.assert_hidden(&body);
    }
    // Another book's token: 403.
    let beta = home.token("beta");
    let (status, refused, _, body) = raw_initialize(&home, "acme", Some(&beta)).await;
    assert_eq!(status, 403, "{body}");
    assert_eq!(refused.as_deref(), Some("wrong_books"));
    assert!(body.contains("beta"), "{body}");
    // Its own: in.
    let acme = connect(&home, "acme").await.unwrap();
    chart(&acme).await;
    // Revoked: the very next call is refused, on the same session.
    match home.admin(Request::TokenRevoke {
        book: "acme".into(),
        name: "test".into(),
        id: None,
    }) {
        Response::TokenRevoked(t) => assert_eq!(t.name, "test"),
        other => panic!("{other:?}"),
    }
    assert!(call(&acme, "books.chart", json!({})).await.is_err());
    let (status, ..) = raw_initialize(&home, "acme", Some(&home.token("acme"))).await;
    assert_eq!(status, 401, "the remembered token was revoked");
    // The tokens listed carry no text.
    match home.admin(Request::TokenList) {
        Response::Tokens(tokens) => {
            assert_eq!(tokens.len(), 1);
            assert_eq!(tokens[0].book, "beta");
            let text = serde_json::to_string(&tokens).unwrap();
            assert!(!text.contains(&beta[3..]));
        }
        other => panic!("{other:?}"),
    }
    // Every refusal at the door is in acme's access log.
    let (records, _) = ledger_store::access::read(&home.book_dir("acme")).unwrap();
    let refused: Vec<&str> = records
        .iter()
        .filter_map(|r| match &r.access.result {
            ledger_store::access::Outcome::Refused(code) => Some(code.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        refused,
        [
            "unauthorized",
            "unauthorized",
            "unauthorized",
            "wrong_books",
            "unauthorized",
            "unauthorized"
        ],
        "{records:?}"
    );
    drop(acme);
    assert!(server.close().success());
}

/// The tool names a client is offered.
async fn offered(c: &Client) -> Vec<String> {
    c.list_all_tools()
        .await
        .unwrap()
        .into_iter()
        .map(|t| t.name.to_string())
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_token_does_only_what_its_scopes_allow() {
    let home = Home::new("scopes");
    let server = Server::start(&home);
    home.create("acme", "USD");
    let full = connect(&home, "acme").await.unwrap();
    chart(&full).await;
    let entry = call(
        &full,
        "books.draft_entry",
        json!({"date": "2026-03-01", "description": "lunch"}),
    )
    .await
    .unwrap()["entry_id"]
        .as_u64()
        .unwrap();
    for (account, side) in [("6000", "debit"), ("1000", "credit")] {
        call(
            &full,
            "books.add_line",
            json!({"entry_id": entry, "account": account, "side": side, "amount": "5.00"}),
        )
        .await
        .unwrap();
    }
    let head = || ledger_store::Snapshot::load(&home.book_dir("acme")).unwrap();
    let before = head().head_hash().to_string();
    // Read only.
    let read = home.token_with("acme", "reader", &["read"]);
    let reader = connect_with(&home, "acme", Some(read)).await.unwrap();
    let tools = offered(&reader).await;
    assert!(tools.contains(&"books.chart".to_string()));
    for hidden in ["books.add_line", "books.post_entry", "books.access_log"] {
        assert!(!tools.contains(&hidden.to_string()), "{hidden} offered");
    }
    assert!(call(&reader, "books.chart", json!({})).await.is_ok());
    let err = call(
        &reader,
        "books.add_account",
        json!({"code": "1100", "name": "Bank", "account_type": "asset"}),
    )
    .await
    .unwrap_err();
    assert!(err.contains("needs books:write"), "{err}");
    assert!(err.contains("reader"), "{err}");
    // Write: a preview, never the post.
    let write = home.token_with("acme", "writer", &["read", "write"]);
    let writer = connect_with(&home, "acme", Some(write)).await.unwrap();
    let tools = offered(&writer).await;
    assert!(
        tools.contains(&"books.post_entry".to_string()),
        "it may preview"
    );
    assert!(!tools.contains(&"books.create_invoice".to_string()));
    let preview = call(&writer, "books.post_entry", json!({"entry_id": entry})).await;
    if let Err(e) = &preview {
        assert!(
            !e.contains("needs books"),
            "a preview is refused by scope: {e}"
        );
    }
    let err = call(
        &writer,
        "books.post_entry",
        json!({"entry_id": entry, "confirmed": true}),
    )
    .await
    .unwrap_err();
    assert!(err.contains("needs books:post"), "{err}");
    assert_eq!(head().head_hash(), before, "nothing posted");
    // No admin: no access log.
    let poster = home.token_with("acme", "poster", &["read", "write", "post"]);
    let poster = connect_with(&home, "acme", Some(poster)).await.unwrap();
    let err = call(&poster, "books.access_log", json!({}))
        .await
        .unwrap_err();
    assert!(err.contains("needs books:admin"), "{err}");
    // A token without books:read reads no resource.
    let blind = home.token_with("acme", "blind", &["post"]);
    let blind = connect_with(&home, "acme", Some(blind)).await.unwrap();
    assert!(
        blind
            .read_resource(rmcp::model::ReadResourceRequestParams::new(
                "wanderbooks://handbook"
            ))
            .await
            .is_err()
    );
    assert!(offered(&blind).await.iter().all(|t| t != "books.chart"));
    drop((full, reader, writer, poster, blind));
    assert!(server.close().success());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_call_is_recorded_once_refusals_included() {
    let home = Home::new("accesslog");
    let server = Server::start(&home);
    home.create("acme", "USD");
    let admin_text = home.token("acme");
    let full = connect(&home, "acme").await.unwrap();
    chart(&full).await; // 3 ok
    let id = post_one(&full, 7).await; // draft, 2 lines, post: ok or an error
    let _ = id;
    // A call that errs (bad arguments) and one refused by scope.
    assert!(
        call(&full, "books.get_entry", json!({"entry_id": 999_999}))
            .await
            .is_err()
    );
    let reader = home.token_with("acme", "reader", &["read"]);
    let reader = connect_with(&home, "acme", Some(reader)).await.unwrap();
    let marker = "a-distinctive-argument-1234";
    assert!(
        call(
            &reader,
            "books.add_account",
            json!({"code": "1100", "name": marker, "account_type": "asset"})
        )
        .await
        .is_err()
    );
    // A request with no token at all.
    assert_eq!(raw_initialize(&home, "acme", None).await.0, 401);
    // Read through the tool (admin): newest first, all of them.
    let log = call(&full, "books.access_log", json!({"limit": 500}))
        .await
        .unwrap();
    let records = log["records"].as_array().unwrap();
    assert_eq!(log["total"].as_u64().unwrap() as usize, records.len());
    assert_eq!(log["more"], false);
    assert_eq!(log["head"]["seq"].as_u64().unwrap() as usize, records.len());
    let seqs: Vec<u64> = records.iter().map(|r| r["seq"].as_u64().unwrap()).collect();
    let mut sorted = seqs.clone();
    sorted.sort_unstable_by(|a, b| b.cmp(a));
    assert_eq!(seqs, sorted, "newest first");
    let tools: Vec<&Value> = records.iter().filter(|r| r["action"] == "tool").collect();
    // Every tool call made so far, one record each: 3 accounts, the
    // posting (draft, two lines, post: up to four), the failed lookup,
    // the refusal.
    let by = |subject: &str| tools.iter().filter(|r| r["subject"] == subject).count();
    assert_eq!(by("books.add_account"), 4);
    assert_eq!(by("books.get_entry"), 1);
    assert_eq!(by("books.draft_entry"), 1);
    assert_eq!(by("books.add_line"), 2);
    assert_eq!(by("books.post_entry"), 1);
    assert_eq!(by("books.access_log"), 0, "recorded once answered");
    let refused: Vec<&&Value> = tools
        .iter()
        .filter(|r| r["result"]["refused"] == "insufficient_scope")
        .collect();
    assert_eq!(refused.len(), 1);
    assert_eq!(refused[0]["principal"]["label"], "reader");
    assert!(
        refused[0]["args_digest"]
            .as_str()
            .is_some_and(|d| d.len() == 64)
    );
    let errored = tools.iter().filter(|r| r["result"] == "error").count();
    assert!(errored >= 1);
    // Writes carry the head after them; reads don't.
    let added = tools
        .iter()
        .find(|r| r["subject"] == "books.add_account" && r["result"] == "ok")
        .unwrap();
    assert!(added["head_after"]["seq"].as_u64().unwrap() >= 1);
    assert!(refused[0].get("head_after").is_none());
    // Each write's head is its own: oldest first, the heads climb, and
    // the last is the journal's head now; a call that wrote nothing has
    // none (review of slice 4, round 2).
    let mut heads: Vec<(u64, String)> = Vec::new();
    for r in records.iter().rev() {
        let wrote = r.get("head_after").is_some();
        let reads = ["books.get_entry", "books.access_log", "books.chart"]
            .contains(&r["subject"].as_str().unwrap_or_default());
        if reads || r["action"] != "tool" || r["result"] != "ok" {
            assert!(!wrote, "{r}");
        }
        if wrote {
            heads.push((
                r["head_after"]["seq"].as_u64().unwrap(),
                r["head_after"]["hash"].as_str().unwrap().to_string(),
            ));
        }
    }
    assert!(heads.windows(2).all(|w| w[0].0 < w[1].0), "{heads:?}");
    let snap = ledger_store::Snapshot::load(&home.book_dir("acme")).unwrap();
    assert_eq!(heads.last().unwrap().1, snap.head_hash());
    assert_eq!(added["client"]["name"], "rmcp");
    let door: Vec<&Value> = records
        .iter()
        .filter(|r| r["action"] == "connect")
        .collect();
    assert_eq!(door.len(), 1);
    assert_eq!(door[0]["result"]["refused"], "unauthorized");
    assert_eq!(door[0]["principal"]["kind"], "unknown");
    // The access_log call itself is the next record.
    let next = call(&full, "books.access_log", json!({"limit": 1}))
        .await
        .unwrap();
    assert_eq!(next["records"][0]["subject"], "books.access_log");
    assert_eq!(next["more"], true);
    // Filters and paging.
    let mine = call(&full, "books.access_log", json!({"token": "reader"}))
        .await
        .unwrap();
    assert!(
        mine["records"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["principal"]["label"] == "reader")
    );
    let only_refused = call(&full, "books.access_log", json!({"outcome": "refused"}))
        .await
        .unwrap();
    assert_eq!(only_refused["records"].as_array().unwrap().len(), 2);
    let top = seqs[0];
    let older = call(
        &full,
        "books.access_log",
        json!({"before": top, "limit": 2}),
    )
    .await
    .unwrap();
    assert!(
        older["records"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["seq"].as_u64().unwrap() < top)
    );
    for bad in [
        json!({"limit": 0}),
        json!({"limit": 501}),
        json!({"outcome": "maybe"}),
    ] {
        assert!(call(&full, "books.access_log", bad).await.is_err());
    }
    // On disk: a chain that verifies, holding no token and no argument.
    let (on_disk, head) = ledger_store::access::read(&home.book_dir("acme")).unwrap();
    assert_eq!(head.seq as usize, on_disk.len());
    let text = std::fs::read_to_string(home.book_dir("acme").join("access.jsonl")).unwrap();
    assert!(
        !text.contains(&admin_text[3..]),
        "a token's text was logged"
    );
    assert!(!text.contains(marker), "an argument was logged");
    assert_eq!(mode(&home.book_dir("acme").join("access.jsonl")), 0o600);
    drop((full, reader));
    assert!(server.close().success());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_session_is_used_only_with_the_token_that_opened_it() {
    let home = Home::new("sessions");
    let server = Server::start(&home);
    home.create("acme", "USD");
    let first = home.token("acme");
    let second = home.token_with("acme", "other", &["read", "write", "post", "admin"]);
    let (status, _, session, _) = raw_initialize(&home, "acme", Some(&first)).await;
    assert_eq!(status, 200);
    let session = session.unwrap();
    let list = |bearer: String| {
        let session = session.clone();
        let http = mcp_http(&home);
        async move {
            http.post("http://localhost/b/acme/mcp")
                .header("content-type", "application/json")
                .header("accept", "application/json, text/event-stream")
                .header("mcp-session-id", session)
                .header("mcp-protocol-version", "2025-06-18")
                .bearer_auth(bearer)
                .body(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}).to_string())
                .send()
                .await
                .unwrap()
        }
    };
    let r = list(second).await;
    assert_eq!(r.status().as_u16(), 403);
    assert_eq!(r.headers()["x-wanderbooks-refused"], "session_token");
    let r = list(first).await;
    assert_eq!(r.status().as_u16(), 200, "{:?}", r.text().await);
    assert!(server.close().success());
}

#[test]
fn a_damaged_access_log_keeps_its_books_closed() {
    let home = Home::new("damagedlog");
    let server = Server::start(&home);
    home.create("acme", "USD");
    home.create("beta", "USD");
    assert!(server.close().success());
    std::fs::write(home.book_dir("acme").join("access.jsonl"), "not a record\n").unwrap();
    let server = Server::start(&home);
    match home.admin(Request::List) {
        Response::Books(books) => {
            let acme = books.iter().find(|b| b.id == "acme").unwrap();
            assert!(!acme.open);
            let why = acme.problem.as_deref().unwrap();
            assert!(why.contains("access log"), "{why}");
            home.assert_hidden(why);
            assert!(books.iter().find(|b| b.id == "beta").unwrap().open);
        }
        other => panic!("{other:?}"),
    }
    assert!(server.close().success());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn damaged_token_records_refuse_every_agent_but_not_the_list() {
    let home = Home::new("damagedauth");
    let server = Server::start(&home);
    home.create("acme", "USD");
    let token = home.token("acme");
    assert!(server.close().success());
    let path = home.root().join("auth/principals.jsonl");
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, text.replace("books:admin", "books:read")).unwrap();
    let server = Server::start(&home);
    assert!(matches!(home.admin(Request::List), Response::Books(_)));
    let (status, _, _, body) = raw_initialize(&home, "acme", Some(&token)).await;
    assert_eq!(status, 401);
    assert!(body.contains("record of agent tokens"), "{body}");
    home.assert_hidden(&body);
    match home.admin(Request::TokenList) {
        Response::Refused(why) => assert!(why.contains("record of agent tokens"), "{why}"),
        other => panic!("{other:?}"),
    }
    assert!(server.close().success());
}

/// Review of slice 4, finding 1: the access log gives no TIN away, not
/// in its text and not through its digest of the arguments (one with the
/// TIN masked, so no guess at it can be checked).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_access_log_gives_no_tin_away() {
    let home = Home::new("tincanary");
    let server = Server::start(&home);
    home.create("acme", "USD");
    let full = connect(&home, "acme").await.unwrap();
    call(
        &full,
        "books.add_party",
        json!({"name": "Pat Example", "roles": ["contractor"]}),
    )
    .await
    .unwrap();
    let tin = "123-45-6789";
    let args = json!({"party": "Pat Example", "tin": tin, "tin_type": "ssn"});
    // Sealed or not (the vault may refuse in a fresh root), the call is
    // recorded; and once more by a token that may not, refused.
    let _ = call(&full, "books.set_tin", args.clone()).await;
    let reader = home.token_with("acme", "reader", &["read"]);
    let reader = connect_with(&home, "acme", Some(reader)).await.unwrap();
    assert!(call(&reader, "books.set_tin", args.clone()).await.is_err());
    let text = std::fs::read_to_string(home.book_dir("acme").join("access.jsonl")).unwrap();
    assert!(!text.contains(tin) && !text.contains("123456789"), "{text}");
    let other = json!({"party": "Pat Example", "tin": "987-81-4321", "tin_type": "ssn"});
    let want = ledger_store::access::digest(other.as_object().map(|_| &other).unwrap());
    let (records, _) = ledger_store::access::read(&home.book_dir("acme")).unwrap();
    let set: Vec<_> = records
        .iter()
        .filter(|r| r.access.subject == "books.set_tin")
        .collect();
    assert_eq!(set.len(), 2, "{records:?}");
    for r in set {
        assert_eq!(
            r.access.args_digest.as_deref(),
            Some(want.as_str()),
            "{r:?}"
        );
    }
    drop((full, reader));
    assert!(server.close().success());
}

/// Review of slice 4, finding 6: a session id the server never opened is
/// bound to nothing (no flood of made-up ids binds or unbinds anything),
/// and an ended session is forgotten.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn only_sessions_the_server_opened_are_bound() {
    let home = Home::new("sessbind");
    let server = Server::start(&home);
    home.create("acme", "USD");
    let first = home.token("acme");
    let second = home.token_with("acme", "other", &["read", "write", "post", "admin"]);
    let (_, _, session, _) = raw_initialize(&home, "acme", Some(&first)).await;
    let session = session.unwrap();
    let request = |bearer: String, sid: String, method: reqwest::Method| {
        let http = mcp_http(&home);
        async move {
            http.request(method, "http://localhost/b/acme/mcp")
                .header("content-type", "application/json")
                .header("accept", "application/json, text/event-stream")
                .header("mcp-session-id", sid)
                .header("mcp-protocol-version", "2025-06-18")
                .bearer_auth(bearer)
                .body(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}).to_string())
                .send()
                .await
                .unwrap()
                .status()
                .as_u16()
        }
    };
    // Made-up ids: not this server's sessions, so not found, and bound to
    // nothing.
    for n in 0..50 {
        let made_up = format!("not-a-session-{n}");
        assert_eq!(
            request(second.clone(), made_up, reqwest::Method::POST).await,
            404
        );
    }
    // The real one is still bound to the token that opened it.
    assert_eq!(
        request(second.clone(), session.clone(), reqwest::Method::POST).await,
        403
    );
    assert_eq!(
        request(first.clone(), session.clone(), reqwest::Method::POST).await,
        200
    );
    // Ended: forgotten (the next use of its id is simply not found).
    let ended = request(first.clone(), session.clone(), reqwest::Method::DELETE).await;
    assert!((200..300).contains(&ended), "{ended}");
    assert_eq!(
        request(second.clone(), session, reqwest::Method::POST).await,
        404
    );
    // "bearer" in any case.
    let (status, ..) = raw_initialize(&home, "acme", None).await;
    assert_eq!(status, 401);
    let r = mcp_http(&home)
        .post("http://localhost/b/acme/mcp")
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("authorization", format!("bEaReR {first}"))
        .body(
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
                "protocolVersion": "2025-06-18", "capabilities": {},
                "clientInfo": {"name": "raw", "version": "0"}}})
            .to_string(),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 200);
    assert!(server.close().success());
}

/// Review of slice 4, findings 2 to 4: no token below books:admin gets a
/// full TIN out, and none below books:post changes the vault or the
/// approval controls.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_vault_and_the_approval_controls_need_their_scopes() {
    let home = Home::new("vaultscope");
    let server = Server::start(&home);
    home.create("acme", "USD");
    let writer = home.token_with("acme", "writer", &["read", "write", "post"]);
    let w = connect_with(&home, "acme", Some(writer)).await.unwrap();
    for (tool, args) in [
        (
            "books.vendor_1099",
            json!({"year": 2026, "include_tins": true}),
        ),
        ("books.export_cpa_package", json!({"as_of": "2026-12-31"})),
        ("books.get_w9", json!({"party": "x"})),
    ] {
        let err = call(&w, tool, args).await.unwrap_err();
        assert!(err.contains("needs books:admin"), "{tool}: {err}");
    }
    let plain = home.token_with("acme", "plain", &["read", "write"]);
    let p = connect_with(&home, "acme", Some(plain)).await.unwrap();
    for (tool, args) in [
        (
            "books.set_tin",
            json!({"party": "x", "tin": "123-45-6789", "tin_type": "ssn"}),
        ),
        (
            "books.retire_convention",
            json!({"id": "x", "decided_by": "Pat"}),
        ),
        (
            "books.approve_entry",
            json!({"entry_id": 1, "approver": "Pat"}),
        ),
        (
            "books.set_convention",
            json!({"id": "x", "kind": "approval_rule", "title": "x", "body_md": "x",
                   "decided_by": "Pat", "structured": {"approver": "Pat"}}),
        ),
    ] {
        let err = call(&p, tool, args).await.unwrap_err();
        assert!(err.contains("needs books:post"), "{tool}: {err}");
    }
    // Another kind of convention is a write.
    let note = call(
        &p,
        "books.set_convention",
        json!({"id": "a-note", "kind": "note", "title": "A note", "body_md": "Hello.",
               "decided_by": "Pat"}),
    )
    .await;
    assert!(note.is_ok(), "{note:?}");
    drop((w, p));
    assert!(server.close().success());
}
