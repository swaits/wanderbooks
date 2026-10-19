//! `wanderbooks server`: every book in the data root, served on local
//! sockets (plan 3.1, 3.9). The host starts it; nothing else should.
//!
//! ```text
//! GOAL: one process, the books' sole writer, serves every book in the
//!       data root to this account alone, over sockets in the root's
//!       private `run/` folder, and lets go of every book cleanly.
//! SOURCE: plan/served.md 3.1 (the server's sockets; the admin socket
//!         creates and adopts books, mints tokens, and can't read books),
//!         3.3 (bearer tokens, scopes), 3.4 (data
//!         layout, run/books.lock), 3.9 (local channels, peer checks,
//!         closing: "no new requests, the one in progress finishes, every
//!         writer lock is released"), D-21 and D-23 (no service: the host
//!         starts and stops this process).
//! INVARIANTS:
//!   1. One server per root: it holds `run/server.lock` from before it
//!      opens anything until it exits; a second server waits for the lock
//!      (an old server still finishing after its host died), touching
//!      nothing, and exits if told to close first. The sockets are
//!      replaced only under that lock.
//!   2. Only this account: a connection to either socket whose peer is
//!      another user ID is dropped before a byte is read.
//!   3. Every book is opened and verified (rebuilding a drifted database)
//!      before the server says it is ready; one that won't open is
//!      reported with why, on its route and in the list, and the others
//!      are served.
//!   4. Each book has one store, shared by every session: one writer.
//!   5. The ready line (stdout, once) comes only after both sockets are
//!      bound and every book has been tried.
//!   6. Closing (stdin reaches its end, or SIGTERM, SIGINT or SIGHUP): new
//!      requests are refused, every request in progress finishes (admin
//!      ones included), then the sessions end, the sockets are removed,
//!      and the process exits 0, releasing every lock.
//!   7. Nothing this process sends a client names the data root.
//!   8. An MCP request reaches a book only with a bearer token that is
//!      live for that book (auth/principals.jsonl; refused 401 without
//!      one, 403 with another book's), and a session only with the token
//!      that opened it (403). A request refused at the door is recorded
//!      in that book's access log; the token rides with the request to
//!      the book's tools, which check its scopes (access.rs). A session is
//!      bound to its token when opened, and forgotten when it ends, is
//!      found gone, or its token is revoked or replaced; past MAX_SESSIONS
//!      bindings a new session is refused (an initialize), never a binding
//!      dropped. Tokens are made, listed and revoked only through
//!      the admin socket.
//! ```

use crate::admin::{self, Request, Response};
use crate::local_files::ImportRoots;
use crate::policy::ReceiptPolicy;
use crate::server::{BooksServer, Layout, SharedStore};
use crate::settings::Live;
use ledger_store::Store;
use ledger_store::access::{Access, AccessLog, Outcome, Principal};
use ledger_store::auth::{Principals, Scope, Token};
use ledger_store::root::{BookId, Root, RootError};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use std::collections::BTreeMap;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

/// The server's MCP socket, in the root's `run/`.
pub const MCP_SOCKET: &str = "mcp.sock";
/// The server's admin socket, in the root's `run/`.
pub const ADMIN_SOCKET: &str = "admin.sock";
/// Held by the one server for a root (INVARIANT 1).
pub const SERVER_LOCK: &str = "server.lock";
/// What the server prints on stdout when it is ready (INVARIANT 5).
pub const READY: &str = "ready";

/// How large a request may be. Receipts still ride inside tool calls as
/// base64 until uploads have their own route (P2 slice 5, which brings
/// this down to 1 MiB).
pub const MAX_BODY: usize = 64 * 1024 * 1024;

/// How long the admin socket waits for a request to arrive.
const ADMIN_READ_TIMEOUT: Duration = Duration::from_secs(30);

use ledger_store::root::THE_ROOT;

/// The header on a request refused before it could run (books that
/// didn't open, books that don't exist): a client knows it didn't run.
pub const REFUSED_HEADER: &str = "x-wanderbooks-refused";

type Service = StreamableHttpService<BooksServer, LocalSessionManager>;

/// A book the server holds.
#[derive(Clone)]
enum Book {
    /// Open and served.
    Open {
        /// Its MCP service.
        service: Service,
        /// Its store, closed when the server closes (INVARIANT 6).
        store: Arc<SharedStore>,
        /// Its access log, where requests refused at the door are
        /// recorded too (access.rs).
        access: Arc<AccessLog>,
    },
    /// Not opened, with why (INVARIANT 3).
    Failed(String),
}

impl Book {
    /// Why it isn't open, if it isn't.
    fn problem(&self) -> Option<String> {
        match self {
            Book::Open { .. } => None,
            Book::Failed(why) => Some(why.clone()),
        }
    }
}

/// Every book the server holds, and what serving them needs.
struct Books {
    root: Root,
    import_roots: ImportRoots,
    held: RwLock<BTreeMap<String, Book>>,
    /// Ends every MCP session (INVARIANT 6, last).
    sessions: CancellationToken,
    /// Set when the server starts closing: new requests are refused.
    closing: CancellationToken,
    /// MCP requests in progress (not counting event streams).
    in_flight: AtomicUsize,
    /// When the last MCP request arrived (milliseconds since the server
    /// started): closing lingers until none has for a while.
    last_arrival: std::sync::atomic::AtomicU64,
    /// When the server started, for `last_arrival`.
    started: std::time::Instant,
    /// Woken when one finishes.
    finished: tokio::sync::Notify,
    /// The agents' tokens (auth/principals.jsonl), or why they can't be
    /// read: then every MCP request is refused (INVARIANT 8).
    principals: RwLock<Result<Principals, String>>,
    /// Which token opened each MCP session: a session is used only with
    /// the token that opened it (INVARIANT 8).
    session_tokens: Mutex<BTreeMap<String, String>>,
}

