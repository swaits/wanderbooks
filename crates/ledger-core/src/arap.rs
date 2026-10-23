//! Accounts receivable and payable: customers, vendors, invoices, bills.
//!
//! Invoices and bills are business documents that auto-post journal entries:
//!
//! * `create_invoice`: Dr `1200` Accounts Receivable, Cr `4000` Revenue.
//! * `create_bill`: Dr `6999` General Expenses, Cr `2000` Accounts Payable.
//! * `receive_payment`: Dr Cash, Cr A/R.
//! * `pay_bill`: Dr A/P, Cr Cash.
//!
//! A document keeps no figure of its own (plan P4b, D-45; rules.md B-38):
//! every ledger line on a receivables or payables account carries the
//! document it belongs to ([`DocTag`]), and what is paid and still owed,
//! and the document's status, are read from those lines that stand
//! ([`DocState`], `Ledger::doc_state`), as of any date. A reversal, a void
//! or a bounced payment is therefore right by construction, and the
//! control accounts always equal their documents.
//!
//! The control accounts (`1200`, `2000`) and the default revenue/expense
//! accounts (`4000`, `6999`) are created on demand if missing. Invoices and
//! bills respect closed periods: they cannot be created with dates in a
//! closed period.

use crate::LedgerError;
use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// Opaque customer handle.
pub type CustomerId = u64;
/// Opaque vendor handle.
pub type VendorId = u64;
/// Opaque invoice handle.
pub type InvoiceId = u64;
/// Opaque bill handle.
pub type BillId = u64;

/// A customer: someone we invoice.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Customer {
    /// Opaque handle.
    pub id: CustomerId,
    /// Display name.
    pub name: String,
}

/// A vendor: someone who bills us.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Vendor {
    /// Opaque handle.
    pub id: VendorId,
    /// Display name.
    pub name: String,
    /// Its 1099 decisions, each from the date it takes effect, in date
    /// order (one from the beginning first); none when nobody has decided
    /// (see [`crate::form1099`], D-40).
    #[serde(default)]
    pub decisions_1099: Vec<crate::form1099::Dated1099>,
    /// A W-9 (taxpayer identification number) is on file. The number
    /// itself is never stored here.
    #[serde(default)]
    pub tin_on_file: bool,
}

impl Vendor {
    /// The 1099 decision in force on `date`, if any was recorded by then.
    pub fn decision_on(&self, date: NaiveDate) -> Option<&crate::form1099::Decision1099> {
        crate::form1099::in_force(&self.decisions_1099, date).map(|d| &d.decision)
    }

    /// The latest 1099 decision recorded (the one in force from the last
    /// effective date on), if any.
    pub fn latest_1099(&self) -> Option<&crate::form1099::Dated1099> {
        self.decisions_1099.last()
    }

    /// The return its latest decision files, if it files one.
    pub fn form_1099(&self) -> Option<crate::form1099::Form1099> {
        self.latest_1099().and_then(|d| d.decision.form())
    }
}

/// Payment status of an invoice or bill, in the three words the
/// documents' records carried before their figures were derived
/// ([`Invoice::status`]). The full status, a void or a write-off
/// included, is [`DocStatus`].
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum DocumentStatus {
    /// Nothing paid yet.
    Open,
    /// Partially paid.
    Partial,
    /// Fully paid.
    Paid,
}

/// A document: an invoice or a bill, the two kinds whose ledger lines
/// carry their document ([`DocTag`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum DocRef {
    /// A sales invoice (receivables).
    Invoice(InvoiceId),
    /// A vendor bill (payables).
    Bill(BillId),
}

impl std::fmt::Display for DocRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DocRef::Invoice(id) => write!(f, "invoice {id}"),
            DocRef::Bill(id) => write!(f, "bill {id}"),
        }
    }
}

impl DocRef {
    /// The side that counts plus for this kind of document: a debit to
    /// receivables raises what an invoice is owed, a credit to payables
    /// what a bill is owed.
    pub fn owed_side(self) -> crate::Side {
        match self {
            DocRef::Invoice(_) => crate::Side::Debit,
            DocRef::Bill(_) => crate::Side::Credit,
        }
    }

