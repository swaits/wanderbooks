//! `books.receive_payment`, `books.pay_bill`, `books.bills_due` and the
//! agings: paying on terms (plan P4b S6; rules.md B-42). A payment settles
//! one or several invoices (or bills) of one party, with the early-payment
//! discounts their terms allow and the processor's fee on a customer
//! payment posted gross. Split from `tools/documents.rs` so S4.4's work
//! there and this slice's do not collide (plan/served.md 8.2 P4b S6).

use super::*;

/// A document as the payment tools and the agings name it ("invoice:7",
/// "bill:12").
fn doc_ref_text(doc: ledger_core::DocRef) -> String {
    match doc {
        ledger_core::DocRef::Invoice(id) => format!("invoice:{id}"),
        ledger_core::DocRef::Bill(id) => format!("bill:{id}"),
    }
}

/// A document's number, when the books have one: an invoice's issued
/// number, or a bill's vendor invoice number.
fn doc_number(ledger: &Ledger, doc: ledger_core::DocRef) -> Option<String> {
    match doc {
        ledger_core::DocRef::Invoice(id) => ledger.get_invoice(id).and_then(|i| i.number.clone()),
        ledger_core::DocRef::Bill(id) => ledger.get_bill(id).and_then(|b| b.vendor_ref.clone()),
    }
}

/// A document's party (the customer of an invoice, the vendor of a bill).
fn doc_party(ledger: &Ledger, doc: ledger_core::DocRef) -> Result<u64, String> {
    match doc {
        ledger_core::DocRef::Invoice(id) => ledger
            .get_invoice(id)
            .map(|i| i.customer_id)
            .ok_or_else(|| format!("no invoice {id}: see books.documents")),
        ledger_core::DocRef::Bill(id) => ledger
            .get_bill(id)
            .map(|b| b.vendor_id)
            .ok_or_else(|| format!("no bill {id}: see books.documents")),
    }
}

/// The one id a payment tool's single form names: `invoice`/`bill`, or
/// the older `document_id`. Both at once is refused unless they agree.
fn single_document(primary: Option<u64>, alias: Option<u64>, what: &str) -> Result<u64, String> {
    match (primary, alias) {
        (Some(a), Some(b)) if a != b => Err(format!(
            "pass {what} or document_id, not both: {} and {b} differ",
            a
        )),
        (Some(id), _) | (None, Some(id)) => Ok(id),
        (None, None) => Err(format!(
            "pass {what} (or an applications list): see books.documents"
        )),
    }
}

/// The one account a payment tool's single form names: `deposit_to`/
/// `paid_from`, or the older `cash_account`.
fn single_account(
    primary: &Option<String>,
    alias: &Option<String>,
    what: &str,
) -> Result<String, String> {
    match (primary, alias) {
        (Some(a), Some(b)) if a != b => Err(format!(
            "pass {what} or cash_account, not both: {a:?} and {b:?} differ"
        )),
        (Some(a), _) | (None, Some(a)) => Ok(a.clone()),
        (None, None) => Err(format!("pass {what} (or cash_account)")),
    }
}

