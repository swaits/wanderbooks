//! Accounts receivable and payable: customers, vendors, invoices, bills.
//!
//! Invoices and bills are business documents that auto-post journal entries:
//!
//! * `create_invoice`: Dr `1200` Accounts Receivable, Cr each line's account
//!   (the revenue role, `4000`, when a line names none).
//! * `create_bill`: Dr each line's account (the general-expenses role,
//!   `6999`, when a line names none), Cr `2000` Accounts Payable.
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
use chrono::{Datelike, Days, NaiveDate};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// The most days a payment term may run: ten years, far beyond any trade
/// term, and short of the calendar's end for every document date the
/// books can hold (B-39).
pub const MAX_TERMS_DAYS: u16 = 3_650;

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

/// The two kinds of document (an invoice or a bill), without an id: what
/// a line's account rule and the journal's event names key on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentKind {
    /// A sales invoice (receivables).
    Invoice,
    /// A vendor bill (payables).
    Bill,
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

/// How a document's payment terms count their days (rules.md B-39; plan
/// P4b S4.1): from the document's date, from the end of its month, or a
/// date the parties agreed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TermsKind {
    /// Due on the document's own date.
    DueOnReceipt,
    /// Due `n` days after the document's date (Net n).
    Net(u16),
    /// Due `n` days after the last day of the document's month (Net n
    /// EOM).
    NetEom(u16),
    /// Due on a date the parties agreed; the caller passes it to
    /// [`Terms::due_date`], and [`Terms::describe`] carries it for
    /// people.
    Custom,
}

/// An early-payment discount: take `percent_bp` off if paid within
/// `days` of the document's date ("2/10 net 30").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EarlyDiscount {
    /// The discount in basis points: 1 to 9,999 (0.01% to 99.99%).
    pub percent_bp: u32,
    /// The days after the document's date the discount is open.
    pub days: u16,
}

/// Payment terms: one due date, and the early-payment discount open
/// before it (rules.md B-39; plan P4b S4.1, A-2, A-8).
///
/// ```text
/// GOAL: terms give one due date and one discount date, exactly.
/// SOURCE: docs/spec/rules.md B-39; plan/served.md 8.3.13 (B-39);
///         CONVENTION: standard trade-terms notation; UCC §2-310(a) for
///         goods; for services, the contract.
/// INVARIANTS:
///   1. Net n = date + n; Net n EOM = the last day of date's month + n;
///      due on receipt = date; Custom = the given date. Every result is
///      >= the document date and in the calendar (calendar::check); n is
///      0..=3650.
///   2. discount_by = date + d, d < the net days; no discount with due on
///      receipt, EOM or custom terms.
///   3. describe() is the canonical text; parse() accepts it and the
///      canonical spellings, case-insensitively, and nothing else.
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Terms {
    /// How the due date counts.
    kind: TermsKind,
    /// The early-payment discount, when there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    discount: Option<EarlyDiscount>,
    /// The canonical text: "Net 30", "Net 15 EOM", "2/10 net 30", "Due on
    /// receipt", "Due 2026-11-15".
    text: String,
}

impl Terms {
    /// Terms for `kind`, with `discount` when given, canonical text
    /// composed. Refused when the days are past [`MAX_TERMS_DAYS`], the
    /// discount is outside 1..=9,999 basis points, or a discount is asked
    /// for terms that count no net days (due on receipt, EOM or custom).
    pub fn new(kind: TermsKind, discount: Option<EarlyDiscount>) -> Result<Self, LedgerError> {
        if let TermsKind::Net(n) | TermsKind::NetEom(n) = kind
            && n > MAX_TERMS_DAYS
        {
            return Err(LedgerError::TermsUnreadable(format!(
                "net days {n} is outside 0..={MAX_TERMS_DAYS}"
            )));
        }
        if let Some(d) = discount {
            if !(1..=9_999).contains(&d.percent_bp) {
                return Err(LedgerError::TermsUnreadable(format!(
                    "discount {} basis points is outside 1..=9999",
                    d.percent_bp
                )));
            }
            match kind {
                TermsKind::Net(n) if d.days < n => {}
                _ => return Err(LedgerError::TermsDiscountNeedsNetDays),
            }
        }
        // Custom terms are a date; without one there is nothing to print
        // or derive (Terms::custom is the way to give it).
        if kind == TermsKind::Custom {
            return Err(LedgerError::TermsUnreadable(
                "custom terms need their date: use Terms::custom".into(),
            ));
        }
        Ok(Terms {
            kind,
            discount,
            text: compose_terms_text(kind, discount),
        })
    }

