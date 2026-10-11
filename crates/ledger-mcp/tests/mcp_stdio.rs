//! End-to-end smoke test: drive the real `ledger-mcp` binary over stdio
//! with raw JSON-RPC, exercising draft → preview → receipt policy → post.

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};

struct Mcp {
    stdin: ChildStdin,
    stdout: BufReader<std::process::ChildStdout>,
    next_id: u64,
    #[allow(dead_code)]
    child: Child,
}

impl Mcp {
    fn spawn(data_dir: &std::path::Path) -> Self {
        let bin = env!("CARGO_BIN_EXE_ledger-mcp");
        let mut child = Command::new(bin)
            .arg("--data-dir")
            .arg(data_dir)
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

    // Tool listing: 23 narrow tools, chart is read-only.
    let list = mcp.request("tools/list", json!({}));
    let tools = list["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 48, "expected 48 tools, got {}", tools.len());
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
    assert_eq!(rent_line["balance_before"], "0");
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
    assert_eq!(tb2["total_debits"], "0");
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
    assert_eq!(preview["final_depreciation"], "0");
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
    assert_eq!(init["result"]["capabilities"]["resources"].is_null(), false);
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
