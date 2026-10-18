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
use crate::local_files::{ImportRoots, Places, Protected, resolved};
use crate::policy::ReceiptPolicy;
use crate::settings::{Live, Settings};
use crate::tin_files;
use crate::types::*;
use chrono::NaiveDate;
#[cfg(test)]
use ledger_config::Company;
use ledger_core::{
    AccountId, AccountType, AssetStatus, Currency, DocumentLine, DocumentStatus, JournalEntry,
    Ledger, Money, NewAsset, PeriodStatus, Side, accumulated_depreciation, book_value,
    depreciation_schedule, reports,
};
use ledger_import::ReportMeta;
use ledger_store::Store;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::{
    Implementation, ListResourcesResult, ReadResourceRequestParams, ReadResourceResponse,
    ReadResourceResult, Resource, ResourceContents, ServerCapabilities, ServerConfig,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::{ServerHandler, tool, tool_handler, tool_router};
use rust_decimal::Decimal;
use std::sync::{Arc, Mutex};

/// Max receipt/document upload: 25 MB.
const MAX_UPLOAD_BYTES: usize = 25 * 1024 * 1024;

/// The server. One store, one receipt policy, shared across tool calls.
/// `Store` is `!Sync`, so access is serialized through a mutex; the server
/// is single-writer by construction. Tool bodies (synchronous disk and
/// SQLite work) run on tokio's blocking threads, never on the async
/// workers, so a long call can't stall the transport ([`run_blocking`]).
#[derive(Clone)]
pub struct BooksServer {
    store: Arc<SharedStore>,
    settings: Arc<Mutex<Live>>,
    import_roots: Arc<ImportRoots>,
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
/// ```
pub struct SharedStore {
    store: tokio::sync::Mutex<Store>,
    stopped: std::sync::atomic::AtomicBool,
    /// The books' base currency, fixed at their creation: read without
    /// the lock (the connect text names it).
    currency: Currency,
}

impl SharedStore {
    /// Share an open store.
    pub fn new(store: Store) -> Arc<Self> {
        Arc::new(Self {
            currency: store.base_currency(),
            store: tokio::sync::Mutex::new(store),
            stopped: std::sync::atomic::AtomicBool::new(false),
        })
    }

    fn stopped(&self) -> bool {
        self.stopped.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn stop(&self) {
        self.stopped
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Why a file with full TINs isn't written into an import root.
const TIN_FILE_IN_ROOT: &str = "a file with full TINs is never saved in the server's import folders (by default your downloads folder), where it could be filed back into the books: save it elsewhere, such as your documents folder";

/// What every request gets after an internal error (SharedStore
/// INVARIANT 2).
const STOPPED: &str = "the books server stopped taking requests after an internal error, to protect the books: ask the human to restart the books server (the books are checked again when it starts)";

/// Why the vault won't open with a key file inside the books.
const KEY_IN_BOOKS: &str = "the TIN vault's key file is inside the books' folder, beside the records it unlocks: ask the human to move it out of the books and set [vault] key_file in the settings";

/// Why the vault won't store into books whose version history would keep
/// its records.
const VAULT_IN_HISTORY: &str = "the TIN vault can't store anything here yet: the books' folder is kept under version history that doesn't leave the vault out, so its records would be kept forever. Ask the human to leave the vault folder out of that history, then try again";

/// Why a file with a full TIN isn't written inside the books.
const INSIDE_BOOKS: &str = "that path is inside the books' folder, whose files are kept forever: a file with a full TIN goes elsewhere, such as the human's documents folder";

/// The store, locked for one stretch of synchronous work. Not `Send`, so
/// the compiler refuses any tool future that would hold it across an
/// `.await` (tool futures must be `Send`): the lock is never held while a
/// task is suspended. Dropped by a panic, it stops the server
/// (SharedStore INVARIANT 1).
pub struct StoreGuard<'a> {
    guard: tokio::sync::MutexGuard<'a, Store>,
    shared: &'a SharedStore,
    _not_send: std::marker::PhantomData<*const ()>,
}

impl Drop for StoreGuard<'_> {
    fn drop(&mut self) {
        if std::thread::panicking() {
            self.shared.stop();
        }
    }
}

impl std::ops::Deref for StoreGuard<'_> {
    type Target = Store;
    fn deref(&self) -> &Store {
        &self.guard
    }
}

impl std::ops::DerefMut for StoreGuard<'_> {
    fn deref_mut(&mut self) -> &mut Store {
        &mut self.guard
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
        return Err(rmcp::ErrorData::internal_error(STOPPED, None));
    }
    let joined = tokio::task::spawn_blocking(move || {
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

#[cfg(test)]
pub use crate::settings::CashRoles;

/// Likely counterparties for an entry, best first (at most three): parties
/// named in its description, then in its receipts' names and
/// descriptions, then the party of an earlier entry with the same
/// description. Names match as whole words, ignoring case; a party also
/// matches by the first word of its name when that word has 4+ letters
/// ("Apple" for "Apple Inc.").
fn suggest_parties(store: &Store, entry: &JournalEntry) -> Result<Vec<PartySuggestionOut>, String> {
    let ledger = store.ledger();
    let words = |t: &str| -> Vec<String> {
        t.to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .map(str::to_string)
            .collect()
    };
    let contains = |hay: &[String], needle: &[String]| {
        !needle.is_empty() && hay.windows(needle.len()).any(|w| w == needle)
    };
    let names_of = |p: &ledger_core::Party| -> Vec<Vec<String>> {
        let mut out = vec![words(&p.name)];
        if let Some(d) = &p.display_name {
            out.push(words(d));
        }
        if let Some(first) = words(&p.name).first()
            && first.len() >= 4
        {
            out.push(vec![first.clone()]);
        }
        out
    };
    let receipts: String = store
        .attachments_for(ledger_store::Subject::Entry(entry.id()))
        .map_err(|e| e.to_string())?
        .iter()
        .map(|a| {
            format!(
                "{} {} ",
                a.file_name,
                a.description.clone().unwrap_or_default()
            )
        })
        .collect();
    let desc = words(entry.description());
    let rec = words(&receipts);
    let mut out: Vec<PartySuggestionOut> = Vec::new();
    let push = |out: &mut Vec<PartySuggestionOut>, p: &ledger_core::Party, why: String| {
        if !out.iter().any(|s| s.party_id == p.id) {
            out.push(PartySuggestionOut {
                party_id: p.id,
                name: p.name.clone(),
                why,
            });
        }
    };
    let live: Vec<&ledger_core::Party> = ledger.parties().filter(|p| !p.archived).collect();
    for p in &live {
        if names_of(p).iter().any(|n| contains(&desc, n)) {
            push(&mut out, p, "named in the description".into());
        }
    }
    for p in &live {
        if names_of(p).iter().any(|n| contains(&rec, n)) {
            push(&mut out, p, "named in a receipt".into());
        }
    }
    let key = entry.description().trim().to_lowercase();
    for e in ledger.posted_entries() {
        if e.id() != entry.id()
            && e.description().trim().to_lowercase() == key
            && let Some(p) = ledger
                .entry_parties(e.id())
                .first()
                .and_then(|s| ledger.get_party(s.party))
            && !p.archived
        {
            push(
                &mut out,
                p,
                format!("tagged on entry {} with the same description", e.id()),
            );
        }
    }
    out.truncate(3);
    Ok(out)
}

/// Where payments to vendors come from, for the 1099 worksheet: the
/// `[books] cash_accounts` and `financing_accounts` in the chart (codes the
/// chart lacks are skipped; `books.settings` reports them).
fn payment_accounts(
    ledger: &ledger_core::Ledger,
    roles: &crate::settings::CashRoles,
) -> ledger_core::PaymentAccounts {
    let ids = |codes: &[String]| {
        codes
            .iter()
            .filter_map(|c| ledger.chart().find_by_code(c).map(|a| a.id))
            .collect()
    };
    ledger_core::PaymentAccounts {
        cash: ids(&roles.cash),
        financing: ids(&roles.financing),
    }
}

/// The 1099 worksheet for `year`, the one every tool and the CPA package
/// use: payments out of [`payment_accounts`], an entry's shares resolved
/// with the settings' members (as [`share_context`] checks them).
fn vendor_1099_report(
    ledger: &ledger_core::Ledger,
    settings: &Settings,
    year: i32,
    threshold: Option<Decimal>,
) -> Result<ledger_core::Vendor1099Report, ledger_core::LedgerError> {
    ledger_core::vendor_1099_report_with_members(
        ledger,
        year,
        threshold,
        &payment_accounts(ledger, &settings.roles),
        &settings.company.party_members(ledger),
    )
}

impl BooksServer {
    /// The settings that name account `code`, by key (or, for books
    /// that still read a settings file, as the file spells them).
    fn config_mentions(&self, code: &str) -> Vec<String> {
        let mut out = Vec::new();
        let settings = self.settings();
        for (key, in_file, codes) in [
            (
                "books.cash_accounts",
                "[books] cash_accounts",
                &settings.roles.cash,
            ),
            (
                "books.financing_accounts",
                "[books] financing_accounts",
                &settings.roles.financing,
            ),
            (
                "books.investing_accounts",
                "[books] investing_accounts",
                &settings.roles.investing,
            ),
            (
                "books.close_sweep",
                "[books] close_sweep",
                &settings.company.close_sweep,
            ),
            (
                "policy.evidence.required_accounts",
                "receipt_required_accounts",
                &settings.policy.receipt_required_accounts,
            ),
            (
                "policy.evidence.exempt_accounts",
                "receipt_exempt_accounts",
                &settings.policy.receipt_exempt_accounts,
            ),
        ] {
            if codes.iter().any(|c| c == code) {
                out.push(self.setting_label(key, in_file));
            }
        }
        if settings.company.close_into.as_deref() == Some(code) {
            out.push(self.setting_label("books.close_into", "[books] close_into"));
        }
        let members = self.setting_label("company.members", "[[company.members]]");
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

    /// Build the server around an open store.
    pub fn new(store: Store, policy: ReceiptPolicy) -> Self {
        Self::new_shared(SharedStore::new(store), policy)
    }

    /// Build the server around a shared store (for HTTP: each connection
    /// gets a handle to the same store).
    pub fn new_shared(store: Arc<SharedStore>, policy: ReceiptPolicy) -> Self {
        Self {
            store,
            settings: Arc::new(Mutex::new(Live::fixed(Settings {
                policy,
                ..Settings::default()
            }))),
            import_roots: Arc::new(ImportRoots::defaults()),
        }
    }

    /// The folders `books.import_local_file` may read from (local_files.rs).
    pub fn with_import_roots(mut self, roots: ImportRoots) -> Self {
        self.import_roots = Arc::new(roots);
        self
    }

    /// Take the settings from files, reloading them whenever they change
    /// (crate::settings), shared with the other sessions holding `live`.
    pub fn with_live_settings(mut self, live: Arc<Mutex<Live>>) -> Self {
        self.settings = live;
        self
    }

    /// Set the company (name for report headers, fiscal year).
    #[cfg(test)]
    pub fn with_company(self, company: Company) -> Self {
        self.live().edit(|s| s.company = company);
        self
    }

    /// Set the `[books]` cash flow account roles.
    #[cfg(test)]
    pub fn with_cash_roles(self, roles: CashRoles) -> Self {
        self.live().edit(|s| s.roles = roles);
        self
    }

    fn live(&self) -> std::sync::MutexGuard<'_, Live> {
        // A panic while holding it can't leave the settings half-written
        // (they're swapped whole), so a poisoned lock is still good.
        self.settings.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The settings in force as the books would keep them: the books' own,
    /// or, for books that still read a settings file, that file's reading
    /// (no source: the file is the source).
    fn settings_as_kept(&self, store: &Store) -> ledger_core::settings::Settings {
        let books = store.ledger().settings();
        if books.is_journaled() {
            return books.clone();
        }
        let Some((toml, json)) = self.live().file_texts() else {
            return books.clone();
        };
        let empty = ledger_core::settings::Settings::default();
        match ledger_config::codec::import(
            toml.as_deref().unwrap_or(""),
            json.as_deref(),
            &empty,
            self.currency(),
        ) {
            Ok(i) => {
                let mut s = empty;
                let file = ledger_core::settings::Source {
                    decided_by: "the settings file".into(),
                    decided_on: today(),
                    citation: None,
                    note: None,
                };
                match s.apply(&i.changes, &file, 0) {
                    Ok(()) => s,
                    Err(_) => books.clone(),
                }
            }
            Err(_) => books.clone(),
        }
    }

    /// How a message names a setting: by its key when the books keep
    /// their own settings, else as the settings file spells it.
    fn setting_label(&self, key: &str, in_file: &str) -> String {
        if self.settings().journaled {
            key.to_string()
        } else {
            in_file.to_string()
        }
    }

    /// Every tool the server serves.
    pub(crate) fn tool_router() -> rmcp::handler::server::router::tool::ToolRouter<Self> {
        Self::core_router() + Self::kept_router()
    }

    /// The settings in force for this request (reloaded at its start, in
    /// `call_tool` and `read_resource`, if their files changed).
    fn settings(&self) -> Arc<Settings> {
        self.live().current()
    }

    /// The books' base currency code, e.g. "USD".
    fn currency(&self) -> &str {
        self.store.currency.code()
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
        let or = |given: &'_ [String], default: &'_ [String]| -> Vec<String> {
            if given.is_empty() {
                default.to_vec()
            } else {
                given.to_vec()
            }
        };
        cash_flow_statement(
            ledger,
            start,
            end,
            &or(cash, &self.settings().roles.cash),
            &or(financing, &self.settings().roles.financing),
            &or(investing, &self.settings().roles.investing),
        )
    }

    /// Header and provenance for a rendered report.
    fn report_meta(&self, store: &Store) -> ReportMeta {
        ReportMeta {
            company: self.settings().company.name.clone(),
            basis: self.settings().company.basis.clone(),
            prepared_on: today(),
            journal_seq: store.journal_seq(),
        }
    }

    /// Members as reports name them (empty unless a multi-member LLC).
    fn report_members(&self) -> Vec<ledger_import::Member> {
        self.settings()
            .company
            .members
            .iter()
            .map(|m| ledger_import::Member {
                name: m.name.clone(),
                capital_code: m.capital_account.clone(),
                distribution_code: m.distribution_account.clone(),
            })
            .collect()
    }

    /// Members with their capital accounts resolved in the chart, for the
    /// statement of members' equity.
    fn member_capitals(&self, ledger: &Ledger) -> Result<Vec<ledger_core::MemberCapital>, String> {
        if self.settings().company.members.is_empty() {
            return Err(format!(
                "no members configured: set {} (name, share, capital_account for each)",
                self.setting_label("company.members", "[[company.members]] in the settings")
            ));
        }
        self.settings()
            .company
            .members
            .iter()
            .map(|m| {
                let account = ledger
                    .chart()
                    .find_by_code(&m.capital_account)
                    .ok_or_else(|| {
                        format!(
                            "member {}: unknown capital account {}",
                            m.name, m.capital_account
                        )
                    })?;
                let distribution_account = match &m.distribution_account {
                    Some(code) => Some(
                        ledger
                            .chart()
                            .find_by_code(code)
                            .ok_or_else(|| {
                                format!("member {}: unknown distribution account {code}", m.name)
                            })?
                            .id,
                    ),
                    None => None,
                };
                Ok(ledger_core::MemberCapital {
                    name: m.name.clone(),
                    capital_account: account.id,
                    distribution_account,
                    share: m.share,
                })
            })
            .collect()
    }

    /// Where a period close goes, from wanderbooks.toml: members' capital
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
        let (fy_start, _) = self.settings().company.fiscal_year_containing(as_of);
        let net_income_ytd = ledger_core::profit_and_loss_for_period(ledger, fy_start, as_of, None)
            .map_err(|e| e.to_string())?
            .net_income();
        // A close takes everything unclosed through its end, earlier years
        // included, so this year's unclosed part is the income from the day
        // after the last close (never before the year's start) through
        // as_of. (Counting the closing entries dated this year instead
        // charged this year with closing last year's income: found by
        // year_earnings_split_closed_from_unclosed, 2026-10-14.)
        let last_close = ledger
            .list_periods()
            .iter()
            .filter(|p| p.status == PeriodStatus::Closed && p.end <= as_of)
            .map(|p| p.end)
            .max();
        let from = match last_close.and_then(|e| e.succ_opt()) {
            Some(after) if after > fy_start => after,
            _ => fy_start,
        };
        let unclosed = if from <= as_of {
            ledger_core::profit_and_loss_for_period(ledger, from, as_of, None)
                .map_err(|e| e.to_string())?
                .net_income()
        } else {
            Decimal::ZERO
        };
        Ok(ledger_import::YearEarnings {
            net_income_ytd,
            unclosed,
        })
    }

    /// Cash and card accounts due for reconciliation
    /// (ledger_store::reconcilable, the viewer's Needs-attention rule):
    /// the cash accounts (`[books] cash_accounts`, else assets named for
    /// cash), the card accounts (liabilities named "card", before their
    /// first statement), and every other liability with a recorded
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

    /// The TIN vault's directory: `vault/` in the books dir (git-ignored).
    fn vault_dir(store: &Store) -> std::path::PathBuf {
        books_dir(store).join("vault")
    }

    /// What is never read through a path, even inside an import root
    /// (local_files.rs INVARIANT 3; Protected::of): the data dir, the
    /// vault, the key file in force and the default one (a key left there
    /// from before still unlocks the vault), and the folders it seals; and
    /// the rest of the books dir.
    fn protected(&self, store: &Store) -> Protected {
        Protected::of(
            &places(store, &self.settings()),
            &self.import_roots.usable(),
        )
    }

    /// Refuse bytes on their way into the books (books.attach_receipt,
    /// books.file_document, books.import_local_file) if they carry a full
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
            &Self::vault_dir(store),
            bytes,
            mime_type,
            file_name,
            || known_tin_last4(store),
            || self.vault_tins(store),
        )
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

    /// Read a file an agent names (books.import_local_file, books.set_w9
    /// w9_file_path) through the import roots' lockdown (local_files.rs),
    /// with its canonical path.
    fn read_local(&self, path: &str) -> Result<(std::path::PathBuf, Vec<u8>), String> {
        let protected = {
            let store = self.lock()?;
            self.protected(&store)
        };
        self.import_roots
            .read(path.trim(), &protected, MAX_UPLOAD_BYTES)
    }

    /// Where a file holding full TINs may be written: a new file outside
    /// the books (outside_books) and outside every import root, so it can
    /// never be filed back in as a receipt and read out again.
    fn tin_file_dest(&self, store: &Store, path: &str) -> Result<std::path::PathBuf, String> {
        let dest = outside_books(store, path)?;
        self.outside_import_roots(&dest)?;
        Ok(dest)
    }

    fn outside_import_roots(&self, dest: &std::path::Path) -> Result<(), String> {
        if self.import_roots.contains(dest) {
            return Err(TIN_FILE_IN_ROOT.to_string());
        }
        Ok(())
    }

    /// Read a W-9 file (PDF, PNG, or JPEG, 25 MB at most) for the vault,
    /// through the import roots' lockdown like any local file: the type
    /// is judged by where the path really leads (a link named w9.pdf to
    /// another file is that file).
    fn read_w9_file(&self, path: &str) -> Result<ledger_vault::W9File, String> {
        let (canonical, bytes) = self.read_local(path)?;
        // Not recorded here: set_w9 records it once the call can only
        // succeed, just before the vault write (tin_files.rs INVARIANT 1),
        // so a refused call never marks an ordinary file.
        let mime_type = match canonical
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_lowercase)
            .as_deref()
        {
            Some("pdf") => "application/pdf",
            Some("png") => "image/png",
            Some("jpg" | "jpeg") => "image/jpeg",
            _ => return Err("w9_file_path is a PDF, PNG, or JPEG (.pdf, .png, .jpg)".into()),
        };
        Ok(ledger_vault::W9File {
            file_name: canonical
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "w9".into()),
            mime_type: mime_type.to_string(),
            bytes,
        })
    }

    /// Record a file read or written with full TINs (tin_files.rs
    /// INVARIANT 2), before it is written out: every session, and every
    /// later server for these books, refuses it.
    fn wrote_tins(store: &Store, bytes: &[u8]) -> Result<(), String> {
        tin_files::record(&Self::vault_dir(store), bytes)
    }

    /// Open the TIN vault with the key file in force (`[vault] key_file`,
    /// else `~/.config/wanderbooks/vault.key`), creating the key only when
    /// `create` (storing a TIN); reading never creates one. A key file
    /// inside the books dir is refused: git would keep it beside the
    /// records it unlocks. Storing into books kept in git is refused
    /// unless their `.gitignore` lists `vault/`.
    fn open_vault(&self, store: &Store, create: bool) -> Result<ledger_vault::Vault, String> {
        let key = self.settings().key_file();
        if is_inside(&books_dir(store), &key) {
            return Err(KEY_IN_BOOKS.to_string());
        }
        // Writing: a git repository must ignore the vault, or its records
        // would be committed (and could never be purged).
        let books = books_dir(store);
        if create && books.join(".git").exists() {
            let ignored = std::fs::read_to_string(books.join(".gitignore"))
                .unwrap_or_default()
                .lines()
                .any(|l| matches!(l.trim(), "vault/" | "/vault/" | "vault" | "/vault"));
            if !ignored {
                return Err(VAULT_IN_HISTORY.to_string());
            }
        }
        // No message names the key file or the books' folders.
        ledger_vault::Vault::open(&Self::vault_dir(store), &key, create).map_err(|e| match e {
            ledger_vault::Error::NoKey(_) => "the TIN vault is locked: its key file is missing (restore it from the human's password manager, or set [vault] key_file in the settings)".to_string(),
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
        // A request that panicked while this one waited stopped it.
        if self.store.stopped() {
            return Err(STOPPED.to_string());
        }
        // Books that keep their own settings: take any change (crate::
        // settings INVARIANT 1).
        let timezone = {
            let mut live = self.live();
            live.sync(guard.ledger().settings(), self.store.currency.code());
            live.current().timezone.clone()
        };
        // "Today" from here on is the zone in force now (a change another
        // session made while this request waited included).
        TIMEZONE.with(|z| *z.borrow_mut() = timezone);
        Ok(StoreGuard {
            guard,
            shared: &self.store,
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
        let split_group = self
            .settings()
            .policy
            .split_group(store.ledger(), entry_id, entry);
        if !self
            .settings()
            .policy
            .receipt_required(store.ledger(), entry)
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

/// A date: exactly YYYY-MM-DD, a real day in the calendar the books keep
/// (0001-01-01 through 9999-12-31, `ledger_core::calendar`). No sign, no
/// space, no five-digit year: every date past here is one date arithmetic
/// reaches (P0 review round 3, S1).
fn parse_date(s: &str) -> Result<NaiveDate, String> {
    let shaped = s.len() == 10
        && s.bytes().enumerate().all(|(i, b)| {
            if i == 4 || i == 7 {
                b == b'-'
            } else {
                b.is_ascii_digit()
            }
        });
    shaped
        .then(|| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
        .flatten()
        .filter(|d| ledger_core::calendar::in_calendar(*d))
        .ok_or_else(|| {
            format!("bad date {s:?}: expected YYYY-MM-DD, a real day from 0001-01-01 to 9999-12-31")
        })
}

/// A decimal string, exactly `-?digits[.digits]` (INVARIANT 3).
fn parse_amount(s: &str) -> Result<Decimal, String> {
    ledger_config::decimal(s)
        .ok_or_else(|| format!("bad amount {s:?}: expected a decimal string like \"123.45\""))
}

fn parse_side(s: &str) -> Result<Side, String> {
    match s.to_lowercase().as_str() {
        "debit" => Ok(Side::Debit),
        "credit" => Ok(Side::Credit),
        _ => Err(format!("bad side {s:?}: expected \"debit\" or \"credit\"")),
    }
}

/// The settings that disagree with the books: each names the setting and
/// what to change. A code must name an account in the chart, of the kind
/// the setting needs (cash and investing: assets; financing: liabilities;
/// capital, distributions, close_into, close_sweep: equity).
/// The live conventions that name account `code` (a posting template's
/// line, an approval rule's accounts): after a renumber, each still names
/// the old code until revised.
fn convention_mentions(ledger: &Ledger, code: &str) -> Vec<String> {
    convention_codes(ledger)
        .into_iter()
        .filter(|(_, c)| c == code)
        .map(|(id, _)| format!("convention {id} (revise it with books.set_convention)"))
        .collect()
}

/// Every account code a live convention names: (convention id, code).
fn convention_codes(ledger: &Ledger) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for c in ledger
        .conventions()
        .conventions()
        .filter(|c| c.retired.is_none())
    {
        if let Some(t) = c.template() {
            for l in t.lines {
                if ledger_core::conventions::slot(&l.account).is_none() {
                    out.push((c.id.clone(), l.account));
                }
            }
        }
        if let Some(r) = c.approval_rule() {
            for a in r.accounts {
                out.push((c.id.clone(), a));
            }
        }
    }
    out.dedup();
    out
}

fn settings_problems(ledger: &Ledger, cfg: &Settings) -> Vec<String> {
    let mut out = Vec::new();
    // Named as the books keep them, or as the settings file spells them.
    let label =
        |key: &'static str, in_file: &'static str| if cfg.journaled { key } else { in_file };
    let how = if cfg.journaled {
        " with books.set_setting"
    } else {
        " in the settings"
    };
    let mut check = |setting: &str, code: &str, want: &[AccountType]| {
        match ledger.chart().find_by_code(code) {
            None => out.push(format!(
                "{setting} names {code}, which isn't in the chart: add that account, or change {setting}{how}"
            )),
            Some(a) if !want.contains(&a.account_type) => out.push(format!(
                "{setting} names {code} \"{}\", a {:?} account, but needs {}: change {setting}{how}",
                a.name,
                a.account_type,
                want.iter().map(|t| format!("{t:?}")).collect::<Vec<_>>().join(" or "),
            )),
            Some(_) => {}
        }
    };
    use AccountType::*;
    for code in &cfg.roles.cash {
        check(
            label("books.cash_accounts", "[books] cash_accounts"),
            code,
            &[Asset],
        );
    }
    for code in &cfg.roles.financing {
        check(
            label("books.financing_accounts", "[books] financing_accounts"),
            code,
            &[Liability, Equity],
        );
    }
    for code in &cfg.roles.investing {
        check(
            label("books.investing_accounts", "[books] investing_accounts"),
            code,
            &[Asset],
        );
    }
    let c = &cfg.company;
    for m in &c.members {
        check(
            &format!(
                "{} {} capital_account",
                label("company.members", "[[company.members]]"),
                m.name
            ),
            &m.capital_account,
            &[Equity],
        );
        if let Some(d) = &m.distribution_account {
            check(
                &format!(
                    "{} {} distribution_account",
                    label("company.members", "[[company.members]]"),
                    m.name
                ),
                d,
                &[Equity],
            );
        }
    }
    if let Some(code) = &c.close_into {
        check(
            label("books.close_into", "[books] close_into"),
            code,
            &[Equity],
        );
    }
    for code in &c.close_sweep {
        check(
            label("books.close_sweep", "[books] close_sweep"),
            code,
            &[Equity],
        );
    }
    // Conventions naming an account no longer in the chart (renumbered or
    // never there): a template can't draft, a rule covers nothing.
    for (id, code) in convention_codes(ledger) {
        if ledger.chart().find_by_code(&code).is_none() {
            out.push(format!(
                "convention {id} names account {code}, which isn't in the chart: revise it with books.set_convention"
            ));
        }
    }
    out
}

/// A tax basis record from a tool's rule name and contributor basis.
fn parse_tax_basis(rule: &str, basis: Option<&str>) -> Result<ledger_core::TaxBasis, String> {
    let rule = ledger_core::TaxBasisRule::parse(rule).ok_or_else(|| {
        format!("tax basis rule {rule:?}: expected cost, carryover, or personal_use_conversion")
    })?;
    Ok(ledger_core::TaxBasis {
        rule,
        contributor_basis: basis.map(parse_amount).transpose()?,
    })
}

/// An asset's tax basis for a tool's output.
fn tax_out(a: &ledger_core::Asset) -> Option<TaxBasisOut> {
    let t = a.tax?;
    Some(TaxBasisOut {
        rule: t.rule.key().to_string(),
        contributor_basis: t.contributor_basis.map(money),
        tax_basis: a.tax_basis().map(money),
        built_in_gain: a.built_in_gain().map(money),
    })
}

/// An asset named by id or by tag (exactly one).
fn resolve_asset(store: &Store, id: Option<u64>, tag: Option<&str>) -> Result<u64, String> {
    match (id, tag) {
        (Some(id), None) => store
            .ledger()
            .get_asset(id)
            .map(|a| a.id)
            .ok_or_else(|| format!("unknown asset {id}")),
        (None, Some(tag)) => store
            .ledger()
            .asset_by_tag(tag)
            .map(|a| a.id)
            .ok_or_else(|| format!("no asset is tagged {tag:?}")),
        _ => Err("name the asset by asset_id or by tag (one of them)".into()),
    }
}

/// An asset named by a string that is its current tag or its id. A tag
/// wins: tags like "1001" are allowed, and the label is what the human
/// reads.
fn resolve_asset_ref(store: &Store, r: &str) -> Result<u64, String> {
    if let Some(a) = store.ledger().asset_by_tag(r) {
        return Ok(a.id);
    }
    match r.parse::<u64>() {
        Ok(id) if store.ledger().get_asset(id).is_some() => Ok(id),
        _ => Err(format!("no asset has the tag or id {r:?}")),
    }
}

/// Every control role and the account filling it, for books.roles.
fn roles_out(ledger: &Ledger) -> Vec<RoleOut> {
    ledger_core::control_accounts()
        .map(|c| {
            let account = ledger.role_account(c).and_then(|id| ledger.chart().get(id));
            RoleOut {
                role: c.key.to_string(),
                purpose: c.role.to_string(),
                account_type: format!("{:?}", c.account_type),
                default_code: c.code.to_string(),
                assigned: ledger.role_assignment(c).is_some(),
                account_code: account.map(|a| a.code.clone()),
                account_name: account.map(|a| a.name.clone()),
            }
        })
        .collect()
}

/// Parse a calendar-month period "YYYY-MM".
fn parse_period_mcp(period: &str) -> Result<(i32, u32), String> {
    let err = || format!("bad period {period:?}: expected YYYY-MM");
    let mut parts = period.split('-');
    let year: i32 = parts.next().ok_or_else(err)?.parse().map_err(|_| err())?;
    let month: u32 = parts.next().ok_or_else(err)?.parse().map_err(|_| err())?;
    // A month of the calendar the books keep (ledger_core::calendar).
    if parts.next().is_some() || !(1..=9999).contains(&year) || !(1..=12).contains(&month) {
        return Err(err());
    }
    Ok((year, month))
}

/// Last calendar day of a month: refused, never a panic, for one chrono
/// can't reach (P0 review round 3, S1).
fn last_day_of_month(year: i32, month: u32) -> Result<NaiveDate, String> {
    NaiveDate::from_ymd_opt(year, month, 1)
        .and_then(|first| first.checked_add_months(chrono::Months::new(1)))
        .and_then(|next| next.pred_opt())
        .ok_or_else(|| format!("bad period {year:04}-{month:02}: no such month"))
}

fn parse_account_type(s: &str) -> Result<AccountType, String> {
    match s.to_lowercase().as_str() {
        "asset" => Ok(AccountType::Asset),
        "liability" => Ok(AccountType::Liability),
        "equity" => Ok(AccountType::Equity),
        "revenue" => Ok(AccountType::Revenue),
        "expense" => Ok(AccountType::Expense),
        _ => Err(format!(
            "bad account_type {s:?}: expected asset, liability, equity, revenue, or expense"
        )),
    }
}

/// Resolve an account reference: an account code, or a numeric id. A
/// number that is one account's code and another's id is refused
/// (INVARIANT 6).
fn resolve_account(store: &Store, account: &str) -> Result<AccountId, String> {
    let chart = store.ledger().chart();
    let by_code = chart.find_by_code(account).map(|a| a.id);
    let by_id = account
        .parse::<u64>()
        .ok()
        .filter(|id| chart.get(*id).is_some());
    match (by_code, by_id) {
        (Some(c), Some(i)) if c != i => Err(format!(
            "account {account:?} is ambiguous: it is the code of account {c} and the id of account {i}; pass the code of the one you mean"
        )),
        (Some(c), _) => Ok(c),
        (None, Some(i)) => Ok(i),
        (None, None) => Err(format!(
            "unknown account {account:?}: use an id or code from books.chart"
        )),
    }
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

/// Resolve a party by id or by name: the name ignoring case and
/// surrounding space, among parties not archived (an archived party goes
/// by id). A number that is one party's id and another's name is refused
/// (INVARIANT 6).
fn resolve_party(store: &Store, party: &str) -> Result<ledger_core::PartyId, String> {
    let ledger = store.ledger();
    let by_id = party
        .trim()
        .parse::<u64>()
        .ok()
        .filter(|id| ledger.get_party(*id).is_some());
    let by_name = ledger.party_by_name(party).map(|p| p.id);
    match (by_id, by_name) {
        (Some(i), Some(n)) if i != n => Err(format!(
            "party {party:?} is ambiguous: it is party {i}'s id and party {n}'s name; pass the id"
        )),
        (Some(i), _) | (None, Some(i)) => Ok(i),
        (None, None) => Err(format!(
            "unknown party {party:?}: use an id or name from books.parties (an archived party goes by id), or add it with books.add_party"
        )),
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
            "Current trial balance, debits equal credits.",
        ),
        mk(
            "books://reports/balance-sheet",
            "balance-sheet",
            "Balance sheet",
            "Assets, liabilities, equity.",
        ),
        mk(
            "books://reports/profit-and-loss",
            "profit-and-loss",
            "Profit and loss",
            "Revenue, expenses, net income.",
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

/// The subject kinds, for messages.
const SUBJECT_KINDS: &str = "entry, account, asset, party, invoice, bill, period, payroll_run, schedule, reconciliation, company";

/// Resolve a tool's subject: its kind, and its id resolved the way the
/// other tools resolve references (an account by code or id, an asset by
/// tag or id, a party by id or name), checked to exist.
fn resolve_subject(store: &Store, s: &SubjectParam) -> Result<ledger_store::Subject, String> {
    use ledger_store::Subject;
    let kind = s.kind.trim();
    let text = match &s.id {
        None => None,
        Some(RefId::Number(n)) => Some(n.to_string()),
        Some(RefId::Text(t)) => Some(t.trim().to_string()),
    };
    if Subject::from_parts(kind, 0).is_none() {
        return Err(format!(
            "unknown subject kind {kind:?}: expected one of {SUBJECT_KINDS}"
        ));
    }
    if kind == "company" {
        return match text {
            None => Ok(Subject::Company),
            Some(_) => Err("the company subject takes no id".into()),
        };
    }
    let t = text.ok_or_else(|| format!("a {kind} subject needs its id"))?;
    let sub = match kind {
        "account" => Subject::Account(resolve_account(store, &t)?),
        "asset" => Subject::Asset(resolve_asset_ref(store, &t)?),
        "party" => Subject::Party(resolve_party(store, &t)?),
        k => {
            let id = t
                .parse::<u64>()
                .map_err(|_| format!("a {k} subject's id is a number, got {t:?}"))?;
            Subject::from_parts(k, id).expect("kind checked")
        }
    };
    if !store.subject_exists(sub).map_err(|e| e.to_string())? {
        return Err(format!("{} doesn't exist", sub.describe()));
    }
    Ok(sub)
}

/// A subject for a tool's output, with human words for it.
fn subject_out(store: &Store, sub: ledger_store::Subject) -> SubjectOut {
    use ledger_store::Subject as S;
    let l = store.ledger();
    let label = match sub {
        S::Entry(i) => l
            .get_entry(i)
            .map(|e| format!("entry #{i} {}", e.description())),
        S::Account(i) => l
            .chart()
            .get(i)
            .map(|a| format!("account {} {}", a.code, a.name)),
        S::Asset(i) => l.get_asset(i).map(|a| match &a.tag {
            Some(t) => format!("asset {t} {}", a.description),
            None => format!("asset {i} {}", a.description),
        }),
        S::Party(i) => l.get_party(i).map(|p| format!("party {i} {}", p.label())),
        S::Period(i) => l
            .list_periods()
            .iter()
            .find(|p| p.id == i)
            .map(|p| format!("period {}", p.name)),
        S::PayrollRun(i) => l
            .get_payroll_run(i)
            .map(|r| format!("payroll run {i} of {}", r.date)),
        S::Schedule(i) => l
            .get_schedule(i)
            .map(|s| format!("schedule {i} {}", s.description)),
        S::Reconciliation(i) => store.reconciliations(None).ok().and_then(|rs| {
            rs.into_iter().find(|r| r.id == i).map(|r| {
                let code = l
                    .chart()
                    .get(r.account_id)
                    .map(|a| a.code.clone())
                    .unwrap_or_default();
                format!("reconciliation {i} of {code} through {}", r.statement_date)
            })
        }),
        S::Invoice(_) | S::Bill(_) | S::Company => None,
    }
    .unwrap_or_else(|| sub.describe());
    SubjectOut {
        kind: sub.kind().to_string(),
        id: (sub != S::Company).then(|| sub.id()),
        label,
    }
}

/// Everything an attachment is linked to, for a tool's output.
fn links_out(store: &Store, attachment_id: u64) -> Result<Vec<SubjectOut>, String> {
    Ok(store
        .attachment_links(attachment_id)
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|s| subject_out(store, s))
        .collect())
}

/// A stored attachment's metadata, by id.
fn find_attachment(store: &Store, id: u64) -> Result<ledger_store::Attachment, String> {
    store
        .list_attachments(None)
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|a| a.id == id)
        .ok_or_else(|| format!("unknown attachment {id}"))
}

/// An attachment for a listing, with its links.
fn receipt_out(store: &Store, a: ledger_store::Attachment) -> Result<ReceiptOut, String> {
    Ok(ReceiptOut {
        links: links_out(store, a.id)?,
        attachment_id: a.id,
        sha256: a.blob_sha,
        entry_id: a.entry_id,
        category: a.category,
        file_name: a.file_name,
        mime_type: a.mime_type,
        size_bytes: a.size_bytes,
        description: a.description,
        withdrawn: a.withdrawn_at.is_some(),
        withdrawn_reason: a.withdrawn_reason,
    })
}

/// Check every `link_to` subject before anything is filed: each resolves,
/// none twice, none the entry the file is filed on (all or nothing).
fn check_link_to(
    store: &Store,
    filed_on: Option<u64>,
    link_to: &[SubjectParam],
) -> Result<Vec<ledger_store::Subject>, String> {
    let mut out: Vec<ledger_store::Subject> = Vec::new();
    for s in link_to {
        let sub = resolve_subject(store, s)?;
        if out.contains(&sub) {
            return Err(format!("link_to names {} twice", sub.describe()));
        }
        if filed_on.map(ledger_store::Subject::Entry) == Some(sub) {
            return Err(format!(
                "link_to names {}, which it is filed on already",
                sub.describe()
            ));
        }
        out.push(sub);
    }
    Ok(out)
}

/// An account's code and name.
fn code_name(store: &Store, id: AccountId) -> (String, String) {
    store
        .ledger()
        .chart()
        .get(id)
        .map(|a| (a.code.clone(), a.name.clone()))
        .unwrap_or_default()
}

/// A recorded statement for a tool's output.
fn statement_out(store: &Store, st: &ledger_store::Statement) -> Result<StatementOut, String> {
    let (account_code, account_name) = code_name(store, st.account_id);
    let file_name = find_attachment(store, st.attachment_id)?.file_name;
    let reconciliation_id = store
        .reconciliations(Some(st.account_id))
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|r| r.attachment_id == st.attachment_id && r.voided.is_none())
        .map(|r| r.id);
    Ok(StatementOut {
        attachment_id: st.attachment_id,
        file_name,
        account_code,
        account_name,
        period_start: st.period_start.to_string(),
        period_end: st.period_end.to_string(),
        opening_balance: money(st.opening_balance),
        closing_balance: money(st.closing_balance),
        reconciliation_id,
    })
}

/// A recorded reconciliation for a tool's output.
fn reconciliation_out(store: &Store, r: &ledger_store::Reconciliation) -> ReconciliationOut {
    let (account_code, account_name) = code_name(store, r.account_id);
    ReconciliationOut {
        reconciliation_id: r.id,
        account_code,
        account_name,
        statement_date: r.statement_date.to_string(),
        statement_balance: money(r.statement_balance),
        statement_attachment: r.attachment_id,
        cleared: r
            .cleared
            .iter()
            .map(|&(entry_id, line_index)| ClearedRefOut {
                entry_id,
                line_index,
            })
            .collect(),
        voided: r.voided.is_some(),
        voided_reason: r.voided.clone(),
    }
}

/// The latest statement date an account is reconciled through
/// (reconciliations not voided), if any.
fn reconciled_through(store: &Store, account: AccountId) -> Result<Option<NaiveDate>, String> {
    Ok(store
        .reconciliations(Some(account))
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter(|r| r.voided.is_none())
        .map(|r| r.statement_date)
        .max())
}

/// A line of a reconciliation report.
fn reconcile_line(
    entry_id: u64,
    line_index: usize,
    date: NaiveDate,
    description: &str,
    side: Side,
    amount: Decimal,
) -> ReconcileLineOut {
    ReconcileLineOut {
        entry_id,
        line_index,
        date: date.to_string(),
        description: description.to_string(),
        side: match side {
            Side::Debit => "debit",
            Side::Credit => "credit",
        }
        .to_string(),
        amount: money(amount),
    }
}

/// The books dir: the data dir's parent, where wanderbooks.toml, the git
/// repository, and `vault/` live.
fn books_dir(store: &Store) -> std::path::PathBuf {
    let d = store.data_dir();
    let d = d.canonicalize().unwrap_or_else(|_| d.to_path_buf());
    d.parent().map(std::path::Path::to_path_buf).unwrap_or(d)
}

/// The last four digits of every TIN the books know of, from the journal
/// (each party's TIN and W-9 facts), never the vault: the filing scan's
/// probe bound (tin_files.rs INVARIANT 4).
fn known_tin_last4(store: &Store) -> Vec<u32> {
    store
        .ledger()
        .parties()
        .flat_map(|p| {
            [
                p.tin.as_ref().map(|t| t.last4.as_str()),
                p.w9.as_ref().map(|w| w.tin_last4.as_str()),
            ]
        })
        .flatten()
        .filter_map(|l| l.parse().ok())
        .collect()
}

/// Where these books keep what an import must never read
/// (local_files.rs): the data dir, the books dir, the vault, the key file
/// in force and the default one, and the home folder.
fn places(store: &Store, settings: &Settings) -> Places {
    Places {
        data_dir: store.data_dir().to_path_buf(),
        books_dir: books_dir(store),
        vault_dir: BooksServer::vault_dir(store),
        key_files: vec![settings.key_file(), ledger_vault::default_key_file()],
        home: std::env::var_os("HOME").map(std::path::PathBuf::from),
    }
}

/// The import roots a server for these books may use (local_files.rs
/// INVARIANT 0): a named root that is `/`, the home folder or holds it,
/// or holds the books, the vault or a key, is refused (the server doesn't
/// start); a default one is dropped, leaving none. The text is for the
/// human starting the server, on its standard error.
pub fn guard_import_roots(
    store: &Store,
    settings: &Settings,
    roots: ImportRoots,
) -> Result<ImportRoots, String> {
    let places = places(store, settings);
    let explicit = roots.explicit();
    let (roots, refused) = roots.guard(places.home.as_deref(), &places.keep_out());
    if let Some((root, why)) = refused.first() {
        let msg = format!(
            "import folder {} can't be used: {why}. Name a folder that holds none of them, such as your downloads folder",
            root.display()
        );
        if explicit {
            return Err(msg);
        }
        eprintln!("ledger-mcp: {msg}; books.import_local_file has no folder to read from");
    }
    Ok(roots)
}

/// Whether `path` is `dir` or inside it.
fn is_inside(dir: &std::path::Path, path: &std::path::Path) -> bool {
    resolved(path).starts_with(resolved(dir))
}

/// Where a tool may write a file holding a full TIN: a new file, in a
/// folder that exists, outside the books dir (which git commits).
fn outside_books(store: &Store, path: &str) -> Result<std::path::PathBuf, String> {
    let p = std::path::PathBuf::from(path.trim());
    if path.trim().is_empty() {
        return Err("give a path to write to".into());
    }
    if is_inside(&books_dir(store), &p) {
        return Err(INSIDE_BOOKS.to_string());
    }
    match p.parent() {
        Some(d) if d.as_os_str().is_empty() || d.is_dir() => {}
        _ => return Err(NO_FOLDER.to_string()),
    }
    if p.exists() {
        return Err(EXISTS.to_string());
    }
    Ok(p)
}

/// A path to write to whose folder doesn't exist.
const NO_FOLDER: &str = "the folder for that path doesn't exist: name a file in a folder that does";

/// A path to write to that is taken.
const EXISTS: &str = "a file is there already: name a new file";

/// Where `books.export_cpa_package` writes its zip: `to` itself when it
/// names a `.zip` file, or `file_name` in `to` when it is an existing
/// folder. A new file, in a folder that exists; with TINs, outside the
/// books dir (which git commits, `exports/` included).
fn package_dest(
    store: &Store,
    to: &str,
    file_name: &str,
    tins: bool,
) -> Result<std::path::PathBuf, String> {
    let raw = std::path::PathBuf::from(to.trim());
    if to.trim().is_empty() {
        return Err("give save_to: a new .zip path or an existing folder".into());
    }
    let dest = if raw.is_dir() {
        raw.join(file_name)
    } else if raw
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("zip"))
    {
        raw
    } else {
        return Err(format!(
            "save_to {} is neither an existing folder nor a .zip file",
            raw.display()
        ));
    };
    if tins {
        return outside_books(store, &dest.to_string_lossy()).map_err(|e| {
            format!("{e} (the CPA package holds full TINs; save it outside the books)")
        });
    }
    match dest.parent() {
        Some(d) if d.as_os_str().is_empty() || d.is_dir() => {}
        _ => return Err(NO_FOLDER.to_string()),
    }
    if dest.exists() {
        return Err(EXISTS.to_string());
    }
    Ok(dest)
}

/// Write a new file readable by its owner only (0600 on Unix).
fn write_private_new(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts
        .open(path)
        .map_err(|e| format!("couldn't create {}: {e}", path.display()))?;
    f.write_all(bytes)
        .and_then(|()| f.sync_all())
        .map_err(|e| format!("couldn't write {}: {e}", path.display()))
}

/// A TIN type from a tool: ssn, ein, or itin.
fn parse_tin_type(s: &str) -> Result<ledger_vault::TinType, String> {
    match s.trim().to_lowercase().as_str() {
        "ssn" => Ok(ledger_vault::TinType::Ssn),
        "ein" => Ok(ledger_vault::TinType::Ein),
        "itin" => Ok(ledger_vault::TinType::Itin),
        _ => Err("tin_type is ssn, ein, or itin".into()),
    }
}

/// A TIN type's wire name.
fn tin_type_key(t: ledger_vault::TinType) -> &'static str {
    match t {
        ledger_vault::TinType::Ssn => "ssn",
        ledger_vault::TinType::Ein => "ein",
        ledger_vault::TinType::Itin => "itin",
    }
}

/// The books' name for a vault TIN's type.
fn tin_kind(t: ledger_vault::TinType) -> ledger_core::TinType {
    match t {
        ledger_vault::TinType::Ssn => ledger_core::TinType::Ssn,
        ledger_vault::TinType::Ein => ledger_core::TinType::Ein,
        ledger_vault::TinType::Itin => ledger_core::TinType::Itin,
    }
}

/// The full TIN the vault holds for a party, checked against what the
/// books record of it; else why not, in words for a worksheet's note (the
/// note never holds more than a last four).
///
/// ```text
/// GOAL: a worksheet for the CPA carries a TIN only when the vault and the
///       books agree on it, and says why when it can't.
/// INVARIANTS:
///   1. The party's TIN facts, when on file, have the vault TIN's type and
///      last four, and the record's hash when they name one.
///   2. Its W-9 facts, when on file, have the vault TIN's last four; with
///      no TIN facts, the W-9 facts' vault record is the record's hash.
///   3. A vault that can't be opened or read, or holds no TIN for the
///      party, is a note, never an error (a package never fails on it).
/// ```
fn vault_tin(
    ledger: &Ledger,
    vault: &Result<ledger_vault::Vault, String>,
    party: ledger_core::PartyId,
) -> Result<ledger_vault::Tin, String> {
    let vault = vault.as_ref().map_err(String::clone)?;
    let t = vault
        .tin(party)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| {
            "TIN not in the vault: record it with books.set_tin (or books.set_w9 tin, from the W-9)"
                .to_string()
        })?;
    let hash = vault.record_hash(party).ok().flatten();
    let p = ledger.get_party(party);
    let (facts, w9) = (
        p.and_then(|p| p.tin.as_ref()),
        p.and_then(|p| p.w9.as_ref()),
    );
    if facts.is_none() && w9.is_none() {
        return Err(
            "the vault holds a TIN but the books have no record of it: record it again with books.set_tin"
                .into(),
        );
    }
    if let Some(f) = facts {
        if f.last4 != t.last4() || f.tin_type != tin_kind(t.kind) {
            return Err(format!(
                "the vault's TIN ({}, ending {}) isn't the one the books record ({}, ending {}): record the right one with books.set_tin",
                tin_type_key(t.kind),
                t.last4(),
                f.tin_type.key(),
                f.last4
            ));
        }
        if f.vault_record.is_some() && f.vault_record != hash {
            return Err("the vault record isn't the one the books recorded: record the TIN again with books.set_tin".into());
        }
    }
    if let Some(f) = w9 {
        if f.tin_last4 != t.last4() {
            return Err(format!(
                "the vault's TIN ends in {}, the W-9 facts say {}: check the W-9 and fix with books.set_w9",
                t.last4(),
                f.tin_last4
            ));
        }
        if facts.is_none() && f.vault_record.is_some() && f.vault_record != hash {
            return Err("the vault record isn't the one the books recorded: check the W-9 and record it again with books.set_w9 tin".into());
        }
    }
    Ok(t)
}

/// Parse role names (`Role::key`), refusing any unknown one.
fn parse_roles(roles: &[String]) -> Result<Vec<ledger_core::Role>, String> {
    roles
        .iter()
        .map(|r| {
            ledger_core::Role::parse(r.trim()).ok_or_else(|| {
                format!(
                    "unknown role {r:?}: expected one of {}",
                    ledger_core::Role::ALL.map(|r| r.key()).join(", ")
                )
            })
        })
        .collect()
}

/// A party's roles (vendor and customer included), in display order.
fn party_roles(p: &ledger_core::Party) -> Vec<String> {
    let all = p.all_roles();
    ledger_core::Role::ALL
        .into_iter()
        .filter(|r| all.contains(r))
        .map(|r| r.key().to_string())
        .collect()
}

/// A party's facts for a tool's output.
fn party_out(p: &ledger_core::Party) -> PartyOut {
    PartyOut {
        party_id: p.id,
        name: p.name.clone(),
        label: p.label().to_string(),
        display_name: p.display_name.clone(),
        roles: party_roles(p),
        website: p.website.clone(),
        address: p.address.clone(),
        notes: p.notes.clone(),
        archived: p.archived,
        vendor_id: p.vendor,
        customer_id: p.customer,
        w9: p.w9.as_ref().map(|w| W9Out {
            received_on: w.received.to_string(),
            entity_type: w.entity_type.clone(),
            exempt_payee_code: w.exempt_payee_code.clone(),
            fatca_code: w.fatca_code.clone(),
            tin_last4: w.tin_last4.clone(),
            kept_at: w.kept_at.clone(),
            in_vault: w.vault_record.is_some(),
        }),
        w4: p.w4.as_ref().map(|w| W4Out {
            received_on: w.received.to_string(),
            state: w.state.clone(),
            kept_at: w.kept_at.clone(),
        }),
        tin: p.tin.as_ref().map(|t| TinOut {
            tin_type: t.tin_type.key().to_string(),
            last4: t.last4.clone(),
            in_vault: t.vault_record.is_some(),
            purged: t.purged.clone(),
        }),
    }
}

/// An optional party text from a tool: trimmed, and "" clears it.
fn optional_text(s: &str) -> Option<String> {
    let t = s.trim();
    (!t.is_empty()).then(|| t.to_string())
}

/// A document's status for a tool's output.
fn status_key(s: DocumentStatus) -> String {
    match s {
        DocumentStatus::Open => "open",
        DocumentStatus::Partial => "partial",
        DocumentStatus::Paid => "paid",
    }
    .to_string()
}

/// An invoice or bill for `books.party`: its (date, due date), (total,
/// paid so far), status, and entry.
fn party_document(
    id: u64,
    (date, due): (NaiveDate, NaiveDate),
    (total, balance, paid): (
        Result<Decimal, ledger_core::LedgerError>,
        Result<Decimal, ledger_core::LedgerError>,
        Decimal,
    ),
    status: DocumentStatus,
    entry_id: u64,
) -> Result<PartyDocumentOut, String> {
    Ok(PartyDocumentOut {
        id,
        date: date.to_string(),
        due_date: due.to_string(),
        total: money(total.map_err(|e| e.to_string())?),
        paid: money(paid),
        balance: money(balance.map_err(|e| e.to_string())?),
        status: status_key(status),
        entry_id,
    })
}

/// The sum of `values`, each computed or refused: refused (as text) when
/// one is, or when the sum is past exact arithmetic. Figures an earlier
/// version journaled are as journaled (rules.md B-28).
fn checked_sum(
    values: impl IntoIterator<Item = Result<Decimal, ledger_core::LedgerError>>,
    what: &str,
) -> Result<Decimal, String> {
    values.into_iter().try_fold(Decimal::ZERO, |sum, v| {
        let v = v.map_err(|e| e.to_string())?;
        sum.checked_add(v).ok_or_else(|| {
            ledger_core::LedgerError::TooLargeToCompute(what.to_string()).to_string()
        })
    })
}

/// Everything the books did with one party (`PartyTotalsOut` defines it).
struct PartyActivity {
    /// The party's entries, drafts included.
    entries: std::collections::BTreeSet<u64>,
    /// Paid to it in the year.
    paid: Decimal,
    /// Received from it in the year.
    received: Decimal,
}

/// The party's entries and the year's money paid and received
/// (`PartyTotalsOut`): ledger_core::parties::activity_with over the
/// calendar year, with the `[books] cash_accounts` and the members (so an
/// entry naming several parties counts this one's share).
fn party_activity(
    store: &Store,
    party: &ledger_core::Party,
    settings: &Settings,
    year: i32,
) -> Result<PartyActivity, String> {
    let ledger = store.ledger();
    let cash = cash_ids(ledger, &settings.roles.cash);
    let span = (
        NaiveDate::from_ymd_opt(year, 1, 1).ok_or("year out of range")?,
        NaiveDate::from_ymd_opt(year, 12, 31).ok_or("year out of range")?,
    );
    let members = settings.company.party_members(ledger);
    let a = ledger_core::parties::activity_with(ledger, party.id, span, &cash, &members);
    Ok(PartyActivity {
        entries: a.entries,
        paid: a.paid,
        received: a.received,
    })
}

/// The `party_required` refusal for an engine-posted entry (payroll, a
/// remittance, a disposal) that names no counterparty under `[books]
/// parties = "require"`: `what` says what to pass.
fn party_required(what: &str) -> String {
    serde_json::json!({
        "code": "party_required",
        "message": format!(
            "these books require a counterparty on every entry (their parties setting is \"require\"): {what} (add the party with books.add_party first if it's new)"
        ),
    })
    .to_string()
}

/// The chart's ids for these account codes (codes it lacks are skipped).
fn cash_ids(ledger: &Ledger, codes: &[String]) -> Vec<AccountId> {
    codes
        .iter()
        .filter_map(|c| ledger.chart().find_by_code(c).map(|a| a.id))
        .collect()
}

/// What party shares resolve with ([`ledger_core::shares::resolve`]): the
/// money accounts over the `[books] cash_accounts`, and the members.
fn share_context(
    ledger: &Ledger,
    settings: &Settings,
) -> (
    ledger_core::parties::MoneyAccounts,
    Vec<ledger_core::PartyMember>,
) {
    (
        ledger_core::parties::money_accounts(ledger, &cash_ids(ledger, &settings.roles.cash)),
        settings.company.party_members(ledger),
    )
}

/// `parties` params as shares: each party resolved, `amount` or `percent`
/// parsed (not both), neither meaning derived.
fn parse_shares(
    store: &Store,
    params: &[PartyShareParam],
) -> Result<Vec<ledger_core::PartyShare>, String> {
    params
        .iter()
        .map(|p| {
            let party = resolve_party(store, &p.party)?;
            let share = match (&p.amount, &p.percent) {
                (Some(_), Some(_)) => {
                    return Err(format!(
                        "party {}: give an amount or a percent, not both",
                        p.party
                    ));
                }
                (Some(a), None) => ledger_core::Share::Amount(parse_amount(a)?),
                (None, Some(pc)) => ledger_core::Share::Percent(parse_amount(pc)?),
                (None, None) => ledger_core::Share::Derived,
            };
            Ok(ledger_core::PartyShare { party, share })
        })
        .collect()
}

/// Whether an entry's lines are final enough to check shares against:
/// posted, or a draft with lines whose debits equal its credits.
fn lines_settled(entry: &JournalEntry) -> bool {
    if entry.status() == ledger_core::EntryStatus::Posted {
        return true;
    }
    // A draft's lines are as journaled, of any size (rules.md B-28): sides
    // past exact arithmetic are not settled lines.
    let side = |s: Side| -> Option<Decimal> {
        entry
            .lines()
            .iter()
            .filter(|l| l.side == s)
            .try_fold(Decimal::ZERO, |sum, l| sum.checked_add(l.amount.amount))
    };
    match (side(Side::Debit), side(Side::Credit)) {
        (Some(debits), Some(credits)) => !entry.lines().is_empty() && debits == credits,
        _ => false,
    }
}

/// Refuse shares that don't resolve against the entry's lines, once the
/// lines are settled ([`lines_settled`]).
fn check_shares_resolve(
    store: &Store,
    settings: &Settings,
    entry_id: u64,
    shares: &[ledger_core::PartyShare],
) -> Result<(), String> {
    let ledger = store.ledger();
    let entry = ledger
        .get_entry(entry_id)
        .ok_or_else(|| format!("unknown entry {entry_id}"))?;
    if !lines_settled(entry) {
        return Ok(());
    }
    let (money, members) = share_context(ledger, settings);
    ledger_core::shares::resolve(entry, shares, &money, &members).map(|_| ())
}

/// Fix a posted entry's derived split among several parties: journal the
/// exact amounts they resolve to now, so a later change to
/// `[[company.members]]` (a new income split, a member archived) never
/// re-splits posted history.
///
/// ```text
/// GOAL: posted is forever, the split between its parties included.
/// SOURCE: AGENTS.md ("Posted is forever"); docs/parties-and-attachments.md
///         "Shares"; ledger_core::shares INVARIANT 10 (derived several:
///         the members' capital lines, else their income shares, both read
///         from the settings).
/// INVARIANTS:
///   1. Only a posted entry naming two or more parties with derived shares
///      changes: its shares become Amount(|part|) for each party, in
///      order, journaled as entry.parties_set (the reversal chain too), so
///      each party's resolved part is exactly what it was.
///   2. Nothing changes when the shares don't resolve, or a part is zero
///      (an amount must be positive): the entry keeps its derived shares.
/// ```
fn fix_derived_shares(store: &mut Store, settings: &Settings, entry_id: u64) -> Result<(), String> {
    let ledger = store.ledger();
    let Some(entry) = ledger.get_entry(entry_id) else {
        return Ok(());
    };
    let shares = ledger.entry_parties(entry_id);
    if entry.status() != ledger_core::EntryStatus::Posted
        || shares.len() < 2
        || !shares
            .iter()
            .any(|s| matches!(s.share, ledger_core::Share::Derived))
    {
        return Ok(());
    }
    let (money, members) = share_context(ledger, settings);
    let Ok(parts) = ledger_core::shares::resolve(entry, shares, &money, &members) else {
        return Ok(());
    };
    if parts.iter().any(|(_, v)| v.is_zero()) {
        return Ok(());
    }
    let fixed: Vec<ledger_core::PartyShare> = parts
        .into_iter()
        .map(|(party, v)| ledger_core::PartyShare {
            party,
            share: ledger_core::Share::Amount(v.abs()),
        })
        .collect();
    store
        .set_entry_parties(entry_id, &fixed)
        .map_err(|e| e.to_string())
}

/// An entry's parties for output, each share resolved, and why they don't
/// resolve when they don't.
fn entry_parties_out(
    store: &Store,
    settings: &Settings,
    entry: &JournalEntry,
) -> (Vec<EntryPartyOut>, Option<String>) {
    let ledger = store.ledger();
    let shares = ledger.entry_parties(entry.id());
    let (accounts, members) = share_context(ledger, settings);
    let (resolved, error) = match ledger_core::shares::resolve(entry, shares, &accounts, &members) {
        Ok(r) => (Some(r), None),
        Err(e) => (None, Some(e)),
    };
    let out = shares
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let (kind, amount, percent) = match s.share {
                ledger_core::Share::Amount(a) => ("amount", Some(money(a)), None),
                ledger_core::Share::Percent(p) => {
                    ("percent", None, Some(p.normalize().to_string()))
                }
                ledger_core::Share::Derived => ("derived", None, None),
            };
            EntryPartyOut {
                party_id: s.party,
                party: ledger
                    .get_party(s.party)
                    .map(|p| p.label().to_string())
                    .unwrap_or_else(|| format!("party {}", s.party)),
                share: kind.to_string(),
                amount,
                percent,
                resolved: resolved.as_ref().map(|r| money(r[i].1)),
            }
        })
        .collect();
    (out, error)
}

/// The party's customer record's invoices, by id.
fn party_invoices<'a>(
    ledger: &'a Ledger,
    party: &ledger_core::Party,
) -> Vec<&'a ledger_core::Invoice> {
    let Some(c) = party.customer else {
        return Vec::new();
    };
    ledger
        .invoices()
        .into_iter()
        .filter(|i| i.customer_id == c)
        .collect()
}

/// The party's vendor record's bills, by id.
fn party_bills<'a>(ledger: &'a Ledger, party: &ledger_core::Party) -> Vec<&'a ledger_core::Bill> {
    let Some(v) = party.vendor else {
        return Vec::new();
    };
    ledger
        .bills()
        .into_iter()
        .filter(|b| b.vendor_id == v)
        .collect()
}

/// This calendar year.
fn this_year() -> i32 {
    chrono::Datelike::year(&today())
}

fn form_1099_label(form: Option<ledger_core::Form1099>) -> Option<String> {
    form.map(|f| f.label().to_string())
}

/// Resolve a class by id or name; a number that is one class's id and
/// another's name is refused (INVARIANT 6).
fn resolve_class(store: &Store, class: &str) -> Result<u32, String> {
    let by_id = class
        .parse::<u32>()
        .ok()
        .filter(|id| store.ledger().get_class(*id).is_some());
    let by_name = store.ledger().find_class_by_name(class).map(|c| c.id);
    match (by_id, by_name) {
        (Some(i), Some(n)) if i != n => Err(format!(
            "class {class:?} is ambiguous: it is class {i}'s id and class {n}'s name; pass the id"
        )),
        (Some(i), _) | (None, Some(i)) => Ok(i),
        (None, None) => Err(format!(
            "unknown class {class:?}: use an id or name from books.list_classes"
        )),
    }
}

/// A money amount as a decimal string with at least two places: "0.00",
/// "207.68", "1000.00". Padding only, never rounding: an exact amount with
/// more places (an FX-converted line) keeps them. Never "-0".
fn money(n: Decimal) -> String {
    let mut n = if n.is_zero() { Decimal::ZERO } else { n };
    if n.scale() < 2 {
        n.rescale(2);
    }
    n.to_string()
}

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

/// Preview lines with resulting balances for an entry.
fn preview_lines(store: &Store, entry: &JournalEntry) -> Result<Vec<PreviewLine>, String> {
    let lines: Vec<(AccountId, Side, Money)> = entry
        .lines()
        .iter()
        .map(|l| (l.account, l.side, l.amount))
        .collect();
    preview_lines_for(store, &lines)
}

fn preview_receipts(store: &Store, entry_id: u64) -> Result<Vec<PreviewReceipt>, String> {
    Ok(store
        .list_attachments(Some(entry_id))
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|a| PreviewReceipt {
            attachment_id: a.id,
            file_name: a.file_name,
            sha256: a.blob_sha,
        })
        .collect())
}

/// Build the cash flow statement from account codes, as the exports need it:
/// `start` defaults to the beginning of time, `end` to today, and at least
/// one cash account is required (the statement reconciles to them).
fn cash_flow_statement(
    ledger: &Ledger,
    start: Option<&str>,
    end: Option<&str>,
    cash_codes: &[String],
    financing_codes: &[String],
    investing_codes: &[String],
) -> Result<ledger_core::reports::CashFlowStatement, String> {
    let resolve = |codes: &[String], what: &str| -> Result<Vec<AccountId>, String> {
        codes
            .iter()
            .map(|code| {
                ledger
                    .chart()
                    .find_by_code(code)
                    .map(|a| a.id)
                    .ok_or_else(|| format!("unknown {what} account code: {code}"))
            })
            .collect()
    };
    let cash_accounts = resolve(cash_codes, "cash")?;
    if cash_accounts.is_empty() {
        return Err(
            "cash_accounts is required for the cash flow statement (pass it, or set the books' cash accounts setting: books.settings shows it)"
                .to_string(),
        );
    }
    let financing_accounts = resolve(financing_codes, "financing")?;
    let investing_accounts = resolve(investing_codes, "investing")?;
    let (start, end) = statement_range(ledger, start, end)?;
    ledger_core::cash_flow(
        ledger,
        &ledger_core::CashFlowParams {
            start,
            end,
            cash_accounts,
            financing_accounts,
            investing_accounts,
        },
    )
    .map_err(|e| e.to_string())
}

/// Net movement of one account over entries in `[from, to]`, normal side.
fn movement_in(ledger: &Ledger, account: AccountId, from: NaiveDate, to: NaiveDate) -> Decimal {
    let movement: Decimal = ledger
        .posted_entries()
        .filter(|e| e.date() >= from && e.date() <= to)
        .flat_map(|e| e.lines())
        .filter(|l| l.account == account)
        .map(|l| match l.side {
            Side::Debit => l.amount.amount,
            Side::Credit => -l.amount.amount,
        })
        .sum();
    let account = ledger.chart().get(account).expect("account exists");
    match account.account_type.normal_side() {
        Side::Debit => movement,
        Side::Credit => -movement,
    }
}

thread_local! {
    /// The company's time zone for the request running on this thread
    /// (set at its start from the settings in force: one version per
    /// request), so every "today" in it agrees.
    static TIMEZONE: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
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

/// Resolve a statement's date range: `end` defaults to today and `start` to
/// the first posted entry (where "from the beginning" really begins, so no
/// statement says "0001-01-01"), or to `end` for empty books.
fn statement_range(
    ledger: &Ledger,
    start: Option<&str>,
    end: Option<&str>,
) -> Result<(NaiveDate, NaiveDate), String> {
    let end = match end {
        Some(s) => parse_date(s)?,
        None => today(),
    };
    let start = match start {
        Some(s) => parse_date(s)?,
        None => ledger
            .posted_entries()
            .map(|e| e.date())
            .min()
            .map_or(end, |first| first.min(end)),
    };
    if end < start {
        return Err("end is before start".to_string());
    }
    Ok((start, end))
}

fn schedule_out(ledger: &Ledger, s: &ledger_core::AmortizationSchedule) -> ScheduleOut {
    // A schedule an earlier version journaled may run past the last date
    // chrono holds (rules.md B-28): shown, with why its plan can't be.
    let plan = s.plan();
    let code = |id| {
        ledger
            .chart()
            .get(id)
            .map(|a| a.code.clone())
            .unwrap_or_default()
    };
    ScheduleOut {
        schedule_id: s.id,
        description: s.description.clone(),
        kind: match s.kind {
            ledger_core::ScheduleKind::PrepaidExpense => "prepaid_expense",
            ledger_core::ScheduleKind::DeferredRevenue => "deferred_revenue",
        }
        .to_string(),
        balance_account: code(s.balance_account),
        pl_account: code(s.pl_account),
        total: money(s.total.amount),
        remaining: plan.as_ref().ok().and(s.remaining().ok()).map(money),
        months: plan
            .as_ref()
            .map(|plan| {
                plan.iter()
                    .map(|(d, a)| ScheduleMonthOut {
                        date: d.to_string(),
                        amount: money(*a),
                        posted: s.posted.contains(d),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        // A null remaining is always explained (ScheduleOut::plan_error).
        plan_error: plan
            .err()
            .or_else(|| s.remaining().err())
            .map(|e| e.to_string()),
    }
}

fn recurring_out(r: &ledger_core::RecurringEntry) -> RecurringOut {
    RecurringOut {
        recurring_id: r.id,
        description: r.description.clone(),
        frequency: format!("{:?}", r.frequency).to_lowercase(),
        start: r.start.to_string(),
        end: r.end.map(|e| e.to_string()),
        generated: r.generated.iter().map(|d| d.to_string()).collect(),
    }
}

/// Profit and loss for a range (see [`statement_range`]), optionally one class.
fn pnl_statement(
    ledger: &Ledger,
    start: Option<&str>,
    end: Option<&str>,
    class_id: Option<ledger_core::ClassId>,
) -> Result<ledger_core::ProfitAndLossStatement, String> {
    let (start, end) = statement_range(ledger, start, end)?;
    ledger_core::profit_and_loss_for_period(ledger, start, end, class_id).map_err(|e| e.to_string())
}

/// Balance sheet as of a date (default today).
fn bs_statement(
    ledger: &Ledger,
    as_of: Option<&str>,
) -> Result<ledger_core::BalanceSheetStatement, String> {
    let as_of = match as_of {
        Some(s) => parse_date(s)?,
        None => today(),
    };
    ledger_core::balance_sheet_as_of(ledger, as_of).map_err(|e| e.to_string())
}

fn lines_out(lines: &[ledger_core::StatementLine]) -> Vec<StatementLineOut> {
    lines
        .iter()
        .map(|l| StatementLineOut {
            code: l.code.clone(),
            name: l.name.clone(),
            amount: money(l.amount),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Tools
// ---------------------------------------------------------------------------

#[path = "tools/kept.rs"]
mod kept;
pub use kept::SettingKeyOut;

#[tool_router(router = core_router)]
impl BooksServer {
    #[tool(
        name = "books.handbook",
        title = "Bookkeeping handbook",
        description = "The guide to keeping these books: every procedure and rule, by section. Read the contract section first, then each section you need, before bookkeeping. Without section: the index; with section: that section's text (also at books://handbook/{id}); section \"all\": the whole handbook.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn handbook(
        &self,
        params: Parameters<HandbookParams>,
    ) -> Result<Json<HandbookOut>, String> {
        let settings = self.settings();
        let sections = handbook::SECTIONS
            .iter()
            .map(|(id, _)| {
                let text = handbook::section(id, &settings, self.currency()).unwrap_or_default();
                HandbookSectionOut {
                    id: id.to_string(),
                    title: handbook::title(&text).to_string(),
                    uri: format!("{}/{id}", handbook::URI),
                    chars: text.chars().count(),
                }
            })
            .collect();
        let section = match params.0.section.as_deref().map(str::trim) {
            None => None,
            Some(id) => {
                let text = handbook::section(id, &settings, self.currency()).ok_or_else(|| {
                    let ids: Vec<&str> = handbook::SECTIONS.iter().map(|(id, _)| *id).collect();
                    format!(
                        "no handbook section {id:?}: the sections are {}, or \"{}\" for all of them",
                        ids.join(", "),
                        handbook::ALL
                    )
                })?;
                let title = if id == handbook::ALL {
                    "The whole handbook".to_string()
                } else {
                    handbook::title(&text).to_string()
                };
                Some(HandbookTextOut {
                    id: id.to_string(),
                    title,
                    text,
                })
            }
        };
        Ok(Json(HandbookOut {
            read_first: "Read the contract section first: the rules every entry follows. Then each section you need, or section \"all\" for the whole handbook.".to_string(),
            sections,
            section,
        }))
    }

    #[tool(
        name = "books.chart",
        title = "Chart of accounts",
        description = "List the chart of accounts with current balances. Start here: every line you post must use a real account from this chart.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn chart(&self) -> Result<Json<ChartOut>, String> {
        let store = self.lock()?;
        let ledger = store.ledger();
        let mut accounts = Vec::new();
        let mut all: Vec<_> = ledger.chart().all().collect();
        all.sort_by(|a, b| a.code.cmp(&b.code));
        for a in all {
            let balance = ledger.account_balance(a.id).map_err(|e| e.to_string())?;
            accounts.push(ChartAccount {
                id: a.id,
                code: a.code.clone(),
                name: a.name.clone(),
                account_type: format!("{:?}", a.account_type).to_lowercase(),
                parent: a.parent,
                placeholder: a.placeholder,
                balance: money(if balance.is_zero() {
                    Decimal::ZERO
                } else {
                    balance
                }),
                tax_category: a.tax_category.clone(),
                deductible_percent: a.deductible_percent.map(|p| p.to_string()),
            });
        }
        Ok(Json(ChartOut {
            accounts,
            base_currency: store.base_currency().code().to_string(),
        }))
    }

    #[tool(
        name = "books.add_account",
        title = "Add an account",
        description = "Add an account to the chart. Codes must be unique; check books.chart first.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn add_account(
        &self,
        params: Parameters<AddAccountParams>,
    ) -> Result<Json<AddAccountOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let id = store
            .add_account(
                &p.code,
                &p.name,
                parse_account_type(&p.account_type)?,
                p.parent,
                p.placeholder.unwrap_or(false),
            )
            .map_err(|e| e.to_string())?;
        Ok(Json(AddAccountOut { account_id: id }))
    }

    #[tool(
        name = "books.trial_balance",
        title = "Trial balance",
        description = "The trial balance as of a date (default today): every account with a non-zero balance, debit and credit columns and totals. Debits must equal credits.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn trial_balance(
        &self,
        params: Parameters<TrialBalanceParams>,
    ) -> Result<Json<TrialBalanceOut>, String> {
        let store = self.lock()?;
        let as_of = match &params.0.as_of {
            Some(s) => parse_date(s)?,
            None => today(),
        };
        let tb = reports::trial_balance_as_of(store.ledger(), as_of);
        Ok(Json(TrialBalanceOut {
            as_of: as_of.to_string(),
            lines: tb
                .lines
                .iter()
                .map(|l| TrialBalanceLineOut {
                    code: l.code.clone(),
                    name: l.name.clone(),
                    debit: money(l.debit),
                    credit: money(l.credit),
                })
                .collect(),
            total_debits: money(tb.total_debits),
            total_credits: money(tb.total_credits),
            balanced: tb.balanced(),
        }))
    }

    #[tool(
        name = "books.profit_and_loss",
        title = "Profit and loss",
        description = "Profit and loss for a period: revenue, expenses, net income. Dates are YYYY-MM-DD; omit for all time. Pass class (name or id) to filter to one division.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn profit_and_loss(
        &self,
        params: Parameters<ProfitAndLossParams>,
    ) -> Result<Json<ProfitAndLossOut>, String> {
        let p = params.0;
        let store = self.lock()?;
        let class_id = match &p.class {
            Some(c) => Some(resolve_class(&store, c)?),
            None => None,
        };
        let pl = pnl_statement(
            store.ledger(),
            p.start.as_deref(),
            p.end.as_deref(),
            class_id,
        )?;
        Ok(Json(ProfitAndLossOut {
            start: pl.start.to_string(),
            end: pl.end.to_string(),
            revenue: money(pl.total_revenue),
            expenses: money(pl.total_expenses),
            net_income: money(pl.net_income()),
            revenue_accounts: lines_out(&pl.revenue),
            expense_accounts: lines_out(&pl.expenses),
        }))
    }

    #[tool(
        name = "books.balance_sheet",
        title = "Balance sheet",
        description = "The balance sheet as of a date: assets, liabilities, equity (including current earnings). Omit as_of for today.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn balance_sheet(
        &self,
        params: Parameters<BalanceSheetParams>,
    ) -> Result<Json<BalanceSheetOut>, String> {
        let p = params.0;
        let store = self.lock()?;
        let bs = bs_statement(store.ledger(), p.as_of.as_deref())?;
        Ok(Json(BalanceSheetOut {
            as_of: bs.as_of.to_string(),
            assets: money(bs.total_assets),
            liabilities: money(bs.total_liabilities),
            equity: money(bs.total_equity),
            current_earnings: money(bs.current_earnings),
            balanced: bs.balanced(),
            asset_accounts: lines_out(&bs.assets),
            liability_accounts: lines_out(&bs.liabilities),
            equity_accounts: lines_out(&bs.equity),
        }))
    }

    #[tool(
        name = "books.cash_flow",
        title = "Cash flow statement",
        description = "Cash flow statement (indirect method) for a period: net income, depreciation add-back, working-capital changes, investing (asset purchases, disposal proceeds), financing. The three sections reconcile to the net change in the given cash accounts.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn cash_flow(
        &self,
        params: Parameters<CashFlowParams>,
    ) -> Result<Json<CashFlowOut>, String> {
        let p = params.0;
        let store = self.lock()?;
        let ledger = store.ledger();
        let stmt = self.cash_flow_for(
            ledger,
            p.start.as_deref(),
            p.end.as_deref(),
            &p.cash_accounts,
            &p.financing_accounts,
            &p.investing_accounts,
        )?;
        let line = |l: &ledger_core::CashFlowLine| CashFlowLineOut {
            label: l.label.clone(),
            amount: money(l.amount),
        };
        Ok(Json(CashFlowOut {
            start: stmt.start.to_string(),
            end: stmt.end.to_string(),
            net_income: money(stmt.net_income),
            depreciation: money(stmt.depreciation),
            working_capital: stmt.working_capital.iter().map(line).collect(),
            cash_from_operations: money(stmt.cash_from_operations),
            investing: stmt.investing.iter().map(line).collect(),
            cash_from_investing: money(stmt.cash_from_investing),
            financing: stmt.financing.iter().map(line).collect(),
            cash_from_financing: money(stmt.cash_from_financing),
            net_change_in_cash: money(stmt.net_change_in_cash),
            cash_beginning: money(stmt.cash_beginning),
            cash_ending: money(stmt.cash_ending),
        }))
    }

    #[tool(
        name = "books.draft_entry",
        title = "Draft an entry",
        description = "Start a draft journal entry. Drafts are journaled and survive restarts, but don't affect reports until posted; add lines with books.add_line. Pass party (id or name) for the counterparty: every entry with a vendor, platform, agency, bank, member, or contractor names its party (a party with a vendor record tags the vendor too, for 1099s). An entry with several counterparties passes parties instead, each with an amount, a percent, or neither (derived): name both members on a joint member transaction and let their shares derive.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn draft_entry(
        &self,
        params: Parameters<DraftEntryParams>,
    ) -> Result<Json<DraftEntryOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        // Resolve every reference before writing anything (INVARIANT 2).
        let date = parse_date(&p.date)?;
        let vendor = p
            .vendor
            .as_deref()
            .map(|v| resolve_vendor(&store, v))
            .transpose()?;
        let party = p
            .party
            .as_deref()
            .map(|r| resolve_party(&store, r))
            .transpose()?;
        let no_party = p
            .no_party_reason
            .as_deref()
            .map(str::trim)
            .filter(|r| !r.is_empty());
        let shares = p
            .parties
            .as_deref()
            .map(|ps| parse_shares(&store, ps))
            .transpose()?;
        if no_party.is_some() && (party.is_some() || vendor.is_some() || shares.is_some()) {
            return Err(
                "no_party_reason is for an entry with no counterparty: pass it without party, parties, or vendor".into(),
            );
        }
        if shares.is_some() && (party.is_some() || vendor.is_some()) {
            return Err(
                "pass party (one) or parties (several, with shares), not both; a vendor party in parties tags its vendor record".into(),
            );
        }
        if let Some(sh) = &shares {
            store
                .ledger()
                .check_party_shares(sh)
                .map_err(|e| e.to_string())?;
        }
        if let (Some(v), Some(pid)) = (vendor, party) {
            let party_vendor = store.ledger().get_party(pid).and_then(|p| p.vendor);
            if party_vendor != Some(v) {
                return Err(format!(
                    "vendor {v} isn't party {pid}'s vendor record: pass only the party (its vendor record is tagged with it), or the vendor alone"
                ));
            }
        }
        let id = store
            .create_entry(date, &p.description)
            .map_err(|e| e.to_string())?;
        if let Some(sh) = shares.filter(|sh| !sh.is_empty()) {
            store
                .set_entry_parties(id, &sh)
                .map_err(|e| e.to_string())?;
        } else if party.is_some() {
            store
                .set_entry_party(id, party)
                .map_err(|e| e.to_string())?;
        } else if vendor.is_some() {
            store
                .set_entry_vendor(id, vendor)
                .map_err(|e| e.to_string())?;
        } else if let Some(r) = no_party {
            store.set_entry_no_party(id, r).map_err(|e| e.to_string())?;
        }
        Ok(Json(DraftEntryOut {
            entry_id: id,
            status: "draft".to_string(),
        }))
    }

    #[tool(
        name = "books.add_line",
        title = "Add a line to a draft",
        description = "Add one line to a draft entry. Amount is a decimal string, never a float. The entry must balance before posting. For foreign-currency lines, pass foreign_currency, foreign_amount, and fx_rate (foreign -> base); the base amount is computed as foreign_amount * fx_rate. Pass class (name or id) to tag the line for divisional tracking.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn add_line(
        &self,
        params: Parameters<AddLineParams>,
    ) -> Result<Json<AddLineOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let account = resolve_account(&store, &p.account)?;
        let side = parse_side(&p.side)?;
        // Resolve the class before writing anything (INVARIANT 2).
        let class_id = match &p.class {
            Some(c) => Some(resolve_class(&store, c)?),
            None => None,
        };
        // Foreign-currency line?
        if p.foreign_currency.is_some() || p.foreign_amount.is_some() || p.fx_rate.is_some() {
            let fc = p
                .foreign_currency
                .ok_or("foreign_currency is required for fx lines")?;
            let fa = p
                .foreign_amount
                .ok_or("foreign_amount is required for fx lines")?;
            let rate = p.fx_rate.ok_or("fx_rate is required for fx lines")?;
            let currency = Currency::new(&fc).map_err(|e| e.to_string())?;
            let foreign_amount = parse_amount(&fa)?;
            let fx_rate = parse_amount(&rate)?;
            store
                .add_line_fx(p.entry_id, account, side, currency, foreign_amount, fx_rate)
                .map_err(|e| e.to_string())?;
        } else {
            let amount = parse_amount(&p.amount)?;
            if amount <= Decimal::ZERO {
                return Err("amount must be positive".to_string());
            }
            let currency = store.base_currency();
            store
                .add_line(p.entry_id, account, side, Money::new(amount, currency))
                .map_err(|e| e.to_string())?;
        }
        // Class tag, if requested. The line just added is the last one.
        if let Some(class_id) = class_id {
            let line_index = store
                .ledger()
                .get_entry(p.entry_id)
                .ok_or_else(|| format!("unknown entry {}", p.entry_id))?
                .lines()
                .len()
                .checked_sub(1)
                .ok_or_else(|| "entry has no lines".to_string())?;
            store
                .set_line_class(p.entry_id, line_index, Some(class_id))
                .map_err(|e| e.to_string())?;
        }
        Ok(Json(AddLineOut { ok: true }))
    }

    #[tool(
        name = "books.post_entry",
        title = "Post an entry",
        description = "Post a draft entry. Without confirmed=true this returns a preview (lines, resulting balances, receipt checklist) and posts nothing. With confirmed=true it posts after enforcing the receipt policy.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn post_entry(
        &self,
        params: Parameters<PostEntryParams>,
    ) -> Result<Json<PostEntryOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let entry = store
            .ledger()
            .get_entry(p.entry_id)
            .ok_or_else(|| format!("unknown entry {}", p.entry_id))?
            .clone();
        if entry.status() != ledger_core::EntryStatus::Draft {
            return Err(format!("entry {} is not a draft", p.entry_id));
        }
        // Pure validation before anything is journaled.
        entry
            .validate(store.ledger().chart())
            .map_err(|e| e.to_string())?;
        // Validated: its lines are in a line's range, so their sum is exact.
        let total = ReceiptPolicy::entry_debits(&entry).map_err(|e| e.to_string())?;
        if let Some(expected) = p.expected_total {
            let want = parse_amount(&expected)?;
            if total != want {
                return Err(format!(
                    "expected_total mismatch: entry totals {total}, expected {want}"
                ));
            }
        }
        let mut receipt_required = self
            .settings()
            .policy
            .receipt_required(store.ledger(), &entry);
        let receipts = preview_receipts(&store, p.entry_id)?;
        // Anti-splitting: if this entry looks like part of a split group,
        // the receipt policy applies to the group total.
        let split_group = self
            .settings()
            .policy
            .split_group(store.ledger(), p.entry_id, &entry);
        if !split_group.is_empty() {
            receipt_required = true;
        }
        let receipt_missing = receipt_required && receipts.is_empty();
        let party_policy = self.settings().parties;
        let party_id = store.ledger().entry_party(p.entry_id);
        let (parties, parties_error) = entry_parties_out(&store, &self.settings(), &entry);
        let no_party_reason = store
            .ledger()
            .entry_no_party(p.entry_id)
            .map(str::to_string);
        // Any party, or a no-party mark, satisfies the policy.
        let party_missing = party_policy != ledger_config::PartyPolicy::Off
            && parties.is_empty()
            && no_party_reason.is_none();
        let party_suggestions = if party_missing {
            suggest_parties(&store, &entry)?
        } else {
            Vec::new()
        };
        let out = PostEntryOut {
            status: "preview".to_string(),
            entry_id: p.entry_id,
            date: entry.date().to_string(),
            description: entry.description().to_string(),
            total: money(total),
            lines: preview_lines(&store, &entry)?,
            receipts,
            receipt_required,
            receipt_missing,
            split_detected: split_group.clone(),
            party_policy: party_policy.key().to_string(),
            party_id,
            party: party_id
                .and_then(|i| store.ledger().get_party(i))
                .map(|x| x.name.clone()),
            parties,
            parties_error,
            no_party_reason,
            party_missing,
            party_suggestions,
            approvals_missing: store
                .ledger()
                .approvals_missing(p.entry_id)
                .map_err(|e| e.to_string())?
                .into_iter()
                .map(|(rule, approver)| ApprovalNeededOut { rule, approver })
                .collect(),
        };
        if !p.confirmed.unwrap_or(false) {
            return Ok(Json(out));
        }
        if !out.approvals_missing.is_empty() {
            let missing: Vec<String> = out
                .approvals_missing
                .iter()
                .map(|a| format!("{} (rule {})", a.approver, a.rule))
                .collect();
            return Err(serde_json::json!({
                "code": "approval_required",
                "message": format!(
                    "entry {} needs approval before it posts (policy.approval_rules is enforce): {}. Ask each approver; when they approve, record it with books.approve_entry, then post.",
                    p.entry_id,
                    missing.join("; ")
                ),
                "entry_id": p.entry_id,
                "remedy_tool": "books.approve_entry",
            })
            .to_string());
        }
        if let Some(why) = &out.parties_error {
            return Err(serde_json::json!({
                "code": "party_shares",
                "message": format!(
                    "entry {}'s party shares don't add up: {why}. Fix them with books.set_entry_party parties (amounts that sum to what it moved, or percents summing to 100).",
                    p.entry_id
                ),
                "entry_id": p.entry_id,
                "remedy_tool": "books.set_entry_party",
            })
            .to_string());
        }
        if party_missing && party_policy == ledger_config::PartyPolicy::Require {
            let names: Vec<String> = out
                .party_suggestions
                .iter()
                .map(|s| format!("{} (party {}, {})", s.name, s.party_id, s.why))
                .collect();
            return Err(serde_json::json!({
                "code": "party_required",
                "message": format!(
                    "entry {} names no counterparty, and these books require one ({} = \"require\"). Tag it with books.set_entry_party (add the party with books.add_party first if it's new), or, if it truly has none, call books.set_entry_party with party null and no_party_reason.{}",
                    p.entry_id,
                    self.setting_label("policy.parties", "[books] parties"),
                    if names.is_empty() { String::new() } else { format!(" Likely: {}.", names.join("; ")) }
                ),
                "entry_id": p.entry_id,
                "remedy_tool": "books.set_entry_party",
                "party_suggestions": out.party_suggestions,
            })
            .to_string());
        }
        if receipt_missing {
            let split_note = if split_group.is_empty() {
                String::new()
            } else {
                format!(
                    " Anti-splitting: this looks like part of a split group with entries {:?}; the policy applies to the group total.",
                    split_group
                )
            };
            return Err(ReceiptRequiredError {
                code: "receipt_required".to_string(),
                message: format!(
                    "entry {} moves {} in debits ({}); attach a receipt with books.attach_receipt before posting.{}",
                    p.entry_id,
                    money(total),
                    self.settings().policy.describe(),
                    split_note
                ),
                entry_id: Some(p.entry_id),
                asset_id: None,
                debits: total.to_string(),
                policy: self.settings().policy.describe(),
                remedy_tool: "books.attach_receipt".to_string(),
                remedy_hint: format!(
                    "attach a receipt to entry {} with books.attach_receipt, then retry books.post_entry with confirmed=true",
                    p.entry_id
                ),
                split_group,
            }
            .to_tool_error());
        }
        store.post_entry(p.entry_id).map_err(|e| e.to_string())?;
        fix_derived_shares(&mut store, &self.settings(), p.entry_id)?;
        let mut posted = out;
        posted.status = "posted".to_string();
        Ok(Json(posted))
    }

    #[tool(
        name = "books.reverse_entry",
        title = "Reverse an entry",
        description = "Reverse a posted entry with an offsetting entry. Without confirmed=true returns a preview; reversals are exempt from the receipt policy.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn reverse_entry(
        &self,
        params: Parameters<ReverseEntryParams>,
    ) -> Result<Json<ReverseEntryOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let orig = store
            .ledger()
            .get_entry(p.entry_id)
            .ok_or_else(|| format!("unknown entry {}", p.entry_id))?
            .clone();
        if orig.status() != ledger_core::EntryStatus::Posted {
            return Err(format!("entry {} is not posted", p.entry_id));
        }
        let date = parse_date(&p.date)?;
        // Build the preview from the flipped lines without mutating.
        let flipped: Vec<(AccountId, Side, Money)> = orig
            .lines()
            .iter()
            .map(|l| {
                let side = match l.side {
                    Side::Debit => Side::Credit,
                    Side::Credit => Side::Debit,
                };
                (l.account, side, l.amount)
            })
            .collect();
        let out = ReverseEntryOut {
            status: "preview".to_string(),
            reversal_id: 0, // filled in on post
            reverses: p.entry_id,
            lines: preview_lines_for(&store, &flipped)?,
        };
        if !p.confirmed.unwrap_or(false) {
            return Ok(Json(out));
        }
        let reversal_id = store
            .reverse_entry(p.entry_id, date, &p.description)
            .map_err(|e| e.to_string())?;
        let mut posted = out;
        posted.status = "posted".to_string();
        posted.reversal_id = reversal_id;
        Ok(Json(posted))
    }

    #[tool(
        name = "books.attach_receipt",
        title = "Attach a receipt",
        description = "Attach a receipt (base64 file bytes) to an entry, or file it unlinked and link later with books.link_receipt. Files are content-addressed and deduplicated. link_to links it to more subjects in the same call (another entry it also supports, an asset, a party, ...: see books.link_attachment); every subject is checked first, so one bad subject files nothing. A file holding a full TIN is refused: one the server read or wrote with TINs, or text with a TIN the vault keeps or an SSN or EIN printed 3-2-4 or 2-7 with hyphens.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn attach_receipt(
        &self,
        params: Parameters<AttachReceiptParams>,
    ) -> Result<Json<AttachReceiptOut>, String> {
        let p = params.0;
        let bytes = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            &p.content_base64,
        )
        .map_err(|_| "content_base64 is not valid base64".to_string())?;
        if bytes.len() > MAX_UPLOAD_BYTES {
            return Err(format!(
                "file is {} bytes; the limit is {MAX_UPLOAD_BYTES}",
                bytes.len()
            ));
        }
        let mut store = self.lock()?;
        if let Some(e) = p.entry_id
            && store.ledger().get_entry(e).is_none()
        {
            return Err(format!("unknown entry {e}"));
        }
        let link_to = check_link_to(&store, p.entry_id, &p.link_to)?;
        self.refuse_tin_file(&store, &bytes, &p.mime_type, &p.file_name)?;
        let (attachment_id, sha256, deduplicated) = store
            .attach_receipt(
                p.entry_id,
                &p.file_name,
                &p.mime_type,
                &bytes,
                p.description.as_deref(),
            )
            .map_err(|e| e.to_string())?;
        for sub in link_to {
            store
                .link_attachment(attachment_id, sub)
                .map_err(|e| e.to_string())?;
        }
        Ok(Json(AttachReceiptOut {
            links: links_out(&store, attachment_id)?,
            attachment_id,
            sha256,
            deduplicated,
        }))
    }

    #[tool(
        name = "books.file_document",
        title = "File a document",
        description = "File a document into the content-addressed store: category formation, legal, tax, accounting, statement (a bank or card statement: then books.record_statement), contract, photo, warranty, import_source (a bank export), or other. link_to links it to subjects in the same call (an account, an asset, a party, the company, ...: see books.link_attachment); every subject is checked first, so one bad subject files nothing. Never file a W-9 here (it shows the full TIN): books.set_w9 w9_file_path puts it in the vault. A file holding a full TIN is refused: one the server read or wrote with TINs, or text with a TIN the vault keeps or an SSN or EIN printed 3-2-4 or 2-7 with hyphens.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn file_document(
        &self,
        params: Parameters<FileDocumentParams>,
    ) -> Result<Json<AttachReceiptOut>, String> {
        let p = params.0;
        match p.category.as_str() {
            "formation" | "legal" | "tax" | "accounting" | "statement" | "contract" | "photo"
            | "warranty" | "import_source" | "other" => {}
            _ => {
                return Err(format!(
                    "bad category {:?}: expected formation, legal, tax, accounting, statement, contract, photo, warranty, import_source, or other",
                    p.category
                ));
            }
        }
        let bytes = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            &p.content_base64,
        )
        .map_err(|_| "content_base64 is not valid base64".to_string())?;
        if bytes.len() > MAX_UPLOAD_BYTES {
            return Err(format!(
                "file is {} bytes; the limit is {MAX_UPLOAD_BYTES}",
                bytes.len()
            ));
        }
        let mut store = self.lock()?;
        let link_to = check_link_to(&store, None, &p.link_to)?;
        self.refuse_tin_file(&store, &bytes, &p.mime_type, &p.file_name)?;
        let (attachment_id, sha256, deduplicated) = store
            .file_document(
                &p.category,
                &p.file_name,
                &p.mime_type,
                &bytes,
                p.description.as_deref(),
            )
            .map_err(|e| e.to_string())?;
        for sub in link_to {
            store
                .link_attachment(attachment_id, sub)
                .map_err(|e| e.to_string())?;
        }
        Ok(Json(AttachReceiptOut {
            links: links_out(&store, attachment_id)?,
            attachment_id,
            sha256,
            deduplicated,
        }))
    }

    #[tool(
        name = "books.link_receipt",
        title = "Link a receipt to an entry",
        description = "Link a previously filed (unlinked) receipt to an entry.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn link_receipt(
        &self,
        params: Parameters<LinkReceiptParams>,
    ) -> Result<Json<LinkReceiptOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        store
            .link_receipt(p.attachment_id, p.entry_id)
            .map_err(|e| e.to_string())?;
        Ok(Json(LinkReceiptOut { ok: true }))
    }

    #[tool(
        name = "books.unlink_receipt",
        title = "Unlink a receipt",
        description = "Detach a receipt from its entry, with a reason recorded in the journal. The receipt stays filed (unlinked); link it elsewhere or withdraw it with books.withdraw_attachment. The receipt policy still holds: an entry that requires a receipt cannot be left with none, so to replace a receipt attach the new one first, then unlink the old.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn unlink_receipt(
        &self,
        params: Parameters<UnlinkReceiptParams>,
    ) -> Result<Json<UnlinkReceiptOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let attachment = store
            .list_attachments(None)
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|a| a.id == p.attachment_id)
            .ok_or_else(|| format!("unknown attachment {}", p.attachment_id))?;
        if let Some(entry_id) = attachment.entry_id {
            let remaining = store
                .list_attachments(Some(entry_id))
                .map_err(|e| e.to_string())?
                .len()
                - 1;
            if remaining == 0 {
                self.ensure_receipt_not_required(&store, entry_id)?;
            }
        }
        let entry_id = store
            .unlink_receipt(p.attachment_id, &p.reason)
            .map_err(|e| e.to_string())?;
        let remaining_receipts = store
            .list_attachments(Some(entry_id))
            .map_err(|e| e.to_string())?
            .len() as u64;
        Ok(Json(UnlinkReceiptOut {
            ok: true,
            entry_id,
            remaining_receipts,
        }))
    }

    #[tool(
        name = "books.withdraw_attachment",
        title = "Withdraw a receipt or document",
        description = "Withdraw an unlinked receipt or a filed document, with a reason recorded in the journal. It drops out of books.list_receipts and can never be linked again. Nothing is deleted: the bytes stay in the blob store and the journal keeps the filing and the withdrawal. Unlink a linked receipt first.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn withdraw_attachment(
        &self,
        params: Parameters<WithdrawAttachmentParams>,
    ) -> Result<Json<WithdrawAttachmentOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        store
            .withdraw_attachment(p.attachment_id, &p.reason)
            .map_err(|e| e.to_string())?;
        Ok(Json(WithdrawAttachmentOut { ok: true }))
    }

    /// ```text
    /// GOAL: one file, linked wherever it is evidence, without refiling.
    /// SOURCE: docs/parties-and-attachments.md "Attachments";
    ///         Store::link_attachment.
    /// INVARIANTS:
    ///   1. Journaled as `attachment.linked`; a refusal journals nothing.
    ///   2. A file linked to an entry counts as that entry's receipt.
    /// SCENARIOS:
    ///   1. IF the attachment is unknown or withdrawn, the subject kind is
    ///      unknown, or the subject doesn't exist: SHOULD refuse.
    ///   2. IF it is already linked there (or filed on that entry): SHOULD
    ///      refuse.
    /// ```
    #[tool(
        name = "books.link_attachment",
        title = "Link a file to a subject",
        description = "Link a stored receipt or document to a subject, so one file is evidence wherever it applies without filing it again. subject: {kind, id}: entry (id), account (code or id: its statements, the agreement that opened it), asset (tag or id: photos, warranty, manual), party (id or name: contracts, correspondence), invoice, bill, period (its close package), payroll_run, schedule (the prepaid contract), reconciliation, or company (no id: formation documents). A file linked to an entry counts as one of its receipts. Journaled as attachment.linked. Returns the attachment's links.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn link_attachment(
        &self,
        params: Parameters<LinkAttachmentParams>,
    ) -> Result<Json<AttachmentLinksOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        find_attachment(&store, p.attachment_id)?;
        let sub = resolve_subject(&store, &p.subject)?;
        store
            .link_attachment(p.attachment_id, sub)
            .map_err(|e| e.to_string())?;
        Ok(Json(AttachmentLinksOut {
            attachment_id: p.attachment_id,
            links: links_out(&store, p.attachment_id)?,
        }))
    }

    /// ```text
    /// GOAL: take a link off, on the record, without leaving evidence
    ///       missing where a rule needs it.
    /// SOURCE: R-13a (receipt corrections are append-only events);
    ///         Store::unlink_attachment.
    /// INVARIANTS:
    ///   1. Journaled as `attachment.unlinked` with the reason; the file
    ///      stays filed. A refusal journals nothing.
    /// SCENARIOS:
    ///   1. IF the reason is blank, or it isn't linked to the subject:
    ///      SHOULD refuse.
    ///   2. IF it is filed on that entry (rather than linked): SHOULD refuse,
    ///      naming books.unlink_receipt.
    ///   3. IF it is a posted entry's last receipt and the receipt policy
    ///      covers the entry: SHOULD refuse (receipt_required).
    ///   4. IF it is the statement of a reconciliation not voided: SHOULD
    ///      refuse (void the reconciliation first).
    /// ```
    #[tool(
        name = "books.unlink_attachment",
        title = "Unlink a file from a subject",
        description = "Take a link (books.link_attachment) off a subject, with the reason recorded in the journal. The file stays filed and keeps its other links. A receipt filed on an entry comes off with books.unlink_receipt instead. Refused when it would leave a posted entry the receipt policy covers without a receipt (attach the replacement first), or take the statement off a reconciliation that isn't voided. Journaled as attachment.unlinked. Returns the attachment's links.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn unlink_attachment(
        &self,
        params: Parameters<UnlinkAttachmentParams>,
    ) -> Result<Json<AttachmentLinksOut>, String> {
        use ledger_store::Subject;
        let p = params.0;
        let mut store = self.lock()?;
        let a = find_attachment(&store, p.attachment_id)?;
        let sub = resolve_subject(&store, &p.subject)?;
        let linked = store
            .attachment_links(a.id)
            .map_err(|e| e.to_string())?
            .contains(&sub);
        match sub {
            Subject::Entry(e) if a.entry_id == Some(e) => {
                return Err(format!(
                    "attachment {} is filed on entry {e}, not linked to it: take it off with books.unlink_receipt",
                    a.id
                ));
            }
            Subject::Entry(e) if linked => {
                let others = store
                    .list_attachments(Some(e))
                    .map_err(|e| e.to_string())?
                    .iter()
                    .filter(|x| x.id != a.id)
                    .count();
                if others == 0 {
                    self.ensure_receipt_not_required(&store, e)?;
                }
            }
            Subject::Reconciliation(r)
                if store
                    .reconciliations(None)
                    .map_err(|e| e.to_string())?
                    .iter()
                    .any(|x| x.id == r && x.attachment_id == a.id && x.voided.is_none()) =>
            {
                return Err(format!(
                    "attachment {} is the statement reconciliation {r} rests on: void the reconciliation first (books.void_reconciliation)",
                    a.id
                ));
            }
            _ => {}
        }
        store
            .unlink_attachment(a.id, sub, &p.reason)
            .map_err(|e| e.to_string())?;
        Ok(Json(AttachmentLinksOut {
            attachment_id: a.id,
            links: links_out(&store, a.id)?,
        }))
    }

    #[tool(
        name = "books.attachments",
        title = "Files linked to a subject",
        description = "The files on a subject ({kind, id} as in books.link_attachment): for an entry, the receipts filed on it and the files linked to it; for anything else, the files linked to it. Withdrawn files are left out. Each comes with all its links.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn attachments(
        &self,
        params: Parameters<AttachmentsParams>,
    ) -> Result<Json<AttachmentsOut>, String> {
        let store = self.lock()?;
        let sub = resolve_subject(&store, &params.0.subject)?;
        let attachments = store
            .attachments_for(sub)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|a| receipt_out(&store, a))
            .collect::<Result<_, _>>()?;
        Ok(Json(AttachmentsOut {
            subject: subject_out(&store, sub),
            attachments,
        }))
    }

    #[tool(
        name = "books.list_receipts",
        title = "List receipts and documents",
        description = "List receipts and filed documents, optionally filtered to one entry (filed on it or linked to it), each with everything it is linked to (links). Withdrawn attachments are hidden unless include_withdrawn=true. For any other subject's files, books.attachments.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn list_receipts(
        &self,
        params: Parameters<ListReceiptsParams>,
    ) -> Result<Json<ListReceiptsOut>, String> {
        let p = params.0;
        let store = self.lock()?;
        let include_withdrawn = p.include_withdrawn.unwrap_or(false);
        let receipts = store
            .list_attachments(p.entry_id)
            .map_err(|e| e.to_string())?
            .into_iter()
            .filter(|a| include_withdrawn || a.withdrawn_at.is_none())
            .map(|a| receipt_out(&store, a))
            .collect::<Result<_, _>>()?;
        Ok(Json(ListReceiptsOut { receipts }))
    }

    #[tool(
        name = "books.get_receipt",
        title = "Get a receipt",
        description = "Fetch a receipt or document's metadata (with everything it is linked to) and base64 bytes. Bytes are hash-verified on read.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn get_receipt(
        &self,
        params: Parameters<GetReceiptParams>,
    ) -> Result<Json<GetReceiptOut>, String> {
        let p = params.0;
        let store = self.lock()?;
        let meta = find_attachment(&store, p.attachment_id)?;
        let bytes = store
            .attachment_bytes(p.attachment_id)
            .map_err(|e| e.to_string())?;
        Ok(Json(GetReceiptOut {
            links: links_out(&store, meta.id)?,
            description: meta.description.clone(),
            attachment_id: meta.id,
            sha256: meta.blob_sha,
            entry_id: meta.entry_id,
            category: meta.category,
            file_name: meta.file_name,
            mime_type: meta.mime_type,
            size_bytes: meta.size_bytes,
            content_base64: base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                &bytes,
            ),
        }))
    }

    #[tool(
        name = "books.get_entry",
        title = "Get an entry",
        description = "Full detail of one entry: lines, totals, status, and attached receipts.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn get_entry(
        &self,
        params: Parameters<GetEntryParams>,
    ) -> Result<Json<GetEntryOut>, String> {
        let p = params.0;
        let store = self.lock()?;
        let entry = store
            .ledger()
            .get_entry(p.entry_id)
            .ok_or_else(|| format!("unknown entry {}", p.entry_id))?
            .clone();
        let lines = entry
            .lines()
            .iter()
            .map(|l| {
                // A draft line an earlier version journaled may name an
                // account the chart doesn't have (rules.md B-28).
                let (code, name) = match store.ledger().chart().get(l.account) {
                    Some(a) => (a.code.clone(), a.name.clone()),
                    None => (String::new(), format!("(no account {})", l.account)),
                };
                EntryLineOut {
                    account_id: l.account,
                    code,
                    name,
                    side: match l.side {
                        Side::Debit => "debit".to_string(),
                        Side::Credit => "credit".to_string(),
                    },
                    amount: money(l.amount.amount),
                }
            })
            .collect();
        let (parties, parties_error) = entry_parties_out(&store, &self.settings(), &entry);
        let total = ReceiptPolicy::entry_debits(&entry);
        Ok(Json(GetEntryOut {
            entry_id: p.entry_id,
            date: entry.date().to_string(),
            description: entry.description().to_string(),
            status: match entry.status() {
                ledger_core::EntryStatus::Draft => "draft".to_string(),
                ledger_core::EntryStatus::Posted => "posted".to_string(),
            },
            reversal_of: entry.reversal_of(),
            lines,
            total: total.as_ref().ok().map(|t| money(*t)),
            total_error: total.err().map(|e| e.to_string()),
            receipts: preview_receipts(&store, p.entry_id)?,
            vendor: store
                .ledger()
                .entry_vendor(p.entry_id)
                .and_then(|v| store.ledger().get_vendor(v))
                .map(|v| v.name.clone()),
            party_id: store.ledger().entry_party(p.entry_id),
            party: store
                .ledger()
                .entry_party(p.entry_id)
                .and_then(|id| store.ledger().get_party(id))
                .map(|p| p.label().to_string()),
            parties,
            parties_error,
            no_party_reason: store
                .ledger()
                .entry_no_party(p.entry_id)
                .map(str::to_string),
        }))
    }

    #[tool(
        name = "books.search_entries",
        title = "Search entries",
        description = "Search posted entries by account, date range, and description text. Newest first.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn search_entries(
        &self,
        params: Parameters<SearchEntriesParams>,
    ) -> Result<Json<SearchEntriesOut>, String> {
        let p = params.0;
        let store = self.lock()?;
        let rows = store
            .search_entries(
                p.account_id,
                p.date_from.as_deref(),
                p.date_to.as_deref(),
                p.text.as_deref(),
                p.limit.unwrap_or(50).min(500),
                p.offset.unwrap_or(0),
            )
            .map_err(|e| e.to_string())?;
        let mut entries = Vec::with_capacity(rows.len());
        for r in rows {
            let e = store
                .ledger()
                .get_entry(r.id)
                .ok_or_else(|| format!("entry {} missing from ledger", r.id))?;
            let total = ReceiptPolicy::entry_debits(e).map_err(|e| e.to_string())?;
            let receipt_count = store
                .list_attachments(Some(r.id))
                .map_err(|e| e.to_string())?
                .len() as u64;
            entries.push(EntrySummary {
                entry_id: r.id,
                date: r.date,
                description: r.description,
                total: money(total),
                reversal_of: r.reversal_of,
                receipt_count,
            });
        }
        Ok(Json(SearchEntriesOut { entries }))
    }

    #[tool(
        name = "books.show",
        title = "Show in the viewer",
        description = "Show something in the human's viewer (the read-only desktop app): an entry (with its lines, receipt, and history), an account's register (optionally for start/end dates), a report (optionally for start/end dates, split into month/quarter/year columns, compared with the prior period or prior year), a stored receipt or document, a fixed asset (by tag or id), a party (by id or name: who they are and everything the company did with them), a reconciliation (by id: its account with its statements and reconciliations), a search of the transactions, or a page. Give exactly one target; work out dates yourself (\"last quarter\" means its first and last day). Use it whenever the human asks to see something, and after posting so they can look at what you did. Changes nothing in the books.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn show(&self, params: Parameters<ShowParams>) -> Result<Json<ShowOut>, String> {
        use ledger_store::viewer::{ViewerTarget, request};
        let p = params.0;
        let store = self.lock()?;
        let ledger = store.ledger();
        let given = [
            p.entry_id.is_some(),
            p.account.is_some(),
            p.report.is_some(),
            p.attachment_id.is_some(),
            p.asset.is_some(),
            p.party.is_some(),
            p.reconciliation.is_some(),
            p.search.is_some(),
            p.page.is_some(),
        ]
        .iter()
        .filter(|x| **x)
        .count();
        if given != 1 {
            return Err(
                "give exactly one of entry_id, account, report, attachment_id, asset, party, reconciliation, search, page"
                    .into(),
            );
        }
        for d in [&p.start, &p.end].into_iter().flatten() {
            parse_date(d)?;
        }
        const REPORTS: [&str; 11] = [
            "profit_and_loss",
            "balance_sheet",
            "cash_flow",
            "members_equity",
            "trial_balance",
            "general_ledger",
            "tax_summary",
            "vendor_1099",
            "ar_aging",
            "ap_aging",
            "fixed_asset_register",
        ];
        const PAGES: [&str; 11] = [
            "overview",
            "cash",
            "profit",
            "transactions",
            "reports",
            "accounts",
            "assets",
            "parties",
            "documents",
            "company",
            "journal",
        ];
        let (target, shown) = if let Some(r) = &p.asset {
            let id = resolve_asset_ref(&store, r)?;
            let a = ledger.get_asset(id).expect("resolved");
            (
                ViewerTarget::Asset { asset_id: id },
                format!(
                    "asset {}{}",
                    a.tag
                        .as_deref()
                        .map(|t| format!("{t}, "))
                        .unwrap_or_default(),
                    a.description
                ),
            )
        } else if let Some(r) = &p.party {
            let id = resolve_party(&store, r)?;
            let party = ledger.get_party(id).expect("resolved");
            (
                ViewerTarget::Party { party_id: id },
                format!("party {id}, {}", party.label()),
            )
        } else if let Some(id) = p.reconciliation {
            let r = store
                .reconciliations(None)
                .map_err(|e| e.to_string())?
                .into_iter()
                .find(|r| r.id == id)
                .ok_or_else(|| format!("unknown reconciliation {id}"))?;
            let (code, name) = code_name(&store, r.account_id);
            (
                ViewerTarget::Reconciliation { id },
                format!(
                    "reconciliation {id}, {code} {name} through {}",
                    r.statement_date
                ),
            )
        } else if let Some(id) = p.entry_id {
            let e = ledger
                .get_entry(id)
                .ok_or_else(|| format!("unknown entry {id}"))?;
            (
                ViewerTarget::Entry { entry_id: id },
                format!("entry #{id}, {}", e.description()),
            )
        } else if let Some(code) = p.account {
            let a = ledger
                .chart()
                .find_by_code(&code)
                .ok_or_else(|| format!("unknown account {code}"))?;
            (
                ViewerTarget::Account {
                    code: code.clone(),
                    start: p.start.clone(),
                    end: p.end.clone(),
                },
                format!("the register of {} {}", a.code, a.name),
            )
        } else if let Some(report) = p.report {
            if !REPORTS.contains(&report.as_str()) {
                return Err(format!(
                    "report {report:?}: expected one of {}",
                    REPORTS.join(", ")
                ));
            }
            if let Some(c) = &p.columns
                && !["total", "month", "quarter", "year"].contains(&c.as_str())
            {
                return Err(format!(
                    "columns {c:?}: expected total, month, quarter, or year"
                ));
            }
            if let Some(c) = &p.compare
                && !["prior_period", "prior_year"].contains(&c.as_str())
            {
                return Err(format!(
                    "compare {c:?}: expected prior_period or prior_year"
                ));
            }
            let mut what = format!("the {} report", report.replace('_', " "));
            if let (Some(s), Some(e)) = (&p.start, &p.end) {
                what.push_str(&format!(", {s} to {e}"));
            }
            if let Some(c) = p.columns.as_deref().filter(|c| *c != "total") {
                what.push_str(&format!(", by {c}"));
            }
            if let Some(c) = &p.compare {
                what.push_str(&format!(", compared with the {}", c.replace('_', " ")));
            }
            (
                ViewerTarget::Report {
                    report: report.clone(),
                    start: p.start.clone(),
                    end: p.end.clone(),
                    columns: p.columns.clone(),
                    compare: p.compare.clone(),
                },
                what,
            )
        } else if let Some(id) = p.attachment_id {
            let a = store
                .list_attachments(None)
                .map_err(|e| e.to_string())?
                .into_iter()
                .find(|a| a.id == id)
                .ok_or_else(|| format!("unknown attachment {id}"))?;
            (
                ViewerTarget::Document { attachment_id: id },
                format!("document {id}, {}", a.file_name),
            )
        } else if let Some(query) = p.search {
            (
                ViewerTarget::Search {
                    query: query.clone(),
                },
                format!("transactions matching {query:?}"),
            )
        } else {
            let page = p.page.unwrap_or_default();
            if !PAGES.contains(&page.as_str()) {
                return Err(format!(
                    "page {page:?}: expected one of {}",
                    PAGES.join(", ")
                ));
            }
            (
                ViewerTarget::Page { page: page.clone() },
                format!("the {page} page"),
            )
        };
        request(store.data_dir(), target).map_err(|e| format!("couldn't reach the viewer: {e}"))?;
        Ok(Json(ShowOut {
            shown: shown.clone(),
            note: format!(
                "Sent to the viewer: {shown}. If it isn't open, ask the human to open the viewer (within a couple of minutes, it opens right there)."
            ),
        }))
    }

    #[tool(
        name = "books.verify",
        title = "Verify the books",
        description = "Check the books end to end: the journal's hash chain; a fresh replay against the live ledger; every table of the books' database, row for row, against a fresh projection of the journal (rebuilding the database from the journal if it drifted, keeping the old copy); and, by default, every stored receipt and document against its SHA-256. Run it whenever a number looks wrong or a human asks whether the books are intact.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn verify(&self, params: Parameters<VerifyParams>) -> Result<Json<VerifyOut>, String> {
        let mut store = self.lock()?;
        let report = store.verify_and_heal().map_err(|e| e.to_string())?;
        let blobs = if params.0.blobs {
            Some(store.verify_blobs().map_err(|e| e.to_string())?)
        } else {
            None
        };
        let mut checked = vec![
            format!("journal hash chain ({} events)", report.events),
            "fresh replay matches the live ledger (every statement, assets, classes, payroll)"
                .to_string(),
            "SQLite database matches a fresh projection of the journal, every table, row for row"
                .to_string(),
        ];
        if let Some(n) = blobs {
            checked.push(format!("{n} stored files match their SHA-256"));
        }
        Ok(Json(VerifyOut {
            status: if report.healed.is_some() {
                "healed"
            } else {
                "ok"
            }
            .to_string(),
            events: report.events,
            head: report.head,
            healed: report.healed,
            blobs_verified: blobs,
            checked,
        }))
    }

    #[tool(
        name = "books.digest",
        title = "Activity digest",
        description = "What changed since a date: entries posted, per-account movement, and receipt coverage; plus what needs doing: rules due for review, settings problems, and cash and card accounts not reconciled through the end of last month (unreconciled_accounts). Built for human review.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn digest(&self, params: Parameters<DigestParams>) -> Result<Json<DigestOut>, String> {
        let p = params.0;
        let since = parse_date(&p.since)?;
        let today = today();
        let store = self.lock()?;
        let ledger = store.ledger();
        let mut entries = Vec::new();
        for e in ledger.posted_entries() {
            if e.date() < since {
                continue;
            }
            let receipts = store
                .list_attachments(Some(e.id()))
                .map_err(|e| e.to_string())?;
            let required = self.settings().policy.receipt_required(ledger, e);
            entries.push(DigestEntry {
                entry_id: e.id(),
                date: e.date().to_string(),
                description: e.description().to_string(),
                total: money(ReceiptPolicy::entry_debits(e).map_err(|e| e.to_string())?),
                reversal_of: e.reversal_of(),
                receipt_count: receipts.len() as u64,
                receipt_required: required,
                receipt_missing: required && receipts.is_empty(),
            });
        }
        entries.sort_by(|a, b| b.date.cmp(&a.date).then(b.entry_id.cmp(&a.entry_id)));
        let mut totals = Vec::new();
        for a in ledger.chart().all() {
            if a.placeholder {
                continue;
            }
            let m = movement_in(ledger, a.id, since, today);
            if !m.is_zero() {
                totals.push(DigestTotal {
                    code: a.code.clone(),
                    name: a.name.clone(),
                    movement: money(m),
                });
            }
        }
        totals.sort_by(|a, b| a.code.cmp(&b.code));
        let stale_rules = ledger_core::rules::stale_rules(today)
            .into_iter()
            .map(|r| StaleRule {
                name: r.name.to_string(),
                known_through: r.known_through,
                source: r.authority.to_string(),
                why: r.review_why.to_string(),
            })
            .collect();
        let journal_repairs = store
            .tail_repairs()
            .map_err(|e| e.to_string())?
            .into_iter()
            .filter(|(at, _, _)| at.get(..10).is_some_and(|d| d >= p.since.as_str()))
            // The stamp only: the file's name would name the journal.
            .map(|(at, bytes, saved_as)| JournalRepair {
                at,
                bytes,
                set_aside: saved_as
                    .rsplit_once(".torn.")
                    .map(|(_, stamp)| stamp.to_string())
                    .unwrap_or_default(),
            })
            .collect();
        let unreconciled_accounts = self.unreconciled_accounts(&store, today)?;
        Ok(Json(DigestOut {
            unreconciled_accounts,
            since: since.to_string(),
            entries,
            totals_by_account: totals,
            stale_rules,
            journal_repairs,
            settings_problems: {
                let mut problems = settings_problems(store.ledger(), &self.settings());
                if let Some(e) = self.live().status().error {
                    problems.insert(
                        0,
                        format!(
                            "the settings didn't load (the last good settings are in force): {e}"
                        ),
                    );
                }
                problems
            },
        }))
    }

    #[tool(
        name = "books.import_bank_csv",
        title = "Import a bank CSV as drafts",
        description = "Import a QuickBooks-style bank/credit-card CSV: 3 columns Date,Description,Amount (payments negative), or 4 columns Date,Description, money out, money in (Intuit heads them Credit,Debit). Slash dates must settle month-first or day-first from the file itself or from date_order (\"month_first\" for mm/dd, \"day_first\" for dd/mm); ask the human when the tool says the dates read either way. Creates one DRAFT entry per transaction, all or none: bank account on one side, categorized offset on the other (the longest matching rule pattern, else default_offset_code). Review with books.digest, then post. File the export first (books.file_document category import_source) and pass source_attachment_id: it is linked to the bank account, so the import traces to its source (never to the drafts: a bank export is not a receipt).",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn import_bank_csv(
        &self,
        params: Parameters<ImportBankCsvParams>,
    ) -> Result<Json<ImportBankCsvOut>, String> {
        let p = params.0;
        let order = match p.date_order.as_deref() {
            None => None,
            Some("month_first") => Some(ledger_import::DateOrder::MonthFirst),
            Some("day_first") => Some(ledger_import::DateOrder::DayFirst),
            Some(other) => {
                return Err(format!(
                    "date_order must be \"month_first\" or \"day_first\", got {other:?}"
                ));
            }
        };
        let txns =
            ledger_import::parse_bank_csv_with(&p.csv_text, order).map_err(|e| e.to_string())?;
        let mut store = self.lock()?;
        let bank = store
            .ledger()
            .chart()
            .find_by_code(&p.bank_account_code)
            .map(|a| (a.id, a.code.clone()))
            .ok_or_else(|| format!("unknown bank account code: {}", p.bank_account_code))?;
        // The source file links to the account, checked before any draft.
        let link_source = match p.source_attachment_id {
            None => false,
            Some(att) => {
                let a = find_attachment(&store, att)?;
                if a.withdrawn_at.is_some() {
                    return Err(format!("attachment {att} is withdrawn"));
                }
                !store
                    .attachment_links(att)
                    .map_err(|e| e.to_string())?
                    .contains(&ledger_store::Subject::Account(bank.0))
            }
        };
        let default_offset = store
            .ledger()
            .chart()
            .find_by_code(&p.default_offset_code)
            .map(|a| (a.id, a.code.clone()))
            .ok_or_else(|| format!("unknown offset account code: {}", p.default_offset_code))?;
        // Resolve rule account codes upfront.
        let mut rules: Vec<(String, (ledger_core::AccountId, String))> = Vec::new();
        for (pattern, code) in &p.rules {
            let (id, c) = store
                .ledger()
                .chart()
                .find_by_code(code)
                .map(|a| (a.id, a.code.clone()))
                .ok_or_else(|| format!("unknown rule account code: {code}"))?;
            rules.push((pattern.to_lowercase(), (id, c)));
        }
        // The most specific pattern wins (longest, then alphabetical): the
        // rules arrive as a map with no order, so "first match" would pick
        // a different account from run to run when two patterns match.
        rules.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(&b.0)));
        let offset_for = |description: &str| {
            let desc_lower = description.to_lowercase();
            rules
                .iter()
                .find(|(pat, _)| desc_lower.contains(pat.as_str()))
                .map(|(_, v)| v.clone())
                .unwrap_or_else(|| default_offset.clone())
        };
        let base = store.ledger().base_currency();
        // Rehearse the whole file on a copy first, so a transaction that
        // can't become a draft (a date in a closed period, say) refuses the
        // file instead of leaving the ones before it behind.
        let mut scratch = store.ledger().clone();
        for txn in &txns {
            let (offset_id, _) = offset_for(&txn.description);
            let amount = Money::new(txn.amount.abs(), base);
            let (bank_side, offset_side) = if txn.amount > Decimal::ZERO {
                (Side::Debit, Side::Credit)
            } else {
                (Side::Credit, Side::Debit)
            };
            let e = scratch
                .try_create_entry(txn.date, &txn.description)
                .and_then(|e| scratch.add_line(e, bank.0, bank_side, amount).map(|()| e))
                .and_then(|e| scratch.add_line(e, offset_id, offset_side, amount))
                .map_err(|e| format!("{} {}: {e}", txn.date, txn.description));
            e?;
        }
        let mut imported = Vec::new();
        for txn in txns {
            let (offset_id, offset_code) = offset_for(&txn.description);
            let amount = Money::new(txn.amount.abs(), store.ledger().base_currency());
            let entry_id = store
                .create_entry(txn.date, &format!("Bank import — {}", txn.description))
                .map_err(|e| e.to_string())?;
            // Money in: debit bank, credit offset. Money out: reverse.
            let (bank_side, offset_side) = if txn.amount > Decimal::ZERO {
                (Side::Debit, Side::Credit)
            } else {
                (Side::Credit, Side::Debit)
            };
            store
                .add_line(entry_id, bank.0, bank_side, amount)
                .map_err(|e| e.to_string())?;
            store
                .add_line(entry_id, offset_id, offset_side, amount)
                .map_err(|e| e.to_string())?;
            imported.push(ImportedTxnOut {
                entry_id,
                date: txn.date.to_string(),
                description: txn.description.clone(),
                amount: money(txn.amount),
                offset_account: offset_code,
            });
        }
        if link_source && let Some(att) = p.source_attachment_id {
            store
                .link_attachment(att, ledger_store::Subject::Account(bank.0))
                .map_err(|e| e.to_string())?;
        }
        let count = imported.len();
        Ok(Json(ImportBankCsvOut {
            imported,
            count,
            source_attachment_id: p.source_attachment_id,
            source_linked: link_source,
        }))
    }

    #[tool(
        name = "books.create_schedule",
        title = "Create an amortization schedule",
        description = "Spread a prepaid expense (asset expensed monthly, e.g. a year of insurance paid up front) or deferred revenue (liability earned monthly, e.g. an annual subscription collected up front) evenly over the months it covers, to the cent (the last month absorbs rounding). Record the original payment first (Dr prepaid / Cr cash, with its receipt; or Dr cash / Cr deferred revenue). Returns the monthly plan; post it with books.run_schedules.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn create_schedule(
        &self,
        params: Parameters<CreateScheduleParams>,
    ) -> Result<Json<ScheduleOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let kind = match p.kind.as_str() {
            "prepaid_expense" => ledger_core::ScheduleKind::PrepaidExpense,
            "deferred_revenue" => ledger_core::ScheduleKind::DeferredRevenue,
            other => {
                return Err(format!(
                    "kind {other:?}: expected prepaid_expense or deferred_revenue"
                ));
            }
        };
        let currency = store.ledger().base_currency();
        let balance_account = resolve_account(&store, &p.balance_account)?;
        let pl_account = resolve_account(&store, &p.pl_account)?;
        let id = store
            .register_schedule(ledger_core::NewSchedule {
                description: p.description,
                kind,
                total: Money::new(parse_amount(&p.total)?, currency),
                start: parse_date(&p.start)?,
                months: p.months,
                balance_account,
                pl_account,
            })
            .map_err(|e| e.to_string())?;
        Ok(Json(schedule_out(
            store.ledger(),
            store.ledger().get_schedule(id).expect("registered"),
        )))
    }

    #[tool(
        name = "books.list_schedules",
        title = "List amortization schedules",
        description = "Amortization schedules (prepaid expenses, deferred revenue): each month's amount, what has posted, and what remains.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn list_schedules(&self) -> Result<Json<ListSchedulesOut>, String> {
        let store = self.lock()?;
        let ledger = store.ledger();
        Ok(Json(ListSchedulesOut {
            schedules: ledger
                .schedules()
                .map(|s| schedule_out(ledger, s))
                .collect(),
        }))
    }

    #[tool(
        name = "books.run_schedules",
        title = "Post scheduled amortization",
        description = "Post every amortization schedule's months ending on or before `through` (default today) that haven't posted. Idempotent: a month posts once. Without confirmed=true, returns the postings it would make. Exempt from the receipt policy: the schedule is the evidence (the original payment carries the receipt).",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn run_schedules(
        &self,
        params: Parameters<RunSchedulesParams>,
    ) -> Result<Json<RunSchedulesOut>, String> {
        let p = params.0;
        let through = match &p.through {
            Some(s) => parse_date(s)?,
            None => today(),
        };
        let mut store = self.lock()?;
        // A schedule whose plan can't be computed refuses the run, naming
        // it, before any month posts.
        let mut due: Vec<(NaiveDate, u64, Decimal)> = Vec::new();
        for s in store.ledger().schedules() {
            let months = s
                .due(through)
                .map_err(|e| format!("schedule {}: {e}", s.id))?;
            due.extend(months.into_iter().map(|(d, a)| (d, s.id, a)));
        }
        due.sort();
        let describe = |store: &Store, id: u64, date: NaiveDate| -> Result<String, String> {
            Ok(store
                .ledger()
                .preview_schedule_month(id, date)
                .map_err(|e| e.to_string())?
                .1)
        };
        if !p.confirmed {
            let postings = due
                .into_iter()
                .map(|(date, id, amount)| {
                    Ok(SchedulePostingOut {
                        schedule_id: id,
                        date: date.to_string(),
                        description: describe(&store, id, date)?,
                        amount: money(amount),
                        entry_id: None,
                    })
                })
                .collect::<Result<_, String>>()?;
            return Ok(Json(RunSchedulesOut {
                status: "preview".to_string(),
                postings,
            }));
        }
        let mut postings = Vec::new();
        for (date, id, amount) in due {
            let description = describe(&store, id, date)?;
            let entry = store
                .post_schedule_month(id, date)
                .map_err(|e| e.to_string())?;
            postings.push(SchedulePostingOut {
                schedule_id: id,
                date: date.to_string(),
                description,
                amount: money(amount),
                entry_id: Some(entry),
            });
        }
        Ok(Json(RunSchedulesOut {
            status: "posted".to_string(),
            postings,
        }))
    }

    #[tool(
        name = "books.create_recurring",
        title = "Create a recurring entry",
        description = "A recurring entry (rent, a subscription, a loan payment): balanced lines and a frequency (weekly, monthly, quarterly, yearly) from a start date, optionally ending. books.run_recurring turns due occurrences into drafts, never posted entries: attach each actual receipt and post through the normal preview.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn create_recurring(
        &self,
        params: Parameters<CreateRecurringParams>,
    ) -> Result<Json<RecurringOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let currency = store.ledger().base_currency();
        let frequency = ledger_core::Frequency::parse(&p.frequency).ok_or_else(|| {
            format!(
                "frequency {:?}: expected weekly, monthly, quarterly, or yearly",
                p.frequency
            )
        })?;
        let lines = p
            .lines
            .iter()
            .map(|l| {
                Ok((
                    resolve_account(&store, &l.account)?,
                    parse_side(&l.side)?,
                    Money::new(parse_amount(&l.amount)?, currency),
                ))
            })
            .collect::<Result<Vec<_>, String>>()?;
        let id = store
            .register_recurring(ledger_core::NewRecurring {
                description: p.description,
                lines,
                frequency,
                start: parse_date(&p.start)?,
                end: match &p.end {
                    Some(e) => Some(parse_date(e)?),
                    None => None,
                },
            })
            .map_err(|e| e.to_string())?;
        let r = store
            .ledger()
            .recurring_entries()
            .find(|r| r.id == id)
            .expect("registered");
        Ok(Json(recurring_out(r)))
    }

    #[tool(
        name = "books.list_recurring",
        title = "List recurring entries",
        description = "Recurring entries and the occurrences generated so far.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn list_recurring(&self) -> Result<Json<ListRecurringOut>, String> {
        let store = self.lock()?;
        Ok(Json(ListRecurringOut {
            recurring: store
                .ledger()
                .recurring_entries()
                .map(recurring_out)
                .collect(),
        }))
    }

    #[tool(
        name = "books.run_recurring",
        title = "Draft recurring entries",
        description = "Turn every recurring entry's occurrences on or before `through` (default today) into drafts. Idempotent: an occurrence generates once. Occurrences dated in a closed period are not generated; they come back in `skipped_closed_period` for the human to decide. Each draft then goes through the normal flow: attach the receipt, preview, post.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn run_recurring(
        &self,
        params: Parameters<RunRecurringParams>,
    ) -> Result<Json<RunRecurringOut>, String> {
        let through = match &params.0.through {
            Some(s) => parse_date(s)?,
            None => today(),
        };
        let mut store = self.lock()?;
        let drafts = store
            .run_recurring(through)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|(id, date, entry)| RecurringDraftOut {
                recurring_id: id,
                date: date.to_string(),
                entry_id: entry,
            })
            .collect();
        let skipped_closed_period = store
            .ledger()
            .recurring_in_closed_periods(through)
            .into_iter()
            .map(|(id, date)| RecurringSkippedOut {
                recurring_id: id,
                date: date.to_string(),
            })
            .collect();
        Ok(Json(RunRecurringOut {
            drafts,
            skipped_closed_period,
        }))
    }

    #[tool(
        name = "books.record_payout",
        title = "Record a platform payout",
        description = "Record a platform payout (app store, payment processor, marketplace) grossed up: Dr the deposit account the net deposit, Dr fee accounts the fees withheld, Dr/Cr adjustments (refunds, chargebacks, reserves, taxes withheld), Cr revenue the gross sales (several lines and classes allowed). Refuses unless gross - fees +/- adjustments equals the deposit to the cent, and says by how much it's off. Creates a draft: attach the payout statement as its receipt, preview, and post.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn record_payout(
        &self,
        params: Parameters<RecordPayoutParams>,
    ) -> Result<Json<RecordPayoutOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let class = |store: &Store, c: &Option<String>| -> Result<Option<u32>, String> {
            c.as_deref().map(|c| resolve_class(store, c)).transpose()
        };
        let lines = |store: &Store, ls: &[PayoutLineParam]| {
            ls.iter()
                .map(|l| {
                    Ok(ledger_core::PayoutLine {
                        account: resolve_account(store, &l.account)?,
                        amount: parse_amount(&l.amount)?,
                        class: class(store, &l.class)?,
                    })
                })
                .collect::<Result<Vec<_>, String>>()
        };
        let payout = ledger_core::Payout {
            deposit_account: resolve_account(&store, &p.deposit_account)?,
            deposit: parse_amount(&p.deposit)?,
            revenue: lines(&store, &p.revenue)?,
            fees: lines(&store, &p.fees)?,
            adjustments: p
                .adjustments
                .iter()
                .map(|a| {
                    Ok(ledger_core::PayoutAdjustment {
                        account: resolve_account(&store, &a.account)?,
                        side: parse_side(&a.side)?,
                        amount: parse_amount(&a.amount)?,
                        class: class(&store, &a.class)?,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?,
        };
        let (plan, totals) =
            ledger_core::payout_plan(&payout, store.ledger().chart()).map_err(|e| e.to_string())?;
        let currency = store.base_currency();
        let entry_id = store
            .create_entry(parse_date(&p.date)?, &p.description)
            .map_err(|e| e.to_string())?;
        let mut described = Vec::new();
        for (index, (account, side, amount, class)) in plan.into_iter().enumerate() {
            store
                .add_line(entry_id, account, side, Money::new(amount, currency))
                .map_err(|e| e.to_string())?;
            if let Some(class) = class {
                store
                    .set_line_class(entry_id, index, Some(class))
                    .map_err(|e| e.to_string())?;
            }
            let code = store
                .ledger()
                .chart()
                .get(account)
                .map(|a| a.code.clone())
                .unwrap_or_default();
            described.push(format!("{side:?} {code} {}", money(amount)));
        }
        Ok(Json(RecordPayoutOut {
            entry_id,
            status: "draft".to_string(),
            gross: money(totals.gross),
            fees: money(totals.fees),
            adjustments: money(totals.adjustments),
            deposit: money(totals.deposit),
            lines: described,
            next: "Attach the payout statement (books.attach_receipt), then preview and post with books.post_entry.".to_string(),
        }))
    }

    #[tool(
        name = "books.list_vendors",
        title = "List vendors",
        description = "Vendors, with which 1099 each gets, whether its W-9 is on file, and the party each belongs to (books.party).",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn list_vendors(&self) -> Result<Json<ListVendorsOut>, String> {
        let store = self.lock()?;
        Ok(Json(ListVendorsOut {
            vendors: store
                .ledger()
                .list_vendors()
                .into_iter()
                .map(|v| VendorOut {
                    id: v.id,
                    name: v.name.clone(),
                    form_1099: form_1099_label(v.form_1099),
                    tin_on_file: v.tin_on_file,
                    party_id: store.ledger().party_of_vendor(v.id),
                })
                .collect(),
        }))
    }

    #[tool(
        name = "books.set_vendor_1099",
        title = "Set a vendor's 1099 status",
        description = "Mark which information return a vendor gets: \"nec\" (1099-NEC: contractors, freelancers, attorneys), \"misc\" (1099-MISC: rent, royalties, prizes), or \"none\" (corporations generally, merchants, anyone paid only by card). Record whether a W-9 is on file; never store the TIN itself. Ask the human when unsure; the CPA confirms.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn set_vendor_1099(
        &self,
        params: Parameters<SetVendor1099Params>,
    ) -> Result<Json<VendorOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let id = resolve_vendor(&store, &p.vendor)?;
        let form = match p.form_1099.trim().to_lowercase().as_str() {
            "none" | "" => None,
            other => Some(
                ledger_core::Form1099::parse(other)
                    .ok_or_else(|| format!("form_1099 {other:?}: expected nec, misc, or none"))?,
            ),
        };
        store
            .set_vendor_1099(id, form, p.tin_on_file)
            .map_err(|e| e.to_string())?;
        let v = store.ledger().get_vendor(id).expect("exists");
        Ok(Json(VendorOut {
            id,
            name: v.name.clone(),
            form_1099: form_1099_label(v.form_1099),
            tin_on_file: v.tin_on_file,
            party_id: store.ledger().party_of_vendor(id),
        }))
    }

    #[tool(
        name = "books.set_entry_vendor",
        title = "Tag an entry's vendor",
        description = "Tag an entry (draft or posted) with the vendor it paid, for 1099 tracking, or omit vendor to clear the tag. Reporting metadata only: amounts never change. On an entry naming several parties (books.set_entry_party parties) the parties stay as they are: the vendor of a party already named is tagged with every share kept; any other vendor, or clearing the tag while its party is named, is refused (change the parties instead). An entry and its reversal are tagged together. books.pay_bill tags its payments automatically; books.draft_entry takes a vendor too.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn set_entry_vendor(
        &self,
        params: Parameters<SetEntryVendorParams>,
    ) -> Result<Json<GetEntryOut>, String> {
        let p = params.0;
        {
            let mut store = self.lock()?;
            let vendor = p
                .vendor
                .as_deref()
                .map(|v| resolve_vendor(&store, v))
                .transpose()?;
            store
                .set_entry_vendor(p.entry_id, vendor)
                .map_err(|e| e.to_string())?;
        }
        self.get_entry(Parameters(GetEntryParams {
            entry_id: p.entry_id,
        }))
        .await
    }

    #[tool(
        name = "books.vendor_1099",
        title = "1099 worksheet",
        description = "1099 worksheet for a calendar year: cash paid to each vendor (credits minus debits to the [books] cash and financing accounts on entries tagged with the vendor, so card payments, which the card processor reports on 1099-K, are excluded; on an entry naming several parties, only the vendor's resolved share of that payment), which vendors are reportable (marked NEC/MISC and paid at least the threshold: $600 through 2025, $2,000 in 2026, indexed for inflation after that), which are missing a W-9, and each payee's TIN last four. include_tins: true adds each payee's full TIN from the TIN vault (or why it can't): only when the human asks for the CPA's worksheet, and never repeat those TINs anywhere else; save_csv_to also writes the worksheet as CSV to a new file outside the books dir. For a year whose threshold wanderbooks doesn't know yet it refuses rather than guess: get the figure from the IRS or the human, pass it as `threshold`, and tell the human, who can report it upstream so the rule is updated for everyone. For the CPA to confirm.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn vendor_1099(
        &self,
        params: Parameters<Vendor1099Params>,
    ) -> Result<Json<Vendor1099Out>, String> {
        let p = params.0;
        let threshold = p.threshold.as_deref().map(parse_amount).transpose()?;
        let store = self.lock()?;
        // A year past what wanderbooks knows is refused with guidance
        // (research or ask, pass `threshold`, then update the rule).
        let r = vendor_1099_report(store.ledger(), &self.settings(), p.year, threshold)
            .map_err(|e| e.to_string())?;
        let missing_w9 = r
            .vendors
            .iter()
            .filter(|v| v.reportable && !v.tin_on_file)
            .map(|v| v.name.clone())
            .collect();
        let untagged = store
            .ledger()
            .posted_entries()
            .filter(|e| {
                chrono::Datelike::year(&e.date()) == p.year
                    && store.ledger().entry_vendor(e.id()).is_none()
            })
            .count();
        let mut notes = vec![
            "Only entries tagged with a vendor count (books.set_entry_vendor; books.pay_bill tags automatically).".to_string(),
            "Corporations are generally exempt, except attorneys' fees and medical payments; royalties report at $10. Confirm with the CPA.".to_string(),
        ];
        if p.year > 2026 {
            notes.push(format!(
                "The $2,000 threshold is indexed for inflation after 2026; confirm the {} amount and pass threshold to override.",
                p.year
            ));
        }
        if untagged > 0 {
            notes.push(format!(
                "{untagged} posted entries in {} have no vendor tag; review any that paid contractors or rent.",
                p.year
            ));
        }
        if !r.unresolved.is_empty() {
            notes.push(format!(
                "Entries {} name several parties whose shares don't resolve, so each counts in full toward its vendor: fix the shares with books.set_entry_party.",
                r.unresolved
                    .iter()
                    .map(|id| id.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        let ledger = store.ledger();
        // With include_tins, the vault is opened once (never creating a
        // key: a locked vault becomes a note on each line).
        let vault = p.include_tins.then(|| self.open_vault(&store, false));
        let mut vendors = Vec::new();
        for v in r.vendors {
            let party_id = ledger.party_of_vendor(v.vendor_id);
            let tin_last4 = party_id.and_then(|id| ledger.get_party(id)).and_then(|p| {
                p.w9.as_ref()
                    .map(|f| f.tin_last4.clone())
                    .or_else(|| p.tin.as_ref().map(|t| t.last4.clone()))
            });
            let mut line = Vendor1099LineOut {
                vendor_id: v.vendor_id,
                name: v.name,
                form_1099: form_1099_label(v.form),
                tin_on_file: v.tin_on_file,
                paid: money(v.paid),
                reportable: v.reportable,
                party_id,
                tin_last4,
                tin: None,
                tin_type: None,
                tin_note: None,
            };
            if let Some(vault) = &vault {
                match party_id.map(|pid| vault_tin(ledger, vault, pid)) {
                    None => line.tin_note = Some("no party for this vendor".into()),
                    Some(Err(why)) => line.tin_note = Some(why),
                    Some(Ok(t)) => {
                        line.tin = Some(t.formatted());
                        line.tin_type = Some(tin_type_key(t.kind).to_string());
                    }
                }
            }
            vendors.push(line);
        }
        let warning = p.include_tins.then(|| {
            "This worksheet holds full TINs: for the CPA only. Don't paste it into email, chat, notes, or the books; delete any saved copy once the CPA has it."
                .to_string()
        });
        let saved_csv = match &p.save_csv_to {
            None => None,
            Some(path) => {
                let dest = if p.include_tins {
                    self.tin_file_dest(&store, path)?
                } else {
                    outside_books(&store, path)?
                };
                let mut csv =
                    String::from("vendor_id,name,form_1099,paid,reportable,w9_on_file,tin_last4");
                if p.include_tins {
                    csv.push_str(",tin,tin_type,tin_note");
                }
                csv.push('\n');
                for l in &vendors {
                    let mut cells = vec![
                        l.vendor_id.to_string(),
                        csv_escape(&l.name),
                        l.form_1099.clone().unwrap_or_default(),
                        l.paid.clone(),
                        l.reportable.to_string(),
                        l.tin_on_file.to_string(),
                        l.tin_last4.clone().unwrap_or_default(),
                    ];
                    if p.include_tins {
                        cells.push(l.tin.clone().unwrap_or_default());
                        cells.push(l.tin_type.clone().unwrap_or_default());
                        cells.push(csv_escape(l.tin_note.as_deref().unwrap_or_default()));
                    }
                    csv.push_str(&cells.join(","));
                    csv.push('\n');
                }
                if p.include_tins {
                    Self::wrote_tins(&store, csv.as_bytes())?;
                }
                write_private_new(&dest, csv.as_bytes())?;
                Some(dest.display().to_string())
            }
        };
        Ok(Json(Vendor1099Out {
            year: r.year,
            threshold: money(r.threshold),
            vendors,
            missing_w9,
            notes,
            warning,
            saved_csv,
        }))
    }

    #[tool(
        name = "books.chart_template",
        title = "Starter chart of accounts",
        description = "A starter chart of accounts for a kind of business (sole proprietorship, partnership or multi-member LLC, S corporation, C corporation): the right equity and owner-pay accounts (draws; per-partner capital and distributions plus guaranteed payments; officer compensation and distributions; dividends and income tax), common income and expense accounts with tax categories assigned, and the settings that go with it. A proposal: review it with the human, adjust, then books.import_chart_csv.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn chart_template(
        &self,
        params: Parameters<ChartTemplateParams>,
    ) -> Result<Json<ChartTemplateOut>, String> {
        let p = params.0;
        let entity = ledger_import::EntityType::parse(&p.entity_type).ok_or_else(|| {
            format!(
                "entity_type {:?}: expected sole_proprietorship, partnership, s_corporation, or c_corporation (an LLC is one of these for tax purposes)",
                p.entity_type
            )
        })?;
        let t = ledger_import::chart_template(entity, &p.owners)?;
        Ok(Json(ChartTemplateOut {
            csv: t.csv(),
            settings_toml: t.settings_toml,
            notes: t.notes,
        }))
    }

    #[tool(
        name = "books.tax_categories",
        title = "Tax categories",
        description = "The tax categories an account can map to (where it lands on the income tax return), filtered to the business's return when the settings say which (tax_form or tax_classification). Each has a default deductible percent (meals 50, nondeductible 0) and guidance. Categories are stable keys, not form line numbers.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn tax_categories(&self) -> Result<Json<TaxCategoriesOut>, String> {
        let form = self.settings().company.tax_form;
        let categories = ledger_core::tax::CATEGORIES
            .iter()
            .filter(|c| form.is_none_or(|f| c.forms.contains(&f)))
            .map(|c| TaxCategoryOut {
                key: c.key.to_string(),
                label: c.label.to_string(),
                kind: match c.kind {
                    ledger_core::TaxKind::Income => "income",
                    ledger_core::TaxKind::CostOfGoods => "cost_of_goods",
                    ledger_core::TaxKind::Deduction => "deduction",
                }
                .to_string(),
                default_deductible_percent: c.default_deductible_percent,
                note: c.note.to_string(),
            })
            .collect();
        Ok(Json(TaxCategoriesOut {
            form: form.map(|f| f.label().to_string()),
            categories,
        }))
    }

    #[tool(
        name = "books.renumber_account",
        title = "Renumber an account",
        description = "Give an account a new code. Only the code changes: its id, name, and every entry posted to it stay, so balances and reports don't move. Refuses a code in use, or a control account's code (invoices, bills, payroll, closes) for an account of the wrong type. Lists any settings that still name the old code. Never renumber the human's accounts on your own: only when they ask.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn renumber_account(
        &self,
        params: Parameters<RenumberAccountParams>,
    ) -> Result<Json<RenumberAccountOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let id = resolve_account(&store, &p.account)?;
        let previous_code = store
            .ledger()
            .chart()
            .get(id)
            .expect("resolved")
            .code
            .clone();
        store
            .renumber_account(id, &p.code)
            .map_err(|e| e.to_string())?;
        Ok(Json(RenumberAccountOut {
            id,
            config_mentions: self
                .config_mentions(&previous_code)
                .into_iter()
                .chain(convention_mentions(store.ledger(), &previous_code))
                .collect(),
            previous_code,
            code: p.code,
        }))
    }

    #[tool(
        name = "books.label_asset",
        title = "Label a fixed asset",
        description = "Change a fixed asset's description, link it to the entry that acquired it (acquisition_entry), or set or change its tag (the label on the thing: letters, digits, - _ . /, at most 32; unique ignoring case and never reused by another asset, even after a retag or disposal) and its manufacturer serial number. The asset is its id or current tag. Journaled. Print labels with books.asset_labels.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn label_asset(
        &self,
        params: Parameters<LabelAssetParams>,
    ) -> Result<Json<LabelAssetOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let id = resolve_asset_ref(&store, &p.asset)?;
        let clear = p.clear_serial.unwrap_or(false);
        if clear && p.serial.is_some() {
            return Err("pass serial or clear_serial, not both".into());
        }
        if p.tag.is_none()
            && p.serial.is_none()
            && !clear
            && p.acquisition_entry.is_none()
            && p.description.is_none()
        {
            return Err(
                "nothing to change: pass tag, serial, clear_serial, description, or acquisition_entry"
                    .into(),
            );
        }
        if let Some(d) = &p.description {
            store
                .ledger()
                .check_asset_description(id, d)
                .map_err(|e| e.to_string())?;
        }
        if let Some(entry) = p.acquisition_entry {
            store
                .ledger()
                .check_asset_acquisition(id, entry)
                .map_err(|e| e.to_string())?;
        }
        // Both checked first, so a bad serial doesn't leave a new tag behind.
        if let Some(tag) = &p.tag {
            store
                .ledger()
                .check_asset_tag_for(Some(id), tag)
                .map_err(|e| e.to_string())?;
        }
        let serial = if clear { None } else { p.serial.as_deref() };
        if p.serial.is_some() || clear {
            store
                .ledger()
                .check_asset_serial(id, serial)
                .map_err(|e| e.to_string())?;
        }
        if let Some(tag) = &p.tag {
            store.set_asset_tag(id, tag).map_err(|e| e.to_string())?;
        }
        if p.serial.is_some() || clear {
            store
                .set_asset_serial(id, serial)
                .map_err(|e| e.to_string())?;
        }
        if let Some(entry) = p.acquisition_entry {
            store
                .set_asset_acquisition(id, entry)
                .map_err(|e| e.to_string())?;
        }
        if let Some(d) = &p.description {
            store
                .set_asset_description(id, d)
                .map_err(|e| e.to_string())?;
        }
        let a = store.ledger().get_asset(id).expect("resolved");
        Ok(Json(LabelAssetOut {
            asset_id: id,
            tag: a.tag.clone(),
            serial: a.serial.clone(),
            acquisition_entry: a.acquired_by,
            description: a.description.clone(),
        }))
    }

    #[tool(
        name = "books.correct_asset",
        title = "Correct a fixed asset",
        description = "Correct a registered fixed asset's cost, salvage value, useful life, or in-service date when it was registered wrong, with the reason; omitted fields keep their values. Refused once any depreciation for it is posted (correct with entries then). Its tag, serial, links, and tax basis record stay; correct the entry that put it on the books too, so the books and the register agree (books.export fixed_asset_register shows any difference). Journaled with the old figures.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn correct_asset(
        &self,
        params: Parameters<CorrectAssetParams>,
    ) -> Result<Json<AssetSummaryOne>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let id = resolve_asset_ref(&store, &p.asset)?;
        let a = store.ledger().get_asset(id).expect("resolved").clone();
        let base = store.ledger().base_currency();
        let fix = ledger_core::AssetCorrection {
            cost: match &p.cost {
                Some(c) => Money::new(parse_amount(c)?, base),
                None => a.cost,
            },
            salvage_value: match &p.salvage_value {
                Some(c) => Money::new(parse_amount(c)?, base),
                None => a.salvage_value,
            },
            useful_life_months: p.useful_life_months.unwrap_or(a.useful_life_months),
            purchase_date: match &p.in_service {
                Some(d) => parse_date(d)?,
                None => a.purchase_date,
            },
        };
        store
            .correct_asset(id, fix, &p.reason)
            .map_err(|e| e.to_string())?;
        let a = store.ledger().get_asset(id).expect("resolved");
        Ok(Json(AssetSummaryOne {
            asset_id: id,
            tag: a.tag.clone(),
            cost: money(a.cost.amount),
            salvage_value: money(a.salvage_value.amount),
            useful_life_months: a.useful_life_months,
            in_service: a.purchase_date.to_string(),
            tax: tax_out(a),
        }))
    }

    #[tool(
        name = "books.set_asset_tax_basis",
        title = "Set an asset's tax basis",
        description = "Record a fixed asset's federal tax basis beside its book value: rule cost (the company bought it: basis is its cost, IRC §1012), carryover (a member contributed business property: the member's adjusted basis, IRC §723), or personal_use_conversion (a member contributed personal-use property: the lesser of fair market value, which is the book cost, and the member's adjusted basis, Treas. Reg. §1.168(i)-4(b)). Contributions take the member's adjusted basis (what they paid, less any depreciation they took). Reports the tax basis and the built-in gain or loss (IRC §704(c)). Journaled; book figures don't change. Every asset should have one: the register and the CPA package show it.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn set_asset_tax_basis(
        &self,
        params: Parameters<SetAssetTaxBasisParams>,
    ) -> Result<Json<TaxBasisOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let id = resolve_asset_ref(&store, &p.asset)?;
        let tax = parse_tax_basis(&p.rule, p.contributor_basis.as_deref())?;
        store
            .set_asset_tax_basis(id, tax)
            .map_err(|e| e.to_string())?;
        let a = store.ledger().get_asset(id).expect("resolved");
        Ok(Json(tax_out(a).expect("just set")))
    }

    #[tool(
        name = "books.asset_labels",
        title = "Asset label sheet",
        description = "A printable sheet of asset labels (PDF, base64): each label shows the company, the tag, a QR code of the tag, the description, and the serial number. Avery 5160/8160 (US letter, 30 labels of 1\" x 2-5/8\"); skip reuses a partly used sheet. Omit assets for every active asset with a tag. Save the file and tell the human to print at 100% scale.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn asset_labels(
        &self,
        params: Parameters<AssetLabelsParams>,
    ) -> Result<Json<AssetLabelsOut>, String> {
        use base64::Engine as _;
        let p = params.0;
        let store = self.lock()?;
        let ledger = store.ledger();
        let assets: Vec<&ledger_core::Asset> = match &p.assets {
            Some(refs) => refs
                .iter()
                .map(|r| {
                    let id = resolve_asset_ref(&store, r)?;
                    Ok(ledger.get_asset(id).expect("resolved"))
                })
                .collect::<Result<_, String>>()?,
            None => ledger
                .registered_assets()
                .filter(|a| a.status == ledger_core::AssetStatus::Active && a.tag.is_some())
                .collect(),
        };
        let labels = assets
            .iter()
            .map(|a| {
                Ok(ledger_import::AssetLabel {
                    tag: a.tag.clone().ok_or_else(|| {
                        format!(
                            "asset {} has no tag: give it one with books.label_asset first",
                            a.id
                        )
                    })?,
                    description: a.description.clone(),
                    serial: a.serial.clone(),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let pdf = ledger_import::asset_labels_pdf(
            &labels,
            &self.settings().company.name,
            ledger_import::LabelSheet::Avery5160,
            p.skip.unwrap_or(0) as usize,
        )
        .map_err(|e| e.to_string())?;
        Ok(Json(AssetLabelsOut {
            file_name: format!("asset-labels-{}.pdf", today()),
            content_base64: base64::engine::general_purpose::STANDARD.encode(pdf),
            tags: labels.into_iter().map(|l| l.tag).collect(),
        }))
    }

    #[tool(
        name = "books.settings",
        title = "Settings in force",
        description = "The settings in force (company, fiscal year, tax form, members, basis, receipt rule with every account it requires or exempts, cash flow roles, close target, time zone, approval rules), every key the books know with its value and who set it, where they come from (source: \"books\", changed with books.set_setting; or \"file\", a settings file still read, reloaded on change: reload_error says why an edit didn't load), and problems: settings or conventions that disagree with the chart, each saying what to change.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn settings_tool(&self) -> Result<Json<SettingsOut>, String> {
        // The books first: their settings sync as they lock, so the
        // status and the settings read after are one version.
        let store = self.lock()?;
        let status = self.live().status();
        let cfg = self.settings();
        let c = &cfg.company;
        Ok(Json(SettingsOut {
            source: if status.journaled { "books" } else { "file" }.to_string(),
            timezone: cfg.timezone.clone(),
            today: today().to_string(),
            approval_rules: match store.ledger().settings().approval_rules() {
                ledger_core::settings::ApprovalMode::Off => "off",
                ledger_core::settings::ApprovalMode::Enforce => "enforce",
            }
            .to_string(),
            keys: kept::keys_out(&self.settings_as_kept(&store)),
            books_toml: status.books_toml.map(|p| p.display().to_string()),
            books_toml_exists: status.books_toml_exists,
            config_json_present: status.config_json_present,
            reload_error: status.error,
            loads: status.loads,
            company: c.name.clone(),
            fiscal_year_start: format!("{:02}-{:02}", c.fiscal_year_start.0, c.fiscal_year_start.1),
            tax_form: c.tax_form.map(|f| f.label().to_string()),
            basis: c.basis.clone(),
            receipt_rule: cfg.policy.rule(self.currency()),
            receipt_required_accounts: cfg.policy.receipt_required_accounts.clone(),
            receipt_exempt_accounts: cfg.policy.receipt_exempt_accounts.clone(),
            parties: cfg.parties.key().to_string(),
            members: c
                .members
                .iter()
                .map(|m| MemberSettingOut {
                    name: m.name.clone(),
                    share: m.share.normalize().to_string(),
                    capital_account: m.capital_account.clone(),
                    distribution_account: m.distribution_account.clone(),
                    files_jointly_with: m.files_jointly_with.clone(),
                })
                .collect(),
            cash_accounts: cfg.roles.cash.clone(),
            financing_accounts: cfg.roles.financing.clone(),
            investing_accounts: cfg.roles.investing.clone(),
            close_into: c.close_into.clone(),
            close_sweep: c.close_sweep.clone(),
            problems: settings_problems(store.ledger(), &cfg),
        }))
    }

    #[tool(
        name = "books.roles",
        title = "Control account roles",
        description = "The control roles invoices, bills, payroll, and closes post to (receivables, revenue, payables, general expense, retained earnings, payroll expense and withholdings), and the account filling each: the one assigned with books.assign_role, else the account at the role's default code.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn roles(&self) -> Result<Json<RolesOut>, String> {
        let store = self.lock()?;
        Ok(Json(RolesOut {
            roles: roles_out(store.ledger()),
        }))
    }

    #[tool(
        name = "books.assign_role",
        title = "Assign a control role",
        description = "Assign a control role (see books.roles) one of the chart's own accounts, so invoices, bills, payroll, or closes post there instead of the default code; account null returns the role to its default code. Journaled. Refuses an account of the wrong type, a placeholder, one already filling another role, and moving a receivables, payables, withholding, or retained earnings role while its account holds a balance (move the balance with an entry first). Posted entries never change. Only when the human agrees.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn assign_role(
        &self,
        params: Parameters<AssignRoleParams>,
    ) -> Result<Json<RolesOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let account = match &p.account {
            Some(a) => Some(resolve_account(&store, a)?),
            None => None,
        };
        store
            .assign_role(&p.role, account)
            .map_err(|e| e.to_string())?;
        Ok(Json(RolesOut {
            roles: roles_out(store.ledger()),
        }))
    }

    #[tool(
        name = "books.set_tax_category",
        title = "Set an account's tax category",
        description = "Map a revenue or expense account to a tax category (see books.tax_categories), optionally overriding its deductible percent; category null clears it. Drives the tax_summary report.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn set_tax_category(
        &self,
        params: Parameters<SetTaxCategoryParams>,
    ) -> Result<Json<SetTaxCategoryOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let id = resolve_account(&store, &p.account)?;
        let pct = match &p.deductible_percent {
            Some(s) => Some(parse_amount(s)?),
            None => None,
        };
        if let Some(cat) = &p.category {
            let t = store
                .ledger()
                .chart()
                .get(id)
                .expect("resolved")
                .account_type;
            ledger_core::tax::check_category(cat, t, self.settings().company.tax_form)
                .map_err(|e| e.to_string())?;
        }
        store
            .set_account_tax(id, p.category.as_deref(), pct)
            .map_err(|e| e.to_string())?;
        Ok(Json(SetTaxCategoryOut { ok: true }))
    }

    #[tool(
        name = "books.import_chart_csv",
        title = "Import a chart of accounts",
        description = "Import a chart of accounts from CSV (header: code,name,type,parent_code). Types: asset, liability, equity, revenue, expense. Parents must come before children. Fails on the first bad row; nothing is half-imported.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn import_chart_csv(
        &self,
        params: Parameters<ImportChartCsvParams>,
    ) -> Result<Json<ImportChartCsvOut>, String> {
        let p = params.0;
        let rows = ledger_import::parse_chart_csv(&p.csv_text).map_err(|e| e.to_string())?;
        let mut store = self.lock()?;
        let parse_type = |t: &str| match t {
            "asset" => Ok(AccountType::Asset),
            "liability" => Ok(AccountType::Liability),
            "equity" => Ok(AccountType::Equity),
            "revenue" => Ok(AccountType::Revenue),
            "expense" => Ok(AccountType::Expense),
            _ => Err(format!("bad account type: {t}")),
        };
        // Rehearse the whole import on a copy of the ledger first, so a bad
        // row anywhere (a duplicate within the file, a parent listed after
        // its child or of another type, a blank code) refuses the file and
        // nothing is half-imported. The old up-front checks covered only
        // clashes with the existing chart and tax categories.
        let mut scratch = store.ledger().clone();
        for row in &rows {
            let account_type = parse_type(&row.account_type)?;
            let parent = match &row.parent_code {
                Some(code) => Some(
                    scratch
                        .chart()
                        .find_by_code(code)
                        .map(|a| a.id)
                        .ok_or_else(|| format!("{}: unknown parent code {code}", row.code))?,
                ),
                None => None,
            };
            let id = scratch
                .add_account(&row.code, &row.name, account_type, parent, false)
                .map_err(|e| format!("{}: {e}", row.code))?;
            if let Some(cat) = &row.tax_category {
                ledger_core::tax::check_category(
                    cat,
                    account_type,
                    self.settings().company.tax_form,
                )
                .map_err(|e| format!("{}: {e}", row.code))?;
                scratch
                    .set_account_tax(id, Some(cat), None)
                    .map_err(|e| format!("{}: {e}", row.code))?;
            }
        }
        let mut created = Vec::new();
        for row in rows {
            let account_type = match row.account_type.as_str() {
                "asset" => AccountType::Asset,
                "liability" => AccountType::Liability,
                "equity" => AccountType::Equity,
                "revenue" => AccountType::Revenue,
                "expense" => AccountType::Expense,
                _ => return Err(format!("bad account type: {}", row.account_type)),
            };
            let parent = match &row.parent_code {
                Some(code) => Some(
                    store
                        .ledger()
                        .chart()
                        .find_by_code(code)
                        .map(|a| a.id)
                        .ok_or_else(|| format!("unknown parent code: {code}"))?,
                ),
                None => None,
            };
            let id = store
                .add_account(&row.code, &row.name, account_type, parent, false)
                .map_err(|e| e.to_string())?;
            if let Some(cat) = &row.tax_category {
                store
                    .set_account_tax(id, Some(cat), None)
                    .map_err(|e| e.to_string())?;
            }
            created.push(row.code.clone());
        }
        let count = created.len();
        Ok(Json(ImportChartCsvOut { created, count }))
    }

    #[tool(
        name = "books.import_local_file",
        title = "File a local file",
        description = "File a receipt or document from a file on this computer: an absolute path to a regular file in the server's import folders (by default your downloads folder; save it there first). Files of the books themselves are never read. A file the server read or wrote with full TINs (a W-9, a 1099 worksheet, a CPA package or a file from one), and text holding a TIN the vault keeps or a number printed as an SSN or EIN is (digits 3-2-4 or 2-7 joined by hyphens), are refused: they never go into the books.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn import_local_file(
        &self,
        params: Parameters<ImportLocalFileParams>,
    ) -> Result<Json<ImportLocalFileOut>, String> {
        let p = params.0;
        // The name and type are those of the file read, where the path
        // really leads (a link named receipt.pdf to a CSV is a CSV).
        let (canonical, bytes) = self.read_local(&p.path)?;
        let file_name = p.file_name.unwrap_or_else(|| {
            canonical
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("file")
                .to_string()
        });
        let mime_type = mime_guess::from_path(&canonical)
            .first_or_octet_stream()
            .to_string();
        let mut store = self.lock()?;
        // A file the server read or wrote with full TINs, or text holding
        // one (the vault's, or one printed as an SSN or EIN is), never goes
        // into the books (tin_files.rs).
        self.refuse_tin_file(&store, &bytes, &mime_type, &file_name)?;
        let (attachment_id, sha256, deduplicated) = match p.category.as_str() {
            "receipt" => store
                .attach_receipt(
                    p.entry_id,
                    &file_name,
                    &mime_type,
                    &bytes,
                    p.description.as_deref(),
                )
                .map_err(|e| e.to_string())?,
            "document" => {
                let cat = p.document_category.as_deref().unwrap_or("other");
                store
                    .file_document(
                        cat,
                        &file_name,
                        &mime_type,
                        &bytes,
                        p.description.as_deref(),
                    )
                    .map_err(|e| e.to_string())?
            }
            _ => return Err("category must be \"receipt\" or \"document\"".to_string()),
        };
        Ok(Json(ImportLocalFileOut {
            attachment_id,
            sha256,
            deduplicated,
            file_name,
        }))
    }

    #[tool(
        name = "books.export",
        title = "Export a report",
        description = "Export a report. Reports: profit_and_loss, balance_sheet, cash_flow (needs cash_accounts), trial_balance (as_of), members_equity (multi-member LLC; members from the settings), general_ledger (by account: running balances, splits), general_journal (every entry in date order; alias journal), fixed_asset_register (as_of: every asset with tag, serial, cost, depreciation, book value, reconciled to the books). Formats: pdf and xlsx (base64) for the five statements and fixed_asset_register; xlsx, csv, json for every report; iif for general_journal (QuickBooks Desktop). Dates: start/end for period reports (start defaults to the first entry, end to today), as_of for balance_sheet, trial_balance, and fixed_asset_register.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn export(&self, params: Parameters<ExportParams>) -> Result<Json<ExportOut>, String> {
        let p = params.0;
        let format = p.format.to_lowercase();
        if !["csv", "json", "xlsx", "pdf", "iif"].contains(&format.as_str()) {
            return Err(format!(
                "bad format {:?}: expected \"csv\", \"json\", \"xlsx\", \"pdf\", or \"iif\"",
                p.format
            ));
        }
        let report = p.report.to_lowercase();
        if format == "iif" && report != "journal" {
            return Err("iif format is only valid for the journal report".to_string());
        }
        if format == "xlsx" || format == "pdf" {
            let bytes = if format == "xlsx" {
                self.export_xlsx(&report, &p)?
            } else {
                self.export_pdf(&report, &p)?
            };
            return Ok(Json(ExportOut {
                format,
                report,
                content: base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes),
                content_encoding: Some("base64".to_string()),
            }));
        }
        if format == "iif" {
            let content = self.export_iif()?;
            return Ok(Json(ExportOut {
                format,
                report,
                content,
                content_encoding: None,
            }));
        }
        let content = match report.as_str() {
            "journal" | "general_journal" => self.export_journal(&format)?,
            "trial_balance" => self.export_trial_balance(&format, p.as_of.as_deref())?,
            "general_ledger" => self.export_general_ledger(&format, &p)?,
            "members_equity" => self.export_members_equity(&format, &p)?,
            "tax_summary" => self.export_tax_summary(&format, &p)?,
            "profit_and_loss" => self.export_pnl(&format, p.start.as_deref(), p.end.as_deref())?,
            "balance_sheet" => self.export_balance_sheet(&format, p.as_of.as_deref())?,
            "cash_flow" => self.export_cash_flow(&format, &p)?,
            "fixed_asset_register" => {
                self.export_fixed_asset_register(&format, p.as_of.as_deref())?
            }
            _ => {
                return Err(format!(
                    "bad report {:?}: expected general_journal, general_ledger, trial_balance, profit_and_loss, balance_sheet, cash_flow, members_equity, tax_summary, or fixed_asset_register",
                    p.report
                ));
            }
        };
        Ok(Json(ExportOut {
            format,
            report,
            content,
            content_encoding: None,
        }))
    }

    /// Lay out a financial statement for rendering, from export params.
    fn layout_statement(
        &self,
        ledger: &Ledger,
        report: &str,
        p: &ExportParams,
    ) -> Result<ledger_import::Statement, String> {
        use ledger_import::report;
        let members = self.report_members();
        Ok(match report {
            "profit_and_loss" => report::profit_and_loss(&pnl_statement(
                ledger,
                p.start.as_deref(),
                p.end.as_deref(),
                None,
            )?),
            "balance_sheet" => {
                let bs = bs_statement(ledger, p.as_of.as_deref())?;
                let earnings = self.year_earnings(ledger, bs.as_of)?;
                report::balance_sheet(&bs, earnings, &members)
            }
            "cash_flow" => report::cash_flow(
                &self.cash_flow_for(
                    ledger,
                    p.start.as_deref(),
                    p.end.as_deref(),
                    &p.cash_accounts,
                    &p.financing_accounts,
                    &p.investing_accounts,
                )?,
                &members,
            ),
            "trial_balance" => {
                let as_of = match &p.as_of {
                    Some(s) => parse_date(s)?,
                    None => today(),
                };
                report::trial_balance(&reports::trial_balance_as_of(ledger, as_of), as_of)
            }
            "tax_summary" => {
                let (start, end) = statement_range(ledger, p.start.as_deref(), p.end.as_deref())?;
                report::tax_summary(
                    &ledger_core::tax_summary(ledger, start, end, self.settings().company.tax_form)
                        .map_err(|e| e.to_string())?,
                )
            }
            "members_equity" => {
                let (start, end) = statement_range(ledger, p.start.as_deref(), p.end.as_deref())?;
                let me =
                    ledger_core::members_equity(ledger, start, end, &self.member_capitals(ledger)?)
                        .map_err(|e| e.to_string())?;
                report::members_equity(&me)
            }
            _ => {
                return Err(format!(
                    "bad report {report:?}: expected profit_and_loss, balance_sheet, cash_flow, trial_balance, members_equity, or tax_summary"
                ));
            }
        })
    }

    /// Render a report to XLSX bytes via ledger-import.
    fn export_xlsx(&self, report: &str, p: &ExportParams) -> Result<Vec<u8>, String> {
        use ledger_import::{XlsxReport, report_to_xlsx};
        let store = self.lock()?;
        let meta = self.report_meta(&store);
        let ledger = store.ledger();
        if report == "fixed_asset_register" {
            let as_of = match p.as_of.as_deref() {
                Some(s) => parse_date(s)?,
                None => today(),
            };
            let r = ledger_core::fixed_asset_register(ledger, as_of).map_err(|e| e.to_string())?;
            return ledger_import::asset_register_xlsx(&r, &meta).map_err(|e| e.to_string());
        }
        let xlsx = |r: XlsxReport<'_>| report_to_xlsx(&r, &meta).map_err(|e| e.to_string());
        match report {
            "general_ledger" => {
                let (start, end) = statement_range(ledger, p.start.as_deref(), p.end.as_deref())?;
                let gl =
                    ledger_core::general_ledger(ledger, start, end).map_err(|e| e.to_string())?;
                xlsx(XlsxReport::GeneralLedger(&gl))
            }
            "general_journal" | "journal" => {
                let (start, end) = statement_range(ledger, p.start.as_deref(), p.end.as_deref())?;
                xlsx(XlsxReport::GeneralJournal(ledger, start, end))
            }
            _ => xlsx(XlsxReport::Statement(
                &self.layout_statement(ledger, report, p)?,
            )),
        }
    }

    /// Render a financial statement to PDF bytes via ledger-import.
    fn export_pdf(&self, report: &str, p: &ExportParams) -> Result<Vec<u8>, String> {
        if matches!(report, "general_ledger" | "general_journal" | "journal") {
            return Err(format!("{report} exports as xlsx, csv, or json, not pdf"));
        }
        let store = self.lock()?;
        let meta = self.report_meta(&store);
        if report == "fixed_asset_register" {
            let as_of = match p.as_of.as_deref() {
                Some(s) => parse_date(s)?,
                None => today(),
            };
            let r = ledger_core::fixed_asset_register(store.ledger(), as_of)
                .map_err(|e| e.to_string())?;
            return ledger_import::asset_register_pdf(&r, &meta).map_err(|e| e.to_string());
        }
        let st = self.layout_statement(store.ledger(), report, p)?;
        ledger_import::statement_pdf(&st, &meta).map_err(|e| e.to_string())
    }

    /// Render the journal as IIF text via ledger-import, declaring every
    /// account with its QuickBooks type: the `[books] cash_accounts` are
    /// BANK, the receivable and payable control accounts AR and AP, a
    /// registered asset's cost and accumulated depreciation FIXASSET, and
    /// the rest follow their wanderbooks type (other current asset, other
    /// current liability, equity, income, expense).
    fn export_iif(&self) -> Result<String, String> {
        use ledger_import::{QbAccountType as Qb, iif_accounts, journal_to_iif};
        let store = self.lock()?;
        let ledger = store.ledger();
        let fixed: std::collections::HashSet<AccountId> = ledger
            .registered_assets()
            .flat_map(|a| [a.cost_account, a.accum_depr_account])
            .collect();
        let receivables = ledger.role_account(&ledger_core::RECEIVABLES);
        let payables = ledger.role_account(&ledger_core::PAYABLES);
        let accounts = iif_accounts(ledger, |a| {
            if self.settings().roles.cash.contains(&a.code) {
                Qb::Bank
            } else if Some(a.id) == receivables {
                Qb::AccountsReceivable
            } else if Some(a.id) == payables {
                Qb::AccountsPayable
            } else if fixed.contains(&a.id) {
                Qb::FixedAsset
            } else {
                match a.account_type {
                    AccountType::Asset => Qb::OtherCurrentAsset,
                    AccountType::Liability => Qb::OtherCurrentLiability,
                    AccountType::Equity => Qb::Equity,
                    AccountType::Revenue => Qb::Income,
                    AccountType::Expense => Qb::Expense,
                }
            }
        });
        let entries: Vec<_> = ledger.posted_entries().collect();
        journal_to_iif(&accounts, &entries).map_err(|e| e.to_string())
    }

    // ------------------------------------------------------------------
    // Fixed assets
    // ------------------------------------------------------------------

    #[tool(
        name = "books.register_asset",
        title = "Register a fixed asset",
        description = "Register a fixed asset (equipment, vehicles, buildings). Links three accounts: cost (asset), accumulated depreciation (asset, contra), depreciation expense. Depreciation is straight-line, book basis only — tax depreciation (MACRS, Section 179) stays with the CPA.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn register_asset(
        &self,
        params: Parameters<RegisterAssetParams>,
    ) -> Result<Json<RegisterAssetOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let base = store.ledger().base_currency();
        let cost_account = resolve_account(&store, &p.cost_account)?;
        let accum_depr_account = resolve_account(&store, &p.accum_depr_account)?;
        let expense_account = resolve_account(&store, &p.expense_account)?;
        let tag = match p.tag {
            Some(t) => t,
            None => store.ledger().next_asset_tag(self.settings().tag_prefix()),
        };
        // Checked before registering, so a bad tag or serial registers
        // nothing.
        store
            .ledger()
            .check_asset_tag_for(None, &tag)
            .map_err(|e| e.to_string())?;
        if let Some(serial) = &p.serial {
            ledger_core::assets::check_serial(serial)
                .map_err(|why| format!("bad serial number: {why}"))?;
        }
        if let Some(entry) = p.acquisition_entry {
            let e = store
                .ledger()
                .get_entry(entry)
                .ok_or_else(|| format!("unknown entry {entry}"))?;
            if !e
                .lines()
                .iter()
                .any(|l| l.account == cost_account && l.side == Side::Debit)
                || e.status() != ledger_core::EntryStatus::Posted
            {
                return Err(format!(
                    "entry {entry} can't be the acquisition: it must be posted and debit the cost account"
                ));
            }
        }
        // Checked before registering, so a bad tax basis registers nothing
        // (found by the spec-blind MCP suite, 2026-10-15).
        let tax = match &p.tax_basis_rule {
            Some(rule) => {
                let tax = parse_tax_basis(rule, p.contributor_basis.as_deref())?;
                tax.check().map_err(|why| format!("bad tax basis: {why}"))?;
                Some(tax)
            }
            None if p.contributor_basis.is_some() => {
                return Err("contributor_basis needs tax_basis_rule".into());
            }
            None => None,
        };
        let asset_id = store
            .register_asset(NewAsset {
                description: p.description,
                purchase_date: parse_date(&p.purchase_date)?,
                cost: Money::new(parse_amount(&p.cost)?, base),
                salvage_value: Money::new(
                    parse_amount(p.salvage_value.as_deref().unwrap_or("0"))?,
                    base,
                ),
                useful_life_months: p.useful_life_months,
                cost_account,
                accum_depr_account,
                expense_account,
            })
            .map_err(|e| e.to_string())?;
        store
            .set_asset_tag(asset_id, &tag)
            .map_err(|e| e.to_string())?;
        if let Some(serial) = &p.serial {
            store
                .set_asset_serial(asset_id, Some(serial))
                .map_err(|e| e.to_string())?;
        }
        if let Some(entry) = p.acquisition_entry {
            store
                .set_asset_acquisition(asset_id, entry)
                .map_err(|e| e.to_string())?;
        }
        if let Some(tax) = tax {
            store
                .set_asset_tax_basis(asset_id, tax)
                .map_err(|e| e.to_string())?;
        }
        Ok(Json(RegisterAssetOut { asset_id, tag }))
    }

    #[tool(
        name = "books.asset_schedule",
        title = "Depreciation schedule",
        description = "Month-by-month straight-line depreciation schedule for an asset: period, amount, accumulated, book value. Partial first and last months are prorated by days; the lifetime total is exactly cost minus salvage.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn asset_schedule(
        &self,
        params: Parameters<AssetScheduleParams>,
    ) -> Result<Json<AssetScheduleOut>, String> {
        let p = params.0;
        let store = self.lock()?;
        let asset_id = resolve_asset(&store, p.asset_id, p.tag.as_deref())?;
        let asset = store
            .ledger()
            .get_asset(asset_id)
            .ok_or_else(|| format!("unknown asset {asset_id}"))?
            .clone();
        // The schedule clamps to the depreciation end internally, so an
        // unbounded default just means "the whole life".
        let through = match p.through {
            Some(t) => parse_date(&t)?,
            None => NaiveDate::MAX,
        };
        let schedule = depreciation_schedule(&asset, through)
            .into_iter()
            .map(|l| ScheduleLineOut {
                period: l.period,
                amount: money(l.amount),
                accumulated: money(l.accumulated),
                book_value: money(l.book_value),
            })
            .collect();
        Ok(Json(AssetScheduleOut {
            asset_id: asset.id,
            description: asset.description.clone(),
            cost: money(asset.cost.amount),
            salvage_value: money(asset.salvage_value.amount),
            useful_life_months: asset.useful_life_months,
            status: match asset.status {
                AssetStatus::Active => "active".to_string(),
                AssetStatus::Disposed { .. } => "disposed".to_string(),
            },
            schedule,
        }))
    }

    #[tool(
        name = "books.list_assets",
        title = "List fixed assets",
        description = "List all registered fixed assets with accumulated depreciation and book value as of today.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn list_assets(&self) -> Result<Json<ListAssetsOut>, String> {
        let store = self.lock()?;
        let today = today();
        let assets = store
            .ledger()
            .registered_assets()
            .map(|a| AssetSummary {
                asset_id: a.id,
                tag: a.tag.clone(),
                serial: a.serial.clone(),
                acquisition_entry: a.acquired_by,
                tax: tax_out(a),
                description: a.description.clone(),
                purchase_date: a.purchase_date.to_string(),
                cost: money(a.cost.amount),
                useful_life_months: a.useful_life_months,
                status: match a.status {
                    AssetStatus::Active => "active".to_string(),
                    AssetStatus::Disposed { .. } => "disposed".to_string(),
                },
                accumulated: money(accumulated_depreciation(a, today)),
                book_value: money(book_value(a, today)),
            })
            .collect();
        Ok(Json(ListAssetsOut { assets }))
    }

    #[tool(
        name = "books.run_depreciation",
        title = "Post depreciation",
        description = "Post one month of depreciation (Dr Depreciation Expense / Cr Accumulated Depreciation) for one asset, or for every active asset when asset_id is omitted. Idempotent: re-running a posted period returns the original entry instead of double-posting. Depreciation is an allocation, not money movement, so it is exempt from the receipt policy; the registered asset and its schedule are the evidence.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn run_depreciation(
        &self,
        params: Parameters<RunDepreciationParams>,
    ) -> Result<Json<RunDepreciationOut>, String> {
        let p = params.0;
        let (py, pm) = parse_period_mcp(&p.period)?;
        let period_end = last_day_of_month(py, pm)?;
        let mut store = self.lock()?;
        let targets: Vec<(u64, String)> = match p.asset_id {
            Some(id) => {
                let a = store
                    .ledger()
                    .get_asset(id)
                    .ok_or_else(|| format!("unknown asset {id}"))?;
                vec![(id, a.description.clone())]
            }
            None => store
                .ledger()
                .registered_assets()
                .filter(|a| a.status == AssetStatus::Active)
                .map(|a| (a.id, a.description.clone()))
                .collect(),
        };
        let mut runs = Vec::with_capacity(targets.len());
        let mut skipped = Vec::new();
        for (id, description) in targets {
            // Run-all skips assets whose depreciable life doesn't touch the
            // period instead of failing the whole batch.
            if p.asset_id.is_none()
                && let Some(asset) = store.ledger().get_asset(id)
            {
                let touches = depreciation_schedule(asset, period_end)
                    .iter()
                    .any(|l| l.period == p.period);
                if !touches {
                    skipped.push(SkippedDepreciation {
                        asset_id: id,
                        description,
                        reason: format!(
                            "period {} is outside the asset's depreciable life",
                            p.period
                        ),
                    });
                    continue;
                }
            }
            let r = store
                .run_depreciation(id, &p.period)
                .map_err(|e| e.to_string())?;
            runs.push(DepreciationRunOut {
                asset_id: r.asset_id,
                description,
                period: r.period,
                amount: money(r.amount),
                entry_id: r.entry_id,
                already_posted: r.already_posted,
            });
        }
        Ok(Json(RunDepreciationOut { runs, skipped }))
    }

    #[tool(
        name = "books.dispose_asset",
        title = "Dispose of a fixed asset",
        description = "Dispose of a fixed asset (sell or retire). Without confirmed=true returns a preview — book value, gain/loss, the auto-posted final depreciation, and the disposal entry's lines — and posts nothing. With confirmed=true posts the disposal entry (plus any missing final-period depreciation) after enforcing the receipt policy: file the bill of sale with books.file_document first and pass its attachment ids as receipt_attachment_ids. Name the buyer (buyer: a party id or name): it is the entry's party, and [books] parties = \"require\" refuses a sale without one; a retirement with no proceeds and no buyer is marked as having no counterparty.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn dispose_asset(
        &self,
        params: Parameters<DisposeAssetParams>,
    ) -> Result<Json<DisposeAssetOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let date = parse_date(&p.date)?;
        let base = store.ledger().base_currency();
        let proceeds = Money::new(parse_amount(&p.proceeds)?, base);
        let proceeds_account = resolve_account(&store, &p.proceeds_account)?;
        let gain_loss_account = resolve_account(&store, &p.gain_loss_account)?;
        let asset_id = resolve_asset(&store, p.asset_id, p.tag.as_deref())?;
        let buyer = p
            .buyer
            .as_deref()
            .map(|r| resolve_party(&store, r))
            .transpose()?;
        let asset = store
            .ledger()
            .get_asset(asset_id)
            .ok_or_else(|| format!("unknown asset {asset_id}"))?
            .clone();
        let preview = store
            .dispose_preview(asset_id, date, proceeds)
            .map_err(|e| e.to_string())?;

        // The would-be disposal entry's lines, for the preview and the
        // receipt-policy check.
        let raw: Vec<(AccountId, Side, Money)> = Store::disposal_lines(
            &asset,
            &preview,
            proceeds.amount,
            proceeds_account,
            gain_loss_account,
        )
        .into_iter()
        .map(|(a, side, amount)| (a, side, Money::new(amount, base)))
        .collect();
        // Disposals are money movement: the bill of sale is the receipt.
        let chart = store.ledger().chart();
        let code_of = |id: AccountId| {
            chart
                .get(id)
                .map(|a| a.code.clone())
                .ok_or_else(|| format!("unknown account {id}"))
        };
        let codes: Vec<String> = [
            asset.accum_depr_account,
            proceeds_account,
            asset.cost_account,
            gain_loss_account,
        ]
        .iter()
        .map(|id| code_of(*id))
        .collect::<Result<_, _>>()?;
        let code_refs: Vec<&str> = codes.iter().map(|s| s.as_str()).collect();
        let debits = preview.accumulated
            + proceeds.amount
            + if preview.gain_loss < Decimal::ZERO {
                -preview.gain_loss
            } else {
                Decimal::ZERO
            };
        let receipt_required = self
            .settings()
            .policy
            .receipt_required_for(&code_refs, debits);

        let out = DisposeAssetOut {
            status: "preview".to_string(),
            asset_id,
            entry_id: 0,
            accumulated: money(preview.accumulated),
            book_value: money(preview.book_value),
            proceeds: money(proceeds.amount),
            gain_loss: money(preview.gain_loss),
            final_depreciation: money(preview.final_depreciation),
            lines: preview_lines_for(&store, &raw)?,
            receipt_required,
        };
        if !p.confirmed.unwrap_or(false) {
            return Ok(Json(out));
        }
        let attachment_ids = p.receipt_attachment_ids.unwrap_or_default();
        if receipt_required && attachment_ids.is_empty() {
            return Err(ReceiptRequiredError {
                code: "receipt_required".to_string(),
                message: format!(
                    "disposing {} moves {} in debits ({}); file the bill of sale with books.file_document, then retry with receipt_attachment_ids",
                    asset.description,
                    money(debits),
                    self.settings().policy.describe()
                ),
                entry_id: None,
                asset_id: Some(asset_id),
                debits: debits.to_string(),
                policy: self.settings().policy.describe(),
                remedy_tool: "books.file_document".to_string(),
                remedy_hint:
                    "file the bill of sale with books.file_document, then retry books.dispose_asset with confirmed=true and receipt_attachment_ids=[<id>]"
                        .to_string(),
                split_group: Vec::new(),
            }
            .to_tool_error());
        }
        for aid in &attachment_ids {
            store.check_linkable(*aid).map_err(|e| e.to_string())?;
        }
        if buyer.is_none()
            && !proceeds.amount.is_zero()
            && self.settings().parties == ledger_config::PartyPolicy::Require
        {
            return Err(party_required(
                "pass buyer: the party the asset was sold to",
            ));
        }
        let entry_id = store
            .dispose_asset(
                asset_id,
                date,
                proceeds,
                proceeds_account,
                gain_loss_account,
            )
            .map_err(|e| e.to_string())?;
        for aid in &attachment_ids {
            store
                .link_receipt(*aid, entry_id)
                .map_err(|e| e.to_string())?;
        }
        match buyer {
            Some(b) => store.set_entry_party(entry_id, Some(b)),
            None if proceeds.amount.is_zero() => {
                store.set_entry_no_party(entry_id, "asset retired with no proceeds")
            }
            None => Ok(()),
        }
        .map_err(|e| e.to_string())?;
        let mut posted = out;
        posted.status = "posted".to_string();
        posted.entry_id = entry_id;
        Ok(Json(posted))
    }

    // ------------------------------------------------------------------
    // Periods
    // ------------------------------------------------------------------

    #[tool(
        name = "books.create_period",
        title = "Create a period",
        description = "Create an accounting period (month, quarter, year). Dates are YYYY-MM-DD, inclusive.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn create_period(
        &self,
        params: Parameters<CreatePeriodParams>,
    ) -> Result<Json<CreatePeriodOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let id = store
            .create_period(&p.name, parse_date(&p.start)?, parse_date(&p.end)?)
            .map_err(|e| e.to_string())?;
        Ok(Json(CreatePeriodOut { period_id: id }))
    }

    #[tool(
        name = "books.close_period",
        title = "Close a period",
        description = "Close an accounting period. The closing entry, dated at period end, zeroes revenue and expense balances as of that date into Retained Earnings, or, for a multi-member LLC (members in the settings), into each member's capital account by share. Periods close in order. Without confirmed=true, returns the exact closing entry it would post and closes nothing. With confirmed=true, posts it and marks the period closed. Closed periods reject new entries.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn close_period(
        &self,
        params: Parameters<ClosePeriodParams>,
    ) -> Result<Json<ClosePeriodOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        // Verify the period exists.
        if store.ledger().get_period(p.period_id).is_none() {
            return Err(format!("unknown period {}", p.period_id));
        }
        let to = self.close_target(store.ledger())?;
        let describe = |ledger: &Ledger, eid: Option<ledger_core::EntryId>| -> Vec<String> {
            let Some(entry) = eid.and_then(|e| ledger.get_entry(e)) else {
                return Vec::new();
            };
            entry
                .lines()
                .iter()
                .map(|line| {
                    let code = ledger
                        .chart()
                        .get(line.account)
                        .map(|a| a.code.clone())
                        .unwrap_or_default();
                    format!("{:?} {} {}", line.side, code, money(line.amount.amount))
                })
                .collect()
        };
        if !p.confirmed {
            // Preview: run the real close on a copy of the ledger, so the
            // preview is exactly what confirming would post.
            let mut scratch = store.ledger().clone();
            let closing = scratch
                .close_period_to(p.period_id, &to)
                .map_err(|e| e.to_string())?;
            return Ok(Json(ClosePeriodOut {
                status: "preview".to_string(),
                period_id: p.period_id,
                closing_entry: None,
                lines: describe(&scratch, closing),
            }));
        }
        let closing = store
            .close_period_to(p.period_id, &to)
            .map_err(|e| e.to_string())?;
        let lines = describe(store.ledger(), closing);
        Ok(Json(ClosePeriodOut {
            status: "closed".to_string(),
            period_id: p.period_id,
            closing_entry: closing,
            lines,
        }))
    }

    #[tool(
        name = "books.list_periods",
        title = "List periods",
        description = "List all accounting periods with their status.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn list_periods(&self) -> Result<Json<ListPeriodsOut>, String> {
        let store = self.lock()?;
        let periods = store
            .ledger()
            .list_periods()
            .into_iter()
            .map(|p| PeriodOut {
                id: p.id,
                name: p.name.clone(),
                start: p.start.to_string(),
                end: p.end.to_string(),
                status: match p.status {
                    PeriodStatus::Open => "open".to_string(),
                    PeriodStatus::Closed => "closed".to_string(),
                },
                closing_entry: p.closing_entry,
            })
            .collect();
        Ok(Json(ListPeriodsOut { periods }))
    }

    // ------------------------------------------------------------------
    // Bank reconciliation
    // ------------------------------------------------------------------

    #[tool(
        name = "books.mark_cleared",
        title = "Mark a line cleared",
        description = "Mark a posted entry's line as cleared (or uncleared) for bank reconciliation. Only posted lines can be marked.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn mark_cleared(
        &self,
        params: Parameters<MarkClearedParams>,
    ) -> Result<Json<MarkClearedOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        store
            .mark_cleared(p.entry_id, p.line_index, p.cleared)
            .map_err(|e| e.to_string())?;
        Ok(Json(MarkClearedOut { ok: true }))
    }

    /// ```text
    /// GOAL: a monthly reconciliation that leaves proof: which statement,
    ///       which balance, which lines it cleared.
    /// SOURCE: CONVENTION (bank reconciliation); docs/parties-and-
    ///         attachments.md "Reconciliations"; Store::complete_reconciliation.
    /// INVARIANTS:
    ///   1. Without confirmed=true nothing is recorded: the preview reports
    ///      the difference, the cleared and uncleared lines, and the
    ///      problems a confirmation would hit.
    ///   2. A reconciliation is recorded only against a recorded statement
    ///      (statement_attachment), only at a difference of exactly zero,
    ///      and only after the account's last reconciliation not voided.
    ///   3. A refusal journals nothing.
    /// SCENARIOS:
    ///   1. IF statement_attachment is given: the account, date, and
    ///      balance are the statement's; any given that disagree: SHOULD
    ///      refuse. IF its attachment is withdrawn (the statement retired)
    ///      or its statement voided: SHOULD refuse, preview or not.
    ///   2. IF it isn't: account, statement_date, and statement_balance are
    ///      required, and confirmed=true SHOULD refuse.
    ///   3. IF confirmed=true and problems is not empty: SHOULD refuse,
    ///      naming them.
    /// ```
    #[tool(
        name = "books.reconcile",
        title = "Reconcile an account",
        description = "Reconcile a bank, cash, or card account against a statement. Give statement_attachment (a statement recorded with books.record_statement: its account, period end, and closing balance), or account, statement_date, and statement_balance for a quick check against any balance. Returns the book balance, cleared balance, statement balance, the difference (cleared minus statement; zero means reconciled), the cleared and uncleared lines through the statement date, and the problems that would stop it being recorded. Without confirmed=true it is a preview and records nothing. With confirmed=true and a recorded statement it records the reconciliation (journaled as reconciliation.completed, linked to the statement), refused unless the difference is exactly zero and it comes after the account's last reconciliation. Mark lines cleared with books.mark_cleared first.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn reconcile(
        &self,
        params: Parameters<ReconcileParams>,
    ) -> Result<Json<ReconcileOut>, String> {
        let p = params.0;
        let confirmed = p.confirmed.unwrap_or(false);
        let mut store = self.lock()?;
        let given_account = p
            .account
            .as_deref()
            .map(|a| resolve_account(&store, a))
            .transpose()?;
        let given_date = p.statement_date.as_deref().map(parse_date).transpose()?;
        let given_balance = p
            .statement_balance
            .as_deref()
            .map(parse_amount)
            .transpose()?;
        let (account, date, balance) = match p.statement_attachment {
            Some(att) => {
                if find_attachment(&store, att)?.withdrawn_at.is_some() {
                    return Err(format!(
                        "attachment {att} is withdrawn, so its statement is retired: file the statement again and record it (books.record_statement)"
                    ));
                }
                let st = store
                    .statement(att)
                    .map_err(|e| e.to_string())?
                    .ok_or_else(|| {
                        format!(
                            "attachment {att} records no statement: record it with books.record_statement first"
                        )
                    })?;
                if let Some(a) = given_account.filter(|a| *a != st.account_id) {
                    return Err(format!(
                        "statement {att} is for account {}, not {}",
                        code_name(&store, st.account_id).0,
                        code_name(&store, a).0
                    ));
                }
                if let Some(d) = given_date.filter(|d| *d != st.period_end) {
                    return Err(format!("statement {att} ends {}, not {d}", st.period_end));
                }
                if let Some(b) = given_balance.filter(|b| *b != st.closing_balance) {
                    return Err(format!(
                        "statement {att} closes at {}, not {}",
                        money(st.closing_balance),
                        money(b)
                    ));
                }
                (st.account_id, st.period_end, st.closing_balance)
            }
            None => {
                if confirmed {
                    return Err("a reconciliation is recorded only against a recorded statement: file it (books.file_document category statement), record what it says (books.record_statement), then pass statement_attachment".into());
                }
                match (given_account, given_date, given_balance) {
                    (Some(a), Some(d), Some(b)) => (a, d, b),
                    _ => {
                        return Err("give statement_attachment, or account, statement_date, and statement_balance".into());
                    }
                }
            }
        };
        let report = store
            .ledger()
            .reconciliation_report(account, date, balance)
            .map_err(|e| e.to_string())?;
        let ledger = store.ledger();
        let mut cleared_lines = Vec::new();
        for e in ledger.posted_entries() {
            if e.date() > date {
                continue;
            }
            for (i, l) in e.lines().iter().enumerate() {
                if l.account == account && ledger.is_cleared(e.id(), i) {
                    cleared_lines.push(reconcile_line(
                        e.id(),
                        i,
                        e.date(),
                        e.description(),
                        l.side,
                        l.amount.amount,
                    ));
                }
            }
        }
        cleared_lines.sort_by(|a, b| {
            a.date
                .cmp(&b.date)
                .then(a.entry_id.cmp(&b.entry_id))
                .then(a.line_index.cmp(&b.line_index))
        });
        let mut problems = Vec::new();
        if p.statement_attachment.is_none() {
            problems.push("no recorded statement: file it, books.record_statement, then pass statement_attachment".to_string());
        }
        if !report.difference.is_zero() {
            problems.push(format!(
                "the cleared balance differs from the statement by {}: mark the lines the statement shows with books.mark_cleared (or find what's missing)",
                money(report.difference)
            ));
        }
        if let Some(last) = reconciled_through(&store, account)?
            && last >= date
        {
            problems.push(format!(
                "already reconciled through {last}: void that reconciliation (books.void_reconciliation) to redo it"
            ));
        }
        let mut out = ReconcileOut {
            status: "preview".to_string(),
            account_code: code_name(&store, account).0,
            statement_date: date.to_string(),
            statement_attachment: p.statement_attachment,
            book_balance: money(report.book_balance),
            cleared_balance: money(report.cleared_balance),
            statement_balance: money(report.statement_balance),
            difference: money(report.difference),
            uncleared_lines: report
                .uncleared_lines
                .into_iter()
                .map(|l| {
                    reconcile_line(
                        l.entry_id,
                        l.line_index,
                        l.date,
                        &l.description,
                        l.side,
                        l.amount,
                    )
                })
                .collect(),
            cleared_lines,
            problems,
            reconciliation: None,
        };
        if !confirmed {
            return Ok(Json(out));
        }
        if !out.problems.is_empty() {
            return Err(format!("not recorded: {}", out.problems.join("; ")));
        }
        let att = p.statement_attachment.expect("required when confirmed");
        let r = store
            .complete_reconciliation(att)
            .map_err(|e| e.to_string())?;
        out.status = "reconciled".to_string();
        out.reconciliation = Some(reconciliation_out(&store, &r));
        Ok(Json(out))
    }

    /// ```text
    /// GOAL: the bank and card statements are in the books, so a
    ///       reconciliation names the one it agrees with.
    /// SOURCE: Store::record_statement.
    /// INVARIANTS:
    ///   1. Journaled as `statement.recorded`, then `attachment.linked` to
    ///      the account (unless linked already). A refusal journals nothing.
    /// SCENARIOS:
    ///   1. IF the attachment is unknown, withdrawn, or already records a
    ///      statement; the account is unknown or not an asset or liability;
    ///      the period runs backwards; or a balance isn't a decimal string
    ///      in cents: SHOULD refuse.
    /// ```
    #[tool(
        name = "books.record_statement",
        title = "Record a statement",
        description = "Record what a filed bank or card statement says (file it first: books.file_document category statement): the account it is for (code or id; an asset or liability), the period it covers (period_start, period_end: the statement date), and its opening and closing balances as printed, in the account's normal direction (what the bank holds; what the card is owed), decimal strings. Links the statement to the account. Then reconcile to it: books.reconcile statement_attachment. Journaled as statement.recorded. Returns the statement.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn record_statement(
        &self,
        params: Parameters<RecordStatementParams>,
    ) -> Result<Json<StatementOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        find_attachment(&store, p.attachment_id)?;
        let st = ledger_store::Statement {
            attachment_id: p.attachment_id,
            account_id: resolve_account(&store, &p.account)?,
            period_start: parse_date(&p.period_start)?,
            period_end: parse_date(&p.period_end)?,
            opening_balance: parse_amount(&p.opening_balance)?,
            closing_balance: parse_amount(&p.closing_balance)?,
        };
        store
            .record_statement(st.clone())
            .map_err(|e| e.to_string())?;
        Ok(Json(statement_out(&store, &st)?))
    }

    #[tool(
        name = "books.statements",
        title = "List statements",
        description = "Recorded bank and card statements (books.record_statement), optionally one account's (code or id), by statement date: the attachment, account, period, opening and closing balances, and the reconciliation made against it, if any.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn statements(
        &self,
        params: Parameters<AccountFilterParams>,
    ) -> Result<Json<StatementsOut>, String> {
        let store = self.lock()?;
        let account = params
            .0
            .account
            .as_deref()
            .map(|a| resolve_account(&store, a))
            .transpose()?;
        let statements = store
            .statements(account)
            .map_err(|e| e.to_string())?
            .iter()
            .map(|st| statement_out(&store, st))
            .collect::<Result<_, _>>()?;
        Ok(Json(StatementsOut { statements }))
    }

    #[tool(
        name = "books.reconciliations",
        title = "List reconciliations",
        description = "Recorded reconciliations (books.reconcile confirmed), optionally one account's (code or id), by statement date: the account, the statement date and balance, the statement's attachment, the lines it found cleared, and whether it was voided (and why).",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn reconciliations(
        &self,
        params: Parameters<AccountFilterParams>,
    ) -> Result<Json<ReconciliationsOut>, String> {
        let store = self.lock()?;
        let account = params
            .0
            .account
            .as_deref()
            .map(|a| resolve_account(&store, a))
            .transpose()?;
        let reconciliations = store
            .reconciliations(account)
            .map_err(|e| e.to_string())?
            .iter()
            .map(|r| reconciliation_out(&store, r))
            .collect();
        Ok(Json(ReconciliationsOut { reconciliations }))
    }

    /// ```text
    /// GOAL: correct a mis-recorded statement on the record.
    /// SOURCE: Store::void_statement.
    /// SCENARIOS:
    ///   1. IF the attachment records no statement, a reconciliation not
    ///      voided rests on it, or the reason is blank: SHOULD refuse,
    ///      journaling nothing.
    ///   2. Otherwise: journaled as `statement.voided`; books.statements no
    ///      longer lists it, and the same attachment can record the
    ///      statement again (books.record_statement).
    /// ```
    #[tool(
        name = "books.void_statement",
        title = "Void a statement",
        description = "Void a recorded statement that was recorded wrong (a mistyped balance or period), with the reason recorded in the journal, so the same filed statement can be recorded again with books.record_statement. Refused while a reconciliation not voided rests on it (books.void_reconciliation first). Nothing is erased: the journal keeps what it said. Journaled as statement.voided. Returns the statement as it was recorded.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn void_statement(
        &self,
        params: Parameters<VoidStatementParams>,
    ) -> Result<Json<StatementOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let st = store
            .statement(p.attachment_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("attachment {} records no statement", p.attachment_id))?;
        let out = statement_out(&store, &st)?;
        store
            .void_statement(p.attachment_id, &p.reason)
            .map_err(|e| e.to_string())?;
        Ok(Json(out))
    }

    /// ```text
    /// GOAL: undo a mistaken reconciliation on the record, to redo it.
    /// SOURCE: Store::void_reconciliation.
    /// SCENARIOS:
    ///   1. IF it is unknown or already voided, or the reason is blank:
    ///      SHOULD refuse, journaling nothing.
    ///   2. Otherwise: journaled as `reconciliation.voided`; it stays
    ///      listed, marked voided, and the account can be reconciled
    ///      through that date again.
    /// ```
    #[tool(
        name = "books.void_reconciliation",
        title = "Void a reconciliation",
        description = "Void a recorded reconciliation that was a mistake, with the reason recorded in the journal, so the account can be reconciled through that date again. Nothing is erased: it stays listed in books.reconciliations, marked voided. Journaled as reconciliation.voided. Returns it.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn void_reconciliation(
        &self,
        params: Parameters<VoidReconciliationParams>,
    ) -> Result<Json<ReconciliationOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        store
            .void_reconciliation(p.reconciliation_id, &p.reason)
            .map_err(|e| e.to_string())?;
        let r = store
            .reconciliations(None)
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|r| r.id == p.reconciliation_id)
            .ok_or_else(|| format!("unknown reconciliation {}", p.reconciliation_id))?;
        Ok(Json(reconciliation_out(&store, &r)))
    }

    // ------------------------------------------------------------------
    // A/R and A/P
    // ------------------------------------------------------------------

    #[tool(
        name = "books.add_customer",
        title = "Add a customer",
        description = "Add a customer record for accounts receivable (invoices). It links to the party of the same name that has no customer record yet (ignoring case), else to a new party; the output names both ids.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn add_customer(
        &self,
        params: Parameters<AddRecordParams>,
    ) -> Result<Json<AddRecordOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let id = store.add_customer(&p.name).map_err(|e| e.to_string())?;
        let party_id = store
            .ledger()
            .party_of_customer(id)
            .ok_or_else(|| format!("customer {id} has no party"))?;
        Ok(Json(AddRecordOut { id, party_id }))
    }

    #[tool(
        name = "books.add_vendor",
        title = "Add a vendor",
        description = "Add a vendor record for accounts payable (bills) and 1099s. It links to the party of the same name that has no vendor record yet (ignoring case), else to a new party; the output names both ids.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn add_vendor(
        &self,
        params: Parameters<AddRecordParams>,
    ) -> Result<Json<AddRecordOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let id = store.add_vendor(&p.name).map_err(|e| e.to_string())?;
        let party_id = store
            .ledger()
            .party_of_vendor(id)
            .ok_or_else(|| format!("vendor {id} has no party"))?;
        Ok(Json(AddRecordOut { id, party_id }))
    }

    // ------------------------------------------------------------------
    // Parties (ledger_core::parties)
    // ------------------------------------------------------------------

    #[tool(
        name = "books.add_party",
        title = "Add a party",
        description = "Add a party to the directory: anyone the company deals with (a contractor, landlord, payout platform, tax agency, government agency, bank or card issuer, member, employee). Name it as on its documents (unique among parties not archived); give its roles: contractor, employee, member, payout_platform, tax_agency, government_agency, financial_institution, landlord. Vendor and customer aren't given by hand: books.add_vendor and books.add_customer add those records and link them to the party of the same name. Never put a full TIN or SSN in any party field: the engine refuses nine-digit numbers. Journaled as party.added; returns the party.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn add_party(
        &self,
        params: Parameters<AddPartyParams>,
    ) -> Result<Json<PartyOut>, String> {
        let p = params.0;
        let mut roles = parse_roles(&p.roles)?;
        roles.sort();
        roles.dedup();
        let mut store = self.lock()?;
        let id = store
            .add_party(&p.name, &roles)
            .map_err(|e| e.to_string())?;
        let party = store.ledger().get_party(id).expect("just added");
        Ok(Json(party_out(party)))
    }

    #[tool(
        name = "books.update_party",
        title = "Update a party",
        description = "Change a party's facts: name, display_name, website, address, notes, archived. Give only the fields to change; an empty string clears display_name, website, address, or notes. Every field is checked before any is written, so a bad one (a blank or taken name, a full TIN, over 500 characters) changes nothing; each is journaled as its own party.updated. Archive a party you no longer deal with (its history stays; it then goes by id). Returns the party.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn update_party(
        &self,
        params: Parameters<UpdatePartyParams>,
    ) -> Result<Json<PartyOut>, String> {
        use ledger_core::PartyField as F;
        let p = params.0;
        let mut store = self.lock()?;
        let id = resolve_party(&store, &p.party)?;
        let mut fields = Vec::new();
        if let Some(n) = p.name {
            fields.push(F::Name(n));
        }
        if let Some(v) = p.display_name {
            fields.push(F::DisplayName(optional_text(&v)));
        }
        if let Some(v) = p.website {
            fields.push(F::Website(optional_text(&v)));
        }
        if let Some(v) = p.address {
            fields.push(F::Address(optional_text(&v)));
        }
        if let Some(v) = p.notes {
            fields.push(F::Notes(optional_text(&v)));
        }
        if let Some(a) = p.archived {
            fields.push(F::Archived(a));
        }
        if fields.is_empty() {
            return Err(
                "give at least one field to change: name, display_name, website, address, notes, archived"
                    .into(),
            );
        }
        // Every field, in order, before any is written (INVARIANT 2).
        let mut rehearsal = store.ledger().clone();
        for f in &fields {
            rehearsal
                .update_party(id, f.clone())
                .map_err(|e| e.to_string())?;
        }
        for f in fields {
            store.update_party(id, f).map_err(|e| e.to_string())?;
        }
        let party = store.ledger().get_party(id).expect("resolved");
        Ok(Json(party_out(party)))
    }

    #[tool(
        name = "books.set_party_roles",
        title = "Set a party's roles",
        description = "Give a party roles (add) or take them away (remove): contractor, employee, member, payout_platform, tax_agency, government_agency, financial_institution, landlord. Vendor and customer follow their records and can't be set by hand. Adds apply before removes; a role it already has (or lacks) is left alone. Removing a party's last 1099 role (contractor, landlord) needs its W-9 facts cleared first (books.set_w9 clear), and removing employee its W-4 facts. All or nothing: a refusal changes nothing. Each change is journaled as party.role_set. Returns the party.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn set_party_roles(
        &self,
        params: Parameters<SetPartyRolesParams>,
    ) -> Result<Json<PartyOut>, String> {
        let p = params.0;
        let add = parse_roles(&p.add)?;
        let remove = parse_roles(&p.remove)?;
        if add.is_empty() && remove.is_empty() {
            return Err("give at least one role to add or remove".into());
        }
        if let Some(r) = add.iter().find(|r| remove.contains(r)) {
            return Err(format!("role {} is both added and removed", r.key()));
        }
        if add
            .iter()
            .chain(&remove)
            .any(|r| matches!(r, ledger_core::Role::Vendor | ledger_core::Role::Customer))
        {
            return Err(
                "vendor and customer roles follow their records: books.add_vendor or books.add_customer"
                    .into(),
            );
        }
        let mut store = self.lock()?;
        let id = resolve_party(&store, &p.party)?;
        let has = store
            .ledger()
            .get_party(id)
            .expect("resolved")
            .roles
            .clone();
        let mut changes: Vec<(ledger_core::Role, bool)> = Vec::new();
        for r in add {
            if !has.contains(&r) && !changes.contains(&(r, true)) {
                changes.push((r, true));
            }
        }
        for r in remove {
            if has.contains(&r) && !changes.contains(&(r, false)) {
                changes.push((r, false));
            }
        }
        // The whole change, in order, before any is written (INVARIANT 2).
        let mut rehearsal = store.ledger().clone();
        for (r, on) in &changes {
            rehearsal
                .set_party_role(id, *r, *on)
                .map_err(|e| e.to_string())?;
        }
        for (r, on) in changes {
            store.set_party_role(id, r, on).map_err(|e| e.to_string())?;
        }
        let party = store.ledger().get_party(id).expect("resolved");
        Ok(Json(party_out(party)))
    }

    /// ```text
    /// GOAL: the books know a W-9 arrived and what it said, and the full
    ///       TIN (and the form, which shows it) is kept where it can be
    ///       handed to the CPA and purged, never in the journal.
    /// SOURCE: docs/parties-and-attachments.md "The TIN vault"; B-26;
    ///         Treas. Reg. §301.6109-4 (truncated TINs); DECISION (owner,
    ///         2026-10-15): full TINs only in the encrypted vault.
    /// INVARIANTS:
    ///   1. A full TIN goes only to the vault (`vault/` in the books dir,
    ///      key outside it, created on first use); the facts journaled
    ///      (`party.w9_set`) carry its last four and the vault record's
    ///      hash. The output never holds more than the last four.
    ///   2. The W-9 file goes only to the vault, never the attachment store.
    ///   3. The facts are checked (on a copy of the ledger) before the vault
    ///      is written; a refusal journals nothing and writes no record.
    ///   4. The facts never disagree with the vault: a TIN in the vault has
    ///      the facts' last four.
    /// SCENARIOS:
    ///   1. IF tin is given: SHOULD validate it (Tin::new, with tin_type),
    ///      take tin_last4 from it (refusing a given one that differs), and
    ///      default kept_at to "wanderbooks vault".
    ///   2. IF no tin is given but the vault holds one whose last four
    ///      differ: SHOULD refuse (pass the new TIN, or books.purge_tin).
    ///   3. IF clearing while the vault holds a record: SHOULD refuse
    ///      (books.purge_tin first).
    /// ```
    #[tool(
        name = "books.set_w9",
        title = "Record W-9 facts",
        description = "Record what a contractor's or landlord's W-9 says, or clear it (clear: true alone): the date received (received_on), line 3 entity type, line 4 exempt payee and FATCA codes, the TIN's last four digits, and where the form is kept. Give the full TIN as tin with tin_type (ssn, ein, itin) and it goes only to the encrypted TIN vault (its key kept outside the books, created on first use): the books keep its last four (tin_last4 is then optional, and refused if it disagrees) and a hash of the vault record, in the W-9 facts and the party's TIN facts alike (party.tin_set, as books.set_tin), and kept_at defaults to \"wanderbooks vault\". The CPA package carries the full TIN to the CPA. Give w9_file_path (a local PDF, PNG, or JPEG of the signed form) and the form goes to the vault too, never to the receipts and documents; read it back with books.get_w9. Never put a TIN in any other field, note, description, or file: the engine refuses any nine-digit number, and no tool output echoes one. The party needs a 1099 role (contractor or landlord: books.set_party_roles). Record it before paying them. A party with a vendor record also gets its W-9 marked on file there (books.vendor_1099 missing_w9). Clearing W-9 facts whose TIN is in the vault needs books.purge_tin first. Journaled as party.w9_set. Returns the party (the TIN's last four only).",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn set_w9(&self, params: Parameters<SetW9Params>) -> Result<Json<PartyOut>, String> {
        let p = params.0;
        let vault_input = p.tin.is_some() || p.tin_type.is_some() || p.w9_file_path.is_some();
        let tin = match (&p.tin, &p.tin_type) {
            (Some(t), Some(k)) => Some(
                ledger_vault::Tin::new(t.0.trim(), parse_tin_type(k)?)
                    .map_err(|e| e.to_string())?,
            ),
            (Some(_), None) => return Err("tin needs tin_type: ssn, ein, or itin".into()),
            (None, Some(_)) => return Err("tin_type goes with tin".into()),
            (None, None) => None,
        };
        let facts = if p.clear {
            if p.received_on.is_some()
                || p.entity_type.is_some()
                || p.exempt_payee_code.is_some()
                || p.fatca_code.is_some()
                || p.tin_last4.is_some()
                || p.kept_at.is_some()
                || vault_input
            {
                return Err("clear: true takes no other W-9 fact".into());
            }
            None
        } else {
            let need = |v: &Option<String>, name: &str| {
                v.as_deref()
                    .map(|s| s.trim().to_string())
                    .ok_or_else(|| format!("{name} is required (or pass clear: true)"))
            };
            let tin_last4 = match (&tin, p.tin_last4.as_deref().map(str::trim)) {
                (Some(t), Some(given)) if given != t.last4() => {
                    return Err(
                        "tin_last4 doesn't match the TIN's last four digits (leave it out: it comes from tin)"
                            .into(),
                    );
                }
                (Some(t), _) => t.last4().to_string(),
                (None, _) => need(&p.tin_last4, "tin_last4")?,
            };
            let kept_at = match &p.kept_at {
                Some(k) => k.trim().to_string(),
                None if vault_input => "wanderbooks vault".to_string(),
                None => need(&p.kept_at, "kept_at")?,
            };
            Some(ledger_core::W9Facts {
                received: parse_date(&need(&p.received_on, "received_on")?)?,
                entity_type: need(&p.entity_type, "entity_type")?,
                exempt_payee_code: p.exempt_payee_code.as_deref().and_then(optional_text),
                fatca_code: p.fatca_code.as_deref().and_then(optional_text),
                tin_last4,
                kept_at,
                vault_record: None,
            })
        };
        let w9_file = p
            .w9_file_path
            .as_deref()
            .map(|path| self.read_w9_file(path))
            .transpose()?;
        let mut store = self.lock()?;
        let id = resolve_party(&store, &p.party)?;
        let existing = store.ledger().get_party(id).and_then(|p| p.w9.clone());
        let party_tin = store.ledger().get_party(id).and_then(|p| p.tin.clone());
        // A new TIN with another last four replaces the party's TIN facts:
        // cleared first, so the two never disagree on the record (B-26).
        let replace_tin = tin
            .as_ref()
            .is_some_and(|t| party_tin.as_ref().is_some_and(|pt| pt.last4 != t.last4()));
        let mut tin_facts: Option<ledger_core::TinFacts> = None;
        let vault_dir = Self::vault_dir(&store);
        let (held_tin, held_w9) =
            ledger_vault::records_held(&vault_dir, id).map_err(|e| e.to_string())?;
        let facts = match facts {
            None => {
                if held_tin
                    || held_w9
                    || existing.as_ref().is_some_and(|f| f.vault_record.is_some())
                {
                    return Err(format!(
                        "the TIN vault holds party {id}'s TIN or W-9: purge it first (books.purge_tin, with the reason), then clear"
                    ));
                }
                None
            }
            Some(mut f) => {
                // The facts must agree with a TIN the vault keeps
                // (INVARIANT 4).
                if tin.is_none() && held_tin {
                    let held = self
                        .open_vault(&store, false)?
                        .tin(id)
                        .map_err(|e| e.to_string())?;
                    if let Some(t) = held
                        && t.last4() != f.tin_last4
                    {
                        return Err(format!(
                            "the TIN vault holds a TIN ending in {} for party {id}: pass the new tin (with tin_type), or purge the old one first (books.purge_tin)",
                            t.last4()
                        ));
                    }
                }
                // Rehearse with a stand-in hash before writing the vault
                // (INVARIANT 3).
                let mut rehearsal = store.ledger().clone();
                f.vault_record = Some("0".repeat(64));
                let stand_in = tin.as_ref().map(|t| ledger_core::TinFacts {
                    tin_type: tin_kind(t.kind),
                    last4: t.last4().to_string(),
                    vault_record: Some("0".repeat(64)),
                    purged: None,
                });
                if replace_tin {
                    rehearsal
                        .set_party_tin(id, None)
                        .map_err(|e| e.to_string())?;
                }
                rehearsal
                    .set_party_w9(id, Some(f.clone()))
                    .map_err(|e| e.to_string())?;
                if stand_in.is_some() {
                    rehearsal
                        .set_party_tin(id, stand_in.clone())
                        .map_err(|e| e.to_string())?;
                }
                f.vault_record = if tin.is_some() || w9_file.is_some() {
                    let vault = self.open_vault(&store, true)?;
                    // The W-9 read is recorded before the vault keeps it:
                    // never filed into the books (tin_files.rs INVARIANT
                    // 1). Only now: every check has passed.
                    if let Some(w) = &w9_file {
                        Self::wrote_tins(&store, &w.bytes)?;
                    }
                    let tin_hash = match &tin {
                        Some(t) => Some(vault.put_tin(id, t).map_err(|e| e.to_string())?),
                        None => None,
                    };
                    let w9_hash = match &w9_file {
                        Some(w) => Some(vault.put_w9(id, w).map_err(|e| e.to_string())?),
                        None => None,
                    };
                    match tin_hash {
                        Some(h) => Some(h),
                        None if held_tin => vault.record_hash(id).map_err(|e| e.to_string())?,
                        None => w9_hash,
                    }
                } else if held_tin {
                    ledger_vault::Vault::open(&vault_dir, &self.settings().key_file(), false)
                        .ok()
                        .and_then(|v| v.record_hash(id).ok().flatten())
                        .or_else(|| existing.as_ref().and_then(|e| e.vault_record.clone()))
                } else if held_w9 {
                    existing.as_ref().and_then(|e| e.vault_record.clone())
                } else {
                    None
                };
                // The TIN's facts carry the same record as the W-9's.
                tin_facts = stand_in.map(|t| ledger_core::TinFacts {
                    vault_record: f.vault_record.clone(),
                    ..t
                });
                Some(f)
            }
        };
        let on_file = facts.is_some();
        if replace_tin {
            store.set_party_tin(id, None).map_err(|e| e.to_string())?;
        }
        store.set_party_w9(id, facts).map_err(|e| e.to_string())?;
        if tin_facts.is_some() {
            store
                .set_party_tin(id, tin_facts)
                .map_err(|e| e.to_string())?;
        }
        // Keep the vendor record's W-9 flag (the 1099 worksheet's) in step.
        let vendor = store
            .ledger()
            .get_party(id)
            .and_then(|p| p.vendor)
            .and_then(|v| store.ledger().get_vendor(v))
            .map(|v| (v.id, v.form_1099, v.tin_on_file));
        if let Some((v, form, tin_on_file)) = vendor
            && tin_on_file != on_file
        {
            store
                .set_vendor_1099(v, form, on_file)
                .map_err(|e| e.to_string())?;
        }
        let party = store.ledger().get_party(id).expect("resolved");
        Ok(Json(party_out(party)))
    }

    /// ```text
    /// GOAL: any party's full TIN (a member's SSN for their K-1, a payee's
    ///       from their W-9) kept where the CPA package can carry it, and
    ///       nowhere else.
    /// SOURCE: docs/parties-and-attachments.md "The TIN vault"; B-26;
    ///         DECISION (owner, 2026-10-15): "In the vault, that's the
    ///         WHOLE reason we built it. And it needs to be in the damn CPA
    ///         package."
    /// INVARIANTS:
    ///   1. The full TIN goes only to the vault (set_w9's path: key outside
    ///      the books, created on first use; books in git must ignore
    ///      vault/); `party.tin_set` journals its type, last four, and the
    ///      record's hash. The output never holds more than the last four.
    ///   2. Any role.
    ///   3. The facts are checked (on a copy of the ledger) before the vault
    ///      is written; a refusal journals nothing and writes no record.
    ///   4. W-9 facts on file must have the same last four (else refused:
    ///      a payee's new TIN comes with its W-9, books.set_w9); when they
    ///      do, their vault record follows the new record (`party.w9_set`).
    /// ```
    #[tool(
        name = "books.set_tin",
        title = "Store a TIN in the vault",
        description = "Put a party's full taxpayer identification number in the encrypted TIN vault: tin (nine digits, hyphens allowed) and tin_type (ssn, ein, itin). Any role: collect every member's (their K-1 needs it) and every 1099 payee's (books.set_w9 tin records a W-9's TIN the same way). The books keep only its type, last four, and a hash of the vault record (journaled as party.tin_set); books.export_cpa_package carries the full number to the CPA. A party whose W-9 facts name another last four is refused: a payee's new TIN comes with its new W-9 (books.set_w9). Never repeat a TIN in chat, descriptions, notes, version-history messages, or file names. Returns the party (the last four only).",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn set_tin(&self, params: Parameters<SetTinParams>) -> Result<Json<PartyOut>, String> {
        let p = params.0;
        let tin = ledger_vault::Tin::new(p.tin.0.trim(), parse_tin_type(&p.tin_type)?)
            .map_err(|e| e.to_string())?;
        let mut store = self.lock()?;
        let id = resolve_party(&store, &p.party)?;
        let w9 = store.ledger().get_party(id).and_then(|p| p.w9.clone());
        let stand_in = ledger_core::TinFacts {
            tin_type: tin_kind(tin.kind),
            last4: tin.last4().to_string(),
            vault_record: Some("0".repeat(64)),
            purged: None,
        };
        // Rehearse on a copy before the vault is written (INVARIANT 3).
        let mut rehearsal = store.ledger().clone();
        rehearsal
            .set_party_tin(id, Some(stand_in.clone()))
            .map_err(|e| e.to_string())?;
        if let Some(f) = &w9 {
            rehearsal
                .set_party_w9(
                    id,
                    Some(ledger_core::W9Facts {
                        vault_record: Some("0".repeat(64)),
                        ..f.clone()
                    }),
                )
                .map_err(|e| e.to_string())?;
        }
        let vault = self.open_vault(&store, true)?;
        let hash = vault.put_tin(id, &tin).map_err(|e| e.to_string())?;
        store
            .set_party_tin(
                id,
                Some(ledger_core::TinFacts {
                    vault_record: Some(hash.clone()),
                    ..stand_in
                }),
            )
            .map_err(|e| e.to_string())?;
        if let Some(f) = w9
            && f.vault_record.as_deref() != Some(hash.as_str())
        {
            store
                .set_party_w9(
                    id,
                    Some(ledger_core::W9Facts {
                        vault_record: Some(hash),
                        ..f
                    }),
                )
                .map_err(|e| e.to_string())?;
        }
        let party = store.ledger().get_party(id).expect("resolved");
        Ok(Json(party_out(party)))
    }

    /// ```text
    /// GOAL: hand the human (for the CPA) the signed W-9 the vault keeps,
    ///       without it passing through the books or the conversation.
    /// SOURCE: docs/parties-and-attachments.md "The TIN vault".
    /// INVARIANTS:
    ///   1. The file is written only to the path given, outside the books
    ///      dir, as a new file readable by its owner only; the output is
    ///      the path, never the contents.
    ///   2. Nothing is journaled.
    /// SCENARIOS:
    ///   1. IF the path is inside the books dir, exists, or its folder
    ///      doesn't: SHOULD refuse.
    ///   2. IF the vault is locked or holds no W-9 file for the party:
    ///      SHOULD refuse, saying which.
    /// ```
    #[tool(
        name = "books.get_w9",
        title = "Write out a W-9",
        description = "Write a party's signed W-9 file from the TIN vault to a local path (save_to: a new file outside the books dir, since the form shows the full TIN) and return the path, never the contents. Give the human the path; never open, read, or attach the file yourself. Nothing is journaled.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn get_w9(&self, params: Parameters<GetW9Params>) -> Result<Json<GetW9Out>, String> {
        let p = params.0;
        let store = self.lock()?;
        let id = resolve_party(&store, &p.party)?;
        let dest = self.tin_file_dest(&store, &p.save_to)?;
        let file = self
            .open_vault(&store, false)?
            .w9(id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| {
                format!(
                    "the TIN vault holds no W-9 file for party {id}: store one with books.set_w9 w9_file_path"
                )
            })?;
        Self::wrote_tins(&store, &file.bytes)?;
        write_private_new(&dest, &file.bytes)?;
        Ok(Json(GetW9Out {
            party_id: id,
            path: dest.display().to_string(),
            file_name: file.file_name,
            size_bytes: file.bytes.len() as u64,
            note: format!(
                "Saved {} (readable by you only). It shows the full TIN: give it to the CPA, keep it out of the books, email, and chat, and delete it when done.",
                dest.display()
            ),
        }))
    }

    /// ```text
    /// GOAL: a TIN kept no longer than needed can be destroyed, on the
    ///       record.
    /// SOURCE: docs/parties-and-attachments.md "The TIN vault" ("Purge");
    ///         IRS Pub. 583 (keep records while needed; then dispose).
    /// INVARIANTS:
    ///   1. Without confirmed=true: a preview, changing nothing.
    ///   2. Confirmed: the party's vault records (TIN and W-9 file) are
    ///      deleted, needing no key; when the party has TIN facts,
    ///      `party.tin_set` journals them with the vault record cleared
    ///      and `purged` saying when and why, and when it has W-9 facts,
    ///      `party.w9_set` likewise in kept_at (the last four stay).
    ///   3. The new facts are checked before anything is deleted; a refusal
    ///      changes nothing.
    /// SCENARIOS:
    ///   1. IF the reason is blank: SHOULD refuse.
    ///   2. IF the vault holds nothing for the party and its facts name no
    ///      vault record: SHOULD refuse (nothing to purge).
    /// ```
    #[tool(
        name = "books.purge_tin",
        title = "Purge a TIN from the vault",
        description = "Delete a party's full TIN and W-9 file from the TIN vault (after the retention period, at the payee's request, or to replace a wrong one), with the reason. Without confirmed=true it previews what the vault holds and changes nothing. Confirmed, the records are deleted for good and the books journal it: the party's TIN facts (party.tin_set: purged says when and why) and W-9 facts (party.w9_set: kept_at says so) keep the last four and drop the vault record. Works without the vault key. The next CPA package then lacks that TIN (and says so).",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn purge_tin(
        &self,
        params: Parameters<PurgeTinParams>,
    ) -> Result<Json<PurgeTinOut>, String> {
        let p = params.0;
        let reason = p.reason.trim();
        if reason.is_empty() {
            return Err("give the reason for purging".into());
        }
        let mut store = self.lock()?;
        let id = resolve_party(&store, &p.party)?;
        let party = store.ledger().get_party(id).expect("resolved").clone();
        let vault_dir = Self::vault_dir(&store);
        let (held_tin, held_w9) =
            ledger_vault::records_held(&vault_dir, id).map_err(|e| e.to_string())?;
        let recorded = party.w9.as_ref().is_some_and(|f| f.vault_record.is_some())
            || party.tin.as_ref().is_some_and(|t| t.vault_record.is_some());
        if !held_tin && !held_w9 && !recorded {
            return Err(format!(
                "the TIN vault holds nothing for party {id} ({}) and its W-9 and TIN facts name no vault record",
                party.name
            ));
        }
        let why = format!("purged from the wanderbooks vault on {}: {reason}", today());
        let new_facts = party.w9.clone().map(|f| ledger_core::W9Facts {
            vault_record: None,
            kept_at: format!("not kept: {why}"),
            ..f
        });
        let new_tin = party.tin.clone().map(|t| ledger_core::TinFacts {
            vault_record: None,
            purged: Some(why.clone()),
            ..t
        });
        // Both checked before anything is deleted (INVARIANT 3).
        let mut rehearsal = store.ledger().clone();
        if let Some(f) = &new_facts {
            rehearsal
                .set_party_w9(id, Some(f.clone()))
                .map_err(|e| e.to_string())?;
        }
        if let Some(t) = &new_tin {
            rehearsal
                .set_party_tin(id, Some(t.clone()))
                .map_err(|e| e.to_string())?;
        }
        let mut out = PurgeTinOut {
            status: "preview".into(),
            party_id: id,
            name: party.name.clone(),
            tin_in_vault: held_tin,
            w9_file_in_vault: held_w9,
            tin_last4: party
                .w9
                .as_ref()
                .map(|f| f.tin_last4.clone())
                .or_else(|| party.tin.as_ref().map(|t| t.last4.clone())),
            journaled: false,
            note: format!(
                "Confirming deletes {} for good{}. Make sure the CPA has what they need first.",
                match (held_tin, held_w9) {
                    (true, true) => "the TIN and the W-9 file",
                    (true, false) => "the TIN",
                    (false, true) => "the W-9 file",
                    (false, false) => "nothing in the vault (the books' record of it is cleared)",
                },
                match (new_facts.is_some(), new_tin.is_some()) {
                    (true, true) =>
                        " and journals the purge in the TIN and W-9 facts (party.tin_set, party.w9_set; the last four stay)",
                    (false, true) =>
                        " and journals the purge in the TIN facts (party.tin_set; the last four stay)",
                    (true, false) =>
                        " and journals the purge in the W-9 facts (party.w9_set; the last four stay)",
                    (false, false) => "",
                }
            ),
        };
        if !p.confirmed.unwrap_or(false) {
            return Ok(Json(out));
        }
        ledger_vault::purge_records(&vault_dir, id).map_err(|e| e.to_string())?;
        if let Some(f) = new_facts {
            store.set_party_w9(id, Some(f)).map_err(|e| e.to_string())?;
            out.journaled = true;
        }
        if let Some(t) = new_tin {
            store
                .set_party_tin(id, Some(t))
                .map_err(|e| e.to_string())?;
            out.journaled = true;
        }
        out.status = "purged".into();
        out.note = format!(
            "Purged from the vault{}.",
            if out.journaled {
                "; the books record when and why"
            } else {
                ""
            }
        );
        Ok(Json(out))
    }

    #[tool(
        name = "books.set_w4",
        title = "Record W-4 facts",
        description = "Record what an employee's W-4 says, or clear it (clear: true alone): the date received (received_on), the work state (two letters), and where the form is kept. Never the SSN. The party needs the employee role. Record it before their first payroll. Journaled as party.w4_set. Returns the party.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn set_w4(&self, params: Parameters<SetW4Params>) -> Result<Json<PartyOut>, String> {
        let p = params.0;
        let facts = if p.clear {
            if p.received_on.is_some() || p.state.is_some() || p.kept_at.is_some() {
                return Err("clear: true takes no other W-4 fact".into());
            }
            None
        } else {
            let need = |v: &Option<String>, name: &str| {
                v.as_deref()
                    .map(|s| s.trim().to_string())
                    .ok_or_else(|| format!("{name} is required (or pass clear: true)"))
            };
            Some(ledger_core::W4Facts {
                received: parse_date(&need(&p.received_on, "received_on")?)?,
                state: need(&p.state, "state")?,
                kept_at: need(&p.kept_at, "kept_at")?,
            })
        };
        let mut store = self.lock()?;
        let id = resolve_party(&store, &p.party)?;
        store.set_party_w4(id, facts).map_err(|e| e.to_string())?;
        let party = store.ledger().get_party(id).expect("resolved");
        Ok(Json(party_out(party)))
    }

    #[tool(
        name = "books.parties",
        title = "List parties",
        description = "The party directory, by id: each party's name, label, roles (vendor and customer included), vendor and customer record ids, whether W-9 facts (with the TIN's last four) and W-4 facts are on file, archived, and what was paid to and received from it in the year (default this year; defined as in books.party). Filter by role; archived parties only with include_archived.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn parties(
        &self,
        params: Parameters<ListPartiesParams>,
    ) -> Result<Json<ListPartiesOut>, String> {
        let p = params.0;
        let role = p
            .role
            .as_deref()
            .map(|r| parse_roles(&[r.to_string()]).map(|v| v[0]))
            .transpose()?;
        let year = p.year.unwrap_or_else(this_year);
        let settings = self.settings();
        let store = self.lock()?;
        let mut parties = Vec::new();
        for party in store.ledger().parties() {
            if party.archived && !p.include_archived {
                continue;
            }
            if role.is_some_and(|r| !party.all_roles().contains(&r)) {
                continue;
            }
            let act = party_activity(&store, party, &settings, year)?;
            parties.push(PartySummaryOut {
                party_id: party.id,
                name: party.name.clone(),
                label: party.label().to_string(),
                roles: party_roles(party),
                vendor_id: party.vendor,
                customer_id: party.customer,
                w9_on_file: party.w9.is_some(),
                tin_last4: party.w9.as_ref().map(|w| w.tin_last4.clone()),
                w4_on_file: party.w4.is_some(),
                archived: party.archived,
                paid: money(act.paid),
                received: money(act.received),
            });
        }
        Ok(Json(ListPartiesOut { year, parties }))
    }

    #[tool(
        name = "books.party",
        title = "Get a party",
        description = "One party, by id or name: its facts and roles, vendor and customer record ids, the year's totals (default this year: paid to it, received from it, invoiced, billed, the 1099 amount for a vendor, payroll gross and net), what it is owed and owes now (open bills and invoices), every entry it is on (tagged with it or its vendor record, its invoices and their payments, and their reversals), its invoices and bills, and the payroll lines naming it. Totals come from the journal: the output's schema says exactly how.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn party(&self, params: Parameters<PartyParams>) -> Result<Json<PartyDetailOut>, String> {
        let p = params.0;
        let year = p.year.unwrap_or_else(this_year);
        let settings = self.settings();
        let store = self.lock()?;
        let ledger = store.ledger();
        let id = resolve_party(&store, &p.party)?;
        let party = ledger.get_party(id).expect("resolved");
        let act = party_activity(&store, party, &settings, year)?;
        let in_year = |d: NaiveDate| chrono::Datelike::year(&d) == year;
        let mut entries: Vec<PartyEntryOut> = act
            .entries
            .iter()
            .filter_map(|&e| ledger.get_entry(e))
            .map(|e| PartyEntryOut {
                entry_id: e.id(),
                date: e.date().to_string(),
                description: e.description().to_string(),
                status: match e.status() {
                    ledger_core::EntryStatus::Draft => "draft",
                    ledger_core::EntryStatus::Posted => "posted",
                }
                .to_string(),
                total: ReceiptPolicy::entry_debits(e).ok().map(money),
                reversal_of: e.reversal_of(),
            })
            .collect();
        entries.sort_by(|a, b| b.date.cmp(&a.date).then(b.entry_id.cmp(&a.entry_id)));
        let invoices = party_invoices(ledger, party);
        let bills = party_bills(ledger, party);
        let amount_1099 = match party.vendor {
            Some(v) => {
                let r = vendor_1099_report(ledger, &self.settings(), year, Some(Decimal::ZERO))
                    .map_err(|e| e.to_string())?;
                Some(money(
                    r.vendors
                        .iter()
                        .find(|l| l.vendor_id == v)
                        .map(|l| l.paid)
                        .unwrap_or_default(),
                ))
            }
            None => None,
        };
        // Documents and runs are restored as journaled (rules.md B-28): a
        // total past exact arithmetic refuses the answer, never a panic.
        let mut payroll = Vec::new();
        let (mut gross, mut net) = (Decimal::ZERO, Decimal::ZERO);
        for run in ledger.list_payroll_runs() {
            for line in run.lines.iter().filter(|l| l.employee == Some(id)) {
                let line_net = line.net_pay().map_err(|e| e.to_string())?;
                if in_year(run.date) {
                    gross = checked_sum([Ok(gross), Ok(line.gross_pay)], "gross pay")?;
                    net = checked_sum([Ok(net), Ok(line_net)], "net pay")?;
                }
                payroll.push(PartyPayrollOut {
                    run_id: run.id,
                    date: run.date.to_string(),
                    entry_id: run.entry_id,
                    gross_pay: money(line.gross_pay),
                    net_pay: money(line_net),
                });
            }
        }
        let totals = PartyTotalsOut {
            paid: money(act.paid),
            received: money(act.received),
            invoiced: money(checked_sum(
                invoices
                    .iter()
                    .filter(|i| in_year(i.date))
                    .map(|i| i.total()),
                "the year's invoices",
            )?),
            billed: money(checked_sum(
                bills.iter().filter(|b| in_year(b.date)).map(|b| b.total()),
                "the year's bills",
            )?),
            owed: money(checked_sum(
                bills.iter().map(|b| b.balance()),
                "what is owed",
            )?),
            receivable: money(checked_sum(
                invoices.iter().map(|i| i.balance()),
                "what is receivable",
            )?),
            amount_1099,
            form_1099: party
                .vendor
                .and_then(|v| ledger.get_vendor(v))
                .and_then(|v| form_1099_label(v.form_1099)),
            gross_pay: money(gross),
            net_pay: money(net),
        };
        Ok(Json(PartyDetailOut {
            party: party_out(party),
            year,
            totals,
            entries,
            invoices: invoices
                .iter()
                .map(|i| {
                    party_document(
                        i.id,
                        (i.date, i.due_date),
                        (i.total(), i.balance(), i.amount_paid),
                        i.status,
                        i.entry_id,
                    )
                })
                .collect::<Result<_, _>>()?,
            bills: bills
                .iter()
                .map(|b| {
                    party_document(
                        b.id,
                        (b.date, b.due_date),
                        (b.total(), b.balance(), b.amount_paid),
                        b.status,
                        b.entry_id,
                    )
                })
                .collect::<Result<_, _>>()?,
            payroll,
        }))
    }

    #[tool(
        name = "books.set_entry_party",
        title = "Tag an entry's parties",
        description = "Tag an entry (draft or posted) with the party it was with (id or name), or pass party null (or omit it) to clear the tag. A party with a vendor record also tags the entry's vendor, so a payment to a contractor or landlord counts in books.vendor_1099; a party without one, or clearing, clears the vendor tag. For an entry with several counterparties pass parties instead: [{party, amount?|percent?}], each share an exact amount (cents), a percent (summing to 100; leftover cents to the first party), or neither (derived: a sole party gets all; members named together get what their capital or distributions lines say, else their [[company.members]] share; once the entry is posted, a derived split among several parties is recorded as the exact amounts it resolves to, so a later change to [[company.members]] never re-splits posted history). Name both members on a joint member transaction (a reimbursement on 2100, a joint contribution) and let the shares derive; give amounts when a split is uneven. Refused: a party twice, mixed kinds of share, two parties with vendor records, and, once the entry has balanced lines, shares that don't add up to what it moved. Metadata only: amounts never change. An entry and its reversal carry the same parties: tagging either tags both, so each party's part nets to zero. Journaled as entry.party_set (one party) or entry.parties_set (parties). Returns the entry (as books.get_entry), each party's share resolved.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn set_entry_party(
        &self,
        params: Parameters<SetEntryPartyParams>,
    ) -> Result<Json<GetEntryOut>, String> {
        let p = params.0;
        {
            let settings = self.settings();
            let mut store = self.lock()?;
            let party = p
                .party
                .as_deref()
                .map(|r| resolve_party(&store, r))
                .transpose()?;
            let shares = p
                .parties
                .as_deref()
                .map(|ps| parse_shares(&store, ps))
                .transpose()?;
            let reason = p
                .no_party_reason
                .as_deref()
                .map(str::trim)
                .filter(|r| !r.is_empty());
            match (party, reason, shares) {
                (Some(_), _, Some(_)) => {
                    return Err(
                        "pass party (one) or parties (several, with shares), not both".into(),
                    );
                }
                (_, Some(_), Some(_)) => {
                    return Err("pass parties or a no_party_reason, not both".into());
                }
                (Some(_), Some(_), None) => {
                    return Err("pass a party or a no_party_reason, not both".into());
                }
                (None, Some(r), _) => {
                    if !store.ledger().entry_parties(p.entry_id).is_empty() {
                        // Check the mark before clearing, so a refusal
                        // journals nothing.
                        if r.trim().is_empty() {
                            return Err("say why the entry has no counterparty".into());
                        }
                        ledger_core::parties::check_text("reason", r)?;
                        store
                            .set_entry_party(p.entry_id, None)
                            .map_err(|e| e.to_string())?;
                    }
                    store
                        .set_entry_no_party(p.entry_id, r)
                        .map_err(|e| e.to_string())?;
                }
                (None, None, Some(sh)) => {
                    store
                        .ledger()
                        .check_set_entry_parties(p.entry_id, &sh)
                        .map_err(|e| e.to_string())?;
                    check_shares_resolve(&store, &settings, p.entry_id, &sh)?;
                    store
                        .set_entry_parties(p.entry_id, &sh)
                        .map_err(|e| e.to_string())?;
                    fix_derived_shares(&mut store, &settings, p.entry_id)?;
                }
                (party, None, None) => {
                    store
                        .set_entry_party(p.entry_id, party)
                        .map_err(|e| e.to_string())?;
                }
            }
        }
        self.get_entry(Parameters(GetEntryParams {
            entry_id: p.entry_id,
        }))
        .await
    }

    #[tool(
        name = "books.create_invoice",
        title = "Create and post an invoice",
        description = "Create a sales invoice. Auto-posts Dr Accounts Receivable / Cr Revenue. Lines are {description, amount} with decimal-string amounts.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn create_invoice(
        &self,
        params: Parameters<CreateDocumentParams>,
    ) -> Result<Json<CreateDocumentOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let lines: Vec<DocumentLine> = p
            .lines
            .into_iter()
            .map(|l| {
                Ok(DocumentLine {
                    description: l.description,
                    amount: parse_amount(&l.amount)?,
                })
            })
            .collect::<Result<_, String>>()?;
        let id = store
            .create_invoice(
                p.party_id,
                parse_date(&p.date)?,
                parse_date(&p.due_date)?,
                lines,
            )
            .map_err(|e| e.to_string())?;
        let inv = store
            .ledger()
            .get_invoice(id)
            .ok_or_else(|| format!("invoice {id} not found after creation"))?;
        Ok(Json(CreateDocumentOut {
            id,
            total: inv.total().map_err(|e| e.to_string())?.to_string(),
            entry_id: inv.entry_id,
        }))
    }

    #[tool(
        name = "books.create_bill",
        title = "Create and post a bill",
        description = "Create a vendor bill. Auto-posts Dr General Expenses / Cr Accounts Payable. Lines are {description, amount} with decimal-string amounts.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn create_bill(
        &self,
        params: Parameters<CreateDocumentParams>,
    ) -> Result<Json<CreateDocumentOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let lines: Vec<DocumentLine> = p
            .lines
            .into_iter()
            .map(|l| {
                Ok(DocumentLine {
                    description: l.description,
                    amount: parse_amount(&l.amount)?,
                })
            })
            .collect::<Result<_, String>>()?;
        let id = store
            .create_bill(
                p.party_id,
                parse_date(&p.date)?,
                parse_date(&p.due_date)?,
                lines,
            )
            .map_err(|e| e.to_string())?;
        let bill = store
            .ledger()
            .get_bill(id)
            .ok_or_else(|| format!("bill {id} not found after creation"))?;
        Ok(Json(CreateDocumentOut {
            id,
            total: bill.total().map_err(|e| e.to_string())?.to_string(),
            entry_id: bill.entry_id,
        }))
    }

    #[tool(
        name = "books.receive_payment",
        title = "Receive an invoice payment",
        description = "Record a customer payment against an invoice. Posts Dr Cash / Cr A/R.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn receive_payment(
        &self,
        params: Parameters<PaymentParams>,
    ) -> Result<Json<PaymentOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let cash = resolve_account(&store, &p.cash_account)?;
        let entry_id = store
            .receive_payment(
                p.document_id,
                parse_date(&p.date)?,
                parse_amount(&p.amount)?,
                cash,
            )
            .map_err(|e| e.to_string())?;
        let inv = store
            .ledger()
            .get_invoice(p.document_id)
            .ok_or_else(|| format!("invoice {} not found", p.document_id))?;
        Ok(Json(PaymentOut {
            entry_id,
            remaining: inv.balance().map_err(|e| e.to_string())?.to_string(),
            status: match inv.status {
                DocumentStatus::Open => "open".to_string(),
                DocumentStatus::Partial => "partial".to_string(),
                DocumentStatus::Paid => "paid".to_string(),
            },
        }))
    }

    #[tool(
        name = "books.pay_bill",
        title = "Pay a bill",
        description = "Pay a vendor bill. Posts Dr A/P / Cr Cash.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn pay_bill(
        &self,
        params: Parameters<PaymentParams>,
    ) -> Result<Json<PaymentOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let cash = resolve_account(&store, &p.cash_account)?;
        let entry_id = store
            .pay_bill(
                p.document_id,
                parse_date(&p.date)?,
                parse_amount(&p.amount)?,
                cash,
            )
            .map_err(|e| e.to_string())?;
        let bill = store
            .ledger()
            .get_bill(p.document_id)
            .ok_or_else(|| format!("bill {} not found", p.document_id))?;
        Ok(Json(PaymentOut {
            entry_id,
            remaining: bill.balance().map_err(|e| e.to_string())?.to_string(),
            status: match bill.status {
                DocumentStatus::Open => "open".to_string(),
                DocumentStatus::Partial => "partial".to_string(),
                DocumentStatus::Paid => "paid".to_string(),
            },
        }))
    }

    #[tool(
        name = "books.ar_aging",
        title = "Receivables aging",
        description = "A/R aging report as of a date: outstanding balances per customer, bucketed by days past due (current, 30, 60, 90, 90+).",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn ar_aging(&self, params: Parameters<AgingParams>) -> Result<Json<AgingOut>, String> {
        let p = params.0;
        let store = self.lock()?;
        let rows = store
            .ledger()
            .ar_aging(parse_date(&p.as_of)?)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|r| {
                let total = r.total().map_err(|e| e.to_string())?.to_string();
                Ok(AgingRowOut {
                    name: r.name,
                    current: money(r.current),
                    days_30: money(r.days_30),
                    days_60: money(r.days_60),
                    days_90: money(r.days_90),
                    days_90_plus: money(r.days_90_plus),
                    total,
                })
            })
            .collect::<Result<_, String>>()?;
        Ok(Json(AgingOut { rows }))
    }

    #[tool(
        name = "books.ap_aging",
        title = "Payables aging",
        description = "A/P aging report as of a date: outstanding balances per vendor, bucketed by days past due (current, 30, 60, 90, 90+).",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn ap_aging(&self, params: Parameters<AgingParams>) -> Result<Json<AgingOut>, String> {
        let p = params.0;
        let store = self.lock()?;
        let rows = store
            .ledger()
            .ap_aging(parse_date(&p.as_of)?)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|r| {
                let total = r.total().map_err(|e| e.to_string())?.to_string();
                Ok(AgingRowOut {
                    name: r.name,
                    current: money(r.current),
                    days_30: money(r.days_30),
                    days_60: money(r.days_60),
                    days_90: money(r.days_90),
                    days_90_plus: money(r.days_90_plus),
                    total,
                })
            })
            .collect::<Result<_, String>>()?;
        Ok(Json(AgingOut { rows }))
    }

    // ------------------------------------------------------------------
    // Foreign exchange
    // ------------------------------------------------------------------

    #[tool(
        name = "books.set_fx_rate",
        title = "Set an exchange rate",
        description = "Record a historical FX rate: how much base currency one unit of the foreign currency buys on the given date. For reference; lines store their own rate at entry time.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn set_fx_rate(
        &self,
        params: Parameters<SetFxRateParams>,
    ) -> Result<Json<SetFxRateOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let currency = Currency::new(&p.currency).map_err(|e| e.to_string())?;
        store
            .set_fx_rate(currency, parse_date(&p.date)?, parse_amount(&p.rate)?)
            .map_err(|e| e.to_string())?;
        Ok(Json(SetFxRateOut { ok: true }))
    }

    // ------------------------------------------------------------------
    // Payroll (accounting only — the caller supplies every number; no tax tables)
    // ------------------------------------------------------------------

    #[tool(
        name = "books.run_payroll",
        title = "Run payroll",
        description = "Run payroll for a pay date. Each line carries one employee's gross pay and every withholding/deduction as decimal strings — the engine cuts one balanced entry (Dr Salary & Wage Expense, Dr Payroll Tax Expense for the employer share, Cr the withholding payables, Cr cash for net pay). Auto-creates the 61xx/21xx accounts. All amounts are decimal strings, never floats. Name each line's employee party (employee: id or name, a party with the employee role; add one with books.add_party and record its W-4 with books.set_w4 first): new runs should, so the run shows on the employee's books.party.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn run_payroll(
        &self,
        params: Parameters<RunPayrollParams>,
    ) -> Result<Json<RunPayrollOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let cash = resolve_account(&store, &p.cash_account)?;
        let lines: Vec<ledger_store::PayrollLineInput> = p
            .lines
            .into_iter()
            .map(|l| {
                let employee = l
                    .employee
                    .as_deref()
                    .map(|r| resolve_party(&store, r))
                    .transpose()?;
                let employee_name = match (l.employee_name, employee) {
                    (Some(n), _) => n,
                    (None, Some(id)) => store
                        .ledger()
                        .get_party(id)
                        .map(|p| p.name.clone())
                        .unwrap_or_default(),
                    (None, None) => {
                        return Err(
                            "each payroll line names its employee: employee (a party) or employee_name"
                                .to_string(),
                        );
                    }
                };
                Ok(ledger_core::PayrollLine {
                    employee_name,
                    employee,
                    gross_pay: parse_amount(&l.gross_pay)?,
                    federal_withholding: parse_amount(&l.federal_withholding)?,
                    state_withholding: parse_amount(&l.state_withholding)?,
                    ss_employee: parse_amount(&l.ss_employee)?,
                    medicare_employee: parse_amount(&l.medicare_employee)?,
                    ss_employer: parse_amount(&l.ss_employer)?,
                    medicare_employer: parse_amount(&l.medicare_employer)?,
                    other_deductions: parse_amount(&l.other_deductions)?,
                })
            })
            .collect::<Result<_, String>>()?;
        if self.settings().parties == ledger_config::PartyPolicy::Require
            && lines
                .iter()
                .any(|l| l.employee.is_none() && l.net_pay().is_ok_and(|n| !n.is_zero()))
        {
            return Err(party_required(
                "name each payroll line's employee party (employee), so the entry names who was paid",
            ));
        }
        let run_id = store
            .run_payroll(parse_date(&p.date)?, lines, cash)
            .map_err(|e| e.to_string())?;
        let run = store
            .ledger()
            .get_payroll_run(run_id)
            .ok_or_else(|| format!("payroll run {run_id} not found after creation"))?;
        let total_gross = checked_sum(run.lines.iter().map(|l| Ok(l.gross_pay)), "gross pay")?;
        let total_net = checked_sum(run.lines.iter().map(|l| l.net_pay()), "net pay")?;
        Ok(Json(RunPayrollOut {
            run_id,
            entry_id: run.entry_id,
            total_gross: money(total_gross),
            total_net: money(total_net),
        }))
    }

    #[tool(
        name = "books.remit_payroll_taxes",
        title = "Remit payroll taxes",
        description = "Remit all outstanding payroll tax payables (federal/state withholding, SS, Medicare, other deductions) to cash in one balanced entry. Name the tax agency paid (agency: a party id or name, e.g. the IRS): it is the entry's party, and [books] parties = \"require\" refuses without it.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn remit_payroll_taxes(
        &self,
        params: Parameters<RemitPayrollTaxesParams>,
    ) -> Result<Json<RemitPayrollTaxesOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let cash = resolve_account(&store, &p.cash_account)?;
        let agency = p
            .agency
            .as_deref()
            .map(|r| resolve_party(&store, r))
            .transpose()?;
        if agency.is_none() && self.settings().parties == ledger_config::PartyPolicy::Require {
            return Err(party_required("pass agency: the tax agency paid"));
        }
        // Total before remitting, for the response.
        let total: Decimal = store
            .ledger()
            .payroll_tax_payables()
            .iter()
            .map(|(_, b)| *b)
            .sum();
        let entry_id = store
            .remit_payroll_taxes(parse_date(&p.date)?, cash)
            .map_err(|e| e.to_string())?;
        if let Some(a) = agency {
            store
                .set_entry_party(entry_id, Some(a))
                .map_err(|e| e.to_string())?;
        }
        Ok(Json(RemitPayrollTaxesOut {
            entry_id,
            total: money(total),
        }))
    }

    #[tool(
        name = "books.list_payroll_runs",
        title = "List payroll runs",
        description = "List payroll runs in id order: date, employee count, totals, and the posted entry id.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn list_payroll_runs(&self) -> Result<Json<ListPayrollRunsOut>, String> {
        let store = self.lock()?;
        let runs = store
            .list_payroll_runs()
            .into_iter()
            .map(|r| {
                // A run is as journaled (rules.md B-28): a total past exact
                // arithmetic refuses the list, never a panic.
                let total_gross =
                    checked_sum(r.lines.iter().map(|l| Ok(l.gross_pay)), "gross pay")?;
                Ok(PayrollRunOut {
                    id: r.id,
                    date: r.date.to_string(),
                    employee_count: r.lines.len(),
                    total_gross: money(total_gross),
                    entry_id: r.entry_id,
                })
            })
            .collect::<Result<_, String>>()?;
        Ok(Json(ListPayrollRunsOut { runs }))
    }

    // ------------------------------------------------------------------
    // Classes (divisional tracking)
    // ------------------------------------------------------------------

    #[tool(
        name = "books.add_class",
        title = "Add a class",
        description = "Add a tracking class for divisional P&L (e.g. an app, website, or channel). Tag journal lines with it via books.add_line's class param, then report per division.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn add_class(
        &self,
        params: Parameters<AddClassParams>,
    ) -> Result<Json<AddClassOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let id = store.add_class(&p.name).map_err(|e| e.to_string())?;
        Ok(Json(AddClassOut { id }))
    }

    #[tool(
        name = "books.list_classes",
        title = "List classes",
        description = "List tracking classes in id order.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn list_classes(&self) -> Result<Json<ListClassesOut>, String> {
        let store = self.lock()?;
        let classes = store
            .list_classes()
            .into_iter()
            .map(|c| ClassOut {
                id: c.id,
                name: c.name.clone(),
            })
            .collect();
        Ok(Json(ListClassesOut { classes }))
    }

    #[tool(
        name = "books.profit_and_loss_by_class",
        title = "Profit and loss by class",
        description = "P&L broken down by tracking class for a period, plus an unclassified row for untagged lines. Dates are YYYY-MM-DD; omit for all time.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn profit_and_loss_by_class(
        &self,
        params: Parameters<ProfitAndLossParams>,
    ) -> Result<Json<ProfitAndLossByClassOut>, String> {
        let p = params.0;
        let store = self.lock()?;
        let ledger = store.ledger();
        let start = match p.start {
            Some(s) => parse_date(&s)?,
            None => NaiveDate::from_ymd_opt(1, 1, 1).unwrap(),
        };
        let end = match p.end {
            Some(s) => parse_date(&s)?,
            None => today(),
        };
        if end < start {
            return Err("end is before start".to_string());
        }
        // One class, if asked (validated like every reference).
        let only = match &p.class {
            Some(c) => Some(resolve_class(&store, c)?),
            None => None,
        };
        // Per-class P&L over the date range: accumulate line by line.
        use std::collections::HashMap;
        #[derive(Default)]
        struct Acc {
            revenue: Decimal,
            expenses: Decimal,
        }
        let mut by_class: HashMap<Option<u32>, Acc> = HashMap::new();
        // A closing entry isn't income or spending (rules.md B-11): left
        // out, as profit_and_loss_for_period leaves it out (found by
        // pnl_by_class_excludes_closing_entries, 2026-10-14).
        let closings = ledger.closing_entry_ids();
        for entry in ledger.posted_entries() {
            if entry.date() < start || entry.date() > end || closings.contains(&entry.id()) {
                continue;
            }
            for line in entry.lines() {
                let account = match ledger.chart().get(line.account) {
                    Some(a) => a,
                    _ => continue,
                };
                let signed = match line.side {
                    Side::Debit => line.amount.amount,
                    Side::Credit => -line.amount.amount,
                };
                // Normal-side positive.
                let normal = match account.account_type.normal_side() {
                    Side::Debit => signed,
                    Side::Credit => -signed,
                };
                let acc = by_class.entry(line.class_id).or_default();
                match account.account_type {
                    AccountType::Revenue => acc.revenue += normal,
                    AccountType::Expense => acc.expenses += normal,
                    _ => {}
                }
            }
        }
        // Sort: named classes by name, unclassified last.
        let mut rows: Vec<(Option<String>, Acc)> = by_class
            .into_iter()
            .filter(|(cid, _)| only.is_none() || *cid == only)
            .map(|(cid, acc)| {
                let name = cid
                    .and_then(|id| ledger.get_class(id))
                    .map(|c| c.name.clone());
                (name, acc)
            })
            // Drop rows with no P&L movement.
            .filter(|(_, acc)| !acc.revenue.is_zero() || !acc.expenses.is_zero())
            .collect();
        rows.sort_by(|a, b| match (&a.0, &b.0) {
            (Some(x), Some(y)) => x.cmp(y),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        });
        let classes = rows
            .into_iter()
            .map(|(class, acc)| ClassPnlOut {
                class,
                revenue: money(acc.revenue),
                expenses: money(acc.expenses),
                net_income: money(acc.revenue - acc.expenses),
            })
            .collect();
        Ok(Json(ProfitAndLossByClassOut {
            start: start.to_string(),
            end: end.to_string(),
            classes,
        }))
    }

    // ------------------------------------------------------------------
    // CPA handoff package
    // ------------------------------------------------------------------

    /// ```text
    /// GOAL: everything the CPA needs for the return in one zip, the K-1s'
    ///       and 1099s' TINs included, without a full TIN ever landing in
    ///       the books (a git repository) or the conversation.
    /// SOURCE: docs/mcp-interface.md; B-26; DECISION (owner,
    ///         2026-10-15): "it needs to be in the damn CPA package."
    /// INVARIANTS:
    ///   1. With include_tins (the default) the zip is only ever written to
    ///      save_to, a new file outside the books dir (0600), and the output
    ///      is its path, never its bytes; without save_to it is refused.
    ///      include_tins false: last fours only, base64 unless save_to.
    ///   2. With members, k1-members.csv has one row per [[company.members]]
    ///      member: name, party, share, capital account, TIN and type, and
    ///      the K-1 worksheet's capital figures (ledger_core::k1) for the
    ///      fiscal year to date. The 1099 worksheet adds each payee's party,
    ///      last four, TIN, and type.
    ///   3. A locked vault or a missing TIN never fails the package: the
    ///      row says why (vault_tin), the README lists it, and so does
    ///      tin_gaps (members, and 1099 payees that are reportable).
    ///   4. Read-only: nothing is journaled.
    /// ```
    #[tool(
        name = "books.export_cpa_package",
        title = "CPA handoff package",
        description = "Build the CPA handoff package as a zip: the profit and loss, balance sheet, cash flow, statement of changes in members' equity (multi-member LLCs), tax summary, and trial balance, each as PDF and XLSX; the fixed asset register as PDF and XLSX when there are fixed assets; the general ledger (by account) and general journal (entry by entry) as XLSX; the journal as IIF (imports into QuickBooks Desktop); the 1099 worksheet as CSV; with members, the K-1 worksheet (k1-members.csv: per member their party, share, capital account, TIN, and beginning capital, contributions in cash and in property, share of income, distributions, and ending capital for the fiscal year to date); every receipt behind the year's entries, filed on them or linked to them (receipts/, with an index.csv tying each file to its entry) and the company's documents (documents/), unless include_receipts is false; and a README with the journal's verified hash. By default (include_tins) the K-1 and 1099 worksheets carry every member's and payee's full TIN from the TIN vault, so the package is saved, never returned: save_to (a new .zip path or an existing folder, outside the books dir) is required, and the output gives only its path; tell the human where it is and never open, attach, or copy it. A locked vault or a missing TIN doesn't stop the package: tin_gaps lists each one (collect them with books.set_tin or books.set_w9 and export again). include_tins: false builds it with last fours only, returned as base64 unless save_to is given. cash_accounts (e.g. [\"1000\"]) is required: the cash flow statement reconciles to them. as_of defaults to today; period reports cover the fiscal year to date.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn export_cpa_package(
        &self,
        params: Parameters<ExportCpaPackageParams>,
    ) -> Result<Json<ExportCpaPackageOut>, String> {
        use ledger_import::{XlsxReport, report, report_to_xlsx, statement_pdf};
        let p = params.0;
        let as_of = match &p.as_of {
            Some(s) => parse_date(s)?,
            None => today(),
        };
        // A package with full TINs is only ever a file outside the books,
        // never tool output (INVARIANT 1).
        if p.include_tins && p.save_to.is_none() {
            return Err(
                "the CPA package carries every member's and 1099 payee's full TIN (include_tins, the default), so it is saved, never returned: pass save_to, a new .zip path or an existing folder outside the books dir, then tell the human where it is. (include_tins: false builds one with last fours only, returned as base64.)"
                    .into(),
            );
        }
        let file_name = format!("cpa-package-{as_of}.zip");
        let store = self.lock()?;
        let dest = p
            .save_to
            .as_deref()
            .map(|to| package_dest(&store, to, &file_name, p.include_tins))
            .transpose()?;
        if p.include_tins
            && let Some(d) = &dest
        {
            self.outside_import_roots(d)?;
        }
        let meta = self.report_meta(&store);
        let ledger = store.ledger();
        // The vault, opened once and never creating a key: locked, each
        // TIN row says so and the package is built anyway (INVARIANT 3).
        let vault = p.include_tins.then(|| self.open_vault(&store, false));
        let mut tin_gaps: Vec<String> = Vec::new();

        // Fiscal year to date, per wanderbooks.toml's fiscal_year_start.
        let (fy_start, _) = self.settings().company.fiscal_year_containing(as_of);
        let (fy_start, as_of_s) = (fy_start.to_string(), as_of.to_string());
        let xlsx = |r: XlsxReport<'_>| report_to_xlsx(&r, &meta).map_err(|e| e.to_string());
        let pdf =
            |st: &ledger_import::Statement| statement_pdf(st, &meta).map_err(|e| e.to_string());
        let fy = NaiveDate::parse_from_str(&fy_start, "%Y-%m-%d").map_err(|e| e.to_string())?;
        let members = self.report_members();

        // The financial statements, each as PDF and XLSX.
        let mut statements: Vec<(&str, ledger_import::Statement)> = vec![
            (
                "profit-and-loss",
                report::profit_and_loss(&pnl_statement(
                    ledger,
                    Some(&fy_start),
                    Some(&as_of_s),
                    None,
                )?),
            ),
            (
                "balance-sheet",
                report::balance_sheet(
                    &bs_statement(ledger, Some(&as_of_s))?,
                    self.year_earnings(ledger, as_of)?,
                    &members,
                ),
            ),
            (
                "cash-flow",
                report::cash_flow(
                    &self.cash_flow_for(
                        ledger,
                        Some(&fy_start),
                        Some(&as_of_s),
                        &p.cash_accounts,
                        &p.financing_accounts,
                        &p.investing_accounts,
                    )?,
                    &members,
                ),
            ),
        ];
        let mut k1_csv: Option<(String, Vec<u8>)> = None;
        if !self.settings().company.members.is_empty() {
            let capitals = self.member_capitals(ledger)?;
            let me = ledger_core::members_equity(ledger, fy, as_of, &capitals)
                .map_err(|e| e.to_string())?;
            statements.push(("members-equity", report::members_equity(&me)));
            // The K-1 worksheet: each member's capital account for the
            // year (Schedule K-1 Item L) and their TIN (INVARIANT 2).
            let cash_codes = if p.cash_accounts.is_empty() {
                &self.settings().roles.cash
            } else {
                &p.cash_accounts
            };
            let cash: Vec<AccountId> = cash_codes
                .iter()
                .filter_map(|c| ledger.chart().find_by_code(c).map(|a| a.id))
                .collect();
            k1_csv = Some(
                match ledger_core::k1::k1_worksheet(ledger, fy, as_of, &capitals, &cash) {
                    Ok(rows) => {
                        let mut w = String::from("member,party_id,share,capital_account,");
                        w.push_str(if p.include_tins {
                            "tin,tin_type,tin_note,"
                        } else {
                            "tin_last4,tin_type,"
                        });
                        w.push_str("beginning_capital,contributions_cash,contributions_property,net_income,distributions,other,ending_capital\n");
                        for (row, m) in rows.iter().zip(&self.settings().company.members) {
                            let party = ledger_core::k1::member_party(ledger, &m.name);
                            let facts = party
                                .and_then(|id| ledger.get_party(id))
                                .and_then(|p| p.tin.clone());
                            let mut cells = vec![
                                csv_escape(&m.name),
                                party.map(|id| id.to_string()).unwrap_or_default(),
                                m.share.to_string(),
                                m.capital_account.clone(),
                            ];
                            match &vault {
                                Some(vault) => {
                                    let got = match party {
                                        None => Err(format!(
                                            "TIN not in the vault: no party named {:?} with the member role (books.add_party with role member, or books.set_party_roles, then books.set_tin)",
                                            m.name
                                        )),
                                        Some(id) => vault_tin(ledger, vault, id),
                                    };
                                    match got {
                                        Ok(t) => {
                                            cells.push(t.formatted());
                                            cells.push(tin_type_key(t.kind).into());
                                            cells.push(String::new());
                                        }
                                        Err(why) => {
                                            tin_gaps.push(format!("member {}: {why}", m.name));
                                            cells.push(String::new());
                                            cells.push(
                                                facts
                                                    .map(|f| f.tin_type.key().to_string())
                                                    .unwrap_or_default(),
                                            );
                                            cells.push(csv_escape(&why));
                                        }
                                    }
                                }
                                None => {
                                    cells.push(
                                        facts.as_ref().map(|f| f.last4.clone()).unwrap_or_default(),
                                    );
                                    cells.push(
                                        facts
                                            .map(|f| f.tin_type.key().to_string())
                                            .unwrap_or_default(),
                                    );
                                }
                            }
                            for v in [
                                row.beginning,
                                row.contributions_cash,
                                row.contributions_property,
                                row.net_income,
                                row.distributions,
                                row.other,
                                row.ending,
                            ] {
                                cells.push(money(v));
                            }
                            w.push_str(&cells.join(","));
                            w.push('\n');
                        }
                        // The members together: ties to the 1065's Schedule L
                        // capital and the statement of members' equity. Not
                        // a K-1 (each member gets their own).
                        if rows.len() > 1 {
                            let sum = |f: fn(&ledger_core::k1::K1Row) -> Decimal| {
                                rows.iter().map(f).sum::<Decimal>()
                            };
                            let share: Decimal = self
                                .settings()
                                .company
                                .members
                                .iter()
                                .map(|m| m.share)
                                .sum();
                            let mut cells = vec![
                                csv_escape("Total, all members (not a K-1)"),
                                String::new(),
                                share.to_string(),
                                String::new(),
                                String::new(),
                                String::new(),
                            ];
                            if p.include_tins && vault.is_some() {
                                cells.push(String::new());
                            }
                            for v in [
                                sum(|r| r.beginning),
                                sum(|r| r.contributions_cash),
                                sum(|r| r.contributions_property),
                                sum(|r| r.net_income),
                                sum(|r| r.distributions),
                                sum(|r| r.other),
                                sum(|r| r.ending),
                            ] {
                                cells.push(money(v));
                            }
                            w.push_str(&cells.join(","));
                            w.push('\n');
                        }
                        ("k1-members.csv".into(), w.into_bytes())
                    }
                    Err(e) => (
                        "k1-members-NOT-INCLUDED.txt".into(),
                        format!("{e}\n").into_bytes(),
                    ),
                },
            );
        }
        statements.push((
            "tax-summary",
            report::tax_summary(
                &ledger_core::tax_summary(ledger, fy, as_of, self.settings().company.tax_form)
                    .map_err(|e| e.to_string())?,
            ),
        ));
        statements.push((
            "trial-balance",
            report::trial_balance(&reports::trial_balance_as_of(ledger, as_of), as_of),
        ));
        let mut files: Vec<(String, Vec<u8>)> = Vec::new();
        for (name, st) in &statements {
            files.push((format!("{name}.pdf"), pdf(st)?));
            files.push((format!("{name}.xlsx"), xlsx(XlsxReport::Statement(st))?));
        }
        // The fixed asset register, when there are fixed assets.
        if ledger.registered_assets().next().is_some() {
            let r = ledger_core::fixed_asset_register(ledger, as_of).map_err(|e| e.to_string())?;
            files.push((
                "fixed-asset-register.pdf".into(),
                ledger_import::asset_register_pdf(&r, &meta).map_err(|e| e.to_string())?,
            ));
            files.push((
                "fixed-asset-register.xlsx".into(),
                ledger_import::asset_register_xlsx(&r, &meta).map_err(|e| e.to_string())?,
            ));
        }
        // The books behind them: by account, and entry by entry.
        let gl = ledger_core::general_ledger(ledger, fy, as_of).map_err(|e| e.to_string())?;
        files.push((
            "general-ledger.xlsx".into(),
            xlsx(XlsxReport::GeneralLedger(&gl))?,
        ));
        files.push((
            "general-journal.xlsx".into(),
            xlsx(XlsxReport::GeneralJournal(ledger, fy, as_of))?,
        ));
        // 1099 worksheet for the calendar year of as_of.
        let year = chrono::Datelike::year(&as_of);
        match vendor_1099_report(ledger, &self.settings(), year, None) {
            Ok(r1099) => {
                let mut w = String::from("vendor,form,w9_on_file,paid,reportable");
                if vault.is_some() {
                    w.push_str(",party_id,tin_last4,tin,tin_type,tin_note");
                }
                w.push('\n');
                for v in &r1099.vendors {
                    w.push_str(&format!(
                        "\"{}\",{},{},{},{}",
                        v.name.replace('"', "\"\""),
                        v.form.map(|f| f.label()).unwrap_or(""),
                        v.tin_on_file,
                        money(v.paid),
                        v.reportable
                    ));
                    // The payee's full TIN from the vault (INVARIANT 2).
                    if let Some(vault) = &vault {
                        let party = ledger.party_of_vendor(v.vendor_id);
                        let last4 = party.and_then(|id| ledger.get_party(id)).and_then(|p| {
                            p.w9.as_ref()
                                .map(|f| f.tin_last4.clone())
                                .or_else(|| p.tin.as_ref().map(|t| t.last4.clone()))
                        });
                        let got = match party {
                            None => Err("TIN not in the vault: no party for this vendor".into()),
                            Some(id) => vault_tin(ledger, vault, id),
                        };
                        let (tin, kind, note) = match got {
                            Ok(t) => (
                                t.formatted(),
                                tin_type_key(t.kind).to_string(),
                                String::new(),
                            ),
                            Err(why) => {
                                if v.reportable {
                                    tin_gaps.push(format!("1099 payee {}: {why}", v.name));
                                }
                                (String::new(), String::new(), why)
                            }
                        };
                        w.push_str(&format!(
                            ",{},{},{tin},{kind},{}",
                            party.map(|id| id.to_string()).unwrap_or_default(),
                            last4.unwrap_or_default(),
                            csv_escape(&note)
                        ));
                    }
                    w.push('\n');
                }
                files.push((format!("1099-worksheet-{year}.csv"), w.into_bytes()));
            }
            // The threshold isn't known for this year: never guess it
            // (rules.md B-19); say so in the package instead.
            Err(e) => files.push((
                format!("1099-worksheet-{year}-NOT-INCLUDED.txt"),
                format!("{e}\n").into_bytes(),
            )),
        }
        // The evidence: every current file behind the year's posted
        // entries, filed on them or linked to them (a file linked to an
        // entry counts as its receipt), one copy per entry with a row in
        // the index; and the company's documents, the files behind no
        // entry.
        let mut receipt_count = 0;
        if p.include_receipts {
            let clean = |s: &str| -> String {
                s.chars()
                    .map(|c| {
                        if c.is_alphanumeric() || ".-_ ".contains(c) {
                            c
                        } else {
                            '_'
                        }
                    })
                    .collect()
            };
            let mut index = String::from("entry_id,date,description,amount,file,sha256\n");
            for a in store.list_attachments(None).map_err(|e| e.to_string())? {
                if a.withdrawn_at.is_some() {
                    continue;
                }
                let mut behind: Vec<u64> = a.entry_id.into_iter().collect();
                for sub in store.attachment_links(a.id).map_err(|e| e.to_string())? {
                    if let ledger_store::Subject::Entry(e) = sub
                        && !behind.contains(&e)
                    {
                        behind.push(e);
                    }
                }
                behind.sort_unstable();
                let bytes = store.attachment_bytes(a.id).map_err(|e| e.to_string())?;
                if behind.is_empty() {
                    if a.category != "receipts" {
                        files.push((
                            format!("documents/{}/{}", clean(&a.category), clean(&a.file_name)),
                            bytes,
                        ));
                    }
                    continue;
                }
                for e in behind.iter().filter_map(|&e| ledger.get_entry(e)) {
                    if e.status() != ledger_core::EntryStatus::Posted
                        || e.date() < fy
                        || e.date() > as_of
                    {
                        continue;
                    }
                    let name = format!(
                        "receipts/{}-entry-{}-{}",
                        e.date(),
                        e.id(),
                        clean(&a.file_name)
                    );
                    let amount: Decimal = e
                        .lines()
                        .iter()
                        .filter(|l| l.side == Side::Debit)
                        .map(|l| l.amount.amount)
                        .sum();
                    index.push_str(&format!(
                        "{},{},\"{}\",{},{},{}\n",
                        e.id(),
                        e.date(),
                        e.description().replace('"', "\"\""),
                        money(amount),
                        name,
                        a.blob_sha
                    ));
                    files.push((name, bytes.clone()));
                    receipt_count += 1;
                }
            }
            files.push(("receipts/index.csv".into(), index.into_bytes()));
        }
        if let Some(k1) = k1_csv {
            files.push(k1);
        }
        let seq = store.journal_seq();
        let vault_dir = Self::vault_dir(&store);
        drop(store);
        files.push((
            "general-journal.iif".into(),
            self.export_iif()?.into_bytes(),
        ));
        let readme = format!(
            "{name} — books package for the CPA\n\nFiscal year to date: {fy_start} through {as_of_s}. {basis} basis.\nKept in wanderbooks: double-entry books from an append-only, hash-chained journal ({seq} events, verified).\n\nStatements (PDF and XLSX): profit and loss, balance sheet, cash flow, {members}tax summary, trial balance.\nLedgers (XLSX): general ledger by account; general journal entry by entry.\ngeneral-journal.iif: every posted entry, for import into QuickBooks Desktop.\n1099-worksheet-{year}.csv: cash paid per vendor marked for a 1099, against the year's threshold{payee_tins}.\n{k1}{receipts}{tins}\nThe tax summary maps accounts to return categories; it's a worksheet, not a filing. Questions go to the owners, who can ask their bookkeeping agent to trace any number to its entries and receipts.\n",
            name = self.settings().company.name,
            basis = self.settings().company.basis,
            members = if self.settings().company.members.is_empty() {
                ""
            } else {
                "members' equity, "
            },
            payee_tins = if p.include_tins {
                ", with each payee's TIN"
            } else {
                ""
            },
            k1 = if self.settings().company.members.is_empty() {
                String::new()
            } else {
                {
                    let mut k = format!(
                        "k1-members.csv: each member's capital account for the year (Schedule K-1 Item L: beginning, contributions in cash and in property, share of income, distributions, ending), with their {}; the last row totals the members (not a K-1) to tie to Schedule L and the statement of members' equity.\n",
                        if p.include_tins {
                            "taxpayer ID"
                        } else {
                            "taxpayer ID's last four"
                        }
                    );
                    let members = &self.settings().company.members;
                    for (i, m) in members.iter().enumerate() {
                        if let Some(j) = &m.files_jointly_with
                            && members[..i].iter().all(|o| o.name != *j)
                        {
                            k.push_str(&format!(
                                "{} and {j} file a joint individual return: each receives their own Schedule K-1; both are reported on the one Form 1040 (Schedule E, Part II).\n",
                                m.name
                            ));
                        }
                    }
                    k
                }
            },
            tins = if p.include_tins {
                let mut t = String::from(
                    "\nThis package holds full taxpayer identification numbers: keep it out of email and shared folders, and delete copies once filed.\n",
                );
                if !tin_gaps.is_empty() {
                    t.push_str("TINs not included:\n");
                    for g in &tin_gaps {
                        t.push_str(&format!("- {g}\n"));
                    }
                }
                t
            } else {
                String::new()
            },
            receipts = if p.include_receipts {
                format!(
                    "receipts/: the {receipt_count} receipts behind the period's entries; index.csv ties each to its entry, date, amount, and SHA-256.\ndocuments/: the company's formation, legal, and tax documents.\n"
                )
            } else {
                String::new()
            },
        );
        files.insert(0, ("README.txt".into(), readme.into_bytes()));

        // Zip it.
        let mut zip_buf = Vec::new();
        {
            let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut zip_buf));
            let opts = zip::write::SimpleFileOptions::default();
            for (name, bytes) in &files {
                zip.start_file(name.as_str(), opts)
                    .map_err(|e| e.to_string())?;
                use std::io::Write;
                zip.write_all(bytes).map_err(|e| e.to_string())?;
            }
            zip.finish().map_err(|e| e.to_string())?;
        }

        let shown = |path: &std::path::Path| path.display().to_string();
        let (content_base64, saved_to, note) = match &dest {
            Some(path) => {
                if p.include_tins {
                    // The zip and each file in it that holds a TIN (the
                    // K-1 and 1099 worksheets): one extracted is refused
                    // too (tin_files.rs INVARIANT 1).
                    for (name, bytes) in &files {
                        let filed = name.starts_with("receipts/") || name.starts_with("documents/");
                        if !filed
                            && ledger_core::parties::has_full_tin(&String::from_utf8_lossy(bytes))
                        {
                            tin_files::record(&vault_dir, bytes)?;
                        }
                    }
                    tin_files::record(&vault_dir, &zip_buf)?;
                }
                write_private_new(path, &zip_buf)?;
                let note = if p.include_tins {
                    format!(
                        "Saved the CPA package to {} (readable by you only). It holds full TINs: tell the human where it is, never open, attach, or copy it into the books, and have them delete it once the CPA has it.{}",
                        shown(path),
                        if tin_gaps.is_empty() {
                            String::new()
                        } else {
                            format!(
                                " {} TIN(s) are missing (tin_gaps): collect them with books.set_tin or books.set_w9 and export again.",
                                tin_gaps.len()
                            )
                        }
                    )
                } else {
                    format!(
                        "Saved the CPA package to {} (last fours only).",
                        shown(path)
                    )
                };
                (None, Some(shown(path)), note)
            }
            None => (
                Some(base64::Engine::encode(
                    &base64::engine::general_purpose::STANDARD,
                    &zip_buf,
                )),
                None,
                "The package, without full TINs (include_tins false).".to_string(),
            ),
        };
        Ok(Json(ExportCpaPackageOut {
            file_name,
            content_base64,
            saved_to,
            tins_included: p.include_tins,
            tin_gaps,
            note,
        }))
    }
}

// ---------------------------------------------------------------------------
// Export builders (called under the tool lock via &self)
// ---------------------------------------------------------------------------

impl BooksServer {
    fn export_journal(&self, format: &str) -> Result<String, String> {
        let store = self.lock()?;
        let ledger = store.ledger();
        if format == "json" {
            let rows: Vec<serde_json::Value> = ledger
                .posted_entries()
                .map(|e| {
                    serde_json::json!({
                        "entry_id": e.id(),
                        "date": e.date().to_string(),
                        "description": e.description(),
                        "reversal_of": e.reversal_of(),
                        "lines": e.lines().iter().map(|l| {
                            let a = ledger.chart().get(l.account).unwrap();
                            serde_json::json!({
                                "account_code": a.code,
                                "account_name": a.name,
                                "side": match l.side { Side::Debit => "debit", Side::Credit => "credit" },
                                "amount": money(l.amount.amount),
                                "currency": l.amount.currency.code(),
                            })
                        }).collect::<Vec<_>>(),
                    })
                })
                .collect();
            return serde_json::to_string_pretty(&rows).map_err(|e| e.to_string());
        }
        let mut csv = String::from(
            "entry_id,date,description,reversal_of,account_code,account_name,side,amount,currency\n",
        );
        for e in ledger.posted_entries() {
            for l in e.lines() {
                let a = ledger.chart().get(l.account).unwrap();
                csv.push_str(&format!(
                    "{},{},{},{},{},{},{},{},{}\n",
                    e.id(),
                    e.date(),
                    csv_escape(e.description()),
                    e.reversal_of().map(|v| v.to_string()).unwrap_or_default(),
                    a.code,
                    csv_escape(&a.name),
                    match l.side {
                        Side::Debit => "debit",
                        Side::Credit => "credit",
                    },
                    money(l.amount.amount),
                    l.amount.currency.code(),
                ));
            }
        }
        Ok(csv)
    }

    fn export_trial_balance(&self, format: &str, as_of: Option<&str>) -> Result<String, String> {
        let store = self.lock()?;
        let as_of = match as_of {
            Some(s) => parse_date(s)?,
            None => today(),
        };
        let tb = reports::trial_balance_as_of(store.ledger(), as_of);
        if format == "json" {
            return serde_json::to_string_pretty(&serde_json::json!({
                "as_of": as_of.to_string(),
                "lines": tb.lines.iter().map(|l| serde_json::json!({
                    "code": l.code, "name": l.name,
                    "debit": money(l.debit), "credit": money(l.credit),
                })).collect::<Vec<_>>(),
                "total_debits": money(tb.total_debits),
                "total_credits": money(tb.total_credits),
                "balanced": tb.balanced(),
            }))
            .map_err(|e| e.to_string());
        }
        let mut csv = String::from("code,name,debit,credit\n");
        for l in &tb.lines {
            csv.push_str(&format!(
                "{},{},{},{}\n",
                l.code,
                csv_escape(&l.name),
                money(l.debit),
                money(l.credit)
            ));
        }
        csv.push_str(&format!(
            "TOTAL,,{},{}",
            money(tb.total_debits),
            money(tb.total_credits)
        ));
        Ok(csv)
    }

    /// The fixed asset register (ledger_core::fixed_asset_register) as
    /// JSON or CSV, reconciled to the books.
    fn export_fixed_asset_register(
        &self,
        format: &str,
        as_of: Option<&str>,
    ) -> Result<String, String> {
        let store = self.lock()?;
        let as_of = match as_of {
            Some(s) => parse_date(s)?,
            None => today(),
        };
        let r =
            ledger_core::fixed_asset_register(store.ledger(), as_of).map_err(|e| e.to_string())?;
        // The same words as the XLSX and PDF register.
        let status = |row: &ledger_core::AssetRegisterRow| match row.disposed {
            Some((d, p)) => format!("Disposed {d}, proceeds {}", money(p)),
            None => "Active".to_string(),
        };
        if format == "json" {
            return serde_json::to_string_pretty(&serde_json::json!({
                "as_of": as_of.to_string(),
                "assets": r.rows.iter().map(|a| serde_json::json!({
                    "asset_id": a.asset_id,
                    "tag": a.tag,
                    "description": a.description,
                    "serial": a.serial,
                    "in_service": a.in_service.to_string(),
                    "cost": money(a.cost),
                    "salvage": money(a.salvage),
                    "useful_life_months": a.useful_life_months,
                    "accumulated": money(a.accumulated),
                    "book_value": money(a.book_value),
                    "status": status(a),
                    "proceeds": a.disposed.map(|(_, p)| money(p)),
                    "tax_basis_rule": a.tax_rule.map(|r| r.key()),
                    "contributor_basis": a.contributor_basis.map(money),
                    "tax_basis": a.tax_basis.map(money),
                    "built_in_gain": a.built_in_gain.map(money),
                })).collect::<Vec<_>>(),
                "total_tax_basis": money(r.total_tax_basis),
                "missing_tax_basis": r.missing_tax_basis,
                "total_cost": money(r.total_cost),
                "total_accumulated": money(r.total_accumulated),
                "total_book_value": money(r.total_book_value),
                "ledger_cost": money(r.ledger_cost),
                "ledger_accumulated": money(r.ledger_accumulated),
                "unposted_depreciation": money(r.unposted_depreciation()),
                "unregistered_cost": money(r.unregistered_cost()),
            }))
            .map_err(|e| e.to_string());
        }
        let mut csv = String::from(
            "tag,description,serial,in_service,cost,salvage,useful_life_months,accumulated,book_value,status,proceeds,tax_basis_rule,contributor_basis,tax_basis,built_in_gain\n",
        );
        for a in &r.rows {
            csv.push_str(&format!(
                "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}\n",
                csv_escape(a.tag.as_deref().unwrap_or("")),
                csv_escape(&a.description),
                csv_escape(a.serial.as_deref().unwrap_or("")),
                a.in_service,
                money(a.cost),
                money(a.salvage),
                a.useful_life_months,
                money(a.accumulated),
                money(a.book_value),
                status(a),
                a.disposed.map(|(_, p)| money(p)).unwrap_or_default(),
                a.tax_rule.map(|r| r.key()).unwrap_or_default(),
                a.contributor_basis.map(money).unwrap_or_default(),
                a.tax_basis.map(money).unwrap_or_default(),
                a.built_in_gain.map(money).unwrap_or_default(),
            ));
        }
        csv.push_str(&format!(
            "TOTAL,,,,{},,,{},{},,,,,{},",
            money(r.total_cost),
            money(r.total_accumulated),
            money(r.total_book_value),
            money(r.total_tax_basis)
        ));
        Ok(csv)
    }

    fn export_general_ledger(&self, format: &str, p: &ExportParams) -> Result<String, String> {
        let store = self.lock()?;
        let ledger = store.ledger();
        let (start, end) = statement_range(ledger, p.start.as_deref(), p.end.as_deref())?;
        let gl = ledger_core::general_ledger(ledger, start, end).map_err(|e| e.to_string())?;
        if format == "json" {
            return serde_json::to_string_pretty(&serde_json::json!({
                "start": gl.start.to_string(), "end": gl.end.to_string(),
                "accounts": gl.accounts.iter().map(|a| serde_json::json!({
                    "code": a.code, "name": a.name,
                    "beginning_balance": money(a.beginning_balance),
                    "postings": a.postings.iter().map(|p| serde_json::json!({
                        "date": p.date.to_string(), "entry_id": p.entry_id,
                        "description": p.description, "split": p.split,
                        "debit": money(p.debit), "credit": money(p.credit),
                        "balance": money(p.balance),
                    })).collect::<Vec<_>>(),
                    "total_debits": money(a.total_debits),
                    "total_credits": money(a.total_credits),
                    "ending_balance": money(a.ending_balance),
                })).collect::<Vec<_>>(),
            }))
            .map_err(|e| e.to_string());
        }
        let mut csv = String::from("account,date,entry,description,split,debit,credit,balance\n");
        for a in &gl.accounts {
            let account = csv_escape(&format!("{} {}", a.code, a.name));
            csv.push_str(&format!(
                "{account},,,Beginning balance,,,,{}\n",
                money(a.beginning_balance)
            ));
            for p in &a.postings {
                csv.push_str(&format!(
                    "{account},{},{},{},{},{},{},{}\n",
                    p.date,
                    p.entry_id,
                    csv_escape(&p.description),
                    csv_escape(&p.split),
                    money(p.debit),
                    money(p.credit),
                    money(p.balance)
                ));
            }
            csv.push_str(&format!(
                "{account},,,Total,,{},{},{}\n",
                money(a.total_debits),
                money(a.total_credits),
                money(a.ending_balance)
            ));
        }
        Ok(csv)
    }

    fn export_tax_summary(&self, format: &str, p: &ExportParams) -> Result<String, String> {
        let store = self.lock()?;
        let ledger = store.ledger();
        let (start, end) = statement_range(ledger, p.start.as_deref(), p.end.as_deref())?;
        let ts = ledger_core::tax_summary(ledger, start, end, self.settings().company.tax_form)
            .map_err(|e| e.to_string())?;
        let line = |l: &ledger_core::TaxSummaryLine| {
            serde_json::json!({
                "category": l.category, "label": l.label, "accounts": l.accounts,
                "book": money(l.book), "tax": money(l.tax),
            })
        };
        if format == "json" {
            return serde_json::to_string_pretty(&serde_json::json!({
                "start": ts.start.to_string(), "end": ts.end.to_string(),
                "form": ts.form.map(|f| f.label()),
                "income": ts.income.iter().map(line).collect::<Vec<_>>(),
                "cost_of_goods": ts.cost_of_goods.iter().map(line).collect::<Vec<_>>(),
                "deductions": ts.deductions.iter().map(line).collect::<Vec<_>>(),
                "separately_stated": ts.separately_stated.iter().map(line).collect::<Vec<_>>(),
                "uncategorized": ts.uncategorized.iter().map(|u| serde_json::json!({
                    "account": u.account, "book": money(u.book),
                })).collect::<Vec<_>>(),
                "book_net": money(ts.book_net()),
                "tax_net": money(ts.tax_net()),
            }))
            .map_err(|e| e.to_string());
        }
        let mut csv = String::from("section,category,label,accounts,book,tax\n");
        for (section, lines) in [
            ("income", &ts.income),
            ("cost_of_goods", &ts.cost_of_goods),
            ("deductions", &ts.deductions),
            ("separately_stated", &ts.separately_stated),
        ] {
            for l in lines {
                csv.push_str(&format!(
                    "{section},{},{},{},{},{}\n",
                    l.category,
                    csv_escape(&l.label),
                    csv_escape(&l.accounts.join("; ")),
                    money(l.book),
                    money(l.tax)
                ));
            }
        }
        for u in &ts.uncategorized {
            csv.push_str(&format!(
                "uncategorized,,,{},{},\n",
                csv_escape(&u.account),
                money(u.book)
            ));
        }
        Ok(csv)
    }

    fn export_members_equity(&self, format: &str, p: &ExportParams) -> Result<String, String> {
        let store = self.lock()?;
        let ledger = store.ledger();
        let (start, end) = statement_range(ledger, p.start.as_deref(), p.end.as_deref())?;
        let me = ledger_core::members_equity(ledger, start, end, &self.member_capitals(ledger)?)
            .map_err(|e| e.to_string())?;
        let col = |c: &ledger_core::MemberEquityColumn| {
            serde_json::json!({
                "name": c.name, "beginning": money(c.beginning),
                "contributions": money(c.contributions), "net_income": money(c.net_income),
                "distributions": money(c.distributions), "other": money(c.other),
                "ending": money(c.ending),
            })
        };
        if format == "json" {
            return serde_json::to_string_pretty(&serde_json::json!({
                "start": me.start.to_string(), "end": me.end.to_string(),
                "members": me.members.iter().map(col).collect::<Vec<_>>(),
                "total": col(&me.total),
            }))
            .map_err(|e| e.to_string());
        }
        let mut csv =
            String::from("member,beginning,contributions,net_income,distributions,other,ending\n");
        for c in me.members.iter().chain(std::iter::once(&me.total)) {
            csv.push_str(&format!(
                "{},{},{},{},{},{},{}\n",
                csv_escape(&c.name),
                money(c.beginning),
                money(c.contributions),
                money(c.net_income),
                money(c.distributions),
                money(c.other),
                money(c.ending)
            ));
        }
        Ok(csv)
    }

    fn export_pnl(
        &self,
        format: &str,
        start: Option<&str>,
        end: Option<&str>,
    ) -> Result<String, String> {
        let store = self.lock()?;
        let pl = pnl_statement(store.ledger(), start, end, None)?;
        if format == "json" {
            return serde_json::to_string_pretty(&serde_json::json!({
                "start": pl.start.to_string(), "end": pl.end.to_string(),
                "revenue_accounts": lines_out(&pl.revenue),
                "expense_accounts": lines_out(&pl.expenses),
                "revenue": money(pl.total_revenue), "expenses": money(pl.total_expenses),
                "net_income": money(pl.net_income()),
            }))
            .map_err(|e| e.to_string());
        }
        let mut csv = String::from("section,code,name,amount\n");
        for (section, lines) in [("revenue", &pl.revenue), ("expenses", &pl.expenses)] {
            for l in lines {
                csv.push_str(&format!(
                    "{section},{},{},{}\n",
                    csv_escape(&l.code),
                    csv_escape(&l.name),
                    money(l.amount)
                ));
            }
        }
        csv.push_str(&format!(
            "total,,revenue,{}\ntotal,,expenses,{}\ntotal,,net_income,{}\n",
            money(pl.total_revenue),
            money(pl.total_expenses),
            money(pl.net_income())
        ));
        Ok(csv)
    }

    fn export_balance_sheet(&self, format: &str, as_of: Option<&str>) -> Result<String, String> {
        let store = self.lock()?;
        let bs = bs_statement(store.ledger(), as_of)?;
        if format == "json" {
            return serde_json::to_string_pretty(&serde_json::json!({
                "as_of": bs.as_of.to_string(),
                "asset_accounts": lines_out(&bs.assets),
                "liability_accounts": lines_out(&bs.liabilities),
                "equity_accounts": lines_out(&bs.equity),
                "assets": money(bs.total_assets), "liabilities": money(bs.total_liabilities),
                "equity": money(bs.total_equity), "current_earnings": money(bs.current_earnings),
                "balanced": bs.balanced(),
            }))
            .map_err(|e| e.to_string());
        }
        let mut csv = String::from("section,code,name,amount\n");
        for (section, lines) in [
            ("assets", &bs.assets),
            ("liabilities", &bs.liabilities),
            ("equity", &bs.equity),
        ] {
            for l in lines {
                csv.push_str(&format!(
                    "{section},{},{},{}\n",
                    csv_escape(&l.code),
                    csv_escape(&l.name),
                    money(l.amount)
                ));
            }
        }
        csv.push_str(&format!(
            "equity,,current_earnings,{}\ntotal,,assets,{}\ntotal,,liabilities,{}\ntotal,,equity,{}\n",
            money(bs.current_earnings),
            money(bs.total_assets),
            money(bs.total_liabilities),
            money(bs.total_equity)
        ));
        Ok(csv)
    }

    fn export_cash_flow(&self, format: &str, p: &ExportParams) -> Result<String, String> {
        let store = self.lock()?;
        let ledger = store.ledger();
        let stmt = self.cash_flow_for(
            ledger,
            p.start.as_deref(),
            p.end.as_deref(),
            &p.cash_accounts,
            &p.financing_accounts,
            &p.investing_accounts,
        )?;
        if format == "json" {
            return serde_json::to_string_pretty(&serde_json::json!({
                "start": stmt.start.to_string(), "end": stmt.end.to_string(),
                "net_income": money(stmt.net_income),
                "depreciation": money(stmt.depreciation),
                "working_capital": stmt.working_capital.iter().map(|l| serde_json::json!({
                    "label": l.label, "amount": money(l.amount),
                })).collect::<Vec<_>>(),
                "cash_from_operations": money(stmt.cash_from_operations),
                "investing": stmt.investing.iter().map(|l| serde_json::json!({
                    "label": l.label, "amount": money(l.amount),
                })).collect::<Vec<_>>(),
                "cash_from_investing": money(stmt.cash_from_investing),
                "financing": stmt.financing.iter().map(|l| serde_json::json!({
                    "label": l.label, "amount": money(l.amount),
                })).collect::<Vec<_>>(),
                "cash_from_financing": money(stmt.cash_from_financing),
                "net_change_in_cash": money(stmt.net_change_in_cash),
                "cash_beginning": money(stmt.cash_beginning),
                "cash_ending": money(stmt.cash_ending),
            }))
            .map_err(|e| e.to_string());
        }
        let mut csv = String::from("section,label,amount\n");
        csv.push_str(&format!(
            "operating,Net income,{}\n",
            money(stmt.net_income)
        ));
        csv.push_str(&format!(
            "operating,Depreciation,{}\n",
            money(stmt.depreciation)
        ));
        for l in &stmt.working_capital {
            csv.push_str(&format!(
                "operating,{},{}\n",
                csv_escape(&l.label),
                money(l.amount)
            ));
        }
        csv.push_str(&format!(
            "operating,Cash from operations,{}\n",
            money(stmt.cash_from_operations)
        ));
        for l in &stmt.investing {
            csv.push_str(&format!(
                "investing,{},{}\n",
                csv_escape(&l.label),
                money(l.amount)
            ));
        }
        csv.push_str(&format!(
            "investing,Cash from investing,{}\n",
            money(stmt.cash_from_investing)
        ));
        for l in &stmt.financing {
            csv.push_str(&format!(
                "financing,{},{}\n",
                csv_escape(&l.label),
                money(l.amount)
            ));
        }
        csv.push_str(&format!(
            "financing,Cash from financing,{}\n",
            money(stmt.cash_from_financing)
        ));
        csv.push_str(&format!(
            "reconciliation,Net change in cash,{}\n",
            money(stmt.net_change_in_cash)
        ));
        Ok(csv)
    }
}

fn csv_escape(s: &str) -> String {
    if s.contains([',', '"', '\n']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

// ---------------------------------------------------------------------------
// ServerHandler
// ---------------------------------------------------------------------------

impl BooksServer {
    /// Render a resource URI to its JSON text. Used by `read_resource`.
    async fn resource_text(&self, uri: &str) -> Result<String, String> {
        // Split off the query string for URIs like books://digest?since=...
        let (path, query) = match uri.split_once('?') {
            Some((p, q)) => (p, q),
            None => (uri, ""),
        };
        match path {
            "books://start" => {
                let Json(out) = self.start(Parameters(kept::StartParams::default())).await?;
                serde_json::to_string_pretty(&out).map_err(|e| e.to_string())
            }
            "books://settings" => {
                let Json(out) = self.settings_tool().await?;
                serde_json::to_string_pretty(&out).map_err(|e| e.to_string())
            }
            "books://conventions" => {
                let Json(out) = self
                    .conventions(Parameters(kept::ConventionsParams::default()))
                    .await?;
                serde_json::to_string_pretty(&out).map_err(|e| e.to_string())
            }
            "books://chart" => {
                let Json(out) = self.chart().await?;
                serde_json::to_string_pretty(&out).map_err(|e| e.to_string())
            }
            "books://reports/trial-balance" => {
                let Json(out) = self
                    .trial_balance(Parameters(TrialBalanceParams::default()))
                    .await?;
                serde_json::to_string_pretty(&out).map_err(|e| e.to_string())
            }
            "books://reports/balance-sheet" => {
                let Json(out) = self
                    .balance_sheet(Parameters(BalanceSheetParams { as_of: None }))
                    .await?;
                serde_json::to_string_pretty(&out).map_err(|e| e.to_string())
            }
            "books://reports/profit-and-loss" => {
                let Json(out) = self
                    .profit_and_loss(Parameters(ProfitAndLossParams {
                        start: None,
                        end: None,
                        class: None,
                    }))
                    .await?;
                serde_json::to_string_pretty(&out).map_err(|e| e.to_string())
            }
            "books://digest" => {
                let since = digest_since(query, today());
                let Json(out) = self.digest(Parameters(DigestParams { since })).await?;
                serde_json::to_string_pretty(&out).map_err(|e| e.to_string())
            }
            handbook::URI => {
                let Json(out) = self.handbook(Parameters(HandbookParams::default())).await?;
                serde_json::to_string_pretty(&out).map_err(|e| e.to_string())
            }
            _ if path.starts_with(&format!("{}/", handbook::URI)) => {
                let id = &path[handbook::URI.len() + 1..];
                handbook::section(id, &self.settings(), self.currency())
                    .ok_or_else(|| format!("unknown handbook section: {id}"))
            }
            _ if path.starts_with("books://receipts/") => {
                let id: u64 = path
                    .strip_prefix("books://receipts/")
                    .unwrap_or("")
                    .parse()
                    .map_err(|_| format!("bad receipt resource URI: {uri}"))?;
                let Json(out) = self
                    .get_receipt(Parameters(GetReceiptParams { attachment_id: id }))
                    .await?;
                serde_json::to_string_pretty(&out).map_err(|e| e.to_string())
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
        // Pick up an edited settings file before the tool runs.
        self.live().refresh();
        let timezone = self.settings().timezone.clone();
        let server = self.clone();
        let result = run_blocking(&self.store, move || {
            Box::pin(async move {
                let tcc =
                    rmcp::handler::server::tool::ToolCallContext::new(&server, request, context);
                let router = Self::tool_router();
                in_timezone_async(timezone, router.call(tcc)).await
            })
        })
        .await;
        // A failed run (the server stopped, or this request panicked) is a
        // plain refusal, redacted like every other.
        let result = match result {
            Ok(out) => out,
            Err(e) => Ok(rmcp::model::CallToolResponse::Complete(
                rmcp::model::CallToolResult::error(vec![rmcp::model::ContentBlock::text(
                    e.message.to_string(),
                )]),
            )),
        };
        redact_refusal(result)
    }

    fn get_info(&self) -> ServerConfig {
        // A client connecting after a settings change sees the change:
        // the books' own when they keep them (never waiting for the books
        // here; a request holding them syncs them when it locks), else the
        // files'.
        if let Ok(store) = self.store.store.try_lock() {
            self.live()
                .sync(store.ledger().settings(), self.store.currency.code());
        }
        self.live().refresh();
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
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

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, rmcp::ErrorData> {
        self.live().refresh();
        let timezone = self.settings().timezone.clone();
        let uri = request.uri.clone();
        let server = self.clone();
        let at = uri.clone();
        let text = run_blocking(&self.store, move || {
            Box::pin(async move { in_timezone_async(timezone, server.resource_text(&at)).await })
        })
        .await?;
        // A refusal quotes the URI it was given: masked like a tool's.
        let text = text.map_err(|e| {
            redact_error(rmcp::ErrorData::resource_not_found(
                uri.clone(),
                Some(serde_json::json!({"reason": e})),
            ))
        })?;
        let mime_type = if uri.starts_with(&format!("{}/", handbook::URI)) {
            "text/markdown"
        } else {
            "application/json"
        };
        Ok(ReadResourceResponse::Complete(ReadResourceResult::new(
            vec![ResourceContents::TextResourceContents {
                uri,
                mime_type: Some(mime_type.to_string()),
                text,
                meta: None,
            }],
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ledger_config::Member;
    use std::str::FromStr;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn usd(s: &str) -> Money {
        Money::new(Decimal::from_str(s).unwrap(), Currency::USD)
    }

    /// A test's books dir, removed when the test ends, pass or panic.
    struct TestDir(std::path::PathBuf);

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn unit_dir(tag: &str) -> TestDir {
        let dir =
            std::env::temp_dir().join(format!("ledger-mcp-unit-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        TestDir(dir)
    }

    /// New books in `dir`, which outlives them.
    fn store(dir: &TestDir) -> Store {
        Store::create(&dir.0, Currency::USD).unwrap()
    }

    /// lines_settled: posted, or a draft with lines whose debits equal its
    /// credits; a draft whose sides are past exact arithmetic (one an
    /// earlier version journaled, rules.md B-28) is not settled.
    #[test]
    fn settled_lines_are_posted_or_balanced_drafts() {
        let mut l = Ledger::new(Currency::USD);
        let cash = l
            .add_account("1000", "Cash", ledger_core::AccountType::Asset, None, false)
            .unwrap();
        let rent = l
            .add_account(
                "6000",
                "Rent",
                ledger_core::AccountType::Expense,
                None,
                false,
            )
            .unwrap();
        let mut draft = |lines: &[(u64, Side, Decimal)]| {
            let e = l.create_entry(d(2026, 3, 1), "x");
            for (a, side, amount) in lines {
                l.replay_add_line(e, *a, *side, Money::new(*amount, Currency::USD))
                    .unwrap();
            }
            e
        };
        let ten = Decimal::TEN;
        let empty = draft(&[]);
        let balanced = draft(&[(rent, Side::Debit, ten), (cash, Side::Credit, ten)]);
        let unbalanced = draft(&[(rent, Side::Debit, ten), (cash, Side::Credit, Decimal::ONE)]);
        let huge = draft(&[
            (rent, Side::Debit, Decimal::MAX),
            (rent, Side::Debit, Decimal::MAX),
            (cash, Side::Credit, Decimal::MAX),
            (cash, Side::Credit, Decimal::MAX),
        ]);
        let posted = draft(&[(rent, Side::Debit, ten), (cash, Side::Credit, ten)]);
        l.post_entry(posted).unwrap();
        let settled = |e: u64| lines_settled(l.get_entry(e).unwrap());
        assert!(!settled(empty));
        assert!(settled(balanced));
        assert!(!settled(unbalanced));
        assert!(!settled(huge));
        assert!(settled(posted));
    }

    fn post(s: &mut Store, date: NaiveDate, dr: AccountId, cr: AccountId, amount: &str) -> u64 {
        let e = s.create_entry(date, "x").unwrap();
        s.add_line(e, dr, Side::Debit, usd(amount)).unwrap();
        s.add_line(e, cr, Side::Credit, usd(amount)).unwrap();
        s.post_entry(e).unwrap();
        e
    }

    fn members() -> Company {
        Company {
            members: vec![
                Member {
                    name: "Ada".into(),
                    share: Decimal::new(5, 1),
                    capital_account: "3000".into(),
                    distribution_account: Some("3100".into()),
                    files_jointly_with: None,
                },
                Member {
                    name: "Bo".into(),
                    share: Decimal::new(5, 1),
                    capital_account: "3001".into(),
                    distribution_account: None,
                    files_jointly_with: None,
                },
            ],
            ..Company::default()
        }
    }

    /// "Max 25 MB" (books.attach_receipt): 25 MiB exactly.
    #[test]
    fn the_upload_limit_is_25_mib() {
        assert_eq!(MAX_UPLOAD_BYTES, 25 * 1024 * 1024);
        assert_eq!(MAX_UPLOAD_BYTES, 26_214_400);
    }

    /// A period is "YYYY-MM": a year of the calendar the books keep
    /// (1-9999), month 1-12, nothing more; its last day is a date, never a
    /// panic (P0 review round 3, S1: "999999-01" and "262142-12" once
    /// stopped the server).
    #[test]
    fn periods_are_a_year_and_a_month() {
        assert_eq!(parse_period_mcp("2026-01"), Ok((2026, 1)));
        assert_eq!(parse_period_mcp("2026-12"), Ok((2026, 12)));
        assert_eq!(parse_period_mcp("0001-01"), Ok((1, 1)));
        assert_eq!(parse_period_mcp("9999-12"), Ok((9999, 12)));
        for bad in [
            "2026-13",
            "2026-00",
            "2026-01-05",
            "0-05",
            "2026",
            "x-01",
            "2026-",
            "",
            "10000-01",
            "999999-01",
            "262142-12",
            "262143-01",
            "-5-01",
        ] {
            assert!(parse_period_mcp(bad).is_err(), "{bad:?}");
        }
        assert_eq!(last_day_of_month(2028, 2), Ok(d(2028, 2, 29)));
        assert_eq!(last_day_of_month(9999, 12), Ok(d(9999, 12, 31)));
        assert_eq!(last_day_of_month(1, 1), Ok(d(1, 1, 31)));
        for (y, m) in [(262142, 12), (999999, 1), (2026, 13), (2026, 0)] {
            assert!(last_day_of_month(y, m).is_err(), "{y}-{m}");
        }
    }

    /// A date is exactly YYYY-MM-DD, a real day from 0001-01-01 to
    /// 9999-12-31 (ledger_core::calendar): no sign, no space, no fifth
    /// year digit, no year 0 (P0 review round 3, S1).
    #[test]
    fn dates_are_four_digit_years_and_real_days() {
        assert_eq!(parse_date("2026-02-28"), Ok(d(2026, 2, 28)));
        assert_eq!(parse_date("2028-02-29"), Ok(d(2028, 2, 29)));
        assert_eq!(parse_date("0001-01-01"), Ok(d(1, 1, 1)));
        assert_eq!(parse_date("9999-12-31"), Ok(d(9999, 12, 31)));
        for bad in [
            "0000-01-01",
            "0000-00-00",
            "2026-02-29",
            "2026-02-30",
            "2026-13-01",
            "2026-1-05",
            "2026-01-5",
            " 2026-01-05",
            "2026-01-05 ",
            "+2026-01-05",
            "-0001-01-01",
            "+262142-12-31",
            "-262143-01-01",
            "99999-12-31",
            "2026/01/05",
            "2026-01-05T00:00",
            "２０２６-01-05",
            "",
        ] {
            let err = parse_date(bad).expect_err(bad);
            assert!(err.contains("0001-01-01 to 9999-12-31"), "{bad:?}: {err}");
        }
    }

    /// Settings naming a code are found: cash/financing/investing roles,
    /// close_into, and each member's capital and distributions accounts.
    #[test]
    fn config_mentions_finds_every_setting_naming_a_code() {
        let books = unit_dir("mentions");
        let server = BooksServer::new(store(&books), ReceiptPolicy::default())
            .with_company(members())
            .with_cash_roles(CashRoles {
                cash: vec!["1000".into()],
                financing: vec!["2100".into()],
                investing: vec!["1400".into()],
            });
        assert_eq!(server.config_mentions("1000"), ["[books] cash_accounts"]);
        assert_eq!(
            server.config_mentions("2100"),
            ["[books] financing_accounts"]
        );
        assert_eq!(
            server.config_mentions("1400"),
            ["[books] investing_accounts"]
        );
        assert_eq!(
            server.config_mentions("3000"),
            ["[[company.members]] Ada capital_account"]
        );
        assert_eq!(
            server.config_mentions("3001"),
            ["[[company.members]] Bo capital_account"]
        );
        assert_eq!(
            server.config_mentions("3100"),
            ["[[company.members]] Ada distribution_account"]
        );
        assert!(server.config_mentions("6000").is_empty());
        let books = unit_dir("mentions-solo");
        let solo =
            BooksServer::new(store(&books), ReceiptPolicy::default()).with_company(Company {
                close_into: Some("3000".into()),
                ..Company::default()
            });
        assert_eq!(solo.config_mentions("3000"), ["[books] close_into"]);
    }

    /// Reports name each member with their accounts, in order.
    #[test]
    fn report_members_carry_names_and_accounts() {
        let books = unit_dir("members");
        let server =
            BooksServer::new(store(&books), ReceiptPolicy::default()).with_company(members());
        let m = server.report_members();
        assert_eq!(m.len(), 2);
        assert_eq!(
            (m[0].name.as_str(), m[0].capital_code.as_str()),
            ("Ada", "3000")
        );
        assert_eq!(m[0].distribution_code.as_deref(), Some("3100"));
        assert_eq!(
            (m[1].name.as_str(), m[1].capital_code.as_str()),
            ("Bo", "3001")
        );
        assert_eq!(m[1].distribution_code, None);
    }

    /// The balance sheet's year earnings: net income for the fiscal year
    /// to date, and the part not yet closed into equity (closing entries
    /// included, so a closed month drops out).
    #[test]
    fn year_earnings_split_closed_from_unclosed() {
        let books = unit_dir("earnings");
        let mut s = store(&books);
        let cash = s
            .add_account("1000", "Cash", AccountType::Asset, None, false)
            .unwrap();
        let sales = s
            .add_account("4000", "Sales", AccountType::Revenue, None, false)
            .unwrap();
        let rent = s
            .add_account("6000", "Rent", AccountType::Expense, None, false)
            .unwrap();
        post(&mut s, d(2025, 12, 15), cash, sales, "999.00"); // last fiscal year
        post(&mut s, d(2026, 1, 10), cash, sales, "1000.00");
        post(&mut s, d(2026, 1, 20), rent, cash, "300.00");
        post(&mut s, d(2026, 2, 5), cash, sales, "50.00");
        post(&mut s, d(2026, 2, 6), rent, cash, "20.00");
        let server = BooksServer::new(s, ReceiptPolicy::default());
        let as_of = d(2026, 2, 28);
        {
            let store = server.lock().unwrap();
            let e = server.year_earnings(store.ledger(), as_of).unwrap();
            assert_eq!(e.net_income_ytd, Decimal::from_str("730.00").unwrap());
            assert_eq!(e.unclosed, Decimal::from_str("730.00").unwrap());
        }
        {
            let mut store = server.lock().unwrap();
            let jan = store
                .create_period("2026-01", d(2026, 1, 1), d(2026, 1, 31))
                .unwrap();
            store.close_period(jan).unwrap();
        }
        let store = server.lock().unwrap();
        let e = server.year_earnings(store.ledger(), as_of).unwrap();
        assert_eq!(
            e.net_income_ytd,
            Decimal::from_str("730.00").unwrap(),
            "still the year's income"
        );
        assert_eq!(
            e.unclosed,
            Decimal::from_str("30.00").unwrap(),
            "January's 700 is closed"
        );
    }

    /// Only a closed period moves where the unclosed part starts: an open
    /// one dated before as_of doesn't, and a close that ended before the
    /// fiscal year began leaves the year starting on its first day (last
    /// December's income is never this year's).
    #[test]
    fn year_earnings_count_only_closes_and_never_before_the_year() {
        let books = unit_dir("earnings-open");
        let mut s = store(&books);
        let cash = s
            .add_account("1000", "Cash", AccountType::Asset, None, false)
            .unwrap();
        let sales = s
            .add_account("4000", "Sales", AccountType::Revenue, None, false)
            .unwrap();
        post(&mut s, d(2025, 12, 15), cash, sales, "999.00");
        post(&mut s, d(2026, 1, 10), cash, sales, "1000.00");
        s.create_period("2026-01", d(2026, 1, 1), d(2026, 1, 31))
            .unwrap();
        let server = BooksServer::new(s, ReceiptPolicy::default());
        let as_of = d(2026, 2, 28);
        let thousand = Decimal::from_str("1000.00").unwrap();
        {
            let store = server.lock().unwrap();
            let e = server.year_earnings(store.ledger(), as_of).unwrap();
            assert_eq!(e.unclosed, thousand, "an open January closes nothing");
        }
        {
            let mut store = server.lock().unwrap();
            let to_nov = store
                .create_period("2025-01..11", d(2025, 1, 1), d(2025, 11, 30))
                .unwrap();
            store.close_period(to_nov).unwrap();
        }
        let store = server.lock().unwrap();
        let e = server.year_earnings(store.ledger(), as_of).unwrap();
        assert_eq!(e.net_income_ytd, thousand);
        assert_eq!(e.unclosed, thousand, "December 2025 is not this year's");
    }

    /// Unlinking a receipt is refused only from a posted, non-reversal
    /// entry the policy covers (crate::policy).
    #[test]
    fn receipts_are_kept_only_where_the_policy_needs_them() {
        let books = unit_dir("unlink");
        let mut s = store(&books);
        let cash = s
            .add_account("1000", "Cash", AccountType::Asset, None, false)
            .unwrap();
        let rent = s
            .add_account("6000", "Rent", AccountType::Expense, None, false)
            .unwrap();
        let draft = s.create_entry(d(2026, 3, 1), "draft").unwrap();
        s.add_line(draft, rent, Side::Debit, usd("10.00")).unwrap();
        s.add_line(draft, cash, Side::Credit, usd("10.00")).unwrap();
        let posted = post(&mut s, d(2026, 3, 2), rent, cash, "10.00");
        let reversal = s.reverse_entry(posted, d(2026, 3, 3), "undo").unwrap();
        let every = ReceiptPolicy {
            require_receipts_above: "0.00".into(),
            ..ReceiptPolicy::default()
        };
        let server = BooksServer::new(s, every);
        let store = server.lock().unwrap();
        assert!(
            server.ensure_receipt_not_required(&store, draft).is_ok(),
            "a draft"
        );
        assert!(
            server.ensure_receipt_not_required(&store, reversal).is_ok(),
            "a reversal"
        );
        assert!(
            server.ensure_receipt_not_required(&store, posted).is_err(),
            "a posted entry"
        );
    }

    #[test]
    fn money_always_has_cents_and_never_rounds() {
        let m = |s: &str| money(Decimal::from_str(s).unwrap());
        assert_eq!(money(Decimal::ZERO), "0.00");
        assert_eq!(m("-0"), "0.00");
        assert_eq!(m("1000"), "1000.00");
        assert_eq!(m("5.8"), "5.80");
        assert_eq!(m("-207.68"), "-207.68");
        assert_eq!(m("12.3456"), "12.3456");
    }

    /// Every tool says what it is and what it does to the books (plan
    /// 4.9, "Annotations (P0)"): a title and readOnlyHint on all; a
    /// mutating tool also says whether it is destructive. None asks the
    /// client for an interaction of its own (owner decision 2026-10-16:
    /// no added prompts).
    #[test]
    fn annotations_complete() {
        // Every tool's hints, pinned (MCP's ToolAnnotations): R read-only;
        // W changes the books, additively; D destructive (posts, or
        // overwrites, unlinks, voids, purges what was there: a vault
        // record, a link, a reconciliation); I idempotent (a repeat with
        // the same arguments leaves the journal and the vault as they
        // were: spec_vault's idempotent_tools_change_nothing_when_repeated
        // calls each twice). export_cpa_package writes a file, so it is
        // not read-only.
        type Hints = (Option<bool>, Option<bool>, Option<bool>);
        const R: Hints = (Some(true), None, None);
        const W: Hints = (Some(false), Some(false), None);
        const D: Hints = (Some(false), Some(true), None);
        const WI: Hints = (Some(false), Some(false), Some(true));
        const DI: Hints = (Some(false), Some(true), Some(true));
        let table: [(&str, Hints); 108] = [
            ("books.handbook", R),
            ("books.start", R),
            ("books.set_setting", DI),
            ("books.import_settings", DI),
            ("books.export_settings", R),
            ("books.conventions", R),
            ("books.set_convention", W),
            ("books.retire_convention", D),
            ("books.record_decision", W),
            ("books.decisions", R),
            ("books.raise_item", W),
            ("books.resolve_item", W),
            ("books.open_items", R),
            ("books.draft_from_template", W),
            ("books.approve_entry", W),
            ("books.chart", R),
            ("books.add_account", W),
            ("books.trial_balance", R),
            ("books.profit_and_loss", R),
            ("books.balance_sheet", R),
            ("books.cash_flow", R),
            ("books.draft_entry", W),
            ("books.add_line", W),
            ("books.post_entry", D),
            ("books.reverse_entry", D),
            ("books.attach_receipt", W),
            ("books.file_document", W),
            ("books.link_receipt", W),
            ("books.unlink_receipt", D),
            ("books.withdraw_attachment", D),
            ("books.link_attachment", W),
            ("books.unlink_attachment", D),
            ("books.attachments", R),
            ("books.list_receipts", R),
            ("books.get_receipt", R),
            ("books.get_entry", R),
            ("books.search_entries", R),
            ("books.show", R),
            ("books.verify", WI),
            ("books.digest", R),
            ("books.import_bank_csv", W),
            ("books.create_schedule", W),
            ("books.list_schedules", R),
            ("books.run_schedules", DI),
            ("books.create_recurring", W),
            ("books.list_recurring", R),
            ("books.run_recurring", DI),
            ("books.record_payout", W),
            ("books.list_vendors", R),
            ("books.set_vendor_1099", W),
            ("books.set_entry_vendor", W),
            ("books.vendor_1099", W),
            ("books.chart_template", R),
            ("books.tax_categories", R),
            ("books.renumber_account", D),
            ("books.label_asset", W),
            ("books.correct_asset", D),
            ("books.set_asset_tax_basis", W),
            ("books.asset_labels", R),
            ("books.settings", R),
            ("books.roles", R),
            ("books.assign_role", D),
            ("books.set_tax_category", W),
            ("books.import_chart_csv", W),
            ("books.import_local_file", W),
            ("books.export", R),
            ("books.register_asset", W),
            ("books.asset_schedule", R),
            ("books.list_assets", R),
            ("books.run_depreciation", DI),
            ("books.dispose_asset", D),
            ("books.create_period", W),
            ("books.close_period", D),
            ("books.list_periods", R),
            ("books.mark_cleared", W),
            ("books.reconcile", D),
            ("books.record_statement", W),
            ("books.statements", R),
            ("books.reconciliations", R),
            ("books.void_statement", D),
            ("books.void_reconciliation", D),
            ("books.add_customer", W),
            ("books.add_vendor", W),
            ("books.add_party", W),
            ("books.update_party", W),
            ("books.set_party_roles", WI),
            ("books.set_w9", D),
            ("books.set_tin", D),
            ("books.get_w9", W),
            ("books.purge_tin", D),
            ("books.set_w4", W),
            ("books.parties", R),
            ("books.party", R),
            ("books.set_entry_party", W),
            ("books.create_invoice", D),
            ("books.create_bill", D),
            ("books.receive_payment", D),
            ("books.pay_bill", D),
            ("books.ar_aging", R),
            ("books.ap_aging", R),
            ("books.set_fx_rate", W),
            ("books.run_payroll", D),
            ("books.remit_payroll_taxes", D),
            ("books.list_payroll_runs", R),
            ("books.add_class", W),
            ("books.list_classes", R),
            ("books.profit_and_loss_by_class", R),
            ("books.export_cpa_package", W),
        ];
        let tools = BooksServer::tool_router().list_all();
        assert_eq!(tools.len(), table.len());
        let mut names: Vec<&str> = table.iter().map(|(n, _)| *n).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), table.len(), "a tool is in the table twice");
        for t in &tools {
            let name = &t.name;
            assert!(
                t.title.as_deref().is_some_and(|s| !s.trim().is_empty()),
                "{name} has no title"
            );
            let a = t
                .annotations
                .as_ref()
                .unwrap_or_else(|| panic!("{name} has no annotations"));
            let want = table
                .iter()
                .find(|(n, _)| n == name)
                .unwrap_or_else(|| panic!("{name} is not in the table"))
                .1;
            assert_eq!(
                (a.read_only_hint, a.destructive_hint, a.idempotent_hint),
                want,
                "{name}: (readOnlyHint, destructiveHint, idempotentHint)"
            );
            let json = serde_json::to_string(t).unwrap().to_lowercase();
            for word in ["requiresuserinteraction", "requires_user_interaction"] {
                assert!(!json.contains(word), "{name} asks for user interaction");
            }
            // The books are a closed world: no tool reaches beyond them.
            assert_eq!(
                a.open_world_hint,
                Some(false),
                "{name} has no openWorldHint false"
            );
        }
        // docs/mcp-interface.md names the 28 destructive tools.
        assert_eq!(table.iter().filter(|(_, h)| h.1 == Some(true)).count(), 28);
    }

    /// Every refusal an agent can be given whose text is fixed here, or
    /// in the crates under the server: the error templates of every
    /// variant (scanned from their sources, so a new variant is linted
    /// too), representative values of those whose text is built, and the
    /// server's own refusal constants.
    fn agent_refusals() -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = Vec::new();
        for (krate, src) in [
            (
                "ledger-core",
                include_str!("../../ledger-core/src/error.rs"),
            ),
            (
                "ledger-store",
                include_str!("../../ledger-store/src/error.rs"),
            ),
            (
                "ledger-vault",
                include_str!("../../ledger-vault/src/lib.rs"),
            ),
            (
                "ledger-import bank",
                include_str!("../../ledger-import/src/bank_csv.rs"),
            ),
            (
                "ledger-import chart",
                include_str!("../../ledger-import/src/chart_csv.rs"),
            ),
            (
                "ledger-import iif",
                include_str!("../../ledger-import/src/iif.rs"),
            ),
            (
                "ledger-import pdf",
                include_str!("../../ledger-import/src/pdf.rs"),
            ),
            (
                "ledger-import xlsx",
                include_str!("../../ledger-import/src/xlsx.rs"),
            ),
        ] {
            let mut rest = src;
            let mut n = 0;
            while let Some(at) = rest.find("#[error(") {
                rest = &rest[at + 8..];
                let end = rest.find(")]").expect("an #[error] attribute ends");
                out.push((format!("{krate} error template"), rest[..end].to_string()));
                rest = &rest[end..];
                n += 1;
            }
            assert!(n > 0, "{krate}: no error templates found");
        }
        let newer = |verified| ledger_store::StoreError::NewerJournal {
            seq: 7,
            event_type: "settings.set".into(),
            need: "journal format 2".into(),
            verified,
        };
        out.push(("NewerJournal".into(), newer(true).to_string()));
        out.push(("NewerJournal unverified".into(), newer(false).to_string()));
        out.push((
            "RuleNotKnown".into(),
            ledger_core::form1099::default_threshold(9999)
                .unwrap_err()
                .to_string(),
        ));
        for (what, text) in [
            ("STOPPED", STOPPED),
            ("TIN_FILE_IN_ROOT", TIN_FILE_IN_ROOT),
            ("KEY_IN_BOOKS", KEY_IN_BOOKS),
            ("VAULT_IN_HISTORY", VAULT_IN_HISTORY),
            ("INSIDE_BOOKS", INSIDE_BOOKS),
            ("NO_FOLDER", NO_FOLDER),
            ("EXISTS", EXISTS),
            ("local_files::CHANGED", crate::local_files::CHANGED),
            (
                "local_files::NO_SUCH_FILE",
                crate::local_files::NO_SUCH_FILE,
            ),
            ("local_files::NO_ROOTS", crate::local_files::NO_ROOTS),
            ("local_files::SEALED", crate::local_files::SEALED),
            (
                "local_files::IN_THE_BOOKS",
                crate::local_files::IN_THE_BOOKS,
            ),
            ("local_files::HARD_LINKED", crate::local_files::HARD_LINKED),
            ("local_files::NOT_REGULAR", crate::local_files::NOT_REGULAR),
            ("tin_files::TIN_FILE", tin_files::TIN_FILE),
            ("tin_files::TIN_HELD", tin_files::TIN_HELD),
            ("tin_files::TIN_PROBE", tin_files::TIN_PROBE),
            ("tin_files::TIN_PRINTED", tin_files::TIN_PRINTED),
        ] {
            out.push((what.to_string(), text.to_string()));
        }
        // Refusals and notes built inline, where the books' own record is
        // kept: every string literal of these sources outside their tests.
        for (what, src) in [
            ("tools/kept.rs", include_str!("tools/kept.rs")),
            (
                "ledger-config codec",
                include_str!("../../ledger-config/src/codec.rs"),
            ),
            (
                "ledger-core settings",
                include_str!("../../ledger-core/src/settings.rs"),
            ),
            (
                "ledger-core conventions",
                include_str!("../../ledger-core/src/conventions.rs"),
            ),
            (
                "ledger-core ledger",
                include_str!("../../ledger-core/src/ledger.rs"),
            ),
            ("settings.rs", include_str!("settings.rs")),
        ] {
            let code = src.split("#[cfg(test)]").next().unwrap_or(src);
            let literals = string_literals(code);
            assert!(!literals.is_empty(), "{what}: no literals found");
            out.extend(literals.into_iter().map(|l| (format!("{what} literal"), l)));
        }
        out
    }

    /// The text of every plain string literal ("...") in Rust source,
    /// skipping comments (a rough scanner: enough for the lint).
    fn string_literals(src: &str) -> Vec<String> {
        let mut out = Vec::new();
        for line in src.lines() {
            let line = line.trim_start();
            if line.starts_with("//") {
                continue;
            }
            let mut chars = line.chars().peekable();
            let mut cur: Option<String> = None;
            while let Some(c) = chars.next() {
                match (&mut cur, c) {
                    (None, '"') => cur = Some(String::new()),
                    (None, '/') if chars.peek() == Some(&'/') => break,
                    (Some(s), '\\') => {
                        if let Some(n) = chars.next() {
                            s.push('\\');
                            s.push(n);
                        }
                    }
                    (Some(s), '"') => {
                        out.push(std::mem::take(s));
                        cur = None;
                    }
                    (Some(s), c) => s.push(c),
                    (None, _) => {}
                }
            }
        }
        out
    }

    /// No text an agent is given sends it to the books' files or to tools
    /// outside these (plan I-12): no helper commands, home paths, data
    /// dir names or flags, file names of the books, git, GitHub, pull
    /// requests, repo docs, the old company's examples, or suggestions to
    /// read files. Linted whole: every tool's JSON (title, description,
    /// input and output schemas), every resource, the connect text, every
    /// handbook section, and every refusal fixed in code
    /// ([`agent_refusals`]). (No resource templates are advertised yet.)
    /// What `tool_text_lint` finds in one text beyond its fixed list: a
    /// helper command however it ends (`just` as a word before a
    /// backtick, a line break, a tab or the end), and where the books are
    /// ("beside the books"; "books dir" except in the tools and refusals
    /// that say a file with a TIN is saved outside it, save_to and
    /// save_csv_to).
    fn helper_or_location(what: &str, text: &str) -> Vec<String> {
        let lower = text.to_lowercase();
        let mut found = Vec::new();
        for (at, _) in lower.match_indices("just") {
            let prev = lower[..at].chars().next_back();
            let next = lower[at + 4..].chars().next();
            let word_start = !prev.is_some_and(|c| c.is_alphanumeric() || c == '_');
            if word_start && matches!(next, None | Some('`' | '\n' | '\r' | '\t')) {
                found.push(format!("{what} names a helper command (`just`)"));
            }
        }
        if lower.contains("beside the books") {
            found.push(format!(
                "{what} says where the books are: \"beside the books\""
            ));
        }
        let saves = [
            "books.vendor_1099",
            "books.get_w9",
            "books.export_cpa_package",
        ]
        .iter()
        .any(|t| what == format!("tool {t}"))
            || lower.contains("save_to")
            || lower.contains("save_csv_to");
        if lower.contains("books dir") && !saves {
            found.push(format!("{what} says where the books are: \"books dir\""));
        }
        found
    }

    /// The lint's own checks catch what they name, and pass ordinary words.
    #[test]
    fn tool_text_lint_catches_helpers_and_locations() {
        for (text, n) in [
            ("run `just`", 1),
            ("run just\nthen", 1),
            ("run just\tgui", 1),
            ("run just", 1),
            ("just`gui`", 1),
            ("Just\n", 1),
            ("adjust`", 0),
            ("just after posting", 0),
            ("just, then", 0),
            ("justly", 0),
            ("the file beside the books", 1),
            ("in the books dir", 1),
            ("save_to: a path outside the books dir", 0),
            ("the books directory", 1),
            ("bookshelf", 0),
        ] {
            assert_eq!(helper_or_location("text", text).len(), n, "{text:?}");
        }
        assert!(helper_or_location("tool books.get_w9", "outside the books dir").is_empty());
        assert_eq!(
            helper_or_location("tool books.chart", "outside the books dir").len(),
            1
        );
    }

    #[test]
    fn tool_text_lint() {
        const NEVER: &[&str] = &[
            "just ",
            "~/",
            "/home",
            ".config/wanderbooks",
            "books-data/",
            "data dir",
            "--data-dir",
            "--import-root",
            "vault.key",
            "journal.jsonl",
            ".jsonl",
            "sqlite",
            ".gitignore",
            "github",
            "pull request",
            "docs/",
            ".md",
            "read the journal",
            "the journal is the evidence",
            "read the file",
            "open the file",
            "cat ",
            "fl-0001",
        ];
        // Whole words only: "digit" and "committee" are fine.
        const NEVER_WORDS: &[&str] = &["git", "commit"];
        let mut texts: Vec<(String, String)> = Vec::new();
        for t in BooksServer::tool_router().list_all() {
            texts.push((
                format!("tool {}", t.name),
                serde_json::to_string(&t).unwrap(),
            ));
        }
        for r in resource_list() {
            texts.push((
                format!("resource {}", r.uri),
                serde_json::to_string(&r).unwrap(),
            ));
        }
        texts.extend(agent_refusals());
        for threshold in ["0.00", "25.00"] {
            let settings = Settings {
                policy: ReceiptPolicy {
                    require_receipts_above: threshold.into(),
                    ..ReceiptPolicy::default()
                },
                ..Settings::default()
            };
            texts.push((
                "connect text".into(),
                handbook::connect_text(&settings, "USD"),
            ));
            for (id, _) in handbook::SECTIONS {
                texts.push((
                    format!("handbook {id}"),
                    handbook::section(id, &settings, "USD").unwrap(),
                ));
            }
        }
        let words = |text: &str| {
            text.split(|c: char| !c.is_alphanumeric() && c != '_')
                .map(str::to_string)
                .collect::<Vec<_>>()
        };
        let mut found: Vec<String> = Vec::new();
        for (what, text) in &texts {
            // A handbook resource's own address is not a file name.
            let lower = text.to_lowercase().replace("books://handbook", "");
            for bad in NEVER {
                if let Some(at) = lower.find(bad) {
                    let from = lower[..at]
                        .char_indices()
                        .rev()
                        .nth(40)
                        .map_or(0, |(i, _)| i);
                    let near: String = lower[from..].chars().take(100).collect();
                    found.push(format!("{what} contains {bad:?}: ...{near}..."));
                }
            }
            let w = words(&lower);
            for bad in NEVER_WORDS {
                if w.iter().any(|x| x == bad) {
                    found.push(format!("{what} says {bad:?}"));
                }
            }
        }
        for (what, text) in &texts {
            found.extend(helper_or_location(what, text));
        }
        assert!(found.is_empty(), "{}", found.join("\n"));
        // The settings file may be named in schemas (its fields are edited
        // there until settings move into the books, P1), but no tool's
        // title or description, no refusal, the connect text and no
        // handbook section names it: it is the file books.settings names.
        let mut prose: Vec<(String, String)> = BooksServer::tool_router()
            .list_all()
            .iter()
            .map(|t| {
                (
                    format!("tool {}", t.name),
                    format!(
                        "{} {}",
                        t.title.as_deref().unwrap_or_default(),
                        t.description.as_deref().unwrap_or_default()
                    ),
                )
            })
            .collect();
        prose.extend(
            texts
                .into_iter()
                .filter(|(what, _)| !what.starts_with("tool ")),
        );
        for (what, text) in &prose {
            assert!(
                !text.to_lowercase().contains("wanderbooks.toml"),
                "{what} names the settings file: {text}"
            );
        }
    }

    /// Tool descriptions are at most 700 characters (plan I-12). The
    /// ones over it today are listed, each to be shortened in P5 when its
    /// text moves into the handbook; no new one may join them, and one
    /// that shrinks under the cap must leave the list.
    #[test]
    fn tool_descriptions_fit_700_characters() {
        // Shorten in P5.
        const OVER: &[&str] = &[
            "books.export",
            "books.export_cpa_package",
            "books.import_bank_csv",
            "books.reconcile",
            "books.set_entry_party",
            "books.set_w9",
            "books.show",
            "books.vendor_1099",
        ];
        let mut over: Vec<String> = BooksServer::tool_router()
            .list_all()
            .iter()
            .filter(|t| t.description.as_deref().unwrap_or_default().chars().count() > 700)
            .map(|t| t.name.to_string())
            .collect();
        over.sort();
        assert_eq!(over, OVER, "tool descriptions over 700 characters");
    }

    /// How many masked TINs a text holds.
    fn data_masked(text: &str) -> usize {
        text.matches("•••-••-••••").count() + text.matches("•••••••••").count()
    }

    /// redact_error (B-26): every full TIN in an error's message and data
    /// is masked, wherever the data holds it (a string, in an array, in a
    /// nested object), and what holds none is left as it was.
    #[test]
    fn redact_error_masks_every_tin_in_the_message_and_data() {
        let tin = "291-12-2846";
        let e = redact_error(rmcp::ErrorData::resource_not_found(
            format!("books://receipts/{tin}"),
            Some(serde_json::json!({
                "reason": format!("bad receipt resource URI: {tin}"),
                "list": ["plain", tin, ["deeper 291122846"]],
                "nested": {"uri": tin, "n": 5, "fine": "no tin here 2026-10-16"},
            })),
        ));
        let text = format!("{} {}", e.message, e.data.as_ref().unwrap());
        assert!(!text.contains(tin) && !text.contains("291122846"), "{text}");
        assert_eq!(data_masked(&text), 5, "every TIN masked: {text}");
        let data = e.data.unwrap();
        assert_eq!(data["list"][0], "plain");
        assert_eq!(data["nested"]["n"], 5);
        assert_eq!(data["nested"]["fine"], "no tin here 2026-10-16");
    }

    /// Fail closed (SharedStore INVARIANTS 1-2): a request that panics,
    /// with the store held or not, stops the server, and every later
    /// request, tool or lock, is refused with the plain STOPPED message,
    /// never the panic's text (which could carry anything, a TIN
    /// included).
    #[test]
    fn a_request_that_panics_stops_the_server() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        type Body = fn(BooksServer) -> Result<u64, String>;
        async fn run(
            server: &BooksServer,
            body: Body,
        ) -> Result<Result<u64, String>, rmcp::ErrorData> {
            let s = server.clone();
            run_blocking(&server.store, move || Box::pin(async move { body(s) })).await
        }
        let bodies: [(&str, Body); 2] = [
            ("holding the store", |s| {
                let _store = s.lock()?;
                panic!("mid-mutation 123-45-6789");
            }),
            ("without the store", |_| {
                panic!("before the lock 123-45-6789")
            }),
        ];
        for (how, body) in bodies {
            let books = unit_dir("stops");
            let server = BooksServer::new(store(&books), ReceiptPolicy::default());
            rt.block_on(async {
                let seq: Body = |s| s.lock().map(|st| st.journal_seq());
                assert_eq!(run(&server, seq).await.unwrap(), Ok(1), "{how}");
                let died = run(&server, body).await.unwrap_err();
                assert_eq!(died.message, STOPPED, "{how}");
                // Every later request is refused, before it runs.
                let after = run(&server, seq).await.unwrap_err();
                assert_eq!(after.message, STOPPED, "{how}");
                // So is any session sharing the store (HTTP).
                let other = BooksServer::new_shared(server.store.clone(), ReceiptPolicy::default());
                assert_eq!(run(&other, seq).await.unwrap_err().message, STOPPED);
            });
            // And the lock itself, from any thread.
            let s = server.clone();
            let locked = std::thread::spawn(move || s.lock().map(|_| ()))
                .join()
                .unwrap();
            assert_eq!(locked, Err(STOPPED.to_string()), "{how}");
        }

        // A guard dropped by a panic on a thread of its own stops it too.
        let books = unit_dir("stops-thread");
        let server = BooksServer::new(store(&books), ReceiptPolicy::default());
        let s = server.clone();
        let died = std::thread::spawn(move || {
            let _store = s.lock().unwrap();
            panic!("mid-mutation");
        })
        .join();
        assert!(died.is_err());
        assert_eq!(server.lock().map(|_| ()), Err(STOPPED.to_string()));
    }

    /// run_blocking drives a request to completion even when it waits: a
    /// future that is pending until another thread wakes it completes,
    /// with its own result (the waker unparks the request's thread).
    #[test]
    fn a_request_that_waits_is_woken_and_completes() {
        /// Pending on its first poll, woken from another thread shortly
        /// after; ready on the next.
        struct WakeLater(bool);
        impl std::future::Future for WakeLater {
            type Output = u64;
            fn poll(
                mut self: std::pin::Pin<&mut Self>,
                cx: &mut std::task::Context<'_>,
            ) -> std::task::Poll<u64> {
                if self.0 {
                    return std::task::Poll::Ready(7);
                }
                self.0 = true;
                let waker = cx.waker().clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                    waker.wake();
                });
                std::task::Poll::Pending
            }
        }
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let books = unit_dir("wakes");
        let shared = SharedStore::new(store(&books));
        let got = rt.block_on(async {
            tokio::time::timeout(
                std::time::Duration::from_secs(10),
                run_blocking(&shared, || Box::pin(WakeLater(false))),
            )
            .await
        });
        // A request never woken would hold its thread: don't wait for it.
        rt.shutdown_background();
        assert_eq!(got.ok().and_then(Result::ok), Some(7), "never woken");
    }

    /// A client that connects after the settings changed is told the rule
    /// now in force (plan P0, "get_info calls refresh()"): the connect text
    /// is composed from the settings as they are at initialize, not as
    /// they were when the server started.
    #[test]
    fn a_new_session_is_told_the_settings_in_force() {
        let dir = std::env::temp_dir().join(format!(
            "ledger-mcp-unit-{}-info-settings",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let _cleanup = TestDir(dir.clone());
        let toml = dir.join("wanderbooks.toml");
        let threshold = |t: &str| {
            std::fs::write(
                &toml,
                format!("[company]\nname = \"Info Co\"\n[books]\nreceipt_threshold = \"{t}\"\n"),
            )
            .unwrap();
        };
        threshold("25.00");
        let live = crate::settings::Live::watching(crate::settings::Source {
            books_toml: toml.clone(),
            required: true,
            config_json: dir.join("config.json"),
        })
        .unwrap();
        let books = unit_dir("info");
        let server = BooksServer::new(store(&books), ReceiptPolicy::default())
            .with_live_settings(Arc::new(Mutex::new(live)));
        let told = |s: &BooksServer| rmcp::ServerHandler::get_info(s).instructions.unwrap();
        let before = told(&server);
        assert!(before.contains("more than 25.00 USD in debits"), "{before}");
        threshold("0.00");
        let after = told(&server);
        assert!(
            after.contains("every entry needs at least one receipt"),
            "{after}"
        );
        assert!(!after.contains("25.00"), "{after}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A bare books://digest, or one with an empty since=, covers this
    /// month so far: since is the 1st of today's month, on any day of it
    /// (not today, which only agrees on the 1st). A given since is kept.
    #[test]
    fn a_bare_digest_covers_this_month_so_far() {
        for (day, want) in [(16, "2026-10-01"), (1, "2026-10-01"), (31, "2026-10-01")] {
            let t = d(2026, 10, day);
            for q in ["", "since=", "since=%20", "x=1", "x=1&since="] {
                let q = q.replace("%20", " ");
                assert_eq!(digest_since(&q, t), want, "{q:?} on {t}");
            }
        }
        assert_eq!(
            digest_since("since=2026-01-05", d(2026, 10, 16)),
            "2026-01-05"
        );
        assert_eq!(
            digest_since("x=1&since=2026-01-05", d(2026, 10, 16)),
            "2026-01-05"
        );
        assert_eq!(digest_since("", d(2027, 1, 9)), "2027-01-01");
    }
}
