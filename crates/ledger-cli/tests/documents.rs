//! Documents over the wire (plan P4b S4 to S7; this file starts with S7's
//! bad debts). One test per behavior the tools promise, through the
//! books server exactly as an agent reaches it: `common::Books` on a data
//! root of the test's own, a session, and `books.*` calls.
//!
//! ```text
//! GOAL: a bad debt is written off with its evidence in one call, reads
//!       written off, and a same-year recovery is the reversal D-49
//!       records; a write-off with no evidence is refused.
//! SOURCE: docs/spec/rules.md B-41; plan P4b S7; IRC §166(a), §111;
//!         Treas. Reg. §1.166-1(e), -2(a), -3(a).
//! ```

mod common;

use serde_json::{Value, json};

/// A call's structured result, or a panic naming the tool and the refusal.
fn ok(wire: &mut common::Wire, tool: &str, args: Value) -> Value {
    common::answer_of(&wire.call(tool, args)).unwrap_or_else(|e| panic!("{tool}: {e}"))
}

/// A call's refusal, or a panic naming the tool.
fn refused(wire: &mut common::Wire, tool: &str, args: Value) -> String {
    match common::answer_of(&wire.call(tool, args)) {
        Err(e) => e,
        Ok(v) => panic!("{tool} accepted: {v}"),
    }
}

/// The books of every test here: a bank account 1000, a customer, and one
/// invoice for 100.00 dated 2026-01-05, due 2026-02-04.
fn invoice(wire: &mut common::Wire) -> u64 {
    ok(
        wire,
        "books.add_account",
        json!({
            "code": "1000",
            "name": "Checking",
            "account_type": "asset",
            "parent": null,
            "placeholder": null,
        }),
    );
    let customer = ok(
        wire,
        "books.add_customer",
        json!({ "name": "Fernhill Labs" }),
    )["id"]
        .as_u64()
        .unwrap();
    ok(
        wire,
        "books.create_invoice",
        json!({
            "party_id": customer,
            "date": "2026-01-05",
            "due_date": "2026-02-04",
            "lines": [{ "description": "Design", "amount": "100.00" }],
        }),
    )["id"]
        .as_u64()
        .unwrap()
}

/// A collection letter, the evidence of worthlessness.
const LETTER: &[u8] = b"%PDF-1.4\nFinal demand, unpaid\n%%EOF\n";

