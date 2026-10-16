//! Which accounts the monthly close reconciles, and when one is due: one
//! rule, read by the server's `books.digest` (`unreconciled_accounts`) and
//! the viewer's Needs attention and account pages alike.
//!
//! ```text
//! GOAL: the monthly close's reconciliation step can't be forgotten, and
//!       the digest and the viewer never disagree on what is due.
//! SOURCE: docs/parties-and-attachments.md "Reconciliations" (Needs
//!         attention: an account with activity not reconciled through last
//!         month's end); CONVENTION (bank and card statements close
//!         monthly); AGENTS.md's monthly close.
//! DEFINITIONS:
//!   cash accounts: `[books] cash_accounts`, resolved; when none are set,
//!     the asset accounts (not placeholders) whose names say they hold
//!     cash ("cash", "bank", "checking", "savings", "money market").
//!   card accounts: liability accounts whose names contain "card".
//! INVARIANTS:
//!   1. reconcilable_accounts: the cash accounts, then the card accounts,
//!      then every other liability with a recorded statement, each once.
//!      A card is expected before its first statement, so its first
//!      reconciliation is asked for; a liability named otherwise (an Amex)
//!      joins once its first statement is recorded.
//!   2. reconciliation_due(account) = Some(through), through the last day
//!      of the month before today's, iff the account has a posted line
//!      dated on or before it and no reconciliation not voided reaches it;
//!      None otherwise (including activity only this month).
//! ```

use crate::{Reconciliation, Statement};
use chrono::{Datelike, NaiveDate};
use ledger_core::{AccountId, AccountType, Ledger};

/// The words an asset account's name uses to say it holds cash.
pub const CASH_WORDS: [&str; 5] = ["cash", "bank", "checking", "savings", "money market"];

/// The cash accounts: `configured` (account codes) resolved, else the
/// asset accounts whose names say they hold cash.
pub fn cash_accounts(ledger: &Ledger, configured: &[String]) -> Vec<AccountId> {
    if !configured.is_empty() {
        return configured
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
                && CASH_WORDS.iter().any(|w| n.contains(w))
        })
        .map(|a| a.id)
        .collect()
}

/// The card accounts: liabilities whose names contain "card".
pub fn card_accounts(ledger: &Ledger) -> Vec<AccountId> {
    ledger
        .chart()
        .all()
        .filter(|a| {
            a.account_type == AccountType::Liability && a.name.to_lowercase().contains("card")
        })
        .map(|a| a.id)
        .collect()
}

/// The accounts the books reconcile monthly (INVARIANT 1).
pub fn reconcilable_accounts(
    ledger: &Ledger,
    cash: &[AccountId],
    statements: &[Statement],
) -> Vec<AccountId> {
    let mut out: Vec<AccountId> = Vec::new();
    let with_statement = statements.iter().map(|s| s.account_id).filter(|&a| {
        ledger
            .chart()
            .get(a)
            .is_some_and(|x| x.account_type == AccountType::Liability)
    });
    for a in cash
        .iter()
        .copied()
        .chain(card_accounts(ledger))
        .chain(with_statement)
    {
        if !out.contains(&a) {
            out.push(a);
        }
    }
    out
}

/// The last day of the month before `today`'s.
pub fn last_month_end(today: NaiveDate) -> NaiveDate {
    today
        .with_day(1)
        .and_then(|d| d.pred_opt())
        .expect("a month before today")
}

/// The latest statement date `account` is reconciled through: its newest
/// reconciliation not voided.
pub fn reconciled_through(
    reconciliations: &[Reconciliation],
    account: AccountId,
) -> Option<NaiveDate> {
    reconciliations
        .iter()
        .filter(|r| r.account_id == account && r.voided.is_none())
        .map(|r| r.statement_date)
        .max()
}

/// When `account` is due a reconciliation (INVARIANT 2).
pub fn reconciliation_due(
    ledger: &Ledger,
    reconciliations: &[Reconciliation],
    account: AccountId,
    today: NaiveDate,
) -> Option<NaiveDate> {
    let through = last_month_end(today);
    let active = ledger
        .posted_entries()
        .any(|e| e.date() <= through && e.lines().iter().any(|l| l.account == account));
    (active && reconciled_through(reconciliations, account).is_none_or(|d| d < through))
        .then_some(through)
}
