//! P3 defects 14 and 24 through the MCP, through a real books server as
//! an agent reaches it (common.rs): each party's part of an entry
//! (`books.get_entry` `resolved`, `direction`, `cash`, `in_kind`), party
//! totals on a basis (`books.party`, `books.parties` `basis`), and a
//! party named the creditor of a liability (`books.update_party`
//! `creditor_of`).
//!
//! Expected behavior comes only from the tool descriptions and parameter /
//! result doc comments in `src/server.rs` and `src/types.rs` (as
//! `tools/list` serves them) and rules.md B-35. The harness is the minimal
//! subset of `tests/spec_mcp.rs` (copied from `tests/spec_parties.rs`):
//! every success validates against the tool's `outputSchema`, and every
//! refusal leaves `journal.jsonl` byte-identical.

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
                "clientInfo": {"name": "spec-party-totals", "version": "0.1.0"},
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

    /// How many journal lines have this type.
    fn count(&self, line_type: &str) -> usize {
        self.journal_types()
            .iter()
            .filter(|t| t.as_str() == line_type)
            .count()
    }
}

const CHART: &[(&str, &str, &str)] = &[
    ("1000", "Checking", "asset"),
    ("1300", "Prepaid Expenses", "asset"),
    ("1500", "Equipment", "asset"),
    ("2050", "Business Card", "liability"),
    ("2100", "Due to Members", "liability"),
    ("3000", "Capital — Tobin Harlow", "equity"),
    ("3001", "Capital — Morgan Ellis", "equity"),
    ("6050", "Domain & Hosting", "expense"),
    ("6400", "Bank Fees", "expense"),
];

/// Fernhill Labs LLC: two members 50/50, cash 1000, the members'
/// financing 2100; the chart; both members and the card's issuer in the
/// directory.
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
    let toml = books_toml(
        &dir,
        "Fernhill Labs LLC",
        "financing_accounts = [\"2100\"]\n\
         [[company.members]]\nname = \"Tobin Harlow\"\nshare = \"50%\"\ncapital_account = \"3000\"\n\
         [[company.members]]\nname = \"Morgan Ellis\"\nshare = \"50%\"\ncapital_account = \"3001\"\n",
    );
    let mut m = Mcp::start(&dir, &toml);
    m.accounts(CHART);
    m.add_party("Tobin Harlow", &["member"]);
    m.add_party("Morgan Ellis", &["member"]);
    m.add_party("Harbor Bank", &["financial_institution"]);
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
}

/// A party's part of an entry as books.get_entry gives it: (resolved,
/// direction, cash, in_kind).
fn part(m: &mut Mcp, entry: u64, party: &str) -> (Value, Value, Value, Value) {
    let e = m.read_only("books.get_entry", json!({"entry_id": entry}));
    let p = e["parties"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["party"] == party)
        .unwrap_or_else(|| panic!("{party} on entry {entry}: {e}"))
        .clone();
    (
        p["resolved"].clone(),
        p["direction"].clone(),
        p["cash"].clone(),
        p["in_kind"].clone(),
    )
}

/// (paid, received, basis) from books.party.
fn totals(m: &mut Mcp, party: &str, basis: Option<&str>) -> (Value, Value, Value) {
    let mut args = json!({"party": party, "year": 2026});
    if let Some(b) = basis {
        args["basis"] = json!(b);
    }
    let p = m.read_only("books.party", args);
    let t = &p["totals"];
    (t["paid"].clone(), t["received"].clone(), t["basis"].clone())
}

fn reverse(m: &mut Mcp, entry: u64, date: &str) -> u64 {
    m.ok(
        "books.reverse_entry",
        json!({"entry_id": entry, "date": date, "description": "reversed", "confirmed": true}),
    )["reversal_id"]
        .as_u64()
        .unwrap_or_else(|| panic!("a reversal of {entry}"))
}

