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
//!      among the members as its book value is (by their property in it,
//!      to the cent, odd cents by largest remainder, ties to the earlier
//!      member), so a member's tax property = book property − their share
//!      of the built-in gain − their share of the property without a basis.
//!   3. A figure the books can't know is never guessed: its line carries
//!      `known`, the part the books have, and `open`, each missing item
//!      with the book amount it replaces. A line with no open item is the
//!      tax figure.
//!      - Net income: tax depreciation of every fixed asset in service
//!        in the span (MACRS, and for contributed property the §704(c)
//!        method's allocation) is the CPA's, as is the tax gain on a
//!        disposal; `known` is the book share, the open items say so.
//!      - Property distributed: its adjusted tax basis is the CPA's.
//!      - A change in book capital the books can't explain ("other").
//!      - Beginning capital from earlier years whose tax figures were open.
//!   4. Each row: ending.known = beginning.known + contributions_cash +
//!      contributions_property.known + net_income.known + other.known −
//!      distributions.known, exactly; ending.open is every open item of
//!      the row.
//!   5. Reconciliation: book ending − tax ending.known = (book beginning −
//!      tax beginning.known) + built-in gain share + property without a
//!      basis (book) − property distributed (book), exactly.
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
    /// adjusted tax basis) is the CPA's.
    Disposal {
        /// The asset.
        asset: AssetId,
        /// The proceeds.
        proceeds: Decimal,
    },
    /// A change in book capital the books can't explain (the statement of
    /// members' equity's "other"): its tax treatment is the CPA's.
    OtherChange {
        /// The book amount.
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
}

