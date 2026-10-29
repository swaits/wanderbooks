//! How every test here reaches books: the one way production serves them
//! (plan D-44). Each test file says `mod common;` and uses what it needs.
//!
//! ```text
//! GOAL: a test drives books exactly as an agent reaches them: a
//!       `wanderbooks server` on a data root of the test's own, spoken to
//!       over the root's MCP socket with a bearer token, with the headers
//!       the agents' bridge sends; a file goes in by its upload and comes
//!       back by its download, never through the session (D-46).
//! SOURCE: plan/served.md D-44 (one way to serve books), 3.1 (the
//!         server's sockets, the admin socket), 3.3 (tokens), 4.7
//!         (uploads and downloads); ledger-mcp local.rs INVARIANTS 8 and
//!         9; ledger-host bridge.rs (what it sends upstream).
//! INVARIANTS:
//!   1. Every home is a fresh temp dir (`wb-` prefixed, so a sweep of
//!      the test temp dir finds it), made by `wanderbooks setup --home`;
//!      its server runs with an empty environment but PATH, and HOME and
//!      XDG_CONFIG_HOME inside the home, on that home's root alone: never
//!      this account's root, tokens, or sockets.
//!   2. A server is ended by its own child's PID, never by a search.
//!   3. Settings a test gives as a settings file's text go into the books
//!      as an agent puts them there: the changes ledger-config's import
//!      reads from the text, made with books.set_setting.
//!   4. The harness answers nothing itself: every message a test is
//!      given is one the server sent.
//!   5. A file a test gives the books is uploaded as an agent uploads it:
//!      books.begin_upload, then its bytes PUT to the upload route with
//!      the session's token, as `wanderbooks upload` sends them; the
//!      upload_id is what the tool is given (D-46).
//! ```
#![allow(dead_code)]

use ledger_mcp::admin::{self, Request, Response};
use serde_json::{Value, json};
use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The id of the books [`Books`] makes.
pub const BOOK: &str = "acme";

/// How long one request may take before the server counts as hung: long
/// enough for a first answer while other test binaries contend for the
/// disk (every journal append is synced).
pub const TIMEOUT: Duration = Duration::from_secs(180);

/// The protocol version the tests speak.
pub const PROTOCOL: &str = "2025-06-18";

/// A home with a data root made by `wanderbooks setup` (INVARIANT 1).
pub struct Home {
    pub dir: tempfile::TempDir,
    /// A token for each book, made on first use (all scopes).
    tokens: Mutex<HashMap<String, String>>,
}

impl Home {
    pub fn new(tag: &str) -> Self {
        let dir = tempfile::Builder::new()
            .prefix(&format!("wb-{tag}-"))
            .tempdir()
            .unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_wanderbooks"))
            .args(["setup", "--home"])
            .arg(dir.path())
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", dir.path())
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        Self {
            dir,
            tokens: Default::default(),
        }
    }

    /// The root `setup --home` made: the platform's data folder under the
    /// home (`.local/share` on Linux, `Library/Application Support` on
    /// macOS), never a Linux path spelled out (the portable macOS job
    /// found the server refusing a root that was never set up).
    pub fn root(&self) -> PathBuf {
        ledger_store::root::default_root(self.dir.path())
    }

    pub fn run(&self) -> PathBuf {
        self.root().join("run")
    }

    /// The folder of the books `id` in the root.
    pub fn book_dir(&self, id: &str) -> PathBuf {
        self.root().join("books").join(id)
    }

    pub fn admin(&self, request: Request) -> Response {
        admin::call(&self.run().join("admin.sock"), request).unwrap()
    }

    /// The token tests use for `book`, made through the admin socket the
    /// first time (every scope).
    pub fn token(&self, book: &str) -> String {
        let mut tokens = self.tokens.lock().unwrap();
        if let Some(t) = tokens.get(book) {
            return t.clone();
        }
        let text = self.token_with(book, "test", &["read", "write", "post", "admin"]);
        tokens.insert(book.to_string(), text.clone());
        text
    }

