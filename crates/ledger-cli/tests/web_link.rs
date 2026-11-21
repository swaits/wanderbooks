//! Web assistants end to end (plan P15 unit B3; WB-link-3, WB-link-6):
//! the real books server (a child process of the test, on a data root of
//! its own), the real remote door and its sign-in, the app's link
//! (`ledger-host` `web.rs` over `ledger-link`), an in-process relay (the
//! relay crate's own) and an in-process certificate authority. A web
//! caller's TLS connection goes through the relay to the app, which ends
//! it with its own certificate and hands it to the door; the caller signs
//! in (the person's answer is made as the window makes it, through the
//! admin socket) and reads the books. Nothing leaves 127.0.0.1.
//!
//! ```text
//! GOAL: a person turns web assistants on for a company; a web assistant
//!       reaches the books through the relay at one durable address,
//!       signs in with the person's approval and reads them; turning it
//!       off closes the line and the door; a new address refuses what the
//!       old one signed in; what was on is on again at the next start.
//! SOURCE: plan/p15-app-and-relay.md 3.3, 4, 5; WB-link-3, WB-link-6,
//!         WB-remote-1 to WB-remote-6.
//! INVARIANTS:
//!   1. The caller's TLS ends in the app: the certificate it is shown is
//!      the app's own, from the authority, never the relay's.
//!   2. The door is on 127.0.0.1 and every connection to it begins with
//!      its per-start secret: a connection made straight to it without
//!      the secret is dropped; one through the relay is served.
//!   3. The sign-in is the door's own: nothing is approved but by the
//!      person's answer, and the address of the caller the door records is
//!      the one the relay saw.
//!   4. Off closes the line and the door; the address is kept and the same
//!      on again; a new address refuses every token of the old one.
//! SCENARIOS:
//!   1. See the tests below.
//! ```

mod common;
#[path = "../../relay/tests/common/mod.rs"]
mod relay_common;

use base64::Engine as _;
use common::{BOOK, Home, clean_env};
use futures::future::BoxFuture;
use ledger_host::app_server::AppServer;
use ledger_host::web::{Admin, WebAssistants};
use ledger_link::{CertTrust, Dial, Issued, Issuer, LinkConfig, OrderRequest, Step, Timing};
use ledger_mcp::admin::{Request, Response};
use rcgen::{CertificateParams, CertifiedIssuer, IsCa, KeyPair, KeyUsagePurpose};
use relay::proto::Authority;
use rustls::pki_types::ServerName;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use zeroize::Zeroizing;

const CLAUDE_CB: &str = "https://claude.ai/api/mcp/auth_callback";

fn exe() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_wanderbooks"))
}

/// An in-process certificate authority: it takes the proof (which goes to
/// the relay, as a real one's would), and signs for the name asked.
struct Authority_ {
    ca: CertifiedIssuer<'static, KeyPair>,
    orders: std::sync::atomic::AtomicUsize,
}

impl Authority_ {
    fn new() -> Arc<Self> {
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        params.is_ca = IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign];
        let ca = CertifiedIssuer::self_signed(params, KeyPair::generate().unwrap()).unwrap();
        Arc::new(Authority_ {
            ca,
            orders: Default::default(),
        })
    }

    fn roots(&self) -> rustls::RootCertStore {
        let mut r = rustls::RootCertStore::empty();
        r.add(self.ca.der().clone()).unwrap();
        r
    }
}

impl Issuer for Authority_ {
    fn issue<'a>(&'a self, req: OrderRequest<'a>) -> BoxFuture<'a, Result<Issued, String>> {
        Box::pin(async move {
            self.orders
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            req.dns.publish(&"A".repeat(43)).await?;
            req.dns.clear().await;
            let mut params = CertificateParams::new(vec![req.identifier.clone()]).unwrap();
            let now = time::OffsetDateTime::now_utc();
            params.not_before = now - time::Duration::hours(1);
            params.not_after = now + time::Duration::days(60);
            let key = KeyPair::generate().unwrap();
            let cert = params.signed_by(&key, &self.ca).unwrap();
            Ok(Issued {
                chain_pem: format!("{}{}", cert.pem(), self.ca.pem()),
                key_pem: Zeroizing::new(key.serialize_pem()),
            })
        })
    }

    fn renewal_window<'a>(
        &'a self,
        _a: &'a Authority,
        _c: &'a str,
    ) -> BoxFuture<'a, Option<(i64, i64)>> {
        Box::pin(async { None })
    }
}

