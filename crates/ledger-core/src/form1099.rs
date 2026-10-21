//! 1099 tracking: what was paid to each vendor in a calendar year.
//!
//! A vendor is marked with the information return it gets (1099-NEC for
//! nonemployee compensation, 1099-MISC for rents and the like) and whether
//! its W-9 is on file. Entries are tagged with the vendor they paid
//! (`pay_bill` tags automatically). The amount paid is what left the
//! company's payment sources on the vendor's tagged entries: credits minus
//! debits to them ([`PaymentAccounts`]). The sources are the cash accounts
//! (bank, cash on hand) and the financing liabilities (what owners or
//! lenders paid on the company's behalf, e.g. Due to Shareholders): the
//! company is the payer either way. A card payment (Dr expense, Cr credit
//! card) leaves neither, so it is excluded, as it should be: card payments
//! are reported by the card processor on 1099-K, not by the payer. A bill
//! (Cr payables) is not a payment until it is paid, and a prepayment
//! (Dr prepaid, Cr cash) counts in full when made: §6041 reports payments
//! made in the year. An entry with several parties (a payment shared
//! between a contractor and a member) counts only the vendor's share of
//! that payment ([`vendor_1099_report_with_members`]).
//!
//! Thresholds are set by law and change; [`default_threshold`] encodes
//! them and the report takes an override. Confirm with the CPA.

use crate::{AccountType, Ledger, LedgerError, VendorId};
use chrono::Datelike;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// The information return a vendor gets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Form1099 {
    /// 1099-NEC: nonemployee compensation (contractors, attorneys' fees).
    Nec,
    /// 1099-MISC: rents, royalties, prizes, other income.
    Misc,
}

impl Form1099 {
    /// Parse "nec", "1099-NEC", "misc", "1099-MISC".
    pub fn parse(s: &str) -> Option<Form1099> {
        let s = s.trim().to_lowercase();
        match s.strip_prefix("1099-").unwrap_or(&s) {
            "nec" => Some(Form1099::Nec),
            "misc" => Some(Form1099::Misc),
            _ => None,
        }
    }

    /// "1099-NEC" or "1099-MISC".
    pub fn label(self) -> &'static str {
        match self {
            Form1099::Nec => "1099-NEC",
            Form1099::Misc => "1099-MISC",
        }
    }
}

/// The reporting threshold for payments made in `year`.
///
/// ```text
/// GOAL: the dollar amount at which a vendor's payments become reportable.
/// SOURCE: docs/spec/rules.md B-19 (26 U.S.C. §6041(a), (h), §6041A(a)(2),
///         Pub. L. 119-21 §70433; sources/irs-1099-thresholds.md).
/// SCENARIOS:
///   1. IF `year` <= 2025: SHOULD be $600.
///   2. IF `year` == 2026: SHOULD be $2,000.
///   3. IF `year` > rules::THRESHOLD_1099.known_through:
///      - SHOULD refuse with RuleNotKnown
///      - NEVER guess $2,000
///      BECAUSE the amount is indexed for inflation after 2026 and not
///      yet known (owner, 2026-10-13: never apply stale rules).
/// ```
///
/// Royalties (1099-MISC, $10) and attorneys' gross proceeds ($600) are not
/// modeled. Pass an explicit threshold to [`vendor_1099_report`] to
/// override.
pub fn default_threshold(year: i32) -> Result<Decimal, LedgerError> {
    let rule = crate::rules::THRESHOLD_1099;
    if year > rule.known_through {
        return Err(LedgerError::RuleNotKnown {
            rule: rule.name.to_string(),
            year,
            known_through: rule.known_through,
            authority: rule.authority.to_string(),
        });
    }
    Ok(if year <= 2025 {
        Decimal::new(600, 0)
    } else {
        Decimal::new(2000, 0)
    })
}

