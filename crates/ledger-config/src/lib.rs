//! Company settings from the books dir's `wanderbooks.toml`.
//!
//! `wanderbooks init` writes that file next to `books-data/`; onboarding
//! fills in `[company]`. The server reads the company name (report headers),
//! the fiscal year start (fiscal-year-to-date reports), the members of a
//! multi-member LLC (`[[company.members]]`: name, share, capital account —
//! for the statement of members' equity), `[books] accounting_basis`
//! (report footers), and `[books] receipt_threshold` (the receipt policy,
//! unless `config.json` sets one). The fiscal year *end* is never stored: it is the day before
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
    /// Members of a multi-member LLC; empty for other entities.
    pub members: Vec<Member>,
    /// "Accrual" or "Cash", for report footers. Default "Accrual".
    pub basis: String,
    /// The income tax return the business files, if known: from
    /// `tax_form`, else from `tax_classification`.
    pub tax_form: Option<ledger_core::TaxForm>,
    /// `[books] close_into`: the equity account a period close puts net
    /// income in (a sole proprietor's capital, say). Unset: Retained
    /// Earnings, or members' capital accounts when there are members.
    pub close_into: Option<String>,
    /// `[books] close_sweep`: equity accounts (draws, dividends,
    /// distributions) a close zeroes into `close_into`.
    pub close_sweep: Vec<String>,
    /// Every simple `[company]` setting (name, EIN, formation date, ...),
    /// in file order, as written: the full profile for display.
    pub profile: Vec<(String, String)>,
}

/// One member of a multi-member LLC.
#[derive(Clone, Debug, PartialEq)]
pub struct Member {
    /// Name, as it appears on reports.
    pub name: String,
    /// Share of income, e.g. 0.50.
    pub share: Decimal,
    /// Code of their capital account in the chart.
    pub capital_account: String,
    /// Code of their distributions account, if distributions are recorded
    /// separately from capital (zeroed into capital at close).
    pub distribution_account: Option<String>,
}

