//! Generate Intuit Interchange Format (IIF) journal entries for QuickBooks
//! Desktop, which many CPAs still run.
//!
//! Layout per entry:
//!
//! ```text
//! !TRNS	TRNSTYPE	DATE	ACCNT	NAME	AMOUNT	DOCNUM	MEMO
//! !SPL	TRNSTYPE	DATE	ACCNT	AMOUNT	DOCNUM	MEMO
//! !ENDTRNS
//! TRNS	GENERAL JOURNAL	01/15/2026	Cash		1000.00	1	Memo
//! SPL	GENERAL JOURNAL	01/15/2026	Revenue	-1000.00	1	Memo
//! ENDTRNS
//! ```
//!
//! Amounts are signed from the entry's perspective: debits positive,
//! credits negative. Dates are `MM/DD/YYYY`. Accounts are referenced by
//! name (QuickBooks matches on name).

use ledger_core::{JournalEntry, Side};
use rust_decimal::Decimal;

/// One entry's data for IIF export.
pub struct IifEntry<'a> {
    /// The journal entry.
    pub entry: &'a JournalEntry,
    /// Account id → account name, for resolving line accounts.
    pub account_names: &'a dyn Fn(ledger_core::AccountId) -> String,
}

/// Render posted entries as IIF text. `docnum` numbers entries sequentially
/// from 1 in the order given.
pub fn journal_to_iif(entries: &[IifEntry<'_>]) -> String {
    let mut out = String::new();
    out.push_str("!TRNS\tTRNSTYPE\tDATE\tACCNT\tNAME\tAMOUNT\tDOCNUM\tMEMO\n");
    out.push_str("!SPL\tTRNSTYPE\tDATE\tACCNT\tAMOUNT\tDOCNUM\tMEMO\n");
    out.push_str("!ENDTRNS\n");
    for (idx, item) in entries.iter().enumerate() {
        let docnum = idx + 1;
        let date = item.entry.date().format("%m/%d/%Y").to_string();
        let memo = sanitize(&item.entry.description().replace('\t', " "));
        let mut lines: Vec<_> = item.entry.lines().iter().collect();
        // TRNS carries the first debit (or first line); the rest are SPLs.
        lines.sort_by_key(|l| match l.side {
            Side::Debit => 0,
            Side::Credit => 1,
        });
        let (first, rest) = lines.split_first().expect("entry has lines");
        let first_acct = sanitize(&(item.account_names)(first.account));
        let first_amount = signed(first.side, first.amount.amount);
        out.push_str(&format!(
            "TRNS\tGENERAL JOURNAL\t{date}\t{first_acct}\t\t{}\t{docnum}\t{memo}\n",
            format_amount(first_amount),
        ));
        for line in rest {
            let acct = sanitize(&(item.account_names)(line.account));
            let amount = signed(line.side, line.amount.amount);
            out.push_str(&format!(
                "SPL\tGENERAL JOURNAL\t{date}\t{acct}\t{}\t{docnum}\t{memo}\n",
                format_amount(amount),
            ));
        }
        out.push_str("ENDTRNS\n");
    }
    out
}

/// Debit positive, credit negative.
fn signed(side: Side, amount: Decimal) -> Decimal {
    match side {
        Side::Debit => amount,
        Side::Credit => -amount,
    }
}

fn format_amount(d: Decimal) -> String {
    // Two decimal places, no thousands separators.
    format!("{d:.2}")
}

/// Strip tabs/newlines that would corrupt the tab-delimited format.
fn sanitize(s: &str) -> String {
    s.replace(['\t', '\n', '\r'], " ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use ledger_core::{AccountType, Currency, Ledger, Money};
    use rust_decimal::Decimal;

    #[test]
    fn iif_round_trip_shape() {
        let mut ledger = Ledger::new(Currency::USD);
        let cash = ledger
            .add_account("1000", "Cash", AccountType::Asset, None, false)
            .unwrap();
        let rev = ledger
            .add_account("4000", "Revenue", AccountType::Revenue, None, false)
            .unwrap();
        let id = ledger.create_entry(
            chrono::NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
            "Sale",
        );
        ledger
            .add_line(
                id,
                cash,
                Side::Debit,
                Money::new(Decimal::new(100000, 2), Currency::USD),
            )
            .unwrap();
        ledger
            .add_line(
                id,
                rev,
                Side::Credit,
                Money::new(Decimal::new(100000, 2), Currency::USD),
            )
            .unwrap();
        ledger.post_entry(id).unwrap();
        let entry = ledger.get_entry(id).unwrap();
        let names = |a: ledger_core::AccountId| {
            ledger
                .chart()
                .get(a)
                .map(|ac| ac.name.clone())
                .unwrap_or_default()
        };
        let text = journal_to_iif(&[IifEntry {
            entry,
            account_names: &names,
        }]);
        assert!(text.contains("!TRNS\tTRNSTYPE\tDATE\tACCNT\tNAME\tAMOUNT\tDOCNUM\tMEMO"));
        assert!(text.contains("TRNS\tGENERAL JOURNAL\t01/15/2026\tCash"));
        assert!(text.contains("\t1000.00\t"));
        assert!(text.contains("SPL\tGENERAL JOURNAL\t01/15/2026\tRevenue\t-1000.00"));
        assert!(text.contains("ENDTRNS"));
    }
}
