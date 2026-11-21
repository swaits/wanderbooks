//! Every write through the tools is one transaction that happens exactly
//! once (plan P15 2a, D-88; WB-format-8 to WB-format-12), through the real
//! server as an agent reaches it.
//!
//! The law: a repeat of a request key posts nothing and returns the first
//! answer, across a restart; the same key for another request is refused;
//! a write without a key is not run; a failed write leaves its key free
//! and the books as they were; a call is framed in the journal and the
//! import is one frame; every tool that can write asks for the key; a call
//! that wrote lines and then failed rolls back, is on the record, and
//! frees its key; a call that wrote nothing leaves no line at all; the
//! same key under two tokens is two calls; a repeat that arrives while its
//! original runs gets its answer; a crash after the closing line is on
//! disk and before SQLite committed leaves the call done, once.

use common::{Books, Wire, answer_of};
use serde_json::{Value, json};

mod common;

/// The journal's lines, parsed.
fn lines(books: &Books) -> Vec<Value> {
    String::from_utf8(books.journal_raw())
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

/// A `tools/call` with exactly these arguments, key or none.
fn call_exactly(w: &mut Wire, tool: &str, args: Value) -> Value {
    let msg = json!({"jsonrpc": "2.0", "id": 4242, "method": "tools/call",
                     "params": {"name": tool, "arguments": args}});
    w.send_exactly(&msg.to_string());
    w.recv(common::TIMEOUT).expect("an answer")
}

fn draft(key: &str, what: &str) -> Value {
    json!({"date": "2026-03-01", "description": what, "no_party_reason": "test",
           "request_key": key})
}

#[test]
fn a_repeat_of_a_key_returns_the_first_answer_and_writes_nothing() {
    let books = Books::new("calls-repeat");
    let mut w = books.session("test");
    let first = call_exactly(
        &mut w,
        "books.draft_entry",
        draft("key-repeat-0001", "Rent"),
    );
    let first = answer_of(&first).unwrap();
    let after_first = books.journal_raw();
    // The answer was lost; the agent asks again, the same call, the same key.
    let again = call_exactly(
        &mut w,
        "books.draft_entry",
        draft("key-repeat-0001", "Rent"),
    );
    assert_eq!(answer_of(&again).unwrap(), first);
    assert_eq!(books.journal_raw(), after_first, "a repeat writes nothing");
    let drafts = answer_of(&w.call("books.list_drafts", json!({}))).unwrap();
    assert_eq!(
        drafts["drafts"].as_array().map(Vec::len),
        Some(1),
        "{drafts}"
    );
    // Across a restart: the key is in the journal.
    books.restart();
    let mut w = books.session("test");
    let later = call_exactly(
        &mut w,
        "books.draft_entry",
        draft("key-repeat-0001", "Rent"),
    );
    assert_eq!(answer_of(&later).unwrap(), first);
    assert_eq!(books.journal_raw(), after_first, "nor after a restart");
}

#[test]
fn the_same_key_for_another_request_is_refused_in_words() {
    let books = Books::new("calls-misuse");
    let mut w = books.session("test");
    answer_of(&call_exactly(
        &mut w,
        "books.draft_entry",
        draft("key-misuse-0001", "Rent"),
    ))
    .unwrap();
    let before = books.journal_raw();
    let refused = call_exactly(
        &mut w,
        "books.draft_entry",
        draft("key-misuse-0001", "Something else"),
    );
    let why = answer_of(&refused).unwrap_err();
    assert!(why.contains("already used"), "{why}");
    assert!(why.contains("nothing was changed"), "{why}");
    assert_eq!(books.journal_raw(), before);
}

#[test]
fn a_write_without_a_usable_key_is_not_run() {
    let books = Books::new("calls-nokey");
    let mut w = books.session("test");
    let before = books.journal_raw();
    let mut bare = draft("x", "No key");
    bare.as_object_mut().unwrap().remove("request_key");
    for (what, args) in [
        ("no key", bare),
        ("a short key", draft("short", "No key")),
        ("a key that isn't text", {
            let mut a = draft("x", "No key");
            a["request_key"] = json!(12345678);
            a
        }),
    ] {
        let r = call_exactly(&mut w, "books.draft_entry", args);
        let why = answer_of(&r).unwrap_err();
        assert!(why.contains("request_key"), "{what}: {why}");
        assert!(why.contains("not run"), "{what}: {why}");
    }
    assert_eq!(books.journal_raw(), before, "a refusal journals nothing");
    // A read needs none, and a preview (a confirmed tool, not confirmed)
    // writes nothing and needs none.
    answer_of(&call_exactly(&mut w, "books.chart", json!({}))).unwrap();
    let r = call_exactly(&mut w, "books.post_entry", json!({"entry_id": 1}));
    let text = serde_json::to_string(&r).unwrap();
    assert!(!text.contains("request_key"), "{text}");
}

#[test]
fn a_failed_write_leaves_the_books_and_its_key_as_they_were() {
    let books = Books::new("calls-failed");
    let mut w = books.session("test");
    let before = books.journal_raw();
    let mut bad = draft("key-failed-0001", "Bad date");
    bad["date"] = json!("not a date");
    let failed = call_exactly(&mut w, "books.draft_entry", bad);
    assert!(answer_of(&failed).is_err(), "{failed}");
    assert_eq!(
        books.journal_raw(),
        before,
        "a call that wrote nothing leaves no line"
    );
    // Nothing happened under the key, so the corrected call may use it.
    let fixed = call_exactly(
        &mut w,
        "books.draft_entry",
        draft("key-failed-0001", "Good date"),
    );
    answer_of(&fixed).unwrap();
}

#[test]
fn a_call_is_one_frame_in_the_journal_closed_with_its_answer() {
    let books = Books::new("calls-frame");
    let mut w = books.session("test");
    let before = lines(&books).len();
    let out = call_exactly(
        &mut w,
        "books.draft_entry",
        draft("key-frame-00001", "Framed"),
    );
    let answer = answer_of(&out).unwrap();
    let all = lines(&books);
    let call = &all[before..];
    assert_eq!(call.first().unwrap()["type"], "call.begun", "{call:?}");
    let closed = call.last().unwrap();
    assert_eq!(closed["type"], "call.closed");
    let p = &closed["payload"];
    assert_eq!(p["outcome"], "ok");
    assert_eq!(p["tool"], "books.draft_entry");
    assert_eq!(p["key"], "key-frame-00001");
    assert_eq!(p["first_seq"], call.first().unwrap()["seq"]);
    assert_eq!(
        p["last_seq"].as_u64().unwrap() + 1,
        closed["seq"].as_u64().unwrap()
    );
    // The answer kept is the answer given.
    let kept: Value = serde_json::from_str(p["answer"].as_str().unwrap()).unwrap();
    assert_eq!(kept["structuredContent"], answer);
}

#[test]
fn the_import_is_one_transaction() {
    let books = Books::new("calls-import");
    let mut w = books.session("test");
    for (code, name, kind) in [("1000", "Bank", "asset"), ("6000", "Supplies", "expense")] {
        answer_of(&w.call(
            "books.add_account",
            json!({"code": code, "name": name, "account_type": kind}),
        ))
        .unwrap();
    }
    let csv = b"Date,Description,Amount\n2026-01-02,Paper,-12.50\n2026-01-03,Pens,-3.25\n2026-01-04,Ink,-40.00\n";
    let up = w.upload(csv, "bank.csv", "text/csv", "import");
    let before = lines(&books).len();
    let args = json!({"upload_id": up, "bank_account_code": "1000",
                      "default_offset_code": "6000", "request_key": "key-import-0001"});
    let out = answer_of(&call_exactly(&mut w, "books.import_bank_csv", args.clone())).unwrap();
    assert_eq!(out["count"], 3, "{out}");
    let all = lines(&books);
    let call = &all[before..];
    assert_eq!(call.first().unwrap()["type"], "call.begun");
    assert_eq!(call.last().unwrap()["type"], "call.closed");
    // Every row of the file is inside the one frame.
    let inside = call.iter().filter(|l| l["type"] == "entry.created").count();
    assert_eq!(inside, 3);
    assert_eq!(
        all.iter().filter(|l| l["type"] == "call.begun").count(),
        all.iter()
            .filter(|l| l["type"] == "call.closed" && l["payload"]["first_seq"].is_u64())
            .count(),
        "every frame that began closed"
    );
    // The answer lost: the repeat gives the first answer, imports nothing.
    let journal = books.journal_raw();
    let again = answer_of(&call_exactly(&mut w, "books.import_bank_csv", args)).unwrap();
    assert_eq!(again, out);
    assert_eq!(books.journal_raw(), journal);
    // A file the books refuse whole writes none of it.
    let bad = b"Date,Description,Amount\n2026-02-02,Ok row,-1.00\nnot a date,Bad row,-2.00\n";
    let up = w.upload(bad, "bad.csv", "text/csv", "import");
    let before = books.journal_raw();
    let refused = call_exactly(
        &mut w,
        "books.import_bank_csv",
        json!({"upload_id": up, "bank_account_code": "1000",
               "default_offset_code": "6000", "request_key": "key-import-0002"}),
    );
    assert!(answer_of(&refused).is_err(), "{refused}");
    assert_eq!(books.journal_raw(), before, "all or nothing");
}

#[test]
fn every_tool_that_can_write_asks_for_a_key_and_no_other_does() {
    let books = Books::new("calls-schema");
    let mut w = books.session("test");
    let listed = w.rpc("tools/list", json!({}));
    let tools = listed["result"]["tools"].as_array().unwrap();
    let (mut asking, mut reading) = (0, 0);
    for t in tools {
        let name = t["name"].as_str().unwrap();
        // Three tools only read what is sealed (and may answer with a TIN):
        // annotated as writes, they are reads and take no key.
        let read_only = t["annotations"]["readOnlyHint"] == json!(true)
            || [
                "books.vendor_1099",
                "books.get_w9",
                "books.export_cpa_package",
            ]
            .contains(&name);
        let required = t["inputSchema"]["required"]
            .as_array()
            .is_some_and(|r| r.contains(&json!("request_key")));
        let property = t["inputSchema"]["properties"].get("request_key").is_some();
        if read_only {
            assert!(!required && !property, "{name} reads and asks for a key");
            reading += 1;
        } else {
            assert!(required && property, "{name} can write and asks for no key");
            asking += 1;
        }
    }
    assert!(
        asking > 50 && reading > 20,
        "{asking} write, {reading} read"
    );
}

/// The books' SQLite projection file.
fn projection(books: &Books) -> std::path::PathBuf {
    std::fs::read_dir(books.dir())
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|e| e == "db"))
        .expect("the projection file")
}

