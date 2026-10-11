//! The MCP server: narrow typed tools over the store, with the receipt
//! policy enforced server-side.

use crate::policy::ReceiptPolicy;
use crate::types::*;
use chrono::NaiveDate;
use ledger_core::{
    AccountId, AccountType, AssetStatus, JournalEntry, Ledger, Money, NewAsset, Side,
    accumulated_depreciation, book_value, depreciation_schedule, reports,
};
use ledger_store::Store;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::{Implementation, ServerCapabilities, ServerConfig};
use rmcp::{ServerHandler, tool, tool_handler, tool_router};
use rust_decimal::Decimal;
use std::sync::{Arc, Mutex};

/// The agent operating contract, delivered via initialize.
const INSTRUCTIONS: &str = "You are the bookkeeping agent for these books. The engine enforces the accounting; you enforce the workflow.\n\nRULES\n- Money is exact decimal. Always pass amounts as decimal strings (\"123.45\"), never floats.\n- Never post directly. The flow is draft (books.draft_entry) -> add lines (books.add_line) -> preview (books.post_entry without confirmed) -> review the preview -> post (books.post_entry with confirmed=true).\n- Every entry must balance: debits equal credits, exactly. Every line needs a real account from books.chart. The engine rejects anything else.\n- Receipts: any entry moving more than $25.00 in debits needs at least one receipt attached (books.attach_receipt) before posting. The server rejects the post otherwise. This is enforced, not a suggestion.\n- Never invent accounts. If the account you need does not exist, add it with books.add_account first.\n- Corrections go through books.reverse_entry. Posted entries are immutable; there is no edit.\n- After posting, check books.digest to review what changed, so a human can review too.\n- Amounts in every response are decimal strings. Treat them as exact.\n- Fixed assets: register with books.register_asset, inspect with books.asset_schedule and books.list_assets, post monthly depreciation with books.run_depreciation (exempt from the receipt policy; the schedule is the evidence), and dispose with books.dispose_asset (preview first; the bill of sale is the receipt). Depreciation is straight-line, book basis only - tax depreciation stays with the CPA.";

/// Max receipt/document upload: 25 MB.
const MAX_UPLOAD_BYTES: usize = 25 * 1024 * 1024;

/// The server. One store, one receipt policy, shared across tool calls.
/// `Store` is `!Sync`, so access is serialized through a mutex; the server
/// is single-writer by construction.
#[derive(Clone)]
pub struct BooksServer {
    store: Arc<Mutex<Store>>,
    policy: ReceiptPolicy,
}