impl Books {
    /// Take the root's path out of a message (INVARIANT 7).
    fn hide(&self, text: impl Into<String>) -> String {
        let mut text = text.into();
        let path = self.root.path();
        let real = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        for p in [path, real.as_path()] {
            text = text.replace(&p.display().to_string(), THE_ROOT);
        }
        text
    }

    /// Change or read the principals; a log that failed to write is read
    /// again before anything else is done with it (chain.rs INVARIANT 2).
    fn with_principals(
        &self,
        f: impl FnOnce(&mut Principals) -> Result<Response, ledger_store::auth::AuthError>,
    ) -> Response {
        let mut guard = self.principals.write().unwrap_or_else(|e| e.into_inner());
        let p = match guard.as_mut() {
            Ok(p) => p,
            Err(why) => return Response::Refused(why.clone()),
        };
        match f(p) {
            Ok(r) => r,
            Err(e) => {
                if matches!(e, ledger_store::auth::AuthError::Log(_)) {
                    *guard = self.load_principals();
                }
                Response::Refused(self.hide(e.to_string()))
            }
        }
    }

    /// Forget the sessions a token opened (it was revoked or replaced:
    /// they can't be used again, and won't be ended by it).
    fn forget_sessions_of(&self, token_id: &str) {
        self.session_tokens
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|_, owner| owner != token_id);
    }

    /// Read the principals from the root's `auth/`.
    fn load_principals(&self) -> Result<Principals, String> {
        let dir = self.root.auth_dir().map_err(|e| e.to_string())?;
        Principals::open(&dir).map_err(|e| self.hide(e.to_string()))
    }

    /// The live token whose text a request carries as its bearer, or why
    /// it is refused (INVARIANT 8): the code for the access log and the
    /// words for the client.
    fn bearer(&self, headers: &axum::http::HeaderMap) -> Result<Token, (&'static str, String)> {
        // The scheme's name in any case (RFC 9110 11.1).
        let presented = headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split_once(' '))
            .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
            .map(|(_, token)| token.trim());
        let Some(presented) = presented else {
            return Err((
                crate::access::UNAUTHORIZED,
                "these books need a token: the human makes one with `wanderbooks token create`"
                    .into(),
            ));
        };
        let guard = self.principals.read().unwrap_or_else(|e| e.into_inner());
        match guard.as_ref() {
            Err(why) => Err((crate::access::UNAUTHORIZED, why.clone())),
            Ok(p) => p.check(presented).cloned().ok_or((
                crate::access::UNAUTHORIZED,
                "that token isn't one these books know, or it was revoked".into(),
            )),
        }
    }

    /// Open the book `id` from disk: served, or failed with why.
    fn open(&self, id: &BookId) -> Book {
        let opened = match Store::open(&self.root.book_dir(id)) {
            Ok(store) => self.serve(store),
            Err(ledger_store::StoreError::Locked(_)) => {
                Err("another program has these books open".to_string())
            }
            Err(e) => Err(self.hide(e.to_string())),
        };
        opened.unwrap_or_else(|why| {
            eprintln!("wanderbooks server: the books {id} were not opened: {why}");
            Book::Failed(why)
        })
    }

    /// Verify an open store and build its service (INVARIANTS 3, 4).
    fn serve(&self, mut store: Store) -> Result<Book, String> {
        let report = store
            .verify_and_heal()
            .map_err(|e| format!("they don't verify: {}", self.hide(e.to_string())))?;
        if let Some(why) = &report.healed {
            eprintln!(
                "wanderbooks server: rebuilt a database from its journal ({})",
                self.hide(why.clone())
            );
        }
        let mut live = Live::own();
        live.sync(store.ledger().settings(), store.base_currency().code());
        let layout = Layout::Root(self.root.path().to_path_buf());
        let roots = crate::server::guard_import_roots(
            &store,
            &live.current(),
            &layout,
            self.import_roots.clone(),
        )
        .map_err(|e| self.hide(e))?;
        let access = Arc::new(
            AccessLog::open(store.data_dir())
                .map_err(|e| format!("its access log can't be used: {e}"))?,
        );
        let live = Arc::new(Mutex::new(live));
        let store = SharedStore::new(store);
        let log = access.clone();
        let shared = store.clone();
        let config = StreamableHttpServerConfig::default()
            .with_json_response(true)
            .with_legacy_session_mode(true)
            .with_max_request_body_bytes(MAX_BODY)
            .enforce_origin_validation()
            .with_cancellation_token(self.sessions.child_token());
        let service = StreamableHttpService::new(
            move || {
                Ok(
                    BooksServer::new_shared(store.clone(), ReceiptPolicy::default())
                        .with_live_settings(live.clone())
                        .with_import_roots(roots.clone())
                        .with_layout(layout.clone())
                        .with_access(log.clone()),
                )
            },
            Arc::new(sessions()),
            config,
        );
        Ok(Book::Open {
            service,
            store: shared,
            access,
        })
    }

    /// Every open book's store.
    fn stores(&self) -> Vec<Arc<SharedStore>> {
        self.held()
            .values()
            .filter_map(|b| match b {
                Book::Open { store, .. } => Some(store.clone()),
                Book::Failed(_) => None,
            })
            .collect()
    }

    fn held(&self) -> std::sync::RwLockReadGuard<'_, BTreeMap<String, Book>> {
        self.held.read().unwrap_or_else(|e| e.into_inner())
    }

    fn hold(&self, id: &BookId, book: Book) {
        self.held
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id.to_string(), book);
    }

    /// Answer an admin request (blocking: it runs on its own thread).
    fn admin(&self, request: Request) -> Response {
        let refused = |e: RootError| Response::Refused(self.hide(e.to_string()));
        match request {
            Request::List => Response::Books(
                self.held()
                    .iter()
                    .map(|(id, book)| admin::BookStatus {
                        id: id.clone(),
                        open: matches!(book, Book::Open { .. }),
                        problem: match book {
                            Book::Open { .. } => None,
                            Book::Failed(why) => Some(why.clone()),
                        },
                    })
                    .collect(),
            ),
            Request::TokenCreate {
                book,
                name,
                scopes,
                replace,
            } => {
                let book = match BookId::parse(&book) {
                    Ok(id) => id,
                    Err(e) => return refused(e),
                };
                if !self.held().contains_key(book.as_str()) {
                    return Response::Refused(format!(
                        "there are no books called {book}: `wanderbooks books list` lists them"
                    ));
                }
                let mut wanted = std::collections::BTreeSet::new();
                for s in &scopes {
                    match Scope::parse(s) {
                        Some(scope) => {
                            wanted.insert(scope);
                        }
                        None => {
                            return Response::Refused(format!(
                                "{:?} isn't a scope: use books:read, books:write, books:post or books:admin",
                                s.chars().take(64).collect::<String>()
                            ));
                        }
                    }
                }
                let mut replaced: Option<String> = None;
                let answer = self.with_principals(|p| {
                    replaced = p
                        .tokens()
                        .iter()
                        .find(|t| t.book == book.as_str() && t.name == name)
                        .map(|t| t.id.clone())
                        .filter(|_| replace);
                    p.create(&book, &name, wanted, replace)
                        .map(|(token, text)| Response::TokenCreated {
                            token: admin::TokenInfo::from(&token),
                            text: admin::TokenText(text.expose().to_string()),
                        })
                });
                if let (Response::TokenCreated { .. }, Some(old)) = (&answer, &replaced) {
                    self.forget_sessions_of(old);
                }
                answer
            }
            Request::TokenList => self.with_principals(|p| {
                Ok(Response::Tokens(
                    p.tokens().iter().map(admin::TokenInfo::from).collect(),
                ))
            }),
            Request::TokenRevoke { book, name, id } => {
                let book = match BookId::parse(&book) {
                    Ok(id) => id,
                    Err(e) => return refused(e),
                };
                let answer = self.with_principals(|p| {
                    p.revoke_if(&book, &name, id.as_deref())
                        .map(|t| Response::TokenRevoked(admin::TokenInfo::from(&t)))
                });
                if let Response::TokenRevoked(t) = &answer {
                    self.forget_sessions_of(&t.id);
                }
                answer
            }
            Request::Create { id, currency } => {
                let id = match BookId::parse(&id) {
                    Ok(id) => id,
                    Err(e) => return refused(e),
                };
                let Ok(currency) = ledger_core::Currency::new(currency.trim()) else {
                    return Response::Refused(
                        "that isn't a currency: give its three-letter ISO code, such as USD".into(),
                    );
                };
                if self.held().contains_key(id.as_str()) {
                    return refused(RootError::Exists(id.to_string()));
                }
                match self.root.create_book(&id, currency) {
                    Ok(store) => {
                        // Made: kept whether or not it opens (INVARIANT 3).
                        let book = self.serve(store).unwrap_or_else(Book::Failed);
                        let problem = book.problem();
                        self.hold(&id, book);
                        Response::Created {
                            id: id.to_string(),
                            problem,
                        }
                    }
                    Err(e) => refused(e),
                }
            }
            Request::Adopt {
                id,
                source,
                settings_toml,
                config_json,
                vault_dir,
                vault_key,
                confirmed,
                expect_head,
                skip_unknown,
            } => {
                let id = match BookId::parse(&id) {
                    Ok(id) => id,
                    Err(e) => return refused(e),
                };
                if self.held().contains_key(id.as_str()) {
                    return refused(RootError::Exists(id.to_string()));
                }
                let found = match self.root.adopt_preview(&source) {
                    Ok(found) => found,
                    Err(e) => return refused(e),
                };
                if vault_dir.is_some() != vault_key.is_some() {
                    return Response::Refused(
                        "a vault comes with its key: give both the vault folder and its key file, or neither"
                            .into(),
                    );
                }
                if confirmed && expect_head.is_none() {
                    // admin INVARIANT 4: a confirm names what it confirms.
                    return Response::Refused(
                        "an adoption is confirmed against the preview's head: preview first".into(),
                    );
                }
                let import = match (&settings_toml, found.keeps_settings) {
                    (Some(toml), false) => {
                        // The key the file names matters only with a vault
                        // to open.
                        let key = vault_dir.as_ref().and(vault_key.as_deref());
                        match settings_to_move(
                            toml,
                            config_json.as_deref(),
                            found.currency.code(),
                            vault_dir.is_some(),
                            key,
                        ) {
                            Ok(import) => Some(import),
                            Err(why) => return Response::Refused(why),
                        }
                    }
                    (None, false) => {
                        return Response::Refused(
                            "these books keep no settings of their own yet: give their settings file (--toml), and its settings are moved in as they are adopted"
                                .into(),
                        );
                    }
                    _ => None,
                };
                let vault_records = match (&vault_dir, &vault_key) {
                    (Some(dir), Some(key)) => match vault_opens(dir, key) {
                        Ok(n) => n,
                        Err(why) => return Response::Refused(why),
                    },
                    _ => 0,
                };
                if !confirmed {
                    let i = import.unwrap_or_default();
                    return Response::Preview(admin::AdoptPreview {
                        source_head: found.head,
                        keeps_settings: found.keeps_settings,
                        settings: i
                            .changes
                            .iter()
                            .map(|c| admin::SettingChange {
                                key: c.key.clone(),
                                value: c.value.clone(),
                            })
                            .collect(),
                        notes: i.notes,
                        unknown: i.unknown,
                        comments: i.comments,
                        vault_records,
                    });
                }
                // admin INVARIANT 5: nothing dropped unseen.
                if let Some(i) = &import
                    && !i.unknown.is_empty()
                    && !skip_unknown
                {
                    return Response::Refused(format!(
                        "the settings file has entries the books don't know, which wouldn't move in: {}. Take them out, or adopt with --skip-unknown to leave them behind",
                        i.unknown.join(", ")
                    ));
                }
                let options = ledger_store::root::AdoptOptions {
                    settings: match import {
                        Some(i) => {
                            let source = moved_in_at_adoption(&i.changes);
                            Some((i.changes, source))
                        }
                        // Given for books that keep their own: passed on so
                        // the report says they weren't used.
                        None if settings_toml.is_some() => {
                            Some((Vec::new(), moved_in_at_adoption(&[])))
                        }
                        None => None,
                    },
                    vault_dir,
                    vault_key,
                    expect_head,
                };
                match self.root.adopt(&source, &id, &options) {
                    Ok(r) => {
                        // Adopted: kept whether or not it opens.
                        let book = self.open(&id);
                        let problem = book.problem();
                        self.hold(&id, book);
                        Response::Adopted(admin::Adopted {
                            id: r.id.to_string(),
                            source_head: r.source_head,
                            head: r.head,
                            blob_files: r.blob_files,
                            blobs_verified: r.blobs_verified,
                            unnamed_blobs: r.unnamed_blobs,
                            skipped_files: r.skipped_files,
                            vault_files: r.vault_files,
                            key_copied: r.key_copied,
                            settings_imported: r.settings_imported,
                            settings_ignored: r.settings_ignored,
                            source_lock_created: r.source_lock_created,
                            problem,
                        })
                    }
                    Err(e) => refused(e),
                }
            }
        }
    }
}

