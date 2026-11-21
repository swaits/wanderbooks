//! The remote door's HTTP side: the listener, the routes (metadata,
//! register, authorize, token, a company's MCP path), and the answers.
//!
//! ```text
//! GOAL: every route of the remote door answers only for the one public
//!       address, in the forms the products expect, and refuses the rest.
//! SOURCE: WB-remote-1 to WB-remote-4, WB-remote-8; RFC 9728 and RFC 8414
//!         (metadata and where it is found, including the forms products
//!         use that the specification only half allows), RFC 6750 (the
//!         401 and its challenge); plan/p15-app-and-relay.md 4 rules 1, 4, 5.
//! INVARIANTS:
//!   1. A request is answered only while the door is on, and only if its
//!      Host is the public address's host (421 otherwise); a request that
//!      has an Origin must have the public address's (403).
//!   2. Every address in a metadata document is built from the public
//!      address the app gave and from nothing in the request.
//!   3. The metadata is served at every place the products look: the
//!      protected-resource document at the root, after the well-known
//!      path with the resource's path appended, and before it; the
//!      authorization server's at its two well-known names, with a path
//!      appended or prepended too.
//!   4. A company's path answers only for a company that is allowed and
//!      open, to a request with a good token (401 and a challenge
//!      otherwise, to `initialize` as to anything), over plain JSON with
//!      no session.
//!   5. A request body is read to at most MAX_FORM bytes on the sign-in
//!      routes; a connection waits MAX_HEADER_WAIT for a request's
//!      headers and lives MAX_LIFE.
//! SCENARIOS:
//!   1. The metadata is the same whatever Host a caller claims, and a
//!      claim of another Host is refused.
//!   2. `OPTIONS` on a sign-in route answers a browser's preflight.
//! ```

