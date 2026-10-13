// Fuzz target: a journal file on disk is untrusted input (a bad backup, a
// hand edit, disk corruption). Opening books from arbitrary journal bytes
// must return an error, never panic, and never hang.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let dir = std::env::temp_dir().join(format!("wb-fuzz-open-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(ledger_store::JOURNAL_FILE), data).unwrap();
    let _ = ledger_store::Store::open(&dir);
});
