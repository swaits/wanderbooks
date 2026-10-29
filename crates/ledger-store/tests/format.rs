//! The format gate: books written by a newer wanderbooks are refused as
//! newer, never as corrupt, and never read in part.
//!
//! ```text
//! GOAL: an older wanderbooks tells the human to update, not that the
//!       books are damaged, and never shows books that differ from the
//!       record without saying so.
//! SOURCE: rules.md B-27; plan/served.md 5.1 (format gate).
//! INVARIANTS:
//!   1. A whole, chained line of a type this version doesn't know, or a
//!      journal.format_set newer than it reads, is NewerJournal: from
//!      Store::open, Snapshot::load and Snapshot::refresh alike.
//!   2. Damage (a bad hash, a bad seq, a line that isn't JSON) stays
//!      JournalCorrupt, whatever the damaged line's type.
//!   3. A declared format this version reads replays like any other line.
//! ```

use chrono::NaiveDate;
use ledger_core::arap::{DocumentLine, SendKind, Sent, SentVia};
use ledger_core::{AccountType, Currency, Money, Side};
use ledger_store::format::{DOCUMENTS_V2, EVENT_TYPES, FORMAT_SET, KNOWN_FEATURES};
use ledger_store::journal::{canonical_json, sha256_hex};
use ledger_store::{
    JOURNAL_FILE, JournalFormat, JournalWriter, SUPPORTED_JOURNAL_FORMAT, Snapshot, Store,
    StoreError, db_file,
};
use rust_decimal::Decimal;
use serde_json::json;
use std::path::{Path, PathBuf};

/// A test's dir, removed when the test ends, pass or panic.
struct TestDir(PathBuf);

impl std::ops::Deref for TestDir {
    type Target = std::path::Path;
    fn deref(&self) -> &std::path::Path {
        &self.0
    }
}

