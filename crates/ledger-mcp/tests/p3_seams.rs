//! P3's seams through the tools: on the seam review's books (a year of a
//! two-member LLC, `ledger-package`'s `tests/seams/books.rs`), built
//! in-process with the real `Store`, closed, then served by the real
//! binary over stdio from the same folder and its settings file, every
//! figure a tool reports is the engine's for the same accounts and dates,
//! and the tools agree with each other: the cash flow statement, party
//! totals on both bases, the 1099 worksheet as of a day, the fixed
//! assets, the members' equity statement and the CPA package.

#[path = "../../ledger-package/tests/seams/books.rs"]
mod books;

use books::{books, d, dec};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(180);
const START: &str = "2026-01-01";

struct Mcp {
    stdin: ChildStdin,
    rx: Receiver<String>,
    child: Child,
    next_id: u64,
}

impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Mcp {
    fn start(dir: &Path) -> Self {
        let run = dir.join("run");
        std::fs::create_dir_all(&run).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_ledger-mcp"))
            .arg("--data-dir")
            .arg(dir.join("books-data"))
            .arg("--books-toml")
            .arg(dir.join("wanderbooks.toml"))
            .env("XDG_RUNTIME_DIR", &run)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
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
        let mut mcp = Mcp {
            stdin,
            rx,
            child,
            next_id: 1,
        };
        let init = mcp.rpc(
            "initialize",
            json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "p3-seams", "version": "0.1.0"},
            }),
        );
        assert!(init.get("result").is_some(), "initialize failed: {init}");
        writeln!(
            mcp.stdin,
            "{}",
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"})
        )
        .unwrap();
        mcp
    }

    fn rpc(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let msg = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        writeln!(self.stdin, "{msg}").unwrap();
        self.stdin.flush().unwrap();
        loop {
            let line = self
                .rx
                .recv_timeout(TIMEOUT)
                .unwrap_or_else(|_| panic!("no response to {method}"));
            let v: Value = serde_json::from_str(&line).unwrap();
            if v.get("id") == Some(&json!(id)) {
                return v;
            }
        }
    }

    /// A tool's structured result; a refusal fails the test.
    fn ok(&mut self, name: &str, args: Value) -> Value {
        let resp = self.rpc("tools/call", json!({"name": name, "arguments": args}));
        let r = &resp["result"];
        assert!(
            resp.get("error").is_none() && r.get("isError") != Some(&json!(true)),
            "{name} {args}: {resp}"
        );
        r["structuredContent"].clone()
    }
}

fn m(v: &Value) -> rust_decimal::Decimal {
    dec(v
        .as_str()
        .unwrap_or_else(|| panic!("a decimal string: {v}")))
}

/// What the engine says, worked out before the server opens the books.
struct Expected {
    end: String,
    cash_flow: ledger_core::CashFlowStatement,
    members_equity: ledger_core::MembersEquityStatement,
    register: ledger_core::AssetRegister,
    vendor_1099: ledger_core::Vendor1099Report,
    depreciation: rust_decimal::Decimal,
    iif: String,
}

