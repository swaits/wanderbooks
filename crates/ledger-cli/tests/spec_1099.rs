//! The 1099 decision and payment methods through the server (plan P3,
//! defects 12, 18 and 23; D-40), driven through a real books server as
//! an agent reaches it (common.rs): every success validates against the tool's outputSchema, and
//! every refusal leaves `journal.jsonl` byte-identical (the harness is
//! `tests/spec_parties.rs`'s). Every name is fictional; the reasons are
//! the ones the first company's cutover records.

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
                "clientInfo": {"name": "spec-1099", "version": "0.1.0"},
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

/// Books with the chart; cash is 1000, financing 2100.
fn books(name: &str) -> Mcp {
    books_in(name, false)
}

/// [`books`], as P2 left them when `p2`: journal format 2 with none of
/// P3's features, books an older version wrote, waiting for their format
/// upgrade (rules.md B-27).
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
    let toml = books_toml(&dir, "Spec Co", "financing_accounts = [\"2100\"]\n");
    let mut m = Mcp::start(&dir, &toml);
    m.accounts(CHART);
    m.books_dir = Some(dir);
    m
}

impl Mcp {
    /// The owner's go-ahead, as the app gives it (ledger-gui owner.rs,
    /// after the passphrase and a verified backup; no tool can): the
    /// server stopped, the books moved to this version's format, and
    /// served again.
    fn upgraded(mut self) -> Mcp {
        let dir = self.books_dir.take().expect("books_dir");
        drop(self);
        dir.stop();
        ledger_store::Store::open(&dir.books_data())
            .unwrap()
            .upgrade_format()
            .unwrap()
            .expect("P2's books wait for the upgrade");
        let mut m = Mcp::start(&dir, &dir.join("wanderbooks.toml"));
        m.books_dir = Some(dir);
        m
    }

    /// The same books, served again (a restart).
    fn restart(mut self) -> Mcp {
        let dir = self.books_dir.take().expect("books_dir");
        drop(self);
        let mut m = Mcp::start(&dir, &dir.join("wanderbooks.toml"));
        m.books_dir = Some(dir);
        m
    }
}