/// A `books.add_account` request.
fn account(key: &str) -> Value {
    json!({"code": "1000", "name": "Bank", "account_type": "asset", "request_key": key})
}

/// The raw journal's line types, in order.
fn types(books: &Books) -> Vec<String> {
    lines(books)
        .iter()
        .map(|l| l["type"].as_str().unwrap().to_string())
        .collect()
}

/// A call that gets as far as writing lines and then fails: its savepoint
/// rolls back, the live books are rebuilt without its lines, a `failed`
/// closing line is on the record, its key is free, and what takes its ids
/// next is a fact of the books (plan P15 2a, SCENARIO 2).
///
/// The failure is the projection refusing the account's line: a
/// trigger put in the books' SQLite file from outside, as a full disk or a
/// corrupt page would.
#[test]
fn a_call_that_wrote_lines_and_then_failed_leaves_the_books_and_its_key_as_they_were() {
    // The fault is put there on purpose: this call fails part way.
    common::expect_failed_calls();
    let books = Books::new("calls-failed-lines");
    let mut w = books.session("test");
    let before = types(&books);
    let db = rusqlite::Connection::open(projection(&books)).unwrap();
    db.execute_batch(
        "CREATE TRIGGER wb_test_refuses BEFORE INSERT ON accounts \
         BEGIN SELECT RAISE(ABORT, 'a test refuses this write'); END;",
    )
    .unwrap();
    let failed = call_exactly(&mut w, "books.add_account", account("key-lines-fail-1"));
    assert!(answer_of(&failed).is_err(), "{failed}");
    let added: Vec<_> = types(&books)[before.len()..].to_vec();
    assert_eq!(
        added.first().map(String::as_str),
        Some("call.begun"),
        "{added:?}"
    );
    assert_eq!(added.last().map(String::as_str), Some("call.closed"));
    assert!(
        added.len() >= 3,
        "it wrote a line before failing: {added:?}"
    );
    let all = lines(&books);
    let closed = &all.last().unwrap()["payload"];
    assert_eq!(closed["outcome"], "failed");
    assert!(closed["error"].as_str().is_some_and(|e| !e.is_empty()));
    assert!(closed.get("answer").is_none());
    // The live books are as they were: no such account, though a line was
    // written.
    db.execute_batch("DROP TRIGGER wb_test_refuses").unwrap();
    drop(db);
    let banks = |w: &mut common::Wire| {
        let chart = answer_of(&w.call("books.chart", json!({}))).unwrap();
        chart.to_string().matches("\"Bank\"").count()
    };
    assert_eq!(banks(&mut w), 0);
    // The key is free: the same request now succeeds, takes the id the dead
    // call had, and is the only account.
    let fixed = answer_of(&call_exactly(
        &mut w,
        "books.add_account",
        account("key-lines-fail-1"),
    ))
    .unwrap();
    assert_eq!(
        fixed["account_id"], 1,
        "the dead call's account id is handed out again: {fixed}"
    );
    assert_eq!(banks(&mut w), 1);
    // History lists every line and says of the dead ones that they did not
    // count; none of the counted lines is a dead one.
    let history = answer_of(&w.call("books.journal_history", json!({"limit": 200}))).unwrap();
    let said: Vec<&str> = history["lines"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|l| l["summary"].as_str())
        .filter(|s| s.contains("not counted"))
        .collect();
    assert_eq!(said.len(), added.len() - 2, "{history}");
    // A restart replays the same books: the dead lines build nothing.
    books.restart();
    let mut w = books.session("test");
    assert_eq!(banks(&mut w), 1);
    answer_of(&w.call("books.verify", json!({}))).unwrap();
}

