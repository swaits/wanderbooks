//! Parse QuickBooks Online bank/credit-card upload CSVs.
//!
//! QBO accepts two layouts:
//!
//! - 3 columns: `Date, Description, Amount` (positive = money in).
//! - 4 columns: `Date, Description, Expense, Income` (or `Debit, Credit`).
//!
//! Headers are optional; the layout is detected from the column count.
//! Dates accept `YYYY-MM-DD`, `MM/DD/YYYY`, and `MM/DD/YY`.

use chrono::NaiveDate;
use rust_decimal::Decimal;
use std::str::FromStr;
use thiserror::Error;

/// One parsed bank transaction.
#[derive(Clone, Debug, PartialEq)]
pub struct BankTransaction {
    /// Transaction date.
    pub date: NaiveDate,
    /// Bank's description.
    pub description: String,
    /// Signed amount: positive = money in, negative = money out.
    pub amount: Decimal,
}

/// Parse errors.
#[derive(Debug, Error, PartialEq)]
pub enum BankCsvError {
    /// The CSV has neither 3 nor 4 columns.
    #[error("expected 3 or 4 columns, found {0}")]
    BadColumnCount(usize),
    /// A date did not parse.
    #[error("bad date {0:?}")]
    BadDate(String),
    /// An amount did not parse.
    #[error("bad amount {0:?}")]
    BadAmount(String),
    /// The CSV itself is malformed.
    #[error("csv error: {0}")]
    Csv(String),
}

/// Parse a QBO bank CSV. Returns transactions in file order.
pub fn parse_bank_csv(text: &str) -> Result<Vec<BankTransaction>, BankCsvError> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .trim(csv::Trim::All)
        .from_reader(text.as_bytes());
    let mut out = Vec::new();
    for (idx, record) in reader.records().enumerate() {
        let record = record.map_err(|e| BankCsvError::Csv(e.to_string()))?;
        let fields: Vec<&str> = record.iter().collect();
        if fields.iter().all(|f| f.is_empty()) {
            continue; // skip blank lines
        }
        // Skip a header row.
        if idx == 0 && fields[0].eq_ignore_ascii_case("date") {
            continue;
        }
        let (date, description, amount) = match fields.len() {
            3 => {
                let amount = parse_amount(fields[2])?;
                (fields[0], fields[1], amount)
            }
            4 => {
                // Expense/Debit in col 2, Income/Credit in col 3. Exactly one
                // should be non-empty; a row with both is ambiguous.
                let out_amt = fields[2].trim();
                let in_amt = fields[3].trim();
                let amount = match (!out_amt.is_empty(), !in_amt.is_empty()) {
                    (true, false) => -parse_amount(out_amt)?,
                    (false, true) => parse_amount(in_amt)?,
                    (false, false) => Decimal::ZERO,
                    (true, true) => {
                        return Err(BankCsvError::Csv(format!(
                            "row {} has both expense and income amounts",
                            idx + 1
                        )));
                    }
                };
                (fields[0], fields[1], amount)
            }
            n => return Err(BankCsvError::BadColumnCount(n)),
        };
        if amount.is_zero() {
            continue; // zero-amount rows carry no money
        }
        out.push(BankTransaction {
            date: parse_date(date)?,
            description: description.to_string(),
            amount,
        });
    }
    Ok(out)
}

fn parse_date(s: &str) -> Result<NaiveDate, BankCsvError> {
    let s = s.trim();
    // YYYY-MM-DD
    if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Ok(d);
    }
    // MM/DD/YYYY
    if let Ok(d) = NaiveDate::parse_from_str(s, "%m/%d/%Y") {
        return Ok(d);
    }
    // MM/DD/YY (assume 2000s)
    if let Ok(d) = NaiveDate::parse_from_str(s, "%m/%d/%y") {
        return Ok(d);
    }
    Err(BankCsvError::BadDate(s.to_string()))
}

fn parse_amount(s: &str) -> Result<Decimal, BankCsvError> {
    let cleaned: String = s.trim().replace(['$', ','], "");
    // Parentheses mean negative: (123.45).
    let (neg, inner) = match cleaned.strip_prefix('(').and_then(|t| t.strip_suffix(')')) {
        Some(inner) => (true, inner),
        None => (false, cleaned.as_str()),
    };
    let mut amount =
        Decimal::from_str(inner).map_err(|_| BankCsvError::BadAmount(s.to_string()))?;
    if neg {
        amount = -amount;
    }
    Ok(amount)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_column_with_header() {
        let csv =
            "Date,Description,Amount\n2026-01-15,Whole Foods,-84.32\n01/16/2026,Paycheck,2500.00\n";
        let txns = parse_bank_csv(csv).unwrap();
        assert_eq!(txns.len(), 2);
        assert_eq!(txns[0].description, "Whole Foods");
        assert_eq!(txns[0].amount, Decimal::new(-8432, 2));
        assert_eq!(txns[1].amount, Decimal::new(250000, 2));
    }

    #[test]
    fn four_column_expense_income() {
        let csv =
            "Date,Description,Expense,Income\n2026-01-15,Coffee,4.50,\n2026-01-16,Refund,,12.00\n";
        let txns = parse_bank_csv(csv).unwrap();
        assert_eq!(txns.len(), 2);
        assert_eq!(txns[0].amount, Decimal::new(-450, 2));
        assert_eq!(txns[1].amount, Decimal::new(1200, 2));
    }

    #[test]
    fn parenthesized_negatives_and_dollar_signs() {
        let csv = "01/15/2026,Store,\"($1,234.56)\"\n";
        let txns = parse_bank_csv(csv).unwrap();
        assert_eq!(txns[0].amount, Decimal::new(-123456, 2));
    }

    #[test]
    fn bad_column_count_rejected() {
        let csv = "a,b\n";
        assert_eq!(parse_bank_csv(csv), Err(BankCsvError::BadColumnCount(2)));
    }
}
