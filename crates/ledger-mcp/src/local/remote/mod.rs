//! The remote door: how web assistants (Claude and ChatGPT on the web,
//! others best effort) reach a company, and the sign-in behind it (plan
//! P15 unit B1; D-81, D-83, D-86, D-88).
//!
//! ```text
//! GOAL: a web assistant reaches ONE company's tools over HTTPS only after
//!       the person at the machine has said yes in the app's window, with
//!       only the permissions they gave, and loses that access the moment
//!       they revoke it; and nothing a stranger on the internet sends can
//!       approve a connection, reach another company, fill the server's
//!       memory or disk, or make the server believe the caller is local.
//! SOURCE: WB-remote-1 to WB-remote-10 (docs/spec/claims/remote.md; the
//!         ten sign-in rules of plan/p15-app-and-relay.md section 4); D-83
//!         (the app is its own sign-in server), D-86 (read and draft by
//!         default), D-88 (every write is one transaction, once); OAuth
//!         2.1 (draft-ietf-oauth-v2-1), RFC 7636 (PKCE), RFC 7591
//!         (dynamic registration), RFC 8414 and RFC 9728 (metadata),
//!         RFC 8707 (resource indicators), RFC 6749 section 10.4 and
//!         RFC 9700 (refresh token rotation and reuse), the IETF draft
//!         for OAuth client ID metadata documents; the MCP authorization
//!         specification, revision 2026-07-28.
//! INVARIANTS:
//!   1. Off unless asked: the door listens on 127.0.0.1 only after an
//!      admin request (`RemoteStart`) and stops on `RemoteStop` or when the
//!      server closes. It serves a company only while that company is
//!      allowed (`RemoteAllow`) AND open. Every connection begins with the
//!      relay's PROXY header (proxy.rs); the address in it is the caller's,
//!      is what the access record and the limits use, and is never taken
//!      from an HTTP header. Nothing on this door is ever treated as local:
//!      every request needs a token, and the local socket's same-user check
//!      is not a thing a request here can claim.
//!   2. One public address: the issuer is the URL the app gave `RemoteStart`
//!      and no other. Every address in every metadata document is built
//!      from it, never from the request's Host; a request whose Host is
//!      not its host is refused (421). A token, code or refresh token made
//!      for one issuer is refused under another.
//!   3. Unsigned is refused: every request to a company's path with no
//!      token, a wrong one, or an expired one is a 401 with a
//!      `WWW-Authenticate` that names the protected-resource metadata,
//!      `initialize` included. Such a request is counted, not written to
//!      the books' access record (a stranger can't fill the disk).
//!   4. Three ways to introduce a client: a client ID metadata document (an
//!      https URL, fetched with the checks of cimd.rs), dynamic
//!      registration, and a client id and secret the app made. A redirect
//!      address must match one registered, exactly; a registration may
//!      carry up to MAX_REDIRECTS. PKCE with S256 is required; the
//!      `resource` must be the one company's address; a code is single-use
//!      and lives CODE_TTL; the consent ends in a 302.
//!   5. No approval without the person. `/authorize` never approves: it
//!      parks the request (PENDING_TTL), and only an admin request, which
//!      the owner-only admin socket carries, turns a parked request into a
//!      code. A parked request is named by two unrelated random ids: the
//!      one the window shows, and the one only the browser's page holds.
//!      Neither a stranger's guess nor a replay approves anything: the
//!      second id only reads the outcome, once.
//!   6. An approval is the owner's decision about a product, a company and
//!      a set of permissions, and survives a product registering again: it
//!      is one token of the books' principals (so "Who can reach these
//!      books" lists it, and revoking it by id ends every access and
//!      refresh token made under it), plus a record here naming the
//!      product. Access tokens live ACCESS_TTL; refresh tokens REFRESH_TTL
//!      since last use, rotated at every use, and a refresh token used
//!      twice ends its whole family.
//!   7. Secrets are never stored: codes, access tokens, refresh tokens and
//!      client secrets are kept as SHA-256 only, and compared by hash (or
//!      in constant time where a hash is compared to a hash).
//!   8. Every table is bounded (MAX_* below) and expires: registered
//!      clients unused for CLIENT_IDLE, pending requests, codes, access
//!      tokens, refresh tokens. A stranger registering clients, or asking
//!      to sign in, can fill none of them; a failure to sign in is never
//!      ground for banning an address (vendors share addresses), only the
//!      state a request creates is bounded, and an approval's own calls
//!      are limited.
//!   9. No company mixing: a token for company A, presented at company
//!      B's path, is refused (403, recorded as the token's own refusal); a
//!      code and a refresh token carry their company and never issue a
//!      token for another.
//!  10. Remote permissions: an approval grants read and draft; posting is
//!      raised per approval by the owner (`RemoteSetPost`) and lowered the
//!      same way; the access record's admin scope is never granted here.
//!      Every call goes through the one write path (server.rs `call_tool`:
//!      a write needs its request key, D-88) and is stateless and plain
//!      JSON: no session is kept for a remote caller.
//! SCENARIOS:
//!   1. A client registers, asks to sign in, the person allows it in the
//!      window, the browser is sent back with a code, the code gives an
//!      access token and a refresh token, and the access token reads the
//!      books. The same with a metadata-document client and with a typed
//!      client id and secret.
//!   2. Every refusal: wrong redirect, no PKCE, a code used twice, a
//!      refresh token used twice, a token for another issuer, an expired
//!      token, an unsigned `initialize`, a token for another company.
//!   3. The person refuses, or no one decides in time: the browser is sent
//!      back with `access_denied`; nothing was made.
//!   4. The person revokes the connection in the list: its access token
//!      stops at once, its refresh token is refused.
//!   5. A caller's forged PROXY header, `X-Forwarded-For`, or second header
//!      changes nothing: the recorded address is the first header's.
//! ```

mod cimd;
mod http;
mod pages;
pub(super) mod proxy;
#[cfg(test)]
mod tests;

pub(super) use cimd::{ClientDocs, WebDocs};
pub(super) use http::{Via, start_door, stop_door, unauthorized};

use super::Books;
use crate::admin::{ApprovalInfo, PendingInfo, RemoteInfo};
use base64::Engine as _;
use ledger_store::auth::{Scope, Token};
use ledger_store::root::BookId;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use tokio_util::sync::CancellationToken;

/// How long an access token lives (seconds): short, so a revoke that
/// somehow missed would still end soon, and the refresh token is the
/// long-lived thing.
pub const ACCESS_TTL: i64 = 600;
/// How long a code lives, and that it is good once.
pub const CODE_TTL: i64 = 60;
/// How long a request waits for the person.
pub const PENDING_TTL: i64 = 300;
/// How long a refresh token lives since it was last used: 60 days, so an
/// assistant that comes back after the app was closed for weeks still
/// works (rule 6).
pub const REFRESH_TTL: i64 = 60 * 24 * 3600;
/// How long a registration nobody used for a sign-in is kept.
pub const CLIENT_IDLE: i64 = 3600;
/// How long a fetched client metadata document is believed.
pub const CIMD_TTL: i64 = 3600;
/// Most registered clients (dynamic, document and typed), together.
pub const MAX_CLIENTS: usize = 2000;
/// Most redirect addresses one client may carry.
pub const MAX_REDIRECTS: usize = 20;
/// Most requests waiting for the person, and from one address.
pub const MAX_PENDING: usize = 20;
/// Most requests one address may have waiting.
pub const MAX_PENDING_PER_ADDRESS: usize = 5;
/// Most codes, access tokens and refresh tokens held.
pub const MAX_CODES: usize = 2000;
/// Most access tokens held.
pub const MAX_ACCESS: usize = 20_000;
/// Most refresh tokens (live and used) held.
pub const MAX_REFRESH: usize = 20_000;
/// Most approvals (connections) held.
pub const MAX_APPROVALS: usize = 500;
/// Most refresh families of one approval.
pub const MAX_FAMILIES: usize = 10;
/// Most typed client ids the app has made.
pub const MAX_MANUAL: usize = 20;
/// Most keys the app has made for products that take only a key.
pub const MAX_KEYS: usize = 50;
/// Registrations one address may make in an hour.
const REGISTER_PER_ADDRESS: u32 = 30;
/// Registrations the door takes in an hour, together.
const REGISTER_TOTAL: u32 = 600;
/// Sign-in starts one address may make in ten minutes.
const AUTHORIZE_PER_ADDRESS: u32 = 60;
/// Client metadata fetches in a minute, together and from one address.
const FETCH_PER_MINUTE: u32 = 30;
const FETCH_PER_ADDRESS: u32 = 6;
/// Token requests a client may make in a minute.
const TOKEN_PER_CLIENT: u32 = 120;
/// Calls an approval may make in a minute.
pub const CALLS_PER_MINUTE: u32 = 600;
/// The limiter's own table is bounded too.
const MAX_LIMITS: usize = 10_000;
/// How long a used code is remembered, to end the family a replay shows.
const USED_CODE_KEPT: i64 = 600;
/// The persisted file, in the root's `auth/`.
pub const FILE: &str = "remote.json";

