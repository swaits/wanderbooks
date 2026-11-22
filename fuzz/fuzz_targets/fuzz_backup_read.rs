// Fuzz target: a backup folder is read from a disk someone else may have
// written (a sync service's, another computer's, a damaged one), so every
// file of it is untrusted input: the manifest, the header, a piece. Finding
// and proving a backup must return books or an error, never panic or hang,
// and it must never accept what is not whole.
#![no_main]

use libfuzzer_sys::fuzz_target;
use std::path::Path;

fn put(root: &Path, rel: &str, bytes: &[u8]) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, bytes).unwrap();
}

fuzz_target!(|data: &[u8]| {
    // The input is split in three: a manifest, a header and a piece.
    let third = data.len() / 3;
    let (manifest, rest) = data.split_at(third);
    let (header, piece) = rest.split_at(third.min(rest.len()));
    let dir = tempfile::tempdir().unwrap();
    let set = dir.path().join("acme");
    put(&set, "manifests/000001.json", manifest);
    put(&set, "manifests/000001", manifest);
    put(&set, "header.json", header);
    put(&set, "journal/000001-0000000000000000", piece);
    put(&set, "pieces/00", piece);
    let ctx = ledger_store::backup::Ctx::new();
    for pass in [None, Some("a passphrase of enough length")] {
        if let Ok(found) = ledger_store::backup::find(dir.path(), pass, &ctx) {
            let scratch = dir.path().join("scratch");
            // Whatever a manifest claims, what is made of this input must
            // fail: nothing in it is a whole backup.
            let proven = ledger_store::backup::verify_found(&found, &scratch, &ctx);
            assert!(proven.is_err(), "random bytes proved whole");
            let _ = std::fs::remove_dir_all(&scratch);
        }
    }
});
