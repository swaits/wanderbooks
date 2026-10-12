//! Company settings from the books dir's `wanderbooks.toml`.
//!
//! `wanderbooks init` writes that file next to `books-data/`; onboarding
//! fills in `[company]`. The server reads the company name (report headers),
//! the fiscal year start (fiscal-year-to-date reports), and
//! `[books] receipt_threshold` (the receipt policy, unless `config.json`
//! sets one). The fiscal year *end* is never stored: it is the day before
//! the next start, so the two can't disagree.

use chrono::{Datelike, NaiveDate};
use rust_decimal::Decimal;
use serde::Deserialize;
use std::path::Path;

/// The company the books belong to.
#[derive(Clone, Debug, PartialEq)]
pub struct Company {
    /// Legal name, for report headers.
    pub name: String,
    /// Month and day the fiscal year starts. Default January 1.
    pub fiscal_year_start: (u32, u32),
}

impl Default for Company {
    fn default() -> Self {
        Self {
            name: "Unnamed company".to_string(),
            fiscal_year_start: (1, 1),
        }
    }
}

impl Company {
    /// The fiscal year containing `d`: `(first day, last day)`, inclusive.
    pub fn fiscal_year_containing(&self, d: NaiveDate) -> (NaiveDate, NaiveDate) {
        let (m, day) = self.fiscal_year_start;
        let on = |year| NaiveDate::from_ymd_opt(year, m, day).expect("validated month-day");
        let start = if on(d.year()) <= d {
            on(d.year())
        } else {
            on(d.year() - 1)
        };
        let end = on(start.year() + 1)
            .pred_opt()
            .expect("not the minimum date");
        (start, end)
    }
}

/// What the server takes from `wanderbooks.toml`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BooksSettings {
    /// `[company]`.
    pub company: Company,
    /// `[books] receipt_threshold`, validated as a decimal string.
    pub receipt_threshold: Option<String>,
}

#[derive(Deserialize)]
struct BooksToml {
    #[serde(default)]
    company: CompanyToml,
    #[serde(default)]
    books: BooksSection,
}

#[derive(Default, Deserialize)]
struct CompanyToml {
    name: Option<String>,
    fiscal_year_start: Option<String>,
}

#[derive(Default, Deserialize)]
struct BooksSection {
    receipt_threshold: Option<String>,
}

/// Parse `"MM-DD"`. February 29 is rejected: a fiscal year must start on a
/// day every year has.
fn parse_month_day(s: &str) -> Result<(u32, u32), String> {
    let bad = || format!("fiscal_year_start {s:?}: expected \"MM-DD\", e.g. \"01-01\"");
    let (m, d) = s.split_once('-').ok_or_else(bad)?;
    let (m, d): (u32, u32) = (m.parse().map_err(|_| bad())?, d.parse().map_err(|_| bad())?);
    // 2001 is not a leap year: a valid date there exists every year.
    NaiveDate::from_ymd_opt(2001, m, d)
        .ok_or_else(|| format!("fiscal_year_start {s:?} is not a day every year has"))?;
    Ok((m, d))
}

/// Parse `wanderbooks.toml` text.
pub fn parse(text: &str) -> Result<BooksSettings, String> {
    let raw: BooksToml = toml::from_str(text).map_err(|e| e.to_string())?;
    let mut company = Company::default();
    if let Some(name) = raw.company.name.filter(|n| !n.trim().is_empty()) {
        company.name = name.trim().to_string();
    }
    if let Some(fy) = raw.company.fiscal_year_start {
        company.fiscal_year_start = parse_month_day(fy.trim())?;
    }
    let receipt_threshold = match raw.books.receipt_threshold {
        Some(t) => {
            t.trim()
                .parse::<Decimal>()
                .map_err(|_| format!("receipt_threshold {t:?} is not a decimal"))?;
            Some(t.trim().to_string())
        }
        None => None,
    };
    Ok(BooksSettings {
        company,
        receipt_threshold,
    })
}

/// Load `wanderbooks.toml` from a path.
pub fn load(path: &Path) -> Result<BooksSettings, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    parse(&text).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn calendar_year_by_default() {
        let c = parse("").unwrap().company;
        assert_eq!(c.fiscal_year_start, (1, 1));
        assert_eq!(
            c.fiscal_year_containing(d(2026, 10, 11)),
            (d(2026, 1, 1), d(2026, 12, 31))
        );
        assert_eq!(
            c.fiscal_year_containing(d(2026, 1, 1)),
            (d(2026, 1, 1), d(2026, 12, 31))
        );
    }

    #[test]
    fn a_july_fiscal_year_spans_two_calendar_years() {
        let c = parse("[company]\nfiscal_year_start = \"07-01\"\n")
            .unwrap()
            .company;
        assert_eq!(
            c.fiscal_year_containing(d(2026, 3, 15)),
            (d(2025, 7, 1), d(2026, 6, 30))
        );
        assert_eq!(
            c.fiscal_year_containing(d(2026, 7, 1)),
            (d(2026, 7, 1), d(2027, 6, 30))
        );
        assert_eq!(
            c.fiscal_year_containing(d(2026, 6, 30)),
            (d(2025, 7, 1), d(2026, 6, 30))
        );
    }

    #[test]
    fn reads_the_scaffolded_file() {
        let s = parse(
            "[company]\nname = \"Fernhill Labs LLC\"\nentity_type = \"x\"\nfiscal_year_start = \"01-01\"\n\n[books]\ndata_dir = \"books-data\"\nreceipt_threshold = \"25.00\"\n",
        )
        .unwrap();
        assert_eq!(s.company.name, "Fernhill Labs LLC");
        assert_eq!(s.receipt_threshold.as_deref(), Some("25.00"));
        // An empty name (fresh scaffold) keeps the placeholder.
        assert_eq!(
            parse("[company]\nname = \"\"\n").unwrap().company.name,
            "Unnamed company"
        );
    }

    #[test]
    fn rejects_bad_settings() {
        assert!(parse("[company]\nfiscal_year_start = \"02-29\"\n").is_err());
        assert!(parse("[company]\nfiscal_year_start = \"13-01\"\n").is_err());
        assert!(parse("[company]\nfiscal_year_start = \"Jan 1\"\n").is_err());
        assert!(parse("[books]\nreceipt_threshold = \"25 dollars\"\n").is_err());
    }
}
