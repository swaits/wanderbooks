//! Fixed assets and depreciation: the asset register and the math.
//!
//! Scope is deliberate: **book depreciation only** (straight-line, GAAP
//! style). Tax depreciation — MACRS, Section 179, bonus depreciation — is a
//! separate set of books with its own rules and stays with the CPA. This
//! module never pretends to do tax.
//!
//! The register is master data owned by [`Ledger`](crate::Ledger): assets are journaled
//! like accounts (see `ledger-store`), and ids are allocator-deterministic
//! for replay. Depreciation itself is pure math — [`depreciation_schedule`]
//! — and postings go through the normal entry machinery, so every
//! accounting invariant holds.

use crate::{AccountId, AccountType, ChartOfAccounts, LedgerError, Money};
use chrono::{Datelike, Months, NaiveDate};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// Opaque handle for a fixed asset. Never forged: ids come from the ledger's
/// allocator.
pub type AssetId = u64;

/// Depreciation method. Straight-line is the whole v1; the enum reserves
/// room for declining-balance later.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DepreciationMethod {
    /// Even allocation over the useful life: (cost − salvage) / months,
    /// prorated by days for partial first and last months.
    StraightLine,
}

/// Lifecycle state of an asset.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AssetStatus {
    /// In service.
    Active,
    /// Retired or sold.
    Disposed {
        /// Disposal date.
        date: NaiveDate,
        /// Cash (or receivable) received.
        proceeds: Money,
    },
}

/// A fixed asset on the books.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Asset {
    /// Opaque handle.
    pub id: AssetId,
    /// What it is, e.g. "2023 Sprinter van".
    pub description: String,
    /// Purchase date. Depreciation starts here.
    pub purchase_date: NaiveDate,
    /// Capitalized cost.
    pub cost: Money,
    /// Expected residual value at end of life.
    pub salvage_value: Money,
    /// Useful life in months.
    pub useful_life_months: u32,
    /// Depreciation method.
    pub method: DepreciationMethod,
    /// Asset account carrying the cost.
    pub cost_account: AccountId,
    /// Contra-asset account carrying accumulated depreciation.
    pub accum_depr_account: AccountId,
    /// Expense account for the periodic charge.
    pub expense_account: AccountId,
    /// Lifecycle state.
    pub status: AssetStatus,
}

/// Parameters for registering an asset. A struct (not eight arguments)
/// because the list is long and stable.
#[derive(Clone, Debug)]
pub struct NewAsset {
    /// What it is.
    pub description: String,
    /// Purchase date.
    pub purchase_date: NaiveDate,
    /// Capitalized cost. Must be positive, in the ledger's base currency.
    pub cost: Money,
    /// Residual value. Non-negative, less than cost, base currency.
    pub salvage_value: Money,
    /// Useful life in months, at least 1.
    pub useful_life_months: u32,
    /// Asset account for the cost.
    pub cost_account: AccountId,
    /// Contra-asset account for accumulated depreciation.
    pub accum_depr_account: AccountId,
    /// Expense account for the charge.
    pub expense_account: AccountId,
}

/// One month of a depreciation schedule.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduleLine {
    /// Calendar month, "YYYY-MM".
    pub period: String,
    /// Depreciation for the month, exact cents.
    pub amount: Decimal,
    /// Accumulated depreciation through this month, exact cents.
    pub accumulated: Decimal,
    /// Book value after this month: cost − accumulated.
    pub book_value: Decimal,
}

fn days_in_month(year: i32, month: u32) -> u32 {
    let first = NaiveDate::from_ymd_opt(year, month, 1).expect("valid month");
    let next = if month == 12 {
        NaiveDate::from_ymd_opt(year + 1, 1, 1).expect("valid month")
    } else {
        NaiveDate::from_ymd_opt(year, month + 1, 1).expect("valid month")
    };
    (next - first).num_days() as u32
}

