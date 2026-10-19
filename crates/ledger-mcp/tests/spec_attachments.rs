//! Spec-blind tests for the attachment-link, statement, and reconciliation
//! tools of `ledger-mcp`, driven over stdio against the real binary.
//!
//! Expected behavior comes only from the tool descriptions and parameter /
//! result doc comments in `src/server.rs` and `src/types.rs` (the declared
//! interface, as `tools/list` serves it), `docs/mcp-interface.md` ("Tools —
//! attachments", "Tools — bank reconciliation"), and
//! `docs/parties-and-attachments.md` ("Attachments", "Reconciliations").
//! The harness is `tests/spec_parties.rs`'s: every success validates
//! against the tool's `outputSchema`, and every refusal leaves
//! `journal.jsonl` byte-identical (server invariant 2).

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
    /// Every response line the server wrote, with the tool it answered.
    transcript: Vec<(String, String)>,
    /// The books dir when the server owns it: removed after the server
    /// stops (fields drop after `Drop::drop`).
    books_dir: Option<TestDir>,
}

impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

static DIR_SEQ: AtomicU64 = AtomicU64::new(0);

/// A test's dir and the folder beside it (`-outside`),
/// removed when the test ends, pass or panic.
struct TestDir(PathBuf);

impl std::ops::Deref for TestDir {
    type Target = std::path::Path;
    fn deref(&self) -> &std::path::Path {
        &self.0
    }
}

impl AsRef<std::path::Path> for TestDir {
    fn as_ref(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
        let _ = std::fs::remove_dir_all(format!("{}-outside", self.0.display()));
    }
}

fn test_dir(name: &str) -> TestDir {
    let n = DIR_SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "ledger-mcp-spec-attachments-{}-{name}-{n}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    TestDir(dir)
}

