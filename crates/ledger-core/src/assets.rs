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
    /// The label on the thing itself, e.g. "WL-0001" (see
    /// [`check_asset_tag`]); `None` until one is set.
    #[serde(default)]
    pub tag: Option<String>,
    /// The manufacturer's serial number, if recorded.
    #[serde(default)]
    pub serial: Option<String>,
    /// The posted entry that put it on the books (its purchase, or a
    /// member's contribution of it), if linked
    /// ([`crate::Ledger::set_asset_acquisition`]).
    #[serde(default)]
    pub acquired_by: Option<crate::EntryId>,
    /// Its federal tax basis, if recorded ([`crate::Ledger::set_asset_tax_basis`]).
    /// Book cost is [`Asset::cost`]; tax basis can differ (a member's
    /// contribution carries the member's basis, not its market value).
    #[serde(default)]
    pub tax: Option<TaxBasis>,
}

/// Corrected economics for a registered asset ([`crate::Ledger::correct_asset`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AssetCorrection {
    /// Capitalized cost.
    pub cost: Money,
    /// Residual value.
    pub salvage_value: Money,
    /// Useful life in months.
    pub useful_life_months: u32,
    /// In-service date (depreciation starts here).
    pub purchase_date: NaiveDate,
}

/// Which rule sets an asset's federal tax basis.
///
/// ```text
/// SOURCE: docs/spec/sources/irs-partnership-basis.md: IRC §1012(a) (cost),
///         IRC §723 (property a partner contributes keeps the partner's
///         adjusted basis), Treas. Reg. §1.168(i)-4(b) (property converted
///         from personal use depreciates from the lesser of its fair market
///         value and its adjusted basis), IRC §704(c)(1)(C) (built-in loss).
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaxBasisRule {
    /// Bought by the company: its cost (IRC §1012(a)), the book cost.
    Cost,
    /// Contributed by a member from business use: the member's adjusted
    /// basis carries over (IRC §723).
    Carryover,
    /// Contributed by a member from personal use: depreciable basis is the
    /// lesser of fair market value (the book cost) and the member's
    /// adjusted basis (Treas. Reg. §1.168(i)-4(b), with IRC §723).
    PersonalUseConversion,
}

impl TaxBasisRule {
    /// Its wire name: `cost`, `carryover`, `personal_use_conversion`.
    pub fn key(self) -> &'static str {
        match self {
            TaxBasisRule::Cost => "cost",
            TaxBasisRule::Carryover => "carryover",
            TaxBasisRule::PersonalUseConversion => "personal_use_conversion",
        }
    }

    /// The rule named `key`.
    pub fn parse(key: &str) -> Option<Self> {
        [Self::Cost, Self::Carryover, Self::PersonalUseConversion]
            .into_iter()
            .find(|r| r.key() == key)
    }
}

/// An asset's federal tax basis: the rule and, for a member's
/// contribution, the member's adjusted basis at the time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaxBasis {
    /// The rule.
    pub rule: TaxBasisRule,
    /// The contributing member's adjusted basis at contribution (cost less
    /// any depreciation they took). `None` for [`TaxBasisRule::Cost`].
    pub contributor_basis: Option<Decimal>,
}

impl TaxBasis {
    /// Whether the record fits its rule: Cost takes no contributor basis;
    /// Carryover and PersonalUseConversion require a non-negative one in
    /// cents at most, and no larger than a cost may be
    /// (kernel::MAX_LINE_AMOUNT: the register sums them; P0 review round 3,
    /// S1). Returns why not. Needs no asset, so a caller can check before
    /// registering one. Replay checks only [`Self::check_journaled`].
    pub fn check(&self) -> Result<(), String> {
        self.check_journaled()?;
        match self.contributor_basis {
            Some(b) if b > crate::kernel::MAX_LINE_AMOUNT => {
                Err("a basis is at most 1000000000000000".into())
            }
            _ => Ok(()),
        }
    }

    /// The checks every journaled `asset.tax_basis_set` passed when it was
    /// written (the rule through 2026-10-16): [`Self::check`] without the
    /// upper bound. Replay applies exactly these (rules.md B-28).
    pub fn check_journaled(&self) -> Result<(), String> {
        use TaxBasisRule::*;
        match (self.rule, self.contributor_basis) {
            (Cost, Some(_)) => {
                Err("a purchase's tax basis is its cost; give no contributor basis".into())
            }
            (Carryover | PersonalUseConversion, None) => {
                Err("a contribution needs the contributing member's adjusted basis".into())
            }
            (_, Some(b)) if b.is_sign_negative() && !b.is_zero() => {
                Err("a basis can't be negative".into())
            }
            (_, Some(b)) if b.normalize().scale() > 2 => Err("a basis is in cents at most".into()),
            _ => Ok(()),
        }
    }
}

