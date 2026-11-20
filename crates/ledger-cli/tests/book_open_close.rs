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
    let server = Server::start(&home);
    home.create(A, "USD");
    home.create(B, "USD");
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
    let mut closed_for = Vec::new();
    for cycle in 0..CYCLES {
        std::thread::sleep(Duration::from_millis(150 + (cycle as u64 * 37) % 100));
        let beta_before = done.lock().unwrap().iter().filter(|d| d.book == B).count();
        assert_eq!(close_book(&home, A), CloseOutcome::Closed, "cycle {cycle}");
        // Answered closed: from here nothing runs on the book until it is
        // opened, so its files stand still (and, the writer lock being
        // released, another program could open it).
        let before = files_of(&home, A);
        assert_eq!(listed(&home, A), (false, true, None));
        closed_on_every_route(&home, A);
        std::thread::sleep(Duration::from_millis(70));
        let after = files_of(&home, A);
        assert_eq!(
            (before.0.len(), before.1),
            (after.0.len(), after.1),
            "cycle {cycle}: the closed book's journal or access log moved"
        );
        closed_for.push(before.0.len());
        // The other book answered while this one was closed.
        let beta_after = done.lock().unwrap().iter().filter(|d| d.book == B).count();
        assert!(
            beta_after > beta_before,
            "cycle {cycle}: no call on {B} finished while {A} was closed"
        );
        assert_eq!(open_book(&home, A), OpenOutcome::Opened, "cycle {cycle}");
    }
    std::thread::sleep(Duration::from_millis(200));
    stop.store(true, Ordering::SeqCst);
    for a in agents {
        a.join().unwrap();
    }
    assert!(server.close().success());

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
/// whole grace for every connected assistant).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_close_doesnt_wait_for_a_clients_open_event_stream() {
    let home = Arc::new(
        tokio::task::spawn_blocking(|| Home::new("open-close-stream"))
            .await
            .unwrap(),
    );
    let server = {
        let home = home.clone();
        tokio::task::spawn_blocking(move || {
            let server = Server::start(&home);
            home.create(A, "USD");
            server
        })
        .await
        .unwrap()
    };
    let client = tokio::task::spawn_blocking({
        let home = home.clone();
        move || home.token(A)
    })
    .await
    .unwrap();
    let c = common::connect_with(&home, A, Some(client)).await.unwrap();
    common::call(&c, "books.chart", json!({})).await.unwrap();
    // Time for the client to open its stream.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let started = std::time::Instant::now();
    let closed = {
        let home = home.clone();
        tokio::task::spawn_blocking(move || close_book(&home, A))
            .await
            .unwrap()
    };
    assert_eq!(closed, CloseOutcome::Closed);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the close waited {:?} on a client's open stream",
        started.elapsed()
    );
    let _ = c.cancel().await;
    tokio::task::spawn_blocking(move || assert!(server.close().success()))
        .await
        .unwrap();
}