/// Every document a payment tool's parameters name, as the core takes
/// them: the single form (`invoice`/`bill` with `amount`) or the
/// applications list, never both.
fn applications(
    store: &Store,
    p: &PaymentParams,
    invoices: bool,
) -> Result<Vec<ledger_core::Application>, String> {
    if !p.applications.is_empty() {
        if p.amount.is_some() {
            return Err(
                "pass amount with invoice or bill, or an applications list, not both".into(),
            );
        }
        return p
            .applications
            .iter()
            .map(|a| {
                let (id, other) = if invoices {
                    (a.invoice, a.bill)
                } else {
                    (a.bill, a.invoice)
                };
                if other.is_some() {
                    return Err(format!(
                        "an application names {} only, not {}",
                        if invoices { "invoice" } else { "bill" },
                        if invoices { "bill" } else { "invoice" }
                    ));
                }
                let id = id.ok_or_else(|| {
                    format!(
                        "an application needs its {}",
                        if invoices { "invoice" } else { "bill" }
                    )
                })?;
                let doc = if invoices {
                    ledger_core::DocRef::Invoice(id)
                } else {
                    ledger_core::DocRef::Bill(id)
                };
                // Refuse an unknown document here, before anything is
                // journaled, with the list tool named.
                doc_party(store.ledger(), doc)?;
                Ok(ledger_core::Application {
                    doc,
                    cash: parse_amount(&a.amount)?,
                    discount: match &a.discount {
                        Some(d) => parse_amount(d)?,
                        None => Decimal::ZERO,
                    },
                    discount_reason: a.discount_reason.clone(),
                })
            })
            .collect();
    }
    let id = single_document(
        if invoices { p.invoice } else { p.bill },
        p.document_id,
        if invoices { "invoice" } else { "bill" },
    )?;
    let amount = p
        .amount
        .as_deref()
        .ok_or_else(|| "pass amount (or an applications list)".to_string())?;
    let doc = if invoices {
        ledger_core::DocRef::Invoice(id)
    } else {
        ledger_core::DocRef::Bill(id)
    };
    doc_party(store.ledger(), doc)?;
    Ok(vec![ledger_core::Application {
        doc,
        cash: parse_amount(amount)?,
        discount: Decimal::ZERO,
        discount_reason: None,
    }])
}

/// The settlement one payment tool describes, and the warnings its
/// parameters add.
fn settlement(
    store: &Store,
    p: &PaymentParams,
    invoices: bool,
) -> Result<(ledger_core::SettleParams, Vec<String>), String> {
    if p.applications.is_empty() {
        if invoices && p.bill.is_some() {
            return Err("books.receive_payment takes invoice, not bill".into());
        }
        if !invoices && p.invoice.is_some() {
            return Err("books.pay_bill takes bill, not invoice".into());
        }
    }
    let apps = applications(store, p, invoices)?;
    let first = apps
        .first()
        .ok_or_else(|| "a payment needs at least one invoice or bill".to_string())?
        .doc;
    let party = doc_party(store.ledger(), first)?;
    let cash_account = resolve_account(
        store,
        &single_account(
            if invoices {
                &p.deposit_to
            } else {
                &p.paid_from
            },
            &p.cash_account,
            if invoices { "deposit_to" } else { "paid_from" },
        )?,
    )?;
    let details = ledger_core::PaymentDetails {
        instrument: p
            .instrument
            .as_deref()
            .map(|r| resolve_instrument(store, r))
            .transpose()?,
        reference: p.reference.clone(),
    };
    let count = apps.len();
    let base = if invoices {
        if count == 1 {
            format!(
                "Payment on invoice {}",
                match first {
                    ledger_core::DocRef::Invoice(id) => id,
                    ledger_core::DocRef::Bill(_) => 0,
                }
            )
        } else {
            format!("Payment on {count} invoices")
        }
    } else if count == 1 {
        format!(
            "Payment on bill {}",
            match first {
                ledger_core::DocRef::Bill(id) => id,
                ledger_core::DocRef::Invoice(_) => 0,
            }
        )
    } else {
        format!("Payment on {count} bills")
    };
    let mut description = details.description(&base);
    if !invoices && p.fee.is_some() {
        return Err("a processor's fee belongs on a customer payment, not on paying a bill".into());
    }
    let fee = match &p.fee {
        None => None,
        Some(fee) => {
            let fee = ledger_core::Fee {
                amount: parse_amount(&fee.amount)?,
                account: fee
                    .account
                    .as_deref()
                    .map(|a| resolve_account(store, a))
                    .transpose()?,
                vendor: fee
                    .vendor
                    .as_deref()
                    .map(|v| resolve_vendor(store, v))
                    .transpose()?,
            };
            if let Some(vendor) = fee.vendor
                && let Some(v) = store.ledger().get_vendor(vendor)
            {
                description = format!("{description} (processing fee to {})", v.name);
            }
            Some(fee)
        }
    };
    let mut warnings = Vec::new();
    if p.paid_in_full_marking {
        warnings.push(
            "The check or note is marked paid in full of a disputed amount. Cashing it may settle \
             the whole claim (UCC §3-311): the books record only this payment; a lawyer or the \
             payer's agreement settles the rest."
                .to_string(),
        );
    }
    Ok((
        ledger_core::SettleParams {
            date: parse_date(&p.date)?,
            description,
            cash_account,
            party,
            applications: apps,
            fee,
            instrument: details.instrument,
            reference: details.reference,
        },
        warnings,
    ))
}

