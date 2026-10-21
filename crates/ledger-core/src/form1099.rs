//! 1099 tracking: what each vendor was paid in a calendar year, and what
//! of it counts toward the information return the company files for it.
//!
//! **The decision.** A vendor's 1099 decision ([`Decision1099`]) says
//! whether it gets one: a 1099-NEC (contractors, attorneys), a 1099-MISC
//! (rent and the like), *not reportable* with its reason (a corporation,
//! the United States or a state, a payee that sells only merchandise:
//! Treas. Reg. §1.6041-3), or *undetermined*, which is what a vendor
//! nobody has decided on reads as. "Not reportable" and "undetermined" are
//! never the same thing (plan defects 18 and 23). Each decision carries
//! who made it and when, and the date it takes effect ([`Dated1099`],
//! D-40): a payee's status can change (an LLC electing to be taxed as a
//! corporation keeps its EIN and changes status on the election's date),
//! so the worksheet applies the decision in force on each payment's date.
//! A payee that incorporates is a new legal person: a new vendor record
//! with its own W-9. A decision recorded without a date (every one written
//! before decisions were dated) is in force from the beginning.
//!
//! **The payments.** Entries are tagged with the vendor they paid
//! (`pay_bill` tags automatically) and, where it matters, with the
//! payment method ([`crate::instruments`]). Each line of a tagged entry
//! goes in a bucket ([`crate::kernel::payment_bucket`]): what *counts*
//! toward the vendor's 1099, what a *card* or payment network paid (the
//! processor reports it on Form 1099-K, never the company:
//! §1.6041-1(a)(1)(iv), whoever's card it was), and what is
//! *undetermined* until the payment method is recorded (a member paying
//! through the financing liability: their card is excluded, their check
//! counts, since the company is still the payor, §1.6041-1(e)(1)). A
//! payment out of the bank with no method recorded counts (presumed check,
//! ACH or transfer, D-40); tag a debit-card payment with the card. Money
//! moved between the company's own sources (the card bill, a member
//! reimbursed) pays no one ([`crate::kernel::net_buckets`]). A bill is not
//! a payment until it is paid, and a prepayment counts when made: §6041
//! reports payments made in the year.
//!
//! Thresholds are set by law and change; [`default_threshold`] encodes
//! them and the report takes an override. Confirm with the CPA.

use crate::kernel::{self, Bucket, PayRole, Tag};
use crate::settings::Source;
use crate::{AccountId, AccountType, EntryId, Ledger, LedgerError, VendorId};
use chrono::{Datelike, NaiveDate};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

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

    /// "nec" or "misc": the journal's and the tools' key.
    pub fn key(self) -> &'static str {
        match self {
            Form1099::Nec => "nec",
            Form1099::Misc => "misc",
        }
    }
}

/// Why a vendor gets no 1099: the kind of payee no return is required
/// for. Each has its authority ([`ExemptReason::citation`],
/// sources/irs-1099-payment-method.md); a decision adds the facts in
/// words.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExemptReason {
    /// A corporation (Treas. Reg. §1.6041-3(p)(1)), an LLC taxed as one
    /// included, shown by its name ("Inc.", "Corp.", never "Co.":
    /// §1.6049-4(c)(1)(ii)(A)(1)), a W-9, a Form 8832, or the public
    /// record. Payments to a corporation for legal services or for medical
    /// and health care stay reportable (§1.6041-3(p)(1); Instructions for
    /// Forms 1099-MISC and 1099-NEC): never this reason for those.
    Corporation,
    /// An organization exempt from tax under §501(a), or an individual
    /// retirement plan (§1.6041-3(p)(2)).
    TaxExempt,
    /// The United States (§1.6041-3(p)(3)).
    UnitedStates,
    /// A State, the District of Columbia, a U.S. possession, or a
    /// political subdivision of any of them (§1.6041-3(p)(4)).
    State,
    /// A foreign government or a political subdivision of one
    /// (§1.6041-3(p)(5)).
    ForeignGovernment,
    /// A foreign payee documented before payment (§1.6041-4(a)(1)): its
    /// payments go on Form 1042-S, if anything.
    ForeignPerson,
    /// A payee that sells only merchandise, telephone, freight, storage
    /// and the like (§1.6041-3(c)). A vendor selling services too stays a
    /// 1099 vendor.
    Merchandise,
    /// An employee: wages go on Form W-2 (§1.6041-2(a)(1)).
    Employee,
    /// Another reason, which the decision's words and citation give.
    Other,
}

impl ExemptReason {
    /// Every reason, in order.
    pub const ALL: [ExemptReason; 9] = [
        ExemptReason::Corporation,
        ExemptReason::TaxExempt,
        ExemptReason::UnitedStates,
        ExemptReason::State,
        ExemptReason::ForeignGovernment,
        ExemptReason::ForeignPerson,
        ExemptReason::Merchandise,
        ExemptReason::Employee,
        ExemptReason::Other,
    ];

    /// Parse a key ("corporation", "united_states", ...), ignoring case and
    /// surrounding space.
    pub fn parse(s: &str) -> Option<ExemptReason> {
        let s = s.trim().to_ascii_lowercase();
        ExemptReason::ALL.into_iter().find(|r| r.key() == s)
    }

    /// Its key, as journaled.
    pub fn key(self) -> &'static str {
        match self {
            ExemptReason::Corporation => "corporation",
            ExemptReason::TaxExempt => "tax_exempt",
            ExemptReason::UnitedStates => "united_states",
            ExemptReason::State => "state",
            ExemptReason::ForeignGovernment => "foreign_government",
            ExemptReason::ForeignPerson => "foreign_person",
            ExemptReason::Merchandise => "merchandise",
            ExemptReason::Employee => "employee",
            ExemptReason::Other => "other",
        }
    }

    /// In words: "a corporation", "the United States", ...
    pub fn label(self) -> &'static str {
        match self {
            ExemptReason::Corporation => "a corporation",
            ExemptReason::TaxExempt => "a tax-exempt organization",
            ExemptReason::UnitedStates => "the United States",
            ExemptReason::State => "a state or local government",
            ExemptReason::ForeignGovernment => "a foreign government",
            ExemptReason::ForeignPerson => "a foreign person",
            ExemptReason::Merchandise => "sells only merchandise",
            ExemptReason::Employee => "an employee",
            ExemptReason::Other => "another reason",
        }
    }

    /// The authority that exempts it; none for [`ExemptReason::Other`],
    /// whose decision cites its own.
    pub fn citation(self) -> Option<&'static str> {
        Some(match self {
            ExemptReason::Corporation => "Treas. Reg. §1.6041-3(p)(1)",
            ExemptReason::TaxExempt => "Treas. Reg. §1.6041-3(p)(2)",
            ExemptReason::UnitedStates => "Treas. Reg. §1.6041-3(p)(3)",
            ExemptReason::State => "Treas. Reg. §1.6041-3(p)(4)",
            ExemptReason::ForeignGovernment => "Treas. Reg. §1.6041-3(p)(5)",
            ExemptReason::ForeignPerson => "Treas. Reg. §1.6041-4(a)(1)",
            ExemptReason::Merchandise => "Treas. Reg. §1.6041-3(c)",
            ExemptReason::Employee => "Treas. Reg. §1.6041-2(a)(1)",
            ExemptReason::Other => return None,
        })
    }
}

