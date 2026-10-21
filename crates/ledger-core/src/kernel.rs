//! The accounting kernel: the arithmetic every bookkeeping invariant rests
//! on, in one place.
//!
//! Each function here is small, pure, and generic over [`Amount`]. The
//! engine runs it on [`Decimal`]; the Kani harnesses in `proofs.rs` run the
//! very same code on machine integers and prove it correct for every input
//! within their bounds; and the Lean model in `verify/lean` proves the
//! laws it implements hold for books of any size. See
//! `docs/verification.md` for which proof covers which law.
//!
//! The laws, in the order they appear (each traced to its authority in
//! `docs/spec/rules.md`):
//!
//! * a line's signed amount is `+amount` for a debit, `-amount` for a credit
//!   (B-2);
//! * an entry is acceptable when it has two or more lines, every amount is
//!   positive and in range, and its debits equal its credits exactly (B-1,
//!   B-2, B-21);
//! * an acceptable entry nets to zero, account by account summed (B-1);
//! * a reversal flips every side, so it is acceptable exactly when the
//!   original is, and the two together net every account to zero (B-9);
//! * an account's balance is its movement read on its normal side, so the
//!   balances satisfy the accounting equation whenever movements net to
//!   zero (B-5, B-7);
//! * a trial balance puts each movement in exactly one column, so its
//!   columns differ by the net of all movements (B-6);
//! * a closing line zeroes the account it closes (B-11);
//! * a payment to a vendor splits, line by line, into what counts toward
//!   its 1099, what a card or payment network paid, and what is unknown,
//!   and money moved between the company's own sources nets out (B-19);
//! * an allocation's parts sum to the whole, to the last unit, by largest
//!   remainder (B-13);
//! * a cumulative allocation hands out exactly the new amount, and leaves
//!   each running total one allocation of everything allocated, so it never
//!   drifts a unit from its exact share (B-13);
//! * an entry's cash is applied to the sections of the statement of cash
//!   flows moving against it, in a fixed order, each up to its own
//!   movement: the parts sum to the entry's cash, what no cash settled sums
//!   to zero, and a reversal's parts are the original's negated (B-14);
//! * what a member or lender paid or received for the company, with no
//!   cash account moving, is split among the kinds it settled, netting to
//!   zero, and mirrored by a reversal (B-14 rule 11);
//! * a party's part of an entry is signed as a line on the party's own
//!   account would be, a reversal's is its original's negated, and what of
//!   it counts as money or as property never exceeds it (B-35);
//! * a span prorated by days (depreciation, B-15; daily schedules, B-16)
//!   splits into parts that sum to its total, each but the last its days
//!   times the monthly rate within one unit, the last the remainder;
//! * a catch-up run brings the books to the schedule without ever carrying
//!   more than it says at a month end already posted (B-15), and a
//!   disposal trues what the books carry up or down to the schedule;
//! * an asset's basis figures follow from its basis rule and facts: the
//!   §723 basis is the contributor's adjusted basis, the depreciable basis
//!   the cost, that basis, or the lesser of it and fair market value, and
//!   the §704(c) built-in gain is fair market value less the §723 basis
//!   (B-25).

use crate::assets::BasisRule;
use crate::{AccountType, Side};
use rust_decimal::Decimal;

/// What the kernel needs of a money amount: exact, ordered, and checked
/// arithmetic that reports overflow instead of rounding or wrapping.
pub trait Amount: Copy + Ord {
    /// Zero.
    const ZERO: Self;
    /// `self + rhs`, or `None` when it cannot be represented exactly.
    fn checked_add(self, rhs: Self) -> Option<Self>;
    /// `-self`, or `None` when it cannot be represented.
    fn checked_neg(self) -> Option<Self>;
    /// True when the amount is small and precise enough for a journal line
    /// (see [`MAX_LINE_AMOUNT`] and [`MAX_LINE_SCALE`] for [`Decimal`]).
    fn in_line_range(self) -> bool;

    /// `self - rhs`, or `None` when it cannot be represented exactly.
    fn checked_sub(self, rhs: Self) -> Option<Self> {
        self.checked_add(rhs.checked_neg()?)
    }
}

/// The largest amount one journal line may carry: one quadrillion units of
/// the base currency. Far beyond any real transaction, and far enough
/// below [`Decimal::MAX`] that no realistic sum of lines can overflow.
pub const MAX_LINE_AMOUNT: Decimal = Decimal::from_parts(0xA4C6_8000, 0x0003_8D7E, 0, false, 0);

/// The most decimal places a journal line may carry. Cents need two;
/// foreign-currency lines (amount times rate) need more; ten keeps every
/// sum of lines exact as long as the total stays under 7.9 quintillion.
pub const MAX_LINE_SCALE: u32 = 10;

impl Amount for Decimal {
    const ZERO: Self = Decimal::ZERO;
    fn checked_add(self, rhs: Self) -> Option<Self> {
        Decimal::checked_add(self, rhs)
    }
    fn checked_neg(self) -> Option<Self> {
        Some(-self)
    }
    fn in_line_range(self) -> bool {
        self.abs() <= MAX_LINE_AMOUNT && self.normalize().scale() <= MAX_LINE_SCALE
    }
}

impl Amount for i64 {
    const ZERO: Self = 0;
    fn checked_add(self, rhs: Self) -> Option<Self> {
        i64::checked_add(self, rhs)
    }
    fn checked_neg(self) -> Option<Self> {
        i64::checked_neg(self)
    }
    fn in_line_range(self) -> bool {
        // Scaled down with the same headroom as Decimal's range: lines
        // this size can be summed many times over without overflow.
        self.unsigned_abs() <= (1 << 40)
    }
}

/// Why the kernel rejected an entry's lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineError<T> {
    /// Fewer than two lines.
    TooFewLines,
    /// A line with a zero amount.
    ZeroAmount,
    /// A line with a negative amount: direction is the side's job.
    NegativeAmount,
    /// A line too large or too precise for a journal line.
    OutOfRange,
    /// Debits and credits differ.
    Unbalanced {
        /// Sum of debit lines.
        debits: T,
        /// Sum of credit lines.
        credits: T,
    },
}

/// The other side.
pub fn flip(side: Side) -> Side {
    match side {
        Side::Debit => Side::Credit,
        Side::Credit => Side::Debit,
    }
}

/// A line's signed amount: positive for a debit, negative for a credit.
pub fn signed<T: Amount>(side: Side, amount: T) -> Option<T> {
    match side {
        Side::Debit => Some(amount),
        Side::Credit => amount.checked_neg(),
    }
}

/// Total debits and total credits of some lines.
pub fn totals<T: Amount>(lines: impl IntoIterator<Item = (Side, T)>) -> Option<(T, T)> {
    let mut debits = T::ZERO;
    let mut credits = T::ZERO;
    for (side, amount) in lines {
        match side {
            Side::Debit => debits = debits.checked_add(amount)?,
            Side::Credit => credits = credits.checked_add(amount)?,
        }
    }
    Some((debits, credits))
}

/// The net of some lines: debits minus credits.
pub fn net<T: Amount>(lines: impl IntoIterator<Item = (Side, T)>) -> Option<T> {
    let mut total = T::ZERO;
    for (side, amount) in lines {
        total = total.checked_add(signed(side, amount)?)?;
    }
    Some(total)
}

/// Check an entry's lines: two or more, every amount positive and within
/// line range, debits equal to credits. Accounts and currencies are the
/// caller's to check; this is the arithmetic.
pub fn check_lines<T: Amount>(lines: &[(Side, T)]) -> Result<(), LineError<T>> {
    if lines.len() < 2 {
        return Err(LineError::TooFewLines);
    }
    for &(_, amount) in lines {
        if amount == T::ZERO {
            return Err(LineError::ZeroAmount);
        }
        if amount < T::ZERO {
            return Err(LineError::NegativeAmount);
        }
        if !amount.in_line_range() {
            return Err(LineError::OutOfRange);
        }
    }
    let (debits, credits) = totals(lines.iter().copied()).ok_or(LineError::OutOfRange)?;
    if debits != credits {
        return Err(LineError::Unbalanced { debits, credits });
    }
    Ok(())
}

/// Net movement per account (debits minus credits), in order of first
/// appearance.
pub fn movements<K: Copy + Eq, T: Amount>(
    lines: impl IntoIterator<Item = (K, Side, T)>,
) -> Option<Vec<(K, T)>> {
    let lines: Vec<_> = lines.into_iter().collect();
    let Some(&(first, _, _)) = lines.first() else {
        return Some(Vec::new());
    };
    // At most one account per line.
    let mut out = vec![(first, T::ZERO); lines.len()];
    let n = movements_into(&lines, &mut out)?;
    out.truncate(n);
    Some(out)
}

/// [`movements`] into a caller's buffer, without allocating: returns how
/// many accounts it wrote, or `None` on overflow or if `out` is too small.
pub fn movements_into<K: Copy + Eq, T: Amount>(
    lines: &[(K, Side, T)],
    out: &mut [(K, T)],
) -> Option<usize> {
    let mut n = 0;
    for &(account, side, amount) in lines {
        let delta = signed(side, amount)?;
        match out[..n].iter_mut().find(|(k, _)| *k == account) {
            Some((_, total)) => *total = total.checked_add(delta)?,
            None => {
                *out.get_mut(n)? = (account, delta);
                n += 1;
            }
        }
    }
    Some(n)
}

/// The lines of a reversing entry: every side flipped, accounts and
/// amounts unchanged.
pub fn reversal<K: Copy, T: Amount>(lines: &[(K, Side, T)]) -> Vec<(K, Side, T)> {
    lines.iter().map(|&line| reverse_line(line)).collect()
}

/// One line of a reversing entry: the side flipped, account and amount
/// unchanged.
pub fn reverse_line<K, T: Amount>((account, side, amount): (K, Side, T)) -> (K, Side, T) {
    (account, flip(side), amount)
}

/// An account's balance read on its normal side: positive is a normal
/// balance, negative a contra balance.
pub fn normal_balance<T: Amount>(account_type: AccountType, movement: T) -> Option<T> {
    match account_type.normal_side() {
        Side::Debit => Some(movement),
        Side::Credit => movement.checked_neg(),
    }
}