/// What a payment settled: each document's figures after it (derived from
/// its lines that stand, rules.md B-38) and what reached or left the
/// account.
fn payment_out(
    ledger: &Ledger,
    entry_id: u64,
    params: &ledger_core::SettleParams,
) -> Result<Json<PaymentOut>, String> {
    let mut documents = Vec::with_capacity(params.applications.len());
    let mut deposited = Decimal::ZERO;
    for app in &params.applications {
        deposited += app.cash;
        let st = ledger
            .doc_state(app.doc, chrono::NaiveDate::MAX)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("{} not found", doc_ref_text(app.doc)))?;
        documents.push(PaidDocumentOut {
            r#ref: doc_ref_text(app.doc),
            number: doc_number(ledger, app.doc),
            outstanding: money(st.outstanding),
            status: st.status.key().to_string(),
        });
    }
    let fee = params.fee.as_ref().map_or(Decimal::ZERO, |f| f.amount);
    deposited -= fee;
    let (remaining, status) = match documents.as_slice() {
        [one] => (Some(one.outstanding.clone()), Some(one.status.clone())),
        _ => (None, None),
    };
    Ok(Json(PaymentOut {
        preview: None,
        entry_id,
        documents,
        deposited: money(deposited),
        remaining,
        status,
        warnings: Vec::new(),
    }))
}

/// A payment's preview: the figures the settled documents would show
/// (entry 0: nothing is posted), with the digest to confirm it by.
fn payment_preview(
    probe: &Ledger,
    settle: &ledger_core::SettleParams,
    warnings: Vec<String>,
    digest: String,
) -> Result<Json<PaymentOut>, String> {
    let mut out = payment_out(probe, 0, settle)?.0;
    out.warnings = warnings;
    out.preview = Some(digest);
    Ok(Json(out))
}