impl Asset {
    /// Its depreciable federal tax basis, by its rule, or `None` if not
    /// recorded.
    ///
    /// ```text
    /// INVARIANTS:
    ///   1. Cost: the book cost.
    ///   2. Carryover: the contributor's adjusted basis.
    ///   3. PersonalUseConversion: min(book cost, contributor's basis); the
    ///      book cost of a contribution is its fair market value.
    /// ```
    pub fn tax_basis(&self) -> Option<Decimal> {
        let t = self.tax?;
        Some(match (t.rule, t.contributor_basis) {
            (TaxBasisRule::Cost, _) => self.cost.amount,
            (TaxBasisRule::Carryover, Some(b)) => b,
            (TaxBasisRule::PersonalUseConversion, Some(b)) => b.min(self.cost.amount),
            (_, None) => return None,
        })
    }

    /// Built-in gain (positive) or loss (negative) at contribution, for
    /// IRC §704(c): fair market value (the book cost) less the contributor's
    /// adjusted basis. `None` for a purchase or when not recorded. A
    /// built-in loss counts only for the contributing member
    /// (§704(c)(1)(C)).
    pub fn built_in_gain(&self) -> Option<Decimal> {
        let b = self.tax?.contributor_basis?;
        Some(self.cost.amount - b)
    }
}

/// The longest asset tag.
pub const MAX_ASSET_TAG_LEN: usize = 32;
/// The longest serial number.
pub const MAX_SERIAL_LEN: usize = 64;

/// Check an asset tag's form: 1 to [`MAX_ASSET_TAG_LEN`] characters, each
/// an ASCII letter or digit or one of `-` `_` `.` `/`, at least one of
/// them a letter or digit. Returns why not.
///
/// ```text
/// GOAL: a tag prints on a small label, reads aloud, scans from a QR code,
///       and types back in exactly.
/// SOURCE: DECISION (Stephen, 2026-10-15): generated tags ("WL-0001") or
///         the human's own.
/// INVARIANTS:
///   1. Accepted tags contain no spaces, no control or non-ASCII
///      characters, and at least one letter or digit.
/// ```
pub fn check_asset_tag(tag: &str) -> Result<(), String> {
    if tag.is_empty() {
        return Err("a tag can't be empty".into());
    }
    if tag.len() > MAX_ASSET_TAG_LEN {
        return Err(format!("a tag is at most {MAX_ASSET_TAG_LEN} characters"));
    }
    if let Some(c) = tag
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/')))
    {
        return Err(format!(
            "{c:?} can't be in a tag (letters, digits, - _ . / only)"
        ));
    }
    if !tag.chars().any(|c| c.is_ascii_alphanumeric()) {
        return Err("a tag needs a letter or digit".into());
    }
    Ok(())
}

/// Check a serial number's form: 1 to [`MAX_SERIAL_LEN`] characters, no
/// control characters, no leading or trailing space. Returns why not.
pub fn check_serial(serial: &str) -> Result<(), String> {
    if serial.trim().is_empty() {
        return Err("a serial number can't be empty".into());
    }
    if serial.chars().count() > MAX_SERIAL_LEN {
        return Err(format!(
            "a serial number is at most {MAX_SERIAL_LEN} characters"
        ));
    }
    if serial.trim() != serial {
        return Err("a serial number can't start or end with a space".into());
    }
    if serial.chars().any(char::is_control) {
        return Err("a serial number can't contain control characters".into());
    }
    Ok(())
}

/// The key tags are compared by: tags are unique ignoring case
/// ("wl-0001" is "WL-0001").
pub fn tag_key(tag: &str) -> String {
    tag.to_ascii_uppercase()
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

/// The days in a month of a year chrono holds, without stepping to the
/// next month, so chrono's last month (December 262142, whose next month
/// chrono can't hold) has its 31 days too (rules.md B-28: a schedule an
/// earlier version accepted is computed, never a panic).
fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        2 if NaiveDate::from_ymd_opt(year, 2, 29).is_some() => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// The first day of the month after `month_start`'s, or `None` past
/// chrono's last day (the month containing NaiveDate::MAX).
fn next_month_start(month_start: NaiveDate) -> Option<NaiveDate> {
    month_start.checked_add_months(Months::new(1))
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
///         depreciated; partial months by days); DECISION (Stephen,
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
    // Past chrono's last month there is no month to walk, and nothing left
    // to accrue: accrual_end is at most NaiveDate::MAX.
    while let Some(month_start) = NaiveDate::from_ymd_opt(year, month, 1) {
        if month_start >= accrual_end {
            break;
        }
        let dim = days_in_month(year, month);
        // Days held this month: overlap of [purchase_date, accrual_end) with
        // the month. In chrono's last month there is no next month to end
        // at, and accrual_end (at most NaiveDate::MAX) is the bound.
        let held_start = asset.purchase_date.max(month_start);
        let held_end = next_month_start(month_start).map_or(accrual_end, |n| accrual_end.min(n));
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
    // The final month named as the lines name theirs: chrono's "%Y" signs
    // a year past 9999 ("+10004"), and the lines don't, so books an
    // earlier version wrote with such a life true up too (rules.md B-28).
    if let Some(last) = out.last_mut()
        && let Some(day_before_end) = end.pred_opt()
        && last.period == format!("{:04}-{:02}", day_before_end.year(), day_before_end.month())
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
