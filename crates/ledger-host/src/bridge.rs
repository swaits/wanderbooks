//! `wanderbooks mcp --book ID`: the agents' bridge (plan 3.9, D-23).
//!
//! ```text
//! GOAL: an agent started in any folder reaches one book over stdio; the
//!       books open by themselves; a server restart goes unnoticed; and
//!       no call the agent made can ever run twice.
//! SOURCE: plan/served.md 3.9 ("The bridge and slow starts", "Closing",
//!         "Restarting"), 3.1 (the bridge never reads the books; it has
//!         no data-dir flag); the plan's tests bridge_opens_closed_books,
//!         bridge_survives_server_restart, bridge_initializes_during_cold_start,
//!         no_local_agent_tcp, close_now_ends_every_lease (in
//!         ledger-cli's open_while_held.rs, under names of their own).
//! INVARIANTS:
//!   1. It holds a lease for the whole session, and talks only to the
//!      root's lease and MCP sockets: never TCP, never the books' files.
//!   2. `initialize` is answered within knobs.init_wait: with the
//!      server's own answer when it is ready by then, else with
//!      opening.rs's; the server is initialized behind it as soon as it
//!      is ready, with the agent's own initialize.
//!   3. A call waits at most knobs.queue_wait for the books to open, then
//!      fails with a message naming `wanderbooks doctor`.
//!   4. A call is sent again only when it is known not to have run: the
//!      connection was refused before it was sent, the server answered
//!      that it is closing and didn't run it, or the session was unknown
//!      (after a restart). A call cut off after it was sent is
//!      answered with an error saying it may have run; never sent again.
//!   5. After a new upstream session (a restart), the agent is sent
//!      tools/list_changed only if the tool list differs from the last
//!      one it was given.
//!   6. Once the owner closes the books, every call is answered with
//!      that until the session ends; the bridge never reopens them.
//!   7. Every request from the agent gets exactly one answer, unless the
//!      agent cancelled it; one the server answered with nothing is
//!      reported as possibly run. Every initialize result it is given
//!      advertises tools.listChanged (5).
//!   8. A call the agent cancels before it was sent is never sent.
//!   9. The server's socket must belong to this account; and the agent's
//!      offers the bridge can't carry back (sampling, elicitation, roots)
//!      are not passed on, in initialize or in a request's own metadata.
//!  10. Ended by its stdin or by a signal, the bridge ends its upstream
//!      session.
//!  11. Every request upstream carries its token, read from the keyring
//!      (tokens.rs) before each new session (even with one held: one the
//!      keyring no longer gives is said on stderr, and the keyring's used)
//!      and before each request until it is found, and again when the
//!      door refuses it (the call then sent once more if another was
//!      found), so a token made or replaced mid-session is used from the
//!      next call. Without one, calls are answered with the server's
//!      refusal, which says how the human makes one; but when the keyring
//!      couldn't be read (locked, unreachable, or holding several items
//!      for the token), with that instead: the token is not said to be
//!      missing. The token is never written to the agent or to stderr.
//! ```

use crate::knobs::{FLAG, Knobs};
use crate::lease::{HostMsg, Kind, Reason};
use crate::opener::Opener;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
use tokio::sync::{Notify, mpsc, watch};
use tokio::time::Instant;

/// The longest message the agent may send (a file bigger than a call
/// takes inline goes by an upload).
const MAX_MESSAGE: usize = ledger_mcp::local::MAX_BODY;
/// The bridge's own request ids upstream start so.
const OWN_ID: &str = "wanderbooks-bridge-";
/// JSON-RPC: a server error.
const SERVER_ERROR: i64 = -32000;
/// What the server says when it is closing and didn't run a request.
const CLOSING: &str = "the books are closing";
/// The refusal code the server's door gives a request it didn't run
/// because the books are closing.
const REFUSED_CLOSING: &str = "closing";
/// What rmcp says, as the whole body, for an unknown session.
const NO_SESSION: &str = "Not Found: Session not found";

/// What a call cut off after it was sent is answered with (INVARIANT 4).
const MAYBE_RAN: &str = "the connection to the books server broke while this call was running, so it may or may not have been done: check before trying it again (books.digest shows the latest changes)";

/// What the agent is told once the owner closed the books (INVARIANT 6).
const CLOSED_BY_OWNER: &str = "the owner closed the books (Close the books, or `wanderbooks close`): nothing more can be done in this session. Start a new agent session to open them again.";

/// One upstream session with the server.
#[derive(Clone, Debug, Default)]
struct Upstream {
    /// The server's session id, if it gave one.
    session: Option<String>,
    /// The protocol version it negotiated.
    version: Option<String>,
    /// Its initialize result (only when the agent initialized).
    init: Option<Value>,
    /// Which session this is (1 for the first).
    epoch: u64,
}

/// Where the bridge stands with the server.
#[derive(Clone, Debug)]
enum Link {
    /// The books are opening (or reopening).
    Opening,
    /// The server is ready, with this session.
    Ready(Arc<Upstream>),
    /// The owner closed the books (INVARIANT 6).
    Closed,
    /// The books can't be opened, with why.
    Failed(String),
    /// The server refused these books (none of that name, or they didn't
    /// open), with why: tried again at the next call.
    Refused(String),
}

/// How a request sent upstream came out.
enum Sent {
    /// Answered (none, one or several messages for the agent).
    Answered(Vec<Value>),
    /// Not run, and safe to send again (INVARIANT 4): why.
    NotRun(NotRun),
    /// Sent and cut off: it may have run.
    MaybeRan,
    /// Refused by the server, with why, for the agent.
    Refused(String),
}

/// Why a request didn't run.
#[derive(Debug, PartialEq, Eq)]
enum NotRun {
    /// No server answering.
    Unreachable,
    /// The server is closing.
    Closing,
    /// The session isn't known (a restart, or idle).
    NoSession,
}

struct Bridge {
    book: String,
    /// The name of the token it uses (tokens.rs).
    token_name: String,
    /// The token's text, once read from the keyring (INVARIANT 11).
    token: Mutex<Option<String>>,
    /// Where it is kept: its account, and the root's `run/`.
    keyring: std::sync::OnceLock<(crate::tokens::Account, PathBuf)>,
    /// Why the keyring last couldn't be read, if it couldn't (INVARIANT
    /// 11).
    keyring_trouble: Mutex<Option<crate::tokens::KeyringError>>,
    http: reqwest::Client,
    knobs: Knobs,
    to_agent: mpsc::UnboundedSender<Value>,
    link: watch::Sender<Link>,
    /// The agent's initialize request's params, once it sent one.
    agent_init: Mutex<Option<Value>>,
    /// Woken when the agent initializes, or a session is found gone.
    renew: Notify,
    /// The last tools/list result the agent was given (INVARIANT 5).
    tools_seen: Mutex<Option<Value>>,
    next_id: std::sync::atomic::AtomicU64,
    /// The agent's requests not yet answered, by id (INVARIANT 8).
    pending: Mutex<std::collections::HashMap<String, Pending>>,
    /// The server's socket, for its owner check (INVARIANT 9).
    socket: PathBuf,
}

/// Who has the server's socket, when it isn't plainly this account.
enum Owner {
    /// Another account: never used.
    Other(String),
    /// Nobody answering: a failure, tried again.
    Unreachable,
}

/// How long opening a session, or asking for the tool list, may take
/// before it counts as not answering.
const UPSTREAM_WAIT: Duration = Duration::from_secs(10);

/// How long the keyring is waited for before a new session when a token
/// is already held (INVARIANT 11): the one held is used if it is slow.
const KEYRING_RECHECK_WAIT: Duration = Duration::from_secs(2);

/// Where one of the agent's requests stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pending {
    /// Waiting for the books.
    Waiting,
    /// Sent upstream: a cancel goes there too, and is remembered in case
    /// the call turns out not to have run and is to be sent again.
    Sent {
        /// The agent cancelled it meanwhile.
        cancelled: bool,
    },
    /// Cancelled before it was sent: never sent (INVARIANT 8).
    Cancelled,
}

/// The protocol version from which requests carry their method and name
/// in headers (SEP-2243), and per-request metadata.
const HEADERS_FROM: &str = "2026-07-28";

/// The per-request protocol version key in `_meta` (2026-07-28).
const META_VERSION: &str = "io.modelcontextprotocol/protocolVersion";

/// A header value, wrapped in base64 when it couldn't travel as it is.
fn header_value(value: &str) -> String {
    let plain = !value.is_empty()
        && value.bytes().all(|b| (0x20..0x7f).contains(&b))
        && !value.starts_with([' ', '\t'])
        && !value.ends_with([' ', '\t'])
        && !value.starts_with("=?base64?");
    if plain {
        value.to_string()
    } else {
        use base64::Engine;
        format!(
            "=?base64?{}?=",
            base64::engine::general_purpose::STANDARD.encode(value)
        )
    }
}

/// The headers a message needs beyond the session's: its protocol version
/// when it names one, and from 2026-07-28 its method and name.
fn message_headers(message: &Value, session_version: Option<&str>) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    let params = message.get("params");
    let version = params
        .and_then(|p| p.get("_meta"))
        .and_then(|m| m.get(META_VERSION))
        .and_then(Value::as_str)
        .or(session_version);
    let Some(version) = version else { return out };
    out.push(("mcp-protocol-version", version.to_string()));
    if version >= HEADERS_FROM
        && let Some(method) = message.get("method").and_then(Value::as_str)
    {
        out.push(("mcp-method", method.to_string()));
        let key = match method {
            "tools/call" | "prompts/get" => Some("name"),
            "resources/read" | "resources/subscribe" | "resources/unsubscribe" => Some("uri"),
            "tasks/get" | "tasks/update" | "tasks/cancel" => Some("taskId"),
            _ => None,
        };
        if let Some(name) = key
            .and_then(|k| params.and_then(|p| p.get(k)))
            .and_then(Value::as_str)
        {
            out.push(("mcp-name", header_value(name)));
        }
    }
    out
}

/// The agent's initialize, without the offers the bridge can't carry
/// back (INVARIANT 9).
fn without_client_offers(mut params: Value) -> Value {
    if let Some(caps) = params
        .get_mut("capabilities")
        .and_then(Value::as_object_mut)
    {
        for offer in ["sampling", "elicitation", "roots"] {
            caps.remove(offer);
        }
    }
    params
}

/// A request's own `_meta` client capabilities (2026-07-28), without the
/// offers the bridge can't carry back (INVARIANT 9).
fn without_meta_offers(mut message: Value) -> Value {
    if let Some(caps) = message
        .pointer_mut("/params/_meta/io.modelcontextprotocol~1clientCapabilities")
        .and_then(Value::as_object_mut)
    {
        for offer in ["sampling", "elicitation", "roots"] {
            caps.remove(offer);
        }
    }
    message
}

/// A request id as a map key.
fn id_key(id: &Value) -> String {
    id.to_string()
}

/// A JSON-RPC error answer.
fn error(id: &Value, message: impl Into<String>) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": SERVER_ERROR, "message": message.into()}})
}

