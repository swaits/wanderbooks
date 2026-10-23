//! Oracle tests for the books server's tools, driven through a real
//! `wanderbooks server` as an agent reaches it (common.rs).
//!
//! Every number a tool reports is checked against what `ledger-core` (or
//! `ledger-store`) computes for the same books, computed here from their
//! public APIs, or against first principles of double entry. Formats are
//! checked by reading them back: CSV is parsed (RFC 4180) to the same rows,
//! JSON deserializes, zips and XLSX unzip, IIF splits into its records.
//!
//! The books are mostly built in-process with [`Store`] (fast, and the
//! oracle sees exactly the books the server will open), in the books'
//! folder of a data root of the test's own; then the store is closed and
//! the server started on that root.

use base64::Engine as _;
use common::{Place, Wire};
use ledger_core::{
    AccountId, AccountType, CashFlowParams, Currency, EntryStatus, Form1099, Ledger, MemberCapital,
    Money, NewAsset, Side, depreciation_schedule, reports,
};
use ledger_store::Store;
use ledger_store::ops::{ViewerCommand, ViewerTarget};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::str::FromStr;

mod common;

// ---------------------------------------------------------------- harness

#[derive(Debug)]
enum Out {
    Ok(Value),
    ToolErr(String),
    RpcErr(Value),
}

struct Mcp {
    wire: Wire,
    next_id: u64,
    /// The books' folder in the data root.
    data: PathBuf,
}

/// A test's place (common.rs): its home, its server, its books.
type TestDir = Place;

fn test_dir(name: &str) -> TestDir {
    Place::new(&format!("oracle-{name}"))
}

/// `wanderbooks.toml` for two 50/50 members on 3000/3001, cash 1000,
/// financing 2100, with the given receipt threshold.
fn books_toml(threshold: &str) -> String {
    format!(
        "[company]\nname = \"Oracle Co\"\nfiscal_year_start = \"01-01\"\n\
         [[company.members]]\nname = \"A\"\nshare = \"50%\"\ncapital_account = \"3000\"\n\
         [[company.members]]\nname = \"B\"\nshare = \"50%\"\ncapital_account = \"3001\"\n\
         [books]\nreceipt_threshold = \"{threshold}\"\ncash_accounts = [\"1000\"]\n\
         financing_accounts = [\"2100\"]\n"
    )
}

/// The books' folder in `dir`'s data root, where the oracle writes the
/// books the server then opens.
fn data_dir(dir: &TestDir) -> PathBuf {
    dir.books_data()
}

impl Mcp {
    /// Start (or restart) `dir`'s server, and a session whose books keep
    /// [`books_toml`]'s settings with this receipt threshold (set as an
    /// agent sets them: a value already set changes nothing).
    fn start(dir: &TestDir, threshold: &str) -> Self {
        let wire = dir.start(None);
        let mut m = Self {
            wire,
            next_id: 1,
            data: dir.books_data(),
        };
        let init = m.rpc(
            "initialize",
            json!({"protocolVersion": "2025-06-18", "capabilities": {},
                   "clientInfo": {"name": "oracle-test", "version": "0"}}),
        );
        assert!(init.get("result").is_some(), "initialize failed: {init}");
        m.send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}).to_string());
        m.ok(
            "books.set_setting",
            json!({"changes": common::settings_changes(&books_toml(threshold), None, "USD"),
                   "decided_by": "Test Owner"}),
        );
        m
    }

    fn send(&mut self, line: &str) {
        self.wire.send_line(line);
    }

    fn rpc(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(
            &json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string(),
        );
        let v = self
            .wire
            .recv(common::TIMEOUT)
            .unwrap_or_else(|| panic!("no response to {method}: {:?}", self.wire.refused));
        assert_eq!(v["id"], json!(id));
        v
    }

    fn tool(&mut self, name: &str, args: Value) -> Out {
        let resp = self.rpc("tools/call", json!({"name": name, "arguments": args}));
        if let Some(e) = resp.get("error") {
            return Out::RpcErr(e.clone());
        }
        let result = &resp["result"];
        if result.get("isError") == Some(&json!(true)) {
            let text = result["content"]
                .as_array()
                .and_then(|c| c.iter().find_map(|b| b["text"].as_str()))
                .unwrap_or_default()
                .to_string();
            return Out::ToolErr(text);
        }
        Out::Ok(result["structuredContent"].clone())
    }

    fn ok(&mut self, name: &str, args: Value) -> Value {
        let a = args.to_string();
        match self.tool(name, args) {
            Out::Ok(v) => v,
            other => panic!("{name} {a}: expected success, got {other:?}"),
        }
    }

    fn refuse(&mut self, name: &str, args: Value) -> String {
        let a = args.to_string();
        match self.tool(name, args) {
            Out::Ok(v) => panic!("{name} {a}: expected a refusal, got {v}"),
            Out::ToolErr(t) => t,
            Out::RpcErr(e) => e.to_string(),
        }
    }

    /// `books.export`, returning the content (base64-decoded when the
    /// server says it is base64).
    fn export(&mut self, args: Value) -> Vec<u8> {
        let out = self.ok("books.export", args);
        let content = out["content"].as_str().unwrap();
        if out["content_encoding"] == "base64" {
            b64d(content)
        } else {
            content.as_bytes().to_vec()
        }
    }

    fn export_text(&mut self, args: Value) -> String {
        let out = self.ok("books.export", args.clone());
        assert!(
            out.get("content_encoding").is_none_or(|e| e.is_null()),
            "text export {args} came back encoded"
        );
        out["content"].as_str().unwrap().to_string()
    }

    /// The viewer command the last `books.show` wrote, in the books' own
    /// `ops/`.
    fn viewer_command(&self) -> ViewerCommand {
        ledger_store::ops::read(&self.data).expect("a show request")
    }

    /// The bytes of a file a tool handed over (a `HandedFile`): fetched by
    /// its download, matching the size and SHA-256 it states.
    fn handed(&mut self, file: &Value) -> Vec<u8> {
        assert!(file["expires_at"].is_string(), "{file}");
        self.wire.fetch(file)
    }
}

// ---------------------------------------------------------------- helpers

fn b64e(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn b64d(s: &str) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD
        .decode(s)
        .expect("valid base64")
}

fn sha256(bytes: &[u8]) -> String {
    ledger_store::journal::sha256_hex(bytes)
}

fn d(s: &str) -> chrono::NaiveDate {
    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
}

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).unwrap_or_else(|e| panic!("{s:?} is not a decimal: {e}"))
}

fn usd(s: &str) -> Money {
    Money::new(dec(s), Currency::USD)
}

/// Amounts out (server invariant 3): `^-?\d+\.\d{2,}$`, parsed exactly.
fn amt(s: &str) -> Decimal {
    let body = s.strip_prefix('-').unwrap_or(s);
    let ok = body.split_once('.').is_some_and(|(i, f)| {
        !i.is_empty()
            && i.bytes().all(|b| b.is_ascii_digit())
            && f.len() >= 2
            && f.bytes().all(|b| b.is_ascii_digit())
    });
    assert!(ok, "amount {s:?} is not a decimal string with 2+ places");
    dec(s)
}

/// A JSON string field as an amount.
fn jamt(v: &Value) -> Decimal {
    amt(v
        .as_str()
        .unwrap_or_else(|| panic!("amount {v} is not a string")))
}

/// RFC 4180: fields separated by commas, records by CRLF or LF, quoted
/// fields may hold commas, quotes (doubled), and line breaks.
fn parse_csv(s: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    quoted = false;
                }
            } else {
                field.push(c);
            }
            continue;
        }
        match c {
            '"' => quoted = true,
            ',' => row.push(std::mem::take(&mut field)),
            '\r' => {}
            '\n' => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            _ => field.push(c),
        }
    }
    assert!(!quoted, "unterminated quoted CSV field");
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

/// Parse CSV, check the header and that every row has its width.
fn csv_rows(s: &str, header: &[&str]) -> Vec<Vec<String>> {
    let mut rows = parse_csv(s);
    assert!(!rows.is_empty(), "empty CSV");
    let head = rows.remove(0);
    assert_eq!(head, header, "CSV header");
    for r in &rows {
        assert_eq!(r.len(), header.len(), "CSV row width: {r:?}");
    }
    rows
}

/// A zip's files, by name.
fn unzip(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    assert!(bytes.starts_with(b"PK"), "not a zip");
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("a readable zip");
    let mut out = BTreeMap::new();
    for i in 0..z.len() {
        let mut f = z.by_index(i).unwrap();
        let name = f.name().expect("a zip member name").to_string();
        let mut buf = Vec::new();
        f.read_to_end(&mut buf).unwrap();
        assert!(
            out.insert(name.clone(), buf).is_none(),
            "duplicate zip member {name}"
        );
    }
    out
}

/// All the text in an XLSX workbook's XML parts.
fn xlsx_text(bytes: &[u8]) -> String {
    let files = unzip(bytes);
    assert!(
        files.contains_key("[Content_Types].xml"),
        "an XLSX has [Content_Types].xml: {:?}",
        files.keys()
    );
    assert!(
        files.keys().any(|k| k.starts_with("xl/worksheets/")),
        "an XLSX has worksheets"
    );
    files
        .iter()
        .filter(|(k, _)| k.ends_with(".xml"))
        .map(|(_, v)| String::from_utf8_lossy(v).into_owned())
        .collect::<Vec<_>>()
        .join("\n")
}

fn id_of(s: &Store, code: &str) -> AccountId {
    s.ledger().chart().find_by_code(code).unwrap().id
}

/// Normal-side sign: +1 when the account's normal side is debit.
fn normal_signed(ledger: &Ledger, account: AccountId, side: Side, amount: Decimal) -> Decimal {
    let a = ledger.chart().get(account).unwrap();
    if side == a.account_type.normal_side() {
        amount
    } else {
        -amount
    }
}

fn side_str(s: Side) -> &'static str {
    match s {
        Side::Debit => "debit",
        Side::Credit => "credit",
    }
}

// ---------------------------------------------------------------- fixture

const CHART: &[(&str, &str, AccountType)] = &[
    ("1000", "Bank", AccountType::Asset),
    ("1200", "Accounts Receivable", AccountType::Asset),
    ("1300", "Prepaid Expenses", AccountType::Asset),
    ("1500", "Equipment", AccountType::Asset),
    ("1510", "Accumulated Depreciation", AccountType::Asset),
    ("2000", "Accounts Payable", AccountType::Liability),
    ("2100", "Due to Shareholders", AccountType::Liability),
    ("3000", "Capital A", AccountType::Equity),
    ("3001", "Capital B", AccountType::Equity),
    ("3999", "Retained Earnings", AccountType::Equity),
    ("4000", "Revenue", AccountType::Revenue),
    ("6000", "Rent", AccountType::Expense),
    ("6010", "Meals, \"client\"", AccountType::Expense),
    ("6020", "Coffee", AccountType::Expense),
    ("6500", "Depreciation Expense", AccountType::Expense),
    ("6999", "General Expenses", AccountType::Expense),
    ("7100", "Gain or Loss on Disposal", AccountType::Expense),
];

fn new_store(dir: &TestDir) -> Store {
    let mut s = Store::create(&data_dir(dir), Currency::USD).unwrap();
    for (code, name, ty) in CHART {
        s.add_account(code, name, *ty, None, false).unwrap();
    }
    s
}

/// Draft an entry from `(code, side, amount, class)` lines.
fn draft(s: &mut Store, date: &str, desc: &str, lines: &[(&str, Side, &str)]) -> u64 {
    let e = s.create_entry(d(date), desc).unwrap();
    for (code, side, a) in lines {
        let id = id_of(s, code);
        s.add_line(e, id, *side, usd(a)).unwrap();
    }
    e
}

fn post(s: &mut Store, date: &str, desc: &str, lines: &[(&str, Side, &str)]) -> u64 {
    let e = draft(s, date, desc, lines);
    s.post_entry(e).unwrap();
    e
}

use Side::{Credit as Cr, Debit as Dr};

struct Rich {
    rent: u64,
    rondo: u32,
    coach: u32,
    contractor: u64,
    prior: u64,
    after: u64,
    never: u64,
}

/// Books with a bit of everything, mostly in 2025: contributions, classes,
/// vendors, an asset and its depreciation, a reversal, a draft, an entry
/// the year before and one after mid-year, and awkward names for CSV.
fn rich_books(s: &mut Store) -> Rich {
    let rondo = s.add_class("Rondo").unwrap();
    let coach = s.add_class("Coach").unwrap();
    let landlord = s.add_vendor("Landlord").unwrap();
    let contractor = s.add_vendor("Contractor").unwrap();
    s.set_vendor_1099(landlord, &decided(Form1099::Misc), Some(true))
        .unwrap();
    s.set_vendor_1099(contractor, &decided(Form1099::Nec), Some(false))
        .unwrap();
    s.set_account_tax(id_of(s, "6000"), Some("rent_property"), None)
        .unwrap();
    s.set_account_tax(id_of(s, "6010"), Some("meals"), None)
        .unwrap();

    let prior = post(
        s,
        "2024-12-15",
        "Prior-year sale",
        &[("1000", Dr, "100.00"), ("4000", Cr, "100.00")],
    );
    post(
        s,
        "2025-01-02",
        "Owner contribution",
        &[
            ("1000", Dr, "10000.00"),
            ("3000", Cr, "5000.00"),
            ("3001", Cr, "5000.00"),
        ],
    );
    let rent = draft(
        s,
        "2025-01-15",
        "Rent January",
        &[("6000", Dr, "1200"), ("1000", Cr, "1200")],
    );
    s.set_line_class(rent, 0, Some(rondo)).unwrap();
    s.set_entry_vendor(rent, Some(landlord)).unwrap();
    s.post_entry(rent).unwrap();
    let sale = draft(
        s,
        "2025-02-01",
        "Rondo sale",
        &[("1000", Dr, "2500.00"), ("4000", Cr, "2500.00")],
    );
    s.set_line_class(sale, 1, Some(rondo)).unwrap();
    s.post_entry(sale).unwrap();
    let coach_sale = draft(
        s,
        "2025-02-10",
        "Coach sale",
        &[("1000", Dr, "700.25"), ("4000", Cr, "700.25")],
    );
    s.set_line_class(coach_sale, 1, Some(coach)).unwrap();
    s.post_entry(coach_sale).unwrap();
    post(
        s,
        "2025-02-20",
        "Invoice, on account",
        &[("1200", Dr, "300.00"), ("4000", Cr, "300.00")],
    );
    post(
        s,
        "2025-02-21",
        "Supplies on account",
        &[("6999", Dr, "40.00"), ("2000", Cr, "40.00")],
    );
    post(
        s,
        "2025-03-05",
        "Lunch, \"team\"\nquarterly",
        &[("6010", Dr, "80.50"), ("2100", Cr, "80.50")],
    );
    post(
        s,
        "2025-03-20",
        "Laptop",
        &[("1500", Dr, "3000.00"), ("1000", Cr, "3000.00")],
    );
    let asset = s
        .register_asset(NewAsset {
            description: "Laptop".into(),
            in_service_date: d("2025-03-20"),
            cost: usd("3000.00"),
            salvage_value: usd("0.00"),
            useful_life_months: 36,
            cost_account: id_of(s, "1500"),
            accum_depr_account: id_of(s, "1510"),
            expense_account: id_of(s, "6500"),
        })
        .unwrap();
    for p in ["2025-03", "2025-04", "2025-05"] {
        s.run_depreciation(asset, p).unwrap();
    }
    post(
        s,
        "2025-04-01",
        "Distribution",
        &[
            ("3000", Dr, "500.00"),
            ("3001", Dr, "500.00"),
            ("1000", Cr, "1000.00"),
        ],
    );
    let oops = post(
        s,
        "2025-04-10",
        "Mistake",
        &[("6999", Dr, "50.00"), ("1000", Cr, "50.00")],
    );
    s.reverse_entry(oops, d("2025-04-11"), "Reverse mistake")
        .unwrap();
    let pay = draft(
        s,
        "2025-05-10",
        "Contractor work",
        &[("6999", Dr, "900.00"), ("1000", Cr, "900.00")],
    );
    s.set_entry_vendor(pay, Some(contractor)).unwrap();
    s.post_entry(pay).unwrap();
    let never = draft(
        s,
        "2025-05-01",
        "Never posted",
        &[("6999", Dr, "999.00"), ("1000", Cr, "999.00")],
    );
    let after = post(
        s,
        "2025-08-01",
        "After mid-year",
        &[("6999", Dr, "25.00"), ("1000", Cr, "25.00")],
    );
    Rich {
        rent,
        rondo,
        coach,
        contractor,
        prior,
        after,
        never,
    }
}

const START: &str = "2025-01-01";
const END: &str = "2025-06-30";

