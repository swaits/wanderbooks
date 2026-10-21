//! Party shares: an entry with more than one party, each with their part of
//! what it moved.
//!
//! An entry names its counterparties as a list of [`PartyShare`]s. Most
//! name one party, who had all of it. A joint transaction names several:
//! the two members of a partnership reimbursed together, a payment split
//! between two contractors, a contribution from both members' joint funds.
//! Each share is an exact amount, a percentage, or derived from the entry
//! itself ([`Share::Derived`]).
//!
//! ```text
//! GOAL: "what did we pay each of them, and what did we get from each?"
//!       right when one entry was with several parties, with apportioning
//!       the human controls exactly, every part signed as the party's own
//!       account would show it, and a reversal always the opposite of what
//!       it reverses.
//! SOURCE: DECISION (owner, 2026-10-15): no "joint" party; a joint
//!         transaction is with both members, each with their share, and
//!         shares can be apportioned explicitly ("do you allow for
//!         controlled apportioning?" Yes). docs/parties-and-attachments.md
//!         "Shares". rules.md B-35 (the sign, direction and money of a
//!         part; DECISION, owner, 2026-10-20: a member's entry counts what
//!         moved their capital).
//! DEFINITIONS:
//!   target (target(), rules.md B-35 item 3): what the shares split,
//!     signed debit-positive as a line on the parties' own account would
//!     be (positive: the company paid or gave; negative: it received),
//!     with its direction (paid, received, neither) and what of it moved
//!     as money (cash) and as property (in_kind), in this order of cases:
//!     1. A period's closing entry: the parties' own-account movement (the
//!        members' equity, when every party named is a member), else its
//!        total debits; neither: a close moves nothing with anyone.
//!     2. A reversal: its original's target negated, paid and received
//!        swapped (kernel::reverse_target), from its root original down;
//!        never read from its own lines (parties::entry_flow isn't odd
//!        for one liability paying down another).
//!     3. Every party named has the member role in the directory and the
//!        entry moves equity other than retained earnings (E): E, received
//!        when a contribution (E < 0), paid when a distribution (E > 0).
//!        Of a contribution, in_kind is the property it brought in (asset
//!        accounts neither cash nor receivables: the K-1 worksheet's test,
//!        k1 INVARIANT 3), at most |E|, and cash the rest (what the K-1
//!        counts as cash: money, or what a member paid for the company).
//!        Of a distribution, cash is the money it paid (case 4's net, at
//!        most E) and in_kind the property it gave up, at most what is
//!        left; the rest, credited to a liability that isn't money for the
//!        entry (distributions payable, or what the members are owed when
//!        they are named its creditors), is still owed and counts as
//!        nothing until it is paid (kernel::party_target INVARIANT 2).
//!     4. Money moved (parties::entry_flow net, a liability owed to a
//!        party named not counted as money: MoneyAccounts::for_parties):
//!        the net, paid (+) or received (−), all of it cash.
//!     5. Otherwise the movement of the parties' own accounts (payables,
//!        receivables, liabilities owed to a party named: a bill −, an
//!        invoice +, a fee on the issuer's own card −), else the total
//!        debits; neither.
//!   magnitude = |target.amount|.
//!   A member (PartyMember): a party, income share, capital account,
//!     distributions account, passed in by the caller from the company's
//!     settings ([[company.members]]); core doesn't read the settings. Only
//!     a derived split among several members reads them (INVARIANT 10),
//!     and the server journals that split as amounts once the entry is
//!     posted, so posted history never depends on today's settings.
//! INVARIANTS (of an entry's shares, checked by check_shares, journaled):
//!   1. No party appears twice; every party is in the directory.
//!   2. All shares are one kind: all Amount, all Percent, or all Derived.
//!   3. An Amount is positive with at most two decimal places (cents).
//!   4. A Percent is positive, at most 100, and the percents sum to exactly
//!      100.
//!   5. At most one party has a vendor record: a 1099 counts a payment
//!      toward a single vendor.
//! INVARIANTS (of split and resolve, against the entry's lines, the
//! target and the members):
//!   6. The parts sum to target.amount exactly, and each has its sign (a
//!      zero part is a plain zero, never "-0").
//!   7. Amount: each party gets sign × its amount; refused unless the
//!      amounts sum to magnitude.
//!   8. Percent: each party gets magnitude × percent / 100 rounded toward
//!      zero to the cent; the cents left over (fewer than one per party)
//!      go to the first party in order. Then the sign.
//!   9. Derived, one party: it gets target.amount.
//!  10. Derived, several parties: every one must be a member (else
//!      refused: give amounts or percents). If the entry has a line on any
//!      named member's capital or distributions account, each member gets
//!      |credits − debits| on their own two accounts, and those must sum
//!      to magnitude (else refused). Otherwise each gets their income share
//!      of magnitude, the shares rescaled to the members named, rounded as
//!      in 8 (remainder to the first).
//!  11. A reversal's parts are its original's negated: its target is the
//!      original's negated, and its lines are the original's with every
//!      side flipped (Ledger::reverse_entry), so every magnitude above is
//!      the same.
//!  12. Each part's cash and in_kind have its sign, |cash| + |in_kind| ≤
//!      |part|, and over the parts they sum to the target's cash and
//!      in_kind exactly: in_kind is shared by each part's size, cash by
//!      what is left of each (k1's share_property: in proportion, to the
//!      cent, leftover cents by largest remainder, ties to the party
//!      listed first in the company's members), so a member's in-kind part
//!      of a joint contribution is the property the K-1 worksheet counts
//!      for them.
//! SCENARIOS:
//!   1. Reimbursing both members $100.00 (Dr 2100 / Cr 1000), both named,
//!      derived, 50/50: $50.00 each, paid, cash.
//!   2. A joint contribution (Dr 1000 300 / Cr 3000 200 / Cr 3001 100):
//!      derived from the capital lines, Tobin −200.00, Morgan −100.00,
//!      received, cash.
//!   3. A property contribution (Dr Equipment 1000 / Cr 3000 500 / Cr 3001
//!      500): −500.00 each, received, in kind; its reversal +500.00 each,
//!      paid, in kind.
//!   4. $100.00 split 1/3 each three ways by percent 33.33, 33.33, 33.34:
//!      33.33, 33.33, 33.34. By 33.333/33.333/33.334: 33.34, 33.33, 33.33
//!      (the leftover cent to the first).
//!   5. Amounts of $60.00 and $30.00 on a $100.00 payment: refused.
//!   6. A bill for $80.00 tagged with its vendor (Dr expense / Cr
//!      payables): −80.00, neither; its reversal +80.00, neither.
//!   7. A joint contribution of cash and property (Dr 1000 600, Dr 1500
//!      400 / Cr 3000 500, Cr 3001 500): −500.00 each, received, of it
//!      −300.00 cash and −200.00 in kind (the K-1 worksheet's SCENARIO 3).
//!   8. A distribution declared to one member (Dr 3000 200 / Cr
//!      Distributions Payable 200): +200.00, neither, nothing counted; its
//!      payment (Dr Distributions Payable 200 / Cr 1000 200) is the 200.00
//!      paid, once.
//!   9. Equipment carried at 1,000 less 300 depreciated, and 100 in cash,
//!      distributed to one member (Dr 3000 800, Dr accumulated
//!      depreciation 300 / Cr equipment 1000, Cr 1000 100): +800.00, paid,
//!      of it 100.00 cash and 700.00 in kind.
//! ```

