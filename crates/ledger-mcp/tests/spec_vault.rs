//! Spec-blind tests for the TIN vault tools of `ledger-mcp`, driven over
//! stdio against the real binary.
//!
//! Expected behavior comes only from the tool descriptions and parameter /
//! result doc comments in `src/server.rs` and `src/types.rs`,
//! `docs/mcp-interface.md` ("Tools — the TIN vault"), and
//! `docs/parties-and-attachments.md` ("The TIN vault"). The rule above all
//! (DECISION, owner, 2026-10-15): a full TIN is never in the journal, the
//! SQLite projection, the viewer, or any tool output, except the 1099
//! worksheet for the CPA when asked (`books.vendor_1099` with
//! `include_tins`); and the vault holds everyone's, members included, for
//! the CPA package, which carries them only in a zip saved outside the
//! books dir (DECISION, owner, 2026-10-15: "In the vault, that's the
//! WHOLE reason we built it. And it needs to be in the damn CPA
//! package."). The harness is `tests/spec_parties.rs`'s, with a
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

/// A test's dir and the folders beside it (`-inbox`, `-outside`),
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
        for beside in ["-inbox", "-outside"] {
            let p = PathBuf::from(format!("{}{beside}", self.0.display()));
            let _ = std::fs::remove_dir_all(&p);
            let _ = std::fs::remove_file(&p);
        }
    }
}

fn test_dir(name: &str) -> TestDir {
    let n = DIR_SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "ledger-mcp-spec-vault-{}-{name}-{n}",
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

/// The server's import root, beside the books dir `dir`: W-9 files handed
/// to books.set_w9 by path. Never where the server writes TINs.
fn inbox(dir: &Path) -> PathBuf {
    let d = PathBuf::from(format!("{}-inbox", dir.display()));
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
            // A settings file naming the usual key outside the books: the
            // command line names it too, as books that move their settings
            // in must (a test naming another key keeps only the file's).
            .args({
                let key = outside(dir).join("vault.key");
                std::fs::read_to_string(toml)
                    .is_ok_and(|t| t.contains(&format!("key_file = \"{}\"", key.display())))
                    .then(|| {
                        [
                            std::ffi::OsString::from("--vault-key-file"),
                            key.into_os_string(),
                        ]
                    })
                    .into_iter()
                    .flatten()
            })
            // Files handed over by path come only from here.
            .arg("--import-root")
            .arg(inbox(dir))
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
    books_with(name, "")
}

/// `books`, with `extra` TOML after `[books]` (members, say).
fn books_with(name: &str, extra: &str) -> Mcp {
    let dir = test_dir(name);
    let toml = books_toml(&dir, "1000000.00", extra);
    let mut m = Mcp::start(&dir, &toml);
    m.accounts(CHART);
    m.add_party("Acme Design", &["contractor"]);
    m.ok("books.add_vendor", json!({"name": "Acme Design"}));
    m.ok(
        "books.set_vendor_1099",
        json!({"vendor": "Acme Design", "form_1099": "nec"}),
    );
    m.add_party("Rent Co", &["landlord"]);
    m.books_dir = Some(dir);
    m
}

/// An SSN and an EIN no other number in these tests resembles.
const SSN: &str = "291-12-2846";
const SSN_DIGITS: &str = "291122846";
const EIN: &str = "12-3279105";
const EIN_DIGITS: &str = "123279105";
/// A member's SSN, unlike the others.
const MEMBER_SSN: &str = "241-40-4915";
const MEMBER_SSN_DIGITS: &str = "241404915";

/// Every file in a zip, by name.
fn unzip(path: &Path) -> HashMap<String, Vec<u8>> {
    let mut z = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    let mut out = HashMap::new();
    for i in 0..z.len() {
        let mut f = z.by_index(i).unwrap();
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut f, &mut bytes).unwrap();
        out.insert(f.name().unwrap().to_string(), bytes);
    }
    out
}

