//! End-to-end smoke test: drive the real `ledger-mcp` binary over stdio
//! with raw JSON-RPC, exercising draft → preview → receipt policy → post.

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};

struct Mcp {
    stdin: ChildStdin,
    stdout: BufReader<std::process::ChildStdout>,
    next_id: u64,
    child: Child,
}

/// Closing stdin ends the server; wait for it to exit, so the next server
/// on the same books never races it for the writer lock.
impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Mcp {
    fn spawn(data_dir: &std::path::Path) -> Self {
        Self::spawn_with(data_dir, &[])
    }

    fn spawn_with(data_dir: &std::path::Path, extra: &[&std::ffi::OsStr]) -> Self {
        let bin = env!("CARGO_BIN_EXE_ledger-mcp");
        let mut child = Command::new(bin)
            .arg("--data-dir")
            .arg(data_dir)
            .args(extra)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn ledger-mcp");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Self {
            stdin,
            stdout,
            next_id: 1,
            child,
        }
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let msg = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        writeln!(self.stdin, "{msg}").unwrap();
        self.stdin.flush().unwrap();
        let mut line = String::new();
        self.stdout
            .read_line(&mut line)
            .expect("read response line");
        let resp: Value = serde_json::from_str(&line).expect("parse response");
        assert_eq!(resp["id"], id, "id mismatch in {resp}");
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
        writeln!(self.stdin, "{msg}").unwrap();
        self.stdin.flush().unwrap();
        let mut line = String::new();
        self.stdout
            .read_line(&mut line)
            .expect("read response line");
        let resp: Value = serde_json::from_str(&line).expect("parse response");
        assert_eq!(resp["id"], id, "id mismatch in {resp}");
        resp
    }

    fn notify(&mut self, method: &str) {
        let msg = json!({"jsonrpc": "2.0", "method": method, "params": {}});
        writeln!(self.stdin, "{msg}").unwrap();
        self.stdin.flush().unwrap();
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
}

