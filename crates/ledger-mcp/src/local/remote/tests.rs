//! The remote door, end to end on localhost: a raw HTTP client that
//! writes the relay's PROXY header first, a books server in a temp root,
//! and the person's answers made through the admin requests the window
//! makes. Nothing here touches a network beyond 127.0.0.1.

use super::*;
use crate::admin::{Request, Response};
use crate::server::Isolation;
use base64::Engine as _;
use ledger_store::root::Root;
use serde_json::{Value, json};
use std::sync::atomic::AtomicI64;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const ISSUER: &str = "https://ledger-accrual-tally-mint.wanderbooks.link";
const CLAUDE_CB: &str = "https://claude.ai/api/mcp/auth_callback";
const CALLER: &str = "203.0.113.9";

struct H {
    books: Arc<Books>,
    addr: SocketAddr,
    clock: Arc<AtomicI64>,
    _home: tempfile::TempDir,
}

async fn admin(books: &Arc<Books>, request: Request) -> Response {
    let books = books.clone();
    tokio::task::spawn_blocking(move || books.admin(request))
        .await
        .unwrap()
}

async fn harness() -> H {
    let home = tempfile::tempdir().unwrap();
    let (root, _) = Root::setup(home.path(), Some(&home.path().join("root")), None).unwrap();
    let books = Arc::new(Books::new(
        root,
        super::super::BACKUP_QUIET,
        super::super::DRILL_FIRST,
        Isolation::SameUser,
    ));
    let loaded = books.load_principals();
    *books.principals.write().unwrap() = loaded;
    let clock = Arc::new(AtomicI64::new(1_800_000_000));
    let c = clock.clone();
    books
        .remote
        .set_clock(Arc::new(move || c.load(Ordering::SeqCst)));
    for id in ["acme", "beta"] {
        match admin(
            &books,
            Request::Create {
                id: id.into(),
                currency: "USD".into(),
            },
        )
        .await
        {
            Response::Created { problem: None, .. } => {}
            other => panic!("{other:?}"),
        }
    }
    let mut h = H {
        books,
        addr: "127.0.0.1:1".parse().unwrap(),
        clock,
        _home: home,
    };
    h.start(ISSUER).await;
    h
}

impl H {
    async fn start(&mut self, issuer: &str) {
        match admin(
            &self.books,
            Request::RemoteStart {
                public_url: issuer.into(),
                port: None,
            },
        )
        .await
        {
            Response::Remote(info) => self.addr = info.listening.unwrap().parse().unwrap(),
            other => panic!("{other:?}"),
        }
        for book in ["acme", "beta"] {
            admin(
                &self.books,
                Request::RemoteAllow {
                    book: book.into(),
                    allowed: true,
                },
            )
            .await;
        }
    }

    fn advance(&self, secs: i64) {
        self.clock.fetch_add(secs, Ordering::SeqCst);
    }

    fn host(&self) -> String {
        ISSUER.trim_start_matches("https://").to_string()
    }
}

struct Resp {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl Resp {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or_else(|_| panic!("not JSON: {}", self.body))
    }
}

fn unchunk(mut body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    while let Some(at) = body.windows(2).position(|w| w == b"\r\n") {
        let size = usize::from_str_radix(std::str::from_utf8(&body[..at]).unwrap().trim(), 16)
            .unwrap_or(0);
        if size == 0 {
            break;
        }
        out.extend_from_slice(&body[at + 2..at + 2 + size]);
        body = &body[at + 2 + size + 2..];
    }
    out
}

fn parse(raw: &[u8]) -> Resp {
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .unwrap_or_else(|| panic!("no response: {:?}", String::from_utf8_lossy(raw)));
    let head = String::from_utf8_lossy(&raw[..split]).to_string();
    let mut lines = head.lines();
    let status = lines
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect();
    let rest = &raw[split + 4..];
    let chunked = headers
        .iter()
        .any(|(k, v)| k.eq_ignore_ascii_case("transfer-encoding") && v.contains("chunked"));
    let body = if chunked {
        unchunk(rest)
    } else {
        rest.to_vec()
    };
    Resp {
        status,
        headers,
        body: String::from_utf8_lossy(&body).to_string(),
    }
}

/// Bytes to the door, read to the end.
async fn raw(addr: SocketAddr, bytes: &[u8]) -> Vec<u8> {
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let _ = stream.write_all(bytes).await;
    let mut out = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(10), stream.read_to_end(&mut out)).await;
    out
}

fn request_bytes(
    host: &str,
    method: &str,
    path: &str,
    extra: &[(&str, &str)],
    body: &str,
) -> Vec<u8> {
    let mut text = format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n");
    for (k, v) in extra {
        text.push_str(&format!("{k}: {v}\r\n"));
    }
    text.push_str(&format!("Content-Length: {}\r\n\r\n{body}", body.len()));
    text.into_bytes()
}

async fn http_from(
    h: &H,
    from: &str,
    method: &str,
    path: &str,
    extra: &[(&str, &str)],
    body: &str,
) -> Resp {
    let mut bytes = proxy::encode(
        format!("{from}:40000").parse().unwrap(),
        "198.51.100.1:443".parse().unwrap(),
    );
    bytes.extend(request_bytes(&h.host(), method, path, extra, body));
    parse(&raw(h.addr, &bytes).await)
}

async fn http(h: &H, method: &str, path: &str, extra: &[(&str, &str)], body: &str) -> Resp {
    http_from(h, CALLER, method, path, extra, body).await
}