impl BooksServer {
    /// Build the server around an open store.
    pub fn new(store: Store, policy: ReceiptPolicy) -> Self {
        Self {
            store: Arc::new(Mutex::new(store)),
            policy,
        }
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Store>, String> {
        self.store
            .lock()
            .map_err(|_| "store lock poisoned".to_string())
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

/// Net movement of one account over entries on or before `through`, in the
/// account's normal side.
fn movement_through(ledger: &Ledger, account: AccountId, through: NaiveDate) -> Decimal {
    let movement: Decimal = ledger
        .posted_entries()
        .filter(|e| e.date() <= through)
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
        description = "Profit and loss for a period: revenue, expenses, net income. Dates are YYYY-MM-DD; omit for all time.",
        annotations(read_only_hint = true)
    )]
    async fn profit_and_loss(
        &self,
        params: Parameters<ProfitAndLossParams>,
    ) -> Result<Json<ProfitAndLossOut>, String> {
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
        let mut revenue = Decimal::ZERO;
        let mut expenses = Decimal::ZERO;
        for a in ledger.chart().all() {
            if a.placeholder {
                continue;
            }
            match a.account_type {
                AccountType::Revenue => revenue += movement_in(ledger, a.id, start, end),
                AccountType::Expense => expenses += movement_in(ledger, a.id, start, end),
                _ => {}
            }
        }
        Ok(Json(ProfitAndLossOut {
            start: start.to_string(),
            end: end.to_string(),
            revenue: money(revenue),
            expenses: money(expenses),
            net_income: money(revenue - expenses),
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
        let ledger = store.ledger();
        let as_of = match p.as_of {
            Some(s) => parse_date(&s)?,
            None => chrono::Utc::now().date_naive(),
        };
        let mut assets = Decimal::ZERO;
        let mut liabilities = Decimal::ZERO;
        let mut equity = Decimal::ZERO;
        let mut revenue = Decimal::ZERO;
        let mut expenses = Decimal::ZERO;
        for a in ledger.chart().all() {
            if a.placeholder {
                continue;
            }
            let m = movement_through(ledger, a.id, as_of);
            match a.account_type {
                AccountType::Asset => assets += m,
                AccountType::Liability => liabilities += m,
                AccountType::Equity => equity += m,
                AccountType::Revenue => revenue += m,
                AccountType::Expense => expenses += m,
            }
        }
        let current_earnings = revenue - expenses;
        equity += current_earnings;
        Ok(Json(BalanceSheetOut {
            as_of: as_of.to_string(),
            assets: money(assets),
            liabilities: money(liabilities),
            equity: money(equity),
            current_earnings: money(current_earnings),
            balanced: assets == liabilities + equity,
        }))
    }

    #[tool(
        name = "books.draft_entry",
        description = "Start a draft journal entry. Drafts are not journaled until posted; add lines with books.add_line."
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
        description = "Add one line to a draft entry. Amount is a decimal string, never a float. The entry must balance before posting."
    )]
    async fn add_line(
        &self,
        params: Parameters<AddLineParams>,
    ) -> Result<Json<AddLineOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let account = resolve_account(&store, &p.account)?;
        let amount = parse_amount(&p.amount)?;
        if amount <= Decimal::ZERO {
            return Err("amount must be positive".to_string());
        }
        let currency = store.base_currency();
        store
            .add_line(
                p.entry_id,
                account,
                parse_side(&p.side)?,
                Money::new(amount, currency),
            )
            .map_err(|e| e.to_string())?;
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
        let receipt_required = self.policy.receipt_required(store.ledger(), &entry);
        let receipts = preview_receipts(&store, p.entry_id)?;
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
        };
        if !p.confirmed.unwrap_or(false) {
            return Ok(Json(out));
        }
        if receipt_missing {
            return Err(format!(
                "receipt_required: entry {} moves {} in debits ({}); attach a receipt with books.attach_receipt before posting",
                p.entry_id,
                money(total),
                self.policy.describe()
            ));
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
        name = "books.list_receipts",
        description = "List receipts and filed documents, optionally filtered to one entry.",
        annotations(read_only_hint = true)
    )]
    async fn list_receipts(
        &self,
        params: Parameters<ListReceiptsParams>,
    ) -> Result<Json<ListReceiptsOut>, String> {
        let p = params.0;
        let store = self.lock()?;
        let receipts = store
            .list_attachments(p.entry_id)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|a| ReceiptOut {
                attachment_id: a.id,
                sha256: a.blob_sha,
                entry_id: a.entry_id,
                category: a.category,
                file_name: a.file_name,
                mime_type: a.mime_type,
                size_bytes: a.size_bytes,
                description: a.description,
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
        name = "books.export",
        description = "Export a report as CSV or JSON. Reports: journal (every posted entry with lines), trial_balance, profit_and_loss, balance_sheet.",
        annotations(read_only_hint = true)
    )]
    async fn export(&self, params: Parameters<ExportParams>) -> Result<Json<ExportOut>, String> {
        let p = params.0;
        let format = p.format.to_lowercase();
        if format != "csv" && format != "json" {
            return Err(format!(
                "bad format {:?}: expected \"csv\" or \"json\"",
                p.format
            ));
        }
        let report = p.report.to_lowercase();
        let content = match report.as_str() {
            "journal" => self.export_journal(&format)?,
            "trial_balance" => self.export_trial_balance(&format)?,
            "profit_and_loss" => self.export_pnl(&format, p.start.as_deref(), p.end.as_deref())?,
            "balance_sheet" => self.export_balance_sheet(&format, p.as_of.as_deref())?,
            _ => {
                return Err(format!(
                    "bad report {:?}: expected journal, trial_balance, profit_and_loss, or balance_sheet",
                    p.report
                ));
            }
        };
        Ok(Json(ExportOut {
            format,
            report,
            content,
        }))
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
            return Err(format!(
                "receipt_required: disposing {} moves {} in debits ({}); file the bill of sale with books.file_document, then retry with receipt_attachment_ids",
                asset.description,
                money(debits),
                self.policy.describe()
            ));
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
        let ledger = store.ledger();
        let start_d = match start {
            Some(s) => parse_date(s)?,
            None => NaiveDate::from_ymd_opt(1, 1, 1).unwrap(),
        };
        let end_d = match end {
            Some(s) => parse_date(s)?,
            None => chrono::Utc::now().date_naive(),
        };
        let mut revenue = Decimal::ZERO;
        let mut expenses = Decimal::ZERO;
        for a in ledger.chart().all() {
            if a.placeholder {
                continue;
            }
            match a.account_type {
                AccountType::Revenue => revenue += movement_in(ledger, a.id, start_d, end_d),
                AccountType::Expense => expenses += movement_in(ledger, a.id, start_d, end_d),
                _ => {}
            }
        }
        if format == "json" {
            return serde_json::to_string_pretty(&serde_json::json!({
                "start": start_d.to_string(), "end": end_d.to_string(),
                "revenue": money(revenue), "expenses": money(expenses),
                "net_income": money(revenue - expenses),
            }))
            .map_err(|e| e.to_string());
        }
        Ok(format!(
            "metric,amount\nrevenue,{}\nexpenses,{}\nnet_income,{}\n",
            money(revenue),
            money(expenses),
            money(revenue - expenses)
        ))
    }

