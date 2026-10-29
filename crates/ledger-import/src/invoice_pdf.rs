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
//!   5. The issued copy (`InvoiceVersion::Issued`) is the page the
//!      customer got: no "re-created" note and no settled rows. The
//!      current copy (`InvoiceVersion::Current { as_of }`) says on which
//!      day it was re-created, and that day is the `as_of` it was given,
//!      never the wall clock, so the bytes stay deterministic.
//!   6. A line given as a quantity and a unit price prints "3 × $33.33"
//!      under its description; the amount beside it is the line's own.
//!      Nothing internal prints: no account code, project or id.
//! SCENARIOS:
//!   - An open invoice: total, nothing settled, balance due = total.
//!   - Part paid: a "Payments received" line, the balance due the rest.
//!   - Paid in full: balance due $0.00 and a PAID stamp with the day; void:
//!     a VOID stamp; past due: PAST DUE; a preview: DRAFT (gray; the rest
//!     are red, as a rubber stamp).
//! ```

use crate::pdf::PdfError;
use crate::report::amount_parts;
use chrono::NaiveDate;
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
    /// How many units, when the line was given as a quantity and a unit
    /// price (both or neither).
    pub quantity: Option<Decimal>,
    /// The price of one unit, with [`Self::quantity`].
    pub unit_price: Option<Decimal>,
}

/// One way to pay, as the page prints it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PaymentMethodDoc {
    /// Its name ("Bank transfer", "Card").
    pub label: String,
    /// The lines under it (bank details, a link).
    pub lines: Vec<String>,
    /// A QR code for it, as an SVG document (`labels::qr_svg`), when it
    /// has one.
    pub qr_svg: Option<String>,
}

/// Which copy of the invoice this is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InvoiceVersion {
    /// The copy issued to the customer (or its draft): no "re-created"
    /// note, nothing settled on it.
    #[default]
    Issued,
    /// The invoice re-created from the books as of a day; the page says
    /// so, with that day.
    Current {
        /// The day the figures are as of, printed in the note.
        as_of: NaiveDate,
    },
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
#[derive(Clone, Debug, Default, PartialEq, Eq)]
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
    /// The seller's email, when it has one.
    pub seller_email: Option<String>,
    /// The seller's website, when it has one.
    pub seller_website: Option<String>,
    /// The customer.
    pub bill_to: InvoiceParty,
    /// Where goods go, when the customer said so: printed as a Ship-to
    /// block.
    pub ship_to: Vec<String>,
    /// The customer's own reference (their purchase order), when given.
    pub customer_ref: Option<String>,
    /// A memo printed under the lines, when given.
    pub memo: Option<String>,
    /// The ways to pay.
    pub payment_methods: Vec<PaymentMethodDoc>,
    /// Text at the foot of the page (`invoicing.footer`).
    pub footer: Option<String>,
    /// Which copy this is.
    pub version: InvoiceVersion,
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
    /// A note under it, besides the re-created note a current copy always
    /// carries.
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

/// An amount as printed: "$1,234.56". Every amount on the page comes
/// through [`amount_parts`].
fn dollars(d: Decimal) -> String {
    let p = amount_parts(d);
    if p.negative {
        format!("-${}", p.digits)
    } else {
        format!("${}", p.digits)
    }
}

/// A unit price as printed: to the cent when it is one, else exactly (a
/// price of $0.125 must not print as $0.13 beside a total it multiplies
/// to).
fn unit_price(d: Decimal) -> String {
    if d.normalize().scale() <= 2 {
        dollars(d)
    } else {
        format!("${}", d.normalize())
    }
}

/// "3 × $33.33" for a line given as a quantity and a price.
fn quantity_detail(i: &InvoiceItem) -> Option<String> {
    match (i.quantity, i.unit_price) {
        (Some(q), Some(p)) => Some(format!("{} × {}", q.normalize(), unit_price(p))),
        _ => None,
    }
}

