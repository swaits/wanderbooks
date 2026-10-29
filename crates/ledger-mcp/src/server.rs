//! The MCP server: narrow typed tools over the store, with the receipt
//! policy enforced server-side.
//!
//! ```text
//! GOAL: let an AI agent keep the books through tools that cannot leave
//!       them wrong, and say exactly what happened.
//! SOURCE: docs/spec/sources/mcp-tools.md (MCP 2025-06-18 tools: structured
//!         results conform to the declared output schema; tool errors are
//!         results with isError; "validate all tool inputs");
//!         docs/spec/sources/jsonrpc-2.0.md (one response per request, id
//!         echoed, result xor error); DECISION (owner, the operating
//!         contract in the connect text and handbook, handbook.rs, and
//!         docs/mcp-interface.md).
//! INVARIANTS:
//!   1. Every tool's structured result MUST conform to its declared output
//!      schema, and the same JSON MUST be in a text block.
//!   2. A refused call (isError) MUST leave the books unchanged: the
//!      journal byte for byte.
//!   3. Amounts in MUST be decimal strings, exactly `-?digits[.digits]`
//!      (no exponent, sign "+", separators, or spaces); amounts out are
//!      decimal strings, never JSON numbers, with at least two places and
//!      never rounded.
//!   4. Posting and reversing MUST preview without `confirmed: true`, and a
//!      preview MUST change nothing.
//!   5. An entry the receipt policy covers MUST NOT post, or keep posted,
//!      without a receipt (crate::policy).
//!   6. A reference (account, vendor, class, party) given as a number
//!      that names one thing by id and another by code or name is
//!      refused, never guessed.
//!   7. The server MUST answer every request with exactly one response
//!      whose id is the request's, and keep serving after any tool error.
//! ```

use crate::handbook;
use crate::policy::ReceiptPolicy;
use crate::settings::{Live, Settings};
use crate::tin_files;
use crate::types::*;
use chrono::NaiveDate;
use ledger_core::{
    AccountId, AccountType, AssetStatus, Currency, DocumentLine, DocumentStatus, JournalEntry,
    Ledger, Money, NewAsset, PeriodStatus, Side, depreciation_schedule, reports,
};
use ledger_import::ReportMeta;
use ledger_package::{csv_escape, form_1099_label, money, parse_date, tin_kind, vault_tin};
use ledger_store::Store;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::{
    Implementation, ListResourcesResult, ReadResourceRequestParams, ReadResourceResponse,
    ReadResourceResult, Resource, ResourceContents, ServerConfig,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::{ServerHandler, tool, tool_handler, tool_router};
use rust_decimal::Decimal;
use std::sync::{Arc, Mutex};

/// The server. One store, one receipt policy, shared across tool calls.
/// `Store` is `!Sync`, so access is serialized through a mutex; the server
/// is single-writer by construction. Tool bodies (synchronous disk and
/// SQLite work) run on tokio's blocking threads, never on the async
/// workers, so a long call can't stall the transport ([`run_blocking`]).
/// Built only from a book as the books server serves it ([`Served`]): the
/// one way books are served (plan D-44).
#[derive(Clone)]
pub struct BooksServer {
    store: Arc<SharedStore>,
    settings: Arc<Mutex<Live>>,
    /// The book's access log: every call is checked against its caller's
    /// token and recorded (access.rs).
    access: Arc<ledger_store::access::AccessLog>,
    /// The book's uploads (`books.begin_upload`).
    uploads: ledger_store::uploads::Uploads,
    /// The files the books made for agents, waiting to be fetched.
    exports: ledger_store::exports::Exports,
    /// The books' backups (backup.rs).
    backups: Arc<crate::backup::Backups>,
    /// How these books are kept from agents (plan 3.8, P9a): what
    /// `books.start` reports.
    isolation: Isolation,
}

/// How the books are kept from agents (plan 3.8, 2.1; D-32, D-51).
///
/// Locally the server runs as the human and the books are hidden, not
/// locked: `SameUser`. On a machine of the owner's the server runs as a
/// dedicated account and agents have no account there: `ServiceUser`.
/// The server never guesses which; `--isolation enforced` says so
/// (`serve --isolation enforced` under the system unit, P9a).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Isolation {
    /// Hidden from the agent in the owner's own account, not locked.
    SameUser,
    /// A separate account on another machine; the agent has none there.
    ServiceUser,
}

impl Isolation {
    /// What `books.start` says (the strings plan 3.8 names).
    pub fn label(self) -> &'static str {
        match self {
            Self::SameUser => "same_user (advisory)",
            Self::ServiceUser => "service_user (enforced)",
        }
    }

    /// What `--isolation` takes: `same_user`, `advisory`, `service_user`
    /// or `enforced`.
    pub fn parse(text: &str) -> Result<Self, String> {
        match text {
            "same_user" | "advisory" => Ok(Self::SameUser),
            "service_user" | "enforced" => Ok(Self::ServiceUser),
            other => Err(format!(
                "--isolation takes same_user or enforced, not {other:?}"
            )),
        }
    }
}

/// One book as the books server serves it (local.rs), shared by every
/// session with it: its one store, the settings in force, its access
/// log, uploads, exports and backups. Each session's [`BooksServer`] is
/// made from it ([`BooksServer::new`]).
#[derive(Clone)]
pub(crate) struct Served {
    pub(crate) store: Arc<SharedStore>,
    pub(crate) settings: Arc<Mutex<Live>>,
    pub(crate) access: Arc<ledger_store::access::AccessLog>,
    pub(crate) uploads: ledger_store::uploads::Uploads,
    pub(crate) exports: ledger_store::exports::Exports,
    pub(crate) backups: Arc<crate::backup::Backups>,
    pub(crate) isolation: Isolation,
}

impl Served {
    /// [`Self::open_with`] for books served the local way
    /// ([`Isolation::SameUser`]): how the server's own tests open them.
    #[cfg(test)]
    pub(crate) fn open(store: Store) -> Result<Self, String> {
        Self::open_with(store, Isolation::SameUser)
    }

    /// What serving the books `store` holds needs, opened beside them:
    /// the settings in force (the books' own), the access log, the uploads
    /// and exports, the viewer's side channel, and the backups. The books
    /// server opens every book this way (local.rs), the only way books are
    /// served (plan D-44), saying how the server runs (plan 3.8, P9a).
    pub(crate) fn open_with(store: Store, isolation: Isolation) -> Result<Self, String> {
        let mut live = Live::own();
        live.sync(store.ledger().settings(), store.base_currency().code());
        let access = Arc::new(
            ledger_store::access::AccessLog::open(store.data_dir())
                .map_err(|e| format!("its access log can't be used: {e}"))?,
        );
        let uploads =
            ledger_store::uploads::Uploads::open(store.data_dir(), chrono::Utc::now().timestamp())
                .map_err(|e| e.to_string())?;
        let exports =
            ledger_store::exports::Exports::open(store.data_dir(), chrono::Utc::now().timestamp())
                .map_err(|e| e.to_string())?;
        // The viewer's side channel, made by the server (ops.rs INVARIANT 2):
        // a viewer can show its presence from the start.
        ledger_store::ops::prepare(store.data_dir()).map_err(|e| e.to_string())?;
        let book_dir = store.data_dir().to_path_buf();
        let book_id = book_dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let store = SharedStore::new(store);
        let backups = Arc::new(crate::backup::Backups::open(
            &book_id,
            &book_dir,
            store.clone(),
        )?);
        Ok(Self {
            store,
            settings: Arc::new(Mutex::new(live)),
            access,
            uploads,
            exports,
            backups,
            isolation,
        })
    }
}

/// The one store every session shares, and whether it may still be used.
///
/// ```text
/// GOAL: a request that dies half way never leaves the server serving
///       books that differ from their journal.
/// SOURCE: DECISION (review of P0, 2026-10-16): fail closed. The journal
///         is appended before memory changes (docs/receipt-store.md), so a
///         panic between the two leaves memory behind the record (a
///         reused next id would be journaled forever).
/// INVARIANTS:
///   1. Once a request panics (its blocking task, or while it holds the
///      store), `stopped` is set and never cleared in this process.
///   2. While `stopped`, every request is refused with STOPPED, before
///      it touches the store; a restart re-verifies the journal.
///   3. Once `close` begins, no request starts running (each is refused
///      with NOT_RUN_CLOSING, untouched), and `close` returns only when
///      every request that did start has finished its blocking work,
///      whether or not anyone still waits for its answer.
/// ```
pub struct SharedStore {
    /// None once let go (`let_go`): its books were replaced by a restore.
    store: tokio::sync::Mutex<Option<Store>>,
    stopped: std::sync::atomic::AtomicBool,
    /// Closing: no new request runs (INVARIANT 3).
    closing: std::sync::atomic::AtomicBool,
    /// Requests running (from before their blocking task to its end).
    running: std::sync::atomic::AtomicUsize,
    /// Woken whenever one ends.
    ended: tokio::sync::Notify,
    /// The books' base currency, fixed at their creation: read without
    /// the lock (the connect text names it).
    currency: Currency,
}

impl SharedStore {
    /// Run `f` on the store under its lock, for a backup (backup.rs):
    /// blocking. It reads only, and runs even while the books close (a
    /// last backup), never after the server stopped.
    pub fn with_store<R>(&self, f: impl FnOnce(&Store) -> R) -> Result<R, String> {
        if self.stopped() {
            return Err(STOPPED.to_string());
        }
        let store = self.store.blocking_lock();
        let store = store.as_ref().ok_or(STOPPED)?;
        Ok(f(store))
    }

