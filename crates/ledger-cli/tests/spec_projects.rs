//! Spec-blind tests for the project tools of the books server (rules.md
//! B-37), driven through a real `wanderbooks server` as an agent reaches
//! it (common.rs).
//!
//! Expected behavior comes only from the tool descriptions and parameter /
//! result doc comments in `src/server.rs` and `src/types.rs` (the declared
//! interface, as `tools/list` serves it) and `docs/mcp-interface.md`
//! ("Tools — parties"). The harness is the minimal subset of
//! `tests/spec_mcp.rs` (copied from `tests/spec_assets.rs`): every success
//! validates against the tool's `outputSchema`, and every refusal leaves
//! `journal.jsonl` byte-identical (server invariant 2: a refused call
//! changes nothing).

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
                "clientInfo": {"name": "spec-projects", "version": "0.1.0"},
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

    /// How many journal lines have this type.
    fn count(&self, line_type: &str) -> usize {
        self.journal_types()
            .iter()
            .filter(|t| t.as_str() == line_type)
            .count()
    }
}

const CHART: &[(&str, &str, &str)] = &[
    ("1000", "Bank", "asset"),
    ("1200", "Equipment", "asset"),
    ("3000", "Capital", "equity"),
    ("4000", "Revenue", "revenue"),
    ("6000", "Contract Labor", "expense"),
    ("6200", "Rent", "expense"),
];

/// Books with the chart; cash is 1000.
fn books(name: &str) -> Mcp {
    books_in(name, false)
}

/// [`books`]; `p2`: as P2 left them (format 2, no P3 feature), waiting
/// for their format upgrade.
fn books_in(name: &str, p2: bool) -> Mcp {
    let dir = test_dir(name);
    if p2 {
        ledger_store::Store::create_in_format(
            &dir.books_data(),
            ledger_core::Currency::USD,
            &ledger_store::JournalFormat {
                format: 2,
                features: Vec::new(),
            },
        )
        .unwrap();
    }
    let toml = books_toml(&dir, "Spec Co", "");
    let mut m = Mcp::start(&dir, &toml);
    m.accounts(CHART);
    m.books_dir = Some(dir);
    m
}

impl Mcp {
    /// Draft with lines (account, side, amount, class), post it; the id.
    fn tagged(
        &mut self,
        date: &str,
        what: &str,
        lines: &[(&str, &str, &str, Option<&str>)],
    ) -> u64 {
        let id = self.ok(
            "books.draft_entry",
            json!({"date": date, "description": what, "no_party_reason": "test"}),
        )["entry_id"]
            .as_u64()
            .unwrap();
        for (account, side, amount, class) in lines {
            let mut a = json!({"entry_id": id, "account": account, "side": side, "amount": amount});
            if let Some(c) = class {
                a["class"] = json!(c);
            }
            self.ok("books.add_line", a);
        }
        self.ok(
            "books.post_entry",
            json!({"entry_id": id, "confirmed": true}),
        );
        id
    }

    /// File `bytes` as a contract, uploaded as an agent uploads them.
    fn doc(&mut self, bytes: &[u8], link_to: Value) -> Out {
        let upload = self
            .wire
            .upload(bytes, "spec.pdf", "application/pdf", "document");
        self.tool(
            "books.file_document",
            json!({"category": "contract", "upload_id": upload, "link_to": link_to}),
        )
    }
}

/// Harbor app (on spec): $1,400.00 + $60.00 of costs, repaid $1,000.00;
/// Zeta site (client): $5,000.00 earned, $1,800.00 of contractors, one
/// line in 2025; rent is no project's; test devices tagged Harbor.
fn studio(name: &str) -> (Mcp, u64, u64) {
    let mut m = books(name);
    m.ok(
        "books.add_project",
        json!({"name": "Harbor app", "kind": "on_spec", "started_on": "2026-04-01"}),
    );
    m.ok(
        "books.add_project",
        json!({"name": "Zeta site", "kind": "client"}),
    );
    m.tagged(
        "2025-12-10",
        "Zeta deposit",
        &[
            ("1000", "debit", "500.00", None),
            ("4000", "credit", "500.00", Some("Zeta site")),
        ],
    );
    let proto = m.tagged(
        "2026-04-02",
        "Prototype",
        &[
            ("6000", "debit", "1400.00", Some("Harbor app")),
            ("1000", "credit", "1400.00", None),
        ],
    );
    m.tagged(
        "2026-04-20",
        "Hosting",
        &[
            ("6000", "debit", "60.00", Some("Harbor app")),
            ("1000", "credit", "60.00", None),
        ],
    );
    m.tagged(
        "2026-04-21",
        "Devices",
        &[
            ("1200", "debit", "900.00", Some("Harbor app")),
            ("1000", "credit", "900.00", None),
        ],
    );
    m.tagged(
        "2026-06-15",
        "Harbor repays",
        &[
            ("1000", "debit", "1000.00", None),
            ("4000", "credit", "1000.00", Some("Harbor app")),
        ],
    );
    m.tagged(
        "2026-03-02",
        "Zeta milestone",
        &[
            ("1000", "debit", "4500.00", None),
            ("4000", "credit", "4500.00", Some("Zeta site")),
        ],
    );
    let zc = m.tagged(
        "2026-03-09",
        "Zeta contractor",
        &[
            ("6000", "debit", "1800.00", Some("Zeta site")),
            ("1000", "credit", "1800.00", None),
        ],
    );
    m.tagged(
        "2026-03-31",
        "Rent",
        &[
            ("6200", "debit", "1500.00", None),
            ("1000", "credit", "1500.00", None),
        ],
    );
    (m, proto, zc)
}

// ------------------------------------------------- add, update

