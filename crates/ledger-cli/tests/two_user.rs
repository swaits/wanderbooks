//! The two-account self-host deployment of the books (plan 8.2 P9a,
//! D-51): the server runs as one account on the owner's own machine and
//! an agent runs as another, reaching the books over HTTP with a static
//! bearer token.
//!
//! ```text
//! GOAL: two things the plan promises are checked, not assumed: an agent
//!       with its own account cannot read the data root, and it reaches
//!       every tool through the server's HTTP door all the same; and the
//!       books moved to that machine verify to the head they had.
//! SOURCE: plan 8.2 P9a (Build items 1 and 2; Done when 1), 2 I-1
//!         (isolation: advisory locally, enforced on a machine of the
//!         owner's), 3.3 (the static bearer), 3.8 (local vs self-host),
//!         3.9 (the server's sockets, and `--http`), D-51 (P9 split),
//!         D-31 (a backup clone is the move's transport).
//! INVARIANTS:
//!   1. The two-account tests run only against a deployment the caller
//!      made (the job in `.github/workflows/two-user.yml`, or the recipe
//!      in `deploy/README.md`); with no deployment they say they skipped,
//!      so a laptop's `cargo test` stays green. `WB_TWO_USER=1` turns
//!      them on, and then every other setting is required.
//!   2. The test never reads the books' files in the tenant: it proves it
//!      cannot (INVARIANT 3), then asks the server for everything.
//!   3. The tenant root is unreadable to this account: `opendir` and a
//!      journal `open` are refused.
//!   4. The books say `isolation: "service_user (enforced)"`: the server
//!      was told `--isolation enforced` by the unit or the job, never
//!      guessed.
//!   5. A read, a write, an upload and an export go over the tunnel's
//!      HTTP door with the bearer; the local Unix socket is not used.
//!      They go to a second book (`WB_WRITE_BOOK`), so the moved books'
//!      head is never advanced by a check.
//!   6. The moved books' head (seq and hash) and every event are the
//!      source's: the move loses nothing.
//! ```

use common::{BOOK, Client, Home, Wire};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

mod common;

// ---------------------------------------------------------------- harness

/// The deployment the caller made, from the environment
/// (INVARIANTS 1 to 3). `None` when `WB_TWO_USER` is unset.
struct Deployment {
    /// The data root this account must not be able to read.
    root: PathBuf,
    /// `http://host:port`, the tunnel's door's base.
    server: String,
    book: String,
    token: String,
    /// A second book the write checks go to, so the moved books' head is
    /// never advanced by a check (defaults to `book`).
    write_book: String,
    write_token: String,
    /// The backup clone the books were moved from, readable here.
    source: Option<PathBuf>,
}

fn deployment() -> Option<Deployment> {
    std::env::var_os("WB_TWO_USER")?;
    let need = |name: &str| -> String {
        std::env::var(name).unwrap_or_else(|_| panic!("WB_TWO_USER is set, so {name} must be too"))
    };
    let book = need("WB_BOOK");
    let token = need("WB_TOKEN");
    let write_book = std::env::var("WB_WRITE_BOOK").unwrap_or_else(|_| book.clone());
    let write_token = std::env::var("WB_WRITE_TOKEN").unwrap_or_else(|_| token.clone());
    Some(Deployment {
        root: PathBuf::from(need("WB_TENANT_ROOT")),
        server: need("WB_SERVER_URL"),
        book,
        token,
        write_book,
        write_token,
        source: std::env::var_os("WB_SOURCE_DIR").map(PathBuf::from),
    })
}

impl Deployment {
    /// The MCP URL of `book`.
    fn mcp_url(&self, book: &str) -> String {
        format!("{}/b/{}/mcp", self.server, book)
    }

    /// A URL under `/b/{book}/`.
    fn book_url(&self, book: &str, rest: &str) -> String {
        format!("{}/b/{}/{}", self.server, book, rest)
    }
}

/// A client of the books over the tunnel's HTTP door (INVARIANT 5), with
/// the static bearer token.
async fn connect_over_http(d: &Deployment, book: &str, token: &str) -> Client {
    use rmcp::ServiceExt;
    use rmcp::transport::StreamableHttpClientTransport;
    use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
    let config = StreamableHttpClientTransportConfig::with_uri(d.mcp_url(book))
        .auth_header(token.to_string());
    let http = reqwest::Client::new();
    let transport = StreamableHttpClientTransport::with_client(http, config);
    rmcp::model::ClientConfig::default()
        .serve(transport)
        .await
        .unwrap_or_else(|e| panic!("the tunnel's door refused the session: {e}"))
}

/// GET `url` with the bearer: the status and the bytes.
async fn get_with_token(url: &str, token: &str) -> (u16, Vec<u8>) {
    let r = reqwest::Client::new()
        .get(url)
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    let status = r.status().as_u16();
    (status, r.bytes().await.unwrap_or_default().to_vec())
}

