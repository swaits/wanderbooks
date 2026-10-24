//! An invoice as PDF (plan P4b.11): what the company billed a customer,
//! typeset by `templates/invoice.typ` with the bundled fonts of
//! [`crate::pdf`].
//!
//! ```text
//! GOAL: an invoice the customer can pay from, and the books can keep as
//!       the copy that was sent.
//! SOURCE: plan/served.md P4b.11; IRS Pub. 583 (invoices are supporting
//!         documents); CONVENTION (what a US small-business invoice
//!         shows: seller, bill-to, number, dates, lines, total, balance
//!         due).
//! INVARIANTS:
//!   1. Deterministic: the same document renders to the same bytes (no
//!      creation time; fonts bundled; typst pinned).
//!   2. Text is data: every string reaches Typst as JSON
//!      (`sys.inputs.data`) and prints literally, Typst syntax included.
//!   3. Money is formatted here ([`format_amount`], with "$"); the template
//!      does no arithmetic, so the printed total is the total given.
//!   4. Refused: no lines.
//! SCENARIOS:
//!   - An open invoice: total, nothing settled, balance due = total.
//!   - Part paid: a "Payments received" line, the balance due the rest.
//!   - Paid in full: balance due $0.00 and a PAID stamp with the day; void:
//!     a VOID stamp; past due: PAST DUE; a preview: DRAFT (gray; the rest
//!     are red, as a rubber stamp).
//! ```

use crate::pdf::PdfError;
use crate::report::format_amount;
use rust_decimal::Decimal;
use std::sync::OnceLock;
use typst::foundations::{Dict, IntoValue};
use typst_as_lib::{TypstEngine, TypstTemplateMainFile};
use typst_layout::PagedDocument;

/// A party on the invoice: its name and the lines under it (an address).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InvoiceParty {
    /// Its name.
    pub name: String,
    /// The lines under it, in order.
    pub lines: Vec<String>,
}

/// One line of the invoice.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvoiceItem {
    /// What it is for.
    pub description: String,
    /// Its amount.
    pub amount: Decimal,
}

/// A stamp across the invoice, as a rubber stamp would mark it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvoiceStamp {
    /// Paid in full: the invoice is now the customer's receipt.
    Paid,
    /// Some paid, a balance still due.
    PartiallyPaid,
    /// Past its due date with a balance still due.
    PastDue,
    /// Voided: issued in error, cancelled.
    Void,
    /// Not issued yet: a preview.
    Draft,
}

impl InvoiceStamp {
    /// What the stamp says.
    pub fn text(self) -> &'static str {
        match self {
            InvoiceStamp::Paid => "PAID",
            InvoiceStamp::PartiallyPaid => "PARTIALLY PAID",
            InvoiceStamp::PastDue => "PAST DUE",
            InvoiceStamp::Void => "VOID",
            InvoiceStamp::Draft => "DRAFT",
        }
    }
}

/// Everything an invoice prints.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvoiceDoc {
    /// Its number, as printed ("Invoice 7"; "INV-0007" once numbered).
    pub number: String,
    /// The issue date, as printed.
    pub issue_date: String,
    /// The due date, as printed.
    pub due_date: String,
    /// The payment terms, as printed ("Net 30"), when known.
    pub terms: Option<String>,
    /// The company.
    pub seller: InvoiceParty,
    /// The customer.
    pub bill_to: InvoiceParty,
    /// Its lines.
    pub items: Vec<InvoiceItem>,
    /// The sum of its lines.
    pub total: Decimal,
    /// What was settled against it, each with its label (a payment's
    /// date, method and reference: "Payment Feb 10, 2026 · ACH · ref
    /// tr_8812"), positive.
    pub settled: Vec<(String, Decimal)>,
    /// What is still due.
    pub balance: Decimal,
    /// A stamp, when there is one.
    pub stamp: Option<InvoiceStamp>,
    /// A line under the stamp's word: for PAID, the day it was paid in
    /// full.
    pub stamp_line: Option<String>,
    /// A note under it (for a re-created copy, saying so).
    pub note: Option<String>,
}

static TEMPLATE: &str = include_str!("../templates/invoice.typ");

fn engine() -> &'static TypstEngine<TypstTemplateMainFile> {
    static ENGINE: OnceLock<TypstEngine<TypstTemplateMainFile>> = OnceLock::new();
    ENGINE.get_or_init(|| {
        TypstEngine::builder()
            .main_file(TEMPLATE)
            .fonts(crate::fonts::typst_fonts())
            .build()
    })
}

/// An amount as printed: "$1,234.56".
fn dollars(d: Decimal) -> String {
    let s = format_amount(d);
    match s.strip_prefix('(').and_then(|s| s.strip_suffix(')')) {
        Some(inner) => format!("-${inner}"),
        None => format!("${s}"),
    }
}