    /// Custom terms due on `due`: the parties agreed a date rather than a
    /// count of days. Its text carries the date for people; the document
    /// stores the due date itself.
    pub fn custom(due: NaiveDate) -> Result<Self, LedgerError> {
        crate::calendar::check(due)?;
        Ok(Terms {
            kind: TermsKind::Custom,
            discount: None,
            text: format!("Due {due}"),
        })
    }

    /// How the due date counts.
    pub fn kind(&self) -> TermsKind {
        self.kind
    }

    /// The early-payment discount, when there is one.
    pub fn discount(&self) -> Option<EarlyDiscount> {
        self.discount
    }

    /// The canonical text ("Net 30", "2/10 net 30", "Due 2026-11-15").
    pub fn describe(&self) -> String {
        self.text.clone()
    }

    /// The date payment is due for a document dated `date`.
    ///
    /// For [`TermsKind::Custom`] the caller passes the agreed date itself
    /// (there is no count to derive it from); for every other kind `date`
    /// is the document's date. The result is never before `date` and
    /// always in the calendar: a due date past 9999-12-31 is refused
    /// ([`LedgerError::DateOverflow`]), never a panic.
    pub fn due_date(&self, date: NaiveDate) -> Result<NaiveDate, LedgerError> {
        crate::calendar::check(date)?;
        let due = match self.kind {
            TermsKind::DueOnReceipt | TermsKind::Custom => date,
            TermsKind::Net(n) => add_days(date, n)?,
            TermsKind::NetEom(n) => add_days(last_day_of_month(date)?, n)?,
        };
        validate_due_date(date, due)?;
        Ok(due)
    }

    /// The last date the early-payment discount is open for a document
    /// dated `date`: `date + discount.days`, or `None` when there is no
    /// discount or the date is outside the calendar.
    pub fn discount_by(&self, date: NaiveDate) -> Option<NaiveDate> {
        let d = self.discount?;
        let by = date.checked_add_days(Days::new(u64::from(d.days)))?;
        crate::calendar::check(by).ok()
    }

    /// Read the canonical spellings, case-insensitively: "Net 30", "Net 15
    /// EOM", "2/10 net 30", "Due on receipt", "Due 2026-11-15". Anything
    /// else is [`LedgerError::TermsUnreadable`].
    pub fn parse(text: &str) -> Result<Self, LedgerError> {
        let raw = text.trim();
        let lower = raw.to_lowercase();
        let unreadable = || LedgerError::TermsUnreadable(raw.to_string());
        if lower == "due on receipt" {
            return Terms::new(TermsKind::DueOnReceipt, None);
        }
        if let Some(rest) = lower.strip_prefix("due ") {
            let due =
                NaiveDate::parse_from_str(rest.trim(), "%Y-%m-%d").map_err(|_| unreadable())?;
            return Terms::custom(due).map_err(|_| unreadable());
        }
        // "2/10 net 30" (an optional decimal percentage), or "net 30" /
        // "net 15 eom".
        let (percent, days, net, eom) = match lower.split_once('/') {
            Some((pct, rest)) => {
                let (days, net_part) = rest.trim().split_once(' ').ok_or_else(unreadable)?;
                let after = net_part
                    .trim()
                    .strip_prefix("net ")
                    .ok_or_else(unreadable)?
                    .trim();
                let (net, eom) = match after.strip_suffix(" eom") {
                    Some(n) => (n, true),
                    None => (after, false),
                };
                (
                    Some(parse_percent(pct).ok_or_else(unreadable)?),
                    days,
                    net,
                    eom,
                )
            }
            None => {
                let rest = lower.strip_prefix("net ").ok_or_else(unreadable)?;
                let (net, eom) = match rest.trim().strip_suffix(" eom") {
                    Some(n) => (n, true),
                    None => (rest.trim(), false),
                };
                (None, "0", net, eom)
            }
        };
        let net: u16 = net.trim().parse().map_err(|_| unreadable())?;
        let discount = match (percent, days) {
            (Some(percent_bp), days_raw) => {
                let days: u16 = days_raw.parse().map_err(|_| unreadable())?;
                Some(EarlyDiscount { percent_bp, days })
            }
            (None, _) => None,
        };
        let kind = if eom {
            TermsKind::NetEom(net)
        } else {
            TermsKind::Net(net)
        };
        Terms::new(kind, discount).map_err(|_| unreadable())
    }
}