/// A call that wrote no line leaves none (WB-format-9): its answer is kept
/// for the life of the server, so a repeat within the session is answered;
/// after a restart the repeat simply runs again.
#[test]
fn a_call_that_wrote_nothing_leaves_no_line_and_is_answered_again_only_within_the_session() {
    let books = Books::new("calls-nothing");
    let mut w = books.session("test");
    let before = books.journal_raw();
    let args = json!({"request_key": "key-nothing-0001", "sha256": "ab".repeat(32),
                      "size": 5, "mime_type": "text/csv", "file_name": "x.csv",
                      "purpose": "import"});
    let first = answer_of(&call_exactly(&mut w, "books.begin_upload", args.clone())).unwrap();
    assert_eq!(books.journal_raw(), before, "no frame, no line");
    let again = answer_of(&call_exactly(&mut w, "books.begin_upload", args.clone())).unwrap();
    assert_eq!(again, first, "the same answer within the session");
    assert_eq!(books.journal_raw(), before);
    books.restart();
    let mut w = books.session("test");
    let after = answer_of(&call_exactly(&mut w, "books.begin_upload", args)).unwrap();
    assert_ne!(after, first, "after a restart it simply runs again");
    assert_eq!(books.journal_raw(), before, "and still writes nothing");
}

/// The same request key under two tokens is two calls: a key belongs to
/// the token that made it.
#[test]
fn the_same_key_under_two_tokens_is_two_calls() {
    let books = Books::new("calls-two-tokens");
    let mut one = books.session("test");
    let second = books
        .home
        .token_with(&books.id, "second", &["read", "write", "post", "admin"]);
    let mut two = common::Wire::with_token(&books.home, &books.id, Some(second));
    two.initialize("test");
    let a = answer_of(&call_exactly(
        &mut one,
        "books.draft_entry",
        draft("key-tokens-0001", "Same"),
    ))
    .unwrap();
    let b = answer_of(&call_exactly(
        &mut two,
        "books.draft_entry",
        draft("key-tokens-0001", "Same"),
    ))
    .unwrap();
    assert_ne!(a["entry_id"], b["entry_id"], "one draft per token: {a} {b}");
    let drafts = answer_of(&one.call("books.list_drafts", json!({}))).unwrap();
    assert_eq!(
        drafts["drafts"].as_array().map(Vec::len),
        Some(2),
        "{drafts}"
    );
    // And each token's repeat finds its own.
    let a2 = answer_of(&call_exactly(
        &mut one,
        "books.draft_entry",
        draft("key-tokens-0001", "Same"),
    ))
    .unwrap();
    assert_eq!(a2, a);
}

