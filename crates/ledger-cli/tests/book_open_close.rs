//! Opening and closing one book while the books server runs, and who is
//! connected to each book (plan/p15-app-and-relay.md 3.1, D-77, D-78).
//! Oracle: ledger-mcp local.rs INVARIANTS 3 and 10 to 12, admin.rs
//! INVARIANTS 8 and 9, sessions.rs INVARIANTS 1 and 3.
//!
//! The rule the hammer proves: every call either ran completely and is in
//! the book's journal, or was refused and nothing of it is; after a
//! close is answered nothing more runs on that book until it is opened
//! again; and a close of one book never refuses or holds up a call on
//! another.

use common::{Home, Server, Wire};
use ledger_mcp::admin::{CloseOutcome, OpenOutcome, Request, Response};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

mod common;

const A: &str = "acme";
const B: &str = "beta";

/// What an agent is told of a book that exists but is closed.
fn closed_words(id: &str) -> String {
    format!("The books \"{id}\" aren't open: open them in Wanderbooks.")
}

/// New books `id` written in `home`'s root by the store itself, before a
/// server opens them.
fn written(home: &Home, id: &str) {
    let dir = home.book_dir(id);
    drop(ledger_store::Store::create(&dir, ledger_core::Currency::USD).unwrap());
    common::private_folder(&dir);
}

/// What the server's list says of `id`: (open, closed, problem).
fn listed(home: &Home, id: &str) -> (bool, bool, Option<String>) {
    match home.admin(Request::List) {
        Response::Books(list) => {
            let b = list
                .into_iter()
                .find(|b| b.id == id)
                .unwrap_or_else(|| panic!("{id} isn't listed"));
            (b.open, b.closed, b.problem)
        }
        other => panic!("{other:?}"),
    }
}

fn open_book(home: &Home, id: &str) -> OpenOutcome {
    match home.admin(Request::OpenBook { id: id.into() }) {
        Response::OpenBook { outcome, .. } => outcome,
        other => panic!("{other:?}"),
    }
}

fn close_book(home: &Home, id: &str) -> CloseOutcome {
    match home.admin(Request::CloseBook { id: id.into() }) {
        Response::CloseBook { outcome, .. } => outcome,
        other => panic!("{other:?}"),
    }
}

/// What the book's files hold now: the journal's lines (read and chain
/// checked) and the access log's head. A closed book's must not move.
fn files_of(home: &Home, id: &str) -> (Vec<ledger_store::JournalLine>, u64) {
    let dir = home.book_dir(id);
    let (lines, _) =
        ledger_store::journal::read_journal_and_format(&dir.join(ledger_store::JOURNAL_FILE))
            .unwrap();
    let (_, head) = ledger_store::access::read(&dir).unwrap();
    (lines, head.seq)
}

/// A request the server refuses before it runs, as a closed book's calls
/// are: its status, and its words.
fn refusal_of(wire: &Wire) -> (u16, String) {
    let r = wire.refused.clone().expect("a refusal");
    assert!(r.not_run, "refused unrun: {r:?}");
    (r.status, r.body)
}

/// Every route to a closed book says so, in words an agent can relay,
/// and a book that doesn't exist says that instead.
fn closed_on_every_route(home: &Home, id: &str) {
    let mut wire = Wire::new(home, id);
    wire.send(
        &json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": common::PROTOCOL, "capabilities": {},
        "clientInfo": {"name": "t", "version": "0"}}}),
    );
    assert!(wire.recv(common::TIMEOUT).is_none());
    assert_eq!(refusal_of(&wire), (503, closed_words(id)));
    for path in ["a/1", "x/e1"] {
        let (status, body) = wire.get(path);
        assert_eq!(
            (status, String::from_utf8(body).unwrap()),
            (503, closed_words(id)),
            "GET {path}"
        );
    }
    assert_eq!(
        wire.put("u/u1", b"bytes"),
        (503, closed_words(id)),
        "PUT an upload"
    );
}

/// The books a server was told to open at the start are open; the others
/// are closed, are listed so, and say so on every route; a name that is no
/// book is told apart; opened on request, a book serves.
#[test]
fn books_left_closed_at_the_start_say_so_and_open_when_asked() {
    let home = Home::new("open-close-start");
    written(&home, A);
    written(&home, B);
    let server = Server::start_with(&home, &["--open", B]);
    assert_eq!(listed(&home, A), (false, true, None));
    assert_eq!(listed(&home, B), (true, false, None));
    closed_on_every_route(&home, A);
    // Told apart from books that don't exist.
    let mut nobody = Wire::with_token(&home, "nobody", Some(home.token(A)));
    nobody.send(&json!({"jsonrpc": "2.0", "id": 1, "method": "ping"}));
    assert_eq!(nobody.status, 404);
    assert!(
        nobody
            .refused
            .as_ref()
            .unwrap()
            .body
            .contains("no books called"),
        "{:?}",
        nobody.refused
    );
    // The other book serves.
    let mut beta = Wire::new(&home, B);
    beta.initialize("t");
    assert_eq!(
        beta.call("books.chart", json!({}))["result"]["isError"],
        json!(false)
    );
    // Opened on request: once, then already; a name that is no book.
    assert_eq!(open_book(&home, A), OpenOutcome::Opened);
    assert_eq!(open_book(&home, A), OpenOutcome::AlreadyOpen);
    assert_eq!(open_book(&home, "nobody"), OpenOutcome::NoSuchBook);
    assert_eq!(listed(&home, A), (true, false, None));
    let mut acme = Wire::new(&home, A);
    acme.initialize("t");
    assert_eq!(
        acme.call("books.chart", json!({}))["result"]["isError"],
        json!(false)
    );
    assert!(server.close().success());
}

/// `--open none` leaves every book closed; an id that names no book stops
/// the server before it opens anything.
#[test]
fn the_open_option_takes_none_and_refuses_a_book_that_doesnt_exist() {
    let home = Home::new("open-close-none");
    written(&home, A);
    let server = Server::start_with(&home, &["--open", "none"]);
    assert_eq!(listed(&home, A), (false, true, None));
    closed_on_every_route(&home, A);
    assert!(server.close().success());
    let mut bad = std::process::Command::new(env!("CARGO_BIN_EXE_wanderbooks"));
    bad.args(["server", "--root"])
        .arg(home.root())
        .args(["--open", "nobody"])
        .stdin(std::process::Stdio::null());
    common::clean_env(&mut bad, home.dir.path());
    let out = bad.output().unwrap();
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("don't exist"),
        "{out:?}"
    );
}

