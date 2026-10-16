//! What the books need: drafts left unposted, missing receipts, schedules
//! and recurring entries due, open past periods, uncategorized tax lines,
//! 1099 vendors without a W-9, overdue bills and invoices, money owed to
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
use ledger_core::reports::profit_and_loss_for_period;
use ledger_core::{AccountType, EntryStatus, PeriodStatus, vendor_1099_report};
use rust_decimal::Decimal;

/// Everything the books need, as items to show.
///
/// ```text
/// GOAL: tell the humans what's unfinished or wrong, and hand them the
///       prompt that gets the agent to fix it; never change the books.
/// SOURCE: DECISION (owner: the viewer is read-only; its "Needs
///         attention" list offers prompts, AGENTS.md); the conditions
///         below; docs/spec/sources/irs-1099-thresholds.md (W-9s).
/// INVARIANTS:
///   1. Pure: reads `b`, writes nothing.
///   2. One item per: draft entry (Todo); posted entry missing a receipt
///      (Problem; four or more are one combined item); recurring entry or
///      schedule due through today (Todo); period ended and still open
///      (Todo); fiscal-year account with activity and no tax category
///      (Info); legal rule known only through a past year (Todo); 1099
///      vendor without a W-9 (Problem); overdue bills, and overdue
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
///      books.set_entry_party).
///   3. Nothing else: books with none of those yield no items.
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
                fmt::dollars(e.amount)
            ),
            target: Some(Route::Entry(e.id)),
            prompt: format!(
                "In wanderbooks, review draft entry {} (\"{}\", {}): attach its receipt if it needs one, preview it, and post it if it's right.",
                e.id,
                e.description,
                fmt::dollars(e.amount)
            ),
        });
    }

    let missing: Vec<_> = b
        .entries
        .iter()
        .filter(|e| e.status == EntryStatus::Posted && e.missing_receipt)
        .collect();
    if missing.len() > 3 {
        let total: Decimal = missing.iter().map(|e| e.amount).sum();
        let list: Vec<String> = missing
            .iter()
            .map(|e| format!("{} ({}, {})", e.id, e.description, e.date))
            .collect();
        out.push(Attention {
            severity: Severity::Problem,
            title: format!("{} without receipts", fmt::count(missing.len(), "entry", "entries")),
            detail: format!(
                "{} in entries with no receipt attached ({}).",
                fmt::dollars(total),
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
                fmt::dollars(e.amount),
                b.receipt_rule()
            ),
            target: Some(Route::Entry(e.id)),
            prompt: format!(
                "In wanderbooks, entry {} (\"{}\", {} on {}) has no receipt. Find it and attach it with books.attach_receipt.",
                e.id,
                e.description,
                fmt::dollars(e.amount),
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
        let due = s.due(today);
        if !due.is_empty() {
            let total: Decimal = due.iter().map(|(_, a)| *a).sum();
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

    let (fy_start, _) = b.fiscal_ytd();
    if let Ok(pl) = profit_and_loss_for_period(&b.ledger, fy_start, today, None) {
        let uncategorized: Vec<String> = pl
            .revenue
            .iter()
            .chain(&pl.expenses)
            .filter(|l| {
                b.ledger
                    .chart()
                    .find_by_code(&l.code)
                    .is_some_and(|a| a.tax_category.is_none())
            })
            .map(|l| format!("{} {}", l.code, l.name))
            .collect();
        if !uncategorized.is_empty() {
            out.push(Attention {
                severity: Severity::Info,
                title: format!(
                    "{} without a tax category",
                    fmt::count(uncategorized.len(), "account", "accounts")
                ),
                detail: format!(
                    "{}. The tax summary lists them as uncategorized.",
                    uncategorized.join(", ")
                ),
                target: Some(Route::Accounts),
                prompt: format!(
                    "In wanderbooks, propose a tax category for each of these accounts ({}) using books.tax_categories, and set them with books.set_tax_category once I agree.",
                    uncategorized.join(", ")
                ),
            });
        }
    }

    // Rules set by law that wanderbooks may no longer know (rules.md;
    // owner, 2026-10-13): research or ask, then update them for everyone.
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
                "wanderbooks' {} is known only through {}. Find the current figure from the official source (see {} in the wanderbooks repo), or ask me or our CPA, and tell me what you found. Then update the rule for everyone: open a pull request at https://github.com/swaits/wanderbooks with the new value and its official source (or at least an issue).",
                rule.name, rule.known_through, rule.source
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
        let Ok(r) = vendor_1099_report(&b.ledger, year, None) else {
            continue;
        };
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
                    "Remind me to request a W-9 from {}; once I have it, mark it on file in wanderbooks with books.set_vendor_1099.",
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

    let overdue = |rows: Vec<ledger_core::AgingRow>| -> (usize, Decimal) {
        let late: Vec<Decimal> = rows
            .iter()
            .map(|r| r.days_30 + r.days_60 + r.days_90 + r.days_90_plus)
            .filter(|d| !d.is_zero())
            .collect();
        (late.len(), late.iter().sum())
    };
    let (n, total) = overdue(b.ledger.ap_aging(today));
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
    let (n, total) = overdue(b.ledger.ar_aging(today));
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

    let due = b.depreciation_due(today);
    if due.amount > Decimal::ZERO {
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
    let register = b.asset_register(today);
    let unregistered = register.unregistered_cost();
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
    for a in b.ledger.registered_assets().filter(|a| {
        a.tag.is_none() && a.status == ledger_core::AssetStatus::Active && a.purchase_date <= today
    }) {
        out.push(Attention {
            severity: Severity::Todo,
            title: format!("No tag: {}", a.description),
            detail: format!(
                "Asset {}, in service {}. A tag is the label on the thing itself.",
                a.id,
                fmt::date(a.purchase_date)
            ),
            target: Some(Route::Asset(a.id)),
            prompt: format!(
                "In wanderbooks, label asset {} (\"{}\") with books.label_asset: give it the next generated tag unless I have my own, and its serial number if I can find it. Then print its label with books.asset_labels.",
                a.id, a.description
            ),
        });
    }

    for a in b.ledger.registered_assets().filter(|a| {
        a.tax_basis().is_none()
            && a.status == ledger_core::AssetStatus::Active
            && a.purchase_date <= today
    }) {
        let name = a.tag.clone().unwrap_or_else(|| format!("asset {}", a.id));
        out.push(Attention {
            severity: Severity::Todo,
            title: format!("No tax basis: {}", b.asset_label(a.id)),
            detail: format!(
                "Book cost {}, in service {}. The CPA needs its federal tax basis beside its book value.",
                fmt::dollars(a.cost.amount),
                fmt::date(a.purchase_date)
            ),
            target: Some(Route::Asset(a.id)),
            prompt: format!(
                "In wanderbooks, record the tax basis of {name} (\"{}\", book cost {}) with books.set_asset_tax_basis. Ask me how the company got it: bought (cost), contributed by a member from business use (carryover, IRC §723: the member's adjusted basis), or converted by a member from personal use (personal-use conversion, Treas. Reg. §1.168(i)-4(b): the member's adjusted basis; the tax basis is the lesser of that and its fair market value). Then show me the asset.",
                a.description,
                fmt::dollars(a.cost.amount)
            ),
        });
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
    out.sort_by_key(|a| a.severity);
    out
}
