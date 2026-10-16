//! The P0 review's thirteen books (round 3's bounds broke every one) and
//! its control (at_limit), written through the writing version's Store
//! API: what that version accepted is what every later version must open.
use chrono::NaiveDate;
use ledger_core::*;
use ledger_store::Store;
use rust_decimal::Decimal;
use std::path::PathBuf;

fn d(y: i32, m: u32, day: u32) -> NaiveDate { NaiveDate::from_ymd_opt(y, m, day).unwrap() }
fn dec(s: &str) -> Decimal { s.parse().unwrap() }
fn usd(s: &str) -> Money { Money::new(dec(s), Currency::USD) }

struct B { s: Store, cash: AccountId, eq: AccountId, exp: AccountId }
fn books(root: &PathBuf, name: &str) -> B {
    let dir = root.join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Store::create(&dir, Currency::USD).unwrap();
    let cash = s.add_account("1000", "Cash", AccountType::Asset, None, false).unwrap();
    let eq = s.add_account("3000", "Capital", AccountType::Equity, None, false).unwrap();
    let exp = s.add_account("6000", "Rent", AccountType::Expense, None, false).unwrap();
    B { s, cash, eq, exp }
}

fn report<T: std::fmt::Debug, E: std::fmt::Display>(name: &str, r: Result<T, E>) {
    match r { Ok(v) => println!("WROTE {name}: {v:?}"), Err(e) => println!("REFUSED {name}: {e}") }
}

fn main() {
    let root = PathBuf::from(std::env::args().nth(1).unwrap());
    // 1. draft line over the limit
    { let mut b = books(&root, "draft_big_line");
      let e = b.s.create_entry(d(2026,3,1), "big draft").unwrap();
      report("draft_big_line", b.s.add_line(e, b.cash, Side::Debit, usd("2000000000000000.00"))); }
    // 2. entry in year 0
    for (name, date) in [("year0_entry", d(0,6,1)), ("year10000_entry", d(10000,1,1)), ("yearneg_entry", d(-1,1,1))] {
      let mut b = books(&root, name);
      let r = (|| -> Result<u64, ledger_store::StoreError> {
        let e = b.s.create_entry(date, "far")?;
        b.s.add_line(e, b.cash, Side::Debit, usd("10.00"))?;
        b.s.add_line(e, b.eq, Side::Credit, usd("10.00"))?;
        b.s.post_entry(e)?; Ok(e) })();
      report(name, r); }
    // 4. asset late
    for (name, date, life) in [("asset_late", d(9999,1,1), 60u32), ("asset_long_life", d(2026,1,1), 120_000u32)] {
      let mut b = books(&root, name);
      let accum = b.s.add_account("1510","Accum",AccountType::Asset,None,false).unwrap();
      let fa = b.s.add_account("1500","Equip",AccountType::Asset,None,false).unwrap();
      report(name, b.s.register_asset(NewAsset { description: "thing".into(), purchase_date: date, cost: usd("1200.00"), salvage_value: usd("0"), useful_life_months: life, cost_account: fa, accum_depr_account: accum, expense_account: b.exp })); }
    // 6. tax basis big
    { let mut b = books(&root, "tax_basis_big");
      let accum = b.s.add_account("1510","Accum",AccountType::Asset,None,false).unwrap();
      let fa = b.s.add_account("1500","Equip",AccountType::Asset,None,false).unwrap();
      let id = b.s.register_asset(NewAsset { description: "thing".into(), purchase_date: d(2026,1,1), cost: usd("1200.00"), salvage_value: usd("0"), useful_life_months: 36, cost_account: fa, accum_depr_account: accum, expense_account: b.exp }).unwrap();
      report("tax_basis_big", b.s.set_asset_tax_basis(id, TaxBasis { rule: TaxBasisRule::Carryover, contributor_basis: Some(dec("2000000000000000.00")) })); }
    // 7. schedule late
    { let mut b = books(&root, "schedule_late");
      let pre = b.s.add_account("1300","Prepaid",AccountType::Asset,None,false).unwrap();
      report("schedule_late", b.s.register_schedule(NewSchedule { description: "late".into(), kind: ScheduleKind::PrepaidExpense, total: usd("1200.00"), start: d(9999,6,1), months: 12, balance_account: pre, pl_account: b.exp })); }
    // 8. recurring big
    { let mut b = books(&root, "recurring_big");
      let big = usd("2000000000000000.00");
      report("recurring_big", b.s.register_recurring(NewRecurring { description: "big".into(), lines: vec![(b.exp, Side::Debit, big), (b.cash, Side::Credit, big)], frequency: Frequency::Monthly, start: d(2026,1,1), end: None })); }
    // 9. shares big
    { let mut b = books(&root, "shares_big");
      let p = b.s.add_party("Acme", &[Role::Member]).unwrap();
      let r = (|| -> Result<(), ledger_store::StoreError> {
        let e = b.s.create_entry(d(2026,3,1), "rent")?;
        b.s.add_line(e, b.exp, Side::Debit, usd("10.00"))?;
        b.s.add_line(e, b.cash, Side::Credit, usd("10.00"))?;
        b.s.post_entry(e)?;
        b.s.set_entry_parties(e, &[PartyShare { party: p, share: Share::Amount(dec("2000000000000000.00")) }]) })();
      report("shares_big", r); }
    // 10. payroll with sub-cent parts that cancel in the totals
    { let mut b = books(&root, "payroll_fine");
      let line = |n: &str, fed: &str| PayrollLine { employee_name: n.into(), employee: None, gross_pay: dec("100.00"), federal_withholding: dec(fed), state_withholding: Decimal::ZERO, ss_employee: Decimal::ZERO, medicare_employee: Decimal::ZERO, ss_employer: Decimal::ZERO, medicare_employer: Decimal::ZERO, other_deductions: Decimal::ZERO };
      report("payroll_fine", b.s.run_payroll(d(2026,3,1), vec![line("A","0.000000000001"), line("B","0.009999999999")], b.cash)); }
    // 11. invoice with sub-cent lines that cancel
    { let mut b = books(&root, "invoice_fine");
      let c = b.s.add_customer("Cust").unwrap();
      report("invoice_fine", b.s.create_invoice(c, d(2026,3,1), d(2026,3,31), vec![DocumentLine{description:"a".into(), amount: dec("0.00000000001")}, DocumentLine{description:"b".into(), amount: dec("9.99999999999")}])); }
    // 12. period year 0
    { let mut b = books(&root, "period_year0");
      report("period_year0", b.s.create_period("y0", d(0,1,1), d(0,12,31))); }
    // 13. exactly at the limit (control: must open)
    { let mut b = books(&root, "at_limit");
      let r = (|| -> Result<u64, ledger_store::StoreError> {
        let e = b.s.create_entry(d(9999,12,31), "max")?;
        b.s.add_line(e, b.cash, Side::Debit, usd("1000000000000000"))?;
        b.s.add_line(e, b.eq, Side::Credit, usd("1000000000000000"))?;
        b.s.post_entry(e)?; Ok(e) })();
      report("at_limit", r); }
}