/// A vendor's 1099 decision.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision1099 {
    /// Nobody has decided (or a line from before decisions were kept,
    /// which can't say: plan 5.5).
    #[default]
    Undetermined,
    /// It gets this return when paid at least the threshold.
    Files(Form1099),
    /// No return is required for it, and why.
    NotReportable {
        /// The kind of payee.
        kind: ExemptReason,
        /// The facts, in words ("a corporation by name", "Delaware public
        /// benefit corporation, from the public record").
        reason: String,
    },
}

impl Decision1099 {
    /// A not-reportable decision, its reason trimmed and checked
    /// ([`Decision1099::check`]).
    pub fn not_reportable(kind: ExemptReason, reason: &str) -> Result<Decision1099, String> {
        let d = Decision1099::NotReportable {
            kind,
            reason: reason.trim().to_string(),
        };
        d.check()?;
        Ok(d)
    }

    /// The return it files, if it files one.
    pub fn form(&self) -> Option<Form1099> {
        match self {
            Decision1099::Files(f) => Some(*f),
            _ => None,
        }
    }

    /// "nec", "misc", "not_reportable" or "undetermined".
    pub fn key(&self) -> &'static str {
        match self {
            Decision1099::Undetermined => "undetermined",
            Decision1099::Files(f) => f.key(),
            Decision1099::NotReportable { .. } => "not_reportable",
        }
    }

    /// The decision in words: "1099-NEC", "not reportable: a corporation
    /// (by name)", "undetermined".
    pub fn describe(&self) -> String {
        match self {
            Decision1099::Undetermined => "undetermined".into(),
            Decision1099::Files(f) => f.label().into(),
            Decision1099::NotReportable { kind, reason } => {
                format!("not reportable: {} ({reason})", kind.label())
            }
        }
    }

    /// Check a decision before it is journaled: a not-reportable reason is
    /// one line of at most `settings::MAX_TEXT` characters, trimmed, with
    /// no full TIN (P3 review R16).
    pub fn check(&self) -> Result<(), String> {
        if let Decision1099::NotReportable { reason, .. } = self {
            let t = crate::settings::line_text("reason", reason, crate::settings::MAX_TEXT)?;
            if t != *reason {
                return Err("reason has space around it".into());
            }
        }
        Ok(())
    }
}

/// A 1099 decision with the date it takes effect and who made it (D-40).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dated1099 {
    /// The day it takes effect: a W-9's date, or a tax election's
    /// effective date. `None`: from the beginning (the vendor's status all
    /// along, and every decision written before decisions were dated).
    pub effective: Option<NaiveDate>,
    /// The decision.
    pub decision: Decision1099,
    /// Who decided it and when; `None` only on lines written before
    /// decisions were kept (the store requires one on every new line).
    pub source: Option<Source>,
}

impl Dated1099 {
    /// A decision in force from the beginning.
    pub fn from_beginning(decision: Decision1099, source: Option<Source>) -> Dated1099 {
        Dated1099 {
            effective: None,
            decision,
            source,
        }
    }
}

/// What recording a dated decision does to a vendor's decisions
/// ([`plan_decision`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Applied {
    /// Nothing: it is already recorded exactly so, or a dated decision the
    /// same as the one in force before its date (no change on that date).
    Unchanged,
    /// Recorded: added, or replacing the one recorded for that date.
    Set,
    /// The decision recorded for that date is taken out: the new one is
    /// the same as the one in force before it, so nothing changes there.
    Removed,
}

/// The decision in force on `date`: the last one taking effect on or
/// before it (one from the beginning counts). `decisions` are in date
/// order, the undated first ([`apply_decision`] keeps them so).
pub fn in_force(decisions: &[Dated1099], date: NaiveDate) -> Option<&Dated1099> {
    decisions
        .iter()
        .rev()
        .find(|d| d.effective.is_none_or(|e| e <= date))
}

/// The decision in force the day before `date`, whatever is recorded for
/// `date` itself.
fn in_force_before(decisions: &[Dated1099], date: NaiveDate) -> Option<&Dated1099> {
    decisions
        .iter()
        .rev()
        .find(|d| d.effective.is_none_or(|e| e < date))
}

/// What recording `new` would do to `decisions` (one per effective date,
/// in date order).
///
/// ```text
/// GOAL: each payment takes the vendor's 1099 decision in force on its
///       date (D-40), and recording one never changes a figure it
///       doesn't mean to.
/// SOURCE: DECISION D-40 (owner, 2026-10-20: dated statuses); P3 review
///         R8 (an identical re-record journals nothing).
/// SCENARIOS:
///   1. IF `new` is recorded exactly so already (decision, date, source):
///      SHOULD be Unchanged.
///   2. IF `new` is undated: SHOULD be Set (the decision from the
///      beginning, replacing any).
///   3. IF `new` is dated and its decision is the one in force the day
///      before: SHOULD be Removed when a decision is recorded for that
///      date (taking it out changes nothing before or after), else
///      Unchanged.
///   4. Otherwise: SHOULD be Set (added for that date, or replacing the
///      one recorded for it).
/// ```
pub fn plan_decision(decisions: &[Dated1099], new: &Dated1099) -> Applied {
    let at = decisions.iter().find(|d| d.effective == new.effective);
    if at == Some(new) {
        return Applied::Unchanged;
    }
    let Some(date) = new.effective else {
        return Applied::Set;
    };
    let before = in_force_before(decisions, date)
        .map(|d| &d.decision)
        .unwrap_or(&Decision1099::Undetermined);
    if *before == new.decision {
        if at.is_some() {
            Applied::Removed
        } else {
            Applied::Unchanged
        }
    } else {
        Applied::Set
    }
}

/// Record `new` in `decisions` ([`plan_decision`]), keeping them one per
/// effective date, in date order. Returns what it did.
pub fn apply_decision(decisions: &mut Vec<Dated1099>, new: Dated1099) -> Applied {
    let applied = plan_decision(decisions, &new);
    match applied {
        Applied::Unchanged => {}
        Applied::Removed => decisions.retain(|d| d.effective != new.effective),
        Applied::Set => {
            match decisions.iter().position(|d| d.effective >= new.effective) {
                Some(i) if decisions[i].effective == new.effective => decisions[i] = new,
                Some(i) => decisions.insert(i, new),
                None => decisions.push(new),
            };
        }
    }
    applied
}

