//! Paying on terms over the wire (plan P4b S6; rules.md B-42): one payment
//! settling several documents, the early-payment discount, the processor's
//! fee, the bills-due list, the agings' tie-out and the exports. Every
//! call goes through the books server as an agent reaches it (common
//! INVARIANT 1).

mod common;

use common::{Books, Wire, answer_of};
use serde_json::{Value, json};

/// Call a tool and unwrap its answer.
fn ok(w: &mut Wire, tool: &str, args: Value) -> Value {
    answer_of(&w.call(tool, args)).unwrap_or_else(|e| panic!("{tool}: {e}"))
}

/// The refusal's text.
fn err(w: &mut Wire, tool: &str, args: Value) -> String {
    answer_of(&w.call(tool, args)).expect_err(&format!("{tool} was accepted"))
}

struct Setup {
    _books: Books,
    wire: Wire,
    customer: u64,
    vendor: u64,
}

/// Bank 1000, card 2050, capital 3000, a customer and a vendor, then a
/// session.
fn setup(tag: &str) -> Setup {
    let books = Books::new(tag);
    let mut wire = books.session("documents");
    ok(
        &mut wire,
        "books.add_account",
        json!({"code": "1000", "name": "Bank", "account_type": "asset"}),
    );
    ok(
        &mut wire,
        "books.add_account",
        json!({"code": "2050", "name": "Card", "account_type": "liability"}),
    );
    ok(
        &mut wire,
        "books.add_account",
        json!({"code": "3000", "name": "Capital", "account_type": "equity"}),
    );
    let customer = ok(
        &mut wire,
        "books.add_customer",
        json!({"name": "Fernhill Labs"}),
    )["id"]
        .as_u64()
        .unwrap();
    let vendor = ok(
        &mut wire,
        "books.add_vendor",
        json!({"name": "Tobin Harlow Supply"}),
    )["id"]
        .as_u64()
        .unwrap();
    Setup {
        _books: books,
        wire,
        customer,
        vendor,
    }
}

impl Setup {
    fn invoice(&mut self, date: &str, due: &str, amount: &str) -> u64 {
        ok(
            &mut self.wire,
            "books.create_invoice",
            json!({"party_id": self.customer, "date": date, "due_date": due,
                   "lines": [{"description": "Consulting", "amount": amount}]}),
        )["id"]
            .as_u64()
            .unwrap()
    }

    fn bill(&mut self, date: &str, due: &str, amount: &str) -> u64 {
        ok(
            &mut self.wire,
            "books.create_bill",
            json!({"party_id": self.vendor, "date": date, "due_date": due,
                   "lines": [{"description": "Paper", "amount": amount}]}),
        )["id"]
            .as_u64()
            .unwrap()
    }

    fn line(&mut self, entry: u64, code: &str) -> (String, String, String) {
        let e = ok(
            &mut self.wire,
            "books.get_entry",
            json!({"entry_id": entry}),
        );
        for l in e["lines"].as_array().unwrap() {
            if l["code"] == json!(code) {
                return (
                    l["side"].as_str().unwrap().to_string(),
                    l["amount"].as_str().unwrap().to_string(),
                    code.to_string(),
                );
            }
        }
        panic!("{code} not in entry {entry}: {e}");
    }
}

/// One deposit settles two invoices of one customer: one entry, one
/// receivable line per invoice, each tagged with its own invoice, and the
/// output names both documents (B-42 INVARIANTS 1 and 3).
#[test]
fn one_payment_settles_two_invoices() {
    let mut s = setup("settle-two");
    let first = s.invoice("2026-02-01", "2026-03-01", "500.00");
    let second = s.invoice("2026-02-05", "2026-04-30", "80.00");
    let out = ok(
        &mut s.wire,
        "books.receive_payment",
        json!({"date": "2026-02-10", "deposit_to": "1000",
        "reference": "one deposit",
        "applications": [
            {"invoice": first, "amount": "300.00"},
            {"invoice": second, "amount": "80.00"}
        ]}),
    );
    assert_eq!(out["deposited"], json!("380.00"));
    let refs: Vec<&str> = out["documents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["ref"].as_str().unwrap())
        .collect();
    assert_eq!(
        refs,
        [format!("invoice:{first}"), format!("invoice:{second}")]
    );
    assert_eq!(out["documents"][0]["outstanding"], json!("200.00"));
    assert_eq!(out["documents"][1]["status"], json!("paid"));
    // The entry: one deposit line, two receivable lines.
    let entry = out["entry_id"].as_u64().unwrap();
    let e = ok(&mut s.wire, "books.get_entry", json!({"entry_id": entry}));
    let lines = e["lines"].as_array().unwrap();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0]["side"], json!("credit"));
    assert_eq!(lines[0]["amount"], json!("300.00"));
    assert_eq!(lines[1]["side"], json!("credit"));
    assert_eq!(lines[1]["amount"], json!("80.00"));
    // Each invoice's timeline shows its own payment from this entry.
    for (id, paid) in [(first, "300.00"), (second, "80.00")] {
        let d = ok(
            &mut s.wire,
            "books.document",
            json!({"ref": format!("invoice:{id}"), "as_of": "2026-12-31"}),
        );
        let payment = d["timeline"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["kind"] == json!("payment"))
            .unwrap();
        assert_eq!(payment["entry_id"].as_u64().unwrap(), entry);
        assert_eq!(payment["amount"], json!(format!("-{paid}")));
    }
}

