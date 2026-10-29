//! What MAIN computes for each books dir: verify, the canonical report
//! snapshot, and every record as replayed. Written to <dir>/expected.json.
fn main() {
    let root = std::path::PathBuf::from(std::env::args().nth(1).unwrap());
    let mut names: Vec<_> = std::fs::read_dir(&root).unwrap().map(|e| e.unwrap().path()).filter(|p| p.is_dir()).collect();
    names.sort();
    for dir in names {
        let n = dir.file_name().unwrap().to_string_lossy().to_string();
        let _ = std::fs::remove_file(dir.join(ledger_store::db_file()));
        let s = match ledger_store::Store::open(&dir) { Ok(s) => s, Err(e) => { println!("FAILS {n}: {e}"); continue } };
        let l = s.ledger();
        let entries: Vec<_> = (1..l.next_entry_id()).filter_map(|id| l.get_entry(id)).collect();
        let parties: Vec<_> = (1..l.next_entry_id()).map(|id| (id, l.entry_parties(id))).filter(|(_, p)| !p.is_empty()).collect();
        let v = serde_json::json!({
            "verify": s.verify().map(|_| ()).map_err(|e| e.to_string()),
            "snapshot": ledger_core::reports::snapshot(l).map_err(|e| e.to_string()),
            "entries": entries,
            "assets": l.registered_assets().collect::<Vec<_>>(),
            "periods": l.list_periods(),
            "schedules": l.schedules().collect::<Vec<_>>(),
            "recurring": l.recurring_entries().collect::<Vec<_>>(),
            "invoices": l.invoices(),
            "bills": l.bills(),
            "payroll_runs": l.list_payroll_runs(),
            "entry_parties": parties,
        });
        // Pretty for a reader; one line for books of many entries.
        let text = if entries.len() > 100 { serde_json::to_string(&v) } else { serde_json::to_string_pretty(&v) };
        std::fs::write(dir.join("expected.json"), text.unwrap() + "\n").unwrap();
        println!("EXPECTED {n}: verify={}", v["verify"]);
    }
}