/// The template's input.
pub fn template_data(doc: &InvoiceDoc) -> Result<serde_json::Value, PdfError> {
    if doc.items.is_empty() {
        return Err(PdfError::Compile(
            "an invoice needs at least one line".into(),
        ));
    }
    let party = |p: &InvoiceParty| serde_json::json!({"name": p.name, "lines": p.lines});
    Ok(serde_json::json!({
        "title": "INVOICE",
        "number": doc.number,
        "issue_date": doc.issue_date,
        "due_date": doc.due_date,
        "terms": doc.terms,
        "seller": party(&doc.seller),
        "bill_to": party(&doc.bill_to),
        "lines": doc.items.iter().map(|i| serde_json::json!({
            "description": i.description,
            "amount": dollars(i.amount),
        })).collect::<Vec<_>>(),
        "total": dollars(doc.total),
        "settled": doc.settled.iter().map(|(label, a)| serde_json::json!({
            "label": label,
            "amount": dollars(-*a),
        })).collect::<Vec<_>>(),
        "balance_label": "Balance due",
        "balance": dollars(doc.balance),
        "stamp": doc.stamp.map(InvoiceStamp::text),
        "stamp_draft": doc.stamp == Some(InvoiceStamp::Draft),
        "stamp_line": doc.stamp_line,
        "note": doc.note,
    }))
}

/// Typeset an invoice into pages, before PDF encoding (for inspection).
pub fn typeset_invoice(doc: &InvoiceDoc) -> Result<PagedDocument, PdfError> {
    let mut inputs = Dict::new();
    inputs.insert("data".into(), template_data(doc)?.to_string().into_value());
    engine()
        .compile_with_input(inputs)
        .output
        .map_err(|e| PdfError::Compile(format!("{e:?}")))
}

/// An invoice as PDF bytes. See the module documentation for the spec.
pub fn invoice_pdf(doc: &InvoiceDoc) -> Result<Vec<u8>, PdfError> {
    let pages = typeset_invoice(doc)?;
    typst_pdf::pdf(&pages, &typst_pdf::PdfOptions::default())
        .map_err(|e| PdfError::Save(format!("{e:?}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc() -> InvoiceDoc {
        InvoiceDoc {
            number: "Invoice 7".into(),
            issue_date: "Feb 1, 2026".into(),
            due_date: "Mar 3, 2026".into(),
            terms: Some("Net 30".into()),
            seller: InvoiceParty {
                name: "Fernhill Labs LLC".into(),
                lines: vec!["12 Orchard Lane".into(), "Springfield, UT 84000".into()],
            },
            bill_to: InvoiceParty {
                name: "Harbor Client".into(),
                lines: vec!["1 Pier Way".into()],
            },
            items: vec![
                InvoiceItem {
                    description: "Consulting, January".into(),
                    amount: Decimal::new(120_000, 2),
                },
                InvoiceItem {
                    description: "#panic(\"no\") *not bold* $x$".into(),
                    amount: Decimal::new(3_450, 2),
                },
            ],
            total: Decimal::new(123_450, 2),
            settled: vec![("Payments received".into(), Decimal::new(20_000, 2))],
            balance: Decimal::new(103_450, 2),
            stamp: None,
            stamp_line: None,
            note: None,
        }
    }

    #[test]
    fn an_invoice_renders_deterministically() {
        let a = invoice_pdf(&doc()).unwrap();
        assert!(a.starts_with(b"%PDF"));
        // SHOT_DIR set: the invoice, to look at.
        if let Ok(dir) = std::env::var("SHOT_DIR") {
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(std::path::Path::new(&dir).join("invoice.pdf"), &a).unwrap();
        }
        assert_eq!(a, invoice_pdf(&doc()).unwrap());
    }

    #[test]
    fn text_is_data_and_money_is_formatted_here() {
        let data = template_data(&doc()).unwrap();
        assert_eq!(data["total"], "$1,234.50");
        assert_eq!(data["balance"], "$1,034.50");
        assert_eq!(data["settled"][0]["amount"], "-$200.00");
        // The Typst-looking description renders (it would fail to compile
        // as markup: #panic stops the document).
        assert!(typeset_invoice(&doc()).is_ok());
    }

    #[test]
    fn stamps_render_and_no_lines_is_refused() {
        let mut d = doc();
        for st in [
            InvoiceStamp::Paid,
            InvoiceStamp::PartiallyPaid,
            InvoiceStamp::PastDue,
            InvoiceStamp::Draft,
        ] {
            d.stamp = Some(st);
            d.stamp_line = Some("Feb 20, 2026".into());
            assert!(invoice_pdf(&d).is_ok(), "{st:?}");
        }
        d.stamp = Some(InvoiceStamp::Void);
        d.note = Some("Re-created from the books; not the copy sent.".into());
        assert!(invoice_pdf(&d).is_ok());
        d.items.clear();
        assert!(invoice_pdf(&d).is_err());
    }
}
