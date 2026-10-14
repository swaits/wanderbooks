//! Parse QuickBooks Online bank/credit-card upload CSVs.
//!
//! ```text
//! GOAL: read a bank or card export into dated, signed transactions, never
//!       guessing at what a cell means.
//! SOURCE: docs/spec/sources/intuit-bank-csv.md (Intuit, "Format CSV files
//!         in Excel to get bank transactions into QuickBooks"; "Manually
//!         upload transactions into QuickBooks Online").
//! INVARIANTS:
//!   1. Two layouts, by column count (Intuit): 3 columns Date, Description,
//!      Amount (a payment negative, a deposit positive); 4 columns Date,
//!      Description, then money out, then money in, each positive with the
//!      other cell blank. Intuit heads those two "Credit, Debit" (its example
//!      puts the payment under Credit and the deposit under Debit); bank
//!      exports often head them "Debit, Credit" in the bank's sense; both
//!      put money out third, so the columns are read by position.
//!   2. Every transaction MUST carry a nonzero amount in whole cents; a zero
//!      row is skipped (Intuit: leave zero cells blank).
//!   3. A slash date's order (month or day first) MUST come from the file or
//!      the caller, never a guess: Intuit recommends dd/mm/yyyy while US
//!      banks write mm/dd/yyyy, and a date like 3/4/2026 reads both ways.
//!      A part over 12 settles the order for the whole file; a file that
//!      settles both ways is refused; a file that settles neither needs the
//!      caller's order. ISO dates (2026-03-04) need none.
//!   4. A two-digit year follows the POSIX rule (69-99 are 1900s, 00-68
//!      2000s).
//!   5. NEVER accept an amount with a fraction of a cent, a double negative
//!      ("(-5)", "--5"), or both of a 4-column row's amounts.
//! ```
//!
//! Amounts may carry `$`, thousands commas, and parentheses for negatives
//! (common in bank exports; Intuit's pages are silent on these: CONVENTION).
//! A header row is optional: the first non-blank row is a header when its
//! first cell says "date".

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

/// Which part of a slash date (3/4/2026) is the month.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DateOrder {
    /// mm/dd/yyyy (US banks).
    MonthFirst,
    /// dd/mm/yyyy (Intuit's recommendation; most of the world).
    DayFirst,
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
    /// An amount did not parse, or isn't whole cents.
    #[error("bad amount {0:?}: expected a number of dollars and cents")]
    BadAmount(String),
    /// The file's slash dates read month-first in one place and day-first
    /// in another.
    #[error("the dates disagree: {0:?} must be day first, {1:?} month first")]
    DatesDisagree(String, String),
    /// The file's slash dates read either way, and no order was given.
    #[error(
        "the dates read either way (month or day first, e.g. {0:?}); say which order the bank uses"
    )]
    DateOrderUnknown(String),
    /// The CSV itself is malformed.
    #[error("csv error: {0}")]
    Csv(String),
}

/// Parse a QBO bank CSV, reading slash dates in the order the file itself
/// settles. Returns transactions in file order.
pub fn parse_bank_csv(text: &str) -> Result<Vec<BankTransaction>, BankCsvError> {
    parse_bank_csv_with(text, None)
}

/// [`parse_bank_csv`], with the caller's date order for a file whose dates
/// read either way. A file that settles its own order must agree with it.
pub fn parse_bank_csv_with(
    text: &str,
    order: Option<DateOrder>,
) -> Result<Vec<BankTransaction>, BankCsvError> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .trim(csv::Trim::All)
        .from_reader(text.as_bytes());
    let mut rows: Vec<(String, String, Decimal)> = Vec::new();
    let mut seen_row = false;
    for (idx, record) in reader.records().enumerate() {
        let record = record.map_err(|e| BankCsvError::Csv(e.to_string()))?;
        let fields: Vec<&str> = record.iter().collect();
        if fields.iter().all(|f| f.is_empty()) {
            continue; // skip blank lines
        }
        let first = !seen_row;
        seen_row = true;
        if first && fields[0].eq_ignore_ascii_case("date") {
            continue; // a header row
        }
        let amount = match fields.len() {
            3 => parse_amount(fields[2])?,
            4 => {
                // Money out in the third column, money in in the fourth.
                match (!fields[2].is_empty(), !fields[3].is_empty()) {
                    (true, false) => -parse_amount(fields[2])?,
                    (false, true) => parse_amount(fields[3])?,
                    (false, false) => Decimal::ZERO,
                    (true, true) => {
                        return Err(BankCsvError::Csv(format!(
                            "row {} has both money out and money in",
                            idx + 1
                        )));
                    }
                }
            }
            n => return Err(BankCsvError::BadColumnCount(n)),
        };
        if amount.is_zero() {
            continue; // zero-amount rows carry no money
        }
        rows.push((fields[0].to_string(), fields[1].to_string(), amount));
    }
    let order = date_order(rows.iter().map(|r| r.0.as_str()), order)?;
    rows.into_iter()
        .map(|(date, description, amount)| {
            Ok(BankTransaction {
                date: parse_date(&date, order)?,
                description,
                amount,
            })
        })
        .collect()
}

