//! The server's backups (plan 3.7, D-91): every book served from the data
//! root backs itself up to the folder its settings name, a while after it
//! is written, when it opens, before it closes, and when asked. Oracles:
//! the INVARIANTS of ledger-mcp backup.rs and ledger-store backup, and
//! plan 3.7's named tests.
#![cfg(unix)]

use ledger_mcp::admin::{self, Request, Response};
use ledger_store::backup::{Ctx, find, verify_found};
use ledger_store::root::{BookId, Root};
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, ClientConfig};
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

type Client = RunningService<RoleClient, ClientConfig>;

/// A home with a data root made by `wanderbooks setup`, and a place for
/// the backups beside it.
struct Home {
    dir: tempfile::TempDir,
}

impl Home {
    fn new(tag: &str) -> Self {
        let dir = tempfile::Builder::new()
            .prefix(&format!("wb-bk-{tag}-"))
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
        ledger_store::root::default_root(self.dir.path())
    }

    fn book_dir(&self, book: &str) -> PathBuf {
        Root::open(&self.root())
            .unwrap()
            .book_dir(&BookId::parse(book).unwrap())
    }

    fn admin(&self, request: Request) -> Response {
        admin::call(&self.root().join("run/admin.sock"), request).unwrap()
    }

    fn create(&self, id: &str) {
        match self.admin(Request::Create {
            id: id.into(),
            currency: "USD".into(),
        }) {
            Response::Created { problem: None, .. } => {}
            other => panic!("{other:?}"),
        }
    }

    fn token(&self, book: &str, scopes: &[&str]) -> String {
        match self.admin(Request::TokenCreate {
            book: book.into(),
            name: format!("test-{}", scopes.join("-")),
            scopes: scopes.iter().map(|s| s.to_string()).collect(),
            replace: true,
        }) {
            Response::TokenCreated { text, .. } => text.0,
            other => panic!("{other:?}"),
        }
    }

    /// A place outside the data root (a folder on "another disk").
    fn outside(&self, name: &str) -> PathBuf {
        let p = self.dir.path().join("elsewhere").join(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        p
    }
}

/// A running `wanderbooks server`, ended (kill) when dropped.
struct Server {
    child: Child,
}

impl Server {
    /// Start one, backing up after `quiet` of no writing (a drill first
    /// looked for after the server's own wait: a minute, past any test).
    fn start(home: &Home, quiet: Duration) -> Self {
        Self::start_with(home, quiet, None)
    }