/// A repeat that arrives while its original is still running waits for it
/// (writes take turns) and then gets its answer: the original's effects
/// are made once. Eight identical requests sent at the same moment make
/// one draft and are answered alike.
#[test]
fn a_repeat_arriving_while_its_original_runs_gets_its_answer_and_changes_nothing() {
    let books = Books::new("calls-concurrent");
    let before = lines(&books).len();
    let barrier = std::sync::Barrier::new(8);
    let (home, id) = (&books.home, books.id.clone());
    let answers: Vec<Value> = std::thread::scope(|scope| {
        let runs: Vec<_> = (0..8)
            .map(|n| {
                let (barrier, id) = (&barrier, &id);
                scope.spawn(move || {
                    let mut w = common::Wire::new(home, id);
                    w.initialize(&format!("racer-{n}"));
                    barrier.wait();
                    call_exactly(
                        &mut w,
                        "books.draft_entry",
                        draft("key-concurrent-1", "Raced"),
                    )
                })
            })
            .collect();
        runs.into_iter().map(|r| r.join().unwrap()).collect()
    });
    let answers: Vec<Value> = answers.iter().map(|a| answer_of(a).unwrap()).collect();
    assert!(answers.windows(2).all(|p| p[0] == p[1]), "{answers:?}");
    let all = lines(&books);
    assert_eq!(
        all[before..]
            .iter()
            .filter(|l| l["type"] == "entry.created")
            .count(),
        1,
        "one draft"
    );
    assert_eq!(
        all[before..]
            .iter()
            .filter(|l| l["type"] == "call.begun")
            .count(),
        1,
        "one frame"
    );
}