/// The (month-or-day, day-or-month) parts of a slash date, if it is one.
fn slash_parts(s: &str) -> Option<(u32, u32)> {
    let mut it = s.trim().split('/');
    let a = it.next()?.parse().ok()?;
    let b = it.next()?.parse().ok()?;
    it.next()?; // a year
    Some((a, b))
}

/// The order the file's slash dates settle, or the caller's (INVARIANT 3).
fn date_order<'a>(
    dates: impl Iterator<Item = &'a str>,
    given: Option<DateOrder>,
) -> Result<DateOrder, BankCsvError> {
    let (mut day_first, mut month_first, mut ambiguous) = (None, None, None);
    for d in dates {
        match slash_parts(d) {
            Some((a, _)) if a > 12 => day_first = day_first.or(Some(d.to_string())),
            Some((_, b)) if b > 12 => month_first = month_first.or(Some(d.to_string())),
            Some((a, b)) if a != b => ambiguous = ambiguous.or(Some(d.to_string())),
            _ => {}
        }
    }
    let settled = match (day_first, month_first) {
        (Some(d), Some(m)) => return Err(BankCsvError::DatesDisagree(d, m)),
        (Some(_), None) => Some(DateOrder::DayFirst),
        (None, Some(_)) => Some(DateOrder::MonthFirst),
        (None, None) => None,
    };
    match (settled, given, ambiguous) {
        (Some(s), Some(g), _) if s != g => Err(BankCsvError::DatesDisagree(
            format!("the file reads {s:?}"),
            format!("the caller said {g:?}"),
        )),
        (Some(s), _, _) => Ok(s),
        (None, Some(g), _) => Ok(g),
        // Nothing to disambiguate (ISO dates, or day equals month).
        (None, None, None) => Ok(DateOrder::MonthFirst),
        (None, None, Some(example)) => Err(BankCsvError::DateOrderUnknown(example)),
    }
}

fn parse_date(s: &str, order: DateOrder) -> Result<NaiveDate, BankCsvError> {
    let s = s.trim();
    if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Ok(d);
    }
    let four = match order {
        DateOrder::MonthFirst => "%m/%d/%Y",
        DateOrder::DayFirst => "%d/%m/%Y",
    };
    let bad = || BankCsvError::BadDate(s.to_string());
    let (rest, year) = s.rsplit_once('/').ok_or_else(bad)?;
    if year.len() != 2 {
        return NaiveDate::parse_from_str(s, four).map_err(|_| bad());
    }
    // A two-digit year follows POSIX strptime %y: 69-99 are 1900s, 00-68
    // 2000s. (chrono's %y puts the cutoff at 70; found by the spec-blind
    // suite, 2026-10-13.)
    let yy: i32 = year.parse().map_err(|_| bad())?;
    let full = if yy >= 69 { 1900 + yy } else { 2000 + yy };
    NaiveDate::parse_from_str(&format!("{rest}/{full}"), four).map_err(|_| bad())
}

fn parse_amount(s: &str) -> Result<Decimal, BankCsvError> {
    let bad = || BankCsvError::BadAmount(s.to_string());
    let cleaned: String = s.trim().replace(['$', ','], "");
    // Parentheses mean negative: (123.45). Inside them, no sign.
    let (neg, inner) = match cleaned.strip_prefix('(').and_then(|t| t.strip_suffix(')')) {
        Some(inner) if inner.starts_with(['-', '+']) => return Err(bad()),
        Some(inner) => (true, inner),
        None => (false, cleaned.as_str()),
    };
    let mut amount = Decimal::from_str(inner).map_err(|_| bad())?;
    // Whole cents only: a bank moves no fraction of one.
    if amount.normalize().scale() > 2 {
        return Err(bad());
    }
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
    fn four_column_money_out_then_in() {
        let csv =
            "Date,Description,Credit,Debit\n2026-01-15,Coffee,4.50,\n2026-01-16,Refund,,12.00\n";
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
