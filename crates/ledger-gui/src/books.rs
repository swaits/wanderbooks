//! The view model: one immutable picture of the books, rebuilt whenever the
//! journal grows. Built off the UI thread from a [`Snapshot`]; the UI only
//! reads it. Every number here comes from `ledger-core`; this module only
//! indexes and arranges.

use chrono::{Datelike, NaiveDate};
use ledger_config::BooksSettings;
use ledger_core::analysis::{balance_history, balance_series};
use ledger_core::reports::{balance_sheet_as_of, profit_and_loss_for_period};
use ledger_core::{AccountId, AccountType, EntryId, EntryStatus, Ledger, Side};
use ledger_store::{Attachment, JournalLine, Snapshot};
use rust_decimal::Decimal;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// One entry, indexed for listing and detail.
#[derive(Clone, Debug)]
pub struct EntryInfo {
    pub id: EntryId,
    pub date: NaiveDate,
    pub description: String,
    pub status: EntryStatus,
    /// Total debits.
    pub amount: Decimal,
    /// Accounts touched, debits first: "6300 Software".
    pub accounts: Vec<AccountId>,
    /// Indexes into [`Books::attachments`] of linked, current receipts.
    pub receipts: Vec<usize>,
    /// How the entry came to be, in words.
    pub origin: &'static str,
    /// Indexes into [`Books::journal`] of every event that touched it.
    pub events: Vec<usize>,
    pub vendor: Option<String>,
    pub reversal_of: Option<EntryId>,
    pub reversed_by: Option<EntryId>,
    /// Spending over the receipt threshold with no receipt.
    pub missing_receipt: bool,
    /// Net effect on cash accounts: positive in, negative out, zero if the
    /// entry doesn't touch cash.
    pub cash_effect: Decimal,
    /// The customer or vendor, from invoices, bills, and vendor tags.
    pub party: Option<String>,
    /// Lower-cased text for search: description, accounts, vendor, amount.
    pub haystack: String,
}

/// Something the books need, with how to fix it.
#[derive(Clone, Debug)]
pub struct Attention {
    pub severity: Severity,
    pub title: String,
    pub detail: String,
    pub target: Option<crate::nav::Route>,
    /// A prompt to paste into the agent session that fixes it.
    pub prompt: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Problem,
    Todo,
    Info,
}

/// The whole picture.
pub struct Books {
    pub ledger: Ledger,
    pub settings: BooksSettings,
    pub data_dir: PathBuf,
    pub attachments: Vec<Attachment>,
    pub journal: Vec<JournalLine>,
    pub head: String,
    pub today: NaiveDate,
    pub loaded_at: chrono::DateTime<chrono::Utc>,
    /// Newest first (date, then id).
    pub entries: Vec<EntryInfo>,
    pub entry_index: HashMap<EntryId, usize>,
    pub cash_accounts: Vec<AccountId>,
    pub card_accounts: Vec<AccountId>,
    /// Entries created or changed by the most recent refresh.
    pub fresh: HashSet<EntryId>,
    /// Journal seq before the most recent refresh.
    pub fresh_from_seq: u64,
    pub receipt_threshold: Decimal,
    pub attention: Vec<Attention>,
    pub kpis: Kpis,
    pub integrity: Integrity,
}

/// Whether what's on disk is what the journal says.
#[derive(Clone, Debug)]
pub struct Integrity {
    /// The writer's SQLite database against the journal.
    pub disk: ledger_store::DiskCheck,
    /// Stored files whose bytes match their hash.
    pub files_ok: u64,
    /// Files that are missing or damaged: (attachment id, why).
    pub file_failures: Vec<(u64, String)>,
}

