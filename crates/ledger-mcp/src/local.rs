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
//!   9. A file's bytes come in only by `PUT /b/{id}/u/{upload}` and go
//!      out only by `GET /b/{id}/a/{attachment}` (a stored file) and `GET
//!      /b/{id}/x/{export}` (a file the books made, D-29) (plan 4.7), each
//!      through the same door as 8 and counted for the close like any
//!      request: an upload only from the token that began it, a download
//!      only with books:read, an export only to the token it was made for
//!      (whose tool passed its own scope check; no other is asked). Each,
//!      refused at the door or not, is recorded in the book's access log
//!      as `upload`, `download` or `export` with its id, and the
//!      bytes (kept, or read) and their record are one task that runs to
//!      its end if the client hangs up (a process killed in that task
//!      excepted); bytes whose record can't be written aren't kept, and
//!      the books stop. Lapsed uploads and exports are swept every
//!      SWEEP_EVERY. Nothing comes in or goes out once the books stopped.
//! ```

use crate::admin::{self, Request, Response};
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
use std::time::{Duration, Instant};
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

/// How large an MCP request may be (plan 3.2): files of any size come
/// through uploads (`/b/{book}/u/{upload}`); inline, at most 256 KiB.
pub const MAX_BODY: usize = 1024 * 1024;

/// How long the admin socket waits for a request to arrive.
const ADMIN_READ_TIMEOUT: Duration = Duration::from_secs(30);

use ledger_store::root::THE_ROOT;

/// The header on a request refused before it could run (books that
/// didn't open, books that don't exist): a client knows it didn't run.
pub const REFUSED_HEADER: &str = "x-wanderbooks-refused";

/// How often each book's lapsed uploads are swept.
const SWEEP_EVERY: Duration = Duration::from_secs(10);
/// How long the books stay unwritten before they are backed up.
const BACKUP_QUIET: Duration = Duration::from_secs(30);
/// How often the books are looked at for a backup due.
const BACKUP_LOOK: Duration = Duration::from_secs(2);
/// How often each book is looked at for a drill due.
const DRILL_LOOK: Duration = Duration::from_secs(3600);
/// How long after books are first seen open their first drill is looked
/// for (a drill that fell due while they were closed): long enough that a
/// backup on opening comes first.
const DRILL_FIRST: Duration = Duration::from_secs(60);
/// The most the last backup at a close may take.
const BACKUP_AT_CLOSE: Duration = Duration::from_secs(60);

type Service = StreamableHttpService<BooksServer, LocalSessionManager>;

