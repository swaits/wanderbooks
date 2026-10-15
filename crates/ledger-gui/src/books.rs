//! The view model: one immutable picture of the books, rebuilt whenever the
//! journal grows. Built off the UI thread from a [`Snapshot`]; the UI only
//! reads it. Every number here comes from `ledger-core`; this module only
//! indexes and arranges.

use chrono::{Datelike, NaiveDate};
use ledger_config::BooksSettings;
use ledger_core::analysis::{balance_history, balance_series};
use ledger_core::reports::{balance_sheet_as_of, profit_and_loss_for_period};
use ledger_core::{
    AccountId, AccountType, AssetId, AssetRegister, AssetStatus, BillId, EntryId, EntryStatus,
    InvoiceId, Ledger, Party, PartyId, PayrollRunId, Side, Vendor1099Line, fixed_asset_register,
};
use ledger_store::{Attachment, JournalLine, Reconciliation, Snapshot, Statement, Subject};
use rust_decimal::Decimal;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;

use crate::nav::Route;

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
    /// Indexes into [`Books::attachments`] of its current receipts: the
    /// files filed on it and the files linked to it (`attachment.linked`
    /// with an entry subject), as the server's receipt policy counts them.
    pub receipts: Vec<usize>,
    /// How the entry came to be, in words.
    pub origin: &'static str,
    /// Indexes into [`Books::journal`] of every event that touched it.
    pub events: Vec<usize>,
    pub vendor: Option<String>,
    pub reversal_of: Option<EntryId>,
    pub reversed_by: Option<EntryId>,
    /// Covered by the receipt policy (the server's rule) and has no receipt.
    pub missing_receipt: bool,
    /// Net effect on cash accounts: positive in, negative out, zero if the
    /// entry doesn't touch cash.
    pub cash_effect: Decimal,
    /// The counterparty's name: its directory party's label (see
    /// [`PartyInfo`] for which entries are a party's), else its vendor's
    /// or invoice customer's name.
    pub party: Option<String>,
    /// The directory party the entry is with (see [`PartyInfo`]).
    pub party_id: Option<PartyId>,
    /// Lower-cased text for search: description, accounts, vendor, amount.
    pub haystack: String,
}

/// One fixed asset's trail through the journal and the entries.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AssetTrail {
    /// Depreciation the books carry, by month ("YYYY-MM"): the entry and
    /// its amount, from `asset.depreciation_posted`.
    pub depreciation: BTreeMap<String, (EntryId, Decimal)>,
    /// The disposal entry, from `asset.disposed`.
    pub disposal_entry: Option<EntryId>,
    /// The entry that put it on the books, when one can be told: posted,
    /// debiting its cost account exactly its cost, dated its in-service
    /// date (else the nearest such entry within 31 days), and not already
    /// claimed by an earlier asset.
    pub purchase_entry: Option<EntryId>,
    /// Indexes into [`Books::journal`] of every `asset.*` event about it.
    pub events: Vec<usize>,
}

impl AssetTrail {
    /// Every entry that touches the asset: purchase, depreciation (by
    /// month), disposal.
    pub fn entries(&self) -> Vec<EntryId> {
        let mut out: Vec<EntryId> = self.purchase_entry.into_iter().collect();
        out.extend(self.depreciation.values().map(|(e, _)| *e));
        out.extend(self.disposal_entry);
        out
    }

    /// Depreciation the books carry for it, summed exactly.
    pub fn posted(&self) -> Decimal {
        self.depreciation.values().map(|(_, a)| *a).sum()
    }
}

/// One party's dealings with the company: its entries, documents, and
/// this year's totals, computed once per refresh.
///
/// ```text
/// GOAL: answer "who is this, and what has the company done with them?"
///       with the books' own numbers.
/// SOURCE: docs/parties-and-attachments.md (Parties → Viewer); the
///         module's rule (every number from ledger-core; this indexes and
///         sums); DECISION (the year and the payment accounts, below).
/// DEFINITIONS:
///   The party's entries: Ledger::entry_party names it (core tags hand
///     tags, vendor tags, bills, invoices, their payments, reversals).
///   The year: the calendar year of `today`, Jan 1 through today (the
///     year 1099s are counted in).
///   Money: ledger_core::parties::activity with Books::cash_accounts
///     (the same numbers books.party reports).
/// INVARIANTS:
///   1. entries: every entry that is the party's, drafts included, in
///      Books::entries order (newest first).
///   2. paid_ytd = activity's paid − received over the year, exactly.
///      Positive: paid to them, net; negative: received from them, net.
///   3. invoiced_ytd = Σ Invoice::total() over its customer record's
///      invoices dated in the year; billed_ytd the same for its vendor
///      record's bills.
///   4. open_invoices = Σ Invoice::balance() over its customer record's
///      invoices; open_bills = Σ Bill::balance() over its vendor record's
///      bills (any date: what is still owed today).
///   5. form_1099: its vendor record's line in
///      vendor_1099_report_with(ledger, year, None, cash and financing
///      accounts), when it has a vendor
///      record in that report and the report knows the year's threshold
///      (threshold_1099 is then that threshold).
///   6. payroll_runs: the runs with a line whose employee is the party,
///      or (older runs, no party on the line) whose employee_name is the
///      party's name ignoring case and surrounding space.
///   7. events: indexes into Books::journal of party.* events with its
///      id, entry.party_set naming it, and its records' vendor.added,
///      customer.added, vendor.1099_set, and entry.vendor_set events.
/// ```
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PartyInfo {
    pub id: PartyId,
    pub entries: Vec<EntryId>,
    pub paid_ytd: Decimal,
    pub invoiced_ytd: Decimal,
    pub billed_ytd: Decimal,
    pub open_invoices: Decimal,
    pub open_bills: Decimal,
    pub invoices: Vec<InvoiceId>,
    pub bills: Vec<BillId>,
    pub payroll_runs: Vec<PayrollRunId>,
    pub form_1099: Option<Vendor1099Line>,
    pub threshold_1099: Option<Decimal>,
    pub events: Vec<usize>,
}

