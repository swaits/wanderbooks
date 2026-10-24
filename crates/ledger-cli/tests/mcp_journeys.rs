//! End-to-end journeys: drive a real `wanderbooks server` with raw
//! JSON-RPC, as an agent reaches it (common.rs), exercising draft →
//! preview → receipt policy → post, and the rest of an agent's day.

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
    /// A new session with `books` (initialized by the test itself).
    fn spawn(books: &Books) -> Self {
        Self {
            wire: books.wire(),
            next_id: 1,
        }
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let resp = self.request_raw(method, params);
        assert!(
            resp.get("error").is_none(),
            "JSON-RPC error for {method}: {}",
            resp["error"]
        );
        resp
    }

    fn request_raw(&mut self, method: &str, params: Value) -> Value {
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

    fn notify(&mut self, method: &str) {
        self.wire
            .send(&json!({"jsonrpc": "2.0", "method": method, "params": {}}));
    }

    /// Call a tool; panic on tool error. Returns structuredContent.
    fn call(&mut self, tool: &str, args: Value) -> Value {
        let resp = self.request("tools/call", json!({"name": tool, "arguments": args}));
        let result = &resp["result"];
        assert_ne!(
            result.get("isError"),
            Some(&json!(true)),
            "tool {tool} returned an error: {}",
            result["content"]
        );
        result.get("structuredContent").cloned().unwrap_or_else(|| {
            let text = result["content"][0]["text"].as_str().unwrap();
            serde_json::from_str(text).unwrap()
        })
    }

    /// Call a tool expecting an error; returns the error text.
    fn call_err(&mut self, tool: &str, args: Value) -> String {
        let resp = self.request("tools/call", json!({"name": tool, "arguments": args}));
        let result = &resp["result"];
        assert_eq!(
            result.get("isError"),
            Some(&json!(true)),
            "expected {tool} to fail, got {result}"
        );
        result["content"][0]["text"].as_str().unwrap().to_string()
    }

    /// The bytes of a file a tool handed over (a `HandedFile`): fetched by
    /// its download, matching the size and SHA-256 it states.
    fn handed(&mut self, file: &Value) -> Vec<u8> {
        assert!(file["expires_at"].is_string(), "{file}");
        self.wire.fetch(file)
    }

    /// A file handed over, as text.
    fn handed_text(&mut self, file: &Value) -> String {
        String::from_utf8(self.handed(file)).expect("the file is text")
    }

    /// Upload `bytes` as an agent does (D-46): the upload_id.
    fn upload(&mut self, bytes: &[u8], file_name: &str, mime_type: &str, purpose: &str) -> String {
        self.wire.upload(bytes, file_name, mime_type, purpose)
    }

    /// Upload a CSV for an import: the upload_id.
    fn csv_upload(&mut self, csv: &str) -> String {
        self.upload(csv.as_bytes(), "import.csv", "text/csv", "import")
    }
}

#[test]
fn mcp_end_to_end() {
    let books = Books::new("main");
    let mut mcp = Mcp::spawn(&books);

    // Initialize: the server identifies itself and delivers the contract.
    let init = mcp.request(
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "smoke-test", "version": "0.1.0"}
        }),
    );
    assert_eq!(init["result"]["serverInfo"]["name"], "wanderbooks");
    let instructions = init["result"]["instructions"].as_str().unwrap();
    assert!(instructions.contains("bookkeeping agent"));
    assert!(instructions.contains("decimal strings"));
    mcp.notify("notifications/initialized");

    // Tool listing: the narrow tools; chart is read-only.
    let list = mcp.request("tools/list", json!({}));
    let tools = list["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 122, "expected 122 tools, got {}", tools.len());
    // Files come in as tool input: books.begin_upload, never a local path.
    assert!(tools.iter().any(|t| t["name"] == "books.begin_upload"));
    assert!(!tools.iter().any(|t| t["name"] == "books.import_local_file"));
    let chart = tools.iter().find(|t| t["name"] == "books.chart").unwrap();
    assert_eq!(chart["annotations"]["readOnlyHint"], true);
    assert!(
        chart["description"]
            .as_str()
            .unwrap()
            .contains("chart of accounts")
    );

    // Fresh books: empty chart.
    let chart_out = mcp.call("books.chart", json!({}));
    assert_eq!(chart_out["accounts"].as_array().unwrap().len(), 0);
    assert_eq!(chart_out["base_currency"], "USD");

    // Build a chart.
    for (code, name, typ) in [
        ("1000", "Cash", "asset"),
        ("3000", "Owner's Equity", "equity"),
        ("4000", "Service Revenue", "revenue"),
        ("6000", "Rent Expense", "expense"),
    ] {
        let out = mcp.call(
            "books.add_account",
            json!({"code": code, "name": name, "account_type": typ}),
        );
        assert!(out["account_id"].as_u64().unwrap() > 0);
    }
    // Duplicate code is rejected.
    let err = mcp.call_err(
        "books.add_account",
        json!({"code": "1000", "name": "Cash 2", "account_type": "asset"}),
    );
    assert!(err.contains("already in use"), "got: {err}");

    // Draft → lines → preview.
    let draft = mcp.call(
        "books.draft_entry",
        json!({"date": "2026-01-15", "description": "January rent"}),
    );
    let entry_id = draft["entry_id"].as_u64().unwrap();
    mcp.call(
        "books.add_line",
        json!({"entry_id": entry_id, "account": "6000", "side": "debit", "amount": "100.00"}),
    );
    mcp.call(
        "books.add_line",
        json!({"entry_id": entry_id, "account": "1000", "side": "credit", "amount": "100.00"}),
    );
    let preview = mcp.call("books.post_entry", json!({"entry_id": entry_id}));
    assert_eq!(preview["status"], "preview");
    assert_eq!(preview["total"], "100.00");
    assert_eq!(preview["receipt_required"], true);
    assert_eq!(preview["receipt_missing"], true);
    assert_eq!(preview["lines"].as_array().unwrap().len(), 2);
    // Resulting balances: rent expense goes 0 → 100.
    let rent_line = preview["lines"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["code"] == "6000")
        .unwrap();
    assert_eq!(rent_line["balance_before"], "0.00");
    assert_eq!(rent_line["balance_after"], "100.00");

    // Confirmed post without a receipt is rejected: >$25, no receipt.
    let err = mcp.call_err(
        "books.post_entry",
        json!({"entry_id": entry_id, "confirmed": true}),
    );
    let err_json: serde_json::Value = serde_json::from_str(&err).unwrap();
    assert_eq!(err_json["code"], "receipt_required");
    assert_eq!(err_json["remedy_tool"], "books.attach_receipt");

    // Attach the receipt, then the confirmed post succeeds.
    let png = mcp.upload(b"fake-png-bytes", "rent.png", "image/png", "receipt");
    let attached = mcp.call(
        "books.attach_receipt",
        json!({"entry_id": entry_id, "upload_id": png}),
    );
    assert_eq!(attached["deduplicated"], false);
    assert_eq!(attached["sha256"].as_str().unwrap().len(), 64);
    let posted = mcp.call(
        "books.post_entry",
        json!({"entry_id": entry_id, "confirmed": true}),
    );
    assert_eq!(posted["status"], "posted");
    assert_eq!(posted["receipt_missing"], false);

    // Reports agree.
    let tb = mcp.call("books.trial_balance", json!({}));
    assert_eq!(tb["balanced"], true);
    assert_eq!(tb["total_debits"], "100.00");
    assert_eq!(tb["total_credits"], "100.00");
    let pnl = mcp.call(
        "books.profit_and_loss",
        json!({"start": "2026-01-01", "end": "2026-01-31"}),
    );
    assert_eq!(pnl["expenses"], "100.00");
    assert_eq!(pnl["net_income"], "-100.00");
    let bs = mcp.call("books.balance_sheet", json!({"as_of": "2026-01-31"}));
    assert_eq!(bs["balanced"], true);

    // Digest and search see the entry.
    let digest = mcp.call("books.digest", json!({"since": "2026-01-01"}));
    assert_eq!(digest["entries"].as_array().unwrap().len(), 1);
    assert_eq!(digest["entries"][0]["receipt_missing"], false);
    // Stale legal rules ride along with the digest. Which ones depends on
    // the server's clock, so assert only what holds on any date.
    for rule in digest["stale_rules"]
        .as_array()
        .expect("stale_rules is a list")
    {
        assert!(rule["name"].is_string() && rule["source"].is_string());
    }
    let search = mcp.call("books.search_entries", json!({"text": "rent"}));
    assert_eq!(search["entries"].as_array().unwrap().len(), 1);
    assert_eq!(search["entries"][0]["receipt_count"], 1);

    // Export works.
    let export = mcp.call(
        "books.export",
        json!({"format": "csv", "report": "journal", "start": "2026-01-01", "end": "2026-12-31"}),
    );
    assert!(mcp.handed_text(&export["file"]).contains("January rent"));

    // Reversal: preview then post; the book still balances.
    let rev_preview = mcp.call(
        "books.reverse_entry",
        json!({"entry_id": entry_id, "date": "2026-01-16", "description": "Reverse rent"}),
    );
    assert_eq!(rev_preview["status"], "preview");
    let rev = mcp.call(
        "books.reverse_entry",
        json!({"entry_id": entry_id, "date": "2026-01-16", "description": "Reverse rent", "confirmed": true}),
    );
    assert_eq!(rev["status"], "posted");
    let tb2 = mcp.call("books.trial_balance", json!({}));
    assert_eq!(tb2["balanced"], true);
    assert_eq!(tb2["total_debits"], "0.00");
}

#[test]
fn mcp_assets_end_to_end() {
    let books = Books::new("assets");
    let mut mcp = Mcp::spawn(&books);
    let init = mcp.request(
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "smoke-test", "version": "0.1.0"}
        }),
    );
    assert_eq!(init["result"]["serverInfo"]["name"], "wanderbooks");
    mcp.notify("notifications/initialized");

    // Chart for the asset: cost, contra, expense, bank, gain.
    for (code, name, typ) in [
        ("1500", "Equipment", "asset"),
        ("1501", "Accumulated Depreciation", "asset"),
        ("6100", "Depreciation Expense", "expense"),
        ("1000", "Bank", "asset"),
        ("4900", "Gain on Disposal", "revenue"),
    ] {
        mcp.call(
            "books.add_account",
            json!({"code": code, "name": name, "account_type": typ}),
        );
    }

    // Register the van: 60,000, 60-month life, bought 2026-01-15.
    let reg = mcp.call(
        "books.register_asset",
        json!({
            "description": "Sprinter van",
            "purchase_date": "2026-01-15",
            "cost": "60000.00",
            "useful_life_months": 60,
            "cost_account": "1500",
            "accum_depr_account": "1501",
            "expense_account": "6100"
        }),
    );
    let asset_id = reg["asset_id"].as_u64().unwrap();

    // Schedule: 61 calendar months, lifetime total exactly 60,000.
    let sched = mcp.call("books.asset_schedule", json!({"asset_id": asset_id}));
    let lines = sched["schedule"].as_array().unwrap();
    assert_eq!(lines.len(), 61);
    assert_eq!(lines[0]["period"], "2026-01");
    assert_eq!(lines[0]["amount"], "548.39");
    assert_eq!(lines[60]["period"], "2031-01");
    let total: f64 = lines
        .iter()
        .map(|l| l["amount"].as_str().unwrap().parse::<f64>().unwrap())
        .sum();
    assert!((total - 60000.0).abs() < 0.005, "total was {total}");
    assert_eq!(sched["status"], "active");

    // List: one active asset.
    let list = mcp.call("books.list_assets", json!({}));
    assert_eq!(list["assets"].as_array().unwrap().len(), 1);
    assert_eq!(list["assets"][0]["status"], "active");

    // Run January: posts 548.39.
    let run1 = mcp.call(
        "books.run_depreciation",
        json!({"period": "2026-01", "asset_id": asset_id}),
    );
    assert_eq!(run1["runs"][0]["amount"], "548.39");
    assert_eq!(run1["runs"][0]["already_posted"], false);
    let entry_id = run1["runs"][0]["entry_id"].as_u64().unwrap();

    // Idempotent: second run returns the same entry.
    let run2 = mcp.call(
        "books.run_depreciation",
        json!({"period": "2026-01", "asset_id": asset_id}),
    );
    assert_eq!(run2["runs"][0]["already_posted"], true);
    assert_eq!(run2["runs"][0]["entry_id"], entry_id);

    // Run-all for February: picks up the van, posts a full 1,000.
    let run_all = mcp.call("books.run_depreciation", json!({"period": "2026-02"}));
    assert_eq!(run_all["runs"][0]["amount"], "1000.00");
    assert_eq!(run_all["skipped"].as_array().unwrap().len(), 0);
    // And again: already posted.
    let run_all2 = mcp.call("books.run_depreciation", json!({"period": "2026-02"}));
    assert_eq!(run_all2["runs"][0]["already_posted"], true);

    // Dispose preview: book value 60000 − 1548.39 = 58451.61;
    // proceeds 60000 → gain 1548.39. Receipt required.
    let preview = mcp.call(
        "books.dispose_asset",
        json!({
            "asset_id": asset_id,
            "date": "2026-02-28",
            "proceeds": "60000",
            "proceeds_account": "1000",
            "gain_loss_account": "4900"
        }),
    );
    assert_eq!(preview["status"], "preview");
    assert_eq!(preview["book_value"], "58451.61");
    assert_eq!(preview["gain_loss"], "1548.39");
    assert_eq!(preview["final_depreciation"], "0.00");
    assert_eq!(preview["receipt_required"], true);

    // Confirmed without the bill of sale: rejected.
    let err = mcp.call_err(
        "books.dispose_asset",
        json!({
            "asset_id": asset_id,
            "date": "2026-02-28",
            "proceeds": "60000",
            "proceeds_account": "1000",
            "gain_loss_account": "4900",
            "confirmed": true
        }),
    );
    let err_json: serde_json::Value = serde_json::from_str(&err).unwrap();
    assert_eq!(err_json["code"], "receipt_required");
    assert_eq!(err_json["remedy_tool"], "books.file_document");

    // File the bill of sale, then confirm with its attachment id.
    let up1 = mcp.upload(
        b"fake bill of sale",
        "bill-of-sale.pdf",
        "application/pdf",
        "document",
    );
    let bill = mcp.call(
        "books.file_document",
        json!({
            "category": "legal",
            "upload_id": up1
        }),
    );
    let aid = bill["attachment_id"].as_u64().unwrap();
    let posted = mcp.call(
        "books.dispose_asset",
        json!({
            "asset_id": asset_id,
            "date": "2026-02-28",
            "proceeds": "60000",
            "proceeds_account": "1000",
            "gain_loss_account": "4900",
            "confirmed": true,
            "receipt_attachment_ids": [aid]
        }),
    );
    assert_eq!(posted["status"], "posted");
    assert_eq!(posted["gain_loss"], "1548.39");

    // The bill of sale is linked to the disposal entry.
    let receipts = mcp.call(
        "books.list_receipts",
        json!({"entry_id": posted["entry_id"]}),
    );
    assert_eq!(receipts["receipts"].as_array().unwrap().len(), 1);

    // Asset is disposed; further depreciation is refused.
    let list2 = mcp.call("books.list_assets", json!({}));
    assert_eq!(list2["assets"][0]["status"], "disposed");
    let err2 = mcp.call_err(
        "books.run_depreciation",
        json!({"period": "2026-03", "asset_id": asset_id}),
    );
    assert!(err2.contains("disposed"), "got: {err2}");

    // The books still balance.
    let tb = mcp.call("books.trial_balance", json!({}));
    assert_eq!(tb["balanced"], true);
}