/// The settings a settings file moves in at adoption, checked as
/// `books.import_settings` checks them: they parse, they apply, and the
/// vault key file it names (which the books don't keep) is the one given.
fn settings_to_move(
    toml: &str,
    config_json: Option<&str>,
    currency: &str,
    with_vault: bool,
    vault_key: Option<&Path>,
) -> Result<ledger_config::codec::Import, String> {
    let import = ledger_config::codec::import(
        toml,
        config_json,
        &ledger_core::settings::Settings::default(),
        currency,
    )
    .map_err(|e| format!("the settings file can't be moved in: {e}"))?;
    let mut after = ledger_core::settings::Settings::default();
    after
        .apply(
            &import.changes,
            &moved_in_at_adoption(&import.changes),
            u64::MAX,
        )
        .map_err(|e| format!("the settings file can't be moved in: {e}"))?;
    if let Some(named) = import.vault_key_file.as_ref().filter(|_| with_vault) {
        let same = vault_key.is_some_and(|given| {
            match (std::fs::canonicalize(named), std::fs::canonicalize(given)) {
                (Ok(a), Ok(b)) => a == b,
                _ => false,
            }
        });
        if !same {
            return Err(
                "the settings file names the TIN vault's key file ([vault] key_file), which the books don't keep: adopt with --vault-key naming that same file, or the vault wouldn't open"
                    .into(),
            );
        }
    }
    Ok(import)
}