/// The note under the totals: the re-created line of a current copy, then
/// whatever note the document carries.
fn note_text(doc: &InvoiceDoc) -> Option<String> {
    let recreated = match doc.version {
        InvoiceVersion::Current { as_of } => Some(format!(
            "Re-created on {} from the books; not a copy of the one sent.",
            as_of.format("%b %-d, %Y")
        )),
        InvoiceVersion::Issued => None,
    };
    match (recreated, &doc.note) {
        (Some(r), Some(n)) => Some(format!("{r} {n}")),
        (Some(r), None) => Some(r),
        (None, n) => n.clone(),
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
        "seller_email": doc.seller_email,
        "seller_website": doc.seller_website,
        "bill_to": party(&doc.bill_to),
        "ship_to": doc.ship_to,
        "customer_ref": doc.customer_ref,
        "memo": doc.memo,
        "lines": doc.items.iter().map(|i| serde_json::json!({
            "description": i.description,
            "detail": quantity_detail(i),
            "amount": dollars(i.amount),
        })).collect::<Vec<_>>(),
        "payment_methods": doc.payment_methods.iter().map(|m| serde_json::json!({
            "label": m.label,
            "lines": m.lines,
            "qr": m.qr_svg,
        })).collect::<Vec<_>>(),
        "footer": doc.footer,
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
        "note": note_text(doc),
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
                    quantity: None,
                    unit_price: None,
                },
                InvoiceItem {
                    description: "#panic(\"no\") *not bold* $x$".into(),
                    amount: Decimal::new(3_450, 2),
                    quantity: Some(Decimal::new(3, 0)),
                    unit_price: Some(Decimal::new(1_150, 2)),
                },
            ],
            total: Decimal::new(123_450, 2),
            settled: vec![("Payments received".into(), Decimal::new(20_000, 2))],
            balance: Decimal::new(103_450, 2),
            stamp: None,
            stamp_line: None,
            note: None,
            ..InvoiceDoc::default()
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

    /// INVARIANT 2 for every string the page takes: Typst syntax in an
    /// item, a party, a memo, a payment method or a footer prints as
    /// written and breaks nothing.
    #[test]
    fn labels_are_data() {
        let mut d = doc();
        d.items[0].description = "#import \"x\": *".into();
        d.memo = Some("#panic(\"memo\") $x$ ]".into());
        d.footer = Some("#panic(\"footer\")".into());
        d.customer_ref = Some("PO #{1}".into());
        d.payment_methods = vec![PaymentMethodDoc {
            label: "#panic(\"label\")".into(),
            lines: vec!["#import \"y\"".into()],
            qr_svg: None,
        }];
        let data = template_data(&d).unwrap();
        assert_eq!(data["lines"][0]["description"], "#import \"x\": *");
        assert_eq!(data["memo"], "#panic(\"memo\") $x$ ]");
        assert_eq!(data["payment_methods"][0]["label"], "#panic(\"label\")");
        assert!(invoice_pdf(&d).is_ok());
    }

    /// A line given as a quantity and a price prints both; the amount is
    /// the line's own, never recomputed here.
    #[test]
    fn a_quantity_prints_beside_its_price() {
        let data = template_data(&doc()).unwrap();
        assert_eq!(data["lines"][1]["detail"], "3 × $11.50");
        assert_eq!(data["lines"][1]["amount"], "$34.50");
        assert!(data["lines"][0]["detail"].is_null());
        let mut d = doc();
        d.items[1].quantity = Some(Decimal::new(8, 0));
        d.items[1].unit_price = Some(Decimal::new(125, 3));
        d.items[1].amount = Decimal::new(100, 2);
        let data = template_data(&d).unwrap();
        assert_eq!(data["lines"][1]["detail"], "8 × $0.125");
    }

    /// The current copy says on which day it was re-created, from the
    /// `as_of` it was given; the issued copy says nothing of the kind.
    #[test]
    fn only_the_current_copy_says_it_was_re_created() {
        let mut d = doc();
        assert!(template_data(&d).unwrap()["note"].is_null());
        d.version = InvoiceVersion::Current {
            as_of: NaiveDate::from_ymd_opt(2026, 3, 1).unwrap(),
        };
        let note = template_data(&d).unwrap()["note"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(note.starts_with("Re-created on Mar 1, 2026"), "{note}");
        d.note = Some("Voided.".into());
        let note = template_data(&d).unwrap()["note"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(note.ends_with(" Voided."), "{note}");
    }

    #[test]
    fn two_hundred_lines_paginate() {
        let mut d = doc();
        d.items = (1..=200)
            .map(|n| InvoiceItem {
                description: format!("Line {n}"),
                amount: Decimal::new(100, 2),
                quantity: None,
                unit_price: None,
            })
            .collect();
        d.total = Decimal::new(20_000, 2);
        d.settled.clear();
        d.balance = d.total;
        let pages = typeset_invoice(&d).unwrap();
        assert!(pages.pages().len() > 1, "{} pages", pages.pages().len());
        assert!(invoice_pdf(&d).unwrap().starts_with(b"%PDF"));
    }
}