pub use crate::kernel::Direction;
use crate::kernel::{self, PartyTarget};
use crate::{AccountId, AccountType, EntryId, JournalEntry, Ledger, LedgerError, PartyId, Side};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// What part of an entry one party had.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Share {
    /// Exactly this much of what the entry moved (a positive amount, in
    /// cents; the sign follows the entry).
    Amount(Decimal),
    /// This percent of what the entry moved (0 < p ≤ 100).
    Percent(Decimal),
    /// From the entry itself: all of it for a sole party; for members,
    /// their capital lines or their income share (see the module's
    /// INVARIANT 10).
    Derived,
}

/// One party named on an entry, with their share.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartyShare {
    /// The party.
    #[serde(rename = "party_id")]
    pub party: PartyId,
    /// Their share.
    pub share: Share,
}

impl PartyShare {
    /// The sole party, with all of it.
    pub fn whole(party: PartyId) -> Self {
        PartyShare {
            party,
            share: Share::Derived,
        }
    }
}

/// A member of the company as shares see them: their party, their share of
/// income (a fraction, e.g. 0.50), and their capital and distributions
/// accounts. Built by the caller from the settings' members.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PartyMember {
    /// The member's party in the directory.
    pub party: PartyId,
    /// Their share of income, as a fraction (the members' sum to 1).
    pub share: Decimal,
    /// Their capital account.
    pub capital: AccountId,
    /// Their distributions account, if they have one.
    pub distributions: Option<AccountId>,
}

