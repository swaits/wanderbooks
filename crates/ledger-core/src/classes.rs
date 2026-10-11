//! Classes: divisional P&L tracking.
//!
//! A class is a label on a journal line — a department, location, product
//! line, or job. Reports can filter the P&L to one class or break it down
//! by class, so one ledger serves several divisions without splitting the
//! chart of accounts. Classes live on lines, not accounts: one entry can
//! touch several divisions.
//!
//! Classes are master data, journaled like accounts. Tagging a line happens
//! on drafts only; posted entries are immutable, so their classes are frozen.

use serde::{Deserialize, Serialize};

/// Opaque class handle. Created by [`crate::Ledger`]; never forged.
pub type ClassId = u32;

/// A named division for line-level P&L tracking.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Class {
    /// Class id (the allocator's).
    pub id: ClassId,
    /// Human name, e.g. `"Retail"` or `"Wholesale"`.
    pub name: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AccountType, Currency, Ledger, LedgerError, Money, Side};
    use chrono::NaiveDate;
    use rust_decimal::Decimal;

    fn d(cents: i64) -> Decimal {
        Decimal::new(cents, 2)
    }

    fn test_ledger() -> Ledger {
        let mut ledger = Ledger::new(Currency::USD);
        ledger
            .add_account("1000", "Cash", AccountType::Asset, None, false)
            .unwrap();
        ledger
            .add_account("4000", "Revenue", AccountType::Revenue, None, false)
            .unwrap();
        ledger
            .add_account("6000", "Expenses", AccountType::Expense, None, false)
            .unwrap();
        ledger
    }

    fn post_sale(
        ledger: &mut Ledger,
        date: NaiveDate,
        revenue: Decimal,
        expense: Decimal,
        class: Option<ClassId>,
    ) {
        let cash = ledger.chart().find_by_code("1000").unwrap().id;
        let rev = ledger.chart().find_by_code("4000").unwrap().id;
        let exp = ledger.chart().find_by_code("6000").unwrap().id;
        let base = Currency::USD;
        let e = ledger.create_entry(date, "sale");
        ledger
            .add_line(e, cash, Side::Debit, Money::new(revenue, base))
            .unwrap();
        ledger
            .add_line(e, rev, Side::Credit, Money::new(revenue, base))
            .unwrap();
        if !expense.is_zero() {
            ledger
                .add_line(e, exp, Side::Debit, Money::new(expense, base))
                .unwrap();
            ledger
                .add_line(e, cash, Side::Credit, Money::new(expense, base))
                .unwrap();
        }
        if let Some(c) = class {
            // Tag the revenue and expense lines (indices 1 and 2).
            ledger.set_line_class(e, 1, Some(c)).unwrap();
            if !expense.is_zero() {
                ledger.set_line_class(e, 2, Some(c)).unwrap();
            }
        }
        ledger.post_entry(e).unwrap();
    }

    #[test]
    fn class_crud() {
        let mut ledger = test_ledger();
        assert_eq!(ledger.next_class_id(), 1);
        let retail = ledger.add_class("Retail").unwrap();
        let wholesale = ledger.add_class("Wholesale").unwrap();
        assert_eq!((retail, wholesale), (1, 2));
        assert_eq!(ledger.next_class_id(), 3);
        assert_eq!(ledger.get_class(retail).unwrap().name, "Retail");
        assert!(ledger.get_class(99).is_none());
        let names: Vec<_> = ledger
            .list_classes()
            .iter()
            .map(|c| c.name.clone())
            .collect();
        assert_eq!(names, vec!["Retail", "Wholesale"]);
        // Case-insensitive lookup.
        assert_eq!(ledger.find_class_by_name("retail").unwrap().id, retail);
        assert_eq!(ledger.find_class_by_name("RETAIL").unwrap().id, retail);
        assert!(ledger.find_class_by_name("nope").is_none());
        // Empty and whitespace names rejected.
        assert_eq!(ledger.add_class(""), Err(LedgerError::EmptyClassName));
        assert_eq!(ledger.add_class("   "), Err(LedgerError::EmptyClassName));
        // The failed adds did not move the allocator.
        assert_eq!(ledger.next_class_id(), 3);
    }

    #[test]
    fn set_line_class_draft_only() {
        let mut ledger = test_ledger();
        let class = ledger.add_class("Retail").unwrap();
        let cash = ledger.chart().find_by_code("1000").unwrap().id;
        let rev = ledger.chart().find_by_code("4000").unwrap().id;
        let date = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();

        let e = ledger.create_entry(date, "draft");
        ledger
            .add_line(e, cash, Side::Debit, Money::new(d(10_000), Currency::USD))
            .unwrap();
        ledger
            .add_line(e, rev, Side::Credit, Money::new(d(10_000), Currency::USD))
            .unwrap();
        // Unknown class rejected.
        assert_eq!(
            ledger.set_line_class(e, 0, Some(99)),
            Err(LedgerError::UnknownClass(99))
        );
        // Line index out of range rejected.
        assert_eq!(
            ledger.set_line_class(e, 7, Some(class)),
            Err(LedgerError::LineIndexOutOfRange(7, e))
        );
        // Set, then clear.
        ledger.set_line_class(e, 0, Some(class)).unwrap();
        assert_eq!(
            ledger.get_entry(e).unwrap().lines()[0].class_id,
            Some(class)
        );
        ledger.set_line_class(e, 0, None).unwrap();
        assert_eq!(ledger.get_entry(e).unwrap().lines()[0].class_id, None);
        // Post: classes freeze. Tagging a posted entry is rejected.
        ledger.post_entry(e).unwrap();
        assert_eq!(
            ledger.set_line_class(e, 0, Some(class)),
            Err(LedgerError::PostedEntryImmutable)
        );
        // Unknown entry rejected.
        assert_eq!(
            ledger.set_line_class(999, 0, Some(class)),
            Err(LedgerError::UnknownEntry(999))
        );
    }

    #[test]
    fn filtered_pl_math() {
        let mut ledger = test_ledger();
        let retail = ledger.add_class("Retail").unwrap();
        let wholesale = ledger.add_class("Wholesale").unwrap();
        let date = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
        post_sale(&mut ledger, date, d(100_000), d(40_000), Some(retail));
        post_sale(&mut ledger, date, d(50_000), d(10_000), Some(wholesale));
        post_sale(&mut ledger, date, d(20_000), d(5_000), None);

        let r = crate::reports::profit_and_loss_filtered(&ledger, Some(retail)).unwrap();
        assert_eq!(r.revenue, d(100_000));
        assert_eq!(r.expenses, d(40_000));
        assert_eq!(r.net_income(), d(60_000));
        let w = crate::reports::profit_and_loss_filtered(&ledger, Some(wholesale)).unwrap();
        assert_eq!(w.revenue, d(50_000));
        assert_eq!(w.expenses, d(10_000));
        // A class with no lines reports zeros.
        let empty = ledger.add_class("Empty").unwrap();
        let z = crate::reports::profit_and_loss_filtered(&ledger, Some(empty)).unwrap();
        assert_eq!((z.revenue, z.expenses), (Decimal::ZERO, Decimal::ZERO));
        // None counts every line: identical to the account-based P&L.
        let all = crate::reports::profit_and_loss_filtered(&ledger, None).unwrap();
        let plain = crate::reports::profit_and_loss(&ledger).unwrap();
        assert_eq!((all.revenue, all.expenses), (plain.revenue, plain.expenses));
        assert_eq!((all.revenue, all.expenses), (d(170_000), d(55_000)));
    }

    #[test]
    fn by_class_partitions_total_pl() {
        let mut ledger = test_ledger();
        let retail = ledger.add_class("Retail").unwrap();
        let wholesale = ledger.add_class("Wholesale").unwrap();
        let date = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
        post_sale(&mut ledger, date, d(100_000), d(40_000), Some(retail));
        post_sale(&mut ledger, date, d(50_000), d(10_000), Some(wholesale));
        post_sale(&mut ledger, date, d(20_000), d(5_000), None);

        let rows = crate::reports::profit_and_loss_by_class(&ledger).unwrap();
        assert_eq!(rows.len(), 3);
        // Sorted by class name; unclassified last.
        assert_eq!(rows[0].class.as_ref().unwrap().name, "Retail");
        assert_eq!(rows[1].class.as_ref().unwrap().name, "Wholesale");
        assert!(rows[2].class.is_none());
        assert_eq!(rows[0].profit_and_loss.net_income(), d(60_000));
        assert_eq!(rows[1].profit_and_loss.net_income(), d(40_000));
        assert_eq!(rows[2].profit_and_loss.net_income(), d(15_000));
        // The rows partition the ledger: they sum to the total P&L.
        let total = crate::reports::profit_and_loss(&ledger).unwrap();
        let rev: Decimal = rows.iter().map(|r| r.profit_and_loss.revenue).sum();
        let exp: Decimal = rows.iter().map(|r| r.profit_and_loss.expenses).sum();
        assert_eq!((rev, exp), (total.revenue, total.expenses));

        // A class with only balance-sheet lines gets no row.
        let cash_only = ledger.add_class("CashOnly").unwrap();
        let cash = ledger.chart().find_by_code("1000").unwrap().id;
        let e = ledger.create_entry(date, "cash move");
        ledger
            .add_line(e, cash, Side::Debit, Money::new(d(1_000), Currency::USD))
            .unwrap();
        // Need a second cash-ish line to balance; use revenue... no — use a
        // second asset account so no P&L line is tagged.
        let bank = ledger
            .add_account("1010", "Bank", AccountType::Asset, None, false)
            .unwrap();
        ledger
            .add_line(e, bank, Side::Credit, Money::new(d(1_000), Currency::USD))
            .unwrap();
        ledger.set_line_class(e, 0, Some(cash_only)).unwrap();
        ledger.set_line_class(e, 1, Some(cash_only)).unwrap();
        ledger.post_entry(e).unwrap();
        let rows = crate::reports::profit_and_loss_by_class(&ledger).unwrap();
        assert_eq!(rows.len(), 3, "balance-sheet-only class must not add a row");
        // Empty ledger: no rows at all.
        let empty = test_ledger();
        assert!(
            crate::reports::profit_and_loss_by_class(&empty)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn replay_class_checks_id() {
        let mut ledger = test_ledger();
        let err = ledger.replay_class(5, "Retail".to_string());
        assert_eq!(err, Err(LedgerError::ReplayIdMismatch(5, 1)));
        assert_eq!(ledger.next_class_id(), 1);
        ledger.replay_class(1, "Retail".to_string()).unwrap();
        assert_eq!(ledger.next_class_id(), 2);
        assert_eq!(ledger.get_class(1).unwrap().name, "Retail");
    }

    #[test]
    fn reversal_carries_class() {
        let mut ledger = test_ledger();
        let retail = ledger.add_class("Retail").unwrap();
        let date = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
        post_sale(&mut ledger, date, d(100_000), Decimal::ZERO, Some(retail));
        let entry_id = ledger.posted_entries().map(|e| e.id()).next().unwrap();
        let rev = ledger.reverse_entry(entry_id, date, "reversal").unwrap();
        let rev_entry = ledger.get_entry(rev).unwrap();
        // The flipped revenue line keeps its class; the cash lines were
        // never tagged.
        assert_eq!(rev_entry.lines()[0].class_id, None);
        assert_eq!(rev_entry.lines()[1].class_id, Some(retail));
        // The class P&L nets to zero after the reversal.
        let pl = crate::reports::profit_and_loss_filtered(&ledger, Some(retail)).unwrap();
        assert_eq!((pl.revenue, pl.expenses), (Decimal::ZERO, Decimal::ZERO));
    }
}