#[test]
fn mcp_receipt_policy_rejects_split_entries() {
    let books = Books::new("split");
    let mut mcp = Mcp::spawn(&books);
    let init = mcp.request(
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "smoke-test", "version": "0.1.0"}
        }),
    );
    assert_eq!(init["result"]["serverInfo"]["name"], "wanderbooks");
    mcp.notify("notifications/initialized");

    for (code, name, typ) in [("1000", "Cash", "asset"), ("6000", "Meals", "expense")] {
        mcp.call(
            "books.add_account",
            json!({"code": code, "name": name, "account_type": typ}),
        );
    }
    // Post a $25 entry (at the threshold, no receipt needed) and attach a
    // receipt to it so the setup is clean.
    let e1 = mcp.call(
        "books.draft_entry",
        json!({"date": "2026-03-01", "description": "Client dinner"}),
    );
    let id1 = e1["entry_id"].as_u64().unwrap();
    mcp.call(
        "books.add_line",
        json!({"entry_id": id1, "account": "6000", "side": "debit", "amount": "25.00"}),
    );
    mcp.call(
        "books.add_line",
        json!({"entry_id": id1, "account": "1000", "side": "credit", "amount": "25.00"}),
    );
    let p1 = mcp.call(
        "books.post_entry",
        json!({"entry_id": id1, "confirmed": true}),
    );
    assert_eq!(p1["status"], "posted");

    // A second $25 entry, same day, same description, same accounts: the
    // group totals $50, so the policy bites.
    let e2 = mcp.call(
        "books.draft_entry",
        json!({"date": "2026-03-01", "description": "Client dinner"}),
    );
    let id2 = e2["entry_id"].as_u64().unwrap();
    mcp.call(
        "books.add_line",
        json!({"entry_id": id2, "account": "6000", "side": "debit", "amount": "25.00"}),
    );
    mcp.call(
        "books.add_line",
        json!({"entry_id": id2, "account": "1000", "side": "credit", "amount": "25.00"}),
    );
    let preview = mcp.call("books.post_entry", json!({"entry_id": id2}));
    assert_eq!(preview["receipt_required"], true);
    assert_eq!(preview["split_detected"].as_array().unwrap().len(), 1);
    assert_eq!(preview["split_detected"][0], id1);

    // Confirming without a receipt fails with the structured error.
    let err = mcp.call_err(
        "books.post_entry",
        json!({"entry_id": id2, "confirmed": true}),
    );
    let err_json: serde_json::Value = serde_json::from_str(&err).unwrap();
    assert_eq!(err_json["code"], "receipt_required");
    assert_eq!(err_json["split_group"].as_array().unwrap().len(), 1);

    // A different description is not a split: posts fine.
    let e3 = mcp.call(
        "books.draft_entry",
        json!({"date": "2026-03-01", "description": "Taxi to airport"}),
    );
    let id3 = e3["entry_id"].as_u64().unwrap();
    mcp.call(
        "books.add_line",
        json!({"entry_id": id3, "account": "6000", "side": "debit", "amount": "25.00"}),
    );
    mcp.call(
        "books.add_line",
        json!({"entry_id": id3, "account": "1000", "side": "credit", "amount": "25.00"}),
    );
    let p3 = mcp.call(
        "books.post_entry",
        json!({"entry_id": id3, "confirmed": true}),
    );
    assert_eq!(p3["status"], "posted");
}

#[test]
fn mcp_receipt_corrections() {
    let books = Books::new("receipt-corrections");
    let mut mcp = Mcp::spawn(&books);
    mcp.request(
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "smoke-test", "version": "0.1.0"}
        }),
    );
    mcp.notify("notifications/initialized");
    for (code, name, typ) in [("1000", "Cash", "asset"), ("6000", "Software", "expense")] {
        mcp.call(
            "books.add_account",
            json!({"code": code, "name": name, "account_type": typ}),
        );
    }
    let post = |mcp: &mut Mcp, desc: &str, amount: &str, receipt: &[u8]| -> (u64, u64) {
        let e = mcp.call(
            "books.draft_entry",
            json!({"date": "2026-04-01", "description": desc}),
        );
        let id = e["entry_id"].as_u64().unwrap();
        mcp.call(
            "books.add_line",
            json!({"entry_id": id, "account": "6000", "side": "debit", "amount": amount}),
        );
        mcp.call(
            "books.add_line",
            json!({"entry_id": id, "account": "1000", "side": "credit", "amount": amount}),
        );
        let up2 = mcp.upload(receipt, "receipt.eml", "message/rfc822", "receipt");
        let att = mcp.call(
            "books.attach_receipt",
            json!({"entry_id": id, "upload_id": up2}),
        );
        let posted = mcp.call(
            "books.post_entry",
            json!({"entry_id": id, "confirmed": true}),
        );
        assert_eq!(posted["status"], "posted");
        (id, att["attachment_id"].as_u64().unwrap())
    };

    // A $100 entry whose only receipt is an .eml.
    let (big, eml) = post(&mut mcp, "Annual license", "100.00", b"eml-bytes");

    // Unlinking its only receipt would break the policy: refused, with the remedy.
    let err = mcp.call_err(
        "books.unlink_receipt",
        json!({"attachment_id": eml, "reason": "replacing with PDF"}),
    );
    let err_json: serde_json::Value = serde_json::from_str(&err).unwrap();
    assert_eq!(err_json["code"], "receipt_required");
    assert_eq!(err_json["entry_id"], big);
    assert_eq!(err_json["remedy_tool"], "books.attach_receipt");

    // Replace: attach the PDF first, then unlink the .eml.
    let up3 = mcp.upload(b"pdf-bytes", "receipt.pdf", "application/pdf", "receipt");
    let pdf = mcp.call(
        "books.attach_receipt",
        json!({"entry_id": big, "upload_id": up3}),
    )["attachment_id"]
        .as_u64()
        .unwrap();
    let un = mcp.call(
        "books.unlink_receipt",
        json!({"attachment_id": eml, "reason": "replaced by the PDF render"}),
    );
    assert_eq!(un["entry_id"], big);
    assert_eq!(un["remaining_receipts"], 1);

    // A linked receipt can't be withdrawn; an unlinked one can, with a reason.
    let err = mcp.call_err(
        "books.withdraw_attachment",
        json!({"attachment_id": pdf, "reason": "x"}),
    );
    assert!(err.contains("unlink it first"), "{err}");
    let err = mcp.call_err(
        "books.withdraw_attachment",
        json!({"attachment_id": eml, "reason": " "}),
    );
    assert!(err.contains("reason"), "{err}");
    mcp.call(
        "books.withdraw_attachment",
        json!({"attachment_id": eml, "reason": "file PDFs, not .eml"}),
    );

    // Listings hide the withdrawn .eml unless asked; the entry keeps the PDF.
    let listed = mcp.call("books.list_receipts", json!({}));
    let ids: Vec<u64> = listed["receipts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["attachment_id"].as_u64().unwrap())
        .collect();
    assert_eq!(ids, vec![pdf]);
    let all = mcp.call("books.list_receipts", json!({"include_withdrawn": true}));
    let gone = all["receipts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["attachment_id"] == eml)
        .unwrap()
        .clone();
    assert_eq!(gone["withdrawn"], true);
    assert_eq!(gone["withdrawn_reason"], "file PDFs, not .eml");
    assert_eq!(gone["entry_id"], serde_json::Value::Null);
    let entry = mcp.call("books.get_entry", json!({"entry_id": big}));
    assert_eq!(entry["receipts"].as_array().unwrap().len(), 1);
    assert_eq!(entry["receipts"][0]["attachment_id"], pdf);
    // A withdrawn attachment can never be linked again.
    let err = mcp.call_err(
        "books.link_receipt",
        json!({"attachment_id": eml, "entry_id": big}),
    );
    assert!(err.contains("withdrawn"), "{err}");

    // Under the threshold, the last receipt may go.
    let (small, small_eml) = post(&mut mcp, "Domain renewal", "10.00", b"small-eml");
    let un = mcp.call(
        "books.unlink_receipt",
        json!({"attachment_id": small_eml, "reason": "wrong file"}),
    );
    assert_eq!(un["entry_id"], small);
    assert_eq!(un["remaining_receipts"], 0);

    // The books never moved.
    let tb = mcp.call("books.trial_balance", json!({}));
    assert_eq!(tb["balanced"], true);
    assert_eq!(tb["total_debits"], "110.00");
}

#[test]
fn mcp_resources() {
    let books = Books::new("resources");
    let mut mcp = Mcp::spawn(&books);
    let init = mcp.request(
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "smoke-test", "version": "0.1.0"}
        }),
    );
    assert_eq!(init["result"]["serverInfo"]["name"], "wanderbooks");
    // Resources are advertised.
    assert!(!init["result"]["capabilities"]["resources"].is_null());
    mcp.notify("notifications/initialized");

    mcp.call(
        "books.add_account",
        json!({"code": "1000", "name": "Cash", "account_type": "asset"}),
    );

    // List resources.
    let list = mcp.request("resources/list", json!({}));
    let uris: Vec<&str> = list["result"]["resources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["uri"].as_str().unwrap())
        .collect();
    assert!(uris.contains(&"books://chart"));
    assert!(uris.contains(&"books://reports/trial-balance"));
    assert!(uris.contains(&"books://reports/balance-sheet"));
    assert!(uris.contains(&"books://reports/profit-and-loss"));
    assert!(uris.contains(&"books://digest"));

    // Read the chart and trial balance as resources.
    let chart = mcp.request("resources/read", json!({"uri": "books://chart"}));
    let text = chart["result"]["contents"][0]["text"].as_str().unwrap();
    let chart_json: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(chart_json["accounts"].as_array().unwrap().len(), 1);

    let tb = mcp.request(
        "resources/read",
        json!({"uri": "books://reports/trial-balance"}),
    );
    let tb_text = tb["result"]["contents"][0]["text"].as_str().unwrap();
    let tb_json: serde_json::Value = serde_json::from_str(tb_text).unwrap();
    assert_eq!(tb_json["balanced"], true);

    // Unknown resource → JSON-RPC error.
    let resp = mcp.request_raw("resources/read", json!({"uri": "books://nope"}));
    assert!(resp.get("error").is_some());
}

/// Start a session: initialize, then initialized.
fn init(mcp: &mut Mcp) -> Value {
    let init = mcp.request(
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "smoke-test", "version": "0.1.0"}
        }),
    );
    mcp.notify("notifications/initialized");
    init
}

/// Outputs that carry full TINs (books.get_w9, books.vendor_1099 with
/// include_tins, books.export_cpa_package with TINs) come back as files
/// handed over (D-29: no tool takes a path), and each is refused by its
/// bytes when given back to be filed: a TIN-bearing file can't be filed
/// back in as a receipt or document and read out through
/// books.get_receipt (S7 of the P0 review, 2026-10-16).
#[test]
fn tin_bearing_files_are_refused_when_given_back_to_be_filed() {
    let books = Books::new("tin-files");
    let w9 = b"%PDF-1.4 W-9 TIN 291-12-2846";
    let mut mcp = Mcp::spawn(&books);
    init(&mut mcp);
    mcp.call(
        "books.add_account",
        json!({"code": "1000", "name": "Cash", "account_type": "asset"}),
    );
    // The 1099 worksheet counts payments out of the books' cash accounts.
    mcp.call(
        "books.set_setting",
        json!({"changes": [{"key": "books.cash_accounts", "value": ["1000"]}], "decided_by": "Test Owner"}),
    );
    mcp.call(
        "books.add_party",
        json!({"name": "Acme", "roles": ["contractor"]}),
    );
    let w9_upload = mcp.upload(w9, "w9.pdf", "application/pdf", "w9");
    mcp.call(
        "books.set_w9",
        json!({"party": "Acme", "received_on": "2026-01-03", "entity_type": "Individual",
               "tin": "291-12-2846", "tin_type": "ssn", "w9_upload_id": w9_upload}),
    );
    // Each comes back by its download.
    let got = mcp.call("books.get_w9", json!({"party": "Acme"}));
    let w9_back = mcp.handed(&got["file"]);
    assert_eq!(w9_back, w9, "the W-9 file, as stored");
    let ws = mcp.call(
        "books.vendor_1099",
        json!({"year": 2026, "include_tins": true, "csv": true}),
    );
    assert_eq!(ws["csv"]["file_name"], "1099-worksheet-2026-with-tins.csv");
    let pkg = mcp.call(
        "books.export_cpa_package",
        json!({"cash_accounts": ["1000"]}),
    );
    assert_eq!(pkg["tins_included"], true);
    let handed_over = [w9_back, mcp.handed(&ws["csv"]), mcp.handed(&pkg["file"])];
    // Given back, each is refused by its bytes, by every way a file is
    // filed.
    let journal = books.journal();
    for (n, bytes) in handed_over.iter().enumerate() {
        // One upload, refused by each tool and so left for the next.
        let up = mcp.upload(bytes, "receipt.pdf", "application/pdf", "receipt");
        for (tool, args) in [
            ("books.attach_receipt", json!({"upload_id": up})),
            (
                "books.file_document",
                json!({"category": "statement", "upload_id": up}),
            ),
        ] {
            let why = mcp.call_err(tool, args);
            assert!(
                why.contains("wrote with full TINs"),
                "{tool} file {n}: {why}"
            );
        }
    }
    assert_eq!(books.journal(), journal, "a refusal stored something");
    // Without TINs, the 1099 worksheet is an ordinary file: it may be
    // filed like any other.
    let plain = mcp.call("books.vendor_1099", json!({"year": 2026, "csv": true}));
    assert!(plain["warning"].is_null(), "{plain}");
    assert_eq!(plain["csv"]["file_name"], "1099-worksheet-2026.csv");
    let bytes = mcp.handed(&plain["csv"]);
    let up6 = mcp.upload(&bytes, "plain.csv", "text/csv", "document");
    mcp.call(
        "books.file_document",
        json!({"category": "statement", "upload_id": up6}),
    );
}