#[test]
fn add_project_then_project_returns_its_facts_and_figures() {
    let mut m = books("add");
    let party = m.ok("books.add_party", json!({"name": "Harbor Co"}))["party_id"]
        .as_u64()
        .unwrap();
    let p = m.ok(
        "books.add_project",
        json!({"name": "  Harbor app ", "description": "An app", "kind": "on_spec",
               "party": "harbor co", "started_on": "2026-04-01",
               "links": ["https://harbor.example"], "notes": "pitched in March"}),
    );
    assert_eq!(p["name"], "Harbor app");
    assert_eq!(p["status"], "active");
    assert_eq!(p["kind"], "on_spec");
    assert_eq!(p["party"]["party_id"], party);
    assert_eq!(p["party"]["label"], "Harbor Co");
    assert_eq!(p["links"], json!(["https://harbor.example"]));
    assert_eq!(m.count("class.added"), 1);
    assert_eq!(m.count("project.set"), 1);
    let d = m.read_only(
        "books.project",
        json!({"project": "harbor app", "start": "2026-01-01", "end": "2026-12-31"}),
    );
    assert_eq!(d["project"]["notes"], "pitched in March");
    assert_eq!(d["period"]["net"], "0.00");
    assert_eq!(d["period"]["by_month"].as_array().unwrap().len(), 12);
    assert_eq!(d["to_date"]["unrecovered_cost"], "0.00");
    // The class tools see it.
    let classes = m.read_only("books.list_classes", json!({}));
    assert_eq!(classes["classes"][0]["name"], "Harbor app");
}

#[test]
fn a_name_only_project_journals_only_class_added() {
    let mut m = books("name-only");
    let p = m.ok("books.add_project", json!({"name": "Dock"}));
    assert_eq!(p["kind"], Value::Null);
    assert_eq!(m.count("class.added"), 1);
    assert_eq!(m.count("project.set"), 0);
    // add_class refuses a taken name now.
    let why = m.refuse("books.add_class", json!({"name": "DOCK"}));
    assert!(why.contains("already exists"), "{why}");
}

#[test]
fn update_project_checks_every_field_before_writing() {
    let mut m = books("update-checks");
    m.ok("books.add_project", json!({"name": "Harbor"}));
    m.ok("books.add_project", json!({"name": "Dock"}));
    for (args, needle) in [
        (json!({"project": "Harbor", "name": " "}), "blank"),
        (
            json!({"project": "Harbor", "name": "dock"}),
            "already exists",
        ),
        (json!({"project": "Harbor", "status": "done"}), "status"),
        (json!({"project": "Harbor", "kind": "grant"}), "kind"),
        (
            json!({"project": "Harbor", "party": "Nobody"}),
            "unknown party",
        ),
        (
            json!({"project": "Harbor", "ended_on": "2026-09-30"}),
            "ended_on",
        ),
        (
            json!({"project": "Harbor", "status": "finished", "started_on": "2026-10-01", "ended_on": "2026-09-30"}),
            "before",
        ),
        (
            json!({"project": "Harbor", "notes": "SSN 123-45-6789"}),
            "TIN",
        ),
        (
            json!({"project": "Nowhere", "status": "paused"}),
            "unknown project",
        ),
        (json!({"project": "Harbor"}), "at least one"),
    ] {
        let why = m.refuse("books.update_project", args.clone());
        assert!(why.contains(needle), "{args}: {why}");
    }
    assert_eq!(m.count("project.set"), 0);
    let p = m.ok(
        "books.update_project",
        json!({"project": "Harbor", "status": "finished", "ended_on": "2026-09-30", "name": "Harbor app"}),
    );
    assert_eq!(p["status"], "finished");
    assert_eq!(p["name"], "Harbor app");
    // "" clears.
    let p = m.ok(
        "books.update_project",
        json!({"project": "Harbor app", "ended_on": "", "status": "active", "kind": "client"}),
    );
    assert_eq!(p["ended_on"], Value::Null);
    let p = m.ok(
        "books.update_project",
        json!({"project": "Harbor app", "kind": ""}),
    );
    assert_eq!(p["kind"], Value::Null);
}

#[test]
fn update_project_with_nothing_new_journals_nothing() {
    let mut m = books("update-same");
    m.ok(
        "books.add_project",
        json!({"name": "Harbor", "kind": "client"}),
    );
    let before = m.journal();
    m.ok(
        "books.update_project",
        json!({"project": "Harbor", "kind": "client"}),
    );
    m.ok(
        "books.update_project",
        json!({"project": "harbor", "name": "Harbor"}),
    );
    assert_eq!(m.journal(), before);
}

// ------------------------------------------------- figures