impl Integrity {
    /// Check the database and every file not already verified (by hash, so
    /// a file is read once however often the books change).
    pub fn check(snap: &Snapshot, verified: &mut HashSet<String>) -> Integrity {
        let store = ledger_store::BlobStore::open_read_only(&snap.data_dir().join("blobs"));
        let mut ok = 0;
        let mut failures = Vec::new();
        for a in snap.attachments(None).unwrap_or_default() {
            if verified.contains(&a.blob_sha) {
                ok += 1;
                continue;
            }
            match store.get(&a.blob_sha) {
                Ok(_) => {
                    verified.insert(a.blob_sha.clone());
                    ok += 1;
                }
                Err(e) => failures.push((a.id, e.to_string())),
            }
        }
        Integrity {
            disk: snap.check_disk(),
            files_ok: ok,
            file_failures: failures,
        }
    }
}

/// The headline numbers.
#[derive(Clone, Debug, Default)]
pub struct Kpis {
    pub cash: Decimal,
    pub cash_30d_ago: Decimal,
    pub liabilities: Decimal,
    pub receivables: Decimal,
    pub revenue_mtd: Decimal,
    pub expenses_mtd: Decimal,
    /// Last month over the same days as this month so far.
    pub revenue_prev_month: Decimal,
    pub expenses_prev_month: Decimal,
    pub net_ytd: Decimal,
    /// Average monthly net loss over the last three full months, if losing.
    pub monthly_burn: Option<Decimal>,
    pub first_entry: Option<NaiveDate>,
}