#[tool_router(router = settlements_router, vis = "pub(super)")]
impl BooksServer {
    #[tool(
        name = "books.receive_payment",
        title = "Receive a payment",
        description = "Record a customer payment against one or several invoices of one customer. Posts Dr the deposit account (the cash less any processor fee), Dr payment-processing fees the fee, Dr sales allowances the discounts, Cr A/R for each invoice's cash and discount, each receivable line tagged with its invoice. An early-payment discount may be at most the invoice's terms allow and dated by their discount date; a later or larger one needs discount_reason. Name instrument (books.instruments) and the processor's reference. Refused above what an invoice owes, before its date, or into an account that is not cash. The books record payments; they never collect them (D-45).",
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
        let (settle, warnings) = settlement(&store, &p, true)?;
        let confirmed = p.confirmed.unwrap_or(false);
        let digest = self.preview_of(
            &store,
            "books.receive_payment",
            confirmed,
            p.preview.as_deref(),
        )?;
        let entry_id = if confirmed {
            store.settle(settle.clone()).map_err(|e| e.to_string())?
        } else {
            // The engine on a clone: every refusal the post would give,
            // and the figures it would leave. Nothing is written.
            let mut probe = store.ledger().clone();
            probe.settle(settle.clone()).map_err(|e| e.to_string())?;
            return payment_preview(&probe, &settle, warnings, digest);
        };
        let mut out = payment_out(store.ledger(), entry_id, &settle)?.0;
        out.warnings = warnings;
        out.preview = Some(digest);
        Ok(Json(out))
    }

    #[tool(
        name = "books.pay_bill",
        title = "Pay a bill",
        description = "Record paying one or several vendor bills of one vendor. Posts Dr A/P for each bill's cash and discount, each payable line tagged with its bill; Cr the payer account the cash and purchase discounts the discounts. An early-payment discount may be at most the bill's terms allow and dated by their discount date; a later or larger one needs discount_reason, and a bill with an asset or prepaid line refuses one (record it net, or reclassify Dr 6995 / Cr the asset). Name instrument (books.instruments: a card payment is left out of 1099-NEC) and the reference. Refused above what a bill owes, before its date, or from an account that is not cash. The books record payments; they never make them (D-45).",
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
        let (settle, warnings) = settlement(&store, &p, false)?;
        let confirmed = p.confirmed.unwrap_or(false);
        let digest = self.preview_of(&store, "books.pay_bill", confirmed, p.preview.as_deref())?;
        if !confirmed {
            let mut probe = store.ledger().clone();
            probe.settle(settle.clone()).map_err(|e| e.to_string())?;
            return payment_preview(&probe, &settle, warnings, digest);
        }
        let entry_id = store.settle(settle.clone()).map_err(|e| e.to_string())?;
        let mut out = payment_out(store.ledger(), entry_id, &settle)?.0;
        out.preview = Some(digest);
        Ok(Json(out))
    }

    #[tool(
        name = "books.bills_due",
        title = "Bills due",
        description = described(
            "What to pay and when: every open bill due on or before a day (through; omitted: today), due date first, with its vendor, the vendor's invoice number and what it still owes, and the early-payment discount each vendor's terms give (the last day it is open, the most off, and what to pay to take it). by_date is the running total of what falls due through each date, for planning. The figures are read from the bills' ledger lines that stand (B-38).",
            ledger_core::calendar::PeriodKind::AsOf
        ),
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn bills_due(
        &self,
        params: Parameters<BillsDueParams>,
    ) -> Result<Json<BillsDueOut>, String> {
        let p = params.0;
        let as_of = self.as_of(p.as_of.as_deref())?;
        let through = p
            .through
            .as_deref()
            .map(parse_date)
            .transpose()?
            .unwrap_or_else(today);
        let store = self.lock()?;
        let ledger = store.ledger();
        let mut bills: Vec<(DueBillOut, Decimal)> = Vec::new();
        for bill in ledger.bills() {
            let doc = ledger_core::DocRef::Bill(bill.id);
            let Some(st) = ledger.doc_state(doc, as_of).map_err(|e| e.to_string())? else {
                continue;
            };
            if st.outstanding <= Decimal::ZERO || bill.due_date > through {
                continue;
            }
            let discounts = bill.terms.as_ref().and_then(|terms| {
                let early = terms.discount()?;
                let by = terms.discount_by(bill.date)?;
                let max = ledger_core::kernel::discount_max(bill.total().ok()?, early.percent_bp)?;
                Some((by, max))
            });
            let (discount_by, max_discount, pay) = match discounts {
                Some((by, max)) if by >= as_of => (
                    Some(by.to_string()),
                    Some(money(max)),
                    Some(money(st.outstanding - max)),
                ),
                _ => (None, None, None),
            };
            bills.push((
                DueBillOut {
                    r#ref: doc_ref_text(doc),
                    vendor: ledger
                        .get_vendor(bill.vendor_id)
                        .map(|v| v.name.clone())
                        .unwrap_or_default(),
                    vendor_ref: bill.vendor_ref.clone(),
                    due_date: bill.due_date.to_string(),
                    outstanding: money(st.outstanding),
                    discount_by,
                    max_discount,
                    pay_by_discount_date_amount: pay,
                },
                st.outstanding,
            ));
        }
        bills.sort_by_key(|(b, _)| (b.due_date.clone(), b.r#ref.clone()));
        let mut by_date = Vec::new();
        let mut running = Decimal::ZERO;
        for (bill, owed) in &bills {
            running += owed;
            match by_date.last_mut() {
                Some(DueDateTotalOut { date, amount }) if *date == bill.due_date => {
                    *amount = money(running);
                }
                _ => by_date.push(DueDateTotalOut {
                    date: bill.due_date.clone(),
                    amount: money(running),
                }),
            }
        }
        Ok(Json(BillsDueOut {
            as_of: as_of.to_string(),
            through: through.to_string(),
            bills: bills.into_iter().map(|(b, _)| b).collect(),
            by_date,
        }))
    }

    #[tool(
        name = "books.ar_aging",
        title = "Receivables aging",
        description = described(
            "A/R aging report as of a date: outstanding balances per customer, bucketed by days past due (current, 30, 60, 90, 90+), with the receivables account's tie-out (its balance against what its invoices owe, its lines no invoice's, the difference: always 0.00). detail: true adds one row per invoice, with its number, dates and terms.",
            ledger_core::calendar::PeriodKind::AsOf
        ),
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn ar_aging(&self, params: Parameters<AgingParams>) -> Result<Json<AgingOut>, String> {
        aging(self, params.0, true)
    }

    #[tool(
        name = "books.ap_aging",
        title = "Payables aging",
        description = described(
            "A/P aging report as of a date: outstanding balances per vendor, bucketed by days past due (current, 30, 60, 90, 90+), with the payables account's tie-out (its balance against what its bills owe, its lines no bill's, the difference: always 0.00). detail: true adds one row per bill, with its number, dates and terms.",
            ledger_core::calendar::PeriodKind::AsOf
        ),
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn ap_aging(&self, params: Parameters<AgingParams>) -> Result<Json<AgingOut>, String> {
        aging(self, params.0, false)
    }
}

/// An aging of invoices (`invoices` true) or bills, with its tie-out.
fn aging(server: &BooksServer, p: AgingParams, invoices: bool) -> Result<Json<AgingOut>, String> {
    let as_of = server.as_of(p.as_of.as_deref())?;
    let store = server.lock()?;
    let ledger = store.ledger();
    let rows = if invoices {
        ledger.ar_aging(as_of)
    } else {
        ledger.ap_aging(as_of)
    }
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
    .collect::<Result<Vec<_>, String>>()?;
    let documents = if p.detail {
        ledger
            .aging_documents(as_of, invoices)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|d| {
                Ok(AgingDocumentOut {
                    r#ref: doc_ref_text(d.doc),
                    number: doc_number(ledger, d.doc),
                    party: d.party,
                    date: d.date.to_string(),
                    due_date: d.due_date.to_string(),
                    terms: d.terms,
                    total: money(d.total),
                    paid: money(d.paid),
                    discounted: money(d.discounted),
                    outstanding: money(d.outstanding),
                    bucket: d.bucket.to_string(),
                    days_overdue: d.days_overdue,
                })
            })
            .collect::<Result<Vec<_>, String>>()?
    } else {
        Vec::new()
    };
    let checks = ledger.control_check(as_of).map_err(|e| e.to_string())?;
    let role = if invoices { "receivables" } else { "payables" };
    let control = match checks.iter().find(|c| c.role == role) {
        Some(check) => {
            let account = ledger.chart().get(check.account);
            AgingControlOut {
                account: match account {
                    Some(a) => format!("{} {}", a.code, a.name),
                    None => String::new(),
                },
                balance: money(check.balance),
                documents_total: money(check.documents_total),
                unattached: check
                    .unattached
                    .iter()
                    .map(|u| UnattachedLineOut {
                        entry_id: u.entry,
                        line: u.line,
                        date: u.date.to_string(),
                        amount: money(u.amount),
                    })
                    .collect(),
                difference: money(check.difference),
            }
        }
        // No document of this kind has posted yet: nothing to tie out.
        None => AgingControlOut {
            account: String::new(),
            balance: money(Decimal::ZERO),
            documents_total: money(Decimal::ZERO),
            unattached: Vec::new(),
            difference: money(Decimal::ZERO),
        },
    };
    Ok(Json(AgingOut {
        as_of: as_of.to_string(),
        rows,
        documents,
        control,
    }))
}
