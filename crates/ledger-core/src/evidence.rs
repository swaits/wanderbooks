//! What the books record about their evidence beyond the file itself
//! (plan P6 step 5): a receipt's own facts (its number, payee, amount,
//! date, instrument, where it came from, who is to be reimbursed) and
//! which entries a payment settles.
//!
//! ```text
//! GOAL: an annotation is a record of what a human read off a file, kept
//!       apart from the file and the ledger, and a settlement is the one
//!       record from which "the member was paid back" is derived.
//! SOURCE: plan/served.md P6 step 5 (defect 10); IZ-18 (no FIFO guess:
//!         `entry.settles` only); IRS Pub. 583 (records are kept with what
//!         they support).
//! INVARIANTS:
//!   1. An annotation's fields are a closed list; every one is optional,
//!      and at least one field or the note is present.
//!   2. A receipt number runs from the highest one annotated
//!      ([`crate::Ledger::next_receipt_number`]).
//!   3. A settlement names existing entries, never itself, never one
//!      twice; it records nothing about amounts.
//! ```

use chrono::NaiveDate;
use rust_decimal::Decimal;

use crate::settings::Settings;
use crate::{EntryId, EntryStatus, JournalEntry, Ledger, LedgerError, Side};

/// The closed set of facts a human may record about a file (plan P6 step
/// 5). Every field is optional; the amount is an exact decimal in the
/// books' own currency, and the date is the date on the document.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AnnotationFields {
    /// The number printed on the receipt, unique in its own series only.
    pub receipt_number: Option<u32>,
    /// Who was paid.
    pub payee: Option<String>,
    /// The amount printed on the receipt, exact.
    pub amount: Option<Decimal>,
    /// The date printed on the receipt.
    pub date: Option<NaiveDate>,
    /// How it was paid: a payment method's id or label.
    pub instrument: Option<String>,
    /// What kind of record it is, as the document categories name them.
    pub category: Option<String>,
    /// Where it came from (a folder, a person, an email printed to PDF).
    pub source: Option<String>,
    /// Who is to be reimbursed for it, if anyone.
    pub reimbursable_to: Option<String>,
}

impl AnnotationFields {
    /// Whether nothing at all is recorded.
    pub fn is_empty(&self) -> bool {
        self.receipt_number.is_none()
            && self.payee.is_none()
            && self.amount.is_none()
            && self.date.is_none()
            && self.instrument.is_none()
            && self.category.is_none()
            && self.source.is_none()
            && self.reimbursable_to.is_none()
    }

    /// Check the fields before they are journaled (INVARIANTS 1 and 2):
    /// every text trimmed and non-empty, the number positive, the amount
    /// not negative, the date in the books' calendar. Returns them as they
    /// will be kept.
    pub fn checked(mut self) -> Result<Self, LedgerError> {
        let trim = |s: Option<String>| -> Option<String> {
            s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
        };
        self.payee = trim(self.payee);
        self.instrument = trim(self.instrument);
        self.category = trim(self.category);
        self.source = trim(self.source);
        self.reimbursable_to = trim(self.reimbursable_to);
        if self.receipt_number == Some(0) {
            return Err(LedgerError::InvalidEvidence(
                "a receipt number is positive".into(),
            ));
        }
        if self.amount.is_some_and(|a| a.is_sign_negative()) {
            return Err(LedgerError::InvalidEvidence(
                "an annotated amount is not negative".into(),
            ));
        }
        if let Some(d) = self.date {
            crate::calendar::check(d)?;
        }
        Ok(self)
    }
}

/// One recorded annotation: the file, the facts read off it, and an
/// optional note.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Annotation {
    /// The attachment the facts are about.
    pub attachment_id: u64,
    /// The facts.
    pub fields: AnnotationFields,
    /// A free note.
    pub note: Option<String>,
}

impl Annotation {
    /// Check the annotation before it is journaled (INVARIANTS 1 and 2).
    pub fn checked(mut self) -> Result<Self, LedgerError> {
        self.fields = self.fields.checked()?;
        self.note = self
            .note
            .map(|n| n.trim().to_string())
            .filter(|n| !n.is_empty());
        if self.fields.is_empty() && self.note.is_none() {
            return Err(LedgerError::InvalidEvidence(
                "an annotation records at least one field or a note".into(),
            ));
        }
        Ok(self)
    }
}

// ---------------------------------------------------------------------------
// The evidence gate (plan P4, I-5, D-47; rules.md B-43)
// ---------------------------------------------------------------------------

