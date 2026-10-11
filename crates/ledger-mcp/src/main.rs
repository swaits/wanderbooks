//! `ledger-mcp`: the AI-first bookkeeping interface, over stdio.
//!
//! Usage: `ledger-mcp [--data-dir PATH] [--config PATH]`
//!
//! The data dir holds `journal.jsonl` (the source of truth), `wanderbooks.db`
//! (the disposable SQLite projection), `blobs/` (content-addressed receipts),
//! and optionally `config.json`. If no store exists, one is created with the
//! configured base currency.

mod policy;
mod server;
mod types;

use ledger_core::Currency;
use ledger_store::Store;
use policy::ReceiptPolicy;
use rmcp::{ServiceExt, transport::stdio};
use serde::Deserialize;
use server::BooksServer;
use std::path::PathBuf;

/// Server configuration, JSON. All fields optional.
#[derive(Debug, Default, Deserialize)]
struct Config {
    /// ISO 4217 base currency for a new store. Default "USD".
    #[serde(default)]
    base_currency: Option<String>,
    /// Receipt policy. Defaults to require-receipts-above $25.00.
    #[serde(default)]
    receipt_policy: ReceiptPolicy,
}

fn default_data_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".local/share/wanderbooks")
}

fn print_usage() {
    eprintln!("usage: ledger-mcp [--data-dir PATH] [--config PATH]");
    eprintln!("  --data-dir  directory holding journal.jsonl (default ~/.local/share/wanderbooks)");
    eprintln!("  --config    JSON config file (default <data-dir>/config.json, optional)");
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut data_dir = default_data_dir();
    let mut config_path: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--data-dir" => {
                data_dir = PathBuf::from(args.next().ok_or("--data-dir needs a path")?);
            }
            "--config" => {
                config_path = Some(PathBuf::from(args.next().ok_or("--config needs a path")?));
            }
            "--help" | "-h" => {
                print_usage();
                return Ok(());
            }
            other => {
                eprintln!("unknown argument: {other}");
                print_usage();
                std::process::exit(2);
            }
        }
    }
    let config_path = config_path.unwrap_or_else(|| data_dir.join("config.json"));
    let config: Config = if config_path.exists() {
        let raw = std::fs::read_to_string(&config_path)?;
        serde_json::from_str(&raw)
            .map_err(|e| format!("bad config {}: {e}", config_path.display()))?
    } else {
        Config::default()
    };
    // Logs go to stderr: stdout is the MCP channel.
    eprintln!("ledger-mcp: data dir {}", data_dir.display());

    let store = if data_dir.join(ledger_store::JOURNAL_FILE).exists() {
        Store::open(&data_dir)?
    } else {
        let code = config.base_currency.as_deref().unwrap_or("USD");
        let currency = Currency::new(code).map_err(|_| format!("bad base currency {code:?}"))?;
        eprintln!("ledger-mcp: creating new store (base currency {code})");
        Store::create(&data_dir, currency)?
    };
    // The journal is the source of truth: verify the chain and the
    // projection before serving a single tool call.
    store
        .verify()
        .map_err(|e| format!("store verify failed: {e}"))?;
    eprintln!("ledger-mcp: store verified, serving over stdio");

    let server = BooksServer::new(store, config.receipt_policy);
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        let running = server.serve(stdio()).await?;
        let _ = running.waiting().await;
        Ok::<(), Box<dyn std::error::Error>>(())
    })
}
