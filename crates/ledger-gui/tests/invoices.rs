//! Invoices & Bills in the viewer (plan P4b S2): the list shows each
//! document's figures as the ledger has them (a reversed payment no
//! longer counts), overdue ones say how late, the tiles add up the open
//! ones and tie the list to its account; a document's page shows its
//! lines and its timeline; the overview shows what is owed both ways once
//! the books have documents.

use chrono::NaiveDate;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use ledger_core::arap::{InvoiceFields, SendKind, Sent, SentVia};
use ledger_core::{AccountType, Currency, DocumentLine};
use ledger_gui::nav::Route;
use ledger_gui::{App, Source, data};
use ledger_store::Store;
use rust_decimal::Decimal;
use std::path::PathBuf;

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        let d =
            std::env::temp_dir().join(format!("ledger-gui-invoices-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        TempDir(d)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn line(description: &str, cents: i64) -> DocumentLine {
    DocumentLine {
        description: description.into(),
        amount: Decimal::new(cents, 2),
        class_id: None,
        account: None,
        quantity: None,
        unit_price: None,
    }
}

/// Today is 2026-03-15. Invoice 1 ($500, due Mar 1): $200 paid, then the
/// payment reversed, so it owes $500 and is 14 days late. Invoice 2 ($80,
/// due Apr 30): open. Invoice 3 ($40): paid in full. Bill 1 ($300, due
/// Mar 20): open.
fn books(dir: &TempDir) -> (u64, u64) {
    let mut s = Store::create(&dir.0.join("books-data"), Currency::USD).unwrap();
    for (code, name, t) in [
        ("1000", "Checking", AccountType::Asset),
        ("3000", "Capital", AccountType::Equity),
    ] {
        s.add_account(code, name, t, None, false).unwrap();
    }
    let checking = s.ledger().chart().find_by_code("1000").unwrap().id;
    let client = s.add_customer("Harbor Client").unwrap();
    let late = s
        .create_invoice(
            client,
            d(2026, 2, 1),
            d(2026, 3, 1),
            vec![line("Consulting", 50_000)],
        )
        .unwrap();
    s.create_invoice(
        client,
        d(2026, 2, 5),
        d(2026, 4, 30),
        vec![line("Support", 8_000)],
    )
    .unwrap();
    let paid = s
        .create_invoice(
            client,
            d(2026, 2, 6),
            d(2026, 3, 6),
            vec![line("Setup", 4_000)],
        )
        .unwrap();
    let pay = s
        .receive_payment(late, d(2026, 2, 10), Decimal::new(20_000, 2), checking)
        .unwrap();
    s.reverse_entry(pay, d(2026, 2, 12), "the transfer bounced")
        .unwrap();
    s.receive_payment(paid, d(2026, 2, 20), Decimal::new(4_000, 2), checking)
        .unwrap();
    let vendor = s.add_vendor("Clay Co").unwrap();
    s.create_bill(
        vendor,
        d(2026, 3, 2),
        d(2026, 3, 20),
        vec![line("Glaze", 30_000)],
    )
    .unwrap();
    (late, pay)
}

fn open(dir: &TempDir, route: Route) -> Harness<'static, App> {
    let books = data::load_once(&Source {
        data_dir: dir.0.join("books-data"),
        today: Some(d(2026, 3, 15)),
    })
    .unwrap();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1200.0, 900.0))
        .build_eframe(|cc| App::with_books(&cc.egui_ctx, books, false, route));
    h.run_steps(4);
    h
}

/// SHOT_DIR set: the page as drawn, to look at.
fn shot(h: &mut Harness<'static, App>, name: &str) {
    if let Ok(out) = std::env::var("SHOT_DIR") {
        let out = PathBuf::from(out);
        std::fs::create_dir_all(&out).unwrap();
        h.render()
            .unwrap()
            .save(out.join(format!("{name}.png")))
            .unwrap();
    }
}

#[test]
fn the_list_shows_the_ledgers_figures() {
    let dir = TempDir::new("list");
    books(&dir);
    let mut h = open(&dir, Route::Receivables);
    shot(&mut h, "receivables");
    // Open: invoices 1 and 2 (3 is paid): $580 owed, $500 of it late.
    assert!(h.query_by_label("$580.00").is_some());
    assert!(h.query_by_label("$500.00").is_some());
    assert!(
        h.query_by_label("Invoice 1, Harbor Client, 14 days late, owes 500.00")
            .is_some()
    );
    assert!(
        h.query_by_label_contains("Invoice 2, Harbor Client, Open")
            .is_some()
    );
    assert!(h.query_by_label_contains("Invoice 3,").is_none());
    assert!(h.query_by_label_contains("with 1200").is_some());
    // All shows the paid one too.
    h.get_by_label("All").click();
    h.run_steps(2);
    assert!(
        h.query_by_label_contains("Invoice 3, Harbor Client, Paid")
            .is_some()
    );
    let mut h = open(&dir, Route::Payables);
    shot(&mut h, "payables");
    assert!(
        h.query_by_label_contains("Bill 1, Clay Co, Open, owes 300.00")
            .is_some()
    );
}

#[test]
fn a_documents_page_shows_its_timeline() {
    let dir = TempDir::new("doc");
    let (late, pay) = books(&dir);
    let mut h = open(&dir, Route::Invoice(late));
    shot(&mut h, "invoice");
    assert!(
        h.query_by_label_contains("Payment (reversed later), entry #")
            .is_some()
    );
    assert!(
        h.query_by_label_contains("Payment reversed, entry #")
            .is_some()
    );
    // Received: nothing stands.
    assert!(h.query_by_label("$0.00").is_some());
    // An invoice can be saved as PDF (a bill can't: it is the vendor's).
    assert!(h.query_by_label_contains("Save invoice PDF").is_some());
    // A timeline row opens its entry.
    h.get_by_label_contains(&format!("Payment (reversed later), entry #{pay},"))
        .click();
    h.run_steps(2);
    assert_eq!(h.state_mut().route(), &Route::Entry(pay));
}

