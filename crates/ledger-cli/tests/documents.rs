//! P4b S5 over the wire: `books.mark_sent` records who sent an invoice,
//! how, to whom and when, and the read tools show it (`books.documents`'
//! `last_sent`, `books.document`'s `sends`). Expected behavior comes only
//! from the tool descriptions and parameter/result doc comments in
//! `src/types.rs` (the declared interface, as `tools/list` serves it) and
//! `docs/mcp-interface.md` ("Tools — receipts & documents"); plan
//! `served.md` P4b S5 and A-4 (the books record sends; they never send).

mod common;

use common::answer_of;
use serde_json::json;

/// Two sends on one invoice: the first the original (defaults: the
/// token's name, email, today's company day), the second a reminder with
/// the channel's message id; then `books.documents` shows the last send
/// and `books.document` the whole record.
#[test]
fn mark_sent_records_who_and_how() {
    let books = common::Books::new("p4b-s5-mark-sent");
    let mut wire = books.session("p4b-s5");
    let customer = answer_of(&wire.call("books.add_customer", json!({"name": "Fernhill Labs"})))
        .unwrap()["id"]
        .as_u64()
        .unwrap();
    let invoice = answer_of(&wire.call(
        "books.create_invoice",
        json!({"party_id": customer, "date": "2026-03-01", "due_date": "2026-03-31",
               "lines": [{"description": "Design", "amount": "1200.00"}]}),
    ))
    .unwrap()["id"]
        .as_u64()
        .unwrap();

    let first = answer_of(&wire.call(
        "books.mark_sent",
        json!({"invoice": format!("invoice:{invoice}"), "on": "2026-03-05",
               "by": "Avery Quill", "recipients": ["billing@fernhill.example"]}),
    ))
    .unwrap();
    assert_eq!(first["invoice"], json!(format!("invoice:{invoice}")));
    assert_eq!(first["sends"][0]["on"], json!("2026-03-05"));
    assert_eq!(first["sends"][0]["by"], json!("Avery Quill"));
    assert_eq!(
        first["sends"][0]["via"],
        json!("email"),
        "the default channel"
    );
    assert_eq!(first["sends"][0]["to"], json!(["billing@fernhill.example"]));
    assert_eq!(
        first["sends"][0]["kind"],
        json!("original"),
        "the first send is the original"
    );

    // The second names no `by`, `via` or `kind`: this session's token
    // (the agent), email, and a reminder now that one send is recorded.
    let second = answer_of(&wire.call(
        "books.mark_sent",
        json!({"invoice": invoice.to_string(), "on": "2026-03-20",
               "recipients": ["billing@fernhill.example"], "reference": "msg-2",
               "note": "Second copy"}),
    ))
    .unwrap();
    let sends = second["sends"].as_array().unwrap();
    assert_eq!(sends.len(), 2, "{second}");
    assert_eq!(sends[1]["by"], json!("test"), "the token's name");
    assert_eq!(sends[1]["via"], json!("email"));
    assert_eq!(sends[1]["kind"], json!("reminder"));
    assert_eq!(sends[1]["reference"], json!("msg-2"));
    assert_eq!(sends[1]["note"], json!("Second copy"));

    let listed = answer_of(&wire.call("books.documents", json!({"kind": "invoices"}))).unwrap();
    assert_eq!(
        listed["documents"][0]["ref"],
        json!(format!("invoice:{invoice}"))
    );
    assert_eq!(
        listed["documents"][0]["last_sent"],
        json!("2026-03-20"),
        "the list shows the latest send"
    );
    let one = answer_of(&wire.call(
        "books.document",
        json!({"ref": format!("invoice:{invoice}"), "as_of": "2026-03-31"}),
    ))
    .unwrap();
    assert_eq!(one["sends"].as_array().unwrap().len(), 2);
    assert_eq!(
        one["outstanding"],
        json!("1200.00"),
        "a send changes no figure"
    );
}