    /// A line's amount as this document counts it: plus on
    /// [`Self::owed_side`], minus on the other.
    pub fn counted(self, side: crate::Side, amount: Decimal) -> Decimal {
        if side == self.owed_side() {
            amount
        } else {
            -amount
        }
    }
}

/// Why a ledger line carries its document: what it does to what is owed.
/// Only `Origin` and `Payment` are written today (P4b S1); the rest are
/// the movements later slices write (P4b.6), named here so a tag's kind
/// never changes meaning.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Movement {
    /// The document's own control line: what it is for.
    Origin,
    /// Cash paid against it.
    Payment,
    /// An early-payment discount taken (S6).
    Discount,
    /// A bad debt written off (S7).
    WriteOff,
    /// A legacy hand line adopted as the document (S7).
    Adopted,
    /// The balance moved with its control role (S7).
    RoleMove,
}

/// A ledger line's document tag: which document the line belongs to, and
/// how ([`crate::Ledger::line_doc`]). Kept beside the line, like its
/// cleared flag: the line and its hash never change.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DocTag {
    /// The document.
    pub doc: DocRef,
    /// What the line does to it.
    pub movement: Movement,
}

/// A document's status as of a date ([`DocState`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocStatus {
    /// Nothing paid, discounted or written off.
    Open,
    /// Some of it settled, some still owed.
    Partial,
    /// Nothing owed, and some of it paid or discounted.
    Paid,
    /// Nothing owed, the last of it written off (S7).
    WrittenOff,
    /// Its own entry doesn't stand: reversed (B-9), number kept.
    Void,
    /// Less than nothing owed: more paid against it than it is for, a
    /// credit balance owed back (to the customer on an invoice, by the
    /// vendor on a bill). Only in books an earlier version left out of
    /// bounds (B-38 INVARIANT 2); never a receivable or a payable, so
    /// never aged.
    Overpaid,
    /// Its own entry stands but carries no line tagged with it (an older
    /// version's entry replay couldn't match): its figures can't be read
    /// from the ledger, and the control check lists it.
    Unattached,
}

impl DocStatus {
    /// Its wire name: `open`, `partial`, `paid`, `written_off`, `void`.
    pub fn key(self) -> &'static str {
        match self {
            DocStatus::Open => "open",
            DocStatus::Partial => "partial",
            DocStatus::Paid => "paid",
            DocStatus::WrittenOff => "written_off",
            DocStatus::Void => "void",
            DocStatus::Overpaid => "overpaid",
            DocStatus::Unattached => "unattached",
        }
    }

    /// Its label for people: `Open`, `Partly paid`, `Paid`, `Written
    /// off`, `Void`, `Overpaid`, `Not in the ledger`.
    pub fn label(self) -> &'static str {
        match self {
            DocStatus::Open => "Open",
            DocStatus::Partial => "Partly paid",
            DocStatus::Paid => "Paid",
            DocStatus::WrittenOff => "Written off",
            DocStatus::Void => "Void",
            DocStatus::Overpaid => "Overpaid",
            DocStatus::Unattached => "Not in the ledger",
        }
    }
}

/// What a document is, as of a date, read from the ledger lines tagged
/// with it ([`crate::Ledger::doc_state`]); never stored.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct DocState {
    /// The sum of its lines.
    pub total: Decimal,
    /// Cash paid against it that stands.
    pub paid: Decimal,
    /// Discounts taken that stand.
    pub discounted: Decimal,
    /// Amounts written off that stand.
    pub written_off: Decimal,
    /// What is still owed: the signed sum of every line tagged with it.
    pub outstanding: Decimal,
    /// Its status.
    pub status: DocStatus,
    /// Days past its due date while something is owed; 0 otherwise.
    pub overdue_days: i64,
}

impl DocState {
    /// It owes less than nothing or more than its total: books an earlier
    /// version left out of bounds (B-38 INVARIANT 2; plan P4b.14 item 3),
    /// which the control check lists for attention.
    pub fn out_of_bounds(&self) -> bool {
        self.outstanding < Decimal::ZERO || self.outstanding > self.total
    }
}

