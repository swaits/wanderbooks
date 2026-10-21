//! What the books need: drafts left unposted, missing receipts, schedules
//! and recurring entries due, open past periods, uncategorized tax lines,
//! 1099 vendors without a W-9 or without a 1099 decision, payments whose
//! method the 1099 worksheet needs, overdue bills and invoices, money owed to
//! owners, fixed asset depreciation not posted, assets without a tag or a
//! tax basis, contractors and landlords paid without W-9 facts, bank and
//! card accounts not reconciled through last month, and legal
//! rules (like the 1099 threshold) due for an update. Each item says
//! what's wrong, where to look, and the prompt that
//! asks the agent to fix it (the GUI never changes the books itself).

use crate::books::{Attention, Books, Severity};
use crate::fmt;
use crate::nav::Route;
use chrono::Datelike;
use ledger_core::{AccountType, EntryStatus, PeriodStatus};
use rust_decimal::Decimal;

/// A figure the books can't compute (rules.md B-28: books an earlier
/// version wrote may hold dates past chrono's calendar or amounts past
/// exact arithmetic): a Problem saying so and why, never a guess.
fn cant_compute(what: String, why: &ledger_core::LedgerError, target: Option<Route>) -> Attention {
    Attention {
        severity: Severity::Problem,
        title: format!("{what} can't be computed"),
        detail: format!("{why}."),
        target,
        prompt: format!(
            "In wanderbooks, {} can't be computed ({why}). Find the record behind it and tell me how to correct it.",
            what.to_lowercase()
        ),
    }
}