/// What posted an entry that the ledger's own state doesn't otherwise say:
/// recorded as the journal records it today (`asset.disposed` and
/// `payroll.taxes_remitted` name their entry), so a reader of the ledger
/// alone can tell it from a hand-made entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Origin {
    /// An asset's disposal entry.
    AssetDisposal,
    /// A payroll tax remittance entry.
    PayrollRemittance,
}

/// Whether an entry needs evidence before it posts, and why (or why not).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Requirement {
    /// The receipt policy covers it: at least one receipt is needed.
    Required {
        /// Why, in words for a refusal or a checklist.
        reason: String,
    },
    /// It needs none of its own.
    Exempt {
        /// What exempts it: the origin or the policy line, with its rule.
        source: &'static str,
    },
}

impl Requirement {
    /// Whether evidence is required.
    pub fn is_required(&self) -> bool {
        matches!(self, Requirement::Required { .. })
    }
}

/// The receipt policy as the books' settings set it
/// (`policy.evidence.*`): required strictly above the threshold, on a
/// required account regardless of amount, never when every line is on an
/// exempt account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Policy {
    /// Receipts are required when an entry's debits are strictly greater
    /// than this.
    pub threshold: Decimal,
    /// Account codes whose entries always need a receipt.
    pub required_accounts: Vec<String>,
    /// Account codes exempt from the policy; the exemption applies only
    /// when every line is on one.
    pub exempt_accounts: Vec<String>,
}

impl Policy {
    /// The policy the books' own settings set (the threshold defaults to
    /// 25.00; one that is not a decimal at least zero requires a receipt
    /// on everything, as an unvalidated policy always did).
    pub fn from_settings(s: &Settings) -> Policy {
        Policy {
            threshold: crate::settings::decimal(
                s.receipt_threshold()
                    .unwrap_or(crate::settings::DEFAULT_THRESHOLD)
                    .trim(),
            )
            .unwrap_or(Decimal::ZERO),
            required_accounts: s.receipt_required_accounts(),
            exempt_accounts: s.receipt_exempt_accounts(),
        }
    }

    /// Total debits of the entry (one side of the balanced entry). A
    /// draft's lines are as journaled, of any size (rules.md B-28): a
    /// total past exact arithmetic is refused (TooLargeToCompute), never
    /// a panic.
    pub fn entry_debits(entry: &JournalEntry) -> Result<Decimal, LedgerError> {
        entry
            .lines()
            .iter()
            .filter(|l| l.side == Side::Debit)
            .try_fold(Decimal::ZERO, |sum, l| sum.checked_add(l.amount.amount))
            .ok_or_else(|| LedgerError::TooLargeToCompute(format!("entry {}'s debits", entry.id())))
    }

    fn account_codes(ledger: &Ledger, entry: &JournalEntry) -> Vec<String> {
        entry
            .lines()
            .iter()
            .map(|l| {
                ledger
                    .chart()
                    .get(l.account)
                    .map(|a| a.code.clone())
                    .unwrap_or_default()
            })
            .collect()
    }

    /// True when an entry touching these account codes with this debit
    /// total requires at least one receipt (the code-level check behind
    /// [`requirement`], for postings computed without a draft in hand,
    /// like a disposal's).
    pub fn receipt_required_for(&self, codes: &[&str], debits: Decimal) -> bool {
        self.reason_for(codes, debits).is_some()
    }

    /// [`Self::receipt_required_for`], with why.
    fn reason_for(&self, codes: &[&str], debits: Decimal) -> Option<String> {
        // Per-account override: any line in a required account.
        if let Some(c) = codes
            .iter()
            .find(|c| self.required_accounts.iter().any(|r| r == **c))
        {
            return Some(format!(
                "it touches account {c}, which always needs a receipt"
            ));
        }
        // Strictly greater than the threshold (owner: ">$25").
        if debits <= self.threshold {
            return None;
        }
        // Exempt only when every line is in an exempt account.
        if codes
            .iter()
            .all(|c| self.exempt_accounts.iter().any(|e| e == *c))
        {
            return None;
        }
        Some(format!(
            "its debits {debits} are strictly greater than the {} threshold",
            self.threshold
        ))
    }

    /// True when the policy, entry by entry, requires a receipt for
    /// `entry`: debits past exact arithmetic are past any threshold.
    pub fn receipt_required(&self, ledger: &Ledger, entry: &JournalEntry) -> bool {
        self.reason(ledger, entry).is_some()
    }

    fn reason(&self, ledger: &Ledger, entry: &JournalEntry) -> Option<String> {
        let codes = Self::account_codes(ledger, entry);
        let refs: Vec<&str> = codes.iter().map(String::as_str).collect();
        let debits = Self::entry_debits(entry).unwrap_or(Decimal::MAX);
        self.reason_for(&refs, debits)
    }