/// A document's status from its figures ([`DocState`]; plan P4b.3).
///
/// ```text
/// GOAL: one status from the ledger's figures, whatever happened to the
///       document, its payments and their reversals.
/// SOURCE: docs/spec/rules.md B-38; plan/served.md P4b.3 ("Status as of D");
///         ASC 310-10 (a credit balance is not a receivable).
/// INVARIANTS:
///   1. Void exactly when its own entry doesn't stand (`origin_stands`
///      false), whatever else is tagged.
///   2. Otherwise Unattached when its own control line carries no tag
///      (`origin_tagged` false): nothing about it can be read.
///   3. Otherwise: Overpaid when less than nothing is owed; Written off
///      when nothing is owed and something was written off; Paid when
///      nothing is owed; Partial when less than the total is owed; Open
///      when the whole total, or more (DocState::out_of_bounds), is.
///   4. A pure function of its arguments.
/// ```
pub fn doc_status(
    origin_stands: bool,
    origin_tagged: bool,
    total: Decimal,
    outstanding: Decimal,
    written_off: Decimal,
) -> DocStatus {
    if !origin_stands {
        DocStatus::Void
    } else if !origin_tagged {
        DocStatus::Unattached
    } else if outstanding < Decimal::ZERO {
        DocStatus::Overpaid
    } else if outstanding.is_zero() && written_off > Decimal::ZERO {
        DocStatus::WrittenOff
    } else if outstanding.is_zero() {
        DocStatus::Paid
    } else if outstanding < total {
        DocStatus::Partial
    } else {
        DocStatus::Open
    }
}

/// One posted ledger line tagged with a document ([`crate::Ledger::doc_lines`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct DocLine {
    /// The entry.
    pub entry: crate::EntryId,
    /// The line's index in it.
    pub line: usize,
    /// The entry's date.
    pub date: NaiveDate,
    /// The account the line is on.
    pub account: crate::AccountId,
    /// What it does to the document.
    pub movement: Movement,
    /// Its amount as the document counts it ([`DocRef::counted`]): plus
    /// raises what is owed.
    pub amount: Decimal,
    /// The entry reverses another (it is the mirror of a line).
    pub reversal: bool,
}

/// A line on a control account that carries no document: one written by
/// hand before the control guard, or by an older version (plan P4b.3,
/// "unattached").
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct UnattachedLine {
    /// The entry.
    pub entry: crate::EntryId,
    /// The line's index in it.
    pub line: usize,
    /// The entry's date.
    pub date: NaiveDate,
    /// Its amount as the control account counts it (plus raises the
    /// receivable or the payable).
    pub amount: Decimal,
}

/// One control account's tie-out as of a date ([`crate::Ledger::control_check`]).
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct ControlCheck {
    /// The role whose documents the account holds: `receivables` or
    /// `payables`.
    pub role: String,
    /// The account.
    pub account: crate::AccountId,
    /// Its balance in the general ledger, on its normal side.
    pub balance: Decimal,
    /// The sum of what its documents still owe on it.
    pub documents_total: Decimal,
    /// The lines on it that carry no document, which still stand.
    pub unattached: Vec<UnattachedLine>,
    /// balance - documents_total - the unattached lines: always zero (both
    /// sides sum the same lines); anything else is an engine error.
    pub difference: Decimal,
    /// The role's documents needing attention as of the day: out of
    /// bounds ([`DocState::out_of_bounds`]: a credit balance, which aging
    /// leaves out, or more owed than the total) or unattached
    /// ([`DocStatus::Unattached`]). Listed on the role's first account.
    pub attention: Vec<DocAttention>,
}

/// A document the control check lists for attention ([`ControlCheck::attention`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct DocAttention {
    /// The document.
    pub doc: DocRef,
    /// Its status as of the day.
    pub status: DocStatus,
    /// Its total.
    pub total: Decimal,
    /// What it owes as of the day.
    pub outstanding: Decimal,
}

/// How a payment is recorded beyond its amount (plan P4b.6, D-40): the
/// payment method it moved through, tagged on its entry in the same call
/// (so a card-paid bill is left out of 1099-NEC without a second call),
/// and the processor's or bank's own reference (a charge id, a trace or
/// check number), carried in the entry's description.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PaymentDetails {
    /// The payment method, when known.
    pub instrument: Option<crate::InstrumentId>,
    /// The reference, when there is one: one line of text.
    pub reference: Option<String>,
}