impl Books {
    /// Build the picture from a snapshot. `prev` is the previous picture,
    /// to mark what's new.
    pub fn build(
        snap: &Snapshot,
        settings: BooksSettings,
        today: NaiveDate,
        prev_seq: Option<u64>,
        integrity: Integrity,
    ) -> Result<Books, String> {
        let ledger = snap.ledger().clone();
        let attachments = snap.attachments(None).map_err(|e| e.to_string())?;
        let journal = snap.journal().to_vec();
        let receipt_threshold = settings
            .receipt_threshold
            .as_deref()
            .and_then(|t| t.parse().ok())
            .unwrap_or(Decimal::new(25, 0));
        let cash_accounts = cash_accounts(&ledger, &settings);
        let card_accounts: Vec<AccountId> = ledger
            .chart()
            .all()
            .filter(|a| {
                a.account_type == AccountType::Liability && a.name.to_lowercase().contains("card")
            })
            .map(|a| a.id)
            .collect();

        // Journal events per entry.
        let mut events: HashMap<EntryId, Vec<usize>> = HashMap::new();
        for (i, line) in journal.iter().enumerate() {
            for id in entry_refs(line) {
                events.entry(id).or_default().push(i);
            }
        }
        let mut receipts: HashMap<EntryId, Vec<usize>> = HashMap::new();
        for (i, a) in attachments.iter().enumerate() {
            if let (Some(e), None) = (a.entry_id, &a.withdrawn_at) {
                receipts.entry(e).or_default().push(i);
            }
        }
        let fresh_from_seq = prev_seq.unwrap_or(journal.len() as u64);
        let parties = parties(&ledger, &journal);
        let mut fresh = HashSet::new();
        let closings = ledger.closing_entry_ids();
        let mut entries = Vec::new();
        let mut reversed_by: HashMap<EntryId, EntryId> = HashMap::new();
        for id in 1..ledger.next_entry_id() {
            if let Some(e) = ledger.get_entry(id)
                && let Some(orig) = e.reversal_of()
            {
                reversed_by.insert(orig, id);
            }
        }
        for id in 1..ledger.next_entry_id() {
            let Some(e) = ledger.get_entry(id) else {
                continue;
            };
            let evs = events.remove(&id).unwrap_or_default();
            if evs.iter().any(|&i| journal[i].seq > fresh_from_seq) {
                fresh.insert(id);
            }
            let origin = origin(&evs, &journal, closings.contains(&id));
            let mut accounts: Vec<AccountId> = Vec::new();
            for side in [Side::Debit, Side::Credit] {
                for l in e.lines().iter().filter(|l| l.side == side) {
                    if !accounts.contains(&l.account) {
                        accounts.push(l.account);
                    }
                }
            }
            let amount: Decimal = e
                .lines()
                .iter()
                .filter(|l| l.side == Side::Debit)
                .map(|l| l.amount.amount)
                .sum();
            let rec = receipts.remove(&id).unwrap_or_default();
            let vendor = ledger
                .entry_vendor(id)
                .and_then(|v| ledger.get_vendor(v))
                .map(|v| v.name.clone());
            // Receipts prove spending: hand-made entries (and recurring
            // drafts) that debit an expense or buy an asset from outside.
            // Invoices, payments, transfers, draws, closes, and schedules
            // carry their own evidence.
            let spends = e.lines().iter().any(|l| {
                l.side == Side::Debit
                    && ledger.chart().get(l.account).is_some_and(|a| {
                        a.account_type == AccountType::Expense
                            || (a.account_type == AccountType::Asset
                                && !cash_accounts.contains(&a.id)
                                && !a.name.to_lowercase().contains("receivable"))
                    })
            });
            let missing_receipt = matches!(origin, "Journal entry" | "Recurring")
                && spends
                && rec.is_empty()
                && amount > receipt_threshold;
            let cash_effect: Decimal = e
                .lines()
                .iter()
                .filter(|l| cash_accounts.contains(&l.account))
                .map(|l| match l.side {
                    Side::Debit => l.amount.amount,
                    Side::Credit => -l.amount.amount,
                })
                .sum();
            let party = vendor.clone().or_else(|| parties.get(&id).cloned());
            let mut haystack = format!(
                "{} {} #{} {}",
                e.description(),
                crate::fmt::money(amount),
                id,
                amount
            );
            for a in &accounts {
                if let Some(acct) = ledger.chart().get(*a) {
                    haystack.push_str(&format!(" {} {}", acct.code, acct.name));
                }
            }
            if let Some(v) = &party {
                haystack.push(' ');
                haystack.push_str(v);
            }
            entries.push(EntryInfo {
                id,
                date: e.date(),
                description: e.description().to_string(),
                status: e.status(),
                amount,
                accounts,
                receipts: rec,
                origin,
                events: evs,
                vendor,
                reversal_of: e.reversal_of(),
                reversed_by: reversed_by.get(&id).copied(),
                missing_receipt,
                cash_effect,
                party,
                haystack: haystack.to_lowercase(),
            });
        }
        entries.sort_by(|a, b| b.date.cmp(&a.date).then(b.id.cmp(&a.id)));
        let entry_index = entries.iter().enumerate().map(|(i, e)| (e.id, i)).collect();
        let mut books = Books {
            ledger,
            settings,
            data_dir: snap.data_dir().to_path_buf(),
            attachments,
            journal,
            head: snap.head_hash().to_string(),
            today,
            loaded_at: chrono::Utc::now(),
            entries,
            entry_index,
            cash_accounts,
            card_accounts,
            fresh,
            fresh_from_seq,
            receipt_threshold,
            attention: Vec::new(),
            kpis: Kpis::default(),
            integrity,
        };
        books.kpis = books.compute_kpis();
        books.attention = crate::attention::find(&books);
        Ok(books)
    }

    /// Why an entry needs a receipt, in words: "every entry needs a
    /// receipt" or "over the $25.00 receipt threshold".
    pub fn receipt_rule(&self) -> String {
        if self.receipt_threshold.is_zero() {
            "every entry needs a receipt".to_string()
        } else {
            format!(
                "over the {} receipt threshold",
                crate::fmt::dollars(self.receipt_threshold)
            )
        }
    }

    pub fn seq(&self) -> u64 {
        self.journal.len() as u64
    }

    pub fn entry(&self, id: EntryId) -> Option<&EntryInfo> {
        self.entry_index.get(&id).map(|&i| &self.entries[i])
    }

    pub fn account_label(&self, id: AccountId) -> String {
        self.ledger
            .chart()
            .get(id)
            .map(|a| format!("{} {}", a.code, a.name))
            .unwrap_or_else(|| format!("account {id}"))
    }