/// The last day depreciation accrues: purchase + life, or disposal, whichever
/// is earlier. The accrual window is [purchase_date, depreciation_end).
fn depreciation_end(asset: &Asset) -> NaiveDate {
    let natural = asset
        .purchase_date
        .checked_add_months(Months::new(asset.useful_life_months))
        .expect("useful life does not overflow dates");
    match &asset.status {
        AssetStatus::Active => natural,
        AssetStatus::Disposed { date, .. } => (*date).min(natural),
    }
}

/// Straight-line depreciation schedule from the purchase month through
/// `through` (clamped to the depreciation end): one line per month, the
/// last prorated by days when `through` falls mid-month.
///
/// ```text
/// GOAL: how much of an asset's cost has been used up, month by month.
/// SOURCE: docs/spec/rules.md B-15 (straight line; salvage never
///         depreciated; partial months by days); DECISION (owner,
///         2026-10-13): a schedule through a mid-month date prorates that
///         month by days, as partial first and last months do.
/// INVARIANTS:
///   1. A full schedule MUST sum to exactly cost - salvage, to the cent.
///   2. Accumulated depreciation NEVER falls; book value NEVER rises and
///      NEVER drops below salvage.
/// SCENARIOS:
///   1. IF $12,000 over 12 months from Jan 1, through Jun 15:
///      - SHOULD end with June at 500.00 (15 of 30 days), accumulated
///        5,500.00
///      BECAUSE depreciation accrues by the day within a month.
/// ```
///
/// Partial first and last months are prorated by days held. Each month is
/// rounded to cents via the running-total method, and the final month
/// trues up: partial months in different-length calendar months don't
/// complement exactly (17/31 + 14/30 ≠ 1), so the last month absorbs the
/// drift and the lifetime total is exactly (cost − salvage) — never a
/// penny off.
pub fn depreciation_schedule(asset: &Asset, through: NaiveDate) -> Vec<ScheduleLine> {
    if through < asset.purchase_date {
        return Vec::new();
    }
    let depreciable = asset.cost.amount - asset.salvage_value.amount;
    if depreciable <= Decimal::ZERO {
        return Vec::new();
    }
    let end = depreciation_end(asset);
    // Accrual is inclusive of `through`: work with an exclusive end.
    let accrual_end = through.succ_opt().unwrap_or(through).min(end);
    if accrual_end <= asset.purchase_date {
        return Vec::new();
    }
    let monthly_rate = depreciable / Decimal::from(asset.useful_life_months);

    let mut out = Vec::new();
    let mut cumulative_exact = Decimal::ZERO;
    let mut accumulated_rounded = Decimal::ZERO;
    // Walk calendar months from the purchase month.
    let (mut year, mut month) = (asset.purchase_date.year(), asset.purchase_date.month());
    loop {
        let month_start = NaiveDate::from_ymd_opt(year, month, 1).expect("valid month");
        if month_start >= accrual_end {
            break;
        }
        let dim = days_in_month(year, month);
        let month_end_excl = month_start
            .checked_add_months(Months::new(1))
            .expect("month arithmetic");
        // Days held this month: overlap of [purchase_date, accrual_end) with the month.
        let held_start = asset.purchase_date.max(month_start);
        let held_end = accrual_end.min(month_end_excl);
        let days_held = (held_end - held_start).num_days().max(0) as u32;
        if days_held > 0 {
            let exact = monthly_rate * Decimal::from(days_held) / Decimal::from(dim);
            cumulative_exact += exact;
            let rounded_cumulative = cumulative_exact
                .round_dp_with_strategy(2, rust_decimal::RoundingStrategy::MidpointAwayFromZero);
            let amount = rounded_cumulative - accumulated_rounded;
            accumulated_rounded = rounded_cumulative;
            out.push(ScheduleLine {
                period: format!("{year:04}-{month:02}"),
                amount,
                accumulated: accumulated_rounded,
                book_value: asset.cost.amount - accumulated_rounded,
            });
        }
        if month == 12 {
            year += 1;
            month = 1;
        } else {
            month += 1;
        }
    }
    // True-up: if the schedule reaches the final month of the asset's life,
    // absorb any month-length drift so the lifetime total is exactly
    // (cost − salvage). The drift is bounded by a fraction of one month's
    // charge, so the final month stays positive.
    if let Some(last) = out.last_mut()
        && let Some(day_before_end) = end.pred_opt()
        && last.period == day_before_end.format("%Y-%m").to_string()
    {
        let plug = depreciable - accumulated_rounded;
        last.amount += plug;
        last.accumulated = depreciable;
        last.book_value = asset.cost.amount - depreciable;
    }
    out
}

