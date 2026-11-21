//! Every write through the tools is one transaction that happens exactly
//! once (plan P15 2a, D-88; WB-format-8 to WB-format-12), through the real
//! server as an agent reaches it.
//!
//! The law: a repeat of a request key posts nothing and returns the first
//! answer, across a restart; the same key for another request is refused;
//! a write without a key is not run; a failed write leaves its key free
//! and the books as they were; a call is framed in the journal and the
//! import is one frame; every tool that can write asks for the key.

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