/// Mark an initialize result as one whose tool list may change
/// (INVARIANT 7).
fn with_list_changed(mut result: Value) -> Value {
    if let Some(caps) = result
        .get_mut("capabilities")
        .and_then(Value::as_object_mut)
    {
        let tools = caps.entry("tools").or_insert_with(|| json!({}));
        if let Some(tools) = tools.as_object_mut() {
            tools.insert("listChanged".into(), Value::Bool(true));
        }
    }
    result
}

/// The messages in a text/event-stream body.
fn sse_messages(body: &str) -> Vec<Value> {
    let mut out = Vec::new();
    let mut data = String::new();
    let flush = |data: &mut String, out: &mut Vec<Value>| {
        if !data.is_empty() {
            if let Ok(v) = serde_json::from_str::<Value>(data) {
                out.push(v);
            }
            data.clear();
        }
    };
    for line in body.lines() {
        if line.is_empty() {
            flush(&mut data, &mut out);
        } else if let Some(rest) = line.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(rest.strip_prefix(' ').unwrap_or(rest));
        }
    }
    flush(&mut data, &mut out);
    out
}

impl Bridge {
    fn url(&self) -> String {
        format!("http://localhost/b/{}/mcp", self.book)
    }

    /// Read the token from the keyring if it isn't known yet (INVARIANT
    /// 11): a token made after the session began is found on the next
    /// try. A keyring that fails is said on stderr; the server's refusal
    /// then tells the agent what to do.
    async fn find_token(&self) {
        if self
            .token
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_none()
        {
            self.read_token().await;
        }
    }

    /// Whether a token is held.
    fn holds_token(&self) -> bool {
        self.token
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
    }

    /// Read the token from the keyring, keeping it if found: whether a
    /// token other than the one held was found (one made since, or one
    /// that replaced it).
    async fn read_token(&self) -> bool {
        self.read_token_within(self.knobs.queue_wait).await
    }

    /// [`Bridge::read_token`], waiting at most `wait` for the keyring.
    async fn read_token_within(&self, wait: Duration) -> bool {
        let Some((account, run_dir)) = self.keyring.get().cloned() else {
            return false;
        };
        let keeper = crate::tokens::keeper(&run_dir, &self.knobs);
        let read = tokio::time::timeout(
            wait,
            tokio::task::spawn_blocking(move || keeper.get(&account)),
        )
        .await;
        let read = match read {
            Ok(Ok(read)) => read,
            Ok(Err(_)) | Err(_) => Err(crate::tokens::KeyringError::Unavailable(
                "it didn't give the token in time: is it locked?".into(),
            )),
        };
        let trouble = read.as_ref().err().cloned();
        *self
            .keyring_trouble
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = trouble;
        match read {
            Ok(Some(token)) => {
                let mut held = self.token.lock().unwrap_or_else(|e| e.into_inner());
                let new = held.as_deref() != Some(token.as_str());
                if new && held.is_some() {
                    eprintln!(
                        "wanderbooks mcp: the keyring gives another token for these books than the one held: using the keyring's"
                    );
                }
                *held = Some(token);
                new
            }
            Ok(None) => false,
            Err(why) => {
                eprintln!("wanderbooks mcp: the token for these books couldn't be read: {why}");
                false
            }
        }
    }

    /// What the agent is told instead of the door's "make a token" when
    /// no token is held because the keyring couldn't be read (INVARIANT
    /// 11).
    fn keyring_refusal(&self) -> Option<String> {
        if self.holds_token() {
            return None;
        }
        let trouble = self
            .keyring_trouble
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()?;
        Some(match trouble {
            crate::tokens::KeyringError::Unavailable(why) => format!(
                "this computer's keyring is locked or unreachable, so the token for the books {} couldn't be read ({why}). The token itself is fine: unlock the keyring (or log in again) and try once more; don't make a new one (`wanderbooks token check --book {}` says where it stands).",
                self.book, self.book
            ),
            duplicates => duplicates.to_string(),
        })
    }

    /// A request upstream, carrying the token (INVARIANT 11).
    fn upstream(&self, method: reqwest::Method) -> reqwest::RequestBuilder {
        let req = self.http.request(method, self.url());
        match self
            .token
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_deref()
        {
            Some(token) => req.bearer_auth(token),
            None => req,
        }
    }

    fn own_id(&self) -> Value {
        let n = self
            .next_id
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Value::String(format!("{OWN_ID}{n}"))
    }

    fn tell(&self, message: Value) {
        let _ = self.to_agent.send(message);
    }

    /// Send one message upstream on `up`'s session.
    async fn send(&self, message: &Value, up: &Upstream) -> Sent {
        // A client of the 2026-07-28 era opens no session: its token is
        // looked for here (INVARIANT 11).
        self.find_token().await;
        let (sent, unauthorized) = self.send_once(message, up).await;
        // Refused at the door for its token (not run): one made or
        // replaced since is read from the keyring (INVARIANT 11). A
        // session belongs to the token that opened it, so with a session
        // the call waits for a new one (opened with the new token); with
        // none it is sent once more at once.
        if unauthorized && self.read_token().await {
            if up.session.is_some() {
                return Sent::NotRun(NotRun::NoSession);
            }
            return self.send_once(message, up).await.0;
        }
        // No token, for a keyring that couldn't be read: that is said,
        // not "make one" (INVARIANT 11).
        if unauthorized && let Some(why) = self.keyring_refusal() {
            return Sent::Refused(why);
        }
        sent
    }

    /// [`Bridge::send`], once: and whether the door refused its token.
    async fn send_once(&self, message: &Value, up: &Upstream) -> (Sent, bool) {
        let mut req = self
            .upstream(reqwest::Method::POST)
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .body(serde_json::to_vec(message).unwrap_or_default());
        if let Some(s) = &up.session {
            req = req.header("mcp-session-id", s);
        }
        for (name, value) in message_headers(message, up.version.as_deref()) {
            req = req.header(name, value);
        }
        let response = match req.send().await {
            Ok(r) => r,
            Err(e) if e.is_connect() => return (Sent::NotRun(NotRun::Unreachable), false),
            Err(_) => return (Sent::MaybeRan, false),
        };
        let status = response.status();
        let unauthorized = status == reqwest::StatusCode::UNAUTHORIZED;
        // Refused before it could run (books that didn't open, or none
        // so named): the server's words, for the agent (INVARIANT 4).
        let sent = self.read_answer(response).await;
        (sent, unauthorized)
    }

