// Fuzz target: arbitrary command sequences against a Ledger, including
// balanced pairs, foreign-currency lines, periods, and closes, with
// amounts built from raw decimal bits (huge, tiny, odd scales). Refusals
// are fine; panics, hangs, and broken books are not. After any sequence:
// the trial balance balances, every posted entry validates, and no draft
// sits in a closed period (the Lean invariant Inv).
#![no_main]

use arbitrary::Arbitrary;
use chrono::NaiveDate;
use ledger_core::{AccountType, Currency, EntryStatus, Ledger, Money, Side};
use libfuzzer_sys::fuzz_target;
use rust_decimal::Decimal;

#[derive(Arbitrary, Debug, Clone, Copy)]
enum Op {
    AddAccount { code: u8, typ: u8, placeholder: bool },
    CreateEntry { day: u8 },
    AddLine { entry: u8, account: u8, debit: bool, amount: Amount },
    Pair { entry: u8, debit: u8, credit: u8, amount: Amount },
    AddLineFx { entry: u8, account: u8, debit: bool, foreign: Amount, rate: Amount },
    PostEntry { entry: u8 },
    ReverseEntry { entry: u8, day: u8 },
    CreatePeriod { start: u8, len: u8 },
    ClosePeriod { period: u8 },
}

/// A decimal from raw parts: any mantissa up to 96 bits, any scale 0-28.
#[derive(Arbitrary, Debug, Clone, Copy)]
struct Amount {
    lo: u32,
    mid: u32,
    hi: u32,
    negative: bool,
    scale: u8,
    small: bool,
}

impl Amount {
    fn get(self) -> Decimal {
        if self.small {
            // Mostly ordinary money, so entries balance and post.
            return Decimal::new(i64::from(self.lo % 1_000_000), 2);
        }
        Decimal::from_parts(self.lo, self.mid, self.hi, self.negative, u32::from(self.scale % 29))
    }
}

fn account_type(t: u8) -> AccountType {
    match t % 5 {
        0 => AccountType::Asset,
        1 => AccountType::Liability,
        2 => AccountType::Equity,
        3 => AccountType::Revenue,
        _ => AccountType::Expense,
    }
}

fn day(d: u8) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 1, 1).unwrap() + chrono::Days::new(u64::from(d % 200))
}

fuzz_target!(|ops: Vec<Op>| {
    let mut l = Ledger::new(Currency::USD);
    let mut entries: Vec<u64> = Vec::new();
    let pick = |v: &[u64], i: u8| v.get(i as usize % v.len().max(1)).copied();
    for op in ops.into_iter().take(200) {
        let accounts: Vec<u64> = l.chart().all().map(|a| a.id).collect();
        match op {
            Op::AddAccount { code, typ, placeholder } => {
                let _ = l.add_account(&format!("{}", 1000 + u32::from(code % 40)), "f", account_type(typ), None, placeholder);
            }
            Op::CreateEntry { day: d } => {
                if let Ok(id) = l.try_create_entry(day(d), "f") {
                    entries.push(id);
                }
            }
            Op::AddLine { entry, account, debit, amount } => {
                if let (Some(e), Some(a)) = (pick(&entries, entry), pick(&accounts, account)) {
                    let side = if debit { Side::Debit } else { Side::Credit };
                    let _ = l.add_line(e, a, side, Money::new(amount.get(), Currency::USD));
                }
            }
            Op::Pair { entry, debit, credit, amount } => {
                if let (Some(e), Some(a), Some(b)) =
                    (pick(&entries, entry), pick(&accounts, debit), pick(&accounts, credit))
                {
                    let m = Money::new(amount.get(), Currency::USD);
                    let _ = l.add_line(e, a, Side::Debit, m);
                    let _ = l.add_line(e, b, Side::Credit, m);
                }
            }
            Op::AddLineFx { entry, account, debit, foreign, rate } => {
                if let (Some(e), Some(a)) = (pick(&entries, entry), pick(&accounts, account)) {
                    let side = if debit { Side::Debit } else { Side::Credit };
                    let _ = l.add_line_fx(e, a, side, Currency::EUR, foreign.get(), rate.get());
                }
            }
            Op::PostEntry { entry } => {
                if let Some(e) = pick(&entries, entry) {
                    let _ = l.post_entry(e);
                }
            }
            Op::ReverseEntry { entry, day: d } => {
                if let Some(e) = pick(&entries, entry)
                    && let Ok(r) = l.reverse_entry(e, day(d), "f")
                {
                    entries.push(r);
                }
            }
            Op::CreatePeriod { start, len } => {
                let _ = l.create_period("p", day(start), day(start.saturating_add(len % 60)));
            }
            Op::ClosePeriod { period } => {
                let ids: Vec<u64> = l.list_periods().iter().map(|p| p.id).collect();
                if let Some(p) = pick(&ids, period) {
                    let _ = l.close_period(p);
                }
            }
        }
    }
    let tb = ledger_core::reports::trial_balance(&l);
    assert_eq!(tb.total_debits, tb.total_credits, "trial balance broken");
    for e in l.posted_entries() {
        assert!(e.validate(l.chart()).is_ok(), "posted entry {} invalid", e.id());
    }
    for id in 1..l.next_entry_id() {
        if let Some(e) = l.get_entry(id)
            && e.status() == EntryStatus::Draft
        {
            assert!(l.closed_period_containing(e.date()).is_none(), "draft in a closed period");
        }
    }
});
