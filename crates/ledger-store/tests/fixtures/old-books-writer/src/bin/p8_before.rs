//! Writes `p8_before`: books the version before P8 (main b97234ac) wrote
//! through its Store API: a receipted opening and purchase, a 40.00
//! purchase with no receipt, a small one under the threshold, a reversal,
//! an invoice with a part payment, an open draft, and a closed January.
//! No audit run, attestation or attestation setting exists in that
//! version; this version opens the same journal and must report the same
//! figures, and an audit of it finds exactly the missing receipt and the
//! open draft.
use chrono::NaiveDate;
use ledger_core::{AccountType, Currency, DocumentLine, Money, Side};
use ledger_store::Store;
use rust_decimal::Decimal;
use std::path::PathBuf;

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}
fn cents(c: i64) -> Decimal {
    Decimal::new(c, 2)
}
fn usd(c: i64) -> Money {
    Money::new(cents(c), Currency::USD)
}

fn main() {
    let root = PathBuf::from(std::env::args().nth(1).unwrap());
    let dir = root.join("p8_before");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Store::create(&dir, Currency::USD).unwrap();
    let bank = s.add_account("1000", "Bank", AccountType::Asset, None, false).unwrap();
    let capital = s.add_account("3000", "Capital", AccountType::Equity, None, false).unwrap();
    let sales = s.add_account("4000", "Sales", AccountType::Revenue, None, false).unwrap();
    let rent = s.add_account("6000", "Rent", AccountType::Expense, None, false).unwrap();
    let software = s.add_account("6100", "Software", AccountType::Expense, None, false).unwrap();
    let post = |s: &mut Store, date: NaiveDate, what: &str, dr: u64, cr: u64, c: i64| {
        let e = s.create_entry(date, what).unwrap();
        s.add_line(e, dr, Side::Debit, usd(c)).unwrap();
        s.add_line(e, cr, Side::Credit, usd(c)).unwrap();
        e
    };
    let seed = post(&mut s, d(2026, 1, 1), "Opening capital", bank, capital, 100_000);
    s.post_entry(seed).unwrap();
    s.attach_receipt(Some(seed), "opening.txt", "text/plain", b"Opening deposit 1000.00 2026-01-01", None)
        .unwrap();
    let rent_jan = post(&mut s, d(2026, 1, 5), "January rent", rent, bank, 50_000);
    s.post_entry(rent_jan).unwrap();
    s.attach_receipt(Some(rent_jan), "rent.txt", "text/plain", b"Rent January 500.00 2026-01-05", None)
        .unwrap();
    // 40.00 is over the 25.00 threshold: no receipt, so an audit finds it.
    let paper = post(&mut s, d(2026, 1, 7), "Printer paper", software, bank, 4_000);
    s.post_entry(paper).unwrap();
    // Under the threshold: needs none.
    let stamps = post(&mut s, d(2026, 1, 8), "Stamps", software, bank, 1_200);
    s.post_entry(stamps).unwrap();
    let wrong = post(&mut s, d(2026, 1, 12), "Rent paid twice", rent, bank, 50_000);
    s.post_entry(wrong).unwrap();
    s.attach_receipt(Some(wrong), "rent2.txt", "text/plain", b"Rent again 500.00 2026-01-12", None)
        .unwrap();
    s.reverse_entry(wrong, d(2026, 1, 14), "Entered twice").unwrap();
    let customer = s.add_customer("Fernhill Labs").unwrap();
    let inv = s
        .create_invoice(
            customer,
            d(2026, 1, 10),
            d(2026, 2, 9),
            vec![DocumentLine {
                description: "Design".into(),
                amount: cents(120_000),
                class_id: None,
                account: None,
                quantity: None,
                unit_price: None,
            }],
        )
        .unwrap();
    s.receive_payment(inv, d(2026, 1, 20), cents(40_000), bank).unwrap();
    // An open draft in February; January closes around it.
    let _open = post(&mut s, d(2026, 2, 12), "Coworking day pass", rent, bank, 3_500);
    let jan = s.create_period("2026-01", d(2026, 1, 1), d(2026, 1, 31)).unwrap();
    s.close_period(jan).unwrap();
    let _ = sales;
    s.verify().unwrap();
    println!("WROTE {:?}", dir);
}
