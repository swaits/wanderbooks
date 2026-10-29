//! P1 through the real server: the books' own settings, conventions,
//! approvals, books.start (plan/served.md P1). Drives a real `wanderbooks
//! server` with raw JSON-RPC, as an agent reaches it (common.rs).

use common::{Books, Wire};
use serde_json::{Value, json};

mod common;

/// One agent's session with the test's books (common.rs).
struct Mcp {
    wire: Wire,
    next_id: u64,
}

impl Mcp {
    /// A new session with `books` (initialized by the test).
    fn spawn(books: &Books) -> Self {
        Self {
            wire: books.wire(),
            next_id: 1,
        }
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let msg = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        self.wire.send(&msg);
        let resp = self
            .wire
            .recv(common::TIMEOUT)
            .unwrap_or_else(|| panic!("no answer to {method}: {:?}", self.wire.refused));
        assert_eq!(resp["id"], id, "id mismatch in {resp}");
        assert!(
            resp.get("error").is_none(),
            "JSON-RPC error for {method}: {}",
            resp["error"]
        );
        resp
    }

    fn notify(&mut self, method: &str) {
        self.wire
            .send(&json!({"jsonrpc": "2.0", "method": method, "params": {}}));
    }

    /// Call a tool; panic on tool error. Returns structuredContent.
    fn call(&mut self, tool: &str, args: Value) -> Value {
        let resp = common::tools_call(|m, p| self.request(m, p), tool, args.clone());
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
        let resp = common::tools_call(|m, p| self.request(m, p), tool, args.clone());
        let result = &resp["result"];
        assert_eq!(
            result.get("isError"),
            Some(&json!(true)),
            "expected {tool} to fail, got {result}"
        );
        result["content"][0]["text"].as_str().unwrap().to_string()
    }
}

fn init(m: &mut Mcp) -> String {
    let r = m.request(
        "initialize",
        json!({"protocolVersion": "2025-06-18", "capabilities": {},
               "clientInfo": {"name": "spec-kept", "version": "0"}}),
    );
    m.notify("notifications/initialized");
    r["result"]["instructions"].as_str().unwrap().to_string()
}

const TOML: &str = r#"[company]
name = "Kept Co LLC"
tax_classification = "partnership"  # Form 1065; a Schedule K-1 per member
fiscal_year_start = "01-01"
ein = "28-8525893"

[[company.members]]
name = "A"
share = "50%"
capital_account = "3000"

[[company.members]]
name = "B"
share = "50%"
capital_account = "3001"

[books]
data_dir = "books-data"
receipt_threshold = "0.00"
cash_accounts = ["1000"]
parties = "require"

[viewer]
open_at_start = "ask"
"#;

fn chart(m: &mut Mcp) {
    for (code, name, t) in [
        ("1000", "Bank", "asset"),
        ("3000", "Capital A", "equity"),
        ("3001", "Capital B", "equity"),
        ("6000", "Software", "expense"),
    ] {
        m.call(
            "books.add_account",
            json!({"code": code, "name": name, "account_type": t}),
        );
    }
}