impl TaxLine {
    fn complete(known: Decimal) -> Self {
        Self {
            known,
            open: Vec::new(),
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
    // before, whose ending is this span's beginning.
    let first = ledger.posted_entries().map(|e| e.date()).min();
    let prior = match (first, start.pred_opt()) {
        (Some(first), Some(before)) if first <= before => {
            Some(span(ledger, first, before, members, cash, None)?)
        }
        _ => None,
    };
    span(ledger, start, end, members, cash, prior.as_ref())
}

fn too_large() -> LedgerError {
    LedgerError::TooLargeToCompute("a member's tax-basis capital".into())
}

/// `whole` shared in proportion to `weights` (non-negative), to the cent:
/// each part rounded toward zero, leftover cents by largest remainder,
/// ties to the earlier part; a negative whole is shared as its magnitude
/// and negated. All zero when the weights sum to zero.
fn share(whole: Decimal, weights: &[Decimal]) -> Result<Vec<Decimal>, LedgerError> {
    use rust_decimal::RoundingStrategy::ToZero;
    let total = weights
        .iter()
        .try_fold(Decimal::ZERO, |t, w| t.checked_add(*w))
        .ok_or_else(too_large)?;
    if whole.is_zero() || total <= Decimal::ZERO {
        return Ok(vec![Decimal::ZERO; weights.len()]);
    }
    let magnitude = whole.abs();
    let exact: Vec<Decimal> = weights
        .iter()
        .map(|w| magnitude.checked_mul(*w).and_then(|x| x.checked_div(total)))
        .collect::<Option<_>>()
        .ok_or_else(too_large)?;
    let mut parts: Vec<Decimal> = exact
        .iter()
        .map(|x| x.round_dp_with_strategy(2, ToZero))
        .collect();
    let remainders: Vec<Decimal> = exact.iter().zip(&parts).map(|(x, p)| x - p).collect();
    crate::kernel::apportion(magnitude, &mut parts, &remainders, Decimal::new(1, 2))
        .ok_or_else(too_large)?;
    if whole < Decimal::ZERO {
        for p in &mut parts {
            *p = -*p;
        }
    }
    Ok(parts)
}

/// What one contribution entry's property is on the tax basis.
struct EntryProperty {
    /// Σ built-in gain of the linked assets with a contribution basis.
    built_in_gain: Decimal,
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
            without_basis: pool,
            mismatch: Some((linked, pool)),
        });
    }
    let rest = pool - linked;
    Ok(EntryProperty {
        built_in_gain: big,
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
) -> Result<TaxCapital, LedgerError> {
    let detail = k1_detail(ledger, start, end, members, cash)?;
    let n = members.len();
    let mut big = vec![Decimal::ZERO; n];
    let mut without = vec![Decimal::ZERO; n];
    let mut distributed = vec![Decimal::ZERO; n];
    let mut prop_open: Vec<Vec<Open>> = vec![Vec::new(); n];
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
        let takes = lines(K1Kind::PropertyContribution);
        if takes.iter().any(|t| !t.is_zero()) {
            let p = entry_property(ledger, e, cash)?;
            let g = share(p.built_in_gain, &takes)?;
            let w = share(p.without_basis, &takes)?;
            for i in (0..n).filter(|&i| !takes[i].is_zero()) {
                big[i] = big[i].checked_add(g[i]).ok_or_else(too_large)?;
                without[i] = without[i].checked_add(w[i]).ok_or_else(too_large)?;
                if let Some((linked, property)) = p.mismatch {
                    prop_open[i].push(Open::LinkedValueExceedsEntry {
                        entry: id,
                        linked,
                        property,
                    });
                }
                if !w[i].is_zero() {
                    prop_open[i].push(Open::PropertyBasis {
                        entry: id,
                        book: w[i],
                    });
                }
            }
        }
        // Distributions are negative lines; weights are what each took.
        let taken: Vec<Decimal> = lines(K1Kind::Distribution)
            .into_iter()
            .map(|d| -d)
            .collect();
        if taken.iter().any(|t| *t > Decimal::ZERO) {
            let property = e
                .lines()
                .iter()
                .filter(|l| {
                    l.side == Side::Credit
                        && crate::k1::is_property_account(ledger, cash, l.account)
                })
                .try_fold(Decimal::ZERO, |t, l| t.checked_add(l.amount.amount))
                .ok_or_else(too_large)?;
            let total_taken = taken
                .iter()
                .try_fold(Decimal::ZERO, |t, x| t.checked_add(*x))
                .ok_or_else(too_large)?;
            let property = property.min(total_taken);
            if property > Decimal::ZERO {
                let parts = share(property, &taken)?;
                for i in (0..n).filter(|&i| !parts[i].is_zero()) {
                    distributed[i] = distributed[i].checked_add(parts[i]).ok_or_else(too_large)?;
                    dist_open[i].push(Open::PropertyDistribution {
                        entry: id,
                        book: parts[i],
                    });
                }
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
        };
        let net_income = TaxLine {
            known: book.net_income,
            open: open.clone(),
        };
        let other = TaxLine {
            known: book.other,
            open: if book.other.is_zero() {
                Vec::new()
            } else {
                vec![Open::OtherChange { book: book.other }]
            },
        };
        let distributions = TaxLine {
            known: book
                .distributions
                .checked_sub(distributed[i])
                .ok_or_else(too_large)?,
            open: std::mem::take(&mut dist_open[i]),
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

    /// share: proportional, to the cent, largest remainder ties to the
    /// first; negative wholes negated; zero weights give zeros.
    #[test]
    fn share_is_exact_and_signed() {
        assert_eq!(
            share(d("-1021.37"), &[d("525"), d("525")]).unwrap(),
            vec![d("-510.69"), d("-510.68")]
        );
        assert_eq!(
            share(d("25"), &[d("525"), d("525")]).unwrap(),
            vec![d("12.50"), d("12.50")]
        );
        assert_eq!(
            share(d("10"), &[d("0"), d("0")]).unwrap(),
            vec![d("0"), d("0")]
        );
        assert_eq!(
            share(d("0.01"), &[d("1"), d("2")]).unwrap(),
            vec![d("0.00"), d("0.01")]
        );
    }
}
