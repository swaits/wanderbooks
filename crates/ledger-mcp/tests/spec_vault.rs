//! Spec-blind tests for the TIN vault tools of `ledger-mcp`, driven over
//! stdio against the real binary.
//!
//! Expected behavior comes only from the tool descriptions and parameter /
//! result doc comments in `src/server.rs` and `src/types.rs`,
//! `docs/mcp-interface.md` ("Tools — the TIN vault"), and
//! `docs/parties-and-attachments.md` ("The TIN vault"). The rule above all
//! (DECISION, Stephen, 2026-10-15): a full TIN is never in the journal, the
//! SQLite projection, the viewer, or any tool output, except the 1099
//! worksheet for the CPA when asked (`books.vendor_1099` with
//! `include_tins`). The harness is `tests/spec_parties.rs`'s, with a
//! transcript of every response and the server's stderr kept, and the vault
//! key in a temp file outside the books dir (never `~/.config`).

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
        "ledger-mcp-spec-vault-{}-{name}-{n}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
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
                "clientInfo": {"name": "spec-vault", "version": "0.1.0"},
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
    ("2050", "Card", "liability"),
    ("3000", "Capital", "equity"),
    ("6000", "Contract Labor", "expense"),
    ("6999", "General Expenses", "expense"),
];

/// Books with the chart (no receipts needed), a contractor "Acme Design"
/// with a vendor record marked NEC, and a landlord "Rent Co".
fn books(name: &str) -> Mcp {
    let dir = test_dir(name);
    let toml = books_toml(&dir, "1000000.00", "");
    let mut m = Mcp::start(&dir, &toml);
    m.accounts(CHART);
    m.add_party("Acme Design", &["contractor"]);
    m.ok("books.add_vendor", json!({"name": "Acme Design"}));
    m.ok(
        "books.set_vendor_1099",
        json!({"vendor": "Acme Design", "form_1099": "nec"}),
    );
    m.add_party("Rent Co", &["landlord"]);
    m
}

/// An SSN and an EIN no other number in these tests resembles.
const SSN: &str = "571-93-2846";
const SSN_DIGITS: &str = "571932846";
const EIN: &str = "47-3829105";
const EIN_DIGITS: &str = "473829105";

impl Mcp {
    fn books_dir(&self) -> PathBuf {
        self.data.parent().unwrap().to_path_buf()
    }
    fn outside(&self) -> PathBuf {
        outside(&self.books_dir())
    }
    fn vault(&self) -> PathBuf {
        self.books_dir().join("vault")
    }
    fn key(&self) -> PathBuf {
        self.outside().join("vault.key")
    }
    fn party_id(&mut self, name: &str) -> u64 {
        self.party(name, 2026)["party"]["party_id"]
            .as_u64()
            .unwrap()
    }
    /// set_w9 for `party` with these extra arguments.
    fn w9(&mut self, party: &str, extra: Value) -> Out {
        let mut args = json!({"party": party, "received_on": "2026-01-05",
                              "entity_type": "individual/sole proprietor"});
        for (k, v) in extra.as_object().unwrap() {
            args[k] = v.clone();
        }
        self.tool("books.set_w9", args)
    }
    /// A W-9 file outside the books; it shows the TIN, like the real one.
    fn w9_file(&self, name: &str, tin: &str) -> PathBuf {
        let p = self.outside().join(name);
        std::fs::write(&p, format!("%PDF-1.4 Form W-9 TIN {tin}")).unwrap();
        p
    }
    /// Pay `vendor` `amount` from the bank in 2026, tagged.
    fn pay(&mut self, vendor: &str, amount: &str) {
        self.post(
            "2026-03-01",
            json!({"vendor": vendor}),
            &[("6000", "debit", amount), ("1000", "credit", amount)],
        );
    }
    /// The W-9 facts the journal holds for a party, last first.
    fn journaled_w9(&self, party: u64) -> Value {
        String::from_utf8(self.journal())
            .unwrap()
            .lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .filter(|v| v["type"] == "party.w9_set" || v["line_type"] == "party.w9_set")
            .rfind(|v| v["payload"]["id"] == json!(party))
            .map(|v| v["payload"]["facts"].clone())
            .expect("a party.w9_set")
    }
}

