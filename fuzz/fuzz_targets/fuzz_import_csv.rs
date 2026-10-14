// Fuzz target: CSV files a person hands the agent (a bank export, a chart
// of accounts) are untrusted input. Parsing must return rows or an error,
// never panic or hang, and what it accepts must be usable as money:
// every bank amount nonzero, and a chart row's code and name non-empty.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else { return };
    if let Ok(txns) = ledger_import::parse_bank_csv(text) {
        for t in &txns {
            assert!(!t.amount.is_zero(), "a zero amount was kept: {t:?}");
        }
    }
    if let Ok(rows) = ledger_import::parse_chart_csv(text) {
        for r in &rows {
            assert!(!r.code.trim().is_empty() && !r.name.trim().is_empty(), "{r:?}");
        }
    }
});