/// The structural checks on an entry's shares (the module's INVARIANTS 1–4;
/// 5 needs the directory and is [`crate::Ledger::check_set_entry_parties`]'s).
/// `known` says whether a party is in the directory. Returns why not.
///
/// For new shares: [`check_shares_journaled`], and no amount larger than a
/// line may be (kernel::MAX_LINE_AMOUNT; P0 review round 3, S1). Replay
/// checks only [`check_shares_journaled`] (rules.md B-28).
pub fn check_shares(shares: &[PartyShare], known: impl Fn(PartyId) -> bool) -> Result<(), String> {
    check_shares_journaled(shares, known)?;
    for s in shares {
        if let Share::Amount(a) = s.share
            && a > crate::kernel::MAX_LINE_AMOUNT
        {
            return Err(format!(
                "party {}'s amount {a} is larger than any entry may move (at most one quadrillion)",
                s.party
            ));
        }
    }
    Ok(())
}

/// The checks every journaled `entry.parties_set` passed when it was
/// written (the rule through 2026-10-16): [`check_shares`] without the
/// amount's upper bound. Replay applies exactly these, so books an earlier
/// version wrote always open (rules.md B-28).
pub fn check_shares_journaled(
    shares: &[PartyShare],
    known: impl Fn(PartyId) -> bool,
) -> Result<(), String> {
    let mut seen = std::collections::BTreeSet::new();
    for s in shares {
        if !known(s.party) {
            return Err(format!("unknown party {}", s.party));
        }
        if !seen.insert(s.party) {
            return Err(format!("party {} is named twice", s.party));
        }
    }
    let kind = |s: &Share| match s {
        Share::Amount(_) => 0,
        Share::Percent(_) => 1,
        Share::Derived => 2,
    };
    if let Some(first) = shares.first()
        && shares.iter().any(|s| kind(&s.share) != kind(&first.share))
    {
        return Err(
            "give every party the same kind of share: all amounts, all percents, or none (derived)"
                .into(),
        );
    }
    let mut percent = Decimal::ZERO;
    let mut any_percent = false;
    for s in shares {
        match s.share {
            Share::Amount(a) => {
                if a <= Decimal::ZERO {
                    return Err(format!("party {}'s amount must be positive", s.party));
                }
                if a.normalize().scale() > 2 {
                    return Err(format!(
                        "party {}'s amount {a} is finer than a cent",
                        s.party
                    ));
                }
            }
            Share::Percent(p) => {
                if p <= Decimal::ZERO || p > Decimal::ONE_HUNDRED {
                    return Err(format!(
                        "party {}'s percent must be more than 0 and at most 100",
                        s.party
                    ));
                }
                any_percent = true;
                percent += p;
            }
            Share::Derived => {}
        }
    }
    if any_percent && percent != Decimal::ONE_HUNDRED {
        return Err(format!("the percents sum to {percent}, not 100"));
    }
    Ok(())
}

/// The refusal of a sum past exact arithmetic.
fn too_large(what: String) -> crate::LedgerError {
    crate::LedgerError::TooLargeToCompute(what)
}

/// The sum of `values`, or None past exact arithmetic.
fn checked_sum(values: impl IntoIterator<Item = Decimal>) -> Option<Decimal> {
    values
        .into_iter()
        .try_fold(Decimal::ZERO, |s, v| s.checked_add(v))
}

