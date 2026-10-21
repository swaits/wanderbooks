//! Payouts: a platform deposit grossed up into what really happened.
//!
//! App stores, payment processors, and marketplaces collect revenue, keep
//! their fees, and deposit the difference. Booking only the deposit as
//! revenue understates income and hides the fees; the correct entry grosses
//! it up:
//!
//! ```text
//! Dr  bank            net deposit
//! Dr  platform fees   fees withheld
//! Dr/Cr adjustments   refunds, chargebacks, reserves, tax withheld, ...
//!     Cr  revenue     gross sales (one line per product or class)
//! ```
//!
//! [`payout_plan`] builds those lines and refuses unless gross − fees ±
//! adjustments equals the deposit exactly, naming the difference. It knows
//! no platform: callers (or a format-specific parser) supply the numbers
//! from the payout statement, which is the entry's receipt.

use crate::{AccountId, AccountType, ChartOfAccounts, ClassId, LedgerError, Side};
use rust_decimal::Decimal;

/// A revenue or fee line of a payout.
#[derive(Clone, Debug, PartialEq)]
pub struct PayoutLine {
    /// The account.
    pub account: AccountId,
    /// Positive amount.
    pub amount: Decimal,
    /// Optional class (product line, app, store).
    pub class: Option<ClassId>,
}

/// Anything else on the statement: refunds, chargebacks, reserves held or
/// released, taxes withheld, currency conversion. A debit reduces the
/// deposit; a credit increases it.
#[derive(Clone, Debug, PartialEq)]
pub struct PayoutAdjustment {
    /// The account.
    pub account: AccountId,
    /// Debit (reduces the deposit) or credit (increases it).
    pub side: Side,
    /// Positive amount.
    pub amount: Decimal,
    /// Optional class.
    pub class: Option<ClassId>,
}

/// A payout to gross up.
#[derive(Clone, Debug, PartialEq)]
pub struct Payout {
    /// Where the money landed.
    pub deposit_account: AccountId,
    /// The net deposit. Negative when the platform debited the account
    /// (fees and refunds exceeded sales).
    pub deposit: Decimal,
    /// Gross revenue, credited.
    pub revenue: Vec<PayoutLine>,
    /// Fees withheld, debited.
    pub fees: Vec<PayoutLine>,
    /// Everything else.
    pub adjustments: Vec<PayoutAdjustment>,
}

/// The payout's totals.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PayoutTotals {
    /// Sum of revenue lines.
    pub gross: Decimal,
    /// Sum of fee lines.
    pub fees: Decimal,
    /// Net effect of adjustments on the deposit (credits − debits).
    pub adjustments: Decimal,
    /// The deposit.
    pub deposit: Decimal,
}

/// One planned line: account, side, amount, class.
pub type PayoutPlanLine = (AccountId, Side, Decimal, Option<ClassId>);

/// Validate a payout and build its entry's lines: the deposit first, then
/// fees, adjustments, and revenue. Errors unless every amount is positive,
/// every account is postable and fits its role (revenue lines credit
/// revenue or liability accounts, e.g. deferred revenue or sales tax
/// collected; fees debit expense accounts), and the payout reconciles to
/// the cent.
/// A zero deposit (fees and adjustments ate the sales exactly) has no
/// deposit line: every planned line is positive (B-2, docs/spec/rules.md).
pub fn payout_plan(
    payout: &Payout,
    chart: &ChartOfAccounts,
) -> Result<(Vec<PayoutPlanLine>, PayoutTotals), LedgerError> {
    let bad = |m: String| Err(LedgerError::InvalidPayout(m));
    if payout.revenue.is_empty() {
        return bad("a payout needs at least one revenue line".into());
    }
    let check = |account: AccountId,
                 amount: Decimal,
                 role: &str,
                 allowed: &[AccountType]|
     -> Result<(), LedgerError> {
        let a = chart
            .get(account)
            .ok_or(LedgerError::UnknownAccount(account))?;
        if a.placeholder {
            return Err(LedgerError::InvalidPayout(format!(
                "{role} account {} is a placeholder",
                a.code
            )));
        }
        if !allowed.is_empty() && !allowed.contains(&a.account_type) {
            return Err(LedgerError::InvalidPayout(format!(
                "{role} account {} is {:?}; expected {}",
                a.code,
                a.account_type,
                allowed
                    .iter()
                    .map(|t| format!("{t:?}"))
                    .collect::<Vec<_>>()
                    .join(" or ")
            )));
        }
        if amount <= Decimal::ZERO {
            return Err(LedgerError::InvalidPayout(format!(
                "{role} amounts must be positive, got {amount} on {}",
                a.code
            )));
        }
        // No larger than a journal line may be: the payout's totals are
        // sums of them (P0 review round 3, S1).
        if !crate::kernel::Amount::in_line_range(amount) {
            return Err(LedgerError::ValueOutOfRange(format!(
                "{role} amount {amount} on {}",
                a.code
            )));
        }
        Ok(())
    };
    if !crate::kernel::Amount::in_line_range(payout.deposit) {
        return Err(LedgerError::ValueOutOfRange(format!(
            "deposit {}",
            payout.deposit
        )));
    }
    let deposit_account = chart
        .get(payout.deposit_account)
        .ok_or(LedgerError::UnknownAccount(payout.deposit_account))?;
    if deposit_account.placeholder || deposit_account.account_type != AccountType::Asset {
        return bad(format!(
            "deposit account {} must be a postable asset account",
            deposit_account.code
        ));
    }
    for l in &payout.revenue {
        check(
            l.account,
            l.amount,
            "revenue",
            &[AccountType::Revenue, AccountType::Liability],
        )?;
    }
    for l in &payout.fees {
        check(l.account, l.amount, "fee", &[AccountType::Expense])?;
    }
    for a in &payout.adjustments {
        check(a.account, a.amount, "adjustment", &[])?;
    }
    let gross: Decimal = payout.revenue.iter().map(|l| l.amount).sum();
    let fees: Decimal = payout.fees.iter().map(|l| l.amount).sum();
    let adjustments: Decimal = payout
        .adjustments
        .iter()
        .map(|a| match a.side {
            Side::Credit => a.amount,
            Side::Debit => -a.amount,
        })
        .sum();
    let expected = gross - fees + adjustments;
    if expected != payout.deposit {
        return bad(format!(
            "payout doesn't reconcile: gross {gross} − fees {fees} + adjustments \
             {adjustments} = {expected}, but the deposit is {} (off by {})",
            payout.deposit,
            payout.deposit - expected
        ));
    }
    let mut plan = Vec::new();
    if payout.deposit > Decimal::ZERO {
        plan.push((payout.deposit_account, Side::Debit, payout.deposit, None));
    } else if payout.deposit < Decimal::ZERO {
        plan.push((payout.deposit_account, Side::Credit, -payout.deposit, None));
    }
    plan.extend(
        payout
            .fees
            .iter()
            .map(|l| (l.account, Side::Debit, l.amount, l.class)),
    );
    plan.extend(
        payout
            .adjustments
            .iter()
            .map(|a| (a.account, a.side, a.amount, a.class)),
    );
    plan.extend(
        payout
            .revenue
            .iter()
            .map(|l| (l.account, Side::Credit, l.amount, l.class)),
    );
    Ok((
        plan,
        PayoutTotals {
            gross,
            fees,
            adjustments,
            deposit: payout.deposit,
        },
    ))
}