/// A CSV's rows (header first) as maps from column name to cell, cells
/// split on commas outside double quotes.
fn csv_rows(bytes: &[u8]) -> Vec<HashMap<String, String>> {
    let split = |l: &str| {
        let (mut cells, mut cur, mut quoted) = (Vec::new(), String::new(), false);
        let mut chars = l.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '"' if quoted && chars.peek() == Some(&'"') => {
                    cur.push('"');
                    chars.next();
                }
                '"' => quoted = !quoted,
                ',' if !quoted => cells.push(std::mem::take(&mut cur)),
                c => cur.push(c),
            }
        }
        cells.push(cur);
        cells
    };
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    let mut lines = text.lines();
    let head = split(lines.next().expect("a header"));
    lines
        .map(|l| {
            let cells = split(l);
            assert_eq!(cells.len(), head.len(), "{l}");
            head.iter().cloned().zip(cells).collect()
        })
        .collect()
}

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
        let p = inbox(&self.books_dir()).join(name);
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
    let missing = inbox(&m.books_dir()).join("no-such-w9.pdf");
    let txt = inbox(&m.books_dir()).join("w9.txt");
    std::fs::write(&txt, "W-9").unwrap();
    for (party, extra) in [
        ("Acme Design", json!({"tin": "57193284", "tin_type": "ssn"})), // eight digits
        (
            "Acme Design",
            json!({"tin": "902-67-2846", "tin_type": "ssn"}),
        ), // an SSN starting 9
        (
            "Acme Design",
            json!({"tin": "291122846", "tin_type": "itin"}),
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
            json!({"tin": 291122846.0, "tin_type": "ssn"}),
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
        json!({"tin": "620-37-1111", "tin_type": "ssn"}),
    );
    let facts = m.journaled_w9(acme);
    assert_eq!(facts["tin_last4"], "1111");
    assert_ne!(facts["vault_record"], first);
    m.pay("Acme Design", "700.00");
    let v = m.read_only(
        "books.vendor_1099",
        json!({"year": 2026, "include_tins": true}),
    );
    assert_eq!(v["vendors"][0]["tin"], "620-37-1111");
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
    // Each refused as such, never as a failed write.
    for (save_to, said) in [
        (to.clone(), "a file is there already"),
        (
            m.outside().join("no/such/dir/w9.pdf"),
            "the folder for that path doesn't exist",
        ),
    ] {
        let why = m.refuse(
            "books.get_w9",
            json!({"party": "Acme Design", "save_to": save_to.to_str().unwrap()}),
        );
        assert!(why.contains(said), "{why}");
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

/// A W-9 file is a PDF, PNG or JPEG, by its extension in any case
/// (.pdf, .png, .jpg, .jpeg): each is taken into the vault and comes back
/// byte for byte, under its own name.
#[test]
fn every_w9_file_type_is_taken() {
    let mut m = books("w9-types");
    for (n, name) in ["w9.pdf", "w9.png", "w9.jpg", "w9.jpeg", "W9.JPG"]
        .iter()
        .enumerate()
    {
        let party = format!("Payee {n}");
        m.add_party(&party, &["contractor"]);
        let src = m.w9_file(name, "000");
        let Out::Ok(p) = m.w9(
            &party,
            json!({"tin_last4": "0000", "w9_file_path": src.to_str().unwrap()}),
        ) else {
            panic!("{name} refused")
        };
        assert_eq!(p["w9"]["in_vault"], true, "{name}");
        let to = m.outside().join(format!("copy-{n}"));
        let out = m.ok(
            "books.get_w9",
            json!({"party": party, "save_to": to.to_str().unwrap()}),
        );
        assert_eq!(out["file_name"], *name);
        assert_eq!(std::fs::read(&to).unwrap(), std::fs::read(&src).unwrap());
    }
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
    assert!(why.contains("inside the books' folder"), "{why}");
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
    let mut m = books_with(
        "no-leak",
        "[[company.members]]\nname = \"Mo\"\nshare = \"100%\"\ncapital_account = \"3000\"\n",
    );
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
    // A member's SSN, by books.set_tin, and refusals of it.
    m.add_party("Mo", &["member"]);
    m.ok(
        "books.set_tin",
        json!({"party": "Mo", "tin": MEMBER_SSN, "tin_type": "ssn"}),
    );
    for args in [
        json!({"party": "Mo", "tin": MEMBER_SSN, "tin_type": "itin"}),
        json!({"party": "Mo", "tin": MEMBER_SSN_DIGITS.parse::<u64>().unwrap() as f64,
               "tin_type": "ssn"}),
        json!({"party": "Nobody", "tin": MEMBER_SSN, "tin_type": "ssn"}),
        json!({"party": "Rent Co", "tin": MEMBER_SSN, "tin_type": "ssn"}),
    ] {
        m.refuse("books.set_tin", args);
    }
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
            json!({"as_of": "2026-12-31", "cash_accounts": ["1000"], "include_tins": false}),
        ),
        (
            "books.export_cpa_package",
            json!({"as_of": "2026-12-31", "cash_accounts": ["1000"],
                   "save_to": m.outside().join("cpa.zip").to_str().unwrap()}),
        ),
        ("books.party", json!({"party": "Mo"})),
        ("books.parties", json!({"role": "member"})),
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
                    || holds_tin(resp.as_bytes(), EIN_DIGITS)
                    || holds_tin(resp.as_bytes(), MEMBER_SSN_DIGITS))
        })
        .collect();
    assert!(leaks.is_empty(), "full TINs in tool output: {leaks:?}");
    assert!(m.transcript.len() > 40, "the session used the books");
    // The server's log.
    let log = std::fs::read(m.outside().join("stderr.log")).unwrap();
    assert!(!holds_tin(&log, SSN_DIGITS) && !holds_tin(&log, EIN_DIGITS));
    assert!(!holds_tin(&log, MEMBER_SSN_DIGITS));
    // The package saved outside the books carries them all: the member's
    // in the K-1 worksheet, the payees' in the 1099 worksheet.
    let pkg = unzip(&m.outside().join("cpa.zip"));
    assert!(holds_tin(&pkg["k1-members.csv"], MEMBER_SSN_DIGITS));
    let w1099 = &pkg["1099-worksheet-2026.csv"];
    assert!(holds_tin(w1099, SSN_DIGITS) && holds_tin(w1099, EIN_DIGITS));
    for (name, bytes) in &pkg {
        if name != "k1-members.csv" && name != "1099-worksheet-2026.csv" {
            assert!(
                !holds_tin(bytes, SSN_DIGITS)
                    && !holds_tin(bytes, EIN_DIGITS)
                    && !holds_tin(bytes, MEMBER_SSN_DIGITS),
                "{name} in the package holds a TIN"
            );
        }
    }
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
            !holds_tin(&bytes, SSN_DIGITS)
                && !holds_tin(&bytes, EIN_DIGITS)
                && !holds_tin(&bytes, MEMBER_SSN_DIGITS),
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
    assert!(vault_files.len() >= 4, "{vault_files:?}");
    for f in vault_files {
        let bytes = std::fs::read(&f).unwrap();
        assert!(
            !holds_tin(&bytes, SSN_DIGITS)
                && !holds_tin(&bytes, EIN_DIGITS)
                && !holds_tin(&bytes, MEMBER_SSN_DIGITS),
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
        why.contains("version history") && why.contains("Ask the human"),
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

// ------------------------------------------------- books.set_tin, any role

/// The TIN facts a party's last `party.tin_set` journaled.
impl Mcp {
    fn journaled_tin(&self, party: u64) -> Value {
        String::from_utf8(self.journal())
            .unwrap()
            .lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .filter(|v| v["type"] == "party.tin_set" || v["line_type"] == "party.tin_set")
            .rfind(|v| v["payload"]["id"] == json!(party))
            .map(|v| v["payload"]["facts"].clone())
            .expect("a party.tin_set")
    }
}

/// Any role may have its TIN in the vault: a member (for the K-1), an
/// employee, a customer, a party with no role at all. The books journal
/// its type, last four, and record hash; the output shows the last four.
#[test]
fn set_tin_takes_any_party_members_included() {
    let mut m = books("set-tin");
    for (name, roles) in [
        ("Mo", vec!["member"]),
        ("Pat", vec!["employee"]),
        ("Nobody In Particular", vec![]),
    ] {
        let id = m.add_party(name, &roles);
        let p = m.ok(
            "books.set_tin",
            json!({"party": name, "tin": MEMBER_SSN, "tin_type": "ssn"}),
        );
        assert_eq!(p["tin"]["last4"], "4915", "{p}");
        assert_eq!(p["tin"]["tin_type"], "ssn");
        assert_eq!(p["tin"]["in_vault"], true);
        assert!(!holds_tin(p.to_string().as_bytes(), MEMBER_SSN_DIGITS));
        assert!(m.vault().join(format!("tin-{id}.bin")).exists());
        let facts = m.journaled_tin(id);
        assert_eq!(facts["last4"], "4915");
        assert_eq!(facts["tin_type"], "ssn");
        assert_eq!(facts["vault_record"].as_str().unwrap().len(), 64);
        assert_eq!(m.party(name, 2026)["party"]["tin"]["last4"], "4915");
    }
    // A customer (its role comes with its record).
    m.ok("books.add_customer", json!({"name": "Client"}));
    let p = m.ok(
        "books.set_tin",
        json!({"party": "Client", "tin": EIN, "tin_type": "ein"}),
    );
    assert_eq!(p["tin"]["last4"], "9105");
    assert!(p["roles"].as_array().unwrap().contains(&json!("customer")));
    // An EIN for a member that is a company; a replacement TIN.
    let mo = m.party_id("Mo");
    let first = m.journaled_tin(mo)["vault_record"].clone();
    m.ok(
        "books.set_tin",
        json!({"party": "Mo", "tin": EIN, "tin_type": "ein"}),
    );
    let facts = m.journaled_tin(mo);
    assert_eq!(
        (facts["last4"].clone(), facts["tin_type"].clone()),
        (json!("9105"), json!("ein"))
    );
    assert_ne!(facts["vault_record"], first);
    assert!(!holds_tin(&m.journal(), EIN_DIGITS));
    assert_eq!(m.ok("books.verify", json!({}))["status"], "ok");
}

#[test]
fn set_tin_refusals_write_nothing() {
    let mut m = books("set-tin-refusals");
    m.add_party("Mo", &["member"]);
    // A W-9 payee whose W-9 facts name another last four.
    m.w9(
        "Acme Design",
        json!({"tin_last4": "1111", "kept_at": "binder"}),
    );
    let acme = m.party_id("Acme Design");
    for (party, tin, kind) in [
        ("Mo", json!("57193284"), "ssn"),    // eight digits
        ("Mo", json!("902-67-2846"), "ssn"), // SSN starting 9
        ("Mo", json!(SSN), "itin"),          // not an ITIN
        ("Mo", json!(SSN), "passport"),      // no such type
        ("Mo", json!(291122846.0), "ssn"),   // a float
        ("Nobody", json!(SSN), "ssn"),        // no such party
        ("Acme Design", json!(SSN), "ssn"),   // W-9 says 1111
    ] {
        let why = m.refuse(
            "books.set_tin",
            json!({"party": party, "tin": tin, "tin_type": kind}),
        );
        assert!(!holds_tin(why.as_bytes(), SSN_DIGITS), "{why}");
    }
    m.refuse("books.set_tin", json!({"party": "Mo", "tin_type": "ssn"}));
    assert!(!m.vault().join(format!("tin-{acme}.bin")).exists());
    assert!(!m.key().exists(), "no key made for a refusal");
    assert_eq!(m.count("party.tin_set"), 0);
}

/// A W-9 with a TIN records the party's TIN facts too, and the two never
/// disagree: set_tin agreeing with the W-9 moves its vault record along; a
/// W-9 with a new last four replaces the TIN facts.
#[test]
fn w9_and_tin_facts_agree() {
    let mut m = books("w9-tin");
    let acme = m.party_id("Acme Design");
    m.w9("Acme Design", json!({"tin": SSN, "tin_type": "ssn"}));
    let p = m.party("Acme Design", 2026);
    assert_eq!(p["party"]["tin"]["last4"], "2846");
    assert_eq!(p["party"]["tin"]["in_vault"], true);
    let (w9, tin) = (m.journaled_w9(acme), m.journaled_tin(acme));
    assert_eq!(w9["vault_record"], tin["vault_record"]);
    // books.set_tin with the same number: both facts follow the new record.
    m.ok(
        "books.set_tin",
        json!({"party": "Acme Design", "tin": SSN, "tin_type": "ssn"}),
    );
    let (w9b, tinb) = (m.journaled_w9(acme), m.journaled_tin(acme));
    assert_ne!(tinb["vault_record"], tin["vault_record"], "a fresh nonce");
    assert_eq!(w9b["vault_record"], tinb["vault_record"]);
    // W-9 facts without a TIN that disagree with the TIN facts: refused.
    m.refuse(
        "books.set_w9",
        json!({"party": "Acme Design", "received_on": "2026-01-05",
               "entity_type": "x", "tin_last4": "1111", "kept_at": "binder"}),
    );
    // A new W-9 with a new TIN: the TIN facts are replaced (cleared first).
    let before = m.count("party.tin_set");
    m.w9(
        "Acme Design",
        json!({"tin": "620-37-1111", "tin_type": "ssn"}),
    );
    assert_eq!(m.count("party.tin_set"), before + 2, "cleared, then set");
    assert_eq!(m.journaled_tin(acme)["last4"], "1111");
    assert_eq!(m.journaled_w9(acme)["tin_last4"], "1111");
    m.pay("Acme Design", "2500.00");
    let v = m.read_only(
        "books.vendor_1099",
        json!({"year": 2026, "include_tins": true}),
    );
    assert_eq!(v["vendors"][0]["tin"], "620-37-1111");
    assert_eq!(m.ok("books.verify", json!({}))["status"], "ok");
}

/// books.purge_tin covers TIN facts: a member's record goes, the books
/// keep the last four and say when and why.
#[test]
fn purge_covers_tin_facts() {
    let mut m = books("purge-tin-facts");
    let mo = m.add_party("Mo", &["member"]);
    m.ok(
        "books.set_tin",
        json!({"party": "Mo", "tin": MEMBER_SSN, "tin_type": "ssn"}),
    );
    let p = m.ok(
        "books.purge_tin",
        json!({"party": "Mo", "reason": "left the company"}),
    );
    assert_eq!(p["status"], "preview");
    assert_eq!(p["tin_in_vault"], true);
    assert_eq!(p["tin_last4"], "4915");
    let p = m.ok(
        "books.purge_tin",
        json!({"party": "Mo", "reason": "left the company", "confirmed": true}),
    );
    assert_eq!(p["journaled"], true);
    assert!(!m.vault().join(format!("tin-{mo}.bin")).exists());
    let facts = m.journaled_tin(mo);
    assert!(facts["vault_record"].is_null(), "{facts}");
    assert_eq!(facts["last4"], "4915");
    assert!(
        facts["purged"]
            .as_str()
            .unwrap()
            .contains("left the company")
    );
    let party = m.party("Mo", 2026);
    assert_eq!(party["party"]["tin"]["in_vault"], false);
    m.refuse(
        "books.purge_tin",
        json!({"party": "Mo", "reason": "again", "confirmed": true}),
    );
    assert_eq!(m.ok("books.verify", json!({}))["status"], "ok");
}

// ------------------------------------------------- the CPA package

const MEMBERS: &str = "[[company.members]]\nname = \"Tobin Example\"\nshare = \"50%\"\ncapital_account = \"3000\"\n\
     [[company.members]]\nname = \"Mo Example\"\nshare = \"50%\"\ncapital_account = \"3001\"\n";

/// Books for the K-1 worksheet, hand-worked:
///
/// ```text
/// 2025-06-01  Dr 1000 2000 / Cr 3000 1000, Cr 3001 1000   both contribute cash
/// 2025-07-01  Dr 1000  400 / Cr 4000  400                 2025 income, 200 each
/// 2026-02-01  Dr 1500  600 / Cr 3000  600                 Tobin: equipment
/// 2026-03-01  Dr 1000  500 / Cr 3001  500                 Mo: cash
/// 2026-03-15  Dr 6999  100 / Cr 3000  100                 Tobin paid an expense
/// 2026-04-01  Dr 1000 1000 / Cr 4000 1000                 revenue
/// 2026-05-01  Dr 3000 150, Dr 3001 150 / Cr 1000 300      distributions
/// 2026-03-01  Dr 6000 2500 / Cr 1000 2500                 Acme Design (m.pay)
/// 2026: net income 1000 - 100 - 2500 = -1600, -800 each.
///   Tobin: beginning 1200, cash 100, property 600, income -800,
///            distributions 150, ending 950.
///   Mo:     beginning 1200, cash 500, property 0, income -800,
///            distributions 150, ending 750.
///   Check: bank 1100 + equipment 600 = 1700 = 950 + 750.
/// ```
///
/// Tobin's party has his SSN in the vault; Mo's has none. Acme Design
/// (NEC, an EIN in the vault) is paid 2,500.00 in 2026.
fn k1_books(name: &str) -> Mcp {
    let mut m = books_with(name, MEMBERS);
    m.accounts(&[
        ("1500", "Equipment", "asset"),
        ("3001", "Capital Mo", "equity"),
        ("4000", "App Revenue", "revenue"),
    ]);
    m.add_party("Tobin Example", &["member"]);
    m.add_party("Mo Example", &["member"]);
    m.ok(
        "books.set_tin",
        json!({"party": "Tobin Example", "tin": MEMBER_SSN, "tin_type": "ssn"}),
    );
    m.w9("Acme Design", json!({"tin": EIN, "tin_type": "ein"}));
    let none = json!({});
    m.post(
        "2025-06-01",
        none.clone(),
        &[
            ("1000", "debit", "2000.00"),
            ("3000", "credit", "1000.00"),
            ("3001", "credit", "1000.00"),
        ],
    );
    m.post(
        "2025-07-01",
        none.clone(),
        &[("1000", "debit", "400.00"), ("4000", "credit", "400.00")],
    );
    m.post(
        "2026-02-01",
        none.clone(),
        &[("1500", "debit", "600.00"), ("3000", "credit", "600.00")],
    );
    m.post(
        "2026-03-01",
        none.clone(),
        &[("1000", "debit", "500.00"), ("3001", "credit", "500.00")],
    );
    m.post(
        "2026-03-15",
        none.clone(),
        &[("6999", "debit", "100.00"), ("3000", "credit", "100.00")],
    );
    m.post(
        "2026-04-01",
        none.clone(),
        &[("1000", "debit", "1000.00"), ("4000", "credit", "1000.00")],
    );
    m.post(
        "2026-05-01",
        none,
        &[
            ("3000", "debit", "150.00"),
            ("3001", "debit", "150.00"),
            ("1000", "credit", "300.00"),
        ],
    );
    m.pay("Acme Design", "2500.00");
    m
}

/// The package carries the K-1 worksheet (one row per member: party,
/// share, capital account, TIN, and the hand-worked capital figures) and
/// the 1099 worksheet with full TINs, saved outside the books only; a
/// missing TIN is a gap, never a failure.
#[test]
fn the_cpa_package_carries_the_k1_and_1099_worksheets_with_tins() {
    let mut m = k1_books("k1");
    let to = m.outside().join("cpa-2026.zip");
    let out = m.read_only(
        "books.export_cpa_package",
        json!({"as_of": "2026-12-31", "cash_accounts": ["1000"],
               "save_to": to.to_str().unwrap()}),
    );
    assert_eq!(out["saved_to"], json!(to.display().to_string()));
    assert!(out["content_base64"].is_null(), "never the bytes: {out}");
    assert_eq!(out["tins_included"], true);
    assert!(
        out["note"]
            .as_str()
            .unwrap()
            .contains(&to.display().to_string())
    );
    let gaps: Vec<&str> = out["tin_gaps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|g| g.as_str().unwrap())
        .collect();
    assert_eq!(gaps.len(), 1, "{gaps:?}");
    assert!(gaps[0].contains("Mo Example") && gaps[0].contains("TIN not in the vault"));
    assert!(!holds_tin(out.to_string().as_bytes(), MEMBER_SSN_DIGITS));
    assert!(!holds_tin(out.to_string().as_bytes(), EIN_DIGITS));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&to).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "the package is the owner's only");
    }
    let pkg = unzip(&to);
    let k1 = csv_rows(&pkg["k1-members.csv"]);
    assert_eq!(k1.len(), 3, "two members and their total");
    let (tobin, mo) = (&k1[0], &k1[1]);
    // The total row: not a K-1; ties to the balance sheet (1,700.00).
    let total = &k1[2];
    assert_eq!(total["member"], "Total, all members (not a K-1)");
    assert_eq!(total["tin"], "");
    for (col, v) in [
        ("beginning_capital", "2400.00"),
        ("contributions_cash", "600.00"),
        ("contributions_property", "600.00"),
        ("net_income", "-1600.00"),
        ("distributions", "300.00"),
        ("other", "0.00"),
        ("ending_capital", "1700.00"),
    ] {
        assert_eq!(total[col], v, "{col}");
    }
    let sid = m.party_id("Tobin Example");
    let kid = m.party_id("Mo Example");
    for (row, want) in [
        (
            tobin,
            [
                ("member", "Tobin Example"),
                ("party_id", sid.to_string().as_str()),
                ("share", "0.50"),
                ("capital_account", "3000"),
                ("tin", MEMBER_SSN),
                ("tin_type", "ssn"),
                ("tin_note", ""),
                ("beginning_capital", "1200.00"),
                ("contributions_cash", "100.00"),
                ("contributions_property", "600.00"),
                ("net_income", "-800.00"),
                ("distributions", "150.00"),
                ("other", "0.00"),
                ("ending_capital", "950.00"),
            ],
        ),
        (
            mo,
            [
                ("member", "Mo Example"),
                ("party_id", kid.to_string().as_str()),
                ("share", "0.50"),
                ("capital_account", "3001"),
                ("tin", ""),
                ("tin_type", ""),
                ("tin_note", "TIN not in the vault"),
                ("beginning_capital", "1200.00"),
                ("contributions_cash", "500.00"),
                ("contributions_property", "0.00"),
                ("net_income", "-800.00"),
                ("distributions", "150.00"),
                ("other", "0.00"),
                ("ending_capital", "750.00"),
            ],
        ),
    ] {
        for (col, v) in want {
            let got = row
                .get(col)
                .unwrap_or_else(|| panic!("no column {col}: {row:?}"));
            if col == "tin_note" && !v.is_empty() {
                assert!(got.starts_with(v), "{col}: {got}");
            } else if col == "share" {
                assert_eq!(
                    got.trim_end_matches('0').trim_end_matches('.'),
                    "0.5",
                    "{col}"
                );
            } else {
                assert_eq!(got, v, "{col} of {row:?}");
            }
        }
    }
    // The 1099 worksheet: the payee's full TIN.
    let w = csv_rows(&pkg["1099-worksheet-2026.csv"]);
    let acme = w.iter().find(|r| r["vendor"] == "Acme Design").unwrap();
    assert_eq!(acme["tin"], EIN);
    assert_eq!(acme["tin_type"], "ein");
    assert_eq!(acme["tin_last4"], "9105");
    assert_eq!(acme["reportable"], "true");
    let readme = String::from_utf8(pkg["README.txt"].clone()).unwrap();
    assert!(
        readme.contains("k1-members.csv") && readme.contains("Mo Example"),
        "{readme}"
    );
    assert!(!holds_tin(readme.as_bytes(), MEMBER_SSN_DIGITS));
    // Nothing of it in the books dir.
    for f in files_under(&m.books_dir()) {
        if f.starts_with(m.vault()) {
            continue;
        }
        let b = std::fs::read(&f).unwrap();
        assert!(
            !holds_tin(&b, MEMBER_SSN_DIGITS) && !holds_tin(&b, EIN_DIGITS),
            "{}",
            f.display()
        );
        assert!(!f.to_string_lossy().ends_with(".zip"), "{}", f.display());
    }

    // A folder: the zip goes in it, by its name.
    let folder = m.outside().join("for-casey");
    std::fs::create_dir_all(&folder).unwrap();
    let out = m.read_only(
        "books.export_cpa_package",
        json!({"as_of": "2026-12-31", "cash_accounts": ["1000"],
               "save_to": folder.to_str().unwrap()}),
    );
    assert_eq!(
        out["saved_to"],
        json!(
            folder
                .join("cpa-package-2026-12-31.zip")
                .display()
                .to_string()
        )
    );

    // Without TINs: last fours only, returned as base64.
    let out = m.read_only(
        "books.export_cpa_package",
        json!({"as_of": "2026-12-31", "cash_accounts": ["1000"], "include_tins": false}),
    );
    assert_eq!(out["tins_included"], false);
    let zip = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        out["content_base64"].as_str().unwrap(),
    )
    .unwrap();
    let plain_path = m.outside().join("plain.zip");
    std::fs::write(&plain_path, &zip).unwrap();
    let plain = unzip(&plain_path);
    let k1 = csv_rows(&plain["k1-members.csv"]);
    assert_eq!(k1[0]["tin_last4"], "4915");
    assert!(!k1[0].contains_key("tin"));
    assert_eq!(k1[0]["ending_capital"], "950.00");
    for bytes in plain.values() {
        assert!(!holds_tin(bytes, MEMBER_SSN_DIGITS) && !holds_tin(bytes, EIN_DIGITS));
    }
}

