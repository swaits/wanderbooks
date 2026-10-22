//! Tax-basis capital: each member's capital account as Schedule K-1 Item L
//! reports it under the tax-basis method, beside the book figures of the
//! K-1 worksheet ([`crate::k1`]), with the reconciliation between them.
//!
//! ```text
//! GOAL: hand the CPA, per member, Item L on the tax basis as far as the
//!       books know it, every figure the books can't know named with what
//!       the books do have, and how book capital reconciles to it.
//! SOURCE: Instructions for Form 1065 (2025), Item L, "Tax-basis method"
//!         (docs/spec/sources/irs-partnerships.md): capital contributed is
//!         "the amount of cash plus the adjusted tax basis of all property
//!         contributed"; net income is the partner's share "as figured for
//!         tax purposes"; distributions are "the amount of cash plus the
//!         adjusted tax basis of all property distributed"; required since
//!         tax year 2020 (Instructions for Form 1065 (2020): "you must use
//!         the tax basis method this year"). IRC §723 (the basis of
//!         contributed property); Treas. Reg. §1.704-3(a)(3)(ii) (built-in
//!         gain); DECISION D-41 (owner, 2026-10-20).
//! INVARIANTS:
//!   1. The book column is the K-1 worksheet's row (k1::k1_detail),
//!      unchanged.
//!   2. Contributions: cash as on the book side; property at its §723
//!      basis. Within an entry, the property's built-in gain (its value at
//!      contribution less its §723 basis, Asset::basis_figures) and the
//!      book amount of property without a basis in the books are shared
//!      among the members as its book value is (by their property in it),
//!      to the cent by a running split from the books' first entry
//!      ([`RunningSplit`], D-38's rule: each member's running share is
//!      the largest-remainder rounding of their exact running share, so
//!      it is never a cent from it and tied odd cents alternate rather
//!      than all going to the earlier member), so a member's tax property
//!      = book property − their share of the built-in gain − their share
//!      of the property without a basis. Property distributed is shared
//!      among the members by what each took, the same running way. The
//!      running split is kept per set of members with a part in the entry,
//!      in date order whatever the span (`entry_parts`): a member never
//!      takes a cent of an entry they had no part in, and an entry's cents
//!      are the same in every span it is in.
//!   3. A figure the books can't know is never guessed: its line carries
//!      `known`, the part the books have, and `open`, each missing item
//!      with the book amount it replaces. A line with no open item is the
//!      tax figure.
//!      - Net income: always the return's (Item L: "as figured for tax
//!        purposes"); the books are kept on their own basis and can't
//!        know a book-tax difference (Schedule M-1). `known` is the book
//!        share; the open items name the differences the books do see:
//!        tax depreciation of every fixed asset in service in the span
//!        (MACRS, and for contributed property the §704(c) method's
//!        allocation), and the tax gain on a disposal, with the
//!        property's built-in gain or loss at contribution (§704(c)(1)(A)).
//!      - Property distributed: its adjusted tax basis is the CPA's.
//!      - A change in book capital the books can't explain ("other").
//!      - Beginning capital from earlier years whose tax figures were open.
//!   4. Each row: ending.known = beginning.known + contributions_cash +
//!      contributions_property.known + net_income.known + other.known −
//!      distributions.known, exactly; ending.open is every open item of
//!      the row, and so is always open (net income is).
//!   5. Reconciliation: book ending − tax ending.known = (book beginning −
//!      tax beginning.known) + built-in gain share + property without a
//!      basis (book) − property distributed (book), exactly.
//!   6. Property contributed to a prepaid account (a prepaid-expense
//!      schedule's balance account) with no fixed asset for it is taken
//!      at its book amount, with a note (Note::PrepaidAtBook), never open:
//!      a service contract's §723 basis is what the books recorded (the
//!      CPA to confirm).
//! SCENARIOS:
//!   1. IF two members jointly contribute a laptop worth 1,440.00 whose
//!      adjusted basis is 1,688.40, and nothing else happens:
//!      - SHOULD give each book property 720.00, tax property 844.20, a
//!        built-in gain share of −124.20, every line complete
//!      BECAUSE Item L counts property at its adjusted tax basis (§723).
//!   2. IF a fixed asset is in service during the span:
//!      - SHOULD leave the tax net income open, with the book depreciation
//!        posted in the span
//!      BECAUSE tax depreciation is the CPA's (rules.md B-15, B-25).
//!   3. IF nothing but cash comes in and goes out:
//!      - SHOULD still leave the tax net income open, the book share known
//!      BECAUSE income "as figured for tax purposes" is the return's.
//! ```

use crate::k1::{K1Kind, K1Row, k1_detail};
use crate::reports::MemberCapital;
use crate::{AccountId, AssetId, AssetStatus, EntryId, Ledger, LedgerError, Side};
use chrono::NaiveDate;
use rust_decimal::Decimal;

