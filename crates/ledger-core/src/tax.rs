//! Tax categories: where each revenue and expense account lands on the
//! business's income tax return, and how much of it is deductible.
//!
//! The books stay book-basis; this is the bridge a CPA (or tax software)
//! walks across. Categories are *stable semantic keys* ("meals",
//! "guaranteed_payments") rather than form line numbers, which move from
//! year to year; each category says which returns it applies to. A
//! deductible percentage (meals 50%, penalties 0%) turns book amounts into
//! deductible amounts and surfaces the book-to-tax differences (Schedule
//! M-1 items) instead of hiding them.
//!
//! Scope is deliberate, like fixed assets: no tax depreciation (MACRS,
//! Section 179, bonus), no basis computations, no filing. Not tax advice.

use crate::{AccountType, Ledger, LedgerError};
use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The federal income tax return a business files.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum TaxForm {
    /// Sole proprietorship or single-member LLC: Schedule C (Form 1040).
    ScheduleC,
    /// Partnership or multi-member LLC: Form 1065.
    Form1065,
    /// S corporation: Form 1120-S.
    Form1120S,
    /// C corporation: Form 1120.
    Form1120,
}

impl TaxForm {
    /// Every form.
    pub const ALL: [TaxForm; 4] = [
        TaxForm::ScheduleC,
        TaxForm::Form1065,
        TaxForm::Form1120S,
        TaxForm::Form1120,
    ];

    /// "Form 1065", "Schedule C", ...
    pub fn label(&self) -> &'static str {
        match self {
            TaxForm::ScheduleC => "Schedule C",
            TaxForm::Form1065 => "Form 1065",
            TaxForm::Form1120S => "Form 1120-S",
            TaxForm::Form1120 => "Form 1120",
        }
    }

    /// Parse "schedule_c", "1065", "1120s", "1120" (and their labels).
    pub fn parse(s: &str) -> Option<TaxForm> {
        let k: String = s
            .to_lowercase()
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect();
        match k.as_str() {
            "schedulec" | "c" => Some(TaxForm::ScheduleC),
            "1065" | "form1065" => Some(TaxForm::Form1065),
            "1120s" | "form1120s" => Some(TaxForm::Form1120S),
            "1120" | "form1120" => Some(TaxForm::Form1120),
            _ => None,
        }
    }
}

/// Where a category sits on the return.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaxKind {
    /// Income (revenue accounts).
    Income,
    /// Cost of goods sold (expense accounts).
    CostOfGoods,
    /// A deduction (expense accounts).
    Deduction,
}

/// One tax category.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct TaxCategory {
    /// Stable key, e.g. `"meals"`.
    pub key: &'static str,
    /// Human label, e.g. `"Meals"`.
    pub label: &'static str,
    /// Income, COGS, or deduction.
    pub kind: TaxKind,
    /// The returns it appears on.
    pub forms: &'static [TaxForm],
    /// Percent deductible by default (100 for most; meals 50).
    pub default_deductible_percent: u32,
    /// What to know about it, for the agent and the CPA.
    pub note: &'static str,
}

const ALL: &[TaxForm] = &TaxForm::ALL;
const CORPS: &[TaxForm] = &[TaxForm::Form1120S, TaxForm::Form1120];

const fn cat(
    key: &'static str,
    label: &'static str,
    kind: TaxKind,
    forms: &'static [TaxForm],
    pct: u32,
    note: &'static str,
) -> TaxCategory {
    TaxCategory {
        key,
        label,
        kind,
        forms,
        default_deductible_percent: pct,
        note,
    }
}

use TaxKind::{CostOfGoods, Deduction, Income};