/// Defect 14 over the MCP (rules.md B-35): `resolved` is signed as a line
/// on the party's own account, with its direction and its money and
/// property; a reversal is the exact opposite; party totals take a basis,
/// cash by default and echoed, all adding property in kind, so that a
/// member's basis-all received is the statement of members' equity's
/// contributions; any other basis is refused.
#[test]
fn defect14_party_totals_take_a_basis() {
    let mut m = books("defect14");
    let tobin = json!({"parties": [{"party": "Tobin Harlow"}]});
    let both = json!({"parties": [{"party": "Tobin Harlow"}, {"party": "Morgan Ellis"}]});
    // Cash in, 400.00 each.
    let cash_in = m.post(
        "2026-10-05",
        tobin.clone(),
        &[("1000", "debit", "400.00"), ("3000", "credit", "400.00")],
    );
    m.post(
        "2026-10-05",
        json!({"parties": [{"party": "Morgan Ellis"}]}),
        &[("1000", "debit", "400.00"), ("3001", "credit", "400.00")],
    );
    // A laptop Tobin contributes, then its reversal.
    let laptop = m.post(
        "2026-10-12",
        tobin.clone(),
        &[("1500", "debit", "1480.00"), ("3000", "credit", "1480.00")],
    );
    let undo = reverse(&mut m, laptop, "2026-10-13");
    // Devices from both, derived from the capital lines; one with its
    // service contract (prepaid: property).
    m.post(
        "2026-10-14",
        both.clone(),
        &[
            ("1500", "debit", "1480.00"),
            ("3000", "credit", "740.00"),
            ("3001", "credit", "740.00"),
        ],
    );
    m.post(
        "2026-10-19",
        both.clone(),
        &[
            ("1500", "debit", "1010.00"),
            ("1300", "debit", "30.00"),
            ("3000", "credit", "520.00"),
            ("3001", "credit", "520.00"),
        ],
    );
    // A reimbursement to both (money, paid).
    let reimburse = m.post(
        "2026-10-20",
        json!({"parties": [{"party": "Tobin Harlow", "amount": "93.25"}, {"party": "Morgan Ellis", "amount": "93.25"}]}),
        &[("2100", "debit", "186.50"), ("1000", "credit", "186.50")],
    );
    assert_eq!(
        part(&mut m, cash_in, "Tobin Harlow"),
        (
            json!("-400.00"),
            json!("received"),
            json!("-400.00"),
            json!("0.00")
        )
    );
    assert_eq!(
        part(&mut m, laptop, "Tobin Harlow"),
        (
            json!("-1480.00"),
            json!("received"),
            json!("0.00"),
            json!("-1480.00")
        )
    );
    assert_eq!(
        part(&mut m, undo, "Tobin Harlow"),
        (
            json!("1480.00"),
            json!("paid"),
            json!("0.00"),
            json!("1480.00")
        )
    );
    assert_eq!(
        part(&mut m, reimburse, "Morgan Ellis"),
        (json!("93.25"), json!("paid"), json!("93.25"), json!("0.00"))
    );
    // Totals: cash by default, labelled; all adds 740.00 + 520.00.
    let cash = (json!("93.25"), json!("400.00"), json!("cash"));
    assert_eq!(totals(&mut m, "Tobin Harlow", None), cash);
    assert_eq!(totals(&mut m, "Tobin Harlow", Some("cash")), cash);
    let all = totals(&mut m, "Tobin Harlow", Some("all"));
    assert_eq!(all, (json!("93.25"), json!("1660.00"), json!("all")));
    // The statement of members' equity, the same dates: contributions
    // 1,660.00 each (the laptop and its reversal cancel), no
    // distributions; basis all's received is exactly that.
    let me = m.read_only(
        "books.export",
        json!({"report": "members_equity", "format": "json", "start": "2026-01-01", "end": "2026-12-31"}),
    );
    let me: Value = serde_json::from_str(&m.wire.fetch_text(&me["file"])).unwrap();
    for who in ["Tobin Harlow", "Morgan Ellis"] {
        let col = me["members"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == who)
            .unwrap_or_else(|| panic!("{who} in {me}"))
            .clone();
        assert_eq!(col["contributions"], totals(&mut m, who, Some("all")).1);
        assert_eq!(col["contributions"], "1660.00");
        assert_eq!(col["distributions"], "0.00");
    }
    // books.parties echoes the basis.
    let list = m.read_only("books.parties", json!({"year": 2026, "basis": "all"}));
    assert_eq!(list["basis"], "all");
    let row = list["parties"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "Morgan Ellis")
        .unwrap()
        .clone();
    assert_eq!(
        (row["paid"].clone(), row["received"].clone()),
        (json!("93.25"), json!("1660.00"))
    );
    assert_eq!(m.read_only("books.parties", json!({}))["basis"], "cash");
    // Any other basis is refused, by both tools.
    for (tool, args) in [
        (
            "books.party",
            json!({"party": "Tobin Harlow", "basis": "accrual"}),
        ),
        ("books.parties", json!({"basis": "accrual"})),
    ] {
        let why = m.refuse(tool, args);
        assert!(why.contains("\"cash\" or \"all\""), "{tool}: {why}");
    }
}