/// The canonical text of terms: `"2/10 net 30"` with a discount, else
/// `"Net 30"`, `"Net 15 EOM"` or `"Due on receipt"`.
fn compose_terms_text(kind: TermsKind, discount: Option<EarlyDiscount>) -> String {
    match (discount, kind) {
        (Some(EarlyDiscount { percent_bp, days }), TermsKind::Net(n)) => {
            format!("{}/{} net {n}", percent_text(percent_bp), days)
        }
        (_, TermsKind::Net(n)) => format!("Net {n}"),
        (_, TermsKind::NetEom(n)) => format!("Net {n} EOM"),
        (_, TermsKind::DueOnReceipt) => "Due on receipt".to_string(),
        (_, TermsKind::Custom) => "Due".to_string(),
    }
}

/// Basis points as the percentage trade terms print: 200 is "2", 250 is
/// "2.5", 205 is "2.05".
fn percent_text(percent_bp: u32) -> String {
    let whole = percent_bp / 100;
    let frac = percent_bp % 100;
    match (whole, frac) {
        (w, 0) => w.to_string(),
        (w, f) if f % 10 == 0 => format!("{w}.{}", f / 10),
        (w, f) => format!("{w}.{f:02}"),
    }
}

/// A percentage in trade terms as basis points: "2" is 200, "2.5" is 250,
/// "2.05" is 205. None when it isn't a percentage with at most two
/// decimal places.
fn parse_percent(s: &str) -> Option<u32> {
    let (whole, frac) = match s.trim().split_once('.') {
        Some((w, f)) => (w, f),
        None => (s.trim(), ""),
    };
    if frac.len() > 2 || !frac.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let whole: u32 = whole.parse().ok()?;
    let frac: u32 = if frac.is_empty() {
        0
    } else {
        format!("{frac:0<2}").parse().ok()?
    };
    whole.checked_mul(100)?.checked_add(frac)
}

/// `date` plus `n` days, or [`LedgerError::DateOverflow`] past the
/// calendar's end.
fn add_days(date: NaiveDate, n: u16) -> Result<NaiveDate, LedgerError> {
    date.checked_add_days(Days::new(u64::from(n)))
        .ok_or(LedgerError::DateOverflow(date))
}

/// The last day of `date`'s month, or [`LedgerError::DateOverflow`] when
/// there is no such date in the calendar.
fn last_day_of_month(date: NaiveDate) -> Result<NaiveDate, LedgerError> {
    let (year, month) = if date.month() == 12 {
        (
            date.year()
                .checked_add(1)
                .ok_or(LedgerError::DateOverflow(date))?,
            1,
        )
    } else {
        (date.year(), date.month() + 1)
    };
    NaiveDate::from_ymd_opt(year, month, 1)
        .and_then(|first| first.pred_opt())
        .ok_or(LedgerError::DateOverflow(date))
}

/// The parties on a document, as they were when it was issued (plan P4b
/// S4.1, A-13): a name, and the address and email that were on the
/// document. Never today's party record when a snapshot exists.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartySnapshot {
    /// The party's name at issue.
    pub name: String,
    /// Its address at issue, when the document printed one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    /// Its email at issue, when the document printed one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
}

/// How an invoice was sent (plan P4b S5; the type S4.1 adds so
/// [`Invoice::sends`] has one from the start).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SentVia {
    /// Email.
    Email,
    /// Paper mail.
    Mail,
    /// A customer or vendor portal.
    Portal,
    /// Handed over.
    Hand,
    /// Another channel, named.
    Other(String),
}

impl SentVia {
    /// Its key as the journal, the tools and the viewer name it: `email`,
    /// `mail`, `portal`, `hand`, or `other:NAME` for another channel,
    /// named (plan P4b S5).
    pub fn key(&self) -> String {
        match self {
            SentVia::Email => "email".to_string(),
            SentVia::Mail => "mail".to_string(),
            SentVia::Portal => "portal".to_string(),
            SentVia::Hand => "hand".to_string(),
            SentVia::Other(name) => format!("other:{name}"),
        }
    }