/// Whether `hay` holds the nine `digits` in any formatting: separators
/// (space, hyphen, dot, slash, underscore) between them are ignored.
fn holds_tin(hay: &[u8], digits: &str) -> bool {
    let squeezed: Vec<u8> = hay
        .iter()
        .filter(|b| !matches!(b, b' ' | b'-' | b'.' | b'/' | b'_'))
        .copied()
        .collect();
    squeezed
        .windows(digits.len())
        .any(|w| w == digits.as_bytes())
}

/// Every file under `dir`, recursively.
fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return out;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(files_under(&p));
        } else {
            out.push(p);
        }
    }
    out
}

// ------------------------------------------------- books.set_w9 with a TIN

/// The TIN goes to the vault (the key created on first use, owner-only),
/// the books keep its last four and the record's hash, and the output
/// never shows it.
#[test]
fn a_tin_goes_to_the_vault_and_the_books_keep_the_last_four() {
    let mut m = books("store");
    assert!(!m.key().exists());
    let out = m.w9("Acme Design", json!({"tin": SSN, "tin_type": "ssn"}));
    let Out::Ok(p) = out else { panic!("{out:?}") };
    assert_eq!(p["w9"]["tin_last4"], "2846", "from the TIN");
    assert_eq!(p["w9"]["kept_at"], "wanderbooks vault");
    assert_eq!(p["w9"]["in_vault"], true);
    assert!(!holds_tin(p.to_string().as_bytes(), SSN_DIGITS));
    let id = p["party_id"].as_u64().unwrap();
    assert!(m.vault().join(format!("tin-{id}.bin")).exists());
    assert!(m.key().exists(), "the key is created on first use");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(m.key()).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "the key is the owner's only");
    }
    let facts = m.journaled_w9(id);
    assert_eq!(facts["tin_last4"], "2846");
    assert_eq!(facts["vault_record"].as_str().unwrap().len(), 64);
    // The vendor record's W-9 flag follows.
    let v = m.read_only("books.vendor_1099", json!({"year": 2026}));
    assert_eq!(v["vendors"][0]["tin_on_file"], true);
    assert_eq!(v["vendors"][0]["tin_last4"], "2846");
    assert!(v["vendors"][0]["tin"].is_null());
    assert!(v["warning"].is_null());
    // A given tin_last4 that agrees is fine; an EIN for the landlord.
    m.w9(
        "Rent Co",
        json!({"tin": EIN, "tin_type": "ein", "tin_last4": "9105",
                          "kept_at": "binder"}),
    );
    let rent = m.party("Rent Co", 2026);
    assert_eq!(rent["party"]["w9"]["kept_at"], "binder");
    assert_eq!(rent["party"]["w9"]["in_vault"], true);
    assert_eq!(m.ok("books.verify", json!({}))["status"], "ok");
}

#[test]
fn set_w9_refusals_write_nothing_anywhere() {
    let mut m = books("refusals");
    let acme = m.party_id("Acme Design");
    m.add_party("IRS", &["tax_agency"]);
    let missing = m.outside().join("no-such-w9.pdf");
    let txt = m.outside().join("w9.txt");
    std::fs::write(&txt, "W-9").unwrap();
    for (party, extra) in [
        ("Acme Design", json!({"tin": "57193284", "tin_type": "ssn"})), // eight digits
        (
            "Acme Design",
            json!({"tin": "971-93-2846", "tin_type": "ssn"}),
        ), // an SSN starting 9
        (
            "Acme Design",
            json!({"tin": "571932846", "tin_type": "itin"}),
        ), // not an ITIN
        ("Acme Design", json!({"tin": SSN})),                           // no type
        (
            "Acme Design",
            json!({"tin_type": "ssn", "tin_last4": "2846", "kept_at": "x"}),
        ),
        ("Acme Design", json!({"tin": SSN, "tin_type": "passport"})),
        (
            "Acme Design",
            json!({"tin": SSN, "tin_type": "ssn", "tin_last4": "1111"}),
        ),
        (
            "Acme Design",
            json!({"tin": 571932846.0, "tin_type": "ssn"}),
        ), // a float
        (
            "Acme Design",
            json!({"tin": SSN, "tin_type": "ssn", "w9_file_path": missing.to_str().unwrap()}),
        ),
        (
            "Acme Design",
            json!({"tin": SSN, "tin_type": "ssn", "w9_file_path": txt.to_str().unwrap()}),
        ),
        (
            "Acme Design",
            json!({"tin": SSN, "tin_type": "ssn", "clear": true}),
        ),
        (
            "Acme Design",
            json!({"tin": SSN, "tin_type": "ssn", "kept_at": "bad\u{7}"}),
        ),
        ("IRS", json!({"tin": SSN, "tin_type": "ssn"})), // no 1099 role
        ("Nobody", json!({"tin": SSN, "tin_type": "ssn"})),
    ] {
        let out = m.w9(party, extra.clone());
        assert!(out.refused(), "{party} {extra} should be refused: {out:?}");
        assert!(
            !holds_tin(out.err_text().as_bytes(), SSN_DIGITS),
            "a refusal never echoes the TIN: {}",
            out.err_text()
        );
    }
    assert!(
        !m.vault().join(format!("tin-{acme}.bin")).exists(),
        "no record written"
    );
    assert!(!m.key().exists(), "no key made for a refusal");
    assert_eq!(m.count("party.w9_set"), 0);
}