    /// What an answer from upstream says.
    async fn read_answer(&self, response: reqwest::Response) -> Sent {
        let status = response.status();
        let refused = response
            .headers()
            .get(ledger_mcp::local::REFUSED_HEADER)
            .map(|v| v.as_bytes().to_vec());
        // Refused at the door because the books are closing: not run, so
        // sent again once they're open (INVARIANT 4).
        if refused.as_deref() == Some(REFUSED_CLOSING.as_bytes()) {
            return Sent::NotRun(NotRun::Closing);
        }
        let refused = refused.is_some();
        let is_stream = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|t| t.starts_with("text/event-stream"));
        let body = match response.text().await {
            Ok(b) => b,
            Err(_) if refused => return Sent::Refused("the books server refused the call".into()),
            Err(_) => return Sent::MaybeRan,
        };
        if refused {
            return Sent::Refused(body.trim().chars().take(500).collect());
        }
        let answered = |messages: Vec<Value>| {
            // The server's "closing: not run" (INVARIANT 4).
            let not_run = messages.iter().any(|m| {
                m.pointer("/error/code").and_then(Value::as_i64)
                    == Some(i64::from(ledger_mcp::NOT_RUN_CLOSING_CODE))
            });
            if not_run {
                Sent::NotRun(NotRun::Closing)
            } else {
                Sent::Answered(messages)
            }
        };
        match status.as_u16() {
            202 => Sent::Answered(Vec::new()),
            200 if is_stream => answered(sse_messages(&body)),
            200 => match serde_json::from_str::<Value>(&body) {
                Ok(v) => answered(vec![v]),
                Err(_) => Sent::MaybeRan,
            },
            // Only rmcp's own words: a JSON-RPC error that merely echoes
            // them (a method so named) is an answer, passed on below.
            404 if body.trim() == NO_SESSION => Sent::NotRun(NotRun::NoSession),
            503 if body.starts_with(CLOSING) => Sent::NotRun(NotRun::Closing),
            _ => {
                // A JSON-RPC error rides in some error statuses: pass it on.
                if let Ok(v) = serde_json::from_str::<Value>(&body)
                    && v.get("jsonrpc").is_some()
                {
                    return answered(vec![v]);
                }
                // A server error with no answer in it: whether the call
                // ran can't be told (INVARIANT 4).
                if status.is_server_error() {
                    return Sent::MaybeRan;
                }
                Sent::Refused(body.trim().chars().take(500).collect())
            }
        }
    }

    /// Open a session with the server using the agent's initialize: the
    /// session, or `Err(Some(why))` when the server refuses these books,
    /// `Err(None)` when it isn't answering yet.
    async fn initialize_upstream(
        &self,
        params: &Value,
        epoch: u64,
    ) -> Result<Upstream, Option<String>> {
        // Read again before every session, even with a token held
        // (INVARIANT 11): a held one the keyring no longer gives isn't
        // kept on in silence. A slow keyring leaves the one held in use.
        if self.holds_token() {
            self.read_token_within(KEYRING_RECHECK_WAIT.min(self.knobs.queue_wait))
                .await;
        } else {
            self.read_token().await;
        }
        let request = json!({"jsonrpc": "2.0", "id": self.own_id(), "method": "initialize", "params": params});
        let post = || {
            self.upstream(reqwest::Method::POST)
                .header("content-type", "application/json")
                .header("accept", "application/json, text/event-stream")
                .body(request.to_string())
                .send()
        };
        let mut response = post().await.map_err(|_| None)?;
        // Refused for its token: once more with one made or replaced
        // since (INVARIANT 11); with none, for a keyring that couldn't be
        // read, that is said instead of the door's words.
        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            if self.read_token().await {
                response = post().await.map_err(|_| None)?;
            } else if let Some(why) = self.keyring_refusal() {
                return Err(Some(why));
            }
        }
        let session = response
            .headers()
            .get("mcp-session-id")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let status = response.status();
        let is_stream = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|t| t.starts_with("text/event-stream"));
        let body = response.text().await.map_err(|_| None)?;
        match status.as_u16() {
            200 => {}
            // Refused (no such books, books that didn't open, or an
            // initialize the server won't take): the server's words. Not
            // asked again until the next call.
            404 | 503 if !body.starts_with(CLOSING) => {
                return Err(Some(body.trim().chars().take(500).collect()));
            }
            400..=499 => return Err(Some(body.trim().chars().take(500).collect())),
            _ => return Err(None),
        }
        let messages = if is_stream {
            sse_messages(&body)
        } else {
            serde_json::from_str::<Value>(&body).into_iter().collect()
        };
        let result = messages
            .into_iter()
            .find_map(|m| m.get("result").cloned())
            .ok_or(None)?;
        let up = Upstream {
            session,
            version: result
                .get("protocolVersion")
                .and_then(Value::as_str)
                .map(str::to_string),
            init: Some(with_list_changed(result)),
            epoch,
        };
        // The bridge says initialized once per session; the agent's own
        // is not sent on.
        let _ = self
            .send(
                &json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
                &up,
            )
            .await;
        Ok(up)
    }

    /// The first page of the server's tool list on `up`.
    async fn tool_list(&self, up: &Upstream) -> Option<Value> {
        let request =
            json!({"jsonrpc": "2.0", "id": self.own_id(), "method": "tools/list", "params": {}});
        match tokio::time::timeout(UPSTREAM_WAIT, self.send(&request, up)).await {
            Ok(Sent::Answered(messages)) => {
                messages.into_iter().find_map(|m| m.get("result").cloned())
            }
            _ => None,
        }
    }

    /// Wait until the link is ready (with a session, if the agent
    /// initialized) or ended, until `deadline`.
    async fn ready_link(&self, deadline: Instant) -> Option<Link> {
        let mut rx = self.link.subscribe();
        let wants_session = self
            .agent_init
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some();
        let good = |link: &Link| match link {
            Link::Ready(up) => !wants_session || up.init.is_some(),
            Link::Closed | Link::Failed(_) | Link::Refused(_) => true,
            Link::Opening => false,
        };
        let waited = tokio::time::timeout_at(deadline, rx.wait_for(good)).await;
        match waited {
            Ok(Ok(link)) => Some(link.clone()),
            _ => None,
        }
    }

    /// Answer one request from the agent (INVARIANTS 3, 4, 6, 7).
    async fn request(self: Arc<Self>, message: Value) {
        let id = message.get("id").cloned().unwrap_or(Value::Null);
        let method = message
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if method == "initialize" {
            return self.initialize(message, id).await;
        }
        // A ping is the bridge's to answer while the books aren't open.
        if method == "ping" && !matches!(&*self.link.borrow(), Link::Ready(_)) {
            return self.answer(&id, json!({"jsonrpc": "2.0", "id": id, "result": {}}));
        }
        self.clone().request_on(message, id, &method).await;
    }

    /// The request `id` is done: no longer waiting, so its id is free
    /// again as soon as its answer goes out (and never before).
    fn forget(&self, id: &Value) {
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&id_key(id));
    }

    /// Give the request `id` its answer.
    fn answer(&self, id: &Value, message: Value) {
        self.forget(id);
        self.tell(message);
    }

    /// A request is waiting from the moment it is read (INVARIANT 8): a
    /// cancel that follows it at once finds it.
    fn waiting(&self, id: &Value) {
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id_key(id), Pending::Waiting);
    }

    /// A request sent that didn't run waits again; one cancelled while it
    /// was out is not sent again (INVARIANT 8).
    fn requeue(&self, id: &Value) {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(p) = pending.get_mut(&id_key(id)) {
            *p = match *p {
                Pending::Sent { cancelled: true } | Pending::Cancelled => Pending::Cancelled,
                _ => Pending::Waiting,
            };
        }
    }

    /// Mark a request sent, unless the agent cancelled it (INVARIANT 8).
    fn may_send(&self, id: &Value) -> bool {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        match pending.get_mut(&id_key(id)) {
            Some(Pending::Cancelled) => false,
            Some(p) => {
                *p = Pending::Sent { cancelled: false };
                true
            }
            None => true,
        }
    }

    /// The agent cancels a request: if it wasn't sent, it never is; if it
    /// was, the server hears of it.
    fn cancel(&self, message: &Value) -> bool {
        let Some(id) = message.pointer("/params/requestId") else {
            return true;
        };
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        match pending.get_mut(&id_key(id)) {
            Some(p @ Pending::Waiting) => {
                *p = Pending::Cancelled;
                false
            }
            Some(Pending::Sent { cancelled }) => {
                *cancelled = true;
                true
            }
            _ => true,
        }
    }

    async fn request_on(self: Arc<Self>, message: Value, id: Value, method: &str) {
        let message = without_meta_offers(message);
        let deadline = Instant::now() + self.knobs.queue_wait;
        let mut asked_again = false;
        loop {
            let link = match self.ready_link(deadline).await {
                Some(link) => link,
                None if method == "ping" => {
                    return self.answer(&id, json!({"jsonrpc": "2.0", "id": id, "result": {}}));
                }
                None => {
                    return self.answer(&id, error(
                        &id,
                        format!(
                            "the books didn't open within {} s: run `wanderbooks doctor` to see why",
                            self.knobs.queue_wait.as_secs()
                        ),
                    ));
                }
            };
            let up = match link {
                Link::Ready(up) => up,
                Link::Closed => return self.answer(&id, error(&id, CLOSED_BY_OWNER)),
                Link::Failed(why) => return self.answer(&id, error(&id, why)),
                // Refused when last asked: the books may exist now (made
                // since), so the server is asked again, once, before
                // this call is refused.
                Link::Refused(_) if !asked_again => {
                    asked_again = true;
                    self.link.send_replace(Link::Opening);
                    self.renew.notify_one();
                    continue;
                }
                Link::Refused(why) => return self.answer(&id, error(&id, why)),
                Link::Opening => continue,
            };
            if !self.may_send(&id) {
                // Cancelled while waiting: no answer is owed.
                return;
            }
            match self.send(&message, &up).await {
                Sent::Answered(messages)
                    if !messages.iter().any(|m| {
                        m.get("id") == Some(&id)
                            && (m.get("result").is_some() || m.get("error").is_some())
                    }) =>
                {
                    // Answered with nothing (its session ended while it
                    // ran): it may have been done (INVARIANT 7).
                    self.forget(&id);
                    for m in messages {
                        self.tell(m);
                    }
                    return self.answer(&id, error(&id, MAYBE_RAN));
                }
                Sent::Answered(messages) => {
                    self.forget(&id);
                    for m in messages {
                        if method == "tools/list"
                            && m.get("id") == Some(&id)
                            && let Some(result) = m.get("result")
                        {
                            *self.tools_seen.lock().unwrap_or_else(|e| e.into_inner()) =
                                Some(result.clone());
                        }
                        self.tell(m);
                    }
                    return;
                }
                // A ping not answered is the bridge's to answer.
                Sent::NotRun(_) if method == "ping" => {
                    return self.answer(&id, json!({"jsonrpc": "2.0", "id": id, "result": {}}));
                }
                Sent::NotRun(NotRun::NoSession) => {
                    // INVARIANT 4: not run; a new session, then again.
                    self.requeue(&id);
                    self.session_gone(up.epoch);
                }
                Sent::NotRun(_) => {
                    self.requeue(&id);
                    self.session_down(up.epoch);
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                Sent::MaybeRan => return self.answer(&id, error(&id, MAYBE_RAN)),
                Sent::Refused(why) => return self.answer(&id, error(&id, why)),
            }
        }
    }

    /// Answer the agent's initialize (INVARIANT 2).
    async fn initialize(self: Arc<Self>, message: Value, id: Value) {
        let params =
            without_client_offers(message.get("params").cloned().unwrap_or_else(|| json!({})));
        let asked = params
            .get("protocolVersion")
            .and_then(Value::as_str)
            .map(str::to_string);
        *self.agent_init.lock().unwrap_or_else(|e| e.into_inner()) = Some(params);
        self.renew.notify_one();
        let deadline = Instant::now() + self.knobs.init_wait;
        let result = match self.ready_link(deadline).await {
            Some(Link::Ready(up)) => up.init.clone(),
            // Not "opening now": closed by the owner, or can't open.
            Some(Link::Closed) => return self.answer(&id, error(&id, CLOSED_BY_OWNER)),
            Some(Link::Failed(why)) => return self.answer(&id, error(&id, why)),
            _ => None,
        };
        let result = result.unwrap_or_else(|| {
            with_list_changed(ledger_mcp::opening::opening_initialize(asked.as_deref()))
        });
        self.answer(&id, json!({"jsonrpc": "2.0", "id": id, "result": result}));
    }

    /// The session `epoch` is unknown to the server: open another.
    fn session_gone(&self, epoch: u64) {
        let gone = matches!(&*self.link.borrow(), Link::Ready(up) if up.epoch == epoch);
        if gone {
            self.link.send_replace(Link::Opening);
            self.renew.notify_one();
        }
    }

    /// The server isn't answering on session `epoch`: wait for the lease
    /// to say what comes next.
    fn session_down(&self, epoch: u64) {
        self.session_gone(epoch);
    }

    /// Keep the lease, and a session whenever the server is ready
    /// (INVARIANTS 1, 2, 5, 6).
    async fn keep(self: Arc<Self>, mut opener: Opener) {
        // INVARIANT 11: where its token is kept; it is read before each
        // new session until one is found.
        let run_dir = opener.run_dir().to_path_buf();
        let root = run_dir.parent().unwrap_or(&run_dir).to_path_buf();
        let account = crate::tokens::Account::new(&root, &self.book, &self.token_name);
        let _ = self.keyring.set((account, run_dir));
        let mut epoch = 0u64;
        loop {
            let mut lease = match opener.hold().await {
                Ok(lease) => lease,
                // INVARIANT 6: closed by the owner while holding again.
                Err(crate::opener::HoldError::ClosedByOwner) => {
                    self.link.send_replace(Link::Closed);
                    return;
                }
                Err(crate::opener::HoldError::Failed(why)) => {
                    self.link.send_replace(Link::Failed(why));
                    return;
                }
            };
            // Until this lease ends: whenever the server is ready and no
            // session is up, open one. The lease is always heard, even
            // while a session is being opened or a try waits (INVARIANT 1).
            let mut ready = false;
            let mut retry_at: Option<Instant> = None;
            let ended = loop {
                let opening = ready && matches!(&*self.link.borrow(), Link::Opening);
                let waiting = still_waiting(retry_at, Instant::now());
                let open_now = opening && waiting.is_none();
                tokio::select! {
                    msg = lease.next() => match msg {
                        Some(HostMsg::Ready) => {
                            opener.opened();
                            // INVARIANT 9: the server's socket is ours; one
                            // not answering is a failure like any other.
                            match self.socket_is_ours() {
                                Ok(()) => ready = true,
                                Err(Owner::Other(why)) => {
                                    self.link.send_replace(Link::Failed(why));
                                    return;
                                }
                                Err(Owner::Unreachable) => break None,
                            }
                        }
                        Some(HostMsg::Ending { reason, .. }) => break Some(reason),
                        Some(_) => {}
                        None => break None,
                    },
                    opened = self.open_session(epoch + 1), if open_now => match opened {
                        Ok(up) => {
                            epoch = up.epoch;
                            retry_at = None;
                            self.after_new_session(&up).await;
                            self.link.send_replace(Link::Ready(Arc::new(up)));
                        }
                        Err(Some(why)) => {
                            self.link.send_replace(Link::Refused(why));
                        }
                        Err(None) => retry_at = Some(Instant::now() + Duration::from_millis(100)),
                    },
                    () = tokio::time::sleep_until(waiting.unwrap_or_else(Instant::now)), if opening && waiting.is_some() => {
                        retry_at = None;
                    }
                    () = self.renew.notified(), if !open_now => {
                        // The agent initialized, or a session is gone:
                        // a session that has no init yet is renewed.
                        let stale = match &*self.link.borrow() {
                            Link::Ready(up) => {
                                up.init.is_none()
                                    && self
                                        .agent_init
                                        .lock()
                                        .unwrap_or_else(|e| e.into_inner())
                                        .is_some()
                            }
                            Link::Refused(_) => true,
                            _ => false,
                        };
                        if stale {
                            self.link.send_replace(Link::Opening);
                        }
                    }
                }
            };
            self.link.send_replace(Link::Opening);
            match ended {
                Some(Reason::ClosedByOwner) => {
                    self.link.send_replace(Link::Closed);
                    return;
                }
                Some(Reason::Restarting | Reason::Closing) => {}
                Some(Reason::ServerFailed) | None => {
                    if let Err(why) = opener.failed().await {
                        self.link.send_replace(Link::Failed(why));
                        return;
                    }
                }
            }
        }
    }

    /// A session with the server: with the agent's initialize when it
    /// sent one, else none (a client without the handshake), within
    /// UPSTREAM_WAIT.
    async fn open_session(&self, epoch: u64) -> Result<Upstream, Option<String>> {
        let params = self
            .agent_init
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        match params {
            Some(params) => {
                tokio::time::timeout(UPSTREAM_WAIT, self.initialize_upstream(&params, epoch))
                    .await
                    .unwrap_or(Err(None))
            }
            None => Ok(Upstream {
                epoch,
                ..Upstream::default()
            }),
        }
    }

    /// Whether the server's socket belongs to this account (INVARIANT 9).
    fn socket_is_ours(&self) -> Result<(), Owner> {
        let stream = std::os::unix::net::UnixStream::connect(&self.socket)
            .map_err(|_| Owner::Unreachable)?;
        if ledger_mcp::local::peer_is_us(&stream) {
            Ok(())
        } else {
            Err(Owner::Other(
                "the books server's socket belongs to another account: not used".into(),
            ))
        }
    }

    /// End the upstream session, if any (the agent's session ended).
    async fn end_session(&self) {
        let link = self.link.borrow().clone();
        if let Link::Ready(up) = link
            && let Some(session) = &up.session
        {
            let _ = tokio::time::timeout(
                Duration::from_secs(2),
                self.upstream(reqwest::Method::DELETE)
                    .header("mcp-session-id", session)
                    .send(),
            )
            .await;
        }
    }

    /// After a new session: tell the agent if the tools changed
    /// (INVARIANT 5).
    async fn after_new_session(&self, up: &Upstream) {
        if up.epoch <= 1 {
            return;
        }
        let seen = self
            .tools_seen
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let Some(seen) = seen else { return };
        if let Some(now) = self.tool_list(up).await
            && now != seen
        {
            *self.tools_seen.lock().unwrap_or_else(|e| e.into_inner()) = Some(now);
            self.tell(json!({"jsonrpc": "2.0", "method": "notifications/tools/list_changed"}));
        }
    }

    /// One message from the agent.
    fn handle(self: &Arc<Self>, message: Value, tasks: &tokio_util::task::TaskTracker) {
        if let Some(batch) = message.as_array() {
            // Batches aren't carried: each request in one is refused, so
            // none goes unanswered (INVARIANT 7).
            let refused: Vec<Value> = batch
                .iter()
                .filter_map(|m| m.get("id").filter(|id| !id.is_null()))
                .map(|id| {
                    json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32600,
                        "message": "batches aren't supported: send each message on its own"}})
                })
                .collect();
            if refused.is_empty() && batch.is_empty() {
                // An empty batch is itself an invalid request.
                self.tell(
                    json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32600,
                    "message": "an empty batch isn't a request"}}),
                );
            } else if !refused.is_empty() {
                self.tell(Value::Array(refused));
            }
            return;
        }
        // INVARIANT 7: what isn't a request, a notification or an answer
        // is answered as an invalid request, with its id if it has one
        // that is an id (a string or a whole number).
        let id = message.get("id").filter(|id| !id.is_null());
        let good_id = id.is_none_or(|id| id.is_string() || id.is_i64() || id.is_u64());
        let is_answer = message.get("result").is_some() || message.get("error").is_some();
        let named = message.get("method").is_some_and(Value::is_string);
        if !message.is_object() || !good_id || (!named && !is_answer) {
            let id = id.filter(|_| good_id).cloned().unwrap_or(Value::Null);
            self.tell(json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32600,
                "message": "invalid request: a message is an object with a method (and a string or number id), or an answer"}}));
            return;
        }
        let has_id = id.is_some();
        let method = message.get("method").and_then(Value::as_str);
        match (method, has_id) {
            (Some(method), true) => {
                // An id already in flight is refused: answers are told
                // apart by id (JSON-RPC asks each to be unique).
                if let Some(id) = message.get("id")
                    && self
                        .pending
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .contains_key(&id_key(id))
                {
                    self.tell(json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32600,
                        "message": "a request with that id is still waiting: give each its own id"}}));
                    return;
                }
                let _ = method;
                if let Some(id) = message.get("id") {
                    self.waiting(id);
                }
                tasks.spawn(self.clone().request(message));
            }
            (Some("notifications/initialized"), false) => {
                // The bridge sends its own, once per session.
            }
            (Some("notifications/cancelled"), false) if !self.cancel(&message) => {
                // Cancelled before it was sent: nothing to tell the server.
            }
            (Some(_), false) => {
                let me = self.clone();
                tasks.spawn(async move {
                    let link = me.link.borrow().clone();
                    if let Link::Ready(up) = link {
                        let _ = me.send(&message, &up).await;
                    }
                });
            }
            (None, _) => {
                // An answer to a request: the bridge sends the agent none.
            }
        }
    }
}

