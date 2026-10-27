//! Needs attention: what the books need, read once for the viewer and the
//! server alike.
//!
//! ```text
//! GOAL: tell the humans what's unfinished or wrong, and hand them the
//!       prompt that gets the agent to fix it; never change the books.
//!       The viewer draws these items and `books.start` lists them as its
//!       next steps, from this one function over [`PackageBooks`], so the
//!       two can never disagree.
//! SOURCE: DECISION (owner: the viewer is read-only; its "Needs
//!         attention" list offers prompts, AGENTS.md); the conditions
//!         below; docs/spec/sources/irs-1099-thresholds.md (W-9s);
//!         DECISION D-40 (owner, 2026-10-20: ask only about vendors paid
//!         in a way that could need a 1099); plan/served.md U0b (D-53: no
//!         `ledger-view` crate).
//! INVARIANTS:
//!   1. Pure: reads the books, writes nothing.
//!   2. One item per: draft entry (Todo); posted entry missing a receipt
//!      (Problem; four or more are one combined item); recurring entry or
//!      schedule due through today (Todo); period ended and still open
//!      (Todo); the revenue and expense accounts (not headers) with no tax
//!      category, with activity or not (ledger_core::tax::
//!      uncategorized_accounts: one item; Info, Todo under
//!      policy.require_tax_category = refuse); legal rule known only
//!      through a past year (Todo); for
//!      this calendar year, and last year in January, from
//!      the year's 1099 report (the server's books.vendor_1099): a vendor a
//!      1099 is due for (reportable) without a W-9 (Problem); a vendor
//!      paid that year in a way that could count toward a 1099 while its
//!      1099 decision was undetermined (Vendor1099Line::decision_needed:
//!      Problem when its status is undetermined, else Todo, its prompt
//!      naming books.set_vendor_1099); the year's payments whose method
//!      decides whether they count (Vendor1099Report::
//!      undetermined_payments, none for a vendor decided not reportable),
//!      one Todo counting reversal chains, its prompt naming
//!      books.instruments, books.add_instrument and
//!      books.set_entry_instrument; a year whose worksheet can't be made
//!      for want of named cash accounts while vendors were paid in it
//!      (Todo); overdue bills, and overdue
//!      invoices (Problem, one each, with the total); a written-off
//!      invoice whose write-off is dated this calendar year and stands
//!      (Todo, one per invoice, "Write-off awaiting the CPA's note",
//!      its prompt naming books.document); money owed to
//!      owners (Info, per account with a balance); the database differing
//!      from the journal or unreadable, and damaged or missing stored
//!      files (Problem); depreciation due through the last finished month
//!      and not posted (amount > 0: Todo, one item); the asset register
//!      and the books' asset cost accounts disagreeing today
//!      (unregistered_cost != 0: Problem); an active asset in service with
//!      no tag (Todo, one per asset); an active asset in service with no
//!      tax basis (Asset::tax_basis is None: Todo, one per asset, its
//!      prompt naming books.set_asset_tax_basis); a party not archived
//!      whose W-9 is Missing (it files a 1099, has no W-9 facts, and its
//!      vendor record doesn't say one is on file) and that was paid this
//!      year (paid_ytd > 0), unless its vendor is already a 1099 vendor
//!      without a W-9 above (Todo, one per party, its prompt from
//!      [`w9_prompt`], naming books.set_w9); a cash or card account whose
//!      reconciliation is due (Todo, one per account, titled "Reconcile
//!      <account> through <date>", its prompt from [`reconcile_prompt`],
//!      naming books.record_statement and books.reconcile); when
//!      `policy.parties` isn't off, the posted entries dated this calendar
//!      year (Jan 1 through today) that name no party and aren't marked as
//!      having none (Ledger::entry_no_party), leaving out the ones the
//!      engine makes from a schedule (period closes, depreciation,
//!      amortization: no counterparty by nature) (one item: Info under
//!      suggest, Todo under require; its prompt names
//!      books.set_entry_party); each warning of the fiscal year's
//!      statement of cash flows (rules.md B-14 rule 12: equipment sold for
//!      an IOU, Todo; bought on an operating payable, Info; one per
//!      entry, its detail the warning's fix).
//!   3. Nothing else: books with none of those yield no items, except
//!      that a figure behind an item that can't be computed (an
//!      amortization plan past chrono's last day, an aging past exact
//!      arithmetic: books an earlier version wrote, rules.md B-28) is one
//!      Problem item saying so, never a panic or a guess.
//!   4. Every item's prompt starts "In wanderbooks," or names
//!      wanderbooks, and its amounts are the books' exact amounts.
//!   5. The same books give the same items through a `Store` and through a
//!      `Snapshot` (the fields each reads are the same: the ledger, the
//!      journal's event types, the attachments and their links, the
//!      statements and reconciliations).
//! ```

use std::collections::{BTreeSet, HashMap, HashSet};

use chrono::{Datelike, NaiveDate};
use ledger_core::parties::MoneyAccounts;
use ledger_core::{
    AccountId, AccountType, AssetId, EntryId, EntryStatus, Ledger, LedgerError, Party, PartyId,
    PartyMember, PeriodStatus,
};
use ledger_store::{Attachment, DiskCheck, JournalLine, Reconciliation, Statement, Subject};
use rust_decimal::Decimal;
use serde_json::Value;

use crate::books::PackageBooks;
use crate::reports::Settings;

/// How urgent an item is; the list is sorted by this, Problems first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Something wrong that wants fixing: a missing receipt, an overdue
    /// bill, a figure the books can't compute.
    Problem,
    /// Something unfinished: a draft, a due schedule, an unreconciled
    /// account.
    Todo,
    /// Worth knowing, not wrong: an uncategorized account under a policy
    /// that only warns.
    Info,
}

/// What an item is about: the viewer maps it to its page, the server to a
/// next step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// Nothing to open (a fact with no page of its own).
    None,
    /// One entry's page.
    Entry(EntryId),
    /// The transactions list.
    Transactions,
    /// The company page.
    Company,
    /// The chart of accounts.
    Accounts,
    /// One account's page.
    Account(AccountId),
    /// One party's page.
    Party(PartyId),
    /// The fixed asset register.
    Assets,
    /// One asset.
    Asset(AssetId),
    /// The documents and attachments list.
    Documents,
    /// The journal.
    Journal,
    /// One of the reports.
    Report(Report),
}

/// A report one of the viewer's pages shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Report {
    /// The 1099 worksheet.
    Vendor1099,
    /// The payables aging.
    ApAging,
    /// The receivables aging.
    ArAging,
    /// The fixed asset register.
    FixedAssetRegister,
}

/// Something the books need, with how to fix it.
#[derive(Clone, Debug)]
pub struct Attention {
    /// How urgent it is.
    pub severity: Severity,
    /// A stable word for the item's kind ("missing_receipt", "draft", ...).
    pub kind: &'static str,
    /// What's wrong, in one line.
    pub title: String,
    /// The particulars: the entry, the amounts, the rule.
    pub detail: String,
    /// Where the viewer opens it.
    pub target: Target,
    /// The prompt to paste into the agent session that fixes it.
    pub prompt: String,
    /// The tool that starts the fix, with its arguments (None: none yet).
    pub tool: Option<(String, Value)>,
}