/// The checks a new 1099 decision passes before it is recorded (replay
/// skips them: rules.md B-28): the decision ([`Decision1099::check`]),
/// its source, its date in the books' calendar, and that a dated decision
/// records a change. Returns what recording it would do.
///
/// ```text
/// GOAL: no dated decision silently leaves the vendor's earlier payments
///       undecided.
/// SOURCE: DECISION D-40 (owner, 2026-10-20); docs/spec/rules.md B-19.
/// SCENARIOS:
///   1. IF `new` is dated, would be recorded, and no decision other than
///      undetermined is in force at any time before its date: SHOULD
///      refuse
///      BECAUSE a dated decision records a change, and recording only
///      the change would leave every earlier payment undetermined:
///      record the vendor's status from the beginning (no date) first.
///   2. IF it would only take out the decision recorded for that date
///      (plan_decision SCENARIO 3): SHOULD be allowed.
/// ```
pub fn check_new_decision(decisions: &[Dated1099], new: &Dated1099) -> Result<Applied, String> {
    new.decision.check()?;
    if let Some(s) = &new.source {
        s.check()?;
    }
    if let Some(d) = new.effective {
        crate::calendar::check(d).map_err(|e| e.to_string())?;
    }
    let applied = plan_decision(decisions, new);
    if let (Some(date), Applied::Set) = (new.effective, applied) {
        let decided_before = decisions
            .iter()
            .filter(|d| d.effective.is_none_or(|e| e < date))
            .any(|d| d.decision != Decision1099::Undetermined);
        if !decided_before {
            return Err(format!(
                "a dated 1099 decision records a change, and nothing is decided before {date}: record the vendor's status from the beginning first (no effective date), then the change"
            ));
        }
    }
    Ok(applied)
}

/// The reporting threshold for payments made in `year`.
///
/// ```text
/// GOAL: the dollar amount at which a vendor's payments become reportable.
/// SOURCE: docs/spec/rules.md B-19 (26 U.S.C. §6041(a), (h), §6041A(a)(2),
///         Pub. L. 119-21 §70433(f): payments made after December 31,
///         2025; sources/irs-1099-thresholds.md,
///         sources/irs-1099-payment-method.md).
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

/// Where a vendor stands for the year.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status1099 {
    /// Decided for a 1099, and what counts reaches the threshold however
    /// the unknown payments turn out: file one.
    Reportable,
    /// What could count stays under the threshold, however the unknowns
    /// are settled. (Royalties at $10 and attorneys' gross proceeds at $600
    /// are not modeled: confirm those with the CPA.)
    UnderThreshold,
    /// Decided not reportable for every payment that could count: no
    /// 1099, and the decision says why.
    NotReportable,
    /// A 1099 depends on something not yet known: the vendor's decision
    /// for some of the year's payments, or how some were paid.
    Undetermined,
}

impl Status1099 {
    /// "reportable", "under_threshold", "not_reportable", "undetermined".
    pub fn key(self) -> &'static str {
        match self {
            Status1099::Reportable => "reportable",
            Status1099::UnderThreshold => "under_threshold",
            Status1099::NotReportable => "not_reportable",
            Status1099::Undetermined => "undetermined",
        }
    }
}

/// One vendor's year.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Vendor1099Line {
    /// The vendor.
    pub vendor_id: VendorId,
    /// Name.
    pub name: String,
    /// The decision in force at the end of the year (or of `through`).
    pub decision: Decision1099,
    /// Who made it and when (none for a line from before decisions were
    /// kept, or a vendor never decided).
    pub decided: Option<Source>,
    /// The date it took effect (none: from the beginning).
    pub effective: Option<NaiveDate>,
    /// Every decision in force during the year, in date order (more than
    /// one when the vendor's status changed in the year; none when the
    /// vendor was never decided).
    pub decisions: Vec<Dated1099>,
    /// W-9 on file.
    pub tin_on_file: bool,
    /// What counts toward its 1099: paid by check, ACH, transfer or cash
    /// (net of refunds and reversals) while it wasn't decided not
    /// reportable.
    pub paid: Decimal,
    /// Paid by card or payment network: the processor's Form 1099-K, never
    /// the company's 1099.
    pub excluded_card: Decimal,
    /// Paid through a financing liability with no payment method
    /// recorded: counts or not once its method is known (net).
    pub undetermined: Decimal,
    /// Paid while the vendor was decided not reportable (not by card).
    pub not_reportable: Decimal,
    /// Some of the year's payments that could count were made while the
    /// vendor's decision was undetermined: a decision is needed.
    pub decision_needed: bool,
    /// Where it stands ([`Status1099`]).
    pub status: Status1099,
    /// `status == Reportable`: file a 1099.
    pub reportable: bool,
}

impl Vendor1099Line {
    /// The return it gets, as decided at the end of the year.
    pub fn form(&self) -> Option<Form1099> {
        self.decision.form()
    }
}

/// One payment on a vendor's year, for the worksheet's lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Payment1099 {
    /// The entry.
    pub entry_id: EntryId,
    /// Its date.
    pub date: NaiveDate,
    /// The vendor it paid.
    pub vendor_id: VendorId,
    /// The vendor's part of it the list is about.
    pub amount: Decimal,
    /// The payment method it is tagged with, if any.
    pub instrument: Option<crate::InstrumentId>,
}

/// The year's 1099 report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Vendor1099Report {
    /// Calendar year.
    pub year: i32,
    /// The threshold applied.
    pub threshold: Decimal,
    /// The last day counted, when the report stops before December 31.
    pub through: Option<NaiveDate>,
    /// Every vendor decided for a 1099 during the year, or with any figure
    /// in it, by name.
    pub vendors: Vec<Vendor1099Line>,
    /// Entries naming several parties whose shares didn't resolve, in
    /// posting order: each counted in full toward its vendor until its
    /// shares are fixed ([`vendor_1099_report`] INVARIANT 5).
    pub unresolved: Vec<EntryId>,
    /// Payments by card or payment network, excluded (posting order).
    pub excluded_card_payments: Vec<Payment1099>,
    /// Payments whose method must be recorded before they can be
    /// classified (posting order); none for a vendor decided not
    /// reportable then.
    pub undetermined_payments: Vec<Payment1099>,
    /// Money received from a vendor for something else (a sale to it, its
    /// payment of an invoice, an asset disposed of to it, a loan from it),
    /// not netted against its payments, with the reversals of each.
    pub other_receipts: Vec<Payment1099>,
    /// Posted entries in the year that paid money out for something, with
    /// no vendor tag and no payee that rules out a 1099 (the quieter note).
    pub untagged_payments: Vec<EntryId>,
}