/// The connect text and the receipt rule show a few account codes of each
/// kind, shortened, and say books.settings lists them: it does, every one,
/// uncut, so no account the rule names is out of the agent's reach.
#[test]
fn every_receipt_account_left_out_of_the_connect_text_is_in_books_settings() {
    let long = "6000-A-CODE-LONGER-THAN-ANY-RULE-SHOWS-IT";
    let required: Vec<String> = std::iter::once(long.to_string())
        .chain((1..20).map(|i| format!("{}", 6000 + i)))
        .collect();
    let exempt: Vec<String> = (0..12).map(|i| format!("{}", 1100 + i)).collect();
    let books = Books::new("receipt-accounts");
    books.settle_with(
        "",
        Some(
            &json!({"receipt_policy": {"require_receipts_above": "0.00",
                   "receipt_required_accounts": required, "receipt_exempt_accounts": exempt}})
            .to_string(),
        ),
    );
    let mut mcp = Mcp::spawn(&books);
    let init = mcp.request(
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "smoke-test", "version": "0.1.0"}
        }),
    );
    mcp.notify("notifications/initialized");
    let connect = init["result"]["instructions"].as_str().unwrap().to_string();
    assert!(connect.contains("books.settings lists"), "{connect}");
    let settings = mcp.call("books.settings", json!({}));
    let rule = settings["receipt_rule"].as_str().unwrap();
    assert!(rule.contains("books.settings lists them"), "{rule}");
    let listed = |key: &str| -> Vec<String> {
        settings[key]
            .as_array()
            .unwrap_or_else(|| panic!("books.settings has no {key}: {settings}"))
            .iter()
            .map(|c| c.as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(listed("receipt_required_accounts"), required);
    assert_eq!(listed("receipt_exempt_accounts"), exempt);
    // Some codes really are left out of the connect text and the rule,
    // and the long one is shown cut there, whole here.
    let left_out: Vec<&String> = required
        .iter()
        .chain(&exempt)
        .filter(|c| !connect.contains(c.as_str()))
        .collect();
    assert!(left_out.len() > 20, "{connect}");
    assert!(!rule.contains(long), "{rule}");
    for code in left_out {
        assert!(
            listed("receipt_required_accounts").contains(code)
                || listed("receipt_exempt_accounts").contains(code),
            "{code} is in neither the connect text nor books.settings"
        );
    }
}

/// A bare `books://digest` (no `since`) reads this month so far: an entry
/// dated this month is in it, one dated last month is not (plan P0).
/// "This month" is the server's: read from its own bare digest, never this
/// process's clock, and the run starts over on fresh books if the month
/// turns while it runs (midnight on the 1st).
#[test]
fn digest_resource_bare_reads() {
    for attempt in 0..3 {
        if digest_bare_reads_once(attempt) {
            return;
        }
    }
    panic!("the month kept turning while the test ran");
}

/// One run of `digest_resource_bare_reads`: false if the server's month
/// turned during it (nothing is checked then).
fn digest_bare_reads_once(attempt: u32) -> bool {
    let books = Books::new(&format!("digest-bare-{attempt}"));
    let mut mcp = Mcp::spawn(&books);
    mcp.request(
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "smoke-test", "version": "0.1.0"}
        }),
    );
    mcp.notify("notifications/initialized");
    let since = |mcp: &mut Mcp, uri: &str| -> Value {
        let read = mcp.request("resources/read", json!({"uri": uri}));
        let text = read["result"]["contents"][0]["text"].as_str().unwrap();
        serde_json::from_str(text).unwrap()
    };
    // The server's first day of this month.
    let first: chrono::NaiveDate = since(&mut mcp, "books://digest")["since"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(chrono::Datelike::day(&first), 1, "{first}");
    let before = first.pred_opt().unwrap();
    for (code, name, typ) in [("1000", "Cash", "asset"), ("6000", "Fees", "expense")] {
        mcp.call(
            "books.add_account",
            json!({"code": code, "name": name, "account_type": typ}),
        );
    }
    for (date, what) in [(first, "this month"), (before, "last month")] {
        let id = mcp.call(
            "books.draft_entry",
            json!({"date": date.to_string(), "description": what}),
        )["entry_id"]
            .as_u64()
            .unwrap();
        for (side, code) in [("debit", "6000"), ("credit", "1000")] {
            mcp.call(
                "books.add_line",
                json!({"entry_id": id, "account": code, "side": side, "amount": "5.00"}),
            );
        }
        mcp.call(
            "books.post_entry",
            json!({"entry_id": id, "confirmed": true}),
        );
    }
    // Bare, and with an empty since=, alike (the default date itself is
    // pinned on every day of a month in server.rs's unit test).
    for uri in ["books://digest", "books://digest?since="] {
        let digest = since(&mut mcp, uri);
        if digest["since"] != first.to_string() {
            return false;
        }
        let seen: Vec<&str> = digest["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["description"].as_str().unwrap())
            .collect();
        assert_eq!(seen, ["this month"], "{uri}");
    }
    true
}

/// The handbook is one tool and one resource per section, with the
/// receipt rule in force rendered in; the connect text points to it.
#[test]
fn handbook_tool_and_resources() {
    let books = Books::new("handbook");
    let mut mcp = Mcp::spawn(&books);
    let init = mcp.request(
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "smoke-test", "version": "0.1.0"}
        }),
    );
    let instructions = init["result"]["instructions"].as_str().unwrap();
    assert!(instructions.chars().count() <= 1400);
    // The first 512 characters (not bytes: a name may be multibyte).
    let lead: String = instructions.chars().take(512).collect();
    assert!(lead.contains("books.handbook"));
    mcp.notify("notifications/initialized");
    let index = mcp.call("books.handbook", json!({}));
    let sections = index["sections"].as_array().unwrap();
    assert_eq!(sections[0]["id"], "contract");
    assert!(index["read_first"].as_str().unwrap().contains("contract"));
    assert_eq!(index["section"], Value::Null);
    // The schema names every section, and "all".
    let list = mcp.request("tools/list", json!({}));
    let tool = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == "books.handbook")
        .unwrap()
        .clone();
    let said = tool["inputSchema"]["properties"]["section"]["description"]
        .as_str()
        .unwrap()
        .to_string();
    for s in sections {
        let id = s["id"].as_str().unwrap();
        assert!(said.contains(id), "the schema doesn't name {id}: {said}");
    }
    assert!(said.contains("\"all\""), "{said}");
    // Section "all": every section's text, in order, under its own title.
    let all = mcp.call("books.handbook", json!({"section": "all"}));
    assert_eq!(all["section"]["title"], "The whole handbook");
    let text = all["section"]["text"].as_str().unwrap();
    let mut at = 0;
    for s in sections {
        let one = mcp.call("books.handbook", json!({"section": s["id"]}));
        // A section is titled by its heading, as the index lists it.
        assert_eq!(one["section"]["title"], s["title"]);
        let part = one["section"]["text"].as_str().unwrap();
        assert_eq!(
            part.lines().next().unwrap(),
            format!("# {}", s["title"].as_str().unwrap())
        );
        at += text[at..].find(part).expect("a section missing from all") + part.len();
    }
    let one = mcp.call(
        "books.handbook",
        json!({"section": "evidence-and-receipts"}),
    );
    let text = one["section"]["text"].as_str().unwrap();
    assert!(text.contains("needs at least one receipt"), "{text}");
    assert!(!text.contains("{evidence_rule}"));
    let why = mcp.call_err("books.handbook", json!({"section": "nope"}));
    assert!(why.contains("contract"), "{why}");

    let list = mcp.request("resources/list", json!({}));
    let uris: Vec<&str> = list["result"]["resources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["uri"].as_str().unwrap())
        .collect();
    assert!(uris.contains(&"books://handbook"));
    for s in sections {
        let uri = s["uri"].as_str().unwrap();
        assert!(uris.contains(&uri), "{uri} not listed");
        let read = mcp.request("resources/read", json!({"uri": uri}));
        let c = &read["result"]["contents"][0];
        assert_eq!(c["mimeType"], "text/markdown");
        assert_eq!(
            c["text"].as_str().unwrap().chars().count() as u64,
            s["chars"].as_u64().unwrap()
        );
    }
    let read = mcp.request("resources/read", json!({"uri": "books://handbook"}));
    let idx: Value =
        serde_json::from_str(read["result"]["contents"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(idx["sections"].as_array().unwrap().len(), sections.len());
    let resp = mcp.request_raw("resources/read", json!({"uri": "books://handbook/nope"}));
    assert!(resp.get("error").is_some());
}

#[test]
fn mcp_import_export() {
    let books = Books::new("import_export");
    let mut mcp = Mcp::spawn(&books);
    mcp.request(
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "smoke-test", "version": "0.1.0"}
        }),
    );
    mcp.notify("notifications/initialized");

    let up1 = mcp.csv_upload("code,name,type,parent_code\n1000,Cash,asset,\n4000,Revenue,revenue,\n6010,Café 北京,expense,\n");
    let chart = mcp.call("books.import_chart_csv", json!({"upload_id": up1}));
    assert_eq!(chart["count"], 3);

    // The IIF is plain ASCII text with CR LF line endings, handed over
    // (D-46); what it writes differently from the books is in warnings
    // (rules.md B-34).
    let iif = mcp.call(
        "books.export",
        json!({"format": "iif", "report": "journal"}),
    );
    let text = &mcp.handed_text(&iif["file"]);
    assert!(text.contains("!TRNS") && text.is_ascii() && text.ends_with("\r\n"));
    assert!(
        text.contains("\r\nACCNT\tCafe Bei Jing\tEXP\t6010\r\n"),
        "{text}"
    );
    assert!(iif.get("content").is_none(), "{iif}");
    assert_eq!(
        iif["file"]["file_name"]
            .as_str()
            .unwrap()
            .rsplit('.')
            .next(),
        Some("iif")
    );
    assert_eq!(
        iif["warnings"],
        json!(["account 6010's name \"Café 北京\" is written \"Cafe Bei Jing\""])
    );

    let xlsx = mcp.call(
        "books.export",
        json!({"format": "xlsx", "report": "trial_balance"}),
    );
    assert_eq!(
        xlsx["file"]["mime_type"],
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
    );
    assert!(mcp.handed(&xlsx["file"]).starts_with(b"PK"), "a zip");
}

#[test]
fn mcp_statement_pdfs_and_cpa_package() {
    let books = Books::new("statement-pdfs");
    let mut mcp = Mcp::spawn(&books);
    mcp.request(
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "smoke-test", "version": "0.1.0"}
        }),
    );
    mcp.notify("notifications/initialized");
    let up2 = mcp.csv_upload("code,name,type,parent_code\n1000,Cash,asset,\n3000,Owner Capital,equity,\n6000,Software,expense,\n");
    mcp.call("books.import_chart_csv", json!({"upload_id": up2}));
    for (desc, debit, credit, amount) in [
        ("Owner contribution", "1000", "3000", "1000.00"),
        ("Domain", "6000", "1000", "12.00"),
    ] {
        let e = mcp.call(
            "books.draft_entry",
            json!({"date": "2026-10-05", "description": desc}),
        );
        let id = e["entry_id"].as_u64().unwrap();
        mcp.call(
            "books.add_line",
            json!({"entry_id": id, "account": debit, "side": "debit", "amount": amount}),
        );
        mcp.call(
            "books.add_line",
            json!({"entry_id": id, "account": credit, "side": "credit", "amount": amount}),
        );
        let up = mcp.upload(desc.as_bytes(), "r.pdf", "application/pdf", "receipt");
        mcp.call(
            "books.attach_receipt",
            json!({"entry_id": id, "upload_id": up}),
        );
        mcp.call(
            "books.post_entry",
            json!({"entry_id": id, "confirmed": true}),
        );
    }

    // All three statements export as PDF.
    for (report, extra) in [
        ("profit_and_loss", json!({})),
        ("balance_sheet", json!({})),
        ("cash_flow", json!({"cash_accounts": ["1000"]})),
        ("trial_balance", json!({"as_of": "2026-10-31"})),
    ] {
        let mut args = json!({"format": "pdf", "report": report});
        args.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        let out = mcp.call("books.export", args);
        assert_eq!(out["file"]["mime_type"], "application/pdf");
        let bytes = mcp.handed(&out["file"]);
        assert!(bytes.starts_with(b"%PDF"), "{report} is not a PDF");
    }
    // The cash flow statement reconciles to named cash accounts: required.
    let err = mcp.call_err(
        "books.export",
        json!({"format": "pdf", "report": "cash_flow"}),
    );
    assert!(err.contains("cash_accounts"), "{err}");
    // The ledger and journal are workbooks, not printable statements.
    let err = mcp.call_err(
        "books.export",
        json!({"format": "pdf", "report": "general_ledger"}),
    );
    assert!(err.contains("not pdf"), "{err}");

    // The CPA package carries all three statements.
    let err = mcp.call_err(
        "books.export_cpa_package",
        json!({"as_of": "2026-10-31", "include_tins": false}),
    );
    assert!(err.contains("cash_accounts"), "{err}");
    // The package's 1099 worksheet is books.vendor_1099's for the
    // package's accounts: the request's, else the settings' (here both
    // name checking).
    mcp.call(
        "books.set_setting",
        json!({"changes": [{"key": "books.cash_accounts", "value": ["1000"]}], "decided_by": "Test Owner"}),
    );
    let pkg = mcp.call(
        "books.export_cpa_package",
        json!({"as_of": "2026-10-31", "cash_accounts": ["1000"], "include_tins": false}),
    );
    assert_eq!(pkg["file"]["file_name"], "cpa-package-2026-10-31.zip");
    assert_eq!(pkg["file"]["mime_type"], "application/zip");
    assert_eq!(pkg["tins_included"], false);
    let zip_bytes = mcp.handed(&pkg["file"]);
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).unwrap();
    let mut names: Vec<String> = (0..zip.len())
        .map(|i| zip.by_index(i).unwrap().name().unwrap().to_string())
        .collect();
    names.sort();
    let (receipts, rest): (Vec<&String>, Vec<&String>) =
        names.iter().partition(|n| n.starts_with("receipts/"));
    assert_eq!(
        rest,
        [
            "1099-worksheet-2026.csv",
            "README.txt",
            "balance-sheet.pdf",
            "balance-sheet.xlsx",
            "cash-flow.pdf",
            "cash-flow.xlsx",
            "general-journal.iif",
            "general-journal.xlsx",
            "general-ledger.xlsx",
            "profit-and-loss.pdf",
            "profit-and-loss.xlsx",
            "tax-summary.pdf",
            "tax-summary.xlsx",
            "trial-balance.pdf",
            "trial-balance.xlsx"
        ]
    );
    // Every receipt behind the period's entries, plus an index tying them
    // to their entries.
    assert!(receipts.iter().any(|n| *n == "receipts/index.csv"));
    let files = receipts.len() - 1;
    assert!(files > 0, "receipts included");
    let mut index = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("receipts/index.csv").unwrap(), &mut index)
        .unwrap();
    assert_eq!(
        index.lines().count(),
        files + 1,
        "one index row per receipt"
    );
    let mut readme = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("README.txt").unwrap(), &mut readme).unwrap();
    assert!(readme.contains("hash-chained journal"), "{readme}");
    let mut cf = Vec::new();
    std::io::Read::read_to_end(&mut zip.by_name("cash-flow.pdf").unwrap(), &mut cf).unwrap();
    assert!(cf.starts_with(b"%PDF"));
    // Without receipts, a lighter package.
    let light = mcp.call(
        "books.export_cpa_package",
        json!({"as_of": "2026-10-31", "cash_accounts": ["1000"], "include_receipts": false, "include_tins": false}),
    );
    let light = zip::ZipArchive::new(std::io::Cursor::new(mcp.handed(&light["file"]))).unwrap();
    assert_eq!(light.len(), rest.len());
}