#[test]
fn project_figures_match_profit_and_loss_by_class() {
    let (mut m, _, _) = studio("by-class");
    let period = json!({"start": "2026-01-01", "end": "2026-12-31"});
    let by_class = m.read_only("books.profit_and_loss_by_class", period.clone());
    for name in ["Harbor app", "Zeta site"] {
        let mut a = period.clone();
        a["project"] = json!(name);
        let p = m.read_only("books.project", a);
        let row = by_class["classes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["class"] == name)
            .unwrap();
        assert_eq!(p["period"]["net"], row["net_income"], "{name}");
        assert_eq!(p["period"]["revenue"], row["revenue"], "{name}");
        assert_eq!(p["period"]["expenses"], row["expenses"], "{name}");
    }
    let z = m.read_only(
        "books.project",
        json!({"project": "Zeta site", "start": "2026-01-01", "end": "2026-12-31"}),
    );
    let accounts: Vec<(&str, &str, &str)> = z["period"]["by_account"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| {
            (
                a["code"].as_str().unwrap(),
                a["type"].as_str().unwrap(),
                a["amount"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        accounts,
        [
            ("4000", "revenue", "4500.00"),
            ("6000", "expense", "1800.00")
        ]
    );
    let march = &z["period"]["by_month"][2];
    assert_eq!(march["month"], "2026-03");
    assert_eq!(march["net"], "2700.00");
}

#[test]
fn project_to_date_counts_lines_before_the_period() {
    let (mut m, _, _) = studio("to-date");
    let z = m.read_only(
        "books.project",
        json!({"project": "Zeta site", "start": "2026-01-01", "end": "2026-12-31"}),
    );
    assert_eq!(z["period"]["revenue"], "4500.00");
    assert_eq!(z["to_date"]["revenue"], "5000.00");
    assert_eq!(z["to_date"]["through"], "2026-12-31");
    let list = m.read_only("books.projects", json!({}));
    let zeta = list["projects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "Zeta site")
        .unwrap()
        .clone();
    assert_eq!(zeta["to_date"]["net"], "3200.00");
}

/// The snapshot's figures (rules.md B-37 item 7), as the viewer shows
/// them: net is the contribution before overhead, margin_pct its share
/// of revenue, to_date.by_account the direct costs by category, and each
/// month's net_to_date the running net, the last being to_date's.
#[test]
fn project_snapshot_margin_by_account_and_running_net() {
    let (mut m, _, _) = studio("snapshot");
    let z = m.read_only(
        "books.project",
        json!({"project": "Zeta site", "start": "2026-01-01", "end": "2026-12-31"}),
    );
    // The period: 4,500 earned, 1,800 spent: 2,700, 60.0% of revenue.
    assert_eq!(z["period"]["net"], "2700.00");
    assert_eq!(z["period"]["margin_pct"], "60.0");
    // To date, the December deposit too: 3,200 of 5,000, 64.0%.
    assert_eq!(z["to_date"]["net"], "3200.00");
    assert_eq!(z["to_date"]["margin_pct"], "64.0");
    let accounts: Vec<(&str, &str, &str)> = z["to_date"]["by_account"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| {
            (
                a["code"].as_str().unwrap(),
                a["type"].as_str().unwrap(),
                a["amount"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        accounts,
        [
            ("4000", "revenue", "5000.00"),
            ("6000", "expense", "1800.00")
        ]
    );
    let running: Vec<&str> = z["period"]["by_month"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["net_to_date"].as_str().unwrap())
        .collect();
    assert_eq!(running.len(), 12);
    // January opens on the December deposit; March adds the milestone
    // less the contractor; the year ends on the net to date.
    assert_eq!(&running[..3], ["500.00", "500.00", "3200.00"]);
    assert_eq!(running[11], z["to_date"]["net"].as_str().unwrap());
    // On spec, repaid in part: a loss for the year, margin below zero.
    let h = m.read_only(
        "books.project",
        json!({"project": "Harbor app", "start": "2026-01-01", "end": "2026-12-31"}),
    );
    assert_eq!(h["period"]["net"], "-460.00");
    assert_eq!(h["period"]["margin_pct"], "-46.0");
    // No revenue yet: no margin.
    let h = m.read_only(
        "books.project",
        json!({"project": "Harbor app", "start": "2026-01-01", "end": "2026-04-30"}),
    );
    assert_eq!(h["to_date"]["margin_pct"], Value::Null);
    assert_eq!(h["period"]["margin_pct"], Value::Null);
    assert_eq!(
        h["to_date"]["by_account"][0]["amount"], "1460.00",
        "the devices, an asset, count nothing"
    );
    // Nothing tagged: books.project still carries to_date.by_account,
    // empty, and no margin.
    m.ok(
        "books.add_project",
        json!({"name": "Quiet one", "kind": "internal"}),
    );
    let q = m.read_only("books.project", json!({"project": "Quiet one"}));
    assert_eq!(q["to_date"]["by_account"], json!([]), "{q}");
    assert_eq!(q["period"]["by_account"], json!([]), "{q}");
    assert_eq!(q["to_date"]["margin_pct"], Value::Null);
    // The list carries the margin, not the accounts.
    let list = m.read_only("books.projects", json!({}));
    for p in list["projects"].as_array().unwrap() {
        assert!(p["to_date"].get("margin_pct").is_some(), "{p}");
        assert!(p["to_date"].get("by_account").is_none(), "{p}");
    }
}

#[test]
fn on_spec_project_shows_unrecovered_cost_and_others_null() {
    let (mut m, _, _) = studio("unrecovered");
    let h = m.read_only(
        "books.project",
        json!({"project": "Harbor app", "start": "2026-01-01", "end": "2026-04-30"}),
    );
    assert_eq!(
        h["to_date"]["expenses"], "1460.00",
        "the devices count nothing"
    );
    assert_eq!(h["to_date"]["unrecovered_cost"], "1460.00");
    let h = m.read_only(
        "books.project",
        json!({"project": "Harbor app", "start": "2026-01-01", "end": "2026-12-31"}),
    );
    assert_eq!(h["to_date"]["unrecovered_cost"], "460.00");
    let z = m.read_only(
        "books.project",
        json!({"project": "Zeta site", "start": "2026-01-01", "end": "2026-12-31"}),
    );
    assert_eq!(z["to_date"]["unrecovered_cost"], Value::Null);
    // Nothing receivable: the balance sheet has no receivable for it.
    let names: Vec<String> = m.read_only("books.projects", json!({"status": "active"}))["projects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(names, ["Harbor app", "Zeta site"]);
    assert!(
        m.read_only("books.projects", json!({"status": "paused"}))["projects"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    m.refuse("books.projects", json!({"status": "done"}));
}

#[test]
fn project_lists_entries_documents_decisions_and_party() {
    let (mut m, proto, _) = studio("detail");
    m.ok("books.add_party", json!({"name": "Harbor Co"}));
    m.ok(
        "books.update_project",
        json!({"project": "Harbor app", "party": "Harbor Co"}),
    );
    let doc = match m.doc(
        b"%PDF spec",
        json!([{"kind": "project", "id": "Harbor app"}]),
    ) {
        Out::Ok(v) => v["attachment_id"].as_u64().unwrap(),
        o => panic!("{o:?}"),
    };
    m.ok(
        "books.record_decision",
        json!({"subject": "Harbor", "decision_md": "Build it on spec.",
               "decided_by": ["Tobin Harlow"], "decided_on": "2026-04-01",
               "projects": ["Harbor app"]}),
    );
    let h = m.read_only(
        "books.project",
        json!({"project": "Harbor app", "start": "2026-01-01", "end": "2026-12-31"}),
    );
    assert_eq!(h["project"]["party"]["label"], "Harbor Co");
    let entries = h["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 4);
    assert_eq!(entries[0]["description"], "Harbor repays", "newest first");
    let p = entries.iter().find(|e| e["entry_id"] == proto).unwrap();
    assert_eq!(p["project_amount"], "-1400.00");
    assert_eq!(p["total"], "1400.00");
    let devices = entries
        .iter()
        .find(|e| e["description"] == "Devices")
        .unwrap();
    assert_eq!(devices["project_amount"], "0.00");
    assert_eq!(h["documents"][0]["attachment_id"], doc);
    assert_eq!(h["decisions"][0]["projects"][0]["name"], "Harbor app");
}

// ------------------------------------------------- links

#[test]
fn link_attachment_to_a_project_by_name_and_id() {
    let mut m = books("link");
    let id = m.ok("books.add_project", json!({"name": "Harbor"}))["id"]
        .as_u64()
        .unwrap();
    let a = match m.doc(b"%PDF a", json!([])) {
        Out::Ok(v) => v["attachment_id"].as_u64().unwrap(),
        o => panic!("{o:?}"),
    };
    let out = m.ok(
        "books.link_attachment",
        json!({"attachment_id": a, "subject": {"kind": "project", "id": "harbor"}}),
    );
    assert!(out.to_string().contains("project"), "{out}");
    m.refuse(
        "books.link_attachment",
        json!({"attachment_id": a, "subject": {"kind": "project", "id": id}}),
    );
    let files = m.read_only(
        "books.attachments",
        json!({"subject": {"kind": "project", "id": id}}),
    );
    assert_eq!(files["subject"]["label"], format!("project {id} Harbor"));
    assert_eq!(files["attachments"][0]["attachment_id"], a);
    m.refuse(
        "books.link_attachment",
        json!({"attachment_id": a, "subject": {"kind": "project", "id": 99}}),
    );
    m.ok(
        "books.unlink_attachment",
        json!({"attachment_id": a, "subject": {"kind": "project", "id": id}, "reason": "wrong"}),
    );
}

#[test]
fn file_document_with_a_bad_project_files_nothing() {
    let mut m = books("bad-link");
    m.ok("books.add_project", json!({"name": "Harbor"}));
    let out = m.doc(b"%PDF b", json!([{"kind": "project", "id": "Nowhere"}]));
    assert!(out.refused());
    assert_eq!(m.count("document.filed"), 0);
}

#[test]
fn record_decision_names_projects_and_decisions_filter_by_project() {
    let mut m = books("decisions");
    m.ok("books.add_project", json!({"name": "Harbor"}));
    m.ok("books.add_project", json!({"name": "Dock"}));
    let base =
        json!({"decision_md": "x", "decided_by": ["Tobin Harlow"], "decided_on": "2026-04-01"});
    let with = |subject: &str, projects: Value| {
        let mut a = base.clone();
        a["subject"] = json!(subject);
        a["projects"] = projects;
        a
    };
    m.ok("books.record_decision", with("One", json!(["Harbor"])));
    m.ok("books.record_decision", with("Two", json!([])));
    m.refuse("books.record_decision", with("Bad", json!(["Nowhere"])));
    m.refuse(
        "books.record_decision",
        with("Twice", json!(["Harbor", "harbor"])),
    );
    let all = m.read_only("books.decisions", json!({}));
    assert_eq!(all["decisions"].as_array().unwrap().len(), 2);
    assert!(
        all["decisions"][1].get("projects").is_none(),
        "none: left out"
    );
    let harbor = m.read_only("books.decisions", json!({"project": "Harbor"}));
    assert_eq!(harbor["decisions"].as_array().unwrap().len(), 1);
    assert_eq!(harbor["decisions"][0]["subject"], "One");
    assert!(
        m.read_only("books.decisions", json!({"project": "Dock"}))["decisions"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn party_lists_its_projects() {
    let mut m = books("party");
    m.ok("books.add_party", json!({"name": "Riley Chen"}));
    m.ok(
        "books.add_project",
        json!({"name": "Zeta", "party": "Riley Chen"}),
    );
    m.ok(
        "books.add_project",
        json!({"name": "alpha", "party": "Riley Chen"}),
    );
    m.ok("books.add_project", json!({"name": "Other"}));
    let p = m.read_only("books.party", json!({"party": "Riley Chen"}));
    let names: Vec<&str> = p["projects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["alpha", "Zeta"]);
}

#[test]
fn start_lists_active_and_paused_projects() {
    let mut m = books("start");
    m.ok(
        "books.add_project",
        json!({"name": "Zeta", "kind": "client"}),
    );
    m.ok(
        "books.add_project",
        json!({"name": "alpha", "status": "paused"}),
    );
    m.ok(
        "books.add_project",
        json!({"name": "Done", "status": "finished", "ended_on": "2026-01-31"}),
    );
    let s = m.read_only("books.start", json!({}));
    let got: Vec<(String, String)> = s["projects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            (
                p["name"].as_str().unwrap().to_string(),
                p["status"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert_eq!(
        got,
        [
            ("alpha".to_string(), "paused".to_string()),
            ("Zeta".into(), "active".into())
        ]
    );
    assert_eq!(s["projects"][1]["kind"], "client");
}

#[test]
fn show_project_sends_its_target() {
    let mut m = books("show");
    let id = m.ok("books.add_project", json!({"name": "Harbor"}))["id"]
        .as_u64()
        .unwrap();
    let s = m.read_only("books.show", json!({"project": "harbor"}));
    assert!(s["shown"].as_str().unwrap().contains("Harbor"), "{s}");
    let cmd = ledger_store::ops::read(&m.data).expect("request written");
    assert_eq!(
        cmd.target,
        ledger_store::ops::ViewerTarget::Project {
            class_id: id as u32
        }
    );
    m.read_only("books.show", json!({"page": "projects"}));
    m.refuse("books.show", json!({"project": "Nowhere"}));
    m.refuse(
        "books.show",
        json!({"project": "Harbor", "page": "projects"}),
    );
}

#[test]
fn project_writes_wait_for_the_format_upgrade() {
    let mut m = books_in("upgrade", true);
    // A name alone: a plain class, accepted.
    m.ok("books.add_project", json!({"name": "Harbor"}));
    let a = match m.doc(b"%PDF c", json!([])) {
        Out::Ok(v) => v["attachment_id"].as_u64().unwrap(),
        o => panic!("{o:?}"),
    };
    let head = m.journal();
    for (tool, args) in [
        (
            "books.add_project",
            json!({"name": "Dock", "kind": "client"}),
        ),
        (
            "books.update_project",
            json!({"project": "Harbor", "kind": "client"}),
        ),
        (
            "books.link_attachment",
            json!({"attachment_id": a, "subject": {"kind": "project", "id": "Harbor"}}),
        ),
        (
            "books.record_decision",
            json!({"subject": "s", "decision_md": "x", "decided_by": ["Tobin Harlow"],
                   "decided_on": "2026-04-01", "projects": ["Harbor"]}),
        ),
    ] {
        let why = m.refuse(tool, args);
        assert!(
            why.starts_with("format_upgrade_pending:") && why.contains("projects"),
            "{tool}: {why}"
        );
    }
    // Filing with a link to a project is refused before the file is filed.
    let out = m.doc(b"%PDF d", json!([{"kind": "project", "id": "Harbor"}]));
    assert!(out.refused());
    assert!(out.err_text().contains("projects"), "{out:?}");
    assert_eq!(m.journal(), head, "nothing journaled");
}

#[test]
fn add_line_notes_a_finished_project() {
    let mut m = books("late");
    m.ok(
        "books.add_project",
        json!({"name": "Harbor", "status": "finished", "ended_on": "2026-09-30"}),
    );
    m.ok("books.add_project", json!({"name": "Dock"}));
    let id = m.ok(
        "books.draft_entry",
        json!({"date": "2026-10-05", "description": "late cost", "no_party_reason": "test"}),
    )["entry_id"]
        .as_u64()
        .unwrap();
    let out = m.ok(
        "books.add_line",
        json!({"entry_id": id, "account": "6000", "side": "debit", "amount": "10.00", "class": "Harbor"}),
    );
    let note = out["note"].as_str().unwrap();
    assert!(
        note.contains("Harbor is finished (ended 2026-09-30)"),
        "{note}"
    );
    let out = m.ok(
        "books.add_line",
        json!({"entry_id": id, "account": "1000", "side": "credit", "amount": "10.00", "class": "Dock"}),
    );
    assert!(out.get("note").is_none());
}

#[test]
fn record_payout_notes_a_finished_project() {
    let mut m = books("late-payout");
    m.ok(
        "books.add_project",
        json!({"name": "Harbor", "status": "abandoned", "ended_on": "2026-09-30"}),
    );
    m.ok("books.add_project", json!({"name": "Dock"}));
    let out = m.ok(
        "books.record_payout",
        json!({"date": "2026-10-05", "description": "App Store payout",
               "deposit_account": "1000", "deposit": "70.00",
               "revenue": [{"account": "4000", "amount": "60.00", "class": "Harbor"},
                           {"account": "4000", "amount": "40.00", "class": "Harbor"},
                           {"account": "4000", "amount": "10.00", "class": "Dock"}],
               "fees": [{"account": "6000", "amount": "40.00", "class": "Harbor"}]}),
    );
    let notes = out["notes"].as_array().unwrap();
    assert_eq!(notes.len(), 1, "one note per project: {notes:?}");
    assert!(
        notes[0]
            .as_str()
            .unwrap()
            .contains("Harbor is abandoned (ended 2026-09-30)"),
        "{notes:?}"
    );
    // No project over: no notes.
    let out = m.ok(
        "books.record_payout",
        json!({"date": "2026-10-06", "description": "App Store payout",
               "deposit_account": "1000", "deposit": "10.00",
               "revenue": [{"account": "4000", "amount": "10.00", "class": "Dock"}]}),
    );
    assert!(out.get("notes").is_none(), "{out}");
}

// ------------------------------------------------- names

#[test]
fn a_name_resolves_as_the_duplicate_check_compares() {
    let mut m = books("fold");
    m.ok("books.add_project", json!({"name": "Éclair"}));
    let why = m.refuse("books.add_project", json!({"name": "éCLAIR"}));
    assert!(why.contains("already exists"), "{why}");
    for name in ["éclair", "ÉCLAIR", " Éclair ", " 1 "] {
        let p = m.read_only("books.project", json!({"project": name}));
        assert_eq!(p["project"]["name"], "Éclair", "{name:?}");
    }
    let p = m.ok(
        "books.update_project",
        json!({"project": " éclair ", "status": "paused"}),
    );
    assert_eq!(
        (p["name"].clone(), p["status"].clone()),
        (json!("Éclair"), json!("paused"))
    );
    let why = m.refuse("books.add_project", json!({"name": "42"}));
    assert!(why.contains("digits only"), "{why}");
}

/// Books an earlier version wrote with these class names (replay never
/// checks names): no uniqueness, digits allowed.
fn old_classes(name: &str, names: &[&str]) -> Mcp {
    let dir = test_dir(name);
    let books = dir.books_data();
    drop(ledger_store::Store::create(&books, ledger_core::Currency::USD).unwrap());
    let mut w = ledger_store::JournalWriter::open(&books.join(ledger_store::JOURNAL_FILE)).unwrap();
    for (i, n) in names.iter().enumerate() {
        w.append("class.added", json!({"id": i + 1, "name": n}))
            .unwrap();
    }
    drop(w);
    let toml = books_toml(&dir, "Spec Co", "");
    let mut m = Mcp::start(&dir, &toml);
    m.accounts(CHART);
    m.books_dir = Some(dir);
    m
}

#[test]
fn old_books_duplicates_take_fact_only_changes() {
    let mut m = old_classes("old-dup", &["Alpha", "alpha", "Harbor"]);
    // The lowest id answers the shared name; each changes by id.
    assert_eq!(
        m.read_only("books.project", json!({"project": "ALPHA"}))["project"]["id"],
        1
    );
    for id in ["1", "2"] {
        let p = m.ok(
            "books.update_project",
            json!({"project": id, "status": "paused"}),
        );
        assert_eq!(p["status"], "paused");
    }
    let why = m.refuse(
        "books.update_project",
        json!({"project": "2", "name": "harbor"}),
    );
    assert!(why.contains("already exists"), "{why}");
}

#[test]
fn a_digit_name_that_shadows_an_id_says_what_to_pass() {
    let mut m = old_classes("old-digits", &["Éclair", "Dock", "1"]);
    let why = m.refuse("books.project", json!({"project": "1"}));
    assert!(
        why.contains("pass 3 for project 3") && why.contains("\"Éclair\" (project 1's name)"),
        "{why}"
    );
    assert_eq!(
        m.read_only("books.project", json!({"project": "3"}))["project"]["name"],
        "1"
    );
    assert_eq!(
        m.read_only("books.project", json!({"project": "éclair"}))["project"]["id"],
        1
    );
    // A JSON number in a subject is an id, never a name.
    let a = match m.doc(b"%PDF digits", json!([])) {
        Out::Ok(v) => v["attachment_id"].as_u64().unwrap(),
        o => panic!("{o:?}"),
    };
    m.ok(
        "books.link_attachment",
        json!({"attachment_id": a, "subject": {"kind": "project", "id": 1}}),
    );
    let files = m.read_only(
        "books.attachments",
        json!({"subject": {"kind": "project", "id": 1}}),
    );
    assert_eq!(files["subject"]["label"], "project 1 Éclair");
}

// ------------------------------------------------- invoices and bills

/// [`books`] as the live books stand until their owner consents again:
/// this version's format without `document_line_projects` (they declared
/// `projects`).
fn books_before_document_projects(name: &str) -> Mcp {
    let dir = test_dir(name);
    let mut format = ledger_store::format::current();
    format
        .features
        .retain(|f| f != ledger_store::format::DOCUMENT_LINE_PROJECTS);
    ledger_store::Store::create_in_format(&dir.books_data(), ledger_core::Currency::USD, &format)
        .unwrap();
    let toml = books_toml(&dir, "Spec Co", "");
    let mut m = Mcp::start(&dir, &toml);
    m.accounts(CHART);
    m.books_dir = Some(dir);
    m
}

impl Mcp {
    /// Each line of the entry as books.get_entry shows it: (code, side,
    /// amount, project).
    fn entry_lines(&mut self, entry: u64) -> Vec<(String, String, String, Option<String>)> {
        self.read_only("books.get_entry", json!({"entry_id": entry}))["lines"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| {
                (
                    l["code"].as_str().unwrap().to_string(),
                    l["side"].as_str().unwrap().to_string(),
                    l["amount"].as_str().unwrap().to_string(),
                    l["project"].as_str().map(str::to_string),
                )
            })
            .collect()
    }

    /// Each line's `class_id` as books.get_entry shows it (absent: None).
    fn entry_class_ids(&mut self, entry: u64) -> Vec<Option<u64>> {
        let lines = self.read_only("books.get_entry", json!({"entry_id": entry}))["lines"].clone();
        lines
            .as_array()
            .unwrap()
            .iter()
            .map(|l| {
                assert!(
                    l.get("class_id").is_none_or(|c| !c.is_null()),
                    "an untagged line omits class_id: {l}"
                );
                l["class_id"].as_u64()
            })
            .collect()
    }

    /// The projects handbook section served as a resource.
    fn handbook_resource(&mut self) -> String {
        let r = self.rpc(
            "resources/read",
            json!({"uri": "books://handbook/projects"}),
        );
        r["result"]["contents"][0]["text"]
            .as_str()
            .unwrap_or_else(|| panic!("{r}"))
            .to_string()
    }
}

fn l4(
    code: &str,
    side: &str,
    amount: &str,
    project: Option<&str>,
) -> (String, String, String, Option<String>) {
    (
        code.into(),
        side.into(),
        amount.into(),
        project.map(str::to_string),
    )
}

/// rules.md B-37 item 8: each invoice and bill line takes a project (by
/// name or id); the entry's revenue or expense side is one line per
/// project, the untagged lines' as one, the receivable or payable line
/// and the payments untagged; books.project, profit_and_loss_by_class,
/// the 1099 worksheet and the party's totals all read them.
#[test]
fn invoice_and_bill_lines_count_toward_their_projects() {
    let mut m = books("doc-lines");
    let zeta = m.ok(
        "books.add_project",
        json!({"name": "Zeta site", "kind": "client"}),
    )["id"]
        .as_u64()
        .unwrap();
    let harbor = m.ok(
        "books.add_project",
        json!({"name": "Harbor app", "kind": "client"}),
    )["id"]
        .as_u64()
        .unwrap();
    let cedar = m.ok("books.add_customer", json!({"name": "Cedar Client"}))["id"].clone();
    let dana = m.ok("books.add_vendor", json!({"name": "Dana Illustration"}));
    let (dana_vendor, dana_party) = (dana["id"].clone(), dana["party_id"].clone());
    m.ok(
        "books.set_vendor_1099",
        json!({"vendor": "Dana Illustration", "form_1099": "nec", "decided_by": "Tobin Harlow", "decided_on": "2026-01-01"}),
    );
    let inv = m.ok(
        "books.create_invoice",
        json!({"party_id": cedar, "date": "2026-03-02", "due_date": "2026-04-01", "lines": [
            {"description": "Design", "amount": "3000.00", "class": "Zeta site"},
            {"description": "Hosting", "amount": "200.00", "class": harbor.to_string()},
            {"description": "Rush fee", "amount": "100.00"},
            {"description": "Revisions", "amount": "500.00", "class": " zeta SITE "},
        ]}),
    );
    assert_eq!(inv["total"], "3800.00");
    assert!(inv.get("notes").is_none(), "{inv}");
    let inv_entry = inv["entry_id"].as_u64().unwrap();
    assert_eq!(
        m.entry_lines(inv_entry),
        vec![
            l4("1200", "debit", "3800.00", None),
            l4("4000", "credit", "3500.00", Some("Zeta site")),
            l4("4000", "credit", "200.00", Some("Harbor app")),
            l4("4000", "credit", "100.00", None),
        ]
    );
    // books.get_entry's class_id is the project's id on a tagged line and
    // absent on the others.
    assert_eq!(
        m.entry_class_ids(inv_entry),
        vec![None, Some(zeta), Some(harbor), None]
    );
    let bill = m.ok(
        "books.create_bill",
        json!({"party_id": dana_vendor, "date": "2026-03-09", "due_date": "2026-04-08", "lines": [
            {"description": "Illustration", "amount": "1200.00", "class": "Zeta site"},
            {"description": "Postage", "amount": "50.00"},
        ]}),
    );
    let bill_entry = bill["entry_id"].as_u64().unwrap();
    assert_eq!(
        m.entry_lines(bill_entry),
        vec![
            l4("6999", "debit", "1200.00", Some("Zeta site")),
            l4("6999", "debit", "50.00", None),
            l4("2000", "credit", "1250.00", None),
        ]
    );
    assert_eq!(m.entry_class_ids(bill_entry), vec![Some(zeta), None, None]);
    // Payments are balance-sheet only: no project.
    let got = m.ok(
        "books.receive_payment",
        json!({"document_id": inv["id"], "date": "2026-03-20", "amount": "3800.00", "cash_account": "1000"}),
    );
    let paid = m.ok(
        "books.pay_bill",
        json!({"document_id": bill["id"], "date": "2026-03-30", "amount": "1250.00", "cash_account": "1000"}),
    );
    for e in [&got["entry_id"], &paid["entry_id"]] {
        let e = e.as_u64().unwrap();
        assert!(m.entry_lines(e).iter().all(|l| l.3.is_none()), "entry {e}");
    }
    // The project's figures, and the class P&L, to the cent.
    let year = json!({"start": "2026-01-01", "end": "2026-12-31"});
    let mut a = year.clone();
    a["project"] = json!("Zeta site");
    let z = m.read_only("books.project", a);
    assert_eq!(
        (
            &z["period"]["revenue"],
            &z["period"]["expenses"],
            &z["period"]["net"]
        ),
        (&json!("3500.00"), &json!("1200.00"), &json!("2300.00"))
    );
    assert_eq!(z["to_date"]["net"], "2300.00");
    assert_eq!(z["period"]["by_month"][2]["net"], "2300.00");
    let entries: Vec<u64> = z["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["entry_id"].as_u64().unwrap())
        .collect();
    assert_eq!(entries, vec![bill_entry, inv_entry]);
    let by_class = m.read_only("books.profit_and_loss_by_class", year.clone());
    let row = |name: &str| {
        by_class["classes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["class"] == name)
            .unwrap_or_else(|| panic!("{name}: {by_class}"))
            .clone()
    };
    assert_eq!(row("Zeta site")["net_income"], "2300.00");
    assert_eq!(row("Harbor app")["net_income"], "200.00");
    // The 1099 worksheet and the party's totals follow the money: the
    // bill's payment, 1,250.00, whatever its lines' projects.
    let w = m.read_only("books.vendor_1099", json!({"year": 2026}));
    let dana_line = w["vendors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == "Dana Illustration")
        .unwrap()
        .clone();
    assert_eq!(dana_line["paid"], "1250.00");
    let p = m.read_only(
        "books.party",
        json!({"party": dana_party.to_string(), "year": 2026}),
    );
    assert_eq!(p["totals"]["paid"], "1250.00", "{p}");
    assert_eq!(p["totals"]["amount_1099"], "1250.00", "{p}");
}

#[test]
fn a_document_naming_an_unknown_project_is_refused_whole() {
    let mut m = books("doc-unknown");
    m.ok("books.add_project", json!({"name": "Zeta site"}));
    let cedar = m.ok("books.add_customer", json!({"name": "Cedar"}))["id"].clone();
    let v = m.ok("books.add_vendor", json!({"name": "Dana"}))["id"].clone();
    for (tool, party) in [("books.create_invoice", &cedar), ("books.create_bill", &v)] {
        let why = m.refuse(
            tool,
            json!({"party_id": party, "date": "2026-03-02", "due_date": "2026-04-01", "lines": [
                {"description": "a", "amount": "10.00", "class": "Zeta site"},
                {"description": "b", "amount": "10.00", "class": "Nowhere"},
            ]}),
        );
        assert!(why.contains("unknown project \"Nowhere\""), "{tool}: {why}");
    }
}

#[test]
fn a_document_for_a_finished_project_says_so() {
    let mut m = books("doc-late");
    m.ok(
        "books.add_project",
        json!({"name": "Harbor", "status": "finished", "ended_on": "2026-09-30"}),
    );
    m.ok(
        "books.add_project",
        json!({"name": "Dock", "status": "abandoned"}),
    );
    let cedar = m.ok("books.add_customer", json!({"name": "Cedar"}))["id"].clone();
    let out = m.ok(
        "books.create_invoice",
        json!({"party_id": cedar, "date": "2026-10-05", "due_date": "2026-11-04", "lines": [
            {"description": "Final", "amount": "10.00", "class": "Harbor"},
            {"description": "Again", "amount": "10.00", "class": "harbor"},
            {"description": "Old", "amount": "10.00", "class": "Dock"},
        ]}),
    );
    let notes: Vec<&str> = out["notes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_str().unwrap())
        .collect();
    assert_eq!(notes.len(), 2, "{notes:?}");
    assert!(
        notes[0].contains("Harbor is finished (ended 2026-09-30)"),
        "{notes:?}"
    );
    assert!(notes[1].contains("Dock is abandoned"), "{notes:?}");
    // Tagged anyway.
    let e = out["entry_id"].as_u64().unwrap();
    assert_eq!(
        m.entry_lines(e)[1],
        l4("4000", "credit", "20.00", Some("Harbor"))
    );
}

/// B-37 items 4 and 8: an invoice line naming an on-spec project is taken
/// and its result asks whether there is now an agreement (a hoped-for
/// payment is no receivable); a bill's cost for one is an expense like any
/// other, so a bill gets no such note.
#[test]
fn an_invoice_for_an_on_spec_project_asks_about_an_agreement() {
    let mut m = books("doc-on-spec");
    m.ok(
        "books.add_project",
        json!({"name": "Kestrel", "kind": "on_spec"}),
    );
    m.ok(
        "books.add_project",
        json!({"name": "Zeta", "kind": "client"}),
    );
    let cedar = m.ok("books.add_customer", json!({"name": "Cedar"}))["id"].clone();
    let v = m.ok("books.add_vendor", json!({"name": "Dana"}))["id"].clone();
    let out = m.ok(
        "books.create_invoice",
        json!({"party_id": cedar, "date": "2026-10-05", "due_date": "2026-11-04", "lines": [
            {"description": "Build", "amount": "10.00", "class": "Kestrel"},
            {"description": "More", "amount": "5.00", "class": "kestrel"},
            {"description": "Site", "amount": "7.00", "class": "Zeta"},
        ]}),
    );
    let notes = out["notes"].as_array().unwrap();
    assert_eq!(notes.len(), 1, "{out}");
    let note = notes[0].as_str().unwrap();
    assert!(
        note.contains("project Kestrel is on spec") && note.contains("is there now an agreement"),
        "{note}"
    );
    let e = out["entry_id"].as_u64().unwrap();
    assert_eq!(
        m.entry_lines(e)[1],
        l4("4000", "credit", "15.00", Some("Kestrel"))
    );
    let bill = m.ok(
        "books.create_bill",
        json!({"party_id": v, "date": "2026-10-05", "due_date": "2026-11-04", "lines": [
            {"description": "Icons", "amount": "10.00", "class": "Kestrel"},
        ]}),
    );
    assert!(bill.get("notes").is_none(), "{bill}");
}

/// D-39: books that declared `projects` but not
/// `document_line_projects` refuse a document naming a project, journaling
/// nothing, and take one naming none; the handbook tells them so.
#[test]
fn books_without_the_feature_refuse_a_tagged_document() {
    let mut m = books_before_document_projects("doc-gate");
    m.ok(
        "books.add_project",
        json!({"name": "Zeta site", "kind": "client"}),
    );
    let cedar = m.ok("books.add_customer", json!({"name": "Cedar"}))["id"].clone();
    let v = m.ok("books.add_vendor", json!({"name": "Dana"}))["id"].clone();
    let head = m.journal();
    for (tool, party) in [("books.create_invoice", &cedar), ("books.create_bill", &v)] {
        let why = m.refuse(
            tool,
            json!({"party_id": party, "date": "2026-03-02", "due_date": "2026-04-01", "lines": [
                {"description": "a", "amount": "10.00", "class": "Zeta site"},
            ]}),
        );
        assert!(
            why.starts_with("format_upgrade_pending:") && why.contains("document_line_projects"),
            "{tool}: {why}"
        );
    }
    assert_eq!(m.journal(), head, "nothing journaled");
    m.ok(
        "books.create_invoice",
        json!({"party_id": cedar, "date": "2026-03-02", "due_date": "2026-04-01", "lines": [
            {"description": "a", "amount": "10.00"},
        ]}),
    );
    let text = m.read_only("books.handbook", json!({"section": "projects"}))["section"]["text"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        text.contains("takes no project in these books until their format upgrade"),
        "{text}"
    );
    // The same section read as a resource tells the same books the same.
    let served = m.handbook_resource();
    assert!(
        served.contains("takes no project in these books until their format upgrade"),
        "{served}"
    );
    assert!(!served.contains("take class on each line"), "{served}");
    let mut fresh = books("doc-gate-fresh");
    let served = fresh.handbook_resource();
    assert!(served.contains("take class on each line"), "{served}");
    assert!(!served.contains("takes no project"), "{served}");
    let text = fresh.read_only("books.handbook", json!({"section": "projects"}))["section"]["text"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(text.contains("take class on each line"), "{text}");
    assert!(!text.contains("takes no project"), "{text}");
}