/// The accounts payments to vendors come out of: the books'
/// `books.cash_accounts` and `books.financing_accounts`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PaymentAccounts {
    /// Cash accounts: bank, savings, cash on hand. Required.
    pub cash: Vec<AccountId>,
    /// Financing liabilities: what members or lenders paid on the
    /// company's behalf (Due to Members, loans).
    pub financing: Vec<AccountId>,
}

/// Each account's payment role ([`PayRole`]) for one report.
///
/// ```text
/// GOAL: a payment line's role comes from what the books record, and a
///       line whose method isn't recorded is never presumed a card's
///       unless only a card pays through its account.
/// SOURCE: docs/spec/rules.md B-19 (P3 review R2); DECISION D-40 (a bank
///         payment with no method recorded counts).
/// INVARIANTS (first match wins):
///   1. Cash: one of the books' cash accounts.
///   2. Financing: one of the books' financing accounts, or the account of
///      a member's card, a member's cash or check, or a payment platform
///      (a members' account pays by card or by check, a platform's balance
///      by the network or friends-and-family: which, only the entry's
///      method says).
///   3. CardAccount: the account of a company card (business_card), the
///      card's liability or a card's own balance.
///   4. Other: anything else (an expense, payables, prepaid).
/// ```
struct Roles {
    cash: HashSet<AccountId>,
    financing: HashSet<AccountId>,
    card: HashSet<AccountId>,
    /// Every instrument's account.
    instrument: HashSet<AccountId>,
}

impl Roles {
    fn new(ledger: &Ledger, sources: &PaymentAccounts) -> Roles {
        use crate::InstrumentKind as K;
        let named = |kinds: &[K]| -> HashSet<AccountId> {
            ledger
                .instruments()
                .filter(|i| kinds.contains(&i.kind))
                .map(|i| i.account)
                .collect()
        };
        Roles {
            cash: sources.cash.iter().copied().collect(),
            financing: sources
                .financing
                .iter()
                .copied()
                .chain(named(&[K::MemberCard, K::MemberCashOrCheck, K::Platform]))
                .collect(),
            card: named(&[K::BusinessCard]),
            instrument: ledger.instruments().map(|i| i.account).collect(),
        }
    }

    fn role(&self, account: AccountId) -> PayRole {
        if self.cash.contains(&account) {
            PayRole::Cash
        } else if self.financing.contains(&account) {
            PayRole::Financing
        } else if self.card.contains(&account) {
            PayRole::CardAccount
        } else {
            PayRole::Other
        }
    }

    /// Whether money is paid out of `account`.
    fn pays(&self, account: AccountId) -> bool {
        self.role(account) != PayRole::Other || self.instrument.contains(&account)
    }
}

/// An entry's payment, per bucket ([`kernel::split_payment`]), netted
/// ([`kernel::net_buckets`]): `[counts, card, undetermined]`.
fn entry_buckets(
    ledger: &Ledger,
    entry: &crate::JournalEntry,
    roles: &Roles,
) -> Result<[Decimal; 3], LedgerError> {
    let tag = ledger
        .entry_instrument(entry.id())
        .and_then(|i| ledger.get_instrument(i));
    let lines: Vec<(Option<Bucket>, crate::Side, Decimal)> = entry
        .lines()
        .iter()
        .map(|l| {
            let t = match tag {
                None => Tag::None,
                Some(i) if i.account == l.account => {
                    if i.kind.is_card_or_network() {
                        Tag::ThisCard
                    } else {
                        Tag::ThisOther
                    }
                }
                Some(_) => Tag::Elsewhere,
            };
            (
                kernel::payment_bucket(roles.role(l.account), t),
                l.side,
                l.amount.amount,
            )
        })
        .collect();
    kernel::split_payment(&lines)
        .and_then(kernel::net_buckets)
        .ok_or_else(|| LedgerError::TooLargeToCompute(format!("entry {}'s payment", entry.id())))
}

fn sum3(b: [Decimal; 3]) -> Result<Decimal, LedgerError> {
    b[0].checked_add(b[1])
        .and_then(|s| s.checked_add(b[2]))
        .ok_or_else(|| LedgerError::TooLargeToCompute("a payment's buckets".into()))
}

/// The vendor's part of `payment` when the entry names several parties:
/// its resolved share scaled from the share target to the payment (the
/// SCENARIOS of [`vendor_1099_report`], INVARIANT 4). Refused, never
/// wrapped or rounded early, when payment × share is beyond exact
/// arithmetic (both near the line limit; P0 review round 3, S1).
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

/// Split the vendor's part `v` of a netted payment `b` across its buckets
/// in proportion, exactly: each bucket's exact share rounded down to the
/// cent, the cents left over handed out by largest remainder
/// ([`kernel::apportion`], P3 review R12), so the buckets sum to `v` to
/// the last cent. The netted buckets share one sign, so every part takes
/// `v`'s.
fn split_share(b: [Decimal; 3], v: Decimal) -> Result<[Decimal; 3], LedgerError> {
    let too_large =
        || LedgerError::TooLargeToCompute(format!("a vendor's share ({v}) of a payment"));
    let p = sum3(b)?;
    if v == p {
        return Ok(b);
    }
    if p.is_zero() || v.is_zero() {
        return Ok([Decimal::ZERO; 3]);
    }
    let (av, ap) = (v.abs(), p.abs());
    let cent = Decimal::new(1, 2);
    let times = |x: Decimal| x.checked_mul(ap).ok_or_else(too_large);
    let mut parts = [Decimal::ZERO; 3];
    let mut rems = [Decimal::ZERO; 3];
    for i in 0..3 {
        if b[i].is_zero() {
            continue;
        }
        // The exact share is num / ap: rounded down to the cent, checked
        // exactly (the division alone rounds past 28 digits).
        let num = b[i].abs().checked_mul(av).ok_or_else(too_large)?;
        let q = num.checked_div(ap).ok_or_else(too_large)?;
        let mut part = q.round_dp_with_strategy(2, rust_decimal::RoundingStrategy::ToZero);
        while times(part)? > num {
            part -= cent;
        }
        while times(part + cent)? <= num {
            part += cent;
        }
        // The remainder, over the common denominator ap.
        rems[i] = num - times(part)?;
        parts[i] = part;
    }
    kernel::apportion(av, &mut parts, &rems, cent).ok_or_else(too_large)?;
    if v.is_sign_negative() {
        for x in parts.iter_mut().filter(|x| !x.is_zero()) {
            *x = -*x;
        }
    }
    Ok(parts)
}

/// Each posted entry's reversal chain's first entry (the payment itself),
/// from one pass in posting order: a reversal is posted after what it
/// reverses.
fn chain_roots(ledger: &Ledger) -> HashMap<EntryId, EntryId> {
    let mut roots: HashMap<EntryId, EntryId> = HashMap::new();
    for e in ledger.posted_entries() {
        let root = e
            .reversal_of()
            .and_then(|o| roots.get(&o).copied())
            .unwrap_or(e.id());
        roots.insert(e.id(), root);
    }
    roots
}