/// A 1099 decision from the beginning, decided by the owner.
fn decided(form: Form1099) -> ledger_core::Dated1099 {
    ledger_core::Dated1099::from_beginning(
        ledger_core::Decision1099::Files(form),
        Some(ledger_core::settings::Source {
            decided_by: "Oracle Owner".into(),
            decided_on: d("2025-01-01"),
            citation: None,
            note: None,
        }),
    )
}

/// The 1099 report the server makes for these books: payments out of
/// cash 1000 and financing 2100 (books_toml), no member parties (the
/// rich books name none, so no shares resolve against members).
fn core_1099(
    ledger: &Ledger,
    year: i32,
    threshold: Option<Decimal>,
) -> ledger_core::Vendor1099Report {
    core_1099_through(ledger, year, threshold, None)
}

/// [`core_1099`], payments dated after `through` left out.
fn core_1099_through(
    ledger: &Ledger,
    year: i32,
    threshold: Option<Decimal>,
    through: Option<chrono::NaiveDate>,
) -> ledger_core::Vendor1099Report {
    let id = |code: &str| ledger.chart().find_by_code(code).unwrap().id;
    ledger_core::vendor_1099_report(
        ledger,
        year,
        threshold,
        &ledger_core::PaymentAccounts {
            cash: vec![id("1000")],
            financing: vec![id("2100")],
        },
        &[],
        through,
    )
    .unwrap()
}

// ------------------------------------------------- statements: csv + json

/// Trial balance CSV and JSON equal `trial_balance_as_of`, line by line.
/// Kills: export_trial_balance stubs, its json/csv switch.
#[test]
fn trial_balance_exports_equal_the_core_trial_balance() {
    let dir = test_dir("tb");
    let mut s = new_store(&dir);
    rich_books(&mut s);
    let tb = reports::trial_balance_as_of(s.ledger(), d(END));
    drop(s);
    assert!(!tb.lines.is_empty());
    let mut m = Mcp::start(&dir, "1000000.00");

    let csv = m.export_text(json!({"format": "csv", "report": "trial_balance", "as_of": END}));
    let rows = csv_rows(&csv, &["code", "name", "debit", "credit"]);
    assert_eq!(rows.len(), tb.lines.len() + 1, "a row per line plus TOTAL");
    for (r, l) in rows.iter().zip(&tb.lines) {
        assert_eq!(r[0], l.code);
        assert_eq!(r[1], l.name);
        assert_eq!(amt(&r[2]), l.debit, "{} debit", l.code);
        assert_eq!(amt(&r[3]), l.credit, "{} credit", l.code);
    }
    let total = rows.last().unwrap();
    assert_eq!(total[0], "TOTAL");
    assert_eq!(amt(&total[2]), tb.total_debits);
    assert_eq!(amt(&total[3]), tb.total_credits);

    let j: Value = serde_json::from_str(
        &m.export_text(json!({"format": "json", "report": "trial_balance", "as_of": END})),
    )
    .expect("JSON export deserializes");
    assert_eq!(j["as_of"], END);
    assert_eq!(j["balanced"], json!(tb.balanced()));
    assert_eq!(jamt(&j["total_debits"]), tb.total_debits);
    assert_eq!(jamt(&j["total_credits"]), tb.total_credits);
    let lines = j["lines"].as_array().unwrap();
    assert_eq!(lines.len(), tb.lines.len());
    for (o, l) in lines.iter().zip(&tb.lines) {
        assert_eq!(o["code"], l.code.as_str());
        assert_eq!(o["name"], l.name.as_str());
        assert_eq!(jamt(&o["debit"]), l.debit);
        assert_eq!(jamt(&o["credit"]), l.credit);
    }
}

fn check_statement_lines(got: &Value, want: &[ledger_core::StatementLine], what: &str) {
    let got = got.as_array().unwrap_or_else(|| panic!("{what} lines"));
    assert_eq!(got.len(), want.len(), "{what}: line count");
    for (g, w) in got.iter().zip(want) {
        assert_eq!(g["code"], w.code.as_str(), "{what}");
        assert_eq!(g["name"], w.name.as_str(), "{what}");
        assert_eq!(jamt(&g["amount"]), w.amount, "{what} {}", w.code);
    }
}

/// P&L CSV and JSON equal `profit_and_loss_for_period`; balance sheet CSV
/// and JSON equal `balance_sheet_as_of`. Kills: export_pnl and
/// export_balance_sheet stubs and json/csv switches, and the export
/// dispatch's balance_sheet arm.
#[test]
fn pnl_and_balance_sheet_exports_equal_the_core_statements() {
    let dir = test_dir("pnl-bs");
    let mut s = new_store(&dir);
    rich_books(&mut s);
    let pl = ledger_core::profit_and_loss_for_period(s.ledger(), d(START), d(END), None).unwrap();
    let bs = ledger_core::balance_sheet_as_of(s.ledger(), d(END)).unwrap();
    drop(s);
    let mut m = Mcp::start(&dir, "1000000.00");

    // P&L, CSV.
    let csv = m.export_text(
        json!({"format": "csv", "report": "profit_and_loss", "start": START, "end": END}),
    );
    let rows = csv_rows(&csv, &["section", "code", "name", "amount"]);
    let mut want: Vec<(String, String, String, Decimal)> = Vec::new();
    for (section, lines) in [("revenue", &pl.revenue), ("expenses", &pl.expenses)] {
        for l in lines {
            want.push((section.into(), l.code.clone(), l.name.clone(), l.amount));
        }
    }
    for (label, v) in [
        ("revenue", pl.total_revenue),
        ("expenses", pl.total_expenses),
        ("net_income", pl.net_income()),
    ] {
        want.push(("total".into(), String::new(), label.into(), v));
    }
    let got: Vec<_> = rows
        .iter()
        .map(|r| (r[0].clone(), r[1].clone(), r[2].clone(), amt(&r[3])))
        .collect();
    assert_eq!(got, want, "P&L CSV");

    // P&L, JSON.
    let j: Value = serde_json::from_str(&m.export_text(
        json!({"format": "json", "report": "profit_and_loss", "start": START, "end": END}),
    ))
    .unwrap();
    assert_eq!(j["start"], START);
    assert_eq!(j["end"], END);
    check_statement_lines(&j["revenue_accounts"], &pl.revenue, "revenue");
    check_statement_lines(&j["expense_accounts"], &pl.expenses, "expenses");
    assert_eq!(jamt(&j["revenue"]), pl.total_revenue);
    assert_eq!(jamt(&j["expenses"]), pl.total_expenses);
    assert_eq!(jamt(&j["net_income"]), pl.net_income());

    // Balance sheet, CSV.
    let csv = m.export_text(json!({"format": "csv", "report": "balance_sheet", "as_of": END}));
    let rows = csv_rows(&csv, &["section", "code", "name", "amount"]);
    let mut want: Vec<(String, String, String, Decimal)> = Vec::new();
    for (section, lines) in [
        ("assets", &bs.assets),
        ("liabilities", &bs.liabilities),
        ("equity", &bs.equity),
    ] {
        for l in lines {
            want.push((section.into(), l.code.clone(), l.name.clone(), l.amount));
        }
    }
    want.push((
        "equity".into(),
        String::new(),
        "current_earnings".into(),
        bs.current_earnings,
    ));
    for (label, v) in [
        ("assets", bs.total_assets),
        ("liabilities", bs.total_liabilities),
        ("equity", bs.total_equity),
    ] {
        want.push(("total".into(), String::new(), label.into(), v));
    }
    let got: Vec<_> = rows
        .iter()
        .map(|r| (r[0].clone(), r[1].clone(), r[2].clone(), amt(&r[3])))
        .collect();
    assert_eq!(got, want, "balance sheet CSV");

    // Balance sheet, JSON.
    let j: Value = serde_json::from_str(
        &m.export_text(json!({"format": "json", "report": "balance_sheet", "as_of": END})),
    )
    .unwrap();
    assert_eq!(j["as_of"], END);
    check_statement_lines(&j["asset_accounts"], &bs.assets, "assets");
    check_statement_lines(&j["liability_accounts"], &bs.liabilities, "liabilities");
    check_statement_lines(&j["equity_accounts"], &bs.equity, "equity");
    assert_eq!(jamt(&j["assets"]), bs.total_assets);
    assert_eq!(jamt(&j["liabilities"]), bs.total_liabilities);
    assert_eq!(jamt(&j["equity"]), bs.total_equity);
    assert_eq!(jamt(&j["current_earnings"]), bs.current_earnings);
    assert_eq!(j["balanced"], json!(bs.balanced()));
}

/// Cash flow CSV and JSON equal `ledger_core::cash_flow` for the same
/// account roles and members, field for field (rules.md B-14; the JSON is
/// the tool's own answer). Kills: export_cash_flow stubs and its json/csv
/// switch, and any field the mapping drops.
#[test]
fn cash_flow_exports_equal_the_core_statement() {
    let dir = test_dir("cf");
    let mut s = new_store(&dir);
    rich_books(&mut s);
    // Equipment on an operating payable: a warning, in the CSV too.
    post(
        &mut s,
        "2025-04-02",
        "Monitor on account",
        &[("1500", Dr, "250.00"), ("2000", Cr, "250.00")],
    );
    let member = |name: &str, code: &str| ledger_core::MemberCapital {
        name: name.into(),
        capital_account: id_of(&s, code),
        distribution_account: None,
        share: Decimal::new(5, 1),
    };
    let members = vec![member("A", "3000"), member("B", "3001")];
    let cf = ledger_core::cash_flow(
        s.ledger(),
        &CashFlowParams {
            start: d(START),
            end: d(END),
            cash_accounts: vec![id_of(&s, "1000")],
            financing_accounts: vec![id_of(&s, "2100")],
            investing_accounts: vec![],
            members,
        },
    )
    .unwrap();
    drop(s);
    let mut m = Mcp::start(&dir, "1000000.00");
    let args = |format: &str| {
        json!({"format": format, "report": "cash_flow", "start": START, "end": END,
               "cash_accounts": ["1000"], "financing_accounts": ["2100"]})
    };
    use ledger_core::CashFlowSection as S;

    let csv = m.export_text(args("csv"));
    let rows = csv_rows(
        &csv,
        &[
            "section",
            "kind",
            "class",
            "entry",
            "date",
            "label",
            "amount",
            "on_behalf",
        ],
    );
    let row = |sec: &str, kind: &str, class: &str, entry: String, date: String, label: &str, v| {
        (
            sec.to_string(),
            kind.to_string(),
            class.to_string(),
            entry,
            date,
            label.to_string(),
            v,
            None,
        )
    };
    let mut want = Vec::new();
    for (sec, total) in [
        (S::Operating, "Net cash from operating activities"),
        (S::Investing, "Net cash from investing activities"),
        (S::Financing, "Net cash from financing activities"),
    ] {
        for l in cf.lines(sec) {
            let mut r = row(
                sec.key(),
                &l.kind,
                l.class.map_or("", |c| c.key()),
                l.entry.to_string(),
                l.date.to_string(),
                &l.label,
                l.amount,
            );
            r.7 = Some(l.on_behalf);
            want.push(r);
        }
        want.push(row(
            sec.key(),
            "",
            "",
            String::new(),
            String::new(),
            total,
            cf.total(sec),
        ));
    }
    let r = &cf.operating_reconciliation;
    want.push(row(
        "reconciliation",
        "",
        "",
        String::new(),
        String::new(),
        "Net income (loss)",
        r.net_income,
    ));
    for l in &r.lines {
        want.push(row(
            "reconciliation",
            l.kind.key(),
            "",
            String::new(),
            String::new(),
            &l.label,
            l.amount,
        ));
    }
    want.push(row(
        "reconciliation",
        "",
        "",
        String::new(),
        String::new(),
        "Net cash from operating activities",
        r.net_cash_from_operating,
    ));
    for n in &cf.noncash_disclosures {
        want.push(row(
            "noncash",
            "",
            "",
            n.entry.to_string(),
            n.date.to_string(),
            &n.label,
            n.amount,
        ));
    }
    for (label, v) in [
        ("Net increase (decrease) in cash", cf.net_change_in_cash),
        ("Cash at beginning of period", cf.cash_beginning),
        ("Cash at end of period", cf.cash_ending),
        ("Sections sum", cf.reconciles.sections_sum),
        ("Difference", cf.reconciles.difference),
    ] {
        want.push(row(
            "footing",
            "",
            "",
            String::new(),
            String::new(),
            label,
            v,
        ));
    }
    // The warnings close the CSV: kind, entry and message, no amount.
    let (warned, rows): (Vec<_>, Vec<_>) = rows.into_iter().partition(|r| r[0] == "warning");
    let want_warned: Vec<Vec<String>> = cf
        .warnings
        .iter()
        .map(|w| {
            vec![
                "warning".to_string(),
                w.kind.key().to_string(),
                String::new(),
                w.entry.to_string(),
                String::new(),
                w.message.clone(),
                String::new(),
                String::new(),
            ]
        })
        .collect();
    assert_eq!(warned, want_warned, "cash flow CSV warnings");
    assert_eq!(cf.warnings.len(), 1, "the monitor bought on account");
    let got: Vec<_> = rows
        .iter()
        .map(|r| {
            (
                r[0].clone(),
                r[1].clone(),
                r[2].clone(),
                r[3].clone(),
                r[4].clone(),
                r[5].clone(),
                amt(&r[6]),
                (!r[7].is_empty()).then(|| amt(&r[7])),
            )
        })
        .collect();
    assert_eq!(got, want, "cash flow CSV");
    assert!(
        cf.operating.len() + cf.financing.len() > 1,
        "the books move cash"
    );
    // The lunch a member paid through 2100 is borrowed and spent (D-42):
    // its lines carry what was paid on the company's behalf.
    assert!(
        cf.financing.iter().any(|l| !l.on_behalf.is_zero()),
        "a payment made for the company"
    );

    let j: Value = serde_json::from_str(&m.export_text(args("json"))).unwrap();
    assert_eq!(j["method"], "direct");
    assert_eq!(j["start"], START);
    assert_eq!(j["end"], END);
    for (k, v) in [
        ("cash_from_operations", cf.cash_from_operations),
        ("cash_from_investing", cf.cash_from_investing),
        ("cash_from_financing", cf.cash_from_financing),
        ("net_change_in_cash", cf.net_change_in_cash),
        ("cash_beginning", cf.cash_beginning),
        ("cash_ending", cf.cash_ending),
    ] {
        assert_eq!(jamt(&j[k]), v, "cash flow {k}");
    }
    for (k, sec) in [
        ("operating", S::Operating),
        ("investing", S::Investing),
        ("financing", S::Financing),
    ] {
        let lines = cf.lines(sec);
        let got = j[k].as_array().unwrap();
        assert_eq!(got.len(), lines.len(), "{k}");
        for (g, l) in got.iter().zip(lines) {
            assert_eq!(g["entry"], json!(l.entry));
            assert_eq!(g["date"], l.date.to_string());
            assert_eq!(g["label"], l.label.as_str());
            assert_eq!(g["kind"], l.kind.as_str());
            assert_eq!(jamt(&g["amount"]), l.amount);
            if l.on_behalf.is_zero() {
                assert!(g.get("on_behalf").is_none(), "{g}");
            } else {
                assert_eq!(jamt(&g["on_behalf"]), l.on_behalf);
            }
            assert_eq!(g["accounts"], json!(l.accounts));
            assert_eq!(g["reverses"], json!(l.reverses));
            assert_eq!(g["class"], json!(l.class.map(|c| c.key())));
        }
        let by = if sec == S::Operating {
            "operating_by_class".to_string()
        } else {
            format!("{k}_by_kind")
        };
        let kinds = cf.kinds(sec);
        let got = j[&by].as_array().unwrap();
        assert_eq!(got.len(), kinds.len(), "{by}");
        for (g, kd) in got.iter().zip(&kinds) {
            assert_eq!(g["kind"], kd.kind.as_str());
            assert_eq!(jamt(&g["amount"]), kd.amount);
            assert_eq!(g["entries"], json!(kd.entries));
            assert_eq!(g["class"], json!(kd.class.map(|c| c.key())));
        }
    }
    let got = j["noncash_disclosures"].as_array().unwrap();
    assert_eq!(got.len(), cf.noncash_disclosures.len());
    for (g, n) in got.iter().zip(&cf.noncash_disclosures) {
        assert_eq!(g["entry"], json!(n.entry));
        assert_eq!(g["date"], n.date.to_string());
        assert_eq!(g["label"], n.label.as_str());
        for (k, v) in [
            ("amount", n.amount),
            ("operating", n.operating),
            ("investing", n.investing),
            ("financing", n.financing),
            ("exchanged", n.exchanged),
        ] {
            assert_eq!(jamt(&g[k]), v, "disclosure {k}");
        }
        assert_eq!(g["reverses"], json!(n.reverses));
    }
    let rj = &j["operating_reconciliation"];
    assert_eq!(jamt(&rj["net_income"]), r.net_income);
    assert_eq!(
        jamt(&rj["net_cash_from_operating"]),
        r.net_cash_from_operating
    );
    let got = rj["lines"].as_array().unwrap();
    assert_eq!(got.len(), r.lines.len());
    for (g, l) in got.iter().zip(&r.lines) {
        assert_eq!(g["kind"], l.kind.key());
        assert_eq!(g["label"], l.label.as_str());
        assert_eq!(jamt(&g["amount"]), l.amount);
        assert_eq!(g["account"], json!(l.account));
        assert_eq!(g["entries"], json!(l.entries));
    }
    assert_eq!(
        jamt(&j["reconciles"]["sections_sum"]),
        cf.reconciles.sections_sum
    );
    assert_eq!(
        jamt(&j["reconciles"]["net_change"]),
        cf.reconciles.net_change
    );
    assert_eq!(j["reconciles"]["difference"], "0.00");
    let got = j["warnings"].as_array().unwrap();
    assert_eq!(got.len(), cf.warnings.len());
    for (g, w) in got.iter().zip(&cf.warnings) {
        assert_eq!(g["entry"], json!(w.entry));
        assert_eq!(g["kind"], w.kind.key());
        assert_eq!(g["message"], w.message.as_str());
    }
    // The tool's answer is the export's JSON.
    let tool = m.ok(
        "books.cash_flow",
        json!({"start": START, "end": END, "cash_accounts": ["1000"], "financing_accounts": ["2100"]}),
    );
    assert_eq!(tool, j);
}

