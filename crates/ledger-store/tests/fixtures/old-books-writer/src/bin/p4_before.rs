//! Writes `p4_before`: books the version before P4 (main 25b52580) wrote
//! through its Store API: hand entries (one posted late with an early
//! date), a reversal, an invoice and its payment, a schedule month, a
//! recurring entry with a generated draft, two open drafts (one with
//! lines, one empty), and a closed January. No draft state, discard or
//! posting sequence exists in that version; this version opens the same
//! journal and must report the same figures.
use chrono::NaiveDate;
use ledger_core::schedules::{Frequency, NewRecurring, NewSchedule, ScheduleConvention, ScheduleKind};
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
    let dir = root.join("p4_before");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Store::create(&dir, Currency::USD).unwrap();
    let bank = s.add_account("1000", "Bank", AccountType::Asset, None, false).unwrap();
    let prepaid = s.add_account("1300", "Prepaid expenses", AccountType::Asset, None, false).unwrap();
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
    // Drafted first, posted last, dated first: posting order is not date order.
    let late = post(&mut s, d(2026, 1, 3), "Domain renewal", software, bank, 1_800);
    let rent_jan = post(&mut s, d(2026, 1, 5), "January rent", rent, bank, 50_000);
    s.post_entry(rent_jan).unwrap();
    s.post_entry(late).unwrap();
    let wrong = post(&mut s, d(2026, 1, 12), "Rent paid twice", rent, bank, 50_000);
    s.post_entry(wrong).unwrap();
    s.reverse_entry(wrong, d(2026, 1, 14), "Entered twice").unwrap();
    // A document and its payment.
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
    // A schedule month.
    let sched = s
        .register_schedule(NewSchedule {
            description: "Annual insurance".into(),
            kind: ScheduleKind::PrepaidExpense,
            total: usd(120_000),
            start: d(2026, 1, 1),
            months: 12,
            balance_account: prepaid,
            pl_account: rent,
            convention: ScheduleConvention::MonthlyEven,
        })
        .unwrap();
    let paid = post(&mut s, d(2026, 1, 2), "Insurance premium", prepaid, bank, 120_000);
    s.post_entry(paid).unwrap();
    s.post_schedule_month(sched, d(2026, 1, 31)).unwrap();
    // A recurring entry with one generated draft (February).
    s.register_recurring(NewRecurring {
        description: "Figma".into(),
        lines: vec![(software, Side::Debit, usd(1_500)), (bank, Side::Credit, usd(1_500))],
        frequency: Frequency::Monthly,
        start: d(2026, 2, 10),
        end: None,
    })
    .unwrap();
    s.run_recurring(d(2026, 2, 28)).unwrap();
    // Two open drafts dated in February; January closes around them.
    let open = post(&mut s, d(2026, 2, 12), "Coworking day pass", rent, bank, 3_500);
    let _ = open;
    let _empty = s.create_entry(d(2026, 2, 14), "Placeholder").unwrap();
    let jan = s.create_period("2026-01", d(2026, 1, 1), d(2026, 1, 31)).unwrap();
    s.close_period(jan).unwrap();
    let _ = sales;
    s.verify().unwrap();
    println!("WROTE {:?}", dir);
}