    /// Run `f` on the store under its lock, writing: the owner's format
    /// upgrade, after its backup (backup.rs). Blocking. Refused once the
    /// books are closing or the server stopped (INVARIANTS 2, 3), counted
    /// as a request while it runs, and one that panics stops the server
    /// (INVARIANT 1).
    pub fn with_store_mut<R>(
        self: &Arc<Self>,
        f: impl FnOnce(&mut Store) -> R,
    ) -> Result<R, String> {
        if self.stopped() {
            return Err(STOPPED.to_string());
        }
        // Counted before the closing check, as a tool's request is.
        let _running = Running::new(self);
        if self.closing() {
            return Err("the books are closing".into());
        }
        let mut store = self.store.blocking_lock();
        if self.stopped() {
            return Err(STOPPED.to_string());
        }
        let store = store.as_mut().ok_or(STOPPED)?;
        /// Stops the server if `f` panics while it holds the store.
        struct StopOnPanic<'a>(&'a SharedStore);
        impl Drop for StopOnPanic<'_> {
            fn drop(&mut self) {
                if std::thread::panicking() {
                    self.0.stop();
                }
            }
        }
        let _stop = StopOnPanic(self);
        Ok(f(store))
    }

    /// Run `f` on the store if no one holds it now (books just opened):
    /// never waits, so never blocks a runtime's thread.
    pub fn try_with_store<R>(&self, f: impl FnOnce(&Store) -> R) -> Option<R> {
        let store = self.store.try_lock().ok()?;
        store.as_ref().map(f)
    }

    /// An attachment's name, type and bytes (hash-verified), for a
    /// download: blocking (the store's lock, then the file).
    /// None when there is no such attachment.
    pub fn attachment_file(&self, id: u64) -> Result<Option<(String, String, Vec<u8>)>, String> {
        if self.stopped() {
            return Err(STOPPED.to_string());
        }
        let store = self.store.blocking_lock();
        let store = store.as_ref().ok_or(STOPPED)?;
        let Some(meta) = store
            .list_attachments(None)
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|a| a.id == id)
        else {
            return Ok(None);
        };
        let bytes = store.attachment_bytes(id).map_err(|e| e.to_string())?;
        Ok(Some((meta.file_name, meta.mime_type, bytes)))
    }

    /// Whether it stopped taking requests (INVARIANT 2).
    pub fn is_stopped(&self) -> bool {
        self.stopped()
    }

    /// Share an open store.
    pub fn new(store: Store) -> Arc<Self> {
        Arc::new(Self {
            currency: store.base_currency(),
            store: tokio::sync::Mutex::new(Some(store)),
            stopped: std::sync::atomic::AtomicBool::new(false),
            closing: std::sync::atomic::AtomicBool::new(false),
            running: std::sync::atomic::AtomicUsize::new(0),
            ended: tokio::sync::Notify::new(),
        })
    }

    /// Stop new requests from running (INVARIANT 3), and wait until every
    /// one already running has ended: its blocking task finished, even if
    /// its client hung up.
    pub async fn close(&self) {
        self.closing
            .store(true, std::sync::atomic::Ordering::SeqCst);
        loop {
            let ended = self.ended.notified();
            if self.running.load(std::sync::atomic::Ordering::SeqCst) == 0 {
                return;
            }
            ended.await;
        }
    }

    fn closing(&self) -> bool {
        self.closing.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn stopped(&self) -> bool {
        self.stopped.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Let go of the books for good (a restore replaces them): close as
    /// [`Self::close`], stop, and drop the store, so its writer lock is
    /// released while sessions still hold this.
    pub async fn let_go(&self) {
        self.close().await;
        self.stop();
        self.store.lock().await.take();
    }

    /// Stop taking requests (INVARIANT 1): after an internal error, or an
    /// access record that couldn't be written (access.rs INVARIANT 3).
    pub(crate) fn stop(&self) {
        self.stopped
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

/// What every request gets after an internal error (SharedStore
/// INVARIANT 2).
pub(crate) const STOPPED: &str = "the books server stopped taking requests after an internal error, to protect the books: ask the human to restart the books server (the books are checked again when it starts)";

/// The store, locked for one stretch of synchronous work. Not `Send`, so
/// the compiler refuses any tool future that would hold it across an
/// `.await` (tool futures must be `Send`): the lock is never held while a
/// task is suspended. Dropped by a panic, it stops the server
/// (SharedStore INVARIANT 1).
pub struct StoreGuard<'a> {
    /// Holds a store (checked when it was taken).
    guard: tokio::sync::MutexGuard<'a, Option<Store>>,
    shared: &'a SharedStore,
    /// The journal's head when the lock was taken.
    before: (u64, String),
    _not_send: std::marker::PhantomData<*const ()>,
}

impl Drop for StoreGuard<'_> {
    fn drop(&mut self) {
        if std::thread::panicking() {
            self.shared.stop();
        }
        // A write while it was held: the head after it, for the call's
        // access record (taken under the same lock, so it is this call's).
        let Some(store) = self.guard.as_ref() else {
            return;
        };
        let after = store.head();
        if after != self.before {
            WROTE.with(|w| *w.borrow_mut() = Some(after));
        }
    }
}

impl std::ops::Deref for StoreGuard<'_> {
    type Target = Store;
    fn deref(&self) -> &Store {
        self.guard.as_ref().expect("a guard holds a store")
    }
}

impl std::ops::DerefMut for StoreGuard<'_> {
    fn deref_mut(&mut self) -> &mut Store {
        self.guard.as_mut().expect("a guard holds a store")
    }
}

/// Run a request's future to completion on one of tokio's blocking
/// threads (`spawn_blocking`), where the store lock is taken with
/// `blocking_lock`. The future is driven here, by parking this thread
/// until it is woken, not by the runtime: tool bodies are synchronous
/// work, and `blocking_lock` may not be called from the runtime's own
/// `block_on`. A stopped server runs nothing; a request that panics stops
/// it (SharedStore), and gets STOPPED, never the panic's own text.
async fn run_blocking<T, F>(shared: &Arc<SharedStore>, make: F) -> Result<T, rmcp::ErrorData>
where
    T: Send + 'static,
    F: FnOnce() -> std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send>> + Send + 'static,
{
    if shared.stopped() {
        return Err(not_run_stopped());
    }
    // SharedStore INVARIANT 3: counted before the closing check, so a
    // close never misses it; the count rides in the blocking task, which
    // runs to its end even when this future is dropped.
    let running = Running::new(shared);
    if shared.closing() {
        return Err(not_run_closing());
    }
    let joined = tokio::task::spawn_blocking(move || {
        let _running = running;
        struct Unpark(std::thread::Thread);
        impl std::task::Wake for Unpark {
            fn wake(self: Arc<Self>) {
                self.0.unpark();
            }
        }
        let waker = std::task::Waker::from(Arc::new(Unpark(std::thread::current())));
        let mut cx = std::task::Context::from_waker(&waker);
        let mut fut = make();
        loop {
            if let std::task::Poll::Ready(out) = fut.as_mut().poll(&mut cx) {
                return out;
            }
            std::thread::park();
        }
    })
    .await;
    joined.map_err(|e| {
        if e.is_panic() {
            shared.stop();
        }
        rmcp::ErrorData::internal_error(STOPPED, None)
    })
}

/// A request running, counted in its store (SharedStore INVARIANT 3).
struct Running(Arc<SharedStore>);

impl Running {
    fn new(shared: &Arc<SharedStore>) -> Self {
        shared
            .running
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Self(shared.clone())
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.0
            .running
            .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        self.0.ended.notify_waiters();
    }
}

/// The JSON-RPC error code of a request refused because the books are
/// closing: it didn't run, so a client may send it again once they are
/// open (the agents' bridge does).
pub const NOT_RUN_CLOSING_CODE: i32 = -32050;

/// The error code of a request not run because the server had stopped
/// (SharedStore INVARIANT 2), told apart from one that ran and failed.
pub const NOT_RUN_STOPPED_CODE: i32 = -32051;

fn not_run_stopped() -> rmcp::ErrorData {
    rmcp::ErrorData::new(rmcp::model::ErrorCode(NOT_RUN_STOPPED_CODE), STOPPED, None)
}

/// A tool's answer that is a refusal in words.
fn tool_error(text: String) -> rmcp::model::CallToolResponse {
    rmcp::model::CallToolResponse::Complete(rmcp::model::CallToolResult::error(vec![
        rmcp::model::ContentBlock::text(text),
    ]))
}

/// What a request refused while closing is told.
pub const NOT_RUN_CLOSING: &str = "the books are closing: this call wasn't run";

fn not_run_closing() -> rmcp::ErrorData {
    rmcp::ErrorData::new(
        rmcp::model::ErrorCode(NOT_RUN_CLOSING_CODE),
        NOT_RUN_CLOSING,
        None,
    )
}

impl BooksServer {
    /// The settings that name account `code`, by key.
    fn config_mentions(&self, code: &str) -> Vec<String> {
        let mut out = Vec::new();
        let settings = self.settings();
        for (key, codes) in [
            ("books.cash_accounts", &settings.roles.cash),
            ("books.financing_accounts", &settings.roles.financing),
            ("books.investing_accounts", &settings.roles.investing),
            ("books.close_sweep", &settings.company.close_sweep),
            (
                "policy.evidence.required_accounts",
                &settings.policy.receipt_required_accounts,
            ),
            (
                "policy.evidence.exempt_accounts",
                &settings.policy.receipt_exempt_accounts,
            ),
        ] {
            if codes.iter().any(|c| c == code) {
                out.push(key.to_string());
            }
        }
        if settings.company.close_into.as_deref() == Some(code) {
            out.push("books.close_into".to_string());
        }
        let members = "company.members";

        for m in &settings.company.members {
            if m.capital_account == code {
                out.push(format!("{members} {} capital_account", m.name));
            }
            if m.distribution_account.as_deref() == Some(code) {
                out.push(format!("{members} {} distribution_account", m.name));
            }
        }
        out
    }