/// A retry set for `retry_at`, if it is still ahead of `now` (until
/// then the keeper doesn't try to open a session again).
fn still_waiting(retry_at: Option<Instant>, now: Instant) -> Option<Instant> {
    retry_at.filter(|at| *at > now)
}

/// Write one message to the agent, a line of JSON, and flush it.
async fn write_line<W: tokio::io::AsyncWrite + Unpin>(
    out: &mut W,
    message: &Value,
) -> std::io::Result<()> {
    let mut line = message.to_string();
    line.push('\n');
    out.write_all(line.as_bytes()).await?;
    out.flush().await
}

/// How the agent's input ended.
#[derive(Debug, PartialEq, Eq)]
enum InputEnd {
    /// It reached its end (or failed).
    Closed,
    /// A signal came first.
    Signalled,
}

/// Read the agent's messages, at most `max` bytes each, and hand each to
/// the bridge, until the input ends or `stop` is notified (INVARIANT 7:
/// every malformed one is answered, and the session goes on).
async fn read_messages<R: tokio::io::AsyncBufRead + Unpin>(
    input: &mut R,
    max: usize,
    bridge: &Arc<Bridge>,
    tasks: &tokio_util::task::TaskTracker,
    stop: &Notify,
) -> InputEnd {
    let mut line: Vec<u8> = Vec::new();
    loop {
        line.clear();
        // Bytes, not text: a malformed byte is answered, never the end
        // of the session (INVARIANT 7).
        let mut limited = (&mut *input).take(max as u64 + 1);
        let read = tokio::select! {
            read = limited.read_until(b'\n', &mut line) => read,
            () = stop.notified() => return InputEnd::Signalled,
        };
        match read {
            Ok(0) | Err(_) => return InputEnd::Closed,
            // A whole line, its newline within the limit: at most `max`
            // bytes of message.
            Ok(_) if line.ends_with(b"\n") => {}
            // `max` + 1 bytes and no newline yet: too large.
            Ok(n) if n > max => {
                // Refused, and the rest of it skipped: the session goes on.
                bridge.tell(error(
                    &Value::Null,
                    format!(
                        "that message is too large (at most {} MiB)",
                        max / (1024 * 1024)
                    ),
                ));
                if !skip_line(input).await {
                    return InputEnd::Closed;
                }
                continue;
            }
            // The input's last line, without a newline.
            Ok(_) => {}
        }
        let Ok(text) = std::str::from_utf8(&line) else {
            bridge.tell(json!({"jsonrpc": "2.0", "id": null,
                "error": {"code": -32700, "message": "parse error: a message is UTF-8 text"}}));
            continue;
        };
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        match serde_json::from_str::<Value>(text) {
            Ok(message) => bridge.handle(message, tasks),
            Err(_) => bridge.tell(json!({"jsonrpc": "2.0", "id": null,
                "error": {"code": -32700, "message": "parse error"}})),
        }
    }
}

/// Skip the rest of a line too long to take; false at the input's end.
async fn skip_line<R: tokio::io::AsyncBufRead + Unpin>(input: &mut R) -> bool {
    loop {
        let buf = match input.fill_buf().await {
            Ok(buf) if !buf.is_empty() => buf,
            _ => return false,
        };
        match buf.iter().position(|b| *b == b'\n') {
            Some(at) => {
                input.consume(at + 1);
                return true;
            }
            None => {
                let n = buf.len();
                input.consume(n);
            }
        }
    }
}

fn usage(prog: &str) {
    // (`--root DIR` too, for tests: the bridge never needs a folder.)
    eprintln!("usage: {prog} --book ID [--token NAME]");
    eprintln!("  Connects an agent (over stdio) to the books called ID, opening them if");
    eprintln!("  they are closed and holding them open for the session. It uses the");
    eprintln!("  token NAME (default: agent) that `wanderbooks token create` kept in");
    eprintln!("  this computer's keyring.");
}