/// The depreciation an asset's disposal entry posts, if it was disposed in
/// `[start, end]`: its lines on the asset's depreciation expense account,
/// debit-positive (the depreciation still due through the disposal day,
/// or, negative, what the month's run posted past it, credited back:
/// rules.md B-15 rule 5). With the runs
/// (`Ledger::depreciation_posted_between`) it is the asset's depreciation
/// expense in the span, the P&L's and the cash flow's (B-14 rule 8).
/// The disposal entry is the posted entry dated the disposal day that
/// credits the asset's cost account by its cost (`Store::dispose_asset`
/// posts it so).
fn disposal_depreciation(
    ledger: &Ledger,
    a: &crate::Asset,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<Decimal, LedgerError> {
    let AssetStatus::Disposed { date, .. } = a.status else {
        return Ok(Decimal::ZERO);
    };
    if date < start || date > end {
        return Ok(Decimal::ZERO);
    }
    let closings = ledger.closing_entry_ids();
    let Some(e) = ledger.posted_entries().find(|e| {
        e.date() == date
            && !closings.contains(&e.id())
            && e.reversal_of().is_none()
            && e.lines().iter().any(|l| {
                l.side == Side::Credit
                    && l.account == a.cost_account
                    && l.amount.amount == a.cost.amount
            })
    }) else {
        return Ok(Decimal::ZERO);
    };
    e.lines()
        .iter()
        .filter(|l| l.account == a.expense_account)
        .try_fold(Decimal::ZERO, |t, l| {
            t.checked_add(match l.side {
                Side::Debit => l.amount.amount,
                Side::Credit => -l.amount.amount,
            })
        })
        .ok_or_else(|| LedgerError::TooLargeToCompute(format!("asset {}'s depreciation", a.id)))
}

/// A figure the books can't know, with what they do have.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Open {
    /// Beginning capital: earlier years left a tax figure open, so last
    /// year's Item L ending (on the tax basis) is the CPA's return's.
    PriorYears,
    /// Property contributed in `entry` with no tax basis in the books (no
    /// registered asset linked to the entry holds a contribution basis for
    /// it): this member's share of its book amount, which the tax figure
    /// replaces with the property's adjusted basis.
    PropertyBasis {
        /// The contribution entry.
        entry: EntryId,
        /// This member's share of that property at book.
        book: Decimal,
    },
    /// The assets linked to `entry` are valued at more than the property
    /// the entry contributed: the books disagree with themselves, so none
    /// of its property's tax basis is taken from them.
    LinkedValueExceedsEntry {
        /// The contribution entry.
        entry: EntryId,
        /// The linked assets' values at contribution.
        linked: Decimal,
        /// The property the entry debits.
        property: Decimal,
    },
    /// Property distributed in `entry`: this member's share of its book
    /// amount, which the tax figure replaces with the company's adjusted
    /// tax basis in it.
    PropertyDistribution {
        /// The distribution entry.
        entry: EntryId,
        /// This member's share of that property at book.
        book: Decimal,
    },
    /// Tax depreciation of the fixed assets in service in the span (MACRS,
    /// IRC §168; for contributed property allocated by its §704(c) method,
    /// Treas. Reg. §1.704-3): the CPA's. The book depreciation posted in
    /// the span, all members, and the part on contributed property.
    TaxDepreciation {
        /// Book depreciation posted in the span: the runs, and what
        /// disposals posted to depreciation expense (the P&L's).
        book: Decimal,
        /// The part of it on contributed property.
        contributed_book: Decimal,
    },
    /// A fixed asset disposed in the span: its tax gain or loss (from its
    /// adjusted tax basis) is the CPA's. For contributed property, the gain
    /// or loss is shared so as to take account of its built-in gain or loss
    /// (IRC §704(c)(1)(A)), which goes to the contributing member(s).
    Disposal {
        /// The asset.
        asset: AssetId,
        /// The proceeds.
        proceeds: Decimal,
        /// Its §704(c) built-in gain (+) or loss (−) at contribution, when
        /// it is contributed property with a basis recorded; `None` for a
        /// purchase or no basis.
        built_in_gain_704c: Option<Decimal>,
    },
    /// The member's share of net income (loss) as figured for tax purposes
    /// (Item L): the return's, never the books'. The books are kept on
    /// their own basis; every difference between book and tax income
    /// (Schedule M-1) is the CPA's. `book` is the member's book share.
    TaxNetIncome {
        /// The member's share of book net income (loss).
        book: Decimal,
    },
    /// A change in book capital the books can't explain (the statement of
    /// members' equity's "other"): its tax treatment is the CPA's.
    OtherChange {
        /// The book amount.
        book: Decimal,
    },
}

/// A figure the worksheet takes on a stated rule, for the CPA to confirm
/// (INVARIANT 6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Note {
    /// Property contributed in `entry` to a prepaid account (the balance
    /// account of a prepaid-expense schedule: a service contract the
    /// member handed over, B-13) with no fixed asset for it: its §723
    /// basis is taken as its book amount, this member's share `book`
    /// (DECISION, P3 review 2026-10-21: the contract's carryover basis is
    /// what the books recorded for it; the CPA to confirm).
    PrepaidAtBook {
        /// The contribution entry.
        entry: EntryId,
        /// This member's share of the prepaid, at book and at basis.
        book: Decimal,
    },
}

/// One Item L line on the tax basis ([`Open`]: INVARIANT 3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaxLine {
    /// The figure as far as the books know it.
    pub known: Decimal,
    /// What the CPA must still settle; empty when `known` is the tax
    /// figure.
    pub open: Vec<Open>,
    /// Figures taken on a stated rule, for the CPA to confirm (INVARIANT
    /// 6); they never make a line open.
    pub notes: Vec<Note>,
}

impl TaxLine {
    fn complete(known: Decimal) -> Self {
        Self {
            known,
            open: Vec::new(),
            notes: Vec::new(),
        }
    }

    /// The tax figure, when the books know all of it.
    pub fn tax(&self) -> Option<Decimal> {
        self.open.is_empty().then_some(self.known)
    }
}

/// Book capital less tax-basis capital, by cause (INVARIANT 5).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Reconciliation {
    /// Beginning capital, book less tax (as far as known).
    pub beginning: Decimal,
    /// This member's share of the §704(c) built-in gain (+) or loss (−) at
    /// contribution of the property they contributed in the span.
    pub built_in_gain_704c: Decimal,
    /// Their share of property contributed with no tax basis in the books,
    /// at book.
    pub property_without_basis: Decimal,
    /// Their share of property distributed, at book (it lowers book
    /// capital by its book amount, tax capital by its basis).
    pub property_distributed: Decimal,
}