/// A movement in trial-balance columns: `(debit, credit)`, one of them
/// zero, both non-negative.
pub fn columns<T: Amount>(movement: T) -> Option<(T, T)> {
    if movement > T::ZERO {
        Some((movement, T::ZERO))
    } else if movement == T::ZERO {
        // Never a negative zero: money never prints "-0".
        Some((T::ZERO, T::ZERO))
    } else {
        Some((T::ZERO, movement.checked_neg()?))
    }
}

/// The line that zeroes an account with this movement: the opposite side,
/// for the same amount. `None` for a zero movement (nothing to close).
pub fn closing_line<T: Amount>(movement: T) -> Option<(Side, T)> {
    if movement > T::ZERO {
        Some((Side::Credit, movement))
    } else if movement < T::ZERO {
        Some((Side::Debit, movement.checked_neg()?))
    } else {
        None
    }
}

/// The line that credits `credit` to an account (a negative credit is a
/// debit). `None` for zero. This is [`closing_line`]: crediting `c` and
/// zeroing a movement of `c` are the same line.
pub fn credit_line<T: Amount>(credit: T) -> Option<(Side, T)> {
    closing_line(credit)
}

// ---------------------------------------------------------------------------
// 1099 payments: where each payment line goes (rules.md B-19)
// ---------------------------------------------------------------------------

/// A line's role in a payment to a vendor, from its account.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PayRole {
    /// One of the books' cash accounts: bank, cash on hand.
    Cash,
    /// One of the books' financing liabilities (what members or lenders
    /// paid on the company's behalf), or an account a member's card or
    /// cash, or a payment platform, pays through: how a payment through
    /// it was made, only the entry's method says.
    Financing,
    /// The account of a company card (a business card's liability).
    CardAccount,
    /// None of these (an expense, payables, prepaid): not a payment line.
    Other,
}

/// What an entry's payment-method tag says about one of its lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tag {
    /// The entry has no payment method recorded.
    None,
    /// The method pays through this line's account and is a card or a
    /// payment network.
    ThisCard,
    /// The method pays through this line's account and is neither (the
    /// company's bank, a member's own cash or check).
    ThisOther,
    /// The method pays through another account: it doesn't describe this
    /// line.
    Elsewhere,
}

/// Where a payment line's amount goes on the 1099 worksheet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bucket {
    /// Counts toward the payee's 1099.
    Counts,
    /// Paid by card or payment network: Form 1099-K's, never 1099-NEC or
    /// 1099-MISC's.
    Card,
    /// Can't be classified until the payment method is recorded.
    Undetermined,
}

impl Bucket {
    /// Every bucket, in [`split_payment`]'s order.
    pub const ALL: [Bucket; 3] = [Bucket::Counts, Bucket::Card, Bucket::Undetermined];

    /// Its place in [`split_payment`]'s array.
    pub const fn index(self) -> usize {
        match self {
            Bucket::Counts => 0,
            Bucket::Card => 1,
            Bucket::Undetermined => 2,
        }
    }
}

/// The bucket a payment line goes in, from its role and the entry's tag.
///
/// ```text
/// GOAL: every payment line to a vendor counts toward its 1099, is a card
///       or payment-network payment (Form 1099-K), or is unknown; never
///       guessed.
/// SOURCE: docs/spec/rules.md B-19 (Treas. Reg. §1.6041-1(a)(1)(iv): a
///         transaction reportable under §6050W is not reported under
///         §6041; §1.6041-1(e)(1): a member writing a check at the
///         company's direction doesn't make the member the payor;
///         sources/irs-1099-payment-method.md); DECISION D-40 (owner,
///         2026-10-20: a bank payment with no method recorded counts).
/// INVARIANTS (first match wins):
///   1. The tag names this line's account: Card for a card or network,
///      else Counts.
///   2. Otherwise the role decides: Cash counts (presumed check, ACH or
///      transfer, D-40); Financing is Undetermined (a member's card or
///      their check, unknown until tagged); CardAccount is Card; Other is
///      no payment line (None).
/// ```
pub fn payment_bucket(role: PayRole, tag: Tag) -> Option<Bucket> {
    match (tag, role) {
        (Tag::ThisCard, _) => Some(Bucket::Card),
        (Tag::ThisOther, _) => Some(Bucket::Counts),
        (Tag::None | Tag::Elsewhere, PayRole::Cash) => Some(Bucket::Counts),
        (Tag::None | Tag::Elsewhere, PayRole::Financing) => Some(Bucket::Undetermined),
        (Tag::None | Tag::Elsewhere, PayRole::CardAccount) => Some(Bucket::Card),
        (Tag::None | Tag::Elsewhere, PayRole::Other) => None,
    }
}

/// What an entry paid, per bucket: credits minus debits of the lines in
/// each (a line in no bucket is not a payment line). `None` on overflow.
pub fn split_payment<T: Amount>(lines: &[(Option<Bucket>, Side, T)]) -> Option<[T; 3]> {
    let mut out = [T::ZERO; 3];
    for &(bucket, side, amount) in lines {
        if let Some(b) = bucket {
            let i = b.index();
            out[i] = match side {
                Side::Credit => out[i].checked_add(amount)?,
                Side::Debit => out[i].checked_sub(amount)?,
            };
        }
    }
    Some(out)
}

/// Net one entry's buckets ([`split_payment`]): money moved between the
/// company's own payment sources pays no one.
///
/// ```text
/// GOAL: a card bill paid from the bank, a member reimbursed, or a loan
///       paid down never counts toward a vendor's 1099.
/// SOURCE: docs/spec/rules.md B-19 (26 U.S.C. §6041(a): payments made to
///         the payee; P3 review R1, 2026-10-20).
/// INVARIANTS:
///   1. The buckets' sum P is kept exactly.
///   2. No bucket ends opposite in sign to P.
///   3. Buckets all of one sign (zeros aside) come back unchanged.
/// SCENARIOS:
///   1. IF the signs differ and P = 0 (Dr 2050 / Cr 1000, the card bill):
///      SHOULD be [0, 0, 0].
///   2. IF the signs differ and exactly one bucket has P's sign: SHOULD
///      give it P and the others 0.
///   3. IF two buckets have P's sign: SHOULD put P in Undetermined
///      BECAUSE which source the other side offsets is unknown (split
///      the entry).
/// ```
pub fn net_buckets<T: Amount>(b: [T; 3]) -> Option<[T; 3]> {
    let positive = b.iter().any(|&x| x > T::ZERO);
    let negative = b.iter().any(|&x| x < T::ZERO);
    if !(positive && negative) {
        return Some(b);
    }
    let p = b[0].checked_add(b[1])?.checked_add(b[2])?;
    let mut out = [T::ZERO; 3];
    if p == T::ZERO {
        return Some(out);
    }
    let mut taker = Bucket::Undetermined.index();
    let mut takers = 0;
    for (i, &x) in b.iter().enumerate() {
        if x != T::ZERO && (x > T::ZERO) == (p > T::ZERO) {
            taker = i;
            takers += 1;
        }
    }
    if takers != 1 {
        taker = Bucket::Undetermined.index();
    }
    out[taker] = p;
    Some(out)
}

/// Make rounded parts sum to the whole: the last part becomes `whole`
/// minus the others, absorbing every unit of rounding. Returns `None` on
/// overflow, leaving `parts` unchanged.
pub fn plug_last<T: Amount>(whole: T, parts: &mut [T]) -> Option<()> {
    let Some((last, rest)) = parts.split_last_mut() else {
        return Some(());
    };
    let mut others = T::ZERO;
    for &p in rest.iter() {
        others = others.checked_add(p)?;
    }
    *last = whole.checked_sub(others)?;
    Some(())
}

/// Largest-remainder apportionment: make rounded-down `parts` sum to
/// `whole`, handing out the shortfall one `unit` at a time, largest
/// `remainders` first (ties to the earlier part). Any final fraction of a
/// unit (when `whole` isn't a whole number of units) goes to the first
/// part in that order.
///
/// ```text
/// GOAL: split a whole into parts, each a whole number of units where
///       possible, summing to the whole exactly, without favoring anyone.
/// SOURCE: docs/spec/rules.md B-13 (shares: 26 U.S.C. §704(a); cent
///         rounding: DECISION, owner 2026-10-13), B-21 (exact totals).
/// INVARIANTS:
///   1. MUST make the parts sum to `whole` exactly.
///   2. MUST change each part by zero or one unit (the first in order
///      may also take the sub-unit fraction).
///   3. NEVER gives a unit to a part while a part with a strictly larger
///      remainder goes without.
/// SCENARIOS:
///   1. IF the shortfall is negative, or `n` units or more (the parts
///      weren't rounded down from shares of `whole`):
///      - SHOULD return None, leaving `parts` unchanged
///      BECAUSE apportioning only corrects rounding.
///   2. IF $100.00 splits three equal ways:
///      - SHOULD be 33.34, 33.33, 33.33
///      - NEVER the same partner absorbing every odd cent by position
///      BECAUSE the cent goes to the largest remainder, ties to the first
///      (owner, 2026-10-13).
/// ```
///
/// Allocation-free and O(n^2): it ranks each part by counting the parts
/// ahead of it, so the model checker can prove it outright.
pub fn apportion<T: Amount>(whole: T, parts: &mut [T], remainders: &[T], unit: T) -> Option<()> {
    if parts.len() != remainders.len() || unit <= T::ZERO {
        return None;
    }
    if parts.is_empty() {
        // Nothing to give the whole to.
        return (whole == T::ZERO).then_some(());
    }
    let mut shortfall = whole;
    for &p in parts.iter() {
        shortfall = shortfall.checked_sub(p)?;
    }
    if shortfall < T::ZERO {
        return None;
    }
    // Whole units short, by subtraction: no division in `Amount`. A
    // shortfall of a unit per part or more isn't rounding; refuse it.
    let mut units = 0;
    while shortfall >= unit {
        shortfall = shortfall.checked_sub(unit)?;
        units += 1;
        if units == parts.len() {
            return None;
        }
    }
    // A part's rank: the parts ahead of it (larger remainder, or equal and
    // earlier).
    let rank = |i: usize| {
        (0..remainders.len())
            .filter(|&j| remainders[j] > remainders[i] || (remainders[j] == remainders[i] && j < i))
            .count()
    };
    for (i, part) in parts.iter_mut().enumerate() {
        let r = rank(i);
        let mut add = T::ZERO;
        if r < units {
            add = unit;
        }
        if r == 0 {
            add = add.checked_add(shortfall)?;
        }
        *part = part.checked_add(add)?;
    }
    Some(())
}

