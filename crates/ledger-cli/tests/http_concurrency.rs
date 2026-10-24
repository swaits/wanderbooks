//! Concurrency: several agents at once against one book's store.
//!
//! The books server gives every session its own service over the book's
//! one shared, locked store (ledger-mcp local.rs INVARIANT 4). Oracle (server.rs INVARIANTS 2 and
//! 5, under contention): clients posting at the same time leave books
//! that verify, a trial balance that balances, every posted entry with
//! its receipt (threshold 0.00), and each draft posted exactly once even
//! when two clients race to post it; a restart replays the same books.

use common::{Client, call};
use serde_json::json;

mod common;

const CLIENTS: usize = 6;
const ENTRIES: usize = 12;

/// Draft a `cents` entry tagged `tag` on `c` (a session with `token`, of
/// the server whose `run/` folder is `run`), with a receipt uploaded for
/// it as an agent uploads one, if `with_receipt`.
async fn draft(
    (c, run, token): (&Client, &std::path::Path, &str),
    tag: &str,
    cents: u64,
    with_receipt: bool,
) -> u64 {
    let e = call(
        c,
        "books.draft_entry",
        json!({"date": "2026-03-01", "description": tag}),
    )
    .await
    .unwrap();
    let id = e["entry_id"].as_u64().unwrap();
    let amount = format!("{}.{:02}", cents / 100, cents % 100);
    for (account, side) in [("6000", "debit"), ("1000", "credit")] {
        call(
            c,
            "books.add_line",
            json!({"entry_id": id, "account": account, "side": side, "amount": amount}),
        )
        .await
        .unwrap();
    }
    if with_receipt {
        let upload = common::upload_on(
            c,
            run,
            common::BOOK,
            token,
            tag.as_bytes(),
            (&format!("{tag}.pdf"), "application/pdf", "receipt"),
        )
        .await;
        call(
            c,
            "books.attach_receipt",
            json!({"entry_id": id, "upload_id": upload}),
        )
        .await
        .unwrap();
    }
    id
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn agents_posting_at_once_leave_whole_books() {
    // Made on a thread of its own: the settings go in through a session
    // with a runtime of its own (common.rs), never inside the test's.
    let books = std::thread::spawn(|| {
        common::Books::with(
            "http-concurrency",
            "USD",
            Some("[books]\nreceipt_threshold = \"0.00\"\n"),
        )
    })
    .join()
    .unwrap();
    let (run, token) = (books.home.run(), books.home.token(common::BOOK));
    let connect = |run: std::path::PathBuf, token: String| async move {
        common::connect_at(&run, common::BOOK, Some(token))
            .await
            .unwrap()
    };
    let setup = connect(run.clone(), token.clone()).await;
    for (code, name, ty) in [
        ("1000", "Cash", "asset"),
        ("3000", "Capital", "equity"),
        ("6000", "Supplies", "expense"),
    ] {
        call(
            &setup,
            "books.add_account",
            json!({"code": code, "name": name, "account_type": ty}),
        )
        .await
        .unwrap();
    }
    // Drafts every client will race to post.
    let mut contested = Vec::new();
    for k in 0..ENTRIES {
        contested.push(
            draft(
                (&setup, &run, &token),
                &format!("contested {k}"),
                100 + k as u64,
                true,
            )
            .await,
        );
    }

    let mut tasks = Vec::new();
    for n in 0..CLIENTS {
        let (run, token) = (run.clone(), token.clone());
        let contested = contested.clone();
        tasks.push(tokio::spawn(async move {
            let c = connect(run.clone(), token.clone()).await;
            let (mut posted, mut refused, mut won) = (0usize, 0usize, 0usize);
            for (k, &race) in contested.iter().enumerate() {
                // A receipted entry posts; a bare one is refused.
                let receipted = k % 3 != 0;
                let id = draft(
                    (&c, &run, &token),
                    &format!("c{n} e{k}"),
                    (n * 1000 + k + 1) as u64,
                    receipted,
                )
                .await;
                match call(
                    &c,
                    "books.post_entry",
                    json!({"entry_id": id, "confirmed": true}),
                )
                .await
                {
                    Ok(_) => posted += 1,
                    Err(_) => refused += 1,
                }
                assert_eq!(posted + refused, k + 1);
                // Race everyone for the contested draft.
                if call(
                    &c,
                    "books.post_entry",
                    json!({"entry_id": race, "confirmed": true}),
                )
                .await
                .is_ok()
                {
                    won += 1;
                }
            }
            let _ = c.cancel().await;
            (posted, refused, won)
        }));
    }
    let (mut posted, mut refused, mut won) = (0, 0, 0);
    for t in tasks {
        let (p, r, w) = t.await.unwrap();
        posted += p;
        refused += r;
        won += w;
    }
    let per_client_bare = (0..ENTRIES).filter(|k| k % 3 == 0).count();
    assert_eq!(
        refused,
        CLIENTS * per_client_bare,
        "every bare entry refused"
    );
    assert_eq!(posted, CLIENTS * (ENTRIES - per_client_bare));
    assert_eq!(won, ENTRIES, "each contested draft posted exactly once");

    let v = call(&setup, "books.verify", json!({})).await.unwrap();
    assert_eq!(v["status"], "ok", "{v}");
    let tb = call(&setup, "books.trial_balance", json!({}))
        .await
        .unwrap();
    assert_eq!(tb["balanced"], true, "{tb}");
    // Every posted entry has its receipt (threshold 0.00).
    for id in 1..=(ENTRIES + CLIENTS * ENTRIES) as u64 {
        let e = call(&setup, "books.get_entry", json!({"entry_id": id}))
            .await
            .unwrap();
        if e["status"] == "posted" {
            assert!(
                e["receipts"].as_array().is_some_and(|r| !r.is_empty()),
                "entry {id} posted without a receipt: {e}"
            );
        }
    }
    let head = v["head"].clone();
    let _ = setup.cancel().await;

    // A restart replays the same books.
    books.restart();
    let c = connect(run.clone(), token.clone()).await;
    let v2 = call(&c, "books.verify", json!({})).await.unwrap();
    assert_eq!(v2["status"], "ok");
    assert_eq!(
        v2["head"], head,
        "the restarted server sees the same journal"
    );
    let _ = c.cancel().await;
}