/// The scopes a web approval gets by default (D-86), and with posting
/// raised. Admin is never among them.
pub fn scopes_for(post: bool) -> BTreeSet<Scope> {
    let mut s = BTreeSet::from([Scope::Read, Scope::Write]);
    if post {
        s.insert(Scope::Post);
    }
    s
}

/// How a client introduced itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// A client ID metadata document: its id is the https URL.
    Document,
    /// Dynamic registration.
    Registered,
    /// A client id and secret the app made, typed into the product.
    Typed,
}

/// A client the door knows. Never a secret: only the secret's hash.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClientRec {
    pub id: String,
    pub kind: Kind,
    /// What it calls itself: asserted, cleaned, never trusted.
    pub name: String,
    /// The redirect addresses it may be sent back to, exactly.
    pub redirects: Vec<String>,
    /// The SHA-256 (hex) of its secret, when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_sha256: Option<String>,
    pub created: i64,
    pub last_seen: i64,
    /// Whether a sign-in was ever begun with it.
    pub used: bool,
    /// When its document was fetched (documents only).
    #[serde(default)]
    pub fetched: i64,
}

/// The owner's decision: a product, a company, permissions. One token of
/// the books' principals carries what it may do.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Approval {
    pub id: String,
    pub book: String,
    /// What identifies the product across registrations.
    pub product: String,
    /// What the window and the list call it.
    pub label: String,
    /// The principals' token that carries the permissions.
    pub token_name: String,
    pub token_id: String,
    pub created: i64,
    pub last_used: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct RefreshRec {
    hash: String,
    family: String,
    approval: String,
    client: String,
    issuer: String,
    exp: i64,
    used: bool,
    created: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct KeyRec {
    token_id: String,
    book: String,
    created: i64,
}

#[derive(Default, Serialize, Deserialize)]
struct Saved {
    version: u32,
    clients: Vec<ClientRec>,
    approvals: Vec<Approval>,
    refresh: Vec<RefreshRec>,
    keys: Vec<KeyRec>,
}

struct Code {
    client: String,
    approval: String,
    family: String,
    book: String,
    issuer: String,
    redirect: String,
    challenge: String,
    exp: i64,
}

struct AccessRec {
    approval: String,
    family: String,
    book: String,
    issuer: String,
    exp: i64,
}

enum Settled {
    Waiting,
    /// The person said yes: the code the browser is sent back with.
    Allowed(String),
    Refused,
}

struct Pending {
    /// What the window names it by.
    id: String,
    /// What only the browser's page holds.
    wait: String,
    client: String,
    client_name: String,
    product: Option<String>,
    redirect: String,
    state: Option<String>,
    challenge: String,
    book: String,
    issuer: String,
    from: String,
    wants_post: bool,
    created: i64,
    exp: i64,
    settled: Settled,
}

#[derive(Default)]
struct Inner {
    loaded: bool,
    clients: BTreeMap<String, ClientRec>,
    approvals: BTreeMap<String, Approval>,
    refresh: HashMap<String, RefreshRec>,
    keys: BTreeMap<String, KeyRec>,
    codes: HashMap<String, Code>,
    used_codes: HashMap<String, (String, i64)>,
    access: HashMap<String, AccessRec>,
    pending: Vec<Pending>,
    limits: HashMap<String, (i64, u32)>,
    failed_docs: HashMap<String, i64>,
    unsigned: u64,
    last_sweep: i64,
}

/// The door while it is on.
pub(super) struct Door {
    /// The one public address, no trailing slash.
    pub issuer: String,
    /// Its host as a `Host` header says it.
    pub host: String,
    pub listening: SocketAddr,
    pub stop: CancellationToken,
}

/// Why a token is refused at the door.
pub(super) struct Denied {
    pub code: &'static str,
    pub why: String,
    /// The token was presented but is not good: `error="invalid_token"`.
    pub invalid_token: bool,
}

/// An OAuth error answer.
pub(super) struct OauthError {
    pub status: u16,
    pub error: &'static str,
    pub description: String,
}

impl OauthError {
    fn new(status: u16, error: &'static str, description: impl Into<String>) -> Self {
        Self {
            status,
            error,
            description: description.into(),
        }
    }
}

/// What the remote door holds (invariants above).
pub(super) struct Remote {
    inner: Mutex<Inner>,
    door: RwLock<Option<Arc<Door>>>,
    allowed: RwLock<BTreeSet<String>>,
    clock: RwLock<Arc<dyn Fn() -> i64 + Send + Sync>>,
    docs: RwLock<Arc<dyn ClientDocs>>,
    dir: Mutex<Option<PathBuf>>,
    version: AtomicU64,
    written: Mutex<u64>,
}

fn sha_hex(text: &str) -> String {
    hex_of(&Sha256::digest(text.as_bytes()))
}

fn hex_of(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// `n` random bytes as unpadded base64url, behind `prefix`.
fn random(prefix: &str, n: usize) -> Result<String, OauthError> {
    let mut buf = vec![0u8; n];
    getrandom::getrandom(&mut buf)
        .map_err(|_| OauthError::new(500, "server_error", "the system gave no randomness"))?;
    Ok(format!(
        "{prefix}{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf)
    ))
}

/// A name, cleaned for showing and for storing: no control characters,
/// single spaces, at most `max` characters.
pub(super) fn clean(text: &str, max: usize) -> String {
    let mut out = String::new();
    for word in text.split_whitespace() {
        let word: String = word.chars().filter(|c| !c.is_control()).collect();
        if word.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&word);
        if out.chars().count() >= max {
            break;
        }
    }
    out.chars().take(max).collect()
}

/// A redirect address, parsed if it may be one: https, or http on the
/// loopback, or a native app's own scheme; never a fragment or userinfo,
/// never a script or file scheme.
pub(super) fn valid_redirect(text: &str) -> Option<reqwest::Url> {
    if text.len() > 2048 || text.chars().any(char::is_control) {
        return None;
    }
    let url = reqwest::Url::parse(text).ok()?;
    if url.fragment().is_some() || !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    match url.scheme() {
        "https" => {
            let has_host = url.host_str().is_some_and(|h| !h.is_empty());
            has_host.then_some(url)
        }
        "http" => {
            matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]")).then_some(url)
        }
        "javascript" | "data" | "file" | "vbscript" | "about" | "blob" | "ftp" | "ws" | "wss" => {
            None
        }
        s if s.len() >= 2 => Some(url),
        _ => None,
    }
}

/// What a redirect address shows the person: its host, or its scheme.
pub(super) fn redirect_host(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .map(|u| match u.host_str() {
            Some(h) => h.to_string(),
            None => format!("{}:", u.scheme()),
        })
        .unwrap_or_else(|| "?".to_string())
}

/// The products whose own redirect hosts we recognise (D-81: Claude and
/// ChatGPT). The name comes from where the browser is sent back, which a
/// client cannot fake, never from what it calls itself.
const KNOWN: &[(&str, &[&str])] = &[
    ("Claude on the web", &["claude.ai", "claude.com"]),
    (
        "ChatGPT on the web",
        &["chatgpt.com", "openai.com", "platform.openai.com"],
    ),
];

/// The product whose hosts every redirect address is on, if one.
pub(super) fn known_product(redirects: &[String]) -> Option<&'static str> {
    KNOWN.iter().find_map(|(label, hosts)| {
        let all = !redirects.is_empty()
            && redirects.iter().all(|r| {
                reqwest::Url::parse(r).ok().is_some_and(|u| {
                    u.scheme() == "https"
                        && u.host_str()
                            .is_some_and(|h| hosts.contains(&h.to_ascii_lowercase().as_str()))
                })
            });
        all.then_some(*label)
    })
}

/// What identifies a product across registrations (rule 6).
fn product_key(client: &ClientRec) -> String {
    match (client.kind, known_product(&client.redirects)) {
        (_, Some(label)) => format!("known:{label}"),
        (Kind::Document | Kind::Typed, None) => format!("{:?}:{}", client.kind, client.id),
        (Kind::Registered, None) => {
            let mut redirects = client.redirects.clone();
            redirects.sort();
            format!(
                "registered:{}|{}",
                client.name.to_lowercase(),
                redirects.join(" ")
            )
        }
    }
}