/// The accounts a refund of a payment credits, giving back what the
/// payment paid for: an expense, payables (a bill's payment refunded), or
/// an asset no money is paid out of that no one owes the company and that
/// is no registered asset's cost (prepaid, a deposit). Revenue,
/// receivables (a customer paying its invoice), a registered asset's cost
/// (a disposal), equity and other liabilities (a loan) are money received
/// for something else (P3 review R6).
fn refund_accounts(ledger: &Ledger, roles: &Roles) -> HashSet<AccountId> {
    let payables = ledger.role_account(&crate::PAYABLES);
    let receivables = ledger.role_account(&crate::RECEIVABLES);
    let costs: HashSet<AccountId> = ledger.registered_assets().map(|a| a.cost_account).collect();
    ledger
        .chart()
        .all()
        .filter(|a| match a.account_type {
            AccountType::Expense => true,
            AccountType::Liability => Some(a.id) == payables,
            AccountType::Asset => {
                !roles.pays(a.id) && Some(a.id) != receivables && !costs.contains(&a.id)
            }
            AccountType::Revenue | AccountType::Equity => false,
        })
        .map(|a| a.id)
        .collect()
}

/// What each vendor's year adds up to, before its status is worked out.
#[derive(Default)]
struct Tally {
    /// Counts while decided for a 1099, and while undetermined.
    paid_files: Decimal,
    paid_undecided: Decimal,
    card: Decimal,
    not_reportable: Decimal,
    /// Undetermined amounts per reversal chain (by its first entry), while
    /// decided for a 1099, and while undetermined.
    unknown_files: BTreeMap<EntryId, Decimal>,
    unknown_undecided: BTreeMap<EntryId, Decimal>,
}

fn add(to: &mut Decimal, x: Decimal) -> Result<(), LedgerError> {
    *to = to
        .checked_add(x)
        .ok_or_else(|| LedgerError::TooLargeToCompute("a vendor's payments for the year".into()))?;
    Ok(())
}

/// The lowest and highest totals `paid` could reach once every chain's
/// unknown amount is settled one way or the other (a chain's unknowns
/// settle together: its entries share their tag; P3 review R3).
fn bounds(
    paid: Decimal,
    unknown: &BTreeMap<EntryId, Decimal>,
) -> Result<(Decimal, Decimal), LedgerError> {
    let (mut lo, mut hi) = (paid, paid);
    for &u in unknown.values() {
        add(&mut lo, u.min(Decimal::ZERO))?;
        add(&mut hi, u.max(Decimal::ZERO))?;
    }
    Ok((lo, hi))
}