/// Split `magnitude` (non-negative) by `weights` (positive, any sum): each
/// part rounded toward zero to the cent, the cents left over to the first.
/// None when a product or sum is past exact arithmetic.
fn split_first(magnitude: Decimal, weights: &[Decimal]) -> Option<Vec<Decimal>> {
    use rust_decimal::RoundingStrategy::ToZero;
    let total = checked_sum(weights.iter().copied())?;
    if weights.is_empty() || total.is_zero() {
        return Some(vec![Decimal::ZERO; weights.len()]);
    }
    let mut parts: Vec<Decimal> = weights
        .iter()
        .map(|w| {
            magnitude
                .checked_mul(*w)
                .and_then(|x| x.checked_div(total))
                .map(|x| x.round_dp_with_strategy(2, ToZero))
        })
        .collect::<Option<_>>()?;
    let left = magnitude.checked_sub(checked_sum(parts.iter().copied())?)?;
    parts[0] = parts[0].checked_add(left)?;
    Some(parts)
}

/// What an entry's shares split, and what of it counts (the module's
/// DEFINITIONS: target; rules.md B-35).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Target {
    /// The whole, signed debit-positive as a line on the parties' own
    /// account would be.
    pub amount: Decimal,
    /// Which way value moved with the parties.
    pub direction: Direction,
    /// What of `amount` moved as money: counted on both bases.
    pub cash: Decimal,
    /// What of `amount` moved as property: counted on basis "all".
    pub in_kind: Decimal,
    /// Which side the parts count on in a party's totals: the direction
    /// of the entry's root original (itself, unless it is a reversal), so
    /// a reversal nets what it reverses (B-35 item 4).
    pub counts_as: Direction,
}

/// One party's part of an entry (the module's INVARIANTS 6-12).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Part {
    /// The party.
    pub party: PartyId,
    /// Its part of the target, with the target's sign.
    pub amount: Decimal,
    /// What of it moved as money.
    pub cash: Decimal,
    /// What of it moved as property.
    pub in_kind: Decimal,
}

/// An entry's target and each party's part of it ([`resolve`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    /// What the shares split.
    pub target: Target,
    /// Each party's part, in the shares' order.
    pub parts: Vec<Part>,
}

impl Resolved {
    /// The part of `party`, when the entry names it.
    pub fn part(&self, party: PartyId) -> Option<&Part> {
        self.parts.iter().find(|p| p.party == party)
    }
}

/// What entry `id`'s shares split (the module's DEFINITIONS: target).
/// `shares` are passed, not read from the ledger, so a proposed set can be
/// checked; they cover the whole reversal chain (Ledger::set_entry_parties
/// INVARIANT 5). Refused (TooLargeToCompute), never a panic, when a sum is
/// past exact arithmetic: a draft's lines are as journaled, of any size
/// (rules.md B-28).
pub fn target(
    ledger: &Ledger,
    id: EntryId,
    shares: &[PartyShare],
    money: &crate::parties::MoneyAccounts,
) -> Result<Target, LedgerError> {
    let entry = ledger.get_entry(id).ok_or(LedgerError::UnknownEntry(id))?;
    let (root, depth) = crate::parties::root_original(ledger, entry);
    let base = original_target(ledger, root, shares, money)?;
    let t = if depth % 2 == 1 {
        kernel::reverse_target(base)
            .ok_or_else(|| too_large(format!("entry {id}'s reversal target")))?
    } else {
        base
    };
    Ok(Target {
        amount: t.amount,
        direction: t.direction,
        cash: t.cash,
        in_kind: t.in_kind,
        counts_as: base.direction,
    })
}

