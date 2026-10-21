//! Fixed assets and depreciation: the asset register and the math.
//!
//! Scope is deliberate: **book depreciation only** (straight-line, GAAP
//! style). Tax depreciation — MACRS, Section 179, bonus depreciation — is a
//! separate set of books with its own rules and stays with the CPA. This
//! module never pretends to do tax.
//!
//! The register is master data owned by [`Ledger`]: assets are journaled
//! like accounts (see `ledger-store`), and ids are allocator-deterministic
//! for replay. Depreciation itself is pure math — [`depreciation_schedule`]
//! — and postings go through the normal entry machinery, so every
//! accounting invariant holds.
//!
//! Two figures of accumulated depreciation, never confused (rules.md B-15 rule 8):
//! what the schedule says is due through a date
//! ([`accumulated_scheduled_through`]) and what the books carry, the runs
//! posted and not reversed ([`crate::Ledger::accumulated_posted`]). Book
//! value is cost less the posted figure. Two dates: the in-service date starts depreciation; the
//! acquired date (the acquisition entry's) puts the asset on the register.
//! Three tax figures beside book value, each named for what it is
//! ([`Asset::basis_figures`]): the §723 basis, the depreciable basis, and
//! the §704(c) built-in gain or loss at contribution.

use crate::money::Steps;
use crate::{
    AccountId, AccountType, ChartOfAccounts, EntryId, EntryStatus, Ledger, LedgerError, Money,
    kernel,
};
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
    /// Even allocation over the useful life by the day: (cost − salvage) /
    /// months a month, prorated by days for a partial first month and any
    /// month a schedule stops in; the life's final month takes what the
    /// others leave, evenly over its days, so the life ends at exactly
    /// cost − salvage ([`depreciation_schedule`]).
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
///
/// Its serde form is internal and frozen: the books an earlier version
/// wrote are compared with it field for field
/// (`ledger-store/tests/fixtures/old-books`), so the in-service date keeps
/// the name `purchase_date` and the basis the name `tax` there.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Asset {
    /// Opaque handle.
    pub id: AssetId,
    /// What it is, e.g. "2023 Sprinter van".
    pub description: String,
    /// The day it was placed in service: depreciation starts here (B-15).
    /// The day the company acquired it is its acquisition entry's
    /// ([`crate::Ledger::asset_acquired_date`]), never later than this.
    #[serde(rename = "purchase_date")]
    pub in_service_date: NaiveDate,
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
    /// The label on the thing itself, e.g. "FL-0001" (see
    /// [`check_asset_tag`]); `None` until one is set.
    #[serde(default)]
    pub tag: Option<String>,
    /// The manufacturer's serial number, if recorded.
    #[serde(default)]
    pub serial: Option<String>,
    /// The posted entry that put it on the books (its purchase, or a
    /// member's contribution of it), if linked
    /// ([`crate::Ledger::set_asset_acquisition`]). Its date is the asset's
    /// acquired date.
    #[serde(default)]
    pub acquired_by: Option<crate::EntryId>,
    /// How the company got it and the facts its tax figures follow from,
    /// if recorded ([`crate::Ledger::set_asset_basis`]). Book cost is
    /// [`Asset::cost`]; the tax figures can differ (a member's contribution
    /// carries the member's basis, not its market value).
    #[serde(default, rename = "tax")]
    pub basis: Option<AssetBasis>,
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
    pub in_service_date: NaiveDate,
}

/// How the company got an asset, which sets its tax figures
/// ([`crate::kernel::basis_figures`]).
///
/// Its wire names ([`Self::key`]) are `purchase`, `carryover_723` and
/// `personal_use_conversion`. Books written before P3 named the first two
/// `cost` and `carryover` ([`Self::parse_legacy`]); the serde names stay
/// those (internal and frozen, see [`Asset`]).
///
/// ```text
/// SOURCE: docs/spec/sources/irs-partnership-basis.md: IRC §1012(a) (cost),
///         IRC §723 (property a partner contributes keeps the partner's
///         adjusted basis), Treas. Reg. §1.168(i)-4(b) (property converted
///         from personal use depreciates from the lesser of its fair market
///         value and its adjusted basis), IRC §704(c) (built-in gain and
///         loss).
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BasisRule {
    /// Bought by the company: its basis is its cost (IRC §1012(a)), the
    /// book cost.
    #[serde(rename = "cost")]
    Purchase,
    /// Contributed by a member from business use: the company's basis is
    /// the member's adjusted basis (IRC §723), and depreciation starts from
    /// it.
    #[serde(rename = "carryover")]
    Carryover723,
    /// Contributed by a member from personal use: the §723 basis as for
    /// carryover, and a depreciable basis of the lesser of fair market
    /// value and that basis (Treas. Reg. §1.168(i)-4(b); DECISION D-41,
    /// owner 2026-10-20, the CPA to confirm at year end).
    #[serde(rename = "personal_use_conversion")]
    PersonalUseConversion,
}

impl BasisRule {
    /// Every rule.
    pub const ALL: [Self; 3] = [
        Self::Purchase,
        Self::Carryover723,
        Self::PersonalUseConversion,
    ];