/// The 1099 report for calendar `year`: per vendor, what counts toward
/// its 1099, what a card or payment network paid, what is unknown, and
/// where it stands; payments out of `sources`, each entry's parties'
/// shares resolved with `members` (the company's members, from the
/// settings), entries dated after `through` left out.
///
/// ```text
/// SOURCE: docs/spec/rules.md B-19 (26 U.S.C. §6041(a): payments made in
///         the year; Treas. Reg. §1.6041-1(a)(1)(iv): card and network
///         payments go on Form 1099-K; §1.6041-1(e)(1): the company is the
///         payor when a member pays its bill; §1.6041-3: payees no return
///         is required for; sources/irs-1099-payment-method.md);
///         crate::shares (an entry with several parties, each with their
///         share; DECISION, owner, 2026-10-15); DECISION D-40 (owner,
///         2026-10-20: dated decisions; a bank payment with no method
///         recorded counts); B-33 (a worksheet as of a day).
/// DEFINITIONS:
///   buckets(e): kernel::payment_bucket per line (role from `sources` and
///     the instruments' accounts; tag from Ledger::entry_instrument),
///     summed by kernel::split_payment and netted by kernel::net_buckets:
///     [counts, card, undetermined], each credits minus debits.
///   root(e): the first entry of e's reversal chain (the payment itself).
///   decision(e): the vendor's decision in force on root(e)'s date.
///   money: crate::parties::money_accounts(ledger, sources.cash,
///     sources.financing), what the shares resolve against (the MCP server
///     checks them with the same).
///   target(e): crate::shares::target(e, money).amount: what the shares
///     split (rules.md B-35); a reversal's is its original's negated.
/// INVARIANTS:
///   1. Refuses when `sources.cash` is empty (name the books' cash
///      accounts; never every asset: P3 review R5), and, with no
///      threshold given, when the books' currency isn't USD (the
///      thresholds are in dollars).
///   2. Only posted entries dated in `year` (and on or before `through`)
///      and tagged with a vendor count, toward that vendor. `through`
///      makes a worksheet as of a day: the payments from January 1
///      through it (§6041(a) and §6041A(a)(2) count what is paid "in any
///      calendar year"; a CPA package as of June 30 shows the year to
///      date, rules.md B-33).
///   3. An entry naming at most one party: the vendor's amounts are
///      buckets(e).
///   4. An entry naming several parties (one of them the vendor's party,
///      crate::shares INVARIANT 5): the vendor's part V of the payment P
///      (the buckets' sum) is its resolved share s scaled to it: s when
///      P = target(e), else P × s / target(e) rounded to the nearest cent,
///      half away from zero (0 when the target is 0). V is split across
///      the buckets in proportion, exactly (largest remainder, to the
///      cent), so they sum to V.
///   5. Several parties whose shares don't resolve (the members changed
///      in the settings, say): buckets(e) in full, the conservative
///      answer, and the entry is listed in `unresolved`.
///   6. A reversal chain whose payment took money in for something else
///      (root(e) nets money in and credits, beside its payment lines,
///      anything a refund doesn't: revenue, receivables, a registered
///      asset's cost account, equity, a liability other than payables; a
///      sale, a customer's payment, a disposal, a loan from the vendor) is
///      left out of the vendor's figures and listed in `other_receipts`
///      (P3 review R6). A refund (money in crediting only an expense,
///      payables, or an asset no money is paid out of and no one owes, as
///      prepaid or a deposit) and a reversal net.
///   7. Card goes to excluded_card whatever the decision. Counts and
///      undetermined go to not_reportable when decision(e) is not
///      reportable, else to paid and undetermined.
///   8. Status, with T the threshold, F the payments decided for a 1099,
///      U those made while undetermined, and lo/hi each part's total with
///      every chain's unknown settled low/high: nothing in F or U is
///      NotReportable when anything was paid while not reportable or the
///      year-end decision is, else UnderThreshold; anything in U is
///      Undetermined when hi(F) + max(hi(U), 0) >= T, else
///      UnderThreshold; otherwise Reportable when lo(F) >= T,
///      UnderThreshold when hi(F) < T, else Undetermined.
///   9. Listed: every vendor with a decision for a 1099 in force during
///      the year, and every vendor with any figure in it.
/// SCENARIOS:
///   1. Paid from the bank (no method recorded, or the bank's): counts.
///      Charged to a card, or tagged with a card or network: excluded,
///      listed. Paid by a member through the financing liability:
///      undetermined until tagged; their card excluded, their cash or
///      check counts.
///   2. A prepayment (Dr prepaid, Cr cash) counts in full; a refund to
///      cash and a reversal subtract; the card bill and a member
///      reimbursed pay no one.
///   3. $1,000.00 from the bank (Dr contract labor / Cr 1000) shared 60/40
///      by amount between a contractor and a member: the contractor's
///      1099 counts 600.00.
///   4. $100.01 from the bank, 50% / 50% contractor first: the
///      contractor gets 50.01 (the leftover cent goes to the first party,
///      crate::shares INVARIANT 8); named second, 50.00.
///   5. $1,000.00 split 60/40, 600.00 of it from the bank and 400.00 on
///      a card nobody recorded as an instrument: payment 600.00, target
///      1,000.00, the contractor counts 600.00 × 600.00 / 1,000.00 =
///      360.00.
///   6. Reversing scenario 3: the reversal carries the shares, resolves
///      to −600.00, and the year nets to 0.00.
///   7. A contractor decided 1099-NEC from the beginning and not
///      reportable from December 30 (a corporate election): payments
///      before December 30 count, payments from it on are not
///      reportable.
/// ```
pub fn vendor_1099_report(
    ledger: &Ledger,
    year: i32,
    threshold: Option<Decimal>,
    sources: &PaymentAccounts,
    members: &[crate::PartyMember],
    through: Option<NaiveDate>,
) -> Result<Vendor1099Report, LedgerError> {
    let threshold = match threshold {
        Some(t) => t,
        None => {
            let currency = ledger.base_currency();
            if currency != crate::Currency::USD {
                return Err(LedgerError::Invalid1099(format!(
                    "the 1099 thresholds are in US dollars and these books keep {}: pass the threshold to use, in {}",
                    currency.code(),
                    currency.code()
                )));
            }
            default_threshold(year)?
        }
    };
    if sources.cash.is_empty() {
        return Err(LedgerError::Invalid1099(
            "the 1099 worksheet counts payments out of the books' cash accounts, and none are named: set them (the books.cash_accounts setting; books.settings shows it)".into(),
        ));
    }
    let start = NaiveDate::from_ymd_opt(year, 1, 1)
        .ok_or_else(|| LedgerError::Invalid1099(format!("year {year} is outside the calendar")))?;
    let year_end = NaiveDate::from_ymd_opt(year, 12, 31).unwrap_or(start);
    let end = through.map_or(year_end, |t| t.min(year_end));
    let in_window = |d: NaiveDate| d.year() == year && d <= end;
    let roles = Roles::new(ledger, sources);
    let money = crate::parties::money_accounts(ledger, &sources.cash, &sources.financing);
    let roots = chain_roots(ledger);
    let refunds = refund_accounts(ledger, &roles);
    let mut tallies: HashMap<VendorId, Tally> = HashMap::new();
    let mut unresolved = Vec::new();
    let mut excluded_card_payments = Vec::new();
    let mut undetermined_payments = Vec::new();
    let mut other_receipts = Vec::new();
    // Each chain's verdict on INVARIANT 6, by its first entry.
    let mut other: HashMap<EntryId, bool> = HashMap::new();
    for entry in ledger.posted_entries() {
        if !in_window(entry.date()) {
            continue;
        }
        let Some(vendor) = ledger.entry_vendor(entry.id()) else {
            continue;
        };
        let Some(v) = ledger.get_vendor(vendor) else {
            continue;
        };
        let root = roots.get(&entry.id()).copied().unwrap_or(entry.id());
        let root_entry = ledger.get_entry(root).unwrap_or(entry);
        let netted = entry_buckets(ledger, entry, &roles)?;
        // The vendor's part of each bucket.
        let shares = ledger.entry_parties(entry.id());
        let parts = if shares.len() < 2 {
            netted
        } else {
            let party = ledger.party_of_vendor(vendor);
            match crate::shares::resolve(ledger, entry.id(), shares, &money, members) {
                Ok(resolved) => match resolved.parts.iter().find(|p| Some(p.party) == party) {
                    Some(share) => split_share(
                        netted,
                        vendor_part(sum3(netted)?, share.amount, resolved.target.amount)?,
                    )?,
                    // Tagged by hand with a vendor none of the parties
                    // is: nothing to scale by, so the whole payment.
                    None => {
                        unresolved.push(entry.id());
                        netted
                    }
                },
                Err(_) => {
                    unresolved.push(entry.id());
                    netted
                }
            }
        };
        let [counts, card, unknown] = parts;
        let instrument = ledger.entry_instrument(entry.id());
        let payment = |amount| Payment1099 {
            entry_id: entry.id(),
            date: entry.date(),
            vendor_id: vendor,
            amount,
            instrument,
        };
        // INVARIANT 6: money received from the vendor for something else,
        // and its chain.
        let is_other = match other.get(&root) {
            Some(&o) => o,
            None => {
                let root_paid = sum3(entry_buckets(ledger, root_entry, &roles)?)?;
                let o = root_paid < Decimal::ZERO
                    && root_entry.lines().iter().any(|l| {
                        l.side == crate::Side::Credit
                            && !roles.pays(l.account)
                            && !refunds.contains(&l.account)
                    });
                other.insert(root, o);
                o
            }
        };
        if is_other {
            let net = sum3(parts)?;
            if !net.is_zero() {
                other_receipts.push(payment(net));
            }
            continue;
        }
        let t = tallies.entry(vendor).or_default();
        if !card.is_zero() {
            add(&mut t.card, card)?;
            excluded_card_payments.push(payment(card));
        }
        let decision = v
            .decision_on(root_entry.date())
            .cloned()
            .unwrap_or_default();
        match decision {
            Decision1099::NotReportable { .. } => {
                add(&mut t.not_reportable, counts)?;
                add(&mut t.not_reportable, unknown)?;
            }
            Decision1099::Files(_) => {
                add(&mut t.paid_files, counts)?;
                if !unknown.is_zero() {
                    add(t.unknown_files.entry(root).or_default(), unknown)?;
                    undetermined_payments.push(payment(unknown));
                }
            }
            Decision1099::Undetermined => {
                add(&mut t.paid_undecided, counts)?;
                if !unknown.is_zero() {
                    add(t.unknown_undecided.entry(root).or_default(), unknown)?;
                    undetermined_payments.push(payment(unknown));
                }
            }
        }
    }
    let empty = Tally::default();
    let mut vendors = Vec::new();
    for v in ledger.list_vendors() {
        let t = tallies.get(&v.id).unwrap_or(&empty);
        let decisions = decisions_during(&v.decisions_1099, start, end);
        let last = in_force(&v.decisions_1099, end);
        let decision = last.map(|d| d.decision.clone()).unwrap_or_default();
        let nonzero = |m: &BTreeMap<EntryId, Decimal>| m.values().any(|u| !u.is_zero());
        let has_files = !t.paid_files.is_zero() || nonzero(&t.unknown_files);
        let has_undecided = !t.paid_undecided.is_zero() || nonzero(&t.unknown_undecided);
        let has_exempt = !t.not_reportable.is_zero();
        let files_in_year = decisions.iter().any(|d| d.decision.form().is_some());
        if !(files_in_year || has_files || has_undecided || has_exempt || !t.card.is_zero()) {
            continue;
        }
        let (lo_f, hi_f) = bounds(t.paid_files, &t.unknown_files)?;
        let (_, hi_u) = bounds(t.paid_undecided, &t.unknown_undecided)?;
        let status = if !has_files && !has_undecided {
            if has_exempt || matches!(decision, Decision1099::NotReportable { .. }) {
                Status1099::NotReportable
            } else {
                Status1099::UnderThreshold
            }
        } else if has_undecided {
            let mut hi = hi_f;
            add(&mut hi, hi_u.max(Decimal::ZERO))?;
            if hi >= threshold {
                Status1099::Undetermined
            } else {
                Status1099::UnderThreshold
            }
        } else if lo_f >= threshold {
            Status1099::Reportable
        } else if hi_f < threshold {
            Status1099::UnderThreshold
        } else {
            Status1099::Undetermined
        };
        let mut paid = t.paid_files;
        add(&mut paid, t.paid_undecided)?;
        let mut undetermined = Decimal::ZERO;
        for &u in t.unknown_files.values().chain(t.unknown_undecided.values()) {
            add(&mut undetermined, u)?;
        }
        vendors.push(Vendor1099Line {
            vendor_id: v.id,
            name: v.name.clone(),
            decision,
            decided: last.and_then(|d| d.source.clone()),
            effective: last.and_then(|d| d.effective),
            decisions,
            // On file by the vendor's flag or its party's W-9 facts.
            tin_on_file: v.tin_on_file
                || ledger
                    .party_of_vendor(v.id)
                    .and_then(|p| ledger.get_party(p))
                    .is_some_and(|p| p.w9.is_some()),
            paid,
            excluded_card: t.card,
            undetermined,
            not_reportable: t.not_reportable,
            decision_needed: has_undecided && hi_u > Decimal::ZERO,
            status,
            reportable: status == Status1099::Reportable,
        });
    }
    vendors.sort_by(|a, b| a.name.cmp(&b.name).then(a.vendor_id.cmp(&b.vendor_id)));
    let untagged_payments = untagged_payments(ledger, &roles, &in_window)?;
    Ok(Vendor1099Report {
        year,
        threshold,
        through: (end < year_end).then_some(end),
        vendors,
        unresolved,
        excluded_card_payments,
        undetermined_payments,
        other_receipts,
        untagged_payments,
    })
}

