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
//!   it counts as money or as property never exceeds it (B-35).

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
pub fn entry_flows<T: Amount>(lines: &[(Flow, Side, T)]) -> Option<EntryFlows<T>> {
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
