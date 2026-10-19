//! `ledger-mcp`: the AI-first bookkeeping interface, over stdio.
//!
//! [`run`] is the whole server; the `ledger-mcp` binary and `wanderbooks
//! dev-stdio` (one executable, plan D-22) both call it.
//!
//! Usage: `ledger-mcp [--data-dir PATH] [--config PATH] [--books-toml PATH]
//! [--import-root DIR]...`
//!
//! The data dir holds `journal.jsonl` (the source of truth), `wanderbooks.db`
//! (the disposable SQLite projection), `blobs/` (content-addressed receipts),
//! and optionally `config.json`. If no store exists, one is created with the
//! configured base currency. Company settings (name, fiscal year start,
//! receipt threshold) come from the books dir's `wanderbooks.toml`, by
//! default the one next to the data dir, and reload whenever it changes
//! (settings.rs). `books.import_local_file` and `books.set_w9`'s
//! `w9_file_path` read only from the import roots: each `--import-root`,
//! or by default the downloads folder (local_files.rs); a root that is
//! `/`, the home folder, or holds the books, the vault or a key is refused
//! at startup.

pub mod admin;
mod handbook;
#[cfg(unix)]
pub mod local;
mod local_files;
pub mod opening;
mod policy;
mod server;
mod settings;
mod tin_files;
mod types;
mod wire;

use ledger_core::Currency;
use ledger_store::Store;
use local_files::ImportRoots;
use policy::ReceiptPolicy;
use rmcp::ServiceExt;
use serde::Deserialize;
use server::BooksServer;
pub use server::{NOT_RUN_CLOSING, NOT_RUN_CLOSING_CODE};
use std::path::PathBuf;

/// Server configuration, JSON. All fields optional.
#[derive(Debug, Default, Deserialize)]
struct Config {
    /// ISO 4217 base currency for a new store. Default "USD".
    #[serde(default)]
    base_currency: Option<String>,
    // `receipt_policy` is read by ledger_config::effective_receipt_policy.
}

fn default_data_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".local/share/wanderbooks")
}

fn print_usage(prog: &str) {
    eprintln!(
        "usage: {prog} [--data-dir PATH] [--config PATH] [--books-toml PATH] [--vault-key-file PATH] [--http ADDR] [--import-root DIR]..."
    );
    eprintln!("  --data-dir  directory holding journal.jsonl (default ~/.local/share/wanderbooks)");
    eprintln!("  --config    JSON config file (default <data-dir>/config.json, optional)");
    eprintln!(
        "  --books-toml  company settings, for books that don't keep their own yet (default <data-dir>/../wanderbooks.toml, optional)"
    );
    eprintln!(
        "  --vault-key-file  the TIN vault's key file (default ~/.config/wanderbooks/vault.key; wins over a settings file's [vault] key_file)"
    );
    eprintln!(
        "  --http      serve over Streamable HTTP at ADDR (e.g. 127.0.0.1:8080) instead of stdio"
    );
    eprintln!(
        "  --import-root  a folder books.import_local_file may read from (repeatable; default: the downloads folder)"
    );
}

/// How [`run`] stops when it can't go on: the reason is already printed
/// on stderr, and the number is the process's exit code (2: a bad argument or a refused import folder; 1: books from a newer wanderbooks). A
/// library never ends the process itself, so the books it opened are
/// closed properly on the way out; the binaries turn this into the exit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Exit(pub i32);

impl std::fmt::Display for Exit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "exit code {}", self.0)
    }
}

impl std::error::Error for Exit {}

