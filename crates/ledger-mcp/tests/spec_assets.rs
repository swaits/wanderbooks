//! Spec-blind tests for the fixed-asset tag tools of `ledger-mcp`, driven
//! over stdio against the real binary.
//!
//! Expected behavior comes only from the tool descriptions and parameter /
//! result doc comments in `src/server.rs` and `src/types.rs` (the declared
//! interface, as `tools/list` serves it), `docs/mcp-interface.md` ("Tools —
//! fixed assets"), and `docs/spec/rules.md` B-24. The harness is the
//! minimal subset of `tests/spec_mcp.rs`: every success validates against
//! the tool's `outputSchema`, and every refusal leaves `journal.jsonl`
//! byte-identical (server invariant 2: a refused call changes nothing).

use serde_json::{Value, json};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(180);

// ---------------------------------------------------------------- harness

#[derive(Debug, Clone)]
enum Out {
    Ok(Value),
    ToolErr(String),
    RpcErr(Value),
}

impl Out {
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
    validators: HashMap<String, jsonschema::Validator>,
}

impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

static DIR_SEQ: AtomicU64 = AtomicU64::new(0);

fn test_dir(name: &str) -> PathBuf {
    let n = DIR_SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "ledger-mcp-spec-assets-{}-{name}-{n}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// `wanderbooks.toml` with a company name and extra `[books]` TOML. The
/// receipt threshold is high so ordinary test entries need no receipt.
fn books_toml(dir: &Path, company: &str, books_extra: &str) -> PathBuf {
    let path = dir.join("wanderbooks.toml");
    std::fs::write(
        &path,
        format!(
            "[company]\nname = \"{company}\"\nfiscal_year_start = \"01-01\"\n\
             [books]\nreceipt_threshold = \"1000000.00\"\ncash_accounts = [\"1000\"]\n{books_extra}"
        ),
    )
    .unwrap();
    path
}

impl Mcp {
    fn start(dir: &Path, toml: &Path) -> Self {
        let run = dir.join("run");
        std::fs::create_dir_all(&run).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_ledger-mcp"))
            .arg("--data-dir")
            .arg(dir.join("books-data"))
            .arg("--books-toml")
            .arg(toml)
            .env("XDG_RUNTIME_DIR", &run)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn ledger-mcp");
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
            validators: HashMap::new(),
        };
        let init = mcp.rpc(
            "initialize",
            json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "spec-assets", "version": "0.1.0"},
            }),
        );
        assert!(init.get("result").is_some(), "initialize failed: {init}");
        mcp.send_line(
            &json!({"jsonrpc": "2.0", "method": "notifications/initialized"}).to_string(),
        );
        let list = mcp.rpc("tools/list", json!({}));
        for tool in list["result"]["tools"].as_array().expect("tools") {
            let name = tool["name"].as_str().unwrap().to_string();
            let validator = jsonschema::validator_for(&tool["outputSchema"])
                .unwrap_or_else(|e| panic!("{name}'s outputSchema is not valid: {e}"));
            mcp.validators.insert(name, validator);
        }
        mcp
    }

    fn journal(&self) -> Vec<u8> {
        std::fs::read(self.data.join("journal.jsonl")).unwrap_or_default()
    }

    /// The journal's line types, in order.
    fn journal_types(&self) -> Vec<String> {
        String::from_utf8(self.journal())
            .unwrap()
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| {
                let v: Value = serde_json::from_str(l).unwrap();
                v["type"]
                    .as_str()
                    .or_else(|| v["line_type"].as_str())
                    .unwrap_or_default()
                    .to_string()
            })
            .collect()
    }

    fn send_line(&mut self, line: &str) {
        writeln!(self.stdin, "{line}").unwrap();
        self.stdin.flush().unwrap();
    }

    fn recv(&mut self) -> Option<Value> {
        loop {
            match self.rx.recv_timeout(TIMEOUT) {
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

    fn rpc(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let msg = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        self.send_line(&msg.to_string());
        let resp = self
            .recv()
            .unwrap_or_else(|| panic!("no response to {method} (id {id})"));
        assert_eq!(resp["id"], json!(id), "response id mismatch: {resp}");
        resp
    }

    /// Call a tool; a refusal must leave the journal unchanged, a success
    /// must match the outputSchema.
    fn tool(&mut self, name: &str, args: Value) -> Out {
        let before = self.journal();
        let resp = self.rpc("tools/call", json!({"name": name, "arguments": args}));
        let out = if let Some(err) = resp.get("error") {
            Out::RpcErr(err.clone())
        } else {
            let result = &resp["result"];
            if result.get("isError") == Some(&json!(true)) {
                Out::ToolErr(
                    result["content"]
                        .as_array()
                        .and_then(|c| c.iter().find_map(|b| b["text"].as_str()))
                        .unwrap_or_default()
                        .to_string(),
                )
            } else {
                let sc = result["structuredContent"].clone();
                let v = self
                    .validators
                    .get(name)
                    .unwrap_or_else(|| panic!("{name} is not in tools/list"));
                let errors: Vec<String> = v.iter_errors(&sc).map(|e| e.to_string()).collect();
                assert!(
                    errors.is_empty(),
                    "{name} violates its outputSchema: {errors:?}\n{sc}"
                );
                Out::Ok(sc)
            }
        };
        if out.refused() {
            assert!(
                self.journal() == before,
                "refused {name} {args} changed the journal ({out:?})"
            );
        }
        out
    }

    fn ok(&mut self, name: &str, args: Value) -> Value {
        let a = args.to_string();
        match self.tool(name, args) {
            Out::Ok(v) => v,
            other => panic!("{name} {a}: expected success, got {other:?}"),
        }
    }

    /// `tool`, expecting a refusal that says why.
    fn refuse(&mut self, name: &str, args: Value) -> String {
        let a = args.to_string();
        let out = self.tool(name, args);
        assert!(out.refused(), "{name} {a} should be refused, got {out:?}");
        let why = out.err_text();
        assert!(!why.trim().is_empty(), "{name} {a}: a refusal says why");
        why
    }

    /// `ok`, and the journal must not change (read-only tools).
    fn read_only(&mut self, name: &str, args: Value) -> Value {
        let before = self.journal();
        let v = self.ok(name, args);
        assert!(
            self.journal() == before,
            "read-only {name} changed the journal"
        );
        v
    }

    fn accounts(&mut self, accts: &[(&str, &str, &str)]) {
        for (code, name, ty) in accts {
            self.ok(
                "books.add_account",
                json!({"code": code, "name": name, "account_type": ty}),
            );
        }
    }

    /// Post an entry (high receipt threshold: none needed).
    fn post(&mut self, date: &str, lines: &[(&str, &str, &str)]) {
        let id = self.ok(
            "books.draft_entry",
            json!({"date": date, "description": "test entry"}),
        )["entry_id"]
            .as_u64()
            .unwrap();
        for (account, side, amount) in lines {
            self.ok(
                "books.add_line",
                json!({"entry_id": id, "account": account, "side": side, "amount": amount}),
            );
        }
        self.ok(
            "books.post_entry",
            json!({"entry_id": id, "confirmed": true}),
        );
    }

    /// Register a $1,200, 12-month laptop from 2026-01-01 with `extra` args
    /// (tag, serial, description).
    fn register(&mut self, extra: Value) -> Out {
        let mut args = json!({"description": "Laptop", "purchase_date": "2026-01-01",
            "cost": "1200.00", "salvage_value": "0.00", "useful_life_months": 12,
            "cost_account": "1500", "accum_depr_account": "1510", "expense_account": "6500"});
        for (k, v) in extra.as_object().unwrap() {
            args[k] = v.clone();
        }
        self.tool("books.register_asset", args)
    }

    fn register_ok(&mut self, extra: Value) -> (u64, String) {
        match self.register(extra.clone()) {
            Out::Ok(v) => (
                v["asset_id"].as_u64().unwrap(),
                v["tag"].as_str().unwrap().to_string(),
            ),
            other => panic!("register {extra}: {other:?}"),
        }
    }

    fn assets(&mut self) -> Vec<Value> {
        self.ok("books.list_assets", json!({}))["assets"]
            .as_array()
            .unwrap()
            .clone()
    }

    fn asset(&mut self, id: u64) -> Value {
        self.assets()
            .into_iter()
            .find(|a| a["asset_id"] == json!(id))
            .unwrap_or_else(|| panic!("asset {id} not listed"))
    }

    fn file_bill_of_sale(&mut self) -> u64 {
        self.ok(
            "books.file_document",
            json!({"category": "legal", "file_name": "bill-of-sale.txt", "mime_type": "text/plain",
                   "content_base64": b64(b"bill of sale"), "description": "Bill of sale"}),
        )["attachment_id"]
            .as_u64()
            .unwrap()
    }
}

const CHART: &[(&str, &str, &str)] = &[
    ("1000", "Bank", "asset"),
    ("1500", "Equipment", "asset"),
    ("1510", "Accumulated Depreciation", "asset"),
    ("3000", "Capital", "equity"),
    ("6500", "Depreciation Expense", "expense"),
    ("7100", "Gain or Loss on Disposal", "revenue"),
];

/// Books with the chart, generated tags prefixed "WL".
fn books(name: &str) -> Mcp {
    let dir = test_dir(name);
    let toml = books_toml(&dir, "Spec Co", "asset_tag_prefix = \"WL\"\n");
    let mut m = Mcp::start(&dir, &toml);
    m.accounts(CHART);
    m
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

fn unb64(s: &str) -> Vec<u8> {
    let val = |c: u8| -> u32 {
        match c {
            b'A'..=b'Z' => u32::from(c - b'A'),
            b'a'..=b'z' => u32::from(c - b'a') + 26,
            b'0'..=b'9' => u32::from(c - b'0') + 52,
            b'+' => 62,
            b'/' => 63,
            _ => panic!("not base64: {c}"),
        }
    };
    let clean: Vec<u8> = s.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    let mut out = Vec::new();
    for chunk in clean.chunks(4) {
        let pad = chunk.iter().filter(|&&b| b == b'=').count();
        let mut n = 0u32;
        for (i, &b) in chunk.iter().enumerate() {
            n |= if b == b'=' { 0 } else { val(b) } << (18 - 6 * i);
        }
        let bytes = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
        out.extend_from_slice(&bytes[..3 - pad]);
    }
    out
}

/// Decimal strings compared by value.
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

/// Every value under `key` anywhere in `v`.
fn find_key<'a>(v: &'a Value, key: &str, out: &mut Vec<&'a Value>) {
    match v {
        Value::Object(m) => {
            for (k, child) in m {
                if k == key {
                    out.push(child);
                }
                find_key(child, key, out);
            }
        }
        Value::Array(a) => a.iter().for_each(|c| find_key(c, key, out)),
        _ => {}
    }
}

fn the_value(v: &Value, key: &str) -> String {
    let mut found = Vec::new();
    find_key(v, key, &mut found);
    assert_eq!(found.len(), 1, "{key} in {v}");
    match found[0] {
        Value::String(s) => canon(s),
        other => panic!("{key} is not a decimal string: {other}"),
    }
}

// ------------------------------------------------- books.register_asset

/// "Without `tag`, the next free `PREFIX-0001` ([books] asset_tag_prefix)";
/// result `{ asset_id, tag }` (the tag given or generated).
#[test]
fn register_generates_the_next_tag_from_the_prefix() {
    let mut m = books("gen");
    let (a, ta) = m.register_ok(json!({}));
    assert_eq!(ta, "WL-0001");
    let (_, tb) = m.register_ok(json!({"description": "Monitor"}));
    assert_eq!(tb, "WL-0002");
    let (c, tc) = m.register_ok(json!({"tag": "DESK-9", "serial": "SN 9"}));
    assert_eq!(tc, "DESK-9");
    let (_, td) = m.register_ok(json!({}));
    assert_eq!(td, "WL-0003");
    // list_assets carries tag and serial.
    let la = m.asset(a);
    assert_eq!(la["tag"], "WL-0001");
    assert_eq!(la["serial"], Value::Null);
    let lc = m.asset(c);
    assert_eq!(lc["tag"], "DESK-9");
    assert_eq!(lc["serial"], "SN 9");
}

/// "else the company name's initials without LLC/Inc".
#[test]
fn register_default_prefix_is_the_company_initials() {
    let dir = test_dir("initials");
    let toml = books_toml(&dir, "Wanderland Labs LLC", "");
    let mut m = Mcp::start(&dir, &toml);
    m.accounts(CHART);
    let (_, t) = m.register_ok(json!({}));
    assert_eq!(t, "WL-0001");
}

/// "\"FA\" when none."
#[test]
fn register_default_prefix_falls_back_to_fa() {
    let dir = test_dir("fa");
    let toml = books_toml(&dir, "LLC", "");
    let mut m = Mcp::start(&dir, &toml);
    m.accounts(CHART);
    let (_, t) = m.register_ok(json!({}));
    assert_eq!(t, "FA-0001");
}

/// A bad tag or serial registers nothing (the harness checks the journal
/// is unchanged on every refusal), and the next generated tag is unmoved.
#[test]
fn register_with_a_bad_tag_or_serial_registers_nothing() {
    let mut m = books("bad-register");
    let (_, t) = m.register_ok(json!({}));
    assert_eq!(t, "WL-0001");
    let n = m.assets().len();
    for extra in [
        json!({"tag": "has space"}),
        json!({"tag": ""}),
        json!({"tag": "Ü-1"}),
        json!({"tag": "A".repeat(33)}),
        json!({"tag": "---"}),
        json!({"tag": "wl-0001"}), // taken, ignoring case
        json!({"serial": " lead"}),
        json!({"serial": "trail "}),
        json!({"serial": ""}),
        json!({"serial": "ctl\u{1}"}),
        json!({"serial": "S".repeat(65)}),
    ] {
        assert!(
            m.register(extra.clone()).refused(),
            "{extra} should be refused"
        );
    }
    assert_eq!(m.assets().len(), n, "nothing registered");
    let (_, t) = m.register_ok(json!({}));
    assert_eq!(t, "WL-0002");
}

/// A generated tag is never one any asset held (B-24).
#[test]
fn register_never_generates_a_released_tag() {
    let mut m = books("released");
    let (a, _) = m.register_ok(json!({}));
    m.ok(
        "books.label_asset",
        json!({"asset": a.to_string(), "tag": "LAPTOP-A"}),
    );
    let (_, t) = m.register_ok(json!({}));
    assert_eq!(t, "WL-0002");
    // And a released tag can't be given to a new asset explicitly.
    assert!(m.register(json!({"tag": "WL-0001"})).refused());
}

// ------------------------------------------------- books.label_asset

/// "The asset is its id or current tag"; result `{ asset_id, tag, serial }`;
/// journaled as `asset.tagged`, `asset.serial_set`.
#[test]
fn label_asset_sets_tag_and_serial_by_id_or_tag() {
    let mut m = books("label");
    let (a, _) = m.register_ok(json!({}));
    let before = m.journal_types().len();
    let r = m.ok(
        "books.label_asset",
        json!({"asset": a.to_string(), "serial": "C02XG0FDJGH5"}),
    );
    assert_eq!(r["asset_id"], json!(a));
    assert_eq!(r["tag"], "WL-0001", "omitted tag is kept");
    assert_eq!(r["serial"], "C02XG0FDJGH5");
    let r = m.ok(
        "books.label_asset",
        json!({"asset": "wl-0001", "tag": "LAPTOP-1"}),
    );
    assert_eq!(r["asset_id"], json!(a));
    assert_eq!(r["tag"], "LAPTOP-1");
    assert_eq!(r["serial"], "C02XG0FDJGH5", "omitted serial is kept");
    let types = m.journal_types();
    let new: Vec<&str> = types[before..].iter().map(String::as_str).collect();
    assert!(new.contains(&"asset.serial_set"), "{new:?}");
    assert!(new.contains(&"asset.tagged"), "{new:?}");
    // clear_serial.
    let r = m.ok(
        "books.label_asset",
        json!({"asset": "LAPTOP-1", "clear_serial": true}),
    );
    assert_eq!(r["serial"], Value::Null);
    assert_eq!(r["tag"], "LAPTOP-1");
    let l = m.asset(a);
    assert_eq!(l["tag"], "LAPTOP-1");
    assert_eq!(l["serial"], Value::Null);
    // The old tag is no longer current: it names nothing.
    m.refuse(
        "books.label_asset",
        json!({"asset": "WL-0001", "serial": "x"}),
    );
}

/// Tags are unique ignoring case and never reused by another asset, even
/// after a retag; bad forms and unknown assets are refused.
#[test]
fn label_asset_refusals() {
    let mut m = books("label-refuse");
    let (a, _) = m.register_ok(json!({}));
    let (b, _) = m.register_ok(json!({}));
    m.ok(
        "books.label_asset",
        json!({"asset": a.to_string(), "tag": "LAPTOP-A"}),
    );
    // b may not take a's current tag (any case) or its released WL-0001.
    for t in ["laptop-a", "LAPTOP-A", "WL-0001", "wl-0001"] {
        m.refuse(
            "books.label_asset",
            json!({"asset": b.to_string(), "tag": t}),
        );
    }
    for t in ["bad tag", "", "--", "É"] {
        m.refuse(
            "books.label_asset",
            json!({"asset": b.to_string(), "tag": t}),
        );
    }
    for s in ["", " x", "x ", "a\nb"] {
        m.refuse(
            "books.label_asset",
            json!({"asset": b.to_string(), "serial": s}),
        );
    }
    m.refuse("books.label_asset", json!({"asset": "999", "tag": "NEW-1"}));
    m.refuse(
        "books.label_asset",
        json!({"asset": "NOPE-1", "tag": "NEW-1"}),
    );
    // a may take back its own old tag.
    let r = m.ok(
        "books.label_asset",
        json!({"asset": "LAPTOP-A", "tag": "WL-0001"}),
    );
    assert_eq!(r["asset_id"], json!(a));
    assert_eq!(m.asset(b)["tag"], "WL-0002");
}

/// resolve_asset_ref: "A tag wins: tags like \"1001\" are allowed".
#[test]
fn an_asset_ref_that_is_a_tag_means_the_tagged_asset() {
    let mut m = books("tag-wins");
    let (a, _) = m.register_ok(json!({}));
    let (b, _) = m.register_ok(json!({"tag": a.to_string()}));
    assert_ne!(a, b);
    let r = m.ok(
        "books.label_asset",
        json!({"asset": a.to_string(), "serial": "which"}),
    );
    assert_eq!(r["asset_id"], json!(b), "the tag {a} wins over the id {a}");
    assert_eq!(m.asset(b)["serial"], "which");
    assert_eq!(m.asset(a)["serial"], Value::Null);
}

// ------------------------------------------------- tag in place of id

/// `books.asset_schedule { asset_id | tag }`: the same schedule either way.
#[test]
fn asset_schedule_takes_the_tag_in_place_of_the_id() {
    let mut m = books("schedule");
    let (a, t) = m.register_ok(json!({}));
    let by_id = m.read_only("books.asset_schedule", json!({"asset_id": a}));
    let by_tag = m.read_only("books.asset_schedule", json!({"tag": t}));
    let by_lower = m.read_only(
        "books.asset_schedule",
        json!({"tag": t.to_ascii_lowercase()}),
    );
    assert_eq!(by_id, by_tag);
    assert_eq!(by_id, by_lower);
    assert_eq!(by_tag["asset_id"], json!(a));
    // Exactly one of asset_id and tag.
    m.refuse("books.asset_schedule", json!({}));
    m.refuse("books.asset_schedule", json!({"asset_id": a, "tag": t}));
    m.refuse("books.asset_schedule", json!({"tag": "NOPE-1"}));
}

/// `books.dispose_asset { asset_id | tag, ... }`: the same preview either
/// way; confirmed by tag disposes it; its tag stays reserved.
#[test]
fn dispose_asset_takes_the_tag_and_the_tag_stays_reserved() {
    let mut m = books("dispose");
    m.post(
        "2026-01-01",
        &[("1000", "debit", "5000.00"), ("3000", "credit", "5000.00")],
    );
    m.post(
        "2026-01-01",
        &[("1500", "debit", "1200.00"), ("1000", "credit", "1200.00")],
    );
    let (a, t) = m.register_ok(json!({"serial": "SN-A"}));
    let (b, tb) = m.register_ok(json!({}));
    let base = json!({"date": "2026-06-30", "proceeds": "500.00",
                      "proceeds_account": "1000", "gain_loss_account": "7100"});
    let mut by_id = base.clone();
    by_id["asset_id"] = json!(a);
    let mut by_tag = base.clone();
    by_tag["tag"] = json!(t.to_ascii_lowercase());
    let p1 = m.ok("books.dispose_asset", by_id.clone());
    let p2 = m.ok("books.dispose_asset", by_tag.clone());
    assert_eq!(p1, p2, "same preview by id and by tag");
    let mut both = by_id.clone();
    both["tag"] = json!(t);
    m.refuse("books.dispose_asset", both);
    m.refuse("books.dispose_asset", base.clone());
    let doc = m.file_bill_of_sale();
    let mut confirm = by_tag;
    confirm["confirmed"] = json!(true);
    confirm["receipt_attachment_ids"] = json!([doc]);
    m.ok("books.dispose_asset", confirm);
    assert_eq!(m.asset(a)["status"], "disposed");
    // B-24: a disposed asset's tag is never reused.
    m.refuse(
        "books.label_asset",
        json!({"asset": b.to_string(), "tag": t}),
    );
    assert!(m.register(json!({"tag": t})).refused());
    // Default labels: active tagged assets only.
    let labels = m.read_only("books.asset_labels", json!({}));
    assert_eq!(labels["tags"], json!([tb]));
}

// ------------------------------------------------- books.asset_labels

#[test]
fn asset_labels_prints_a_pdf_of_the_tags_in_order() {
    let mut m = books("labels");
    let (_, t1) = m.register_ok(json!({"serial": "SN-1"}));
    let (_, t2) = m.register_ok(json!({}));
    let (a3, t3) = m.register_ok(json!({"tag": "DESK/7"}));
    let r = m.read_only("books.asset_labels", json!({"assets": [t3, t1]}));
    assert_eq!(r["tags"], json!([t3, t1]), "the tags printed, in order");
    let pdf = unb64(r["content_base64"].as_str().unwrap());
    assert!(pdf.starts_with(b"%PDF"), "a PDF");
    assert!(!r["file_name"].as_str().unwrap().is_empty());
    // By id too.
    let r = m.read_only("books.asset_labels", json!({"assets": [a3.to_string()]}));
    assert_eq!(r["tags"], json!(["DESK/7"]));
    // Default: every active asset with a tag, all three here.
    let r = m.read_only("books.asset_labels", json!({}));
    let mut tags: Vec<String> = r["tags"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    tags.sort();
    let mut want = vec![t1.clone(), t2.clone(), t3.clone()];
    want.sort();
    assert_eq!(tags, want);
    // skip: 0 to 29.
    let r = m.read_only("books.asset_labels", json!({"assets": [t1], "skip": 29}));
    assert!(unb64(r["content_base64"].as_str().unwrap()).starts_with(b"%PDF"));
    m.refuse("books.asset_labels", json!({"assets": [t2], "skip": 30}));
    // Unknown asset.
    m.refuse("books.asset_labels", json!({"assets": ["NOPE-1"]}));
}

/// "asset {} has no tag: give it one with books.label_asset first": an
/// asset with no tag can't be printed. Assets registered before tags
/// existed have none; here one is made by the store directly.
#[test]
fn asset_labels_refuses_an_asset_without_a_tag() {
    let dir = test_dir("untagged");
    let toml = books_toml(&dir, "Spec Co", "asset_tag_prefix = \"WL\"\n");
    // Books written by the store, outside the server: an untagged asset.
    {
        use ledger_core::{AccountType, Currency, Money, NewAsset};
        let data = dir.join("books-data");
        std::fs::create_dir_all(&data).unwrap();
        let mut s = ledger_store::Store::create(&data, Currency::USD).unwrap();
        let mut add = |c: &str, n: &str, t| s.add_account(c, n, t, None, false).unwrap();
        let cost = add("1500", "Equipment", AccountType::Asset);
        let accum = add("1510", "Accumulated Depreciation", AccountType::Asset);
        let exp = add("6500", "Depreciation Expense", AccountType::Expense);
        let usd = |a: &str| Money::new(a.parse().unwrap(), Currency::USD);
        s.register_asset(NewAsset {
            description: "Old laptop".into(),
            purchase_date: chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            cost: usd("1200.00"),
            salvage_value: usd("0.00"),
            useful_life_months: 12,
            cost_account: cost,
            accum_depr_account: accum,
            expense_account: exp,
        })
        .unwrap();
    }
    let mut m = Mcp::start(&dir, &toml);
    let assets = m.assets();
    assert_eq!(assets.len(), 1);
    assert_eq!(assets[0]["tag"], Value::Null);
    let id = assets[0]["asset_id"].as_u64().unwrap();
    m.refuse("books.asset_labels", json!({"assets": [id.to_string()]}));
    // The default (every active tagged asset) has nothing to print, or
    // prints nothing; either way it never prints the untagged one.
    if let Out::Ok(r) = m.tool("books.asset_labels", json!({})) {
        assert_eq!(r["tags"], json!([]));
    }
    // Tag it and it prints.
    m.ok(
        "books.label_asset",
        json!({"asset": id.to_string(), "tag": "OLD-1"}),
    );
    let r = m.read_only("books.asset_labels", json!({"assets": ["OLD-1"]}));
    assert_eq!(r["tags"], json!(["OLD-1"]));
}

// ------------------------------------------------- books.show { asset }

#[test]
fn show_opens_an_asset_by_tag_or_id() {
    let mut m = books("show");
    let (a, t) = m.register_ok(json!({}));
    m.read_only("books.show", json!({"asset": t}));
    m.read_only("books.show", json!({"asset": a.to_string()}));
    m.read_only("books.show", json!({"report": "fixed_asset_register"}));
    m.read_only("books.show", json!({"page": "assets"}));
    // Exactly one target.
    m.refuse("books.show", json!({"asset": t, "page": "assets"}));
}

// ------------------------------------------------- export fixed_asset_register

/// `books.export { report: "fixed_asset_register", format: json | csv,
/// as_of? }`: tag, serial, ..., `unposted_depreciation` and
/// `unregistered_cost` (zero when register and books agree).
#[test]
fn export_fixed_asset_register_reconciles_to_the_books() {
    let mut m = books("export");
    m.post(
        "2026-01-01",
        &[("1000", "debit", "5000.00"), ("3000", "credit", "5000.00")],
    );
    m.post(
        "2026-01-01",
        &[("1500", "debit", "1200.00"), ("1000", "credit", "1200.00")],
    );
    // $1,200 over 12 months from Jan 1: $100 a month.
    m.register_ok(json!({"tag": "LAPTOP-1", "serial": "C02XG0FDJGH5"}));
    m.ok("books.run_depreciation", json!({"period": "2026-01"}));
    m.ok("books.run_depreciation", json!({"period": "2026-02"}));

    let export = |m: &mut Mcp, format: &str, as_of: &str| -> String {
        let r = m.read_only(
            "books.export",
            json!({"report": "fixed_asset_register", "format": format, "as_of": as_of}),
        );
        assert_eq!(r["report"], "fixed_asset_register");
        assert_eq!(r["format"], format);
        r["content"].as_str().unwrap().to_string()
    };

    // Agreeing: depreciation posted through February.
    let j: Value = serde_json::from_str(&export(&mut m, "json", "2026-02-28")).unwrap();
    assert_eq!(the_value(&j, "unposted_depreciation"), "0");
    assert_eq!(the_value(&j, "unregistered_cost"), "0");
    let text = j.to_string();
    assert!(
        text.contains("LAPTOP-1") && text.contains("C02XG0FDJGH5"),
        "{text}"
    );

    // March due by schedule, not posted.
    let j: Value = serde_json::from_str(&export(&mut m, "json", "2026-03-31")).unwrap();
    assert_eq!(the_value(&j, "unposted_depreciation"), "100");
    assert_eq!(the_value(&j, "unregistered_cost"), "0");

    // Equipment in the books that no asset accounts for.
    m.post(
        "2026-03-15",
        &[("1500", "debit", "250.00"), ("1000", "credit", "250.00")],
    );
    let j: Value = serde_json::from_str(&export(&mut m, "json", "2026-03-31")).unwrap();
    assert_eq!(the_value(&j, "unregistered_cost"), "250");

    let csv = export(&mut m, "csv", "2026-02-28");
    assert!(csv.contains("LAPTOP-1"), "{csv}");
    assert!(csv.contains("C02XG0FDJGH5"), "{csv}");
    let header = csv.lines().next().unwrap().to_ascii_lowercase();
    assert!(header.contains("tag"), "{header}");
    assert!(header.contains("serial"), "{header}");
}

/// The register lists assets registered by the date only.
#[test]
fn export_fixed_asset_register_as_of_leaves_out_later_assets() {
    let mut m = books("export-later");
    m.register_ok(json!({"tag": "EARLY-1"}));
    m.register_ok(json!({"tag": "LATE-1", "purchase_date": "2026-06-01"}));
    let r = m.read_only(
        "books.export",
        json!({"report": "fixed_asset_register", "format": "json", "as_of": "2026-03-31"}),
    );
    let text = r["content"].as_str().unwrap();
    assert!(text.contains("EARLY-1"));
    assert!(!text.contains("LATE-1"), "{text}");
}