/// `wanderbooks mcp --book ID`.
pub fn run(
    prog: &str,
    args: impl IntoIterator<Item = String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut book: Option<String> = None;
    let mut token_name = crate::tokens::DEFAULT_NAME.to_string();
    let mut root: Option<PathBuf> = None;
    let mut knobs = Knobs::default();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--book" => book = Some(args.next().ok_or("--book needs an id")?),
            "--token" => token_name = args.next().ok_or("--token needs a name")?,
            "--root" => root = Some(PathBuf::from(args.next().ok_or("--root needs a folder")?)),
            FLAG => knobs = Knobs::parse(&args.next().ok_or("--test-knobs needs a value")?)?,
            "--help" | "-h" => {
                usage(prog);
                return Ok(());
            }
            other => {
                eprintln!("unknown argument: {other}");
                usage(prog);
                return Err(Box::new(crate::Exit(2)));
            }
        }
    }
    let Some(book) = book else {
        eprintln!("{prog}: which books? give --book ID");
        usage(prog);
        return Err(Box::new(crate::Exit(2)));
    };
    let book = ledger_store::root::BookId::parse(&book)
        .map_err(|e| e.to_string())?
        .to_string();
    let opener = Opener::new(root.as_deref(), knobs, Kind::Bridge, Some(book.clone()))?;
    // Kept for the whole session: the client reaches the socket by it.
    let socket =
        ledger_store::root::SocketPath::new(opener.run_dir(), ledger_mcp::local::MCP_SOCKET)
            .map_err(|e| format!("can't reach the books server: {e}"))?;
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let result = rt.block_on(async move {
        // A connection per call: one the server closes while idle can't
        // swallow a call (INVARIANT 4).
        let http = reqwest::Client::builder()
            .unix_socket(socket.path())
            .pool_max_idle_per_host(0)
            .no_proxy()
            .build()
            .map_err(|e| e.to_string())?;
        let (to_agent, mut outbox) = mpsc::unbounded_channel::<Value>();
        let (link, _) = watch::channel(Link::Opening);
        let bridge = Arc::new(Bridge {
            book,
            token_name,
            token: Mutex::new(None),
            keyring: std::sync::OnceLock::new(),
            keyring_trouble: Mutex::new(None),
            http,
            knobs,
            to_agent,
            link,
            agent_init: Mutex::new(None),
            renew: Notify::new(),
            tools_seen: Mutex::new(None),
            next_id: std::sync::atomic::AtomicU64::new(1),
            pending: Mutex::new(std::collections::HashMap::new()),
            socket: socket.path().to_path_buf(),
        });
        let keeper = tokio::spawn(bridge.clone().keep(opener));
        let writer = tokio::spawn(async move {
            let mut out = tokio::io::stdout();
            while let Some(message) = outbox.recv().await {
                if write_line(&mut out, &message).await.is_err() {
                    break;
                }
            }
        });
        let tasks = tokio_util::task::TaskTracker::new();
        let mut input = tokio::io::BufReader::new(tokio::io::stdin());
        // A client that stops the bridge with a signal (most do) still has
        // its session ended upstream (INVARIANT 10).
        let stop = Arc::new(Notify::new());
        for kind in [
            tokio::signal::unix::SignalKind::terminate(),
            tokio::signal::unix::SignalKind::interrupt(),
            tokio::signal::unix::SignalKind::hangup(),
        ] {
            let mut signals = tokio::signal::unix::signal(kind).map_err(|e| e.to_string())?;
            let stop = stop.clone();
            tokio::spawn(async move {
                if signals.recv().await.is_some() {
                    stop.notify_one();
                }
            });
        }
        let mut signalled = read_messages(&mut input, MAX_MESSAGE, &bridge, &tasks, &stop).await
            == InputEnd::Signalled;
        // The agent's session ended: every request answered (unless it
        // was, or is now, stopped by a signal: it isn't waiting for them),
        // its session ended upstream, then the lease goes with this
        // process.
        tasks.close();
        if !signalled {
            tokio::select! {
                () = tasks.wait() => {}
                () = stop.notified() => signalled = true,
            }
        }
        bridge.end_session().await;
        keeper.abort();
        drop(bridge);
        if signalled {
            // Stopped: nothing more is said, and calls still waiting for
            // the books go with the process (they were never sent).
            writer.abort();
        } else {
            let _ = writer.await;
        }
        Ok::<(), String>(())
    });
    rt.shutdown_timeout(Duration::from_secs(1));
    result.map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_event_stream_yields_each_message() {
        let body = "id: 1\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"n\"}\n\n\
                    data: {\"jsonrpc\":\"2.0\",\n\
                    data: \"id\":1,\"result\":{}}\n\n\
                    data:\n\n: comment\nretry: 3000\n\n";
        let got = sse_messages(body);
        assert_eq!(
            got,
            vec![
                json!({"jsonrpc": "2.0", "method": "n"}),
                json!({"jsonrpc": "2.0", "id": 1, "result": {}}),
            ]
        );
        assert!(sse_messages("").is_empty());
        assert_eq!(
            sse_messages("data: {\"a\":1}").len(),
            1,
            "no blank line at the end"
        );
    }

    #[test]
    fn every_initialize_result_says_the_tools_may_change() {
        let r = with_list_changed(json!({"capabilities": {"tools": {}}}));
        assert_eq!(r["capabilities"]["tools"]["listChanged"], true);
        let r = with_list_changed(json!({"capabilities": {}}));
        assert_eq!(r["capabilities"]["tools"]["listChanged"], true);
        let r = with_list_changed(json!({"capabilities": {"tools": {"listChanged": false}}}));
        assert_eq!(r["capabilities"]["tools"]["listChanged"], true);
    }

    #[test]
    fn requests_carry_the_headers_their_protocol_needs() {
        let call = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": "books.start", "_meta": {META_VERSION: "2026-07-28"}}});
        let h = message_headers(&call, None);
        assert!(h.contains(&("mcp-protocol-version", "2026-07-28".into())));
        assert!(h.contains(&("mcp-method", "tools/call".into())));
        assert!(h.contains(&("mcp-name", "books.start".into())));
        let read = json!({"method": "resources/read", "params": {"uri": "books://digest"}});
        let h = message_headers(&read, Some("2026-07-28"));
        assert!(h.contains(&("mcp-name", "books://digest".into())));
        // Older sessions: the version alone.
        let h = message_headers(&call, Some("2025-06-18"));
        assert!(
            h.contains(&("mcp-protocol-version", "2026-07-28".into())),
            "the message's own wins"
        );
        let plain = json!({"method": "tools/list"});
        assert_eq!(
            message_headers(&plain, Some("2025-06-18")),
            vec![("mcp-protocol-version", "2025-06-18".to_string())]
        );
        assert!(message_headers(&plain, None).is_empty());
        // A value that couldn't travel as it is goes in base64.
        assert_eq!(header_value("books.start"), "books.start");
        assert!(header_value(" x").starts_with("=?base64?"));
        assert!(header_value("café").starts_with("=?base64?"));
    }

    #[test]
    fn a_requests_own_offers_are_left_out_too() {
        let m = without_meta_offers(json!({"method": "tools/list", "params": {"_meta": {
            "io.modelcontextprotocol/clientCapabilities": {"sampling": {}, "roots": {}, "x": 1}}}}));
        assert_eq!(
            m["params"]["_meta"]["io.modelcontextprotocol/clientCapabilities"],
            json!({"x": 1})
        );
        let plain = json!({"method": "ping"});
        assert_eq!(without_meta_offers(plain.clone()), plain);
    }

    #[test]
    fn the_agents_offers_the_bridge_cant_carry_are_left_out() {
        let p = without_client_offers(json!({"capabilities": {
            "sampling": {}, "elicitation": {}, "roots": {"listChanged": true}, "experimental": {}}}));
        assert_eq!(p["capabilities"], json!({"experimental": {}}));
        assert_eq!(without_client_offers(json!({})), json!({}));
    }

    #[test]
    fn an_error_answer_keeps_the_request_id() {
        let e = error(&json!(7), "no");
        assert_eq!(e["id"], 7);
        assert_eq!(e["error"]["code"], SERVER_ERROR);
        assert_eq!(e["error"]["code"], -32000, "JSON-RPC's server error");
        assert_eq!(e["error"]["message"], "no");
    }

    #[test]
    fn an_events_data_lines_are_joined_by_newlines() {
        // Joined without one, these two would read as {"n":12}.
        assert!(sse_messages("data: {\"n\":1\ndata: 2}\n\n").is_empty());
        assert_eq!(
            sse_messages("data: {\"n\":\ndata: 12}\n\n"),
            vec![json!({"n": 12})]
        );
    }

    #[test]
    fn a_task_request_names_its_task() {
        let get = json!({"method": "tasks/get", "params": {"taskId": "t-1"}});
        let h = message_headers(&get, Some("2026-07-28"));
        assert!(h.contains(&("mcp-name", "t-1".into())), "{h:?}");
    }

    #[test]
    fn a_retry_is_waited_for_only_while_it_is_ahead() {
        let now = Instant::now();
        let later = now + Duration::from_secs(1);
        assert_eq!(still_waiting(Some(later), now), Some(later));
        assert_eq!(still_waiting(Some(now), now), None);
        assert_eq!(still_waiting(Some(now), later), None);
        assert_eq!(still_waiting(None, now), None);
    }

    #[tokio::test]
    async fn a_message_to_the_agent_is_one_line() {
        let mut out: Vec<u8> = Vec::new();
        write_line(&mut out, &json!({"a": 1})).await.unwrap();
        write_line(&mut out, &json!([2])).await.unwrap();
        assert_eq!(out, b"{\"a\":1}\n[2]\n");
    }
}

/// The bridge against a scripted server on a Unix socket: every way an
/// answer can come back, and the bridge's own bookkeeping.
#[cfg(test)]
mod against_a_server {
    use super::*;
    use std::collections::{HashMap, VecDeque};
    use std::path::Path;

    /// What the scripted server answers next.
    struct Reply {
        status: u16,
        headers: Vec<(&'static str, String)>,
        body: String,
        /// Promise more body than is sent, then hang up.
        cut: bool,
    }

    fn reply(status: u16, content_type: &str, body: &str) -> Reply {
        Reply {
            status,
            headers: vec![("content-type", content_type.to_string())],
            body: body.to_string(),
            cut: false,
        }
    }

    fn json_reply(status: u16, body: Value) -> Reply {
        reply(status, "application/json", &body.to_string())
    }

    /// A request the server received.
    #[derive(Debug, Clone)]
    struct Seen {
        method: String,
        headers: HashMap<String, String>,
        body: String,
    }

    struct Server {
        _dir: tempfile::TempDir,
        socket: PathBuf,
        replies: Arc<Mutex<VecDeque<Reply>>>,
        seen: Arc<Mutex<Vec<Seen>>>,
    }

    impl Server {
        fn start() -> Self {
            let dir = tempfile::Builder::new().prefix("wb-br-").tempdir().unwrap();
            let socket = dir.path().join("mcp.sock");
            let listener = tokio::net::UnixListener::bind(&socket).unwrap();
            let replies: Arc<Mutex<VecDeque<Reply>>> = Arc::default();
            let seen: Arc<Mutex<Vec<Seen>>> = Arc::default();
            let (r, s) = (replies.clone(), seen.clone());
            tokio::spawn(async move {
                while let Ok((mut stream, _)) = listener.accept().await {
                    let (r, s) = (r.clone(), s.clone());
                    tokio::spawn(async move {
                        let mut buf = Vec::new();
                        let mut chunk = [0u8; 4096];
                        let head_end = loop {
                            let n = stream.read(&mut chunk).await.unwrap_or(0);
                            if n == 0 {
                                return;
                            }
                            buf.extend_from_slice(&chunk[..n]);
                            if let Some(at) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                                break at + 4;
                            }
                        };
                        let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
                        let mut lines = head.split("\r\n");
                        let method = lines
                            .next()
                            .unwrap_or_default()
                            .split(' ')
                            .next()
                            .unwrap_or_default()
                            .to_string();
                        let headers: HashMap<String, String> = lines
                            .filter_map(|l| l.split_once(':'))
                            .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
                            .collect();
                        let len: usize = headers
                            .get("content-length")
                            .and_then(|v| v.parse().ok())
                            .unwrap_or(0);
                        while buf.len() < head_end + len {
                            let n = stream.read(&mut chunk).await.unwrap_or(0);
                            if n == 0 {
                                break;
                            }
                            buf.extend_from_slice(&chunk[..n]);
                        }
                        let body = String::from_utf8_lossy(&buf[head_end..]).to_string();
                        s.lock().unwrap().push(Seen {
                            method,
                            headers,
                            body,
                        });
                        let next = r.lock().unwrap().pop_front();
                        let next = next.unwrap_or_else(|| reply(500, "text/plain", "no reply"));
                        let mut out = format!(
                            "HTTP/1.1 {} X\r\ncontent-length: {}\r\nconnection: close\r\n",
                            next.status,
                            next.body.len() + if next.cut { 100 } else { 0 }
                        );
                        for (k, v) in &next.headers {
                            out.push_str(&format!("{k}: {v}\r\n"));
                        }
                        out.push_str("\r\n");
                        out.push_str(&next.body);
                        let _ = stream.write_all(out.as_bytes()).await;
                        let _ = stream.shutdown().await;
                    });
                }
            });
            Self {
                _dir: dir,
                socket,
                replies,
                seen,
            }
        }

