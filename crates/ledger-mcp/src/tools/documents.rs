//! `books.documents`' group: the A/R and A/P documents (invoices and
//! bills), the payments and voids against them, the read tools
//! (`books.documents`, `books.document`), the control check and the
//! agings. Split whole out of `server.rs` (plan/served.md 8.2 U0a): the
//! tools, their names, descriptions and rules are unchanged
//! (`docs/mcp-interface.md`; rules.md B-38).

use super::*;

/// A payment's method and reference from its parameters (the method by
/// id or label, books.instruments).
fn payment_details(
    store: &Store,
    p: &PaymentParams,
) -> Result<ledger_core::PaymentDetails, String> {
    Ok(ledger_core::PaymentDetails {
        instrument: p
            .instrument
            .as_deref()
            .map(|r| resolve_instrument(store, r))
            .transpose()?,
        reference: p.reference.clone(),
    })
}

/// What a payment tool answers: the document's figures after it, derived
/// from its lines that stand (rules.md B-38), as of the end of the books.
fn payment_out(
    ledger: &Ledger,
    doc: ledger_core::DocRef,
    entry_id: u64,
) -> Result<Json<PaymentOut>, String> {
    let st = ledger
        .doc_state(doc, chrono::NaiveDate::MAX)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("{} not found", doc_label(doc)))?;
    Ok(Json(PaymentOut {
        entry_id,
        remaining: money(st.outstanding),
        status: st.status.key().to_string(),
    }))
}

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

/// A document's kind and id for people ("invoice 7").
fn doc_label(doc: ledger_core::DocRef) -> String {
    match doc {
        ledger_core::DocRef::Invoice(id) => format!("invoice {id}"),
        ledger_core::DocRef::Bill(id) => format!("bill {id}"),
    }
}

