//! Cross-check against GnuCash (rank 3: a differential oracle).
//!
//! GnuCash is *an* independent implementation of double entry, not the
//! spec (docs/oracle.md, "Who is right"). This test builds random books
//! through wanderbooks' engine (entries, foreign-currency lines, reversals,
//! period closes), hands their posted entries to GnuCash 5.10 running in a
//! pinned container (oracle/gnucash/), and compares every account's
//! balance at month ends and random dates, exactly. A difference is a
//! finding to adjudicate from principles (rules.md), never a reason to
//! change wanderbooks to match.
//!
//! Needs Docker: `just oracle` builds the image and runs this.

use chrono::NaiveDate;
use ledger_core::reports::{profit_and_loss_for_period, trial_balance_as_of};
use ledger_core::{AccountId, AccountType, Currency, Ledger, Money, Side};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::Write;
use std::process::{Command, Stdio};

/// Books per run, and the seed of the first.
const BOOKS: u64 = 200;
const SEED: u64 = 0x005e_ed0f_b00c;

/// A small deterministic generator (xorshift64*): the run is reproducible
/// from its seed, with no new dependency.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn d(m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, m, day).unwrap()
}

fn month_end(m: u32) -> NaiveDate {
    if m == 12 {
        d(12, 31)
    } else {
        d(m + 1, 1).pred_opt().unwrap()
    }
}

/// Random books through the engine's public commands. Closing months
/// (when `closes`) happens in order, after the month's entries.
fn generate(seed: u64, closes: bool, fx: bool) -> (Ledger, Vec<AccountId>) {
    let mut r = Rng(seed | 1);
    let mut l = Ledger::new(Currency::USD);
    let chart = [
        ("1000", "Cash", AccountType::Asset),
        ("1500", "Equipment", AccountType::Asset),
        ("2000", "Payables", AccountType::Liability),
        ("3000", "Capital", AccountType::Equity),
        ("4000", "Sales", AccountType::Revenue),
        ("4100", "Interest", AccountType::Revenue),
        ("6000", "Rent", AccountType::Expense),
        ("6100", "Software", AccountType::Expense),
    ];
    let ids: Vec<AccountId> = chart
        .iter()
        .map(|(c, n, t)| l.add_account(c, n, *t, None, false).unwrap())
        .collect();
    let mut posted = Vec::new();
    for month in 1..=6u32 {
        let entries = 1 + r.below(6);
        for _ in 0..entries {
            let day = 1 + r.below(28) as u32;
            let e = l.create_entry(d(month, day), "generated");
            let lines = 1 + r.below(3);
            let mut net = Decimal::ZERO;
            for _ in 0..lines {
                let account = ids[r.below(ids.len() as u64) as usize];
                let side = if r.below(2) == 0 {
                    Side::Debit
                } else {
                    Side::Credit
                };
                if fx && r.below(8) == 0 {
                    // A foreign line: odd decimal places in the base amount.
                    let foreign = Decimal::new(1 + r.below(500_000) as i64, 2);
                    let rate = Decimal::new(5_000 + r.below(20_000) as i64, 4);
                    l.add_line_fx(e, account, side, Currency::EUR, foreign, rate)
                        .unwrap();
                    // Balance against what was booked (rounded to the cent).
                    let base = l
                        .get_entry(e)
                        .unwrap()
                        .lines()
                        .last()
                        .unwrap()
                        .amount
                        .amount;
                    net += if side == Side::Debit { base } else { -base };
                } else {
                    let amount = Decimal::new(1 + r.below(1_000_000) as i64, 2);
                    l.add_line(e, account, side, Money::new(amount, Currency::USD))
                        .unwrap();
                    net += if side == Side::Debit { amount } else { -amount };
                }
            }
            // A plug line on cash balances the entry.
            if !net.is_zero() {
                let side = if net > Decimal::ZERO {
                    Side::Credit
                } else {
                    Side::Debit
                };
                l.add_line(e, ids[0], side, Money::new(net.abs(), Currency::USD))
                    .unwrap();
            } else {
                let amount = Money::new(Decimal::ONE, Currency::USD);
                l.add_line(e, ids[0], Side::Debit, amount).unwrap();
                l.add_line(e, ids[3], Side::Credit, amount).unwrap();
            }
            if l.post_entry(e).is_ok() {
                posted.push(e);
            }
        }
        // Now and then, reverse an earlier entry of this month.
        if r.below(3) == 0
            && let Some(&e) = posted.last()
        {
            let date = l.get_entry(e).unwrap().date();
            let _ = l.reverse_entry(e, date, "reversal");
        }
        if closes {
            let p = l
                .create_period(&format!("2026-{month:02}"), d(month, 1), month_end(month))
                .unwrap();
            l.close_period(p).unwrap();
        }
    }
    (l, ids)
}