/// A call whose answer is too long for the books to keep (above 4 KiB):
/// its closing line keeps the answer's hash and length, and a repeat, even
/// one that arrived while the original ran, is told in words that it was
/// done, with the lines it wrote; nothing is done twice.
#[test]
fn a_repeat_of_a_call_whose_answer_was_too_long_is_told_it_was_done() {
    let books = Books::new("calls-long-answer");
    let mut w = books.session("test");
    for (code, name, kind) in [("1000", "Bank", "asset"), ("6000", "Supplies", "expense")] {
        answer_of(&w.call(
            "books.add_account",
            json!({"code": code, "name": name, "account_type": kind}),
        ))
        .unwrap();
    }
    let rows = 300;
    let mut csv = String::from("Date,Description,Amount\n");
    for n in 0..rows {
        csv.push_str(&format!("2026-01-02,Row {n},-1.{:02}\n", n % 100));
    }
    let up = w.upload(csv.as_bytes(), "bank.csv", "text/csv", "import");
    let args = json!({"upload_id": up, "bank_account_code": "1000",
                      "default_offset_code": "6000", "request_key": "key-long-answer1"});
    let before = lines(&books).len();
    let (home, id) = (&books.home, books.id.clone());
    let (first, second) = std::thread::scope(|scope| {
        let original = scope.spawn(|| {
            let mut w = common::Wire::new(home, &id);
            w.initialize("original");
            call_exactly(&mut w, "books.import_bank_csv", args.clone())
        });
        std::thread::sleep(std::time::Duration::from_millis(30));
        let repeat = scope.spawn(|| {
            let mut w = common::Wire::new(home, &id);
            w.initialize("repeat");
            call_exactly(&mut w, "books.import_bank_csv", args.clone())
        });
        (original.join().unwrap(), repeat.join().unwrap())
    });
    // One of the two ran; the other was told, in words, that it was done.
    let (ran, told) = match (answer_of(&first), answer_of(&second)) {
        (Ok(a), Err(why)) | (Err(why), Ok(a)) => (a, why),
        other => panic!("one runs and one is told: {other:?}"),
    };
    assert_eq!(ran["count"], rows, "{ran}");
    assert!(told.contains("was done"), "{told}");
    assert!(told.contains("nothing more was changed"), "{told}");
    assert!(told.contains("journal lines"), "{told}");
    let all = lines(&books);
    assert_eq!(
        all[before..]
            .iter()
            .filter(|l| l["type"] == "entry.created")
            .count(),
        rows,
        "the file was imported once"
    );
    let closed = all.last().unwrap();
    assert_eq!(closed["type"], "call.closed");
    assert!(
        closed["payload"].get("answer").is_none(),
        "no answer kept whole"
    );
    assert_eq!(
        closed["payload"]["answer_sha256"].as_str().map(str::len),
        Some(64)
    );
    assert!(closed["payload"]["answer_bytes"].as_u64().unwrap() > 4096);
    // Across a restart the same words.
    books.restart();
    let mut w = books.session("test");
    let again = call_exactly(&mut w, "books.import_bank_csv", args);
    assert!(answer_of(&again).unwrap_err().contains("was done"));
    assert_eq!(lines(&books).len(), all.len());
}