    /// A session's server for the book `served` serves.
    pub(crate) fn new(served: &Served) -> Self {
        Self {
            store: served.store.clone(),
            settings: served.settings.clone(),
            access: served.access.clone(),
            uploads: served.uploads.clone(),
            exports: served.exports.clone(),
            backups: served.backups.clone(),
            isolation: served.isolation,
        }
    }

    fn live(&self) -> std::sync::MutexGuard<'_, Live> {
        // A panic while holding it can't leave the settings half-written
        // (they're swapped whole), so a poisoned lock is still good.
        self.settings.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Every tool the server serves.
    pub(crate) fn tool_router() -> rmcp::handler::server::router::tool::ToolRouter<Self> {
        Self::kept_router()
            + Self::access_router()
            + Self::files_router()
            + Self::backup_router()
            + Self::instruments_router()
            + Self::projects_router()
            + Self::documents_router()
            + Self::sent_router()
            + Self::settlements_router()
            + Self::entries_router()
            + Self::reports_router()
            + Self::assets_router()
            + Self::schedules_router()
            + Self::periods_router()
            + Self::reconciliation_router()
            + Self::parties_router()
            + Self::attachments_router()
            + Self::evidence_router()
            + Self::settings_router()
            + Self::chart_router()
            + Self::payroll_router()
            + Self::payouts_router()
            + Self::tax_router()
            + Self::write_offs_router()
            + Self::drafts_router()
            + Self::journal_router()
    }

    /// The settings in force for this request: the books' own, taken from
    /// their journal whenever the store is held (settings.rs INVARIANT 1).
    fn settings(&self) -> Arc<Settings> {
        self.live().current()
    }

    /// Whether the books declared the journal feature
    /// `document_line_projects` (the handbook's projects section says how
    /// an invoice's or bill's lines reach a project by it).
    fn document_line_projects(&self) -> bool {
        self.lock().is_ok_and(|s| {
            s.journal_format()
                .has_feature(ledger_store::format::DOCUMENT_LINE_PROJECTS)
        })
    }

    /// Whether the books declared the journal feature `documents_v2` (the
    /// handbook's invoices-and-bills section says what waits for it).
    fn documents_v2(&self) -> bool {
        self.lock().is_ok_and(|s| {
            s.journal_format()
                .has_feature(ledger_store::format::DOCUMENTS_V2)
        })
    }

    /// The books' base currency code, e.g. "USD".
    fn currency(&self) -> &str {
        self.store.currency.code()
    }

    /// What omitted dates mean today, for these books (rules.md B-30): the
    /// one resolver every report tool, `books.show` and the viewer use.
    fn periods(&self) -> ledger_core::calendar::PeriodDefaults {
        ledger_core::calendar::period_defaults(&self.settings().company, today())
    }

    /// A period report's range: the dates as given, each omitted one the
    /// default (rules.md B-30); a start after the end, given or derived,
    /// is refused.
    fn statement_range(
        &self,
        start: Option<&str>,
        end: Option<&str>,
    ) -> Result<(NaiveDate, NaiveDate), String> {
        ledger_package::statement_range(&self.settings().company, start, end, today())
    }

    /// A point-in-time report's day: as given, else today (rules.md B-30).
    fn as_of(&self, as_of: Option<&str>) -> Result<NaiveDate, String> {
        ledger_package::as_of_date(&self.settings().company, as_of, today())
    }

    /// Profit and loss for a range (see [`Self::statement_range`]),
    /// optionally one class.
    fn pnl_statement(
        &self,
        ledger: &Ledger,
        start: Option<&str>,
        end: Option<&str>,
        class_id: Option<ledger_core::ClassId>,
    ) -> Result<ledger_core::ProfitAndLossStatement, String> {
        ledger_package::pnl_statement(
            ledger,
            &self.settings().company,
            start,
            end,
            class_id,
            today(),
        )
    }

    /// Balance sheet as of a day (see [`Self::as_of`]).
    fn bs_statement(
        &self,
        ledger: &Ledger,
        as_of: Option<&str>,
    ) -> Result<ledger_core::BalanceSheetStatement, String> {
        ledger_package::bs_statement(ledger, &self.settings().company, as_of, today())
    }

    /// The cash flow statement from account codes, each list defaulting to
    /// the `[books]` roles when the call passes none.
    fn cash_flow_for(
        &self,
        ledger: &Ledger,
        start: Option<&str>,
        end: Option<&str>,
        cash: &[String],
        financing: &[String],
        investing: &[String],
    ) -> Result<ledger_core::reports::CashFlowStatement, String> {
        ledger_package::cash_flow_for(
            ledger,
            &package_settings(&self.settings()),
            start,
            end,
            cash,
            financing,
            investing,
            today(),
        )
    }

    /// Header and provenance for a rendered report.
    fn report_meta(&self, store: &Store) -> ReportMeta {
        ledger_package::report_meta(&self.settings().company, today(), store.journal_seq())
    }

    /// Members as reports name them (empty unless a multi-member LLC).
    fn report_members(&self) -> Vec<ledger_import::Member> {
        ledger_package::report_members(&self.settings().company)
    }

    /// Members with their capital accounts resolved in the chart, for the
    /// statement of members' equity.
    fn member_capitals(&self, ledger: &Ledger) -> Result<Vec<ledger_core::MemberCapital>, String> {
        if self.settings().company.members.is_empty() {
            return Err(
                "no members configured: set company.members (name, share, capital_account for each)"
                    .to_string(),
            );
        }
        ledger_package::member_capitals(ledger, &self.settings().company.members)
    }

    /// Where a period close goes, from the settings: members' capital
    /// accounts by share (with their distribution accounts swept in) for a
    /// partnership; else `close_into` (Retained Earnings when unset or
    /// `3999`) with `close_sweep` zeroed into it.
    fn close_target(&self, ledger: &Ledger) -> Result<ledger_core::CloseTo, String> {
        let code = |c: &str| -> Result<AccountId, String> {
            ledger
                .chart()
                .find_by_code(c)
                .map(|a| a.id)
                .ok_or_else(|| format!("the settings name unknown account {c}"))
        };
        if !self.settings().company.members.is_empty() {
            return Ok(ledger_core::CloseTo::Members(
                self.member_capitals(ledger)?
                    .into_iter()
                    .map(|m| ledger_core::MemberClose {
                        capital: m.capital_account,
                        share: m.share,
                        distributions: m.distribution_account,
                    })
                    .collect(),
            ));
        }
        let sweep = self
            .settings()
            .company
            .close_sweep
            .iter()
            .map(|c| code(c))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(match self.settings().company.close_into.as_deref() {
            None | Some(ledger_core::RETAINED_EARNINGS_CODE) => {
                ledger_core::CloseTo::RetainedEarnings { sweep }
            }
            Some(c) => ledger_core::CloseTo::Account {
                into: code(c)?,
                sweep,
            },
        })
    }

    /// The fiscal year's earnings through `as_of`, for the balance sheet:
    /// net income (loss) year to date (closing entries excluded), and the
    /// part of it not yet closed into equity: the income after the last
    /// close (closes are cumulative), from no earlier than the year's start.
    fn year_earnings(
        &self,
        ledger: &Ledger,
        as_of: NaiveDate,
    ) -> Result<ledger_import::YearEarnings, String> {
        ledger_package::year_earnings(ledger, &self.settings().company, as_of)
    }

    /// Cash and card accounts due for reconciliation
    /// (ledger_store::reconcilable, the viewer's Needs-attention rule):
    /// the cash accounts (`[books] cash_accounts`, else assets named for
    /// cash), the card accounts (ledger_core::parties::card_accounts: the
    /// business-card payment methods' liabilities, else liabilities named
    /// "card", not "gift", before their first statement), and every other
    /// liability with a recorded
    /// statement, with posted activity through the end of last month, not
    /// reconciled through it.
    ///
    /// ```text
    /// GOAL: the monthly close's reconciliation step can't be forgotten,
    ///       and the digest and the viewer agree on what is due.
    /// SOURCE: docs/parties-and-attachments.md "Reconciliations" (Needs
    ///         attention: an account with activity not reconciled through
    ///         last month's end).
    /// ```
    fn unreconciled_accounts(
        &self,
        store: &Store,
        today: NaiveDate,
    ) -> Result<Vec<UnreconciledAccountOut>, String> {
        use ledger_store::reconcilable;
        let ledger = store.ledger();
        let cash = reconcilable::cash_accounts(ledger, &self.settings().roles.cash);
        let statements = store.statements(None).map_err(|e| e.to_string())?;
        let reconciliations = store.reconciliations(None).map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for id in reconcilable::reconcilable_accounts(ledger, &cash, &statements) {
            let Some(due) = reconcilable::reconciliation_due(ledger, &reconciliations, id, today)
            else {
                continue;
            };
            let (code, name) = code_name(store, id);
            out.push(UnreconciledAccountOut {
                next_step: format!(
                    "file the {code} statement through {due} (books.file_document category statement), books.record_statement, mark the lines it shows (books.mark_cleared), then books.reconcile with statement_attachment and confirmed: true"
                ),
                code,
                name,
                reconciled_through: reconcilable::reconciled_through(&reconciliations, id)
                    .map(|d| d.to_string()),
                due_through: due.to_string(),
            });
        }
        Ok(out)
    }

    /// The TIN vault's directory: `vault/` in the book's own folder in
    /// the data root.
    fn vault_dir(&self, store: &Store) -> std::path::PathBuf {
        store.data_dir().join(ledger_store::root::VAULT_DIR)
    }