/// P4b S7: an invoice half paid is written off for the rest with its
/// evidence in the same call, reads written off, and the same-year
/// recovery D-49 records (reverse the write-off, then a payment) reopens
/// and settles it.
#[test]
fn write_off_then_recovery() {
    let books = common::Books::new("s7-recovery");
    let mut wire = books.wire();
    wire.initialize("s7-recovery");
    let inv = invoice(&mut wire);
    ok(
        &mut wire,
        "books.receive_payment",
        json!({
            "document_id": inv,
            "date": "2026-01-20",
            "amount": "40.00",
            "cash_account": "1000",
        }),
    );
    // The write-off's evidence, uploaded as an agent uploads it (D-46).
    let upload = wire.upload(LETTER, "collection-letter.pdf", "application/pdf", "document");
    let out = ok(
        &mut wire,
        "books.write_off_invoice",
        json!({
            "invoice": inv,
            "date": "2026-01-31",
            "reason": "collection exhausted",
            "upload_id": upload,
            "confirmed": true,
        }),
    );
    let entry = out["entry_id"].as_u64().unwrap();
    assert!(entry > 0, "{out}");
    assert_eq!(out["document"]["ref"], json!(format!("invoice:{inv}")));
    assert_eq!(out["document"]["status"], json!("written_off"));
    assert_eq!(out["document"]["paid"], json!("40.00"));
    assert_eq!(out["document"]["written_off"], json!("60.00"));
    assert_eq!(out["document"]["outstanding"], json!("0.00"));
    // The evidence is on the write-off's entry, as a receipt, and on the
    // invoice: get_entry shows the one, the document's timeline the other.
    let got = ok(&mut wire, "books.get_entry", json!({ "entry_id": entry }));
    assert_eq!(got["receipts"].as_array().map(Vec::len), Some(1), "{got}");
    assert_eq!(got["receipts"][0]["file_name"], json!("collection-letter.pdf"));
    let doc = ok(
        &mut wire,
        "books.document",
        json!({ "ref": format!("invoice:{inv}"), "as_of": "2026-02-01" }),
    );
    assert_eq!(doc["status"], json!("written_off"));
    assert_eq!(doc["written_off"], json!("60.00"));
    let write_off_lines: Vec<&Value> = doc["timeline"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|t| t["kind"] == json!("write_off"))
        .collect();
    assert_eq!(write_off_lines.len(), 1, "{doc}");
    // The same tax year: reverse the write-off; the invoice reopens.
    ok(
        &mut wire,
        "books.reverse_entry",
        json!({
            "entry_id": entry,
            "date": "2026-03-01",
            "description": "Paid after all",
            "confirmed": true,
        }),
    );
    let doc = ok(
        &mut wire,
        "books.document",
        json!({ "ref": format!("invoice:{inv}"), "as_of": "2026-03-02" }),
    );
    assert_eq!(doc["status"], json!("partial"));
    assert_eq!(doc["outstanding"], json!("60.00"));
    ok(
        &mut wire,
        "books.receive_payment",
        json!({
            "document_id": inv,
            "date": "2026-03-02",
            "amount": "60.00",
            "cash_account": "1000",
        }),
    );
    let doc = ok(
        &mut wire,
        "books.document",
        json!({ "ref": format!("invoice:{inv}"), "as_of": "2026-03-03" }),
    );
    assert_eq!(doc["status"], json!("paid"));
    assert_eq!(doc["outstanding"], json!("0.00"));
    // The control accounts still equal their documents.
    let check = ok(
        &mut wire,
        "books.control_check",
        json!({ "as_of": "2026-03-31" }),
    );
    assert_eq!(check["agrees"], json!(true), "{check}");
}

/// P4b S7: no evidence, no write-off (B-41 INVARIANT 1): the tool refuses
/// with neither or both, and an attachment already filed is linked
/// instead of a new file.
#[test]
fn write_off_needs_evidence() {
    let books = common::Books::new("s7-evidence");
    let mut wire = books.wire();
    wire.initialize("s7-evidence");
    let inv = invoice(&mut wire);
    let args = |extra: Value| {
        let mut v = json!({
            "invoice": inv,
            "date": "2026-01-31",
            "reason": "uncollectible",
            "confirmed": true,
        });
        v.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        v
    };
    let why = refused(&mut wire, "books.write_off_invoice", args(json!({})));
    assert!(why.contains("upload_id"), "{why}");
    assert!(why.contains("existing_attachment_id"), "{why}");
    let why = refused(
        &mut wire,
        "books.write_off_invoice",
        args(json!({ "upload_id": "up_nope", "existing_attachment_id": 1 })),
    );
    assert!(why.contains("exactly one"), "{why}");
    // A filed attachment (not linked to an entry) is linked to the
    // write-off's entry and the invoice.
    let upload = wire.upload(LETTER, "letter.pdf", "application/pdf", "document");
    let filed = ok(
        &mut wire,
        "books.file_document",
        json!({
            "category": "other",
            "upload_id": upload,
            "description": "Collection attempts",
        }),
    );
    let attachment = filed["attachment_id"].as_u64().unwrap();
    let out = ok(
        &mut wire,
        "books.write_off_invoice",
        args(json!({ "existing_attachment_id": attachment })),
    );
    assert_eq!(out["document"]["status"], json!("written_off"));
    assert_eq!(out["document"]["written_off"], json!("100.00"));
    let receipt = ok(
        &mut wire,
        "books.get_receipt",
        json!({ "attachment_id": attachment }),
    );
    let links = receipt["links"].as_array().cloned().unwrap_or_default();
    assert!(links.iter().any(|l| l["kind"] == json!("invoice")), "{receipt}");
    assert!(links.iter().any(|l| l["kind"] == json!("entry")), "{receipt}");
}