/// The catalog.
pub const CATEGORIES: &[TaxCategory] = &[
    cat(
        "gross_receipts",
        "Gross receipts or sales",
        Income,
        ALL,
        100,
        "Sales recorded gross (platform and processor fees are a separate deduction).",
    ),
    cat(
        "returns_allowances",
        "Returns and allowances",
        Income,
        ALL,
        100,
        "Refunds and allowances; reduces gross receipts.",
    ),
    cat(
        "interest_income",
        "Interest income",
        Income,
        ALL,
        100,
        "Pass-throughs report it separately to owners (portfolio income).",
    ),
    cat("other_income", "Other income", Income, ALL, 100, ""),
    cat(
        "cost_of_goods_sold",
        "Cost of goods sold",
        CostOfGoods,
        ALL,
        100,
        "Inventory and direct costs of what was sold.",
    ),
    cat("advertising", "Advertising", Deduction, ALL, 100, ""),
    cat(
        "bad_debts",
        "Bad debts",
        Deduction,
        ALL,
        100,
        "Only debts previously included in income (accrual basis).",
    ),
    cat(
        "car_truck",
        "Car and truck expenses",
        Deduction,
        ALL,
        100,
        "Actual costs or the standard mileage rate, not both.",
    ),
    cat(
        "charitable_contributions",
        "Charitable contributions",
        Deduction,
        ALL,
        100,
        "Separately stated to owners for partnerships and S corps; limited for C corps; not a Schedule C deduction.",
    ),
    cat(
        "commissions_fees",
        "Commissions and fees",
        Deduction,
        ALL,
        100,
        "Includes app store, marketplace, and payment processor fees.",
    ),
    cat(
        "contract_labor",
        "Contract labor",
        Deduction,
        ALL,
        100,
        "Payments to non-employees; see 1099 reporting.",
    ),
    cat(
        "depreciation",
        "Depreciation (book)",
        Deduction,
        ALL,
        100,
        "Book depreciation; tax depreciation (MACRS, Section 179, bonus) is computed separately by the CPA.",
    ),
    cat(
        "employee_benefits",
        "Employee benefit programs",
        Deduction,
        ALL,
        100,
        "",
    ),
    cat(
        "guaranteed_payments",
        "Guaranteed payments to partners",
        Deduction,
        &[TaxForm::Form1065],
        100,
        "Payments to partners for services or capital, regardless of profit; reported on each partner's K-1.",
    ),
    cat(
        "insurance",
        "Insurance (other than health)",
        Deduction,
        ALL,
        100,
        "",
    ),
    cat("interest_expense", "Interest", Deduction, ALL, 100, ""),
    cat(
        "legal_professional",
        "Legal and professional services",
        Deduction,
        ALL,
        100,
        "Attorneys, accountants, consultants.",
    ),
    cat(
        "meals",
        "Meals",
        Deduction,
        ALL,
        50,
        "Generally 50% deductible; the other half is a book-to-tax difference.",
    ),
    cat("office_expense", "Office expense", Deduction, ALL, 100, ""),
    cat(
        "officer_compensation",
        "Compensation of officers",
        Deduction,
        CORPS,
        100,
        "Wages paid to officer-shareholders through payroll.",
    ),
    cat(
        "organization_startup",
        "Organizational and start-up costs",
        Deduction,
        ALL,
        100,
        "Tax: up to $5,000 each deductible in the first year, the rest amortized over 180 months; the CPA computes it.",
    ),
    cat(
        "pension_profit_sharing",
        "Pension and profit-sharing plans",
        Deduction,
        ALL,
        100,
        "",
    ),
    cat(
        "rent_equipment",
        "Rent or lease: vehicles, machinery, equipment",
        Deduction,
        ALL,
        100,
        "",
    ),
    cat(
        "rent_property",
        "Rent or lease: other business property",
        Deduction,
        ALL,
        100,
        "",
    ),
    cat(
        "repairs",
        "Repairs and maintenance",
        Deduction,
        ALL,
        100,
        "",
    ),
    cat(
        "salaries_wages",
        "Salaries and wages",
        Deduction,
        ALL,
        100,
        "Employees (not owners of a partnership, whose pay is guaranteed payments).",
    ),
    cat("supplies", "Supplies", Deduction, ALL, 100, ""),
    cat(
        "taxes_licenses",
        "Taxes and licenses",
        Deduction,
        ALL,
        100,
        "Payroll taxes, business licenses, state fees; not federal income tax.",
    ),
    cat(
        "travel",
        "Travel",
        Deduction,
        ALL,
        100,
        "Lodging and transportation away from home; meals go under Meals.",
    ),
    cat("utilities", "Utilities", Deduction, ALL, 100, ""),
    cat(
        "other_deductions",
        "Other deductions",
        Deduction,
        ALL,
        100,
        "Software, subscriptions, bank fees, dues, and the rest; the CPA itemizes.",
    ),
    cat(
        "nondeductible",
        "Nondeductible expenses",
        Deduction,
        ALL,
        0,
        "Fines, penalties, political contributions, federal income tax: a book expense with no deduction.",
    ),
];

/// Look up a category by key.
pub fn category(key: &str) -> Option<&'static TaxCategory> {
    CATEGORIES.iter().find(|c| c.key == key)
}

/// Check that `key` can tag an account of `account_type` and, when a form
/// is known, that it appears on that form.
pub fn check_category(
    key: &str,
    account_type: AccountType,
    form: Option<TaxForm>,
) -> Result<&'static TaxCategory, LedgerError> {
    let c = category(key)
        .ok_or_else(|| LedgerError::InvalidTaxCategory(format!("unknown tax category {key:?}")))?;
    let fits = match c.kind {
        TaxKind::Income => matches!(account_type, AccountType::Revenue | AccountType::Expense),
        TaxKind::CostOfGoods | TaxKind::Deduction => account_type == AccountType::Expense,
    };
    if !fits {
        return Err(LedgerError::InvalidTaxCategory(format!(
            "{key} is for {} accounts, not {account_type:?}",
            if c.kind == TaxKind::Income {
                "revenue"
            } else {
                "expense"
            }
        )));
    }
    if let Some(f) = form
        && !c.forms.contains(&f)
    {
        return Err(LedgerError::InvalidTaxCategory(format!(
            "{key} does not appear on {}",
            f.label()
        )));
    }
    Ok(c)
}