/// The journal export, CSV and JSON, over every date, is every posted
/// entry line by line (no drafts), as the ledger holds them. Kills: export_journal's
/// json/csv switch.
#[test]
fn journal_exports_hold_every_posted_line() {
    let dir = test_dir("journal");
    let mut s = new_store(&dir);
    rich_books(&mut s);
    let ledger = s.ledger().clone();
    drop(s);
    let posted: Vec<_> = ledger.posted_entries().collect();
    let mut m = Mcp::start(&dir, "1000000.00");

    for report in ["journal", "general_journal"] {
        // Every date: the journal covers a period (rules.md B-30).
        let csv = m.export_text(
            json!({"format": "csv", "report": report, "start": "0001-01-01", "end": "9999-12-31"}),
        );
        let rows = csv_rows(
            &csv,
            &[
                "entry_id",
                "date",
                "description",
                "reversal_of",
                "account_code",
                "account_name",
                "side",
                "amount",
                "currency",
            ],
        );
        let want: Vec<Vec<String>> = posted
            .iter()
            .flat_map(|e| {
                e.lines().iter().map(|l| {
                    let a = ledger.chart().get(l.account).unwrap();
                    vec![
                        e.id().to_string(),
                        e.date().to_string(),
                        e.description().to_string(),
                        e.reversal_of().map(|r| r.to_string()).unwrap_or_default(),
                        a.code.clone(),
                        a.name.clone(),
                        side_str(l.side).to_string(),
                        l.amount.amount.to_string(),
                        l.amount.currency.code().to_string(),
                    ]
                })
            })
            .collect();
        assert_eq!(rows.len(), want.len(), "{report}: a row per posted line");
        for (g, w) in rows.iter().zip(&want) {
            assert_eq!(g[..7], w[..7], "{report}");
            assert_eq!(amt(&g[7]), dec(&w[7]), "{report}");
            assert_eq!(g[8], w[8]);
        }
    }

    let j: Value = serde_json::from_str(&m.export_text(
        json!({"format": "json", "report": "journal", "start": "0001-01-01", "end": "9999-12-31"}),
    ))
    .unwrap();
    let got = j.as_array().unwrap();
    assert_eq!(got.len(), posted.len(), "a JSON entry per posted entry");
    for (g, e) in got.iter().zip(&posted) {
        assert_eq!(g["entry_id"], json!(e.id()));
        assert_eq!(g["date"], e.date().to_string());
        assert_eq!(g["description"], e.description());
        assert_eq!(g["reversal_of"], json!(e.reversal_of()));
        let lines = g["lines"].as_array().unwrap();
        assert_eq!(lines.len(), e.lines().len());
        for (gl, l) in lines.iter().zip(e.lines()) {
            let a = ledger.chart().get(l.account).unwrap();
            assert_eq!(gl["account_code"], a.code.as_str());
            assert_eq!(gl["account_name"], a.name.as_str());
            assert_eq!(gl["side"], side_str(l.side));
            assert_eq!(jamt(&gl["amount"]), l.amount.amount);
            assert_eq!(gl["currency"], l.amount.currency.code());
        }
    }
}

/// General ledger, tax summary, and members' equity exports equal the core
/// reports for the same range.
#[test]
fn ledger_tax_and_equity_exports_equal_the_core_reports() {
    let dir = test_dir("gl-tax-me");
    let mut s = new_store(&dir);
    rich_books(&mut s);
    let ledger = s.ledger().clone();
    let gl = ledger_core::general_ledger(&ledger, d(START), d(END)).unwrap();
    let ts = ledger_core::tax_summary(&ledger, d(START), d(END), None).unwrap();
    let me = ledger_core::members_equity(
        &ledger,
        d(START),
        d(END),
        &[
            MemberCapital {
                name: "A".into(),
                capital_account: id_of(&s, "3000"),
                distribution_account: None,
                share: dec("0.5"),
            },
            MemberCapital {
                name: "B".into(),
                capital_account: id_of(&s, "3001"),
                distribution_account: None,
                share: dec("0.5"),
            },
        ],
    )
    .unwrap();
    drop(s);
    let mut m = Mcp::start(&dir, "1000000.00");
    let range = |format: &str, report: &str| json!({"format": format, "report": report, "start": START, "end": END});

    // General ledger, CSV.
    let csv = m.export_text(range("csv", "general_ledger"));
    let rows = csv_rows(
        &csv,
        &[
            "account",
            "date",
            "entry",
            "description",
            "split",
            "debit",
            "credit",
            "balance",
        ],
    );
    let mut it = rows.iter();
    for a in &gl.accounts {
        let label = format!("{} {}", a.code, a.name);
        let beg = it.next().expect("beginning row");
        assert_eq!(beg[0], label);
        assert_eq!(beg[3], "Beginning balance");
        assert_eq!(amt(&beg[7]), a.beginning_balance, "{label} beginning");
        for p in &a.postings {
            let r = it.next().expect("posting row");
            assert_eq!(r[0], label);
            assert_eq!(r[1], p.date.to_string());
            assert_eq!(r[2], p.entry_id.to_string());
            assert_eq!(r[3], p.description);
            assert_eq!(r[4], p.split);
            assert_eq!(amt(&r[5]), p.debit);
            assert_eq!(amt(&r[6]), p.credit);
            assert_eq!(amt(&r[7]), p.balance);
        }
        let t = it.next().expect("total row");
        assert_eq!(t[0], label);
        assert_eq!(t[3], "Total");
        assert_eq!(amt(&t[5]), a.total_debits);
        assert_eq!(amt(&t[6]), a.total_credits);
        assert_eq!(amt(&t[7]), a.ending_balance);
    }
    assert!(it.next().is_none(), "no rows past the last account");

    // General ledger, JSON.
    let j: Value = serde_json::from_str(&m.export_text(range("json", "general_ledger"))).unwrap();
    assert_eq!(j["start"], START);
    assert_eq!(j["end"], END);
    let accts = j["accounts"].as_array().unwrap();
    assert_eq!(accts.len(), gl.accounts.len());
    for (g, a) in accts.iter().zip(&gl.accounts) {
        assert_eq!(g["code"], a.code.as_str());
        assert_eq!(jamt(&g["beginning_balance"]), a.beginning_balance);
        assert_eq!(jamt(&g["ending_balance"]), a.ending_balance);
        assert_eq!(jamt(&g["total_debits"]), a.total_debits);
        assert_eq!(jamt(&g["total_credits"]), a.total_credits);
        let ps = g["postings"].as_array().unwrap();
        assert_eq!(ps.len(), a.postings.len());
        for (gp, p) in ps.iter().zip(&a.postings) {
            assert_eq!(gp["entry_id"], json!(p.entry_id));
            assert_eq!(gp["split"], p.split.as_str());
            assert_eq!(jamt(&gp["balance"]), p.balance);
        }
    }

    // Tax summary, CSV.
    let csv = m.export_text(range("csv", "tax_summary"));
    let rows = csv_rows(
        &csv,
        &["section", "category", "label", "accounts", "book", "tax"],
    );
    let mut want: Vec<Vec<String>> = Vec::new();
    for (section, lines) in [
        ("income", &ts.income),
        ("cost_of_goods", &ts.cost_of_goods),
        ("deductions", &ts.deductions),
        ("separately_stated", &ts.separately_stated),
    ] {
        for l in lines {
            want.push(vec![
                section.into(),
                l.category.clone(),
                l.label.clone(),
                l.accounts.join("; "),
                l.book.to_string(),
                l.tax.to_string(),
            ]);
        }
    }
    // Book amounts only: Form 4797 figures the tax gain.
    for l in &ts.form_4797 {
        want.push(vec![
            "form_4797".into(),
            l.category.clone(),
            l.label.clone(),
            l.accounts.join("; "),
            l.book.to_string(),
            String::new(),
        ]);
    }
    for u in &ts.uncategorized {
        want.push(vec![
            "uncategorized".into(),
            String::new(),
            String::new(),
            u.account.clone(),
            u.book.to_string(),
            String::new(),
        ]);
    }
    assert!(!ts.deductions.is_empty() && !ts.uncategorized.is_empty());
    assert_eq!(rows.len(), want.len(), "tax summary rows");
    for (g, w) in rows.iter().zip(&want) {
        assert_eq!(g[..4], w[..4]);
        assert_eq!(amt(&g[4]), dec(&w[4]));
        if w[5].is_empty() {
            assert_eq!(g[5], "");
        } else {
            assert_eq!(amt(&g[5]), dec(&w[5]));
        }
    }

    // Tax summary, JSON.
    let j: Value = serde_json::from_str(&m.export_text(range("json", "tax_summary"))).unwrap();
    assert_eq!(jamt(&j["book_net"]), ts.book_net());
    assert_eq!(jamt(&j["tax_net"]), ts.tax_net());
    assert_eq!(
        j["deductions"].as_array().unwrap().len(),
        ts.deductions.len()
    );
    for (g, l) in j["deductions"]
        .as_array()
        .unwrap()
        .iter()
        .zip(&ts.deductions)
    {
        assert_eq!(g["category"], l.category.as_str());
        assert_eq!(jamt(&g["book"]), l.book);
        assert_eq!(jamt(&g["tax"]), l.tax);
    }
    assert_eq!(
        j["uncategorized"].as_array().unwrap().len(),
        ts.uncategorized.len()
    );

    // Members' equity, CSV and JSON.
    let csv = m.export_text(range("csv", "members_equity"));
    let rows = csv_rows(
        &csv,
        &[
            "member",
            "beginning",
            "contributions",
            "net_income",
            "distributions",
            "other",
            "ending",
        ],
    );
    let cols: Vec<_> = me
        .members
        .iter()
        .chain(std::iter::once(&me.total))
        .collect();
    assert_eq!(rows.len(), cols.len());
    for (r, c) in rows.iter().zip(&cols) {
        assert_eq!(r[0], c.name);
        let got: Vec<Decimal> = r[1..].iter().map(|x| amt(x)).collect();
        assert_eq!(
            got,
            vec![
                c.beginning,
                c.contributions,
                c.net_income,
                c.distributions,
                c.other,
                c.ending
            ],
            "members' equity {}",
            c.name
        );
    }
    let j: Value = serde_json::from_str(&m.export_text(range("json", "members_equity"))).unwrap();
    let members = j["members"].as_array().unwrap();
    assert_eq!(members.len(), me.members.len());
    for (g, c) in members
        .iter()
        .chain(std::iter::once(&j["total"]))
        .zip(&cols)
    {
        assert_eq!(g["name"], c.name.as_str());
        assert_eq!(jamt(&g["contributions"]), c.contributions);
        assert_eq!(jamt(&g["net_income"]), c.net_income);
        assert_eq!(jamt(&g["distributions"]), c.distributions);
        assert_eq!(jamt(&g["ending"]), c.ending);
    }
}

// ------------------------------------------------- binary formats and IIF

/// XLSX for every report is a workbook holding the report's content; PDF
/// for every statement is a PDF. Kills: export_xlsx's general_ledger and
/// journal arms, layout_statement's tax_summary arm.
#[test]
fn xlsx_and_pdf_exports_are_real_files_with_the_books_in_them() {
    let dir = test_dir("xlsx-pdf");
    let mut s = new_store(&dir);
    rich_books(&mut s);
    drop(s);
    let mut m = Mcp::start(&dir, "1000000.00");
    let args = |format: &str, report: &str| json!({"format": format, "report": report, "start": START, "end": END, "as_of": END});
    for report in [
        "profit_and_loss",
        "balance_sheet",
        "cash_flow",
        "trial_balance",
        "members_equity",
        "tax_summary",
    ] {
        let pdf = m.export(args("pdf", report));
        assert!(pdf.starts_with(b"%PDF-"), "{report} PDF");
        let text = xlsx_text(&m.export(args("xlsx", report)));
        assert!(
            text.contains("Oracle Co"),
            "{report} XLSX names the company"
        );
    }
    // The ledgers: by account and entry by entry, for the range only.
    let gl = xlsx_text(&m.export(args("xlsx", "general_ledger")));
    for name in ["Bank", "Rent", "Depreciation Expense"] {
        assert!(gl.contains(name), "general ledger XLSX has {name}");
    }
    for report in ["journal", "general_journal"] {
        let gj = xlsx_text(&m.export(args("xlsx", report)));
        assert!(gj.contains("Rent January"), "{report} XLSX has the rent");
        assert!(gj.contains("Contractor work"));
        assert!(
            !gj.contains("Prior-year sale") && !gj.contains("After mid-year"),
            "{report} XLSX keeps to the range"
        );
        assert!(!gj.contains("Never posted"), "{report} XLSX has no drafts");
    }
}

/// IIF: accounts declared with QuickBooks types per the documented mapping
/// (cash accounts BANK, 1200 AR, 2000 AP, a registered asset's accounts
/// FIXASSET, the rest by type), then one balanced TRNS/SPL/ENDTRNS block
/// per posted entry dated on or before as_of, numbered (DOCNUM) with its
/// entry id (rules.md B-33, B-34); plain ASCII text with CR LF, what it
/// writes differently (a line break and double quotes, the engine's own em
/// dashes) in warnings, grouped. Kills: export_iif's AR and AP comparisons.
#[test]
fn iif_declares_accounts_by_role_and_balances_every_transaction() {
    let dir = test_dir("iif");
    let mut s = new_store(&dir);
    rich_books(&mut s);
    let ledger = s.ledger().clone();
    drop(s);
    let mut m = Mcp::start(&dir, "1000000.00");
    let out = m.ok(
        "books.export",
        json!({"format": "iif", "report": "journal", "as_of": END}),
    );
    assert!(out.get("content_encoding").is_none(), "plain text: {out}");
    let lunch = ledger
        .posted_entries()
        .find(|e| e.description().starts_with("Lunch"))
        .unwrap()
        .id();
    // The chart's `Meals, "client"`, its double quotes written single;
    // then the memos, grouped by what is written differently: the lunch's
    // double quotes and line break, and the em dashes of every
    // depreciation memo in one line.
    let months: Vec<String> = ledger
        .posted_entries()
        .filter(|e| e.description().starts_with("Depreciation"))
        .map(|e| e.id().to_string())
        .collect();
    assert_eq!(months.len(), 3);
    let warnings = vec![
        "account 6010's name \"Meals, \\\"client\\\"\" is written \"Meals, 'client'\"".to_string(),
        format!("\"\\\"\" is written \"'\" in the memo of entry {lunch}"),
        format!("\"\\\"\\n\" is written \"' \" in the memo of entry {lunch}"),
        format!(
            "\"—\" is written \"--\" in the memos of entries {}",
            months.join(", ")
        ),
    ];
    assert_eq!(out["warnings"], json!(warnings));
    let iif = out["content"].as_str().unwrap();
    assert!(iif.ends_with("\r\n") && !iif.replace("\r\n", "").contains(['\r', '\n']));

    let mut types: HashMap<String, String> = HashMap::new();
    let mut header: Vec<&str> = Vec::new();
    let mut docnums: Vec<u64> = Vec::new();
    let mut sum = Decimal::ZERO;
    let mut in_trns = false;
    for line in iif.split("\r\n") {
        let f: Vec<&str> = line.split('\t').collect();
        match f[0] {
            "!ACCNT" => header = f.clone(),
            "ACCNT" => {
                let col = |name: &str| f[header.iter().position(|h| *h == name).unwrap()];
                types.insert(col("ACCNUM").to_string(), col("ACCNTTYPE").to_string());
            }
            "TRNS" | "SPL" => {
                if f[0] == "TRNS" {
                    assert!(!in_trns, "TRNS inside a transaction");
                    in_trns = true;
                    sum = Decimal::ZERO;
                    docnums.push(f[7].parse().unwrap());
                }
                assert_eq!(
                    f[7],
                    docnums.last().unwrap().to_string(),
                    "DOCNUM on every line"
                );
                sum += dec(f[6]);
            }
            "ENDTRNS" => {
                assert!(in_trns);
                assert_eq!(sum, Decimal::ZERO, "an IIF transaction sums to zero");
                in_trns = false;
            }
            _ => {}
        }
    }
    let through: Vec<u64> = ledger
        .posted_entries()
        .filter(|e| e.date() <= d(END))
        .map(|e| e.id())
        .collect();
    assert_eq!(docnums, through, "every posted entry through as_of, by id");
    for (code, ty) in [
        ("1000", "BANK"),
        ("1200", "AR"),
        ("1300", "OCASSET"),
        ("1500", "FIXASSET"),
        ("1510", "FIXASSET"),
        ("2000", "AP"),
        ("2100", "OCLIAB"),
        ("3000", "EQUITY"),
        ("4000", "INC"),
        ("6000", "EXP"),
    ] {
        assert_eq!(
            types.get(code).map(String::as_str),
            Some(ty),
            "account {code}'s IIF type"
        );
    }
    // general_journal is the report's name; journal its alias (both work).
    let alias = m.ok(
        "books.export",
        json!({"format": "iif", "report": "general_journal", "as_of": END}),
    );
    assert_eq!(alias["content"], out["content"]);
    // The file holds everything from the first entry: a range is refused,
    // and so is another report.
    let why = m.refuse(
        "books.export",
        json!({"format": "iif", "report": "journal", "start": START, "end": END}),
    );
    assert!(why.contains("iif takes as_of, not start or end"), "{why}");
    let why = m.refuse(
        "books.export",
        json!({"format": "iif", "report": "trial_balance"}),
    );
    assert!(
        why.contains("only valid for the general_journal report"),
        "{why}"
    );
}