/// Facts that change without a new TIN keep the vault record, unless they
/// disagree with it; a new TIN replaces it.
#[test]
fn the_facts_never_disagree_with_the_vault() {
    let mut m = books("agree");
    let acme = m.party_id("Acme Design");
    m.w9("Acme Design", json!({"tin": SSN, "tin_type": "ssn"}));
    let first = m.journaled_w9(acme)["vault_record"].clone();
    // New facts, same last four: the record stays.
    let Out::Ok(p) = m.w9(
        "Acme Design",
        json!({"tin_last4": "2846", "kept_at": "wanderbooks vault",
                                                "exempt_payee_code": "5"}),
    ) else {
        panic!("same last four")
    };
    assert_eq!(p["w9"]["in_vault"], true);
    assert_eq!(m.journaled_w9(acme)["vault_record"], first);
    // Another last four without the TIN: refused.
    let why = m
        .w9(
            "Acme Design",
            json!({"tin_last4": "1111", "kept_at": "binder"}),
        )
        .err_text();
    assert!(
        why.contains("books.purge_tin") && why.contains("2846"),
        "{why}"
    );
    // A new TIN replaces the old.
    m.w9(
        "Acme Design",
        json!({"tin": "571-93-1111", "tin_type": "ssn"}),
    );
    let facts = m.journaled_w9(acme);
    assert_eq!(facts["tin_last4"], "1111");
    assert_ne!(facts["vault_record"], first);
    m.pay("Acme Design", "700.00");
    let v = m.read_only(
        "books.vendor_1099",
        json!({"year": 2026, "include_tins": true}),
    );
    assert_eq!(v["vendors"][0]["tin"], "571-93-1111");
}

// ------------------------------------------------- the W-9 file

#[test]
fn the_w9_file_goes_to_the_vault_never_the_documents() {
    let mut m = books("w9-file");
    let acme = m.party_id("Acme Design");
    let src = m.w9_file("acme-w9.pdf", SSN);
    let Out::Ok(p) = m.w9(
        "Acme Design",
        json!({"tin": SSN, "tin_type": "ssn", "w9_file_path": src.to_str().unwrap()}),
    ) else {
        panic!("set_w9 with a file")
    };
    assert_eq!(p["w9"]["in_vault"], true);
    assert!(m.vault().join(format!("w9-{acme}.bin")).exists());
    let docs = m.read_only("books.list_receipts", json!({"include_withdrawn": true}));
    assert!(
        docs["receipts"].as_array().unwrap().is_empty(),
        "not a document: {docs}"
    );
    // Read back to a path outside the books: the bytes, owner-only.
    let to = m.outside().join("copy.pdf");
    let out = m.ok(
        "books.get_w9",
        json!({"party": "Acme Design", "save_to": to.to_str().unwrap()}),
    );
    assert_eq!(out["path"], json!(to.display().to_string()));
    assert_eq!(out["file_name"], "acme-w9.pdf");
    assert_eq!(std::fs::read(&to).unwrap(), std::fs::read(&src).unwrap());
    assert!(!holds_tin(out.to_string().as_bytes(), SSN_DIGITS));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&to).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    let books = m.books_dir();
    for save_to in [
        to.clone(),                             // exists
        books.join("w9.pdf"),                   // in the books dir
        m.data.join("w9.pdf"),                  // in the data dir
        books.join("vault/../w9.pdf"),          // in it, by a detour
        m.outside().join("no/such/dir/w9.pdf"), // no folder
    ] {
        m.refuse(
            "books.get_w9",
            json!({"party": "Acme Design", "save_to": save_to.to_str().unwrap()}),
        );
    }
    // A party with no W-9 file in the vault.
    m.w9("Rent Co", json!({"tin": EIN, "tin_type": "ein"}));
    m.refuse(
        "books.get_w9",
        json!({"party": "Rent Co", "save_to": m.outside().join("rent.pdf").to_str().unwrap()}),
    );
    // A W-9 file alone (the TIN typed in later) records the vault too.
    m.add_party("Solo", &["contractor"]);
    let solo = m.w9_file("solo.png", "000");
    let Out::Ok(p) = m.w9(
        "Solo",
        json!({"tin_last4": "0000", "w9_file_path": solo.to_str().unwrap()}),
    ) else {
        panic!("a W-9 file alone")
    };
    assert_eq!(p["w9"]["in_vault"], true);
    assert_eq!(p["w9"]["kept_at"], "wanderbooks vault");
}

