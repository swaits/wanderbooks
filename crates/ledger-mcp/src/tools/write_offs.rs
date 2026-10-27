//! `books.write_off_invoice`'s group: bad debts (plan P4b S7; rules.md
//! B-41). A file of its own (plan 8.2 U0a) so S4.4's document tools and
//! this one land apart: a tool is added to its group's file and nowhere
//! else.

use super::*;

/// An invoice's figures after a write-off, as the tool answers them: read
/// from its tagged lines that stand (rules.md B-38), at the end of the
/// books.
fn write_off_out(
    ledger: &Ledger,
    invoice: u64,
    entry_id: u64,
) -> Result<WriteOffInvoiceOut, String> {
    let st = ledger
        .doc_state(ledger_core::DocRef::Invoice(invoice), NaiveDate::MAX)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("invoice {invoice} not found"))?;
    Ok(WriteOffInvoiceOut {
        entry_id,
        document: WriteOffDocumentOut {
            r#ref: format!("invoice:{invoice}"),
            status: st.status.key().to_string(),
            paid: money(st.paid),
            written_off: money(st.written_off),
            outstanding: money(st.outstanding),
        },
    })
}

#[tool_router(router = write_offs_router, vis = "pub(super)")]
impl BooksServer {
    #[tool(
        name = "books.write_off_invoice",
        title = "Write off an invoice",
        description = "Write off an invoice that will not be collected (a bad debt): Dr Bad Debt Expense 6990 / Cr Accounts Receivable, the receivable line tagged with the invoice, so it reads written_off and its figures stay derived. The evidence of worthlessness (collection correspondence, a bankruptcy notice) is filed with it in the same call: upload_id, or existing_attachment_id for one already filed. amount (default: all it owes) may be partial. Refused on cash-basis books, above what it owes, before its date or in a closed period. To recover it: reverse the write-off (books.reverse_entry; the invoice reopens), then books.receive_payment. Without confirmed=true, a preview.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn write_off_invoice(
        &self,
        params: Parameters<WriteOffInvoiceParams>,
    ) -> Result<Json<WriteOffInvoiceOut>, String> {
        let p = params.0;
        let date = parse_date(&p.date)?;
        let mut store = self.lock()?;
        // The event type is new: books that haven't declared documents_v2
        // refuse it with the upgrade remedy, journaling nothing (B-27).
        store
            .ensure_feature(ledger_store::format::DOCUMENTS_V2)
            .map_err(|e| e.to_string())?;
        let doc = ledger_core::DocRef::Invoice(p.invoice);
        let owed = store
            .ledger()
            .doc_state(doc, date)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("invoice {} is dated after {date}", p.invoice))?
            .outstanding;
        let amount = match p.amount.as_deref() {
            Some(a) => parse_amount(a)?,
            None => owed,
        };
        // The evidence is the point of a write-off (B-41 INVARIANT 1): one
        // of the two, and the preview says so too rather than posting
        // without it later.
        match (p.upload_id.is_some(), p.existing_attachment_id.is_some()) {
            (true, true) | (false, false) => {
                return Err(
                    "give the evidence once: upload_id (a file to file with the write-off) or \
                     existing_attachment_id (one already filed), exactly one"
                        .into(),
                );
            }
            _ => {}
        }
        if !p.confirmed.unwrap_or(false) {
            // The engine on a clone: every refusal the post would give, and
            // the figures the write-off would leave. Nothing is written.
            let mut probe = store.ledger().clone();
            probe
                .write_off(p.invoice, date, amount, &p.reason)
                .map_err(|e| e.to_string())?;
            return Ok(Json(write_off_out(&probe, p.invoice, 0)?));
        }
        let entry_id = match p.upload_id.as_deref() {
            Some(upload_id) => {
                let file = self.take_file(files::Given {
                    upload_id,
                    purposes: files::FILED,
                })?;
                let (file_name, mime_type) = named(None, None, &file)?;
                let (bytes, claim) = (file.bytes, file.claim);
                self.refuse_tin_file(&store, &bytes, &mime_type, &file_name)?;
                let (entry_id, _) = store
                    .write_off_invoice(
                        p.invoice,
                        date,
                        amount,
                        &p.reason,
                        Some(ledger_store::WriteOffEvidence::File {
                            file_name: &file_name,
                            mime_type: &mime_type,
                            bytes: &bytes,
                            description: Some("Evidence of worthlessness"),
                        }),
                    )
                    .map_err(|e| e.to_string())?;
                files::filed(claim);
                entry_id
            }
            None => {
                let id = p
                    .existing_attachment_id
                    .expect("checked: one of the two is given");
                let (entry_id, _) = store
                    .write_off_invoice(
                        p.invoice,
                        date,
                        amount,
                        &p.reason,
                        Some(ledger_store::WriteOffEvidence::Existing(id)),
                    )
                    .map_err(|e| e.to_string())?;
                entry_id
            }
        };
        Ok(Json(write_off_out(store.ledger(), p.invoice, entry_id)?))
    }
}
