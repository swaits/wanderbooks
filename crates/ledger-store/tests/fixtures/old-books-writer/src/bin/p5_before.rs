//! Writes `p5_before`: books the version before P5 (main b97234ac) wrote
//! through its Store API, with the records P5's source corrections act on:
//! a convention revised once, two decisions (one superseding the other)
//! and two open items (one resolved), each with its source, beside a few
//! posted entries and a draft awaiting its evidence. No source correction
//! exists in that version; this version opens the same journal and must
//! report the same figures.
use chrono::NaiveDate;
use ledger_core::conventions::{ConventionKind, NewConvention, NewDecision, NewOpenItem};
use ledger_core::settings::Source;
use ledger_core::{AccountType, Currency, Money, Side};
use ledger_store::Store;
use rust_decimal::Decimal;
use std::path::PathBuf;

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}
fn usd(c: i64) -> Money {
    Money::new(Decimal::new(c, 2), Currency::USD)
}
fn source(by: &str, day: u32) -> Source {
    Source {
        decided_by: by.into(),
        decided_on: d(2026, 10, day),
        citation: None,
        note: None,
    }
}

fn main() {
    let root = PathBuf::from(std::env::args().nth(1).unwrap());
    let dir = root.join("p5_before");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Store::create(&dir, Currency::USD).unwrap();
    let bank = s.add_account("1000", "Bank", AccountType::Asset, None, false).unwrap();
    let capital = s.add_account("3000", "Capital", AccountType::Equity, None, false).unwrap();
    let rent = s.add_account("6000", "Rent", AccountType::Expense, None, false).unwrap();
    let post = |s: &mut Store, date: NaiveDate, what: &str, dr: u64, cr: u64, c: i64| {
        let e = s.create_entry(date, what).unwrap();
        s.add_line(e, dr, Side::Debit, usd(c)).unwrap();
        s.add_line(e, cr, Side::Credit, usd(c)).unwrap();
        e
    };
    let seed = post(&mut s, d(2026, 1, 1), "Opening capital", bank, capital, 100_000);
    s.post_entry(seed).unwrap();
    let jan = post(&mut s, d(2026, 1, 5), "January rent", rent, bank, 50_000);
    s.post_entry(jan).unwrap();
    let _waiting = post(&mut s, d(2026, 2, 3), "Awaiting a receipt", rent, bank, 4_000);
    // How the company keeps its books: a close step, revised once.
    let step = |body: &str, by: &str, day: u32| NewConvention {
        id: "close-checklist".into(),
        kind: ConventionKind::CloseStep,
        title: "Close checklist".into(),
        body_md: body.into(),
        structured: None,
        source: source(by, day),
    };
    s.set_convention(&step("Reconcile, then close.", "Owner", 20)).unwrap();
    s.set_convention(&step("Reconcile, review drafts, then close.", "Owner", 22)).unwrap();
    // Two decisions, the second replacing the first.
    let first = s
        .record_decision(&NewDecision {
            subject: "Materiality".into(),
            decision_md: "Five hundred dollars.".into(),
            decided_by: vec!["Owner".into()],
            decided_on: d(2026, 10, 20),
            evidence_attachment: None,
            supersedes: None,
            citation: None,
            projects: vec![],
        })
        .unwrap();
    s.record_decision(&NewDecision {
        subject: "Materiality".into(),
        decision_md: "One thousand dollars.".into(),
        decided_by: vec!["Owner".into(), "Pat Smith".into()],
        decided_on: d(2026, 10, 25),
        evidence_attachment: None,
        supersedes: Some(first),
        citation: Some("IRS Pub. 538".into()),
        projects: vec![],
    })
    .unwrap();
    // Two open items, one resolved.
    let ask = s
        .raise_open_item(&NewOpenItem {
            title: "Ask the CPA about the vehicle".into(),
            detail_md: None,
            due: Some(d(2026, 11, 15)),
            source: source("Owner", 20),
        })
        .unwrap();
    s.raise_open_item(&NewOpenItem {
        title: "Mail the annual report".into(),
        detail_md: Some("Due with the state.".into()),
        due: None,
        source: source("Owner", 21),
    })
    .unwrap();
    s.resolve_open_item(ask, "Answered: lease it.", &source("Pat Smith", 26)).unwrap();
    s.verify().unwrap();
    println!("WROTE {:?}", dir);
}