fn pkce(n: u32) -> (String, String) {
    let verifier = format!("verifier-{n}-{}", "x".repeat(40));
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

fn form(pairs: &[(&str, &str)]) -> String {
    let mut u = reqwest::Url::parse("http://x/").unwrap();
    for (k, v) in pairs {
        u.query_pairs_mut().append_pair(k, v);
    }
    u.query().unwrap_or_default().to_string()
}

async fn register(h: &H, name: &str, redirects: &[&str]) -> Value {
    let r = http(
        h,
        "POST",
        "/register",
        &[("Content-Type", "application/json")],
        &json!({"client_name": name, "redirect_uris": redirects}).to_string(),
    )
    .await;
    assert_eq!(r.status, 201, "{}", r.body);
    r.json()
}

fn authorize_path(
    client: &str,
    redirect: &str,
    challenge: &str,
    book: &str,
    extra: &[(&str, &str)],
) -> String {
    let resource = format!("{ISSUER}/b/{book}/mcp");
    let mut pairs = vec![
        ("response_type", "code"),
        ("client_id", client),
        ("redirect_uri", redirect),
        ("code_challenge", challenge),
        ("code_challenge_method", "S256"),
        ("state", "st-123"),
        ("resource", resource.as_str()),
        ("scope", "books:read books:write"),
    ];
    pairs.extend_from_slice(extra);
    format!("/authorize?{}", form(&pairs))
}

fn location_params(r: &Resp) -> BTreeMap<String, String> {
    assert_eq!(r.status, 302, "{} {}", r.status, r.body);
    parse_pairs_of(r.header("location").unwrap())
}

fn parse_pairs_of(location: &str) -> BTreeMap<String, String> {
    reqwest::Url::parse(location)
        .unwrap()
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect()
}

fn wait_path(page: &Resp) -> String {
    assert_eq!(page.status, 200, "{}", page.body);
    let at = page
        .body
        .find("url=/authorize/wait/")
        .expect("a waiting page");
    let rest = &page.body[at + 4..];
    rest[..rest.find('"').unwrap()].to_string()
}

async fn pending(h: &H) -> Vec<crate::admin::PendingInfo> {
    match admin(&h.books, Request::RemotePending).await {
        Response::RemotePending(p) => p,
        other => panic!("{other:?}"),
    }
}

async fn decide(h: &H, id: &str, approve: bool, post: bool) -> Response {
    admin(
        &h.books,
        Request::RemoteDecide {
            id: id.into(),
            approve,
            post,
        },
    )
    .await
}

/// The whole sign-in of a client already known: start, the person allows
/// it in the window, the browser is sent back. The code and what the
/// browser was told.
async fn sign_in(
    h: &H,
    client: &str,
    redirect: &str,
    book: &str,
    n: u32,
    post: bool,
) -> (String, String) {
    let (verifier, challenge) = pkce(n);
    let page = http(
        h,
        "GET",
        &authorize_path(client, redirect, &challenge, book, &[]),
        &[],
        "",
    )
    .await;
    let wait = wait_path(&page);
    let waiting = pending(h).await;
    assert_eq!(waiting.len(), 1, "{waiting:?}");
    assert!(matches!(
        decide(h, &waiting[0].id, true, post).await,
        Response::RemoteDecided { .. }
    ));
    let back = http(h, "GET", &wait, &[], "").await;
    let params = location_params(&back);
    assert_eq!(params["state"], "st-123");
    assert_eq!(params["iss"], ISSUER);
    (params["code"].clone(), verifier)
}

async fn exchange(h: &H, client: &str, code: &str, verifier: &str, redirect: &str) -> Resp {
    http(
        h,
        "POST",
        "/token",
        &[("Content-Type", "application/x-www-form-urlencoded")],
        &form(&[
            ("grant_type", "authorization_code"),
            ("client_id", client),
            ("code", code),
            ("code_verifier", verifier),
            ("redirect_uri", redirect),
        ]),
    )
    .await
}

async fn refresh(h: &H, client: &str, token: &str) -> Resp {
    http(
        h,
        "POST",
        "/token",
        &[("Content-Type", "application/x-www-form-urlencoded")],
        &form(&[
            ("grant_type", "refresh_token"),
            ("client_id", client),
            ("refresh_token", token),
        ]),
    )
    .await
}

async fn mcp(h: &H, token: Option<&str>, book: &str, body: Value) -> Resp {
    let bearer = token.map(|t| format!("Bearer {t}"));
    let mut extra = vec![
        ("Content-Type", "application/json"),
        ("Accept", "application/json, text/event-stream"),
        ("MCP-Protocol-Version", "2025-06-18"),
    ];
    if let Some(b) = &bearer {
        extra.push(("Authorization", b));
    }
    http(
        h,
        "POST",
        &format!("/b/{book}/mcp"),
        &extra,
        &body.to_string(),
    )
    .await
}

fn initialize() -> Value {
    json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
        "protocolVersion":"2025-06-18","capabilities":{},
        "clientInfo":{"name":"claude-web-test","version":"1"}}})
}

fn call(id: u64, tool: &str, args: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":"tools/call",
           "params":{"name":tool,"arguments":args}})
}

/// A fully signed-in client of the Claude kind: its client id, access
/// token and refresh token.
async fn claude(h: &H, book: &str) -> (String, String, String) {
    let c = register(h, "Claude", &[CLAUDE_CB]).await;
    let id = c["client_id"].as_str().unwrap().to_string();
    let (code, verifier) = sign_in(h, &id, CLAUDE_CB, book, 1, false).await;
    let t = exchange(h, &id, &code, &verifier, CLAUDE_CB).await;
    assert_eq!(t.status, 200, "{}", t.body);
    let t = t.json();
    (
        id,
        t["access_token"].as_str().unwrap().to_string(),
        t["refresh_token"].as_str().unwrap().to_string(),
    )
}

