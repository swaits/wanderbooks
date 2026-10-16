//! Two more main-era books past bounds P0 added to commands: depreciation
//! posted for a month past 9999, and one run of recurring entries that
//! created more than MAX_RECURRING_RUN (1000) drafts.
use chrono::NaiveDate;
use ledger_core::*;
use ledger_store::Store;
use rust_decimal::Decimal;
use std::path::PathBuf;
fn d(y: i32, m: u32, day: u32) -> NaiveDate { NaiveDate::from_ymd_opt(y, m, day).unwrap() }
fn usd(s: &str) -> Money { Money::new(s.parse::<Decimal>().unwrap(), Currency::USD) }
fn books(root: &PathBuf, name: &str) -> (Store, AccountId, AccountId) {
    let dir = root.join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Store::create(&dir, Currency::USD).unwrap();
    let cash = s.add_account("1000", "Cash", AccountType::Asset, None, false).unwrap();
    s.add_account("3000", "Capital", AccountType::Equity, None, false).unwrap();
    let exp = s.add_account("6000", "Rent", AccountType::Expense, None, false).unwrap();
    (s, cash, exp)
}
fn main() {
    let root = PathBuf::from(std::env::args().nth(1).unwrap());
    { let (mut s, _cash, exp) = books(&root, "x_depreciation_year10000");
      let accum = s.add_account("1510","Accum",AccountType::Asset,None,false).unwrap();
      let fa = s.add_account("1500","Equip",AccountType::Asset,None,false).unwrap();
      let id = s.register_asset(NewAsset { description: "late".into(), purchase_date: d(9999,1,1), cost: usd("1200.00"), salvage_value: usd("0"), useful_life_months: 60, cost_account: fa, accum_depr_account: accum, expense_account: exp }).unwrap();
      for period in ["9999-12", "10000-01"] {
        match s.run_depreciation(id, period) { Ok(r) => println!("WROTE x_depreciation_year10000 {period}: {}", r.amount), Err(e) => println!("REFUSED {period}: {e}") }
      } }
    { let (mut s, cash, exp) = books(&root, "x_recurring_run_1043");
      let m = usd("5.00");
      s.register_recurring(NewRecurring { description: "weekly".into(), lines: vec![(exp, Side::Debit, m), (cash, Side::Credit, m)], frequency: Frequency::Weekly, start: d(2026,1,1), end: None }).unwrap();
      match s.run_recurring(d(2045,12,31)) { Ok(v) => println!("WROTE x_recurring_run_1043: {} drafts", v.len()), Err(e) => println!("REFUSED: {e}") } }
}
