fn main() {
    let root = std::path::PathBuf::from(std::env::args().nth(1).unwrap());
    let mut names: Vec<_> = std::fs::read_dir(&root).unwrap().map(|e| e.unwrap().path()).collect();
    names.sort();
    for dir in names {
        let n = dir.file_name().unwrap().to_string_lossy().to_string();
        let db = dir.join(ledger_store::DB_FILE);
        let _ = std::fs::remove_file(&db);
        match ledger_store::Store::open(&dir) {
            Ok(s) => println!("OPENS {n}: verify={:?}", s.verify().map(|_| ())),
            Err(e) => println!("FAILS {n}: {e}"),
        }
        match ledger_store::Snapshot::load(&dir) { Ok(_) => println!("   viewer ok"), Err(e) => println!("   viewer FAILS: {e}") }
    }
}