/// Depreciation due and not yet in the books.
///
/// ```text
/// GOAL: say when monthly depreciation hasn't been run, without crying
///       wolf over the month still under way.
/// SOURCE: docs/spec/rules.md B-15 (monthly straight-line depreciation);
///         CONVENTION (a month's depreciation is posted when the month
///         ends: ledger-store dates it the month's last day).
/// INVARIANTS:
///   1. through is as_of when as_of is a month's last day, else the last
///      day of the month before as_of's.
///   2. amount is fixed_asset_register(ledger, through)
///      .unposted_depreciation(), exactly.
///   3. periods: the months ("YYYY-MM", ascending, distinct) that some
///      asset held on `through` has in its schedule through `through`
///      and no depreciation posted for.
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct DepreciationDue {
    pub through: NaiveDate,
    pub amount: Decimal,
    pub periods: Vec<String>,
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
    /// `[books] financing_accounts`, resolved (empty when not set).
    pub financing_accounts: Vec<AccountId>,
    /// `[books] investing_accounts`, resolved (empty when not set).
    pub investing_accounts: Vec<AccountId>,
    pub card_accounts: Vec<AccountId>,
    /// Entries created or changed by the most recent refresh.
    pub fresh: HashSet<EntryId>,
    /// Journal seq before the most recent refresh.
    pub fresh_from_seq: u64,
    pub receipt_threshold: Decimal,
    pub attention: Vec<Attention>,
    pub kpis: Kpis,
    pub integrity: Integrity,
    /// Each registered asset's trail, by asset id.
    pub asset_trails: HashMap<AssetId, AssetTrail>,
    /// The asset each depreciation, disposal, or purchase entry is about.
    pub entry_assets: HashMap<EntryId, AssetId>,
    /// Every party's dealings with the company, by party id.
    pub parties: BTreeMap<PartyId, PartyInfo>,
    /// Every attachment's subjects (see [`Books::links_of`]), by
    /// attachment id; withdrawn attachments keep theirs.
    pub links: BTreeMap<u64, Vec<Subject>>,
    /// Recorded bank and card statements, by period end.
    pub statements: Vec<Statement>,
    /// Reconciliations, voided ones included, by date then id.
    pub reconciliations: Vec<Reconciliation>,
}

/// A span of dates a party's money is summed over (the party page's and
/// the directory's period picker).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PartyPeriod {
    /// January 1 of today's year through today.
    #[default]
    ThisYear,
    /// Last calendar year.
    LastYear,
    /// One calendar year (through today, if it is this one).
    Year(i32),
    /// The books' first entry through today.
    AllTime,
}

/// What one party's posted entries moved over a span, and what for.
///
/// ```text
/// GOAL: answer "how much did we spend with them, and how much did we
///       get back?" (Stephen, 2026-10-15) with the books' own numbers.
/// SOURCE: ledger_core::parties::activity (paid, received) and
///         ledger_core::parties::entry_flow (what for), with
///         Books::cash_accounts, as PartyInfo uses them.
/// INVARIANTS:
///   1. paid, received: activity(ledger, party, span, cash accounts)'s,
///      exactly.
///   2. net() = paid − received.
///   3. what_for: over the party's posted entries dated in the span, the
///      sum by account of each entry_flow's what_for (a liability paid
///      down, like a reimbursement, appears as what the money paid for),
///      the accounts whose sum is zero left out, largest magnitude first
///      (then by account id). Positive: what money paid for; negative:
///      where money received came from (or, on payables and
///      receivables, what is still owed).
///   4. Σ what_for = net(), exactly: each entry_flow's what_for sums to
///      its net (a reversal reverses both).
///   5. entries: how many of the party's entries are posted and dated
///      in the span.
/// ```
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PartyFlow {
    pub paid: Decimal,
    pub received: Decimal,
    pub what_for: Vec<(AccountId, Decimal)>,
    pub entries: usize,
}

