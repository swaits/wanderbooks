//! What a wanderbooks older than main wrote and main still opened: a child
//! account of another type, partially overlapping periods, a draft line on
//! an account that doesn't exist, a foreign line booked before rounding to
//! the cent. Written with MAIN's JournalWriter, as
//! books_written_under_older_rules_still_open does inline.
use ledger_core::*;
use ledger_store::{JournalWriter, Store};
use serde_json::json;
fn main() {
    let dir = std::path::PathBuf::from(std::env::args().nth(1).unwrap()).join("x_older_rules");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut store = Store::create(&dir, Currency::USD).unwrap();
    let cash = store.add_account("1000", "Cash", AccountType::Asset, None, true).unwrap();
    drop(store);
    let mut w = JournalWriter::open(&dir.join(ledger_store::JOURNAL_FILE)).unwrap();
    w.append("account.added", json!({"id": 2, "code": "1010", "name": "Rent", "account_type": "Expense", "parent": cash, "placeholder": false, "currency": "USD"})).unwrap();
    w.append("period.created", json!({"id": 1, "name": "2026-01", "start": "2026-01-01", "end": "2026-01-31"})).unwrap();
    w.append("period.created", json!({"id": 2, "name": "straddle", "start": "2026-01-15", "end": "2026-02-15"})).unwrap();
    w.append("entry.created", json!({"id": 1, "date": "2026-03-01", "description": "draft"})).unwrap();
    w.append("entry.line_added", json!({"id": 1, "account": 999, "side": "debit", "amount": "10.00", "currency": "USD"})).unwrap();
    w.append("entry.line_added", json!({"id": 1, "account": cash, "side": "credit", "amount": "108.34567", "currency": "USD", "foreign_currency": "EUR", "foreign_amount": "100.00", "fx_rate": "1.0834567"})).unwrap();
    drop(w);
    println!("WROTE x_older_rules");
}