/// Every figure the tools report on the seam review's books is the
/// engine's for the same accounts and dates, on every date (B-33's "the
/// tool's for the same accounts and through"), and party totals for the
/// year are the engine's on both bases.
#[test]
fn the_tools_figures_are_the_engines_on_every_date() {
    let dir = tempfile::tempdir().unwrap();
    let b = books(dir.path());
    let l = b.store.ledger();
    let settings = ledger_package::Settings {
        company: &b.company,
        cash_accounts: &b.cash,
        financing_accounts: &b.financing,
        investing_accounts: &[],
    };
    let capitals = ledger_package::member_capitals(l, &b.company.members).unwrap();
    let mut expected = Vec::new();
    for end in ["2026-06-30", "2026-09-15", "2026-12-31"] {
        let e = d(end);
        expected.push(Expected {
            end: end.to_string(),
            cash_flow: ledger_package::cash_flow_for(
                l,
                &settings,
                Some(START),
                Some(end),
                &[],
                &[],
                &[],
                e,
            )
            .unwrap(),
            members_equity: ledger_core::members_equity(l, d(START), e, &capitals).unwrap(),
            register: ledger_core::fixed_asset_register(l, e).unwrap(),
            vendor_1099: ledger_package::vendor_1099_report(l, &settings, 2026, None, Some(e))
                .unwrap(),
            depreciation: ledger_core::profit_and_loss_for_period(l, d(START), e, None)
                .unwrap()
                .expenses
                .iter()
                .filter(|x| x.code == "6500")
                .map(|x| x.amount)
                .sum(),
            iif: ledger_package::export_iif(l, &b.cash, e).unwrap().text,
        });
    }
    let money = ledger_core::parties::money_accounts(
        l,
        &[l.chart().find_by_code("1000").unwrap().id],
        &b.financing
            .iter()
            .map(|c| l.chart().find_by_code(c).unwrap().id)
            .collect::<Vec<_>>(),
    );
    let members = b.company.party_members(l);
    let year = (d(START), d("2026-12-31"));
    let parties: Vec<(String, ledger_core::parties::Activity)> = l
        .parties()
        .map(|p| {
            (
                p.name.clone(),
                ledger_core::parties::activity_with(l, p.id, year, &money, &members),
            )
        })
        .collect();
    drop(b);

    let mut mcp = Mcp::start(dir.path());
    for x in &expected {
        let end = x.end.as_str();
        // The cash flow statement: every line, kind and reconciling item.
        let cf = mcp.ok("books.cash_flow", json!({"start": START, "end": end}));
        for (key, total) in [
            ("cash_from_operations", x.cash_flow.cash_from_operations),
            ("cash_from_investing", x.cash_flow.cash_from_investing),
            ("cash_from_financing", x.cash_flow.cash_from_financing),
            ("net_change_in_cash", x.cash_flow.net_change_in_cash),
        ] {
            assert_eq!(m(&cf[key]), total, "{end} {key}");
        }
        for (key, section) in [
            ("operating", ledger_core::CashFlowSection::Operating),
            ("investing", ledger_core::CashFlowSection::Investing),
            ("financing", ledger_core::CashFlowSection::Financing),
        ] {
            let lines: Vec<(String, rust_decimal::Decimal)> = cf[key]
                .as_array()
                .unwrap()
                .iter()
                .map(|l| (l["kind"].as_str().unwrap().to_string(), m(&l["amount"])))
                .collect();
            let engine: Vec<(String, rust_decimal::Decimal)> = x
                .cash_flow
                .lines(section)
                .iter()
                .map(|l| (l.kind.clone(), l.amount))
                .collect();
            assert_eq!(lines, engine, "{end} {key}");
        }
        let recon: Vec<(String, rust_decimal::Decimal)> = cf["operating_reconciliation"]["lines"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| (l["label"].as_str().unwrap().to_string(), m(&l["amount"])))
            .collect();
        let engine: Vec<(String, rust_decimal::Decimal)> = x
            .cash_flow
            .operating_reconciliation
            .lines
            .iter()
            .map(|l| (l.label.clone(), l.amount))
            .collect();
        assert_eq!(recon, engine, "{end}");
        let depreciation: rust_decimal::Decimal = recon
            .iter()
            .filter(|(label, _)| label == "Depreciation")
            .map(|(_, a)| *a)
            .sum();
        assert_eq!(
            depreciation, x.depreciation,
            "{end}: the P&L's depreciation"
        );
        // The P&L's depreciation.
        let pnl = mcp.ok("books.profit_and_loss", json!({"start": START, "end": end}));
        let dep: rust_decimal::Decimal = pnl["expense_accounts"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|a| a["code"] == "6500")
            .map(|a| m(&a["amount"]))
            .sum();
        assert_eq!(dep, x.depreciation, "{end}");
        // The members' equity statement.
        let me = mcp.ok(
            "books.export",
            json!({"report": "members_equity", "format": "json", "start": START, "end": end}),
        );
        let me: Value = serde_json::from_str(me["content"].as_str().unwrap()).unwrap();
        for (col, engine) in me["members"]
            .as_array()
            .unwrap()
            .iter()
            .zip(&x.members_equity.members)
        {
            for (key, v) in [
                ("beginning", engine.beginning),
                ("contributions", engine.contributions),
                ("net_income", engine.net_income),
                ("distributions", engine.distributions),
                ("other", engine.other),
                ("ending", engine.ending),
            ] {
                assert_eq!(m(&col[key]), v, "{end} {} {key}", engine.name);
            }
        }
        // The fixed assets, as of the day: list_assets is the register.
        let assets = mcp.ok("books.list_assets", json!({"as_of": end}));
        for (a, row) in assets["assets"]
            .as_array()
            .unwrap()
            .iter()
            .zip(&x.register.rows)
        {
            assert_eq!(m(&a["accumulated_posted"]), row.accumulated_posted, "{end}");
            assert_eq!(
                m(&a["accumulated_scheduled_through"]),
                row.accumulated_scheduled,
                "{end}"
            );
            assert_eq!(m(&a["book_value"]), row.book_value, "{end}");
            let disposed = row.disposed.is_some();
            assert_eq!(a["held"], json!(!disposed), "{end}");
            assert_eq!(
                a["status"],
                json!(if disposed { "disposed" } else { "active" }),
                "{end}: a disposal after as_of doesn't show"
            );
        }
        // The 1099 worksheet as of the day.
        let w = mcp.ok("books.vendor_1099", json!({"year": 2026, "through": end}));
        let vendors = w["vendors"].as_array().unwrap();
        assert_eq!(vendors.len(), x.vendor_1099.vendors.len(), "{end}");
        for (v, engine) in vendors.iter().zip(&x.vendor_1099.vendors) {
            assert_eq!(v["vendor_id"], json!(engine.vendor_id));
            for (key, amount) in [
                ("paid", engine.paid),
                ("excluded_card", engine.excluded_card),
                ("undetermined", engine.undetermined),
                ("not_reportable", engine.not_reportable),
            ] {
                assert_eq!(m(&v[key]), amount, "{end} {} {key}", engine.name);
            }
        }
        // The IIF as of the day.
        let iif = mcp.ok(
            "books.export",
            json!({"report": "general_journal", "format": "iif", "as_of": end}),
        );
        assert_eq!(iif["content"].as_str().unwrap(), x.iif, "{end}");
    }
    // Party totals for the year, on both bases.
    for (name, a) in &parties {
        for (basis, key) in [
            (ledger_core::parties::Basis::Cash, "cash"),
            (ledger_core::parties::Basis::All, "all"),
        ] {
            let p = mcp.ok(
                "books.party",
                json!({"party": name, "year": 2026, "basis": key}),
            );
            let (paid, received) = a.totals(basis);
            assert_eq!(m(&p["totals"]["paid"]), paid, "{name} {key}");
            assert_eq!(m(&p["totals"]["received"]), received, "{name} {key}");
        }
    }
}