    /// The vault's key file: the book's own `keys/legacy.key`.
    fn key_file(&self, store: &Store) -> std::path::PathBuf {
        store
            .data_dir()
            .join(ledger_store::root::KEYS_DIR)
            .join(ledger_store::root::LEGACY_KEY_FILE)
    }

    /// Refuse bytes on their way into the books (books.attach_receipt,
    /// books.file_document, books.set_w9's file) if they carry a full
    /// TIN: a file the server read or wrote with TINs, or text holding one
    /// the vault holds or one printed as an SSN or EIN is (tin_files.rs
    /// INVARIANT 3), or text probing for one (INVARIANT 4).
    fn refuse_tin_file(
        &self,
        store: &Store,
        bytes: &[u8],
        mime_type: &str,
        file_name: &str,
    ) -> Result<(), String> {
        tin_files::refuse(
            &self.vault_dir(store),
            bytes,
            mime_type,
            file_name,
            || known_tin_last4(store),
            || self.vault_tins(store),
            self.company_ein(),
        )
    }

    /// The company's own EIN, as its settings hold it, as nine digits:
    /// printed 2-7 in a filed document, it is no person's TIN
    /// (tin_files.rs INVARIANT 3b).
    fn company_ein(&self) -> Option<u32> {
        let settings = self.settings();
        let (_, ein) = settings.company.profile.iter().find(|(k, _)| k == "ein")?;
        let digits: String = ein.chars().filter(char::is_ascii_digit).collect();
        (digits.len() == 9).then(|| digits.parse().ok()).flatten()
    }

    /// Every full TIN the vault holds for a party of the books, for the
    /// filing scan only (tin_files.rs INVARIANT 3a): never returned or
    /// logged. None when the vault can't be opened (its key missing or
    /// bad), and none for a record that can't be read: the scan then
    /// falls back to the printed shapes.
    fn vault_tins(&self, store: &Store) -> Vec<ledger_vault::Tin> {
        let Ok(vault) = self.open_vault(store, false) else {
            return Vec::new();
        };
        store
            .ledger()
            .parties()
            .filter_map(|p| vault.tin(p.id).ok().flatten())
            .collect()
    }