/// One vendor's year.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Vendor1099Line {
    /// The vendor.
    pub vendor_id: VendorId,
    /// Name.
    pub name: String,
    /// The return it gets, if marked.
    pub form: Option<Form1099>,
    /// W-9 on file.
    pub tin_on_file: bool,
    /// Cash paid in the year (net of refunds and reversals).
    pub paid: Decimal,
    /// Marked for a 1099 and paid at least the threshold.
    pub reportable: bool,
}

/// The year's 1099 report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Vendor1099Report {
    /// Calendar year.
    pub year: i32,
    /// The threshold applied.
    pub threshold: Decimal,
    /// Every vendor marked for a 1099 or paid in the year, by name.
    pub vendors: Vec<Vendor1099Line>,
    /// Entries naming several parties whose shares didn't resolve, in
    /// posting order: each counted in full toward its vendor until its
    /// shares are fixed ([`vendor_1099_report_with_members`] INVARIANT 4).
    pub unresolved: Vec<crate::EntryId>,
}

/// Cash paid per vendor in calendar `year`, from posted entries tagged
/// with the vendor.
pub fn vendor_1099_report(
    ledger: &Ledger,
    year: i32,
    threshold: Option<Decimal>,
) -> Result<Vendor1099Report, LedgerError> {
    vendor_1099_report_with(ledger, year, threshold, &PaymentAccounts::default())
}

/// The accounts payments to vendors come out of, for
/// [`vendor_1099_report_with`]: the books' `[books] cash_accounts` and
/// `[books] financing_accounts`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PaymentAccounts {
    /// Cash accounts: bank, savings, cash on hand.
    pub cash: Vec<crate::AccountId>,
    /// Financing liabilities: owner advances, loans (paid on the company's
    /// behalf). Never a payment card.
    pub financing: Vec<crate::AccountId>,
}

impl PaymentAccounts {
    /// Whether `account` is a payment source. With no cash accounts given,
    /// every asset account is one (books that haven't named their cash).
    fn includes(&self, ledger: &Ledger, account: crate::AccountId) -> bool {
        if self.cash.is_empty() {
            ledger
                .chart()
                .get(account)
                .is_some_and(|a| a.account_type == AccountType::Asset)
                || self.financing.contains(&account)
        } else {
            self.cash.contains(&account) || self.financing.contains(&account)
        }
    }
}

/// [`vendor_1099_report`], counting payments out of `sources`, with no
/// members: [`vendor_1099_report_with_members`] with none (an entry naming
/// several parties then needs amount or percent shares to resolve, which
/// a vendor beside anyone always does, since a vendor is not a member).
pub fn vendor_1099_report_with(
    ledger: &Ledger,
    year: i32,
    threshold: Option<Decimal>,
    sources: &PaymentAccounts,
) -> Result<Vendor1099Report, LedgerError> {
    vendor_1099_report_with_members(ledger, year, None, threshold, sources, &[])
}

/// What an entry paid out of `sources`: credits minus debits to them.
fn payment(ledger: &Ledger, entry: &crate::JournalEntry, sources: &PaymentAccounts) -> Decimal {
    entry
        .lines()
        .iter()
        .filter(|l| sources.includes(ledger, l.account))
        .map(|l| match l.side {
            crate::Side::Credit => l.amount.amount,
            crate::Side::Debit => -l.amount.amount,
        })
        .sum()
}

/// The vendor's part of `payment` when the entry names several parties:
/// its resolved share scaled from the share target to the payment (the
/// SCENARIOS of [`vendor_1099_report_with_members`], INVARIANT 3).
/// Refused, never wrapped or rounded early, when payment × share is
/// beyond exact arithmetic (both near the line limit; P0 review round 3,
/// S1).
fn vendor_part(payment: Decimal, share: Decimal, target: Decimal) -> Result<Decimal, LedgerError> {
    if payment == target {
        Ok(share)
    } else if target.is_zero() {
        Ok(Decimal::ZERO)
    } else {
        let scaled = payment
            .checked_mul(share)
            .and_then(|x| x.checked_div(target))
            .ok_or_else(|| {
                LedgerError::TooLargeToCompute(format!(
                    "a vendor's share ({share}) of a payment of {payment}"
                ))
            })?;
        Ok(scaled.round_dp_with_strategy(2, rust_decimal::RoundingStrategy::MidpointAwayFromZero))
    }
}

