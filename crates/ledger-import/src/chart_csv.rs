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
}

/// Parse errors.
#[derive(Debug, Error, PartialEq)]
pub enum ChartCsvError {
    /// Wrong column count.
    #[error("expected 4 columns (code,name,type,parent_code), found {0}")]
    BadColumnCount(usize),
    /// Missing or bad header.
    #[error("bad header: expected code,name,type,parent_code")]
    BadHeader,
    /// Unknown account type.
    #[error("bad account type {0:?}: expected asset, liability, equity, revenue, or expense")]
    BadType(String),
    /// Malformed CSV.
    #[error("csv error: {0}")]
    Csv(String),
}

/// Parse a chart CSV. Returns rows in file order (parents should come first,
/// but order is not validated here).
pub fn parse_chart_csv(text: &str) -> Result<Vec<ChartRow>, ChartCsvError> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .trim(csv::Trim::All)
        .from_reader(text.as_bytes());
    let mut out = Vec::new();
    for (idx, record) in reader.records().enumerate() {
        let record = record.map_err(|e| ChartCsvError::Csv(e.to_string()))?;
        let fields: Vec<&str> = record.iter().collect();
        if fields.iter().all(|f| f.is_empty()) {
            continue;
        }
        if idx == 0 {
            let lower: Vec<String> = fields.iter().map(|f| f.to_lowercase()).collect();
            if lower != ["code", "name", "type", "parent_code"] {
                return Err(ChartCsvError::BadHeader);
            }
            continue;
        }
        if fields.len() != 4 {
            return Err(ChartCsvError::BadColumnCount(fields.len()));
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
    fn bad_type_rejected() {
        let csv = "code,name,type,parent_code\n1000,Cash,money,\n";
        assert!(matches!(
            parse_chart_csv(csv),
            Err(ChartCsvError::BadType(_))
        ));
    }
}