/// Check that `key` opens every record in the vault folder `dir`; how
/// many there are.
fn vault_opens(dir: &Path, key: &Path) -> Result<usize, String> {
    let vault = ledger_vault::Vault::open(dir, key, false).map_err(|_| {
        "the vault key given isn't one this vault can use (64 hex digits): nothing was adopted"
            .to_string()
    })?;
    let entries = std::fs::read_dir(dir)
        .map_err(|e| format!("the vault folder given can't be read: {}", e.kind()))?;
    let mut records = 0;
    for entry in entries {
        let entry = entry.map_err(|e| format!("the vault folder can't be read: {}", e.kind()))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(stem) = name.strip_suffix(".bin") else {
            continue;
        };
        let opened = match stem.split_once('-') {
            Some(("tin", n)) => n.parse().ok().map(|n| vault.tin(n).map(|_| ())),
            Some(("w9", n)) => n.parse().ok().map(|n| vault.w9(n).map(|_| ())),
            _ => None,
        };
        match opened {
            Some(Ok(())) => records += 1,
            Some(Err(_)) => {
                return Err(
                    "the vault key given doesn't open the vault's records: give the key they were sealed with; nothing was adopted"
                        .into(),
                );
            }
            None => {}
        }
    }
    Ok(records)
}

/// How long a session may go without a message before it ends: far
/// longer than any call (rmcp's idle timer doesn't count a call in
/// progress, so a call longer than it would lose its answer while its work
/// went on). It only bounds sessions whose client vanished without ending
/// them; the bridge ends its own, even when signalled.
const SESSION_IDLE: Duration = Duration::from_secs(24 * 60 * 60);

/// The book's MCP sessions (see SESSION_IDLE).
fn sessions() -> LocalSessionManager {
    let mut sessions = LocalSessionManager::default();
    sessions.session_config.keep_alive = Some(SESSION_IDLE);
    sessions
}

/// Who and when, for settings moved in from a file at adoption: dated in
/// the company's time zone when the file sets one.
fn moved_in_at_adoption(
    changes: &[ledger_core::settings::Change],
) -> ledger_core::settings::Source {
    let zone = changes
        .iter()
        .find(|c| c.key == "company.timezone")
        .and_then(|c| c.value.as_ref())
        .and_then(|v| v.as_str())
        .map(str::to_string);
    ledger_core::settings::Source {
        decided_by: "the settings file".into(),
        decided_on: ledger_core::settings::today_in(zone.as_deref()),
        citation: None,
        note: Some(
            "moved in by the owner when the books were adopted (wanderbooks books adopt)".into(),
        ),
    }
}

/// The user ID at the other end of a Unix socket.
pub fn peer_uid(fd: &impl AsRawFd) -> std::io::Result<u32> {
    let fd = fd.as_raw_fd();
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        let mut cred = libc::ucred {
            pid: 0,
            uid: 0,
            gid: 0,
        };
        let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        // SAFETY: `cred` and `len` are valid for writes of the sizes given.
        let rc = unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                (&mut cred as *mut libc::ucred).cast(),
                &mut len,
            )
        };
        if rc != 0 {
            return Err(std::io::Error::last_os_error());
        }
        if len as usize != std::mem::size_of::<libc::ucred>() {
            return Err(std::io::Error::other(
                "the peer's credentials came back short",
            ));
        }
        Ok(cred.uid)
    }
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    {
        let mut uid: libc::uid_t = 0;
        let mut gid: libc::gid_t = 0;
        // SAFETY: `uid` and `gid` are valid for writes.
        if unsafe { libc::getpeereid(fd, &mut uid, &mut gid) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(uid)
    }
}