/// Cumulative apportionment: what one close gives each part when the
/// running `total` is apportioned afresh ([`apportion`]) and what earlier
/// closes gave each part (`given`) is taken back. `parts` and
/// `remainders` are the rounded-down shares of `total` and what rounding
/// took from each, as for [`apportion`].
///
/// ```text
/// GOAL: members' running totals that never drift a unit from their
///       exact shares, whatever earlier closes did.
/// SOURCE: docs/spec/rules.md B-13 (shares: 26 U.S.C. §704(a); capital
///         accounts carry the allocations: Treas. Reg.
///         §1.704-1(b)(2)(iv)(b); cent rounding: DECISION, owner
///         2026-10-13; counted from the first close: D-10, 2026-10-16).
/// INVARIANTS:
///   1. MUST make the parts sum to `total − Σ given` exactly.
///   2. MUST leave given[i] + parts[i] equal to what `apportion(total, …)`
///      gives part i: a running total is one apportionment, so it carries
///      one apportionment's rounding, never a sum of them.
///   3. On None, `parts` MUST be unchanged.
/// SCENARIOS:
///   1. IF the lengths differ, the unit isn't positive, `total`, the
///      unit, a part or a `given` is out of line range, or apportion
///      refuses: SHOULD return None, leaving `parts` unchanged
///      BECAUSE in range, the subtraction can't overflow, so a refusal is
///      known before anything changes.
///   2. IF 0.03 was closed as A 0.03 / B 0 and the running total is now
///      0.04 (50/50, floors 0.02 / 0.02): SHOULD give A −0.01, B +0.02
///      BECAUSE 0.04 apportions 0.02 each.
/// ```
///
/// Allocation-free, with no bulk copy (docs/verification.md, "Trusting
/// the checker"), so the model checker can prove it outright.
pub fn apportion_cumulative<T: Amount>(
    total: T,
    parts: &mut [T],
    remainders: &[T],
    unit: T,
    given: &[T],
) -> Option<()> {
    if parts.len() != given.len() || !total.in_line_range() || !unit.in_line_range() {
        return None;
    }
    if parts.iter().chain(given).any(|a| !a.in_line_range()) {
        return None;
    }
    apportion(total, parts, remainders, unit)?;
    // Each part is now under three line ranges and each `given` within
    // one: the subtraction can't fail (Kani apportion_cumulative_is_exact).
    for (part, &g) in parts.iter_mut().zip(given) {
        *part = part.checked_sub(g)?;
    }
    Some(())
}

/// Where a line falls on the statement of cash flows (rules.md B-14 rule
/// 2): a cash account, or one of the three sections.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Flow {
    /// A cash account: what the statement explains the change in.
    Cash,
    /// Operating activities.
    Operating,
    /// Investing activities.
    Investing,
    /// Financing activities.
    Financing,
}

impl Flow {
    /// The three sections, in the order cash is applied to them (rules.md
    /// B-14 rule 3): operating, investing, financing.
    pub const SECTIONS: [Flow; 3] = [Flow::Operating, Flow::Investing, Flow::Financing];
}

/// One entry's effect on the statement of cash flows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EntryFlows<T> {
    /// Net movement of the entry's cash lines, debit-positive: + is cash
    /// in.
    pub cash: T,
    /// Cash per section, `[operating, investing, financing]`, + is in.
    /// Sums to `cash`.
    pub flows: [T; 3],
    /// What no cash settled, per section, debit-positive: the section's
    /// movement plus its cash. Sums to zero.
    pub noncash: [T; 3],
}

/// The cash one section takes from what is left of an entry's cash
/// (`remaining`), given the section's net movement (debit-positive): only
/// a section moving against the cash takes any, and never more than its
/// own movement. Cash in (`remaining > 0`) settles credits (`movement <
/// 0`); cash out settles debits.
pub fn take<T: Amount>(remaining: T, movement: T) -> Option<T> {
    let against = movement.checked_neg()?;
    Some(if remaining > T::ZERO && movement < T::ZERO {
        remaining.min(against)
    } else if remaining < T::ZERO && movement > T::ZERO {
        remaining.max(against)
    } else {
        T::ZERO
    })
}

/// Apply `cash` to `movements` in order, each taking [`take`] of what is
/// left: `applied[k]` is the cash of part `k`. `None` when something is
/// left over (the movements against the cash are smaller than it), the
/// slices differ in length, or on overflow.
///
/// ```text
/// GOAL: split an entry's cash among the parts it settled, without
///       inventing or losing any.
/// SOURCE: rules.md B-14 rule 3 (ASC 230-10-50-5: only the cash portion
///         of a part-cash transaction is a cash flow; the order is a
///         CONVENTION, the ASC is silent).
/// INVARIANTS:
///   1. On success the parts sum to `cash` exactly.
///   2. Each part has the sign of `cash` (or is zero) and is no larger
///      than its own movement, which runs against the cash.
///   3. Applying `-cash` to the negated movements gives the negated parts
///      (a reversal mirrors its original).
/// SCENARIOS:
///   1. IF the movements against the cash sum to at least the cash:
///      - SHOULD succeed (a balanced entry always does)
///   2. IF they don't: SHOULD return None, never a part out of thin air.
/// ```
pub fn apply_cash<T: Amount>(cash: T, movements: &[T], applied: &mut [T]) -> Option<()> {
    if movements.len() != applied.len() {
        return None;
    }
    let mut remaining = cash;
    for (out, &movement) in applied.iter_mut().zip(movements) {
        let part = take(remaining, movement)?;
        *out = part;
        remaining = remaining.checked_sub(part)?;
    }
    (remaining == T::ZERO).then_some(())
}

/// An entry's cash flows (rules.md B-14 rule 3): the net movement of its
/// cash lines, applied to the sections moving against it in the order
/// operating, investing, financing, each up to its own movement; what is
/// left of a section's movement is the part no cash settled. `None` when
/// the lines don't net to zero (never for a posted entry) or on overflow.
/// It is [`entry_movements`] then [`flows_of`], so each is proved on its
/// own (docs/verification.md, law 10).
pub fn entry_flows<T: Amount>(lines: &[(Flow, Side, T)]) -> Option<EntryFlows<T>> {
    let (cash, moved) = entry_movements(lines)?;
    flows_of(cash, moved)
}

/// An entry's net movement on its cash lines and on each section's,
/// `(cash, [operating, investing, financing])`, debit-positive. `None`
/// on overflow.
pub fn entry_movements<T: Amount>(lines: &[(Flow, Side, T)]) -> Option<(T, [T; 3])> {
    let mut cash = T::ZERO;
    let mut moved = [T::ZERO; 3];
    for &(flow, side, amount) in lines {
        let delta = signed(side, amount)?;
        let slot = match flow {
            Flow::Cash => &mut cash,
            Flow::Operating => &mut moved[0],
            Flow::Investing => &mut moved[1],
            Flow::Financing => &mut moved[2],
        };
        *slot = slot.checked_add(delta)?;
    }
    Some((cash, moved))
}

/// The cash flows of an entry whose cash lines moved `cash` and whose
/// sections moved `moved` ([`entry_flows`]). `None` when they don't net
/// to zero, or on overflow.
pub fn flows_of<T: Amount>(cash: T, moved: [T; 3]) -> Option<EntryFlows<T>> {
    // Balanced lines only: every part below rests on it.
    let mut total = cash;
    for &m in &moved {
        total = total.checked_add(m)?;
    }
    if total != T::ZERO {
        return None;
    }
    let mut flows = [T::ZERO; 3];
    apply_cash(cash, &moved, &mut flows)?;
    let mut noncash = [T::ZERO; 3];
    for ((n, m), f) in noncash.iter_mut().zip(moved).zip(flows) {
        *n = m.checked_add(f)?;
    }
    Some(EntryFlows {
        cash,
        flows,
        noncash,
    })
}

/// The credits and the debits among some movements (debit-positive):
/// `(Σ max(-m, 0), Σ max(m, 0))`.
fn credits_and_debits<T: Amount>(movements: &[T]) -> Option<(T, T)> {
    let (mut credits, mut debits) = (T::ZERO, T::ZERO);
    for &m in movements {
        if m < T::ZERO {
            credits = credits.checked_sub(m)?;
        } else {
            debits = debits.checked_add(m)?;
        }
    }
    Some((credits, debits))
}

/// Which way money a member or a lender handled for the company went
/// (rules.md B-14 rule 11, D-42).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OnBehalf {
    /// Paid for the company: sources credited (borrowed, contributed)
    /// against uses debited (spent).
    Paid,
    /// Received for the company: sources debited (repaid, distributed)
    /// against uses credited (collected).
    Received,
}

impl OnBehalf {
    /// The other way: what an entry's reversal does.
    pub fn mirror(self) -> Self {
        match self {
            OnBehalf::Paid => OnBehalf::Received,
            OnBehalf::Received => OnBehalf::Paid,
        }
    }
}

/// What a member or a lender paid, or received, for the company with no
/// cash account moving: a constructive receipt and disbursement (rules.md
/// B-14 rule 11, D-42). `sources` are what is left unsettled of the
/// entry's kinds that can pay or receive for the company (a financing
/// liability, a member's equity), `uses` of the kinds they can pay or
/// receive for, each debit-positive. [`OnBehalf::Paid`]: `amount =
/// min(source credits, use debits)`, cash in to the sources' credits and
/// out of the uses' debits (borrowed or contributed, then spent);
/// [`OnBehalf::Received`]: `amount = min(source debits, use credits)`,
/// cash in to the uses' credits and out of the sources' debits (collected,
/// then repaid or distributed). `from[i]` is source `i`'s cash (+ in) and
/// `to[j]` use `j`'s, each applied in order by [`apply_cash`]. `None` when
/// the slices differ in length, or on overflow.
///
/// ```text
/// GOAL: show money a member or lender handled for the company as the
///       company's own, borrowed (or contributed) and spent, without
///       inventing or losing a cent.
/// SOURCE: rules.md B-14 rule 11 (D-42; ASC 230 is silent, PwC Viewpoint
///         FSP 6.9.3.1, Deloitte DART Roadmap 7.2, BDO Blueprint 3.4).
/// INVARIANTS:
///   1. The parts net to zero: Σ from + Σ to = 0, so the entry's cash
///      flows still sum to its cash.
///   2. Each part settles its own movement, toward zero and never past it
///      (zero, or the opposite sign and no larger).
///   3. The other way, minus the sources and minus the uses give minus
///      the parts (a reversal mirrors its original).
///   4. As much is settled as can be: afterwards, paid, no source credit
///      is left beside a use debit; received, no source debit beside a use
///      credit.
/// ```
pub fn constructive<T: Amount>(
    way: OnBehalf,
    sources: &[T],
    uses: &[T],
    from: &mut [T],
    to: &mut [T],
) -> Option<()> {
    let (source_credits, source_debits) = credits_and_debits(sources)?;
    let (use_credits, use_debits) = credits_and_debits(uses)?;
    // The cash the sources take (+ in): what was paid, or minus what was
    // received; the uses take the opposite.
    let cash = match way {
        OnBehalf::Paid => source_credits.min(use_debits),
        OnBehalf::Received => source_debits.min(use_credits).checked_neg()?,
    };
    apply_cash(cash, sources, from)?;
    apply_cash(cash.checked_neg()?, uses, to)
}

