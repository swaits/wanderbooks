//! Spec-blind tests for `ledger-mcp`, driven over stdio against the real
//! binary.
//!
//! Expected behavior comes only from the GOAL/SOURCE/INVARIANTS blocks of
//! `src/server.rs` and `src/policy.rs`, the server's `INSTRUCTIONS`,
//! `docs/mcp-interface.md`, `docs/spec/sources/{mcp-tools,jsonrpc-2.0,
//! irs-records}.md`, and the server's own `tools/list` (the declared
//! interface: names, input and output schemas). Where those are silent the
//! tests don't assert.
//!
//! Every tool call goes through [`Mcp::tool`], which checks, whatever the
//! test: one response with the request's id, result xor error (JSON-RPC
//! 2.0 §5); a refused call (isError or a JSON-RPC error) leaves
//! `journal.jsonl` byte-identical (server invariant 2); a success has
//! `structuredContent` that validates against the tool's `outputSchema`
//! and the same JSON in a text block (invariant 1); and no money key holds
//! a JSON number or a string outside `^-?\d+\.\d{2,}$` (invariant 3).

use proptest::prelude::*;
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashMap};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::Duration;

/// How long any single response may take before the server counts as hung.
/// Long enough for a server's first answer while a mutation run's jobs
/// contend for the disk (every journal append is synced).
const TIMEOUT: Duration = Duration::from_secs(180);

/// Random cases for the state machine: 64 on every check; `MCP_CASES`
/// sets fewer for a mutation run, more for a deeper one.
fn cases() -> u32 {
    std::env::var("MCP_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(64)
}

/// Keys whose values are money. Derived from the `outputSchema`s: every
/// string property described as an amount or "decimal string", plus the
/// payout totals (`gross`, `fees`, `adjustments`, `deposit`), the
/// reconciliation `difference`, and the doc's `debits`/`credits`/
/// `before`/`after`. Arrays and objects under these keys (e.g. `assets`
/// in `list_assets`) are walked, not checked.
const MONEY_KEYS: &[&str] = &[
    "accumulated",
    "adjustments",
    "after",
    "amount",
    "assets",
    "balance",
    "balance_after",
    "balance_before",
    "before",
    "book_balance",
    "book_value",
    "cash_beginning",
    "cash_ending",
    "cash_from_financing",
    "cash_from_investing",
    "cash_from_operations",
    "cleared_balance",
    "cost",
    "credit",
    "credits",
    "current",
    "current_earnings",
    "days_30",
    "days_60",
    "days_90",
    "days_90_plus",
    "debit",
    "debits",
    "deposit",
    "depreciation",
    "difference",
    "equity",
    "expenses",
    "fees",
    "final_depreciation",
    "gain_loss",
    "gross",
    "liabilities",
    "movement",
    "net_change_in_cash",
    "net_income",
    "paid",
    "proceeds",
    "remaining",
    "revenue",
    "salvage_value",
    "statement_balance",
    "threshold",
    "total",
    "total_credits",
    "total_debits",
    "total_gross",
    "total_net",
];

// ---------------------------------------------------------------- harness

/// What a `tools/call` came back as.
#[derive(Debug, Clone)]
enum Out {
    /// Success: the `structuredContent`.
    Ok(Value),
    /// `isError: true`: the first text block.
    ToolErr(String),
    /// A JSON-RPC error object.
    RpcErr(Value),
}

impl Out {
    fn ok(self, what: &str) -> Value {
        match self {
            Out::Ok(v) => v,
            other => panic!("{what}: expected success, got {other:?}"),
        }
    }
    fn refused(&self) -> bool {
        !matches!(self, Out::Ok(_))
    }
    fn err_text(&self) -> String {
        match self {
            Out::Ok(v) => panic!("expected a refusal, got {v}"),
            Out::ToolErr(t) => t.clone(),
            Out::RpcErr(e) => e.to_string(),
        }
    }
}

struct Mcp {
    stdin: ChildStdin,
    rx: Receiver<String>,
    child: Child,
    next_id: u64,
    data: PathBuf,
    schemas: HashMap<String, Value>,
    validators: HashMap<String, jsonschema::Validator>,
    /// Tools that returned a success at least once.
    exercised: BTreeSet<String>,
    /// Invariant 3 (amounts out) violations: `tool: path = value`.
    money_violations: Vec<String>,
}

impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

static DIR_SEQ: AtomicU64 = AtomicU64::new(0);

/// A fresh, empty per-test directory under the system temp dir.
fn test_dir(name: &str) -> PathBuf {
    let n = DIR_SEQ.fetch_add(1, Ordering::Relaxed);
    let dir =
        std::env::temp_dir().join(format!("ledger-mcp-spec-{}-{name}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Write a `wanderbooks.toml` with the given receipt threshold and extra
/// TOML (appended under `[books]` unless it opens its own table).
fn books_toml(dir: &Path, threshold: &str, extra: &str) -> PathBuf {
    let path = dir.join("wanderbooks.toml");
    std::fs::write(
        &path,
        format!(
            "[company]\nname = \"Spec Co\"\nfiscal_year_start = \"01-01\"\n\
             [books]\nreceipt_threshold = \"{threshold}\"\ncash_accounts = [\"1000\"]\n{extra}"
        ),
    )
    .unwrap();
    path
}

fn spawn_child(dir: &Path, toml: &Path) -> Child {
    let run = dir.join("run");
    std::fs::create_dir_all(&run).unwrap();
    Command::new(env!("CARGO_BIN_EXE_ledger-mcp"))
        .arg("--data-dir")
        .arg(dir.join("books-data"))
        .arg("--books-toml")
        .arg(toml)
        // books.show writes a viewer request here; keep it out of the
        // user's real runtime dir.
        .env("XDG_RUNTIME_DIR", &run)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn ledger-mcp")
}

impl Mcp {
    /// Spawn on `dir` with `toml`, initialize, and load `tools/list`.
    fn start(dir: &Path, toml: &Path) -> Self {
        let mut child = spawn_child(dir, toml);
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            for line in stdout.lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        let mut mcp = Self {
            stdin,
            rx,
            child,
            next_id: 1,
            data: dir.join("books-data"),
            schemas: HashMap::new(),
            validators: HashMap::new(),
            exercised: BTreeSet::new(),
            money_violations: Vec::new(),
        };
        let init = mcp.rpc(
            "initialize",
            json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "spec-test", "version": "0.1.0"},
            }),
        );
        assert!(init.get("result").is_some(), "initialize failed: {init}");
        mcp.send_line(
            &json!({"jsonrpc": "2.0", "method": "notifications/initialized"}).to_string(),
        );
        let list = mcp.rpc("tools/list", json!({}));
        for tool in list["result"]["tools"].as_array().expect("tools") {
            let name = tool["name"].as_str().unwrap().to_string();
            let schema = tool["outputSchema"].clone();
            assert!(schema.is_object(), "{name} declares no outputSchema");
            let validator = jsonschema::validator_for(&schema)
                .unwrap_or_else(|e| panic!("{name}'s outputSchema is not a valid schema: {e}"));
            mcp.validators.insert(name.clone(), validator);
            mcp.schemas.insert(name, tool.clone());
        }
        mcp
    }

    fn journal(&self) -> Vec<u8> {
        std::fs::read(self.data.join("journal.jsonl")).unwrap_or_default()
    }

    fn send_line(&mut self, line: &str) {
        writeln!(self.stdin, "{line}").unwrap();
        self.stdin.flush().unwrap();
    }

    /// The next JSON-RPC response (messages without an id, i.e. server
    /// notifications, are skipped). `None` on timeout or EOF.
    fn recv(&mut self, timeout: Duration) -> Option<Value> {
        loop {
            match self.rx.recv_timeout(timeout) {
                Ok(line) => {
                    let v: Value = serde_json::from_str(&line)
                        .unwrap_or_else(|e| panic!("server wrote non-JSON {line:?}: {e}"));
                    if v.get("id").is_none() && v.get("method").is_some() {
                        continue;
                    }
                    return Some(v);
                }
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => return None,
            }
        }
    }

    /// Send a request and return its response, checking JSON-RPC 2.0 §5:
    /// one response, same id, `jsonrpc: "2.0"`, result xor error.
    fn rpc(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let msg = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        self.send_line(&msg.to_string());
        let resp = self
            .recv(TIMEOUT)
            .unwrap_or_else(|| panic!("no response to {method} (id {id}): server hung or died"));
        check_response_shape(&resp);
        assert_eq!(resp["id"], json!(id), "response id mismatch: {resp}");
        resp
    }

    /// Call a tool and check invariants 1, 2, 3 and 7 on the outcome.
    fn tool(&mut self, name: &str, args: Value) -> Out {
        let before = self.journal();
        let resp = self.rpc("tools/call", json!({"name": name, "arguments": args}));
        let out = if let Some(err) = resp.get("error") {
            Out::RpcErr(err.clone())
        } else {
            let result = &resp["result"];
            if result.get("isError") == Some(&json!(true)) {
                let text = result["content"]
                    .as_array()
                    .and_then(|c| c.iter().find_map(|b| b["text"].as_str()))
                    .unwrap_or_default()
                    .to_string();
                Out::ToolErr(text)
            } else {
                Out::Ok(self.check_success(name, result))
            }
        };
        if out.refused() {
            assert!(
                self.journal() == before,
                "invariant 2: refused {name} {args} changed the journal ({out:?})"
            );
        }
        out
    }

    /// Invariant 1 (schema + text block) and invariant 3 (amounts out).
    fn check_success(&mut self, name: &str, result: &Value) -> Value {
        let sc = result
            .get("structuredContent")
            .unwrap_or_else(|| panic!("{name}: success without structuredContent: {result}"))
            .clone();
        let validator = self
            .validators
            .get(name)
            .unwrap_or_else(|| panic!("{name} is not in tools/list"));
        let errors: Vec<String> = validator
            .iter_errors(&sc)
            .map(|e| format!("{} at {}", e, e.instance_path()))
            .collect();
        assert!(
            errors.is_empty(),
            "invariant 1: {name} result violates its outputSchema: {errors:?}\n{sc}"
        );
        let same_text = result["content"].as_array().is_some_and(|blocks| {
            blocks.iter().any(|b| {
                b["type"] == "text"
                    && b["text"]
                        .as_str()
                        .and_then(|t| serde_json::from_str::<Value>(t).ok())
                        .is_some_and(|v| v == sc)
            })
        });
        assert!(
            same_text,
            "invariant 1: {name} has no text block holding its structuredContent"
        );
        let mut bad = Vec::new();
        money_walk(&sc, "$", None, &mut bad);
        for b in bad {
            self.money_violations.push(format!("{name}: {b}"));
        }
        self.exercised.insert(name.to_string());
        sc
    }

    /// `tool`, expecting success.
    fn ok(&mut self, name: &str, args: Value) -> Value {
        let a = args.to_string();
        self.tool(name, args).ok(&format!("{name} {a}"))
    }

    /// `tool`, expecting a refusal (isError or JSON-RPC error).
    fn refuse(&mut self, name: &str, args: Value) -> String {
        let a = args.to_string();
        let out = self.tool(name, args);
        assert!(out.refused(), "{name} {a} should be refused, got {out:?}");
        out.err_text()
    }

    /// Add accounts `(code, name, type)`; returns their ids.
    fn accounts(&mut self, accts: &[(&str, &str, &str)]) -> Vec<u64> {
        accts
            .iter()
            .map(|(code, name, ty)| {
                self.ok(
                    "books.add_account",
                    json!({"code": code, "name": name, "account_type": ty}),
                )["account_id"]
                    .as_u64()
                    .unwrap()
            })
            .collect()
    }

    /// Draft an entry with `(account, side, amount)` lines; returns its id.
    fn draft(&mut self, date: &str, desc: &str, lines: &[(&str, &str, &str)]) -> u64 {
        let id = self.ok(
            "books.draft_entry",
            json!({"date": date, "description": desc}),
        )["entry_id"]
            .as_u64()
            .unwrap();
        for (account, side, amount) in lines {
            self.ok(
                "books.add_line",
                json!({"entry_id": id, "account": account, "side": side, "amount": amount}),
            );
        }
        id
    }

    fn attach(&mut self, entry: Option<u64>, bytes: &[u8]) -> u64 {
        let mut args = json!({
            "file_name": "receipt.txt",
            "mime_type": "text/plain",
            "content_base64": b64(bytes),
        });
        if let Some(e) = entry {
            args["entry_id"] = json!(e);
        }
        self.ok("books.attach_receipt", args)["attachment_id"]
            .as_u64()
            .unwrap()
    }

    fn post(&mut self, entry: u64) -> Out {
        self.tool(
            "books.post_entry",
            json!({"entry_id": entry, "confirmed": true}),
        )
    }

    fn entry_status(&mut self, entry: u64) -> String {
        self.ok("books.get_entry", json!({"entry_id": entry}))["status"]
            .as_str()
            .unwrap()
            .to_string()
    }
}

fn check_response_shape(resp: &Value) {
    assert_eq!(
        resp["jsonrpc"], "2.0",
        "not a JSON-RPC 2.0 response: {resp}"
    );
    let has_result = resp.get("result").is_some();
    let has_error = resp.get("error").is_some();
    assert!(
        has_result ^ has_error,
        "response must have exactly one of result/error: {resp}"
    );
    assert!(resp.get("id").is_some(), "response without id: {resp}");
    if has_error {
        assert!(
            resp["error"]["code"].is_i64(),
            "error code must be an integer: {resp}"
        );
    }
}

/// Invariant 3, amounts out: under a money key, no JSON number, and every
/// string matches `^-?\d+\.\d{2,}$`.
fn money_walk(v: &Value, path: &str, key: Option<&str>, bad: &mut Vec<String>) {
    let money = key.is_some_and(|k| MONEY_KEYS.contains(&k));
    match v {
        Value::Number(_) if money => bad.push(format!("{path} = {v} (JSON number)")),
        Value::String(s) if money && !is_money_out(s) => bad.push(format!("{path} = {s:?}")),
        Value::Object(m) => {
            for (k, child) in m {
                money_walk(child, &format!("{path}.{k}"), Some(k), bad);
            }
        }
        Value::Array(a) => {
            for (i, child) in a.iter().enumerate() {
                money_walk(child, &format!("{path}[{i}]"), key, bad);
            }
        }
        _ => {}
    }
}

/// `^-?\d+\.\d{2,}$`
fn is_money_out(s: &str) -> bool {
    let s = s.strip_prefix('-').unwrap_or(s);
    let Some((int, frac)) = s.split_once('.') else {
        return false;
    };
    !int.is_empty()
        && int.bytes().all(|b| b.is_ascii_digit())
        && frac.len() >= 2
        && frac.bytes().all(|b| b.is_ascii_digit())
}

/// The amount-in grammar, server invariant 3: `-?digits[.digits]`.
fn is_amount_in(s: &str) -> bool {
    let s = s.strip_prefix('-').unwrap_or(s);
    let (int, frac) = match s.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (s, None),
    };
    !int.is_empty()
        && int.bytes().all(|b| b.is_ascii_digit())
        && frac.is_none_or(|f| !f.is_empty() && f.bytes().all(|b| b.is_ascii_digit()))
}

