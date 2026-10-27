//! An invoice as the books hold it, ready to print (plan P4b.11): the
//! data `ledger_import::invoice_pdf` typesets, built in one place so the
//! server and the viewer give the same bytes for the same books.
//!
//! ```text
//! GOAL: print an invoice from the books, as of a day.
//! SOURCE: plan/served.md P4b.11 (the "current" version); rules.md B-38
//!         (a document's figures are its lines that stand).
//! INVARIANTS:
//!   1. Its figures are the invoice's derived state as of the day (as of
//!      its own date when the day is earlier): total, what was paid,
//!      discounted and written off, and the balance due.
//!   2. Its lines are the invoice's, in order; projects, accounts and ids
//!      never print.
//!   3. Each payment that stands prints on its own line, with its date,
//!      payment method and reference ("Payment Feb 10, 2026 · Harbor
//!      checking · ref tr_8812"), so a paid invoice is the customer's
//!      receipt. Discounts and write-offs print as one line each.
//!   4. Stamps: void VOID; paid in full PAID with the day the last
//!      payment came in; owing and past due PAST DUE (over PARTIALLY
//!      PAID); part paid PARTIALLY PAID; otherwise none.
//!   5. Until invoices keep the copy issued (P4b S4), every render says it
//!      was re-created from the books, and on which day.
//!   6. The seller is the company (`company.name`, `company.address`); the
//!      customer is its party's display name and address when it has
//!      them, else the customer's name.
//! ```

use chrono::NaiveDate;
use ledger_core::settings::Company;
use ledger_core::{DocRef, DocStatus, InvoiceId, Ledger};
use ledger_import::{InvoiceDoc, InvoiceItem, InvoiceParty, InvoiceStamp};

/// A date as an invoice prints it: "Feb 1, 2026".
fn printed(d: NaiveDate) -> String {
    d.format("%b %-d, %Y").to_string()
}