/// Accumulated depreciation through `as_of` (inclusive), exact cents.
pub fn accumulated_depreciation(asset: &Asset, as_of: NaiveDate) -> Decimal {
    depreciation_schedule(asset, as_of)
        .last()
        .map(|l| l.accumulated)
        .unwrap_or(Decimal::ZERO)
}

/// Book value (cost − accumulated depreciation) as of a date.
pub fn book_value(asset: &Asset, as_of: NaiveDate) -> Decimal {
    asset.cost.amount - accumulated_depreciation(asset, as_of)
}

/// The amounts for a disposal entry: what to debit/credit where.
/// Gain (proceeds > book value) is positive; loss is negative.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisposalAmounts {
    /// Book value at disposal.
    pub book_value: Decimal,
    /// Accumulated depreciation at disposal.
    pub accumulated: Decimal,
    /// proceeds − book_value. Positive is a gain.
    pub gain_loss: Decimal,
}

/// Compute disposal amounts. Pure math; posting is the caller's job.
/// A disposal may fall on the purchase date (one day held, accrual being
/// inclusive, B-15 in docs/spec/rules.md); one before it is an error.
pub fn disposal_amounts(
    asset: &Asset,
    date: NaiveDate,
    proceeds: Money,
) -> Result<DisposalAmounts, LedgerError> {
    if asset.status != AssetStatus::Active {
        let id = asset.id;
        return Err(LedgerError::AssetAlreadyDisposed(id));
    }
    if proceeds.amount < Decimal::ZERO {
        return Err(LedgerError::NegativeProceeds(proceeds.amount));
    }
    if date < asset.purchase_date {
        return Err(LedgerError::DisposalBeforePurchase(date.to_string()));
    }
    let accumulated = accumulated_depreciation(asset, date);
    let book = asset.cost.amount - accumulated;
    Ok(DisposalAmounts {
        book_value: book,
        accumulated,
        gain_loss: proceeds.amount - book,
    })
}

/// Validate asset account links against the chart: cost and accumulated
/// depreciation must be asset accounts, the expense account an expense
/// account, and all three distinct.
pub(crate) fn validate_asset_accounts(
    chart: &ChartOfAccounts,
    cost_account: AccountId,
    accum_depr_account: AccountId,
    expense_account: AccountId,
) -> Result<(), LedgerError> {
    for (id, role, expected) in [
        (cost_account, "cost", AccountType::Asset),
        (
            accum_depr_account,
            "accumulated depreciation",
            AccountType::Asset,
        ),
        (
            expense_account,
            "depreciation expense",
            AccountType::Expense,
        ),
    ] {
        let account = chart.get(id).ok_or(LedgerError::UnknownAccount(id))?;
        if account.account_type != expected {
            return Err(LedgerError::AssetAccountWrongType {
                code: account.code.clone(),
                role,
                expected: match expected {
                    AccountType::Asset => "asset",
                    AccountType::Expense => "expense",
                    _ => unreachable!("asset links are asset or expense accounts"),
                },
                found: match account.account_type {
                    AccountType::Asset => "asset",
                    AccountType::Liability => "liability",
                    AccountType::Equity => "equity",
                    AccountType::Revenue => "revenue",
                    AccountType::Expense => "expense",
                },
            });
        }
    }
    for dup in [cost_account, accum_depr_account, expense_account] {
        let uses = [cost_account, accum_depr_account, expense_account]
            .iter()
            .filter(|&&id| id == dup)
            .count();
        if uses > 1 {
            return Err(LedgerError::DuplicateAssetAccount(dup));
        }
    }
    Ok(())
}