/// Canonical form of a grammatical decimal: no leading zeros, no trailing
/// fractional zeros. Compares two decimal strings by value.
fn canon(s: &str) -> String {
    let (neg, s) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s),
    };
    let (int, frac) = s.split_once('.').unwrap_or((s, ""));
    let int = int.trim_start_matches('0');
    let frac = frac.trim_end_matches('0');
    let int = if int.is_empty() { "0" } else { int };
    let body = if frac.is_empty() {
        int.to_string()
    } else {
        format!("{int}.{frac}")
    };
    if neg && body != "0" {
        format!("-{body}")
    } else {
        body
    }
}

fn b64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(T[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// A chart used by several tests.
const BASE_CHART: &[(&str, &str, &str)] = &[
    ("1000", "Bank", "asset"),
    ("1200", "Accounts Receivable", "asset"),
    ("2000", "Accounts Payable", "liability"),
    ("2100", "Due to Shareholders", "liability"),
    ("3000", "Capital A", "equity"),
    ("3001", "Capital B", "equity"),
    ("4000", "Revenue", "revenue"),
    ("6000", "Rent", "expense"),
    ("6999", "General Expenses", "expense"),
];

// ------------------------------------------------- invariant 1: schemas

/// Every tool, read-only and mutating, on a scripted happy path; every
/// success validated by `Mcp::tool`. Returns the server for inspection.
fn happy_path(name: &str) -> Mcp {
    let dir = test_dir(name);
    let toml = books_toml(
        &dir,
        "1000000.00",
        "financing_accounts = [\"2100\"]\n\
         [[company.members]]\nname = \"A\"\nshare = \"50%\"\ncapital_account = \"3000\"\n\
         [[company.members]]\nname = \"B\"\nshare = \"50%\"\ncapital_account = \"3001\"\n",
    );
    let mut m = Mcp::start(&dir, &toml);

    // Empty books: every read-only tool.
    m.ok("books.chart", json!({}));
    m.ok(
        "books.chart_template",
        json!({"entity_type": "partnership", "owners": ["A", "B"]}),
    );
    let cats = m.ok("books.tax_categories", json!({}));
    m.ok("books.trial_balance", json!({}));
    m.ok("books.balance_sheet", json!({}));
    m.ok("books.profit_and_loss", json!({}));
    m.ok("books.list_assets", json!({}));
    m.ok("books.list_classes", json!({}));
    m.ok("books.list_payroll_runs", json!({}));
    m.ok("books.list_periods", json!({}));
    m.ok("books.list_receipts", json!({}));
    m.ok("books.list_recurring", json!({}));
    m.ok("books.list_schedules", json!({}));
    m.ok("books.list_vendors", json!({}));
    m.ok("books.digest", json!({"since": "2026-01-01"}));

    // Chart.
    m.accounts(BASE_CHART);
    m.accounts(&[
        ("1300", "Prepaid Expenses", "asset"),
        ("1500", "Equipment", "asset"),
        ("1510", "Accumulated Depreciation", "asset"),
        ("2400", "Deferred Revenue", "liability"),
        ("3999", "Retained Earnings", "equity"),
        ("6010", "Platform Fees", "expense"),
        ("6500", "Depreciation Expense", "expense"),
    ]);
    m.ok(
        "books.import_chart_csv",
        json!({"csv_text": "code,name,type,parent_code\n7000,Gain or Loss on Disposal,expense,\n"}),
    );
    m.ok(
        "books.renumber_account",
        json!({"account": "7000", "code": "7100"}),
    );
    m.ok("books.settings", json!({}));
    m.ok("books.roles", json!({}));
    m.ok(
        "books.assign_role",
        json!({"role": "general_expense", "account": "7100"}),
    );
    m.ok(
        "books.assign_role",
        json!({"role": "general_expense", "account": null}),
    );
    let deduction = cats["categories"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["kind"] == "deduction")
        .expect("a deduction category")["key"]
        .clone();
    m.ok(
        "books.set_tax_category",
        json!({"account": "6000", "category": deduction}),
    );

    // Classes, parties, 1099.
    m.ok("books.add_class", json!({"name": "Rondo"}));
    m.ok("books.list_classes", json!({}));
    let vendor = m.ok("books.add_vendor", json!({"name": "Landlord"}))["id"]
        .as_u64()
        .unwrap();
    let customer = m.ok("books.add_customer", json!({"name": "Client"}))["id"]
        .as_u64()
        .unwrap();
    m.ok(
        "books.set_vendor_1099",
        json!({"vendor": "Landlord", "form_1099": "misc", "tin_on_file": true}),
    );
    m.ok("books.list_vendors", json!({}));

    // Owner contribution: draft, lines, receipt, preview, post.
    let e1 = m.draft(
        "2026-01-02",
        "Owner contribution",
        &[
            ("1000", "debit", "10000.00"),
            ("3000", "credit", "5000.00"),
            ("3001", "credit", "5000.00"),
        ],
    );
    m.attach(Some(e1), b"contribution receipt");
    m.ok("books.post_entry", json!({"entry_id": e1}));
    m.ok(
        "books.post_entry",
        json!({"entry_id": e1, "confirmed": true, "expected_total": "10000.00"}),
    );
    m.ok("books.get_entry", json!({"entry_id": e1}));

    // Rent: vendor, class, unlinked receipt then link, clearing.
    let e2 = m.ok(
        "books.draft_entry",
        json!({"date": "2026-01-15", "description": "January rent", "vendor": "Landlord"}),
    )["entry_id"]
        .as_u64()
        .unwrap();
    m.ok(
        "books.add_line",
        json!({"entry_id": e2, "account": "6000", "side": "debit", "amount": "100.00", "class": "Rondo"}),
    );
    m.ok(
        "books.add_line",
        json!({"entry_id": e2, "account": "1000", "side": "credit", "amount": "100.00"}),
    );
    let r_old = m.attach(None, b"rent receipt v1");
    m.ok(
        "books.link_receipt",
        json!({"attachment_id": r_old, "entry_id": e2}),
    );
    m.post(e2).ok("post rent");
    m.ok(
        "books.mark_cleared",
        json!({"entry_id": e2, "line_index": 1, "cleared": true}),
    );
    m.ok(
        "books.set_entry_vendor",
        json!({"entry_id": e2, "vendor": vendor.to_string()}),
    );
    // Replace the receipt: attach new, unlink old, retire it.
    m.attach(Some(e2), b"rent receipt v2");
    m.ok(
        "books.unlink_receipt",
        json!({"attachment_id": r_old, "reason": "replaced by v2"}),
    );
    m.ok(
        "books.withdraw_attachment",
        json!({"attachment_id": r_old, "reason": "superseded"}),
    );
    m.ok("books.list_receipts", json!({"include_withdrawn": true}));
    m.ok("books.list_receipts", json!({"entry_id": e2}));
    m.ok("books.get_receipt", json!({"attachment_id": r_old}));
    let doc = m.ok(
        "books.file_document",
        json!({"category": "legal", "file_name": "agreement.txt", "mime_type": "text/plain",
               "content_base64": b64(b"operating agreement"), "description": "OA"}),
    )["attachment_id"]
        .as_u64()
        .unwrap();
    let local = dir.join("local-receipt.txt");
    std::fs::write(&local, b"a local receipt").unwrap();
    m.ok(
        "books.import_local_file",
        json!({"path": local.to_str().unwrap(), "category": "receipt"}),
    );

    // Reversal: preview, then confirmed.
    let e3 = m.draft(
        "2026-01-20",
        "Mistake",
        &[("6999", "debit", "50.00"), ("1000", "credit", "50.00")],
    );
    m.attach(Some(e3), b"mistake receipt");
    m.post(e3).ok("post mistake");
    let rev = json!({"entry_id": e3, "date": "2026-01-21", "description": "Reverse mistake"});
    m.ok("books.reverse_entry", rev.clone());
    let mut rev_c = rev;
    rev_c["confirmed"] = json!(true);
    m.ok("books.reverse_entry", rev_c);

    // FX.
    m.ok(
        "books.set_fx_rate",
        json!({"currency": "EUR", "date": "2026-01-01", "rate": "1.10"}),
    );
    let fx = m.ok(
        "books.draft_entry",
        json!({"date": "2026-04-22", "description": "EUR draft"}),
    )["entry_id"]
        .as_u64()
        .unwrap();
    m.ok(
        "books.add_line",
        json!({"entry_id": fx, "account": "6999", "side": "debit", "amount": "11.00",
               "foreign_currency": "EUR", "foreign_amount": "10.00", "fx_rate": "1.10"}),
    );

    // A/R and A/P.
    let inv = m.ok(
        "books.create_invoice",
        json!({"party_id": customer, "date": "2026-02-01", "due_date": "2026-03-01",
               "lines": [{"description": "Consulting", "amount": "500.00"}]}),
    )["id"]
        .as_u64()
        .unwrap();
    m.ok(
        "books.receive_payment",
        json!({"document_id": inv, "date": "2026-02-10", "amount": "200.00", "cash_account": "1000"}),
    );
    let bill = m.ok(
        "books.create_bill",
        json!({"party_id": vendor, "date": "2026-02-01", "due_date": "2026-02-28",
               "lines": [{"description": "Office rent", "amount": "300.00"}]}),
    )["id"]
        .as_u64()
        .unwrap();
    m.ok(
        "books.pay_bill",
        json!({"document_id": bill, "date": "2026-02-05", "amount": "300.00", "cash_account": "1000"}),
    );
    m.ok("books.ar_aging", json!({"as_of": "2026-06-30"}));
    m.ok("books.ap_aging", json!({"as_of": "2026-06-30"}));

    // Fixed assets.
    let registered = m.ok(
        "books.register_asset",
        json!({"description": "Laptop", "purchase_date": "2026-01-10", "cost": "3000.00",
               "salvage_value": "0.00", "useful_life_months": 36, "cost_account": "1500",
               "accum_depr_account": "1510", "expense_account": "6500"}),
    );
    assert_eq!(registered["tag"], "S-0001", "\"Spec Co\" without \"Co\": S");
    let asset = registered["asset_id"].as_u64().unwrap();
    m.ok("books.asset_schedule", json!({"asset_id": asset}));
    m.ok("books.asset_schedule", json!({"tag": "s-0001"}));
    m.ok(
        "books.label_asset",
        json!({"asset": "S-0001", "tag": "LAPTOP-1", "serial": "SN 123"}),
    );
    m.ok(
        "books.set_asset_tax_basis",
        json!({"asset": "LAPTOP-1", "rule": "personal_use_conversion", "contributor_basis": "3200.00"}),
    );
    m.ok(
        "books.correct_asset",
        json!({"asset": "LAPTOP-1", "useful_life_months": 48, "reason": "registered with the wrong life"}),
    );
    m.ok("books.asset_labels", json!({}));
    m.ok(
        "books.asset_labels",
        json!({"assets": ["LAPTOP-1"], "skip": 3}),
    );
    m.ok(
        "books.export",
        json!({"report": "fixed_asset_register", "format": "json", "as_of": "2026-03-31"}),
    );
    m.ok("books.list_assets", json!({}));
    m.ok("books.run_depreciation", json!({"period": "2026-01"}));
    let dispose = json!({"asset_id": asset, "date": "2026-06-15", "proceeds": "1000.00",
                         "proceeds_account": "1000", "gain_loss_account": "7100"});
    m.ok("books.dispose_asset", dispose.clone());
    let mut dispose_c = dispose;
    dispose_c["confirmed"] = json!(true);
    dispose_c["receipt_attachment_ids"] = json!([doc]);
    m.ok("books.dispose_asset", dispose_c);

    // Schedules and recurring.
    m.ok(
        "books.create_schedule",
        json!({"kind": "prepaid_expense", "description": "Insurance", "total": "1200.00",
               "start": "2026-01-01", "months": 12, "balance_account": "1300", "pl_account": "6999"}),
    );
    m.ok("books.list_schedules", json!({}));
    m.ok("books.run_schedules", json!({"through": "2026-03-31"}));
    m.ok(
        "books.run_schedules",
        json!({"through": "2026-03-31", "confirmed": true}),
    );
    m.ok(
        "books.create_recurring",
        json!({"description": "Hosting", "frequency": "monthly", "start": "2026-04-30",
               "end": "2026-06-30", "lines": [
                   {"account": "6999", "side": "debit", "amount": "10.00"},
                   {"account": "1000", "side": "credit", "amount": "10.00"}]}),
    );
    m.ok("books.list_recurring", json!({}));
    m.ok("books.run_recurring", json!({"through": "2026-06-30"}));

    // Payouts.
    m.ok(
        "books.record_payout",
        json!({"date": "2026-02-28", "description": "App Store payout", "deposit_account": "1000",
               "deposit": "85.00", "revenue": [{"account": "4000", "amount": "100.00"}],
               "fees": [{"account": "6010", "amount": "15.00"}]}),
    );

    // Payroll.
    m.ok(
        "books.run_payroll",
        json!({"date": "2026-02-15", "cash_account": "1000", "lines": [{
            "employee_name": "Pat", "gross_pay": "1000.00", "federal_withholding": "100.00",
            "state_withholding": "50.00", "ss_employee": "62.00", "medicare_employee": "14.50",
            "ss_employer": "62.00", "medicare_employer": "14.50", "other_deductions": "0.00"}]}),
    );
    m.ok("books.list_payroll_runs", json!({}));
    m.ok(
        "books.remit_payroll_taxes",
        json!({"date": "2026-02-20", "cash_account": "1000"}),
    );

    // Bank import and reconciliation.
    m.ok(
        "books.import_bank_csv",
        json!({"csv_text": "Date,Description,Amount\n2026-03-05,Coffee,-4.50\n",
               "bank_account_code": "1000", "default_offset_code": "6999"}),
    );
    m.ok(
        "books.reconcile",
        json!({"account": "1000", "statement_date": "2026-12-31", "statement_balance": "100.00"}),
    );

    // Periods.
    let p = m.ok(
        "books.create_period",
        json!({"name": "2026-01", "start": "2026-01-01", "end": "2026-01-31"}),
    )["period_id"]
        .as_u64()
        .unwrap();
    m.ok("books.list_periods", json!({}));
    m.ok("books.close_period", json!({"period_id": p}));
    m.ok(
        "books.close_period",
        json!({"period_id": p, "confirmed": true}),
    );

    // Reports over the populated books.
    m.ok("books.chart", json!({}));
    m.ok("books.trial_balance", json!({"as_of": "2026-12-31"}));
    m.ok("books.balance_sheet", json!({"as_of": "2026-12-31"}));
    m.ok(
        "books.profit_and_loss",
        json!({"start": "2026-01-01", "end": "2026-12-31"}),
    );
    m.ok("books.profit_and_loss", json!({"class": "Rondo"}));
    m.ok(
        "books.profit_and_loss_by_class",
        json!({"start": "2026-01-01", "end": "2026-12-31"}),
    );
    m.ok(
        "books.cash_flow",
        json!({"start": "2026-01-01", "end": "2026-12-31"}),
    );
    m.ok("books.digest", json!({"since": "2026-01-01"}));
    m.ok("books.search_entries", json!({"text": "rent", "limit": 10}));
    m.ok("books.vendor_1099", json!({"year": 2026}));
    m.ok("books.verify", json!({}));
    m.ok("books.show", json!({"entry_id": e1}));
    m.ok(
        "books.show",
        json!({"report": "balance_sheet", "end": "2026-12-31"}),
    );
    for (format, report) in [
        ("csv", "trial_balance"),
        ("json", "profit_and_loss"),
        ("xlsx", "balance_sheet"),
        ("pdf", "profit_and_loss"),
        ("csv", "cash_flow"),
        ("iif", "journal"),
        ("csv", "general_ledger"),
        ("json", "members_equity"),
        ("csv", "tax_summary"),
    ] {
        m.ok(
            "books.export",
            json!({"format": format, "report": report, "start": "2026-01-01", "end": "2026-12-31"}),
        );
    }
    m.ok(
        "books.export_cpa_package",
        json!({"as_of": "2026-12-31", "include_receipts": false}),
    );

    m
}

/// Invariant 1 over every tool. Fails listing any tool never exercised
/// successfully.
#[test]
fn inv1_every_tool_result_matches_its_output_schema() {
    let m = happy_path("inv1");
    let missing: Vec<&String> = m
        .schemas
        .keys()
        .filter(|t| !m.exercised.contains(*t))
        .collect();
    assert!(missing.is_empty(), "tools never exercised: {missing:?}");
}

/// Invariant 3, amounts out, weak form: no money key ever holds a JSON
/// number, over every tool's results.
#[test]
fn inv3_amounts_out_are_never_json_numbers() {
    let m = happy_path("inv3-out-numbers");
    let numbers: Vec<&String> = m
        .money_violations
        .iter()
        .filter(|v| v.contains("JSON number"))
        .collect();
    assert!(numbers.is_empty(), "{numbers:#?}");
}

/// Invariant 3, amounts out, full form: every amount is a decimal string
/// with at least two places.
#[test]
fn inv3_amounts_out_have_at_least_two_places() {
    let m = happy_path("inv3-out-places");
    assert!(
        m.money_violations.is_empty(),
        "invariant 3 (amounts out): {:#?}",
        m.money_violations
    );
}

// ------------------------------------- invariants 2 and 7: state machine

/// An id argument: usually one the books handed out (by index into what
/// this run has created), sometimes a raw number that may not exist.
#[derive(Debug, Clone, Copy)]
enum Pick {
    Known(usize),
    Raw(u64),
}

impl Pick {
    fn resolve(self, known: &[u64]) -> u64 {
        match self {
            Pick::Known(i) if !known.is_empty() => known[i % known.len()],
            Pick::Known(i) => i as u64,
            Pick::Raw(n) => n,
        }
    }
}

fn pick() -> impl Strategy<Value = Pick> {
    prop_oneof![3 => (0usize..16).prop_map(Pick::Known), 1 => (0u64..12).prop_map(Pick::Raw)]
}

/// Ids handed out so far in a run.
#[derive(Default)]
struct Known {
    entries: Vec<u64>,
    attachments: Vec<u64>,
    periods: Vec<u64>,
    vendors: Vec<u64>,
    bills: Vec<u64>,
}

/// One step of the random walk.
#[derive(Debug, Clone)]
enum Op {
    Draft {
        date: String,
        desc: String,
    },
    /// Draft plus two lines (debit `account`, credit 1000), optionally
    /// with a receipt: several calls, each checked.
    Balanced {
        date: String,
        desc: String,
        account: String,
        amount: String,
        receipt: bool,
    },
    AddLine {
        entry: Pick,
        account: String,
        side: String,
        amount: String,
    },
    Post {
        entry: Pick,
        confirmed: Option<bool>,
    },
    Reverse {
        entry: Pick,
        date: String,
        confirmed: Option<bool>,
    },
    Attach {
        entry: Option<Pick>,
        content: String,
    },
    Link {
        att: Pick,
        entry: Pick,
    },
    Unlink {
        att: Pick,
    },
    Withdraw {
        att: Pick,
    },
    CreatePeriod {
        start: String,
        end: String,
    },
    ClosePeriod {
        id: Pick,
        confirmed: bool,
    },
    AddAccount {
        code: String,
        ty: String,
    },
    AddVendor {
        name: String,
    },
    CreateBill {
        party: Pick,
        amount: String,
    },
    PayBill {
        doc: Pick,
        amount: String,
    },
    Payout {
        deposit: String,
    },
    Renumber {
        account: String,
        code: String,
    },
    Read(u8),
    UnknownTool,
    MissingArgs,
    WrongTypes,
}

fn date_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        8 => (1u32..=6, 1u32..=28).prop_map(|(m, d)| format!("2026-{m:02}-{d:02}")),
        1 => prop::sample::select(vec![
            "2026-02-30", "2026-13-01", "26-1-1", "", "2026/01/05", "yesterday", "2026-1-5",
        ])
        .prop_map(String::from),
    ]
}

