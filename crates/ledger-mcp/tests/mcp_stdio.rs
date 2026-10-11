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

fn test_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("ledger-mcp-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn mcp_end_to_end_over_stdio() {
    let dir = test_dir();
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

    // Tool listing: 16 narrow tools, chart is read-only.
    let list = mcp.request("tools/list", json!({}));
    let tools = list["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 18, "expected 18 tools, got {}", tools.len());
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
    assert!(err.contains("receipt_required"), "got: {err}");

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

fn base64_like(bytes: &[u8]) -> String {
    // Minimal base64 for the test (no external crate in dev-deps).
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
