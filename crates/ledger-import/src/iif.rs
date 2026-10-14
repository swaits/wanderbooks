//! Generate Intuit Interchange Format (IIF) journal entries for QuickBooks
//! Desktop, which many CPAs still run.
//!
//! ```text
//! GOAL: hand a CPA's QuickBooks Desktop the posted entries, landing on the
//!       right accounts, with nothing invented.
//! SOURCE: docs/spec/sources/intuit-iif.md (Intuit's IIF Overview, the IIF
//!         Import Kit's header help and its 2007+ General Journal example).
//! INVARIANTS:
//!   1. Accounts MUST be declared first (`!ACCNT`: NAME, ACCNTTYPE, ACCNUM)
//!      BECAUSE QuickBooks creates an unknown account "as a new bank
//!      account", and the kit says lists come before transactions.
//!   2. A subaccount's NAME MUST be its path, highest level first, joined by
//!      colons; a colon inside one account's own name is replaced, or it
//!      would read as another level.
//!   3. Each transaction is a TRNS line, its SPL lines, then ENDTRNS, all
//!      `GENERAL JOURNAL`, dated M/D/YYYY (the kit's 2007+ example), with
//!      debits positive and credits negative, so the lines sum to zero.
//!   4. Fields MUST hold no tab or line break (no escape exists) and only
//!      ASCII (IIF is ASCII text): common punctuation is transliterated,
//!      anything else becomes '?'.
//!   5. NEVER write an entry whose account wasn't declared: refuse instead.
//! ```
//!
//! Layout (each `→` is a tab):
//!
//! ```text
//! !ACCNT→NAME→ACCNTTYPE→ACCNUM
//! ACCNT→Cash→BANK→1000
//! !TRNS→TRNSID→TRNSTYPE→DATE→ACCNT→CLASS→AMOUNT→DOCNUM→MEMO
//! !SPL→SPLID→TRNSTYPE→DATE→ACCNT→CLASS→AMOUNT→DOCNUM→MEMO
//! !ENDTRNS
//! TRNS→→GENERAL JOURNAL→1/15/2026→Cash→→1000.00→1→Sale
//! SPL→→GENERAL JOURNAL→1/15/2026→Revenue→→-1000.00→1→Sale
//! ENDTRNS
//! ```

use chrono::Datelike;
use ledger_core::{AccountId, JournalEntry, Ledger, Side};
use rust_decimal::Decimal;
use std::collections::HashMap;
use thiserror::Error;

/// A QuickBooks account type: the IIF `ACCNTTYPE` keywords (Intuit's IIF
/// Import Kit, `!ACCNT` header help). Wanderbooks' five types don't say
/// which of these an account is (checking is `BANK`, not "other current
/// asset"), so the caller decides.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QbAccountType {
    /// `BANK`: checking or savings.
    Bank,
    /// `AR`: accounts receivable.
    AccountsReceivable,
    /// `OCASSET`: other current asset.
    OtherCurrentAsset,
    /// `FIXASSET`: fixed asset.
    FixedAsset,
    /// `OASSET`: other asset.
    OtherAsset,
    /// `AP`: accounts payable.
    AccountsPayable,
    /// `CCARD`: credit card.
    CreditCard,
    /// `OCLIAB`: other current liability.
    OtherCurrentLiability,
    /// `LTLIAB`: long term liability.
    LongTermLiability,
    /// `EQUITY`: capital/equity.
    Equity,
    /// `INC`: income.
    Income,
    /// `COGS`: cost of goods sold.
    CostOfGoodsSold,
    /// `EXP`: expense.
    Expense,
    /// `EXINC`: other income.
    OtherIncome,
    /// `EXEXP`: other expense.
    OtherExpense,
}

impl QbAccountType {
    /// The IIF keyword.
    pub fn keyword(self) -> &'static str {
        match self {
            Self::Bank => "BANK",
            Self::AccountsReceivable => "AR",
            Self::OtherCurrentAsset => "OCASSET",
            Self::FixedAsset => "FIXASSET",
            Self::OtherAsset => "OASSET",
            Self::AccountsPayable => "AP",
            Self::CreditCard => "CCARD",
            Self::OtherCurrentLiability => "OCLIAB",
            Self::LongTermLiability => "LTLIAB",
            Self::Equity => "EQUITY",
            Self::Income => "INC",
            Self::CostOfGoodsSold => "COGS",
            Self::Expense => "EXP",
            Self::OtherIncome => "EXINC",
            Self::OtherExpense => "EXEXP",
        }
    }
}

/// One account as QuickBooks will know it.
#[derive(Clone, Debug, PartialEq)]
pub struct IifAccount {
    /// The wanderbooks account.
    pub id: AccountId,
    /// Its name with its parents', highest first, joined by colons, each
    /// already made safe for IIF ([`iif_accounts`] builds these).
    pub path: String,
    /// Its account number (ACCNUM): the wanderbooks code.
    pub number: String,
    /// Its QuickBooks type.
    pub qb_type: QbAccountType,
}

/// Errors.
#[derive(Debug, Error, PartialEq)]
pub enum IifError {
    /// An entry uses an account that wasn't declared.
    #[error("entry {entry} uses account {account}, which isn't in the account list")]
    UndeclaredAccount {
        /// The entry.
        entry: u64,
        /// The account.
        account: AccountId,
    },
}

