//! Hostile input never stops the server (SharedStore, server.rs: a request
//! that panics stops the books server for every session, so a panic an
//! input can reach is a denial of service).
//!
//! ```text
//! GOAL: no argument, resource URI or combination of them reaches a panic,
//!       an overflow, or an internal error: every call is answered, with a
//!       result or a refusal, and the server keeps serving.
//! SOURCE: DECISION (P0 review round 3, 2026-10-16, finding S1): fail
//!         closed makes every reachable panic a denial of service, so none
//!         may be reachable.
//! INVARIANTS (oracle):
//!   1. Every tool in tools/list, called with arguments generated from its
//!      input schema and then made hostile (wrong types, missing or extra
//!      fields, empty, huge, negative and fractional numbers, u64::MAX,
//!      decimal strings with 30+ digits or absurd exponents, impossible and
//!      far-off dates and periods, control and format characters, long
//!      strings), gets a JSON-RPC answer that is no internal error, and a
//!      tool result that never says the server stopped.
//!   2. After every call the server still serves (books.chart answers).
//!   3. The same for resources/read with generated URIs.
//!   4. After them, the books reopen: a restarted server answers
//!      books.chart and books.verify (P0 review round 4: a call a command
//!      accepts but replay refuses would brick the books).
//!   5. Books an earlier version wrote (ledger-store's old-books fixtures)
//!      open, and every tool and resource answers over them.
//! ```
//!
//! Against a small fixture company with every kind of record, so ids and
//! names the generator picks often exist. `MCP_CASES` sets the number of
//! cases (default 256).

