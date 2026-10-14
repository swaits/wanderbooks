//! Parse chart-of-accounts CSVs.
//!
//! Expected layout (header row required):
//!
//! ```csv
//! code,name,type,parent_code
//! 1000,Cash,asset,
//! 1010,Checking,asset,1000
//! ```
//!
//! `type` is one of `asset`, `liability`, `equity`, `revenue`, `expense`
//! (case-insensitive). `parent_code` is optional; empty means top-level.

use thiserror::Error;

/// One chart row.
#[derive(Clone, Debug, PartialEq)]
pub struct ChartRow {
    /// Account code.
    pub code: String,
    /// Account name.
    pub name: String,
    /// Account type, lowercase: asset, liability, equity, revenue, expense.
    pub account_type: String,
    /// Parent account code, if any.
    pub parent_code: Option<String>,
    /// Optional fifth column: the tax category key (see
    /// `ledger_core::tax`).
    pub tax_category: Option<String>,
}

/// Parse errors.
#[derive(Debug, Error, PartialEq)]
pub enum ChartCsvError {
    /// Wrong column count: (found, expected by the header).
    #[error("expected {1} columns to match the header, found {0}")]
    BadColumnCount(usize, usize),
    /// Missing or bad header.
    #[error("bad header: expected code,name,type,parent_code[,tax_category]")]
    BadHeader,
    /// Unknown account type.
    #[error("bad account type {0:?}: expected asset, liability, equity, revenue, or expense")]
    BadType(String),
    /// A row's code or name is blank (row numbers count the header as 1).
    #[error("row {row}: the {field} is blank")]
    Blank {
        /// The row, counting the header as row 1.
        row: usize,
        /// "code" or "name".
        field: &'static str,
    },
    /// Malformed CSV.
    #[error("csv error: {0}")]
    Csv(String),
}

/// Parse a chart CSV. Returns rows in file order (parents should come first,
/// but order is not validated here).
///
/// ```text
/// GOAL: read a chart of accounts a person hands the agent.
/// SOURCE: docs/mcp-interface.md (books.import_chart_csv); CONVENTION for
///         the layout.
/// INVARIANTS:
///   1. The first non-blank row is the header, exactly code,name,type,
///      parent_code with an optional fifth tax_category (any case);
///      anything else is BadHeader.
///   2. Every later row has the header's width (BadColumnCount), a type
///      among asset, liability, equity, revenue, expense, any case
///      (BadType), and a non-blank code and name (Blank, with its row).
///   3. A blank parent or tax category is None; blank lines are skipped;
///      an empty file is no rows.
/// ```
pub fn parse_chart_csv(text: &str) -> Result<Vec<ChartRow>, ChartCsvError> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .trim(csv::Trim::All)
        .from_reader(text.as_bytes());
    let mut out = Vec::new();
    let mut width = 4;
    let mut header_seen = false;
    for (idx, record) in reader.records().enumerate() {
        let record = record.map_err(|e| ChartCsvError::Csv(e.to_string()))?;
        let fields: Vec<&str> = record.iter().collect();
        if fields.iter().all(|f| f.is_empty()) {
            continue;
        }
        if !header_seen {
            header_seen = true;
            let lower: Vec<String> = fields.iter().map(|f| f.to_lowercase()).collect();
            width = if lower == ["code", "name", "type", "parent_code"] {
                4
            } else if lower == ["code", "name", "type", "parent_code", "tax_category"] {
                5
            } else {
                return Err(ChartCsvError::BadHeader);
            };
            continue;
        }
        if fields.len() != width {
            return Err(ChartCsvError::BadColumnCount(fields.len(), width));
        }
        for (field, value) in [("code", fields[0]), ("name", fields[1])] {
            if value.trim().is_empty() {
                return Err(ChartCsvError::Blank {
                    row: idx + 1,
                    field,
                });
            }
        }
        let account_type = fields[2].to_lowercase();
        match account_type.as_str() {
            "asset" | "liability" | "equity" | "revenue" | "expense" => {}
            _ => return Err(ChartCsvError::BadType(fields[2].to_string())),
        }
        let parent_code = fields[3].trim();
        out.push(ChartRow {
            code: fields[0].to_string(),
            name: fields[1].to_string(),
            account_type,
            parent_code: if parent_code.is_empty() {
                None
            } else {
                Some(parent_code.to_string())
            },
            tax_category: fields
                .get(4)
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(str::to_string),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_chart() {
        let csv = "code,name,type,parent_code\n1000,Cash,asset,\n1010,Checking,asset,1000\n4000,Revenue,revenue,\n";
        let rows = parse_chart_csv(csv).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].parent_code, None);
        assert_eq!(rows[1].parent_code, Some("1000".to_string()));
        assert_eq!(rows[2].account_type, "revenue");
    }

    #[test]
    fn parses_optional_tax_category_column() {
        let csv = "code,name,type,parent_code,tax_category\n4000,Sales,revenue,,gross_receipts\n1000,Cash,asset,,\n";
        let rows = parse_chart_csv(csv).unwrap();
        assert_eq!(rows[0].tax_category.as_deref(), Some("gross_receipts"));
        assert_eq!(rows[1].tax_category, None);
        // Rows must match the header's width.
        let short = "code,name,type,parent_code,tax_category\n1000,Cash,asset,\n";
        assert_eq!(
            parse_chart_csv(short),
            Err(ChartCsvError::BadColumnCount(4, 5))
        );
    }

    #[test]
    fn bad_type_rejected() {
        let csv = "code,name,type,parent_code\n1000,Cash,money,\n";
        assert!(matches!(
            parse_chart_csv(csv),
            Err(ChartCsvError::BadType(_))
        ));
    }

    /// Found by fuzz_import_csv (2026-10-13): a row with a blank code or
    /// name was accepted.
    #[test]
    fn a_blank_code_or_name_is_refused_with_its_row() {
        let header = "code,name,type,parent_code\n";
        assert_eq!(
            parse_chart_csv(&format!("{header}1000,Cash,asset,\n,Rent,expense,\n")),
            Err(ChartCsvError::Blank {
                row: 3,
                field: "code"
            })
        );
        assert_eq!(
            parse_chart_csv(&format!("{header}1000,  ,asset,\n")),
            Err(ChartCsvError::Blank {
                row: 2,
                field: "name"
            })
        );
    }
}