#[test]
fn mcp_period_statements() {
    // A rejected receipt policy would show up as a failed post below: the
    // settings lower the threshold from 25.00 to 5.00.
    let books = Books::with(
        "period-statements",
        "USD",
        Some(
            "[company]\nname = \"Test Co\"\nfiscal_year_start = \"02-01\"\ntax_classification = \"partnership\"\n[[company.members]]\nname = \"Owner\"\nshare = \"100%\"\ncapital_account = \"3000\"\n[books]\nreceipt_threshold = \"5.00\"\n",
        ),
    );
    let mut mcp = Mcp::spawn(&books);
    mcp.request(
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "smoke-test", "version": "0.1.0"}
        }),
    );
    mcp.notify("notifications/initialized");
    let up3 = mcp.csv_upload("code,name,type,parent_code\n1000,Cash,asset,\n3000,Capital,equity,\n4000,Sales,revenue,\n6000,Software,expense,\n");
    mcp.call("books.import_chart_csv", json!({"upload_id": up3}));
    let post = |mcp: &mut Mcp, date: &str, dr: &str, cr: &str, amount: &str| {
        let e = mcp.call(
            "books.draft_entry",
            json!({"date": date, "description": "x"}),
        );
        let id = e["entry_id"].as_u64().unwrap();
        mcp.call(
            "books.add_line",
            json!({"entry_id": id, "account": dr, "side": "debit", "amount": amount}),
        );
        mcp.call(
            "books.add_line",
            json!({"entry_id": id, "account": cr, "side": "credit", "amount": amount}),
        );
        id
    };
    let id = post(&mut mcp, "2026-01-02", "1000", "3000", "1000.00");
    // 1000.00 > 5.00 (the toml threshold): a receipt is required.
    let err = mcp.call_err(
        "books.post_entry",
        json!({"entry_id": id, "confirmed": true}),
    );
    assert!(err.contains("5.00"), "{err}");
    let up7 = mcp.upload(b"r", "r.pdf", "application/pdf", "receipt");
    mcp.call(
        "books.attach_receipt",
        json!({"entry_id": id, "upload_id": up7}),
    );
    mcp.call(
        "books.post_entry",
        json!({"entry_id": id, "confirmed": true}),
    );
    for (date, dr, cr, amt) in [
        ("2026-01-20", "1000", "4000", "4.00"),
        ("2026-02-05", "6000", "1000", "3.00"),
        ("2026-02-09", "1000", "4000", "2.50"),
    ] {
        let id = post(&mut mcp, date, dr, cr, amt);
        mcp.call(
            "books.post_entry",
            json!({"entry_id": id, "confirmed": true}),
        );
    }

    // February only: 2.50 sales, 3.00 software.
    let feb = mcp.call(
        "books.profit_and_loss",
        json!({"start": "2026-02-01", "end": "2026-02-28"}),
    );
    assert_eq!(feb["revenue"], "2.50");
    assert_eq!(feb["expenses"], "3.00");
    assert_eq!(feb["net_income"], "-0.50");
    assert_eq!(feb["expense_accounts"][0]["code"], "6000");
    assert_eq!(feb["expense_accounts"][0]["amount"], "3.00");
    // No start (rules.md B-30): the fiscal year that holds the end, to
    // it. The fiscal year starts 02-01, so January's 4.00 of sales is
    // last year's, and February's 2.50 is this one's.
    let ytd = mcp.call("books.profit_and_loss", json!({"end": "2026-02-28"}));
    assert_eq!(ytd["start"], "2026-02-01");
    assert_eq!(ytd["revenue"], "2.50");
    let last = mcp.call("books.profit_and_loss", json!({"end": "2026-01-31"}));
    assert_eq!(last["start"], "2025-02-01");
    assert_eq!(last["revenue"], "4.00");

    // As of Jan 31: cash 1,004.00 = capital 1,000.00 + earnings 4.00.
    let jan = mcp.call("books.balance_sheet", json!({"as_of": "2026-01-31"}));
    assert_eq!(jan["assets"], "1004.00");
    assert_eq!(jan["current_earnings"], "4.00");
    assert_eq!(jan["equity_accounts"][0]["amount"], "1000.00");
    assert_eq!(jan["balanced"], true);

    // Trial balance as of Jan 31: Dr cash 1,004.00 = Cr capital + sales.
    let tb = mcp.call("books.trial_balance", json!({"as_of": "2026-01-31"}));
    assert_eq!(tb["as_of"], "2026-01-31");
    assert_eq!(tb["total_debits"], "1004.00");

    // General ledger for February: cash starts at 1,004.00, pays 3.00 for
    // software, takes in 2.50 of sales, ends at 1,003.50.
    let gl = mcp.call(
        "books.export",
        json!({"format": "json", "report": "general_ledger", "start": "2026-02-01", "end": "2026-02-28"}),
    );
    let gl: serde_json::Value = serde_json::from_str(&mcp.handed_text(&gl["file"])).unwrap();
    let cash = &gl["accounts"][0];
    assert_eq!(cash["beginning_balance"], "1004.00");
    assert_eq!(cash["postings"][0]["split"], "6000 Software");
    assert_eq!(cash["postings"][0]["balance"], "1001.00");
    assert_eq!(cash["postings"][1]["split"], "4000 Sales");
    assert_eq!(cash["ending_balance"], "1003.50");

    // Members' equity, Jan-Feb: the one member contributed 1,000.00 and
    // earned 3.50 (6.50 sales - 3.00 software), ending at balance-sheet
    // equity, 1,003.50.
    let me = mcp.call(
        "books.export",
        json!({"format": "json", "report": "members_equity", "start": "2026-01-01", "end": "2026-02-28"}),
    );
    let me: serde_json::Value = serde_json::from_str(&mcp.handed_text(&me["file"])).unwrap();
    assert_eq!(me["members"][0]["name"], "Owner");
    assert_eq!(me["total"]["contributions"], "1000.00");
    assert_eq!(me["total"]["net_income"], "3.50");
    assert_eq!(me["total"]["ending"], "1003.50");
    let pdf = mcp.call(
        "books.export",
        json!({"format": "pdf", "report": "members_equity", "end": "2026-02-28"}),
    );
    assert!(mcp.handed(&pdf["file"]).starts_with(b"%PDF"));

    // Closing January: a one-member LLC closes into the member's capital
    // account, not Retained Earnings. The preview is exactly what posts.
    let jan = mcp.call(
        "books.create_period",
        json!({"name": "2026-01", "start": "2026-01-01", "end": "2026-01-31"}),
    );
    let jan_id = jan["period_id"].as_u64().unwrap();
    let preview = mcp.call(
        "books.close_period",
        json!({"period_id": jan_id, "confirmed": false}),
    );
    assert_eq!(
        preview["lines"],
        json!(["Debit 4000 4.00", "Credit 3000 4.00"])
    );
    let closed = mcp.call(
        "books.close_period",
        json!({"period_id": jan_id, "confirmed": true}),
    );
    assert_eq!(closed["lines"], preview["lines"]);
    let jan_bs = mcp.call("books.balance_sheet", json!({"as_of": "2026-01-31"}));
    assert_eq!(jan_bs["equity_accounts"][0]["amount"], "1004.00");
    assert_eq!(jan_bs["current_earnings"], "0.00");
    // The close is income, not a contribution.
    let me = mcp.call(
        "books.export",
        json!({"format": "json", "report": "members_equity", "start": "2026-01-01", "end": "2026-01-31"}),
    );
    let me: serde_json::Value = serde_json::from_str(&mcp.handed_text(&me["file"])).unwrap();
    assert_eq!(me["total"]["contributions"], "1000.00");
    assert_eq!(me["total"]["net_income"], "4.00");
    assert_eq!(me["total"]["ending"], "1004.00");

    // Tax categories: filtered to the partnership return.
    let cats = mcp.call("books.tax_categories", json!({}));
    assert_eq!(cats["form"], "Form 1065");
    let keys: Vec<&str> = cats["categories"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["key"].as_str().unwrap())
        .collect();
    assert!(keys.contains(&"guaranteed_payments"));
    assert!(
        !keys.contains(&"officer_compensation"),
        "a corporate category"
    );
    mcp.call(
        "books.set_tax_category",
        json!({"account": "4000", "category": "gross_receipts"}),
    );
    mcp.call(
        "books.set_tax_category",
        json!({"account": "6000", "category": "other_deductions"}),
    );
    let err = mcp.call_err(
        "books.set_tax_category",
        json!({"account": "4000", "category": "meals"}),
    );
    assert!(err.contains("expense"), "{err}");
    let chart = mcp.call("books.chart", json!({}));
    let sales = chart["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["code"] == "4000")
        .unwrap()
        .clone();
    assert_eq!(sales["tax_category"], "gross_receipts");
    // Jan-Feb: 6.50 of sales, 3.00 of software, all categorized.
    let ts = mcp.call(
        "books.export",
        json!({"format": "json", "report": "tax_summary", "start": "2026-01-01", "end": "2026-02-28"}),
    );
    let ts: serde_json::Value = serde_json::from_str(&mcp.handed_text(&ts["file"])).unwrap();
    assert_eq!(ts["form"], "Form 1065");
    assert_eq!(ts["income"][0]["book"], "6.50");
    assert_eq!(ts["deductions"][0]["category"], "other_deductions");
    assert_eq!(ts["uncategorized"].as_array().unwrap().len(), 0);
    assert_eq!(ts["tax_net"], "3.50");
    // A chart import validates categories before creating anything.
    let up4 = mcp.csv_upload("code,name,type,parent_code,tax_category\n6100,Meals,expense,,meals\n4100,Fees,revenue,,commissions_fees\n");
    let err = mcp.call_err("books.import_chart_csv", json!({"upload_id": up4}));
    assert!(err.contains("4100"), "{err}");
    let chart = mcp.call("books.chart", json!({}));
    assert!(
        chart["accounts"]
            .as_array()
            .unwrap()
            .iter()
            .all(|a| a["code"] != "6100")
    );
}