/// What's on disk against what the journal says, as an attention item needs
/// it: the database check, and the stored files that are missing or
/// damaged.
#[derive(Clone, Debug, PartialEq)]
pub struct Integrity {
    /// How the writer's database compares with the journal.
    pub disk: DiskCheck,
    /// Stored files whose bytes are missing or don't match their hash:
    /// (attachment id, why).
    pub file_failures: Vec<(u64, String)>,
}

impl Default for Integrity {
    fn default() -> Self {
        Integrity {
            disk: DiskCheck::Matches,
            file_failures: Vec::new(),
        }
    }
}

impl Integrity {
    /// Check the database and every stored file: the attachment's bytes
    /// are read (and hash-checked) through the books, once per blob that
    /// is intact; a damaged blob is reported for each attachment filed
    /// from it.
    pub fn check(books: &impl PackageBooks) -> Integrity {
        let mut intact: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut file_failures = Vec::new();
        for a in books.all_attachments().unwrap_or_default() {
            if intact.contains(&a.blob_sha) {
                continue;
            }
            match books.attachment_bytes(a.id) {
                Ok(_) => {
                    intact.insert(a.blob_sha.clone());
                }
                Err(e) => file_failures.push((a.id, e.to_string())),
            }
        }
        Integrity {
            disk: books.disk_check(),
            file_failures,
        }
    }
}

/// Everything the books need, as items to show, computing the integrity
/// check itself (the viewer passes its own, already checked, to
/// [`find_with`]).
pub fn find(books: &impl PackageBooks, today: NaiveDate, settings: &Settings) -> Vec<Attention> {
    let integrity = Integrity::check(books);
    find_with(books, today, settings, &integrity)
}

