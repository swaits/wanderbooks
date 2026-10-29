//! `books.documents`' group: the A/R and A/P documents (invoices and
//! bills), their voids, the read tools (`books.documents`,
//! `books.document`) and the control check. Split whole out of
//! `server.rs` (plan/served.md 8.2 U0a); the payments and the agings live
//! in `tools/settlements.rs` (P4b S6).

use super::*;
use ledger_core::Terms;
use ledger_core::arap::{self, PartySnapshot};

/// A document's id.
fn doc_id(doc: ledger_core::DocRef) -> u64 {
    match doc {
        ledger_core::DocRef::Invoice(id) | ledger_core::DocRef::Bill(id) => id,
    }
}

/// A document as books.documents and books.document name it
/// ("invoice:7", "bill:12").
fn doc_ref_text(doc: ledger_core::DocRef) -> String {
    match doc {
        ledger_core::DocRef::Invoice(id) => format!("invoice:{id}"),
        ledger_core::DocRef::Bill(id) => format!("bill:{id}"),
    }
}

/// A tagged line's movement on the wire.
fn movement_key(m: ledger_core::Movement) -> &'static str {
    match m {
        ledger_core::Movement::Origin => "origin",
        ledger_core::Movement::Payment => "payment",
        ledger_core::Movement::Discount => "discount",
        ledger_core::Movement::WriteOff => "write_off",
        ledger_core::Movement::Adopted => "adopted",
        ledger_core::Movement::RoleMove => "role_move",
    }
}

/// An invoice's or bill's lines as the tool takes them: each amount
/// parsed, each project and account resolved ([`resolve_class`],
/// [`resolve_account`]), and quantity and unit price together, before
/// anything is written (plan P4b S4.4).
fn document_lines(
    store: &Store,
    lines: Vec<DocumentLineParam>,
) -> Result<Vec<DocumentLine>, String> {
    lines
        .into_iter()
        .map(|l| {
            let quantity = l.quantity.as_deref().map(parse_amount).transpose()?;
            let unit_price = l.unit_price.as_deref().map(parse_amount).transpose()?;
            if quantity.is_some() != unit_price.is_some() {
                return Err(format!(
                    "line {:?}: quantity and unit_price go together, both or neither",
                    l.description
                ));
            }
            Ok(DocumentLine {
                description: l.description,
                amount: parse_amount(&l.amount)?,
                class_id: l
                    .class
                    .as_deref()
                    .map(|c| resolve_class(store, c))
                    .transpose()?,
                account: l
                    .account
                    .as_deref()
                    .map(|a| resolve_account(store, a))
                    .transpose()?,
                quantity,
                unit_price,
            })
        })
        .collect()
}

/// A document's notes, each project once, in the order first named: its
/// late-tag note ([`projects::late_tag_note`]) for a finished or abandoned
/// project, and on an invoice ([`projects::on_spec_invoice_note`]) one for
/// an on-spec project (rules.md B-37 items 4, 5 and 8).
fn document_notes(
    ledger: &ledger_core::Ledger,
    lines: &[DocumentLine],
    invoice: bool,
) -> Vec<String> {
    let mut named: Vec<u32> = Vec::new();
    for c in lines.iter().filter_map(|l| l.class_id) {
        if !named.contains(&c) {
            named.push(c);
        }
    }
    let mut notes = Vec::new();
    for c in named {
        notes.extend(projects::late_tag_note(ledger, c));
        if invoice {
            notes.extend(projects::on_spec_invoice_note(ledger, c));
        }
    }
    notes
}

/// Resolve a customer the way [`resolve_vendor`] resolves a vendor: a
/// customer id, one of its party's ids, or its name (ignoring case and
/// surrounding space); a number that is one customer's id and another's
/// name is refused, never guessed (plan P4b S4.4).
fn resolve_customer(store: &Store, customer: &str) -> Result<u64, String> {
    let ledger = store.ledger();
    let by_id = customer.trim().parse::<u64>().ok().and_then(|id| {
        ledger.get_customer(id).map(|_| id).or_else(|| {
            ledger
                .get_party(id)
                .and_then(|p| p.customer)
                .filter(|c| ledger.get_customer(*c).is_some())
        })
    });
    let matches: Vec<u64> = ledger
        .list_customers()
        .into_iter()
        .filter(|c| c.name.eq_ignore_ascii_case(customer.trim()))
        .map(|c| c.id)
        .collect();
    if let Some(id) = by_id {
        return match matches.as_slice() {
            [] => Ok(id),
            [m] if *m == id => Ok(id),
            _ => Err(format!(
                "customer {customer:?} is ambiguous: it is customer {id}'s id and another customer's name; pass the id"
            )),
        };
    }
    match matches.as_slice() {
        [id] => Ok(*id),
        [] => Err(format!(
            "unknown customer {customer:?}: add it with books.add_customer"
        )),
        _ => Err(format!(
            "several customers are named {customer:?}: pass the id"
        )),
    }
}

