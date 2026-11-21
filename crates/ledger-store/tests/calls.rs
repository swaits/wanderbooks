//! Every write a tool makes is one transaction that happens once (plan
//! P15 2a, D-88, D-89; WB-format-8 to WB-format-12): a call's lines are
//! framed by `call.begun` and `call.closed`, count only when the closing
//! line says `ok`, and a call that failed or never closed builds nothing.
//!
//! The tests encode the law: a crash at any point of a call leaves the
//! whole call or none of it, in the ledger and in the projection; a
//! failed call is on the record and builds nothing; a replay of any
//! prefix that ends inside a call equals the replay of the prefix before
//! it; a repeated key finds the first answer; books that have not
//! declared the feature take no call and still read.

use chrono::NaiveDate;
use ledger_core::{AccountType, Currency, Ledger, Money, Side};
use ledger_store::calls::{
    ANSWER_CAP, CallEnd, CallId, CallOutcome, Recorded, Replay, SESSION_KEYS_MAX, Standing,
    begun_of, closed_of,
};
use ledger_store::{JOURNAL_FILE, JournalFormat, Snapshot, Store, StoreError};
use rust_decimal::Decimal;
use serde_json::Value;
use std::path::Path;

fn d(y: i32, m: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, day).unwrap()
}

fn money(units: i64) -> Money {
    Money::new(Decimal::from(units), Currency::USD)
}

fn id(key: &str) -> CallId {
    CallId {
        token: "token-1".into(),
        tool: "books.draft_entry".into(),
        key: key.into(),
        args: "a".repeat(64),
    }
}

/// What the store kept of a call, if anything.
fn kept(s: &Store, token: &str, key: &str) -> Option<Recorded> {
    s.recorded_call(token, key).unwrap()
}

fn answer_of(r: &Recorded) -> &str {
    match &r.replay {
        Replay::Answer(a) => a,
        other => panic!("kept no whole answer: {other:?}"),
    }
}

fn lines_of(dir: &Path) -> Vec<ledger_store::JournalLine> {
    ledger_store::journal::read_and_verify_journal(&dir.join(JOURNAL_FILE)).unwrap()
}

fn types_of(dir: &Path) -> Vec<String> {
    lines_of(dir).into_iter().map(|l| l.line_type).collect()
}

/// Books with a chart, in this version's format, and the number of lines
/// their journal holds.
fn books(dir: &Path) -> Store {
    let mut s = Store::create(dir, Currency::USD).unwrap();
    s.add_account("1000", "Bank", AccountType::Asset, None, false)
        .unwrap();
    s.add_account("3000", "Capital", AccountType::Equity, None, false)
        .unwrap();
    s
}

/// One step of a call: a draft with two lines.
fn draft(s: &mut Store, what: &str) {
    let e = s.create_entry(d(2026, 1, 5), what).unwrap();
    let bank = s.ledger().chart().find_by_code("1000").unwrap().id;
    let capital = s.ledger().chart().find_by_code("3000").unwrap().id;
    s.add_line(e, bank, Side::Debit, money(10)).unwrap();
    s.add_line(e, capital, Side::Credit, money(10)).unwrap();
}

fn same_books(a: &Ledger, b: &Ledger) {
    assert_eq!(a.differs_from(b), Vec::<&str>::new());
}