impl Reconciliation {
    /// Book ending less tax ending (as far as known).
    pub fn total(&self) -> Option<Decimal> {
        self.beginning
            .checked_add(self.built_in_gain_704c)?
            .checked_add(self.property_without_basis)?
            .checked_sub(self.property_distributed)
    }
}

/// One member's capital, book beside tax basis.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaxCapitalRow {
    /// The K-1 worksheet's book row (INVARIANT 1).
    pub book: K1Row,
    /// Beginning capital on the tax basis.
    pub beginning: TaxLine,
    /// Cash contributed (the same on both bases).
    pub contributions_cash: Decimal,
    /// Property contributed, at its adjusted tax basis (§723).
    pub contributions_property: TaxLine,
    /// Share of net income (loss) as figured for tax purposes.
    pub net_income: TaxLine,
    /// Other increase (decrease).
    pub other: TaxLine,
    /// Withdrawals and distributions: cash plus the adjusted tax basis of
    /// property distributed.
    pub distributions: TaxLine,
    /// Ending capital on the tax basis (INVARIANT 4).
    pub ending: TaxLine,
    /// Book less tax, by cause (INVARIANT 5).
    pub reconciliation: Reconciliation,
}

/// The tax-basis capital worksheet for a span.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaxCapital {
    /// One row per member, in order.
    pub rows: Vec<TaxCapitalRow>,
    /// The items open for every member (tax depreciation, disposals).
    pub open: Vec<Open>,
}

/// Tax-basis capital for `start..=end` (INVARIANTS 1-5). `cash` are the
/// company's cash accounts, as for [`crate::k1::k1_worksheet`].
pub fn tax_basis_capital(
    ledger: &Ledger,
    start: NaiveDate,
    end: NaiveDate,
    members: &[MemberCapital],
    cash: &[AccountId],
) -> Result<TaxCapital, LedgerError> {
    // Beginning: the same worksheet from the books' first entry to the day
    // before, whose ending is this span's beginning. The odd cents of
    // every entry through `end`, whatever the span (INVARIANT 2).
    let parts = entry_parts(ledger, end, members, cash)?;
    let first = ledger.posted_entries().map(|e| e.date()).min();
    let prior = match (first, start.pred_opt()) {
        (Some(first), Some(before)) if first <= before => {
            Some(span(ledger, first, before, members, cash, None, &parts)?)
        }
        _ => None,
    };
    span(ledger, start, end, members, cash, prior.as_ref(), &parts)
}

fn too_large() -> LedgerError {
    LedgerError::TooLargeToCompute("a member's tax-basis capital".into())
}

/// A running split of amounts among members, one split at a time, each
/// by its own weights (B-36, the odd cent as D-38 settles it): each
/// member's running total is rounded afresh from their exact running
/// total, so it never drifts a cent from it, and tied cents don't pile up
/// on one member. The worksheet keeps one per set of members with weight
/// (`entry_parts`), so in its use every member of a split has weight.
///
/// ```text
/// GOAL: no member's share of the property's built-in gain, of the
///       property without a basis, of the prepaid at book or of property
///       distributed is a cent or more from their exact share, however
///       many entries it is summed over; three equal odd-cent properties
///       contributed 50/50 leave the members one cent apart, never three.
/// SOURCE: rules.md B-36 (shared within each entry as its book value is),
///         B-13 (cent rounding by largest remainder: DECISION, owner
///         2026-10-13); DECISION D-38 (owner, 2026-10-20: odd cents
///         everywhere alike, cumulative largest remainder), here for the
///         tax basis by the owner's ask, 2026-10-22 (kernel::
///         apportion_cumulative, the members closes' rule).
/// INVARIANTS:
///   1. The parts of each split sum to its whole exactly.
///   2. A member's exact running total is Σ whole × weight / Σ weights
///      over every split so far; their running total (Σ of their parts)
///      is the largest-remainder rounding to the cent of the exact running
///      totals (ties to the earlier member), so it is within a cent of
///      exact, strictly. A negative running whole is rounded as its
///      magnitude and negated; a running whole of zero is rounded by the
///      sign of the whole just split. So a run of losses mirrors the run
///      of gains with every whole negated, part for part.
///   3. A split can move a member who has no weight in it by a cent: the
///      true-up that keeps their running total one rounding. The
///      worksheet never gives one such a split: it keeps a running split
///      per set of members with weight.
///   4. The exact shares are Decimal quotients (28 significant digits),
///      so a running total of thirds can come out a hair under the cent
///      it truly is (399.93 as 399.9299…9): each exact running total is
///      taken to EXACT_PLACES (10) decimal places before it is rounded to
///      the cent, far finer than any remainder that decides a cent and far
///      coarser than the quotients' error. Should the floors still come
///      out a cent per member short, refuse (TooLargeToCompute), never
///      guess.
///   5. unsplit takes a split back out (an entry's reversal): the running
///      wholes and exact totals are as before it, and each member's
///      running total is less exactly the part that split gave them.
/// SCENARIOS:
///   1. IF −0.01 is split 50/50 three times:
///      - SHOULD give −0.01 / 0.00, then 0.00 / −0.01, then −0.01 / 0.00:
///        running −0.02 / −0.01
///      BECAUSE the exact running totals are −0.005, −0.01, −0.015 each.
///   2. IF the weights sum to zero, or the whole is zero:
///      - SHOULD give zeros, the running totals as they were.
///   3. IF 0.01 is split 1:1, then −0.01 1:0 (running 0.00, exact
///      −0.005 / 0.005):
///      - SHOULD give 0.01 / 0.00, then −0.02 / 0.01 (running −0.01 /
///        0.01, rounded as a loss), and the run with each whole negated
///        −0.01 / 0.00, then 0.02 / −0.01
///      BECAUSE a loss mirrors a gain (INVARIANT 2); the member with no
///      weight moves (INVARIANT 3), which the worksheet never asks.
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunningSplit {
    /// Each member's exact running total.
    exact: Vec<Decimal>,
    /// Each member's running total, to the cent.
    given: Vec<Decimal>,
    /// Σ wholes split so far.
    total: Decimal,
}

