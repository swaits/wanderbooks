//! Accounts receivable and payable: customers, vendors, invoices, bills.
//!
//! Invoices and bills are business documents that auto-post journal entries:
//!
//! * `create_invoice`: Dr `1200` Accounts Receivable, Cr `4000` Revenue.
//! * `create_bill`: Dr `6999` General Expenses, Cr `2000` Accounts Payable.
//! * `receive_payment`: Dr Cash, Cr A/R. Updates the invoice's paid amount.
//! * `pay_bill`: Dr A/P, Cr Cash. Updates the bill's paid amount.
//!
//! The control accounts (`1200`, `2000`) and the default revenue/expense
//! accounts (`4000`, `6999`) are created on demand if missing. Invoices and
//! bills respect closed periods: they cannot be created with dates in a
//! closed period.

use crate::LedgerError;
use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// Opaque customer handle.
pub type CustomerId = u64;
/// Opaque vendor handle.
pub type VendorId = u64;
/// Opaque invoice handle.
pub type InvoiceId = u64;
/// Opaque bill handle.
pub type BillId = u64;

/// A customer: someone we invoice.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Customer {
    /// Opaque handle.
    pub id: CustomerId,
    /// Display name.
    pub name: String,
}

/// A vendor: someone who bills us.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Vendor {
    /// Opaque handle.
    pub id: VendorId,
    /// Display name.
    pub name: String,
}

/// Payment status of an invoice or bill.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum DocumentStatus {
    /// Nothing paid yet.
    Open,
    /// Partially paid.
    Partial,
    /// Fully paid.
    Paid,
}

/// One line on an invoice or bill.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DocumentLine {
    /// Description of the goods/services.
    pub description: String,
    /// Line amount (positive).
    pub amount: Decimal,
}

/// A sales invoice: we billed a customer.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Invoice {
    /// Opaque handle.
    pub id: InvoiceId,
    /// The customer.
    pub customer_id: CustomerId,
    /// Invoice date.
    pub date: NaiveDate,
    /// When payment is due.
    pub due_date: NaiveDate,
    /// Line items.
    pub lines: Vec<DocumentLine>,
    /// How much has been paid.
    pub amount_paid: Decimal,
    /// Derived from amount_paid vs total.
    pub status: DocumentStatus,
    /// The auto-posted journal entry.
    pub entry_id: u64,
}

impl Invoice {
    /// Total of all lines.
    pub fn total(&self) -> Decimal {
        self.lines.iter().map(|l| l.amount).sum()
    }

    /// Amount still owed.
    pub fn balance(&self) -> Decimal {
        self.total() - self.amount_paid
    }
}

/// A vendor bill: we owe a vendor.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Bill {
    /// Opaque handle.
    pub id: BillId,
    /// The vendor.
    pub vendor_id: VendorId,
    /// Bill date.
    pub date: NaiveDate,
    /// When payment is due.
    pub due_date: NaiveDate,
    /// Line items.
    pub lines: Vec<DocumentLine>,
    /// How much has been paid.
    pub amount_paid: Decimal,
    /// Derived from amount_paid vs total.
    pub status: DocumentStatus,
    /// The auto-posted journal entry.
    pub entry_id: u64,
}

impl Bill {
    /// Total of all lines.
    pub fn total(&self) -> Decimal {
        self.lines.iter().map(|l| l.amount).sum()
    }

    /// Amount still owed.
    pub fn balance(&self) -> Decimal {
        self.total() - self.amount_paid
    }
}

/// Validate document lines: at least one, all positive amounts.
pub fn validate_lines(lines: &[DocumentLine]) -> Result<(), LedgerError> {
    if lines.is_empty() {
        return Err(LedgerError::EmptyDocumentLines);
    }
    for line in lines {
        if line.amount <= Decimal::ZERO {
            return Err(LedgerError::InvalidDocumentLineAmount(line.amount));
        }
    }
    Ok(())
}

/// Validate that due date is not before the document date.
pub fn validate_due_date(date: NaiveDate, due_date: NaiveDate) -> Result<(), LedgerError> {
    if due_date < date {
        return Err(LedgerError::DueBeforeDate(due_date.to_string()));
    }
    Ok(())
}