#[test]
fn mcp_partnership_from_template() {
    let init = |mcp: &mut Mcp| {
        mcp.request(
            "initialize",
            json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "smoke-test", "version": "0.1.0"},
            }),
        );
        mcp.notify("notifications/initialized");
    };
    // 1. Ask for a template (any server will do).
    let scratch_books = Books::new("template-scratch");
    let mut scratch = Mcp::spawn(&scratch_books);
    init(&mut scratch);
    let t = scratch.call(
        "books.chart_template",
        json!({"entity_type": "multi-member LLC", "owners": ["Ann", "Bo"]}),
    );
    let settings = t["settings"].clone();
    let members = settings
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["key"] == "company.members")
        .unwrap_or_else(|| panic!("{settings}"));
    assert_eq!(
        members["value"][1]["distribution_account"],
        json!("3101"),
        "{settings}"
    );
    let err = scratch.call_err("books.chart_template", json!({"entity_type": "LLC"}));
    assert!(err.contains("entity_type"), "{err}");

    // 2. Its chart becomes the chart; its settings, the company's
    // (books.set_setting, the one way settings change).
    let books = Books::with(
        "template",
        "USD",
        Some("[books]\nreceipt_threshold = \"10000\"\n"),
    );
    let mut mcp = Mcp::spawn(&books);
    init(&mut mcp);
    let up5 = mcp.csv_upload(t["csv"].as_str().unwrap());
    mcp.call("books.import_chart_csv", json!({"upload_id": up5}));
    mcp.call(
        "books.set_setting",
        json!({"changes": settings, "decided_by": "Ann"}),
    );
    let post = |mcp: &mut Mcp, date: &str, dr: &str, cr: &str, amount: &str| {
        let e = mcp.call(
            "books.draft_entry",
            json!({"date": date, "description": "x"}),
        );
        let id = e["entry_id"].as_u64().unwrap();
        mcp.call(
            "books.add_line",
            json!({"entry_id": id, "account": dr, "side": "debit", "amount": amount}),
        );
        mcp.call(
            "books.add_line",
            json!({"entry_id": id, "account": cr, "side": "credit", "amount": amount}),
        );
        mcp.call(
            "books.post_entry",
            json!({"entry_id": id, "confirmed": true}),
        );
    };
    post(&mut mcp, "2026-01-02", "1000", "3000", "1000.00");
    post(&mut mcp, "2026-01-02", "1000", "3001", "1000.00");
    post(&mut mcp, "2026-01-10", "1000", "4000", "400.00");
    post(&mut mcp, "2026-01-20", "3101", "1000", "50.00");

    // 3. Members' equity sees Bo's distribution in his distributions account.
    let me = mcp.call(
        "books.export",
        json!({"format": "json", "report": "members_equity", "start": "2026-01-01", "end": "2026-01-31"}),
    );
    let me: serde_json::Value = serde_json::from_str(&mcp.handed_text(&me["file"])).unwrap();
    assert_eq!(me["members"][1]["distributions"], "50.00");
    assert_eq!(me["members"][1]["ending"], "1150.00");

    // 4. Closing January: 200 each to capital, Bo's distributions swept in.
    let jan = mcp.call(
        "books.create_period",
        json!({"name": "2026-01", "start": "2026-01-01", "end": "2026-01-31"}),
    );
    let closed = mcp.call(
        "books.close_period",
        json!({"period_id": jan["period_id"], "confirmed": true}),
    );
    assert_eq!(
        closed["lines"],
        json!([
            "Debit 4000 400.00",
            "Credit 3000 200.00",
            "Credit 3001 200.00",
            "Credit 3101 50.00",
            "Debit 3001 50.00"
        ])
    );
    let bs = mcp.call("books.balance_sheet", json!({"as_of": "2026-01-31"}));
    let equity: Vec<(String, String)> = bs["equity_accounts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| {
            (
                a["code"].as_str().unwrap().to_string(),
                a["amount"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert_eq!(
        equity,
        [
            ("3000".into(), "1200.00".into()),
            ("3001".into(), "1150.00".into())
        ]
    );
}

/// Contract (rules.md B-13, D-10; docs/mcp-interface.md close_period and
/// list_periods): a close to the members shows each member's running
/// split, the same in the preview as once confirmed, and the periods say
/// which rule they closed under. Two members at 50% each, hand-computed:
/// January's cent is 0.005 each exactly, the tie to Ann (listed first):
/// Ann 0.01, Bo 0.00. February's cent makes the running 0.02, 0.01 each,
/// so February gives Bo 0.01. After a restart, both closes count from
/// close 1, the books verify, and the statement of members' equity shows
/// 0.01 each.
#[test]
fn close_period_shows_each_members_running_split() {
    let scratch_books = Books::new("members-split-scratch");
    let mut scratch = Mcp::spawn(&scratch_books);
    init(&mut scratch);
    let t = scratch.call(
        "books.chart_template",
        json!({"entity_type": "multi-member LLC", "owners": ["Ann", "Bo"]}),
    );
    let books = Books::with(
        "members-split",
        "USD",
        Some("[books]\nreceipt_threshold = \"10000\"\n"),
    );
    let spawn = |books: &Books| {
        let mut mcp = Mcp::spawn(books);
        init(&mut mcp);
        mcp
    };
    let mut mcp = spawn(&books);
    let up6 = mcp.csv_upload(t["csv"].as_str().unwrap());
    mcp.call("books.import_chart_csv", json!({"upload_id": up6}));
    mcp.call(
        "books.set_setting",
        json!({"changes": t["settings"], "decided_by": "Ann"}),
    );
    let sale = |mcp: &mut Mcp, date: &str| {
        let e = mcp.call(
            "books.draft_entry",
            json!({"date": date, "description": "a cent"}),
        );
        let id = e["entry_id"].as_u64().unwrap();
        for (account, side) in [("1000", "debit"), ("4000", "credit")] {
            mcp.call(
                "books.add_line",
                json!({"entry_id": id, "account": account, "side": side, "amount": "0.01"}),
            );
        }
        mcp.call(
            "books.post_entry",
            json!({"entry_id": id, "confirmed": true}),
        );
    };
    let member = |name: &str, code: &str, this: &str, running: &str, exact: &str| {
        json!({"name": name, "capital_account": code, "share": "0.50",
               "this_close": this, "running": running, "exact": exact})
    };

    sale(&mut mcp, "2026-01-10");
    let jan = mcp.call(
        "books.create_period",
        json!({"name": "2026-01", "start": "2026-01-01", "end": "2026-01-31"}),
    );
    let preview = mcp.call(
        "books.close_period",
        json!({"period_id": jan["period_id"], "confirmed": false}),
    );
    assert_eq!(
        preview["members"],
        json!({"since_close": 1, "members": [
            member("Ann", "3000", "0.01", "0.01", "0.005"),
            member("Bo", "3001", "0.00", "0.00", "0.005"),
        ]})
    );
    assert_eq!(
        preview["lines"],
        json!(["Debit 4000 0.01", "Credit 3000 0.01"])
    );
    let closed = mcp.call(
        "books.close_period",
        json!({"period_id": jan["period_id"], "confirmed": true}),
    );
    assert_eq!(closed["members"], preview["members"]);
    assert_eq!(closed["lines"], preview["lines"]);

    sale(&mut mcp, "2026-02-10");
    let feb = mcp.call(
        "books.create_period",
        json!({"name": "2026-02", "start": "2026-02-01", "end": "2026-02-28"}),
    );
    let closed = mcp.call(
        "books.close_period",
        json!({"period_id": feb["period_id"], "confirmed": true}),
    );
    assert_eq!(
        closed["members"],
        json!({"since_close": 1, "members": [
            member("Ann", "3000", "0.00", "0.01", "0.01"),
            member("Bo", "3001", "0.01", "0.01", "0.01"),
        ]})
    );
    assert_eq!(
        closed["lines"],
        json!(["Debit 4000 0.01", "Credit 3001 0.01"])
    );
    drop(mcp);

    books.restart();
    let mut mcp = spawn(&books);
    assert_eq!(mcp.call("books.verify", json!({}))["status"], "ok");
    let periods = mcp.call("books.list_periods", json!({}));
    let rules: Vec<&Value> = periods["periods"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| &p["close_rule"])
        .collect();
    assert_eq!(
        rules,
        [
            &json!("members_cumulative_since:1"),
            &json!("members_cumulative_since:1")
        ]
    );
    let me = mcp.call(
        "books.export",
        json!({"format": "json", "report": "members_equity", "start": "2026-01-01", "end": "2026-02-28"}),
    );
    let me: Value = serde_json::from_str(&mcp.handed_text(&me["file"])).unwrap();
    assert_eq!(me["members"][0]["ending"], "0.01");
    assert_eq!(me["members"][1]["ending"], "0.01");
}

/// Contract (rules.md B-13, D-10; docs/mcp-interface.md list_periods and
/// close_period) on books an earlier version wrote: ledger-store's
/// old-books fixture x_member_closes (two members at 50%, A with a
/// distributions account; three closes untagged, three
/// "largest_remainder", then "cumulative" broken by a close to retained
/// earnings), left in a data root in P2's format. list_periods names each
/// close's rule; December's first close under the rule since P3 counts
/// from close 1 and carries the true-up, refused (naming its feature)
/// until the owner upgrades the books, then the same as previewed.
/// By hand (the fixture's writer): the members closed 0.12, A 0.07 and B
/// 0.05 of it (exact 0.06 each). December earns 0.02: the running 0.14
/// splits 0.07 each, less 0.07 / 0.05 given, so Ann gets 0.00 and Bo
/// 0.02, both at their exact 0.07.
#[test]
fn old_books_name_each_closes_rule_and_true_up_at_the_next_close() {
    // Books P1 kept, which P2 moved to its journal format (format 2, none
    // of the features since), left in a data root, with their members set
    // as an agent sets them: books waiting for their format upgrade.
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../ledger-store/tests/fixtures/old-books/x_member_closes");
    let books = Books::placed("members-old", &fixture);
    books.stop();
    ledger_store::Store::open(&books.dir())
        .unwrap()
        .declare_journal_format(2, &[])
        .unwrap();
    books.start();
    books.settle(
        "[company]\ntax_form = \"1065\"\n\n\
         [[company.members]]\nname = \"Ann\"\nshare = \"50%\"\ncapital_account = \"3000\"\n\
         distribution_account = \"3100\"\n\n\
         [[company.members]]\nname = \"Bo\"\nshare = \"50%\"\ncapital_account = \"3001\"\n\n\
         [books]\nreceipt_threshold = \"10000\"\n",
    );
    let mut mcp = Mcp::spawn(&books);
    init(&mut mcp);
    let rules = |mcp: &mut Mcp| -> Vec<Value> {
        mcp.call("books.list_periods", json!({}))["periods"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["close_rule"].clone())
            .collect()
    };
    let old = [
        "members_last_absorbs",
        "members_last_absorbs",
        "members_last_absorbs",
        "members_per_close",
        "members_per_close",
        "members_per_close",
        "members_run",
        "members_run",
    ];
    let mut want: Vec<Value> = old.iter().map(|r| json!(r)).collect();
    want.extend([Value::Null, json!("members_run"), json!("members_run")]);
    assert_eq!(rules(&mut mcp), want);

    let e = mcp.call(
        "books.draft_entry",
        json!({"date": "2026-12-10", "description": "two cents"}),
    );
    let id = e["entry_id"].as_u64().unwrap();
    for (account, side) in [("1000", "debit"), ("4000", "credit")] {
        mcp.call(
            "books.add_line",
            json!({"entry_id": id, "account": account, "side": side, "amount": "0.02"}),
        );
    }
    mcp.call(
        "books.post_entry",
        json!({"entry_id": id, "confirmed": true}),
    );
    let dec = mcp.call(
        "books.create_period",
        json!({"name": "2026-12", "start": "2026-12-01", "end": "2026-12-31"}),
    );
    let preview = mcp.call(
        "books.close_period",
        json!({"period_id": dec["period_id"], "confirmed": false}),
    );
    let member = |name: &str, code: &str, this: &str| {
        json!({"name": name, "capital_account": code, "share": "0.50",
               "this_close": this, "running": "0.07", "exact": "0.07"})
    };
    assert_eq!(
        preview["members"],
        json!({"since_close": 1, "members": [
            member("Ann", "3000", "0.00"),
            member("Bo", "3001", "0.02"),
        ]})
    );
    assert_eq!(
        preview["lines"],
        json!(["Debit 4000 0.02", "Credit 3001 0.02"])
    );
    // Books an older version wrote: the close counted from its regime's
    // start waits for the owner's go-ahead (D-39), which no tool gives.
    let refused = mcp.call_err(
        "books.close_period",
        json!({"period_id": dec["period_id"], "confirmed": true}),
    );
    assert!(
        refused.starts_with("format_upgrade_pending:") && refused.contains("members_cumulative"),
        "{refused}"
    );
    // The owner's go-ahead, as the app gives it (ledger-gui owner.rs, after
    // the passphrase and a verified backup): the server stopped, the books
    // moved to this version's format, and served again.
    drop(mcp);
    books.stop();
    ledger_store::Store::open(&books.dir())
        .unwrap()
        .upgrade_format()
        .unwrap()
        .expect("old books wait for the upgrade");
    books.start();
    let mut mcp = Mcp::spawn(&books);
    init(&mut mcp);
    let closed = mcp.call(
        "books.close_period",
        json!({"period_id": dec["period_id"], "confirmed": true}),
    );
    assert_eq!(closed["members"], preview["members"]);
    assert_eq!(closed["lines"], preview["lines"]);
    want.push(json!("members_cumulative_since:1"));
    assert_eq!(rules(&mut mcp), want);
    assert_eq!(mcp.call("books.verify", json!({}))["status"], "ok");
}

#[test]
fn mcp_schedules_and_recurring() {
    let books = Books::new("schedules");
    let init = |mcp: &mut Mcp| {
        mcp.request(
            "initialize",
            json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "smoke-test", "version": "0.1.0"},
            }),
        );
        mcp.notify("notifications/initialized");
    };
    let mut mcp = Mcp::spawn(&books);
    init(&mut mcp);
    for (code, name, typ) in [
        ("1000", "Bank", "asset"),
        ("1300", "Prepaid Expenses", "asset"),
        ("6300", "Insurance", "expense"),
        ("6600", "Rent", "expense"),
    ] {
        mcp.call(
            "books.add_account",
            json!({"code": code, "name": name, "account_type": typ}),
        );
    }

    // 1,000.00 of insurance over 3 equal months: 333.33, 333.33, 333.34
    // (an equal-months schedule starts on the 1st, D-37).
    let s = mcp.call(
        "books.create_schedule",
        json!({
            "kind": "prepaid_expense",
            "description": "Liability insurance",
            "total": "1000.00",
            "start": "2026-11-01",
            "months": 3,
            "balance_account": "1300",
            "pl_account": "6300",
            "convention": "monthly_even"
        }),
    );
    assert_eq!(s["convention"], "monthly_even");
    let amounts: Vec<&str> = s["months"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["amount"].as_str().unwrap())
        .collect();
    assert_eq!(amounts, ["333.33", "333.33", "333.34"]);
    let err = mcp.call_err(
        "books.create_schedule",
        json!({
            "kind": "prepaid_expense", "description": "x", "total": "10",
            "start": "2026-11-01", "months": 2,
            "balance_account": "6300", "pl_account": "6300"
        }),
    );
    assert!(err.contains("Asset"), "{err}");

    // Preview posts nothing; confirmed posts November and December.
    let preview = mcp.call("books.run_schedules", json!({"through": "2026-12-31"}));
    assert_eq!(preview["status"], "preview");
    assert_eq!(preview["postings"].as_array().unwrap().len(), 2);
    let pl = mcp.call(
        "books.profit_and_loss",
        json!({"start": "2026-11-01", "end": "2026-12-31"}),
    );
    assert_eq!(pl["expenses"], "0.00");
    let run = mcp.call(
        "books.run_schedules",
        json!({"through": "2026-12-31", "confirmed": true}),
    );
    assert_eq!(run["status"], "posted");
    assert_eq!(run["postings"][1]["date"], "2026-12-31");
    let again = mcp.call(
        "books.run_schedules",
        json!({"through": "2026-12-31", "confirmed": true}),
    );
    assert!(
        again["postings"].as_array().unwrap().is_empty(),
        "idempotent"
    );

    // Monthly rent: drafts only, never posted.
    mcp.call(
        "books.create_recurring",
        json!({
            "description": "Office rent",
            "frequency": "monthly",
            "start": "2026-11-01",
            "lines": [
                {"account": "6600", "side": "debit", "amount": "2000.00"},
                {"account": "1000", "side": "credit", "amount": "2000.00"},
            ]
        }),
    );
    let drafts = mcp.call("books.run_recurring", json!({"through": "2026-12-15"}));
    assert_eq!(drafts["drafts"].as_array().unwrap().len(), 2);
    let pl = mcp.call(
        "books.profit_and_loss",
        json!({"start": "2026-11-01", "end": "2026-12-31"}),
    );
    assert_eq!(pl["expenses"], "666.66", "rent drafts don't post");
    drop(mcp);

    // Restart: everything replays from the journal.
    books.restart();
    let mut mcp = Mcp::spawn(&books);
    init(&mut mcp);
    let list = mcp.call("books.list_schedules", json!({}));
    assert_eq!(list["schedules"][0]["remaining"], "333.34");
    let rec = mcp.call("books.list_recurring", json!({}));
    assert_eq!(
        rec["recurring"][0]["generated"],
        json!(["2026-11-01", "2026-12-01"])
    );
    let more = mcp.call("books.run_recurring", json!({"through": "2026-12-31"}));
    assert!(more["drafts"].as_array().unwrap().is_empty(), "idempotent");
}

#[test]
fn mcp_payout_gross_up() {
    let books = Books::new("payout");
    let mut mcp = Mcp::spawn(&books);
    mcp.request(
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "smoke-test", "version": "0.1.0"}
        }),
    );
    mcp.notify("notifications/initialized");
    for (code, name, typ) in [
        ("1000", "Bank", "asset"),
        ("4000", "App Sales", "revenue"),
        ("4100", "Refunds", "expense"),
        ("6000", "Platform Fees", "expense"),
    ] {
        mcp.call(
            "books.add_account",
            json!({"code": code, "name": name, "account_type": typ}),
        );
    }
    mcp.call("books.add_class", json!({"name": "App A"}));
    mcp.call("books.add_class", json!({"name": "App B"}));

    // 1,000.00 + 500.00 gross, 225.00 commission, a 9.99 refund: 1,265.01.
    let mut payout = json!({
        "date": "2026-11-30",
        "description": "App Store payout, November 2026",
        "deposit_account": "1000",
        "deposit": "1265.00",
        "revenue": [
            {"account": "4000", "amount": "1000.00", "class": "App A"},
            {"account": "4000", "amount": "500.00", "class": "App B"},
        ],
        "fees": [
            {"account": "6000", "amount": "150.00", "class": "App A"},
            {"account": "6000", "amount": "75.00", "class": "App B"},
        ],
        "adjustments": [
            {"account": "4100", "side": "debit", "amount": "9.99", "class": "App A"},
        ],
    });
    let err = mcp.call_err("books.record_payout", payout.clone());
    assert!(err.contains("off by -0.01"), "{err}");
    payout["deposit"] = json!("1265.01");
    let out = mcp.call("books.record_payout", payout);
    assert_eq!(out["status"], "draft");
    assert_eq!(out["gross"], "1500.00");
    assert_eq!(out["adjustments"], "-9.99");
    assert_eq!(
        out["lines"],
        json!([
            "Debit 1000 1265.01",
            "Debit 6000 150.00",
            "Debit 6000 75.00",
            "Debit 4100 9.99",
            "Credit 4000 1000.00",
            "Credit 4000 500.00"
        ])
    );

    // The statement is the receipt; then it posts like any entry.
    let entry_id = out["entry_id"].as_u64().unwrap();
    let up8 = mcp.upload(
        b"%PDF-1.4 statement",
        "payout.pdf",
        "application/pdf",
        "receipt",
    );
    mcp.call(
        "books.attach_receipt",
        json!({
            "entry_id": entry_id,
            "upload_id": up8
        }),
    );
    mcp.call(
        "books.post_entry",
        json!({"entry_id": entry_id, "confirmed": true}),
    );
    let by_class = mcp.call(
        "books.profit_and_loss_by_class",
        json!({"start": "2026-11-01", "end": "2026-11-30"}),
    );
    let net: Vec<(String, String)> = by_class["classes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            (
                c["class"].as_str().unwrap_or("").to_string(),
                c["net_income"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert_eq!(
        net,
        [
            ("App A".into(), "840.01".into()),
            ("App B".into(), "425.00".into())
        ]
    );
}

#[test]
fn mcp_vendor_1099() {
    let books = Books::new("form1099");
    let init = |mcp: &mut Mcp| {
        mcp.request(
            "initialize",
            json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "smoke-test", "version": "0.1.0"},
            }),
        );
        mcp.notify("notifications/initialized");
    };
    let mut mcp = Mcp::spawn(&books);
    init(&mut mcp);
    for (code, name, typ) in [
        ("1000", "Bank", "asset"),
        ("6000", "Contract Labor", "expense"),
    ] {
        mcp.call(
            "books.add_account",
            json!({"code": code, "name": name, "account_type": typ}),
        );
    }
    // The worksheet counts payments out of the books' cash accounts.
    mcp.call(
        "books.set_setting",
        json!({"changes": [{"key": "books.cash_accounts", "value": ["1000"]}], "decided_by": "Test Owner"}),
    );
    mcp.call("books.add_vendor", json!({"name": "Dana Designs"}));
    let v = mcp.call(
        "books.set_vendor_1099",
        json!({"vendor": "dana designs", "form_1099": "nec", "decided_by": "Test Owner"}),
    );
    assert_eq!(v["form_1099"], "1099-NEC");
    assert_eq!(v["decision_1099"]["decided_by"], "Test Owner");
    let err = mcp.call_err(
        "books.set_vendor_1099",
        json!({"vendor": "Dana Designs", "form_1099": "k", "decided_by": "Test Owner"}),
    );
    assert!(
        err.contains("expected nec, misc, not_reportable, or undetermined"),
        "{err}"
    );

    // A 2,500.00 check to Dana, tagged at draft time.
    let e = mcp.call(
        "books.draft_entry",
        json!({"date": "2026-05-01", "description": "Logo", "vendor": "Dana Designs"}),
    );
    let id = e["entry_id"].as_u64().unwrap();
    for (account, side) in [("6000", "debit"), ("1000", "credit")] {
        mcp.call(
            "books.add_line",
            json!({"entry_id": id, "account": account, "side": side, "amount": "2500.00"}),
        );
    }
    let up9 = mcp.upload(
        b"%PDF-1.4 invoice",
        "invoice.pdf",
        "application/pdf",
        "receipt",
    );
    mcp.call(
        "books.attach_receipt",
        json!({
            "entry_id": id,
            "upload_id": up9
        }),
    );
    mcp.call(
        "books.post_entry",
        json!({"entry_id": id, "confirmed": true}),
    );
    let got = mcp.call("books.get_entry", json!({"entry_id": id}));
    assert_eq!(got["vendor"], "Dana Designs");

    let r = mcp.call("books.vendor_1099", json!({"year": 2026}));
    assert_eq!(r["threshold"], "2000.00");
    assert_eq!(r["vendors"][0]["paid"], "2500.00");
    assert_eq!(r["vendors"][0]["reportable"], true);
    assert_eq!(r["missing_w9"], json!(["Dana Designs"]));
    // An override above what was paid: not reportable.
    let high = mcp.call(
        "books.vendor_1099",
        json!({"year": 2026, "threshold": "3000"}),
    );
    assert_eq!(high["vendors"][0]["reportable"], false);
    // W-9 arrives: the flag alone, the decision kept.
    mcp.call(
        "books.set_vendor_1099",
        json!({"vendor": "Dana Designs", "tin_on_file": true}),
    );
    drop(mcp);

    books.restart();
    let mut mcp = Mcp::spawn(&books);
    init(&mut mcp);
    let r = mcp.call("books.vendor_1099", json!({"year": 2026}));
    assert_eq!(r["vendors"][0]["paid"], "2500.00");
    assert_eq!(r["missing_w9"], json!([]));
    // Clearing the tag removes the payment.
    mcp.call("books.set_entry_vendor", json!({"entry_id": id}));
    let r = mcp.call("books.vendor_1099", json!({"year": 2026}));
    assert_eq!(r["vendors"][0]["paid"], "0.00");
    // A year whose threshold isn't known yet: refused, with what to do
    // (rules.md B-19). An explicit threshold works.
    let why = mcp.call_err("books.vendor_1099", json!({"year": 2027}));
    for needle in [
        "only through 2026",
        "official source",
        "irs.gov",
        "tell the human",
    ] {
        assert!(why.contains(needle), "missing {needle:?} in: {why}");
    }
    let r = mcp.call(
        "books.vendor_1099",
        json!({"year": 2027, "threshold": "2100"}),
    );
    assert_eq!(r["threshold"], "2100.00");
    let vendors = mcp.call("books.list_vendors", json!({}));
    assert_eq!(vendors["vendors"][0]["tin_on_file"], true);
    // The server made the books' ops/ as they opened: a viewer open from
    // before the first books.show is seen by it.
    let viewer = ledger_store::ops::Presence::hold(&books.dir()).expect("ops/ made at start");
    // Show the entry in the viewer: a request in the books' ops/, never
    // journaled.
    let shown = mcp.call("books.show", json!({"entry_id": id}));
    assert!(shown["shown"].as_str().unwrap().contains(&format!("#{id}")));
    assert_eq!(shown["viewer_open"], true);
    let cmd = ledger_store::ops::read(&books.dir()).expect("request written");
    assert_eq!(
        cmd.target,
        ledger_store::ops::ViewerTarget::Entry { entry_id: id }
    );
    // With no viewer, the note says to open one.
    drop(viewer);
    let shown = mcp.call("books.show", json!({"page": "cash"}));
    assert_eq!(shown["viewer_open"], false);
    assert!(shown["note"].as_str().unwrap().contains("No viewer"));
    let err = mcp.call_err("books.show", json!({"entry_id": 9999}));
    assert!(err.contains("unknown entry"), "{err}");
    let err = mcp.call_err("books.show", json!({"page": "cash", "search": "x"}));
    assert!(err.contains("exactly one"), "{err}");
    // The books check out end to end, receipt included.
    let v = mcp.call("books.verify", json!({}));
    assert_eq!(v["status"], "ok");
    assert_eq!(v["blobs_verified"], 1);
    assert!(v["events"].as_u64().unwrap() > 10);
}

