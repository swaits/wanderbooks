//! Concurrency over HTTP: several agents at once against one store.
//!
//! In `--http` mode every connection gets its own server over one shared,
//! locked store (main.rs `serve_http`). Oracle (server.rs INVARIANTS 2 and
//! 5, under contention): clients posting at the same time leave books
//! that verify, a trial balance that balances, every posted entry with
//! its receipt (threshold 0.00), and each draft posted exactly once even
//! when two clients race to post it; a restart replays the same books.

use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, ClientConfig};
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::StreamableHttpClientTransport;
use serde_json::{Value, json};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

type Client = RunningService<RoleClient, ClientConfig>;

const CLIENTS: usize = 6;
const ENTRIES: usize = 12;

struct Server(Child);
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn b64(bytes: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in bytes.chunks(3) {
        let n = (u32::from(c[0]) << 16)
            | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
            | u32::from(*c.get(2).unwrap_or(&0));
        for (i, shift) in [18, 12, 6, 0].iter().enumerate() {
            if i <= c.len() {
                out.push(T[((n >> shift) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

async fn connect(url: &str) -> Client {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let transport = StreamableHttpClientTransport::from_uri(url.to_string());
        match ClientConfig::default().serve(transport).await {
            Ok(c) => return c,
            Err(e) if Instant::now() > deadline => panic!("connect: {e}"),
            Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
        }
    }
}

/// Call a tool: Ok(structured result) or Err(the tool's error text).
async fn call(c: &Client, tool: &str, args: Value) -> Result<Value, String> {
    let args = args.as_object().cloned().unwrap_or_default();
    let r = c
        .call_tool(CallToolRequestParams::new(tool.to_string()).with_arguments(args))
        .await
        .map_err(|e| format!("protocol: {e}"))?;
    let text = r
        .content
        .iter()
        .find_map(|c| c.as_text().map(|t| t.text.clone()))
        .unwrap_or_default();
    if r.is_error == Some(true) {
        Err(text)
    } else {
        Ok(r.structured_content
            .unwrap_or_else(|| serde_json::from_str(&text).unwrap()))
    }
}

async fn draft(c: &Client, tag: &str, cents: u64, with_receipt: bool) -> u64 {
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
        call(
            c,
            "books.attach_receipt",
            json!({"entry_id": id, "file_name": format!("{tag}.pdf"),
                   "mime_type": "application/pdf", "content_base64": b64(tag.as_bytes())}),
        )
        .await
        .unwrap();
    }
    id
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn agents_posting_at_once_leave_whole_books() {
    let dir = std::env::temp_dir().join(format!("ledger-mcp-http-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let toml = dir.join("wanderbooks.toml");
    std::fs::write(&toml, "[books]\nreceipt_threshold = \"0.00\"\n").unwrap();
    let data = dir.join("books-data");
    let port = free_port();
    let addr = format!("127.0.0.1:{port}");
    let bin = env!("CARGO_BIN_EXE_ledger-mcp");
    let server = Server(
        Command::new(bin)
            .arg("--data-dir")
            .arg(&data)
            .arg("--books-toml")
            .arg(&toml)
            .arg("--http")
            .arg(&addr)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let url = format!("http://{addr}/mcp");
    let setup = connect(&url).await;
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
        contested.push(draft(&setup, &format!("contested {k}"), 100 + k as u64, true).await);
    }

    let mut tasks = Vec::new();
    for n in 0..CLIENTS {
        let url = url.clone();
        let contested = contested.clone();
        tasks.push(tokio::spawn(async move {
            let c = connect(&url).await;
            let (mut posted, mut refused, mut won) = (0usize, 0usize, 0usize);
            for (k, &race) in contested.iter().enumerate() {
                // A receipted entry posts; a bare one is refused.
                let receipted = k % 3 != 0;
                let id = draft(
                    &c,
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
    drop(server);

    // A restart replays the same books.
    let port = free_port();
    let addr = format!("127.0.0.1:{port}");
    let _server = Server(
        Command::new(bin)
            .arg("--data-dir")
            .arg(&data)
            .arg("--books-toml")
            .arg(&toml)
            .arg("--http")
            .arg(&addr)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let c = connect(&format!("http://{addr}/mcp")).await;
    let v2 = call(&c, "books.verify", json!({})).await.unwrap();
    assert_eq!(v2["status"], "ok");
    assert_eq!(
        v2["head"], head,
        "the restarted server sees the same journal"
    );
    let _ = c.cancel().await;
    let _ = std::fs::remove_dir_all(&dir);
}
