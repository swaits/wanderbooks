//! The MCP server: narrow typed tools over the store, with the receipt
//! policy enforced server-side.

use crate::company::Company;
use crate::policy::ReceiptPolicy;
use crate::types::*;
use chrono::NaiveDate;
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
const INSTRUCTIONS: &str = "You are the bookkeeping agent for these books. The engine enforces the accounting; you enforce the workflow.\n\nRULES\n- Money is exact decimal. Always pass amounts as decimal strings (\"123.45\"), never floats.\n- Never post directly. The flow is draft (books.draft_entry) -> add lines (books.add_line) -> preview (books.post_entry without confirmed) -> review the preview -> post (books.post_entry with confirmed=true).\n- Every entry must balance: debits equal credits, exactly. Every line needs a real account from books.chart. The engine rejects anything else.\n- Receipts: any entry moving more than $25.00 in debits needs at least one receipt attached (books.attach_receipt) before posting. The server rejects the post otherwise. This is enforced, not a suggestion.\n- Never invent accounts. If the account you need does not exist, add it with books.add_account first.\n- Corrections go through books.reverse_entry. Posted entries are immutable; there is no edit.\n- Receipt corrections: receipts can change, but nothing is erased. To replace one, attach the new receipt, then books.unlink_receipt the old (with a reason); retire unwanted receipts or documents with books.withdraw_attachment (with a reason). The journal keeps every step.\n- After posting, check books.digest to review what changed, so a human can review too.\n- Amounts in every response are decimal strings. Treat them as exact.\n- Fixed assets: register with books.register_asset, inspect with books.asset_schedule and books.list_assets, post monthly depreciation with books.run_depreciation (exempt from the receipt policy; the schedule is the evidence), and dispose with books.dispose_asset (preview first; the bill of sale is the receipt). Depreciation is straight-line, book basis only - tax depreciation stays with the CPA.";

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
            journal_seq: store.journal_seq(),
        }
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