/// A crash after the closing line is on disk and before SQLite committed:
/// the books are rebuilt from the journal, the call is done, and a repeat
/// of its key answers the first answer and changes nothing.
#[test]
fn a_crash_with_the_ok_close_on_disk_but_sqlite_not_committed_leaves_the_call_done_once() {
    let books = Books::new("calls-sqlite-behind");
    let mut w = books.session("test");
    answer_of(&call_exactly(
        &mut w,
        "books.draft_entry",
        draft("key-crash-prior1", "Earlier"),
    ))
    .unwrap();
    drop(w);
    // The projection as it was before the call.
    books.stop();
    let db = projection(&books);
    let old = std::fs::read(&db).unwrap();
    let side: Vec<_> = ["-wal", "-shm"]
        .iter()
        .map(|x| std::path::PathBuf::from(format!("{}{x}", db.display())))
        .collect();
    assert!(
        side.iter().all(|p| !p.exists()),
        "a stopped server leaves no -wal or -shm"
    );
    books.start();
    let mut w = books.session("test");
    let first = answer_of(&call_exactly(
        &mut w,
        "books.draft_entry",
        draft("key-crash-0001", "Done once"),
    ))
    .unwrap();
    drop(w);
    let journal = books.journal_raw();
    assert_eq!(
        lines(&books).last().unwrap()["payload"]["outcome"],
        "ok",
        "the ok closing line is on disk"
    );
    // Power fails: the journal has it (it was fsynced first), SQLite does not.
    books.stop();
    std::fs::write(&db, old).unwrap();
    books.start();
    let mut w = books.session("test");
    // The books are rebuilt from the journal: both drafts are there.
    let drafts = answer_of(&w.call("books.list_drafts", json!({}))).unwrap();
    assert_eq!(
        drafts["drafts"].as_array().map(Vec::len),
        Some(2),
        "{drafts}"
    );
    // The repeat is answered with the first answer and does nothing.
    let again = answer_of(&call_exactly(
        &mut w,
        "books.draft_entry",
        draft("key-crash-0001", "Done once"),
    ))
    .unwrap();
    assert_eq!(again, first);
    assert_eq!(books.journal_raw(), journal, "nothing more was written");
    let drafts = answer_of(&w.call("books.list_drafts", json!({}))).unwrap();
    assert_eq!(
        drafts["drafts"].as_array().map(Vec::len),
        Some(2),
        "{drafts}"
    );
    answer_of(&w.call("books.verify", json!({}))).unwrap();
}

// ---- The tax-id vault is inside the transaction (BS-partner-2). Obviously
// fake values: a TIN is never written in a journal line, an error, or here
// beyond these two.

const OLD_TIN: &str = "123-45-6789";
const NEW_TIN: &str = "765-43-2109";

fn digits(tin: &str) -> String {
    tin.replace('-', "")
}

/// Whether `text` holds either fake TIN, hyphenated or not.
fn holds_a_tin(text: &str) -> bool {
    [OLD_TIN, NEW_TIN]
        .iter()
        .any(|t| text.contains(t) || text.contains(&digits(t)))
}