/// P3 review R5: books.export iif without as_of is as of today, like the
/// trial balance: an entry dated later is left out, and the two exports
/// made with no dates tie, account by account.
#[test]
fn iif_without_as_of_is_as_of_today_and_ties_to_the_trial_balance() {
    let dir = test_dir("iif-today");
    let mut s = new_store(&dir);
    rich_books(&mut s);
    post(
        &mut s,
        "2099-01-04",
        "Far future rent",
        &[("6000", Dr, "1234.00"), ("1000", Cr, "1234.00")],
    );
    drop(s);
    let mut m = Mcp::start(&dir, "1000000.00");
    let iif = m.export_text(json!({"format": "iif", "report": "journal"}));
    assert!(!iif.contains("Far future rent"));
    let mut number: HashMap<String, String> = HashMap::new();
    let mut iif_bal: BTreeMap<String, Decimal> = BTreeMap::new();
    for line in iif.split("\r\n") {
        let f: Vec<&str> = line.split('\t').collect();
        match f[0] {
            "ACCNT" => {
                number.insert(f[1].to_string(), f[3].to_string());
            }
            "TRNS" | "SPL" => *iif_bal.entry(number[f[4]].clone()).or_default() += dec(f[6]),
            _ => {}
        }
    }
    iif_bal.retain(|_, v| !v.is_zero());
    let tb = m.export_text(json!({"format": "csv", "report": "trial_balance"}));
    let mut tb_bal: BTreeMap<String, Decimal> = BTreeMap::new();
    for row in csv_rows(&tb, &["code", "name", "debit", "credit"]) {
        if row[0] != "TOTAL" {
            tb_bal.insert(row[0].clone(), dec(&row[2]) - dec(&row[3]));
        }
    }
    tb_bal.retain(|_, v| !v.is_zero());
    assert_eq!(iif_bal, tb_bal);
}

// ------------------------------------------------- P&L by class

/// Per-class revenue, expenses, and net equal `profit_and_loss_for_period`
/// for the class; the unclassified row is the whole period's P&L less the
/// classes (they partition it). Boundaries are inclusive. Kills: the date
/// filter, the end-before-start check, the class filter, and the
/// drop-empty-rows filter.
#[test]
fn pnl_by_class_equals_the_core_pnl_per_class() {
    let dir = test_dir("by-class");
    let mut s = new_store(&dir);
    let r = rich_books(&mut s);
    let ledger = s.ledger().clone();
    drop(s);
    let mut m = Mcp::start(&dir, "1000000.00");

    let check = |m: &mut Mcp, start: &str, end: &str| {
        let out = m.ok(
            "books.profit_and_loss_by_class",
            json!({"start": start, "end": end}),
        );
        let total =
            ledger_core::profit_and_loss_for_period(&ledger, d(start), d(end), None).unwrap();
        let mut want: Vec<(Option<String>, Decimal, Decimal)> = Vec::new();
        let (mut rev, mut exp) = (Decimal::ZERO, Decimal::ZERO);
        let mut classes: Vec<_> = [r.rondo, r.coach]
            .iter()
            .map(|c| ledger.get_class(*c).unwrap().clone())
            .collect();
        classes.sort_by(|a, b| a.name.cmp(&b.name));
        for c in classes {
            let p = ledger_core::profit_and_loss_for_period(&ledger, d(start), d(end), Some(c.id))
                .unwrap();
            rev += p.total_revenue;
            exp += p.total_expenses;
            if !(p.total_revenue.is_zero() && p.total_expenses.is_zero()) {
                want.push((Some(c.name.clone()), p.total_revenue, p.total_expenses));
            }
        }
        let (urev, uexp) = (total.total_revenue - rev, total.total_expenses - exp);
        if !(urev.is_zero() && uexp.is_zero()) {
            want.push((None, urev, uexp));
        }
        let got: Vec<_> = out["classes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| {
                let (rv, ex) = (jamt(&c["revenue"]), jamt(&c["expenses"]));
                assert_eq!(jamt(&c["net_income"]), rv - ex, "net = revenue - expenses");
                (c["class"].as_str().map(String::from), rv, ex)
            })
            .collect();
        assert_eq!(got, want, "P&L by class {start}..{end}");
        got
    };

    // Whole year: Coach has revenue only, Rondo both, plus unclassified.
    let year = check(&mut m, "2025-01-01", "2025-12-31");
    assert_eq!(year.len(), 3);
    // Boundaries on entry dates: the rent (Jan 15) and the Rondo sale
    // (Feb 1) are in; the prior-year sale and the Coach sale are out.
    let edge = check(&mut m, "2025-01-15", "2025-02-01");
    assert_eq!(edge.len(), 1, "only Rondo moves in Jan 15..Feb 1");
    // One day.
    let day = check(&mut m, "2025-02-10", "2025-02-10");
    assert_eq!(day.len(), 1, "only Coach on Feb 10");
    m.refuse(
        "books.profit_and_loss_by_class",
        json!({"start": "2025-02-02", "end": "2025-02-01"}),
    );

    // One class: only its row.
    let only = m.ok(
        "books.profit_and_loss_by_class",
        json!({"start": "2025-01-01", "end": "2025-12-31", "class": "Coach"}),
    );
    let rows = only["classes"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "class filter keeps one row: {only}");
    assert_eq!(rows[0]["class"], "Coach");
    let coach =
        ledger_core::profit_and_loss_for_period(&ledger, d(START), d("2025-12-31"), Some(r.coach))
            .unwrap();
    assert_eq!(jamt(&rows[0]["revenue"]), coach.total_revenue);
}

/// A period's P&L by class still shows what each class earned after the
/// period is closed: closing entries are excluded, as in
/// `profit_and_loss_for_period` (docs: "Closing entries are excluded, so a
/// closed period still shows what it earned").
#[test]
fn pnl_by_class_excludes_closing_entries() {
    let dir = test_dir("by-class-closed");
    let mut s = new_store(&dir);
    let r = rich_books(&mut s);
    let p = s
        .create_period("2025-01", d("2025-01-01"), d("2025-01-31"))
        .unwrap();
    s.close_period(p).unwrap();
    let ledger = s.ledger().clone();
    drop(s);
    let mut m = Mcp::start(&dir, "1000000.00");
    let out = m.ok(
        "books.profit_and_loss_by_class",
        json!({"start": "2025-01-01", "end": "2025-01-31"}),
    );
    let rondo = ledger_core::profit_and_loss_for_period(
        &ledger,
        d("2025-01-01"),
        d("2025-01-31"),
        Some(r.rondo),
    )
    .unwrap();
    let all =
        ledger_core::profit_and_loss_for_period(&ledger, d("2025-01-01"), d("2025-01-31"), None)
            .unwrap();
    // January: only the Rondo rent moves P&L; nothing is unclassified.
    assert_eq!(all.net_income(), rondo.net_income());
    let rows = out["classes"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "only Rondo, no unclassified row: {out}");
    assert_eq!(jamt(&rows[0]["expenses"]), rondo.total_expenses);
}

// ------------------------------------------------- 1099