fn amount_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        6 => (1u64..=6000).prop_map(|c| format!("{}.{:02}", c / 100, c % 100)),
        2 => prop::sample::select(vec!["25.00", "25.01", "20.00", "5"]).prop_map(String::from),
        2 => prop::sample::select(vec![
            "1e3", "+5", ".5", "5.", "1,000", " 5", "abc", "", "-", "-5.00", "0", "0.001", "NaN",
        ])
        .prop_map(String::from),
    ]
}

fn account_strategy() -> impl Strategy<Value = String> {
    prop::sample::select(vec![
        "1000", "1000", "2000", "4000", "6000", "6000", "6999", "1", "3", "9999", "", "abc",
    ])
    .prop_map(String::from)
}

fn desc_strategy() -> impl Strategy<Value = String> {
    prop::sample::select(vec!["Coffee", "coffee ", "Rent", "Supplies"]).prop_map(String::from)
}

fn op_strategy() -> impl Strategy<Value = Op> {
    let confirmed = || prop::option::of(any::<bool>());
    prop_oneof![
        2 => (date_strategy(), desc_strategy()).prop_map(|(date, desc)| Op::Draft { date, desc }),
        5 => (date_strategy(), desc_strategy(),
              prop::sample::select(vec!["6000", "6999", "2000", "9999"]),
              amount_strategy(), any::<bool>())
            .prop_map(|(date, desc, a, amount, receipt)| Op::Balanced {
                date, desc, account: a.to_string(), amount, receipt,
            }),
        5 => (pick(), account_strategy(),
              prop::sample::select(vec!["debit", "credit", "debit", "credit", "Debit", "sideways"]),
              amount_strategy())
            .prop_map(|(entry, account, s, amount)| Op::AddLine { entry, account, side: s.to_string(), amount }),
        6 => (pick(), confirmed()).prop_map(|(entry, confirmed)| Op::Post { entry, confirmed }),
        2 => (pick(), date_strategy(), confirmed())
            .prop_map(|(entry, date, confirmed)| Op::Reverse { entry, date, confirmed }),
        3 => (prop::option::of(pick()), prop_oneof![
                4 => prop::collection::vec(any::<u8>(), 1..24).prop_map(|b| b64(&b)),
                1 => Just("!!not base64!!".to_string()),
            ])
            .prop_map(|(entry, content)| Op::Attach { entry, content }),
        1 => (pick(), pick()).prop_map(|(att, entry)| Op::Link { att, entry }),
        1 => pick().prop_map(|att| Op::Unlink { att }),
        1 => pick().prop_map(|att| Op::Withdraw { att }),
        1 => (date_strategy(), date_strategy()).prop_map(|(start, end)| Op::CreatePeriod { start, end }),
        1 => (pick(), any::<bool>()).prop_map(|(id, confirmed)| Op::ClosePeriod { id, confirmed }),
        1 => (prop::sample::select(vec!["1000", "5000", "5001", "", "x"]),
              prop::sample::select(vec!["asset", "expense", "revenue", "bogus"]))
            .prop_map(|(c, t)| Op::AddAccount { code: c.to_string(), ty: t.to_string() }),
        1 => prop::sample::select(vec!["Acme", "Acme", "", "1"]).prop_map(|n| Op::AddVendor { name: n.to_string() }),
        1 => (pick(), amount_strategy()).prop_map(|(party, amount)| Op::CreateBill { party, amount }),
        1 => (pick(), amount_strategy()).prop_map(|(doc, amount)| Op::PayBill { doc, amount }),
        1 => amount_strategy().prop_map(|deposit| Op::Payout { deposit }),
        1 => (account_strategy(), prop::sample::select(vec!["8000", "1000", ""]))
            .prop_map(|(account, c)| Op::Renumber { account, code: c.to_string() }),
        3 => (0u8..6).prop_map(Op::Read),
        1 => Just(Op::UnknownTool),
        1 => Just(Op::MissingArgs),
        1 => Just(Op::WrongTypes),
    ]
}