    /// [`Self::key`] the other way; `None` for a channel this version
    /// doesn't know (a newer version's `sent_via`).
    pub fn parse(key: &str) -> Option<SentVia> {
        match key {
            "email" => Some(SentVia::Email),
            "mail" => Some(SentVia::Mail),
            "portal" => Some(SentVia::Portal),
            "hand" => Some(SentVia::Hand),
            other => other
                .strip_prefix("other:")
                .filter(|name| !name.trim().is_empty())
                .map(|name| SentVia::Other(name.to_string())),
        }
    }
}

/// Whether a send was the original or a reminder (plan P4b S5).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SendKind {
    /// The document first went out.
    Original,
    /// It went out again.
    Reminder,
}

impl SendKind {
    /// Its key as the journal, the tools and the viewer name it.
    pub fn key(self) -> &'static str {
        match self {
            SendKind::Original => "original",
            SendKind::Reminder => "reminder",
        }
    }

    /// [`Self::key`] the other way; `None` for a kind this version
    /// doesn't know.
    pub fn parse(key: &str) -> Option<SendKind> {
        match key {
            "original" => Some(SendKind::Original),
            "reminder" => Some(SendKind::Reminder),
            _ => None,
        }
    }
}

/// One recorded send of an invoice (plan P4b S5): who sent it, how, to
/// whom, and the channel's own reference. A send changes no figure.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sent {
    /// The day it went out.
    pub on: NaiveDate,
    /// Who sent it: the owner, or a named delegate such as an agent (A-4).
    pub by: String,
    /// The channel.
    pub via: SentVia,
    /// The recipients.
    pub to: Vec<String>,
    /// Original or reminder.
    pub kind: SendKind,
    /// The channel's own id (a message id), when there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    /// A note about the send, when there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// The fields a new invoice carries beyond its figures (plan P4b S4.1).
/// Every one is optional: all `None` is the invoice every earlier version
/// wrote, posted and journaled exactly as before.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InvoiceFields {
    /// The number to issue it under ([`crate::Ledger::next_invoice_number`]).
    pub number: Option<String>,
    /// Its payment terms.
    pub terms: Option<Terms>,
    /// The customer as the document prints it.
    pub bill_to: Option<PartySnapshot>,
    /// Us as the document prints us.
    pub seller: Option<PartySnapshot>,
    /// The customer's own reference (their purchase order).
    pub customer_ref: Option<String>,
    /// A memo printed on the document.
    pub memo: Option<String>,
}

/// The fields a new bill carries beyond its figures (plan P4b S4.1).
/// Every one is optional: all `None` is the bill every earlier version
/// wrote, posted and journaled exactly as before.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BillFields {
    /// The vendor's own invoice number.
    pub vendor_ref: Option<String>,
    /// The day we received the bill, when it differs from its date.
    pub received_on: Option<NaiveDate>,
    /// Its payment terms.
    pub terms: Option<Terms>,
    /// A memo about the bill.
    pub memo: Option<String>,
    /// Why a vendor invoice number that already names a standing bill is
    /// entered again anyway. Journaled with the bill (S4.3); never part of
    /// the record itself.
    pub duplicate_reason: Option<String>,
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
    /// The account the line's revenue, expense or asset goes to (plan P4b
    /// S4.1, A-10): an invoice line's revenue or a liability holding no
    /// role; a bill line's expense or a non-cash asset holding no role.
    /// None is the kind's role account (revenue for an invoice, general
    /// expenses for a bill), which is also what every legacy line means.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<crate::AccountId>,
    /// How many units the line bills, when it gives a quantity and price
    /// rather than one amount. It and `unit_price` are both given or
    /// neither, and `quantity × unit_price` must equal `amount` exactly
    /// (plan P4b S4.1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quantity: Option<Decimal>,
    /// The price of one unit, when the line gives a quantity. See
    /// [`Self::quantity`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit_price: Option<Decimal>,
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
    /// The number it was issued under ("INV-0007"), from
    /// [`crate::Ledger::next_invoice_number`]; stored at issue, never
    /// recomputed, kept by a void (A-2, B-40). None on a legacy invoice:
    /// it is shown as "Invoice {id}" and never counted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number: Option<String>,
    /// Its payment terms, when it has them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terms: Option<Terms>,
    /// The customer as the document printed it, when it printed one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bill_to: Option<PartySnapshot>,
    /// Us as the document printed us, when it printed us.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seller: Option<PartySnapshot>,
    /// The customer's own reference (their purchase order), when given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub customer_ref: Option<String>,
    /// A memo printed on the document, when given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memo: Option<String>,
    /// Every recorded send, in order (plan P4b S5; empty until then).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sends: Vec<Sent>,
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

