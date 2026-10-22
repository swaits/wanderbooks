//! A project's snapshot (rules.md B-37 item 7) in the viewer and from the
//! books server, on the same books: `books.project`'s strings are the
//! figures the viewer's page draws (its `PageFigures` and `Snap`), to the
//! cent, to date, by account, by month with the running net, and for the
//! fiscal year. Both read ledger-core's `project_figures`; this checks
//! that neither side rearranges them.
#![cfg(feature = "gui")]

use chrono::NaiveDate;
use ledger_core::{AccountType, Currency, Money, ProjectFacts, ProjectKind, Side};
use ledger_gui::pages::projects::{Snap, page_figures};
use ledger_gui::{Source, data};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn usd(c: i64) -> Money {
    Money::new(Decimal::new(c, 2), Currency::USD)
}

/// A decimal as the tools print an amount: two places.
fn s(v: Decimal) -> String {
    format!("{v:.2}")
}

/// The books server over stdio (`wanderbooks dev-stdio`), one request at
/// a time.
struct Server {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    stdout: BufReader<std::process::ChildStdout>,
    id: u64,
}

impl Server {
    fn start(data: &std::path::Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_wanderbooks"))
            .arg("dev-stdio")
            .arg("--data-dir")
            .arg(data)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn wanderbooks dev-stdio");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let mut s = Server {
            child,
            stdin,
            stdout,
            id: 0,
        };
        s.ask(
            "initialize",
            json!({"protocolVersion": "2025-06-18", "capabilities": {},
                   "clientInfo": {"name": "projects-parity", "version": "0"}}),
        );
        writeln!(
            s.stdin,
            "{}",
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"})
        )
        .unwrap();
        s
    }

    fn ask(&mut self, method: &str, params: Value) -> Value {
        self.id += 1;
        let msg = json!({"jsonrpc": "2.0", "id": self.id, "method": method, "params": params});
        writeln!(self.stdin, "{msg}").unwrap();
        self.stdin.flush().unwrap();
        let mut line = String::new();
        self.stdout.read_line(&mut line).expect("a response");
        let resp: Value = serde_json::from_str(&line).expect("JSON");
        assert_eq!(resp["id"], self.id, "{resp}");
        resp
    }

    /// A tool's structured result.
    fn tool(&mut self, name: &str, args: Value) -> Value {
        let r = self.ask("tools/call", json!({"name": name, "arguments": args}));
        let out = &r["result"];
        assert_ne!(out["isError"], true, "{name}: {r}");
        out["structuredContent"].clone()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn a_projects_snapshot_is_the_same_in_the_viewer_and_from_the_server() {
    let dir = tempfile::Builder::new()
        .prefix("wb-projects-parity-")
        .tempdir()
        .unwrap();
    let data = dir.path().join("books-data");
    std::fs::write(
        dir.path().join("wanderbooks.toml"),
        "[company]\nname = \"Fernhill Labs\"\nfiscal_year_start = \"01-01\"\n\
         [books]\nreceipt_threshold = \"1000000.00\"\ncash_accounts = [\"1000\"]\n",
    )
    .unwrap();
    let today = d(2026, 10, 15);
    let id = {
        let mut st = ledger_store::Store::create(&data, Currency::USD).unwrap();
        for (code, name, ty) in [
            ("1000", "Cash", AccountType::Asset),
            ("4000", "Design Services", AccountType::Revenue),
            ("6000", "Contractors", AccountType::Expense),
            ("6100", "Software", AccountType::Expense),
            ("6200", "Rent", AccountType::Expense),
        ] {
            st.add_account(code, name, ty, None, false).unwrap();
        }
        let id = st
            .add_project(
                "Kestrel",
                &ProjectFacts {
                    kind: Some(ProjectKind::OnSpec),
                    started_on: Some(d(2025, 11, 1)),
                    ..Default::default()
                },
            )
            .unwrap();
        let code =
            |st: &ledger_store::Store, c: &str| st.ledger().chart().find_by_code(c).unwrap().id;
        let mut posted = Vec::new();
        // (date, debit, credit, cents, tagged): last year's sale, this
        // year's costs (one reversed), a refund, and rent (overhead).
        for (date, dr, cr, c, tagged) in [
            (d(2025, 11, 3), "1000", "4000", 100_000, true),
            (d(2026, 2, 9), "6000", "1000", 40_000, true),
            (d(2026, 3, 9), "6100", "1000", 12_345, true),
            (d(2026, 5, 1), "4000", "1000", 10_000, true),
            (d(2026, 6, 30), "6200", "1000", 150_000, false),
            (d(2026, 8, 4), "6000", "1000", 22_222, true),
        ] {
            let e = st.create_entry(date, "Line").unwrap();
            let (a, b) = (code(&st, dr), code(&st, cr));
            st.add_line(e, a, Side::Debit, usd(c)).unwrap();
            st.add_line(e, b, Side::Credit, usd(c)).unwrap();
            if tagged {
                st.set_line_class(e, if dr == "1000" { 1 } else { 0 }, Some(id))
                    .unwrap();
            }
            st.post_entry(e).unwrap();
            posted.push(e);
        }
        // The software, reversed in September.
        st.reverse_entry(posted[2], d(2026, 9, 2), "Reverse software")
            .unwrap();
        id
    };
    let b = data::load_once(&Source {
        data_dir: data.clone(),
        today: Some(today),
    })
    .unwrap();
    let mut server = Server::start(&data);

    // To date, and by month over the viewer's chart span.
    let page = page_figures(&b, id, Some(d(2025, 11, 1)), false);
    let t = page.to_date.as_ref().unwrap();
    let snap = Snap::to_date(t);
    let ((start, end), months) = page.months.as_ref().unwrap();
    assert_eq!((*start, *end), (d(2025, 11, 1), today));
    let p = server.tool(
        "books.project",
        json!({"project": "Kestrel", "start": start.to_string(), "end": end.to_string()}),
    );
    let to = &p["to_date"];
    assert_eq!(to["revenue"], s(snap.revenue));
    assert_eq!(to["expenses"], s(snap.expenses));
    assert_eq!(to["net"], s(snap.contribution));
    assert_eq!(
        to["margin_pct"],
        snap.margin_pct
            .map_or(Value::Null, |m| json!(m.to_string()))
    );
    assert_eq!(
        to["unrecovered_cost"],
        t.unrecovered_cost.map_or(Value::Null, |u| json!(s(u)))
    );
    // The figures themselves, so a slip on both sides can't pass: $900
    // earned net of the refund, $622.22 of contractors (the software
    // reversed), $277.78, 30.9% of revenue.
    assert_eq!(
        (to["net"].as_str(), to["margin_pct"].as_str()),
        (Some("277.78"), Some("30.9"))
    );
    let accounts: Vec<(String, String, String)> = t
        .revenue_by_account
        .iter()
        .map(|l| (l.code.clone(), "revenue".to_string(), s(l.amount)))
        .chain(
            t.expenses_by_account
                .iter()
                .map(|l| (l.code.clone(), "expense".to_string(), s(l.amount))),
        )
        .collect();
    let served: Vec<(String, String, String)> = to["by_account"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| {
            (
                a["code"].as_str().unwrap().to_string(),
                a["type"].as_str().unwrap().to_string(),
                a["amount"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert_eq!(served, accounts);
    let by_month = p["period"]["by_month"].as_array().unwrap();
    assert_eq!(by_month.len(), months.months.len());
    for ((m, ours), net) in by_month
        .iter()
        .zip(&months.months)
        .zip(&months.months_net_to_date)
    {
        assert_eq!(m["month"], ours.start.format("%Y-%m").to_string());
        assert_eq!(m["revenue"], s(ours.revenue), "{m}");
        assert_eq!(m["expenses"], s(ours.expenses), "{m}");
        assert_eq!(m["net_to_date"], s(*net), "{m}");
    }
    assert_eq!(by_month.last().unwrap()["net_to_date"], to["net"]);

    // This fiscal year, the snapshot's other view: books.project's
    // default period.
    let page = page_figures(&b, id, Some(d(2025, 11, 1)), true);
    let y = page.year.as_ref().unwrap();
    let fy = Snap::period(y);
    let (fs, fe) = b.periods().statement(None, None).unwrap();
    let p = server.tool(
        "books.project",
        json!({"project": "Kestrel", "start": fs.to_string(), "end": fe.to_string()}),
    );
    let period = &p["period"];
    assert_eq!(period["start"], fs.to_string());
    assert_eq!(period["revenue"], s(fy.revenue));
    assert_eq!(period["expenses"], s(fy.expenses));
    assert_eq!(period["net"], s(fy.contribution));
    assert_eq!(
        period["margin_pct"],
        fy.margin_pct.map_or(Value::Null, |m| json!(m.to_string()))
    );
    assert_eq!(period["net"], "-722.22", "a refund and costs, no sale");
}