    /// Human-readable policy summary for previews and errors.
    pub fn describe(&self) -> String {
        if self.threshold.is_zero() {
            "receipts required on every entry".to_string()
        } else {
            format!(
                "receipts required when entry debits are strictly greater than {}",
                self.threshold
            )
        }
    }

    fn normalize_description(desc: &str) -> String {
        desc.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    }

    fn debit_accounts(entry: &JournalEntry) -> Vec<u64> {
        let mut ids: Vec<u64> = entry
            .lines()
            .iter()
            .filter(|l| l.side == Side::Debit)
            .map(|l| l.account)
            .collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    }

    /// Detect a potential threshold split: posted entries on the same
    /// date with the same normalized description and debit accounts,
    /// whose combined debits with this entry exceed the threshold while
    /// this entry alone does not. Returns the matching posted entry ids.
    ///
    /// This is the anti-gaming rule: splitting a $100 purchase into four
    /// $25 entries does not dodge the receipt policy. Same-day,
    /// same-description, same-accounts is the signature; recurring
    /// spending spread across days is not flagged.
    pub fn split_group(
        &self,
        ledger: &Ledger,
        entry_id: EntryId,
        entry: &JournalEntry,
    ) -> Vec<u64> {
        // Debits past exact arithmetic are past the threshold too.
        let debits = Self::entry_debits(entry).unwrap_or(Decimal::MAX);
        if debits > self.threshold {
            return Vec::new(); // Normal policy already applies.
        }
        let desc = Self::normalize_description(entry.description());
        let accounts = Self::debit_accounts(entry);
        let date = entry.date();
        let mut group = Vec::new();
        let mut total = debits;
        for other in ledger.posted_entries() {
            if other.id() == entry_id
                || other.date() != date
                || Self::normalize_description(other.description()) != desc
                || Self::debit_accounts(other) != accounts
            {
                continue;
            }
            // Reversals are exempt from the policy; don't count them.
            if other.reversal_of().is_some() {
                continue;
            }
            group.push(other.id());
            // A sum past exact arithmetic is past the threshold.
            total = Self::entry_debits(other)
                .ok()
                .and_then(|d| total.checked_add(d))
                .unwrap_or(Decimal::MAX);
        }
        if !group.is_empty() && total > self.threshold {
            group
        } else {
            Vec::new()
        }
    }
}

/// Whether `entry` needs evidence before it posts: the one function that
/// decides (plan P4 step 2, I-5, D-47; rules.md B-43). The store's gate,
/// the server's previews and digest, the viewer and Needs attention all
/// call it.
///
/// ```text
/// GOAL: one answer, from what the ledger knows, to "does this entry need
///       a receipt".
/// SOURCE: docs/spec/rules.md B-43; IRS Pub. 583 (supporting documents);
///         DECISION D-47 (a reversal's evidence is its original's, D-12);
///         DECISION (owner: the threshold, strictly greater than, 2026-10-12).
/// INVARIANTS:
///   1. Exempt by origin, each with its source: a closing entry, a
///      schedule month's posting, a depreciation run, an asset's
///      disposal, an entry a document command posted (any line tagged
///      with a document), a payroll run or its tax remittance, and a
///      reversal. Every other entry, hand entries and recurring drafts
///      included, is the policy's.
///   2. The policy's: Required when it touches a required account, or its
///      debits are strictly greater than the threshold and some line is
///      off the exempt accounts; a DRAFT is also Required when it joins a
///      split group (same day, description and debit accounts as posted
///      entries whose total passes the threshold). A posted entry's
///      decision was made when it posted and is not made again from its
///      neighbours.
///   3. Pure: nothing changes.
/// SCENARIOS:
///   1. A reversal of a 5,000.00 hand entry: Exempt (D-47).
///   2. A 40.00 hand entry at the default 25.00 threshold: Required.
///   3. The same under threshold 0.00 on an exempt account: Exempt.
/// ```
pub fn requirement(ledger: &Ledger, entry: &JournalEntry, settings: &Settings) -> Requirement {
    let policy = Policy::from_settings(settings);
    let mut reason = policy.reason(ledger, entry);
    if reason.is_none() && entry.status() == EntryStatus::Draft {
        let group = policy.split_group(ledger, entry.id(), entry);
        if !group.is_empty() {
            reason = Some(format!(
                "it looks like part of a split group with entries {group:?}; the policy applies to the group total"
            ));
        }
    }
    let Some(reason) = reason else {
        return Requirement::Exempt {
            source: "the receipt policy (policy.evidence.*): under the threshold, or on exempt accounts",
        };
    };
    match ledger.origin_exemption(entry) {
        Some(source) => Requirement::Exempt { source },
        None => Requirement::Required { reason },
    }
}