/// With TINs the package is never returned and never lands in the books:
/// no save_to, a path in the books dir or its exports/, an existing file,
/// or a path that is neither a folder nor a .zip is refused.
#[test]
fn a_package_with_tins_goes_only_outside_the_books() {
    let mut m = k1_books("k1-where");
    let why = m.refuse(
        "books.export_cpa_package",
        json!({"as_of": "2026-12-31", "cash_accounts": ["1000"]}),
    );
    assert!(why.contains("save_to"), "{why}");
    let books = m.books_dir();
    std::fs::create_dir_all(books.join("exports")).unwrap();
    let existing = m.outside().join("taken.zip");
    std::fs::write(&existing, b"x").unwrap();
    for to in [
        books.join("cpa.zip"),
        books.join("exports"),
        books.join("exports/cpa.zip"),
        m.data.join("cpa.zip"),
        books.join("vault/../cpa.zip"),
        existing.clone(),
        m.outside().join("cpa.txt"),
        m.outside().join("no/such/dir/cpa.zip"),
    ] {
        m.refuse(
            "books.export_cpa_package",
            json!({"as_of": "2026-12-31", "cash_accounts": ["1000"],
                   "save_to": to.to_str().unwrap()}),
        );
    }
    assert_eq!(std::fs::read(&existing).unwrap(), b"x");
    assert!(
        std::fs::read_dir(books.join("exports"))
            .unwrap()
            .next()
            .is_none()
    );
    assert!(!books.join("cpa.zip").exists());
}

