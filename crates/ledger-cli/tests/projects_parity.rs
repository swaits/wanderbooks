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

mod common;

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

/// The settings both tests' books keep.
const SETTINGS: &str = "[company]\nname = \"Fernhill Labs\"\nfiscal_year_start = \"01-01\"\n\
     [books]\nreceipt_threshold = \"1000000.00\"\ncash_accounts = [\"1000\"]\n";

/// A session with the books server (common.rs), one request at a time.
struct Server {
    wire: common::Wire,
}

impl Server {
    /// Start `place`'s server on the books the test wrote in its root,
    /// with [`SETTINGS`], and a session with them.
    fn start(place: &common::Place) -> Self {
        let mut wire = place.start(Some(SETTINGS));
        wire.initialize("projects-parity");
        Server { wire }
    }

    /// A tool's structured result.
    fn tool(&mut self, name: &str, args: Value) -> Value {
        let r = self.wire.call(name, args);
        let out = &r["result"];
        assert_ne!(out["isError"], true, "{name}: {r}");
        out["structuredContent"].clone()
    }
}

#[test]
fn a_projects_snapshot_is_the_same_in_the_viewer_and_from_the_server() {
    // The books written in a data root by the store itself, then served
    // there.
    let place = common::Place::new("projects-parity");
    let data = place.books_data();
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
    let mut server = Server::start(&place);
    let b = data::load_once(&Source {
        data_dir: data.clone(),
        today: Some(today),
    })
    .unwrap();

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

/// rules.md B-37 item 8: a project's invoiced revenue and billed costs,
/// made through the books server's own tools, read the same in the viewer
/// (its snapshot, and the invoice's and bill's pages: each line's
/// project) and from the server (books.project, books.get_entry).
#[test]
fn invoiced_and_billed_project_figures_are_the_same_in_the_viewer_and_from_the_server() {
    let place = common::Place::new("projects-parity-docs");
    let data = place.books_data();
    let today = d(2026, 10, 15);
    {
        let mut st = ledger_store::Store::create(&data, Currency::USD).unwrap();
        st.add_account("1000", "Cash", AccountType::Asset, None, false)
            .unwrap();
    }
    let mut server = Server::start(&place);
    server.tool(
        "books.add_project",
        json!({"name": "Kestrel", "kind": "client", "started_on": "2026-01-05"}),
    );
    server.tool("books.add_project", json!({"name": "Heron"}));
    let cedar = server.tool("books.add_customer", json!({"name": "Cedar Client"}))["id"].clone();
    let dana = server.tool("books.add_vendor", json!({"name": "Dana Illustration"}))["id"].clone();
    let doc = |key: &str, party: &Value, date: &str, lines: Value| {
        json!({key: party.to_string(), "date": date, "due_date": date, "lines": lines,
               "confirmed": true})
    };
    server.tool(
        "books.add_account",
        json!({"code": "6500", "name": "Supplies", "account_type": "expense"}),
    );
    let inv = server.tool(
        "books.create_invoice",
        doc(
            "customer",
            &cedar,
            "2026-02-02",
            json!([
                {"description": "Discovery", "amount": "1500.00", "class": "Kestrel"},
                {"description": "Heron sketches", "amount": "250.00", "class": "Heron"},
                {"description": "Travel", "amount": "80.00"},
            ]),
        ),
    );
    let inv2 = server.tool(
        "books.create_invoice",
        doc(
            "customer",
            &cedar,
            "2026-05-04",
            json!([{"description": "Build", "amount": "4321.09", "class": "kestrel"}]),
        ),
    );
    let bill_file = server
        .wire
        .upload(b"Dana's invoice", "dana.pdf", "application/pdf", "receipt");
    let mut bill_args = doc(
        "vendor",
        &dana,
        "2026-03-09",
        json!([
            {"description": "Illustration", "amount": "987.65", "account": "6500", "class": "Kestrel"},
            {"description": "Postage", "amount": "12.00", "account": "6500"},
        ]),
    );
    bill_args["vendor_ref"] = json!("DANA-9");
    bill_args["upload_id"] = json!(bill_file);
    let bill = server.tool("books.create_bill", bill_args);
    server.tool(
        "books.receive_payment",
        json!({"confirmed": true, "invoice": inv["id"], "date": "2026-02-20", "amount": "1830.00", "deposit_to": "1000"}),
    );
    let b = data::load_once(&Source {
        data_dir: data.clone(),
        today: Some(today),
    })
    .unwrap();
    let id = b.ledger.find_class_by_name("Kestrel").unwrap().id;
    let page = page_figures(&b, id, Some(d(2026, 1, 5)), false);
    let t = page.to_date.as_ref().unwrap();
    let snap = Snap::to_date(t);
    let p = server.tool(
        "books.project",
        json!({"project": "Kestrel", "start": "2026-01-01", "end": today.to_string()}),
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
    // By hand: 1,500.00 + 4,321.09 invoiced, 987.65 billed.
    assert_eq!(
        (
            to["revenue"].as_str(),
            to["expenses"].as_str(),
            to["net"].as_str()
        ),
        (Some("5821.09"), Some("987.65"), Some("4833.44"))
    );
    let ((_, _), months) = page.months.as_ref().unwrap();
    let by_month = p["period"]["by_month"].as_array().unwrap();
    assert!(!by_month.is_empty(), "{p}");
    assert_eq!(by_month.len(), months.months.len());
    for (m, ours) in by_month.iter().zip(&months.months) {
        assert_eq!(m["month"], ours.start.format("%Y-%m").to_string());
        assert_eq!(m["revenue"], s(ours.revenue), "{m}");
        assert_eq!(m["expenses"], s(ours.expenses), "{m}");
    }
    // Each document's lines: the viewer's page names the project the
    // server's entry line carries.
    for out in [&inv, &inv2, &bill] {
        let entry = out["entry_id"].as_u64().unwrap();
        let doc = ledger_gui::pages::entry::document_of(&b, entry).unwrap();
        let viewer: Vec<Option<String>> = {
            let mut seen: Vec<Option<String>> = Vec::new();
            for l in doc.lines {
                let name = l
                    .class_id
                    .and_then(|c| b.ledger.get_class(c))
                    .map(|c| c.name.clone());
                if !seen.contains(&name) {
                    seen.push(name);
                }
            }
            seen
        };
        let served = server.tool("books.get_entry", json!({"entry_id": entry}));
        let served: Vec<Option<String>> = served["lines"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|l| matches!(l["code"].as_str(), Some("4000" | "6500" | "6999")))
            .map(|l| l["project"].as_str().map(str::to_string))
            .collect();
        assert_eq!(viewer, served, "entry {entry}");
    }
}