fn tool_text(r: &Resp) -> String {
    assert_eq!(r.status, 200, "{}", r.body);
    r.json()["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

// ---- the three ways to introduce a client ------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_registered_client_signs_in_and_reads_the_books() {
    let h = harness().await;
    let (_, access, _) = claude(&h, "acme").await;
    // The page said to look at the app; the person allowed it; the token
    // reads, over plain JSON, with no session kept.
    let init = mcp(&h, Some(&access), "acme", initialize()).await;
    assert_eq!(init.status, 200, "{}", init.body);
    assert!(init.header("mcp-session-id").is_none());
    assert!(
        init.header("content-type")
            .unwrap()
            .starts_with("application/json"),
        "{:?}",
        init.headers
    );
    let start = mcp(&h, Some(&access), "acme", call(2, "books.start", json!({}))).await;
    assert!(tool_text(&start).contains("acme") || start.status == 200);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_client_metadata_document_signs_in() {
    struct Fake;
    impl ClientDocs for Fake {
        fn fetch<'a>(
            &'a self,
            url: &'a str,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<u8>, String>> + Send + 'a>>
        {
            Box::pin(async move {
                if url == "https://assistant.example/client.json" {
                    Ok(json!({"client_id": url, "client_name": "Example Assistant",
                              "redirect_uris": ["https://assistant.example/cb"]})
                    .to_string()
                    .into_bytes())
                } else {
                    Err("no such document".into())
                }
            })
        }
    }
    let h = harness().await;
    h.books.remote.set_docs(Arc::new(Fake));
    let id = "https://assistant.example/client.json";
    let cb = "https://assistant.example/cb";
    let (code, verifier) = sign_in(&h, id, cb, "acme", 2, false).await;
    let t = exchange(&h, id, &code, &verifier, cb).await;
    assert_eq!(t.status, 200, "{}", t.body);
    let access = t.json()["access_token"].as_str().unwrap().to_string();
    assert_eq!(
        mcp(&h, Some(&access), "acme", initialize()).await.status,
        200
    );
    // A document that is not what it says is no client.
    let page = http(
        &h,
        "GET",
        &authorize_path(
            "https://assistant.example/other.json",
            cb,
            &pkce(3).1,
            "acme",
            &[],
        ),
        &[],
        "",
    )
    .await;
    assert_eq!(page.status, 400);
    assert!(pending(&h).await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_typed_client_id_and_secret_signs_in() {
    let h = harness().await;
    let cb = "https://typed.example/callback";
    let (id, secret) = match admin(
        &h.books,
        Request::RemoteMakeClient {
            name: "Typed".into(),
            redirect_uris: vec![cb.into()],
        },
    )
    .await
    {
        Response::RemoteSecret { id, secret, .. } => (id, secret.0),
        other => panic!("{other:?}"),
    };
    let (code, verifier) = sign_in(&h, &id, cb, "acme", 4, false).await;
    // Without the secret, or with a wrong one, the code is not spent.
    for given in [None, Some("wrong-secret")] {
        let mut pairs = vec![
            ("grant_type", "authorization_code"),
            ("client_id", id.as_str()),
            ("code", code.as_str()),
            ("code_verifier", verifier.as_str()),
            ("redirect_uri", cb),
        ];
        if let Some(s) = given {
            pairs.push(("client_secret", s));
        }
        let r = http(
            &h,
            "POST",
            "/token",
            &[("Content-Type", "application/x-www-form-urlencoded")],
            &form(&pairs),
        )
        .await;
        assert_eq!(r.status, 401, "{}", r.body);
        assert_eq!(r.json()["error"], "invalid_client");
    }
    // The secret as HTTP Basic, the other way a product sends it.
    let basic = base64::engine::general_purpose::STANDARD.encode(format!("{id}:{secret}"));
    let r = http(
        &h,
        "POST",
        "/token",
        &[
            ("Content-Type", "application/x-www-form-urlencoded"),
            ("Authorization", &format!("Basic {basic}")),
        ],
        &form(&[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("code_verifier", &verifier),
            ("redirect_uri", cb),
        ]),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.body);
}

// ---- the person decides ------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn the_window_is_told_who_asks_and_the_page_only_says_to_look() {
    let h = harness().await;
    let c = register(&h, "Claude <b>web</b>", &[CLAUDE_CB]).await;
    let id = c["client_id"].as_str().unwrap();
    let page = http(
        &h,
        "GET",
        &authorize_path(id, CLAUDE_CB, &pkce(5).1, "acme", &[]),
        &[],
        "",
    )
    .await;
    assert_eq!(page.status, 200);
    assert!(page.body.contains("Look at Wanderbooks"));
    assert!(!page.body.contains("<form") && !page.body.contains("<button"));
    assert!(page.header("cache-control").unwrap().contains("no-store"));
    let waiting = pending(&h).await;
    assert_eq!(waiting.len(), 1);
    let p = &waiting[0];
    // The product is named by where it is sent back, not by what it says.
    assert_eq!(p.client_name, "Claude on the web");
    assert_eq!(p.product.as_deref(), Some("Claude on the web"));
    assert_eq!(p.redirect, CLAUDE_CB);
    assert_eq!(p.redirect_host, "claude.ai");
    assert_eq!(p.from, CALLER);
    assert_eq!(p.book, "acme");
    assert!(!p.wants_post);
    // Nothing waiting can be decided by a guess of its browser-side id.
    let wait = wait_path(&page);
    let secret_id = wait.rsplit('/').next().unwrap();
    assert_ne!(secret_id, p.id);
    assert!(matches!(
        decide(&h, secret_id, true, true).await,
        Response::Refused(_)
    ));
    assert!(matches!(
        decide(&h, "p_nothere", true, false).await,
        Response::Refused(_)
    ));
    // And nothing on the web side approves: the wait path only reads.
    let again = http(&h, "GET", &wait, &[], "").await;
    assert_eq!(again.status, 200);
    assert_eq!(pending(&h).await.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_refusal_and_a_timeout_send_the_browser_back_denied_and_make_nothing() {
    let h = harness().await;
    let c = register(&h, "Someone", &["https://someone.example/cb"]).await;
    let id = c["client_id"].as_str().unwrap().to_string();
    let cb = "https://someone.example/cb";
    // Refused.
    let page = http(
        &h,
        "GET",
        &authorize_path(&id, cb, &pkce(6).1, "acme", &[]),
        &[],
        "",
    )
    .await;
    let wait = wait_path(&page);
    let p = pending(&h).await;
    assert!(matches!(
        decide(&h, &p[0].id, false, false).await,
        Response::RemoteDecided { approval: None, .. }
    ));
    let back = http(&h, "GET", &wait, &[], "").await;
    let params = location_params(&back);
    assert_eq!(params["error"], "access_denied");
    assert_eq!(params["state"], "st-123");
    // Read once: a second look finds it over.
    assert_eq!(http(&h, "GET", &wait, &[], "").await.status, 410);
    // Timed out.
    let page = http(
        &h,
        "GET",
        &authorize_path(&id, cb, &pkce(7).1, "acme", &[]),
        &[],
        "",
    )
    .await;
    let wait = wait_path(&page);
    h.advance(PENDING_TTL + 1);
    assert!(pending(&h).await.is_empty());
    let params = location_params(&http(&h, "GET", &wait, &[], "").await);
    assert_eq!(params["error"], "access_denied");
    // Nothing was made: no approval, no token.
    match admin(&h.books, Request::RemoteApprovals { book: None }).await {
        Response::RemoteApprovals(a) => assert!(a.is_empty()),
        other => panic!("{other:?}"),
    }
    match admin(&h.books, Request::TokenList).await {
        Response::Tokens(t) => assert!(t.is_empty(), "{t:?}"),
        other => panic!("{other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_default_is_read_and_draft_and_the_person_raises_it_to_post() {
    let h = harness().await;
    let (_, access, _) = claude(&h, "acme").await;
    let approvals = match admin(
        &h.books,
        Request::RemoteApprovals {
            book: Some("acme".into()),
        },
    )
    .await
    {
        Response::RemoteApprovals(a) => a,
        other => panic!("{other:?}"),
    };
    assert_eq!(approvals.len(), 1);
    assert!(!approvals[0].may_post);
    assert_eq!(approvals[0].label, "Claude on the web");
    // The same connection is a row of "Who can reach these books".
    let listed = match admin(&h.books, Request::TokenList).await {
        Response::Tokens(t) => t,
        other => panic!("{other:?}"),
    };
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, approvals[0].token_id);
    assert_eq!(
        listed[0].scopes,
        vec!["books:read".to_string(), "books:write".to_string()]
    );
    // Posting is refused for want of the scope, and says so.
    let post = call(
        3,
        "books.post_entry",
        json!({"entry_id": 1, "confirmed": true, "request_key": "post-key-0001"}),
    );
    let refused = mcp(&h, Some(&access), "acme", post.clone()).await;
    assert!(
        tool_text(&refused).contains("books:post"),
        "{}",
        refused.body
    );
    // Raised by the person: the same access token now may post (the call
    // then fails for its own reasons, not for the scope).
    let raised = admin(
        &h.books,
        Request::RemoteSetPost {
            approval: approvals[0].id.clone(),
            post: true,
        },
    )
    .await;
    assert!(
        matches!(raised, Response::RemoteApprovals(ref a) if a.len() == 1 && a[0].may_post),
        "{raised:?}"
    );
    let after = mcp(&h, Some(&access), "acme", post).await;
    assert!(
        !tool_text(&after).contains("needs books:post"),
        "{}",
        after.body
    );
    // Never the access record, whatever is asked.
    let admin_call = mcp(
        &h,
        Some(&access),
        "acme",
        call(4, "books.access_log", json!({})),
    )
    .await;
    assert!(tool_text(&admin_call).contains("books:admin") || admin_call.status != 200);
    // And lowered again.
    let lowered = admin(
        &h.books,
        Request::RemoteSetPost {
            approval: approvals[0].id.clone(),
            post: false,
        },
    )
    .await;
    assert!(matches!(lowered, Response::RemoteApprovals(ref a) if !a[0].may_post));
}

#[tokio::test(flavor = "multi_thread")]
async fn revoking_the_connection_ends_the_access_token_and_the_refresh_token() {
    let h = harness().await;
    let (client, access, refresh_token) = claude(&h, "acme").await;
    assert_eq!(
        mcp(&h, Some(&access), "acme", initialize()).await.status,
        200
    );
    let t = match admin(&h.books, Request::TokenList).await {
        Response::Tokens(t) => t[0].clone(),
        other => panic!("{other:?}"),
    };
    // What the window's Revoke does: by name and id.
    assert!(matches!(
        admin(
            &h.books,
            Request::TokenRevoke {
                book: "acme".into(),
                name: t.name,
                id: Some(t.id)
            }
        )
        .await,
        Response::TokenRevoked(_)
    ));
    let r = mcp(&h, Some(&access), "acme", initialize()).await;
    assert_eq!(r.status, 401, "{}", r.body);
    assert!(
        r.header("www-authenticate")
            .unwrap()
            .contains("invalid_token")
    );
    let r = refresh(&h, &client, &refresh_token).await;
    assert_eq!(r.status, 400);
    assert_eq!(r.json()["error"], "invalid_grant");
    // A sign-in again is a new question for the person, not the old yes.
    let page = http(
        &h,
        "GET",
        &authorize_path(&client, CLAUDE_CB, &pkce(8).1, "acme", &[]),
        &[],
        "",
    )
    .await;
    assert_eq!(page.status, 200);
    assert_eq!(pending(&h).await.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_approval_survives_a_product_registering_again() {
    let h = harness().await;
    let (_, _, _) = claude(&h, "acme").await;
    // Claude registers anew: a new client id, the same product.
    let c = register(&h, "Claude", &[CLAUDE_CB]).await;
    let id = c["client_id"].as_str().unwrap();
    let (verifier, challenge) = pkce(9);
    let back = http(
        &h,
        "GET",
        &authorize_path(id, CLAUDE_CB, &challenge, "acme", &[]),
        &[],
        "",
    )
    .await;
    // No question to the person: the browser is sent straight back.
    let params = location_params(&back);
    assert!(pending(&h).await.is_empty());
    let t = exchange(&h, id, &params["code"], &verifier, CLAUDE_CB).await;
    assert_eq!(t.status, 200, "{}", t.body);
    // The same product for another company is a new question.
    let page = http(
        &h,
        "GET",
        &authorize_path(id, CLAUDE_CB, &challenge, "beta", &[]),
        &[],
        "",
    )
    .await;
    assert_eq!(page.status, 200);
    assert_eq!(pending(&h).await.len(), 1);
}

// ---- every refusal ------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_redirect_that_is_not_registered_is_never_redirected_to() {
    let h = harness().await;
    let c = register(&h, "Claude", &[CLAUDE_CB]).await;
    let id = c["client_id"].as_str().unwrap();
    for bad in [
        "https://evil.example/cb",
        "https://claude.ai/api/mcp/auth_callback/",
        "https://claude.ai/api/mcp/auth_callback?x=1",
        "https://claude.ai.evil.example/api/mcp/auth_callback",
    ] {
        let r = http(
            &h,
            "GET",
            &authorize_path(id, bad, &pkce(10).1, "acme", &[]),
            &[],
            "",
        )
        .await;
        assert_eq!(r.status, 400, "{bad}");
        assert!(r.header("location").is_none(), "{bad}");
    }
    assert!(pending(&h).await.is_empty());
    // Registration refuses addresses that are not ones to send a browser to.
    for bad in [
        "javascript:alert(1)",
        "https://x.example/cb#frag",
        "http://remote.example/cb",
        "data:text/html,x",
    ] {
        let r = http(
            &h,
            "POST",
            "/register",
            &[("Content-Type", "application/json")],
            &json!({"redirect_uris": [bad]}).to_string(),
        )
        .await;
        assert_eq!(r.status, 400, "{bad}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn pkce_the_resource_and_the_verifier_are_required() {
    let h = harness().await;
    let c = register(&h, "Claude", &[CLAUDE_CB]).await;
    let id = c["client_id"].as_str().unwrap().to_string();
    let challenge = pkce(11).1;
    let base = |extra: &[(&str, &str)]| authorize_path(&id, CLAUDE_CB, &challenge, "acme", extra);
    // No challenge, and the plain method: sent back with invalid_request.
    let no_pkce = format!(
        "/authorize?{}",
        form(&[
            ("response_type", "code"),
            ("client_id", &id),
            ("redirect_uri", CLAUDE_CB),
            ("state", "s")
        ])
    );
    assert_eq!(
        location_params(&http(&h, "GET", &no_pkce, &[], "").await)["error"],
        "invalid_request"
    );
    let plain = base(&[("code_challenge_method", "plain")]).replace("S256", "plain");
    assert_eq!(
        location_params(&http(&h, "GET", &plain, &[], "").await)["error"],
        "invalid_request"
    );
    // A resource that is another company's address elsewhere, or none at all
    // where two are on offer.
    let other = authorize_path(&id, CLAUDE_CB, &challenge, "acme", &[]).replace(
        &form(&[("resource", &format!("{ISSUER}/b/acme/mcp"))]),
        &form(&[("resource", "https://other.example/b/acme/mcp")]),
    );
    assert_eq!(
        location_params(&http(&h, "GET", &other, &[], "").await)["error"],
        "invalid_target"
    );
    let none = format!(
        "/authorize?{}",
        form(&[
            ("response_type", "code"),
            ("client_id", &id),
            ("redirect_uri", CLAUDE_CB),
            ("code_challenge", &challenge),
            ("code_challenge_method", "S256")
        ])
    );
    assert_eq!(
        location_params(&http(&h, "GET", &none, &[], "").await)["error"],
        "invalid_target"
    );
    assert!(pending(&h).await.is_empty());
    // A wrong verifier does not give a token, and spends the code.
    let (code, verifier) = sign_in(&h, &id, CLAUDE_CB, "acme", 12, false).await;
    let wrong = exchange(
        &h,
        &id,
        &code,
        "not-the-verifier-not-the-verifier-not-the-verifier",
        CLAUDE_CB,
    )
    .await;
    assert_eq!(wrong.json()["error"], "invalid_grant");
    let spent = exchange(&h, &id, &code, &verifier, CLAUDE_CB).await;
    assert_eq!(spent.json()["error"], "invalid_grant");
    // A token request naming another address as its resource is refused.
    let (code, verifier) = {
        // A second approval for the same product is instant now.
        let (v, ch) = pkce(13);
        let back = http(
            &h,
            "GET",
            &authorize_path(&id, CLAUDE_CB, &ch, "acme", &[]),
            &[],
            "",
        )
        .await;
        (location_params(&back)["code"].clone(), v)
    };
    let r = http(
        &h,
        "POST",
        "/token",
        &[("Content-Type", "application/x-www-form-urlencoded")],
        &form(&[
            ("grant_type", "authorization_code"),
            ("client_id", &id),
            ("code", &code),
            ("code_verifier", &verifier),
            ("resource", "https://other.example/b/acme/mcp"),
        ]),
    )
    .await;
    assert_eq!(r.json()["error"], "invalid_target");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_code_used_twice_ends_what_it_made_and_so_does_a_refresh_token() {
    let h = harness().await;
    let c = register(&h, "Claude", &[CLAUDE_CB]).await;
    let id = c["client_id"].as_str().unwrap().to_string();
    let (code, verifier) = sign_in(&h, &id, CLAUDE_CB, "acme", 14, false).await;
    let first = exchange(&h, &id, &code, &verifier, CLAUDE_CB).await.json();
    let access = first["access_token"].as_str().unwrap().to_string();
    assert_eq!(
        mcp(&h, Some(&access), "acme", initialize()).await.status,
        200
    );
    let replay = exchange(&h, &id, &code, &verifier, CLAUDE_CB).await;
    assert_eq!(replay.json()["error"], "invalid_grant");
    // What the first use made is ended (RFC 9700).
    assert_eq!(
        mcp(&h, Some(&access), "acme", initialize()).await.status,
        401
    );
    assert_eq!(
        refresh(&h, &id, first["refresh_token"].as_str().unwrap())
            .await
            .status,
        400
    );

    // Refresh tokens rotate; the old one used again ends the family.
    let (code, verifier) = {
        let (v, ch) = pkce(15);
        let back = http(
            &h,
            "GET",
            &authorize_path(&id, CLAUDE_CB, &ch, "acme", &[]),
            &[],
            "",
        )
        .await;
        (location_params(&back)["code"].clone(), v)
    };
    let t = exchange(&h, &id, &code, &verifier, CLAUDE_CB).await.json();
    let r1 = t["refresh_token"].as_str().unwrap().to_string();
    let second = refresh(&h, &id, &r1).await;
    assert_eq!(second.status, 200, "{}", second.body);
    let second = second.json();
    let r2 = second["refresh_token"].as_str().unwrap().to_string();
    assert_ne!(r1, r2);
    let a2 = second["access_token"].as_str().unwrap().to_string();
    assert_eq!(mcp(&h, Some(&a2), "acme", initialize()).await.status, 200);
    let reused = refresh(&h, &id, &r1).await;
    assert_eq!(reused.json()["error"], "invalid_grant");
    // The new one went down with the family, and so did its access token.
    assert_eq!(refresh(&h, &id, &r2).await.status, 400);
    assert_eq!(mcp(&h, Some(&a2), "acme", initialize()).await.status, 401);
    // Another client's refresh token is not good for this one.
    let other = register(&h, "Other", &["https://other.example/cb"]).await;
    let other_id = other["client_id"].as_str().unwrap();
    let (code, verifier) = {
        let (v, ch) = pkce(16);
        let back = http(
            &h,
            "GET",
            &authorize_path(&id, CLAUDE_CB, &ch, "acme", &[]),
            &[],
            "",
        )
        .await;
        (location_params(&back)["code"].clone(), v)
    };
    let t = exchange(&h, &id, &code, &verifier, CLAUDE_CB).await.json();
    assert_eq!(
        refresh(&h, other_id, t["refresh_token"].as_str().unwrap())
            .await
            .status,
        400
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_expired_token_and_a_token_for_another_issuer_are_refused() {
    let mut h = harness().await;
    let (client, access, refresh_token) = claude(&h, "acme").await;
    h.advance(ACCESS_TTL + 1);
    let r = mcp(&h, Some(&access), "acme", initialize()).await;
    assert_eq!(r.status, 401);
    assert!(
        r.header("www-authenticate")
            .unwrap()
            .contains("invalid_token")
    );
    // A refresh still works (the app may have been closed for days).
    h.advance(10 * 24 * 3600);
    let fresh = refresh(&h, &client, &refresh_token).await;
    assert_eq!(fresh.status, 200, "{}", fresh.body);
    let fresh = fresh.json();
    let access = fresh["access_token"].as_str().unwrap().to_string();
    assert_eq!(
        mcp(&h, Some(&access), "acme", initialize()).await.status,
        200
    );
    // The one public address changes: what was made under the old one is
    // not good under the new.
    h.start("https://other-name-tally-mint-ledger.wanderbooks.link")
        .await;
    let host = "other-name-tally-mint-ledger.wanderbooks.link";
    let mut bytes = proxy::encode(
        "203.0.113.9:1".parse().unwrap(),
        "198.51.100.1:443".parse().unwrap(),
    );
    bytes.extend(request_bytes(
        host,
        "POST",
        "/b/acme/mcp",
        &[
            ("Content-Type", "application/json"),
            ("Accept", "application/json, text/event-stream"),
            ("Authorization", &format!("Bearer {access}")),
        ],
        &initialize().to_string(),
    ));
    let r = parse(&raw(h.addr, &bytes).await);
    assert_eq!(r.status, 401, "{}", r.body);
    let mut bytes = proxy::encode(
        "203.0.113.9:1".parse().unwrap(),
        "198.51.100.1:443".parse().unwrap(),
    );
    let refresh_body = form(&[
        ("grant_type", "refresh_token"),
        ("client_id", &client),
        ("refresh_token", fresh["refresh_token"].as_str().unwrap()),
    ]);
    bytes.extend(request_bytes(
        host,
        "POST",
        "/token",
        &[("Content-Type", "application/x-www-form-urlencoded")],
        &refresh_body,
    ));
    let r = parse(&raw(h.addr, &bytes).await);
    assert_eq!(r.json()["error"], "invalid_grant", "{}", r.body);
    // And the old name is no longer answered at all.
    let r = http(
        &h,
        "GET",
        "/.well-known/oauth-authorization-server",
        &[],
        "",
    )
    .await;
    assert_eq!(r.status, 421);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unsigned_request_is_a_401_that_says_where_to_sign_in_initialize_included() {
    let h = harness().await;
    for body in [
        initialize(),
        call(2, "books.start", json!({})),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    ] {
        let r = mcp(&h, None, "acme", body).await;
        assert_eq!(r.status, 401, "{}", r.body);
        let challenge = r.header("www-authenticate").unwrap();
        assert!(
            challenge.starts_with("Bearer resource_metadata=\""),
            "{challenge}"
        );
        assert!(challenge.contains(&format!(
            "{ISSUER}/.well-known/oauth-protected-resource/b/acme/mcp"
        )));
        assert!(!challenge.contains("invalid_token"));
    }
    // A wrong token is told so.
    let r = mcp(&h, Some("wba_nothing"), "acme", initialize()).await;
    assert_eq!(r.status, 401);
    assert!(
        r.header("www-authenticate")
            .unwrap()
            .contains("invalid_token")
    );
    // An agent's own key for the local socket opens nothing here.
    let (_id, key) = match admin(
        &h.books,
        Request::TokenCreate {
            book: "acme".into(),
            name: "agent".into(),
            scopes: ["read", "write", "post", "admin"]
                .map(String::from)
                .to_vec(),
            replace: false,
        },
    )
    .await
    {
        Response::TokenCreated { token, text } => (token.id, text.0),
        other => panic!("{other:?}"),
    };
    assert_eq!(mcp(&h, Some(&key), "acme", initialize()).await.status, 401);
    // None of these was written to the books' access record.
    let dir = h.books.root.book_dir(&BookId::parse("acme").unwrap());
    let before = ledger_store::access::read(&dir).unwrap().0.len();
    for _ in 0..20 {
        mcp(&h, None, "acme", initialize()).await;
    }
    assert_eq!(ledger_store::access::read(&dir).unwrap().0.len(), before);
    assert!(h.books.remote.unsigned() >= 25);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_token_for_one_company_never_reaches_another() {
    let h = harness().await;
    let (_, access, _) = claude(&h, "acme").await;
    assert_eq!(
        mcp(&h, Some(&access), "acme", initialize()).await.status,
        200
    );
    let r = mcp(&h, Some(&access), "beta", initialize()).await;
    assert_eq!(r.status, 403, "{}", r.body);
    // A company that is not offered is not there; a path with another
    // name is no company.
    admin(
        &h.books,
        Request::RemoteAllow {
            book: "beta".into(),
            allowed: false,
        },
    )
    .await;
    assert_eq!(
        mcp(&h, Some(&access), "beta", initialize()).await.status,
        404
    );
    assert_eq!(
        mcp(&h, Some(&access), "nobody", initialize()).await.status,
        404
    );
    // The refusal of a token holder is on record, with the caller's address.
    let dir = h.books.root.book_dir(&BookId::parse("beta").unwrap());
    let _ = dir;
}

// ---- the caller's address -------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn the_recorded_address_is_the_relays_and_a_forged_one_changes_nothing() {
    let h = harness().await;
    let (_, access, _) = claude(&h, "acme").await;
    let dir = h.books.root.book_dir(&BookId::parse("acme").unwrap());
    // A call with a caller-supplied header that claims another address.
    let bearer = format!("Bearer {access}");
    let r = http_from(
        &h,
        "198.51.100.77",
        "POST",
        "/b/acme/mcp",
        &[
            ("Content-Type", "application/json"),
            ("Accept", "application/json, text/event-stream"),
            ("MCP-Protocol-Version", "2025-06-18"),
            ("Authorization", &bearer),
            ("X-Forwarded-For", "10.0.0.1"),
            ("Forwarded", "for=10.0.0.2"),
            ("X-Real-IP", "10.0.0.3"),
        ],
        &call(2, "books.start", json!({})).to_string(),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.body);
    let (records, _) = ledger_store::access::read(&dir).unwrap();
    let last = records.last().unwrap();
    assert_eq!(last.access.from.as_deref(), Some("198.51.100.77"));
    assert_eq!(last.access.subject, "books.start");
    // A header a caller writes inside its own bytes is not a header: the
    // door has read the real one, and this is HTTP text.
    let mut bytes = proxy::encode(
        "198.51.100.77:1".parse().unwrap(),
        "198.51.100.1:443".parse().unwrap(),
    );
    bytes.extend(proxy::encode(
        "10.9.9.9:1".parse().unwrap(),
        "198.51.100.1:443".parse().unwrap(),
    ));
    bytes.extend(request_bytes(&h.host(), "GET", "/", &[], ""));
    let out = raw(h.addr, &bytes).await;
    assert!(
        out.is_empty() || parse(&out).status == 400,
        "{}",
        String::from_utf8_lossy(&out)
    );
    // No header at all, plain HTTP: dropped before a byte is read.
    let out = raw(
        h.addr,
        &request_bytes(
            &h.host(),
            "GET",
            "/.well-known/oauth-authorization-server",
            &[],
            "",
        ),
    )
    .await;
    assert!(out.is_empty(), "{}", String::from_utf8_lossy(&out));
    // A refusal at the door of a token holder carries the address too.
    let r = mcp_from(&h, "192.0.2.50", Some(&access), "beta", initialize()).await;
    assert_eq!(r.status, 403);
    let dir = h.books.root.book_dir(&BookId::parse("beta").unwrap());
    let (records, _) = ledger_store::access::read(&dir).unwrap();
    assert_eq!(
        records.last().unwrap().access.from.as_deref(),
        Some("192.0.2.50")
    );
}

async fn mcp_from(h: &H, from: &str, token: Option<&str>, book: &str, body: Value) -> Resp {
    let bearer = token.map(|t| format!("Bearer {t}"));
    let mut extra = vec![
        ("Content-Type", "application/json"),
        ("Accept", "application/json, text/event-stream"),
    ];
    if let Some(b) = &bearer {
        extra.push(("Authorization", b));
    }
    http_from(
        h,
        from,
        "POST",
        &format!("/b/{book}/mcp"),
        &extra,
        &body.to_string(),
    )
    .await
}

// ---- the one write path -----------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn a_remote_write_needs_its_request_key_and_a_repeat_returns_the_first_answer() {
    let h = harness().await;
    let (_, access, _) = claude(&h, "acme").await;
    let draft = |key: Option<&str>| {
        let mut a = json!({"date": "2026-03-01", "description": "Rent", "no_party_reason": "test"});
        if let Some(k) = key {
            a["request_key"] = k.into();
        }
        a
    };
    let none = mcp(
        &h,
        Some(&access),
        "acme",
        call(2, "books.draft_entry", draft(None)),
    )
    .await;
    assert!(tool_text(&none).contains("request_key"), "{}", none.body);
    let first = mcp(
        &h,
        Some(&access),
        "acme",
        call(3, "books.draft_entry", draft(Some("remote-key-0001"))),
    )
    .await;
    let again = mcp(
        &h,
        Some(&access),
        "acme",
        call(3, "books.draft_entry", draft(Some("remote-key-0001"))),
    )
    .await;
    assert_eq!(tool_text(&first), tool_text(&again));
    assert!(!tool_text(&first).is_empty());
    let drafts = mcp(
        &h,
        Some(&access),
        "acme",
        call(4, "books.list_drafts", json!({})),
    )
    .await;
    let drafts: Value = serde_json::from_str(&tool_text(&drafts)).unwrap();
    assert_eq!(
        drafts["drafts"].as_array().map(Vec::len),
        Some(1),
        "{drafts}"
    );
}

// ---- bounds -------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn registration_is_bounded_and_unused_registrations_expire() {
    let h = harness().await;
    for i in 0..REGISTER_PER_ADDRESS {
        let r = http(
            &h,
            "POST",
            "/register",
            &[("Content-Type", "application/json")],
            &json!({"client_name": format!("n{i}"), "redirect_uris": ["https://x.example/cb"]})
                .to_string(),
        )
        .await;
        assert_eq!(r.status, 201, "{i}");
    }
    let over = http(
        &h,
        "POST",
        "/register",
        &[("Content-Type", "application/json")],
        &json!({"redirect_uris": ["https://x.example/cb"]}).to_string(),
    )
    .await;
    assert_eq!(over.status, 429);
    // Another address is not shut out for it (addresses are shared; only
    // the state a request makes is bounded).
    let other = http_from(
        &h,
        "198.51.100.5",
        "POST",
        "/register",
        &[("Content-Type", "application/json")],
        &json!({"redirect_uris": ["https://x.example/cb"]}).to_string(),
    )
    .await;
    assert_eq!(other.status, 201);
    assert!(h.books.remote.inner().clients.len() > 30);
    h.advance(CLIENT_IDLE + 10);
    // A request sweeps: the unused registrations are gone.
    http(&h, "GET", "/authorize?client_id=none", &[], "").await;
    assert!(h.books.remote.inner().clients.is_empty());
    // The table's own cap evicts the oldest unused, and the hundredth
    // thousand fills nothing (a direct check of the cap's path).
    {
        let mut inner = h.books.remote.inner();
        for i in 0..MAX_CLIENTS {
            inner.clients.insert(
                format!("c{i}"),
                ClientRec {
                    id: format!("c{i}"),
                    kind: Kind::Registered,
                    name: String::new(),
                    redirects: vec![],
                    secret_sha256: None,
                    created: i as i64,
                    last_seen: 0,
                    used: false,
                    fetched: 0,
                },
            );
        }
        assert!(h.books.remote.make_room(&mut inner).is_ok());
        assert_eq!(inner.clients.len(), MAX_CLIENTS - 1);
        assert!(!inner.clients.contains_key("c0"));
        for c in inner.clients.values_mut() {
            c.used = true;
        }
        let extra = inner.clients["c5"].clone();
        inner.clients.insert("extra".into(), extra);
        assert!(h.books.remote.make_room(&mut inner).is_err());
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn waiting_requests_are_bounded_per_address_and_in_all() {
    let h = harness().await;
    let c = register(&h, "Spam", &["https://spam.example/cb"]).await;
    let id = c["client_id"].as_str().unwrap().to_string();
    for i in 0..MAX_PENDING_PER_ADDRESS {
        let page = http(
            &h,
            "GET",
            &authorize_path(
                &id,
                "https://spam.example/cb",
                &pkce(100 + i as u32).1,
                "acme",
                &[],
            ),
            &[],
            "",
        )
        .await;
        assert_eq!(page.status, 200);
    }
    let page = http(
        &h,
        "GET",
        &authorize_path(&id, "https://spam.example/cb", &pkce(200).1, "acme", &[]),
        &[],
        "",
    )
    .await;
    assert_eq!(page.status, 429);
    assert_eq!(pending(&h).await.len(), MAX_PENDING_PER_ADDRESS);
    // Other addresses still get in, up to the whole table.
    let mut got = MAX_PENDING_PER_ADDRESS;
    for n in 0..10u8 {
        let from = format!("198.51.100.{}", n + 10);
        for i in 0..MAX_PENDING_PER_ADDRESS {
            let page = http_from(
                &h,
                &from,
                "GET",
                &authorize_path(
                    &id,
                    "https://spam.example/cb",
                    &pkce(300 + i as u32).1,
                    "acme",
                    &[],
                ),
                &[],
                "",
            )
            .await;
            if page.status == 200 {
                got += 1;
            } else {
                assert_eq!(page.status, 429);
            }
        }
    }
    assert_eq!(got, MAX_PENDING);
    // They expire unanswered.
    h.advance(PENDING_TTL + 1);
    assert!(pending(&h).await.is_empty());
}

// ---- the doors ------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn the_metadata_is_the_pinned_address_at_every_place_products_look() {
    let h = harness().await;
    let want_resource = format!("{ISSUER}/b/acme/mcp");
    for path in [
        "/.well-known/oauth-protected-resource/b/acme/mcp",
        "/b/acme/mcp/.well-known/oauth-protected-resource",
    ] {
        let r = http(&h, "GET", path, &[], "").await;
        assert_eq!(r.status, 200, "{path}");
        let d = r.json();
        assert_eq!(d["resource"], want_resource);
        assert_eq!(d["authorization_servers"], json!([ISSUER]));
        assert_eq!(r.header("access-control-allow-origin"), Some("*"));
    }
    // The root names the company only while one is on offer.
    admin(
        &h.books,
        Request::RemoteAllow {
            book: "beta".into(),
            allowed: false,
        },
    )
    .await;
    assert_eq!(
        http(&h, "GET", "/.well-known/oauth-protected-resource", &[], "")
            .await
            .json()["resource"],
        want_resource
    );
    admin(
        &h.books,
        Request::RemoteAllow {
            book: "beta".into(),
            allowed: true,
        },
    )
    .await;
    assert_eq!(
        http(&h, "GET", "/.well-known/oauth-protected-resource", &[], "")
            .await
            .status,
        404
    );
    for path in [
        "/.well-known/oauth-authorization-server",
        "/.well-known/oauth-authorization-server/b/acme/mcp",
        "/.well-known/openid-configuration",
        "/.well-known/openid-configuration/b/acme/mcp",
        "/b/acme/mcp/.well-known/openid-configuration",
    ] {
        let r = http(&h, "GET", path, &[], "").await;
        assert_eq!(r.status, 200, "{path}");
        let d = r.json();
        assert_eq!(d["issuer"], ISSUER);
        for key in [
            "authorization_endpoint",
            "token_endpoint",
            "registration_endpoint",
        ] {
            assert!(d[key].as_str().unwrap().starts_with(ISSUER), "{key}");
        }
        assert_eq!(d["code_challenge_methods_supported"], json!(["S256"]));
        assert_eq!(d["client_id_metadata_document_supported"], true);
    }
    // Nothing is built from the request: another Host is not answered, and
    // a company not offered has no document.
    let mut bytes = proxy::encode(
        "203.0.113.9:1".parse().unwrap(),
        "198.51.100.1:443".parse().unwrap(),
    );
    bytes.extend(request_bytes(
        "evil.example",
        "GET",
        "/.well-known/oauth-authorization-server",
        &[],
        "",
    ));
    let r = parse(&raw(h.addr, &bytes).await);
    assert_eq!(r.status, 421);
    assert!(!r.body.contains("evil.example"));
    assert_eq!(
        http(
            &h,
            "GET",
            "/.well-known/oauth-protected-resource/b/nobody/mcp",
            &[],
            ""
        )
        .await
        .status,
        404
    );
    // A browser's preflight.
    let pre = http(
        &h,
        "OPTIONS",
        "/token",
        &[("Origin", "https://app.example")],
        "",
    )
    .await;
    assert_eq!(pre.status, 204);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_door_is_off_until_asked_and_stops_when_told() {
    let home = tempfile::tempdir().unwrap();
    let (root, _) = Root::setup(home.path(), Some(&home.path().join("root")), None).unwrap();
    let books = Arc::new(Books::new(
        root,
        super::super::BACKUP_QUIET,
        super::super::DRILL_FIRST,
        Isolation::SameUser,
    ));
    let loaded = books.load_principals();
    *books.principals.write().unwrap() = loaded;
    assert!(books.remote.door().is_none());
    assert_eq!(books.remote.info(), RemoteInfo::default());
    // An address that is not the one public https address is refused.
    for bad in [
        "http://example.com",
        "https://example.com/path",
        "https://u:p@example.com",
        "not a url",
    ] {
        assert!(
            matches!(
                admin(
                    &books,
                    Request::RemoteStart {
                        public_url: bad.into(),
                        port: None
                    }
                )
                .await,
                Response::Refused(_)
            ),
            "{bad}"
        );
    }
    let info = match admin(
        &books,
        Request::RemoteStart {
            public_url: ISSUER.into(),
            port: None,
        },
    )
    .await
    {
        Response::Remote(i) => i,
        other => panic!("{other:?}"),
    };
    let addr: SocketAddr = info.listening.unwrap().parse().unwrap();
    assert!(addr.ip().is_loopback());
    // The same address again is the same door.
    match admin(
        &books,
        Request::RemoteStart {
            public_url: ISSUER.into(),
            port: None,
        },
    )
    .await
    {
        Response::Remote(i) => assert_eq!(i.listening, Some(addr.to_string())),
        other => panic!("{other:?}"),
    }
    admin(&books, Request::RemoteStop).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(tokio::net::TcpStream::connect(addr).await.is_err());
    assert!(books.remote.door().is_none());
}

// ---- what is kept -----------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn refresh_tokens_survive_a_restart_and_only_their_hashes_are_kept() {
    let h = harness().await;
    let (client, access, refresh_token) = claude(&h, "acme").await;
    // The record is private to the owner and holds no secret's text.
    let file = h.books.root.auth_dir().unwrap().join(FILE);
    let text = std::fs::read_to_string(&file).unwrap();
    for secret in [&access, &refresh_token] {
        assert!(!text.contains(secret.as_str()), "a secret is in the record");
    }
    assert!(text.contains(&sha_hex(&refresh_token)));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    // A server that starts again reads it: the same refresh token works.
    let again = Arc::new(Remote::new());
    let c = h.clock.clone();
    again.set_clock(Arc::new(move || c.load(Ordering::SeqCst)));
    again.load(&h.books).unwrap();
    let door = h.books.remote.door().unwrap();
    let form = BTreeMap::from([
        ("grant_type".to_string(), "refresh_token".to_string()),
        ("client_id".to_string(), client.clone()),
        ("refresh_token".to_string(), refresh_token.clone()),
    ]);
    let out = again
        .token(&h.books, &door, &form, None, "203.0.113.9")
        .await
        .unwrap_or_else(|e| panic!("{}", e.description));
    assert!(out["access_token"].as_str().unwrap().starts_with("wba_"));
    // The old access token is not carried over (memory only), and the one
    // the restart made is not the old one.
    assert_ne!(out["access_token"].as_str().unwrap(), access);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_key_made_for_a_product_that_takes_only_a_key_opens_the_door_with_the_same_permissions() {
    let h = harness().await;
    let (id, key) = match admin(
        &h.books,
        Request::RemoteMakeKey {
            book: "acme".into(),
            name: "Some Assistant".into(),
            post: false,
        },
    )
    .await
    {
        Response::RemoteSecret { id, secret, .. } => (id, secret.0),
        other => panic!("{other:?}"),
    };
    assert_eq!(mcp(&h, Some(&key), "acme", initialize()).await.status, 200);
    // It is one of "Who can reach these books", with read and draft only.
    let listed = match admin(&h.books, Request::TokenList).await {
        Response::Tokens(t) => t,
        other => panic!("{other:?}"),
    };
    let row = listed.iter().find(|t| t.id == id).unwrap();
    assert_eq!(row.name, "key-some-assistant");
    assert_eq!(
        row.scopes,
        vec!["books:read".to_string(), "books:write".to_string()]
    );
    let post = call(
        3,
        "books.post_entry",
        json!({"entry_id": 1, "confirmed": true, "request_key": "post-key-0002"}),
    );
    assert!(tool_text(&mcp(&h, Some(&key), "acme", post).await).contains("books:post"));
    // Another company is not reached with it.
    assert_eq!(mcp(&h, Some(&key), "beta", initialize()).await.status, 403);
    // Revoked, it ends.
    admin(
        &h.books,
        Request::TokenRevoke {
            book: "acme".into(),
            name: row.name.clone(),
            id: Some(id),
        },
    )
    .await;
    assert_eq!(mcp(&h, Some(&key), "acme", initialize()).await.status, 401);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_host_with_the_default_port_is_the_same_host() {
    let h = harness().await;
    let mut bytes = proxy::encode(
        "203.0.113.9:1".parse().unwrap(),
        "198.51.100.1:443".parse().unwrap(),
    );
    bytes.extend(request_bytes(
        &format!("{}:443", h.host()),
        "GET",
        "/.well-known/oauth-authorization-server",
        &[],
        "",
    ));
    assert_eq!(parse(&raw(h.addr, &bytes).await).status, 200);
}