/// [`find`], with the integrity check the caller already made.
pub fn find_with(
    books: &impl PackageBooks,
    today: NaiveDate,
    settings: &Settings,
    integrity: &Integrity,
) -> Vec<Attention> {
    let ledger = books.ledger();
    let policy = ledger_config::codec::receipt_policy(ledger.settings());
    let threshold = policy.threshold();
    let journal = books.journal().unwrap_or_default();
    let statements = books.statements().unwrap_or_default();
    let reconciliations = books.reconciliations().unwrap_or_default();
    let cash_accounts = ledger_store::reconcilable::cash_accounts(ledger, settings.cash_accounts);
    let financing: Vec<AccountId> = settings
        .financing_accounts
        .iter()
        .filter_map(|c| ledger.chart().find_by_code(c).map(|a| a.id))
        .collect();
    let money_cash: Vec<AccountId> = {
        let configured: Vec<AccountId> = settings
            .cash_accounts
            .iter()
            .filter_map(|c| ledger.chart().find_by_code(c).map(|a| a.id))
            .collect();
        if configured.is_empty() {
            let mut all: Vec<AccountId> = ledger_core::parties::money_accounts(ledger, &[], &[])
                .cash
                .into_iter()
                .collect();
            all.sort_unstable();
            all
        } else {
            configured
        }
    };
    let money = ledger_core::parties::money_accounts(ledger, &money_cash, &financing);
    let members = settings.company.party_members(ledger);
    let entries = entries(books, &journal, &policy);
    let mut out = Vec::new();

    for e in entries.iter().filter(|e| e.status == EntryStatus::Draft) {
        out.push(Attention {
            severity: Severity::Todo,
            kind: "draft",
            title: format!("Draft not posted: {}", e.description),
            detail: format!(
                "#{} · {} · {}. Drafts appear in no report until posted.",
                e.id,
                date(e.date),
                dollars_or(e.amount)
            ),
            target: Target::Entry(e.id),
            prompt: format!(
                "In wanderbooks, review draft entry {} (\"{}\", {}): attach its receipt if it needs one, preview it, and post it if it's right.",
                e.id,
                e.description,
                dollars_or(e.amount)
            ),
            tool: Some(("books.get_entry".into(), serde_json::json!({ "entry_id": e.id }))),
        });
    }

    let missing: Vec<&Entry> = entries
        .iter()
        .filter(|e| e.status == EntryStatus::Posted && e.missing_receipt)
        .collect();
    if missing.len() > 3 {
        let total = missing
            .iter()
            .try_fold(Decimal::ZERO, |t, e| t.checked_add(e.amount?));
        let list: Vec<String> = missing
            .iter()
            .map(|e| format!("{} ({}, {})", e.id, e.description, e.date))
            .collect();
        out.push(Attention {
            severity: Severity::Problem,
            kind: "missing_receipt",
            title: format!("{} without receipts", count(missing.len(), "entry", "entries")),
            detail: format!(
                "{} in entries with no receipt attached ({}).",
                dollars_or(total),
                receipt_rule(threshold)
            ),
            target: Target::Transactions,
            prompt: format!(
                "In wanderbooks, these entries have no receipt: {}. Help me find each receipt and attach it with books.attach_receipt.",
                list.join("; ")
            ),
            tool: Some(("books.attach_receipt".into(), Value::Null)),
        });
    }
    for e in missing.iter().filter(|_| missing.len() <= 3) {
        out.push(Attention {
            severity: Severity::Problem,
            kind: "missing_receipt",
            title: format!("No receipt: {}", e.description),
            detail: format!(
                "#{} · {} · {}: {}.",
                e.id,
                date(e.date),
                dollars_or(e.amount),
                receipt_rule(threshold)
            ),
            target: Target::Entry(e.id),
            prompt: format!(
                "In wanderbooks, entry {} (\"{}\", {} on {}) has no receipt. Find it and attach it with books.attach_receipt.",
                e.id,
                e.description,
                dollars_or(e.amount),
                e.date
            ),
            tool: Some(("books.attach_receipt".into(), Value::Null)),
        });
    }

    for r in ledger.recurring_entries() {
        let due = r.due(today);
        if !due.is_empty() {
            out.push(Attention {
                severity: Severity::Todo,
                kind: "recurring_due",
                title: format!("Recurring \"{}\" is due", r.description),
                detail: format!(
                    "{} since {}.",
                    count(due.len(), "occurrence", "occurrences"),
                    date(due[0])
                ),
                target: Target::Company,
                prompt: format!(
                    "In wanderbooks, run books.run_recurring through {today}, then attach each draft's receipt and post them."
                ),
                tool: Some(("books.run_recurring".into(), Value::Null)),
            });
        }
    }

    for s in ledger.schedules() {
        // A schedule an earlier version journaled may not compute (its
        // months past chrono's last day): say so, never guess.
        let due = match s.due(today) {
            Ok(due) => due,
            Err(e) => {
                out.push(cant_compute(
                    format!("Amortization schedule \"{}\"", s.description),
                    &e,
                    Target::Company,
                ));
                continue;
            }
        };
        if !due.is_empty() {
            let Some(total) = due
                .iter()
                .try_fold(Decimal::ZERO, |t, (_, a)| t.checked_add(*a))
            else {
                out.push(cant_compute(
                    format!("Amortization schedule \"{}\"", s.description),
                    &LedgerError::TooLargeToCompute("the months due".into()),
                    Target::Company,
                ));
                continue;
            };
            out.push(Attention {
                severity: Severity::Todo,
                kind: "schedule_due",
                title: format!("Amortization due: {}", s.description),
                detail: format!(
                    "{} totaling {} not yet posted.",
                    count(due.len(), "month", "months"),
                    dollars(total)
                ),
                target: Target::Company,
                prompt: format!(
                    "In wanderbooks, preview books.run_schedules through {today}, then post it if it's right."
                ),
                tool: Some(("books.run_schedules".into(), Value::Null)),
            });
        }
    }

    for p in ledger.list_periods() {
        if p.status == PeriodStatus::Open && p.end < today {
            out.push(Attention {
                severity: Severity::Todo,
                kind: "period_open",
                title: format!("Period {} ended but is open", p.name),
                detail: format!(
                    "{} – {}. Closing locks it and moves its earnings to equity.",
                    date(p.start),
                    date(p.end)
                ),
                target: Target::Company,
                prompt: format!(
                    "In wanderbooks, review period {} and, if everything for it is recorded and reconciled, preview and close it.",
                    p.name
                ),
                tool: Some(("books.list_periods".into(), Value::Null)),
            });
        }
    }

    // Every revenue and expense account (not a header) without a tax
    // category, with activity or not (rules.md B-32): the one list the
    // digest gives the agent too.
    let uncategorized: Vec<String> = ledger_core::tax::uncategorized_accounts(ledger)
        .into_iter()
        .map(|a| format!("{} {}", a.code, a.name))
        .collect();
    if !uncategorized.is_empty() {
        let refuse = ledger.settings().require_tax_category()
            == ledger_core::settings::TaxCategoryPolicy::Refuse;
        out.push(Attention {
            severity: if refuse {
                Severity::Todo
            } else {
                Severity::Info
            },
            kind: "uncategorized",
            title: format!(
                "{} without a tax category",
                count(uncategorized.len(), "account", "accounts")
            ),
            detail: format!(
                "{}. Each revenue and expense account maps to a line of the return (the tax summary and the CPA package list it as uncategorized until then).",
                uncategorized.join(", ")
            ),
            target: Target::Accounts,
            prompt: format!(
                "In wanderbooks, propose a tax category for each of these accounts ({}) using books.tax_categories, and set them with books.set_tax_category once I agree.",
                uncategorized.join(", ")
            ),
            tool: Some(("books.tax_categories".into(), Value::Null)),
        });
    }

    // Rules set by law that wanderbooks may no longer know (rules.md;
    // owner, 2026-10-13): the agent finds the figure from the authority
    // or asks, confirms it, and passes it explicitly (the handbook's
    // "Rules that change with the law"); the human reports it upstream.
    for rule in ledger_core::rules::stale_rules(today) {
        out.push(Attention {
            severity: Severity::Todo,
            kind: "stale_rule",
            title: format!("Check the {}", rule.name),
            detail: format!(
                "wanderbooks knows it through {}. {}",
                rule.known_through, rule.review_why
            ),
            target: Target::None,
            prompt: format!(
                "In wanderbooks, the {} is known only through {}. Find the current figure from the official authority, {}, or ask me or our CPA, and confirm it with me; then pass it explicitly wherever a tool asks for it. Tell me what you found and its official source, so I can report it upstream and the rule is updated for everyone.",
                rule.name, rule.known_through, rule.authority
            ),
            tool: Some((
                "books.handbook".into(),
                serde_json::json!({ "section": "rules-that-change" }),
            )),
        });
    }

    let mut years = vec![today.year()];
    if today.month() == 1 {
        years.insert(0, today.year() - 1);
    }
    // Vendors the "needs a W-9" item already names this year.
    let mut asked_w9 = HashSet::new();
    for year in years {
        // A year whose threshold isn't known is flagged by the stale-rule
        // item above; never guess it here.
        let r = match crate::vendor_1099::vendor_1099_report(ledger, settings, year, None, None) {
            Ok(r) => r,
            Err(LedgerError::Invalid1099(why))
                if ledger
                    .posted_entries()
                    .any(|e| e.date().year() == year && ledger.entry_vendor(e.id()).is_some()) =>
            {
                out.push(Attention {
                    severity: Severity::Todo,
                    kind: "worksheet_unmakeable",
                    title: format!("The {year} 1099 worksheet can't be made"),
                    detail: format!("{why}."),
                    target: Target::Report(Report::Vendor1099),
                    prompt: format!(
                        "In wanderbooks, the {year} 1099 worksheet can't be made: {why}. Ask me which accounts hold the company's cash, then set them with books.set_setting."
                    ),
                    tool: Some(("books.settings".into(), Value::Null)),
                });
                continue;
            }
            Err(_) => continue,
        };
        let in_year = if year == today.year() {
            String::new()
        } else {
            format!(" for {year}")
        };
        for v in r.vendors.iter().filter(|v| v.decision_needed) {
            let party = ledger.party_of_vendor(v.vendor_id);
            let unknown = if v.undetermined > Decimal::ZERO {
                format!(
                    " and {} more whose payment method isn't recorded",
                    dollars(v.undetermined)
                )
            } else {
                String::new()
            };
            out.push(Attention {
                severity: if v.status == ledger_core::Status1099::Undetermined {
                    Severity::Problem
                } else {
                    Severity::Todo
                },
                kind: "vendor_1099_decision",
                title: format!("Decide whether {} gets a 1099{in_year}", v.name),
                detail: format!(
                    "Paid {} by check, ACH, transfer or cash{unknown} in {year}, with no 1099 decision.",
                    dollars(v.paid)
                ),
                target: party.map_or(Target::Company, Target::Party),
                prompt: format!(
                    "In wanderbooks, {} was paid {}{unknown} in {year} and has no 1099 decision. Ask me whether it gets a 1099-NEC, a 1099-MISC, or none and why (a corporation, the United States or a state, a payee selling only merchandise, ...); then record it with books.set_vendor_1099, with who decided and when.",
                    v.name,
                    dollars(v.paid)
                ),
                tool: Some((
                    "books.set_vendor_1099".into(),
                    serde_json::json!({ "vendor": v.vendor_id.to_string() }),
                )),
            });
        }
        // Payments whose method decides whether they count, a payment
        // and its reversals counted once.
        let mut chains: Vec<EntryId> = r
            .undetermined_payments
            .iter()
            .map(|p| ledger.reversal_chain(p.entry_id)[0])
            .collect();
        chains.sort_unstable();
        chains.dedup();
        if !chains.is_empty() {
            let n = chains.len();
            let ids = chains
                .iter()
                .map(|e| e.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            let (payments, entries) = if n == 1 {
                ("payment has", "entry")
            } else {
                ("payments have", "entries")
            };
            out.push(Attention {
                severity: Severity::Todo,
                kind: "payment_method",
                title: format!("{n} {payments} no payment method{in_year}"),
                detail: format!(
                    "Paid for the company through a financing account in {year} ({entries} {ids}): a member's card doesn't count toward a 1099, their cash or check does, so the 1099 worksheet can't tell until each is recorded."
                ),
                target: Target::Report(Report::Vendor1099),
                prompt: format!(
                    "In wanderbooks, {entries} {ids} paid a vendor in {year} through a financing account with no payment method recorded, so the 1099 worksheet can't tell whether they count. Ask me how each was paid (whose card, or cash, check or a transfer); add any method not yet in books.instruments with books.add_instrument, then tag each entry with books.set_entry_instrument."
                ),
                tool: Some(("books.instruments".into(), Value::Null)),
            });
        }
        for v in r.vendors.iter().filter(|v| v.reportable && !v.tin_on_file) {
            if year == today.year() {
                asked_w9.insert(v.vendor_id);
            }
            let party = ledger.party_of_vendor(v.vendor_id);
            out.push(Attention {
                severity: Severity::Problem,
                kind: "missing_w9",
                title: format!("{} needs a W-9", v.name),
                detail: format!(
                    "Paid {} in {year}, over the {} 1099 threshold.",
                    dollars(v.paid),
                    dollars(r.threshold)
                ),
                target: party.map_or(Target::Company, Target::Party),
                prompt: format!(
                    "Remind me to request a W-9 from {}; once I have it, record what it says in wanderbooks with books.set_w9 (that marks it on file and keeps the 1099 decision).",
                    v.name
                ),
                tool: Some((
                    "books.set_w9".into(),
                    party.map_or(Value::Null, |p| {
                        serde_json::json!({ "party": p.to_string() })
                    }),
                )),
            });
        }
    }

    for p in ledger.parties().filter(|p| {
        !p.archived && w9_missing(ledger, p) && !p.vendor.is_some_and(|v| asked_w9.contains(&v))
    }) {
        let paid_ytd = paid_this_year(ledger, &money, &members, p.id, today);
        if paid_ytd <= Decimal::ZERO {
            continue;
        }
        out.push(Attention {
            severity: Severity::Todo,
            kind: "collect_w9",
            title: format!("Collect a W-9 from {}", p.label()),
            detail: format!(
                "Paid {} in {} as {}, with no W-9 facts recorded: collect a W-9 before the next payment.",
                dollars(paid_ytd),
                today.year(),
                role_word(p)
            ),
            target: Target::Party(p.id),
            prompt: w9_prompt(p, paid_ytd, today.year()),
            tool: Some((
                "books.set_w9".into(),
                serde_json::json!({ "party": p.id.to_string() }),
            )),
        });
    }

    // Overdue: (parties, total), or why it can't be computed (documents
    // are restored as journaled, of any size).
    let overdue = |rows: Result<Vec<ledger_core::AgingRow>, LedgerError>| {
        let too_large = || LedgerError::TooLargeToCompute("the overdue total".into());
        let late: Vec<Decimal> = rows?
            .iter()
            .map(|r| {
                [r.days_60, r.days_90, r.days_90_plus]
                    .into_iter()
                    .try_fold(r.days_30, Decimal::checked_add)
                    .ok_or_else(too_large)
            })
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .filter(|d| !d.is_zero())
            .collect();
        let total = late
            .iter()
            .try_fold(Decimal::ZERO, |t, d| t.checked_add(*d))
            .ok_or_else(too_large)?;
        Ok::<_, LedgerError>((late.len(), total))
    };
    let (n, total) = match overdue(ledger.ap_aging(today)) {
        Ok(v) => v,
        Err(e) => {
            out.push(cant_compute(
                "The payables aging".into(),
                &e,
                Target::Report(Report::ApAging),
            ));
            (0, Decimal::ZERO)
        }
    };
    if n > 0 {
        out.push(Attention {
            severity: Severity::Problem,
            kind: "ap_overdue",
            title: format!("{} overdue to pay", dollars(total)),
            detail: format!("{} with bills past due.", count(n, "vendor", "vendors")),
            target: Target::Report(Report::ApAging),
            prompt: "In wanderbooks, show me the A/P aging and which bills are past due.".into(),
            tool: Some(("books.ap_aging".into(), Value::Null)),
        });
    }
    let (n, total) = match overdue(ledger.ar_aging(today)) {
        Ok(v) => v,
        Err(e) => {
            out.push(cant_compute(
                "The receivables aging".into(),
                &e,
                Target::Report(Report::ArAging),
            ));
            (0, Decimal::ZERO)
        }
    };
    if n > 0 {
        out.push(Attention {
            severity: Severity::Problem,
            kind: "ar_overdue",
            title: format!("{} overdue to collect", dollars(total)),
            detail: format!(
                "{} with invoices past due.",
                count(n, "customer", "customers")
            ),
            target: Target::Report(Report::ArAging),
            prompt:
                "In wanderbooks, show me the A/R aging and draft reminders for past-due invoices."
                    .into(),
            tool: Some(("books.ar_aging".into(), Value::Null)),
        });
    }

    // A write-off this year waits for the CPA's note: the deduction rests
    // on evidence of worthlessness a human signs off, and §166's judgments
    // stay with the CPA (plan P4b S7, 8.3.16; IRC §166(a); Treas. Reg.
    // §1.166-2(a)). Listed until the year ends.
    let mut written_off: Vec<(u64, Option<String>, NaiveDate, Decimal)> = Vec::new();
    for inv in ledger.invoices() {
        let doc = ledger_core::DocRef::Invoice(inv.id);
        let state = ledger.invoice_state(inv.id, today).ok().flatten();
        if state.map(|s| s.status) != Some(ledger_core::DocStatus::WrittenOff) {
            continue;
        }
        let lines: Vec<ledger_core::DocLine> = ledger
            .doc_lines(doc)
            .into_iter()
            .filter(|l| {
                l.movement == ledger_core::Movement::WriteOff
                    && l.date.year() == today.year()
                    && ledger.stands_on(l.entry, today)
            })
            .collect();
        let Some(last) = lines.last() else {
            continue;
        };
        match lines
            .iter()
            .try_fold(Decimal::ZERO, |t, l| t.checked_sub(l.amount))
        {
            Some(amount) => written_off.push((inv.id, inv.number.clone(), last.date, amount)),
            None => out.push(cant_compute(
                format!("Invoice {}", inv.id),
                &LedgerError::TooLargeToCompute("what was written off".into()),
                Target::Documents,
            )),
        }
    }
    for (id, number, on, amount) in written_off {
        let name = number.unwrap_or_else(|| format!("Invoice {id}"));
        out.push(Attention {
            severity: Severity::Todo,
            kind: "write_off_note",
            title: format!("Write-off awaiting the CPA's note: {name}"),
            detail: format!(
                "{} written off on {} (IRC §166; Treas. Reg. §1.166-2: the evidence of worthlessness is filed with the write-off).",
                dollars(amount),
                date(on)
            ),
            target: Target::Documents,
            prompt: format!(
                "In wanderbooks, show me invoice {id}'s write-off (books.document), with its reason and the evidence filed with it, and note it for the CPA at year end."
            ),
            tool: Some((
                "books.document".into(),
                serde_json::json!({ "ref": format!("invoice:{id}") }),
            )),
        });
    }

    for a in ledger.chart().all().filter(|a| {
        a.account_type == AccountType::Liability && {
            let n = a.name.to_lowercase();
            n.contains("due to") || n.contains("owed to")
        }
    }) {
        let bal = balance_on(ledger, a.id, today);
        if bal > Decimal::ZERO {
            out.push(Attention {
                severity: Severity::Info,
                kind: "due_to_owners",
                title: format!("{} owed on {}", dollars(bal), a.name),
                detail: "Expenses someone paid personally, not yet reimbursed.".into(),
                target: Target::Account(a.id),
                prompt: format!(
                    "In wanderbooks, list what makes up the {} balance on {} {} and who is owed what.",
                    dollars(bal),
                    a.code,
                    a.name
                ),
                tool: Some((
                    "books.search_entries".into(),
                    serde_json::json!({ "account_id": a.id }),
                )),
            });
        }
    }

    for a in reconcilable_accounts(ledger, &cash_accounts, &statements) {
        let Some(through) = reconciliation_due(ledger, &reconciliations, a, today) else {
            continue;
        };
        let name = account_name(ledger, a);
        let reconciled = reconciled_through(&reconciliations, a);
        out.push(Attention {
            severity: Severity::Todo,
            kind: "reconcile",
            title: format!("Reconcile {name} through {}", date(through)),
            detail: match reconciled {
                Some(d) => format!(
                    "Reconciled through {}. The monthly close reconciles it to each statement since.",
                    date(d)
                ),
                None => "Never reconciled. The monthly close reconciles it to its statement.".into(),
            },
            target: Target::Account(a),
            prompt: reconcile_prompt(&account_label(ledger, a), reconciled, through),
            tool: Some(("books.record_statement".into(), Value::Null)),
        });
    }

    let policy = ledger.settings().parties();
    if policy != ledger_core::settings::PartyPolicy::Off {
        let year_start = NaiveDate::from_ymd_opt(today.year(), 1, 1).expect("valid");
        let unnamed: Vec<&Entry> = entries
            .iter()
            .filter(|e| {
                e.status == EntryStatus::Posted
                    && e.date >= year_start
                    && e.date <= today
                    && e.parties.is_empty()
                    && ledger.entry_no_party(e.id).is_none()
                    && !matches!(e.origin, "Period close" | "Depreciation" | "Amortization")
            })
            .collect();
        if !unnamed.is_empty() {
            let list: Vec<String> = unnamed
                .iter()
                .map(|e| format!("{} ({}, {})", e.id, e.description, e.date))
                .collect();
            out.push(Attention {
                severity: if policy == ledger_core::settings::PartyPolicy::Require {
                    Severity::Todo
                } else {
                    Severity::Info
                },
                kind: "no_counterparty",
                title: format!(
                    "{} no counterparty",
                    count(unnamed.len(), "entry names", "entries name")
                ),
                detail: format!(
                    "Posted in {} with no party and not marked as having none. Tagging each entry with who it was with keeps the parties' totals and 1099s complete.",
                    today.year()
                ),
                target: Target::Transactions,
                prompt: format!(
                    "In wanderbooks, these posted entries from {} name no counterparty: {}. Propose a party for each, grouped by party (adding any that aren't in the directory yet), and once I agree, tag them with books.set_entry_party. For any with truly no counterparty (like a transfer between the company's own accounts), mark it with books.set_entry_party with party null and a no_party_reason.",
                    today.year(),
                    list.join("; ")
                ),
                tool: Some(("books.set_entry_party".into(), Value::Null)),
            });
        }
    }

    // The register computes unless a contributor basis an earlier version
    // journaled is past exact arithmetic: then one item says so.
    let (due, register) = match (
        depreciation_due(ledger, today),
        fixed_asset_register(ledger, today),
    ) {
        (Ok(due), Ok(register)) => (Some(due), Some(register)),
        (Err(e), _) | (_, Err(e)) => {
            out.push(cant_compute(
                "The fixed asset register".into(),
                &e,
                Target::Report(Report::FixedAssetRegister),
            ));
            (None, None)
        }
    };
    if let Some(due) = due.filter(|d| d.amount > Decimal::ZERO) {
        let months = if due.periods.is_empty() {
            "every month not yet posted".to_string()
        } else {
            due.periods.join(", ")
        };
        out.push(Attention {
            severity: Severity::Todo,
            kind: "depreciation",
            title: format!("Depreciation not posted through {}", date(due.through)),
            detail: format!(
                "{} scheduled and not yet in the books ({}).",
                dollars(due.amount),
                months
            ),
            target: Target::Assets,
            prompt: format!(
                "In wanderbooks, run books.run_depreciation for {months} (depreciation due through {} and not posted: {}), then show me the fixed asset register.",
                due.through,
                dollars(due.amount)
            ),
            tool: Some(("books.run_depreciation".into(), Value::Null)),
        });
    }
    // Accumulated depreciation no asset's runs account for: a hand entry,
    // or a run reversed after its asset left the books.
    let unattributed = register
        .as_ref()
        .map_or(Decimal::ZERO, |r| r.unattributed_accumulated());
    if !unattributed.is_zero() {
        out.push(Attention {
            severity: Severity::Problem,
            kind: "unattributed_depreciation",
            title: format!(
                "{} of accumulated depreciation not from any asset's runs",
                dollars(unattributed.abs())
            ),
            detail: "The accumulated depreciation accounts hold depreciation (or take it back) that no registered asset's depreciation run accounts for: an entry made by hand, or a run reversed after its asset was disposed.".into(),
            target: Target::Assets,
            prompt: format!(
                "In wanderbooks, the accumulated depreciation accounts differ by {} from what the assets' depreciation runs put there (books.export fixed_asset_register, unattributed_accumulated). Find the entries behind it in those accounts' registers and tell me how to fix them.",
                dollars(unattributed)
            ),
            tool: Some((
                "books.export".into(),
                serde_json::json!({ "format": "csv", "report": "fixed_asset_register" }),
            )),
        });
    }
    let unregistered = register.map_or(Decimal::ZERO, |r| r.unregistered_cost());
    if !unregistered.is_zero() {
        let (title, why) = if unregistered > Decimal::ZERO {
            (
                format!(
                    "{} of equipment not in the asset register",
                    dollars(unregistered)
                ),
                "The asset cost accounts hold more than the registered assets cost: something bought was never registered.",
            )
        } else {
            (
                format!(
                    "{} of registered assets not in the books",
                    dollars(-unregistered)
                ),
                "Registered assets cost more than their cost accounts hold: a purchase entry may be missing.",
            )
        };
        out.push(Attention {
            severity: Severity::Problem,
            kind: "unregistered_assets",
            title,
            detail: why.into(),
            target: Target::Assets,
            prompt: format!(
                "In wanderbooks, the fixed asset register and the asset cost accounts differ by {} today. Find what's behind it (books.list_assets and the cost accounts' registers) and tell me how to fix it.",
                dollars(unregistered)
            ),
            tool: Some(("books.list_assets".into(), Value::Null)),
        });
    }
    let held = |a: &&ledger_core::Asset| {
        a.status == ledger_core::AssetStatus::Active
            && ledger.asset_acquired_date(a.id).is_some_and(|d| d <= today)
    };
    for a in ledger
        .registered_assets()
        .filter(held)
        .filter(|a| a.tag.is_none())
    {
        out.push(Attention {
            severity: Severity::Todo,
            kind: "asset_tag",
            title: format!("No tag: {}", a.description),
            detail: format!(
                "Asset {}, in service {}. A tag is the label on the thing itself.",
                a.id,
                date(a.in_service_date)
            ),
            target: Target::Asset(a.id),
            prompt: format!(
                "In wanderbooks, label asset {} (\"{}\") with books.label_asset: give it the next generated tag unless I have my own, and its serial number if I can find it. Then print its label with books.asset_labels.",
                a.id, a.description
            ),
            tool: Some((
                "books.label_asset".into(),
                serde_json::json!({ "asset": a.id.to_string() }),
            )),
        });
    }

    for a in ledger.registered_assets().filter(held) {
        let name = a.tag.clone().unwrap_or_else(|| format!("asset {}", a.id));
        let Some(basis) = &a.basis else {
            out.push(Attention {
                severity: Severity::Todo,
                kind: "asset_tax_basis",
                title: format!("No tax basis: {}", asset_label(ledger, a.id)),
                detail: format!(
                    "Book cost {}, in service {}. The CPA needs its federal tax basis beside its book value.",
                    dollars(a.cost.amount),
                    date(a.in_service_date)
                ),
                target: Target::Asset(a.id),
                prompt: format!(
                    "In wanderbooks, record the tax basis of {name} (\"{}\", book cost {}) with books.set_asset_tax_basis. Ask me how the company got it: bought (purchase), contributed by a member from business use (carryover_723, IRC §723: the member's adjusted basis), or contributed by a member from personal use (personal_use_conversion, Treas. Reg. §1.168(i)-4(b): the member's adjusted basis, depreciated from the lesser of that and its fair market value); for a contribution, its fair market value when contributed, and who says so. Then show me the asset.",
                    a.description,
                    dollars(a.cost.amount)
                ),
                tool: Some((
                    "books.set_asset_tax_basis".into(),
                    serde_json::json!({ "asset": a.id.to_string() }),
                )),
            });
            continue;
        };
        if !basis.rule.is_contribution() {
            continue;
        }
        if !a.fmv_is_recorded() {
            out.push(Attention {
                severity: Severity::Todo,
                kind: "asset_fmv",
                title: format!("Confirm its value at contribution: {}", asset_label(ledger, a.id)),
                detail: format!(
                    "Its basis was recorded before the value at contribution was asked, so the books read it as the book cost, {}. The §704(c) built-in gain or loss rests on it.",
                    dollars(a.cost.amount)
                ),
                target: Target::Asset(a.id),
                prompt: format!(
                    "In wanderbooks, {name} (\"{}\") was contributed with its value at contribution not recorded; the books read it as its book cost, {}. Ask me what it was worth when contributed, then record it with books.set_asset_tax_basis (rule {}, contributor_adjusted_basis {}, fmv_at_contribution, and basis_source: who says so, citing the contribution entry). Then show me the asset.",
                    a.description,
                    dollars(a.cost.amount),
                    basis.rule.key(),
                    basis
                        .contributor_adjusted_basis
                        .map(|v| v.to_string())
                        .unwrap_or_default()
                ),
                tool: Some((
                    "books.set_asset_tax_basis".into(),
                    serde_json::json!({ "asset": a.id.to_string() }),
                )),
            });
        }
        if a.built_in_gain_704c().is_some_and(|g| !g.is_zero()) && basis.sec704c_method.is_none() {
            out.push(Attention {
                severity: Severity::Todo,
                kind: "sec704c",
                title: format!("Choose a §704(c) method with the CPA: {}", asset_label(ledger, a.id)),
                detail: "It was contributed with a built-in gain or loss, and no §704(c) method (traditional, traditional with curative allocations, or remedial) is recorded for it.".into(),
                target: Target::Asset(a.id),
                prompt: format!(
                    "In wanderbooks, {name} (\"{}\") has a §704(c) built-in gain or loss and no §704(c) method. Ask me which method the CPA and I chose (traditional, traditional_with_curative, or remedial: Treas. Reg. §1.704-3(b)-(d)), then record it with books.set_asset_tax_basis, keeping its other facts as they are. Then show me the asset.",
                    a.description
                ),
                tool: Some((
                    "books.set_asset_tax_basis".into(),
                    serde_json::json!({ "asset": a.id.to_string() }),
                )),
            });
        }
    }

    let verify_prompt = "In wanderbooks, run books.verify and tell me what it found.".to_string();
    match &integrity.disk {
        DiskCheck::Differs(why) => out.push(Attention {
            severity: Severity::Problem,
            kind: "database",
            title: "The database doesn't match the journal".into(),
            detail: format!("{why}. The journal is the record; the server rebuilds the database from it (books.verify, or on its next start)."),
            target: Target::Journal,
            prompt: verify_prompt.clone(),
            tool: Some(("books.verify".into(), Value::Null)),
        }),
        DiskCheck::Unreadable(why) => out.push(Attention {
            severity: Severity::Problem,
            kind: "database",
            title: "Couldn't check the database".into(),
            detail: format!("{why}."),
            target: Target::Journal,
            prompt: verify_prompt.clone(),
            tool: Some(("books.verify".into(), Value::Null)),
        }),
        _ => {}
    }
    if !integrity.file_failures.is_empty() {
        let ids: Vec<String> = integrity
            .file_failures
            .iter()
            .map(|(id, _)| id.to_string())
            .collect();
        out.push(Attention {
            severity: Severity::Problem,
            kind: "stored_files",
            title: format!(
                "{} damaged or missing",
                count(ids.len(), "stored file is", "stored files are")
            ),
            detail: format!(
                "Attachment {}: the bytes on disk don't match their recorded hash. Restore them from a backup (the books dir's git history).",
                ids.join(", ")
            ),
            target: Target::Documents,
            prompt: verify_prompt,
            tool: Some(("books.verify".into(), Value::Null)),
        });
    }
    // The statement of cash flows' warnings this fiscal year (rules.md
    // B-14 rule 12, D-35): equipment sold for an IOU, flagged with its fix
    // (Todo); equipment bought on an operating payable, following how the
    // payable is classified (Info). A statement the settings can't give
    // (no cash accounts yet) has none.
    let (fy_start, _) = fiscal_ytd(settings.company, today);
    if let Ok(cf) = crate::reports::cash_flow_for(
        ledger,
        settings,
        Some(&fy_start.to_string()),
        Some(&today.to_string()),
        &[],
        &[],
        &[],
        today,
    ) {
        for w in cf.warnings {
            let sold = w.kind == ledger_core::CashFlowWarningKind::AssetSoldOnOperatingReceivable;
            out.push(Attention {
                severity: if sold { Severity::Todo } else { Severity::Info },
                kind: "cash_flow",
                title: if sold {
                    format!("Equipment sold for an IOU: entry #{}", w.entry)
                } else {
                    format!("Equipment bought on an operating payable: entry #{}", w.entry)
                },
                detail: w.message.clone(),
                target: Target::Entry(w.entry),
                prompt: format!(
                    "In wanderbooks, the cash flow statement flags entry {}: {} Show me the entry and what you would change, and change nothing until I agree.",
                    w.entry, w.message
                ),
                tool: Some((
                    "books.get_entry".into(),
                    serde_json::json!({ "entry_id": w.entry }),
                )),
            });
        }
    }
    out.sort_by_key(|a| a.severity);
    out
}

/// One entry, indexed the way the items read it.
struct Entry {
    id: EntryId,
    date: NaiveDate,
    description: String,
    status: EntryStatus,
    /// Total debits; None when they can't be summed exactly.
    amount: Option<Decimal>,
    origin: &'static str,
    parties: Vec<PartyId>,
    /// Covered by the receipt policy and has no receipt.
    missing_receipt: bool,
}

/// Every entry, with the journal-derived facts the items read: its origin
/// and whether the receipt policy covers it without a receipt.
fn entries(
    books: &impl PackageBooks,
    journal: &[JournalLine],
    policy: &ledger_config::ReceiptPolicy,
) -> Vec<Entry> {
    let ledger = books.ledger();
    let mut events: HashMap<EntryId, Vec<usize>> = HashMap::new();
    for (i, line) in journal.iter().enumerate() {
        for id in entry_refs(line) {
            events.entry(id).or_default().push(i);
        }
    }
    // An entry's receipts: filed on it, or linked to it.
    let mut receipts: HashMap<EntryId, usize> = HashMap::new();
    let attachments: Vec<Attachment> = books.all_attachments().unwrap_or_default();
    for a in &attachments {
        if a.withdrawn_at.is_some() {
            continue;
        }
        let mut subjects: Vec<Subject> = a.entry_id.map(Subject::Entry).into_iter().collect();
        subjects.extend(books.links_of(a.id).unwrap_or_default());
        let mut seen: HashSet<EntryId> = HashSet::new();
        for s in subjects {
            if let Subject::Entry(e) = s
                && seen.insert(e)
            {
                *receipts.entry(e).or_default() += 1;
            }
        }
    }
    let closings = ledger.closing_entry_ids();
    let mut out = Vec::new();
    for id in 1..ledger.next_entry_id() {
        let Some(e) = ledger.get_entry(id) else {
            continue;
        };
        let evs = events.remove(&id).unwrap_or_default();
        let origin = origin(&evs, journal, closings.contains(&id));
        let amount = ledger_config::ReceiptPolicy::entry_debits(e).ok();
        let covered = matches!(origin, "Journal entry" | "Recurring")
            && e.reversal_of().is_none()
            && receipts.get(&id).copied().unwrap_or(0) == 0
            && policy.receipt_required(ledger, e);
        out.push(Entry {
            id,
            date: e.date(),
            description: e.description().to_string(),
            status: e.status(),
            amount,
            origin,
            parties: ledger.entry_parties(id).iter().map(|p| p.party).collect(),
            missing_receipt: covered,
        });
    }
    out
}

/// Entry ids a journal line refers to.
fn entry_refs(line: &JournalLine) -> Vec<EntryId> {
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

/// Depreciation due and not yet in the books.
struct Due {
    through: NaiveDate,
    amount: Decimal,
    periods: Vec<String>,
}

/// Depreciation due through the last finished month on or before `as_of`
/// and not yet posted; refused when the register can't be computed.
fn depreciation_due(ledger: &Ledger, as_of: NaiveDate) -> Result<Due, LedgerError> {
    // A month's last day, or chrono's last day; else the end of the
    // month before (none before chrono's first month: its first day).
    let through = if as_of.succ_opt().is_none_or(|d| d.day() == 1) {
        as_of
    } else {
        as_of
            .with_day(1)
            .and_then(|m| m.pred_opt())
            .unwrap_or(NaiveDate::MIN)
    };
    let amount = fixed_asset_register(ledger, through)?.unposted_depreciation();
    let mut periods = BTreeSet::new();
    for a in ledger.registered_assets().filter(|a| {
        ledger
            .asset_acquired_date(a.id)
            .is_some_and(|d| d <= through)
    }) {
        if matches!(a.status, ledger_core::AssetStatus::Disposed { date, .. } if date <= through) {
            continue;
        }
        let runs: Vec<(EntryId, Decimal)> = ledger
            .depreciation_runs(a.id)
            .iter()
            .map(|r| (r.entry, r.amount))
            .collect();
        let in_effect: HashSet<&str> = ledger
            .depreciation_runs(a.id)
            .iter()
            .filter(|r| ledger.in_effect(r.entry))
            .map(|r| r.period.as_str())
            .collect();
        let effects = ledger_core::depreciation_effects(ledger, &runs);
        let due: HashMap<(i32, u32), Decimal> =
            ledger_core::run_amounts(a, (through.year(), through.month()), &effects)?
                .into_iter()
                .map(|(y, m, r)| ((y, m), r.amount))
                .collect();
        for l in ledger_core::depreciation_schedule(a, through)? {
            if in_effect.contains(l.period.as_str()) {
                continue;
            }
            if period_month(&l.period)
                .and_then(|ym| due.get(&ym))
                .is_some_and(|a| !a.is_zero())
            {
                periods.insert(l.period);
            }
        }
    }
    Ok(Due {
        through,
        amount,
        periods: periods.into_iter().collect(),
    })
}

/// A "YYYY-MM" period as (year, month), read leniently (an earlier version
/// posted "10000-01").
fn period_month(period: &str) -> Option<(i32, u32)> {
    let (y, m) = period.split_once('-')?;
    let (y, m) = (y.parse().ok()?, m.parse().ok()?);
    (1..=12).contains(&m).then_some((y, m))
}

fn fixed_asset_register(
    ledger: &Ledger,
    as_of: NaiveDate,
) -> Result<ledger_core::AssetRegister, LedgerError> {
    ledger_core::fixed_asset_register(ledger, as_of)
}

fn fiscal_ytd(
    company: &ledger_core::settings::Company,
    today: NaiveDate,
) -> (NaiveDate, NaiveDate) {
    (
        ledger_core::calendar::fiscal_year_start_on(company.fiscal_year_start, today),
        today,
    )
}

fn balance_on(ledger: &Ledger, account: AccountId, date: NaiveDate) -> Decimal {
    ledger_core::analysis::balance_series(ledger, &[account], &[date])[0]
}

fn account_name(ledger: &Ledger, id: AccountId) -> String {
    ledger
        .chart()
        .get(id)
        .map(|a| a.name.clone())
        .unwrap_or_else(|| format!("account {id}"))
}

fn account_label(ledger: &Ledger, id: AccountId) -> String {
    ledger
        .chart()
        .get(id)
        .map(|a| format!("{} {}", a.code, a.name))
        .unwrap_or_else(|| format!("account {id}"))
}

fn asset_label(ledger: &Ledger, id: AssetId) -> String {
    match ledger.get_asset(id) {
        Some(a) => match &a.tag {
            Some(t) => format!("{t} {}", a.description),
            None => a.description.clone(),
        },
        None => format!("asset {id}"),
    }
}

fn reconcilable_accounts(
    ledger: &Ledger,
    cash_accounts: &[AccountId],
    statements: &[Statement],
) -> Vec<AccountId> {
    ledger_store::reconcilable::reconcilable_accounts(ledger, cash_accounts, statements)
}

fn reconciliation_due(
    ledger: &Ledger,
    reconciliations: &[Reconciliation],
    account: AccountId,
    today: NaiveDate,
) -> Option<NaiveDate> {
    ledger_store::reconcilable::reconciliation_due(ledger, reconciliations, account, today)
}

/// The latest statement date an account is reconciled through: the newest
/// reconciliation of it not voided (None: never reconciled).
fn reconciled_through(reconciliations: &[Reconciliation], account: AccountId) -> Option<NaiveDate> {
    reconciliations
        .iter()
        .filter(|r| r.account_id == account && r.voided.is_none())
        .map(|r| r.statement_date)
        .max()
}

/// Whether a party files a 1099: it has a 1099 role (contractor, landlord),
/// or its vendor record has been decided for one (from any date: its W-9
/// matters for those payments).
pub fn files_1099(ledger: &Ledger, p: &Party) -> bool {
    p.roles.iter().any(|r| r.gets_1099())
        || p.vendor
            .and_then(|v| ledger.get_vendor(v))
            .is_some_and(|v| v.decisions_1099.iter().any(|d| d.decision.form().is_some()))
}

/// A party files a 1099, has no W-9 facts, and its vendor record doesn't
/// say one is on file: the Missing arm of the viewer's W-9 column.
pub fn w9_missing(ledger: &Ledger, p: &Party) -> bool {
    if p.w9.is_some() || !files_1099(ledger, p) {
        return false;
    }
    !p.vendor
        .and_then(|v| ledger.get_vendor(v))
        .is_some_and(|v| v.tin_on_file)
}

/// "as a contractor" or "as a landlord", by the party's 1099 role.
fn role_word(p: &Party) -> &'static str {
    if p.roles.contains(&ledger_core::Role::Landlord)
        && !p.roles.contains(&ledger_core::Role::Contractor)
    {
        "a landlord"
    } else {
        "a contractor"
    }
}

/// The agent prompt that collects and records a party's W-9.
pub fn w9_prompt(p: &Party, paid_ytd: Decimal, year: i32) -> String {
    let paid = if paid_ytd > Decimal::ZERO {
        format!(", paid {} so far in {year}", dollars(paid_ytd))
    } else {
        String::new()
    };
    format!(
        "In wanderbooks, {} (party {}) is a {}{paid} with no W-9 facts recorded. Remind me to collect a W-9 before the next payment. Once I have it, record what it says with books.set_w9: the date received, the entity type (line 3), any exempt payee or FATCA codes (line 4), the last four digits of the TIN only (the full TIN goes in the vault, never the books), and where the form is kept. Then show me the party.",
        p.label(),
        p.id,
        role_word(p)
    )
}

/// The agent prompt that has the agent record a statement and reconcile an
/// account through a date.
pub fn reconcile_prompt(label: &str, reconciled: Option<NaiveDate>, through: NaiveDate) -> String {
    let since = match reconciled {
        Some(d) => format!("It is reconciled through {d}, so start with the statement after that"),
        None => "It has never been reconciled, so start with its first statement".to_string(),
    };
    format!(
        "In wanderbooks, reconcile {label} through {through}. {since}. For each monthly statement: file the PDF, record it with books.record_statement (its period and opening and closing balances), mark the lines that cleared with books.mark_cleared, then run books.reconcile against it (preview first; the difference must be exactly zero). Show me anything that doesn't match, and the account when it's done."
    )
}

/// What a party was paid this calendar year (net, cash basis): the
/// viewer's `PartyInfo::paid_ytd`, through the same `activity_with`.
fn paid_this_year(
    ledger: &Ledger,
    money: &MoneyAccounts,
    members: &[PartyMember],
    party: PartyId,
    today: NaiveDate,
) -> Decimal {
    let start = NaiveDate::from_ymd_opt(today.year(), 1, 1).expect("valid");
    let last = ledger
        .posted_entries()
        .filter(|e| e.date().year() == today.year())
        .map(|e| e.date())
        .max()
        .unwrap_or(today)
        .max(today);
    let end = last.min(NaiveDate::from_ymd_opt(today.year(), 12, 31).expect("valid"));
    let a = ledger_core::parties::activity_with(ledger, party, (start, end), money, members);
    a.paid - a.received
}

/// A figure the books can't compute (rules.md B-28: books an earlier
/// version wrote may hold dates past chrono's calendar or amounts past
/// exact arithmetic): a Problem saying so and why, never a guess.
fn cant_compute(what: String, why: &LedgerError, target: Target) -> Attention {
    Attention {
        severity: Severity::Problem,
        kind: "cant_compute",
        title: format!("{what} can't be computed"),
        detail: format!("{why}."),
        target,
        prompt: format!(
            "In wanderbooks, {} can't be computed ({why}). Find the record behind it and tell me how to correct it.",
            what.to_lowercase()
        ),
        tool: None,
    }
}

/// Why an entry needs a receipt, in words: "every entry needs a receipt"
/// or "over the $25.00 receipt threshold".
fn receipt_rule(threshold: Decimal) -> String {
    if threshold.is_zero() {
        "every entry needs a receipt".to_string()
    } else {
        format!("over the {} receipt threshold", dollars(threshold))
    }
}

// The viewer's own spellings, so an item reads the same wherever it is
// shown (ledger-gui::fmt): thousands grouped, every decimal place the
// amount has (at least two), a true minus sign.
fn dollars(d: Decimal) -> String {
    let s = format!("${}", group(d.abs()));
    if d.is_sign_negative() && !d.is_zero() {
        format!("\u{2212}{s}")
    } else {
        s
    }
}

/// A non-negative amount, every decimal it has (at least two), grouped.
fn group(d: Decimal) -> String {
    let places = d.normalize().scale().max(2) as usize;
    let s = format!("{d:.places$}");
    let (int, frac) = s.split_once('.').unwrap_or((&s, "00"));
    let mut out = String::new();
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    format!("{out}.{frac}")
}

/// [`dollars`], or "too large to compute" for a figure the books can't
/// compute.
fn dollars_or(d: Option<Decimal>) -> String {
    d.map_or_else(|| "too large to compute".to_string(), dollars)
}

fn date(d: NaiveDate) -> String {
    d.format("%b %-d, %Y").to_string()
}

fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}