impl RunningSplit {
    /// A split among `n` members, nothing split yet.
    pub fn new(n: usize) -> Self {
        Self {
            exact: vec![Decimal::ZERO; n],
            given: vec![Decimal::ZERO; n],
            total: Decimal::ZERO,
        }
    }

    /// Each member's running total, to the cent (INVARIANT 2).
    pub fn running(&self) -> &[Decimal] {
        &self.given
    }

    /// Σ weights, or None when they are all zero (nothing to split by).
    fn weight_sum(&self, weights: &[Decimal]) -> Result<Option<Decimal>, LedgerError> {
        let n = self.given.len();
        if weights.len() != n {
            return Err(LedgerError::InvalidMembers(format!(
                "{} weights for {n} members",
                weights.len()
            )));
        }
        let sum = weights
            .iter()
            .try_fold(Decimal::ZERO, |t, w| t.checked_add(*w))
            .ok_or_else(too_large)?;
        Ok((sum > Decimal::ZERO).then_some(sum))
    }

    /// `whole × weight / sum`, the exact share of one member.
    fn share(whole: Decimal, weight: Decimal, sum: Decimal) -> Result<Decimal, LedgerError> {
        whole
            .checked_mul(weight)
            .and_then(|p| p.checked_div(sum))
            .ok_or_else(too_large)
    }

    /// Split `whole` (to the cent) by `weights` (non-negative, one per
    /// member): each member's part, so that their running total is the
    /// rounding of their exact running total (INVARIANTS 1-4).
    pub fn split(
        &mut self,
        whole: Decimal,
        weights: &[Decimal],
    ) -> Result<Vec<Decimal>, LedgerError> {
        use rust_decimal::RoundingStrategy::ToNegativeInfinity;
        let n = self.given.len();
        let Some(sum) = self.weight_sum(weights)? else {
            return Ok(vec![Decimal::ZERO; n]);
        };
        if whole.is_zero() {
            return Ok(vec![Decimal::ZERO; n]);
        }
        let mut exact = self.exact.clone();
        for (x, w) in exact.iter_mut().zip(weights) {
            *x = x
                .checked_add(Self::share(whole, *w, sum)?)
                .ok_or_else(too_large)?;
        }
        let total = self.total.checked_add(whole).ok_or_else(too_large)?;
        // A loss is rounded as its magnitude, then negated; at a running
        // whole of zero, by the sign of this whole (INVARIANT 2).
        let sign = if total < Decimal::ZERO || (total.is_zero() && whole < Decimal::ZERO) {
            Decimal::NEGATIVE_ONE
        } else {
            Decimal::ONE
        };
        // Each exact running total to EXACT_PLACES (INVARIANT 4), then
        // rounded down to the cent, what rounding took its remainder.
        let snapped: Vec<Decimal> = exact
            .iter()
            .map(|x| (x * sign).round_dp(EXACT_PLACES))
            .collect();
        let mut parts: Vec<Decimal> = snapped
            .iter()
            .map(|x| x.round_dp_with_strategy(2, ToNegativeInfinity))
            .collect();
        let remainders: Vec<Decimal> = snapped.iter().zip(&parts).map(|(x, p)| x - p).collect();
        let given: Vec<Decimal> = self.given.iter().map(|g| g * sign).collect();
        crate::kernel::apportion_cumulative(
            total * sign,
            &mut parts,
            &remainders,
            Decimal::new(1, 2),
            &given,
        )
        .ok_or_else(too_large)?;
        for (p, g) in parts.iter_mut().zip(self.given.iter_mut()) {
            *p *= sign;
            *g = g.checked_add(*p).ok_or_else(too_large)?;
        }
        self.exact = exact;
        self.total = total;
        Ok(parts)
    }

    /// Take back out a split [`Self::split`] gave `parts` for, from the
    /// same `whole` and `weights` (INVARIANT 5): what an entry's reversal
    /// does to the running split, so the pair leaves the later splits as
    /// they would be without it.
    pub fn unsplit(
        &mut self,
        whole: Decimal,
        weights: &[Decimal],
        parts: &[Decimal],
    ) -> Result<(), LedgerError> {
        let n = self.given.len();
        let Some(sum) = self.weight_sum(weights)? else {
            return Ok(());
        };
        if whole.is_zero() {
            return Ok(());
        }
        if parts.len() != n {
            return Err(LedgerError::InvalidMembers(format!(
                "{} parts for {n} members",
                parts.len()
            )));
        }
        let mut exact = self.exact.clone();
        let mut given = self.given.clone();
        for i in 0..n {
            exact[i] = exact[i]
                .checked_sub(Self::share(whole, weights[i], sum)?)
                .ok_or_else(too_large)?;
            given[i] = given[i].checked_sub(parts[i]).ok_or_else(too_large)?;
        }
        self.total = self.total.checked_sub(whole).ok_or_else(too_large)?;
        self.exact = exact;
        self.given = given;
        Ok(())
    }
}

/// The decimal places a member's exact running total is taken to before
/// it is rounded to the cent ([`RunningSplit`] INVARIANT 4).
const EXACT_PLACES: u32 = 10;

