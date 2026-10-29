//! Invoices and bills through the books server, as an agent reaches them
//! (common.rs): the preview that writes nothing, the confirm that posts,
//! numbers, terms, line accounts, party snapshots and the issued copy filed
//! with the invoice (plan/served.md 8.2 P4b S4 item 17; docs/mcp-interface.md
//! "Tools — A/R and A/P"; rules.md B-39, B-40; D-55).
//!
//! Every refusal is checked to leave the journal byte for byte as it was.

use common::{Books, Wire};
use serde_json::{Value, json};

mod common;

/// One agent's session with the test's books, over the books server's
/// socket (common.rs).
struct Mcp {
    wire: Wire,
    next_id: u64,
}

impl Mcp {
    /// A new, initialized session with `books`.
    fn spawn(books: &Books) -> Self {
        Self {
            wire: books.session("documents"),
            next_id: 1,
        }
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let msg = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        self.wire.send(&msg);
        let resp = self
            .wire
            .recv(common::TIMEOUT)
            .unwrap_or_else(|| panic!("no answer to {method}: {:?}", self.wire.refused));
        assert_eq!(resp["id"], id, "id mismatch in {resp}");
        resp
    }

    /// Call a tool; panic on a tool error. Returns structuredContent.
    fn call(&mut self, tool: &str, args: Value) -> Value {
        match self.try_call(tool, args) {
            Ok(v) => v,
            Err(e) => panic!("{tool}: {e}"),
        }
    }

    /// Call a tool: its result, or the refusal's text.
    fn try_call(&mut self, tool: &str, args: Value) -> Result<Value, String> {
        let resp = self.request("tools/call", json!({"name": tool, "arguments": args}));
        let result = &resp["result"];
        if result.get("isError") == Some(&json!(true)) {
            let text = result["content"]
                .as_array()
                .and_then(|c| c.iter().find_map(|b| b["text"].as_str()))
                .unwrap_or_default()
                .to_string();
            return Err(text);
        }
        Ok(result.get("structuredContent").cloned().unwrap_or_default())
    }

    /// The bytes of a file a tool handed over (a `HandedFile`).
    fn handed(&mut self, file: &Value) -> Vec<u8> {
        assert!(file["expires_at"].is_string(), "{file}");
        self.wire.fetch(file)
    }

    fn upload(&mut self, bytes: &[u8], file_name: &str, purpose: &str) -> String {
        self.wire
            .upload(bytes, file_name, "application/pdf", purpose)
    }

    /// A customer with `name`: its id.
    fn customer(&mut self, name: &str) -> u64 {
        self.call("books.add_customer", json!({"name": name}))["id"]
            .as_u64()
            .unwrap()
    }

    /// A vendor with `name`: its id.
    fn vendor(&mut self, name: &str) -> u64 {
        self.call("books.add_vendor", json!({"name": name}))["id"]
            .as_u64()
            .unwrap()
    }

    /// An expense account, so a bill's line can name one (A-10).
    fn expense(&mut self, code: &str, name: &str) {
        self.call(
            "books.add_account",
            json!({"code": code, "name": name, "account_type": "expense"}),
        );
    }

    /// The journal's `kind` events, oldest first, as `{type, payload}`.
    fn events(books: &Books, kind: &str) -> Vec<Value> {
        String::from_utf8(books.journal())
            .unwrap()
            .lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .filter(|v| v["type"] == json!(kind))
            .collect()
    }
}