/// What the sessions request says: each connection's token NAME, its
/// client, what the agent said of itself, its last call; never a token;
/// a session that ended isn't listed; and a book's sessions end with it.
#[test]
fn the_sessions_request_names_each_connection_and_never_its_token() {
    let home = Home::new("open-close-sessions");
    let server = Server::start(&home);
    home.create(A, "USD");
    home.create(B, "USD");
    let (one, two) = (
        home.token_with(A, "alpha", &["read", "write"]),
        home.token_with(A, "beta", &["read"]),
    );
    let mut first = Wire::with_token(&home, A, Some(one.clone()));
    first.initialize("claude-code");
    let started = first.call(
        "books.start",
        json!({"agent": {"model": "m-1", "reasoning_effort": "high", "harness": "h-1"}}),
    );
    assert_eq!(started["result"]["isError"], json!(false), "{started}");
    let mut second = Wire::with_token(&home, A, Some(two.clone()));
    second.initialize("chatgpt");
    let sessions = |home: &Home| match home.admin(Request::Sessions) {
        Response::Sessions(all) => all,
        other => panic!("{other:?}"),
    };
    let all = sessions(&home);
    let text = serde_json::to_string(&all).unwrap();
    for token in [&one, &two] {
        assert!(!text.contains(token.as_str()), "a token's text was listed");
        assert!(!text.contains(&ledger_store::journal::sha256_hex(token.as_bytes())));
    }
    let a = all.iter().find(|b| b.book == A).unwrap();
    assert!(
        all.iter()
            .find(|b| b.book == B)
            .unwrap()
            .sessions
            .is_empty()
    );
    assert_eq!(a.sessions.len(), 2, "{a:?}");
    let alpha = a.sessions.iter().find(|s| s.token == "alpha").unwrap();
    assert_eq!(alpha.client_name.as_deref(), Some("claude-code"));
    assert_eq!(alpha.client_version.as_deref(), Some("0.1.0"));
    let said = alpha.agent.as_ref().expect("the agent's report");
    assert_eq!(said.model.as_deref(), Some("m-1"));
    assert_eq!(said.harness.as_deref(), Some("h-1"));
    assert_eq!(alpha.running, 0);
    let last = alpha.last.as_ref().expect("a last call");
    assert_eq!(
        (last.tool.as_str(), last.outcome.as_str()),
        ("books.start", "ok")
    );
    let beta = a.sessions.iter().find(|s| s.token == "beta").unwrap();
    assert_eq!(beta.client_name.as_deref(), Some("chatgpt"));
    assert!(beta.last.is_none() && beta.agent.is_none(), "{beta:?}");

    // Ended: not listed.
    second.end();
    let a = sessions(&home).into_iter().find(|b| b.book == A).unwrap();
    assert_eq!(a.sessions.len(), 1);
    // Closed with the book: its session is gone; the closed book lists none.
    assert_eq!(close_book(&home, A), CloseOutcome::Closed);
    assert!(sessions(&home).iter().all(|b| b.book != A));
    first.send(&json!({"jsonrpc": "2.0", "id": 9, "method": "tools/list"}));
    assert!(first.recv(common::TIMEOUT).is_none());
    assert_eq!(refusal_of(&first), (503, closed_words(A)));
    assert_eq!(open_book(&home, A), OpenOutcome::Opened);
    assert!(
        sessions(&home)
            .iter()
            .find(|b| b.book == A)
            .unwrap()
            .sessions
            .is_empty()
    );
    assert!(server.close().success());
}

/// How one call came out.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Said {
    /// It ran: the answer says so (with the draft's id, for a draft).
    Ran(Option<u64>),
    /// It was refused, unrun.
    Refused,
}

/// One call's tag (the draft's description) and what came of it.
#[derive(Debug, Clone)]
struct Done {
    book: &'static str,
    tag: String,
    said: Said,
}

/// Send `tools/call` and read what came of it: it ran, or it was refused
/// unrun (at the book's door, as closing, or as a session the reopened
/// book never knew). Anything else is neither, and fails the test.
fn once(wire: &mut Wire, book: &str, id: u64, tool: &str, args: Value) -> Said {
    wire.send(&json!({"jsonrpc": "2.0", "id": id, "method": "tools/call",
        "params": {"name": tool, "arguments": args}}));
    match wire.recv(common::TIMEOUT) {
        Some(resp) => {
            if let Some(code) = resp.pointer("/error/code").and_then(Value::as_i64) {
                assert_eq!(code, i64::from(ledger_mcp::NOT_RUN_CLOSING_CODE), "{resp}");
                return Said::Refused;
            }
            assert_eq!(resp["result"]["isError"], json!(false), "{resp}");
            Said::Ran(resp["result"]["structuredContent"]["entry_id"].as_u64())
        }
        None => {
            let r = wire.refused.clone().expect("an answer or a refusal");
            let closed = r.status == 503 && r.not_run && r.body == closed_words(book);
            let unknown_session = r.status == 404 && r.body.contains("Session not found");
            assert!(closed || unknown_session, "neither run nor refused: {r:?}");
            Said::Refused
        }
    }
}

/// An agent: keeps a session with `book`, draft entries (journaled
/// writes) and chart reads, until told to stop; a refused call ends its
/// session, and it opens another when it can.
fn agent(
    home: Arc<Home>,
    book: &'static str,
    n: usize,
    stop: Arc<AtomicBool>,
    done: Arc<Mutex<Vec<Done>>>,
    reads: Arc<[AtomicUsize; 2]>,
) {
    let token = home.token(book);
    let mut wire: Option<Wire> = None;
    let mut k = 0u64;
    while !stop.load(Ordering::SeqCst) {
        if wire.is_none() {
            let mut w = Wire::with_token(&home, book, Some(token.clone()));
            w.send(
                &json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
                "protocolVersion": common::PROTOCOL, "capabilities": {},
                "clientInfo": {"name": format!("agent-{n}"), "version": "1"}}}),
            );
            match w.recv(common::TIMEOUT) {
                Some(r) if r.get("result").is_some() => {
                    w.send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
                    wire = Some(w);
                }
                _ => {
                    let r = w.refused.clone().expect("a refusal");
                    assert!(r.not_run && r.status == 503, "{r:?}");
                    std::thread::sleep(Duration::from_millis(3));
                    continue;
                }
            }
        }
        let w = wire.as_mut().unwrap();
        k += 1;
        let said = if k.is_multiple_of(3) {
            let said = once(w, book, k + 10, "books.chart", json!({}));
            let which = usize::from(said == Said::Refused);
            if book == B {
                reads[which].fetch_add(1, Ordering::SeqCst);
            }
            said
        } else {
            let tag = format!("{book}-agent{n}-call{k}");
            let said = once(
                w,
                book,
                k + 10,
                "books.draft_entry",
                json!({"date": "2026-03-01", "description": tag}),
            );
            done.lock().unwrap().push(Done {
                book,
                tag,
                said: said.clone(),
            });
            said
        };
        if said == Said::Refused {
            wire = None;
        }
    }
}