fn book_json(l: &Ledger, as_of: &[NaiveDate]) -> Value {
    let accounts: Vec<Value> = l
        .chart()
        .all()
        .map(|a| {
            json!({"id": a.id, "code": a.code, "name": a.name,
                   "type": format!("{:?}", a.account_type)})
        })
        .collect();
    let entries: Vec<Value> = l
        .posted_entries()
        .map(|e| {
            json!({
                "date": e.date().to_string(),
                "description": e.description(),
                "lines": e.lines().iter().map(|ln| json!({
                    "account": ln.account,
                    "side": if ln.side == Side::Debit { "debit" } else { "credit" },
                    "amount": ln.amount.amount.to_string(),
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    json!({
        "accounts": accounts,
        "entries": entries,
        "as_of": as_of.iter().map(|d| d.to_string()).collect::<Vec<_>>(),
    })
}

/// wanderbooks' trial balance as of a date, as GnuCash reports balances:
/// debits positive, by account id, zero for accounts not listed.
fn ours(l: &Ledger, as_of: NaiveDate) -> BTreeMap<AccountId, Decimal> {
    let tb = trial_balance_as_of(l, as_of);
    let mut out: BTreeMap<AccountId, Decimal> =
        l.chart().all().map(|a| (a.id, Decimal::ZERO)).collect();
    for line in tb.lines {
        let id = l.chart().find_by_code(&line.code).unwrap().id;
        out.insert(id, line.debit - line.credit);
    }
    out
}

fn gnucash(books: &[Value]) -> Vec<Value> {
    let mut child = Command::new("docker")
        .args(["run", "--rm", "-i", "wanderbooks-gnucash-oracle"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("docker is needed: run `just oracle`");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(serde_json::to_string(books).unwrap().as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "the GnuCash oracle failed; build it with `just oracle`"
    );
    serde_json::from_slice(&out.stdout).expect("oracle output is JSON")
}

fn dec(v: &Value) -> Decimal {
    v.as_str().unwrap().parse().unwrap()
}

#[test]
#[ignore = "needs Docker and the GnuCash image: `just oracle`"]
fn balances_match_gnucash_on_generated_books() {
    let mut cases = Vec::new();
    for i in 0..BOOKS {
        let seed = SEED.wrapping_add(i.wrapping_mul(0x9e37_79b9_7f4a_7c15));
        let closes = i % 2 == 1;
        let (l, _) = generate(seed, closes, true);
        let mut r = Rng(seed ^ 0xa5a5);
        let mut as_of: Vec<NaiveDate> = (1..=6).map(month_end).collect();
        for _ in 0..3 {
            as_of.push(d(1 + r.below(6) as u32, 1 + r.below(28) as u32));
        }
        cases.push((seed, closes, l, as_of));
    }
    let books: Vec<Value> = cases
        .iter()
        .map(|(_, _, l, as_of)| book_json(l, as_of))
        .collect();
    let results = gnucash(&books);
    assert_eq!(results.len(), cases.len());

    let mut diffs = Vec::new();
    let mut compared = 0usize;
    for ((seed, closes, l, as_of), result) in cases.iter().zip(&results) {
        for date in as_of {
            let theirs = &result["balances"][date.to_string()];
            for (id, mine) in ours(l, *date) {
                let g = dec(&theirs[id.to_string()]);
                compared += 1;
                if g != mine {
                    let code = &l.chart().get(id).unwrap().code;
                    diffs.push(format!(
                        "seed {seed:#x} (closes: {closes}) account {code} as of {date}: wanderbooks {mine}, GnuCash {g}"
                    ));
                }
            }
        }
        // A month's P&L is the change in its income and expense balances,
        // on books with no closing entries (GnuCash books a close as an
        // ordinary transaction; wanderbooks' period P&L leaves it out).
        if !closes {
            for m in 2..=6u32 {
                let pnl = profit_and_loss_for_period(l, d(m, 1), month_end(m), None).unwrap();
                let before = &result["balances"][month_end(m - 1).to_string()];
                let after = &result["balances"][month_end(m).to_string()];
                let mut gnucash_net = Decimal::ZERO;
                for a in l.chart().all() {
                    if matches!(a.account_type, AccountType::Revenue | AccountType::Expense) {
                        let k = a.id.to_string();
                        // Income is credit-normal: net income = -(change).
                        gnucash_net -= dec(&after[&k]) - dec(&before[&k]);
                    }
                }
                compared += 1;
                if gnucash_net != pnl.net_income() {
                    diffs.push(format!(
                        "seed {seed:#x} P&L 2026-{m:02}: wanderbooks {}, GnuCash {gnucash_net}",
                        pnl.net_income()
                    ));
                }
            }
        }
    }
    eprintln!(
        "compared {compared} balances and P&Ls across {} books",
        cases.len()
    );
    assert!(
        diffs.is_empty(),
        "{} difference(s) with GnuCash; adjudicate each from rules.md:\n{}",
        diffs.len(),
        diffs.join("\n")
    );
}