/// The decisions in force at some time from `start` through `end`, in
/// date order, a decision the same as the one before it left out.
fn decisions_during(decisions: &[Dated1099], start: NaiveDate, end: NaiveDate) -> Vec<Dated1099> {
    let mut out: Vec<Dated1099> = Vec::new();
    let first = in_force(decisions, start);
    let later = decisions
        .iter()
        .filter(|d| d.effective.is_some_and(|e| e > start && e <= end));
    for d in first.into_iter().chain(later) {
        if out.last().map(|l| &l.decision) != Some(&d.decision) {
            out.push(d.clone());
        }
    }
    out
}

/// The quieter untagged note's entries (P3 review R7): posted entries in
/// the window with no vendor tag that paid money out (their netted counts
/// and undetermined buckets, as if tagged, above zero) for something (a
/// debit to an expense, to payables, or to an asset no money is paid out
/// of and no one owes), skipping payroll runs, entries marked as having
/// no counterparty, entries naming only parties with no 1099 role, and
/// entries reversed within the window.
fn untagged_payments(
    ledger: &Ledger,
    roles: &Roles,
    in_window: &impl Fn(NaiveDate) -> bool,
) -> Result<Vec<EntryId>, LedgerError> {
    let payroll: HashSet<EntryId> = ledger
        .list_payroll_runs()
        .into_iter()
        .map(|r| r.entry_id)
        .collect();
    let payables = ledger.role_account(&crate::PAYABLES);
    let receivables = ledger.role_account(&crate::RECEIVABLES);
    let reversed_in_window: HashSet<EntryId> = ledger
        .posted_entries()
        .filter(|e| in_window(e.date()))
        .filter_map(|e| e.reversal_of())
        .collect();
    let mut out = Vec::new();
    for e in ledger.posted_entries() {
        if !in_window(e.date())
            || ledger.entry_vendor(e.id()).is_some()
            || payroll.contains(&e.id())
            || ledger.entry_no_party(e.id()).is_some()
            || reversed_in_window.contains(&e.id())
        {
            continue;
        }
        let named = ledger.entry_parties(e.id());
        if !named.is_empty()
            && !named.iter().any(|s| {
                ledger
                    .get_party(s.party)
                    .is_some_and(|p| p.roles.iter().any(|r| r.gets_1099()))
            })
        {
            continue;
        }
        let [counts, _, unknown] = entry_buckets(ledger, e, roles)?;
        let out_of_pocket = counts
            .checked_add(unknown)
            .ok_or_else(|| LedgerError::TooLargeToCompute("a payment".into()))?;
        if out_of_pocket <= Decimal::ZERO {
            continue;
        }
        let pays_for_something = e.lines().iter().any(|l| {
            l.side == crate::Side::Debit
                && match ledger.chart().get(l.account).map(|a| a.account_type) {
                    Some(AccountType::Expense) => true,
                    Some(AccountType::Asset) => {
                        !roles.pays(l.account) && Some(l.account) != receivables
                    }
                    Some(AccountType::Liability) => Some(l.account) == payables,
                    _ => false,
                }
        });
        if pays_for_something {
            out.push(e.id());
        }
    }
    Ok(out)
}

