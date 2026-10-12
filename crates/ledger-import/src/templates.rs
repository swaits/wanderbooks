//! Starter charts of accounts by entity type.
//!
//! A new business shouldn't start from a blank chart, and the right chart
//! depends on what the business *is*: a sole proprietor has an owner's
//! capital and draws; a partnership (or multi-member LLC) has a capital and
//! distributions account per partner and pays partners guaranteed payments;
//! an S corporation pays officer wages and makes shareholder distributions;
//! a C corporation pays dividends and income tax. Each template is a
//! proposal for the agent to review with the human before importing (via
//! `books.import_chart_csv`), with tax categories already assigned, plus
//! the `wanderbooks.toml` settings that go with it (tax form, owners, and
//! where period closes go).

use crate::chart_csv::ChartRow;
use rust_decimal::Decimal;

/// The kinds of business a template exists for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntityType {
    /// Sole proprietorship or single-member LLC (Schedule C).
    SoleProprietorship,
    /// Partnership or multi-member LLC (Form 1065).
    Partnership,
    /// S corporation, including an LLC electing S status (Form 1120-S).
    SCorporation,
    /// C corporation (Form 1120).
    CCorporation,
}

impl EntityType {
    /// Parse "sole_proprietorship", "partnership", "s_corporation",
    /// "c_corporation" and common spellings ("llc" alone is ambiguous).
    pub fn parse(s: &str) -> Option<EntityType> {
        let k: String = s
            .to_lowercase()
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect();
        match k.as_str() {
            "soleproprietorship" | "soleprop" | "singlememberllc" | "smllc" => {
                Some(EntityType::SoleProprietorship)
            }
            "partnership" | "multimemberllc" | "mmllc" => Some(EntityType::Partnership),
            "scorporation" | "scorp" | "s" => Some(EntityType::SCorporation),
            "ccorporation" | "ccorp" | "c" | "corporation" => Some(EntityType::CCorporation),
            _ => None,
        }
    }
}

/// A starter chart plus the settings that go with it.
#[derive(Clone, Debug)]
pub struct ChartTemplate {
    /// Accounts, parents first.
    pub rows: Vec<ChartRow>,
    /// The `wanderbooks.toml` additions: tax form, owners, close settings.
    pub settings_toml: String,
    /// What the human should decide or know, in plain words.
    pub notes: Vec<String>,
}

impl ChartTemplate {
    /// The accounts as chart CSV with the tax_category column.
    pub fn csv(&self) -> String {
        let mut out = String::from("code,name,type,parent_code,tax_category\n");
        for r in &self.rows {
            let field = |s: &str| {
                if s.contains([',', '"']) {
                    format!("\"{}\"", s.replace('"', "\"\""))
                } else {
                    s.to_string()
                }
            };
            out.push_str(&format!(
                "{},{},{},{},{}\n",
                r.code,
                field(&r.name),
                r.account_type,
                r.parent_code.as_deref().unwrap_or(""),
                r.tax_category.as_deref().unwrap_or("")
            ));
        }
        out
    }
}

fn acct(code: &str, name: &str, t: &str, tax: Option<&str>) -> ChartRow {
    ChartRow {
        code: code.to_string(),
        name: name.to_string(),
        account_type: t.to_string(),
        parent_code: None,
        tax_category: tax.map(str::to_string),
    }
}

/// Equal shares for `n` owners as percents to two places, the last
/// absorbing the remainder so they sum to exactly 100.
fn equal_shares(n: usize) -> Vec<Decimal> {
    let each = (Decimal::from(100) / Decimal::from(n as u64)).round_dp(2);
    let mut v = vec![each; n];
    if let Some((last, rest)) = v.split_last_mut() {
        *last = Decimal::from(100) - rest.iter().sum::<Decimal>();
    }
    v
}

