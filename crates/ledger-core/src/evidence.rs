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

use crate::LedgerError;

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