/// How much of an entry is an exchange inside investing and financing
/// that no cash settled (rules.md B-14 rule 7, ASC 230-10-50-4: debt
/// converted to equity, one asset traded for another). `debits` and
/// `credits` are the net debits and net credits of the entry's investing
/// and financing balance-sheet accounts (each account netted first; both
/// non-negative); `inflow` and `outflow` are the cash those two sections
/// took in and paid out (both non-negative; constructive cash included,
/// rule 11). Cash in settles credits and cash out settles debits; what's
/// left of the smaller side was exchanged:
/// `max(0, min(debits - outflow, credits - inflow))`.
pub fn exchanged<T: Amount>(debits: T, credits: T, inflow: T, outflow: T) -> Option<T> {
    let d = debits.checked_sub(outflow)?;
    let c = credits.checked_sub(inflow)?;
    Some(d.min(c).max(T::ZERO))
}

/// Which way an entry moved value with its parties (rules.md B-35).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Direction {
    /// The company paid the party, or gave it value: a line on the
    /// party's own account would be a debit.
    Paid,
    /// The company received from the party: a line on its own account
    /// would be a credit.
    Received,
    /// Neither: nothing moved with the party yet (a bill, an invoice, a
    /// fee charged on its own card, a distribution still owed), or the
    /// entry moved nothing with anyone (a close, a reclassification).
    /// Counted in no total.
    Neither,
}

impl Direction {
    /// The other way: paid and received swap; neither stays neither.
    pub fn flip(self) -> Self {
        match self {
            Direction::Paid => Direction::Received,
            Direction::Received => Direction::Paid,
            Direction::Neither => Direction::Neither,
        }
    }

    /// The direction of a signed amount: paid above zero, received below,
    /// neither at zero.
    pub fn of<T: Amount>(amount: T) -> Self {
        if amount > T::ZERO {
            Direction::Paid
        } else if amount < T::ZERO {
            Direction::Received
        } else {
            Direction::Neither
        }
    }

    /// Its wire name: `paid`, `received`, or `neither`.
    pub fn key(self) -> &'static str {
        match self {
            Direction::Paid => "paid",
            Direction::Received => "received",
            Direction::Neither => "neither",
        }
    }
}

/// What an entry's parties split, and what of it counts in their totals
/// (rules.md B-35).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PartyTarget<T> {
    /// The whole, signed as a line on the parties' own account would be
    /// (debit-positive): positive, the company paid or gave; negative, it
    /// received.
    pub amount: T,
    /// Which way value moved.
    pub direction: Direction,
    /// The part of `amount` that moved as money (of a member's
    /// contribution, what the K-1 worksheet counts as cash; of a
    /// distribution, the money it paid): counted on both bases.
    pub cash: T,
    /// The part of `amount` that moved as property: counted on basis
    /// "all" only.
    pub in_kind: T,
}

/// The magnitude of `a`, or `None` past the type's range.
fn magnitude<T: Amount>(a: T) -> Option<T> {
    if a < T::ZERO {
        a.checked_neg()
    } else {
        Some(a)
    }
}

/// The magnitude `m` with the sign of `like`, never a negative zero (a
/// zero `m` is the plain zero, whatever sign arithmetic left on it).
fn with_sign<T: Amount>(like: T, m: T) -> Option<T> {
    if m == T::ZERO {
        Some(T::ZERO)
    } else if like < T::ZERO {
        m.checked_neg()
    } else {
        Some(m)
    }
}

/// The smaller of `a` and `b`.
fn least<T: Amount>(a: T, b: T) -> T {
    if a < b { a } else { b }
}

/// What an entry's parties split and what of it counts (rules.md B-35
/// item 3, every case but a close and a reversal, which the caller
/// handles: a close is its parties' own-account movement with direction
/// neither, a reversal is [`reverse_target`] of its original's).
///
/// ```text
/// GOAL: one signed whole for an entry's parties, debit-positive as a line
///       on their own account, with its direction and what of it moved as
///       money or as property, so a property contribution and its
///       reversal never read alike, and a promise is never counted as if
///       it were paid.
/// SOURCE: rules.md B-35 (B-2 signs; DECISION, owner, 2026-10-20: a
///         member's entry counts what moved their capital, money is cash,
///         cards, the members' financing and loans, and a liability credit
///         never counts as paying that liability's own creditor; Helvering
///         v. Price: a promise to pay is not payment).
/// INPUTS (each signed debit-positive over the entry's lines):
///   capital: Some((e, property)) when every party named is a member: e
///     the movement of equity other than retained earnings; property what
///     property accounts took in (their debits, when e < 0) or gave up
///     (their credits, when e > 0).
///   net: the money paid (+) or received (−) (parties::entry_flow, a
///     named party's own liabilities not counted as money).
///   own: the movement of the parties' own accounts (payables,
///     receivables, liabilities owed to a party named).
///   debits: the entry's total debits.
/// INVARIANTS:
///   1. capital with e < 0 (a contribution): amount = e; |in_kind| =
///      min(|e|, property⁺); cash the rest (what the K-1 worksheet counts
///      as cash: money, or what the member paid for the company).
///   2. capital with e > 0 (a distribution): amount = e; cash = min(e,
///      net⁺), the money it paid; in_kind = min(e − cash, property⁺), the
///      property it gave; the rest (credited to a liability that isn't
///      money for the entry: a distribution declared, still owed) counts
///      as nothing until it is paid.
///   3. In 1 and 2, direction is the sign of e, or neither when nothing
///      counts.
///   4. Else net ≠ 0: amount = cash = net, in_kind = 0, direction its
///      sign.
///   5. Else amount = own when it isn't zero, else debits; neither.
///   6. cash and in_kind each have amount's sign or are zero (never a
///      negative zero), |cash| + |in_kind| ≤ |amount|, and direction is
///      neither exactly when both are zero.
///   7. Odd, but for the debits fallback and a member's entry whose cash
///      isn't exactly its money (an expense booked straight to capital, a
///      debt converted, a distribution still owed): negating e, net and
///      own (property comes from the other side of the lines, so it keeps
///      its size) gives reverse_target of the result.
/// ```
///
/// `None` past the type's range.
pub fn party_target<T: Amount>(
    capital: Option<(T, T)>,
    net: T,
    own: T,
    debits: T,
) -> Option<PartyTarget<T>> {
    if let Some((e, property)) = capital
        && e != T::ZERO
    {
        let whole = magnitude(e)?;
        let property = if property > T::ZERO {
            property
        } else {
            T::ZERO
        };
        let (cash, in_kind) = if e < T::ZERO {
            let in_kind = least(property, whole);
            (whole.checked_sub(in_kind)?, in_kind)
        } else {
            let paid = if net > T::ZERO {
                least(net, whole)
            } else {
                T::ZERO
            };
            (paid, least(property, whole.checked_sub(paid)?))
        };
        let counted = cash.checked_add(in_kind)?;
        return Some(PartyTarget {
            amount: e,
            direction: if counted == T::ZERO {
                Direction::Neither
            } else {
                Direction::of(e)
            },
            cash: with_sign(e, cash)?,
            in_kind: with_sign(e, in_kind)?,
        });
    }
    if net != T::ZERO {
        return Some(PartyTarget {
            amount: net,
            direction: Direction::of(net),
            cash: net,
            in_kind: T::ZERO,
        });
    }
    Some(PartyTarget {
        amount: if own != T::ZERO { own } else { debits },
        direction: Direction::Neither,
        cash: T::ZERO,
        in_kind: T::ZERO,
    })
}

/// A reversal's target from its original's (rules.md B-35 item 3; B-9: a
/// correction is the opposite entry): every amount negated (never to a
/// negative zero), paid and received swapped. An involution. `None` past
/// the type's range.
pub fn reverse_target<T: Amount>(t: PartyTarget<T>) -> Option<PartyTarget<T>> {
    let neg = |a: T| {
        if a == T::ZERO {
            Some(T::ZERO)
        } else {
            a.checked_neg()
        }
    };
    Some(PartyTarget {
        amount: neg(t.amount)?,
        direction: t.direction.flip(),
        cash: neg(t.cash)?,
        in_kind: neg(t.in_kind)?,
    })
}

// ---------------------------------------------------------------------------
// Proration by days
// ---------------------------------------------------------------------------

/// Exact integer arithmetic the proration by days needs: money as a whole
/// number of its smallest step (`i128` in the engine; `i64` and, for the
/// harnesses whose properties relate a product to a quotient, `i32` under
/// Kani). Kept apart from [`Amount`]: proration multiplies and divides,
/// which no journal line ever does.
pub trait Units: Copy + Ord {
    /// Zero.
    const ZERO: Self;
    /// One.
    const ONE: Self;
    /// `n`, exactly, or `None` when it doesn't fit.
    fn from_u32(n: u32) -> Option<Self>;
    /// `self + rhs`, or `None` on overflow.
    fn checked_add(self, rhs: Self) -> Option<Self>;
    /// `self - rhs`, or `None` on overflow.
    fn checked_sub(self, rhs: Self) -> Option<Self>;
    /// `self * rhs`, or `None` on overflow.
    fn checked_mul(self, rhs: Self) -> Option<Self>;
    /// `(self / rhs, self % rhs)` for `self >= 0` and `rhs > 0`; `None`
    /// otherwise.
    fn checked_div_rem(self, rhs: Self) -> Option<(Self, Self)>;
}

