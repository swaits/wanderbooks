// Fuzz target: every type the store persists from ledger-core, fed
// arbitrary bytes. Garbage must be refused, never panic; whatever parses
// must round-trip: serialize, parse again, serialize, same bytes.
#![no_main]

use libfuzzer_sys::fuzz_target;
use serde::{Serialize, de::DeserializeOwned};

fn round_trip<T: Serialize + DeserializeOwned>(data: &[u8]) {
    if let Ok(v) = serde_json::from_slice::<T>(data) {
        let once = serde_json::to_string(&v).expect("a parsed value serializes");
        let again: T = serde_json::from_str(&once).expect("its own output parses");
        assert_eq!(serde_json::to_string(&again).unwrap(), once, "not a fixed point");
    }
}

fuzz_target!(|data: &[u8]| {
    round_trip::<ledger_core::JournalEntry>(data);
    round_trip::<ledger_core::Asset>(data);
    round_trip::<ledger_core::Period>(data);
    round_trip::<ledger_core::Invoice>(data);
    round_trip::<ledger_core::Bill>(data);
    round_trip::<ledger_core::AmortizationSchedule>(data);
    round_trip::<ledger_core::RecurringEntry>(data);
    round_trip::<ledger_core::PayrollRun>(data);
    round_trip::<ledger_core::Money>(data);
});