    pub fn account_name(&self, id: AccountId) -> String {
        self.ledger
            .chart()
            .get(id)
            .map(|a| a.name.clone())
            .unwrap_or_else(|| format!("account {id}"))
    }

    pub fn company_name(&self) -> &str {
        &self.settings.company.name
    }

    /// Cash balance at the end of each day it changed.
    pub fn cash_history(&self) -> Vec<(NaiveDate, Decimal)> {
        balance_history(&self.ledger, &self.cash_accounts)
    }

    /// Normal-side balance of one account at the end of `date`.
    pub fn balance_on(&self, account: AccountId, date: NaiveDate) -> Decimal {
        balance_series(&self.ledger, &[account], &[date])[0]
    }

    /// First day of the current month.
    pub fn month_start(&self) -> NaiveDate {
        NaiveDate::from_ymd_opt(self.today.year(), self.today.month(), 1).expect("valid")
    }

    /// Range of the last `n` months, ending with the current one.
    pub fn trailing_months(&self, n: u32) -> (NaiveDate, NaiveDate) {
        let start = self.month_start() - chrono::Months::new(n.saturating_sub(1));
        (start, self.today)
    }

    /// The fiscal year to date.
    pub fn fiscal_ytd(&self) -> (NaiveDate, NaiveDate) {
        let (start, _) = self.settings.company.fiscal_year_containing(self.today);
        (start, self.today)
    }

    fn compute_kpis(&self) -> Kpis {
        let l = &self.ledger;
        let today = self.today;
        let cash = balance_series(
            l,
            &self.cash_accounts,
            &[today - chrono::Days::new(30), today],
        );
        let bs = balance_sheet_as_of(l, today).ok();
        let receivables: Decimal = l
            .chart()
            .all()
            .filter(|a| {
                a.account_type == AccountType::Asset && a.name.to_lowercase().contains("receivable")
            })
            .map(|a| self.balance_on(a.id, today))
            .sum();
        let ms = self.month_start();
        let prev_start = ms - chrono::Months::new(1);
        let prev_end = ms - chrono::Days::new(1);
        let pl = |s, e| profit_and_loss_for_period(l, s, e, None).ok();
        let mtd = pl(ms, today);
        // Same days of last month: a fair comparison mid-month.
        let same_day = (prev_start + chrono::Days::new(u64::from(today.day() - 1))).min(prev_end);
        let prev = pl(prev_start, same_day);
        let (fy_start, _) = self.fiscal_ytd();
        let ytd = pl(fy_start, today);
        let three = pl(ms - chrono::Months::new(3), prev_end);
        let first_entry = self
            .entries
            .iter()
            .filter(|e| e.status == EntryStatus::Posted)
            .map(|e| e.date)
            .min();
        let monthly_burn = three.and_then(|p| {
            let loss = p.total_expenses - p.total_revenue;
            // Only months the books cover count toward the average.
            let months = match first_entry {
                Some(f) if f > ms - chrono::Months::new(3) => {
                    let m = (prev_end.year() * 12 + prev_end.month() as i32)
                        - (f.year() * 12 + f.month() as i32)
                        + 1;
                    m.max(0)
                }
                Some(_) => 3,
                None => 0,
            };
            (loss > Decimal::ZERO && months > 0).then(|| (loss / Decimal::from(months)).round_dp(2))
        });
        Kpis {
            cash: cash[1],
            cash_30d_ago: cash[0],
            liabilities: bs.as_ref().map(|b| b.total_liabilities).unwrap_or_default(),
            receivables,
            revenue_mtd: mtd.as_ref().map(|p| p.total_revenue).unwrap_or_default(),
            expenses_mtd: mtd.as_ref().map(|p| p.total_expenses).unwrap_or_default(),
            revenue_prev_month: prev.as_ref().map(|p| p.total_revenue).unwrap_or_default(),
            expenses_prev_month: prev.as_ref().map(|p| p.total_expenses).unwrap_or_default(),
            net_ytd: ytd
                .map(|p| p.total_revenue - p.total_expenses)
                .unwrap_or_default(),
            monthly_burn,
            first_entry,
        }
    }