/// A package goes to a new file in a folder that exists, with or without
/// TINs: a missing folder and a taken file are each refused as such (not
/// as a failed write); without TINs, a new .zip path or an existing folder
/// takes it.
#[test]
fn a_package_goes_only_to_a_new_file_in_a_folder_that_exists() {
    let mut m = k1_books("k1-dest");
    let missing = m.outside().join("no/such/dir/cpa.zip");
    let taken = m.outside().join("taken.zip");
    std::fs::write(&taken, b"x").unwrap();
    for tins in [true, false] {
        for (to, said) in [
            (&missing, "the folder for that path doesn't exist"),
            (&taken, "a file is there already"),
        ] {
            let why = m.refuse(
                "books.export_cpa_package",
                json!({"as_of": "2026-12-31", "cash_accounts": ["1000"],
                       "include_tins": tins, "save_to": to.to_str().unwrap()}),
            );
            assert!(why.contains(said), "include_tins {tins}: {why}");
        }
    }
    assert_eq!(std::fs::read(&taken).unwrap(), b"x");
    let to = m.outside().join("plain.zip");
    m.read_only(
        "books.export_cpa_package",
        json!({"as_of": "2026-12-31", "cash_accounts": ["1000"],
               "include_tins": false, "save_to": to.to_str().unwrap()}),
    );
    assert!(unzip(&to).contains_key("README.txt"));
    let folder = m.outside().join("packages");
    std::fs::create_dir_all(&folder).unwrap();
    m.read_only(
        "books.export_cpa_package",
        json!({"as_of": "2026-12-31", "cash_accounts": ["1000"],
               "include_tins": false, "save_to": folder.to_str().unwrap()}),
    );
    let saved: Vec<_> = std::fs::read_dir(&folder).unwrap().collect();
    assert_eq!(saved.len(), 1, "one package in the folder");
}