/// Run the books server with `args` (the command line after the program
/// name). `prog` names the program in the usage text: `ledger-mcp`, or
/// `wanderbooks dev-stdio`. It returns when the client disconnects; a bad
/// argument, books from a newer wanderbooks, or a refused import root
/// return [`Exit`] after a plain message.
pub fn run(
    prog: &str,
    args: impl IntoIterator<Item = String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut data_dir = default_data_dir();
    let mut config_path: Option<PathBuf> = None;
    let mut books_toml: Option<PathBuf> = None;
    let mut vault_key_file: Option<PathBuf> = None;
    let mut http_addr: Option<String> = None;
    let mut import_roots: Vec<PathBuf> = Vec::new();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--data-dir" => {
                data_dir = PathBuf::from(args.next().ok_or("--data-dir needs a path")?);
            }
            "--config" => {
                config_path = Some(PathBuf::from(args.next().ok_or("--config needs a path")?));
            }
            "--books-toml" => {
                books_toml = Some(PathBuf::from(
                    args.next().ok_or("--books-toml needs a path")?,
                ));
            }
            "--vault-key-file" => {
                vault_key_file = Some(PathBuf::from(
                    args.next().ok_or("--vault-key-file needs a path")?,
                ));
            }
            "--http" => {
                http_addr = Some(args.next().ok_or("--http needs an address")?);
            }
            "--import-root" => {
                import_roots.push(PathBuf::from(
                    args.next().ok_or("--import-root needs a folder")?,
                ));
            }
            "--help" | "-h" => {
                print_usage(prog);
                return Ok(());
            }
            other => {
                eprintln!("unknown argument: {other}");
                print_usage(prog);
                return Err(Box::new(Exit(2)));
            }
        }
    }
    // Books in the data root are the books server's alone (plan 3.1):
    // a second writer beside it is refused before anything is opened.
    if ledger_store::root::inside_a_root(&data_dir) {
        eprintln!(
            "ledger-mcp: those books are in the books' private folder, which only the books server opens: connect to them with `wanderbooks mcp --book <id>`"
        );
        return Err(Box::new(Exit(2)));
    }
    let import_roots = if import_roots.is_empty() {
        ImportRoots::defaults()
    } else {
        ImportRoots::new(import_roots)
    };
    let config_path = config_path.unwrap_or_else(|| data_dir.join("config.json"));
    // config.json's base currency matters only to books being created: it
    // is read then, and never otherwise (books that keep their own
    // settings read no file).
    let read_config = || -> Result<Config, Box<dyn std::error::Error>> {
        if config_path.exists() {
            let raw = std::fs::read_to_string(&config_path)?;
            Ok(serde_json::from_str(&raw)
                .map_err(|e| format!("bad config {}: {e}", config_path.display()))?)
        } else {
            Ok(Config::default())
        }
    };
    // Logs go to stderr: stdout is the MCP channel.
    eprintln!("ledger-mcp: data dir {}", data_dir.display());
    // An explicit --books-toml must exist; the default (beside the data
    // dir) is optional. Either is reloaded whenever it changes.
    let source = settings::Source {
        required: books_toml.is_some(),
        books_toml: books_toml.unwrap_or_else(|| {
            data_dir
                .parent()
                .map(|p| p.join("wanderbooks.toml"))
                .unwrap_or_else(|| PathBuf::from("wanderbooks.toml"))
        }),
        config_json: config_path.clone(),
    };

    let mut store = if data_dir.join(ledger_store::JOURNAL_FILE).exists() {
        match open_waiting_for_lock(&data_dir) {
            Ok(store) => store,
            // Books from a newer wanderbooks are fine; this server is old.
            // Say so in plain words (the error's text), and leave them be.
            Err(e @ ledger_store::StoreError::NewerJournal { .. }) => {
                eprintln!("ledger-mcp: {e}");
                return Err(Box::new(Exit(1)));
            }
            Err(e) => return Err(e.into()),
        }
    } else {
        let config = read_config()?;
        let code = config.base_currency.as_deref().unwrap_or("USD");
        let currency = Currency::new(code).map_err(|_| format!("bad base currency {code:?}"))?;
        eprintln!("ledger-mcp: creating new store (base currency {code})");
        Store::create(&data_dir, currency)?
    };
    // The journal is the source of truth: verify the chain, the replay, and
    // the whole projection before serving a single tool call, rebuilding
    // the projection if it drifted.
    let report = store
        .verify_and_heal()
        .map_err(|e| format!("store verify failed: {e}"))?;
    if let Some(why) = &report.healed {
        eprintln!("ledger-mcp: rebuilt the database from the journal ({why})");
    }
    // Books that keep their own settings never read a settings file, not
    // even to start (settings INVARIANT 1); others read it as before.
    let mut live = if store.ledger().settings().is_journaled() {
        settings::Live::journaled(source)
    } else {
        settings::Live::watching(source)?
    }
    .with_vault_key_file(vault_key_file);
    live.sync(store.ledger().settings(), store.base_currency().code());
    eprintln!(
        "ledger-mcp: company {:?} (settings from {})",
        live.current().company.name,
        if live.is_journaled() {
            "the books"
        } else {
            "the settings file"
        }
    );
    let live = std::sync::Arc::new(std::sync::Mutex::new(live));
    // No import root may be `/`, the home folder, or hold the books, the
    // vault or a key (local_files.rs INVARIANT 0): a named one stops the
    // server here, a default one is dropped.
    let settings_now = live.lock().unwrap_or_else(|e| e.into_inner()).current();
    let import_roots = match server::guard_import_roots(
        &store,
        &settings_now,
        &server::Layout::BooksDir,
        import_roots,
    ) {
        Ok(roots) => roots,
        Err(why) => {
            eprintln!("ledger-mcp: {why}");
            return Err(Box::new(Exit(2)));
        }
    };

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    if let Some(addr) = http_addr {
        eprintln!("ledger-mcp: store verified, serving Streamable HTTP on {addr}");
        rt.block_on(serve_http(addr, store, live, import_roots))?;
        return Ok(());
    }

    eprintln!("ledger-mcp: store verified, serving over stdio");
    let server = BooksServer::new(store, ReceiptPolicy::default())
        .with_live_settings(live)
        .with_import_roots(import_roots);
    rt.block_on(async {
        // The wire answers what JSON-RPC requires that rmcp drops (wire.rs).
        let (input, output, writer) = wire::spawn(tokio::io::stdin(), tokio::io::stdout());
        let running = server.serve((input, output)).await?;
        let _ = running.waiting().await;
        let _ = writer.await;
        Ok::<(), Box<dyn std::error::Error>>(())
    })
}