        fn script(&self, replies: impl IntoIterator<Item = Reply>) {
            self.replies.lock().unwrap().extend(replies);
        }

        fn seen(&self) -> Vec<Seen> {
            self.seen.lock().unwrap().clone()
        }
    }

    fn bridge_at(socket: &Path, knobs: Knobs) -> (Arc<Bridge>, mpsc::UnboundedReceiver<Value>) {
        let http = reqwest::Client::builder()
            .unix_socket(socket)
            .pool_max_idle_per_host(0)
            .no_proxy()
            .build()
            .unwrap();
        let (to_agent, rx) = mpsc::unbounded_channel();
        let (link, _) = watch::channel(Link::Opening);
        let bridge = Arc::new(Bridge {
            book: "acme".into(),
            // A token the fake server doesn't check; no keyring is read.
            token_name: crate::tokens::DEFAULT_NAME.into(),
            token: Mutex::new(Some("wb_test".into())),
            keyring: std::sync::OnceLock::new(),
            keyring_trouble: Mutex::new(None),
            http,
            knobs,
            to_agent,
            link,
            agent_init: Mutex::new(None),
            renew: Notify::new(),
            tools_seen: Mutex::new(None),
            next_id: std::sync::atomic::AtomicU64::new(1),
            pending: Mutex::new(std::collections::HashMap::new()),
            socket: socket.to_path_buf(),
        });
        (bridge, rx)
    }

    fn knobs(queue_ms: u64, init_ms: u64) -> Knobs {
        Knobs {
            queue_wait: Duration::from_millis(queue_ms),
            init_wait: Duration::from_millis(init_ms),
            ..Knobs::default()
        }
    }

    fn up(epoch: u64) -> Upstream {
        Upstream {
            session: Some("s1".into()),
            version: Some("2025-06-18".into()),
            init: Some(json!({})),
            epoch,
        }
    }

    fn ready(b: &Bridge, epoch: u64) {
        b.link.send_replace(Link::Ready(Arc::new(up(epoch))));
    }

    fn what(sent: &Sent) -> String {
        match sent {
            Sent::Answered(m) => format!("answered {}", Value::Array(m.clone())),
            Sent::NotRun(why) => format!("not run {why:?}"),
            Sent::MaybeRan => "maybe ran".into(),
            Sent::Refused(why) => format!("refused {why}"),
        }
    }