/// What the window calls a client: the product we recognise, else what it
/// calls itself.
fn label_of(client: &ClientRec) -> String {
    known_product(&client.redirects)
        .map(str::to_string)
        .unwrap_or_else(|| {
            let n = clean(&client.name, 60);
            if n.is_empty() {
                "An assistant".to_string()
            } else {
                n
            }
        })
}

/// A principals' token name from a label: lowercase letters, digits and
/// hyphens, at most 24 characters after `web-`.
fn token_slug(label: &str) -> String {
    let mut slug = String::new();
    for c in label.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }
    }
    let slug: String = slug.trim_matches('-').chars().take(24).collect();
    let slug = slug.trim_end_matches('-').to_string();
    format!(
        "web-{}",
        if slug.is_empty() {
            "assistant".to_string()
        } else {
            slug
        }
    )
}

impl Remote {
    pub(super) fn new() -> Self {
        Self {
            inner: Mutex::new(Inner::default()),
            door: RwLock::new(None),
            allowed: RwLock::new(BTreeSet::new()),
            clock: RwLock::new(Arc::new(|| {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs() as i64)
            })),
            docs: RwLock::new(Arc::new(WebDocs)),
            dir: Mutex::new(None),
            version: AtomicU64::new(0),
            written: Mutex::new(0),
        }
    }

    fn inner(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(super) fn now(&self) -> i64 {
        (self.clock.read().unwrap_or_else(|e| e.into_inner()))()
    }

    /// Test seam: the clock.
    #[cfg(test)]
    pub(super) fn set_clock(&self, clock: Arc<dyn Fn() -> i64 + Send + Sync>) {
        *self.clock.write().unwrap() = clock;
    }

    /// Test seam: where client metadata documents come from.
    #[cfg(test)]
    pub(super) fn set_docs(&self, docs: Arc<dyn ClientDocs>) {
        *self.docs.write().unwrap() = docs;
    }

    pub(super) fn door(&self) -> Option<Arc<Door>> {
        self.door.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// The one company on offer that is open, if there is exactly one.
    pub(super) fn only_open_book(&self, books: &Books) -> Option<String> {
        let open: Vec<String> = self
            .allowed
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter(|b| super::book_is_open(books, b))
            .cloned()
            .collect();
        match open.as_slice() {
            [only] => Some(only.clone()),
            _ => None,
        }
    }

    pub(super) fn is_allowed(&self, book: &str) -> bool {
        self.allowed
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .contains(book)
    }

    /// Count a request refused for want of a good token (INVARIANT 3).
    pub(super) fn count_unsigned(&self) {
        let mut inner = self.inner();
        inner.unsigned = inner.unsigned.saturating_add(1);
    }

    /// Requests refused for want of a good token, since the server began.
    #[cfg(test)]
    pub(super) fn unsigned(&self) -> u64 {
        self.inner().unsigned
    }

    // ---- persistence ------------------------------------------------------

    /// Read what was kept, once (from the root's `auth/`).
    fn load(&self, books: &Books) -> Result<(), String> {
        let dir = books.root.auth_dir().map_err(|e| e.to_string())?;
        *self.dir.lock().unwrap_or_else(|e| e.into_inner()) = Some(dir.clone());
        let mut inner = self.inner();
        if inner.loaded {
            return Ok(());
        }
        let saved: Saved = match std::fs::read(dir.join(FILE)) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|_| "the record of web sign-ins can't be read".to_string())?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Saved::default(),
            Err(_) => return Err("the record of web sign-ins can't be read".into()),
        };
        inner.clients = saved
            .clients
            .into_iter()
            .map(|c| (c.id.clone(), c))
            .collect();
        inner.approvals = saved
            .approvals
            .into_iter()
            .map(|a| (a.id.clone(), a))
            .collect();
        inner.refresh = saved
            .refresh
            .into_iter()
            .map(|r| (r.hash.clone(), r))
            .collect();
        inner.keys = saved
            .keys
            .into_iter()
            .map(|k| (k.token_id.clone(), k))
            .collect();
        inner.loaded = true;
        Ok(())
    }

    fn snapshot(&self, inner: &Inner) -> (u64, String) {
        let saved = Saved {
            version: 1,
            clients: inner.clients.values().cloned().collect(),
            approvals: inner.approvals.values().cloned().collect(),
            refresh: inner.refresh.values().cloned().collect(),
            keys: inner.keys.values().cloned().collect(),
        };
        let v = self.version.fetch_add(1, Ordering::SeqCst) + 1;
        (v, serde_json::to_string(&saved).unwrap_or_default())
    }

    /// Write the state (blocking), never letting an older snapshot
    /// overwrite a newer one. Private to the owner (0600), whole or not at
    /// all (written beside, then renamed over).
    fn save(&self) -> Result<(), String> {
        let (version, text) = self.snapshot(&self.inner());
        let dir = self
            .dir
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .ok_or_else(|| "the record of web sign-ins isn't open".to_string())?;
        let mut written = self.written.lock().unwrap_or_else(|e| e.into_inner());
        if version < *written {
            return Ok(());
        }
        let tmp = dir.join(format!("{FILE}.tmp"));
        {
            use std::io::Write;
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options
                .open(&tmp)
                .map_err(|_| "the record of web sign-ins can't be written".to_string())?;
            file.write_all(text.as_bytes())
                .and_then(|()| file.sync_all())
                .map_err(|_| "the record of web sign-ins can't be written".to_string())?;
        }
        std::fs::rename(&tmp, dir.join(FILE))
            .map_err(|_| "the record of web sign-ins can't be written".to_string())?;
        *written = version;
        Ok(())
    }

    /// [`Remote::save`] off the async threads.
    async fn save_async(self: &Arc<Self>) -> Result<(), String> {
        let me = self.clone();
        tokio::task::spawn_blocking(move || me.save())
            .await
            .unwrap_or_else(|_| Err("the record of web sign-ins can't be written".into()))
    }

    // ---- limits and sweeps ------------------------------------------------

    /// A fixed-window counter: whether `key` may do one more thing in
    /// `window` seconds, `max` at most. The table is bounded (INVARIANT 8).
    fn allow(&self, inner: &mut Inner, key: String, max: u32, window: i64) -> bool {
        let now = self.now();
        if inner.limits.len() >= MAX_LIMITS && !inner.limits.contains_key(&key) {
            inner.limits.retain(|_, (start, _)| now - *start < 3600);
            if inner.limits.len() >= MAX_LIMITS {
                inner.limits.clear();
            }
        }
        let slot = inner.limits.entry(key).or_insert((now, 0));
        if now - slot.0 >= window {
            *slot = (now, 0);
        }
        if slot.1 >= max {
            return false;
        }
        slot.1 += 1;
        true
    }

    /// Whether `key` may do one more thing in `window` seconds (callers
    /// outside the lock).
    pub(super) fn limited(&self, key: String, max: u32, window: i64) -> bool {
        let mut inner = self.inner();
        !self.allow(&mut inner, key, max, window)
    }

    /// Drop what has expired (INVARIANT 8). At most every few seconds.
    fn sweep(&self, inner: &mut Inner) {
        let now = self.now();
        if now - inner.last_sweep < 5 && now >= inner.last_sweep {
            return;
        }
        inner.last_sweep = now;
        inner.codes.retain(|_, c| c.exp > now);
        inner.used_codes.retain(|_, (_, until)| *until > now);
        inner.access.retain(|_, a| a.exp > now);
        inner.refresh.retain(|_, r| r.exp > now);
        inner.failed_docs.retain(|_, until| *until > now);
        inner.limits.retain(|_, (start, _)| now - *start < 3600);
        // A sign-in nobody decided is over (the page finds out on its
        // next look); one decided and collected is gone.
        for p in &mut inner.pending {
            if p.exp <= now && matches!(p.settled, Settled::Waiting) {
                p.settled = Settled::Refused;
            }
        }
        inner
            .pending
            .retain(|p| p.exp + 60 > now || matches!(p.settled, Settled::Waiting));
        // A registration nobody used is kept an hour; one a sign-in used
        // stays while a refresh token or an approval still names it.
        let named: BTreeSet<String> = inner.refresh.values().map(|r| r.client.clone()).collect();
        inner.clients.retain(|id, c| {
            c.kind == Kind::Typed
                || (c.used && (named.contains(id) || now - c.last_seen < CLIENT_IDLE))
                || (!c.used && now - c.created < CLIENT_IDLE)
        });
    }

    // ---- the door's own state ----------------------------------------------

    /// Turn the door's state on for `issuer`. Called by `RemoteStart`.
    pub(super) fn set_door(&self, door: Option<Arc<Door>>) {
        *self.door.write().unwrap_or_else(|e| e.into_inner()) = door;
    }

    pub(super) fn set_allowed(&self, book: &str, allowed: bool) {
        let mut set = self.allowed.write().unwrap_or_else(|e| e.into_inner());
        if allowed {
            set.insert(book.to_string());
        } else {
            set.remove(book);
            drop(set);
            // Nobody is left waiting for books that are no longer offered.
            let mut inner = self.inner();
            inner.pending.retain(|p| p.book != book);
        }
    }

    pub(super) fn info(&self) -> RemoteInfo {
        let door = self.door();
        RemoteInfo {
            listening: door.as_ref().map(|d| d.listening.to_string()),
            public_url: door.map(|d| d.issuer.clone()),
            allowed: self
                .allowed
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .cloned()
                .collect(),
        }
    }

    // ---- clients ------------------------------------------------------------

    /// Make room for one more client: the oldest unused registration goes
    /// (INVARIANT 8); none unused and the table is full is a refusal.
    fn make_room(&self, inner: &mut Inner) -> Result<(), OauthError> {
        if inner.clients.len() < MAX_CLIENTS {
            return Ok(());
        }
        let oldest = inner
            .clients
            .values()
            .filter(|c| !c.used && c.kind != Kind::Typed)
            .min_by_key(|c| c.created)
            .map(|c| c.id.clone());
        match oldest {
            Some(id) => {
                inner.clients.remove(&id);
                Ok(())
            }
            None => Err(OauthError::new(
                503,
                "temporarily_unavailable",
                "too many assistants are signed in to take another registration now",
            )),
        }
    }

    /// Dynamic registration (RFC 7591): the answer's JSON, or why not.
    pub(super) async fn register(
        self: &Arc<Self>,
        books: &Arc<Books>,
        meta: &serde_json::Value,
        from: &str,
    ) -> Result<serde_json::Value, OauthError> {
        self.ready(books)?;
        {
            let mut inner = self.inner();
            self.sweep(&mut inner);
            let ok = self.allow(
                &mut inner,
                format!("register:{from}"),
                REGISTER_PER_ADDRESS,
                3600,
            ) && self.allow(&mut inner, "register".into(), REGISTER_TOTAL, 3600);
            if !ok {
                return Err(OauthError::new(
                    429,
                    "temporarily_unavailable",
                    "too many registrations just now: try again later",
                ));
            }
        }
        let bad = |d: &str| OauthError::new(400, "invalid_client_metadata", d);
        let Some(obj) = meta.as_object() else {
            return Err(bad("the registration must be a JSON object"));
        };
        let uris = obj
            .get("redirect_uris")
            .and_then(|v| v.as_array())
            .ok_or_else(|| {
                OauthError::new(400, "invalid_redirect_uri", "redirect_uris is required")
            })?;
        if uris.is_empty() || uris.len() > MAX_REDIRECTS {
            return Err(OauthError::new(
                400,
                "invalid_redirect_uri",
                format!("give between 1 and {MAX_REDIRECTS} redirect_uris"),
            ));
        }
        let mut redirects = Vec::new();
        for u in uris {
            let text = u.as_str().unwrap_or_default();
            if valid_redirect(text).is_none() {
                return Err(OauthError::new(
                    400,
                    "invalid_redirect_uri",
                    "a redirect address isn't one this server can send you back to",
                ));
            }
            if !redirects.iter().any(|r| r == text) {
                redirects.push(text.to_string());
            }
        }
        let method = obj
            .get("token_endpoint_auth_method")
            .and_then(|v| v.as_str())
            .unwrap_or("none");
        if !["none", "client_secret_post", "client_secret_basic"].contains(&method) {
            return Err(bad("that token_endpoint_auth_method isn't supported"));
        }
        let name = clean(
            obj.get("client_name")
                .and_then(|v| v.as_str())
                .unwrap_or(""),
            100,
        );
        let id = random("dcr_", 18)?;
        let secret = if method == "none" {
            None
        } else {
            Some(random("", 32)?)
        };
        let now = self.now();
        {
            let mut inner = self.inner();
            self.make_room(&mut inner)?;
            inner.clients.insert(
                id.clone(),
                ClientRec {
                    id: id.clone(),
                    kind: Kind::Registered,
                    name: name.clone(),
                    redirects: redirects.clone(),
                    secret_sha256: secret.as_deref().map(sha_hex),
                    created: now,
                    last_seen: now,
                    used: false,
                    fetched: 0,
                },
            );
        }
        // A registration that can't be kept is not given: the client would
        // hold an id this server forgets.
        if let Err(e) = self.save_async().await {
            self.inner().clients.remove(&id);
            return Err(OauthError::new(500, "server_error", e));
        }
        let mut out = serde_json::json!({
            "client_id": id,
            "client_id_issued_at": now,
            "client_name": name,
            "redirect_uris": redirects,
            "token_endpoint_auth_method": method,
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
        });
        if let Some(s) = secret {
            out["client_secret"] = s.into();
            out["client_secret_expires_at"] = 0.into();
        }
        Ok(out)
    }

    /// The remote state is loaded (and its file readable).
    fn ready(&self, books: &Books) -> Result<(), OauthError> {
        self.load(books)
            .map_err(|e| OauthError::new(503, "temporarily_unavailable", e))
    }

    /// The client `id` names: registered, typed, or (an https address) a
    /// metadata document, fetched and kept.
    pub(super) async fn resolve_client(
        self: &Arc<Self>,
        books: &Arc<Books>,
        id: &str,
        from: &str,
    ) -> Result<ClientRec, OauthError> {
        self.ready(books)?;
        let now = self.now();
        {
            let mut inner = self.inner();
            self.sweep(&mut inner);
            if let Some(c) = inner.clients.get(id) {
                let fresh = c.kind != Kind::Document || now - c.fetched < CIMD_TTL;
                if fresh {
                    return Ok(c.clone());
                }
            }
        }
        if !id.starts_with("https://") {
            return Err(OauthError::new(
                401,
                "invalid_client",
                "that client_id isn't known here",
            ));
        }
        // A document: fetched outside the lock, within limits (the door
        // must not be a way to make us fetch the internet).
        {
            let mut inner = self.inner();
            if inner.failed_docs.get(id).is_some_and(|until| *until > now) {
                return Err(OauthError::new(
                    401,
                    "invalid_client",
                    "that client's metadata document couldn't be used a moment ago",
                ));
            }
            if !self.allow(&mut inner, "fetch".into(), FETCH_PER_MINUTE, 60)
                || !self.allow(&mut inner, format!("fetch:{from}"), FETCH_PER_ADDRESS, 60)
            {
                return Err(OauthError::new(
                    429,
                    "temporarily_unavailable",
                    "too many new assistants just now: try again in a minute",
                ));
            }
        }
        let docs = self.docs.read().unwrap_or_else(|e| e.into_inner()).clone();
        let fetched = docs.fetch(id).await;
        let parsed = fetched.and_then(|bytes| cimd::parse(id, &bytes));
        let mut inner = self.inner();
        match parsed {
            Ok((name, redirects)) => {
                let old = inner.clients.get(id).cloned();
                if old.is_none() {
                    self.make_room(&mut inner)?;
                }
                let rec = ClientRec {
                    id: id.to_string(),
                    kind: Kind::Document,
                    name,
                    redirects,
                    secret_sha256: None,
                    created: old.as_ref().map_or(now, |o| o.created),
                    last_seen: now,
                    used: old.as_ref().is_some_and(|o| o.used),
                    fetched: now,
                };
                inner.clients.insert(id.to_string(), rec.clone());
                Ok(rec)
            }
            Err(why) => {
                // A client we knew whose host is unreachable keeps working
                // from what we hold; a new one is refused.
                if let Some(c) = inner.clients.get(id) {
                    return Ok(c.clone());
                }
                if inner.failed_docs.len() < 1000 {
                    inner.failed_docs.insert(id.to_string(), now + 60);
                }
                Err(OauthError::new(401, "invalid_client", why))
            }
        }
    }

    /// A typed client id and secret, made by the app for a product that
    /// takes them (rule 2): its id and secret, shown this once.
    pub(super) fn make_client(
        &self,
        books: &Books,
        name: &str,
        redirect_uris: &[String],
    ) -> Result<(String, String), String> {
        self.load(books)?;
        let name = clean(name, 60);
        if name.is_empty() {
            return Err("give the assistant a name".into());
        }
        if redirect_uris.is_empty() || redirect_uris.len() > MAX_REDIRECTS {
            return Err(format!(
                "give between 1 and {MAX_REDIRECTS} addresses to send the assistant back to"
            ));
        }
        for u in redirect_uris {
            if valid_redirect(u).is_none() {
                return Err("one of the addresses can't be used to send an assistant back".into());
            }
        }
        let id = random("wbc_", 12).map_err(|e| e.description)?;
        let secret = random("", 32).map_err(|e| e.description)?;
        {
            let mut inner = self.inner();
            if inner
                .clients
                .values()
                .filter(|c| c.kind == Kind::Typed)
                .count()
                >= MAX_MANUAL
            {
                return Err("there are too many typed client ids; revoke some first".into());
            }
            self.make_room(&mut inner).map_err(|e| e.description)?;
            let now = self.now();
            inner.clients.insert(
                id.clone(),
                ClientRec {
                    id: id.clone(),
                    kind: Kind::Typed,
                    name,
                    redirects: redirect_uris.to_vec(),
                    secret_sha256: Some(sha_hex(&secret)),
                    created: now,
                    last_seen: now,
                    used: false,
                    fetched: 0,
                },
            );
        }
        self.save()?;
        Ok((id, secret))
    }

    // ---- authorize -----------------------------------------------------------

    /// Which company a `resource` names: its path under the one issuer.
    fn book_of_resource(&self, issuer: &str, resource: &str) -> Option<String> {
        let rest = resource.strip_prefix(issuer)?;
        let rest = rest.strip_suffix('/').unwrap_or(rest);
        let book = rest.strip_prefix("/b/")?.strip_suffix("/mcp")?;
        (BookId::parse(book).is_ok()).then(|| book.to_string())
    }

    /// A sign-in begins (`GET /authorize`). Either an answer to send the
    /// browser, or the request is parked.
    pub(super) async fn authorize(
        self: &Arc<Self>,
        books: &Arc<Books>,
        door: &Door,
        q: &BTreeMap<String, String>,
        from: &str,
    ) -> Authorize {
        {
            let mut inner = self.inner();
            if !self.allow(
                &mut inner,
                format!("authorize:{from}"),
                AUTHORIZE_PER_ADDRESS,
                600,
            ) {
                return Authorize::Page(
                    429,
                    pages::problem("Too many tries", "Wait a few minutes, then try again."),
                );
            }
        }
        let no_client = || {
            Authorize::Page(
                400,
                pages::problem(
                    "This assistant can't sign in",
                    "Wanderbooks doesn't know the assistant that sent you here. Start again from the assistant.",
                ),
            )
        };
        let Some(client_id) = q.get("client_id").filter(|s| !s.is_empty()) else {
            return no_client();
        };
        let Ok(client) = self.resolve_client(books, client_id, from).await else {
            return no_client();
        };
        // The redirect address: exactly one the client gave. Until it is
        // proven, an error is a page here, never a redirect (no open
        // redirect).
        let redirect = match q.get("redirect_uri") {
            Some(r) if client.redirects.iter().any(|x| x == r) => r.clone(),
            None if client.redirects.len() == 1 => client.redirects[0].clone(),
            _ => {
                return Authorize::Page(
                    400,
                    pages::problem(
                        "This assistant can't sign in",
                        "The address it asked to be sent back to isn't one it registered. Start again from the assistant.",
                    ),
                );
            }
        };
        let state = q.get("state").cloned();
        let fail = |code: &str, why: &str| {
            Authorize::Redirect(error_redirect(
                &redirect,
                state.as_deref(),
                code,
                why,
                &door.issuer,
            ))
        };
        if state.as_ref().is_some_and(|s| s.len() > 512) {
            return fail("invalid_request", "state is too long");
        }
        if q.get("response_type").map(String::as_str) != Some("code") {
            return fail("unsupported_response_type", "response_type must be code");
        }
        let challenge = q.get("code_challenge").cloned().unwrap_or_default();
        if q.get("code_challenge_method").map(String::as_str) != Some("S256")
            || challenge.len() != 43
            || !challenge
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return fail(
                "invalid_request",
                "PKCE with code_challenge_method=S256 is required",
            );
        }
        let book = match q.get("resource") {
            Some(r) => match self.book_of_resource(&door.issuer, r) {
                Some(b) => b,
                None => {
                    return fail(
                        "invalid_target",
                        "resource isn't one company's address here",
                    );
                }
            },
            None => {
                // A product that sends no resource: the only company on
                // offer, if there is only one.
                match self.only_open_book(books) {
                    Some(only) => only,
                    None => return fail("invalid_target", "say which company with resource"),
                }
            }
        };
        if !self.is_allowed(&book) || !super::book_is_open(books, &book) {
            return fail(
                "invalid_target",
                "those books aren't open to web assistants right now",
            );
        }
        let wants_post = q
            .get("scope")
            .is_some_and(|s| s.split_whitespace().any(|w| w == "books:post"));
        let product = product_key(&client);
        let label = label_of(&client);
        let now = self.now();
        // An approval the person already gave this product for this
        // company stands, however many times the product registers again
        // (rule 6): its principals' token must still be live.
        let existing = {
            let inner = self.inner();
            inner
                .approvals
                .values()
                .find(|a| a.book == book && a.product == product)
                .cloned()
        };
        if let Some(approval) = existing
            && now - approval.last_used < REFRESH_TTL
            && super::token_is_live(
                books,
                &approval.book,
                &approval.token_name,
                &approval.token_id,
            )
        {
            let code = {
                let mut inner = self.inner();
                if let Some(c) = inner.clients.get_mut(&client.id) {
                    c.used = true;
                    c.last_seen = now;
                }
                self.new_code(
                    &mut inner,
                    &client.id,
                    &approval,
                    &door.issuer,
                    &redirect,
                    &challenge,
                    None,
                )
            };
            return match code {
                Ok(code) => Authorize::Redirect(success_redirect(
                    &redirect,
                    state.as_deref(),
                    &code,
                    &door.issuer,
                )),
                Err(_) => fail("server_error", "no code could be made"),
            };
        }
        // Parked, for the person to decide in the app's window (rule 5).
        let parked = {
            let mut inner = self.inner();
            self.sweep(&mut inner);
            let total = inner
                .pending
                .iter()
                .filter(|p| matches!(p.settled, Settled::Waiting))
                .count();
            let mine = inner
                .pending
                .iter()
                .filter(|p| matches!(p.settled, Settled::Waiting) && p.from == from)
                .count();
            if total >= MAX_PENDING || mine >= MAX_PENDING_PER_ADDRESS {
                return Authorize::Page(
                    429,
                    pages::problem(
                        "Wanderbooks is busy",
                        "Too many assistants are waiting to be allowed. Try again in a few minutes.",
                    ),
                );
            }
            let (Ok(id), Ok(wait)) = (random("p_", 9), random("w_", 24)) else {
                return fail("server_error", "no id could be made");
            };
            if let Some(c) = inner.clients.get_mut(&client.id) {
                c.used = true;
                c.last_seen = now;
            }
            inner.pending.push(Pending {
                id,
                wait: wait.clone(),
                client: client.id.clone(),
                client_name: label.clone(),
                product: known_product(&client.redirects).map(str::to_string),
                redirect: redirect.clone(),
                state: state.clone(),
                challenge,
                book: book.clone(),
                issuer: door.issuer.clone(),
                from: from.to_string(),
                wants_post,
                created: now,
                exp: now + PENDING_TTL,
                settled: Settled::Waiting,
            });
            wait
        };
        let company = super::company_name(books, &book);
        Authorize::Page(200, pages::waiting(&label, &company, &parked))
    }

    /// The page's next look (`GET /authorize/wait/{wait}`): still waiting,
    /// or sent back with a code, or sent back refused. Read once.
    pub(super) fn wait(&self, wait: &str) -> Authorize {
        let now = self.now();
        let mut inner = self.inner();
        self.sweep(&mut inner);
        let Some(at) = inner.pending.iter().position(|p| p.wait == wait) else {
            return Authorize::Page(
                410,
                pages::problem("This request has ended", "Start again from your assistant."),
            );
        };
        if matches!(inner.pending[at].settled, Settled::Waiting) && inner.pending[at].exp <= now {
            inner.pending[at].settled = Settled::Refused;
        }
        let settled = match &inner.pending[at].settled {
            Settled::Waiting => None,
            Settled::Allowed(code) => Some(Some(code.clone())),
            Settled::Refused => Some(None),
        };
        match settled {
            None => {
                let p = &inner.pending[at];
                let company = p.book.clone();
                Authorize::Page(200, pages::waiting(&p.client_name, &company, wait))
            }
            Some(Some(code)) => {
                let p = inner.pending.remove(at);
                Authorize::Redirect(success_redirect(
                    &p.redirect,
                    p.state.as_deref(),
                    &code,
                    &p.issuer,
                ))
            }
            Some(None) => {
                let p = inner.pending.remove(at);
                let why = if p.exp <= now {
                    "nobody allowed it in time"
                } else {
                    "the owner didn't allow it"
                };
                Authorize::Redirect(error_redirect(
                    &p.redirect,
                    p.state.as_deref(),
                    "access_denied",
                    why,
                    &p.issuer,
                ))
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn new_code(
        &self,
        inner: &mut Inner,
        client: &str,
        approval: &Approval,
        issuer: &str,
        redirect: &str,
        challenge: &str,
        family: Option<String>,
    ) -> Result<String, OauthError> {
        if inner.codes.len() >= MAX_CODES {
            let now = self.now();
            inner.codes.retain(|_, c| c.exp > now);
            if inner.codes.len() >= MAX_CODES {
                return Err(OauthError::new(503, "temporarily_unavailable", "busy"));
            }
        }
        let code = random("wbcode_", 32)?;
        let family = match family {
            Some(f) => f,
            None => random("fam_", 12)?,
        };
        inner.codes.insert(
            sha_hex(&code),
            Code {
                client: client.to_string(),
                approval: approval.id.clone(),
                family,
                book: approval.book.clone(),
                issuer: issuer.to_string(),
                redirect: redirect.to_string(),
                challenge: challenge.to_string(),
                exp: self.now() + CODE_TTL,
            },
        );
        Ok(code)
    }

    // ---- the person decides (admin socket only) --------------------------

    pub(super) fn pending_list(&self) -> Vec<PendingInfo> {
        let now = self.now();
        let mut inner = self.inner();
        self.sweep(&mut inner);
        inner
            .pending
            .iter()
            .filter(|p| matches!(p.settled, Settled::Waiting) && p.exp > now)
            .map(|p| PendingInfo {
                id: p.id.clone(),
                book: p.book.clone(),
                client_name: p.client_name.clone(),
                product: p.product.clone(),
                redirect: p.redirect.clone(),
                redirect_host: redirect_host(&p.redirect),
                from: p.from.clone(),
                wants_post: p.wants_post,
                age_secs: (now - p.created).max(0) as u64,
                expires_in_secs: (p.exp - now).max(0) as u64,
            })
            .collect()
    }

    /// The person's answer to one waiting request (INVARIANT 5): `post`
    /// raises it beyond read and draft.
    pub(super) fn decide(
        &self,
        books: &Books,
        id: &str,
        approve: bool,
        post: bool,
    ) -> Result<(String, Option<ApprovalInfo>), String> {
        self.load(books)?;
        let door = self.door().ok_or("web assistants are switched off")?;
        let now = self.now();
        let (book, client_id, redirect, challenge) = {
            let mut inner = self.inner();
            self.sweep(&mut inner);
            let p = inner
                .pending
                .iter_mut()
                .find(|p| p.id == id && matches!(p.settled, Settled::Waiting) && p.exp > now)
                .ok_or("that request isn't waiting any more: it was decided, or it ran out")?;
            if !approve {
                p.settled = Settled::Refused;
                return Ok(("Refused. The assistant was told no.".into(), None));
            }
            (
                p.book.clone(),
                p.client.clone(),
                p.redirect.clone(),
                p.challenge.clone(),
            )
        };
        if !self.is_allowed(&book) || !super::book_is_open(books, &book) {
            return Err("those books aren't open to web assistants now".into());
        }
        let client = self
            .inner()
            .clients
            .get(&client_id)
            .cloned()
            .ok_or("that assistant's registration ran out: ask it to start again")?;
        let product = product_key(&client);
        let existing = self
            .inner()
            .approvals
            .values()
            .find(|a| a.book == book && a.product == product)
            .cloned();
        let approval = match existing {
            Some(a) if super::token_is_live(books, &a.book, &a.token_name, &a.token_id) => {
                if post && !self.may_post(books, &a) {
                    self.set_post(books, &a.id, true)?
                } else {
                    a
                }
            }
            old => self.make_approval(books, &book, &client, &product, post, old)?,
        };
        {
            let mut inner = self.inner();
            let code = self
                .new_code(
                    &mut inner,
                    &client_id,
                    &approval,
                    &door.issuer,
                    &redirect,
                    &challenge,
                    None,
                )
                .map_err(|e| e.description)?;
            if let Some(p) = inner.pending.iter_mut().find(|p| p.id == id) {
                p.settled = Settled::Allowed(code);
            }
            if let Some(a) = inner.approvals.get_mut(&approval.id) {
                a.last_used = now;
            }
        }
        self.save()?;
        let info = self.approval_info(books, &approval);
        Ok((
            format!("Allowed. {} can reach these books.", approval.label),
            info,
        ))
    }

    fn may_post(&self, books: &Books, a: &Approval) -> bool {
        super::live_token(books, &a.book, &a.token_name, &a.token_id)
            .is_some_and(|t| t.scopes.contains(&Scope::Post))
    }

    /// A new approval, and the principals' token that carries it.
    fn make_approval(
        &self,
        books: &Books,
        book: &str,
        client: &ClientRec,
        product: &str,
        post: bool,
        old: Option<Approval>,
    ) -> Result<Approval, String> {
        let book_id = BookId::parse(book).map_err(|e| e.to_string())?;
        {
            let inner = self.inner();
            if old.is_none() && inner.approvals.len() >= MAX_APPROVALS {
                return Err("there are too many connections: revoke some first".into());
            }
        }
        let label = label_of(client);
        let base = token_slug(&label);
        let taken: BTreeSet<String> = super::token_names(books, book);
        let name = (1..)
            .map(|n| {
                if n == 1 {
                    base.clone()
                } else {
                    format!("{base}-{n}")
                }
            })
            .find(|n| n.len() <= 32 && !taken.contains(n))
            .ok_or("no name is free for this connection")?;
        let token = super::make_token(books, &book_id, &name, scopes_for(post), false)?;
        let now = self.now();
        let approval = Approval {
            id: old
                .as_ref()
                .map(|o| o.id.clone())
                .unwrap_or(random("apr_", 9).map_err(|e| e.description)?),
            book: book.to_string(),
            product: product.to_string(),
            label,
            token_name: name,
            token_id: token.id,
            created: now,
            last_used: now,
        };
        self.inner()
            .approvals
            .insert(approval.id.clone(), approval.clone());
        Ok(approval)
    }

    /// Raise or lower what an approval may do (rule 10): the token that
    /// carries it is replaced by one with the new scopes, in one record.
    pub(super) fn set_post(
        &self,
        books: &Books,
        approval_id: &str,
        post: bool,
    ) -> Result<Approval, String> {
        self.load(books)?;
        let mut approval = self
            .inner()
            .approvals
            .get(approval_id)
            .cloned()
            .ok_or("there is no such connection")?;
        if !super::token_is_live(
            books,
            &approval.book,
            &approval.token_name,
            &approval.token_id,
        ) {
            return Err("that connection was revoked".into());
        }
        let book_id = BookId::parse(&approval.book).map_err(|e| e.to_string())?;
        let token = super::make_token(
            books,
            &book_id,
            &approval.token_name,
            scopes_for(post),
            true,
        )?;
        approval.token_id = token.id;
        self.inner()
            .approvals
            .insert(approval.id.clone(), approval.clone());
        self.save()?;
        Ok(approval)
    }

    fn approval_info(&self, books: &Books, approval: &Approval) -> Option<ApprovalInfo> {
        let token = super::live_token(
            books,
            &approval.book,
            &approval.token_name,
            &approval.token_id,
        )?;
        Some(ApprovalInfo {
            id: approval.id.clone(),
            book: approval.book.clone(),
            label: approval.label.clone(),
            token_id: token.id.clone(),
            token_name: token.name.clone(),
            may_post: token.scopes.contains(&Scope::Post),
            created: approval.created,
            last_used: (approval.last_used > approval.created).then_some(approval.last_used),
        })
    }

    /// The live approvals (those whose token still stands), for `book` or
    /// all; the ones revoked are forgotten, with their refresh tokens.
    pub(super) fn approvals(&self, books: &Books, book: Option<&str>) -> Vec<ApprovalInfo> {
        let _ = self.load(books);
        let all: Vec<Approval> = self.inner().approvals.values().cloned().collect();
        let mut gone = Vec::new();
        let mut out = Vec::new();
        for a in all {
            match self.approval_info(books, &a) {
                Some(info) => {
                    if book.is_none_or(|b| b == a.book) {
                        out.push(info);
                    }
                }
                None => gone.push(a.id),
            }
        }
        if !gone.is_empty() {
            let mut inner = self.inner();
            for id in &gone {
                inner.approvals.remove(id);
            }
            inner.refresh.retain(|_, r| !gone.contains(&r.approval));
            inner.access.retain(|_, r| !gone.contains(&r.approval));
            drop(inner);
            let _ = self.save();
        }
        out
    }

    /// A key for a product that takes only a key (rule 9): a token of the
    /// books' principals the remote door accepts, read and draft (and
    /// post if raised), shown once.
    pub(super) fn make_key(
        &self,
        books: &Books,
        book: &str,
        name: &str,
        post: bool,
    ) -> Result<(Token, String), String> {
        self.load(books)?;
        let book_id = BookId::parse(book).map_err(|e| e.to_string())?;
        if !super::book_is_open(books, book) {
            return Err(format!("there are no open books called {book}"));
        }
        if self.inner().keys.len() >= MAX_KEYS {
            return Err("there are too many keys; revoke some first".into());
        }
        let name = format!("key-{}", token_slug(name).trim_start_matches("web-"));
        let (token, text) = super::make_token_with_text(books, &book_id, &name, scopes_for(post))?;
        self.inner().keys.insert(
            token.id.clone(),
            KeyRec {
                token_id: token.id.clone(),
                book: book.to_string(),
                created: self.now(),
            },
        );
        self.save()?;
        Ok((token, text))
    }

    // ---- the token endpoint ------------------------------------------------

    /// A token request (`POST /token`): `form` is its parameters, `basic`
    /// the client credentials of an Authorization header, if any.
    pub(super) async fn token(
        self: &Arc<Self>,
        books: &Arc<Books>,
        door: &Door,
        form: &BTreeMap<String, String>,
        basic: Option<(String, String)>,
        from: &str,
    ) -> Result<serde_json::Value, OauthError> {
        let (client_id, secret) = match basic {
            Some((id, s)) => (Some(id), Some(s)),
            None => (
                form.get("client_id").cloned(),
                form.get("client_secret").cloned(),
            ),
        };
        let Some(client_id) = client_id.filter(|s| !s.is_empty()) else {
            return Err(OauthError::new(401, "invalid_client", "no client_id"));
        };
        {
            let mut inner = self.inner();
            if !self.allow(
                &mut inner,
                format!("token:{client_id}"),
                TOKEN_PER_CLIENT,
                60,
            ) {
                return Err(OauthError::new(
                    429,
                    "temporarily_unavailable",
                    "too many token requests: slow down",
                ));
            }
        }
        let grant = form.get("grant_type").map(String::as_str).unwrap_or("");
        let client = match self.resolve_client(books, &client_id, from).await {
            Ok(c) => c,
            Err(e) if grant == "refresh_token" && e.error == "invalid_client" => {
                return Err(OauthError::new(
                    400,
                    "invalid_grant",
                    "that refresh token isn't one this server knows",
                ));
            }
            Err(e) => return Err(e),
        };
        // A client with a secret proves it, in constant time, before
        // anything is consumed.
        if let Some(want) = &client.secret_sha256 {
            use subtle::ConstantTimeEq;
            let given = sha_hex(secret.as_deref().unwrap_or(""));
            let same: bool = given.as_bytes().ct_eq(want.as_bytes()).into();
            if secret.is_none() || !same {
                return Err(OauthError::new(
                    401,
                    "invalid_client",
                    "client authentication failed",
                ));
            }
        }
        if let Some(r) = form.get("resource")
            && self.book_of_resource(&door.issuer, r).is_none()
        {
            return Err(OauthError::new(
                400,
                "invalid_target",
                "resource isn't one company's address here",
            ));
        }
        match grant {
            "authorization_code" => self.exchange_code(books, door, &client, form).await,
            "refresh_token" => self.refresh(books, door, &client, form).await,
            _ => Err(OauthError::new(
                400,
                "unsupported_grant_type",
                "grant_type must be authorization_code or refresh_token",
            )),
        }
    }

    async fn exchange_code(
        self: &Arc<Self>,
        books: &Arc<Books>,
        door: &Door,
        client: &ClientRec,
        form: &BTreeMap<String, String>,
    ) -> Result<serde_json::Value, OauthError> {
        let bad = |d: &str| OauthError::new(400, "invalid_grant", d);
        let given = form.get("code").cloned().unwrap_or_default();
        let key = sha_hex(&given);
        let now = self.now();
        let (taken, replay) = {
            let mut inner = self.inner();
            self.sweep(&mut inner);
            let taken = inner.codes.remove(&key);
            // A code used before: whoever holds what it made may be a
            // thief; end that family (RFC 9700 4.3).
            let replay = if taken.is_none() {
                inner.used_codes.get(&key).map(|(f, _)| f.clone())
            } else {
                None
            };
            if let Some(family) = &replay {
                inner.refresh.retain(|_, r| r.family != *family);
                inner.access.retain(|_, a| a.family != *family);
            }
            (taken, replay)
        };
        let Some(code) = taken else {
            if replay.is_some() {
                let _ = self.save_async().await;
            }
            return Err(bad("that code is invalid, used, or has expired"));
        };
        // Single use: remembered, so a replay is seen.
        {
            let mut inner = self.inner();
            if inner.used_codes.len() < MAX_CODES * 2 {
                inner
                    .used_codes
                    .insert(key, (code.family.clone(), now + USED_CODE_KEPT));
            }
        }
        if code.exp <= now || code.client != client.id {
            return Err(bad("that code is invalid, used, or has expired"));
        }
        if code.issuer != door.issuer {
            return Err(bad("that code was made for another address"));
        }
        if let Some(r) = form.get("redirect_uri")
            && *r != code.redirect
        {
            return Err(bad("redirect_uri isn't the one of the sign-in"));
        }
        let verifier = form.get("code_verifier").cloned().unwrap_or_default();
        let ok_shape = (43..=128).contains(&verifier.len())
            && verifier
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-._~".contains(&b));
        let computed = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(Sha256::digest(verifier.as_bytes()));
        {
            use subtle::ConstantTimeEq;
            let same: bool = computed.as_bytes().ct_eq(code.challenge.as_bytes()).into();
            if !ok_shape || !same {
                return Err(bad("the PKCE verifier doesn't match"));
            }
        }
        let approval = self
            .inner()
            .approvals
            .get(&code.approval)
            .cloned()
            .filter(|a| a.book == code.book)
            .ok_or_else(|| bad("that connection no longer exists"))?;
        if !super::token_is_live(
            books,
            &approval.book,
            &approval.token_name,
            &approval.token_id,
        ) {
            return Err(bad("that connection was revoked"));
        }
        self.issue(books, door, client, &approval, &code.family, None)
            .await
    }

    async fn refresh(
        self: &Arc<Self>,
        books: &Arc<Books>,
        door: &Door,
        client: &ClientRec,
        form: &BTreeMap<String, String>,
    ) -> Result<serde_json::Value, OauthError> {
        let bad = |d: &str| OauthError::new(400, "invalid_grant", d);
        let given = form.get("refresh_token").cloned().unwrap_or_default();
        let key = sha_hex(&given);
        let _now = self.now();
        let (found, reused) = {
            let mut inner = self.inner();
            self.sweep(&mut inner);
            let found = inner.refresh.get(&key).cloned();
            let reused = found.as_ref().is_some_and(|r| r.used);
            if let Some(rec) = found.as_ref().filter(|r| r.used) {
                // Used twice: the family is over (RFC 9700 4.14).
                let family = rec.family.clone();
                inner.refresh.retain(|_, r| r.family != family);
                inner.access.retain(|_, a| a.family != family);
            }
            (found, reused)
        };
        let Some(rec) = found else {
            return Err(bad("that refresh token is invalid, used, or has expired"));
        };
        if reused {
            let _ = self.save_async().await;
            return Err(bad("that refresh token was already used: sign in again"));
        }
        if rec.client != client.id {
            return Err(bad("that refresh token belongs to another assistant"));
        }
        if rec.issuer != door.issuer {
            return Err(bad("that refresh token was made for another address"));
        }
        let approval = self
            .inner()
            .approvals
            .get(&rec.approval)
            .cloned()
            .ok_or_else(|| bad("that connection no longer exists"))?;
        if !super::token_is_live(
            books,
            &approval.book,
            &approval.token_name,
            &approval.token_id,
        ) {
            return Err(bad("that connection was revoked"));
        }
        self.issue(books, door, client, &approval, &rec.family, Some(key))
            .await
    }

    /// Mint an access token and a refresh token under `approval`, and (a
    /// refresh) retire the refresh token it came from.
    async fn issue(
        self: &Arc<Self>,
        books: &Arc<Books>,
        door: &Door,
        client: &ClientRec,
        approval: &Approval,
        family: &str,
        retire: Option<String>,
    ) -> Result<serde_json::Value, OauthError> {
        let access = random("wba_", 32)?;
        let refresh = random("wbr_", 32)?;
        let now = self.now();
        let (new_hash, undo) = {
            let mut inner = self.inner();
            if inner.access.len() >= MAX_ACCESS {
                inner.access.retain(|_, a| a.exp > now);
                if inner.access.len() >= MAX_ACCESS {
                    return Err(OauthError::new(503, "temporarily_unavailable", "busy"));
                }
            }
            // The family's count of one approval is bounded: the one used
            // longest ago goes.
            let mut families: Vec<(String, i64)> = Vec::new();
            for r in inner.refresh.values().filter(|r| r.approval == approval.id) {
                match families.iter_mut().find(|(f, _)| *f == r.family) {
                    Some(slot) => slot.1 = slot.1.max(r.created),
                    None => families.push((r.family.clone(), r.created)),
                }
            }
            if !families.iter().any(|(f, _)| f == family) && families.len() >= MAX_FAMILIES {
                families.sort_by_key(|(_, at)| *at);
                let drop_family = families[0].0.clone();
                inner.refresh.retain(|_, r| r.family != drop_family);
            }
            if inner.refresh.len() >= MAX_REFRESH {
                inner.refresh.retain(|_, r| r.exp > now);
                if inner.refresh.len() >= MAX_REFRESH {
                    return Err(OauthError::new(503, "temporarily_unavailable", "busy"));
                }
            }
            let new_hash = sha_hex(&refresh);
            inner.refresh.insert(
                new_hash.clone(),
                RefreshRec {
                    hash: new_hash.clone(),
                    family: family.to_string(),
                    approval: approval.id.clone(),
                    client: client.id.clone(),
                    issuer: door.issuer.clone(),
                    exp: now + REFRESH_TTL,
                    used: false,
                    created: now,
                },
            );
            if let Some(old) = &retire
                && let Some(r) = inner.refresh.get_mut(old)
            {
                r.used = true;
                // A used token is kept to see a replay, only as long as it
                // would have lived, and no longer than a week.
                r.exp = r.exp.min(now + 7 * 24 * 3600);
            }
            // The families keep at most two used tokens each.
            let mut used: Vec<(String, i64)> = inner
                .refresh
                .values()
                .filter(|r| r.family == family && r.used)
                .map(|r| (r.hash.clone(), r.created))
                .collect();
            if used.len() > 2 {
                used.sort_by_key(|(_, at)| *at);
                for (h, _) in &used[..used.len() - 2] {
                    inner.refresh.remove(h);
                }
            }
            inner.access.insert(
                sha_hex(&access),
                AccessRec {
                    approval: approval.id.clone(),
                    family: family.to_string(),
                    book: approval.book.clone(),
                    issuer: door.issuer.clone(),
                    exp: now + ACCESS_TTL,
                },
            );
            if let Some(a) = inner.approvals.get_mut(&approval.id) {
                a.last_used = now;
            }
            (new_hash, retire.clone())
        };
        // Kept before it is given (a refresh token this server forgot
        // would sign the person out): if it can't be kept, nothing was made.
        if let Err(e) = self.save_async().await {
            let mut inner = self.inner();
            inner.refresh.remove(&new_hash);
            inner.access.remove(&sha_hex(&access));
            if let Some(old) = undo
                && let Some(r) = inner.refresh.get_mut(&old)
            {
                r.used = false;
            }
            return Err(OauthError::new(500, "server_error", e));
        }
        let scope = super::live_token(
            books,
            &approval.book,
            &approval.token_name,
            &approval.token_id,
        )
        .map(|t| {
            t.scopes
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default();
        Ok(serde_json::json!({
            "access_token": access,
            "token_type": "Bearer",
            "expires_in": ACCESS_TTL,
            "refresh_token": refresh,
            "scope": scope,
        }))
    }

    // ---- the door's check --------------------------------------------------

    /// The token a request carries, if it is good here (INVARIANTS 3, 6,
    /// 9): an access token made by this door, or a key the app made.
    pub(super) fn authenticate(
        &self,
        books: &Books,
        headers: &axum::http::HeaderMap,
    ) -> Result<Token, Denied> {
        let denied = |code: &'static str, why: &str, invalid: bool| Denied {
            code,
            why: why.to_string(),
            invalid_token: invalid,
        };
        let presented = headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split_once(' '))
            .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
            .map(|(_, token)| token.trim());
        let Some(presented) = presented.filter(|t| !t.is_empty()) else {
            return Err(denied(
                crate::access::UNAUTHORIZED,
                "sign in to reach these books",
                false,
            ));
        };
        let door = self
            .door()
            .ok_or_else(|| denied(crate::access::UNAUTHORIZED, "web assistants are off", false))?;
        let now = self.now();
        let mut token = if presented.starts_with("wba_") {
            let (approval, rec) = {
                let mut inner = self.inner();
                self.sweep(&mut inner);
                let rec = inner.access.get(&sha_hex(presented)).ok_or_else(|| {
                    denied(
                        crate::access::UNAUTHORIZED,
                        "that token isn't one these books know, or it has ended",
                        true,
                    )
                })?;
                if rec.exp <= now {
                    return Err(denied(
                        crate::access::UNAUTHORIZED,
                        "that token has expired",
                        true,
                    ));
                }
                let rec = (rec.approval.clone(), rec.issuer.clone(), rec.book.clone());
                let approval = inner.approvals.get(&rec.0).cloned();
                (approval, rec)
            };
            if rec.1 != door.issuer {
                return Err(denied(
                    crate::access::UNAUTHORIZED,
                    "that token was made for another address",
                    true,
                ));
            }
            let approval = approval.ok_or_else(|| {
                denied(
                    crate::access::UNAUTHORIZED,
                    "that connection no longer exists",
                    true,
                )
            })?;
            // The company the token was made for is the approval's, always.
            if rec.2 != approval.book {
                return Err(denied(
                    crate::access::UNAUTHORIZED,
                    "that token isn't for these books",
                    true,
                ));
            }
            let token = super::live_token(
                books,
                &approval.book,
                &approval.token_name,
                &approval.token_id,
            )
            .ok_or_else(|| {
                denied(
                    crate::access::UNAUTHORIZED,
                    "that connection was revoked",
                    true,
                )
            })?;
            if let Some(a) = self.inner().approvals.get_mut(&approval.id) {
                a.last_used = now;
            }
            token
        } else {
            // A key the app made (rule 9): a principals' token, and only
            // one of those, never an agent's own key for the local socket.
            let guard = books.principals.read().unwrap_or_else(|e| e.into_inner());
            let found = guard
                .as_ref()
                .ok()
                .and_then(|p| p.check(presented))
                .cloned();
            drop(guard);
            let token = found.ok_or_else(|| {
                denied(
                    crate::access::UNAUTHORIZED,
                    "that token isn't one these books know, or it was revoked",
                    true,
                )
            })?;
            if !self.inner().keys.contains_key(&token.id) {
                return Err(denied(
                    crate::access::UNAUTHORIZED,
                    "that key isn't one for web assistants",
                    true,
                ));
            }
            token
        };
        // Never the access record, whatever the token says.
        token.scopes.remove(&Scope::Admin);
        Ok(token)
    }

    /// The approval a token belongs to, for the call limit.
    pub(super) fn call_key(&self, token: &Token) -> String {
        format!("call:{}", token.id)
    }
}

/// What `/authorize` and its waiting page answer.
pub(super) enum Authorize {
    /// An HTML page, with its status.
    Page(u16, String),
    /// A 302 to this address.
    Redirect(String),
}

fn with_query(base: &str, params: &[(&str, &str)]) -> String {
    let mut url = match reqwest::Url::parse(base) {
        Ok(u) => u,
        Err(_) => return base.to_string(),
    };
    {
        let mut q = url.query_pairs_mut();
        for (k, v) in params {
            q.append_pair(k, v);
        }
    }
    url.to_string()
}

fn success_redirect(redirect: &str, state: Option<&str>, code: &str, issuer: &str) -> String {
    let mut params = vec![("code", code)];
    if let Some(s) = state {
        params.push(("state", s));
    }
    params.push(("iss", issuer));
    with_query(redirect, &params)
}

fn error_redirect(
    redirect: &str,
    state: Option<&str>,
    error: &str,
    why: &str,
    issuer: &str,
) -> String {
    let mut params = vec![("error", error), ("error_description", why)];
    if let Some(s) = state {
        params.push(("state", s));
    }
    params.push(("iss", issuer));
    with_query(redirect, &params)
}