    fn export_balance_sheet(&self, format: &str, as_of: Option<&str>) -> Result<String, String> {
        let store = self.lock()?;
        let ledger = store.ledger();
        let as_of_d = match as_of {
            Some(s) => parse_date(s)?,
            None => chrono::Utc::now().date_naive(),
        };
        let mut assets = Decimal::ZERO;
        let mut liabilities = Decimal::ZERO;
        let mut equity = Decimal::ZERO;
        let mut revenue = Decimal::ZERO;
        let mut expenses = Decimal::ZERO;
        for a in ledger.chart().all() {
            if a.placeholder {
                continue;
            }
            let m = movement_through(ledger, a.id, as_of_d);
            match a.account_type {
                AccountType::Asset => assets += m,
                AccountType::Liability => liabilities += m,
                AccountType::Equity => equity += m,
                AccountType::Revenue => revenue += m,
                AccountType::Expense => expenses += m,
            }
        }
        let current_earnings = revenue - expenses;
        equity += current_earnings;
        if format == "json" {
            return serde_json::to_string_pretty(&serde_json::json!({
                "as_of": as_of_d.to_string(),
                "assets": money(assets), "liabilities": money(liabilities),
                "equity": money(equity), "current_earnings": money(current_earnings),
                "balanced": assets == liabilities + equity,
            }))
            .map_err(|e| e.to_string());
        }
        Ok(format!(
            "metric,amount\nassets,{}\nliabilities,{}\nequity,{}\ncurrent_earnings,{}\n",
            money(assets),
            money(liabilities),
            money(equity),
            money(current_earnings)
        ))
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

#[tool_handler]
impl ServerHandler for BooksServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions(INSTRUCTIONS)
            .with_server_info(Implementation::new(
                "wanderbooks",
                env!("CARGO_PKG_VERSION"),
            ))
    }
}