/// Many agents call two books (journaled drafts and reads) while one book
/// is closed and opened again, over and over. Afterwards both journals are
/// replayed and checked: every acknowledged draft is in its book's journal
/// exactly once, no refused call left anything, a closed book's journal
/// and access log never moved between its close answer and its reopening,
/// and no call on the other book was ever refused.
#[test]
fn closing_and_opening_a_book_over_and_over_loses_and_duplicates_nothing() {
    const CYCLES: usize = 12;
    let home = Arc::new(Home::new("open-close-hammer"));
    // Every timer of the server runs every few milliseconds, and the
    // hammered book has a backup target, which backs it up and drills it
    // as it is written: the writers a close must stop are all at work.
    let server = Server::start_with(
        &home,
        &[
            "--timers",
            "10",
            "--backup-quiet",
            "30",
            "--drill-first",
            "0",
            "--answer-grace",
            "1500",
        ],
    );
    home.create(A, "USD");
    home.create(B, "USD");
    let disk = home.dir.path().join("disk");
    std::fs::create_dir_all(&disk).unwrap();
    {
        let mut w = Wire::new(&home, A);
        w.initialize("owner");
        set_backup_folder(&mut w, &disk);
        w.end();
    }
    let stop = Arc::new(AtomicBool::new(false));
    let done = Arc::new(Mutex::new(Vec::new()));
    let reads = Arc::new([AtomicUsize::new(0), AtomicUsize::new(0)]);
    let mut agents = Vec::new();
    for (book, how_many) in [(A, 6), (B, 4)] {
        for n in 0..how_many {
            let (home, stop, done, reads) =
                (home.clone(), stop.clone(), done.clone(), reads.clone());
            agents.push(std::thread::spawn(move || {
                agent(home, book, n, stop, done, reads)
            }));
        }
    }
    // Refusals at the door of the hammered book, whose records are written
    // as the close comes (the late writers the review found).
    let refusers: Vec<_> = (0..2)
        .map(|n| {
            let (home, stop) = (home.clone(), stop.clone());
            std::thread::spawn(move || {
                let mut wire = Wire::with_token(&home, A, Some(format!("no-such-token-{n}")));
                while !stop.load(Ordering::SeqCst) {
                    wire.send(&json!({"jsonrpc": "2.0", "id": 1, "method": "ping"}));
                }
            })
        })
        .collect();
    let mut closed_for = Vec::new();
    for cycle in 0..CYCLES {
        std::thread::sleep(Duration::from_millis(150 + (cycle as u64 * 37) % 100));
        let beta_before = done.lock().unwrap().iter().filter(|d| d.book == B).count();
        assert_eq!(close_book(&home, A), CloseOutcome::Closed, "cycle {cycle}");
        // Answered closed: from here nothing runs on the book until it is
        // opened, so its files stand still (and, the writer lock being
        // released, another program could open it).
        let before = files_of(&home, A);
        // Every file of the book (the journal, the access log, every ops
        // log, stored files, upload parts, the backup's repository) as it
        // is when the close is answered.
        let tree = tree_of(&home.book_dir(A));
        assert_eq!(listed(&home, A), (false, true, None));
        closed_on_every_route(&home, A);
        // The other book answers while this one is closed: waited for as a
        // condition (while it is closed), not guessed at with a sleep.
        wait_for(
            &format!("a call on {B} finished while {A} was closed (cycle {cycle})"),
            Duration::from_secs(120),
            || done.lock().unwrap().iter().filter(|d| d.book == B).count() > beta_before,
        );
        let after = files_of(&home, A);
        assert_eq!(
            (before.0.len(), before.1),
            (after.0.len(), after.1),
            "cycle {cycle}: the closed book's journal or access log moved"
        );
        // Still, to the byte, just before it is opened again, however the
        // timers and the late refusals tried (a window to catch them in).
        stands_still(
            &home,
            A,
            Duration::from_millis(70),
            &format!("cycle {cycle}"),
        );
        assert_eq!(
            moved(&tree, &tree_of(&home.book_dir(A))),
            Vec::<String>::new(),
            "cycle {cycle}: the closed book's folder moved after the close was answered"
        );
        closed_for.push(before.0.len());
        assert_eq!(open_book(&home, A), OpenOutcome::Opened, "cycle {cycle}");
    }
    std::thread::sleep(Duration::from_millis(200));
    stop.store(true, Ordering::SeqCst);
    for a in agents.into_iter().chain(refusers) {
        a.join().unwrap();
    }
    assert!(server.close().success());
    // Every log is a whole chain after all those closings.
    access_of(&home, A);
    ledger_store::chain::read(
        &home.book_dir(A).join("ops").join("backup.jsonl"),
        "wanderbooks-backup-v1",
    )
    .expect("the backup record is a whole chain");

    let done = done.lock().unwrap().clone();
    for book in [A, B] {
        let (lines, _) = files_of(&home, book);
        let mut journaled: BTreeMap<String, Vec<u64>> = BTreeMap::new();
        for l in lines.iter().filter(|l| l.line_type == "entry.created") {
            journaled
                .entry(l.payload["description"].as_str().unwrap().to_string())
                .or_default()
                .push(l.payload["id"].as_u64().unwrap());
        }
        let ran: BTreeMap<&str, Option<u64>> = done
            .iter()
            .filter(|d| d.book == book)
            .filter_map(|d| match &d.said {
                Said::Ran(id) => Some((d.tag.as_str(), *id)),
                Said::Refused => None,
            })
            .collect();
        let refused: BTreeSet<&str> = done
            .iter()
            .filter(|d| d.book == book && d.said == Said::Refused)
            .map(|d| d.tag.as_str())
            .collect();
        // Every acknowledged write is there exactly once, under the id it
        // was acknowledged with.
        for (tag, id) in &ran {
            assert_eq!(
                journaled.get(*tag),
                Some(&vec![id.expect("a draft's id")]),
                "{book}: {tag} was acknowledged"
            );
        }
        // A refused call left nothing; nothing is in the journal that
        // wasn't acknowledged.
        for tag in &refused {
            assert!(!journaled.contains_key(*tag), "{book}: {tag} was refused");
        }
        let in_journal: BTreeSet<&str> = journaled.keys().map(String::as_str).collect();
        assert_eq!(
            in_journal,
            ran.keys().copied().collect::<BTreeSet<_>>(),
            "{book}: the journal holds what was acknowledged and nothing else"
        );
        // The journal replays: the books open, and the ids run on without
        // a gap.
        let store = ledger_store::Store::open(&home.book_dir(book)).unwrap();
        assert_eq!(
            store.ledger().next_entry_id(),
            ran.len() as u64 + 1,
            "{book}: replayed ids"
        );
        eprintln!(
            "{book}: {} acknowledged, {} refused",
            ran.len(),
            refused.len()
        );
        match book {
            A => {
                assert!(!ran.is_empty() && !refused.is_empty(), "the hammer hit {A}");
            }
            _ => {
                assert!(refused.is_empty(), "{B} was never closed: no call refused");
                assert!(!ran.is_empty());
            }
        }
    }
    assert_eq!(
        reads[1].load(Ordering::SeqCst),
        0,
        "no read of {B} was refused"
    );
    assert!(reads[0].load(Ordering::SeqCst) > 0);
    // Between closings the journal only grew.
    assert!(
        closed_for.windows(2).all(|w| w[0] <= w[1]),
        "{closed_for:?}"
    );
}