/// A locked vault never fails the package: every TIN row says why, and
/// tin_gaps lists each member and payee.
#[test]
fn a_locked_vault_still_builds_the_package() {
    let mut m = k1_books("k1-locked");
    std::fs::remove_file(m.key()).unwrap();
    let to = m.outside().join("cpa.zip");
    let out = m.read_only(
        "books.export_cpa_package",
        json!({"as_of": "2026-12-31", "cash_accounts": ["1000"],
               "save_to": to.to_str().unwrap()}),
    );
    let gaps = out["tin_gaps"].as_array().unwrap();
    assert_eq!(gaps.len(), 3, "two members and Acme: {gaps:?}");
    assert!(
        gaps.iter().all(|g| g.as_str().unwrap().contains("locked")),
        "{gaps:?}"
    );
    assert!(!m.key().exists(), "reading never makes a key");
    let pkg = unzip(&to);
    let k1 = csv_rows(&pkg["k1-members.csv"]);
    let members: Vec<_> = k1
        .iter()
        .filter(|r| !r["member"].starts_with("Total"))
        .collect();
    assert_eq!(members.len(), 2);
    assert!(
        members
            .iter()
            .all(|r| r["tin"].is_empty() && r["tin_note"].contains("locked"))
    );
    assert_eq!(k1[0]["ending_capital"], "950.00", "the numbers are there");
    let w = csv_rows(&pkg["1099-worksheet-2026.csv"]);
    assert!(w.iter().all(|r| r["tin"].is_empty()));
}