    /// Its wire name: `purchase`, `carryover_723`, `personal_use_conversion`.
    pub fn key(self) -> &'static str {
        match self {
            BasisRule::Purchase => "purchase",
            BasisRule::Carryover723 => "carryover_723",
            BasisRule::PersonalUseConversion => "personal_use_conversion",
        }
    }

    /// The rule named `key` (a wire name).
    pub fn parse(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.key() == key)
    }

    /// The rule a pre-P3 name means: `cost`, `carryover` or
    /// `personal_use_conversion`. Only for replaying `asset.tax_basis_set`,
    /// reading the projection, and the tools' aliases (one release).
    pub fn parse_legacy(key: &str) -> Option<Self> {
        match key {
            "cost" => Some(Self::Purchase),
            "carryover" => Some(Self::Carryover723),
            "personal_use_conversion" => Some(Self::PersonalUseConversion),
            _ => None,
        }
    }

    /// Whether a member contributed it (§723 and §704(c) apply).
    pub fn is_contribution(self) -> bool {
        self != Self::Purchase
    }
}

/// How the company allocates the tax items of a contributed property
/// among the members, IRC §704(c): chosen property by property
/// (Treas. Reg. §1.704-3(a)(2)).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sec704cMethod {
    /// The traditional method, Treas. Reg. §1.704-3(b): tax items follow
    /// the book allocation to the noncontributing members as far as the
    /// property's tax items allow (the ceiling rule).
    Traditional,
    /// The traditional method with curative allocations, Treas. Reg.
    /// §1.704-3(c): other tax items correct the ceiling rule's distortion.
    TraditionalWithCurative,
    /// The remedial allocation method, Treas. Reg. §1.704-3(d).
    Remedial,
}

impl Sec704cMethod {
    /// Every method.
    pub const ALL: [Self; 3] = [
        Self::Traditional,
        Self::TraditionalWithCurative,
        Self::Remedial,
    ];

    /// Its wire name: `traditional`, `traditional_with_curative`,
    /// `remedial`.
    pub fn key(self) -> &'static str {
        match self {
            Self::Traditional => "traditional",
            Self::TraditionalWithCurative => "traditional_with_curative",
            Self::Remedial => "remedial",
        }
    }

    /// The method named `key`.
    pub fn parse(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.key() == key)
    }

    /// The paragraph of Treas. Reg. §1.704-3 that describes it.
    pub fn citation(self) -> &'static str {
        match self {
            Self::Traditional => "Treas. Reg. §1.704-3(b)",
            Self::TraditionalWithCurative => "Treas. Reg. §1.704-3(c)",
            Self::Remedial => "Treas. Reg. §1.704-3(d)",
        }
    }
}

/// How the company got an asset and the facts its tax figures follow
/// from (facts only: the figures are derived, [`Asset::basis_figures`]).
///
/// Its serde form keeps the pre-P3 shape for a record written then
/// (`{rule, contributor_basis}`): the newer facts are left out when absent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetBasis {
    /// The rule.
    pub rule: BasisRule,
    /// The contributing member's adjusted basis at contribution (cost plus
    /// improvements, less depreciation allowed or allowable, IRC
    /// §1016(a)). `None` for a purchase.
    #[serde(rename = "contributor_basis")]
    pub contributor_adjusted_basis: Option<Decimal>,
    /// Fair market value at contribution. `None` for a purchase, and for a
    /// contribution recorded before P3, whose value is read as the book
    /// cost ([`Asset::fmv_at_contribution`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fmv_at_contribution: Option<Decimal>,
    /// The §704(c) method chosen for it, if chosen (contributions only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sec704c_method: Option<Sec704cMethod>,
    /// Who gave the facts, when, and on what (rules.md B-29).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<crate::settings::Source>,
}

impl AssetBasis {
    /// A purchase's record.
    pub fn purchase() -> Self {
        Self {
            rule: BasisRule::Purchase,
            contributor_adjusted_basis: None,
            fmv_at_contribution: None,
            sec704c_method: None,
            source: None,
        }
    }

    /// A contribution's record: the member's adjusted basis and the value
    /// at contribution.
    pub fn contribution(rule: BasisRule, adjusted: Decimal, fmv: Decimal) -> Self {
        Self {
            rule,
            contributor_adjusted_basis: Some(adjusted),
            fmv_at_contribution: Some(fmv),
            sec704c_method: None,
            source: None,
        }
    }