/// Every account in the chart as QuickBooks will know it: its colon path
/// (parents first) and number, with `qb_type` deciding its QuickBooks type.
pub fn iif_accounts(
    ledger: &Ledger,
    qb_type: impl Fn(&ledger_core::Account) -> QbAccountType,
) -> Vec<IifAccount> {
    let path = |mut id: AccountId| {
        let mut names = Vec::new();
        // A chart's parents form a tree; the bound guards a malformed one.
        for _ in 0..64 {
            let Some(a) = ledger.chart().get(id) else {
                break;
            };
            names.push(field(&a.name).replace(':', "-"));
            match a.parent {
                Some(p) => id = p,
                None => break,
            }
        }
        names.reverse();
        names.join(":")
    };
    let mut out: Vec<IifAccount> = ledger
        .chart()
        .all()
        .map(|a| IifAccount {
            id: a.id,
            path: path(a.id),
            number: field(&a.code),
            qb_type: qb_type(a),
        })
        .collect();
    out.sort_by(|a, b| a.number.cmp(&b.number));
    out
}

/// Render posted entries as IIF: the accounts declared first, then one
/// transaction per entry. `DOCNUM` numbers entries from 1 in the order
/// given.
pub fn journal_to_iif(
    accounts: &[IifAccount],
    entries: &[&JournalEntry],
) -> Result<String, IifError> {
    let by_id: HashMap<AccountId, &IifAccount> = accounts.iter().map(|a| (a.id, a)).collect();
    let mut out = String::new();
    out.push_str("!ACCNT\tNAME\tACCNTTYPE\tACCNUM\n");
    for a in accounts {
        out.push_str(&format!(
            "ACCNT\t{}\t{}\t{}\n",
            a.path,
            a.qb_type.keyword(),
            a.number
        ));
    }
    out.push_str("!TRNS\tTRNSID\tTRNSTYPE\tDATE\tACCNT\tCLASS\tAMOUNT\tDOCNUM\tMEMO\n");
    out.push_str("!SPL\tSPLID\tTRNSTYPE\tDATE\tACCNT\tCLASS\tAMOUNT\tDOCNUM\tMEMO\n");
    out.push_str("!ENDTRNS\n");
    for (idx, entry) in entries.iter().enumerate() {
        let docnum = idx + 1;
        let d = entry.date();
        let date = format!("{}/{}/{}", d.month(), d.day(), d.year());
        let memo = field(entry.description());
        // TRNS carries the first debit; the rest are SPLs.
        let mut lines: Vec<_> = entry.lines().iter().collect();
        lines.sort_by_key(|l| match l.side {
            Side::Debit => 0,
            Side::Credit => 1,
        });
        for (n, line) in lines.iter().enumerate() {
            let account = by_id
                .get(&line.account)
                .ok_or(IifError::UndeclaredAccount {
                    entry: entry.id(),
                    account: line.account,
                })?;
            let keyword = if n == 0 { "TRNS" } else { "SPL" };
            out.push_str(&format!(
                "{keyword}\t\tGENERAL JOURNAL\t{date}\t{}\t\t{}\t{docnum}\t{memo}\n",
                account.path,
                format_amount(signed(line.side, line.amount.amount)),
            ));
        }
        out.push_str("ENDTRNS\n");
    }
    Ok(out)
}

/// Debit positive, credit negative (Intuit: "Debit amounts are always
/// positive, credit amounts are always negative").
fn signed(side: Side, amount: Decimal) -> Decimal {
    match side {
        Side::Debit => amount,
        Side::Credit => -amount,
    }
}

/// Two decimal places, no thousands separators.
fn format_amount(d: Decimal) -> String {
    format!("{d:.2}")
}

/// A field IIF can carry: no tab or line break (there is no escape), and
/// ASCII only, common punctuation transliterated.
fn field(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '\t' | '\n' | '\r' => ' ',
            '\u{2013}' | '\u{2014}' | '\u{2212}' => '-',
            '\u{2018}' | '\u{2019}' => '\'',
            '\u{201C}' | '\u{201D}' => '"',
            '\u{00A0}' => ' ',
            c if c.is_ascii() && !c.is_ascii_control() => c,
            _ => '?',
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ledger_core::{AccountType, Currency, Money};

    #[test]
    fn iif_declares_accounts_then_journals() {
        let mut ledger = Ledger::new(Currency::USD);
        let cash = ledger
            .add_account("1000", "Cash", AccountType::Asset, None, false)
            .unwrap();
        let rev = ledger
            .add_account("4000", "Revenue", AccountType::Revenue, None, false)
            .unwrap();
        let id = ledger.create_entry(chrono::NaiveDate::from_ymd_opt(2026, 1, 5).unwrap(), "Sale");
        let m = Money::new(Decimal::new(100000, 2), Currency::USD);
        ledger.add_line(id, cash, Side::Debit, m).unwrap();
        ledger.add_line(id, rev, Side::Credit, m).unwrap();
        ledger.post_entry(id).unwrap();
        let accounts = iif_accounts(&ledger, |a| match a.account_type {
            AccountType::Asset => QbAccountType::Bank,
            _ => QbAccountType::Income,
        });
        let text = journal_to_iif(&accounts, &[ledger.get_entry(id).unwrap()]).unwrap();
        assert_eq!(
            text,
            "!ACCNT\tNAME\tACCNTTYPE\tACCNUM\n\
             ACCNT\tCash\tBANK\t1000\n\
             ACCNT\tRevenue\tINC\t4000\n\
             !TRNS\tTRNSID\tTRNSTYPE\tDATE\tACCNT\tCLASS\tAMOUNT\tDOCNUM\tMEMO\n\
             !SPL\tSPLID\tTRNSTYPE\tDATE\tACCNT\tCLASS\tAMOUNT\tDOCNUM\tMEMO\n\
             !ENDTRNS\n\
             TRNS\t\tGENERAL JOURNAL\t1/5/2026\tCash\t\t1000.00\t1\tSale\n\
             SPL\t\tGENERAL JOURNAL\t1/5/2026\tRevenue\t\t-1000.00\t1\tSale\n\
             ENDTRNS\n"
        );
    }
}