/// B-26 ("No tool output, error, or log holds a full TIN"): a TIN put in
/// the wrong field is refused, and the refusal never repeats it, however
/// it was written: as a party name, a bare number where a string goes, or
/// a share's percent.
#[test]
fn refusals_never_echo_a_tin() {
    let mut m = books("echo");
    let id = m.post(
        "2026-02-01",
        json!({}),
        &[("6999", "debit", "100.00"), ("1000", "credit", "100.00")],
    );
    for (tool, args) in [
        (
            "books.set_tin",
            json!({"party": MEMBER_SSN, "tin": MEMBER_SSN, "tin_type": "ssn"}),
        ),
        (
            "books.set_tin",
            json!({"party": MEMBER_SSN_DIGITS.parse::<u64>().unwrap(), "tin": MEMBER_SSN,
                   "tin_type": "ssn"}),
        ),
        (
            "books.purge_tin",
            json!({"party": "241.40.4915", "reason": "x", "confirmed": true}),
        ),
        (
            "books.set_entry_party",
            json!({"entry_id": id, "parties": [
                {"party": "Acme Design", "percent": MEMBER_SSN},
                {"party": MEMBER_SSN, "amount": "1.00"},
            ]}),
        ),
    ] {
        let why = m.refuse(tool, args.clone());
        assert!(
            !holds_tin(why.as_bytes(), MEMBER_SSN_DIGITS),
            "{tool} {args} echoed the TIN: {why}"
        );
    }
}