/// Defect 24 over the MCP: `books.update_party` `creditor_of` names the
/// liabilities the company owes a party itself on (journaled as
/// party.creditor_set, under the journal feature the books declared when
/// they were created, so nothing is declared on the way), replaces
/// the list, refuses what isn't a liability with nothing journaled; and a
/// fee on the issuer's own card then reads neither, so the issuer is paid
/// what the card payment paid, once.
#[test]
fn defect24_creditor_of_round_trips_and_a_card_fee_is_not_a_payment() {
    let mut m = books("defect24");
    let issuer = json!({"parties": [{"party": "Harbor Bank"}]});
    let fee = m.post(
        "2026-05-01",
        issuer.clone(),
        &[("6400", "debit", "25.00"), ("2050", "credit", "25.00")],
    );
    m.post(
        "2026-05-20",
        issuer.clone(),
        &[("2050", "debit", "125.00"), ("1000", "credit", "125.00")],
    );
    assert_eq!(totals(&mut m, "Harbor Bank", None).0, json!("150.00"));
    // Not a liability: refused, nothing journaled (the tool checks).
    let why = m.refuse(
        "books.update_party",
        json!({"party": "Harbor Bank", "creditor_of": ["1000"]}),
    );
    assert!(why.contains("liability"), "{why}");
    assert_eq!(m.count("party.creditor_set"), 0);
    let before = m.count("journal.format_set");
    let p = m.ok(
        "books.update_party",
        json!({"party": "Harbor Bank", "creditor_of": ["2050"]}),
    );
    assert_eq!(p["creditor_of"], json!(["2050"]));
    assert_eq!(m.count("party.creditor_set"), 1);
    assert_eq!(
        m.count("journal.format_set"),
        before,
        "nothing declared on the way"
    );
    // The same list again changes nothing.
    let journal = m.journal();
    m.ok(
        "books.update_party",
        json!({"party": "Harbor Bank", "creditor_of": ["2050"]}),
    );
    assert!(m.journal() == journal);
    assert_eq!(
        part(&mut m, fee, "Harbor Bank"),
        (
            json!("-25.00"),
            json!("neither"),
            json!("0.00"),
            json!("0.00")
        )
    );
    assert_eq!(totals(&mut m, "Harbor Bank", None).0, json!("125.00"));
    assert_eq!(
        totals(&mut m, "Harbor Bank", Some("all")).0,
        json!("125.00")
    );
    // Both members are owed what the members advanced; then one isn't.
    for who in ["Tobin Harlow", "Morgan Ellis"] {
        let p = m.ok(
            "books.update_party",
            json!({"party": who, "creditor_of": ["2100"]}),
        );
        assert_eq!(p["creditor_of"], json!(["2100"]));
    }
    let p = m.ok(
        "books.update_party",
        json!({"party": "Morgan Ellis", "creditor_of": []}),
    );
    assert_eq!(p["creditor_of"], json!([]));
    assert_eq!(m.count("party.creditor_set"), 4);
    assert_eq!(m.count("journal.format_set"), before);
    let p = m.read_only("books.party", json!({"party": "Tobin Harlow"}));
    assert_eq!(p["party"]["creditor_of"], json!(["2100"]));
    assert_eq!(m.ok("books.verify", json!({}))["status"], "ok");
}