/// A client's standalone event stream (the GET an MCP client keeps open
/// for server messages) ends only with its session: a close must not wait
/// on it as it waits on answers on their way out (it would sit out the
/// whole grace for every connected assistant). The stream is known to be
/// open (its response head has come) before the close begins.
#[test]
fn a_close_doesnt_wait_for_a_clients_open_event_stream() {
    let home = Home::new("open-close-stream");
    let server = Server::start(&home);
    home.create(A, "USD");
    let mut w = Wire::new(&home, A);
    w.initialize("streamer");
    assert_eq!(
        w.call("books.chart", json!({}))["result"]["isError"],
        json!(false)
    );
    // The client's own multi-thread runtime keeps the stream's connection
    // going while this thread asks for the close.
    let rt = tokio::runtime::Runtime::new().unwrap();
    let http = common::mcp_http(&home);
    let stream = rt.block_on(async {
        http.get(format!("http://localhost/b/{A}/mcp"))
            .bearer_auth(w.token().unwrap())
            .header("accept", "text/event-stream")
            .header("mcp-session-id", w.session().unwrap())
            .header("mcp-protocol-version", w.version().unwrap())
            .send()
            .await
            .expect("the stream opens")
    });
    // Its response head has come: the stream is open (no sleep guesses it).
    assert_eq!(stream.status().as_u16(), 200);
    assert!(
        stream
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|t| t.starts_with("text/event-stream")),
        "{:?}",
        stream.headers()
    );
    let started = std::time::Instant::now();
    assert_eq!(close_book(&home, A), CloseOutcome::Closed);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the close waited {:?} on a client's open stream",
        started.elapsed()
    );
    drop(stream);
    drop(rt);
    assert!(server.close().success());
}

// ---------------------------------------------------------------------
// After a close is answered, nothing of the server's reaches the book
// (local.rs INVARIANT 10). The tests below misbehave the way a real
// client does (an answer never read, a body never finished) and keep a
// backup target on the book, which the hammer above did not.
// ---------------------------------------------------------------------