/// Books keep their settings, changed through the tools with who decided
/// and when; the company's EIN only ever leaves the server masked (in
/// books.settings, books.set_setting, books.export_settings and
/// books.start); and a restarted server reads them from the books.
#[test]
fn settings_live_in_the_books_and_the_ein_leaves_masked() {
    let books = Books::with("kept-ein", "USD", Some(TOML));
    let mut m = Mcp::spawn(&books);
    let told = init(&mut m);
    assert!(told.contains("books.start"), "{told}");
    chart(&mut m);
    let settings = m.call("books.settings", json!({}));
    assert_eq!(settings["parties"], "require");
    let shown = settings.to_string();
    assert!(!shown.contains("8525893"), "the EIN is masked: {shown}");
    let ein = settings["keys"]
        .as_array()
        .unwrap()
        .iter()
        .find(|k| k["key"] == "company.ein")
        .unwrap()
        .clone();
    assert_eq!(ein["value"], "**-***5893");
    assert_eq!(ein["set_by"]["decided_by"], "Test Owner");
    let rule = settings["receipt_rule"].clone();
    assert!(rule.as_str().unwrap().contains("every entry"), "{rule}");
    // Settings change through the tools.
    let set = m.call(
        "books.set_setting",
        json!({"changes": [{"key": "company.timezone", "value": "America/Denver"},
                           {"key": "company.ein", "value": "12-3456789"}],
               "decided_by": "B", "decided_on": "2026-10-17"}),
    );
    assert!(!set.to_string().contains("3456789"), "{set}");
    let export = m.call("books.export_settings", json!({}));
    let text = export["toml_text"].as_str().unwrap();
    assert!(text.contains("timezone = \"America/Denver\""), "{text}");
    assert!(!text.contains("3456789"), "{text}");
    let start = m.call("books.start", json!({}));
    assert_eq!(start["book"]["timezone"], "America/Denver");
    assert!(!start.to_string().contains("3456789"));
    drop(m);
    books.restart();
    let mut m = Mcp::spawn(&books);
    let told = init(&mut m);
    assert!(
        told.contains("Settings and conventions live in the books"),
        "{told}"
    );
    assert!(told.contains("Kept Co LLC"), "{told}");
    // What was set before the restart, read back from the books: the
    // timezone, and the new EIN, still masked.
    let start = m.call("books.start", json!({}));
    assert_eq!(start["book"]["timezone"], "America/Denver");
    let settings = m.call("books.settings", json!({}));
    let ein = settings["keys"]
        .as_array()
        .unwrap()
        .iter()
        .find(|k| k["key"] == "company.ein")
        .unwrap()
        .clone();
    assert_eq!(ein["value"], "**-***6789");
    assert_eq!(ein["set_by"]["decided_by"], "B");
    assert!(!settings.to_string().contains("3456789"), "{settings}");
    assert_eq!(m.call("books.verify", json!({}))["status"], "ok");
}

/// Approval rules through the server: enforced, a covered post is refused
/// until its approver approves, then posts; off, they block nothing.
#[test]
fn approval_rules_through_the_server() {
    let books = Books::new("kept-approvals");
    let mut m = Mcp::spawn(&books);
    init(&mut m);
    chart(&mut m);
    m.call(
        "books.set_setting",
        json!({"changes": [{"key": "policy.evidence.threshold", "value": "1000000"},
                           {"key": "policy.approval_rules", "value": "enforce"}],
               "decided_by": "A"}),
    );
    m.call(
        "books.set_convention",
        json!({"id": "big-software", "kind": "approval_rule", "title": "Big software",
               "body_md": "Software over 500 needs B.",
               "structured": {"approver": "B", "accounts": ["6000"], "amount_over": "500"},
               "decided_by": "A"}),
    );
    let e = m.call(
        "books.draft_entry",
        json!({"date": "2026-10-01", "description": "Laptop", "no_party_reason": "test"}),
    )["entry_id"]
        .clone();
    for (account, side) in [("6000", "debit"), ("1000", "credit")] {
        m.call(
            "books.add_line",
            json!({"entry_id": e, "account": account, "side": side, "amount": "900.00"}),
        );
    }
    let preview = m.call("books.post_entry", json!({"entry_id": e}));
    assert_eq!(
        preview["approvals_missing"],
        json!([{"rule": "big-software", "approver": "B"}])
    );
    let refused = m.call_err(
        "books.post_entry",
        json!({"entry_id": e, "confirmed": true}),
    );
    assert!(refused.contains("approval_required"), "{refused}");
    assert!(refused.contains("books.approve_entry"), "{refused}");
    m.call(
        "books.approve_entry",
        json!({"entry_id": e, "approver": "B"}),
    );
    let preview = m.call("books.post_entry", json!({"entry_id": e}));
    assert!(preview.get("approvals_missing").is_none(), "{preview}");
    m.call(
        "books.post_entry",
        json!({"entry_id": e, "confirmed": true}),
    );
    m.call(
        "books.set_setting",
        json!({"changes": [{"key": "policy.approval_rules", "value": null}], "decided_by": "A"}),
    );
    assert_eq!(
        m.call("books.start", json!({}))["rules"]["approval_rules"],
        "off"
    );
}