/// B-26 for resource reads: a refused `resources/read` quotes the URI it
/// was given, in its message and its data, and never repeats a TIN in it,
/// however the URI carries one (a receipt id, a handbook section, an
/// unknown path, a digest's `since`).
#[test]
fn resource_refusals_never_echo_a_tin() {
    let mut m = books("resource-echo");
    for uri in [
        format!("books://receipts/{MEMBER_SSN_DIGITS}"),
        format!("books://receipts/{MEMBER_SSN}"),
        format!("books://handbook/{MEMBER_SSN}"),
        format!("books://handbook/{MEMBER_SSN_DIGITS}"),
        format!("books://{MEMBER_SSN_DIGITS}"),
        format!("books://reports/{MEMBER_SSN}"),
        format!("books://digest?since={MEMBER_SSN}"),
        format!("books://digest?since={MEMBER_SSN_DIGITS}"),
    ] {
        let resp = m.rpc("resources/read", json!({"uri": uri}));
        let err = resp
            .get("error")
            .unwrap_or_else(|| panic!("{uri} should be refused: {resp}"));
        assert!(
            !holds_tin(err.to_string().as_bytes(), MEMBER_SSN_DIGITS),
            "{uri} echoed the TIN: {err}"
        );
        assert!(!err["message"].as_str().unwrap().is_empty(), "{err}");
    }
}

/// k1::member_party: a vendor whose display name is a member's name, with
/// a W-9 EIN in the vault, never puts its TIN on the member's K-1 row:
/// the member's TIN is a gap.
#[test]
fn a_k1_row_never_carries_another_partys_tin() {
    let mut m = k1_books("k1-lookalike");
    m.ok(
        "books.update_party",
        json!({"party": "Mo Example", "archived": true}),
    );
    m.add_party("KE Consulting LLC", &["contractor"]);
    m.ok(
        "books.update_party",
        json!({"party": "KE Consulting LLC", "display_name": "Mo Example"}),
    );
    m.w9(
        "KE Consulting LLC",
        json!({"tin": "12-3456789", "tin_type": "ein"}),
    );
    let to = m.outside().join("cpa-lookalike.zip");
    let out = m.read_only(
        "books.export_cpa_package",
        json!({"as_of": "2026-12-31", "cash_accounts": ["1000"],
               "save_to": to.to_str().unwrap()}),
    );
    let gaps = out["tin_gaps"].to_string();
    assert!(gaps.contains("member Mo Example"), "{gaps}");
    let pkg = unzip(&to);
    let k1 = csv_rows(&pkg["k1-members.csv"]);
    let mo = k1.iter().find(|r| r["member"] == "Mo Example").unwrap();
    assert_eq!(mo["tin"], "", "{mo:?}");
    assert_eq!(mo["party_id"], "", "{mo:?}");
    assert!(!holds_tin(&pkg["k1-members.csv"], "123456789"));
}

/// Members who file a joint individual return (files_jointly_with on
/// both): the README says each gets their own K-1 and both go on one Form
/// 1040; books.settings reports it; a one-sided setting is a settings error.
#[test]
fn joint_filers_are_noted_in_the_package() {
    let joint = "[[company.members]]\nname = \"Tobin Example\"\nshare = \"50%\"\ncapital_account = \"3000\"\nfiles_jointly_with = \"Mo Example\"\n\
         [[company.members]]\nname = \"Mo Example\"\nshare = \"50%\"\ncapital_account = \"3001\"\nfiles_jointly_with = \"Tobin Example\"\n";
    let mut m = books_with("joint", joint);
    m.accounts(&[("3001", "Capital Mo", "equity")]);
    let s = m.read_only("books.settings", json!({}));
    assert_eq!(s["members"][0]["files_jointly_with"], "Mo Example");
    assert_eq!(s["members"][1]["files_jointly_with"], "Tobin Example");
    let to = m.outside().join("joint.zip");
    m.read_only(
        "books.export_cpa_package",
        json!({"as_of": "2026-12-31", "cash_accounts": ["1000"],
               "save_to": to.to_str().unwrap()}),
    );
    let pkg = unzip(&to);
    let readme = String::from_utf8(pkg["README.txt"].clone()).unwrap();
    let note = "Tobin Example and Mo Example file a joint individual return: each receives their own Schedule K-1; both are reported on the one Form 1040 (Schedule E, Part II).";
    assert_eq!(readme.matches(note).count(), 1, "once per pair: {readme}");
}