#[test]
fn a_locked_vault_says_so_and_never_guesses() {
    let mut m = books("locked");
    let src = m.w9_file("w9.pdf", SSN);
    m.w9(
        "Acme Design",
        json!({"tin": SSN, "tin_type": "ssn", "w9_file_path": src.to_str().unwrap()}),
    );
    m.pay("Acme Design", "700.00");
    let key = std::fs::read(m.key()).unwrap();
    std::fs::remove_file(m.key()).unwrap();
    let why = m.refuse(
        "books.get_w9",
        json!({"party": "Acme Design", "save_to": m.outside().join("x.pdf").to_str().unwrap()}),
    );
    assert!(why.contains("locked"), "{why}");
    let v = m.read_only(
        "books.vendor_1099",
        json!({"year": 2026, "include_tins": true}),
    );
    assert!(v["vendors"][0]["tin"].is_null());
    assert!(
        v["vendors"][0]["tin_note"]
            .as_str()
            .unwrap()
            .contains("locked"),
        "{v}"
    );
    assert!(!m.key().exists(), "reading never makes a key");
    // Another key can't read it: refused, not guessed.
    std::fs::write(m.key(), "11".repeat(32)).unwrap();
    let v = m.read_only(
        "books.vendor_1099",
        json!({"year": 2026, "include_tins": true}),
    );
    assert!(v["vendors"][0]["tin"].is_null());
    assert!(
        v["vendors"][0]["tin_note"]
            .as_str()
            .unwrap()
            .contains("can't be read"),
        "{v}"
    );
    std::fs::write(m.key(), key).unwrap();
    let v = m.read_only(
        "books.vendor_1099",
        json!({"year": 2026, "include_tins": true}),
    );
    assert_eq!(v["vendors"][0]["tin"], SSN);
}

#[test]
fn a_key_file_inside_the_books_is_refused() {
    let dir = test_dir("key-inside");
    let toml = dir.join("wanderbooks.toml");
    std::fs::write(
        &toml,
        format!(
            "[company]\nname = \"Spec Co\"\n[books]\nreceipt_threshold = \"1000000.00\"\n\
             [vault]\nkey_file = \"{}\"\n",
            dir.join("vault.key").display()
        ),
    )
    .unwrap();
    let mut m = Mcp::start(&dir, &toml);
    m.accounts(CHART);
    m.add_party("Acme Design", &["contractor"]);
    let why = m
        .w9("Acme Design", json!({"tin": SSN, "tin_type": "ssn"}))
        .err_text();
    assert!(why.contains("inside the books dir"), "{why}");
    assert!(!dir.join("vault.key").exists());
    assert!(!dir.join("vault").exists());
}

// ------------------------------------------------- books.vendor_1099