/// A discount outside the document's terms needs a reason; with one the
/// payment is taken and recorded (B-42 INVARIANT 2; the terms are S4.3's
/// to journal, so this bill states none).
#[test]
fn late_discount_needs_a_reason() {
    let mut s = setup("late-discount");
    let bill = s.bill("2026-02-01", "2026-03-03", "100.00");
    let applications = |reason: Option<&str>| {
        let mut a = json!([{"bill": bill, "amount": "98.00", "discount": "2.00"}]);
        if let Some(reason) = reason {
            a[0]["discount_reason"] = json!(reason);
        }
        a
    };
    let why = err(
        &mut s.wire,
        "books.pay_bill",
        json!({"date": "2026-02-05", "paid_from": "1000", "applications": applications(None)}),
    );
    assert!(why.contains("needs a reason"), "{why}");
    assert!(why.contains("discount_reason"), "{why}");
    ok(
        &mut s.wire,
        "books.pay_bill",
        json!({"date": "2026-02-05", "paid_from": "1000",
               "applications": applications(Some("the vendor honoured it late"))}),
    );
}

/// A fee posts gross (A-4): the deposit gets the net, the fee its expense,
/// and A/R is credited the gross (B-42 INVARIANT 1).
#[test]
fn a_fee_posts_gross_to_receivables_and_net_to_the_bank() {
    let mut s = setup("fee-gross");
    let invoice = s.invoice("2026-02-01", "2026-03-01", "100.00");
    let out = ok(
        &mut s.wire,
        "books.receive_payment",
        json!({"date": "2026-02-05", "deposit_to": "1000",
               "invoice": invoice, "amount": "100.00",
               "fee": {"amount": "3.00"}}),
    );
    assert_eq!(out["deposited"], json!("97.00"));
    assert_eq!(out["status"], json!("paid"));
    let entry = out["entry_id"].as_u64().unwrap();
    assert_eq!(
        s.line(entry, "1200"),
        ("credit".into(), "100.00".into(), "1200".into())
    );
    assert_eq!(
        s.line(entry, "6060"),
        ("debit".into(), "3.00".into(), "6060".into())
    );
    assert_eq!(
        s.line(entry, "1000"),
        ("debit".into(), "97.00".into(), "1000".into())
    );
}