/// Serve MCP over Streamable HTTP (for network clients).
async fn serve_http(
    addr: String,
    store: Store,
    live: std::sync::Arc<std::sync::Mutex<settings::Live>>,
    import_roots: ImportRoots,
) -> Result<(), Box<dyn std::error::Error>> {
    use rmcp::transport::streamable_http_server::{
        StreamableHttpService, session::local::LocalSessionManager,
    };
    use std::sync::Arc;

    // One store for every connection: the async mutex keeps it
    // single-writer, and tool bodies run on blocking threads (server.rs).
    let store = server::SharedStore::new(store);
    let service = StreamableHttpService::new(
        move || {
            let server = server::BooksServer::new_shared(store.clone(), ReceiptPolicy::default())
                .with_live_settings(live.clone())
                .with_import_roots(import_roots.clone());
            Ok(server)
        },
        Arc::new(LocalSessionManager::default()),
        Default::default(),
    );

    let router = axum::Router::new().nest_service("/mcp", service);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    eprintln!("ledger-mcp: listening on http://{addr}/mcp");
    axum::serve(listener, router).await?;
    Ok(())
}

/// Open the books, waiting up to ten seconds for another writer to let go.
/// A restarted server (a client reconnecting) often starts while the old
/// process is still exiting with the lock; waiting turns that race into a
/// short delay. A writer that keeps the books longer is a real conflict,
/// and the refusal says so.
fn open_waiting_for_lock(data_dir: &std::path::Path) -> Result<Store, ledger_store::StoreError> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        match Store::open(data_dir) {
            Err(ledger_store::StoreError::Locked(_)) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            other => return other,
        }
    }
}

#[cfg(test)]
mod exit_tests {
    use super::Exit;

    #[test]
    fn an_exit_says_its_code() {
        assert_eq!(Exit(2).to_string(), "exit code 2");
        assert_eq!(Exit(1).to_string(), "exit code 1");
    }
}