#[test]
fn the_1099_worksheet_carries_full_tins_only_when_asked() {
    let mut m = books("worksheet");
    m.w9("Acme Design", json!({"tin": SSN, "tin_type": "ssn"}));
    m.pay("Acme Design", "2500.00");
    m.ok("books.add_vendor", json!({"name": "Rent Co"}));
    m.ok(
        "books.set_vendor_1099",
        json!({"vendor": "Rent Co", "form_1099": "misc"}),
    );
    m.w9(
        "Rent Co",
        json!({"tin_last4": "9105", "kept_at": "CPA portal"}),
    );
    m.pay("Rent Co", "3000.00");
    let plain = m.read_only("books.vendor_1099", json!({"year": 2026}));
    assert!(!holds_tin(plain.to_string().as_bytes(), SSN_DIGITS));
    assert!(plain["warning"].is_null());
    let full = m.read_only(
        "books.vendor_1099",
        json!({"year": 2026, "include_tins": true}),
    );
    assert!(full["warning"].as_str().unwrap().contains("CPA"), "{full}");
    let line = |name: &str| {
        full["vendors"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["name"] == name)
            .unwrap()
            .clone()
    };
    let acme = line("Acme Design");
    assert_eq!(acme["tin"], SSN);
    assert_eq!(acme["tin_type"], "ssn");
    assert!(acme["tin_note"].is_null());
    let rent = line("Rent Co");
    assert!(rent["tin"].is_null());
    assert!(
        rent["tin_note"]
            .as_str()
            .unwrap()
            .contains("not in the vault"),
        "{rent}"
    );
    assert_eq!(rent["tin_last4"], "9105");
    // The CSV goes to a new file outside the books.
    let csv = m.outside().join("1099-2026.csv");
    let out = m.read_only(
        "books.vendor_1099",
        json!({"year": 2026, "include_tins": true, "save_csv_to": csv.to_str().unwrap()}),
    );
    assert_eq!(out["saved_csv"], json!(csv.display().to_string()));
    let text = std::fs::read_to_string(&csv).unwrap();
    assert!(text.contains(SSN), "{text}");
    assert!(text.starts_with("vendor_id,name,form_1099,paid,reportable,w9_on_file,tin_last4,tin"));
    let without = m.outside().join("1099-plain.csv");
    m.read_only(
        "books.vendor_1099",
        json!({"year": 2026, "save_csv_to": without.to_str().unwrap()}),
    );
    assert!(!holds_tin(&std::fs::read(&without).unwrap(), SSN_DIGITS));
    for to in [
        csv.clone(),
        m.books_dir().join("1099.csv"),
        m.data.join("1099.csv"),
    ] {
        m.refuse(
            "books.vendor_1099",
            json!({"year": 2026, "include_tins": true, "save_csv_to": to.to_str().unwrap()}),
        );
    }
    assert!(!m.books_dir().join("1099.csv").exists());
}

// ------------------------------------------------- books.purge_tin

#[test]
fn purge_deletes_the_vault_records_and_journals_why() {
    let mut m = books("purge");
    let acme = m.party_id("Acme Design");
    let src = m.w9_file("w9.pdf", SSN);
    m.w9(
        "Acme Design",
        json!({"tin": SSN, "tin_type": "ssn", "w9_file_path": src.to_str().unwrap()}),
    );
    // Clearing while the vault holds it: refused.
    let why = m
        .tool(
            "books.set_w9",
            json!({"party": "Acme Design", "clear": true}),
        )
        .err_text();
    assert!(why.contains("books.purge_tin"), "{why}");
    m.refuse(
        "books.purge_tin",
        json!({"party": "Acme Design", "reason": "  ", "confirmed": true}),
    );
    m.refuse(
        "books.purge_tin",
        json!({"party": "Nobody", "reason": "x", "confirmed": true}),
    );
    // A preview changes nothing.
    let journal = m.journal();
    let p = m.ok(
        "books.purge_tin",
        json!({"party": "Acme Design", "reason": "retention over"}),
    );
    assert_eq!(p["status"], "preview");
    assert_eq!(p["tin_in_vault"], true);
    assert_eq!(p["w9_file_in_vault"], true);
    assert_eq!(p["tin_last4"], "2846");
    assert!(m.journal() == journal);
    assert!(m.vault().join(format!("tin-{acme}.bin")).exists());
    // Without the key, confirmed: gone, and journaled.
    std::fs::remove_file(m.key()).unwrap();
    let p = m.ok(
        "books.purge_tin",
        json!({"party": "Acme Design", "reason": "retention over", "confirmed": true}),
    );
    assert_eq!(p["status"], "purged");
    assert_eq!(p["journaled"], true);
    assert!(!m.vault().join(format!("tin-{acme}.bin")).exists());
    assert!(!m.vault().join(format!("w9-{acme}.bin")).exists());
    let facts = m.journaled_w9(acme);
    assert!(facts["vault_record"].is_null(), "{facts}");
    assert_eq!(facts["tin_last4"], "2846", "the last four stay");
    assert!(
        facts["kept_at"]
            .as_str()
            .unwrap()
            .contains("retention over"),
        "{facts}"
    );
    assert_eq!(
        m.party("Acme Design", 2026)["party"]["w9"]["in_vault"],
        false
    );
    // Nothing left: refused. Clearing works now.
    m.refuse(
        "books.purge_tin",
        json!({"party": "Acme Design", "reason": "again", "confirmed": true}),
    );
    m.ok(
        "books.set_w9",
        json!({"party": "Acme Design", "clear": true}),
    );
    assert_eq!(m.ok("books.verify", json!({}))["status"], "ok");
}

