//! Writes `p4_s5_before`: books the version before P4b S5 (main eeee9658)
//! wrote through its Store API: a chart, a customer's invoice part paid,
//! and a vendor's bill. No send exists in that version; this version opens
//! the same journal and must report the same figures.
use chrono::NaiveDate;
use ledger_core::{AccountType, Currency, DocRef, DocumentLine, Money, Side};
use ledger_store::Store;
use rust_decimal::Decimal;
use std::path::PathBuf;

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}
fn cents(c: i64) -> Decimal {
    Decimal::new(c, 2)
}
fn line(what: &str, c: i64) -> DocumentLine {
    DocumentLine {
        description: what.into(),
        amount: cents(c),
        class_id: None,
        account: None,
        quantity: None,
        unit_price: None,
    }
}

fn main() {
    let root = PathBuf::from(std::env::args().nth(1).unwrap());
    let dir = root.join("p4_s5_before");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Store::create(&dir, Currency::USD).unwrap();
    let bank = s
        .add_account("1000", "Bank", AccountType::Asset, None, false)
        .unwrap();
    let capital = s
        .add_account("3000", "Capital", AccountType::Equity, None, false)
        .unwrap();
    let seed = s.create_entry(d(2026, 1, 1), "Opening capital").unwrap();
    let m = Money::new(Decimal::from(1000), Currency::USD);
    s.add_line(seed, bank, Side::Debit, m).unwrap();
    s.add_line(seed, capital, Side::Credit, m).unwrap();
    s.post_entry(seed).unwrap();
    let fernhill = s.add_customer("Fernhill Labs").unwrap();
    let vendor = s.add_vendor("Tobin Harlow Supply").unwrap();
    let inv = s
        .create_invoice(
            fernhill,
            d(2026, 2, 1),
            d(2026, 3, 3),
            vec![line("Design", 120_000)],
        )
        .unwrap();
    s.receive_payment(inv, d(2026, 2, 15), cents(40_000), bank)
        .unwrap();
    s.create_bill(
        vendor,
        d(2026, 2, 10),
        d(2026, 3, 12),
        vec![line("Drafting", 30_000)],
    )
    .unwrap();
    println!("WROTE {:?} (invoice {inv}, {:?})", dir, DocRef::Invoice(inv));
}