    /// A new token named `name` for `book` with these scopes (replacing
    /// one of that name).
    pub fn token_with(&self, book: &str, name: &str, scopes: &[&str]) -> String {
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

    /// The text of every answer must not name the root (local.rs
    /// INVARIANT 7).
    pub fn assert_hidden(&self, text: &str) {
        let root = self.root().display().to_string();
        assert!(!text.contains(&root), "names the data root: {text}");
        assert!(
            !text.contains(&self.dir.path().display().to_string()),
            "names the home: {text}"
        );
    }

    pub fn create(&self, id: &str, currency: &str) {
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

    /// Adopt the old data dir `source` as the books `id` (plan 5.5), with
    /// its settings file's text: previewed, then confirmed at the head the
    /// preview read, as the owner's `wanderbooks books adopt` does.
    pub fn adopt(&self, id: &str, source: &Path, settings_toml: Option<&str>) {
        let request = |confirmed: bool, expect_head: Option<(u64, String)>| Request::Adopt {
            id: id.into(),
            source: source.to_path_buf(),
            settings_toml: settings_toml.map(str::to_string),
            config_json: None,
            vault_dir: None,
            vault_key: None,
            confirmed,
            expect_head,
            skip_unknown: true,
        };
        let head = match self.admin(request(false, None)) {
            Response::Preview(p) => p.source_head,
            other => panic!("adopt preview: {other:?}"),
        };
        match self.admin(request(true, Some(head))) {
            Response::Adopted(_) => {}
            other => panic!("adopt: {other:?}"),
        }
    }
}

/// A running `wanderbooks server`, ended (by its own PID) when dropped.
pub struct Server {
    pub child: Child,
}

impl Server {
    /// Start one on `home`'s root and wait for its ready line.
    pub fn start(home: &Home) -> Self {
        Self::start_in(home, home.dir.path(), &[])
    }

    /// [`Server::start`], in the folder `cwd`, with `env` beside the
    /// home's own (INVARIANT 1).
    pub fn start_in(home: &Home, cwd: &Path, env: &[(&str, &Path)]) -> Self {
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(home.dir.path().join("server.log"))
            .unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_wanderbooks"))
            .arg("server")
            .arg("--root")
            .arg(home.root())
            .current_dir(cwd)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", home.dir.path())
            .env("XDG_CONFIG_HOME", home.dir.path().join(".config"))
            .envs(env.iter().map(|(k, v)| (*k, v.as_os_str())))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(log)
            .spawn()
            .unwrap();
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        if line != "ready\n" {
            // Say why (the portable jobs run this on macOS and Windows,
            // where the only evidence is what the server wrote).
            let said =
                std::fs::read_to_string(home.dir.path().join("server.log")).unwrap_or_default();
            panic!("the server didn't say ready (got {line:?}); its log:\n{said}");
        }
        Self { child }
    }

    /// Close it as the host does (stdin ends) and wait for it to exit.
    pub fn close(mut self) -> std::process::ExitStatus {
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

/// An HTTP client for `home`'s MCP socket, with the socket's short path
/// (good while it lives).
pub fn mcp_http(home: &Home) -> reqwest::Client {
    mcp_http_builder(home).build().unwrap()
}

/// [`mcp_http`]'s builder.
pub fn mcp_http_builder(home: &Home) -> reqwest::ClientBuilder {
    mcp_http_builder_at(&home.run())
}

/// [`mcp_http_builder`], for the root's `run/` folder `run`.
pub fn mcp_http_builder_at(run: &Path) -> reqwest::ClientBuilder {
    // The short path is good in this process only, while it lives: kept.
    let short = ledger_store::root::SocketPath::new(run, "mcp.sock").unwrap();
    let path = short.path().to_path_buf();
    std::mem::forget(short);
    reqwest::Client::builder().unix_socket(path)
}

/// The settings a settings file's text gives, as books.set_setting changes
/// (INVARIANT 3): what ledger-config's import reads from it for books
/// that set nothing yet.
pub fn settings_changes(toml: &str, config_json: Option<&str>, currency: &str) -> Value {
    let import = ledger_config::codec::import(
        toml,
        config_json,
        &ledger_core::settings::Settings::default(),
        currency,
    )
    .unwrap_or_else(|e| panic!("the test's settings don't read: {e}\n{toml}"));
    assert!(
        import.unknown.is_empty(),
        "the test's settings hold entries the books don't know: {:?}",
        import.unknown
    );
    Value::Array(
        import
            .changes
            .iter()
            .map(|c| json!({"key": c.key, "value": c.value}))
            .collect(),
    )
}

/// What one HTTP exchange came back as, when it wasn't a JSON-RPC
/// message: refused at the door (a status and its text).
#[derive(Debug, Clone)]
pub struct Refusal {
    pub status: u16,
    pub body: String,
    /// It carried local.rs's REFUSED_HEADER (not run).
    pub not_run: bool,
}

/// One MCP session with one book, synchronous: what a test sends is
/// POSTed to `/b/{book}/mcp` as the bridge does, and every message the
/// server answers with is kept for [`Wire::recv`] (INVARIANT 4).
pub struct Wire {
    http: reqwest::Client,
    book: String,
    token: Option<String>,
    session: Option<String>,
    version: Option<String>,
    told: VecDeque<Value>,
    /// The last exchange that came back as no message at all.
    pub refused: Option<Refusal>,
    /// The HTTP status of the last exchange.
    pub status: u16,
    next_id: u64,
    /// Last: the client's connections are dropped inside it.
    rt: tokio::runtime::Runtime,
}

impl Wire {
    /// A session with `book` (not yet initialized), with its token.
    pub fn new(home: &Home, book: &str) -> Self {
        let token = home.token(book);
        Self::with_token(home, book, Some(token))
    }

    /// [`Wire::new`], with this token (or none).
    pub fn with_token(home: &Home, book: &str, token: Option<String>) -> Self {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let http = mcp_http_builder(home).timeout(TIMEOUT).build().unwrap();
        Self {
            rt,
            http,
            book: book.to_string(),
            token,
            session: None,
            version: None,
            told: VecDeque::new(),
            refused: None,
            status: 0,
            next_id: 1_000_000,
        }
    }

    /// The token this session sends.
    pub fn token(&self) -> Option<&str> {
        self.token.as_deref()
    }

    /// Initialize and send `notifications/initialized`: the initialize
    /// answer.
    pub fn initialize(&mut self, client: &str) -> Value {
        let init = self.rpc(
            "initialize",
            json!({"protocolVersion": PROTOCOL, "capabilities": {},
                   "clientInfo": {"name": client, "version": "0.1.0"}}),
        );
        assert!(init.get("result").is_some(), "initialize failed: {init}");
        self.send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        init
    }

    /// Send a request with an id of the wire's own and take its answer.
    pub fn rpc(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        let resp = self
            .recv(TIMEOUT)
            .unwrap_or_else(|| panic!("no answer to {method} (id {id}): {:?}", self.refused));
        assert_eq!(resp["id"], json!(id), "answer id mismatch: {resp}");
        resp
    }

    /// Call a tool: the `tools/call` answer. A call with `confirmed: true`
    /// and no `preview` is made the way an agent makes it (plan P4, D-50):
    /// the same call without `confirmed` first, then the confirm with the
    /// `preview` it returned. A preview that is refused is the answer
    /// (the refusal a confirm would have met); a tool that returns no
    /// preview is called as given. Use [`Self::call_raw`] to send exactly
    /// these arguments.
    pub fn call(&mut self, tool: &str, mut args: Value) -> Value {
        if args.get("confirmed") == Some(&json!(true)) && args.get("preview").is_none() {
            let mut first = args.clone();
            first["confirmed"] = json!(false);
            let seen = self.call_raw(tool, first);
            match answer_of(&seen) {
                Err(_) => return seen,
                Ok(out) => {
                    if let Some(digest) = out.get("preview").and_then(Value::as_str) {
                        args["preview"] = json!(digest);
                    }
                }
            }
        }
        self.call_raw(tool, args)
    }

    /// Call a tool with exactly these arguments.
    pub fn call_raw(&mut self, tool: &str, args: Value) -> Value {
        self.rpc("tools/call", json!({"name": tool, "arguments": args}))
    }

    /// Send one message.
    pub fn send(&mut self, message: &Value) {
        self.send_line(&message.to_string());
    }

    /// Send `text` as one message's body, as it is (a test's own bytes,
    /// malformed ones included).
    pub fn send_line(&mut self, text: &str) {
        let parsed: Option<Value> = serde_json::from_str(text).ok();
        let method = parsed
            .as_ref()
            .and_then(|m| m.get("method"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let mut req = self
            .http
            .post(format!("http://localhost/b/{}/mcp", self.book))
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .body(text.to_string());
        if let Some(t) = &self.token {
            req = req.bearer_auth(t);
        }
        if let Some(s) = &self.session {
            req = req.header("mcp-session-id", s);
        }
        if let Some(v) = &self.version {
            req = req.header("mcp-protocol-version", v);
        }
        let (status, headers, body) = self.rt.block_on(async {
            let r = req.send().await.expect("the server's socket answers");
            let status = r.status().as_u16();
            let headers = r.headers().clone();
            let body = r.text().await.unwrap_or_default();
            (status, headers, body)
        });
        if method.as_deref() == Some("initialize")
            && let Some(s) = headers.get("mcp-session-id").and_then(|v| v.to_str().ok())
        {
            self.session = Some(s.to_string());
        }
        let is_stream = headers
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|t| t.starts_with("text/event-stream"));
        // A message the server sends is JSON, or the test fails here: a
        // stream's data line (an empty one is the stream's priming), or a
        // body it says is JSON. Anything else is a refusal, kept below.
        let is_json = headers
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|t| t.starts_with("application/json"));
        let messages: Vec<Value> = if is_stream {
            body.lines()
                .filter_map(|l| l.strip_prefix("data:"))
                .map(str::trim)
                .filter(|d| !d.is_empty())
                .map(|d| {
                    serde_json::from_str(d)
                        .unwrap_or_else(|e| panic!("the server streamed no JSON ({e}): {d}"))
                })
                .collect()
        } else if is_json && !body.trim().is_empty() {
            let v: Value = serde_json::from_str(body.trim())
                .unwrap_or_else(|e| panic!("the server's JSON isn't ({e}): {body}"));
            [v].into_iter()
                .filter(|v| v.get("jsonrpc").is_some() || v.is_array())
                .collect()
        } else {
            Vec::new()
        };
        self.status = status;
        if messages.is_empty() && status != 202 {
            self.refused = Some(Refusal {
                status,
                body: body.clone(),
                not_run: headers.contains_key(ledger_mcp::local::REFUSED_HEADER),
            });
        } else {
            self.refused = None;
        }
        for m in messages {
            if method.as_deref() == Some("initialize")
                && let Some(v) = m.pointer("/result/protocolVersion").and_then(Value::as_str)
            {
                self.version = Some(v.to_string());
            }
            // Notifications the server sends on its own are not answers.
            if m.get("id").is_none() && m.get("method").is_some() {
                continue;
            }
            self.told.push_back(m);
        }
    }

    /// The next message the server answered with, if any. An answer
    /// comes back with its request, so nothing is waited for.
    pub fn recv(&mut self, _wait: Duration) -> Option<Value> {
        self.told.pop_front()
    }

    /// GET `path` (under `/b/{book}/`) with this session's token: the
    /// status and the bytes.
    pub fn get(&mut self, path: &str) -> (u16, Vec<u8>) {
        let mut req = self
            .http
            .get(format!("http://localhost/b/{}/{path}", self.book));
        if let Some(t) = &self.token {
            req = req.bearer_auth(t);
        }
        self.rt.block_on(async {
            let r = req.send().await.expect("the server's socket answers");
            let status = r.status().as_u16();
            (status, r.bytes().await.unwrap_or_default().to_vec())
        })
    }

    /// The bytes of a file the books handed over (a tool's HandedFile):
    /// fetched by its download with this session's token, and checked
    /// against its size and SHA-256.
    pub fn fetch(&mut self, file: &Value) -> Vec<u8> {
        let command = file["download"]
            .as_str()
            .unwrap_or_else(|| panic!("no download in {file}"));
        let id = command
            .split(' ')
            .find(|w| w.starts_with("ex_"))
            .unwrap_or_else(|| panic!("no export id in {command}"));
        let (status, bytes) = self.get(&format!("x/{id}"));
        assert_eq!(
            status,
            200,
            "fetching {id}: {}",
            String::from_utf8_lossy(&bytes)
        );
        assert_eq!(
            file["sha256"],
            json!(ledger_store::journal::sha256_hex(&bytes)),
            "{file}"
        );
        assert_eq!(file["size_bytes"], json!(bytes.len()), "{file}");
        bytes
    }

    /// Upload `bytes` as an agent does (INVARIANT 5): books.begin_upload
    /// with their SHA-256, size, type, name and purpose, then, if it began
    /// one, the bytes PUT to the upload route. books.begin_upload's answer:
    /// its upload_id, or the existing_attachment_id of the same bytes
    /// stored already (nothing sent then).
    pub fn begin_and_send(
        &mut self,
        bytes: &[u8],
        file_name: &str,
        mime_type: &str,
        purpose: &str,
    ) -> Value {
        let r = self.call(
            "books.begin_upload",
            upload_args(bytes, file_name, mime_type, purpose),
        );
        let begun = answer_of(&r).unwrap_or_else(|e| panic!("books.begin_upload: {e}"));
        if let Some(id) = begun["upload_id"].as_str() {
            let (status, said) = self.put(&format!("u/{id}"), bytes);
            assert_eq!(status, 200, "sending {id}: {said}");
        }
        begun
    }

    /// [`Wire::begin_and_send`] for bytes not stored yet: the upload_id to
    /// name in the tool that files them.
    pub fn upload(
        &mut self,
        bytes: &[u8],
        file_name: &str,
        mime_type: &str,
        purpose: &str,
    ) -> String {
        let begun = self.begin_and_send(bytes, file_name, mime_type, purpose);
        begun["upload_id"]
            .as_str()
            .unwrap_or_else(|| panic!("no upload begun: {begun}"))
            .to_string()
    }

    /// PUT `bytes` to `path` (under `/b/{book}/`) with this session's
    /// token, as `wanderbooks upload` does: the status and the text.
    pub fn put(&mut self, path: &str, bytes: &[u8]) -> (u16, String) {
        let mut req = self
            .http
            .put(format!("http://localhost/b/{}/{path}", self.book))
            .body(bytes.to_vec());
        if let Some(t) = &self.token {
            req = req.bearer_auth(t);
        }
        self.rt.block_on(async {
            let r = req.send().await.expect("the server's socket answers");
            let status = r.status().as_u16();
            (status, r.text().await.unwrap_or_default())
        })
    }

    /// [`Wire::fetch`], as text.
    pub fn fetch_text(&mut self, file: &Value) -> String {
        String::from_utf8(self.fetch(file)).expect("the file is text")
    }

    /// End the session (DELETE, as the bridge does when its agent goes).
    pub fn end(&mut self) {
        let Some(s) = self.session.take() else { return };
        let mut req = self
            .http
            .delete(format!("http://localhost/b/{}/mcp", self.book))
            .header("mcp-session-id", s);
        if let Some(t) = &self.token {
            req = req.bearer_auth(t);
        }
        let _ = self.rt.block_on(async { req.send().await });
    }
}

/// books.begin_upload's arguments for `bytes`.
pub fn upload_args(bytes: &[u8], file_name: &str, mime_type: &str, purpose: &str) -> Value {
    json!({"sha256": ledger_store::journal::sha256_hex(bytes), "size": bytes.len(),
           "mime_type": mime_type, "file_name": file_name, "purpose": purpose})
}

/// `tools/call` through `rpc` (a harness's own request function), the way
/// an agent confirms (plan P4, D-50): a call with `confirmed: true` and no
/// `preview` is made first without `confirmed`, then with the `preview` it
/// returned. A refused preview is the answer; a tool that returns no
/// preview is called as given.
pub fn tools_call(mut rpc: impl FnMut(&str, Value) -> Value, name: &str, mut args: Value) -> Value {
    if args.get("confirmed") == Some(&json!(true)) && args.get("preview").is_none() {
        let mut first = args.clone();
        first["confirmed"] = json!(false);
        let seen = rpc("tools/call", json!({"name": name, "arguments": first}));
        match answer_of(&seen) {
            Err(_) => return seen,
            Ok(out) => {
                if let Some(digest) = out.get("preview").and_then(Value::as_str) {
                    args["preview"] = json!(digest);
                }
            }
        }
    }
    rpc("tools/call", json!({"name": name, "arguments": args}))
}

/// A `tools/call` answer: Ok(its structured result) or Err(the refusal's
/// or the protocol error's text).
pub fn answer_of(r: &Value) -> Result<Value, String> {
    if let Some(e) = r.get("error") {
        return Err(e["message"].as_str().unwrap_or_default().to_string());
    }
    let res = &r["result"];
    let text = res["content"][0]["text"].as_str().unwrap_or_default();
    if res["isError"] == json!(true) {
        return Err(text.to_string());
    }
    Ok(res
        .get("structuredContent")
        .cloned()
        .unwrap_or_else(|| serde_json::from_str(text).unwrap_or(Value::String(text.into()))))
}

/// A test's own books: a home, its server, and the books [`BOOK`] made in
/// it, with the settings the test gives.
pub struct Books {
    pub home: Home,
    server: std::cell::RefCell<Option<Server>>,
    pub id: String,
    pub currency: String,
}

impl Books {
    /// New books in a new home, kept in USD, with no settings set.
    pub fn new(tag: &str) -> Self {
        Self::with(tag, "USD", None)
    }

    /// New books in `currency`, with the settings `settings_toml` (a
    /// settings file's text) gives, set as an agent sets them.
    pub fn with(tag: &str, currency: &str, settings_toml: Option<&str>) -> Self {
        let home = Home::new(tag);
        let server = Server::start(&home);
        home.create(BOOK, currency);
        let books = Self {
            home,
            server: std::cell::RefCell::new(Some(server)),
            id: BOOK.to_string(),
            currency: currency.to_string(),
        };
        if let Some(toml) = settings_toml {
            books.settle(toml);
        }
        books
    }

    /// Books an earlier wanderbooks left in the data root: the files of
    /// `fixture` (a book's folder: `journal.jsonl` and what else it holds)
    /// put in the root before the server opens them, private as the server
    /// keeps a book.
    pub fn placed(tag: &str, fixture: &Path) -> Self {
        let home = Home::new(tag);
        copy_private(fixture, &home.book_dir(BOOK));
        let server = Server::start(&home);
        Self {
            home,
            server: std::cell::RefCell::new(Some(server)),
            id: BOOK.to_string(),
            currency: "USD".to_string(),
        }
    }

    /// Books adopted from the old data dir `source`, with its settings
    /// file's text (plan 5.5).
    pub fn adopted(tag: &str, source: &Path, settings_toml: Option<&str>) -> Self {
        let home = Home::new(tag);
        let server = Server::start(&home);
        home.adopt(BOOK, source, settings_toml);
        Self {
            home,
            server: std::cell::RefCell::new(Some(server)),
            id: BOOK.to_string(),
            currency: "USD".to_string(),
        }
    }

    /// The books' folder in the root.
    pub fn dir(&self) -> PathBuf {
        self.home.book_dir(&self.id)
    }

    /// The books' journal, as it is on disk.
    pub fn journal(&self) -> Vec<u8> {
        std::fs::read(self.dir().join("journal.jsonl")).unwrap_or_default()
    }

    /// A new session with the books (not yet initialized).
    pub fn wire(&self) -> Wire {
        Wire::new(&self.home, &self.id)
    }

    /// A new, initialized session with the books.
    pub fn session(&self, client: &str) -> Wire {
        let mut w = self.wire();
        w.initialize(client);
        w
    }

    /// Set the settings a settings file's text gives (INVARIANT 3).
    pub fn settle(&self, toml: &str) {
        self.settle_with(toml, None);
    }

    /// [`Books::settle`], with a receipt policy file's text too (its
    /// `receipt_policy` wins over the settings file's receipt settings).
    pub fn settle_with(&self, toml: &str, config_json: Option<&str>) {
        if let Err(e) = self.try_settle(toml, config_json) {
            panic!("the test's settings were refused: {e}\n{toml}");
        }
    }

    /// [`Books::settle_with`]: Err with the refusal's text.
    pub fn try_settle(&self, toml: &str, config_json: Option<&str>) -> Result<(), String> {
        settle_through(&self.home, &self.id, &self.currency, toml, config_json)
    }

    /// Close the server (as the host does) and wait for it to exit.
    pub fn stop(&self) {
        if let Some(s) = self.server.borrow_mut().take() {
            assert!(s.close().success(), "the server didn't close cleanly");
        }
    }

    /// Start the server again (after [`Books::stop`]).
    pub fn start(&self) {
        let mut server = self.server.borrow_mut();
        assert!(server.is_none(), "the server is running");
        *server = Some(Server::start(&self.home));
    }

    /// Close the server and start it again on the same root.
    pub fn restart(&self) {
        self.stop();
        self.start();
    }

    /// The running server's process id.
    pub fn server_pid(&self) -> Option<u32> {
        self.server.borrow().as_ref().map(|s| s.child.id())
    }

    /// The server log (its stderr).
    pub fn server_log(&self) -> String {
        std::fs::read_to_string(self.home.dir.path().join("server.log")).unwrap_or_default()
    }
}

/// Every file under `dir`, by path relative to it, with the SHA-256 of its
/// contents, leaving out what a call writes by design whatever the call:
/// the access log (every call is recorded), the viewer's side channel
/// (`ops/`), the books' own backup (`archive.git/`), the files handed over
/// and on their way in (`exports/`, `uploads/`), and SQLite's -wal and
/// -shm files (it moves pages between them on its own).
pub fn book_files(dir: &Path) -> std::collections::BTreeMap<PathBuf, String> {
    fn walk(base: &Path, dir: &Path, out: &mut std::collections::BTreeMap<PathBuf, String>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd.flatten() {
            let p = e.path();
            let rel = p.strip_prefix(base).unwrap().to_path_buf();
            let name = rel.to_string_lossy().into_owned();
            let first = rel
                .components()
                .next()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .unwrap_or_default();
            if ["ops", "archive.git", "exports", "uploads"].contains(&first.as_str())
                || name == "access.jsonl"
                || name.ends_with("-wal")
                || name.ends_with("-shm")
            {
                continue;
            }
            if p.is_dir() {
                walk(base, &p, out);
            } else if let Ok(bytes) = std::fs::read(&p) {
                out.insert(rel, ledger_store::journal::sha256_hex(&bytes));
            }
        }
    }
    let mut out = std::collections::BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

/// Copy the tree `from` into `to`, private as the server keeps a book
/// (folders 0700, files 0600).
pub fn copy_private(from: &Path, to: &Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(to).unwrap();
    std::fs::set_permissions(to, std::fs::Permissions::from_mode(0o700)).unwrap();
    for entry in std::fs::read_dir(from).unwrap().flatten() {
        let (src, dst) = (entry.path(), to.join(entry.file_name()));
        if entry.file_type().unwrap().is_dir() {
            copy_private(&src, &dst);
        } else {
            std::fs::copy(&src, &dst).unwrap();
            std::fs::set_permissions(&dst, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
    }
}

/// A test's place: a home (whose folder holds the test's own files, its
/// settings file among them) and the books [`BOOK`] in its root, served
/// by a server [`Place::start`] starts, and starts again on each next
/// call, as a new server process reads the same books afresh. A test may
/// write books into [`Place::books_data`] before the first start (books
/// an earlier version left in the root, a journal written by hand); the
/// server opens them as it opens any books in its root.
pub struct Place {
    pub home: Home,
    server: std::cell::RefCell<Option<Server>>,
    /// The settings file's text set in the books, once set.
    settled: std::cell::RefCell<Option<String>>,
    /// The currency books made by the first start are kept in.
    pub currency: String,
    /// The folder the server runs in, when not the home.
    cwd: std::cell::RefCell<Option<PathBuf>>,
}

impl std::ops::Deref for Place {
    type Target = Path;
    fn deref(&self) -> &Path {
        self.home.dir.path()
    }
}

impl AsRef<Path> for Place {
    fn as_ref(&self) -> &Path {
        self.home.dir.path()
    }
}

impl Place {
    pub fn new(tag: &str) -> Self {
        Self {
            home: Home::new(tag),
            server: Default::default(),
            settled: Default::default(),
            currency: "USD".to_string(),
            cwd: Default::default(),
        }
    }

    /// Run the server in `cwd` from its next start (where a relative path
    /// would land).
    pub fn run_in(&self, cwd: &Path) {
        *self.cwd.borrow_mut() = Some(cwd.to_path_buf());
    }

    /// The books' folder in the root (where a test may write books before
    /// the first start).
    pub fn books_data(&self) -> PathBuf {
        self.home.book_dir(BOOK)
    }

    /// Start the server (closing the one running first): books made if
    /// the root holds none yet, the settings `settings_toml` gives set the
    /// first time; and a new session with them (not yet initialized).
    pub fn start(&self, settings_toml: Option<&str>) -> Wire {
        self.stop();
        let dir = self.books_data();
        let fresh = !dir.join("journal.jsonl").exists();
        if dir.exists() {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let cwd = self.cwd.borrow().clone();
        let cwd = cwd.as_deref().unwrap_or(self.home.dir.path());
        *self.server.borrow_mut() = Some(Server::start_in(&self.home, cwd, &[]));
        if fresh {
            self.home.create(BOOK, &self.currency);
        }
        if let Some(toml) = settings_toml {
            let mut settled = self.settled.borrow_mut();
            match settled.as_deref() {
                Some(was) => assert_eq!(
                    was, toml,
                    "the test changed its settings file between starts: books keep their settings, change them with books.set_setting"
                ),
                None => {
                    settle_through(&self.home, BOOK, &self.currency, toml, None).unwrap_or_else(
                        |e| panic!("the test's settings were refused: {e}\n{toml}"),
                    );
                    *settled = Some(toml.to_string());
                }
            }
        }
        Wire::new(&self.home, BOOK)
    }

    /// Close the server, if one runs, and wait for it to exit.
    pub fn stop(&self) {
        if let Some(s) = self.server.borrow_mut().take() {
            assert!(s.close().success(), "the server didn't close cleanly");
        }
    }

    /// The running server's process id.
    pub fn server_pid(&self) -> Option<u32> {
        self.server.borrow().as_ref().map(|s| s.child.id())
    }
}

/// Set the settings `toml` (and `config_json`) give in `book`, through a
/// session of its own (INVARIANT 3): Err with the refusal.
pub fn settle_through(
    home: &Home,
    book: &str,
    currency: &str,
    toml: &str,
    config_json: Option<&str>,
) -> Result<(), String> {
    let changes = settings_changes(toml, config_json, currency);
    if changes.as_array().is_some_and(Vec::is_empty) {
        return Ok(());
    }
    let mut w = Wire::new(home, book);
    w.initialize("settings");
    let r = w.call(
        "books.set_setting",
        json!({"changes": changes, "decided_by": "Test Owner"}),
    );
    w.end();
    if r["result"]["isError"] == json!(true) || r.get("error").is_some() {
        return Err(r.to_string());
    }
    Ok(())
}

/// An rmcp client session with a book, for async tests.
pub type Client =
    rmcp::service::RunningService<rmcp::service::RoleClient, rmcp::model::ClientConfig>;

/// An HTTP client for `home`'s MCP socket that sends `book`'s token on
/// every request.
pub fn mcp_http_for(home: &Home, book: &str) -> reqwest::Client {
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

/// An initialized rmcp client session with `book`, with its token.
pub async fn connect(home: &Home, book: &str) -> Result<Client, String> {
    let token = home.token(book);
    connect_with(home, book, Some(token)).await
}

/// [`connect`], with this token (or none).
pub async fn connect_with(
    home: &Home,
    book: &str,
    token: Option<String>,
) -> Result<Client, String> {
    connect_at(&home.run(), book, token).await
}

/// [`connect_with`], to the server whose `run/` folder is `run` (for a
/// task that holds no [`Home`]).
pub async fn connect_at(run: &Path, book: &str, token: Option<String>) -> Result<Client, String> {
    use rmcp::ServiceExt;
    use rmcp::transport::StreamableHttpClientTransport;
    use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
    let http = mcp_http_builder_at(run).build().unwrap();
    let mut config =
        StreamableHttpClientTransportConfig::with_uri(format!("http://localhost/b/{book}/mcp"));
    if let Some(token) = token {
        config = config.auth_header(token);
    }
    let transport = StreamableHttpClientTransport::with_client(http, config);
    rmcp::model::ClientConfig::default()
        .serve(transport)
        .await
        .map_err(|e| e.to_string())
}

/// Call a tool: Ok(structured result) or Err(the error text).
/// Call a tool the way an agent confirms (plan P4, D-50): a call with
/// `confirmed: true` and no `preview` is made first without `confirmed`,
/// then with the `preview` it returned. A refused preview is the answer.
pub async fn call(c: &Client, tool: &str, mut args: Value) -> Result<Value, String> {
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
    use rmcp::model::CallToolRequestParams;
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

/// Upload `bytes` for `book` on the rmcp session `c` (to the server whose
/// `run/` folder is `run`) as an agent does (INVARIANT 5), with `token`
/// (the session's), declared as `(file_name, mime_type, purpose)`: the
/// upload_id.
pub async fn upload_on(
    c: &Client,
    run: &Path,
    book: &str,
    token: &str,
    bytes: &[u8],
    (file_name, mime_type, purpose): (&str, &str, &str),
) -> String {
    let begun = call(
        c,
        "books.begin_upload",
        upload_args(bytes, file_name, mime_type, purpose),
    )
    .await
    .unwrap_or_else(|e| panic!("books.begin_upload: {e}"));
    let id = begun["upload_id"]
        .as_str()
        .unwrap_or_else(|| panic!("no upload begun: {begun}"))
        .to_string();
    let r = mcp_http_builder_at(run)
        .build()
        .unwrap()
        .put(format!("http://localhost/b/{book}/u/{id}"))
        .bearer_auth(token)
        .body(bytes.to_vec())
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 200, "sending {id}");
    id
}

/// Fetch the export a tool handed over in `file` (its `download`
/// command's id) from `book` with `token`: its bytes, checked against its
/// size and SHA-256.
pub async fn fetch_export(home: &Home, book: &str, token: &str, file: &Value) -> Vec<u8> {
    let command = file["download"]
        .as_str()
        .unwrap_or_else(|| panic!("{file}"));
    let id = command
        .split(' ')
        .find(|w| w.starts_with("ex_"))
        .unwrap_or_else(|| panic!("{command}"));
    let r = mcp_http(home)
        .get(format!("http://localhost/b/{book}/x/{id}"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 200, "{:?}", r.headers());
    let bytes = r.bytes().await.unwrap().to_vec();
    assert_eq!(
        file["sha256"],
        json!(ledger_store::journal::sha256_hex(&bytes))
    );
    assert_eq!(file["size_bytes"], json!(bytes.len()));
    bytes
}
