//! Books written by a newer wanderbooks: the viewer says to update, never
//! that the books are corrupt, on its first load and while following them.
//!
//! ```text
//! GOAL: Tobin, Mo or the CPA, on an older viewer, learn to update it,
//!       not that the books are damaged.
//! SOURCE: rules.md B-27; plan/served.md 5.1 (the viewer reads the journal
//!         directly, so it meets the same gate).
//! ```

use chrono::NaiveDate;
use ledger_core::{AccountType, Currency};
use ledger_gui::data::{self, Loader, Source, Update};
use ledger_store::{JOURNAL_FILE, JournalWriter, Store};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// A test's data dir; its books dir is removed when the test ends, pass or
/// panic.
struct DataDir {
    dir: PathBuf,
    data: PathBuf,
}

impl std::ops::Deref for DataDir {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.data
    }
}

impl Drop for DataDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn books(tag: &str) -> DataDir {
    let dir = std::env::temp_dir().join(format!("ledger-gui-gate-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let data = dir.join("books-data");
    let mut store = Store::create(&data, Currency::USD).unwrap();
    store
        .add_account("1000", "Checking", AccountType::Asset, None, false)
        .unwrap();
    DataDir { dir, data }
}

fn source(data: &Path) -> Source {
    Source {
        data_dir: data.to_path_buf(),
        today: NaiveDate::from_ymd_opt(2026, 10, 16),
    }
}

fn rate() -> serde_json::Value {
    json!({"currency": "EUR", "date": "2026-10-01", "rate": "1.10"})
}

/// The last line can't be checked: the viewer says so, and never that the
/// books are not damaged (rules.md B-27).
fn says_newer_or_damaged(text: &str) {
    assert!(text.starts_with("Journal line"), "{text}");
    assert!(text.contains("or the line is damaged"), "{text}");
    assert!(text.contains("Update wanderbooks"), "{text}");
    assert!(!text.contains("not damaged"), "{text}");
}

fn says_update(text: &str) {
    assert!(
        text.starts_with("These books were written by a newer version of wanderbooks"),
        "{text}"
    );
    assert!(text.contains("Update wanderbooks"), "{text}");
    assert!(!text.contains("corrupt"), "{text}");
}

/// Opening: a newer event type, or a newer declared format, reads as
/// "update"; a damaged journal still reads as corrupt.
#[test]
fn the_viewer_says_update_for_newer_books_and_corrupt_for_damaged_ones() {
    for (tag, line_type, payload) in [
        ("type", "period.attested", json!({"key": "company.name"})),
        (
            "format",
            "journal.format_set",
            json!({"format": 3, "features": []}),
        ),
    ] {
        let data = books(tag);
        let mut w = JournalWriter::open(&data.join(JOURNAL_FILE)).unwrap();
        w.append(line_type, payload.clone()).unwrap();
        // The next line chains onto it: its bytes are checked.
        w.append("fx.rate_set", rate()).unwrap();
        let err = data::load_once(&source(&data))
            .err()
            .expect("newer books loaded");
        says_update(&err);

        // As the last line nothing checks it: newer, or damaged.
        let data = books(&format!("{tag}-last"));
        JournalWriter::open(&data.join(JOURNAL_FILE))
            .unwrap()
            .append(line_type, payload)
            .unwrap();
        let err = data::load_once(&source(&data))
            .err()
            .expect("newer books loaded");
        says_newer_or_damaged(&err);
    }

    let data = books("damaged");
    let path = data.join(JOURNAL_FILE);
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, text.replacen("\"prev\":\"", "\"prev\":\"1", 2)).unwrap();
    let err = data::load_once(&source(&data))
        .err()
        .expect("damaged books loaded");
    assert!(err.contains("journal corrupt"), "{err}");
    assert!(!err.contains("newer"), "{err}");
}

/// Following: the viewer has the books open when a newer wanderbooks
/// writes a line this one can't read. It says to update, and keeps the
/// books it had.
#[test]
fn a_live_viewer_says_update_when_the_books_move_ahead_of_it() {
    let data = books("live");
    let loader = Loader::spawn(source(&data), egui::Context::default());
    match loader.wait(Duration::from_secs(10)) {
        Some(Update::Books(_)) => {}
        _ => panic!("no initial load"),
    }
    let mut w = JournalWriter::open(&data.join(JOURNAL_FILE)).unwrap();
    w.append("entry.evidence_inherited", json!({"entry_id": 1}))
        .unwrap();
    w.append("fx.rate_set", rate()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut said = None;
    while Instant::now() < deadline {
        match loader.wait(Duration::from_millis(200)) {
            Some(Update::Error(e)) => {
                said = Some(e);
                break;
            }
            Some(Update::Books(_)) => panic!("the viewer read a line it doesn't know"),
            _ => {}
        }
    }
    says_update(&said.expect("the viewer never said to update"));
}

/// A declared format this version reads opens like any books, and the
/// journal page tells it as its own event (it is no step of an entry).
#[test]
fn a_format_this_version_reads_opens_and_the_journal_page_tells_it() {
    let data = books("current-format");
    let mut w = JournalWriter::open(&data.join(JOURNAL_FILE)).unwrap();
    w.append(
        ledger_store::format::FORMAT_SET,
        json!({"format": ledger_store::format::SUPPORTED_JOURNAL_FORMAT, "features": []}),
    )
    .unwrap();
    w.append("fx.rate_set", rate()).unwrap();
    let b = data::load_once(&source(&data)).expect("books in a format this version reads");
    let told: Vec<_> = b
        .journal
        .iter()
        .filter(|l| l.line_type == ledger_store::format::FORMAT_SET)
        .map(|l| ledger_gui::pages::journal::describe(&b, l).map(|e| (e.text, e.route)))
        .collect();
    assert_eq!(
        told,
        [Some((
            format!(
                "Journal format {} declared",
                ledger_store::format::SUPPORTED_JOURNAL_FORMAT
            ),
            None
        ))]
    );
}