/// The four amounts an entry's members share (B-36): the property's
/// built-in gain, its property without a basis, its prepaid at book (each
/// by the members' property in it) and the property it distributes (by
/// what each took).
const BUILT_IN_GAIN: usize = 0;
const WITHOUT_BASIS: usize = 1;
const PREPAID: usize = 2;
const DISTRIBUTED: usize = 3;

/// One entry's parts of the four amounts, per member; an empty list is
/// zeros.
#[derive(Clone, Debug, Default)]
struct EntryParts([Vec<Decimal>; 4]);

impl EntryParts {
    fn get(&self, amount: usize, member: usize) -> Decimal {
        self.0[amount].get(member).copied().unwrap_or(Decimal::ZERO)
    }
}

/// One amount an entry splits: which, the whole, each member's weight.
type Whole = (usize, Decimal, Vec<Decimal>);

/// What one split put into a running split, to take it back out: the
/// amount, the members with weight, the whole, their weights and parts.
type Taken = (usize, Vec<usize>, Decimal, Vec<Decimal>, Vec<Decimal>);

/// The amounts `e` splits ([`Whole`]), from its K-1 lines.
fn entry_wholes(
    ledger: &Ledger,
    e: &crate::JournalEntry,
    lines: &[crate::k1::K1Line],
    n: usize,
    cash: &[AccountId],
) -> Result<Vec<Whole>, LedgerError> {
    let by_member = |kind| -> Vec<Decimal> {
        (0..n)
            .map(|i| {
                lines
                    .iter()
                    .filter(|l| l.member == i && l.kind == kind)
                    .map(|l| l.amount)
                    .sum()
            })
            .collect()
    };
    let mut out = Vec::new();
    let takes = by_member(K1Kind::PropertyContribution);
    if takes.iter().any(|t| !t.is_zero()) {
        let p = entry_property(ledger, e, cash)?;
        out.push((BUILT_IN_GAIN, p.built_in_gain, takes.clone()));
        out.push((WITHOUT_BASIS, p.without_basis, takes.clone()));
        out.push((PREPAID, p.prepaid, takes));
    }
    // Distributions are negative lines; weights are what each took.
    let taken: Vec<Decimal> = by_member(K1Kind::Distribution)
        .into_iter()
        .map(|d| -d)
        .collect();
    if taken.iter().any(|t| *t > Decimal::ZERO) {
        let property = e
            .lines()
            .iter()
            .filter(|l| {
                l.side == Side::Credit && crate::k1::is_property_account(ledger, cash, l.account)
            })
            .try_fold(Decimal::ZERO, |t, l| t.checked_add(l.amount.amount))
            .ok_or_else(too_large)?;
        let total_taken = taken
            .iter()
            .try_fold(Decimal::ZERO, |t, x| t.checked_add(*x))
            .ok_or_else(too_large)?;
        let property = property.min(total_taken);
        if property > Decimal::ZERO {
            out.push((DISTRIBUTED, property, taken));
        }
    }
    Ok(out)
}

/// Every entry's parts of the amounts it splits (B-36 with D-38), from
/// the books' first entry through `end`, whatever span the worksheet is
/// asked for, so an entry's cents are the same in every span it is in.
///
/// ```text
/// GOAL: odd cents that run (RunningSplit) and never touch a member who
///       has no part in the entry, nor change with the span asked for.
/// SOURCE: rules.md B-36; DECISION D-38; Form 1065 Item L (a partner's
///         capital contributed is the basis of the property that partner
///         contributed).
/// INVARIANTS:
///   1. The entries are taken in date order, ties in posting order,
///      every posted entry through `end` but the periods' closing
///      entries, with no regard to the span: the reversed ones too.
///   2. One running split per amount and per set of members with weight
///      in the entry (members with property in it, or who took): a split
///      never moves a member outside its set (RunningSplit INVARIANT 3
///      never arises), and a set's count never restarts, since its
///      weights are each entry's own amounts, not the members' shares.
///   3. An entry that reverses one already split takes that split back
///      out of its count (RunningSplit::unsplit), each member exactly
///      their part: the pair leaves every later entry's cents as they
///      would be without it. Its own parts (it reads as the contrary
///      kind, a distribution for a contribution) are split on its own,
///      largest remainder, ties to the earlier member.
/// ```
fn entry_parts(
    ledger: &Ledger,
    end: NaiveDate,
    members: &[MemberCapital],
    cash: &[AccountId],
) -> Result<std::collections::BTreeMap<EntryId, EntryParts>, LedgerError> {
    use std::collections::BTreeMap;
    let n = members.len();
    let places = ledger.base_currency().allocation_places();
    let closings = ledger.closing_entry_ids();
    let mut walk: Vec<&crate::JournalEntry> = ledger
        .posted_entries()
        .filter(|e| e.date() <= end && !closings.contains(&e.id()))
        .collect();
    walk.sort_by_key(|e| e.date());
    let mut counts: BTreeMap<(usize, Vec<usize>), RunningSplit> = BTreeMap::new();
    let mut taken_in: BTreeMap<EntryId, Vec<Taken>> = BTreeMap::new();
    let mut out = BTreeMap::new();
    for e in walk {
        let lines = crate::k1::entry_lines(ledger, e, members, cash, places)?;
        let wholes = entry_wholes(ledger, e, &lines, n, cash)?;
        let mut parts = EntryParts::default();
        if let Some(back) = e.reversal_of().and_then(|o| taken_in.remove(&o)) {
            for (amount, set, whole, weights, given) in back {
                counts
                    .get_mut(&(amount, set))
                    .expect("a split taken in has its count")
                    .unsplit(whole, &weights, &given)?;
            }
            for (amount, whole, weights) in wholes {
                parts.0[amount] = RunningSplit::new(n).split(whole, &weights)?;
            }
        } else {
            let mut taken = Vec::new();
            for (amount, whole, weights) in wholes {
                let set: Vec<usize> = (0..n).filter(|&i| !weights[i].is_zero()).collect();
                let sub: Vec<Decimal> = set.iter().map(|&i| weights[i]).collect();
                let sub_parts = counts
                    .entry((amount, set.clone()))
                    .or_insert_with(|| RunningSplit::new(set.len()))
                    .split(whole, &sub)?;
                let mut full = vec![Decimal::ZERO; n];
                for (k, &i) in set.iter().enumerate() {
                    full[i] = sub_parts[k];
                }
                parts.0[amount] = full;
                taken.push((amount, set, whole, sub, sub_parts));
            }
            if !taken.is_empty() {
                taken_in.insert(e.id(), taken);
            }
        }
        out.insert(e.id(), parts);
    }
    Ok(out)
}

