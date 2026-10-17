//! P1 through the real server: settings moving into the books, the books'
//! own settings, conventions, approvals, books.start (plan/served.md P1).
//! Drives the real `ledger-mcp` binary over stdio with raw JSON-RPC.

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};

struct Mcp {
    stdin: ChildStdin,
    stdout: BufReader<std::process::ChildStdout>,
    next_id: u64,
    child: Child,
}

/// Closing stdin ends the server; wait for it to exit, so the next server
/// on the same books never races it for the writer lock.
impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Mcp {
    fn spawn(data_dir: &std::path::Path) -> Self {
        Self::spawn_with(data_dir, &[])
    }

    fn spawn_with(data_dir: &std::path::Path, extra: &[&std::ffi::OsStr]) -> Self {
        Self::spawn_env(data_dir, extra, &[])
    }

    fn spawn_env(
        data_dir: &std::path::Path,
        extra: &[&std::ffi::OsStr],
        env: &[(&str, &std::path::Path)],
    ) -> Self {
        let bin = env!("CARGO_BIN_EXE_ledger-mcp");
        let mut child = Command::new(bin)
            .arg("--data-dir")
            .arg(data_dir)
            .args(extra)
            .envs(env.iter().map(|(k, v)| (*k, v.as_os_str())))
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

fn test_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("ledger-mcp-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
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

/// A settings file moves into the books, once, with the human's go: set
/// is refused before (it would drop the rest of the file); the preview
/// changes nothing; after, the file is never read, even to start, and the
/// company's EIN only ever leaves the server masked.
#[test]
fn a_settings_file_moves_into_the_books() {
    let dir = test_dir("kept-move");
    let data = dir.join("books-data");
    std::fs::write(dir.join("wanderbooks.toml"), TOML).unwrap();
    let mut m = Mcp::spawn(&data);
    let told = init(&mut m);
    assert!(told.contains("books.start"), "{told}");
    assert!(
        told.contains("the settings file books.settings names"),
        "{told}"
    );
    chart(&mut m);
    assert_eq!(m.call("books.settings", json!({}))["source"], "file");
    let start = m.call("books.start", json!({}));
    assert_eq!(start["settings_source"], "file");
    assert!(
        start["next_steps"]
            .to_string()
            .contains("books.import_settings")
    );
    let refused = m.call_err(
        "books.set_setting",
        json!({"changes": [{"key": "company.cpa", "value": "Casey"}], "decided_by": "A"}),
    );
    assert!(refused.contains("books.import_settings"), "{refused}");
    // The preview records nothing.
    let head = |m: &mut Mcp| m.call("books.start", json!({}))["head"]["seq"].clone();
    let before = head(&mut m);
    let preview = m.call(
        "books.import_settings",
        json!({"toml_text": TOML, "decided_by": "A", "note": "Moving in."}),
    );
    assert_eq!(preview["status"], "preview");
    assert_eq!(head(&mut m), before);
    assert!(
        preview["notes"].to_string().contains("data_dir"),
        "{preview}"
    );
    assert!(
        preview["unknown"].as_array().unwrap().is_empty(),
        "{preview}"
    );
    // The file's comments are listed, never dropped unseen.
    assert_eq!(
        preview["comments"],
        json!(["Form 1065; a Schedule K-1 per member"]),
        "{preview}"
    );
    let shown = preview.to_string();
    assert!(!shown.contains("2215893"), "the EIN is masked: {shown}");
    assert!(shown.contains("**-***5893"), "{shown}");
    let done = m.call(
        "books.import_settings",
        json!({"toml_text": TOML, "decided_by": "A", "confirmed": true}),
    );
    assert_eq!(done["status"], "imported");
    let settings = m.call("books.settings", json!({}));
    assert_eq!(settings["source"], "books");
    assert_eq!(settings["books_toml"], Value::Null);
    assert_eq!(settings["parties"], "require");
    let ein = settings["keys"]
        .as_array()
        .unwrap()
        .iter()
        .find(|k| k["key"] == "company.ein")
        .unwrap()
        .clone();
    assert_eq!(ein["value"], "**-***5893");
    assert_eq!(ein["set_by"]["decided_by"], "A");
    // The file is no longer read: an edit changes nothing.
    std::fs::write(dir.join("wanderbooks.toml"), TOML.replace("0.00", "99.00")).unwrap();
    let rule = m.call("books.settings", json!({}))["receipt_rule"].clone();
    assert!(rule.as_str().unwrap().contains("every entry"), "{rule}");
    // Settings change through the tools now.
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
    assert_eq!(start["settings_source"], "books");
    assert_eq!(start["book"]["timezone"], "America/Denver");
    assert!(!start.to_string().contains("3456789"));
    drop(m);
    // A broken settings file beside books that keep their own can't stop
    // the server: it is never read.
    std::fs::write(dir.join("wanderbooks.toml"), "[books\nthis is not toml").unwrap();
    let mut m = Mcp::spawn(&data);
    let told = init(&mut m);
    assert!(
        told.contains("Settings and conventions live in the books"),
        "{told}"
    );
    assert!(!told.contains("local file you may edit"), "{told}");
    assert!(told.contains("Kept Co LLC"), "{told}");
    assert_eq!(m.call("books.settings", json!({}))["source"], "books");
    assert_eq!(m.call("books.verify", json!({}))["status"], "ok");
}

/// INVARIANT 2 (tools/kept.rs): an import that would read any setting
/// differently than the server does now is refused. Here the server reads
/// its receipt policy from config.json, which the import must carry.
#[test]
fn an_import_that_reads_differently_is_refused() {
    let dir = test_dir("kept-differs");
    let data = dir.join("books-data");
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(dir.join("wanderbooks.toml"), TOML).unwrap();
    let policy = r#"{"receipt_policy": {"require_receipts_above": "10.00", "receipt_exempt_accounts": ["6999"]}}"#;
    std::fs::write(data.join("config.json"), policy).unwrap();
    let mut m = Mcp::spawn(&data);
    init(&mut m);
    let e = m.call_err(
        "books.import_settings",
        json!({"toml_text": TOML, "decided_by": "A", "confirmed": true}),
    );
    assert!(e.contains("policy.evidence.threshold"), "{e}");
    assert!(e.contains("config_json_text"), "{e}");
    assert_eq!(m.call("books.settings", json!({}))["source"], "file");
    m.call(
        "books.import_settings",
        json!({"toml_text": TOML, "decided_by": "A", "confirmed": true,
               "config_json_text": policy}),
    );
    let s = m.call("books.settings", json!({}));
    assert_eq!(s["source"], "books");
    assert_eq!(s["receipt_exempt_accounts"], json!(["6999"]));
}

/// The vault key a settings file names must come from the command line
/// before the settings move in, or the vault would stop opening.
#[test]
fn an_import_needs_the_vault_key_on_the_command_line() {
    let dir = test_dir("kept-vault");
    let data = dir.join("books-data");
    let key = dir.join("elsewhere").join("vault.key");
    let toml = format!("{TOML}\n[vault]\nkey_file = \"{}\"\n", key.display());
    std::fs::write(dir.join("wanderbooks.toml"), &toml).unwrap();
    let mut m = Mcp::spawn(&data);
    init(&mut m);
    let e = m.call_err(
        "books.import_settings",
        json!({"toml_text": toml, "decided_by": "A", "confirmed": true}),
    );
    assert!(e.contains("--vault-key-file"), "{e}");
    drop(m);
    let mut m = Mcp::spawn_with(&data, &["--vault-key-file".as_ref(), key.as_os_str()]);
    init(&mut m);
    m.call(
        "books.import_settings",
        json!({"toml_text": toml, "decided_by": "A", "confirmed": true}),
    );
    assert_eq!(m.call("books.settings", json!({}))["source"], "books");
}

/// Approval rules through the server: enforced, a covered post is refused
/// until its approver approves, then posts; off, they block nothing.
#[test]
fn approval_rules_through_the_server() {
    let dir = test_dir("kept-approvals");
    let data = dir.join("books-data");
    let mut m = Mcp::spawn(&data);
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
    let dir = test_dir("kept-template");
    let data = dir.join("books-data");
    let mut m = Mcp::spawn(&data);
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
    let dir = test_dir("kept-start");
    let data = dir.join("books-data");
    let mut m = Mcp::spawn(&data);
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
}

/// Review of P1 (2026-10-17): the text imported is the file the server
/// reads, whole; books that keep their own settings take no import that
/// changes them; a settings file never turns approvals on; after the
/// move, a broken config.json can't stop the server; and books.settings
/// shows a file's values before the move.
#[test]
fn an_import_is_the_file_the_server_reads() {
    let dir = test_dir("kept-whole");
    let data = dir.join("books-data");
    std::fs::write(dir.join("wanderbooks.toml"), TOML).unwrap();
    let mut m = Mcp::spawn(&data);
    init(&mut m);
    let keys = m.call("books.settings", json!({}))["keys"].clone();
    let ein = keys
        .as_array()
        .unwrap()
        .iter()
        .find(|k| k["key"] == "company.ein")
        .unwrap()
        .clone();
    assert_eq!(ein["value"], "**-***5893", "the file's value, masked");
    // A text that leaves the EIN out is not the file.
    let short = TOML.replace("ein = \"28-8525893\"\n", "");
    let e = m.call_err(
        "books.import_settings",
        json!({"toml_text": short, "decided_by": "A", "confirmed": true}),
    );
    assert!(e.contains("company.ein"), "{e}");
    // Approval rules never come in from a file.
    std::fs::write(
        dir.join("wanderbooks.toml"),
        format!("{TOML}approval_rules = \"enforce\"\n")
            .replace("[viewer]\nopen_at_start = \"ask\"\n", ""),
    )
    .unwrap();
    let with_rules = std::fs::read_to_string(dir.join("wanderbooks.toml")).unwrap();
    let e = m.call_err(
        "books.import_settings",
        json!({"toml_text": with_rules, "decided_by": "A", "confirmed": true}),
    );
    assert!(e.contains("approval"), "{e}");
    std::fs::write(dir.join("wanderbooks.toml"), TOML).unwrap();
    m.call(
        "books.import_settings",
        json!({"toml_text": TOML, "decided_by": "A", "confirmed": true}),
    );
    // Imported books: the same text again is no change; another is refused.
    let again = m.call(
        "books.import_settings",
        json!({"toml_text": TOML, "decided_by": "A", "confirmed": true}),
    );
    assert!(again["changes"].as_array().unwrap().is_empty(), "{again}");
    let e = m.call_err(
        "books.import_settings",
        json!({"toml_text": short, "decided_by": "A", "confirmed": true}),
    );
    assert!(e.contains("books.set_setting"), "{e}");
    drop(m);
    std::fs::write(data.join("config.json"), "{ not json").unwrap();
    let mut m = Mcp::spawn(&data);
    init(&mut m);
    assert_eq!(m.call("books.settings", json!({}))["source"], "books");
}

/// A convention that names an account is listed when that account is
/// renumbered, and as a problem while it names a code no longer in the
/// chart.
#[test]
fn renumbering_names_the_conventions_to_revise() {
    let dir = test_dir("kept-renumber");
    let data = dir.join("books-data");
    let mut m = Mcp::spawn(&data);
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
    let dir = test_dir("kept-resources");
    let data = dir.join("books-data");
    let mut m = Mcp::spawn(&data);
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
    assert_eq!(settings["source"], "file", "{settings}");
    let conventions = read(&mut m, "books://conventions");
    assert_eq!(
        conventions["conventions"][0]["id"], "software",
        "{conventions}"
    );
    drop(m);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A renumber lists exactly the conventions naming the old code.
#[test]
fn renumbering_lists_only_the_conventions_naming_it() {
    let dir = test_dir("kept-renumber-only");
    let data = dir.join("books-data");
    let mut m = Mcp::spawn(&data);
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
    drop(m);
    let _ = std::fs::remove_dir_all(&dir);
}