impl AsRef<std::path::Path> for TestDir {
    fn as_ref(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn test_dir(name: &str) -> TestDir {
    let dir =
        std::env::temp_dir().join(format!("wanderbooks-format-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    TestDir(dir)
}

/// Books with one posted entry, closed again: in journal format 1, as
/// every version before the format gate wrote them (this version's own
/// declare its format as they are created).
fn books(dir: &Path) {
    let mut store = Store::create_in_format(dir, Currency::USD, &JournalFormat::default()).unwrap();
    let cash = store
        .add_account("1000", "Cash", AccountType::Asset, None, false)
        .unwrap();
    let equity = store
        .add_account("3000", "Capital", AccountType::Equity, None, false)
        .unwrap();
    let e = store
        .create_entry(NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(), "Capital")
        .unwrap();
    let m = Money::new(Decimal::from(500), Currency::USD);
    store.add_line(e, cash, Side::Debit, m).unwrap();
    store.add_line(e, equity, Side::Credit, m).unwrap();
    store.post_entry(e).unwrap();
}

/// Append whole, correctly chained lines, as a newer wanderbooks would,
/// returning their seqs.
fn append(dir: &Path, lines: &[(&str, serde_json::Value)]) -> Vec<u64> {
    let mut w = JournalWriter::open(&dir.join(JOURNAL_FILE)).unwrap();
    lines
        .iter()
        .map(|(t, p)| w.append(t, p.clone()).unwrap().seq)
        .collect()
}

/// NewerJournal for line `want_seq`. `want_verified`: the next line
/// chains onto it, so that line is intact (and the message vouches for
/// nothing after it); otherwise the message says newer or damaged, never
/// "intact".
fn assert_newer<T>(
    got: Result<T, StoreError>,
    want_seq: u64,
    want_type: &str,
    want_need: &str,
    want_verified: bool,
) {
    match got {
        Err(e @ StoreError::NewerJournal { .. }) => {
            let text = e.to_string();
            let StoreError::NewerJournal {
                seq,
                event_type,
                need,
                verified,
            } = e
            else {
                unreachable!()
            };
            assert_eq!(seq, want_seq);
            assert_eq!(event_type, want_type);
            assert_eq!(need, want_need);
            assert_eq!(verified, want_verified, "{text}");
            assert!(text.contains("newer version of wanderbooks"), "{text}");
            assert!(text.contains("Update wanderbooks"), "{text}");
            assert!(!text.contains("corrupt"), "{text}");
            assert!(!text.contains("not damaged"), "{text}");
            if verified {
                assert!(text.contains(&format!("Line {seq} is intact")), "{text}");
                assert!(text.contains("can't check the lines after it"), "{text}");
            } else {
                assert!(!text.contains("intact"), "{text}");
                assert!(text.contains("or the line is damaged"), "{text}");
                assert!(text.contains("run verify"), "{text}");
            }
        }
        Err(e) => panic!("expected NewerJournal, got {e:?}"),
        Ok(_) => panic!("expected NewerJournal, the books opened"),
    }
}

fn assert_corrupt<T>(got: Result<T, StoreError>) {
    match got {
        Err(StoreError::JournalCorrupt(_)) => {}
        Err(e) => panic!("expected JournalCorrupt, got {e:?}"),
        Ok(_) => panic!("expected JournalCorrupt, the books opened"),
    }
}

/// Oracle (plan/served.md 5.1): an older reader meets books a newer
/// wanderbooks wrote. A valid line of an unknown type, and a declared
/// format 3, are each NewerJournal, from the writer and the viewer alike,
/// and the refusal reads nothing further: the database isn't rebuilt and
/// the journal is untouched.
#[test]
fn old_reader_gets_newer_journal() {
    let cases = [
        (
            "period.attested",
            json!({"key": "company.name", "value": "X"}),
            "an event type this version doesn't know",
        ),
        (
            FORMAT_SET,
            json!({"format": 3, "features": []}),
            "journal format 3",
        ),
        (
            FORMAT_SET,
            json!({"format": 1, "features": ["actors"]}),
            "journal features actors",
        ),
    ];
    for (i, (line_type, payload, need)) in cases.into_iter().enumerate() {
        let dir = test_dir(&format!("newer-{i}"));
        books(&dir);
        // A later line of a known type changes nothing: the first line
        // this version can't read is the one named.
        let seq = append(
            &dir,
            &[
                (line_type, payload),
                (
                    "fx.rate_set",
                    json!({"currency": "EUR", "date": "2026-10-01", "rate": "1.10"}),
                ),
            ],
        )[0];
        std::fs::remove_file(dir.join(db_file())).unwrap();
        let journal = std::fs::read(dir.join(JOURNAL_FILE)).unwrap();

        assert_newer(Store::open(&dir), seq, line_type, need, true);
        assert!(
            !dir.join(db_file()).exists(),
            "a refused open rebuilt the database"
        );
        assert_eq!(std::fs::read(dir.join(JOURNAL_FILE)).unwrap(), journal);
        assert_newer(Snapshot::load(&dir), seq, line_type, need, true);
    }
}

/// A viewer already following the books meets a newer line mid-stream:
/// refresh says NewerJournal (every time, until it is updated), applies
/// none of the lines written with it (as the writer refuses the books
/// whole), and keeps showing the books as it last read them.
#[test]
fn a_viewer_following_the_books_gets_newer_journal() {
    let dir = test_dir("follow");
    books(&dir);
    let mut snap = Snapshot::load(&dir).unwrap();
    let before = ledger_core::reports::snapshot(snap.ledger()).unwrap();
    let declared = append(&dir, &[(FORMAT_SET, json!({"format": 1, "features": []}))])[0];
    assert_eq!(snap.refresh().unwrap(), 1);
    let seq = append(
        &dir,
        &[
            (
                "fx.rate_set",
                json!({"currency": "EUR", "date": "2026-10-01", "rate": "1.10"}),
            ),
            ("entry.evidence_inherited", json!({"entry_id": 1})),
        ],
    )[1];
    for _ in 0..2 {
        assert_newer(
            snap.refresh(),
            seq,
            "entry.evidence_inherited",
            "an event type this version doesn't know",
            false,
        );
        assert_eq!(snap.journal_seq(), declared);
        assert_eq!(
            ledger_core::reports::snapshot(snap.ledger()).unwrap(),
            before
        );
    }
}

/// The writer and the viewer refuse the same journal the same way, even
/// when an earlier line's facts are bad too: every line passes the gate
/// before any is replayed.
#[test]
fn the_writer_and_the_viewer_agree_on_newer_books() {
    let dir = test_dir("agree");
    books(&dir);
    let seq = append(
        &dir,
        &[
            // Chained and of a known type, but replay refuses it.
            ("entry.posted", json!({"id": 99})),
            ("period.attested", json!({"key": "company.name"})),
        ],
    )[1];
    let need = "an event type this version doesn't know";
    assert_newer(Store::open(&dir), seq, "period.attested", need, false);
    assert_newer(Snapshot::load(&dir), seq, "period.attested", need, false);
}

/// Damage stays corruption, whatever the damaged line's type: the gate
/// sees only lines that parsed, chained and kept their seq, so damage up
/// to and including the first line this version can't read is
/// JournalCorrupt. (Lines after that one are the newer version's to judge:
/// its format may frame them differently.)
#[test]
fn damaged_journals_are_still_corrupt() {
    type Damage = fn(&mut Vec<String>);
    let damages: [(&str, Damage); 4] = [
        ("hash", |lines| {
            let last = lines.last_mut().unwrap();
            *last = last.replace("\"prev\":\"", "\"prev\":\"0");
        }),
        ("seq", |lines| {
            let n = lines.len();
            let last = lines.last_mut().unwrap();
            *last = last.replace(&format!("\"seq\":{n}"), &format!("\"seq\":{}", n + 1));
        }),
        ("json", |lines| {
            let last = lines.last_mut().unwrap();
            last.truncate(last.len() / 2);
        }),
        ("payload", |lines| {
            // The line before, edited after it was written: the last
            // line's prev no longer matches.
            let n = lines.len();
            lines[n - 2] = lines[n - 2].replace("\"Capital\"", "\"Capitol\"");
        }),
    ];
    let newer = [
        (FORMAT_SET, json!({"format": 3, "features": []})),
        ("period.attested", json!({"key": "company.name"})),
    ];
    for (name, damage) in damages {
        for (i, last) in newer.iter().enumerate() {
            let dir = test_dir(&format!("damaged-{name}-{i}"));
            books(&dir);
            append(&dir, std::slice::from_ref(last));
            let path = dir.join(JOURNAL_FILE);
            let mut lines: Vec<String> = std::fs::read_to_string(&path)
                .unwrap()
                .lines()
                .map(str::to_string)
                .collect();
            let whole = lines.clone();
            damage(&mut lines);
            assert_ne!(lines, whole, "{name} damaged nothing");
            std::fs::write(&path, lines.join("\n") + "\n").unwrap();
            assert_corrupt(Store::open(&dir));
            assert_corrupt(Snapshot::load(&dir));
        }
    }
}

/// A declared format this version reads opens, verifies, rebuilds, and
/// reads the same through the viewer; the format in force is recorded.
#[test]
fn a_declared_format_1_replays() {
    let dir = test_dir("format-1");
    books(&dir);
    let mut store = Store::open(&dir).unwrap();
    assert_eq!(store.journal_format(), &JournalFormat::default());
    let before = ledger_core::reports::snapshot(store.ledger()).unwrap();
    store.declare_journal_format(1, &[]).unwrap();
    assert_eq!(store.journal_format().format, 1);
    store.verify().unwrap();
    drop(store);

    let mut store = Store::open(&dir).unwrap();
    store.verify().unwrap();
    store.rebuild().unwrap();
    store.verify().unwrap();
    assert_eq!(store.journal_format(), &JournalFormat::default());
    assert_eq!(
        ledger_core::reports::snapshot(store.ledger()).unwrap(),
        before
    );
    let snap = Snapshot::load(&dir).unwrap();
    assert_eq!(snap.journal_format(), &JournalFormat::default());
    assert_eq!(snap.journal().last().unwrap().line_type, FORMAT_SET);
    assert_eq!(
        ledger_core::reports::snapshot(snap.ledger()).unwrap(),
        before
    );
}

/// A writer never declares a format it couldn't read back: refused, and
/// the journal is byte-identical.
#[test]
fn a_format_this_version_cannot_read_is_never_declared() {
    let dir = test_dir("declare-refused");
    books(&dir);
    let mut store = Store::open(&dir).unwrap();
    let journal = std::fs::read(dir.join(JOURNAL_FILE)).unwrap();
    let unknown = ["no-such-feature"];
    assert!(!KNOWN_FEATURES.contains(&unknown[0]));
    for (format, features) in [
        (SUPPORTED_JOURNAL_FORMAT + 1, &[][..]),
        (0, &[][..]),
        (1, &unknown[..]),
    ] {
        assert!(
            matches!(
                store.declare_journal_format(format, features),
                Err(StoreError::FormatRefused(_))
            ),
            "declared {format} {features:?}"
        );
        assert_eq!(std::fs::read(dir.join(JOURNAL_FILE)).unwrap(), journal);
    }
    assert_eq!(store.journal_format(), &JournalFormat::default());
}

/// Every type the store dispatches on is one the gate admits: a type with
/// a replay or projection arm in store.rs that the list lacked would be
/// refused as newer in the books that hold it. (The converse, every listed
/// type has both arms, is checked inside the crate.)
#[test]
fn every_event_the_store_replays_passes_the_gate() {
    let src = include_str!("../src/store.rs");
    let mut arms: Vec<&str> = src
        .lines()
        .map(str::trim_start)
        .map(|l| l.strip_prefix("| ").unwrap_or(l))
        .filter_map(|l| l.strip_prefix('"'))
        .filter_map(|l| l.split_once('"'))
        .filter(|(_, rest)| {
            let rest = rest.trim_start();
            rest.starts_with("=>") || rest.starts_with('|') || rest.is_empty()
        })
        .map(|(t, _)| t)
        .filter(|t| {
            t.split_once('.').is_some_and(|(a, b)| {
                !a.is_empty()
                    && !b.is_empty()
                    && t.chars().all(|c| {
                        c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '.'
                    })
            })
        })
        .collect();
    arms.sort_unstable();
    arms.dedup();
    assert!(arms.len() > 50, "found only {arms:?}");
    for t in &arms {
        assert!(
            EVENT_TYPES.contains(t),
            "{t} is replayed but not in EVENT_TYPES"
        );
    }
    for t in EVENT_TYPES {
        assert!(
            arms.contains(t),
            "{t} is in EVENT_TYPES but has no arm in store.rs"
        );
    }
}

/// Rewrite the journal's lines with `edit`, as damage on disk would.
fn edit_journal(dir: &Path, edit: impl FnOnce(&mut Vec<String>)) {
    let path = dir.join(JOURNAL_FILE);
    let mut lines: Vec<String> = std::fs::read_to_string(&path)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect();
    let whole = lines.clone();
    edit(&mut lines);
    assert_ne!(lines, whole, "the edit changed nothing");
    std::fs::write(&path, lines.join("\n") + "\n").unwrap();
}

/// A line's own hash is checked only by the next line's prev, so the
/// gate's refusal waits for it (rules.md B-27): a flipped bit in a middle
/// line's type, or a declared format edited from 1 to 2, reads like books
/// from a newer wanderbooks until the next line shows the line changed.
/// That is corruption, from the writer and the viewer alike.
#[test]
fn a_damaged_line_that_looks_newer_is_corrupt_when_the_next_line_checks_it() {
    type Edit = fn(&mut Vec<String>);
    let edits: [(&str, Edit); 2] = [
        ("type", |lines| {
            // account.added (seq 2) becomes a type no version writes.
            lines[1] = lines[1].replace("\"account.added\"", "\"account.addee\"");
        }),
        ("format", |lines| {
            let at = lines.iter().position(|l| l.contains(FORMAT_SET)).unwrap();
            lines[at] = lines[at].replace("\"format\":1", "\"format\":2");
        }),
    ];
    for (name, edit) in edits {
        let dir = test_dir(&format!("flipped-{name}"));
        books(&dir);
        let mut store = Store::open(&dir).unwrap();
        store.declare_journal_format(1, &[]).unwrap();
        store
            .add_account("1010", "Savings", AccountType::Asset, None, false)
            .unwrap();
        drop(store);
        edit_journal(&dir, edit);
        assert_corrupt(Store::open(&dir));
        assert_corrupt(Snapshot::load(&dir));
    }
}

/// When the line the gate refuses is the journal's last, nothing checks
/// its bytes: a damaged type and a genuinely newer one look the same. The
/// refusal says exactly that (newer or damaged; update, then verify),
/// never that the books are not damaged.
#[test]
fn an_unreadable_last_line_is_reported_as_newer_or_damaged() {
    let dir = test_dir("flipped-last");
    books(&dir);
    let seq = append(
        &dir,
        &[(
            "fx.rate_set",
            json!({"currency": "EUR", "date": "2026-10-01", "rate": "1.10"}),
        )],
    )[0];
    edit_journal(&dir, |lines| {
        let last = lines.last_mut().unwrap();
        *last = last.replace("\"fx.rate_set\"", "\"fx.rate_sex\"");
    });
    let need = "an event type this version doesn't know";
    assert_newer(Store::open(&dir), seq, "fx.rate_sex", need, false);
    assert_newer(Snapshot::load(&dir), seq, "fx.rate_sex", need, false);
}

/// A genuinely newer line with a whole, chained line after it: its bytes
/// are checked, so the books are newer and that line is intact.
#[test]
fn a_newer_line_the_next_line_chains_onto_is_newer_and_intact() {
    let dir = test_dir("newer-verified");
    books(&dir);
    let seq = append(
        &dir,
        &[
            ("entry.evidence_inherited", json!({"entry_id": 1})),
            ("entry.evidence_inherited", json!({"entry_id": 1})),
        ],
    )[0];
    let need = "an event type this version doesn't know";
    assert_newer(
        Store::open(&dir),
        seq,
        "entry.evidence_inherited",
        need,
        true,
    );
    assert_newer(
        Snapshot::load(&dir),
        seq,
        "entry.evidence_inherited",
        need,
        true,
    );
}

/// A line the gate refuses, followed by a complete line this version
/// can't even parse as JSON (or a blank one), is unverified: nothing
/// checks the refused line, so it is newer or damaged, never corrupt for
/// certain (a newer format may frame its lines its own way; rules.md
/// B-27), and never intact.
#[test]
fn a_refused_line_followed_by_a_garbled_line_is_unverified() {
    for (i, garbled) in ["{not json", "", "[1,2]", "{\"prev\":7}"]
        .iter()
        .enumerate()
    {
        let dir = test_dir(&format!("newer-then-garbled-{i}"));
        books(&dir);
        let seq = append(
            &dir,
            &[("entry.evidence_inherited", json!({"entry_id": 1}))],
        )[0];
        edit_journal(&dir, |lines| lines.push(garbled.to_string()));
        let need = "an event type this version doesn't know";
        let t = "entry.evidence_inherited";
        assert_newer(Store::open(&dir), seq, t, need, false);
        assert_newer(Snapshot::load(&dir), seq, t, need, false);
    }
}

/// The journal's last line, as raw bytes, and its hash: what the next
/// line's prev must be.
fn last_line(dir: &Path) -> (u64, String) {
    let raw = std::fs::read(dir.join(JOURNAL_FILE)).unwrap();
    let body = raw.strip_suffix(b"\n").unwrap();
    let last = body.rsplit(|&b| b == b'\n').next().unwrap();
    let v: serde_json::Value = serde_json::from_slice(last).unwrap();
    (v["seq"].as_u64().unwrap(), sha256_hex(last))
}

/// Append one line's bytes and a newline, as another writer would.
fn append_raw(dir: &Path, bytes: &[u8]) {
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(dir.join(JOURNAL_FILE))
        .unwrap();
    std::io::Write::write_all(&mut f, bytes).unwrap();
}

/// A newer format frames the lines after its declaration its own way
/// (rules.md B-27): a float, a string seq, a key of its own. This version
/// reads only the next line's prev, so it never calls such books corrupt:
/// the declaration is intact when that prev checks it, unverified when the
/// next line has no prev it can read, and corrupt only when a prev it can
/// read doesn't match. The writer and the viewer agree.
#[test]
fn a_newer_format_frames_its_lines_its_own_way() {
    type Next = fn(u64, &str) -> serde_json::Value;
    let nexts: [(&str, Next, Option<bool>); 6] = [
        (
            "float",
            |seq, prev| json!({"seq": seq, "ts": "t", "type": "period.attested", "payload": {"rate": 0.5}, "prev": prev}),
            Some(true),
        ),
        (
            "string-seq",
            |seq, prev| json!({"seq": seq.to_string(), "ts": "t", "type": "period.attested", "payload": {}, "prev": prev}),
            Some(true),
        ),
        (
            "envelope-key",
            |seq, prev| json!({"seq": seq, "ts": "t", "type": "period.attested", "payload": {}, "prev": prev, "actor": "agent"}),
            Some(true),
        ),
        (
            "no-prev",
            |seq, prev| json!({"seq": seq, "ts": "t", "type": "period.attested", "payload": {}, "parent": prev}),
            Some(false),
        ),
        (
            "prev-not-text",
            |seq, _| json!({"seq": seq, "ts": "t", "type": "period.attested", "payload": {}, "prev": 1.5}),
            Some(false),
        ),
        (
            "wrong-prev",
            |seq, _| json!({"seq": seq, "ts": "t", "type": "period.attested", "payload": {"rate": 0.5}, "prev": "00"}),
            None,
        ),
    ];
    for (name, next, verified) in nexts {
        let dir = test_dir(&format!("v2-{name}"));
        books(&dir);
        let seq = append(
            &dir,
            &[(FORMAT_SET, json!({"format": 3, "features": ["actors"]}))],
        )[0];
        let (_, hash) = last_line(&dir);
        append_raw(
            &dir,
            format!("{}\n", canonical_json(&next(seq + 1, &hash))).as_bytes(),
        );
        match verified {
            Some(v) => {
                assert_newer(Store::open(&dir), seq, FORMAT_SET, "journal format 3", v);
                assert_newer(Snapshot::load(&dir), seq, FORMAT_SET, "journal format 3", v);
            }
            None => {
                assert_corrupt(Store::open(&dir));
                assert_corrupt(Snapshot::load(&dir));
            }
        }
    }
}

/// A key beside the envelope's is never dropped (rules.md B-27): the line
/// is refused as newer, and its hash covers the key, so the next line
/// still checks it (intact), a next line hashed without the key shows it
/// was added (corrupt), and as the last line it is unverified. The writer
/// and the viewer agree, and JournalLine::parse refuses such a line.
#[test]
fn an_unknown_envelope_key_is_newer_and_checked_through() {
    for case in ["chained", "hashed-without-the-key", "last"] {
        let dir = test_dir(&format!("envelope-{case}"));
        books(&dir);
        let (seq, prev) = last_line(&dir);
        let seq = seq + 1;
        let line = json!({"seq": seq, "ts": "2026-10-16T00:00:00Z", "type": "fx.rate_set",
            "payload": {"currency": "EUR", "date": "2026-10-01", "rate": "1.10"},
            "prev": prev, "actor": "agent"});
        let raw = canonical_json(&line);
        let err = ledger_store::JournalLine::parse(raw.as_bytes()).unwrap_err();
        assert!(err.contains("actor"), "{err}");
        append_raw(&dir, format!("{raw}\n").as_bytes());
        let without = {
            let mut l = line.clone();
            l.as_object_mut().unwrap().remove("actor");
            sha256_hex(canonical_json(&l).as_bytes())
        };
        let next_prev = match case {
            "chained" => Some(sha256_hex(raw.as_bytes())),
            "hashed-without-the-key" => Some(without),
            _ => None,
        };
        if let Some(p) = &next_prev {
            let next = json!({"seq": seq + 1, "ts": "t", "type": "fx.rate_set",
                "payload": {"currency": "EUR", "date": "2026-10-02", "rate": "1.10"}, "prev": p});
            append_raw(&dir, format!("{}\n", canonical_json(&next)).as_bytes());
        }
        let need = "journal line fields this version doesn't know (actor)";
        match case {
            "hashed-without-the-key" => {
                assert_corrupt(Store::open(&dir));
                assert_corrupt(Snapshot::load(&dir));
            }
            _ => {
                let v = case == "chained";
                assert_newer(Store::open(&dir), seq, "fx.rate_set", need, v);
                assert_newer(Snapshot::load(&dir), seq, "fx.rate_set", need, v);
            }
        }
    }
}

/// A line that isn't UTF-8 is damage, for the writer as for the viewer
/// (they read the journal the same way: bytes, split on newlines), never
/// an I/O error.
#[test]
fn a_line_that_is_not_utf8_is_corrupt_for_the_viewer_and_the_writer() {
    for at in ["end", "after-a-newer-line"] {
        let dir = test_dir(&format!("not-utf8-{at}"));
        books(&dir);
        let path = dir.join(JOURNAL_FILE);
        if at == "end" {
            append_raw(&dir, b"{\"seq\":99,\"x\":\"\xff\xfe\"}\n");
            assert_corrupt(Store::open(&dir));
            assert_corrupt(Snapshot::load(&dir));
            assert_corrupt(ledger_store::journal::read_and_verify_journal(&path));
        } else {
            // After a refused line, it is a next line without a readable
            // prev: unverified, for both.
            let seq = append(&dir, &[("period.attested", json!({}))])[0];
            append_raw(&dir, b"{\"prev\":\"\xff\"}\n");
            let need = "an event type this version doesn't know";
            assert_newer(Store::open(&dir), seq, "period.attested", need, false);
            assert_newer(Snapshot::load(&dir), seq, "period.attested", need, false);
        }
    }
}

/// An older version never changes books it can't read: a newer line
/// followed by a cut-off last line (a newer version mid-append) is
/// refused as newer, and the journal is byte for byte as it was, with
/// nothing set aside. Books this version reads still get the repair.
#[test]
fn an_old_reader_never_repairs_newer_books() {
    let dir = test_dir("newer-torn");
    books(&dir);
    let seq = append(&dir, &[("period.attested", json!({"key": "company.name"}))])[0];
    append_raw(&dir, b"{\"seq\":");
    let path = dir.join(JOURNAL_FILE);
    let before = std::fs::read(&path).unwrap();
    let need = "an event type this version doesn't know";
    assert_newer(Store::open(&dir), seq, "period.attested", need, false);
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let set_aside: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().contains(".torn."))
        .collect();
    assert!(set_aside.is_empty(), "{set_aside:?}");

    // The same cut-off tail after lines this version reads is repaired,
    // and the repair is journaled.
    let dir = test_dir("known-torn");
    books(&dir);
    append_raw(&dir, b"{\"seq\":");
    let store = Store::open(&dir).unwrap();
    assert_eq!(store.tail_repairs().unwrap().len(), 1);
}

/// B-27 ("a cut-off last line is set aside only when the lines before it
/// pass"; Store::open SCENARIO 3): a complete line that chains and passes
/// the gate but can't be applied (a post of an entry that doesn't exist),
/// or a first line that isn't store.created, refuses the open with the
/// journal byte for byte as it was and nothing set aside, whatever comes
/// after it: a cut-off tail stays where it is, for a human to look at.
#[test]
fn a_bad_complete_line_before_a_torn_tail_leaves_the_journal_as_it_was() {
    let unapplied = test_dir("bad-then-torn");
    books(&unapplied);
    append(&unapplied, &[("entry.posted", json!({"entry_id": 999}))]);
    let headless = test_dir("headless-then-torn");
    append(
        &headless,
        &[(
            "account.added",
            json!({"id": 1, "code": "1000", "name": "Cash", "account_type": "Asset"}),
        )],
    );
    for dir in [&unapplied, &headless] {
        append_raw(dir, b"{\"seq\":");
        let path = dir.join(JOURNAL_FILE);
        let before = std::fs::read(&path).unwrap();
        let had_db = dir.join(ledger_store::db_file()).exists();
        assert!(Store::open(dir).is_err(), "{}", dir.display());
        assert_eq!(std::fs::read(&path).unwrap(), before, "{}", dir.display());
        let set_aside: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().contains(".torn."))
            .collect();
        assert!(set_aside.is_empty(), "{set_aside:?}");
        assert_eq!(
            dir.join(ledger_store::db_file()).exists(),
            had_db,
            "no database made for books that can't open"
        );
    }
    assert!(matches!(
        Store::open(&headless),
        Err(StoreError::JournalCorrupt(_))
    ));
}

/// The viewer applies the writer's rule for the first line (B-27: the
/// two read the same journal the same way): it must be a store.created
/// naming a currency. Any other first line, or a store.created without a
/// sound base_currency, is corrupt for both; neither guesses a currency.
#[test]
fn the_first_line_is_store_created_for_the_viewer_and_the_writer() {
    let firsts = [
        (
            "account.added",
            json!({"id": 1, "code": "1000", "name": "Cash", "account_type": "Asset"}),
        ),
        ("store.created", json!({})),
        ("store.created", json!({"base_currency": 5})),
        ("store.created", json!({"base_currency": "dollars"})),
        ("journal.tail_repaired", json!({"bytes": 1, "sha256": "00"})),
    ];
    for (n, (t, payload)) in firsts.into_iter().enumerate() {
        let dir = test_dir(&format!("first-line-{n}"));
        append(&dir, &[(t, payload.clone())]);
        assert_corrupt(Store::open(&dir));
        assert_corrupt(Snapshot::load(&dir));
    }
    // A sound one opens for both, in its own currency.
    let dir = test_dir("first-line-ok");
    append(&dir, &[("store.created", json!({"base_currency": "EUR"}))]);
    assert_eq!(Store::open(&dir).unwrap().base_currency(), Currency::EUR);
    assert_eq!(Snapshot::load(&dir).unwrap().base_currency(), Currency::EUR);
}

/// A blank complete line is damage, for the viewer as for the writer: the
/// writer never writes one, and the two read the same journal the same
/// way (S9 b, 2026-10-16).
#[test]
fn a_blank_line_is_corrupt_for_the_viewer_and_the_writer() {
    for at in ["middle", "end"] {
        let dir = test_dir(&format!("blank-{at}"));
        books(&dir);
        edit_journal(&dir, |lines| match at {
            "middle" => lines.insert(2, String::new()),
            _ => lines.push(String::new()),
        });
        assert_corrupt(Store::open(&dir));
        assert_corrupt(Snapshot::load(&dir));
    }
    // A viewer following the books meets one appended later.
    let dir = test_dir("blank-follow");
    books(&dir);
    let mut snap = Snapshot::load(&dir).unwrap();
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(dir.join(JOURNAL_FILE))
        .unwrap();
    std::io::Write::write_all(&mut f, b"\n").unwrap();
    assert_corrupt(snap.refresh());
}

/// The writer and the viewer, opening and following, say the same thing,
/// word for word, about the same damaged line (the newline that ends it
/// is no part of the line): a JSON value cut off where its line ends.
#[test]
fn the_viewer_and_the_writer_say_the_same_of_a_damaged_line() {
    for (tag, raw) in [
        ("string", &b"{\"seq\":9,\"note\":\"cut"[..]),
        ("object", &b"{\"seq\":9,"[..]),
    ] {
        let dir = test_dir(&format!("same-words-{tag}"));
        books(&dir);
        let mut snap = Snapshot::load(&dir).unwrap();
        append_raw(&dir, &[raw, b"\n"].concat());
        let writer = Store::open(&dir).err().expect("damaged books opened");
        assert!(matches!(writer, StoreError::JournalCorrupt(_)), "{writer}");
        let writer = writer.to_string();
        assert!(writer.contains("unparseable line"), "{writer}");
        let viewer = Snapshot::load(&dir).err().expect("damaged books loaded");
        assert_eq!(viewer.to_string(), writer, "{tag}: opening");
        let following = snap.refresh().expect_err("damaged books followed");
        assert_eq!(following.to_string(), writer, "{tag}: following");
    }
}

/// read_journal_and_format reads every line, in order, as the plain
/// verified read does, with the format in force.
#[test]
fn read_journal_and_format_gives_every_line_and_the_format() {
    let dir = test_dir("read-and-format");
    books(&dir);
    append(
        &dir,
        &[(
            FORMAT_SET,
            json!({"format": SUPPORTED_JOURNAL_FORMAT, "features": []}),
        )],
    );
    let path = dir.join(JOURNAL_FILE);
    let (lines, format) = ledger_store::journal::read_journal_and_format(&path).unwrap();
    let want: Vec<(u64, String)> = ledger_store::journal::read_and_verify_journal(&path)
        .unwrap()
        .into_iter()
        .map(|l| (l.seq, l.line_type))
        .collect();
    assert!(want.len() > 5, "{want:?}");
    assert_eq!(
        lines
            .into_iter()
            .map(|l| (l.seq, l.line_type))
            .collect::<Vec<_>>(),
        want
    );
    assert_eq!(
        format,
        JournalFormat {
            format: SUPPORTED_JOURNAL_FORMAT,
            features: vec![]
        }
    );
}

/// P3's journal extensions (rules.md B-27; plan D-39): each is announced
/// by a feature this version knows, `account_renamed` for an
/// `account.renamed`, `require_tax_category` for a `settings.set` naming
/// `policy.require_tax_category`. Books an older version made (books():
/// format 1, nothing declared) refuse both, journaling nothing, until the
/// owner's format upgrade declares this version's whole format at once;
/// then both are taken, under that one declaration. A reader whose
/// features don't include them (every earlier version: none) stops at the
/// declaration as newer, never at the line, and never calls it corrupt.
#[test]
fn p3_features_wait_for_the_upgrade_and_are_declared_once() {
    for f in ["account_renamed", "require_tax_category", "projects"] {
        assert!(KNOWN_FEATURES.contains(&f), "{f}");
    }
    let dir = test_dir("p3-features");
    books(&dir);
    let mut store = Store::open(&dir).unwrap();
    assert!(store.journal_format().features.is_empty());
    let policy = || ledger_core::settings::Change {
        key: "policy.require_tax_category".into(),
        value: Some(json!("refuse")),
    };
    let source = ledger_core::settings::Source {
        decided_by: "Owner".into(),
        decided_on: NaiveDate::from_ymd_opt(2026, 10, 20).unwrap(),
        citation: None,
        note: None,
    };
    let head = store.head();
    let waits = |r: Result<(), StoreError>, need: &str| match r {
        Err(StoreError::FormatUpgradePending { need: n }) => assert_eq!(n, need),
        other => panic!("expected FormatUpgradePending, got {other:?}"),
    };
    waits(
        store.rename_account(1, "Business Checking", None),
        "the journal feature account_renamed",
    );
    waits(
        store.change_settings(&[policy()], &source).map(|_| ()),
        "the journal feature require_tax_category",
    );
    let facts = ledger_core::ProjectFacts {
        kind: Some(ledger_core::ProjectKind::OnSpec),
        ..Default::default()
    };
    waits(
        store.add_project("Harbor", &facts).map(|_| ()),
        "the journal feature projects",
    );
    assert_eq!(store.head(), head, "refused whole");
    assert!(store.upgrade_format().unwrap().is_some());
    store.rename_account(1, "Business Checking", None).unwrap();
    store.rename_account(1, "Checking", None).unwrap();
    store.change_settings(&[policy()], &source).unwrap();
    store.add_project("Harbor", &facts).unwrap();
    assert_eq!(*store.journal_format(), ledger_store::format::current());
    drop(store);
    let text = std::fs::read_to_string(dir.join(JOURNAL_FILE)).unwrap();
    let lines: Vec<serde_json::Value> = text
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let kinds: Vec<(&str, serde_json::Value)> = lines
        .iter()
        .map(|l| (l["type"].as_str().unwrap(), l["payload"].clone()))
        .filter(|(t, _)| {
            matches!(
                *t,
                FORMAT_SET | "account.renamed" | "settings.set" | "project.set"
            )
        })
        .collect();
    let current = ledger_store::format::current();
    assert_eq!(kinds[0].0, FORMAT_SET);
    assert_eq!(
        kinds[0].1,
        json!({"format": current.format, "features": current.features})
    );
    assert_eq!(kinds[1].0, "account.renamed");
    assert_eq!(kinds[2].0, "account.renamed");
    assert_eq!(kinds[3].0, "settings.set");
    assert_eq!(kinds[4].0, "project.set");
    assert_eq!(kinds.len(), 5);
    // An earlier reader knows no feature (P2: KNOWN_FEATURES was empty),
    // so the gate stops it at the declaration, before the rename: the
    // gate's refusal of a feature it doesn't know is pinned by
    // an_unknown_feature_before_a_rename_is_newer_at_the_declaration.
    // This version opens them, in the writer and the viewer.
    let store = Store::open(&dir).unwrap();
    store.verify().unwrap();
    assert_eq!(store.ledger().chart().get(1).unwrap().name, "Checking");
    drop(store);
    let snap = Snapshot::load(&dir).unwrap();
    assert_eq!(snap.ledger().chart().get(1).unwrap().name, "Checking");
}

/// A feature this version doesn't know, declared before an account.renamed
/// (as a later version would), is newer at the declaration: the line after
/// it is never read.
#[test]
fn an_unknown_feature_before_a_rename_is_newer_at_the_declaration() {
    let dir = test_dir("p3-unknown-feature");
    books(&dir);
    let seqs = append(
        &dir,
        &[
            (
                FORMAT_SET,
                json!({"format": 2, "features": ["account_renamed", "later"]}),
            ),
            (
                "account.renamed",
                json!({"account_id": 1, "code": "1000", "name": "x", "previous": "Cash", "reason": null}),
            ),
        ],
    );
    let _ = std::fs::remove_file(dir.join(db_file()));
    match Store::open(&dir) {
        Err(StoreError::NewerJournal { seq, need, .. }) => {
            assert_eq!(seq, seqs[0]);
            assert_eq!(need, "journal features later");
        }
        other => panic!("expected NewerJournal, got {:?}", other.map(|_| ())),
    }
}

/// Books with a prepaid account and an expense to schedule against;
/// returns (prepaid, insurance).
fn schedule_books(dir: &Path) -> (u64, u64) {
    books(dir);
    let mut store = Store::open(dir).unwrap();
    let prepaid = store
        .add_account("1300", "Prepaid", AccountType::Asset, None, false)
        .unwrap();
    let insurance = store
        .add_account("6300", "Insurance", AccountType::Expense, None, false)
        .unwrap();
    (prepaid, insurance)
}

fn insurance(
    accounts: (u64, u64),
    start: NaiveDate,
    convention: ledger_core::ScheduleConvention,
) -> ledger_core::NewSchedule {
    ledger_core::NewSchedule {
        description: "Liability insurance".into(),
        kind: ledger_core::ScheduleKind::PrepaidExpense,
        total: Money::new(Decimal::from(1200), Currency::USD),
        start,
        months: 12,
        balance_account: accounts.0,
        pl_account: accounts.1,
        convention,
    }
}

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

/// The journal's lines, parsed.
fn lines(dir: &Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(dir.join(JOURNAL_FILE))
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

/// P3 (defect 9; rules.md B-27, D-39): a daily schedule's
/// schedule.registered, which carries convention "daily", needs journal
/// feature schedule_convention. Books an older version made (books():
/// format 1) refuse one, journaling nothing, until the owner's format
/// upgrade, whose one declaration is on the line before it. A version
/// before P3 knew no features, so it stops at that declaration
/// (NewerJournal; format.rs's an_older_reader_stops_at_p3s_features),
/// never reading the schedule as equal months. This version reads it back,
/// posts by the day, and verifies; a second daily schedule declares
/// nothing more.
#[test]
fn a_daily_schedule_waits_for_its_feature_and_an_older_reader_stops_there() {
    let dir = test_dir("daily-feature");
    let accounts = schedule_books(&dir);
    let mut store = Store::open(&dir).unwrap();
    let daily = |start| insurance(accounts, start, ledger_core::ScheduleConvention::Daily);
    let head = store.head();
    match store.register_schedule(daily(day(2026, 3, 15))) {
        Err(StoreError::FormatUpgradePending { need }) => {
            assert_eq!(need, "the journal feature schedule_convention")
        }
        other => panic!("expected FormatUpgradePending, got {other:?}"),
    }
    assert_eq!(store.head(), head, "refused whole");
    store.upgrade_format().unwrap();
    let id = store.register_schedule(daily(day(2026, 3, 15))).unwrap();
    let journal = lines(&dir);
    let n = journal.len();
    assert_eq!(journal[n - 1]["type"], "schedule.registered");
    assert_eq!(journal[n - 1]["payload"]["convention"], "daily");
    assert_eq!(journal[n - 2]["type"], FORMAT_SET);
    let current = ledger_store::format::current();
    assert_eq!(
        journal[n - 2]["payload"],
        json!({"format": current.format, "features": current.features})
    );
    store.run_schedules(day(2026, 5, 31)).unwrap();
    store
        .register_schedule(insurance(
            accounts,
            day(2026, 4, 2),
            ledger_core::ScheduleConvention::Daily,
        ))
        .unwrap();
    let declared = lines(&dir)
        .iter()
        .filter(|l| l["type"] == FORMAT_SET)
        .count();
    assert_eq!(declared, 1, "declared once");
    store.verify().unwrap();
    let posted: Vec<NaiveDate> = store
        .ledger()
        .get_schedule(id)
        .unwrap()
        .posted
        .iter()
        .copied()
        .collect();
    assert_eq!(
        posted,
        [day(2026, 3, 31), day(2026, 4, 30), day(2026, 5, 31)]
    );
    drop(store);
    // Reopened, rebuilt, and through the viewer: the same plan, posted.
    let mut store = Store::open(&dir).unwrap();
    store.rebuild().unwrap();
    store.verify().unwrap();
    let s = store.ledger().get_schedule(id).unwrap();
    assert_eq!(s.convention, ledger_core::ScheduleConvention::Daily);
    assert_eq!(s.plan().unwrap()[0].1, Decimal::new(5484, 2));
    let snap = Snapshot::load(&dir).unwrap();
    assert_eq!(
        snap.ledger().differs_from(store.ledger()),
        Vec::<&str>::new()
    );
}

/// P3: an equal-months schedule (journals before P3 hold only these)
/// declares no feature: books that never use a daily schedule stay
/// readable by versions before it. Its line names its convention.
#[test]
fn a_monthly_even_schedule_declares_no_feature() {
    let dir = test_dir("monthly-even-no-feature");
    let accounts = schedule_books(&dir);
    let mut store = Store::open(&dir).unwrap();
    store
        .register_schedule(insurance(
            accounts,
            day(2026, 3, 1),
            ledger_core::ScheduleConvention::MonthlyEven,
        ))
        .unwrap();
    let journal = lines(&dir);
    assert!(journal.iter().all(|l| l["type"] != FORMAT_SET));
    assert_eq!(
        journal.last().unwrap()["payload"]["convention"],
        "monthly_even"
    );
    assert_eq!(store.journal_format(), &JournalFormat::default());
    store.verify().unwrap();
}

/// P3: a schedule convention this version doesn't know (one a newer
/// version adds) is newer, never corrupt (rules.md B-27); one that isn't a
/// string is damage.
#[test]
fn an_unknown_schedule_convention_is_newer_not_corrupt() {
    let dir = test_dir("unknown-convention");
    let accounts = schedule_books(&dir);
    let registered = |convention: serde_json::Value| {
        json!({
            "id": 1,
            "description": "Insurance",
            "kind": "prepaid_expense",
            "total": {"amount": "1200", "currency": "USD"},
            "start": "2026-03-15",
            "months": 12,
            "balance_account": accounts.0,
            "pl_account": accounts.1,
            "convention": convention,
        })
    };
    let seq = append(
        &dir,
        &[("schedule.registered", registered(json!("weekly")))],
    )[0];
    std::fs::remove_file(dir.join(db_file())).unwrap();
    for got in [Store::open(&dir).err(), Snapshot::load(&dir).err()] {
        match got {
            Some(StoreError::NewerJournal { seq: s, need, .. }) => {
                assert_eq!(s, seq);
                assert_eq!(need, "the schedule convention weekly");
            }
            other => panic!("expected NewerJournal, got {other:?}"),
        }
    }
    let dir = test_dir("bad-convention");
    let accounts2 = schedule_books(&dir);
    assert_eq!(accounts2, accounts);
    append(&dir, &[("schedule.registered", registered(json!(7)))]);
    std::fs::remove_file(dir.join(db_file())).unwrap();
    assert_corrupt(Store::open(&dir));
}

/// P4b S4.2 (rules.md B-27): a `party.updated` naming a field this version
/// doesn't know is newer, never corrupt; a known field with a malformed
/// value is still damage.
#[test]
fn an_unknown_party_field_is_newer_not_corrupt() {
    let dir = test_dir("unknown-party-field");
    books(&dir);
    let seq = append(
        &dir,
        &[
            (
                "party.updated",
                json!({"id": 0, "change": {"field": "ssn", "value": "x"}}),
            ),
            (
                "fx.rate_set",
                json!({"currency": "EUR", "date": "2026-10-01", "rate": "1.10"}),
            ),
        ],
    )[0];
    std::fs::remove_file(dir.join(db_file())).unwrap();
    assert_newer(
        Store::open(&dir),
        seq,
        "party.updated",
        "the party field ssn",
        false,
    );
    assert_newer(
        Snapshot::load(&dir),
        seq,
        "party.updated",
        "the party field ssn",
        false,
    );

    let dir = test_dir("bad-party-field-value");
    books(&dir);
    append(
        &dir,
        &[(
            "party.updated",
            json!({"id": 0, "change": {"field": "email", "value": 5}}),
        )],
    );
    std::fs::remove_file(dir.join(db_file())).unwrap();
    assert_corrupt(Store::open(&dir));
}

/// P3 (R3; rules.md B-27): a schedule month reversed within its month is
/// posted again only in books that declare journal feature month_reposting
/// (an older version would refuse the second schedule.posted as posted
/// twice): books an older version made wait for the format upgrade, and
/// the books replay to the same ledger. A month reversed after it ended is
/// left by the run, listed with why (it would count twice until the
/// reversal), and journals nothing. Hand check: equal months of 100.00
/// from 2026-03-01; March posted, reversed on Mar 31, run again: one
/// March, 100.00; April posted, reversed on May 2: April held.
#[test]
fn a_schedule_month_posted_again_waits_for_month_reposting_and_replays() {
    let dir = test_dir("schedule-repost");
    let accounts = schedule_books(&dir);
    let mut store = Store::open(&dir).unwrap();
    let id = store
        .register_schedule(insurance(
            accounts,
            day(2026, 3, 1),
            ledger_core::ScheduleConvention::MonthlyEven,
        ))
        .unwrap();
    let mar = store.post_schedule_month(id, day(2026, 3, 31)).unwrap();
    store
        .reverse_entry(mar, day(2026, 3, 31), "posted early")
        .unwrap();
    // Books an older version made (books(): format 1) wait for the
    // upgrade before March posts again; the owner's go-ahead declares it.
    assert!(store.journal_format().features.is_empty());
    let head = store.head();
    assert!(matches!(
        store.run_schedules(day(2026, 3, 31)),
        Err(StoreError::FormatUpgradePending { need }) if need == "the journal feature month_reposting"
    ));
    assert_eq!(store.head(), head, "refused whole");
    store.upgrade_format().unwrap();
    let posted = store.run_schedules(day(2026, 3, 31)).unwrap();
    assert_eq!(posted.len(), 1);
    assert!(
        store
            .journal_format()
            .has_feature(ledger_store::format::MONTH_REPOSTING)
    );
    let apr = store.post_schedule_month(id, day(2026, 4, 30)).unwrap();
    store
        .reverse_entry(apr, day(2026, 5, 2), "wrong policy")
        .unwrap();
    let head = store.head();
    let posted = store.run_schedules(day(2026, 4, 30)).unwrap();
    assert!(posted.is_empty(), "{posted:?}");
    assert_eq!(store.head(), head);
    let held = store.schedule_months_held(day(2026, 4, 30)).unwrap();
    assert_eq!(held.len(), 1);
    assert_eq!((held[0].0, held[0].1), (id, day(2026, 4, 30)));
    assert!(held[0].2.contains("2026-05-02"), "{}", held[0].2);
    store.verify().unwrap();
    drop(store);
    let mut store = Store::open(&dir).unwrap();
    store.rebuild().unwrap();
    store.verify().unwrap();
    let s = store.ledger().get_schedule(id).unwrap();
    assert!(s.posted.contains(&day(2026, 3, 31)));
    assert!(!s.posted.contains(&day(2026, 4, 30)));
    let snap = Snapshot::load(&dir).unwrap();
    assert_eq!(
        snap.ledger().differs_from(store.ledger()),
        Vec::<&str>::new()
    );
}

/// P3 review round 2, and the format-upgrade gate (D-39): in books an
/// older version wrote, every command that needs a journal feature is
/// refused (FormatUpgradePending) before its first journaled line, so the
/// refusal journals nothing: never a depreciation entry without the
/// `asset.depreciation_posted` that makes it the period's run, nor a
/// disposal's final depreciation without its line. Each command: a period
/// run again after its run was reversed (month_reposting), a disposal whose
/// month's run was reversed (its final depreciation, month_reposting), a
/// daily schedule (schedule_convention), and a schedule month posted again
/// (month_reposting). After the owner's upgrade each journals, and the
/// books verify.
#[test]
fn a_write_waiting_for_the_upgrade_journals_nothing() {
    let dir = test_dir("feature-refused");
    let accounts = schedule_books(&dir);
    let mut store = Store::open(&dir).unwrap();
    let add = |s: &mut Store, code: &str, t| s.add_account(code, code, t, None, false).unwrap();
    let cost = add(&mut store, "1500", AccountType::Asset);
    let accum = add(&mut store, "1501", AccountType::Asset);
    let expense = add(&mut store, "6100", AccountType::Expense);
    let gain = add(&mut store, "4900", AccountType::Revenue);
    let cash = store.ledger().chart().find_by_code("1000").unwrap().id;
    let usd = |a: i64| Money::new(Decimal::new(a, 2), Currency::USD);
    let lathe = |s: &mut Store, what: &str| {
        s.register_asset(ledger_core::NewAsset {
            description: what.into(),
            in_service_date: day(2026, 1, 1),
            cost: usd(1_200_000),
            salvage_value: usd(0),
            useful_life_months: 12,
            cost_account: cost,
            accum_depr_account: accum,
            expense_account: expense,
        })
        .unwrap()
    };
    // One asset's January run, reversed within January: due again.
    let asset = lathe(&mut store, "Lathe");
    let run = store.run_depreciation(asset, "2026-01").unwrap();
    store
        .reverse_entry(run.entry_id.unwrap(), day(2026, 1, 31), "doubted")
        .unwrap();
    // Another's January to March run, March reversed within March: a
    // disposal on Mar 15 posts March's final depreciation again.
    let sold = lathe(&mut store, "Old lathe");
    for m in ["2026-01", "2026-02"] {
        store.run_depreciation(sold, m).unwrap();
    }
    let mar = store.run_depreciation(sold, "2026-03").unwrap();
    store
        .reverse_entry(mar.entry_id.unwrap(), day(2026, 3, 31), "sold")
        .unwrap();
    let monthly = store
        .register_schedule(insurance(
            accounts,
            day(2026, 1, 1),
            ledger_core::ScheduleConvention::MonthlyEven,
        ))
        .unwrap();
    let jan = store
        .post_schedule_month(monthly, day(2026, 1, 31))
        .unwrap();
    store
        .reverse_entry(jan, day(2026, 1, 31), "doubted")
        .unwrap();
    // Books as an older version made them (format 1, books()): nothing
    // declared, and everything above needs nothing new.
    assert!(store.journal_format().features.is_empty());

    let head = store.head();
    let refused = |r: Result<(), StoreError>, what: &str| match r {
        Err(StoreError::FormatUpgradePending { need }) => {
            assert!(need.starts_with("the journal feature "), "{what}: {need}")
        }
        other => panic!("{what}: {other:?}"),
    };
    refused(
        store.run_depreciation(asset, "2026-01").map(|_| ()),
        "a period run again",
    );
    refused(
        store
            .dispose_asset(sold, day(2026, 3, 15), usd(900_000), cash, gain)
            .map(|_| ()),
        "a disposal's final depreciation",
    );
    refused(
        store
            .register_schedule(insurance(
                accounts,
                day(2026, 3, 15),
                ledger_core::ScheduleConvention::Daily,
            ))
            .map(|_| ()),
        "a daily schedule",
    );
    refused(
        store
            .post_schedule_month(monthly, day(2026, 1, 31))
            .map(|_| ()),
        "a schedule month posted again",
    );
    assert_eq!(store.head(), head, "nothing journaled");
    assert_eq!(store.ledger().next_entry_id(), {
        let snap = Snapshot::load(&dir).unwrap();
        snap.ledger().next_entry_id()
    });
    store.verify().unwrap();

    assert!(store.upgrade_format().unwrap().is_some());
    assert!(
        store
            .run_depreciation(asset, "2026-01")
            .unwrap()
            .entry_id
            .is_some()
    );
    store
        .post_schedule_month(monthly, day(2026, 1, 31))
        .unwrap();
    store
        .dispose_asset(sold, day(2026, 3, 15), usd(900_000), cash, gain)
        .unwrap();
    assert_eq!(*store.journal_format(), ledger_store::format::current());
    store.verify().unwrap();
}

/// The replay fuzz target (`fuzz_store_replay`: the writer and the viewer
/// agree on every journal) draws its lines from every event type this
/// version reads, P3's included (`asset.basis_set`, `party.creditor_set`
/// were missing: the fuzzer never wrote them, so it never checked the two
/// readers agree on them).
#[test]
fn the_replay_fuzzer_writes_every_event_type() {
    let target = include_str!("../../../fuzz/fuzz_targets/fuzz_store_replay.rs");
    let list = target
        .split("const TYPES: &[&str] = &[")
        .nth(1)
        .and_then(|rest| rest.split("];").next())
        .expect("the fuzz target's TYPES");
    let missing: Vec<&str> = EVENT_TYPES
        .iter()
        .copied()
        .filter(|t| *t != "store.created" && !list.contains(&format!("\"{t}\"")))
        .collect();
    assert!(
        missing.is_empty(),
        "the replay fuzzer never writes {missing:?}"
    );
}

/// Done-when (plan P4b S5): the send event is this version's, it needs
/// `documents_v2`, and the books declare that feature before the first
/// `invoice.sent` line, so a version without it stops at the declaration
/// and reports the books as newer, never corrupt (rules.md B-27).
#[test]
fn an_older_version_stops_at_invoice_sent() {
    assert!(EVENT_TYPES.contains(&"invoice.sent"));
    let older = JournalFormat {
        format: SUPPORTED_JOURNAL_FORMAT,
        features: KNOWN_FEATURES
            .iter()
            .copied()
            .filter(|f| *f != DOCUMENTS_V2)
            .map(str::to_string)
            .collect(),
    };
    assert!(
        matches!(
            older.require_feature(DOCUMENTS_V2),
            Err(StoreError::FormatUpgradePending { .. })
        ),
        "a reader without documents_v2 waits for it"
    );
    let dir = test_dir("sent-gate");
    let mut store =
        Store::create_in_format(&dir, Currency::USD, &JournalFormat::default()).unwrap();
    let customer = store.add_customer("Fernhill Labs").unwrap();
    let date = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
    let inv = store
        .create_invoice(
            customer,
            date,
            date + chrono::Days::new(30),
            vec![DocumentLine {
                description: "Design".into(),
                amount: Decimal::new(10_000, 2),
                class_id: None,
                account: None,
                quantity: None,
                unit_price: None,
            }],
        )
        .unwrap();
    store.upgrade_format().unwrap().expect("the go-ahead");
    store
        .mark_sent(
            inv,
            Sent {
                on: date,
                by: "agent".into(),
                via: SentVia::Email,
                to: vec!["billing@fernhill.example".into()],
                kind: SendKind::Original,
                reference: None,
                note: None,
            },
        )
        .unwrap();
    let journal = std::fs::read_to_string(dir.join(JOURNAL_FILE)).unwrap();
    let mut declared = false;
    let mut sent = false;
    for line in journal.lines() {
        let v: serde_json::Value = serde_json::from_str(line).unwrap();
        if v["type"] == FORMAT_SET
            && v["payload"]["features"]
                .as_array()
                .is_some_and(|f| f.iter().any(|x| x == DOCUMENTS_V2))
        {
            declared = true;
        }
        if v["type"] == "invoice.sent" {
            assert!(declared, "documents_v2 must be declared before the send");
            sent = true;
        }
    }
    assert!(sent);
    drop(store);
    Store::open(&dir).unwrap().verify().unwrap();
}