/// What one contribution entry's property is on the tax basis.
struct EntryProperty {
    /// Σ built-in gain of the linked assets with a contribution basis.
    built_in_gain: Decimal,
    /// The property debited to prepaid accounts (INVARIANT 6), at book.
    prepaid: Decimal,
    /// The property's book amount with no basis in the books.
    without_basis: Decimal,
    /// The linked assets' values exceed the entry's property (linked,
    /// property).
    mismatch: Option<(Decimal, Decimal)>,
}

fn entry_property(
    ledger: &Ledger,
    entry: &crate::JournalEntry,
    cash: &[AccountId],
) -> Result<EntryProperty, LedgerError> {
    let pool = entry
        .lines()
        .iter()
        .filter(|l| {
            l.side == Side::Debit && crate::k1::is_property_account(ledger, cash, l.account)
        })
        .try_fold(Decimal::ZERO, |t, l| t.checked_add(l.amount.amount))
        .ok_or_else(too_large)?;
    // Prepaid accounts: those a prepaid-expense schedule amortizes, and no
    // registered asset carries its cost in (INVARIANT 6).
    let prepaid_account = |id: AccountId| {
        ledger.schedules().any(|s| {
            s.kind == crate::schedules::ScheduleKind::PrepaidExpense && s.balance_account == id
        }) && !ledger.registered_assets().any(|a| a.cost_account == id)
    };
    let prepaid = entry
        .lines()
        .iter()
        .filter(|l| {
            l.side == Side::Debit
                && crate::k1::is_property_account(ledger, cash, l.account)
                && prepaid_account(l.account)
        })
        .try_fold(Decimal::ZERO, |t, l| t.checked_add(l.amount.amount))
        .ok_or_else(too_large)?;
    let pool = pool - prepaid;
    let (mut linked, mut big, mut unknown) = (Decimal::ZERO, Decimal::ZERO, Decimal::ZERO);
    for a in ledger
        .registered_assets()
        .filter(|a| a.acquired_by == Some(entry.id()))
    {
        let figures = a.basis_figures()?;
        let contributed = figures.and_then(|f| f.built_in_gain_704c);
        // Its book amount in the entry: the value at contribution, which
        // the books recorded (D-41), or the book cost.
        let value = a.fmv_at_contribution().unwrap_or(a.cost.amount);
        linked = linked.checked_add(value).ok_or_else(too_large)?;
        match contributed {
            Some(g) => big = big.checked_add(g).ok_or_else(too_large)?,
            // No basis, or a purchase's basis on a contribution: unknown.
            None => unknown = unknown.checked_add(value).ok_or_else(too_large)?,
        }
    }
    if linked > pool {
        return Ok(EntryProperty {
            built_in_gain: Decimal::ZERO,
            prepaid,
            without_basis: pool,
            mismatch: Some((linked, pool)),
        });
    }
    let rest = pool - linked;
    Ok(EntryProperty {
        built_in_gain: big,
        prepaid,
        without_basis: unknown.checked_add(rest).ok_or_else(too_large)?,
        mismatch: None,
    })
}

