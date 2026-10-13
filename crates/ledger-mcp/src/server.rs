//! The MCP server: narrow typed tools over the store, with the receipt
//! policy enforced server-side.

use crate::policy::ReceiptPolicy;
use crate::types::*;
use chrono::NaiveDate;
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
const INSTRUCTIONS: &str = "You are the bookkeeping agent for these books. The engine enforces the accounting; you enforce the workflow.\n\nRULES\n- Money is exact decimal. Always pass amounts as decimal strings (\"123.45\"), never floats.\n- Never post directly. The flow is draft (books.draft_entry) -> add lines (books.add_line) -> preview (books.post_entry without confirmed) -> review the preview -> post (books.post_entry with confirmed=true).\n- Every entry must balance: debits equal credits, exactly. Every line needs a real account from books.chart. The engine rejects anything else.\n- Receipts: any entry moving more than $25.00 in debits needs at least one receipt attached (books.attach_receipt) before posting. The server rejects the post otherwise. This is enforced, not a suggestion.\n- Never invent accounts. If the account you need does not exist, add it with books.add_account first.\n- Corrections go through books.reverse_entry. Posted entries are immutable; there is no edit.\n- Receipt corrections: receipts can change, but nothing is erased. To replace one, attach the new receipt, then books.unlink_receipt the old (with a reason); retire unwanted receipts or documents with books.withdraw_attachment (with a reason). The journal keeps every step.\n- After posting, check books.digest to review what changed, so a human can review too.\n- Amounts in every response are decimal strings. Treat them as exact.\n- Fixed assets: register with books.register_asset, inspect with books.asset_schedule and books.list_assets, post monthly depreciation with books.run_depreciation (exempt from the receipt policy; the schedule is the evidence), and dispose with books.dispose_asset (preview first; the bill of sale is the receipt). Depreciation is straight-line, book basis only - tax depreciation stays with the CPA.\n- Prepaid expenses and deferred revenue: record the original payment (with its receipt), then books.create_schedule; books.run_schedules posts the monthly amortization (preview first; exempt from the receipt policy like depreciation). Recurring entries (books.create_recurring, books.run_recurring) only ever create drafts: attach each actual receipt and post through the normal preview.\n- Platform payouts (app stores, payment processors, marketplaces): never book the net deposit as revenue. books.record_payout grosses it up (gross revenue, fees, adjustments) into a draft and refuses unless it reconciles to the cent; attach the payout statement, then preview and post.\n- 1099s: mark contractors and landlords with books.set_vendor_1099 (and whether their W-9 is on file; never store a TIN), and tag every payment to them with its vendor (books.draft_entry vendor, or books.set_entry_vendor; books.pay_bill tags automatically). books.vendor_1099 is the year-end worksheet for the CPA.";

/// Max receipt/document upload: 25 MB.
const MAX_UPLOAD_BYTES: usize = 25 * 1024 * 1024;

/// The server. One store, one receipt policy, shared across tool calls.
/// `Store` is `!Sync`, so access is serialized through a mutex; the server
/// is single-writer by construction.
#[derive(Clone)]
pub struct BooksServer {
    store: Arc<Mutex<Store>>,
    policy: ReceiptPolicy,
    company: Company,
}

impl BooksServer {
    /// Build the server around an open store.
    pub fn new(store: Store, policy: ReceiptPolicy) -> Self {
        Self {
            store: Arc::new(Mutex::new(store)),
            policy,
            company: Company::default(),
        }
    }

    /// Build the server around a shared store (for HTTP: each connection
    /// gets a handle to the same store).
    pub fn new_shared(store: Arc<Mutex<Store>>, policy: ReceiptPolicy) -> Self {
        Self {
            store,
            policy,
            company: Company::default(),
        }
    }

    /// Set the company (name for report headers, fiscal year).
    pub fn with_company(mut self, company: Company) -> Self {
        self.company = company;
        self
    }

    /// Header and provenance for a rendered report.
    fn report_meta(&self, store: &Store) -> ReportMeta {
        ReportMeta {
            company: self.company.name.clone(),
            basis: self.company.basis.clone(),
            prepared_on: today(),
            journal_seq: store.journal_seq(),
        }
    }