/// The longest payment reference taken, in characters.
pub const MAX_REFERENCE_CHARS: usize = 200;

impl PaymentDetails {
    /// The payment entry's description: `base`, then the reference when
    /// there is one (`base (ref R)`).
    pub fn description(&self, base: &str) -> String {
        match self.reference.as_deref().map(str::trim) {
            Some(r) if !r.is_empty() => format!("{base} (ref {r})"),
            _ => base.to_string(),
        }
    }

    /// Refuse a reference that isn't one line of at most
    /// [`MAX_REFERENCE_CHARS`] characters. A blank one is none. (A bank
    /// trace number is fifteen digits, so no nine-digit scan here.)
    pub fn check(&self) -> Result<(), LedgerError> {
        if let Some(r) = &self.reference
            && (r.contains(['\n', '\r']) || r.chars().count() > MAX_REFERENCE_CHARS)
        {
            return Err(LedgerError::InvalidPayment(format!(
                "a payment's reference is one line of at most {MAX_REFERENCE_CHARS} characters"
            )));
        }
        Ok(())
    }
}

/// One line on an invoice or bill.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DocumentLine {
    /// Description of the goods/services.
    pub description: String,
    /// Line amount (positive).
    pub amount: Decimal,
    /// The project (a class) the line's revenue or cost belongs to, if
    /// any (rules.md B-37): the posted entry's revenue or expense line for
    /// it carries the tag; the receivable or payable line never does.
    /// Documents journaled before the `document_line_projects` feature
    /// have none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class_id: Option<crate::ClassId>,
}

/// A sales invoice: we billed a customer.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Invoice {
    /// Opaque handle.
    pub id: InvoiceId,
    /// The customer.
    pub customer_id: CustomerId,
    /// Invoice date.
    pub date: NaiveDate,
    /// When payment is due.
    pub due_date: NaiveDate,
    /// Line items.
    pub lines: Vec<DocumentLine>,
    /// What was settled on it at the end of the books: paid, discounted
    /// and written off, as they stand ([`crate::Ledger::doc_state`] as of
    /// the last day). Derived from the lines tagged with it and refreshed
    /// by the ledger with every change (reversals included); never
    /// journaled, never state of its own. A void settles nothing, so a
    /// voided document never reads as paid.
    pub amount_paid: Decimal,
    /// What is still owed on it at the end of the books (`doc_state`'s
    /// outstanding as of the last day), derived with
    /// [`Self::amount_paid`]: [`Self::balance`]. Nothing for a void.
    /// Never serialized: a figure, not part of the record.
    #[serde(skip)]
    pub owed: Decimal,
    /// The three-word status of [`Self::amount_paid`]; derived with it
    /// (Open for a void: nothing was paid). The full status (a void, a
    /// write-off) is `doc_state`'s.
    pub status: DocumentStatus,
    /// The auto-posted journal entry.
    pub entry_id: u64,
}

/// The sum of a document's lines; refused (TooLargeToCompute), never a
/// panic, past exact arithmetic. A document is restored from its journal
/// line as written (rules.md B-28), so its lines are bounded only by what
/// the version that wrote it checked.
pub fn document_total(lines: &[DocumentLine], what: &str) -> Result<Decimal, LedgerError> {
    lines
        .iter()
        .try_fold(Decimal::ZERO, |sum, l| sum.checked_add(l.amount))
        .ok_or_else(|| LedgerError::TooLargeToCompute(format!("{what}'s total")))
}