impl PartyFlow {
    /// Paid out less received in.
    pub fn net(&self) -> Decimal {
        self.paid - self.received
    }
}

/// One month of a party's money, for the out-versus-in chart.
#[derive(Clone, Debug, PartialEq)]
pub struct PartyMonth {
    /// The month's first day.
    pub month: NaiveDate,
    /// activity's paid over the month (clipped to the span).
    pub paid: Decimal,
    /// activity's received over the month (clipped to the span).
    pub received: Decimal,
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
///
/// ```text
/// GOAL: the overview's numbers are the books' numbers.
/// SOURCE: the module's rule ("every number here comes from ledger-core;
///         this module only indexes and arranges"); DECISION (the
///         comparisons and the burn estimate, as defined here).
/// INVARIANTS (all at `today`; m0 = the first of today's month):
///   1. cash, cash_30d_ago: the cash accounts' balances (normal side) at
///      the end of today and of today - 30 days.
///   2. liabilities: the balance sheet's total liabilities at today.
///   3. receivables: the balances of the asset accounts whose name says
///      "receivable".
///   4. revenue_mtd, expenses_mtd: the P&L from m0 through today.
///      revenue_prev_month, expenses_prev_month: the P&L from the first
///      of last month through the same day of it (its last day, if
///      shorter).
///   5. net_ytd: revenue minus expenses from the fiscal year's first day
///      (ledger_config) through today.
///   6. monthly_burn: over the three full months before m0, expenses
///      minus revenue, divided by the months of those the books cover
///      (from the first posted entry's month); Some only when positive,
///      rounded to the cent half away from zero.
///   7. first_entry: the earliest posted entry's date.
/// ```
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
        // The server's own rule (config.json, else wanderbooks.toml), so
        // the viewer flags exactly what the server would refuse.
        let policy = ledger_config::effective_receipt_policy(
            &snap.data_dir().join("config.json"),
            &settings,
        )?;
        let receipt_threshold = policy.threshold();
        let cash_accounts = cash_accounts(&ledger, &settings);
        let codes = |cs: &[String]| -> Vec<AccountId> {
            cs.iter()
                .filter_map(|c| ledger.chart().find_by_code(c).map(|a| a.id))
                .collect()
        };
        let financing_accounts = codes(&settings.financing_accounts);
        let investing_accounts = codes(&settings.investing_accounts);
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
        let link_rows = snap.attachment_links().map_err(|e| e.to_string())?;
        let mut links: BTreeMap<u64, Vec<Subject>> = BTreeMap::new();
        for a in &attachments {
            if let Some(e) = a.entry_id {
                links.entry(a.id).or_default().push(Subject::Entry(e));
            }
        }
        for (att, sub) in &link_rows {
            links.entry(*att).or_default().push(*sub);
        }
        // An entry's receipts: filed on it, or linked to it (the server's
        // receipt policy counts both).
        let mut receipts: HashMap<EntryId, Vec<usize>> = HashMap::new();
        for (i, a) in attachments.iter().enumerate() {
            if a.withdrawn_at.is_some() {
                continue;
            }
            for sub in links.get(&a.id).into_iter().flatten() {
                if let Subject::Entry(e) = sub {
                    let r = receipts.entry(*e).or_default();
                    if !r.contains(&i) {
                        r.push(i);
                    }
                }
            }
        }
        let statements = snap.statements(None).map_err(|e| e.to_string())?;
        let reconciliations = snap.reconciliations(None).map_err(|e| e.to_string())?;
        let fresh_from_seq = prev_seq.unwrap_or(journal.len() as u64);
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
            // The receipt policy covers what is posted by hand (journal
            // entries and recurring drafts); invoices, payments, closes,
            // depreciation, and schedules carry their own evidence, and a
            // reversal leans on its original's.
            let missing_receipt = matches!(origin, "Journal entry" | "Recurring")
                && e.reversal_of().is_none()
                && rec.is_empty()
                && policy.receipt_required(&ledger, e);
            let cash_effect: Decimal = e
                .lines()
                .iter()
                .filter(|l| cash_accounts.contains(&l.account))
                .map(|l| match l.side {
                    Side::Debit => l.amount.amount,
                    Side::Credit => -l.amount.amount,
                })
                .sum();
            // Core tags every entry with its counterparty (hand tags,
            // vendor tags, bills, invoices, their payments, reversals).
            let party_id = ledger.entry_party(id);
            let party = party_id
                .and_then(|p| ledger.get_party(p))
                .map(|p| p.label().to_string())
                .or_else(|| vendor.clone());
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
            if let Some(p) = party_id.and_then(|p| ledger.get_party(p))
                && p.display_name.is_some()
            {
                haystack.push(' ');
                haystack.push_str(&p.name);
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
                party_id,
                haystack: haystack.to_lowercase(),
            });
        }
        entries.sort_by(|a, b| b.date.cmp(&a.date).then(b.id.cmp(&a.id)));
        let entry_index = entries.iter().enumerate().map(|(i, e)| (e.id, i)).collect();
        let asset_trails = asset_trails(&ledger, &journal);
        let mut entry_assets = HashMap::new();
        for (&a, t) in &asset_trails {
            for e in t.entries() {
                entry_assets.insert(e, a);
            }
        }
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
            financing_accounts,
            investing_accounts,
            card_accounts,
            fresh,
            fresh_from_seq,
            receipt_threshold,
            attention: Vec::new(),
            kpis: Kpis::default(),
            integrity,
            asset_trails,
            entry_assets,
            parties: BTreeMap::new(),
            links,
            statements,
            reconciliations,
        };
        books.parties = books.party_infos();
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

    /// Cash balance at the end of each day a cash account moved.
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
            (loss > Decimal::ZERO && months > 0).then(|| {
                (loss / Decimal::from(months))
                    .round_dp_with_strategy(2, rust_decimal::RoundingStrategy::MidpointAwayFromZero)
            })
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

    /// The fixed asset register on `as_of`, exactly as ledger-core
    /// computes it.
    pub fn asset_register(&self, as_of: NaiveDate) -> AssetRegister {
        fixed_asset_register(&self.ledger, as_of)
    }

    /// An asset's trail (empty for an unknown asset).
    pub fn asset_trail(&self, id: AssetId) -> AssetTrail {
        self.asset_trails.get(&id).cloned().unwrap_or_default()
    }

    /// "WL-0001 MacBook Pro" (or just the description, untagged).
    pub fn asset_label(&self, id: AssetId) -> String {
        match self.ledger.get_asset(id) {
            Some(a) => match &a.tag {
                Some(t) => format!("{t} {}", a.description),
                None => a.description.clone(),
            },
            None => format!("asset {id}"),
        }
    }

    /// One party's dealings (None for an unknown party).
    pub fn party(&self, id: PartyId) -> Option<&PartyInfo> {
        self.parties.get(&id)
    }

    /// A party's label (its display name, else its name).
    pub fn party_label(&self, id: PartyId) -> String {
        self.ledger
            .get_party(id)
            .map(|p| p.label().to_string())
            .unwrap_or_else(|| format!("party {id}"))
    }

    /// Whether a party files a 1099: it has a 1099 role (contractor,
    /// landlord), or its vendor record is marked for a 1099.
    pub fn files_1099(&self, p: &Party) -> bool {
        p.roles.iter().any(|r| r.gets_1099())
            || p.vendor
                .and_then(|v| self.ledger.get_vendor(v))
                .is_some_and(|v| v.form_1099.is_some())
    }

    /// Every party's [`PartyInfo`].
    fn party_infos(&self) -> BTreeMap<PartyId, PartyInfo> {
        let l = &self.ledger;
        let year = self.today.year();
        let year_start = NaiveDate::from_ymd_opt(year, 1, 1).expect("valid");
        let in_year = |d: NaiveDate| d >= year_start && d <= self.today;
        let mut out: BTreeMap<PartyId, PartyInfo> = l
            .parties()
            .map(|p| {
                (
                    p.id,
                    PartyInfo {
                        id: p.id,
                        ..Default::default()
                    },
                )
            })
            .collect();
        for (id, info) in out.iter_mut() {
            let a = ledger_core::parties::activity(
                l,
                *id,
                (year_start, self.today),
                &self.cash_accounts,
            );
            info.entries = self
                .entries
                .iter()
                .filter(|e| a.entries.contains(&e.id))
                .map(|e| e.id)
                .collect();
            info.paid_ytd = a.paid - a.received;
        }
        for inv in l.invoices() {
            let id = inv.id;
            if let Some(info) = l
                .party_of_customer(inv.customer_id)
                .and_then(|p| out.get_mut(&p))
            {
                info.invoices.push(id);
                info.open_invoices += inv.balance();
                if in_year(inv.date) {
                    info.invoiced_ytd += inv.total();
                }
            }
        }
        for bill in l.bills() {
            let id = bill.id;
            if let Some(info) = l
                .party_of_vendor(bill.vendor_id)
                .and_then(|p| out.get_mut(&p))
            {
                info.bills.push(id);
                info.open_bills += bill.balance();
                if in_year(bill.date) {
                    info.billed_ytd += bill.total();
                }
            }
        }
        let sources = ledger_core::PaymentAccounts {
            cash: self.cash_accounts.clone(),
            financing: self.financing_accounts.clone(),
        };
        if let Ok(r) = ledger_core::vendor_1099_report_with(l, year, None, &sources) {
            for v in &r.vendors {
                if let Some(info) = l.party_of_vendor(v.vendor_id).and_then(|p| out.get_mut(&p)) {
                    info.form_1099 = Some(v.clone());
                    info.threshold_1099 = Some(r.threshold);
                }
            }
        }
        for run in l.list_payroll_runs() {
            for p in l.parties() {
                let names = |x: &ledger_core::PayrollLine| match x.employee {
                    Some(e) => e == p.id,
                    None => {
                        ledger_core::parties::name_key(&x.employee_name)
                            == ledger_core::parties::name_key(&p.name)
                    }
                };
                if run.lines.iter().any(names)
                    && let Some(info) = out.get_mut(&p.id)
                    && !info.payroll_runs.contains(&run.id)
                {
                    info.payroll_runs.push(run.id);
                }
            }
        }
        let u = |v: &Value, k: &str| v.get(k).and_then(|v| v.as_u64());
        for (i, line) in self.journal.iter().enumerate() {
            let p = &line.payload;
            let t = line.line_type.as_str();
            let party = if t.starts_with("party.") {
                u(p, "id")
            } else {
                match t {
                    "entry.party_set" => u(p, "party_id"),
                    "vendor.added" => u(p, "id").and_then(|v| l.party_of_vendor(v)),
                    "customer.added" => u(p, "id").and_then(|c| l.party_of_customer(c)),
                    "vendor.1099_set" | "entry.vendor_set" => {
                        u(p, "vendor_id").and_then(|v| l.party_of_vendor(v))
                    }
                    _ => None,
                }
            };
            if let Some(info) = party.and_then(|p| out.get_mut(&p)) {
                info.events.push(i);
            }
        }
        out
    }

    /// Depreciation due through the last finished month on or before
    /// `as_of` and not yet posted (see [`DepreciationDue`]).
    pub fn depreciation_due(&self, as_of: NaiveDate) -> DepreciationDue {
        let through = if as_of.succ_opt().is_some_and(|d| d.day() == 1) {
            as_of
        } else {
            NaiveDate::from_ymd_opt(as_of.year(), as_of.month(), 1).expect("valid")
                - chrono::Days::new(1)
        };
        let amount = self.asset_register(through).unposted_depreciation();
        let mut periods = std::collections::BTreeSet::new();
        for a in self
            .ledger
            .registered_assets()
            .filter(|a| a.purchase_date <= through)
        {
            if matches!(a.status, AssetStatus::Disposed { date, .. } if date <= through) {
                continue;
            }
            let posted = self.asset_trails.get(&a.id);
            for l in ledger_core::depreciation_schedule(a, through) {
                if !posted.is_some_and(|t| t.depreciation.contains_key(&l.period)) {
                    periods.insert(l.period);
                }
            }
        }
        DepreciationDue {
            through,
            amount,
            periods: periods.into_iter().collect(),
        }
    }

    // ------------------------------------------------------------------
    // Attachment links, statements, reconciliations
    // ------------------------------------------------------------------

    /// One attachment by id.
    pub fn attachment(&self, id: u64) -> Option<&Attachment> {
        self.attachments.iter().find(|a| a.id == id)
    }

    /// Every subject an attachment is linked to: its filing entry (a
    /// receipt filed on an entry) first, then its `attachment.linked`
    /// subjects, as `Store::attachment_links` lists them.
    pub fn links_of(&self, attachment_id: u64) -> &[Subject] {
        self.links.get(&attachment_id).map_or(&[], |v| v.as_slice())
    }

    /// The current (not withdrawn) attachments linked to any of
    /// `subjects`, each once, by id.
    ///
    /// ```text
    /// GOAL: every subject's page shows the files that are evidence of it.
    /// SOURCE: docs/parties-and-attachments.md (Attachments: an
    ///         attachment links to any number of subjects; an entry's are
    ///         those filed on it and those linked to it);
    ///         Store::attachments_for, which this mirrors.
    /// INVARIANTS:
    ///   1. a is listed iff it isn't withdrawn and links_of(a) shares a
    ///      subject with `subjects`.
    ///   2. No attachment twice; ascending id.
    /// ```
    pub fn documents_for(&self, subjects: &[Subject]) -> Vec<&Attachment> {
        self.attachments
            .iter()
            .filter(|a| a.withdrawn_at.is_none())
            .filter(|a| self.links_of(a.id).iter().any(|s| subjects.contains(s)))
            .collect()
    }

    /// The subjects whose documents an entry's page shows: the entry, and
    /// the invoice, bill, or payroll run it records (those have no page
    /// of their own).
    pub fn entry_subjects(&self, id: EntryId) -> Vec<Subject> {
        let l = &self.ledger;
        let mut out = vec![Subject::Entry(id)];
        out.extend(
            l.invoices()
                .into_iter()
                .filter(|i| i.entry_id == id)
                .map(|i| Subject::Invoice(i.id)),
        );
        out.extend(
            l.bills()
                .into_iter()
                .filter(|b| b.entry_id == id)
                .map(|b| Subject::Bill(b.id)),
        );
        out.extend(
            l.list_payroll_runs()
                .into_iter()
                .filter(|r| r.entry_id == id)
                .map(|r| Subject::PayrollRun(r.id)),
        );
        out
    }

    /// A subject in words: "#12 Adobe (…)", "1000 Operating Checking",
    /// "WL-0001 MacBook Pro", a party's label, "Invoice 3", "Period 2025",
    /// "Reconciliation of 1000 Operating Checking through Sep 30, 2026",
    /// "the company".
    pub fn subject_label(&self, s: Subject) -> String {
        let l = &self.ledger;
        match s {
            Subject::Entry(e) => self
                .entry(e)
                .map(|x| format!("#{} {}", x.id, x.description))
                .unwrap_or_else(|| format!("#{e}")),
            Subject::Account(a) => self.account_label(a),
            Subject::Asset(a) => self.asset_label(a),
            Subject::Party(p) => self.party_label(p),
            Subject::Invoice(i) => format!("Invoice {i}"),
            Subject::Bill(i) => format!("Bill {i}"),
            Subject::Period(i) => l
                .list_periods()
                .iter()
                .find(|p| p.id == i)
                .map(|p| format!("Period {}", p.name))
                .unwrap_or_else(|| format!("Period {i}")),
            Subject::PayrollRun(i) => match l.get_payroll_run(i) {
                Some(r) => format!("Payroll {}", crate::fmt::date(r.date)),
                None => format!("Payroll run {i}"),
            },
            Subject::Schedule(i) => l
                .get_schedule(i)
                .map(|x| x.description.clone())
                .unwrap_or_else(|| format!("Schedule {i}")),
            Subject::Reconciliation(i) => match self.reconciliation(i) {
                Some(r) => format!(
                    "Reconciliation of {} through {}",
                    self.account_label(r.account_id),
                    crate::fmt::date(r.statement_date)
                ),
                None => format!("Reconciliation {i}"),
            },
            Subject::Company => "the company".into(),
        }
    }

    /// Where a subject opens: its page, or (an invoice, bill, or payroll
    /// run) the entry that records it, or (a period, schedule, the
    /// company) the company page.
    pub fn subject_route(&self, s: Subject) -> Option<Route> {
        let l = &self.ledger;
        Some(match s {
            Subject::Entry(e) => Route::Entry(e),
            Subject::Account(a) => Route::Account(a),
            Subject::Asset(a) => Route::Asset(a),
            Subject::Party(p) => Route::Party(p),
            Subject::Invoice(i) => Route::Entry(l.get_invoice(i)?.entry_id),
            Subject::Bill(i) => Route::Entry(l.get_bill(i)?.entry_id),
            Subject::PayrollRun(i) => Route::Entry(l.get_payroll_run(i)?.entry_id),
            Subject::Reconciliation(i) => Route::Reconciliation(i),
            Subject::Period(_) | Subject::Schedule(_) | Subject::Company => Route::Company,
        })
    }

    /// The statement an attachment records, if any.
    pub fn statement(&self, attachment_id: u64) -> Option<&Statement> {
        self.statements
            .iter()
            .find(|s| s.attachment_id == attachment_id)
    }

    /// One account's recorded statements, by period end.
    pub fn statements_for(&self, account: AccountId) -> Vec<&Statement> {
        self.statements
            .iter()
            .filter(|s| s.account_id == account)
            .collect()
    }

    /// One reconciliation by id.
    pub fn reconciliation(&self, id: u64) -> Option<&Reconciliation> {
        self.reconciliations.iter().find(|r| r.id == id)
    }

    /// One account's reconciliations, voided ones included, by date.
    pub fn reconciliations_for(&self, account: AccountId) -> Vec<&Reconciliation> {
        self.reconciliations
            .iter()
            .filter(|r| r.account_id == account)
            .collect()
    }

    /// The latest statement date an account is reconciled through: the
    /// newest reconciliation of it not voided (None: never reconciled).
    pub fn reconciled_through(&self, account: AccountId) -> Option<NaiveDate> {
        self.reconciliations
            .iter()
            .filter(|r| r.account_id == account && r.voided.is_none())
            .map(|r| r.statement_date)
            .max()
    }

    /// The last day of the month before today's.
    pub fn last_month_end(&self) -> NaiveDate {
        self.month_start() - chrono::Days::new(1)
    }

    /// The accounts the books expect to reconcile monthly: the cash
    /// accounts, then the card accounts, each once.
    pub fn reconcilable_accounts(&self) -> Vec<AccountId> {
        let mut out = self.cash_accounts.clone();
        for a in &self.card_accounts {
            if !out.contains(a) {
                out.push(*a);
            }
        }
        out
    }

    /// When an account is due a reconciliation: the date it should be
    /// reconciled through, else None.
    ///
    /// ```text
    /// GOAL: the monthly close leaves every bank and card account
    ///       reconciled through last month's end (docs/parties-and-
    ///       attachments.md, Reconciliations: "an account with activity
    ///       not reconciled through last month's end").
    /// SOURCE: that rule; CONVENTION (a bank or card statement closes
    ///         monthly; AGENTS.md's monthly close reconciles the bank).
    /// INVARIANTS:
    ///   1. Some(last_month_end()) iff the account has a posted line
    ///      dated on or before last_month_end() (activity to reconcile)
    ///      and reconciled_through(account) is None or earlier than
    ///      last_month_end().
    ///   2. None otherwise (including activity only this month).
    /// ```
    pub fn reconciliation_due(&self, account: AccountId) -> Option<NaiveDate> {
        let through = self.last_month_end();
        let active = self
            .ledger
            .posted_entries()
            .any(|e| e.date() <= through && e.lines().iter().any(|l| l.account == account));
        (active && self.reconciled_through(account).is_none_or(|d| d < through)).then_some(through)
    }

    // ------------------------------------------------------------------
    // A party's money over a span
    // ------------------------------------------------------------------

    /// The dates a [`PartyPeriod`] covers: a year is January 1 to
    /// December 31, cut at today; all time starts at the earliest entry
    /// in the books (today when there is none).
    pub fn party_span(&self, period: PartyPeriod) -> (NaiveDate, NaiveDate) {
        let year = |y: i32| {
            (
                NaiveDate::from_ymd_opt(y, 1, 1).expect("valid"),
                NaiveDate::from_ymd_opt(y, 12, 31)
                    .expect("valid")
                    .min(self.today),
            )
        };
        match period {
            PartyPeriod::ThisYear => year(self.today.year()),
            PartyPeriod::LastYear => year(self.today.year() - 1),
            PartyPeriod::Year(y) => year(y),
            PartyPeriod::AllTime => {
                let first = self
                    .entries
                    .iter()
                    .map(|e| e.date)
                    .min()
                    .unwrap_or(self.today)
                    .min(self.today);
                (first, self.today)
            }
        }
    }

    /// The calendar year a period's 1099 amount is for (all time: this
    /// year's).
    pub fn party_period_year(&self, period: PartyPeriod) -> i32 {
        match period {
            PartyPeriod::LastYear => self.today.year() - 1,
            PartyPeriod::Year(y) => y,
            PartyPeriod::ThisYear | PartyPeriod::AllTime => self.today.year(),
        }
    }

    /// A period in words: "This year (2026)", "Last year (2025)", "2024",
    /// "All time".
    pub fn party_period_label(&self, period: PartyPeriod) -> String {
        let y = self.today.year();
        match period {
            PartyPeriod::ThisYear => format!("This year ({y})"),
            PartyPeriod::LastYear => format!("Last year ({})", y - 1),
            PartyPeriod::Year(x) => x.to_string(),
            PartyPeriod::AllTime => "All time".into(),
        }
    }

    /// The periods to offer for a party (None: for the directory): this
    /// year, last year, every other year with a posted entry (the
    /// party's, or any for the directory), newest first, then all time.
    pub fn party_periods(&self, party: Option<PartyId>) -> Vec<PartyPeriod> {
        let y = self.today.year();
        let mut years: Vec<i32> = self
            .entries
            .iter()
            .filter(|e| e.status == EntryStatus::Posted && e.date <= self.today)
            .filter(|e| party.is_none_or(|p| e.party_id == Some(p)))
            .map(|e| e.date.year())
            .filter(|&x| x < y - 1)
            .collect();
        years.sort_unstable_by(|a, b| b.cmp(a));
        years.dedup();
        let mut out = vec![PartyPeriod::ThisYear, PartyPeriod::LastYear];
        out.extend(years.into_iter().map(PartyPeriod::Year));
        out.push(PartyPeriod::AllTime);
        out
    }

    /// What a party's posted entries moved over `span` (see
    /// [`PartyFlow`]). `with_breakdown` false leaves what_for empty (the
    /// directory needs only the totals).
    pub fn party_flow(
        &self,
        party: PartyId,
        span: (NaiveDate, NaiveDate),
        with_breakdown: bool,
    ) -> PartyFlow {
        let l = &self.ledger;
        let a = ledger_core::parties::activity(l, party, span, &self.cash_accounts);
        let posted: Vec<&ledger_core::JournalEntry> = a
            .entries
            .iter()
            .filter_map(|&id| l.get_entry(id))
            .filter(|e| {
                e.status() == EntryStatus::Posted && e.date() >= span.0 && e.date() <= span.1
            })
            .collect();
        let mut what_for = Vec::new();
        if with_breakdown {
            let money = ledger_core::parties::money_accounts(l, &self.cash_accounts);
            let mut by: BTreeMap<AccountId, Decimal> = BTreeMap::new();
            for e in &posted {
                for (account, amount) in ledger_core::parties::entry_flow(e, &money).what_for {
                    *by.entry(account).or_default() += amount;
                }
            }
            what_for = by.into_iter().filter(|(_, v)| !v.is_zero()).collect();
            what_for.sort_by(|x, y| y.1.abs().cmp(&x.1.abs()).then(x.0.cmp(&y.0)));
        }
        PartyFlow {
            paid: a.paid,
            received: a.received,
            what_for,
            entries: posted.len(),
        }
    }

    /// A party's 1099 worksheet line for a calendar year and that year's
    /// threshold: ledger-core's vendor_1099_report_with (cash and
    /// financing accounts) line for its vendor record; None without a
    /// vendor record in the report, or when the year's threshold isn't
    /// known.
    pub fn party_1099(&self, party: PartyId, year: i32) -> Option<(Vendor1099Line, Decimal)> {
        let l = &self.ledger;
        let vendor = l.get_party(party)?.vendor?;
        let sources = ledger_core::PaymentAccounts {
            cash: self.cash_accounts.clone(),
            financing: self.financing_accounts.clone(),
        };
        let r = ledger_core::vendor_1099_report_with(l, year, None, &sources).ok()?;
        let line = r.vendors.iter().find(|v| v.vendor_id == vendor)?.clone();
        Some((line, r.threshold))
    }

    /// A party's money month by month over `span`: every month the span
    /// touches, each month's activity clipped to the span (so the months
    /// sum to the span's paid and received exactly).
    pub fn party_months(&self, party: PartyId, span: (NaiveDate, NaiveDate)) -> Vec<PartyMonth> {
        let mut out = Vec::new();
        let mut m = NaiveDate::from_ymd_opt(span.0.year(), span.0.month(), 1).expect("valid");
        while m <= span.1 {
            let end = (m + chrono::Months::new(1) - chrono::Days::new(1)).min(span.1);
            let start = m.max(span.0);
            let a = ledger_core::parties::activity(
                &self.ledger,
                party,
                (start, end),
                &self.cash_accounts,
            );
            out.push(PartyMonth {
                month: m,
                paid: a.paid,
                received: a.received,
            });
            m = m + chrono::Months::new(1);
        }
        out
    }

    /// Blob store path for an attachment.
    pub fn blob_root(&self) -> PathBuf {
        self.data_dir.join("blobs")
    }
}

