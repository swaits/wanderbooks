//! What the books need: drafts left unposted, missing receipts, schedules
//! and recurring entries due, open past periods, uncategorized tax lines,
//! 1099 vendors without a W-9, overdue bills and invoices, money owed to
//! owners, and legal rules (like the 1099 threshold) due for an update. Each item says what's wrong, where to look, and the prompt that
//! asks the agent to fix it (the GUI never changes the books itself).

use crate::books::{Attention, Books, Severity};
use crate::fmt;
use crate::nav::Route;
use chrono::Datelike;
use ledger_core::reports::profit_and_loss_for_period;
use ledger_core::{AccountType, EntryStatus, PeriodStatus, vendor_1099_report};
use rust_decimal::Decimal;

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
            title: format!("{} without receipts", fmt::count(missing.len(), "expense", "expenses")),
            detail: format!(
                "{} of spending has no receipt attached ({}).",
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
    // Stephen, 2026-10-13): research or ask, then update them for everyone.
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
    for year in years {
        // A year whose threshold isn't known is flagged by the stale-rule
        // item above; never guess it here.
        let Ok(r) = vendor_1099_report(&b.ledger, year, None) else {
            continue;
        };
        for v in r.vendors.iter().filter(|v| v.reportable && !v.tin_on_file) {
            out.push(Attention {
                severity: Severity::Problem,
                title: format!("{} needs a W-9", v.name),
                detail: format!(
                    "Paid {} in {year}, over the {} 1099 threshold.",
                    fmt::dollars(v.paid),
                    fmt::dollars(r.threshold)
                ),
                target: Some(Route::Company),
                prompt: format!(
                    "Remind me to request a W-9 from {}; once I have it, mark it on file in wanderbooks with books.set_vendor_1099.",
                    v.name
                ),
            });
        }
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