/// The server, as the window talks to it.
struct Srv<'a>(&'a AppServer);

impl Admin for Srv<'_> {
    fn admin(&self, request: Request) -> Result<Response, String> {
        self.0.call(request, Some(Duration::from_secs(30)))
    }
}

struct Rig {
    home: Home,
    server: AppServer,
    env: relay_common::Env,
    ca: Arc<Authority_>,
    web: WebAssistants,
}

fn start_server(home: &Home) -> AppServer {
    let dir = home.dir.path().to_path_buf();
    let server = AppServer::start_in(
        exe(),
        Some(&home.root()),
        &[],
        move |c: &mut std::process::Command| {
            clean_env(c, &dir);
            c.env("XDG_CONFIG_HOME", dir.join(".config"));
        },
    )
    .unwrap_or_else(|why| panic!("{why}"));
    server.wait_ready(Duration::from_secs(60)).unwrap();
    server
}

fn web_for(home: &Home, env: &relay_common::Env, ca: &Arc<Authority_>) -> WebAssistants {
    let (relay_tls, router, issuer) = (env.line_tls(), env.router(), ca.clone());
    let roots = ca.roots();
    WebAssistants::with_maker(
        Some(home.root()),
        Box::new(move |dir| {
            Ok(LinkConfig {
                dir,
                domain: relay_common::DOMAIN.into(),
                relay_host: relay_common::RELAY_HOST.into(),
                dial: Dial::Addr(router),
                relay_tls: relay_tls.clone(),
                issuer: issuer.clone(),
                trust: CertTrust::for_tests(roots.clone()),
                timing: Timing {
                    ping_every: Duration::from_millis(500),
                    backoff_min: Duration::from_millis(50),
                    backoff_max: Duration::from_millis(400),
                    ..Timing::default()
                },
            })
        }),
    )
}

async fn rig(tag: &str) -> Rig {
    let home = Home::new(tag);
    let server = start_server(&home);
    home.create(BOOK, "USD");
    server.open_book(BOOK).unwrap();
    let env = relay_common::start_with(
        relay_common::quick_limits(),
        relay::authorities::AuthorityList::from_toml(
            "[[authority]]\nid = \"test\"\ndirectory = \"https://ca.example/dir\"\ncaa_domain = \"ca.example\"\n",
        )
        .unwrap(),
    )
    .await;
    let ca = Authority_::new();
    let web = web_for(&home, &env, &ca);
    Rig {
        home,
        server,
        env,
        ca,
        web,
    }
}

fn wait_connected(r: &Rig) -> ledger_host::web::WebState {
    let end = Instant::now() + Duration::from_secs(40);
    loop {
        let s = r.web.state(BOOK);
        if s.step == Step::Connected {
            return s;
        }
        assert!(
            Instant::now() < end,
            "never connected: {:?} {:?}",
            s.step,
            s.problem
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn host_of(connector: &str) -> String {
    connector
        .trim_start_matches("https://")
        .split('/')
        .next()
        .unwrap()
        .to_owned()
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
        let (mut out, mut body) = (Vec::new(), rest);
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
    } else {
        rest.to_vec()
    };
    Resp {
        status,
        headers,
        body: String::from_utf8_lossy(&body).to_string(),
    }
}

fn tls(roots: rustls::RootCertStore, alpn: &[&[u8]]) -> TlsConnector {
    let mut cfg = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_root_certificates(roots)
    .with_no_client_auth();
    cfg.alpn_protocols = alpn.iter().map(|a| a.to_vec()).collect();
    TlsConnector::from(Arc::new(cfg))
}

/// One HTTP/1.1 request, as a web assistant's vendor would send it: TLS to
/// the relay's address with the app's host name, the whole answer.
async fn https(
    r: &Rig,
    host: &str,
    method: &str,
    path: &str,
    extra: &[(&str, &str)],
    body: &str,
) -> std::io::Result<Resp> {
    let tcp = TcpStream::connect(r.env.router()).await?;
    let mut t = tokio::time::timeout(
        Duration::from_secs(10),
        tls(r.ca.roots(), &[b"http/1.1"])
            .connect(ServerName::try_from(host.to_owned()).unwrap(), tcp),
    )
    .await
    .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "no handshake"))??;
    let mut text = format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n");
    for (k, v) in extra {
        text.push_str(&format!("{k}: {v}\r\n"));
    }
    text.push_str(&format!("Content-Length: {}\r\n\r\n{body}", body.len()));
    t.write_all(text.as_bytes()).await?;
    let mut out = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(20), t.read_to_end(&mut out)).await;
    Ok(parse(&out))
}

