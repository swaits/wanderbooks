// Fuzz target: `wanderbooks.toml` is written by people and agents, so its
// text is untrusted input. Parsing must return settings or an error, never
// panic or hang, and what it accepts must honor parse's invariants: the
// fiscal year starts on the first of a month, and members' shares (each in
// (0, 1]) sum to exactly 1.
#![no_main]

use libfuzzer_sys::fuzz_target;
use rust_decimal::Decimal;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    if let Ok(s) = ledger_config::parse(text) {
        let (m, d) = s.company.fiscal_year_start;
        assert!(
            (1..=12).contains(&m) && d == 1,
            "{:?}",
            s.company.fiscal_year_start
        );
        if !s.company.members.is_empty() {
            let total: Decimal = s.company.members.iter().map(|m| m.share).sum();
            assert_eq!(total, Decimal::ONE, "{:?}", s.company.members);
            for mem in &s.company.members {
                assert!(
                    mem.share > Decimal::ZERO && mem.share <= Decimal::ONE,
                    "{mem:?}"
                );
            }
        }
    }
});