// ------------------------------------------------- the rule above all

/// A session that stores TINs (an SSN and an EIN, and a W-9 file showing
/// one) and then uses the books every way it can: no tool output, error,
/// or log, and no byte of the journal, the database, the stored files, or
/// the viewer's requests holds either TIN in any formatting, except
/// books.vendor_1099 with include_tins. The vault holds them only
/// encrypted.
#[test]
fn no_full_tin_leaves_the_vault_except_the_worksheet_when_asked() {
    let mut m = books("no-leak");
    let src = m.w9_file("acme-w9.pdf", SSN);
    m.w9(
        "Acme Design",
        json!({"tin": SSN, "tin_type": "ssn", "w9_file_path": src.to_str().unwrap()}),
    );
    m.w9("Rent Co", json!({"tin": EIN, "tin_type": "ein"}));
    m.ok("books.add_vendor", json!({"name": "Rent Co"}));
    m.ok(
        "books.set_vendor_1099",
        json!({"vendor": "Rent Co", "form_1099": "misc"}),
    );
    m.pay("Acme Design", "2500.00");
    m.pay("Rent Co", "3000.00");
    let e = m.post(
        "2026-03-02",
        json!({"party": "Acme Design"}),
        &[("6999", "debit", "1.00"), ("1000", "credit", "1.00")],
    );
    // Refusals, which must not echo what they were given.
    for args in [
        json!({"party": "Acme Design", "received_on": "2026-01-05", "entity_type": "x",
               "tin": SSN, "tin_type": "ein", "tin_last4": "0000"}),
        json!({"party": "Acme Design", "received_on": "2026-01-05", "entity_type": "x",
               "tin": EIN, "tin_type": "ssn", "tin_last4": "2846"}),
        json!({"party": "Acme Design", "received_on": "2026-01-05", "entity_type": "x",
               "tin": SSN_DIGITS.parse::<u64>().unwrap() as f64, "tin_type": "ssn"}),
        json!({"party": "Acme Design", "clear": true}),
    ] {
        m.refuse("books.set_w9", args);
    }
    // A TIN typed into another field is refused, and not echoed.
    m.refuse(
        "books.update_party",
        json!({"party": "Acme Design", "notes": format!("ssn {SSN}")}),
    );
    m.refuse("books.add_party", json!({"name": format!("Acme {EIN}")}));
    // Everything else, read and write.
    for (tool, args) in [
        ("books.parties", json!({})),
        ("books.parties", json!({"role": "contractor", "year": 2026})),
        ("books.party", json!({"party": "Acme Design"})),
        ("books.party", json!({"party": "Rent Co", "year": 2026})),
        ("books.list_vendors", json!({})),
        ("books.vendor_1099", json!({"year": 2026})),
        ("books.get_entry", json!({"entry_id": e})),
        ("books.search_entries", json!({"text": "Acme"})),
        ("books.digest", json!({"since": "2026-01-01"})),
        ("books.trial_balance", json!({})),
        ("books.list_receipts", json!({"include_withdrawn": true})),
        (
            "books.attachments",
            json!({"subject": {"kind": "party", "id": "Acme Design"}}),
        ),
        ("books.settings", json!({})),
        ("books.verify", json!({})),
        ("books.show", json!({"party": "Acme Design"})),
        (
            "books.export",
            json!({"report": "journal", "format": "csv"}),
        ),
        (
            "books.export",
            json!({"report": "journal", "format": "json"}),
        ),
        (
            "books.export_cpa_package",
            json!({"as_of": "2026-12-31", "cash_accounts": ["1000"]}),
        ),
        (
            "books.purge_tin",
            json!({"party": "Rent Co", "reason": "preview only"}),
        ),
        (
            "books.get_w9",
            json!({"party": "Acme Design",
                                "save_to": m.outside().join("for-cpa.pdf").to_str().unwrap()}),
        ),
        (
            "books.update_party",
            json!({"party": "Acme Design", "notes": "W-9 in the vault"}),
        ),
    ] {
        m.ok(tool, args);
    }
    let full = m.read_only(
        "books.vendor_1099",
        json!({"year": 2026, "include_tins": true}),
    );
    assert!(
        holds_tin(full.to_string().as_bytes(), SSN_DIGITS),
        "the worksheet, asked"
    );
    assert!(holds_tin(full.to_string().as_bytes(), EIN_DIGITS));
    let csv = m.outside().join("w.csv");
    m.read_only(
        "books.vendor_1099",
        json!({"year": 2026, "include_tins": true, "save_csv_to": csv.to_str().unwrap()}),
    );
    // Every response but the worksheet's with include_tins.
    let leaks: Vec<&(String, String)> = m
        .transcript
        .iter()
        .filter(|(tool, resp)| {
            let worksheet =
                tool == "books.vendor_1099" && resp.contains("\"warning\":\"This worksheet");
            !worksheet
                && (holds_tin(resp.as_bytes(), SSN_DIGITS)
                    || holds_tin(resp.as_bytes(), EIN_DIGITS))
        })
        .collect();
    assert!(leaks.is_empty(), "full TINs in tool output: {leaks:?}");
    assert!(m.transcript.len() > 40, "the session used the books");
    // The server's log.
    let log = std::fs::read(m.outside().join("stderr.log")).unwrap();
    assert!(!holds_tin(&log, SSN_DIGITS) && !holds_tin(&log, EIN_DIGITS));
    // Every byte in the books dir outside the vault: the journal, the
    // database (and its WAL), the stored files, the settings.
    let books = m.books_dir();
    let mut seen = Vec::new();
    for f in files_under(&books) {
        if f.starts_with(m.vault()) {
            continue;
        }
        let bytes = std::fs::read(&f).unwrap();
        assert!(
            !holds_tin(&bytes, SSN_DIGITS) && !holds_tin(&bytes, EIN_DIGITS),
            "{} holds a full TIN",
            f.display()
        );
        seen.push(f.file_name().unwrap().to_string_lossy().into_owned());
    }
    for must in ["journal.jsonl", "wanderbooks.db"] {
        assert!(seen.iter().any(|s| s == must), "{must} scanned: {seen:?}");
    }
    // The vault holds them encrypted only.
    let vault_files = files_under(&m.vault());
    assert!(vault_files.len() >= 3, "{vault_files:?}");
    for f in vault_files {
        let bytes = std::fs::read(&f).unwrap();
        assert!(
            !holds_tin(&bytes, SSN_DIGITS) && !holds_tin(&bytes, EIN_DIGITS),
            "{} holds a TIN in the clear",
            f.display()
        );
    }
}

/// Books kept in git must ignore the vault before a TIN is stored.
#[test]
fn books_in_git_must_ignore_the_vault() {
    let mut m = books("git");
    let books = m.books_dir();
    std::fs::create_dir_all(books.join(".git")).unwrap();
    std::fs::write(books.join(".gitignore"), "books-data/wanderbooks.db\n").unwrap();
    let why = m
        .w9("Acme Design", json!({"tin": SSN, "tin_type": "ssn"}))
        .err_text();
    assert!(
        why.contains(".gitignore") && why.contains("vault/"),
        "{why}"
    );
    assert!(!m.vault().exists() && !m.key().exists());
    std::fs::write(
        books.join(".gitignore"),
        "books-data/wanderbooks.db\nvault/\n",
    )
    .unwrap();
    assert!(
        !m.w9("Acme Design", json!({"tin": SSN, "tin_type": "ssn"}))
            .refused()
    );
}