/// The target of an entry that reverses none (the module's DEFINITIONS:
/// target, cases 1 and 3-5).
fn original_target(
    ledger: &Ledger,
    entry: &JournalEntry,
    shares: &[PartyShare],
    money: &crate::parties::MoneyAccounts,
) -> Result<PartyTarget<Decimal>, LedgerError> {
    let past_exact = || too_large(format!("entry {}'s target", entry.id()));
    let named: Vec<PartyId> = shares.iter().map(|s| s.party).collect();
    let money = money.for_parties(ledger, &named);
    let parties: Vec<&crate::Party> = named.iter().filter_map(|p| ledger.get_party(*p)).collect();
    let members_only = !named.is_empty()
        && parties.len() == named.len()
        && parties
            .iter()
            .all(|p| p.roles.contains(&crate::Role::Member));
    let owed = |a: AccountId| parties.iter().any(|p| p.creditor_of.contains(&a));
    let retained = ledger.role_account(&crate::RETAINED_EARNINGS);
    let receivables = ledger.role_account(&crate::RECEIVABLES);
    let payables = ledger.role_account(&crate::PAYABLES);
    let kind = |a: AccountId| ledger.chart().get(a).map(|x| x.account_type);
    // Property: the K-1 worksheet's test (k1 INVARIANT 3).
    let property = |a: AccountId| {
        kind(a) == Some(AccountType::Asset) && !money.cash.contains(&a) && Some(a) != receivables
    };
    let add = |t: &mut Decimal, v: Decimal| -> Result<(), LedgerError> {
        *t = t.checked_add(v).ok_or_else(past_exact)?;
        Ok(())
    };
    let zero = Decimal::ZERO;
    let (mut equity, mut prop_in, mut prop_out, mut own, mut debits) =
        (zero, zero, zero, zero, zero);
    for l in entry.lines() {
        let v = l.amount.amount;
        let signed = kernel::signed(l.side, v).ok_or_else(past_exact)?;
        let k = kind(l.account);
        if k == Some(AccountType::Equity) && Some(l.account) != retained {
            add(&mut equity, signed)?;
        }
        if property(l.account) {
            match l.side {
                Side::Debit => add(&mut prop_in, v)?,
                Side::Credit => add(&mut prop_out, v)?,
            }
        }
        let theirs = k == Some(AccountType::Liability) && owed(l.account);
        if theirs || Some(l.account) == payables || Some(l.account) == receivables {
            add(&mut own, signed)?;
        }
        if l.side == Side::Debit {
            add(&mut debits, v)?;
        }
    }
    let property = if equity < zero { prop_in } else { prop_out };
    let capital = members_only.then_some((equity, property));
    if ledger.is_closing_entry(entry.id()) {
        let t = kernel::party_target(capital, zero, own, debits).ok_or_else(past_exact)?;
        return Ok(PartyTarget {
            amount: t.amount,
            direction: Direction::Neither,
            cash: zero,
            in_kind: zero,
        });
    }
    let net = crate::parties::entry_flow(entry, &money)?.net;
    kernel::party_target(capital, net, own, debits).ok_or_else(past_exact)
}

/// Split `whole` (at most the sum of `sizes`) over `sizes` in proportion,
/// to the cent, leftover cents by largest remainder (k1's
/// share_property), ties to the earlier in `order` (each part's rank).
/// None past exact arithmetic.
fn share_by(whole: Decimal, sizes: &[Decimal], order: &[usize]) -> Option<Vec<Decimal>> {
    let ranked: Vec<Decimal> = order.iter().map(|&i| sizes[i]).collect();
    let shared = crate::k1::share_property(whole, &ranked, 2).ok()?;
    let mut out = vec![Decimal::ZERO; sizes.len()];
    for (k, &i) in order.iter().enumerate() {
        out[i] = shared[k];
    }
    Some(out)
}

