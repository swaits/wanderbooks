// Fuzz target: journal line parsing must never panic, and the canonical
// form must be a fixed point: parse(canonical(x)) re-serializes to the
// same canonical bytes.
#![no_main]

use libfuzzer_sys::fuzz_target;
use ledger_store::JournalLine;

fuzz_target!(|data: &[u8]| {
    // Garbage must return Err, never panic.
    // Parse as the journal reader does (it refuses floats: the journal
    // never holds one, and a float's canonical form isn't a fixed point).
    if let Ok(line) = JournalLine::parse(data) {
        let canon = line.to_canonical();
        let line2 = JournalLine::parse(canon.as_bytes()).expect("canonical form must re-parse");
        assert_eq!(
            line2.to_canonical(),
            canon,
            "canonical form is not a fixed point"
        );
        // Sequence numbers survive the round trip.
        assert_eq!(line.seq, line2.seq);
        assert_eq!(line.line_type, line2.line_type);
    }
});
