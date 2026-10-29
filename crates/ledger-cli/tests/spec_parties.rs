//! Spec-blind tests for the party directory tools of the books server,
//! driven through a real `wanderbooks server` as an agent reaches it
//! (common.rs).
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
                "clientInfo": {"name": "spec-parties", "version": "0.1.0"},
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

    fn party(&mut self, party: &str, year: i32) -> Value {
        self.read_only("books.party", json!({"party": party, "year": year}))
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
    ("1300", "Prepaid Expenses", "asset"),
    ("2050", "Card", "liability"),
    ("2100", "Due to Members", "liability"),
    ("3000", "Capital", "equity"),
    ("4000", "Revenue", "revenue"),
    ("6000", "Contract Labor", "expense"),
    ("6999", "General Expenses", "expense"),
];

/// Books with the chart; cash is 1000.
fn books(name: &str) -> Mcp {
    let dir = test_dir(name);
    let toml = books_toml(&dir, "Spec Co", "");
    let mut m = Mcp::start(&dir, &toml);
    m.accounts(CHART);
    m.books_dir = Some(dir);
    m
}

const W9: &str = r#"{"received_on": "2026-01-05", "entity_type": "individual/sole proprietor",
    "tin_last4": "6789", "kept_at": "CPA portal"}"#;

fn w9(party: &str) -> Value {
    let mut v: Value = serde_json::from_str(W9).unwrap();
    v["party"] = json!(party);
    v
}