/// The id a success handed out under `key`, if any.
fn new_id(out: &Out, key: &str) -> Option<u64> {
    match out {
        Out::Ok(v) => v[key].as_u64(),
        _ => None,
    }
}

fn run_op(m: &mut Mcp, k: &mut Known, op: &Op) {
    let opt = |v: Option<bool>| v.map_or(Value::Null, Value::Bool);
    let (tool, args) = match op {
        Op::Draft { date, desc } => (
            "books.draft_entry",
            json!({"date": date, "description": desc}),
        ),
        Op::Balanced {
            date,
            desc,
            account,
            amount,
            receipt,
        } => {
            let out = m.tool(
                "books.draft_entry",
                json!({"date": date, "description": desc}),
            );
            let Some(e) = new_id(&out, "entry_id") else {
                return;
            };
            k.entries.push(e);
            for (acct, side) in [(account.as_str(), "debit"), ("1000", "credit")] {
                m.tool(
                    "books.add_line",
                    json!({"entry_id": e, "account": acct, "side": side, "amount": amount}),
                );
            }
            if *receipt {
                let bytes = format!("receipt for entry {e}");
                let out = m.tool(
                    "books.attach_receipt",
                    json!({"entry_id": e, "file_name": "r.txt", "mime_type": "text/plain",
                           "content_base64": b64(bytes.as_bytes())}),
                );
                k.attachments.extend(new_id(&out, "attachment_id"));
            }
            return;
        }
        Op::AddLine {
            entry,
            account,
            side,
            amount,
        } => (
            "books.add_line",
            json!({"entry_id": entry.resolve(&k.entries), "account": account, "side": side,
                   "amount": amount}),
        ),
        Op::Post { entry, confirmed } => (
            "books.post_entry",
            json!({"entry_id": entry.resolve(&k.entries), "confirmed": opt(*confirmed)}),
        ),
        Op::Reverse {
            entry,
            date,
            confirmed,
        } => (
            "books.reverse_entry",
            json!({"entry_id": entry.resolve(&k.entries), "date": date, "description": "undo",
                   "confirmed": opt(*confirmed)}),
        ),
        Op::Attach { entry, content } => (
            "books.attach_receipt",
            json!({"entry_id": entry.map(|p| p.resolve(&k.entries)), "file_name": "r.bin",
                   "mime_type": "application/octet-stream", "content_base64": content}),
        ),
        Op::Link { att, entry } => (
            "books.link_receipt",
            json!({"attachment_id": att.resolve(&k.attachments),
                   "entry_id": entry.resolve(&k.entries)}),
        ),
        Op::Unlink { att } => (
            "books.unlink_receipt",
            json!({"attachment_id": att.resolve(&k.attachments), "reason": "test"}),
        ),
        Op::Withdraw { att } => (
            "books.withdraw_attachment",
            json!({"attachment_id": att.resolve(&k.attachments), "reason": "test"}),
        ),
        Op::CreatePeriod { start, end } => (
            "books.create_period",
            json!({"name": "P", "start": start, "end": end}),
        ),
        Op::ClosePeriod { id, confirmed } => (
            "books.close_period",
            json!({"period_id": id.resolve(&k.periods), "confirmed": confirmed}),
        ),
        Op::AddAccount { code, ty } => (
            "books.add_account",
            json!({"code": code, "name": format!("Acct {code}"), "account_type": ty}),
        ),
        Op::AddVendor { name } => ("books.add_vendor", json!({"name": name})),
        Op::CreateBill { party, amount } => (
            "books.create_bill",
            json!({"party_id": party.resolve(&k.vendors), "date": "2026-03-01",
                   "due_date": "2026-03-31", "lines": [{"description": "thing", "amount": amount}]}),
        ),
        Op::PayBill { doc, amount } => (
            "books.pay_bill",
            json!({"document_id": doc.resolve(&k.bills), "date": "2026-03-15", "amount": amount,
                   "cash_account": "1000"}),
        ),
        Op::Payout { deposit } => (
            "books.record_payout",
            json!({"date": "2026-03-31", "description": "payout", "deposit_account": "1000",
                   "deposit": deposit, "revenue": [{"account": "4000", "amount": "100.00"}],
                   "fees": [{"account": "6999", "amount": "15.00"}]}),
        ),
        Op::Renumber { account, code } => (
            "books.renumber_account",
            json!({"account": account, "code": code}),
        ),
        Op::Read(i) => match i {
            0 => ("books.chart", json!({})),
            1 => ("books.trial_balance", json!({"as_of": "2026-12-31"})),
            2 => (
                "books.get_entry",
                json!({"entry_id": Pick::Known(0).resolve(&k.entries)}),
            ),
            3 => ("books.list_receipts", json!({})),
            4 => ("books.list_periods", json!({})),
            _ => ("books.digest", json!({"since": "2026-01-01"})),
        },
        Op::UnknownTool => ("books.no_such_tool", json!({})),
        Op::MissingArgs => ("books.add_line", json!({"entry_id": 1})),
        Op::WrongTypes => (
            "books.post_entry",
            json!({"entry_id": "one", "confirmed": "yes"}),
        ),
    };
    // Mcp::tool checks invariants 1, 2, 3 (schema, text) and 7.
    let out = m.tool(tool, args);
    match tool {
        "books.draft_entry" | "books.record_payout" => k.entries.extend(new_id(&out, "entry_id")),
        "books.reverse_entry" => k.entries.extend(new_id(&out, "reversal_id")),
        "books.attach_receipt" => k.attachments.extend(new_id(&out, "attachment_id")),
        "books.create_period" => k.periods.extend(new_id(&out, "period_id")),
        "books.add_vendor" => k.vendors.extend(new_id(&out, "id")),
        "books.create_bill" => {
            k.bills.extend(new_id(&out, "id"));
            k.entries.extend(new_id(&out, "entry_id"));
        }
        _ => {}
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: cases(), failure_persistence: None, ..ProptestConfig::default() })]

    /// Invariants 2 and 7 under random call sequences: every call gets
    /// exactly one response with its id, result xor error; any refusal
    /// leaves the journal byte-identical; successes match their schemas;
    /// and the server keeps answering afterwards.
    #[test]
    fn inv2_inv7_random_calls_keep_books_and_server_intact(
        ops in prop::collection::vec(op_strategy(), 30..60)
    ) {
        let dir = test_dir("sm");
        let toml = books_toml(&dir, "25.00", "");
        let mut m = Mcp::start(&dir, &toml);
        m.accounts(BASE_CHART);
        let mut known = Known::default();
        for op in &ops {
            run_op(&mut m, &mut known, op);
        }
        // Still alive and consistent at the end.
        m.ok("books.chart", json!({}));
        let tb = m.ok("books.trial_balance", json!({"as_of": "2026-12-31"}));
        prop_assert_eq!(&tb["balanced"], &json!(true));
        drop(m);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

// ---------------------------------------------- invariant 3: amounts in

const REFUSED_AMOUNTS: &[&str] = &[
    "1e3", "1E3", "+5", ".5", "5.", "1_000", "1,000", " 5", "5 ", "", "-", "abc", "١٢٣", "5.0.0",
    "0x10", "5..0", "--5", "5-", "Infinity", "NaN", "５",
];

/// Grammatical amounts with at most two places and a positive value: what
/// the core is expected to accept on a line.
const ACCEPTED_AMOUNTS: &[&str] = &["5", "5.00", "123.45", "5.5", "007.50", "1000000.00"];

fn amounts_server(name: &str) -> (PathBuf, Mcp, u64) {
    let dir = test_dir(name);
    let toml = books_toml(&dir, "25.00", "");
    let mut m = Mcp::start(&dir, &toml);
    m.accounts(BASE_CHART);
    let e = m.draft("2026-03-01", "amounts", &[]);
    (dir, m, e)
}

#[test]
fn inv3_add_line_amount_grammar_table() {
    let (_dir, mut m, e) = amounts_server("inv3-table");
    for a in REFUSED_AMOUNTS {
        m.refuse(
            "books.add_line",
            json!({"entry_id": e, "account": "6000", "side": "debit", "amount": a}),
        );
    }
    for a in ACCEPTED_AMOUNTS {
        m.ok(
            "books.add_line",
            json!({"entry_id": e, "account": "6000", "side": "debit", "amount": a}),
        );
        // Amounts out: the stored line keeps the value, never rounded.
        let entry = m.ok("books.get_entry", json!({"entry_id": e}));
        let last = entry["lines"].as_array().unwrap().last().unwrap().clone();
        let out = last["amount"].as_str().expect("amount is a string");
        assert_eq!(canon(out), canon(a), "line amount {out} for input {a}");
    }
    assert!(m.money_violations.is_empty(), "{:#?}", m.money_violations);
}

/// The amount grammar applies to every amount-taking tool, not just
/// add_line.
#[test]
fn inv3_other_tools_refuse_ungrammatical_amounts() {
    let (_dir, mut m, e) = amounts_server("inv3-other");
    m.ok(
        "books.add_line",
        json!({"entry_id": e, "account": "6000", "side": "debit", "amount": "10.00"}),
    );
    m.ok(
        "books.add_line",
        json!({"entry_id": e, "account": "1000", "side": "credit", "amount": "10.00"}),
    );
    let vendor = m.ok("books.add_vendor", json!({"name": "V"}))["id"].clone();
    for bad in ["1e1", "+10.00", "10,00", " 10.00", "10.00 "] {
        m.refuse(
            "books.post_entry",
            json!({"entry_id": e, "confirmed": true, "expected_total": bad}),
        );
        m.refuse(
            "books.create_bill",
            json!({"party_id": vendor, "date": "2026-03-01", "due_date": "2026-03-31",
                   "lines": [{"description": "x", "amount": bad}]}),
        );
        m.refuse(
            "books.reconcile",
            json!({"account": "1000", "statement_date": "2026-03-31", "statement_balance": bad}),
        );
        m.refuse(
            "books.record_payout",
            json!({"date": "2026-03-31", "description": "p", "deposit_account": "1000",
                   "deposit": bad, "revenue": [{"account": "4000", "amount": "10.00"}]}),
        );
        m.refuse(
            "books.create_schedule",
            json!({"kind": "prepaid_expense", "description": "s", "total": bad,
                   "start": "2026-01-01", "months": 12, "balance_account": "1200",
                   "pl_account": "6000"}),
        );
    }
}

fn amount_char() -> impl Strategy<Value = char> {
    prop::sample::select(vec![
        '0', '1', '5', '9', '0', '1', '5', '9', '.', '.', '-', '+', 'e', 'E', '_', ',', ' ', '١',
        'x',
    ])
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 16, failure_persistence: None, ..ProptestConfig::default() })]

    /// Random strings: ungrammatical ones are refused; grammatical ones
    /// with at most two places and a positive value are accepted and come
    /// back with the same value. Others (zero, negative, sub-cent) are the
    /// core's business and aren't asserted.
    #[test]
    fn inv3_add_line_amount_grammar_random(
        amounts in prop::collection::vec(
            prop::collection::vec(amount_char(), 0..9).prop_map(|cs| cs.into_iter().collect::<String>()),
            20..40,
        )
    ) {
        let (dir, mut m, e) = amounts_server("inv3-prop");
        for a in &amounts {
            let out = m.tool(
                "books.add_line",
                json!({"entry_id": e, "account": "6000", "side": "debit", "amount": a}),
            );
            if !is_amount_in(a) {
                prop_assert!(out.refused(), "ungrammatical {:?} accepted", a);
                continue;
            }
            let places = a.split_once('.').map_or(0, |(_, f)| f.len());
            let positive = !a.starts_with('-') && canon(a) != "0";
            if places <= 2 && positive && a.len() <= 7 {
                prop_assert!(!out.refused(), "grammatical {:?} refused: {:?}", a, out);
                let entry = m.ok("books.get_entry", json!({"entry_id": e}));
                let last = entry["lines"].as_array().unwrap().last().unwrap().clone();
                prop_assert_eq!(canon(last["amount"].as_str().unwrap()), canon(a));
            }
        }
        prop_assert!(m.money_violations.is_empty(), "{:#?}", m.money_violations);
        drop(m);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

// ----------------------------------------- invariant 4: preview changes nothing

#[test]
fn inv4_post_and_reverse_preview_without_confirmed() {
    let dir = test_dir("inv4");
    let toml = books_toml(&dir, "1000000.00", "");
    let mut m = Mcp::start(&dir, &toml);
    m.accounts(BASE_CHART);
    let e = m.draft(
        "2026-04-01",
        "Rent",
        &[("6000", "debit", "40.00"), ("1000", "credit", "40.00")],
    );
    m.attach(Some(e), b"rent");

    for args in [
        json!({"entry_id": e}),
        json!({"entry_id": e, "confirmed": false}),
        json!({"entry_id": e, "confirmed": null}),
    ] {
        let before = m.journal();
        let p = m.ok("books.post_entry", args.clone());
        assert_eq!(p["status"], "preview", "{args}: {p}");
        assert!(m.journal() == before, "post_entry preview {args} wrote");
        assert_eq!(m.entry_status(e), "draft");
    }
    let before = m.journal();
    let posted = m.post(e).ok("confirmed post");
    assert_eq!(posted["status"], "posted");
    assert!(m.journal() != before, "confirmed post wrote nothing");
    assert_eq!(m.entry_status(e), "posted");

    let rev = json!({"entry_id": e, "date": "2026-04-02", "description": "undo"});
    for extra in [json!({}), json!({"confirmed": false})] {
        let mut args = rev.clone();
        args.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        let before = m.journal();
        let p = m.ok("books.reverse_entry", args.clone());
        assert_eq!(p["status"], "preview", "{args}: {p}");
        assert!(m.journal() == before, "reverse_entry preview {args} wrote");
    }
    let mut args = rev;
    args["confirmed"] = json!(true);
    let before = m.journal();
    let r = m.ok("books.reverse_entry", args);
    assert_eq!(r["status"], "posted", "{r}");
    assert!(m.journal() != before);
    let tb = m.ok("books.trial_balance", json!({"as_of": "2026-12-31"}));
    assert_eq!(tb["balanced"], true);
}

// ------------------------------------------ invariant 5: receipt policy

#[test]
fn inv5_threshold_is_strictly_greater_than() {
    let dir = test_dir("inv5-25");
    let toml = books_toml(&dir, "25.00", "");
    let mut m = Mcp::start(&dir, &toml);
    m.accounts(BASE_CHART);
    let at = m.draft(
        "2026-05-01",
        "At threshold",
        &[("6000", "debit", "25.00"), ("1000", "credit", "25.00")],
    );
    m.post(at).ok("25.00 posts without a receipt");
    let over = m.draft(
        "2026-05-02",
        "Over threshold",
        &[("6000", "debit", "25.01"), ("1000", "credit", "25.01")],
    );
    let preview = m.ok("books.post_entry", json!({"entry_id": over}));
    assert_eq!(preview["receipt_missing"], true, "{preview}");
    let err = m.post(over).err_text();
    assert!(err.contains("receipt_required"), "{err}");
    assert_eq!(m.entry_status(over), "draft");
    m.attach(Some(over), b"receipt for 25.01");
    m.post(over).ok("25.01 posts with a receipt");
}

#[test]
fn inv5_zero_threshold_needs_a_receipt_on_every_entry() {
    let dir = test_dir("inv5-0");
    let toml = books_toml(&dir, "0.00", "");
    let mut m = Mcp::start(&dir, &toml);
    m.accounts(BASE_CHART);
    let e = m.draft(
        "2026-05-01",
        "A penny",
        &[("6000", "debit", "0.01"), ("1000", "credit", "0.01")],
    );
    let err = m.post(e).err_text();
    assert!(err.contains("receipt_required"), "{err}");
    m.attach(Some(e), b"penny receipt");
    m.post(e).ok("with a receipt");
    // Keep posted: the last receipt can't be unlinked from a covered entry.
    let receipts = m.ok("books.list_receipts", json!({"entry_id": e}));
    let att = receipts["receipts"][0]["attachment_id"].clone();
    let err = m.refuse(
        "books.unlink_receipt",
        json!({"attachment_id": att, "reason": "try"}),
    );
    assert!(err.contains("receipt_required"), "{err}");
}

#[test]
fn inv5_splitting_does_not_dodge_the_threshold() {
    let dir = test_dir("inv5-split");
    let toml = books_toml(&dir, "25.00", "");
    let mut m = Mcp::start(&dir, &toml);
    m.accounts(BASE_CHART);
    let a = m.draft(
        "2026-05-03",
        "Office supplies",
        &[("6999", "debit", "20.00"), ("1000", "credit", "20.00")],
    );
    m.post(a).ok("first half posts");
    // Same day, same description (case and surrounding whitespace aside),
    // same debit account: grouped, 40.00 > 25.00.
    let b = m.draft(
        "2026-05-03",
        "  OFFICE supplies ",
        &[("6999", "debit", "20.00"), ("1000", "credit", "20.00")],
    );
    let err = m.post(b).err_text();
    assert!(err.contains("receipt_required"), "{err}");
    // Not the signature: another day, or another debit account.
    let c = m.draft(
        "2026-05-04",
        "Office supplies",
        &[("6999", "debit", "20.00"), ("1000", "credit", "20.00")],
    );
    m.post(c).ok("another day");
    let d = m.draft(
        "2026-05-03",
        "Office supplies",
        &[("6000", "debit", "20.00"), ("1000", "credit", "20.00")],
    );
    m.post(d).ok("another debit account");
    // A receipt clears it.
    m.attach(Some(b), b"supplies receipt");
    m.post(b).ok("split with a receipt");
}

#[test]
fn inv5_reversals_are_exempt() {
    let dir = test_dir("inv5-rev");
    let toml = books_toml(&dir, "0.00", "");
    let mut m = Mcp::start(&dir, &toml);
    m.accounts(BASE_CHART);
    let e = m.draft(
        "2026-05-01",
        "Big",
        &[("6000", "debit", "500.00"), ("1000", "credit", "500.00")],
    );
    m.attach(Some(e), b"big receipt");
    m.post(e).ok("post");
    m.ok(
        "books.reverse_entry",
        json!({"entry_id": e, "date": "2026-05-02", "description": "undo", "confirmed": true}),
    );
}

/// Policy invariant 4: a threshold that isn't a decimal, or is below
/// zero, is refused at startup.
#[test]
fn inv5_bad_threshold_refused_at_startup() {
    for bad in ["-1", "abc", "-0.01", "", "1,000", "ten"] {
        let dir = test_dir("inv5-startup");
        let toml = books_toml(&dir, bad, "");
        let mut child = spawn_child(&dir, &toml);
        let mut stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let init = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2025-06-18", "capabilities": {},
            "clientInfo": {"name": "spec-test", "version": "0.1.0"}}});
        let _ = writeln!(stdin, "{init}");
        let _ = stdin.flush();
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let mut line = String::new();
            let _ = BufReader::new(stdout).read_line(&mut line);
            let _ = tx.send(line);
        });
        let line = rx.recv_timeout(TIMEOUT).unwrap_or_default();
        let answered = serde_json::from_str::<Value>(&line)
            .ok()
            .is_some_and(|v| v.get("result").is_some());
        drop(stdin);
        let status = wait_with_timeout(&mut child, TIMEOUT);
        assert!(
            !answered,
            "threshold {bad:?}: server completed initialize: {line}"
        );
        assert!(
            status.is_some_and(|s| !s.success()),
            "threshold {bad:?}: server did not exit nonzero ({status:?})"
        );
    }
}

