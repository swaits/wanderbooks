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
//!         echoed, result xor error); DECISION (Stephen, the operating
//!         contract in INSTRUCTIONS and docs/mcp-interface.md).
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
//!   6. A reference (account, vendor, class) given as a number that names
//!      one thing by id and another by code or name is refused, never
//!      guessed.
//!   7. The server MUST answer every request with exactly one response
//!      whose id is the request's, and keep serving after any tool error.
//! ```

use crate::policy::ReceiptPolicy;
use crate::settings::{Live, Settings};
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

/// The agent operating contract, delivered via initialize.
const INSTRUCTIONS: &str = "You are the bookkeeping agent for these books. The engine enforces the accounting; you enforce the workflow.\n\nRULES\n- Money is exact decimal. Always pass amounts as decimal strings (\"123.45\"), never floats.\n- Never post directly. The flow is draft (books.draft_entry) -> add lines (books.add_line) -> preview (books.post_entry without confirmed) -> review the preview -> post (books.post_entry with confirmed=true).\n- Every entry must balance: debits equal credits, exactly. Every line needs a real account from books.chart. The engine rejects anything else.\n- Receipts: any entry moving more than $25.00 in debits needs at least one receipt attached (books.attach_receipt) before posting. The server rejects the post otherwise. This is enforced, not a suggestion.\n- Never invent accounts. If the account you need does not exist, add it with books.add_account first.\n- Corrections go through books.reverse_entry. Posted entries are immutable; there is no edit.\n- Receipt corrections: receipts can change, but nothing is erased. To replace one, attach the new receipt, then books.unlink_receipt the old (with a reason); retire unwanted receipts or documents with books.withdraw_attachment (with a reason). The journal keeps every step.\n- After posting, check books.digest to review what changed, so a human can review too.\n- Amounts in every response are decimal strings. Treat them as exact.\n- Fixed assets: register with books.register_asset, inspect with books.asset_schedule and books.list_assets, post monthly depreciation with books.run_depreciation (exempt from the receipt policy; the schedule is the evidence), and dispose with books.dispose_asset (preview first; the bill of sale is the receipt). Depreciation is straight-line, book basis only - tax depreciation stays with the CPA. Every asset gets a tag for the label on the thing itself (books.register_asset generates the next one, like WL-0001, unless the human has their own); record the manufacturer's serial number too, change either with books.label_asset, and print label sheets with books.asset_labels (save the PDF; print at 100% on Avery 5160/8160). Asset tools take the tag in place of the id. books.export fixed_asset_register is the register for the CPA; books.show asset opens one in the viewer.\n- Prepaid expenses and deferred revenue: record the original payment (with its receipt), then books.create_schedule; books.run_schedules posts the monthly amortization (preview first; exempt from the receipt policy like depreciation). Recurring entries (books.create_recurring, books.run_recurring) only ever create drafts: attach each actual receipt and post through the normal preview.\n- Platform payouts (app stores, payment processors, marketplaces): never book the net deposit as revenue. books.record_payout grosses it up (gross revenue, fees, adjustments) into a draft and refuses unless it reconciles to the cent; attach the payout statement, then preview and post.\n- The viewer: the human usually has the read-only viewer open beside you (`just gui`). When they ask to see something, and after you post, call books.show so it's on their screen.\n- 1099s: mark contractors and landlords with books.set_vendor_1099 (and whether their W-9 is on file; never store a TIN), and tag every payment to them with its vendor (books.draft_entry vendor, or books.set_entry_vendor; books.pay_bill tags automatically). books.vendor_1099 is the year-end worksheet for the CPA.\n- Rules that change with the law: wanderbooks knows some legal amounts (like the 1099 threshold) only through a given year and never guesses past it. When a tool refuses with \"wanderbooks knows the ... only through\", or books.digest lists stale_rules: check today's date, find the current figure from the official source (the refusal names the vendored source file) or ask the human (owner, CPA), confirm it with them, and pass it explicitly. Then update the rule for everyone: open a pull request at https://github.com/swaits/wanderbooks with the new value and its official text (best), or at least an issue; if you can't reach GitHub, give the human the issue text to file.\n- Control roles: invoices, bills, payroll, and closes post to the account filling each role (receivables, revenue, payables, general expense, retained earnings, payroll expense and withholdings); books.roles lists them. A role uses its default code (1200, 4000, 2000, 6999, 3999, 6100, 6110, 2110-2150) until it is assigned one of the chart's own accounts with books.assign_role (journaled). If the chart uses a default code for a different kind of account, the engine refuses and says which. It can't tell an account of the right type but another purpose (a hosting expense at payroll's 6100): check books.roles before running payroll, bills, or invoices. Never assign roles or renumber the human's accounts on your own: explain the conflict, and when they decide, assign the role to their account (best: their numbering stays) or books.renumber_account the account (its history stays).\n- Settings: wanderbooks.toml beside the books holds the company's facts and settings (name, fiscal year, tax form, members and their capital accounts, basis, receipt threshold, cash flow roles, close target); books.settings shows what is in force and lists problems, settings that disagree with the chart. Edit the file when the human tells you a company fact, when a tool refuses naming a wanderbooks.toml setting, when books.settings or books.digest lists a settings problem, and after books.renumber_account lists config_mentions. The server reloads the file whenever it changes, with no restart: after each edit, call books.settings and check reload_error is null (a broken edit leaves the last good settings in force) and problems is empty. Never put a member's SSN or any TIN in it.";

/// Max receipt/document upload: 25 MB.
const MAX_UPLOAD_BYTES: usize = 25 * 1024 * 1024;

/// The server. One store, one receipt policy, shared across tool calls.
/// `Store` is `!Sync`, so access is serialized through a mutex; the server
/// is single-writer by construction.
#[derive(Clone)]
pub struct BooksServer {
    store: Arc<Mutex<Store>>,
    settings: Arc<Mutex<Live>>,
}

#[cfg(test)]
pub use crate::settings::CashRoles;

impl BooksServer {
    /// The wanderbooks.toml settings that name account `code`.
    fn config_mentions(&self, code: &str) -> Vec<String> {
        let mut out = Vec::new();
        for (key, codes) in [
            ("[books] cash_accounts", &self.settings().roles.cash),
            (
                "[books] financing_accounts",
                &self.settings().roles.financing,
            ),
            (
                "[books] investing_accounts",
                &self.settings().roles.investing,
            ),
        ] {
            if codes.iter().any(|c| c == code) {
                out.push(key.to_string());
            }
        }
        if self.settings().company.close_into.as_deref() == Some(code) {
            out.push("[company] close_into".to_string());
        }
        for m in &self.settings().company.members {
            if m.capital_account == code {
                out.push(format!("[[company.members]] {} capital_account", m.name));
            }
            if m.distribution_account.as_deref() == Some(code) {
                out.push(format!(
                    "[[company.members]] {} distribution_account",
                    m.name
                ));
            }
        }
        out
    }