/// An invoice's or bill's lines as the tool takes them: each amount
/// parsed and each project resolved ([`resolve_class`]) before anything is
/// written.
fn document_lines(
    store: &Store,
    lines: Vec<DocumentLineParam>,
) -> Result<Vec<DocumentLine>, String> {
    lines
        .into_iter()
        .map(|l| {
            Ok(DocumentLine {
                description: l.description,
                amount: parse_amount(&l.amount)?,
                class_id: l
                    .class
                    .as_deref()
                    .map(|c| resolve_class(store, c))
                    .transpose()?,
                account: None,
                quantity: None,
                unit_price: None,
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

#[tool_router(router = documents_router, vis = "pub(super)")]
impl BooksServer {
    #[tool(
        name = "books.create_invoice",
        title = "Create and post an invoice",
        description = "Create a sales invoice. Auto-posts Dr Accounts Receivable / Cr Revenue. Lines are {description, amount, class?} with decimal-string amounts; class (a project: name or id, books.projects) tags that line's revenue: one revenue line per project, the untagged lines' as one; the receivable line is never tagged. Its result gains notes for a finished, abandoned or on-spec project (on spec: is there now an agreement?), tagged anyway.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn create_invoice(
        &self,
        params: Parameters<CreateDocumentParams>,
    ) -> Result<Json<CreateDocumentOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let lines = document_lines(&store, p.lines)?;
        let first_new = store.ledger().next_account_id();
        let id = store
            .create_invoice(
                p.party_id,
                parse_date(&p.date)?,
                parse_date(&p.due_date)?,
                lines,
            )
            .map_err(|e| e.to_string())?;
        let inv = store
            .ledger()
            .get_invoice(id)
            .ok_or_else(|| format!("invoice {id} not found after creation"))?;
        Ok(Json(CreateDocumentOut {
            id,
            total: inv.total().map_err(|e| e.to_string())?.to_string(),
            entry_id: inv.entry_id,
            warnings: created_uncategorized(store.ledger(), first_new),
            notes: document_notes(store.ledger(), &inv.lines, true),
        }))
    }

    #[tool(
        name = "books.create_bill",
        title = "Create and post a bill",
        description = "Create a vendor bill. Auto-posts Dr General Expenses / Cr Accounts Payable. Lines are {description, amount, class?} with decimal-string amounts; class (a project: name or id, books.projects) tags that line's cost: one expense line per project, the untagged lines' as one; the payable line is never tagged. Its result gains notes for a finished or abandoned project, tagged anyway.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn create_bill(
        &self,
        params: Parameters<CreateDocumentParams>,
    ) -> Result<Json<CreateDocumentOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let lines = document_lines(&store, p.lines)?;
        let first_new = store.ledger().next_account_id();
        let id = store
            .create_bill(
                p.party_id,
                parse_date(&p.date)?,
                parse_date(&p.due_date)?,
                lines,
            )
            .map_err(|e| e.to_string())?;
        let bill = store
            .ledger()
            .get_bill(id)
            .ok_or_else(|| format!("bill {id} not found after creation"))?;
        Ok(Json(CreateDocumentOut {
            id,
            total: bill.total().map_err(|e| e.to_string())?.to_string(),
            entry_id: bill.entry_id,
            warnings: created_uncategorized(store.ledger(), first_new),
            notes: document_notes(store.ledger(), &bill.lines, false),
        }))
    }

    #[tool(
        name = "books.receive_payment",
        title = "Receive an invoice payment",
        description = "Record a customer payment against an invoice. Posts Dr Cash / Cr A/R, the receivable line tagged with the invoice (its figures are derived from its lines that stand: a reversed payment no longer counts). Name instrument (how the money moved, books.instruments) and the processor's or bank's reference when known. The books record payments; they never collect them (D-45). Refused above what it owes, before its date, or into an account that isn't cash.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn receive_payment(
        &self,
        params: Parameters<PaymentParams>,
    ) -> Result<Json<PaymentOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let cash = resolve_account(&store, &p.cash_account)?;
        let details = payment_details(&store, &p)?;
        let entry_id = store
            .receive_payment_with(
                p.document_id,
                parse_date(&p.date)?,
                parse_amount(&p.amount)?,
                cash,
                &details,
            )
            .map_err(|e| e.to_string())?;
        payment_out(
            store.ledger(),
            ledger_core::DocRef::Invoice(p.document_id),
            entry_id,
        )
    }

    #[tool(
        name = "books.pay_bill",
        title = "Pay a bill",
        description = "Record a payment of a vendor bill. Posts Dr A/P / Cr Cash, the payable line tagged with the bill (its figures are derived from its lines that stand: a reversed payment no longer counts). Name instrument (how the money moved, books.instruments: a card payment is left out of 1099-NEC) and the payment's reference when known. The books record payments; they never make them (D-45). Refused above what it owes, before its date, or from an account that isn't cash.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn pay_bill(
        &self,
        params: Parameters<PaymentParams>,
    ) -> Result<Json<PaymentOut>, String> {
        let p = params.0;
        let mut store = self.lock()?;
        let cash = resolve_account(&store, &p.cash_account)?;
        let details = payment_details(&store, &p)?;
        let entry_id = store
            .pay_bill_with(
                p.document_id,
                parse_date(&p.date)?,
                parse_amount(&p.amount)?,
                cash,
                &details,
            )
            .map_err(|e| e.to_string())?;
        payment_out(
            store.ledger(),
            ledger_core::DocRef::Bill(p.document_id),
            entry_id,
        )
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
        let docs: Vec<(ledger_core::DocRef, String, NaiveDate, NaiveDate)> = if invoices {
            ledger
                .invoices()
                .into_iter()
                .map(|i| {
                    let who = ledger
                        .get_customer(i.customer_id)
                        .map(|c| c.name.clone())
                        .unwrap_or_default();
                    (ledger_core::DocRef::Invoice(i.id), who, i.date, i.due_date)
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
                    (ledger_core::DocRef::Bill(b.id), who, b.date, b.due_date)
                })
                .collect()
        };
        let party = p.party.as_deref().map(|s| s.trim().to_lowercase());
        let mut rows = Vec::new();
        let mut owed = Decimal::ZERO;
        for (doc, who, date, due) in docs {
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
        let (who, date, due, entry_id, lines) = match doc {
            ledger_core::DocRef::Invoice(id) => {
                let i = ledger
                    .get_invoice(id)
                    .ok_or_else(|| format!("no invoice {id}: see books.documents"))?;
                let who = ledger
                    .get_customer(i.customer_id)
                    .map(|c| c.name.clone())
                    .unwrap_or_default();
                (who, i.date, i.due_date, i.entry_id, i.lines.clone())
            }
            ledger_core::DocRef::Bill(id) => {
                let b = ledger
                    .get_bill(id)
                    .ok_or_else(|| format!("no bill {id}: see books.documents"))?;
                let who = ledger
                    .get_vendor(b.vendor_id)
                    .map(|v| v.name.clone())
                    .unwrap_or_default();
                (who, b.date, b.due_date, b.entry_id, b.lines.clone())
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
                let printed = ledger_package::invoice_document(
                    ledger,
                    &settings.company,
                    id,
                    as_of,
                    today(),
                )?;
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

    #[tool(
        name = "books.ar_aging",
        title = "Receivables aging",
        description = described(
            "A/R aging report as of a date: outstanding balances per customer, bucketed by days past due (current, 30, 60, 90, 90+).",
            ledger_core::calendar::PeriodKind::AsOf
        ),
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn ar_aging(&self, params: Parameters<AgingParams>) -> Result<Json<AgingOut>, String> {
        let p = params.0;
        let as_of = self.as_of(p.as_of.as_deref())?;
        let store = self.lock()?;
        let rows = store
            .ledger()
            .ar_aging(as_of)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|r| {
                let total = r.total().map_err(|e| e.to_string())?.to_string();
                Ok(AgingRowOut {
                    name: r.name,
                    current: money(r.current),
                    days_30: money(r.days_30),
                    days_60: money(r.days_60),
                    days_90: money(r.days_90),
                    days_90_plus: money(r.days_90_plus),
                    total,
                })
            })
            .collect::<Result<_, String>>()?;
        Ok(Json(AgingOut {
            as_of: as_of.to_string(),
            rows,
        }))
    }

    #[tool(
        name = "books.ap_aging",
        title = "Payables aging",
        description = described(
            "A/P aging report as of a date: outstanding balances per vendor, bucketed by days past due (current, 30, 60, 90, 90+).",
            ledger_core::calendar::PeriodKind::AsOf
        ),
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn ap_aging(&self, params: Parameters<AgingParams>) -> Result<Json<AgingOut>, String> {
        let p = params.0;
        let as_of = self.as_of(p.as_of.as_deref())?;
        let store = self.lock()?;
        let rows = store
            .ledger()
            .ap_aging(as_of)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|r| {
                let total = r.total().map_err(|e| e.to_string())?.to_string();
                Ok(AgingRowOut {
                    name: r.name,
                    current: money(r.current),
                    days_30: money(r.days_30),
                    days_60: money(r.days_60),
                    days_90: money(r.days_90),
                    days_90_plus: money(r.days_90_plus),
                    total,
                })
            })
            .collect::<Result<_, String>>()?;
        Ok(Json(AgingOut {
            as_of: as_of.to_string(),
            rows,
        }))
    }
}