/// An address's lines: one per line of the text, blank ones dropped.
fn address_lines(address: Option<&str>) -> Vec<String> {
    address
        .unwrap_or("")
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

/// Invoice `id` as of `as_of`, re-created on `today` (INVARIANT 4).
pub fn invoice_document(
    ledger: &Ledger,
    company: &Company,
    id: InvoiceId,
    as_of: NaiveDate,
    today: NaiveDate,
) -> Result<InvoiceDoc, String> {
    let inv = ledger
        .get_invoice(id)
        .ok_or_else(|| format!("no invoice {id}"))?;
    let doc = DocRef::Invoice(id);
    let st = ledger
        .doc_state(doc, as_of.max(inv.date))
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("invoice {id} is not in the ledger"))?;
    let customer = ledger.get_customer(inv.customer_id);
    let party = ledger
        .party_of_customer(inv.customer_id)
        .and_then(|p| ledger.get_party(p));
    let bill_to = InvoiceParty {
        name: party
            .and_then(|p| p.display_name.clone())
            .or_else(|| customer.map(|c| c.name.clone()))
            .unwrap_or_default(),
        lines: address_lines(party.and_then(|p| p.address.as_deref())),
    };
    let seller = InvoiceParty {
        name: company.name.clone(),
        lines: address_lines(
            company
                .profile
                .iter()
                .find(|(k, _)| k == "address")
                .map(|(_, v)| v.as_str()),
        ),
    };
    // Each payment that stands, oldest first (INVARIANT 3): a payment
    // reversed later, and its reversal, print nothing.
    let mut settled = Vec::new();
    let mut last_paid = None;
    for l in ledger.doc_lines(doc) {
        if l.movement != ledger_core::Movement::Payment
            || l.reversal
            || l.date > as_of
            || !ledger.stands_on(l.entry, as_of)
        {
            continue;
        }
        let mut label = format!("Payment {}", printed(l.date));
        if let Some(i) = ledger
            .entry_instrument(l.entry)
            .and_then(|i| ledger.get_instrument(i))
        {
            label.push_str(&format!(" · {}", i.label));
        }
        if let Some(r) = ledger
            .get_entry(l.entry)
            .and_then(|e| e.description().rsplit_once(" (ref "))
            .and_then(|(_, r)| r.strip_suffix(')'))
        {
            label.push_str(&format!(" · ref {r}"));
        }
        settled.push((label, -l.amount));
        last_paid = Some(l.date);
    }
    for (label, amount) in [
        ("Discounts", st.discounted),
        ("Written off", st.written_off),
    ] {
        if !amount.is_zero() {
            settled.push((label.to_string(), amount));
        }
    }
    let (stamp, stamp_line) = match st.status {
        DocStatus::Void => (Some(InvoiceStamp::Void), None),
        DocStatus::Paid => (Some(InvoiceStamp::Paid), last_paid.map(printed)),
        _ if st.overdue_days > 0 => (Some(InvoiceStamp::PastDue), None),
        DocStatus::Partial => (Some(InvoiceStamp::PartiallyPaid), None),
        _ => (None, None),
    };
    Ok(InvoiceDoc {
        number: format!("Invoice {id}"),
        issue_date: printed(inv.date),
        due_date: printed(inv.due_date),
        terms: None,
        seller,
        bill_to,
        items: inv
            .lines
            .iter()
            .map(|l| InvoiceItem {
                description: l.description.clone(),
                amount: l.amount,
            })
            .collect(),
        total: st.total,
        settled,
        balance: st.outstanding,
        stamp,
        stamp_line,
        note: Some(format!(
            "Re-created on {} from the books; not a copy of the one sent.",
            printed(today)
        )),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ledger_core::{AccountType, Currency, DocumentLine};
    use rust_decimal::Decimal;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn an_invoice_prints_its_figures_as_of_a_day() {
        let mut l = Ledger::new(Currency::USD);
        let cash = l
            .add_account("1000", "Checking", AccountType::Asset, None, false)
            .unwrap();
        let c = l.add_customer("Harbor Client").unwrap();
        let inv = l
            .create_invoice(
                c,
                d(2026, 2, 1),
                d(2026, 3, 3),
                vec![DocumentLine {
                    description: "Consulting".into(),
                    amount: Decimal::new(50_000, 2),
                    class_id: None,
                    account: None,
                    quantity: None,
                    unit_price: None,
                }],
            )
            .unwrap();
        l.receive_payment(inv, d(2026, 2, 10), Decimal::new(50_000, 2), cash)
            .unwrap();
        let mut company = Company {
            name: "Fernhill Labs LLC".into(),
            ..Company::default()
        };
        company
            .profile
            .push(("address".into(), "12 Orchard Lane\nSpringfield, UT".into()));
        let before = invoice_document(&l, &company, inv, d(2026, 2, 5), d(2026, 3, 1)).unwrap();
        assert_eq!(before.balance, Decimal::new(50_000, 2));
        assert!(before.settled.is_empty() && before.stamp.is_none());
        let after = invoice_document(&l, &company, inv, d(2026, 2, 28), d(2026, 3, 1)).unwrap();
        assert_eq!(after.balance, Decimal::ZERO);
        assert_eq!(
            after.settled,
            vec![("Payment Feb 10, 2026".to_string(), Decimal::new(50_000, 2))]
        );
        assert_eq!(after.stamp, Some(InvoiceStamp::Paid));
        assert_eq!(after.stamp_line.as_deref(), Some("Feb 10, 2026"));
        assert_eq!(after.seller.lines, ["12 Orchard Lane", "Springfield, UT"]);
        assert_eq!(after.bill_to.name, "Harbor Client");
        assert_eq!(after.issue_date, "Feb 1, 2026");
        assert!(after.note.as_deref().unwrap().contains("Mar 1, 2026"));
        assert!(ledger_import::invoice_pdf(&after).is_ok());
    }

    /// The sample invoice, through the books, in three states (part paid
    /// and past due, paid in full, void): placeholder company and customer
    /// with full addresses, several lines, payments with their method and
    /// reference. SHOT_DIR set: each PDF is saved there, to look at.
    #[test]
    fn the_sample_invoice_in_each_state() {
        let mut l = Ledger::new(Currency::USD);
        let cash = l
            .add_account("1000", "Checking", AccountType::Asset, None, false)
            .unwrap();
        let bank = l
            .add_instrument(
                ledger_core::InstrumentKind::Bank,
                "ACH",
                Some("4821"),
                None,
                cash,
            )
            .unwrap();
        let c = l.add_customer("Harbor Coffee Roasters").unwrap();
        let party = l.party_of_customer(c).unwrap();
        l.update_party(
            party,
            ledger_core::PartyField::DisplayName(Some("Harbor Coffee Roasters, Inc.".into())),
        )
        .unwrap();
        l.update_party(
            party,
            ledger_core::PartyField::Address(Some(
                "Attn: Accounts Payable\n455 Wharf Street, Suite 210\nPortland, OR 97209".into(),
            )),
        )
        .unwrap();
        let line = |d: &str, cents: i64| DocumentLine {
            description: d.into(),
            amount: Decimal::new(cents, 2),
            class_id: None,
            account: None,
            quantity: None,
            unit_price: None,
        };
        let inv = l
            .create_invoice(
                c,
                d(2026, 9, 1),
                d(2026, 10, 1),
                vec![
                    line(
                        "iOS app: ordering flow, design and build (40 hours at $125)",
                        500_000,
                    ),
                    line(
                        "Android app: port of the ordering flow (24 hours at $125)",
                        300_000,
                    ),
                    line(
                        "App Store and Google Play submission and review support",
                        45_000,
                    ),
                    line("Hosting and push notifications, September", 8_900),
                ],
            )
            .unwrap();
        let pay = |l: &mut Ledger, day: u32, cents: i64, r: &str| {
            l.receive_payment_with(
                inv,
                d(2026, 10, day),
                Decimal::new(cents, 2),
                cash,
                &ledger_core::PaymentDetails {
                    instrument: Some(bank),
                    reference: Some(r.into()),
                },
            )
            .unwrap()
        };
        pay(&mut l, 6, 400_000, "tr_8812");
        let mut company = Company {
            name: "Fernhill Labs LLC".into(),
            ..Company::default()
        };
        company.profile.push((
            "address".into(),
            "12 Orchard Lane, Suite 4\nSpringfield, UT 84000\nhello@fernhill.example".into(),
        ));
        let save = |name: &str, doc: &ledger_import::InvoiceDoc| {
            let pdf = ledger_import::invoice_pdf(doc).unwrap();
            if let Ok(dir) = std::env::var("SHOT_DIR") {
                std::fs::create_dir_all(&dir).unwrap();
                std::fs::write(std::path::Path::new(&dir).join(name), pdf).unwrap();
            }
        };
        let today = d(2026, 10, 24);
        let part = invoice_document(&l, &company, inv, today, today).unwrap();
        assert_eq!(part.stamp, Some(InvoiceStamp::PastDue));
        assert_eq!(part.balance, Decimal::new(453_900, 2));
        assert_eq!(part.settled[0].0, "Payment Oct 6, 2026 · ACH · ref tr_8812");
        save("sample-part-paid.pdf", &part);
        pay(&mut l, 20, 453_900, "tr_9310");
        let paid = invoice_document(&l, &company, inv, today, today).unwrap();
        assert_eq!(paid.stamp, Some(InvoiceStamp::Paid));
        assert_eq!(paid.stamp_line.as_deref(), Some("Oct 20, 2026"));
        assert_eq!(paid.balance, Decimal::ZERO);
        save("sample-paid.pdf", &paid);
        let spare = l
            .create_invoice(
                c,
                d(2026, 10, 2),
                d(2026, 11, 1),
                vec![line("Entered twice", 8_900)],
            )
            .unwrap();
        l.void_document(DocRef::Invoice(spare), d(2026, 10, 2), "entered twice")
            .unwrap();
        let void = invoice_document(&l, &company, spare, today, today).unwrap();
        assert_eq!(void.stamp, Some(InvoiceStamp::Void));
        save("sample-void.pdf", &void);
    }
}