/// The process at the other end of a Unix socket, where the system says
/// (Linux); `None` elsewhere or if it can't be read.
pub fn peer_pid(fd: &impl AsRawFd) -> Option<u32> {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        let mut cred = libc::ucred {
            pid: 0,
            uid: 0,
            gid: 0,
        };
        let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        // SAFETY: `cred` and `len` are valid for writes of the sizes given.
        let rc = unsafe {
            libc::getsockopt(
                fd.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                (&mut cred as *mut libc::ucred).cast(),
                &mut len,
            )
        };
        (rc == 0 && len as usize == std::mem::size_of::<libc::ucred>() && cred.pid > 0)
            .then_some(cred.pid as u32)
    }
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    {
        let _ = fd;
        None
    }
}

/// This process's user ID.
fn own_uid() -> u32 {
    // SAFETY: geteuid has no preconditions and can't fail.
    unsafe { libc::geteuid() }
}

/// Whether the other end of a socket is this account (INVARIANT 2).
pub fn peer_is_us(fd: &impl AsRawFd) -> bool {
    peer_uid(fd).is_ok_and(|uid| uid == own_uid())
}

/// The client's check of the server (plan 3.9: both ends check).
pub(crate) fn check_peer_std(stream: &std::os::unix::net::UnixStream) -> Result<(), String> {
    if peer_is_us(stream) {
        Ok(())
    } else {
        Err("the books server's socket belongs to another account: not used".into())
    }
}

/// A Unix listener that hands on only this account's connections
/// (INVARIANT 2).
struct OwnerOnly(tokio::net::UnixListener);

impl OwnerOnly {
    async fn next(&mut self) -> tokio::net::UnixStream {
        loop {
            match self.0.accept().await {
                Ok((stream, _)) if peer_is_us(&stream) => return stream,
                Ok((stream, _)) => {
                    eprintln!("wanderbooks server: refused a connection from another account");
                    drop(stream);
                }
                Err(e) => {
                    // Out of descriptors and the like: wait, as axum does,
                    // rather than spin. A peer that hung up is no matter.
                    if !matches!(
                        e.kind(),
                        std::io::ErrorKind::ConnectionAborted
                            | std::io::ErrorKind::ConnectionReset
                            | std::io::ErrorKind::ConnectionRefused
                    ) {
                        eprintln!("wanderbooks server: accept failed: {e}");
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                }
            }
        }
    }
}

impl axum::serve::Listener for OwnerOnly {
    type Io = tokio::net::UnixStream;
    type Addr = tokio::net::unix::SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        loop {
            let stream = self.next().await;
            if let Ok(addr) = stream.peer_addr() {
                return (stream, addr);
            }
        }
    }

    fn local_addr(&self) -> std::io::Result<Self::Addr> {
        self.0.local_addr()
    }
}

/// Counts an MCP request in progress, from its arrival until its answer's
/// body is dropped (sent whole, or its client gone): it rides in that
/// body, which a session fills later. Whether a tool is still running is
/// counted by its store (SharedStore::close), not here.
struct InFlight(Arc<Books>);

impl InFlight {
    fn new(books: &Arc<Books>) -> Self {
        books.in_flight.fetch_add(1, Ordering::SeqCst);
        Self(books.clone())
    }
}

impl Drop for InFlight {
    fn drop(&mut self) {
        self.0.in_flight.fetch_sub(1, Ordering::SeqCst);
        self.0.finished.notify_waiters();
    }
}

/// The MCP session header.
const SESSION_HEADER: &str = "mcp-session-id";

/// The most sessions open at once (each bound to its token): past it a
/// new session is refused until one ends, never a binding forgotten
/// (INVARIANT 8; review of slice 4: clearing them let a flood unbind
/// live sessions).
const MAX_SESSIONS: usize = 100_000;

/// A request refused at the door: recorded in the book's access log
/// (access.rs INVARIANT 3), then answered with `status` and `why`.
async fn refused_at_door(
    book: &Book,
    principal: Principal,
    method: &axum::http::Method,
    code: &'static str,
    status: axum::http::StatusCode,
    why: String,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    if let Book::Open { access, store, .. } = book {
        let access = access.clone();
        let record = Access {
            principal,
            client: None,
            request_id: None,
            action: "connect".into(),
            subject: method.as_str().to_string(),
            args_digest: None,
            result: Outcome::Refused(code.into()),
            head_after: None,
        };
        let written = tokio::task::spawn_blocking(move || access.record(&record)).await;
        if !matches!(written, Ok(Ok(_))) {
            store.stop();
            return (
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                [(REFUSED_HEADER, "not-recorded")],
                crate::access::NOT_RECORDED,
            )
                .into_response();
        }
    }
    let mut response = (status, [(REFUSED_HEADER, code)], why).into_response();
    if status == axum::http::StatusCode::UNAUTHORIZED {
        response.headers_mut().insert(
            axum::http::header::WWW_AUTHENTICATE,
            axum::http::HeaderValue::from_static("Bearer realm=\"wanderbooks\""),
        );
    }
    response
}