use super::{Authorize, Denied, Door, OauthError, proxy};
use crate::admin::RemoteInfo;
use crate::local::{Books, REFUSED_HEADER};
use axum::extract::{Extension, Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// The largest body of a sign-in route.
const MAX_FORM: usize = 64 * 1024;
/// How long a connection has to send a request's headers.
const MAX_HEADER_WAIT: Duration = Duration::from_secs(15);
/// The longest a connection lives.
const MAX_LIFE: Duration = Duration::from_secs(30 * 60);

/// How a request reached the books' door.
#[derive(Clone, Copy)]
pub(in crate::local) enum Via {
    /// The local socket (same-user): a static token.
    Local,
    /// The remote door: a token this door made, from this caller.
    Remote(SocketAddr),
}

/// The caller of a connection, from its PROXY header.
#[derive(Clone, Copy)]
struct Peer(SocketAddr);

/// Turn the door on at `public_url` (the one address, `https://…`), on
/// 127.0.0.1 at `port` (any free one when None). Idempotent for the same
/// address; another address restarts it.
pub(in crate::local) fn start_door(
    books: &Arc<Books>,
    public_url: &str,
    port: Option<u16>,
) -> Result<RemoteInfo, String> {
    let remote = &books.remote;
    let url = reqwest::Url::parse(public_url)
        .map_err(|_| "the public address isn't an address".to_string())?;
    let local_host = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    let host = url
        .host_str()
        .map(str::to_ascii_lowercase)
        .filter(|_| {
            (url.scheme() == "https" || (url.scheme() == "http" && local_host))
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none()
                && matches!(url.path(), "" | "/")
        })
        .ok_or("the public address must be https://name, with no path")?;
    let authority = match url.port() {
        Some(p) => format!("{host}:{p}"),
        None => host,
    };
    let issuer = format!("{}://{authority}", url.scheme());
    if let Some(on) = remote.door() {
        if on.issuer == issuer && port.is_none_or(|p| p == on.listening.port()) {
            return Ok(remote.info());
        }
        stop_door(books);
    }
    remote.load(books)?;
    let listener = std::net::TcpListener::bind(("127.0.0.1", port.unwrap_or(0)))
        .map_err(|e| format!("the door can't listen on that port ({})", e.kind()))?;
    listener
        .set_nonblocking(true)
        .map_err(|e| format!("the door can't listen ({})", e.kind()))?;
    let listening = listener
        .local_addr()
        .map_err(|e| format!("the door can't listen ({})", e.kind()))?;
    let handle = tokio::runtime::Handle::try_current()
        .map_err(|_| "the server isn't running".to_string())?;
    let _entered = handle.enter();
    let listener = tokio::net::TcpListener::from_std(listener)
        .map_err(|e| format!("the door can't listen ({})", e.kind()))?;
    let stop = books.sessions.child_token();
    let door = Arc::new(Door {
        issuer: issuer.clone(),
        host: host_header_form(&issuer),
        listening,
        stop: stop.clone(),
    });
    remote.set_door(Some(door));
    let app = remote_router(books.clone());
    let accept_stop = stop.clone();
    handle.spawn(proxy::accept_loop(listener, stop, move |io, caller| {
        let app = app.clone().layer(Extension(Peer(caller)));
        let stop = accept_stop.clone();
        tokio::spawn(serve_connection(io, app, stop));
    }));
    Ok(remote.info())
}

/// Turn the door off: no new connection, and the ones open end.
pub(in crate::local) fn stop_door(books: &Arc<Books>) {
    if let Some(door) = books.remote.door() {
        door.stop.cancel();
    }
    books.remote.set_door(None);
}

fn host_header_form(issuer: &str) -> String {
    let rest = issuer.split_once("://").map_or(issuer, |(_, r)| r);
    let https = issuer.starts_with("https://");
    let rest = if https {
        rest.strip_suffix(":443").unwrap_or(rest)
    } else {
        rest.strip_suffix(":80").unwrap_or(rest)
    };
    rest.to_ascii_lowercase()
}

async fn serve_connection(io: proxy::Proxied, app: axum::Router, stop: CancellationToken) {
    use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
    use hyper_util::server::conn::auto::Builder;
    use hyper_util::service::TowerToHyperService;
    let mut builder = Builder::new(TokioExecutor::new());
    builder
        .http1()
        .timer(TokioTimer::new())
        .header_read_timeout(MAX_HEADER_WAIT);
    let conn = builder.serve_connection(TokioIo::new(io), TowerToHyperService::new(app));
    tokio::pin!(conn);
    let life = tokio::time::sleep(MAX_LIFE);
    tokio::pin!(life);
    let mut ending = false;
    loop {
        tokio::select! {
            _ = conn.as_mut() => break,
            () = stop.cancelled(), if !ending => {
                ending = true;
                conn.as_mut().graceful_shutdown();
            }
            () = &mut life, if !ending => {
                ending = true;
                conn.as_mut().graceful_shutdown();
            }
        }
    }
}

pub(in crate::local) fn remote_router(books: Arc<Books>) -> axum::Router {
    axum::Router::new()
        .route("/b/{book}/mcp", axum::routing::any(remote_mcp))
        .route("/authorize", axum::routing::get(authorize))
        .route("/authorize/wait/{wait}", axum::routing::get(wait))
        .route("/token", axum::routing::post(token).options(preflight))
        .route(
            "/register",
            axum::routing::post(register).options(preflight),
        )
        .fallback(fallback)
        .with_state(books)
}

fn no_store(mut response: Response) -> Response {
    let h = response.headers_mut();
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}

fn cors(mut response: Response) -> Response {
    let h = response.headers_mut();
    h.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    h.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, POST, OPTIONS"),
    );
    h.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("authorization, content-type, mcp-protocol-version"),
    );
    response
}

fn html(status: u16, page: String) -> Response {
    let mut r = (
        StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_REQUEST),
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        page,
    )
        .into_response();
    let h = r.headers_mut();
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'none'; style-src 'unsafe-inline'; frame-ancestors 'none'",
        ),
    );
    h.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    h.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    no_store(r)
}

fn json(status: u16, value: &serde_json::Value) -> Response {
    no_store(
        (
            StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_REQUEST),
            [(header::CONTENT_TYPE, "application/json")],
            value.to_string(),
        )
            .into_response(),
    )
}

fn oauth_error(e: OauthError) -> Response {
    let mut r = json(
        e.status,
        &serde_json::json!({"error": e.error, "error_description": e.description}),
    );
    if e.status == 401 && e.error == "invalid_client" {
        r.headers_mut().insert(
            header::WWW_AUTHENTICATE,
            HeaderValue::from_static("Basic realm=\"wanderbooks\", charset=\"UTF-8\""),
        );
    }
    cors(r)
}

fn redirect(to: &str) -> Response {
    let mut r = no_store(StatusCode::FOUND.into_response());
    if let Ok(v) = HeaderValue::from_str(to) {
        r.headers_mut().insert(header::LOCATION, v);
    }
    r.headers_mut().insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    r
}

fn answer(a: Authorize) -> Response {
    match a {
        Authorize::Page(status, page) => html(status, page),
        Authorize::Redirect(to) => redirect(&to),
    }
}