/// Every invoice and bill figure and the copies they keep, from the
/// preview that writes nothing to the confirm that files the issued copy.
#[test]
fn an_invoice_from_issue_to_paid() {
    let books = Books::new("documents-issue");
    let mut m = Mcp::spawn(&books);
    let harbor = m.customer("Harbor Coffee");
    let issue = json!({
        "customer": harbor.to_string(),
        "date": "2026-02-01",
        "terms": "Net 30",
        "lines": [{"description": "Design", "amount": "500.00"}],
    });

    // The preview: nothing written, a DRAFT copy handed over, the number
    // and due date the confirm will take.
    let before = books.journal();
    let preview = m.call("books.create_invoice", issue.clone());
    assert_eq!(preview["preview"], json!(true), "{preview}");
    assert_eq!(preview["number"], "INV-0001", "{preview}");
    assert_eq!(preview["due_date"], "2026-03-03", "{preview}");
    assert_eq!(preview["terms"], "Net 30", "{preview}");
    assert_eq!(preview["total"], "500.00", "{preview}");
    let posting: Vec<(String, String, String)> = preview["lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| {
            (
                l["account"].as_str().unwrap().to_string(),
                l["side"].as_str().unwrap().to_string(),
                l["amount"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert_eq!(
        posting,
        [
            ("1200".into(), "debit".into(), "500.00".into()),
            ("4000".into(), "credit".into(), "500.00".into()),
        ]
    );
    let draft = m.handed(&preview["pdf"]);
    assert!(draft.starts_with(b"%PDF"), "a PDF, not bytes inline");
    assert!(draft.len() > 1_000, "the DRAFT copy is a page: {preview}");
    assert_eq!(books.journal(), before, "a preview journals nothing");
    assert!(books.journal().len() == before.len());

    // The confirm: the same number, the issued copy filed with the invoice.
    let mut confirmed = issue.clone();
    confirmed["confirmed"] = json!(true);
    let issued = m.call("books.create_invoice", confirmed);
    assert_eq!(issued["number"], "INV-0001", "{issued}");
    assert_eq!(issued["due_date"], "2026-03-03", "{issued}");
    assert_eq!(issued["total"], "500.00", "{issued}");
    let entry = issued["entry_id"].as_u64().unwrap();
    let attachment = issued["pdf"]["attachment_id"].as_u64().unwrap();
    let sha = issued["pdf"]["sha256"].as_str().unwrap().to_string();
    let receipt = m.call("books.get_receipt", json!({"attachment_id": attachment}));
    assert_eq!(receipt["sha256"], json!(sha), "the same SHA-256");
    assert_eq!(receipt["entry_id"], json!(entry), "filed on its entry");
    assert_eq!(receipt["file_name"], "INV-0001.pdf", "{receipt}");
    assert_eq!(receipt["mime_type"], "application/pdf", "{receipt}");
    assert!(
        receipt["links"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l["kind"] == json!("invoice") && l["id"] == issued["id"]),
        "linked to the invoice: {receipt}"
    );
    // The entry carries the issued copy as its evidence.
    let got = m.call("books.get_entry", json!({"entry_id": entry}));
    assert!(
        got["receipts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["attachment_id"] == json!(attachment)),
        "{got}"
    );
    // A convenience for a human sender; the books never send (A-4).
    let mailto = issued["mailto"].as_str().unwrap();
    assert!(mailto.starts_with("mailto:"), "{mailto}");
    assert!(
        mailto.contains("Invoice%20INV-0001%20from%20"),
        "the subject names the number and the company: {mailto}"
    );

    // The next one takes the next number; a preview between them takes
    // none.
    let second = m.call(
        "books.create_invoice",
        json!({"customer": "Harbor Coffee", "date": "2026-02-02", "terms": "Net 30",
               "lines": [{"description": "Build", "amount": "250.00"}], "confirmed": true}),
    );
    assert_eq!(second["number"], "INV-0002", "{second}");
    let third = m.call(
        "books.create_invoice",
        json!({"customer": harbor.to_string(), "date": "2026-02-03", "terms": "Net 30",
               "lines": [{"description": "Abandoned", "amount": "10.00"}],
               "confirmed": true}),
    );
    assert_eq!(third["number"], "INV-0003", "{third}");
    m.call("books.verify", json!({}));
}

/// A bill keeps the vendor's invoice, names its own accounts, and carries
/// its terms' discount dates (A-10, A-13; B-39).
#[test]
fn a_bill_with_its_vendor_invoice_and_terms() {
    let books = Books::new("documents-bill");
    let mut m = Mcp::spawn(&books);
    let coast = m.vendor("Coastline Hosting");
    m.expense("6500", "Hosting");
    let file = m.upload(
        b"%PDF-1.4 Coastline's own invoice",
        "coastline.pdf",
        "receipt",
    );
    let bill = json!({
        "vendor": coast.to_string(),
        "date": "2026-02-01",
        "terms": "2/10 net 30",
        "vendor_ref": "CH-4471",
        "lines": [{"description": "Hosting, February", "amount": "1000.00",
                   "account": "6500"}],
    });

    let before = books.journal();
    let mut preview = bill.clone();
    preview["upload_id"] = json!(file);
    let p = m.call("books.create_bill", preview.clone());
    assert_eq!(p["preview"], json!(true), "{p}");
    assert_eq!(p["due_date"], "2026-03-03", "{p}");
    assert_eq!(p["terms"], "2/10 net 30", "{p}");
    assert_eq!(p["discount_by"], "2026-02-11", "{p}");
    assert_eq!(p["max_discount"], "20.00", "2% of 1,000.00");
    assert_eq!(books.journal(), before, "a preview journals nothing");

    let mut confirm = preview;
    confirm["confirmed"] = json!(true);
    let made = m.call("books.create_bill", confirm);
    assert_eq!(made["terms"], "2/10 net 30", "{made}");
    assert_eq!(made["discount_by"], "2026-02-11", "{made}");
    let attachment = made["file"]["attachment_id"].as_u64().unwrap();
    let sha = made["file"]["sha256"].as_str().unwrap().to_string();
    assert_eq!(
        sha,
        ledger_store::journal::sha256_hex(b"%PDF-1.4 Coastline's own invoice"),
        "the vendor's own bytes"
    );
    let receipt = m.call("books.get_receipt", json!({"attachment_id": attachment}));
    assert_eq!(receipt["entry_id"], made["entry_id"], "{receipt}");
    assert_eq!(receipt["description"], "Vendor invoice", "{receipt}");
    assert!(
        receipt["links"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l["kind"] == json!("bill") && l["id"] == made["id"]),
        "linked to the bill: {receipt}"
    );
    // The posting: the expense the line names, the payable, tagged.
    let entry = m.call("books.get_entry", json!({"entry_id": made["entry_id"]}));
    let lines: Vec<(String, String, String)> = entry["lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| {
            (
                l["code"].as_str().unwrap().to_string(),
                l["side"].as_str().unwrap().to_string(),
                l["amount"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert_eq!(
        lines,
        [
            ("6500".into(), "debit".into(), "1000.00".into()),
            ("2000".into(), "credit".into(), "1000.00".into()),
        ]
    );
    // The vendor's number is the bill's, and the document names it.
    let doc = m.call(
        "books.document",
        json!({"ref": format!("bill:{}", made["id"])}),
    );
    assert_eq!(doc["total"], "1000.00", "{doc}");
    m.call("books.verify", json!({}));
}

/// One vendor's invoice number, entered twice: refused unless a reason is
/// given, which is journaled with the second bill (B-40).
#[test]
fn duplicate_vendor_invoice_refused() {
    let books = Books::new("documents-duplicate");
    let mut m = Mcp::spawn(&books);
    let coast = m.vendor("Coastline Hosting");
    m.expense("6500", "Hosting");
    let first_file = m.upload(b"first copy", "ch-1.pdf", "receipt");
    let bill = |file: &str, date: &str| {
        json!({"vendor": coast.to_string(), "date": date, "due_date": "2026-03-03",
               "vendor_ref": "CH-4471",
               "lines": [{"description": "Hosting", "amount": "100.00", "account": "6500"}],
               "upload_id": file, "confirmed": true})
    };
    m.call("books.create_bill", bill(&first_file, "2026-02-01"));
    // The same number (however written) is the same vendor invoice.
    let second_file = m.upload(b"second copy", "ch-1-again.pdf", "receipt");
    let before = books.journal();
    let refused = m
        .try_call("books.create_bill", bill(&second_file, "2026-02-02"))
        .unwrap_err();
    assert!(refused.contains("CH-4471"), "{refused}");
    assert!(refused.contains("duplicate_reason"), "{refused}");
    assert_eq!(books.journal(), before, "a refusal journals nothing");
    // The upload was given back, not used up (an upload is spent only by
    // the call that files it).
    let mut with_reason = bill(&second_file, "2026-02-02");
    with_reason["duplicate_reason"] =
        json!("the vendor sent it twice; this one has the credit line");
    let made = m.call("books.create_bill", with_reason);
    let created = Mcp::events(&books, "bill.created");
    let second = created.last().unwrap();
    assert_eq!(
        second["payload"]["duplicate_reason"],
        "the vendor sent it twice; this one has the credit line",
        "{second}"
    );
    assert_eq!(made["total"], "100.00");
}

/// A line's account obeys the document's kind: revenue or a role-free
/// liability on an invoice, an expense or a non-cash role-free asset on a
/// bill; a role account, cash, a placeholder or none at all is refused
/// (A-10, A-11; rules.md B-39).
#[test]
fn a_line_account_must_be_allowed() {
    let books = Books::new("documents-accounts");
    let mut m = Mcp::spawn(&books);
    let harbor = m.customer("Harbor Coffee");
    let coast = m.vendor("Coastline Hosting");
    for (code, name, kind) in [
        ("1000", "Checking", "asset"),
        ("2000", "Accounts Payable", "liability"),
        ("6999", "General Expenses", "expense"),
    ] {
        m.call(
            "books.add_account",
            json!({"code": code, "name": name, "account_type": kind}),
        );
    }
    m.call(
        "books.add_account",
        json!({"code": "2400", "name": "Deferred Revenue", "account_type": "liability"}),
    );
    m.call(
        "books.add_account",
        json!({"code": "1500", "name": "Equipment", "account_type": "asset"}),
    );
    let invoice = |account: &str| {
        json!({"customer": harbor.to_string(), "date": "2026-02-01", "terms": "Net 30",
               "lines": [{"description": "Work", "amount": "100.00", "account": account}],
               "confirmed": true})
    };
    let before = books.journal();
    // An invoice line may not post to the receivable control or a role
    // account.
    let why = m
        .try_call("books.create_invoice", invoice("1200"))
        .unwrap_err();
    assert!(why.contains("1200"), "{why}");
    assert_eq!(books.journal(), before, "a refusal journals nothing");
    // Nor to an expense account.
    let why = m
        .try_call("books.create_invoice", invoice("6999"))
        .unwrap_err();
    assert!(why.contains("6999"), "{why}");
    // A liability holding no role (deferred revenue) is allowed.
    let deferred = m.call("books.create_invoice", invoice("2400"));
    assert_eq!(deferred["total"], "100.00", "{deferred}");

    // A bill's line names its account in these books, and it must be an
    // expense or a non-cash asset holding no role.
    let file = m.upload(b"coastline", "coastline.pdf", "receipt");
    let bill = |account: &str| {
        json!({"vendor": coast.to_string(), "date": "2026-02-01", "due_date": "2026-03-03",
               "vendor_ref": "CH-1",
               "lines": [{"description": "Hosting", "amount": "100.00", "account": account}],
               "upload_id": file, "confirmed": true})
    };
    let why = m.try_call("books.create_bill", bill("1000")).unwrap_err();
    assert!(why.contains("1000") && why.contains("cash"), "{why}");
    let why = m.try_call("books.create_bill", bill("2000")).unwrap_err();
    assert!(why.contains("2000"), "{why}");
    // A line naming no account is refused in books with documents_v2.
    let mut unnamed = bill("1500");
    unnamed["lines"] = json!([{"description": "Hosting", "amount": "100.00"}]);
    let why = m.try_call("books.create_bill", unnamed).unwrap_err();
    assert!(why.contains("A-10"), "{why}");
    // Equipment is an asset that is not cash and holds no role.
    let equipment = m.call("books.create_bill", bill("1500"));
    assert_eq!(equipment["total"], "100.00", "{equipment}");
}

/// The due date falls back in one order: the date given, the terms given,
/// the party's default terms, the books' invoicing default (plan 8.2 S4.4
/// item 14; A-13).
#[test]
fn terms_fall_back_in_order() {
    let books = Books::new("documents-terms");
    let mut m = Mcp::spawn(&books);
    let harbor = m.customer("Harbor Coffee");
    let line = json!([{"description": "Work", "amount": "100.00"}]);
    let invoice = |extra: Value| {
        let mut args = json!({"customer": harbor.to_string(), "date": "2026-10-23",
                              "lines": line, "confirmed": true});
        for (k, v) in extra.as_object().unwrap() {
            args[k] = v.clone();
        }
        args
    };

    // The books' default, unset: Net 30.
    let plain = m.call("books.create_invoice", invoice(json!({})));
    assert_eq!(plain["due_date"], "2026-11-22", "{plain}");
    assert_eq!(plain["terms"], "Net 30", "{plain}");

    // The customer's own terms come before the books'.
    let party = m.party_of("Harbor Coffee");
    m.call(
        "books.update_party",
        json!({"party": party.to_string(), "default_terms_as_customer": "Net 15 EOM"}),
    );
    let by_party = m.call("books.create_invoice", invoice(json!({})));
    assert_eq!(by_party["due_date"], "2026-11-15", "{by_party}");
    assert_eq!(by_party["terms"], "Net 15 EOM", "{by_party}");

    // Terms given come before the customer's.
    let given = m.call("books.create_invoice", invoice(json!({"terms": "Net 45"})));
    assert_eq!(given["due_date"], "2026-12-07", "{given}");
    assert_eq!(given["terms"], "Net 45", "{given}");

    // A due date given wins over all of them, and prints no terms.
    let dated = m.call(
        "books.create_invoice",
        invoice(json!({"due_date": "2026-12-31"})),
    );
    assert_eq!(dated["due_date"], "2026-12-31", "{dated}");
    assert!(dated.get("terms").is_none(), "{dated}");

    // The books' setting, when the customer has no terms.
    m.call(
        "books.update_party",
        json!({"party": party.to_string(), "default_terms_as_customer": ""}),
    );
    m.call(
        "books.set_setting",
        json!({"decided_by": "Avery Quill",
               "changes": [{"key": "invoicing.default_terms", "value": "Net 60"}]}),
    );
    let setting = m.call("books.create_invoice", invoice(json!({})));
    assert_eq!(setting["due_date"], "2026-12-22", "{setting}");
    assert_eq!(setting["terms"], "Net 60", "{setting}");

    // A bill falls back to its vendor's terms, and no further.
    let coast = m.vendor("Coastline Hosting");
    m.expense("6500", "Hosting");
    let coast_party = m.party_of("Coastline Hosting");
    m.call(
        "books.update_party",
        json!({"party": coast_party.to_string(), "default_terms_as_vendor": "Net 10"}),
    );
    let file = m.upload(b"coastline", "coastline.pdf", "receipt");
    let bill = m.call(
        "books.create_bill",
        json!({"vendor": coast.to_string(), "date": "2026-10-23", "vendor_ref": "CH-9",
               "lines": [{"description": "Hosting", "amount": "100.00", "account": "6500"}],
               "upload_id": file, "confirmed": true}),
    );
    assert_eq!(bill["due_date"], "2026-11-02", "{bill}");
    assert_eq!(bill["terms"], "Net 10", "{bill}");
}

/// `party_id` is the old spelling of `customer`/`vendor` and still names
/// the same record (until P5); a number that is one record's id and
/// another's name is refused, never guessed.
#[test]
fn party_id_alias_still_works() {
    let books = Books::new("documents-alias");
    let mut m = Mcp::spawn(&books);
    let harbor = m.customer("Harbor Coffee");
    let line = json!([{"description": "Work", "amount": "10.00"}]);
    let by_alias = m.call(
        "books.create_invoice",
        json!({"party_id": harbor, "date": "2026-02-01", "terms": "Net 30",
               "lines": line, "confirmed": true}),
    );
    let by_name = m.call(
        "books.create_invoice",
        json!({"customer": "harbor coffee", "date": "2026-02-01", "terms": "Net 30",
               "lines": line, "confirmed": true}),
    );
    let invoices = Mcp::events(&books, "invoice.created");
    let parties: Vec<u64> = invoices
        .iter()
        .map(|e| e["payload"]["customer_id"].as_u64().unwrap())
        .collect();
    assert_eq!(parties, [harbor, harbor], "one customer, two spellings");
    assert_eq!(by_alias["total"], by_name["total"]);

    // A customer named "2" that is not customer 2: the number is
    // ambiguous, and is refused rather than guessed.
    m.customer("Zeta");
    m.customer("2");
    let before = books.journal();
    let why = m
        .try_call(
            "books.create_invoice",
            json!({"customer": "2", "date": "2026-02-01", "terms": "Net 30",
                   "lines": line, "confirmed": true}),
        )
        .unwrap_err();
    assert!(why.contains("ambiguous"), "{why}");
    assert_eq!(books.journal(), before, "a refusal journals nothing");
    // Nothing at all names no customer.
    let why = m
        .try_call(
            "books.create_invoice",
            json!({"date": "2026-02-01", "terms": "Net 30", "lines": line, "confirmed": true}),
        )
        .unwrap_err();
    assert!(why.contains("customer"), "{why}");
}

impl Mcp {
    /// The party a name belongs to (its id).
    fn party_of(&mut self, name: &str) -> u64 {
        self.call("books.party", json!({"party": name}))["party"]["party_id"]
            .as_u64()
            .unwrap()
    }
}

/// D-55: `books.update_party` sets each contact field and `""` clears it.
#[test]
fn update_party_sets_and_clears_the_contact_block() {
    let books = Books::new("documents-contact");
    let mut m = Mcp::spawn(&books);
    m.customer("Harbor Coffee");
    let party = m.party_of("Harbor Coffee");
    for (key, value) in [
        ("phone", "555-0142"),
        ("contact", "Dana Reed"),
        ("billing_address", "12 Billing Way\nPortland, OR 97209"),
        ("shipping_address", "Dock 4, 9 Wharf Street"),
        ("billing_email", "ap@harbor.example"),
    ] {
        let out = m.call(
            "books.update_party",
            json!({"party": party.to_string(), key: value}),
        );
        assert_eq!(out[key], json!(value), "{key}: {out}");
    }
    // Its general email is still its own field, not the billing one.
    m.call(
        "books.update_party",
        json!({"party": party.to_string(), "email": "hello@harbor.example"}),
    );
    let shown = m.call("books.party", json!({"party": party.to_string()}));
    assert_eq!(shown["party"]["email"], "hello@harbor.example", "{shown}");
    assert_eq!(
        shown["party"]["billing_email"], "ap@harbor.example",
        "{shown}"
    );
    // "" clears each, one party.updated apiece.
    let before = Mcp::events(&books, "party.updated").len();
    let cleared = m.call(
        "books.update_party",
        json!({"party": party.to_string(), "phone": "", "contact": "", "billing_address": "",
               "shipping_address": "", "billing_email": ""}),
    );
    for key in [
        "phone",
        "contact",
        "billing_address",
        "shipping_address",
        "billing_email",
    ] {
        assert_eq!(cleared[key], Value::Null, "{key}: {cleared}");
    }
    assert_eq!(
        Mcp::events(&books, "party.updated").len(),
        before + 5,
        "one event per field"
    );
    // A full TIN never goes into the block, and a bad email is refused.
    let why = m
        .try_call(
            "books.update_party",
            json!({"party": party.to_string(), "phone": "123-45-6789"}),
        )
        .unwrap_err();
    assert!(why.contains("taxpayer"), "{why}");
    let why = m
        .try_call(
            "books.update_party",
            json!({"party": party.to_string(), "billing_email": "not an address"}),
        )
        .unwrap_err();
    assert!(why.contains("address"), "{why}");
}

/// D-55: an invoice's bill-to is the party's billing details when it has
/// them, else the general ones, and it is fixed at issue: a later edit
/// does not change the copy sent, though the next invoice takes it.
#[test]
fn an_invoice_bill_to_uses_the_billing_details() {
    let books = Books::new("documents-bill-to");
    let mut m = Mcp::spawn(&books);
    let harbor = m.customer("Harbor Coffee");
    let party = m.party_of("Harbor Coffee");
    m.call(
        "books.update_party",
        json!({"party": party.to_string(),
               "address": "PO Box 9, Portland, OR 97209",
               "email": "hello@harbor.example",
               "phone": "555-0142",
               "contact": "Dana Reed",
               "billing_address": "12 Billing Way, Portland, OR 97209",
               "billing_email": "ap@harbor.example",
               "shipping_address": "Dock 4, 9 Wharf Street"}),
    );
    let issue = |date: &str| {
        json!({"customer": harbor.to_string(), "date": date, "terms": "Net 30",
               "lines": [{"description": "Coffee", "amount": "100.00"}],
               "confirmed": true})
    };
    m.call("books.create_invoice", issue("2026-02-01"));
    let created = Mcp::events(&books, "invoice.created");
    let bill_to = &created.last().unwrap()["payload"]["bill_to"];
    assert_eq!(
        bill_to["address"], "12 Billing Way, Portland, OR 97209",
        "the billing address, not the general one: {bill_to}"
    );
    assert_eq!(bill_to["email"], "ap@harbor.example", "{bill_to}");
    assert_eq!(bill_to["billing_email"], "ap@harbor.example", "{bill_to}");
    assert_eq!(
        bill_to["billing_address"],
        "12 Billing Way, Portland, OR 97209"
    );
    assert_eq!(bill_to["phone"], "555-0142", "{bill_to}");
    assert_eq!(bill_to["contact"], "Dana Reed", "{bill_to}");
    assert_eq!(bill_to["shipping_address"], "Dock 4, 9 Wharf Street");

    // A later edit: the issued snapshot is what it was.
    m.call(
        "books.update_party",
        json!({"party": party.to_string(), "billing_address": "99 New Way", "phone": "555-9999"}),
    );
    let again = Mcp::events(&books, "invoice.created");
    assert_eq!(
        again.last().unwrap()["payload"]["bill_to"]["address"],
        "12 Billing Way, Portland, OR 97209",
        "an issued copy never changes"
    );
    // The next invoice takes today's details.
    m.call("books.create_invoice", issue("2026-03-01"));
    let third = Mcp::events(&books, "invoice.created");
    assert_eq!(
        third.last().unwrap()["payload"]["bill_to"]["address"],
        "99 New Way"
    );
    assert_eq!(
        third.last().unwrap()["payload"]["bill_to"]["phone"],
        "555-9999"
    );

    // A one-off bill-to override carries the whole block, for this
    // document alone.
    m.call(
        "books.create_invoice",
        json!({"customer": harbor.to_string(), "date": "2026-04-01", "terms": "Net 30",
               "lines": [{"description": "Coffee", "amount": "100.00"}],
               "bill_to": {"name": "Harbor Coffee — Accounts Payable",
                           "billing_address": "1 One-Off Plaza",
                           "billing_email": "oneoff@harbor.example",
                           "contact": "Sam Ortiz", "phone": "555-0100",
                           "shipping_address": "Gate 2"},
               "confirmed": true}),
    );
    let overridden = Mcp::events(&books, "invoice.created");
    let bill_to = &overridden.last().unwrap()["payload"]["bill_to"];
    assert_eq!(bill_to["name"], "Harbor Coffee — Accounts Payable");
    assert_eq!(bill_to["address"], "1 One-Off Plaza", "{bill_to}");
    assert_eq!(bill_to["email"], "oneoff@harbor.example", "{bill_to}");
    assert_eq!(bill_to["contact"], "Sam Ortiz", "{bill_to}");
    assert_eq!(bill_to["phone"], "555-0100", "{bill_to}");
    assert_eq!(bill_to["shipping_address"], "Gate 2", "{bill_to}");
}

// P4b S5: sent tracking (merged from branch S5).

/// Two sends on one invoice: the first the original (defaults: the
/// token's name, email, today's company day), the second a reminder with
/// the channel's message id; then `books.documents` shows the last send
/// and `books.document` the whole record.
#[test]
fn mark_sent_records_who_and_how() {
    let books = common::Books::new("p4b-s5-mark-sent");
    let mut wire = books.session("p4b-s5");
    let customer =
        common::answer_of(&wire.call("books.add_customer", json!({"name": "Fernhill Labs"})))
            .unwrap()["id"]
            .as_u64()
            .unwrap();
    let invoice = common::answer_of(&wire.call(
        "books.create_invoice",
        json!({"party_id": customer, "date": "2026-03-01", "due_date": "2026-03-31",
               "lines": [{"description": "Design", "amount": "1200.00"}], "confirmed": true}),
    ))
    .unwrap()["id"]
        .as_u64()
        .unwrap();

    let first = common::answer_of(&wire.call(
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
    let second = common::answer_of(&wire.call(
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

    let listed =
        common::answer_of(&wire.call("books.documents", json!({"kind": "invoices"}))).unwrap();
    assert_eq!(
        listed["documents"][0]["ref"],
        json!(format!("invoice:{invoice}"))
    );
    assert_eq!(
        listed["documents"][0]["last_sent"],
        json!("2026-03-20"),
        "the list shows the latest send"
    );
    let one = common::answer_of(&wire.call(
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

// P4b S6: paying on terms (merged from branch S6), in its own module so its
// helpers keep their names beside S4.4's.
mod paying_on_terms {
    use super::common::answer_of;
    use super::*;

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
        bills: u32,
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
        ok(
            &mut wire,
            "books.add_account",
            json!({"code": "6999", "name": "General Expenses", "account_type": "expense"}),
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
            bills: 0,
        }
    }

    impl Setup {
        fn invoice(&mut self, date: &str, due: &str, amount: &str) -> u64 {
            ok(
                &mut self.wire,
                "books.create_invoice",
                json!({"party_id": self.customer, "date": date, "due_date": due,
                   "lines": [{"description": "Consulting", "amount": amount}],
                   "confirmed": true}),
            )["id"]
                .as_u64()
                .unwrap()
        }

        fn bill(&mut self, date: &str, due: &str, amount: &str) -> u64 {
            // Under documents_v2 (P4b S4.4) a bill names the vendor's invoice
            // number, each line's account, and carries the vendor's invoice.
            self.bills += 1;
            let file = self.wire.upload(
                format!("vendor invoice TH-{}", self.bills).as_bytes(),
                "bill.pdf",
                "application/pdf",
                "receipt",
            );
            ok(
                &mut self.wire,
                "books.create_bill",
                json!({"party_id": self.vendor, "date": date, "due_date": due,
                   "vendor_ref": format!("TH-{}", self.bills),
                   "lines": [{"description": "Paper", "amount": amount, "account": "6999"}],
                   "upload_id": file, "confirmed": true}),
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
}