/// The revenue (an invoice's) or expense (a bill's) side of a document's
/// entry: one part per project the lines name, in the order each first
/// appears, the untagged lines together as one part (`None`), each the
/// sum of its lines ([`document_total`]).
///
/// ```text
/// GOAL: a document's lines count toward their projects, and a document
///       whose lines name none posts exactly as before.
/// SOURCE: docs/spec/rules.md B-37 item 3 (job costing: the revenue and
///         direct costs identified with one job) and item 8 (an
///         invoice's or bill's line names its project).
/// INVARIANTS:
///   1. The parts sum to the document's total (conservation), and each
///      line is in exactly one part.
///   2. No two parts name the same project; no part is empty.
///   3. Lines naming no project make one part, so an untagged document is
///      one part: the one line every document posted before projects.
///   4. NEVER panics: TooLargeToCompute past exact arithmetic.
/// ```
pub fn document_parts(
    lines: &[DocumentLine],
    what: &str,
) -> Result<Vec<(Option<crate::ClassId>, Decimal)>, LedgerError> {
    let mut keys: Vec<Option<crate::ClassId>> = Vec::new();
    for l in lines {
        if !keys.contains(&l.class_id) {
            keys.push(l.class_id);
        }
    }
    keys.into_iter()
        .map(|k| {
            let part: Vec<DocumentLine> =
                lines.iter().filter(|l| l.class_id == k).cloned().collect();
            Ok((k, document_total(&part, what)?))
        })
        .collect()
}

impl Invoice {
    /// Total of all lines ([`document_total`]).
    pub fn total(&self) -> Result<Decimal, LedgerError> {
        document_total(&self.lines, &format!("invoice {}", self.id))
    }

    /// Amount still owed at the end of the books ([`Self::owed`]); as
    /// of a date, `Ledger::doc_state`. Refused (TooLargeToCompute) when
    /// its total can't be summed.
    pub fn balance(&self) -> Result<Decimal, LedgerError> {
        self.total()?;
        Ok(self.owed)
    }
}

/// A vendor bill: we owe a vendor.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Bill {
    /// Opaque handle.
    pub id: BillId,
    /// The vendor.
    pub vendor_id: VendorId,
    /// Bill date.
    pub date: NaiveDate,
    /// When payment is due.
    pub due_date: NaiveDate,
    /// Line items.
    pub lines: Vec<DocumentLine>,
    /// What was settled on it at the end of the books: paid, discounted
    /// and written off, as they stand ([`crate::Ledger::doc_state`] as of
    /// the last day). Derived from the lines tagged with it and refreshed
    /// by the ledger with every change (reversals included); never
    /// journaled, never state of its own. A void settles nothing, so a
    /// voided document never reads as paid.
    pub amount_paid: Decimal,
    /// What is still owed on it at the end of the books (`doc_state`'s
    /// outstanding as of the last day), derived with
    /// [`Self::amount_paid`]: [`Self::balance`]. Nothing for a void.
    /// Never serialized: a figure, not part of the record.
    #[serde(skip)]
    pub owed: Decimal,
    /// The three-word status of [`Self::amount_paid`]; derived with it
    /// (Open for a void: nothing was paid). The full status (a void, a
    /// write-off) is `doc_state`'s.
    pub status: DocumentStatus,
    /// The auto-posted journal entry.
    pub entry_id: u64,
}

impl Bill {
    /// Total of all lines ([`document_total`]).
    pub fn total(&self) -> Result<Decimal, LedgerError> {
        document_total(&self.lines, &format!("bill {}", self.id))
    }

    /// Amount still owed at the end of the books ([`Self::owed`]); as
    /// of a date, `Ledger::doc_state`. Refused (TooLargeToCompute) when
    /// its total can't be summed.
    pub fn balance(&self) -> Result<Decimal, LedgerError> {
        self.total()?;
        Ok(self.owed)
    }
}

/// Validate document lines: at least one, all positive amounts, each in a
/// journal line's range. For new documents; replay restores them as
/// journaled (rules.md B-28).
pub fn validate_lines(lines: &[DocumentLine]) -> Result<(), LedgerError> {
    if lines.is_empty() {
        return Err(LedgerError::EmptyDocumentLines);
    }
    for line in lines {
        if line.amount <= Decimal::ZERO {
            return Err(LedgerError::InvalidDocumentLineAmount(line.amount));
        }
        // No larger than a journal line may be (kernel::MAX_LINE_AMOUNT):
        // the document's total is summed from them (P0 review round 3, S1).
        if !crate::kernel::Amount::in_line_range(line.amount) {
            return Err(LedgerError::ValueOutOfRange(format!(
                "document line amount {}",
                line.amount
            )));
        }
    }
    Ok(())
}