/// A posting template drafts through the server, to the cent.
#[test]
fn templates_draft_through_the_server() {
    let books = Books::new("kept-template");
    let mut m = Mcp::spawn(&books);
    init(&mut m);
    chart(&mut m);
    m.call(
        "books.set_convention",
        json!({"id": "contribution", "kind": "posting_template",
               "title": "Joint contribution", "body_md": "Half each.",
               "structured": {"description": "Contribution", "lines": [
                   {"account": "1000", "side": "debit", "share": "1"},
                   {"account": "3000", "side": "credit", "share": "0.5"},
                   {"account": "3001", "side": "credit", "share": "0.5"}]},
               "decided_by": "A"}),
    );
    let d = m.call(
        "books.draft_from_template",
        json!({"template_id": "contribution", "date": "2026-10-02", "amount": "0.03",
               "no_party_reason": "between the members"}),
    );
    assert_eq!(d["description"], "Contribution");
    let amounts: Vec<&str> = d["lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["amount"].as_str().unwrap())
        .collect();
    assert_eq!(amounts, ["0.03", "0.02", "0.01"]);
    let e = m.call_err(
        "books.draft_from_template",
        json!({"template_id": "contribution", "date": "2026-10-02", "amount": "0.01"}),
    );
    assert!(e.contains("too small"), "{e}");
    // A slot takes the account each draft names.
    m.call(
        "books.set_convention",
        json!({"id": "bank-expense", "kind": "posting_template",
               "title": "Expense paid from the bank", "body_md": "Dr the expense / Cr 1000.",
               "structured": {"lines": [
                   {"account": "{expense}", "side": "debit", "share": "1"},
                   {"account": "1000", "side": "credit", "share": "1"}]},
               "decided_by": "A"}),
    );
    let e = m.call_err(
        "books.draft_from_template",
        json!({"template_id": "bank-expense", "date": "2026-10-02", "amount": "12.00"}),
    );
    assert!(e.contains("expense"), "{e}");
    let d = m.call(
        "books.draft_from_template",
        json!({"template_id": "bank-expense", "date": "2026-10-02", "amount": "12.00",
               "accounts": {"expense": "6000"}, "no_party_reason": "test"}),
    );
    assert_eq!(d["lines"][0]["account"], "6000");
    assert_eq!(d["lines"][1]["account"], "1000");
}

/// books.start stays at most 8 KB whatever the books hold, and says what
/// it left out.
#[test]
fn start_fits_in_8_kb() {
    let books = Books::new("kept-start");
    let mut m = Mcp::spawn(&books);
    init(&mut m);
    let body =
        "A first line long enough to fill its summary to the brim, and then some more words. "
            .repeat(3);
    for i in 0..60 {
        m.call(
            "books.set_convention",
            json!({"id": format!("note-{i}"), "kind": "note",
                   "title": format!("A rather long convention title number {i}"),
                   "body_md": body, "decided_by": "A"}),
        );
    }
    for i in 0..40 {
        m.call(
            "books.raise_item",
            json!({"title": format!("Open item number {i}, with a title of some length"),
                   "due": "2026-12-31", "decided_by": "A"}),
        );
    }
    let resp = m.request(
        "tools/call",
        json!({"name": "books.start", "arguments": {}}),
    );
    let start = &resp["result"]["structuredContent"];
    let bytes = serde_json::to_string(start).unwrap().len();
    assert!(bytes <= 8 * 1024, "{bytes} bytes");
    let cut = start["truncated"].to_string();
    assert!(cut.contains("books.conventions"), "{cut}");
    assert!(start["conventions"].as_array().unwrap().len() >= 10);
    // isolation_label_is_advisory (plan P2, D-32).
    assert_eq!(start["isolation"], "same_user (advisory)");
}

/// Review of P1 (2026-10-17) and D-44 (owner, 2026-10-23: the import
/// tool goes): books keep their own settings and take no settings file;
/// books.settings shows each value (an identity value masked), and
/// books.import_settings is no tool (an unknown tool: a protocol error).
#[test]
fn books_keeping_their_settings_take_no_settings_file() {
    let books = Books::with("kept-whole", "USD", Some(TOML));
    let mut m = Mcp::spawn(&books);
    init(&mut m);
    let keys = m.call("books.settings", json!({}))["keys"].clone();
    let ein = keys
        .as_array()
        .unwrap()
        .iter()
        .find(|k| k["key"] == "company.ein")
        .unwrap()
        .clone();
    assert_eq!(ein["value"], "**-***5893", "the value, masked");
    let tools = m.request("tools/list", json!({}))["result"]["tools"].clone();
    assert!(
        !tools.to_string().contains("books.import_settings"),
        "{tools}"
    );
    let id = m.next_id;
    m.next_id += 1;
    m.wire
        .send(&json!({"jsonrpc": "2.0", "id": id, "method": "tools/call",
        "params": {"name": "books.import_settings",
                   "arguments": {"toml_text": TOML, "decided_by": "A", "confirmed": true}}}));
    let resp = m.wire.recv(common::TIMEOUT).expect("an answer");
    assert!(resp.get("error").is_some(), "{resp}");
}

/// A convention that names an account is listed when that account is
/// renumbered, and as a problem while it names a code no longer in the
/// chart.
#[test]
fn renumbering_names_the_conventions_to_revise() {
    let books = Books::new("kept-renumber");
    let mut m = Mcp::spawn(&books);
    init(&mut m);
    chart(&mut m);
    m.call(
        "books.set_convention",
        json!({"id": "software", "kind": "posting_template", "title": "Software",
               "body_md": "Dr 6000 / Cr 1000.",
               "structured": {"lines": [
                   {"account": "6000", "side": "debit", "share": "1"},
                   {"account": "1000", "side": "credit", "share": "1"}]},
               "decided_by": "A"}),
    );
    let out = m.call(
        "books.renumber_account",
        json!({"account": "6000", "code": "6010"}),
    );
    assert!(
        out["config_mentions"]
            .to_string()
            .contains("convention software"),
        "{out}"
    );
    let problems = m.call("books.settings", json!({}))["problems"].to_string();
    assert!(
        problems.contains("convention software names account 6000"),
        "{problems}"
    );
}

/// books://start, books://settings and books://conventions read as their
/// tools do.
#[test]
fn the_kept_resources_read() {
    let books = Books::new("kept-resources");
    let mut m = Mcp::spawn(&books);
    init(&mut m);
    chart(&mut m);
    m.call(
        "books.set_convention",
        json!({"id": "software", "kind": "note", "title": "Software",
               "body_md": "Software is 6000.", "decided_by": "A"}),
    );
    let read = |m: &mut Mcp, uri: &str| -> Value {
        let r = m.request("resources/read", json!({"uri": uri}));
        let text = r["result"]["contents"][0]["text"].as_str().unwrap();
        serde_json::from_str(text).unwrap()
    };
    let start = read(&mut m, "books://start");
    assert_eq!(start["state"], "active", "{start}");
    assert_eq!(start["conventions"][0]["id"], "software", "{start}");
    let settings = read(&mut m, "books://settings");
    assert_eq!(settings, m.call("books.settings", json!({})), "{settings}");
    let conventions = read(&mut m, "books://conventions");
    assert_eq!(
        conventions["conventions"][0]["id"], "software",
        "{conventions}"
    );
}

/// A renumber lists exactly the conventions naming the old code.
#[test]
fn renumbering_lists_only_the_conventions_naming_it() {
    let books = Books::new("kept-renumber-only");
    let mut m = Mcp::spawn(&books);
    init(&mut m);
    chart(&mut m);
    for (id, dr, cr) in [("software", "6000", "1000"), ("deposit", "1000", "3000")] {
        m.call(
            "books.set_convention",
            json!({"id": id, "kind": "posting_template", "title": id,
                   "body_md": format!("Dr {dr} / Cr {cr}."),
                   "structured": {"lines": [
                       {"account": dr, "side": "debit", "share": "1"},
                       {"account": cr, "side": "credit", "share": "1"}]},
                   "decided_by": "A"}),
        );
    }
    let out = m.call(
        "books.renumber_account",
        json!({"account": "6000", "code": "6010"}),
    );
    let mentions: Vec<String> = out["config_mentions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .filter(|s| s.starts_with("convention "))
        .collect();
    assert_eq!(
        mentions,
        ["convention software (revise it with books.set_convention)"],
        "{out}"
    );
}