/// One category's line on the tax summary.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TaxSummaryLine {
    /// Category key.
    pub category: String,
    /// Category label.
    pub label: String,
    /// The accounts that roll into it, `"CODE Name"`.
    pub accounts: Vec<String>,
    /// Book amount: revenue for income, expense for COGS and deductions.
    pub book: Decimal,
    /// Deductible (or taxable) amount after the deductible percentage.
    pub tax: Decimal,
}

/// An account with P&L activity and no tax category.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Uncategorized {
    /// `"CODE Name"`.
    pub account: String,
    /// Revenue or Expense.
    pub account_type: AccountType,
    /// Book amount (normal side).
    pub book: Decimal,
}

/// The tax summary for a period: book amounts by tax category, deductible
/// amounts, and every account still needing a category.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaxSummary {
    /// First date, inclusive.
    pub start: NaiveDate,
    /// Last date, inclusive.
    pub end: NaiveDate,
    /// The return, when known.
    pub form: Option<TaxForm>,
    /// Income categories.
    pub income: Vec<TaxSummaryLine>,
    /// Cost of goods sold.
    pub cost_of_goods: Vec<TaxSummaryLine>,
    /// Deductions.
    pub deductions: Vec<TaxSummaryLine>,
    /// P&L accounts with activity and no category.
    pub uncategorized: Vec<Uncategorized>,
}

impl TaxSummary {
    /// Book net income of the categorized accounts.
    pub fn book_net(&self) -> Decimal {
        sum(&self.income, |l| l.book)
            - sum(&self.cost_of_goods, |l| l.book)
            - sum(&self.deductions, |l| l.book)
    }

    /// Income less deductible amounts, before the CPA's other adjustments.
    pub fn tax_net(&self) -> Decimal {
        sum(&self.income, |l| l.tax)
            - sum(&self.cost_of_goods, |l| l.tax)
            - sum(&self.deductions, |l| l.tax)
    }
}

fn sum(lines: &[TaxSummaryLine], f: fn(&TaxSummaryLine) -> Decimal) -> Decimal {
    lines.iter().map(f).sum()
}

/// Build the tax summary for `[start, end]`, closing entries excluded (like
/// the P&L). Deductible amounts round to the cent, half away from zero.
pub fn tax_summary(
    ledger: &Ledger,
    start: NaiveDate,
    end: NaiveDate,
    form: Option<TaxForm>,
) -> Result<TaxSummary, LedgerError> {
    let pl = crate::reports::profit_and_loss_for_period(ledger, start, end, None)?;
    let mut by_cat: BTreeMap<&'static str, TaxSummaryLine> = BTreeMap::new();
    let mut uncategorized = Vec::new();
    for line in pl.revenue.iter().chain(pl.expenses.iter()) {
        let account = ledger.chart().find_by_code(&line.code).ok_or_else(|| {
            LedgerError::InvalidTaxCategory(format!("unknown account {}", line.code))
        })?;
        let name = format!("{} {}", account.code, account.name);
        let Some(c) = account.tax_category.as_deref().and_then(category) else {
            uncategorized.push(Uncategorized {
                account: name,
                account_type: account.account_type,
                book: line.amount,
            });
            continue;
        };
        // Income from an expense-type account (e.g. a contra) counts with
        // its sign flipped, so returns reduce receipts.
        let book = if c.kind == TaxKind::Income && account.account_type == AccountType::Expense {
            -line.amount
        } else {
            line.amount
        };
        let pct = account
            .deductible_percent
            .unwrap_or_else(|| Decimal::from(c.default_deductible_percent));
        let tax = (book * pct / Decimal::from(100))
            .round_dp_with_strategy(2, rust_decimal::RoundingStrategy::MidpointAwayFromZero);
        let entry = by_cat.entry(c.key).or_insert_with(|| TaxSummaryLine {
            category: c.key.to_string(),
            label: c.label.to_string(),
            accounts: vec![],
            book: Decimal::ZERO,
            tax: Decimal::ZERO,
        });
        entry.accounts.push(name);
        entry.book += book;
        entry.tax += tax;
    }
    let mut out = TaxSummary {
        start,
        end,
        form,
        income: vec![],
        cost_of_goods: vec![],
        deductions: vec![],
        uncategorized,
    };
    // Catalog order, so the summary reads like the return.
    for c in CATEGORIES {
        if let Some(line) = by_cat.remove(c.key) {
            match c.kind {
                TaxKind::Income => out.income.push(line),
                TaxKind::CostOfGoods => out.cost_of_goods.push(line),
                TaxKind::Deduction => out.deductions.push(line),
            }
        }
    }
    Ok(out)
}