    /// Blob store path for an attachment.
    pub fn blob_root(&self) -> PathBuf {
        self.data_dir.join("blobs")
    }
}

/// Customer or vendor names for entries created by invoices and bills and
/// their payments, from the journal.
fn parties(ledger: &Ledger, journal: &[JournalLine]) -> HashMap<EntryId, String> {
    let mut out = HashMap::new();
    let customer = |id: Option<u64>| {
        id.and_then(|c| ledger.get_customer(c))
            .map(|c| c.name.clone())
    };
    let vendor = |id: Option<u64>| {
        id.and_then(|v| ledger.get_vendor(v))
            .map(|v| v.name.clone())
    };
    let u = |p: &Value, k: &str| p.get(k).and_then(|v| v.as_u64());
    for line in journal {
        let p = &line.payload;
        let Some(entry) = u(p, "entry_id") else {
            continue;
        };
        let name = match line.line_type.as_str() {
            "invoice.created" => customer(u(p, "customer_id")),
            "invoice.payment_received" => customer(
                u(p, "invoice_id")
                    .and_then(|i| ledger.get_invoice(i))
                    .map(|i| i.customer_id),
            ),
            "bill.created" => vendor(u(p, "vendor_id")),
            "bill.paid" => vendor(
                u(p, "bill_id")
                    .and_then(|b| ledger.get_bill(b))
                    .map(|b| b.vendor_id),
            ),
            _ => None,
        };
        if let Some(n) = name {
            out.insert(entry, n);
        }
    }
    out
}

/// Entry ids a journal line refers to.
pub fn entry_refs(line: &JournalLine) -> Vec<EntryId> {
    let p = &line.payload;
    let mut out = Vec::new();
    let mut push = |v: Option<&Value>| {
        if let Some(id) = v.and_then(|v| v.as_u64())
            && !out.contains(&id)
        {
            out.push(id);
        }
    };
    push(p.get("entry_id"));
    push(p.get("closing_entry"));
    push(p.get("entry").and_then(|e| e.get("id")));
    if line.line_type.starts_with("entry.") {
        push(p.get("id"));
    }
    out
}

fn origin(events: &[usize], journal: &[JournalLine], closing: bool) -> &'static str {
    if closing {
        return "Period close";
    }
    let has = |t: &str| events.iter().any(|&i| journal[i].line_type == t);
    if has("entry.reversed") {
        "Reversal"
    } else if has("schedule.posted") {
        "Amortization"
    } else if has("asset.depreciation_posted") {
        "Depreciation"
    } else if has("asset.disposed") {
        "Asset disposal"
    } else if has("bill.created") {
        "Bill"
    } else if has("bill.paid") {
        "Bill payment"
    } else if has("invoice.created") {
        "Invoice"
    } else if has("invoice.payment_received") {
        "Customer payment"
    } else if has("payroll.run") {
        "Payroll"
    } else if has("payroll.remitted") || has("payroll.taxes_remitted") {
        "Payroll taxes"
    } else if has("recurring.generated") {
        "Recurring"
    } else {
        "Journal entry"
    }
}

/// Cash accounts: from `[books] cash_accounts`, else asset accounts whose
/// names say they hold cash.
fn cash_accounts(ledger: &Ledger, settings: &BooksSettings) -> Vec<AccountId> {
    if !settings.cash_accounts.is_empty() {
        return settings
            .cash_accounts
            .iter()
            .filter_map(|c| ledger.chart().find_by_code(c).map(|a| a.id))
            .collect();
    }
    ledger
        .chart()
        .all()
        .filter(|a| {
            let n = a.name.to_lowercase();
            a.account_type == AccountType::Asset
                && !a.placeholder
                && ["cash", "bank", "checking", "savings", "money market"]
                    .iter()
                    .any(|w| n.contains(w))
        })
        .map(|a| a.id)
        .collect()
}