impl Units for i32 {
    const ZERO: Self = 0;
    const ONE: Self = 1;
    fn from_u32(n: u32) -> Option<Self> {
        i32::try_from(n).ok()
    }
    fn checked_add(self, rhs: Self) -> Option<Self> {
        i32::checked_add(self, rhs)
    }
    fn checked_sub(self, rhs: Self) -> Option<Self> {
        i32::checked_sub(self, rhs)
    }
    fn checked_mul(self, rhs: Self) -> Option<Self> {
        i32::checked_mul(self, rhs)
    }
    fn checked_div_rem(self, rhs: Self) -> Option<(Self, Self)> {
        if self < 0 || rhs <= 0 {
            return None;
        }
        Some((self / rhs, self % rhs))
    }
}

impl Units for i64 {
    const ZERO: Self = 0;
    const ONE: Self = 1;
    fn from_u32(n: u32) -> Option<Self> {
        Some(i64::from(n))
    }
    fn checked_add(self, rhs: Self) -> Option<Self> {
        i64::checked_add(self, rhs)
    }
    fn checked_sub(self, rhs: Self) -> Option<Self> {
        i64::checked_sub(self, rhs)
    }
    fn checked_mul(self, rhs: Self) -> Option<Self> {
        i64::checked_mul(self, rhs)
    }
    fn checked_div_rem(self, rhs: Self) -> Option<(Self, Self)> {
        if self < 0 || rhs <= 0 {
            return None;
        }
        Some((self / rhs, self % rhs))
    }
}

impl Units for i128 {
    const ZERO: Self = 0;
    const ONE: Self = 1;
    fn from_u32(n: u32) -> Option<Self> {
        Some(i128::from(n))
    }
    fn checked_add(self, rhs: Self) -> Option<Self> {
        i128::checked_add(self, rhs)
    }
    fn checked_sub(self, rhs: Self) -> Option<Self> {
        i128::checked_sub(self, rhs)
    }
    fn checked_mul(self, rhs: Self) -> Option<Self> {
        i128::checked_mul(self, rhs)
    }
    fn checked_div_rem(self, rhs: Self) -> Option<(Self, Self)> {
        if self < 0 || rhs <= 0 {
            return None;
        }
        Some((self / rhs, self % rhs))
    }
}

/// lcm(28, 29, 30, 31): every month's length divides it, so a day of any
/// month is a whole number of `MONTH_L`ths of a month, and proration needs
/// no fractions.
pub const MONTH_L: u32 = 377_580;

/// One calendar month's share of a span of days: `days` of the month's
/// `month_days`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MonthPart {
    /// Days of the span in the month, 1 to `month_days`.
    pub days: u32,
    /// Days in the month, 28 to 31.
    pub month_days: u32,
}

impl MonthPart {
    /// Whether it is a share a calendar month can have: 28 to 31 days in
    /// the month, 1 to all of them in the span.
    pub fn is_valid(self) -> bool {
        (28..=31).contains(&self.month_days) && (1..=self.month_days).contains(&self.days)
    }
}

/// A part's weight in `MONTH_L`ths of a month: `days × (MONTH_L /
/// month_days)`, exactly. `None` for a part no month has.
pub fn part_weight<T: Units>(p: MonthPart) -> Option<T> {
    if !p.is_valid() {
        return None;
    }
    T::from_u32(p.days)?.checked_mul(T::from_u32(MONTH_L / p.month_days)?)
}

/// `num / den` rounded half away from zero to a multiple of `unit` (`num`
/// not negative; `den` and `unit` positive).
fn round_to_unit<T: Units>(num: T, den: T, unit: T) -> Option<T> {
    let step = den.checked_mul(unit)?;
    let (q, r) = num.checked_div_rem(step)?;
    // Up when the remainder is half a step or more: compared as r >=
    // step - r, so nothing is doubled and nothing can overflow.
    let q = if r >= step.checked_sub(r)? {
        q.checked_add(T::ONE)?
    } else {
        q
    };
    q.checked_mul(unit)
}

/// The exact cumulative charge of `total` (in units) spread over `months`
/// months by day, through `k` days of `part`, after earlier parts weighing
/// `before` `MONTH_L`ths; `last` marks the span's last part. Rounded half
/// away from zero (half up: nothing here is negative) to a multiple of
/// `unit`, or `total` itself when that rounds above it (a total that isn't
/// a whole number of units); and the last part's last day is `total`.
///
/// ```text
/// GOAL: how much of a total a span of days has used up by a given day,
///       each day of a month bearing the same share, the whole total
///       reached on the span's last day and never before.
/// SOURCE: docs/spec/rules.md B-15 (straight line, partial months by days;
///         ASC 360-10-35-4: "systematic and rational", as equitably as
///         possible to the periods served), B-16 (daily schedules, D-11),
///         B-21 (exact totals).
/// INVARIANTS (W = months × MONTH_L, m = part.month_days, d = part.days):
///   1. Not the last part: round(total × (before + k × MONTH_L / m) / W):
///      the monthly rate total/months times the days held over the days
///      in each month.
///   2. The last part: round(total × (before × d + (W − before) × k) /
///      (W × d)): what the earlier parts leave of the total, C = total ×
///      before / W, plus the remainder R = total − C spread evenly over
///      its d days. It is never "plugged": through an earlier day of the
///      last month it is prorated by days.
///   3. It is non-decreasing in (part, k), never above total, continuous
///      at part boundaries ((i, days_i) equals (i + 1, 0)), and exactly
///      total at (last, d).
/// SCENARIOS:
///   1. IF total is negative, months is 0, unit isn't positive, the part
///      is no month's (see MonthPart::is_valid), k > d, before is negative
///      or above W, a part but the last is charged through a day past W
///      (before + k × MONTH_L / m > W; prorate_by_days charges every part
///      but the last through its last day, so it refuses a part ending past
///      W), or any product overflows:
///      - SHOULD return None
///      - NEVER panic
///      BECAUSE a figure the books can't compute is refused (rules.md
///      B-28), and symbolic parts are not a calendar.
///   2. IF 12,000.00 over 12 months from Jan 1, through Dec 15 (the last
///      part, 31 days, before = 11 months):
///      - SHOULD be 11,000.00 + 1,000.00 × 15/31 = 11,483.87
///      - NEVER 12,000.00 (the old plug)
///      BECAUSE the final month is charged by its days like any other.
/// ```
pub fn prorated_through<T: Units>(
    total: T,
    months: u32,
    before: T,
    part: MonthPart,
    k: u32,
    last: bool,
    unit: T,
) -> Option<T> {
    if total < T::ZERO
        || months == 0
        || unit <= T::ZERO
        || !part.is_valid()
        || k > part.days
        || before < T::ZERO
    {
        return None;
    }
    let whole = T::from_u32(months)?.checked_mul(T::from_u32(MONTH_L)?)?;
    if before > whole {
        return None;
    }
    let charged = if last {
        if k == part.days {
            // Exactly the total, even one that isn't a whole number of
            // units (an asset an earlier version took in tenths of a cent).
            return Some(total);
        }
        let d = T::from_u32(part.days)?;
        let left = whole.checked_sub(before)?;
        let elapsed = before
            .checked_mul(d)?
            .checked_add(left.checked_mul(T::from_u32(k)?)?)?;
        round_to_unit(total.checked_mul(elapsed)?, whole.checked_mul(d)?, unit)?
    } else {
        let rate = T::from_u32(MONTH_L / part.month_days)?;
        let elapsed = before.checked_add(T::from_u32(k)?.checked_mul(rate)?)?;
        if elapsed > whole {
            return None;
        }
        round_to_unit(total.checked_mul(elapsed)?, whole, unit)?
    };
    // A total that isn't a whole number of units can round above itself
    // on the day before the last; the cap keeps every part non-negative.
    Some(charged.min(total))
}

/// Split `total` (in units) over a span of `months` months cut into
/// calendar-month `parts`: `out[i]` is the cumulative charge through the
/// end of part `i` ([`prorated_through`]) less the one before it, and the
/// last part is `total` less all the others. `out` must be as long as
/// `parts`; on `None` its contents mean nothing.
///
/// ```text
/// GOAL: one month's posting of a span spread by day (depreciation,
///       daily prepaid and deferred-revenue schedules).
/// SOURCE: docs/spec/rules.md B-15, B-16 (D-11), B-21; plan/served.md P3
///         defect 9 (one shared prorate_by_days, Kani: parts sum to the
///         total, each is days × rate within one unit, the last absorbs
///         the remainder).
/// INVARIANTS:
///   1. The parts sum to total exactly.
///   2. Every part is >= 0.
///   3. Each part but the last is within one unit of total × days_i /
///      (months × month_days_i).
///   4. The last part is total − Σ others.
/// SCENARIOS:
///   1. IF there are no parts, out isn't as long as parts, or prorated_
///      through refuses a part (the weights of all but the last above W,
///      among others): SHOULD return None, never panic.
///   2. IF 1,200.00 over 12 months from Mar 15 (parts 17/31, eleven full
///      months, then 14 days):
///      - SHOULD be 54.84, eleven × 100.00, then 45.16
///      BECAUSE each day bears the same share, and the last part takes
///      what the others leave.
/// ```
///
/// Allocation-free and O(n): a loop over fixed slices, so the model
/// checker can prove it outright.
pub fn prorate_by_days<T: Units>(
    total: T,
    months: u32,
    parts: &[MonthPart],
    unit: T,
    out: &mut [T],
) -> Option<()> {
    if out.len() != parts.len() {
        return None;
    }
    let (&last, rest) = parts.split_last()?;
    let mut before = T::ZERO;
    let mut charged = T::ZERO;
    for (o, &p) in out.iter_mut().zip(rest) {
        let through = prorated_through(total, months, before, p, p.days, false, unit)?;
        *o = through.checked_sub(charged)?;
        charged = through;
        before = before.checked_add(part_weight(p)?)?;
    }
    // The last part is checked too (its weight before it within the span),
    // and its whole is the total, so it takes the remainder.
    let through = prorated_through(total, months, before, last, last.days, true, unit)?;
    *out.last_mut()? = through.checked_sub(charged)?;
    Some(())
}

// ---------------------------------------------------------------------------
// Catching up and truing up
// ---------------------------------------------------------------------------

