// Fuzz target: arbitrary JSON as a journal entry. If validation accepts
// it, the entry MUST balance exactly — validation is the enforcement
// point, and this pins it. Staged here; `cargo fuzz init` wires it up.
#![no_main]

use libfuzzer_sys::fuzz_target;
use ledger_core::{AccountType, Currency, JournalEntry, Ledger, Side};
use rust_decimal::Decimal;

fn test_chart() -> Ledger {
    let mut ledger = Ledger::new(Currency::USD);
    for (code, name, typ) in [
        ("1000", "Cash", AccountType::Asset),
        ("2000", "Payable", AccountType::Liability),
        ("3000", "Equity", AccountType::Equity),
        ("4000", "Revenue", AccountType::Revenue),
        ("6000", "Rent", AccountType::Expense),
    ] {
        ledger.add_account(code, name, typ, None, false).unwrap();
    }
    ledger
}

fuzz_target!(|data: &[u8]| {
    if let Ok(entry) = serde_json::from_slice::<JournalEntry>(data) {
        let ledger = test_chart();
        // Validation must never panic on hostile input.
        if entry.validate(ledger.chart()).is_ok() {
            let debits: Decimal = entry
                .lines()
                .iter()
                .filter(|l| l.side == Side::Debit)
                .map(|l| l.amount.amount)
                .sum();
            let credits: Decimal = entry
                .lines()
                .iter()
                .filter(|l| l.side == Side::Credit)
                .map(|l| l.amount.amount)
                .sum();
            assert_eq!(
                debits, credits,
                "validate() accepted an unbalanced entry: {entry:?}"
            );
            assert!(
                debits > Decimal::ZERO,
                "validate() accepted a zero-total entry"
            );
        }
    }
});