    /// Build the server around an open store.
    pub fn new(store: Store, policy: ReceiptPolicy) -> Self {
        Self::new_shared(Arc::new(Mutex::new(store)), policy)
    }

    /// Build the server around a shared store (for HTTP: each connection
    /// gets a handle to the same store).
    pub fn new_shared(store: Arc<Mutex<Store>>, policy: ReceiptPolicy) -> Self {
        Self {
            store,
            settings: Arc::new(Mutex::new(Live::fixed(Settings {
                policy,
                ..Settings::default()
            }))),
        }
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

    /// The settings in force for this request (reloaded at its start, in
    /// `call_tool` and `read_resource`, if their files changed).
    fn settings(&self) -> Arc<Settings> {
        self.live().current()
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
            return Err(
                "no members configured: add [[company.members]] (name, share, capital_account) to wanderbooks.toml"
                    .to_string(),
            );
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
                .ok_or_else(|| format!("wanderbooks.toml names unknown account {c}"))
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

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Store>, String> {
        self.store
            .lock()
            .map_err(|_| "store lock poisoned".to_string())
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
        let total = ReceiptPolicy::entry_debits(entry);
        Err(ReceiptRequiredError {
            code: "receipt_required".to_string(),
            message: format!(
                "entry {entry_id} moves {} in debits ({}); unlinking its last receipt would leave it without one.",
                money(total),
                self.settings().policy.describe()
            ),
            entry_id: Some(entry_id),
            asset_id: None,
            debits: total.to_string(),
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

fn parse_date(s: &str) -> Result<NaiveDate, String> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map_err(|_| format!("bad date {s:?}: expected YYYY-MM-DD"))
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
fn settings_problems(ledger: &Ledger, cfg: &Settings) -> Vec<String> {
    let mut out = Vec::new();
    let mut check = |setting: &str, code: &str, want: &[AccountType]| {
        match ledger.chart().find_by_code(code) {
            None => out.push(format!(
                "{setting} names {code}, which isn't in the chart: add that account, or change {setting} in wanderbooks.toml"
            )),
            Some(a) if !want.contains(&a.account_type) => out.push(format!(
                "{setting} names {code} \"{}\", a {:?} account, but needs {}: change {setting} in wanderbooks.toml",
                a.name,
                a.account_type,
                want.iter().map(|t| format!("{t:?}")).collect::<Vec<_>>().join(" or "),
            )),
            Some(_) => {}
        }
    };
    use AccountType::*;
    for code in &cfg.roles.cash {
        check("[books] cash_accounts", code, &[Asset]);
    }
    for code in &cfg.roles.financing {
        check("[books] financing_accounts", code, &[Liability, Equity]);
    }
    for code in &cfg.roles.investing {
        check("[books] investing_accounts", code, &[Asset]);
    }
    let c = &cfg.company;
    for m in &c.members {
        check(
            &format!("[[company.members]] {} capital_account", m.name),
            &m.capital_account,
            &[Equity],
        );
        if let Some(d) = &m.distribution_account {
            check(
                &format!("[[company.members]] {} distribution_account", m.name),
                d,
                &[Equity],
            );
        }
    }
    if let Some(code) = &c.close_into {
        check("[books] close_into", code, &[Equity]);
    }
    for code in &c.close_sweep {
        check("[books] close_sweep", code, &[Equity]);
    }
    out
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
    if parts.next().is_some() || year < 1 || !(1..=12).contains(&month) {
        return Err(err());
    }
    Ok((year, month))
}

/// Last calendar day of a month.
fn last_day_of_month(year: i32, month: u32) -> NaiveDate {
    let first = NaiveDate::from_ymd_opt(year, month, 1).expect("valid period");
    first
        .checked_add_months(chrono::Months::new(1))
        .expect("month arithmetic")
        .pred_opt()
        .expect("month arithmetic")
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
            "cash_accounts is required for the cash flow statement (pass it, or set [books] cash_accounts in wanderbooks.toml)"
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

/// Today in the books' local time zone. Not UTC: in the evening (US
/// time) UTC is already tomorrow, and "through today" must not include it.
fn today() -> NaiveDate {
    chrono::Local::now().date_naive()
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
        remaining: money(s.remaining()),
        months: ledger_core::schedules::amortization_plan(s.total.amount, s.start, s.months)
            .into_iter()
            .map(|(d, a)| ScheduleMonthOut {
                date: d.to_string(),
                amount: money(a),
                posted: s.posted.contains(&d),
            })
            .collect(),
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

#[tool_router]
impl BooksServer {
    #[tool(
        name = "books.chart",
        description = "List the chart of accounts with current balances. Start here: every line you post must use a real account from this chart.",
        annotations(read_only_hint = true)
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
        description = "Add an account to the chart. Codes must be unique; check books.chart first."
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
        description = "The trial balance as of a date (default today): every account with a non-zero balance, debit and credit columns and totals. Debits must equal credits.",
        annotations(read_only_hint = true)
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
        description = "Profit and loss for a period: revenue, expenses, net income. Dates are YYYY-MM-DD; omit for all time. Pass class (name or id) to filter to one division.",
        annotations(read_only_hint = true)
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
        description = "The balance sheet as of a date: assets, liabilities, equity (including current earnings). Omit as_of for today.",
        annotations(read_only_hint = true)
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
        description = "Cash flow statement (indirect method) for a period: net income, depreciation add-back, working-capital changes, investing (asset purchases, disposal proceeds), financing. The three sections reconcile to the net change in the given cash accounts.",
        annotations(read_only_hint = true)
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
        description = "Start a draft journal entry. Drafts are journaled and survive restarts, but don't affect reports until posted; add lines with books.add_line."
    )]
    async fn draft_entry(
        &self,
        params: Parameters<DraftEntryParams>,
    ) -> Result<Json<DraftEntryOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let vendor = p
            .vendor
            .as_deref()
            .map(|v| resolve_vendor(&store, v))
            .transpose()?;
        let id = store
            .create_entry(parse_date(&p.date)?, &p.description)
            .map_err(|e| e.to_string())?;
        if vendor.is_some() {
            store
                .set_entry_vendor(id, vendor)
                .map_err(|e| e.to_string())?;
        }
        Ok(Json(DraftEntryOut {
            entry_id: id,
            status: "draft".to_string(),
        }))
    }

    #[tool(
        name = "books.add_line",
        description = "Add one line to a draft entry. Amount is a decimal string, never a float. The entry must balance before posting. For foreign-currency lines, pass foreign_currency, foreign_amount, and fx_rate (foreign -> base); the base amount is computed as foreign_amount * fx_rate. Pass class (name or id) to tag the line for divisional tracking."
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
        description = "Post a draft entry. Without confirmed=true this returns a preview (lines, resulting balances, receipt checklist) and posts nothing. With confirmed=true it posts after enforcing the receipt policy."
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
        let total = ReceiptPolicy::entry_debits(&entry);
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
        };
        if !p.confirmed.unwrap_or(false) {
            return Ok(Json(out));
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
        let mut posted = out;
        posted.status = "posted".to_string();
        Ok(Json(posted))
    }

    #[tool(
        name = "books.reverse_entry",
        description = "Reverse a posted entry with an offsetting entry. Without confirmed=true returns a preview; reversals are exempt from the receipt policy."
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
        description = "Attach a receipt (base64 file bytes) to an entry, or file it unlinked and link later with books.link_receipt. Files are content-addressed and deduplicated."
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
        let (attachment_id, sha256, deduplicated) = store
            .attach_receipt(
                p.entry_id,
                &p.file_name,
                &p.mime_type,
                &bytes,
                p.description.as_deref(),
            )
            .map_err(|e| e.to_string())?;
        Ok(Json(AttachReceiptOut {
            attachment_id,
            sha256,
            deduplicated,
        }))
    }

    #[tool(
        name = "books.file_document",
        description = "File a formation, legal, tax, or accounting document into the content-addressed store."
    )]
    async fn file_document(
        &self,
        params: Parameters<FileDocumentParams>,
    ) -> Result<Json<AttachReceiptOut>, String> {
        let p = params.0;
        match p.category.as_str() {
            "formation" | "legal" | "tax" | "accounting" | "other" => {}
            _ => {
                return Err(format!(
                    "bad category {:?}: expected formation, legal, tax, accounting, or other",
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
        let (attachment_id, sha256, deduplicated) = store
            .file_document(
                &p.category,
                &p.file_name,
                &p.mime_type,
                &bytes,
                p.description.as_deref(),
            )
            .map_err(|e| e.to_string())?;
        Ok(Json(AttachReceiptOut {
            attachment_id,
            sha256,
            deduplicated,
        }))
    }

    #[tool(
        name = "books.link_receipt",
        description = "Link a previously filed (unlinked) receipt to an entry."
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
        description = "Detach a receipt from its entry, with a reason recorded in the journal. The receipt stays filed (unlinked); link it elsewhere or withdraw it with books.withdraw_attachment. The receipt policy still holds: an entry that requires a receipt cannot be left with none, so to replace a receipt attach the new one first, then unlink the old."
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
        description = "Withdraw an unlinked receipt or a filed document, with a reason recorded in the journal. It drops out of books.list_receipts and can never be linked again. Nothing is deleted: the bytes stay in the blob store and the journal keeps the filing and the withdrawal. Unlink a linked receipt first."
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

    #[tool(
        name = "books.list_receipts",
        description = "List receipts and filed documents, optionally filtered to one entry. Withdrawn attachments are hidden unless include_withdrawn=true.",
        annotations(read_only_hint = true)
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
            .map(|a| ReceiptOut {
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
            .collect();
        Ok(Json(ListReceiptsOut { receipts }))
    }

    #[tool(
        name = "books.get_receipt",
        description = "Fetch a receipt or document's metadata and base64 bytes. Bytes are hash-verified on read.",
        annotations(read_only_hint = true)
    )]
    async fn get_receipt(
        &self,
        params: Parameters<GetReceiptParams>,
    ) -> Result<Json<GetReceiptOut>, String> {
        let p = params.0;
        let store = self.lock()?;
        let meta = store
            .list_attachments(None)
            .map_err(|e| e.to_string())?
            .into_iter()
            .find(|a| a.id == p.attachment_id)
            .ok_or_else(|| format!("unknown attachment {}", p.attachment_id))?;
        let bytes = store
            .attachment_bytes(p.attachment_id)
            .map_err(|e| e.to_string())?;
        Ok(Json(GetReceiptOut {
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
        description = "Full detail of one entry: lines, totals, status, and attached receipts.",
        annotations(read_only_hint = true)
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
                let a = store.ledger().chart().get(l.account).unwrap();
                EntryLineOut {
                    account_id: l.account,
                    code: a.code.clone(),
                    name: a.name.clone(),
                    side: match l.side {
                        Side::Debit => "debit".to_string(),
                        Side::Credit => "credit".to_string(),
                    },
                    amount: money(l.amount.amount),
                }
            })
            .collect();
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
            total: money(ReceiptPolicy::entry_debits(&entry)),
            receipts: preview_receipts(&store, p.entry_id)?,
            vendor: store
                .ledger()
                .entry_vendor(p.entry_id)
                .and_then(|v| store.ledger().get_vendor(v))
                .map(|v| v.name.clone()),
        }))
    }

    #[tool(
        name = "books.search_entries",
        description = "Search posted entries by account, date range, and description text. Newest first.",
        annotations(read_only_hint = true)
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
            let total = ReceiptPolicy::entry_debits(e);
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
        description = "Show something in the human's viewer (the read-only desktop app, `just gui`): an entry (with its lines, receipt, and history), an account's register (optionally for start/end dates), a report (optionally for start/end dates, split into month/quarter/year columns, compared with the prior period or prior year), a stored receipt or document, a fixed asset (by tag or id), a search of the transactions, or a page. Give exactly one target; work out dates yourself (\"last quarter\" means its first and last day). Use it whenever the human asks to see something, and after posting so they can look at what you did. Changes nothing in the books.",
        annotations(read_only_hint = true)
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
            p.search.is_some(),
            p.page.is_some(),
        ]
        .iter()
        .filter(|x| **x)
        .count();
        if given != 1 {
            return Err(
                "give exactly one of entry_id, account, report, attachment_id, asset, search, page"
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
        const PAGES: [&str; 10] = [
            "overview",
            "cash",
            "profit",
            "transactions",
            "reports",
            "accounts",
            "assets",
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
                "Sent to the viewer: {shown}. If it isn't open, the human can open it with `just gui` (within a couple of minutes, it opens right there)."
            ),
        }))
    }

    #[tool(
        name = "books.verify",
        description = "Check the books end to end: the journal's hash chain; a fresh replay against the live ledger; every table of the SQLite database, row for row, against a fresh projection of the journal (rebuilding the database from the journal if it drifted, keeping the old file); and, by default, every stored receipt and document against its SHA-256. Run it whenever a number looks wrong or a human asks whether the books are intact."
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
        description = "What changed since a date: entries posted, per-account movement, and receipt coverage. Built for human review.",
        annotations(read_only_hint = true)
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
                total: money(ReceiptPolicy::entry_debits(e)),
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
                catalog: r.catalog.to_string(),
                source: r.source.to_string(),
                why: r.review_why.to_string(),
            })
            .collect();
        let journal_repairs = store
            .tail_repairs()
            .map_err(|e| e.to_string())?
            .into_iter()
            .filter(|(at, _, _)| at.get(..10).is_some_and(|d| d >= p.since.as_str()))
            .map(|(at, bytes, saved_as)| JournalRepair {
                at,
                bytes,
                saved_as,
            })
            .collect();
        Ok(Json(DigestOut {
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
                        format!("wanderbooks.toml didn't load (the last good settings are in force): {e}"),
                    );
                }
                problems
            },
        }))
    }

    #[tool(
        name = "books.import_bank_csv",
        description = "Import a QuickBooks-style bank/credit-card CSV: 3 columns Date,Description,Amount (payments negative), or 4 columns Date,Description, money out, money in (Intuit heads them Credit,Debit). Slash dates must settle month-first or day-first from the file itself or from date_order (\"month_first\" for mm/dd, \"day_first\" for dd/mm); ask the human when the tool says the dates read either way. Creates one DRAFT entry per transaction, all or none: bank account on one side, categorized offset on the other (the longest matching rule pattern, else default_offset_code). Review with books.digest, then post."
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
        let count = imported.len();
        Ok(Json(ImportBankCsvOut { imported, count }))
    }

    #[tool(
        name = "books.create_schedule",
        description = "Spread a prepaid expense (asset expensed monthly, e.g. a year of insurance paid up front) or deferred revenue (liability earned monthly, e.g. an annual subscription collected up front) evenly over the months it covers, to the cent (the last month absorbs rounding). Record the original payment first (Dr prepaid / Cr cash, with its receipt; or Dr cash / Cr deferred revenue). Returns the monthly plan; post it with books.run_schedules."
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
        description = "Amortization schedules (prepaid expenses, deferred revenue): each month's amount, what has posted, and what remains.",
        annotations(read_only_hint = true)
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
        description = "Post every amortization schedule's months ending on or before `through` (default today) that haven't posted. Idempotent: a month posts once. Without confirmed=true, returns the postings it would make. Exempt from the receipt policy: the schedule is the evidence (the original payment carries the receipt)."
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
        let mut due: Vec<(NaiveDate, u64, Decimal)> = store
            .ledger()
            .schedules()
            .flat_map(|s| s.due(through).into_iter().map(move |(d, a)| (d, s.id, a)))
            .collect();
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
        description = "A recurring entry (rent, a subscription, a loan payment): balanced lines and a frequency (weekly, monthly, quarterly, yearly) from a start date, optionally ending. books.run_recurring turns due occurrences into drafts, never posted entries: attach each actual receipt and post through the normal preview."
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
        description = "Recurring entries and the occurrences generated so far.",
        annotations(read_only_hint = true)
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
        description = "Turn every recurring entry's occurrences on or before `through` (default today) into drafts. Idempotent: an occurrence generates once. Occurrences dated in a closed period are not generated; they come back in `skipped_closed_period` for the human to decide. Each draft then goes through the normal flow: attach the receipt, preview, post."
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
        description = "Record a platform payout (app store, payment processor, marketplace) grossed up: Dr the deposit account the net deposit, Dr fee accounts the fees withheld, Dr/Cr adjustments (refunds, chargebacks, reserves, taxes withheld), Cr revenue the gross sales (several lines and classes allowed). Refuses unless gross - fees +/- adjustments equals the deposit to the cent, and says by how much it's off. Creates a draft: attach the payout statement as its receipt, preview, and post."
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
        description = "Vendors, with which 1099 each gets and whether its W-9 is on file.",
        annotations(read_only_hint = true)
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
                })
                .collect(),
        }))
    }

    #[tool(
        name = "books.set_vendor_1099",
        description = "Mark which information return a vendor gets: \"nec\" (1099-NEC: contractors, freelancers, attorneys), \"misc\" (1099-MISC: rent, royalties, prizes), or \"none\" (corporations generally, merchants, anyone paid only by card). Record whether a W-9 is on file; never store the TIN itself. Ask the human when unsure; the CPA confirms."
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
        }))
    }

    #[tool(
        name = "books.set_entry_vendor",
        description = "Tag an entry (draft or posted) with the vendor it paid, for 1099 tracking, or omit vendor to clear the tag. Reporting metadata only: amounts never change. books.pay_bill tags its payments automatically; books.draft_entry takes a vendor too."
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
        description = "1099 worksheet for a calendar year: cash paid to each vendor (credits minus debits to asset accounts on entries tagged with the vendor, so card payments, which the card processor reports on 1099-K, are excluded), which vendors are reportable (marked NEC/MISC and paid at least the threshold: $600 through 2025, $2,000 in 2026, indexed for inflation after that), and which are missing a W-9. For a year whose threshold wanderbooks doesn't know yet it refuses rather than guess: get the figure from the IRS or the human, pass it as `threshold`, and update the rule for everyone (pull request or issue at github.com/swaits/wanderbooks). For the CPA to confirm.",
        annotations(read_only_hint = true)
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
        let r = ledger_core::vendor_1099_report(store.ledger(), p.year, threshold)
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
        Ok(Json(Vendor1099Out {
            year: r.year,
            threshold: money(r.threshold),
            vendors: r
                .vendors
                .into_iter()
                .map(|v| Vendor1099LineOut {
                    vendor_id: v.vendor_id,
                    name: v.name,
                    form_1099: form_1099_label(v.form),
                    tin_on_file: v.tin_on_file,
                    paid: money(v.paid),
                    reportable: v.reportable,
                })
                .collect(),
            missing_w9,
            notes,
        }))
    }

    #[tool(
        name = "books.chart_template",
        description = "A starter chart of accounts for a kind of business (sole proprietorship, partnership or multi-member LLC, S corporation, C corporation): the right equity and owner-pay accounts (draws; per-partner capital and distributions plus guaranteed payments; officer compensation and distributions; dividends and income tax), common income and expense accounts with tax categories assigned, and the wanderbooks.toml settings that go with it. A proposal: review it with the human, adjust, then books.import_chart_csv.",
        annotations(read_only_hint = true)
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
        description = "The tax categories an account can map to (where it lands on the income tax return), filtered to the business's return when wanderbooks.toml says which (tax_form or tax_classification). Each has a default deductible percent (meals 50, nondeductible 0) and guidance. Categories are stable keys, not form line numbers.",
        annotations(read_only_hint = true)
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
        description = "Give an account a new code. Only the code changes: its id, name, and every entry posted to it stay, so balances and reports don't move. Refuses a code in use, or a control account's code (invoices, bills, payroll, closes) for an account of the wrong type. Lists any wanderbooks.toml settings that still name the old code. Never renumber the human's accounts on your own: only when they ask."
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
            config_mentions: self.config_mentions(&previous_code),
            previous_code,
            code: p.code,
        }))
    }

    #[tool(
        name = "books.label_asset",
        description = "Change a fixed asset's description, link it to the entry that acquired it (acquisition_entry), or set or change its tag (the label on the thing: letters, digits, - _ . /, at most 32; unique ignoring case and never reused by another asset, even after a retag or disposal) and its manufacturer serial number. The asset is its id or current tag. Journaled. Print labels with books.asset_labels."
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
        name = "books.asset_labels",
        description = "A printable sheet of asset labels (PDF, base64): each label shows the company, the tag, a QR code of the tag, the description, and the serial number. Avery 5160/8160 (US letter, 30 labels of 1\" x 2-5/8\"); skip reuses a partly used sheet. Omit assets for every active asset with a tag. Save the file and tell the human to print at 100% scale.",
        annotations(read_only_hint = true)
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
        description = "The settings in force from wanderbooks.toml (company, fiscal year, tax form, members, basis, receipt rule, cash flow roles, close target), the file they come from, and problems: settings that disagree with the chart, each saying what to change. The server reloads the file whenever it changes, with no restart; after editing it, call this to confirm the edit loaded (reload_error says why one didn't, and the last good settings stay in force).",
        annotations(read_only_hint = true)
    )]
    async fn settings_tool(&self) -> Result<Json<SettingsOut>, String> {
        let status = self.live().status();
        let cfg = self.settings();
        let store = self.lock()?;
        let c = &cfg.company;
        Ok(Json(SettingsOut {
            books_toml: status.books_toml.map(|p| p.display().to_string()),
            books_toml_exists: status.books_toml_exists,
            config_json: status.config_json.map(|p| p.display().to_string()),
            reload_error: status.error,
            loads: status.loads,
            company: c.name.clone(),
            fiscal_year_start: format!("{:02}-{:02}", c.fiscal_year_start.0, c.fiscal_year_start.1),
            tax_form: c.tax_form.map(|f| f.label().to_string()),
            basis: c.basis.clone(),
            receipt_rule: cfg.policy.rule(),
            members: c
                .members
                .iter()
                .map(|m| MemberSettingOut {
                    name: m.name.clone(),
                    share: m.share.normalize().to_string(),
                    capital_account: m.capital_account.clone(),
                    distribution_account: m.distribution_account.clone(),
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
        description = "The control roles invoices, bills, payroll, and closes post to (receivables, revenue, payables, general expense, retained earnings, payroll expense and withholdings), and the account filling each: the one assigned with books.assign_role, else the account at the role's default code.",
        annotations(read_only_hint = true)
    )]
    async fn roles(&self) -> Result<Json<RolesOut>, String> {
        let store = self.lock()?;
        Ok(Json(RolesOut {
            roles: roles_out(store.ledger()),
        }))
    }

    #[tool(
        name = "books.assign_role",
        description = "Assign a control role (see books.roles) one of the chart's own accounts, so invoices, bills, payroll, or closes post there instead of the default code; account null returns the role to its default code. Journaled. Refuses an account of the wrong type, a placeholder, one already filling another role, and moving a receivables, payables, withholding, or retained earnings role while its account holds a balance (move the balance with an entry first). Posted entries never change. Only when the human agrees."
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
        description = "Map a revenue or expense account to a tax category (see books.tax_categories), optionally overriding its deductible percent; category null clears it. Drives the tax_summary report."
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
        description = "Import a chart of accounts from CSV (header: code,name,type,parent_code). Types: asset, liability, equity, revenue, expense. Parents must come before children. Fails on the first bad row; nothing is half-imported."
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
        description = "Import a file from the server's local filesystem into the blob store (receipt or document). Used for backfills: the agent downloads from Google Drive (or elsewhere) to a temp path, then calls this. Path must be absolute."
    )]
    async fn import_local_file(
        &self,
        params: Parameters<ImportLocalFileParams>,
    ) -> Result<Json<ImportLocalFileOut>, String> {
        let p = params.0;
        let path = std::path::Path::new(&p.path);
        if !path.is_absolute() {
            return Err("path must be absolute".to_string());
        }
        let bytes = std::fs::read(path).map_err(|e| format!("read {}: {e}", p.path))?;
        if bytes.len() > 25 * 1024 * 1024 {
            return Err("file exceeds 25 MB limit".to_string());
        }
        let file_name = p.file_name.unwrap_or_else(|| {
            path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("file")
                .to_string()
        });
        let mime_type = mime_guess::from_path(path)
            .first_or_octet_stream()
            .to_string();
        let mut store = self.lock()?;
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
        description = "Export a report. Reports: profit_and_loss, balance_sheet, cash_flow (needs cash_accounts), trial_balance (as_of), members_equity (multi-member LLC; members from wanderbooks.toml), general_ledger (by account: running balances, splits), general_journal (every entry in date order; alias journal), fixed_asset_register (as_of: every asset with tag, serial, cost, depreciation, book value, reconciled to the books). Formats: pdf and xlsx (base64) for the five statements and fixed_asset_register; xlsx, csv, json for every report; iif for general_journal (QuickBooks Desktop). Dates: start/end for period reports (start defaults to the first entry, end to today), as_of for balance_sheet, trial_balance, and fixed_asset_register.",
        annotations(read_only_hint = true)
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
            let r = ledger_core::fixed_asset_register(ledger, as_of);
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
            let r = ledger_core::fixed_asset_register(store.ledger(), as_of);
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
        description = "Register a fixed asset (equipment, vehicles, buildings). Links three accounts: cost (asset), accumulated depreciation (asset, contra), depreciation expense. Depreciation is straight-line, book basis only — tax depreciation (MACRS, Section 179) stays with the CPA."
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
        Ok(Json(RegisterAssetOut { asset_id, tag }))
    }

    #[tool(
        name = "books.asset_schedule",
        description = "Month-by-month straight-line depreciation schedule for an asset: period, amount, accumulated, book value. Partial first and last months are prorated by days; the lifetime total is exactly cost minus salvage.",
        annotations(read_only_hint = true)
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
        description = "List all registered fixed assets with accumulated depreciation and book value as of today.",
        annotations(read_only_hint = true)
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
        description = "Post one month of depreciation (Dr Depreciation Expense / Cr Accumulated Depreciation) for one asset, or for every active asset when asset_id is omitted. Idempotent: re-running a posted period returns the original entry instead of double-posting. Depreciation is an allocation, not money movement, so it is exempt from the receipt policy; the registered asset and its schedule are the evidence."
    )]
    async fn run_depreciation(
        &self,
        params: Parameters<RunDepreciationParams>,
    ) -> Result<Json<RunDepreciationOut>, String> {
        let p = params.0;
        let (py, pm) = parse_period_mcp(&p.period)?;
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
                let period_end = last_day_of_month(py, pm);
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
        description = "Dispose of a fixed asset (sell or retire). Without confirmed=true returns a preview — book value, gain/loss, the auto-posted final depreciation, and the disposal entry's lines — and posts nothing. With confirmed=true posts the disposal entry (plus any missing final-period depreciation) after enforcing the receipt policy: file the bill of sale with books.file_document first and pass its attachment ids as receipt_attachment_ids."
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
        description = "Create an accounting period (month, quarter, year). Dates are YYYY-MM-DD, inclusive."
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
        description = "Close an accounting period. The closing entry, dated at period end, zeroes revenue and expense balances as of that date into Retained Earnings, or, for a multi-member LLC (members in wanderbooks.toml), into each member's capital account by share. Periods close in order. Without confirmed=true, returns the exact closing entry it would post and closes nothing. With confirmed=true, posts it and marks the period closed. Closed periods reject new entries."
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
        description = "List all accounting periods with their status."
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
        description = "Mark a posted entry's line as cleared (or uncleared) for bank reconciliation. Only posted lines can be marked."
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

    #[tool(
        name = "books.reconcile",
        description = "Reconcile a bank/cash account against a statement. Returns book balance, cleared balance, statement balance, the difference (zero means reconciled), and the list of uncleared lines."
    )]
    async fn reconcile(
        &self,
        params: Parameters<ReconcileParams>,
    ) -> Result<Json<ReconcileOut>, String> {
        let p = params.0;
        let store = self.lock()?;
        let account = resolve_account(&store, &p.account)?;
        let report = store
            .ledger()
            .reconciliation_report(
                account,
                parse_date(&p.statement_date)?,
                parse_amount(&p.statement_balance)?,
            )
            .map_err(|e| e.to_string())?;
        let code = store
            .ledger()
            .chart()
            .get(account)
            .map(|a| a.code.clone())
            .unwrap_or_default();
        let uncleared = report
            .uncleared_lines
            .into_iter()
            .map(|l| UnclearedLineOut {
                entry_id: l.entry_id,
                line_index: l.line_index,
                date: l.date.to_string(),
                description: l.description,
                side: match l.side {
                    Side::Debit => "debit".to_string(),
                    Side::Credit => "credit".to_string(),
                },
                amount: l.amount.to_string(),
            })
            .collect();
        Ok(Json(ReconcileOut {
            account_code: code,
            book_balance: report.book_balance.to_string(),
            cleared_balance: report.cleared_balance.to_string(),
            statement_balance: report.statement_balance.to_string(),
            difference: report.difference.to_string(),
            uncleared_lines: uncleared,
        }))
    }

    // ------------------------------------------------------------------
    // A/R and A/P
    // ------------------------------------------------------------------

    #[tool(
        name = "books.add_customer",
        description = "Add a customer for accounts receivable."
    )]
    async fn add_customer(
        &self,
        params: Parameters<AddPartyParams>,
    ) -> Result<Json<AddPartyOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let id = store.add_customer(&p.name).map_err(|e| e.to_string())?;
        Ok(Json(AddPartyOut { id }))
    }

    #[tool(
        name = "books.add_vendor",
        description = "Add a vendor for accounts payable."
    )]
    async fn add_vendor(
        &self,
        params: Parameters<AddPartyParams>,
    ) -> Result<Json<AddPartyOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let id = store.add_vendor(&p.name).map_err(|e| e.to_string())?;
        Ok(Json(AddPartyOut { id }))
    }

    #[tool(
        name = "books.create_invoice",
        description = "Create a sales invoice. Auto-posts Dr Accounts Receivable / Cr Revenue. Lines are {description, amount} with decimal-string amounts."
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
            total: inv.total().to_string(),
            entry_id: inv.entry_id,
        }))
    }

    #[tool(
        name = "books.create_bill",
        description = "Create a vendor bill. Auto-posts Dr General Expenses / Cr Accounts Payable. Lines are {description, amount} with decimal-string amounts."
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
            total: bill.total().to_string(),
            entry_id: bill.entry_id,
        }))
    }

    #[tool(
        name = "books.receive_payment",
        description = "Record a customer payment against an invoice. Posts Dr Cash / Cr A/R."
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
            remaining: inv.balance().to_string(),
            status: match inv.status {
                DocumentStatus::Open => "open".to_string(),
                DocumentStatus::Partial => "partial".to_string(),
                DocumentStatus::Paid => "paid".to_string(),
            },
        }))
    }

    #[tool(
        name = "books.pay_bill",
        description = "Pay a vendor bill. Posts Dr A/P / Cr Cash."
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
            remaining: bill.balance().to_string(),
            status: match bill.status {
                DocumentStatus::Open => "open".to_string(),
                DocumentStatus::Partial => "partial".to_string(),
                DocumentStatus::Paid => "paid".to_string(),
            },
        }))
    }

    #[tool(
        name = "books.ar_aging",
        description = "A/R aging report as of a date: outstanding balances per customer, bucketed by days past due (current, 30, 60, 90, 90+)."
    )]
    async fn ar_aging(&self, params: Parameters<AgingParams>) -> Result<Json<AgingOut>, String> {
        let p = params.0;
        let store = self.lock()?;
        let rows = store
            .ledger()
            .ar_aging(parse_date(&p.as_of)?)
            .into_iter()
            .map(|r| {
                let total = r.total().to_string();
                AgingRowOut {
                    name: r.name,
                    current: money(r.current),
                    days_30: money(r.days_30),
                    days_60: money(r.days_60),
                    days_90: money(r.days_90),
                    days_90_plus: money(r.days_90_plus),
                    total,
                }
            })
            .collect();
        Ok(Json(AgingOut { rows }))
    }

    #[tool(
        name = "books.ap_aging",
        description = "A/P aging report as of a date: outstanding balances per vendor, bucketed by days past due (current, 30, 60, 90, 90+)."
    )]
    async fn ap_aging(&self, params: Parameters<AgingParams>) -> Result<Json<AgingOut>, String> {
        let p = params.0;
        let store = self.lock()?;
        let rows = store
            .ledger()
            .ap_aging(parse_date(&p.as_of)?)
            .into_iter()
            .map(|r| {
                let total = r.total().to_string();
                AgingRowOut {
                    name: r.name,
                    current: money(r.current),
                    days_30: money(r.days_30),
                    days_60: money(r.days_60),
                    days_90: money(r.days_90),
                    days_90_plus: money(r.days_90_plus),
                    total,
                }
            })
            .collect();
        Ok(Json(AgingOut { rows }))
    }

    // ------------------------------------------------------------------
    // Foreign exchange
    // ------------------------------------------------------------------

    #[tool(
        name = "books.set_fx_rate",
        description = "Record a historical FX rate: how much base currency one unit of the foreign currency buys on the given date. For reference; lines store their own rate at entry time."
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
        description = "Run payroll for a pay date. Each line carries one employee's gross pay and every withholding/deduction as decimal strings — the engine cuts one balanced entry (Dr Salary & Wage Expense, Dr Payroll Tax Expense for the employer share, Cr the withholding payables, Cr cash for net pay). Auto-creates the 61xx/21xx accounts. All amounts are decimal strings, never floats."
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
                Ok(ledger_core::PayrollLine {
                    employee_name: l.employee_name,
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
        let run_id = store
            .run_payroll(parse_date(&p.date)?, lines, cash)
            .map_err(|e| e.to_string())?;
        let run = store
            .ledger()
            .get_payroll_run(run_id)
            .ok_or_else(|| format!("payroll run {run_id} not found after creation"))?;
        let total_gross: Decimal = run.lines.iter().map(|l| l.gross_pay).sum();
        let total_net: Decimal = run.lines.iter().map(|l| l.net_pay()).sum();
        Ok(Json(RunPayrollOut {
            run_id,
            entry_id: run.entry_id,
            total_gross: money(total_gross),
            total_net: money(total_net),
        }))
    }

    #[tool(
        name = "books.remit_payroll_taxes",
        description = "Remit all outstanding payroll tax payables (federal/state withholding, SS, Medicare, other deductions) to cash in one balanced entry."
    )]
    async fn remit_payroll_taxes(
        &self,
        params: Parameters<RemitPayrollTaxesParams>,
    ) -> Result<Json<RemitPayrollTaxesOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let cash = resolve_account(&store, &p.cash_account)?;
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
        Ok(Json(RemitPayrollTaxesOut {
            entry_id,
            total: money(total),
        }))
    }

    #[tool(
        name = "books.list_payroll_runs",
        description = "List payroll runs in id order: date, employee count, totals, and the posted entry id.",
        annotations(read_only_hint = true)
    )]
    async fn list_payroll_runs(&self) -> Result<Json<ListPayrollRunsOut>, String> {
        let store = self.lock()?;
        let runs = store
            .list_payroll_runs()
            .into_iter()
            .map(|r| {
                let total_gross: Decimal = r.lines.iter().map(|l| l.gross_pay).sum();
                PayrollRunOut {
                    id: r.id,
                    date: r.date.to_string(),
                    employee_count: r.lines.len(),
                    total_gross: money(total_gross),
                    entry_id: r.entry_id,
                }
            })
            .collect();
        Ok(Json(ListPayrollRunsOut { runs }))
    }

    // ------------------------------------------------------------------
    // Classes (divisional tracking)
    // ------------------------------------------------------------------

    #[tool(
        name = "books.add_class",
        description = "Add a tracking class for divisional P&L (e.g. an app, website, or channel). Tag journal lines with it via books.add_line's class param, then report per division."
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
        description = "List tracking classes in id order.",
        annotations(read_only_hint = true)
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
        description = "P&L broken down by tracking class for a period, plus an unclassified row for untagged lines. Dates are YYYY-MM-DD; omit for all time.",
        annotations(read_only_hint = true)
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

    #[tool(
        name = "books.export_cpa_package",
        description = "Build the CPA handoff package as a zip (base64): the profit and loss, balance sheet, cash flow, statement of changes in members' equity (multi-member LLCs), tax summary, and trial balance, each as PDF and XLSX; the fixed asset register as PDF and XLSX when there are fixed assets; the general ledger (by account) and general journal (entry by entry) as XLSX; the journal as IIF (imports into QuickBooks Desktop); the 1099 worksheet as CSV; every receipt behind the year's entries (receipts/, with an index.csv tying each file to its entry) and the company's documents (documents/), unless include_receipts is false; and a README with the journal's verified hash. cash_accounts (e.g. [\"1000\"]) is required: the cash flow statement reconciles to them. as_of defaults to today; period reports cover the fiscal year to date.",
        annotations(read_only_hint = true)
    )]
    async fn export_cpa_package(
        &self,
        params: Parameters<ExportCpaPackageParams>,
    ) -> Result<Json<ExportCpaPackageOut>, String> {
        use ledger_import::{XlsxReport, report, report_to_xlsx, statement_pdf};
        let p = params.0;
        let as_of = match p.as_of {
            Some(s) => parse_date(&s)?,
            None => today(),
        };
        let store = self.lock()?;
        let meta = self.report_meta(&store);
        let ledger = store.ledger();

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
        if !self.settings().company.members.is_empty() {
            let me = ledger_core::members_equity(ledger, fy, as_of, &self.member_capitals(ledger)?)
                .map_err(|e| e.to_string())?;
            statements.push(("members-equity", report::members_equity(&me)));
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
            let r = ledger_core::fixed_asset_register(ledger, as_of);
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
        match ledger_core::vendor_1099_report(ledger, year, None) {
            Ok(r1099) => {
                let mut w = String::from("vendor,form,w9_on_file,paid,reportable\n");
                for v in &r1099.vendors {
                    w.push_str(&format!(
                        "\"{}\",{},{},{},{}\n",
                        v.name.replace('"', "\"\""),
                        v.form.map(|f| f.label()).unwrap_or(""),
                        v.tin_on_file,
                        money(v.paid),
                        v.reportable
                    ));
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
        // The evidence: every current receipt on the year's posted entries,
        // and the company's documents, with an index tying them together.
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
                let bytes = store.attachment_bytes(a.id).map_err(|e| e.to_string())?;
                match a.entry_id.and_then(|e| ledger.get_entry(e)) {
                    Some(e)
                        if e.status() == ledger_core::EntryStatus::Posted
                            && e.date() >= fy
                            && e.date() <= as_of =>
                    {
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
                        files.push((name, bytes));
                        receipt_count += 1;
                    }
                    None if a.entry_id.is_none() && a.category != "receipts" => {
                        files.push((
                            format!("documents/{}/{}", clean(&a.category), clean(&a.file_name)),
                            bytes,
                        ));
                    }
                    _ => {}
                }
            }
            files.push(("receipts/index.csv".into(), index.into_bytes()));
        }
        let seq = store.journal_seq();
        drop(store);
        files.push((
            "general-journal.iif".into(),
            self.export_iif()?.into_bytes(),
        ));
        let readme = format!(
            "{name} — books package for the CPA\n\nFiscal year to date: {fy_start} through {as_of_s}. {basis} basis.\nKept in wanderbooks: double-entry books from an append-only, hash-chained journal ({seq} events, verified).\n\nStatements (PDF and XLSX): profit and loss, balance sheet, cash flow, {members}tax summary, trial balance.\nLedgers (XLSX): general ledger by account; general journal entry by entry.\ngeneral-journal.iif: every posted entry, for import into QuickBooks Desktop.\n1099-worksheet-{year}.csv: cash paid per vendor marked for a 1099, against the year's threshold.\n{receipts}\nThe tax summary maps accounts to return categories; it's a worksheet, not a filing. Questions go to the owners, who can ask their bookkeeping agent to trace any number to its entries and receipts.\n",
            name = self.settings().company.name,
            basis = self.settings().company.basis,
            members = if self.settings().company.members.is_empty() {
                ""
            } else {
                "members' equity, "
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

        Ok(Json(ExportCpaPackageOut {
            file_name: format!("cpa-package-{as_of}.zip"),
            content_base64: base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                &zip_buf,
            ),
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
        let r = ledger_core::fixed_asset_register(store.ledger(), as_of);
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
                })).collect::<Vec<_>>(),
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
            "tag,description,serial,in_service,cost,salvage,useful_life_months,accumulated,book_value,status,proceeds\n",
        );
        for a in &r.rows {
            csv.push_str(&format!(
                "{},{},{},{},{},{},{},{},{},{},{}\n",
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
            ));
        }
        csv.push_str(&format!(
            "TOTAL,,,,{},,,{},{},,",
            money(r.total_cost),
            money(r.total_accumulated),
            money(r.total_book_value)
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
                let since = query
                    .split('&')
                    .find_map(|kv| kv.strip_prefix("since="))
                    .unwrap_or("")
                    .to_string();
                let Json(out) = self.digest(Parameters(DigestParams { since })).await?;
                serde_json::to_string_pretty(&out).map_err(|e| e.to_string())
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
        // Pick up an edited wanderbooks.toml before the tool runs.
        self.live().refresh();
        let tcc = rmcp::handler::server::tool::ToolCallContext::new(self, request, context);
        Self::tool_router().call(tcc).await
    }

    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
        // Tell the agent the policy these books actually use.
        .with_instructions(INSTRUCTIONS.replace(
            "any entry moving more than $25.00 in debits needs at least one receipt",
            &self.settings().policy.rule(),
        ))
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
        let mk = |uri: &str, name: &str, title: &str, description: &str| {
            let mut r = Resource::new(uri, name);
            r.title = Some(title.to_string());
            r.description = Some(description.to_string());
            r.mime_type = Some("application/json".to_string());
            r
        };
        let resources = vec![
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
                "What changed in the books. Optional ?since=YYYY-MM-DD query.",
            ),
        ];
        Ok(ListResourcesResult::with_all_items(resources))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, rmcp::ErrorData> {
        self.live().refresh();
        let uri = request.uri.clone();
        let text = self.resource_text(&uri).await.map_err(|e| {
            rmcp::ErrorData::resource_not_found(uri.clone(), Some(serde_json::json!({"reason": e})))
        })?;
        Ok(ReadResourceResponse::Complete(ReadResourceResult::new(
            vec![ResourceContents::TextResourceContents {
                uri,
                mime_type: Some("application/json".to_string()),
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

    fn store(tag: &str) -> Store {
        let dir =
            std::env::temp_dir().join(format!("ledger-mcp-unit-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Store::create(&dir, Currency::USD).unwrap()
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
                },
                Member {
                    name: "Bo".into(),
                    share: Decimal::new(5, 1),
                    capital_account: "3001".into(),
                    distribution_account: None,
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

    /// A period is "YYYY-MM": year at least 1, month 1-12, nothing more.
    #[test]
    fn periods_are_a_year_and_a_month() {
        assert_eq!(parse_period_mcp("2026-01"), Ok((2026, 1)));
        assert_eq!(parse_period_mcp("2026-12"), Ok((2026, 12)));
        assert_eq!(parse_period_mcp("0001-01"), Ok((1, 1)));
        for bad in [
            "2026-13",
            "2026-00",
            "2026-01-05",
            "0-05",
            "2026",
            "x-01",
            "2026-",
            "",
        ] {
            assert!(parse_period_mcp(bad).is_err(), "{bad:?}");
        }
    }

    /// Settings naming a code are found: cash/financing/investing roles,
    /// close_into, and each member's capital and distributions accounts.
    #[test]
    fn config_mentions_finds_every_setting_naming_a_code() {
        let server = BooksServer::new(store("mentions"), ReceiptPolicy::default())
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
        let solo = BooksServer::new(store("mentions-solo"), ReceiptPolicy::default()).with_company(
            Company {
                close_into: Some("3000".into()),
                ..Company::default()
            },
        );
        assert_eq!(solo.config_mentions("3000"), ["[company] close_into"]);
    }

    /// Reports name each member with their accounts, in order.
    #[test]
    fn report_members_carry_names_and_accounts() {
        let server =
            BooksServer::new(store("members"), ReceiptPolicy::default()).with_company(members());
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
        let mut s = store("earnings");
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
        let mut s = store("earnings-open");
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
        let mut s = store("unlink");
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
}
