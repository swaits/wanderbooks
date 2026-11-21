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
    // `initialized` is a notification, handled after its 202: each client's
    // name is known once the server has (waited for, not guessed).
    wait_for("both clients are known", Duration::from_secs(60), || {
        sessions(&home)
            .iter()
            .find(|b| b.book == A)
            .is_some_and(|a| {
                a.sessions.len() == 2 && a.sessions.iter().all(|s| s.client_name.is_some())
            })
    });
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
/// and no call on the other book was ever refused. Meanwhile admin jobs on
/// other books (a third book closed and opened over and over, new books made)
/// are all done and none is ever stopped, refused or told of an error by the
/// closings (local.rs INVARIANT 10).
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
    home.create("cedar", "USD");
    let disk = home.dir.path().join("disk");
    std::fs::create_dir_all(&disk).unwrap();
    {
        let mut w = Wire::new(&home, A);
        w.initialize("owner");
        set_backup_folder(&mut w, &disk);
        w.end();
    }
    let stop = Arc::new(AtomicBool::new(false));
    // Admin jobs on other books, while the hammered one is closed and opened:
    // a third book closed and opened over and over, and new books made. Every
    // one is done as asked; what isn't is said here.
    let spared = Arc::new(Mutex::new((0usize, Vec::<String>::new())));
    let maker = {
        let (home, stop, spared) = (home.clone(), stop.clone(), spared.clone());
        std::thread::spawn(move || {
            let mut made = 0;
            while !stop.load(Ordering::SeqCst) {
                let mut said = Vec::new();
                if made < 8 {
                    made += 1;
                    match home.admin(Request::Create {
                        id: format!("made-{made}"),
                        currency: "USD".into(),
                    }) {
                        Response::Created { problem: None, .. } => {}
                        other => said.push(format!("create: {other:?}")),
                    }
                }
                match home.admin(Request::CloseBook { id: "cedar".into() }) {
                    Response::CloseBook {
                        outcome: CloseOutcome::Closed,
                        ..
                    } => {}
                    other => said.push(format!("close: {other:?}")),
                }
                match home.admin(Request::OpenBook { id: "cedar".into() }) {
                    Response::OpenBook {
                        outcome: OpenOutcome::Opened,
                        ..
                    } => {}
                    other => said.push(format!("open: {other:?}")),
                }
                let mut spared = spared.lock().unwrap();
                spared.0 += 1;
                spared.1.extend(said);
            }
        })
    };
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
    maker.join().unwrap();
    {
        let spared = spared.lock().unwrap();
        assert!(spared.0 > 0, "no admin job ran on the other books");
        assert_eq!(
            spared.1,
            Vec::<String>::new(),
            "a close of {A} stopped or refused a job on another book"
        );
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

        /// [`Raw::start`] for a request that sends its body only once the
        /// server has asked for it (`Expect: 100-continue`): the server asks
        /// when its handler first reads the body, which it does inside the
        /// book's door. So once this returns the request is known to be
        /// inside, with `sent` bytes of its body written: a condition, not a
        /// guess at how long the server takes to read a head.
        pub fn start_inside(
            &mut self,
            method: &str,
            path: &str,
            extra: &[(&str, &str)],
            body: &[u8],
            sent: usize,
        ) {
            let mut extra = extra.to_vec();
            extra.push(("expect", "100-continue"));
            self.start(method, path, &extra, body, 0);
            self.stream
                .set_read_timeout(Some(Duration::from_secs(120)))
                .unwrap();
            let mut seen = Vec::new();
            let mut byte = [0u8; 1];
            while !seen.ends_with(b"\r\n\r\n") {
                self.stream
                    .read_exact(&mut byte)
                    .expect("the server never asked for the body");
                seen.push(byte[0]);
            }
            let said = String::from_utf8_lossy(&seen).to_string();
            assert!(said.starts_with("HTTP/1.1 100"), "{said}");
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

        /// A piece of the rest of a body; false once the server has closed
        /// the connection (nothing more can be sent).
        pub fn try_finish(&mut self, piece: &[u8]) -> bool {
            self.stream.write_all(piece).is_ok()
        }

        /// Everything the server sends until it closes the connection,
        /// which it must do within `within`.
        pub fn until_closed(&mut self, within: Duration) -> String {
            self.stream.set_read_timeout(Some(within)).unwrap();
            let mut seen = std::mem::take(&mut self.taken);
            let mut buf = [0u8; 4096];
            loop {
                match self.stream.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => seen.extend_from_slice(&buf[..n]),
                    Err(e)
                        if matches!(
                            e.kind(),
                            std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::BrokenPipe
                        ) =>
                    {
                        break;
                    }
                    Err(e) => panic!("the connection was left open: {e}"),
                }
            }
            String::from_utf8_lossy(&seen).to_string()
        }

        /// The whole response: its status, and its body (as long as its
        /// `content-length` says).
        pub fn answer(&mut self) -> (u16, String) {
            let (status, head) = self.head();
            let lower = head.to_ascii_lowercase();
            let mut body = Vec::new();
            if lower.contains("transfer-encoding: chunked") {
                // Chunks, each a hex size line, its bytes and a line end.
                loop {
                    let mut size = String::new();
                    let mut byte = [0u8; 1];
                    while !size.ends_with("\r\n") {
                        self.stream.read_exact(&mut byte).unwrap();
                        size.push(byte[0] as char);
                    }
                    let n = usize::from_str_radix(size.trim(), 16).unwrap();
                    let mut chunk = vec![0u8; n + 2];
                    self.stream.read_exact(&mut chunk).unwrap();
                    if n == 0 {
                        break;
                    }
                    body.extend_from_slice(&chunk[..n]);
                }
            } else {
                let length: usize = lower
                    .lines()
                    .find_map(|l| {
                        l.strip_prefix("content-length:")
                            .map(|v| v.trim().parse().unwrap())
                    })
                    .unwrap_or(0);
                body = vec![0u8; length];
                self.stream.read_exact(&mut body).unwrap();
            }
            (status, String::from_utf8_lossy(&body).to_string())
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
    // The backup the opening makes (it may still be under way, and would be
    // stopped by the close) is waited for: a backup asked for takes its turn
    // after it.
    match home.admin(Request::BackUpNow {
        id: A.into(),
        new_disk: false,
    }) {
        Response::BackedUp { targets, .. } => {
            assert!(targets.iter().all(|t| t.problem.is_none()), "{targets:?}");
        }
        other => panic!("{other:?}"),
    }
    let mut w = Wire::new(&home, A);
    w.initialize("owner");
    draft(&mut w, "backed up");
    w.end();
    let outcome = close_book(&home, A);
    assert_eq!(
        outcome,
        CloseOutcome::Closed,
        "{}",
        std::fs::read_to_string(home.book_dir(A).join("ops").join("backup.jsonl"))
            .unwrap_or_default()
    );
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
    // A negative window: it can only fail if the close was answered, never
    // because the machine was slow.
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

/// Whether the access log of `id` holds a record of `action` on `subject`
/// that the close cut (never a record of it having run).
fn cut_records(home: &Home, id: &str, action: &str) -> Vec<ledger_store::access::Recorded> {
    access_of(home, id)
        .into_iter()
        .filter(|r| {
            r.access.action == action
                && r.access.result
                    == ledger_store::access::Outcome::Refused("cut_by_close".to_string())
        })
        .collect()
}

/// An upload whose body stalls half way (the client neither sends the rest
/// nor hangs up) is inside the book's door, and would hold the close for
/// ever. The close waits the grace, cuts it: its connection is closed, its
/// part file is gone, its record says the close cut it, nothing was
/// kept. The close is answered, and the log is a whole chain through a
/// reopening (local.rs INVARIANT 10, the cut).
#[cfg(unix)]
#[test]
fn a_stalled_upload_body_cannot_hold_a_close_and_leaves_no_part_file() {
    let home = Home::new("close-stalled-upload");
    let server = Server::start_with(&home, &["--answer-grace", "600", "--timers", "10"]);
    home.create(A, "USD");
    let mut w = Wire::new(&home, A);
    w.initialize("uploader");
    let bytes: Vec<u8> = (0..200_000u32).map(|n| (n % 251) as u8).collect();
    let (id, mut raw) = half_sent_upload(&home, &mut w, A, &bytes);
    let started = std::time::Instant::now();
    assert_eq!(close_book(&home, A), CloseOutcome::Closed);
    let took = started.elapsed();
    assert!(
        took >= Duration::from_millis(500),
        "the upload was cut before the grace ran out ({took:?})"
    );
    assert!(
        took < Duration::from_secs(20),
        "a stalled upload held the close ({took:?})"
    );
    assert_eq!(parts_under(&home.book_dir(A)), Vec::<String>::new());
    assert!(
        !tree_of(&home.book_dir(A))
            .keys()
            .any(|k| k.contains(&id) && k.ends_with(".bin")),
        "bytes of a cut upload were kept"
    );
    let said = raw.until_closed(Duration::from_secs(10));
    assert!(said.contains("cut_by_close"), "{said}");
    assert_eq!(cut_records(&home, A, "upload").len(), 1);
    stands_still(
        &home,
        A,
        Duration::from_millis(300),
        "after the close was answered",
    );
    assert_eq!(open_book(&home, A), OpenOutcome::Opened);
    let mut again = Wire::new(&home, A);
    again.initialize("after");
    draft(&mut again, "after");
    access_of(&home, A);
    assert!(server.close().success());
    access_of(&home, A);
}

/// The same for an MCP call whose body never finishes arriving: no tool saw
/// it, so nothing is in the journal; the close is not held; the record says
/// the close cut it.
#[cfg(unix)]
#[test]
fn a_stalled_mcp_body_cannot_hold_a_close_and_nothing_of_it_is_journaled() {
    let home = Home::new("close-stalled-mcp");
    let server = Server::start_with(&home, &["--answer-grace", "600", "--timers", "10"]);
    home.create(A, "USD");
    let mut w = Wire::new(&home, A);
    w.initialize("caller");
    draft(&mut w, "before");
    let mut raw = raw::Raw::connect(&home, A);
    let body = json!({"jsonrpc": "2.0", "id": 9, "method": "tools/call", "params": {
        "name": "books.draft_entry",
        "arguments": {"date": "2026-03-01", "description": "never finished"}}})
    .to_string();
    raw.start_inside(
        "POST",
        "mcp",
        &[
            ("content-type", "application/json"),
            ("accept", "application/json, text/event-stream"),
            ("mcp-session-id", w.session().unwrap()),
            ("mcp-protocol-version", w.version().unwrap()),
        ],
        body.as_bytes(),
        body.len() / 2,
    );
    let started = std::time::Instant::now();
    assert_eq!(close_book(&home, A), CloseOutcome::Closed);
    let took = started.elapsed();
    assert!(
        took >= Duration::from_millis(500),
        "cut too early ({took:?})"
    );
    assert!(took < Duration::from_secs(20), "held by a stall ({took:?})");
    let said = raw.until_closed(Duration::from_secs(10));
    assert!(said.contains("cut_by_close"), "{said}");
    assert!(
        !files_of(&home, A)
            .0
            .iter()
            .any(|l| l.payload["description"] == json!("never finished")),
        "a call whose body never arrived was journaled"
    );
    assert_eq!(cut_records(&home, A, "connect").len(), 1);
    assert_eq!(open_book(&home, A), OpenOutcome::Opened);
    let mut again = Wire::new(&home, A);
    again.initialize("after");
    draft(&mut again, "after");
    access_of(&home, A);
    assert!(server.close().success());
}

/// A slow upload that is still arriving, and finishes within the grace, is
/// not cut: it is kept and answered, as without a close. Driven by
/// conditions, not by time: the grace (a minute) is far past anything the
/// test does, and the rest of the bytes are sent only after the close has
/// begun (the book is listed closed), so the one thing this proves is that
/// the cut waits for the grace, never firing at the start of the close (it
/// fails with the cut set to fire at once). Its mirror, that the cut does
/// fire when the grace is out, is
/// `an_upload_still_arriving_when_the_grace_runs_out_is_cut_however_active_it_is`.
#[cfg(unix)]
#[test]
fn a_slow_upload_that_finishes_within_the_grace_is_not_cut() {
    let home = Home::new("close-slow-upload");
    let server = Server::start_with(&home, &["--answer-grace", "60000", "--timers", "10"]);
    home.create(A, "USD");
    let mut w = Wire::new(&home, A);
    w.initialize("uploader");
    let bytes: Vec<u8> = (0..200_000u32).map(|n| (n % 247) as u8).collect();
    let (id, mut raw) = half_sent_upload(&home, &mut w, A, &bytes);
    let closing = std::thread::spawn({
        let admin = home.run().join("admin.sock");
        move || ledger_mcp::admin::call(&admin, Request::CloseBook { id: A.into() }).unwrap()
    });
    wait_for("the book is listed closed", Duration::from_secs(30), || {
        listed(&home, A).1
    });
    for chunk in bytes[bytes.len() / 2..].chunks(25_000) {
        std::thread::sleep(Duration::from_millis(150));
        raw.finish(chunk);
    }
    let (status, head) = raw.head();
    assert_eq!(status, 200, "{head}");
    match closing.join().unwrap() {
        Response::CloseBook { outcome, .. } => assert_eq!(outcome, CloseOutcome::Closed),
        other => panic!("{other:?}"),
    }
    assert!(cut_records(&home, A, "upload").is_empty());
    assert!(
        tree_of(&home.book_dir(A))
            .keys()
            .any(|k| k.ends_with(&format!("{id}.bin"))),
        "the bytes were kept"
    );
    assert!(server.close().success());
}

/// A call whose body is whole and that is running when the grace runs out
/// is not cut: it ends, is answered and recorded as having run, and only
/// then is the close answered. The call is an ordinary one (`books.attach_
/// receipt`, which reads its upload: bounded by one request, so a close waits
/// for it instead of stopping it), held inside its read of the file, which
/// is a named pipe nothing writes to yet. What this proves is that the close
/// waits for such a call and does not kill it; that a body read to its end
/// is never cut (the line of INVARIANT 15) is proved at its seat, by
/// `local::tests::a_body_read_to_its_end_is_never_cut_and_one_not_yet_is`:
/// this test passes even with a cut that marks every request inside (the
/// call runs in the session's task, past the handler's own check for a cut).
/// (A backup asked for is no longer this call: it is stoppable, and a close
/// stops it: `a_close_stops_a_running_backup_call_and_says_so`.)
#[cfg(unix)]
#[test]
fn a_running_call_is_never_cut_when_the_grace_runs_out() {
    use std::os::unix::fs::OpenOptionsExt;
    let home = Home::new("close-running-call");
    let server = Server::start_with(&home, &["--answer-grace", "400", "--timers", "20"]);
    home.create(A, "USD");
    let mut w = Wire::new(&home, A);
    w.initialize("owner");
    let bytes = b"%PDF-1.4 a receipt, read by a call that is held".to_vec();
    let up = w.upload(&bytes, "r.pdf", "application/pdf", "receipt");
    // The call takes its file (renaming it to `.taken`) and reads it: make
    // what it reads a named pipe, so it waits there for a writer.
    let uploads = home.book_dir(A).join("uploads");
    let bin = uploads.join(format!("{up}.bin"));
    std::fs::remove_file(&bin).unwrap();
    assert!(
        std::process::Command::new("mkfifo")
            .arg(&bin)
            .status()
            .unwrap()
            .success()
    );
    let mut raw = raw::Raw::connect(&home, A);
    let body = json!({"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {
        "name": "books.attach_receipt", "arguments": {"upload_id": up}}})
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
    // Inside its read: a writer can open the pipe without blocking only
    // once a reader has it open. (The writer is kept, to feed it later.)
    let taken = uploads.join(format!("{up}.taken"));
    let mut feed = None;
    wait_for(
        "the call is reading its file",
        Duration::from_secs(60),
        || match std::fs::OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&taken)
        {
            Ok(f) => {
                feed = Some(f);
                true
            }
            Err(_) => false,
        },
    );
    let closing = std::thread::spawn({
        let admin = home.run().join("admin.sock");
        move || ledger_mcp::admin::call(&admin, Request::CloseBook { id: A.into() }).unwrap()
    });
    wait_for("the book is listed closed", Duration::from_secs(30), || {
        listed(&home, A).1
    });
    // A negative window well past the grace (400 ms): the call is still
    // running, and not cut. It can only fail if the close was answered, never
    // because the machine was slow.
    std::thread::sleep(Duration::from_millis(1200));
    assert!(
        !closing.is_finished(),
        "the close didn't wait for a running call"
    );
    {
        use std::io::Write;
        let mut feed = feed.take().unwrap();
        feed.write_all(&bytes).unwrap();
    }
    let (status, said) = raw.answer();
    assert_eq!(status, 200, "{said}");
    let _ = closing.join().unwrap();
    assert!(cut_records(&home, A, "books.attach_receipt").is_empty());
    assert!(cut_records(&home, A, "connect").is_empty());
    assert!(
        access_of(&home, A)
            .iter()
            .any(|r| r.access.subject == "books.attach_receipt"
                && r.access.result == ledger_store::access::Outcome::Ok),
        "the call that ran is on record as having run: {said}"
    );
    drop(raw);
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
            "15000",
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
    // (The grace is longer than the 5 seconds watched below: an upload
    // that finishes within it is waited for, never cut.)
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
    // A call for the books being replaced is told they aren't open (503,
    // the words, the closed header), as in a close; never that there are
    // no such books (404), which they would be told if they were taken
    // out of the list while the restore waits (INVARIANT 10).
    let late_call = || {
        let mut late = Wire::with_token(&home, A, Some(w.token().unwrap().to_string()));
        late.send(&json!({"jsonrpc": "2.0", "id": 1, "method": "ping"}));
        late.refused.clone().map(|r| (r.status, r.body))
    };
    wait_for(
        "the replaced books are let go",
        Duration::from_secs(30),
        || match late_call() {
            Some((404, body)) => panic!("books being restored were `no such books`: {body}"),
            Some((503, body)) => {
                assert_eq!(body, closed_words(A));
                true
            }
            _ => false,
        },
    );
    // The restore (a clone and a check of the backup, which take a second
    // or so) is not answered while the upload is half received: watched
    // for five seconds, in which a restore that doesn't wait would finish.
    // Every call meanwhile gets the same answer.
    let watched = std::time::Instant::now() + Duration::from_secs(5);
    while std::time::Instant::now() < watched {
        assert!(
            !restoring.is_finished(),
            "the restore was answered with an upload half received"
        );
        assert_eq!(
            late_call(),
            Some((503, closed_words(A))),
            "a call while the books were being restored"
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
    // A negative window (it can only fail if something moved).
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(moved(&at_answer, &tree_of(&legacy)), Vec::<String>::new());
    assert!(server.close().success());
}

/// A restore over an open book is held by an upload that stalls no longer
/// than the grace: the upload is cut (no part file in `legacy/`, its record
/// in the replaced book's log says so), and the restore is answered.
#[cfg(unix)]
#[test]
fn a_restore_over_an_open_book_is_not_held_by_a_stalled_upload() {
    let home = Home::new("restore-stalled");
    let server = Server::start_with(
        &home,
        &[
            "--answer-grace",
            "600",
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
    let bytes: Vec<u8> = (0..200_000u32).map(|n| (n % 239) as u8).collect();
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
    let started = std::time::Instant::now();
    let restored = match home.admin(Request::Restore {
        id: A.into(),
        source,
        replace: true,
        confirmed: true,
        expect_head: Some(preview.head),
    }) {
        Response::Restored(r) => r,
        other => panic!("{other:?}"),
    };
    assert!(started.elapsed() < Duration::from_secs(30));
    assert!(restored.replaced.is_some(), "{restored:?}");
    let said = raw.until_closed(Duration::from_secs(10));
    assert!(said.contains("cut_by_close"), "{said}");
    let legacy = home.root().join("legacy");
    assert_eq!(parts_under(&legacy), Vec::<String>::new());
    let old = std::fs::read_dir(&legacy)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let (records, _) = ledger_store::access::read(&old).expect("the replaced book's log is whole");
    assert!(
        records.iter().any(|r| r.access.action == "upload"
            && r.access.subject == id
            && r.access.result
                == ledger_store::access::Outcome::Refused("cut_by_close".to_string())),
        "{records:?}"
    );
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

/// The mirror of `a_slow_upload_that_finishes_within_the_grace_is_not_cut`:
/// an upload whose client is not stalled at all, sending steadily, is cut
/// the moment the grace has run out, because its body isn't whole then (the
/// line is the body's end, INVARIANT 15). Driven by conditions: the client
/// sends until the server closes the connection, and the body is far too
/// large to be whole within the grace (the test fails if the upload is
/// let finish: with the cut removed it is kept, answered 200, and the close
/// answered ten seconds late).
#[cfg(unix)]
#[test]
fn an_upload_still_arriving_when_the_grace_runs_out_is_cut_however_active_it_is() {
    let home = Home::new("close-active-upload");
    let server = Server::start_with(&home, &["--answer-grace", "600", "--timers", "10"]);
    home.create(A, "USD");
    let mut w = Wire::new(&home, A);
    w.initialize("uploader");
    let bytes: Vec<u8> = (0..3_000_000u32).map(|n| (n % 241) as u8).collect();
    let (id, mut raw) = half_sent_upload(&home, &mut w, A, &bytes);
    let closing = std::thread::spawn({
        let admin = home.run().join("admin.sock");
        move || ledger_mcp::admin::call(&admin, Request::CloseBook { id: A.into() }).unwrap()
    });
    wait_for("the book is listed closed", Duration::from_secs(30), || {
        listed(&home, A).1
    });
    // Steady: 8 KB every 25 ms (about 320 KB/s: the rest takes ten seconds).
    let begun = std::time::Instant::now();
    let mut sent = bytes.len() / 2;
    while sent < bytes.len() {
        let piece = &bytes[sent..(sent + 8192).min(bytes.len())];
        if !raw.try_finish(piece) {
            break;
        }
        sent += piece.len();
        std::thread::sleep(Duration::from_millis(25));
        assert!(
            begun.elapsed() < Duration::from_secs(8),
            "an upload that was still arriving was never cut"
        );
    }
    assert!(
        sent < bytes.len(),
        "the whole upload went in before the cut"
    );
    assert!(
        begun.elapsed() >= Duration::from_millis(400),
        "cut before the grace ran out ({:?})",
        begun.elapsed()
    );
    let said = raw.until_closed(Duration::from_secs(10));
    assert!(said.contains("cut_by_close"), "{said}");
    match closing.join().unwrap() {
        Response::CloseBook { outcome, .. } => assert_eq!(outcome, CloseOutcome::Closed),
        other => panic!("{other:?}"),
    }
    assert_eq!(parts_under(&home.book_dir(A)), Vec::<String>::new());
    assert!(
        !tree_of(&home.book_dir(A))
            .keys()
            .any(|k| k.contains(&id) && k.ends_with(".bin")),
        "bytes of a cut upload were kept"
    );
    assert_eq!(cut_records(&home, A, "upload").len(), 1);
    assert!(server.close().success());
}

/// The server's own close is the book's close, all books together
/// (local.rs INVARIANT 15): with an upload stalled half way on one book and
/// an MCP call's body stalled half way on another, the server told to close
/// (its standard input ends) cuts both one grace after the close began,
/// exits 0 within a bound, leaves no part file, journals nothing of the
/// call, and its records say `cut_by_close`.
#[cfg(unix)]
#[test]
fn the_servers_own_close_cuts_stalled_requests_and_exits_zero_in_time() {
    let home = Home::new("server-close-stalled");
    let server = Server::start_with(&home, &["--answer-grace", "600", "--timers", "10"]);
    home.create(A, "USD");
    home.create(B, "USD");
    let mut wa = Wire::new(&home, A);
    wa.initialize("uploader");
    let bytes: Vec<u8> = (0..200_000u32).map(|n| (n % 251) as u8).collect();
    let (id, mut raw_upload) = half_sent_upload(&home, &mut wa, A, &bytes);
    let mut wb = Wire::new(&home, B);
    wb.initialize("caller");
    draft(&mut wb, "before");
    let mut raw_call = raw::Raw::connect(&home, B);
    let body = json!({"jsonrpc": "2.0", "id": 9, "method": "tools/call", "params": {
        "name": "books.draft_entry",
        "arguments": {"date": "2026-03-01", "description": "never finished"}}})
    .to_string();
    raw_call.start_inside(
        "POST",
        "mcp",
        &[
            ("content-type", "application/json"),
            ("accept", "application/json, text/event-stream"),
            ("mcp-session-id", wb.session().unwrap()),
            ("mcp-protocol-version", wb.version().unwrap()),
        ],
        body.as_bytes(),
        body.len() / 2,
    );
    let started = std::time::Instant::now();
    let status = server.close();
    let took = started.elapsed();
    assert_eq!(status.code(), Some(0), "{status:?}");
    assert!(
        took >= Duration::from_millis(500),
        "cut before the grace ran out ({took:?})"
    );
    // The bound: the grace (600 ms, the cut), the answers' grace (600 ms),
    // and the linger and the HTTP side's end (3 s and 5 s at most).
    assert!(took < Duration::from_secs(12), "{took:?}");
    assert!(
        raw_upload
            .until_closed(Duration::from_secs(10))
            .contains("cut_by_close")
    );
    assert!(
        raw_call
            .until_closed(Duration::from_secs(10))
            .contains("cut_by_close")
    );
    assert_eq!(parts_under(&home.book_dir(A)), Vec::<String>::new());
    assert!(
        !tree_of(&home.book_dir(A))
            .keys()
            .any(|k| k.contains(&id) && k.ends_with(".bin")),
        "bytes of a cut upload were kept"
    );
    assert_eq!(cut_records(&home, A, "upload").len(), 1);
    assert_eq!(cut_records(&home, B, "connect").len(), 1);
    assert!(
        !files_of(&home, B)
            .0
            .iter()
            .any(|l| l.payload["description"] == json!("never finished")),
        "a call whose body never arrived was journaled"
    );
    // Every log is whole, and the books open again.
    access_of(&home, A);
    access_of(&home, B);
    ledger_store::Store::open(&home.book_dir(A)).expect("the books open again");
    ledger_store::Store::open(&home.book_dir(B)).expect("the books open again");
}

/// A timer that took a book's handles before the close and acts on them
/// after it does nothing to the book (local.rs INVARIANT 13). The knob
/// `--timer-hold` holds a timer between its taking the handles and its
/// acting on them; the test closes the book meanwhile, plants what a sweep
/// would take, lets the timer go on, and the tree of the closed book must
/// not move. `held` names the timers held: the sweep, and the backup or the
/// drill (the backup loop is one task: the drill is held only once the
/// write is backed up).
#[cfg(unix)]
fn a_late_timer_does_nothing(name: &str, held_timers: &str) {
    let home = Home::new(name);
    let server = Server::start_with(
        &home,
        &[
            "--timers",
            "5",
            "--timer-hold",
            "--backup-quiet",
            "0",
            "--drill-first",
            "0",
            "--answer-grace",
            "300",
        ],
    );
    home.create(A, "USD");
    let run = home.run();
    let seen = || std::fs::read_to_string(run.join("timer-held")).unwrap_or_default();
    let all_held = || {
        held_timers
            .split(',')
            .all(|t| seen().lines().any(|l| l == t))
    };
    // From the start of the book's work: the timers hold what they take.
    std::fs::write(run.join("timer-hold"), held_timers).unwrap();
    let mut w = Wire::new(&home, A);
    w.initialize("owner");
    let disk = home.dir.path().join("disk");
    std::fs::create_dir_all(&disk).unwrap();
    set_backup_folder(&mut w, &disk);
    w.end();
    wait_for(
        "the timers hold their handles",
        Duration::from_secs(60),
        all_held,
    );
    let dir = home.book_dir(A);
    assert!(matches!(close_book(&home, A), CloseOutcome::Closed));
    for folder in ["uploads", "exports"] {
        std::fs::write(dir.join(folder).join("stray.bin"), b"x").unwrap();
    }
    let at_close = tree_of(&dir);
    // The timers go on, with what they took before the close.
    std::fs::remove_file(run.join("timer-hold")).unwrap();
    // (A window in which nothing may move, begun at the release: no wait
    // before it for the timers to get going, which proved nothing.)
    stands_still(&home, A, Duration::from_millis(800), "a late timer");
    assert_eq!(moved(&at_close, &tree_of(&dir)), Vec::<String>::new());
    assert!(dir.join("uploads").join("stray.bin").exists());
    assert!(dir.join("exports").join("stray.bin").exists());
    // Opened again, the timers work on it again.
    assert_eq!(open_book(&home, A), OpenOutcome::Opened);
    wait_for(
        "the sweep takes the stray files",
        Duration::from_secs(30),
        || !dir.join("uploads").join("stray.bin").exists(),
    );
    assert!(server.close().success());
    access_of(&home, A);
    ledger_store::chain::read(
        &dir.join("ops").join("backup.jsonl"),
        "wanderbooks-backup-v1",
    )
    .expect("the backup record is a whole chain");
}

/// The sweep and the backup, each holding its handles across a close.
#[cfg(unix)]
#[test]
fn a_backup_and_a_sweep_that_took_their_handles_before_a_close_do_nothing_after_it() {
    a_late_timer_does_nothing("close-late-backup", "sweep,backup");
}

/// The sweep and the drill, each holding its handles across a close.
#[cfg(unix)]
#[test]
fn a_drill_and_a_sweep_that_took_their_handles_before_a_close_do_nothing_after_it() {
    a_late_timer_does_nothing("close-late-drill", "sweep,drill");
}

/// A close does not wait for a drill, however long the book makes it: the
/// drill is stopped at its next look (a cancel flag), its scratch clone is
/// removed, and the close is answered within its stated bound (local.rs
/// INVARIANT 10: the grace for what hasn't arrived, the grace for the
/// answers, the last backup's bound, the stop of a step, which for a
/// folder target includes the lock settling time), with the last backup
/// made. `--slow-steps` makes each phase of the drill wait a minute, as a
/// book of a million events would make its replay; without the hurry the
/// last backup waits behind the drill and the close says it wasn't whole.
#[cfg(unix)]
#[test]
fn a_close_stops_a_slow_drill_instead_of_waiting_for_it() {
    let home = Home::new("close-slow-drill");
    let (grace, last_backup) = (500u64, 3000u64);
    let server = Server::start_with(
        &home,
        &[
            "--slow-steps",
            "60000",
            "--timers",
            "20",
            "--backup-quiet",
            "0",
            "--drill-first",
            "0",
            "--answer-grace",
            &grace.to_string(),
            "--close-backup",
            &last_backup.to_string(),
        ],
    );
    home.create(A, "USD");
    let disk = home.dir.path().join("disk");
    std::fs::create_dir_all(&disk).unwrap();
    let mut w = Wire::new(&home, A);
    w.initialize("owner");
    set_backup_folder(&mut w, &disk);
    draft(&mut w, "backed up first");
    w.end();
    // The write is backed up, then its first drill begins and sits in its
    // wait: the clone is made, its scratch folder is there.
    let scratch = home.book_dir(A).join("ops").join("drill-folder");
    wait_for("a drill is under way", Duration::from_secs(60), || {
        scratch.exists()
    });
    // A write the close's last backup has to make.
    let mut w = Wire::new(&home, A);
    w.initialize("owner");
    draft(&mut w, "after the drill began");
    w.end();
    let started = std::time::Instant::now();
    let outcome = close_book(&home, A);
    let took = started.elapsed();
    assert_eq!(
        outcome,
        CloseOutcome::Closed,
        "the last backup waited behind the drill"
    );
    // The stated bound, as a sum of its parts: the grace, the answers'
    // grace, the last backup's bound, and the stop of a step (git's kill
    // and its group's end, 2 s; its pipes, 10 s; the lock settling, 1.5 s).
    let bound = Duration::from_millis(2 * grace + last_backup) + Duration::from_millis(13_500);
    assert!(took < bound, "{took:?} past the stated bound {bound:?}");
    assert!(took < Duration::from_secs(8), "{took:?}: the drill ran on");
    assert!(!scratch.exists(), "the stopped drill's clone was left");
    // The last backup took the write.
    let records = std::fs::read_to_string(home.book_dir(A).join("ops").join("backup.jsonl"))
        .unwrap_or_default();
    assert!(
        records.matches("\"why\":\"close\"").count() >= 1,
        "no backup at the close: {records}"
    );
    stands_still(
        &home,
        A,
        Duration::from_millis(400),
        "after the close was answered",
    );
    assert_eq!(open_book(&home, A), OpenOutcome::Opened);
    assert!(server.close().success());
    ledger_store::chain::read(
        &home.book_dir(A).join("ops").join("backup.jsonl"),
        "wanderbooks-backup-v1",
    )
    .expect("the backup record is a whole chain");
}

/// A stand-in `git` that is the real one with a hooks folder of its own
/// (every `core.hooksPath=/dev/null` it is given, in its arguments and in
/// the `--receive-pack` command of a push to a folder, names it instead),
/// where a `reference-transaction` hook waits, while `slow-commit` (for the
/// book's archive) or `slow-push` (for any other repository) exists in the
/// shim folder, at the point where git holds its ref locks. `hooked` has a
/// line per hook that waited. A real git, killed inside real work.
#[cfg(unix)]
fn slow_real_git(home: &Home) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let dir = home.dir.path().join("slow-shim");
    let hooks = dir.join("hooks");
    std::fs::create_dir_all(&hooks).unwrap();
    let real = ["/usr/bin/git", "/bin/git", "/usr/local/bin/git"]
        .into_iter()
        .find(|p| std::path::Path::new(p).is_file())
        .expect("git is installed");
    let hook = hooks.join("reference-transaction");
    std::fs::write(
        &hook,
        "#!/bin/sh\ncat >/dev/null\n[ \"$1\" = prepared ] || exit 0\nd=\"$(dirname \"$0\")/..\"\ncase \"$GIT_DIR $PWD\" in *archive.git*) f=slow-commit ;; *) f=slow-push ;; esac\nif [ -e \"$d/$f\" ]; then\n  echo $$ >> \"$d/hooked\"\n  while [ -e \"$d/$f\" ]; do sleep 0.05; done\nfi\nexit 0\n",
    )
    .unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    let exe = dir.join("git");
    std::fs::write(
        &exe,
        format!(
            "#!/bin/sh\nn=$#\nwhile [ $n -gt 0 ]; do\n  a=\"$1\"; shift\n  case \"$a\" in\n    *core.hooksPath=/dev/null*) a=$(printf '%s' \"$a\" | sed 's|core.hooksPath=/dev/null|core.hooksPath={}|g') ;;\n  esac\n  set -- \"$@\" \"$a\"\n  n=$((n-1))\ndone\nexec {real} \"$@\"\n",
            hooks.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
    dir
}

/// Every `*.lock` file under a bare repository.
#[cfg(unix)]
fn locks_under(repo: &std::path::Path) -> Vec<String> {
    tree_of(repo)
        .into_keys()
        .filter(|k| k.ends_with(".lock"))
        .collect()
}

#[cfg(unix)]
fn fsck_of(repo: &std::path::Path) {
    let out = std::process::Command::new("git")
        .arg(format!("--git-dir={}", repo.display()))
        .args(["fsck", "--strict"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{} isn't whole: {}",
        repo.display(),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A close that stops real git inside a commit, and inside a push to a
/// folder, leaves no lock for anyone to remove: git is sent SIGTERM and
/// removes its own locks, so there is nothing for the server to clean up
/// (the backup's record holds no `lock_removed` or `lock_left`), the book
/// opened again at once backs up with no step by hand, and both
/// repositories pass `git fsck` (ledger_store::backup INVARIANT 5).
#[cfg(unix)]
#[test]
fn a_close_that_stops_git_in_a_commit_or_a_push_leaves_no_lock_and_the_next_backup_succeeds() {
    let home = Home::new("close-git-locks");
    let shim = slow_real_git(&home);
    let server = server_with_shim(
        &home,
        &shim,
        &[
            "--close-backup",
            "1500",
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
    draft(&mut w, "to be backed up inside a commit");
    w.end();
    let archive = home.book_dir(A).join("archive.git");
    let target = disk.join(format!("{A}.git"));
    let record = || {
        std::fs::read_to_string(home.book_dir(A).join("ops").join("backup.jsonl"))
            .unwrap_or_default()
    };
    // 0. Inside the making of the archive itself (`git init`): a lock in
    // a repository that isn't one yet stops every later `init`.
    std::fs::write(shim.join("slow-commit"), b"").unwrap();
    let outcome = close_book(&home, A);
    assert!(
        matches!(outcome, CloseOutcome::ClosedBackupIncomplete(_)),
        "{outcome:?}"
    );
    assert!(
        shim.join("hooked").exists(),
        "git never reached its ref update"
    );
    assert_eq!(locks_under(&archive), Vec::<String>::new());
    assert!(
        !record().contains("lock_removed") && !record().contains("lock_left"),
        "git removed its own locks: nothing was cleaned up here: {}",
        record()
    );
    std::fs::remove_file(shim.join("slow-commit")).unwrap();
    std::fs::remove_file(shim.join("hooked")).unwrap();
    assert_eq!(open_book(&home, A), OpenOutcome::Opened);
    let mut w = Wire::new(&home, A);
    w.initialize("owner");
    draft(&mut w, "to be backed up inside a commit");
    w.end();
    match home.admin(Request::BackUpNow {
        id: A.into(),
        new_disk: false,
    }) {
        Response::BackedUp { targets, .. } => {
            assert!(targets.iter().all(|t| t.problem.is_none()), "{targets:?}");
        }
        other => panic!("{other:?}"),
    }
    let mut w = Wire::new(&home, A);
    w.initialize("owner");
    draft(&mut w, "to be committed inside a commit");
    w.end();
    // 1. Inside the commit's ref update.
    std::fs::write(shim.join("slow-commit"), b"").unwrap();
    let outcome = close_book(&home, A);
    assert!(
        matches!(outcome, CloseOutcome::ClosedBackupIncomplete(_)),
        "{outcome:?}"
    );
    assert!(
        shim.join("hooked").exists(),
        "git never reached its ref update"
    );
    assert_eq!(locks_under(&archive), Vec::<String>::new());
    assert!(
        !record().contains("lock_removed") && !record().contains("lock_left"),
        "git removed its own locks: nothing was cleaned up here: {}",
        record()
    );
    std::fs::remove_file(shim.join("slow-commit")).unwrap();
    std::fs::remove_file(shim.join("hooked")).unwrap();
    assert_eq!(open_book(&home, A), OpenOutcome::Opened);
    let status = home.admin(Request::BackUpNow {
        id: A.into(),
        new_disk: false,
    });
    match &status {
        Response::BackedUp { targets, .. } => {
            assert!(targets.iter().all(|t| t.problem.is_none()), "{status:?}");
        }
        other => panic!("{other:?}"),
    }
    fsck_of(&archive);
    fsck_of(&target);
    // 2. Inside the push's ref update at the folder.
    let mut w = Wire::new(&home, A);
    w.initialize("owner");
    draft(&mut w, "to be pushed inside a push");
    w.end();
    std::fs::write(shim.join("slow-push"), b"").unwrap();
    let outcome = close_book(&home, A);
    assert!(
        matches!(outcome, CloseOutcome::ClosedBackupIncomplete(_)),
        "{outcome:?}"
    );
    assert!(
        shim.join("hooked").exists(),
        "git never reached its ref update"
    );
    assert_eq!(locks_under(&target), Vec::<String>::new());
    assert_eq!(locks_under(&archive), Vec::<String>::new());
    assert!(
        !record().contains("lock_removed") && !record().contains("lock_left"),
        "git removed its own locks: nothing was cleaned up here: {}",
        record()
    );
    std::fs::remove_file(shim.join("slow-push")).unwrap();
    assert_eq!(open_book(&home, A), OpenOutcome::Opened);
    let status = home.admin(Request::BackUpNow {
        id: A.into(),
        new_disk: false,
    });
    match &status {
        Response::BackedUp { targets, .. } => {
            assert!(targets.iter().all(|t| t.problem.is_none()), "{status:?}");
        }
        other => panic!("{other:?}"),
    }
    fsck_of(&archive);
    fsck_of(&target);
    ledger_store::chain::read(
        &home.book_dir(A).join("ops").join("backup.jsonl"),
        "wanderbooks-backup-v1",
    )
    .expect("the backup record is a whole chain");
    assert!(server.close().success());
}

/// A tool call sent on `w`'s session, its whole body written and its answer
/// not yet read: the connection it went out on.
#[cfg(unix)]
fn call_unanswered(home: &Home, w: &Wire, id: u64, tool: &str, args: Value) -> raw::Raw {
    let mut raw = raw::Raw::connect(home, A);
    let body = json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {
        "name": tool, "arguments": args}})
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
    raw
}

/// Hold every process of the group `pid` leads still (SIGSTOP): a SIGTERM
/// sent to it then stays pending, git can't end on it, and only the kill
/// after the grace ends it (how a git that can't end on SIGTERM, holding
/// its locks, is made on purpose).
#[cfg(unix)]
fn freeze_group_of(pid: u32) {
    let out = std::process::Command::new("ps")
        .args(["-o", "pgid=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    let group: i32 = String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .expect("ps names the group");
    // SAFETY: stopping the group of a git this test's server started.
    assert_eq!(unsafe { libc::kill(-group, libc::SIGSTOP) }, 0);
}

/// The most a close may take in these tests where what it stops would
/// otherwise run for ever (a stall that is never lifted): a guard against a
/// hang, not a measure of speed. That the close ends at all, with the stall
/// still in place, is what proves it stopped the call or job.
#[cfg(unix)]
const NEVER_LONGER: Duration = Duration::from_secs(120);

/// The backup record of `id`, whole.
#[cfg(unix)]
fn backup_record_of(home: &Home, id: &str) -> Vec<serde_json::Map<String, Value>> {
    ledger_store::chain::read(
        &ledger_store::ops::ops_dir(&home.book_dir(id)).join("backup.jsonl"),
        "wanderbooks-backup-v1",
    )
    .expect("the backup record is a whole chain")
    .records
}

/// A backup call that is running (git stalled) when a close begins is
/// STOPPED by it, not waited for: the call is answered, while git is still
/// stalled, that it was stopped because the books are closing; its access
/// record says `refused: stopped_by_close`; the backup records `stopped`
/// and no result of the run's own (nothing it was doing is recorded as
/// done); git and all it started are gone; and the close's own last
/// backup is made as always (local.rs INVARIANT 10).
#[cfg(unix)]
#[test]
fn a_close_stops_a_running_backup_call_and_says_so() {
    let home = Home::new("close-stops-backup-call");
    let shim = shim_git(&home);
    let server = server_with_shim(
        &home,
        &shim,
        &[
            "--close-backup",
            "20000",
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
    draft(&mut w, "moves the head");
    std::fs::write(shim.join("stall"), b"").unwrap();
    let mut call = call_unanswered(&home, &w, 4, "books.back_up_now", json!({}));
    wait_for("the call reached git", Duration::from_secs(60), || {
        shim.join("stalled").exists()
    });
    let started = std::time::Instant::now();
    let closing = std::thread::spawn({
        let admin = home.run().join("admin.sock");
        move || ledger_mcp::admin::call(&admin, Request::CloseBook { id: A.into() }).unwrap()
    });
    // Answered with git still stalled: stopped by the close, not let go.
    let (status, said) = call.answer();
    assert_eq!(status, 200, "{said}");
    assert!(
        said.contains("stopped because the books are closing")
            && said.contains("nothing it was doing is recorded as done"),
        "{said}"
    );
    assert!(shim.join("stall").exists(), "the stall was lifted first");
    // Lifted now, so the close's own last backup can run.
    std::fs::remove_file(shim.join("stall")).unwrap();
    match closing.join().unwrap() {
        Response::CloseBook { outcome, .. } => assert_eq!(outcome, CloseOutcome::Closed),
        other => panic!("{other:?}"),
    }
    assert!(started.elapsed() < NEVER_LONGER);
    let stalled: Vec<u32> = std::fs::read_to_string(shim.join("stalled"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.trim().parse().ok())
        .collect();
    for pid in &stalled {
        assert!(!alive(*pid), "git {pid} still runs");
    }
    let records = access_of(&home, A);
    let mine = records
        .iter()
        .find(|r| r.access.subject == "books.back_up_now")
        .expect("the call is on record");
    assert_eq!(
        mine.access.result,
        ledger_store::access::Outcome::Refused("stopped_by_close".into())
    );
    assert!(mine.access.head_after.is_none());
    let record = backup_record_of(&home, A);
    assert!(
        record
            .iter()
            .any(|r| r.get("kind") == Some(&json!("stopped"))
                && r.get("why") == Some(&json!("asked"))),
        "{record:?}"
    );
    assert!(
        !record
            .iter()
            .any(|r| r.get("why") == Some(&json!("asked"))
                && r.get("kind") != Some(&json!("stopped"))),
        "the stopped run recorded a result of its own: {record:?}"
    );
    assert!(server.close().success());
}

/// A verification and a package that are running when a close begins are
/// stopped by it, each answering that it was stopped because the books are
/// closing, each on record as `refused: stopped_by_close`, and the journal
/// untouched. Each is held as it begins by the server's test knob
/// (`--timer-hold`: `call:books.verify`, `call:books.export_cpa_package`),
/// which the close's start releases; the stop is then at the call's first
/// look.
#[cfg(unix)]
#[test]
fn a_close_stops_a_running_verification_and_a_running_package_and_says_so() {
    let home = Home::new("close-stops-verify-package");
    let server = Server::start_with(
        &home,
        &[
            "--timer-hold",
            "--timers",
            "20",
            "--backup-quiet",
            "3600000",
        ],
    );
    home.create(A, "USD");
    let run = home.run();
    let mut w = Wire::new(&home, A);
    w.initialize("owner");
    let mut other = Wire::new(&home, A);
    other.initialize("packager");
    let held = ["call:books.verify", "call:books.export_cpa_package"];
    std::fs::write(run.join("timer-hold"), held.join(",")).unwrap();
    let mut verify = call_unanswered(&home, &w, 4, "books.verify", json!({}));
    let mut package = call_unanswered(
        &home,
        &other,
        5,
        "books.export_cpa_package",
        json!({"cash_accounts": ["1000"]}),
    );
    let seen = || std::fs::read_to_string(run.join("timer-held")).unwrap_or_default();
    wait_for("both calls are held", Duration::from_secs(60), || {
        held.iter().all(|h| seen().lines().any(|l| l == *h))
    });
    let before = files_of(&home, A);
    let started = std::time::Instant::now();
    // The hold is never lifted by the test: the close does it (it is the
    // close that releases the calls, which then stop at their first look).
    let closing = std::thread::spawn({
        let admin = home.run().join("admin.sock");
        move || ledger_mcp::admin::call(&admin, Request::CloseBook { id: A.into() }).unwrap()
    });
    for (what, call) in [("verify", &mut verify), ("package", &mut package)] {
        let (status, said) = call.answer();
        assert_eq!(status, 200, "{what}: {said}");
        assert!(
            said.contains("stopped because the books are closing"),
            "{what}: {said}"
        );
    }
    match closing.join().unwrap() {
        Response::CloseBook { outcome, .. } => assert_eq!(outcome, CloseOutcome::Closed),
        other => panic!("{other:?}"),
    }
    assert!(started.elapsed() < NEVER_LONGER);
    assert_eq!(files_of(&home, A).0.len(), before.0.len());
    let records = access_of(&home, A);
    for tool in ["books.verify", "books.export_cpa_package"] {
        let mine = records
            .iter()
            .find(|r| r.access.subject == tool)
            .unwrap_or_else(|| panic!("{tool} isn't on record"));
        assert_eq!(
            mine.access.result,
            ledger_store::access::Outcome::Refused("stopped_by_close".into()),
            "{tool}"
        );
        assert!(mine.access.head_after.is_none(), "{tool}");
    }
    std::fs::remove_file(run.join("timer-hold")).unwrap();
    assert!(server.close().success());
}

/// How many decisions the journal of `id` holds.
#[cfg(unix)]
fn decisions_of(home: &Home, id: &str) -> usize {
    files_of(home, id)
        .0
        .iter()
        .filter(|l| l.line_type == "decision.recorded")
        .count()
}

/// A package that goes with a decision (a reason to export with drafts open
/// or without an audit) is journaled ahead of its build, because the package
/// counts the decision (its README and every footer carry the journal's
/// event count at the build), so the decision may be on record only for a
/// package that is made: a close that begins before the decision is journaled
/// stops the package, and nothing is journaled; one that begins after it
/// doesn't stop it, and the package is handed over whole (tools/reports.rs
/// `export_cpa_package` INVARIANT 4, local.rs INVARIANT 10). The first call is
/// held as it begins and the second right after its decision is journaled, by
/// the server's test knob (`--timer-hold`), which the close's start releases.
#[cfg(unix)]
#[test]
fn a_close_never_leaves_a_decision_on_record_for_a_package_that_was_not_made() {
    let home = Home::new("close-package-decision");
    let server = Server::start_with(
        &home,
        &[
            "--timer-hold",
            "--timers",
            "20",
            "--backup-quiet",
            "3600000",
        ],
    );
    home.create(A, "USD");
    let run = home.run();
    let args = json!({
        "cash_accounts": ["1000"],
        "allow_open_drafts": {"reason": "the CPA wants it before the receipt comes"}
    });
    let seen = |name: &str| {
        std::fs::read_to_string(run.join("timer-held"))
            .unwrap_or_default()
            .lines()
            .any(|l| l == name)
    };
    let mut w = Wire::new(&home, A);
    w.initialize("owner");
    let r = w.call(
        "books.add_account",
        json!({"code": "1000", "name": "Bank", "account_type": "asset"}),
    );
    assert_eq!(r["result"]["isError"], json!(false), "{r}");
    draft(&mut w, "still open");
    // 1. The close begins before the decision is journaled.
    let at_start = "call:books.export_cpa_package";
    std::fs::write(run.join("timer-hold"), at_start).unwrap();
    let mut package = call_unanswered(&home, &w, 7, "books.export_cpa_package", args.clone());
    wait_for("the package is held", Duration::from_secs(60), || {
        seen(at_start)
    });
    let started = std::time::Instant::now();
    assert_eq!(decisions_of(&home, A), 0);
    let closing = admin_in_thread(&home, Request::CloseBook { id: A.into() });
    let (status, said) = package.answer();
    assert_eq!(status, 200, "{said}");
    assert!(
        said.contains("stopped because the books are closing"),
        "{said}"
    );
    assert_eq!(
        finished("the close", closing),
        Response::CloseBook {
            id: A.into(),
            outcome: CloseOutcome::Closed
        }
    );
    assert!(started.elapsed() < NEVER_LONGER);
    assert_eq!(
        decisions_of(&home, A),
        0,
        "a decision is on record for a package that was stopped"
    );
    let stopped = access_of(&home, A)
        .into_iter()
        .find(|r| r.access.subject == "books.export_cpa_package")
        .expect("the call is on record");
    assert_eq!(
        stopped.access.result,
        ledger_store::access::Outcome::Refused("stopped_by_close".into())
    );
    // 2. The close begins after it: the package is made all the same.
    std::fs::remove_file(run.join("timer-hold")).unwrap();
    assert_eq!(open_book(&home, A), OpenOutcome::Opened);
    let after = "call:books.export_cpa_package:journaled";
    std::fs::write(run.join("timer-hold"), after).unwrap();
    let mut w = Wire::new(&home, A);
    w.initialize("owner");
    let mut package = call_unanswered(&home, &w, 8, "books.export_cpa_package", args);
    wait_for(
        "the package is held after its decision",
        Duration::from_secs(60),
        || seen(after),
    );
    assert_eq!(
        decisions_of(&home, A),
        1,
        "the decision isn't journaled yet"
    );
    let closing = admin_in_thread(&home, Request::CloseBook { id: A.into() });
    let (status, said) = package.answer();
    assert_eq!(status, 200, "{said}");
    assert!(
        !said.contains("stopped because the books are closing") && said.contains("download"),
        "a package with its decision journaled was stopped by a close: {said}"
    );
    assert_eq!(
        finished("the close", closing),
        Response::CloseBook {
            id: A.into(),
            outcome: CloseOutcome::Closed
        }
    );
    assert_eq!(decisions_of(&home, A), 1);
    let made = access_of(&home, A)
        .into_iter()
        .rev()
        .find(|r| r.access.subject == "books.export_cpa_package")
        .expect("the call is on record");
    assert_eq!(made.access.result, ledger_store::access::Outcome::Ok);
    std::fs::remove_file(run.join("timer-hold")).unwrap();
    assert!(server.close().success());
}

/// The files of `books/` byte for byte (the staging and scratch folders
/// among them), but for the books in `live`: those the server has open and
/// works on by itself (its timers write their archives and logs).
#[cfg(unix)]
fn root_without(home: &Home, live: &[&str]) -> BTreeMap<String, (u64, String)> {
    tree_of(&home.root().join("books"))
        .into_iter()
        .filter(|(k, _)| !live.iter().any(|b| k.starts_with(&format!("{b}/"))))
        .collect()
}

/// The jobs on the admin socket that can run long (restore, adopt, create,
/// open), each held inside a phase by the server's test knob (`--timer-hold`,
/// `job:phase`) until the name is gone or a stop is asked. Returns, for each,
/// the request, the held name, what it says it didn't do, and whether it
/// leaves a folder of its own in `books/` while it is held.
#[cfg(unix)]
struct Jobs {
    /// The backup a restore reads (`acme`'s).
    repo: PathBuf,
    restore_head: (u64, String),
    adopt_head: (u64, String),
    data: PathBuf,
    toml: String,
    /// Keeps the adoption's source alive.
    _old: tempfile::TempDir,
}

#[cfg(unix)]
impl Jobs {
    /// Books `A` (backed up to a folder) and `cedar` (closed) exist; the
    /// previews of a restore of that backup and of an adoption are taken.
    fn new(home: &Home) -> Self {
        home.create(A, "USD");
        home.create(CEDAR, "USD");
        assert!(matches!(close_book(home, CEDAR), CloseOutcome::Closed));
        let mut w = Wire::new(home, A);
        w.initialize("owner");
        let repo = backed_up_to_folder(home, &mut w, A);
        w.end();
        let old = tempfile::Builder::new()
            .prefix("wb-close-old-")
            .tempdir()
            .unwrap();
        let data = old.path().join("books-data");
        drop(ledger_store::Store::create(&data, ledger_core::Currency::USD).unwrap());
        let toml = "[company]\nname = \"Acme Widgets LLC\"\n".to_string();
        let restore_head = match home.admin(Request::Restore {
            id: "gamma".into(),
            source: repo.display().to_string(),
            replace: false,
            confirmed: false,
            expect_head: None,
        }) {
            Response::RestorePreview(p) => p.head,
            other => panic!("{other:?}"),
        };
        let mut jobs = Self {
            repo,
            restore_head,
            adopt_head: (0, String::new()),
            data,
            toml,
            _old: old,
        };
        jobs.adopt_head = match home.admin(jobs.adopt(false)) {
            Response::Preview(p) => p.source_head,
            other => panic!("{other:?}"),
        };
        jobs
    }

    fn adopt(&self, confirmed: bool) -> Request {
        Request::Adopt {
            id: "delta".into(),
            source: self.data.clone(),
            settings_toml: Some(self.toml.clone()),
            config_json: None,
            vault_dir: None,
            vault_key: None,
            confirmed,
            expect_head: confirmed.then(|| self.adopt_head.clone()),
            skip_unknown: true,
        }
    }

    /// A restore of the backup as books `id`, over the books that exist
    /// when `replace`.
    fn restore(&self, id: &str, replace: bool) -> Request {
        Request::Restore {
            id: id.into(),
            source: self.repo.display().to_string(),
            replace,
            confirmed: true,
            expect_head: Some(self.restore_head.clone()),
        }
    }
}

#[cfg(unix)]
const CEDAR: &str = "cedar";

/// How many times the server has said it holds `name` (`run/timer-held`).
#[cfg(unix)]
fn times_held(run: &std::path::Path, name: &str) -> usize {
    std::fs::read_to_string(run.join("timer-held"))
        .unwrap_or_default()
        .lines()
        .filter(|l| *l == name)
        .count()
}

/// A job started on its own thread, and waited for until the server holds it
/// inside the phase `name`; the thread that will have its answer.
#[cfg(unix)]
fn job_held_in(
    home: &Home,
    name: &str,
    job: &Request,
    hold: &str,
) -> std::thread::JoinHandle<Response> {
    let run = home.run();
    std::fs::write(run.join("timer-hold"), hold).unwrap();
    let seen = times_held(&run, name);
    let running = std::thread::spawn({
        let admin = run.join("admin.sock");
        let job = job.clone();
        move || ledger_mcp::admin::call(&admin, job).unwrap()
    });
    wait_for(
        &format!("{name} is held inside the job"),
        Duration::from_secs(300),
        || times_held(&run, name) > seen || running.is_finished(),
    );
    if running.is_finished() {
        panic!(
            "{name}: the job ended before it was held: {:?}",
            running.join()
        );
    }
    running
}

/// A job on the admin socket (restore, adopt, create, open) is stopped by a
/// close only when the close is of the book the job works on, or is the
/// server's own (local.rs INVARIANT 10). Stopped, it answers that it was
/// stopped because the books are closing, what it wrote is removed, and the
/// root is byte for byte as it was (the job's staging or scratch folder is
/// there while it is held, and gone after; asked again with no close, it
/// goes through). Each job is held inside a phase by the server's test knob
/// (`--timer-hold`, `job:phase`), which the stop releases; the stop is then at
/// the job's next look.
///
/// A restore over a book, and the opening of a book, are work on that book: a
/// close of it stops them. A create, an adoption and a restore that replaces
/// nothing make a NEW book, which no close of a book is the close of: only the
/// server's stops them (the other test below, of closes that stop nothing).
#[cfg(unix)]
#[test]
fn a_close_stops_the_restore_and_the_open_of_its_own_book_leaving_the_root_as_it_was() {
    let home = Home::new("close-stops-own-jobs");
    let server = Server::start_with(&home, &JOB_ARGS);
    let jobs = Jobs::new(&home);
    let run = home.run();
    // (the held name, the job, what it says it didn't do, the book whose
    // close stops it, whether it leaves a folder of its own in `books/`)
    let table: Vec<(&str, Request, &str, &str, bool)> = vec![
        (
            "restore:copy",
            jobs.restore(A, true),
            "nothing was restored",
            A,
            true,
        ),
        (
            "open:start",
            Request::OpenBook { id: CEDAR.into() },
            "not opened",
            CEDAR,
            false,
        ),
    ];
    // The books open and live while a job is held: not compared (the job's
    // own book, not yet there, is).
    let live = [A, CEDAR];
    for (name, job, said, closes, leaves_a_folder) in table {
        let before = root_without(&home, &live);
        let legacy_before = tree_of(&home.root().join("legacy"));
        let running = job_held_in(&home, name, &job, name);
        if leaves_a_folder {
            assert_ne!(
                root_without(&home, &live),
                before,
                "{name}: the job wrote nothing while held"
            );
        }
        let started = std::time::Instant::now();
        // The close of the book the job works on: it stops the job (it
        // waits for the job's turn otherwise), and the test lifts nothing.
        let closed = close_book(&home, closes);
        assert_eq!(closed, CloseOutcome::WasNotOpen, "{name}");
        assert!(started.elapsed() < NEVER_LONGER, "{name}");
        let answer = running.join().unwrap();
        let text = format!("{answer:?}");
        assert!(
            text.contains("stopped because the books are closing") && text.contains(said),
            "{name}: {text}"
        );
        std::fs::remove_file(run.join("timer-hold")).unwrap();
        assert_eq!(
            moved(&before, &root_without(&home, &live)),
            Vec::<String>::new(),
            "{name}: the root isn't as it was"
        );
        assert_eq!(
            tree_of(&home.root().join("legacy")),
            legacy_before,
            "{name}"
        );
        // Asked again, with nothing closing, it goes through.
        if closes == A {
            assert_eq!(open_book(&home, A), OpenOutcome::Opened, "{name}");
        }
        match (name, home.admin(job)) {
            ("restore:copy", Response::Restored(_)) | ("open:start", Response::OpenBook { .. }) => {
            }
            (name, other) => panic!("{name}, asked again: {other:?}"),
        }
    }
    for id in [A, CEDAR] {
        assert_eq!(listed(&home, id), (true, false, None), "{id}");
    }
    assert!(server.close().success());
}

/// The server's own close stops every job on the admin socket, as a close of
/// one book stops the work on that book: each held inside its phase, the
/// server is closed, and each answers that it was stopped because the books
/// are closing, with the root byte for byte as it was, and the server
/// exits 0.
#[cfg(unix)]
#[test]
fn the_servers_close_stops_every_running_admin_job_leaving_the_root_as_it_was() {
    let home = Home::new("server-close-stops-jobs");
    let mut server = Server::start_with(&home, &JOB_ARGS);
    let jobs = Jobs::new(&home);
    let table: Vec<(&str, Request, &str, bool)> = vec![
        (
            "restore:copy",
            jobs.restore("gamma", false),
            "nothing was restored",
            true,
        ),
        (
            "restore:copy",
            jobs.restore(A, true),
            "nothing was restored",
            true,
        ),
        ("adopt:open", jobs.adopt(true), "nothing was adopted", true),
        (
            "create:start",
            Request::Create {
                id: "epsilon".into(),
                currency: "USD".into(),
            },
            "nothing was created",
            false,
        ),
        (
            "open:start",
            Request::OpenBook { id: CEDAR.into() },
            "not opened",
            false,
        ),
    ];
    // The books a server has open when it is closed write to their folders as
    // they are let go: `acme`, and `gamma` once the first job is asked again.
    let live = [A, "gamma"];
    for (n, (name, job, said, leaves_a_folder)) in table.into_iter().enumerate() {
        let before = root_without(&home, &live);
        let legacy_before = tree_of(&home.root().join("legacy"));
        let running = job_held_in(&home, name, &job, name);
        if leaves_a_folder {
            assert_ne!(
                root_without(&home, &live),
                before,
                "{name}: the job wrote nothing while held"
            );
        }
        let started = std::time::Instant::now();
        // Nothing lifts the hold but the server's close.
        assert!(server.close().success(), "{name}");
        assert!(started.elapsed() < NEVER_LONGER, "{name}");
        let text = format!("{:?}", running.join().unwrap());
        assert!(
            text.contains("stopped because the books are closing") && text.contains(said),
            "{name}: {text}"
        );
        std::fs::remove_file(home.run().join("timer-hold")).unwrap();
        assert_eq!(
            moved(&before, &root_without(&home, &live)),
            Vec::<String>::new(),
            "{name}: the root isn't as it was"
        );
        assert_eq!(
            tree_of(&home.root().join("legacy")),
            legacy_before,
            "{name}"
        );
        server = job_server_again(&home);
        // Asked again, with nothing closing, it goes through (the first time
        // on this server only: the others would change what comes next).
        if n == 0 {
            match home.admin(job) {
                Response::Restored(_) => {}
                other => panic!("{name}, asked again: {other:?}"),
            }
            // Only `acme` is opened again: `cedar` is closed, as the open
            // job after this needs it.
            assert_eq!(listed(&home, CEDAR), (false, true, None));
        }
    }
    assert!(server.close().success());
}

/// An admin request made on its own thread.
#[cfg(unix)]
fn admin_in_thread(home: &Home, request: Request) -> std::thread::JoinHandle<Response> {
    let admin = home.run().join("admin.sock");
    std::thread::spawn(move || ledger_mcp::admin::call(&admin, request).unwrap())
}

/// Wait for a thread to finish, failing (never hanging) if it doesn't.
#[cfg(unix)]
fn finished<T>(what: &str, thread: std::thread::JoinHandle<T>) -> T {
    wait_for(what, Duration::from_secs(120), || thread.is_finished());
    thread.join().unwrap()
}

/// Closing and opening one book never stops, fails, refuses or delays
/// another book's work (local.rs INVARIANT 10, the first rule of the unit),
/// the work on the admin socket included. A restore over `beta`, a create of
/// `epsilon` and an adoption of `delta` are each held in flight (inside a
/// phase, by the server's test knob; they make books one at a time, so the
/// second and third wait for their turn, not yet held, until the one before
/// is let go), and `acme` is closed and opened again while each is: its
/// close is made slow (its backup's git stalls), and during it another book
/// (`cedar`) is opened and closed and answers at once, while the close of
/// `acme` is still not answered. All three jobs finish correctly, none
/// stopped, none told of any error.
#[cfg(unix)]
#[test]
fn closing_and_opening_one_book_never_stops_waits_for_or_refuses_another_books_jobs() {
    let home = Home::new("close-spares-other-jobs");
    let shim = shim_git(&home);
    let mut args = JOB_ARGS.to_vec();
    args.extend(["--close-backup", "120000"]);
    let server = server_with_shim(&home, &shim, &args);
    let jobs = Jobs::new(&home);
    home.create(B, "USD");
    let run = home.run();
    let stalled = || {
        std::fs::read_to_string(shim.join("stalled"))
            .unwrap_or_default()
            .lines()
            .count()
    };
    // One close and reopening of `acme`, while the jobs are held.
    let cycle = |why: &str| {
        let before = stalled();
        std::fs::write(shim.join("stall"), b"").unwrap();
        let closing = admin_in_thread(&home, Request::CloseBook { id: A.into() });
        wait_for(
            &format!("{why}: the close of {A} is in its backup"),
            Duration::from_secs(120),
            || stalled() > before || closing.is_finished(),
        );
        assert!(
            !closing.is_finished(),
            "{why}: the close of {A} was answered with its backup stalled"
        );
        // Another book is opened, and closed, while this close is not
        // answered: neither waits for it, nor is refused.
        let opening = admin_in_thread(&home, Request::OpenBook { id: CEDAR.into() });
        assert_eq!(
            finished(
                &format!("{why}: an open of {CEDAR} during the close of {A}"),
                opening
            ),
            Response::OpenBook {
                id: CEDAR.into(),
                outcome: OpenOutcome::Opened
            },
            "{why}"
        );
        assert!(!closing.is_finished(), "{why}: the close was answered");
        let shutting = admin_in_thread(&home, Request::CloseBook { id: CEDAR.into() });
        assert_eq!(
            finished(
                &format!("{why}: a close of {CEDAR} during the close of {A}"),
                shutting
            ),
            Response::CloseBook {
                id: CEDAR.into(),
                outcome: CloseOutcome::Closed
            },
            "{why}"
        );
        assert!(!closing.is_finished(), "{why}: the close was answered");
        std::fs::remove_file(shim.join("stall")).unwrap();
        assert_eq!(
            finished(&format!("{why}: the close of {A}"), closing),
            Response::CloseBook {
                id: A.into(),
                outcome: CloseOutcome::Closed
            },
            "{why}"
        );
        assert_eq!(open_book(&home, A), OpenOutcome::Opened, "{why}");
    };
    let hold = |names: &[&str]| std::fs::write(run.join("timer-hold"), names.join(",")).unwrap();
    // The restore holds the turn; the other two are asked, and wait for it.
    let restore = jobs.restore(B, true);
    let restoring = job_held_in(
        &home,
        "restore:copy",
        &restore,
        "restore:copy,create:start,adopt:open",
    );
    let create = Request::Create {
        id: "epsilon".into(),
        currency: "USD".into(),
    };
    // (The server says every phase it passes, held or not: the books made
    // so far have passed `create:start`.)
    let seen = |name: &str| times_held(&run, name);
    let base = [
        ("create:start", seen("create:start")),
        ("adopt:open", seen("adopt:open")),
    ];
    let taken = |name: &str| {
        seen(name)
            > base
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, c)| *c)
                .unwrap()
    };
    let queued = [
        ("create:start", admin_in_thread(&home, create)),
        ("adopt:open", admin_in_thread(&home, jobs.adopt(true))),
    ];
    for n in 0..2 {
        cycle(&format!("restore over {B}, round {n}"));
        assert!(!restoring.is_finished(), "the restore was stopped or ended");
        assert!(
            queued.iter().all(|(_, q)| !q.is_finished()),
            "a queued job was stopped or ended"
        );
    }
    assert!(
        !taken("create:start") && !taken("adopt:open"),
        "a job took the turn from the restore"
    );
    // The restore is let go; it finishes.
    hold(&["create:start", "adopt:open"]);
    match finished("the restore over beta", restoring) {
        Response::Restored(r) => assert!(r.problem.is_none(), "{r:?}"),
        other => panic!("the restore: {other:?}"),
    }
    // Then each of the others takes the turn in its time, and is held.
    let mut waiting: Vec<(&str, std::thread::JoinHandle<Response>)> = queued.into_iter().collect();
    while !waiting.is_empty() {
        wait_for(
            "a queued job has its turn",
            Duration::from_secs(300),
            || waiting.iter().any(|(n, _)| taken(n)),
        );
        let at = waiting.iter().position(|(n, _)| taken(n)).unwrap();
        let (name, job) = waiting.remove(at);
        assert!(!job.is_finished(), "{name} ended while it was held");
        cycle(name);
        assert!(!job.is_finished(), "{name} was stopped or ended");
        let names: Vec<&str> = waiting.iter().map(|(n, _)| *n).collect();
        hold(&names);
        match (name, finished(name, job)) {
            ("create:start", Response::Created { problem, .. }) => {
                assert!(problem.is_none(), "{problem:?}");
            }
            ("adopt:open", Response::Adopted(a)) => assert!(a.problem.is_none(), "{a:?}"),
            (name, other) => panic!("{name}: {other:?}"),
        }
    }
    for id in [A, B, "epsilon", "delta"] {
        assert_eq!(listed(&home, id), (true, false, None), "{id}");
    }
    assert_eq!(listed(&home, CEDAR), (false, true, None));
    assert!(server.close().success());
}

/// The arguments of the servers the job tests start.
#[cfg(unix)]
const JOB_ARGS: [&str; 7] = [
    "--timer-hold",
    "--timers",
    "20",
    "--backup-quiet",
    "3600000",
    "--answer-grace",
    "300",
];

/// A server again, on books made by an earlier one: only `acme` open, as it
/// was left (the other books of the root are closed).
#[cfg(unix)]
fn job_server_again(home: &Home) -> Server {
    let mut args = JOB_ARGS.to_vec();
    args.extend(["--open", A]);
    Server::start_with(home, &args)
}

/// A git that can't end on SIGTERM (its group frozen here, so the signal
/// stays pending) is killed once the grace has run out, and the lock it
/// leaves in a backup FOLDER is the owner's: the server never removes it. It
/// is recorded with its whole path, the target's backup is marked failed with
/// words naming the file and saying when it may be removed, the close says
/// its last backup wasn't whole, and the status (`books.backup`) says the
/// same after the book is opened again. The next backup, meeting the lock,
/// fails by git's own words and leaves the file; once the owner removes it
/// the backup goes on, and both repositories pass `git fsck`.
#[cfg(unix)]
#[test]
fn a_lock_a_kill_leaves_in_the_backup_folder_is_reported_and_never_removed_by_the_server() {
    let home = Home::new("close-lock-left");
    let shim = slow_real_git(&home);
    // A bound long enough that the test freezes git before the close's own
    // bound runs out: the freeze is made on the condition that git is
    // inside the push, not on a guess at how long that takes.
    let server = server_with_shim(
        &home,
        &shim,
        &[
            "--close-backup",
            "8000",
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
    draft(&mut w, "the first backup");
    match home.admin(Request::BackUpNow {
        id: A.into(),
        new_disk: false,
    }) {
        Response::BackedUp { targets, .. } => {
            assert!(targets.iter().all(|t| t.problem.is_none()), "{targets:?}");
        }
        other => panic!("{other:?}"),
    }
    draft(&mut w, "to be pushed inside a push");
    let target = disk.join(format!("{A}.git"));
    let lock = target.join("refs").join("heads").join("main.lock");
    // Another writer's lock, there before our push: never touched, never
    // named.
    let another = target.join("refs").join("heads").join("another.lock");
    std::fs::write(&another, b"").unwrap();
    std::fs::write(shim.join("slow-push"), b"").unwrap();
    let closing = std::thread::spawn({
        let admin = home.run().join("admin.sock");
        move || ledger_mcp::admin::call(&admin, Request::CloseBook { id: A.into() }).unwrap()
    });
    wait_for("git is inside the push", Duration::from_secs(60), || {
        shim.join("hooked").exists() && lock.exists()
    });
    let hooked: u32 = std::fs::read_to_string(shim.join("hooked"))
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    freeze_group_of(hooked);
    let outcome = match closing.join().unwrap() {
        Response::CloseBook { outcome, .. } => outcome,
        other => panic!("{other:?}"),
    };
    let why = match outcome {
        CloseOutcome::ClosedBackupIncomplete(why) => why,
        other => panic!("a backup that left a lock was answered as {other:?}"),
    };
    let lock_text = lock.display().to_string();
    assert!(why.contains(&lock_text), "{why}");
    assert!(
        why.contains("nothing else is using the backup folder"),
        "{why}"
    );
    assert!(!why.contains("another.lock"), "{why}");
    assert!(!why.contains(&home.root().display().to_string()), "{why}");
    // Left where it was: the server deleted nothing in the owner's folder.
    assert!(lock.exists(), "a file in the owner's folder was deleted");
    assert!(another.exists());
    let record = backup_record_of(&home, A);
    assert!(
        record
            .iter()
            .any(|r| r.get("kind") == Some(&json!("lock_left"))
                && r.get("repo") == Some(&json!("folder"))
                && r.get("path") == Some(&json!(lock_text))),
        "{record:?}"
    );
    assert!(
        !record
            .iter()
            .any(|r| r.get("kind") == Some(&json!("lock_removed"))
                && r.get("repo") == Some(&json!("folder"))),
        "{record:?}"
    );
    // Opened again, the status says it, in words that name the file.
    std::fs::remove_file(shim.join("slow-push")).unwrap();
    assert_eq!(open_book(&home, A), OpenOutcome::Opened);
    let mut w = Wire::new(&home, A);
    w.initialize("owner");
    let status = w.call("books.backup", json!({}));
    let shown = status["result"]["structuredContent"]["targets"][0]["last_error"]
        .as_str()
        .unwrap_or_else(|| panic!("the status names no failure: {status}"))
        .to_string();
    assert!(shown.contains(&lock_text), "{shown}");
    assert!(
        shown.contains("nothing else is using the backup folder"),
        "{shown}"
    );
    // With the lock still there the next backup fails (git's own words) and
    // the file stays; once the owner removes it, it goes on.
    match home.admin(Request::BackUpNow {
        id: A.into(),
        new_disk: false,
    }) {
        Response::BackedUp { targets, .. } => {
            assert!(targets.iter().any(|t| t.problem.is_some()), "{targets:?}");
        }
        other => panic!("{other:?}"),
    }
    assert!(lock.exists(), "a later backup removed it");
    std::fs::remove_file(&lock).unwrap();
    match home.admin(Request::BackUpNow {
        id: A.into(),
        new_disk: false,
    }) {
        Response::BackedUp { targets, .. } => {
            assert!(targets.iter().all(|t| t.problem.is_none()), "{targets:?}");
        }
        other => panic!("{other:?}"),
    }
    assert!(another.exists());
    fsck_of(&home.book_dir(A).join("archive.git"));
    fsck_of(&target);
    assert!(server.close().success());
}
