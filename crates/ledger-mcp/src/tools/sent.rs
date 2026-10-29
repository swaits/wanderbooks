//! `books.mark_sent`'s group: the record that an invoice went out (plan
//! P4b S5, A-4). The books never send; this records who did, how, to whom,
//! when, whether it was the original or a reminder, and the channel's own
//! reference. A send moves no figure.

use super::*;

/// `books.mark_sent`'s `invoice`: `invoice:7` (as books.documents names
/// it), or a bare id.
fn invoice_id(r: &str) -> Result<u64, String> {
    let t = r.trim();
    let t = t.strip_prefix("invoice:").map(str::trim).unwrap_or(t);
    t.parse::<u64>()
        .map_err(|_| format!("bad invoice {r:?}: expected invoice:7 or 7"))
}

/// The channel a send went by: email, mail, portal, hand, or another
/// channel's own name (`Other`).
fn sent_via(s: &str) -> Result<ledger_core::arap::SentVia, String> {
    use ledger_core::arap::SentVia;
    let t = s.trim();
    match t.to_ascii_lowercase().as_str() {
        "email" => Ok(SentVia::Email),
        "mail" | "post" | "paper mail" => Ok(SentVia::Mail),
        "portal" => Ok(SentVia::Portal),
        "hand" | "in person" => Ok(SentVia::Hand),
        _ if t.is_empty() => {
            Err("via is empty: give email, mail, portal, hand or another channel".into())
        }
        _ => Ok(SentVia::Other(t.to_string())),
    }
}

/// original or reminder.
fn send_kind(s: &str) -> Result<ledger_core::arap::SendKind, String> {
    ledger_core::arap::SendKind::parse(s.trim())
        .ok_or_else(|| format!("bad kind {s:?}: expected original or reminder"))
}

#[tool_router(router = sent_router, vis = "pub(super)")]
impl BooksServer {
    #[tool(
        name = "books.mark_sent",
        title = "Record an invoice sent",
        description = "Record that an invoice went out: who sent it (by), how (via: email, mail, portal, hand, or another channel's name), to whom, the day, original or reminder, and the channel's own reference (a message id). The books never send mail: the owner or a delegate sends, then this records it. Several sends are allowed (reminders). A send changes no figure and never moves the due date. Refused on a void invoice, dated before it, with no recipients, or while the books wait for their format upgrade (documents_v2).",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn mark_sent(
        &self,
        params: Parameters<MarkSentParams>,
    ) -> Result<Json<MarkSentOut>, String> {
        use ledger_core::arap::{SendKind, Sent};
        let p = params.0;
        let on = self.as_of(p.on.as_deref())?;
        let id = invoice_id(&p.invoice)?;
        let mut store = self.lock()?;
        let (to, kind) = {
            let invoice = store
                .ledger()
                .get_invoice(id)
                .ok_or_else(|| format!("no invoice {id}: see books.documents"))?;
            let to = match p.recipients {
                Some(to) if !to.is_empty() => to,
                _ => {
                    let bill_to = invoice.bill_to.as_ref().and_then(|b| b.email.clone());
                    let party = store
                        .ledger()
                        .party_of_customer(invoice.customer_id)
                        .and_then(|pid| store.ledger().get_party(pid))
                        .and_then(|p| p.email.clone());
                    vec![bill_to.or(party).ok_or_else(|| {
                        format!(
                            "invoice {id} has no bill-to email and its customer no email: give to"
                        )
                    })?]
                }
            };
            let kind = match p.kind.as_deref() {
                Some(k) => send_kind(k)?,
                None if invoice.sends.is_empty() => SendKind::Original,
                None => SendKind::Reminder,
            };
            (to, kind)
        };
        let by = p.by.unwrap_or_else(|| {
            CALLER
                .with(|c| c.borrow().as_ref().map(|(_, name)| name.clone()))
                .unwrap_or_else(|| ledger_store::auth::DEFAULT_NAME.to_string())
        });
        let via = sent_via(p.via.as_deref().unwrap_or("email"))?;
        store
            .mark_sent(
                id,
                Sent {
                    on,
                    by,
                    via,
                    to,
                    kind,
                    reference: p.reference,
                    note: p.note,
                },
            )
            .map_err(|e| e.to_string())?;
        let invoice = store
            .ledger()
            .get_invoice(id)
            .ok_or_else(|| format!("invoice {id} not found after its send"))?;
        Ok(Json(MarkSentOut {
            invoice: format!("invoice:{id}"),
            sends: invoice.sends.iter().map(sent_out).collect(),
        }))
    }
}