/// What a run catches up: the largest amount, at least zero, that can be
/// added to what the books carry at every month end from the run's on
/// without carrying more than the schedule says there:
/// `max(0, min_i(scheduled[i] − posted[i]))`, where `scheduled[i]` and
/// `posted[i]` are cumulative at successive month ends starting with the
/// run's own. `None` when the slices differ in length or are empty, or on
/// overflow.
///
/// ```text
/// GOAL: a skipped month is caught up in the month the run is made, and a
///       run never overtakes the schedule at a month end already posted.
/// SOURCE: plan/served.md P3 "Settled in design" (2026-10-20): a month of
///         depreciation skipped is caught up in the month the run is made
///         (an immaterial miss; a material one is restated, ASC 250);
///         docs/spec/rules.md B-15.
/// INVARIANTS:
///   1. The result is >= 0.
///   2. Whenever every posted[i] <= scheduled[i], posted[i] + result <=
///      scheduled[i] for every i: the run never carries the books past the
///      schedule at any month end it reaches.
///   3. The result is 0 or some scheduled[i] − posted[i]: with one month
///      (no run after this one), it is everything the schedule says is due
///      through it.
/// ```
pub fn catch_up<T: Amount>(scheduled: &[T], posted: &[T]) -> Option<T> {
    if scheduled.len() != posted.len() {
        return None;
    }
    let mut least: Option<T> = None;
    for (&s, &p) in scheduled.iter().zip(posted) {
        let gap = s.checked_sub(p)?;
        least = Some(match least {
            Some(l) if l <= gap => l,
            _ => gap,
        });
    }
    let least = least?;
    Some(if least > T::ZERO { least } else { T::ZERO })
}

/// [`catch_up`] of every suffix at once: `out[i]` is `catch_up(&scheduled[i..],
/// &posted[i..])`, what a run at each month end posts. One pass from the
/// end, O(n), so a viewer can say of every month of a long life whether
/// anything is due for it. `None` when the slices differ in length, or on
/// overflow.
pub fn catch_ups<T: Amount>(scheduled: &[T], posted: &[T], out: &mut [T]) -> Option<()> {
    if scheduled.len() != posted.len() || out.len() != scheduled.len() {
        return None;
    }
    let mut least: Option<T> = None;
    for ((&s, &p), o) in scheduled.iter().zip(posted).zip(out.iter_mut()).rev() {
        let gap = s.checked_sub(p)?;
        let l = match least {
            Some(l) if l <= gap => l,
            _ => gap,
        };
        least = Some(l);
        *o = if l > T::ZERO { l } else { T::ZERO };
    }
    Some(())
}

/// Bring what the books carry (`posted`) to `target`: `(post, take_back)`,
/// both >= 0, one of them zero, with `posted + post − take_back = target`.
/// This is [`columns`] of `target − posted`: raising the books by `post`
/// and lowering them by `take_back` are a debit and a credit of the gap.
///
/// ```text
/// GOAL: a disposal clears exactly what the schedule says was used up
///       through its day, whatever the runs posted.
/// SOURCE: docs/spec/rules.md B-15 (a disposed asset depreciates through
///         its disposal day); Ledger::dispose_asset INVARIANT 1 (gain =
///         proceeds − book value at the disposal date).
/// ```
pub fn true_up<T: Amount>(target: T, posted: T) -> Option<(T, T)> {
    columns(target.checked_sub(posted)?)
}

/// An asset's three basis figures ([`basis_figures`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BasisFigures<T> {
    /// The partnership's basis in contributed property, IRC §723: the
    /// contributor's adjusted basis. `None` for a purchase.
    pub tax_basis_723: Option<T>,
    /// The basis tax depreciation starts from.
    pub depreciable_basis: T,
    /// Fair market value less the §723 basis at contribution, IRC §704(c):
    /// positive a built-in gain, negative a built-in loss. `None` for a
    /// purchase (not property contributed).
    pub built_in_gain_704c: Option<T>,
}

/// An asset's basis figures from its rule and facts: `cost` the book cost
/// (a purchase's basis), `adjusted` the contributing member's adjusted
/// basis and `fmv` the fair market value at contribution (both ignored for
/// a purchase).
///
/// ```text
/// GOAL: the three tax figures of an asset, each named for what it is,
///       from one function the register, the tools and the viewer share.
/// SOURCE: docs/spec/rules.md B-25: IRC §1012(a) (cost); IRC §723 (the
///         contributor's adjusted basis carries over); Treas. Reg.
///         §1.168(i)-4(b)(1) (property converted from personal use: "the
///         lesser of its fair market value or its adjusted depreciable
///         basis"; DECISION D-41, owner 2026-10-20); Treas. Reg.
///         §1.704-3(a)(3)(ii) (built-in gain: "the excess of the
///         property's book value over the contributing partner's adjusted
///         tax basis upon contribution", book value being fair market
///         value at contribution, §1.704-3(a)(3)(i)).
/// INVARIANTS:
///   1. Purchase: depreciable = cost; no §723 basis, no built-in gain.
///   2. Contribution: tax_basis_723 = adjusted; built_in_gain_704c =
///      fmv − adjusted, negative exactly when adjusted > fmv (a built-in
///      loss, IRC §704(c)(1)(C)).
///   3. Carryover723: depreciable = adjusted.
///   4. PersonalUseConversion: depreciable = min(fmv, adjusted), at most
///      each of them and equal to one.
///   5. NEVER wraps: None when a figure can't be represented.
/// ```
pub fn basis_figures<T: Amount>(
    rule: BasisRule,
    cost: T,
    adjusted: T,
    fmv: T,
) -> Option<BasisFigures<T>> {
    Some(match rule {
        BasisRule::Purchase => BasisFigures {
            tax_basis_723: None,
            depreciable_basis: cost,
            built_in_gain_704c: None,
        },
        BasisRule::Carryover723 => BasisFigures {
            tax_basis_723: Some(adjusted),
            depreciable_basis: adjusted,
            built_in_gain_704c: Some(fmv.checked_sub(adjusted)?),
        },
        BasisRule::PersonalUseConversion => BasisFigures {
            tax_basis_723: Some(adjusted),
            depreciable_basis: if fmv < adjusted { fmv } else { adjusted },
            built_in_gain_704c: Some(fmv.checked_sub(adjusted)?),
        },
    })
}