use proptest::prelude::*;
use proptest::test_runner::{Config, TestCaseError, TestRunner};
use serde_json::{Map, Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

/// The longest any one call may take: a call that hangs is a denial of
/// service too.
const TIMEOUT: Duration = Duration::from_secs(120);

/// What every request gets once the server stopped (server.rs STOPPED).
const STOPPED: &str = "stopped taking requests";

fn cases() -> u32 {
    std::env::var("MCP_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(256)
}

struct Mcp {
    stdin: ChildStdin,
    rx: Receiver<String>,
    child: Child,
    next_id: u64,
}

impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The places one run uses: the books, the vault key, the import root, the
/// folder the server runs in (where a relative path lands).
struct Places {
    base: PathBuf,
    books: PathBuf,
    inbox: PathBuf,
    sandbox: PathBuf,
}

/// The places go when the test ends, pass or panic.
impl Drop for Places {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

fn places(tag: &str) -> Places {
    let base =
        std::env::temp_dir().join(format!("ledger-mcp-hostile-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let p = Places {
        books: base.join("books"),
        inbox: base.join("inbox"),
        sandbox: base.join("sandbox"),
        base,
    };
    for d in [
        &p.books,
        &p.inbox,
        &p.sandbox.join("out"),
        &p.base.join("home"),
        &p.base.join("config"),
    ] {
        std::fs::create_dir_all(d).unwrap();
    }
    std::fs::write(
        p.books.join("wanderbooks.toml"),
        format!(
            "[company]\nname = \"Hostile Co\"\nfiscal_year_start = \"07-01\"\ntax_form = \"1065\"\n\
             [[company.members]]\nname = \"Member A\"\nshare = \"50%\"\ncapital_account = \"3000\"\n\
             [[company.members]]\nname = \"Member B\"\nshare = \"50%\"\ncapital_account = \"3001\"\n\
             [books]\nreceipt_threshold = \"25.00\"\ncash_accounts = [\"1000\"]\nparties = \"suggest\"\n\
             [vault]\nkey_file = \"{}\"\n",
            p.base.join("config/vault.key").display()
        ),
    )
    .unwrap();
    std::fs::write(p.inbox.join("receipt.pdf"), b"%PDF-1.4 a receipt\n%%EOF\n").unwrap();
    std::fs::write(
        p.inbox.join("bank.csv"),
        "Date,Description,Amount\n2026-03-01,Coffee,-4.50\n2026-03-02,Deposit,100.00\n",
    )
    .unwrap();
    p
}

impl Mcp {
    fn start(p: &Places) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_ledger-mcp"))
            .arg("--data-dir")
            .arg(p.books.join("books-data"))
            .arg("--books-toml")
            .arg(p.books.join("wanderbooks.toml"))
            .arg("--import-root")
            .arg(&p.inbox)
            .current_dir(&p.sandbox)
            .env("HOME", p.base.join("home"))
            .env("XDG_CONFIG_HOME", p.base.join("config"))
            .env("XDG_RUNTIME_DIR", p.base.join("config"))
            .env_remove("XDG_DOWNLOAD_DIR")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::from(
                std::fs::File::create(p.base.join("stderr.log")).unwrap(),
            ))
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
        let mut m = Self {
            stdin,
            rx,
            child,
            next_id: 1,
        };
        let init = m.rpc(
            "initialize",
            json!({"protocolVersion": "2025-06-18", "capabilities": {},
                   "clientInfo": {"name": "hostile", "version": "0"}}),
        );
        assert!(init.get("result").is_some(), "{init}");
        writeln!(
            m.stdin,
            "{}",
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"})
        )
        .unwrap();
        m
    }

    /// One request and its answer (Err: none came within TIMEOUT).
    fn rpc(&mut self, method: &str, params: Value) -> Value {
        self.try_rpc(method, params)
            .unwrap_or_else(|e| panic!("{method}: {e}"))
    }

    fn try_rpc(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        let msg = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        writeln!(self.stdin, "{msg}").map_err(|e| format!("write: {e}"))?;
        self.stdin.flush().map_err(|e| format!("flush: {e}"))?;
        loop {
            let line = self
                .rx
                .recv_timeout(TIMEOUT)
                .map_err(|_| format!("no answer within {TIMEOUT:?} (or the server died)"))?;
            let v: Value =
                serde_json::from_str(&line).map_err(|e| format!("non-JSON {line:?}: {e}"))?;
            if v.get("id") == Some(&json!(id)) {
                return Ok(v);
            }
        }
    }

    /// A tool's structured result; panics on any refusal (fixture setup).
    fn ok(&mut self, tool: &str, args: Value) -> Value {
        let r = self.rpc("tools/call", json!({"name": tool, "arguments": args}));
        assert!(
            r.get("error").is_none() && r["result"]["isError"] != json!(true),
            "fixture {tool} {args}: {r}"
        );
        r["result"]["structuredContent"].clone()
    }
}

/// A small company with one of everything, so generated ids and names
/// often find a record.
fn fixture(m: &mut Mcp, p: &Places) {
    for (code, name, ty) in [
        ("1000", "Bank", "asset"),
        ("1200", "Receivables", "asset"),
        ("1300", "Prepaid", "asset"),
        ("1500", "Equipment", "asset"),
        ("1501", "Accumulated Depreciation", "asset"),
        ("2000", "Payables", "liability"),
        ("2050", "Card", "liability"),
        ("2400", "Deferred Revenue", "liability"),
        ("3000", "Capital A", "equity"),
        ("3001", "Capital B", "equity"),
        ("4000", "Revenue", "revenue"),
        ("4900", "Gain", "revenue"),
        ("6000", "Contract Labor", "expense"),
        ("6100", "Depreciation", "expense"),
        ("6300", "Insurance", "expense"),
        ("6999", "General", "expense"),
    ] {
        m.ok(
            "books.add_account",
            json!({"code": code, "name": name, "account_type": ty}),
        );
    }
    m.ok(
        "books.add_party",
        json!({"name": "Acme", "roles": ["contractor"]}),
    );
    m.ok("books.add_vendor", json!({"name": "Acme"}));
    m.ok("books.add_customer", json!({"name": "Client Co"}));
    m.ok(
        "books.add_party",
        json!({"name": "Dana", "roles": ["employee"]}),
    );
    m.ok(
        "books.add_party",
        json!({"name": "Member A", "roles": ["member"]}),
    );
    // An unlinked receipt (attachment 1).
    m.ok(
        "books.attach_receipt",
        json!({"file_name": "r.pdf", "mime_type": "application/pdf",
               "content_base64": "JVBERi0xLjQgYSByZWNlaXB0"}),
    );
    for (date, dr, cr, amount) in [
        ("2026-01-02", "1000", "3000", "5000.00"),
        ("2026-01-02", "1000", "3001", "5000.00"),
        ("2026-02-10", "6000", "1000", "1200.00"),
        ("2026-03-05", "1500", "2050", "2400.00"),
    ] {
        let id = m.ok(
            "books.draft_entry",
            json!({"date": date, "description": format!("{dr} from {cr}")}),
        )["entry_id"]
            .clone();
        for (side, account) in [("debit", dr), ("credit", cr)] {
            m.ok(
                "books.add_line",
                json!({"entry_id": id, "account": account, "side": side, "amount": amount}),
            );
        }
        let pdf = format!("%PDF-1.4 receipt for {date} {dr} {cr}");
        m.ok(
            "books.attach_receipt",
            json!({"entry_id": id, "file_name": "r.pdf", "mime_type": "application/pdf",
                   "content_base64": b64(pdf.as_bytes())}),
        );
        m.ok(
            "books.post_entry",
            json!({"entry_id": id, "confirmed": true}),
        );
    }
    // A draft left open.
    let draft = m.ok(
        "books.draft_entry",
        json!({"date": "2026-04-01", "description": "open draft"}),
    )["entry_id"]
        .clone();
    m.ok(
        "books.add_line",
        json!({"entry_id": draft, "account": "6999", "side": "debit", "amount": "10.00"}),
    );
    m.ok(
        "books.register_asset",
        json!({"description": "Laptop", "purchase_date": "2026-03-05", "cost": "2400.00",
               "useful_life_months": 24, "cost_account": "1500",
               "accum_depr_account": "1501", "expense_account": "6100"}),
    );
    m.ok(
        "books.create_schedule",
        json!({"kind": "prepaid_expense", "description": "Insurance", "total": "300.00",
               "start": "2026-01-01", "months": 3, "balance_account": "1300",
               "pl_account": "6300"}),
    );
    m.ok(
        "books.create_recurring",
        json!({"description": "Rent", "frequency": "monthly", "start": "2026-01-01",
               "lines": [{"account": "6999", "side": "debit", "amount": "50.00"},
                         {"account": "1000", "side": "credit", "amount": "50.00"}]}),
    );
    m.ok(
        "books.set_tin",
        json!({"party": "Member A", "tin": "241-40-4915", "tin_type": "ssn"}),
    );
    m.ok(
        "books.file_document",
        json!({"category": "statement", "file_name": "s.pdf", "mime_type": "application/pdf",
               "content_base64": "JVBERi0xLjQgYSBzdGF0ZW1lbnQ="}),
    );
    let _ = p;
}

// ------------------------------------------------------------- generators

/// Strings that break naive parsers: dates, periods, decimals, ids,
/// control and format characters, the long and the empty.
fn hostile_strings() -> Vec<String> {
    let mut v: Vec<String> = [
        "",
        " ",
        "0",
        "-0",
        "1",
        "-1",
        "2",
        "3",
        "18446744073709551615",
        "18446744073709551616",
        "-9223372036854775808",
        "0.0000000000000000000000000001",
        "79228162514264337593543950335",
        "-79228162514264337593543950335",
        "79228162514264337593543950336",
        "999999999999999.9999999999",
        "1000000000000000",
        "1000000000000001",
        "123456789012345678901234567890.12",
        "1e28",
        "1E-30",
        "1e999999999",
        "NaN",
        "inf",
        "1.5",
        "-12.50",
        "12.345678901234567",
        "0001-01-01",
        "0000-00-00",
        "9999-12-31",
        "99999-12-31",
        "+262142-12-31",
        "-262143-01-01",
        "+262142-06-30",
        "2026-02-30",
        "2026-02-29",
        "2028-02-29",
        "2026-13-01",
        "2026-00-10",
        " 2026-01-15",
        "2026-01-15 ",
        "2026-1-5",
        "2026-10-16",
        "2026-12-31",
        "2025-12-31",
        "2026-06-30",
        "999999-01",
        "262142-12",
        "262143-01",
        "0000-01",
        "0-1",
        "2026-12",
        "2026-02",
        "2026-13",
        "-2026-01",
        "2026-01-01T00:00:00Z",
        "1000",
        "1500",
        "6000",
        "3000",
        "4000",
        "9999",
        "Acme",
        "Dana",
        "Member A",
        "Client Co",
        "Nobody",
        "FL-0001",
        "1/1/2026",
        "\u{0}",
        "a\u{0}b",
        "\n\n\r\t",
        "\u{202e}evil\u{200b}",
        "\u{feff}1000",
        "１０００",
        "291-12-2846",
        "241404915",
        "../../../../etc/passwd",
        "books://chart",
        "%s%n%x",
        "{\"a\":1}",
        "null",
        "true",
        "debit",
        "credit",
        "asset",
        "expense",
        "monthly",
        "weekly",
        "yearly",
        "usd",
        "EUR",
        "XXX",
        "ssn",
        "ein",
        "nec",
        "misc",
        "cost",
        "carryover",
        "personal_use_conversion",
        "prepaid_expense",
        "deferred_revenue",
        "receipt",
        "document",
        "statement",
        "csv",
        "xlsx",
        "pdf",
        "json",
        "trial_balance",
        "balance_sheet",
        "profit_and_loss",
        "fixed_asset_register",
        "general_ledger",
        "month",
        "quarter",
        "year",
        "entry",
        "account",
        "company",
        "all",
        "contract",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    v.push("9".repeat(40));
    v.push(format!("0.{}", "1".repeat(40)));
    v.push("A".repeat(5000));
    v.push("é".repeat(3000));
    v.push("\u{7}".repeat(100));
    v.push("2026-01-01".repeat(50));
    v
}

/// Numbers that break naive arithmetic.
fn hostile_numbers() -> Vec<Value> {
    vec![
        json!(0),
        json!(1),
        json!(2),
        json!(3),
        json!(4),
        json!(5),
        json!(7),
        json!(12),
        json!(24),
        json!(100),
        json!(-1),
        json!(i64::MIN),
        json!(i64::MAX),
        json!(u64::MAX),
        json!(u32::MAX),
        json!(u32::MAX as u64 + 1),
        json!(i32::MAX),
        json!(9_007_199_254_740_993u64),
        json!(1.5),
        json!(-0.0),
        json!(1e300),
        json!(-1e300),
        json!(2026),
        json!(9999),
        json!(10000),
        json!(262142),
        json!(1_000_000),
    ]
}

/// Paths a tool may be given: inside the sandbox, the import root, the
/// books, relative ones, odd ones. Never anywhere a real file lives.
fn hostile_paths(p: &Places) -> Vec<String> {
    let s = |p: &Path| p.to_str().unwrap().to_string();
    vec![
        s(&p.sandbox.join("out")),
        s(&p.sandbox.join("out/new.zip")),
        s(&p.sandbox.join("out/new.csv")),
        s(&p.sandbox.join("out/new.pdf")),
        s(&p.sandbox.join("out/missing-dir/x.zip")),
        s(&p.inbox.join("receipt.pdf")),
        s(&p.inbox.join("bank.csv")),
        s(&p.inbox.join("missing.pdf")),
        s(&p.inbox.join("x.zip")),
        format!("{}/../inbox/receipt.pdf", s(&p.inbox)),
        format!("{}/./receipt.pdf", s(&p.inbox)),
        s(&p.books.join("wanderbooks.toml")),
        s(&p.books.join("books-data/journal.jsonl")),
        s(&p.books.join("out.zip")),
        s(&p.base.join("config/vault.key")),
        "relative.pdf".into(),
        "out/rel.zip".into(),
        String::new(),
        "\u{0}".into(),
        format!("{}/{}", s(&p.sandbox.join("out")), "n".repeat(300)),
    ]
}

/// What a JSON value of any type can be, hostile: the wrong-type filler.
fn any_value(p: &Places) -> BoxedStrategy<Value> {
    let strings = hostile_strings();
    let numbers = hostile_numbers();
    let paths = hostile_paths(p);
    prop_oneof![
        proptest::sample::select(strings).prop_map(Value::String),
        proptest::sample::select(numbers),
        any::<bool>().prop_map(Value::Bool),
        Just(Value::Null),
        Just(json!([])),
        Just(json!({})),
        Just(json!(["1000", 1, null])),
        proptest::sample::select(paths).prop_map(Value::String),
        "\\PC{0,24}".prop_map(Value::String),
    ]
    .boxed()
}

/// A field's value, from its schema node, often hostile.
fn value_for(
    name: &str,
    node: &Value,
    defs: &Map<String, Value>,
    p: &Places,
    depth: u32,
) -> BoxedStrategy<Value> {
    // Follow a reference; take any of anyOf's branches.
    if let Some(r) = node.get("$ref").and_then(Value::as_str) {
        let key = r.trim_start_matches("#/$defs/");
        let target = defs.get(key).cloned().unwrap_or(json!({}));
        return value_for(name, &target, defs, p, depth);
    }
    if let Some(branches) = node.get("anyOf").and_then(Value::as_array) {
        let parts: Vec<BoxedStrategy<Value>> = branches
            .iter()
            .map(|b| value_for(name, b, defs, p, depth))
            .collect();
        return proptest::strategy::Union::new(parts).boxed();
    }
    let types: Vec<String> = match node.get("type") {
        Some(Value::String(t)) => vec![t.clone()],
        Some(Value::Array(ts)) => ts
            .iter()
            .filter_map(|t| t.as_str().map(str::to_string))
            .collect(),
        _ => vec![],
    };
    let mut parts: Vec<BoxedStrategy<Value>> = Vec::new();
    for t in &types {
        let s = match t.as_str() {
            "string" => string_for(name, node, p),
            "integer" | "number" => proptest::sample::select(hostile_numbers()).boxed(),
            "boolean" => prop_oneof![3 => Just(json!(true)), 1 => Just(json!(false))].boxed(),
            "null" => Just(Value::Null).boxed(),
            "array" if depth < 3 => {
                let item = node.get("items").cloned().unwrap_or(json!({}));
                let inner = value_for(name, &item, defs, p, depth + 1);
                prop_oneof![
                    8 => proptest::collection::vec(inner.clone(), 0..4).prop_map(Value::Array),
                    1 => proptest::collection::vec(inner, 40..60).prop_map(Value::Array),
                ]
                .boxed()
            }
            "object" if depth < 3 => object_for(node, defs, p, depth + 1),
            _ => any_value(p),
        };
        parts.push(s);
    }
    if parts.is_empty() {
        return any_value(p);
    }
    let typed = proptest::strategy::Union::new(parts).boxed();
    // Mostly the right type; sometimes any type at all.
    prop_oneof![6 => typed, 1 => any_value(p)].boxed()
}

/// A string field's value: what its name suggests, and the hostile set.
fn string_for(name: &str, node: &Value, p: &Places) -> BoxedStrategy<Value> {
    let n = name.to_lowercase();
    let mut likely: Vec<String> = Vec::new();
    if let Some(e) = node.get("enum").and_then(Value::as_array) {
        likely.extend(e.iter().filter_map(|v| v.as_str().map(str::to_string)));
    }
    if n.contains("path") || n.starts_with("save") {
        return proptest::sample::select(hostile_paths(p))
            .prop_map(Value::String)
            .boxed();
    }
    if n.contains("base64") {
        likely.extend(
            [
                "JVBERi0xLjQ=",
                "",
                "!!!not base64",
                "AAAA",
                "NTcxLTkzLTI4NDY=",
                "RGF0ZSxBbW91bnQKMjAyNi0wMS0wMSwxCg==",
            ]
            .map(String::from),
        );
    }
    let pick = |pred: fn(&str) -> bool| -> Vec<String> {
        hostile_strings().into_iter().filter(|s| pred(s)).collect()
    };
    let date_like = |s: &str| {
        s.len() >= 8 && s.chars().filter(|c| *c == '-').count() >= 2
            || s.is_empty()
            || s.starts_with(' ')
    };
    let period_like = |s: &str| s.matches('-').count() == 1 || s.is_empty();
    let number_like = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E'))
            && s.matches('-').count() <= 1
    };
    let dates = [
        "date",
        "as_of",
        "start",
        "end",
        "through",
        "since",
        "received_on",
        "in_service",
    ];
    let amounts = [
        "amount",
        "total",
        "cost",
        "deposit",
        "proceeds",
        "rate",
        "percent",
        "threshold",
        "balance",
        "basis",
        "salvage",
        "gross",
        "withholding",
        "ss_",
        "medicare",
        "deductions",
    ];
    if n.contains("period") && !n.contains("period_") {
        likely.extend(pick(period_like));
    } else if dates.iter().any(|d| n.contains(d)) {
        likely.extend(pick(date_like));
    } else if amounts.iter().any(|a| n.contains(a)) {
        likely.extend(pick(number_like));
    }
    let strings = hostile_strings();
    let named = if likely.is_empty() {
        proptest::sample::select(strings.clone()).boxed()
    } else {
        proptest::sample::select(likely).boxed()
    };
    prop_oneof![
        3 => named.prop_map(Value::String),
        4 => proptest::sample::select(strings).prop_map(Value::String),
        1 => "\\PC{0,40}".prop_map(Value::String),
    ]
    .boxed()
}

/// An object's value: each property present or not (required ones mostly
/// present), and now and then a field no schema names.
fn object_for(
    node: &Value,
    defs: &Map<String, Value>,
    p: &Places,
    depth: u32,
) -> BoxedStrategy<Value> {
    let props = node
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let required: Vec<String> = node
        .get("required")
        .and_then(Value::as_array)
        .map(|r| {
            r.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let fields: Vec<BoxedStrategy<Option<(String, Value)>>> = props
        .iter()
        .map(|(k, v)| {
            let key = k.clone();
            let present = if required.contains(k) { 0.92 } else { 0.5 };
            proptest::option::weighted(present, value_for(k, v, defs, p, depth))
                .prop_map(move |o| o.map(|v| (key.clone(), v)))
                .boxed()
        })
        .collect();
    let extra = proptest::option::weighted(0.05, any_value(p));
    (fields, extra)
        .prop_map(|(fields, extra)| {
            let mut m = Map::new();
            for (k, v) in fields.into_iter().flatten() {
                m.insert(k, v);
            }
            if let Some(x) = extra {
                m.insert("unexpected_field".into(), x);
            }
            Value::Object(m)
        })
        .boxed()
}

/// A tool's arguments: an object from its schema, rarely not an object.
fn args_for(schema: &Value, p: &Places) -> BoxedStrategy<Value> {
    let defs = schema
        .get("$defs")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    prop_oneof![
        30 => object_for(schema, &defs, p, 0),
        1 => any_value(p),
    ]
    .boxed()
}

/// INVARIANT 1-2 for one answer: no internal error, no "stopped", and the
/// server still serves.
fn check_answer(m: &mut Mcp, what: &str, r: Result<Value, String>) -> Result<(), TestCaseError> {
    let r = r.map_err(|e| TestCaseError::fail(format!("{what}: {e}")))?;
    if let Some(e) = r.get("error") {
        prop_assert_ne!(
            e["code"].as_i64(),
            Some(-32603),
            "{} got an internal error: {}",
            what,
            e
        );
    }
    let text = r.to_string();
    prop_assert!(
        !text.contains(STOPPED),
        "{} stopped the server: {}",
        what,
        text
    );
    prop_assert!(!text.contains("panicked"), "{} panicked: {}", what, text);
    let chart = m
        .try_rpc(
            "tools/call",
            json!({"name": "books.chart", "arguments": {}}),
        )
        .map_err(|e| TestCaseError::fail(format!("after {what}: {e}")))?;
    prop_assert!(
        chart.get("error").is_none() && chart["result"]["isError"] != json!(true),
        "after {}, books.chart fails: {}",
        what,
        chart
    );
    Ok(())
}

/// INVARIANTS 1-2: every tool, hostile arguments, the server serves on.
#[test]
fn hostile_tool_arguments_never_stop_the_server() {
    let p = places("tools");
    let mut m = Mcp::start(&p);
    fixture(&mut m, &p);
    let list = m.rpc("tools/list", json!({}));
    let tools: Vec<(String, Value)> = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            (
                t["name"].as_str().unwrap().to_string(),
                t["inputSchema"].clone(),
            )
        })
        .collect();
    assert!(tools.len() > 90, "{} tools", tools.len());
    let per_tool: Vec<BoxedStrategy<(String, Value)>> = tools
        .iter()
        .map(|(name, schema)| {
            let name = name.clone();
            args_for(schema, &p)
                .prop_map(move |a| (name.clone(), a))
                .boxed()
        })
        .collect();
    let call = proptest::strategy::Union::new(per_tool);
    // No shrinking: the server is shared, so once one call stops it
    // every smaller case fails too; the case reported is the first.
    let mut runner = TestRunner::new(Config {
        cases: cases(),
        failure_persistence: None,
        max_shrink_iters: 0,
        ..Config::default()
    });
    let m = std::cell::RefCell::new(m);
    // How the calls were answered: (reached a tool, the tool succeeded),
    // so the property can't pass by never getting past the schema.
    let counts = std::cell::Cell::new((0u32, 0u32, 0u32));
    runner
        .run(&call, |(tool, args)| {
            let mut m = m.borrow_mut();
            let what = format!("{tool} {}", truncated(&args));
            let r = m.try_rpc("tools/call", json!({"name": tool, "arguments": args}));
            if let Ok(v) = &r {
                let (n, reached, ok) = counts.get();
                let got = v.get("error").is_none();
                let fine = got && v["result"]["isError"] != json!(true);
                counts.set((n + 1, reached + u32::from(got), ok + u32::from(fine)));
            }
            check_answer(&mut m, &what, r)
        })
        .unwrap();
    let (n, reached, ok) = counts.get();
    eprintln!("{n} calls: {reached} reached a tool, {ok} succeeded");
    assert!(
        reached * 2 >= n,
        "most calls must reach a tool: {reached} of {n}"
    );
    assert!(ok * 20 >= n, "some calls must succeed: {ok} of {n}");
    // Every tool is called at least once, whatever the cases drew.
    let all = tools.clone();
    let mut m = m.into_inner();
    let mut runner = TestRunner::deterministic();
    for (tool, schema) in &all {
        let args = args_for(schema, &p)
            .new_tree(&mut runner)
            .unwrap()
            .current();
        let what = format!("{tool} {}", truncated(&args));
        let r = m.try_rpc("tools/call", json!({"name": tool, "arguments": args}));
        check_answer(&mut m, &what, r).unwrap();
    }
    reopens(m, &p, "every tool");
    let _ = std::fs::remove_dir_all(&p.base);
}

/// INVARIANT 4: whatever the calls journaled, the books open again. A
/// hostile call a command accepts but replay refuses would leave books
/// that never reopen (rules.md B-28): the server is restarted on them and
/// must answer books.chart and books.verify.
fn reopens(m: Mcp, p: &Places, after: &str) {
    drop(m);
    let mut m = Mcp::start(p);
    for tool in ["books.chart", "books.verify"] {
        let r = m
            .try_rpc("tools/call", json!({"name": tool, "arguments": {}}))
            .unwrap_or_else(|e| panic!("after {after}, restarted: {tool}: {e}"));
        assert!(
            r.get("error").is_none() && r["result"]["isError"] != json!(true),
            "after {after}, the books don't reopen: {tool}: {r}"
        );
    }
}

/// The id of the party named `name`.
fn party_id(m: &mut Mcp, name: &str) -> Value {
    let parties = m.ok("books.parties", json!({}));
    parties["parties"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == name)
        .unwrap_or_else(|| panic!("no party {name}: {parties}"))["party_id"]
        .clone()
}

/// A pinned call: it must reach the tool (no JSON-RPC error at all, so a
/// mistyped argument can't make the check vacuous), then INVARIANTS 1-2.
fn pinned(m: &mut Mcp, tool: &str, args: Value) -> Value {
    let what = format!("{tool} {}", truncated(&args));
    let r = m
        .try_rpc("tools/call", json!({"name": tool, "arguments": args}))
        .unwrap_or_else(|e| panic!("{what}: {e}"));
    assert!(
        r.get("error").is_none(),
        "{what} never reached the tool: {r}"
    );
    check_answer(m, &what, Ok(r.clone())).unwrap();
    r["result"].clone()
}

/// The calls that once stopped the server, each pinned (P0 review round 3,
/// S1, and the audit after it): each is answered, and the server serves on.
#[test]
fn inputs_that_once_panicked_are_refused_or_answered() {
    let p = places("known");
    let mut m = Mcp::start(&p);
    fixture(&mut m, &p);
    const MAX: &str = "79228162514264337593543950335";
    let big = "900000000000000.00";
    let acme = party_id(&mut m, "Acme");
    let client = party_id(&mut m, "Client Co");
    let lines = |a: &str, b: &str| {
        json!([{"account": "6999", "side": "debit", "amount": a},
               {"account": "6999", "side": "debit", "amount": b},
               {"account": "1000", "side": "credit", "amount": "1"}])
    };
    let payroll = |fed: &str, ss: &str| {
        json!([{"employee": "Dana", "gross_pay": "1", "federal_withholding": fed,
                "state_withholding": fed, "ss_employee": "0", "medicare_employee": "0",
                "ss_employer": ss, "medicare_employer": "1", "other_deductions": "0"}])
    };
    let schedule = |start: &str, months: Value| {
        json!({"kind": "prepaid_expense", "description": "x", "total": "100.00",
               "start": start, "months": months, "balance_account": "1300",
               "pl_account": "6300"})
    };
    let asset = |date: &str, months: Value| {
        json!({"description": "x", "purchase_date": date, "cost": "100.00",
               "useful_life_months": months, "cost_account": "1500",
               "accum_depr_account": "1501", "expense_account": "6100"})
    };
    let calls: Vec<(&str, Value)> = vec![
        ("books.run_depreciation", json!({"period": "999999-01"})),
        (
            "books.run_depreciation",
            json!({"period": "999999-01", "asset_id": 1}),
        ),
        ("books.run_depreciation", json!({"period": "262142-12"})),
        ("books.run_depreciation", json!({"period": "9999-12"})),
        ("books.create_schedule", schedule("+262142-12-15", json!(1))),
        (
            "books.create_schedule",
            schedule("+262142-06-01", json!(12)),
        ),
        ("books.create_schedule", schedule("9999-12-15", json!(2))),
        (
            "books.create_schedule",
            schedule("2026-01-01", json!(u32::MAX)),
        ),
        ("books.list_schedules", json!({})),
        ("books.run_schedules", json!({"through": "9999-12-31"})),
        ("books.register_asset", asset("+262142-11-30", json!(1))),
        ("books.register_asset", asset("9999-11-30", json!(1))),
        ("books.register_asset", asset("2026-01-01", json!(u32::MAX))),
        ("books.asset_schedule", json!({"asset_id": 2})),
        ("books.asset_schedule", json!({"asset_id": 3})),
        ("books.list_assets", json!({})),
        (
            "books.register_asset",
            json!({"description": "y", "purchase_date": "2026-01-01", "cost": "100.00",
                   "useful_life_months": 12, "cost_account": "1500",
                   "accum_depr_account": "1501", "expense_account": "6100",
                   "tax_basis_rule": "carryover", "contributor_basis": MAX}),
        ),
        (
            "books.set_asset_tax_basis",
            json!({"asset": "1", "rule": "carryover", "contributor_basis": MAX}),
        ),
        (
            "books.export",
            json!({"report": "fixed_asset_register", "format": "xlsx"}),
        ),
        (
            "books.create_invoice",
            json!({"party_id": client, "date": "2026-05-01", "due_date": "2026-06-01",
                   "lines": [{"description": "a", "amount": MAX},
                             {"description": "b", "amount": "1"}]}),
        ),
        (
            "books.create_bill",
            json!({"party_id": acme, "date": "2026-05-01", "due_date": "2026-06-01",
                   "lines": [{"description": "a", "amount": MAX},
                             {"description": "b", "amount": "1"}]}),
        ),
        (
            "books.create_invoice",
            json!({"party_id": client, "date": "2026-05-01", "due_date": "+262142-12-31",
                   "lines": [{"description": "a", "amount": "1"}]}),
        ),
        (
            "books.record_payout",
            json!({"date": "2026-05-01", "description": "x", "deposit": "1",
                   "deposit_account": "1000",
                   "revenue": [{"account": "4000", "amount": MAX},
                               {"account": "4000", "amount": "1"}]}),
        ),
        (
            "books.record_payout",
            json!({"date": "2026-05-01", "description": "x", "deposit": format!("-{MAX}"),
                   "deposit_account": "1000",
                   "revenue": [{"account": "4000", "amount": big}],
                   "fees": [{"account": "6999", "amount": format!("-{MAX}")}]}),
        ),
        (
            "books.run_payroll",
            json!({"date": "2026-05-01", "cash_account": "1000", "lines": payroll(MAX, "1")}),
        ),
        (
            "books.run_payroll",
            json!({"date": "2026-05-01", "cash_account": "1000", "lines": payroll("0", MAX)}),
        ),
        (
            "books.create_recurring",
            json!({"description": "x", "frequency": "monthly", "start": "2026-01-01",
                   "lines": lines(MAX, "1")}),
        ),
        (
            "books.dispose_asset",
            json!({"asset_id": 1, "date": "2026-06-30", "proceeds": MAX,
                   "proceeds_account": "1000", "gain_loss_account": "4900"}),
        ),
        (
            "books.dispose_asset",
            json!({"asset_id": 1, "date": "+262142-12-31", "proceeds": "1",
                   "proceeds_account": "1000", "gain_loss_account": "4900"}),
        ),
        (
            "books.reconcile",
            json!({"account": "1000", "statement_date": "2026-06-30",
                   "statement_balance": format!("-{MAX}")}),
        ),
        (
            "books.record_statement",
            json!({"attachment_id": 2, "account": "1000", "period_start": "2026-06-01",
                   "period_end": "2026-06-30", "opening_balance": "0",
                   "closing_balance": format!("-{MAX}")}),
        ),
        (
            "books.reconcile",
            json!({"account": "1000", "statement_attachment": 2}),
        ),
        (
            "books.export",
            json!({"report": "balance_sheet", "format": "xlsx", "as_of": "+262142-06-30"}),
        ),
        (
            "books.export",
            json!({"report": "balance_sheet", "format": "pdf", "as_of": "-262143-01-01"}),
        ),
        (
            "books.export",
            json!({"report": "balance_sheet", "format": "pdf", "as_of": "9999-12-31"}),
        ),
        (
            "books.export_cpa_package",
            json!({"as_of": "+262142-06-30", "cash_accounts": ["1000"], "include_tins": false}),
        ),
        (
            "books.export_cpa_package",
            json!({"as_of": "9999-12-31", "cash_accounts": ["1000"], "include_tins": false}),
        ),
        (
            "books.profit_and_loss",
            json!({"start": "0001-01-01", "end": "9999-12-31"}),
        ),
        (
            "books.profit_and_loss",
            json!({"start": "-262143-01-01", "end": "+262142-12-31"}),
        ),
        (
            "books.show",
            json!({"report": "profit_and_loss", "start": "+10000-01-01", "end": "+10000-12-31"}),
        ),
        (
            "books.create_recurring",
            json!({"description": "x", "frequency": "weekly", "start": "-262143-01-01",
                   "lines": lines("1", "1")}),
        ),
        ("books.run_recurring", json!({"through": "9999-12-31"})),
        ("books.vendor_1099", json!({"year": 2026})),
        ("books.vendor_1099", json!({"year": 262142})),
        ("books.vendor_1099", json!({"year": i64::MIN})),
        ("books.digest", json!({"since": "+262142-12-31"})),
        (
            "books.create_period",
            json!({"name": "far", "start": "+262142-12-01", "end": "+262142-12-31"}),
        ),
        (
            "books.create_period",
            json!({"name": "far", "start": "9999-12-01", "end": "9999-12-31"}),
        ),
    ];
    for (tool, args) in calls {
        pinned(&mut m, tool, args);
    }
    // Two vendors' shares of one payment at the line limit (the 1099
    // worksheet multiplies a payment by a share), and members' capital at
    // it (the K-1 worksheet multiplies too).
    let id = m.ok(
        "books.draft_entry",
        json!({"date": "2026-05-02", "description": "huge but legal"}),
    )["entry_id"]
        .clone();
    for (account, side, amount) in [
        ("6000", "debit", big),
        ("1000", "credit", "450000000000000.00"),
        ("2050", "credit", "450000000000000.00"),
    ] {
        pinned(
            &mut m,
            "books.add_line",
            json!({"entry_id": id, "account": account, "side": side, "amount": amount}),
        );
    }
    pinned(
        &mut m,
        "books.set_entry_party",
        json!({"entry_id": id, "parties": [{"party": "Acme", "amount": "450000000000000.00"},
                                           {"party": "Dana", "amount": "450000000000000.00"}]}),
    );
    pinned(
        &mut m,
        "books.set_entry_vendor",
        json!({"entry_id": id, "vendor": "Acme"}),
    );
    pinned(
        &mut m,
        "books.attach_receipt",
        json!({"entry_id": id, "file_name": "h.pdf", "mime_type": "application/pdf",
               "content_base64": "JVBERi0xLjQgaHVnZQ=="}),
    );
    pinned(
        &mut m,
        "books.post_entry",
        json!({"entry_id": id, "confirmed": true}),
    );
    for (tool, args) in [
        ("books.vendor_1099", json!({"year": 2026})),
        (
            "books.export_cpa_package",
            json!({"as_of": "2026-12-31", "cash_accounts": ["1000"], "include_tins": false}),
        ),
        ("books.trial_balance", json!({})),
        ("books.cash_flow", json!({"cash_accounts": ["1000"]})),
    ] {
        pinned(&mut m, tool, args);
    }
    // A draft holding a line the books can't sum, read every way.
    let id = m.ok(
        "books.draft_entry",
        json!({"date": "2026-05-01", "description": "huge", "party": "Acme"}),
    )["entry_id"]
        .clone();
    for amount in [MAX, "1"] {
        pinned(
            &mut m,
            "books.add_line",
            json!({"entry_id": id, "account": "6999", "side": "debit", "amount": amount}),
        );
    }
    for (tool, args) in [
        ("books.get_entry", json!({"entry_id": id})),
        ("books.party", json!({"party": "Acme"})),
        (
            "books.set_entry_party",
            json!({"entry_id": id, "parties": [{"party": "Acme", "amount": MAX},
                                               {"party": "Dana", "amount": "1"}]}),
        ),
        ("books.post_entry", json!({"entry_id": id})),
        ("books.search_entries", json!({"text": "huge"})),
        ("books.digest", json!({})),
    ] {
        pinned(&mut m, tool, args);
    }
    // And after a restart, the books open and serve.
    drop(m);
    let mut m = Mcp::start(&p);
    for (tool, args) in [
        ("books.chart", json!({})),
        ("books.list_schedules", json!({})),
        ("books.list_assets", json!({})),
        ("books.run_schedules", json!({"through": "2026-12-31"})),
    ] {
        pinned(&mut m, tool, args);
    }
    let _ = std::fs::remove_dir_all(&p.base);
}

/// INVARIANT 3: resources/read with generated URIs.
#[test]
fn hostile_resource_uris_never_stop_the_server() {
    let p = places("resources");
    let mut m = Mcp::start(&p);
    fixture(&mut m, &p);
    let bases = [
        "books://chart",
        "books://reports/trial-balance",
        "books://reports/balance-sheet",
        "books://reports/profit-and-loss",
        "books://digest",
        "books://digest?since=",
        "books://digest?since=+262142-12-31",
        "books://digest?since=0000-00-00",
        "books://digest?since=9999-12-31",
        "books://digest?since=2026-02-30&since=1",
        "books://handbook",
        "books://handbook/",
        "books://handbook/contract",
        "books://handbook/all",
        "books://handbook/../contract",
        "books://receipts/",
        "books://receipts/1",
        "books://receipts/0",
        "books://receipts/18446744073709551615",
        "books://receipts/18446744073709551616",
        "books://receipts/-1",
        "books://nope",
        "",
        "books:",
        "file:///etc/passwd",
        "books://chart?x=1",
    ];
    let uri = prop_oneof![
        proptest::sample::select(bases.to_vec()).prop_map(str::to_string),
        (proptest::sample::select(bases.to_vec()), "\\PC{0,24}")
            .prop_map(|(b, s)| format!("{b}{s}")),
        "\\PC{0,64}",
        "books://[a-z/?=&.%-]{0,40}",
    ];
    // No shrinking: the server is shared, so once one call stops it
    // every smaller case fails too; the case reported is the first.
    let mut runner = TestRunner::new(Config {
        cases: cases(),
        failure_persistence: None,
        max_shrink_iters: 0,
        ..Config::default()
    });
    let m = std::cell::RefCell::new(m);
    runner
        .run(&uri, |uri| {
            let mut m = m.borrow_mut();
            let r = m.try_rpc("resources/read", json!({"uri": uri}));
            check_answer(&mut m, &format!("resources/read {uri:?}"), r)
        })
        .unwrap();
    reopens(m.into_inner(), &p, "the resources");
    let _ = std::fs::remove_dir_all(&p.base);
}

fn b64(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// The arguments, cut for a failure message.
fn truncated(v: &Value) -> String {
    let s = v.to_string();
    if s.chars().count() > 600 {
        format!("{}…", s.chars().take(600).collect::<String>())
    } else {
        s
    }
}

/// Every file of one fixture's books (its journal and blobs; never the
/// expected figures beside them) copied into `to`.
fn copy_books(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let name = e.file_name();
        if name == "expected.json" {
            continue;
        }
        if e.file_type().unwrap().is_dir() {
            copy_books(&e.path(), &to.join(&name));
        } else {
            std::fs::copy(e.path(), to.join(&name)).unwrap();
        }
    }
}

/// The books an earlier wanderbooks wrote (ledger-store's
/// tests/fixtures/old-books, made with main 188c90ea's Store API: dates
/// past 9999 and before 0001, lines and shares past the line limit,
/// sub-cent document and payroll lines, and the edges of chrono's calendar
/// and of Decimal), in the order the directory lists them.
fn old_books() -> Vec<PathBuf> {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../ledger-store/tests/fixtures/old-books");
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    assert!(dirs.len() >= 14, "{} old books", dirs.len());
    dirs
}

/// Calls that read old books the ways a human or an agent does: every
/// report over the widest range a command takes, every record by id.
fn reads_of_old_books() -> Vec<(&'static str, Value)> {
    let wide = json!({"start": "0001-01-01", "end": "9999-12-31"});
    let mut calls = vec![
        ("books.chart", json!({})),
        ("books.verify", json!({})),
        ("books.digest", json!({})),
        ("books.digest", json!({"since": "0001-01-01"})),
        ("books.settings", json!({})),
        ("books.roles", json!({})),
        ("books.trial_balance", json!({})),
        ("books.trial_balance", json!({"as_of": "9999-12-31"})),
        ("books.balance_sheet", json!({})),
        ("books.balance_sheet", json!({"as_of": "9999-12-31"})),
        ("books.balance_sheet", json!({"as_of": "0001-01-01"})),
        ("books.profit_and_loss", wide.clone()),
        ("books.profit_and_loss", json!({})),
        ("books.profit_and_loss_by_class", wide.clone()),
        ("books.cash_flow", wide.clone()),
        ("books.cash_flow", json!({"cash_accounts": ["1000"]})),
        ("books.ar_aging", json!({})),
        ("books.ar_aging", json!({"as_of": "9999-12-31"})),
        ("books.ap_aging", json!({"as_of": "0001-01-01"})),
        ("books.list_assets", json!({})),
        ("books.list_schedules", json!({})),
        ("books.list_recurring", json!({})),
        ("books.list_periods", json!({})),
        ("books.list_payroll_runs", json!({})),
        ("books.list_receipts", json!({})),
        ("books.list_vendors", json!({})),
        ("books.list_classes", json!({})),
        ("books.parties", json!({})),
        ("books.statements", json!({})),
        ("books.reconciliations", json!({})),
        ("books.attachments", json!({})),
        ("books.vendor_1099", json!({"year": 2026})),
        ("books.vendor_1099", json!({"year": 9999})),
        ("books.search_entries", json!({"text": "e"})),
        ("books.asset_labels", json!({})),
        ("books.run_schedules", json!({"through": "9999-12-31"})),
        ("books.run_recurring", json!({"through": "2026-12-31"})),
        ("books.run_depreciation", json!({"period": "2026-03"})),
        ("books.run_depreciation", json!({"period": "9999-12"})),
        (
            "books.reconcile",
            json!({"account": "1000", "statement_date": "2026-03-31",
                   "statement_balance": "10.00"}),
        ),
        (
            "books.reconcile",
            json!({"account": "1000", "statement_attachment": 1}),
        ),
        (
            "books.export_cpa_package",
            json!({"as_of": "9999-12-31", "cash_accounts": ["1000"], "include_tins": false}),
        ),
        (
            "books.export_cpa_package",
            json!({"as_of": "2026-12-31", "cash_accounts": ["1000"], "include_tins": false}),
        ),
    ];
    for id in 1..=3 {
        calls.push(("books.get_entry", json!({"entry_id": id})));
        calls.push(("books.asset_schedule", json!({"asset_id": id})));
        calls.push(("books.post_entry", json!({"entry_id": id})));
        calls.push(("books.party", json!({"party": id})));
        calls.push((
            "books.close_period",
            json!({"period_id": id, "confirmed": false}),
        ));
        calls.push((
            "books.dispose_asset",
            json!({"asset_id": id, "date": "9999-12-31", "proceeds": "0.00",
                   "proceeds_account": "1000", "gain_loss_account": "6000"}),
        ));
    }
    for report in [
        "trial_balance",
        "balance_sheet",
        "profit_and_loss",
        "cash_flow",
        "general_ledger",
        "fixed_asset_register",
    ] {
        for format in ["csv", "xlsx", "pdf", "json"] {
            calls.push(("books.export", json!({"report": report, "format": format})));
            calls.push((
                "books.export",
                json!({"report": report, "format": format, "as_of": "9999-12-31",
                       "start": "0001-01-01", "end": "9999-12-31"}),
            ));
        }
    }
    for (report, interval) in [
        ("profit_and_loss", "month"),
        ("profit_and_loss", "year"),
        ("balance_sheet", "quarter"),
        ("cash_flow", "year"),
    ] {
        calls.push((
            "books.show",
            json!({"report": report, "start": "0001-01-01", "end": "9999-12-31",
                   "interval": interval}),
        ));
    }
    calls
}

/// Books an earlier version wrote open, and every tool answers over them
/// (rules.md B-28: posted is forever; a figure the books can't compute is
/// refused, never a panic). Against each fixture: the reads above, then
/// OLD_BOOKS_CASES (default 3) generated calls of every tool; after them
/// the server restarts and must open the books again (a call a command
/// accepts but replay refuses would leave books that never reopen).
#[test]
fn old_books_answer_every_tool() {
    let per_tool: usize = std::env::var("OLD_BOOKS_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3);
    // Each book on its own server, several at once.
    let books = old_books();
    let workers = std::thread::available_parallelism().map_or(4, |n| n.get().min(8));
    let next = std::sync::atomic::AtomicUsize::new(0);
    let failures: Vec<String> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..workers)
            .map(|_| {
                scope.spawn(|| {
                    let mut failures = Vec::new();
                    loop {
                        let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(dir) = books.get(i) else { break };
                        failures.extend(old_book_answers_every_tool(dir, per_tool));
                    }
                    failures
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().expect("a worker panicked"))
            .collect()
    });
    assert!(
        failures.is_empty(),
        "{} failures:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// INVARIANT 5 for one old book: the failures, if any.
fn old_book_answers_every_tool(dir: &Path, per_tool: usize) -> Vec<String> {
    let mut failures: Vec<String> = Vec::new();
    let name = dir.file_name().unwrap().to_string_lossy().to_string();
    let p = places(&format!("old-{name}"));
    copy_books(dir, &p.books.join("books-data"));
    let mut m = Mcp::start(&p);
    let list = m.rpc("tools/list", json!({}));
    let tools: Vec<(String, Value)> = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            (
                t["name"].as_str().unwrap().to_string(),
                t["inputSchema"].clone(),
            )
        })
        .collect();
    let mut calls: Vec<(String, Value)> = reads_of_old_books()
        .into_iter()
        .map(|(t, a)| (t.to_string(), a))
        .collect();
    let mut runner = TestRunner::deterministic();
    for (tool, schema) in &tools {
        for _ in 0..per_tool {
            let args = args_for(schema, &p)
                .new_tree(&mut runner)
                .unwrap()
                .current();
            calls.push((tool.clone(), args));
        }
    }
    for (tool, args) in calls {
        let what = format!("{name}: {tool} {}", truncated(&args));
        let r = m.try_rpc("tools/call", json!({"name": tool, "arguments": args}));
        if let Err(e) = check_answer(&mut m, &what, r) {
            failures.push(e.to_string());
            drop(m);
            m = Mcp::start(&p);
        }
    }
    for uri in [
        "books://chart",
        "books://reports/trial-balance",
        "books://reports/balance-sheet",
        "books://reports/profit-and-loss",
        "books://digest",
    ] {
        let r = m.try_rpc("resources/read", json!({"uri": uri}));
        if let Err(e) = check_answer(&mut m, &format!("{name}: {uri}"), r) {
            failures.push(e.to_string());
            drop(m);
            m = Mcp::start(&p);
        }
    }
    // Whatever the calls wrote, the books reopen.
    drop(m);
    let mut m = Mcp::start(&p);
    let r = m.try_rpc(
        "tools/call",
        json!({"name": "books.chart", "arguments": {}}),
    );
    if let Err(e) = check_answer(&mut m, &format!("{name}: reopened"), r) {
        failures.push(e.to_string());
    }
    drop(m);
    let _ = std::fs::remove_dir_all(&p.base);
    failures
}