#[test]
fn mcp_receipts_always_required_when_threshold_is_zero() {
    let books = Books::with(
        "always-receipts",
        "USD",
        Some("[books]\nreceipt_threshold = \"0.00\"\n"),
    );
    let mut mcp = Mcp::spawn(&books);
    let init = mcp.request(
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "smoke-test", "version": "0.1.0"}
        }),
    );
    let instructions = init["result"]["instructions"].as_str().unwrap();
    assert!(
        instructions.contains("every entry needs at least one receipt"),
        "the agent is told the real policy: {instructions}"
    );
    assert!(!instructions.contains("$25.00"));
    mcp.notify("notifications/initialized");
    for (code, name, typ) in [("1000", "Bank", "asset"), ("6000", "Fees", "expense")] {
        mcp.call(
            "books.add_account",
            json!({"code": code, "name": name, "account_type": typ}),
        );
    }
    let e = mcp.call(
        "books.draft_entry",
        json!({"date": "2026-10-01", "description": "Bank fee"}),
    );
    let id = e["entry_id"].as_u64().unwrap();
    for (account, side) in [("6000", "debit"), ("1000", "credit")] {
        mcp.call(
            "books.add_line",
            json!({"entry_id": id, "account": account, "side": side, "amount": "5.00"}),
        );
    }
    let err = mcp.call_err(
        "books.post_entry",
        json!({"entry_id": id, "confirmed": true}),
    );
    assert!(
        err.contains("receipt"),
        "a $5 entry needs a receipt too: {err}"
    );
}

/// `books.cash_flow` with no accounts passed reads the roles from
/// `[books]` in wanderbooks.toml (rules.md B-14): the expense the owners
/// paid through 2100 is borrowed from them and spent on its date, each
/// line with `on_behalf` (D-42, rule 11); repaying the advance is
/// financing; a loan to a member is investing; the statement is the direct
/// method's, and it foots.
#[test]
fn cash_flow_reads_its_account_roles_from_the_books_settings() {
    let books = Books::with(
        "cash-roles",
        "USD",
        Some(
            "[books]\nreceipt_threshold = \"1000000.00\"\ncash_accounts = [\"1000\"]\nfinancing_accounts = [\"2100\"]\ninvesting_accounts = [\"1400\"]\n",
        ),
    );
    let mut mcp = Mcp::spawn(&books);
    mcp.request(
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "smoke-test", "version": "0.1.0"}
        }),
    );
    mcp.notify("notifications/initialized");
    for (code, name, typ) in [
        ("1000", "Bank", "asset"),
        ("1400", "Loan to member", "asset"),
        ("2100", "Due to owners", "liability"),
        ("3000", "Capital", "equity"),
        ("6000", "Software", "expense"),
    ] {
        mcp.call(
            "books.add_account",
            json!({"code": code, "name": name, "account_type": typ}),
        );
    }
    let mut post = |date: &str, dr: &str, cr: &str, amount: &str| {
        let e = mcp.call(
            "books.draft_entry",
            json!({"date": date, "description": "test"}),
        );
        let id = e["entry_id"].as_u64().unwrap();
        for (account, side) in [(dr, "debit"), (cr, "credit")] {
            mcp.call(
                "books.add_line",
                json!({"entry_id": id, "account": account, "side": side, "amount": amount}),
            );
        }
        mcp.call(
            "books.post_entry",
            json!({"entry_id": id, "confirmed": true}),
        );
    };
    post("2026-03-01", "1000", "3000", "1000.00");
    post("2026-03-05", "6000", "2100", "100.00");
    post("2026-03-10", "1400", "1000", "500.00");
    post("2026-03-28", "2100", "1000", "100.00");
    let cf = mcp.call(
        "books.cash_flow",
        json!({"start": "2026-03-01", "end": "2026-03-31"}),
    );
    let amounts = |section: &str| -> Vec<String> {
        cf[section]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| l["amount"].as_str().unwrap().to_string())
            .collect()
    };
    // Financing by kind: the contribution, what the owners paid for the
    // company (borrowed through 2100), then the repayment of 2100.
    assert_eq!(amounts("financing"), ["1000.00", "100.00", "-100.00"]);
    assert_eq!(cf["financing"][1]["entry"], 2);
    assert_eq!(cf["financing"][1]["on_behalf"], "100.00");
    assert!(cf["financing"][0].get("on_behalf").is_none());
    let kinds: Vec<&str> = cf["financing_by_kind"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| k["kind"].as_str().unwrap())
        .collect();
    assert_eq!(
        kinds,
        [
            "Contributions from owners — 3000 Capital",
            "Proceeds from borrowing — 2100 Due to owners",
            "Repayments of borrowing — 2100 Due to owners"
        ]
    );
    assert_eq!(amounts("investing"), ["-500.00"]);
    assert_eq!(amounts("operating"), ["-100.00"]);
    assert_eq!(cf["operating"][0]["on_behalf"], "-100.00");
    assert_eq!(cf["operating"][0]["class"], "to_suppliers_and_employees");
    assert_eq!(cf["method"], "direct");
    assert_eq!(cf["net_change_in_cash"], "400.00");
    assert_eq!(cf["reconciles"]["sections_sum"], "400.00");
    assert_eq!(cf["reconciles"]["difference"], "0.00");
    // Nothing noncash: the software is net income's only item.
    assert_eq!(cf["noncash_disclosures"], json!([]));
    let r = &cf["operating_reconciliation"];
    assert_eq!(r["net_income"], "-100.00");
    assert_eq!(r["lines"], json!([]));
    assert_eq!(r["net_cash_from_operating"], "-100.00");
    for gone in ["net_income", "depreciation", "working_capital"] {
        assert!(cf.get(gone).is_none(), "{gone} is gone");
    }
}

/// rules.md B-14 rule 2: roles that contradict the chart are refused,
/// naming the setting and the fix, never a statement on a wrong partition.
#[test]
fn cash_flow_refuses_contradictory_roles() {
    let books = Books::new("cash-refuse");
    let mut mcp = Mcp::spawn(&books);
    mcp.request(
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "smoke-test", "version": "0.1.0"}
        }),
    );
    mcp.notify("notifications/initialized");
    for (code, name, typ) in [
        ("1000", "Bank", "asset"),
        ("1500", "Equipment", "asset"),
        ("2100", "Due to owners", "liability"),
    ] {
        mcp.call(
            "books.add_account",
            json!({"code": code, "name": name, "account_type": typ}),
        );
    }
    let err = mcp.call_err(
        "books.cash_flow",
        json!({"cash_accounts": ["1000"], "financing_accounts": ["1500"]}),
    );
    assert!(
        err.contains("financing_accounts (from the call or books.financing_accounts) lists 1500"),
        "{err}"
    );
    assert!(err.contains("financing accounts are liabilities"), "{err}");
    let err = mcp.call_err(
        "books.cash_flow",
        json!({"cash_accounts": ["1000", "2100"]}),
    );
    assert!(err.contains("cash accounts are asset accounts"), "{err}");
    let err = mcp.call_err(
        "books.cash_flow",
        json!({"cash_accounts": ["1000"], "investing_accounts": ["1000"]}),
    );
    assert!(err.contains("is in both cash_accounts"), "{err}");
}

/// A chart CSV with a bad row anywhere is refused whole: nothing is
/// half-imported (the old checks missed duplicates within the file and a
/// parent listed after its child).
#[test]
fn a_chart_file_with_a_bad_row_imports_nothing() {
    let books = Books::new("chart-atomic");
    let mut mcp = Mcp::spawn(&books);
    mcp.request(
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "smoke-test", "version": "0.1.0"}
        }),
    );
    mcp.notify("notifications/initialized");
    let count = |mcp: &mut Mcp| {
        mcp.call("books.chart", json!({}))["accounts"]
            .as_array()
            .unwrap()
            .len()
    };
    let before = count(&mut mcp);
    for bad in [
        // A duplicate code within the file.
        "code,name,type,parent_code\n1000,Cash,asset,\n1010,Savings,asset,\n1000,Again,asset,\n",
        // A parent listed after its child.
        "code,name,type,parent_code\n1000,Cash,asset,\n1011,Sub,asset,1010\n1010,Bank,asset,\n",
        // A child of another type than its parent.
        "code,name,type,parent_code\n1000,Cash,asset,\n4000,Sales,revenue,1000\n",
    ] {
        let up7 = mcp.csv_upload(bad);
        let err = mcp.call_err("books.import_chart_csv", json!({"upload_id": up7}));
        assert!(!err.is_empty());
        assert_eq!(count(&mut mcp), before, "half-imported: {bad}");
    }
    let up8 =
        mcp.csv_upload("code,name,type,parent_code\n1000,Cash,asset,\n1010,Checking,asset,1000\n");
    let ok = mcp.call("books.import_chart_csv", json!({"upload_id": up8}));
    assert_eq!(ok["count"], 2);
}

/// Bank CSV import: slash dates that read either way need the caller's
/// order; overlapping rules pick the longest pattern every time; and a
/// transaction that can't become a draft refuses the whole file.
#[test]
fn bank_import_reads_dates_rules_and_files_whole() {
    let books = Books::new("bank-import");
    let mut mcp = Mcp::spawn(&books);
    mcp.request(
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "smoke-test", "version": "0.1.0"}
        }),
    );
    mcp.notify("notifications/initialized");
    for (code, name, typ) in [
        ("1000", "Bank", "asset"),
        ("6000", "Other", "expense"),
        ("6100", "Coffee", "expense"),
        ("6200", "Coffee beans", "expense"),
    ] {
        mcp.call(
            "books.add_account",
            json!({"code": code, "name": name, "account_type": typ}),
        );
    }
    // The CSV's upload: one refused is left for the next call.
    let base = |upload: &str| {
        json!({
            "upload_id": upload,
            "bank_account_code": "1000",
            "default_offset_code": "6000",
            "rules": {"coffee": "6100", "coffee beans": "6200"},
        })
    };
    // 3/4/2026 reads both ways and nothing in the file settles it.
    let ambiguous = "Date,Description,Amount\n3/4/2026,Coffee shop,-4.50\n";
    let up = mcp.csv_upload(ambiguous);
    let err = mcp.call_err("books.import_bank_csv", base(&up));
    assert!(err.contains("either way"), "{err}");
    let mut args = base(&up);
    args["date_order"] = json!("day_first");
    let out = mcp.call("books.import_bank_csv", args);
    assert_eq!(out["imported"][0]["date"], "2026-04-03");
    // Both patterns match "Coffee beans": the longer wins, every time.
    for _ in 0..5 {
        let up = mcp.csv_upload("Date,Description,Amount\n2026-05-01,Coffee beans 2kg,-30.00\n");
        let out = mcp.call("books.import_bank_csv", base(&up));
        assert_eq!(out["imported"][0]["offset_account"], "6200");
    }
    // A transaction dated in a closed period refuses the file: nothing of
    // it becomes a draft.
    let p = mcp.call(
        "books.create_period",
        json!({"name": "2026-01", "start": "2026-01-01", "end": "2026-01-31"}),
    );
    mcp.call(
        "books.close_period",
        json!({"period_id": p["period_id"], "confirmed": true}),
    );
    let up = mcp.csv_upload(
        "Date,Description,Amount\n2026-06-01,Fine,-1.00\n2026-01-15,In a closed month,-2.00\n",
    );
    let err = mcp.call_err("books.import_bank_csv", base(&up));
    assert!(err.contains("closed"), "{err}");
    let journal = String::from_utf8(books.journal()).unwrap();
    assert!(
        !journal.contains("2026-06-01"),
        "the first row was journaled"
    );
}