    /// The signed W-9 a call gives (its upload, `upload_id`), for the vault:
    /// a PDF, PNG, or JPEG by its own first bytes, whatever it is called.
    fn w9_file(
        &self,
        p: &SetW9Params,
        upload_id: &str,
    ) -> Result<(ledger_vault::W9File, Option<ledger_store::uploads::Claim>), String> {
        let file = self.take_file(files::Given {
            upload_id,
            purposes: files::W9,
        })?;
        // Not recorded here: set_w9 records it once the call can only
        // succeed, just before the vault write (tin_files.rs INVARIANT 1),
        // so a refused call never marks an ordinary file.
        let mime_type = if file.bytes.starts_with(b"%PDF-") {
            "application/pdf"
        } else if file.bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            "image/png"
        } else if file.bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
            "image/jpeg"
        } else {
            return Err("the W-9 is a PDF, PNG, or JPEG".into());
        };
        let file_name = p.w9_file_name.clone().unwrap_or(file.file_name);
        if let Some(why) = ledger_store::uploads::file_name_problem(&file_name) {
            return Err(why);
        }
        Ok((
            ledger_vault::W9File {
                file_name,
                mime_type: mime_type.to_string(),
                bytes: file.bytes,
            },
            file.claim,
        ))
    }

    /// A CSV's text, from its upload (books.begin_upload purpose import),
    /// and the upload's hold.
    fn csv_given(
        &self,
        upload_id: &str,
    ) -> Result<(String, Option<ledger_store::uploads::Claim>), String> {
        let file = self.take_file(files::Given {
            upload_id,
            purposes: files::IMPORTED,
        })?;
        let text =
            String::from_utf8(file.bytes).map_err(|_| "the CSV isn't UTF-8 text".to_string())?;
        Ok((text, file.claim))
    }

    /// Record a file read or written with full TINs (tin_files.rs
    /// INVARIANT 2), before it is written out: every session, and every
    /// later server for these books, refuses it.
    fn wrote_tins(&self, store: &Store, bytes: &[u8]) -> Result<(), String> {
        tin_files::record(&self.tin_record_dir(store)?, bytes)
    }

    /// Where records of files with TINs go: the vault's folder, made
    /// private first, as the vault itself makes it.
    fn tin_record_dir(&self, store: &Store) -> Result<std::path::PathBuf, String> {
        ledger_store::root::private_book_folder(store.data_dir(), ledger_store::root::VAULT_DIR)
            .map_err(|e| format!("the TIN vault can't be written: {e}"))?;
        Ok(self.vault_dir(store))
    }

    /// Open the TIN vault with the book's own key file, creating the key
    /// only when `create` (storing a TIN); reading never creates one.
    fn open_vault(&self, store: &Store, create: bool) -> Result<ledger_vault::Vault, String> {
        let key = self.key_file(store);
        // Its folder and the vault's are made private when a TIN is first
        // stored.
        if create {
            for folder in [ledger_store::root::KEYS_DIR, ledger_store::root::VAULT_DIR] {
                ledger_store::root::private_book_folder(store.data_dir(), folder)
                    .map_err(|e| format!("the TIN vault can't store anything yet: {e}"))?;
            }
        }
        // No message names the key file or the books' folders.
        ledger_vault::Vault::open(&self.vault_dir(store), &key, create).map_err(|e| match e {
            ledger_vault::Error::NoKey(_) => "the TIN vault is locked: its key file is missing (restore it from the human's password manager)".to_string(),
            ledger_vault::Error::BadKey(_) => "the TIN vault's key file is not a vault key (64 hex digits): restore it from the human's password manager".to_string(),
            ledger_vault::Error::Io(e) => format!("the TIN vault can't be read: {}", e.kind()),
            other => other.to_string(),
        })
    }

    /// Lock the store. Called only on a blocking thread (every request
    /// runs through [`run_blocking`]); `blocking_lock` panics on an async
    /// worker.
    fn lock(&self) -> Result<StoreGuard<'_>, String> {
        if self.store.stopped() {
            return Err(STOPPED.to_string());
        }
        let guard = self.store.store.blocking_lock();
        // A request that panicked while this one waited stopped it; books
        // let go hold no store.
        if self.store.stopped() {
            return Err(STOPPED.to_string());
        }
        let Some(store) = guard.as_ref() else {
            return Err(STOPPED.to_string());
        };
        // Books that keep their own settings: take any change (crate::
        // settings INVARIANT 1).
        let timezone = {
            let mut live = self.live();
            live.sync(store.ledger().settings(), self.store.currency.code());
            live.current().timezone.clone()
        };
        // "Today" from here on is the zone in force now (a change another
        // session made while this request waited included).
        TIMEZONE.with(|z| *z.borrow_mut() = timezone);
        let before = store.head();
        Ok(StoreGuard {
            guard,
            shared: &self.store,
            before,
            _not_send: std::marker::PhantomData,
        })
    }

    /// Fail with the structured `receipt_required` error when the receipt
    /// policy requires this posted entry to keep at least one receipt.
    /// Drafts pass (posting enforces the policy later); reversals pass (they
    /// are exempt: the original carries the evidence).
    fn ensure_receipt_not_required(&self, store: &Store, entry_id: u64) -> Result<(), String> {
        let entry = store
            .ledger()
            .get_entry(entry_id)
            .ok_or_else(|| format!("unknown entry {entry_id}"))?;
        if entry.status() != ledger_core::EntryStatus::Posted || entry.reversal_of().is_some() {
            return Ok(());
        }
        let split_group = ledger_core::evidence::Policy::from_settings(store.ledger().settings())
            .split_group(store.ledger(), entry_id, entry);
        if !ledger_core::evidence::requirement(store.ledger(), entry, store.ledger().settings())
            .is_required()
            && split_group.is_empty()
        {
            return Ok(());
        }
        // A draft's debits may be past exact arithmetic (rules.md B-28):
        // then the refusal says so in place of the sum.
        let (total, debits) = match ReceiptPolicy::entry_debits(entry) {
            Ok(t) => (money(t), t.to_string()),
            Err(e) => (e.to_string(), e.to_string()),
        };
        Err(ReceiptRequiredError {
            code: "receipt_required".to_string(),
            message: format!(
                "entry {entry_id} moves {total} in debits ({}); unlinking its last receipt would leave it without one.",
                self.settings().policy.describe()
            ),
            entry_id: Some(entry_id),
            asset_id: None,
            debits,
            policy: self.settings().policy.describe(),
            remedy_tool: "books.attach_receipt".to_string(),
            remedy_hint: format!(
                "attach the replacement receipt to entry {entry_id} with books.attach_receipt first, then retry books.unlink_receipt"
            ),
            split_group,
        }
        .to_tool_error())
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// How the viewer reads `report`'s dates with these options, or why it
/// would ignore one of them (`books.show` refuses what its target
/// ignores). Splits are the P&L's, the balance sheet's and the cash
/// flow's; comparisons theirs and the tax summary's, in one total column;
/// a balance sheet in one total column is as of its end, unless compared
/// with the period before (which starts the day before start).
fn show_report_dates(
    report: &str,
    columns: Option<&str>,
    compare: Option<&str>,
) -> Result<ShownDates, String> {
    let split = columns.is_some_and(|c| c != "total");
    if columns.is_some() && !matches!(report, "profit_and_loss" | "balance_sheet" | "cash_flow") {
        return Err(format!(
            "columns is for profit_and_loss, balance_sheet and cash_flow, not {report}"
        ));
    }
    if compare.is_some() {
        if !matches!(
            report,
            "profit_and_loss" | "balance_sheet" | "cash_flow" | "tax_summary"
        ) {
            return Err(format!(
                "compare is for profit_and_loss, balance_sheet, cash_flow and tax_summary, not {report}"
            ));
        }
        if split {
            return Err(
                "compare is for a report in one total column: a report split into columns shows no comparison"
                    .into(),
            );
        }
    }
    Ok(match report {
        "trial_balance" | "ar_aging" | "ap_aging" | "fixed_asset_register" => ShownDates::AsOf,
        "vendor_1099" => ShownDates::Year,
        "balance_sheet" if !split && compare != Some("prior_period") => ShownDates::AsOf,
        _ => ShownDates::Period,
    })
}

/// "invoice:7" or "bill:12" as a document.
fn parse_doc_ref(r: &str) -> Result<ledger_core::DocRef, String> {
    let bad = || format!("bad ref {r:?}: expected invoice:N or bill:N (from books.documents)");
    let (kind, id) = r.trim().split_once(':').ok_or_else(bad)?;
    let id = id.trim().parse::<u64>().map_err(|_| bad())?;
    doc_ref(kind, id).map_err(|_| bad())
}

/// Resolve a vendor by id or name (case-insensitive); a number that is
/// one vendor's id and another's name is refused (INVARIANT 6).
fn resolve_vendor(store: &Store, vendor: &str) -> Result<u64, String> {
    let by_id = vendor
        .parse::<u64>()
        .ok()
        .filter(|id| store.ledger().get_vendor(*id).is_some());
    let matches: Vec<u64> = store
        .ledger()
        .list_vendors()
        .into_iter()
        .filter(|v| v.name.eq_ignore_ascii_case(vendor.trim()))
        .map(|v| v.id)
        .collect();
    if let Some(id) = by_id {
        return match matches.as_slice() {
            [] => Ok(id),
            [m] if *m == id => Ok(id),
            _ => Err(format!(
                "vendor {vendor:?} is ambiguous: it is vendor {id}'s id and another vendor's name; pass the id"
            )),
        };
    }
    match matches.as_slice() {
        [id] => Ok(*id),
        [] => Err(format!(
            "unknown vendor {vendor:?}: add it with books.add_vendor"
        )),
        _ => Err(format!("several vendors are named {vendor:?}: pass the id")),
    }
}

/// Mask any full TIN in a refusal (`ledger_core::parties::redact_full_tins`):
/// a refusal that quotes its input (an unknown party, a bad amount, a
/// parameter of the wrong type) never echoes a TIN the agent put in the
/// wrong field. No tool output, error, or log holds a full TIN
/// (docs/spec/rules.md B-26); a successful result is left alone, since
/// the one that may (`books.vendor_1099` with `include_tins`) is one.
fn redact_refusal(
    out: Result<rmcp::model::CallToolResponse, rmcp::ErrorData>,
) -> Result<rmcp::model::CallToolResponse, rmcp::ErrorData> {
    use ledger_core::parties::{has_full_tin, redact_full_tins};
    match out {
        Err(e) => Err(redact_error(e)),
        Ok(rmcp::model::CallToolResponse::Complete(mut r)) if r.is_error == Some(true) => {
            for c in &mut r.content {
                if let rmcp::model::ContentBlock::Text(t) = c
                    && has_full_tin(&t.text)
                {
                    t.text = redact_full_tins(&t.text).into_owned();
                }
            }
            if let Some(v) = r.structured_content.as_mut() {
                redact_value(v);
            }
            Ok(rmcp::model::CallToolResponse::Complete(r))
        }
        other => other,
    }
}

/// An error with any full TIN in its message or data masked (B-26): a
/// tool's, or a resource read's, which quotes the URI it was given.
fn redact_error(mut e: rmcp::ErrorData) -> rmcp::ErrorData {
    use ledger_core::parties::{has_full_tin, redact_full_tins};
    if has_full_tin(&e.message) {
        e.message = redact_full_tins(&e.message).into_owned().into();
    }
    if let Some(d) = e.data.as_mut() {
        redact_value(d);
    }
    e
}

/// Every string in `v` with any full TIN masked.
fn redact_value(v: &mut serde_json::Value) {
    use ledger_core::parties::{has_full_tin, redact_full_tins};
    use serde_json::Value;
    match v {
        Value::String(t) if has_full_tin(t) => *t = redact_full_tins(t).into_owned(),
        Value::Array(a) => a.iter_mut().for_each(redact_value),
        Value::Object(o) => o.values_mut().for_each(redact_value),
        _ => {}
    }
}

/// The resources the server lists (resources/list): fixed reports, the
/// digest, and the handbook, by section.
fn resource_list() -> Vec<Resource> {
    let mk = |uri: &str, name: &str, title: &str, description: &str| {
        let mut r = Resource::new(uri, name);
        r.title = Some(title.to_string());
        r.description = Some(description.to_string());
        r.mime_type = Some("application/json".to_string());
        r
    };
    let resources = vec![
        mk(
            "books://start",
            "start",
            "Start here",
            "The books in brief: rules in force, settings, conventions, open items, next steps (books.start).",
        ),
        mk(
            "books://settings",
            "settings",
            "Settings",
            "Every setting the books know, its value and who set it (books.settings).",
        ),
        mk(
            "books://conventions",
            "conventions",
            "Conventions",
            "How this company keeps its books: templates, rules, policies, close steps (books.conventions).",
        ),
        mk(
            "books://chart",
            "chart",
            "Chart of accounts",
            "All accounts with codes, types, and hierarchy.",
        ),
        mk(
            "books://reports/trial-balance",
            "trial-balance",
            "Trial balance",
            "The trial balance as of today (debits equal credits): books.trial_balance with its as_of omitted.",
        ),
        mk(
            "books://reports/balance-sheet",
            "balance-sheet",
            "Balance sheet",
            "Assets, liabilities, equity, as of today: books.balance_sheet with its as_of omitted.",
        ),
        mk(
            "books://reports/profit-and-loss",
            "profit-and-loss",
            "Profit and loss",
            "Revenue, expenses, net income for the fiscal year to date (company.fiscal_year_start through today): books.profit_and_loss with its dates omitted.",
        ),
        mk(
            "books://digest",
            "digest",
            "Activity digest",
            "What changed in the books, since the start of this month or ?since=YYYY-MM-DD.",
        ),
        mk(
            handbook::URI,
            "handbook",
            "Handbook",
            "The bookkeeping guide: its sections, each at books://handbook/{id}.",
        ),
    ];
    let mut resources = resources;
    for (id, md) in handbook::SECTIONS {
        let mut r = mk(
            &format!("{}/{id}", handbook::URI),
            &format!("handbook-{id}"),
            handbook::title(md),
            "A section of the bookkeeping guide.",
        );
        r.mime_type = Some("text/markdown".to_string());
        resources.push(r);
    }
    resources
}

/// The advertised resource templates (plan 4.5; RFC 6570): the URIs a
/// client can build, each read by `resource_body`.
fn resource_templates() -> Vec<rmcp::model::ResourceTemplate> {
    use rmcp::model::ResourceTemplate;
    let mk = |uri: &str, name: &str, title: &str, description: &str| {
        ResourceTemplate::new(uri, name)
            .with_title(title)
            .with_description(description)
            .with_mime_type("application/json")
    };
    vec![
        mk(
            "books://entries/{id}",
            "entry",
            "An entry",
            "One entry with its lines, receipts and history (books.get_entry).",
        ),
        mk(
            "books://attachments/{id}",
            "attachment",
            "A stored file",
            "A receipt or document: its metadata and the start of its text (books.get_receipt); its pages are books://attachments/{id}/page/{n}.",
        ),
        mk(
            "books://journal/{seq}",
            "journal-line",
            "A journal line",
            "One journal line in words (books.journal_history).",
        ),
        mk(
            "books://reports/{kind}{?start,end,as_of}",
            "report",
            "A report",
            "trial-balance, balance-sheet or profit-and-loss, for a day (as_of) or a range (start, end); without them as of today, or the year to date.",
        ),
        mk(
            "books://digest{?since,since_seq,as_of}",
            "digest",
            "The activity digest",
            "What changed, since a date, since a journal line (since_seq), as of a day; bare, this month so far.",
        ),
    ]
}

/// Prompts are listed and read by a token that may read the books.
fn prompt_reader(context: &RequestContext<RoleServer>) -> Result<(), rmcp::ErrorData> {
    match crate::access::caller(context) {
        None => Err(rmcp::ErrorData::invalid_request(
            crate::access::UNAUTHORIZED,
            None,
        )),
        Some(c) if !c.may(ledger_store::auth::Scope::Read) => Err(rmcp::ErrorData::invalid_request(
            crate::access::INSUFFICIENT_SCOPE,
            None,
        )),
        Some(_) => Ok(()),
    }
}

/// The subject kinds, for messages.
const SUBJECT_KINDS: &str = "entry, account, asset, party, project, invoice, bill, period, payroll_run, schedule, reconciliation, company";

/// Preview lines with resulting balances for raw (account, side, amount) lines.
fn preview_lines_for(
    store: &Store,
    lines: &[(AccountId, Side, Money)],
) -> Result<Vec<PreviewLine>, String> {
    let ledger = store.ledger();
    let mut out = Vec::with_capacity(lines.len());
    for (account_id, side, amount) in lines {
        let account = ledger
            .chart()
            .get(*account_id)
            .ok_or_else(|| format!("unknown account {account_id}"))?;
        let before = ledger
            .account_balance(*account_id)
            .map_err(|e| e.to_string())?;
        let normal = account.account_type.normal_side();
        let delta = if *side == normal {
            amount.amount
        } else {
            -amount.amount
        };
        let after = before + delta;
        out.push(PreviewLine {
            account_id: *account_id,
            code: account.code.clone(),
            name: account.name.clone(),
            side: match side {
                Side::Debit => "debit".to_string(),
                Side::Credit => "credit".to_string(),
            },
            amount: money(amount.amount),
            balance_before: money(if before.is_zero() {
                Decimal::ZERO
            } else {
                before
            }),
            balance_after: money(if after.is_zero() {
                Decimal::ZERO
            } else {
                after
            }),
        });
    }
    Ok(out)
}

thread_local! {
    /// The token of the request running on this thread (books served from
    /// the data root), its id and name, for what is bound to it (its
    /// uploads) and the commands it is given.
    static CALLER: std::cell::RefCell<Option<(String, String)>> = const { std::cell::RefCell::new(None) };
    /// The journal's head after the last write made by the request
    /// running on this thread (StoreGuard's drop), for its access record.
    static WROTE: std::cell::RefCell<Option<(u64, String)>> = const { std::cell::RefCell::new(None) };
    /// Who the request running on this thread is, as `books.start` shows
    /// it: `{principal, kind, scopes, client_id}`. The token's own facts
    /// and the client's name; set at the start of the request.
    static YOU: std::cell::RefCell<Option<serde_json::Value>> = const { std::cell::RefCell::new(None) };
    /// What the agent said about itself in a `books.start` this request is
    /// running (`agent`), for the call's access record.
    static AGENT_NOTE: std::cell::RefCell<Option<ledger_store::access::Agent>> = const { std::cell::RefCell::new(None) };
    /// The company's time zone for the request running on this thread
    /// (set at its start from the settings in force: one version per
    /// request), so every "today" in it agrees.
    static TIMEZONE: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
    /// The arguments the request running on this thread was called with,
    /// as the caller sent them: what the preview digest (plan P4 step 4,
    /// D-50) is taken over, so a changed argument is a changed preview.
    static ARGS: std::cell::RefCell<Option<serde_json::Map<String, serde_json::Value>>> = const { std::cell::RefCell::new(None) };
}

/// Poll `fut` with "today" in the request's zone on this thread, each
/// time it runs: the request is driven on its own thread (run_blocking),
/// so the zone is the request's alone. It starts as `timezone`; when the
/// request locks the books the zone in force then replaces it (lock), and
/// that carries over to the request's later polls.
async fn in_timezone_async<T>(
    timezone: Option<String>,
    fut: impl std::future::Future<Output = T>,
) -> T {
    let mut fut = std::pin::pin!(fut);
    let mut zone = timezone;
    std::future::poll_fn(move |cx| {
        let before = TIMEZONE.with(|z| z.replace(zone.take()));
        let polled = fut.as_mut().poll(cx);
        zone = TIMEZONE.with(|z| z.replace(before));
        polled
    })
    .await
}

/// Today in the company's time zone (`company.timezone`, for the request
/// running), else the computer's. Not UTC: in the evening (US time) UTC
/// is already tomorrow, and "through today" must not include it.
fn today() -> NaiveDate {
    TIMEZONE.with(|z| ledger_core::settings::today_in(z.borrow().as_deref()))
}

/// A resource URI's query parameter `key`, when given and not empty.
fn query_param(query: &str, key: &str) -> Option<String> {
    query
        .split('&')
        .find_map(|kv| kv.strip_prefix(&format!("{key}=")))
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

/// The `since` of a `books://digest` read: the query's `since=`, or, bare
/// or with an empty `since=`, the first day of `today`'s month (this
/// month so far).
fn digest_since(query: &str, today: NaiveDate) -> String {
    query
        .split('&')
        .find_map(|kv| kv.strip_prefix("since="))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            chrono::Datelike::with_day(&today, 1)
                .unwrap_or(today)
                .to_string()
        })
}