/// Resolve a vendor the way [`resolve_customer`] resolves a customer: a
/// vendor id, one of its party's ids, or its name (plan P4b S4.4).
fn resolve_vendor_arg(store: &Store, vendor: &str) -> Result<u64, String> {
    if let Ok(id) = vendor.trim().parse::<u64>() {
        let ledger = store.ledger();
        if ledger.get_vendor(id).is_none()
            && let Some(v) = ledger.get_party(id).and_then(|p| p.vendor)
            && ledger.get_vendor(v).is_some()
        {
            return Ok(v);
        }
    }
    resolve_vendor(store, vendor)
}

/// Whether these books declared the journal feature `documents_v2`: the
/// bills and invoices carrying a number, terms, snapshots, references, a
/// line's account or a quantity need it (rules.md B-27, D-52), and the
/// bill rules A-10, A-13 and the file apply only where it is declared.
fn documents_v2(store: &Store) -> bool {
    store
        .journal_format()
        .has_feature(ledger_store::format::DOCUMENTS_V2)
}

/// The customer's or vendor's name to give a snapshot: its display name,
/// else its record's name.
fn party_label(store: &Store, customer: Option<u64>, vendor: Option<u64>) -> String {
    let ledger = store.ledger();
    if let Some(c) = customer {
        if let Some(p) = ledger
            .party_of_customer(c)
            .and_then(|p| ledger.get_party(p))
        {
            return p.display_name.clone().unwrap_or_else(|| p.name.clone());
        }
        return ledger
            .get_customer(c)
            .map(|c| c.name.clone())
            .unwrap_or_default();
    }
    if let Some(v) = vendor {
        if let Some(p) = ledger.party_of_vendor(v).and_then(|p| ledger.get_party(p)) {
            return p.display_name.clone().unwrap_or_else(|| p.name.clone());
        }
        return ledger
            .get_vendor(v)
            .map(|v| v.name.clone())
            .unwrap_or_default();
    }
    String::new()
}

/// Us as the document prints us (A-13, 8.3.4): the company's name, address
/// and email from its settings.
fn seller_snapshot(settings: &Settings) -> PartySnapshot {
    let profile = |key: &str| {
        settings
            .company
            .profile
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
            .filter(|v| !v.trim().is_empty())
    };
    PartySnapshot {
        name: settings.company.name.clone(),
        address: profile("address"),
        email: profile("email"),
        // The contact block is the party's (D-55): the company has none
        // yet (its own details are settings the page prints separately).
        phone: None,
        contact: None,
        billing_address: None,
        shipping_address: None,
        billing_email: None,
    }
}

/// The customer's snapshot at issue: what the caller gives, each part
/// omitted the party record's (A-13, D-55). The billing address and
/// billing email, when there are any, are what the page prints (D-55);
/// [`PartySnapshot::from_party`] is where that rule lives.
fn customer_snapshot(
    store: &Store,
    customer: u64,
    given: Option<BillToParam>,
) -> Result<PartySnapshot, String> {
    let ledger = store.ledger();
    let party = ledger
        .party_of_customer(customer)
        .and_then(|p| ledger.get_party(p));
    let mut snapshot = match party {
        Some(p) => PartySnapshot::from_party(p),
        None => PartySnapshot {
            name: party_label(store, Some(customer), None),
            address: None,
            email: None,
            phone: None,
            contact: None,
            billing_address: None,
            shipping_address: None,
            billing_email: None,
        },
    };
    if let Some(g) = given {
        let text = |v: String| {
            let t = v.trim().to_string();
            Some(t).filter(|t| !t.is_empty())
        };
        if let Some(v) = g.name {
            snapshot.name = v.trim().to_string();
        }
        if let Some(v) = g.address {
            snapshot.address = text(v);
        }
        if let Some(v) = g.email {
            snapshot.email = text(v);
        }
        if let Some(v) = g.phone {
            snapshot.phone = text(v);
        }
        if let Some(v) = g.contact {
            snapshot.contact = text(v);
        }
        if let Some(v) = g.shipping_address {
            snapshot.shipping_address = text(v);
        }
        if let Some(v) = g.billing_address {
            snapshot.billing_address = text(v);
            snapshot.address = snapshot.billing_address.clone();
        }
        if let Some(v) = g.billing_email {
            snapshot.billing_email = text(v);
            snapshot.email = snapshot.billing_email.clone();
        }
    }
    ledger_core::parties::check_text("the bill-to name", &snapshot.name)?;
    if let Some(a) = &snapshot.address {
        ledger_core::parties::check_text("the bill-to address", a)?;
    }
    for (field, value) in [
        ("the bill-to contact", &snapshot.contact),
        ("the bill-to phone", &snapshot.phone),
        ("the billing address", &snapshot.billing_address),
        ("the shipping address", &snapshot.shipping_address),
    ] {
        if let Some(v) = value {
            ledger_core::parties::check_text(field, v)?;
        }
    }
    for (field, value) in [
        ("the bill-to email", &snapshot.email),
        ("the billing email", &snapshot.billing_email),
    ] {
        if let Some(v) = value {
            ledger_core::parties::check_email(field, v)?;
        }
    }
    Ok(snapshot)
}

