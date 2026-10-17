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
//! GOAL: "what did we pay each of them?" right when one entry was with
//!       several parties, with apportioning the human controls exactly.
//! SOURCE: DECISION (owner, 2026-10-15): no "joint" party; a joint
//!         transaction is with both members, each with their share, and
//!         shares can be apportioned explicitly ("do you allow for
//!         controlled apportioning?" Yes). docs/parties-and-attachments.md
//!         "Shares".
//! DEFINITIONS:
//!   flow: crate::parties::entry_flow(entry, money).
//!   target: flow.net when it is not zero, else the entry's total debits
//!     (a member contributing property moves no money but is still all
//!     theirs). magnitude = |target|; sign = the sign of target (positive
//!     when net is zero).
//!   A member: a PartyMember (party, income share, capital account,
//!     distributions account), passed in by the caller from the company's
//!     settings ([[company.members]]); core doesn't read the settings.
//! INVARIANTS (of an entry's shares, checked by check_shares, journaled):
//!   1. No party appears twice; every party is in the directory.
//!   2. All shares are one kind: all Amount, all Percent, or all Derived.
//!   3. An Amount is positive with at most two decimal places (cents).
//!   4. A Percent is positive, at most 100, and the percents sum to exactly
//!      100.
//!   5. At most one party has a vendor record: a 1099 counts a payment
//!      toward a single vendor.
//! INVARIANTS (of resolve, against the entry's lines and the members):
//!   6. Resolved amounts sum to target exactly, and each has target's sign.
//!   7. Amount: each party gets sign × its amount; refused unless the
//!      amounts sum to magnitude.
//!   8. Percent: each party gets magnitude × percent / 100 rounded toward
//!      zero to the cent; the cents left over (fewer than one per party)
//!      go to the first party in order. Then the sign.
//!   9. Derived, one party: it gets target.
//!  10. Derived, several parties: every one must be a member (else
//!      refused: give amounts or percents). If the entry has a line on any
//!      named member's capital or distributions account, each member gets
//!      |credits − debits| on their own two accounts, and those must sum
//!      to magnitude (else refused). Otherwise each gets their income share
//!      of magnitude, the shares rescaled to the members named, rounded as
//!      in 8 (remainder to the first).
//! SCENARIOS:
//!   1. Reimbursing both members $100.00 (Dr 2100 / Cr 1000), both named,
//!      derived, 50/50: $50.00 each.
//!   2. A joint contribution (Dr 1000 300 / Cr 3000 200 / Cr 3001 100):
//!      derived from the capital lines, Tobin −200.00, Morgan −100.00
//!      (received).
//!   3. A property contribution (Dr Equipment 1000 / Cr 3000 500 / Cr 3001
//!      500): net is zero; target is the total, 500.00 each.
//!   4. $100.00 split 1/3 each three ways by percent 33.33, 33.33, 33.34:
//!      33.33, 33.33, 33.34. By 33.333/33.333/33.334: 33.34, 33.33, 33.33
//!      (the leftover cent to the first).
//!   5. Amounts of $60.00 and $30.00 on a $100.00 payment: refused.
//! ```

use crate::{AccountId, JournalEntry, PartyId, Side};
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

/// What the shares are split over: the entry's flow net, else its total
/// debits (the module's DEFINITIONS).
/// Refused (TooLargeToCompute), never a panic, when a sum is past exact
/// arithmetic: a draft's lines and an entry's shares are as journaled, of
/// any size (rules.md B-28).
pub fn share_target(
    entry: &JournalEntry,
    money: &crate::parties::MoneyAccounts,
) -> Result<Decimal, crate::LedgerError> {
    let net = crate::parties::entry_flow(entry, money)?.net;
    if net.is_zero() {
        entry
            .lines()
            .iter()
            .filter(|l| l.side == Side::Debit)
            .try_fold(Decimal::ZERO, |s, l| s.checked_add(l.amount.amount))
            .ok_or_else(|| too_large(format!("entry {}'s debits", entry.id())))
    } else {
        Ok(net)
    }
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

/// Each party's exact part of what `entry` moved (the module's INVARIANTS
/// 6–10), in the shares' order. Refused, with why, when the shares can't
/// resolve against these lines and members.
pub fn resolve(
    entry: &JournalEntry,
    shares: &[PartyShare],
    money: &crate::parties::MoneyAccounts,
    members: &[PartyMember],
) -> Result<Vec<(PartyId, Decimal)>, String> {
    let target = share_target(entry, money).map_err(|e| e.to_string())?;
    let past_exact = || too_large(format!("entry {}'s shares", entry.id())).to_string();
    let magnitude = target.abs();
    let signed = |v: Decimal| if target.is_sign_negative() { -v } else { v };
    let Some(first) = shares.first() else {
        return Ok(Vec::new());
    };
    let parts: Vec<Decimal> = match first.share {
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
    Ok(shares
        .iter()
        .zip(parts)
        .map(|(s, v)| (s.party, signed(v)))
        .collect())
}