    const CALL: &str =
        r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"books.start"}}"#;

    #[tokio::test]
    async fn every_kind_of_answer_is_read_for_what_it_is() {
        let server = Server::start();
        let (b, _rx) = bridge_at(&server.socket, knobs(1000, 1000));
        let answer = json!({"jsonrpc": "2.0", "id": 7, "result": {}});
        let closing = json!({"jsonrpc": "2.0", "id": 7,
            "error": {"code": ledger_mcp::NOT_RUN_CLOSING_CODE, "message": "closing"}});
        let refused_header = Reply {
            status: 404,
            headers: vec![(ledger_mcp::local::REFUSED_HEADER, "no-such-books".into())],
            body: "there are no books called \"x\"".into(),
            cut: false,
        };
        let cases: Vec<(Reply, &str)> = vec![
            (reply(202, "text/plain", ""), "answered []"),
            (
                json_reply(200, answer.clone()),
                "answered [{\"id\":7,\"jsonrpc\":\"2.0\",\"result\":{}}]",
            ),
            (json_reply(200, json!({"x": 1})), "answered [{\"x\":1}]"),
            (
                reply(200, "text/event-stream", &format!("data: {answer}\n\n")),
                "answered [{\"id\":7,\"jsonrpc\":\"2.0\",\"result\":{}}]",
            ),
            (json_reply(200, closing.clone()), "not run Closing"),
            (reply(404, "text/plain", NO_SESSION), "not run NoSession"),
            (
                reply(404, "text/plain", "Not Found: Session not found."),
                "refused Not Found: Session not found.",
            ),
            (
                reply(
                    503,
                    "text/plain",
                    "the books are closing: this request wasn't run",
                ),
                "not run Closing",
            ),
            (reply(503, "text/plain", "overloaded"), "maybe ran"),
            (refused_header, "refused there are no books called \"x\""),
            (
                json_reply(500, answer.clone()),
                "answered [{\"id\":7,\"jsonrpc\":\"2.0\",\"result\":{}}]",
            ),
            (reply(400, "text/plain", "bad"), "refused bad"),
            (reply(500, "text/plain", "boom"), "maybe ran"),
        ];
        let message: Value = serde_json::from_str(CALL).unwrap();
        for (r, want) in cases {
            server.script([r]);
            let got = what(&b.send(&message, &up(1)).await);
            assert_eq!(got, want);
        }
        // Every request carried its session and version.
        for seen in server.seen() {
            assert_eq!(seen.method, "POST");
            assert_eq!(
                seen.headers.get("mcp-session-id").map(String::as_str),
                Some("s1")
            );
            assert_eq!(
                seen.headers.get("mcp-protocol-version").map(String::as_str),
                Some("2025-06-18")
            );
            assert_eq!(
                serde_json::from_str::<Value>(&seen.body).unwrap(),
                serde_json::from_str::<Value>(CALL).unwrap()
            );
        }
        // Refused at the door, its words cut off: still refused, unrun.
        server.script([Reply {
            status: 503,
            headers: vec![(ledger_mcp::local::REFUSED_HEADER, "not-open".into())],
            body: "the books acme didn't".into(),
            cut: true,
        }]);
        assert_eq!(
            what(&b.send(&message, &up(1)).await),
            "refused the books server refused the call"
        );
        // An answer cut off: it may have run.
        server.script([Reply {
            cut: true,
            ..json_reply(200, answer.clone())
        }]);
        assert_eq!(what(&b.send(&message, &up(1)).await), "maybe ran");
        // Nobody listening: not run, safe to send again.
        let gone = server.socket.with_file_name("gone.sock");
        let (b, _rx) = bridge_at(&gone, knobs(1000, 1000));
        assert_eq!(what(&b.send(&message, &up(1)).await), "not run Unreachable");
    }

    #[tokio::test]
    async fn a_session_is_opened_or_refused_plainly() {
        let server = Server::start();
        let (b, _rx) = bridge_at(&server.socket, knobs(1000, 1000));
        let params = json!({"protocolVersion": "2025-06-18", "capabilities": {},
            "clientInfo": {"name": "t", "version": "0"}});
        // Closing, or a server error: not answering yet.
        server.script([
            reply(
                503,
                "text/plain",
                "the books are closing: this request wasn't run",
            ),
            reply(500, "text/plain", "boom"),
        ]);
        assert_eq!(b.initialize_upstream(&params, 1).await.unwrap_err(), None);
        assert_eq!(b.initialize_upstream(&params, 1).await.unwrap_err(), None);
        // Refused: the server's words.
        server.script([
            reply(503, "text/plain", "the books acme didn't open: damaged"),
            reply(404, "text/plain", "there are no books called acme"),
            reply(400, "text/plain", "bad initialize"),
        ]);
        for want in [
            "the books acme didn't open: damaged",
            "there are no books called acme",
            "bad initialize",
        ] {
            assert_eq!(
                b.initialize_upstream(&params, 1)
                    .await
                    .unwrap_err()
                    .as_deref(),
                Some(want)
            );
        }
        // Opened: its session, its version, and the bridge's initialized.
        let result = json!({"protocolVersion": "2025-06-18", "capabilities": {"tools": {}},
            "serverInfo": {"name": "wanderbooks", "version": "0"}});
        server.script([
            Reply {
                status: 200,
                headers: vec![
                    ("content-type", "application/json".into()),
                    ("mcp-session-id", "abc".into()),
                ],
                body: json!({"jsonrpc": "2.0", "id": "x", "result": result}).to_string(),
                cut: false,
            },
            reply(202, "text/plain", ""),
        ]);
        let up = b.initialize_upstream(&params, 4).await.unwrap();
        assert_eq!(up.session.as_deref(), Some("abc"));
        assert_eq!(up.version.as_deref(), Some("2025-06-18"));
        assert_eq!(up.epoch, 4);
        assert_eq!(
            up.init.unwrap()["capabilities"]["tools"]["listChanged"],
            true
        );
        let seen = server.seen();
        let last = seen.last().unwrap();
        assert!(last.body.contains("notifications/initialized"), "{last:?}");
        assert_eq!(
            last.headers.get("mcp-session-id").map(String::as_str),
            Some("abc")
        );
        // Without the agent's initialize, a session needs none.
        let none = b.open_session(5).await.unwrap();
        assert_eq!(none.epoch, 5);
        assert!(none.session.is_none() && none.init.is_none());
    }

    /// A bridge whose token is kept in a test keyring in `dir` (none held
    /// yet), and that keyring's folder.
    fn bridge_with_keyring(
        socket: &Path,
        dir: &Path,
    ) -> (Arc<Bridge>, mpsc::UnboundedReceiver<Value>, PathBuf) {
        let knobs = Knobs {
            file_keyring: true,
            ..knobs(1000, 1000)
        };
        let (b, rx) = bridge_at(socket, knobs);
        *b.token.lock().unwrap() = None;
        let account = crate::tokens::Account::new(dir, "acme", crate::tokens::DEFAULT_NAME);
        b.keyring.set((account, dir.to_path_buf())).unwrap();
        let keyring = dir.join("test-keyring");
        std::fs::create_dir_all(&keyring).unwrap();
        (b, rx, keyring)
    }

    /// The door's refusal of a request with no token.
    fn no_token_at_the_door() -> Reply {
        Reply {
            status: 401,
            headers: vec![(ledger_mcp::local::REFUSED_HEADER, "unauthorized".into())],
            body: "these books need a token: the human makes one with `wanderbooks token create`"
                .into(),
            cut: false,
        }
    }

    /// INVARIANT 11: with no token because the keyring couldn't be read,
    /// the agent is told that, never to make a token; with a keyring that
    /// simply has none, the door's words.
    #[tokio::test]
    async fn a_keyring_that_cant_be_read_is_said_never_make_a_token() {
        let server = Server::start();
        let dir = tempfile::tempdir().unwrap();
        let (b, _rx, keyring) = bridge_with_keyring(&server.socket, dir.path());
        let call: Value = serde_json::from_str(CALL).unwrap();
        let params = json!({"protocolVersion": "2025-06-18", "capabilities": {}});
        let unreachable = keyring.join(crate::tokens::TEST_UNREACHABLE);
        std::fs::write(&unreachable, "").unwrap();
        server.script([no_token_at_the_door(), no_token_at_the_door()]);
        let said = what(&b.send(&call, &up(1)).await);
        assert!(
            said.contains("keyring is locked or unreachable")
                && said.contains("token itself is fine")
                && said.contains("token check --book acme"),
            "{said}"
        );
        assert!(!said.contains("token create"), "{said}");
        let refused = b
            .initialize_upstream(&params, 1)
            .await
            .unwrap_err()
            .unwrap();
        assert!(refused.contains("token itself is fine"), "{refused}");
        assert!(
            server
                .seen()
                .iter()
                .all(|s| !s.headers.contains_key("authorization")),
            "no bearer was sent"
        );
        // Two items for the token: that, and how to keep one again.
        std::fs::remove_file(&unreachable).unwrap();
        let account = b.keyring.get().unwrap().0.clone();
        crate::tokens::keeper(dir.path(), &b.knobs)
            .set(&account, "wb_one")
            .unwrap();
        let other = keyring.join("login");
        std::fs::create_dir_all(&other).unwrap();
        for entry in std::fs::read_dir(keyring.join(crate::tokens::TEST_DEFAULT)).unwrap() {
            std::fs::write(other.join(entry.unwrap().file_name()), "wb_two").unwrap();
        }
        server.script([no_token_at_the_door()]);
        let said = what(&b.send(&call, &up(1)).await);
        assert!(
            said.contains("2 items") && said.contains("--replace"),
            "{said}"
        );
        assert!(!said.contains("wb_"), "{said}");
        // A keyring that has none: the door's own words.
        std::fs::remove_dir_all(&keyring).unwrap();
        server.script([no_token_at_the_door()]);
        let said = what(&b.send(&call, &up(1)).await);
        assert!(said.contains("wanderbooks token create"), "{said}");
    }

    /// INVARIANT 11: before each new session the keyring is read again,
    /// even with a token held, and the keyring's token is the one used.
    #[tokio::test]
    async fn each_new_session_reads_the_keyring_again() {
        let server = Server::start();
        let dir = tempfile::tempdir().unwrap();
        let (b, _rx, _keyring) = bridge_with_keyring(&server.socket, dir.path());
        *b.token.lock().unwrap() = Some("wb_held".into());
        let account = b.keyring.get().unwrap().0.clone();
        let keeper = crate::tokens::keeper(dir.path(), &b.knobs);
        keeper.set(&account, "wb_kept").unwrap();
        let params = json!({"protocolVersion": "2025-06-18", "capabilities": {}});
        let result = json!({"protocolVersion": "2025-06-18", "capabilities": {"tools": {}},
            "serverInfo": {"name": "wanderbooks", "version": "0"}});
        let opened = || {
            [
                json_reply(
                    200,
                    json!({"jsonrpc": "2.0", "id": "x", "result": result.clone()}),
                ),
                reply(202, "text/plain", ""),
            ]
        };
        server.script(opened());
        b.initialize_upstream(&params, 1).await.unwrap();
        let bearers = |server: &Server| -> Vec<String> {
            server
                .seen()
                .iter()
                .filter_map(|s| s.headers.get("authorization").cloned())
                .collect()
        };
        assert_eq!(bearers(&server), ["Bearer wb_kept", "Bearer wb_kept"]);
        // A keyring that can't be read then: the token held goes on.
        std::fs::write(
            dir.path()
                .join("test-keyring")
                .join(crate::tokens::TEST_UNREACHABLE),
            "",
        )
        .unwrap();
        server.script(opened());
        b.initialize_upstream(&params, 2).await.unwrap();
        assert_eq!(bearers(&server)[2..], ["Bearer wb_kept", "Bearer wb_kept"]);
        assert!(b.keyring_refusal().is_none(), "a token is held");
    }

    #[tokio::test]
    async fn the_tool_list_is_read_from_its_answer() {
        let server = Server::start();
        let (b, _rx) = bridge_at(&server.socket, knobs(1000, 1000));
        let tools = json!({"tools": [{"name": "books.start"}]});
        server.script([
            json_reply(200, json!({"jsonrpc": "2.0", "id": "x", "result": tools})),
            reply(500, "text/plain", "boom"),
        ]);
        assert_eq!(b.tool_list(&up(1)).await, Some(tools));
        assert_eq!(b.tool_list(&up(1)).await, None);
    }

    #[test]
    fn a_request_is_waiting_sent_or_cancelled() {
        let (b, mut rx) = bridge_at(Path::new("/nonexistent"), knobs(10, 10));
        let state = |id: i64| {
            b.pending
                .lock()
                .unwrap()
                .get(&id_key(&json!(id)))
                .map(|p| match p {
                    Pending::Waiting => "waiting",
                    Pending::Sent { cancelled: false } => "sent",
                    Pending::Sent { cancelled: true } => "sent, cancelled",
                    Pending::Cancelled => "cancelled",
                })
        };
        let cancel =
            |id: i64| json!({"method": "notifications/cancelled", "params": {"requestId": id}});
        // Cancelled while waiting: never sent, nothing to tell upstream.
        b.waiting(&json!(1));
        assert!(!b.cancel(&cancel(1)));
        assert_eq!(state(1), Some("cancelled"));
        assert!(!b.may_send(&json!(1)));
        // Cancelled once sent: upstream hears; not run, it isn't sent again.
        b.waiting(&json!(2));
        assert!(b.may_send(&json!(2)));
        assert_eq!(state(2), Some("sent"));
        assert!(b.cancel(&cancel(2)));
        assert_eq!(state(2), Some("sent, cancelled"));
        b.requeue(&json!(2));
        assert_eq!(state(2), Some("cancelled"));
        assert!(!b.may_send(&json!(2)));
        // Sent and not run: waiting again, sent again.
        b.waiting(&json!(3));
        assert!(b.may_send(&json!(3)));
        b.requeue(&json!(3));
        assert_eq!(state(3), Some("waiting"));
        assert!(b.may_send(&json!(3)));
        // Answered: forgotten, its id free again.
        b.answer(&json!(3), json!({"id": 3, "result": {}}));
        assert_eq!(state(3), None);
        assert_eq!(rx.try_recv().unwrap(), json!({"id": 3, "result": {}}));
        // A cancel naming nothing goes on as it is.
        assert!(b.cancel(&json!({"method": "notifications/cancelled", "params": {}})));
        assert!(b.cancel(&cancel(99)));
    }

    /// The next message to the agent, within `ms`.
    async fn told(rx: &mut mpsc::UnboundedReceiver<Value>, ms: u64) -> Option<Value> {
        tokio::time::timeout(Duration::from_millis(ms), rx.recv())
            .await
            .ok()
            .flatten()
    }

    #[tokio::test]
    async fn a_ping_is_the_bridges_to_answer_while_the_books_arent_ready() {
        let server = Server::start();
        // A long queue: an answer within a second is the bridge's own.
        let (b, mut rx) = bridge_at(&server.socket, knobs(30_000, 1000));
        let ping = json!({"jsonrpc": "2.0", "id": 1, "method": "ping"});
        b.clone().request(ping.clone()).await;
        assert_eq!(
            told(&mut rx, 1000).await,
            Some(json!({"jsonrpc": "2.0", "id": 1, "result": {}}))
        );
        // Ready, and the server didn't run it: answered by the bridge too.
        ready(&b, 1);
        server.script([reply(
            503,
            "text/plain",
            "the books are closing: this request wasn't run",
        )]);
        let task = tokio::spawn(b.clone().request_on(ping.clone(), json!(1), "ping"));
        assert_eq!(
            told(&mut rx, 1000).await,
            Some(json!({"jsonrpc": "2.0", "id": 1, "result": {}}))
        );
        task.abort();
        // The books never ready in time: still answered, not an error.
        let (b, mut rx) = bridge_at(&server.socket, knobs(50, 1000));
        b.clone().request_on(ping, json!(2), "ping").await;
        assert_eq!(
            told(&mut rx, 1000).await,
            Some(json!({"jsonrpc": "2.0", "id": 2, "result": {}}))
        );
    }

    #[tokio::test]
    async fn a_call_not_run_isnt_answered_as_if_it_had_been() {
        let server = Server::start();
        let (b, mut rx) = bridge_at(&server.socket, knobs(300, 1000));
        ready(&b, 1);
        server.script([reply(
            503,
            "text/plain",
            "the books are closing: this request wasn't run",
        )]);
        let call: Value = serde_json::from_str(CALL).unwrap();
        b.waiting(&json!(7));
        b.clone().request_on(call, json!(7), "tools/call").await;
        let got = told(&mut rx, 2000).await.unwrap();
        assert!(
            got["error"]["message"]
                .as_str()
                .unwrap()
                .contains("didn't open"),
            "{got}"
        );
    }

    #[tokio::test]
    async fn an_answer_that_isnt_this_calls_says_it_may_have_run() {
        let server = Server::start();
        let (b, mut rx) = bridge_at(&server.socket, knobs(1000, 1000));
        ready(&b, 1);
        let call: Value = serde_json::from_str(CALL).unwrap();
        let note = json!({"jsonrpc": "2.0", "method": "notifications/progress"});
        let not_ours = [
            // Only a notification.
            format!("data: {note}\n\n"),
            // An answer to another request.
            format!(
                "data: {}\n\n",
                json!({"jsonrpc": "2.0", "id": 8, "result": {}})
            ),
            // Its id, but neither a result nor an error.
            format!(
                "data: {}\n\n",
                json!({"jsonrpc": "2.0", "id": 7, "method": "x"})
            ),
        ];
        for body in not_ours {
            server.script([reply(200, "text/event-stream", &body)]);
            b.waiting(&json!(7));
            b.clone()
                .request_on(call.clone(), json!(7), "tools/call")
                .await;
            let mut last = None;
            while let Some(m) = told(&mut rx, 200).await {
                last = Some(m);
            }
            let last = last.unwrap();
            assert_eq!(last["error"]["message"], MAYBE_RAN, "{body}");
        }
        // An error answer is this call's answer.
        let err = json!({"jsonrpc": "2.0", "id": 7, "error": {"code": -1, "message": "no"}});
        server.script([json_reply(200, err.clone())]);
        b.waiting(&json!(7));
        b.clone().request_on(call, json!(7), "tools/call").await;
        assert_eq!(told(&mut rx, 1000).await, Some(err));
        // A tool list given to the agent is remembered (INVARIANT 5).
        let tools = json!({"tools": [{"name": "books.start"}]});
        server.script([json_reply(
            200,
            json!({"jsonrpc": "2.0", "id": 9, "result": tools}),
        )]);
        let list = json!({"jsonrpc": "2.0", "id": 9, "method": "tools/list"});
        b.waiting(&json!(9));
        b.clone().request_on(list, json!(9), "tools/list").await;
        assert!(told(&mut rx, 1000).await.is_some());
        assert_eq!(*b.tools_seen.lock().unwrap(), Some(tools));
    }

    #[tokio::test]
    async fn an_initialize_is_refused_when_the_books_are_closed_or_failed() {
        let (b, mut rx) = bridge_at(Path::new("/nonexistent"), knobs(1000, 200));
        let init = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"protocolVersion": "2025-06-18", "capabilities": {}}});
        b.link.send_replace(Link::Closed);
        b.clone().initialize(init.clone(), json!(1)).await;
        assert_eq!(
            told(&mut rx, 500).await.unwrap()["error"]["message"],
            CLOSED_BY_OWNER
        );
        b.link.send_replace(Link::Failed("it broke".into()));
        b.clone().initialize(init.clone(), json!(2)).await;
        assert_eq!(
            told(&mut rx, 500).await.unwrap()["error"]["message"],
            "it broke"
        );
        // Opening still: the bridge's own answer.
        b.link.send_replace(Link::Opening);
        b.clone().initialize(init, json!(3)).await;
        let got = told(&mut rx, 1000).await.unwrap();
        assert!(got["result"]["capabilities"].is_object(), "{got}");
    }

    #[test]
    fn only_the_session_found_gone_is_renewed() {
        let (b, _rx) = bridge_at(Path::new("/nonexistent"), knobs(10, 10));
        ready(&b, 2);
        b.session_gone(1);
        assert!(
            matches!(&*b.link.borrow(), Link::Ready(_)),
            "an older session"
        );
        b.session_gone(2);
        assert!(matches!(&*b.link.borrow(), Link::Opening));
        ready(&b, 3);
        b.session_down(2);
        assert!(matches!(&*b.link.borrow(), Link::Ready(_)));
        b.session_down(3);
        assert!(matches!(&*b.link.borrow(), Link::Opening));
    }

    #[tokio::test]
    async fn the_servers_socket_must_answer_and_be_ours() {
        let server = Server::start();
        let (b, _rx) = bridge_at(&server.socket, knobs(10, 10));
        assert!(b.socket_is_ours().is_ok());
        let (b, _rx) = bridge_at(&server.socket.with_file_name("gone.sock"), knobs(10, 10));
        assert!(matches!(b.socket_is_ours(), Err(Owner::Unreachable)));
    }

    #[tokio::test]
    async fn the_agents_session_ending_ends_the_servers() {
        let server = Server::start();
        let (b, _rx) = bridge_at(&server.socket, knobs(10, 10));
        // No session: nothing sent.
        b.end_session().await;
        let mut sessionless = up(1);
        sessionless.session = None;
        b.link.send_replace(Link::Ready(Arc::new(sessionless)));
        b.end_session().await;
        assert!(server.seen().is_empty());
        server.script([reply(200, "text/plain", "")]);
        ready(&b, 1);
        b.end_session().await;
        let seen = server.seen();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].method, "DELETE");
        assert_eq!(
            seen[0].headers.get("mcp-session-id").map(String::as_str),
            Some("s1")
        );
    }

    #[tokio::test]
    async fn the_agent_hears_of_changed_tools_only_after_a_restart() {
        let server = Server::start();
        let (b, mut rx) = bridge_at(&server.socket, knobs(10, 10));
        let a = json!({"tools": [{"name": "a"}]});
        let bb = json!({"tools": [{"name": "b"}]});
        // Nothing seen yet: nothing asked.
        b.after_new_session(&up(2)).await;
        assert!(server.seen().is_empty());
        *b.tools_seen.lock().unwrap() = Some(a.clone());
        // The first session: nothing asked.
        b.after_new_session(&up(1)).await;
        assert!(server.seen().is_empty());
        // The same tools after a restart: nothing said.
        server.script([json_reply(
            200,
            json!({"jsonrpc": "2.0", "id": "x", "result": a}),
        )]);
        b.after_new_session(&up(2)).await;
        assert_eq!(server.seen().len(), 1);
        assert!(told(&mut rx, 100).await.is_none());
        // Others: said, and remembered.
        server.script([json_reply(
            200,
            json!({"jsonrpc": "2.0", "id": "x", "result": bb}),
        )]);
        b.after_new_session(&up(3)).await;
        assert_eq!(
            told(&mut rx, 500).await,
            Some(json!({"jsonrpc": "2.0", "method": "notifications/tools/list_changed"}))
        );
        assert_eq!(*b.tools_seen.lock().unwrap(), Some(bb));
    }

    #[tokio::test]
    async fn every_message_the_agent_sends_is_handled_for_what_it_is() {
        let server = Server::start();
        let (b, mut rx) = bridge_at(&server.socket, knobs(30_000, 10));
        let tasks = tokio_util::task::TaskTracker::new();
        let invalid = |id: Value| move |m: &Value| m["id"] == id && m["error"]["code"] == -32600;
        // A batch: each request in it refused; notifications in it ignored.
        b.handle(
            json!([{"jsonrpc": "2.0", "id": 1, "method": "ping"},
            {"jsonrpc": "2.0", "method": "notifications/x"},
            {"jsonrpc": "2.0", "id": 2, "method": "ping"}]),
            &tasks,
        );
        let got = told(&mut rx, 200).await.unwrap();
        assert_eq!(got.as_array().unwrap().len(), 2, "{got}");
        b.handle(
            json!([{"jsonrpc": "2.0", "method": "notifications/x"}]),
            &tasks,
        );
        assert!(
            told(&mut rx, 100).await.is_none(),
            "notifications only: nothing"
        );
        b.handle(json!([]), &tasks);
        assert!(invalid(Value::Null)(&told(&mut rx, 200).await.unwrap()));
        // Ids: a string or a whole number.
        b.handle(json!({"jsonrpc": "2.0", "id": 1.5, "method": "x"}), &tasks);
        assert!(invalid(Value::Null)(&told(&mut rx, 200).await.unwrap()));
        b.handle(json!({"jsonrpc": "2.0", "id": "a", "method": "x"}), &tasks);
        b.handle(json!({"jsonrpc": "2.0", "id": 3, "method": "x"}), &tasks);
        b.handle(
            json!({"jsonrpc": "2.0", "id": u64::MAX, "method": "x"}),
            &tasks,
        );
        assert!(told(&mut rx, 100).await.is_none(), "valid ids are taken");
        // An id still waiting is refused.
        b.handle(json!({"jsonrpc": "2.0", "id": 3, "method": "x"}), &tasks);
        assert!(invalid(json!(3))(&told(&mut rx, 200).await.unwrap()));
        // Answers (a result, or an error) are the agent's; nothing said.
        b.handle(json!({"jsonrpc": "2.0", "id": 5, "result": {}}), &tasks);
        b.handle(
            json!({"jsonrpc": "2.0", "id": 6, "error": {"code": 1, "message": "x"}}),
            &tasks,
        );
        assert!(told(&mut rx, 100).await.is_none());
        // Neither a method nor an answer.
        b.handle(json!({"jsonrpc": "2.0", "id": 9}), &tasks);
        assert!(invalid(json!(9))(&told(&mut rx, 200).await.unwrap()));
        b.handle(json!("hello"), &tasks);
        assert!(invalid(Value::Null)(&told(&mut rx, 200).await.unwrap()));
        // Notifications: initialized is the bridge's own; a cancel of a
        // call not sent stays here; one of a call sent, and others, go up.
        ready(&b, 1);
        server.script([reply(202, "text/plain", ""), reply(202, "text/plain", "")]);
        b.handle(
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            &tasks,
        );
        b.waiting(&json!(40));
        b.handle(
            json!({"jsonrpc": "2.0", "method": "notifications/cancelled",
            "params": {"requestId": 40}}),
            &tasks,
        );
        b.waiting(&json!(41));
        assert!(b.may_send(&json!(41)));
        b.handle(
            json!({"jsonrpc": "2.0", "method": "notifications/cancelled",
            "params": {"requestId": 41}}),
            &tasks,
        );
        b.handle(
            json!({"jsonrpc": "2.0", "method": "notifications/progress"}),
            &tasks,
        );
        tokio::time::sleep(Duration::from_millis(300)).await;
        // (The requests taken above went up too, once ready.)
        let notes: Vec<String> = server
            .seen()
            .into_iter()
            .map(|s| s.body)
            .filter(|b| b.contains("notifications/"))
            .collect();
        assert_eq!(notes.len(), 2, "{notes:?}");
        assert!(notes.iter().any(|b| b.contains("\"requestId\":41")));
        assert!(notes.iter().any(|b| b.contains("notifications/progress")));
        tasks.close();
    }

    /// Read `input` as the agent's, at most `max` bytes a message: what
    /// the agent was told, and how it ended.
    async fn read_all(input: &[u8], max: usize) -> (Vec<Value>, InputEnd) {
        let (b, mut rx) = bridge_at(Path::new("/nonexistent"), knobs(10, 10));
        let tasks = tokio_util::task::TaskTracker::new();
        let stop = Notify::new();
        let mut reader = tokio::io::BufReader::new(input);
        let end = read_messages(&mut reader, max, &b, &tasks, &stop).await;
        let mut out = Vec::new();
        while let Ok(m) = rx.try_recv() {
            out.push(m);
        }
        (out, end)
    }

    fn codes(told: &[Value]) -> Vec<i64> {
        told.iter()
            .map(|m| m["error"]["code"].as_i64().unwrap_or(0))
            .collect()
    }

    #[tokio::test]
    async fn the_agents_input_is_read_message_by_message() {
        // A malformed line is answered and the next one read; a blank one
        // is skipped; the last one needs no newline.
        let (told, end) = read_all(b"nonsense\n\n\xff\n[]\n[]", 1024).await;
        assert_eq!(end, InputEnd::Closed);
        assert_eq!(codes(&told), [-32700, -32700, -32600, -32600]);
        // At the limit, with no newline: the input's last line, taken.
        let (told, _) = read_all(b"[]      ", 8).await;
        assert_eq!(codes(&told), [-32600]);
        // At the limit, with its newline: taken too.
        let (told, _) = read_all(b"[]      \n[]\n", 8).await;
        assert_eq!(codes(&told), [-32600, -32600]);
        // One byte over: refused, the rest of it skipped, then on.
        let (told, _) = read_all(b"[]       and more\n[]\n", 8).await;
        assert_eq!(codes(&told), [SERVER_ERROR, -32600]);
        assert!(
            told[0]["error"]["message"]
                .as_str()
                .unwrap()
                .contains("too large")
        );
        // Too large and never ended: refused, then the input's end.
        let (told, end) = read_all(b"[]       and more", 8).await;
        assert_eq!(codes(&told), [SERVER_ERROR]);
        assert_eq!(end, InputEnd::Closed);
        // Nothing at all.
        let (told, end) = read_all(b"", 8).await;
        assert!(told.is_empty());
        assert_eq!(end, InputEnd::Closed);
    }

    #[tokio::test]
    async fn a_signal_ends_the_reading() {
        let (b, _rx) = bridge_at(Path::new("/nonexistent"), knobs(10, 10));
        let tasks = tokio_util::task::TaskTracker::new();
        let stop = Notify::new();
        // Input that never ends.
        let (_keep, quiet) = tokio::io::duplex(64);
        let mut reader = tokio::io::BufReader::new(quiet);
        stop.notify_one();
        let end = read_messages(&mut reader, 64, &b, &tasks, &stop).await;
        assert_eq!(end, InputEnd::Signalled);
    }

    #[tokio::test]
    async fn the_rest_of_a_line_too_long_is_skipped() {
        let mut r = tokio::io::BufReader::with_capacity(4, &b"abcdefgh\nnext\n"[..]);
        assert!(skip_line(&mut r).await, "the line ended");
        let mut rest = String::new();
        r.read_to_string(&mut rest).await.unwrap();
        assert_eq!(rest, "next\n");
        let mut r = tokio::io::BufReader::new(&b"no end"[..]);
        assert!(!skip_line(&mut r).await, "the input ended first");
        let mut r = tokio::io::BufReader::new(&b""[..]);
        assert!(!skip_line(&mut r).await);
    }
}