/// The vault's files and their bytes, by path.
fn vault_bytes(m: &Mcp) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
    files_under(&m.vault())
        .into_iter()
        .map(|p| {
            let bytes = std::fs::read(&p).unwrap();
            (p, bytes)
        })
        .collect()
}

/// MCP's idempotentHint ("calling it again with the same arguments has no
/// further effect"; plan/served.md 4.9): a tool says so only when it is
/// true of the books. Each tool here is called twice with the same
/// arguments on a fixture where the first call does something; the hint is
/// set exactly when the second call leaves the journal and the TIN vault
/// byte for byte as the first left them. Every tool marked idempotent is in
/// the table, so a new one is checked too.
#[test]
fn idempotent_tools_change_nothing_when_repeated() {
    let mut m = books("idempotent");
    m.accounts(&[
        ("1200", "Receivables", "asset"),
        ("1300", "Prepaid", "asset"),
        ("1500", "Equipment", "asset"),
        ("1501", "Accumulated Depreciation", "asset"),
        ("6100", "Depreciation", "expense"),
        ("6300", "Insurance", "expense"),
    ]);
    m.add_party("Dana Employee", &["employee"]);
    let entry = m.post(
        "2026-02-01",
        json!({}),
        &[("6000", "debit", "100.00"), ("1000", "credit", "100.00")],
    );
    let asset = m.ok(
        "books.register_asset",
        json!({"description": "Laptop", "purchase_date": "2026-01-15", "cost": "2400.00",
               "useful_life_months": 24, "cost_account": "1500",
               "accum_depr_account": "1501", "expense_account": "6100"}),
    )["asset_id"]
        .as_u64()
        .unwrap();
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
    let category = m.read_only("books.tax_categories", json!({}))["categories"][0]["key"]
        .as_str()
        .unwrap()
        .to_string();
    let settings_file =
        std::fs::read_to_string(m.data.parent().unwrap().join("wanderbooks.toml")).unwrap();
    let calls: Vec<(&str, Value)> = vec![
        ("books.verify", json!({"blobs": true})),
        (
            "books.import_settings",
            json!({"toml_text": settings_file, "decided_by": "Owner", "confirmed": true}),
        ),
        (
            "books.set_setting",
            json!({"changes": [{"key": "company.timezone", "value": "America/Denver"}],
                   "decided_by": "Owner"}),
        ),
        (
            "books.run_schedules",
            json!({"through": "2026-03-31", "confirmed": true}),
        ),
        ("books.run_recurring", json!({"through": "2026-03-15"})),
        (
            "books.set_vendor_1099",
            json!({"vendor": "Acme Design", "form_1099": "misc"}),
        ),
        (
            "books.set_entry_vendor",
            json!({"entry_id": entry, "vendor": "Acme Design"}),
        ),
        (
            "books.label_asset",
            json!({"asset": asset.to_string(), "serial": "SN-1"}),
        ),
        (
            "books.set_asset_tax_basis",
            json!({"asset": asset.to_string(), "rule": "cost"}),
        ),
        (
            "books.assign_role",
            json!({"role": "receivables", "account": "1200"}),
        ),
        (
            "books.set_tax_category",
            json!({"account": "6000", "category": category}),
        ),
        (
            "books.run_depreciation",
            json!({"period": "2026-03", "asset_id": asset}),
        ),
        (
            "books.mark_cleared",
            json!({"entry_id": entry, "line_index": 1, "cleared": true}),
        ),
        (
            "books.update_party",
            json!({"party": "Rent Co", "notes": "pays on the 1st"}),
        ),
        (
            "books.set_party_roles",
            json!({"party": "Rent Co", "add": ["financial_institution"]}),
        ),
        (
            "books.set_w9",
            json!({"party": "Acme Design", "received_on": "2026-01-05",
                   "entity_type": "individual/sole proprietor", "tin": SSN, "tin_type": "ssn"}),
        ),
        (
            "books.set_w9",
            json!({"party": "Rent Co", "received_on": "2026-01-05", "entity_type": "llc",
                   "tin_last4": "9105", "kept_at": "the office safe"}),
        ),
        (
            "books.set_tin",
            json!({"party": "Dana Employee", "tin": MEMBER_SSN, "tin_type": "ssn"}),
        ),
        (
            "books.set_w4",
            json!({"party": "Dana Employee", "received_on": "2026-01-05", "state": "UT",
                   "kept_at": "the office safe"}),
        ),
        (
            "books.set_entry_party",
            json!({"entry_id": entry, "party": "Acme Design"}),
        ),
        (
            "books.set_fx_rate",
            json!({"currency": "EUR", "date": "2026-01-02", "rate": "1.10"}),
        ),
    ];
    let list = m.rpc("tools/list", json!({}));
    let marked: std::collections::BTreeSet<String> = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|t| t["annotations"]["idempotentHint"] == json!(true))
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    for name in &marked {
        assert!(
            calls.iter().any(|(n, _)| n == name),
            "{name} is marked idempotent: add it to this table"
        );
    }
    let mut repeats_change: std::collections::BTreeMap<&str, bool> = Default::default();
    for (name, args) in &calls {
        m.ok(name, args.clone());
        let (journal, vault) = (m.journal(), vault_bytes(&m));
        m.ok(name, args.clone());
        let changed = m.journal() != journal || vault_bytes(&m) != vault;
        *repeats_change.entry(name).or_default() |= changed;
    }
    let wrong: Vec<String> = repeats_change
        .into_iter()
        .filter(|(name, changed)| marked.contains(*name) == *changed)
        .map(|(name, changed)| {
            format!(
                "{name}: marked idempotent {}, but a repeat {} the books",
                marked.contains(name),
                if changed { "changes" } else { "leaves alone" },
            )
        })
        .collect();
    assert!(wrong.is_empty(), "{wrong:#?}");
}
