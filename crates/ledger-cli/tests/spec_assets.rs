//! Spec-blind tests for the fixed-asset tag tools of the books server,
//! driven through a real `wanderbooks server` as an agent reaches it
//! (common.rs).
//!
//! Expected behavior comes only from the tool descriptions and parameter /
//! result doc comments in `src/server.rs` and `src/types.rs` (the declared
//! interface, as `tools/list` serves it), `docs/mcp-interface.md` ("Tools —
//! fixed assets"), and `docs/spec/rules.md` B-24. The harness is the
//! minimal subset of `tests/spec_mcp.rs`: every success validates against
//! the tool's `outputSchema`, and every refusal leaves `journal.jsonl`
//! byte-identical (server invariant 2: a refused call changes nothing).

use common::{Place, Wire};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

mod common;

const TIMEOUT: Duration = common::TIMEOUT;

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
    wire: Wire,
    next_id: u64,
    data: PathBuf,
    validators: HashMap<String, jsonschema::Validator>,
    /// The test's place, when the session owns it (its server is closed
    /// and its home removed after the session).
    books_dir: Option<TestDir>,
}

/// A test's place (common.rs): its home, its server, its books.
type TestDir = Place;

fn test_dir(name: &str) -> TestDir {
    Place::new(name)
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
    fn start(dir: &TestDir, toml: &Path) -> Self {
        let wire = dir.start(Some(&std::fs::read_to_string(toml).unwrap()));
        let mut mcp = Self {
            books_dir: None,
            wire,
            next_id: 1,
            data: dir.books_data(),
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
        self.wire.send_line(line);
    }

    fn recv(&mut self) -> Option<Value> {
        self.wire.recv(TIMEOUT)
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
        let resp = common::tools_call(|m, p| self.rpc(m, p), name, args.clone());
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
        let mut args = json!({"description": "Laptop", "in_service_date": "2026-01-01",
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
        let upload = self.wire.upload(
            b"bill of sale",
            "bill-of-sale.txt",
            "text/plain",
            "document",
        );
        self.ok(
            "books.file_document",
            json!({"category": "legal", "upload_id": upload, "description": "Bill of sale"}),
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

/// Books with the chart, generated tags prefixed "FL".
fn books(name: &str) -> Mcp {
    let dir = test_dir(name);
    let toml = books_toml(&dir, "Spec Co", "asset_tag_prefix = \"FL\"\n");
    let mut m = Mcp::start(&dir, &toml);
    m.accounts(CHART);
    m.books_dir = Some(dir);
    m
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
    assert_eq!(ta, "FL-0001");
    let (_, tb) = m.register_ok(json!({"description": "Monitor"}));
    assert_eq!(tb, "FL-0002");
    let (c, tc) = m.register_ok(json!({"tag": "DESK-9", "serial": "SN 9"}));
    assert_eq!(tc, "DESK-9");
    let (_, td) = m.register_ok(json!({}));
    assert_eq!(td, "FL-0003");
    // list_assets carries tag and serial.
    let la = m.asset(a);
    assert_eq!(la["tag"], "FL-0001");
    assert_eq!(la["serial"], Value::Null);
    let lc = m.asset(c);
    assert_eq!(lc["tag"], "DESK-9");
    assert_eq!(lc["serial"], "SN 9");
}

/// "else the company name's initials without LLC/Inc".
#[test]
fn register_default_prefix_is_the_company_initials() {
    let dir = test_dir("initials");
    let toml = books_toml(&dir, "Fernhill Labs LLC", "");
    let mut m = Mcp::start(&dir, &toml);
    m.accounts(CHART);
    let (_, t) = m.register_ok(json!({}));
    assert_eq!(t, "FL-0001");
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
    assert_eq!(t, "FL-0001");
    let n = m.assets().len();
    for extra in [
        json!({"tag": "has space"}),
        json!({"tag": ""}),
        json!({"tag": "Ü-1"}),
        json!({"tag": "A".repeat(33)}),
        json!({"tag": "---"}),
        json!({"tag": "fl-0001"}), // taken, ignoring case
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
    assert_eq!(t, "FL-0002");
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
    assert_eq!(t, "FL-0002");
    // And a released tag can't be given to a new asset explicitly.
    assert!(m.register(json!({"tag": "FL-0001"})).refused());
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
    assert_eq!(r["tag"], "FL-0001", "omitted tag is kept");
    assert_eq!(r["serial"], "C02XG0FDJGH5");
    let r = m.ok(
        "books.label_asset",
        json!({"asset": "fl-0001", "tag": "LAPTOP-1"}),
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
        json!({"asset": "FL-0001", "serial": "x"}),
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
    // b may not take a's current tag (any case) or its released FL-0001.
    for t in ["laptop-a", "LAPTOP-A", "FL-0001", "fl-0001"] {
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
        json!({"asset": "LAPTOP-A", "tag": "FL-0001"}),
    );
    assert_eq!(r["asset_id"], json!(a));
    assert_eq!(m.asset(b)["tag"], "FL-0002");
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
    // (The digest names the arguments as sent, so it differs by id and
    // by tag, and so does the confirm step the envelope offers, which
    // repeats them; everything the preview shows is the same.)
    let (mut p1, mut p2) = (p1, p2);
    for p in [&mut p1, &mut p2] {
        p.as_object_mut().unwrap().remove("preview");
        p.as_object_mut().unwrap().remove("next_steps");
    }
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
    // Handed over (D-46): its bytes come by its download, never here.
    assert!(r.get("content_base64").is_none(), "{r}");
    let pdf = m.wire.fetch(&r["file"]);
    assert!(pdf.starts_with(b"%PDF"), "a PDF");
    assert_eq!(r["file"]["mime_type"], "application/pdf");
    assert!(!r["file"]["file_name"].as_str().unwrap().is_empty());
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
    assert!(m.wire.fetch(&r["file"]).starts_with(b"%PDF"));
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
    let toml = books_toml(&dir, "Spec Co", "asset_tag_prefix = \"FL\"\n");
    // Books written by the store, outside the server: an untagged asset.
    {
        use ledger_core::{AccountType, Currency, Money, NewAsset};
        let data = dir.books_data();
        std::fs::create_dir_all(&data).unwrap();
        let mut s = ledger_store::Store::create(&data, Currency::USD).unwrap();
        let mut add = |c: &str, n: &str, t| s.add_account(c, n, t, None, false).unwrap();
        let cost = add("1500", "Equipment", AccountType::Asset);
        let accum = add("1510", "Accumulated Depreciation", AccountType::Asset);
        let exp = add("6500", "Depreciation Expense", AccountType::Expense);
        let usd = |a: &str| Money::new(a.parse().unwrap(), Currency::USD);
        s.register_asset(NewAsset {
            description: "Old laptop".into(),
            in_service_date: chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
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
        m.wire.fetch_text(&r["file"])
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
    m.register_ok(json!({"tag": "LATE-1", "in_service_date": "2026-06-01"}));
    let r = m.read_only(
        "books.export",
        json!({"report": "fixed_asset_register", "format": "json", "as_of": "2026-03-31"}),
    );
    let text = &m.wire.fetch_text(&r["file"]);
    assert!(text.contains("EARLY-1"));
    assert!(!text.contains("LATE-1"), "{text}");
}

// ------------------------------------------------- tax basis (rules.md B-25)
//
// From the `books.set_asset_tax_basis` and `books.register_asset`
// descriptions, `SetAssetTaxBasisParams` / `AssetBasisOut` /
// `RegisterAssetParams` docs, and the fixed_asset_register export.
// Purchase: the book cost (IRC §1012); carryover_723: the member's
// adjusted basis (IRC §723); personal_use_conversion: §723 basis as for
// carryover, depreciable basis the lesser of fair market value and that
// basis (Treas. Reg. §1.168(i)-4(b)); built_in_gain_704c = value less the
// §723 basis at contribution (Treas. Reg. §1.704-3(a)(3)(ii)).

/// A decimal-string field, compared by value; `None` for null.
fn dec(v: &Value, key: &str) -> Option<String> {
    match &v[key] {
        Value::Null => None,
        Value::String(s) => Some(canon(s)),
        other => panic!("{key} is not a decimal string or null: {other}"),
    }
}

fn basis_lines(m: &Mcp) -> usize {
    m.journal_types()
        .iter()
        .filter(|t| *t == "asset.basis_set")
        .count()
}

/// books.set_asset_tax_basis by tag and by id; the output names each
/// figure for what it is, with its authority; each call journals an
/// asset.basis_set; the old rule names are aliases, noted.
#[test]
fn set_asset_tax_basis_by_tag_or_id_reports_each_figure() {
    let mut m = books("tax-set");
    let (a, t) = m.register_ok(json!({}));
    assert_eq!(m.asset(a)["basis"], Value::Null, "not recorded yet");

    // By tag (any case): the member's $900 basis in a $1,200 laptop from
    // business use.
    let r = m.ok(
        "books.set_asset_tax_basis",
        json!({"asset": t.to_ascii_lowercase(), "rule": "carryover_723",
               "contributor_adjusted_basis": "900.00", "fmv_at_contribution": "1200.00",
               "sec704c_method": "traditional",
               "basis_source": {"decided_by": "Morgan Ellis", "decided_on": "2026-10-20",
                                "citation": "entry 1"}}),
    );
    assert_eq!(r["asset_id"], json!(a));
    assert!(r.get("notes").is_none(), "{r}");
    let b = &r["basis"];
    assert_eq!(b["rule"], "carryover_723");
    assert_eq!(dec(b, "contributor_adjusted_basis").as_deref(), Some("900"));
    assert_eq!(dec(b, "fmv_at_contribution").as_deref(), Some("1200"));
    assert_eq!(b["fmv_recorded"], true);
    assert_eq!(dec(b, "tax_basis_723").as_deref(), Some("900"));
    assert_eq!(dec(b, "depreciable_basis").as_deref(), Some("900"));
    assert_eq!(dec(b, "built_in_gain_704c").as_deref(), Some("300"));
    assert!(b.get("sec704c_1c_note").is_none(), "a gain has no note");
    assert_eq!(b["sec704c_method"], "traditional");
    assert_eq!(b["source"]["decided_by"], "Morgan Ellis");
    assert_eq!(b["sources"]["tax_basis_723"], "IRC §723");
    assert_eq!(b["sources"]["depreciable_basis"], "IRC §723");
    assert_eq!(
        b["sources"]["built_in_gain_704c"],
        "Treas. Reg. §1.704-3(a)(3)(ii)"
    );
    assert_eq!(basis_lines(&m), 1);
    assert_eq!(
        m.journal_types()
            .iter()
            .filter(|t| *t == "journal.format_set")
            .count(),
        1,
        "the feature declared once"
    );

    // By id: personal use, the member's $2,000 laptop worth $1,200.
    let r = m.ok(
        "books.set_asset_tax_basis",
        json!({"asset": a.to_string(), "rule": "personal_use_conversion",
               "contributor_adjusted_basis": "2000.00", "fmv_at_contribution": "1200.00"}),
    );
    let b = &r["basis"];
    assert_eq!(dec(b, "tax_basis_723").as_deref(), Some("2000"));
    assert_eq!(
        dec(b, "depreciable_basis").as_deref(),
        Some("1200"),
        "the lesser"
    );
    assert_eq!(dec(b, "built_in_gain_704c").as_deref(), Some("-800"));
    assert!(
        b["sec704c_1c_note"]
            .as_str()
            .unwrap()
            .contains("IRC §704(c)(1)(C)"),
        "{b}"
    );
    assert_eq!(
        b["sources"]["depreciable_basis"],
        "Treas. Reg. §1.168(i)-4(b)"
    );
    assert_eq!(b["sec704c_method"], Value::Null);

    // A purchase: the book cost, nothing contributed.
    let r = m.ok(
        "books.set_asset_tax_basis",
        json!({"asset": t, "rule": "purchase"}),
    );
    let b = &r["basis"];
    assert_eq!(b["rule"], "purchase");
    assert_eq!(b["contributor_adjusted_basis"], Value::Null);
    assert_eq!(b["tax_basis_723"], Value::Null);
    assert_eq!(dec(b, "depreciable_basis").as_deref(), Some("1200"));
    assert_eq!(b["built_in_gain_704c"], Value::Null);
    assert_eq!(b["sources"]["depreciable_basis"], "IRC §1012");
    assert_eq!(basis_lines(&m), 3);

    // list_assets shows the latest record.
    let basis = m.asset(a)["basis"].clone();
    assert_eq!(basis["rule"], "purchase");
}

/// The old names (rule cost, carryover; contributor_basis) are aliases for
/// one release, noted in the result; a name and its alias that disagree
/// are refused.
#[test]
fn basis_rule_legacy_names_are_aliases() {
    let mut m = books("tax-alias");
    let (_, t) = m.register_ok(json!({}));
    let r = m.ok(
        "books.set_asset_tax_basis",
        json!({"asset": t, "rule": "carryover", "contributor_basis": "900.00",
               "fmv_at_contribution": "1200.00"}),
    );
    assert_eq!(r["basis"]["rule"], "carryover_723");
    let notes = r["notes"].to_string();
    assert!(
        notes.contains("contributor_basis is now contributor_adjusted_basis"),
        "{notes}"
    );
    assert!(
        notes.contains("\\\"carryover\\\" is now \\\"carryover_723\\\""),
        "{notes}"
    );
    let r = m.ok(
        "books.set_asset_tax_basis",
        json!({"asset": t, "rule": "cost"}),
    );
    assert_eq!(r["basis"]["rule"], "purchase");
    let why = m.refuse(
        "books.set_asset_tax_basis",
        json!({"asset": t, "rule": "carryover_723", "contributor_adjusted_basis": "1.00",
               "contributor_basis": "2.00", "fmv_at_contribution": "3.00"}),
    );
    assert!(why.contains("disagree"), "{why}");
    let (_, t2) = m.register_ok(json!({"tax_basis_rule": "carryover",
        "contributor_basis": "400.00", "fmv_at_contribution": "1200.00"}));
    assert_eq!(m.assets().len(), 2);
    let _ = t2;
}

/// A name and its alias that mean the same, however written, agree (P3
/// review, 2026-10-21: they were compared as text, so "100" and "100.00",
/// or carryover and carryover_723, were refused as disagreeing).
#[test]
fn a_name_and_its_alias_that_mean_the_same_agree() {
    let mut m = books("alias-same");
    let (_, t) = m.register_ok(json!({}));
    let r = m.ok(
        "books.set_asset_tax_basis",
        json!({"asset": t, "rule": "carryover_723", "contributor_adjusted_basis": "900",
               "contributor_basis": "900.00", "fmv_at_contribution": "1200.00"}),
    );
    assert_eq!(r["basis"]["contributor_adjusted_basis"], "900.00", "{r}");
    m.register_ok(
        json!({"basis_rule": "carryover_723", "tax_basis_rule": "carryover",
        "contributor_adjusted_basis": "400.00", "contributor_basis": "400",
        "fmv_at_contribution": "1200.00"}),
    );
    m.register_ok(json!({"basis_rule": "purchase", "tax_basis_rule": "cost"}));
    assert_eq!(m.assets().len(), 3);
    let why = m.refuse(
        "books.set_asset_tax_basis",
        json!({"asset": t, "rule": "carryover_723", "contributor_adjusted_basis": "900.00",
               "contributor_basis": "900.01", "fmv_at_contribution": "1200.00"}),
    );
    assert!(why.contains("disagree"), "{why}");
}

/// A contribution's fair market value is a fact the human gives, never
/// assumed from the cost (review R11): without it the call is refused.
#[test]
fn fmv_is_required_for_a_contribution() {
    let mut m = books("tax-fmv");
    let (_, t) = m.register_ok(json!({}));
    for rule in ["carryover_723", "personal_use_conversion"] {
        let why = m.refuse(
            "books.set_asset_tax_basis",
            json!({"asset": t, "rule": rule, "contributor_adjusted_basis": "900.00"}),
        );
        assert!(why.contains("fmv_at_contribution"), "{why}");
        let out = m.register(json!({"basis_rule": rule, "contributor_adjusted_basis": "1.00"}));
        assert!(out.refused(), "{out:?}");
    }
    assert_eq!(m.assets().len(), 1, "nothing registered");
    assert_eq!(basis_lines(&m), 0);
}

/// Refusals: unknown asset, bad rule or method, a purchase with
/// contribution facts, a contribution without its basis, negative, finer
/// than cents, not a decimal string. Each leaves the journal byte-identical
/// (the harness checks) and the record.
#[test]
fn set_asset_tax_basis_refusals_change_nothing() {
    let mut m = books("tax-refuse");
    let (a, t) = m.register_ok(json!({}));
    m.ok(
        "books.set_asset_tax_basis",
        json!({"asset": t, "rule": "carryover_723", "contributor_adjusted_basis": "500.00",
               "fmv_at_contribution": "1200.00"}),
    );
    let before = m.asset(a)["basis"].clone();
    let c = |extra: Value| {
        let mut v = json!({"asset": t, "rule": "carryover_723",
                           "contributor_adjusted_basis": "500.00", "fmv_at_contribution": "1200.00"});
        for (k, x) in extra.as_object().unwrap() {
            v[k] = x.clone();
        }
        v
    };
    for args in [
        json!({"asset": "NOPE-1", "rule": "purchase"}),
        json!({"asset": "999", "rule": "purchase"}),
        json!({"asset": t, "rule": "fmv"}),
        json!({"asset": t, "rule": "Purchase"}),
        json!({"asset": t, "rule": ""}),
        json!({"asset": t, "rule": "purchase", "contributor_adjusted_basis": "100.00"}),
        json!({"asset": t, "rule": "purchase", "fmv_at_contribution": "100.00"}),
        json!({"asset": t, "rule": "purchase", "sec704c_method": "traditional"}),
        json!({"asset": t, "rule": "carryover_723", "fmv_at_contribution": "1.00"}),
        c(json!({"contributor_adjusted_basis": "-1.00"})),
        c(json!({"contributor_adjusted_basis": "1.001"})),
        c(json!({"fmv_at_contribution": "lots"})),
        c(json!({"fmv_at_contribution": "1,000.00"})),
        c(json!({"sec704c_method": "ceiling"})),
        c(json!({"basis_source": {"decided_by": " "}})),
    ] {
        m.refuse("books.set_asset_tax_basis", args);
    }
    assert_eq!(m.asset(a)["basis"], before);
    assert_eq!(basis_lines(&m), 1);
}

/// register_asset with basis_rule and its facts records it at
/// registration; without, list_assets shows basis null.
#[test]
fn register_asset_records_a_basis() {
    let mut m = books("tax-register");
    let (a, _) = m.register_ok(json!({"basis_rule": "purchase"}));
    let (b, _) = m.register_ok(json!({"basis_rule": "personal_use_conversion",
        "contributor_adjusted_basis": "1500.00", "fmv_at_contribution": "1200.00"}));
    let (c, _) = m.register_ok(json!({"basis_rule": "carryover_723",
        "contributor_adjusted_basis": "400.00", "fmv_at_contribution": "1200.00"}));
    let (n, _) = m.register_ok(json!({}));
    let ta = m.asset(a)["basis"].clone();
    assert_eq!(ta["rule"], "purchase");
    assert_eq!(dec(&ta, "depreciable_basis").as_deref(), Some("1200"));
    let tb = m.asset(b)["basis"].clone();
    assert_eq!(dec(&tb, "tax_basis_723").as_deref(), Some("1500"));
    assert_eq!(dec(&tb, "depreciable_basis").as_deref(), Some("1200"));
    assert_eq!(dec(&tb, "built_in_gain_704c").as_deref(), Some("-300"));
    let tc = m.asset(c)["basis"].clone();
    assert_eq!(dec(&tc, "depreciable_basis").as_deref(), Some("400"));
    assert_eq!(dec(&tc, "built_in_gain_704c").as_deref(), Some("800"));
    assert_eq!(m.asset(n)["basis"], Value::Null);
}

/// A basis fact without basis_rule, a bad rule, or a record the engine
/// refuses, given at registration, is a refused call: it registers
/// nothing (server invariant 2).
#[test]
fn register_asset_refuses_a_bad_basis_and_registers_nothing() {
    let mut m = books("tax-register-bad");
    m.register_ok(json!({}));
    for extra in [
        json!({"contributor_adjusted_basis": "500.00"}),
        json!({"fmv_at_contribution": "500.00"}),
        json!({"basis_rule": "market_value"}),
        json!({"basis_rule": "carryover_723", "contributor_adjusted_basis": "five",
               "fmv_at_contribution": "1.00"}),
        json!({"basis_rule": "carryover_723", "fmv_at_contribution": "1.00"}),
        json!({"basis_rule": "purchase", "contributor_adjusted_basis": "100.00"}),
        json!({"basis_rule": "carryover_723", "contributor_adjusted_basis": "-1.00",
               "fmv_at_contribution": "1.00"}),
        json!({"basis_rule": "purchase", "tax_basis_rule": "carryover"}),
    ] {
        let out = m.register(extra.clone());
        assert!(out.refused(), "{extra}: {out:?}");
    }
    assert_eq!(m.assets().len(), 1, "nothing registered");
}

// ------------------------------------------- the dates (rules.md B-15)

/// purchase_date is an alias for in_service_date for one release, noted.
#[test]
fn purchase_date_is_an_alias_for_in_service_date() {
    let mut m = books("alias-date");
    let r = match m.register(json!({"in_service_date": null, "purchase_date": "2026-02-01"})) {
        Out::Ok(v) => v,
        other => panic!("{other:?}"),
    };
    assert_eq!(r["in_service_date"], "2026-02-01");
    assert_eq!(r["acquired_date"], "2026-02-01");
    assert!(
        r["notes"]
            .to_string()
            .contains("purchase_date is now in_service_date"),
        "{r}"
    );
    let a = m.asset(r["asset_id"].as_u64().unwrap());
    assert_eq!(a["in_service_date"], "2026-02-01");
    assert!(a.get("purchase_date").is_none(), "{a}");
}

/// Both given with different dates: refused, nothing registered; neither:
/// refused.
#[test]
fn purchase_date_and_a_different_in_service_date_are_refused() {
    let mut m = books("alias-differ");
    let out = m.register(json!({"in_service_date": "2026-01-01", "purchase_date": "2026-02-01"}));
    assert!(out.refused(), "{out:?}");
    let out = m.register(json!({"in_service_date": null}));
    assert!(out.refused(), "{out:?}");
    assert!(m.assets().is_empty());
    m.register_ok(json!({"in_service_date": "2026-01-01", "purchase_date": "2026-01-01"}));
}

/// The acquired date is the acquisition entry's (an asset bought before it
/// is in service); an entry dated after the in-service date is refused,
/// registering nothing.
#[test]
fn acquired_date_comes_from_the_acquisition_entry() {
    let mut m = books("acquired");
    m.post(
        "2026-01-10",
        &[("1500", "debit", "1200.00"), ("1000", "credit", "1200.00")],
    );
    let entry = 1;
    let r = match m.register(json!({"in_service_date": "2026-02-01", "acquisition_entry": entry})) {
        Out::Ok(v) => v,
        other => panic!("{other:?}"),
    };
    assert_eq!(r["acquired_date"], "2026-01-10");
    assert_eq!(r["in_service_date"], "2026-02-01");
    let a = m.asset(r["asset_id"].as_u64().unwrap());
    assert_eq!(a["acquired_date"], "2026-01-10");
    let why = match m.register(json!({"in_service_date": "2026-01-05", "acquisition_entry": entry}))
    {
        Out::Ok(v) => panic!("registered {v}"),
        other => other.err_text(),
    };
    assert!(why.contains("after the in-service date"), "{why}");
    assert_eq!(m.assets().len(), 1);
    // correct_asset: never before the acquisition entry; in_service the
    // old name of in_service_date.
    let id = r["asset_id"].as_u64().unwrap();
    let why = m.refuse(
        "books.correct_asset",
        json!({"asset": id.to_string(), "in_service_date": "2026-01-09", "reason": "typo"}),
    );
    assert!(why.contains("before it was acquired"), "{why}");
    let c = m.ok(
        "books.correct_asset",
        json!({"asset": id.to_string(), "in_service": "2026-01-15", "reason": "typo"}),
    );
    assert_eq!(c["in_service_date"], "2026-01-15");
    assert_eq!(c["acquired_date"], "2026-01-10");
    assert!(
        c["notes"]
            .to_string()
            .contains("in_service is now in_service_date")
    );
}

// ------------------------------- posted and scheduled (rules.md B-15 rule 8)

/// Named regression 1 (the brief's reproduction 1, its figures made up): a
/// $1,440.00 laptop, 36 months, in service 2026-05-15, nothing run. As of
/// 2026-05-20 the schedule has 1,440/36 × 6/31 = 7.74 due, the books carry
/// nothing: posted 0.00, scheduled 7.74, unposted 7.74, book value
/// 1,440.00 (the tool called 1,432.26 book value before P3). May's run is
/// dated 05-31: as of 05-20 still 0.00 posted; as of 05-31 posted =
/// scheduled = 1,440/36 × 17/31 = 21.94.
#[test]
fn list_assets_book_value_is_what_the_books_carry() {
    let mut m = books("book-value");
    let (id, _) = m.register_ok(json!({"in_service_date": "2026-05-15", "cost": "1440.00",
                                       "useful_life_months": 36}));
    let at = |m: &mut Mcp, as_of: &str| -> Value {
        let r = m.read_only("books.list_assets", json!({"as_of": as_of}));
        assert_eq!(r["as_of"], as_of);
        r["assets"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["asset_id"] == json!(id))
            .unwrap()
            .clone()
    };
    let a = at(&mut m, "2026-05-20");
    assert_eq!(a["held"], true);
    assert_eq!(dec(&a, "accumulated_posted").as_deref(), Some("0"));
    assert_eq!(
        dec(&a, "accumulated_scheduled_through").as_deref(),
        Some("7.74")
    );
    assert_eq!(dec(&a, "unposted_depreciation").as_deref(), Some("7.74"));
    assert_eq!(dec(&a, "book_value").as_deref(), Some("1440"));
    assert!(a.get("accumulated").is_none(), "{a}");
    m.ok(
        "books.run_depreciation",
        json!({"period": "2026-05", "asset_id": id}),
    );
    let a = at(&mut m, "2026-05-20");
    assert_eq!(dec(&a, "accumulated_posted").as_deref(), Some("0"));
    let a = at(&mut m, "2026-05-31");
    assert_eq!(dec(&a, "accumulated_posted").as_deref(), Some("21.94"));
    assert_eq!(
        dec(&a, "accumulated_scheduled_through").as_deref(),
        Some("21.94")
    );
    assert_eq!(dec(&a, "unposted_depreciation").as_deref(), Some("0"));
    assert_eq!(dec(&a, "book_value").as_deref(), Some("1418.06"));
    // Before it was acquired: not held, no figures.
    let a = at(&mut m, "2026-05-14");
    assert_eq!(a["held"], false);
    assert_eq!(a["book_value"], Value::Null);
    assert_eq!(a["accumulated_posted"], Value::Null);
}

/// A run reversed is no longer posted (named regression 2, its posted
/// figure): the books' accumulated depreciation and book value go back,
/// the scheduled figure stays, and the difference is due again.
#[test]
fn a_reversed_run_is_not_posted() {
    let mut m = books("reversed-run");
    let (id, _) = m.register_ok(json!({}));
    let run = m.ok(
        "books.run_depreciation",
        json!({"period": "2026-01", "asset_id": id}),
    );
    let entry = run["runs"][0]["entry_id"].as_u64().unwrap();
    let at = |m: &mut Mcp| -> Value {
        m.read_only("books.list_assets", json!({"as_of": "2026-01-31"}))["assets"][0].clone()
    };
    assert_eq!(
        dec(&at(&mut m), "accumulated_posted").as_deref(),
        Some("100")
    );
    m.ok(
        "books.reverse_entry",
        json!({"entry_id": entry, "date": "2026-01-31", "description": "undo", "confirmed": true}),
    );
    let a = at(&mut m);
    assert_eq!(dec(&a, "accumulated_posted").as_deref(), Some("0"));
    assert_eq!(
        dec(&a, "accumulated_scheduled_through").as_deref(),
        Some("100")
    );
    assert_eq!(dec(&a, "unposted_depreciation").as_deref(), Some("100"));
    assert_eq!(dec(&a, "book_value").as_deref(), Some("1200"));
}

/// A disposed asset is not held: every figure 0.00 once it left the books.
#[test]
fn a_disposed_asset_has_left_the_books() {
    let mut m = books("disposed-list");
    m.post(
        "2026-01-01",
        &[("1000", "debit", "9000.00"), ("3000", "credit", "9000.00")],
    );
    let (id, sold) = m.register_ok(json!({}));
    let doc = m.file_bill_of_sale();
    m.ok(
        "books.dispose_asset",
        json!({"tag": sold, "date": "2026-03-31", "proceeds": "100.00",
               "proceeds_account": "1000", "gain_loss_account": "7100",
               "confirmed": true, "receipt_attachment_ids": [doc]}),
    );
    let a = m.asset(id);
    assert_eq!(a["status"], "disposed");
    assert_eq!(a["held"], false);
    for k in [
        "accumulated_posted",
        "accumulated_scheduled_through",
        "unposted_depreciation",
        "book_value",
    ] {
        assert_eq!(dec(&a, k).as_deref(), Some("0"), "{k}");
    }
}

/// The fixed_asset_register export carries, per asset, its acquired and
/// in-service dates, accumulated depreciation scheduled and posted, book
/// value (cost less posted), and the basis facts and figures; JSON the
/// totals and the three differences. A disposed asset is out of the
/// totals. By hand, as of 2026-06-30: held with a basis, depreciable
/// BOUGHT-1 1,200.00, CARRY-1 650.25 and CONV-1 1,200.00, together
/// 3,050.25; §723 650.25 and 2,000.00, together 2,650.25; BOUGHT-1 ran
/// January: posted 100.00 of 600.00 scheduled.
#[test]
fn export_fixed_asset_register_carries_posted_scheduled_and_704c() {
    let mut m = books("tax-export");
    m.post(
        "2026-01-01",
        &[("1000", "debit", "9000.00"), ("3000", "credit", "9000.00")],
    );
    let (bought, _) = m.register_ok(json!({"tag": "BOUGHT-1", "basis_rule": "purchase"}));
    m.register_ok(json!({"tag": "CARRY-1", "basis_rule": "carryover_723",
                         "contributor_adjusted_basis": "650.25", "fmv_at_contribution": "1200.00",
                         "sec704c_method": "traditional"}));
    m.register_ok(
        json!({"tag": "CONV-1", "basis_rule": "personal_use_conversion",
                         "contributor_adjusted_basis": "2000.00", "fmv_at_contribution": "1200.00",
                         "basis_source": {"decided_by": "Tobin Harlow", "decided_on": "2026-10-20",
                                          "citation": "entry 9"}}),
    );
    m.register_ok(json!({"tag": "NONE-1"}));
    let (_, sold) = m.register_ok(json!({"tag": "SOLD-1", "basis_rule": "purchase"}));
    m.ok(
        "books.run_depreciation",
        json!({"period": "2026-01", "asset_id": bought}),
    );
    let doc = m.file_bill_of_sale();
    m.ok(
        "books.dispose_asset",
        json!({"tag": sold, "date": "2026-03-31", "proceeds": "100.00",
               "proceeds_account": "1000", "gain_loss_account": "7100",
               "confirmed": true, "receipt_attachment_ids": [doc]}),
    );
    let export = |m: &mut Mcp, format: &str| -> String {
        let r = m.read_only(
            "books.export",
            json!({"report": "fixed_asset_register", "format": format, "as_of": "2026-06-30"}),
        );
        m.wire.fetch_text(&r["file"])
    };
    let j: Value = serde_json::from_str(&export(&mut m, "json")).unwrap();
    let rows = j["assets"].as_array().unwrap();
    assert_eq!(rows.len(), 5, "{j}");
    let row = |tag: &str| rows.iter().find(|r| r["tag"] == tag).unwrap().clone();
    let r = row("BOUGHT-1");
    assert_eq!(r["basis_rule"], "purchase");
    assert_eq!(dec(&r, "accumulated_scheduled").as_deref(), Some("600"));
    assert_eq!(dec(&r, "accumulated_posted").as_deref(), Some("100"));
    assert_eq!(dec(&r, "book_value").as_deref(), Some("1100"));
    assert_eq!(dec(&r, "depreciable_basis").as_deref(), Some("1200"));
    assert_eq!(r["acquired_date"], "2026-01-01");
    assert_eq!(r["in_service_date"], "2026-01-01");
    let r = row("CARRY-1");
    assert_eq!(dec(&r, "tax_basis_723").as_deref(), Some("650.25"));
    assert_eq!(dec(&r, "built_in_gain_704c").as_deref(), Some("549.75"));
    assert_eq!(r["sec704c_method"], "traditional");
    assert_eq!(r["fmv_recorded"], true);
    let r = row("CONV-1");
    assert_eq!(dec(&r, "depreciable_basis").as_deref(), Some("1200"));
    assert_eq!(dec(&r, "built_in_gain_704c").as_deref(), Some("-800"));
    assert_eq!(r["sec704c_1c"], true);
    assert_eq!(r["basis_source"], "Tobin Harlow, 2026-10-20: entry 9");
    assert_eq!(row("NONE-1")["basis_rule"], Value::Null);
    assert_eq!(the_value(&j, "total_depreciable_basis"), "3050.25");
    assert_eq!(the_value(&j, "total_tax_basis_723"), "2650.25");
    assert_eq!(the_value(&j, "total_accumulated_posted"), "100");
    assert_eq!(the_value(&j, "unattributed_accumulated"), "0");
    assert_eq!(j["missing_basis"], json!(1));
    assert!(
        j["tax_figures_are"]
            .as_str()
            .unwrap()
            .contains("at contribution")
    );

    // CSV: by header.
    let csv = export(&mut m, "csv");
    let mut lines = csv.lines();
    let header: Vec<String> = lines
        .next()
        .unwrap()
        .split(',')
        .map(str::to_string)
        .collect();
    assert_eq!(
        header,
        [
            "tag",
            "description",
            "serial",
            "acquired",
            "in_service",
            "cost",
            "salvage",
            "useful_life_months",
            "accumulated_scheduled",
            "accumulated_posted",
            "book_value",
            "status",
            "proceeds",
            "basis_rule",
            "contributor_adjusted_basis",
            "fmv_at_contribution",
            "fmv_recorded",
            "tax_basis_723",
            "depreciable_basis",
            "built_in_gain_704c",
            "sec704c_1c",
            "sec704c_method",
            "basis_source"
        ]
    );
    let col = |name: &str| header.iter().position(|h| h == name).unwrap();
    let body: Vec<Vec<String>> = lines
        .map(|l| l.split(',').map(str::to_string).collect())
        .collect();
    let csv_row = |tag: &str| {
        body.iter()
            .find(|r| r.first().map(String::as_str) == Some(tag))
            .unwrap_or_else(|| panic!("{tag} in {csv}"))
            .clone()
    };
    let r = csv_row("CARRY-1");
    assert_eq!(r[col("basis_rule")], "carryover_723");
    assert_eq!(canon(&r[col("tax_basis_723")]), "650.25");
    assert_eq!(canon(&r[col("built_in_gain_704c")]), "549.75");
    assert_eq!(r[col("fmv_recorded")], "true");
    assert_eq!(r[col("sec704c_1c")], "false", "a built-in gain");
    assert_eq!(
        csv_row("CONV-1")[col("sec704c_1c")],
        "true",
        "a built-in loss"
    );
    let r = csv_row("BOUGHT-1");
    assert_eq!(r[col("sec704c_1c")], "", "a purchase");
    assert_eq!(canon(&r[col("accumulated_posted")]), "100");
    assert_eq!(canon(&r[col("book_value")]), "1100");
    assert_eq!(r[col("contributor_adjusted_basis")], "");
    let total = csv_row("TOTAL");
    assert_eq!(total.len(), header.len());
    assert_eq!(canon(&total[col("depreciable_basis")]), "3050.25");
    assert_eq!(canon(&total[col("tax_basis_723")]), "2650.25");
    assert_eq!(canon(&total[col("accumulated_posted")]), "100");
}

// ------------------------------------- P3: defect 22 and runs, over MCP

/// Books holding asset X of the P3 depreciation brief (12,000.00, no
/// salvage, 12 months from 2026-01-01: 1,000.00 a month), paid for.
fn asset_x(name: &str) -> (Mcp, u64) {
    let mut m = books(name);
    m.post(
        "2026-01-01",
        &[
            ("1000", "debit", "20000.00"),
            ("3000", "credit", "20000.00"),
        ],
    );
    m.post(
        "2026-01-01",
        &[
            ("1500", "debit", "12000.00"),
            ("1000", "credit", "12000.00"),
        ],
    );
    let (x, _) = m.register_ok(json!({"cost": "12000.00"}));
    (m, x)
}

fn dispose_args(x: u64, date: &str, proceeds: &str) -> Value {
    json!({"asset_id": x, "date": date, "proceeds": proceeds,
           "proceeds_account": "1000", "gain_loss_account": "7100"})
}

/// Defect 22, reproduction B over MCP: January through May run, then a
/// disposal on 2026-06-15 for 7,000.00. Hand check: June held 15 of 30
/// days, 500.00 (the final depreciation); accumulated 5,500.00, book value
/// 6,500.00, gain 500.00. Afterwards books.asset_schedule ends there, and
/// books.list_assets lists it as gone: not held, every figure 0.00 (it
/// left the books with its disposal; rules.md B-15 rule 8). Before P3 the
/// schedule showed June 7,000.00, accumulated 12,000.00, book value 0.00.
#[test]
fn a_disposed_asset_lists_what_it_depreciated_through_its_disposal() {
    let (mut m, x) = asset_x("disposed-figures");
    for p in ["2026-01", "2026-02", "2026-03", "2026-04", "2026-05"] {
        m.ok(
            "books.run_depreciation",
            json!({"period": p, "asset_id": x}),
        );
    }
    let preview = m.ok(
        "books.dispose_asset",
        dispose_args(x, "2026-06-15", "7000.00"),
    );
    assert_eq!(the_value(&preview, "final_depreciation"), "500");
    assert_eq!(the_value(&preview, "excess_depreciation"), "0");
    assert_eq!(the_value(&preview, "book_value"), "6500");
    assert_eq!(the_value(&preview, "gain_loss"), "500");
    let doc = m.file_bill_of_sale();
    let mut confirm = dispose_args(x, "2026-06-15", "7000.00");
    confirm["confirmed"] = json!(true);
    confirm["receipt_attachment_ids"] = json!([doc]);
    m.ok("books.dispose_asset", confirm);
    let s = m.read_only("books.asset_schedule", json!({"asset_id": x}));
    assert_eq!(s["status"], "disposed");
    let lines = s["schedule"].as_array().unwrap();
    let jun = lines.last().unwrap();
    assert_eq!(jun["period"], "2026-06");
    assert_eq!(canon(jun["amount"].as_str().unwrap()), "500");
    assert_eq!(canon(jun["accumulated"].as_str().unwrap()), "5500");
    assert_eq!(canon(jun["book_value"].as_str().unwrap()), "6500");
    let listed = m.asset(x);
    assert_eq!(listed["held"], false);
    for figure in [
        "accumulated_posted",
        "accumulated_scheduled_through",
        "unposted_depreciation",
        "book_value",
    ] {
        assert_eq!(canon(listed[figure].as_str().unwrap()), "0", "{figure}");
    }
    assert_eq!(listed["schedule_error"], Value::Null);
}

/// P3 depreciation review R2 over MCP: with June already run (dated Jun
/// 30), a disposal on Jun 15 for 7,000.00 credits back June's last 15
/// days. Hand check: the runs carry 6,000.00; the schedule through Jun 15
/// is 5,500.00, so 500.00 is excess; book value 6,500.00, gain 500.00. The
/// entry: Dr 1510 6,000.00, Dr 1000 7,000.00 / Cr 1500 12,000.00, Cr 6500
/// 500.00, Cr 7100 500.00.
#[test]
fn a_disposal_inside_a_posted_month_takes_back_the_extra_days() {
    let (mut m, x) = asset_x("disposed-inside");
    for p in 1..=6 {
        m.ok(
            "books.run_depreciation",
            json!({"period": format!("2026-{p:02}"), "asset_id": x}),
        );
    }
    let preview = m.ok(
        "books.dispose_asset",
        dispose_args(x, "2026-06-15", "7000.00"),
    );
    assert_eq!(the_value(&preview, "final_depreciation"), "0");
    assert_eq!(the_value(&preview, "excess_depreciation"), "500");
    assert_eq!(the_value(&preview, "catch_up_depreciation"), "0");
    assert_eq!(the_value(&preview, "accumulated"), "6000");
    assert_eq!(the_value(&preview, "book_value"), "6500");
    assert_eq!(the_value(&preview, "gain_loss"), "500");
    let line = |code: &str| {
        preview["lines"]
            .as_array()
            .unwrap()
            .iter()
            .find(|l| l["code"] == code)
            .unwrap_or_else(|| panic!("no {code} line in {preview}"))
            .clone()
    };
    assert_eq!(line("6500")["side"], "credit");
    assert_eq!(canon(line("6500")["amount"].as_str().unwrap()), "500");
    assert_eq!(canon(line("1510")["amount"].as_str().unwrap()), "6000");
}

/// P3 depreciation review R10 over MCP: a run-all for a month whose
/// depreciation rounds to 0.00 posts nothing for that asset and lists it
/// under skipped, with why; journaling nothing. Hand check: 0.01 over 3
/// months from 2026-01-01: January's cumulative a third of a cent, 0.00.
#[test]
fn run_depreciation_skips_a_month_that_rounds_to_nothing() {
    let mut m = books("zero-month");
    let (a, _) = m.register_ok(json!({"cost": "0.01", "useful_life_months": 3}));
    let before = m.journal();
    let r = m.ok("books.run_depreciation", json!({"period": "2026-01"}));
    assert_eq!(r["runs"], json!([]));
    let skipped = r["skipped"].as_array().unwrap();
    assert_eq!(skipped.len(), 1);
    assert_eq!(skipped[0]["asset_id"], json!(a));
    assert!(
        skipped[0]["reason"]
            .as_str()
            .unwrap()
            .contains("rounds to nothing"),
        "{r}"
    );
    assert_eq!(m.journal(), before, "nothing journaled");
    let r = m.ok("books.run_depreciation", json!({"period": "2026-02"}));
    assert_eq!(canon(r["runs"][0]["amount"].as_str().unwrap()), "0.01");
}

/// The catch-up rule over MCP (plan/served.md P3, settled in design):
/// January run, February skipped, a March run posts 2,000.00, of which
/// 1,000.00 is caught up; a run reversed within its month is due again
/// (reversed after it, the next run catches it up: the store's
/// a_run_reversed_after_its_month_is_caught_up_in_the_reversals_month), and
/// a disposal entry can't be reversed.
#[test]
fn a_skipped_month_is_caught_up_and_said() {
    let (mut m, x) = asset_x("caught-up");
    m.ok(
        "books.run_depreciation",
        json!({"period": "2026-01", "asset_id": x}),
    );
    let r = m.ok(
        "books.run_depreciation",
        json!({"period": "2026-03", "asset_id": x}),
    );
    assert_eq!(canon(r["runs"][0]["amount"].as_str().unwrap()), "2000");
    assert_eq!(canon(r["runs"][0]["caught_up"].as_str().unwrap()), "1000");
    let jan_entry = {
        let again = m.ok(
            "books.run_depreciation",
            json!({"period": "2026-01", "asset_id": x}),
        );
        assert_eq!(again["runs"][0]["already_posted"], json!(true));
        again["runs"][0]["entry_id"].as_u64().unwrap()
    };
    m.ok(
        "books.reverse_entry",
        json!({"entry_id": jan_entry, "date": "2026-01-31", "description": "undo",
               "confirmed": true}),
    );
    let r = m.ok(
        "books.run_depreciation",
        json!({"period": "2026-01", "asset_id": x}),
    );
    assert_eq!(r["runs"][0]["already_posted"], json!(false));
    assert_eq!(canon(r["runs"][0]["amount"].as_str().unwrap()), "1000");
    let doc = m.file_bill_of_sale();
    let mut confirm = dispose_args(x, "2026-03-31", "9000.00");
    confirm["confirmed"] = json!(true);
    confirm["receipt_attachment_ids"] = json!([doc]);
    let disposed = m.ok("books.dispose_asset", confirm);
    let entry = disposed["entry_id"].as_u64().unwrap();
    let err = m.refuse(
        "books.reverse_entry",
        json!({"entry_id": entry, "date": "2026-04-02", "description": "undo"}),
    );
    assert!(err.contains("can't be reversed"), "{err}");
}

/// A basis recorded before P3 has no value at contribution and reads it as
/// the book cost, so correcting the cost moves its built-in gain: the
/// result says so (P3 review, 2026-10-21: the tool text said the built-in
/// gain was "as contributed" whatever the record). By hand: cost 1,200.00,
/// basis 400.00, gain 800.00; cost corrected to 1,100.00, gain 700.00.
#[test]
fn correcting_a_legacy_contributions_cost_says_its_built_in_gain_moved() {
    let mut m = books("legacy-correct");
    let (id, t) = m.register_ok(json!({}));
    let dir = m.books_dir.take().unwrap();
    let data = m.data.clone();
    drop(m);
    dir.stop();
    let mut w = ledger_store::JournalWriter::open(&data.join("journal.jsonl")).unwrap();
    w.append(
        "asset.tax_basis_set",
        json!({"asset_id": id, "rule": "carryover", "contributor_basis": "400.00"}),
    )
    .unwrap();
    drop(w);
    let toml = dir.join("wanderbooks.toml");
    let mut m = Mcp::start(&dir, &toml);
    m.books_dir = Some(dir);
    assert_eq!(m.asset(id)["basis"]["built_in_gain_704c"], "800.00");
    let r = m.ok(
        "books.correct_asset",
        json!({"asset": t, "cost": "1100.00", "reason": "invoice misread"}),
    );
    assert_eq!(r["basis"]["built_in_gain_704c"], "700.00", "{r}");
    let notes = r["notes"].to_string();
    assert!(
        notes.contains("moved with the cost (800.00 to 700.00)"),
        "{notes}"
    );
    // A recorded value never moves, and says nothing.
    m.ok(
        "books.set_asset_tax_basis",
        json!({"asset": t, "rule": "carryover_723", "contributor_adjusted_basis": "400.00",
               "fmv_at_contribution": "1100.00"}),
    );
    let r = m.ok(
        "books.correct_asset",
        json!({"asset": t, "cost": "1000.00", "reason": "invoice misread again"}),
    );
    assert_eq!(r["basis"]["built_in_gain_704c"], "700.00", "{r}");
    assert!(
        r.get("notes")
            .is_none_or(|n| n.as_array().is_some_and(|a| a.is_empty())),
        "{r}"
    );
}