/// The worksheet equals `vendor_1099_report` (for the server's cash and
/// financing accounts); `missing_w9` is the vendors a 1099 is or may be
/// due for without a W-9; the untagged-entries note counts the year's
/// untagged entries that paid money out for something (the report's
/// untagged_payments, listed as untagged_entries); the indexing note
/// appears only after 2026. Kills: vendor_1099's year/tag filter and both
/// note guards.
#[test]
fn vendor_1099_equals_the_core_report_and_counts_untagged_entries() {
    let dir = test_dir("1099");
    let mut s = new_store(&dir);
    let contractor = rich_books(&mut s).contractor;
    // A 2027 payment to the contractor, tagged, so 2027 has no untagged.
    let e = draft(
        &mut s,
        "2027-03-01",
        "Contractor 2027",
        &[("6999", Dr, "2500.00"), ("1000", Cr, "2500.00")],
    );
    s.set_entry_vendor(e, Some(contractor)).unwrap();
    s.post_entry(e).unwrap();
    let ledger = s.ledger().clone();
    drop(s);
    let untagged = |year: i32| {
        core_1099(&ledger, year, Some(Decimal::ZERO))
            .untagged_payments
            .len()
    };
    assert!(untagged(2025) > 0 && untagged(2025) != untagged(2024));
    let mut m = Mcp::start(&dir, "1000000.00");

    for (year, threshold) in [(2025, None), (2026, None), (2027, Some("2000.00"))] {
        let mut args = json!({"year": year});
        if let Some(t) = threshold {
            args["threshold"] = json!(t);
        }
        let out = m.ok("books.vendor_1099", args);
        let want = core_1099(&ledger, year, threshold.map(dec));
        assert_eq!(out["year"], json!(want.year));
        assert_eq!(out["untagged_entries"], json!(want.untagged_payments));
        assert_eq!(jamt(&out["threshold"]), want.threshold);
        let vendors = out["vendors"].as_array().unwrap();
        assert_eq!(vendors.len(), want.vendors.len(), "{year} vendors");
        for (g, w) in vendors.iter().zip(&want.vendors) {
            assert_eq!(g["vendor_id"], json!(w.vendor_id));
            assert_eq!(g["name"], w.name.as_str());
            assert_eq!(jamt(&g["paid"]), w.paid, "{year} {}", w.name);
            assert_eq!(jamt(&g["excluded_card"]), w.excluded_card);
            assert_eq!(jamt(&g["undetermined"]), w.undetermined);
            assert_eq!(jamt(&g["not_reportable"]), w.not_reportable);
            assert_eq!(g["status"], w.status.key());
            assert_eq!(g["decision_1099"]["value"], w.decision.key());
            assert_eq!(g["reportable"], json!(w.reportable));
            assert_eq!(g["tin_on_file"], json!(w.tin_on_file));
        }
        let missing: Vec<String> = want
            .vendors
            .iter()
            .filter(|v| !v.tin_on_file)
            .filter_map(|v| {
                if v.reportable {
                    Some(v.name.clone())
                } else if v.status == ledger_core::Status1099::Undetermined
                    && v.decisions.iter().any(|d| d.decision.form().is_some())
                {
                    Some(format!("{} (may be reportable)", v.name))
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(out["missing_w9"], json!(missing), "{year} missing W-9s");

        let notes: Vec<&str> = out["notes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n.as_str().unwrap())
            .collect();
        let tag_note = notes.iter().find(|n| n.contains("no vendor tag"));
        match untagged(year) {
            0 => assert!(tag_note.is_none(), "{year}: no untagged note: {notes:?}"),
            n => {
                let note = tag_note.unwrap_or_else(|| panic!("{year}: untagged note: {notes:?}"));
                let count: usize = note.split_whitespace().next().unwrap().parse().unwrap();
                assert_eq!(count, n, "{year}: untagged count");
            }
        }
        let indexed = notes.iter().any(|n| n.contains("indexed for inflation"));
        assert_eq!(indexed, year > 2026, "{year}: indexing note: {notes:?}");
    }
    assert_eq!(untagged(2026), 0);
    assert_eq!(untagged(2027), 0);
}

// ------------------------------------------------- fixed assets

/// Dispose previews: accumulated, book value, gain/loss, and final
/// depreciation equal the store's disposal preview (`disposal_amounts`
/// plus the final month); `receipt_required` is exactly "the disposal
/// entry's debits exceed the threshold", the debits being accumulated +
/// proceeds + any loss. Probed at thresholds one cent under and exactly at
/// the debits, for a loss and a gain. Then a confirmed disposal posts
/// the previewed lines. Kills: dispose_asset's debits arithmetic.
#[test]
fn dispose_asset_previews_equal_the_store_and_gate_receipts_on_debits() {
    let dir = test_dir("dispose");
    let mut s = new_store(&dir);
    let reg = |s: &mut Store, desc: &str| {
        s.register_asset(NewAsset {
            description: desc.into(),
            in_service_date: d("2025-01-01"),
            cost: usd("1200.00"),
            salvage_value: usd("0.00"),
            useful_life_months: 12,
            cost_account: id_of(s, "1500"),
            accum_depr_account: id_of(s, "1510"),
            expense_account: id_of(s, "6500"),
        })
        .unwrap()
    };
    post(
        &mut s,
        "2025-01-01",
        "Two machines",
        &[("1500", Dr, "2400.00"), ("1000", Cr, "2400.00")],
    );
    let loss = reg(&mut s, "Loss machine");
    let gain = reg(&mut s, "Gain machine");
    for p in ["2025-01", "2025-02", "2025-03"] {
        s.run_depreciation(loss, p).unwrap();
        s.run_depreciation(gain, p).unwrap();
    }
    let cases = [(loss, "300.00"), (gain, "1000.00")];
    let mut want = Vec::new();
    for (asset, proceeds) in cases {
        let p = s
            .dispose_preview(asset, d("2025-04-30"), usd(proceeds))
            .unwrap();
        let a = s.ledger().get_asset(asset).unwrap();
        let core = ledger_core::disposal_amounts(a, d("2025-04-30"), usd(proceeds)).unwrap();
        assert_eq!(p.book_value, core.book_value);
        assert_eq!(p.gain_loss, core.gain_loss);
        let debits = p.accumulated + dec(proceeds) + (-p.gain_loss).max(Decimal::ZERO);
        want.push((asset, proceeds, p, debits));
    }
    assert!(want[0].2.gain_loss < Decimal::ZERO && want[1].2.gain_loss > Decimal::ZERO);
    let bank_before = s.ledger().account_balance(id_of(&s, "1000")).unwrap();
    drop(s);

    let preview = |m: &mut Mcp, asset: u64, proceeds: &str| {
        m.ok(
            "books.dispose_asset",
            json!({"asset_id": asset, "date": "2025-04-30", "proceeds": proceeds,
                   "proceeds_account": "1000", "gain_loss_account": "7100"}),
        )
    };
    let mut thresholds = Vec::new();
    for (_, _, _, debits) in &want {
        thresholds.push(*debits - dec("0.01"));
        thresholds.push(*debits);
    }
    for t in thresholds {
        let mut m = Mcp::start(&dir, &t.to_string());
        for (asset, proceeds, p, debits) in &want {
            let out = preview(&mut m, *asset, proceeds);
            assert_eq!(out["status"], "preview");
            assert_eq!(jamt(&out["accumulated"]), p.accumulated);
            assert_eq!(jamt(&out["book_value"]), p.book_value);
            assert_eq!(jamt(&out["gain_loss"]), p.gain_loss);
            assert_eq!(jamt(&out["final_depreciation"]), p.final_depreciation);
            assert_eq!(jamt(&out["proceeds"]), dec(proceeds));
            let line_debits: Decimal = out["lines"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|l| l["side"] == "debit")
                .map(|l| jamt(&l["amount"]))
                .sum();
            assert_eq!(line_debits, *debits, "the preview's debit lines");
            assert_eq!(
                out["receipt_required"],
                json!(*debits > t),
                "asset {asset}, debits {debits}, threshold {t}"
            );
        }
    }

    // Confirmed: posts the previewed amounts.
    let mut m = Mcp::start(&dir, "1000000.00");
    let (asset, proceeds, p, _) = &want[0];
    let out = m.ok(
        "books.dispose_asset",
        json!({"asset_id": asset, "date": "2025-04-30", "proceeds": proceeds,
               "proceeds_account": "1000", "gain_loss_account": "7100", "confirmed": true}),
    );
    assert_eq!(out["status"], "posted");
    let entry = out["entry_id"].as_u64().unwrap();
    drop(m);
    dir.stop();
    let s = Store::open(&data_dir(&dir)).unwrap();
    let l = s.ledger();
    assert_eq!(
        l.get_entry(entry).unwrap().status(),
        EntryStatus::Posted,
        "the disposal is posted"
    );
    assert_eq!(
        l.account_balance(id_of(&s, "7100")).unwrap(),
        -p.gain_loss,
        "a loss lands as expense"
    );
    assert_eq!(
        l.account_balance(id_of(&s, "1000")).unwrap(),
        bank_before + dec(proceeds),
        "proceeds land in the bank"
    );
}

/// Run-all depreciation posts each asset what its schedule says is due
/// through the month's end (plan/served.md P3, settled in design: a month
/// skipped is caught up in the month the run is made), and skips an asset
/// not yet in service, or one with nothing due. Hand check: "Short-lived"
/// is 600.00 over 2 months from 2025-01-01 (life Jan-Feb 2025), never run;
/// "Long-lived" 600.00 over 12 months from 2025-05-01, 50.00 a month, May
/// never run. A June run posts the first's whole 600.00 (all caught up)
/// and the second's May and June, 100.00 (50.00 caught up). An April run
/// then skips both: the second isn't in service yet, and the first's life
/// is over with June's run carrying all of it. Kills: run_depreciation's
/// catch-up and its skips.
#[test]
fn run_depreciation_catches_up_and_skips_what_has_nothing_due() {
    let dir = test_dir("depr");
    let mut s = new_store(&dir);
    let reg = |s: &mut Store, desc: &str, date: &str, months: u32| {
        s.register_asset(NewAsset {
            description: desc.into(),
            in_service_date: d(date),
            cost: usd("600.00"),
            salvage_value: usd("0.00"),
            useful_life_months: months,
            cost_account: id_of(s, "1500"),
            accum_depr_account: id_of(s, "1510"),
            expense_account: id_of(s, "6500"),
        })
        .unwrap()
    };
    let old = reg(&mut s, "Short-lived", "2025-01-01", 2);
    let new = reg(&mut s, "Long-lived", "2025-05-01", 12);
    let sched = depreciation_schedule(s.ledger().get_asset(new).unwrap(), d("2025-06-30")).unwrap();
    let june = sched.iter().find(|l| l.period == "2025-06").unwrap().amount;
    assert_eq!(june, dec("50.00"));
    drop(s);
    let mut m = Mcp::start(&dir, "1000000.00");
    let out = m.ok("books.run_depreciation", json!({"period": "2025-06"}));
    let runs = out["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 2, "{out}");
    let run = |id: u64| {
        runs.iter()
            .find(|r| r["asset_id"] == json!(id))
            .unwrap_or_else(|| panic!("{id} in {out}"))
            .clone()
    };
    assert_eq!(jamt(&run(old)["amount"]), dec("600.00"));
    assert_eq!(jamt(&run(old)["caught_up"]), dec("600.00"));
    assert_eq!(jamt(&run(new)["amount"]), dec("100.00"));
    assert_eq!(jamt(&run(new)["caught_up"]), dec("50.00"));
    assert_eq!(out["skipped"], json!([]));
    let out = m.ok("books.run_depreciation", json!({"period": "2025-04"}));
    assert_eq!(out["runs"], json!([]), "{out}");
    let skipped = out["skipped"].as_array().unwrap();
    assert_eq!(skipped.len(), 2, "{out}");
    let why = |id: u64| {
        skipped.iter().find(|r| r["asset_id"] == json!(id)).unwrap()["reason"]
            .as_str()
            .unwrap()
            .to_string()
    };
    assert!(why(new).contains("placed in service 2025-05-01"), "{out}");
    assert!(why(old).contains("life is over"), "{out}");
}

// ------------------------------------------------- payroll

/// Remitting clears every withholding payable, and the reported total is
/// what payroll put in them: gross − net + employer taxes. Kills:
/// remit_payroll_taxes's total accumulation.
#[test]
fn remit_payroll_total_is_what_payroll_withheld_and_owed() {
    let dir = test_dir("payroll");
    let s = new_store(&dir);
    drop(s);
    let mut m = Mcp::start(&dir, "1000000.00");
    let lines = [
        (
            "1000.00", "100.00", "50.00", "62.00", "14.50", "62.00", "14.50", "5.00",
        ),
        (
            "2000.00", "250.00", "80.00", "124.00", "29.00", "124.00", "29.00", "0.00",
        ),
    ];
    let mut owed = Decimal::ZERO;
    let payroll_lines: Vec<Value> = lines
        .iter()
        .enumerate()
        .map(|(i, (g, fed, st, ss, med, sser, meder, other))| {
            owed += [fed, st, ss, med, sser, meder, other]
                .iter()
                .map(|x| dec(x))
                .sum::<Decimal>();
            json!({"employee_name": format!("E{i}"), "gross_pay": g,
                   "federal_withholding": fed, "state_withholding": st,
                   "ss_employee": ss, "medicare_employee": med, "ss_employer": sser,
                   "medicare_employer": meder, "other_deductions": other})
        })
        .collect();
    post_contribution(&mut m);
    m.ok(
        "books.run_payroll",
        json!({"date": "2025-02-15", "cash_account": "1000", "lines": payroll_lines}),
    );
    let out = m.ok(
        "books.remit_payroll_taxes",
        json!({"date": "2025-02-20", "cash_account": "1000"}),
    );
    assert_eq!(jamt(&out["total"]), owed);
    let entry = out["entry_id"].as_u64().unwrap();
    drop(m);
    dir.stop();
    let s = Store::open(&data_dir(&dir)).unwrap();
    let e = s.ledger().get_entry(entry).unwrap();
    let cash = id_of(&s, "1000");
    let paid: Decimal = e
        .lines()
        .iter()
        .filter(|l| l.account == cash && l.side == Cr)
        .map(|l| l.amount.amount)
        .sum();
    assert_eq!(paid, owed, "the remittance pays out the total");
    for code in ["2110", "2120", "2130", "2140", "2150"] {
        if let Some(a) = s.ledger().chart().find_by_code(code) {
            assert!(
                s.ledger().account_balance(a.id).unwrap().is_zero(),
                "{code}"
            );
        }
    }
}

/// Owner contribution of 10000.00 to 1000 through the MCP flow.
fn post_contribution(m: &mut Mcp) -> u64 {
    let e = m.ok(
        "books.draft_entry",
        json!({"date": "2025-01-02", "description": "Contribution"}),
    )["entry_id"]
        .as_u64()
        .unwrap();
    for (a, side) in [("1000", "debit"), ("3000", "credit")] {
        m.ok(
            "books.add_line",
            json!({"entry_id": e, "account": a, "side": side, "amount": "10000.00"}),
        );
    }
    m.ok(
        "books.attach_receipt",
        json!({"entry_id": e, "file_name": "c.txt", "mime_type": "text/plain",
               "content_base64": b64e(b"contribution"), "sha256": sha256(b"contribution")}),
    );
    m.ok(
        "books.post_entry",
        json!({"entry_id": e, "confirmed": true}),
    );
    e
}

// ------------------------------------------------- digest

/// The digest lists exactly the posted entries dated on or after `since`
/// (newest first), with their debits and receipt coverage per the policy
/// (strictly above the threshold); per-account movement equals the trial
/// balance difference across the period, on each account's normal side.
/// Kills: digest's date filter and nonzero filter, movement_in.
#[test]
fn digest_equals_the_journal_since_a_date() {
    let dir = test_dir("digest");
    let mut s = new_store(&dir);
    post(
        &mut s,
        "2025-02-28",
        "Before",
        &[("1000", Dr, "5000.00"), ("3000", Cr, "5000.00")],
    );
    let on = post(
        &mut s,
        "2025-03-01",
        "On the day",
        &[("1000", Dr, "300.00"), ("4000", Cr, "300.00")],
    );
    let after = post(
        &mut s,
        "2025-03-15",
        "After",
        &[("6000", Dr, "80"), ("2100", Cr, "80")],
    );
    let big = post(
        &mut s,
        "2025-03-20",
        "Big",
        &[("6999", Dr, "150.00"), ("1000", Cr, "150.00")],
    );
    s.attach_receipt(Some(on), "on.pdf", "application/pdf", b"on", None)
        .unwrap();
    draft(
        &mut s,
        "2025-03-25",
        "Draft",
        &[("6999", Dr, "1.00"), ("1000", Cr, "1.00")],
    );
    let ledger = s.ledger().clone();
    let receipts: HashMap<u64, usize> = ledger
        .posted_entries()
        .map(|e| (e.id(), s.list_attachments(Some(e.id())).unwrap().len()))
        .collect();
    drop(s);
    let since = "2025-03-01";
    let today = chrono::Local::now().date_naive();
    let mut m = Mcp::start(&dir, "100.00");
    let out = m.ok("books.digest", json!({"since": since}));
    assert_eq!(out["since"], since);

    let mut want: Vec<_> = ledger
        .posted_entries()
        .filter(|e| e.date() >= d(since))
        .collect();
    want.sort_by(|a, b| b.date().cmp(&a.date()).then(b.id().cmp(&a.id())));
    let got = out["entries"].as_array().unwrap();
    assert_eq!(
        got.iter()
            .map(|e| e["entry_id"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        want.iter().map(|e| e.id()).collect::<Vec<_>>(),
        "entries on or after since, newest first"
    );
    assert_eq!(want.len(), 3);
    for (g, e) in got.iter().zip(&want) {
        let debits: Decimal = e
            .lines()
            .iter()
            .filter(|l| l.side == Dr)
            .map(|l| l.amount.amount)
            .sum();
        assert_eq!(jamt(&g["total"]), debits);
        assert_eq!(g["date"], e.date().to_string());
        assert_eq!(g["description"], e.description());
        let count = receipts[&e.id()];
        let required = debits > dec("100.00");
        assert_eq!(g["receipt_count"], json!(count));
        assert_eq!(g["receipt_required"], json!(required), "entry {}", e.id());
        assert_eq!(g["receipt_missing"], json!(required && count == 0));
    }
    assert!(
        got.iter()
            .any(|g| g["entry_id"] == json!(big) && g["receipt_missing"] == true)
    );
    assert!(got.iter().any(|g| g["entry_id"] == json!(after)));

    // Movement: trial balance through today minus through the day before.
    let tb_net = |as_of: chrono::NaiveDate| -> HashMap<String, Decimal> {
        reports::trial_balance_as_of(&ledger, as_of)
            .lines
            .into_iter()
            .map(|l| (l.code, l.debit - l.credit))
            .collect()
    };
    let (now, before) = (tb_net(today), tb_net(d(since).pred_opt().unwrap()));
    let mut want: BTreeMap<String, Decimal> = BTreeMap::new();
    for a in ledger.chart().all() {
        let net = now.get(&a.code).copied().unwrap_or_default()
            - before.get(&a.code).copied().unwrap_or_default();
        let m = normal_signed(&ledger, a.id, Dr, net);
        if !m.is_zero() {
            want.insert(a.code.clone(), m);
        }
    }
    let got: BTreeMap<String, Decimal> = out["totals_by_account"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            (
                t["code"].as_str().unwrap().to_string(),
                jamt(&t["movement"]),
            )
        })
        .collect();
    assert_eq!(got, want, "movement per account");
    assert!(want.values().any(|v| *v > Decimal::ZERO) && want.contains_key("2100"));
}

/// A torn journal tail set aside on open shows in the digest for any
/// `since` on or before the repair, and not after. Kills: digest's
/// journal_repairs filter.
#[test]
fn digest_reports_journal_repairs_since_the_date() {
    let dir = test_dir("repair");
    let mut s = new_store(&dir);
    post(
        &mut s,
        "2025-01-02",
        "Seed",
        &[("1000", Dr, "1.00"), ("4000", Cr, "1.00")],
    );
    drop(s);
    let journal = data_dir(&dir).join(ledger_store::JOURNAL_FILE);
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(&journal)
        .unwrap();
    f.write_all(b"{\"seq\": 999, \"torn").unwrap();
    drop(f);
    let mut m = Mcp::start(&dir, "1000000.00");
    let out = m.ok("books.digest", json!({"since": "2025-01-01"}));
    let repairs = out["journal_repairs"].as_array().unwrap();
    drop(m);
    dir.stop();
    let s = Store::open(&data_dir(&dir)).unwrap();
    let want = s.tail_repairs().unwrap();
    drop(s);
    assert_eq!(want.len(), 1, "the store set the torn tail aside");
    assert_eq!(repairs.len(), 1, "the digest reports it: {out}");
    assert_eq!(repairs[0]["at"], want[0].0.as_str());
    assert_eq!(repairs[0]["bytes"], json!(want[0].1));
    // Only the stamp the bytes were set aside under: never the journal's
    // file name (plan I-12).
    let stamp = want[0].2.rsplit_once(".torn.").unwrap().1;
    assert_eq!(repairs[0]["set_aside"], stamp);
    assert!(
        !out.to_string().contains(ledger_store::JOURNAL_FILE),
        "{out}"
    );
    let mut m = Mcp::start(&dir, "1000000.00");
    let later = m.ok("books.digest", json!({"since": "2999-01-01"}));
    assert_eq!(later["journal_repairs"], json!([]), "none since 2999");
}

// ------------------------------------------------- the viewer

/// `books.show` writes the viewer command for exactly the target asked
/// for; bad report columns, comparisons, and pages are refused, good ones
/// accepted. Kills: show's validations, the attachment lookup, and the
/// columns description.
#[test]
fn show_writes_the_viewer_command_for_each_target() {
    let dir = test_dir("show");
    let mut s = new_store(&dir);
    let e = post(
        &mut s,
        "2025-01-02",
        "Seed",
        &[("1000", Dr, "1.00"), ("4000", Cr, "1.00")],
    );
    let (a1, _, _) = s
        .attach_receipt(
            Some(e),
            "alpha-receipt.pdf",
            "application/pdf",
            b"one",
            None,
        )
        .unwrap();
    let (a2, _, _) = s
        .attach_receipt(None, "omega-receipt.pdf", "application/pdf", b"two", None)
        .unwrap();
    drop(s);
    let mut m = Mcp::start(&dir, "1000000.00");

    m.ok("books.show", json!({"entry_id": e}));
    assert_eq!(
        m.viewer_command().target,
        ViewerTarget::Entry { entry_id: e }
    );
    m.refuse("books.show", json!({"entry_id": 999}));

    m.ok(
        "books.show",
        json!({"account": "1000", "start": "2025-01-01", "end": "2025-01-31"}),
    );
    assert_eq!(
        m.viewer_command().target,
        ViewerTarget::Account {
            code: "1000".into(),
            start: Some("2025-01-01".into()),
            end: Some("2025-01-31".into())
        }
    );

    // A split, or one total column compared (the viewer compares only a
    // report in one total column: defect 15, options a target ignores are
    // refused).
    for (columns, compare) in [
        (Some("month"), None),
        (Some("quarter"), None),
        (Some("year"), None),
        (Some("total"), Some("prior_period")),
        (None, Some("prior_year")),
    ] {
        let out = m.ok(
            "books.show",
            json!({"report": "profit_and_loss", "start": "2025-01-01", "end": "2025-12-31",
                   "columns": columns, "compare": compare}),
        );
        assert_eq!(
            m.viewer_command().target,
            ViewerTarget::Report {
                report: "profit_and_loss".into(),
                start: Some("2025-01-01".into()),
                end: Some("2025-12-31".into()),
                columns: columns.map(Into::into),
                compare: compare.map(Into::into),
            }
        );
        let shown = out["shown"].as_str().unwrap();
        assert!(shown.contains("2025-01-01 to 2025-12-31"), "{out}");
        if let Some(c) = columns.filter(|c| *c != "total") {
            assert!(shown.contains(&format!("by {c}")), "{out}");
        }
        if let Some(c) = compare {
            assert!(shown.contains(&c.replace('_', " ")), "{out}");
        }
    }
    for (columns, compare) in [("month", "prior_year"), ("quarter", "prior_period")] {
        let e = m.refuse(
            "books.show",
            json!({"report": "profit_and_loss", "columns": columns, "compare": compare}),
        );
        assert!(e.contains("one total column"), "{e}");
    }
    m.refuse(
        "books.show",
        json!({"report": "profit_and_loss", "columns": "weekly"}),
    );
    m.refuse(
        "books.show",
        json!({"report": "profit_and_loss", "compare": "prior_decade"}),
    );
    m.refuse("books.show", json!({"report": "horoscope"}));

    for (id, name, other) in [
        (a1, "alpha-receipt.pdf", "omega-receipt.pdf"),
        (a2, "omega-receipt.pdf", "alpha-receipt.pdf"),
    ] {
        let out = m.ok("books.show", json!({"attachment_id": id}));
        assert_eq!(
            m.viewer_command().target,
            ViewerTarget::Document { attachment_id: id }
        );
        let shown = out["shown"].as_str().unwrap();
        assert!(
            shown.contains(name) && !shown.contains(other),
            "shown names attachment {id}: {shown}"
        );
    }

    m.ok("books.show", json!({"search": "coffee"}));
    assert_eq!(
        m.viewer_command().target,
        ViewerTarget::Search {
            query: "coffee".into()
        }
    );

    for page in [
        "overview",
        "cash",
        "profit",
        "transactions",
        "reports",
        "accounts",
        "documents",
        "company",
        "journal",
    ] {
        m.ok("books.show", json!({"page": page}));
        assert_eq!(
            m.viewer_command().target,
            ViewerTarget::Page { page: page.into() }
        );
    }
    m.refuse("books.show", json!({"page": "settings"}));
    m.refuse("books.show", json!({}));
}

/// The command `books.show` last wrote, or None before any.
fn last_show(m: &Mcp) -> Option<ledger_store::ops::ViewerCommand> {
    ledger_store::ops::read(&m.data)
}

/// Defect 15, repros 1.1.1 and 1.1.2: a start after the end, given, is
/// refused for a report and for an account, and nothing reaches the
/// viewer (the last command it was sent stays the last). A start equal to
/// the end is a one-day period.
#[test]
fn show_refuses_start_after_end_defect_15() {
    let dir = test_dir("show-inverted");
    drop(new_store(&dir));
    let mut m = Mcp::start(&dir, "1000000.00");
    m.ok("books.show", json!({"page": "overview"}));
    let before = last_show(&m).unwrap();
    for args in [
        json!({"report": "profit_and_loss", "start": "2026-12-31", "end": "2026-01-01"}),
        json!({"account": "1000", "start": "2026-12-31", "end": "2026-01-01"}),
        json!({"report": "cash_flow", "start": "2026-01-02", "end": "2026-01-01"}),
        json!({"report": "balance_sheet", "columns": "month", "start": "2026-03-01", "end": "2026-02-28"}),
    ] {
        let e = m.refuse("books.show", args.clone());
        assert!(e.contains("is before start"), "{args}: {e}");
        assert_eq!(last_show(&m), Some(before.clone()), "{args}");
    }
    let out = m.ok(
        "books.show",
        json!({"report": "profit_and_loss", "start": "2026-03-01", "end": "2026-03-01"}),
    );
    assert!(
        out["shown"]
            .as_str()
            .unwrap()
            .contains("2026-03-01 to 2026-03-01"),
        "{out}"
    );
}

/// Defect 15, repro 1.1.3: a start after today with no end (the end an
/// omitted end means) is refused, saying the end was defaulted; nothing
/// reaches the viewer.
#[test]
fn show_refuses_a_derived_inverted_range_defect_15() {
    let dir = test_dir("show-derived");
    drop(new_store(&dir));
    let mut m = Mcp::start(&dir, "1000000.00");
    m.ok("books.show", json!({"page": "journal"}));
    let before = last_show(&m).unwrap();
    for args in [
        json!({"account": "1000", "start": "9999-12-31"}),
        json!({"report": "general_ledger", "start": "9999-01-01"}),
    ] {
        let e = m.refuse("books.show", args.clone());
        assert!(
            e.contains("defaults to today") && e.contains("pass end"),
            "{args}: {e}"
        );
        assert_eq!(last_show(&m), Some(before.clone()), "{args}");
    }
}

/// Defect 16, repro 1.1.4: books.show sends the dates it names, resolved
/// by the one default (rules.md B-30): only an end covers that end's
/// fiscal year to it (2025-01-01 to 2025-06-30, never today's year's
/// start); no dates cover today's fiscal year to date (checked against
/// the clock read before and after, R14); a report as of a day says "as
/// of", vendor_1099 "for" its year; and the viewer gets the same pair.
#[test]
fn show_sends_the_dates_it_names_defect_16() {
    let dir = test_dir("show-dates");
    drop(new_store(&dir));
    let mut m = Mcp::start(&dir, "1000000.00");
    let target = |m: &Mcp| match last_show(m).unwrap().target {
        ViewerTarget::Report { start, end, .. } | ViewerTarget::Account { start, end, .. } => {
            (start.unwrap(), end.unwrap())
        }
        other => panic!("not dated: {other:?}"),
    };
    for (args, shown, dates) in [
        (
            json!({"report": "profit_and_loss", "end": "2025-06-30"}),
            "the profit and loss report, 2025-01-01 to 2025-06-30",
            ("2025-01-01", "2025-06-30"),
        ),
        (
            json!({"account": "1000", "end": "2025-06-30"}),
            "the register of 1000 Bank, 2025-01-01 to 2025-06-30",
            ("2025-01-01", "2025-06-30"),
        ),
        (
            json!({"report": "profit_and_loss", "start": "2025-03-01", "end": "2025-03-31"}),
            "the profit and loss report, 2025-03-01 to 2025-03-31",
            ("2025-03-01", "2025-03-31"),
        ),
        (
            json!({"report": "trial_balance", "end": "2025-06-30"}),
            "the trial balance report, as of 2025-06-30",
            ("2025-01-01", "2025-06-30"),
        ),
        (
            json!({"report": "balance_sheet", "end": "2025-06-30"}),
            "the balance sheet report, as of 2025-06-30",
            ("2025-01-01", "2025-06-30"),
        ),
        (
            json!({"report": "balance_sheet", "end": "2025-06-30", "columns": "quarter"}),
            "the balance sheet report, 2025-01-01 to 2025-06-30, by quarter",
            ("2025-01-01", "2025-06-30"),
        ),
        (
            json!({"report": "fixed_asset_register", "end": "2024-12-31"}),
            "the fixed asset register report, as of 2024-12-31",
            ("2024-01-01", "2024-12-31"),
        ),
        (
            json!({"report": "ar_aging", "end": "2025-02-15"}),
            "the ar aging report, as of 2025-02-15",
            ("2025-01-01", "2025-02-15"),
        ),
        (
            json!({"report": "vendor_1099", "end": "2025-12-31"}),
            "the vendor 1099 report, for 2025",
            ("2025-01-01", "2025-12-31"),
        ),
    ] {
        let out = m.ok("books.show", args.clone());
        assert_eq!(out["shown"], shown, "{args}");
        assert_eq!(
            target(&m),
            (dates.0.to_string(), dates.1.to_string()),
            "{args}"
        );
    }
    // No dates: today's fiscal year to date, whichever day the call saw.
    let before = ledger_core::settings::today_in(None);
    let out = m.ok("books.show", json!({"report": "profit_and_loss"}));
    let after = ledger_core::settings::today_in(None);
    let (s, e) = target(&m);
    let ok = [before, after].iter().any(|t| {
        let (fy, _) = ledger_core::settings::Company::default().fiscal_year_containing(*t);
        (s.clone(), e.clone()) == (fy.to_string(), t.to_string())
            && out["shown"] == format!("the profit and loss report, {fy} to {t}")
    });
    assert!(ok, "{s} to {e}: {out}");
}

/// Defect 15 (settled: books.show refuses options its target ignores):
/// dates, columns or a comparison for an entry, a page or any other
/// non-dated target; columns or a comparison for an account; columns for
/// a report that doesn't split; a comparison for one that doesn't compare;
/// a start for a report as of a day, or for the 1099 worksheet. Each names
/// what takes the option, and nothing reaches the viewer.
#[test]
fn show_refuses_options_its_target_ignores_defect_15() {
    let dir = test_dir("show-options");
    let mut s = new_store(&dir);
    let e = post(
        &mut s,
        "2025-01-02",
        "Seed",
        &[("1000", Dr, "1.00"), ("4000", Cr, "1.00")],
    );
    drop(s);
    let mut m = Mcp::start(&dir, "1000000.00");
    m.ok("books.show", json!({"page": "overview"}));
    let before = last_show(&m).unwrap();
    for (args, says) in [
        (
            json!({"entry_id": e, "start": "2025-01-01"}),
            "for an account or a report",
        ),
        (
            json!({"page": "reports", "end": "2025-01-31"}),
            "for an account or a report",
        ),
        (
            json!({"search": "x", "columns": "month"}),
            "for an account or a report",
        ),
        (
            json!({"entry_id": e, "start": "2025-01-01", "compare": "prior_year"}),
            "start and compare are",
        ),
        (
            json!({"account": "1000", "columns": "month"}),
            "takes only start and end",
        ),
        (
            json!({"account": "1000", "compare": "prior_year"}),
            "takes only start and end",
        ),
        (
            json!({"report": "trial_balance", "columns": "total"}),
            "columns is for profit_and_loss, balance_sheet and cash_flow",
        ),
        (
            json!({"report": "tax_summary", "columns": "month"}),
            "columns is for",
        ),
        (
            json!({"report": "general_ledger", "compare": "prior_year"}),
            "compare is for profit_and_loss, balance_sheet, cash_flow and tax_summary",
        ),
        (
            json!({"report": "trial_balance", "start": "2025-01-01", "end": "2025-03-31"}),
            "is as of a day: pass end",
        ),
        (
            json!({"report": "balance_sheet", "start": "2025-01-01", "end": "2025-03-31"}),
            "in one total column is as of a day",
        ),
        (
            json!({"report": "vendor_1099", "start": "2025-01-01"}),
            "is for a calendar year",
        ),
        // A split past the viewer's 240 columns, which it shows as the
        // total: 312 months.
        (
            json!({"report": "profit_and_loss", "start": "2000-01-01", "end": "2025-12-31", "columns": "month"}),
            "is 312 columns, and the viewer splits a report into at most 240",
        ),
        (
            json!({"report": "balance_sheet", "start": "0001-01-01", "end": "2025-12-31", "columns": "year"}),
            "at most 240",
        ),
    ] {
        let err = m.refuse("books.show", args.clone());
        assert!(err.contains(says), "{args}: {err}");
        assert_eq!(last_show(&m), Some(before.clone()), "{args}");
    }
    // What a balance sheet takes a start for: a split, or the prior period;
    // and 240 months is a split the viewer shows.
    for args in [
        json!({"report": "balance_sheet", "start": "2025-01-01", "end": "2025-03-31", "columns": "month"}),
        json!({"report": "balance_sheet", "start": "2025-01-01", "end": "2025-03-31", "compare": "prior_period"}),
        json!({"report": "profit_and_loss", "start": "2006-01-01", "end": "2025-12-31", "columns": "month"}),
        json!({"report": "profit_and_loss", "start": "0001-01-01", "end": "2025-12-31", "columns": "total"}),
    ] {
        m.ok("books.show", args);
    }
}

// ------------------------------------------------- resources

/// Each advertised resource reads back as the matching tool's result, and
/// receipts resolve by id; an unknown URI is "not found". Kills:
/// resource_text's balance-sheet, profit-and-loss, digest, and receipts
/// arms.
#[test]
fn resources_read_back_as_the_matching_tools() {
    let dir = test_dir("resources");
    let mut s = new_store(&dir);
    rich_books(&mut s);
    let (r1, _, _) = s
        .attach_receipt(None, "first.txt", "text/plain", b"first receipt", None)
        .unwrap();
    let (r2, _, _) = s
        .attach_receipt(None, "second.txt", "text/plain", b"second receipt", None)
        .unwrap();
    drop(s);
    let mut m = Mcp::start(&dir, "1000000.00");
    let list = m.rpc("resources/list", json!({}));
    let uris: Vec<&str> = list["result"]["resources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["uri"].as_str().unwrap())
        .collect();
    for u in [
        "books://chart",
        "books://reports/trial-balance",
        "books://reports/balance-sheet",
        "books://reports/profit-and-loss",
        "books://digest",
    ] {
        assert!(uris.contains(&u), "resources/list has {u}");
    }
    let read = |m: &mut Mcp, uri: &str| -> Value {
        let r = m.rpc("resources/read", json!({"uri": uri}));
        let text = r["result"]["contents"][0]["text"]
            .as_str()
            .unwrap_or_else(|| panic!("{uri}: {r}"));
        serde_json::from_str(text).unwrap()
    };
    for (uri, tool, args) in [
        ("books://chart", "books.chart", json!({})),
        (
            "books://reports/trial-balance",
            "books.trial_balance",
            json!({}),
        ),
        (
            "books://reports/balance-sheet",
            "books.balance_sheet",
            json!({}),
        ),
        (
            "books://reports/profit-and-loss",
            "books.profit_and_loss",
            json!({}),
        ),
        (
            "books://digest?since=2025-03-01",
            "books.digest",
            json!({"since": "2025-03-01"}),
        ),
    ] {
        let got = read(&mut m, uri);
        let want = m.ok(tool, args);
        assert_eq!(got, want, "{uri} reads as {tool}");
    }
    // A receipt reads as books.get_receipt's meta view: what it is, never
    // its bytes (those come with view bytes).
    for (id, bytes) in [(r1, &b"first receipt"[..]), (r2, &b"second receipt"[..])] {
        let got = read(&mut m, &format!("books://receipts/{id}"));
        assert_eq!(got["attachment_id"], json!(id));
        assert!(got.get("content_base64").is_none(), "no bytes: {got}");
        assert_eq!(got["sha256"], sha256(bytes));
        assert_eq!(got["size_bytes"], json!(bytes.len()));
        assert_eq!(
            got,
            m.ok("books.get_receipt", json!({"attachment_id": id})),
            "books://receipts/{id} reads as books.get_receipt"
        );
        let full = m.ok(
            "books.get_receipt",
            json!({"attachment_id": id, "view": "bytes"}),
        );
        assert_eq!(b64d(full["content_base64"].as_str().unwrap()), bytes);
    }
    let r = m.rpc("resources/read", json!({"uri": "books://nope"}));
    let err = r.get("error").unwrap_or_else(|| panic!("unknown URI: {r}"));
    assert!(
        err.to_string().contains("unknown resource"),
        "an unknown URI is reported as unknown, not as a bad receipt: {err}"
    );
}

// ------------------------------------------------- files in and out

/// Receipts and documents round-trip: the bytes come back exactly (SHA-256
/// of what was sent), with the name, MIME type, category, entry, and
/// description given; only view bytes carries the bytes. Kills:
/// get_receipt's lookup and its views.
#[test]
fn receipts_and_documents_round_trip() {
    let dir = test_dir("files");
    let s = new_store(&dir);
    drop(s);
    let mut m = Mcp::start(&dir, "1000000.00");
    let e = post_contribution(&mut m);
    let first: &[u8] = b"%PDF-1.4 first";
    let second: &[u8] = b"\x89PNG second \x00\xff";
    let a = m.ok(
        "books.attach_receipt",
        json!({"entry_id": e, "file_name": "first.pdf", "mime_type": "application/pdf",
               "content_base64": b64e(first), "sha256": sha256(first),
               "description": "Receipt ID 1007; Paid from: Quillmere"}),
    );
    let b = m.ok(
        "books.file_document",
        json!({"category": "legal", "file_name": "oa.png", "mime_type": "image/png",
               "content_base64": b64e(second), "sha256": sha256(second),
               "description": "Operating agreement"}),
    );
    let c = m.ok(
        "books.file_document",
        json!({"category": "tax", "file_name": "ein-letter.pdf", "mime_type": "application/pdf",
               "content_base64": b64e(b"EIN letter bytes"), "sha256": sha256(b"EIN letter bytes"),
               "description": "IRS CP 575"}),
    );
    let r = m.ok(
        "books.attach_receipt",
        json!({"entry_id": e, "file_name": "renamed.png", "mime_type": "image/png",
               "content_base64": b64e(b"a scanned receipt"),
               "sha256": sha256(b"a scanned receipt"), "description": "scan"}),
    );

    // (result, bytes, file name, MIME type, entry, description)
    type Case<'a> = (&'a Value, &'a [u8], &'a str, &'a str, Option<u64>, &'a str);
    let cases: [Case<'_>; 4] = [
        (
            &a,
            first,
            "first.pdf",
            "application/pdf",
            Some(e),
            "Receipt ID 1007; Paid from: Quillmere",
        ),
        (
            &b,
            second,
            "oa.png",
            "image/png",
            None,
            "Operating agreement",
        ),
        (
            &c,
            b"EIN letter bytes",
            "ein-letter.pdf",
            "application/pdf",
            None,
            "IRS CP 575",
        ),
        (
            &r,
            b"a scanned receipt",
            "renamed.png",
            "image/png",
            Some(e),
            "scan",
        ),
    ];
    let listed = m.ok("books.list_receipts", json!({}))["receipts"].clone();
    for (out, bytes, name, mime, entry, desc) in cases {
        let id = out["attachment_id"].as_u64().unwrap();
        assert_eq!(out["sha256"], sha256(bytes), "{name}: sha256 of the bytes");
        let meta = m.ok("books.get_receipt", json!({"attachment_id": id}));
        assert!(
            meta.get("content_base64").is_none(),
            "{name}: meta, no bytes"
        );
        let got = m.ok(
            "books.get_receipt",
            json!({"attachment_id": id, "view": "bytes"}),
        );
        assert_eq!(got["attachment_id"], json!(id));
        assert_eq!(
            b64d(got["content_base64"].as_str().unwrap()),
            bytes,
            "{name}"
        );
        let mut without = got.clone();
        without.as_object_mut().unwrap().remove("content_base64");
        assert_eq!(without, meta, "{name}: view bytes is meta and the bytes");
        assert_eq!(got["sha256"], sha256(bytes));
        assert_eq!(got["file_name"], name);
        assert_eq!(got["mime_type"], mime);
        assert_eq!(got["entry_id"], json!(entry));
        assert_eq!(got["size_bytes"], json!(bytes.len()));
        let row = listed
            .as_array()
            .unwrap()
            .iter()
            .find(|x| x["attachment_id"] == json!(id))
            .unwrap();
        assert_eq!(row["description"], desc, "{name}: description as given");
    }
    let mut cat = |out: &Value| {
        m.ok(
            "books.get_receipt",
            json!({"attachment_id": out["attachment_id"]}),
        )["category"]
            .clone()
    };
    assert_eq!(cat(&b), "legal");
    assert_eq!(cat(&c), "tax");
    m.refuse("books.get_receipt", json!({"attachment_id": 999}));
    for view in ["", "BYTES", "content", "meta "] {
        let why = m.refuse(
            "books.get_receipt",
            json!({"attachment_id": a["attachment_id"], "view": view}),
        );
        assert!(
            why.contains("meta (the default) or bytes"),
            "{view:?}: {why}"
        );
    }
}

/// 256 KiB is the most a file may be given inline (plan 2.1): exactly
/// the limit is accepted by books.attach_receipt and books.file_document
/// and stored whole, one byte more is refused by both, pointing at
/// books.begin_upload, and nothing is stored. Kills: the limit comparison
/// in take_file.
#[test]
fn inline_files_up_to_256_kib_are_accepted_and_no_more() {
    const LIMIT: usize = 256 * 1024;
    let dir = test_dir("limits");
    let s = new_store(&dir);
    drop(s);
    let mut m = Mcp::start(&dir, "1000000.00");
    let at = vec![b'a'; LIMIT];
    let out = m.ok(
        "books.attach_receipt",
        json!({"file_name": "big.bin", "mime_type": "application/octet-stream",
               "content_base64": b64e(&at), "sha256": sha256(&at)}),
    );
    let at_id = out["attachment_id"].as_u64().unwrap();
    let doc = vec![b'b'; LIMIT];
    let out = m.ok(
        "books.file_document",
        json!({"category": "other", "file_name": "big.bin",
               "mime_type": "application/octet-stream", "content_base64": b64e(&doc),
               "sha256": sha256(&doc)}),
    );
    let doc_id = out["attachment_id"].as_u64().unwrap();
    let listed = m.ok("books.list_receipts", json!({}));
    for (id, bytes) in [(at_id, &at), (doc_id, &doc)] {
        let row = listed["receipts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["attachment_id"] == json!(id))
            .unwrap()
            .clone();
        assert_eq!(row["size_bytes"], json!(LIMIT), "stored whole");
        // And it comes back inline, whole.
        let got = m.ok(
            "books.get_receipt",
            json!({"attachment_id": id, "view": "bytes"}),
        );
        assert_eq!(&b64d(got["content_base64"].as_str().unwrap()), bytes);
    }
    let journal = std::fs::read(data_dir(&dir).join(ledger_store::JOURNAL_FILE)).unwrap();
    let over = vec![b'c'; LIMIT + 1];
    for (tool, args) in [
        (
            "books.attach_receipt",
            json!({"file_name": "over.bin", "mime_type": "application/octet-stream",
                   "content_base64": b64e(&over), "sha256": sha256(&over)}),
        ),
        (
            "books.file_document",
            json!({"category": "other", "file_name": "over.bin",
                   "mime_type": "application/octet-stream", "content_base64": b64e(&over),
                   "sha256": sha256(&over)}),
        ),
    ] {
        let why = m.refuse(tool, args);
        assert!(
            why.contains("books.begin_upload") && why.contains(&(LIMIT + 1).to_string()),
            "{tool}: {why}"
        );
    }
    assert_eq!(
        std::fs::read(data_dir(&dir).join(ledger_store::JOURNAL_FILE)).unwrap(),
        journal,
        "a refusal stored something"
    );
}

/// The bytes a filing tool takes are inline base64 with its own SHA-256,
/// or an upload; never both, never neither. A sha256 missing, or not the
/// bytes' own (a byte lost on the way), or base64 that isn't, is refused
/// (guard base64_corrupt); inline bytes need their file_name and
/// mime_type. An upload_id that names no upload is refused, and
/// books.begin_upload names the attachment for bytes stored already, and
/// begins an upload for new ones (journaling nothing). Every refusal
/// stores nothing.
#[test]
fn inline_bytes_come_with_their_own_sha256_or_as_an_upload() {
    let dir = test_dir("inline");
    let s = new_store(&dir);
    drop(s);
    let mut m = Mcp::start(&dir, "1000000.00");
    let bytes: &[u8] = b"%PDF-1.4 a receipt";
    let other: &[u8] = b"%PDF-1.4 a receipt!";
    let journal = || std::fs::read(data_dir(&dir).join(ledger_store::JOURNAL_FILE)).unwrap();
    let before = journal();
    for (tool, base) in [
        ("books.attach_receipt", json!({})),
        ("books.file_document", json!({"category": "other"})),
    ] {
        let with = |extra: Value| {
            let mut args = base.clone();
            for (k, v) in json!({"file_name": "r.pdf", "mime_type": "application/pdf"})
                .as_object()
                .unwrap()
                .iter()
                .chain(extra.as_object().unwrap())
            {
                args[k] = v.clone();
            }
            args
        };
        for (args, said) in [
            // Neither, both.
            (with(json!({})), "books.begin_upload"),
            (
                with(json!({"upload_id": "u1", "content_base64": b64e(bytes),
                            "sha256": sha256(bytes)})),
                "not both",
            ),
            // The sha256 missing, another file's, not hex; base64 that isn't.
            (with(json!({"content_base64": b64e(bytes)})), "sha256"),
            (
                with(json!({"content_base64": b64e(bytes), "sha256": sha256(other)})),
                "base64_corrupt",
            ),
            (
                with(json!({"content_base64": b64e(bytes), "sha256": "not hex"})),
                "base64_corrupt",
            ),
            (
                with(json!({"content_base64": "!!not base64", "sha256": sha256(bytes)})),
                "base64_corrupt",
            ),
            // An upload there is none of.
            (
                with(json!({"upload_id": "u1"})),
                "there is no upload \"u1\"",
            ),
        ] {
            let why = m.refuse(tool, args.clone());
            assert!(why.contains(said), "{tool} {args}: {why}");
        }
        // Inline bytes need their name and type.
        for missing in ["file_name", "mime_type"] {
            let mut args = with(json!({"content_base64": b64e(bytes), "sha256": sha256(bytes)}));
            args.as_object_mut().unwrap().remove(missing);
            let why = m.refuse(tool, args);
            assert!(why.contains(missing), "{tool} without {missing}: {why}");
        }
    }
    // A sha256 given in capitals, or with spaces around it, is the same
    // hash.
    let out = m.ok(
        "books.attach_receipt",
        json!({"file_name": "r.pdf", "mime_type": "application/pdf",
               "content_base64": b64e(bytes),
               "sha256": format!(" {} ", sha256(bytes).to_ascii_uppercase())}),
    );
    assert_eq!(out["sha256"], sha256(bytes));
    let id = out["attachment_id"].as_u64().unwrap();
    assert_ne!(journal(), before);
    // begin_upload: the bytes stored already name their attachment; new
    // ones begin an upload.
    let up = |sha: String| {
        json!({"sha256": sha, "size": bytes.len(), "mime_type": "application/pdf",
               "file_name": "r.pdf", "purpose": "receipt"})
    };
    let got = m.ok("books.begin_upload", up(sha256(bytes)));
    assert_eq!(got["existing_attachment_id"], json!(id), "{got}");
    assert!(got.get("upload_id").is_none(), "{got}");
    assert!(got.get("command").is_none(), "{got}");
    let got = m.ok("books.begin_upload", up(sha256(bytes).to_ascii_uppercase()));
    assert_eq!(got["existing_attachment_id"], json!(id), "{got}");
    let stored = journal();
    let begun = m.ok("books.begin_upload", up(sha256(other)));
    assert!(begun["upload_id"].is_string(), "{begun}");
    assert_eq!(journal(), stored, "beginning an upload journals nothing");
    // The CSV imports take csv_text or an upload, never both, never
    // neither; an upload there is none of is refused.
    for (tool, base) in [
        (
            "books.import_bank_csv",
            json!({"bank_account_code": "1000", "default_offset_code": "4000"}),
        ),
        ("books.import_chart_csv", json!({})),
    ] {
        for (extra, said) in [
            (json!({}), "csv_text"),
            (json!({"csv_text": "a,b\n", "upload_id": "u1"}), "not both"),
            (json!({"upload_id": "u1"}), "there is no upload \"u1\""),
        ] {
            let mut args = base.clone();
            for (k, v) in extra.as_object().unwrap() {
                args[k] = v.clone();
            }
            let why = m.refuse(tool, args.clone());
            assert!(why.contains(said), "{tool} {args}: {why}");
        }
    }
    assert_eq!(journal(), stored, "a refusal stored something");
}

/// books.get_receipt with view "bytes" gives a file's bytes inline up to
/// 256 KiB; a larger one, its download (a stored file, fetched with
/// books:read by `GET /b/{book}/a/{id}`: those very bytes), never its
/// bytes inline.
#[test]
fn get_receipt_gives_bytes_up_to_256_kib_and_a_download_above() {
    const LIMIT: usize = 256 * 1024;
    let dir = test_dir("get-bytes");
    let mut s = new_store(&dir);
    let at = vec![b'x'; LIMIT];
    let over = vec![b'y'; LIMIT + 1];
    let (at_id, _, _) = s
        .attach_receipt(None, "at.bin", "application/octet-stream", &at, None)
        .unwrap();
    let (over_id, _, _) = s
        .attach_receipt(None, "over.bin", "application/octet-stream", &over, None)
        .unwrap();
    drop(s);
    let mut m = Mcp::start(&dir, "1000000.00");
    let got = m.ok(
        "books.get_receipt",
        json!({"attachment_id": at_id, "view": "bytes"}),
    );
    assert_eq!(b64d(got["content_base64"].as_str().unwrap()), at);
    let big = m.ok(
        "books.get_receipt",
        json!({"attachment_id": over_id, "view": "bytes"}),
    );
    assert!(big.get("content_base64").is_none(), "{big}");
    let download = big["download"].as_str().unwrap_or_else(|| panic!("{big}"));
    assert!(download.contains(&format!(" {over_id} ")), "{download}");
    assert_eq!(big["size_bytes"], json!(LIMIT + 1));
    assert_eq!(big["sha256"], sha256(&over));
    let (status, bytes) = m.wire.get(&format!("a/{over_id}"));
    assert_eq!(status, 200);
    assert_eq!(bytes, over);
    let meta = m.ok("books.get_receipt", json!({"attachment_id": over_id}));
    assert_eq!(meta["size_bytes"], json!(LIMIT + 1));
    assert_eq!(meta["sha256"], sha256(&over));
    assert!(meta.get("content_base64").is_none(), "{meta}");
}

/// Bank CSV import: money in debits the bank, money out credits it; the
/// longest matching rule picks the offset, else the default; day-first
/// dates read as given; amounts come back signed. Kills: import_bank_csv's
/// side choice.
#[test]
fn bank_import_puts_money_in_and_out_on_the_right_sides() {
    let dir = test_dir("bank");
    let s = new_store(&dir);
    drop(s);
    let mut m = Mcp::start(&dir, "1000000.00");
    let out = m.ok(
        "books.import_bank_csv",
        json!({
            "csv_text": "Date,Description,Amount\n\
                         03/04/2025,Corner Coffee Shop,-4.50\n\
                         13/04/2025,Client payment,250.00\n\
                         20/04/2025,STARBUCKS COFFEE #12,-6.00\n",
            "bank_account_code": "1000",
            "default_offset_code": "4000",
            "rules": {"coffee": "6010", "starbucks coffee": "6020"},
            "date_order": "day_first",
        }),
    );
    let want = [
        ("2025-04-03", "Corner Coffee Shop", "-4.50", "6010"),
        ("2025-04-13", "Client payment", "250.00", "4000"),
        ("2025-04-20", "STARBUCKS COFFEE #12", "-6.00", "6020"),
    ];
    let imported = out["imported"].as_array().unwrap();
    assert_eq!(out["count"], json!(want.len()));
    assert_eq!(imported.len(), want.len());
    for (g, (date, desc, amount, offset)) in imported.iter().zip(want) {
        assert_eq!(g["date"], date);
        assert_eq!(g["description"], desc);
        assert_eq!(jamt(&g["amount"]), dec(amount));
        assert_eq!(g["offset_account"], offset);
        let e = m.ok("books.get_entry", json!({"entry_id": g["entry_id"]}));
        assert_eq!(e["status"], "draft");
        assert_eq!(e["date"], date);
        let money_in = dec(amount) > Decimal::ZERO;
        let side_of = |code: &str| {
            let l = e["lines"]
                .as_array()
                .unwrap()
                .iter()
                .find(|l| l["code"] == code)
                .unwrap_or_else(|| panic!("line on {code}: {e}"));
            assert_eq!(jamt(&l["amount"]), dec(amount).abs());
            l["side"].as_str().unwrap().to_string()
        };
        assert_eq!(
            side_of("1000"),
            if money_in { "debit" } else { "credit" },
            "{desc}: bank side"
        );
        assert_eq!(
            side_of(offset),
            if money_in { "credit" } else { "debit" },
            "{desc}: offset side"
        );
    }
}

// ------------------------------------------------- references and lines

/// References (INVARIANT 6): a number that is one account's code and the
/// same account's id resolves; a vendor's or class's id that is another's
/// name is refused; a vendor or class named by its own id resolves; an
/// unknown vendor is reported as unknown. Kills: resolve_account's, resolve_vendor's, and
/// resolve_class's ambiguity guards, resolve_vendor's unknown arm.
#[test]
fn references_resolve_unless_truly_ambiguous() {
    let dir = test_dir("refs");
    let s = Store::create(&data_dir(&dir), Currency::USD).unwrap();
    drop(s);
    // Classes as an earlier version could write them (a name of digits
    // only can't be added now): 1 "placeholder-name", 2 named "1", 3
    // named "3".
    let mut w = ledger_store::JournalWriter::open(&data_dir(&dir).join(ledger_store::JOURNAL_FILE))
        .unwrap();
    for (i, n) in ["placeholder-name", "1", "3"].iter().enumerate() {
        w.append("class.added", json!({"id": i + 1, "name": n}))
            .unwrap();
    }
    drop(w);
    let mut m = Mcp::start(&dir, "1000000.00");
    // An account whose code is its own id.
    let first = m.ok(
        "books.add_account",
        json!({"code": "1000", "name": "Bank", "account_type": "asset"}),
    )["account_id"]
        .as_u64()
        .unwrap();
    m.ok(
        "books.renumber_account",
        json!({"account": "1000", "code": first.to_string()}),
    );
    m.ok(
        "books.add_account",
        json!({"code": "4000", "name": "Revenue", "account_type": "revenue"}),
    );
    let e = m.ok(
        "books.draft_entry",
        json!({"date": "2025-01-02", "description": "Sale"}),
    )["entry_id"]
        .as_u64()
        .unwrap();
    m.ok(
        "books.add_line",
        json!({"entry_id": e, "account": first.to_string(), "side": "debit", "amount": "5.00"}),
    );

    // Class 2 is named "1", class 1's id.
    let c = 1u64;
    // "c" is class c's id and class c2's name: ambiguous.
    m.refuse(
        "books.add_line",
        json!({"entry_id": e, "account": "4000", "side": "credit", "amount": "5.00",
               "class": c.to_string()}),
    );
    // c3 named by its own id: only one class is meant.
    let c3 = 3u64;
    m.ok(
        "books.add_line",
        json!({"entry_id": e, "account": "4000", "side": "credit", "amount": "5.00",
               "class": c3.to_string()}),
    );

    // Vendors: id 1 is "Alpha"; "1" is another vendor's name.
    let v1 = m.ok("books.add_vendor", json!({"name": "Alpha"}))["id"]
        .as_u64()
        .unwrap();
    let v2 = m.ok("books.add_vendor", json!({"name": v1.to_string()}))["id"]
        .as_u64()
        .unwrap();
    assert_ne!(v1, v2);
    m.refuse(
        "books.set_entry_vendor",
        json!({"entry_id": e, "vendor": v1.to_string()}),
    );
    m.ok(
        "books.set_entry_vendor",
        json!({"entry_id": e, "vendor": v2.to_string()}),
    );
    let got = m.ok("books.get_entry", json!({"entry_id": e}));
    assert!(!got["vendor"].is_null(), "tagged: {got}");
    // A vendor named by its own id: only one vendor is meant.
    let v3 = m.ok("books.add_vendor", json!({"name": (v2 + 1).to_string()}))["id"]
        .as_u64()
        .unwrap();
    assert_eq!(v3, v2 + 1, "vendors number in order");
    m.ok(
        "books.set_entry_vendor",
        json!({"entry_id": e, "vendor": v3.to_string()}),
    );
    let err = m.refuse(
        "books.set_entry_vendor",
        json!({"entry_id": e, "vendor": "Nobody Inc"}),
    );
    assert!(err.contains("unknown vendor"), "{err}");
}

/// A foreign-currency line needs all three of currency, amount, and rate:
/// any one of them alone is refused rather than read as a base line. Kills:
/// add_line's fx detection.
#[test]
fn a_partial_foreign_currency_line_is_refused() {
    let dir = test_dir("fx");
    let s = new_store(&dir);
    drop(s);
    let mut m = Mcp::start(&dir, "1000000.00");
    let e = m.ok(
        "books.draft_entry",
        json!({"date": "2025-01-02", "description": "EUR"}),
    )["entry_id"]
        .as_u64()
        .unwrap();
    for partial in [
        json!({"foreign_currency": "EUR"}),
        json!({"foreign_amount": "10.00"}),
        json!({"fx_rate": "1.10"}),
        json!({"foreign_amount": "10.00", "fx_rate": "1.10"}),
    ] {
        let mut args =
            json!({"entry_id": e, "account": "6999", "side": "debit", "amount": "11.00"});
        for (k, v) in partial.as_object().unwrap() {
            args[k] = v.clone();
        }
        m.refuse("books.add_line", args);
    }
    let got = m.ok("books.get_entry", json!({"entry_id": e}));
    assert_eq!(got["lines"], json!([]), "nothing was added");
    m.ok(
        "books.add_line",
        json!({"entry_id": e, "account": "6999", "side": "debit", "amount": "11.00",
               "foreign_currency": "EUR",
               "foreign_amount": "10.00", "fx_rate": "1.10"}),
    );
    let got = m.ok("books.get_entry", json!({"entry_id": e}));
    assert_eq!(jamt(&got["lines"][0]["amount"]), dec("11.00"));
}

/// Post and reverse previews show each line's balance before and after on
/// the account's normal side: a credit to an asset lowers it. Kills:
/// preview_lines_for's sign.
#[test]
fn previews_move_balances_by_normal_side() {
    let dir = test_dir("preview");
    let s = new_store(&dir);
    drop(s);
    let mut m = Mcp::start(&dir, "1000000.00");
    let c = post_contribution(&mut m);
    let e = m.ok(
        "books.draft_entry",
        json!({"date": "2025-01-05", "description": "Rent"}),
    )["entry_id"]
        .as_u64()
        .unwrap();
    for (a, side) in [("6000", "debit"), ("1000", "credit")] {
        m.ok(
            "books.add_line",
            json!({"entry_id": e, "account": a, "side": side, "amount": "1200"}),
        );
    }
    let p = m.ok("books.post_entry", json!({"entry_id": e}));
    assert_eq!(p["status"], "preview");
    let line = |p: &Value, code: &str| -> (Decimal, Decimal, Decimal) {
        let l = p["lines"]
            .as_array()
            .unwrap()
            .iter()
            .find(|l| l["code"] == code)
            .unwrap()
            .clone();
        (
            jamt(&l["amount"]),
            jamt(&l["balance_before"]),
            jamt(&l["balance_after"]),
        )
    };
    assert_eq!(
        line(&p, "1000"),
        (dec("1200"), dec("10000"), dec("8800")),
        "credit to an asset"
    );
    assert_eq!(line(&p, "6000"), (dec("1200"), dec("0"), dec("1200")));
    // Reversing the contribution: debit equity, credit the bank.
    let r = m.ok(
        "books.reverse_entry",
        json!({"entry_id": c, "date": "2025-01-06", "description": "Undo"}),
    );
    assert_eq!(
        line(&r, "1000"),
        (dec("10000"), dec("10000"), dec("0")),
        "reversal credits the bank"
    );
    assert_eq!(line(&r, "3000"), (dec("10000"), dec("10000"), dec("0")));
}

// ------------------------------------------------- CPA package

/// The CPA package holds the documented files for the fiscal year to date;
/// the receipts behind the year's posted entries (and only those), indexed
/// with each entry's date, description, and debits; the company's
/// documents but not loose receipts or withdrawn files; a README counting
/// the receipts; a 1099 worksheet equal to `vendor_1099_report` through
/// as_of (a payment after it to the same contractor left out); and the
/// journal as IIF from the first entry through as_of. With
/// include_receipts false, no evidence. Kills: export_cpa_package's
/// receipt and document guards, the receipt count, the file-name cleaner.
#[test]
fn cpa_package_holds_the_year_and_its_evidence() {
    let dir = test_dir("cpa");
    let mut s = new_store(&dir);
    let r = rich_books(&mut s);
    // The payment after as_of went to the contractor too.
    s.set_entry_vendor(r.after, Some(r.contractor)).unwrap();
    // Two receipts on the rent, same-length names.
    let rent_a: &[u8] = b"rent receipt A";
    let rent_b: &[u8] = b"rent receipt B";
    let (ra, _, _) = s
        .attach_receipt(
            Some(r.rent),
            "receipt-a.pdf",
            "application/pdf",
            rent_a,
            None,
        )
        .unwrap();
    let (rb, _, _) = s
        .attach_receipt(
            Some(r.rent),
            "receipt-b.pdf",
            "application/pdf",
            rent_b,
            None,
        )
        .unwrap();
    for (e, name) in [
        (r.prior, "old.pdf"),
        (r.after, "later.pdf"),
        (r.never, "draft.pdf"),
    ] {
        s.attach_receipt(Some(e), name, "application/pdf", name.as_bytes(), None)
            .unwrap();
    }
    s.attach_receipt(None, "loose.pdf", "application/pdf", b"loose", None)
        .unwrap();
    s.file_document(
        "legal",
        "agreement.pdf",
        "application/pdf",
        b"agreement",
        None,
    )
    .unwrap();
    let (gone, _, _) = s
        .file_document(
            "tax",
            "withdrawn.pdf",
            "application/pdf",
            b"withdrawn",
            None,
        )
        .unwrap();
    s.withdraw_attachment(gone, "filed by mistake").unwrap();
    let ledger = s.ledger().clone();
    let shas: HashMap<u64, String> = s
        .list_attachments(None)
        .unwrap()
        .into_iter()
        .map(|a| (a.id, a.blob_sha))
        .collect();
    drop(s);
    let rent = ledger.get_entry(r.rent).unwrap();
    let rent_debits: Decimal = rent
        .lines()
        .iter()
        .filter(|l| l.side == Dr)
        .map(|l| l.amount.amount)
        .sum();

    let mut m = Mcp::start(&dir, "1000000.00");
    let out = m.ok(
        "books.export_cpa_package",
        json!({"as_of": END, "cash_accounts": ["1000"], "include_tins": false}),
    );
    assert_eq!(out["file"]["file_name"], format!("cpa-package-{END}.zip"));
    assert_eq!(out["file"]["mime_type"], "application/zip");
    let files = unzip(&m.handed(&out["file"]));
    let names: Vec<&str> = files.keys().map(String::as_str).collect();

    let mut want = vec![
        "README.txt".to_string(),
        "general-ledger.xlsx".into(),
        "general-journal.xlsx".into(),
        "general-journal.iif".into(),
        "1099-worksheet-2025.csv".into(),
        "receipts/index.csv".into(),
    ];
    for st in [
        "profit-and-loss",
        "balance-sheet",
        "cash-flow",
        "members-equity",
        "tax-summary",
        "trial-balance",
    ] {
        want.push(format!("{st}.pdf"));
        want.push(format!("{st}.xlsx"));
    }
    for w in &want {
        assert!(files.contains_key(w), "package has {w}: {names:?}");
    }
    for st in ["profit-and-loss", "balance-sheet", "trial-balance"] {
        assert!(files[&format!("{st}.pdf")].starts_with(b"%PDF-"));
        xlsx_text(&files[&format!("{st}.xlsx")]);
    }
    let gj = xlsx_text(&files["general-journal.xlsx"]);
    assert!(gj.contains("Rent January") && !gj.contains("Prior-year sale"));
    assert!(!gj.contains("After mid-year"), "fiscal year to as_of only");
    // The IIF: from the first entry (the prior year's sale) through as_of.
    let iif = std::str::from_utf8(&files["general-journal.iif"]).unwrap();
    assert!(iif.contains("\tPrior-year sale\r\n") && iif.contains("\tRent January\r\n"));
    assert!(!iif.contains("After mid-year"), "through as_of only");
    assert!(iif.contains("\tLunch, 'team' quarterly\r\n"));
    // The meals account's double quotes, the lunch's and its line break,
    // and the em dashes of the depreciation memos: written differently, so
    // said, the memos grouped (four lines for one name and four memos), in
    // warnings and the README alike.
    let warnings: Vec<String> = out["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w.as_str().unwrap().to_string())
        .collect();
    assert_eq!(warnings.len(), 4, "{out}");
    let readme = std::str::from_utf8(&files["README.txt"]).unwrap();
    for w in &warnings {
        assert!(readme.contains(&format!("\n- {w}\n")), "{w} in {readme}");
    }
    assert!(out["note"].as_str().unwrap().ends_with(
        " 5 account or class name(s), number(s), place(s) or memo(s) are written differently in general-journal.iif to fit QuickBooks Desktop (warnings says how; so does the README): tell the human."
    ));

    // Receipts: exactly the two behind the rent.
    let receipt_files: Vec<&str> = names
        .iter()
        .copied()
        .filter(|n| n.starts_with("receipts/") && *n != "receipts/index.csv")
        .collect();
    assert_eq!(receipt_files.len(), 2, "{receipt_files:?}");
    for (id, name, bytes) in [(ra, "receipt-a.pdf", rent_a), (rb, "receipt-b.pdf", rent_b)] {
        let f = receipt_files
            .iter()
            .find(|n| n.ends_with(name))
            .unwrap_or_else(|| panic!("{name} is in receipts/: {receipt_files:?}"));
        assert_eq!(files[*f], bytes);
        assert_eq!(sha256(&files[*f]), shas[&id]);
    }
    let index = csv_rows(
        std::str::from_utf8(&files["receipts/index.csv"]).unwrap(),
        &[
            "entry_id",
            "date",
            "description",
            "amount",
            "file",
            "sha256",
        ],
    );
    assert_eq!(index.len(), 2);
    for row in &index {
        assert_eq!(row[0], r.rent.to_string());
        assert_eq!(row[1], rent.date().to_string());
        assert_eq!(row[2], rent.description());
        assert_eq!(amt(&row[3]), rent_debits);
        assert!(files.contains_key(&row[4]), "index names a file in the zip");
        assert_eq!(sha256(&files[&row[4]]), row[5]);
    }
    // Documents: the agreement, not the loose receipt or the withdrawn.
    let docs: Vec<&str> = names
        .iter()
        .copied()
        .filter(|n| n.starts_with("documents/"))
        .collect();
    assert_eq!(docs.len(), 1, "{docs:?}");
    assert!(docs[0].starts_with("documents/legal/") && docs[0].ends_with("agreement.pdf"));
    assert_eq!(files[docs[0]], b"agreement");
    assert!(
        !names
            .iter()
            .any(|n| n.contains("loose") || n.contains("withdrawn"))
    );
    let readme = String::from_utf8(files["README.txt"].clone()).unwrap();
    assert!(
        readme.contains("the 2 receipts"),
        "README counts the receipts: {readme}"
    );

    // 1099 worksheet.
    let w = csv_rows(
        std::str::from_utf8(&files["1099-worksheet-2025.csv"]).unwrap(),
        &[
            "vendor_id",
            "name",
            "decision_1099",
            "form_1099",
            "reason_kind",
            "reason",
            "decided_by",
            "decided_on",
            "effective",
            "decisions_in_year",
            "paid",
            "excluded_card",
            "undetermined",
            "not_reportable",
            "status",
            "reportable",
            "w9_on_file",
            "tin_last4",
        ],
    );
    // Through as_of (rules.md B-33), out of the package's cash account
    // and the settings' financing account (one list each per package).
    let want = core_1099_through(&ledger, 2025, None, Some(d(END)));
    assert_eq!(w.len(), want.vendors.len());
    let contractor = w.iter().find(|row| row[1] == "Contractor").unwrap();
    assert_eq!(
        amt(&contractor[10]),
        dec("900.00"),
        "the payment after as_of is left out"
    );
    for (row, v) in w.iter().zip(&want.vendors) {
        assert_eq!(row[0], v.vendor_id.to_string());
        assert_eq!(row[1], v.name);
        assert_eq!(row[2], v.decision.key());
        assert_eq!(row[3], v.form().map(|f| f.label()).unwrap_or(""));
        assert_eq!(
            row[6],
            v.decided.as_ref().map_or("", |s| s.decided_by.as_str())
        );
        assert_eq!(amt(&row[10]), v.paid);
        assert_eq!(amt(&row[11]), v.excluded_card);
        assert_eq!(amt(&row[12]), v.undetermined);
        assert_eq!(amt(&row[13]), v.not_reportable);
        assert_eq!(row[14], v.status.key());
        assert_eq!(row[15], v.reportable.to_string());
        assert_eq!(row[16], v.tin_on_file.to_string());
    }
    // The package's 1099 worksheet is books.vendor_1099's CSV for the
    // package's accounts and its as_of, byte for byte (ledger_package
    // INVARIANT 7, docs/mcp-interface.md): the tool given the same cash
    // account (the financing account the settings', as the package's)
    // and through as_of hands over the same file; without through, the
    // payment after as_of counts, and the files differ.
    let mut tool_csv = |args: Value| {
        let out = m.ok("books.vendor_1099", args);
        m.handed(&out["csv"])
    };
    let same =
        tool_csv(json!({"year": 2025, "csv": true, "cash_accounts": ["1000"], "through": END}));
    assert_eq!(same, files["1099-worksheet-2025.csv"]);
    let named = tool_csv(json!({
        "year": 2025, "csv": true, "cash_accounts": ["1000"],
        "financing_accounts": ["2100"], "through": END
    }));
    assert_eq!(
        named, files["1099-worksheet-2025.csv"],
        "the settings' financing, named"
    );
    let whole_year = tool_csv(json!({"year": 2025, "csv": true, "cash_accounts": ["1000"]}));
    assert_ne!(whole_year, files["1099-worksheet-2025.csv"]);
    // A through outside the year, or an account the chart lacks: refused.
    let why = m.refuse(
        "books.vendor_1099",
        json!({"year": 2025, "through": "2026-01-01"}),
    );
    assert!(why.contains("is a day of 2025"), "{why}");
    let why = m.refuse(
        "books.vendor_1099",
        json!({"year": 2025, "cash_accounts": ["1999"]}),
    );
    assert!(why.contains("no account 1999"), "{why}");

    // Without receipts: no evidence at all.
    let out = m.ok(
        "books.export_cpa_package",
        json!({"as_of": END, "cash_accounts": ["1000"], "include_receipts": false, "include_tins": false}),
    );
    let files = unzip(&m.handed(&out["file"]));
    assert!(
        !files
            .keys()
            .any(|n| n.starts_with("receipts/") || n.starts_with("documents/")),
        "{:?}",
        files.keys()
    );
    assert!(files.contains_key("1099-worksheet-2025.csv"));
}