fn pct(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                char::from(b).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn form(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", pct(k), pct(v)))
        .collect::<Vec<_>>()
        .join("&")
}

fn pkce(n: u32) -> (String, String) {
    let verifier = format!("verifier-{n}-{}", "x".repeat(40));
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

/// The whole sign-in of a Claude-like client at `host`, the person's
/// answer made through the admin socket: the access token. The address
/// the door recorded for the request is returned too.
async fn sign_in(r: &Rig, host: &str, n: u32) -> (String, String) {
    let reg = https(
        r,
        host,
        "POST",
        "/register",
        &[("Content-Type", "application/json")],
        &json!({"client_name": "Claude", "redirect_uris": [CLAUDE_CB]}).to_string(),
    )
    .await
    .unwrap();
    assert_eq!(reg.status, 201, "{}", reg.body);
    let client = reg.json()["client_id"].as_str().unwrap().to_owned();
    let (verifier, challenge) = pkce(n);
    let resource = format!("https://{host}/b/{BOOK}/mcp");
    let path = format!(
        "/authorize?{}",
        form(&[
            ("response_type", "code"),
            ("client_id", &client),
            ("redirect_uri", CLAUDE_CB),
            ("code_challenge", &challenge),
            ("code_challenge_method", "S256"),
            ("state", "st-123"),
            ("resource", &resource),
            ("scope", "books:read books:write"),
        ])
    );
    let page = https(r, host, "GET", &path, &[], "").await.unwrap();
    assert_eq!(page.status, 200, "{}", page.body);
    let at = page
        .body
        .find("url=/authorize/wait/")
        .expect("a waiting page");
    let rest = &page.body[at + 4..];
    let wait = rest[..rest.find('"').unwrap()].to_string();
    // The person, in the window: the request is listed with the caller's
    // address, and allowed.
    let pending = match r
        .server
        .call(Request::RemotePending, Some(Duration::from_secs(30)))
        .unwrap()
    {
        Response::RemotePending(p) => p,
        other => panic!("{other:?}"),
    };
    assert_eq!(pending.len(), 1, "{pending:?}");
    let from = pending[0].from.clone();
    match r
        .server
        .call(
            Request::RemoteDecide {
                id: pending[0].id.clone(),
                approve: true,
                post: false,
            },
            Some(Duration::from_secs(30)),
        )
        .unwrap()
    {
        Response::RemoteDecided { .. } => {}
        other => panic!("{other:?}"),
    }
    let back = https(r, host, "GET", &wait, &[], "").await.unwrap();
    assert_eq!(back.status, 302, "{}", back.body);
    let location = back.header("location").unwrap().to_owned();
    let code = location
        .split(['?', '&'])
        .find_map(|p| p.strip_prefix("code="))
        .unwrap()
        .to_owned();
    let token = https(
        r,
        host,
        "POST",
        "/token",
        &[("Content-Type", "application/x-www-form-urlencoded")],
        &form(&[
            ("grant_type", "authorization_code"),
            ("client_id", &client),
            ("code", &code),
            ("code_verifier", &verifier),
            ("redirect_uri", CLAUDE_CB),
        ]),
    )
    .await
    .unwrap();
    assert_eq!(token.status, 200, "{}", token.body);
    (
        token.json()["access_token"].as_str().unwrap().to_owned(),
        from,
    )
}

async fn mcp(r: &Rig, host: &str, token: Option<&str>, body: Value) -> Resp {
    let bearer = token.map(|t| format!("Bearer {t}"));
    let mut extra = vec![
        ("Content-Type", "application/json"),
        ("Accept", "application/json, text/event-stream"),
        ("MCP-Protocol-Version", "2025-06-18"),
    ];
    if let Some(b) = &bearer {
        extra.push(("Authorization", b));
    }
    https(
        r,
        host,
        "POST",
        &format!("/b/{BOOK}/mcp"),
        &extra,
        &body.to_string(),
    )
    .await
    .unwrap()
}

fn initialize() -> Value {
    json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
        "protocolVersion":"2025-06-18","capabilities":{},
        "clientInfo":{"name":"claude-web-test","version":"1"}}})
}