    /// Members as reports name them (empty unless a multi-member LLC).
    fn report_members(&self) -> Vec<ledger_import::Member> {
        self.company
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
        if self.company.members.is_empty() {
            return Err(
                "no members configured: add [[company.members]] (name, share, capital_account) to wanderbooks.toml"
                    .to_string(),
            );
        }
        self.company
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
        if !self.company.members.is_empty() {
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
            .company
            .close_sweep
            .iter()
            .map(|c| code(c))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(match self.company.close_into.as_deref() {
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
    /// part of it still unclosed (closing entries included, so months
    /// already closed into equity drop out).
    fn year_earnings(
        &self,
        ledger: &Ledger,
        as_of: NaiveDate,
    ) -> Result<ledger_import::YearEarnings, String> {
        let (fy_start, _) = self.company.fiscal_year_containing(as_of);
        let net_income_ytd = ledger_core::profit_and_loss_for_period(ledger, fy_start, as_of, None)
            .map_err(|e| e.to_string())?
            .net_income();
        let mut unclosed = Decimal::ZERO;
        for a in ledger.chart().all().filter(|a| !a.placeholder) {
            match a.account_type {
                AccountType::Revenue => unclosed += movement_in(ledger, a.id, fy_start, as_of),
                AccountType::Expense => unclosed -= movement_in(ledger, a.id, fy_start, as_of),
                _ => {}
            }
        }
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
        let split_group = self.policy.split_group(store.ledger(), entry_id, entry);
        if !self.policy.receipt_required(store.ledger(), entry) && split_group.is_empty() {
            return Ok(());
        }
        let total = ReceiptPolicy::entry_debits(entry);
        Err(ReceiptRequiredError {
            code: "receipt_required".to_string(),
            message: format!(
                "entry {entry_id} moves {} in debits ({}); unlinking its last receipt would leave it without one.",
                money(total),
                self.policy.describe()
            ),
            entry_id: Some(entry_id),
            asset_id: None,
            debits: total.to_string(),
            policy: self.policy.describe(),
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

fn parse_amount(s: &str) -> Result<Decimal, String> {
    s.parse::<Decimal>()
        .map_err(|_| format!("bad amount {s:?}: expected a decimal string like \"123.45\""))
}

fn parse_side(s: &str) -> Result<Side, String> {
    match s.to_lowercase().as_str() {
        "debit" => Ok(Side::Debit),
        "credit" => Ok(Side::Credit),
        _ => Err(format!("bad side {s:?}: expected \"debit\" or \"credit\"")),
    }
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

/// Resolve an account reference: a numeric id, or an account code.
fn resolve_account(store: &Store, account: &str) -> Result<AccountId, String> {
    if let Ok(id) = account.parse::<u64>()
        && store.ledger().chart().get(id).is_some()
    {
        return Ok(id);
    }
    store
        .ledger()
        .chart()
        .find_by_code(account)
        .map(|a| a.id)
        .ok_or_else(|| format!("unknown account {account:?}: use an id or code from books.chart"))
}

/// Resolve a class by id or name (case-insensitive).
fn resolve_vendor(store: &Store, vendor: &str) -> Result<u64, String> {
    if let Ok(id) = vendor.parse::<u64>()
        && store.ledger().get_vendor(id).is_some()
    {
        return Ok(id);
    }
    let matches: Vec<u64> = store
        .ledger()
        .list_vendors()
        .into_iter()
        .filter(|v| v.name.eq_ignore_ascii_case(vendor.trim()))
        .map(|v| v.id)
        .collect();
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

fn resolve_class(store: &Store, class: &str) -> Result<u32, String> {
    if let Ok(id) = class.parse::<u32>()
        && store.ledger().get_class(id).is_some()
    {
        return Ok(id);
    }
    store
        .ledger()
        .find_class_by_name(class)
        .map(|c| c.id)
        .ok_or_else(|| {
            format!("unknown class {class:?}: use an id or name from books.list_classes")
        })
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
        return Err("cash_accounts is required for the cash flow statement".to_string());
    }
    let financing_accounts = resolve(financing_codes, "financing")?;
    let (start, end) = statement_range(ledger, start, end)?;
    ledger_core::cash_flow(
        ledger,
        &ledger_core::CashFlowParams {
            start,
            end,
            cash_accounts,
            financing_accounts,
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
        let start = match p.start {
            Some(s) => parse_date(&s)?,
            None => NaiveDate::from_ymd_opt(1, 1, 1).unwrap(),
        };
        let end = match p.end {
            Some(s) => parse_date(&s)?,
            None => today(),
        };
        // Resolve cash and financing account codes to ids.
        let mut cash_accounts = Vec::new();
        for code in &p.cash_accounts {
            let id = ledger
                .chart()
                .find_by_code(code)
                .map(|a| a.id)
                .ok_or_else(|| format!("unknown cash account code: {code}"))?;
            cash_accounts.push(id);
        }
        if cash_accounts.is_empty() {
            return Err("at least one cash account code is required".to_string());
        }
        let mut financing_accounts = Vec::new();
        for code in &p.financing_accounts {
            let id = ledger
                .chart()
                .find_by_code(code)
                .map(|a| a.id)
                .ok_or_else(|| format!("unknown financing account code: {code}"))?;
            financing_accounts.push(id);
        }
        let stmt = ledger_core::cash_flow(
            ledger,
            &ledger_core::CashFlowParams {
                start,
                end,
                cash_accounts,
                financing_accounts,
            },
        )
        .map_err(|e| e.to_string())?;
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
        if let Some(class_ref) = p.class {
            let class_id = resolve_class(&store, &class_ref)?;
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
        let mut receipt_required = self.policy.receipt_required(store.ledger(), &entry);
        let receipts = preview_receipts(&store, p.entry_id)?;
        // Anti-splitting: if this entry looks like part of a split group,
        // the receipt policy applies to the group total.
        let split_group = self.policy.split_group(store.ledger(), p.entry_id, &entry);
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
                    self.policy.describe(),
                    split_note
                ),
                entry_id: Some(p.entry_id),
                asset_id: None,
                debits: total.to_string(),
                policy: self.policy.describe(),
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
            let required = self.policy.receipt_required(ledger, e);
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
        Ok(Json(DigestOut {
            since: since.to_string(),
            entries,
            totals_by_account: totals,
        }))
    }

    #[tool(
        name = "books.import_bank_csv",
        description = "Import a QuickBooks-style bank/credit-card CSV (Date,Description,Amount or Date,Description,Expense,Income). Creates one DRAFT entry per transaction: bank account on one side, categorized offset on the other (via rules or default_offset_code). Review with books.digest, then post."
    )]
    async fn import_bank_csv(
        &self,
        params: Parameters<ImportBankCsvParams>,
    ) -> Result<Json<ImportBankCsvOut>, String> {
        let p = params.0;
        let txns = ledger_import::parse_bank_csv(&p.csv_text).map_err(|e| e.to_string())?;
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
        let mut imported = Vec::new();
        for txn in txns {
            let desc_lower = txn.description.to_lowercase();
            let (offset_id, offset_code) = rules
                .iter()
                .find(|(pat, _)| desc_lower.contains(pat))
                .map(|(_, v)| v.clone())
                .unwrap_or_else(|| default_offset.clone());
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
        description = "Turn every recurring entry's occurrences on or before `through` (default today) into drafts. Idempotent: an occurrence generates once. Each draft then goes through the normal flow: attach the receipt, preview, post."
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
        Ok(Json(RunRecurringOut { drafts }))
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
        description = "1099 worksheet for a calendar year: cash paid to each vendor (credits minus debits to asset accounts on entries tagged with the vendor, so card payments, which the card processor reports on 1099-K, are excluded), which vendors are reportable (marked NEC/MISC and paid at least the threshold: $600 through 2025, $2,000 from 2026), and which are missing a W-9. For the CPA to confirm.",
        annotations(read_only_hint = true)
    )]
    async fn vendor_1099(
        &self,
        params: Parameters<Vendor1099Params>,
    ) -> Result<Json<Vendor1099Out>, String> {
        let p = params.0;
        let threshold = p.threshold.as_deref().map(parse_amount).transpose()?;
        let store = self.lock()?;
        let r = ledger_core::vendor_1099_report(store.ledger(), p.year, threshold);
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
        let form = self.company.tax_form;
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
            ledger_core::tax::check_category(cat, t, self.company.tax_form)
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
        // Validate all rows first: nothing is half-imported.
        let parse_type = |t: &str| match t {
            "asset" => Ok(AccountType::Asset),
            "liability" => Ok(AccountType::Liability),
            "equity" => Ok(AccountType::Equity),
            "revenue" => Ok(AccountType::Revenue),
            "expense" => Ok(AccountType::Expense),
            _ => Err(format!("bad account type: {t}")),
        };
        for row in &rows {
            if store.ledger().chart().find_by_code(&row.code).is_some() {
                return Err(format!("duplicate account code: {}", row.code));
            }
            if let Some(cat) = &row.tax_category {
                ledger_core::tax::check_category(
                    cat,
                    parse_type(&row.account_type)?,
                    self.company.tax_form,
                )
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
        description = "Export a report. Reports: profit_and_loss, balance_sheet, cash_flow (needs cash_accounts), trial_balance (as_of), members_equity (multi-member LLC; members from wanderbooks.toml), general_ledger (by account: running balances, splits), general_journal (every entry in date order; alias journal). Formats: pdf and xlsx (base64) for the five statements; xlsx, csv, json for every report; iif for general_journal (QuickBooks Desktop). Dates: start/end for period reports (start defaults to the first entry, end to today), as_of for balance_sheet and trial_balance.",
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
            _ => {
                return Err(format!(
                    "bad report {:?}: expected general_journal, general_ledger, trial_balance, profit_and_loss, balance_sheet, cash_flow, members_equity, or tax_summary",
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
                &cash_flow_statement(
                    ledger,
                    p.start.as_deref(),
                    p.end.as_deref(),
                    &p.cash_accounts,
                    &p.financing_accounts,
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
                    &ledger_core::tax_summary(ledger, start, end, self.company.tax_form)
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
        let st = self.layout_statement(store.ledger(), report, p)?;
        ledger_import::statement_pdf(&st, &meta).map_err(|e| e.to_string())
    }

    /// Render the journal as IIF text via ledger-import.
    fn export_iif(&self) -> Result<String, String> {
        use ledger_import::{IifEntry, journal_to_iif};
        let store = self.lock()?;
        let ledger = store.ledger();
        let account_names = |id: ledger_core::AccountId| {
            ledger
                .chart()
                .get(id)
                .map(|a| a.name.clone())
                .unwrap_or_default()
        };
        let entries: Vec<IifEntry<'_>> = ledger
            .posted_entries()
            .map(|e| IifEntry {
                entry: e,
                account_names: &account_names,
            })
            .collect();
        Ok(journal_to_iif(&entries))
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
        Ok(Json(RegisterAssetOut { asset_id }))
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
        let asset = store
            .ledger()
            .get_asset(p.asset_id)
            .ok_or_else(|| format!("unknown asset {}", p.asset_id))?
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
        let asset = store
            .ledger()
            .get_asset(p.asset_id)
            .ok_or_else(|| format!("unknown asset {}", p.asset_id))?
            .clone();
        let preview = store
            .dispose_preview(p.asset_id, date, proceeds)
            .map_err(|e| e.to_string())?;

        // The would-be disposal entry's lines, for the preview and the
        // receipt-policy check.
        let mut raw: Vec<(AccountId, Side, Money)> = vec![
            (
                asset.accum_depr_account,
                Side::Debit,
                Money::new(preview.accumulated, base),
            ),
            (proceeds_account, Side::Debit, proceeds),
            (asset.cost_account, Side::Credit, asset.cost),
        ];
        if preview.gain_loss > Decimal::ZERO {
            raw.push((
                gain_loss_account,
                Side::Credit,
                Money::new(preview.gain_loss, base),
            ));
        } else if preview.gain_loss < Decimal::ZERO {
            raw.push((
                gain_loss_account,
                Side::Debit,
                Money::new(-preview.gain_loss, base),
            ));
        }
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
        let receipt_required = self.policy.receipt_required_for(&code_refs, debits);

        let out = DisposeAssetOut {
            status: "preview".to_string(),
            asset_id: p.asset_id,
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
                    self.policy.describe()
                ),
                entry_id: None,
                asset_id: Some(p.asset_id),
                debits: debits.to_string(),
                policy: self.policy.describe(),
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
                p.asset_id,
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
                    current: r.current.to_string(),
                    days_30: r.days_30.to_string(),
                    days_60: r.days_60.to_string(),
                    days_90: r.days_90.to_string(),
                    days_90_plus: r.days_90_plus.to_string(),
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
                    current: r.current.to_string(),
                    days_30: r.days_30.to_string(),
                    days_60: r.days_60.to_string(),
                    days_90: r.days_90.to_string(),
                    days_90_plus: r.days_90_plus.to_string(),
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
        let mut total = Decimal::ZERO;
        for code in ["2110", "2120", "2130", "2140", "2150"] {
            if let Some(a) = store.ledger().chart().find_by_code(code) {
                total += store
                    .ledger()
                    .account_balance(a.id)
                    .map_err(|e| e.to_string())?;
            }
        }
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
        // Per-class P&L over the date range: accumulate line by line.
        use std::collections::HashMap;
        #[derive(Default)]
        struct Acc {
            revenue: Decimal,
            expenses: Decimal,
        }
        let mut by_class: HashMap<Option<u32>, Acc> = HashMap::new();
        for entry in ledger.posted_entries() {
            if entry.date() < start || entry.date() > end {
                continue;
            }
            for line in entry.lines() {
                let account = match ledger.chart().get(line.account) {
                    Some(a) if !a.placeholder => a,
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
        description = "Build the CPA handoff package as a zip (base64): the profit and loss, balance sheet, cash flow, statement of changes in members' equity (multi-member LLCs), tax summary, and trial balance, each as PDF and XLSX; the general ledger (by account) and general journal (entry by entry) as XLSX; the journal as IIF (imports into QuickBooks Desktop); the 1099 worksheet as CSV; every receipt behind the year's entries (receipts/, with an index.csv tying each file to its entry) and the company's documents (documents/), unless include_receipts is false; and a README with the journal's verified hash. cash_accounts (e.g. [\"1000\"]) is required: the cash flow statement reconciles to them. as_of defaults to today; period reports cover the fiscal year to date.",
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
        let (fy_start, _) = self.company.fiscal_year_containing(as_of);
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
                    &cash_flow_statement(
                        ledger,
                        Some(&fy_start),
                        Some(&as_of_s),
                        &p.cash_accounts,
                        &p.financing_accounts,
                    )?,
                    &members,
                ),
            ),
        ];
        if !self.company.members.is_empty() {
            let me = ledger_core::members_equity(ledger, fy, as_of, &self.member_capitals(ledger)?)
                .map_err(|e| e.to_string())?;
            statements.push(("members-equity", report::members_equity(&me)));
        }
        statements.push((
            "tax-summary",
            report::tax_summary(
                &ledger_core::tax_summary(ledger, fy, as_of, self.company.tax_form)
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
        let r1099 = ledger_core::vendor_1099_report(ledger, year, None);
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
            name = self.company.name,
            basis = self.company.basis,
            members = if self.company.members.is_empty() {
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
        let ts = ledger_core::tax_summary(ledger, start, end, self.company.tax_form)
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
        let mut cash_accounts = Vec::new();
        for code in &p.cash_accounts {
            let id = ledger
                .chart()
                .find_by_code(code)
                .map(|a| a.id)
                .ok_or_else(|| format!("unknown cash account code: {code}"))?;
            cash_accounts.push(id);
        }
        if cash_accounts.is_empty() {
            return Err("cash_accounts is required for cash_flow".to_string());
        }
        let start = match &p.start {
            Some(s) => parse_date(s)?,
            None => NaiveDate::from_ymd_opt(1, 1, 1).unwrap(),
        };
        let end = match &p.end {
            Some(s) => parse_date(s)?,
            None => today(),
        };
        let stmt = ledger_core::cash_flow(
            ledger,
            &ledger_core::CashFlowParams {
                start,
                end,
                cash_accounts,
                financing_accounts: vec![],
            },
        )
        .map_err(|e| e.to_string())?;
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
            &self.policy.rule(),
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
    use super::money;
    use rust_decimal::Decimal;
    use std::str::FromStr;

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