/// How the viewer reads a report's dates (ledger-gui pages/reports.rs):
/// over a period, as of its end, or for its end's calendar year.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShownDates {
    /// From start through end.
    Period,
    /// As of end.
    AsOf,
    /// For end's calendar year.
    Year,
}

// ---------------------------------------------------------------------------
// Tools
// ---------------------------------------------------------------------------

#[path = "guards.rs"]
mod guards;
#[path = "next_steps.rs"]
mod next_steps;
#[path = "tools/access.rs"]
mod access_tools;
#[path = "tools/backup.rs"]
mod backup_tools;
#[path = "tools/files.rs"]
mod files;
#[path = "tools/instruments.rs"]
mod instruments;
#[path = "tools/journal.rs"]
mod journal_tools;
#[path = "tools/prompts.rs"]
mod prompts;
#[path = "tools/kept.rs"]
mod kept;
#[path = "tools/projects.rs"]
mod projects;
pub use kept::SettingKeyOut;

// The tool groups: one file each (plan/served.md 8.2 U0a). A unit adds a
// handler only to its own group's file; the shared helpers are in
// tools/helpers.rs, re-exported below.
#[path = "tools/assets.rs"]
mod assets;
#[path = "tools/attachments.rs"]
mod attachments;
#[path = "tools/chart.rs"]
mod chart;
#[path = "tools/documents.rs"]
mod documents;
#[path = "tools/drafts.rs"]
mod drafts;
#[path = "tools/entries.rs"]
mod entries;
#[path = "tools/evidence.rs"]
mod evidence;
#[path = "tools/helpers.rs"]
mod helpers;
#[path = "tools/parties.rs"]
mod parties;
#[path = "tools/payouts.rs"]
mod payouts;
#[path = "tools/payroll.rs"]
mod payroll;
#[path = "tools/periods.rs"]
mod periods;
#[path = "tools/reconciliation.rs"]
mod reconciliation;
#[path = "tools/reports.rs"]
mod reports_tools;
#[path = "tools/schedules.rs"]
mod schedules;
#[path = "tools/sent.rs"]
mod sent;
#[path = "tools/settings.rs"]
mod settings_tools;
#[path = "tools/settlements.rs"]
mod settlements;
#[path = "tools/tax.rs"]
mod tax;
#[path = "tools/write_offs.rs"]
mod write_offs;

// The shared helpers are re-exported so every group file reaches them
// through `use super::*`, and no group file reaches into another.
pub(crate) use helpers::*;

// ---------------------------------------------------------------------------
// ServerHandler
// ---------------------------------------------------------------------------

/// What a resource read gives: JSON/Markdown text, or a file's bytes (a
/// receipt's page picture, plan P6).
enum ResourceBody {
    /// Text the agent reads.
    Text(String),
    /// Bytes, base64 in the MCP answer.
    Blob { mime: String, bytes: Vec<u8> },
}

impl BooksServer {
    /// Render a resource URI to what it gives. Used by `read_resource`.
    async fn resource_body(&self, uri: &str) -> Result<ResourceBody, String> {
        // Split off the query string for URIs like books://digest?since=...
        let (path, query) = match uri.split_once('?') {
            Some((p, q)) => (p, q),
            None => (uri, ""),
        };
        fn json<T: serde::Serialize>(v: &T) -> Result<ResourceBody, String> {
            serde_json::to_string_pretty(v)
                .map(ResourceBody::Text)
                .map_err(|e| e.to_string())
        }
        match path {
            "books://start" => {
                let Json(out) = self.start(Parameters(kept::StartParams::default())).await?;
                json(&out)
            }
            "books://settings" => {
                let Json(out) = self.settings_tool().await?;
                json(&out)
            }
            "books://conventions" => {
                let Json(out) = self
                    .conventions(Parameters(kept::ConventionsParams::default()))
                    .await?;
                json(&out)
            }
            "books://chart" => {
                let Json(out) = self.chart().await?;
                json(&out)
            }
            "books://reports/trial-balance" => {
                let Json(out) = self
                    .trial_balance(Parameters(TrialBalanceParams {
                        as_of: query_param(query, "as_of"),
                        ..TrialBalanceParams::default()
                    }))
                    .await?;
                json(&out)
            }
            "books://reports/balance-sheet" => {
                let Json(out) = self
                    .balance_sheet(Parameters(BalanceSheetParams {
                        as_of: query_param(query, "as_of"),
                        strict: None,
                    }))
                    .await?;
                json(&out)
            }
            "books://reports/profit-and-loss" => {
                let Json(out) = self
                    .profit_and_loss(Parameters(ProfitAndLossParams {
                        start: query_param(query, "start"),
                        end: query_param(query, "end"),
                        class: None,
                        strict: None,
                    }))
                    .await?;
                json(&out)
            }
            "books://digest" => {
                // `since_seq` (what was posted since a journal line) wins
                // over `since`; with neither, this month so far.
                let since_seq = query_param(query, "since_seq")
                    .map(|n| n.parse::<u64>().map_err(|_| format!("bad since_seq in {uri}")))
                    .transpose()?;
                let since = if since_seq.is_some() {
                    None
                } else {
                    Some(digest_since(query, today()))
                };
                let Json(out) = self
                    .digest(Parameters(DigestParams {
                        since,
                        since_seq,
                        as_of: query_param(query, "as_of"),
                    }))
                    .await?;
                json(&out)
            }
            _ if path.starts_with("books://entries/") => {
                let id: u64 = path["books://entries/".len()..]
                    .parse()
                    .map_err(|_| format!("bad entry resource URI: {uri}"))?;
                let Json(out) = self
                    .get_entry(Parameters(GetEntryParams { entry_id: id }))
                    .await?;
                json(&out)
            }
            _ if path.starts_with("books://journal/") => {
                let seq: u64 = path["books://journal/".len()..]
                    .parse()
                    .map_err(|_| format!("bad journal resource URI: {uri}"))?;
                if seq == 0 {
                    return Err("journal seqs start at 1".into());
                }
                let Json(out) = self
                    .journal_history(Parameters(journal_tools::JournalHistoryParams {
                        after_seq: Some(seq - 1),
                        limit: Some(1),
                        ..Default::default()
                    }))
                    .await?;
                if out.lines.first().is_none_or(|l| l.seq != seq) {
                    return Err(format!("the journal has no line {seq}"));
                }
                json(&out)
            }
            handbook::URI => {
                let Json(out) = self.handbook(Parameters(HandbookParams::default())).await?;
                json(&out)
            }
            _ if path.starts_with(&format!("{}/", handbook::URI)) => {
                let id = &path[handbook::URI.len() + 1..];
                handbook::section_with(
                    id,
                    &self.settings(),
                    self.currency(),
                    self.document_line_projects(),
                    self.documents_v2(),
                )
                .map(ResourceBody::Text)
                .ok_or_else(|| format!("unknown handbook section: {id}"))
            }
            _ if (path.starts_with("books://receipts/")
                || path.starts_with("books://attachments/")) =>
            {
                // `books://attachments/{id}/page/{n}`: one page as a PNG
                // (plan P6; D-46: a file is handed over, but a resource
                // read is the agent's own fetch of a picture, never a
                // path).
                let prefix = if path.starts_with("books://receipts/") {
                    "books://receipts/"
                } else {
                    "books://attachments/"
                };
                let rest = &path[prefix.len()..];
                if let Some((id, page)) = rest.split_once("/page/") {
                    let id: u64 = id
                        .parse()
                        .map_err(|_| format!("bad attachment resource URI: {uri}"))?;
                    let page: u32 = page
                        .parse()
                        .map_err(|_| format!("bad page in {uri}: give a page number from 1"))?;
                    if page == 0 {
                        return Err("page numbers start at 1".into());
                    }
                    let store = self.lock()?;
                    let meta = find_attachment(&store, id)?;
                    let bytes = store.attachment_bytes(id).map_err(|e| e.to_string())?;
                    let png = ledger_import::extract::page_png(
                        &bytes,
                        usize::try_from(page - 1).unwrap_or(usize::MAX),
                        1600,
                    )
                    .map_err(|e| e.to_string())?;
                    let _ = meta;
                    return Ok(ResourceBody::Blob {
                        mime: "image/png".into(),
                        bytes: png,
                    });
                }
                let id: u64 = rest
                    .parse()
                    .map_err(|_| format!("bad attachment resource URI: {uri}"))?;
                let Json(out) = self
                    .get_receipt(Parameters(GetReceiptParams {
                        attachment_id: id,
                        view: None,
                        page: None,
                        max_px: None,
                    }))
                    .await?;
                json(&out)
            }
            _ => Err(format!("unknown resource: {uri}")),
        }
    }
}