fn wait_with_timeout(child: &mut Child, timeout: Duration) -> Option<std::process::ExitStatus> {
    let start = std::time::Instant::now();
    loop {
        if let Ok(Some(s)) = child.try_wait() {
            return Some(s);
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

// --------------------------------------- invariant 6: ambiguous references

#[test]
fn inv6_ambiguous_account_reference_refused() {
    let dir = test_dir("inv6-acct");
    let toml = books_toml(&dir, "1000000.00", "");
    let mut m = Mcp::start(&dir, &toml);
    // Account "2" first, then others; find one whose id is 2 and whose
    // code is not "2".
    let ids = m.accounts(&[
        ("2", "Two by code", "expense"),
        ("1000", "Bank", "asset"),
        ("6000", "Rent", "expense"),
    ]);
    assert!(
        ids.contains(&2) && ids[0] != 2,
        "need an account with id 2 that is not code \"2\": ids {ids:?}"
    );
    let e = m.draft("2026-06-01", "ambiguity", &[]);
    let err = m.refuse(
        "books.add_line",
        json!({"entry_id": e, "account": "2", "side": "debit", "amount": "10.00"}),
    );
    assert!(!err.is_empty());
    m.refuse(
        "books.reconcile",
        json!({"account": "2", "statement_date": "2026-06-30", "statement_balance": "0.00"}),
    );
    m.refuse(
        "books.set_tax_category",
        json!({"account": "2", "category": null}),
    );
    m.refuse(
        "books.renumber_account",
        json!({"account": "2", "code": "6100"}),
    );
    // Unique references work: a code nobody's id, an id nobody's code.
    m.ok(
        "books.add_line",
        json!({"entry_id": e, "account": "6000", "side": "debit", "amount": "10.00"}),
    );
    let unique_id = ids
        .iter()
        .copied()
        .find(|&i| i != 2 && !["2", "1000", "6000"].contains(&i.to_string().as_str()))
        .expect("an id that is no account's code");
    m.ok(
        "books.add_line",
        json!({"entry_id": e, "account": unique_id.to_string(), "side": "credit", "amount": "10.00"}),
    );
    let entry = m.ok("books.get_entry", json!({"entry_id": e}));
    assert_eq!(entry["lines"].as_array().unwrap().len(), 2, "{entry}");
}

#[test]
fn inv6_ambiguous_vendor_reference_refused() {
    let dir = test_dir("inv6-vendor");
    let toml = books_toml(&dir, "1000000.00", "");
    let mut m = Mcp::start(&dir, &toml);
    m.accounts(BASE_CHART);
    let acme = m.ok("books.add_vendor", json!({"name": "Acme"}))["id"]
        .as_u64()
        .unwrap();
    // A vendor literally named after Acme's id.
    let named = m.ok("books.add_vendor", json!({"name": acme.to_string()}))["id"]
        .as_u64()
        .unwrap();
    assert_ne!(acme, named);
    let amb = acme.to_string();
    m.refuse(
        "books.set_vendor_1099",
        json!({"vendor": amb, "form_1099": "nec", "tin_on_file": false}),
    );
    m.refuse(
        "books.draft_entry",
        json!({"date": "2026-06-01", "description": "x", "vendor": amb}),
    );
    let e = m.draft("2026-06-01", "tag me", &[]);
    m.refuse(
        "books.set_entry_vendor",
        json!({"entry_id": e, "vendor": amb}),
    );
    // Unique: the name "Acme", and the second vendor's id (no vendor has
    // that name).
    m.ok(
        "books.set_vendor_1099",
        json!({"vendor": "Acme", "form_1099": "nec", "tin_on_file": false}),
    );
    let out = m.ok(
        "books.set_entry_vendor",
        json!({"entry_id": e, "vendor": named.to_string()}),
    );
    assert!(out.get("vendor").is_some(), "{out}");
}

#[test]
fn inv6_ambiguous_class_reference_refused() {
    let dir = test_dir("inv6-class");
    let toml = books_toml(&dir, "1000000.00", "");
    let mut m = Mcp::start(&dir, &toml);
    m.accounts(BASE_CHART);
    let alpha = m.ok("books.add_class", json!({"name": "Alpha"}))["id"]
        .as_u64()
        .unwrap();
    let named = m.ok("books.add_class", json!({"name": alpha.to_string()}))["id"]
        .as_u64()
        .unwrap();
    assert_ne!(alpha, named);
    let e = m.draft("2026-06-01", "classy", &[]);
    // add_line with an ambiguous class is refused too, but writes the
    // line first: see add_line_bad_class_leaves_journal_unchanged.
    m.refuse("books.profit_and_loss", json!({"class": alpha.to_string()}));

    m.ok(
        "books.add_line",
        json!({"entry_id": e, "account": "6000", "side": "debit", "amount": "10.00", "class": "Alpha"}),
    );
    m.ok(
        "books.add_line",
        json!({"entry_id": e, "account": "1000", "side": "credit", "amount": "10.00",
               "class": named.to_string()}),
    );
    m.ok("books.profit_and_loss", json!({"class": "Alpha"}));
}

/// Invariant 2: a refused add_line leaves the journal unchanged, here when
/// the line's class doesn't resolve (unknown, or ambiguous per invariant
/// 6).
#[test]
fn add_line_bad_class_leaves_journal_unchanged() {
    let dir = test_dir("bug-class");
    let toml = books_toml(&dir, "25.00", "");
    let mut m = Mcp::start(&dir, &toml);
    m.accounts(BASE_CHART);
    m.ok("books.add_class", json!({"name": "Alpha"}));
    m.ok("books.add_class", json!({"name": "1"}));
    let e = m.draft("2026-06-01", "classy", &[]);
    for class in ["Nope", "99", "1"] {
        // Mcp::tool asserts the journal is unchanged on refusal.
        m.refuse(
            "books.add_line",
            json!({"entry_id": e, "account": "6000", "side": "debit", "amount": "10.00",
                   "class": class}),
        );
    }
    let entry = m.ok("books.get_entry", json!({"entry_id": e}));
    assert_eq!(entry["lines"], json!([]), "{entry}");
}

/// Invariant 6 and "validate all tool inputs": profit_and_loss_by_class
/// declares a `class` filter; an unknown or ambiguous class must be
/// refused, not ignored.
#[test]
fn profit_and_loss_by_class_validates_class() {
    let dir = test_dir("bug-pnl-class");
    let toml = books_toml(&dir, "1000000.00", "");
    let mut m = Mcp::start(&dir, &toml);
    m.accounts(BASE_CHART);
    m.ok("books.add_class", json!({"name": "Alpha"}));
    m.ok("books.add_class", json!({"name": "1"}));
    m.refuse("books.profit_and_loss_by_class", json!({"class": "Nope"}));
    m.refuse("books.profit_and_loss_by_class", json!({"class": "1"}));
}

/// Invariant 2 for other class-taking tools: a payout with an unknown
/// class is refused and creates no draft.
#[test]
fn payout_bad_class_leaves_journal_unchanged() {
    let dir = test_dir("payout-class");
    let toml = books_toml(&dir, "25.00", "");
    let mut m = Mcp::start(&dir, &toml);
    m.accounts(BASE_CHART);
    m.refuse(
        "books.record_payout",
        json!({"date": "2026-03-31", "description": "p", "deposit_account": "1000",
               "deposit": "85.00", "revenue": [{"account": "4000", "amount": "100.00", "class": "Nope"}],
               "fees": [{"account": "6999", "amount": "15.00", "class": "Nope"}]}),
    );
}

// --------------------------------------------- JSON-RPC 2.0 conformance

/// mcp-tools.md: "Unknown tools" are protocol errors (JSON-RPC errors),
/// not isError results.
#[test]
fn jsonrpc_unknown_tool_is_a_protocol_error() {
    let dir = test_dir("rpc-tool");
    let toml = books_toml(&dir, "25.00", "");
    let mut m = Mcp::start(&dir, &toml);
    let out = m.tool("books.no_such_tool", json!({}));
    assert!(
        matches!(out, Out::RpcErr(_)),
        "unknown tool should be a JSON-RPC error, got {out:?}"
    );
    m.ok("books.chart", json!({}));
}

/// jsonrpc-2.0.md §5.1: unknown method → -32601.
#[test]
fn jsonrpc_unknown_method_is_32601() {
    let dir = test_dir("rpc-method");
    let toml = books_toml(&dir, "25.00", "");
    let mut m = Mcp::start(&dir, &toml);
    let resp = m.rpc("books/does_not_exist", json!({}));
    assert_eq!(resp["error"]["code"], -32601, "{resp}");
    m.ok("books.chart", json!({}));
}

/// jsonrpc-2.0.md §5 and §5.1: invalid JSON → a response with code
/// -32700 and id null; the server keeps serving.
#[test]
fn jsonrpc_parse_error_is_32700_with_null_id() {
    let dir = test_dir("rpc-parse");
    let toml = books_toml(&dir, "25.00", "");
    let mut m = Mcp::start(&dir, &toml);
    m.send_line("{\"jsonrpc\": \"2.0\", \"id\": 99, \"method\": ");
    let resp = m.recv(Duration::from_secs(5));
    let resp = resp.expect("no response to a malformed JSON line");
    check_response_shape(&resp);
    assert_eq!(resp["error"]["code"], -32700, "{resp}");
    assert_eq!(resp["id"], Value::Null, "{resp}");
}

/// jsonrpc-2.0.md §5: `id` is REQUIRED in every response; when it can't
/// be detected (Invalid Request) it MUST be null, else the request's.
#[test]
fn jsonrpc_invalid_request_response_has_id() {
    let dir = test_dir("rpc-invalid");
    let toml = books_toml(&dir, "25.00", "");
    let mut m = Mcp::start(&dir, &toml);
    for (junk, id) in [
        ("[]", Value::Null),
        ("{\"jsonrpc\": \"2.0\", \"id\": 98}", json!(98)),
    ] {
        m.send_line(junk);
        let resp = m.recv(Duration::from_secs(5)).expect("no response");
        assert_eq!(resp["error"]["code"], -32600, "{resp}");
        assert!(
            resp.get("id").is_some(),
            "{junk}: response has no id: {resp}"
        );
        // A detectable id should be echoed; Null when it isn't.
        assert!(resp["id"] == id || resp["id"].is_null(), "{junk}: {resp}");
    }
}

/// Whatever the server does with a malformed line, it keeps serving
/// (invariant 7).
#[test]
fn jsonrpc_server_survives_malformed_input() {
    let dir = test_dir("rpc-survive");
    let toml = books_toml(&dir, "25.00", "");
    let mut m = Mcp::start(&dir, &toml);
    m.accounts(BASE_CHART);
    for junk in [
        "{\"jsonrpc\": \"2.0\", \"id\": 99, \"method\": ",
        "not json at all",
        "[]",
        "{\"jsonrpc\": \"2.0\", \"id\": 98}",
    ] {
        m.send_line(junk);
        // Drain anything it answered with, without requiring an answer
        // (the missing id is jsonrpc_invalid_request_response_has_id).
        while let Some(v) = m.recv(Duration::from_millis(300)) {
            assert_eq!(v["jsonrpc"], "2.0", "{v}");
            assert!(v.get("result").is_some() ^ v.get("error").is_some(), "{v}");
        }
        m.ok("books.chart", json!({}));
    }
}

/// Missing or mistyped arguments: mcp-tools.md lists "invalid arguments"
/// as protocol errors and "invalid input data" as tool errors, so either
/// is accepted; the books must not change and the server must survive.
#[test]
fn jsonrpc_invalid_arguments_refused_cleanly() {
    let dir = test_dir("rpc-args");
    let toml = books_toml(&dir, "25.00", "");
    let mut m = Mcp::start(&dir, &toml);
    m.accounts(BASE_CHART);
    m.refuse("books.add_line", json!({"entry_id": 1}));
    m.refuse("books.post_entry", json!({"entry_id": "one"}));
    m.refuse(
        "books.draft_entry",
        json!({"date": 20260101, "description": "x"}),
    );
    m.refuse("books.add_account", json!({}));
    m.ok("books.chart", json!({}));
}

#[test]
fn helpers_self_check() {
    for ok in ["0.00", "-12.34", "1.234"] {
        assert!(is_money_out(ok), "{ok}");
    }
    for bad in ["0", "1.2", "1e3", "", "-", ".50", "1,000.00"] {
        assert!(!is_money_out(bad), "{bad}");
    }
    for ok in ["0", "5", "-5", "5.00", "007.5"] {
        assert!(is_amount_in(ok), "{ok}");
    }
    for bad in REFUSED_AMOUNTS {
        assert!(!is_amount_in(bad), "{bad}");
    }
    assert_eq!(canon("007.50"), "7.5");
    assert_eq!(canon("5.00"), "5");
    assert_eq!(canon("-0.00"), "0");
    assert_eq!(b64(b"abcd"), "YWJjZA==");
    let mut bad = Vec::new();
    money_walk(
        &json!({"total": 5, "lines": [{"amount": "1.5"}], "id": 3}),
        "$",
        None,
        &mut bad,
    );
    assert_eq!(bad.len(), 2, "{bad:?}");
}

// ------------------------------------- settings reload (settings.rs spec)

/// INVARIANT 1: a call after an edit sees it, with no restart, and two
/// same-size edits in a row are both seen (compared by content).
#[test]
fn settings_reload_on_the_next_call() {
    let dir = test_dir("reload");
    let toml = books_toml(&dir, "25.00", "");
    let mut m = Mcp::start(&dir, &toml);
    let s = m.ok("books.settings", json!({}));
    assert_eq!(s["company"], "Spec Co");
    assert_eq!(s["loads"], 1);
    for name in ["Aaaa Co", "Bbbb Co"] {
        let text = std::fs::read_to_string(&toml)
            .unwrap()
            .replace("Spec Co", name)
            .replace("Aaaa Co", name);
        std::fs::write(&toml, text).unwrap();
        let s = m.ok("books.settings", json!({}));
        assert_eq!(s["company"], name);
        assert_eq!(s["reload_error"], Value::Null);
    }
    assert_eq!(m.ok("books.settings", json!({}))["loads"], 3);
}

/// INVARIANT 1, enforced: a lowered receipt threshold refuses the next
/// post without a receipt.
#[test]
fn a_reloaded_threshold_is_enforced() {
    let dir = test_dir("reload-threshold");
    let toml = books_toml(&dir, "1000.00", "");
    let mut m = Mcp::start(&dir, &toml);
    m.accounts(BASE_CHART);
    let first = m.draft(
        "2026-03-01",
        "small",
        &[("6000", "debit", "1.00"), ("1000", "credit", "1.00")],
    );
    m.post(first).ok("under the threshold");
    let text = std::fs::read_to_string(&toml)
        .unwrap()
        .replace("\"1000.00\"", "\"0.00\"");
    std::fs::write(&toml, text).unwrap();
    let second = m.draft(
        "2026-03-02",
        "small",
        &[("6000", "debit", "1.00"), ("1000", "credit", "1.00")],
    );
    let out = m.post(second);
    assert!(out.err_text().contains("receipt_required"), "{out:?}");
    assert_eq!(m.entry_status(second), "draft");
}

/// INVARIANTS 2 and 3: a broken edit, and a named file gone missing, keep
/// the last good settings and say why; fixing it loads again.
#[test]
fn a_broken_edit_keeps_the_last_good_settings() {
    let dir = test_dir("reload-broken");
    let toml = books_toml(&dir, "25.00", "");
    let good = std::fs::read_to_string(&toml).unwrap();
    let mut m = Mcp::start(&dir, &toml);
    std::fs::write(
        &toml,
        good.replace("Spec Co", "Next Co")
            .replace("[books]", "[books"),
    )
    .unwrap();
    let s = m.ok("books.settings", json!({}));
    assert_eq!(s["company"], "Spec Co");
    assert!(s["reload_error"].is_string(), "{s}");
    let digest = m.ok("books.digest", json!({"since": "2026-01-01"}));
    assert!(
        digest["settings_problems"][0]
            .as_str()
            .unwrap()
            .contains("didn't load"),
        "{digest}"
    );
    std::fs::remove_file(&toml).unwrap();
    let s = m.ok("books.settings", json!({}));
    assert!(
        s["reload_error"].is_string(),
        "a named file gone missing: {s}"
    );
    assert_eq!(s["books_toml_exists"], false);
    assert_eq!(s["company"], "Spec Co");
    std::fs::write(&toml, good.replace("Spec Co", "Fixed Co")).unwrap();
    let s = m.ok("books.settings", json!({}));
    assert_eq!(s["company"], "Fixed Co");
    assert_eq!(s["reload_error"], Value::Null);
    assert_eq!(s["loads"], 2);
}

/// books.settings and the digest list settings that disagree with the
/// chart, each naming the setting.
#[test]
fn settings_that_disagree_with_the_chart_are_problems() {
    let dir = test_dir("problems");
    let toml = books_toml(
        &dir,
        "25.00",
        "financing_accounts = [\"6000\"]\nclose_into = \"3000\"\n",
    );
    let mut m = Mcp::start(&dir, &toml);
    m.accounts(BASE_CHART);
    let s = m.ok("books.settings", json!({}));
    let problems: Vec<String> = s["problems"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p.as_str().unwrap().to_string())
        .collect();
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].contains("[books] financing_accounts names 6000"));
    let text = std::fs::read_to_string(&toml)
        .unwrap()
        .replace("cash_accounts = [\"1000\"]", "cash_accounts = [\"1010\"]")
        .replace("close_into = \"3000\"", "close_into = \"4000\"");
    std::fs::write(&toml, text).unwrap();
    let digest = m.ok("books.digest", json!({"since": "2026-01-01"}));
    let problems = digest["settings_problems"].to_string();
    assert!(
        problems.contains("[books] cash_accounts names 1010, which isn't in the chart"),
        "{problems}"
    );
    assert!(
        problems.contains("[books] close_into names 4000"),
        "{problems}"
    );
    assert!(
        problems.contains("[books] financing_accounts names 6000"),
        "{problems}"
    );
}

/// books.assign_role steers what an invoice posts, and books.roles shows
/// it; a refusal names the reason.
#[test]
fn assigned_roles_steer_invoices() {
    let dir = test_dir("roles");
    let toml = books_toml(&dir, "1000000.00", "");
    let mut m = Mcp::start(&dir, &toml);
    m.accounts(&[
        ("1000", "Bank", "asset"),
        ("1110", "Trade Receivables", "asset"),
        ("4100", "Consulting Income", "revenue"),
        ("6000", "Rent", "expense"),
    ]);
    let roles = m.ok("books.roles", json!({}));
    let receivables = roles["roles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["role"] == "receivables")
        .unwrap()
        .clone();
    assert_eq!(receivables["default_code"], "1200");
    assert_eq!(receivables["assigned"], false);
    assert_eq!(receivables["account_code"], Value::Null);
    let err = m.refuse(
        "books.assign_role",
        json!({"role": "receivables", "account": "6000"}),
    );
    assert!(err.contains("accounts receivable"), "{err}");
    let err = m.refuse(
        "books.assign_role",
        json!({"role": "receivable", "account": "1110"}),
    );
    assert!(err.contains("no role"), "{err}");
    m.ok(
        "books.assign_role",
        json!({"role": "receivables", "account": "1110"}),
    );
    let roles = m.ok(
        "books.assign_role",
        json!({"role": "revenue", "account": "4100"}),
    );
    let revenue = roles["roles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["role"] == "revenue")
        .unwrap()
        .clone();
    assert_eq!(revenue["assigned"], true);
    assert_eq!(revenue["account_code"], "4100");
    let customer = m.ok("books.add_customer", json!({"name": "Client"}))["id"]
        .as_u64()
        .unwrap();
    m.ok(
        "books.create_invoice",
        json!({"party_id": customer, "date": "2026-02-01", "due_date": "2026-03-01",
               "lines": [{"description": "Consulting", "amount": "500.00"}]}),
    );
    let tb = m.ok("books.trial_balance", json!({})).to_string();
    assert!(tb.contains("1110") && tb.contains("4100"), "{tb}");
    assert!(!tb.contains("\"1200\"") && !tb.contains("\"4000\""), "{tb}");
}