/// The terms an invoice takes when neither it nor the customer gives any:
/// `invoicing.default_terms`, else its default "Net 30" (plan P4b S4.2,
/// S4.4). None in books that haven't declared `documents_v2`: their
/// invoices keep the legacy shape, and a due date is given instead.
fn books_default_terms(store: &Store) -> Result<Option<Terms>, String> {
    if !documents_v2(store) {
        return Ok(None);
    }
    let text = store
        .ledger()
        .settings()
        .get("invoicing.default_terms")
        .and_then(|r| r.value.as_str().map(str::to_string))
        .unwrap_or_else(|| "Net 30".to_string());
    Ok(Some(Terms::parse(&text).map_err(|e| e.to_string())?))
}

/// A party's default terms on one side (A-13).
fn party_default_terms(
    store: &Store,
    party: Option<ledger_core::PartyId>,
    customer: bool,
) -> Option<Terms> {
    let p = store.ledger().get_party(party?)?;
    if customer {
        p.default_terms_as_customer.clone()
    } else {
        p.default_terms_as_vendor.clone()
    }
}

/// The due date a document takes and the terms to store with it: the due
/// date given; else the terms given; else the party's default terms; else
/// the books' own default (invoice only); else refused, naming the ways
/// to give one (plan P4b S4.4 item 14).
///
/// Terms are stored only when they came from the caller or a default: a
/// document given only a due date keeps the legacy shape (`terms: None`)
/// and so is never refused for a new field on books that haven't declared
/// `documents_v2`.
fn due_and_terms(
    date: NaiveDate,
    due_date: Option<&str>,
    terms: Option<&str>,
    defaults: Vec<Option<Terms>>,
    what: &str,
    ways: &str,
) -> Result<(NaiveDate, Option<Terms>), String> {
    let given = terms
        .map(Terms::parse)
        .transpose()
        .map_err(|e| e.to_string())?;
    if let Some(d) = due_date {
        return Ok((parse_date(d)?, given));
    }
    if let Some(t) = given {
        let due = t.due_date(date).map_err(|e| e.to_string())?;
        return Ok((due, Some(t)));
    }
    if let Some(t) = defaults.into_iter().flatten().next() {
        let due = t.due_date(date).map_err(|e| e.to_string())?;
        return Ok((due, Some(t)));
    }
    Err(format!("give {what} a due date: {ways}"))
}

/// A posting line as the preview shows it, with its project's name.
fn posting_out(ledger: &Ledger, lines: &[ledger_store::PreviewPosting]) -> Vec<DocumentPostingOut> {
    lines
        .iter()
        .map(|l| DocumentPostingOut {
            account: l.account.clone(),
            name: l.name.clone(),
            side: match l.side {
                Side::Debit => "debit".to_string(),
                Side::Credit => "credit".to_string(),
            },
            amount: money(l.amount),
            class: l
                .class_id
                .and_then(|c| ledger.get_class(c))
                .map(|c| c.name.clone()),
        })
        .collect()
}

