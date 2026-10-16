//! More legal-but-extreme books, written through MAIN's Store API: the
//! edges of chrono's calendar and of Decimal, which an earlier wanderbooks
//! accepted. Each must open under every later version, and nothing
//! computed over them may panic.
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
fn post(b: &mut B, date: NaiveDate, dr: AccountId, cr: AccountId, amt: &str) -> Result<u64, ledger_store::StoreError> {
    let e = b.s.create_entry(date, "edge")?;
    b.s.add_line(e, dr, Side::Debit, usd(amt))?;
    b.s.add_line(e, cr, Side::Credit, usd(amt))?;
    b.s.post_entry(e)?;
    Ok(e)
}

const MAX: &str = "79228162514264337593543950335";
const HALF: &str = "39614081257132168796771975167";
const MAX_CENTS: &str = "792281625142643375935439503.35";

fn main() {
    let root = PathBuf::from(std::env::args().nth(1).unwrap());
    // Posted entries on chrono's first and last days, and a draft on each.
    for (name, date) in [("x_date_max_entry", NaiveDate::MAX), ("x_date_min_entry", NaiveDate::MIN)] {
        let mut b = books(&root, name);
        let (cash, eq) = (b.cash, b.eq);
        report(name, post(&mut b, date, cash, eq, "10.00"));
        report(name, b.s.create_entry(date, "draft at the edge"));
    }
    // A draft whose lines are Decimal's largest: its debits can't be summed.
    { let mut b = books(&root, "x_draft_max_lines");
      let e = b.s.create_entry(d(2026, 3, 1), "huge draft").unwrap();
      report("x_draft_max_lines", b.s.add_line(e, b.cash, Side::Debit, usd(MAX)));
      report("x_draft_max_lines", b.s.add_line(e, b.exp, Side::Debit, usd(MAX)));
      report("x_draft_max_lines", b.s.add_line(e, b.eq, Side::Credit, usd("1.00"))); }
    // A schedule whose months run past chrono's last day.
    { let mut b = books(&root, "x_schedule_chrono_end");
      let pre = b.s.add_account("1300","Prepaid",AccountType::Asset,None,false).unwrap();
      report("x_schedule_chrono_end", b.s.register_schedule(NewSchedule { description: "edge".into(), kind: ScheduleKind::PrepaidExpense, total: usd("1200.00"), start: d(262142,6,1), months: 12, balance_account: pre, pl_account: b.exp })); }
    // An asset whose life ends in chrono's last month.
    { let mut b = books(&root, "x_asset_chrono_end");
      let accum = b.s.add_account("1510","Accum",AccountType::Asset,None,false).unwrap();
      let fa = b.s.add_account("1500","Equip",AccountType::Asset,None,false).unwrap();
      report("x_asset_chrono_end", b.s.register_asset(NewAsset { description: "edge".into(), purchase_date: d(262142,1,15), cost: usd("1200.00"), salvage_value: usd("0"), useful_life_months: 11, cost_account: fa, accum_depr_account: accum, expense_account: b.exp })); }
    // Two members' shares of one entry, each Decimal's largest (a whole number of cents).
    { let mut b = books(&root, "x_shares_max");
      let p = b.s.add_party("Member A", &[Role::Member]).unwrap();
      let q = b.s.add_party("Member B", &[Role::Member]).unwrap();
      let (exp, cash) = (b.exp, b.cash);
      let r = (|| -> Result<(), ledger_store::StoreError> {
        let e = post(&mut b, d(2026,3,1), exp, cash, "10.00")?;
        b.s.set_entry_parties(e, &[PartyShare { party: p, share: Share::Amount(dec(MAX)) }, PartyShare { party: q, share: Share::Amount(dec(MAX)) }]) })();
      report("x_shares_max", r); }
    // Two assets whose contributor bases are Decimal's largest.
    { let mut b = books(&root, "x_tax_basis_max");
      let accum = b.s.add_account("1510","Accum",AccountType::Asset,None,false).unwrap();
      let fa = b.s.add_account("1500","Equip",AccountType::Asset,None,false).unwrap();
      for n in 0..2 {
        let id = b.s.register_asset(NewAsset { description: format!("thing {n}"), purchase_date: d(2026,1,1), cost: usd("1200.00"), salvage_value: usd("0"), useful_life_months: 36, cost_account: fa, accum_depr_account: accum, expense_account: b.exp }).unwrap();
        report("x_tax_basis_max", b.s.set_asset_tax_basis(id, TaxBasis { rule: TaxBasisRule::Carryover, contributor_basis: Some(dec(MAX)) }));
      } }
    // An exchange rate of Decimal's largest.
    { let mut b = books(&root, "x_fx_rate_max");
      report("x_fx_rate_max", b.s.set_fx_rate(Currency::new("EUR").unwrap(), d(2026,1,1), dec(MAX))); }
    // A statement whose balances are near Decimal's largest in cents.
    { let mut b = books(&root, "x_statement_max");
      let (cash, eq) = (b.cash, b.eq);
      report("x_statement_max", post(&mut b, d(2026,3,1), cash, eq, "10.00"));
      let (att, _, _) = b.s.file_document("statement", "s.pdf", "application/pdf", b"%PDF-1.4 statement", None).unwrap();
      report("x_statement_max", b.s.record_statement(ledger_store::Statement { attachment_id: att, account_id: cash, period_start: d(2026,3,1), period_end: d(2026,3,31), opening_balance: dec(&format!("-{MAX_CENTS}")), closing_balance: dec(MAX_CENTS) })); }
    // A recurring entry of half Decimal's largest, generated as drafts.
    { let mut b = books(&root, "x_recurring_generated_max");
      let big = usd(HALF);
      report("x_recurring_generated_max", b.s.register_recurring(NewRecurring { description: "huge".into(), lines: vec![(b.exp, Side::Debit, big), (b.cash, Side::Credit, big)], frequency: Frequency::Monthly, start: d(2026,1,1), end: Some(d(2026,3,31)) }));
      report("x_recurring_generated_max", b.s.run_recurring(d(2026,3,31))); }
    // Periods at chrono's first and last years, the last one closed.
    { let mut b = books(&root, "x_periods_chrono_edges");
      report("x_periods_chrono_edges", b.s.create_period("first", NaiveDate::MIN, d(NaiveDate::MIN.format("%Y").to_string().parse().unwrap(), 12, 31)));
      let p = b.s.create_period("last", d(262142,1,1), NaiveDate::MAX);
      report("x_periods_chrono_edges", p.as_ref().map(|_| ()).map_err(|e| e.to_string()));
      let (cash, eq, exp) = (b.cash, b.eq, b.exp);
      report("x_periods_chrono_edges", post(&mut b, d(262142,6,1), cash, eq, "100.00"));
      report("x_periods_chrono_edges", post(&mut b, d(262142,7,1), exp, cash, "40.00"));
      if let Ok(p) = p { report("x_periods_chrono_edges", b.s.close_period(p)); } }
    // A weekly recurring entry from year 1 (a hundred thousand due dates).
    { let mut b = books(&root, "x_recurring_weekly_year1");
      let m = usd("5.00");
      report("x_recurring_weekly_year1", b.s.register_recurring(NewRecurring { description: "weekly".into(), lines: vec![(b.exp, Side::Debit, m), (b.cash, Side::Credit, m)], frequency: Frequency::Weekly, start: d(1,1,1), end: None })); }
}