fn test_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("ledger-mcp-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn mcp_end_to_end_over_stdio() {
    let dir = test_dir("main");
    let mut mcp = Mcp::spawn(&dir);

    // Initialize: the server identifies itself and delivers the contract.
    let init = mcp.request(
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "smoke-test", "version": "0.1.0"},
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
    assert_eq!(tools.len(), 91, "expected 91 tools, got {}", tools.len());
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
    let png = base64_like(b"fake-png-bytes");
    let attached = mcp.call(
        "books.attach_receipt",
        json!({
            "entry_id": entry_id,
            "file_name": "rent.png",
            "mime_type": "image/png",
            "content_base64": png,
        }),
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
        json!({"format": "csv", "report": "journal"}),
    );
    assert!(export["content"].as_str().unwrap().contains("January rent"));

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
fn mcp_assets_end_to_end_over_stdio() {
    let dir = test_dir("assets");
    let mut mcp = Mcp::spawn(&dir);
    let init = mcp.request(
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "smoke-test", "version": "0.1.0"},
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
            "expense_account": "6100",
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
            "gain_loss_account": "4900",
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
            "confirmed": true,
        }),
    );
    let err_json: serde_json::Value = serde_json::from_str(&err).unwrap();
    assert_eq!(err_json["code"], "receipt_required");
    assert_eq!(err_json["remedy_tool"], "books.file_document");

    // File the bill of sale, then confirm with its attachment id.
    let bill = mcp.call(
        "books.file_document",
        json!({
            "category": "legal",
            "file_name": "bill-of-sale.pdf",
            "mime_type": "application/pdf",
            "content_base64": base64_like(b"fake bill of sale"),
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
            "receipt_attachment_ids": [aid],
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
    let dir = test_dir("split");
    let mut mcp = Mcp::spawn(&dir);
    let init = mcp.request(
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "smoke-test", "version": "0.1.0"},
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
fn mcp_receipt_corrections_over_stdio() {
    let dir = test_dir("receipt-corrections");
    let mut mcp = Mcp::spawn(&dir);
    mcp.request(
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "smoke-test", "version": "0.1.0"},
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
        let att = mcp.call(
            "books.attach_receipt",
            json!({"entry_id": id, "file_name": "receipt.eml", "mime_type": "message/rfc822",
                   "content_base64": base64_like(receipt)}),
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
    let pdf = mcp.call(
        "books.attach_receipt",
        json!({"entry_id": big, "file_name": "receipt.pdf", "mime_type": "application/pdf",
               "content_base64": base64_like(b"pdf-bytes")}),
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
fn mcp_resources_over_stdio() {
    let dir = test_dir("resources");
    let mut mcp = Mcp::spawn(&dir);
    let init = mcp.request(
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "smoke-test", "version": "0.1.0"},
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

fn base64_like(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let mut n: u32 = 0;
        for (i, &b) in chunk.iter().enumerate() {
            n |= (b as u32) << (16 - 8 * i);
        }
        let pad = 3 - chunk.len();
        for i in 0..4 - pad {
            out.push(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize] as char);
        }
        for _ in 0..pad {
            out.push('=');
        }
    }
    out
}

#[test]
fn mcp_import_export_over_stdio() {
    let dir = test_dir("import_export");
    let mut mcp = Mcp::spawn(&dir);
    mcp.request(
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "smoke-test", "version": "0.1.0"},
        }),
    );
    mcp.notify("notifications/initialized");

    let chart = mcp.call(
        "books.import_chart_csv",
        json!({"csv_text": "code,name,type,parent_code\n1000,Cash,asset,\n4000,Revenue,revenue,\n"}),
    );
    assert_eq!(chart["count"], 2);

    let iif = mcp.call(
        "books.export",
        json!({"format": "iif", "report": "journal"}),
    );
    assert!(iif["content"].as_str().unwrap().contains("!TRNS"));

    let xlsx = mcp.call(
        "books.export",
        json!({"format": "xlsx", "report": "trial_balance"}),
    );
    assert_eq!(xlsx["content_encoding"], "base64");
    assert!(xlsx["content"].as_str().unwrap().starts_with("UEs"));
}

#[test]
fn mcp_statement_pdfs_and_cpa_package_over_stdio() {
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD;
    let dir = test_dir("statement-pdfs");
    let mut mcp = Mcp::spawn(&dir);
    mcp.request(
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "smoke-test", "version": "0.1.0"},
        }),
    );
    mcp.notify("notifications/initialized");
    mcp.call(
        "books.import_chart_csv",
        json!({"csv_text": "code,name,type,parent_code\n1000,Cash,asset,\n3000,Owner Capital,equity,\n6000,Software,expense,\n"}),
    );
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
        mcp.call(
            "books.attach_receipt",
            json!({"entry_id": id, "file_name": "r.pdf", "mime_type": "application/pdf",
                   "content_base64": base64_like(desc.as_bytes())}),
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
        assert_eq!(out["content_encoding"], "base64");
        let bytes = b64.decode(out["content"].as_str().unwrap()).unwrap();
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
    let err = mcp.call_err("books.export_cpa_package", json!({"as_of": "2026-10-31"}));
    assert!(err.contains("cash_accounts"), "{err}");
    let pkg = mcp.call(
        "books.export_cpa_package",
        json!({"as_of": "2026-10-31", "cash_accounts": ["1000"]}),
    );
    let zip_bytes = b64.decode(pkg["content_base64"].as_str().unwrap()).unwrap();
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
        json!({"as_of": "2026-10-31", "cash_accounts": ["1000"], "include_receipts": false}),
    );
    let light = zip::ZipArchive::new(std::io::Cursor::new(
        b64.decode(light["content_base64"].as_str().unwrap())
            .unwrap(),
    ))
    .unwrap();
    assert_eq!(light.len(), rest.len());
}