fn on(r: &Rig) -> ledger_host::web::WebState {
    r.web.turn_on(&Srv(&r.server), BOOK).unwrap();
    wait_connected(r)
}

/// INVARIANTS 1 to 3: a web assistant signs in through the relay and reads
/// the books; the certificate it was shown is the app's; the door refuses a
/// connection that comes straight to it without the secret.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_web_assistant_signs_in_through_the_relay_and_reads_the_books() {
    let r = rig("web-e2e").await;
    let s = on(&r);
    let connector = s.connector.clone().unwrap();
    let host = host_of(&connector);
    assert!(connector.ends_with(&format!("/b/{BOOK}/mcp")));
    // Off by default, then on: one order for the certificate.
    assert_eq!(r.ca.orders.load(std::sync::atomic::Ordering::SeqCst), 1);

    // Unsigned: the door says where to sign in (a 401), through the relay.
    let unsigned = mcp(&r, &host, None, initialize()).await;
    assert_eq!(unsigned.status, 401, "{}", unsigned.body);
    assert!(unsigned.header("www-authenticate").unwrap().contains(&host));

    // Signed in, with the person's answer, the books are read.
    let (token, from) = sign_in(&r, &host, 1).await;
    assert!(
        from.starts_with("127.0.0.1"),
        "the door records the address the relay saw: {from}"
    );
    let init = mcp(&r, &host, Some(&token), initialize()).await;
    assert_eq!(init.status, 200, "{}", init.body);
    let start = mcp(
        &r,
        &host,
        Some(&token),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"books.start","arguments":{}}}),
    )
    .await;
    assert_eq!(start.status, 200, "{}", start.body);
    let text = start.json()["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert!(text.contains(BOOK) || !text.is_empty(), "{}", start.body);

    // The relay's own certificate is not the one the caller was shown: a
    // caller that trusts only the relay's CA cannot talk to the app.
    let tcp = TcpStream::connect(r.env.router()).await.unwrap();
    let wrong = tls(r.env.relay_ca.roots(), &[b"http/1.1"])
        .connect(ServerName::try_from(host.clone()).unwrap(), tcp)
        .await;
    assert!(wrong.is_err(), "the relay holds no certificate for the app");

    // The door, reached straight on 127.0.0.1 without the secret, drops the
    // connection before it says anything.
    let door: SocketAddr = match r
        .server
        .call(Request::RemoteStatus, Some(Duration::from_secs(30)))
        .unwrap()
    {
        Response::Remote(info) => info.listening.unwrap().parse().unwrap(),
        other => panic!("{other:?}"),
    };
    let mut straight = TcpStream::connect(door).await.unwrap();
    let _ = straight
        .write_all(b"GET /b/acme/mcp HTTP/1.1\r\nHost: x\r\n\r\n")
        .await;
    let mut got = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(10), straight.read_to_end(&mut got)).await;
    assert!(got.is_empty(), "no answer without the secret: {got:?}");
}

/// INVARIANT 2, HTTP/2: the same connection over h2, which has no Host
/// header but an authority, reaches the door too.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_door_answers_http2_by_its_authority() {
    let r = rig("web-h2").await;
    let host = host_of(&on(&r).connector.unwrap());
    let tcp = TcpStream::connect(r.env.router()).await.unwrap();
    let t = tls(r.ca.roots(), &[b"h2"])
        .connect(ServerName::try_from(host.clone()).unwrap(), tcp)
        .await
        .unwrap();
    assert_eq!(t.get_ref().1.alpn_protocol(), Some(&b"h2"[..]));
    let (mut send, conn) =
        hyper::client::conn::http2::handshake::<_, _, http_body_util::Empty<bytes::Bytes>>(
            hyper_util::rt::TokioExecutor::new(),
            hyper_util::rt::TokioIo::new(t),
        )
        .await
        .unwrap();
    tokio::spawn(conn);
    let req = hyper::Request::builder()
        .method("GET")
        .uri(format!("https://{host}/b/{BOOK}/mcp"))
        .body(http_body_util::Empty::new())
        .unwrap();
    let resp = tokio::time::timeout(Duration::from_secs(10), send.send_request(req))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(resp.status(), 401);
    assert!(
        resp.headers()["www-authenticate"]
            .to_str()
            .unwrap()
            .contains(&host)
    );
}

