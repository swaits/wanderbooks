//! What a person is told when their backups are in trouble (WB-backup-13 to
//! WB-backup-16): the key file lost from a sealed destination, books behind
//! the folder's (a new set at their request), a newest backup that is
//! damaged while an older one is whole, and a folder a sync service keeps.
#![cfg(unix)]
#![allow(dead_code)]

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
    verify_found(&found, &scratch, &Ctx::new())
        .unwrap_or_else(|e| panic!("{tag}: {e}"))
        .head
}

fn live_head(home: &Home, book: &str) -> (u64, String) {
    let s = ledger_store::Snapshot::load(&home.book_dir(book)).unwrap();
    (s.journal_seq(), s.head_hash().to_string())
}

const ALL: [&str; 4] = ["read", "write", "post", "admin"];

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

const PASS: &str = "correct horse battery staple";

fn said(r: Response) -> String {
    match r {
        Response::Said { lines } => lines.join(" "),
        other => panic!("{other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_lost_key_file_never_makes_backups_plain() {
    let home = Home::new("lost-key");
    let server = Server::start(&home, Duration::from_secs(3600));
    home.create("acme");
    let folder = home.outside("disk");
    std::fs::create_dir_all(&folder).unwrap();
    said(home.admin(Request::BackupSetup {
        id: "acme".into(),
        folder: Some(folder.display().to_string()),
        passphrase: Some(admin::Passphrase(PASS.into())),
        remove_passphrase: false,
    }));
    let c = connect(&home, "acme", home.token("acme", &ALL)).await;
    post_one(&c, 1).await;
    let s = call(&c, "books.back_up_now", json!({})).await.unwrap();
    assert_eq!(s["targets"][0]["last_error"], Value::Null, "{s}");
    assert_eq!(s["targets"][0]["sealed"], true);
    // The key file is lost from this computer (a restored home folder, a
    // cleaned `local/`).
    let key = home.book_dir("acme").join("local").join("backup-key.json");
    std::fs::remove_file(&key).unwrap();
    let before: Vec<_> = files_below(&folder, "");
    post_one(&c, 2).await;
    let s = call(&c, "books.back_up_now", json!({})).await.unwrap();
    let t = &s["targets"][0];
    // Told in words, and nothing was written: no plain copy of the books.
    let why = t["last_error"].as_str().unwrap_or_default();
    assert!(why.contains("no longer has its key"), "{s}");
    assert!(why.contains("remove the passphrase"), "{s}");
    assert_eq!(t["error_kind"], "key_missing", "{s}");
    assert_eq!(files_below(&folder, ""), before, "nothing was written");
    assert!(!folder.join("wanderbooks-acme").exists());
    // The words a person reads in the window.
    let standing = match home.admin(Request::BackupStatus { id: "acme".into() }) {
        Response::BackupStatus(st) => st,
        other => panic!("{other:?}"),
    };
    let words = ledger_host::commands::backup_words(&standing).join(" | ");
    assert!(words.contains("no longer has its key"), "{words}");
    assert!(
        words.contains("Enter the backup passphrase again"),
        "{words}"
    );

    // Even with the backup record gone (set aside as damaged at the next
    // open), the folder itself says its backups are sealed.
    drop(c);
    assert!(server.close().success());
    std::fs::write(
        home.book_dir("acme").join("ops").join("backup.jsonl"),
        b"not a record\n",
    )
    .unwrap();
    let server = Server::start(&home, Duration::from_secs(3600));
    let c = connect(&home, "acme", home.token("acme", &ALL)).await;
    let s = call(&c, "books.back_up_now", json!({"new_disk": true}))
        .await
        .unwrap();
    assert_eq!(s["targets"][0]["error_kind"], "key_missing", "{s}");
    assert!(!folder.join("wanderbooks-acme").exists());

    // Plain is written only after an explicit "remove the passphrase".
    said(home.admin(Request::BackupSetup {
        id: "acme".into(),
        folder: None,
        passphrase: None,
        remove_passphrase: true,
    }));
    let s = call(&c, "books.back_up_now", json!({"new_disk": true}))
        .await
        .unwrap();
    assert_eq!(s["targets"][0]["last_error"], Value::Null, "{s}");
    assert_eq!(s["targets"][0]["sealed"], false);
    assert!(folder.join("wanderbooks-acme").join("manifests").is_dir());
    drop(c);
    assert!(server.close().success());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn books_restored_from_an_older_backup_are_ahead_and_a_new_set_begins_at_their_request() {
    // Books A back up twice to a folder; books C are restored from a copy of
    // the folder as it was after the first, and then pointed at the folder
    // that holds more.
    let a = Home::new("ahead-a");
    let server_a = Server::start(&a, Duration::from_secs(3600));
    a.create("acme");
    settle().await;
    let folder = a.outside("disk");
    std::fs::create_dir_all(&folder).unwrap();
    let ca = connect(&a, "acme", a.token("acme", &ALL)).await;
    set_folder(&ca, &folder).await;
    post_one(&ca, 1).await;
    let s = call(&ca, "books.back_up_now", json!({})).await.unwrap();
    assert_eq!(s["targets"][0]["last_error"], Value::Null, "{s}");
    let older = a.outside("older");
    copy_dir(&folder, &older);
    post_one(&ca, 2).await;
    let s = call(&ca, "books.back_up_now", json!({})).await.unwrap();
    assert_eq!(s["targets"][0]["last_error"], Value::Null, "{s}");
    let longer = live_head(&a, "acme");
    drop(ca);
    assert!(server_a.close().success());

    let c_home = Home::new("ahead-c");
    let server_c = Server::start(&c_home, Duration::from_secs(3600));
    let restored = restore(&c_home, "acme", &older.join("wanderbooks-acme"), false).unwrap();
    assert!(restored.head.0 < longer.0);
    let cc = connect(&c_home, "acme", c_home.token("acme", &ALL)).await;
    set_folder(&cc, &folder).await;
    let before = files_below(&folder, "");
    let bytes: Vec<Vec<u8>> = before.iter().map(|p| std::fs::read(p).unwrap()).collect();
    let s = call(&cc, "books.back_up_now", json!({})).await.unwrap();
    let t = &s["targets"][0];
    // Told as ahead (not diverged), in words that say what to do.
    assert_eq!(t["error_kind"], "ahead", "{s}");
    let why = t["last_error"].as_str().unwrap();
    assert!(why.contains("a longer history of these books"), "{why}");
    assert!(why.contains("start a new backup set"), "{why}");
    assert_eq!(files_below(&folder, ""), before, "nothing was written");
    let standing = match c_home.admin(Request::BackupStatus { id: "acme".into() }) {
        Response::BackupStatus(st) => st,
        other => panic!("{other:?}"),
    };
    let words = ledger_host::commands::backup_words(&standing).join(" | ");
    assert!(words.contains("start a new backup set"), "{words}");

    // At the person's request: a new set beside the old, which is
    // untouched; and the backups go on there.
    match c_home.admin(Request::BackUpNewSet { id: "acme".into() }) {
        Response::BackedUp { targets, .. } => {
            assert!(targets.iter().all(|t| t.problem.is_none()), "{targets:?}");
        }
        other => panic!("{other:?}"),
    }
    assert!(folder.join("wanderbooks-acme.2").join("manifests").is_dir());
    for (p, b) in before.iter().zip(&bytes) {
        assert_eq!(&std::fs::read(p).unwrap(), b, "{}", p.display());
    }
    post_one(&cc, 9).await;
    let s = call(&cc, "books.back_up_now", json!({})).await.unwrap();
    assert_eq!(s["targets"][0]["last_error"], Value::Null, "{s}");
    assert_eq!(s["targets"][0]["error_kind"], Value::Null, "{s}");
    assert_eq!(
        proven(&c_home, &folder.join("wanderbooks-acme.2"), "new").0,
        live_head(&c_home, "acme").0
    );
    // The old set, still whole, still holds the longer history.
    assert_eq!(
        proven(&c_home, &folder.join("wanderbooks-acme"), "old"),
        longer
    );
    drop(cc);
    assert!(server_c.close().success());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_drill_that_finds_only_an_older_backup_is_a_failure_in_words_and_never_ok() {
    let home = Home::new("older-only");
    let server = Server::start(&home, Duration::from_secs(3600));
    home.create("acme");
    let (folder, _) = backed_up_acme(&home).await;
    let c = connect(&home, "acme", home.token("acme", &ALL)).await;
    post_one(&c, 6).await;
    let s = call(&c, "books.back_up_now", json!({})).await.unwrap();
    assert_eq!(s["targets"][0]["last_error"], Value::Null, "{s}");
    drop(c);
    assert!(server.close().success());
    // The newest backup's own segment is damaged after the fact; the one
    // before it is whole.
    let set = folder.join("wanderbooks-acme");
    let newest = files_below(&set, "manifests/").pop().unwrap();
    let text = std::fs::read_to_string(&newest).unwrap();
    let m: Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
    // Whatever the newest manifest itself adds: a journal, access or index
    // segment (a manifest names only what is new).
    let seg = ["journal", "access", "files"]
        .iter()
        .find_map(|k| m[*k]["segments"].as_array()?.last().cloned())
        .expect("the newest manifest adds a segment");
    let seg = set.join(seg["file"].as_str().unwrap());
    let mut b = std::fs::read(&seg).unwrap();
    let mid = b.len() / 2;
    b[mid] ^= 1;
    std::fs::write(&seg, b).unwrap();
    let server = Server::start_with(
        &home,
        Duration::from_secs(3600),
        Some(Duration::from_millis(300)),
    );
    let c = connect(&home, "acme", home.token("acme", &["read", "admin"])).await;
    let s = drilled(&c).await;
    let t = &s["targets"][0];
    let why = t["drill_error"].as_str().unwrap_or_default();
    assert!(why.contains("the newest backup"), "{s}");
    assert!(why.contains("is damaged; an older one from"), "{s}");
    assert_eq!(t["error_kind"], "newest_damaged", "{s}");
    let standing = match home.admin(Request::BackupStatus { id: "acme".into() }) {
        Response::BackupStatus(st) => st,
        other => panic!("{other:?}"),
    };
    let words = ledger_host::commands::backup_words(&standing).join(" | ");
    assert!(words.contains("failed"), "{words}");
    assert!(words.contains("older books"), "{words}");
    assert!(!words.contains("Proven restorable"), "{words}");
    // A restore from the folder says so before anything else, with both
    // dates and how many backups it passes over. (The books' own next
    // backup would find the damage and write the piece again, so the
    // damage is made again, after the books are closed.)
    drop(c);
    assert!(server.close().success());
    let newest = files_below(&set, "manifests/").pop().unwrap();
    let text = std::fs::read_to_string(&newest).unwrap();
    let m: Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
    let seg = ["journal", "access", "files"]
        .iter()
        .find_map(|k| m[*k]["segments"].as_array()?.last().cloned())
        .expect("the newest manifest adds a segment");
    let seg = set.join(seg["file"].as_str().unwrap());
    let mut b = std::fs::read(&seg).unwrap();
    let mid = b.len() / 2;
    b[mid] ^= 1;
    std::fs::write(&seg, b).unwrap();
    let other = Home::new("older-only-new");
    let server = Server::start(&other, Duration::from_secs(3600));
    match other.admin(Request::Restore {
        id: "acme".into(),
        source: folder.display().to_string(),
        replace: false,
        confirmed: false,
        expect_head: None,
        passphrase: None,
    }) {
        Response::RestorePreview(p) => {
            assert!(p.passed_over >= 1, "{p:?}");
            assert!(p.written.is_some() && p.newest_written.is_some(), "{p:?}");
            let words = ledger_host::commands::older_backup_words(&p);
            assert!(words.starts_with("NOT THE NEWEST BACKUP"), "{words}");
            assert!(words.contains("passes over "), "{words}");
            assert!(words.contains(" newer backup"), "{words}");
            assert!(words.contains("This backup is from"), "{words}");
            assert!(
                words.contains("the newest in the folder is from"),
                "{words}"
            );
        }
        other => panic!("{other:?}"),
    }
    assert!(server.close().success());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_folder_a_sync_service_keeps_is_said_in_plain_words_at_set_up() {
    let home = Home::new("sync");
    let server = Server::start(&home, Duration::from_secs(3600));
    home.create("acme");
    let dropbox = home.outside("Dropbox/Backups");
    std::fs::create_dir_all(&dropbox).unwrap();
    let words = said(home.admin(Request::BackupSetup {
        id: "acme".into(),
        folder: Some(dropbox.display().to_string()),
        passphrase: None,
        remove_passphrase: false,
    }));
    assert!(words.contains("kept in sync by Dropbox"), "{words}");
    assert!(words.contains("one computer only"), "{words}");
    assert!(words.contains("available offline"), "{words}");
    assert!(words.contains("a folder of its own"), "{words}");
    // A folder no service is known to keep still gets the general words.
    let plain = home.outside("a-usb-stick");
    std::fs::create_dir_all(&plain).unwrap();
    let words = said(home.admin(Request::BackupSetup {
        id: "acme".into(),
        folder: Some(plain.display().to_string()),
        passphrase: None,
        remove_passphrase: false,
    }));
    assert!(
        words.contains("If this folder is kept in sync by a service"),
        "{words}"
    );
    assert!(words.contains("one computer only"), "{words}");
    // And the window shows the same, under the status.
    let standing = match home.admin(Request::BackupStatus { id: "acme".into() }) {
        Response::BackupStatus(st) => st,
        other => panic!("{other:?}"),
    };
    let lines = ledger_host::commands::backup_words(&standing).join(" | ");
    assert!(lines.contains("one computer only"), "{lines}");
    assert!(server.close().success());
}