impl Default for Company {
    fn default() -> Self {
        Self {
            name: "Unnamed company".to_string(),
            fiscal_year_start: (1, 1),
            members: Vec::new(),
            basis: "Accrual".to_string(),
            tax_form: None,
            close_into: None,
            close_sweep: Vec::new(),
            profile: Vec::new(),
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
    /// `[books] cash_accounts`: account codes that hold cash (bank,
    /// savings, cash on hand), for cash reports and dashboards. Empty:
    /// callers decide (e.g. by account name).
    pub cash_accounts: Vec<String>,
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
    tax_form: Option<String>,
    tax_classification: Option<String>,
    #[serde(default)]
    members: Vec<MemberToml>,
}

#[derive(Deserialize)]
struct MemberToml {
    name: String,
    share: String,
    capital_account: Option<String>,
    distribution_account: Option<String>,
}

#[derive(Default, Deserialize)]
struct BooksSection {
    receipt_threshold: Option<String>,
    accounting_basis: Option<String>,
    close_into: Option<String>,
    #[serde(default)]
    close_sweep: Vec<String>,
    #[serde(default)]
    cash_accounts: Vec<String>,
}

/// The return a federal tax classification files: "partnership" → 1065,
/// "s corporation" → 1120-S, "c corporation" → 1120, "sole proprietorship"
/// or "disregarded entity" → Schedule C. Unrecognized: unknown.
fn form_for_classification(c: &str) -> Option<ledger_core::TaxForm> {
    let c = c.to_lowercase();
    if c.contains("partnership") {
        Some(ledger_core::TaxForm::Form1065)
    } else if c.contains("s corp") || c.contains("s-corp") || c.contains("s_corp") {
        Some(ledger_core::TaxForm::Form1120S)
    } else if c.contains("c corp") || c.contains("c-corp") || c.contains("c_corp") {
        Some(ledger_core::TaxForm::Form1120)
    } else if c.contains("sole") || c.contains("disregarded") {
        Some(ledger_core::TaxForm::ScheduleC)
    } else {
        None
    }
}

/// Parse a share: "50%" or "0.5".
fn parse_share(name: &str, s: &str) -> Result<Decimal, String> {
    let s = s.trim();
    let bad = || format!("member {name}: share {s:?} must be like \"50%\" or \"0.5\"");
    let share = match s.strip_suffix('%') {
        Some(pct) => pct.trim().parse::<Decimal>().map_err(|_| bad())? / Decimal::from(100),
        None => s.parse::<Decimal>().map_err(|_| bad())?,
    };
    if share <= Decimal::ZERO || share > Decimal::ONE {
        return Err(bad());
    }
    Ok(share)
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
    for m in raw.company.members {
        let share = parse_share(&m.name, &m.share)?;
        let capital_account = m
            .capital_account
            .filter(|c| !c.trim().is_empty())
            .ok_or_else(|| format!("member {}: capital_account is required", m.name))?;
        company.members.push(Member {
            name: m.name.trim().to_string(),
            share,
            capital_account: capital_account.trim().to_string(),
            distribution_account: m
                .distribution_account
                .map(|d| d.trim().to_string())
                .filter(|d| !d.is_empty()),
        });
    }
    if !company.members.is_empty() {
        let total: Decimal = company.members.iter().map(|m| m.share).sum();
        if total != Decimal::ONE {
            return Err(format!("member shares sum to {total}, not 100%"));
        }
    }
    company.tax_form =
        match (&raw.company.tax_form, &raw.company.tax_classification) {
            (Some(f), _) => Some(ledger_core::TaxForm::parse(f).ok_or_else(|| {
                format!("tax_form {f:?}: expected schedule_c, 1065, 1120s, or 1120")
            })?),
            (None, Some(c)) => form_for_classification(c),
            (None, None) => None,
        };
    if !company.members.is_empty() && raw.books.close_into.is_some() {
        return Err(
            "close_into is for single-owner businesses; with [[company.members]] a close goes to each member's capital account"
                .to_string(),
        );
    }
    company.close_into = raw.books.close_into.map(|c| c.trim().to_string());
    company.close_sweep = raw.books.close_sweep;
    if let Some(basis) = raw.books.accounting_basis {
        company.basis = match basis.trim().to_lowercase().as_str() {
            "accrual" => "Accrual".to_string(),
            "cash" => "Cash".to_string(),
            other => {
                return Err(format!(
                    "accounting_basis {other:?}: expected accrual or cash"
                ));
            }
        };
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
    let table: toml::Table = toml::from_str(text).map_err(|e| e.to_string())?;
    if let Some(toml::Value::Table(c)) = table.get("company") {
        company.profile = c
            .iter()
            .filter_map(|(k, v)| {
                let s = match v {
                    toml::Value::String(s) => s.clone(),
                    toml::Value::Integer(i) => i.to_string(),
                    toml::Value::Float(f) => f.to_string(),
                    toml::Value::Boolean(b) => b.to_string(),
                    toml::Value::Datetime(d) => d.to_string(),
                    _ => return None,
                };
                Some((k.clone(), s))
            })
            .collect();
    }
    Ok(BooksSettings {
        company,
        receipt_threshold,
        cash_accounts: raw.books.cash_accounts,
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
            "[company]\nname = \"Wanderland Labs LLC\"\nentity_type = \"x\"\nfiscal_year_start = \"01-01\"\n\n[books]\ndata_dir = \"books-data\"\nreceipt_threshold = \"25.00\"\n",
        )
        .unwrap();
        assert_eq!(s.company.name, "Wanderland Labs LLC");
        assert_eq!(s.receipt_threshold.as_deref(), Some("25.00"));
        // An empty name (fresh scaffold) keeps the placeholder.
        assert_eq!(
            parse("[company]\nname = \"\"\n").unwrap().company.name,
            "Unnamed company"
        );
    }

    #[test]
    fn keeps_the_whole_profile_in_order() {
        let s = parse(
            "[company]\nname = \"Acme LLC\"\nein = \"12-3456789\"\nformed = \"2026-10-04\"\n[books]\ncash_accounts = [\"1000\", \"1010\"]\n",
        )
        .unwrap();
        assert_eq!(
            s.company.profile,
            [
                ("name".to_string(), "Acme LLC".to_string()),
                ("ein".to_string(), "12-3456789".to_string()),
                ("formed".to_string(), "2026-10-04".to_string()),
            ]
        );
        assert_eq!(s.cash_accounts, ["1000", "1010"]);
    }

    #[test]
    fn reads_members_and_basis() {
        let s = parse(
            "[company]\n[[company.members]]\nname = \"Stephen Waits\"\nshare = \"50%\"\ncapital_account = \"3000\"\n[[company.members]]\nname = \"Katherine Pendergast\"\nshare = \"0.5\"\ncapital_account = \"3001\"\n[books]\naccounting_basis = \"cash\"\n",
        )
        .unwrap();
        let half = Decimal::new(5, 1);
        assert_eq!(
            s.company.members,
            vec![
                Member {
                    name: "Stephen Waits".into(),
                    share: half,
                    capital_account: "3000".into(),
                    distribution_account: None,
                },
                Member {
                    name: "Katherine Pendergast".into(),
                    share: half,
                    capital_account: "3001".into(),
                    distribution_account: None,
                },
            ]
        );
        assert_eq!(s.company.basis, "Cash");
        assert_eq!(parse("").unwrap().company.basis, "Accrual");
        // Shares must add up; every member needs a capital account.
        let two = |a: &str, b: &str| {
            format!(
                "[company]\n[[company.members]]\nname = \"A\"\nshare = \"{a}\"\ncapital_account = \"3000\"\n[[company.members]]\nname = \"B\"\nshare = \"{b}\"\ncapital_account = \"3001\"\n"
            )
        };
        assert!(parse(&two("50%", "40%")).is_err());
        assert!(parse(&two("60%", "40%")).is_ok());
        assert!(parse("[company]\n[[company.members]]\nname = \"A\"\nshare = \"100%\"\n").is_err());
        assert!(parse("[books]\naccounting_basis = \"modified\"\n").is_err());
    }

    #[test]
    fn tax_form_from_setting_or_classification() {
        use ledger_core::TaxForm;
        let form = |toml: &str| parse(toml).unwrap().company.tax_form;
        assert_eq!(
            form("[company]\ntax_form = \"1120s\"\n"),
            Some(TaxForm::Form1120S)
        );
        assert_eq!(
            form("[company]\ntax_classification = \"partnership\"\n"),
            Some(TaxForm::Form1065)
        );
        assert_eq!(
            form("[company]\ntax_classification = \"S corporation\"\n"),
            Some(TaxForm::Form1120S)
        );
        assert_eq!(
            form("[company]\ntax_classification = \"disregarded entity\"\n"),
            Some(TaxForm::ScheduleC)
        );
        assert_eq!(
            form("[company]\ntax_classification = \"partnership\"\ntax_form = \"1120\"\n"),
            Some(TaxForm::Form1120)
        );
        assert_eq!(form(""), None);
        assert!(parse("[company]\ntax_form = \"990\"\n").is_err());
    }

    #[test]
    fn rejects_bad_settings() {
        assert!(parse("[company]\nfiscal_year_start = \"02-29\"\n").is_err());
        assert!(parse("[company]\nfiscal_year_start = \"13-01\"\n").is_err());
        assert!(parse("[company]\nfiscal_year_start = \"Jan 1\"\n").is_err());
        assert!(parse("[books]\nreceipt_threshold = \"25 dollars\"\n").is_err());
    }
}