    /// Whether a new record fits its rule (for `asset.basis_set`): a
    /// purchase takes no contributor basis, value or method; a
    /// contribution needs the member's adjusted basis and the value at
    /// contribution, each non-negative, in cents at most, and no larger
    /// than a cost may be (kernel::MAX_LINE_AMOUNT); its source, if given,
    /// passes Source::check. Returns why not. Needs no asset, so a caller
    /// can check before registering one. Replay checks only
    /// [`Self::check_journaled`].
    pub fn check(&self) -> Result<(), String> {
        self.check_journaled()?;
        if let Some(source) = &self.source {
            source.check().map_err(|why| format!("source: {why}"))?;
        }
        for (what, v) in [
            ("an adjusted basis", self.contributor_adjusted_basis),
            ("a fair market value", self.fmv_at_contribution),
        ] {
            match v {
                Some(v) if v.normalize().scale() > 2 => {
                    return Err(format!("{what} is in cents at most"));
                }
                Some(v) if v > crate::kernel::MAX_LINE_AMOUNT => {
                    return Err(format!("{what} is at most one quadrillion"));
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// The checks every journaled `asset.basis_set` passed, which replay
    /// applies (rules.md B-28): the facts fit the rule and none is
    /// negative. No bound on size or places.
    pub fn check_journaled(&self) -> Result<(), String> {
        if self.rule == BasisRule::Purchase {
            if self.contributor_adjusted_basis.is_some()
                || self.fmv_at_contribution.is_some()
                || self.sec704c_method.is_some()
            {
                return Err("a purchase's basis is its cost: give no contributor basis, value at contribution or §704(c) method".into());
            }
            return Ok(());
        }
        match (self.contributor_adjusted_basis, self.fmv_at_contribution) {
            (None, _) => {
                Err("a contribution needs the contributing member's adjusted basis".into())
            }
            (_, None) => Err("a contribution needs its fair market value at contribution".into()),
            (Some(b), _) if b.is_sign_negative() && !b.is_zero() => {
                Err("a basis can't be negative".into())
            }
            (_, Some(v)) if v.is_sign_negative() && !v.is_zero() => {
                Err("a value can't be negative".into())
            }
            _ => Ok(()),
        }
    }

    /// The checks every journaled `asset.tax_basis_set` passed when it was
    /// written (the rule through 2026-10-16), for a record with no value
    /// at contribution: a purchase takes no contributor basis; a
    /// contribution needs one, non-negative, in cents at most. Replay of
    /// that event applies exactly these (rules.md B-28).
    pub fn check_legacy_journaled(&self) -> Result<(), String> {
        use BasisRule::*;
        match (self.rule, self.contributor_adjusted_basis) {
            (Purchase, Some(_)) => {
                Err("a purchase's tax basis is its cost; give no contributor basis".into())
            }
            (Carryover723 | PersonalUseConversion, None) => {
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
    /// Its fair market value at contribution: the value recorded, or for a
    /// contribution recorded before P3 (no value recorded), the book cost,
    /// which is what that version assumed. `None` for a purchase or when
    /// no basis is recorded.
    pub fn fmv_at_contribution(&self) -> Option<Decimal> {
        let b = self.basis.as_ref()?;
        if !b.rule.is_contribution() {
            return None;
        }
        Some(b.fmv_at_contribution.unwrap_or(self.cost.amount))
    }

    /// Whether its value at contribution was recorded as a fact (false for
    /// a record from before P3, read as the book cost, and for a purchase).
    pub fn fmv_is_recorded(&self) -> bool {
        self.basis
            .as_ref()
            .is_some_and(|b| b.fmv_at_contribution.is_some())
    }

    /// Its three tax figures (kernel::basis_figures), or `None` when no
    /// basis is recorded. The figures are at contribution (or purchase):
    /// the CPA adjusts them for tax depreciation (IRC §1016(a)(2)), and
    /// the §704(c) gain or loss as Treas. Reg. §1.704-3(a)(3)(ii) goes on
    /// to reduce it.
    ///
    /// ```text
    /// INVARIANTS:
    ///   1. A purchase's depreciable basis is the book cost now.
    ///   2. A contribution's figures follow its facts: the member's
    ///      adjusted basis and the value at contribution (the book cost for
    ///      a record from before P3, which reproduces that version's
    ///      figures exactly).
    ///   3. NEVER panics: a figure past exact arithmetic is refused
    ///      (TooLargeToCompute).
    /// ```
    pub fn basis_figures(
        &self,
    ) -> Result<Option<crate::kernel::BasisFigures<Decimal>>, LedgerError> {
        let Some(b) = &self.basis else {
            return Ok(None);
        };
        let adjusted = b.contributor_adjusted_basis.unwrap_or(Decimal::ZERO);
        let fmv = self.fmv_at_contribution().unwrap_or(Decimal::ZERO);
        if b.rule.is_contribution() && b.contributor_adjusted_basis.is_none() {
            // A contribution without its basis never passes a check; one
            // can't be reported.
            return Ok(None);
        }
        crate::kernel::basis_figures(b.rule, self.cost.amount, adjusted, fmv)
            .map(Some)
            .ok_or_else(|| {
                LedgerError::TooLargeToCompute(format!("asset {}'s basis figures", self.id))
            })
    }

    /// Its §723 basis (a contribution: the member's adjusted basis), if
    /// recorded.
    pub fn tax_basis_723(&self) -> Option<Decimal> {
        self.basis_figures().ok().flatten()?.tax_basis_723
    }

    /// Its depreciable tax basis, if recorded.
    pub fn depreciable_basis(&self) -> Option<Decimal> {
        Some(self.basis_figures().ok().flatten()?.depreciable_basis)
    }

    /// Its §704(c) built-in gain (+) or loss (−) at contribution, if a
    /// contribution is recorded.
    pub fn built_in_gain_704c(&self) -> Option<Decimal> {
        self.basis_figures().ok().flatten()?.built_in_gain_704c
    }

    /// Whether it is contributed property with a built-in loss, which
    /// counts only for the contributing member(s): IRC §704(c)(1)(C).
    pub fn sec704c_1c(&self) -> bool {
        self.built_in_gain_704c().is_some_and(|g| g < Decimal::ZERO)
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
/// SOURCE: DECISION (owner, 2026-10-15): generated tags ("FL-0001") or
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
/// ("fl-0001" is "FL-0001").
pub fn tag_key(tag: &str) -> String {
    tag.to_ascii_uppercase()
}

/// Parameters for registering an asset. A struct (not eight arguments)
/// because the list is long and stable.
#[derive(Clone, Debug)]
pub struct NewAsset {
    /// What it is.
    pub description: String,
    /// The day it was placed in service: depreciation starts here. The
    /// acquired date is its acquisition entry's, linked after
    /// (`Ledger::set_asset_acquisition`).
    pub in_service_date: NaiveDate,
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

/// One depreciation run the books recorded for an asset
/// (`asset.depreciation_posted`): the month, the entry that posted it, and
/// the amount that entry credited to accumulated depreciation. A run counts
/// while its entry stands: a reversal takes it back, and a reversal of the
/// reversal reinstates it ([`crate::Ledger::accumulated_posted`]).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DepreciationPosting {
    /// The month run, "YYYY-MM" as journaled.
    pub period: String,
    /// The entry that posted it.
    pub entry: crate::EntryId,
    /// What it credited to accumulated depreciation.
    pub amount: Decimal,
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

/// The end of an asset's life, exclusive: its in-service date plus its
/// life in months (chrono clamps a day past a month's end, so Jan 31 plus
/// one month is Feb 28). The life is `[in_service_date, life_end)`.
/// Refused (DateOutOfRange), never a panic, past chrono's last day: replay
/// validation keeps every journaled asset's end a date, so this is for
/// figures asked of a value no command took.
pub fn life_end(asset: &Asset) -> Result<NaiveDate, LedgerError> {
    asset
        .in_service_date
        .checked_add_months(Months::new(asset.useful_life_months))
        .ok_or(LedgerError::DateOutOfRange(asset.in_service_date))
}

/// One month of a schedule, with its month as numbers (the line names it
/// "YYYY-MM").
struct MonthLine {
    year: i32,
    month: u32,
    line: ScheduleLine,
}

/// Straight-line depreciation schedule from the in-service month through
/// `through`, inclusive, and through the disposal day of a disposed asset:
/// one line per month, the last prorated by days when the schedule stops
/// mid-month.
///
/// ```text
/// GOAL: how much of an asset's cost has been used up, month by month.
/// SOURCE: docs/spec/rules.md B-15 (straight line; salvage never
///         depreciated; partial months by days; ASC 360-10-35-4: cost less
///         salvage over the useful life "in a systematic and rational
///         manner", as equitably as possible to the periods served);
///         DECISION (owner, 2026-10-13): a schedule through a mid-month
///         date prorates that month by days, as partial first and last
///         months do.
/// INVARIANTS:
///   1. The life is [in service, in service + life months), cut into
///      calendar months (calendar::month_parts). Through day k of a month but the
///      last, accumulated is (cost − salvage) / life × (the months held
///      before it, each as days held over days in the month, + k / days
///      in this month): the monthly rate by days.
///   2. The last month of the life is charged what the months before it
///      leave, evenly over its days (kernel::prorated_through): through
///      its last day accumulated is exactly cost − salvage, and through an
///      earlier day it is prorated by days, never plugged to the whole.
///   3. Each accumulated figure is rounded half away from zero to the
///      currency's minor unit (cents; whole yen; a thousandth of a dinar);
///      a month's amount is the difference of consecutive accumulated
///      figures.
///   4. It stops at through, or at the disposal day of a disposed asset,
///      whichever is earlier, both inclusive (the disposal day is held:
///      disposal_amounts), and never runs past the life.
///   5. Accumulated depreciation NEVER falls and NEVER exceeds cost −
///      salvage; book value NEVER rises and NEVER drops below salvage.
///   6. Through every month end it equals the per-month running-total
///      method books posted before P3, except that a half-cent tie is
///      rounded exactly here (Decimal's 28-digit quotients could land on
///      either side of one).
///   7. NEVER panics: a figure that can't be computed (a life past
///      chrono's last day, amounts past exact arithmetic) is refused
///      (rules.md B-28).
/// SCENARIOS:
///   1. IF $12,000 over 12 months from Jan 1, through Jun 15:
///      - SHOULD end with June at 500.00 (15 of 30 days), accumulated
///        5,500.00
///      BECAUSE depreciation accrues by the day within a month.
///   2. IF the same asset through Dec 15, its final month:
///      - SHOULD end with December at 1,000 × 15/31 = 483.87, accumulated
///        11,483.87
///      - NEVER plug December to 1,000.00 before its last day
///      BECAUSE the final month is held by the day like any other.
///   3. IF $930 depreciable over 3 months from Jan 15 (the life ends
///      Apr 15; Jan 17/31, Feb, Mar leave 140.00 for Apr 1-14), through
///      Apr 13:
///      - SHOULD be April 130.00 (13 of its 14 days), accumulated 920.00
///      BECAUSE the month-length drift is spread over the final days.
///   4. IF the first asset is disposed of on Jun 15:
///      - SHOULD stop there: June 500.00, accumulated 5,500.00, what the
///        disposal posted
///      - NEVER show full depreciation for a disposed asset.
/// ```
pub fn depreciation_schedule(
    asset: &Asset,
    through: NaiveDate,
) -> Result<Vec<ScheduleLine>, LedgerError> {
    Ok(schedule_months(asset, through)?
        .into_iter()
        .map(|m| m.line)
        .collect())
}

/// [`depreciation_schedule`], each line with its year and month.
fn schedule_months(asset: &Asset, through: NaiveDate) -> Result<Vec<MonthLine>, LedgerError> {
    let stop = match &asset.status {
        AssetStatus::Active => through,
        AssetStatus::Disposed { date, .. } => through.min(*date),
    };
    if stop < asset.in_service_date {
        return Ok(Vec::new());
    }
    let too_large =
        || LedgerError::TooLargeToCompute(format!("asset {}'s depreciation schedule", asset.id));
    let depreciable = asset
        .cost
        .amount
        .checked_sub(asset.salvage_value.amount)
        .ok_or_else(too_large)?;
    if depreciable <= Decimal::ZERO {
        return Ok(Vec::new());
    }
    let end = life_end(asset)?;
    let steps = Steps::of(depreciable, asset.cost.currency).ok_or_else(too_large)?;
    let mut out = Vec::new();
    let (mut before, mut charged) = (0i128, 0i128);
    for p in crate::calendar::month_parts(asset.in_service_date, end) {
        if p.first > stop {
            break;
        }
        // Days of this month held through the stop day, inclusive.
        let held = (stop - p.first).num_days().saturating_add(1);
        let k = u32::try_from(held).map_or(p.part.days, |h| h.min(p.part.days));
        let through_k = kernel::prorated_through(
            steps.units,
            asset.useful_life_months,
            before,
            p.part,
            k,
            p.last,
            steps.unit,
        )
        .ok_or_else(too_large)?;
        let accumulated = steps.decimal(through_k).ok_or_else(too_large)?;
        out.push(MonthLine {
            year: p.year,
            month: p.month,
            line: ScheduleLine {
                period: format!("{:04}-{:02}", p.year, p.month),
                amount: steps.decimal(through_k - charged).ok_or_else(too_large)?,
                accumulated,
                book_value: asset
                    .cost
                    .amount
                    .checked_sub(accumulated)
                    .ok_or_else(too_large)?,
            },
        });
        charged = through_k;
        if k < p.part.days {
            break;
        }
        before = before
            .checked_add(kernel::part_weight(p.part).ok_or_else(too_large)?)
            .ok_or_else(too_large)?;
    }
    Ok(out)
}

/// Accumulated depreciation the schedule says is due through `as_of`
/// (inclusive), exact to the minor unit ([`depreciation_schedule`]): the
/// tools' `accumulated_scheduled_through`. What the books carry is
/// [`crate::Ledger::accumulated_posted`]; the two agree once every month
/// due is posted (rules.md B-15 rule 8).
pub fn accumulated_scheduled_through(
    asset: &Asset,
    as_of: NaiveDate,
) -> Result<Decimal, LedgerError> {
    Ok(depreciation_schedule(asset, as_of)?
        .last()
        .map(|l| l.accumulated)
        .unwrap_or(Decimal::ZERO))
}

/// Cost less the depreciation the *schedule* says is due through `as_of`:
/// the schedule's own book value (as [`ScheduleLine::book_value`]), never
/// what the books carry, which is cost less
/// [`crate::Ledger::accumulated_posted`] (the register's and the tools'
/// `book_value`, rules.md B-24).
pub fn scheduled_book_value(asset: &Asset, as_of: NaiveDate) -> Result<Decimal, LedgerError> {
    asset
        .cost
        .amount
        .checked_sub(accumulated_scheduled_through(asset, as_of)?)
        .ok_or_else(|| LedgerError::TooLargeToCompute(format!("asset {}'s book value", asset.id)))
}

/// A change to what the books carry for an asset's depreciation, in the
/// month of the entry that made it, as the catch-up rule reads it
/// ([`run_amount`], [`depreciation_effects`]): a run adds its amount in its
/// month; a reversal of the run takes it away (a negative amount) in the
/// reversal's month, and a reversal of that reversal adds it back in its
/// own. Summed through a month's end, they are what the books carry for
/// the asset's runs then.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PostedRun {
    /// The entry's year.
    pub year: i32,
    /// The entry's month, 1 to 12.
    pub month: u32,
    /// What it adds to what the books carry (negative for a reversal).
    pub amount: Decimal,
}

/// What an asset's depreciation runs did to what the books carry, entry by
/// entry, each in its own month: for every run (`runs`: each run's entry
/// and amount, reversed or not, its disposal's final depreciation
/// included), the run itself and every entry of its reversal chain
/// ([`Ledger::reversal_chain`]), alternately adding and taking away its
/// amount.
///
/// ```text
/// GOAL: the catch-up rule reads the books as they stood at each month
///       end: a run reversed after its month ended still carried that
///       month, so running the month again would carry it twice.
/// SOURCE: docs/spec/rules.md B-9 (a mistake is corrected by an opposite
///         entry, dated when it is made), B-10 (reversing the reversal
///         re-instates), B-15 (the catch-up rule).
/// INVARIANTS:
///   1. Through any month's end, the effects sum to what the runs' entries
///      and their reversals dated on or before it carry in the asset's
///      accumulated depreciation.
///   2. A run in effect sums to its amount; one reversed, to zero.
/// ```
pub fn depreciation_effects(ledger: &Ledger, runs: &[(EntryId, Decimal)]) -> Vec<PostedRun> {
    run_chain_entries(ledger, runs, &ledger.posted_reversals())
        .into_iter()
        .map(|(d, amount)| PostedRun {
            year: d.year(),
            month: d.month(),
            amount,
        })
        .collect()
}

/// Every entry of each run's reversal chain the books carry (posted), in
/// order, with its date and what it adds to what the books carry for the
/// asset: the run its amount, a reversal of it the amount taken away, a
/// reversal of that reversal the amount back, and so on (`reversals`:
/// each posted reversal's original, [`Ledger::posted_reversals`], read
/// once for many runs). The one reading of what the books carry for an
/// asset's depreciation: [`depreciation_effects`] by month, and
/// [`Ledger::accumulated_posted`] by day.
pub(crate) fn run_chain_entries(
    ledger: &Ledger,
    runs: &[(EntryId, Decimal)],
    reversals: &std::collections::HashMap<EntryId, EntryId>,
) -> Vec<(NaiveDate, Decimal)> {
    let mut out = Vec::new();
    for &(run, amount) in runs {
        let (mut link, mut adds) = (Some(run), true);
        // A chain has no more links than there are reversals: the bound
        // is a guard, never reached.
        for _ in 0..=reversals.len() {
            let Some(id) = link else { break };
            if let Some(e) = ledger
                .get_entry(id)
                .filter(|x| x.status() == EntryStatus::Posted)
            {
                out.push((e.date(), if adds { amount } else { -amount }));
            }
            link = reversals.get(&id).copied();
            adds = !adds;
        }
    }
    out
}

/// What a depreciation run for a month posts ([`run_amount`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunAmount {
    /// What the run posts: zero when nothing is due.
    pub amount: Decimal,
    /// The month's own scheduled amount ([`depreciation_schedule`]; zero
    /// for a month after the life).
    pub scheduled: Decimal,
    /// What the books carry beyond the schedule through the month's end:
    /// zero unless something carried them past it (a run an earlier
    /// version posted on a half-cent tie, a correction by hand), which no
    /// run takes back: a human corrects it.
    pub over: Decimal,
}

/// What a depreciation run would post for every month from the asset's
/// first through `through` (year, month), in one pass: [`run_amount`] of
/// each, as a viewer asks it of every month of a life (a 10,000-year life
/// included). It stops at the later of the life's last month and the last
/// month an entry in `posted` ([`depreciation_effects`]) falls in, if
/// `through` is past both: every month after them would say the same as
/// the first one after.
///
/// ```text
/// INVARIANTS:
///   1. Each month's amount and over are run_amount's for it
///      (kernel::catch_ups is catch_up of every suffix).
///   2. Empty when `through` is before the asset's first month.
/// ```
pub fn run_amounts(
    asset: &Asset,
    through: (i32, u32),
    posted: &[PostedRun],
) -> Result<Vec<(i32, u32, RunAmount)>, LedgerError> {
    let first = (asset.in_service_date.year(), asset.in_service_date.month());
    if through < first {
        return Ok(Vec::new());
    }
    let last_run = posted.iter().map(|r| (r.year, r.month)).max();
    let life_last = life_end(asset)?
        .pred_opt()
        .map_or(first, |d| (d.year(), d.month()));
    // One month past the life and the runs says what every later one does
    // (and past chrono's last month there is none).
    let after = life_last.max(last_run.unwrap_or(first));
    let settled = Some(next_month(after))
        .filter(|&(y, m)| NaiveDate::from_ymd_opt(y, m, 1).is_some())
        .unwrap_or(after);
    let through = through.min(settled);
    let latest = last_run.map_or(through, |r| r.max(through));
    let end = NaiveDate::from_ymd_opt(latest.0, latest.1, 1)
        .map(crate::schedules::month_end)
        .ok_or_else(|| {
            LedgerError::InvalidDepreciationPeriod(format!("{:04}-{:02}", latest.0, latest.1))
        })?;
    let lines = schedule_months(asset, end)?;
    let c = cumulatives(asset, first, latest, &lines, posted)?;
    let mut due = vec![Decimal::ZERO; c.months.len()];
    kernel::catch_ups(&c.scheduled, &c.carried, &mut due).ok_or_else(|| {
        LedgerError::TooLargeToCompute(format!("asset {}'s depreciation runs", asset.id))
    })?;
    let mut li = 0;
    Ok(c.months
        .into_iter()
        .zip(due)
        .take_while(|((y, m), _)| (*y, *m) <= through)
        .zip(c.scheduled.iter().zip(&c.carried))
        .map(|(((y, m), amount), (&s, &g))| {
            while lines.get(li).is_some_and(|l| (l.year, l.month) < (y, m)) {
                li += 1;
            }
            let own = lines
                .get(li)
                .filter(|l| (l.year, l.month) == (y, m))
                .map_or(Decimal::ZERO, |l| l.line.amount);
            (
                y,
                m,
                RunAmount {
                    amount,
                    scheduled: own,
                    over: g
                        .checked_sub(s)
                        .map_or(Decimal::ZERO, |o| o.max(Decimal::ZERO)),
                },
            )
        })
        .collect())
}

/// The month after (year, month).
fn next_month((y, m): (i32, u32)) -> (i32, u32) {
    if m == 12 { (y + 1, 1) } else { (y, m + 1) }
}

/// The schedule and what the books carry, each cumulative, at successive
/// month ends.
struct Cumulatives {
    /// The months, (year, month).
    months: Vec<(i32, u32)>,
    /// The schedule through each month's end.
    scheduled: Vec<Decimal>,
    /// What the books carry through each month's end: the effects
    /// ([`PostedRun`]) of months on or before it.
    carried: Vec<Decimal>,
}

/// The months from `first` through `latest`, with the schedule and what
/// the books carry, each cumulative, at every month end: one pass over the
/// months, the lines and the effects. The schedule through a month's end
/// is its line, or the last line before it (after the life, the whole of
/// it).
fn cumulatives(
    asset: &Asset,
    first: (i32, u32),
    latest: (i32, u32),
    lines: &[MonthLine],
    posted: &[PostedRun],
) -> Result<Cumulatives, LedgerError> {
    let too_large =
        || LedgerError::TooLargeToCompute(format!("asset {}'s depreciation run", asset.id));
    let mut runs: Vec<&PostedRun> = posted.iter().collect();
    runs.sort_by_key(|r| (r.year, r.month));
    let (mut months, mut scheduled, mut carried) = (Vec::new(), Vec::new(), Vec::new());
    let (mut through, mut carry) = (Decimal::ZERO, Decimal::ZERO);
    let (mut li, mut ri) = (0, 0);
    let (mut y, mut m) = first;
    loop {
        while let Some(l) = lines.get(li).filter(|l| (l.year, l.month) <= (y, m)) {
            through = l.line.accumulated;
            li += 1;
        }
        while let Some(r) = runs.get(ri).filter(|r| (r.year, r.month) <= (y, m)) {
            carry = carry.checked_add(r.amount).ok_or_else(too_large)?;
            ri += 1;
        }
        months.push((y, m));
        scheduled.push(through);
        carried.push(carry);
        if (y, m) >= latest {
            break;
        }
        (y, m) = next_month((y, m));
    }
    Ok(Cumulatives {
        months,
        scheduled,
        carried,
    })
}

/// What a depreciation run for `year`-`month` posts, dated at that month's
/// end, given what the books carry for the asset (`posted`: every run and
/// every reversal of one, each in its own month, [`depreciation_effects`]).
///
/// ```text
/// GOAL: the books carry the schedule through every month end a run
///       reaches; a month skipped is caught up in the month the run is
///       made, and a run never posts more than the schedule says.
/// SOURCE: plan/served.md P3 "Settled in design" (2026-10-20): a month of
///         depreciation skipped is caught up in the month the run is made
///         (an immaterial miss; a material one is restated, ASC 250, a
///         correction for a human and the CPA); docs/spec/rules.md B-15.
/// INVARIANTS:
///   1. amount = catch_up_room for the month: kernel::catch_up over the
///      month ends M from this month's through the latest month an effect
///      falls in, the schedule through M against what the books carry
///      through M. So amount >= 0, and the books never carry more than the
///      schedule at any of those month ends after this run (when they
///      didn't before).
///   2. With nothing after this month, amount is everything due through
///      its end: earlier months skipped, or a run reversed, are caught up
///      here.
///   3. A month a later run already caught up posts nothing; so does a
///      month whose run was reversed after the month ended (the books
///      carried it then: the run for the reversal's month catches it up),
///      so a month is never carried twice at its end.
/// SCENARIOS:
///   1. IF January ran, February was skipped, and March runs:
///      - SHOULD post February's and March's depreciation in March
///      BECAUSE a skipped month is caught up in the month the run is made.
///   2. IF February is run after that: SHOULD post nothing.
///   3. IF January's run is reversed on Feb 2 and January is run again:
///      - SHOULD post nothing, and a February run 2,000.00
///      - NEVER carry January twice on Jan 31
///      BECAUSE the reversal is February's correction, caught up there.
///   4. IF the month is after the life and everything is posted: SHOULD
///      post nothing; IF something is still due (a final month skipped,
///      or in a closed period): SHOULD post it.
///   5. IF the month ends before the asset was placed in service: SHOULD
///      refuse (InvalidDepreciationPeriod).
/// ```
pub fn run_amount(
    asset: &Asset,
    year: i32,
    month: u32,
    posted: &[PostedRun],
) -> Result<RunAmount, LedgerError> {
    let period = format!("{year:04}-{month:02}");
    if month_end_of(year, month)? < asset.in_service_date {
        return Err(LedgerError::InvalidDepreciationPeriod(format!(
            "{period} ends before asset {} was placed in service on {}",
            asset.id, asset.in_service_date
        )));
    }
    catch_up_room(asset, year, month, posted)
}

/// The last day of `year`-`month`, refused for a month chrono can't hold.
fn month_end_of(year: i32, month: u32) -> Result<NaiveDate, LedgerError> {
    NaiveDate::from_ymd_opt(year, month, 1)
        .map(crate::schedules::month_end)
        .ok_or_else(|| LedgerError::InvalidDepreciationPeriod(format!("{year:04}-{month:02}")))
}

/// The most an entry dated in `year`-`month` can add to what the books
/// carry for the asset (`posted`, [`depreciation_effects`]) without their
/// carrying more than its schedule at that month's end or any later one an
/// effect reaches: what a run of the month posts ([`run_amount`]), and the
/// most a reversal that re-instates a run may add back
/// (`Store::check_reverse_entry`). A month before the asset's first counts
/// its schedule as nothing.
///
/// ```text
/// INVARIANTS:
///   1. amount = kernel::catch_up(scheduled[M..], carried[M..]) over the
///      month ends from this month's through the latest an effect falls in;
///      over = what the books carry beyond the schedule through this
///      month's end (zero normally).
///   2. NEVER panics: a month chrono can't hold is refused.
/// ```
pub fn catch_up_room(
    asset: &Asset,
    year: i32,
    month: u32,
    posted: &[PostedRun],
) -> Result<RunAmount, LedgerError> {
    month_end_of(year, month)?;
    let latest = posted
        .iter()
        .map(|r| (r.year, r.month))
        .chain([(year, month)])
        .max()
        .expect("this month is in the list");
    let lines = schedule_months(asset, month_end_of(latest.0, latest.1)?)?;
    let too_large =
        || LedgerError::TooLargeToCompute(format!("asset {}'s depreciation run", asset.id));
    let c = cumulatives(asset, (year, month), latest, &lines, posted)?;
    let over = c.carried[0]
        .checked_sub(c.scheduled[0])
        .ok_or_else(too_large)?
        .max(Decimal::ZERO);
    Ok(RunAmount {
        amount: kernel::catch_up(&c.scheduled, &c.carried).ok_or_else(too_large)?,
        scheduled: lines
            .iter()
            .find(|l| (l.year, l.month) == (year, month))
            .map_or(Decimal::ZERO, |l| l.line.amount),
        over,
    })
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

/// Whether an asset can be disposed of for `proceeds`: it is active and
/// the proceeds aren't negative. The checks every journaled
/// disposal passed, so replaying one never depends on a figure (rules.md
/// B-28). The date's floor, the day the company acquired the asset, needs
/// its acquisition entry, so the ledger checks it for new disposals only
/// (`Ledger::dispose_asset`): a disposal may fall on the in-service date
/// (one day held, accrual being inclusive, B-15), or before it once the
/// asset is acquired (nothing accrued yet).
pub fn check_disposal(asset: &Asset, proceeds: Money) -> Result<(), LedgerError> {
    if asset.status != AssetStatus::Active {
        return Err(LedgerError::AssetAlreadyDisposed(asset.id));
    }
    if proceeds.amount < Decimal::ZERO {
        return Err(LedgerError::NegativeProceeds(proceeds.amount));
    }
    Ok(())
}

/// Compute disposal amounts by the schedule. Pure math; posting is the
/// caller's job. The checks are [`check_disposal`]'s (the acquired-date
/// floor is the ledger's, for new disposals). Accumulated depreciation is
/// the schedule through the disposal day, inclusive: what the schedule of
/// the asset, once disposed, shows ([`depreciation_schedule`] INVARIANT 4);
/// nothing before the in-service date.
pub fn disposal_amounts(
    asset: &Asset,
    date: NaiveDate,
    proceeds: Money,
) -> Result<DisposalAmounts, LedgerError> {
    check_disposal(asset, proceeds)?;
    let accumulated = accumulated_scheduled_through(asset, date)?;
    let too_large = || LedgerError::TooLargeToCompute(format!("asset {}'s disposal", asset.id));
    let book = asset
        .cost
        .amount
        .checked_sub(accumulated)
        .ok_or_else(too_large)?;
    Ok(DisposalAmounts {
        book_value: book,
        accumulated,
        gain_loss: proceeds.amount.checked_sub(book).ok_or_else(too_large)?,
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