/// Derive status from paid vs total.
pub fn derive_status(paid: Decimal, total: Decimal) -> DocumentStatus {
    if paid.is_zero() {
        DocumentStatus::Open
    } else if paid >= total {
        DocumentStatus::Paid
    } else {
        DocumentStatus::Partial
    }
}

/// One row of an aging report.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgingRow {
    /// Customer or vendor name.
    pub name: String,
    /// Not yet due.
    pub current: Decimal,
    /// 1-30 days past due.
    pub days_30: Decimal,
    /// 31-60 days past due.
    pub days_60: Decimal,
    /// 61-90 days past due.
    pub days_90: Decimal,
    /// Over 90 days past due.
    pub days_90_plus: Decimal,
}

impl AgingRow {
    /// Total outstanding.
    pub fn total(&self) -> Decimal {
        self.current + self.days_30 + self.days_60 + self.days_90 + self.days_90_plus
    }
}

/// Bucket a past-due balance by days overdue.
pub fn aging_bucket(due_date: NaiveDate, as_of: NaiveDate, balance: Decimal) -> AgingRow {
    let mut row = AgingRow {
        name: String::new(),
        current: Decimal::ZERO,
        days_30: Decimal::ZERO,
        days_60: Decimal::ZERO,
        days_90: Decimal::ZERO,
        days_90_plus: Decimal::ZERO,
    };
    if due_date > as_of {
        row.current = balance;
    } else {
        let days = (as_of - due_date).num_days();
        if days <= 30 {
            row.days_30 = balance;
        } else if days <= 60 {
            row.days_60 = balance;
        } else if days <= 90 {
            row.days_90 = balance;
        } else {
            row.days_90_plus = balance;
        }
    }
    row
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AccountType, Currency, Ledger, Money, Side};

    fn arap_ledger() -> Ledger {
        let mut l = Ledger::new(Currency::USD);
        l.add_account("1000", "Cash", AccountType::Asset, None, false)
            .unwrap();
        l
    }

    #[test]
    fn invoice_lifecycle() {
        let mut l = arap_ledger();
        let cust = l.add_customer("Acme Corp").unwrap();
        let inv = l
            .create_invoice(
                cust,
                NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
                NaiveDate::from_ymd_opt(2026, 10, 31).unwrap(),
                vec![DocumentLine {
                    description: "Widgets".to_string(),
                    amount: "1000.00".parse().unwrap(),
                }],
            )
            .unwrap();
        let invoice = l.get_invoice(inv).unwrap();
        assert_eq!(invoice.total(), "1000.00".parse().unwrap());
        assert_eq!(invoice.status, DocumentStatus::Open);

        // A/R has a debit balance of 1000.
        let ar = l.chart().find_by_code("1200").unwrap().id;
        assert_eq!(l.account_balance(ar).unwrap(), "1000".parse().unwrap());

        // Partial payment.
        let cash = l.chart().find_by_code("1000").unwrap().id;
        l.receive_payment(
            inv,
            NaiveDate::from_ymd_opt(2026, 10, 15).unwrap(),
            "400.00".parse().unwrap(),
            cash,
        )
        .unwrap();
        let invoice = l.get_invoice(inv).unwrap();
        assert_eq!(invoice.status, DocumentStatus::Partial);
        assert_eq!(invoice.balance(), "600.00".parse().unwrap());

        // Full payment.
        l.receive_payment(
            inv,
            NaiveDate::from_ymd_opt(2026, 10, 20).unwrap(),
            "600.00".parse().unwrap(),
            cash,
        )
        .unwrap();
        let invoice = l.get_invoice(inv).unwrap();
        assert_eq!(invoice.status, DocumentStatus::Paid);
        assert_eq!(l.account_balance(ar).unwrap(), Decimal::ZERO);
    }

    #[test]
    fn bill_lifecycle() {
        let mut l = arap_ledger();
        let vend = l.add_vendor("Supply Co").unwrap();
        let bill = l
            .create_bill(
                vend,
                NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
                NaiveDate::from_ymd_opt(2026, 10, 31).unwrap(),
                vec![DocumentLine {
                    description: "Paper".to_string(),
                    amount: "250.00".parse().unwrap(),
                }],
            )
            .unwrap();
        let ap = l.chart().find_by_code("2000").unwrap().id;
        // A/P is credit-normal; balance should be 250 (credit).
        assert_eq!(l.account_balance(ap).unwrap(), "250".parse().unwrap());

        let cash = l.chart().find_by_code("1000").unwrap().id;
        // Seed cash so the payment doesn't go negative (not required, but tidy).
        let e = l.create_entry(NaiveDate::from_ymd_opt(2026, 9, 1).unwrap(), "seed");
        let m = Money::new("1000.00".parse().unwrap(), Currency::USD);
        l.add_line(e, cash, Side::Debit, m).unwrap();
        let eq = l
            .add_account("3000", "Equity", AccountType::Equity, None, false)
            .unwrap();
        l.add_line(e, eq, Side::Credit, m).unwrap();
        l.post_entry(e).unwrap();

        l.pay_bill(
            bill,
            NaiveDate::from_ymd_opt(2026, 10, 15).unwrap(),
            "250.00".parse().unwrap(),
            cash,
        )
        .unwrap();
        assert_eq!(l.get_bill(bill).unwrap().status, DocumentStatus::Paid);
        assert_eq!(l.account_balance(ap).unwrap(), Decimal::ZERO);
    }

    #[test]
    fn overpayment_rejected() {
        let mut l = arap_ledger();
        let cust = l.add_customer("Acme").unwrap();
        let inv = l
            .create_invoice(
                cust,
                NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
                NaiveDate::from_ymd_opt(2026, 10, 31).unwrap(),
                vec![DocumentLine {
                    description: "X".to_string(),
                    amount: "100.00".parse().unwrap(),
                }],
            )
            .unwrap();
        let cash = l.chart().find_by_code("1000").unwrap().id;
        let err = l.receive_payment(
            inv,
            NaiveDate::from_ymd_opt(2026, 10, 15).unwrap(),
            "150.00".parse().unwrap(),
            cash,
        );
        assert!(matches!(err, Err(LedgerError::Overpayment(_, _))));
    }

    #[test]
    fn ar_aging_buckets_correctly() {
        let mut l = arap_ledger();
        let cust = l.add_customer("Acme").unwrap();
        // Due in the future: current.
        l.create_invoice(
            cust,
            NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 11, 15).unwrap(),
            vec![DocumentLine {
                description: "A".to_string(),
                amount: "100.00".parse().unwrap(),
            }],
        )
        .unwrap();
        // 45 days past due: 60 bucket.
        l.create_invoice(
            cust,
            NaiveDate::from_ymd_opt(2026, 7, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 8, 17).unwrap(),
            vec![DocumentLine {
                description: "B".to_string(),
                amount: "200.00".parse().unwrap(),
            }],
        )
        .unwrap();
        let as_of = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
        let aging = l.ar_aging(as_of);
        assert_eq!(aging.len(), 1);
        assert_eq!(aging[0].current, "100".parse().unwrap());
        assert_eq!(aging[0].days_60, "200".parse().unwrap());
    }

    #[test]
    fn invoice_in_closed_period_rejected() {
        let mut l = arap_ledger();
        let cust = l.add_customer("Acme").unwrap();
        let pid = l
            .create_period(
                "2026-10",
                NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
                NaiveDate::from_ymd_opt(2026, 10, 31).unwrap(),
            )
            .unwrap();
        l.close_period(pid).unwrap();
        let err = l.create_invoice(
            cust,
            NaiveDate::from_ymd_opt(2026, 10, 15).unwrap(),
            NaiveDate::from_ymd_opt(2026, 11, 15).unwrap(),
            vec![DocumentLine {
                description: "X".to_string(),
                amount: "100.00".parse().unwrap(),
            }],
        );
        assert!(matches!(err, Err(LedgerError::ClosedPeriod(_, _))));
    }
}