/// What the vault holds for the party, by the vault's own reader: the
/// TIN's last four, or None.
fn vaulted_last4(books: &Books, party: u64) -> Option<String> {
    let book = books.dir();
    let key = book.join("keys").join("legacy.key");
    let vault = ledger_vault::Vault::open(&book.join("vault"), &key, false).ok()?;
    vault.tin(party).unwrap().map(|t| t.last4().to_string())
}

fn staged_files(books: &Books) -> Vec<String> {
    std::fs::read_dir(books.dir().join("vault"))
        .map(|d| {
            d.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n.contains(".staged-") || n.contains(".purging-"))
                .collect()
        })
        .unwrap_or_default()
}

fn set_tin(key: &str, tin: &str) -> Value {
    json!({"party": "Mo", "tin": tin, "tin_type": "ssn", "request_key": key})
}

/// A set_tin that fails after it wrote the vault: the vault keeps the old
/// tax id whole and holds nothing of the new one, the journal holds
/// neither, and once the call can run again it is the new one that is kept.
#[test]
fn a_set_tin_that_fails_leaves_the_vault_as_it_was() {
    // The fault is put there on purpose: this call fails part way.
    common::expect_failed_calls();
    let books = Books::new("calls-vault-fail");
    let mut w = books.session("test");
    let party = answer_of(&w.call(
        "books.add_party",
        json!({"name": "Mo", "roles": ["member"]}),
    ))
    .unwrap()["party_id"]
        .as_u64()
        .unwrap();
    answer_of(&call_exactly(
        &mut w,
        "books.set_tin",
        set_tin("key-vault-first1", OLD_TIN),
    ))
    .unwrap();
    assert_eq!(vaulted_last4(&books, party).as_deref(), Some("6789"));
    // The projection refuses the call's first line (its begin line).
    let db = rusqlite::Connection::open(projection(&books)).unwrap();
    db.execute_batch(
        "CREATE TRIGGER wb_test_refuses BEFORE UPDATE ON meta \
         BEGIN SELECT RAISE(ABORT, 'a test refuses this write'); END;",
    )
    .unwrap();
    let failed = call_exactly(
        &mut w,
        "books.set_tin",
        set_tin("key-vault-fail01", NEW_TIN),
    );
    let why = answer_of(&failed).unwrap_err();
    assert!(!holds_a_tin(&why), "an error never holds a tax id");
    db.execute_batch("DROP TRIGGER wb_test_refuses").unwrap();
    drop(db);
    // The failure here is one that persists (the projection refuses every
    // write, as a full disk would), so the books server stops: the call's
    // failed closing line is on disk, and what it staged is settled by what
    // the journal says at the next start.
    books.restart();
    let mut w = books.session("test");
    let log = books.server_log();
    assert!(log.contains("vault records unsettled"), "{log}");
    assert!(!holds_a_tin(&log), "a log never holds a tax id");
    // The journal's failed frame holds no tax id, and the old one is whole in
    // the vault; nothing is left staged.
    assert_eq!(
        lines(&books)[lines(&books).len() - 1]["payload"]["outcome"],
        "failed"
    );
    assert!(!holds_a_tin(
        &String::from_utf8(books.journal_raw()).unwrap()
    ));
    assert_eq!(vaulted_last4(&books, party).as_deref(), Some("6789"));
    assert_eq!(staged_files(&books), Vec::<String>::new());
    // The party still shows the old last four.
    let shown = answer_of(&w.call("books.party", json!({"party": "Mo"}))).unwrap();
    assert!(shown.to_string().contains("6789"), "{shown}");
    assert!(!shown.to_string().contains("2109"), "{shown}");
    // The key is free: the same request now succeeds and the vault holds the new one.
    answer_of(&call_exactly(
        &mut w,
        "books.set_tin",
        set_tin("key-vault-fail01", NEW_TIN),
    ))
    .unwrap();
    assert_eq!(vaulted_last4(&books, party).as_deref(), Some("2109"));
    assert_eq!(staged_files(&books), Vec::<String>::new());
    assert!(!holds_a_tin(
        &String::from_utf8(books.journal_raw()).unwrap()
    ));
}