/// A bill paid by card is excluded from 1099-NEC (D-40): the worksheet
/// counts it as card, not as paid.
#[test]
fn card_paid_bill_is_not_1099_nec() {
    let mut s = setup("card-1099");
    ok(
        &mut s.wire,
        "books.add_instrument",
        json!({"kind": "business_card", "label": "Visa", "last4": "4242", "account": "2050"}),
    );
    let bill = s.bill("2026-02-01", "2026-03-03", "100.00");
    ok(
        &mut s.wire,
        "books.pay_bill",
        json!({"date": "2026-02-05", "paid_from": "2050", "bill": bill, "amount": "100.00",
               "instrument": "Visa"}),
    );
    let worksheet = ok(
        &mut s.wire,
        "books.vendor_1099",
        json!({"year": 2026, "cash_accounts": ["1000"]}),
    );
    let line = worksheet["vendors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["vendor_id"] == json!(s.vendor))
        .unwrap();
    assert_eq!(line["paid"], json!("0.00"), "{worksheet}");
    assert_eq!(line["excluded_card"], json!("100.00"), "{worksheet}");
}

/// Two bills paid in one entry count once toward 1099s and the party's
/// totals (B-42; 8.3.16's multi-document risk).
#[test]
fn multi_document_payment_counts_once_for_1099_and_party_totals() {
    let mut s = setup("multi-1099");
    let first = s.bill("2026-02-01", "2026-03-03", "100.00");
    let second = s.bill("2026-02-02", "2026-03-04", "200.00");
    ok(
        &mut s.wire,
        "books.pay_bill",
        json!({"date": "2026-02-05", "paid_from": "1000",
        "applications": [
            {"bill": first, "amount": "100.00"},
            {"bill": second, "amount": "200.00"}
        ]}),
    );
    let worksheet = ok(
        &mut s.wire,
        "books.vendor_1099",
        json!({"year": 2026, "cash_accounts": ["1000"]}),
    );
    let line = worksheet["vendors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["vendor_id"] == json!(s.vendor))
        .unwrap();
    assert_eq!(line["paid"], json!("300.00"), "{worksheet}");
    let parties = ok(&mut s.wire, "books.parties", json!({"year": 2026}));
    let party = parties["parties"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == json!("Tobin Harlow Supply"))
        .unwrap();
    assert_eq!(party["paid"], json!("300.00"), "{parties}");
}

/// An overpayment is refused showing what the document owes and the
/// remedy for the excess (A-12, D-49).
#[test]
fn overpayment_refused_with_the_excess() {
    let mut s = setup("overpayment");
    let invoice = s.invoice("2026-02-01", "2026-03-01", "100.00");
    let why = err(
        &mut s.wire,
        "books.receive_payment",
        json!({"date": "2026-02-05", "deposit_to": "1000", "invoice": invoice, "amount": "150.00"}),
    );
    assert!(why.contains("150.00"), "{why}");
    assert!(why.contains("100.00"), "{why}");
    assert!(why.contains("excess"), "{why}");
    assert!(why.contains("2300"), "{why}");
}

/// The bills-due list orders by due date and gives the running total
/// through each date; the agings' detail rows and their tie-out agree with
/// the control check (8.3.10; B-42).
#[test]
fn bills_due_orders_by_due_and_discount() {
    let mut s = setup("bills-due");
    s.bill("2026-02-01", "2026-02-20", "100.00");
    s.bill("2026-02-02", "2026-02-10", "250.00");
    s.bill("2026-02-03", "2026-03-10", "400.00");
    let due = ok(
        &mut s.wire,
        "books.bills_due",
        json!({"through": "2026-02-28", "as_of": "2026-02-05"}),
    );
    let dates: Vec<&str> = due["bills"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["due_date"].as_str().unwrap())
        .collect();
    assert_eq!(dates, ["2026-02-10", "2026-02-20"]);
    assert_eq!(
        due["by_date"],
        json!([
            {"date": "2026-02-10", "amount": "250.00"},
            {"date": "2026-02-20", "amount": "350.00"}
        ])
    );
    // The discount fields exist only when the bill's terms give one, which
    // S4.3 journals: without them they are absent, not zero.
    assert!(due["bills"][0].get("discount_by").is_none());
    let aging = ok(
        &mut s.wire,
        "books.ap_aging",
        json!({"as_of": "2026-02-05", "detail": true}),
    );
    assert_eq!(aging["control"]["difference"], json!("0.00"), "{aging}");
    assert_eq!(aging["control"]["documents_total"], json!("750.00"));
    let rows = aging["documents"].as_array().unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0]["ref"], json!(format!("bill:{}", 2)));
    assert_eq!(rows[0]["bucket"], json!("current"));
    // A/R is empty and still ties.
    let ar = ok(
        &mut s.wire,
        "books.ar_aging",
        json!({"as_of": "2026-02-05", "detail": true}),
    );
    assert_eq!(ar["control"]["balance"], json!("0.00"));
    assert_eq!(ar["control"]["difference"], json!("0.00"));
    assert!(ar.get("documents").is_none());
}

/// The aging exports are the same CSV the CPA package carries: one row per
/// document and a tie-out line, and the CPA package's CSVs tie to the
/// control account (8.3.10; ledger-package's test repeats the tie-out).
#[test]
fn the_aging_export_ties_out() {
    let mut s = setup("aging-export");
    let invoice = s.invoice("2026-02-01", "2026-03-01", "500.00");
    ok(
        &mut s.wire,
        "books.receive_payment",
        json!({"date": "2026-02-05", "deposit_to": "1000", "invoice": invoice, "amount": "200.00"}),
    );
    let out = ok(
        &mut s.wire,
        "books.export",
        json!({"format": "csv", "report": "ar_aging", "as_of": "2026-02-28"}),
    );
    let csv = s.wire.fetch_text(&out["file"]);
    let mut lines = csv.lines();
    assert_eq!(
        lines.next(),
        Some(
            "number,party,date,due_date,terms,total,paid,discounted,outstanding,bucket,days_overdue"
        )
    );
    assert!(lines.next().unwrap().contains("500.00"));
    let total = lines.next().unwrap();
    assert!(total.starts_with("TOTAL,,,,,,,,"), "{total}");
    assert!(total.contains("300.00"), "{total}");
    assert!(total.contains("GL 1200 300.00"), "{total}");
    assert!(total.ends_with("difference 0.00"), "{total}");
}
