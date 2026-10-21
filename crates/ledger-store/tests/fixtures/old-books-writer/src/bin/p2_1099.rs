//! The P2 books of 1099 marks: a vendor marked 1099-NEC, two marked
//! "none" (books.set_vendor_1099 "none" and ""), one undecided whose W-9
//! sync wrote a null, and payments out of the bank (1000), the members'
//! financing (2100) and a card (2050). Every name is fictional.
use chrono::NaiveDate;
use ledger_core::{AccountType, Currency, Form1099, Money, Side};
use ledger_store::Store;
use rust_decimal::Decimal;

fn main() {
    let out = std::path::PathBuf::from(std::env::args().nth(1).unwrap()).join("p2_vendor_1099");
    let _ = std::fs::remove_dir_all(&out);
    let mut s = Store::create(&out, Currency::USD).unwrap();
    let a = |s: &mut Store, c: &str, n: &str, t| s.add_account(c, n, t, None, false).unwrap();
    let bank = a(&mut s, "1000", "Bank", AccountType::Asset);
    let card = a(&mut s, "2050", "Company card", AccountType::Liability);
    let due = a(&mut s, "2100", "Due to Members", AccountType::Liability);
    let labor = a(&mut s, "6000", "Contract Labor", AccountType::Expense);
    let harbor = s.add_vendor("Harbor Design LLC").unwrap();
    let bigco = s.add_vendor("Bigco Supply").unwrap();
    let cedar = s.add_vendor("Cedar Studio").unwrap();
    let dana = s.add_vendor("Dana Studio").unwrap();
    s.set_vendor_1099(harbor, Some(Form1099::Nec), false).unwrap(); // "nec"
    s.set_vendor_1099(bigco, None, false).unwrap(); // "none"
    s.set_vendor_1099(cedar, None, false).unwrap(); // ""
    s.set_vendor_1099(dana, None, true).unwrap(); // set_w9's sync, undecided
    let d = |y, m, day| NaiveDate::from_ymd_opt(y, m, day).unwrap();
    let mut pay = |date, vendor, cr, cents: i64| {
        let e = s.create_entry(date, "work").unwrap();
        let m = Money::new(Decimal::new(cents, 2), Currency::USD);
        s.add_line(e, labor, Side::Debit, m).unwrap();
        s.add_line(e, cr, Side::Credit, m).unwrap();
        s.post_entry(e).unwrap();
        s.set_entry_vendor(e, Some(vendor)).unwrap();
    };
    pay(d(2025, 6, 1), dana, due, 65_000);
    pay(d(2026, 3, 1), harbor, bank, 250_000);
    pay(d(2026, 4, 1), harbor, due, 70_000);
    pay(d(2026, 5, 1), harbor, card, 40_000);
    pay(d(2026, 6, 1), bigco, bank, 90_000);
    println!("WROTE {}", out.display());
}