/// The door is on, and this request is for its one public host
/// (INVARIANT 1).
fn front(books: &Books, headers: &HeaderMap) -> Result<Arc<Door>, Box<Response>> {
    let Some(door) = books.remote.door() else {
        return Err(Box::new(no_store(
            (StatusCode::SERVICE_UNAVAILABLE, "web assistants are off").into_response(),
        )));
    };
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .map(str::to_ascii_lowercase);
    if host.as_deref() != Some(door.host.as_str()) {
        return Err(Box::new(no_store(
            (StatusCode::MISDIRECTED_REQUEST, "not this address").into_response(),
        )));
    }
    Ok(door)
}

async fn preflight(State(books): State<Arc<Books>>, headers: HeaderMap) -> Response {
    if let Err(r) = front(&books, &headers) {
        return *r;
    }
    cors(StatusCode::NO_CONTENT.into_response())
}

fn parse_pairs(text: &str) -> BTreeMap<String, String> {
    reqwest::Url::parse(&format!("http://x/?{text}"))
        .map(|u| {
            u.query_pairs()
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect()
        })
        .unwrap_or_default()
}

async fn register(
    State(books): State<Arc<Books>>,
    Extension(peer): Extension<Peer>,
    headers: HeaderMap,
    body: axum::body::Body,
) -> Response {
    if let Err(r) = front(&books, &headers) {
        return *r;
    }
    let Ok(bytes) = axum::body::to_bytes(body, MAX_FORM).await else {
        return oauth_error(OauthError {
            status: 413,
            error: "invalid_request",
            description: "the request is too large".into(),
        });
    };
    let Ok(meta) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return oauth_error(OauthError {
            status: 400,
            error: "invalid_client_metadata",
            description: "the registration must be JSON".into(),
        });
    };
    match books
        .remote
        .register(&books, &meta, &peer.0.ip().to_string())
        .await
    {
        Ok(out) => cors(json(201, &out)),
        Err(e) => oauth_error(e),
    }
}

async fn authorize(
    State(books): State<Arc<Books>>,
    Extension(peer): Extension<Peer>,
    headers: HeaderMap,
    Query(q): Query<BTreeMap<String, String>>,
) -> Response {
    let door = match front(&books, &headers) {
        Ok(d) => d,
        Err(r) => return *r,
    };
    answer(
        books
            .remote
            .authorize(&books, &door, &q, &peer.0.ip().to_string())
            .await,
    )
}

async fn wait(
    State(books): State<Arc<Books>>,
    headers: HeaderMap,
    Path(wait): Path<String>,
) -> Response {
    if let Err(r) = front(&books, &headers) {
        return *r;
    }
    answer(books.remote.wait(&wait))
}

async fn token(
    State(books): State<Arc<Books>>,
    Extension(peer): Extension<Peer>,
    headers: HeaderMap,
    body: axum::body::Body,
) -> Response {
    let door = match front(&books, &headers) {
        Ok(d) => d,
        Err(r) => return *r,
    };
    let bad = |d: &str| {
        oauth_error(OauthError {
            status: 400,
            error: "invalid_request",
            description: d.into(),
        })
    };
    let Ok(bytes) = axum::body::to_bytes(body, MAX_FORM).await else {
        return bad("the request is too large");
    };
    let form = if headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|t| t.contains("json"))
    {
        match serde_json::from_slice::<BTreeMap<String, serde_json::Value>>(&bytes) {
            Ok(m) => m
                .into_iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k, s.to_string())))
                .collect(),
            Err(_) => return bad("the request isn't understood"),
        }
    } else {
        match std::str::from_utf8(&bytes) {
            Ok(t) => parse_pairs(t),
            Err(_) => return bad("the request isn't understood"),
        }
    };
    let basic = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split_once(' '))
        .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("basic"))
        .and_then(|(_, b64)| {
            use base64::Engine as _;
            base64::engine::general_purpose::STANDARD
                .decode(b64.trim())
                .ok()
        })
        .and_then(|raw| String::from_utf8(raw).ok())
        .and_then(|text| {
            let (id, secret) = text.split_once(':')?;
            let decode = |s: &str| {
                parse_pairs(&format!("v={s}"))
                    .remove("v")
                    .unwrap_or_default()
            };
            Some((decode(id), decode(secret)))
        });
    match books
        .remote
        .token(&books, &door, &form, basic, &peer.0.ip().to_string())
        .await
    {
        Ok(out) => cors(json(200, &out)),
        Err(e) => oauth_error(e),
    }
}

/// The protected-resource document of a company's address (RFC 9728).
fn resource_document(books: &Books, door: &Door, book: &str) -> Option<serde_json::Value> {
    if !books.remote.is_allowed(book) || !crate::local::book_is_open(books, book) {
        return None;
    }
    Some(serde_json::json!({
        "resource": format!("{}/b/{book}/mcp", door.issuer),
        "authorization_servers": [door.issuer],
        "scopes_supported": ["books:read", "books:write"],
        "bearer_methods_supported": ["header"],
        "resource_name": crate::local::company_name(books, book),
    }))
}