/// The account and project a document's lines group by (plan P4b S4.1):
/// None for either means the kind's role account (an invoice's revenue, a
/// bill's general expenses) and no project.
pub type DocumentPartKey = (Option<crate::AccountId>, Option<crate::ClassId>);

/// The revenue (an invoice's), expense or asset (a bill's) side of a
/// document's entry: one part per (account, project) the lines name, in
/// the order each first appears, the lines naming neither together as one
/// part (`(None, None)`), each the sum of its lines ([`document_total`]).
///
/// ```text
/// GOAL: a document's lines post to their accounts and count toward their
///       projects, and a document whose lines name neither posts exactly
///       as before.
/// SOURCE: docs/spec/rules.md B-37 item 3 (job costing: the revenue and
///         direct costs identified with one job) and item 8 (an
///         invoice's or bill's line names its project); B-39 and plan
///         P4b S4.1 A-10 (every bill line names its account).
/// INVARIANTS:
///   1. The parts sum to the document's total (conservation), and each
///      line is in exactly one part.
///   2. No two parts name the same account and project; no part is empty.
///   3. Lines naming neither make one part, so a document with no new
///      fields is one part: the one line every document posted before
///      S4.1.
///   4. NEVER panics: TooLargeToCompute past exact arithmetic.
/// ```
pub fn document_parts(
    lines: &[DocumentLine],
    what: &str,
) -> Result<Vec<(DocumentPartKey, Decimal)>, LedgerError> {
    let mut keys: Vec<DocumentPartKey> = Vec::new();
    for l in lines {
        let key = (l.account, l.class_id);
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    keys.into_iter()
        .map(|k| {
            let part: Vec<DocumentLine> = lines
                .iter()
                .filter(|l| (l.account, l.class_id) == k)
                .cloned()
                .collect();
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
    /// The vendor's own invoice number, when given: what a duplicate
    /// check reads (B-40; [`crate::Ledger::check_vendor_ref`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vendor_ref: Option<String>,
    /// The day we received the bill, when it differs from its date.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub received_on: Option<NaiveDate>,
    /// Its payment terms, when it has them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terms: Option<Terms>,
    /// A memo about the bill, when given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memo: Option<String>,
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
/// journal line's range, and a quantity and unit price that multiply to
/// the amount exactly (plan P4b S4.1). For new documents; replay restores
/// them as journaled (rules.md B-28).
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
        match (line.quantity, line.unit_price) {
            (None, None) => {}
            (Some(quantity), Some(unit_price)) => {
                if quantity <= Decimal::ZERO || unit_price <= Decimal::ZERO {
                    return Err(LedgerError::LineArithmetic(format!(
                        "line '{}': quantity {quantity} and unit price {unit_price} must both be positive",
                        line.description
                    )));
                }
                let product =
                    crate::kernel::line_amount(quantity, unit_price).ok_or_else(|| {
                        LedgerError::LineArithmetic(format!(
                            "line '{}': {quantity} x {unit_price} is too large to compute exactly",
                            line.description
                        ))
                    })?;
                if product != line.amount {
                    return Err(LedgerError::LineArithmetic(format!(
                        "line '{}': {quantity} x {unit_price} = {product}, not the line's amount {}; the engine never rounds a quantity times a price",
                        line.description, line.amount
                    )));
                }
            }
            (quantity, unit_price) => {
                return Err(LedgerError::LineArithmetic(format!(
                    "line '{}': a quantity and a unit price are both given or neither (quantity {quantity:?}, unit price {unit_price:?})",
                    line.description
                )));
            }
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
                    account: None,
                    quantity: None,
                    unit_price: None,
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
                    account: None,
                    quantity: None,
                    unit_price: None,
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
                    account: None,
                    quantity: None,
                    unit_price: None,
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
                account: None,
                quantity: None,
                unit_price: None,
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
                account: None,
                quantity: None,
                unit_price: None,
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
                account: None,
                quantity: None,
                unit_price: None,
            }],
        );
        assert!(matches!(err, Err(LedgerError::ClosedPeriod(_, _))));
    }
}
