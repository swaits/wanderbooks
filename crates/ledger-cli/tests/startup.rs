//! Starting the books server on books it can't serve: books another
//! program holds, and books a newer wanderbooks wrote. Oracle: ledger-mcp
//! local.rs INVARIANT 3 (every book is tried before the server says it is
//! ready; one that won't open is reported with why, on its route and in
//! the list, and the others are served) and rules.md B-27 (newer books
//! are refused as newer, never as corrupt, and left as they are).

use common::{Home, Server, Wire};
use ledger_mcp::admin::{Request, Response};
use serde_json::json;
use std::time::{Duration, Instant};

mod common;

/// New books `id` written in `home`'s root by the store itself, before a
/// server opens them; the store, still open.
fn written(home: &Home, id: &str) -> ledger_store::Store {
    use std::os::unix::fs::PermissionsExt;
    let dir = home.book_dir(id);
    let store = ledger_store::Store::create(&dir, ledger_core::Currency::USD).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    store
}

/// What the server's list says of `id`: whether it is open, and why not.
fn listed(home: &Home, id: &str) -> (bool, Option<String>) {
    match home.admin(Request::List) {
        Response::Books(list) => {
            let b = list
                .into_iter()
                .find(|b| b.id == id)
                .unwrap_or_else(|| panic!("{id} isn't listed"));
            (b.open, b.problem)
        }
        other => panic!("{other:?}"),
    }
}

/// A request to `id`'s route is refused before it runs, with why.
fn refused_at_the_door(home: &Home, id: &str, why: &str) {
    let mut wire = Wire::with_token(home, id, None);
    wire.send(
        &json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": common::PROTOCOL, "capabilities": {},
        "clientInfo": {"name": "t", "version": "0"}}}),
    );
    let refusal = wire.refused.clone().expect("refused");
    assert!(refusal.not_run, "{refusal:?}");
    assert!(refusal.body.contains(why), "{refusal:?}");
    home.assert_hidden(&refusal.body);
}

/// A writer that keeps the books is a conflict: the server reports those
/// books not open, with why, serves the others, and never hangs on them.
#[test]
fn a_held_lock_is_refused_not_waited_on_forever() {
    let home = Home::new("start-locked");
    let _holder = written(&home, "acme");
    drop(written(&home, "beta"));
    let start = Instant::now();
    let server = Server::start(&home);
    assert!(
        start.elapsed() < Duration::from_secs(60),
        "the server waited on a held lock"
    );
    let (open, problem) = listed(&home, "acme");
    assert!(!open);
    let problem = problem.expect("why");
    assert!(
        problem.contains("another program has these books open"),
        "{problem}"
    );
    refused_at_the_door(&home, "acme", "another program has these books open");
    // The others are served.
    assert_eq!(listed(&home, "beta"), (true, None));
    let mut beta = Wire::new(&home, "beta");
    beta.initialize("t");
    let chart = beta.call("books.chart", json!({}));
    assert_eq!(chart["result"]["isError"], json!(false), "{chart}");
    assert!(server.close().success());
}

/// Books written by a newer wanderbooks: the server won't open them, and
/// says to update it, not that the books are corrupt (rules.md B-27); it
/// leaves them as they are.
#[test]
fn newer_books_are_refused_as_newer_not_corrupt() {
    let home = Home::new("start-newer");
    drop(written(&home, "acme"));
    let dir = home.book_dir("acme");
    let mut w = ledger_store::JournalWriter::open(&dir.join(ledger_store::JOURNAL_FILE)).unwrap();
    w.append("journal.format_set", json!({"format": 3, "features": []}))
        .unwrap();
    // The next line chains onto it, so its bytes are checked (B-27).
    w.append(
        "fx.rate_set",
        json!({"currency": "EUR", "date": "2026-10-01", "rate": "1.10"}),
    )
    .unwrap();
    drop(w);
    let journal = std::fs::read(dir.join(ledger_store::JOURNAL_FILE)).unwrap();
    let server = Server::start(&home);
    let (open, problem) = listed(&home, "acme");
    assert!(!open);
    let err = problem.expect("why");
    assert!(
        err.contains("written by a newer version of wanderbooks"),
        "{err}"
    );
    assert!(err.contains("Update wanderbooks"), "{err}");
    assert!(!err.contains("corrupt"), "{err}");
    refused_at_the_door(&home, "acme", "Update wanderbooks");
    assert!(server.close().success());
    assert_eq!(
        std::fs::read(dir.join(ledger_store::JOURNAL_FILE)).unwrap(),
        journal
    );
}