/// Two requests that differ only in a tax id are two requests: the same key
/// with another tax id is refused, never answered as a repeat, and nothing
/// of either is in the journal to test a guess against.
#[test]
fn a_key_reused_for_another_tax_id_is_refused_not_repeated() {
    let books = Books::new("calls-vault-mask");
    let mut w = books.session("test");
    answer_of(&w.call(
        "books.add_party",
        json!({"name": "Mo", "roles": ["member"]}),
    ))
    .unwrap();
    let first = answer_of(&call_exactly(
        &mut w,
        "books.set_tin",
        set_tin("key-mask-000001", OLD_TIN),
    ))
    .unwrap();
    // The same request again: the first answer, nothing more written.
    let journal = books.journal_raw();
    let again = answer_of(&call_exactly(
        &mut w,
        "books.set_tin",
        set_tin("key-mask-000001", OLD_TIN),
    ))
    .unwrap();
    assert_eq!(again, first);
    // Another tax id under the same key: refused in words.
    let other = call_exactly(&mut w, "books.set_tin", set_tin("key-mask-000001", NEW_TIN));
    let why = answer_of(&other).unwrap_err();
    assert!(why.contains("already used"), "{why}");
    assert!(!holds_a_tin(&why));
    assert_eq!(books.journal_raw(), journal);
    assert!(!holds_a_tin(&String::from_utf8(journal).unwrap()));
    // And across a restart, with the digest keyed by the books' own secret.
    books.restart();
    let mut w = books.session("test");
    let other = call_exactly(&mut w, "books.set_tin", set_tin("key-mask-000001", NEW_TIN));
    assert!(answer_of(&other).unwrap_err().contains("already used"));
    let same = answer_of(&call_exactly(
        &mut w,
        "books.set_tin",
        set_tin("key-mask-000001", OLD_TIN),
    ))
    .unwrap();
    assert_eq!(same, first);
}

/// A crash after the closing line is on disk and before the staged vault
/// record was made final: the next start finishes what the journal says
/// was done (the new tax id is the party's), and drops what no committed
/// call staged.
#[test]
fn a_crash_between_the_commit_and_the_vault_settling_is_finished_at_the_next_start() {
    let books = Books::new("calls-vault-crash");
    let mut w = books.session("test");
    let party = answer_of(&w.call(
        "books.add_party",
        json!({"name": "Mo", "roles": ["member"]}),
    ))
    .unwrap()["party_id"]
        .as_u64()
        .unwrap();
    answer_of(&call_exactly(
        &mut w,
        "books.set_tin",
        set_tin("key-vcrash-first", OLD_TIN),
    ))
    .unwrap();
    drop(w);
    books.stop();
    let record = books.dir().join("vault").join(format!("tin-{party}.bin"));
    let old = std::fs::read(&record).unwrap();
    books.start();
    let mut w = books.session("test");
    answer_of(&call_exactly(
        &mut w,
        "books.set_tin",
        set_tin("key-vcrash-second", NEW_TIN),
    ))
    .unwrap();
    drop(w);
    let closed = lines(&books)
        .into_iter()
        .rev()
        .find(|l| l["type"] == "call.closed")
        .unwrap();
    assert_eq!(closed["payload"]["outcome"], "ok");
    let tag = ledger_store::calls::call_tag(
        closed["payload"]["token"].as_str().unwrap(),
        closed["payload"]["key"].as_str().unwrap(),
    );
    books.stop();
    let new = std::fs::read(&record).unwrap();
    assert_ne!(old, new);
    // As a crash would have left it: the closing line is on disk, the new
    // record is still staged, the old one still final. And a record staged
    // by a call the journal never closed.
    std::fs::write(&record, &old).unwrap();
    let staged = record.with_file_name(format!("tin-{party}.staged-{tag}"));
    std::fs::write(&staged, &new).unwrap();
    let stray = record.with_file_name(format!("tin-99.staged-{}", "0123456789abcdef"));
    std::fs::write(&stray, &new).unwrap();
    books.start();
    assert_eq!(
        vaulted_last4(&books, party).as_deref(),
        Some("2109"),
        "finished"
    );
    assert!(
        !staged.exists() && !stray.exists(),
        "{:?}",
        staged_files(&books)
    );
    assert!(books.server_log().contains("vault records unsettled"));
}