/// `/b/{book}/mcp`: the book's MCP service, for a request carrying a token
/// for that book (INVARIANT 8).
async fn mcp(
    axum::extract::State(books): axum::extract::State<Arc<Books>>,
    axum::extract::Path(book): axum::extract::Path<String>,
    mut request: axum::extract::Request,
) -> axum::response::Response {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use tower::ServiceExt;
    // Counted before the closing check, so closing never misses it.
    let counted = (request.method() != axum::http::Method::GET).then(|| InFlight::new(&books));
    books
        .last_arrival
        .store(books.started.elapsed().as_millis() as u64, Ordering::SeqCst);
    let closing = books.closing.is_cancelled();
    let found = books.held().get(&book).cloned();
    let (service, held) = match found {
        Some(Book::Failed(why)) if !closing => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                [(REFUSED_HEADER, "not-open")],
                format!("the books {book} didn't open: {why}"),
            )
                .into_response();
        }
        None if !closing => {
            return (
                StatusCode::NOT_FOUND,
                [(REFUSED_HEADER, "no-such-books")],
                format!(
                    "there are no books called {:?}: `wanderbooks books list` lists them",
                    book.chars().take(64).collect::<String>()
                ),
            )
                .into_response();
        }
        Some(b @ Book::Open { .. }) => match &b {
            Book::Open { service, .. } => (service.clone(), b.clone()),
            Book::Failed(_) => unreachable!(),
        },
        _ => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "the books are closing: this request wasn't run",
            )
                .into_response();
        }
    };
    let method = request.method().clone();
    let token = books.bearer(request.headers());
    let principal = |t: &Token| Principal {
        id: t.id.clone(),
        kind: "agent".into(),
        label: t.name.clone(),
    };
    if closing {
        let who = token
            .as_ref()
            .map_or_else(|_| Principal::unknown(), principal);
        return refused_at_door(
            &held,
            who,
            &method,
            crate::access::CLOSING,
            StatusCode::SERVICE_UNAVAILABLE,
            "the books are closing: this request wasn't run".into(),
        )
        .await;
    }
    let token = match token {
        Ok(t) => t,
        Err((code, why)) => {
            return refused_at_door(
                &held,
                Principal::unknown(),
                &method,
                code,
                StatusCode::UNAUTHORIZED,
                why,
            )
            .await;
        }
    };
    if token.book != book {
        return refused_at_door(
            &held,
            principal(&token),
            &method,
            "wrong_books",
            StatusCode::FORBIDDEN,
            format!("this token is for the books {}, not {book}", token.book),
        )
        .await;
    }
    let session = request
        .headers()
        .get(SESSION_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    // A session is bound when it is opened (below): one bound to another
    // token is refused; one never bound isn't one this server opened, and
    // is left to the MCP service, which doesn't know it either (and says
    // so, which a client takes as "open a new one").
    let (elsewhere, full) = {
        let bound = books
            .session_tokens
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let elsewhere = session
            .as_ref()
            .and_then(|sid| bound.get(sid))
            .is_some_and(|owner| *owner != token.id);
        (elsewhere, session.is_none() && bound.len() >= MAX_SESSIONS)
    };
    // The cap is on opening sessions: a request opens one only if it is an
    // initialize (a 2026-07-28 call without a session opens none).
    let full = full && {
        let (parts, body) = request.into_parts();
        let bytes = axum::body::to_bytes(body, MAX_BODY)
            .await
            .unwrap_or_default();
        let opens = serde_json::from_slice::<serde_json::Value>(&bytes)
            .ok()
            .is_some_and(|m| m.get("method").and_then(|v| v.as_str()) == Some("initialize"));
        request = axum::extract::Request::from_parts(parts, axum::body::Body::from(bytes));
        opens
    };
    if full {
        return refused_at_door(
            &held,
            principal(&token),
            &method,
            "too_many_sessions",
            StatusCode::SERVICE_UNAVAILABLE,
            "the books server has too many sessions open: end some, then try again".into(),
        )
        .await;
    }
    if elsewhere {
        return refused_at_door(
            &held,
            principal(&token),
            &method,
            "session_token",
            StatusCode::FORBIDDEN,
            "this session was opened with another token".into(),
        )
        .await;
    }
    let token_id = token.id.clone();
    request
        .extensions_mut()
        .insert(crate::access::Caller { token });
    let response = match service.oneshot(request).await {
        Ok(response) => response,
        Err(never) => match never {},
    };
    // A session the answer opened is bound to the token that opened it;
    // one ended, or found gone, is forgotten.
    {
        let mut bound = books
            .session_tokens
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if session.is_none()
            && let Some(sid) = response
                .headers()
                .get(SESSION_HEADER)
                .and_then(|v| v.to_str().ok())
        {
            bound.insert(sid.to_string(), token_id);
        }
        let ended = method == axum::http::Method::DELETE && response.status().is_success();
        let gone = response.status() == StatusCode::NOT_FOUND;
        if (ended || gone)
            && let Some(sid) = &session
        {
            bound.remove(sid);
        }
    }
    response.map(|body| {
        use http_body_util::BodyExt;
        axum::body::Body::new(body.map_frame(move |frame| {
            let _held = &counted;
            frame
        }))
    })
}

/// Anything but `/b/{book}/mcp`.
async fn not_found() -> (axum::http::StatusCode, &'static str) {
    (axum::http::StatusCode::NOT_FOUND, "not found")
}

/// One admin connection: one request, one answer (admin INVARIANT 1).
async fn serve_admin(stream: tokio::net::UnixStream, books: Arc<Books>) {
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
    let (read, mut write) = stream.into_split();
    let mut reader = tokio::io::BufReader::new(read.take(admin::MAX_LINE as u64 + 1));
    let mut line = String::new();
    // A request still arriving when closing begins is turned away.
    let read = tokio::select! {
        read = tokio::time::timeout(ADMIN_READ_TIMEOUT, reader.read_line(&mut line)) => Some(read),
        () = books.closing.cancelled() => None,
    };
    let answer = match read {
        None => Response::Refused("the books are closing: nothing was done".into()),
        Some(Err(_)) => Response::Refused("no request arrived in time".into()),
        Some(Ok(Err(e))) => {
            Response::Refused(format!("the request couldn't be read: {}", e.kind()))
        }
        Some(Ok(Ok(_))) if !line.ends_with('\n') => {
            Response::Refused("the request is too long, or cut off".into())
        }
        Some(Ok(Ok(_))) => match serde_json::from_str::<admin::Envelope>(&line) {
            Err(e) => Response::Refused(format!("the request isn't understood: {e}")),
            Ok(envelope) if !admin::accepts(envelope.proto) => Response::Refused(
                "this command is from another version of wanderbooks than the books server: \
                 close the books (`wanderbooks close`) and try again"
                    .into(),
            ),
            Ok(_) if books.closing.is_cancelled() => {
                Response::Refused("the books are closing: nothing was done".into())
            }
            Ok(envelope) => {
                let books = books.clone();
                tokio::task::spawn_blocking(move || books.admin(envelope.request))
                    .await
                    .unwrap_or_else(|_| {
                        Response::Refused(
                            "the books server hit an internal error doing that".into(),
                        )
                    })
            }
        },
    };
    let mut text = serde_json::to_string(&answer).unwrap_or_else(|_| {
        r#"{"refused":"the books server couldn't write its answer"}"#.to_string()
    });
    text.push('\n');
    let _ = write.write_all(text.as_bytes()).await;
    let _ = write.shutdown().await;
}