#[test]
fn mcp_period_statements_over_stdio() {
    let dir = test_dir("period-statements");
    // A rejected receipt policy would show up as a failed post below: the
    // toml lowers the threshold from 25.00 to 5.00.
    let toml = dir.join("wanderbooks.toml");
    std::fs::write(
        &toml,
        "[company]\nname = \"Test Co\"\nfiscal_year_start = \"02-01\"\ntax_classification = \"partnership\"\n[[company.members]]\nname = \"Owner\"\nshare = \"100%\"\ncapital_account = \"3000\"\n[books]\nreceipt_threshold = \"5.00\"\n",
    )
    .unwrap();
    let data = dir.join("books-data");
    let mut mcp = Mcp::spawn_with(
        &data,
        &[std::ffi::OsStr::new("--books-toml"), toml.as_os_str()],
    );
    mcp.request(
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "smoke-test", "version": "0.1.0"},
        }),
    );
    mcp.notify("notifications/initialized");
    mcp.call(
        "books.import_chart_csv",
        json!({"csv_text": "code,name,type,parent_code\n1000,Cash,asset,\n3000,Capital,equity,\n4000,Sales,revenue,\n6000,Software,expense,\n"}),
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
        id
    };
    let id = post(&mut mcp, "2026-01-02", "1000", "3000", "1000.00");
    // 1000.00 > 5.00 (the toml threshold): a receipt is required.
    let err = mcp.call_err(
        "books.post_entry",
        json!({"entry_id": id, "confirmed": true}),
    );
    assert!(err.contains("5.00"), "{err}");
    mcp.call(
        "books.attach_receipt",
        json!({"entry_id": id, "file_name": "r.pdf", "mime_type": "application/pdf",
               "content_base64": base64_like(b"r")}),
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
    // No start: the statement begins at the first entry, not year 1.
    let all = mcp.call("books.profit_and_loss", json!({"end": "2026-02-28"}));
    assert_eq!(all["start"], "2026-01-02");
    assert_eq!(all["revenue"], "6.50");

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
    let gl: serde_json::Value = serde_json::from_str(gl["content"].as_str().unwrap()).unwrap();
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
    let me: serde_json::Value = serde_json::from_str(me["content"].as_str().unwrap()).unwrap();
    assert_eq!(me["members"][0]["name"], "Owner");
    assert_eq!(me["total"]["contributions"], "1000.00");
    assert_eq!(me["total"]["net_income"], "3.50");
    assert_eq!(me["total"]["ending"], "1003.50");
    let pdf = mcp.call(
        "books.export",
        json!({"format": "pdf", "report": "members_equity", "end": "2026-02-28"}),
    );
    assert_eq!(pdf["content_encoding"], "base64");

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
    let me: serde_json::Value = serde_json::from_str(me["content"].as_str().unwrap()).unwrap();
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
    let ts: serde_json::Value = serde_json::from_str(ts["content"].as_str().unwrap()).unwrap();
    assert_eq!(ts["form"], "Form 1065");
    assert_eq!(ts["income"][0]["book"], "6.50");
    assert_eq!(ts["deductions"][0]["category"], "other_deductions");
    assert_eq!(ts["uncategorized"].as_array().unwrap().len(), 0);
    assert_eq!(ts["tax_net"], "3.50");
    // A chart import validates categories before creating anything.
    let err = mcp.call_err(
        "books.import_chart_csv",
        json!({"csv_text": "code,name,type,parent_code,tax_category\n6100,Meals,expense,,meals\n4100,Fees,revenue,,commissions_fees\n"}),
    );
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
fn mcp_partnership_from_template_over_stdio() {
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
    let dir = test_dir("template");
    let mut scratch = Mcp::spawn(&dir.join("scratch"));
    init(&mut scratch);
    let t = scratch.call(
        "books.chart_template",
        json!({"entity_type": "multi-member LLC", "owners": ["Ann", "Bo"]}),
    );
    let settings = t["settings_toml"].as_str().unwrap();
    assert!(settings.contains("distribution_account = \"3101\""));
    let err = scratch.call_err("books.chart_template", json!({"entity_type": "LLC"}));
    assert!(err.contains("entity_type"), "{err}");

    // 2. Its settings become the company config; its chart, the chart.
    let toml = dir.join("wanderbooks.toml");
    std::fs::write(
        &toml,
        format!("{settings}\n[books]\nreceipt_threshold = \"10000\"\n"),
    )
    .unwrap();
    let mut mcp = Mcp::spawn_with(
        &dir.join("books-data"),
        &[std::ffi::OsStr::new("--books-toml"), toml.as_os_str()],
    );
    init(&mut mcp);
    mcp.call("books.import_chart_csv", json!({"csv_text": t["csv"]}));
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
    let me: serde_json::Value = serde_json::from_str(me["content"].as_str().unwrap()).unwrap();
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

#[test]
fn mcp_schedules_and_recurring_over_stdio() {
    let dir = test_dir("schedules");
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
    let mut mcp = Mcp::spawn(&dir);
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

    // 1,000.00 of insurance over 3 months: 333.33, 333.33, 333.34.
    let s = mcp.call(
        "books.create_schedule",
        json!({
            "kind": "prepaid_expense",
            "description": "Liability insurance",
            "total": "1000.00",
            "start": "2026-11-15",
            "months": 3,
            "balance_account": "1300",
            "pl_account": "6300",
        }),
    );
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
            "balance_account": "6300", "pl_account": "6300",
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
            ],
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
    let mut mcp = Mcp::spawn(&dir);
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
fn mcp_payout_gross_up_over_stdio() {
    let dir = test_dir("payout");
    let mut mcp = Mcp::spawn(&dir);
    mcp.request(
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "smoke-test", "version": "0.1.0"},
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
    mcp.call(
        "books.attach_receipt",
        json!({
            "entry_id": entry_id,
            "file_name": "payout.pdf",
            "mime_type": "application/pdf",
            "content_base64": base64_like(b"%PDF-1.4 statement"),
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
fn mcp_vendor_1099_over_stdio() {
    let dir = test_dir("form1099");
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
    let mut mcp = Mcp::spawn(&dir);
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
    mcp.call("books.add_vendor", json!({"name": "Dana Designs"}));
    let v = mcp.call(
        "books.set_vendor_1099",
        json!({"vendor": "dana designs", "form_1099": "nec"}),
    );
    assert_eq!(v["form_1099"], "1099-NEC");
    let err = mcp.call_err(
        "books.set_vendor_1099",
        json!({"vendor": "Dana Designs", "form_1099": "k"}),
    );
    assert!(err.contains("nec, misc, or none"), "{err}");

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
    mcp.call(
        "books.attach_receipt",
        json!({
            "entry_id": id,
            "file_name": "invoice.pdf",
            "mime_type": "application/pdf",
            "content_base64": base64_like(b"%PDF-1.4 invoice"),
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
    // W-9 arrives.
    mcp.call(
        "books.set_vendor_1099",
        json!({"vendor": "Dana Designs", "form_1099": "nec", "tin_on_file": true}),
    );
    drop(mcp);

    let mut mcp = Mcp::spawn(&dir);
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
        "github.com/swaits/wanderbooks",
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
    // Show the entry in the viewer: a request outside the books.
    let shown = mcp.call("books.show", json!({"entry_id": id}));
    assert!(shown["shown"].as_str().unwrap().contains(&format!("#{id}")));
    let cmd = ledger_store::viewer::read(&dir).expect("request written");
    assert_eq!(
        cmd.target,
        ledger_store::viewer::ViewerTarget::Entry { entry_id: id }
    );
    let err = mcp.call_err("books.show", json!({"entry_id": 9999}));
    assert!(err.contains("unknown entry"), "{err}");
    let err = mcp.call_err("books.show", json!({"page": "cash", "search": "x"}));
    assert!(err.contains("exactly one"), "{err}");
    let _ = std::fs::remove_file(ledger_store::viewer::command_path(&dir));
    // The books check out end to end, receipt included.
    let v = mcp.call("books.verify", json!({}));
    assert_eq!(v["status"], "ok");
    assert_eq!(v["blobs_verified"], 1);
    assert!(v["events"].as_u64().unwrap() > 10);
}

#[test]
fn mcp_receipts_always_required_when_threshold_is_zero() {
    let dir = test_dir("always-receipts");
    let toml = dir.join("wanderbooks.toml");
    std::fs::write(&toml, "[books]\nreceipt_threshold = \"0.00\"\n").unwrap();
    let mut mcp = Mcp::spawn_with(
        &dir.join("books-data"),
        &[std::ffi::OsStr::new("--books-toml"), toml.as_os_str()],
    );
    let init = mcp.request(
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "smoke-test", "version": "0.1.0"},
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

/// A client reconnecting starts a new server while the old one still has
/// the books: the new one waits for the lock instead of dying.
#[test]
fn a_restarted_server_waits_for_the_old_one_to_let_go() {
    let dir = test_dir("restart-race");
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
    let mut old = Mcp::spawn(&dir);
    init(&mut old);
    let mut new = Mcp::spawn(&dir);
    std::thread::sleep(std::time::Duration::from_millis(500));
    drop(old);
    init(&mut new);
    let chart = new.call("books.chart", json!({}));
    assert!(chart.is_object());
}

/// `books.cash_flow` with no accounts passed reads the roles from
/// `[books]` in wanderbooks.toml (rules.md B-14): the owners' advance is
/// financing, gross; a loan to a member is investing.
#[test]
fn cash_flow_reads_its_account_roles_from_the_books_settings() {
    let dir = test_dir("cash-roles");
    let toml = dir.join("wanderbooks.toml");
    std::fs::write(
        &toml,
        "[books]\nreceipt_threshold = \"1000000.00\"\ncash_accounts = [\"1000\"]\nfinancing_accounts = [\"2100\"]\ninvesting_accounts = [\"1400\"]\n",
    )
    .unwrap();
    let mut mcp = Mcp::spawn_with(
        &dir.join("books-data"),
        &[std::ffi::OsStr::new("--books-toml"), toml.as_os_str()],
    );
    mcp.request(
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "smoke-test", "version": "0.1.0"},
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
    // Financing, gross and sorted by code: 2100 in and out, then capital.
    assert_eq!(amounts("financing"), ["100.00", "-100.00", "1000.00"]);
    assert_eq!(amounts("investing"), ["-500.00"]);
    assert_eq!(cf["net_change_in_cash"], "400.00");
}

/// A chart CSV with a bad row anywhere is refused whole: nothing is
/// half-imported (the old checks missed duplicates within the file and a
/// parent listed after its child).
#[test]
fn a_chart_file_with_a_bad_row_imports_nothing() {
    let dir = test_dir("chart-atomic");
    let mut mcp = Mcp::spawn(&dir);
    mcp.request(
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "smoke-test", "version": "0.1.0"},
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
        let err = mcp.call_err("books.import_chart_csv", json!({"csv_text": bad}));
        assert!(!err.is_empty());
        assert_eq!(count(&mut mcp), before, "half-imported: {bad}");
    }
    let ok = mcp.call(
        "books.import_chart_csv",
        json!({"csv_text": "code,name,type,parent_code\n1000,Cash,asset,\n1010,Checking,asset,1000\n"}),
    );
    assert_eq!(ok["count"], 2);
}

/// Bank CSV import: slash dates that read either way need the caller's
/// order; overlapping rules pick the longest pattern every time; and a
/// transaction that can't become a draft refuses the whole file.
#[test]
fn bank_import_reads_dates_rules_and_files_whole() {
    let dir = test_dir("bank-import");
    let mut mcp = Mcp::spawn(&dir);
    mcp.request(
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "smoke-test", "version": "0.1.0"},
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
    let base = |csv: &str| {
        json!({
            "csv_text": csv,
            "bank_account_code": "1000",
            "default_offset_code": "6000",
            "rules": {"coffee": "6100", "coffee beans": "6200"},
        })
    };
    // 3/4/2026 reads both ways and nothing in the file settles it.
    let ambiguous = "Date,Description,Amount\n3/4/2026,Coffee shop,-4.50\n";
    let err = mcp.call_err("books.import_bank_csv", base(ambiguous));
    assert!(err.contains("either way"), "{err}");
    let mut args = base(ambiguous);
    args["date_order"] = json!("day_first");
    let out = mcp.call("books.import_bank_csv", args);
    assert_eq!(out["imported"][0]["date"], "2026-04-03");
    // Both patterns match "Coffee beans": the longer wins, every time.
    for _ in 0..5 {
        let out = mcp.call(
            "books.import_bank_csv",
            base("Date,Description,Amount\n2026-05-01,Coffee beans 2kg,-30.00\n"),
        );
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
    let err = mcp.call_err(
        "books.import_bank_csv",
        base(
            "Date,Description,Amount\n2026-06-01,Fine,-1.00\n2026-01-15,In a closed month,-2.00\n",
        ),
    );
    assert!(err.contains("closed"), "{err}");
    let journal = std::fs::read_to_string(dir.join(ledger_store::JOURNAL_FILE)).unwrap();
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
    let dir = test_dir("renumber");
    let toml = dir.join("wanderbooks.toml");
    std::fs::write(
        &toml,
        "[books]\nreceipt_threshold = \"1000000.00\"\ncash_accounts = [\"1000\"]\n",
    )
    .unwrap();
    let mut mcp = Mcp::spawn_with(
        &dir.join("books-data"),
        &[std::ffi::OsStr::new("--books-toml"), toml.as_os_str()],
    );
    mcp.request(
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "smoke-test", "version": "0.1.0"},
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
    // The cash account is named in wanderbooks.toml: listed.
    let out = mcp.call(
        "books.renumber_account",
        json!({"account": "1000", "code": "1010"}),
    );
    assert_eq!(out["config_mentions"], json!(["[books] cash_accounts"]));
    drop(mcp);

    // A restart replays the journal to the same chart.
    let mut mcp = Mcp::spawn(&dir.join("books-data"));
    mcp.request(
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "smoke-test", "version": "0.1.0"},
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