#[tool_handler]
impl ServerHandler for BooksServer {
    async fn call_tool(
        &self,
        request: rmcp::model::CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::CallToolResponse, rmcp::ErrorData> {
        use crate::access::{self, Call};
        use ledger_store::access::Outcome;
        let mut call = Call::new(&self.access, &context, "tool", &request.name);
        let outcome_of = |out: &Result<rmcp::model::CallToolResponse, rmcp::ErrorData>| match out {
            Ok(rmcp::model::CallToolResponse::Complete(r)) if r.is_error != Some(true) => {
                Outcome::Ok
            }
            _ => Outcome::Error,
        };
        // A refused call's record carries its arguments' digest too,
        // worked out off the async threads.
        let refused_digest = |args: Option<serde_json::Map<String, serde_json::Value>>| async move {
            tokio::task::spawn_blocking(move || Call::digest_of(args.as_ref()))
                .await
                .ok()
                .flatten()
        };
        // Closing: refused as a protocol error, so a client knows it
        // didn't run (SharedStore INVARIANT 3).
        if self.store.closing() {
            call.digested(refused_digest(request.arguments.clone()).await);
            return self
                .refuse(&call, access::CLOSING, Err(not_run_closing()))
                .await;
        }
        if self.store.stopped() {
            call.digested(refused_digest(request.arguments.clone()).await);
            return self
                .refuse(&call, access::STOPPED, Ok(tool_error(STOPPED.to_string())))
                .await;
        }
        let needs = crate::scopes::needs(&request.name, request.arguments.as_ref());
        // access.rs INVARIANTS 1, 2.
        let refusal = match (call.caller(), needs) {
            (None, _) => Some((
                access::UNAUTHORIZED,
                Ok(tool_error(
                    "this call came without a token for these books".to_string(),
                )),
            )),
            // A tool no table names isn't run (fail closed): a protocol
            // error, as for any unknown tool (mcp-tools.md).
            (Some(_), None) => Some((
                "unknown_tool",
                Err(rmcp::ErrorData::invalid_params(
                    format!(
                        "there is no tool {:?}",
                        request.name.chars().take(64).collect::<String>()
                    ),
                    None,
                )),
            )),
            (Some(caller), Some(scope)) if !caller.may(scope) => Some((
                access::INSUFFICIENT_SCOPE,
                Ok(tool_error(format!(
                    "this agent's token can't do that: {} needs {scope}, and its token ({:?}) has {}. Only the human can give a token more (`wanderbooks token create`).",
                    request.name,
                    caller.token.name,
                    caller
                        .token
                        .scopes
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ))),
            )),
            // A file's bytes or text given as an argument: refused as an
            // unknown argument, before it runs (files.rs INVARIANT 5).
            _ => files::inline_refusal(&request.name, request.arguments.as_ref()).map(|why| {
                (
                    access::INLINE_FILE,
                    Err(rmcp::ErrorData::invalid_params(why, None)),
                )
            }),
        };
        if let Some((code, answer)) = refusal {
            call.digested(refused_digest(request.arguments.clone()).await);
            return self.refuse(&call, code, answer).await;
        }
        let timezone = self.settings().timezone.clone();
        let server = self.clone();
        // The journal's head once a call that may change the books has
        // run, for its record.
        // The call's record is written on its own blocking thread, before
        // it counts as done: a close waits for the record too (access.rs
        // INVARIANT 3).
        let running_call = call.clone();
        // Kept for the record of a call the server stopped before running.
        let request_args = request.arguments.clone();
        let tool_name = request.name.to_string();
        let envelope_args = request.arguments.clone();
        let result = run_blocking(&self.store, move || {
            Box::pin(async move {
                WROTE.with(|w| w.borrow_mut().take());
                ARGS.with(|a| *a.borrow_mut() = request.arguments.clone());
                let caller = running_call
                    .caller()
                    .map(|c| (c.token.id.clone(), c.token.name.clone()));
                CALLER.with(|c| *c.borrow_mut() = caller);
                AGENT_NOTE.with(|a| a.borrow_mut().take());
                YOU.with(|y| {
                    *y.borrow_mut() = running_call.caller().map(|c| {
                        serde_json::json!({
                            "principal": c.token.name,
                            "kind": "agent",
                            "scopes": c.token.scopes.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
                            "client_id": running_call.client_name(),
                        })
                    })
                });
                // Counted as running by now (run_blocking): hashed here.
                let digest = Call::digest_of(request.arguments.as_ref());
                let tcc =
                    rmcp::handler::server::tool::ToolCallContext::new(&server, request, context);
                let router = Self::tool_router();
                use futures_util::FutureExt;
                let out =
                    std::panic::AssertUnwindSafe(in_timezone_async(timezone, router.call(tcc)))
                        .catch_unwind()
                        .await;
                // The head after this call's own last write, if it wrote,
                // whatever it answered.
                let wrote = WROTE
                    .with(|w| w.borrow_mut().take())
                    .map(|(seq, hash)| ledger_store::chain::Head { seq, hash });
                let out = match out {
                    Ok(out) => out,
                    Err(panic) => {
                        // It ran and failed part way: recorded as an error
                        // with what it wrote, while still counted as
                        // running; then the panic goes on and stops the
                        // server (SharedStore INVARIANT 1).
                        let mut c = running_call;
                        c.digested(digest);
                        let _ = c.record_blocking(Outcome::Error, wrote);
                        std::panic::resume_unwind(panic)
                    }
                };
                // The result envelope (next_steps.rs INVARIANT 1).
                let out = next_steps::apply(&server, &tool_name, envelope_args.as_ref(), out);
                // Who asked is the request's alone: a later request on this
                // thread (a resource read) must not see it.
                YOU.with(|y| y.borrow_mut().take());
                let mut c = running_call;
                c.digested(digest);
                c.with_agent(AGENT_NOTE.with(|a| a.borrow_mut().take()));
                let recorded = c.record_blocking(outcome_of(&out), wrote);
                (out, recorded)
            })
        })
        .await;
        match result {
            Ok((out, Ok(()))) => redact_refusal(out),
            Ok((_, Err(()))) => {
                // access.rs INVARIANT 3.
                self.store.stop();
                Ok(tool_error(crate::access::RAN_NOT_RECORDED.to_string()))
            }
            // Not run because the books are closing: a protocol error, so
            // the client knows (SharedStore INVARIANT 3). (Its arguments
            // went with the request: no digest.)
            Err(e) if e.code.0 == NOT_RUN_CLOSING_CODE => {
                self.refuse(&call, access::CLOSING, Err(e)).await
            }
            // Not run: the server had stopped by the time it would have.
            Err(e) if e.code.0 == NOT_RUN_STOPPED_CODE => {
                call.digested(refused_digest(request_args.clone()).await);
                self.refuse(&call, access::STOPPED, Ok(tool_error(STOPPED.to_string())))
                    .await
            }
            // It panicked part way (recorded as it went): a plain refusal,
            // redacted like every other.
            Err(e) => redact_refusal(Ok(tool_error(e.message.to_string()))),
        }
    }

    async fn list_tools(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ListToolsResult, rmcp::ErrorData> {
        let mut tools = Self::tool_router().list_all();
        // A token is shown what it can call (scopes.rs); the list then
        // differs by caller, so it may be cached only for this one.
        let scopes = crate::access::caller(&context)
            .map(|c| c.token.scopes)
            .unwrap_or_default();
        tools.retain(|t| crate::scopes::shown(&t.name, &scopes));
        let hints = context
            .protocol_version()
            .is_some_and(|v| v >= rmcp::model::ProtocolVersion::V_2026_07_28);
        Ok(rmcp::model::ListToolsResult {
            result_type: Some(rmcp::model::ResultType::COMPLETE),
            tools,
            meta: None,
            next_cursor: None,
            ttl_ms: hints.then_some(0),
            cache_scope: hints.then_some(rmcp::model::CacheScope::Private),
        })
    }

    fn get_info(&self) -> ServerConfig {
        // A client connecting after a settings change sees the change
        // (never waiting for the books here; a request holding them syncs
        // them when it locks).
        if let Ok(store) = self.store.store.try_lock()
            && let Some(store) = store.as_ref()
        {
            self.live()
                .sync(store.ledger().settings(), self.store.currency.code());
        }
        ServerConfig::new(crate::opening::capabilities())
            // The rules in brief, with the policy these books actually use;
            // the rest is in the handbook.
            .with_instructions(handbook::connect_text(&self.settings(), self.currency()))
            .with_server_info(Implementation::new(
                "wanderbooks",
                env!("CARGO_PKG_VERSION"),
            ))
    }

    async fn list_resources(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, rmcp::ErrorData> {
        Ok(ListResourcesResult::with_all_items(resource_list()))
    }

    async fn list_resource_templates(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ListResourceTemplatesResult, rmcp::ErrorData> {
        Ok(rmcp::model::ListResourceTemplatesResult::with_all_items(
            resource_templates(),
        ))
    }

    async fn list_prompts(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ListPromptsResult, rmcp::ErrorData> {
        prompt_reader(&context)?;
        Ok(Self::prompt_list())
    }

    async fn get_prompt(
        &self,
        request: rmcp::model::GetPromptRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::GetPromptResponse, rmcp::ErrorData> {
        prompt_reader(&context)?;
        self.prompt_get(&request)
            .map(Into::into)
            .map_err(|e| rmcp::ErrorData::invalid_params(e, None))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, rmcp::ErrorData> {
        use crate::access::{self, Call};
        use ledger_store::access::Outcome;
        // Reading a resource is reading the books: books:read, recorded
        // (access.rs).
        let call = Call::new(&self.access, &context, "resource", &request.uri);
        let result = self.resource(request, &call).await;
        {
            let outcome = match &result {
                Ok(_) => Outcome::Ok,
                Err(e) if e.code.0 == NOT_RUN_CLOSING_CODE => {
                    Outcome::Refused(access::CLOSING.into())
                }
                Err(e) if e.message == access::INSUFFICIENT_SCOPE => {
                    Outcome::Refused(access::INSUFFICIENT_SCOPE.into())
                }
                Err(e) if e.message == access::UNAUTHORIZED => {
                    Outcome::Refused(access::UNAUTHORIZED.into())
                }
                Err(_) => Outcome::Error,
            };
            if call.record(outcome, None).await.is_err() {
                self.store.stop();
                // Not run stays so; the words say why.
                return Err(match result {
                    Err(e) if e.code.0 == NOT_RUN_CLOSING_CODE => rmcp::ErrorData::new(
                        e.code,
                        format!("{}; {}", e.message, access::NOT_RECORDED),
                        None,
                    ),
                    _ => rmcp::ErrorData::internal_error(access::NOT_RECORDED, None),
                });
            }
        }
        result
    }
}

impl BooksServer {
    /// Record a call refused unrun, then give its answer (access.rs
    /// INVARIANT 3).
    async fn refuse(
        &self,
        call: &crate::access::Call,
        code: &str,
        answer: Result<rmcp::model::CallToolResponse, rmcp::ErrorData>,
    ) -> Result<rmcp::model::CallToolResponse, rmcp::ErrorData> {
        if call
            .record(ledger_store::access::Outcome::Refused(code.into()), None)
            .await
            .is_err()
        {
            self.store.stop();
            // A call refused as not run stays so (the bridge sends it
            // again when the books are back); the words say why.
            return match answer {
                Err(e) if e.code.0 == NOT_RUN_CLOSING_CODE => Err(rmcp::ErrorData::new(
                    e.code,
                    format!("{}; {}", e.message, crate::access::NOT_RECORDED),
                    None,
                )),
                _ => Ok(tool_error(crate::access::NOT_RECORDED.to_string())),
            };
        }
        answer
    }

    /// A resource's contents, once its caller may read the books.
    async fn resource(
        &self,
        request: ReadResourceRequestParams,
        call: &crate::access::Call,
    ) -> Result<ReadResourceResponse, rmcp::ErrorData> {
        match call.caller() {
            None => {
                return Err(rmcp::ErrorData::invalid_request(
                    crate::access::UNAUTHORIZED,
                    None,
                ));
            }
            Some(c) if !c.may(ledger_store::auth::Scope::Read) => {
                return Err(rmcp::ErrorData::invalid_request(
                    crate::access::INSUFFICIENT_SCOPE,
                    None,
                ));
            }
            Some(_) => {}
        }
        let timezone = self.settings().timezone.clone();
        let uri = request.uri.clone();
        let server = self.clone();
        let at = uri.clone();
        let body = run_blocking(&self.store, move || {
            Box::pin(async move {
                YOU.with(|y| y.borrow_mut().take());
                in_timezone_async(timezone, server.resource_body(&at)).await
            })
        })
        .await?;
        // A refusal quotes the URI it was given: masked like a tool's.
        let body = body.map_err(|e| {
            redact_error(rmcp::ErrorData::resource_not_found(
                uri.clone(),
                Some(serde_json::json!({"reason": e})),
            ))
        })?;
        let contents = match body {
            ResourceBody::Text(text) => {
                let mime_type = if uri.starts_with(&format!("{}/", handbook::URI)) {
                    "text/markdown"
                } else {
                    "application/json"
                };
                ResourceContents::TextResourceContents {
                    uri,
                    mime_type: Some(mime_type.to_string()),
                    text,
                    meta: None,
                }
            }
            ResourceBody::Blob { mime, bytes } => {
                use base64::Engine;
                ResourceContents::BlobResourceContents {
                    uri,
                    mime_type: Some(mime),
                    blob: base64::engine::general_purpose::STANDARD.encode(bytes),
                    meta: None,
                }
            }
        };
        Ok(ReadResourceResponse::Complete(ReadResourceResult::new(
            vec![contents],
        )))
    }
}

#[cfg(test)]
#[path = "server_tests.rs"]
mod tests;

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use std::str::FromStr;

    pub(crate) fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    pub(crate) fn usd(s: &str) -> Money {
        Money::new(Decimal::from_str(s).unwrap(), Currency::USD)
    }

    /// A test's books' folder (`.0`) in a data root of its own, under a
    /// home (`.1`) removed when the test ends, pass or panic.
    pub(crate) struct TestDir(pub(crate) std::path::PathBuf, pub(crate) std::path::PathBuf);

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.1);
        }
    }

    pub(crate) fn unit_dir(tag: &str) -> TestDir {
        let home =
            std::env::temp_dir().join(format!("ledger-mcp-unit-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        let (root, _) =
            ledger_store::root::Root::setup(&home, Some(&home.join("root")), None).unwrap();
        TestDir(root.path().join("books").join("acme"), home)
    }

    /// New books in `dir`, which outlives them.
    pub(crate) fn store(dir: &TestDir) -> Store {
        Store::create(&dir.0, Currency::USD).unwrap()
    }

    /// A session's server for the books `store` holds, made as the books
    /// server makes every one (Served::open): driven here without a
    /// transport.
    pub(crate) fn served(store: Store) -> BooksServer {
        BooksServer::new(&Served::open(store).expect("the books are served"))
    }

    pub(crate) fn post(
        s: &mut Store,
        date: NaiveDate,
        dr: AccountId,
        cr: AccountId,
        amount: &str,
    ) -> u64 {
        let e = s.create_entry(date, "x").unwrap();
        s.add_line(e, dr, Side::Debit, usd(amount)).unwrap();
        s.add_line(e, cr, Side::Credit, usd(amount)).unwrap();
        s.post_entry(e).unwrap();
        e
    }

    /// `store` with `changes` journaled, as `books.set_setting` journals
    /// them: a session's server reads its settings only from there.
    pub(crate) fn settled(mut store: Store, changes: &[(&str, serde_json::Value)]) -> Store {
        let changes: Vec<_> = changes
            .iter()
            .map(|(key, value)| ledger_core::settings::Change {
                key: (*key).to_string(),
                value: Some(value.clone()),
            })
            .collect();
        let source = ledger_core::settings::Source {
            decided_by: "Morgan Ellis".into(),
            decided_on: d(2026, 1, 1),
            citation: None,
            note: None,
        };
        store
            .change_settings(&changes, &source)
            .expect("the settings are taken");
        store
    }

    /// Two members, Ada (with a distributions account) and Bo.
    pub(crate) fn members() -> (&'static str, serde_json::Value) {
        (
            "company.members",
            serde_json::json!([
                {"name": "Ada", "share": "0.5", "capital_account": "3000",
                 "distribution_account": "3100"},
                {"name": "Bo", "share": "0.5", "capital_account": "3001"},
            ]),
        )
    }
}