    /// [`Self::start`], a drill first looked for `drill_first` after books
    /// are seen open.
    fn start_with(home: &Home, quiet: Duration, drill_first: Option<Duration>) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_wanderbooks"));
        command
            .arg("server")
            .arg("--root")
            .arg(home.root())
            .arg("--backup-quiet")
            .arg(quiet.as_millis().to_string());
        if let Some(d) = drill_first {
            command.arg("--drill-first").arg(d.as_millis().to_string());
        }
        let mut child = command
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
        let deadline = Instant::now() + Duration::from_secs(60);
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

/// A session with one book, and what it takes to send the book a file as
/// an agent does (D-46: a file comes only by upload).
struct Agent {
    client: Client,
    http: reqwest::Client,
    book: String,
    token: String,
}

impl std::ops::Deref for Agent {
    type Target = Client;
    fn deref(&self) -> &Client {
        &self.client
    }
}

impl Agent {
    /// Upload `bytes` as `wanderbooks upload` does: books.begin_upload,
    /// then the bytes PUT with this session's token. The upload_id.
    async fn upload(&self, bytes: &[u8], file_name: &str, mime_type: &str) -> String {
        let begun = call(
            self,
            "books.begin_upload",
            json!({"sha256": ledger_store::journal::sha256_hex(bytes), "size": bytes.len(),
                   "mime_type": mime_type, "file_name": file_name, "purpose": "receipt"}),
        )
        .await
        .unwrap();
        let id = begun["upload_id"].as_str().unwrap().to_string();
        let r = self
            .http
            .put(format!("http://localhost/b/{}/u/{id}", self.book))
            .bearer_auth(&self.token)
            .body(bytes.to_vec())
            .send()
            .await
            .unwrap();
        assert_eq!(r.status().as_u16(), 200);
        id
    }
}

async fn connect(home: &Home, book: &str, token: String) -> Agent {
    // The short path is good in this process only, while it lives: kept.
    let short = ledger_store::root::SocketPath::new(&home.root().join("run"), "mcp.sock").unwrap();
    let path = short.path().to_path_buf();
    std::mem::forget(short);
    let http = reqwest::Client::builder()
        .unix_socket(path)
        .build()
        .unwrap();
    let config =
        StreamableHttpClientTransportConfig::with_uri(format!("http://localhost/b/{book}/mcp"))
            .auth_header(token.clone());
    let client = ClientConfig::default()
        .serve(StreamableHttpClientTransport::with_client(
            http.clone(),
            config,
        ))
        .await
        .unwrap();
    Agent {
        client,
        http,
        book: book.to_string(),
        token,
    }
}

/// Call a tool: Ok(structured result) or Err(the error text).
/// Call a tool the way an agent confirms (plan P4, D-50): a call with
/// `confirmed: true` and no `preview` is made first without `confirmed`,
/// then with the `preview` it returned. A refused preview is the answer.
async fn call(c: &Client, tool: &str, mut args: Value) -> Result<Value, String> {
    if args.get("confirmed") == Some(&json!(true)) && args.get("preview").is_none() {
        let mut first = args.clone();
        first["confirmed"] = json!(false);
        let seen = call_once(c, tool, first).await?;
        if let Some(digest) = seen.get("preview").and_then(Value::as_str) {
            args["preview"] = json!(digest);
        }
    }
    call_once(c, tool, args).await
}

async fn call_once(c: &Client, tool: &str, args: Value) -> Result<Value, String> {
    let mut args = args.as_object().cloned().unwrap_or_default();
    // The key every write call carries (D-88).
    static NEXT_KEY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    args.entry("request_key").or_insert_with(|| {
        let n = NEXT_KEY.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        json!(format!("backups-key-{}-{n}", std::process::id()))
    });
    let r = tokio::time::timeout(
        Duration::from_secs(60),
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

async fn head(c: &Client) -> u64 {
    call(c, "books.start", json!({})).await.unwrap()["head"]["seq"]
        .as_u64()
        .unwrap()
}

/// Name the backup folder in the book's settings.
async fn set_folder(c: &Client, folder: &Path) {
    call(
        c,
        "books.set_setting",
        json!({"changes": [{"key": "backup.folder", "value": folder.display().to_string()}],
               "decided_by": "Test Owner", "note": "Backups."}),
    )
    .await
    .unwrap();
}

/// Draft, receipt and post one $n entry.
async fn post_one(c: &Agent, n: u64) {
    for (code, name, ty) in [("1000", "Cash", "asset"), ("6000", "Supplies", "expense")] {
        let _ = call(
            c,
            "books.add_account",
            json!({"code": code, "name": name, "account_type": ty}),
        )
        .await;
    }
    let e = call(
        c,
        "books.draft_entry",
        json!({"date": "2026-03-01", "description": format!("entry {n}")}),
    )
    .await
    .unwrap();
    let id = e["entry_id"].as_u64().unwrap();
    for (account, side) in [("6000", "debit"), ("1000", "credit")] {
        call(
            c,
            "books.add_line",
            json!({"entry_id": id, "account": account, "side": side, "amount": format!("{n}.00")}),
        )
        .await
        .unwrap();
    }
    // Bytes of each entry's own: the same bytes stored already would be
    // named for linking, not uploaded again.
    let upload = c
        .upload(
            format!("receipt {n}").as_bytes(),
            &format!("r{n}.txt"),
            "text/plain",
        )
        .await;
    call(
        c,
        "books.attach_receipt",
        json!({"entry_id": id, "upload_id": upload}),
    )
    .await
    .unwrap();
    call(
        c,
        "books.post_entry",
        json!({"entry_id": id, "confirmed": true, "no_party_reason": "test"}),
    )
    .await
    .unwrap();
}

/// Long enough for the server's backup loop (which looks every 2 s) to
/// have seen books just made, before they name any target: what it does
/// on first seeing them (a backup, a drill) is then done.
async fn settle() {
    tokio::time::sleep(Duration::from_millis(4500)).await;
}

/// Wait until every target holds the books' head (or `within` passes).
async fn backed_up(c: &Client, within: Duration) -> Value {
    let deadline = Instant::now() + within;
    loop {
        let s = call(c, "books.backup", json!({})).await.unwrap();
        let head = s["head_seq"].as_u64().unwrap();
        let targets = s["targets"].as_array().unwrap();
        if !targets.is_empty()
            && targets
                .iter()
                .all(|t| t["through_seq"].as_u64() == Some(head))
        {
            return s;
        }
        assert!(Instant::now() < deadline, "not backed up in time: {s}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Prove the backup at `source` (a destination folder, a set's folder, or a
/// file) whole; its head.
fn proven(home: &Home, source: &Path, tag: &str) -> (u64, String) {
    let scratch = home.outside(&format!("scratch-{tag}"));
    let found = find(source, None, &Ctx::new()).unwrap_or_else(|e| panic!("{tag}: {e}"));
    verify_found(&found, &scratch, false, &Ctx::new())
        .unwrap_or_else(|e| panic!("{tag}: {e}"))
        .head
}

fn live_head(home: &Home, book: &str) -> (u64, String) {
    let s = ledger_store::Snapshot::load(&home.book_dir(book)).unwrap();
    (s.journal_seq(), s.head_hash().to_string())
}

const ALL: [&str; 4] = ["read", "write", "post", "admin"];

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_book_backs_itself_up_to_its_folder_soon_after_a_write() {
    let home = Home::new("folder");
    let server = Server::start(&home, Duration::from_millis(300));
    home.create("acme");
    let c = connect(&home, "acme", home.token("acme", &ALL)).await;
    // None set up: said, and nothing is stale.
    let s = call(&c, "books.backup", json!({})).await.unwrap();
    assert_eq!(s["targets"], json!([]));
    assert!(s["note"].as_str().unwrap().contains("backup.folder"), "{s}");
    let steps = |v: Value| v["next_steps"].to_string();
    let start = steps(call(&c, "books.start", json!({})).await.unwrap());
    assert!(start.contains("No backups are set up"), "{start}");
    let folder = home.outside("disk");
    std::fs::create_dir_all(&folder).unwrap();
    set_folder(&c, &folder).await;
    post_one(&c, 7).await;
    let s = backed_up(&c, Duration::from_secs(30)).await;
    assert_eq!(s["note"], Value::Null);
    let targets = s["targets"].as_array().unwrap();
    assert_eq!(targets.len(), 1, "{s}");
    assert_eq!(targets[0]["stale"], false, "{s}");
    assert_eq!(targets[0]["sealed"], false, "{s}");
    assert_eq!(targets[0]["last_error"], Value::Null, "{s}");
    assert!(targets[0]["last_ok_at"].is_string(), "{s}");
    let start = steps(call(&c, "books.start", json!({})).await.unwrap());
    assert!(!start.contains("books.backup"), "{start}");
    // The books' head is what was written, and the folder holds them whole
    // at it: a folder for the books, plain files, nothing about git.
    let live = live_head(&home, "acme");
    assert_eq!(s["head_seq"].as_u64(), Some(live.0));
    assert_eq!(proven(&home, &folder, "folder"), live);
    assert!(folder.join("acme").join("READ-ME.txt").is_file());
    // Its journal is the live journal's beginning (a drill's check).
    let scratch = home.outside("scratch-prefix");
    let found = find(&folder, None, &Ctx::new()).unwrap();
    let v = verify_found(&found, &scratch, false, &Ctx::new()).unwrap();
    assert!(
        std::fs::read(home.book_dir("acme").join(ledger_store::JOURNAL_FILE))
            .unwrap()
            .starts_with(&std::fs::read(v.tree.join(ledger_store::JOURNAL_FILE)).unwrap())
    );
    // Nothing an agent is shown names the data root, or git.
    assert!(
        !s.to_string().contains(&home.root().display().to_string()),
        "{s}"
    );
    assert!(!s.to_string().to_lowercase().contains("git"), "{s}");
    drop(c);
    assert!(server.close().success());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn closing_backs_the_books_up_and_opening_catches_up() {
    let home = Home::new("close");
    let folder = home.outside("disk");
    std::fs::create_dir_all(&folder).unwrap();
    // Too long a quiet to back up while running: only the close does.
    let server = Server::start(&home, Duration::from_secs(3600));
    home.create("acme");
    settle().await;
    let c = connect(&home, "acme", home.token("acme", &ALL)).await;
    set_folder(&c, &folder).await;
    post_one(&c, 3).await;
    drop(c);
    assert!(server.close().success());
    assert_eq!(proven(&home, &folder, "closed"), live_head(&home, "acme"));
    // A folder named while the server was down, then made, is caught up
    // when the books open.
    let late = home.outside("late");
    let server = Server::start(&home, Duration::from_secs(3600));
    let c = connect(&home, "acme", home.token("acme", &ALL)).await;
    set_folder(&c, &late).await;
    // The folder doesn't exist: said plainly, the books unharmed.
    let s = call(&c, "books.back_up_now", json!({})).await.unwrap();
    let err = s["targets"][0]["last_error"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(err.contains("folder"), "{s}");
    assert_eq!(s["targets"][0]["disconnected"], true, "{s}");
    let start = call(&c, "books.start", json!({})).await.unwrap()["next_steps"].to_string();
    assert!(start.contains("behind or failing"), "{start}");
    assert!(!err.contains(&home.root().display().to_string()), "{err}");
    drop(c);
    assert!(server.close().success());
    std::fs::create_dir_all(&late).unwrap();
    let server = Server::start(&home, Duration::from_secs(3600));
    let c = connect(&home, "acme", home.token("acme", &ALL)).await;
    let s = backed_up(&c, Duration::from_secs(30)).await;
    assert_eq!(s["targets"][0]["last_error"], Value::Null, "{s}");
    assert_eq!(s["targets"][0]["disconnected"], false, "{s}");
    assert_eq!(proven(&home, &late, "late"), live_head(&home, "acme"));
    drop(c);
    assert!(server.close().success());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn backing_up_now_needs_admin_and_never_moves_the_head() {
    let home = Home::new("now");
    let folder = home.outside("disk");
    std::fs::create_dir_all(&folder).unwrap();
    let server = Server::start(&home, Duration::from_secs(3600));
    home.create("acme");
    let admin = connect(&home, "acme", home.token("acme", &ALL)).await;
    set_folder(&admin, &folder).await;
    post_one(&admin, 2).await;
    let before = head(&admin).await;
    // A reader sees the standing but can't back up.
    let reader = connect(&home, "acme", home.token("acme", &["read"])).await;
    assert!(call(&reader, "books.backup", json!({})).await.is_ok());
    let refused = call(&reader, "books.back_up_now", json!({}))
        .await
        .unwrap_err();
    assert!(refused.contains("admin"), "{refused}");
    // Twice: the second has nothing new to write, and still answers.
    for _ in 0..2 {
        let s = call(&admin, "books.back_up_now", json!({})).await.unwrap();
        assert_eq!(s["targets"][0]["through_seq"].as_u64(), Some(before), "{s}");
        assert_eq!(s["targets"][0]["last_error"], Value::Null, "{s}");
    }
    assert_eq!(head(&admin).await, before);
    assert_eq!(proven(&home, &folder, "now"), live_head(&home, "acme"));
    drop((admin, reader));
    assert!(server.close().success());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_backup_of_another_history_is_never_written_over() {
    // Two computers whose books have one id and different histories, one
    // folder: the second's backup is refused, the folder unchanged.
    let (a, b) = (Home::new("div-a"), Home::new("div-b"));
    let folder = a.outside("disk");
    std::fs::create_dir_all(&folder).unwrap();
    let (sa, sb) = (
        Server::start(&a, Duration::from_secs(3600)),
        Server::start(&b, Duration::from_secs(3600)),
    );
    a.create("acme");
    b.create("acme");
    let ca = connect(&a, "acme", a.token("acme", &ALL)).await;
    set_folder(&ca, &folder).await;
    post_one(&ca, 9).await;
    let s = call(&ca, "books.back_up_now", json!({})).await.unwrap();
    assert_eq!(s["targets"][0]["last_error"], Value::Null, "{s}");
    let theirs = proven(&a, &folder, "theirs");
    let cb = connect(&b, "acme", b.token("acme", &ALL)).await;
    set_folder(&cb, &folder).await;
    post_one(&cb, 1).await;
    let s = call(&cb, "books.back_up_now", json!({})).await.unwrap();
    assert!(
        s["targets"][0]["last_error"]
            .as_str()
            .unwrap()
            .contains("different history"),
        "{s}"
    );
    assert_eq!(s["targets"][0]["through_seq"], Value::Null, "{s}");
    assert_eq!(proven(&a, &folder, "after"), theirs);
    drop((ca, cb));
    assert!(sa.close().success());
    assert!(sb.close().success());
}

/// Restore `source` as `id`: the preview, then (if it was one) the
/// restore confirmed against its head. Ok(what was restored) or Err(why).
fn restore(home: &Home, id: &str, source: &Path, replace: bool) -> Result<admin::Restored, String> {
    restore_with(home, id, source, replace, None)
}

fn restore_with(
    home: &Home,
    id: &str,
    source: &Path,
    replace: bool,
    passphrase: Option<&str>,
) -> Result<admin::Restored, String> {
    let request = |expect_head: Option<(u64, String)>| Request::Restore {
        id: id.into(),
        source: source.display().to_string(),
        replace,
        confirmed: expect_head.is_some(),
        expect_head,
        passphrase: passphrase.map(|p| admin::Passphrase(p.to_string())),
    };
    let preview = match home.admin(request(None)) {
        Response::RestorePreview(p) => p,
        Response::Refused(why) => return Err(why),
        other => panic!("{other:?}"),
    };
    match home.admin(request(Some(preview.head.clone()))) {
        Response::Restored(r) => {
            assert_eq!(r.head, preview.head);
            assert_eq!(r.problem, None);
            Ok(r)
        }
        Response::Refused(why) => Err(why),
        other => panic!("{other:?}"),
    }
}

fn books_held(home: &Home) -> Vec<String> {
    match home.admin(Request::List) {
        Response::Books(b) => b.into_iter().map(|b| b.id).collect(),
        other => panic!("{other:?}"),
    }
}

/// What the books say, for comparing two that should be the same.
async fn figures(c: &Client) -> Value {
    json!({
        "tb": call(c, "books.trial_balance", json!({})).await.unwrap(),
        "receipts": call(c, "books.list_receipts", json!({})).await.unwrap(),
        "settings": call(c, "books.settings", json!({})).await.unwrap()["settings"].clone(),
    })
}

/// A backup of acme (two entries, a receipt each) in the folder.
async fn backed_up_acme(home: &Home) -> (PathBuf, Value) {
    settle().await;
    let folder = home.outside("disk");
    std::fs::create_dir_all(&folder).unwrap();
    let c = connect(home, "acme", home.token("acme", &ALL)).await;
    set_folder(&c, &folder).await;
    post_one(&c, 4).await;
    post_one(&c, 5).await;
    let s = call(&c, "books.back_up_now", json!({})).await.unwrap();
    assert_eq!(s["targets"][0]["last_error"], Value::Null, "{s}");
    (folder, figures(&c).await)
}

/// The files under `dir` whose path (below it) starts with `prefix`.
fn files_below(dir: &Path, prefix: &str) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(&p, out);
            } else {
                out.push(p);
            }
        }
    }
    let mut all = Vec::new();
    walk(dir, &mut all);
    all.retain(|p| {
        p.strip_prefix(dir)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/")
            .starts_with(prefix)
    });
    all.sort();
    all
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn restore_beside_existing_books_and_on_a_clean_install() {
    let home = Home::new("beside");
    let server = Server::start(&home, Duration::from_secs(3600));
    home.create("acme");
    let (folder, want) = backed_up_acme(&home).await;
    // Beside: its own books, the same figures and head.
    let r = restore(&home, "acme-copy", &folder, false).unwrap();
    assert_eq!(r.replaced, None);
    assert_eq!(r.head, live_head(&home, "acme"));
    let copy = connect(
        &home,
        "acme-copy",
        home.token("acme-copy", &["read", "admin"]),
    )
    .await;
    assert_eq!(figures(&copy).await, want);
    // Its own backups carry on in the same folder, under its own name; the
    // original's stay as they were.
    let orig = proven(&home, &folder.join("acme"), "orig-before");
    let s = call(&copy, "books.back_up_now", json!({})).await.unwrap();
    assert_eq!(s["targets"][0]["last_error"], Value::Null, "{s}");
    assert_eq!(
        proven(&home, &folder.join("acme-copy"), "copy"),
        live_head(&home, "acme-copy")
    );
    assert_eq!(proven(&home, &folder.join("acme"), "orig-after"), orig);
    drop(copy);
    assert!(server.close().success());
    // restore_on_clean_install: another computer's root, nothing in it.
    let other = Home::new("clean");
    let server = Server::start(&other, Duration::from_secs(3600));
    assert_eq!(books_held(&other), Vec::<String>::new());
    let r = restore(&other, "acme", &folder.join("acme"), false).unwrap();
    assert_eq!(r.head, live_head(&home, "acme"));
    let c = connect(&other, "acme", other.token("acme", &["read"])).await;
    assert_eq!(figures(&c).await, want);
    drop(c);
    assert!(server.close().success());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn restore_never_overwrites_and_replacing_keeps_the_old_books() {
    let home = Home::new("replace");
    let server = Server::start(&home, Duration::from_secs(3600));
    home.create("acme");
    let (folder, want) = backed_up_acme(&home).await;
    let at_backup = live_head(&home, "acme");
    let c = connect(&home, "acme", home.token("acme", &ALL)).await;
    post_one(&c, 6).await;
    let moved_on = live_head(&home, "acme");
    assert!(moved_on.0 > at_backup.0);
    // Not over them.
    let refused = restore(&home, "acme", &folder, false).unwrap_err();
    assert!(refused.contains("already books called"), "{refused}");
    assert_eq!(live_head(&home, "acme"), moved_on);
    // Replacing books that aren't there is refused too.
    let refused = restore(&home, "nobody", &folder, true).unwrap_err();
    assert!(refused.contains("no books called"), "{refused}");
    // Replacing: the old books kept whole in legacy/, the backup in place.
    let r = restore(&home, "acme", &folder, true).unwrap();
    let fresh = connect(&home, "acme", home.token("acme", &["read"])).await;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let s = call(&fresh, "books.backup", json!({})).await.unwrap();
        if s["targets"][0]["last_ok_at"].is_string() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the restored books weren't backed up: {s}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    drop(fresh);
    let kept = home.root().join("legacy").join(r.replaced.clone().unwrap());
    let old = ledger_store::Snapshot::load(&kept).unwrap();
    assert_eq!((old.journal_seq(), old.head_hash().to_string()), moved_on);
    assert_eq!(live_head(&home, "acme"), at_backup);
    let c = connect(&home, "acme", home.token("acme", &ALL)).await;
    assert_eq!(figures(&c).await, want);
    // And it is written to and backed up as before: the folder's backup of
    // these books goes on from where it was.
    post_one(&c, 7).await;
    let s = call(&c, "books.back_up_now", json!({})).await.unwrap();
    assert_eq!(s["targets"][0]["last_error"], Value::Null, "{s}");
    assert_eq!(proven(&home, &folder, "after"), live_head(&home, "acme"));
    drop(c);
    assert!(server.close().success());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn restore_refuses_a_damaged_backup_and_writes_nothing() {
    let home = Home::new("damaged");
    let server = Server::start(&home, Duration::from_secs(3600));
    home.create("acme");
    let (folder, _) = backed_up_acme(&home).await;
    let before = books_held(&home);
    let books = home.root().join("books");
    let names = || {
        let mut v: Vec<String> = std::fs::read_dir(&books)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    };
    let listed = names();
    // Each damage in a copy of its own, made from the good one.
    type Spoil<'a> = (&'a str, &'a dyn Fn(&Path));
    let damage: [Spoil; 5] = [
        ("flipped", &|set: &Path| {
            let j = &files_below(set, "journal/")[0];
            let mut b = std::fs::read(j).unwrap();
            let mid = b.len() / 2;
            b[mid] ^= 1;
            std::fs::write(j, b).unwrap();
        }),
        ("no-files", &|set: &Path| {
            for f in files_below(set, "files/") {
                std::fs::remove_file(f).unwrap();
            }
        }),
        ("cut", &|set: &Path| {
            let j = &files_below(set, "journal/")[0];
            let mut b = std::fs::read(j).unwrap();
            b.truncate(b.len() - 10);
            std::fs::write(j, b).unwrap();
        }),
        ("manifest-cut", &|set: &Path| {
            for f in files_below(set, "manifests/") {
                let b = std::fs::read(&f).unwrap();
                std::fs::write(&f, &b[..b.len() / 2]).unwrap();
            }
        }),
        ("no-manifest", &|set: &Path| {
            for f in files_below(set, "manifests/") {
                std::fs::remove_file(f).unwrap();
            }
        }),
    ];
    for (tag, spoil) in damage {
        let copy = home.outside(&format!("bad-{tag}"));
        copy_dir(&folder.join("acme"), &copy);
        spoil(&copy);
        let why = restore(&home, "acme-bad", &copy, false).unwrap_err();
        assert!(why.contains("can't be restored"), "{tag}: {why}");
        assert!(
            !why.contains(&home.root().display().to_string()),
            "{tag}: {why}"
        );
        assert!(!why.contains(&copy.display().to_string()), "{tag}: {why}");
        assert_eq!(books_held(&home), before, "{tag}");
        assert_eq!(names(), listed, "{tag}");
    }
    // A place that isn't a backup: said, nothing written.
    let why = restore(&home, "acme-bad", &home.outside("nothing-here"), false).unwrap_err();
    assert!(!why.contains("nothing-here"), "{why}");
    assert_eq!(names(), listed);
    assert!(server.close().success());
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let target = to.join(e.file_name());
        if e.path().is_dir() {
            copy_dir(&e.path(), &target);
        } else {
            std::fs::copy(e.path(), target).unwrap();
        }
    }
}

/// Wait until the folder's backup has been drilled.
async fn drilled(c: &Client) -> Value {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let s = call(c, "books.backup", json!({})).await.unwrap();
        let targets = s["targets"].as_array().unwrap();
        if !targets.is_empty() && targets.iter().all(|t| t["last_drill_at"].is_string()) {
            return s;
        }
        assert!(Instant::now() < deadline, "no drill: {s}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn opening_the_books_drills_a_backup_never_drilled() {
    let home = Home::new("drill");
    let server = Server::start(&home, Duration::from_secs(3600));
    home.create("acme");
    let (folder, _) = backed_up_acme(&home).await;
    assert!(server.close().success());
    let server = Server::start_with(
        &home,
        Duration::from_secs(3600),
        Some(Duration::from_millis(300)),
    );
    let c = connect(&home, "acme", home.token("acme", &["read", "admin"])).await;
    let s = drilled(&c).await;
    assert_eq!(s["targets"][0]["drill_error"], Value::Null, "{s}");
    let start = call(&c, "books.start", json!({})).await.unwrap()["next_steps"].to_string();
    assert!(!start.contains("books.backup"), "{start}");
    assert!(proven(&home, &folder, "drilled").0 > 0);
    drop(c);
    assert!(server.close().success());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn restore_drill_failure_is_flagged() {
    let home = Home::new("drill-bad");
    let server = Server::start(&home, Duration::from_secs(3600));
    home.create("acme");
    let (folder, _) = backed_up_acme(&home).await;
    assert!(server.close().success());
    // The backup is changed behind the books' back.
    let j = &files_below(&folder.join("acme"), "journal/")[0];
    let mut b = std::fs::read(j).unwrap();
    let mid = b.len() / 2;
    b[mid] ^= 1;
    std::fs::write(j, b).unwrap();
    // Opening the books: its first drill fails, said in books.backup and
    // books.start.
    let server = Server::start_with(
        &home,
        Duration::from_secs(3600),
        Some(Duration::from_millis(300)),
    );
    let c = connect(&home, "acme", home.token("acme", &["read", "admin"])).await;
    let s = drilled(&c).await;
    assert!(s["targets"][0]["drill_error"].is_string(), "{s}");
    let start = call(&c, "books.start", json!({})).await.unwrap()["next_steps"].to_string();
    assert!(start.contains("behind or failing"), "{start}");
    drop(c);
    assert!(server.close().success());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_write_waits_for_the_quiet_before_it_is_backed_up() {
    let home = Home::new("quiet");
    let server = Server::start(&home, Duration::from_secs(3600));
    home.create("acme");
    backed_up_acme(&home).await;
    let c = connect(&home, "acme", home.token("acme", &ALL)).await;
    post_one(&c, 8).await;
    // Several looks of the loop later, still not: the writing hasn't
    // stopped for long enough.
    tokio::time::sleep(Duration::from_secs(5)).await;
    let s = call(&c, "books.backup", json!({})).await.unwrap();
    let head = s["head_seq"].as_u64().unwrap();
    assert!(
        s["targets"][0]["through_seq"].as_u64().unwrap() < head,
        "{s}"
    );
    assert_eq!(s["targets"][0]["stale"], false, "{s}");
    drop(c);
    assert!(server.close().success());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_restore_is_of_the_backup_previewed_only() {
    let home = Home::new("previewed");
    let server = Server::start(&home, Duration::from_secs(3600));
    home.create("acme");
    let (folder, _) = backed_up_acme(&home).await;
    let request = |expect_head: Option<(u64, String)>, confirmed| Request::Restore {
        id: "copy".into(),
        source: folder.display().to_string(),
        replace: false,
        confirmed,
        expect_head,
        passphrase: None,
    };
    // A preview leaves nothing behind in the books' folder, and clears
    // what a restore that never finished left.
    let books = home.root().join("books");
    std::fs::create_dir(books.join(".restore-junk")).unwrap();
    let previewed = match home.admin(request(None, false)) {
        Response::RestorePreview(p) => p.head,
        other => panic!("{other:?}"),
    };
    let dotted: Vec<_> = std::fs::read_dir(&books)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with('.'))
        .collect();
    assert!(dotted.is_empty(), "{dotted:?}");
    // Confirmed with no head: refused.
    match home.admin(request(None, true)) {
        Response::Refused(why) => assert!(why.contains("preview first"), "{why}"),
        other => panic!("{other:?}"),
    }
    // The backup moves on after the preview: refused, nothing written.
    let c = connect(&home, "acme", home.token("acme", &ALL)).await;
    post_one(&c, 9).await;
    call(&c, "books.back_up_now", json!({})).await.unwrap();
    match home.admin(request(Some(previewed), true)) {
        Response::Refused(why) => assert!(why.contains("isn't the one previewed"), "{why}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(books_held(&home), vec!["acme".to_string()]);
    assert!(!home.root().join("books").join("copy").exists());
    drop(c);
    assert!(server.close().success());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_restore_brings_the_vault_and_its_key() {
    let home = Home::new("vault");
    let server = Server::start(&home, Duration::from_secs(3600));
    home.create("acme");
    let c = connect(&home, "acme", home.token("acme", &ALL)).await;
    call(
        &c,
        "books.add_party",
        json!({"name": "Mo", "roles": ["member"]}),
    )
    .await
    .unwrap();
    call(
        &c,
        "books.set_tin",
        json!({"party": "Mo", "tin": "12-3456789", "tin_type": "ein"}),
    )
    .await
    .unwrap();
    let (folder, want) = backed_up_acme(&home).await;
    drop(c);
    assert!(server.close().success());
    let other = Home::new("vault-clean");
    let server = Server::start(&other, Duration::from_secs(3600));
    let request = |expect_head: Option<(u64, String)>| Request::Restore {
        id: "acme".into(),
        source: folder.display().to_string(),
        replace: false,
        confirmed: expect_head.is_some(),
        expect_head,
        passphrase: None,
    };
    let preview = match other.admin(request(None)) {
        Response::RestorePreview(p) => p,
        other => panic!("{other:?}"),
    };
    assert_eq!(preview.vault_files, 1);
    let r = match other.admin(request(Some(preview.head))) {
        Response::Restored(r) => r,
        other => panic!("{other:?}"),
    };
    assert_eq!(r.vault_files, 1);
    // The record and its key, byte for byte.
    let files = |h: &Home, sub: &str| {
        let mut v: Vec<(String, Vec<u8>)> = std::fs::read_dir(h.book_dir("acme").join(sub))
            .unwrap()
            .map(|e| {
                let e = e.unwrap();
                (
                    e.file_name().to_string_lossy().into_owned(),
                    std::fs::read(e.path()).unwrap(),
                )
            })
            .collect();
        v.sort();
        v
    };
    for sub in ["vault", "keys"] {
        assert!(!files(&home, sub).is_empty(), "{sub}");
        assert_eq!(files(&other, sub), files(&home, sub), "{sub}");
    }
    let c = connect(&other, "acme", other.token("acme", &["read"])).await;
    assert_eq!(figures(&c).await, want);
    let party = call(&c, "books.party", json!({"party": "Mo"}))
        .await
        .unwrap();
    assert_eq!(party["party"]["tin"]["in_vault"], true, "{party}");
    drop(c);
    assert!(server.close().success());
    // A backup missing the vault's key, or the TIN record the books name,
    // is not whole: refused, nothing restored.
    let third = Home::new("vault-short");
    let server = Server::start(&third, Duration::from_secs(3600));
    for gone in ["keys/", "vault/"] {
        let copy = home.outside(&format!("no-{}", gone.trim_end_matches('/')));
        copy_dir(&folder.join("acme"), &copy);
        for f in files_below(&copy, gone) {
            std::fs::remove_file(f).unwrap();
        }
        let why = restore(&third, "acme", &copy, false).unwrap_err();
        assert!(why.contains("can't be restored"), "{gone}: {why}");
        assert_eq!(books_held(&third), Vec::<String>::new());
    }
    assert!(server.close().success());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_replacing_restore_that_fails_leaves_the_books_open() {
    let home = Home::new("replace-bad");
    let server = Server::start(&home, Duration::from_secs(3600));
    home.create("acme");
    let (folder, want) = backed_up_acme(&home).await;
    let before = live_head(&home, "acme");
    let bad = home.outside("bad");
    copy_dir(&folder.join("acme"), &bad);
    let j = &files_below(&bad, "journal/")[0];
    let mut b = std::fs::read(j).unwrap();
    let mid = b.len() / 2;
    b[mid] ^= 1;
    std::fs::write(j, b).unwrap();
    // Its preview is refused; so is a confirm (sent without one), which
    // lets the books go first, then fails: they are open again, as they
    // were, nothing moved to legacy.
    assert!(restore(&home, "acme", &bad, true).is_err());
    let why = match home.admin(Request::Restore {
        id: "acme".into(),
        source: bad.display().to_string(),
        replace: true,
        confirmed: true,
        expect_head: Some(before.clone()),
        passphrase: None,
    }) {
        Response::Refused(why) => why,
        other => panic!("{other:?}"),
    };
    assert!(why.contains("can't be"), "{why}");
    match home.admin(Request::List) {
        Response::Books(b) => {
            assert_eq!(b.len(), 1);
            assert!(b[0].open, "{b:?}");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(live_head(&home, "acme"), before);
    let legacy = home.root().join("legacy");
    assert!(!legacy.exists() || std::fs::read_dir(&legacy).unwrap().count() == 0);
    let c = connect(&home, "acme", home.token("acme", &ALL)).await;
    assert_eq!(figures(&c).await, want);
    post_one(&c, 3).await;
    drop(c);
    assert!(server.close().success());
}

/// The book's backup record (the test's own data root).
fn records(home: &Home, book: &str) -> Vec<Value> {
    std::fs::read_to_string(home.book_dir(book).join("ops").join("backup.jsonl"))
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn restored_books_never_make_their_backup_folder_again() {
    let home = Home::new("restore-mount");
    let server = Server::start(&home, Duration::from_secs(3600));
    home.create("acme");
    let (folder, _) = backed_up_acme(&home).await;
    assert!(server.close().success());
    // On another computer, its backup disk not mounted (the mount point
    // there, empty): restored from a copy of the backup.
    let copy = home.outside("copy-of-disk");
    copy_dir(&folder, &copy);
    std::fs::rename(folder.join("acme"), home.outside("unmounted")).unwrap();
    let other = Home::new("restore-mount-new");
    let server = Server::start(&other, Duration::from_secs(3600));
    restore(&other, "acme", &copy, false).unwrap();
    let c = connect(&other, "acme", other.token("acme", &["read", "admin"])).await;
    let s = call(&c, "books.back_up_now", json!({})).await.unwrap();
    assert!(
        s["targets"][0]["last_error"]
            .as_str()
            .unwrap()
            .contains("holds no backup"),
        "{s}"
    );
    assert!(!folder.join("acme").exists(), "made on the wrong disk");
    // The human says it's a new disk: begun there again.
    let s = call(&c, "books.back_up_now", json!({"new_disk": true}))
        .await
        .unwrap();
    assert_eq!(s["targets"][0]["last_error"], Value::Null, "{s}");
    assert!(folder.join("acme").join("manifests").is_dir());
    drop(c);
    assert!(server.close().success());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_damaged_backup_record_never_keeps_the_books_shut() {
    let home = Home::new("damaged-record");
    let server = Server::start(&home, Duration::from_secs(3600));
    home.create("acme");
    let (folder, _) = backed_up_acme(&home).await;
    assert!(server.close().success());
    let log = home.book_dir("acme").join("ops").join("backup.jsonl");
    let text = std::fs::read_to_string(&log).unwrap();
    std::fs::write(&log, text.replacen("\"ok\":true", "\"ok\":false", 1)).unwrap();
    let server = Server::start(&home, Duration::from_secs(3600));
    match home.admin(Request::List) {
        Response::Books(b) => assert!(b[0].open, "{b:?}"),
        other => panic!("{other:?}"),
    }
    let aside: Vec<_> = std::fs::read_dir(home.book_dir("acme").join("ops"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains("damaged"))
        .collect();
    assert_eq!(aside.len(), 1, "{aside:?}");
    let c = connect(&home, "acme", home.token("acme", &["read", "admin"])).await;
    let s = call(&c, "books.back_up_now", json!({})).await.unwrap();
    assert_eq!(s["targets"][0]["last_error"], Value::Null, "{s}");
    drop(c);
    assert!(server.close().success());
    // What it knew of whose backups the folder holds was kept: gone, the
    // folder's backup is a disk not connected, never made again.
    std::fs::rename(folder.join("acme"), home.outside("away")).unwrap();
    let server = Server::start(&home, Duration::from_secs(3600));
    let c = connect(&home, "acme", home.token("acme", &["read", "admin"])).await;
    let s = call(&c, "books.back_up_now", json!({})).await.unwrap();
    assert!(
        s["targets"][0]["last_error"]
            .as_str()
            .unwrap()
            .contains("holds no backup"),
        "{s}"
    );
    assert!(!folder.join("acme").exists());
    drop(c);
    assert!(server.close().success());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_failing_destination_is_not_tried_again_after_each_write() {
    let home = Home::new("failing");
    let server = Server::start(&home, Duration::from_millis(300));
    home.create("acme");
    settle().await;
    let unplugged = home.outside("unplugged");
    let c = connect(&home, "acme", home.token("acme", &ALL)).await;
    set_folder(&c, &unplugged).await;
    let s = call(&c, "books.back_up_now", json!({})).await.unwrap();
    let pushes = |home: &Home| {
        records(home, "acme")
            .iter()
            .filter(|r| r["kind"] == "push")
            .count()
    };
    let before = pushes(&home);
    assert!(before >= 1, "{s}");
    // Writes: the folder isn't tried again while it failed lately.
    for n in 0..3 {
        post_one(&c, 10 + n).await;
        tokio::time::sleep(Duration::from_millis(1500)).await;
    }
    assert_eq!(pushes(&home), before);
    drop(c);
    assert!(server.close().success());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn old_settings_and_an_old_backup_history_are_left_alone_and_said() {
    // Books that name a private repository, and carry an old backup
    // history in their folder: they open and work; the status says plainly
    // what is no longer used; nothing of the old is read, moved or deleted.
    let home = Home::new("old-forms");
    let server = Server::start(&home, Duration::from_secs(3600));
    home.create("acme");
    settle().await;
    let c = connect(&home, "acme", home.token("acme", &ALL)).await;
    call(
        &c,
        "books.set_setting",
        json!({"changes": [{"key": "backup.git_remote", "value": "https://example.invalid/books"}],
               "decided_by": "Test Owner", "note": "Old."}),
    )
    .await
    .unwrap();
    let old = home.book_dir("acme").join("archive.git");
    drop(c);
    assert!(server.close().success());
    std::fs::create_dir_all(old.join("objects")).unwrap();
    std::fs::write(old.join("HEAD"), "ref: refs/heads/main\n").unwrap();
    let before = std::fs::read(old.join("HEAD")).unwrap();
    let server = Server::start(&home, Duration::from_secs(3600));
    let c = connect(&home, "acme", home.token("acme", &ALL)).await;
    let s = call(&c, "books.backup", json!({})).await.unwrap();
    let note = s["note"].as_str().unwrap().to_string();
    assert!(note.contains("private repository"), "{note}");
    assert!(note.contains("no longer"), "{note}");
    assert!(note.contains("older backup folder"), "{note}");
    assert!(!note.to_lowercase().contains("git"), "{note}");
    // The books work, and back up to a folder when one is named; the old
    // is untouched.
    let folder = home.outside("disk");
    std::fs::create_dir_all(&folder).unwrap();
    set_folder(&c, &folder).await;
    post_one(&c, 1).await;
    let s = call(&c, "books.back_up_now", json!({})).await.unwrap();
    assert_eq!(s["targets"][0]["last_error"], Value::Null, "{s}");
    assert_eq!(std::fs::read(old.join("HEAD")).unwrap(), before);
    assert!(folder.join("acme").join("manifests").is_dir());
    drop(c);
    assert!(server.close().success());
}
