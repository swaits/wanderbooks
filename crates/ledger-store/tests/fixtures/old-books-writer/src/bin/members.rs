//! x_member_closes: members closes under each rounding rule main wrote
//! (P3 review R-2), through MAIN's Store API (main 00226a4c, 2026-10-20).
//! Two members, A and B, 50/50; A has a distributions account. Eleven
//! months, each closed in order:
//!   Jan-Mar: untagged (the last member absorbs): +0.01, -0.03, +0.05;
//!            A's 1.00 distribution in February is swept into A's capital.
//!   Apr-Jun: "largest_remainder" (each close on its own): +0.07, -0.01, +0.03.
//!   Jul-Nov: "cumulative" (the run rule), broken by a close to retained
//!            earnings in September: +0.01, +0.01, (RE +0.01), +0.03, -0.05.
//! Hand-computed: A's earnings 0.07, B's 0.05 (of 0.12 closed to them;
//! exact 0.06 each); retained earnings 0.01.
use chrono::NaiveDate;
use ledger_core::*;
use ledger_store::Store;
use rust_decimal::Decimal;

fn d(m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, m, day).unwrap()
}
fn usd(s: &str) -> Money {
    Money::new(s.parse::<Decimal>().unwrap(), Currency::USD)
}

fn main() {
    let dir = std::path::PathBuf::from(std::env::args().nth(1).unwrap()).join("x_member_closes");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Store::create(&dir, Currency::USD).unwrap();
    let cash = s.add_account("1000", "Cash", AccountType::Asset, None, false).unwrap();
    let cap_a = s.add_account("3000", "Capital A", AccountType::Equity, None, false).unwrap();
    let cap_b = s.add_account("3001", "Capital B", AccountType::Equity, None, false).unwrap();
    let dist_a = s.add_account("3100", "Distributions A", AccountType::Equity, None, false).unwrap();
    let rev = s.add_account("4000", "Sales", AccountType::Revenue, None, false).unwrap();
    let exp = s.add_account("6000", "Fees", AccountType::Expense, None, false).unwrap();
    let post = |s: &mut Store, date, dr, cr, amount: &str| {
        let e = s.create_entry(date, "x").unwrap();
        s.add_line(e, dr, Side::Debit, usd(amount)).unwrap();
        s.add_line(e, cr, Side::Credit, usd(amount)).unwrap();
        s.post_entry(e).unwrap();
    };
    post(&mut s, d(1, 2), cash, cap_a, "100.00");
    post(&mut s, d(1, 2), cash, cap_b, "100.00");
    post(&mut s, d(2, 20), dist_a, cash, "1.00");
    let members = vec![
        MemberClose { capital: cap_a, share: "0.5".parse().unwrap(), distributions: Some(dist_a) },
        MemberClose { capital: cap_b, share: "0.5".parse().unwrap(), distributions: None },
    ];
    let months: [(u32, &str, &str); 11] = [
        (1, "0.01", "last"),
        (2, "-0.03", "last"),
        (3, "0.05", "last"),
        (4, "0.07", "per"),
        (5, "-0.01", "per"),
        (6, "0.03", "per"),
        (7, "0.01", "run"),
        (8, "0.01", "run"),
        (9, "0.01", "re"),
        (10, "0.03", "run"),
        (11, "-0.05", "run"),
    ];
    for (m, net, rule) in months {
        match net.strip_prefix('-') {
            Some(loss) => post(&mut s, d(m, 10), exp, cash, loss),
            None => post(&mut s, d(m, 10), cash, rev, net),
        }
        let end = NaiveDate::from_ymd_opt(2026, m + 1, 1).unwrap().pred_opt().unwrap();
        let p = s.create_period(&format!("2026-{m:02}"), d(m, 1), end).unwrap();
        let to = match rule {
            "last" => CloseTo::MembersLastAbsorbs(members.clone()),
            "per" => CloseTo::MembersPerClose(members.clone()),
            "run" => CloseTo::Members(members.clone()),
            _ => CloseTo::RetainedEarnings { sweep: vec![] },
        };
        s.close_period_to(p, &to).unwrap();
    }
    let bal = |id| s.ledger().account_balance(id).unwrap();
    println!(
        "WROTE x_member_closes: A {} B {} dist {}",
        bal(cap_a),
        bal(cap_b),
        bal(dist_a)
    );
}