/// books.renumber_account moves an account off a control code: payroll then
/// gets its own wages account, the renumbered account keeps its entries,
/// and settings naming the old code are listed.
#[test]
fn renumbering_an_account_keeps_its_history() {
    let books = Books::with(
        "renumber",
        "USD",
        Some("[books]\nreceipt_threshold = \"1000000.00\"\ncash_accounts = [\"1000\"]\n"),
    );
    let mut mcp = Mcp::spawn(&books);
    mcp.request(
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "smoke-test", "version": "0.1.0"}
        }),
    );
    mcp.notify("notifications/initialized");
    for (code, name, typ) in [
        ("1000", "Bank", "asset"),
        ("2100", "Due to owners", "liability"),
        ("6100", "Domain & Hosting", "expense"),
    ] {
        mcp.call(
            "books.add_account",
            json!({"code": code, "name": name, "account_type": typ}),
        );
    }
    let e = mcp.call(
        "books.draft_entry",
        json!({"date": "2026-03-05", "description": "domain"}),
    );
    let id = e["entry_id"].as_u64().unwrap();
    for (account, side) in [("6100", "debit"), ("2100", "credit")] {
        mcp.call(
            "books.add_line",
            json!({"entry_id": id, "account": account, "side": side, "amount": "12.34"}),
        );
    }
    mcp.call(
        "books.post_entry",
        json!({"entry_id": id, "confirmed": true}),
    );

    // Refusals: a code in use; a control code of another type (2110 is
    // payroll's federal withholding, a liability).
    let err = mcp.call_err(
        "books.renumber_account",
        json!({"account": "6100", "code": "1000"}),
    );
    assert!(err.contains("already in use"), "{err}");
    let err = mcp.call_err(
        "books.renumber_account",
        json!({"account": "6100", "code": "2110"}),
    );
    assert!(err.contains("2110"), "{err}");

    let out = mcp.call(
        "books.renumber_account",
        json!({"account": "6100", "code": "6050"}),
    );
    assert_eq!(out["previous_code"], "6100");
    assert_eq!(out["code"], "6050");
    assert_eq!(out["config_mentions"], json!([]));
    let chart = mcp.call("books.chart", json!({}));
    let acct = chart["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["code"] == "6050")
        .expect("renumbered");
    assert_eq!(acct["name"], "Domain & Hosting");
    assert_eq!(acct["balance"], "12.34");
    assert!(
        !chart["accounts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["code"] == "6100")
    );
    // The cash account is named in the books' settings: listed.
    let out = mcp.call(
        "books.renumber_account",
        json!({"account": "1000", "code": "1010"}),
    );
    assert_eq!(out["config_mentions"], json!(["books.cash_accounts"]));
    drop(mcp);

    // A restart replays the journal to the same chart.
    books.restart();
    let mut mcp = Mcp::spawn(&books);
    mcp.request(
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "smoke-test", "version": "0.1.0"}
        }),
    );
    mcp.notify("notifications/initialized");
    let chart = mcp.call("books.chart", json!({}));
    let codes: Vec<&str> = chart["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["code"].as_str().unwrap())
        .collect();
    assert!(
        codes.contains(&"6050") && codes.contains(&"1010"),
        "{codes:?}"
    );
    assert!(
        !codes.contains(&"6100") && !codes.contains(&"1000"),
        "{codes:?}"
    );
}

/// A session with `books`, settings set from `toml` first, initialized.
fn served(books: &Books, toml: &str) -> Mcp {
    books.settle(toml);
    let mut mcp = Mcp::spawn(books);
    init(&mut mcp);
    mcp
}

/// Post one balanced two-line entry.
fn post_pair(mcp: &mut Mcp, date: &str, dr: &str, cr: &str, amount: &str) -> u64 {
    let e = mcp.call(
        "books.draft_entry",
        json!({"date": date, "description": "x", "no_party_reason": "test"}),
    );
    let id = e["entry_id"].as_u64().unwrap();
    for (account, side) in [(dr, "debit"), (cr, "credit")] {
        mcp.call(
            "books.add_line",
            json!({"entry_id": id, "account": account, "side": side, "amount": amount}),
        );
    }
    mcp.call(
        "books.post_entry",
        json!({"entry_id": id, "confirmed": true}),
    );
    id
}

/// Defect 16 (rules.md B-30): one default period everywhere. On books with
/// a July fiscal year and a first entry in the fiscal year before, every
/// period report given only an end covers the fiscal year that holds it
/// (2025-07-01 through 2026-03-31, never the first entry's day, never
/// 0001-01-01): profit and loss, by class, cash flow, and the exported tax
/// summary, members' equity, general ledger and general journal (csv and
/// json, whose entries are the period's); the CPA package as of that day
/// covers the same. With no dates at all, every one of them returns the
/// same pair, today's fiscal year to date (the clock read before and
/// after, so midnight can't fail it).
#[test]
fn one_default_period_everywhere_defect_16() {
    let books = Books::new("one-default");
    let mut mcp = served(
        &books,
        "[company]\nname = \"Fernhill Labs LLC\"\nfiscal_year_start = \"07-01\"\ntax_classification = \"partnership\"\n[[company.members]]\nname = \"A\"\nshare = \"50%\"\ncapital_account = \"3000\"\n[[company.members]]\nname = \"B\"\nshare = \"50%\"\ncapital_account = \"3001\"\n[books]\nreceipt_threshold = \"1000000.00\"\ncash_accounts = [\"1000\"]\n",
    );
    let up9 = mcp.csv_upload("code,name,type,parent_code,tax_category\n1000,Cash,asset,,\n3000,Capital A,equity,,\n3001,Capital B,equity,,\n4000,Sales,revenue,,gross_receipts\n6000,Software,expense,,other_deductions\n");
    mcp.call("books.import_chart_csv", json!({"upload_id": up9}));
    let first = post_pair(&mut mcp, "2025-03-15", "1000", "3000", "500.00");
    let mid = post_pair(&mut mcp, "2025-09-10", "1000", "4000", "120.00");
    let late = post_pair(&mut mcp, "2026-02-01", "6000", "1000", "20.00");
    let _after = post_pair(&mut mcp, "2026-05-01", "1000", "4000", "7.00");
    let end = json!({"end": "2026-03-31"});
    let pair = |v: &Value| {
        (
            v["start"].as_str().unwrap().to_string(),
            v["end"].as_str().unwrap().to_string(),
        )
    };
    let want = ("2025-07-01".to_string(), "2026-03-31".to_string());
    let pl = mcp.call("books.profit_and_loss", end.clone());
    assert_eq!(pair(&pl), want);
    assert_eq!(pl["revenue"], "120.00");
    assert_eq!(pl["expenses"], "20.00");
    assert_eq!(
        pair(&mcp.call("books.profit_and_loss_by_class", end.clone())),
        want
    );
    assert_eq!(pair(&mcp.call("books.cash_flow", end.clone())), want);
    for report in ["tax_summary", "members_equity", "general_ledger"] {
        let out = mcp.call(
            "books.export",
            json!({"report": report, "format": "json", "end": "2026-03-31"}),
        );
        let v: Value = serde_json::from_str(&mcp.handed_text(&out["file"])).unwrap();
        assert_eq!(pair(&v), want, "{report}");
    }
    // The general journal: the period's entries, in json and csv alike.
    let out = mcp.call(
        "books.export",
        json!({"report": "general_journal", "format": "json", "end": "2026-03-31"}),
    );
    let v: Value = serde_json::from_str(&mcp.handed_text(&out["file"])).unwrap();
    let ids: Vec<u64> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["entry_id"].as_u64().unwrap())
        .collect();
    assert_eq!(ids, [mid, late], "not {first}, not after the end");
    let csv = mcp.call(
        "books.export",
        json!({"report": "journal", "format": "csv", "end": "2026-03-31"}),
    );
    let rows = mcp.handed_text(&csv["file"]).lines().count();
    assert_eq!(rows, 1 + 4, "header and two entries of two lines");
    // An inverted range is refused by the journal too (review R1).
    let err = mcp.call_err(
        "books.export",
        json!({"report": "general_journal", "format": "json", "start": "2026-12-31", "end": "2026-01-01"}),
    );
    assert!(err.contains("is before start"), "{err}");
    // The CPA package as of that day: the same fiscal year to date.
    let pkg = mcp.call(
        "books.export_cpa_package",
        json!({"as_of": "2026-03-31", "include_receipts": false, "include_tins": false}),
    );
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(mcp.handed(&pkg["file"]))).unwrap();
    let mut readme = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("README.txt").unwrap(), &mut readme).unwrap();
    assert!(
        readme.contains("Fiscal year to date: 2025-07-01 through 2026-03-31."),
        "{readme}"
    );
    // No dates: the same pair from every period report.
    let today = || ledger_core::settings::today_in(None);
    let before = today();
    let mut got = vec![
        pair(&mcp.call("books.profit_and_loss", json!({}))),
        pair(&mcp.call("books.profit_and_loss_by_class", json!({}))),
        pair(&mcp.call("books.cash_flow", json!({}))),
    ];
    for report in ["tax_summary", "members_equity", "general_ledger"] {
        let out = mcp.call("books.export", json!({"report": report, "format": "json"}));
        let v: Value = serde_json::from_str(&mcp.handed_text(&out["file"])).unwrap();
        got.push(pair(&v));
    }
    let after = today();
    let fy = |t: chrono::NaiveDate| {
        let c = ledger_core::settings::Company {
            fiscal_year_start: (7, 1),
            ..Default::default()
        };
        let (s, _) = ledger_core::calendar::period_defaults(&c, t).statement_range();
        (s.to_string(), t.to_string())
    };
    for p in &got {
        assert!(*p == fy(before) || *p == fy(after), "{p:?}");
    }
    assert!(
        got.iter().all(|p| p == &got[0]) || before != after,
        "{got:?}"
    );
}

/// The entry ids (DOCNUM) of an IIF file's transactions, in file order.
fn iif_docnums(iif: &str) -> Vec<u64> {
    let mut docnum = None;
    let mut ids = Vec::new();
    for line in iif.split("\r\n") {
        let f: Vec<&str> = line.split('\t').collect();
        match f[0] {
            "!TRNS" => docnum = f.iter().position(|c| *c == "DOCNUM"),
            "TRNS" => ids.push(f[docnum.expect("!TRNS before TRNS")].parse().unwrap()),
            _ => {}
        }
    }
    ids
}

/// The default period (rules.md B-30) against the IIF (B-34, D-36) and
/// the CPA package (B-33): a general journal asked for without dates is
/// the fiscal year to date, but the IIF is never a period. Without dates
/// it holds every posted entry from the first through today, under
/// either name of the report, and a start or end is refused; the
/// package's IIF holds every entry from the first through its as_of, the
/// one before its fiscal year included.
#[test]
fn the_default_period_never_shortens_the_iif_or_the_package() {
    let books = Books::new("default-vs-iif");
    let mut mcp = served(
        &books,
        "[company]\nname = \"Fernhill Labs LLC\"\nfiscal_year_start = \"07-01\"\n[books]\nreceipt_threshold = \"1000000.00\"\ncash_accounts = [\"1000\"]\n",
    );
    let up10 = mcp.csv_upload("code,name,type,parent_code,tax_category\n1000,Cash,asset,,\n3000,Capital,equity,,\n4000,Sales,revenue,,gross_receipts\n6000,Software,expense,,other_deductions\n");
    mcp.call("books.import_chart_csv", json!({"upload_id": up10}));
    let entries = [
        ("2025-03-15", "1000", "3000", "500.00"),
        ("2025-09-10", "1000", "4000", "120.00"),
        ("2026-02-01", "6000", "1000", "20.00"),
        ("2026-05-01", "1000", "4000", "7.00"),
    ]
    .map(|(date, dr, cr, amount)| (date, post_pair(&mut mcp, date, dr, cr, amount)));
    let every: Vec<u64> = entries.iter().map(|(_, id)| *id).collect();
    let today = || ledger_core::settings::today_in(None);
    let before = today();
    // The general journal without dates: the fiscal year to date.
    let out = mcp.call(
        "books.export",
        json!({"report": "general_journal", "format": "json"}),
    );
    let after = today();
    let v: Value = serde_json::from_str(&mcp.handed_text(&out["file"])).unwrap();
    let journal: Vec<u64> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["entry_id"].as_u64().unwrap())
        .collect();
    let fytd = |t: chrono::NaiveDate| -> Vec<u64> {
        let c = ledger_core::settings::Company {
            fiscal_year_start: (7, 1),
            ..Default::default()
        };
        let (start, end) = ledger_core::calendar::period_defaults(&c, t).statement_range();
        entries
            .iter()
            .filter(|(date, _)| {
                let d = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap();
                start <= d && d <= end
            })
            .map(|(_, id)| *id)
            .collect()
    };
    assert!(
        journal == fytd(before) || journal == fytd(after),
        "{journal:?}"
    );
    assert_ne!(journal, every, "the journal's default is a period");
    // The IIF without dates: every posted entry through today, by either
    // name of the report.
    for report in ["general_journal", "journal", "GENERAL_JOURNAL"] {
        let iif = mcp.call("books.export", json!({"report": report, "format": "iif"}));
        assert_eq!(iif["report"], report.to_lowercase());
        assert_eq!(
            iif_docnums(&mcp.handed_text(&iif["file"])),
            every,
            "{report}"
        );
    }
    for dates in [json!({"start": "2025-07-01"}), json!({"end": "2026-03-31"})] {
        let mut args = json!({"report": "general_journal", "format": "iif"});
        args.as_object_mut()
            .unwrap()
            .extend(dates.as_object().unwrap().clone());
        let err = mcp.call_err("books.export", args);
        assert!(err.contains("iif takes as_of, not start or end"), "{err}");
    }
    // The package as of 2026-03-31: its statements' fiscal year starts
    // 2025-07-01, and its IIF still holds the entry before it.
    let pkg = mcp.call(
        "books.export_cpa_package",
        json!({"as_of": "2026-03-31", "include_receipts": false, "include_tins": false}),
    );
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(mcp.handed(&pkg["file"]))).unwrap();
    let mut read = |name: &str| {
        let mut text = String::new();
        std::io::Read::read_to_string(&mut zip.by_name(name).unwrap(), &mut text).unwrap();
        text
    };
    let readme = read("README.txt");
    assert!(
        readme.contains("Fiscal year to date: 2025-07-01 through 2026-03-31."),
        "{readme}"
    );
    assert_eq!(iif_docnums(&read("general-journal.iif")), every[..3]);
}