/// [`vendor_1099_report`], counting payments out of `sources` made in
/// `year` (through `through`, when given), each vendor's part of an entry
/// with several parties resolved with `members` (the company's members,
/// from the settings).
///
/// ```text
/// SOURCE: docs/spec/rules.md B-19 (26 U.S.C. §6041(a): payments made in
///         the year; card payments go on 1099-K), B-33 (a worksheet as of
///         a day); crate::shares (an entry with several parties, each with
///         their share; DECISION, owner, 2026-10-15).
/// DEFINITIONS:
///   payment(e): credits minus debits to `sources` on entry e.
///   money: crate::parties::money_accounts(ledger, sources.cash,
///     sources.financing), what the shares resolve against (the MCP server
///     checks them with the same).
///   target(e): crate::shares::target(e, money).amount: what the shares
///     split (rules.md B-35); a reversal's is its original's negated.
/// INVARIANTS:
///   1. Only posted entries dated in `year` (and, given `through`, on or
///      before it) and tagged with a vendor (Ledger::entry_vendor) count,
///      toward that vendor. `through` makes a worksheet as of a day: the
///      payments from January 1 through it (§6041(a) and §6041A(a)(2)
///      count what is paid "in any calendar year"; a CPA package as of
///      June 30 shows the year to date, rules.md B-33).
///   2. An entry naming at most one party (Ledger::entry_parties): the
///      vendor's amount is payment(e), exactly as before shares.
///   3. An entry naming several parties (one of them the vendor's party,
///      crate::shares INVARIANT 5): the vendor's amount is its resolved
///      share s (crate::shares::resolve with money and `members`) scaled
///      to the payment: s when payment(e) = target(e) (no rounding: an
///      expense paid from cash or by a member), else
///      payment(e) × s / target(e) rounded to the nearest cent, half away
///      from zero (0 when the payment is 0: charged to a card; 0 when the
///      target is 0). The other parties' parts are not anyone's 1099.
///   4. An entry naming several parties whose shares don't resolve (the
///      members changed in the settings, say): counts payment(e) in full
///      toward the vendor, the conservative answer, and is listed in
///      `unresolved` so the human fixes the shares.
/// SCENARIOS:
///   1. Paid from cash, or by a member through a financing liability:
///      counts. Charged to a card, or billed and unpaid: doesn't.
///   2. A prepayment (Dr prepaid, Cr cash) counts in full; a refund to
///      cash and a reversal subtract.
///   3. $1,000.00 from the bank (Dr contract labor / Cr 1000) shared 60/40
///      by amount between a contractor and a member: the contractor's
///      1099 counts 600.00.
///   4. $100.01 from the bank, 50% / 50% contractor first: the
///      contractor gets 50.01 (the leftover cent goes to the first party,
///      crate::shares INVARIANT 8); named second, 50.00.
///   5. $1,000.00 split 60/40, 600.00 of it from the bank and 400.00 on
///      the card: payment 600.00, target 1,000.00, the contractor counts
///      600.00 × 600.00 / 1,000.00 = 360.00.
///   6. Reversing scenario 3: the reversal carries the shares, resolves
///      to −600.00, and the year nets to 0.00.
/// ```
pub fn vendor_1099_report_with_members(
    ledger: &Ledger,
    year: i32,
    through: Option<chrono::NaiveDate>,
    threshold: Option<Decimal>,
    sources: &PaymentAccounts,
    members: &[crate::PartyMember],
) -> Result<Vendor1099Report, LedgerError> {
    let threshold = match threshold {
        Some(t) => t,
        None => default_threshold(year)?,
    };
    let money = crate::parties::money_accounts(ledger, &sources.cash, &sources.financing);
    let mut paid: std::collections::HashMap<VendorId, Decimal> = Default::default();
    let mut unresolved = Vec::new();
    for entry in ledger.posted_entries() {
        if entry.date().year() != year || through.is_some_and(|t| entry.date() > t) {
            continue;
        }
        let Some(vendor) = ledger.entry_vendor(entry.id()) else {
            continue;
        };
        let cash = payment(ledger, entry, sources);
        let shares = ledger.entry_parties(entry.id());
        let amount = if shares.len() < 2 {
            cash
        } else {
            let party = ledger.party_of_vendor(vendor);
            match crate::shares::resolve(ledger, entry.id(), shares, &money, members) {
                Ok(resolved) => {
                    match resolved.parts.iter().find(|p| Some(p.party) == party) {
                        Some(share) => vendor_part(cash, share.amount, resolved.target.amount)?,
                        // Tagged by hand with a vendor none of the parties
                        // is: nothing to scale by, so the whole payment.
                        None => {
                            unresolved.push(entry.id());
                            cash
                        }
                    }
                }
                Err(_) => {
                    unresolved.push(entry.id());
                    cash
                }
            }
        };
        let total = paid.entry(vendor).or_default();
        *total = total.checked_add(amount).ok_or_else(|| {
            LedgerError::TooLargeToCompute("a vendor's payments for the year".into())
        })?;
    }
    let mut vendors: Vec<Vendor1099Line> = ledger
        .list_vendors()
        .into_iter()
        .filter(|v| v.form_1099.is_some() || paid.get(&v.id).is_some_and(|p| !p.is_zero()))
        .map(|v| {
            let amount = paid.get(&v.id).copied().unwrap_or_default();
            Vendor1099Line {
                vendor_id: v.id,
                name: v.name.clone(),
                form: v.form_1099,
                // On file by the vendor's flag or its party's W-9 facts.
                tin_on_file: v.tin_on_file
                    || ledger
                        .party_of_vendor(v.id)
                        .and_then(|p| ledger.get_party(p))
                        .is_some_and(|p| p.w9.is_some()),
                paid: amount,
                reportable: v.form_1099.is_some() && amount >= threshold,
            }
        })
        .collect();
    vendors.sort_by(|a, b| a.name.cmp(&b.name).then(a.vendor_id.cmp(&b.vendor_id)));
    Ok(Vendor1099Report {
        year,
        threshold,
        vendors,
        unresolved,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// vendor_part: the vendor's share scaled from the share target to the
    /// payment, exactly, rounded to the cent; refused, never a panic, when
    /// payment x share is past exact arithmetic (P0 review round 3, S1).
    #[test]
    fn a_vendor_part_is_scaled_exactly_or_refused() {
        let d = |s: &str| s.parse::<Decimal>().unwrap();
        assert_eq!(vendor_part(d("100"), d("40"), d("100")), Ok(d("40")));
        assert_eq!(vendor_part(d("50"), d("40"), d("100")), Ok(d("20")));
        assert_eq!(vendor_part(d("1"), d("1"), d("3")), Ok(d("0.33")));
        assert_eq!(vendor_part(d("2"), d("1"), d("3")), Ok(d("0.67")));
        assert_eq!(vendor_part(d("5"), d("1"), d("0")), Ok(Decimal::ZERO));
        let near = d("450000000000000");
        assert!(matches!(
            vendor_part(near * d("2"), near, near - d("1")),
            Err(LedgerError::TooLargeToCompute(_))
        ));
        assert!(matches!(
            vendor_part(Decimal::MAX, Decimal::MAX, d("1")),
            Err(LedgerError::TooLargeToCompute(_))
        ));
    }
}