/// Bind a socket in `run/`, replacing a stale one (INVARIANT 1: only the
/// server holding the lock gets here).
fn bind(run: &Path, name: &str) -> std::io::Result<tokio::net::UnixListener> {
    let path = run.join(name);
    match std::fs::remove_file(&path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let short = ledger_store::root::SocketPath::new(run, name)?;
    let listener = tokio::net::UnixListener::bind(short.path())?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

fn print_usage(prog: &str) {
    eprintln!("usage: {prog} [--root DIR] [--import-root DIR]...");
    eprintln!("  The books server for the data root: started by the host, not by hand.");
    eprintln!("  It runs until its standard input ends, or SIGTERM, SIGINT or SIGHUP.");
}

/// `wanderbooks server`: serve every book in the data root until told to
/// close (INVARIANT 6).
pub fn run(
    prog: &str,
    args: impl IntoIterator<Item = String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut root: Option<PathBuf> = None;
    let mut import_roots: Vec<PathBuf> = Vec::new();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--root" => root = Some(PathBuf::from(args.next().ok_or("--root needs a folder")?)),
            "--import-root" => import_roots.push(PathBuf::from(
                args.next().ok_or("--import-root needs a folder")?,
            )),
            "--help" | "-h" => {
                print_usage(prog);
                return Ok(());
            }
            other => {
                eprintln!("unknown argument: {other}");
                print_usage(prog);
                return Err(Box::new(crate::Exit(2)));
            }
        }
    }
    let root = match Root::find(root.as_deref()) {
        Ok(root) => root,
        Err(e) => {
            eprintln!("wanderbooks server: {e}");
            return Err(Box::new(crate::Exit(2)));
        }
    };
    let run = root.run_dir().map_err(|e| e.to_string())?;
    // INVARIANT 1: the lock first, before anything else is opened.
    let lock_path = run.join(SERVER_LOCK);
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .map_err(|e| format!("can't open the server lock: {}", e.kind()))?;
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    // INVARIANT 6: told to close at any point (a long wait or rebuild
    // included), the server closes cleanly as soon as it can.
    let stops = Stops::default();
    {
        let _in_runtime = rt.enter();
        watch_for_close(&stops)?;
    }
    // INVARIANT 1: a server still finishing (its host gone) has the lock;
    // this one waits for it, touching nothing, until told to close.
    let mut said = false;
    loop {
        match lock.try_lock() {
            Ok(()) => break,
            Err(std::fs::TryLockError::WouldBlock) => {
                if stops.close.is_cancelled() {
                    return Ok(());
                }
                if !said {
                    eprintln!(
                        "wanderbooks server: another books server has these books open; waiting for it to finish"
                    );
                    said = true;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(std::fs::TryLockError::Error(e)) => {
                return Err(format!("can't take the server lock: {}", e.kind()).into());
            }
        }
    }
    {
        // Only once it is ours: a refused server touches nothing.
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&lock_path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("can't make the server lock private: {}", e.kind()))?;
    }
    let import_roots = if import_roots.is_empty() {
        ImportRoots::defaults()
    } else {
        ImportRoots::new(import_roots)
    };
    let books = Arc::new(Books {
        root: root.clone(),
        import_roots,
        held: RwLock::new(BTreeMap::new()),
        sessions: CancellationToken::new(),
        closing: CancellationToken::new(),
        in_flight: AtomicUsize::new(0),
        finished: tokio::sync::Notify::new(),
        last_arrival: std::sync::atomic::AtomicU64::new(0),
        started: std::time::Instant::now(),
        principals: RwLock::new(Err(String::new())),
        session_tokens: Mutex::new(BTreeMap::new()),
    });
    {
        let loaded = books.load_principals();
        if let Err(why) = &loaded {
            eprintln!("wanderbooks server: agents' tokens can't be read: {why}");
        }
        *books.principals.write().unwrap_or_else(|e| e.into_inner()) = loaded;
    }
    // INVARIANT 3: every book tried before ready.
    for id in root.list_books().map_err(|e| e.to_string())? {
        let book = books.open(&id);
        books.hold(&id, book);
    }
    let served = if stops.close.is_cancelled() {
        eprintln!("wanderbooks server: told to close while opening: closed");
        Ok(())
    } else {
        rt.block_on(serve(books.clone(), &run, &stops))
    };
    // No request is running now (serve waited for each); the stores go
    // before the lock, and the process exit would release both.
    books
        .held
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .clear();
    rt.shutdown_timeout(Duration::from_secs(1));
    for name in [MCP_SOCKET, ADMIN_SOCKET] {
        let _ = std::fs::remove_file(run.join(name));
    }
    drop(lock);
    served
}

/// How long, once every running request has finished, the server still
/// waits for answers on their way out (or requests still arriving).
const ANSWER_GRACE: Duration = Duration::from_secs(10);
/// How long the door stays open, refusing, after the last request came.
const LINGER: Duration = Duration::from_millis(500);
/// The longest it stays open so.
const LINGER_MAX: Duration = Duration::from_secs(3);
/// How long the HTTP side then has to end before it is dropped.
const HTTP_GRACE: Duration = Duration::from_secs(5);

/// Being told to close: once (`close`), and again (`hurry`: stop waiting
/// for answers on their way out; never for a request still running).
#[derive(Default)]
struct Stops {
    close: CancellationToken,
    hurry: CancellationToken,
}

/// Bind, say ready, serve, and close (INVARIANTS 5, 6).
async fn serve(
    books: Arc<Books>,
    run: &Path,
    stops: &Stops,
) -> Result<(), Box<dyn std::error::Error>> {
    let mcp_listener = OwnerOnly(bind(run, MCP_SOCKET)?);
    let mut admin_listener = OwnerOnly(bind(run, ADMIN_SOCKET)?);
    let stop = stops.close.clone();
    {
        use std::io::Write;
        let mut out = std::io::stdout().lock();
        writeln!(out, "{READY}")?;
        out.flush()?;
    }
    let held: Vec<String> = books.held().keys().cloned().collect();
    eprintln!("wanderbooks server: ready, holding {} books", held.len());

    let app = axum::Router::new()
        .route("/b/{book}/mcp", axum::routing::any(mcp))
        .fallback(not_found)
        .with_state(books.clone());
    // The HTTP side keeps answering (refusing, once closing) until the
    // close is done: a client is never cut off mid-request by it.
    let shut = CancellationToken::new();
    let graceful = shut.clone();
    let mut http = tokio::spawn(async move {
        axum::serve(mcp_listener, app)
            .with_graceful_shutdown(async move { graceful.cancelled().await })
            .await
    });
    let admin_jobs = TaskTracker::new();
    loop {
        tokio::select! {
            _ = stop.cancelled() => break,
            stream = admin_listener.next() => {
                admin_jobs.spawn(serve_admin(stream, books.clone()));
            }
        }
    }
    drop(admin_listener);
    eprintln!("wanderbooks server: closing");
    // INVARIANT 6. New requests are refused from here on.
    books.closing.cancel();
    // Admin work under way (a create, an adoption) finishes.
    admin_jobs.close();
    admin_jobs.wait().await;
    // Every request already running finishes its work, however long it
    // takes and whether or not its client is still there: a store is
    // never let go of mid-write (SharedStore INVARIANT 3).
    for store in books.stores() {
        store.close().await;
    }
    // Answers on their way out are sent, for a while: a client sending
    // a request it never finishes doesn't hold the books open.
    let answers = async {
        loop {
            let notified = books.finished.notified();
            if books.in_flight.load(Ordering::SeqCst) == 0 {
                break;
            }
            notified.await;
        }
    };
    tokio::select! {
        () = answers => {}
        () = tokio::time::sleep(ANSWER_GRACE) => {
            eprintln!("wanderbooks server: a client didn't finish its request; closing without it");
        }
        () = stops.hurry.cancelled() => {}
    }
    books.sessions.cancel();
    // A call that set out just before its client heard of the close still
    // meets a refusal, never a connection cut mid-way (which it couldn't
    // tell from a call that ran): the door stays open, refusing, until
    // none has come for LINGER.
    let linger = async {
        let deadline = tokio::time::Instant::now() + LINGER_MAX;
        loop {
            let since = (books.started.elapsed().as_millis() as u64)
                .saturating_sub(books.last_arrival.load(Ordering::SeqCst));
            let quiet = Duration::from_millis(since);
            if quiet >= LINGER || tokio::time::Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(LINGER - quiet).await;
        }
    };
    tokio::select! {
        () = linger => {}
        () = stops.hurry.cancelled() => {}
    }
    shut.cancel();
    match tokio::time::timeout(HTTP_GRACE, &mut http).await {
        Ok(Ok(Ok(()))) => {}
        Ok(Ok(Err(e))) => eprintln!("wanderbooks server: {e}"),
        Ok(Err(e)) => eprintln!("wanderbooks server: {e}"),
        Err(_) => http.abort(),
    }
    eprintln!("wanderbooks server: closed");
    Ok(())
}

/// Close when standard input ends (the host let go, or died) or on
/// SIGTERM, SIGINT or SIGHUP; hurry on the next signal. Runs inside the
/// runtime, before the books are opened.
fn watch_for_close(stops: &Stops) -> std::io::Result<()> {
    use tokio::signal::unix::{SignalKind, signal};
    let on_eof = stops.close.clone();
    // A plain thread: a blocked read must never hold up the runtime's end.
    std::thread::Builder::new()
        .name("stdin-watch".into())
        .spawn(move || {
            use std::io::Read;
            let mut sink = [0u8; 256];
            let mut stdin = std::io::stdin();
            loop {
                match stdin.read(&mut sink) {
                    Ok(0) => break,
                    Ok(_) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(_) => break,
                }
            }
            on_eof.cancel();
        })?;
    for kind in [
        SignalKind::terminate(),
        SignalKind::interrupt(),
        SignalKind::hangup(),
    ] {
        let mut signals = signal(kind)?;
        let (close, hurry) = (stops.close.clone(), stops.hurry.clone());
        tokio::spawn(async move {
            while signals.recv().await.is_some() {
                if close.is_cancelled() {
                    hurry.cancel();
                } else {
                    close.cancel();
                }
            }
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_socket_pair_is_this_account_at_both_ends() {
        let (a, b) = std::os::unix::net::UnixStream::pair().unwrap();
        assert_eq!(peer_uid(&a).unwrap(), own_uid());
        assert!(peer_is_us(&b));
        assert!(check_peer_std(&a).is_ok());
    }

    #[test]
    fn a_socket_pair_names_this_process() {
        let (a, _b) = std::os::unix::net::UnixStream::pair().unwrap();
        if cfg!(target_os = "linux") {
            assert_eq!(peer_pid(&a), Some(std::process::id()));
        }
        let file = std::fs::File::open("/dev/null").unwrap();
        assert_eq!(peer_pid(&file), None);
    }

    #[test]
    fn a_descriptor_that_isnt_a_socket_is_never_taken_for_us() {
        let file = std::fs::File::open("/dev/null").unwrap();
        assert!(peer_uid(&file).is_err());
        assert!(!peer_is_us(&file));
    }
}