/// A running `wanderbooks server` on `home`'s root with `args` beside the
/// root, ended (by its own PID) when dropped. The harness's `Server` is
/// the same without extra flags; this test needs `--isolation` and
/// `--http`.
struct FlaggedServer {
    child: Child,
}

impl FlaggedServer {
    fn start(home: &Home, args: &[String]) -> Self {
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(home.dir.path().join("server.log"))
            .unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_wanderbooks"))
            .arg("server")
            .arg("--root")
            .arg(home.root())
            .args(args)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", home.dir.path())
            .env("XDG_CONFIG_HOME", home.dir.path().join(".config"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(log)
            .spawn()
            .unwrap();
        let mut line = String::new();
        std::io::BufRead::read_line(
            &mut std::io::BufReader::new(child.stdout.as_mut().unwrap()),
            &mut line,
        )
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

impl Drop for FlaggedServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Run the server with `args` that should be refused before it serves
/// anything: its status, stdout and stderr.
fn refused_server(home: &Home, args: &[String]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_wanderbooks"))
        .arg("server")
        .arg("--root")
        .arg(home.root())
        .args(args)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", home.dir.path())
        .env("XDG_CONFIG_HOME", home.dir.path().join(".config"))
        .output()
        .unwrap()
}

/// A port nothing is listening on now, for the tunnel's door.
fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

/// `books.start` over a wire, as the tools' answer.
fn start_of(wire: &mut Wire) -> Value {
    let r = wire.call("books.start", json!({}));
    common::answer_of(&r).unwrap_or_else(|e| panic!("books.start: {e}"))
}

// ---------------------------------------------------------------- the tests

/// The tunnel's door is loopback only (plan 3.9, P9a): the connection is
/// plaintext with a static bearer, so any other address is refused before
/// anything binds or a host starts, never warned about or fallen back
/// from. A loopback address still starts and answers with the bearer.
#[test]
fn the_tunnel_door_binds_loopback_only() {
    let home = Home::new("tunnel-loopback");
    for addr in ["0.0.0.0:0", "192.0.2.1:0", "[::]:0"] {
        let out = refused_server(&home, &["--http".to_string(), addr.to_string()]);
        assert!(!out.status.success(), "--http {addr} was accepted");
        let said = String::from_utf8_lossy(&out.stderr);
        assert!(
            said.contains("loopback only"),
            "--http {addr} was refused for the wrong reason: {said}"
        );
        assert!(
            !String::from_utf8_lossy(&out.stdout).contains("ready"),
            "--http {addr} said ready"
        );
        // Nothing was left half-open: no sockets, no lock holder.
        for sock in ["mcp.sock", "admin.sock", "lease.sock"] {
            assert!(
                !home.run().join(sock).exists(),
                "--http {addr} left {sock} behind"
            );
        }
    }

    // The same refusal through `serve`, before it starts a host at all.
    let out = Command::new(env!("CARGO_BIN_EXE_wanderbooks"))
        .args(["serve", "--root"])
        .arg(home.root())
        .args(["--http", "0.0.0.0:0"])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", home.dir.path())
        .env("XDG_CONFIG_HOME", home.dir.path().join(".config"))
        .output()
        .unwrap();
    assert!(!out.status.success(), "serve accepted a network address");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("loopback only"),
        "serve: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!home.run().join("lease.sock").exists(), "a host started");

    // A loopback address starts, and the bearer reaches the books.
    let port = free_port();
    let server = FlaggedServer::start(&home, &["--http".to_string(), format!("127.0.0.1:{port}")]);
    home.create(BOOK, "USD");
    let d = Deployment {
        root: home.root(),
        server: format!("http://127.0.0.1:{port}"),
        book: BOOK.into(),
        token: home.token(BOOK),
        write_book: BOOK.into(),
        write_token: home.token(BOOK),
        source: None,
    };
    let rt = tokio::runtime::Runtime::new().unwrap();
    let start = rt.block_on(async {
        let client = connect_over_http(&d, &d.book, &d.token).await;
        let answer = common::call(&client, "books.start", json!({})).await;
        let _ = client.cancel().await;
        answer
    });
    let start = start.expect("books.start over the loopback door");
    assert_eq!(start["isolation"], "same_user (advisory)");
    assert!(server.close().success());
}

/// The server told `--isolation enforced` says so (`books.start`), over
/// its Unix socket and over the HTTP door, and a server not told says the
/// local truth: the server never guesses (plan 3.8, P9a).
#[test]
fn isolation_label_is_enforced_under_the_unit() {
    // The local default first: no flag, the advisory truth (D-32).
    let local = Home::new("isolation-local");
    let server = FlaggedServer::start(&local, &[]);
    local.create(BOOK, "USD");
    {
        let mut wire = Wire::new(&local, BOOK);
        wire.initialize("self-host-check");
        assert_eq!(start_of(&mut wire)["isolation"], "same_user (advisory)");
    }
    assert!(server.close().success());

    // Told enforced: `books.start` says so, on both doors (INVARIANT 4).
    let home = Home::new("isolation-enforced");
    let port = free_port();
    let server = FlaggedServer::start(
        &home,
        &[
            "--isolation".into(),
            "enforced".into(),
            "--http".into(),
            format!("127.0.0.1:{port}"),
        ],
    );
    home.create(BOOK, "USD");
    let over_socket = {
        let mut wire = Wire::new(&home, BOOK);
        wire.initialize("self-host-check");
        start_of(&mut wire)
    };
    assert_eq!(over_socket["isolation"], "service_user (enforced)");
    // The HTTP door is the same service with the same bearer token.
    let d = Deployment {
        root: home.root(),
        server: format!("http://127.0.0.1:{port}"),
        book: BOOK.into(),
        token: home.token(BOOK),
        write_book: BOOK.into(),
        write_token: home.token(BOOK),
        source: None,
    };
    let rt = tokio::runtime::Runtime::new().unwrap();
    let over_http = rt.block_on(async {
        let client = connect_over_http(&d, &d.book, &d.token).await;
        let answer = common::call(&client, "books.start", json!({})).await;
        let _ = client.cancel().await;
        answer
    });
    let over_http = over_http.expect("books.start over HTTP");
    assert_eq!(over_http["isolation"], "service_user (enforced)");
    assert_eq!(over_http["book"]["name"], over_socket["book"]["name"]);
    assert_eq!(over_http["head"], over_socket["head"]);
    assert!(server.close().success());

    // The unit's own line is what starts a server that way (P9a step 1):
    // it passes the flag this test exercises, and the tunnel's door.
    let unit = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../deploy/wanderbooks-server.service"
    ))
    .expect("deploy/wanderbooks-server.service");
    for line in [
        "User=wanderbooks",
        "--isolation enforced",
        "--http 127.0.0.1:7411",
        "IPAddressDeny=any",
        "IPAddressAllow=localhost",
    ] {
        assert!(unit.contains(line), "the unit lacks {line:?}:\n{unit}");
    }
}

/// An agent with its own account cannot read the books' data root, and
/// every tool it needs still answers over the tunnel's HTTP door with the
/// bearer (INVARIANTS 2, 3, 4 and 5).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_uid_cannot_read_tenant() {
    let Some(d) = deployment() else {
        eprintln!(
            "SKIPPED agent_uid_cannot_read_tenant: no two-account deployment (WB_TWO_USER unset; `.github/workflows/two-user.yml` or deploy/README.md makes one)"
        );
        return;
    };
    // INVARIANT 3: the root, and a journal inside it, are unreadable.
    match std::fs::read_dir(&d.root) {
        Err(e) => assert_eq!(
            e.kind(),
            std::io::ErrorKind::PermissionDenied,
            "the tenant root is not refused for the wrong reason: {e}"
        ),
        Ok(_) => panic!(
            "this account can read the tenant root ({}): the deployment did not make it private",
            d.root.display()
        ),
    }
    let journal = d.root.join("books").join(&d.book).join("journal.jsonl");
    match std::fs::File::open(&journal) {
        Err(e) => assert_eq!(
            e.kind(),
            std::io::ErrorKind::PermissionDenied,
            "the journal is not refused for the wrong reason: {e}"
        ),
        Ok(_) => panic!("this account can read the books' journal"),
    }

    // INVARIANT 4: the books were told enforced, and say so.
    let books = connect_over_http(&d, &d.book, &d.token).await;
    let start = common::call(&books, "books.start", json!({}))
        .await
        .expect("books.start over the tunnel");
    assert_eq!(start["isolation"], "service_user (enforced)");
    assert!(
        !start.to_string().contains(&d.root.display().to_string()),
        "the answer names the data root"
    );

    // The write checks go to a second book, so the moved books' head is
    // never advanced by a check (`self_host_adopt_same_head` reads it).
    let client = connect_over_http(&d, &d.write_book, &d.write_token).await;
    let write_start = common::call(&client, "books.start", json!({}))
        .await
        .expect("books.start over the tunnel");
    assert_eq!(write_start["isolation"], "service_user (enforced)");

    // A read.
    let chart = common::call(&client, "books.chart", json!({}))
        .await
        .expect("books.chart over the tunnel");
    assert!(chart["accounts"].is_array(), "{chart}");

    // A write: an account added over the tunnel, then read back.
    let code = "1999";
    let added = common::call(
        &client,
        "books.add_account",
        json!({"code": code, "name": "Over the tunnel", "account_type": "asset"}),
    )
    .await
    .expect("books.add_account over the tunnel");
    assert!(added["account_id"].is_number(), "{added}");
    let chart = common::call(&client, "books.chart", json!({}))
        .await
        .expect("books.chart over the tunnel");
    assert!(
        chart["accounts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["code"] == code),
        "the account written over the tunnel is not in the chart"
    );

    // A file in: an upload through the tunnel's own route.
    let bytes = b"a receipt filed over the tunnel";
    let begun = common::call(
        &client,
        "books.begin_upload",
        common::upload_args(bytes, "tunnel.txt", "text/plain", "receipt"),
    )
    .await
    .expect("books.begin_upload over the tunnel");
    let upload_id = begun["upload_id"]
        .as_str()
        .unwrap_or_else(|| panic!("no upload begun: {begun}"));
    let sent = reqwest::Client::new()
        .put(d.book_url(&d.write_book, &format!("u/{upload_id}")))
        .bearer_auth(&d.write_token)
        .body(bytes.to_vec())
        .send()
        .await
        .unwrap();
    assert_eq!(sent.status().as_u16(), 200, "the upload route refused it");
    // And file it, so it is a stored, named attachment the books verify.
    let filed = common::call(
        &client,
        "books.file_document",
        json!({"category": "other", "upload_id": upload_id,
               "file_name": "tunnel.txt", "mime_type": "text/plain",
               "description": "filed over the tunnel"}),
    )
    .await
    .expect("books.file_document over the tunnel");
    assert!(filed["attachment_id"].is_number(), "{filed}");

    // A file out: an export handed over, fetched through the tunnel.
    let exported = common::call(
        &client,
        "books.export",
        json!({"format": "csv", "report": "trial_balance"}),
    )
    .await
    .expect("books.export over the tunnel");
    let file = &exported["file"];
    let command = file["download"].as_str().expect("a download command");
    let id = command
        .split(' ')
        .find(|w| w.starts_with("ex_"))
        .unwrap_or_else(|| panic!("no export id in {command}"));
    let (status, fetched) = get_with_token(
        &d.book_url(&d.write_book, &format!("x/{id}")),
        &d.write_token,
    )
    .await;
    assert_eq!(status, 200, "the export route refused it");
    assert_eq!(file["size_bytes"], json!(fetched.len()));
    assert_eq!(
        file["sha256"],
        json!(ledger_store::journal::sha256_hex(&fetched))
    );

    // And the books verify, stored files included: the upload landed.
    let verified = common::call(&client, "books.verify", json!({"blobs": true}))
        .await
        .expect("books.verify over the tunnel");
    assert_eq!(verified["status"], "ok");
    assert!(verified["blobs_verified"].as_u64().unwrap_or(0) >= 1);

    let _ = client.cancel().await;
    let _ = books.cancel().await;
}

/// The books moved to the server verify to the head the source clone had:
/// the same events, the same seq and hash (INVARIANTS 2 and 6).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn self_host_adopt_same_head() {
    let Some(d) = deployment() else {
        eprintln!(
            "SKIPPED self_host_adopt_same_head: no two-account deployment (WB_TWO_USER unset)"
        );
        return;
    };
    let Some(source) = d.source.as_deref() else {
        eprintln!("SKIPPED self_host_adopt_same_head: WB_SOURCE_DIR is not set");
        return;
    };
    let (seq, hash, events) = source_head(source);
    let client = connect_over_http(&d, &d.book, &d.token).await;
    let start = common::call(&client, "books.start", json!({}))
        .await
        .expect("books.start over the tunnel");
    assert_eq!(start["head"]["seq"], seq, "the moved books' seq");
    assert_eq!(start["head"]["hash"], hash, "the moved books' head hash");
    let verified = common::call(&client, "books.verify", json!({}))
        .await
        .expect("books.verify over the tunnel");
    assert_eq!(verified["status"], "ok");
    assert_eq!(verified["events"], events, "every event moved");
    assert_eq!(verified["head"], hash, "the verified head");
    let _ = client.cancel().await;
}

/// The source clone's head and event count, read as the move's transport
/// is read (plan D-31): the journal's last line's seq and canonical hash.
fn source_head(source: &Path) -> (u64, String, u64) {
    let lines = ledger_store::journal::read_and_verify_journal(&source.join("journal.jsonl"))
        .unwrap_or_else(|e| panic!("the source clone doesn't verify: {e}"));
    let seq = lines.last().map_or(0, |l| l.seq);
    let hash = lines.last().map_or_else(
        || ledger_store::journal::GENESIS_PREV.to_string(),
        |l| ledger_store::journal::sha256_hex(l.to_canonical().as_bytes()),
    );
    (seq, hash, lines.len() as u64)
}