/// A project's unrecovered cost from its tagged revenue and expenses to
/// date: what it would take to be repaid in full (rules.md B-37 item 4).
///
/// ```text
/// GOAL: the "if repaid" figure of work done on spec, from one function
///       the tools, the viewer and bookspec share.
/// SOURCE: docs/spec/rules.md B-37 item 4 (ASC 450-30-25-1: a hoped-for
///         payment is a gain contingency, never recorded before it is
///         received; so this is a figure, never a receivable).
/// INVARIANTS:
///   1. Never below zero.
///   2. expenses - revenue when expenses >= revenue; zero otherwise.
///   3. Never above max(expenses, 0) when revenue >= 0 (expenses can be
///      negative: a reimbursement credited to the expense above its cost).
///   4. NEVER wraps: None when the difference can't be represented.
/// ```
pub fn unrecovered_cost<T: Amount>(revenue: T, expenses: T) -> Option<T> {
    let gap = expenses.checked_sub(revenue)?;
    Some(if gap > T::ZERO { gap } else { T::ZERO })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// rules.md B-14 rule 3, by hand. Equipment 1,000 for 300 cash and a
    /// 700 note: investing takes the 300, the note's 700 and the rest of
    /// the equipment's are the noncash part.
    #[test]
    fn part_cash_part_noncash_takes_only_the_cash() {
        use Flow::*;
        let f = entry_flows(&[
            (Investing, Side::Debit, 1000i64),
            (Cash, Side::Credit, 300),
            (Financing, Side::Credit, 700),
        ])
        .unwrap();
        assert_eq!(f.cash, -300);
        assert_eq!(f.flows, [0, -300, 0]);
        assert_eq!(f.noncash, [0, 700, -700]);
        // A fee withheld from a loan: Dr cash 9,900, Dr fee 100 / Cr loan
        // 10,000. Operating moves with the cash (a debit), so takes none.
        let f = entry_flows(&[
            (Cash, Side::Debit, 9900i64),
            (Operating, Side::Debit, 100),
            (Financing, Side::Credit, 10000),
        ])
        .unwrap();
        assert_eq!(f.flows, [0, 0, 9900]);
        assert_eq!(f.noncash, [100, 0, -100]);
    }

    /// Lines that don't net to zero are refused (review item 12): no part
    /// is made up for them.
    #[test]
    fn unbalanced_lines_have_no_flows() {
        use Flow::*;
        assert_eq!(
            entry_flows(&[(Operating, Side::Debit, 5i64), (Investing, Side::Debit, 5)]),
            None
        );
        assert_eq!(
            entry_flows(&[(Cash, Side::Debit, 5i64), (Investing, Side::Credit, 4)]),
            None
        );
        // Too little against the cash: nothing is invented.
        let mut out = [0i64; 2];
        assert_eq!(apply_cash(10, &[-3, 4], &mut out), None);
        assert_eq!(apply_cash(10, &[-3], &mut out), None);
    }

    /// The exchange measure (rules.md B-14 rule 7) on the review's
    /// hand-checked cases.
    #[test]
    fn exchanged_matches_the_hand_checked_cases() {
        // Debt to equity, 500.
        assert_eq!(exchanged(500i64, 500, 0, 0), Some(500));
        // Part-cash purchase with the seller's note: D 1,000, C 700, cash
        // out 300.
        assert_eq!(exchanged(1000i64, 700, 0, 300), Some(700));
        // Note received for equipment: D 700, C 1,000, cash in 300.
        assert_eq!(exchanged(700i64, 1000, 300, 0), Some(700));
        // Trade-in with 1,500 boot: D 2,000, C 1,000, cash out 1,500.
        assert_eq!(exchanged(2000i64, 1000, 0, 1500), Some(500));
        // A cash sale: nothing debited, cash in 500.
        assert_eq!(exchanged(0i64, 1000, 500, 0), Some(0));
        // A cash purchase: cash out covers every debit.
        assert_eq!(exchanged(1000i64, 0, 0, 1000), Some(0));
        // Equipment a lender paid for (rule 11): 1,000 borrowed in, 1,000
        // spent out, nothing exchanged.
        assert_eq!(exchanged(1000i64, 1000, 1000, 1000), Some(0));
    }

    /// rules.md B-14 rule 11 (D-42), by hand.
    #[test]
    fn constructive_pays_and_receives_for_the_company() {
        use OnBehalf::{Paid, Received};
        // A member pays a 59 expense (Dr expense / Cr due to members): the
        // source's 59 credit pays the use's 59 debit.
        let (mut from, mut to) = ([0i64; 1], [0i64; 1]);
        constructive(Paid, &[-59], &[59], &mut from, &mut to).unwrap();
        assert_eq!((from, to), ([59], [-59]));
        // Its reversal, the other way, mirrors it.
        constructive(Received, &[59], &[-59], &mut from, &mut to).unwrap();
        assert_eq!((from, to), ([-59], [59]));
        // The reversal's shape read as a payment settles nothing.
        constructive(Paid, &[59], &[-59], &mut from, &mut to).unwrap();
        assert_eq!((from, to), ([0], [0]));
        // A 100 credit from the source against 60 of uses (the other 40
        // against something else): only 60 is paid; uses take in order.
        let (mut from, mut to) = ([0i64; 2], [0i64; 2]);
        constructive(Paid, &[-100, 0], &[25, 35], &mut from, &mut to).unwrap();
        assert_eq!((from, to), ([60, 0], [-25, -35]));
        // Two sources, one use smaller than both: the first source pays
        // first.
        constructive(Paid, &[-30, -30], &[40, 0], &mut from, &mut to).unwrap();
        assert_eq!((from, to), ([30, 10], [-40, 0]));
        // Each way settles only its own side.
        constructive(Paid, &[-50, 20], &[50, -20], &mut from, &mut to).unwrap();
        assert_eq!((from, to), ([50, 0], [-50, 0]));
        constructive(Received, &[-50, 20], &[50, -20], &mut from, &mut to).unwrap();
        assert_eq!((from, to), ([0, -20], [0, 20]));
        // Nothing against: nothing moves.
        constructive(Paid, &[-50, 0], &[-10, 0], &mut from, &mut to).unwrap();
        assert_eq!((from, to), ([0, 0], [0, 0]));
        // Mismatched lengths are refused.
        let one = [-1i64];
        assert_eq!(
            constructive(Paid, &one, &[1], &mut [0i64; 2], &mut [0]),
            None
        );
        assert_eq!(
            constructive(Paid, &one, &[1], &mut [0i64], &mut [0; 2]),
            None
        );
        assert_eq!(Paid.mirror(), Received);
        assert_eq!(Received.mirror(), Paid);
    }

    fn part(days: u32, month_days: u32) -> MonthPart {
        MonthPart { days, month_days }
    }

    /// Every month length divides MONTH_L, and it is the least such.
    #[test]
    fn month_l_is_the_lcm_of_month_lengths() {
        for m in 28..=31 {
            assert_eq!(MONTH_L % m, 0, "{m}");
        }
        for smaller in 1..MONTH_L {
            assert!((28..=31).any(|m| smaller % m != 0), "{smaller}");
        }
        assert_eq!(part_weight::<i64>(part(31, 31)), Some(i64::from(MONTH_L)));
        assert_eq!(part_weight::<i64>(part(1, 28)), Some(13_485));
        assert_eq!(part_weight::<i64>(part(0, 30)), None);
        assert_eq!(part_weight::<i64>(part(32, 31)), None);
        assert_eq!(part_weight::<i64>(part(27, 27)), None);
    }

    /// Spec scenario 2 of prorated_through, and the brief's 1,200.00 from
    /// March 15 (prorate_by_days scenario 2), in cents.
    #[test]
    fn the_final_month_is_prorated_and_a_mid_month_span_splits_by_day() {
        let l = i128::from(MONTH_L);
        let dec15 = prorated_through(1_200_000i128, 12, 11 * l, part(31, 31), 15, true, 1);
        assert_eq!(dec15, Some(1_148_387));
        assert_eq!(
            prorated_through(1_200_000i128, 12, 11 * l, part(31, 31), 31, true, 1),
            Some(1_200_000)
        );
        let mut parts = vec![part(17, 31)];
        parts.extend([30, 31, 30, 31, 31, 30, 31, 30, 31, 31, 28].map(|m| part(m, m)));
        parts.push(part(14, 31));
        let mut out = vec![0i128; parts.len()];
        prorate_by_days(120_000i128, 12, &parts, 1, &mut out).unwrap();
        let mut want = vec![5_484i128];
        want.extend([10_000; 11]);
        want.push(4_516);
        assert_eq!(out, want);
    }

    /// Half a unit rounds away from zero (up, for the non-negative amounts
    /// here): 0.01 over 2 months from the 1st gives 0.005 after one month.
    #[test]
    fn half_a_unit_rounds_up() {
        let mut out = [0i64; 2];
        prorate_by_days(1, 2, &[part(31, 31), part(28, 28)], 1, &mut out).unwrap();
        assert_eq!(out, [1, 0]);
        // Just under half a unit stays down: 0.01 over 3 months.
        let mut out = [0i64; 3];
        prorate_by_days(
            1,
            3,
            &[part(31, 31), part(28, 28), part(31, 31)],
            1,
            &mut out,
        )
        .unwrap();
        assert_eq!(out, [0, 1, 0]);
    }

    /// R4(a): a total that isn't a whole number of units (100.006 in
    /// thousandths, rounded to cents) never makes the last part negative.
    #[test]
    fn a_total_finer_than_its_unit_never_gives_a_negative_part() {
        let l = i64::from(MONTH_L);
        let mut out = [0i64; 2];
        prorate_by_days(100_006, 2, &[part(31, 31), part(28, 28)], 10, &mut out).unwrap();
        assert_eq!(out, [50_000, 50_006]);
        // 30 of 31 days of a one-month span: 100.006 × 30/31 = 96.780
        // exactly, already whole cents.
        assert_eq!(
            prorated_through(100_006i64, 1, 0, part(31, 31), 30, true, 10),
            Some(96_780)
        );
        // One day short of a 4,000-month span: 100.006 less 0.0008 is
        // 100.0052, which rounds to 100.01, past the total: capped there,
        // so the day left gets 0, never -0.004.
        let w = 4_000 * l;
        assert_eq!(
            prorated_through(100_006i64, 4_000, w - l, part(30, 31), 30, false, 10),
            Some(100_006)
        );
    }

    /// Spec scenario 1 of prorated_through (R4(b), R4(c)): malformed input
    /// is refused, never a panic.
    #[test]
    fn malformed_proration_is_refused() {
        let l = i64::from(MONTH_L);
        let p = part(10, 30);
        assert_eq!(prorated_through(-1i64, 1, 0, p, 1, false, 1), None);
        assert_eq!(prorated_through(1i64, 0, 0, p, 1, false, 1), None);
        assert_eq!(prorated_through(1i64, 1, 0, p, 1, false, 0), None);
        assert_eq!(prorated_through(1i64, 1, 0, p, 11, false, 1), None);
        assert_eq!(prorated_through(1i64, 1, -1, p, 1, false, 1), None);
        assert_eq!(prorated_through(1i64, 1, l + 1, p, 1, true, 1), None);
        assert_eq!(prorated_through(1i64, 1, 0, part(0, 30), 0, false, 1), None);
        assert_eq!(
            prorated_through(1i64, 1, 0, part(31, 30), 1, false, 1),
            None
        );
        // A part but the last that ends past the span.
        assert_eq!(prorated_through(1i64, 1, l, part(1, 31), 1, false, 1), None);
        let mut out = [0i64; 3];
        assert_eq!(
            prorate_by_days(
                100,
                1,
                &[part(31, 31), part(31, 31), part(5, 30)],
                1,
                &mut out
            ),
            None
        );
        assert_eq!(prorate_by_days(100i64, 1, &[], 1, &mut []), None);
        assert_eq!(
            prorate_by_days(100i64, 1, &[part(31, 31)], 1, &mut []),
            None
        );
        assert_eq!(
            prorated_through(i64::MAX, 600, 0, part(31, 31), 31, false, 1),
            None
        );
    }

    /// prorated_through over every span of 1 to 3 months starting on any
    /// day of a common year and a leap year, walked day by day, for a
    /// spread of totals and both units: it never falls (within a month and
    /// across a month's end: the charge through a month's last day is the
    /// next month's through none of its days), never exceeds the total, and
    /// is the total on the last day. Exhaustive over those spans; the
    /// relation between two divisions is too costly for Kani's solver
    /// (docs/verification.md), and Lean proves the law for the model.
    /// Not under Miri, which looks for undefined behavior: this is safe
    /// integer arithmetic the other kernel tests already take through it,
    /// and walked exhaustively under the interpreter it held `just miri`
    /// for over ten minutes.
    #[test]
    #[cfg_attr(
        miri,
        ignore = "exhaustive arithmetic: the other kernel tests cover it under Miri"
    )]
    fn the_charge_never_falls_and_is_continuous_over_real_calendars() {
        use chrono::NaiveDate;
        let mut start = NaiveDate::from_ymd_opt(2027, 1, 1).unwrap();
        let stop = NaiveDate::from_ymd_opt(2029, 1, 1).unwrap();
        let mut spans = 0;
        while start < stop {
            for months in 1..=3u32 {
                let end = start
                    .checked_add_months(chrono::Months::new(months))
                    .unwrap();
                let parts: Vec<_> = crate::calendar::month_parts(start, end).collect();
                for total in [0i64, 1, 3, 7, 10, 999, 100_005, 65_536] {
                    for unit in [1i64, 10] {
                        let mut before = 0i64;
                        let mut last_charge = 0i64;
                        for p in &parts {
                            for k in 0..=p.part.days {
                                let c = prorated_through(
                                    total, months, before, p.part, k, p.last, unit,
                                )
                                .unwrap();
                                assert!(
                                    c >= last_charge && c <= total,
                                    "{start} + {months}: {c} after {last_charge}"
                                );
                                last_charge = c;
                            }
                            before += part_weight::<i64>(p.part).unwrap();
                        }
                        assert_eq!(last_charge, total, "{start} + {months}");
                    }
                }
                spans += 1;
            }
            start = start.succ_opt().unwrap();
        }
        assert_eq!(spans, 731 * 3);
    }

    /// catch_up: everything due through the run's month when no run comes
    /// after it; never past a later month end already posted.
    #[test]
    fn a_catch_up_never_overtakes_a_posted_month_end() {
        // Jan run, Feb skipped, a Feb run with nothing after it: Feb's.
        assert_eq!(catch_up(&[200i64], &[100]), Some(100));
        // March already caught February up: a Feb run posts nothing.
        assert_eq!(catch_up(&[200i64, 300], &[100, 300]), Some(0));
        // A run carried past the schedule (an old tie cent): nothing.
        assert_eq!(catch_up(&[200i64], &[201]), Some(0));
        // January reversed after March ran: January posts it back.
        assert_eq!(catch_up(&[100i64, 200, 300], &[0, 100, 200]), Some(100));
        assert_eq!(catch_up::<i64>(&[], &[]), None);
        assert_eq!(catch_up(&[1i64], &[1, 2]), None);
    }

    /// catch_ups is catch_up of every suffix: positive and distinct ones
    /// (a catch_ups that wrote zeros would fail), and a month carried past
    /// the schedule (zero from it back).
    #[test]
    fn catch_ups_is_every_suffixs_catch_up() {
        let (s, p) = ([100i64, 200, 300, 400], [0i64, 100, 150, 250]);
        let mut out = [0i64; 4];
        catch_ups(&s, &p, &mut out).unwrap();
        for i in 0..4 {
            assert_eq!(Some(out[i]), catch_up(&s[i..], &p[i..]), "{i}");
        }
        assert_eq!(out, [100, 100, 150, 150]);
        let (s, p) = ([100i64, 200, 300, 400], [0i64, 300, 200, 400]);
        catch_ups(&s, &p, &mut out).unwrap();
        for i in 0..4 {
            assert_eq!(Some(out[i]), catch_up(&s[i..], &p[i..]), "{i}");
        }
        assert_eq!(out, [0, 0, 0, 0]);
        let mut none: [i64; 0] = [];
        assert_eq!(catch_ups::<i64>(&[], &[], &mut none), Some(()));
        assert_eq!(catch_ups(&s, &p, &mut [0i64; 3]), None);
    }

    /// true_up is the gap's columns.
    #[test]
    fn a_true_up_reaches_the_target() {
        assert_eq!(true_up(5_500i64, 5_000), Some((500, 0)));
        assert_eq!(true_up(5_500i64, 6_000), Some((0, 500)));
        assert_eq!(true_up(5_500i64, 5_500), Some((0, 0)));
    }

    /// basis_figures INVARIANTS 1-4, by hand: a 1,440.00 laptop the member
    /// holds at 1,688.40 (personal use): §723 1,688.40, depreciable
    /// 1,440.00, built-in loss 248.40; as business property, depreciable
    /// 1,688.40; a purchase at 900.00: depreciable 900.00 and nothing else.
    #[test]
    fn basis_figures_by_rule() {
        let (fmv, adj) = (144_000i64, 168_840i64);
        let p = basis_figures(BasisRule::PersonalUseConversion, fmv, adj, fmv).unwrap();
        assert_eq!(
            p,
            BasisFigures {
                tax_basis_723: Some(adj),
                depreciable_basis: fmv,
                built_in_gain_704c: Some(-24_840),
            }
        );
        let c = basis_figures(BasisRule::Carryover723, fmv, adj, fmv).unwrap();
        assert_eq!(c.depreciable_basis, adj);
        assert_eq!(c.built_in_gain_704c, Some(-24_840));
        // Value above basis: depreciable is the basis, a built-in gain.
        let g = basis_figures(BasisRule::PersonalUseConversion, 1000, 400, 1000).unwrap();
        assert_eq!(
            (g.depreciable_basis, g.built_in_gain_704c),
            (400, Some(600))
        );
        let b = basis_figures(BasisRule::Purchase, 90_000i64, 7, 8).unwrap();
        assert_eq!(
            b,
            BasisFigures {
                tax_basis_723: None,
                depreciable_basis: 90_000,
                built_in_gain_704c: None,
            }
        );
        assert_eq!(
            basis_figures(BasisRule::Carryover723, 0, i64::MAX, i64::MIN),
            None
        );
    }

    #[test]
    fn max_line_amount_is_one_quadrillion() {
        assert_eq!(MAX_LINE_AMOUNT, Decimal::from(1_000_000_000_000_000u64));
        assert!(MAX_LINE_AMOUNT.in_line_range());
        assert!(!(MAX_LINE_AMOUNT + Decimal::ONE).in_line_range());
        assert!("0.0000000001".parse::<Decimal>().unwrap().in_line_range());
        assert!(!"0.00000000001".parse::<Decimal>().unwrap().in_line_range());
        // Trailing zeros are not precision.
        assert!("1.000000000000".parse::<Decimal>().unwrap().in_line_range());
    }

    /// Regression (Kani `apportion_is_exact`, 2026-10-13): a shortfall of
    /// two units and a bit, with one part, was accepted and handed almost
    /// two units to that part. Spec scenario 1: more than one unit short
    /// per part is refused.
    #[test]
    fn apportion_refuses_a_shortfall_of_a_unit_per_part_or_more() {
        let mut parts = [0i64];
        assert_eq!(apportion(1805, &mut parts, &[0], 903), None);
        assert_eq!(parts, [0]);
        let mut parts = [0i64, 0];
        assert_eq!(apportion(1806, &mut parts, &[0, 0], 903), None);
        // Just under: accepted.
        let mut parts = [0i64, 0];
        assert_eq!(apportion(1805, &mut parts, &[1, 0], 903), Some(()));
        assert_eq!(parts, [1805, 0]);
    }

    /// The i64 model's line range: up to 2^40 either way, inclusive.
    #[test]
    fn i64_line_range_is_two_to_the_fortieth() {
        assert!((1i64 << 40).in_line_range());
        assert!((-(1i64 << 40)).in_line_range());
        assert!(!((1i64 << 40) + 1).in_line_range());
        assert!(!i64::MIN.in_line_range());
    }

    /// B-9: a reversal flips every side and keeps accounts and amounts, so
    /// with the original it nets every account to zero.
    #[test]
    fn a_reversal_flips_every_line_and_nets_to_zero() {
        let lines = [
            (1u8, Side::Debit, 70i64),
            (2, Side::Credit, 50),
            (3, Side::Credit, 20),
        ];
        let rev = reversal(&lines);
        assert_eq!(rev.len(), lines.len());
        for (o, r) in lines.iter().zip(&rev) {
            assert_eq!((r.0, r.1, r.2), (o.0, flip(o.1), o.2));
        }
        let both = movements(lines.iter().chain(&rev).copied()).unwrap();
        assert_eq!(both, vec![(1, 0), (2, 0), (3, 0)]);
    }

    /// Spec scenario 1's preconditions: parts and remainders pair up, and
    /// the unit is positive; otherwise nothing is apportioned.
    #[test]
    fn apportion_refuses_mismatched_inputs_or_a_nonpositive_unit() {
        let mut parts = [0i64, 0];
        assert_eq!(apportion(0, &mut parts, &[0], 1), None);
        assert_eq!(parts, [0, 0]);
        let mut none: [i64; 0] = [];
        assert_eq!(apportion(0, &mut none, &[], 0), None);
        assert_eq!(apportion(0, &mut none, &[], -1), None);
    }

    /// apportion_cumulative scenario 1: mismatched lengths, a bad unit, an
    /// input out of line range, or a shortfall apportion refuses: None,
    /// and the parts untouched.
    #[test]
    fn apportion_cumulative_refuses_mismatched_or_out_of_range_inputs() {
        let big = (1i64 << 40) + 1;
        for (total, parts, rems, unit, given) in [
            (4i64, [2i64, 2], [0i64, 0], 1i64, &[0i64][..]),
            (4, [2, 2], [0, 0], 0, &[0, 0][..]),
            (4, [2, 2], [0, 0], big, &[0, 0][..]),
            (big, [2, 2], [0, 0], 1, &[0, 0][..]),
            (4, [big, 2], [0, 0], 1, &[0, 0][..]),
            (4, [2, 2], [0, 0], 1, &[big, 0][..]),
            // Three units short of two parts: not rounding.
            (7, [2, 2], [0, 0], 1, &[0, 0][..]),
        ] {
            let mut p = parts;
            assert_eq!(
                apportion_cumulative(total, &mut p, &rems, unit, given),
                None,
                "{total} {parts:?} {unit} {given:?}"
            );
            assert_eq!(p, parts);
        }
        // Remainders that don't pair with the parts: apportion's refusal.
        let mut p = [2i64, 2];
        assert_eq!(apportion_cumulative(4, &mut p, &[0], 1, &[0, 0]), None);
        assert_eq!(p, [2, 2]);
    }

    /// apportion_cumulative scenario 2 (cents, i64): three cents closed
    /// one at a time gave A all three; the fourth cent's close apportions
    /// the running 4 cents (2 / 2) and takes back what A and B had.
    #[test]
    fn apportion_cumulative_trues_up_a_legacy_split() {
        let mut parts = [2i64, 2];
        apportion_cumulative(4, &mut parts, &[0, 0], 1, &[3, 0]).unwrap();
        assert_eq!(parts, [-1, 2]);
        // A running total of 5: floors 2 / 2 with equal remainders, so the
        // tied unit goes to the first (3 / 2), less what each had (3 / 1).
        let mut parts = [2i64, 2];
        apportion_cumulative(5, &mut parts, &[5, 5], 1, &[3, 1]).unwrap();
        assert_eq!(parts, [0, 1]);
    }

    /// With no parts, only a zero whole is fully apportioned.
    #[test]
    fn apportion_to_no_parts_accepts_only_zero() {
        let mut none: [i64; 0] = [];
        assert_eq!(apportion(0, &mut none, &[], 1), Some(()));
        assert_eq!(apportion(5, &mut none, &[], 1), None);
    }

    /// A zero movement's columns are both a plain zero, never "-0" (found
    /// by mutation triage, 2026-10-13: it returned a negative-zero credit).
    #[test]
    fn a_zero_movement_has_no_negative_zero_column() {
        let (d, c) = columns(Decimal::ZERO).unwrap();
        assert_eq!(
            (d.to_string(), c.to_string()),
            ("0".to_string(), "0".to_string())
        );
    }

    #[test]
    fn credit_line_sides() {
        assert_eq!(credit_line(5i64), Some((Side::Credit, 5)));
        assert_eq!(credit_line(-5i64), Some((Side::Debit, 5)));
        assert_eq!(credit_line(0i64), None);
    }
}