/// The posted entries tagged with `vendor` whose decision in force would
/// change if `new` were recorded, each with its date (the year its 1099
/// is in): for the warning when a decision reaches a closed year, or one
/// whose 1099s were due (D-40, P3 review R15).
pub fn decision_changes(
    ledger: &Ledger,
    vendor: VendorId,
    new: &Dated1099,
) -> Result<Vec<(EntryId, NaiveDate)>, LedgerError> {
    let v = ledger
        .get_vendor(vendor)
        .ok_or(LedgerError::UnknownVendor(vendor))?;
    let mut after = v.decisions_1099.clone();
    apply_decision(&mut after, new.clone());
    let roots = chain_roots(ledger);
    let undecided = Decision1099::Undetermined;
    let mut out = Vec::new();
    for e in ledger.posted_entries() {
        if ledger.entry_vendor(e.id()) != Some(vendor) {
            continue;
        }
        let root = roots.get(&e.id()).copied().unwrap_or(e.id());
        let on = ledger.get_entry(root).map_or(e.date(), |r| r.date());
        let was = in_force(&v.decisions_1099, on).map_or(&undecided, |d| &d.decision);
        let will = in_force(&after, on).map_or(&undecided, |d| &d.decision);
        if was != will {
            out.push((e.id(), e.date()));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    /// vendor_part: the vendor's share scaled from the share target to the
    /// payment, exactly, rounded to the cent; refused, never a panic, when
    /// payment x share is past exact arithmetic (P0 review round 3, S1).
    #[test]
    fn a_vendor_part_is_scaled_exactly_or_refused() {
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

    /// split_share (P3 review R12), by hand. 100.00 netted as 60.00 counts
    /// and 40.00 card, the vendor's part 33.33: exactly 19.998 and 13.332,
    /// down to the cent 19.99 and 13.33, one cent short of 33.33, which
    /// goes to the larger remainder (0.008 against 0.002): 20.00 and
    /// 13.33. The parts always sum to the vendor's part, sign included.
    #[test]
    fn a_vendor_share_splits_across_buckets_to_the_cent() {
        let z = Decimal::ZERO;
        assert_eq!(
            split_share([d("60.00"), d("40.00"), z], d("33.33")),
            Ok([d("20.00"), d("13.33"), z])
        );
        assert_eq!(
            split_share([d("-60.00"), d("-40.00"), z], d("-33.33")),
            Ok([d("-20.00"), d("-13.33"), z])
        );
        // The whole payment: unchanged.
        assert_eq!(
            split_share([d("1"), d("2"), d("3")], d("6")),
            Ok([d("1"), d("2"), d("3")])
        );
        // A cent in thirds: the cent to the first of equal remainders.
        assert_eq!(
            split_share([d("1.00"), d("1.00"), d("1.00")], d("0.01")),
            Ok([d("0.01"), z, z])
        );
        // Nothing paid, or no share: nothing.
        assert_eq!(split_share([z, z, z], d("5")), Ok([z, z, z]));
        assert_eq!(split_share([d("5"), z, z], z), Ok([z, z, z]));
        // Past exact arithmetic: refused, never wrapped.
        let big = d("900000000000000");
        assert!(matches!(
            split_share([big, big, z], big),
            Err(LedgerError::TooLargeToCompute(_))
        ));
    }

    fn dated(effective: Option<&str>, decision: Decision1099) -> Dated1099 {
        Dated1099 {
            effective: effective.map(|s| s.parse().unwrap()),
            decision,
            source: None,
        }
    }

    /// plan_decision SCENARIOS 1-4, in_force and check_new_decision
    /// SCENARIO 1, by hand.
    #[test]
    fn dated_decisions_take_effect_on_their_dates() {
        let nec = Decision1099::Files(Form1099::Nec);
        let corp = Decision1099::not_reportable(ExemptReason::Corporation, "elected").unwrap();
        let mut ds = Vec::new();
        assert_eq!(
            apply_decision(&mut ds, dated(None, nec.clone())),
            Applied::Set
        );
        assert_eq!(
            apply_decision(&mut ds, dated(Some("2026-12-30"), corp.clone())),
            Applied::Set
        );
        let on = |ds: &[Dated1099], s: &str| {
            in_force(ds, s.parse().unwrap()).map(|d| d.decision.clone())
        };
        assert_eq!(on(&ds, "2026-12-29"), Some(nec.clone()));
        assert_eq!(on(&ds, "2026-12-30"), Some(corp.clone()));
        assert_eq!(on(&ds, "2030-01-01"), Some(corp.clone()));
        // Recorded exactly so: unchanged.
        assert_eq!(
            plan_decision(&ds, &dated(Some("2026-12-30"), corp.clone())),
            Applied::Unchanged
        );
        // A dated decision the same as before its date: no change there.
        assert_eq!(
            plan_decision(&ds, &dated(Some("2026-06-01"), nec.clone())),
            Applied::Unchanged
        );
        // Undoing the change: taken out.
        assert_eq!(
            apply_decision(&mut ds, dated(Some("2026-12-30"), nec.clone())),
            Applied::Removed
        );
        assert_eq!(ds, vec![dated(None, nec.clone())]);
        // Kept in date order, the undated first, one per date.
        apply_decision(&mut ds, dated(Some("2027-03-01"), corp.clone()));
        apply_decision(&mut ds, dated(Some("2026-03-01"), corp.clone()));
        let dates: Vec<_> = ds.iter().map(|d| d.effective).collect();
        assert_eq!(
            dates,
            [
                None,
                Some("2026-03-01".parse().unwrap()),
                Some("2027-03-01".parse().unwrap())
            ]
        );
        // Nothing decided before a dated decision: refused.
        let fresh: Vec<Dated1099> = vec![dated(None, Decision1099::Undetermined)];
        assert!(check_new_decision(&fresh, &dated(Some("2026-05-01"), nec.clone())).is_err());
        assert!(check_new_decision(&[], &dated(Some("2026-05-01"), nec.clone())).is_err());
        assert_eq!(
            check_new_decision(&fresh, &dated(None, nec.clone())),
            Ok(Applied::Set)
        );
    }

    #[test]
    fn decisions_say_what_they_are() {
        let corp =
            Decision1099::not_reportable(ExemptReason::Corporation, "  a corporation by name ")
                .unwrap();
        assert_eq!(corp.key(), "not_reportable");
        assert_eq!(corp.form(), None);
        assert_eq!(
            corp.describe(),
            "not reportable: a corporation (a corporation by name)"
        );
        assert_eq!(Decision1099::Files(Form1099::Misc).key(), "misc");
        assert_eq!(Decision1099::Undetermined.key(), "undetermined");
        for bad in ["", "two\nlines", "TIN 123-45-6789"] {
            assert!(
                Decision1099::not_reportable(ExemptReason::Other, bad).is_err(),
                "{bad:?}"
            );
        }
        let long = "x".repeat(crate::settings::MAX_TEXT + 1);
        assert!(Decision1099::not_reportable(ExemptReason::Other, &long).is_err());
        assert!(
            Decision1099::NotReportable {
                kind: ExemptReason::Other,
                reason: " padded".into()
            }
            .check()
            .is_err()
        );
        for r in ExemptReason::ALL {
            assert_eq!(ExemptReason::parse(r.key()), Some(r));
            assert_eq!(r.citation().is_none(), r == ExemptReason::Other);
        }
        assert_eq!(ExemptReason::parse("card"), None);
    }
}
