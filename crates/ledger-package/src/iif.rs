//! The journal as IIF, for QuickBooks Desktop: `books.export` format iif
//! and the package's `general-journal.iif`.
//!
//! ```text
//! GOAL: the CPA imports the books through a day into QuickBooks Desktop,
//!       each account declared with the type QuickBooks needs, and gets
//!       the balances and the profit of the package's statements.
//! SOURCE: ledger_import::iif (the IIF format and account types); rules.md
//!         B-33 (as of a day), B-34; DECISION (owner, 2026-10-20, plan
//!         D-36: closing entries are left out).
//! INVARIANTS:
//!   1. The cash accounts are BANK; the receivable and payable control
//!      accounts AR and AP; the cost and accumulated depreciation accounts
//!      of an asset bought on or before as_of FIXASSET; every other
//!      account follows its type. Every account in the chart is declared.
//!   2. Every posted entry dated on or before as_of, from the first, in
//!      the order posted, except period closes: QuickBooks keeps no
//!      closing entries (it works out retained earnings itself), so an
//!      imported close would zero its period's profit. The file's
//!      balances are the trial balance as of as_of without the closes.
//! ```

use chrono::NaiveDate;
use ledger_core::{AccountId, AccountType, Ledger};

/// The journal through `as_of` as IIF, via ledger-import (INVARIANTS 1-2),
/// declaring every account with its QuickBooks type: the cash accounts
/// (`cash_codes`, the books' `cash_accounts`) are BANK, the receivable and
/// payable control accounts AR and AP, the cost and accumulated
/// depreciation accounts of an asset bought by `as_of` FIXASSET, and the
/// rest follow their wanderbooks type (other current asset, other current
/// liability, equity, income, expense). Returns the file and every text it
/// writes differently from the books.
pub fn export_iif(
    ledger: &Ledger,
    cash_codes: &[String],
    as_of: NaiveDate,
) -> Result<ledger_import::Iif, String> {
    use ledger_import::{QbAccountType as Qb, iif_accounts, journal_to_iif};
    let fixed: std::collections::HashSet<AccountId> = ledger
        .registered_assets()
        .filter(|a| a.purchase_date <= as_of)
        .flat_map(|a| [a.cost_account, a.accum_depr_account])
        .collect();
    let receivables = ledger.role_account(&ledger_core::RECEIVABLES);
    let payables = ledger.role_account(&ledger_core::PAYABLES);
    let accounts = iif_accounts(ledger, |a| {
        if cash_codes.contains(&a.code) {
            Qb::Bank
        } else if Some(a.id) == receivables {
            Qb::AccountsReceivable
        } else if Some(a.id) == payables {
            Qb::AccountsPayable
        } else if fixed.contains(&a.id) {
            Qb::FixedAsset
        } else {
            match a.account_type {
                AccountType::Asset => Qb::OtherCurrentAsset,
                AccountType::Liability => Qb::OtherCurrentLiability,
                AccountType::Equity => Qb::Equity,
                AccountType::Revenue => Qb::Income,
                AccountType::Expense => Qb::Expense,
            }
        }
    });
    let closes = ledger.closing_entry_ids();
    let entries: Vec<_> = ledger
        .posted_entries()
        .filter(|e| e.date() <= as_of && !closes.contains(&e.id()))
        .collect();
    journal_to_iif(&accounts, &entries).map_err(|e| e.to_string())
}