/// Defect 15 (rules.md B-31) over the real binary: books.rename_account
/// gives an account a new name and nothing else. The chart shows it; a
/// past period's P&L export prints it; the general journal's IIF declares
/// the account by it; the books verify, the journal grew by the rename
/// and its feature's declaration only; refusals (a blank name, the name
/// it has, a sibling's ignoring capitals, nine digits) write nothing; and
/// a restarted server reads the new name back.
#[test]
fn rename_account_end_to_end_defect_15() {
    let books = Books::new("rename-e2e");
    let mut mcp = served(
        &books,
        "[company]\nname = \"Fernhill Labs LLC\"\n[books]\nreceipt_threshold = \"1000000.00\"\ncash_accounts = [\"1000\"]\n",
    );
    let up11 = mcp.csv_upload("code,name,type,parent_code,tax_category\n1000,Business Checking,asset,,\n3000,Capital,equity,,\n6000,Operating,expense,,other_deductions\n6150,Software,expense,6000,other_deductions\n6160,Hosting,expense,6000,other_deductions\n");
    mcp.call("books.import_chart_csv", json!({"upload_id": up11}));
    post_pair(&mut mcp, "2025-11-04", "1000", "3000", "800.00");
    post_pair(&mut mcp, "2025-11-20", "6150", "1000", "29.00");
    let verified = mcp.call("books.verify", json!({"blobs": false}));
    let events = verified["events"].as_u64().unwrap();
    for (name, says) in [
        ("", "must not be empty"),
        (" Software ", "already named"),
        ("HOSTING", "same parent"),
        ("Card 1234 5678 9", "nine digits"),
    ] {
        let err = mcp.call_err(
            "books.rename_account",
            json!({"account": "6150", "name": name}),
        );
        assert!(err.contains(says), "{name:?}: {err}");
    }
    assert_eq!(
        mcp.call("books.verify", json!({"blobs": false}))["events"],
        json!(events),
        "refusals write nothing"
    );
    let out = mcp.call(
        "books.rename_account",
        json!({"account": "6150", "name": "Software subscriptions", "reason": "owner asked"}),
    );
    assert!(out["id"].as_u64().unwrap() > 0);
    assert_eq!(out["code"], "6150");
    assert_eq!(out["previous_name"], "Software");
    assert_eq!(out["name"], "Software subscriptions");
    let notes = out["notes"].to_string();
    assert!(
        notes.contains("QuickBooks matches accounts by name"),
        "{notes}"
    );
    let after = mcp.call("books.verify", json!({"blobs": false}));
    assert_eq!(after["status"], "ok");
    // New books declare account_renamed (with the rest of this version's
    // format) as they are created: the rename journals only itself.
    assert_eq!(after["events"], json!(events + 1), "the rename");
    let chart = mcp.call("books.chart", json!({}));
    let acct = chart["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["code"] == "6150")
        .unwrap()
        .clone();
    assert_eq!(acct["name"], "Software subscriptions");
    assert_eq!(acct["balance"], "29.00");
    assert_eq!(acct["tax_category"], "other_deductions");
    // A past period prints the new name; the amounts are the same.
    let out = mcp.call(
        "books.export",
        json!({"report": "profit_and_loss", "format": "json", "start": "2025-11-01", "end": "2025-11-30"}),
    );
    let pl: Value = serde_json::from_str(&mcp.handed_text(&out["file"])).unwrap();
    assert_eq!(pl["expense_accounts"][0]["name"], "Software subscriptions");
    assert_eq!(pl["expense_accounts"][0]["amount"], "29.00");
    let iif = mcp.call(
        "books.export",
        json!({"report": "journal", "format": "iif"}),
    );
    let iif = &mcp.handed_text(&iif["file"]);
    assert!(iif.contains("Operating:Software subscriptions"), "{iif}");
    // A renamed parent: its children's full names follow, and the note
    // says so.
    let out = mcp.call(
        "books.rename_account",
        json!({"account": "6000", "name": "Operating expenses"}),
    );
    assert!(out["notes"].to_string().contains("2 sub-accounts"), "{out}");
    // Renaming the bank account to a name without "checking" or "bank"
    // moves nothing: books.cash_accounts names it by code.
    let out = mcp.call(
        "books.rename_account",
        json!({"account": "1000", "name": "Treasury"}),
    );
    assert!(!out["notes"].to_string().contains("cash account"), "{out}");
    drop(mcp);
    books.restart();
    let mut mcp = Mcp::spawn(&books);
    init(&mut mcp);
    let chart = mcp.call("books.chart", json!({}));
    let names: Vec<(String, String)> = chart["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| {
            (
                a["code"].as_str().unwrap().to_string(),
                a["name"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    for want in [
        ("6150", "Software subscriptions"),
        ("6000", "Operating expenses"),
        ("1000", "Treasury"),
    ] {
        assert!(
            names.contains(&(want.0.to_string(), want.1.to_string())),
            "{names:?}"
        );
    }
    assert_eq!(
        mcp.call("books.verify", json!({"blobs": false}))["status"],
        "ok"
    );
}

/// Review R10 (rules.md B-31): on books that name no cash accounts
/// (books.cash_accounts unset, so they are picked by name), a rename that
/// moves an account into or out of the cash or card accounts says so; one
/// that moves nothing says nothing of them. A convention naming the old
/// name is listed, and one that doesn't isn't.
#[test]
fn rename_says_what_the_new_name_moves_defect_15() {
    let books = Books::new("rename-sets");
    let mut mcp = served(
        &books,
        "[company]\nname = \"Fernhill Labs LLC\"\n[books]\nreceipt_threshold = \"1000000.00\"\n",
    );
    let up12 = mcp.csv_upload("code,name,type,parent_code,tax_category\n1000,Business Checking,asset,,\n1010,Petty Fund,asset,,\n2100,Visa,liability,,\n2200,Credit Card,liability,,\n6000,Software,expense,,other_deductions\n");
    mcp.call("books.import_chart_csv", json!({"upload_id": up12}));
    mcp.call(
        "books.set_convention",
        json!({"id": "software-note", "kind": "note", "title": "Subscriptions",
               "body_md": "Every SaaS bill goes to Software.", "decided_by": "A"}),
    );
    let notes = |out: &Value| out["notes"].to_string();
    for (account, name, says) in [
        (
            "1000",
            "Treasury",
            "1000 Treasury is no longer a cash account",
        ),
        (
            "1010",
            "Petty Cash",
            "1010 Petty Cash is now a cash account",
        ),
        ("2100", "Visa Card", "2100 Visa Card is now a card account"),
        ("2200", "Amex", "2200 Amex is no longer a card account"),
    ] {
        let out = mcp.call(
            "books.rename_account",
            json!({"account": account, "name": name}),
        );
        assert!(notes(&out).contains(says), "{name}: {out}");
    }
    let out = mcp.call(
        "books.rename_account",
        json!({"account": "1010", "name": "Petty Cash Box"}),
    );
    assert!(!notes(&out).contains("cash account"), "{out}");
    let out = mcp.call(
        "books.rename_account",
        json!({"account": "6000", "name": "Software subscriptions"}),
    );
    assert!(
        notes(&out).contains("Convention software-note names \u{201c}Software\u{201d}"),
        "{out}"
    );
    let out = mcp.call(
        "books.rename_account",
        json!({"account": "6000", "name": "Tools"}),
    );
    assert!(!notes(&out).contains("Convention"), "{out}");
}

/// Defect 9 end to end (plan/served.md D-11, D-37): a schedule created
/// without a convention spreads by the day; books.list_schedules says how,
/// the first and last day covered, the next month to post, and what
/// remains; a run posts by the day; with books.schedule_convention set to
/// monthly_even, new schedules are equal months, and one that doesn't
/// start on the 1st is refused. Hand check: 1,200.00 from 2026-03-15 for
/// 12 months: March 100 × 17/31 = 54.84, then 100.00 a month, the last
/// March 45.16; through April 30, 154.84 posted, 1,045.16 left.
#[test]
fn mcp_daily_schedule() {
    let books = Books::new("daily-schedule");
    let mut mcp = Mcp::spawn(&books);
    init(&mut mcp);
    for (code, name, typ) in [
        ("1300", "Prepaid Expenses", "asset"),
        ("6300", "Insurance", "expense"),
    ] {
        mcp.call(
            "books.add_account",
            json!({"code": code, "name": name, "account_type": typ}),
        );
    }
    let create = |start: &str| {
        json!({
            "kind": "prepaid_expense",
            "description": "Liability insurance",
            "total": "1200.00",
            "start": start,
            "months": 12,
            "balance_account": "1300",
            "pl_account": "6300",
        })
    };
    let s = mcp.call("books.create_schedule", create("2026-03-15"));
    assert_eq!(s["convention"], "daily");
    assert_eq!(s["start"], "2026-03-15");
    assert_eq!(s["end"], "2027-03-14");
    assert_eq!(s["next"], "2026-03-31");
    assert_eq!(s["remaining"], "1200.00");
    let months = s["months"].as_array().unwrap();
    assert_eq!(months.len(), 13);
    assert_eq!(months[0]["amount"], "54.84");
    assert_eq!(months[1]["amount"], "100.00");
    assert_eq!(months[12]["date"], "2027-03-31");
    assert_eq!(months[12]["amount"], "45.16");

    let preview = mcp.call("books.run_schedules", json!({"through": "2026-04-30"}));
    assert_eq!(preview["status"], "preview");
    let amounts: Vec<&str> = preview["postings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["amount"].as_str().unwrap())
        .collect();
    assert_eq!(amounts, ["54.84", "100.00"]);
    assert_eq!(preview["skipped"], json!([]));
    let run = mcp.call(
        "books.run_schedules",
        json!({"through": "2026-04-30", "confirmed": true}),
    );
    assert_eq!(run["postings"].as_array().unwrap().len(), 2);
    let list = mcp.call("books.list_schedules", json!({}));
    let one = &list["schedules"][0];
    assert_eq!(one["convention"], "daily");
    assert_eq!(one["next"], "2026-05-31");
    assert_eq!(one["remaining"], "1045.16");

    // New schedules are set to equal months.
    mcp.call(
        "books.set_setting",
        json!({"changes": [{"key": "books.schedule_convention", "value": "monthly_even"}],
               "decided_by": "Owner", "decided_on": "2026-10-20"}),
    );
    let err = mcp.call_err("books.create_schedule", create("2026-03-15"));
    assert!(err.contains("2026-03-01"), "{err}");
    let even = mcp.call("books.create_schedule", create("2026-03-01"));
    assert_eq!(even["convention"], "monthly_even");
    assert_eq!(even["months"].as_array().unwrap().len(), 12);
    assert_eq!(even["end"], "2027-02-28");
    // A convention asked for wins over the setting; an unknown one is
    // refused, naming the two.
    let daily = mcp.call(
        "books.create_schedule",
        json!({"kind": "prepaid_expense", "description": "x", "total": "10.00",
               "start": "2026-05-20", "months": 1, "balance_account": "1300",
               "pl_account": "6300", "convention": "daily"}),
    );
    assert_eq!(daily["convention"], "daily");
    let err = mcp.call_err(
        "books.create_schedule",
        json!({"kind": "prepaid_expense", "description": "x", "total": "10.00",
               "start": "2026-05-01", "months": 1, "balance_account": "1300",
               "pl_account": "6300", "convention": "weekly"}),
    );
    assert!(err.contains("daily or monthly_even"), "{err}");
}

/// D-46: files pass only by handle. The five tools that file a file take
/// an upload and no other argument (their schemas close, and none names
/// an inline field); an inline argument is refused as unknown before the
/// tool runs, pointing to books.begin_upload, storing nothing; and every
/// file a tool once gave inline (books.export in each format,
/// books.asset_labels, books.get_receipt view bytes) comes back handed
/// over, its download the very bytes, with the size and SHA-256 it
/// states, and no bytes in the answer.
#[test]
fn files_pass_only_by_handle_in_and_out() {
    let books = Books::with(
        "handles",
        "USD",
        Some("[books]\nreceipt_threshold = \"0.00\"\ncash_accounts = [\"1000\"]\n"),
    );
    let mut mcp = Mcp::spawn(&books);
    init(&mut mcp);
    // The schemas: closed, an upload, nothing inline.
    let list = mcp.request("tools/list", json!({}));
    let tools = list["result"]["tools"].as_array().unwrap();
    let tool = |name: &str| {
        tools
            .iter()
            .find(|t| t["name"] == name)
            .unwrap_or_else(|| panic!("{name} listed"))
            .clone()
    };
    for (name, upload) in [
        ("books.attach_receipt", "upload_id"),
        ("books.file_document", "upload_id"),
        ("books.set_w9", "w9_upload_id"),
        ("books.import_bank_csv", "upload_id"),
        ("books.import_chart_csv", "upload_id"),
    ] {
        let input = &tool(name)["inputSchema"];
        assert_eq!(input["additionalProperties"], json!(false), "{name}");
        let props = input["properties"].as_object().unwrap();
        assert!(props.contains_key(upload), "{name}");
        for gone in [
            "content_base64",
            "sha256",
            "csv_text",
            "w9_content_base64",
            "w9_sha256",
        ] {
            assert!(!props.contains_key(gone), "{name} takes {gone}");
        }
    }
    for name in ["books.export", "books.asset_labels", "books.get_receipt"] {
        let out = tool(name)["outputSchema"].to_string();
        assert!(!out.contains("content_base64"), "{name}: {out}");
        assert!(!out.contains("content_encoding"), "{name}: {out}");
        assert!(out.contains("\"file\""), "{name}: {out}");
    }
    let chart = mcp.csv_upload(
        "code,name,type,parent_code\n1000,Cash,asset,\n1500,Equipment,asset,\n\
         1510,Accumulated Depreciation,asset,\n3000,Capital,equity,\n\
         6500,Depreciation,expense,\n",
    );
    mcp.call("books.import_chart_csv", json!({"upload_id": chart}));
    // In: an inline file is refused as unknown, storing nothing.
    let journal = books.journal();
    for (name, args) in [
        (
            "books.attach_receipt",
            json!({"file_name": "r.pdf", "mime_type": "application/pdf",
                   "content_base64": "JVBERi0xLjQ=",
                   "sha256": ledger_store::journal::sha256_hex(b"%PDF-1.4")}),
        ),
        (
            "books.import_chart_csv",
            json!({"csv_text": "code,name,type,parent_code\n1001,Petty,asset,\n"}),
        ),
    ] {
        let resp = mcp.request_raw("tools/call", json!({"name": name, "arguments": args}));
        let why = resp["error"]["message"].as_str().unwrap_or_default();
        assert!(
            why.contains("unknown argument") && why.contains("books.begin_upload"),
            "{name}: {resp}"
        );
    }
    assert_eq!(books.journal(), journal, "an inline file stored something");
    // A posted entry with its receipt, and an asset with its tag.
    let receipt = b"%PDF-1.4 a receipt for the laptop";
    let e = mcp.call(
        "books.draft_entry",
        json!({"date": "2026-01-02", "description": "Laptop"}),
    );
    let id = e["entry_id"].as_u64().unwrap();
    for (account, side) in [("1500", "debit"), ("3000", "credit")] {
        mcp.call(
            "books.add_line",
            json!({"entry_id": id, "account": account, "side": side, "amount": "1200.00"}),
        );
    }
    let up = mcp.upload(receipt, "laptop.pdf", "application/pdf", "receipt");
    let att = mcp.call(
        "books.attach_receipt",
        json!({"entry_id": id, "upload_id": up}),
    )["attachment_id"]
        .clone();
    mcp.call(
        "books.post_entry",
        json!({"entry_id": id, "confirmed": true, "no_party_reason": "a member's contribution"}),
    );
    mcp.call(
        "books.register_asset",
        json!({"description": "Laptop", "in_service_date": "2026-01-02", "cost": "1200.00",
               "useful_life_months": 36, "cost_account": "1500",
               "accum_depr_account": "1510", "expense_account": "6500"}),
    );
    // Out: each file handed over, its download the bytes it states.
    let handed = |mcp: &mut Mcp, out: &Value, mime: &str| -> Vec<u8> {
        for inline in ["content", "content_base64", "content_encoding"] {
            assert!(out.get(inline).is_none(), "{inline} in {out}");
        }
        let file = &out["file"];
        assert_eq!(file["mime_type"], mime, "{out}");
        assert!(
            file["download"]
                .as_str()
                .unwrap()
                .starts_with("wanderbooks download --book acme"),
            "{file}"
        );
        let bytes = mcp.handed(file);
        assert_eq!(
            file["sha256"],
            json!(ledger_store::journal::sha256_hex(&bytes))
        );
        assert_eq!(file["size_bytes"], json!(bytes.len()));
        bytes
    };
    for (format, report, mime, starts) in [
        ("csv", "general_journal", "text/csv", &b"entry_id,"[..]),
        ("json", "trial_balance", "application/json", b"{"),
        (
            "xlsx",
            "balance_sheet",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            b"PK",
        ),
        ("pdf", "profit_and_loss", "application/pdf", b"%PDF"),
        ("iif", "general_journal", "text/plain", b"!"),
    ] {
        let out = mcp.call("books.export", json!({"format": format, "report": report}));
        let bytes = handed(&mut mcp, &out, mime);
        assert!(
            bytes.starts_with(starts),
            "{format} {report}: {:?}",
            String::from_utf8_lossy(&bytes[..bytes.len().min(40)])
        );
        assert!(
            out["file"]["file_name"]
                .as_str()
                .unwrap()
                .ends_with(&format!(".{format}")),
            "{out}"
        );
    }
    let labels = mcp.call("books.asset_labels", json!({}));
    assert_eq!(labels["tags"].as_array().unwrap().len(), 1, "{labels}");
    assert!(handed(&mut mcp, &labels, "application/pdf").starts_with(b"%PDF"));
    let back = mcp.call(
        "books.get_receipt",
        json!({"attachment_id": att, "view": "bytes"}),
    );
    assert_eq!(handed(&mut mcp, &back, "application/pdf"), receipt);
    assert_eq!(back["file"]["sha256"], back["sha256"]);
    assert_eq!(back["file"]["file_name"], "laptop.pdf");
    let meta = mcp.call("books.get_receipt", json!({"attachment_id": att}));
    assert!(meta.get("file").is_none(), "{meta}");
}