/// Build the template for `entity` with these owners (names; at least two
/// for a partnership, which gets a capital and distributions account each).
pub fn chart_template(entity: EntityType, owners: &[String]) -> Result<ChartTemplate, String> {
    use EntityType::*;
    if entity == Partnership && owners.len() < 2 {
        return Err("a partnership template needs at least two owners' names".to_string());
    }
    if owners.len() > 90 {
        return Err("at most 90 owners".to_string());
    }
    let owed_to = match entity {
        SoleProprietorship => "Due to Owner",
        Partnership => "Due to Members",
        SCorporation | CCorporation => "Due to Shareholders",
    };
    let mut rows = vec![
        acct("1000", "Business Checking", "asset", None),
        acct("1100", "Savings", "asset", None),
        acct("1200", "Accounts Receivable", "asset", None),
        acct("1300", "Prepaid Expenses", "asset", None),
        acct("1500", "Equipment", "asset", None),
        acct("1590", "Accumulated Depreciation", "asset", None),
        acct("2000", "Accounts Payable", "liability", None),
        acct("2100", "Credit Card", "liability", None),
        acct("2200", "Sales Tax Payable", "liability", None),
        acct("2300", "Payroll Liabilities", "liability", None),
        acct("2400", "Deferred Revenue", "liability", None),
        acct("2500", owed_to, "liability", None),
        acct("2700", "Loans Payable", "liability", None),
    ];
    let mut settings = String::new();
    let mut notes = vec![
        "Rename, add, or drop accounts before importing; codes are a convention, not a rule."
            .to_string(),
        format!(
            "{owed_to}: business expenses an owner paid personally, until the business reimburses them."
        ),
    ];
    match entity {
        SoleProprietorship => {
            rows.push(acct("3000", "Owner's Capital", "equity", None));
            rows.push(acct("3100", "Owner's Draws", "equity", None));
            settings.push_str("[company]\ntax_form = \"schedule_c\"\n\n[books]\n# Net income and draws close into the owner's capital.\nclose_into = \"3000\"\nclose_sweep = [\"3100\"]\n");
            notes
                .push("Money you take out is a draw (3100), not a wage or an expense.".to_string());
        }
        Partnership => {
            let shares = equal_shares(owners.len());
            settings.push_str("[company]\ntax_form = \"1065\"\n\n# Each partner's share drives allocations and closes. Equal shares are a\n# placeholder: use the operating agreement's numbers.\n");
            for (i, name) in owners.iter().enumerate() {
                let cap = format!("{}", 3000 + i);
                let dist = format!("{}", 3100 + i);
                rows.push(acct(&cap, &format!("Capital — {name}"), "equity", None));
                rows.push(acct(
                    &dist,
                    &format!("Distributions — {name}"),
                    "equity",
                    None,
                ));
                settings.push_str(&format!(
                    "\n[[company.members]]\nname = \"{name}\"\nshare = \"{}%\"\ncapital_account = \"{cap}\"\ndistribution_account = \"{dist}\"\n",
                    shares[i]
                ));
            }
            notes.push("A partner paid for work gets guaranteed payments (7200), not wages; profit taken out is a distribution.".to_string());
            notes.push(
                "Shares default to equal; set them from the operating agreement.".to_string(),
            );
        }
        SCorporation | CCorporation => {
            rows.push(acct("3000", "Common Stock", "equity", None));
            rows.push(acct("3100", "Additional Paid-in Capital", "equity", None));
            if entity == SCorporation {
                rows.push(acct("3200", "Shareholder Distributions", "equity", None));
                settings.push_str("[company]\ntax_form = \"1120s\"\n\n[books]\n# Net income and distributions close into Retained Earnings (3999).\nclose_sweep = [\"3200\"]\n");
                notes.push("Owner-employees must take reasonable W-2 wages (officer compensation, 7200) before distributions.".to_string());
            } else {
                rows.push(acct("3200", "Dividends Paid", "equity", None));
                settings.push_str("[company]\ntax_form = \"1120\"\n\n[books]\n# Net income and dividends close into Retained Earnings (3999).\nclose_sweep = [\"3200\"]\n");
                notes.push("The corporation pays its own income tax (7900, not deductible); dividends are not expenses.".to_string());
            }
            rows.push(acct("3999", "Retained Earnings", "equity", None));
        }
    }
    rows.extend([
        acct("4000", "Sales", "revenue", Some("gross_receipts")),
        acct(
            "4100",
            "Refunds and Allowances",
            "revenue",
            Some("returns_allowances"),
        ),
        acct(
            "4900",
            "Interest Income",
            "revenue",
            Some("interest_income"),
        ),
        acct("4950", "Other Income", "revenue", Some("other_income")),
        acct(
            "5000",
            "Cost of Goods Sold",
            "expense",
            Some("cost_of_goods_sold"),
        ),
        acct("6000", "Advertising", "expense", Some("advertising")),
        acct("6050", "Bank Fees", "expense", Some("other_deductions")),
        acct(
            "6100",
            "Platform and Payment Processing Fees",
            "expense",
            Some("commissions_fees"),
        ),
        acct("6150", "Contract Labor", "expense", Some("contract_labor")),
        acct("6200", "Depreciation", "expense", Some("depreciation")),
        acct(
            "6250",
            "Dues and Memberships",
            "expense",
            Some("other_deductions"),
        ),
        acct("6300", "Insurance", "expense", Some("insurance")),
        acct(
            "6350",
            "Interest Expense",
            "expense",
            Some("interest_expense"),
        ),
        acct(
            "6400",
            "Legal and Professional Fees",
            "expense",
            Some("legal_professional"),
        ),
        acct("6450", "Meals", "expense", Some("meals")),
        acct("6500", "Office Expense", "expense", Some("office_expense")),
        acct(
            "6550",
            "Organizational and Start-up Costs",
            "expense",
            Some("organization_startup"),
        ),
        acct("6600", "Rent", "expense", Some("rent_property")),
        acct(
            "6650",
            "Repairs and Maintenance",
            "expense",
            Some("repairs"),
        ),
        acct(
            "6700",
            "Software and Subscriptions",
            "expense",
            Some("other_deductions"),
        ),
        acct("6750", "Supplies", "expense", Some("supplies")),
        acct(
            "6800",
            "Taxes and Licenses",
            "expense",
            Some("taxes_licenses"),
        ),
        acct("6850", "Travel", "expense", Some("travel")),
        acct("6900", "Utilities", "expense", Some("utilities")),
        acct(
            "6950",
            "Penalties and Nondeductible Expenses",
            "expense",
            Some("nondeductible"),
        ),
        acct(
            "7000",
            "Salaries and Wages",
            "expense",
            Some("salaries_wages"),
        ),
        acct("7050", "Payroll Taxes", "expense", Some("taxes_licenses")),
        acct(
            "7100",
            "Employee Benefits",
            "expense",
            Some("employee_benefits"),
        ),
    ]);
    match entity {
        Partnership => rows.push(acct(
            "7200",
            "Guaranteed Payments to Partners",
            "expense",
            Some("guaranteed_payments"),
        )),
        SCorporation | CCorporation => rows.push(acct(
            "7200",
            "Officer Compensation",
            "expense",
            Some("officer_compensation"),
        )),
        SoleProprietorship => {}
    }
    if entity == CCorporation {
        rows.push(acct(
            "7900",
            "Income Tax Expense",
            "expense",
            Some("nondeductible"),
        ));
    }
    Ok(ChartTemplate {
        rows,
        settings_toml: settings,
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chart_csv::parse_chart_csv;

    #[test]
    fn templates_round_trip_through_the_chart_parser() {
        for e in [
            EntityType::SoleProprietorship,
            EntityType::Partnership,
            EntityType::SCorporation,
            EntityType::CCorporation,
        ] {
            let t = chart_template(e, &["Ann Lee".into(), "Bo, Jr.".into(), "Cy".into()]).unwrap();
            let parsed = parse_chart_csv(&t.csv()).unwrap();
            assert_eq!(parsed, t.rows, "{e:?}");
            // Codes are unique.
            let mut codes: Vec<&str> = t.rows.iter().map(|r| r.code.as_str()).collect();
            codes.sort_unstable();
            codes.dedup();
            assert_eq!(codes.len(), t.rows.len(), "{e:?}");
        }
    }

    #[test]
    fn equity_fits_the_entity() {
        let names = |t: &ChartTemplate| -> Vec<String> {
            t.rows
                .iter()
                .filter(|r| r.account_type == "equity")
                .map(|r| r.name.clone())
                .collect()
        };
        let sole = chart_template(EntityType::SoleProprietorship, &[]).unwrap();
        assert_eq!(names(&sole), ["Owner's Capital", "Owner's Draws"]);
        assert!(sole.settings_toml.contains("close_into = \"3000\""));

        let p = chart_template(
            EntityType::Partnership,
            &["A".into(), "B".into(), "C".into()],
        )
        .unwrap();
        assert_eq!(
            names(&p),
            [
                "Capital — A",
                "Distributions — A",
                "Capital — B",
                "Distributions — B",
                "Capital — C",
                "Distributions — C"
            ]
        );
        // Three equal shares that sum to exactly 100.
        assert!(p.settings_toml.contains("share = \"33.33%\""));
        assert!(p.settings_toml.contains("share = \"33.34%\""));
        assert!(
            p.rows
                .iter()
                .any(|r| r.tax_category.as_deref() == Some("guaranteed_payments"))
        );
        assert!(chart_template(EntityType::Partnership, &["solo".into()]).is_err());

        let s = chart_template(EntityType::SCorporation, &[]).unwrap();
        assert!(names(&s).contains(&"Shareholder Distributions".to_string()));
        assert!(
            s.rows
                .iter()
                .any(|r| r.tax_category.as_deref() == Some("officer_compensation"))
        );
        let c = chart_template(EntityType::CCorporation, &[]).unwrap();
        assert!(names(&c).contains(&"Dividends Paid".to_string()));
        assert!(c.rows.iter().any(|r| r.name == "Income Tax Expense"));
    }

    #[test]
    fn every_category_is_valid_for_the_entitys_return() {
        use ledger_core::{AccountType, TaxForm, tax::check_category};
        for (e, form) in [
            (EntityType::SoleProprietorship, TaxForm::ScheduleC),
            (EntityType::Partnership, TaxForm::Form1065),
            (EntityType::SCorporation, TaxForm::Form1120S),
            (EntityType::CCorporation, TaxForm::Form1120),
        ] {
            for r in chart_template(e, &["A".into(), "B".into()]).unwrap().rows {
                let Some(cat) = &r.tax_category else { continue };
                let t = if r.account_type == "revenue" {
                    AccountType::Revenue
                } else {
                    AccountType::Expense
                };
                check_category(cat, t, Some(form))
                    .unwrap_or_else(|err| panic!("{e:?} {}: {err}", r.code));
            }
        }
    }

    #[test]
    fn entity_types_parse() {
        assert_eq!(
            EntityType::parse("multi-member LLC"),
            Some(EntityType::Partnership)
        );
        assert_eq!(EntityType::parse("S corp"), Some(EntityType::SCorporation));
        assert_eq!(
            EntityType::parse("sole_proprietorship"),
            Some(EntityType::SoleProprietorship)
        );
        assert_eq!(EntityType::parse("LLC"), None, "ambiguous");
    }
}