fn money(n: Decimal) -> String {
    // Decimal's Display never emits floats or "-0" for exact values.
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

fn today() -> NaiveDate {
    chrono::Utc::now().date_naive()
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
        description = "The trial balance: every account with a non-zero balance, debit and credit columns and totals. Debits must equal credits.",
        annotations(read_only_hint = true)
    )]
    async fn trial_balance(&self) -> Result<Json<TrialBalanceOut>, String> {
        let store = self.lock()?;
        let tb = reports::trial_balance(store.ledger());
        Ok(Json(TrialBalanceOut {
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
            None => chrono::Utc::now().date_naive(),
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
        let id = store
            .create_entry(parse_date(&p.date)?, &p.description)
            .map_err(|e| e.to_string())?;
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
        name = "books.digest",
        description = "What changed since a date: entries posted, per-account movement, and receipt coverage. Built for human review.",
        annotations(read_only_hint = true)
    )]
    async fn digest(&self, params: Parameters<DigestParams>) -> Result<Json<DigestOut>, String> {
        let p = params.0;
        let since = parse_date(&p.since)?;
        let today = chrono::Utc::now().date_naive();
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
        for row in &rows {
            if store.ledger().chart().find_by_code(&row.code).is_some() {
                return Err(format!("duplicate account code: {}", row.code));
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
            store
                .add_account(&row.code, &row.name, account_type, parent, false)
                .map_err(|e| e.to_string())?;
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
        description = "Export a report. Formats: csv, json, xlsx (base64), pdf (base64; profit_and_loss, balance_sheet, cash_flow), iif (journal only, for QuickBooks Desktop). Reports: journal, trial_balance, profit_and_loss, balance_sheet, cash_flow. cash_flow needs cash_accounts.",
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
            "journal" => self.export_journal(&format)?,
            "trial_balance" => self.export_trial_balance(&format)?,
            "profit_and_loss" => self.export_pnl(&format, p.start.as_deref(), p.end.as_deref())?,
            "balance_sheet" => self.export_balance_sheet(&format, p.as_of.as_deref())?,
            "cash_flow" => self.export_cash_flow(&format, &p)?,
            _ => {
                return Err(format!(
                    "bad report {:?}: expected journal, trial_balance, profit_and_loss, balance_sheet, or cash_flow",
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
        Ok(match report {
            "profit_and_loss" => report::profit_and_loss(&pnl_statement(
                ledger,
                p.start.as_deref(),
                p.end.as_deref(),
                None,
            )?),
            "balance_sheet" => report::balance_sheet(&bs_statement(ledger, p.as_of.as_deref())?),
            "cash_flow" => report::cash_flow(&cash_flow_statement(
                ledger,
                p.start.as_deref(),
                p.end.as_deref(),
                &p.cash_accounts,
                &p.financing_accounts,
            )?),
            _ => {
                return Err(format!(
                    "bad report {report:?}: expected profit_and_loss, balance_sheet, or cash_flow"
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
        if report == "trial_balance" {
            let tb = reports::trial_balance(ledger);
            return report_to_xlsx(&XlsxReport::TrialBalance(&tb), &meta)
                .map_err(|e| e.to_string());
        }
        let st = self.layout_statement(ledger, report, p)?;
        report_to_xlsx(&XlsxReport::Statement(&st), &meta).map_err(|e| e.to_string())
    }

    /// Render a financial statement to PDF bytes via ledger-import.
    fn export_pdf(&self, report: &str, p: &ExportParams) -> Result<Vec<u8>, String> {
        let store = self.lock()?;
        let meta = self.report_meta(&store);
        let st = self
            .layout_statement(store.ledger(), report, p)
            .map_err(|e| e.replace("bad report", "bad report for pdf"))?;
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
        let today = chrono::Utc::now().date_naive();
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
        description = "Close an accounting period. Without confirmed=true, returns a preview of the closing entry (revenue/expense zeroed to Retained Earnings) and closes nothing. With confirmed=true, posts the closing entry and marks the period closed. Closed periods reject new entries."
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
        if !p.confirmed {
            // Preview: compute what the closing entry would look like
            // without mutating. We do this by inspecting revenue/expense
            // balances directly.
            let mut lines = Vec::new();
            let mut accounts: Vec<_> = store.ledger().chart().all().collect();
            accounts.sort_by(|a, b| a.code.cmp(&b.code));
            for account in accounts {
                match account.account_type {
                    AccountType::Revenue | AccountType::Expense => {
                        let movement = store.ledger().account_movement(account.id);
                        if movement.is_zero() {
                            continue;
                        }
                        let (side, amount) = if movement > Decimal::ZERO {
                            ("credit", movement)
                        } else {
                            ("debit", -movement)
                        };
                        lines.push(format!("{} {} {} to zero", side, account.code, amount));
                    }
                    _ => {}
                }
            }
            return Ok(Json(ClosePeriodOut {
                status: "preview".to_string(),
                period_id: p.period_id,
                closing_entry: None,
                lines,
            }));
        }
        let closing = store.close_period(p.period_id).map_err(|e| e.to_string())?;
        // Build line descriptions from the posted closing entry.
        let mut lines = Vec::new();
        if let Some(eid) = closing
            && let Some(entry) = store.ledger().get_entry(eid)
        {
            for line in entry.lines() {
                let code = store
                    .ledger()
                    .chart()
                    .get(line.account)
                    .map(|a| a.code.clone())
                    .unwrap_or_default();
                lines.push(format!("{:?} {} {}", line.side, code, line.amount.amount));
            }
        }
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
            None => chrono::Utc::now().date_naive(),
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
        description = "Build the CPA handoff package as a zip (base64): trial-balance.xlsx, general-ledger.xlsx (every posted line), profit-and-loss.pdf, balance-sheet.pdf, cash-flow.pdf. cash_accounts (e.g. [\"1000\"]) is required: the cash flow statement reconciles to them. The as_of date defaults to today; the P&L and cash flow cover the fiscal year to date.",
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
            |st: ledger_import::Statement| statement_pdf(&st, &meta).map_err(|e| e.to_string());

        let tb_xlsx = xlsx(XlsxReport::TrialBalance(&reports::trial_balance(ledger)))?;
        let gl_xlsx = xlsx(XlsxReport::GeneralLedger(ledger))?;
        let pnl_pdf = pdf(report::profit_and_loss(&pnl_statement(
            ledger,
            Some(&fy_start),
            Some(&as_of_s),
            None,
        )?))?;
        let bs_pdf = pdf(report::balance_sheet(&bs_statement(
            ledger,
            Some(&as_of_s),
        )?))?;
        let cf_pdf = pdf(report::cash_flow(&cash_flow_statement(
            ledger,
            Some(&fy_start),
            Some(&as_of_s),
            &p.cash_accounts,
            &p.financing_accounts,
        )?))?;
        drop(store);

        // Zip it.
        let mut zip_buf = Vec::new();
        {
            let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut zip_buf));
            let opts = zip::write::SimpleFileOptions::default();
            for (name, bytes) in [
                ("trial-balance.xlsx", tb_xlsx.as_slice()),
                ("general-ledger.xlsx", gl_xlsx.as_slice()),
                ("profit-and-loss.pdf", pnl_pdf.as_slice()),
                ("balance-sheet.pdf", bs_pdf.as_slice()),
                ("cash-flow.pdf", cf_pdf.as_slice()),
            ] {
                zip.start_file(name, opts).map_err(|e| e.to_string())?;
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

    fn export_trial_balance(&self, format: &str) -> Result<String, String> {
        let store = self.lock()?;
        let tb = reports::trial_balance(store.ledger());
        if format == "json" {
            return serde_json::to_string_pretty(&serde_json::json!({
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
            None => chrono::Utc::now().date_naive(),
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
                let Json(out) = self.trial_balance().await?;
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
        .with_instructions(INSTRUCTIONS)
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
