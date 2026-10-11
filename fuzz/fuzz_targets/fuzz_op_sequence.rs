// Fuzz target: arbitrary operation sequences against a Ledger. Errors
// are fine (invalid ops are rejected); the invariant is that the trial
// balance ALWAYS balances afterward, no matter the sequence. This is the
// stateful counterpart of the proptest properties — libfuzzer explores
// deeper interleavings. Staged here; `cargo fuzz init` wires it up.
#![no_main]

use arbitrary::Arbitrary;
use chrono::NaiveDate;
use ledger_core::{AccountType, Currency, Ledger, Money, Side};
use libfuzzer_sys::fuzz_target;
use rust_decimal::Decimal;

#[derive(Arbitrary, Debug, Clone, Copy)]
enum Op {
    AddAccount { code: u8, typ: u8 },
    CreateEntry { day: u8 },
    AddLine {
        entry: u8,
        account: u8,
        debit: bool,
        cents: u32,
    },
    PostEntry { entry: u8 },
    ReverseEntry { entry: u8 },
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

fuzz_target!(|ops: Vec<Op>| {
    let mut ledger = Ledger::new(Currency::USD);
    let mut entry_ids: Vec<u64> = Vec::new();
    let mut next_code: u32 = 1000;

    for op in ops {
        match op {
            Op::AddAccount { code, typ } => {
                // Unique codes by construction; duplicates are rejected.
                let c = format!("{:04}", next_code + (code as u32 % 50));
                next_code += 1;
                let _ = ledger.add_account(&c, "fuzz", account_type(typ), None, false);
            }
            Op::CreateEntry { day } => {
                let date =
                    NaiveDate::from_ymd_opt(2026, 1, 1 + (day as u32 % 28)).unwrap();
                let id = ledger.create_entry(date, "fuzz");
                entry_ids.push(id);
            }
            Op::AddLine {
                entry,
                account,
                debit,
                cents,
            } => {
                if entry_ids.is_empty() {
                    continue;
                }
                let eid = entry_ids[entry as usize % entry_ids.len()];
                // Account ids are 1-based; may not exist — rejection is fine.
                let aid = (account as u64 % 60) + 1;
                let side = if debit { Side::Debit } else { Side::Credit };
                let amount = Decimal::new((cents % 1_000_000) as i64, 2);
                let _ = ledger.add_line(eid, aid, side, Money::new(amount, Currency::USD));
            }
            Op::PostEntry { entry } => {
                if entry_ids.is_empty() {
                    continue;
                }
                let eid = entry_ids[entry as usize % entry_ids.len()];
                let _ = ledger.post_entry(eid);
            }
            Op::ReverseEntry { entry } => {
                if entry_ids.is_empty() {
                    continue;
                }
                let eid = entry_ids[entry as usize % entry_ids.len()];
                let _ = ledger.reverse_entry(
                    eid,
                    NaiveDate::from_ymd_opt(2026, 12, 31).unwrap(),
                    "fuzz reversal",
                );
            }
        }
    }

    // THE invariant: no sequence of operations, valid or rejected, may
    // leave the trial balance out of balance.
    let tb = ledger_core::reports::trial_balance(&ledger);
    assert_eq!(
        tb.total_debits, tb.total_credits,
        "trial balance broken after fuzzed op sequence"
    );
});