/// The authorization server's document (RFC 8414), every address from the
/// one public address (INVARIANT 2).
fn server_document(door: &Door) -> serde_json::Value {
    let i = &door.issuer;
    serde_json::json!({
        "issuer": i,
        "authorization_endpoint": format!("{i}/authorize"),
        "token_endpoint": format!("{i}/token"),
        "registration_endpoint": format!("{i}/register"),
        "scopes_supported": ["books:read", "books:write"],
        "response_types_supported": ["code"],
        "response_modes_supported": ["query"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "token_endpoint_auth_methods_supported": ["none", "client_secret_post", "client_secret_basic"],
        "code_challenge_methods_supported": ["S256"],
        "client_id_metadata_document_supported": true,
        "authorization_response_iss_parameter_supported": true,
    })
}

const PRM: &str = "/.well-known/oauth-protected-resource";

async fn fallback(
    State(books): State<Arc<Books>>,
    method: Method,
    uri: axum::http::Uri,
    headers: HeaderMap,
) -> Response {
    let door = match front(&books, &headers) {
        Ok(d) => d,
        Err(r) => return *r,
    };
    let not_found = || {
        cors(no_store(
            (StatusCode::NOT_FOUND, "not found").into_response(),
        ))
    };
    let path = uri.path();
    if !path.contains("/.well-known/") {
        return not_found();
    }
    if method == Method::OPTIONS {
        return cors(StatusCode::NO_CONTENT.into_response());
    }
    if method != Method::GET && method != Method::HEAD {
        return cors(no_store(
            (StatusCode::METHOD_NOT_ALLOWED, "GET only").into_response(),
        ));
    }
    if let Some(at) = path.find(PRM) {
        let before = &path[..at];
        let after = &path[at + PRM.len()..];
        let resource = if after.is_empty() { before } else { after };
        let resource = resource.trim_end_matches('/');
        let book = match resource
            .strip_prefix("/b/")
            .and_then(|r| r.strip_suffix("/mcp"))
        {
            Some(b) => Some(b.to_string()),
            None if resource.is_empty() => {
                // The root: the company, if only one is on offer.
                books.remote.only_open_book(&books)
            }
            None => None,
        };
        return match book.and_then(|b| resource_document(&books, &door, &b)) {
            Some(doc) => cors(json(200, &doc)),
            None => not_found(),
        };
    }
    if path.contains("/.well-known/oauth-authorization-server")
        || path.contains("/.well-known/openid-configuration")
    {
        return cors(json(200, &server_document(&door)));
    }
    not_found()
}

/// A company's MCP path on the remote door.
async fn remote_mcp(
    State(books): State<Arc<Books>>,
    Extension(peer): Extension<Peer>,
    Path(book): Path<String>,
    request: axum::extract::Request,
) -> Response {
    let door = match front(&books, request.headers()) {
        Ok(d) => d,
        Err(r) => return *r,
    };
    if let Some(origin) = request.headers().get(header::ORIGIN)
        && origin.to_str().ok() != Some(door.issuer.as_str())
    {
        return no_store((StatusCode::FORBIDDEN, "origin not allowed").into_response());
    }
    // A company not on offer is not there at all.
    if !books.remote.is_allowed(&book) {
        return no_store(
            (
                StatusCode::NOT_FOUND,
                [(REFUSED_HEADER, "no-such-books")],
                "there are no books here",
            )
                .into_response(),
        );
    }
    crate::local::mcp_via(books, book, request, Via::Remote(peer.0)).await
}

/// The 401 of a request with no good token (INVARIANT 3 of mod.rs): the
/// challenge names the company's protected-resource metadata, so a client
/// that was never told anything can find where to sign in.
pub(in crate::local) fn unauthorized(door: &Door, book: &str, denied: &Denied) -> Response {
    let mut challenge = format!(
        "Bearer resource_metadata=\"{}/.well-known/oauth-protected-resource/b/{}/mcp\", scope=\"books:read books:write\"",
        door.issuer,
        book.chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect::<String>()
    );
    if denied.invalid_token {
        challenge.push_str(", error=\"invalid_token\"");
    }
    let mut r = no_store(
        (
            StatusCode::UNAUTHORIZED,
            [(REFUSED_HEADER, denied.code)],
            serde_json::json!({"error": "unauthorized", "error_description": denied.why})
                .to_string(),
        )
            .into_response(),
    );
    if let Ok(v) = HeaderValue::from_str(&challenge) {
        r.headers_mut().insert(header::WWW_AUTHENTICATE, v);
    }
    r.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    r
}