/// A book the server holds.
#[derive(Clone)]
enum Book {
    /// Open and served.
    Open {
        /// Its MCP service (boxed: the other variant is a line of text).
        service: Box<Service>,
        /// Its store, closed when the server closes (INVARIANT 6).
        store: Arc<SharedStore>,
        /// Its access log, where requests refused at the door are
        /// recorded too (access.rs).
        access: Arc<AccessLog>,
        /// Its files on their way in (`/b/{id}/u/{upload}`).
        uploads: ledger_store::uploads::Uploads,
        /// The files it made for agents (`/b/{id}/x/{export}`).
        exports: ledger_store::exports::Exports,
        /// Its backups (backup.rs).
        backups: Arc<crate::backup::Backups>,
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
    /// How long the books stay unwritten before a backup is made
    /// (`--backup-quiet`, for tests; else BACKUP_QUIET).
    backup_quiet: Duration,
    /// How long after books are first seen open a drill is first looked
    /// for (`--drill-first`, for tests; else DRILL_FIRST).
    drill_first: Duration,
    /// Held through a restore: one at a time.
    restoring: Mutex<()>,
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
        let access = Arc::new(
            AccessLog::open(store.data_dir())
                .map_err(|e| format!("its access log can't be used: {e}"))?,
        );
        let uploads =
            ledger_store::uploads::Uploads::open(store.data_dir(), chrono::Utc::now().timestamp())
                .map_err(|e| self.hide(e.to_string()))?;
        let server_uploads = uploads.clone();
        let exports =
            ledger_store::exports::Exports::open(store.data_dir(), chrono::Utc::now().timestamp())
                .map_err(|e| self.hide(e.to_string()))?;
        let server_exports = exports.clone();
        // The viewer's side channel, made by the server (ops.rs
        // INVARIANT 2): a viewer can show its presence from the start.
        ledger_store::ops::prepare(store.data_dir()).map_err(|e| self.hide(e.to_string()))?;
        let live = Arc::new(Mutex::new(live));
        let book_dir = store.data_dir().to_path_buf();
        let book_id = book_dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let store = SharedStore::new(store);
        let backups = Arc::new(
            crate::backup::Backups::open(&book_id, &book_dir, store.clone())
                .map_err(|e| self.hide(e))?,
        );
        let server_backups = backups.clone();
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
                        .with_layout(layout.clone())
                        .with_access(log.clone())
                        .with_uploads(server_uploads.clone())
                        .with_exports(server_exports.clone())
                        .with_backups(server_backups.clone()),
                )
            },
            Arc::new(sessions()),
            config,
        );
        Ok(Book::Open {
            service: Box::new(service),
            store: shared,
            access,
            uploads,
            exports,
            backups,
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
            Request::Restore {
                id,
                source,
                replace,
                confirmed,
                expect_head,
            } => self.restore(id, source, replace, confirmed, expect_head),
            Request::BackUpNow { id, new_disk } => {
                let id = match BookId::parse(&id) {
                    Ok(id) => id,
                    Err(e) => return refused(e),
                };
                let backups = match self.held().get(id.as_str()) {
                    Some(Book::Open { backups, .. }) => backups.clone(),
                    Some(Book::Failed(why)) => {
                        return Response::Refused(format!("those books didn't open: {why}"));
                    }
                    None => {
                        return Response::Refused(format!(
                            "there are no books called {:?}",
                            id.as_str()
                        ));
                    }
                };
                let rt = tokio::runtime::Handle::current();
                match rt.block_on(backups.run_with("asked", new_disk)) {
                    Ok(status) => Response::BackedUp {
                        head_seq: status.head_seq,
                        targets: status
                            .targets
                            .into_iter()
                            .map(|t| admin::BackedUpTarget {
                                target: t.target,
                                at: t.at,
                                through_seq: t.through_seq,
                                problem: t.last_error,
                            })
                            .collect(),
                    },
                    Err(why) => Response::Refused(self.hide(why)),
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

impl Books {
    /// Restore a backup (admin INVARIANT 4; ledger_store::root INVARIANT
    /// 7): a preview, or the books restored and opened. Books it replaces
    /// are let go first, and opened again if the restore fails.
    fn restore(
        &self,
        id: String,
        source: String,
        replace: bool,
        confirmed: bool,
        expect_head: Option<(u64, String)>,
    ) -> Response {
        // admin INVARIANT 3: never the root, nor the address given.
        let said = |text: String| {
            let text = if source.chars().count() > 1 {
                text.replace(&source, "the backup given")
            } else {
                text
            };
            Response::Refused(self.hide(text))
        };
        let id = match BookId::parse(&id) {
            Ok(id) => id,
            Err(e) => return said(e.to_string()),
        };
        let _one = self.restoring.lock().unwrap_or_else(|e| e.into_inner());
        let git = match ledger_store::backup::Git::find() {
            Ok(git) => git,
            Err(e) => return said(e.to_string()),
        };
        // A preview only reports (whether books of that id exist among
        // what it says): the restore itself is refused over books, or
        // replacing none.
        if !confirmed {
            return match self.root.restore_preview(&git, &source, &id) {
                Ok(p) => Response::RestorePreview(admin::RestorePreview {
                    head: p.head,
                    blobs: p.blobs,
                    vault_files: p.vault_files,
                    exists: p.exists,
                    origin: p.origin,
                }),
                Err(e) => said(e.to_string()),
            };
        }
        let Some(expect_head) = expect_head else {
            return said("a restore is confirmed against the preview's head: preview first".into());
        };
        let held = self.held().contains_key(id.as_str());
        if replace && !held {
            return said(format!(
                "there are no books called {:?} to replace: nothing was restored",
                id.as_str()
            ));
        }
        if !replace && held {
            return said(format!(
                "there are already books called {:?}: restore them under another id (--id), or replace them (--replace; they are kept in legacy/)",
                id.as_str()
            ));
        }
        // The books replaced are let go (their writer lock released),
        // their backups idle until the restore is done: none of theirs
        // reaches a target after the backup is read.
        let old = if replace {
            self.held
                .write()
                .unwrap_or_else(|e| e.into_inner())
                .remove(id.as_str())
        } else {
            None
        };
        let old_backups = match &old {
            Some(Book::Open { backups, .. }) => Some(backups.clone()),
            _ => None,
        };
        let rt = tokio::runtime::Handle::current();
        // Its requests end first (a books.back_up_now among them waits for
        // a run of the loop's), then its last run.
        if let Some(Book::Open { store, .. }) = &old {
            rt.block_on(store.let_go());
        }
        let _paused = old_backups.as_ref().map(|b| rt.block_on(b.pause()));
        match self.root.restore(&git, &source, &id, &expect_head, replace) {
            Ok(r) => {
                // Restored: kept whether or not it opens; its backups told
                // whose books these were (backup.rs INVARIANTS 6, 7).
                let book = self.open(&id);
                let problem = book.problem();
                self.hold(&id, book);
                Response::Restored(admin::Restored {
                    id: r.id.to_string(),
                    head: r.head,
                    blobs_verified: r.blobs_verified,
                    vault_files: r.vault_files,
                    replaced: r.replaced,
                    origin: r.origin,
                    problem,
                })
            }
            Err(e) => {
                // The books it would have replaced, let go above, open
                // again (only this restore runs, so nothing else holds them).
                if replace && !self.held().contains_key(id.as_str()) {
                    let book = self.open(&id);
                    self.hold(&id, book);
                }
                said(e.to_string())
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

/// What a request asks for, as the access log names it: an MCP request
/// (action `connect`, its method), an upload (`upload`, its id) or a
/// download (`download`, the attachment's id).
struct Asked {
    action: &'static str,
    subject: String,
}

impl Asked {
    fn connect(method: &axum::http::Method) -> Self {
        Self {
            action: "connect",
            subject: method.as_str().to_string(),
        }
    }
}

/// A request refused at the door: recorded in the book's access log
/// (access.rs INVARIANT 3), then answered with `status` and `why`.
async fn refused_at_door(
    book: &Book,
    principal: Principal,
    asked: &Asked,
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
            action: asked.action.into(),
            // Masked, then cut, by the log itself (access.rs INVARIANT 2).
            subject: asked.subject.clone(),
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

/// The door to `book` (INVARIANT 8): the books held open, the server not
/// closing, and a live token for those books; else the answer refusing
/// the request (recorded in the books' access log, where there is one).
async fn door(
    books: &Arc<Books>,
    book: &str,
    asked: &Asked,
    headers: &axum::http::HeaderMap,
) -> Result<(Book, Token), Box<axum::response::Response>> {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    let closing = books.closing.is_cancelled();
    let found = books.held().get(book).cloned();
    let held = match found {
        Some(Book::Failed(why)) if !closing => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                [(REFUSED_HEADER, "not-open")],
                format!("the books {book} didn't open: {why}"),
            )
                .into_response()
                .into());
        }
        None if !closing => {
            return Err((
                StatusCode::NOT_FOUND,
                [(REFUSED_HEADER, "no-such-books")],
                format!(
                    "there are no books called {:?}: `wanderbooks books list` lists them",
                    book.chars().take(64).collect::<String>()
                ),
            )
                .into_response()
                .into());
        }
        Some(b @ Book::Open { .. }) => b,
        _ => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "the books are closing: this request wasn't run",
            )
                .into_response()
                .into());
        }
    };
    let token = books.bearer(headers);
    let principal = |t: &Token| Principal {
        id: t.id.clone(),
        kind: "agent".into(),
        label: t.name.clone(),
    };
    if closing {
        let who = token
            .as_ref()
            .map_or_else(|_| Principal::unknown(), principal);
        return Err(refused_at_door(
            &held,
            who,
            asked,
            crate::access::CLOSING,
            StatusCode::SERVICE_UNAVAILABLE,
            "the books are closing: this request wasn't run".into(),
        )
        .await
        .into());
    }
    let token = match token {
        Ok(t) => t,
        Err((code, why)) => {
            return Err(refused_at_door(
                &held,
                Principal::unknown(),
                asked,
                code,
                StatusCode::UNAUTHORIZED,
                why,
            )
            .await
            .into());
        }
    };
    if token.book != book {
        return Err(refused_at_door(
            &held,
            principal(&token),
            asked,
            "wrong_books",
            StatusCode::FORBIDDEN,
            format!("this token is for the books {}, not {book}", token.book),
        )
        .await
        .into());
    }
    Ok((held, token))
}

/// `PUT /b/{book}/u/{upload}`: the bytes of an upload begun with
/// `books.begin_upload`, from the token that began it (INVARIANT 9; plan
/// 4.7): at most the upload's limit, kept only if exactly the size and
/// SHA-256 declared, once; recorded in the access log (action `upload`).
async fn upload(
    axum::extract::State(books): axum::extract::State<Arc<Books>>,
    axum::extract::Path((book, id)): axum::extract::Path<(String, String)>,
    request: axum::extract::Request,
) -> axum::response::Response {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use ledger_store::uploads::UploadError;
    let _counted = InFlight::new(&books);
    books
        .last_arrival
        .store(books.started.elapsed().as_millis() as u64, Ordering::SeqCst);
    let asked = Asked {
        action: "upload",
        subject: id.clone(),
    };
    let (held, token) = match door(&books, &book, &asked, request.headers()).await {
        Ok(passed) => passed,
        Err(refused) => {
            // What was sent is read and dropped (up to the most an upload
            // can be: only this account reaches the socket), so the client
            // gets the refusal, not a connection cut mid-send.
            use futures_util::StreamExt;
            let mut rest = request.into_body().into_data_stream();
            let mut read = 0u64;
            while let Some(Ok(chunk)) = rest.next().await {
                read += chunk.len() as u64;
                if read > ledger_store::uploads::MAX_UPLOAD {
                    break;
                }
            }
            return *refused;
        }
    };
    let Book::Open {
        access,
        uploads,
        store,
        ..
    } = &held
    else {
        unreachable!("the door lets only open books through")
    };
    // Streamed to the disk as it comes, never held whole; `receive` reads
    // no more than the size declared and one byte. The bytes and their
    // record are one blocking task, which runs to its end even if the
    // client hangs up: never bytes kept without their record, and a close
    // waits for both (the count goes with them).
    use futures_util::TryStreamExt;
    let stream = request
        .into_body()
        .into_data_stream()
        .map_err(std::io::Error::other);
    let mut body = tokio_util::io::SyncIoBridge::new(tokio_util::io::StreamReader::new(stream));
    let (uploads, access, stopped) = (uploads.clone(), access.clone(), store.is_stopped());
    let task = tokio::task::spawn_blocking(move || {
        let _counted = _counted;
        // The books server stopped after an internal error: nothing more
        // comes in (SharedStore INVARIANT 2).
        let received = if stopped {
            Err(UploadError::Bad(crate::server::STOPPED.into()))
        } else {
            uploads.receive(&id, &token.id, &mut body, chrono::Utc::now().timestamp())
        };
        // Refused: the rest of what was sent (up to the most an upload
        // can be) is read and dropped, so the client gets the answer, not
        // a connection cut while it was still sending.
        if received.is_err() {
            use std::io::Read;
            let rest = ledger_store::uploads::MAX_UPLOAD + 1;
            let _ = std::io::copy(&mut (&mut body).take(rest), &mut std::io::sink());
        }
        let (status, outcome, text) = match &received {
            Ok(g) => (
                StatusCode::OK,
                Outcome::Ok,
                serde_json::json!({"received": g.id, "size": g.size, "sha256": g.sha256})
                    .to_string(),
            ),
            Err(e) => {
                let (status, code) = match e {
                    _ if stopped => (StatusCode::SERVICE_UNAVAILABLE, "stopped"),
                    UploadError::Unknown(_) => (StatusCode::NOT_FOUND, "no_such_upload"),
                    UploadError::NotYours => (StatusCode::FORBIDDEN, "not_yours"),
                    UploadError::AlreadySent => (StatusCode::CONFLICT, "already_sent"),
                    UploadError::InUse => (StatusCode::CONFLICT, "in_use"),
                    UploadError::WrongSize { .. }
                    | UploadError::TooMuch { .. }
                    | UploadError::WrongHash => (StatusCode::BAD_REQUEST, "not_as_declared"),
                    _ => (StatusCode::INTERNAL_SERVER_ERROR, "failed"),
                };
                (status, Outcome::Refused(code.into()), e.to_string())
            }
        };
        let record = Access {
            principal: Principal {
                id: token.id.clone(),
                kind: "agent".into(),
                label: token.name.clone(),
            },
            client: None,
            request_id: None,
            action: "upload".into(),
            subject: id.clone(),
            args_digest: None,
            result: outcome.clone(),
            head_after: None,
        };
        let recorded = access.record(&record).is_ok();
        if !recorded && received.is_ok() {
            // Never bytes kept without their record: sent again later.
            uploads.discard(&id);
        }
        (status, outcome, text, recorded)
    });
    let Ok((status, outcome, text, true)) = task.await else {
        store.stop();
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            [(REFUSED_HEADER, "not-recorded")],
            crate::access::NOT_RECORDED,
        )
            .into_response();
    };
    match outcome {
        Outcome::Refused(code) => (status, [(REFUSED_HEADER, code)], text).into_response(),
        _ => (status, text).into_response(),
    }
}

/// `GET /b/{book}/a/{attachment}`: a stored file's bytes, for a token
/// with books:read (INVARIANT 9; what `wanderbooks download` asks for);
/// recorded in the access log (action `download`).
async fn download(
    axum::extract::State(books): axum::extract::State<Arc<Books>>,
    axum::extract::Path((book, attachment)): axum::extract::Path<(String, String)>,
    request: axum::extract::Request,
) -> axum::response::Response {
    fetch(books, book, Wanted::Attachment(attachment), request).await
}

/// `GET /b/{book}/x/{export}`: a file the books made for an agent
/// (exports.rs; D-29), only to the token it was made for (INVARIANT 9);
/// recorded in the access log (action `export`).
async fn export(
    axum::extract::State(books): axum::extract::State<Arc<Books>>,
    axum::extract::Path((book, id)): axum::extract::Path<(String, String)>,
    request: axum::extract::Request,
) -> axum::response::Response {
    fetch(books, book, Wanted::Export(id), request).await
}

/// What a GET fetches.
enum Wanted {
    /// A stored receipt or document, by its id.
    Attachment(String),
    /// A file the books made, by its id.
    Export(String),
}

/// A download or an export: the door, the read and its record, the
/// headers (INVARIANT 9).
async fn fetch(
    books: Arc<Books>,
    book: String,
    wanted: Wanted,
    request: axum::extract::Request,
) -> axum::response::Response {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    // GET only (axum hands a HEAD to a GET route): a HEAD would read the
    // file and record a download that sent nothing.
    if request.method() != axum::http::Method::GET {
        return (
            StatusCode::METHOD_NOT_ALLOWED,
            [(axum::http::header::ALLOW, "GET")],
            "a download is a GET",
        )
            .into_response();
    }
    let _counted = InFlight::new(&books);
    books
        .last_arrival
        .store(books.started.elapsed().as_millis() as u64, Ordering::SeqCst);
    let (action, subject) = match &wanted {
        Wanted::Attachment(id) => ("download", id.clone()),
        Wanted::Export(id) => ("export", id.clone()),
    };
    let asked = Asked {
        action,
        subject: subject.clone(),
    };
    let (held, token) = match door(&books, &book, &asked, request.headers()).await {
        Ok(passed) => passed,
        Err(refused) => return *refused,
    };
    let Book::Open {
        access,
        store,
        exports,
        ..
    } = &held
    else {
        unreachable!("the door lets only open books through")
    };
    // The read and its record are one blocking task, run to its end even
    // if the client hangs up, and counted until then.
    let (access, store_for, exports) = (access.clone(), store.clone(), exports.clone());
    let task = tokio::task::spawn_blocking(move || {
        let _counted = _counted;
        use ledger_store::exports::ExportError;
        let not_found = |code, what: &str| {
            (
                StatusCode::NOT_FOUND,
                code,
                format!("there is no such {what}"),
            )
        };
        let found = if store_for.is_stopped() {
            // Stopped after an internal error: as for uploads.
            Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "stopped",
                crate::server::STOPPED.to_string(),
            ))
        // An export is bound to the token it was made for, whose tool
        // passed its own scope check; a stored file needs books:read.
        } else if matches!(wanted, Wanted::Attachment(_)) && !token.scopes.contains(&Scope::Read) {
            Err((
                StatusCode::FORBIDDEN,
                "insufficient_scope",
                format!(
                    "this agent's token can't do that: a download needs books:read, and its token ({:?}) hasn't it",
                    token.name
                ),
            ))
        } else {
            match &wanted {
                Wanted::Export(id) => {
                    match exports.get(id, &token.id, chrono::Utc::now().timestamp()) {
                        Ok((x, bytes)) => Ok((x.file_name, x.mime, bytes)),
                        Err(e @ ExportError::Unknown(_)) => {
                            Err((StatusCode::NOT_FOUND, "no_such_export", e.to_string()))
                        }
                        Err(e @ ExportError::NotYours) => {
                            Err((StatusCode::FORBIDDEN, "not_yours", e.to_string()))
                        }
                        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, "failed", e.to_string())),
                    }
                }
                // An id as the books write it (no sign, no leading zeros),
                // so the log holds no alias of it.
                Wanted::Attachment(a) => {
                    match a.parse::<u64>().ok().filter(|n| n.to_string() == *a) {
                        None => Err(not_found("no_such_attachment", "attachment")),
                        Some(id) => match store_for.attachment_file(id) {
                            Ok(Some(file)) => Ok(file),
                            Ok(None) => Err(not_found("no_such_attachment", "attachment")),
                            // The stored file damaged: not a missing one.
                            Err(why) => Err((StatusCode::INTERNAL_SERVER_ERROR, "failed", why)),
                        },
                    }
                }
            }
        };
        let record = Access {
            principal: Principal {
                id: token.id.clone(),
                kind: "agent".into(),
                label: token.name.clone(),
            },
            client: None,
            request_id: None,
            action: action.into(),
            subject,
            args_digest: None,
            result: match &found {
                Ok(_) => Outcome::Ok,
                Err((_, code, _)) => Outcome::Refused((*code).into()),
            },
            head_after: None,
        };
        let recorded = access.record(&record).is_ok();
        (found, recorded)
    });
    let Ok((found, true)) = task.await else {
        store.stop();
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            [(REFUSED_HEADER, "not-recorded")],
            crate::access::NOT_RECORDED,
        )
            .into_response();
    };
    match found {
        Ok((name, mime, bytes)) => {
            let digest = format!(
                "sha-256=:{}:",
                base64::Engine::encode(
                    &base64::engine::general_purpose::STANDARD,
                    <sha2::Sha256 as sha2::Digest>::digest(&bytes)
                )
            );
            let mime = axum::http::HeaderValue::from_str(&mime).unwrap_or(
                axum::http::HeaderValue::from_static("application/octet-stream"),
            );
            (
                StatusCode::OK,
                [
                    (axum::http::header::CONTENT_TYPE, mime),
                    (
                        axum::http::header::CONTENT_DISPOSITION,
                        axum::http::HeaderValue::from_str(&disposition(&name))
                            .expect("only visible ASCII"),
                    ),
                    (
                        axum::http::header::X_CONTENT_TYPE_OPTIONS,
                        axum::http::HeaderValue::from_static("nosniff"),
                    ),
                    (
                        axum::http::HeaderName::from_static(DIGEST_HEADER),
                        axum::http::HeaderValue::from_str(&digest).expect("base64 is ASCII"),
                    ),
                ],
                bytes,
            )
                .into_response()
        }
        Err((status, code, why)) => (status, [(REFUSED_HEADER, code)], why).into_response(),
    }
}