/// Validate that due date is not before the document date, and both are
/// in the calendar the books keep (P0 review round 3, S1). For new
/// documents; replay restores them as journaled (rules.md B-28).
pub fn validate_due_date(date: NaiveDate, due_date: NaiveDate) -> Result<(), LedgerError> {
    crate::calendar::check(date)?;
    crate::calendar::check(due_date)?;
    if due_date < date {
        return Err(LedgerError::DueBeforeDate(due_date.to_string()));
    }
    Ok(())
}

/// The three-word status from what is settled vs the total
/// ([`DocumentStatus`]).
pub fn derive_status(paid: Decimal, total: Decimal) -> DocumentStatus {
    if paid.is_zero() {
        DocumentStatus::Open
    } else if paid >= total {
        DocumentStatus::Paid
    } else {
        DocumentStatus::Partial
    }
}

/// One row of an aging report.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AgingRow {
    /// Customer or vendor name.
    pub name: String,
    /// Not yet due.
    pub current: Decimal,
    /// 1-30 days past due.
    pub days_30: Decimal,
    /// 31-60 days past due.
    pub days_60: Decimal,
    /// 61-90 days past due.
    pub days_90: Decimal,
    /// Over 90 days past due.
    pub days_90_plus: Decimal,
}

impl AgingRow {
    /// Total outstanding; refused past exact arithmetic.
    pub fn total(&self) -> Result<Decimal, LedgerError> {
        [self.days_30, self.days_60, self.days_90, self.days_90_plus]
            .into_iter()
            .try_fold(self.current, Decimal::checked_add)
            .ok_or_else(|| LedgerError::TooLargeToCompute(format!("{}'s aging total", self.name)))
    }

    /// Add `other`'s buckets to this row's; refused past exact arithmetic.
    pub(crate) fn add(&mut self, other: &AgingRow) -> Result<(), LedgerError> {
        let too_large = || LedgerError::TooLargeToCompute(format!("{}'s aging", self.name));
        let sum = |a: Decimal, b: Decimal| a.checked_add(b).ok_or_else(too_large);
        let (current, days_30, days_60, days_90, days_90_plus) = (
            sum(self.current, other.current)?,
            sum(self.days_30, other.days_30)?,
            sum(self.days_60, other.days_60)?,
            sum(self.days_90, other.days_90)?,
            sum(self.days_90_plus, other.days_90_plus)?,
        );
        (
            self.current,
            self.days_30,
            self.days_60,
            self.days_90,
            self.days_90_plus,
        ) = (current, days_30, days_60, days_90, days_90_plus);
        Ok(())
    }
}