#[test]
fn the_overview_shows_what_is_owed_both_ways() {
    let dir = TempDir::new("overview");
    books(&dir);
    let mut h = open(&dir, Route::Overview);
    shot(&mut h, "overview");
    assert!(h.query_by_label("Owed to you").is_some());
    assert!(h.query_by_label("You owe on bills").is_some());
    assert!(h.query_by_label("1 invoice overdue").is_some());
}

#[test]
fn books_without_documents_show_no_overview_tiles_for_them() {
    let dir = TempDir::new("none");
    Store::create(&dir.0.join("books-data"), Currency::USD).unwrap();
    let h = open(&dir, Route::Overview);
    assert!(h.query_by_label("Owed to you").is_none());
    let mut h = open(&dir, Route::Receivables);
    shot(&mut h, "receivables-empty");
    assert!(h.query_by_label("No invoices").is_some());
}

/// Books with one issued invoice (a number, so the sent-tracking items
/// apply) and a checking account, for the attention items (plan P4b S5).
fn issued_invoice(dir: &TempDir, due: NaiveDate) -> u64 {
    let mut s = Store::create(&dir.0.join("books-data"), Currency::USD).unwrap();
    s.add_account("1000", "Checking", AccountType::Asset, None, false)
        .unwrap();
    let client = s.add_customer("Harbor Client").unwrap();
    s.create_invoice_with(
        client,
        d(2026, 2, 1),
        due,
        vec![line("Consulting", 50_000)],
        InvoiceFields {
            number: Some("INV-0001".into()),
            ..Default::default()
        },
    )
    .unwrap()
}

/// A send as these tests record one.
fn sent(on: NaiveDate) -> Sent {
    Sent {
        on,
        by: "agent".into(),
        via: SentVia::Email,
        to: vec!["billing@harbor.example".into()],
        kind: SendKind::Original,
        reference: None,
        note: None,
    }
}

fn attention_titles(dir: &TempDir) -> Vec<String> {
    data::load_once(&Source {
        data_dir: dir.0.join("books-data"),
        today: Some(d(2026, 3, 15)),
    })
    .unwrap()
    .attention
    .into_iter()
    .map(|a| a.title)
    .collect()
}

/// Plan P4b S5, step 4 (R-F): an issued invoice nobody marked sent, past
/// `invoicing.unsent_days` (2 by default), is a to-do that opens the
/// invoice; recording a send clears it. The overview draws it (the
/// screenshot is the R-F pattern's).
#[test]
fn an_unsent_invoice_is_listed_until_a_send_clears_it() {
    let dir = TempDir::new("attention-unsent");
    let id = issued_invoice(&dir, d(2026, 4, 30));
    let titles = attention_titles(&dir);
    assert!(
        titles
            .iter()
            .any(|t| t == "Invoice INV-0001 was never sent"),
        "{titles:?}"
    );
    let mut h = open(&dir, Route::Overview);
    assert!(
        h.query_by_label_contains("Invoice INV-0001 was never sent")
            .is_some()
    );
    shot(&mut h, "attention-unsent-invoice");
    let mut s = Store::open(&dir.0.join("books-data")).unwrap();
    s.mark_sent(id, sent(d(2026, 2, 10))).unwrap();
    drop(s);
    let titles = attention_titles(&dir);
    assert!(
        !titles.iter().any(|t| t.contains("never sent")),
        "a send clears the item: {titles:?}"
    );
}

/// The legacy case: an invoice with no number (issued before
/// `documents_v2`) is exempt from "never sent", however old.
#[test]
fn a_legacy_invoice_with_no_number_is_never_listed_as_unsent() {
    let dir = TempDir::new("attention-legacy");
    let mut s = Store::create(&dir.0.join("books-data"), Currency::USD).unwrap();
    s.add_account("1000", "Checking", AccountType::Asset, None, false)
        .unwrap();
    let client = s.add_customer("Harbor Client").unwrap();
    s.create_invoice(
        client,
        d(2026, 1, 5),
        d(2026, 4, 30),
        vec![line("Consulting", 50_000)],
    )
    .unwrap();
    drop(s);
    let titles = attention_titles(&dir);
    assert!(
        !titles.iter().any(|t| t.contains("never sent")),
        "{titles:?}"
    );
}

/// An overdue invoice whose last send is older than
/// `invoicing.reminder_days` (14 by default) asks for a reminder, on the
/// invoice's page.
#[test]
fn an_overdue_invoice_with_an_old_send_is_asked_to_remind() {
    let dir = TempDir::new("attention-remind");
    let id = issued_invoice(&dir, d(2026, 3, 1));
    let mut s = Store::open(&dir.0.join("books-data")).unwrap();
    s.mark_sent(id, sent(d(2026, 2, 20))).unwrap();
    drop(s);
    let books = data::load_once(&Source {
        data_dir: dir.0.join("books-data"),
        today: Some(d(2026, 3, 15)),
    })
    .unwrap();
    let item = books
        .attention
        .iter()
        .find(|a| a.title.contains("remind the customer"))
        .unwrap_or_else(|| panic!("no reminder item: {:?}", attention_titles(&dir)));
    assert_eq!(item.target, Some(Route::Invoice(id)));
    assert!(item.detail.contains("14 days overdue"), "{}", item.detail);
    assert!(item.prompt.contains("books.mark_sent"), "{}", item.prompt);
}
