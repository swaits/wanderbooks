//! The books server's HTTP door, in-process, for the fuzz targets
//! (`fuzz_http_mcp_request`, `fuzz_upload_put`; P12). It is the very router
//! `serve` mounts (`router`), over a real book in a folder of its own, fed
//! requests by `tower::oneshot`: no socket, no port. Behind the `fuzzing`
//! feature, which only the fuzz workspace turns on.
//!
//! Oracle: whatever the bytes, the door answers (no panic, no hang), and
//! never lets in a request without a token live for the book (INVARIANT 8)
//! or bytes that are not the ones declared (INVARIANT 9).

use super::*;
use tower::ServiceExt;

/// One book `acme`, two tokens (`bearer`, `other`), and the router.
pub struct FuzzDoor {
    rt: tokio::runtime::Runtime,
    books: Arc<Books>,
    app: axum::Router,
    bearer: String,
    token_id: String,
    other: String,
    _home: tempfile::TempDir,
}

impl FuzzDoor {
    /// A fresh book in a fresh folder.
    pub fn new() -> Self {
        let home = tempfile::tempdir().expect("a folder");
        let (root, _) =
            Root::setup(home.path(), Some(&home.path().join("root")), None).expect("a data root");
        let books = Arc::new(Books {
            root,
            held: RwLock::new(BTreeMap::new()),
            sessions: CancellationToken::new(),
            closing: CancellationToken::new(),
            in_flight: AtomicUsize::new(0),
            finished: tokio::sync::Notify::new(),
            last_arrival: std::sync::atomic::AtomicU64::new(0),
            started: Instant::now(),
            principals: RwLock::new(Err(String::new())),
            session_tokens: Mutex::new(BTreeMap::new()),
            backup_quiet: BACKUP_QUIET,
            drill_first: DRILL_FIRST,
            restoring: Mutex::new(()),
            isolation: Isolation::SameUser,
        });
        let loaded = books.load_principals();
        *books.principals.write().unwrap() = loaded;
        match books.admin(Request::Create {
            id: "acme".into(),
            currency: "USD".into(),
        }) {
            Response::Created { problem: None, .. } => {}
            other => panic!("{other:?}"),
        }
        let token = |name: &str| match books.admin(Request::TokenCreate {
            book: "acme".into(),
            name: name.into(),
            scopes: ["read", "write", "post", "admin"]
                .map(String::from)
                .to_vec(),
            replace: true,
        }) {
            Response::TokenCreated { token, text } => (token.id, text.0),
            other => panic!("{other:?}"),
        };
        let (token_id, bearer) = token("fuzz");
        let (_, other) = token("fuzz-other");
        let app = router(books.clone());
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("a runtime");
        FuzzDoor {
            rt,
            books,
            app,
            bearer,
            token_id,
            other,
            _home: home,
        }
    }

    /// The text of the token that is live for the book.
    pub fn bearer(&self) -> &str {
        &self.bearer
    }

    /// The text of a second live token (the one an upload is not its).
    pub fn other_bearer(&self) -> &str {
        &self.other
    }

    /// An upload granted to the live token: its id, or why not.
    pub fn grant(&self, sha256: &str, size: u64) -> Result<String, String> {
        let held = self.books.held();
        let Some(Book::Open { uploads, .. }) = held.get("acme") else {
            return Err("acme isn't open".into());
        };
        uploads
            .grant(
                &self.token_id,
                sha256,
                size,
                "application/pdf",
                "fuzz.pdf",
                "document",
                chrono::Utc::now().timestamp(),
            )
            .map(|g| g.id)
            .map_err(|e| e.to_string())
    }

    /// Whether the door would take one of these headers as a bearer token
    /// live for the book: the same check every request passes, so a target
    /// never has to model the rule a second time.
    pub fn carries_a_token(&self, headers: &[(Vec<u8>, Vec<u8>)]) -> bool {
        let mut map = axum::http::HeaderMap::new();
        for (name, value) in headers {
            let (Ok(name), Ok(value)) = (
                axum::http::HeaderName::from_bytes(name),
                axum::http::HeaderValue::from_bytes(value),
            ) else {
                return false;
            };
            map.append(name, value);
        }
        self.books.bearer(&map).is_ok()
    }

    /// Send a request as the router receives it; the status and the body
    /// of the answer, or `None` when the pieces do not make an HTTP request
    /// at all (a method or header the HTTP types refuse never reaches the
    /// door in life either).
    pub fn send(
        &self,
        method: &[u8],
        path: &str,
        headers: &[(Vec<u8>, Vec<u8>)],
        body: Vec<u8>,
    ) -> Option<(u16, Vec<u8>)> {
        let mut request = axum::http::Request::builder()
            .method(axum::http::Method::from_bytes(method).ok()?)
            .uri(
                format!("http://localhost{path}")
                    .parse::<axum::http::Uri>()
                    .ok()?,
            );
        for (name, value) in headers {
            request = request.header(
                axum::http::HeaderName::from_bytes(name).ok()?,
                axum::http::HeaderValue::from_bytes(value).ok()?,
            );
        }
        let request = request.body(axum::body::Body::from(body)).ok()?;
        let app = self.app.clone();
        Some(self.rt.block_on(async move {
            let response = app.oneshot(request).await.unwrap_or_else(|e| match e {});
            let status = response.status().as_u16();
            // An event stream never ends on its own: read what comes in a
            // moment, as a client that hangs up would.
            let bytes = tokio::time::timeout(
                Duration::from_millis(200),
                axum::body::to_bytes(response.into_body(), 1 << 20),
            )
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or_default();
            (status, bytes.to_vec())
        }))
    }

    /// Sessions the door has bound (they only grow, so the target starts
    /// a fresh door now and then).
    pub fn sessions(&self) -> usize {
        self.books.session_tokens.lock().unwrap().len()
    }
}

impl Default for FuzzDoor {
    fn default() -> Self {
        Self::new()
    }
}