/// rules.md B-27: `books.update_party` asks for the `party_creditors`
/// feature before the first line it writes, never between them. In books
/// waiting for their format upgrade (P2's), a name change with the first
/// creditor is refused whole, journaling nothing (never the notes changed
/// and the creditor not); once the owner upgrades them, as in books this
/// version made, it journals the change, then the creditor, and declares
/// nothing.
#[test]
fn update_party_asks_for_the_creditor_feature_before_its_first_line() {
    let change =
        json!({"party": "Harbor Bank", "notes": "the card's issuer", "creditor_of": ["2050"]});
    let mut m = books_in("feature-first", true);
    let why = m.refuse("books.update_party", change.clone());
    assert!(
        why.starts_with("format_upgrade_pending:") && why.contains("party_creditors"),
        "{why}"
    );
    assert_eq!(m.count("party.updated"), 0, "nothing half done");
    for mut m in [m.upgraded(), books("feature-first-new")] {
        let start = m.journal_types().len();
        let p = m.ok("books.update_party", change.clone());
        assert_eq!(p["creditor_of"], json!(["2050"]));
        assert_eq!(
            m.journal_types()[start..].to_vec(),
            ["party.updated", "party.creditor_set"],
        );
        assert_eq!(m.ok("books.verify", json!({}))["status"], "ok");
    }
}

/// What `books.post_entry` previews is what it posts: each party's
/// `resolved`, `direction`, `cash` and `in_kind` in the preview are
/// `books.get_entry`'s once confirmed (a derived split is then journaled as
/// the amounts it resolved to, the same parts), for a joint contribution
/// of money and property and for a distribution partly still owed.
#[test]
fn the_post_preview_shows_the_parts_it_posts() {
    let mut m = books("preview");
    m.accounts(&[("2150", "Distributions Payable", "liability")]);
    let both = json!({"parties": [{"party": "Tobin Harlow"}, {"party": "Morgan Ellis"}]});
    let tobin = json!({"parties": [{"party": "Tobin Harlow"}]});
    let parts = |v: &Value| -> Vec<Value> {
        v["parties"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| {
                json!([
                    p["party"],
                    p["resolved"],
                    p["direction"],
                    p["cash"],
                    p["in_kind"]
                ])
            })
            .collect()
    };
    let joint = m.draft(
        "2026-03-02",
        both,
        &[
            ("1000", "debit", "610.00"),
            ("1500", "debit", "390.00"),
            ("3000", "credit", "500.00"),
            ("3001", "credit", "500.00"),
        ],
    );
    let owed = m.draft(
        "2026-03-03",
        tobin,
        &[
            ("3000", "debit", "300.00"),
            ("1000", "credit", "120.00"),
            ("2150", "credit", "180.00"),
        ],
    );
    for (id, want) in [
        (
            joint,
            vec![
                json!(["Tobin Harlow", "-500.00", "received", "-305.00", "-195.00"]),
                json!(["Morgan Ellis", "-500.00", "received", "-305.00", "-195.00"]),
            ],
        ),
        (
            owed,
            vec![json!(["Tobin Harlow", "300.00", "paid", "120.00", "0.00"])],
        ),
    ] {
        let preview = m.read_only("books.post_entry", json!({"entry_id": id}));
        assert_eq!(preview["status"], "preview");
        assert_eq!(parts(&preview), want, "entry {id}'s preview");
        m.ok(
            "books.post_entry",
            json!({"entry_id": id, "confirmed": true}),
        );
        let posted = m.read_only("books.get_entry", json!({"entry_id": id}));
        assert_eq!(parts(&posted), want, "entry {id} posted");
    }
    let (paid, received, _) = totals(&mut m, "Tobin Harlow", None);
    assert_eq!((paid, received), (json!("120.00"), json!("305.00")));
}
