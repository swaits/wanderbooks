//! The journal as IIF, for QuickBooks Desktop: `books.export` format iif
//! and the package's `general-journal.iif`.
//!
//! ```text
//! GOAL: the CPA imports every posted entry into QuickBooks Desktop, each
//!       account declared with the type QuickBooks needs.
//! SOURCE: ledger_import::iif (the IIF format and account types).
//! INVARIANTS:
//!   1. The cash accounts are BANK; the receivable and payable control
//!      accounts AR and AP; a registered asset's cost and accumulated
//!      depreciation FIXASSET; every other account follows its type.
//!   2. Every posted entry, and only those.
//! ```

use ledger_core::{AccountId, AccountType, Ledger};

/// Render the journal as IIF text via ledger-import, declaring every
/// account with its QuickBooks type: the cash accounts (`cash_codes`, the
/// books' `cash_accounts`) are BANK, the receivable and payable control
/// accounts AR and AP, a registered asset's cost and accumulated
/// depreciation FIXASSET, and the rest follow their wanderbooks type
/// (other current asset, other current liability, equity, income,
/// expense).
pub fn export_iif(ledger: &Ledger, cash_codes: &[String]) -> Result<String, String> {
    use ledger_import::{QbAccountType as Qb, iif_accounts, journal_to_iif};
    let fixed: std::collections::HashSet<AccountId> = ledger
        .registered_assets()
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
    let entries: Vec<_> = ledger.posted_entries().collect();
    journal_to_iif(&accounts, &entries).map_err(|e| e.to_string())
}
