//! The server's backups (plan 3.7, D-30, D-31): every book served from the
//! data root backs itself up to the targets its settings name, a while
//! after it is written, when it opens, before it closes, and when asked.
//! Oracles: the INVARIANTS of ledger-mcp backup.rs and ledger-store
//! backup.rs, and plan 3.7's named tests.
#![cfg(unix)]

use ledger_mcp::admin::{self, Request, Response};
use ledger_store::backup::{self, Git};
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
        self.dir.path().join(".local/share/wanderbooks")
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

    /// A bare repository standing for the private git remote.
    fn remote(&self) -> PathBuf {
        let p = self.outside("remote.git");
        let out = Command::new("git")
            .args(["init", "-q", "--bare", "-b", "main"])
            .arg(&p)
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
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

async fn connect(home: &Home, book: &str, token: String) -> Client {
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
            .auth_header(token);
    ClientConfig::default()
        .serve(StreamableHttpClientTransport::with_client(http, config))
        .await
        .unwrap()
}

/// Call a tool: Ok(structured result) or Err(the error text).
async fn call(c: &Client, tool: &str, args: Value) -> Result<Value, String> {
    let args = args.as_object().cloned().unwrap_or_default();
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

/// Name the backup targets in the book's settings.
async fn set_targets(c: &Client, remote: Option<&Path>, folder: Option<&Path>) {
    let mut changes = Vec::new();
    if let Some(r) = remote {
        changes.push(json!({"key": "backup.git_remote", "value": r.display().to_string()}));
    }
    if let Some(f) = folder {
        changes.push(json!({"key": "backup.folder", "value": f.display().to_string()}));
    }
    call(
        c,
        "books.set_setting",
        json!({"changes": changes, "decided_by": "Test Owner", "note": "Backups."}),
    )
    .await
    .unwrap();
}

/// Draft, receipt and post one $n entry.
async fn post_one(c: &Client, n: u64) {
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
    call(
        c,
        "books.attach_receipt",
        json!({"entry_id": id, "file_name": format!("r{n}.txt"),
               "mime_type": "text/plain", "content_base64": "cmVjZWlwdA==",
               "sha256": "6f32860910ca0fb2a20c7fda143666b09dbf8db5238195c90a586fb542ff0cad"}),
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

/// Prove the backup at `source` whole; its head.
fn proven(home: &Home, source: &Path, tag: &str) -> (u64, String) {
    let scratch = home.outside(&format!("scratch-{tag}"));
    let v = backup::verify(
        &Git::find().unwrap(),
        &source.display().to_string(),
        &scratch,
    )
    .unwrap_or_else(|e| panic!("{tag}: {e}"));
    v.head
}

fn live_head(home: &Home, book: &str) -> (u64, String) {
    let s = ledger_store::Snapshot::load(&home.book_dir(book)).unwrap();
    (s.journal_seq(), s.head_hash().to_string())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_book_backs_itself_up_to_both_targets_soon_after_a_write() {
    let home = Home::new("both");
    let server = Server::start(&home, Duration::from_millis(300));
    home.create("acme");
    let c = connect(
        &home,
        "acme",
        home.token("acme", &["read", "write", "post", "admin"]),
    )
    .await;
    // None set up: said, and nothing is stale.
    let s = call(&c, "books.backup", json!({})).await.unwrap();
    assert_eq!(s["targets"], json!([]));
    assert!(
        s["note"].as_str().unwrap().contains("backup.git_remote"),
        "{s}"
    );
    // books.start says so too.
    let steps = |v: Value| v["next_steps"].to_string();
    let start = steps(call(&c, "books.start", json!({})).await.unwrap());
    assert!(start.contains("No backups are set up"), "{start}");
    let (remote, folder) = (home.remote(), home.outside("disk"));
    std::fs::create_dir_all(&folder).unwrap();
    set_targets(&c, Some(&remote), Some(&folder)).await;
    post_one(&c, 7).await;
    let s = backed_up(&c, Duration::from_secs(30)).await;
    assert_eq!(s["note"], Value::Null);
    let targets = s["targets"].as_array().unwrap();
    assert_eq!(targets.len(), 2, "{s}");
    for t in targets {
        assert_eq!(t["stale"], false, "{t}");
        assert_eq!(t["last_error"], Value::Null, "{t}");
        assert!(t["last_ok_at"].is_string(), "{t}");
    }
    let start = steps(call(&c, "books.start", json!({})).await.unwrap());
    assert!(!start.contains("books.backup"), "{start}");
    // backup_does_not_advance_head: the books' head is what was written.
    let live = live_head(&home, "acme");
    assert_eq!(s["head_seq"].as_u64(), Some(live.0));
    // Each target holds the books whole, at their head.
    assert_eq!(proven(&home, &remote, "remote"), live);
    assert_eq!(proven(&home, &folder.join("acme.git"), "folder"), live);
    // Its journal is the live journal's beginning (a drill's check).
    let tree = home.outside("scratch-folder");
    assert!(
        backup::is_prefix(
            &home.book_dir("acme").join(ledger_store::JOURNAL_FILE),
            &tree.join(ledger_store::JOURNAL_FILE)
        )
        .unwrap()
    );
    // Nothing an agent is shown names the data root.
    assert!(
        !s.to_string().contains(&home.root().display().to_string()),
        "{s}"
    );
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
    let c = connect(
        &home,
        "acme",
        home.token("acme", &["read", "write", "post", "admin"]),
    )
    .await;
    set_targets(&c, None, Some(&folder)).await;
    post_one(&c, 3).await;
    drop(c);
    assert!(server.close().success());
    let live = live_head(&home, "acme");
    assert_eq!(proven(&home, &folder.join("acme.git"), "closed"), live);

    // archive_catches_up_at_start: a backup target named while the server
    // was down (here: the folder was missing then made) is caught up when
    // the books open.
    let late = home.outside("late");
    let server = Server::start(&home, Duration::from_secs(3600));
    let c = connect(
        &home,
        "acme",
        home.token("acme", &["read", "write", "post", "admin"]),
    )
    .await;
    set_targets(&c, None, Some(&late)).await;
    // The folder doesn't exist: said plainly, the books unharmed.
    let s = call(&c, "books.back_up_now", json!({})).await.unwrap();
    let err = s["targets"][0]["last_error"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(err.contains("folder"), "{s}");
    let start = call(&c, "books.start", json!({})).await.unwrap()["next_steps"].to_string();
    assert!(start.contains("behind or failing"), "{start}");
    assert!(!err.contains(&home.root().display().to_string()), "{err}");
    drop(c);
    assert!(server.close().success());
    // The disk is connected while the books are closed: the books catch
    // up when they open.
    std::fs::create_dir_all(&late).unwrap();
    let server = Server::start(&home, Duration::from_secs(3600));
    let c = connect(
        &home,
        "acme",
        home.token("acme", &["read", "write", "post", "admin"]),
    )
    .await;
    let s = backed_up(&c, Duration::from_secs(30)).await;
    assert_eq!(s["targets"][0]["last_error"], Value::Null, "{s}");
    assert_eq!(
        proven(&home, &late.join("acme.git"), "late"),
        live_head(&home, "acme")
    );
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
    let admin = connect(
        &home,
        "acme",
        home.token("acme", &["read", "write", "post", "admin"]),
    )
    .await;
    set_targets(&admin, None, Some(&folder)).await;
    post_one(&admin, 2).await;
    let before = head(&admin).await;
    // A reader sees the standing but can't back up.
    let reader = connect(&home, "acme", home.token("acme", &["read"])).await;
    assert!(call(&reader, "books.backup", json!({})).await.is_ok());
    let refused = call(&reader, "books.back_up_now", json!({}))
        .await
        .unwrap_err();
    assert!(refused.contains("admin"), "{refused}");
    // Twice: the second has nothing new to commit, and still answers.
    for _ in 0..2 {
        let s = call(&admin, "books.back_up_now", json!({})).await.unwrap();
        assert_eq!(s["targets"][0]["through_seq"].as_u64(), Some(before), "{s}");
        assert_eq!(s["targets"][0]["last_error"], Value::Null, "{s}");
    }
    assert_eq!(head(&admin).await, before);
    assert_eq!(
        proven(&home, &folder.join("acme.git"), "now"),
        live_head(&home, "acme")
    );
    drop((admin, reader));
    assert!(server.close().success());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_backup_that_moved_on_is_never_overwritten() {
    let home = Home::new("diverged");
    let remote = home.remote();
    let server = Server::start(&home, Duration::from_secs(3600));
    home.create("acme");
    home.create("other");
    // Another book's history in the remote first.
    let other = connect(
        &home,
        "other",
        home.token("other", &["read", "write", "post", "admin"]),
    )
    .await;
    set_targets(&other, Some(&remote), None).await;
    post_one(&other, 9).await;
    let s = call(&other, "books.back_up_now", json!({})).await.unwrap();
    assert_eq!(s["targets"][0]["last_error"], Value::Null, "{s}");
    let theirs = proven(&home, &remote, "theirs");
    // This book's push is refused, the remote unchanged.
    let acme = connect(
        &home,
        "acme",
        home.token("acme", &["read", "write", "post", "admin"]),
    )
    .await;
    set_targets(&acme, Some(&remote), None).await;
    post_one(&acme, 1).await;
    let s = call(&acme, "books.back_up_now", json!({})).await.unwrap();
    assert!(s["targets"][0]["last_error"].is_string(), "{s}");
    assert_eq!(s["targets"][0]["through_seq"], Value::Null, "{s}");
    assert_eq!(proven(&home, &remote, "after"), theirs);
    drop((other, acme));
    assert!(server.close().success());
}

/// Restore `source` as `id`: the preview, then (if it was one) the
/// restore confirmed against its head. Ok(what was restored) or Err(why).
fn restore(home: &Home, id: &str, source: &Path, replace: bool) -> Result<admin::Restored, String> {
    let request = |expect_head: Option<(u64, String)>| Request::Restore {
        id: id.into(),
        source: source.display().to_string(),
        replace,
        confirmed: expect_head.is_some(),
        expect_head,
    };
    let preview = match home.admin(request(None)) {
        Response::RestorePreview(p) => p,
        Response::Refused(why) => return Err(why),
        other => panic!("{other:?}"),
    };
    // (Whether books of that id exist is said; the restore itself refuses
    // over them, or replacing none.)
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

/// A git command in `dir` with an identity, for making damaged backups.
fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t.invalid",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "{args:?}: {out:?}");
}

/// A backup of acme (two entries, a receipt each) in the folder target.
async fn backed_up_acme(home: &Home) -> (PathBuf, Value) {
    settle().await;
    let folder = home.outside("disk");
    std::fs::create_dir_all(&folder).unwrap();
    let c = connect(
        home,
        "acme",
        home.token("acme", &["read", "write", "post", "admin"]),
    )
    .await;
    set_targets(&c, None, Some(&folder)).await;
    post_one(&c, 4).await;
    post_one(&c, 5).await;
    let s = call(&c, "books.back_up_now", json!({})).await.unwrap();
    assert_eq!(s["targets"][0]["last_error"], Value::Null, "{s}");
    (folder.join("acme.git"), figures(&c).await)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn restore_beside_existing_books_and_on_a_clean_install() {
    let home = Home::new("beside");
    let server = Server::start(&home, Duration::from_secs(3600));
    home.create("acme");
    let (backup, want) = backed_up_acme(&home).await;
    // Beside: its own books, the same figures and head.
    let r = restore(&home, "acme-copy", &backup, false).unwrap();
    assert_eq!(r.replaced, None);
    assert_eq!(r.head, live_head(&home, "acme"));
    let copy = connect(
        &home,
        "acme-copy",
        home.token("acme-copy", &["read", "admin"]),
    )
    .await;
    assert_eq!(figures(&copy).await, want);
    // Its own backups carry on from the restored history: to the folder
    // under its own name.
    let s = call(&copy, "books.back_up_now", json!({})).await.unwrap();
    assert_eq!(s["targets"][0]["last_error"], Value::Null, "{s}");
    let folder = backup.parent().unwrap();
    assert_eq!(
        proven(&home, &folder.join("acme-copy.git"), "copy"),
        live_head(&home, "acme-copy")
    );
    drop(copy);
    assert!(server.close().success());

    // restore_on_clean_install: another computer's root, nothing in it.
    let other = Home::new("clean");
    let server = Server::start(&other, Duration::from_secs(3600));
    assert_eq!(books_held(&other), Vec::<String>::new());
    let r = restore(&other, "acme", &backup, false).unwrap();
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
    let (backup, want) = backed_up_acme(&home).await;
    let at_backup = live_head(&home, "acme");
    // The books move on past the backup.
    let c = connect(
        &home,
        "acme",
        home.token("acme", &["read", "write", "post", "admin"]),
    )
    .await;
    post_one(&c, 6).await;
    let moved_on = live_head(&home, "acme");
    assert!(moved_on.0 > at_backup.0);
    // Not over them.
    let refused = restore(&home, "acme", &backup, false).unwrap_err();
    assert!(refused.contains("already books called"), "{refused}");
    assert_eq!(live_head(&home, "acme"), moved_on);
    // Replacing books that aren't there is refused too.
    let refused = restore(&home, "nobody", &backup, true).unwrap_err();
    assert!(refused.contains("no books called"), "{refused}");
    // Replacing: the old books kept whole in legacy/, the backup in place.
    let r = restore(&home, "acme", &backup, true).unwrap();
    // The restored books are new to the server: backed up as books just
    // opened, unasked.
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
    // The session of before never reaches the old books: refused, or
    // (its client opening a new session) the restored ones.
    if let Ok(start) = call(&c, "books.start", json!({})).await {
        assert_eq!(start["head"]["seq"].as_u64(), Some(at_backup.0), "{start}");
    }
    let c = connect(
        &home,
        "acme",
        home.token("acme", &["read", "write", "post", "admin"]),
    )
    .await;
    assert_eq!(figures(&c).await, want);
    // And it is written to and backed up as before (the backup's history
    // carries on: a fast-forward).
    post_one(&c, 7).await;
    let s = call(&c, "books.back_up_now", json!({})).await.unwrap();
    assert_eq!(s["targets"][0]["last_error"], Value::Null, "{s}");
    assert_eq!(proven(&home, &backup, "after"), live_head(&home, "acme"));
    drop(c);
    assert!(server.close().success());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn restore_refuses_a_damaged_backup_and_writes_nothing() {
    let home = Home::new("damaged");
    let server = Server::start(&home, Duration::from_secs(3600));
    home.create("acme");
    let (backup, _) = backed_up_acme(&home).await;
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
    // Each damage in a backup of its own, made from the good one.
    type Spoil<'a> = (&'a str, &'a dyn Fn(&Path));
    let damage: [Spoil; 4] = [
        ("flipped", &|w: &Path| {
            let j = w.join(ledger_store::JOURNAL_FILE);
            let text = std::fs::read_to_string(&j).unwrap();
            std::fs::write(&j, text.replacen("entry 4", "entry 9", 1)).unwrap();
            git(w, &["commit", "-qam", &git_subject(w)]);
        }),
        ("no-blob", &|w: &Path| {
            git(w, &["rm", "-rq", "blobs"]);
            git(w, &["commit", "-qm", &git_subject(w)]);
        }),
        ("cut", &|w: &Path| {
            let j = w.join(ledger_store::JOURNAL_FILE);
            let mut bytes = std::fs::read(&j).unwrap();
            bytes.truncate(bytes.len() - 10);
            std::fs::write(&j, bytes).unwrap();
            git(w, &["commit", "-qam", &git_subject(w)]);
        }),
        ("head", &|w: &Path| {
            git(
                w,
                &["commit", "-q", "--allow-empty", "-m", "seq 99 head 00"],
            );
        }),
    ];
    for (tag, spoil) in damage {
        let work = home.outside(&format!("work-{tag}"));
        let out = Command::new("git")
            .args(["clone", "-q"])
            .arg(&backup)
            .arg(&work)
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        spoil(&work);
        let bad = home.outside(&format!("bad-{tag}.git"));
        let out = Command::new("git")
            .args(["clone", "-q", "--bare"])
            .arg(&work)
            .arg(&bad)
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        let why = restore(&home, "acme-bad", &bad, false).unwrap_err();
        assert!(
            why.contains("can't be restored") || why.contains("can't be trusted"),
            "{tag}: {why}"
        );
        assert!(
            !why.contains(&home.root().display().to_string()),
            "{tag}: {why}"
        );
        assert!(!why.contains(&bad.display().to_string()), "{tag}: {why}");
        assert_eq!(books_held(&home), before, "{tag}");
        assert_eq!(names(), listed, "{tag}");
    }
    // An address that isn't a backup: said, nothing written.
    let why = restore(&home, "acme-bad", &home.outside("nothing-here"), false).unwrap_err();
    assert!(!why.contains("nothing-here"), "{why}");
    assert_eq!(names(), listed);
    assert!(server.close().success());
}

/// The subject a backup commit of `w`'s journal would have.
fn git_subject(w: &Path) -> String {
    let s = ledger_store::Snapshot::load(w);
    match s {
        Ok(s) => format!("seq {} head {}", s.journal_seq(), s.head_hash()),
        // A journal that no longer loads: the subject it had.
        Err(_) => {
            let out = Command::new("git")
                .arg("-C")
                .arg(w)
                .args(["log", "-1", "--format=%s"])
                .output()
                .unwrap();
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        }
    }
}

/// Wait until every target's backup has been drilled.
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
    let (backup, _) = backed_up_acme(&home).await;
    assert!(server.close().success());
    // Opening the books drills a backup never drilled.
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
    // Its backup was proven where it is: the record names the head.
    assert!(proven(&home, &backup, "drilled").0 > 0);
    drop(c);
    assert!(server.close().success());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn restore_drill_failure_is_flagged() {
    let home = Home::new("drill-bad");
    let server = Server::start(&home, Duration::from_secs(3600));
    home.create("acme");
    let (backup, _) = backed_up_acme(&home).await;
    assert!(server.close().success());
    // The backup is changed behind the books' back.
    let work = home.outside("tamper");
    let out = Command::new("git")
        .args(["clone", "-q"])
        .arg(&backup)
        .arg(&work)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let j = work.join(ledger_store::JOURNAL_FILE);
    let text = std::fs::read_to_string(&j).unwrap();
    std::fs::write(&j, text.replacen("entry 4", "entry 9", 1)).unwrap();
    git(&work, &["commit", "-qam", "seq 0 head x"]);
    git(&work, &["push", "-q", "origin", "HEAD:main"]);
    // Opening the books: nothing is pushed over it, and its first drill
    // fails, said in books.backup and books.start.
    let server = Server::start_with(
        &home,
        Duration::from_secs(3600),
        Some(Duration::from_millis(300)),
    );
    let c = connect(&home, "acme", home.token("acme", &["read", "admin"])).await;
    let s = drilled(&c).await;
    assert!(s["targets"][0]["drill_error"].is_string(), "{s}");
    assert!(s["targets"][0]["last_error"].is_string(), "{s}");
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
    let c = connect(
        &home,
        "acme",
        home.token("acme", &["read", "write", "post", "admin"]),
    )
    .await;
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
    let (backup, _) = backed_up_acme(&home).await;
    let request = |expect_head: Option<(u64, String)>, confirmed| Request::Restore {
        id: "copy".into(),
        source: backup.display().to_string(),
        replace: false,
        confirmed,
        expect_head,
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
    let c = connect(
        &home,
        "acme",
        home.token("acme", &["read", "write", "post", "admin"]),
    )
    .await;
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
    let c = connect(
        &home,
        "acme",
        home.token("acme", &["read", "write", "post", "admin"]),
    )
    .await;
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
    let (backup, want) = backed_up_acme(&home).await;
    drop(c);
    assert!(server.close().success());
    let other = Home::new("vault-clean");
    let server = Server::start(&other, Duration::from_secs(3600));
    let request = |expect_head: Option<(u64, String)>| Request::Restore {
        id: "acme".into(),
        source: backup.display().to_string(),
        replace: false,
        confirmed: expect_head.is_some(),
        expect_head,
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

    // A backup without the vault's key, or without a TIN record the books
    // name, is not whole.
    let third = Home::new("vault-short");
    let server = Server::start(&third, Duration::from_secs(3600));
    for (gone, said) in [("keys", "key"), ("vault", "TIN record")] {
        let work = home.outside(&format!("no-{gone}"));
        let out = Command::new("git")
            .args(["clone", "-q"])
            .arg(&backup)
            .arg(&work)
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        git(&work, &["rm", "-rq", gone]);
        git(&work, &["commit", "-qm", &git_subject(&work)]);
        let bad = home.outside(&format!("no-{gone}.git"));
        let out = Command::new("git")
            .args(["clone", "-q", "--bare"])
            .arg(&work)
            .arg(&bad)
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        let why = restore(&third, "acme", &bad, false).unwrap_err();
        assert!(why.contains(said), "{gone}: {why}");
        assert_eq!(books_held(&third), Vec::<String>::new());
    }
    assert!(server.close().success());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_replacing_restore_that_fails_leaves_the_books_open() {
    let home = Home::new("replace-bad");
    let server = Server::start(&home, Duration::from_secs(3600));
    home.create("acme");
    let (backup, want) = backed_up_acme(&home).await;
    let before = live_head(&home, "acme");
    // A backup whose head isn't the one it records.
    let work = home.outside("work");
    let out = Command::new("git")
        .args(["clone", "-q"])
        .arg(&backup)
        .arg(&work)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    git(
        &work,
        &["commit", "-q", "--allow-empty", "-m", "seq 99 head 00"],
    );
    let bad = home.outside("bad.git");
    let out = Command::new("git")
        .args(["clone", "-q", "--bare"])
        .arg(&work)
        .arg(&bad)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
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
    let c = connect(
        &home,
        "acme",
        home.token("acme", &["read", "write", "post", "admin"]),
    )
    .await;
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
async fn books_restored_beside_their_original_leave_its_repository_alone() {
    let home = Home::new("beside-remote");
    let server = Server::start(&home, Duration::from_secs(3600));
    home.create("acme");
    settle().await;
    let (remote, folder) = (home.remote(), home.outside("disk"));
    std::fs::create_dir_all(&folder).unwrap();
    let c = connect(
        &home,
        "acme",
        home.token("acme", &["read", "write", "post", "admin"]),
    )
    .await;
    set_targets(&c, Some(&remote), Some(&folder)).await;
    post_one(&c, 1).await;
    let s = call(&c, "books.back_up_now", json!({})).await.unwrap();
    assert!(
        s["targets"]
            .as_array()
            .unwrap()
            .iter()
            .all(|t| t["last_error"].is_null()),
        "{s}"
    );
    // A copy beside them, from their repository.
    let r = restore(&home, "copy", &remote, false).unwrap();
    assert_eq!(r.origin.as_deref(), Some("acme"));
    let copy = connect(
        &home,
        "copy",
        home.token("copy", &["read", "write", "post", "admin"]),
    )
    .await;
    post_one(&copy, 2).await;
    let s = call(&copy, "books.back_up_now", json!({})).await.unwrap();
    let by = |kind: &str| {
        s["targets"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["target"] == kind)
            .unwrap()
            .clone()
    };
    // Its own folder repository; the shared one paused, and said.
    assert_eq!(by("folder")["last_error"], Value::Null, "{s}");
    assert!(
        by("repository")["last_error"]
            .as_str()
            .unwrap()
            .contains("restored as a copy"),
        "{s}"
    );
    let start = call(&copy, "books.start", json!({})).await.unwrap()["next_steps"].to_string();
    assert!(start.contains("behind or failing"), "{start}");
    // The original carries on backing up there.
    post_one(&c, 3).await;
    let s = call(&c, "books.back_up_now", json!({})).await.unwrap();
    assert!(
        s["targets"]
            .as_array()
            .unwrap()
            .iter()
            .all(|t| t["last_error"].is_null()),
        "{s}"
    );
    assert_eq!(proven(&home, &remote, "orig"), live_head(&home, "acme"));
    // Naming a repository of its own ends the pause.
    let own = home.outside("own.git");
    let out = Command::new("git")
        .args(["init", "-q", "--bare", "-b", "main"])
        .arg(&own)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    set_targets(&copy, Some(&own), None).await;
    let s = call(&copy, "books.back_up_now", json!({})).await.unwrap();
    assert!(
        s["targets"]
            .as_array()
            .unwrap()
            .iter()
            .all(|t| t["last_error"].is_null()),
        "{s}"
    );
    drop((c, copy));
    assert!(server.close().success());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn restored_books_never_make_their_backup_folder_again() {
    let home = Home::new("restore-mount");
    let server = Server::start(&home, Duration::from_secs(3600));
    home.create("acme");
    settle().await;
    let (remote, folder) = (home.remote(), home.outside("disk"));
    std::fs::create_dir_all(&folder).unwrap();
    let c = connect(
        &home,
        "acme",
        home.token("acme", &["read", "write", "post", "admin"]),
    )
    .await;
    set_targets(&c, Some(&remote), Some(&folder)).await;
    post_one(&c, 1).await;
    call(&c, "books.back_up_now", json!({})).await.unwrap();
    drop(c);
    assert!(server.close().success());
    // On another computer, its backup disk not mounted (the mount point
    // there, empty): restored from the repository.
    std::fs::rename(folder.join("acme.git"), home.outside("unmounted.git")).unwrap();
    let other = Home::new("restore-mount-new");
    let server = Server::start(&other, Duration::from_secs(3600));
    restore(&other, "acme", &remote, false).unwrap();
    let c = connect(&other, "acme", other.token("acme", &["read", "admin"])).await;
    let s = call(&c, "books.back_up_now", json!({})).await.unwrap();
    let by = |kind: &str| {
        s["targets"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["target"] == kind)
            .unwrap()
            .clone()
    };
    assert!(
        by("folder")["last_error"]
            .as_str()
            .unwrap()
            .contains("holds no backup"),
        "{s}"
    );
    assert!(!folder.join("acme.git").exists(), "made on the wrong disk");
    assert_eq!(by("repository")["last_error"], Value::Null, "{s}");
    // The human says it's a new disk: begun there again.
    let s = call(&c, "books.back_up_now", json!({"new_disk": true}))
        .await
        .unwrap();
    assert!(
        s["targets"]
            .as_array()
            .unwrap()
            .iter()
            .all(|t| t["last_error"].is_null()),
        "{s}"
    );
    assert!(folder.join("acme.git").join("HEAD").is_file());
    drop(c);
    assert!(server.close().success());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_damaged_backup_record_never_keeps_the_books_shut() {
    let home = Home::new("damaged-record");
    let server = Server::start(&home, Duration::from_secs(3600));
    home.create("acme");
    backed_up_acme(&home).await;
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
    // What it knew of whose backups the folder holds was kept: gone, the
    // folder's backup is a disk not connected, never made again.
    drop(c);
    assert!(server.close().success());
    let folder = home.outside("disk");
    std::fs::rename(folder.join("acme.git"), home.outside("away.git")).unwrap();
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
    assert!(!folder.join("acme.git").exists());
    drop(c);
    assert!(server.close().success());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_failing_target_waits_while_the_others_carry_on() {
    let home = Home::new("one-failing");
    let server = Server::start(&home, Duration::from_millis(300));
    home.create("acme");
    settle().await;
    let remote = home.remote();
    let unplugged = home.outside("unplugged");
    let c = connect(
        &home,
        "acme",
        home.token("acme", &["read", "write", "post", "admin"]),
    )
    .await;
    set_targets(&c, Some(&remote), Some(&unplugged)).await;
    let s = call(&c, "books.back_up_now", json!({})).await.unwrap();
    let folder_fails = |home: &Home| {
        records(home, "acme")
            .iter()
            .filter(|r| r["kind"] == "push" && r["target"] == "folder")
            .count()
    };
    let before = folder_fails(&home);
    assert!(before >= 1, "{s}");
    // Writes: backed up to the repository soon after each, the folder not
    // tried again (it failed lately).
    for n in 0..3 {
        post_one(&c, 10 + n).await;
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let s = call(&c, "books.backup", json!({})).await.unwrap();
            let remote_t = s["targets"]
                .as_array()
                .unwrap()
                .iter()
                .find(|t| t["target"] == "repository")
                .unwrap()
                .clone();
            if remote_t["through_seq"] == s["head_seq"] {
                break;
            }
            assert!(Instant::now() < deadline, "not backed up: {s}");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    assert_eq!(folder_fails(&home), before);
    drop(c);
    assert!(server.close().success());
}