#[test]
fn a_call_is_framed_and_its_answer_kept_by_key() {
    let tmp = tempfile::tempdir().unwrap();
    let mut s = books(tmp.path());
    let before = lines_of(tmp.path()).len();
    s.begin_call(id("key-aaaa-1")).unwrap();
    draft(&mut s, "Rent");
    s.close_call(CallEnd::Done(r#"{"entry_id":1}"#)).unwrap();
    let lines = lines_of(tmp.path());
    // begun, entry.created, two lines, closed.
    assert_eq!(lines.len(), before + 5);
    assert_eq!(lines[before].line_type, "call.begun");
    assert_eq!(begun_of(&lines[before]).unwrap().key, "key-aaaa-1");
    let closed = closed_of(lines.last().unwrap()).unwrap();
    assert_eq!(closed.outcome, CallOutcome::Ok);
    assert_eq!(closed.first_seq, Some(lines[before].seq));
    assert_eq!(closed.last_seq, Some(lines.last().unwrap().seq - 1));
    assert_eq!(closed.answer.as_deref(), Some(r#"{"entry_id":1}"#));
    let was = kept(&s, "token-1", "key-aaaa-1").unwrap();
    assert_eq!(answer_of(&was), r#"{"entry_id":1}"#);
    assert_eq!(was.args, "a".repeat(64));
    // Scoped to the token: another token has not used this key.
    assert!(kept(&s, "token-2", "key-aaaa-1").is_none());
    s.verify().unwrap();
    // A restart rebuilds the table from the journal alone.
    drop(s);
    let s = Store::open(tmp.path()).unwrap();
    assert_eq!(
        answer_of(&kept(&s, "token-1", "key-aaaa-1").unwrap()),
        r#"{"entry_id":1}"#
    );
    s.verify().unwrap();
}

#[test]
fn a_call_that_wrote_nothing_leaves_no_line_and_is_remembered_for_the_session() {
    let tmp = tempfile::tempdir().unwrap();
    let mut s = books(tmp.path());
    let before = lines_of(tmp.path()).len();
    s.begin_call(id("key-aaaa-2")).unwrap();
    s.close_call(CallEnd::Done(r#"{"ok":true}"#)).unwrap();
    // No line at all: a call that wrote nothing has nothing to frame.
    assert_eq!(lines_of(tmp.path()).len(), before);
    // But a repeat within the session is answered as the first was.
    assert_eq!(
        answer_of(&kept(&s, "token-1", "key-aaaa-2").unwrap()),
        r#"{"ok":true}"#
    );
    // A failed call that wrote nothing leaves no line either, and frees its key.
    s.begin_call(id("key-aaaa-3")).unwrap();
    s.close_call(CallEnd::Failed("refused")).unwrap();
    assert_eq!(lines_of(tmp.path()).len(), before);
    assert!(kept(&s, "token-1", "key-aaaa-3").is_none());
    // After a restart the key is forgotten: the repeat runs again, which is
    // harmless because the first wrote nothing.
    drop(s);
    let s = Store::open(tmp.path()).unwrap();
    assert!(kept(&s, "token-1", "key-aaaa-2").is_none());
    assert_eq!(lines_of(tmp.path()).len(), before);
}

#[test]
fn the_sessions_key_table_is_bounded_in_count_and_in_bytes() {
    let tmp = tempfile::tempdir().unwrap();
    let mut s = books(tmp.path());
    for n in 0..SESSION_KEYS_MAX + 20 {
        s.begin_call(id(&format!("key-bound-{n:05}"))).unwrap();
        s.close_call(CallEnd::Done(r#"{"ok":true}"#)).unwrap();
    }
    let (_, (held, bytes)) = s.call_table();
    assert_eq!(held, SESSION_KEYS_MAX);
    assert!(bytes <= SESSION_KEYS_MAX * 16);
    // The oldest went first.
    assert!(kept(&s, "token-1", "key-bound-00000").is_none());
    assert!(
        kept(
            &s,
            "token-1",
            &format!("key-bound-{:05}", SESSION_KEYS_MAX + 19)
        )
        .is_some()
    );
    // An answer above the cap is not kept for a call that wrote nothing.
    s.begin_call(id("key-big-0001")).unwrap();
    s.close_call(CallEnd::Done(&"x".repeat(ANSWER_CAP + 1)))
        .unwrap();
    assert!(kept(&s, "token-1", "key-big-0001").is_none());
}

#[test]
fn a_long_answer_is_kept_as_its_hash_and_a_repeat_is_told_in_words_what_was_done() {
    let tmp = tempfile::tempdir().unwrap();
    let mut s = books(tmp.path());
    let long = format!(r#"{{"text":"{}"}}"#, "y".repeat(ANSWER_CAP));
    s.begin_call(id("key-long-0001")).unwrap();
    draft(&mut s, "Big answer");
    s.close_call(CallEnd::Done(&long)).unwrap();
    let lines = lines_of(tmp.path());
    let last = lines.last().unwrap();
    // The line is bounded: no answer text, its hash and length instead.
    assert!(last.payload.get("answer").is_none());
    let closed = closed_of(last).unwrap();
    assert_eq!(closed.answer_bytes, Some(long.len() as u64));
    assert_eq!(closed.answer_sha256.as_deref().map(str::len), Some(64));
    assert!(serde_json::to_string(&last.payload).unwrap().len() < ANSWER_CAP);
    let expect = |s: &Store| match kept(s, "token-1", "key-long-0001").unwrap().replay {
        Replay::Summary {
            first_seq,
            last_seq,
            closed_seq,
            answer_sha256,
            answer_bytes,
        } => {
            assert_eq!(first_seq, closed.first_seq.unwrap());
            assert_eq!(last_seq, closed.last_seq.unwrap());
            assert_eq!(closed_seq, last.seq);
            assert_eq!(Some(answer_sha256), closed.answer_sha256.clone());
            assert_eq!(answer_bytes, long.len() as u64);
        }
        other => panic!("expected a summary: {other:?}"),
    };
    expect(&s);
    // After a restart the same.
    drop(s);
    let s = Store::open(tmp.path()).unwrap();
    expect(&s);
    s.verify().unwrap();
    // A whole answer at the cap is kept whole.
    let mut s = s;
    let at_cap = "z".repeat(ANSWER_CAP);
    s.begin_call(id("key-long-0002")).unwrap();
    draft(&mut s, "At the cap");
    s.close_call(CallEnd::Done(&at_cap)).unwrap();
    assert_eq!(
        answer_of(&kept(&s, "token-1", "key-long-0002").unwrap()),
        at_cap
    );
}

#[test]
fn the_committed_journal_leaves_out_the_lines_of_a_call_that_failed() {
    let tmp = tempfile::tempdir().unwrap();
    let mut s = books(tmp.path());
    // A failed call that wrote a draft: it took entry id 1 ...
    s.begin_call(id("key-fail-0001")).unwrap();
    draft(&mut s, "Dead");
    s.close_call(CallEnd::Failed("it failed")).unwrap();
    // ... which a later call hands out again.
    s.begin_call(id("key-fail-0002")).unwrap();
    draft(&mut s, "Alive");
    s.close_call(CallEnd::Done("{}")).unwrap();
    drop(s);
    let lines = lines_of(tmp.path());
    let standing = ledger_store::calls::standings(&lines).unwrap();
    let of = |ty: &str, n: usize| {
        lines
            .iter()
            .zip(&standing)
            .filter(|(l, _)| l.line_type == ty)
            .nth(n)
            .map(|(_, s)| *s)
            .unwrap()
    };
    assert_eq!(of("entry.created", 0), Standing::Failed);
    assert_eq!(of("entry.created", 1), Standing::Counts);
    assert_eq!(of("call.begun", 0), Standing::Frame);
    let committed = ledger_store::calls::committed(&lines).unwrap();
    // Both calls reused the same entry id; only the second's lines count.
    let created: Vec<_> = committed
        .iter()
        .filter(|l| l.line_type == "entry.created")
        .map(|l| l.payload["description"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(created, ["Alive"]);
    assert!(committed.iter().all(|l| !l.line_type.starts_with("call.")));
    // The history lists every line, and says of the dead ones that they did not count.
    let h = Store::open(tmp.path())
        .unwrap()
        .journal_history(&ledger_store::describe::HistoryFilter {
            limit: 200,
            ..Default::default()
        })
        .unwrap();
    let dead: Vec<_> = h
        .lines
        .iter()
        .filter(|d| d.standing == Standing::Failed)
        .collect();
    assert_eq!(dead.len(), 3);
    assert!(dead.iter().all(|d| d.summary.contains("not counted")));
    assert!(
        h.lines
            .iter()
            .filter(|d| d.standing == Standing::Counts)
            .all(|d| !d.summary.contains("not counted"))
    );
    // A snapshot reads the same.
    let snap = Snapshot::load(tmp.path()).unwrap();
    assert_eq!(snap.committed_journal().unwrap().len(), committed.len());
    assert!(snap.journal().len() > committed.len());
}

#[test]
fn a_failed_call_is_on_the_record_and_builds_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let mut s = books(tmp.path());
    let committed = s.ledger().clone();
    s.begin_call(id("key-aaaa-4")).unwrap();
    draft(&mut s, "Half");
    // What the call can see while it runs.
    assert_ne!(s.ledger().differs_from(&committed), Vec::<&str>::new());
    s.close_call(CallEnd::Failed("it went wrong")).unwrap();
    // Live: the lines did not count, in memory or in the projection.
    same_books(s.ledger(), &committed);
    s.verify().unwrap();
    // On the record: the lines, then a closing line that says failed.
    let lines = lines_of(tmp.path());
    let closed = closed_of(lines.last().unwrap()).unwrap();
    assert_eq!(closed.outcome, CallOutcome::Failed);
    assert_eq!(closed.error.as_deref(), Some("it went wrong"));
    assert!(lines.iter().any(|l| l.line_type == "entry.created"));
    // A failed key is free again: nothing happened under it.
    assert!(kept(&s, "token-1", "key-aaaa-4").is_none());
    // Books go on: the next call's ids do not collide with the failed one's.
    s.begin_call(id("key-aaaa-5")).unwrap();
    draft(&mut s, "Whole");
    s.close_call(CallEnd::Done("{}")).unwrap();
    s.verify().unwrap();
    // A restart builds the same books.
    let live = s.ledger().clone();
    drop(s);
    let s = Store::open(tmp.path()).unwrap();
    same_books(s.ledger(), &live);
    s.verify().unwrap();
}

type Step = Box<dyn Fn(&mut Store)>;

/// A call of every kind of step, by how far it got.
fn steps() -> Vec<Step> {
    vec![
        Box::new(|s| {
            s.create_entry(d(2026, 1, 5), "One").unwrap();
        }),
        Box::new(|s| {
            let bank = s.ledger().chart().find_by_code("1000").unwrap().id;
            s.add_line(1, bank, Side::Debit, money(10)).unwrap();
        }),
        Box::new(|s| {
            let capital = s.ledger().chart().find_by_code("3000").unwrap().id;
            s.add_line(1, capital, Side::Credit, money(10)).unwrap();
        }),
        Box::new(|s| {
            s.add_account("4000", "Sales", AccountType::Revenue, None, false)
                .unwrap();
        }),
    ]
}

#[test]
fn a_crash_at_each_point_of_a_call_leaves_the_whole_call_or_none_of_it() {
    let n = steps().len();
    for k in 0..=n {
        let tmp = tempfile::tempdir().unwrap();
        let mut s = books(tmp.path());
        let committed = s.ledger().clone();
        let seq_before = s.head().0;
        s.begin_call(id("key-crash-01")).unwrap();
        for step in steps().iter().take(k) {
            step(&mut s);
        }
        // The process dies here: no closing line, no clean shutdown.
        drop(s);
        let reopened = Store::open(tmp.path()).unwrap();
        same_books(reopened.ledger(), &committed);
        reopened.verify().unwrap();
        assert!(kept(&reopened, "token-1", "key-crash-01").is_none());
        let types = types_of(tmp.path());
        if k == 0 {
            // It had written nothing: there is nothing to close.
            assert_eq!(reopened.head().0, seq_before);
        } else {
            // The record says what became of its lines.
            let last = lines_of(tmp.path()).pop().unwrap();
            let closed = closed_of(&last).unwrap();
            assert_eq!(closed.outcome, CallOutcome::Unfinished, "{types:?}");
            assert_eq!(closed.first_seq, Some(seq_before + 1));
        }
        // And the books take the next call as if nothing had happened.
        let mut s = reopened;
        s.begin_call(id("key-crash-02")).unwrap();
        for step in steps().iter().take(1) {
            step(&mut s);
        }
        s.close_call(CallEnd::Done("{}")).unwrap();
        s.verify().unwrap();
        let live = s.ledger().clone();
        drop(s);
        let s = Store::open(tmp.path()).unwrap();
        same_books(s.ledger(), &live);
    }
    // And a call that closed is whole, however the process ends after.
    let tmp = tempfile::tempdir().unwrap();
    let mut s = books(tmp.path());
    s.begin_call(id("key-crash-03")).unwrap();
    for step in steps().iter() {
        step(&mut s);
    }
    s.close_call(CallEnd::Done("{}")).unwrap();
    let live = s.ledger().clone();
    drop(s);
    let s = Store::open(tmp.path()).unwrap();
    same_books(s.ledger(), &live);
    assert!(s.ledger().chart().find_by_code("4000").is_some());
}

#[test]
fn a_replay_of_any_prefix_inside_a_call_is_the_replay_of_the_prefix_before_it() {
    let tmp = tempfile::tempdir().unwrap();
    let mut s = books(tmp.path());
    s.begin_call(id("key-prefix-1")).unwrap();
    for step in steps().iter() {
        step(&mut s);
    }
    s.close_call(CallEnd::Done("{}")).unwrap();
    s.begin_call(id("key-prefix-2")).unwrap();
    draft(&mut s, "Second");
    s.close_call(CallEnd::Failed("no")).unwrap();
    s.begin_call(id("key-prefix-3")).unwrap();
    draft(&mut s, "Third");
    s.close_call(CallEnd::Done("{}")).unwrap();
    drop(s);
    let raw = std::fs::read(tmp.path().join(JOURNAL_FILE)).unwrap();
    let lines = lines_of(tmp.path());
    let mut ends = Vec::new();
    let mut at = 0usize;
    for line in raw.split_inclusive(|&b| b == b'\n') {
        at += line.len();
        ends.push(at);
    }
    // The books as the viewer reads the journal cut after line `i`.
    let work = tempfile::tempdir().unwrap();
    let books_after = |i: usize| {
        std::fs::write(work.path().join(JOURNAL_FILE), &raw[..ends[i]]).unwrap();
        Snapshot::load(work.path()).unwrap().ledger().clone()
    };
    let mut begun: Option<usize> = None;
    let mut inside = 0;
    for (i, l) in lines.iter().enumerate() {
        match l.line_type.as_str() {
            "call.begun" => begun = Some(i),
            "call.closed" => {
                let first = begun.take().expect("a close follows its begin");
                let after = books_after(i);
                let before = books_after(first);
                let closed = closed_of(l).unwrap();
                if closed.outcome == CallOutcome::Ok {
                    // Committed: the call's lines count, all at once.
                    assert_ne!(after.differs_from(&before), Vec::<&str>::new());
                } else {
                    // Failed: on the record, and not one of its lines counts.
                    same_books(&after, &before);
                }
            }
            _ => {
                if let Some(first) = begun {
                    // Any prefix that ends inside a call.
                    inside += 1;
                    same_books(&books_after(i), &books_after(first));
                }
            }
        }
    }
    assert!(inside >= 8, "the cuts inside a call were tried: {inside}");
}

#[test]
fn books_that_have_not_declared_the_feature_take_no_call_and_still_read() {
    let tmp = tempfile::tempdir().unwrap();
    {
        // Books as an older version made them: format 1, no feature.
        let mut s =
            Store::create_in_format(tmp.path(), Currency::USD, &JournalFormat::default()).unwrap();
        s.add_account("1000", "Bank", AccountType::Asset, None, false)
            .unwrap();
        s.add_account("3000", "Capital", AccountType::Equity, None, false)
            .unwrap();
    }
    let mut s = Store::open(tmp.path()).unwrap();
    let before = s.head();
    assert!(!s.takes_calls());
    assert!(s.books_id().is_none());
    let refused = s.begin_call(id("key-old-0001")).unwrap_err();
    assert!(matches!(refused, StoreError::FormatUpgradePending { .. }));
    let words = refused.to_string();
    assert!(words.contains("nothing can be written"), "{words}");
    // Nothing was written, and the books read as ever.
    assert_eq!(s.head(), before);
    assert!(s.ledger().chart().find_by_code("1000").is_some());
    s.verify().unwrap();
    // The owner's go-ahead: the format upgrade. After it, calls are taken,
    // and the books have the id the upgrade gave them.
    s.upgrade_format().unwrap().expect("waited for an upgrade");
    assert!(s.takes_calls());
    let given = s.books_id().expect("the upgrade gives an id").to_string();
    assert_eq!(given.len(), 32);
    s.begin_call(id("key-old-0002")).unwrap();
    draft(&mut s, "After");
    s.close_call(CallEnd::Done("{}")).unwrap();
    drop(s);
    // The id is in the journal, so it is the same after a restart and for
    // a viewer.
    let s = Store::open(tmp.path()).unwrap();
    assert_eq!(s.books_id(), Some(given.as_str()));
    assert_eq!(Snapshot::load(tmp.path()).unwrap().books_id(), Some(given));
}

#[test]
fn new_books_are_born_with_an_id_and_no_two_share_one() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let sa = Store::create(a.path(), Currency::USD).unwrap();
    let sb = Store::create(b.path(), Currency::USD).unwrap();
    let (ia, ib) = (
        sa.books_id().unwrap().to_string(),
        sb.books_id().unwrap().to_string(),
    );
    assert_ne!(ia, ib);
    assert_eq!(
        lines_of(a.path())[0].payload["books_id"].as_str(),
        Some(ia.as_str())
    );
    // Upgrading books that have one gives them no second.
    drop(sa);
    let mut sa = Store::open(a.path()).unwrap();
    assert!(sa.upgrade_format().unwrap().is_none());
    assert_eq!(sa.books_id(), Some(ia.as_str()));
}

#[test]
fn a_viewer_applies_a_call_only_when_it_closes() {
    let tmp = tempfile::tempdir().unwrap();
    let mut s = books(tmp.path());
    let mut snap = Snapshot::load(tmp.path()).unwrap();
    let committed = snap.ledger().clone();
    s.begin_call(id("key-view-01")).unwrap();
    draft(&mut s, "Viewed");
    // Lines are on disk, the call is not closed: the viewer holds none.
    assert!(snap.refresh().unwrap() > 0);
    same_books(snap.ledger(), &committed);
    s.close_call(CallEnd::Done("{}")).unwrap();
    snap.refresh().unwrap();
    assert_ne!(snap.ledger().differs_from(&committed), Vec::<&str>::new());
    same_books(snap.ledger(), s.ledger());
}

#[test]
fn a_frame_that_does_not_close_what_it_follows_is_corrupt() {
    let tmp = tempfile::tempdir().unwrap();
    let mut s = books(tmp.path());
    s.begin_call(id("key-bad-0001")).unwrap();
    draft(&mut s, "Framed");
    s.close_call(CallEnd::Done("{}")).unwrap();
    drop(s);
    // A closing line whose range is not the frame's: rewritten with a
    // valid chain, so only the frame rule can refuse it.
    let mut lines = lines_of(tmp.path());
    let last = lines.len() - 1;
    lines[last].payload["last_seq"] = Value::from(1);
    let mut prev = lines[last - 1].clone();
    let _ = &mut prev;
    let bad = tempfile::tempdir().unwrap();
    let mut w = ledger_store::JournalWriter::open(&bad.path().join(JOURNAL_FILE)).unwrap();
    for l in &lines {
        w.append(&l.line_type, l.payload.clone()).unwrap();
    }
    let err = Store::open(bad.path()).err().expect("refused");
    assert!(
        matches!(err, StoreError::JournalCorrupt(_)),
        "{err} should be corrupt"
    );
}

#[test]
fn a_call_cannot_begin_inside_another_or_after_a_failed_close() {
    let tmp = tempfile::tempdir().unwrap();
    let mut s = books(tmp.path());
    s.begin_call(id("key-twice-01")).unwrap();
    assert!(matches!(
        s.begin_call(id("key-twice-02")),
        Err(StoreError::CallRefused(_))
    ));
    s.close_call(CallEnd::Failed("x")).unwrap();
    assert!(matches!(
        s.close_call(CallEnd::Done("{}")),
        Err(StoreError::CallRefused(_))
    ));
}

#[test]
fn a_call_that_rebuilds_the_projection_still_commits() {
    // books.verify heals a drifted projection by rebuilding it, and is a
    // call like any other: its transaction moves to the new database.
    let tmp = tempfile::tempdir().unwrap();
    let mut s = books(tmp.path());
    s.begin_call(id("key-heal-0001")).unwrap();
    s.rebuild().unwrap();
    s.close_call(CallEnd::Done("{}")).unwrap();
    s.verify().unwrap();
    assert!(kept(&s, "token-1", "key-heal-0001").is_some());
    drop(s);
    let s = Store::open(tmp.path()).unwrap();
    s.verify().unwrap();
}

/// The closing line is on disk but the projection can't be brought up to
/// it: the call committed, and is not reported as if it hadn't. The store
/// says so in words, takes no more writes, and the next open rebuilds the
/// projection from the journal with the call's effects in it (SCENARIO 4).
#[test]
fn a_closing_line_on_disk_but_sqlite_failing_is_a_call_that_counts() {
    let tmp = tempfile::tempdir().unwrap();
    let mut s = books(tmp.path());
    // From the closing line on, the projection refuses every write: the
    // call writes a begin line, a draft and two lines (four), then closes.
    let closing_seq = lines_of(tmp.path()).last().unwrap().seq + 5;
    let db_file = tmp.path().join(s.database_file());
    let db = rusqlite::Connection::open(&db_file).unwrap();
    db.execute_batch(&format!(
        "CREATE TRIGGER wb_test_refuses BEFORE UPDATE ON meta \
         WHEN NEW.key = 'last_applied_seq' AND CAST(NEW.value AS INTEGER) >= {closing_seq} \
         BEGIN SELECT RAISE(ABORT, 'a test refuses this write'); END;"
    ))
    .unwrap();
    drop(db);
    s.begin_call(id("key-commit-0001")).unwrap();
    draft(&mut s, "Counts");
    assert_eq!(lines_of(tmp.path()).last().unwrap().seq + 1, closing_seq);
    let err = s
        .close_call(CallEnd::Done(r#"{"entry_id":1}"#))
        .unwrap_err();
    let StoreError::CallCommitted(why) = &err else {
        panic!("expected CallCommitted, got {err:?}");
    };
    assert!(why.contains("counts"), "{why}");
    assert!(!why.contains("none of it counts"), "{why}");
    // The record says ok, and the key is kept: a repeat is the first answer.
    let last = lines_of(tmp.path()).into_iter().last().unwrap();
    assert_eq!(closed_of(&last).unwrap().outcome, CallOutcome::Ok);
    assert_eq!(
        answer_of(&kept(&s, "token-1", "key-commit-0001").unwrap()),
        r#"{"entry_id":1}"#
    );
    // No more writes until the books are opened again.
    assert!(matches!(
        s.begin_call(id("key-commit-0002")),
        Err(StoreError::CallRefused(_))
    ));
    drop(s);
    rusqlite::Connection::open(&db_file)
        .unwrap()
        .execute_batch("DROP TRIGGER wb_test_refuses")
        .unwrap();
    let s = Store::open(tmp.path()).unwrap();
    assert_eq!(
        s.ledger().get_entry(1).map(|e| e.description()),
        Some("Counts")
    );
    assert_eq!(
        answer_of(&kept(&s, "token-1", "key-commit-0001").unwrap()),
        r#"{"entry_id":1}"#
    );
    s.verify().unwrap();
}

/// The projection refuses a call's begin line itself: the line is on disk,
/// so the frame is open and the call must close it (`failed`). Left open,
/// the next call would begin inside it and the journal would be corrupt.
#[test]
fn a_call_whose_begin_line_the_projection_refuses_still_closes_its_frame() {
    let tmp = tempfile::tempdir().unwrap();
    let mut s = books(tmp.path());
    let begin_seq = lines_of(tmp.path()).last().unwrap().seq + 1;
    let db_file = tmp.path().join(s.database_file());
    let db = rusqlite::Connection::open(&db_file).unwrap();
    db.execute_batch(&format!(
        "CREATE TRIGGER wb_test_refuses BEFORE UPDATE ON meta \
         WHEN NEW.key = 'last_applied_seq' AND CAST(NEW.value AS INTEGER) = {begin_seq} \
         BEGIN SELECT RAISE(ABORT, 'a test refuses this write'); END;"
    ))
    .unwrap();
    drop(db);
    s.begin_call(id("key-begin-0001")).unwrap();
    assert!(s.create_entry(d(2026, 1, 5), "Never").is_err());
    s.close_call(CallEnd::Failed("the projection refused it"))
        .ok();
    drop(s);
    rusqlite::Connection::open(&db_file)
        .unwrap()
        .execute_batch("DROP TRIGGER wb_test_refuses")
        .unwrap();
    let all = lines_of(tmp.path());
    assert_eq!(
        all.last().unwrap().line_type,
        "call.closed",
        "{:?}",
        types_of(tmp.path())
    );
    assert_eq!(
        closed_of(all.last().unwrap()).unwrap().outcome,
        CallOutcome::Failed
    );
    // The books open, and the next call begins on a closed frame.
    let mut s = Store::open(tmp.path()).unwrap();
    s.begin_call(id("key-begin-0002")).unwrap();
    draft(&mut s, "Fine");
    s.close_call(CallEnd::Done("{}")).unwrap();
    drop(s);
    Store::open(tmp.path()).unwrap().verify().unwrap();
}