/// A vendor's decision as books.list_vendors shows it today.
fn decision(m: &mut Mcp, vendor: &str) -> Value {
    let all = m.read_only("books.list_vendors", json!({}));
    all["vendors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == vendor)
        .unwrap_or_else(|| panic!("{vendor} not listed: {all}"))
        .clone()
}

/// A vendor's line on the 2026 worksheet.
fn line_2026(m: &mut Mcp, vendor: &str) -> Value {
    let w = m.read_only("books.vendor_1099", json!({"year": 2026}));
    w["vendors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["name"] == vendor)
        .unwrap_or_else(|| panic!("{vendor} not on the worksheet: {w}"))
        .clone()
}

/// Defect 23, from its reproduction (P3 brief 1.1): "none" stored the same
/// null as a vendor nobody decided on. Now "none" is refused with what to
/// say instead, not_reportable is kept with its reason and who decided
/// when, a fresh vendor reads undetermined with no one named, and both
/// survive a restart.
#[test]
fn a_vendor_marked_no_1099_is_not_undecided() {
    let mut m = books("decided");
    m.ok("books.add_vendor", json!({"name": "Bigco"}));
    m.ok("books.add_vendor", json!({"name": "Newco"}));
    for none in ["none", "", "None", "no_1099"] {
        let why = m.refuse(
            "books.set_vendor_1099",
            json!({"vendor": "Bigco", "form_1099": none, "decided_by": "Riley Chen"}),
        );
        assert!(
            why.contains("not_reportable") && why.contains("undetermined"),
            "{why}"
        );
        assert!(why.contains("Paid by card is not a reason"), "{why}");
    }
    let v = m.ok(
        "books.set_vendor_1099",
        json!({"vendor": "Bigco", "form_1099": "not_reportable", "reason_kind": "corporation",
               "reason": "a corporation by name", "decided_by": "Riley Chen",
               "decided_on": "2026-10-16", "citation": "Treas. Reg. §1.6041-3(p)(1)"}),
    );
    assert_eq!(v["changed"], "set");
    assert!(v["warnings"].as_array().unwrap().is_empty());
    let check = |m: &mut Mcp| {
        let b = decision(m, "Bigco");
        assert!(b["form_1099"].is_null(), "{b}");
        let d = &b["decision_1099"];
        assert_eq!(d["value"], "not_reportable");
        assert_eq!(d["reason_kind"], "corporation");
        assert_eq!(d["reason"], "a corporation by name");
        assert_eq!(d["authority"], "Treas. Reg. §1.6041-3(p)(1)");
        assert_eq!(d["decided_by"], "Riley Chen");
        assert_eq!(d["decided_on"], "2026-10-16");
        assert_eq!(d["citation"], "Treas. Reg. §1.6041-3(p)(1)");
        assert!(d["effective"].is_null(), "from the beginning");
        assert_eq!(b["decisions_1099"].as_array().unwrap().len(), 1);
        let n = decision(m, "Newco");
        assert_eq!(n["decision_1099"]["value"], "undetermined");
        assert!(n["decision_1099"]["decided_by"].is_null(), "nobody decided");
        assert!(n["decisions_1099"].as_array().unwrap().is_empty());
    };
    check(&mut m);
    assert_eq!(m.count("vendor.1099_set"), 1);
    let mut m = m.restart();
    check(&mut m);
}

/// D-40: the first company's cutover records exactly these decisions (a
/// reason kind and its words), and each comes back as given.
#[test]
fn the_cutovers_reasons_are_recorded_as_given() {
    let mut m = books("cutover");
    for (vendor, kind, reason, citation) in [
        (
            "Northwind Research, PBC",
            "corporation",
            "a corporation (Delaware public benefit corporation, from the public record)",
            None,
        ),
        (
            "Tidewater Hosting, Inc.",
            "corporation",
            "a corporation by name",
            Some("Treas. Reg. §1.6041-3(p)(1)"),
        ),
        (
            "Federal Parcel Service",
            "united_states",
            "the United States",
            Some("Treas. Reg. §1.6041-3(p)(3)"),
        ),
    ] {
        m.ok("books.add_vendor", json!({"name": vendor}));
        let mut args = json!({"vendor": vendor, "form_1099": "not_reportable", "reason_kind": kind,
                              "reason": reason, "decided_by": "Riley Chen", "decided_on": "2026-10-16"});
        if let Some(c) = citation {
            args["citation"] = json!(c);
        }
        m.ok("books.set_vendor_1099", args);
        let d = decision(&mut m, vendor)["decision_1099"].clone();
        assert_eq!(
            (
                d["value"].as_str(),
                d["reason_kind"].as_str(),
                d["reason"].as_str()
            ),
            (Some("not_reportable"), Some(kind), Some(reason))
        );
        assert_eq!(d["citation"].as_str(), citation);
    }
}

/// INVARIANTS 1-3 of tools/instruments.rs: what a decision needs, each
/// refusal saying why and changing nothing.
#[test]
fn set_vendor_1099_says_what_a_decision_needs() {
    let mut m = books("refusals");
    m.ok("books.add_vendor", json!({"name": "Harbor Design LLC"}));
    let v = "Harbor Design LLC";
    let by = "Riley Chen";
    for (args, needle) in [
        (
            json!({"vendor": v, "form_1099": "not_reportable", "reason": "x", "decided_by": by}),
            "reason_kind",
        ),
        (
            json!({"vendor": v, "form_1099": "not_reportable", "reason_kind": "corporation", "decided_by": by}),
            "needs reason",
        ),
        (
            json!({"vendor": v, "form_1099": "not_reportable", "reason_kind": "paid_by_card", "reason": "x", "decided_by": by}),
            "paid by card is not a reason",
        ),
        (
            json!({"vendor": v, "form_1099": "not_reportable", "reason_kind": "other", "reason": "x", "decided_by": by}),
            "citation",
        ),
        (
            json!({"vendor": v, "form_1099": "nec", "reason": "x", "decided_by": by}),
            "go with not_reportable",
        ),
        (json!({"vendor": v, "form_1099": "nec"}), "decided_by"),
        (
            json!({"vendor": v, "form_1099": "nec", "effective": "2026-07-01", "decided_by": by}),
            "from the beginning",
        ),
        (
            json!({"vendor": v, "form_1099": "not_reportable", "reason_kind": "corporation", "reason": "EIN 12-3456789", "decided_by": by}),
            "EIN",
        ),
        (json!({"vendor": v}), "nothing to change"),
        (
            json!({"vendor": v, "reason": "x", "tin_on_file": true}),
            "go with form_1099",
        ),
        (
            json!({"vendor": v, "form_1099": "1099-k", "decided_by": by}),
            "expected nec",
        ),
        (
            json!({"vendor": "Nobody", "form_1099": "nec", "decided_by": by}),
            "vendor",
        ),
    ] {
        let why = m.refuse("books.set_vendor_1099", args.clone());
        assert!(why.contains(needle), "{args}: {why}");
    }
    assert_eq!(m.count("vendor.1099_set"), 0);
    assert_eq!(
        m.count("journal.format_set"),
        1,
        "nothing declared either: only the books' own format, from their creation"
    );
}

/// P3 brief 1.5 and review R8: tin_on_file omitted keeps the W-9 flag;
/// tin_on_file alone keeps the decision and who made it.
#[test]
fn set_vendor_1099_keeps_the_w9_flag_when_omitted() {
    let mut m = books("w9-flag");
    m.ok("books.add_vendor", json!({"name": "Harbor Design LLC"}));
    let v = "Harbor Design LLC";
    m.ok(
        "books.set_vendor_1099",
        json!({"vendor": v, "form_1099": "nec", "tin_on_file": true, "decided_by": "Riley Chen", "decided_on": "2026-03-02"}),
    );
    let out = m.ok(
        "books.set_vendor_1099",
        json!({"vendor": v, "form_1099": "misc", "decided_by": "Riley Chen", "decided_on": "2026-04-01"}),
    );
    assert_eq!(out["tin_on_file"], true, "omitted: kept");
    assert_eq!(out["form_1099"], "1099-MISC");
    let out = m.ok(
        "books.set_vendor_1099",
        json!({"vendor": v, "tin_on_file": false}),
    );
    assert_eq!(out["changed"], "unchanged");
    assert_eq!(out["tin_on_file"], false);
    assert_eq!(out["decision_1099"]["value"], "misc");
    assert_eq!(out["decision_1099"]["decided_on"], "2026-04-01");
    assert_eq!(m.count("vendor.1099_set"), 2);
    assert_eq!(m.count("vendor.w9_flag_set"), 1);
    // Already so: nothing journaled.
    m.ok(
        "books.set_vendor_1099",
        json!({"vendor": v, "tin_on_file": false}),
    );
    assert_eq!(m.count("vendor.w9_flag_set"), 1);
}

/// P3 brief 4.4 and review R8: recording a W-9 (books.set_w9) marks the
/// vendor's W-9 on file and keeps its 1099 decision with who made it and
/// when, verbatim.
#[test]
fn set_w9_keeps_the_1099_decision_and_its_source() {
    let mut m = books("set-w9");
    m.add_party("Harbor Design LLC", &["contractor"]);
    m.ok("books.add_vendor", json!({"name": "Harbor Design LLC"}));
    m.ok(
        "books.set_vendor_1099",
        json!({"vendor": "Harbor Design LLC", "form_1099": "nec", "decided_by": "Riley Chen",
               "decided_on": "2026-01-05", "citation": "W-9 requested"}),
    );
    let before = decision(&mut m, "Harbor Design LLC")["decision_1099"].clone();
    m.ok(
        "books.set_w9",
        json!({"party": "Harbor Design LLC", "received_on": "2026-03-02",
               "entity_type": "LLC (partnership)", "tin_last4": "4321", "kept_at": "CPA portal"}),
    );
    let after = decision(&mut m, "Harbor Design LLC");
    assert_eq!(after["tin_on_file"], true);
    assert_eq!(after["decision_1099"], before);
    assert_eq!(m.count("vendor.1099_set"), 1, "no decision re-journaled");
    assert_eq!(m.count("vendor.w9_flag_set"), 1);
}

/// A member's method on an account the books don't name as financing
/// warns: the worksheet holds its untagged payments undetermined (never a
/// card's), and the settings should say what the account is.
#[test]
fn a_members_method_off_the_financing_accounts_warns() {
    let mut m = books("member-method");
    m.ok(
        "books.add_account",
        json!({"code": "2110", "name": "Due to Riley", "account_type": "liability"}),
    );
    let out = m.ok(
        "books.add_instrument",
        json!({"kind": "member_card", "label": "Riley's Visa", "account": "2110"}),
    );
    let w = out["warnings"].as_array().unwrap();
    assert_eq!(w.len(), 1, "{out}");
    assert!(
        w[0].as_str()
            .unwrap()
            .contains("not one of the books' financing accounts"),
        "{out}"
    );
    let out = m.ok(
        "books.add_instrument",
        json!({"kind": "member_cash_or_check", "label": "Riley's checks", "account": "2100"}),
    );
    assert!(out["warnings"].as_array().unwrap().is_empty(), "{out}");
}

/// B-27 and the format-upgrade gate (D-39): books.set_w9 journals the W-9
/// and then the vendor's W-9 flag, a line of the vendor_1099_decisions
/// feature, so it asks for the feature before the command's first line,
/// never between its lines. In books waiting for their format upgrade
/// (P2's) the whole command is refused, journaling nothing (never the W-9
/// recorded and the flag not); once the owner upgrades them, as in books
/// this version made, it journals the W-9, then the flag, and declares
/// nothing.
#[test]
fn set_w9_asks_for_the_w9_flags_feature_before_its_first_line() {
    let w9 = json!({"party": "Harbor Design LLC", "received_on": "2026-03-02",
                    "entity_type": "LLC (partnership)", "tin_last4": "4321", "kept_at": "CPA portal"});
    let mut m = books_in("set-w9-feature", true);
    m.add_party("Harbor Design LLC", &["contractor"]);
    m.ok("books.add_vendor", json!({"name": "Harbor Design LLC"}));
    let why = m.refuse("books.set_w9", w9.clone());
    assert!(
        why.starts_with("format_upgrade_pending:") && why.contains("vendor_1099_decisions"),
        "{why}"
    );
    assert_eq!(m.count("party.w9_set"), 0, "nothing half done");
    let mut fresh = books("set-w9-feature-new");
    fresh.add_party("Harbor Design LLC", &["contractor"]);
    fresh.ok("books.add_vendor", json!({"name": "Harbor Design LLC"}));
    for mut m in [m.upgraded(), fresh] {
        let before = m.journal_types().len();
        m.ok("books.set_w9", w9.clone());
        let added = m.journal_types()[before..].to_vec();
        assert_eq!(added, ["party.w9_set", "vendor.w9_flag_set"]);
        assert_eq!(decision(&mut m, "Harbor Design LLC")["tin_on_file"], true);
    }
}

/// Defect 12 end to end, restarting between steps (P3 brief 8.2): payment
/// methods added (and refused when unsound), a draft taking one, a posted
/// entry and its reversal tagged together, and the worksheet showing what
/// counts, what a card paid, and what is undetermined.
#[test]
fn payment_methods_round_trip_and_classify_payments() {
    let mut m = books("methods");
    m.add_party("Riley Chen", &["member"]);
    let added = m.ok(
        "books.add_instrument",
        json!({"kind": "bank", "label": "Harbor Bank checking", "last4": "1234", "account": "1000"}),
    );
    assert_eq!(added["card_or_network"], false);
    assert!(added["warnings"].as_array().unwrap().is_empty());
    m.ok(
        "books.add_instrument",
        json!({"kind": "business_card", "label": "Harbor Visa", "last4": "0042", "account": "2050"}),
    );
    let visa = m.ok(
        "books.add_instrument",
        json!({"kind": "member_card", "label": "Riley's Visa", "owner_party": "Riley Chen", "account": "2100"}),
    );
    assert_eq!(visa["card_or_network"], true);
    assert_eq!(visa["account"], "2100");
    m.ok(
        "books.add_instrument",
        json!({"kind": "member_cash_or_check", "label": "Riley's checks", "account": "2100"}),
    );
    for (args, needle) in [
        (
            json!({"kind": "card", "label": "x", "account": "2050"}),
            "kind",
        ),
        (
            json!({"kind": "bank", "label": "Savings", "last4": "12345", "account": "1000"}),
            "four digits",
        ),
        (
            json!({"kind": "bank", "label": "harbor visa", "account": "1000"}),
            "already named",
        ),
        (
            json!({"kind": "bank", "label": "Capital", "account": "3000"}),
            "asset",
        ),
        (
            json!({"kind": "bank", "label": "Nowhere", "account": "9999"}),
            "unknown account",
        ),
    ] {
        let why = m.refuse("books.add_instrument", args.clone());
        assert!(why.contains(needle), "{args}: {why}");
    }
    m.ok("books.add_vendor", json!({"name": "Dana Studio"}));
    m.ok(
        "books.set_vendor_1099",
        json!({"vendor": "Dana Studio", "form_1099": "nec", "decided_by": "Riley Chen"}),
    );
    let dana = json!({"vendor": "Dana Studio", "instrument": "Riley's Visa"});
    let e1 = m.post(
        "2026-05-01",
        dana,
        &[("6000", "debit", "2500.00"), ("2100", "credit", "2500.00")],
    );
    assert_eq!(
        m.read_only("books.get_entry", json!({"entry_id": e1}))["instrument"],
        "Riley's Visa"
    );
    let e2 = m.post(
        "2026-05-02",
        json!({"vendor": "Dana Studio"}),
        &[("6000", "debit", "3000.00"), ("2100", "credit", "3000.00")],
    );
    let e3 = m.post(
        "2026-05-03",
        json!({"vendor": "Dana Studio"}),
        &[("6000", "debit", "400.00"), ("2050", "credit", "400.00")],
    );
    let mut m = m.restart();
    let d = line_2026(&mut m, "Dana Studio");
    assert_eq!(
        (
            &d["paid"],
            &d["excluded_card"],
            &d["undetermined"],
            &d["status"]
        ),
        (
            &json!("0.00"),
            &json!("2900.00"),
            &json!("3000.00"),
            &json!("undetermined")
        )
    );
    let w = m.read_only("books.vendor_1099", json!({"year": 2026}));
    let listed = |k: &str| -> Vec<(u64, String, Value)> {
        w[k].as_array()
            .unwrap()
            .iter()
            .map(|p| {
                (
                    p["entry_id"].as_u64().unwrap(),
                    p["amount"].as_str().unwrap().to_string(),
                    p["instrument"].clone(),
                )
            })
            .collect()
    };
    assert_eq!(
        listed("excluded_card_payments"),
        [
            (e1, "2500.00".into(), json!("Riley's Visa")),
            (e3, "400.00".into(), Value::Null)
        ]
    );
    assert_eq!(
        listed("undetermined_payments"),
        [(e2, "3000.00".into(), Value::Null)]
    );
    assert!(
        w["notes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n.as_str().unwrap().contains("books.set_entry_instrument")),
        "{w}"
    );
    // The member paid e2 by check, and it was reversed: tagging either
    // tags both.
    let rev = m.ok(
        "books.reverse_entry",
        json!({"entry_id": e2, "date": "2026-05-04", "description": "paid twice", "confirmed": true}),
    )["reversal_id"]
        .as_u64()
        .unwrap();
    let out = m.ok(
        "books.set_entry_instrument",
        json!({"entry_id": e2, "instrument": "Riley's checks"}),
    );
    assert_eq!(out["changed"], true);
    assert_eq!(out["instrument"], "Riley's checks");
    assert!(out["warnings"].as_array().unwrap().is_empty());
    let mut m = m.restart();
    assert_eq!(
        m.read_only("books.get_entry", json!({"entry_id": rev}))["instrument"],
        "Riley's checks"
    );
    let d = line_2026(&mut m, "Dana Studio");
    assert_eq!(
        (&d["paid"], &d["undetermined"], &d["status"]),
        (&json!("0.00"), &json!("0.00"), &json!("under_threshold"))
    );
    let all = m.read_only("books.instruments", json!({}));
    let labels: Vec<(&str, bool)> = all["instruments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| {
            (
                i["label"].as_str().unwrap(),
                i["card_or_network"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        labels,
        [
            ("Harbor Bank checking", false),
            ("Harbor Visa", true),
            ("Riley's Visa", true),
            ("Riley's checks", false)
        ]
    );
    // Cleared again: back to undetermined, both of the chain.
    m.ok("books.set_entry_instrument", json!({"entry_id": rev}));
    assert!(m.read_only("books.get_entry", json!({"entry_id": e2}))["instrument"].is_null());
    let p = m.party("Dana Studio", 2026);
    assert_eq!(p["totals"]["decision_1099"], "nec");
    assert_eq!(p["totals"]["form_1099"], "1099-NEC");
}

/// D-40 and P3 review R15: a decision changed for payments in a closed
/// period, or in a year whose 1099s were due (January 31 of the next
/// year), warns, naming the year and the entries; one reaching only this
/// year's open payments doesn't.
#[test]
fn a_change_reaching_a_filed_year_warns() {
    let mut m = books("warns");
    m.ok("books.add_vendor", json!({"name": "Harbor Design LLC"}));
    let v = "Harbor Design LLC";
    m.ok(
        "books.set_vendor_1099",
        json!({"vendor": v, "form_1099": "nec", "decided_by": "Riley Chen"}),
    );
    let harbor = json!({"vendor": v});
    let pay = |m: &mut Mcp, date: &str| {
        m.post(
            date,
            harbor.clone(),
            &[("6000", "debit", "700.00"), ("1000", "credit", "700.00")],
        )
    };
    let e2024 = pay(&mut m, "2024-03-01");
    let e2025 = pay(&mut m, "2025-03-01");
    pay(&mut m, "2026-03-01");
    let period = m.ok(
        "books.create_period",
        json!({"name": "FY2025", "start": "2025-01-01", "end": "2025-12-31"}),
    )["period_id"]
        .as_u64()
        .unwrap();
    m.ok(
        "books.close_period",
        json!({"period_id": period, "confirmed": true}),
    );
    // From July 2026 on: only an open, not-yet-due year's payments: quiet.
    let out = m.ok(
        "books.set_vendor_1099",
        json!({"vendor": v, "form_1099": "not_reportable", "reason_kind": "corporation",
               "reason": "elected to be taxed as a corporation", "effective": "2026-07-01",
               "decided_by": "Riley Chen"}),
    );
    assert!(out["warnings"].as_array().unwrap().is_empty(), "{out}");
    assert_eq!(out["decisions_1099"].as_array().unwrap().len(), 2);
    // From the beginning: reaches 2024 (1099s due 2025-01-31) and the
    // closed 2025.
    let out = m.ok(
        "books.set_vendor_1099",
        json!({"vendor": v, "form_1099": "not_reportable", "reason_kind": "corporation",
               "reason": "a corporation by name", "decided_by": "Riley Chen"}),
    );
    let warnings: Vec<String> = out["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w.as_str().unwrap().to_string())
        .collect();
    assert_eq!(warnings.len(), 2, "{warnings:?}");
    assert!(warnings[0].contains("for 2024") && warnings[0].contains(&format!("entries {e2024}")));
    assert!(warnings[0].contains("due January 31"), "{}", warnings[0]);
    assert!(warnings[1].contains("for 2025") && warnings[1].contains("closed period"));
    assert!(warnings.iter().all(|w| w.contains("corrected")));
    // A payment method on the closed year's payment warns too.
    m.ok(
        "books.add_instrument",
        json!({"kind": "business_card", "label": "Harbor debit card", "account": "1000"}),
    );
    let out = m.ok(
        "books.set_entry_instrument",
        json!({"entry_id": e2025, "instrument": "Harbor debit card"}),
    );
    let w = out["warnings"].as_array().unwrap();
    assert_eq!(w.len(), 1, "{out}");
    assert!(w[0].as_str().unwrap().contains("closed period"));
}