/// INVARIANT 4: off closes the line and the door and keeps the address; on
/// again is the same address and signs in again.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn off_closes_the_line_and_the_door_and_on_again_is_the_same_address() {
    let r = rig("web-off").await;
    let before = on(&r);
    let host = host_of(&before.connector.clone().unwrap());
    assert_eq!(r.env.relay.shared().lines.len(), 1);

    let said = r.web.turn_off(&Srv(&r.server), BOOK).unwrap();
    assert!(said.contains("kept"), "{said}");
    let s = r.web.state(BOOK);
    assert!(!s.on && s.step == Step::Off && s.kept);
    // The line is gone at the relay, and the door no longer listens.
    let end = Instant::now() + Duration::from_secs(10);
    while !r.env.relay.shared().lines.is_empty() {
        assert!(Instant::now() < end, "the relay still has the line");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    match r
        .server
        .call(Request::RemoteStatus, Some(Duration::from_secs(30)))
        .unwrap()
    {
        Response::Remote(info) => {
            assert!(
                info.listening.is_none() && info.public_url.is_none(),
                "{info:?}"
            );
        }
        other => panic!("{other:?}"),
    }
    assert!(
        tokio::time::timeout(Duration::from_secs(10), async {
            let tcp = TcpStream::connect(r.env.router()).await.unwrap();
            tls(r.ca.roots(), &[b"http/1.1"])
                .connect(ServerName::try_from(host.clone()).unwrap(), tcp)
                .await
        })
        .await
        .unwrap()
        .is_err(),
        "no line, no TLS"
    );

    // On again: the same address, and the sign-in works.
    let after = on(&r);
    assert_eq!(after.connector, before.connector);
    assert_eq!(
        r.ca.orders.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the certificate was kept"
    );
    let (token, _) = sign_in(&r, &host, 2).await;
    assert_eq!(mcp(&r, &host, Some(&token), initialize()).await.status, 200);
}

/// INVARIANT 4: a new address stops the old one, and every token made at
/// the old address is refused at the new.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_new_address_refuses_what_the_old_one_signed_in() {
    let r = rig("web-new").await;
    let old_connector = on(&r).connector.unwrap();
    let old = host_of(&old_connector);
    let (token, _) = sign_in(&r, &old, 3).await;
    assert_eq!(mcp(&r, &old, Some(&token), initialize()).await.status, 200);

    let said = r.web.new_address(&Srv(&r.server)).unwrap();
    assert!(said.contains("old one has stopped working"), "{said}");
    let new_connector = wait_connected(&r).connector.unwrap();
    assert_ne!(new_connector, old_connector);
    let new = host_of(&new_connector);
    // The old address gets no TLS at all.
    let tcp = TcpStream::connect(r.env.router()).await.unwrap();
    assert!(
        tls(r.ca.roots(), &[b"http/1.1"])
            .connect(ServerName::try_from(old.clone()).unwrap(), tcp)
            .await
            .is_err()
    );
    // The old token at the new address is refused; a new sign-in works.
    assert_eq!(mcp(&r, &new, Some(&token), initialize()).await.status, 401);
    let (fresh, _) = sign_in(&r, &new, 4).await;
    assert_eq!(mcp(&r, &new, Some(&fresh), initialize()).await.status, 200);
    assert_eq!(
        r.ca.orders.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the wildcard certificate serves the new label"
    );
}

/// INVARIANT 4 (WB-link-6): what was on when the app closed is on again at
/// the next start, for the company that is open, at the same address.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_switches_are_put_back_at_the_next_start() {
    let mut r = rig("web-restore").await;
    let before = on(&r).connector.unwrap();
    // The app closes: the line ends with it, the switch stays on disk.
    r.web.shutdown();
    drop(std::mem::replace(
        &mut r.web,
        web_for(&r.home, &r.env, &r.ca),
    ));
    assert!(
        !r.web.state(BOOK).on,
        "a new start begins off, until put back"
    );
    r.web.restore(&Srv(&r.server), &[BOOK.to_string()]);
    let after = wait_connected(&r).connector.unwrap();
    assert_eq!(after, before);
    assert_eq!(r.ca.orders.load(std::sync::atomic::Ordering::SeqCst), 1);
    // A company that is not open waits for its tab: nothing is put back.
    let r2 = rig("web-restore-none").await;
    r2.web.restore(&Srv(&r2.server), &[BOOK.to_string()]);
    assert!(!r2.web.state(BOOK).on, "never turned on, never put back");
}
