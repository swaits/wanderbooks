//! `wanderbooks server`: every book in the data root on the root's local
//! sockets (ledger-mcp local.rs). Oracles: the INVARIANTS of local.rs and
//! admin.rs, and plan 3.9 (closing: "no new requests, the one in progress
//! finishes, every writer lock is released").
#![cfg(unix)]

use common::{Client, Home, Server, call, connect, connect_with, mcp_http, mcp_http_for};
use ledger_mcp::admin::{self, Request, Response};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

mod common;

/// Name cash 1000 the books' cash account (adding it if the chart lacks
/// it): the 1099 worksheet counts payments out of the books' cash
/// accounts and is refused without them.
async fn name_cash(c: &Client) {
    let _ = call(
        c,
        "books.add_account",
        json!({"code": "1000", "name": "Cash", "account_type": "asset"}),
    )
    .await;
    call(
        c,
        "books.set_setting",
        json!({"changes": [{"key": "books.cash_accounts", "value": ["1000"]}], "decided_by": "Test Owner"}),
    )
    .await
    .unwrap();
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

/// Draft a receipted $1.00 + n entry on `c`, a session with acme's books
/// with `token` (of the server whose `run/` folder is `run`), its receipt
/// uploaded as an agent uploads one; post it. Ok(entry id) or Err(why).
async fn post_one(c: &Client, (run, token): (&Path, &str), n: u64) -> Result<u64, String> {
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
    // Bytes of each entry's own: the same bytes stored already would be
    // named for linking, not uploaded again.
    let bytes = format!("receipt {n}");
    let begun = call(
        c,
        "books.begin_upload",
        common::upload_args(
            bytes.as_bytes(),
            &format!("r{n}.txt"),
            "text/plain",
            "receipt",
        ),
    )
    .await?;
    let upload = begun["upload_id"].as_str().unwrap_or_default().to_string();
    let put = common::mcp_http_builder_at(run)
        .build()
        .unwrap()
        .put(format!("http://localhost/b/acme/u/{upload}"))
        .bearer_auth(token)
        .body(bytes.into_bytes())
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if put.status().as_u16() != 200 {
        return Err(put.text().await.unwrap_or_default());
    }
    call(
        c,
        "books.attach_receipt",
        json!({"entry_id": id, "upload_id": upload}),
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
    let id = post_one(&acme, (&home.run(), &home.token("acme")), 1)
        .await
        .unwrap();
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
    // How many each client has posted: the close comes once every one has
    // posted, not after a fixed time (under load a fixed 1.5 s once left
    // a client with nothing posted).
    let counts: std::sync::Arc<[std::sync::atomic::AtomicUsize; 4]> =
        std::sync::Arc::new(Default::default());
    let mut tasks = Vec::new();
    for n in 0..4u64 {
        let c = connect(&home, "acme").await.unwrap();
        let (run, token) = (home.run(), home.token("acme"));
        let counts = counts.clone();
        tasks.push(tokio::spawn(async move {
            // Until the close turns it away: it can't finish first.
            let mut posted = Vec::new();
            for k in 0.. {
                match post_one(&c, (&run, &token), n * 100_000 + k).await {
                    Ok(id) => {
                        posted.push(id);
                        counts[n as usize].fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    }
                    Err(why) => return (posted, why),
                }
            }
            unreachable!()
        }));
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    while counts
        .iter()
        .any(|c| c.load(std::sync::atomic::Ordering::SeqCst) == 0)
    {
        assert!(
            std::time::Instant::now() < deadline,
            "a client posted nothing within a minute"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    // Then a little more, so the close lands mid-burst.
    tokio::time::sleep(Duration::from_millis(300)).await;
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

/// Send `line` to the admin socket as is; the answer's text. Connects
/// through `SocketPath`, as the server binds: under a long TMPDIR the
/// socket's path is longer than a socket address holds (SUN_LEN).
fn raw_admin(sock: &Path, line: &str) -> String {
    use std::io::{Read, Write};
    let short = ledger_store::root::SocketPath::new(
        sock.parent().unwrap(),
        sock.file_name().unwrap().to_str().unwrap(),
    )
    .unwrap();
    let mut s = std::os::unix::net::UnixStream::connect(short.path()).unwrap();
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
    // A file with TINs is handed over, never written loose in the root.
    name_cash(&c).await;
    let w = call(
        &c,
        "books.vendor_1099",
        json!({"year": 2026, "include_tins": true, "csv": true}),
    )
    .await
    .unwrap();
    home.assert_hidden(&w.to_string());
    assert!(
        w["csv"]["download"].as_str().unwrap().contains(" ex_"),
        "{w}"
    );
    assert!(w["csv"].get("content_base64").is_none(), "{w}");
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

/// Begin an upload on `session` (books.begin_upload) and send `bytes` for
/// it over the upload route: its id.
async fn raw_upload(http: &reqwest::Client, session: &str, bytes: &[u8], name: &str) -> String {
    let begin = json!({"jsonrpc": "2.0", "id": 9, "method": "tools/call", "params": {
        "name": "books.begin_upload", "arguments": {
            "sha256": ledger_store::journal::sha256_hex(bytes), "size": bytes.len(),
            "mime_type": "application/pdf", "file_name": name, "purpose": "document"}}});
    let r = http
        .post("http://localhost/b/acme/mcp")
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("mcp-session-id", session)
        .header("mcp-protocol-version", "2025-06-18")
        .body(begin.to_string())
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    let id = r
        .split("\"upload_id\":\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .unwrap_or_else(|| panic!("no upload: {r}"))
        .to_string();
    let put = http
        .put(format!("http://localhost/b/acme/u/{id}"))
        .body(bytes.to_vec())
        .send()
        .await
        .unwrap();
    assert_eq!(put.status().as_u16(), 200, "{:?}", put.text().await);
    id
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
    // A large document, sent as an upload: its filing takes long enough to
    // close mid-way.
    let mut bytes = b"%PDF-1.4\n".to_vec();
    bytes.extend((0..20_000_000u32).map(|i| (i % 251) as u8));
    let upload = raw_upload(&http, &session, &bytes, "big.pdf").await;
    let call = json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {
        "name": "books.file_document", "arguments": {
            "category": "formation", "upload_id": upload}}});
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
    call(
        &c,
        "books.add_account",
        json!({"code": "1000", "name": "Cash", "account_type": "asset"}),
    )
    .await
    .unwrap();
    let got = call(
        &c,
        "books.export_cpa_package",
        json!({"as_of": "2026-10-18", "include_tins": false, "cash_accounts": ["1000"]}),
    )
    .await
    .unwrap();
    home.assert_hidden(&got.to_string());
    let zips: Vec<_> = std::fs::read_dir(&book)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().ends_with(".zip"))
        .collect();
    assert!(zips.is_empty(), "a package was written into the root");
    // A TIN file made first: the vault folder it records into is made
    // private, and storing a TIN then works.
    let _ = call(
        &c,
        "books.vendor_1099",
        json!({"year": 2026, "include_tins": true, "csv": true}),
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
    // draft, 2 lines, begin_upload, attach, post: ok or an error
    let id = post_one(&full, (&home.run(), &admin_text), 7).await;
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
    // posting (draft, two lines, its receipt's upload and filing, post),
    // the failed lookup, the refusal.
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
    // Each record carries its request's own id, as the client numbered
    // it: one connection's calls, oldest first, climb, none twice.
    let ids: Vec<u64> = tools
        .iter()
        .rev()
        .filter(|r| r["principal"]["label"] != "reader")
        .map(|r| {
            r["request_id"]
                .as_str()
                .and_then(|id| id.parse().ok())
                .unwrap_or_else(|| panic!("{r}"))
        })
        .collect();
    assert!(ids.len() >= 8, "{ids:?}");
    assert!(ids.windows(2).all(|w| w[0] < w[1]), "{ids:?}");
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

// P2 slice 5: files in and out (ledger-mcp tools/files.rs, local.rs's
// upload and download routes; ledger-store uploads.rs).

fn b64(bytes: &[u8]) -> String {
    base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes)
}

/// Begin an upload on `c` for `bytes`: its id.
async fn begin(c: &Client, bytes: &[u8], name: &str, purpose: &str) -> String {
    let r = call(
        c,
        "books.begin_upload",
        json!({"sha256": ledger_store::journal::sha256_hex(bytes), "size": bytes.len(),
               "mime_type": "application/pdf", "file_name": name, "purpose": purpose}),
    )
    .await
    .unwrap();
    r["upload_id"].as_str().unwrap().to_string()
}

/// PUT `bytes` for upload `id` with `token`: the status, the refusal
/// code and the body.
async fn put(home: &Home, token: &str, id: &str, bytes: &[u8]) -> (u16, Option<String>, String) {
    let r = mcp_http(home)
        .put(format!("http://localhost/b/acme/u/{id}"))
        .bearer_auth(token)
        .body(bytes.to_vec())
        .send()
        .await
        .unwrap();
    let status = r.status().as_u16();
    let code = r
        .headers()
        .get(ledger_mcp::local::REFUSED_HEADER)
        .map(|v| v.to_str().unwrap().to_string());
    (status, code, r.text().await.unwrap())
}

/// A receipt-like PDF of `size` bytes, different for each `tag`.
fn pdf(tag: &str, size: usize) -> Vec<u8> {
    let mut b = format!("%PDF-1.4 {tag} ").into_bytes();
    let mut n: u32 = 0;
    while b.len() < size {
        b.extend_from_slice(&n.to_le_bytes());
        n = n.wrapping_mul(1_103_515_245).wrapping_add(12_345);
    }
    b.truncate(size);
    b
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_upload_is_filed_once_by_its_own_token_exactly_as_declared() {
    let home = Home::new("upload");
    let server = Server::start(&home);
    home.create("acme", "USD");
    let c = connect(&home, "acme").await.unwrap();
    let token = home.token("acme");
    let bytes = pdf("one", 300 * 1024);
    // The command it gives runs with the agent's own path; it names no
    // path of the server's.
    let r = call(
        &c,
        "books.begin_upload",
        json!({"sha256": ledger_store::journal::sha256_hex(&bytes), "size": bytes.len(),
               "mime_type": "application/pdf", "file_name": "big receipt.pdf",
               "purpose": "document"}),
    )
    .await
    .unwrap();
    home.assert_hidden(&r.to_string());
    let id = r["upload_id"].as_str().unwrap().to_string();
    assert_eq!(
        r["command"],
        json!(format!(
            "wanderbooks upload --book acme --token test {id} -- <the file's path>"
        ))
    );
    assert_eq!(r["max_bytes"], json!(25 * 1024 * 1024));
    assert!(r["expires_at"].as_str().unwrap().ends_with('Z'));
    // Not sent yet: nothing to file.
    let e = call(
        &c,
        "books.file_document",
        json!({"category": "other", "upload_id": id}),
    )
    .await
    .unwrap_err();
    assert!(e.contains("haven't been sent"), "{e}");
    // Anything but the bytes declared is refused and kept nowhere.
    let mut short = bytes.clone();
    short.pop();
    let mut changed = bytes.clone();
    changed[100] ^= 1;
    for wrong in [&short, &changed] {
        let (status, code, _) = put(&home, &token, &id, wrong).await;
        assert_eq!((status, code.as_deref()), (400, Some("not_as_declared")));
    }
    // Another token's upload takes nothing from this one.
    let other = home.token_with("acme", "other", &["read", "write"]);
    let (status, code, _) = put(&home, &other, &id, &bytes).await;
    assert_eq!((status, code.as_deref()), (403, Some("not_yours")));
    let (status, code, _) = put(&home, &token, "up_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", &bytes).await;
    assert_eq!((status, code.as_deref()), (404, Some("no_such_upload")));
    let (status, _, body) = put(&home, &token, &id, &bytes).await;
    assert_eq!(status, 200, "{body}");
    let got: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(got["size"], json!(bytes.len()));
    let (status, code, _) = put(&home, &token, &id, &bytes).await;
    assert_eq!((status, code.as_deref()), (409, Some("already_sent")));
    // No token: refused at the door, and recorded as an upload too; what
    // was sent (25 MiB, more than a socket holds) is read, so the client
    // gets the answer.
    let r = mcp_http(&home)
        .put(format!("http://localhost/b/acme/u/{id}"))
        .body(vec![b'x'; 25 * 1024 * 1024])
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 401);
    // Every PUT is in the access log, refusals too, under its token.
    let log = call(
        &c,
        "books.access_log",
        json!({"action": "upload", "limit": 20}),
    )
    .await
    .unwrap();
    let seen: Vec<(String, String)> = log["records"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .map(|r| {
            (
                r["principal"]["label"].as_str().unwrap().to_string(),
                r["result"].to_string(),
            )
        })
        .collect();
    let outcome = |s: &(String, String)| s.1.clone();
    assert_eq!(seen.len(), 7, "{seen:?}");
    assert!(
        outcome(&seen[0]).contains("not_as_declared")
            && outcome(&seen[1]).contains("not_as_declared")
    );
    assert_eq!(seen[2].0, "other");
    assert!(outcome(&seen[2]).contains("not_yours"));
    assert!(outcome(&seen[3]).contains("no_such_upload"));
    assert_eq!(outcome(&seen[4]), "\"ok\"");
    assert!(outcome(&seen[5]).contains("already_sent"));
    let at_door = &log["records"][0];
    assert_eq!(at_door["subject"], json!(id), "{at_door}");
    assert!(
        at_door["result"].to_string().contains("refused"),
        "{at_door}"
    );
    // Only the token that began it files it.
    let c2 = connect_with(&home, "acme", Some(other.clone()))
        .await
        .unwrap();
    let e = call(
        &c2,
        "books.file_document",
        json!({"category": "other", "upload_id": id}),
    )
    .await
    .unwrap_err();
    assert!(e.contains("another token"), "{e}");
    // Filed once, under the name and type it was declared with.
    let filed = call(
        &c,
        "books.file_document",
        json!({"category": "other", "upload_id": id}),
    )
    .await
    .unwrap();
    let att = filed["attachment_id"].as_u64().unwrap();
    assert_eq!(
        filed["sha256"],
        json!(ledger_store::journal::sha256_hex(&bytes))
    );
    let e = call(
        &c,
        "books.file_document",
        json!({"category": "other", "upload_id": id}),
    )
    .await
    .unwrap_err();
    assert!(e.contains("no upload"), "{e}");
    let meta = call(&c, "books.get_receipt", json!({"attachment_id": att}))
        .await
        .unwrap();
    assert_eq!(meta["file_name"], json!("big receipt.pdf"));
    assert_eq!(meta["mime_type"], json!("application/pdf"));
    assert_eq!(meta["size_bytes"], json!(bytes.len()));
    assert!(meta.get("file").is_none_or(Value::is_null), "{meta}");
    // Its bytes: handed over (D-46), the command that saves a copy, and
    // the copy is the stored bytes.
    let big = call(
        &c,
        "books.get_receipt",
        json!({"attachment_id": att, "view": "bytes"}),
    )
    .await
    .unwrap();
    assert!(big.get("content_base64").is_none(), "{big}");
    let command = big["file"]["download"].as_str().unwrap();
    assert!(
        command.starts_with("wanderbooks download --book acme --token test ex_")
            && command.ends_with(" --to <a folder>"),
        "{command}"
    );
    assert_eq!(big["file"]["file_name"], json!("big receipt.pdf"));
    assert_eq!(
        common::fetch_export(&home, "acme", &home.token("acme"), &big["file"]).await,
        bytes
    );
    // The same bytes again: the attachment that holds them, no upload.
    let again = call(
        &c,
        "books.begin_upload",
        json!({"sha256": ledger_store::journal::sha256_hex(&bytes), "size": bytes.len(),
               "mime_type": "application/pdf", "file_name": "copy.pdf", "purpose": "receipt"}),
    )
    .await
    .unwrap();
    assert_eq!(again["existing_attachment_id"], json!(att));
    assert!(again.get("upload_id").is_none_or(Value::is_null));
    // Nothing is left behind in the uploads' folder.
    let left: Vec<_> = std::fs::read_dir(home.book_dir("acme").join("uploads"))
        .unwrap()
        .collect();
    assert!(left.is_empty(), "{left:?}");
    drop((c, c2));
    assert!(server.close().success());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_call_that_fails_leaves_its_upload_for_the_next() {
    let home = Home::new("upload-again");
    let server = Server::start(&home);
    home.create("acme", "USD");
    let c = connect(&home, "acme").await.unwrap();
    let token = home.token("acme");
    chart(&c).await;
    let csv = b"Date,Description,Amount\n2026-03-02,Coffee,-4.50\n".to_vec();
    let id = begin(&c, &csv, "bank.csv", "import").await;
    let (status, _, body) = put(&home, &token, &id, &csv).await;
    assert_eq!(status, 200, "{body}");
    // Refused after the file was taken (no such account): given back.
    let e = call(
        &c,
        "books.import_bank_csv",
        json!({"upload_id": id, "bank_account_code": "1999", "default_offset_code": "6000"}),
    )
    .await
    .unwrap_err();
    assert!(e.contains("unknown bank account"), "{e}");
    let done = call(
        &c,
        "books.import_bank_csv",
        json!({"upload_id": id, "bank_account_code": "1000", "default_offset_code": "6000"}),
    )
    .await
    .unwrap();
    assert_eq!(done["count"], json!(1));
    let e = call(
        &c,
        "books.import_bank_csv",
        json!({"upload_id": id, "bank_account_code": "1000", "default_offset_code": "6000"}),
    )
    .await
    .unwrap_err();
    assert!(e.contains("no upload"), "{e}");
    // A chart refused after it was taken is given back; filed, used up.
    let chart = b"code,name,type,parent_code\n7000,Rent,expense,\n".to_vec();
    let id = begin(&c, &chart, "chart.csv", "import").await;
    assert_eq!(put(&home, &token, &id, &chart).await.0, 200);
    let bad = b"code,name,type,parent_code\n7000,Rent,nonsense,\n".to_vec();
    let bad_id = begin(&c, &bad, "bad.csv", "import").await;
    assert_eq!(put(&home, &token, &bad_id, &bad).await.0, 200);
    let e = call(&c, "books.import_chart_csv", json!({"upload_id": bad_id}))
        .await
        .unwrap_err();
    assert!(e.contains("nonsense"), "{e}");
    let e = call(&c, "books.import_chart_csv", json!({"upload_id": bad_id}))
        .await
        .unwrap_err();
    assert!(e.contains("nonsense"), "given back: {e}");
    call(&c, "books.import_chart_csv", json!({"upload_id": id}))
        .await
        .unwrap();
    let e = call(&c, "books.import_chart_csv", json!({"upload_id": id}))
        .await
        .unwrap_err();
    assert!(e.contains("no upload"), "{e}");
    // A receipt attached is used up too.
    let receipt = pdf("receipt", 900);
    let id = begin(&c, &receipt, "r.pdf", "receipt").await;
    assert_eq!(put(&home, &token, &id, &receipt).await.0, 200);
    call(&c, "books.attach_receipt", json!({"upload_id": id}))
        .await
        .unwrap();
    let e = call(&c, "books.attach_receipt", json!({"upload_id": id}))
        .await
        .unwrap_err();
    assert!(e.contains("no upload"), "{e}");
    // Filed only by a tool of its purpose; the wrong tool leaves it.
    let doc = pdf("purpose", 500);
    let id = begin(&c, &doc, "doc.pdf", "import").await;
    let (status, _, body) = put(&home, &token, &id, &doc).await;
    assert_eq!(status, 200, "{body}");
    let e = call(
        &c,
        "books.file_document",
        json!({"category": "other", "upload_id": id}),
    )
    .await
    .unwrap_err();
    assert!(e.contains("begun for import"), "{e}");
    let e = call(&c, "books.import_chart_csv", json!({"upload_id": id}))
        .await
        .unwrap_err();
    assert!(e.contains("isn't UTF-8"), "{e}");
    // The import refused the bytes (not a chart) after taking them: still
    // there for the next call, which is refused for its purpose again.
    let e = call(&c, "books.attach_receipt", json!({"upload_id": id}))
        .await
        .unwrap_err();
    assert!(e.contains("begun for import"), "{e}");
    let e = call(
        &c,
        "books.begin_upload",
        json!({"sha256": ledger_store::journal::sha256_hex(&doc), "size": doc.len(),
               "mime_type": "application/pdf", "file_name": "x", "purpose": "anything"}),
    )
    .await
    .unwrap_err();
    assert!(e.contains("purpose is"), "{e}");
    drop(c);
    assert!(server.close().success());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stored_bytes_are_named_only_where_they_can_be_linked() {
    let home = Home::new("stored");
    let server = Server::start(&home);
    home.create("acme", "USD");
    let c = connect(&home, "acme").await.unwrap();
    let csv = b"code,name,type,parent_code\n1000,Cash,asset,\n".to_vec();
    let up = common::upload_on(
        &c,
        &home.run(),
        "acme",
        &home.token("acme"),
        &csv,
        ("chart.csv", "text/csv", "document"),
    )
    .await;
    let filed = call(
        &c,
        "books.file_document",
        json!({"category": "import_source", "upload_id": up}),
    )
    .await
    .unwrap();
    let att = filed["attachment_id"].as_u64().unwrap();
    let begin_for = |purpose: &'static str| {
        let c = &c;
        let csv = csv.clone();
        async move {
            call(
                c,
                "books.begin_upload",
                json!({"sha256": ledger_store::journal::sha256_hex(&csv), "size": csv.len(),
                       "mime_type": "text/csv", "file_name": "chart.csv", "purpose": purpose}),
            )
            .await
            .unwrap()
        }
    };
    // A document: the attachment that holds it. An import needs its bytes.
    assert_eq!(
        begin_for("document").await["existing_attachment_id"],
        json!(att)
    );
    let import = begin_for("import").await;
    assert!(import["upload_id"].as_str().unwrap().starts_with("up_"));
    assert!(
        import
            .get("existing_attachment_id")
            .is_none_or(Value::is_null)
    );
    // A withdrawn attachment can't be linked: never named.
    call(
        &c,
        "books.withdraw_attachment",
        json!({"attachment_id": att, "reason": "filed twice"}),
    )
    .await
    .unwrap();
    let again = begin_for("document").await;
    assert!(again["upload_id"].as_str().unwrap().starts_with("up_"));
    assert!(
        again
            .get("existing_attachment_id")
            .is_none_or(Value::is_null)
    );
    drop(c);
    assert!(server.close().success());
}

/// D-46: a file comes only by its upload. The inline arguments that once
/// carried one are refused as unknown, whatever else comes with them,
/// pointing to books.begin_upload; a filing tool given no upload is
/// refused; the name and type a call gives are checked as the upload's
/// are; and the file comes back only handed over, its download the
/// stored bytes.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_file_comes_only_by_its_upload_and_goes_back_by_its_download() {
    let home = Home::new("handles");
    let server = Server::start(&home);
    home.create("acme", "USD");
    let c = connect(&home, "acme").await.unwrap();
    let token = home.token("acme");
    let small = pdf("small", 1000);
    let sha = ledger_store::journal::sha256_hex(&small);
    let up = common::upload_on(
        &c,
        &home.run(),
        "acme",
        &token,
        &small,
        ("s.pdf", "application/pdf", "document"),
    )
    .await;
    let file = |extra: Value| {
        let mut args = json!({"category": "other"});
        args.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        args
    };
    for extra in [
        json!({"content_base64": "JVBERi0xLjQ="}),
        json!({"content_base64": "JVBERi0xLjQ=", "sha256": sha}),
        json!({"sha256": sha, "upload_id": up}),
        json!({"content_base64": "JVBERi0xLjQ=", "upload_id": up}),
    ] {
        let e = call(&c, "books.file_document", file(extra.clone()))
            .await
            .unwrap_err();
        assert!(
            e.contains("unknown argument") && e.contains("books.begin_upload"),
            "{extra}: {e}"
        );
    }
    let e = call(&c, "books.file_document", file(json!({})))
        .await
        .unwrap_err();
    assert!(e.contains("upload_id"), "{e}");
    // A plain name and a real type, given or the upload's.
    for (name, mime, why) in [
        ("../escape.pdf", "application/pdf", "no folder"),
        ("a\\b.pdf", "application/pdf", "no folder"),
        ("rcpt\u{202e}fdp.exe", "application/pdf", "invisible"),
        ("a\nb.pdf", "application/pdf", "control"),
        (" ", "application/pdf", "name is needed"),
        ("ok.pdf", "pdf", "MIME type"),
        ("ok.pdf", "text/plain\r\nX-Evil: 1", "MIME type"),
    ] {
        let e = call(
            &c,
            "books.file_document",
            json!({"category": "other", "file_name": name, "mime_type": mime, "upload_id": up}),
        )
        .await
        .unwrap_err();
        assert!(e.contains(why), "{name:?} {mime:?}: {e}");
    }
    let e = call(
        &c,
        "books.file_document",
        file(json!({"file_name": "x".repeat(256), "upload_id": up})),
    )
    .await
    .unwrap_err();
    assert!(e.contains("at most 255 bytes"), "{e}");
    // Every refusal left the upload: filed now, under its own name.
    let filed = call(&c, "books.file_document", file(json!({"upload_id": up})))
        .await
        .unwrap();
    let att = filed["attachment_id"].as_u64().unwrap();
    let back = call(
        &c,
        "books.get_receipt",
        json!({"attachment_id": att, "view": "bytes"}),
    )
    .await
    .unwrap();
    assert!(back.get("content_base64").is_none(), "{back}");
    assert_eq!(back["file"]["file_name"], json!("s.pdf"));
    assert_eq!(back["file"]["sha256"], json!(sha));
    assert_eq!(
        common::fetch_export(&home, "acme", &token, &back["file"]).await,
        small
    );
    let e = call(
        &c,
        "books.get_receipt",
        json!({"attachment_id": att, "view": "everything"}),
    )
    .await
    .unwrap_err();
    assert!(e.contains("meta") && e.contains("bytes"), "{e}");
    drop(c);
    assert!(server.close().success());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_download_is_the_stored_bytes_for_a_reader_and_is_recorded() {
    let home = Home::new("download");
    let server = Server::start(&home);
    home.create("acme", "USD");
    let c = connect(&home, "acme").await.unwrap();
    let token = home.token("acme");
    let bytes = pdf("down", 2000);
    let up = common::upload_on(
        &c,
        &home.run(),
        "acme",
        &token,
        &bytes,
        ("Résumé \"final\".pdf", "application/pdf", "document"),
    )
    .await;
    let filed = call(
        &c,
        "books.file_document",
        json!({"category": "other", "upload_id": up}),
    )
    .await
    .unwrap();
    let att = filed["attachment_id"].as_u64().unwrap();
    let get = |bearer: Option<String>, what: String| {
        let http = mcp_http(&home);
        async move {
            let mut r = http.get(format!("http://localhost/b/acme/a/{what}"));
            if let Some(b) = bearer {
                r = r.bearer_auth(b);
            }
            r.send().await.unwrap()
        }
    };
    let r = get(Some(token.clone()), att.to_string()).await;
    assert_eq!(r.status().as_u16(), 200);
    let h = r.headers().clone();
    assert_eq!(h["content-type"], "application/pdf");
    assert_eq!(h["x-content-type-options"], "nosniff");
    assert_eq!(
        h["content-disposition"],
        "attachment; filename=\"R_sum_ _final_.pdf\"; filename*=UTF-8''R%C3%A9sum%C3%A9%20%22final%22.pdf"
    );
    let digest = <sha2::Sha256 as sha2::Digest>::digest(&bytes);
    assert_eq!(
        h["repr-digest"],
        format!("sha-256=:{}:", b64(&digest)).as_str()
    );
    assert_eq!(r.bytes().await.unwrap().to_vec(), bytes);
    // A token without books:read gets nothing; nor does no token.
    let writer = home.token_with("acme", "writer", &["write"]);
    let r = get(Some(writer), att.to_string()).await;
    assert_eq!(r.status().as_u16(), 403);
    assert!(r.text().await.unwrap().contains("books:read"));
    assert_eq!(get(None, att.to_string()).await.status().as_u16(), 401);
    // GET only: a HEAD reads nothing and records nothing.
    let head = mcp_http(&home)
        .head(format!("http://localhost/b/acme/a/{att}"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(head.status().as_u16(), 405);
    assert_eq!(head.headers()["allow"], "GET");
    for missing in [
        "999".to_string(),
        "x".to_string(),
        "-1".to_string(),
        format!("+{att}"),
        format!("0{att}"),
    ] {
        let r = get(Some(token.clone()), missing).await;
        assert_eq!(r.status().as_u16(), 404);
        home.assert_hidden(&r.text().await.unwrap());
    }
    // Each is in the access log, refusals too.
    let log = call(
        &c,
        "books.access_log",
        json!({"action": "download", "limit": 10}),
    )
    .await
    .unwrap();
    let outcomes: Vec<String> = log["records"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["result"].to_string())
        .collect();
    assert_eq!(outcomes.len(), 8, "{outcomes:?}");
    // A TIN in the path is masked in the log, whole.
    let r = get(Some(token.clone()), "123-45-6789".to_string()).await;
    assert_eq!(r.status().as_u16(), 404);
    let log = call(
        &c,
        "books.access_log",
        json!({"action": "download", "limit": 1}),
    )
    .await
    .unwrap();
    let subject = log["records"][0]["subject"].as_str().unwrap().to_string();
    assert!(
        !subject.contains("6789") && !subject.contains("123-45"),
        "{subject}"
    );
    assert!(
        outcomes
            .iter()
            .filter(|o| o.contains("no_such_attachment"))
            .count()
            == 5
    );
    assert!(outcomes.iter().any(|o| o.contains("insufficient_scope")));
    drop(c);
    assert!(server.close().success());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_w9_and_a_file_holding_a_tin_go_by_upload_only_where_they_belong() {
    let home = Home::new("w9-upload");
    let server = Server::start(&home);
    home.create("acme", "USD");
    let c = connect(&home, "acme").await.unwrap();
    let token = home.token("acme");
    call(
        &c,
        "books.add_party",
        json!({"name": "Payee", "roles": ["contractor"]}),
    )
    .await
    .unwrap();
    call(
        &c,
        "books.set_tin",
        json!({"party": "Payee", "tin": "620-37-1111", "tin_type": "ssn"}),
    )
    .await
    .unwrap();
    // A file holding a TIN the vault keeps is refused, upload or not.
    let notes = b"Payee notes: SSN 620-37-1111".to_vec();
    let id = begin(&c, &notes, "notes.txt", "document").await;
    assert_eq!(put(&home, &token, &id, &notes).await.0, 200);
    let e = call(
        &c,
        "books.file_document",
        json!({"category": "other", "upload_id": id, "mime_type": "text/plain"}),
    )
    .await
    .unwrap_err();
    assert!(e.contains("TIN vault"), "{e}");
    // A W-9 goes only to the vault.
    let w9 = b"%PDF-1.4 Form W-9 Payee 620-37-1111".to_vec();
    let id = begin(&c, &w9, "payee-w9.pdf", "w9").await;
    assert_eq!(put(&home, &token, &id, &w9).await.0, 200);
    for (tool, args) in [
        ("books.attach_receipt", json!({"upload_id": id})),
        (
            "books.file_document",
            json!({"category": "tax", "upload_id": id}),
        ),
    ] {
        let e = call(&c, tool, args).await.unwrap_err();
        assert!(e.contains("begun for w9"), "{tool}: {e}");
    }
    let w9_args = |party: &str| {
        json!({"party": party, "received_on": "2026-01-05",
               "entity_type": "individual/sole proprietor", "tin_last4": "1111",
               "w9_upload_id": id})
    };
    // Refused after it was taken: given back.
    let e = call(&c, "books.set_w9", w9_args("Nobody"))
        .await
        .unwrap_err();
    assert!(!e.contains("upload"), "{e}");
    let set = call(&c, "books.set_w9", w9_args("Payee")).await.unwrap();
    assert_eq!(set["w9"]["in_vault"], json!(true), "{set}");
    let e = call(&c, "books.set_w9", w9_args("Payee"))
        .await
        .unwrap_err();
    assert!(e.contains("no upload"), "{e}");
    let got = call(&c, "books.get_w9", json!({"party": "Payee"}))
        .await
        .unwrap();
    assert_eq!(got["file"]["file_name"], json!("payee-w9.pdf"));
    assert_eq!(
        common::fetch_export(&home, "acme", &token, &got["file"]).await,
        w9
    );
    // The W-9's upload is gone; the refused file's waits to lapse.
    let left: Vec<String> = std::fs::read_dir(home.book_dir("acme").join("uploads"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(left.iter().all(|n| !n.starts_with(&id)), "{left:?}");
    drop(c);
    assert!(server.close().success());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn lapsed_uploads_are_swept_while_the_server_runs() {
    let home = Home::new("sweep");
    let server = Server::start(&home);
    home.create("acme", "USD");
    let c = connect(&home, "acme").await.unwrap();
    let token = home.token("acme");
    let bytes = pdf("lapsing", 2000);
    let id = begin(&c, &bytes, "w9.pdf", "w9").await;
    assert_eq!(put(&home, &token, &id, &bytes).await.0, 200);
    let folder = home.book_dir("acme").join("uploads");
    // Its time runs out (the grant aged on the disk), and nothing else
    // happens: the server sweeps it anyway.
    let json = folder.join(format!("{id}.json"));
    let mut grant: Value = serde_json::from_slice(&std::fs::read(&json).unwrap()).unwrap();
    grant["expires"] = json!(0);
    std::fs::write(&json, grant.to_string()).unwrap();
    // An export too (a file the books made), aged the same way.
    name_cash(&c).await;
    let made = call(&c, "books.vendor_1099", json!({"year": 2026, "csv": true}))
        .await
        .unwrap();
    assert!(made["csv"]["download"].as_str().unwrap().contains(" ex_"));
    let exports = home.book_dir("acme").join("exports");
    for e in std::fs::read_dir(&exports).unwrap() {
        let p = e.unwrap().path();
        if p.extension().is_some_and(|x| x == "json") {
            let mut x: Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
            x["expires"] = json!(0);
            std::fs::write(&p, x.to_string()).unwrap();
        }
    }
    let deadline = Instant::now() + Duration::from_secs(25);
    while std::fs::read_dir(&folder).unwrap().count() > 0
        || std::fs::read_dir(&exports).unwrap().count() > 0
    {
        assert!(
            Instant::now() < deadline,
            "a lapsed upload or export wasn't swept"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    drop(c);
    assert!(server.close().success());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_mcp_request_is_at_most_a_mebibyte() {
    let home = Home::new("body");
    let server = Server::start(&home);
    home.create("acme", "USD");
    let http = mcp_http_for(&home, "acme");
    let session = raw_session(&http, "acme").await;
    let post = |len: usize| {
        // A call whose arguments pad it to `len` bytes exactly.
        let head = r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"books.chart","arguments":{"pad":""#;
        let tail = r#""}}}"#;
        let body = format!("{head}{}{tail}", "a".repeat(len - head.len() - tail.len()));
        assert_eq!(body.len(), len);
        http.post("http://localhost/b/acme/mcp")
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .header("mcp-session-id", &session)
            .header("mcp-protocol-version", "2025-06-18")
            .body(body)
            .send()
    };
    let max = ledger_mcp::local::MAX_BODY;
    assert_eq!(post(max + 1).await.unwrap().status().as_u16(), 413);
    let at = post(max).await.unwrap();
    assert_ne!(at.status().as_u16(), 413, "{:?}", at.text().await);
    drop(http);
    assert!(server.close().success());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_file_the_books_make_is_fetched_by_its_own_token() {
    let home = Home::new("exports");
    let server = Server::start(&home);
    home.create("acme", "USD");
    let c = connect(&home, "acme").await.unwrap();
    let token = home.token("acme");
    call(
        &c,
        "books.add_party",
        json!({"name": "Member One", "roles": ["member"]}),
    )
    .await
    .unwrap();
    call(
        &c,
        "books.set_tin",
        json!({"party": "Member One", "tin": "620-37-1111", "tin_type": "ssn"}),
    )
    .await
    .unwrap();
    for (code, name, ty) in [("1000", "Cash", "asset"), ("3000", "Capital", "equity")] {
        call(
            &c,
            "books.add_account",
            json!({"code": code, "name": name, "account_type": ty}),
        )
        .await
        .unwrap();
    }
    name_cash(&c).await;
    // The CPA package, TINs and all: handed over, never inline, never a
    // path; the command runs as this token.
    let got = call(
        &c,
        "books.export_cpa_package",
        json!({"as_of": "2026-10-18", "cash_accounts": ["1000"]}),
    )
    .await
    .unwrap();
    home.assert_hidden(&got.to_string());
    assert_eq!(got["tins_included"], json!(true));
    let file = &got["file"];
    assert_eq!(file["file_name"], json!("cpa-package-2026-10-18.zip"));
    assert_eq!(file["mime_type"], json!("application/zip"));
    assert!(file.get("content_base64").is_none(), "{file}");
    let command = file["download"].as_str().unwrap();
    assert!(
        command.starts_with("wanderbooks download --book acme --token test ex_")
            && command.ends_with(" --to <a folder>"),
        "{command}"
    );
    assert!(file["expires_at"].as_str().unwrap().ends_with('Z'));
    // Fetched whole, as often as needed while it's kept.
    let zip = common::fetch_export(&home, "acme", &token, file).await;
    assert!(zip.starts_with(b"PK"), "a zip");
    assert_eq!(common::fetch_export(&home, "acme", &token, file).await, zip);
    let id = command.split(' ').find(|w| w.starts_with("ex_")).unwrap();
    let get = |bearer: Option<String>, what: String| {
        let http = mcp_http(&home);
        async move {
            let mut r = http.get(format!("http://localhost/b/acme/x/{what}"));
            if let Some(b) = bearer {
                r = r.bearer_auth(b);
            }
            r.send().await.unwrap()
        }
    };
    // Only its own token; a reader at that.
    let other = home.token_with("acme", "other", &["read"]);
    let r = get(Some(other), id.to_string()).await;
    assert_eq!(r.status().as_u16(), 403);
    assert_eq!(r.headers()[ledger_mcp::local::REFUSED_HEADER], "not_yours");
    let writer = home.token_with("acme", "writer", &["write"]);
    assert_eq!(
        get(Some(writer), id.to_string()).await.status().as_u16(),
        403
    );
    assert_eq!(get(None, id.to_string()).await.status().as_u16(), 401);
    let r = get(
        Some(token.clone()),
        "ex_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
    )
    .await;
    assert_eq!(r.status().as_u16(), 404);
    home.assert_hidden(&r.text().await.unwrap());
    // A token that may make an export but not read the books fetches its
    // own: the export is bound to it, whatever its scopes.
    let admin = home.token_with("acme", "cpa", &["admin"]);
    let ca = connect_with(&home, "acme", Some(admin.clone()))
        .await
        .unwrap();
    let w = call(&ca, "books.vendor_1099", json!({"year": 2026, "csv": true}))
        .await
        .unwrap();
    let csv = common::fetch_export(&home, "acme", &admin, &w["csv"]).await;
    assert!(csv.starts_with(b"vendor_id,name,"));
    drop(ca);
    // Each fetch is in the access log as an export, under its id.
    let log = call(
        &c,
        "books.access_log",
        json!({"action": "export", "limit": 20}),
    )
    .await
    .unwrap();
    let records = log["records"].as_array().unwrap();
    assert_eq!(records.len(), 7, "{log}");
    assert!(records.iter().filter(|r| r["subject"] == json!(id)).count() >= 5);
    // Given back to be filed, it is refused: the server made it with TINs.
    let up = common::upload_on(
        &c,
        &home.run(),
        "acme",
        &token,
        &zip,
        ("p.zip", "application/zip", "document"),
    )
    .await;
    let e = call(
        &c,
        "books.file_document",
        json!({"category": "tax", "upload_id": up}),
    )
    .await;
    let e = e.unwrap_err();
    assert!(e.contains("TIN"), "{e}");
    // Nothing loose in the book's folder: the export is in its own.
    let loose: Vec<String> = std::fs::read_dir(home.book_dir("acme"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".zip") || n.ends_with(".csv") || n.ends_with(".pdf"))
        .collect();
    assert!(loose.is_empty(), "{loose:?}");
    drop(c);
    assert!(server.close().success());
}

// Mutation survivors of local.rs (P2): the close, the waits, the admin
// line, adoption's settings and vault, and the peer check.

/// INVARIANT 6: once closing, every request is refused as closing, a
/// route for no books included (never a connection cut it couldn't tell
/// from a call that ran), and the door stays open refusing while they
/// keep coming (LINGER_MAX, 3 s, after the work is done); then the
/// sockets go. Anything but a books route is simply not found.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn while_closing_the_door_stays_open_refusing_as_requests_come() {
    let home = Home::new("linger");
    let mut server = Server::start(&home);
    home.create("acme", "USD");
    let http = mcp_http(&home);
    let r = http
        .get("http://localhost/b/acme/nothing")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 404);
    assert_eq!(r.text().await.unwrap(), "not found");
    // A books route arriving just before the close: what LINGER counts
    // from (a route for no books is not one, so without this the close
    // could find none in the last LINGER and rightly shut at once).
    let r = http
        .post("http://localhost/b/nobody/mcp")
        .header("content-type", "application/json")
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 404);
    drop(server.child.stdin.take());
    let dropped = Instant::now();
    let mut first = None;
    // The longest wait between two requests sent since the close began:
    // the door stays open only while they come within LINGER (500 ms), up
    // to LINGER_MAX (3 s). A client starved past LINGER on a loaded machine
    // may rightly see it shut sooner, so the 3 s holds only when none was.
    let (mut sent, mut widest) = (dropped, Duration::ZERO);
    let shut = loop {
        assert!(dropped.elapsed() < Duration::from_secs(60), "never shut");
        let now = Instant::now();
        widest = widest.max(now - sent);
        sent = now;
        let Ok(r) = http
            .post("http://localhost/b/nobody/mcp")
            .header("content-type", "application/json")
            .body("{}")
            .send()
            .await
        else {
            break Instant::now();
        };
        let status = r.status().as_u16();
        let text = r.text().await.unwrap_or_default();
        if text.contains("closing") {
            assert_eq!(status, 503, "{text}");
            first.get_or_insert_with(Instant::now);
        } else {
            // Not found, until closing; never again after.
            assert!(first.is_none(), "after closing: {status} {text}");
            assert_eq!(status, 404, "{text}");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    let open = shut - dropped;
    // Never before LINGER has passed with no request (the last came just
    // before the close began, above).
    assert!(open >= Duration::from_millis(500), "{open:?}");
    if widest < Duration::from_millis(400) {
        // Requests kept coming: refused as closing, and open to LINGER_MAX.
        assert!(first.is_some(), "never refused as closing ({widest:?})");
        assert!(open >= Duration::from_millis(2500), "{open:?} {widest:?}");
    }
    let status = tokio::task::spawn_blocking(move || wait_exit(&mut server.child, 30))
        .await
        .unwrap();
    assert!(status.is_some_and(|s| s.success()), "{status:?}");
}

/// INVARIANT 6: standard input that can't be read (an error, not a read
/// interrupted) counts as its end, the host gone: the server closes.
#[test]
fn a_server_whose_input_cant_be_read_closes() {
    let home = Home::new("badstdin");
    let server = Server::start(&home);
    home.create("acme", "USD");
    assert!(server.close().success());
    // A folder as its input: every read of it fails (EISDIR).
    let mut child = Command::new(env!("CARGO_BIN_EXE_wanderbooks"))
        .args(["server", "--root"])
        .arg(home.root())
        .env_clear()
        .env("HOME", home.dir.path())
        .stdin(std::fs::File::open(home.dir.path()).unwrap())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let status = wait_exit(&mut child, 30);
    if status.is_none() {
        let _ = child.kill();
        let _ = child.wait();
    }
    assert!(status.is_some_and(|s| s.success()), "{status:?}");
    let mut store = ledger_store::Store::open(&home.book_dir("acme")).unwrap();
    store.verify_and_heal().unwrap();
}

/// Lines a child writes on stderr, as they come.
fn stderr_lines(child: &mut Child) -> std::sync::mpsc::Receiver<String> {
    let (tx, rx) = std::sync::mpsc::channel();
    let err = child.stderr.take().unwrap();
    std::thread::spawn(move || {
        for line in BufReader::new(err).lines() {
            let Ok(line) = line else { break };
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    rx
}

/// INVARIANT 1: a server waiting for another to finish says so, once,
/// however long it waits.
#[test]
fn a_server_waiting_for_another_says_so_once() {
    let home = Home::new("saywait");
    let first = Server::start(&home);
    let mut second = Command::new(env!("CARGO_BIN_EXE_wanderbooks"))
        .args(["server", "--root"])
        .arg(home.root())
        .env_clear()
        .env("HOME", home.dir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let lines = stderr_lines(&mut second);
    let waiting = "another books server has these books open; waiting for it to finish";
    let said = lines.recv_timeout(Duration::from_secs(30));
    if !said.as_ref().is_ok_and(|l| l.contains(waiting)) {
        let _ = second.kill();
        let _ = second.wait();
        panic!("it never said it was waiting: {said:?}");
    }
    // Many more tries at the lock (one each 100 ms), said nothing more.
    std::thread::sleep(Duration::from_millis(1000));
    drop(second.stdin.take());
    assert!(wait_exit(&mut second, 20).is_some_and(|s| s.success()));
    let rest: Vec<String> = lines.iter().collect();
    assert!(!rest.iter().any(|l| l.contains(waiting)), "{rest:?}");
    assert!(first.close().success());
}

/// admin INVARIANT 1: a request is one line of at most MAX_LINE bytes
/// before its newline: one that long is answered, a byte more is refused
/// as too long.
#[test]
fn an_admin_request_may_be_as_long_as_a_line_may_be() {
    let home = Home::new("adminline");
    let server = Server::start(&home);
    let sock = home.run().join("admin.sock");
    let request = "{\"proto\":1,\"op\":\"list\"}";
    let padded = |len: usize| format!("{request}{}\n", " ".repeat(len - request.len()));
    let answer = raw_admin(&sock, &padded(admin::MAX_LINE));
    let answered: Response = serde_json::from_str(answer.trim_end())
        .unwrap_or_else(|e| panic!("{e}: {}", &answer[..answer.len().min(200)]));
    assert_eq!(answered, Response::Books(vec![]));
    // A byte more and no newline yet: all of it read, refused.
    let longer = padded(admin::MAX_LINE + 1);
    let answer = raw_admin(&sock, longer.trim_end_matches('\n'));
    assert!(answer.contains("too long"), "{answer}");
    assert!(server.close().success());
}

/// Old books that keep settings of their own (a `settings.set` in their
/// journal), in `dir`.
fn old_books_keeping_settings(dir: &Path) -> PathBuf {
    let data = old_books(dir);
    let mut store = ledger_store::Store::open(&data).unwrap();
    store
        .change_settings(
            &[ledger_core::settings::Change {
                key: "company.name".into(),
                value: Some(json!("Kept LLC")),
            }],
            &ledger_core::settings::Source {
                decided_by: "the owner".into(),
                decided_on: ledger_core::settings::today_in(None),
                citation: None,
                note: None,
            },
        )
        .unwrap();
    data
}

/// Adoption (admin INVARIANTS 4, 5): books that keep no settings need
/// their file, even for a preview; a file naming the very key given is
/// taken; every vault record, TINs and W-9s, is opened and counted; and a
/// file given for books that keep their own is reported unused, never
/// when none was given.
#[test]
fn adoption_takes_the_settings_and_the_vault_as_they_are() {
    let home = Home::new("adoptsv");
    let server = Server::start(&home);
    let old = tempfile::Builder::new()
        .prefix("wb-srv-oldsv-")
        .tempdir()
        .unwrap();
    let (data, vault, key) = old_books_with_vault(old.path());
    ledger_vault::Vault::open(&vault, &key, false)
        .unwrap()
        .put_w9(
            1,
            &ledger_vault::W9File {
                file_name: "w9.pdf".into(),
                mime_type: "application/pdf".into(),
                bytes: b"%PDF-1.4 w9".to_vec(),
            },
        )
        .unwrap();
    let vaulted = Adopt {
        vault: Some((vault.clone(), key.clone())),
        ..Adopt::new("acme", &data)
    };
    match home.admin(vaulted.clone().request()) {
        Response::Refused(why) => {
            assert!(why.contains("keep no settings of their own"), "{why}");
        }
        other => panic!("{other:?}"),
    }
    let toml = format!(
        "[company]\nname = \"Acme\"\n[vault]\nkey_file = \"{}\"\n",
        key.display()
    );
    let named = Adopt {
        settings_toml: Some(toml),
        ..vaulted
    };
    match home.admin(named.request()) {
        Response::Preview(p) => assert_eq!(p.vault_records, 2, "{p:?}"),
        other => panic!("{other:?}"),
    }
    let toml = "[company]\nname = \"Acme\"\n".to_string();
    for (id, given, ignored) in [("beta", Some(toml), true), ("gamma", None, false)] {
        let dir = tempfile::Builder::new()
            .prefix("wb-srv-kept-")
            .tempdir()
            .unwrap();
        let plan = Adopt {
            settings_toml: given,
            ..Adopt::new(id, &old_books_keeping_settings(dir.path()))
        };
        let head = match home.admin(plan.clone().request()) {
            Response::Preview(p) => {
                assert!(p.keeps_settings);
                p.source_head
            }
            other => panic!("{id}: {other:?}"),
        };
        match home.admin(plan.at(&head).request()) {
            Response::Adopted(a) => {
                assert_eq!(
                    (a.settings_ignored, a.settings_imported),
                    (ignored, 0),
                    "{id}"
                );
            }
            other => panic!("{id}: {other:?}"),
        }
    }
    assert!(server.close().success());
}

/// INVARIANT 2: a connection from another account is dropped before a
/// byte is read, even with the sockets' permissions loosened (as a slip
/// might). The other account is made in a user namespace, as a user ID
/// of this account's /etc/subuid range; where that can't be made (no
/// `unshare --map-auto` or python3 here), the test says so and skips.
#[test]
fn a_connection_from_another_account_is_dropped_unread() {
    let home = Home::new("otheruid");
    let server = Server::start(&home);
    std::fs::set_permissions(home.run(), std::fs::Permissions::from_mode(0o711)).unwrap();
    for sock in ["mcp.sock", "admin.sock"] {
        std::fs::set_permissions(
            home.run().join(sock),
            std::fs::Permissions::from_mode(0o666),
        )
        .unwrap();
    }
    // Connect (from the run folder), send, read to the end: what came.
    let script = "import socket, sys\n\
                  s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)\n\
                  s.connect(sys.argv[1])\n\
                  print('connected', flush=True)\n\
                  got = b''\n\
                  try:\n    \
                      s.sendall(sys.argv[2].encode())\n    \
                      while True:\n        \
                          b = s.recv(65536)\n        \
                          if not b:\n            \
                              break\n        \
                          got += b\n\
                  except OSError:\n    \
                      pass\n\
                  print('answer:' + got.decode(errors='replace'))\n";
    let ask = |other: bool, sock: &str, request: &str| -> Option<String> {
        let mut cmd = if other {
            let mut c = Command::new("unshare");
            c.args([
                "--user",
                "--map-auto",
                "--setuid",
                "0",
                "--setgid",
                "0",
                "--",
                "python3",
            ]);
            c
        } else {
            Command::new("python3")
        };
        let out = cmd
            .args(["-I", "-c", script, sock, request])
            .current_dir(home.run())
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .stderr(Stdio::null())
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        text.contains("connected\n").then_some(text)
    };
    let http = "GET /nothing HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";
    let list = "{\"proto\":1,\"op\":\"list\"}\n";
    // This account: answered, on both sockets.
    let Some(ours) = ask(false, "mcp.sock", http) else {
        eprintln!("SKIPPED a_connection_from_another_account_is_dropped_unread: no python3");
        return;
    };
    assert!(ours.contains("404"), "{ours}");
    assert!(ask(false, "admin.sock", list).is_some_and(|a| a.contains("books")));
    // Another account: connected, and dropped unanswered.
    for (sock, request) in [("mcp.sock", http), ("admin.sock", list)] {
        let Some(theirs) = ask(true, sock, request) else {
            eprintln!(
                "SKIPPED a_connection_from_another_account_is_dropped_unread: no other account can be made here (unshare --map-auto)"
            );
            return;
        };
        assert!(theirs.ends_with("answer:\n"), "{sock}: {theirs}");
    }
    // And this account is still served.
    assert!(ask(false, "admin.sock", list).is_some_and(|a| a.contains("books")));
    assert!(server.close().success());
}

/// Out of descriptors, the server says so and waits a moment before
/// accepting again (never spinning), and serves again once some are free.
#[test]
fn a_server_out_of_descriptors_waits_then_serves_again() {
    use std::os::unix::process::CommandExt;
    let home = Home::new("nofile");
    let mut server = Command::new(env!("CARGO_BIN_EXE_wanderbooks"));
    server
        .args(["server", "--root"])
        .arg(home.root())
        .env_clear()
        .env("HOME", home.dir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // SAFETY: setrlimit is async-signal-safe; nothing else runs here.
    unsafe {
        server.pre_exec(|| {
            let limit = libc::rlimit {
                rlim_cur: 40,
                rlim_max: 40,
            };
            if libc::setrlimit(libc::RLIMIT_NOFILE, &limit) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = server.spawn().unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    assert_eq!(line, "ready\n");
    let lines = stderr_lines(&mut child);
    let short = ledger_store::root::SocketPath::new(&home.run(), "mcp.sock").unwrap();
    let held: Vec<_> = (0..80)
        .map(|_| std::os::unix::net::UnixStream::connect(short.path()).unwrap())
        .collect();
    let said = std::iter::from_fn(|| lines.recv_timeout(Duration::from_secs(30)).ok())
        .find(|l| l.contains("accept failed"));
    if said.is_none() {
        let _ = child.kill();
        let _ = child.wait();
        panic!("out of descriptors, it never said so");
    }
    drop(held);
    // Served again once descriptors are free (the admin socket too).
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Ok(Response::Books(b)) = admin::call(&home.run().join("admin.sock"), Request::List) {
            assert!(b.is_empty());
            break;
        }
        assert!(Instant::now() < deadline, "never served again");
        std::thread::sleep(Duration::from_millis(100));
    }
    drop(child.stdin.take());
    let status = wait_exit(&mut child, 30);
    if status.is_none() {
        let _ = child.kill();
        let _ = child.wait();
    }
    assert!(status.is_some_and(|s| s.success()), "{status:?}");
}