/// The header a download's SHA-256 rides in (RFC 9530 Repr-Digest), so
/// `wanderbooks download` keeps only what was stored.
pub const DIGEST_HEADER: &str = "repr-digest";

/// A download's Content-Disposition (RFC 6266): its name in ASCII, every
/// other character `_`, and whole in UTF-8 as `filename*` (RFC 8187).
fn disposition(name: &str) -> String {
    let plain: String = name
        .chars()
        .map(|c| {
            if (c.is_ascii_graphic() || c == ' ') && c != '"' && c != '\\' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let mut encoded = String::new();
    for b in name.bytes() {
        if b.is_ascii_alphanumeric() || b"!#$&+-.^_`|~".contains(&b) {
            encoded.push(b as char);
        } else {
            encoded.push_str(&format!("%{b:02X}"));
        }
    }
    format!("attachment; filename=\"{plain}\"; filename*=UTF-8''{encoded}")
}

/// `/b/{book}/mcp`: the book's MCP service, for a request carrying a token
/// for that book (INVARIANT 8).
async fn mcp(
    axum::extract::State(books): axum::extract::State<Arc<Books>>,
    axum::extract::Path(book): axum::extract::Path<String>,
    mut request: axum::extract::Request,
) -> axum::response::Response {
    use axum::http::StatusCode;
    use tower::ServiceExt;
    // Counted before the closing check, so closing never misses it.
    let counted = (request.method() != axum::http::Method::GET).then(|| InFlight::new(&books));
    books
        .last_arrival
        .store(books.started.elapsed().as_millis() as u64, Ordering::SeqCst);
    let method = request.method().clone();
    let (held, token) = match door(&books, &book, &Asked::connect(&method), request.headers()).await
    {
        Ok(passed) => passed,
        Err(refused) => return *refused,
    };
    let Book::Open { service, .. } = &held else {
        unreachable!("the door lets only open books through")
    };
    let service = (**service).clone();
    let principal = |t: &Token| Principal {
        id: t.id.clone(),
        kind: "agent".into(),
        label: t.name.clone(),
    };
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
            &Asked::connect(&method),
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
            &Asked::connect(&method),
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
    eprintln!("usage: {prog} [--root DIR]");
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
    let mut backup_quiet = BACKUP_QUIET;
    let mut drill_first = DRILL_FIRST;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--root" => root = Some(PathBuf::from(args.next().ok_or("--root needs a folder")?)),
            // How long the books stay unwritten before a backup (tests).
            "--backup-quiet" => {
                let ms: u64 = args
                    .next()
                    .and_then(|v| v.parse().ok())
                    .ok_or("--backup-quiet needs milliseconds")?;
                backup_quiet = Duration::from_millis(ms);
            }
            // How soon after books open a drill is first looked for (tests).
            "--drill-first" => {
                let ms: u64 = args
                    .next()
                    .and_then(|v| v.parse().ok())
                    .ok_or("--drill-first needs milliseconds")?;
                drill_first = Duration::from_millis(ms);
            }
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
    let books = Arc::new(Books {
        root: root.clone(),
        held: RwLock::new(BTreeMap::new()),
        sessions: CancellationToken::new(),
        closing: CancellationToken::new(),
        in_flight: AtomicUsize::new(0),
        finished: tokio::sync::Notify::new(),
        last_arrival: std::sync::atomic::AtomicU64::new(0),
        started: std::time::Instant::now(),
        principals: RwLock::new(Err(String::new())),
        session_tokens: Mutex::new(BTreeMap::new()),
        backup_quiet,
        drill_first,
        restoring: Mutex::new(()),
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
    // Every open book is backed up when it opens, once its writing has
    // stopped for a while, and again after a failed run (backup.rs).
    let (backing, backup_stop) = (books.clone(), stop.clone());
    let backup_loop = tokio::spawn(async move { keep_backed_up(backing, backup_stop).await });
    // Every open book's lapsed uploads go within SWEEP_EVERY of lapsing,
    // whatever else the server is doing (uploads.rs INVARIANT 3).
    let (sweeping, sweep_stop) = (books.clone(), stop.clone());
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(SWEEP_EVERY);
        loop {
            tokio::select! {
                () = sweep_stop.cancelled() => break,
                _ = tick.tick() => {
                    let held: Vec<_> = sweeping
                        .held()
                        .values()
                        .filter_map(|b| match b {
                            Book::Open { uploads, exports, .. } => {
                                Some((uploads.clone(), exports.clone()))
                            }
                            Book::Failed(_) => None,
                        })
                        .collect();
                    let now = chrono::Utc::now().timestamp();
                    let _ = tokio::task::spawn_blocking(move || {
                        for (u, x) in &held {
                            u.sweep(now);
                            x.sweep(now);
                        }
                    })
                    .await;
                }
            }
        }
    });

    let app = axum::Router::new()
        .route("/b/{book}/mcp", axum::routing::any(mcp))
        .route("/b/{book}/u/{upload}", axum::routing::put(upload))
        .route("/b/{book}/a/{attachment}", axum::routing::get(download))
        .route("/b/{book}/x/{export}", axum::routing::get(export))
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
    // A last backup of what was written since the last one, for a while
    // at most: the books are closing, nothing more is written. The loop
    // stops at its next step (`stop`); a run of its still under way
    // finishes first (each book runs one at a time).
    drop(backup_loop);
    let held: Vec<Arc<crate::backup::Backups>> = books
        .held()
        .values()
        .filter_map(|b| match b {
            Book::Open { backups, .. } => Some(backups.clone()),
            Book::Failed(_) => None,
        })
        .collect();
    for b in held {
        let _ = tokio::time::timeout(BACKUP_AT_CLOSE, b.run("close")).await;
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

/// What the backup loop knows of one open book.
struct Seen {
    /// Its backups, by identity: books restored over others are new.
    backups: Arc<crate::backup::Backups>,
    watch: Watch,
}

/// One open book's backups, as the loop has watched them.
#[derive(Debug, Clone, Copy)]
struct Watch {
    /// Its head's seq when last looked at, and when that changed.
    seq: u64,
    moved_at: Instant,
    /// The seq last committed and offered to the targets.
    backed: Option<u64>,
    /// When the last run's commit failed, if it did: nothing runs again
    /// for RETRY_AFTER.
    failed_at: Option<Instant>,
    /// When the last run left a target failing, if it did: it is tried
    /// again RETRY_AFTER on, written to or not.
    target_failed_at: Option<Instant>,
    /// When a drill is next looked for.
    drill_next: Instant,
}

impl Watch {
    /// Books first seen open now, at `seq`: a drill is first looked for
    /// `drill_first` from now.
    fn new(seq: u64, now: Instant, drill_first: Duration) -> Self {
        Watch {
            seq,
            moved_at: now,
            backed: None,
            failed_at: None,
            target_failed_at: None,
            drill_next: now + drill_first,
        }
    }

    /// A run through `seq` came to `outcome` at `now`: None when its
    /// commit failed, else whether every target holds it.
    fn ran(&mut self, seq: u64, outcome: Option<bool>, now: Instant) {
        match outcome {
            None => self.failed_at = Some(now),
            Some(all) => {
                self.backed = Some(seq);
                self.failed_at = None;
                self.target_failed_at = (!all).then_some(now);
            }
        }
    }
}

/// Why a backup is due now, if one is: the books just seen open; or
/// written and quiet for `quiet` since (not yet backed up); or a target
/// left failing RETRY_AFTER ago. Nothing within RETRY_AFTER of a failed
/// commit.
fn backup_due(
    seen: Option<&Watch>,
    seq: u64,
    now: Instant,
    quiet: Duration,
) -> Option<&'static str> {
    let Some(s) = seen else {
        return Some("open");
    };
    let after =
        |t: Option<Instant>| t.is_some_and(|f| now.duration_since(f) >= crate::backup::RETRY_AFTER);
    if s.failed_at.is_some() && !after(s.failed_at) {
        return None;
    }
    let quiet_now = if s.seq == seq {
        now.duration_since(s.moved_at) >= quiet
    } else {
        quiet.is_zero()
    };
    if s.backed != Some(seq) && quiet_now {
        return Some("write");
    }
    after(s.target_failed_at).then_some("retry")
}

/// Whether a run's every target holds the books: a target paused for
/// these books (restored as a copy of others) is no failure to try again.
fn every_target_held(status: &crate::backup::Status) -> bool {
    status
        .targets
        .iter()
        .all(|t| t.paused || t.last_error.is_none())
}

/// Whether to look for a drill due now (then again DRILL_LOOK on).
fn drill_look_due(seen: &Watch, now: Instant) -> bool {
    now >= seen.drill_next
}

/// Back every open book up (backup.rs) whenever [`backup_due`] says, and
/// drill its backups when a drill is due (looked for every DRILL_LOOK).
/// Until `stop`.
async fn keep_backed_up(books: Arc<Books>, stop: CancellationToken) {
    let mut seen: BTreeMap<String, Seen> = BTreeMap::new();
    let mut tick = tokio::time::interval(BACKUP_LOOK);
    loop {
        tokio::select! {
            () = stop.cancelled() => return,
            _ = tick.tick() => {}
        }
        let open: Vec<(String, Arc<crate::backup::Backups>)> = books
            .held()
            .iter()
            .filter_map(|(id, b)| match b {
                Book::Open { backups, .. } => Some((id.clone(), backups.clone())),
                Book::Failed(_) => None,
            })
            .collect();
        seen.retain(|id, _| open.iter().any(|(o, _)| o == id));
        for (id, backups) in open {
            if stop.is_cancelled() {
                return;
            }
            let Some((seq, _)) = backups.head().await else {
                continue;
            };
            let now = Instant::now();
            // Books restored over others are seen anew.
            if seen
                .get(&id)
                .is_some_and(|s| !Arc::ptr_eq(&s.backups, &backups))
            {
                seen.remove(&id);
            }
            let due = backup_due(
                seen.get(&id).map(|s| &s.watch),
                seq,
                now,
                books.backup_quiet,
            );
            let s = &mut seen
                .entry(id)
                .or_insert(Seen {
                    backups: backups.clone(),
                    watch: Watch::new(seq, now, books.drill_first),
                })
                .watch;
            if s.seq != seq {
                s.seq = seq;
                s.moved_at = now;
            }
            if let Some(why) = due {
                let outcome = backups
                    .run(why)
                    .await
                    .ok()
                    .map(|status| every_target_held(&status));
                s.ran(seq, outcome, now);
            }
            if stop.is_cancelled() {
                return;
            }
            // Drills (backup.rs INVARIANT 5).
            if drill_look_due(s, now) {
                s.drill_next = now + DRILL_LOOK;
                if backups.drill_due().await {
                    backups.drill().await;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_backup_is_due_on_open_then_once_quiet_after_a_write() {
        let t0 = Instant::now();
        let quiet = Duration::from_secs(30);
        let s = |seq, moved: u64, backed, failed: Option<u64>| Watch {
            seq,
            moved_at: t0 + Duration::from_secs(moved),
            backed,
            failed_at: failed.map(|f| t0 + Duration::from_secs(f)),
            target_failed_at: None,
            drill_next: t0,
        };
        let at = |secs: u64| t0 + Duration::from_secs(secs);
        // First seen: at once.
        assert_eq!(backup_due(None, 5, t0, quiet), Some("open"));
        // Backed up through the head: nothing.
        assert_eq!(
            backup_due(Some(&s(5, 0, Some(5), None)), 5, at(100), quiet),
            None
        );
        // Written: not until it has been quiet `quiet`.
        assert_eq!(
            backup_due(Some(&s(5, 0, Some(5), None)), 6, at(1), quiet),
            None
        );
        assert_eq!(
            backup_due(Some(&s(6, 10, Some(5), None)), 6, at(39), quiet),
            None
        );
        assert_eq!(
            backup_due(Some(&s(6, 10, Some(5), None)), 6, at(40), quiet),
            Some("write")
        );
        // No quiet asked for: at once.
        assert_eq!(
            backup_due(Some(&s(5, 0, Some(5), None)), 6, at(1), Duration::ZERO),
            Some("write")
        );
        // A failure waits RETRY_AFTER.
        let retry = crate::backup::RETRY_AFTER.as_secs();
        let failed = s(6, 10, Some(5), Some(50));
        assert_eq!(
            backup_due(Some(&failed), 6, at(50 + retry - 1), quiet),
            None
        );
        assert_eq!(
            backup_due(Some(&failed), 6, at(50 + retry), quiet),
            Some("write")
        );
        // A drill is first looked for `drill_first` after the books are
        // seen open, never at once.
        let first = Duration::from_secs(60);
        let w = Watch::new(1, t0, first);
        assert!(!drill_look_due(&w, t0));
        assert!(!drill_look_due(&w, t0 + first - Duration::from_millis(1)));
        assert!(drill_look_due(&w, t0 + first));
        assert_eq!((w.backed, w.failed_at, w.seq), (None, None, 1));
        // A run's outcome: its commit failed (nothing again for a
        // while), or made and some target failing (tried again later,
        // while writes still go to the others), or all well.
        let mut w = Watch::new(1, t0, first);
        w.ran(4, None, at(5));
        assert_eq!(
            (w.backed, w.failed_at, w.target_failed_at),
            (None, Some(at(5)), None)
        );
        w.ran(4, Some(false), at(9));
        assert_eq!(
            (w.backed, w.failed_at, w.target_failed_at),
            (Some(4), None, Some(at(9)))
        );
        w.seq = 5;
        w.moved_at = at(10);
        assert_eq!(backup_due(Some(&w), 5, at(10 + 30), quiet), Some("write"));
        w.backed = Some(5);
        assert_eq!(backup_due(Some(&w), 5, at(9 + retry - 1), quiet), None);
        assert_eq!(backup_due(Some(&w), 5, at(9 + retry), quiet), Some("retry"));
        // A paused target is no failure; another's error is.
        let target = |paused: bool, error: Option<&str>| crate::backup::TargetStatus {
            target: "repository".into(),
            at: "x".into(),
            last_ok_at: None,
            through_seq: None,
            stale: false,
            last_error: error.map(str::to_string),
            last_drill_at: None,
            drill_error: None,
            paused,
        };
        let status = |targets| crate::backup::Status {
            head_seq: 5,
            targets,
            note: None,
        };
        assert!(every_target_held(&status(vec![
            target(true, Some("paused")),
            target(false, None)
        ])));
        assert!(!every_target_held(&status(vec![
            target(false, Some("no")),
            target(false, None)
        ])));
        assert!(every_target_held(&status(vec![])));
        w.ran(5, Some(true), at(9 + retry));
        assert_eq!((w.failed_at, w.target_failed_at), (None, None));
        assert_eq!(backup_due(Some(&w), 5, at(9 + retry * 3), quiet), None);
    }

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

    /// A socket with no other end: the system answers for it (Linux: no
    /// process, no user), and that answer names neither this process nor
    /// this account; the client's check refuses it.
    #[test]
    fn a_socket_with_no_other_end_names_no_process_and_isnt_us() {
        use std::os::fd::FromRawFd;
        // SAFETY: socket(2) has no preconditions; the descriptor is checked
        // and then owned by the stream alone.
        let fd = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
        assert!(fd >= 0);
        // SAFETY: a fresh descriptor of a Unix stream socket, owned by none.
        let lone = unsafe { std::os::unix::net::UnixStream::from_raw_fd(fd) };
        assert_eq!(peer_pid(&lone), None);
        assert!(!peer_is_us(&lone));
        assert_eq!(
            check_peer_std(&lone).unwrap_err(),
            "the books server's socket belongs to another account: not used"
        );
    }

    /// A session ends only after a day without a message (SESSION_IDLE),
    /// never rmcp's few minutes: a call longer than that keeps its answer.
    #[test]
    fn a_session_ends_only_after_a_day_without_a_message() {
        assert_eq!(
            sessions().session_config.keep_alive,
            Some(Duration::from_secs(86_400))
        );
    }

    /// Settings moved in at adoption are dated in the time zone the file
    /// sets (company.timezone), not by any other entry's text. The two
    /// zones are 26 hours apart, so their dates always differ.
    #[test]
    fn settings_moved_in_are_dated_in_the_files_own_time_zone() {
        let change = |key: &str, value: &str| ledger_core::settings::Change {
            key: key.into(),
            value: Some(serde_json::json!(value)),
        };
        let changes = [
            change("company.name", "Etc/GMT+12"),
            change("company.timezone", "Pacific/Kiritimati"),
        ];
        let source = moved_in_at_adoption(&changes);
        assert_eq!(
            source.decided_on,
            ledger_core::settings::today_in(Some("Pacific/Kiritimati"))
        );
        assert_ne!(
            source.decided_on,
            ledger_core::settings::today_in(Some("Etc/GMT+12"))
        );
        assert_eq!(source.decided_by, "the settings file");
    }

    /// A server's books in a fresh root under `home`, as `run` holds them,
    /// without its sockets: each test drives the handlers itself.
    fn books_in(home: &Path) -> Arc<Books> {
        let (root, _) = Root::setup(home, Some(&home.join("root")), None).unwrap();
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
        });
        let loaded = books.load_principals();
        *books.principals.write().unwrap() = loaded;
        books
    }

    /// Make the books `id` through the admin request.
    fn create(books: &Books, id: &str) {
        match books.admin(Request::Create {
            id: id.into(),
            currency: "USD".into(),
        }) {
            Response::Created { problem: None, .. } => {}
            other => panic!("{other:?}"),
        }
    }

    /// A token for `book` named `name` with every scope (made, or made
    /// again): its id and its text.
    fn token(books: &Books, book: &str, name: &str) -> (String, String) {
        match books.admin(Request::TokenCreate {
            book: book.into(),
            name: name.into(),
            scopes: ["read", "write", "post", "admin"]
                .map(String::from)
                .to_vec(),
            replace: true,
        }) {
            Response::TokenCreated { token, text } => (token.id, text.0),
            other => panic!("{other:?}"),
        }
    }

    /// The sessions bound, by id.
    fn bound(books: &Books) -> Vec<String> {
        books
            .session_tokens
            .lock()
            .unwrap()
            .keys()
            .cloned()
            .collect()
    }

    /// The status, the refusal code and the text of an answer.
    async fn answer(r: axum::response::Response) -> (u16, Option<String>, String) {
        let code = r
            .headers()
            .get(REFUSED_HEADER)
            .map(|v| v.to_str().unwrap().to_string());
        let status = r.status().as_u16();
        let body = axum::body::to_bytes(r.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, code, String::from_utf8_lossy(&body).into_owned())
    }

    /// A PUT of `body` for upload `id` of the books `acme`.
    fn put(id: &str, bearer: Option<&str>, body: axum::body::Body) -> axum::extract::Request {
        let mut r = axum::http::Request::builder()
            .method("PUT")
            .uri(format!("http://localhost/b/acme/u/{id}"));
        if let Some(b) = bearer {
            r = r.header("authorization", format!("Bearer {b}"));
        }
        r.body(body).unwrap()
    }

    /// Send a PUT to the upload route as the router would.
    async fn send_upload(
        books: &Arc<Books>,
        id: &str,
        bearer: Option<&str>,
        body: axum::body::Body,
    ) -> axum::response::Response {
        upload(
            axum::extract::State(books.clone()),
            axum::extract::Path(("acme".to_string(), id.to_string())),
            put(id, bearer, body),
        )
        .await
    }

    /// A request body that arrives as the test sends it, one chunk at a
    /// time (the sender's `reserve` returns once the last chunk was taken).
    fn fed() -> (tokio::sync::mpsc::Sender<Vec<u8>>, axum::body::Body) {
        let (tx, rx) = tokio::sync::mpsc::channel::<Vec<u8>>(1);
        let stream = futures_util::stream::unfold(rx, |mut rx| async move {
            rx.recv()
                .await
                .map(|b| (Ok::<_, std::io::Error>(axum::body::Bytes::from(b)), rx))
        });
        (tx, axum::body::Body::from_stream(stream))
    }

    /// The upload's grant, for `token_id`, of `bytes`.
    fn grant(books: &Books, token_id: &str, bytes: &[u8]) -> ledger_store::uploads::Grant {
        let held = books.held();
        let Some(Book::Open { uploads, .. }) = held.get("acme") else {
            panic!("acme isn't open");
        };
        uploads
            .grant(
                token_id,
                &ledger_store::journal::sha256_hex(bytes),
                bytes.len() as u64,
                "application/pdf",
                "upload.pdf",
                "document",
                chrono::Utc::now().timestamp(),
            )
            .unwrap()
    }

    /// The last access record of `acme`: its action and its outcome.
    fn last_access(books: &Books) -> (String, Outcome) {
        let dir = books.root.book_dir(&BookId::parse("acme").unwrap());
        let (records, _) = ledger_store::access::read(&dir).unwrap();
        let last = records.last().unwrap();
        (last.access.action.clone(), last.access.result.clone())
    }

    #[test]
    fn a_book_that_didnt_open_says_why_and_one_open_says_nothing() {
        let why = "its access log can't be used".to_string();
        assert_eq!(Book::Failed(why.clone()).problem(), Some(why));
        let home = tempfile::tempdir().unwrap();
        let books = books_in(home.path());
        create(&books, "acme");
        assert_eq!(books.held()["acme"].problem(), None);
    }

    /// INVARIANT 6: once closing, every request is refused as closing,
    /// even one for books that didn't open or don't exist (until then
    /// told so), so a client knows the server is going, not that its
    /// books are gone.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn once_closing_every_request_is_refused_as_closing() {
        let home = tempfile::tempdir().unwrap();
        let books = books_in(home.path());
        create(&books, "acme");
        books.hold(
            &BookId::parse("broken").unwrap(),
            Book::Failed("its access log can't be used".into()),
        );
        let asked = Asked::connect(&axum::http::Method::POST);
        let headers = axum::http::HeaderMap::new();
        let refused = |books: Arc<Books>, book: &'static str| {
            let (asked, headers) = (&asked, &headers);
            async move {
                match door(&books, book, asked, headers).await {
                    Ok(_) => panic!("{book}: let through"),
                    Err(r) => answer(*r).await,
                }
            }
        };
        let (status, code, text) = refused(books.clone(), "broken").await;
        assert_eq!((status, code.as_deref()), (503, Some("not-open")), "{text}");
        let (status, code, text) = refused(books.clone(), "nobody").await;
        assert_eq!(
            (status, code.as_deref()),
            (404, Some("no-such-books")),
            "{text}"
        );
        books.closing.cancel();
        for book in ["broken", "nobody"] {
            let (status, code, text) = refused(books.clone(), book).await;
            assert_eq!((status, code), (503, None), "{book}: {text}");
            assert_eq!(text, "the books are closing: this request wasn't run");
        }
        let (status, code, text) = refused(books.clone(), "acme").await;
        assert_eq!(
            (status, code.as_deref()),
            (503, Some(crate::access::CLOSING))
        );
        assert_eq!(text, "the books are closing: this request wasn't run");
    }

    /// INVARIANT 9: nothing comes in once the books stopped (SharedStore
    /// INVARIANT 2): an upload is refused as stopped, and recorded so.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_upload_to_books_that_stopped_is_refused_as_stopped() {
        let home = tempfile::tempdir().unwrap();
        let books = books_in(home.path());
        create(&books, "acme");
        let (id, text) = token(&books, "acme", "agent");
        let bytes = b"%PDF-1.4 stopped".to_vec();
        let g = grant(&books, &id, &bytes);
        match books.held().get("acme") {
            Some(Book::Open { store, .. }) => store.stop(),
            _ => panic!("acme isn't open"),
        }
        let r = send_upload(&books, &g.id, Some(&text), bytes.into()).await;
        let (status, code, said) = answer(r).await;
        assert_eq!((status, code.as_deref()), (503, Some("stopped")), "{said}");
        assert!(said.contains("stopped taking requests"), "{said}");
        assert_eq!(
            last_access(&books),
            ("upload".to_string(), Outcome::Refused("stopped".into()))
        );
    }

    /// INVARIANT 9: an upload is received from one request at a time;
    /// a second sent while the first is still arriving is refused as in
    /// use, and the first is kept.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_upload_still_arriving_is_refused_to_a_second_sender() {
        let home = tempfile::tempdir().unwrap();
        let books = books_in(home.path());
        create(&books, "acme");
        let (id, text) = token(&books, "acme", "agent");
        let bytes: Vec<u8> = b"%PDF-1.4 "
            .iter()
            .copied()
            .chain((0..2_000_000u32).map(|i| (i % 251) as u8))
            .collect();
        let g = grant(&books, &id, &bytes);
        let (tx, body) = fed();
        let first = tokio::spawn({
            let (books, id, text) = (books.clone(), g.id.clone(), text.clone());
            async move { send_upload(&books, &id, Some(&text), body).await }
        });
        tx.send(bytes[..1_000_000].to_vec()).await.unwrap();
        // Arriving: its part is on the disk.
        let part = books
            .root
            .book_dir(&BookId::parse("acme").unwrap())
            .join(ledger_store::uploads::UPLOADS_DIR)
            .join(format!("{}.part", g.id));
        let deadline = Instant::now() + Duration::from_secs(30);
        while !part.exists() {
            assert!(Instant::now() < deadline, "the first upload never began");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let second = send_upload(&books, &g.id, Some(&text), bytes.clone().into()).await;
        let (status, code, said) = answer(second).await;
        assert_eq!((status, code.as_deref()), (409, Some("in_use")), "{said}");
        assert_eq!(
            last_access(&books),
            ("upload".to_string(), Outcome::Refused("in_use".into()))
        );
        tx.send(bytes[1_000_000..].to_vec()).await.unwrap();
        drop(tx);
        let (status, _, said) = answer(first.await.unwrap()).await;
        assert_eq!(status, 200, "{said}");
    }

    /// INVARIANT 9: a refused upload's bytes are read and dropped up to
    /// the most an upload can be, so a client sending no more than that
    /// is never cut off mid-send, and no further, so one that never stops
    /// sending is still answered: refused at the door (no token) and past
    /// it (no such upload) alike.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_refused_upload_is_read_up_to_the_most_an_upload_can_be() {
        let home = tempfile::tempdir().unwrap();
        let books = books_in(home.path());
        create(&books, "acme");
        let (_, text) = token(&books, "acme", "agent");
        let max = ledger_store::uploads::MAX_UPLOAD as usize;
        let mib = 1024 * 1024;
        assert_eq!(max % mib, 0);
        for (bearer, status, code) in [
            (None, 401, crate::access::UNAUTHORIZED),
            (Some(text.clone()), 404, "no_such_upload"),
        ] {
            let (tx, body) = fed();
            let answered = tokio::spawn({
                let books = books.clone();
                async move {
                    let id = "up_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
                    send_upload(&books, id, bearer.as_deref(), body).await
                }
            });
            for _ in 0..max / mib {
                tx.send(vec![b'x'; mib]).await.unwrap();
            }
            // Every byte up to the most taken, and a moment to act on it:
            // still reading.
            let more = tx.reserve().await.unwrap();
            tokio::time::sleep(Duration::from_millis(500)).await;
            assert!(
                !answered.is_finished(),
                "{status}: answered before the most an upload can be was read"
            );
            // One byte past it: answered, though the body never ends.
            more.send(vec![b'x']);
            let r = tokio::time::timeout(Duration::from_secs(60), answered)
                .await
                .unwrap_or_else(|_| panic!("{status}: never answered"))
                .unwrap();
            let (got, refused, said) = answer(r).await;
            assert_eq!((got, refused.as_deref()), (status, Some(code)), "{said}");
            drop(tx);
        }
    }

    /// INVARIANT 8: a token revoked or replaced lets go of the sessions it
    /// opened, and only those: not another book's token of its name, nor
    /// its own book's token of another name.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_replaced_or_revoked_token_lets_go_of_its_own_sessions_alone() {
        let home = tempfile::tempdir().unwrap();
        let books = books_in(home.path());
        create(&books, "acme");
        create(&books, "beta");
        // Oldest first: another book's token of the same name, then one of
        // the same book with another name, then the one replaced.
        let (beta, _) = token(&books, "beta", "agent");
        let (other, _) = token(&books, "acme", "other");
        let (agent, _) = token(&books, "acme", "agent");
        {
            let mut bound = books.session_tokens.lock().unwrap();
            for (sid, owner) in [("s-beta", &beta), ("s-other", &other), ("s-agent", &agent)] {
                bound.insert(sid.into(), owner.clone());
            }
        }
        token(&books, "acme", "agent");
        assert_eq!(bound(&books), ["s-beta", "s-other"]);
        match books.admin(Request::TokenRevoke {
            book: "beta".into(),
            name: "agent".into(),
            id: None,
        }) {
            Response::TokenRevoked(t) => assert_eq!(t.id, beta),
            other => panic!("{other:?}"),
        }
        assert_eq!(bound(&books), ["s-other"]);
    }

    /// An MCP request to `acme` with `bearer`, on `session` if given.
    fn mcp_request(
        method: axum::http::Method,
        bearer: &str,
        session: Option<&str>,
        body: serde_json::Value,
    ) -> axum::extract::Request {
        let mut r = axum::http::Request::builder()
            .method(method)
            .uri("http://localhost/b/acme/mcp")
            .header("host", "localhost")
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .header("authorization", format!("Bearer {bearer}"));
        if let Some(sid) = session {
            r = r
                .header(SESSION_HEADER, sid)
                .header("mcp-protocol-version", "2025-06-18");
        }
        r.body(axum::body::Body::from(body.to_string())).unwrap()
    }

    async fn send_mcp(
        books: &Arc<Books>,
        request: axum::extract::Request,
    ) -> axum::response::Response {
        mcp(
            axum::extract::State(books.clone()),
            axum::extract::Path("acme".to_string()),
            request,
        )
        .await
    }

    fn initialize() -> serde_json::Value {
        serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2025-06-18", "capabilities": {},
            "clientInfo": {"name": "test", "version": "0"}}})
    }

    /// INVARIANT 8: a session stays bound to the token that opened it
    /// through every request it serves, and is forgotten when it ends.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_session_stays_bound_to_its_token_until_it_ends() {
        use axum::http::Method;
        let home = tempfile::tempdir().unwrap();
        let books = books_in(home.path());
        create(&books, "acme");
        let (agent_id, agent) = token(&books, "acme", "agent");
        let (_, other) = token(&books, "acme", "other");
        let r = send_mcp(
            &books,
            mcp_request(Method::POST, &agent, None, initialize()),
        )
        .await;
        let sid = r.headers()[SESSION_HEADER].to_str().unwrap().to_string();
        let (status, _, said) = answer(r).await;
        assert_eq!(status, 200, "{said}");
        let initialized =
            serde_json::json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
        let list = serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"});
        for body in [initialized, list.clone()] {
            let r = send_mcp(&books, mcp_request(Method::POST, &agent, Some(&sid), body)).await;
            let (status, _, said) = answer(r).await;
            assert!((200..300).contains(&status), "{status}: {said}");
            assert_eq!(
                books.session_tokens.lock().unwrap().get(&sid),
                Some(&agent_id)
            );
        }
        let r = send_mcp(&books, mcp_request(Method::POST, &other, Some(&sid), list)).await;
        let (status, code, _) = answer(r).await;
        assert_eq!((status, code.as_deref()), (403, Some("session_token")));
        // Ended: forgotten.
        let r = send_mcp(
            &books,
            mcp_request(Method::DELETE, &agent, Some(&sid), serde_json::json!({})),
        )
        .await;
        let (status, _, said) = answer(r).await;
        assert!((200..300).contains(&status), "{status}: {said}");
        assert!(bound(&books).is_empty());
    }

    /// INVARIANT 8: past MAX_SESSIONS bindings only a request that opens
    /// a session (an initialize) is refused; any other still reaches the
    /// MCP service.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn past_the_most_sessions_only_opening_another_is_refused() {
        use axum::http::Method;
        let home = tempfile::tempdir().unwrap();
        let books = books_in(home.path());
        create(&books, "acme");
        let (agent_id, agent) = token(&books, "acme", "agent");
        {
            let mut bound = books.session_tokens.lock().unwrap();
            for n in 0..MAX_SESSIONS {
                bound.insert(format!("s-{n}"), agent_id.clone());
            }
        }
        let r = send_mcp(
            &books,
            mcp_request(Method::POST, &agent, None, initialize()),
        )
        .await;
        let (status, code, said) = answer(r).await;
        assert_eq!(
            (status, code.as_deref()),
            (503, Some("too_many_sessions")),
            "{said}"
        );
        let list = serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"});
        let r = send_mcp(&books, mcp_request(Method::POST, &agent, None, list)).await;
        let (status, code, said) = answer(r).await;
        assert_ne!(
            code.as_deref(),
            Some("too_many_sessions"),
            "{status}: {said}"
        );
        assert_eq!(bound(&books).len(), MAX_SESSIONS);
    }
}
