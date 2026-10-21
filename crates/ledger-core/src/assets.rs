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
    /// The label on the thing itself, e.g. "FL-0001" (see
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
                Err("a basis is at most one quadrillion".into())
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

/// The end of an asset's life, exclusive: its in-service date plus its
/// life in months (chrono clamps a day past a month's end, so Jan 31 plus
/// one month is Feb 28). The life is `[purchase_date, life_end)`.
/// Refused (DateOutOfRange), never a panic, past chrono's last day: replay
/// validation keeps every journaled asset's end a date, so this is for
/// figures asked of a value no command took.
pub fn life_end(asset: &Asset) -> Result<NaiveDate, LedgerError> {
    asset
        .purchase_date
        .checked_add_months(Months::new(asset.useful_life_months))
        .ok_or(LedgerError::DateOutOfRange(asset.purchase_date))
}

/// One month of a schedule, with its month as numbers (the line names it
/// "YYYY-MM").
struct MonthLine {
    year: i32,
    month: u32,
    line: ScheduleLine,
}

/// Straight-line depreciation schedule from the purchase month through
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
///   1. The life is [purchase, purchase + life months), cut into calendar
///      months (calendar::month_parts). Through day k of a month but the
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
    if stop < asset.purchase_date {
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
    for p in crate::calendar::month_parts(asset.purchase_date, end) {
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

/// Accumulated depreciation through `as_of` (inclusive), exact to the
/// minor unit ([`depreciation_schedule`]).
pub fn accumulated_depreciation(asset: &Asset, as_of: NaiveDate) -> Result<Decimal, LedgerError> {
    Ok(depreciation_schedule(asset, as_of)?
        .last()
        .map(|l| l.accumulated)
        .unwrap_or(Decimal::ZERO))
}

/// Book value (cost − accumulated depreciation) as of a date.
pub fn book_value(asset: &Asset, as_of: NaiveDate) -> Result<Decimal, LedgerError> {
    asset
        .cost
        .amount
        .checked_sub(accumulated_depreciation(asset, as_of)?)
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
    let mut out = Vec::new();
    for &(run, amount) in runs {
        let chain = ledger.reversal_chain(run);
        let from = chain.iter().position(|&e| e == run).unwrap_or(0);
        for (i, e) in chain[from..].iter().enumerate() {
            let Some(entry) = ledger
                .get_entry(*e)
                .filter(|x| x.status() == EntryStatus::Posted)
            else {
                continue;
            };
            let d = entry.date();
            out.push(PostedRun {
                year: d.year(),
                month: d.month(),
                amount: if i % 2 == 0 { amount } else { -amount },
            });
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
    let first = (asset.purchase_date.year(), asset.purchase_date.month());
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
    if month_end_of(year, month)? < asset.purchase_date {
        return Err(LedgerError::InvalidDepreciationPeriod(format!(
            "{period} ends before asset {} was placed in service on {}",
            asset.id, asset.purchase_date
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

/// Whether an asset can be disposed of on `date` for `proceeds`: it is
/// active, the proceeds aren't negative, and the date isn't before it was
/// placed in service (a disposal on that date holds it one day, accrual
/// being inclusive, B-15). The checks only: no depreciation is computed,
/// so replaying a journaled disposal never depends on a figure (rules.md
/// B-28).
pub fn check_disposal(asset: &Asset, date: NaiveDate, proceeds: Money) -> Result<(), LedgerError> {
    if asset.status != AssetStatus::Active {
        return Err(LedgerError::AssetAlreadyDisposed(asset.id));
    }
    if proceeds.amount < Decimal::ZERO {
        return Err(LedgerError::NegativeProceeds(proceeds.amount));
    }
    if date < asset.purchase_date {
        return Err(LedgerError::DisposalBeforePurchase(date.to_string()));
    }
    Ok(())
}

/// Compute disposal amounts. Pure math; posting is the caller's job.
/// A disposal may fall on the purchase date (one day held, accrual being
/// inclusive, B-15 in docs/spec/rules.md); one before it is an error
/// ([`check_disposal`]). Accumulated depreciation is the schedule through
/// the disposal day, inclusive: what the schedule of the asset, once
/// disposed, shows ([`depreciation_schedule`] INVARIANT 4).
pub fn disposal_amounts(
    asset: &Asset,
    date: NaiveDate,
    proceeds: Money,
) -> Result<DisposalAmounts, LedgerError> {
    check_disposal(asset, date, proceeds)?;
    let accumulated = accumulated_depreciation(asset, date)?;
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