/// Put an open balance in its aging bucket.
///
/// ```text
/// GOAL: say how overdue a balance is, as of a date.
/// SOURCE: docs/spec/rules.md B-18 (bucket edges: CONVENTION, ASC
///         326-20-55-38 illustrative; due today is Current: DECISION,
///         owner 2026-10-13).
/// INVARIANTS:
///   1. MUST put the whole balance in exactly one bucket; the others are
///      zero.
///   2. NEVER moves a balance to a younger bucket as `as_of` advances.
/// SCENARIOS:
///   1. IF `as_of` is on or before the due date:
///      - SHOULD be Current
///      - NEVER past due
///      BECAUSE a document due today is not yet overdue (QuickBooks'
///      convention; owner, 2026-10-13).
///   2. IF it is 1 to 30 days past due: SHOULD be 1-30.
///   3. IF 31 to 60: SHOULD be 31-60.
///   4. IF 61 to 90: SHOULD be 61-90.
///   5. IF more than 90: SHOULD be 90+.
///      BECAUSE these are the standard 30-day aging buckets.
/// ```
pub fn aging_bucket(due_date: NaiveDate, as_of: NaiveDate, balance: Decimal) -> AgingRow {
    let mut row = AgingRow {
        name: String::new(),
        current: Decimal::ZERO,
        days_30: Decimal::ZERO,
        days_60: Decimal::ZERO,
        days_90: Decimal::ZERO,
        days_90_plus: Decimal::ZERO,
    };
    if due_date >= as_of {
        row.current = balance;
    } else {
        let days = (as_of - due_date).num_days();
        if days <= 30 {
            row.days_30 = balance;
        } else if days <= 60 {
            row.days_60 = balance;
        } else if days <= 90 {
            row.days_90 = balance;
        } else {
            row.days_90_plus = balance;
        }
    }
    row
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AccountType, Currency, Ledger, Money, Side};

    fn arap_ledger() -> Ledger {
        let mut l = Ledger::new(Currency::USD);
        l.add_account("1000", "Cash", AccountType::Asset, None, false)
            .unwrap();
        l
    }

    #[test]
    fn invoice_lifecycle() {
        let mut l = arap_ledger();
        let cust = l.add_customer("Acme Corp").unwrap();
        let inv = l
            .create_invoice(
                cust,
                NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
                NaiveDate::from_ymd_opt(2026, 10, 31).unwrap(),
                vec![DocumentLine {
                    description: "Widgets".to_string(),
                    amount: "1000.00".parse().unwrap(),
                    class_id: None,
                }],
            )
            .unwrap();
        let invoice = l.get_invoice(inv).unwrap();
        assert_eq!(invoice.total().unwrap(), "1000.00".parse().unwrap());
        assert_eq!(invoice.status, DocumentStatus::Open);

        // A/R has a debit balance of 1000.
        let ar = l.chart().find_by_code("1200").unwrap().id;
        assert_eq!(l.account_balance(ar).unwrap(), "1000".parse().unwrap());

        // Partial payment.
        let cash = l.chart().find_by_code("1000").unwrap().id;
        l.receive_payment(
            inv,
            NaiveDate::from_ymd_opt(2026, 10, 15).unwrap(),
            "400.00".parse().unwrap(),
            cash,
        )
        .unwrap();
        let invoice = l.get_invoice(inv).unwrap();
        assert_eq!(invoice.status, DocumentStatus::Partial);
        assert_eq!(invoice.balance().unwrap(), "600.00".parse().unwrap());

        // Full payment.
        l.receive_payment(
            inv,
            NaiveDate::from_ymd_opt(2026, 10, 20).unwrap(),
            "600.00".parse().unwrap(),
            cash,
        )
        .unwrap();
        let invoice = l.get_invoice(inv).unwrap();
        assert_eq!(invoice.status, DocumentStatus::Paid);
        assert_eq!(l.account_balance(ar).unwrap(), Decimal::ZERO);
    }

    #[test]
    fn bill_lifecycle() {
        let mut l = arap_ledger();
        let vend = l.add_vendor("Supply Co").unwrap();
        let bill = l
            .create_bill(
                vend,
                NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
                NaiveDate::from_ymd_opt(2026, 10, 31).unwrap(),
                vec![DocumentLine {
                    description: "Paper".to_string(),
                    amount: "250.00".parse().unwrap(),
                    class_id: None,
                }],
            )
            .unwrap();
        let ap = l.chart().find_by_code("2000").unwrap().id;
        // A/P is credit-normal; balance should be 250 (credit).
        assert_eq!(l.account_balance(ap).unwrap(), "250".parse().unwrap());

        let cash = l.chart().find_by_code("1000").unwrap().id;
        // Seed cash so the payment doesn't go negative (not required, but tidy).
        let e = l.create_entry(NaiveDate::from_ymd_opt(2026, 9, 1).unwrap(), "seed");
        let m = Money::new("1000.00".parse().unwrap(), Currency::USD);
        l.add_line(e, cash, Side::Debit, m).unwrap();
        let eq = l
            .add_account("3000", "Equity", AccountType::Equity, None, false)
            .unwrap();
        l.add_line(e, eq, Side::Credit, m).unwrap();
        l.post_entry(e).unwrap();

        l.pay_bill(
            bill,
            NaiveDate::from_ymd_opt(2026, 10, 15).unwrap(),
            "250.00".parse().unwrap(),
            cash,
        )
        .unwrap();
        assert_eq!(l.get_bill(bill).unwrap().status, DocumentStatus::Paid);
        assert_eq!(l.account_balance(ap).unwrap(), Decimal::ZERO);
    }

    #[test]
    fn overpayment_rejected() {
        let mut l = arap_ledger();
        let cust = l.add_customer("Acme").unwrap();
        let inv = l
            .create_invoice(
                cust,
                NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
                NaiveDate::from_ymd_opt(2026, 10, 31).unwrap(),
                vec![DocumentLine {
                    description: "X".to_string(),
                    amount: "100.00".parse().unwrap(),
                    class_id: None,
                }],
            )
            .unwrap();
        let cash = l.chart().find_by_code("1000").unwrap().id;
        let err = l.receive_payment(
            inv,
            NaiveDate::from_ymd_opt(2026, 10, 15).unwrap(),
            "150.00".parse().unwrap(),
            cash,
        );
        assert!(matches!(err, Err(LedgerError::Overpayment(_, _))));
    }

    #[test]
    fn aging_boundaries() {
        // Spec scenarios 1-5, at every edge.
        let due = NaiveDate::from_ymd_opt(2026, 3, 31).unwrap();
        let b = Decimal::from(7);
        let bucket = |days: i64| {
            let r = aging_bucket(due, due + chrono::Duration::days(days), b);
            [r.current, r.days_30, r.days_60, r.days_90, r.days_90_plus]
                .iter()
                .position(|x| !x.is_zero())
                .unwrap()
        };
        for (days, want) in [
            (-10, 0),
            (0, 0),
            (1, 1),
            (30, 1),
            (31, 2),
            (60, 2),
            (61, 3),
            (90, 3),
            (91, 4),
            (4000, 4),
        ] {
            assert_eq!(bucket(days), want, "{days} days past due");
        }
    }

    proptest::proptest! {
        /// Invariants 1 and 2: one bucket holds the whole balance, and
        /// time only ever ages it.
        #[test]
        fn aging_is_exclusive_and_only_ages(
            due_off in 0i64..2000, a in 0i64..2000, step in 0i64..400, cents in 1i64..10_000_000,
        ) {
            let base = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
            let due = base + chrono::Duration::days(due_off);
            let b = Decimal::new(cents, 2);
            let idx = |as_of: NaiveDate| {
                let r = aging_bucket(due, as_of, b);
                let cells = [r.current, r.days_30, r.days_60, r.days_90, r.days_90_plus];
                let held: Vec<usize> = (0..5).filter(|&i| !cells[i].is_zero()).collect();
                proptest::prop_assert_eq!(held.len(), 1);
                proptest::prop_assert_eq!(cells[held[0]], b);
                Ok(held[0])
            };
            let early = base + chrono::Duration::days(a);
            let later = early + chrono::Duration::days(step);
            proptest::prop_assert!(idx(early)? <= idx(later)?);
        }
    }

    #[test]
    fn ar_aging_buckets_correctly() {
        let mut l = arap_ledger();
        let cust = l.add_customer("Acme").unwrap();
        // Due in the future: current.
        l.create_invoice(
            cust,
            NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 11, 15).unwrap(),
            vec![DocumentLine {
                description: "A".to_string(),
                amount: "100.00".parse().unwrap(),
                class_id: None,
            }],
        )
        .unwrap();
        // 45 days past due: 60 bucket.
        l.create_invoice(
            cust,
            NaiveDate::from_ymd_opt(2026, 7, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 8, 17).unwrap(),
            vec![DocumentLine {
                description: "B".to_string(),
                amount: "200.00".parse().unwrap(),
                class_id: None,
            }],
        )
        .unwrap();
        let as_of = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
        let aging = l.ar_aging(as_of).unwrap();
        assert_eq!(aging.len(), 1);
        assert_eq!(aging[0].current, "100".parse().unwrap());
        assert_eq!(aging[0].days_60, "200".parse().unwrap());
    }

    #[test]
    fn invoice_in_closed_period_rejected() {
        let mut l = arap_ledger();
        let cust = l.add_customer("Acme").unwrap();
        let pid = l
            .create_period(
                "2026-10",
                NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
                NaiveDate::from_ymd_opt(2026, 10, 31).unwrap(),
            )
            .unwrap();
        l.close_period(pid).unwrap();
        let err = l.create_invoice(
            cust,
            NaiveDate::from_ymd_opt(2026, 10, 15).unwrap(),
            NaiveDate::from_ymd_opt(2026, 11, 15).unwrap(),
            vec![DocumentLine {
                description: "X".to_string(),
                amount: "100.00".parse().unwrap(),
                class_id: None,
            }],
        );
        assert!(matches!(err, Err(LedgerError::ClosedPeriod(_, _))));
    }
}