fn roles(v: &Value) -> Vec<String> {
    v["roles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r.as_str().unwrap().to_string())
        .collect()
}

/// Pat's payroll line, naming the employee by `employee` (and no name).
fn pat_line(employee: &str) -> Value {
    json!({"employee": employee, "gross_pay": "1000.00", "federal_withholding": "100.00",
           "state_withholding": "50.00", "ss_employee": "62.00", "medicare_employee": "14.50",
           "ss_employer": "62.00", "medicare_employer": "14.50", "other_deductions": "0.00"})
}

// ------------------------------------------------- books.add_party

#[test]
fn add_party_returns_the_party_and_journals_party_added() {
    let mut m = books("add");
    let p = m.ok(
        "books.add_party",
        json!({"name": "  Acme Design  ", "roles": ["contractor", "landlord", "contractor"]}),
    );
    assert_eq!(p["name"], "Acme Design", "the name is trimmed");
    assert_eq!(p["label"], "Acme Design");
    assert_eq!(roles(&p), ["contractor", "landlord"]);
    assert_eq!(p["archived"], false);
    assert!(p["w9"].is_null() && p["w4"].is_null());
    assert!(p["vendor_id"].is_null() && p["customer_id"].is_null());
    assert_eq!(m.count("party.added"), 1);
    // No roles is fine.
    let q = m.ok("books.add_party", json!({"name": "IRS"}));
    assert!(roles(&q).is_empty());
}

#[test]
fn add_party_refusals_change_nothing() {
    let mut m = books("add-refusals");
    m.add_party("Acme Design", &[]);
    for args in [
        json!({"name": "acme design "}),    // duplicate, case and space
        json!({"name": "   "}),             // blank
        json!({"name": "Bob 123-45-6789"}), // a full SSN
        json!({"name": "Bob 123456789"}),   // a full TIN, no separators
        json!({"name": "Shop", "roles": ["vendor"]}), // vendor by hand
        json!({"name": "Shop", "roles": ["customer"]}), // customer by hand
        json!({"name": "Shop", "roles": ["wizard"]}), // unknown role
        json!({"name": "x".repeat(501)}),   // too long
    ] {
        m.refuse("books.add_party", args);
    }
    assert_eq!(m.count("party.added"), 1);
}

// ------------------------------------------------- resolution

#[test]
fn a_party_resolves_by_id_or_by_name_and_refuses_ambiguity() {
    let mut m = books("resolve");
    let acme = m.add_party("Acme Design", &[]);
    let by_name = m.party("  ACME design ", 2026);
    assert_eq!(by_name["party"]["party_id"], json!(acme));
    let by_id = m.party(&acme.to_string(), 2026);
    assert_eq!(by_id["party"]["party_id"], json!(acme));
    // A party named like another's id: the number is ambiguous.
    let named_1 = m.add_party(&acme.to_string(), &[]);
    assert_ne!(named_1, acme);
    let why = m.refuse("books.party", json!({"party": acme.to_string()}));
    assert!(why.contains("ambiguous"), "{why}");
    // The other party's id is unambiguous.
    let p = m.party(&named_1.to_string(), 2026);
    assert_eq!(p["party"]["party_id"], json!(named_1));
    m.refuse("books.party", json!({"party": "Nobody"}));
    m.refuse("books.party", json!({"party": "999"}));
}

#[test]
fn an_archived_party_goes_by_id_and_frees_its_name() {
    let mut m = books("archive");
    let old = m.add_party("Acme Design", &[]);
    let p = m.ok(
        "books.update_party",
        json!({"party": "Acme Design", "archived": true}),
    );
    assert_eq!(p["archived"], true);
    m.refuse("books.party", json!({"party": "Acme Design"}));
    m.party(&old.to_string(), 2026);
    // The name is free for a new party.
    let new = m.add_party("Acme Design", &[]);
    assert_ne!(new, old);
    assert_eq!(
        m.party("acme design", 2026)["party"]["party_id"],
        json!(new)
    );
    // Un-archiving the old one is refused while the name is taken.
    m.refuse(
        "books.update_party",
        json!({"party": old.to_string(), "archived": false}),
    );
    // The list hides archived parties unless asked.
    let listed = m.read_only("books.parties", json!({"year": 2026}));
    let ids: Vec<&Value> = listed["parties"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| &p["party_id"])
        .collect();
    assert_eq!(ids, [&json!(new)]);
    let all = m.read_only(
        "books.parties",
        json!({"year": 2026, "include_archived": true}),
    );
    assert_eq!(all["parties"].as_array().unwrap().len(), 2);
}

// ------------------------------------------------- books.update_party

#[test]
fn update_party_journals_each_field_and_empty_clears() {
    let mut m = books("update");
    m.add_party("Acme Design", &[]);
    let p = m.ok(
        "books.update_party",
        json!({"party": "Acme Design", "name": "Acme Design LLC", "display_name": "Acme",
               "website": "https://acme.example", "address": "1 Main St, Midvale, UT",
               "notes": "logo work"}),
    );
    assert_eq!(m.count("party.updated"), 5, "one event per field");
    assert_eq!(p["name"], "Acme Design LLC");
    assert_eq!(p["label"], "Acme", "the display name is the label");
    assert_eq!(p["website"], "https://acme.example");
    assert_eq!(p["address"], "1 Main St, Midvale, UT");
    assert_eq!(p["notes"], "logo work");
    // "" clears an optional field.
    let p = m.ok(
        "books.update_party",
        json!({"party": "Acme Design LLC", "display_name": "", "notes": ""}),
    );
    assert!(p["display_name"].is_null() && p["notes"].is_null());
    assert_eq!(p["label"], "Acme Design LLC");
    assert_eq!(m.count("party.updated"), 7);
}

#[test]
fn update_party_refusals_change_nothing() {
    let mut m = books("update-refusals");
    m.add_party("Acme Design", &[]);
    m.add_party("Other", &[]);
    // A bad field refuses the whole call, the good field included.
    m.refuse(
        "books.update_party",
        json!({"party": "Acme Design", "website": "https://acme.example",
               "notes": "EIN 12-3456789"}),
    );
    m.refuse(
        "books.update_party",
        json!({"party": "Acme Design", "website": "https://acme.example", "name": "other"}),
    );
    m.refuse(
        "books.update_party",
        json!({"party": "Acme Design", "website": "https://acme.example", "name": " "}),
    );
    m.refuse(
        "books.update_party",
        json!({"party": "Acme Design", "address": "a".repeat(501)}),
    );
    m.refuse("books.update_party", json!({"party": "Acme Design"}));
    m.refuse(
        "books.update_party",
        json!({"party": "Nobody", "notes": "x"}),
    );
    let p = m.party("Acme Design", 2026);
    assert!(p["party"]["website"].is_null(), "nothing was written");
    assert_eq!(m.count("party.updated"), 0);
}

// ------------------------------------------------- books.set_party_roles

#[test]
fn set_party_roles_adds_then_removes_and_skips_no_ops() {
    let mut m = books("roles");
    m.add_party("Pat", &["contractor"]);
    let p = m.ok(
        "books.set_party_roles",
        json!({"party": "Pat", "add": ["employee", "contractor"], "remove": ["member"]}),
    );
    assert_eq!(roles(&p), ["contractor", "employee"]);
    assert_eq!(m.count("party.role_set"), 1, "only the real change");
    // W-9 on file: swapping one 1099 role for the other works (adds first).
    m.ok("books.set_w9", w9("Pat"));
    let p = m.ok(
        "books.set_party_roles",
        json!({"party": "Pat", "add": ["landlord"], "remove": ["contractor"]}),
    );
    assert_eq!(roles(&p), ["employee", "landlord"]);
}

#[test]
fn set_party_roles_refusals_change_nothing() {
    let mut m = books("roles-refusals");
    m.add_party("Pat", &["contractor", "employee"]);
    m.ok("books.set_w9", w9("Pat"));
    m.ok(
        "books.set_w4",
        json!({"party": "Pat", "received_on": "2026-01-02", "state": "UT", "kept_at": "binder"}),
    );
    let before = m.count("party.role_set");
    for args in [
        json!({"party": "Pat", "add": ["vendor"]}),
        json!({"party": "Pat", "remove": ["customer"]}),
        json!({"party": "Pat", "add": ["wizard"]}),
        json!({"party": "Pat"}),
        json!({"party": "Pat", "add": ["member"], "remove": ["member"]}),
        // The last 1099 role, with W-9 facts on file (the add goes too).
        json!({"party": "Pat", "add": ["member"], "remove": ["contractor"]}),
        // Employee, with W-4 facts on file.
        json!({"party": "Pat", "remove": ["employee"]}),
        json!({"party": "Nobody", "add": ["member"]}),
    ] {
        m.refuse("books.set_party_roles", args);
    }
    assert_eq!(m.count("party.role_set"), before);
    // Clearing the facts first makes the removals work.
    m.ok("books.set_w9", json!({"party": "Pat", "clear": true}));
    m.ok("books.set_w4", json!({"party": "Pat", "clear": true}));
    let p = m.ok(
        "books.set_party_roles",
        json!({"party": "Pat", "remove": ["contractor", "employee"]}),
    );
    assert!(roles(&p).is_empty());
}

// ------------------------------------------------- books.set_w9 / books.set_w4

#[test]
fn set_w9_records_the_facts_and_marks_the_vendor() {
    let mut m = books("w9");
    m.add_party("Acme Design", &["contractor"]);
    let v = m.ok("books.add_vendor", json!({"name": "Acme Design"}));
    let mut args = w9("Acme Design");
    args["exempt_payee_code"] = json!("5");
    args["fatca_code"] = json!("");
    let p = m.ok("books.set_w9", args);
    let f = &p["w9"];
    assert_eq!(f["received_on"], "2026-01-05");
    assert_eq!(f["entity_type"], "individual/sole proprietor");
    assert_eq!(f["exempt_payee_code"], "5");
    assert!(f["fatca_code"].is_null(), "an empty code is none");
    assert_eq!(f["tin_last4"], "6789");
    assert_eq!(f["kept_at"], "CPA portal");
    assert_eq!(m.count("party.w9_set"), 1);
    // The vendor record's W-9 flag follows.
    let vendors = m.read_only("books.list_vendors", json!({}));
    let vendor = &vendors["vendors"][0];
    assert_eq!(vendor["id"], v["id"]);
    assert_eq!(vendor["tin_on_file"], true);
    assert_eq!(vendor["party_id"], p["party_id"]);
    let listed = m.read_only("books.parties", json!({"role": "contractor", "year": 2026}));
    assert_eq!(listed["parties"][0]["w9_on_file"], true);
    assert_eq!(listed["parties"][0]["tin_last4"], "6789");
    // Clearing clears both.
    let p = m.ok(
        "books.set_w9",
        json!({"party": "Acme Design", "clear": true}),
    );
    assert!(p["w9"].is_null());
    let vendors = m.read_only("books.list_vendors", json!({}));
    assert_eq!(vendors["vendors"][0]["tin_on_file"], false);
}

#[test]
fn set_w9_refusals_change_nothing() {
    let mut m = books("w9-refusals");
    m.add_party("Acme Design", &["contractor"]);
    m.add_party("Insurer", &[]);
    m.refuse("books.set_w9", w9("Insurer")); // no 1099 role
    for (field, value) in [
        ("tin_last4", "123456789"),
        ("tin_last4", "12-3456789"),
        ("tin_last4", "123"),
        ("tin_last4", "12a4"),
        ("entity_type", "individual 123 45 6789"),
        ("kept_at", "SSN 123-45-6789"),
        ("exempt_payee_code", "123456789"),
        ("fatca_code", "987-81-4321"),
        ("entity_type", " "),
        ("kept_at", ""),
        ("received_on", "2026-13-01"),
    ] {
        let mut args = w9("Acme Design");
        args[field] = json!(value);
        m.refuse("books.set_w9", args);
    }
    for missing in ["received_on", "entity_type", "tin_last4", "kept_at"] {
        let mut args = w9("Acme Design");
        args.as_object_mut().unwrap().remove(missing);
        m.refuse("books.set_w9", args);
    }
    let mut args = w9("Acme Design");
    args["clear"] = json!(true);
    m.refuse("books.set_w9", args);
    m.refuse("books.set_w9", w9("Nobody"));
    assert_eq!(m.count("party.w9_set"), 0);
}

#[test]
fn set_w4_records_and_clears_and_refuses_change_nothing() {
    let mut m = books("w4");
    m.add_party("Pat", &["employee"]);
    m.add_party("Acme Design", &["contractor"]);
    let good = json!({"party": "Pat", "received_on": "2026-01-02", "state": "UT",
                      "kept_at": "payroll binder"});
    for (field, value) in [
        ("party", "Acme Design"), // not an employee
        ("state", "ut"),
        ("state", "Utah"),
        ("kept_at", " "),
        ("kept_at", "SSN 123 45 6789"),
        ("received_on", "January"),
    ] {
        let mut args = good.clone();
        args[field] = json!(value);
        m.refuse("books.set_w4", args);
    }
    let mut args = good.clone();
    args["clear"] = json!(true);
    m.refuse("books.set_w4", args);
    assert_eq!(m.count("party.w4_set"), 0);
    let p = m.ok("books.set_w4", good);
    assert_eq!(p["w4"]["state"], "UT");
    assert_eq!(p["w4"]["received_on"], "2026-01-02");
    assert_eq!(p["w4"]["kept_at"], "payroll binder");
    let listed = m.read_only("books.parties", json!({"role": "employee", "year": 2026}));
    assert_eq!(listed["parties"][0]["w4_on_file"], true);
    let p = m.ok("books.set_w4", json!({"party": "Pat", "clear": true}));
    assert!(p["w4"].is_null());
    assert_eq!(m.count("party.w4_set"), 2);
}

// ------------------------------------------------- vendor and customer records

#[test]
fn add_vendor_and_add_customer_link_a_party() {
    let mut m = books("records");
    let acme = m.add_party("Acme Design", &["contractor"]);
    let v = m.ok("books.add_vendor", json!({"name": "ACME DESIGN"}));
    assert_eq!(v["party_id"], json!(acme), "links the party of that name");
    let c = m.ok("books.add_customer", json!({"name": "acme design"}));
    assert_eq!(
        c["party_id"],
        json!(acme),
        "the same party, as a customer too"
    );
    let p = m.party("Acme Design", 2026);
    assert_eq!(
        roles(&p["party"]),
        ["vendor", "contractor", "customer"],
        "vendor and customer follow the records"
    );
    assert_eq!(p["party"]["vendor_id"], v["id"]);
    assert_eq!(p["party"]["customer_id"], c["id"]);
    // A new name makes a new party.
    let w = m.ok("books.add_vendor", json!({"name": "Hosting Co"}));
    assert_ne!(w["party_id"], json!(acme));
    let listed = m.read_only("books.parties", json!({"role": "vendor", "year": 2026}));
    assert_eq!(listed["parties"].as_array().unwrap().len(), 2);
    let vendors = m.read_only("books.list_vendors", json!({}));
    for v in vendors["vendors"].as_array().unwrap() {
        assert!(v["party_id"].is_u64(), "{v}");
    }
}

// ------------------------------------------------- tagging entries

#[test]
fn set_entry_party_tags_the_vendor_so_vendor_1099_counts_it() {
    let mut m = books("tag-1099");
    m.add_party("Landlord LLC", &["landlord"]);
    m.ok("books.add_vendor", json!({"name": "Landlord LLC"}));
    m.ok(
        "books.set_vendor_1099",
        json!({"vendor": "Landlord LLC", "form_1099": "misc", "decided_by": "Test Owner"}),
    );
    let rent = m.post(
        "2026-02-01",
        json!({}),
        &[("6999", "debit", "2500.00"), ("1000", "credit", "2500.00")],
    );
    let paid = |m: &mut Mcp| {
        m.read_only("books.vendor_1099", json!({"year": 2026}))["vendors"][0]["paid"].clone()
    };
    assert_eq!(paid(&mut m), "0.00");
    let e = m.ok(
        "books.set_entry_party",
        json!({"entry_id": rent, "party": "landlord llc"}),
    );
    assert_eq!(e["party"], "Landlord LLC");
    assert_eq!(e["vendor"], "Landlord LLC", "the vendor tag follows");
    assert!(e["party_id"].is_u64());
    assert_eq!(paid(&mut m), "2500.00");
    assert_eq!(m.count("entry.party_set"), 1);
    // Clearing clears both tags.
    let e = m.ok(
        "books.set_entry_party",
        json!({"entry_id": rent, "party": null}),
    );
    assert!(e["party"].is_null() && e["vendor"].is_null());
    assert_eq!(paid(&mut m), "0.00");
    // A party without a vendor record clears a vendor tag.
    m.add_party("IRS", &["tax_agency"]);
    m.ok(
        "books.set_entry_vendor",
        json!({"entry_id": rent, "vendor": "Landlord LLC"}),
    );
    let e = m.ok(
        "books.set_entry_party",
        json!({"entry_id": rent, "party": "IRS"}),
    );
    assert_eq!(e["party"], "IRS");
    assert!(e["vendor"].is_null());
    // Refusals.
    m.refuse(
        "books.set_entry_party",
        json!({"entry_id": 999, "party": "IRS"}),
    );
    m.refuse(
        "books.set_entry_party",
        json!({"entry_id": rent, "party": "Nobody"}),
    );
}

#[test]
fn draft_entry_takes_a_party_and_refuses_a_mismatched_vendor() {
    let mut m = books("draft-party");
    m.add_party("Acme Design", &["contractor"]);
    m.ok("books.add_vendor", json!({"name": "Acme Design"}));
    m.ok("books.add_vendor", json!({"name": "Hosting Co"}));
    let id = m.draft("2026-03-01", json!({"party": "Acme Design"}), &[]);
    let e = m.read_only("books.get_entry", json!({"entry_id": id}));
    assert_eq!(e["party"], "Acme Design");
    assert_eq!(e["vendor"], "Acme Design");
    // Both, agreeing.
    let id = m.draft(
        "2026-03-02",
        json!({"party": "Acme Design", "vendor": "Acme Design"}),
        &[],
    );
    assert_eq!(
        m.read_only("books.get_entry", json!({"entry_id": id}))["party"],
        "Acme Design"
    );
    // A vendor alone tags its party too.
    let id = m.draft("2026-03-03", json!({"vendor": "Hosting Co"}), &[]);
    assert_eq!(
        m.read_only("books.get_entry", json!({"entry_id": id}))["party"],
        "Hosting Co"
    );
    // Refusals create no draft.
    m.refuse(
        "books.draft_entry",
        json!({"date": "2026-03-04", "description": "x", "party": "Acme Design", "vendor": "Hosting Co"}),
    );
    m.refuse(
        "books.draft_entry",
        json!({"date": "2026-03-04", "description": "x", "party": "Nobody"}),
    );
}

// ------------------------------------------------- books.party totals

/// A small year, computed by hand (see `PartyTotalsOut`): cash is 1000;
/// 2050 (card) and 2100 (members) are liabilities, so they count as money;
/// 2000 (payables) and 1200 (receivables) don't.
#[test]
fn party_totals_match_a_hand_computed_year() {
    let mut m = books("totals");
    m.add_party("Acme Design", &["contractor"]);
    let v = m.ok("books.add_vendor", json!({"name": "Acme Design"}))["id"]
        .as_u64()
        .unwrap();
    m.ok(
        "books.set_vendor_1099",
        json!({"vendor": "Acme Design", "form_1099": "nec", "decided_by": "Test Owner"}),
    );
    let acme = json!({"party": "Acme Design"});
    // Last year: not in 2026.
    let e0 = m.post(
        "2025-12-15",
        acme.clone(),
        &[("6000", "debit", "999.00"), ("1000", "credit", "999.00")],
    );
    // Paid from the bank: paid 500, 1099 500.
    let e1 = m.post(
        "2026-02-01",
        acme.clone(),
        &[("6000", "debit", "500.00"), ("1000", "credit", "500.00")],
    );
    // By card: paid 300, not in the 1099 (the card processor reports it).
    let e2 = m.post(
        "2026-03-01",
        acme.clone(),
        &[("6000", "debit", "300.00"), ("2050", "credit", "300.00")],
    );
    // A refund: received 50; the 1099 nets it.
    let e3 = m.post(
        "2026-05-01",
        acme.clone(),
        &[("1000", "debit", "50.00"), ("6000", "credit", "50.00")],
    );
    // Paid 100 and reversed: nets to nothing.
    let e4 = m.post(
        "2026-06-01",
        acme.clone(),
        &[("6000", "debit", "100.00"), ("1000", "credit", "100.00")],
    );
    let e4r = m.ok(
        "books.reverse_entry",
        json!({"entry_id": e4, "date": "2026-06-02", "description": "undo", "confirmed": true}),
    )["reversal_id"]
        .as_u64()
        .unwrap();
    // A bill of 800 (not paid until paid), 300 of it paid from the bank.
    let logo_invoice = m.wire.upload(
        b"logo invoice",
        "logo-invoice.pdf",
        "application/pdf",
        "receipt",
    );
    let bill = m.ok(
        "books.create_bill",
        json!({"party_id": v, "date": "2026-07-01", "due_date": "2026-07-31",
               "vendor_ref": "D-1041",
               "lines": [{"description": "Logo", "amount": "800.00", "account": "6000"}],
               "upload_id": logo_invoice, "confirmed": true}),
    );
    let pay = m.ok(
        "books.pay_bill",
        json!({"document_id": bill["id"], "date": "2026-07-15", "amount": "300.00",
               "cash_account": "1000"}),
    )["entry_id"]
        .as_u64()
        .unwrap();
    // A draft: listed, never counted.
    let d = m.draft(
        "2026-08-01",
        acme.clone(),
        &[("6000", "debit", "70.00"), ("1000", "credit", "70.00")],
    );
    // Someone else's entry: not Acme's.
    m.post(
        "2026-02-02",
        json!({}),
        &[("6000", "debit", "40.00"), ("1000", "credit", "40.00")],
    );

    let p = m.party("Acme Design", 2026);
    let t = &p["totals"];
    assert_eq!(t["paid"], "1100.00", "500 + 300 + 100 - 100 + 300");
    assert_eq!(t["received"], "50.00");
    assert_eq!(t["billed"], "800.00");
    assert_eq!(t["owed"], "500.00");
    assert_eq!(t["invoiced"], "0.00");
    assert_eq!(t["receivable"], "0.00");
    assert_eq!(t["amount_1099"], "750.00", "500 - 50 + 100 - 100 + 300");
    assert_eq!(t["form_1099"], "1099-NEC");
    assert_eq!(t["gross_pay"], "0.00");
    // The 1099 amount is books.vendor_1099's.
    let w = m.read_only("books.vendor_1099", json!({"year": 2026}));
    assert_eq!(w["vendors"][0]["paid"], "750.00");
    // Every entry it is on, newest first, drafts included.
    let ids: Vec<u64> = p["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["entry_id"].as_u64().unwrap())
        .collect();
    let bill_entry = bill["entry_id"].as_u64().unwrap();
    assert_eq!(ids, [d, pay, bill_entry, e4r, e4, e3, e2, e1, e0]);
    let draft = &p["entries"][0];
    assert_eq!(draft["status"], "draft");
    assert_eq!(draft["total"], "70.00");
    assert_eq!(p["entries"][3]["reversal_of"], json!(e4));
    // Its bill.
    let b = &p["bills"][0];
    assert_eq!(b["id"], bill["id"]);
    assert_eq!(b["total"], "800.00");
    assert_eq!(b["paid"], "300.00");
    assert_eq!(b["balance"], "500.00");
    assert_eq!(b["status"], "partial");
    assert_eq!(b["due_date"], "2026-07-31");
    assert!(p["invoices"].as_array().unwrap().is_empty());
    // Last year, on its own.
    let last = m.party("Acme Design", 2025);
    assert_eq!(last["year"], 2025);
    assert_eq!(last["totals"]["paid"], "999.00");
    assert_eq!(last["totals"]["billed"], "0.00");
    assert_eq!(last["totals"]["owed"], "500.00", "owed is now, every year");
    // The list carries the same paid and received.
    let listed = m.read_only("books.parties", json!({"year": 2026}));
    let row = &listed["parties"][0];
    assert_eq!(row["paid"], "1100.00");
    assert_eq!(row["received"], "50.00");
}

#[test]
fn party_totals_for_a_customer_a_member_and_a_prepaid() {
    let mut m = books("totals-others");
    let c = m.ok("books.add_customer", json!({"name": "Client Co"}))["id"]
        .as_u64()
        .unwrap();
    let inv = m.ok(
        "books.create_invoice",
        json!({"party_id": c, "date": "2026-03-01", "due_date": "2026-03-31",
               "lines": [{"description": "Work", "amount": "1000.00"}],
               "confirmed": true}),
    );
    let pay = m.ok(
        "books.receive_payment",
        json!({"document_id": inv["id"], "date": "2026-03-20", "amount": "600.00",
               "cash_account": "1000"}),
    )["entry_id"]
        .as_u64()
        .unwrap();
    let p = m.party("Client Co", 2026);
    let t = &p["totals"];
    assert_eq!(t["invoiced"], "1000.00");
    assert_eq!(t["received"], "600.00", "the payment on its invoice");
    assert_eq!(t["paid"], "0.00");
    assert_eq!(t["receivable"], "400.00");
    assert!(t["amount_1099"].is_null(), "no vendor record");
    let ids: Vec<u64> = p["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["entry_id"].as_u64().unwrap())
        .collect();
    assert_eq!(ids, [pay, inv["entry_id"].as_u64().unwrap()]);
    assert_eq!(p["invoices"][0]["balance"], "400.00");
    assert_eq!(p["invoices"][0]["status"], "partial");

    m.add_party("Tobin", &["member"]);
    let member = json!({"party": "Tobin"});
    m.post(
        "2026-01-02",
        member.clone(),
        &[("1000", "debit", "2000.00"), ("3000", "credit", "2000.00")],
    );
    m.post(
        "2026-09-30",
        member,
        &[("3000", "debit", "500.00"), ("1000", "credit", "500.00")],
    );
    let t = m.party("Tobin", 2026)["totals"].clone();
    assert_eq!(t["received"], "2000.00", "a contribution");
    assert_eq!(t["paid"], "500.00", "a distribution");

    // A prepaid bought with a member's card: an asset, paid through what
    // the members advanced. That is money when it is a financing account
    // (rules.md B-35 item 5, P3 defect 24); unlisted, Due to Members is a
    // promise like any other liability, counted when the member is paid
    // back.
    let prepaid = |m: &mut Mcp| {
        m.add_party("Insurer", &[]);
        m.post(
            "2026-04-01",
            json!({"party": "Insurer"}),
            &[("1300", "debit", "1200.00"), ("2100", "credit", "1200.00")],
        );
        m.party("Insurer", 2026)["totals"].clone()
    };
    let t = prepaid(&mut m);
    assert_eq!(
        (t["paid"].clone(), t["received"].clone()),
        (json!("0.00"), json!("0.00"))
    );
    let dir = test_dir("totals-financing");
    let toml = books_toml(&dir, "Spec Co", "financing_accounts = [\"2100\"]\n");
    let mut f = Mcp::start(&dir, &toml);
    f.accounts(CHART);
    f.books_dir = Some(dir);
    let t = prepaid(&mut f);
    assert_eq!(t["paid"], "1200.00");
    assert_eq!(t["received"], "0.00");
}

// ------------------------------------------------- payroll

#[test]
fn payroll_names_the_employee_party() {
    let mut m = books("payroll");
    let pat = m.add_party("Pat Doe", &["employee"]);
    m.add_party("Acme Design", &["contractor"]);
    // Not an employee: refused, nothing written.
    m.refuse(
        "books.run_payroll",
        json!({"date": "2026-02-15", "cash_account": "1000", "lines": [pat_line("Acme Design")]}),
    );
    m.refuse(
        "books.run_payroll",
        json!({"date": "2026-02-15", "cash_account": "1000", "lines": [pat_line("Nobody")]}),
    );
    // Neither employee nor name.
    let mut nameless = pat_line("Pat Doe");
    nameless.as_object_mut().unwrap().remove("employee");
    m.refuse(
        "books.run_payroll",
        json!({"date": "2026-02-15", "cash_account": "1000", "lines": [nameless]}),
    );
    let run = m.ok(
        "books.run_payroll",
        json!({"date": "2026-02-15", "cash_account": "1000", "lines": [pat_line("pat doe")]}),
    );
    let p = m.party(&pat.to_string(), 2026);
    let line = &p["payroll"][0];
    assert_eq!(line["run_id"], run["run_id"]);
    assert_eq!(line["entry_id"], run["entry_id"]);
    assert_eq!(line["date"], "2026-02-15");
    assert_eq!(line["gross_pay"], "1000.00");
    assert_eq!(line["net_pay"], "773.50", "1000 - 100 - 50 - 62 - 14.50");
    assert_eq!(p["totals"]["gross_pay"], "1000.00");
    assert_eq!(p["totals"]["net_pay"], "773.50");
    // The run record carries the employee's name by default.
    let journal = String::from_utf8(m.journal()).unwrap();
    assert!(
        journal.contains("\"employee_name\":\"Pat Doe\""),
        "{journal}"
    );
    assert_eq!(m.party("Pat Doe", 2025)["totals"]["gross_pay"], "0.00");
}

/// The books verify (replay and the SQLite projection) after a payroll run
/// that names its employee party.
#[test]
fn books_verify_after_payroll_with_an_employee_party() {
    let mut m = books("payroll-verify");
    m.add_party("Pat Doe", &["employee"]);
    m.ok(
        "books.run_payroll",
        json!({"date": "2026-02-15", "cash_account": "1000", "lines": [pat_line("Pat Doe")]}),
    );
    let v = m.ok("books.verify", json!({}));
    assert_eq!(v["status"], "ok", "{v}");
}

// ------------------------------------------------- books.parties

#[test]
fn parties_lists_by_id_and_filters_by_role() {
    let mut m = books("list");
    let a = m.add_party("Acme Design", &["contractor"]);
    let i = m.add_party("IRS", &["tax_agency"]);
    let b = m.add_party("Quillmere", &["financial_institution"]);
    let u = m.add_party("Utah Division of Corporations", &["government_agency"]);
    let all = m.read_only("books.parties", json!({}));
    assert!(all["year"].is_i64(), "defaults to this year");
    let ids: Vec<u64> = all["parties"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["party_id"].as_u64().unwrap())
        .collect();
    assert_eq!(ids, [a, i, b, u]);
    let row = &all["parties"][0];
    assert_eq!(row["name"], "Acme Design");
    assert_eq!(row["label"], "Acme Design");
    assert_eq!(roles(row), ["contractor"]);
    assert_eq!(row["w9_on_file"], false);
    assert!(row["tin_last4"].is_null());
    assert_eq!(row["paid"], "0.00");
    let agencies = m.read_only("books.parties", json!({"role": "tax_agency"}));
    assert_eq!(agencies["parties"].as_array().unwrap().len(), 1);
    assert_eq!(agencies["parties"][0]["party_id"], json!(i));
    // A regulator is its own role, not a tax agency.
    let regulators = m.read_only("books.parties", json!({"role": "government_agency"}));
    assert_eq!(regulators["parties"].as_array().unwrap().len(), 1);
    assert_eq!(regulators["parties"][0]["party_id"], json!(u));
    m.refuse("books.parties", json!({"role": "wizard"}));
}

// ------------------------------------------------- books.show { party }

#[test]
fn show_opens_a_party_by_name_or_id() {
    let mut m = books("show");
    let a = m.add_party("Acme Design", &[]);
    let s = m.read_only("books.show", json!({"party": "acme design"}));
    assert!(s["shown"].as_str().unwrap().contains("Acme Design"), "{s}");
    m.read_only("books.show", json!({"party": a.to_string()}));
    m.read_only("books.show", json!({"page": "parties"}));
    m.refuse("books.show", json!({"party": "Nobody"}));
    m.refuse(
        "books.show",
        json!({"party": "Acme Design", "page": "parties"}),
    );
}

// ------------------------------------------------- never a full TIN out

/// No tool output in a session full of party facts holds a nine-digit run.
#[test]
fn no_party_output_holds_a_full_tin() {
    let mut m = books("no-tin");
    m.add_party("Acme Design", &["contractor"]);
    m.ok("books.add_vendor", json!({"name": "Acme Design"}));
    m.ok("books.set_w9", w9("Acme Design"));
    for args in [
        json!({"name": "Acme 123456789"}),
        json!({"name": "Acme 123 45 6789"}),
    ] {
        m.refuse("books.add_party", args);
    }
    let outs = [
        m.read_only("books.parties", json!({"year": 2026})),
        m.party("Acme Design", 2026),
        m.read_only("books.list_vendors", json!({})),
    ];
    for out in outs {
        let text = out.to_string();
        let digits: Vec<usize> = text
            .split(|c: char| !c.is_ascii_digit())
            .map(str::len)
            .collect();
        assert!(digits.iter().all(|&n| n < 9), "{text}");
    }
}

// ---------------------------------------------------------------- [books] parties

/// Books with the chart and extra `[books]` TOML.
fn books_with(name: &str, extra: &str) -> Mcp {
    let dir = test_dir(name);
    let toml = books_toml(&dir, "Spec Co", extra);
    let mut m = Mcp::start(&dir, &toml);
    m.accounts(CHART);
    m.books_dir = Some(dir);
    m
}

/// Oracle: PostEntryOut party fields and the "suggest" default
/// (docs/parties-and-attachments.md "Counterparties"): an untagged preview
/// names likely parties (description, then the same description on an
/// earlier tagged entry); nothing is refused.
#[test]
fn suggest_names_likely_parties_and_refuses_nothing() {
    let mut m = books("suggest");
    assert_eq!(
        m.read_only("books.settings", json!({}))["parties"],
        "suggest"
    );
    let apple = m.add_party("Apple Inc.", &[]);
    let id = m.ok(
        "books.draft_entry",
        json!({"date": "2026-03-01", "description": "APPLE.COM/BILL iCloud"}),
    )["entry_id"]
        .as_u64()
        .unwrap();
    m.ok(
        "books.add_line",
        json!({"entry_id": id, "account": "6999", "side": "debit", "amount": "2.99"}),
    );
    m.ok(
        "books.add_line",
        json!({"entry_id": id, "account": "2050", "side": "credit", "amount": "2.99"}),
    );
    let pv = m.ok("books.post_entry", json!({"entry_id": id}));
    assert_eq!(pv["party_policy"], "suggest");
    assert_eq!(pv["party_missing"], true);
    assert_eq!(pv["party_suggestions"][0]["party_id"], apple);
    assert_eq!(
        pv["party_suggestions"][0]["why"],
        "named in the description"
    );
    m.ok(
        "books.post_entry",
        json!({"entry_id": id, "confirmed": true}),
    );
    // Tagged: the next entry with the same description is suggested from it.
    m.ok(
        "books.set_entry_party",
        json!({"entry_id": id, "party": apple.to_string()}),
    );
    let other = m.add_party("Zed", &[]);
    let _ = other;
    let id2 = m.ok(
        "books.draft_entry",
        json!({"date": "2026-04-01", "description": "APPLE.COM/BILL iCloud"}),
    )["entry_id"]
        .as_u64()
        .unwrap();
    m.ok(
        "books.add_line",
        json!({"entry_id": id2, "account": "6999", "side": "debit", "amount": "2.99"}),
    );
    m.ok(
        "books.add_line",
        json!({"entry_id": id2, "account": "2050", "side": "credit", "amount": "2.99"}),
    );
    let pv = m.ok("books.post_entry", json!({"entry_id": id2}));
    let s = pv["party_suggestions"].as_array().unwrap();
    assert_eq!(s.len(), 1, "one party, listed once: {s:?}");
    assert_eq!(s[0]["party_id"], apple);
}

/// Oracle: "require": posting is refused (party_required) and journals
/// nothing until the entry names a party or is marked as having none;
/// no_party_reason is refused alongside a party.
#[test]
fn require_refuses_until_tagged_or_marked_none() {
    let mut m = books_with("require", "parties = \"require\"\n");
    assert_eq!(
        m.read_only("books.settings", json!({}))["parties"],
        "require"
    );
    let bank = m.add_party("Quillmere", &["financial_institution"]);
    let id = m.draft(
        "2026-03-01",
        json!({}),
        &[("6999", "debit", "5.00"), ("1000", "credit", "5.00")],
    );
    let before = m.journal();
    let e = m.refuse(
        "books.post_entry",
        json!({"entry_id": id, "confirmed": true}),
    );
    assert!(e.contains("party_required"), "{e}");
    assert_eq!(m.journal(), before, "refusal journals nothing");
    m.ok(
        "books.set_entry_party",
        json!({"entry_id": id, "party": bank.to_string()}),
    );
    let pv = m.ok(
        "books.post_entry",
        json!({"entry_id": id, "confirmed": true}),
    );
    assert_eq!(pv["status"], "posted");
    assert_eq!(pv["party_id"], bank);
    // Marked none at draft time: posts, and the reason is journaled.
    let t = m.draft(
        "2026-03-02",
        json!({"no_party_reason": "transfer between own accounts"}),
        &[("1300", "debit", "1.00"), ("1000", "credit", "1.00")],
    );
    let pv = m.ok(
        "books.post_entry",
        json!({"entry_id": t, "confirmed": true}),
    );
    assert_eq!(pv["no_party_reason"], "transfer between own accounts");
    assert_eq!(pv["party_missing"], false);
    assert_eq!(m.count("entry.no_party"), 1);
    // Both at once is refused.
    let e = m.refuse(
        "books.draft_entry",
        json!({"date": "2026-03-03", "description": "x", "party": "Quillmere", "no_party_reason": "none"}),
    );
    assert!(e.contains("no_party_reason"), "{e}");
    let e = m.refuse(
        "books.set_entry_party",
        json!({"entry_id": t, "party": "Quillmere", "no_party_reason": "none"}),
    );
    assert!(e.contains("not both"), "{e}");
    // Tagging later clears the mark; marking again needs no party.
    m.ok(
        "books.set_entry_party",
        json!({"entry_id": t, "party": "Quillmere"}),
    );
    m.ok(
        "books.set_entry_party",
        json!({"entry_id": t, "no_party_reason": "own-account transfer"}),
    );
    let r = m.read_only("books.verify", json!({}));
    assert_eq!(r["status"], "ok", "{r}");
}

/// Oracle: "off": the preview never raises a party; a bad value is a
/// settings error.
#[test]
fn off_never_raises_it() {
    let mut m = books_with("off", "parties = \"off\"\n");
    m.add_party("Apple", &[]);
    let id = m.draft(
        "2026-03-01",
        json!({}),
        &[("6999", "debit", "5.00"), ("1000", "credit", "5.00")],
    );
    let pv = m.ok("books.post_entry", json!({"entry_id": id}));
    assert_eq!(pv["party_missing"], false);
    assert!(pv["party_suggestions"].as_array().unwrap().is_empty());
    m.ok(
        "books.post_entry",
        json!({"entry_id": id, "confirmed": true}),
    );
}

// ------------------------------------------------- party shares
//
// Oracles: SetEntryPartyParams.parties and PartyShareParam (the declared
// share rules), EntryPartyOut, PostEntryOut.parties / parties_error (code
// party_shares), PartyTotalsOut (each party's share), and
// docs/mcp-interface.md "Several parties". Values worked by hand.

/// Books with both members (50/50, capital 3000 and 3001) in the settings
/// and the directory; `extra` goes under `[books]`.
fn member_books(name: &str, extra: &str) -> Mcp {
    let dir = test_dir(name);
    let toml = books_toml(
        &dir,
        "Spec Co",
        &format!(
            "{extra}\n[[company.members]]\nname = \"Tobin Harlow\"\nshare = \"50%\"\ncapital_account = \"3000\"\n\
             [[company.members]]\nname = \"Morgan Ellis\"\nshare = \"50%\"\ncapital_account = \"3001\"\n"
        ),
    );
    let mut m = Mcp::start(&dir, &toml);
    m.accounts(CHART);
    m.accounts(&[("3001", "Capital K", "equity")]);
    m.add_party("Tobin Harlow", &["member"]);
    m.add_party("Morgan Ellis", &["member"]);
    m.books_dir = Some(dir);
    m
}

fn both() -> Value {
    json!({"parties": [{"party": "Tobin Harlow"}, {"party": "Morgan Ellis"}]})
}

fn resolved(v: &Value) -> Vec<(String, String)> {
    v["parties"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            (
                p["party"].as_str().unwrap().to_string(),
                p["resolved"].as_str().unwrap_or("null").to_string(),
            )
        })
        .collect()
}

#[test]
fn a_joint_reimbursement_names_both_members_and_splits_by_share() {
    let mut m = member_books("shares-joint", "");
    let id = m.draft(
        "2026-04-01",
        both(),
        &[("2100", "debit", "100.00"), ("1000", "credit", "100.00")],
    );
    let preview = m.ok("books.post_entry", json!({"entry_id": id}));
    assert_eq!(
        resolved(&preview),
        [
            ("Tobin Harlow".to_string(), "50.00".to_string()),
            ("Morgan Ellis".to_string(), "50.00".to_string())
        ]
    );
    assert_eq!(preview["parties"][0]["share"], "derived");
    assert!(preview["party_id"].is_null(), "no sole party");
    assert!(preview["parties_error"].is_null());
    assert_eq!(preview["party_missing"], false);
    m.ok(
        "books.post_entry",
        json!({"entry_id": id, "confirmed": true}),
    );
    // Named derived on the draft, then fixed at the amounts it posted at.
    assert_eq!(m.count("entry.parties_set"), 2);
    for who in ["Tobin Harlow", "Morgan Ellis"] {
        let t = m.party(who, 2026)["totals"].clone();
        assert_eq!(t["paid"], "50.00", "{who}'s half");
    }
    let list = m.read_only("books.parties", json!({"year": 2026}));
    let paid: Vec<&str> = list["parties"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["paid"].as_str().unwrap())
        .collect();
    assert_eq!(paid, ["50.00", "50.00"]);
}

#[test]
fn a_joint_contribution_derives_from_the_capital_lines() {
    let mut m = member_books("shares-capital", "");
    let id = m.post(
        "2026-01-02",
        both(),
        &[
            ("1000", "debit", "300.00"),
            ("3000", "credit", "200.00"),
            ("3001", "credit", "100.00"),
        ],
    );
    let e = m.read_only("books.get_entry", json!({"entry_id": id}));
    assert_eq!(
        resolved(&e),
        [
            ("Tobin Harlow".to_string(), "-200.00".to_string()),
            ("Morgan Ellis".to_string(), "-100.00".to_string())
        ]
    );
    assert_eq!(
        m.party("Tobin Harlow", 2026)["totals"]["received"],
        "200.00"
    );
    assert_eq!(
        m.party("Morgan Ellis", 2026)["totals"]["received"],
        "100.00"
    );
}

#[test]
fn explicit_amounts_and_percents_apportion_exactly() {
    let mut m = member_books("shares-explicit", "");
    m.add_party("Riley", &["contractor"]);
    let id = m.post(
        "2026-05-01",
        json!({"party": "Riley"}),
        &[("6000", "debit", "100.00"), ("1000", "credit", "100.00")],
    );
    let e = m.ok(
        "books.set_entry_party",
        json!({"entry_id": id, "parties": [
            {"party": "Riley", "amount": "70.00"},
            {"party": "Tobin Harlow", "amount": "30.00"},
        ]}),
    );
    assert_eq!(e["parties"][0]["share"], "amount");
    assert_eq!(e["parties"][0]["amount"], "70.00");
    assert_eq!(e["parties"][1]["resolved"], "30.00");
    assert_eq!(m.party("Riley", 2026)["totals"]["paid"], "70.00");
    let e = m.ok(
        "books.set_entry_party",
        json!({"entry_id": id, "parties": [
            {"party": "Riley", "percent": "33.333"},
            {"party": "Tobin Harlow", "percent": "66.667"},
        ]}),
    );
    // 33.333 → 33.33, 66.667 → 66.66; the leftover cent to the first.
    assert_eq!(e["parties"][0]["resolved"], "33.34");
    assert_eq!(e["parties"][1]["resolved"], "66.66");
    assert_eq!(e["parties"][0]["percent"], "33.333");
    assert_eq!(m.count("entry.parties_set"), 2);
}

#[test]
fn shares_that_dont_add_up_are_refused_changing_nothing() {
    let mut m = member_books("shares-refused", "");
    m.add_party("Riley", &["contractor"]);
    let id = m.post(
        "2026-05-01",
        json!({}),
        &[("6000", "debit", "100.00"), ("1000", "credit", "100.00")],
    );
    let set = |parties: Value| json!({"entry_id": id, "parties": parties});
    let why = m.refuse(
        "books.set_entry_party",
        set(json!([{"party": "Riley", "amount": "60.00"}, {"party": "Tobin Harlow", "amount": "30.00"}])),
    );
    assert!(why.contains("sum"), "{why}");
    m.refuse(
        "books.set_entry_party",
        set(json!([{"party": "Riley", "percent": "50"}, {"party": "Tobin Harlow", "percent": "40"}])),
    );
    m.refuse(
        "books.set_entry_party",
        set(json!([{"party": "Riley"}, {"party": "Riley"}])),
    );
    m.refuse(
        "books.set_entry_party",
        set(json!([{"party": "Riley", "amount": "50.00", "percent": "50"}])),
    );
    m.refuse(
        "books.set_entry_party",
        set(json!([{"party": "Riley", "amount": "50.00"}, {"party": "Tobin Harlow"}])),
    );
    // Derived for a non-member among several.
    m.refuse(
        "books.set_entry_party",
        set(json!([{"party": "Riley"}, {"party": "Tobin Harlow"}])),
    );
    m.refuse(
        "books.set_entry_party",
        json!({"entry_id": id, "party": "Riley", "parties": [{"party": "Riley"}]}),
    );
    assert_eq!(m.count("entry.parties_set"), 0);
}

#[test]
fn a_draft_whose_shares_dont_add_up_previews_the_error_and_wont_post() {
    let mut m = member_books("shares-draft", "");
    let id = m.draft(
        "2026-06-01",
        json!({"parties": [
            {"party": "Tobin Harlow", "amount": "10.00"},
            {"party": "Morgan Ellis", "amount": "10.00"},
        ]}),
        &[("2100", "debit", "25.00"), ("1000", "credit", "25.00")],
    );
    let preview = m.ok("books.post_entry", json!({"entry_id": id}));
    assert!(preview["parties_error"].is_string(), "{preview}");
    assert!(preview["parties"][0]["resolved"].is_null());
    let why = m.refuse(
        "books.post_entry",
        json!({"entry_id": id, "confirmed": true}),
    );
    assert!(why.contains("party_shares"), "{why}");
    m.ok(
        "books.set_entry_party",
        json!({"entry_id": id, "parties": [
            {"party": "Tobin Harlow", "amount": "15.00"},
            {"party": "Morgan Ellis", "amount": "10.00"},
        ]}),
    );
    m.ok(
        "books.post_entry",
        json!({"entry_id": id, "confirmed": true}),
    );
    assert_eq!(m.party("Tobin Harlow", 2026)["totals"]["paid"], "15.00");
}

#[test]
fn under_require_several_parties_satisfy_the_policy() {
    let mut m = member_books("shares-require", "parties = \"require\"");
    let id = m.draft(
        "2026-07-01",
        both(),
        &[("2100", "debit", "40.00"), ("1000", "credit", "40.00")],
    );
    let preview = m.ok("books.post_entry", json!({"entry_id": id}));
    assert_eq!(preview["party_missing"], false);
    m.ok(
        "books.post_entry",
        json!({"entry_id": id, "confirmed": true}),
    );
    // The reversal carries both shares.
    m.ok(
        "books.reverse_entry",
        json!({"entry_id": id, "date": "2026-07-02", "description": "undo", "confirmed": true}),
    );
    assert_eq!(m.party("Morgan Ellis", 2026)["totals"]["paid"], "0.00");
    m.read_only("books.verify", json!({}));
}

/// docs/parties-and-attachments.md "Counterparties" (require): the
/// engine's own posting paths name their counterparty or are refused
/// before anything posts. books.run_payroll names the employee party on
/// the entry (so their books.party counts the net pay); a line by name
/// only is refused. books.remit_payroll_taxes needs `agency`;
/// books.dispose_asset needs `buyer` for a sale, and marks a retirement
/// with no proceeds as having none.
#[test]
fn under_require_engine_postings_name_their_counterparty() {
    let mut m = books_with("require-engine", "parties = \"require\"\n");
    m.accounts(&[
        ("1500", "Equipment", "asset"),
        ("1510", "Accumulated Depreciation", "asset"),
        ("6500", "Depreciation", "expense"),
        ("7100", "Gain or Loss on Disposal", "revenue"),
    ]);
    let pat = m.add_party("Pat Doe", &["employee"]);
    let irs = m.add_party("IRS", &["tax_agency"]);
    let buyer = m.add_party("Buyer Co", &[]);
    let mut by_name = pat_line("Pat Doe");
    by_name.as_object_mut().unwrap().remove("employee");
    by_name["employee_name"] = json!("Pat Doe");
    let e = m.refuse(
        "books.run_payroll",
        json!({"date": "2026-02-15", "cash_account": "1000", "lines": [by_name]}),
    );
    assert!(e.contains("party_required"), "{e}");
    let run = m.ok(
        "books.run_payroll",
        json!({"date": "2026-02-15", "cash_account": "1000", "lines": [pat_line("Pat Doe")]}),
    );
    let entry = m.read_only("books.get_entry", json!({"entry_id": run["entry_id"]}));
    assert_eq!(entry["parties"][0]["party_id"], json!(pat), "{entry}");
    assert_eq!(m.party("Pat Doe", 2026)["totals"]["paid"], "773.50");
    let e = m.refuse(
        "books.remit_payroll_taxes",
        json!({"date": "2026-03-01", "cash_account": "1000"}),
    );
    assert!(e.contains("party_required"), "{e}");
    let r = m.ok(
        "books.remit_payroll_taxes",
        json!({"date": "2026-03-01", "cash_account": "1000", "agency": "IRS"}),
    );
    let entry = m.read_only("books.get_entry", json!({"entry_id": r["entry_id"]}));
    assert_eq!(entry["parties"][0]["party_id"], json!(irs), "{entry}");
    // Assets: one sold, one retired.
    let reg = |m: &mut Mcp, tag: &str| {
        m.ok(
            "books.register_asset",
            json!({"description": "Laptop", "purchase_date": "2026-01-10", "cost": "1200.00",
                   "salvage_value": "0.00", "useful_life_months": 36, "cost_account": "1500",
                   "accum_depr_account": "1510", "expense_account": "6500", "tag": tag,
                   "tax_basis_rule": "cost"}),
        )["asset_id"]
            .as_u64()
            .unwrap()
    };
    let (sold, retired) = (reg(&mut m, "LAP-1"), reg(&mut m, "LAP-2"));
    let sale = json!({"asset_id": sold, "date": "2026-06-30", "proceeds": "500.00",
                      "proceeds_account": "1000", "gain_loss_account": "7100",
                      "confirmed": true});
    let before = m.journal();
    let e = m.refuse("books.dispose_asset", sale.clone());
    assert!(e.contains("party_required"), "{e}");
    assert!(m.journal() == before, "refused, nothing journaled");
    let mut with_buyer = sale;
    with_buyer["buyer"] = json!("Buyer Co");
    let d = m.ok("books.dispose_asset", with_buyer);
    let entry = m.read_only("books.get_entry", json!({"entry_id": d["entry_id"]}));
    assert_eq!(entry["parties"][0]["party_id"], json!(buyer), "{entry}");
    let d = m.ok(
        "books.dispose_asset",
        json!({"asset_id": retired, "date": "2026-06-30", "proceeds": "0.00",
               "proceeds_account": "1000", "gain_loss_account": "7100", "confirmed": true}),
    );
    let entry = m.read_only("books.get_entry", json!({"entry_id": d["entry_id"]}));
    assert!(entry["no_party_reason"].is_string(), "{entry}");
    assert_eq!(m.ok("books.verify", json!({}))["status"], "ok");
}

/// fix_derived_shares INVARIANT 1 (docs/parties-and-attachments.md
/// "Shares": posted is forever, the split included): a joint
/// reimbursement posted with derived shares at 50/50 keeps 50.00 each when
/// the members' shares later become 60/40, and when a member is archived;
/// a draft still follows the settings until it posts.
#[test]
fn a_posted_derived_split_survives_settings_changes_and_archiving() {
    let mut m = member_books("shares-fixed", "");
    let posted = m.post(
        "2026-03-01",
        both(),
        &[("2100", "debit", "100.00"), ("1000", "credit", "100.00")],
    );
    let e = m.read_only("books.get_entry", json!({"entry_id": posted}));
    assert_eq!(
        resolved(&e),
        [
            ("Tobin Harlow".to_string(), "50.00".to_string()),
            ("Morgan Ellis".to_string(), "50.00".to_string())
        ]
    );
    assert_eq!(e["parties"][0]["share"], "amount", "fixed when posted: {e}");
    assert_eq!(e["parties"][0]["amount"], "50.00", "{e}");
    let draft = m.draft(
        "2026-03-02",
        both(),
        &[("2100", "debit", "100.00"), ("1000", "credit", "100.00")],
    );
    // The members' split changes to 60/40, as an agent changes it.
    let toml = m.books_dir.as_ref().unwrap().join("wanderbooks.toml");
    let text = std::fs::read_to_string(&toml).unwrap();
    let changed = text
        .replacen("share = \"50%\"", "share = \"60%\"", 1)
        .replacen("share = \"50%\"", "share = \"40%\"", 1);
    m.ok(
        "books.set_setting",
        json!({"changes": common::settings_changes(&changed, None, "USD"),
               "decided_by": "Test Owner"}),
    );
    let d = m.ok("books.post_entry", json!({"entry_id": draft}));
    assert_eq!(
        resolved(&d),
        [
            ("Tobin Harlow".to_string(), "60.00".to_string()),
            ("Morgan Ellis".to_string(), "40.00".to_string())
        ],
        "a draft follows the settings"
    );
    let e = m.read_only("books.get_entry", json!({"entry_id": posted}));
    assert_eq!(
        resolved(&e),
        [
            ("Tobin Harlow".to_string(), "50.00".to_string()),
            ("Morgan Ellis".to_string(), "50.00".to_string())
        ],
        "posted history keeps its split"
    );
    // Morgan archived: her history stays, and Tobin's.
    let mo = e["parties"][1]["party_id"].as_u64().unwrap();
    m.ok(
        "books.update_party",
        json!({"party": mo.to_string(), "archived": true}),
    );
    assert_eq!(m.party("Tobin Harlow", 2026)["totals"]["paid"], "50.00");
    assert_eq!(m.party(&mo.to_string(), 2026)["totals"]["paid"], "50.00");
    assert_eq!(m.ok("books.verify", json!({}))["status"], "ok");
}