/// Every registered asset's trail, from the journal (depreciation,
/// disposal, its own events) and the entries (the purchase).
fn asset_trails(ledger: &Ledger, journal: &[JournalLine]) -> HashMap<AssetId, AssetTrail> {
    let mut out: HashMap<AssetId, AssetTrail> = ledger
        .registered_assets()
        .map(|a| (a.id, AssetTrail::default()))
        .collect();
    let u = |p: &Value, k: &str| p.get(k).and_then(|v| v.as_u64());
    for (i, line) in journal.iter().enumerate() {
        let p = &line.payload;
        let asset = match line.line_type.as_str() {
            "asset.registered" => u(p, "id"),
            t if t.starts_with("asset.") => u(p, "asset_id"),
            _ => None,
        };
        let Some(t) = asset.and_then(|a| out.get_mut(&a)) else {
            continue;
        };
        t.events.push(i);
        match line.line_type.as_str() {
            "asset.depreciation_posted" => {
                if let (Some(period), Some(entry), Some(amount)) = (
                    p.get("period").and_then(|v| v.as_str()),
                    u(p, "entry_id"),
                    p.get("amount")
                        .and_then(|v| v.as_str())
                        .and_then(|s| s.parse::<Decimal>().ok()),
                ) {
                    t.depreciation.insert(period.to_string(), (entry, amount));
                }
            }
            "asset.disposed" => t.disposal_entry = u(p, "entry_id"),
            _ => {}
        }
    }
    // The purchase: an entry that debits the cost account exactly the cost,
    // on the in-service date or as near it as can be (within 31 days).
    // A linked acquisition is the purchase; otherwise match it.
    for a in ledger.registered_assets() {
        if let (Some(id), Some(t)) = (a.acquired_by, out.get_mut(&a.id)) {
            t.purchase_entry = Some(id);
        }
    }
    let reversed: HashSet<EntryId> = (1..ledger.next_entry_id())
        .filter_map(|id| ledger.get_entry(id).and_then(|e| e.reversal_of()))
        .collect();
    let mut claimed: HashSet<EntryId> = out.values().flat_map(|t| t.entries()).collect();
    for a in ledger
        .registered_assets()
        .filter(|a| a.acquired_by.is_none())
    {
        let mut best: Option<(i64, EntryId)> = None;
        for id in 1..ledger.next_entry_id() {
            let Some(e) = ledger.get_entry(id) else {
                continue;
            };
            if e.status() != EntryStatus::Posted
                || e.reversal_of().is_some()
                || reversed.contains(&id)
                || claimed.contains(&id)
            {
                continue;
            }
            let days = (e.date() - a.purchase_date).num_days().abs();
            if days > 31 {
                continue;
            }
            let buys = e.lines().iter().any(|l| {
                l.account == a.cost_account
                    && l.side == Side::Debit
                    && l.amount.amount == a.cost.amount
            });
            if buys && best.is_none_or(|(d, _)| days < d) {
                best = Some((days, id));
            }
        }
        if let Some((_, id)) = best {
            claimed.insert(id);
            if let Some(t) = out.get_mut(&a.id) {
                t.purchase_entry = Some(id);
            }
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