/// Everything the books need, as items to show.
///
/// ```text
/// GOAL: tell the humans what's unfinished or wrong, and hand them the
///       prompt that gets the agent to fix it; never change the books.
/// SOURCE: DECISION (owner: the viewer is read-only; its "Needs
///         attention" list offers prompts, AGENTS.md); the conditions
///         below; docs/spec/sources/irs-1099-thresholds.md (W-9s);
///         DECISION D-40 (owner, 2026-10-20: ask only about vendors paid
///         in a way that could need a 1099).
/// INVARIANTS:
///   1. Pure: reads `b`, writes nothing.
///   2. One item per: draft entry (Todo); posted entry missing a receipt
///      (Problem; four or more are one combined item); recurring entry or
///      schedule due through today (Todo); period ended and still open
///      (Todo); the revenue and expense accounts (not headers) with no tax
///      category, with activity or not (ledger_core::tax::
///      uncategorized_accounts: one item; Info, Todo under
///      policy.require_tax_category = refuse); legal rule known only
///      through a past year (Todo); for
///      this calendar year, and last year in January, from
///      Books::vendor_1099 (the server's books.vendor_1099): a vendor a
///      1099 is due for (reportable) without a W-9 (Problem); a vendor
///      paid that year in a way that could count toward a 1099 while its
///      1099 decision was undetermined (Vendor1099Line::decision_needed:
///      Problem when its status is undetermined, else Todo, its prompt
///      naming books.set_vendor_1099); the year's payments whose method
///      decides whether they count (Vendor1099Report::
///      undetermined_payments, none for a vendor decided not reportable),
///      one Todo counting reversal chains, its prompt naming
///      books.instruments, books.add_instrument and
///      books.set_entry_instrument; a year whose worksheet can't be made
///      for want of named cash accounts while vendors were paid in it
///      (Todo); overdue bills, and overdue
///      invoices (Problem, one each, with the total); money owed to
///      owners (Info, per account with a balance); the database differing
///      from the journal or unreadable, and damaged or missing stored
///      files (Problem); depreciation due through the last finished month
///      and not posted (Books::depreciation_due amount > 0: Todo, one
///      item); the asset register and the books' asset cost accounts
///      disagreeing today (unregistered_cost != 0: Problem); an active
///      asset in service with no tag (Todo, one per asset); an active
///      asset in service with no tax basis (Asset::tax_basis is None:
///      Todo, one per asset, its prompt naming books.set_asset_tax_basis);
///      a party not archived whose W-9 is Missing (pages::parties::
///      w9_status: it files a 1099, has no W-9 facts, and its vendor
///      record doesn't say one is on file) and that was paid this year
///      (PartyInfo::paid_ytd > 0),
///      unless its vendor is already a 1099 vendor without a W-9 above
///      (Todo, one per party, its prompt from
///      pages::parties::collect_w9_prompt, naming books.set_w9); a cash
///      or card account (Books::reconcilable_accounts) whose
///      Books::reconciliation_due is Some(date) (Todo, one per account,
///      titled "Reconcile <account> through <date>", its prompt from
///      pages::reconciliation::reconcile_prompt, naming
///      books.record_statement and books.reconcile); when `[books]
///      parties` isn't off, the posted entries dated this calendar year
///      (Jan 1 through today) that name no party and aren't marked as
///      having none (Ledger::entry_no_party), leaving out the ones the
///      engine makes from a schedule (period closes, depreciation,
///      amortization: no counterparty by nature) (one item: Info under
///      suggest, Todo under require; its prompt names
///      books.set_entry_party); each warning of the fiscal year's
///      statement of cash flows (rules.md B-14 rule 12: equipment sold for
///      an IOU, Todo; bought on an operating payable, Info; one per
///      entry, its detail the warning's fix).
///   3. Nothing else: books with none of those yield no items, except
///      that a figure behind an item that can't be computed (an
///      amortization plan past chrono's last day, an aging past exact
///      arithmetic: books an earlier version wrote, rules.md B-28) is one
///      Problem item saying so, never a panic or a guess.
///   4. Every item's prompt starts "In wanderbooks," or names
///      wanderbooks, and its amounts are the books' exact amounts.
/// ```
pub fn find(b: &Books) -> Vec<Attention> {
    let mut out = Vec::new();
    let today = b.today;

    for e in b.entries.iter().filter(|e| e.status == EntryStatus::Draft) {
        out.push(Attention {
            severity: Severity::Todo,
            title: format!("Draft not posted: {}", e.description),
            detail: format!(
                "#{} · {} · {}. Drafts appear in no report until posted.",
                e.id,
                fmt::date(e.date),
                fmt::dollars_or(e.amount)
            ),
            target: Some(Route::Entry(e.id)),
            prompt: format!(
                "In wanderbooks, review draft entry {} (\"{}\", {}): attach its receipt if it needs one, preview it, and post it if it's right.",
                e.id,
                e.description,
                fmt::dollars_or(e.amount)
            ),
        });
    }

    let missing: Vec<_> = b
        .entries
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
            title: format!("{} without receipts", fmt::count(missing.len(), "entry", "entries")),
            detail: format!(
                "{} in entries with no receipt attached ({}).",
                fmt::dollars_or(total),
                b.receipt_rule()
            ),
            target: Some(Route::Transactions),
            prompt: format!(
                "In wanderbooks, these entries have no receipt: {}. Help me find each receipt and attach it with books.attach_receipt.",
                list.join("; ")
            ),
        });
    }
    for e in missing.iter().filter(|_| missing.len() <= 3) {
        out.push(Attention {
            severity: Severity::Problem,
            title: format!("No receipt: {}", e.description),
            detail: format!(
                "#{} · {} · {}: {}.",
                e.id,
                fmt::date(e.date),
                fmt::dollars_or(e.amount),
                b.receipt_rule()
            ),
            target: Some(Route::Entry(e.id)),
            prompt: format!(
                "In wanderbooks, entry {} (\"{}\", {} on {}) has no receipt. Find it and attach it with books.attach_receipt.",
                e.id,
                e.description,
                fmt::dollars_or(e.amount),
                e.date
            ),
        });
    }

    for r in b.ledger.recurring_entries() {
        let due = r.due(today);
        if !due.is_empty() {
            out.push(Attention {
                severity: Severity::Todo,
                title: format!("Recurring \"{}\" is due", r.description),
                detail: format!(
                    "{} since {}.",
                    fmt::count(due.len(), "occurrence", "occurrences"),
                    fmt::date(due[0])
                ),
                target: Some(Route::Company),
                prompt: format!(
                    "In wanderbooks, run books.run_recurring through {today}, then attach each draft's receipt and post them."
                ),
            });
        }
    }

    for s in b.ledger.schedules() {
        // A schedule an earlier version journaled may not compute (its
        // months past chrono's last day): say so, never guess.
        let due = match s.due(today) {
            Ok(due) => due,
            Err(e) => {
                out.push(cant_compute(
                    format!("Amortization schedule \"{}\"", s.description),
                    &e,
                    Some(Route::Company),
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
                    &ledger_core::LedgerError::TooLargeToCompute("the months due".into()),
                    Some(Route::Company),
                ));
                continue;
            };
            out.push(Attention {
                severity: Severity::Todo,
                title: format!("Amortization due: {}", s.description),
                detail: format!(
                    "{} totaling {} not yet posted.",
                    fmt::count(due.len(), "month", "months"),
                    fmt::dollars(total)
                ),
                target: Some(Route::Company),
                prompt: format!(
                    "In wanderbooks, preview books.run_schedules through {today}, then post it if it's right."
                ),
            });
        }
    }

    for p in b.ledger.list_periods() {
        if p.status == PeriodStatus::Open && p.end < today {
            out.push(Attention {
                severity: Severity::Todo,
                title: format!("Period {} ended but is open", p.name),
                detail: format!(
                    "{} – {}. Closing locks it and moves its earnings to equity.",
                    fmt::date(p.start),
                    fmt::date(p.end)
                ),
                target: Some(Route::Company),
                prompt: format!(
                    "In wanderbooks, review period {} and, if everything for it is recorded and reconciled, preview and close it.",
                    p.name
                ),
            });
        }
    }

    // Every revenue and expense account (not a header) without a tax
    // category, with activity or not (rules.md B-32): the one list the
    // digest gives the agent too.
    let uncategorized: Vec<String> = ledger_core::tax::uncategorized_accounts(&b.ledger)
        .into_iter()
        .map(|a| format!("{} {}", a.code, a.name))
        .collect();
    if !uncategorized.is_empty() {
        let refuse = b.ledger.settings().require_tax_category()
            == ledger_core::settings::TaxCategoryPolicy::Refuse;
        out.push(Attention {
            severity: if refuse {
                Severity::Todo
            } else {
                Severity::Info
            },
            title: format!(
                "{} without a tax category",
                fmt::count(uncategorized.len(), "account", "accounts")
            ),
            detail: format!(
                "{}. Each revenue and expense account maps to a line of the return (the tax summary and the CPA package list it as uncategorized until then).",
                uncategorized.join(", ")
            ),
            target: Some(Route::Accounts),
            prompt: format!(
                "In wanderbooks, propose a tax category for each of these accounts ({}) using books.tax_categories, and set them with books.set_tax_category once I agree.",
                uncategorized.join(", ")
            ),
        });
    }

    // Rules set by law that wanderbooks may no longer know (rules.md;
    // owner, 2026-10-13): the agent finds the figure from the authority
    // or asks, confirms it, and passes it explicitly (the handbook's
    // "Rules that change with the law"); the human reports it upstream.
    for rule in ledger_core::rules::stale_rules(today) {
        out.push(Attention {
            severity: Severity::Todo,
            title: format!("Check the {}", rule.name),
            detail: format!(
                "wanderbooks knows it through {}. {}",
                rule.known_through, rule.review_why
            ),
            target: None,
            prompt: format!(
                "In wanderbooks, the {} is known only through {}. Find the current figure from the official authority, {}, or ask me or our CPA, and confirm it with me; then pass it explicitly wherever a tool asks for it. Tell me what you found and its official source, so I can report it upstream and the rule is updated for everyone.",
                rule.name, rule.known_through, rule.authority
            ),
        });
    }

    let mut years = vec![today.year()];
    if today.month() == 1 {
        years.insert(0, today.year() - 1);
    }
    // Vendors the "needs a W-9" item already names this year.
    let mut asked_w9 = std::collections::HashSet::new();
    for year in years {
        // A year whose threshold isn't known is flagged by the stale-rule
        // item above; never guess it here.
        let r = match b.vendor_1099(year) {
            Ok(r) => r,
            Err(ledger_core::LedgerError::Invalid1099(why))
                if b.ledger.posted_entries().any(|e| {
                    e.date().year() == year && b.ledger.entry_vendor(e.id()).is_some()
                }) =>
            {
                out.push(Attention {
                    severity: Severity::Todo,
                    title: format!("The {year} 1099 worksheet can't be made"),
                    detail: format!("{why}."),
                    target: Some(Route::Reports(crate::nav::ReportKind::Vendor1099)),
                    prompt: format!(
                        "In wanderbooks, the {year} 1099 worksheet can't be made: {why}. Ask me which accounts hold the company's cash, then set them with books.set_setting."
                    ),
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
            let party = b.ledger.party_of_vendor(v.vendor_id);
            let unknown = if v.undetermined > Decimal::ZERO {
                format!(
                    " and {} more whose payment method isn't recorded",
                    fmt::dollars(v.undetermined)
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
                title: format!("Decide whether {} gets a 1099{in_year}", v.name),
                detail: format!(
                    "Paid {} by check, ACH, transfer or cash{unknown} in {year}, with no 1099 decision.",
                    fmt::dollars(v.paid)
                ),
                target: Some(party.map_or(Route::Company, Route::Party)),
                prompt: format!(
                    "In wanderbooks, {} was paid {}{unknown} in {year} and has no 1099 decision. Ask me whether it gets a 1099-NEC, a 1099-MISC, or none and why (a corporation, the United States or a state, a payee selling only merchandise, ...); then record it with books.set_vendor_1099, with who decided and when.",
                    v.name,
                    fmt::dollars(v.paid)
                ),
            });
        }
        // Payments whose method decides whether they count, a payment
        // and its reversals counted once.
        let mut chains: Vec<ledger_core::EntryId> = r
            .undetermined_payments
            .iter()
            .map(|p| b.ledger.reversal_chain(p.entry_id)[0])
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
                title: format!("{n} {payments} no payment method{in_year}"),
                detail: format!(
                    "Paid for the company through a financing account in {year} ({entries} {ids}): a member's card doesn't count toward a 1099, their cash or check does, so the 1099 worksheet can't tell until each is recorded."
                ),
                target: Some(Route::Reports(crate::nav::ReportKind::Vendor1099)),
                prompt: format!(
                    "In wanderbooks, {entries} {ids} paid a vendor in {year} through a financing account with no payment method recorded, so the 1099 worksheet can't tell whether they count. Ask me how each was paid (whose card, or cash, check or a transfer); add any method not yet in books.instruments with books.add_instrument, then tag each entry with books.set_entry_instrument."
                ),
            });
        }
        for v in r.vendors.iter().filter(|v| v.reportable && !v.tin_on_file) {
            if year == today.year() {
                asked_w9.insert(v.vendor_id);
            }
            let party = b.ledger.party_of_vendor(v.vendor_id);
            out.push(Attention {
                severity: Severity::Problem,
                title: format!("{} needs a W-9", v.name),
                detail: format!(
                    "Paid {} in {year}, over the {} 1099 threshold.",
                    fmt::dollars(v.paid),
                    fmt::dollars(r.threshold)
                ),
                target: Some(party.map_or(Route::Company, Route::Party)),
                prompt: format!(
                    "Remind me to request a W-9 from {}; once I have it, record what it says in wanderbooks with books.set_w9 (that marks it on file and keeps the 1099 decision).",
                    v.name
                ),
            });
        }
    }

    for p in b.ledger.parties().filter(|p| {
        !p.archived
            && crate::pages::parties::w9_status(b, p) == crate::pages::parties::W9Status::Missing
            && !p.vendor.is_some_and(|v| asked_w9.contains(&v))
    }) {
        let Some(info) = b.party(p.id).filter(|i| i.paid_ytd > Decimal::ZERO) else {
            continue;
        };
        out.push(Attention {
            severity: Severity::Todo,
            title: format!("Collect a W-9 from {}", p.label()),
            detail: format!(
                "Paid {} in {} as {}, with no W-9 facts recorded: collect a W-9 before the next payment.",
                fmt::dollars(info.paid_ytd),
                today.year(),
                if p.roles.contains(&ledger_core::Role::Landlord)
                    && !p.roles.contains(&ledger_core::Role::Contractor)
                {
                    "a landlord"
                } else {
                    "a contractor"
                }
            ),
            target: Some(Route::Party(p.id)),
            prompt: crate::pages::parties::collect_w9_prompt(b, p),
        });
    }

    // Overdue: (parties, total), or why it can't be computed (documents
    // are restored as journaled, of any size).
    let overdue = |rows: Result<Vec<ledger_core::AgingRow>, ledger_core::LedgerError>| {
        let too_large = || ledger_core::LedgerError::TooLargeToCompute("the overdue total".into());
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
        Ok::<_, ledger_core::LedgerError>((late.len(), total))
    };
    let (n, total) = match overdue(b.ledger.ap_aging(today)) {
        Ok(v) => v,
        Err(e) => {
            out.push(cant_compute(
                "The payables aging".into(),
                &e,
                Some(Route::Reports(crate::nav::ReportKind::ApAging)),
            ));
            (0, Decimal::ZERO)
        }
    };
    if n > 0 {
        out.push(Attention {
            severity: Severity::Problem,
            title: format!("{} overdue to pay", fmt::dollars(total)),
            detail: format!(
                "{} with bills past due.",
                fmt::count(n, "vendor", "vendors")
            ),
            target: Some(Route::Reports(crate::nav::ReportKind::ApAging)),
            prompt: "In wanderbooks, show me the A/P aging and which bills are past due.".into(),
        });
    }
    let (n, total) = match overdue(b.ledger.ar_aging(today)) {
        Ok(v) => v,
        Err(e) => {
            out.push(cant_compute(
                "The receivables aging".into(),
                &e,
                Some(Route::Reports(crate::nav::ReportKind::ArAging)),
            ));
            (0, Decimal::ZERO)
        }
    };
    if n > 0 {
        out.push(Attention {
            severity: Severity::Problem,
            title: format!("{} overdue to collect", fmt::dollars(total)),
            detail: format!(
                "{} with invoices past due.",
                fmt::count(n, "customer", "customers")
            ),
            target: Some(Route::Reports(crate::nav::ReportKind::ArAging)),
            prompt:
                "In wanderbooks, show me the A/R aging and draft reminders for past-due invoices."
                    .into(),
        });
    }

    for a in b.ledger.chart().all().filter(|a| {
        a.account_type == AccountType::Liability && {
            let n = a.name.to_lowercase();
            n.contains("due to") || n.contains("owed to")
        }
    }) {
        let bal = b.balance_on(a.id, today);
        if bal > Decimal::ZERO {
            out.push(Attention {
                severity: Severity::Info,
                title: format!("{} owed on {}", fmt::dollars(bal), a.name),
                detail: "Expenses someone paid personally, not yet reimbursed.".into(),
                target: Some(Route::Account(a.id)),
                prompt: format!(
                    "In wanderbooks, list what makes up the {} balance on {} {} and who is owed what.",
                    fmt::dollars(bal),
                    a.code,
                    a.name
                ),
            });
        }
    }

    for a in b.reconcilable_accounts() {
        let Some(through) = b.reconciliation_due(a) else {
            continue;
        };
        let name = b.account_name(a);
        out.push(Attention {
            severity: Severity::Todo,
            title: format!("Reconcile {name} through {}", fmt::date(through)),
            detail: match b.reconciled_through(a) {
                Some(d) => format!(
                    "Reconciled through {}. The monthly close reconciles it to each statement since.",
                    fmt::date(d)
                ),
                None => "Never reconciled. The monthly close reconciles it to its statement.".into(),
            },
            target: Some(Route::Account(a)),
            prompt: crate::pages::reconciliation::reconcile_prompt(b, a, through),
        });
    }

    let policy = b.settings.parties;
    if policy != ledger_config::PartyPolicy::Off {
        let year_start = chrono::NaiveDate::from_ymd_opt(today.year(), 1, 1).expect("valid");
        let unnamed: Vec<_> = b
            .entries
            .iter()
            .filter(|e| {
                e.status == EntryStatus::Posted
                    && e.date >= year_start
                    && e.date <= today
                    && e.parties.is_empty()
                    && b.ledger.entry_no_party(e.id).is_none()
                    && !matches!(e.origin, "Period close" | "Depreciation" | "Amortization")
            })
            .collect();
        if !unnamed.is_empty() {
            let list: Vec<String> = unnamed
                .iter()
                .map(|e| format!("{} ({}, {})", e.id, e.description, e.date))
                .collect();
            out.push(Attention {
                severity: if policy == ledger_config::PartyPolicy::Require {
                    Severity::Todo
                } else {
                    Severity::Info
                },
                title: format!(
                    "{} no counterparty",
                    fmt::count(unnamed.len(), "entry names", "entries name")
                ),
                detail: format!(
                    "Posted in {} with no party and not marked as having none. Tagging each entry with who it was with keeps the parties' totals and 1099s complete.",
                    today.year()
                ),
                target: Some(Route::Transactions),
                prompt: format!(
                    "In wanderbooks, these posted entries from {} name no counterparty: {}. Propose a party for each, grouped by party (adding any that aren't in the directory yet), and once I agree, tag them with books.set_entry_party. For any with truly no counterparty (like a transfer between the company's own accounts), mark it with books.set_entry_party with party null and a no_party_reason.",
                    today.year(),
                    list.join("; ")
                ),
            });
        }
    }

    // The register computes unless a contributor basis an earlier version
    // journaled is past exact arithmetic: then one item says so.
    let (due, register) = match (b.depreciation_due(today), b.asset_register(today)) {
        (Ok(due), Ok(register)) => (Some(due), Some(register)),
        (Err(e), _) | (_, Err(e)) => {
            out.push(cant_compute(
                "The fixed asset register".into(),
                &e,
                Some(Route::Reports(crate::nav::ReportKind::FixedAssetRegister)),
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
            title: format!("Depreciation not posted through {}", fmt::date(due.through)),
            detail: format!(
                "{} scheduled and not yet in the books ({}).",
                fmt::dollars(due.amount),
                months
            ),
            target: Some(Route::Assets),
            prompt: format!(
                "In wanderbooks, run books.run_depreciation for {months} (depreciation due through {} and not posted: {}), then show me the fixed asset register.",
                due.through,
                fmt::dollars(due.amount)
            ),
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
            title: format!(
                "{} of accumulated depreciation not from any asset's runs",
                fmt::dollars(unattributed.abs())
            ),
            detail: "The accumulated depreciation accounts hold depreciation (or take it back) that no registered asset's depreciation run accounts for: an entry made by hand, or a run reversed after its asset was disposed.".into(),
            target: Some(Route::Assets),
            prompt: format!(
                "In wanderbooks, the accumulated depreciation accounts differ by {} from what the assets' depreciation runs put there (books.export fixed_asset_register, unattributed_accumulated). Find the entries behind it in those accounts' registers and tell me how to fix them.",
                fmt::dollars(unattributed)
            ),
        });
    }
    let unregistered = register.map_or(Decimal::ZERO, |r| r.unregistered_cost());
    if !unregistered.is_zero() {
        let (title, why) = if unregistered > Decimal::ZERO {
            (
                format!(
                    "{} of equipment not in the asset register",
                    fmt::dollars(unregistered)
                ),
                "The asset cost accounts hold more than the registered assets cost: something bought was never registered.",
            )
        } else {
            (
                format!(
                    "{} of registered assets not in the books",
                    fmt::dollars(-unregistered)
                ),
                "Registered assets cost more than their cost accounts hold: a purchase entry may be missing.",
            )
        };
        out.push(Attention {
            severity: Severity::Problem,
            title,
            detail: why.into(),
            target: Some(Route::Assets),
            prompt: format!(
                "In wanderbooks, the fixed asset register and the asset cost accounts differ by {} today. Find what's behind it (books.list_assets and the cost accounts' registers) and tell me how to fix it.",
                fmt::dollars(unregistered)
            ),
        });
    }
    let held = |a: &&ledger_core::Asset| {
        a.status == ledger_core::AssetStatus::Active
            && b.ledger
                .asset_acquired_date(a.id)
                .is_some_and(|d| d <= today)
    };
    for a in b
        .ledger
        .registered_assets()
        .filter(held)
        .filter(|a| a.tag.is_none())
    {
        out.push(Attention {
            severity: Severity::Todo,
            title: format!("No tag: {}", a.description),
            detail: format!(
                "Asset {}, in service {}. A tag is the label on the thing itself.",
                a.id,
                fmt::date(a.in_service_date)
            ),
            target: Some(Route::Asset(a.id)),
            prompt: format!(
                "In wanderbooks, label asset {} (\"{}\") with books.label_asset: give it the next generated tag unless I have my own, and its serial number if I can find it. Then print its label with books.asset_labels.",
                a.id, a.description
            ),
        });
    }

    for a in b.ledger.registered_assets().filter(held) {
        let name = a.tag.clone().unwrap_or_else(|| format!("asset {}", a.id));
        let Some(basis) = &a.basis else {
            out.push(Attention {
                severity: Severity::Todo,
                title: format!("No tax basis: {}", b.asset_label(a.id)),
                detail: format!(
                    "Book cost {}, in service {}. The CPA needs its federal tax basis beside its book value.",
                    fmt::dollars(a.cost.amount),
                    fmt::date(a.in_service_date)
                ),
                target: Some(Route::Asset(a.id)),
                prompt: format!(
                    "In wanderbooks, record the tax basis of {name} (\"{}\", book cost {}) with books.set_asset_tax_basis. Ask me how the company got it: bought (purchase), contributed by a member from business use (carryover_723, IRC §723: the member's adjusted basis), or contributed by a member from personal use (personal_use_conversion, Treas. Reg. §1.168(i)-4(b): the member's adjusted basis, depreciated from the lesser of that and its fair market value); for a contribution, its fair market value when contributed, and who says so. Then show me the asset.",
                    a.description,
                    fmt::dollars(a.cost.amount)
                ),
            });
            continue;
        };
        if !basis.rule.is_contribution() {
            continue;
        }
        if !a.fmv_is_recorded() {
            out.push(Attention {
                severity: Severity::Todo,
                title: format!("Confirm its value at contribution: {}", b.asset_label(a.id)),
                detail: format!(
                    "Its basis was recorded before the value at contribution was asked, so the books read it as the book cost, {}. The §704(c) built-in gain or loss rests on it.",
                    fmt::dollars(a.cost.amount)
                ),
                target: Some(Route::Asset(a.id)),
                prompt: format!(
                    "In wanderbooks, {name} (\"{}\") was contributed with its value at contribution not recorded; the books read it as its book cost, {}. Ask me what it was worth when contributed, then record it with books.set_asset_tax_basis (rule {}, contributor_adjusted_basis {}, fmv_at_contribution, and basis_source: who says so, citing the contribution entry). Then show me the asset.",
                    a.description,
                    fmt::dollars(a.cost.amount),
                    basis.rule.key(),
                    basis
                        .contributor_adjusted_basis
                        .map(|v| v.to_string())
                        .unwrap_or_default()
                ),
            });
        }
        if a.built_in_gain_704c().is_some_and(|g| !g.is_zero()) && basis.sec704c_method.is_none() {
            out.push(Attention {
                severity: Severity::Todo,
                title: format!("Choose a §704(c) method with the CPA: {}", b.asset_label(a.id)),
                detail: "It was contributed with a built-in gain or loss, and no §704(c) method (traditional, traditional with curative allocations, or remedial) is recorded for it.".into(),
                target: Some(Route::Asset(a.id)),
                prompt: format!(
                    "In wanderbooks, {name} (\"{}\") has a §704(c) built-in gain or loss and no §704(c) method. Ask me which method the CPA and I chose (traditional, traditional_with_curative, or remedial: Treas. Reg. §1.704-3(b)-(d)), then record it with books.set_asset_tax_basis, keeping its other facts as they are. Then show me the asset.",
                    a.description
                ),
            });
        }
    }

    use ledger_store::DiskCheck;
    let verify_prompt = "In wanderbooks, run books.verify and tell me what it found.".to_string();
    match &b.integrity.disk {
        DiskCheck::Differs(why) => out.push(Attention {
            severity: Severity::Problem,
            title: "The database doesn't match the journal".into(),
            detail: format!("{why}. The journal is the record; the server rebuilds the database from it (books.verify, or on its next start)."),
            target: Some(Route::Journal),
            prompt: verify_prompt.clone(),
        }),
        DiskCheck::Unreadable(why) => out.push(Attention {
            severity: Severity::Problem,
            title: "Couldn't check the database".into(),
            detail: format!("{why}."),
            target: Some(Route::Journal),
            prompt: verify_prompt.clone(),
        }),
        _ => {}
    }
    if !b.integrity.file_failures.is_empty() {
        let ids: Vec<String> = b
            .integrity
            .file_failures
            .iter()
            .map(|(id, _)| id.to_string())
            .collect();
        out.push(Attention {
            severity: Severity::Problem,
            title: format!("{} damaged or missing", fmt::count(ids.len(), "stored file is", "stored files are")),
            detail: format!(
                "Attachment {}: the bytes on disk don't match their recorded hash. Restore them from a backup (the books dir's git history).",
                ids.join(", ")
            ),
            target: Some(Route::Documents),
            prompt: verify_prompt,
        });
    }
    // The statement of cash flows' warnings this fiscal year (rules.md
    // B-14 rule 12, D-35): equipment sold for an IOU, flagged with its fix
    // (Todo); equipment bought on an operating payable, following how the
    // payable is classified (Info). A statement the settings can't give
    // (no cash accounts yet) has none.
    let settings = ledger_package::Settings {
        company: &b.settings.company,
        cash_accounts: &b.settings.cash_accounts,
        financing_accounts: &b.settings.financing_accounts,
        investing_accounts: &b.settings.investing_accounts,
    };
    let (fy_start, _) = b.fiscal_ytd();
    if let Ok(cf) = ledger_package::cash_flow_for(
        &b.ledger,
        &settings,
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
                title: if sold {
                    format!("Equipment sold for an IOU: entry #{}", w.entry)
                } else {
                    format!("Equipment bought on an operating payable: entry #{}", w.entry)
                },
                detail: w.message.clone(),
                target: Some(Route::Entry(w.entry)),
                prompt: format!(
                    "In wanderbooks, the cash flow statement flags entry {}: {} Show me the entry and what you would change, and change nothing until I agree.",
                    w.entry, w.message
                ),
            });
        }
    }
    out.sort_by_key(|a| a.severity);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Empty books opened as of `today`.
    fn books(tag: &str, today: chrono::NaiveDate) -> Books {
        let dir =
            std::env::temp_dir().join(format!("ledger-gui-attention-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // Loaded books need no files: removed once loaded, pass or panic.
        let _cleanup = crate::test_support::TempDir(dir.clone());
        let data_dir = dir.join("books-data");
        ledger_store::Store::create(&data_dir, ledger_core::Currency::USD).unwrap();
        crate::data::load_once(&crate::data::Source {
            data_dir,
            today: Some(today),
        })
        .unwrap()
    }

    /// INVARIANT 2's cash flow items (rules.md B-14 rule 12, D-35): a
    /// sale of equipment for an IOU is a Todo naming the entry, with the
    /// statement's fix; equipment bought on an operating card is Info; a
    /// sale for cash asks nothing.
    #[test]
    fn cash_flow_warnings_are_items() {
        use crate::test_support::{Build, d};
        use ledger_core::{AccountType, Currency, Money, NewAsset, Side};
        let mut b = Build::new("attention-cf");
        let mut add =
            |code: &str, name: &str, ty| b.s.add_account(code, name, ty, None, false).unwrap();
        let equipment = add("1500", "Equipment", AccountType::Asset);
        let accum = add("1510", "Accumulated Depreciation", AccountType::Asset);
        let iou = add("1210", "Due from buyer", AccountType::Asset);
        let card = add("2050", "Card", AccountType::Liability);
        let depr = add("6500", "Depreciation", AccountType::Expense);
        let import = ledger_config::codec::import(
            "[books]\ncash_accounts = [\"1000\"]\n",
            None,
            b.s.ledger().settings(),
            "USD",
        )
        .unwrap();
        b.s.adopt_settings(
            &import.changes,
            &ledger_core::settings::Source {
                decided_by: "the test".into(),
                decided_on: d(2026, 1, 1),
                citation: None,
                note: None,
            },
        )
        .unwrap();
        let (cash, capital) = (b.cash, b.capital);
        b.post(d(2026, 2, 1), cash, capital, 500_000);
        b.s.register_asset(NewAsset {
            description: "Laptop".into(),
            in_service_date: d(2026, 2, 2),
            cost: Money::new(rust_decimal::Decimal::new(100_000, 2), Currency::USD),
            salvage_value: Money::new(rust_decimal::Decimal::ZERO, Currency::USD),
            useful_life_months: 36,
            cost_account: equipment,
            accum_depr_account: accum,
            expense_account: depr,
        })
        .unwrap();
        let bought = b.post(d(2026, 2, 2), equipment, card, 100_000);
        let usd = |c: i64| Money::new(rust_decimal::Decimal::new(c, 2), Currency::USD);
        let e =
            b.s.create_entry(d(2026, 3, 2), "Sell the laptop on an IOU")
                .unwrap();
        b.s.add_line(e, iou, Side::Debit, usd(100_000)).unwrap();
        b.s.add_line(e, equipment, Side::Credit, usd(100_000))
            .unwrap();
        b.s.attach_receipt(Some(e), "r.pdf", "application/pdf", b"%PDF r", None)
            .unwrap();
        b.s.post_entry(e).unwrap();
        let (_dir, books) = b.books();
        let items = find(&books);
        let sold = items
            .iter()
            .find(|a| a.title == format!("Equipment sold for an IOU: entry #{e}"))
            .unwrap_or_else(|| panic!("{items:?}"));
        assert_eq!(sold.severity, Severity::Todo);
        assert_eq!(sold.target, Some(Route::Entry(e)));
        assert!(
            sold.detail.contains("books.investing_accounts"),
            "{}",
            sold.detail
        );
        assert!(sold.prompt.starts_with("In wanderbooks,"));
        let on_card = items
            .iter()
            .find(|a| {
                a.title == format!("Equipment bought on an operating payable: entry #{bought}")
            })
            .unwrap_or_else(|| panic!("{items:?}"));
        assert_eq!(on_card.severity, Severity::Info);
        assert_eq!(
            items
                .iter()
                .filter(|a| a.title.starts_with("Equipment"))
                .count(),
            2
        );
    }

    /// INVARIANTS 2 and 4: a dated rule past its review is one Todo item
    /// whose prompt asks the agent to find the figure from the official
    /// authority (named, never the repo's copy), confirm it with the human
    /// and pass it explicitly, and to tell the human so it can be reported
    /// upstream; before the review there is no such item. No prompt points
    /// the agent at files, git, GitHub or pull requests.
    #[test]
    fn a_stale_rule_asks_for_the_authority_and_the_humans_confirmation() {
        let rule = ledger_core::rules::THRESHOLD_1099;
        let review = rule.review_after();
        assert!(
            find(&books("fresh", review.pred_opt().unwrap()))
                .iter()
                .all(|a| !a.title.contains(rule.name)),
            "no item before the review date"
        );
        let items = find(&books("stale", review));
        let stale: Vec<_> = items
            .iter()
            .filter(|a| a.title == format!("Check the {}", rule.name))
            .collect();
        assert_eq!(stale.len(), 1, "{items:?}");
        let a = stale[0];
        assert_eq!(a.severity, Severity::Todo);
        assert!(
            a.detail.contains(&rule.known_through.to_string()),
            "{}",
            a.detail
        );
        for want in [
            "In wanderbooks,",
            rule.name,
            &rule.known_through.to_string(),
            rule.authority,
            "confirm it with me",
            "pass it explicitly",
            "report it upstream",
        ] {
            assert!(a.prompt.contains(want), "{want:?} missing: {}", a.prompt);
        }
        for item in &items {
            let lower = item.prompt.to_lowercase();
            for never in ["pull request", ".md", "docs/", rule.source] {
                assert!(!lower.contains(never), "{never:?} in {}", item.prompt);
            }
            for word in lower.split(|c: char| !c.is_alphanumeric()) {
                assert!(
                    !["git", "github", "repo", "repository", "issue"].contains(&word),
                    "{word:?} in {}",
                    item.prompt
                );
            }
        }
    }
}