/// A directory beside the books dir `dir`, outside it: the vault key,
/// files the tests hand the server, files it writes, its stderr.
fn outside(dir: &Path) -> PathBuf {
    let d = PathBuf::from(format!("{}-outside", dir.display()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// `wanderbooks.toml` with a company name, the receipt threshold, and
/// extra `[books]` TOML; the vault key file is outside the books dir.
fn books_toml(dir: &Path, threshold: &str, books_extra: &str) -> PathBuf {
    let path = dir.join("wanderbooks.toml");
    std::fs::write(
        &path,
        format!(
            "[company]\nname = \"Spec Co\"\nfiscal_year_start = \"01-01\"\n\
             [books]\nreceipt_threshold = \"{threshold}\"\ncash_accounts = [\"1000\"]\n{books_extra}\n\
             [vault]\nkey_file = \"{}\"\n",
            outside(dir).join("vault.key").display()
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
            // Never the user's real vault key (the default key file is
            // under XDG_CONFIG_HOME); the tests name theirs anyway.
            .env("XDG_CONFIG_HOME", outside(dir).join("config"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::from(
                std::fs::File::create(outside(dir).join("stderr.log")).unwrap(),
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
        let mut mcp = Self {
            books_dir: None,
            stdin,
            rx,
            child,
            next_id: 1,
            data: dir.join("books-data"),
            validators: HashMap::new(),
            transcript: Vec::new(),
        };
        let init = mcp.rpc(
            "initialize",
            json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "spec-attachments", "version": "0.1.0"},
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
        let what = params["name"].as_str().unwrap_or(method).to_string();
        self.transcript.push((what, resp.to_string()));
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

    /// Draft an entry with `args` extra (party, vendor) and lines; the id.
    fn draft(&mut self, date: &str, extra: Value, lines: &[(&str, &str, &str)]) -> u64 {
        let mut args = json!({"date": date, "description": format!("entry {date}")});
        for (k, v) in extra.as_object().unwrap() {
            args[k] = v.clone();
        }
        let id = self.ok("books.draft_entry", args)["entry_id"]
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

    /// `draft`, then post it (high receipt threshold: no receipt needed).
    fn post(&mut self, date: &str, extra: Value, lines: &[(&str, &str, &str)]) -> u64 {
        let id = self.draft(date, extra, lines);
        self.ok(
            "books.post_entry",
            json!({"entry_id": id, "confirmed": true}),
        );
        id
    }

    fn add_party(&mut self, name: &str, roles: &[&str]) -> u64 {
        self.ok("books.add_party", json!({"name": name, "roles": roles}))["party_id"]
            .as_u64()
            .unwrap()
    }

    /// How many journal lines have this type.
    fn count(&self, line_type: &str) -> usize {
        self.journal_types()
            .iter()
            .filter(|t| t.as_str() == line_type)
            .count()
    }
}

fn b64(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// The SHA-256 (hex) a file given inline comes with.
fn sha(bytes: &[u8]) -> String {
    ledger_store::journal::sha256_hex(bytes)
}

const CHART: &[(&str, &str, &str)] = &[
    ("1000", "Bank", "asset"),
    ("1300", "Prepaid Expenses", "asset"),
    ("1500", "Equipment", "asset"),
    ("1510", "Accumulated Depreciation", "asset"),
    ("2050", "Card", "liability"),
    ("2100", "Due to Members", "liability"),
    ("3000", "Capital", "equity"),
    ("4000", "Revenue", "revenue"),
    ("6000", "Contract Labor", "expense"),
    ("6500", "Depreciation", "expense"),
    ("6999", "General Expenses", "expense"),
];

/// Books with the chart and a receipt threshold of `threshold`.
fn books_at(name: &str, threshold: &str, extra: &str) -> Mcp {
    let dir = test_dir(name);
    let toml = books_toml(&dir, threshold, extra);
    let mut m = Mcp::start(&dir, &toml);
    m.accounts(CHART);
    m.books_dir = Some(dir);
    m
}

/// Books where ordinary entries need no receipt.
fn books(name: &str) -> Mcp {
    books_at(name, "1000000.00", "")
}

impl Mcp {
    /// File a document; its id.
    fn doc(&mut self, category: &str, bytes: &[u8], link_to: Value) -> u64 {
        self.ok(
            "books.file_document",
            json!({"category": category, "file_name": format!("{category}.pdf"),
                   "mime_type": "application/pdf", "content_base64": b64(bytes), "sha256": sha(bytes),
                   "link_to": link_to}),
        )["attachment_id"]
            .as_u64()
            .unwrap()
    }

    /// Attach a receipt (to `entry`, or unlinked); its id.
    fn receipt(&mut self, entry: Option<u64>, bytes: &[u8]) -> u64 {
        let mut args = json!({"file_name": "r.pdf", "mime_type": "application/pdf",
                              "content_base64": b64(bytes), "sha256": sha(bytes)});
        if let Some(e) = entry {
            args["entry_id"] = json!(e);
        }
        self.ok("books.attach_receipt", args)["attachment_id"]
            .as_u64()
            .unwrap()
    }

    fn link(&mut self, attachment: u64, subject: Value) -> Value {
        self.ok(
            "books.link_attachment",
            json!({"attachment_id": attachment, "subject": subject}),
        )
    }

    /// The files on a subject (read-only).
    fn files_on(&mut self, subject: Value) -> Vec<u64> {
        self.read_only("books.attachments", json!({"subject": subject}))["attachments"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["attachment_id"].as_u64().unwrap())
            .collect()
    }

    /// Record a statement for `account`.
    fn statement(&mut self, account: &str, start: &str, end: &str, close: &str) -> u64 {
        let id = self.doc(
            "statement",
            format!("{account} {end}").as_bytes(),
            json!([]),
        );
        self.ok(
            "books.record_statement",
            json!({"attachment_id": id, "account": account, "period_start": start,
                   "period_end": end, "opening_balance": "0.00", "closing_balance": close}),
        );
        id
    }

    /// Mark every line of `entry` on any account cleared.
    fn clear(&mut self, entry: u64, line: usize) {
        self.ok(
            "books.mark_cleared",
            json!({"entry_id": entry, "line_index": line, "cleared": true}),
        );
    }
}

/// The (kind, label) pairs of an output's `links`.
fn links(v: &Value) -> Vec<(String, String)> {
    v["links"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| {
            (
                l["kind"].as_str().unwrap().to_string(),
                l["label"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

fn kinds(v: &Value) -> Vec<String> {
    links(v).into_iter().map(|(k, _)| k).collect()
}

// ------------------------------------------------- books.link_attachment

/// Every subject kind links, each named the way the other tools name it
/// (account by code, asset by tag, party by name), each journaled as its
/// own attachment.linked, and every listing shows the links.
#[test]
fn a_file_links_to_every_kind_of_subject() {
    let mut m = books("link-kinds");
    let e = m.post(
        "2026-01-05",
        json!({}),
        &[("6999", "debit", "10.00"), ("1000", "credit", "10.00")],
    );
    let asset = m.ok(
        "books.register_asset",
        json!({"description": "Laptop", "purchase_date": "2026-01-10", "cost": "3000.00",
               "salvage_value": "0.00", "useful_life_months": 36, "cost_account": "1500",
               "accum_depr_account": "1510", "expense_account": "6500", "tag": "LAP-1",
               "tax_basis_rule": "cost"}),
    );
    assert!(asset.get("asset_id").is_some(), "{asset}");
    m.add_party("Acme Design", &["contractor"]);
    let vendor = m.ok("books.add_vendor", json!({"name": "Landlord"}))["id"].clone();
    let customer = m.ok("books.add_customer", json!({"name": "Client"}))["id"].clone();
    let bill = m.ok(
        "books.create_bill",
        json!({"party_id": vendor, "date": "2026-02-01", "due_date": "2026-02-28",
               "lines": [{"description": "Rent", "amount": "300.00"}]}),
    )["id"]
        .clone();
    let invoice = m.ok(
        "books.create_invoice",
        json!({"party_id": customer, "date": "2026-02-01", "due_date": "2026-03-01",
               "lines": [{"description": "Work", "amount": "500.00"}]}),
    )["id"]
        .clone();
    let period = m.ok(
        "books.create_period",
        json!({"name": "2026-01", "start": "2026-01-01", "end": "2026-01-31"}),
    )["period_id"]
        .clone();
    let schedule = m.ok(
        "books.create_schedule",
        json!({"kind": "prepaid_expense", "description": "Insurance", "total": "1200.00",
               "start": "2026-01-01", "months": 12, "balance_account": "1300", "pl_account": "6999"}),
    )["schedule_id"]
        .clone();
    m.add_party("Pat Doe", &["employee"]);
    let run = m.ok(
        "books.run_payroll",
        json!({"date": "2026-02-15", "cash_account": "1000", "lines": [{
            "employee": "Pat Doe", "gross_pay": "1000.00", "federal_withholding": "100.00",
            "state_withholding": "50.00", "ss_employee": "62.00", "medicare_employee": "14.50",
            "ss_employer": "62.00", "medicare_employer": "14.50", "other_deductions": "0.00"}]}),
    )["run_id"]
        .clone();
    m.clear(e, 1);
    let st = m.statement("1000", "2026-01-01", "2026-01-31", "-10.00");
    let rec = m.ok(
        "books.reconcile",
        json!({"statement_attachment": st, "confirmed": true}),
    )["reconciliation"]["reconciliation_id"]
        .clone();

    let f = m.doc("other", b"one file, many subjects", json!([]));
    let before = m.count("attachment.linked");
    let subjects = [
        json!({"kind": "entry", "id": e}),
        json!({"kind": "account", "id": "1000"}),
        json!({"kind": "asset", "id": "LAP-1"}),
        json!({"kind": "party", "id": "  acme DESIGN "}),
        json!({"kind": "invoice", "id": invoice}),
        json!({"kind": "bill", "id": bill}),
        json!({"kind": "period", "id": period}),
        json!({"kind": "payroll_run", "id": run}),
        json!({"kind": "schedule", "id": schedule}),
        json!({"kind": "reconciliation", "id": rec}),
        json!({"kind": "company"}),
    ];
    for s in &subjects {
        m.link(f, s.clone());
    }
    assert_eq!(m.count("attachment.linked"), before + subjects.len());
    let got = m.read_only("books.get_receipt", json!({"attachment_id": f}));
    let mut ks = kinds(&got);
    ks.sort();
    let mut want: Vec<String> = subjects
        .iter()
        .map(|s| s["kind"].as_str().unwrap().to_string())
        .collect();
    want.sort();
    assert_eq!(ks, want);
    let labels = links(&got);
    assert!(
        labels
            .iter()
            .any(|(k, l)| k == "account" && l.contains("1000") && l.contains("Bank")),
        "{labels:?}"
    );
    assert!(
        labels
            .iter()
            .any(|(k, l)| k == "asset" && l.contains("LAP-1")),
        "{labels:?}"
    );
    assert!(
        labels
            .iter()
            .any(|(k, l)| k == "party" && l.contains("Acme Design")),
        "{labels:?}"
    );
    let company = got["links"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["kind"] == "company")
        .unwrap()
        .clone();
    assert!(company["id"].is_null(), "the company has no id: {company}");
    // The listing shows the same links.
    let listed = m.read_only("books.list_receipts", json!({}))["receipts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["attachment_id"] == json!(f))
        .unwrap()
        .clone();
    assert_eq!(links(&listed), links(&got));
    // Each subject lists the file.
    for s in subjects {
        assert!(m.files_on(s.clone()).contains(&f), "{s} lists it");
    }
    // An account named by id works too (its id is not a code here).
    let chart = m.read_only("books.chart", json!({}));
    let card_id = chart["accounts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["code"] == "2050")
        .unwrap()["id"]
        .clone();
    m.link(f, json!({"kind": "account", "id": card_id}));
    assert!(
        m.files_on(json!({"kind": "account", "id": "2050"}))
            .contains(&f)
    );
    assert_eq!(m.ok("books.verify", json!({}))["status"], "ok");
}

#[test]
fn link_refusals_change_nothing() {
    let mut m = books("link-refusals");
    let e = m.draft("2026-01-05", json!({}), &[]);
    let r = m.receipt(Some(e), b"filed on e");
    let f = m.doc("other", b"a document", json!([]));
    let gone = m.doc("other", b"withdrawn", json!([]));
    m.ok(
        "books.withdraw_attachment",
        json!({"attachment_id": gone, "reason": "not ours"}),
    );
    m.link(f, json!({"kind": "account", "id": "1000"}));
    for (att, subject) in [
        (999, json!({"kind": "company"})),             // unknown attachment
        (gone, json!({"kind": "company"})),            // withdrawn
        (f, json!({"kind": "planet", "id": 1})),       // unknown kind
        (f, json!({"kind": "entry"})),                 // no id
        (f, json!({"kind": "company", "id": 1})),      // the company takes none
        (f, json!({"kind": "entry", "id": 999})),      // no such entry
        (f, json!({"kind": "entry", "id": "five"})),   // not a number
        (f, json!({"kind": "account", "id": "9999"})), // no such account
        (f, json!({"kind": "asset", "id": "NOPE"})),   // no such asset
        (f, json!({"kind": "party", "id": "Nobody"})), // no such party
        (f, json!({"kind": "reconciliation", "id": 1})), // none recorded
        (f, json!({"kind": "account", "id": "1000"})), // linked already
        (r, json!({"kind": "entry", "id": e})),        // filed on it already
    ] {
        m.refuse(
            "books.link_attachment",
            json!({"attachment_id": att, "subject": subject}),
        );
    }
    assert_eq!(m.count("attachment.linked"), 1);
}

// ------------------------------------------------- receipts by link

/// A file linked to an entry is one of its receipts: the policy counts it
/// when posting, and won't let the last one come off a posted entry.
#[test]
fn a_linked_file_is_a_receipt_the_policy_counts() {
    let mut m = books_at("link-receipt", "0.00", "");
    let a = m.draft(
        "2026-01-05",
        json!({}),
        &[("6999", "debit", "10.00"), ("1000", "credit", "10.00")],
    );
    let b = m.draft(
        "2026-01-05",
        json!({}),
        &[("6000", "debit", "20.00"), ("1000", "credit", "20.00")],
    );
    // One invoice covering both entries: filed on a, linked to b.
    let inv = m.receipt(Some(a), b"one invoice, two entries");
    m.refuse(
        "books.post_entry",
        json!({"entry_id": b, "confirmed": true}),
    );
    m.link(inv, json!({"kind": "entry", "id": b}));
    m.ok(
        "books.post_entry",
        json!({"entry_id": a, "confirmed": true}),
    );
    let preview = m.ok("books.post_entry", json!({"entry_id": b}));
    assert_eq!(preview["receipt_missing"], false, "{preview}");
    m.ok(
        "books.post_entry",
        json!({"entry_id": b, "confirmed": true}),
    );
    assert_eq!(m.files_on(json!({"kind": "entry", "id": b})), [inv]);
    let listed = m.read_only("books.list_receipts", json!({"entry_id": b}))["receipts"]
        .as_array()
        .unwrap()
        .len();
    assert_eq!(listed, 1, "list_receipts by entry counts links");
    // Its last receipt can't come off.
    let why = m.refuse(
        "books.unlink_attachment",
        json!({"attachment_id": inv, "subject": {"kind": "entry", "id": b}, "reason": "wrong entry"}),
    );
    assert!(why.contains("receipt_required"), "{why}");
    // A filed receipt comes off with unlink_receipt, not here.
    let why = m.refuse(
        "books.unlink_attachment",
        json!({"attachment_id": inv, "subject": {"kind": "entry", "id": a}, "reason": "x"}),
    );
    assert!(why.contains("books.unlink_receipt"), "{why}");
    // With a replacement on b, the link comes off.
    m.receipt(Some(b), b"b's own receipt");
    let out = m.ok(
        "books.unlink_attachment",
        json!({"attachment_id": inv, "subject": {"kind": "entry", "id": b}, "reason": "b has its own"}),
    );
    assert_eq!(kinds(&out), ["entry"], "still filed on a: {out}");
    assert_eq!(m.count("attachment.unlinked"), 1);
}

// ------------------------------------------------- books.unlink_attachment

#[test]
fn unlink_refusals_change_nothing() {
    let mut m = books("unlink-refusals");
    let f = m.doc("contract", b"lease", json!([{"kind": "company"}]));
    for args in [
        json!({"attachment_id": f, "subject": {"kind": "company"}, "reason": "  "}),
        json!({"attachment_id": f, "subject": {"kind": "account", "id": "1000"}, "reason": "not there"}),
        json!({"attachment_id": 999, "subject": {"kind": "company"}, "reason": "x"}),
        json!({"attachment_id": f, "subject": {"kind": "planet"}, "reason": "x"}),
    ] {
        m.refuse("books.unlink_attachment", args);
    }
    let out = m.ok(
        "books.unlink_attachment",
        json!({"attachment_id": f, "subject": {"kind": "company"}, "reason": "a lease, not formation"}),
    );
    assert!(kinds(&out).is_empty(), "{out}");
    assert!(m.files_on(json!({"kind": "company"})).is_empty());
    // The file stays filed and listed.
    let got = m.read_only("books.get_receipt", json!({"attachment_id": f}));
    assert_eq!(got["category"], "contract");
    // The journal keeps both the link and the unlink, with the reason.
    let journal = String::from_utf8(m.journal()).unwrap();
    assert!(journal.contains("a lease, not formation"));
    assert_eq!(m.count("attachment.linked"), 1);
    assert_eq!(m.count("attachment.unlinked"), 1);
}

// ------------------------------------------------- link_to on filing

/// `link_to` links in the same call, each link its own event; one bad
/// subject files nothing.
#[test]
fn filing_links_in_the_same_call_all_or_nothing() {
    let mut m = books("link-to");
    let e = m.draft("2026-01-05", json!({}), &[]);
    let other = m.draft("2026-01-06", json!({}), &[]);
    m.add_party("Acme Design", &["contractor"]);
    let count = |m: &mut Mcp| {
        m.read_only("books.list_receipts", json!({"include_withdrawn": true}))["receipts"]
            .as_array()
            .unwrap()
            .len()
    };
    let n = count(&mut m);
    for link_to in [
        json!([{"kind": "party", "id": "Acme Design"}, {"kind": "entry", "id": 999}]),
        json!([{"kind": "company"}, {"kind": "company"}]),
        json!([{"kind": "entry", "id": e}]),
        json!([{"kind": "nope"}]),
    ] {
        m.refuse(
            "books.attach_receipt",
            json!({"entry_id": e, "file_name": "r.pdf", "mime_type": "application/pdf",
                   "content_base64": b64(b"r"), "sha256": sha(b"r"), "link_to": link_to}),
        );
    }
    m.refuse(
        "books.file_document",
        json!({"category": "contract", "file_name": "c.pdf", "mime_type": "application/pdf",
               "content_base64": b64(b"c"), "sha256": sha(b"c"), "link_to": [{"kind": "asset", "id": "NOPE"}]}),
    );
    // An unknown entry_id is refused before anything is filed.
    m.refuse(
        "books.attach_receipt",
        json!({"entry_id": 999, "file_name": "r.pdf", "mime_type": "application/pdf",
               "content_base64": b64(b"r"), "sha256": sha(b"r"), "link_to": [{"kind": "company"}]}),
    );
    assert_eq!(count(&mut m), n, "nothing filed");
    let out = m.ok(
        "books.attach_receipt",
        json!({"entry_id": e, "file_name": "r.pdf", "mime_type": "application/pdf",
               "content_base64": b64(b"r"), "sha256": sha(b"r"),
               "link_to": [{"kind": "entry", "id": other}, {"kind": "party", "id": "acme design"}]}),
    );
    assert_eq!(
        kinds(&out),
        ["entry", "entry", "party"],
        "filed entry first: {out}"
    );
    assert_eq!(out["links"][0]["id"], json!(e));
    assert_eq!(m.count("receipt.attached"), 1);
    assert_eq!(m.count("attachment.linked"), 2);
    let doc = m.ok(
        "books.file_document",
        json!({"category": "formation", "file_name": "cert.pdf", "mime_type": "application/pdf",
               "content_base64": b64(b"certificate"), "sha256": sha(b"certificate"), "link_to": [{"kind": "company"}]}),
    );
    assert_eq!(kinds(&doc), ["company"]);
    assert_eq!(m.count("document.filed"), 1);
}

#[test]
fn documents_file_under_the_attachment_kinds() {
    let mut m = books("categories");
    for c in [
        "formation",
        "legal",
        "tax",
        "accounting",
        "statement",
        "contract",
        "photo",
        "warranty",
        "import_source",
        "other",
    ] {
        let id = m.doc(c, c.as_bytes(), json!([]));
        assert_eq!(
            m.read_only("books.get_receipt", json!({"attachment_id": id}))["category"],
            c
        );
    }
    m.refuse(
        "books.file_document",
        json!({"category": "receipt-ish", "file_name": "x", "mime_type": "text/plain",
               "content_base64": b64(b"x"), "sha256": sha(b"x")}),
    );
}

// ------------------------------------------------- books.attachments

/// An entry's files are those filed on it and those linked to it, without
/// the withdrawn; each comes with all its links.
#[test]
fn a_subjects_files_are_filed_and_linked_not_withdrawn() {
    let mut m = books("attachments");
    let e = m.draft("2026-01-05", json!({}), &[]);
    let filed = m.receipt(Some(e), b"filed");
    let linked = m.doc(
        "other",
        b"linked",
        json!([{"kind": "entry", "id": e}, {"kind": "company"}]),
    );
    let gone = m.receipt(None, b"to withdraw");
    m.link(gone, json!({"kind": "company"}));
    m.ok(
        "books.unlink_attachment",
        json!({"attachment_id": gone, "subject": {"kind": "company"}, "reason": "wrong"}),
    );
    m.ok(
        "books.withdraw_attachment",
        json!({"attachment_id": gone, "reason": "a duplicate"}),
    );
    assert_eq!(
        m.files_on(json!({"kind": "entry", "id": e})),
        [filed, linked]
    );
    assert_eq!(m.files_on(json!({"kind": "company"})), [linked]);
    let out = m.read_only(
        "books.attachments",
        json!({"subject": {"kind": "entry", "id": e}}),
    );
    assert_eq!(out["subject"]["kind"], "entry");
    assert_eq!(out["subject"]["id"], json!(e));
    // Its links: the filing entry first (none here), then by kind.
    assert_eq!(kinds(&out["attachments"][1]), ["company", "entry"]);
    m.refuse(
        "books.attachments",
        json!({"subject": {"kind": "entry", "id": 999}}),
    );
}

// ------------------------------------------------- statements

#[test]
fn record_statement_links_it_to_its_account() {
    let mut m = books("statement");
    let f = m.doc("statement", b"jan", json!([]));
    let st = m.ok(
        "books.record_statement",
        json!({"attachment_id": f, "account": "1000", "period_start": "2026-01-01",
               "period_end": "2026-01-31", "opening_balance": "0", "closing_balance": "1250.5"}),
    );
    assert_eq!(st["account_code"], "1000");
    assert_eq!(st["account_name"], "Bank");
    assert_eq!(st["period_end"], "2026-01-31");
    assert_eq!(st["opening_balance"], "0.00");
    assert_eq!(st["closing_balance"], "1250.50");
    assert_eq!(st["file_name"], "statement.pdf");
    assert!(st["reconciliation_id"].is_null());
    assert_eq!(m.count("statement.recorded"), 1);
    assert!(
        m.files_on(json!({"kind": "account", "id": "1000"}))
            .contains(&f)
    );
    // A card statement too (a liability).
    let card = m.statement("2050", "2026-01-01", "2026-01-31", "80.00");
    let all = m.read_only("books.statements", json!({}))["statements"]
        .as_array()
        .unwrap()
        .len();
    assert_eq!(all, 2);
    let one = m.read_only("books.statements", json!({"account": "2050"}));
    assert_eq!(one["statements"].as_array().unwrap().len(), 1);
    assert_eq!(one["statements"][0]["attachment_id"], json!(card));
}

#[test]
fn record_statement_refusals_change_nothing() {
    let mut m = books("statement-refusals");
    let f = m.doc("statement", b"jan", json!([]));
    let gone = m.doc("statement", b"gone", json!([]));
    m.ok(
        "books.withdraw_attachment",
        json!({"attachment_id": gone, "reason": "duplicate"}),
    );
    let base = json!({"attachment_id": f, "account": "1000", "period_start": "2026-01-01",
                      "period_end": "2026-01-31", "opening_balance": "0.00",
                      "closing_balance": "10.00"});
    let with = |k: &str, v: Value| {
        let mut a = base.clone();
        a[k] = v;
        a
    };
    for args in [
        with("attachment_id", json!(999)),
        with("attachment_id", json!(gone)),
        with("account", json!("6999")),           // an expense account
        with("account", json!("4000")),           // revenue
        with("account", json!("9999")),           // unknown
        with("period_end", json!("2025-12-31")),  // backwards
        with("period_end", json!("2026-02-30")),  // not a date
        with("closing_balance", json!("10.001")), // past cents
        with("closing_balance", json!("1e1")),    // not a decimal string
        with("opening_balance", json!(10.0)),     // a JSON number
    ] {
        m.refuse("books.record_statement", args);
    }
    m.ok("books.record_statement", base.clone());
    m.refuse("books.record_statement", base);
    assert_eq!(m.count("statement.recorded"), 1);
}

// ------------------------------------------------- books.reconcile

/// Books with a January: a deposit, a payment, and one more payment after
/// the statement date. Returns (deposit, payment) entry ids.
fn january(m: &mut Mcp) -> (u64, u64) {
    let dep = m.post(
        "2026-01-02",
        json!({}),
        &[("1000", "debit", "1000.00"), ("3000", "credit", "1000.00")],
    );
    let pay = m.post(
        "2026-01-20",
        json!({}),
        &[("6000", "debit", "200.00"), ("1000", "credit", "200.00")],
    );
    m.post(
        "2026-02-03",
        json!({}),
        &[("6999", "debit", "5.00"), ("1000", "credit", "5.00")],
    );
    (dep, pay)
}

#[test]
fn reconcile_previews_without_recording() {
    let mut m = books("reconcile-preview");
    let (dep, pay) = january(&mut m);
    // A quick check against any balance: a preview, never recorded.
    let p = m.read_only(
        "books.reconcile",
        json!({"account": "1000", "statement_date": "2026-01-31", "statement_balance": "800.00"}),
    );
    assert_eq!(p["status"], "preview");
    assert_eq!(p["book_balance"], "795.00");
    assert_eq!(p["cleared_balance"], "0.00");
    assert_eq!(p["difference"], "-800.00");
    assert_eq!(p["uncleared_lines"].as_array().unwrap().len(), 2);
    assert!(p["cleared_lines"].as_array().unwrap().is_empty());
    assert!(p["reconciliation"].is_null());
    assert!(!p["problems"].as_array().unwrap().is_empty());
    // Confirming without a recorded statement is refused.
    m.refuse(
        "books.reconcile",
        json!({"account": "1000", "statement_date": "2026-01-31", "statement_balance": "800.00",
               "confirmed": true}),
    );
    // Neither a statement nor all three facts.
    m.refuse("books.reconcile", json!({"account": "1000"}));
    // Against a recorded statement, with the lines cleared: no problems.
    let st = m.statement("1000", "2026-01-01", "2026-01-31", "800.00");
    m.clear(dep, 0);
    m.clear(pay, 1);
    let p = m.read_only("books.reconcile", json!({"statement_attachment": st}));
    assert_eq!(p["status"], "preview");
    assert_eq!(p["statement_attachment"], json!(st));
    assert_eq!(p["statement_date"], "2026-01-31");
    assert_eq!(p["cleared_balance"], "800.00");
    assert_eq!(p["difference"], "0.00");
    assert!(p["problems"].as_array().unwrap().is_empty(), "{p}");
    let cleared: Vec<(u64, u64)> = p["cleared_lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| {
            (
                l["entry_id"].as_u64().unwrap(),
                l["line_index"].as_u64().unwrap(),
            )
        })
        .collect();
    assert_eq!(cleared, [(dep, 0), (pay, 1)]);
    assert_eq!(m.count("reconciliation.completed"), 0);
}

#[test]
fn reconcile_confirmed_records_against_the_statement() {
    let mut m = books("reconcile-confirm");
    let (dep, pay) = january(&mut m);
    let st = m.statement("1000", "2026-01-01", "2026-01-31", "800.00");
    m.clear(dep, 0);
    // A difference refuses, naming it.
    let why = m.refuse(
        "books.reconcile",
        json!({"statement_attachment": st, "confirmed": true}),
    );
    assert!(why.contains("200.00"), "{why}");
    m.clear(pay, 1);
    // Facts given beside the statement must be the statement's.
    for (k, v) in [
        ("account", json!("2050")),
        ("statement_date", json!("2026-01-30")),
        ("statement_balance", json!("800.01")),
    ] {
        let mut args = json!({"statement_attachment": st, "confirmed": true});
        args[k] = v;
        m.refuse("books.reconcile", args);
    }
    // A file that records no statement.
    let not = m.doc("other", b"not a statement", json!([]));
    m.refuse(
        "books.reconcile",
        json!({"statement_attachment": not, "confirmed": true}),
    );
    let out = m.ok(
        "books.reconcile",
        json!({"statement_attachment": st, "account": "1000", "confirmed": true}),
    );
    assert_eq!(out["status"], "reconciled");
    let r = &out["reconciliation"];
    assert_eq!(r["account_code"], "1000");
    assert_eq!(r["statement_date"], "2026-01-31");
    assert_eq!(r["statement_balance"], "800.00");
    assert_eq!(r["statement_attachment"], json!(st));
    assert_eq!(r["voided"], false);
    assert_eq!(
        r["cleared"],
        json!([{"entry_id": dep, "line_index": 0}, {"entry_id": pay, "line_index": 1}])
    );
    assert_eq!(m.count("reconciliation.completed"), 1);
    let rid = r["reconciliation_id"].as_u64().unwrap();
    // The statement is linked to the reconciliation and lists it.
    assert!(
        m.files_on(json!({"kind": "reconciliation", "id": rid}))
            .contains(&st)
    );
    let sts = m.read_only("books.statements", json!({"account": "1000"}));
    assert_eq!(sts["statements"][0]["reconciliation_id"], json!(rid));
    // The same date again is refused; the preview says why.
    let p = m.read_only("books.reconcile", json!({"statement_attachment": st}));
    assert!(
        p["problems"].as_array().unwrap().iter().any(|x| x
            .as_str()
            .unwrap()
            .contains("already reconciled through 2026-01-31")),
        "{p}"
    );
    m.refuse(
        "books.reconcile",
        json!({"statement_attachment": st, "confirmed": true}),
    );
    // The statement can't come off a reconciliation that stands.
    m.refuse(
        "books.unlink_attachment",
        json!({"attachment_id": st, "subject": {"kind": "reconciliation", "id": rid}, "reason": "x"}),
    );
    // February, forward: fine.
    let feb_pay = m.read_only("books.search_entries", json!({"text": "2026-02-03"}));
    let feb = feb_pay["entries"][0]["entry_id"].as_u64().unwrap();
    m.clear(feb, 1);
    let st2 = m.statement("1000", "2026-02-01", "2026-02-28", "795.00");
    m.ok(
        "books.reconcile",
        json!({"statement_attachment": st2, "confirmed": true}),
    );
    let all = m.read_only("books.reconciliations", json!({"account": "1000"}));
    assert_eq!(all["reconciliations"].as_array().unwrap().len(), 2);
    assert!(
        m.read_only("books.reconciliations", json!({"account": "2050"}))["reconciliations"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(m.ok("books.verify", json!({}))["status"], "ok");
}

#[test]
fn void_reconciliation_lets_it_be_redone() {
    let mut m = books("void");
    let (dep, pay) = january(&mut m);
    m.clear(dep, 0);
    m.clear(pay, 1);
    let st = m.statement("1000", "2026-01-01", "2026-01-31", "800.00");
    let rid = m.ok(
        "books.reconcile",
        json!({"statement_attachment": st, "confirmed": true}),
    )["reconciliation"]["reconciliation_id"]
        .as_u64()
        .unwrap();
    for args in [
        json!({"reconciliation_id": rid, "reason": " "}),
        json!({"reconciliation_id": 999, "reason": "no such"}),
    ] {
        m.refuse("books.void_reconciliation", args);
    }
    let v = m.ok(
        "books.void_reconciliation",
        json!({"reconciliation_id": rid, "reason": "cleared the wrong deposit"}),
    );
    assert_eq!(v["voided"], true);
    assert_eq!(v["voided_reason"], "cleared the wrong deposit");
    m.refuse(
        "books.void_reconciliation",
        json!({"reconciliation_id": rid, "reason": "again"}),
    );
    assert_eq!(m.count("reconciliation.voided"), 1);
    // Voided: the statement can come off it, and the month can be redone.
    let listed = m.read_only("books.reconciliations", json!({}));
    assert_eq!(listed["reconciliations"][0]["voided"], true);
    m.ok(
        "books.unlink_attachment",
        json!({"attachment_id": st, "subject": {"kind": "reconciliation", "id": rid},
               "reason": "voided"}),
    );
    let redo = m.ok(
        "books.reconcile",
        json!({"statement_attachment": st, "confirmed": true}),
    );
    assert_ne!(redo["reconciliation"]["reconciliation_id"], json!(rid));
}

/// books.reconcile SCENARIO 1: a statement whose file was withdrawn is
/// retired; the preview and the confirmation are refused, and nothing is
/// journaled (it used to return an error after journaling
/// reconciliation.completed). books.statements no longer lists it.
#[test]
fn a_withdrawn_statement_is_retired_and_reconciles_nothing() {
    let mut m = books("withdrawn-statement");
    let (dep, pay) = january(&mut m);
    m.clear(dep, 0);
    m.clear(pay, 1);
    let st = m.statement("1000", "2026-01-01", "2026-01-31", "800.00");
    m.ok(
        "books.unlink_attachment",
        json!({"attachment_id": st, "subject": {"kind": "account", "id": 1}, "reason": "wrong file"}),
    );
    m.ok(
        "books.withdraw_attachment",
        json!({"attachment_id": st, "reason": "wrong file"}),
    );
    let why = m.refuse("books.reconcile", json!({"statement_attachment": st}));
    assert!(why.contains("withdrawn"), "{why}");
    m.refuse(
        "books.reconcile",
        json!({"statement_attachment": st, "confirmed": true}),
    );
    assert_eq!(m.count("reconciliation.completed"), 0);
    assert!(
        m.read_only("books.reconciliations", json!({}))["reconciliations"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        m.read_only("books.statements", json!({}))["statements"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(m.ok("books.verify", json!({}))["status"], "ok");
}

/// books.void_statement SCENARIOS 1–2: a statement recorded with the wrong
/// closing balance is voided and recorded again on the same file, then
/// reconciles; voiding it while that reconciliation stands is refused.
#[test]
fn a_mis_recorded_statement_is_voided_and_recorded_again() {
    let mut m = books("void-statement");
    let (dep, pay) = january(&mut m);
    m.clear(dep, 0);
    m.clear(pay, 1);
    let st = m.statement("1000", "2026-01-01", "2026-01-31", "880.00");
    m.refuse(
        "books.void_statement",
        json!({"attachment_id": st, "reason": " "}),
    );
    let v = m.ok(
        "books.void_statement",
        json!({"attachment_id": st, "reason": "closing balance mistyped"}),
    );
    assert_eq!(v["closing_balance"], "880.00");
    assert_eq!(m.count("statement.voided"), 1);
    m.refuse(
        "books.void_statement",
        json!({"attachment_id": st, "reason": "again"}),
    );
    assert!(
        m.read_only("books.statements", json!({}))["statements"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    m.ok(
        "books.record_statement",
        json!({"attachment_id": st, "account": "1000", "period_start": "2026-01-01",
               "period_end": "2026-01-31", "opening_balance": "0.00", "closing_balance": "800.00"}),
    );
    m.ok(
        "books.reconcile",
        json!({"statement_attachment": st, "confirmed": true}),
    );
    m.refuse(
        "books.void_statement",
        json!({"attachment_id": st, "reason": "why"}),
    );
    // A line the reconciliation cleared stays cleared while it stands.
    m.refuse(
        "books.mark_cleared",
        json!({"entry_id": pay, "line_index": 1, "cleared": false}),
    );
    assert_eq!(m.ok("books.verify", json!({}))["status"], "ok");
}

/// books.export_cpa_package `include_receipts`: receipts/ holds every
/// current file behind each posted entry, filed on it or linked to it (a
/// linked file counts as the entry's receipt, so the receipt policy let
/// the entry post), one copy per entry with an index row; documents/ holds
/// the files behind no entry. A linked-only receipt used to be left out
/// of the package entirely.
#[test]
fn the_cpa_package_carries_receipts_linked_to_entries() {
    use base64::Engine as _;
    let mut m = books_at("cpa-linked", "0.00", "");
    let receipt = m.receipt(None, b"%PDF linked receipt");
    let e = m.draft(
        "2026-03-01",
        json!({}),
        &[("6999", "debit", "40.00"), ("1000", "credit", "40.00")],
    );
    m.link(receipt, json!({"kind": "entry", "id": e}));
    // A contract linked to the same entry is evidence for it too.
    m.doc(
        "contract",
        b"%PDF contract",
        json!([{"kind": "entry", "id": e}]),
    );
    m.ok(
        "books.post_entry",
        json!({"entry_id": e, "confirmed": true}),
    );
    let pkg = m.ok(
        "books.export_cpa_package",
        json!({"as_of": "2026-12-31", "cash_accounts": ["1000"], "include_tins": false}),
    );
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(pkg["content_base64"].as_str().unwrap())
        .unwrap();
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let names: Vec<String> = (0..zip.len())
        .map(|i| zip.by_index(i).unwrap().name().unwrap().to_string())
        .collect();
    let mut index = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("receipts/index.csv").unwrap(), &mut index)
        .unwrap();
    let rows: Vec<&str> = index.lines().skip(1).collect();
    let for_e: Vec<&&str> = rows
        .iter()
        .filter(|r| r.starts_with(&format!("{e},")))
        .collect();
    assert_eq!(for_e.len(), 2, "the linked receipt and contract: {index}");
    for r in for_e {
        let file = r.split(',').nth(4).unwrap();
        assert!(names.iter().any(|n| n == file), "{file} in {names:?}");
    }
    assert!(
        !names.iter().any(|n| n.starts_with("documents/contract")),
        "the contract is behind an entry, filed with it: {names:?}"
    );
}

// ------------------------------------------------- import sources

#[test]
fn a_bank_import_links_its_source_to_the_account_not_the_drafts() {
    let mut m = books("import-source");
    let csv = "Date,Description,Amount\n2026-03-05,Coffee,-4.50\n2026-03-06,Refund,10.00\n";
    let src = m.doc("import_source", csv.as_bytes(), json!([]));
    let gone = m.doc("import_source", b"old", json!([]));
    m.ok(
        "books.withdraw_attachment",
        json!({"attachment_id": gone, "reason": "superseded"}),
    );
    let args = |source: u64| {
        json!({"csv_text": csv, "bank_account_code": "1000", "default_offset_code": "6999",
               "source_attachment_id": source})
    };
    m.refuse("books.import_bank_csv", args(999));
    m.refuse("books.import_bank_csv", args(gone));
    let out = m.ok("books.import_bank_csv", args(src));
    assert_eq!(out["count"], 2);
    assert_eq!(out["source_attachment_id"], json!(src));
    assert_eq!(out["source_linked"], true);
    assert!(
        m.files_on(json!({"kind": "account", "id": "1000"}))
            .contains(&src)
    );
    for t in out["imported"].as_array().unwrap() {
        let e = t["entry_id"].clone();
        assert!(
            m.files_on(json!({"kind": "entry", "id": e})).is_empty(),
            "a bank export is not a receipt"
        );
    }
    // Linked already: imported again, not linked twice.
    let again = m.ok("books.import_bank_csv", args(src));
    assert_eq!(again["source_linked"], false);
    assert_eq!(m.count("attachment.linked"), 1);
    // Without a source, as before.
    let plain = m.ok(
        "books.import_bank_csv",
        json!({"csv_text": csv, "bank_account_code": "1000", "default_offset_code": "6999"}),
    );
    assert!(plain["source_attachment_id"].is_null());
    assert_eq!(plain["source_linked"], false);
}

// ------------------------------------------------- the viewer

#[test]
fn show_opens_a_reconciliation() {
    let mut m = books("show");
    let (dep, pay) = january(&mut m);
    m.clear(dep, 0);
    m.clear(pay, 1);
    let st = m.statement("1000", "2026-01-01", "2026-01-31", "800.00");
    let rid = m.ok(
        "books.reconcile",
        json!({"statement_attachment": st, "confirmed": true}),
    )["reconciliation"]["reconciliation_id"]
        .as_u64()
        .unwrap();
    let out = m.read_only("books.show", json!({"reconciliation": rid}));
    assert!(out["shown"].as_str().unwrap().contains("1000"), "{out}");
    let run = m.data.parent().unwrap().join("run/wanderbooks");
    let cmd: Value = std::fs::read_dir(&run)
        .unwrap()
        .filter_map(|e| std::fs::read(e.unwrap().path()).ok())
        .find_map(|b| serde_json::from_slice(&b).ok())
        .expect("a viewer request");
    assert_eq!(cmd["target"], json!({"show": "reconciliation", "id": rid}));
    m.refuse("books.show", json!({"reconciliation": 999}));
    m.refuse(
        "books.show",
        json!({"reconciliation": rid, "entry_id": dep}),
    );
}

// ------------------------------------------------- books.digest

/// The end of last month, from today.
fn last_month_end() -> chrono::NaiveDate {
    use chrono::Datelike;
    let today = chrono::Local::now().date_naive();
    today.with_day(1).unwrap().pred_opt().unwrap()
}

fn unreconciled(m: &mut Mcp) -> Vec<String> {
    m.read_only("books.digest", json!({"since": "2020-01-01"}))["unreconciled_accounts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["code"].as_str().unwrap().to_string())
        .collect()
}

/// Cash accounts (`[books] cash_accounts`), card accounts (liabilities
/// named "card", before their first statement, so the first
/// reconciliation is asked for), and any other liability once it has a
/// statement are listed until reconciled through the end of last month;
/// accounts without activity by then aren't (ledger_store::reconcilable,
/// the viewer's rule too).
#[test]
fn the_digest_lists_accounts_not_reconciled_through_last_month() {
    let mut m = books("digest");
    assert!(unreconciled(&mut m).is_empty(), "no activity, nothing due");
    let due = last_month_end();
    let day = due.to_string();
    let start = due.with_day0(0).unwrap().to_string();
    let dep = m.post(
        &day,
        json!({}),
        &[("1000", "debit", "100.00"), ("3000", "credit", "100.00")],
    );
    let card = m.post(
        &day,
        json!({}),
        &[("6999", "debit", "40.00"), ("2050", "credit", "40.00")],
    );
    let owed = m.post(
        &day,
        json!({}),
        &[("6999", "debit", "15.00"), ("2100", "credit", "15.00")],
    );
    assert_eq!(
        unreconciled(&mut m),
        ["1000", "2050"],
        "the card is due before its first statement; 2100 isn't a card"
    );
    let d = m.read_only("books.digest", json!({"since": "2020-01-01"}));
    let row = &d["unreconciled_accounts"][0];
    assert_eq!(row["due_through"], json!(day));
    assert!(row["reconciled_through"].is_null());
    assert!(
        row["next_step"]
            .as_str()
            .unwrap()
            .contains("books.record_statement")
    );
    let card_st = m.statement("2050", &start, &day, "40.00");
    m.clear(card, 1);
    m.ok(
        "books.reconcile",
        json!({"statement_attachment": card_st, "confirmed": true}),
    );
    assert_eq!(unreconciled(&mut m), ["1000"]);
    // A liability not named for a card joins with its first statement.
    let owed_st = m.statement("2100", &start, &day, "15.00");
    assert_eq!(unreconciled(&mut m), ["1000", "2100"]);
    m.clear(owed, 1);
    m.ok(
        "books.reconcile",
        json!({"statement_attachment": owed_st, "confirmed": true}),
    );
    m.clear(dep, 0);
    let st = m.statement("1000", &start, &day, "100.00");
    m.ok(
        "books.reconcile",
        json!({"statement_attachment": st, "confirmed": true}),
    );
    assert!(unreconciled(&mut m).is_empty());
}

use chrono::Datelike as _;