/// Each party's part of `target` (the module's INVARIANTS 6-12), in the
/// shares' order. Refused, with why, when the shares can't resolve against
/// these lines and members.
pub fn split(
    entry: &JournalEntry,
    shares: &[PartyShare],
    target: &Target,
    members: &[PartyMember],
) -> Result<Vec<Part>, String> {
    let past_exact = || too_large(format!("entry {}'s shares", entry.id())).to_string();
    let magnitude = target.amount.abs();
    let Some(first) = shares.first() else {
        return Ok(Vec::new());
    };
    let sizes: Vec<Decimal> = match first.share {
        Share::Amount(_) => {
            let amounts: Vec<Decimal> = shares
                .iter()
                .map(|s| match s.share {
                    Share::Amount(a) => a,
                    _ => Decimal::ZERO,
                })
                .collect();
            let sum = checked_sum(amounts.iter().copied()).ok_or_else(past_exact)?;
            if sum != magnitude {
                return Err(format!(
                    "the parties' amounts sum to {sum}, but entry {} moved {magnitude}",
                    entry.id()
                ));
            }
            amounts
        }
        Share::Percent(_) => {
            let weights: Vec<Decimal> = shares
                .iter()
                .map(|s| match s.share {
                    Share::Percent(p) => p,
                    _ => Decimal::ZERO,
                })
                .collect();
            split_first(magnitude, &weights).ok_or_else(past_exact)?
        }
        Share::Derived if shares.len() == 1 => vec![magnitude],
        Share::Derived => {
            let named: Vec<&PartyMember> = shares
                .iter()
                .map(|s| {
                    members.iter().find(|m| m.party == s.party).ok_or_else(|| {
                        format!(
                            "party {} isn't a member, so its share can't be derived: give each party an amount or a percent",
                            s.party
                        )
                    })
                })
                .collect::<Result<_, _>>()?;
            let on = |m: &PartyMember, a: AccountId| a == m.capital || Some(a) == m.distributions;
            let touches = entry
                .lines()
                .iter()
                .any(|l| named.iter().any(|m| on(m, l.account)));
            if touches {
                let parts: Vec<Decimal> = named
                    .iter()
                    .map(|m| {
                        checked_sum(entry.lines().iter().filter(|l| on(m, l.account)).map(|l| {
                            match l.side {
                                Side::Credit => l.amount.amount,
                                Side::Debit => -l.amount.amount,
                            }
                        }))
                        .map(|s| s.abs())
                    })
                    .collect::<Option<_>>()
                    .ok_or_else(past_exact)?;
                let sum = checked_sum(parts.iter().copied()).ok_or_else(past_exact)?;
                if sum != magnitude {
                    return Err(format!(
                        "the members' capital lines on entry {} come to {sum}, but it moved {magnitude}: give each member an amount",
                        entry.id()
                    ));
                }
                parts
            } else {
                let weights: Vec<Decimal> = named.iter().map(|m| m.share).collect();
                if weights.iter().any(|w| *w <= Decimal::ZERO) {
                    return Err(
                        "a member named has no income share: give each party an amount or a percent"
                            .into(),
                    );
                }
                split_first(magnitude, &weights).ok_or_else(past_exact)?
            }
        }
    };
    // Ties go to the party listed first in the company's members, as on
    // the K-1 worksheet; other parties after them, in the shares' order.
    let mut order: Vec<usize> = (0..shares.len()).collect();
    order.sort_by_key(|&i| {
        members
            .iter()
            .position(|m| m.party == shares[i].party)
            .unwrap_or(members.len() + i)
    });
    let (cash, in_kind) = (target.cash.abs(), target.in_kind.abs());
    let in_kind_sizes = if in_kind.is_zero() {
        vec![Decimal::ZERO; sizes.len()]
    } else {
        share_by(in_kind, &sizes, &order).ok_or_else(past_exact)?
    };
    let left: Vec<Decimal> = sizes
        .iter()
        .zip(&in_kind_sizes)
        .map(|(s, k)| s.checked_sub(*k))
        .collect::<Option<_>>()
        .ok_or_else(past_exact)?;
    let cash_sizes = if cash == checked_sum(left.iter().copied()).ok_or_else(past_exact)? {
        left
    } else if cash.is_zero() {
        vec![Decimal::ZERO; sizes.len()]
    } else {
        share_by(cash, &left, &order).ok_or_else(past_exact)?
    };
    let negative = target.amount < Decimal::ZERO;
    let signed = |v: Decimal| if negative && !v.is_zero() { -v } else { v };
    Ok(shares
        .iter()
        .enumerate()
        .map(|(i, s)| Part {
            party: s.party,
            amount: signed(sizes[i]),
            cash: signed(cash_sizes[i]),
            in_kind: signed(in_kind_sizes[i]),
        })
        .collect())
}

/// Entry `id`'s target and each party's exact part of it (the module's
/// INVARIANTS 6-12), in the shares' order. Refused, with why, when the
/// shares can't resolve against these lines and members.
pub fn resolve(
    ledger: &Ledger,
    id: EntryId,
    shares: &[PartyShare],
    money: &crate::parties::MoneyAccounts,
    members: &[PartyMember],
) -> Result<Resolved, String> {
    let entry = ledger
        .get_entry(id)
        .ok_or_else(|| LedgerError::UnknownEntry(id).to_string())?;
    let target = target(ledger, id, shares, money).map_err(|e| e.to_string())?;
    let parts = split(entry, shares, &target, members)?;
    Ok(Resolved { target, parts })
}