/// A word of `text` as a URL carries it: unreserved ASCII as it is, every
/// other byte `%XX` (UTF-8).
fn url_encode(text: &str) -> String {
    let mut out = String::new();
    for b in text.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// The `mailto:` convenience link for a human to send an invoice from
/// their own mail program (A-4; the books never send): "Invoice INV-0007
/// from {company}" as the subject, a short body naming the file to
/// attach, the body URL-encoded and cut to 1,800 bytes at the last whole
/// word (plan P4b S4.4 item 14).
fn mailto_link(settings: &Settings, invoice: &arap::Invoice) -> String {
    let number = invoice
        .number
        .clone()
        .unwrap_or_else(|| format!("Invoice {}", invoice.id));
    let to = invoice
        .bill_to
        .as_ref()
        .and_then(|b| b.email.clone())
        .unwrap_or_default();
    let subject = url_encode(&format!("Invoice {number} from {}", settings.company.name));
    let body = format!(
        "Hello,\n\nInvoice {number} for {} is due {}. It is attached as {number}.pdf.\n\nThank you,\n{}",
        money(invoice.total().unwrap_or_default()),
        invoice.due_date,
        settings.company.name
    );
    let mut encoded = url_encode(&body);
    const LIMIT: usize = 1_800;
    if encoded.len() > LIMIT {
        // The last whole word that fits: everything before a "%20".
        match encoded[..LIMIT].rfind("%20") {
            Some(at) => encoded.truncate(at),
            None => encoded.truncate(LIMIT),
        }
    }
    format!("mailto:{to}?subject={subject}&body={encoded}")
}

#[tool_router(router = documents_router, vis = "pub(super)")]
impl BooksServer {
    #[tool(
        name = "books.create_invoice",
        title = "Create and post an invoice",
        description = "Create a sales invoice: without confirmed it is a preview (the posting, the number and due date it will take, its DRAFT PDF handed over) and nothing is written; with confirmed=true it posts Dr Accounts Receivable / Cr each line's account, numbers it, files the issued PDF with it, and returns the attachment and its sha256. Lines are {description, amount, quantity?, unit_price?, account?, class?}. Due date: from terms (\"Net 30\", \"2/10 net 30\") or given; else the customer's default, else invoicing.default_terms. customer is an id or a name. mailto is a link for a human to send it; the books never send (A-4).",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn create_invoice(
        &self,
        params: Parameters<CreateInvoiceParams>,
    ) -> Result<Json<InvoiceOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let customer = p.customer.trim().to_string();
        let customer_id = resolve_customer(&store, &customer)?;
        let date = parse_date(&p.date)?;
        let lines = document_lines(&store, p.lines)?;
        let v2 = documents_v2(&store);
        let party = store.ledger().party_of_customer(customer_id);
        let (due_date, terms) = due_and_terms(
            date,
            p.due_date.as_deref(),
            p.terms.as_deref(),
            vec![
                party_default_terms(&store, party, true),
                books_default_terms(&store)?,
            ],
            "the invoice",
            "due_date, or terms, or the customer's default terms, or invoicing.default_terms",
        )?;
        if let Some(r) = &p.customer_ref {
            ledger_core::parties::check_text("the customer reference", r)?;
        }
        if let Some(m) = &p.memo {
            ledger_core::parties::check_text("the memo", m)?;
        }
        let settings = self.settings();
        let fields = arap::InvoiceFields {
            number: None,
            terms,
            bill_to: if v2 {
                Some(customer_snapshot(&store, customer_id, p.bill_to)?)
            } else {
                None
            },
            seller: if v2 {
                Some(seller_snapshot(&settings))
            } else {
                None
            },
            customer_ref: p.customer_ref.clone(),
            memo: p.memo.clone(),
        };
        let preview = store
            .preview_invoice(customer_id, date, due_date, lines.clone(), fields.clone())
            .map_err(|e| e.to_string())?;
        let terms_text = preview
            .invoice
            .terms
            .as_ref()
            .map(ledger_core::Terms::describe);
        let digest = self.preview_of(
            &store,
            "books.create_invoice",
            p.confirmed.unwrap_or(false),
            p.preview.as_deref(),
        )?;
        if !p.confirmed.unwrap_or(false) {
            let printed = ledger_package::draft_invoice_document(
                store.ledger(),
                &settings.company,
                &preview.invoice,
            )?;
            let bytes =
                ledger_import::invoice_pdf::invoice_pdf(&printed).map_err(|e| e.to_string())?;
            let file_name = format!(
                "{}-draft.pdf",
                preview.number.as_deref().unwrap_or("invoice")
            );
            let pdf = self.hand_over(&book_name(&store), &file_name, "application/pdf", bytes)?;
            return Ok(Json(InvoiceOut {
                preview: Some(digest),
                id: None,
                number: preview.number,
                total: money(preview.total),
                due_date: preview.due_date.to_string(),
                terms: terms_text,
                entry_id: None,
                lines: posting_out(store.ledger(), &preview.lines),
                pdf: Some(DocumentPdfOut::Draft(pdf)),
                mailto: None,
                warnings: Vec::new(),
                notes: Vec::new(),
            }));
        }
        // The number the preview showed is the one it issues (A-1: an
        // abandoned preview uses up no number).
        let fields = arap::InvoiceFields {
            number: preview.number.clone(),
            ..fields
        };
        // Rendered before anything is journaled: a Typst failure leaves
        // the books as they were (8.3.11).
        let printed = ledger_package::issued_invoice_document(
            store.ledger(),
            &settings.company,
            &preview.invoice,
        )?;
        let bytes = ledger_import::invoice_pdf::invoice_pdf(&printed).map_err(|e| e.to_string())?;
        let sha256 = ledger_store::journal::sha256_hex(&bytes);
        let first_new = store.ledger().next_account_id();
        let (id, entry_id, attachment_id) = store
            .create_invoice_issued(customer_id, date, due_date, lines.clone(), fields, bytes)
            .map_err(|e| e.to_string())?;
        let invoice = store
            .ledger()
            .get_invoice(id)
            .ok_or_else(|| format!("invoice {id} not found after creation"))?
            .clone();
        Ok(Json(InvoiceOut {
            preview: None,
            id: Some(id),
            number: invoice.number.clone(),
            total: money(invoice.total().map_err(|e| e.to_string())?),
            due_date: invoice.due_date.to_string(),
            terms: invoice.terms.as_ref().map(ledger_core::Terms::describe),
            entry_id: Some(entry_id),
            lines: Vec::new(),
            pdf: Some(DocumentPdfOut::Issued(IssuedFileOut {
                attachment_id,
                sha256,
            })),
            mailto: Some(mailto_link(&settings, &invoice)),
            warnings: created_uncategorized(store.ledger(), first_new),
            notes: document_notes(store.ledger(), &lines, true),
        }))
    }

    #[tool(
        name = "books.create_bill",
        title = "Create and post a bill",
        description = "Create a vendor bill: without confirmed it is a preview (the posting, due date and early discount) and nothing is written; with confirmed=true it posts Dr each line's account / Cr Accounts Payable and files the vendor's invoice (upload_id, or an existing_attachment_id) on its entry. The vendor's invoice number is vendor_ref; the same vendor and number twice is refused unless duplicate_reason says why. In books with documents_v2 every line names its account (A-10) and the file is required. account: an expense, or a non-cash asset; class: a project. Terms or a due date as for invoices.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn create_bill(
        &self,
        params: Parameters<CreateBillParams>,
    ) -> Result<Json<BillOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let vendor = p.vendor.trim().to_string();
        let vendor_id = resolve_vendor_arg(&store, &vendor)?;
        let date = parse_date(&p.date)?;
        let received_on = p.received_on.as_deref().map(parse_date).transpose()?;
        let lines = document_lines(&store, p.lines)?;
        let v2 = documents_v2(&store);
        let vendor_ref = p
            .vendor_ref
            .as_deref()
            .map(str::trim)
            .filter(|r| !r.is_empty())
            .map(str::to_string);
        if p.upload_id.is_some() && p.existing_attachment_id.is_some() {
            return Err(
                "give the vendor's invoice once: upload_id or existing_attachment_id, not both"
                    .into(),
            );
        }
        if let Some(m) = &p.memo {
            ledger_core::parties::check_text("the memo", m)?;
        }
        if let Some(r) = &p.duplicate_reason {
            ledger_core::parties::check_text("the duplicate reason", r)?;
        }
        // The file is taken before anything is written: an upload is used
        // up only by the call that files it (files.rs INVARIANT 1).
        let taken = match &p.upload_id {
            Some(id) => Some(self.take_file(files::Given {
                upload_id: id,
                purposes: files::FILED,
            })?),
            None => None,
        };
        if let Some(f) = &taken {
            self.refuse_tin_file(&store, &f.bytes, &f.mime, &f.file_name)?;
        }
        if let Some(aid) = p.existing_attachment_id {
            let kept = store
                .list_attachments(None)
                .map_err(|e| e.to_string())?
                .into_iter()
                .any(|a| a.id == aid && a.withdrawn_at.is_none());
            if !kept {
                return Err(format!(
                    "no attachment {aid} (or it was withdrawn): books.attachments lists them"
                ));
            }
        }
        let party = store.ledger().party_of_vendor(vendor_id);
        let (due_date, terms) = due_and_terms(
            date,
            p.due_date.as_deref(),
            p.terms.as_deref(),
            vec![party_default_terms(&store, party, false)],
            "the bill",
            "due_date, or terms, or the vendor's default terms",
        )?;
        let fields = arap::BillFields {
            vendor_ref: vendor_ref.clone(),
            received_on,
            terms,
            memo: p.memo.clone(),
            duplicate_reason: p.duplicate_reason.clone(),
        };
        let preview = store
            .preview_bill(vendor_id, date, due_date, lines.clone(), fields.clone())
            .map_err(|e| e.to_string())?;
        let discount_by = fields
            .terms
            .as_ref()
            .and_then(|t| t.discount_by(date))
            .map(|d| d.to_string());
        let max_discount = fields.terms.as_ref().and_then(|t| {
            let bp = t.discount()?.percent_bp;
            ledger_core::kernel::discount_max(preview.total, bp)
        });
        // What these books require of a new bill, after the preview has
        // run the issue's own checks: so a line naming a project the books
        // haven't declared (document_line_projects) still says so, rather
        // than being answered with a missing vendor number.
        if v2 {
            if vendor_ref.is_none() {
                return Err(
                    "give vendor_ref: the vendor's own invoice number. A second bill with the same vendor and number is refused unless duplicate_reason says why"
                        .into(),
                );
            }
            if let Some(n) = lines.iter().position(|l| l.account.is_none()) {
                return Err(format!(
                    "every bill line names its account in these books (A-10): line {} ({:?}) does not; name the expense, equipment or prepaid account it belongs to (books.chart)",
                    n + 1,
                    lines[n].description
                ));
            }
            if p.upload_id.is_none() && p.existing_attachment_id.is_none() {
                return Err(
                    "give the vendor's invoice: upload_id (books.begin_upload, then `wanderbooks upload`) or existing_attachment_id. Every bill keeps the invoice it was entered from (A-9)"
                        .into(),
                );
            }
        }
        let digest = self.preview_of(
            &store,
            "books.create_bill",
            p.confirmed.unwrap_or(false),
            p.preview.as_deref(),
        )?;
        if !p.confirmed.unwrap_or(false) {
            return Ok(Json(BillOut {
                preview: Some(digest),
                id: None,
                total: money(preview.total),
                due_date: preview.due_date.to_string(),
                terms: fields.terms.as_ref().map(ledger_core::Terms::describe),
                entry_id: None,
                lines: posting_out(store.ledger(), &preview.lines),
                discount_by,
                max_discount: max_discount.map(money),
                file: None,
                warnings: Vec::new(),
                notes: Vec::new(),
            }));
        }
        let first_new = store.ledger().next_account_id();
        let id = store
            .create_bill_with(vendor_id, date, due_date, lines.clone(), fields)
            .map_err(|e| e.to_string())?;
        let bill = store
            .ledger()
            .get_bill(id)
            .ok_or_else(|| format!("bill {id} not found after creation"))?
            .clone();
        let entry_id = bill.entry_id;
        // The vendor's invoice, filed on the bill's entry and linked to the
        // bill (A-9, A-13).
        let (attachment_id, sha256) = match (taken, p.existing_attachment_id) {
            (Some(f), _) => {
                let (aid, sha, _) = store
                    .attach_receipt(
                        Some(entry_id),
                        &f.file_name,
                        &f.mime,
                        &f.bytes,
                        Some("Vendor invoice"),
                    )
                    .map_err(|e| e.to_string())?;
                files::filed(f.claim);
                store
                    .link_attachment(aid, ledger_store::Subject::Bill(id))
                    .map_err(|e| e.to_string())?;
                (aid, sha)
            }
            (None, Some(aid)) => {
                store
                    .link_attachment(aid, ledger_store::Subject::Bill(id))
                    .map_err(|e| e.to_string())?;
                let linked = store
                    .attachments_for(ledger_store::Subject::Entry(entry_id))
                    .map_err(|e| e.to_string())?
                    .iter()
                    .any(|a| a.id == aid);
                if !linked {
                    store
                        .link_attachment(aid, ledger_store::Subject::Entry(entry_id))
                        .map_err(|e| e.to_string())?;
                }
                let sha = store
                    .list_attachments(None)
                    .map_err(|e| e.to_string())?
                    .into_iter()
                    .find(|a| a.id == aid)
                    .map(|a| a.blob_sha)
                    .unwrap_or_default();
                (aid, sha)
            }
            (None, None) => (0, String::new()),
        };
        Ok(Json(BillOut {
            preview: None,
            id: Some(id),
            total: money(bill.total().map_err(|e| e.to_string())?),
            due_date: bill.due_date.to_string(),
            terms: bill.terms.as_ref().map(ledger_core::Terms::describe),
            entry_id: Some(entry_id),
            lines: Vec::new(),
            discount_by,
            max_discount: max_discount.map(money),
            file: if attachment_id == 0 {
                None
            } else {
                Some(IssuedFileOut {
                    attachment_id,
                    sha256,
                })
            },
            warnings: created_uncategorized(store.ledger(), first_new),
            notes: document_notes(store.ledger(), &lines, false),
        }))
    }

    #[tool(
        name = "books.void_document",
        title = "Void an invoice or bill",
        description = "Void an invoice or bill that should never have existed (a duplicate, the wrong customer or vendor): reverses its own entry, dated as given, with the reason. It keeps its number and shows as void; nothing is deleted. Refused while a payment stands on it (reverse that payment first: the refusal names it), when it is void already, before its date, or in a closed period. To lower a valid document, not void it: a credit memo (later). Posts at once.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn void_document(
        &self,
        params: Parameters<VoidDocumentParams>,
    ) -> Result<Json<VoidDocumentOut>, String> {
        let p = params.0;
        let doc = doc_ref(&p.kind, p.id)?;
        let date = parse_date(&p.date)?;
        let mut store = self.lock()?;
        let entry_id = store
            .void_document(doc, date, &p.reason)
            .map_err(|e| e.to_string())?;
        let status = store
            .ledger()
            .doc_state(doc, date)
            .map_err(|e| e.to_string())?
            .map_or("void", |st| st.status.key());
        Ok(Json(VoidDocumentOut {
            entry_id,
            status: status.to_string(),
        }))
    }

    #[tool(
        name = "books.documents",
        title = "List invoices or bills",
        description = described(
            "List invoices (kind invoices: money owed to the company) or bills (kind bills: money it owes), due date first: each with its ref (for books.document), party, dates, total, what it still owes and its status, all derived from its ledger lines that stand. status: open (default: anything still owed), overdue, paid, void, written_off, overpaid, or all; party and due_by narrow it (due_by: what is due by a day, e.g. what to pay this week).",
            ledger_core::calendar::PeriodKind::AsOf
        ),
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn documents(
        &self,
        params: Parameters<DocumentsParams>,
    ) -> Result<Json<DocumentsOut>, String> {
        let p = params.0;
        let as_of = self.as_of(p.as_of.as_deref())?;
        let due_by = p.due_by.as_deref().map(parse_date).transpose()?;
        let invoices = match p.kind.trim().to_ascii_lowercase().as_str() {
            "invoices" | "invoice" => true,
            "bills" | "bill" => false,
            other => return Err(format!("bad kind {other:?}: expected invoices or bills")),
        };
        let want = p
            .status
            .as_deref()
            .unwrap_or("open")
            .trim()
            .to_ascii_lowercase();
        if !matches!(
            want.as_str(),
            "open" | "overdue" | "paid" | "void" | "written_off" | "overpaid" | "all"
        ) {
            return Err(format!(
                "bad status {want:?}: expected open, overdue, paid, void, written_off, overpaid or all"
            ));
        }
        let store = self.lock()?;
        let ledger = store.ledger();
        let docs: Vec<(
            ledger_core::DocRef,
            String,
            NaiveDate,
            NaiveDate,
            Option<String>,
        )> = if invoices {
            ledger
                .invoices()
                .into_iter()
                .map(|i| {
                    let who = ledger
                        .get_customer(i.customer_id)
                        .map(|c| c.name.clone())
                        .unwrap_or_default();
                    (
                        ledger_core::DocRef::Invoice(i.id),
                        who,
                        i.date,
                        i.due_date,
                        last_sent(&i.sends),
                    )
                })
                .collect()
        } else {
            ledger
                .bills()
                .into_iter()
                .map(|b| {
                    let who = ledger
                        .get_vendor(b.vendor_id)
                        .map(|v| v.name.clone())
                        .unwrap_or_default();
                    (
                        ledger_core::DocRef::Bill(b.id),
                        who,
                        b.date,
                        b.due_date,
                        None,
                    )
                })
                .collect()
        };
        let party = p.party.as_deref().map(|s| s.trim().to_lowercase());
        let mut rows = Vec::new();
        let mut owed = Decimal::ZERO;
        for (doc, who, date, due, last_sent) in docs {
            if let Some(want_party) = &party
                && who.to_lowercase() != *want_party
                && doc_id(doc).to_string() != *want_party
            {
                continue;
            }
            if due_by.is_some_and(|d| due > d) {
                continue;
            }
            let Some(st) = ledger.doc_state(doc, as_of).map_err(|e| e.to_string())? else {
                continue;
            };
            let key = st.status.key();
            let keep = match want.as_str() {
                "all" => true,
                "open" => !st.outstanding.is_zero() && key != "void",
                "overdue" => st.overdue_days > 0,
                other => key == other,
            };
            if !keep {
                continue;
            }
            owed = owed
                .checked_add(st.outstanding)
                .ok_or("the total is too large to compute")?;
            rows.push((
                due,
                doc_id(doc),
                DocumentRowOut {
                    r#ref: doc_ref_text(doc),
                    party: who,
                    date: date.to_string(),
                    due_date: due.to_string(),
                    total: money(st.total),
                    outstanding: money(st.outstanding),
                    status: key.to_string(),
                    overdue_days: st.overdue_days,
                    last_sent,
                },
            ));
        }
        rows.sort_by_key(|r| (r.0, r.1));
        Ok(Json(DocumentsOut {
            as_of: as_of.to_string(),
            kind: if invoices { "invoices" } else { "bills" }.to_string(),
            documents: rows.into_iter().map(|r| r.2).collect(),
            outstanding_total: money(owed),
        }))
    }

    #[tool(
        name = "books.document",
        title = "An invoice or bill",
        description = described(
            "One invoice or bill (ref \"invoice:7\" or \"bill:12\", from books.documents) as of a day: party, dates, lines with projects, total, paid, discounted, written off, still owed, status; and its timeline, every ledger line tagged with it in posting order, each saying whether its entry still stands (a reversed payment shows as the payment, not standing, and its reversal, standing: they net to nothing). pdf: true also hands over an invoice's PDF to download, re-created from the books.",
            ledger_core::calendar::PeriodKind::AsOf
        ),
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn document(
        &self,
        params: Parameters<DocumentParams>,
    ) -> Result<Json<DocumentOut>, String> {
        let p = params.0;
        let as_of = self.as_of(p.as_of.as_deref())?;
        let doc = parse_doc_ref(&p.r#ref)?;
        let store = self.lock()?;
        let ledger = store.ledger();
        let (who, date, due, entry_id, lines, sends) = match doc {
            ledger_core::DocRef::Invoice(id) => {
                let i = ledger
                    .get_invoice(id)
                    .ok_or_else(|| format!("no invoice {id}: see books.documents"))?;
                let who = ledger
                    .get_customer(i.customer_id)
                    .map(|c| c.name.clone())
                    .unwrap_or_default();
                (
                    who,
                    i.date,
                    i.due_date,
                    i.entry_id,
                    i.lines.clone(),
                    i.sends.iter().map(sent_out).collect(),
                )
            }
            ledger_core::DocRef::Bill(id) => {
                let b = ledger
                    .get_bill(id)
                    .ok_or_else(|| format!("no bill {id}: see books.documents"))?;
                let who = ledger
                    .get_vendor(b.vendor_id)
                    .map(|v| v.name.clone())
                    .unwrap_or_default();
                (
                    who,
                    b.date,
                    b.due_date,
                    b.entry_id,
                    b.lines.clone(),
                    Vec::new(),
                )
            }
        };
        let st = ledger
            .doc_state(doc, as_of)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("{} is dated after {as_of}", doc_ref_text(doc)))?;
        let pdf = match (p.pdf, doc) {
            (false, _) => None,
            (true, ledger_core::DocRef::Bill(_)) => {
                return Err(
                    "a bill is the vendor's document: its PDF is the file filed with it (books.list_receipts)"
                        .into(),
                );
            }
            (true, ledger_core::DocRef::Invoice(id)) => {
                let settings = self.settings();
                let printed =
                    ledger_package::invoice_document(ledger, &settings.company, id, as_of)?;
                let bytes =
                    ledger_import::invoice_pdf::invoice_pdf(&printed).map_err(|e| e.to_string())?;
                Some(self.hand_over(
                    &book_name(&store),
                    &format!("invoice-{id}.pdf"),
                    "application/pdf",
                    bytes,
                )?)
            }
        };
        let timeline = ledger
            .doc_lines(doc)
            .into_iter()
            .filter(|l| l.date <= as_of)
            .map(|l| DocumentEventOut {
                date: l.date.to_string(),
                entry_id: l.entry,
                kind: movement_key(l.movement).to_string(),
                amount: money(l.amount),
                reversal: l.reversal,
                stands: ledger.stands_on(l.entry, as_of),
            })
            .collect();
        Ok(Json(DocumentOut {
            r#ref: doc_ref_text(doc),
            party: who,
            date: date.to_string(),
            due_date: due.to_string(),
            entry_id,
            lines: lines
                .iter()
                .map(|l| DocumentLineOut {
                    description: l.description.clone(),
                    amount: money(l.amount),
                    project: l
                        .class_id
                        .and_then(|c| ledger.get_class(c))
                        .map(|c| c.name.clone()),
                })
                .collect(),
            total: money(st.total),
            paid: money(st.paid),
            discounted: money(st.discounted),
            written_off: money(st.written_off),
            outstanding: money(st.outstanding),
            status: st.status.key().to_string(),
            overdue_days: st.overdue_days,
            timeline,
            sends,
            pdf,
        }))
    }

    #[tool(
        name = "books.control_check",
        title = "Receivables and payables agree",
        description = described(
            "Check that Accounts Receivable and Accounts Payable equal their documents as of a day: for each control account, its ledger balance, what its open invoices or bills still owe on it, every line on it that is no document's (a hand entry or an older version's), the difference (always 0.00), and the documents needing attention (overpaid, or not in the ledger). agrees is true when there is nothing to look at. The first thing a CPA ties out.",
            ledger_core::calendar::PeriodKind::AsOf
        ),
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn control_check(
        &self,
        params: Parameters<AgingParams>,
    ) -> Result<Json<ControlCheckOut>, String> {
        let p = params.0;
        let as_of = self.as_of(p.as_of.as_deref())?;
        let store = self.lock()?;
        let ledger = store.ledger();
        let checks = ledger.control_check(as_of).map_err(|e| e.to_string())?;
        Ok(Json(control_check_out(ledger, as_of, &checks)))
    }
}