fn span(
    ledger: &Ledger,
    start: NaiveDate,
    end: NaiveDate,
    members: &[MemberCapital],
    cash: &[AccountId],
    prior: Option<&TaxCapital>,
    parts: &std::collections::BTreeMap<EntryId, EntryParts>,
) -> Result<TaxCapital, LedgerError> {
    let detail = k1_detail(ledger, start, end, members, cash)?;
    let n = members.len();
    let mut big = vec![Decimal::ZERO; n];
    let mut without = vec![Decimal::ZERO; n];
    let mut distributed = vec![Decimal::ZERO; n];
    let mut prop_open: Vec<Vec<Open>> = vec![Vec::new(); n];
    let mut prop_notes: Vec<Vec<Note>> = vec![Vec::new(); n];
    let mut dist_open: Vec<Vec<Open>> = vec![Vec::new(); n];
    // Entries in journal order, each once.
    let mut entries: Vec<EntryId> = Vec::new();
    for l in &detail.lines {
        if !entries.contains(&l.entry) {
            entries.push(l.entry);
        }
    }
    for id in entries {
        let e = ledger.get_entry(id).ok_or(LedgerError::UnknownEntry(id))?;
        let lines = |kind| -> Vec<Decimal> {
            (0..n)
                .map(|i| {
                    detail
                        .lines
                        .iter()
                        .filter(|l| l.entry == id && l.member == i && l.kind == kind)
                        .map(|l| l.amount)
                        .sum()
                })
                .collect()
        };
        let none = EntryParts::default();
        let part = parts.get(&id).unwrap_or(&none);
        let takes = lines(K1Kind::PropertyContribution);
        if takes.iter().any(|t| !t.is_zero()) {
            let p = entry_property(ledger, e, cash)?;
            // Only the members with property in the entry have a part of
            // it (entry_parts INVARIANT 2).
            for i in (0..n).filter(|&i| !takes[i].is_zero()) {
                let (g, w, pp) = (
                    part.get(BUILT_IN_GAIN, i),
                    part.get(WITHOUT_BASIS, i),
                    part.get(PREPAID, i),
                );
                if !pp.is_zero() {
                    prop_notes[i].push(Note::PrepaidAtBook {
                        entry: id,
                        book: pp,
                    });
                }
                big[i] = big[i].checked_add(g).ok_or_else(too_large)?;
                without[i] = without[i].checked_add(w).ok_or_else(too_large)?;
                if let Some((linked, property)) = p.mismatch {
                    prop_open[i].push(Open::LinkedValueExceedsEntry {
                        entry: id,
                        linked,
                        property,
                    });
                }
                if !w.is_zero() {
                    prop_open[i].push(Open::PropertyBasis { entry: id, book: w });
                }
            }
        }
        // Property distributed, by what each took.
        for i in 0..n {
            let book = part.get(DISTRIBUTED, i);
            if !book.is_zero() {
                distributed[i] = distributed[i].checked_add(book).ok_or_else(too_large)?;
                dist_open[i].push(Open::PropertyDistribution { entry: id, book });
            }
        }
    }
    // Worksheet-level open items: tax depreciation while a fixed asset is
    // in service in the span; each disposal in it.
    let mut open = Vec::new();
    let (mut book_depr, mut contributed_depr) = (Decimal::ZERO, Decimal::ZERO);
    let mut in_service = false;
    for a in ledger.registered_assets() {
        let gone_before = matches!(a.status, AssetStatus::Disposed { date, .. } if date < start);
        if a.in_service_date <= end && !gone_before {
            in_service = true;
        }
        let d = ledger
            .depreciation_posted_between(a.id, start, end)?
            .checked_add(disposal_depreciation(ledger, a, start, end)?)
            .ok_or_else(too_large)?;
        book_depr = book_depr.checked_add(d).ok_or_else(too_large)?;
        if a.basis.as_ref().is_some_and(|b| b.rule.is_contribution()) {
            contributed_depr = contributed_depr.checked_add(d).ok_or_else(too_large)?;
        }
    }
    if in_service || !book_depr.is_zero() {
        open.push(Open::TaxDepreciation {
            book: book_depr,
            contributed_book: contributed_depr,
        });
    }
    for a in ledger.registered_assets() {
        if let AssetStatus::Disposed { date, proceeds } = &a.status
            && *date >= start
            && *date <= end
        {
            open.push(Open::Disposal {
                asset: a.id,
                proceeds: proceeds.amount,
                built_in_gain_704c: a.basis_figures()?.and_then(|f| f.built_in_gain_704c),
            });
        }
    }
    let mut rows = Vec::with_capacity(n);
    for (i, book) in detail.rows.into_iter().enumerate() {
        let beginning = match prior.and_then(|p| p.rows.get(i)) {
            Some(p) => TaxLine {
                known: p.ending.known,
                open: if p.ending.open.is_empty() {
                    Vec::new()
                } else {
                    vec![Open::PriorYears]
                },
                notes: Vec::new(),
            },
            None => TaxLine::complete(book.beginning),
        };
        let contributions_property = TaxLine {
            known: book
                .contributions_property
                .checked_sub(big[i])
                .and_then(|x| x.checked_sub(without[i]))
                .ok_or_else(too_large)?,
            open: std::mem::take(&mut prop_open[i]),
            notes: std::mem::take(&mut prop_notes[i]),
        };
        let net_income = TaxLine {
            known: book.net_income,
            open: std::iter::once(Open::TaxNetIncome {
                book: book.net_income,
            })
            .chain(open.iter().cloned())
            .collect(),
            notes: Vec::new(),
        };
        let other = TaxLine {
            known: book.other,
            open: if book.other.is_zero() {
                Vec::new()
            } else {
                vec![Open::OtherChange { book: book.other }]
            },
            notes: Vec::new(),
        };
        let distributions = TaxLine {
            known: book
                .distributions
                .checked_sub(distributed[i])
                .ok_or_else(too_large)?,
            open: std::mem::take(&mut dist_open[i]),
            notes: Vec::new(),
        };
        let known = beginning
            .known
            .checked_add(book.contributions_cash)
            .and_then(|x| x.checked_add(contributions_property.known))
            .and_then(|x| x.checked_add(net_income.known))
            .and_then(|x| x.checked_add(other.known))
            .and_then(|x| x.checked_sub(distributions.known))
            .ok_or_else(too_large)?;
        let ending = TaxLine {
            known,
            open: [
                &beginning,
                &contributions_property,
                &net_income,
                &other,
                &distributions,
            ]
            .iter()
            .flat_map(|l| l.open.iter().cloned())
            .collect(),
            notes: Vec::new(),
        };
        let reconciliation = Reconciliation {
            beginning: book
                .beginning
                .checked_sub(beginning.known)
                .ok_or_else(too_large)?,
            built_in_gain_704c: big[i],
            property_without_basis: without[i],
            property_distributed: distributed[i],
        };
        rows.push(TaxCapitalRow {
            contributions_cash: book.contributions_cash,
            book,
            beginning,
            contributions_property,
            net_income,
            other,
            distributions,
            ending,
            reconciliation,
        });
    }
    Ok(TaxCapital { rows, open })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn d(s: &str) -> Decimal {
        Decimal::from_str(s).unwrap()
    }

    fn split(r: &mut RunningSplit, whole: &str, weights: &[&str]) -> Vec<Decimal> {
        let w: Vec<Decimal> = weights.iter().map(|x| d(x)).collect();
        r.split(d(whole), &w).unwrap()
    }

    /// RunningSplit: one split is proportional, to the cent, largest
    /// remainder ties to the first; a negative whole mirrors a positive
    /// one; zero weights give zeros; weights that don't pair are refused.
    #[test]
    fn one_split_is_exact_and_signed() {
        let mut r = RunningSplit::new(2);
        assert_eq!(
            split(&mut r, "-1021.37", &["525", "525"]),
            vec![d("-510.69"), d("-510.68")]
        );
        let mut r = RunningSplit::new(2);
        assert_eq!(
            split(&mut r, "36", &["525", "525"]),
            vec![d("18.00"), d("18.00")]
        );
        let mut r = RunningSplit::new(2);
        assert_eq!(split(&mut r, "10", &["0", "0"]), vec![d("0"), d("0")]);
        assert_eq!(r.running(), &[d("0"), d("0")]);
        let mut r = RunningSplit::new(2);
        assert_eq!(
            split(&mut r, "0.01", &["1", "2"]),
            vec![d("0.00"), d("0.01")]
        );
        assert!(r.split(d("1"), &[d("1")]).is_err());
    }

    /// RunningSplit scenario 1: three odd cents 50/50 alternate, so the
    /// members end one cent apart, never three.
    #[test]
    fn tied_odd_cents_alternate() {
        let mut r = RunningSplit::new(2);
        assert_eq!(
            split(&mut r, "-0.01", &["1", "1"]),
            vec![d("-0.01"), d("0.00")]
        );
        assert_eq!(
            split(&mut r, "-0.01", &["1", "1"]),
            vec![d("0.00"), d("-0.01")]
        );
        assert_eq!(
            split(&mut r, "-0.01", &["1", "1"]),
            vec![d("-0.01"), d("0.00")]
        );
        assert_eq!(r.running(), &[d("-0.02"), d("-0.01")]);
    }

    /// RunningSplit INVARIANT 3: a member with no weight in a split takes
    /// back the cent their running total no longer earns.
    #[test]
    fn a_member_outside_a_split_is_trued_up() {
        let mut r = RunningSplit::new(3);
        // A and B 50/50 of 0.01: exact 0.005 each, the cent to A.
        assert_eq!(
            split(&mut r, "0.01", &["1", "1", "0"]),
            vec![d("0.01"), d("0"), d("0")]
        );
        // B and C 1:3 of 0.02: exact 0.005 / 0.01 / 0.015 of 0.03; A and
        // C tie on remainders, the odd cent stays with A.
        assert_eq!(
            split(&mut r, "0.02", &["0", "1", "3"]),
            vec![d("0"), d("0.01"), d("0.01")]
        );
        // B and C 2:1 of 0.01: exact 0.005 / 0.01667 / 0.01833 of 0.04;
        // C's and B's remainders now beat A's, so A gives the cent back.
        assert_eq!(
            split(&mut r, "0.01", &["0", "2", "1"]),
            vec![d("-0.01"), d("0.01"), d("0.01")]
        );
        assert_eq!(r.running(), &[d("0"), d("0.02"), d("0.02")]);
    }

    /// RunningSplit scenario 3, INVARIANT 2: at a running whole of zero
    /// the rounding takes the sign of the whole just split, so a run of
    /// losses mirrors its run of gains part for part (P4 review: rounded
    /// as a gain at zero, the negated run ended 0.01 / −0.01, moving the
    /// member with no weight).
    #[test]
    fn a_loss_mirrors_a_gain_at_zero() {
        let mut gain = RunningSplit::new(2);
        let mut loss = RunningSplit::new(2);
        for (whole, weights, want) in [
            ("0.01", ["1", "1"], ["0.01", "0"]),
            ("-0.01", ["1", "0"], ["-0.02", "0.01"]),
        ] {
            let g = split(&mut gain, whole, &weights);
            let l = split(&mut loss, &format!("{}", -d(whole)), &weights);
            assert_eq!(g, vec![d(want[0]), d(want[1])]);
            assert_eq!(l, g.iter().map(|x| -x).collect::<Vec<_>>());
        }
        assert_eq!(gain.running(), &[d("-0.01"), d("0.01")]);
        assert_eq!(loss.running(), &[d("0.01"), d("-0.01")]);
    }

    /// RunningSplit INVARIANT 5: a split taken back out leaves the next
    /// split as if it never was.
    #[test]
    fn unsplit_takes_a_split_back_out() {
        let mut r = RunningSplit::new(2);
        let parts = split(&mut r, "-12.37", &["1", "1"]);
        assert_eq!(parts, vec![d("-6.19"), d("-6.18")]);
        r.unsplit(d("-12.37"), &[d("1"), d("1")], &parts).unwrap();
        assert_eq!(r, RunningSplit::new(2));
        assert_eq!(
            split(&mut r, "-55.91", &["1", "1"]),
            vec![d("-27.96"), d("-27.95")]
        );
        assert!(r.unsplit(d("1"), &[d("1")], &parts).is_err());
        assert!(r.unsplit(d("1"), &[d("1"), d("1")], &[d("1")]).is_err());
    }

    /// RunningSplit INVARIANT 4: thirds summed in Decimal come out a hair
    /// under the cents they are (399.93 and 791.11); taken to ten places
    /// they are those cents, and the split goes through.
    #[test]
    fn thirds_summed_are_the_cents_they_are() {
        let mut r = RunningSplit::new(2);
        split(&mut r, "283.43", &["4", "2"]);
        split(&mut r, "274.68", &["0", "1"]);
        split(&mut r, "824.32", &["2", "4"]);
        split(&mut r, "-191.39", &["1", "2"]);
        assert_eq!(r.running(), &[d("399.93"), d("791.11")]);
    }
}