/// Every file of a book's folder, by relative path: its size and SHA-256.
/// What stands still while the book is closed.
fn tree_of(dir: &std::path::Path) -> BTreeMap<String, (u64, String)> {
    fn walk(
        base: &std::path::Path,
        at: &std::path::Path,
        out: &mut BTreeMap<String, (u64, String)>,
    ) {
        let Ok(entries) = std::fs::read_dir(at) else {
            return;
        };
        for e in entries.flatten() {
            let path = e.path();
            if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                walk(base, &path, out);
            } else if let Ok(bytes) = std::fs::read(&path) {
                out.insert(
                    path.strip_prefix(base).unwrap().display().to_string(),
                    (
                        bytes.len() as u64,
                        ledger_store::journal::sha256_hex(&bytes),
                    ),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

/// Say what moved between two trees.
fn moved(
    before: &BTreeMap<String, (u64, String)>,
    after: &BTreeMap<String, (u64, String)>,
) -> Vec<String> {
    let mut out = Vec::new();
    for (k, v) in before {
        match after.get(k) {
            None => out.push(format!("{k} went")),
            Some(a) if a != v => out.push(format!("{k} changed")),
            Some(_) => {}
        }
    }
    out.extend(
        after
            .keys()
            .filter(|k| !before.contains_key(*k))
            .map(|k| format!("{k} appeared")),
    );
    out
}

/// Poll `ready` until it holds, or fail after `within`: a test waits on
/// the condition it needs, never on a sleep that guesses how long it takes.
fn wait_for(what: &str, within: Duration, mut ready: impl FnMut() -> bool) {
    let by = std::time::Instant::now() + within;
    while !ready() {
        assert!(std::time::Instant::now() < by, "never: {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(unix)]
mod raw {
    use super::*;
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;

    /// A client that speaks HTTP over the MCP socket by hand, so that a
    /// test can leave an answer unread or a body unfinished.
    pub struct Raw {
        stream: UnixStream,
        token: String,
        book: String,
        /// What `wait_for_bytes` took off the connection to see it.
        taken: Vec<u8>,
    }

    impl Raw {
        pub fn connect(home: &Home, book: &str) -> Self {
            let endpoint = ledger_mcp::ipc::Endpoint::new(&home.run(), "mcp.sock").unwrap();
            let stream = UnixStream::connect(endpoint.path()).unwrap();
            std::mem::forget(endpoint);
            Self {
                stream,
                token: home.token(book),
                book: book.into(),
                taken: Vec::new(),
            }
        }

        /// Write a request whose body is `body`, of which only `sent`
        /// bytes go out now (the rest is for [`Raw::finish`]).
        pub fn start(
            &mut self,
            method: &str,
            path: &str,
            extra: &[(&str, &str)],
            body: &[u8],
            sent: usize,
        ) {
            let mut head = format!(
                "{method} /b/{}/{path} HTTP/1.1\r\nhost: localhost\r\nauthorization: Bearer {}\r\ncontent-length: {}\r\n",
                self.book,
                self.token,
                body.len()
            );
            for (k, v) in extra {
                head.push_str(&format!("{k}: {v}\r\n"));
            }
            head.push_str("\r\n");
            self.stream.write_all(head.as_bytes()).unwrap();
            self.stream.write_all(&body[..sent]).unwrap();
        }

        /// Wait until the server has sent something on this connection
        /// (one byte is taken, and kept for [`Raw::head`]).
        pub fn wait_for_bytes(&mut self) {
            self.stream.set_nonblocking(true).unwrap();
            let mut byte = [0u8; 1];
            let by = std::time::Instant::now() + Duration::from_secs(60);
            loop {
                match self.stream.read(&mut byte) {
                    Ok(n) if n > 0 => {
                        self.taken.push(byte[0]);
                        break;
                    }
                    _ => {
                        assert!(std::time::Instant::now() < by, "the server sent nothing");
                        std::thread::sleep(Duration::from_millis(5));
                    }
                }
            }
            self.stream.set_nonblocking(false).unwrap();
        }

        /// The rest of a body begun with [`Raw::start`].
        pub fn finish(&mut self, rest: &[u8]) {
            self.stream.write_all(rest).unwrap();
        }

        /// The response's status line and headers (the body is left).
        pub fn head(&mut self) -> (u16, String) {
            self.stream
                .set_read_timeout(Some(Duration::from_secs(60)))
                .unwrap();
            let mut seen = std::mem::take(&mut self.taken);
            let mut byte = [0u8; 1];
            while !seen.ends_with(b"\r\n\r\n") {
                self.stream.read_exact(&mut byte).unwrap();
                seen.push(byte[0]);
            }
            let text = String::from_utf8_lossy(&seen).to_string();
            let status = text.split(' ').nth(1).unwrap().parse().unwrap();
            (status, text)
        }
    }
}

/// Name a folder as the book's backup target, as an agent would.
fn set_backup_folder(w: &mut Wire, folder: &std::path::Path) {
    let r = w.call(
        "books.set_setting",
        json!({"changes": [{"key": "backup.folder", "value": folder.display().to_string()}],
               "decided_by": "Test Owner", "note": "Backups."}),
    );
    assert_eq!(r["result"]["isError"], json!(false), "{r}");
}

/// A draft entry, journaled: its id.
fn draft(w: &mut Wire, tag: &str) -> u64 {
    let r = w.call(
        "books.draft_entry",
        json!({"date": "2026-03-01", "description": tag}),
    );
    assert_eq!(r["result"]["isError"], json!(false), "{r}");
    r["result"]["structuredContent"]["entry_id"]
        .as_u64()
        .unwrap()
}

/// What the access log of `id` says, whole (read checks its chain).
fn access_of(home: &Home, id: &str) -> Vec<ledger_store::access::Recorded> {
    ledger_store::access::read(&home.book_dir(id))
        .unwrap_or_else(|e| panic!("{id}'s access log isn't whole: {e}"))
        .0
}

/// The tree of `id` must not move for a while, however long the server's
/// timers (a few milliseconds) and its backups and sweeps have to move it:
/// compared again and again over `window` (a window to catch a late
/// writer in, not a time anything waits for).
fn stands_still(home: &Home, id: &str, window: Duration, what: &str) {
    let first = tree_of(&home.book_dir(id));
    let until = std::time::Instant::now() + window;
    while std::time::Instant::now() < until {
        let now = tree_of(&home.book_dir(id));
        assert_eq!(moved(&first, &now), Vec::<String>::new(), "{what}");
        std::thread::sleep(Duration::from_millis(15));
    }
}

/// The answer to a call a client never reads is stuck in the server's
/// socket (the tools list is over 500 KB: more than the socket holds), so
/// the book's close is made to wait for it, and may not for ever: it ends
/// the sessions after its grace, answers closed, and then nothing is
/// written to the book, whose access log stays a whole chain through the
/// reopening (local.rs INVARIANT 10; the call that was being answered ran
/// and is on record, as one whose connection dropped at any other moment).
#[cfg(unix)]
#[test]
fn a_client_that_never_reads_its_answer_cannot_hold_a_close_nor_break_the_log() {
    let home = Home::new("close-unread");
    let server = Server::start_with(&home, &["--answer-grace", "700", "--timers", "10"]);
    home.create(A, "USD");
    let mut w = Wire::new(&home, A);
    w.initialize("reader");
    draft(&mut w, "before");
    let mut raw = raw::Raw::connect(&home, A);
    let body =
        json!({"jsonrpc": "2.0", "id": 77, "method": "tools/list", "params": {}}).to_string();
    raw.start(
        "POST",
        "mcp",
        &[
            ("content-type", "application/json"),
            ("accept", "application/json, text/event-stream"),
            ("mcp-session-id", w.session().unwrap()),
            ("mcp-protocol-version", w.version().unwrap()),
        ],
        body.as_bytes(),
        body.len(),
    );
    // The server is answering: it has sent what the socket holds, and the
    // rest is stuck behind a client that doesn't read.
    raw.wait_for_bytes();
    let started = std::time::Instant::now();
    assert_eq!(close_book(&home, A), CloseOutcome::Closed);
    let took = started.elapsed();
    assert!(
        took >= Duration::from_millis(600),
        "the close didn't wait for the answer on its way out ({took:?})"
    );
    assert!(
        took < Duration::from_secs(30),
        "the close was held by a client that never reads ({took:?})"
    );
    let at_close = tree_of(&home.book_dir(A));
    stands_still(
        &home,
        A,
        Duration::from_millis(400),
        "after the close was answered",
    );
    assert_eq!(
        moved(&at_close, &tree_of(&home.book_dir(A))),
        Vec::<String>::new()
    );
    // Opened again: the log, which the old one wrote into until the close,
    // takes records against its own head.
    assert_eq!(open_book(&home, A), OpenOutcome::Opened);
    let mut again = Wire::new(&home, A);
    again.initialize("after");
    draft(&mut again, "after");
    // One record for each of the two drafts, in one chain across the
    // close and the reopening.
    let log = access_of(&home, A);
    assert_eq!(
        log.iter().map(|r| r.seq).collect::<Vec<_>>(),
        vec![1, 2],
        "{log:?}"
    );
    drop(raw);
    assert!(server.close().success());
    access_of(&home, A);
}

/// A call that journaled, whose client then went without reading the
/// answer, is on record as having run: the access record is written
/// before the answer goes, so a client that retries (nothing in these
/// calls makes a retry safe: local.rs INVARIANT 10) can find out.
#[cfg(unix)]
#[test]
fn a_call_whose_answer_was_never_read_is_on_record_as_having_run() {
    let home = Home::new("close-ran");
    let server = Server::start_with(&home, &["--answer-grace", "300"]);
    home.create(A, "USD");
    let mut w = Wire::new(&home, A);
    w.initialize("reader");
    let mut raw = raw::Raw::connect(&home, A);
    let body = json!({"jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": {
        "name": "books.draft_entry",
        "arguments": {"date": "2026-03-01", "description": "unread"}}})
    .to_string();
    raw.start(
        "POST",
        "mcp",
        &[
            ("content-type", "application/json"),
            ("accept", "application/json, text/event-stream"),
            ("mcp-session-id", w.session().unwrap()),
            ("mcp-protocol-version", w.version().unwrap()),
        ],
        body.as_bytes(),
        body.len(),
    );
    // It ran (it is in the journal), and the client goes without a word.
    wait_for("the draft is journaled", Duration::from_secs(30), || {
        files_of(&home, A)
            .0
            .iter()
            .any(|l| l.payload["description"] == json!("unread"))
    });
    drop(raw);
    assert_eq!(close_book(&home, A), CloseOutcome::Closed);
    let log = access_of(&home, A);
    let rec = log
        .iter()
        .find(|r| r.access.subject == "books.draft_entry")
        .expect("the call has a record");
    assert_eq!(rec.access.result, ledger_store::access::Outcome::Ok);
    assert!(rec.access.head_after.is_some(), "{rec:?}");
    assert!(server.close().success());
}

/// A stand-in `git` for the server's PATH: it runs the real one, unless
/// the test has said otherwise with a file beside it: `stall` (it waits,
/// recording itself in `stalled`, until the file is gone, then in
/// `resumed`), which is how a backup is made slow.
#[cfg(unix)]
fn shim_git(home: &Home) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let dir = home.dir.path().join("shim");
    std::fs::create_dir_all(&dir).unwrap();
    let real = ["/usr/bin/git", "/bin/git", "/usr/local/bin/git"]
        .into_iter()
        .find(|p| std::path::Path::new(p).is_file())
        .expect("git is installed");
    let exe = dir.join("git");
    std::fs::write(
        &exe,
        format!(
            "#!/bin/sh\ndir=\"$(dirname \"$0\")\"\necho \"$$ $*\" >> \"$dir/calls\"\nif [ -e \"$dir/stall\" ]; then\n  echo $$ >> \"$dir/stalled\"\n  while [ -e \"$dir/stall\" ]; do sleep 0.05; done\n  echo $$ >> \"$dir/resumed\"\nfi\nexec {real} \"$@\"\n"
        ),
    )
    .unwrap();
    std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
    dir
}

/// Whether process `pid` is still there.
#[cfg(unix)]
fn alive(pid: u32) -> bool {
    std::path::Path::new(&format!("/proc/{pid}")).exists()
        && std::fs::read_to_string(format!("/proc/{pid}/stat"))
            .map(|s| !s.contains(") Z "))
            .unwrap_or(false)
}

/// The server, with the stand-in git first on its PATH.
#[cfg(unix)]
fn server_with_shim(home: &Home, shim: &std::path::Path, args: &[&str]) -> Server {
    let path = std::env::join_paths([
        shim,
        std::path::Path::new("/usr/bin"),
        std::path::Path::new("/bin"),
    ])
    .unwrap();
    let path: PathBuf = path.into();
    Server::start_full(home, &[("PATH", path.as_path())], args)
}

/// A book with a backup target whose backup is slow: the close is not
/// answered while the backup's work runs. Its bound lapses, git is
/// stopped (with all it started), the close waits for that to be done,
/// and says the backup wasn't whole. Nothing of the backup runs after the
/// answer, though the stall is lifted: a git left running would go on.
#[cfg(unix)]
#[test]
fn a_close_is_not_answered_while_its_backup_still_runs() {
    let home = Home::new("close-slow-backup");
    let shim = shim_git(&home);
    let server = server_with_shim(
        &home,
        &shim,
        &[
            "--close-backup",
            "1200",
            "--backup-quiet",
            "3600000",
            "--timers",
            "20",
        ],
    );
    home.create(A, "USD");
    let disk = home.dir.path().join("disk");
    std::fs::create_dir_all(&disk).unwrap();
    let mut w = Wire::new(&home, A);
    w.initialize("owner");
    set_backup_folder(&mut w, &disk);
    draft(&mut w, "moves the head past the last backup");
    w.end();
    // From here every git waits.
    std::fs::write(shim.join("stall"), b"").unwrap();
    let started = std::time::Instant::now();
    let outcome = close_book(&home, A);
    let took = started.elapsed();
    assert!(
        took >= Duration::from_millis(1100),
        "answered before the backup's bound ({took:?}): {outcome:?}"
    );
    assert!(took < Duration::from_secs(30), "{took:?}");
    match &outcome {
        CloseOutcome::ClosedBackupIncomplete(why) => {
            assert!(why.contains("longer than"), "{why}");
            home.assert_hidden(why);
        }
        other => panic!("a stopped backup was answered as {other:?}"),
    }
    // Answered: the git that was stalled is gone, with what it started.
    let stalled: Vec<u32> = std::fs::read_to_string(shim.join("stalled"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.trim().parse().ok())
        .collect();
    assert!(!stalled.is_empty(), "the backup never reached git");
    for pid in &stalled {
        assert!(
            !alive(*pid),
            "git {pid} still runs after the close was answered"
        );
    }
    let at_close = tree_of(&home.book_dir(A));
    // The stall is lifted: anything that had survived would go on now.
    std::fs::remove_file(shim.join("stall")).unwrap();
    stands_still(
        &home,
        A,
        Duration::from_millis(500),
        "after the close was answered",
    );
    assert!(
        !shim.join("resumed").exists(),
        "a git went on after the close was answered"
    );
    assert_eq!(
        moved(&at_close, &tree_of(&home.book_dir(A))),
        Vec::<String>::new()
    );
    // Opened again, the book's backup record is a whole chain, and it
    // backs up (nothing of the old backups is in its way).
    assert_eq!(open_book(&home, A), OpenOutcome::Opened);
    let status = home.admin(Request::BackUpNow {
        id: A.into(),
        new_disk: false,
    });
    assert!(matches!(status, Response::BackedUp { .. }), "{status:?}");
    ledger_store::chain::read(
        &ledger_store::ops::ops_dir(&home.book_dir(A)).join("backup.jsonl"),
        "wanderbooks-backup-v1",
    )
    .expect("the backup record is a whole chain");
    assert!(server.close().success());
}

/// A backup that fails (its disk isn't there) closes the book all the same,
/// quickly, and says so: never a plain "closed" for a backup that wasn't
/// made; and nothing runs after.
#[cfg(unix)]
#[test]
fn a_close_whose_backup_fails_says_so_and_leaves_nothing_running() {
    let home = Home::new("close-failing-backup");
    let shim = shim_git(&home);
    let server = server_with_shim(
        &home,
        &shim,
        &["--backup-quiet", "3600000", "--timers", "20"],
    );
    home.create(A, "USD");
    let unplugged = home.dir.path().join("elsewhere").join("unplugged");
    std::fs::create_dir_all(unplugged.parent().unwrap()).unwrap();
    let mut w = Wire::new(&home, A);
    w.initialize("owner");
    set_backup_folder(&mut w, &unplugged);
    draft(&mut w, "unbacked");
    w.end();
    let outcome = close_book(&home, A);
    match &outcome {
        CloseOutcome::ClosedBackupIncomplete(why) => {
            assert!(
                why.contains("failed") || why.contains("isn't there"),
                "{why}"
            );
            home.assert_hidden(why);
        }
        other => panic!("a backup that failed was answered as {other:?}"),
    }
    assert_eq!(listed(&home, A), (false, true, None));
    let at_close = tree_of(&home.book_dir(A));
    let calls = std::fs::read_to_string(shim.join("calls"))
        .unwrap_or_default()
        .lines()
        .count();
    stands_still(
        &home,
        A,
        Duration::from_millis(400),
        "after the failed backup's close",
    );
    assert_eq!(
        moved(&at_close, &tree_of(&home.book_dir(A))),
        Vec::<String>::new()
    );
    assert_eq!(
        std::fs::read_to_string(shim.join("calls"))
            .unwrap_or_default()
            .lines()
            .count(),
        calls,
        "git was run after the close was answered"
    );
    // The same book, backed up fine, is answered plainly.
    assert_eq!(open_book(&home, A), OpenOutcome::Opened);
    std::fs::create_dir(&unplugged).unwrap();
    let mut w = Wire::new(&home, A);
    w.initialize("owner");
    draft(&mut w, "backed up");
    w.end();
    assert_eq!(close_book(&home, A), CloseOutcome::Closed);
    assert!(server.close().success());
}

/// Every `.part` file (an upload being received) under `dir`.
fn parts_under(dir: &std::path::Path) -> Vec<String> {
    tree_of(dir)
        .into_keys()
        .filter(|k| k.ends_with(".part"))
        .collect()
}

/// Begin an upload for `bytes` as `w`, and send the first half of the
/// bytes by hand, leaving the rest for the caller: the id, and the
/// connection, with the server part way through receiving them.
#[cfg(unix)]
fn half_sent_upload(home: &Home, w: &mut Wire, book: &str, bytes: &[u8]) -> (String, raw::Raw) {
    let begun = w.call(
        "books.begin_upload",
        common::upload_args(bytes, "r.bin", "application/octet-stream", "receipt"),
    );
    let id = begun["result"]["structuredContent"]["upload_id"]
        .as_str()
        .unwrap_or_else(|| panic!("{begun}"))
        .to_string();
    let mut raw = raw::Raw::connect(home, book);
    raw.start("PUT", &format!("u/{id}"), &[], bytes, bytes.len() / 2);
    // The server is receiving: its part file is there (the request is
    // inside the book's door, its record not yet written).
    wait_for("the upload's part file", Duration::from_secs(30), || {
        !parts_under(&home.book_dir(book)).is_empty()
    });
    (id, raw)
}

/// An upload part way in when the close begins finishes before the close
/// is answered (nothing is cut, nothing half done): its bytes are kept,
/// its record written, no part file left, and nothing moves afterwards.
#[cfg(unix)]
#[test]
fn an_upload_in_flight_finishes_before_the_close_is_answered() {
    let home = Home::new("close-upload");
    let server = Server::start_with(&home, &["--answer-grace", "500", "--timers", "10"]);
    home.create(A, "USD");
    let mut w = Wire::new(&home, A);
    w.initialize("uploader");
    let bytes: Vec<u8> = (0..200_000u32).map(|n| (n % 251) as u8).collect();
    let (id, mut raw) = half_sent_upload(&home, &mut w, A, &bytes);
    let closing = std::thread::spawn({
        let home_root = home.run();
        move || {
            ledger_mcp::admin::call(
                &home_root.join("admin.sock"),
                Request::CloseBook { id: A.into() },
            )
            .unwrap()
        }
    });
    wait_for("the book is listed closed", Duration::from_secs(30), || {
        listed(&home, A).1
    });
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        !closing.is_finished(),
        "the close was answered with an upload half received"
    );
    raw.finish(&bytes[bytes.len() / 2..]);
    let (status, head) = raw.head();
    assert_eq!(status, 200, "{head}");
    match closing.join().unwrap() {
        Response::CloseBook { outcome, .. } => assert_eq!(outcome, CloseOutcome::Closed),
        other => panic!("{other:?}"),
    }
    let tree = tree_of(&home.book_dir(A));
    assert_eq!(parts_under(&home.book_dir(A)), Vec::<String>::new());
    assert!(
        tree.keys().any(|k| k.ends_with(&format!("{id}.bin"))),
        "the bytes were kept"
    );
    let log = access_of(&home, A);
    assert!(
        log.iter().any(|r| r.access.action == "upload"
            && r.access.subject == id
            && r.access.result == ledger_store::access::Outcome::Ok),
        "the upload's record is written before the answer: {log:?}"
    );
    stands_still(
        &home,
        A,
        Duration::from_millis(300),
        "after the close was answered",
    );
    assert!(server.close().success());
}

/// A backup of `id`, taken now to its folder target; the repository.
#[cfg(unix)]
fn backed_up_to_folder(home: &Home, w: &mut Wire, id: &str) -> PathBuf {
    let disk = home.dir.path().join("disk");
    std::fs::create_dir_all(&disk).unwrap();
    set_backup_folder(w, &disk);
    draft(w, "kept in the backup");
    match home.admin(Request::BackUpNow {
        id: id.into(),
        new_disk: false,
    }) {
        Response::BackedUp { targets, .. } => assert!(targets[0].problem.is_none(), "{targets:?}"),
        other => panic!("{other:?}"),
    }
    disk.join(format!("{id}.git"))
}

/// A restore over an open book lets go of it as a close does: what is
/// inside the book's door finishes first (an upload half received is
/// completed and recorded in the book it was sent to, and no part file is
/// left in `legacy/`), its sessions end, nothing of it moves once the
/// restore is answered.
#[cfg(unix)]
#[test]
fn a_restore_over_an_open_book_waits_for_what_is_inside_it() {
    let home = Home::new("restore-drain");
    let server = Server::start_with(
        &home,
        &[
            "--answer-grace",
            "500",
            "--timers",
            "10",
            "--backup-quiet",
            "3600000",
        ],
    );
    home.create(A, "USD");
    let mut w = Wire::new(&home, A);
    w.initialize("owner");
    let repo = backed_up_to_folder(&home, &mut w, A);
    let bytes: Vec<u8> = (0..200_000u32).map(|n| (n % 241) as u8).collect();
    let (id, mut raw) = half_sent_upload(&home, &mut w, A, &bytes);
    let source = repo.display().to_string();
    let preview = match home.admin(Request::Restore {
        id: A.into(),
        source: source.clone(),
        replace: true,
        confirmed: false,
        expect_head: None,
    }) {
        Response::RestorePreview(p) => p,
        other => panic!("{other:?}"),
    };
    let restoring = std::thread::spawn({
        let admin = home.run().join("admin.sock");
        move || {
            ledger_mcp::admin::call(
                &admin,
                Request::Restore {
                    id: A.into(),
                    source,
                    replace: true,
                    confirmed: true,
                    expect_head: Some(preview.head),
                },
            )
            .unwrap()
        }
    });
    // The books being replaced are no longer served as these.
    wait_for(
        "the replaced books are let go",
        Duration::from_secs(30),
        || {
            let mut late = Wire::with_token(&home, A, Some(w.token().unwrap().to_string()));
            late.send(&json!({"jsonrpc": "2.0", "id": 1, "method": "ping"}));
            late.refused
                .as_ref()
                .is_some_and(|r| r.status == 404 || r.status == 503)
        },
    );
    // The restore (a clone and a check of the backup, which take a second
    // or so) is not answered while the upload is half received: watched
    // for five seconds, in which a restore that doesn't wait would finish.
    let watched = std::time::Instant::now() + Duration::from_secs(5);
    while std::time::Instant::now() < watched {
        assert!(
            !restoring.is_finished(),
            "the restore was answered with an upload half received"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    raw.finish(&bytes[bytes.len() / 2..]);
    let (status, head) = raw.head();
    assert_eq!(status, 200, "{head}");
    let restored = match restoring.join().unwrap() {
        Response::Restored(r) => r,
        other => panic!("{other:?}"),
    };
    let legacy = home.root().join("legacy");
    assert!(restored.replaced.is_some(), "{restored:?}");
    // No part file anywhere, and the replaced book's own log, kept in
    // legacy/, is whole and holds the upload, which finished in it.
    assert_eq!(parts_under(&legacy), Vec::<String>::new());
    let old = std::fs::read_dir(&legacy)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let (records, _) = ledger_store::access::read(&old).expect("the replaced book's log is whole");
    assert!(
        records
            .iter()
            .any(|r| r.access.action == "upload" && r.access.subject == id),
        "{records:?}"
    );
    let at_answer = tree_of(&legacy);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(moved(&at_answer, &tree_of(&legacy)), Vec::<String>::new());
    assert!(server.close().success());
}

/// Downloads running while a book closes each complete whole, and are on
/// record, or are refused unrun: never cut, never answered from a book
/// after it was closed.
#[cfg(unix)]
#[test]
fn downloads_during_a_close_complete_or_are_refused() {
    let home = Arc::new(Home::new("close-download"));
    let server = Server::start_with(&home, &["--answer-grace", "500", "--timers", "10"]);
    home.create(A, "USD");
    let mut w = Wire::new(&home, A);
    w.initialize("reader");
    let bytes: Vec<u8> = (0..300_000u32).map(|n| (n % 239) as u8).collect();
    let id = w.upload(&bytes, "big.bin", "application/octet-stream", "receipt");
    let filed = w.call("books.attach_receipt", json!({"upload_id": id}));
    let att = filed["result"]["structuredContent"]["attachment_id"]
        .as_u64()
        .unwrap_or_else(|| panic!("{filed}"));
    let stop = Arc::new(AtomicBool::new(false));
    let answers = Arc::new(Mutex::new(Vec::<(u16, usize)>::new()));
    let token = w.token().unwrap().to_string();
    let readers: Vec<_> = (0..4)
        .map(|_| {
            let (home, stop, answers, token) =
                (home.clone(), stop.clone(), answers.clone(), token.clone());
            std::thread::spawn(move || {
                let mut wire = Wire::with_token(&home, A, Some(token));
                while !stop.load(Ordering::SeqCst) {
                    let (status, body) = wire.get(&format!("a/{att}"));
                    answers.lock().unwrap().push((status, body.len()));
                }
            })
        })
        .collect();
    wait_for(
        "downloads are being answered",
        Duration::from_secs(30),
        || answers.lock().unwrap().len() > 3,
    );
    assert_eq!(close_book(&home, A), CloseOutcome::Closed);
    let at_close = tree_of(&home.book_dir(A));
    let recorded = access_of(&home, A)
        .iter()
        .filter(|r| {
            r.access.action == "download" && r.access.result == ledger_store::access::Outcome::Ok
        })
        .count();
    stands_still(
        &home,
        A,
        Duration::from_millis(300),
        "after the close was answered",
    );
    stop.store(true, Ordering::SeqCst);
    for r in readers {
        r.join().unwrap();
    }
    let answers = answers.lock().unwrap().clone();
    let ok = answers.iter().filter(|(s, _)| *s == 200).count();
    for (status, len) in &answers {
        match *status {
            200 => assert_eq!(*len, bytes.len(), "a download was cut"),
            503 => {}
            other => panic!("a download answered {other}"),
        }
    }
    assert_eq!(
        ok, recorded,
        "every download that was answered is on record, and no other"
    );
    assert_eq!(
        moved(&at_close, &tree_of(&home.book_dir(A))),
        Vec::<String>::new()
    );
    assert!(server.close().success());
}

/// Every timer of the server (the sweep of lapsed uploads and exports, the
/// look for a backup due, the look for a drill due) runs every few
/// milliseconds, on a book with a backup target, closed and opened over
/// and over: while the book is closed not one file of its folder moves,
/// though files for a sweep to take are planted in it; while it is open
/// the timers work (it is backed up and drilled), and every log, at the
/// end, is a whole chain.
#[cfg(unix)]
#[test]
fn the_timers_never_touch_a_closed_book() {
    let home = Home::new("close-timers");
    let server = Server::start_with(
        &home,
        &[
            "--timers",
            "5",
            "--backup-quiet",
            "0",
            "--drill-first",
            "0",
            "--answer-grace",
            "300",
        ],
    );
    home.create(A, "USD");
    home.create(B, "USD");
    let mut w = Wire::new(&home, A);
    w.initialize("owner");
    let disk = home.dir.path().join("disk");
    std::fs::create_dir_all(&disk).unwrap();
    set_backup_folder(&mut w, &disk);
    w.end();
    let backup_records = |kind: &str| {
        std::fs::read_to_string(home.book_dir(A).join("ops").join("backup.jsonl"))
            .unwrap_or_default()
            .lines()
            .filter(|l| l.contains(&format!("\"kind\":\"{kind}\"")))
            .count()
    };
    let dir = home.book_dir(A);
    for cycle in 0..8 {
        // Open: the timers are at work on it, a write is backed up soon.
        let pushed_before = backup_records("push");
        let mut w = Wire::new(&home, A);
        w.initialize("owner");
        draft(&mut w, &format!("cycle {cycle}"));
        w.end();
        wait_for("the write is backed up", Duration::from_secs(60), || {
            backup_records("push") > pushed_before
        });
        wait_for("its backup is drilled", Duration::from_secs(60), || {
            backup_records("drill") >= 1
        });
        assert!(
            matches!(close_book(&home, A), CloseOutcome::Closed),
            "cycle {cycle}"
        );
        // Closed: what a sweep would take is planted, and nothing moves.
        for folder in ["uploads", "exports"] {
            std::fs::write(dir.join(folder).join("stray.bin"), b"x").unwrap();
        }
        stands_still(
            &home,
            A,
            Duration::from_millis(150),
            &format!("cycle {cycle}, closed"),
        );
        assert!(dir.join("uploads").join("stray.bin").exists());
        assert!(dir.join("exports").join("stray.bin").exists());
        assert_eq!(open_book(&home, A), OpenOutcome::Opened, "cycle {cycle}");
    }
    assert!(server.close().success());
    access_of(&home, A);
    ledger_store::chain::read(
        &dir.join("ops").join("backup.jsonl"),
        "wanderbooks-backup-v1",
    )
    .expect("the backup record is a whole chain");
    ledger_store::Store::open(&dir).expect("the journal replays");
}
